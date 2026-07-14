//! FM-77AV serial keyboard encoder and its embedded real-time clock.
//!
//! The FM-77AV places an intelligent keyboard controller on the sub bus at
//! `0xD431` (data) and `0xD432` (status). The host drives it with a small command
//! protocol layered on top of the raw keycode stream: it selects the scancode
//! mode, controls the keyboard LEDs, programs auto-repeat timing, and reads or
//! writes the calendar clock kept inside the encoder.
//!
//! The handshake is polled rather than interrupt driven. Each command byte the
//! host writes lowers the ACK bit; the bus re-raises it a short delay later
//! (modelled with the [`crate::scheduler::EventFm7::EncoderAck`] event). Response
//! bytes are queued and drained one at a time while the RXRDY bit reports data is
//! waiting.
//!
//! This module owns only the encoder state and protocol logic; scheduling the ACK
//! delay, the one-second RTC tick, and the auto-repeat timer stays on the bus,
//! which acts on the [`EncoderAction`] returned from [`KeyboardEncoder::write_data`].

/// Depth of the command and response byte FIFOs. The longest command (an RTC set)
/// is nine bytes, so sixteen entries leaves ample headroom; the oldest entry is
/// dropped on overflow, matching the hardware ring.
const ENCODER_FIFO_CAPACITY: usize = 16;

/// Status bit 7 (RXRDY): cleared while the encoder has response data to read.
const STATUS_RECEIVE_READY_BIT: u8 = 0x80;
/// Status bit 0 (ACK): cleared while the encoder is busy accepting a command byte.
const STATUS_ACK_BIT: u8 = 0x01;
/// Status value with no bit asserted low (idle, nothing to read).
const STATUS_IDLE: u8 = 0xFF;

/// Command `0x00`: set the scancode reporting mode.
const CMD_SET_SCANCODE_MODE: u8 = 0x00;
/// Command `0x01`: read the scancode reporting mode.
const CMD_GET_SCANCODE_MODE: u8 = 0x01;
/// Command `0x02`: set the keyboard LED state.
const CMD_SET_LEDS: u8 = 0x02;
/// Command `0x03`: read the keyboard LED state.
const CMD_GET_LEDS: u8 = 0x03;
/// Command `0x04`: enable or disable key auto-repeat.
const CMD_SET_REPEAT_TYPE: u8 = 0x04;
/// Command `0x05`: set the auto-repeat first-delay and interval times.
const CMD_SET_REPEAT_TIME: u8 = 0x05;
/// Command `0x80`: access the real-time clock (get or set).
const CMD_RTC_ACCESS: u8 = 0x80;
/// Command `0x81`: start a video digitize; recognized but not modelled.
const CMD_DIGITIZE: u8 = 0x81;
/// Command `0x82`: set the screen mode; recognized but not modelled.
const CMD_SET_SCREEN_MODE: u8 = 0x82;
/// Command `0x83`: read the screen mode; recognized but not modelled.
const CMD_GET_SCREEN_MODE: u8 = 0x83;
/// Command `0x84`: set the display brightness; recognized but not modelled.
const CMD_SET_BRIGHTNESS: u8 = 0x84;

/// RTC sub-command `0x00`: read the calendar clock (returns seven packed bytes).
const RTC_SUBCOMMAND_GET: u8 = 0x00;
/// RTC sub-command `0x01`: write the calendar clock (takes seven packed bytes).
const RTC_SUBCOMMAND_SET: u8 = 0x01;

/// Number of packed bytes exchanged for the calendar clock.
const RTC_PACKED_BYTES: usize = 7;
/// Total length of an RTC set command: opcode, sub-command, then seven payload
/// bytes.
const RTC_SET_COMMAND_LENGTH: usize = 2 + RTC_PACKED_BYTES;

/// Command length of the fixed two-byte "set" commands (opcode plus one argument).
const SET_COMMAND_LENGTH: usize = 2;
/// Command length of the three-byte set-repeat-time command.
const REPEAT_TIME_COMMAND_LENGTH: usize = 3;

/// LED selector bit 1: choose the KANA LED when set, the CAPS LED when clear.
const LED_SELECT_KANA_BIT: u8 = 0x02;
/// LED selector bit 0: the addressed LED turns on when this bit is clear.
const LED_VALUE_OFF_BIT: u8 = 0x01;
/// LED status bit 0: the INSERT LED is on.
const LED_STATUS_INSERT_BIT: u8 = 0x01;
/// LED status bit 1: the KANA LED is on.
const LED_STATUS_KANA_BIT: u8 = 0x02;
/// LED status bit 2: the CAPS LED is on.
const LED_STATUS_CAPS_BIT: u8 = 0x04;

/// Auto-repeat set-type argument selecting repeat on. Any other value turns it off.
const REPEAT_TYPE_ENABLE: u8 = 0x00;
/// Milliseconds represented by one count of the set-repeat-time arguments.
const REPEAT_TIME_UNIT_MS: u64 = 10;
/// Default delay before the first auto-repeat, in milliseconds.
const DEFAULT_REPEAT_DELAY_MS: u64 = 700;
/// Default interval between auto-repeats, in milliseconds.
const DEFAULT_REPEAT_INTERVAL_MS: u64 = 70;

/// First scancode that never auto-repeats. Codes at or above this (the modifier
/// keys and BREAK) are excluded from repeat generation.
const REPEAT_SCANCODE_LIMIT: u8 = 0x5C;

/// Packed byte 2 low bits: day-of-month BCD (tens in bits 5-4, ones in bits 3-0).
/// Bit 6 carries a leap-year flag on read and is masked off here.
const RTC_DAY_MASK: u8 = 0x3F;
/// Packed byte 4 bit 3: twelve-hour representation flag.
const RTC_TWELVE_HOUR_BIT: u8 = 0x08;
/// Packed byte 4 bit 2: PM flag, valid only in twelve-hour representation.
const RTC_PM_BIT: u8 = 0x04;
/// Packed byte 4 low bits: hour tens digit.
const RTC_HOUR_TENS_MASK: u8 = 0x03;
/// Bit shift moving the day-of-week into the high nibble of packed byte 4.
const RTC_DAY_OF_WEEK_SHIFT: u8 = 4;

/// Two-digit-year value at or above which the year maps into the 1900s; lower
/// values map into the 2000s.
const RTC_YEAR_WINDOW_SPLIT: u16 = 80;
/// Century base applied to two-digit years below the window split.
const RTC_CENTURY_2000: u16 = 2000;
/// Century base applied to two-digit years at or above the window split.
const RTC_CENTURY_1900: u16 = 1900;

/// Scancode reporting mode selected through the `0x00`/`0x01` commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum ScancodeMode {
    /// Standard F-BASIC keycodes (the power-on default).
    Standard = 0,
    /// FM-16beta compatible codes.
    Fm16Beta = 1,
    /// Raw make/break scancodes.
    Scan = 2,
}

/// Follow-up work the bus must perform after a command byte is accepted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct EncoderAction {
    /// The encoder lowered ACK and needs it re-raised after the handshake delay.
    pub schedule_ack: bool,
    /// A calendar-clock set completed; the one-second cadence must re-anchor so
    /// the freshly written second elapses in full.
    pub reanchor_rtc: bool,
}

/// Fixed-capacity byte FIFO backing the command and response queues. The oldest
/// entry is dropped when the ring overflows.
struct ByteFifo {
    entries: [u8; ENCODER_FIFO_CAPACITY],
    head: usize,
    len: usize,
}

impl ByteFifo {
    /// Creates an empty FIFO.
    fn new() -> Self {
        Self {
            entries: [0; ENCODER_FIFO_CAPACITY],
            head: 0,
            len: 0,
        }
    }

    /// Appends `byte`, dropping the oldest entry when the FIFO is full.
    fn push(&mut self, byte: u8) {
        if self.len == ENCODER_FIFO_CAPACITY {
            self.head = (self.head + 1) % ENCODER_FIFO_CAPACITY;
            self.len -= 1;
        }
        let tail = (self.head + self.len) % ENCODER_FIFO_CAPACITY;
        self.entries[tail] = byte;
        self.len += 1;
    }

    /// Removes and returns the oldest entry, if any.
    fn pop(&mut self) -> Option<u8> {
        if self.len == 0 {
            return None;
        }
        let byte = self.entries[self.head];
        self.head = (self.head + 1) % ENCODER_FIFO_CAPACITY;
        self.len -= 1;
        Some(byte)
    }

    /// Returns the entry `index` places from the front without removing it, or
    /// zero when the index is past the end.
    fn get(&self, index: usize) -> u8 {
        if index >= self.len {
            return 0;
        }
        self.entries[(self.head + index) % ENCODER_FIFO_CAPACITY]
    }

    /// The number of queued entries.
    fn len(&self) -> usize {
        self.len
    }

    /// Whether the FIFO holds no entries.
    fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Discards every queued entry.
    fn clear(&mut self) {
        self.head = 0;
        self.len = 0;
    }
}

/// FM-77AV keyboard encoder: the command handshake, configuration state, and the
/// embedded real-time clock.
pub(crate) struct KeyboardEncoder {
    /// Whether response data is waiting to be read (drives status bit 7).
    receive_ready: bool,
    /// Whether the encoder has acknowledged the last byte and can accept another
    /// (drives status bit 0).
    command_acknowledged: bool,
    /// Bytes the host has written for the in-flight command.
    command_fifo: ByteFifo,
    /// Response bytes queued for the host to read back.
    response_fifo: ByteFifo,
    /// Opcode of the command currently being assembled (the first queued byte).
    command_opcode: u8,
    /// Last byte handed out on a data read, returned again when the queue is empty.
    data_register: u8,
    /// Selected scancode reporting mode.
    scancode_mode: ScancodeMode,
    /// Whether the CAPS LED is lit.
    caps_led_on: bool,
    /// Whether the KANA LED is lit.
    kana_led_on: bool,
    /// Whether the INSERT LED is lit.
    insert_led_on: bool,
    /// Whether auto-repeat is enabled.
    repeat_enabled: bool,
    /// Delay before the first auto-repeat, in milliseconds.
    repeat_delay_ms: u64,
    /// Interval between subsequent auto-repeats, in milliseconds.
    repeat_interval_ms: u64,
    /// Physical scancode currently repeating, if any.
    repeat_scancode: Option<u8>,
    /// Calendar year (full four-digit form).
    rtc_year: u16,
    /// Calendar month, 1-12.
    rtc_month: u8,
    /// Calendar day of month, 1-31.
    rtc_day: u8,
    /// Day of week, 0 (Sunday) through 6 (Saturday).
    rtc_day_of_week: u8,
    /// Hour of day in canonical 24-hour form, 0-23.
    rtc_hour: u8,
    /// Minute, 0-59.
    rtc_minute: u8,
    /// Second, 0-59.
    rtc_second: u8,
    /// Mirrors the encoder's clock-representation flag. When set the clock is
    /// exchanged in twelve-hour plus AM/PM form; when clear, in 24-hour form. The
    /// stored [`Self::rtc_hour`] is always the canonical 24-hour value.
    rtc_twelve_hour: bool,
}

impl KeyboardEncoder {
    /// Creates an encoder in its power-on state: ready to accept a command, no
    /// response pending, standard scancode mode, LEDs off, and default repeat
    /// timing.
    pub(crate) fn new() -> Self {
        Self {
            receive_ready: false,
            command_acknowledged: true,
            command_fifo: ByteFifo::new(),
            response_fifo: ByteFifo::new(),
            command_opcode: 0,
            data_register: 0,
            scancode_mode: ScancodeMode::Standard,
            caps_led_on: false,
            kana_led_on: false,
            insert_led_on: false,
            repeat_enabled: false,
            repeat_delay_ms: DEFAULT_REPEAT_DELAY_MS,
            repeat_interval_ms: DEFAULT_REPEAT_INTERVAL_MS,
            repeat_scancode: None,
            rtc_year: RTC_CENTURY_2000,
            rtc_month: 1,
            rtc_day: 1,
            rtc_day_of_week: 0,
            rtc_hour: 0,
            rtc_minute: 0,
            rtc_second: 0,
            rtc_twelve_hour: false,
        }
    }

    /// Reads the status register (`0xD432`). Both meaningful bits are active low:
    /// bit 7 clears while response data is waiting, bit 0 clears while the encoder
    /// is still busy with the previous byte.
    pub(crate) fn read_status(&self) -> u8 {
        let mut status = STATUS_IDLE;
        if self.receive_ready {
            status &= !STATUS_RECEIVE_READY_BIT;
        }
        if !self.command_acknowledged {
            status &= !STATUS_ACK_BIT;
        }
        status
    }

    /// Reads the data register (`0xD431`), draining one response byte. The last
    /// byte is held when the queue empties, matching the hardware latch.
    pub(crate) fn read_data(&mut self) -> u8 {
        if let Some(byte) = self.response_fifo.pop() {
            self.data_register = byte;
        }
        self.receive_ready = !self.response_fifo.is_empty();
        self.data_register
    }

    /// Accepts a command byte written to the data register (`0xD431`) and returns
    /// the follow-up work for the bus. A byte written while the encoder is busy
    /// (ACK low) is ignored, as on hardware.
    pub(crate) fn write_data(&mut self, byte: u8) -> EncoderAction {
        if !self.command_acknowledged {
            return EncoderAction::default();
        }
        if self.command_fifo.is_empty() {
            self.command_opcode = byte;
        }
        self.command_fifo.push(byte);
        self.command_acknowledged = false;
        self.receive_ready = false;
        let reanchor_rtc = self.dispatch_command();
        EncoderAction {
            schedule_ack: true,
            reanchor_rtc,
        }
    }

    /// Re-raises the ACK bit once the handshake delay elapses.
    pub(crate) fn acknowledge(&mut self) {
        self.command_acknowledged = true;
    }

    /// Dispatches the in-flight command once enough bytes have accumulated,
    /// returning whether a calendar-clock set completed.
    fn dispatch_command(&mut self) -> bool {
        match self.command_opcode {
            CMD_SET_SCANCODE_MODE => self.complete_fixed(SET_COMMAND_LENGTH, |encoder| {
                encoder.apply_scancode_mode(encoder.command_fifo.get(1));
            }),
            CMD_GET_SCANCODE_MODE => {
                let mode = self.scancode_mode as u8;
                self.push_response(mode);
                self.command_fifo.clear();
            }
            CMD_SET_LEDS => self.complete_fixed(SET_COMMAND_LENGTH, |encoder| {
                encoder.apply_leds(encoder.command_fifo.get(1));
            }),
            CMD_GET_LEDS => {
                let status = self.caps_kana_led_byte();
                self.push_response(status);
                self.command_fifo.clear();
            }
            CMD_SET_REPEAT_TYPE => self.complete_fixed(SET_COMMAND_LENGTH, |encoder| {
                encoder.apply_repeat_type(encoder.command_fifo.get(1));
            }),
            CMD_SET_REPEAT_TIME => self.complete_fixed(REPEAT_TIME_COMMAND_LENGTH, |encoder| {
                encoder.apply_repeat_time(encoder.command_fifo.get(1), encoder.command_fifo.get(2));
            }),
            CMD_RTC_ACCESS => return self.dispatch_rtc(),
            CMD_DIGITIZE | CMD_SET_SCREEN_MODE | CMD_SET_BRIGHTNESS => {
                if self.command_fifo.len() >= SET_COMMAND_LENGTH {
                    self.command_fifo.clear();
                }
            }
            CMD_GET_SCREEN_MODE => self.command_fifo.clear(),
            _ => self.command_fifo.clear(),
        }
        false
    }

    /// Runs `apply` once the command has reached `length` bytes, then clears the
    /// command FIFO.
    fn complete_fixed(&mut self, length: usize, apply: impl FnOnce(&mut Self)) {
        if self.command_fifo.len() >= length {
            apply(self);
            self.command_fifo.clear();
        }
    }

    /// Handles the RTC access command once its sub-command and any payload have
    /// arrived, returning whether a set completed.
    fn dispatch_rtc(&mut self) -> bool {
        if self.command_fifo.len() < SET_COMMAND_LENGTH {
            return false;
        }
        match self.command_fifo.get(1) {
            RTC_SUBCOMMAND_GET => {
                self.pack_rtc_response();
                self.command_fifo.clear();
                false
            }
            RTC_SUBCOMMAND_SET => {
                if self.command_fifo.len() >= RTC_SET_COMMAND_LENGTH {
                    let mut payload = [0u8; RTC_PACKED_BYTES];
                    for (index, slot) in payload.iter_mut().enumerate() {
                        *slot = self.command_fifo.get(2 + index);
                    }
                    self.unpack_rtc(&payload);
                    self.command_fifo.clear();
                    true
                } else {
                    false
                }
            }
            _ => {
                self.command_fifo.clear();
                false
            }
        }
    }

    /// Queues one response byte and marks response data as ready.
    fn push_response(&mut self, byte: u8) {
        self.response_fifo.push(byte);
        self.receive_ready = true;
    }

    /// Applies the scancode-mode argument, ignoring out-of-range values.
    fn apply_scancode_mode(&mut self, mode: u8) {
        self.scancode_mode = match mode {
            0 => ScancodeMode::Standard,
            1 => ScancodeMode::Fm16Beta,
            2 => ScancodeMode::Scan,
            _ => self.scancode_mode,
        };
    }

    /// Applies the LED selector: bit 1 chooses KANA or CAPS, and the addressed LED
    /// lights when bit 0 is clear.
    fn apply_leds(&mut self, selector: u8) {
        let on = selector & LED_VALUE_OFF_BIT == 0;
        if selector & LED_SELECT_KANA_BIT != 0 {
            self.kana_led_on = on;
        } else {
            self.caps_led_on = on;
        }
    }

    /// Encodes the CAPS and KANA LED state for the get-LED response.
    fn caps_kana_led_byte(&self) -> u8 {
        let mut byte = 0;
        if self.caps_led_on {
            byte |= LED_STATUS_INSERT_BIT;
        }
        if self.kana_led_on {
            byte |= LED_STATUS_KANA_BIT;
        }
        byte
    }

    /// Enables auto-repeat when the argument selects it, disabling it otherwise.
    fn apply_repeat_type(&mut self, mode: u8) {
        self.repeat_enabled = mode == REPEAT_TYPE_ENABLE;
    }

    /// Sets the auto-repeat first-delay and interval from their count arguments.
    /// A zero count restores the default timing.
    fn apply_repeat_time(&mut self, delay_count: u8, interval_count: u8) {
        if delay_count == 0 || interval_count == 0 {
            self.repeat_delay_ms = DEFAULT_REPEAT_DELAY_MS;
            self.repeat_interval_ms = DEFAULT_REPEAT_INTERVAL_MS;
        } else {
            self.repeat_delay_ms = u64::from(delay_count) * REPEAT_TIME_UNIT_MS;
            self.repeat_interval_ms = u64::from(interval_count) * REPEAT_TIME_UNIT_MS;
        }
    }

    /// Packs the calendar clock into its seven-byte response representation.
    fn pack_rtc_response(&mut self) {
        let (display_hour, is_pm) = if self.rtc_twelve_hour {
            (self.rtc_hour % 12, self.rtc_hour >= 12)
        } else {
            (self.rtc_hour, false)
        };
        let hour_tens = display_hour / 10;
        let hour_ones = display_hour % 10;

        let byte0 = to_bcd((self.rtc_year % 100) as u8);
        let byte1 = to_bcd(self.rtc_month);
        let byte2 = to_bcd(self.rtc_day);
        let mut byte3 = self.rtc_day_of_week << RTC_DAY_OF_WEEK_SHIFT;
        if self.rtc_twelve_hour {
            byte3 |= RTC_TWELVE_HOUR_BIT;
        }
        if is_pm {
            byte3 |= RTC_PM_BIT;
        }
        byte3 |= hour_tens & RTC_HOUR_TENS_MASK;
        let byte4 = (hour_ones << 4) | (self.rtc_minute / 10);
        let byte5 = ((self.rtc_minute % 10) << 4) | (self.rtc_second / 10);
        let byte6 = (self.rtc_second % 10) << 4;

        for byte in [byte0, byte1, byte2, byte3, byte4, byte5, byte6] {
            self.push_response(byte);
        }
    }

    /// Decodes a seven-byte calendar-clock payload written by the host.
    fn unpack_rtc(&mut self, payload: &[u8; RTC_PACKED_BYTES]) {
        let two_digit_year = u16::from(from_bcd(payload[0]));
        self.rtc_year = if two_digit_year < RTC_YEAR_WINDOW_SPLIT {
            RTC_CENTURY_2000 + two_digit_year
        } else {
            RTC_CENTURY_1900 + two_digit_year
        };
        self.rtc_month = from_bcd(payload[1]);
        self.rtc_day = from_bcd(payload[2] & RTC_DAY_MASK);

        let byte3 = payload[3];
        self.rtc_day_of_week = byte3 >> RTC_DAY_OF_WEEK_SHIFT;
        self.rtc_twelve_hour = byte3 & RTC_TWELVE_HOUR_BIT != 0;
        let is_pm = byte3 & RTC_PM_BIT != 0;
        let hour_tens = byte3 & RTC_HOUR_TENS_MASK;
        let hour_ones = payload[4] >> 4;
        let display_hour = hour_tens * 10 + hour_ones;
        self.rtc_hour = if self.rtc_twelve_hour {
            (display_hour % 12) + if is_pm { 12 } else { 0 }
        } else {
            display_hour
        };

        let minute_tens = payload[4] & 0x0F;
        let minute_ones = payload[5] >> 4;
        self.rtc_minute = minute_tens * 10 + minute_ones;
        let second_tens = payload[5] & 0x0F;
        let second_ones = payload[6] >> 4;
        self.rtc_second = second_tens * 10 + second_ones;
    }

    /// Seeds the calendar clock from a host time in canonical 24-hour form.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn seed_from_host(
        &mut self,
        year: u16,
        month: u8,
        day: u8,
        day_of_week: u8,
        hour: u8,
        minute: u8,
        second: u8,
    ) {
        self.rtc_year = year;
        self.rtc_month = month;
        self.rtc_day = day;
        self.rtc_day_of_week = day_of_week;
        self.rtc_hour = hour;
        self.rtc_minute = minute;
        self.rtc_second = second;
    }

    /// Advances the calendar clock by one second, carrying into the minute, hour,
    /// day, day-of-week, month, and year as needed.
    pub(crate) fn advance_one_second(&mut self) {
        self.rtc_second += 1;
        if self.rtc_second < 60 {
            return;
        }
        self.rtc_second = 0;
        self.rtc_minute += 1;
        if self.rtc_minute < 60 {
            return;
        }
        self.rtc_minute = 0;
        self.rtc_hour += 1;
        if self.rtc_hour < 24 {
            return;
        }
        self.rtc_hour = 0;
        self.rtc_day += 1;
        self.rtc_day_of_week = (self.rtc_day_of_week + 1) % 7;
        if self.rtc_day <= days_in_month(self.rtc_year, self.rtc_month) {
            return;
        }
        self.rtc_day = 1;
        self.rtc_month += 1;
        if self.rtc_month <= 12 {
            return;
        }
        self.rtc_month = 1;
        self.rtc_year += 1;
    }

    /// Lights or clears the INSERT LED (driven by the sub `0xD40D` register).
    pub(crate) fn set_insert_led(&mut self, on: bool) {
        self.insert_led_on = on;
    }

    /// The combined LED status: bit 0 INSERT, bit 1 KANA, bit 2 CAPS.
    pub(crate) fn led_status(&self) -> u8 {
        let mut status = 0;
        if self.insert_led_on {
            status |= LED_STATUS_INSERT_BIT;
        }
        if self.kana_led_on {
            status |= LED_STATUS_KANA_BIT;
        }
        if self.caps_led_on {
            status |= LED_STATUS_CAPS_BIT;
        }
        status
    }

    /// The selected scancode reporting mode.
    pub(crate) fn scancode_mode(&self) -> ScancodeMode {
        self.scancode_mode
    }

    /// Whether auto-repeat should generate keystrokes: enabled and not in the raw
    /// scancode mode, which suppresses repeat.
    pub(crate) fn auto_repeat_active(&self) -> bool {
        self.repeat_enabled && self.scancode_mode != ScancodeMode::Scan
    }

    /// The configured delay before the first auto-repeat, in milliseconds.
    pub(crate) fn repeat_delay_ms(&self) -> u64 {
        self.repeat_delay_ms
    }

    /// The configured interval between auto-repeats, in milliseconds.
    pub(crate) fn repeat_interval_ms(&self) -> u64 {
        self.repeat_interval_ms
    }

    /// Records `scancode` as the key now repeating.
    pub(crate) fn arm_repeat(&mut self, scancode: u8) {
        self.repeat_scancode = Some(scancode);
    }

    /// The scancode currently repeating, if any.
    pub(crate) fn repeat_scancode(&self) -> Option<u8> {
        self.repeat_scancode
    }

    /// Cancels auto-repeat when `scancode` matches the repeating key, returning
    /// whether it was cancelled.
    pub(crate) fn cancel_repeat_if(&mut self, scancode: u8) -> bool {
        if self.repeat_scancode == Some(scancode) {
            self.repeat_scancode = None;
            true
        } else {
            false
        }
    }
}

impl Default for KeyboardEncoder {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether `scancode` is eligible for auto-repeat: below the modifier/BREAK range.
pub(crate) fn is_repeatable_scancode(scancode: u8) -> bool {
    scancode < REPEAT_SCANCODE_LIMIT
}

/// Converts a binary value (0-99) to packed binary-coded decimal.
fn to_bcd(value: u8) -> u8 {
    ((value / 10) << 4) | (value % 10)
}

/// Converts a packed binary-coded-decimal byte back to binary.
fn from_bcd(value: u8) -> u8 {
    ((value >> 4) * 10) + (value & 0x0F)
}

/// The number of days in `month` (1-12) of `year`, accounting for leap years.
fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Whether `year` is a Gregorian leap year.
fn is_leap_year(year: u16) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}
