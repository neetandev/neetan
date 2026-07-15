//! µPD4990A serial real-time clock controller.
//!
//! The µPD4990A is accessed through port 0x20 (write) with serial data
//! output on port 0x33 bit 0 (CDAT). The host provides the time via a
//! 6-byte BCD buffer; the device itself has no time source dependency.

/// Register length in bytes (64-bit shift register).
const REG_LEN: usize = 8;

save_state::runtime_state! {
/// Authoritative uPD4990A state.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Upd4990aState {
    /// Previous port 0x20 value (for edge detection).
    pub last: u8,
    /// Serial command accumulator (for extended mode, cmd=7).
    pub serial: u8,
    /// Latched parallel command bits (C2:C0 from DATA phase).
    pub parallel: u8,
    /// 8-byte shift register (48-bit calendar + status/padding bytes).
    pub reg: [u8; REG_LEN],
    /// Current bit position in shift register (0..63).
    pub pos: usize,
    /// Current serial data output bit (0 or 1).
    pub cdat: u8,
    /// Whether register shift mode is active.
    pub regsft: bool,
}}

/// µPD4990A serial real-time clock controller.
#[derive(Default)]
pub struct Upd4990aRtc {
    /// Embedded state for save/restore.
    pub state: Upd4990aState,
}

impl_simple_state_accessors!(Upd4990aRtc, Upd4990aState);

impl Upd4990aRtc {
    /// Creates a new µPD4990A in its reset state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns current CDAT output bit (0 or 1) for port 0x33 bit 0.
    pub fn cdat(&self) -> u8 {
        self.state.cdat
    }

    /// Called on port 0x20 write.
    ///
    /// `value` is the byte written. `host_time_bcd` is a 6-byte BCD buffer
    /// representing the current host local time, used only when the TIME_READ
    /// command is latched on STB rising edge:
    ///   `[year, month<<4|day_of_week, day, hour, minute, second]`
    pub fn write_port(&mut self, value: u8, host_time_bcd: &[u8; 6]) {
        let changed = value ^ self.state.last;
        self.state.last = value;

        if value & 0x08 != 0 {
            // STB asserted - execute command on rising edge.
            if changed & 0x08 != 0 {
                let cmd = if self.state.parallel == 7 {
                    self.state.serial & 0x0F
                } else {
                    self.state.parallel
                };

                match cmd {
                    // Register Hold
                    0x00 => {
                        self.state.regsft = false;
                    }
                    // Register Shift
                    0x01 => {
                        self.state.regsft = true;
                        self.state.pos = (REG_LEN * 8) - 1;
                        self.state.cdat = self.state.reg[REG_LEN - 1] & 1;
                    }
                    // Time Set / Counter Hold
                    0x02 => {
                        self.state.regsft = false;
                    }
                    // Time Read
                    0x03 => {
                        self.state.regsft = false;
                        self.state.reg = [0; REG_LEN];
                        // Copy 6-byte BCD time into reg[2..8].
                        self.state.reg[2..8].copy_from_slice(host_time_bcd);
                        self.state.cdat = self.state.reg[REG_LEN - 1] & 1;
                        // Status byte: 0x01 = valid data.
                        self.state.reg[1] = 0x01;
                    }
                    _ => {}
                }
            }
        } else if value & 0x10 != 0 {
            // CLK asserted (STB not set) - shift on rising edge.
            if changed & 0x10 != 0 {
                if self.state.parallel == 7 {
                    self.state.serial >>= 1;
                }
                if self.state.regsft && self.state.pos > 0 {
                    self.state.pos -= 1;
                }
                self.state.cdat =
                    (self.state.reg[self.state.pos / 8] >> ((!self.state.pos) & 7)) & 1;
            }
        } else {
            // DATA phase (neither STB nor CLK set).
            self.state.parallel = value & 7;
            if self.state.parallel == 7 {
                self.state.serial &= 0x0F;
                self.state.serial |= (value >> 1) & 0x10;
            }
            // Write data bit (bit 5) into shift register at current position.
            if value & 0x20 != 0 {
                self.state.reg[self.state.pos / 8] |= 0x80 >> (self.state.pos & 7);
            } else {
                self.state.reg[self.state.pos / 8] &= !(0x80 >> (self.state.pos & 7));
            }
        }
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

        assert_eq!(rtc.state.reg[0], 0x00); // padding
        assert_eq!(rtc.state.reg[1], 0x01); // status
        assert_eq!(rtc.state.reg[2], 0x26); // year
        assert_eq!(rtc.state.reg[3], 0x31); // month|dow
        assert_eq!(rtc.state.reg[4], 0x03); // day
        assert_eq!(rtc.state.reg[5], 0x14); // hour
        assert_eq!(rtc.state.reg[6], 0x30); // minute
        assert_eq!(rtc.state.reg[7], 0x45); // second
    }

    #[test]
    fn register_shift_outputs_correct_bits() {
        let mut rtc = Upd4990aRtc::new();
        // Load time into register.
        stb_command(&mut rtc, 0x03);
        // Enter shift mode.
        stb_command(&mut rtc, 0x01);

        // Position starts at 63 (MSB of reg[7]).
        // reg[7] = 0x45 = 0100_0101.
        // Position 63 -> bit ((!63) & 7) = bit 0 -> (0x45 >> 0) & 1 = 1.
        assert_eq!(rtc.cdat(), 1);

        // Clock to position 62 -> bit 1 -> (0x45 >> 1) & 1 = 0.
        clock_pulse(&mut rtc);
        assert_eq!(rtc.cdat(), 0);

        // Clock to position 61 -> bit 2 -> (0x45 >> 2) & 1 = 1.
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

        // Reconstruct bytes from the bit stream (MSB first per byte, MSB byte first).
        let mut bytes = [0u8; 6];
        for (i, bit) in bits.iter().enumerate() {
            let byte_idx = i / 8;
            let bit_idx = i % 8;
            bytes[byte_idx] |= bit << (7 - bit_idx);
        }

        // The shift register outputs reg[7] first (seconds), then reg[6], etc.
        // Bits come out MSB-first within each byte due to the bit indexing.
        let mut reconstructed = [0u8; 6];
        for (i, bit) in bits.iter().enumerate() {
            let byte_idx = 5 - (i / 8); // reg[7] down to reg[2]
            let bit_idx = i % 8;
            reconstructed[byte_idx] |= bit << bit_idx;
        }
        assert_eq!(reconstructed, TEST_TIME);
    }

    #[test]
    fn register_hold_clears_shift_flag() {
        let mut rtc = Upd4990aRtc::new();
        stb_command(&mut rtc, 0x01);
        assert!(rtc.state.regsft);

        stb_command(&mut rtc, 0x00);
        assert!(!rtc.state.regsft);
    }

    #[test]
    fn data_phase_writes_bits_into_register() {
        let mut rtc = Upd4990aRtc::new();
        // Position starts at 0. Write a 1 bit (bit 5 of value set).
        rtc.write_port(0x20, &TEST_TIME);
        // pos=0, bit mask = 0x80 >> (0 & 7) = 0x80.
        assert_eq!(rtc.state.reg[0] & 0x80, 0x80);

        // Write a 0 bit.
        rtc.write_port(0x00, &TEST_TIME);
        assert_eq!(rtc.state.reg[0] & 0x80, 0x00);
    }

    #[test]
    fn extended_mode_serial_command() {
        let mut rtc = Upd4990aRtc::new();

        // Set parallel to 7 (extended mode).
        rtc.write_port(0x07, &TEST_TIME);
        assert_eq!(rtc.state.parallel, 7);

        // Accumulate serial bits via CLK pulses.
        // Each CLK right-shifts the serial register.
        // Bit 5 of the data byte feeds into bit 4 of serial register.
        // Write bit 5 = 1 -> serial bit 4 = 1.
        rtc.write_port(0x27, &TEST_TIME); // DATA phase: parallel=7, serial |= (0x27 >> 1) & 0x10 = 0x10.
        assert_eq!(rtc.state.serial & 0x10, 0x10);

        // CLK pulse shifts serial right by 1.
        clock_pulse(&mut rtc);
        assert_eq!(rtc.state.serial & 0x10, 0x00); // bit 4 shifted to bit 3

        // After more shifts we can build up a 4-bit command.
        // For the purpose of this test, verify that STB with parallel=7
        // uses the serial register's low nibble as the command.
        rtc.state.serial = 0x03; // Time Read via serial.
        rtc.write_port(0x07, &TEST_TIME); // DATA phase: parallel=7.
        rtc.write_port(0x0F, &TEST_TIME); // STB rising edge with parallel=7.

        assert_eq!(rtc.state.reg[1], 0x01); // Status byte was set by TIME_READ.
    }
}
