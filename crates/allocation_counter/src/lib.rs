//! A global allocator that counts the allocations of each thread.
//!
//! [`count_allocations`] measures the allocations of a function on the current thread while
//! [`CountingAllocator`] is the global allocator.

#![warn(missing_docs)]

use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};

std::thread_local! {
    /// The number of allocations of the current thread.
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

/// The system allocator with a count of the allocations of each thread.
pub struct CountingAllocator;

// SAFETY: Every call forwards to the system allocator with the same arguments.
unsafe impl GlobalAlloc for CountingAllocator {
    /// Counts the allocation and allocates with the system allocator.
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count();
        // SAFETY: The caller upholds the contract of `GlobalAlloc::alloc`.
        unsafe { System.alloc(layout) }
    }

    /// Counts the allocation and allocates zeroed memory with the system allocator.
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count();
        // SAFETY: The caller upholds the contract of `GlobalAlloc::alloc_zeroed`.
        unsafe { System.alloc_zeroed(layout) }
    }

    /// Counts the reallocation and reallocates with the system allocator.
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count();
        // SAFETY: The caller upholds the contract of `GlobalAlloc::realloc`.
        unsafe { System.realloc(pointer, layout, new_size) }
    }

    /// Frees the memory with the system allocator without a count.
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: The caller upholds the contract of `GlobalAlloc::dealloc`.
        unsafe { System.dealloc(pointer, layout) }
    }
}

/// The global allocator of the test binary of the crate.
#[cfg(test)]
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

/// Adds one allocation to the count of the current thread.
fn count() {
    let _ = ALLOCATIONS.try_with(|allocations| allocations.set(allocations.get() + 1));
}

/// Runs a function and returns the number of allocations and reallocations it made on the
/// current thread together with its result.
///
/// Panics when [`CountingAllocator`] is not the global allocator of the binary.
pub fn count_allocations<T>(function: impl FnOnce() -> T) -> (usize, T) {
    assert_counts(|| drop(std::hint::black_box(Box::new(0_u64))));
    let before = ALLOCATIONS.with(Cell::get);
    let result = function();
    let after = ALLOCATIONS.with(Cell::get);
    (after - before, result)
}

/// Runs a probe that makes exactly one allocation and panics when the counter does not count it.
fn assert_counts(probe: impl FnOnce()) {
    let before = ALLOCATIONS.with(Cell::get);
    probe();
    let after = ALLOCATIONS.with(Cell::get);
    assert!(
        after - before == 1,
        "the allocation counter counts no allocations: CountingAllocator is not the global allocator"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies that the count of a function with one box is one and that the result returns.
    #[test]
    fn count_allocations_counts_a_box() {
        let (allocations, value) = count_allocations(|| *std::hint::black_box(Box::new(7_u32)));
        assert_eq!(allocations, 1);
        assert_eq!(value, 7);
    }

    /// Verifies that the count of a function without allocations is zero.
    #[test]
    fn count_allocations_counts_nothing_for_a_function_without_allocations() {
        let (allocations, value) = count_allocations(|| std::hint::black_box(3_u32) + 4);
        assert_eq!(allocations, 0);
        assert_eq!(value, 7);
    }

    /// Verifies that a probe without a counted allocation panics.
    #[test]
    #[should_panic(expected = "the allocation counter counts no allocations")]
    fn a_probe_that_is_not_counted_panics() {
        assert_counts(|| {
            std::hint::black_box(0_u64);
        });
    }
}
