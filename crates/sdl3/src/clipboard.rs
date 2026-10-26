//! Clipboard text I/O.

use std::ffi::{CStr, CString, c_void};

use crate::{Error, get_error};

/// Sets the system clipboard to the given UTF-8 text.
///
/// Returns an error if SDL3 reports a clipboard failure or if `text`
/// contains an interior NUL byte.
pub fn set_text(text: &str) -> Result<(), Error> {
    let c_string = CString::new(text)
        .map_err(|_| Error("clipboard text contained an interior NUL byte".into()))?;
    // Safety: SDL_SetClipboardText reads the C string pointer and copies the
    // contents internally. The pointer is valid for the duration of the call.
    let ok = unsafe { sdl3_sys::clipboard::SDL_SetClipboardText(c_string.as_ptr()) };
    if ok { Ok(()) } else { Err(get_error()) }
}

/// Returns the UTF-8 text of the system clipboard. An empty clipboard gives
/// `None`.
///
/// SDL3 reports a failure as an empty text with an error message. The
/// function clears the error message of the thread before the read and
/// returns an error when an empty text comes with a message.
pub fn text() -> Result<Option<String>, Error> {
    sdl3_sys::error::SDL_ClearError();
    // Safety: SDL_GetClipboardText returns null or a null terminated C string
    // that the caller frees with SDL_free.
    let pointer = unsafe { sdl3_sys::clipboard::SDL_GetClipboardText() };
    if pointer.is_null() {
        return Err(get_error());
    }
    // Safety: the pointer is a valid null terminated C string until it is freed.
    let text = unsafe { CStr::from_ptr(pointer) }
        .to_string_lossy()
        .into_owned();
    // Safety: the string is one allocation of SDL that the caller frees with SDL_free.
    unsafe { sdl3_sys::stdinc::SDL_free(pointer.cast::<c_void>()) };
    read_result(text, get_error())
}

/// The result of a clipboard read of `text` with the error message `error`
/// of the thread after the read. A text gives itself. An empty text gives
/// `None` without a message and the error with one.
fn read_result(text: String, error: Error) -> Result<Option<String>, Error> {
    if !text.is_empty() {
        Ok(Some(text))
    } else if error.0.is_empty() {
        Ok(None)
    } else {
        Err(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies that a text gives itself also with a stale message, and that an empty text gives
    /// `None` or the error of its message.
    #[test]
    fn a_read_gives_the_text_none_or_the_error() {
        let read = read_result("ディスク".into(), Error("stale".into()));
        assert_eq!(read.ok(), Some(Some("ディスク".to_owned())));
        assert_eq!(
            read_result(String::new(), Error(String::new())).ok(),
            Some(None)
        );
        let failure = read_result(String::new(), Error("no video".into()));
        assert_eq!(failure.err().map(|error| error.0), Some("no video".into()));
    }

    /// Verifies that a read without the video subsystem reports the failure of SDL3. The tests
    /// of the crate never initialize the video subsystem.
    #[test]
    fn a_read_without_the_video_subsystem_fails() {
        let error = text().expect_err("the read needs the video subsystem");
        assert!(!error.0.is_empty());
    }
}
