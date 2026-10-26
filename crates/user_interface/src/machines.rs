//! Host-side machine services: the SDL wall clock.

use common::{HostDateTime, HostDateTimeSource, SharedHostDateTimeSource};
pub use machine_factory::machines::initialize_machine;

/// Host date-time source backed by the SDL local wall clock.
struct SdlHostDateTime;

impl HostDateTimeSource for SdlHostDateTime {
    fn now(&self) -> HostDateTime {
        host_date_time()
    }
}

/// Returns a shared host date-time source backed by the SDL wall clock.
pub(crate) fn sdl_host_date_time_source() -> SharedHostDateTimeSource {
    std::sync::Arc::new(SdlHostDateTime)
}

/// Returns the current host local date and time for emulated RTCs.
fn host_date_time() -> HostDateTime {
    let Ok(dt) = sdl3::time::local_date_time() else {
        return HostDateTime {
            year: 0,
            month: 0,
            day: 0,
            day_of_week: 0,
            hour: 0,
            minute: 0,
            second: 0,
        };
    };
    HostDateTime {
        year: dt.year as u16,
        month: dt.month as u8,
        day: dt.day as u8,
        day_of_week: dt.day_of_week as u8,
        hour: dt.hour as u8,
        minute: dt.minute as u8,
        second: dt.second as u8,
    }
}
