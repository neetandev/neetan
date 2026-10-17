//! Byte views of the GPU records.
//!
//! This module holds the only unsafe code of the crate.

#![allow(unsafe_code)]

use crate::{Instance, SlugGlyph};

/// A type whose values are plain bytes.
///
/// # Safety
///
/// An implementing type has no padding bytes, no pointers and no interior
/// mutability. Every byte of a value is initialized.
pub(crate) unsafe trait Plain: Copy {}

// SAFETY: `u32` has no padding.
unsafe impl Plain for u32 {}

// SAFETY: An array of `f32` has no padding.
unsafe impl Plain for [f32; 4] {}

// SAFETY: `SlugGlyph` is `#[repr(C)]` with fields of f32 and u32 with an
// alignment of 4. The assertion below proves that the fields fill the record
// without padding.
unsafe impl Plain for SlugGlyph {}

// SAFETY: `Instance` is `#[repr(C)]` with fields of f32 and u32 with an
// alignment of 4. The assertion below proves that the fields fill the record
// without padding.
unsafe impl Plain for Instance {}

// The records fill their size without padding and need no alignment above 4.
const _: () = assert!(size_of::<SlugGlyph>() == 32 && align_of::<SlugGlyph>() == 4);
const _: () = assert!(size_of::<Instance>() == 128 && align_of::<Instance>() == 4);

/// The bytes of `values` in native byte order.
pub(crate) fn as_bytes<T: Plain>(values: &[T]) -> &[u8] {
    // SAFETY: `Plain` guarantees that every byte of `T` is initialized. The
    // pointer and length come from a valid slice, `u8` has an alignment of 1
    // and the returned slice borrows `values`.
    unsafe { std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), size_of_val(values)) }
}
