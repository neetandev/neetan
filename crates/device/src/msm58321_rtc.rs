//! MSM58321-class serial real-time clock controller.
//!
//! Accessed through a data port and a command port. A register address is latched
//! by writing it to the data port and strobing the ADDRESS-WRITE command; the
//! selected calendar register is then read back one BCD nibble at a time from the
//! data port, with bit 7 acting as a ready flag. The host provides the wall-clock
//! time via the shared 6-byte BCD buffer, so the device itself has no time-source
//! dependency.

/// Calendar register addresses (one BCD digit each).
const REG_ONE_SECOND: u8 = 0x00;
const REG_TEN_SECOND: u8 = 0x01;
const REG_ONE_MINUTE: u8 = 0x02;
const REG_TEN_MINUTE: u8 = 0x03;
const REG_ONE_HOUR: u8 = 0x04;
const REG_TEN_HOUR: u8 = 0x05;
const REG_WEEKDAY: u8 = 0x06;
const REG_ONE_DAY: u8 = 0x07;
const REG_TEN_DAY: u8 = 0x08;
const REG_ONE_MONTH: u8 = 0x09;
const REG_TEN_MONTH: u8 = 0x0A;
const REG_ONE_YEAR: u8 = 0x0B;
const REG_TEN_YEAR: u8 = 0x0C;

/// Command-port bit 7 enables the controller (chip select); the low bits then
/// select the ADDRESS-WRITE, WRITE, or READ strobe.
const COMMAND_ENABLE_BIT: u8 = 0x80;
const COMMAND_ADDRESS_WRITE: u8 = 0x81;
const COMMAND_WRITE: u8 = 0x82;
const COMMAND_READ: u8 = 0x84;

/// Data-port bit 7 is the ready flag; the low nibble carries the selected digit.
const DATA_READY_BIT: u8 = 0x80;
const DATA_DIGIT_MASK: u8 = 0x0F;

/// The ten-hour register's bit 3 selects 24-hour mode, bit 2 is the PM flag.
const TEN_HOUR_24H_BIT: u8 = 0x08;
const TEN_HOUR_PM_BIT: u8 = 0x04;

/// The ready flag reads low for the first 674 us of each second.
const READY_LOW_MICROS: u32 = 674;

/// Controller access state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Msm58321Mode {
    /// Idle: waiting for the enable bit to begin a command sequence.
    #[default]
    Idle,
    /// Enabled: accepting ADDRESS-WRITE / WRITE / READ strobes.
    Command,
}

/// Snapshot of the MSM58321 state.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Msm58321State {
    /// Current access mode.
    pub mode: Msm58321Mode,
    /// 24-hour mode flag (true = 00-23, false = 12-hour with AM/PM).
    pub hour_24: bool,
    /// Latched register address selected for read/write.
    pub register_latch: u8,
    /// Last value written to the data port (register address or write data).
    pub last_data_write: u8,
}

/// MSM58321-class serial real-time clock controller.
#[derive(Default)]
pub struct Msm58321Rtc {
    /// Embedded state for save/restore.
    pub state: Msm58321State,
}

impl Msm58321Rtc {
    /// Creates a controller in its reset state (idle, 24-hour mode).
    pub fn new() -> Self {
        Self {
            state: Msm58321State {
                mode: Msm58321Mode::Idle,
                hour_24: true,
                register_latch: 0,
                last_data_write: 0,
            },
        }
    }

    /// Handles a write to the data port: latches the byte for the next strobe.
    pub fn write_data(&mut self, value: u8) {
        self.state.last_data_write = value;
    }

    /// Handles a write to the command port: drives the access state machine.
    pub fn write_command(&mut self, value: u8) {
        match self.state.mode {
            Msm58321Mode::Idle => {
                if value & COMMAND_ENABLE_BIT != 0 {
                    self.state.mode = Msm58321Mode::Command;
                }
            }
            Msm58321Mode::Command => {
                if value & COMMAND_ENABLE_BIT == 0 {
                    self.state.mode = Msm58321Mode::Idle;
                } else if value == COMMAND_ADDRESS_WRITE {
                    self.state.register_latch = self.state.last_data_write & DATA_DIGIT_MASK;
                } else if value == COMMAND_WRITE {
                    if self.state.register_latch == REG_TEN_HOUR {
                        self.state.hour_24 = self.state.last_data_write & TEN_HOUR_24H_BIT != 0;
                    }
                } else if value == COMMAND_READ {
                    // The selected digit is read back through the data port.
                }
            }
        }
    }

    /// Handles a read from the data port. `host_time_bcd` is the shared 6-byte
    /// BCD time buffer `[year, month<<4|day_of_week, day, hour, minute, second]`;
    /// `subsecond_micros` is the elapsed microseconds within the current second,
    /// used to model the ready flag. Bit 7 is the ready flag (low while busy) and
    /// the low nibble is the selected register's BCD digit.
    pub fn read_data(&self, host_time_bcd: &[u8; 6], subsecond_micros: u32) -> u8 {
        let mut data = if subsecond_micros % 1_000_000 < READY_LOW_MICROS {
            0
        } else {
            DATA_READY_BIT
        };
        if self.state.mode != Msm58321Mode::Idle {
            data |= self.digit(host_time_bcd) & DATA_DIGIT_MASK;
        }
        data
    }

    /// The BCD digit for the latched register, derived from the host time.
    fn digit(&self, host_time_bcd: &[u8; 6]) -> u8 {
        let [year, month_weekday, day, hour, minute, second] = *host_time_bcd;
        let month = month_weekday >> 4;
        let weekday = month_weekday & DATA_DIGIT_MASK;
        let hour_value = bcd_to_binary(hour);

        match self.state.register_latch {
            REG_ONE_SECOND => second & DATA_DIGIT_MASK,
            REG_TEN_SECOND => second >> 4,
            REG_ONE_MINUTE => minute & DATA_DIGIT_MASK,
            REG_TEN_MINUTE => minute >> 4,
            REG_ONE_HOUR => self.display_hour(hour_value) % 10,
            REG_TEN_HOUR => {
                let mut digit = if self.state.hour_24 {
                    TEN_HOUR_24H_BIT
                } else {
                    0
                };
                if !self.state.hour_24 && hour_value >= 12 {
                    digit |= TEN_HOUR_PM_BIT;
                }
                digit | (self.display_hour(hour_value) / 10)
            }
            REG_WEEKDAY => weekday,
            REG_ONE_DAY => day & DATA_DIGIT_MASK,
            REG_TEN_DAY => day >> 4,
            REG_ONE_MONTH => month % 10,
            REG_TEN_MONTH => month / 10,
            REG_ONE_YEAR => year & DATA_DIGIT_MASK,
            REG_TEN_YEAR => year >> 4,
            _ => 0,
        }
    }

    /// The hour value to display, folded into 1-12 range in 12-hour mode.
    fn display_hour(&self, hour_value: u8) -> u8 {
        if !self.state.hour_24 && hour_value > 12 {
            hour_value - 12
        } else {
            hour_value
        }
    }
}

fn bcd_to_binary(value: u8) -> u8 {
    (value >> 4) * 10 + (value & 0x0F)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-03-03 (Monday) 14:30:45.
    const TEST_TIME: [u8; 6] = [0x26, 0x31, 0x03, 0x14, 0x30, 0x45];

    /// Latches a register address through the ADDRESS-WRITE strobe.
    fn select_register(rtc: &mut Msm58321Rtc, register: u8) {
        rtc.write_command(COMMAND_ENABLE_BIT);
        rtc.write_data(register);
        rtc.write_command(COMMAND_ADDRESS_WRITE);
    }

    #[test]
    fn reads_each_time_digit() {
        let mut rtc = Msm58321Rtc::new();

        select_register(&mut rtc, REG_ONE_SECOND);
        assert_eq!(
            rtc.read_data(&TEST_TIME, READY_LOW_MICROS),
            DATA_READY_BIT | 5
        );
        select_register(&mut rtc, REG_TEN_SECOND);
        assert_eq!(
            rtc.read_data(&TEST_TIME, READY_LOW_MICROS),
            DATA_READY_BIT | 4
        );
        select_register(&mut rtc, REG_ONE_MINUTE);
        assert_eq!(rtc.read_data(&TEST_TIME, READY_LOW_MICROS), DATA_READY_BIT);
        select_register(&mut rtc, REG_TEN_MINUTE);
        assert_eq!(
            rtc.read_data(&TEST_TIME, READY_LOW_MICROS),
            DATA_READY_BIT | 3
        );
        select_register(&mut rtc, REG_ONE_HOUR);
        assert_eq!(
            rtc.read_data(&TEST_TIME, READY_LOW_MICROS),
            DATA_READY_BIT | 4
        );
        // 24-hour mode: bit 3 set, tens digit of 14 is 1.
        select_register(&mut rtc, REG_TEN_HOUR);
        assert_eq!(
            rtc.read_data(&TEST_TIME, READY_LOW_MICROS),
            DATA_READY_BIT | TEN_HOUR_24H_BIT | 1
        );
    }

    #[test]
    fn reads_calendar_digits() {
        let mut rtc = Msm58321Rtc::new();

        select_register(&mut rtc, REG_WEEKDAY);
        assert_eq!(
            rtc.read_data(&TEST_TIME, READY_LOW_MICROS),
            DATA_READY_BIT | 1
        );
        select_register(&mut rtc, REG_ONE_DAY);
        assert_eq!(
            rtc.read_data(&TEST_TIME, READY_LOW_MICROS),
            DATA_READY_BIT | 3
        );
        select_register(&mut rtc, REG_TEN_DAY);
        assert_eq!(rtc.read_data(&TEST_TIME, READY_LOW_MICROS), DATA_READY_BIT);
        select_register(&mut rtc, REG_ONE_MONTH);
        assert_eq!(
            rtc.read_data(&TEST_TIME, READY_LOW_MICROS),
            DATA_READY_BIT | 3
        );
        select_register(&mut rtc, REG_TEN_MONTH);
        assert_eq!(rtc.read_data(&TEST_TIME, READY_LOW_MICROS), DATA_READY_BIT);
        select_register(&mut rtc, REG_ONE_YEAR);
        assert_eq!(
            rtc.read_data(&TEST_TIME, READY_LOW_MICROS),
            DATA_READY_BIT | 6
        );
        select_register(&mut rtc, REG_TEN_YEAR);
        assert_eq!(
            rtc.read_data(&TEST_TIME, READY_LOW_MICROS),
            DATA_READY_BIT | 2
        );
    }

    #[test]
    fn ready_flag_reads_low_during_busy_window() {
        let mut rtc = Msm58321Rtc::new();
        select_register(&mut rtc, REG_ONE_SECOND);
        // Within the busy window the ready bit is clear.
        assert_eq!(rtc.read_data(&TEST_TIME, 0) & DATA_READY_BIT, 0);
        assert_eq!(
            rtc.read_data(&TEST_TIME, READY_LOW_MICROS - 1) & DATA_READY_BIT,
            0
        );
        // After the busy window the ready bit is set.
        assert_eq!(
            rtc.read_data(&TEST_TIME, READY_LOW_MICROS) & DATA_READY_BIT,
            DATA_READY_BIT
        );
    }

    #[test]
    fn idle_read_returns_no_digit() {
        let rtc = Msm58321Rtc::new();
        // Without an enabled command sequence the low nibble stays zero.
        assert_eq!(rtc.read_data(&TEST_TIME, READY_LOW_MICROS), DATA_READY_BIT);
    }

    #[test]
    fn twelve_hour_mode_sets_pm_bit() {
        let mut rtc = Msm58321Rtc::new();
        // Switch to 12-hour mode by writing the ten-hour register with bit 3 clear.
        rtc.write_command(COMMAND_ENABLE_BIT);
        rtc.write_data(REG_TEN_HOUR);
        rtc.write_command(COMMAND_ADDRESS_WRITE);
        rtc.write_data(0x00);
        rtc.write_command(COMMAND_WRITE);
        assert!(!rtc.state.hour_24);

        // 14:xx in 12-hour mode is 2 PM: PM bit set, tens digit 0.
        select_register(&mut rtc, REG_TEN_HOUR);
        assert_eq!(
            rtc.read_data(&TEST_TIME, READY_LOW_MICROS),
            DATA_READY_BIT | TEN_HOUR_PM_BIT
        );
        select_register(&mut rtc, REG_ONE_HOUR);
        assert_eq!(
            rtc.read_data(&TEST_TIME, READY_LOW_MICROS),
            DATA_READY_BIT | 2
        );
    }

    #[test]
    fn disabling_enable_bit_returns_to_idle() {
        let mut rtc = Msm58321Rtc::new();
        rtc.write_command(COMMAND_ENABLE_BIT);
        assert_eq!(rtc.state.mode, Msm58321Mode::Command);
        rtc.write_command(0x00);
        assert_eq!(rtc.state.mode, Msm58321Mode::Idle);
    }
}
