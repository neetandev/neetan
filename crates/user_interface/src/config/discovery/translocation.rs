//! The original location of an application bundle that macOS runs from a
//! translocated copy.
//!
//! macOS runs a quarantined bundle that the user did not move with the Finder
//! from a read-only copy at a random path. Files beside the bundle are not
//! beside the copy.

use std::{
    ffi::{OsStr, c_void},
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    ptr,
};

type CFTypeRef = *const c_void;
type CFUrlRef = *const c_void;
type CFAllocatorRef = *const c_void;
type CFErrorRef = *mut c_void;
type CFIndex = isize;
type Boolean = u8;

/// The size of the buffer that receives a path from Core Foundation.
const PATH_BUFFER_SIZE: usize = 4096;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFURLCreateFromFileSystemRepresentation(
        allocator: CFAllocatorRef,
        buffer: *const u8,
        buffer_length: CFIndex,
        is_directory: Boolean,
    ) -> CFUrlRef;
    fn CFURLGetFileSystemRepresentation(
        url: CFUrlRef,
        resolve_against_base: Boolean,
        buffer: *mut u8,
        maximum_buffer_length: CFIndex,
    ) -> Boolean;
    fn CFRelease(object: CFTypeRef);
}

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn SecTranslocateIsTranslocatedURL(
        path: CFUrlRef,
        is_translocated: *mut bool,
        error: *mut CFErrorRef,
    ) -> Boolean;
    fn SecTranslocateCreateOriginalPathForURL(
        translocated_path: CFUrlRef,
        error: *mut CFErrorRef,
    ) -> CFUrlRef;
}

/// A Core Foundation URL that the value owns.
struct OwnedUrl(CFUrlRef);

impl OwnedUrl {
    /// Returns the URL of the folder `path`.
    fn of_folder(path: &Path) -> Option<Self> {
        let bytes = path.as_os_str().as_bytes();
        // Safety: The buffer is valid for `bytes.len()` bytes and a null
        // allocator selects the default allocator.
        let url = unsafe {
            CFURLCreateFromFileSystemRepresentation(
                ptr::null(),
                bytes.as_ptr(),
                CFIndex::try_from(bytes.len()).ok()?,
                1,
            )
        };
        Self::from_created(url)
    }

    /// Takes ownership of a URL that a Core Foundation "Create" function returned.
    fn from_created(url: CFUrlRef) -> Option<Self> {
        (!url.is_null()).then_some(Self(url))
    }

    /// Returns the file system path of the URL.
    fn path(&self) -> Option<PathBuf> {
        let mut buffer = [0u8; PATH_BUFFER_SIZE];
        // Safety: The URL is valid and the buffer is valid for its length.
        let written = unsafe {
            CFURLGetFileSystemRepresentation(
                self.0,
                1,
                buffer.as_mut_ptr(),
                CFIndex::try_from(buffer.len()).ok()?,
            )
        };
        if written == 0 {
            return None;
        }
        let length = buffer.iter().position(|&byte| byte == 0)?;
        Some(PathBuf::from(OsStr::from_bytes(&buffer[..length])))
    }
}

impl Drop for OwnedUrl {
    fn drop(&mut self) {
        // Safety: The value owns one reference to the URL.
        unsafe { CFRelease(self.0) };
    }
}

/// An error object that a Security function may return. Dropping the value
/// releases the object.
struct ErrorSlot(CFErrorRef);

impl ErrorSlot {
    /// Returns an empty slot.
    fn new() -> Self {
        Self(ptr::null_mut())
    }
}

impl Drop for ErrorSlot {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // Safety: A Security function stored an owned error object.
            unsafe { CFRelease(self.0) };
        }
    }
}

/// Returns the place where the user put `bundle` when macOS runs `bundle` as
/// a translocated copy. Returns `None` for a bundle that runs where it lies
/// and when macOS does not tell the original place.
pub(super) fn original_location(bundle: &Path) -> Option<PathBuf> {
    let url = OwnedUrl::of_folder(bundle)?;
    let mut is_translocated = false;
    let mut error = ErrorSlot::new();
    // Safety: The URL is valid, the flag is a valid `bool` and the slot
    // receives an owned error object on failure.
    let checked = unsafe {
        SecTranslocateIsTranslocatedURL(url.0, &raw mut is_translocated, &raw mut error.0)
    };
    if checked == 0 || !is_translocated {
        return None;
    }
    let mut error = ErrorSlot::new();
    // Safety: The URL is valid and the slot receives an owned error object on
    // failure. The returned URL follows the "Create" rule.
    let original = OwnedUrl::from_created(unsafe {
        SecTranslocateCreateOriginalPathForURL(url.0, &raw mut error.0)
    })?;
    original.path()
}

#[cfg(test)]
mod tests {
    use common::temporary::TemporaryFolder;

    use super::*;

    /// Verifies that a bundle that runs where it lies has no original location.
    #[test]
    fn a_bundle_in_place_has_no_original_location() {
        let temporary = TemporaryFolder::new("translocation_in_place_test");
        let bundle = temporary.path().join("Neetan.app");
        std::fs::create_dir_all(bundle.join("Contents").join("MacOS")).expect("folder is created");
        assert_eq!(original_location(&bundle), None);
        assert_eq!(
            original_location(&temporary.path().join("missing.app")),
            None
        );
    }

    /// Verifies that a folder path with spaces and characters outside of ASCII survives the round trip through a URL.
    #[test]
    fn a_folder_path_survives_the_round_trip_through_a_url() {
        let temporary = TemporaryFolder::new("translocation_round_trip_test");
        let base = std::fs::canonicalize(temporary.path()).expect("folder exists");
        let folder = base.join("My Games").join("ゲーム").join("Neetan.app");
        std::fs::create_dir_all(&folder).expect("folder is created");
        let url = OwnedUrl::of_folder(&folder).expect("the URL is created");
        assert_eq!(url.path(), Some(folder));
    }
}
