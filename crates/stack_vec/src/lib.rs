//! A fixed-capacity vector stored entirely on the stack, for small bounded lists that must avoid
//! heap allocation.
//!
//! The capacity of a [`StackVec`] is the largest count of its kind. A push past the capacity is a
//! programming error and panics in every build.

#![cfg_attr(not(test), no_std)]
#![warn(missing_docs)]

use core::{
    fmt,
    mem::MaybeUninit,
    ops::{Deref, DerefMut},
};

/// A fixed-capacity vector stored entirely on the stack.
pub struct StackVec<T, const N: usize> {
    /// The storage. Elements `0..length` are initialized.
    data: [MaybeUninit<T>; N],
    /// The number of initialized elements.
    length: usize,
}

impl<T, const N: usize> Default for StackVec<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, const N: usize> StackVec<T, N> {
    /// The number of elements the vector holds at most.
    pub const CAPACITY: usize = N;

    /// Creates an empty `StackVec`.
    pub const fn new() -> Self {
        Self {
            data: [const { MaybeUninit::uninit() }; N],
            length: 0,
        }
    }

    /// Returns whether the vector holds `N` elements.
    pub const fn is_full(&self) -> bool {
        self.length == N
    }

    /// Appends an element.
    ///
    /// Panics when the vector is full. A push past the capacity is a programming error.
    pub fn push(&mut self, value: T) {
        assert!(self.length < N, "StackVec overflow");
        self.data[self.length] = MaybeUninit::new(value);
        self.length += 1;
    }

    /// Removes and drops every element.
    pub fn clear(&mut self) {
        let length = self.length;
        self.length = 0;
        for slot in &mut self.data[..length] {
            // SAFETY: Elements 0..length were initialized, and the length is zero before the drop.
            unsafe {
                slot.assume_init_drop();
            }
        }
    }
}

impl<T, const N: usize> Deref for StackVec<T, N> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        // SAFETY: Elements 0..length are initialized.
        unsafe { core::slice::from_raw_parts(self.data.as_ptr().cast(), self.length) }
    }
}

impl<T, const N: usize> DerefMut for StackVec<T, N> {
    fn deref_mut(&mut self) -> &mut [T] {
        // SAFETY: Elements 0..length are initialized.
        unsafe { core::slice::from_raw_parts_mut(self.data.as_mut_ptr().cast(), self.length) }
    }
}

impl<T, const N: usize> Drop for StackVec<T, N> {
    fn drop(&mut self) {
        self.clear();
    }
}

impl<T: Clone, const N: usize> Clone for StackVec<T, N> {
    fn clone(&self) -> Self {
        self.iter().cloned().collect()
    }
}

impl<T: fmt::Debug, const N: usize> fmt::Debug for StackVec<T, N> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_list().entries(self.iter()).finish()
    }
}

impl<T: PartialEq, const N: usize> PartialEq for StackVec<T, N> {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl<T: Eq, const N: usize> Eq for StackVec<T, N> {}

impl<'a, T, const N: usize> IntoIterator for &'a StackVec<T, N> {
    type Item = &'a T;
    type IntoIter = core::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.deref().iter()
    }
}

impl<'a, T, const N: usize> IntoIterator for &'a mut StackVec<T, N> {
    type Item = &'a mut T;
    type IntoIter = core::slice::IterMut<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.deref_mut().iter_mut()
    }
}

impl<T, const N: usize> Extend<T> for StackVec<T, N> {
    /// Appends every element of `iter` with [`StackVec::push`].
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        for item in iter {
            self.push(item);
        }
    }
}

impl<T, const N: usize, const M: usize> From<[T; M]> for StackVec<T, N> {
    /// Collects the elements of `array` with [`StackVec::push`]. An array longer than the capacity
    /// fails to compile.
    fn from(array: [T; M]) -> Self {
        const {
            assert!(
                M <= N,
                "the array is longer than the capacity of the StackVec"
            )
        };
        array.into_iter().collect()
    }
}

impl<T, const N: usize> FromIterator<T> for StackVec<T, N> {
    /// Collects the elements with [`StackVec::push`].
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let mut vector = Self::new();
        vector.extend(iter);
        vector
    }
}

#[cfg(test)]
mod tests {
    use core::cell::Cell;

    use super::*;

    /// A value that counts its drops in a shared counter.
    struct Counted<'a>(&'a Cell<usize>);

    impl Drop for Counted<'_> {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    /// Verifies that pushed elements read back in order and that the vector reports when it is
    /// full.
    #[test]
    fn push_appends_in_order_up_to_the_capacity() {
        let mut vector = StackVec::<u8, 3>::new();
        assert!(vector.is_empty());
        vector.push(1);
        vector.push(2);
        assert!(!vector.is_full());
        vector.push(3);
        assert!(vector.is_full());
        assert_eq!(&*vector, &[1, 2, 3]);
        assert_eq!(StackVec::<u8, 3>::CAPACITY, 3);
    }

    /// Verifies that a push past the capacity panics.
    #[test]
    #[should_panic(expected = "StackVec overflow")]
    fn a_push_past_the_capacity_panics() {
        let mut vector = StackVec::<u8, 1>::new();
        vector.push(1);
        vector.push(2);
    }

    /// Verifies that clear drops every element and that the vector takes new elements after it.
    #[test]
    fn clear_drops_every_element() {
        let drops = Cell::new(0);
        let mut vector = StackVec::<Counted, 4>::new();
        vector.push(Counted(&drops));
        vector.push(Counted(&drops));
        vector.clear();
        assert_eq!(drops.get(), 2);
        assert!(vector.is_empty());
        vector.push(Counted(&drops));
        assert_eq!(vector.len(), 1);
        drop(vector);
        assert_eq!(drops.get(), 3);
    }

    /// Verifies that extend, collect, the conversion of an array, clone and the comparison work
    /// on the elements.
    #[test]
    fn extend_collect_clone_and_compare() {
        let mut vector: StackVec<u8, 4> = [1, 2].into_iter().collect();
        assert_eq!(StackVec::<u8, 4>::from([1, 2]), vector);
        vector.extend([3, 4]);
        let copy = vector.clone();
        assert_eq!(copy, vector);
        assert_eq!(&*copy, &[1, 2, 3, 4]);
        vector[0] = 9;
        assert_ne!(copy, vector);
    }
}
