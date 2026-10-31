//! MSM58321-class serial real-time clock controller.
//!
//! Accessed through a data port and a command port. A register address is latched
//! by writing it to the data port and strobing the ADDRESS-WRITE command; the
//! selected calendar register is then read back one BCD nibble at a time from the
//! data port, with bit 7 acting as a ready flag. The clock runs on emulated time:
//! it starts from a seed date and advances with the elapsed CPU cycles.

use common::HostDateTime;

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
const READY_LOW_MICROS: u64 = 674;

/// Number of microseconds in one second.
const MICROS_PER_SECOND: u64 = 1_000_000;

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
    /// Seed date and time in seconds since 1970-01-01 00:00:00.
    pub seed_seconds: u64,
    /// CPU cycle at which the clock showed the seed time.
    pub seed_cycle: u64,
}

/// MSM58321-class serial real-time clock controller.
#[derive(Default)]
pub struct Msm58321Rtc {
    /// Embedded state for save/restore.
    pub state: Msm58321State,
}

save_state::runtime_state! {
/// Encodable MSM58321 access state.
#[derive(Clone)]
pub struct Msm58321RuntimeState {
    mode: u8,
    hour_24: bool,
    register_latch: u8,
    last_data_write: u8,
    seed_seconds: u64,
    seed_cycle: u64,
}}

impl Msm58321Rtc {
    /// Creates a controller in its reset state (idle, 24-hour mode).
    pub fn new() -> Self {
        Self {
            state: Msm58321State {
                mode: Msm58321Mode::Idle,
                hour_24: true,
                register_latch: 0,
                last_data_write: 0,
                seed_seconds: 0,
                seed_cycle: 0,
            },
        }
    }

    /// Sets the clock to `seed` at `current_cycle`. The clock advances from there.
    pub fn seed_time(&mut self, seed: HostDateTime, current_cycle: u64) {
        self.state.seed_seconds = seed.to_unix_seconds();
        self.state.seed_cycle = current_cycle;
    }

    /// Captures the serial access state and register latch.
    pub fn capture_state(&self) -> Msm58321RuntimeState {
        Msm58321RuntimeState {
            mode: match self.state.mode {
                Msm58321Mode::Idle => 0,
                Msm58321Mode::Command => 1,
            },
            hour_24: self.state.hour_24,
            register_latch: self.state.register_latch,
            last_data_write: self.state.last_data_write,
            seed_seconds: self.state.seed_seconds,
            seed_cycle: self.state.seed_cycle,
        }
    }

    /// Restores the serial access state and register latch.
    pub fn restore_state(
        &mut self,
        state: Msm58321RuntimeState,
    ) -> Result<(), save_state::StateValidationError> {
        let mode = match state.mode {
            0 => Msm58321Mode::Idle,
            1 => Msm58321Mode::Command,
            _ => return Err(save_state::StateValidationError::new("RTC mode is invalid")),
        };
        if state.register_latch > REG_TEN_YEAR {
            return Err(save_state::StateValidationError::new(
                "RTC register latch is invalid",
            ));
        }
        self.state = Msm58321State {
            mode,
            hour_24: state.hour_24,
            register_latch: state.register_latch,
            last_data_write: state.last_data_write,
            seed_seconds: state.seed_seconds,
            seed_cycle: state.seed_cycle,
        };
        Ok(())
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

    /// Handles a read from the data port at `current_cycle`. Bit 7 is the ready
    /// flag (low while busy) and the low nibble is the selected register's BCD
    /// digit.
    pub fn read_data(&self, current_cycle: u64, cpu_clock_hz: u32) -> u8 {
        let elapsed_micros = u128::from(current_cycle.saturating_sub(self.state.seed_cycle))
            * u128::from(MICROS_PER_SECOND)
            / u128::from(cpu_clock_hz.max(1));
        let elapsed_micros = elapsed_micros as u64;
        let mut data = if elapsed_micros % MICROS_PER_SECOND < READY_LOW_MICROS {
            0
        } else {
            DATA_READY_BIT
        };
        if self.state.mode != Msm58321Mode::Idle {
            let seconds = self.state.seed_seconds + elapsed_micros / MICROS_PER_SECOND;
            let time = HostDateTime::from_unix_seconds(seconds).to_bcd_bytes();
            data |= self.digit(&time) & DATA_DIGIT_MASK;
        }
        data
    }

    /// The BCD digit for the latched register, derived from the 6-byte BCD time
    /// `[year, month<<4|day_of_week, day, hour, minute, second]`.
    fn digit(&self, time_bcd: &[u8; 6]) -> u8 {
        let [year, month_weekday, day, hour, minute, second] = *time_bcd;
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

    /// 2026-03-03 (Tuesday) 14:30:45.
    const TEST_TIME: HostDateTime = HostDateTime {
        year: 2026,
        month: 3,
        day: 3,
        day_of_week: 2,
        hour: 14,
        minute: 30,
        second: 45,
    };

    /// A 1 MHz clock, so one cycle is one microsecond.
    const CPU_HZ: u32 = 1_000_000;

    /// A controller seeded with [`TEST_TIME`] at cycle 0.
    fn seeded_rtc() -> Msm58321Rtc {
        let mut rtc = Msm58321Rtc::new();
        rtc.seed_time(TEST_TIME, 0);
        rtc
    }

    /// Latches a register address through the ADDRESS-WRITE strobe.
    fn select_register(rtc: &mut Msm58321Rtc, register: u8) {
        rtc.write_command(COMMAND_ENABLE_BIT);
        rtc.write_data(register);
        rtc.write_command(COMMAND_ADDRESS_WRITE);
    }

    #[test]
    fn reads_each_time_digit() {
        let mut rtc = seeded_rtc();

        select_register(&mut rtc, REG_ONE_SECOND);
        assert_eq!(rtc.read_data(READY_LOW_MICROS, CPU_HZ), DATA_READY_BIT | 5);
        select_register(&mut rtc, REG_TEN_SECOND);
        assert_eq!(rtc.read_data(READY_LOW_MICROS, CPU_HZ), DATA_READY_BIT | 4);
        select_register(&mut rtc, REG_ONE_MINUTE);
        assert_eq!(rtc.read_data(READY_LOW_MICROS, CPU_HZ), DATA_READY_BIT);
        select_register(&mut rtc, REG_TEN_MINUTE);
        assert_eq!(rtc.read_data(READY_LOW_MICROS, CPU_HZ), DATA_READY_BIT | 3);
        select_register(&mut rtc, REG_ONE_HOUR);
        assert_eq!(rtc.read_data(READY_LOW_MICROS, CPU_HZ), DATA_READY_BIT | 4);
        // 24-hour mode: bit 3 set, tens digit of 14 is 1.
        select_register(&mut rtc, REG_TEN_HOUR);
        assert_eq!(
            rtc.read_data(READY_LOW_MICROS, CPU_HZ),
            DATA_READY_BIT | TEN_HOUR_24H_BIT | 1
        );
    }

    #[test]
    fn reads_calendar_digits() {
        let mut rtc = seeded_rtc();

        select_register(&mut rtc, REG_WEEKDAY);
        assert_eq!(rtc.read_data(READY_LOW_MICROS, CPU_HZ), DATA_READY_BIT | 2);
        select_register(&mut rtc, REG_ONE_DAY);
        assert_eq!(rtc.read_data(READY_LOW_MICROS, CPU_HZ), DATA_READY_BIT | 3);
        select_register(&mut rtc, REG_TEN_DAY);
        assert_eq!(rtc.read_data(READY_LOW_MICROS, CPU_HZ), DATA_READY_BIT);
        select_register(&mut rtc, REG_ONE_MONTH);
        assert_eq!(rtc.read_data(READY_LOW_MICROS, CPU_HZ), DATA_READY_BIT | 3);
        select_register(&mut rtc, REG_TEN_MONTH);
        assert_eq!(rtc.read_data(READY_LOW_MICROS, CPU_HZ), DATA_READY_BIT);
        select_register(&mut rtc, REG_ONE_YEAR);
        assert_eq!(rtc.read_data(READY_LOW_MICROS, CPU_HZ), DATA_READY_BIT | 6);
        select_register(&mut rtc, REG_TEN_YEAR);
        assert_eq!(rtc.read_data(READY_LOW_MICROS, CPU_HZ), DATA_READY_BIT | 2);
    }

    #[test]
    fn ready_flag_reads_low_during_busy_window() {
        let mut rtc = seeded_rtc();
        select_register(&mut rtc, REG_ONE_SECOND);
        // Within the busy window the ready bit is clear.
        assert_eq!(rtc.read_data(0, CPU_HZ) & DATA_READY_BIT, 0);
        assert_eq!(
            rtc.read_data(READY_LOW_MICROS - 1, CPU_HZ) & DATA_READY_BIT,
            0
        );
        // After the busy window the ready bit is set.
        assert_eq!(
            rtc.read_data(READY_LOW_MICROS, CPU_HZ) & DATA_READY_BIT,
            DATA_READY_BIT
        );
    }

    #[test]
    fn clock_advances_with_emulated_time() {
        let mut rtc = seeded_rtc();
        let second = u64::from(CPU_HZ);
        select_register(&mut rtc, REG_ONE_SECOND);
        assert_eq!(rtc.read_data(second - 1, CPU_HZ) & DATA_DIGIT_MASK, 5);
        assert_eq!(rtc.read_data(second, CPU_HZ) & DATA_DIGIT_MASK, 6);
        // Fifteen seconds later the minute rolls over to 14:31:00.
        select_register(&mut rtc, REG_ONE_MINUTE);
        assert_eq!(rtc.read_data(15 * second, CPU_HZ) & DATA_DIGIT_MASK, 1);
        select_register(&mut rtc, REG_TEN_SECOND);
        assert_eq!(rtc.read_data(15 * second, CPU_HZ) & DATA_DIGIT_MASK, 0);
    }

    #[test]
    fn ready_flag_follows_the_seeded_second() {
        let mut rtc = Msm58321Rtc::new();
        rtc.seed_time(TEST_TIME, 500);
        select_register(&mut rtc, REG_ONE_SECOND);
        // The busy window starts at the seed cycle, not at cycle 0.
        assert_eq!(rtc.read_data(500, CPU_HZ) & DATA_READY_BIT, 0);
        assert_eq!(
            rtc.read_data(500 + READY_LOW_MICROS, CPU_HZ) & DATA_READY_BIT,
            DATA_READY_BIT
        );
    }

    #[test]
    fn save_state_keeps_the_running_clock() {
        let mut rtc = seeded_rtc();
        select_register(&mut rtc, REG_ONE_SECOND);
        let mut restored = Msm58321Rtc::new();
        restored.restore_state(rtc.capture_state()).unwrap();
        let cycle = 3 * u64::from(CPU_HZ) + READY_LOW_MICROS;
        assert_eq!(
            restored.read_data(cycle, CPU_HZ),
            rtc.read_data(cycle, CPU_HZ)
        );
        assert_eq!(restored.read_data(cycle, CPU_HZ) & DATA_DIGIT_MASK, 8);
    }

    #[test]
    fn idle_read_returns_no_digit() {
        let rtc = seeded_rtc();
        // Without an enabled command sequence the low nibble stays zero.
        assert_eq!(rtc.read_data(READY_LOW_MICROS, CPU_HZ), DATA_READY_BIT);
    }

    #[test]
    fn twelve_hour_mode_sets_pm_bit() {
        let mut rtc = seeded_rtc();
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
            rtc.read_data(READY_LOW_MICROS, CPU_HZ),
            DATA_READY_BIT | TEN_HOUR_PM_BIT
        );
        select_register(&mut rtc, REG_ONE_HOUR);
        assert_eq!(rtc.read_data(READY_LOW_MICROS, CPU_HZ), DATA_READY_BIT | 2);
    }

    #[test]
    fn disabling_enable_bit_returns_to_idle() {
        let mut rtc = seeded_rtc();
        rtc.write_command(COMMAND_ENABLE_BIT);
        assert_eq!(rtc.state.mode, Msm58321Mode::Command);
        rtc.write_command(0x00);
        assert_eq!(rtc.state.mode, Msm58321Mode::Idle);
    }
}
