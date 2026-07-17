//! Ricoh RP5C01 real-time clock and battery-backed nibble RAM.

use common::HostDateTime;

/// Number of register blocks in the RP5C01.
const BLOCK_COUNT: usize = 4;
/// Number of registers in each RP5C01 block.
const REGISTERS_PER_BLOCK: usize = 13;
/// Clock register block.
const CLOCK_BLOCK: usize = 0;
/// Alarm register block.
const ALARM_BLOCK: usize = 1;
/// Mode register address.
const MODE_REGISTER: u8 = 13;
/// Test register address.
const TEST_REGISTER: u8 = 14;
/// Reset register address.
const RESET_REGISTER: u8 = 15;
/// Mode bits selecting a register block.
const MODE_BLOCK_MASK: u8 = 0x03;
/// Mode bit enabling the alarm output.
const MODE_ALARM_ENABLE: u8 = 0x04;
/// Mode bit enabling clock counting.
const MODE_TIMER_ENABLE: u8 = 0x08;
/// Test bit accelerating the seconds counter.
const TEST_SECONDS: u8 = 0x01;
/// Test bit accelerating the minutes counter.
const TEST_MINUTES: u8 = 0x02;
/// Test bit accelerating the hours counter.
const TEST_HOURS: u8 = 0x04;
/// Test bit accelerating the day counter.
const TEST_DAYS: u8 = 0x08;
/// Reset bit clearing the alarm registers.
const RESET_ALARM: u8 = 0x01;
/// Reset bit clearing the fractional-second counter.
const RESET_FRACTION: u8 = 0x02;
/// RP5C01 test-clock ticks per second.
const TEST_CLOCK_HZ: u64 = 16_384;
/// Register selecting 12-hour or 24-hour time.
const HOUR_MODE_REGISTER: usize = 10;
/// Register holding the modulo-four leap-year counter.
const LEAP_YEAR_REGISTER: usize = 11;
/// PM flag stored in the tens-of-hours register in 12-hour mode.
const HOUR_PM_FLAG: u8 = 0x02;
/// Register masks for the clock and alarm blocks.
const REGISTER_MASKS: [[u8; REGISTERS_PER_BLOCK]; 2] = [
    [
        0x0F, 0x07, 0x0F, 0x07, 0x0F, 0x03, 0x07, 0x0F, 0x03, 0x0F, 0x01, 0x0F, 0x0F,
    ],
    [
        0x00, 0x00, 0x0F, 0x07, 0x0F, 0x03, 0x07, 0x0F, 0x03, 0x00, 0x01, 0x03, 0x00,
    ],
];

save_state::runtime_state! {
/// Ricoh RP5C01 clock state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rp5c01State {
    registers: [[u8; REGISTERS_PER_BLOCK]; BLOCK_COUNT],
    mode: u8,
    test: u8,
    reset: u8,
    subsecond_ticks: u64,
    cycle_remainder: u64,
    reference_cycle: u64,
}}

/// Ricoh RP5C01 real-time clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rp5c01 {
    registers: [[u8; REGISTERS_PER_BLOCK]; BLOCK_COUNT],
    mode: u8,
    test: u8,
    reset: u8,
    subsecond_ticks: u64,
    cycle_remainder: u64,
    reference_cycle: u64,
}

impl Rp5c01 {
    /// Creates a clock seeded from a host date and time.
    pub fn new(seed: HostDateTime, current_cycle: u64) -> Self {
        let mut clock = Self {
            // TODO: Replace the zeroed backup blocks if initialized HB-F1XD or
            // HB-F1XDJ hardware dumps establish model-specific contents.
            registers: [[0; REGISTERS_PER_BLOCK]; BLOCK_COUNT],
            mode: MODE_TIMER_ENABLE,
            test: 0,
            reset: 0,
            subsecond_ticks: 0,
            cycle_remainder: 0,
            reference_cycle: current_cycle,
        };
        clock.registers[ALARM_BLOCK][HOUR_MODE_REGISTER] = 1;
        clock.seed_time(seed, current_cycle);
        clock
    }

    /// Captures clock registers, controls, and fractional timing.
    pub fn capture_state(&self) -> Rp5c01State {
        Rp5c01State {
            registers: self.registers,
            mode: self.mode,
            test: self.test,
            reset: self.reset,
            subsecond_ticks: self.subsecond_ticks,
            cycle_remainder: self.cycle_remainder,
            reference_cycle: self.reference_cycle,
        }
    }

    /// Restores clock registers, controls, and fractional timing.
    pub fn restore_state(
        &mut self,
        state: Rp5c01State,
    ) -> Result<(), save_state::StateValidationError> {
        if state.mode & !0x0F != 0
            || state.test & !0x0F != 0
            || state.reset & !0x0F != 0
            || state.subsecond_ticks >= TEST_CLOCK_HZ
        {
            return Err(save_state::StateValidationError::new(
                "RP5C01 state is invalid",
            ));
        }
        self.registers = state.registers;
        self.mode = state.mode;
        self.test = state.test;
        self.reset = state.reset;
        self.subsecond_ticks = state.subsecond_ticks;
        self.cycle_remainder = state.cycle_remainder;
        self.reference_cycle = state.reference_cycle;
        Ok(())
    }

    /// Resets control registers without clearing clock or battery-backed data.
    pub fn reset(&mut self, current_cycle: u64, cpu_clock_hz: u32) {
        self.synchronize(current_cycle, cpu_clock_hz);
        self.mode = MODE_TIMER_ENABLE;
        self.test = 0;
        self.reset = 0;
        self.reference_cycle = current_cycle;
    }

    /// Replaces the running date and time from a host clock source.
    pub fn seed_time(&mut self, seed: HostDateTime, current_cycle: u64) {
        self.write_pair(0, seed.second);
        self.write_pair(2, seed.minute);
        self.write_hour(seed.hour);
        self.registers[CLOCK_BLOCK][6] = seed.day_of_week % 7;
        self.write_pair(7, seed.day);
        self.write_pair(9, seed.month);
        self.write_pair(11, (seed.year % 100) as u8);
        self.registers[ALARM_BLOCK][LEAP_YEAR_REGISTER] = (seed.year % 4) as u8;
        self.subsecond_ticks = 0;
        self.cycle_remainder = 0;
        self.reference_cycle = current_cycle;
    }

    /// Reads one four-bit register after advancing to `current_cycle`.
    pub fn read(&mut self, register: u8, current_cycle: u64, cpu_clock_hz: u32) -> u8 {
        self.synchronize(current_cycle, cpu_clock_hz);
        match register & 0x0F {
            MODE_REGISTER => self.mode,
            TEST_REGISTER | RESET_REGISTER => 0x0F,
            register => {
                let block = usize::from(self.mode & MODE_BLOCK_MASK);
                self.registers[block][usize::from(register)] & self.register_mask(block, register)
            }
        }
    }

    /// Writes one four-bit register at `current_cycle`.
    pub fn write(&mut self, register: u8, value: u8, current_cycle: u64, cpu_clock_hz: u32) {
        self.synchronize(current_cycle, cpu_clock_hz);
        let value = value & 0x0F;
        match register & 0x0F {
            MODE_REGISTER => self.mode = value,
            TEST_REGISTER => self.test = value,
            RESET_REGISTER => {
                self.reset = value;
                if value & RESET_ALARM != 0 {
                    for register in 2..=8 {
                        self.registers[ALARM_BLOCK][register] = 0;
                    }
                }
                if value & RESET_FRACTION != 0 {
                    self.subsecond_ticks = 0;
                    self.cycle_remainder = 0;
                }
            }
            register => {
                let block = usize::from(self.mode & MODE_BLOCK_MASK);
                let mask = self.register_mask(block, register);
                self.registers[block][usize::from(register)] = value & mask;
            }
        }
    }

    /// Returns one stored nibble without advancing time.
    pub fn peek_block(&self, block: usize, register: usize) -> Option<u8> {
        self.registers
            .get(block)
            .and_then(|block| block.get(register))
            .copied()
    }

    fn register_mask(&self, block: usize, register: u8) -> u8 {
        if block < REGISTER_MASKS.len() {
            REGISTER_MASKS[block][usize::from(register)]
        } else {
            0x0F
        }
    }

    fn synchronize(&mut self, current_cycle: u64, cpu_clock_hz: u32) {
        let cpu_clock_hz = u64::from(cpu_clock_hz.max(1));
        if current_cycle < self.reference_cycle {
            self.reference_cycle = current_cycle;
            self.cycle_remainder = 0;
            return;
        }
        let elapsed_cycles = current_cycle - self.reference_cycle;
        self.reference_cycle = current_cycle;
        let numerator = u128::from(self.cycle_remainder)
            + u128::from(elapsed_cycles) * u128::from(TEST_CLOCK_HZ);
        let ticks = (numerator / u128::from(cpu_clock_hz)) as u64;
        self.cycle_remainder = (numerator % u128::from(cpu_clock_hz)) as u64;
        if ticks == 0 {
            return;
        }

        let mut normal_seconds = 0;
        if self.mode & MODE_TIMER_ENABLE != 0 {
            self.subsecond_ticks += ticks;
            normal_seconds = self.subsecond_ticks / TEST_CLOCK_HZ;
            self.subsecond_ticks %= TEST_CLOCK_HZ;
        }
        let seconds = if self.test & TEST_SECONDS != 0 {
            ticks
        } else {
            normal_seconds
        };
        let minute_carry = self.add_seconds(seconds);
        let minutes = if self.test & TEST_MINUTES != 0 {
            ticks
        } else {
            minute_carry
        };
        let hour_carry = self.add_minutes(minutes);
        let hours = if self.test & TEST_HOURS != 0 {
            ticks
        } else {
            hour_carry
        };
        let day_carry = self.add_hours(hours);
        let days = if self.test & TEST_DAYS != 0 {
            ticks
        } else {
            day_carry
        };
        self.advance_days(days);
    }

    fn add_seconds(&mut self, count: u64) -> u64 {
        let total = u64::from(self.read_pair(0)) + count;
        self.write_pair(0, (total % 60) as u8);
        total / 60
    }

    fn add_minutes(&mut self, count: u64) -> u64 {
        let total = u64::from(self.read_pair(2)) + count;
        self.write_pair(2, (total % 60) as u8);
        total / 60
    }

    fn add_hours(&mut self, count: u64) -> u64 {
        let total = u64::from(self.read_hour()) + count;
        self.write_hour((total % 24) as u8);
        total / 24
    }

    fn advance_days(&mut self, mut count: u64) {
        if count == 0 {
            return;
        }
        let weekday = (u64::from(self.registers[CLOCK_BLOCK][6]) + count) % 7;
        self.registers[CLOCK_BLOCK][6] = weekday as u8;

        let mut day = self.read_pair(7).max(1);
        let mut month = self.read_pair(9).clamp(1, 12);
        while count != 0 {
            let days_in_month = self.days_in_month(month);
            let remaining = u64::from(days_in_month - day);
            if count <= remaining {
                day += count as u8;
                count = 0;
            } else {
                count -= remaining + 1;
                day = 1;
                month += 1;
                if month > 12 {
                    month = 1;
                    self.advance_years(1);
                }
            }
        }
        self.write_pair(7, day);
        self.write_pair(9, month);
    }

    fn advance_years(&mut self, count: u64) {
        if count == 0 {
            return;
        }
        let year = (u64::from(self.read_pair(11)) + count) % 100;
        self.write_pair(11, year as u8);
        let leap = (u64::from(self.registers[ALARM_BLOCK][LEAP_YEAR_REGISTER]) + count) % 4;
        self.registers[ALARM_BLOCK][LEAP_YEAR_REGISTER] = leap as u8;
    }

    fn days_in_month(&self, month: u8) -> u8 {
        match month {
            2 if self.registers[ALARM_BLOCK][LEAP_YEAR_REGISTER] == 0 => 29,
            2 => 28,
            4 | 6 | 9 | 11 => 30,
            _ => 31,
        }
    }

    fn read_pair(&self, register: usize) -> u8 {
        self.registers[CLOCK_BLOCK][register] + 10 * self.registers[CLOCK_BLOCK][register + 1]
    }

    fn write_pair(&mut self, register: usize, value: u8) {
        self.registers[CLOCK_BLOCK][register] = value % 10;
        self.registers[CLOCK_BLOCK][register + 1] = value / 10;
    }

    fn read_hour(&self) -> u8 {
        let units = self.registers[CLOCK_BLOCK][4];
        let tens = self.registers[CLOCK_BLOCK][5];
        if self.registers[ALARM_BLOCK][HOUR_MODE_REGISTER] != 0 {
            units + 10 * tens
        } else {
            let hour = units + 10 * (tens & 0x01);
            hour + if tens & HOUR_PM_FLAG != 0 { 12 } else { 0 }
        }
    }

    fn write_hour(&mut self, hour: u8) {
        if self.registers[ALARM_BLOCK][HOUR_MODE_REGISTER] != 0 {
            self.registers[CLOCK_BLOCK][4] = hour % 10;
            self.registers[CLOCK_BLOCK][5] = hour / 10;
        } else {
            let half_day_hour = hour % 12;
            self.registers[CLOCK_BLOCK][4] = half_day_hour % 10;
            self.registers[CLOCK_BLOCK][5] =
                (half_day_hour / 10) | if hour >= 12 { HOUR_PM_FLAG } else { 0 };
        }
    }

    /// Whether the alarm output is enabled by the mode register.
    pub const fn alarm_enabled(&self) -> bool {
        self.mode & MODE_ALARM_ENABLE != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test clock rate making one CPU cycle equal one RP5C01 test tick.
    const TEST_CPU_CLOCK_HZ: u32 = TEST_CLOCK_HZ as u32;

    fn seed() -> HostDateTime {
        HostDateTime {
            year: 1999,
            month: 12,
            day: 31,
            day_of_week: 5,
            hour: 23,
            minute: 59,
            second: 59,
        }
    }

    fn select_block(clock: &mut Rp5c01, block: u8) {
        clock.write(
            MODE_REGISTER,
            MODE_TIMER_ENABLE | block,
            0,
            TEST_CPU_CLOCK_HZ,
        );
    }

    #[test]
    fn one_second_rolls_the_complete_calendar() {
        let mut clock = Rp5c01::new(seed(), 0);
        assert_eq!(clock.read(0, TEST_CLOCK_HZ, TEST_CPU_CLOCK_HZ), 0);
        assert_eq!(clock.read(2, TEST_CLOCK_HZ, TEST_CPU_CLOCK_HZ), 0);
        assert_eq!(clock.read(4, TEST_CLOCK_HZ, TEST_CPU_CLOCK_HZ), 0);
        assert_eq!(clock.read(7, TEST_CLOCK_HZ, TEST_CPU_CLOCK_HZ), 1);
        assert_eq!(clock.read(9, TEST_CLOCK_HZ, TEST_CPU_CLOCK_HZ), 1);
        assert_eq!(clock.read(11, TEST_CLOCK_HZ, TEST_CPU_CLOCK_HZ), 0);
        assert_eq!(clock.read(6, TEST_CLOCK_HZ, TEST_CPU_CLOCK_HZ), 6);
        select_block(&mut clock, ALARM_BLOCK as u8);
        assert_eq!(
            clock.read(LEAP_YEAR_REGISTER as u8, TEST_CLOCK_HZ, TEST_CPU_CLOCK_HZ),
            0
        );
    }

    #[test]
    fn leap_counter_controls_february_rollover() {
        for (year, expected_day, expected_month) in [(2000, 29, 2), (2001, 1, 3)] {
            let mut clock = Rp5c01::new(
                HostDateTime {
                    year,
                    month: 2,
                    day: 28,
                    day_of_week: 1,
                    hour: 23,
                    minute: 59,
                    second: 59,
                },
                0,
            );
            assert_eq!(clock.read(0, TEST_CLOCK_HZ, TEST_CPU_CLOCK_HZ), 0);
            assert_eq!(
                clock.read(7, TEST_CLOCK_HZ, TEST_CPU_CLOCK_HZ),
                expected_day % 10
            );
            assert_eq!(
                clock.read(9, TEST_CLOCK_HZ, TEST_CPU_CLOCK_HZ),
                expected_month
            );
        }
    }

    #[test]
    fn twelve_hour_mode_encodes_the_pm_flag() {
        let mut clock = Rp5c01::new(seed(), 0);
        select_block(&mut clock, ALARM_BLOCK as u8);
        clock.write(HOUR_MODE_REGISTER as u8, 0, 0, TEST_CPU_CLOCK_HZ);
        select_block(&mut clock, CLOCK_BLOCK as u8);
        clock.seed_time(HostDateTime { hour: 13, ..seed() }, 0);
        assert_eq!(clock.read(4, 0, TEST_CPU_CLOCK_HZ), 1);
        assert_eq!(clock.read(5, 0, TEST_CPU_CLOCK_HZ), HOUR_PM_FLAG);
    }

    #[test]
    fn hold_and_fraction_reset_control_rollover() {
        let mut clock = Rp5c01::new(seed(), 0);
        clock.write(MODE_REGISTER, 0, 0, TEST_CPU_CLOCK_HZ);
        assert_eq!(clock.read(0, TEST_CLOCK_HZ * 2, TEST_CPU_CLOCK_HZ), 9);
        clock.write(
            MODE_REGISTER,
            MODE_TIMER_ENABLE,
            TEST_CLOCK_HZ * 2,
            TEST_CPU_CLOCK_HZ,
        );
        clock.write(
            RESET_REGISTER,
            RESET_FRACTION,
            TEST_CLOCK_HZ * 2 + TEST_CLOCK_HZ / 2,
            TEST_CPU_CLOCK_HZ,
        );
        assert_eq!(
            clock.read(
                0,
                TEST_CLOCK_HZ * 3 + TEST_CLOCK_HZ / 2 - 1,
                TEST_CPU_CLOCK_HZ
            ),
            9
        );
        assert_eq!(
            clock.read(0, TEST_CLOCK_HZ * 3 + TEST_CLOCK_HZ / 2, TEST_CPU_CLOCK_HZ),
            0
        );
    }

    #[test]
    fn alarm_reset_and_backup_blocks_are_independent() {
        let mut clock = Rp5c01::new(seed(), 0);
        select_block(&mut clock, ALARM_BLOCK as u8);
        clock.write(2, 7, 0, TEST_CPU_CLOCK_HZ);
        clock.write(8, 3, 0, TEST_CPU_CLOCK_HZ);
        clock.write(RESET_REGISTER, RESET_ALARM, 0, TEST_CPU_CLOCK_HZ);
        assert_eq!(clock.read(2, 0, TEST_CPU_CLOCK_HZ), 0);
        assert_eq!(clock.read(8, 0, TEST_CPU_CLOCK_HZ), 0);

        for block in [2, 3] {
            select_block(&mut clock, block);
            clock.write(4, 0x0A + block, 0, TEST_CPU_CLOCK_HZ);
        }
        select_block(&mut clock, 2);
        assert_eq!(clock.read(4, 0, TEST_CPU_CLOCK_HZ), 0x0C);
        select_block(&mut clock, 3);
        assert_eq!(clock.read(4, 0, TEST_CPU_CLOCK_HZ), 0x0D);
        clock.reset(0, TEST_CPU_CLOCK_HZ);
        select_block(&mut clock, 2);
        assert_eq!(clock.read(4, 0, TEST_CPU_CLOCK_HZ), 0x0C);
    }

    #[test]
    fn register_masks_and_write_only_reads_match_the_chip() {
        let mut clock = Rp5c01::new(seed(), 0);
        clock.write(1, 0x0F, 0, TEST_CPU_CLOCK_HZ);
        assert_eq!(clock.read(1, 0, TEST_CPU_CLOCK_HZ), 0x07);
        assert_eq!(clock.read(TEST_REGISTER, 0, TEST_CPU_CLOCK_HZ), 0x0F);
        assert_eq!(clock.read(RESET_REGISTER, 0, TEST_CPU_CLOCK_HZ), 0x0F);
    }

    #[test]
    fn test_clock_replaces_the_selected_counter_input() {
        let mut clock = Rp5c01::new(
            HostDateTime {
                second: 0,
                ..seed()
            },
            0,
        );
        clock.write(TEST_REGISTER, TEST_SECONDS, 0, TEST_CPU_CLOCK_HZ);
        assert_eq!(
            clock.read(0, TEST_CLOCK_HZ, TEST_CPU_CLOCK_HZ),
            (TEST_CLOCK_HZ % 60) as u8
        );
    }
}
