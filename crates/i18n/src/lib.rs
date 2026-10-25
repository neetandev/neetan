//! Translated texts of the user interface.
//!
//! A language file holds the texts of one language. The reference language defines the [`Schema`]
//! of every other language: its keys, their placeholders and their plural texts. The `generate`
//! module of the crate feature `generate` turns the reference file into Rust code: the schema as a
//! `static`, a [`Key`] constant for each fixed text and a function for each text with
//! placeholders.
//!
//! A [`Catalog`] holds the texts of one language for a schema. A fixed text is a `&str` that the
//! catalog borrows from its text storage. A [`Message`] holds the key and the arguments of a text
//! with placeholders on the stack. The catalog writes it into a caller buffer or through
//! [`core::fmt::Display`]. A key that a language file lacks takes the text of the fallback
//! catalog.

#[cfg(test)]
extern crate self as i18n;

mod catalog;
#[cfg(test)]
mod fixture;
#[cfg(any(feature = "generate", test))]
pub mod generate;
mod message;
mod parse;
mod plural;
mod schema;

pub use catalog::{COUNT_PLACEHOLDER, Catalog, Loaded};
pub use message::{ListDisplay, ListKind, Message, MessageDisplay};
pub use parse::{Fault, FaultKind, FullKey, Header, read_header};
pub use plural::{PluralCategory, PluralRule};
pub use schema::{
    Key, KeyInfo, LIST_PATTERN_KEYS, LIST_PLACEHOLDERS, LIST_SECTION, MAXIMUM_KEY_COUNT, Schema,
};

/// A global allocator for the test binary that counts the allocations of each thread.
#[cfg(test)]
mod allocation_counter {
    use std::{
        alloc::{GlobalAlloc, Layout, System},
        cell::Cell,
    };

    std::thread_local! {
        /// The number of allocations of the current thread.
        static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
    }

    /// The system allocator with a count of the allocations of each thread.
    struct CountingAllocator;

    // SAFETY: Every call forwards to the system allocator with the same arguments.
    unsafe impl GlobalAlloc for CountingAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            count();
            // SAFETY: The caller upholds the contract of `GlobalAlloc::alloc`.
            unsafe { System.alloc(layout) }
        }

        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            count();
            // SAFETY: The caller upholds the contract of `GlobalAlloc::alloc_zeroed`.
            unsafe { System.alloc_zeroed(layout) }
        }

        unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            count();
            // SAFETY: The caller upholds the contract of `GlobalAlloc::realloc`.
            unsafe { System.realloc(pointer, layout, new_size) }
        }

        unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
            // SAFETY: The caller upholds the contract of `GlobalAlloc::dealloc`.
            unsafe { System.dealloc(pointer, layout) }
        }
    }

    /// The global allocator of the test binary.
    #[global_allocator]
    static ALLOCATOR: CountingAllocator = CountingAllocator;

    /// Adds one allocation to the count of the current thread.
    fn count() {
        let _ = ALLOCATIONS.try_with(|allocations| allocations.set(allocations.get() + 1));
    }

    /// Runs a function and returns the number of allocations and reallocations it made on the
    /// current thread together with its result.
    pub(crate) fn count_allocations<T>(function: impl FnOnce() -> T) -> (usize, T) {
        let before = ALLOCATIONS.with(Cell::get);
        let result = function();
        let after = ALLOCATIONS.with(Cell::get);
        (after - before, result)
    }
}
