//! Deterministic guest real-time clock for automated machines.

use std::sync::atomic::{AtomicU64, Ordering};

use common::{HostDateTime, HostDateTimeSource};

/// Nanoseconds in one second.
const NANOSECONDS_PER_SECOND: u64 = 1_000_000_000;

/// Guest date-time source that reports a start time plus the emulated time elapsed.
///
/// The elapsed time changes only through `set_elapsed_nanoseconds`. The host wall clock
/// never enters the reported time.
#[derive(Debug)]
pub struct GuestClock {
    start_unix_seconds: u64,
    elapsed_nanoseconds: AtomicU64,
}

impl GuestClock {
    /// Creates a clock that starts at `start` with no emulated time elapsed.
    #[must_use]
    pub fn new(start: HostDateTime) -> Self {
        Self {
            start_unix_seconds: start.to_unix_seconds(),
            elapsed_nanoseconds: AtomicU64::new(0),
        }
    }

    /// Sets the emulated time elapsed since the start time, in nanoseconds.
    pub fn set_elapsed_nanoseconds(&self, nanoseconds: u128) {
        let clamped = u64::try_from(nanoseconds).unwrap_or(u64::MAX);
        self.elapsed_nanoseconds.store(clamped, Ordering::Relaxed);
    }

    /// Returns the emulated time elapsed since the start time, in nanoseconds.
    #[must_use]
    pub fn elapsed_nanoseconds(&self) -> u64 {
        self.elapsed_nanoseconds.load(Ordering::Relaxed)
    }
}

impl HostDateTimeSource for GuestClock {
    fn now(&self) -> HostDateTime {
        let elapsed_seconds = self.elapsed_nanoseconds() / NANOSECONDS_PER_SECOND;
        HostDateTime::from_unix_seconds(self.start_unix_seconds.saturating_add(elapsed_seconds))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start() -> HostDateTime {
        HostDateTime {
            year: 1999,
            month: 12,
            day: 31,
            day_of_week: 5,
            hour: 23,
            minute: 59,
            second: 58,
        }
    }

    #[test]
    fn reports_start_time_before_emulated_time_elapses() {
        let clock = GuestClock::new(start());
        assert_eq!(clock.now(), start());
    }

    #[test]
    fn whole_seconds_of_emulated_time_advance_the_calendar() {
        let clock = GuestClock::new(start());
        clock.set_elapsed_nanoseconds(1_999_999_999);
        assert_eq!(clock.now().second, 59);
        clock.set_elapsed_nanoseconds(2_000_000_000);
        assert_eq!(
            clock.now(),
            HostDateTime {
                year: 2000,
                month: 1,
                day: 1,
                day_of_week: 6,
                hour: 0,
                minute: 0,
                second: 0,
            }
        );
    }

    #[test]
    fn elapsed_time_beyond_the_counter_range_saturates() {
        let clock = GuestClock::new(start());
        clock.set_elapsed_nanoseconds(u128::MAX);
        assert_eq!(clock.elapsed_nanoseconds(), u64::MAX);
    }
}
