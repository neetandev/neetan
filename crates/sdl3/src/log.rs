//! SDL3 log output capture.

use std::ffi::{CStr, c_char, c_int, c_void};

use sdl3_sys::log::{SDL_LogPriority, SDL_SetLogOutputFunction, SDL_SetLogPriorities};

/// Log priority levels matching SDL3's `SDL_LogPriority`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogPriority {
    /// Trace level (most verbose).
    Trace,
    /// Verbose level.
    Verbose,
    /// Debug level.
    Debug,
    /// Info level.
    Info,
    /// Warning level.
    Warn,
    /// Error level.
    Error,
    /// Critical level (least verbose).
    Critical,
}

impl LogPriority {
    fn to_sdl(self) -> SDL_LogPriority {
        match self {
            LogPriority::Trace => SDL_LogPriority::TRACE,
            LogPriority::Verbose => SDL_LogPriority::VERBOSE,
            LogPriority::Debug => SDL_LogPriority::DEBUG,
            LogPriority::Info => SDL_LogPriority::INFO,
            LogPriority::Warn => SDL_LogPriority::WARN,
            LogPriority::Error => SDL_LogPriority::ERROR,
            LogPriority::Critical => SDL_LogPriority::CRITICAL,
        }
    }

    /// Converts from SDL3's priority value.
    pub fn from_sdl(priority: SDL_LogPriority) -> Option<Self> {
        match priority {
            SDL_LogPriority::TRACE => Some(LogPriority::Trace),
            SDL_LogPriority::VERBOSE => Some(LogPriority::Verbose),
            SDL_LogPriority::DEBUG => Some(LogPriority::Debug),
            SDL_LogPriority::INFO => Some(LogPriority::Info),
            SDL_LogPriority::WARN => Some(LogPriority::Warn),
            SDL_LogPriority::ERROR => Some(LogPriority::Error),
            SDL_LogPriority::CRITICAL => Some(LogPriority::Critical),
            _ => None,
        }
    }
}

/// A callback that receives SDL3 log messages.
///
/// Parameters: `(category: i32, priority: LogPriority, message: &str)`.
pub type LogCallback = fn(i32, LogPriority, &str);

/// Sets the SDL3 log output function to a Rust callback.
///
/// All SDL3 internal log messages (audio, video, GPU, etc.) will be routed
/// through the given callback.
pub fn set_log_output_function(callback: LogCallback) {
    // Safety: SDL_SetLogOutputFunction is thread-safe per SDL3 docs.
    // We pass the function pointer as userdata and use a thin C trampoline.
    unsafe {
        SDL_SetLogOutputFunction(Some(log_trampoline), callback as *mut c_void);
    }
}

/// Sets the minimum priority for all SDL3 log categories.
pub fn set_log_priorities(priority: LogPriority) {
    // Safety: SDL_SetLogPriorities is thread-safe per SDL3 docs.
    unsafe {
        SDL_SetLogPriorities(priority.to_sdl());
    }
}

unsafe extern "C" fn log_trampoline(
    userdata: *mut c_void,
    category: c_int,
    priority: SDL_LogPriority,
    message: *const c_char,
) {
    let callback: LogCallback = unsafe { std::mem::transmute(userdata) };
    let priority = LogPriority::from_sdl(priority).unwrap_or(LogPriority::Info);
    let msg = if message.is_null() {
        ""
    } else {
        // Safety: SDL3 guarantees the message is a valid null-terminated C string.
        unsafe { CStr::from_ptr(message) }
            .to_str()
            .unwrap_or("<invalid UTF-8>")
    };
    callback(category, priority, msg);
}
