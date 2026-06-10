use core::{
    mem::MaybeUninit,
    ops::{Deref, DerefMut},
};

/// A fixed-capacity vector stored entirely on the stack.
pub struct StackVec<T, const N: usize> {
    data: [MaybeUninit<T>; N],
    len: usize,
}

impl<T, const N: usize> Default for StackVec<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, const N: usize> StackVec<T, N> {
    /// Creates an empty `StackVec`.
    pub fn new() -> Self {
        Self {
            data: [const { MaybeUninit::uninit() }; N],
            len: 0,
        }
    }

    /// Appends an element. Panics if at capacity.
    pub fn push(&mut self, value: T) {
        assert!(self.len < N, "StackVec overflow");
        self.data[self.len] = MaybeUninit::new(value);
        self.len += 1;
    }
}

impl<T, const N: usize> Deref for StackVec<T, N> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        // Safety: elements 0..len are initialized.
        #![allow(unsafe_code)]
        unsafe { core::slice::from_raw_parts(self.data.as_ptr().cast(), self.len) }
    }
}

impl<T, const N: usize> DerefMut for StackVec<T, N> {
    fn deref_mut(&mut self) -> &mut [T] {
        // Safety: elements 0..len are initialized.
        #![allow(unsafe_code)]
        unsafe { core::slice::from_raw_parts_mut(self.data.as_mut_ptr().cast(), self.len) }
    }
}

impl<T, const N: usize> Drop for StackVec<T, N> {
    fn drop(&mut self) {
        // Safety: elements 0..len are initialized and must be dropped.
        #![allow(unsafe_code)]
        for slot in &mut self.data[..self.len] {
            unsafe { slot.assume_init_drop() };
        }
    }
}

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

impl<T, const N: usize> FromIterator<T> for StackVec<T, N> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let mut vec = Self::new();
        for item in iter {
            vec.push(item);
        }
        vec
    }
}
