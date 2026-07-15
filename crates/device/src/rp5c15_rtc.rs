//! Ricoh RP5C15 real-time clock.

/// RP5C15 oscillator frequency.
pub const RP5C15_CLOCK_HZ: u64 = 32_768;

/// Mode-register bit selecting register bank 1.
const MODE_BANK: u8 = 0x01;
/// Mode-register bit enabling calendar alarm comparison.
const MODE_ALARM_ENABLE: u8 = 0x04;
/// Mode-register bit enabling calendar advancement.
const MODE_TIMER_ENABLE: u8 = 0x08;
/// Reset-register bit clearing all programmed alarm fields.
const RESET_ALARM: u8 = 0x01;
/// Reset-register bit clearing the subsecond timer state.
const RESET_TIMER: u8 = 0x02;
/// Reset-register bit suppressing the 16 Hz ALARM output component.
const RESET_16_HZ: u8 = 0x04;
/// Reset-register bit suppressing the 1 Hz ALARM output component.
const RESET_1_HZ: u8 = 0x08;

save_state::runtime_state! {
/// Ricoh RP5C15 real-time clock.
#[derive(Debug, Clone)]
pub struct Rp5c15Rtc {
    bank0: [u8; 16],
    bank1: [u8; 16],
    alarm_valid: [bool; 7],
    mode: u8,
    test: u8,
    reset: u8,
    current_tick: u64,
    subsecond_ticks: u64,
    stopped_second_pending: bool,
    seeded: bool,
}}

impl Rp5c15Rtc {
    /// Captures complete calendar and oscillator phase state.
    pub fn capture_state(&self) -> Self {
        self.clone()
    }

    /// Restores complete calendar and oscillator phase state.
    pub fn restore_state(&mut self, state: Self) {
        *self = state;
    }
}

impl Default for Rp5c15Rtc {
    fn default() -> Self {
        Self::new()
    }
}

impl Rp5c15Rtc {
    /// Creates a running, unseeded RTC in 24-hour mode.
    pub const fn new() -> Self {
        let mut bank1 = [0; 16];
        bank1[10] = 1;
        Self {
            bank0: [0; 16],
            bank1,
            alarm_valid: [false; 7],
            mode: MODE_TIMER_ENABLE,
            test: 0,
            reset: RESET_16_HZ | RESET_1_HZ,
            current_tick: 0,
            subsecond_ticks: 0,
            stopped_second_pending: false,
            seeded: false,
        }
    }

    /// Resets register state and timekeeping.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Seeds the clock once from calendar BCD values.
    ///
    /// The input is `[year, month_and_weekday, day, hour, minute, second]`.
    pub fn seed_from_calendar_bcd(&mut self, value: [u8; 6]) {
        if self.seeded {
            return;
        }
        let [year, month_weekday, day, hour, minute, second] = value;
        set_pair(&mut self.bank0, 0, bcd_to_binary(second));
        set_pair(&mut self.bank0, 2, bcd_to_binary(minute));
        set_pair(&mut self.bank0, 4, bcd_to_binary(hour));
        self.bank0[6] = month_weekday & 7;
        set_pair(&mut self.bank0, 7, bcd_to_binary(day));
        set_pair(&mut self.bank0, 9, month_weekday >> 4);
        set_pair(&mut self.bank0, 11, bcd_to_binary(year));
        self.bank1[11] = bcd_to_binary(year) & 3;
        self.seeded = true;
    }

    /// Returns whether the clock has received its host seed.
    pub const fn seeded(&self) -> bool {
        self.seeded
    }

    /// Reads one of the sixteen register offsets.
    pub fn read_register(&mut self, register: u8, tick: u64) -> u8 {
        self.advance_to(tick);
        let register = usize::from(register & 0x0F);
        match register {
            13 => self.mode,
            14 | 15 => 0,
            _ if self.mode & MODE_BANK != 0 => self.bank1[register],
            _ => self.bank0[register],
        }
    }

    /// Writes one of the sixteen register offsets.
    pub fn write_register(&mut self, register: u8, value: u8, tick: u64) {
        self.advance_to(tick);
        let register = usize::from(register & 0x0F);
        match register {
            13 => self.write_mode(value),
            14 => self.test = value & 0x0F,
            15 => self.write_reset(value),
            _ if self.mode & MODE_BANK != 0 => self.write_bank1(register, value),
            _ => self.bank0[register] = value & BANK0_MASKS[register],
        }
    }

    /// Advances the RTC through `tick`.
    pub fn advance_to(&mut self, tick: u64) {
        if tick <= self.current_tick {
            return;
        }
        let elapsed = tick - self.current_tick;
        self.current_tick = tick;

        if self.test != 0 {
            self.advance_test(elapsed);
        }

        self.subsecond_ticks += elapsed;
        let seconds = self.subsecond_ticks / RP5C15_CLOCK_HZ;
        self.subsecond_ticks %= RP5C15_CLOCK_HZ;
        if seconds == 0 {
            return;
        }
        if self.mode & MODE_TIMER_ENABLE != 0 {
            for _ in 0..seconds {
                self.increment_second();
            }
        } else {
            self.stopped_second_pending = true;
        }
    }

    /// Returns the next output or counter transition.
    pub const fn next_event_tick(&self) -> Option<u64> {
        Some(self.current_tick + (1024 - self.current_tick % 1024))
    }

    /// Returns the active-high electrical ALARM pin level.
    pub fn alarm_level(&self) -> bool {
        if self.mode & MODE_ALARM_ENABLE != 0 {
            return self.alarm_matches();
        }
        let stage = self.current_tick / 1024 % 32;
        match self.reset >> 2 & 3 {
            0 => stage < 16 && stage & 1 == 0,
            1 => stage < 16,
            2 => stage & 1 == 0,
            _ => false,
        }
    }

    /// Returns the CLKOUT pin, or `None` for high impedance.
    pub fn clkout_level(&self) -> Option<bool> {
        let select = self.bank1[0] & 7;
        match select {
            0 => None,
            1 => Some(self.current_tick & 1 == 0),
            2 => Some((self.current_tick / 16) & 1 == 0),
            3 => Some((self.current_tick / 128) & 1 == 0),
            4 => Some((self.current_tick / 1024) & 1 == 0),
            5 => Some((self.current_tick / 16_384) & 1 == 0),
            6 => Some(read_pair(&self.bank0, 0) < 30),
            _ => Some(false),
        }
    }

    fn write_mode(&mut self, value: u8) {
        let old_running = self.mode & MODE_TIMER_ENABLE != 0;
        self.mode = value & (MODE_BANK | MODE_ALARM_ENABLE | MODE_TIMER_ENABLE);
        if !old_running && self.mode & MODE_TIMER_ENABLE != 0 && self.stopped_second_pending {
            self.stopped_second_pending = false;
            self.increment_second();
        }
    }

    fn write_reset(&mut self, value: u8) {
        self.reset = value & 0x0F;
        if value & RESET_ALARM != 0 {
            self.alarm_valid = [false; 7];
        }
        if value & RESET_TIMER != 0 {
            self.subsecond_ticks = 0;
            self.stopped_second_pending = false;
        }
    }

    fn write_bank1(&mut self, register: usize, value: u8) {
        self.bank1[register] = value & BANK1_MASKS[register];
        if (2..=8).contains(&register) {
            self.alarm_valid[register - 2] = true;
        }
        if register == 1 && value & 1 != 0 {
            let seconds = read_pair(&self.bank0, 0);
            if seconds >= 30 {
                self.increment_minute();
            }
            set_pair(&mut self.bank0, 0, 0);
        }
    }

    fn advance_test(&mut self, elapsed: u64) {
        let pulses = elapsed / 2;
        if pulses == 0 {
            return;
        }
        if self.test & 1 != 0 {
            for _ in 0..pulses.min(100_000) {
                self.increment_second();
            }
        }
        if self.test & 2 != 0 {
            for _ in 0..pulses.min(100_000) {
                self.increment_minute();
            }
        }
        if self.test & 4 != 0 {
            for _ in 0..pulses.min(100_000) {
                self.increment_day();
            }
        }
        if self.test & 8 != 0 {
            let years = pulses % 100;
            let year = (read_pair(&self.bank0, 11) + years as u8) % 100;
            set_pair(&mut self.bank0, 11, year);
            self.bank1[11] = (self.bank1[11] + years as u8) & 3;
        }
    }

    fn increment_second(&mut self) {
        let second = read_pair(&self.bank0, 0) + 1;
        if second < 60 {
            set_pair(&mut self.bank0, 0, second);
        } else {
            set_pair(&mut self.bank0, 0, 0);
            self.increment_minute();
        }
    }

    fn increment_minute(&mut self) {
        let minute = read_pair(&self.bank0, 2) + 1;
        if minute < 60 {
            set_pair(&mut self.bank0, 2, minute);
        } else {
            set_pair(&mut self.bank0, 2, 0);
            self.increment_hour();
        }
    }

    fn increment_hour(&mut self) {
        let hour = self.hour_24() + 1;
        if hour < 24 {
            self.set_hour_24(hour);
        } else {
            self.set_hour_24(0);
            self.increment_day();
        }
    }

    fn increment_day(&mut self) {
        self.bank0[6] = (self.bank0[6] + 1) % 7;
        let day = read_pair(&self.bank0, 7) + 1;
        let month = read_pair(&self.bank0, 9).clamp(1, 12);
        if day <= days_in_month(month, self.bank1[11]) {
            set_pair(&mut self.bank0, 7, day);
            return;
        }
        set_pair(&mut self.bank0, 7, 1);
        if month < 12 {
            set_pair(&mut self.bank0, 9, month + 1);
        } else {
            set_pair(&mut self.bank0, 9, 1);
            let year = (read_pair(&self.bank0, 11) + 1) % 100;
            set_pair(&mut self.bank0, 11, year);
            self.bank1[11] = (self.bank1[11] + 1) & 3;
        }
    }

    fn hour_24(&self) -> u8 {
        let raw = read_pair(&self.bank0, 4);
        if self.bank1[10] & 1 != 0 {
            raw
        } else {
            let pm = self.bank0[5] & 2 != 0;
            let hour = ((self.bank0[5] & 1) * 10 + self.bank0[4]).clamp(1, 12);
            match (pm, hour) {
                (false, 12) => 0,
                (false, _) => hour,
                (true, 12) => 12,
                (true, _) => hour + 12,
            }
        }
    }

    fn set_hour_24(&mut self, hour: u8) {
        if self.bank1[10] & 1 != 0 {
            set_pair(&mut self.bank0, 4, hour);
        } else {
            let pm = hour >= 12;
            let display = match hour % 12 {
                0 => 12,
                value => value,
            };
            self.bank0[4] = display % 10;
            self.bank0[5] = (display / 10) | if pm { 2 } else { 0 };
        }
    }

    fn alarm_matches(&self) -> bool {
        (2..=8).all(|register| {
            !self.alarm_valid[register - 2] || self.bank1[register] == self.bank0[register]
        })
    }
}

/// Writable-bit masks for timekeeping bank 0.
const BANK0_MASKS: [u8; 16] = [
    0x0F, 0x07, 0x0F, 0x07, 0x0F, 0x03, 0x07, 0x0F, 0x03, 0x0F, 0x01, 0x0F, 0x0F, 0, 0, 0,
];
/// Writable-bit masks for alarm and configuration bank 1.
const BANK1_MASKS: [u8; 16] = [
    0x07, 0x01, 0x0F, 0x07, 0x0F, 0x07, 0x07, 0x0F, 0x03, 0, 0x01, 0x03, 0, 0, 0, 0,
];

fn read_pair(registers: &[u8; 16], offset: usize) -> u8 {
    registers[offset] + registers[offset + 1] * 10
}

fn set_pair(registers: &mut [u8; 16], offset: usize, value: u8) {
    registers[offset] = value % 10;
    registers[offset + 1] = value / 10;
}

fn bcd_to_binary(value: u8) -> u8 {
    (value >> 4) * 10 + (value & 0x0F)
}

fn days_in_month(month: u8, leap_counter: u8) -> u8 {
    match month {
        2 if leap_counter & 3 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeded() -> Rp5c15Rtc {
        let mut rtc = Rp5c15Rtc::new();
        rtc.seed_from_calendar_bcd([0x44, 0x22, 0x28, 0x23, 0x59, 0x59]);
        rtc
    }

    #[test]
    fn leap_day_rolls_through_midnight() {
        let mut rtc = seeded();
        rtc.advance_to(RP5C15_CLOCK_HZ);
        assert_eq!(rtc.read_register(0, RP5C15_CLOCK_HZ), 0);
        assert_eq!(rtc.read_register(7, RP5C15_CLOCK_HZ), 9);
        assert_eq!(rtc.read_register(8, RP5C15_CLOCK_HZ), 2);
        assert_eq!(rtc.read_register(6, RP5C15_CLOCK_HZ), 3);
    }

    #[test]
    fn stopped_clock_applies_one_pending_second() {
        let mut rtc = seeded();
        rtc.write_register(13, 0, 0);
        rtc.advance_to(RP5C15_CLOCK_HZ * 3);
        rtc.write_register(13, MODE_TIMER_ENABLE, RP5C15_CLOCK_HZ * 3);
        assert_eq!(rtc.read_register(0, RP5C15_CLOCK_HZ * 3), 0);
        assert_eq!(rtc.read_register(2, RP5C15_CLOCK_HZ * 3), 0);
    }

    #[test]
    fn alarm_fields_are_dont_care_after_reset() {
        let mut rtc = seeded();
        rtc.write_register(13, MODE_BANK | MODE_ALARM_ENABLE | MODE_TIMER_ENABLE, 0);
        rtc.write_register(15, RESET_ALARM | RESET_16_HZ | RESET_1_HZ, 0);
        assert!(rtc.alarm_level());
        rtc.write_register(2, 8, 0);
        assert!(!rtc.alarm_level());
    }

    #[test]
    fn host_seed_is_used_only_once() {
        let mut rtc = seeded();
        rtc.seed_from_calendar_bcd([0x99, 0x11, 0x01, 0, 0, 0]);
        assert_eq!(rtc.read_register(11, 0), 4);
        assert_eq!(rtc.read_register(12, 0), 4);
    }
}
