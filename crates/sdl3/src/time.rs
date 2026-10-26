use sdl3_sys::{stdinc::SDL_Time, time as ffi};

use crate::Error;

/// A date and time from the host system.
pub struct DateTime {
    /// Full year (e.g. 2026).
    pub year: i32,
    /// Month (1–12).
    pub month: i32,
    /// Day of the month (1–31).
    pub day: i32,
    /// Hour (0–23).
    pub hour: i32,
    /// Minute (0–59).
    pub minute: i32,
    /// Second (0–59).
    pub second: i32,
    /// Day of the week (0 = Sunday).
    pub day_of_week: i32,
}

/// Returns the current local date and time.
pub fn local_date_time() -> Result<DateTime, Error> {
    let mut ticks: SDL_Time = 0;
    let mut dt = ffi::SDL_DateTime::default();

    // Safety: SDL3 time functions are thread-safe and only write to the provided pointers.
    let ok = unsafe {
        ffi::SDL_GetCurrentTime(&raw mut ticks) && ffi::SDL_TimeToDateTime(ticks, &raw mut dt, true)
    };

    if !ok {
        return Err(crate::get_error());
    }

    Ok(DateTime {
        year: dt.year,
        month: dt.month,
        day: dt.day,
        hour: dt.hour,
        minute: dt.minute,
        second: dt.second,
        day_of_week: dt.day_of_week,
    })
}
