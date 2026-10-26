//! Filesystem path utilities.

use std::{
    ffi::{CStr, CString, c_void},
    path::PathBuf,
};

/// Returns the OS-specific user data directory for the given organization and application.
///
/// The returned path is suitable for storing user preferences and configuration files.
/// The directory is created automatically if it does not exist.
///
/// Platform-specific locations:
/// - Linux: `$XDG_DATA_HOME/<app>/` (typically `~/.local/share/<app>/`)
/// - Windows: `%APPDATA%\<org>\<app>\`
/// - macOS: `~/Library/Application Support/<app>/`
///
/// Returns `None` if the path cannot be determined.
pub fn get_pref_path(org: &str, app: &str) -> Option<PathBuf> {
    let org = CString::new(org).ok()?;
    let app = CString::new(app).ok()?;

    // Safety: SDL_GetPrefPath accepts valid C strings and returns either a valid
    // heap-allocated C string or null. It does not require SDL_Init().
    unsafe {
        let ptr = sdl3_sys::filesystem::SDL_GetPrefPath(org.as_ptr(), app.as_ptr());
        if ptr.is_null() {
            return None;
        }
        let path = CStr::from_ptr(ptr).to_string_lossy().into_owned();
        sdl3_sys::stdinc::SDL_free(ptr as *mut c_void);
        Some(PathBuf::from(path))
    }
}
