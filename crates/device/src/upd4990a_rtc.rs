//! uPD4990A serial calendar clock.
//!
//! The chip sees one byte with its input lines: C0-C2 in bits 0-2, STB in
//! bit 3, CLK in bit 4 and DATA IN in bit 5 (PC-98 port 0x20). A write with
//! STB and CLK low latches C0-C2 and writes DATA IN into the shift register
//! cell at the current bit position. A rising STB edge executes the latched
//! command. A rising CLK edge steps the bit position and presents the next
//! cell on DATA OUT.
//!
//! The shift register holds 64 cells in eight bytes. Bytes 2 to 7 hold the
//! calendar in BCD: year, month and day of week, day, hour, minute and second.
//! Byte 1 holds the status and byte 0 is padding. The register shift command
//! starts at the least significant bit of the seconds byte. Every CLK edge then
//! steps one bit toward the most significant bit of the same byte and from
//! there to the next lower byte.
//!
//! With C0-C2 all high the chip takes its command from a serial command
//! register. A write with STB and CLK low places DATA IN in bit 4 of that
//! register and every CLK edge shifts it one bit toward bit 0.
//!
//! The host provides the time on every port write. The chip keeps no time
//! counter of its own.

/// Mask of the C0-C2 command lines.
const LINE_COMMAND: u8 = 0x07;

/// STB line.
const LINE_STROBE: u8 = 0x08;

/// CLK line.
const LINE_CLOCK: u8 = 0x10;

/// DATA IN line.
const LINE_DATA_IN: u8 = 0x20;

/// Command code on C0-C2 that selects the serial command register.
const SERIAL_MODE_SELECT: u8 = 0x07;

/// Command: register hold.
const COMMAND_REGISTER_HOLD: u8 = 0x0;

/// Command: register shift.
const COMMAND_REGISTER_SHIFT: u8 = 0x1;

/// Command: time set and counter hold.
const COMMAND_TIME_SET: u8 = 0x2;

/// Command: time read.
const COMMAND_TIME_READ: u8 = 0x3;

/// Mask of the command bits of the serial command register.
const SERIAL_COMMAND_MASK: u8 = 0x0F;

/// Bit of the serial command register that receives DATA IN.
const SERIAL_COMMAND_INPUT: u8 = 0x10;

/// Shift register length in bytes.
const REGISTER_BYTES: usize = 8;

/// Bit position of the first cell that the register shift command presents.
const FIRST_SHIFT_POSITION: usize = REGISTER_BYTES * 8 - 1;

/// Shift register byte that receives the status of a time read.
const STATUS_BYTE: usize = 1;

/// Status of a completed time read.
const STATUS_TIME_VALID: u8 = 0x01;

/// First shift register byte of the calendar.
const CALENDAR_FIRST_BYTE: usize = 2;

save_state::runtime_state! {
/// uPD4990A state.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Upd4990aState {
    /// Shift register cells.
    pub register: [u8; REGISTER_BYTES],
    /// Bit position of the current shift register cell (0 to 63).
    pub bit_position: usize,
    /// Whether the register shift command is active.
    pub shifting: bool,
    /// DATA OUT line (0 or 1).
    pub data_out: u8,
    /// C0-C2 lines latched by the last write with STB and CLK low.
    pub command_lines: u8,
    /// Serial command register. Bits 3-0 hold the command, bit 4 takes DATA IN.
    pub serial_command: u8,
    /// Input lines of the last port write.
    pub input_lines: u8,
}}

/// uPD4990A serial calendar clock.
#[derive(Default)]
pub struct Upd4990aRtc {
    /// Embedded state for save/restore.
    pub state: Upd4990aState,
}

impl_simple_state_accessors!(Upd4990aRtc, Upd4990aState);

/// Phase of the serial interface selected by the STB and CLK lines of a write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputPhase {
    /// STB is high. A rising STB edge executes the latched command.
    Strobe,
    /// STB is low and CLK is high. A rising CLK edge steps the registers.
    Clock,
    /// STB and CLK are low. The chip takes C0-C2 and DATA IN.
    Data,
}

impl InputPhase {
    /// Returns the phase of the input lines `lines`.
    fn of(lines: u8) -> Self {
        if lines & LINE_STROBE != 0 {
            Self::Strobe
        } else if lines & LINE_CLOCK != 0 {
            Self::Clock
        } else {
            Self::Data
        }
    }
}

/// Command that a rising STB edge executes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    /// Stops the register shift.
    RegisterHold,
    /// Starts the register shift at the first cell.
    RegisterShift,
    /// Stops the register shift. The host time stays authoritative.
    TimeSet,
    /// Loads the host time into the shift register.
    TimeRead,
    /// A command without an effect in this model.
    Other,
}

impl Command {
    /// Decodes a command code of C0-C2 or of the serial command register.
    fn decode(code: u8) -> Self {
        match code {
            COMMAND_REGISTER_HOLD => Self::RegisterHold,
            COMMAND_REGISTER_SHIFT => Self::RegisterShift,
            COMMAND_TIME_SET => Self::TimeSet,
            COMMAND_TIME_READ => Self::TimeRead,
            _ => Self::Other,
        }
    }
}

impl Upd4990aState {
    /// Returns whether C0-C2 select the serial command register.
    fn serial_mode(&self) -> bool {
        self.command_lines == SERIAL_MODE_SELECT
    }

    /// Returns the shift register bit at `position`. Position 0 is bit 7 of
    /// byte 0 and every following position is the next lower bit.
    fn bit_at(&self, position: usize) -> u8 {
        let byte = self.register[position / 8];
        (byte >> (7 - position % 8)) & 1
    }

    /// Sets or clears the shift register bit at `position`.
    fn set_bit_at(&mut self, position: usize, set: bool) {
        let mask = 0x80 >> (position % 8);
        let byte = &mut self.register[position / 8];
        *byte = if set { *byte | mask } else { *byte & !mask };
    }

    /// Presents the cell at `position` on DATA OUT.
    fn present(&mut self, position: usize) {
        self.data_out = self.bit_at(position);
    }
}

impl Upd4990aRtc {
    /// Creates a new uPD4990A in its reset state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the DATA OUT line (0 or 1).
    pub fn cdat(&self) -> u8 {
        self.state.data_out
    }

    /// Writes a BCD time in the layout `[year, month << 4 | day_of_week, day,
    /// hour, minute, second]` into the calendar bytes of the shift register.
    pub fn load_shift_register(&mut self, time_bcd: &[u8; 6]) {
        self.state.register[CALENDAR_FIRST_BYTE..].copy_from_slice(time_bcd);
    }

    /// Applies a write of the chip input lines.
    ///
    /// `host_time_bcd` is the current time in the layout
    /// `[year, month << 4 | day_of_week, day, hour, minute, second]`. The time
    /// read command copies it into the shift register.
    pub fn write_port(&mut self, value: u8, host_time_bcd: &[u8; 6]) {
        let previous = std::mem::replace(&mut self.state.input_lines, value);
        let raised = |line: u8| value & line != 0 && previous & line == 0;
        match InputPhase::of(value) {
            InputPhase::Strobe if raised(LINE_STROBE) => {
                self.execute(self.latched_command(), host_time_bcd);
            }
            InputPhase::Clock if raised(LINE_CLOCK) => self.clock_edge(),
            InputPhase::Data => self.data_phase(value),
            InputPhase::Strobe | InputPhase::Clock => {}
        }
    }

    /// Returns the command that a rising STB edge executes. With C0-C2 all
    /// high the serial command register supplies it.
    fn latched_command(&self) -> Command {
        let code = if self.state.serial_mode() {
            self.state.serial_command & SERIAL_COMMAND_MASK
        } else {
            self.state.command_lines
        };
        Command::decode(code)
    }

    /// Executes `command`.
    ///
    /// The register shift and the time read present the first shift cell on
    /// DATA OUT. The time read keeps the bit position.
    fn execute(&mut self, command: Command, host_time_bcd: &[u8; 6]) {
        match command {
            Command::RegisterShift => self.state.bit_position = FIRST_SHIFT_POSITION,
            Command::TimeRead => {
                let mut register = [0; REGISTER_BYTES];
                register[STATUS_BYTE] = STATUS_TIME_VALID;
                register[CALENDAR_FIRST_BYTE..].copy_from_slice(host_time_bcd);
                self.state.register = register;
            }
            Command::RegisterHold | Command::TimeSet | Command::Other => {}
        }
        if command != Command::Other {
            self.state.shifting = command == Command::RegisterShift;
        }
        if matches!(command, Command::RegisterShift | Command::TimeRead) {
            self.state.present(FIRST_SHIFT_POSITION);
        }
    }

    /// Handles a rising CLK edge.
    ///
    /// In serial mode the serial command register shifts one bit toward bit 0.
    /// An active register shift steps to the next cell until it reaches the
    /// last one. DATA OUT then shows the current cell.
    fn clock_edge(&mut self) {
        let state = &mut self.state;
        if state.serial_mode() {
            state.serial_command >>= 1;
        }
        if state.shifting {
            state.bit_position = state.bit_position.saturating_sub(1);
        }
        state.present(state.bit_position);
    }

    /// Handles a write with STB and CLK low.
    ///
    /// The chip latches C0-C2. DATA IN enters bit 4 of the serial command
    /// register in serial mode and the current shift register cell in every
    /// mode.
    fn data_phase(&mut self, value: u8) {
        let data_in = value & LINE_DATA_IN != 0;
        let state = &mut self.state;
        state.command_lines = value & LINE_COMMAND;
        if state.serial_mode() {
            let input = if data_in { SERIAL_COMMAND_INPUT } else { 0 };
            state.serial_command = (state.serial_command & SERIAL_COMMAND_MASK) | input;
        }
        state.set_bit_at(state.bit_position, data_in);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_TIME: [u8; 6] = [
        0x26, // year: 2026 (BCD)
        0x31, // month=3, day_of_week=1 (Monday)
        0x03, // day: 3rd (BCD)
        0x14, // hour: 14 (BCD)
        0x30, // minute: 30 (BCD)
        0x45, // second: 45 (BCD)
    ];

    fn stb_command(rtc: &mut Upd4990aRtc, cmd: u8) {
        // DATA phase: latch command.
        rtc.write_port(cmd & 0x07, &TEST_TIME);
        // STB rising edge.
        rtc.write_port((cmd & 0x07) | 0x08, &TEST_TIME);
        // Release STB.
        rtc.write_port(0x00, &TEST_TIME);
    }

    fn clock_pulse(rtc: &mut Upd4990aRtc) {
        rtc.write_port(0x10, &TEST_TIME);
        rtc.write_port(0x00, &TEST_TIME);
    }

    #[test]
    fn time_read_populates_register() {
        let mut rtc = Upd4990aRtc::new();
        stb_command(&mut rtc, 0x03);

        assert_eq!(rtc.state.register[0], 0x00); // padding
        assert_eq!(rtc.state.register[1], 0x01); // status
        assert_eq!(rtc.state.register[2], 0x26); // year
        assert_eq!(rtc.state.register[3], 0x31); // month|dow
        assert_eq!(rtc.state.register[4], 0x03); // day
        assert_eq!(rtc.state.register[5], 0x14); // hour
        assert_eq!(rtc.state.register[6], 0x30); // minute
        assert_eq!(rtc.state.register[7], 0x45); // second
    }

    #[test]
    fn register_shift_outputs_correct_bits() {
        let mut rtc = Upd4990aRtc::new();
        // Load time into register.
        stb_command(&mut rtc, 0x03);
        // Enter shift mode.
        stb_command(&mut rtc, 0x01);

        // The shift starts at position 63, bit 0 of the seconds byte.
        // The seconds byte is 0x45 = 0100_0101, bit 0 is 1.
        assert_eq!(rtc.cdat(), 1);

        // Position 62 presents bit 1 of the seconds byte, which is 0.
        clock_pulse(&mut rtc);
        assert_eq!(rtc.cdat(), 0);

        // Position 61 presents bit 2 of the seconds byte, which is 1.
        clock_pulse(&mut rtc);
        assert_eq!(rtc.cdat(), 1);
    }

    #[test]
    fn clock_shifts_through_all_48_bits() {
        let mut rtc = Upd4990aRtc::new();
        stb_command(&mut rtc, 0x03);
        stb_command(&mut rtc, 0x01);

        // Clock all 48 BCD time bits (positions 63 down to 16).
        let mut bits = Vec::new();
        bits.push(rtc.cdat());
        for _ in 0..47 {
            clock_pulse(&mut rtc);
            bits.push(rtc.cdat());
        }

        // The shift register outputs the seconds byte first, then the minute
        // byte and so on. Each byte comes out least significant bit first.
        let mut reconstructed = [0u8; 6];
        for (i, bit) in bits.iter().enumerate() {
            let byte_index = 5 - (i / 8); // seconds byte down to year byte
            let bit_index = i % 8;
            reconstructed[byte_index] |= bit << bit_index;
        }
        assert_eq!(reconstructed, TEST_TIME);
    }

    #[test]
    fn register_hold_clears_shift_flag() {
        let mut rtc = Upd4990aRtc::new();
        stb_command(&mut rtc, 0x01);
        assert!(rtc.state.shifting);

        stb_command(&mut rtc, 0x00);
        assert!(!rtc.state.shifting);
    }

    #[test]
    fn data_phase_writes_bits_into_register() {
        let mut rtc = Upd4990aRtc::new();
        // Position starts at 0. Write a 1 bit (bit 5 of value set).
        rtc.write_port(0x20, &TEST_TIME);
        // Position 0 is bit 7 of byte 0.
        assert_eq!(rtc.state.register[0] & 0x80, 0x80);

        // Write a 0 bit.
        rtc.write_port(0x00, &TEST_TIME);
        assert_eq!(rtc.state.register[0] & 0x80, 0x00);
    }

    #[test]
    fn extended_mode_serial_command() {
        let mut rtc = Upd4990aRtc::new();

        // Set C0-C2 to 7 (serial mode).
        rtc.write_port(0x07, &TEST_TIME);
        assert_eq!(rtc.state.command_lines, 7);

        // Accumulate serial bits via CLK pulses.
        // Each CLK right-shifts the serial register.
        // Bit 5 of the data byte feeds into bit 4 of serial register.
        // Write bit 5 = 1 -> serial bit 4 = 1.
        rtc.write_port(0x27, &TEST_TIME); // DATA phase: C0-C2 = 7, DATA IN = 1 enters bit 4.
        assert_eq!(rtc.state.serial_command & 0x10, 0x10);

        // CLK pulse shifts serial right by 1.
        clock_pulse(&mut rtc);
        assert_eq!(rtc.state.serial_command & 0x10, 0x00); // bit 4 shifted to bit 3

        // After more shifts we can build up a 4-bit command.
        // For the purpose of this test, verify that STB with C0-C2 = 7
        // uses the serial register's low nibble as the command.
        rtc.state.serial_command = 0x03; // Time Read via serial.
        rtc.write_port(0x07, &TEST_TIME); // DATA phase: C0-C2 = 7.
        rtc.write_port(0x0F, &TEST_TIME); // STB rising edge with C0-C2 = 7.

        assert_eq!(rtc.state.register[1], 0x01); // Status byte was set by TIME_READ.
    }
}
