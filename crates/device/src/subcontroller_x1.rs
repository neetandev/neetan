//! High-level emulation of the X1 sub-CPU (an 80C49 microcontroller).
//!
//! The main Z80 talks to the sub-CPU through a single-byte mailbox at I/O
//! `0x1900` with IBF/OBF handshake flags mirrored onto PPI port B. The sub-CPU
//! handles the keyboard, the calendar/clock (an in-line uPD1990A RTC), the
//! interval timers, and the cassette transport. We do not emulate the MCU.
//! We model its command/mailbox protocol.
//!
//! The sub-CPU is stepped by a periodic `SubPoll` event (~400 us). Each step
//! consumes a command or parameter byte from the main CPU, or pushes a result
//! byte or a keyboard interrupt vector back.

mod keytables;

use alloc_free_collections::KeyFifo;
use keytables::{
    KEYCODE_CTRL, KEYCODE_GRAPH, KEYCODE_KANA, KEYCODE_KANA_B, KEYCODE_KANA_SHIFT,
    KEYCODE_KANA_SHIFT_B, KEYCODE_NORMAL, KEYCODE_SHIFT,
};

/// Position of the X1 turbo keyboard mode switch.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum X1KeyboardMode {
    /// Standard kana layout with an inactive game-key command.
    #[default]
    ModeA,
    /// Alternate kana layout with a live game-key matrix.
    ModeB,
}

impl std::fmt::Display for X1KeyboardMode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ModeA => formatter.write_str("A"),
            Self::ModeB => formatter.write_str("B"),
        }
    }
}

impl std::str::FromStr for X1KeyboardMode {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text.to_ascii_uppercase().as_str() {
            "A" => Ok(Self::ModeA),
            "B" => Ok(Self::ModeB),
            _ => Err(format!(
                "unknown X1 keyboard mode '{text}', expected A or B"
            )),
        }
    }
}

impl save_state::StateEncode for X1KeyboardMode {
    fn encode_state(&self, output: &mut Vec<u8>) {
        save_state::StateEncode::encode_state(&(*self as u8), output);
    }
}

impl save_state::StateDecode for X1KeyboardMode {
    fn decode_state(
        decoder: &mut save_state::StateDecoder<'_>,
    ) -> Result<Self, save_state::StateDecodeError> {
        match <u8 as save_state::StateDecode>::decode_state(decoder)? {
            0 => Ok(Self::ModeA),
            1 => Ok(Self::ModeB),
            _ => Err(save_state::StateDecodeError::InvalidTag),
        }
    }
}

mod alloc_free_collections {
    save_state::runtime_state! {
    /// Small FIFO of pending key events (matches the 8-entry hardware buffer).
    #[derive(Debug, Clone)]
    pub struct KeyFifo {
        entries: [u16; Self::CAPACITY],
        head: usize,
        len: usize,
    }}

    impl KeyFifo {
        const CAPACITY: usize = 8;

        pub fn new() -> Self {
            Self {
                entries: [0; Self::CAPACITY],
                head: 0,
                len: 0,
            }
        }

        pub fn clear(&mut self) {
            self.head = 0;
            self.len = 0;
        }

        pub fn is_empty(&self) -> bool {
            self.len == 0
        }

        pub fn push(&mut self, value: u16) {
            if self.len == Self::CAPACITY {
                // Drop the oldest entry, as the hardware buffer overwrites.
                self.head = (self.head + 1) % Self::CAPACITY;
                self.len -= 1;
            }
            let tail = (self.head + self.len) % Self::CAPACITY;
            self.entries[tail] = value;
            self.len += 1;
        }

        pub fn pop(&mut self) -> Option<u16> {
            if self.len == 0 {
                return None;
            }
            let value = self.entries[self.head];
            self.head = (self.head + 1) % Self::CAPACITY;
            self.len -= 1;
            Some(value)
        }

        pub(super) fn is_valid(&self) -> bool {
            self.head < Self::CAPACITY && self.len <= Self::CAPACITY
        }
    }
}

const DATABUF_ROWS: usize = 32;
const DATABUF_COLS: usize = 8;
const DATABUF_SIZE: usize = DATABUF_ROWS * DATABUF_COLS;

/// Row (command - 0xD0) holding the key-interrupt enable byte / vector (0xE4).
const ROW_KEY_IRQ: usize = 0x14;
/// Row holding the two-byte keycode result (0xE6).
const ROW_KEYCODE: usize = 0x16;
/// Row holding the game-key result (0xE3, turbo).
const ROW_GAME_KEY: usize = 0x13;
/// Row holding the TV-control parameter (0xE7) and read result (0xE8 uses 0x18).
const ROW_TV_WRITE: usize = 0x17;
const ROW_TV_READ: usize = 0x18;
/// Row holding the CMT-control parameter (0xE9).
const ROW_CMT_CONTROL: usize = 0x19;
/// Row holding the CMT status (0xEA).
const ROW_CMT_STATUS: usize = 0x1A;
/// Row holding the CMT sensor bits (0xEB).
const ROW_CMT_SENSOR: usize = 0x1B;
const ROW_CALENDAR_SET: usize = 0x1C;
const ROW_CALENDAR_GET: usize = 0x1D;
const ROW_TIME_SET: usize = 0x1E;
const ROW_TIME_GET: usize = 0x1F;

/// Cassette transport commands (values written to the CMT-control mailbox).
const CMT_EJECT: u8 = 0x00;
const CMT_STOP: u8 = 0x01;
const CMT_PLAY: u8 = 0x02;
const CMT_FAST_FORWARD: u8 = 0x03;
const CMT_FAST_REWIND: u8 = 0x04;
const CMT_APSS_PLUS: u8 = 0x05;
const CMT_APSS_MINUS: u8 = 0x06;
const CMT_RECORD: u8 = 0x0A;

/// PPI port-B handshake bit for input-buffer-full (sub received from main).
pub const PORT_B_IBF: u8 = 0x40;
/// PPI port-B handshake bit for output-buffer-full (sub can send to main).
pub const PORT_B_OBF: u8 = 0x20;

/// Keyboard modifier virtual-key codes recognised in `key_down`/`key_up`.
const VK_SHIFT: u8 = 0x10;
const VK_CTRL: u8 = 0x11;
const VK_GRAPH: u8 = 0x12;
const VK_CAPS: u8 = 0x14;
const VK_KANA: u8 = 0x15;

/// Auto-repeat timing: first delay then repeat interval.
const REPEAT_FIRST_MICROS: u64 = 557_085;
const REPEAT_INTERVAL_MICROS: u64 = 61_165;

/// A cassette transport action requested by the sub-CPU, applied by the bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CassetteAction {
    /// Stop the transport.
    Stop,
    /// Play forward.
    Play,
    /// Wind forward at speed.
    FastForward,
    /// Wind backward at speed.
    Rewind,
    /// Automatic program search, forward.
    ApssForward,
    /// Automatic program search, backward.
    ApssBackward,
    /// Record.
    Record,
    /// Eject the tape.
    Eject,
}

impl save_state::StateEncode for CassetteAction {
    fn encode_state(&self, output: &mut Vec<u8>) {
        save_state::StateEncode::encode_state(&(*self as u8), output);
    }
}

impl save_state::StateDecode for CassetteAction {
    fn decode_state(
        decoder: &mut save_state::StateDecoder<'_>,
    ) -> Result<Self, save_state::StateDecodeError> {
        match <u8 as save_state::StateDecode>::decode_state(decoder)? {
            0 => Ok(Self::Stop),
            1 => Ok(Self::Play),
            2 => Ok(Self::FastForward),
            3 => Ok(Self::Rewind),
            4 => Ok(Self::ApssForward),
            5 => Ok(Self::ApssBackward),
            6 => Ok(Self::Record),
            7 => Ok(Self::Eject),
            _ => Err(save_state::StateDecodeError::InvalidTag),
        }
    }
}

save_state::runtime_state! {
/// In-line uPD1990A calendar/clock state (all fields are plain integers; the
/// mailbox exchanges BCD).
#[derive(Debug, Clone, Copy)]
struct RtcTime {
    year: u16,
    month: u8,
    day: u8,
    day_of_week: u8,
    hour: u8,
    minute: u8,
    second: u8,
}}

impl RtcTime {
    fn default_epoch() -> Self {
        // 2000-01-01 (Saturday) 00:00:00 as a deterministic power-on value.
        Self {
            year: 2000,
            month: 1,
            day: 1,
            day_of_week: 6,
            hour: 0,
            minute: 0,
            second: 0,
        }
    }

    fn days_in_month(year: u16, month: u8) -> u8 {
        match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 => {
                let leap = (year.is_multiple_of(4) && !year.is_multiple_of(100))
                    || year.is_multiple_of(400);
                if leap { 29 } else { 28 }
            }
            _ => 31,
        }
    }

    /// Recomputes the weekday from the calendar date (Sakamoto's methods).
    fn update_day_of_week(&mut self) {
        const MONTH_OFFSET: [u16; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
        let month = usize::from(self.month.clamp(1, 12));
        let year = self.year.saturating_sub(u16::from(self.month < 3));
        let sum = year + year / 4 - year / 100
            + year / 400
            + MONTH_OFFSET[month - 1]
            + u16::from(self.day);
        self.day_of_week = (sum % 7) as u8;
    }

    fn increment_one_second(&mut self) {
        self.second += 1;
        if self.second < 60 {
            return;
        }
        self.second = 0;
        self.minute += 1;
        if self.minute < 60 {
            return;
        }
        self.minute = 0;
        self.hour += 1;
        if self.hour < 24 {
            return;
        }
        self.hour = 0;
        self.day += 1;
        self.day_of_week = (self.day_of_week + 1) % 7;
        if self.day <= Self::days_in_month(self.year, self.month) {
            return;
        }
        self.day = 1;
        self.month += 1;
        if self.month <= 12 {
            return;
        }
        self.month = 1;
        self.year += 1;
    }
}

fn to_bcd(value: u8) -> u8 {
    ((value / 10) << 4) | (value % 10)
}

fn from_bcd(value: u8) -> u8 {
    ((value >> 4) * 10) + (value & 0x0F)
}

save_state::runtime_state! {
/// HLE sub-CPU state.
#[derive(Clone)]
pub struct SubHle {
    turbo: bool,
    keyboard_mode: X1KeyboardMode,
    cpu_clock_hz: u32,

    databuf: [u8; DATABUF_SIZE],
    datap: usize,
    mode: u8,
    inbuf: u8,
    outbuf: u8,
    ibf: bool,
    obf: bool,
    command_length: i32,
    data_length: i32,

    key_buf: KeyFifo,
    /// Which virtual keys are currently held (indexed by VK code), for the turbo
    /// game-key read (0xE3).
    key_pressed: [bool; 256],
    key_previous: u8,
    key_break: u8,
    key_shift: bool,
    key_ctrl: bool,
    key_graph: bool,
    key_caps_locked: bool,
    key_kana_locked: bool,
    repeat_deadline: Option<u64>,
    break_low: bool,

    clock: RtcTime,
    rtc_phase_reset: bool,

    play: bool,
    rec: bool,
    end_of_tape: bool,
    cassette_action: Option<CassetteAction>,

    interrupt_pending: bool,
    interrupt_enabled_line: bool,
}}

impl SubHle {
    /// Creates a sub-CPU HLE for the given model (turbo enables the game-key
    /// command and the keyboard mode switch) at `cpu_clock_hz`.
    pub fn new(turbo: bool, cpu_clock_hz: u32) -> Self {
        let mut sub = Self {
            turbo,
            keyboard_mode: X1KeyboardMode::ModeA,
            cpu_clock_hz,
            databuf: [0; DATABUF_SIZE],
            datap: 0,
            mode: 0,
            inbuf: 0,
            outbuf: 0,
            ibf: false,
            obf: false,
            command_length: 0,
            data_length: 0,
            key_buf: KeyFifo::new(),
            key_pressed: [false; 256],
            key_previous: 0,
            key_break: 0,
            key_shift: false,
            key_ctrl: false,
            key_graph: false,
            key_caps_locked: false,
            key_kana_locked: false,
            repeat_deadline: None,
            break_low: false,
            clock: RtcTime::default_epoch(),
            rtc_phase_reset: false,
            play: false,
            rec: false,
            end_of_tape: false,
            cassette_action: None,
            interrupt_pending: false,
            interrupt_enabled_line: true,
        };
        sub.reset();
        sub
    }

    /// Captures the complete mailbox, keyboard, RTC, and cassette state.
    pub fn capture_state(&self) -> Self {
        self.clone()
    }

    /// Restores complete state while retaining immutable model configuration.
    pub fn restore_state(&mut self, state: Self) -> Result<(), save_state::StateValidationError> {
        if state.turbo != self.turbo
            || state.cpu_clock_hz != self.cpu_clock_hz
            || state.datap >= DATABUF_SIZE
            || !state.key_buf.is_valid()
            || !(1..=12).contains(&state.clock.month)
            || state.clock.day == 0
            || state.clock.day > RtcTime::days_in_month(state.clock.year, state.clock.month)
            || state.clock.hour >= 24
            || state.clock.minute >= 60
            || state.clock.second >= 60
        {
            return Err(save_state::StateValidationError::new(
                "X1 sub-controller state is invalid",
            ));
        }
        *self = state;
        Ok(())
    }

    /// Resets the mailbox, key buffer and handshake to power-on state.
    pub fn reset(&mut self) {
        self.databuf = [0; DATABUF_SIZE];
        self.databuf[ROW_KEYCODE * DATABUF_COLS] = 0xFF;
        self.datap = 0;
        self.mode = 0;
        self.command_length = 0;
        self.data_length = 0;
        self.ibf = false;
        self.obf = true;
        self.key_buf.clear();
        self.key_pressed = [false; 256];
        self.key_previous = 0;
        self.key_break = 0;
        self.key_shift = false;
        self.key_ctrl = false;
        self.key_graph = false;
        self.key_caps_locked = false;
        self.key_kana_locked = false;
        self.repeat_deadline = None;
        self.break_low = false;
        self.interrupt_enabled_line = true;
        self.interrupt_pending = false;
        self.rtc_phase_reset = false;
    }

    /// Selects the turbo keyboard's mode switch position (A or B).
    pub fn set_keyboard_mode(&mut self, mode: X1KeyboardMode) {
        self.keyboard_mode = mode;
    }

    /// Whether the turbo keyboard's mode switch is in position B.
    fn mode_b(&self) -> bool {
        self.turbo && self.keyboard_mode == X1KeyboardMode::ModeB
    }

    /// Sets the calendar/clock from a host time.
    #[allow(clippy::too_many_arguments)]
    pub fn set_host_time(
        &mut self,
        year: u16,
        month: u8,
        day: u8,
        day_of_week: u8,
        hour: u8,
        minute: u8,
        second: u8,
    ) {
        self.clock = RtcTime {
            year,
            month,
            day,
            day_of_week,
            hour,
            minute,
            second,
        };
    }

    fn cell(&self, row: usize, col: usize) -> u8 {
        self.databuf[row * DATABUF_COLS + col]
    }

    fn cell_mut(&mut self, row: usize, col: usize) -> &mut u8 {
        &mut self.databuf[row * DATABUF_COLS + col]
    }

    /// The state of PPI port-B handshake bits contributed by the sub-CPU.
    pub fn port_b_handshake(&self) -> u8 {
        let mut bits = 0;
        if self.ibf {
            bits |= PORT_B_IBF;
        }
        if self.obf {
            bits |= PORT_B_OBF;
        }
        bits
    }

    /// Whether the break line is currently asserted low (port-B bit 0 reads 0).
    pub fn break_low(&self) -> bool {
        self.break_low
    }

    /// Main CPU writes a command / parameter byte to the mailbox (`0x1900`).
    pub fn write_mailbox(&mut self, value: u8) {
        self.inbuf = value;
        self.ibf = true;
    }

    /// Main CPU reads a result byte / interrupt vector from the mailbox.
    pub fn read_mailbox(&mut self) -> u8 {
        self.obf = true;
        self.interrupt_pending = false;
        self.outbuf
    }

    /// Whether a keyboard interrupt is pending (and enabled through IEI).
    pub fn key_irq_pending(&self) -> bool {
        self.interrupt_pending && self.interrupt_enabled_line
    }

    /// Drives the IEI input from the daisy chain: while a higher-priority
    /// device has an interrupt under service, the sub-CPU holds back the
    /// keyboard interrupt and keeps the mailbox free.
    pub fn set_interrupt_enabled_line(&mut self, enabled: bool) {
        self.interrupt_enabled_line = enabled;
    }

    /// Acknowledges the keyboard interrupt. The CPU's interrupt-acknowledge
    /// cycle reads the mailbox, consuming the vector byte that was latched
    /// into it when the interrupt was raised; the two key-data bytes follow
    /// through the mailbox on later polls.
    pub fn acknowledge_key_irq(&mut self) -> u8 {
        self.read_mailbox()
    }

    /// Takes any pending cassette transport action requested by the sub-CPU.
    pub fn take_cassette_action(&mut self) -> Option<CassetteAction> {
        self.cassette_action.take()
    }

    /// Marks a tape as loaded / removed (PLAY button engaged). The CMT status
    /// the main CPU reads follows: a loaded tape sits at STOP, none at EJECT.
    pub fn set_tape_playable(&mut self, playable: bool) {
        self.play = playable;
        self.rec = false;
        *self.cell_mut(ROW_CMT_STATUS, 0) = if playable { CMT_STOP } else { CMT_EJECT };
    }

    /// Reports that the transport stopped on its own (end or top of tape), as
    /// the deck's remote line dropping does on the real machine.
    pub fn notify_transport_stopped(&mut self) {
        if self.play || self.rec {
            *self.cell_mut(ROW_CMT_STATUS, 0) = CMT_STOP;
        }
    }

    /// Updates the end-of-tape sensor.
    pub fn set_tape_end(&mut self, end: bool) {
        self.end_of_tape = end;
    }

    /// Advances the RTC by one second (driven by a 1 Hz event).
    pub fn tick_one_second(&mut self) {
        self.clock.increment_one_second();
    }

    /// Takes the flag raised by the set-time command; the caller restarts the
    /// 1 Hz tick so the first second elapses fully after the time was set.
    pub fn take_rtc_phase_reset(&mut self) -> bool {
        std::mem::take(&mut self.rtc_phase_reset)
    }

    /// Steps the mailbox once (driven by the periodic `SubPoll` event). `now` is
    /// the current CPU cycle, used for auto-repeat timing.
    pub fn poll(&mut self, now: u64) {
        // Auto-repeat: re-inject the last key when its deadline elapses.
        if let Some(deadline) = self.repeat_deadline
            && now >= deadline
        {
            self.repeat_deadline = None;
            let key = self.key_previous;
            self.key_down(key, true, now);
        }

        if self.ibf {
            if self.command_length > 0 {
                self.databuf[self.datap] = self.inbuf;
                self.datap += 1;
                self.command_length -= 1;
            } else {
                self.mode = self.inbuf;
                if (0xD0..=0xD7).contains(&self.mode) {
                    self.command_length = 6;
                    self.datap = (self.mode as usize - 0xD0) * DATABUF_COLS;
                } else if (0xE3..=0xEF).contains(&self.mode) {
                    self.command_length =
                        i32::from(COMMAND_PARAMETER_LENGTH[self.mode as usize - 0xE3]);
                    self.datap = (self.mode as usize - 0xD0) * DATABUF_COLS;
                }
            }
            if self.command_length == 0 {
                self.process_command();
            }
            self.ibf = false;
            self.obf = true;
            if self.command_length != 0 || self.data_length != 0 {
                return;
            }
        }

        if self.obf {
            if self.data_length > 0 {
                self.outbuf = self.databuf[self.datap];
                self.datap += 1;
                self.obf = false;
                self.data_length -= 1;
            } else if !self.key_buf.is_empty()
                && self.cell(ROW_KEY_IRQ, 0) != 0
                && !self.interrupt_pending
                && self.interrupt_enabled_line
            {
                // Raise the keyboard interrupt: latch the programmed vector
                // byte into the mailbox. The interrupt-acknowledge cycle (or a
                // polling read of the mailbox) consumes it, then the two key-
                // data bytes queued here follow on later polls.
                self.outbuf = self.cell(ROW_KEY_IRQ, 0);
                self.obf = false;
                self.interrupt_pending = true;
                self.mode = 0xE6;
                self.process_command();
            }
        }
    }

    fn process_command(&mut self) {
        if (0xD0..0xF0).contains(&self.mode) {
            self.datap = (self.mode as usize - 0xD0) * DATABUF_COLS;
        }
        self.data_length = 0;

        match self.mode {
            0x00 => {}
            0xD0..=0xD7 => {}
            0xD8..=0xDF => {
                self.datap = (self.mode as usize - 0xD8) * DATABUF_COLS;
                self.data_length = 6;
            }
            0xE3 if self.turbo => {
                // Game key read (turbo): three bytes of the direct key matrix
                // for games that scan the keyboard for controls. Only the
                // mode-B keyboard reports the matrix; mode A returns zeros.
                let [byte0, byte1, byte2] = if self.mode_b() {
                    self.game_key_bytes()
                } else {
                    [0, 0, 0]
                };
                *self.cell_mut(ROW_GAME_KEY, 0) = byte0;
                *self.cell_mut(ROW_GAME_KEY, 1) = byte1;
                *self.cell_mut(ROW_GAME_KEY, 2) = byte2;
                self.data_length = 3;
            }
            0xE4 => {
                self.drain_all_keys();
                self.interrupt_pending = false;
            }
            0xE6 => {
                if self.cell(ROW_KEY_IRQ, 0) == 0 {
                    self.drain_all_keys();
                } else if let Some(entry) = self.key_buf.pop() {
                    *self.cell_mut(ROW_KEYCODE, 0) = (entry & 0xFF) as u8;
                    *self.cell_mut(ROW_KEYCODE, 1) = (entry >> 8) as u8;
                }
                if self.turbo {
                    let low = self.cell(ROW_KEYCODE, 0) & !0x1F;
                    *self.cell_mut(ROW_KEYCODE, 0) = low | (self.key_low() & 0x1F);
                }
                self.data_length = 2;
            }
            0xE7 => {
                let value = self.cell(ROW_TV_WRITE, 0);
                *self.cell_mut(ROW_TV_READ, 0) = value;
            }
            0xE8 => {
                self.data_length = 1;
            }
            0xE9 => {
                self.process_cassette_command();
            }
            0xEA => {
                self.data_length = 1;
            }
            0xEB => {
                let sensor = (if self.play {
                    2
                } else if self.rec {
                    6
                } else {
                    0
                }) | u8::from((self.play || self.rec) && !self.end_of_tape);
                *self.cell_mut(ROW_CMT_SENSOR, 0) = sensor;
                self.data_length = 1;
            }
            0xEC => {
                // Two-digit years span 1970-2069.
                let year = u16::from(from_bcd(self.cell(ROW_CALENDAR_SET, 0)));
                self.clock.year = if year < 70 {
                    year + 2000
                } else if year < 100 {
                    year + 1900
                } else {
                    year
                };
                let month_dow = self.cell(ROW_CALENDAR_SET, 1);
                if (month_dow & 0xF0) != 0 {
                    self.clock.month = month_dow >> 4;
                }
                let day = self.cell(ROW_CALENDAR_SET, 2);
                if day != 0 {
                    self.clock.day = from_bcd(day);
                }
                self.clock.update_day_of_week();
            }
            0xED => {
                *self.cell_mut(ROW_CALENDAR_GET, 0) = to_bcd((self.clock.year % 100) as u8);
                *self.cell_mut(ROW_CALENDAR_GET, 1) =
                    (self.clock.month << 4) | self.clock.day_of_week;
                *self.cell_mut(ROW_CALENDAR_GET, 2) = to_bcd(self.clock.day);
                self.data_length = 3;
            }
            0xEE => {
                self.clock.hour = from_bcd(self.cell(ROW_TIME_SET, 0));
                self.clock.minute = from_bcd(self.cell(ROW_TIME_SET, 1) & 0x7F);
                self.clock.second = from_bcd(self.cell(ROW_TIME_SET, 2) & 0x7F);
                self.rtc_phase_reset = true;
            }
            0xEF => {
                *self.cell_mut(ROW_TIME_GET, 0) = to_bcd(self.clock.hour);
                *self.cell_mut(ROW_TIME_GET, 1) = to_bcd(self.clock.minute);
                *self.cell_mut(ROW_TIME_GET, 2) = to_bcd(self.clock.second);
                self.data_length = 3;
            }
            _ => {}
        }
    }

    fn process_cassette_command(&mut self) {
        let requested = self.cell(ROW_CMT_CONTROL, 0);
        if requested == self.cell(ROW_CMT_STATUS, 0) {
            return;
        }
        let mut new_status = requested;
        let mut action = None;
        match requested {
            CMT_EJECT => action = Some(CassetteAction::Eject),
            CMT_STOP => action = Some(CassetteAction::Stop),
            CMT_PLAY => {
                if self.play {
                    action = Some(CassetteAction::Play);
                } else if self.rec {
                    new_status = CMT_STOP;
                } else {
                    new_status = CMT_EJECT;
                }
            }
            CMT_FAST_FORWARD => {
                if self.play {
                    action = Some(CassetteAction::FastForward);
                } else if self.rec {
                    new_status = CMT_STOP;
                } else {
                    new_status = CMT_EJECT;
                }
            }
            CMT_FAST_REWIND => {
                if self.play {
                    action = Some(CassetteAction::Rewind);
                } else if self.rec {
                    new_status = CMT_STOP;
                } else {
                    new_status = CMT_EJECT;
                }
            }
            CMT_APSS_PLUS | CMT_APSS_MINUS => {
                if self.play {
                    action = Some(if requested == CMT_APSS_PLUS {
                        CassetteAction::ApssForward
                    } else {
                        CassetteAction::ApssBackward
                    });
                    new_status = CMT_STOP;
                } else if self.rec {
                    new_status = CMT_STOP;
                } else {
                    new_status = CMT_EJECT;
                }
            }
            CMT_RECORD => {
                if self.play {
                    new_status = CMT_STOP;
                } else if self.rec {
                    action = Some(CassetteAction::Record);
                } else {
                    new_status = CMT_EJECT;
                }
            }
            _ => {}
        }
        self.cassette_action = action;
        *self.cell_mut(ROW_CMT_STATUS, 0) = new_status;
    }

    /// Assembles the three game-key bytes from the live key matrix, following the
    /// turbo sub-CPU's direct-scan bit layout.
    fn game_key_bytes(&self) -> [u8; 3] {
        let bit = |code: u8, mask: u8| -> u8 {
            if self.key_pressed[code as usize] {
                mask
            } else {
                0
            }
        };
        let byte0 = bit(0x51, 0x80) // Q
            | bit(0x57, 0x40) // W
            | bit(0x45, 0x20) // E
            | bit(0x41, 0x10) // A
            | bit(0x44, 0x08) // D
            | bit(0x5A, 0x04) // Z
            | bit(0x58, 0x02) // X
            | bit(0x43, 0x01); // C
        let byte1 = bit(0x67, 0x80) // numpad 7
            | bit(0x64, 0x40) // numpad 4
            | bit(0x61, 0x20) // numpad 1
            | bit(0x68, 0x10) // numpad 8
            | bit(0x62, 0x08) // numpad 2
            | bit(0x69, 0x04) // numpad 9
            | bit(0x66, 0x02) // numpad 6
            | bit(0x63, 0x01); // numpad 3
        let byte2 = bit(0x1B, 0x80) // ESC
            | bit(0x61, 0x40) // numpad 1
            | bit(0x6D, 0x20) // numpad -
            | bit(0x6B, 0x10) // numpad +
            | bit(0x6A, 0x08) // numpad *
            | bit(0x09, 0x04) // TAB
            | bit(0x20, 0x02) // space
            | bit(0x0D, 0x01); // return
        [byte0, byte1, byte2]
    }

    fn drain_all_keys(&mut self) {
        while let Some(entry) = self.key_buf.pop() {
            *self.cell_mut(ROW_KEYCODE, 0) = (entry & 0xFF) as u8;
            *self.cell_mut(ROW_KEYCODE, 1) = (entry >> 8) as u8;
        }
    }

    fn micros_to_cycles(&self, micros: u64) -> u64 {
        (u64::from(self.cpu_clock_hz) * micros) / 1_000_000
    }

    /// Handles a key press (`code` is a host virtual-key code). `now` is the
    /// current CPU cycle, for auto-repeat scheduling.
    pub fn key_down(&mut self, code: u8, repeat: bool, now: u64) {
        self.key_pressed[code as usize] = true;
        match code {
            VK_SHIFT => self.key_shift = true,
            VK_CTRL => self.key_ctrl = true,
            VK_GRAPH => self.key_graph = true,
            VK_CAPS => self.key_caps_locked = !self.key_caps_locked,
            VK_KANA => self.key_kana_locked = !self.key_kana_locked,
            _ => {}
        }

        let key = self.resolve_key(code, repeat);
        if (key & 0xFF00) != 0 {
            let entry = key & !0x40;
            if self.cell(ROW_KEY_IRQ, 0) == 0 {
                self.key_buf.clear();
            }
            self.key_buf.push(entry);
            self.key_previous = code;

            let is_function_key = (0x70..=0x87).contains(&code);
            if is_function_key {
                self.repeat_deadline = None;
            } else if repeat {
                self.repeat_deadline = Some(now + self.micros_to_cycles(REPEAT_INTERVAL_MICROS));
            } else {
                self.repeat_deadline = Some(now + self.micros_to_cycles(REPEAT_FIRST_MICROS));
            }

            if (entry >> 8) == 3 {
                self.break_low = true;
                self.key_break = code;
            }
        } else if self.turbo && self.key_previous == 0 && is_low_byte_key(code) {
            self.key_buf.push(0xFF);
        }
    }

    /// Handles a key release (`code` is a host virtual-key code).
    pub fn key_up(&mut self, code: u8) {
        self.key_pressed[code as usize] = false;
        match code {
            VK_SHIFT => self.key_shift = false,
            VK_CTRL => self.key_ctrl = false,
            VK_GRAPH => self.key_graph = false,
            _ => {}
        }

        let matches_previous = code == self.key_previous
            || (self.turbo && self.key_previous == 0 && is_low_byte_key(code));
        if matches_previous {
            if self.cell(ROW_KEY_IRQ, 0) == 0 {
                self.key_buf.clear();
            }
            self.key_buf.push(0xFF);
            self.key_previous = 0;
            self.repeat_deadline = None;
        }
        if code == self.key_break {
            self.break_low = false;
            self.key_break = 0;
        }
    }

    fn key_low(&self) -> u8 {
        let mut low = 0xFF;
        if self.key_ctrl {
            low &= !0x01;
        }
        if self.key_shift {
            low &= !0x02;
        }
        if self.key_kana_locked {
            low &= !0x04;
        }
        if self.key_caps_locked {
            low &= !0x08;
        }
        if self.key_graph {
            low &= !0x10;
        }
        low
    }

    fn resolve_key(&self, code: u8, repeat: bool) -> u16 {
        let mut low = self.key_low();
        if repeat {
            low &= !0x20;
        }
        if (0x60..=0x74).contains(&code) {
            low &= !0x80;
        }

        let index = code as usize;
        let high = if self.key_kana_locked {
            if (low & 0x02) == 0 {
                if self.mode_b() {
                    KEYCODE_KANA_SHIFT_B[index]
                } else {
                    KEYCODE_KANA_SHIFT[index]
                }
            } else if self.mode_b() {
                KEYCODE_KANA_B[index]
            } else {
                KEYCODE_KANA[index]
            }
        } else if (low & 0x01) == 0 {
            KEYCODE_CTRL[index]
        } else if (low & 0x10) == 0 {
            KEYCODE_GRAPH[index]
        } else {
            let mut high = if (low & 0x02) == 0 {
                KEYCODE_SHIFT[index]
            } else {
                KEYCODE_NORMAL[index]
            };
            if self.key_caps_locked && (0x41..=0x5A).contains(&code) {
                high ^= 0x20;
            }
            high
        };

        if !self.turbo && high == 0 {
            low = 0xFF;
        }
        u16::from(low) | (u16::from(high) << 8)
    }
}

/// shift, ctrl, graph, caps, kana.
fn is_low_byte_key(code: u8) -> bool {
    (0x10..=0x12).contains(&code) || code == 0x14 || code == 0x15
}

/// Parameter-byte counts for sub-CPU commands 0xE3..0xEF.
const COMMAND_PARAMETER_LENGTH: [u8; 13] = [0, 1, 0, 0, 1, 0, 1, 0, 0, 3, 0, 3, 0];

#[cfg(test)]
mod tests {
    use super::*;

    const CLOCK: u32 = 4_000_000;

    fn run_until_idle(sub: &mut SubHle, now: u64) {
        for _ in 0..8 {
            sub.poll(now);
        }
    }

    #[test]
    fn keydata_command_returns_modifier_then_keycode() {
        let mut sub = SubHle::new(false, CLOCK);
        // Press 'A' (VK 0x41) with no modifiers. Key IRQ stays disabled so the
        // keydata command drains the buffer directly.
        sub.key_down(0x41, false, 0);

        // Read keydata (0xE6, no params, 2 result bytes: modifier then keycode).
        sub.write_mailbox(0xE6);
        run_until_idle(&mut sub, 0);
        let modifier = sub.read_mailbox();
        run_until_idle(&mut sub, 0);
        let keycode = sub.read_mailbox();
        assert_ne!(modifier, 0x00);
        assert_eq!(keycode, 0x61); // 'a' with the reference tables
    }

    #[test]
    fn shift_modifies_the_keycode() {
        let mut sub = SubHle::new(false, CLOCK);
        sub.key_down(VK_SHIFT, false, 0);
        sub.key_down(0x41, false, 0); // shift + A
        sub.write_mailbox(0xE6);
        run_until_idle(&mut sub, 0);
        let _modifier = sub.read_mailbox();
        run_until_idle(&mut sub, 0);
        let keycode = sub.read_mailbox();
        assert_eq!(keycode, 0x41); // 'A'
    }

    #[test]
    fn rtc_round_trips_through_bcd() {
        let mut sub = SubHle::new(false, CLOCK);
        sub.set_host_time(2023, 11, 25, 6, 12, 34, 56);
        // Get time (0xEF): 3 result bytes hh mm ss in BCD.
        sub.write_mailbox(0xEF);
        run_until_idle(&mut sub, 0);
        let hour = sub.read_mailbox();
        run_until_idle(&mut sub, 0);
        let minute = sub.read_mailbox();
        run_until_idle(&mut sub, 0);
        let second = sub.read_mailbox();
        assert_eq!(hour, 0x12);
        assert_eq!(minute, 0x34);
        assert_eq!(second, 0x56);
    }

    #[test]
    fn one_second_tick_advances_the_clock() {
        let mut sub = SubHle::new(false, CLOCK);
        sub.set_host_time(2000, 1, 1, 6, 23, 59, 59);
        sub.tick_one_second();
        sub.write_mailbox(0xEF);
        run_until_idle(&mut sub, 0);
        assert_eq!(sub.read_mailbox(), 0x00); // hour rolled to 0
    }

    #[test]
    fn key_irq_vector_flows_through_the_mailbox() {
        let mut sub = SubHle::new(false, CLOCK);
        // Enable the key IRQ with vector 0x86 (command 0xE4, one parameter).
        sub.write_mailbox(0xE4);
        run_until_idle(&mut sub, 0);
        sub.write_mailbox(0x86);
        run_until_idle(&mut sub, 0);

        sub.key_down(0x41, false, 0);
        sub.poll(0);
        assert!(sub.key_irq_pending());
        // The mailbox holds the vector until the interrupt-acknowledge cycle
        // consumes it.
        assert_eq!(sub.port_b_handshake() & PORT_B_OBF, 0);
        assert_eq!(sub.acknowledge_key_irq(), 0x86);
        assert!(!sub.key_irq_pending());

        // The two key-data bytes follow on later polls.
        sub.poll(0);
        let modifier = sub.read_mailbox();
        sub.poll(0);
        let keycode = sub.read_mailbox();
        assert_ne!(modifier & 0x02, 0); // shift released
        assert_eq!(keycode, 0x61); // 'a'
    }

    #[test]
    fn cmt_status_follows_the_tape_state() {
        fn read_status(sub: &mut SubHle) -> u8 {
            sub.write_mailbox(0xEA);
            run_until_idle(sub, 0);
            sub.read_mailbox()
        }

        let mut sub = SubHle::new(false, CLOCK);
        assert_eq!(read_status(&mut sub), CMT_EJECT);

        sub.set_tape_playable(true);
        assert_eq!(read_status(&mut sub), CMT_STOP);

        sub.write_mailbox(0xE9);
        run_until_idle(&mut sub, 0);
        sub.write_mailbox(CMT_PLAY);
        run_until_idle(&mut sub, 0);
        assert_eq!(sub.take_cassette_action(), Some(CassetteAction::Play));
        assert_eq!(read_status(&mut sub), CMT_PLAY);

        // The transport running against the end of the tape reports STOP.
        sub.notify_transport_stopped();
        assert_eq!(read_status(&mut sub), CMT_STOP);

        sub.set_tape_playable(false);
        assert_eq!(read_status(&mut sub), CMT_EJECT);
    }

    #[test]
    fn set_calendar_windows_the_year_and_recomputes_the_weekday() {
        let mut sub = SubHle::new(false, CLOCK);
        // Set calendar (0xEC): year 87 -> 1987; 1987-07-21 was a Tuesday.
        sub.write_mailbox(0xEC);
        run_until_idle(&mut sub, 0);
        for parameter in [0x87, 0x70, 0x21] {
            sub.write_mailbox(parameter);
            run_until_idle(&mut sub, 0);
        }

        // Get calendar (0xED): yy, (month << 4) | day_of_week, dd.
        sub.write_mailbox(0xED);
        run_until_idle(&mut sub, 0);
        assert_eq!(sub.read_mailbox(), 0x87);
        run_until_idle(&mut sub, 0);
        assert_eq!(sub.read_mailbox(), (7 << 4) | 2);
        run_until_idle(&mut sub, 0);
        assert_eq!(sub.read_mailbox(), 0x21);
    }

    #[test]
    fn set_time_requests_an_rtc_phase_reset() {
        let mut sub = SubHle::new(false, CLOCK);
        assert!(!sub.take_rtc_phase_reset());
        sub.write_mailbox(0xEE);
        run_until_idle(&mut sub, 0);
        for parameter in [0x12, 0x34, 0x56] {
            sub.write_mailbox(parameter);
            run_until_idle(&mut sub, 0);
        }
        assert!(sub.take_rtc_phase_reset());
        assert!(!sub.take_rtc_phase_reset());
    }

    #[test]
    fn game_key_read_needs_a_turbo_mode_b_keyboard() {
        // The base X1 does not answer the game-key command at all.
        let mut sub = SubHle::new(false, CLOCK);
        sub.key_down(0x51, false, 0); // Q
        sub.write_mailbox(0xE3);
        run_until_idle(&mut sub, 0);
        assert_eq!(sub.port_b_handshake() & PORT_B_OBF, PORT_B_OBF);

        // A turbo mode-A keyboard reads zeros.
        let mut sub = SubHle::new(true, CLOCK);
        sub.key_down(0x51, false, 0);
        sub.write_mailbox(0xE3);
        run_until_idle(&mut sub, 0);
        assert_eq!(sub.read_mailbox(), 0x00);

        // A turbo mode-B keyboard reads the live matrix.
        let mut sub = SubHle::new(true, CLOCK);
        sub.set_keyboard_mode(X1KeyboardMode::ModeB);
        sub.key_down(0x51, false, 0);
        sub.write_mailbox(0xE3);
        run_until_idle(&mut sub, 0);
        assert_eq!(sub.read_mailbox() & 0x80, 0x80);
    }

    #[test]
    fn kana_tables_follow_the_keyboard_mode() {
        fn kana_keycode(sub: &mut SubHle) -> u8 {
            sub.key_down(VK_KANA, false, 0);
            sub.key_down(0x41, false, 0); // kana + A
            sub.write_mailbox(0xE6);
            run_until_idle(sub, 0);
            let _modifier = sub.read_mailbox();
            run_until_idle(sub, 0);
            sub.read_mailbox()
        }

        let mut sub = SubHle::new(true, CLOCK);
        assert_eq!(kana_keycode(&mut sub), 0xC1);

        let mut sub = SubHle::new(true, CLOCK);
        sub.set_keyboard_mode(X1KeyboardMode::ModeB);
        assert_eq!(kana_keycode(&mut sub), 0xBB);
    }

    #[test]
    fn cassette_play_requires_a_loaded_tape() {
        let mut sub = SubHle::new(false, CLOCK);
        // Without a tape, PLAY collapses to EJECT and requests no action.
        sub.write_mailbox(0xE9);
        run_until_idle(&mut sub, 0);
        sub.write_mailbox(CMT_PLAY);
        run_until_idle(&mut sub, 0);
        assert_eq!(sub.take_cassette_action(), None);

        // With a tape loaded, PLAY engages the transport.
        sub.set_tape_playable(true);
        sub.write_mailbox(0xE9);
        run_until_idle(&mut sub, 0);
        sub.write_mailbox(CMT_PLAY);
        run_until_idle(&mut sub, 0);
        assert_eq!(sub.take_cassette_action(), Some(CassetteAction::Play));
    }
}
