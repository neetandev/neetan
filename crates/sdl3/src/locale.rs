//! Preferred locales of the user.

use std::ffi::{CStr, c_char, c_int, c_void};

/// A locale: an ISO 639 language and an optional ISO 3166 country.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locale {
    /// The language, for example `en` or `ja`.
    pub language: String,
    /// The country, for example `US` or `JP`. A locale without a country has `None`.
    pub country: Option<String>,
}

/// Returns the preferred locales of the user, the most preferred first.
///
/// The list is empty when the platform gives no locales or the query fails.
pub fn preferred_locales() -> Vec<Locale> {
    let mut count: c_int = 0;
    // Safety: SDL_GetPreferredLocales writes the count and returns either null or one
    // allocation that holds a null terminated array of locale pointers.
    let array = unsafe { sdl3_sys::locale::SDL_GetPreferredLocales(&mut count) };
    if array.is_null() {
        return Vec::new();
    }
    let count = usize::try_from(count).unwrap_or(0);
    let mut locales = Vec::with_capacity(count);
    for index in 0..count {
        // Safety: the array holds `count` valid locale pointers before its terminator.
        let locale = unsafe { *array.add(index) };
        if locale.is_null() {
            break;
        }
        // Safety: the locale pointer is valid until the array is freed.
        let locale = unsafe { *locale };
        // Safety: the language is a valid C string or null, and the country likewise.
        let language = unsafe { text(locale.language) };
        let Some(language) = language else {
            continue;
        };
        // Safety: as above.
        let country = unsafe { text(locale.country) };
        locales.push(Locale { language, country });
    }
    // Safety: the array is one allocation of SDL that the caller frees with SDL_free.
    unsafe { sdl3_sys::stdinc::SDL_free(array.cast::<c_void>()) };
    locales
}

/// Returns the text of a C string. A null pointer and an empty string give `None`.
///
/// # Safety
///
/// `pointer` is null or points to a valid null terminated C string.
unsafe fn text(pointer: *const c_char) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    // Safety: the caller guarantees a valid null terminated C string.
    let text = unsafe { CStr::from_ptr(pointer) }.to_string_lossy();
    (!text.is_empty()).then(|| text.into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies that a null pointer and an empty C string give no text and that a C string gives
    /// its text.
    #[test]
    fn a_c_string_gives_its_text() {
        // Safety: a null pointer is allowed.
        assert_eq!(unsafe { text(std::ptr::null()) }, None);
        // Safety: the literal is a valid null terminated C string.
        assert_eq!(unsafe { text(c"".as_ptr()) }, None);
        // Safety: the literal is a valid null terminated C string.
        assert_eq!(unsafe { text(c"ja".as_ptr()) }, Some("ja".to_owned()));
    }

    /// Verifies that the query returns well formed locales without a panic.
    #[test]
    fn preferred_locales_have_a_language() {
        for locale in preferred_locales() {
            assert!(!locale.language.is_empty());
            assert!(
                locale
                    .country
                    .as_ref()
                    .is_none_or(|country| !country.is_empty())
            );
        }
    }
}
