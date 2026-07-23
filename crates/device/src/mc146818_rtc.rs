//! MC146818-compatible real-time clock and 128-byte CMOS RAM for the PC/AT.
//!
//! The RTC seeds its time and calendar from the host clock once at
//! construction and then free-runs: the machine schedules a one-second update
//! event and an optional periodic event, and the device advances its own
//! registers on those ticks. The CMOS RAM is volatile, seeded from a
//! configuration-derived default image the machine synthesizes.

use common::HostDateTime;

/// CMOS RAM size in bytes.
pub const CMOS_SIZE: usize = 128;

save_state::runtime_state! {
/// Authoritative MC146818 calendar and CMOS state.
#[derive(Debug, Clone)]
pub struct Mc146818RtcState {
    cmos: [u8; CMOS_SIZE],
    address: u8,
    next_update_cycle: u64,
}}

/// Register index: seconds.
const REG_SECONDS: usize = 0x00;
/// Register index: seconds alarm.
const REG_SECONDS_ALARM: usize = 0x01;
/// Register index: minutes.
const REG_MINUTES: usize = 0x02;
/// Register index: minutes alarm.
const REG_MINUTES_ALARM: usize = 0x03;
/// Register index: hours.
const REG_HOURS: usize = 0x04;
/// Register index: hours alarm.
const REG_HOURS_ALARM: usize = 0x05;
/// Register index: day of week (1-7, Sunday = 1).
const REG_DAY_OF_WEEK: usize = 0x06;
/// Register index: day of month.
const REG_DAY_OF_MONTH: usize = 0x07;
/// Register index: month (1-12).
const REG_MONTH: usize = 0x08;
/// Register index: year (two digits).
const REG_YEAR: usize = 0x09;
/// Register index: control register A.
const REG_A: usize = 0x0A;
/// Register index: control register B.
const REG_B: usize = 0x0B;
/// Register index: control register C (flags).
const REG_C: usize = 0x0C;
/// Register index: control register D (valid RAM/time).
const REG_D: usize = 0x0D;
/// Register index: century (BCD), per the AMI/DS12885 convention.
const REG_CENTURY: usize = 0x32;

/// Register A: update-in-progress flag.
const REG_A_UIP: u8 = 0x80;
/// Register A: rate-select field mask (bits 3:0).
const REG_A_RATE_MASK: u8 = 0x0F;

/// Register B: block clock updates (SET).
const REG_B_SET: u8 = 0x80;
/// Register B: periodic interrupt enable.
const REG_B_PIE: u8 = 0x40;
/// Register B: alarm interrupt enable.
const REG_B_AIE: u8 = 0x20;
/// Register B: update-ended interrupt enable.
const REG_B_UIE: u8 = 0x10;
/// Register B: data mode (1 = binary, 0 = BCD).
const REG_B_BINARY: u8 = 0x04;
/// Register B: hour format (1 = 24-hour, 0 = 12-hour).
const REG_B_24HOUR: u8 = 0x02;

/// Register C: interrupt request flag.
const REG_C_IRQF: u8 = 0x80;
/// Register C: periodic interrupt flag.
const REG_C_PF: u8 = 0x40;
/// Register C: alarm interrupt flag.
const REG_C_AF: u8 = 0x20;
/// Register C: update-ended interrupt flag.
const REG_C_UF: u8 = 0x10;

/// Register D: valid RAM and time (battery good).
const REG_D_VRT: u8 = 0x80;

/// Hours register PM flag in 12-hour mode.
const HOUR_PM_FLAG: u8 = 0x80;

/// Update-in-progress window in microseconds (244 us lead + 1984 us update).
const UIP_WINDOW_MICROS: u64 = 2228;

/// Effect of an RTC register write, for the bus to act on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RtcWriteEffect {
    /// The periodic-interrupt rate changed; reschedule the periodic event.
    pub reschedule_periodic: bool,
    /// The update behavior changed (SET bit); reschedule the update event.
    pub reschedule_update: bool,
}

/// MC146818 RTC with 128 bytes of CMOS RAM.
pub struct Mc146818Rtc {
    /// CMOS RAM including the time/alarm/control registers at 0x00-0x0D.
    pub cmos: [u8; CMOS_SIZE],
    /// Currently selected register (latched via port 0x70).
    pub address: u8,
    /// CPU cycle at which the next one-second update falls (UIP anchor).
    pub next_update_cycle: u64,
}

impl Mc146818Rtc {
    /// Creates the RTC, seeding the time/calendar from the host clock and the
    /// CMOS RAM from the supplied default image.
    pub fn new(seed: HostDateTime, cmos_seed: &[u8; CMOS_SIZE]) -> Self {
        let mut rtc = Self {
            cmos: *cmos_seed,
            address: 0,
            next_update_cycle: 0,
        };
        rtc.seed_time(seed);
        rtc
    }

    /// Captures the CMOS bytes, address latch, and update phase.
    pub fn capture_state(&self) -> Mc146818RtcState {
        Mc146818RtcState {
            cmos: self.cmos,
            address: self.address,
            next_update_cycle: self.next_update_cycle,
        }
    }

    /// Restores the CMOS bytes, address latch, and update phase.
    pub fn restore_state(
        &mut self,
        state: Mc146818RtcState,
    ) -> Result<(), save_state::StateValidationError> {
        if state.address as usize >= CMOS_SIZE {
            return Err(save_state::StateValidationError::new(
                "RTC address latch is invalid",
            ));
        }
        self.cmos = state.cmos;
        self.address = state.address;
        self.next_update_cycle = state.next_update_cycle;
        Ok(())
    }

    /// Reseeds the time/calendar registers from a new host date-time value.
    pub fn reseed_time(&mut self, seed: HostDateTime) {
        self.seed_time(seed);
    }

    /// Writes the time/calendar registers from a host date-time value.
    fn seed_time(&mut self, seed: HostDateTime) {
        self.set_field(REG_SECONDS, seed.second);
        self.set_field(REG_MINUTES, seed.minute);
        self.set_hour24(seed.hour);
        self.set_field(REG_DAY_OF_WEEK, seed.day_of_week + 1);
        self.set_field(REG_DAY_OF_MONTH, seed.day);
        self.set_field(REG_MONTH, seed.month);
        self.set_field(REG_YEAR, (seed.year % 100) as u8);
        self.cmos[REG_CENTURY] = to_bcd((seed.year / 100) as u8);
    }

    /// Latches the register selected by a port-0x70 write.
    pub fn set_address(&mut self, address: u8) {
        self.address = address & 0x7F;
    }

    /// Sets the anchor cycle for the next one-second update (UIP timing).
    pub fn set_next_update_cycle(&mut self, cycle: u64) {
        self.next_update_cycle = cycle;
    }

    /// Reads the currently selected register (port 0x71 read).
    pub fn read(&mut self, current_cycle: u64, cpu_clock_hz: u32) -> u8 {
        match self.address as usize {
            REG_A => {
                let mut value = self.cmos[REG_A] & !REG_A_UIP;
                if self.update_in_progress(current_cycle, cpu_clock_hz) {
                    value |= REG_A_UIP;
                }
                value
            }
            REG_C => {
                // Reading register C clears all flags and deasserts the IRQ.
                let value = self.cmos[REG_C];
                self.cmos[REG_C] = 0;
                value
            }
            REG_D => REG_D_VRT,
            register => self.cmos[register],
        }
    }

    /// Writes the currently selected register (port 0x71 write).
    pub fn write(&mut self, value: u8) -> RtcWriteEffect {
        let mut effect = RtcWriteEffect::default();
        match self.address as usize {
            REG_A => {
                // The UIP bit is read-only; preserve the rest.
                self.cmos[REG_A] = value & !REG_A_UIP;
                effect.reschedule_periodic = true;
            }
            REG_B => {
                self.cmos[REG_B] = value;
                effect.reschedule_update = true;
                effect.reschedule_periodic = true;
            }
            REG_C | REG_D => {} // Read-only flag/status registers.
            register => self.cmos[register] = value,
        }
        effect
    }

    /// Returns whether the update-in-progress flag currently reads high.
    fn update_in_progress(&self, current_cycle: u64, cpu_clock_hz: u32) -> bool {
        if self.cmos[REG_B] & REG_B_SET != 0 {
            return false;
        }
        let window = UIP_WINDOW_MICROS * cpu_clock_hz as u64 / 1_000_000;
        self.next_update_cycle.saturating_sub(current_cycle) < window
    }

    /// Advances the clock by one second, updating flags and returning whether
    /// IRQ 8 should be asserted.
    pub fn advance_one_second(&mut self) -> bool {
        if self.cmos[REG_B] & REG_B_SET != 0 {
            return false;
        }
        self.increment_time();

        self.cmos[REG_C] |= REG_C_UF;
        if self.alarm_matches() {
            self.cmos[REG_C] |= REG_C_AF;
        }
        self.refresh_irqf()
    }

    /// Fires a periodic tick, setting the periodic flag and returning whether
    /// IRQ 8 should be asserted.
    pub fn periodic_tick(&mut self) -> bool {
        self.cmos[REG_C] |= REG_C_PF;
        self.refresh_irqf()
    }

    /// Recomputes the IRQF bit from the enabled flags and returns whether the
    /// interrupt line is asserted.
    fn refresh_irqf(&mut self) -> bool {
        let flags = self.cmos[REG_C];
        let enables = self.cmos[REG_B];
        let asserted = (flags & REG_C_PF != 0 && enables & REG_B_PIE != 0)
            || (flags & REG_C_AF != 0 && enables & REG_B_AIE != 0)
            || (flags & REG_C_UF != 0 && enables & REG_B_UIE != 0);
        if asserted {
            self.cmos[REG_C] |= REG_C_IRQF;
        }
        asserted
    }

    /// Returns the periodic interrupt period in CPU cycles, or `None` if the
    /// periodic timer is disabled.
    pub fn periodic_period_cycles(&self, cpu_clock_hz: u32) -> Option<u64> {
        let rate = self.cmos[REG_A] & REG_A_RATE_MASK;
        let frequency = match rate {
            0 => return None,
            1 => 256,
            2 => 128,
            other => 65536u32 >> other,
        };
        Some((cpu_clock_hz as u64 / frequency as u64).max(1))
    }

    /// Increments the time and calendar registers with carry.
    fn increment_time(&mut self) {
        let mut seconds = self.get_field(REG_SECONDS) + 1;
        if seconds < 60 {
            self.set_field(REG_SECONDS, seconds);
            return;
        }
        seconds = 0;
        self.set_field(REG_SECONDS, seconds);

        let mut minutes = self.get_field(REG_MINUTES) + 1;
        if minutes < 60 {
            self.set_field(REG_MINUTES, minutes);
            return;
        }
        minutes = 0;
        self.set_field(REG_MINUTES, minutes);

        let mut hours = self.get_hour24() + 1;
        if hours < 24 {
            self.set_hour24(hours);
            return;
        }
        hours = 0;
        self.set_hour24(hours);

        let day_of_week = self.get_field(REG_DAY_OF_WEEK) % 7 + 1;
        self.set_field(REG_DAY_OF_WEEK, day_of_week);

        let day = self.get_field(REG_DAY_OF_MONTH);
        let month = self.get_field(REG_MONTH);
        let year = self.get_field(REG_YEAR);
        if day < days_in_month(month, year) {
            self.set_field(REG_DAY_OF_MONTH, day + 1);
            return;
        }
        self.set_field(REG_DAY_OF_MONTH, 1);

        if month < 12 {
            self.set_field(REG_MONTH, month + 1);
            return;
        }
        self.set_field(REG_MONTH, 1);
        self.set_field(REG_YEAR, (year + 1) % 100);
    }

    /// Returns whether the alarm registers match the current time, treating
    /// any alarm byte in 0xC0-0xFF as a don't-care.
    fn alarm_matches(&self) -> bool {
        alarm_byte_matches(self.cmos[REG_SECONDS], self.cmos[REG_SECONDS_ALARM])
            && alarm_byte_matches(self.cmos[REG_MINUTES], self.cmos[REG_MINUTES_ALARM])
            && alarm_byte_matches(self.cmos[REG_HOURS], self.cmos[REG_HOURS_ALARM])
    }

    /// Reads a numeric time field, decoding BCD when register B selects it.
    fn get_field(&self, register: usize) -> u8 {
        if self.cmos[REG_B] & REG_B_BINARY != 0 {
            self.cmos[register]
        } else {
            from_bcd(self.cmos[register])
        }
    }

    /// Writes a numeric time field, encoding BCD when register B selects it.
    fn set_field(&mut self, register: usize, value: u8) {
        self.cmos[register] = if self.cmos[REG_B] & REG_B_BINARY != 0 {
            value
        } else {
            to_bcd(value)
        };
    }

    /// Reads the hours register as a 0-23 value, resolving 12-hour mode.
    fn get_hour24(&self) -> u8 {
        let raw = self.cmos[REG_HOURS];
        let binary = self.cmos[REG_B] & REG_B_BINARY != 0;
        if self.cmos[REG_B] & REG_B_24HOUR != 0 {
            if binary { raw } else { from_bcd(raw) }
        } else {
            let pm = raw & HOUR_PM_FLAG != 0;
            let masked = raw & !HOUR_PM_FLAG;
            let mut hour = if binary { masked } else { from_bcd(masked) };
            if hour == 12 {
                hour = 0;
            }
            if pm {
                hour += 12;
            }
            hour
        }
    }

    /// Writes the hours register from a 0-23 value, honoring the hour format.
    fn set_hour24(&mut self, hour24: u8) {
        let binary = self.cmos[REG_B] & REG_B_BINARY != 0;
        if self.cmos[REG_B] & REG_B_24HOUR != 0 {
            self.cmos[REG_HOURS] = if binary { hour24 } else { to_bcd(hour24) };
        } else {
            let pm = hour24 >= 12;
            let mut hour12 = hour24 % 12;
            if hour12 == 0 {
                hour12 = 12;
            }
            let encoded = if binary { hour12 } else { to_bcd(hour12) };
            self.cmos[REG_HOURS] = encoded | if pm { HOUR_PM_FLAG } else { 0 };
        }
    }
}

/// Returns whether a time byte matches an alarm byte (0xC0-0xFF = don't care).
fn alarm_byte_matches(time: u8, alarm: u8) -> bool {
    alarm >= 0xC0 || time == alarm
}

/// Returns the number of days in a month, using the two-digit year for the
/// leap-year test (valid across a single century).
fn days_in_month(month: u8, two_digit_year: u8) -> u8 {
    match month {
        4 | 6 | 9 | 11 => 30,
        2 => {
            if two_digit_year.is_multiple_of(4) {
                29
            } else {
                28
            }
        }
        _ => 31,
    }
}

/// Converts a decimal value below one hundred to packed BCD.
fn to_bcd(value: u8) -> u8 {
    ((value / 10) << 4) | (value % 10)
}

/// Converts a packed BCD byte to its binary value.
fn from_bcd(value: u8) -> u8 {
    (value >> 4) * 10 + (value & 0x0F)
}
