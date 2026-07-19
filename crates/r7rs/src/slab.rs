//! Private stable-key slab storage.

use std::num::NonZeroU32;

/// A stable key created only by its owning [`Slab`].
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct Key(NonZeroU32);

// The nonzero representation keeps optional keys at one machine word.
const _: () = assert!(size_of::<Option<Key>>() == size_of::<u32>());

/// Dense stable-key storage with LIFO reuse of vacant entries.
pub(crate) struct Slab<T> {
    entries: Vec<Option<T>>,
    free: Vec<NonZeroU32>,
}

impl<T> Default for Slab<T> {
    fn default() -> Self {
        Self {
            // Index zero stays permanently vacant so every issued key is
            // representable as `NonZeroU32`.
            entries: vec![None],
            free: Vec::new(),
        }
    }
}

impl<T> Slab<T> {
    /// Inserts a value and returns its stable key.
    pub(crate) fn insert(&mut self, value: T) -> Key {
        if let Some(index) = self.free.pop() {
            let Some(entry) = self.entries.get_mut(index.get() as usize) else {
                std::process::abort();
            };
            if entry.is_some() {
                std::process::abort();
            }
            *entry = Some(value);
            return Key(index);
        }

        // Root cloning is infallible, so exhausting the complete key space is
        // treated like allocator exhaustion. Index zero is reserved and the
        // final vector length is kept representable as `u32`.
        if self.entries.len() >= u32::MAX as usize {
            std::process::abort();
        }
        let Some(index) = NonZeroU32::new(self.entries.len() as u32) else {
            std::process::abort();
        };
        self.entries.push(Some(value));
        Key(index)
    }

    /// Removes the value for `key`, returning `None` when already vacant.
    pub(crate) fn remove(&mut self, key: &Key) -> Option<T> {
        let entry = self.entries.get_mut(key.0.get() as usize)?;
        let value = entry.take()?;
        self.free.push(key.0);
        Some(value)
    }

    /// Iterates over every occupied value.
    pub(crate) fn values(&self) -> impl Iterator<Item = &T> {
        self.entries.iter().filter_map(Option::as_ref)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserves_zero_and_reuses_vacancies_in_lifo_order() {
        let mut slab = Slab::default();
        let first = slab.insert(10);
        let second = slab.insert(20);
        let third = slab.insert(30);

        assert_eq!(first.0.get(), 1);
        assert_eq!(second.0.get(), 2);
        assert_eq!(third.0.get(), 3);
        assert_eq!(slab.remove(&first), Some(10));
        assert_eq!(slab.remove(&third), Some(30));

        let reused_third = slab.insert(31);
        let reused_first = slab.insert(11);
        assert_eq!(reused_third, third);
        assert_eq!(reused_first, first);
        assert_eq!(slab.values().copied().collect::<Vec<_>>(), [11, 20, 31]);
    }

    #[test]
    fn removing_a_vacant_key_does_not_enqueue_it_twice() {
        let mut slab = Slab::default();
        let first = slab.insert(10);
        let second = slab.insert(20);

        assert_eq!(slab.remove(&first), Some(10));
        assert_eq!(slab.remove(&first), None);

        let reused = slab.insert(30);
        let appended = slab.insert(40);
        assert_eq!(reused, first);
        assert_eq!(appended.0.get(), 3);
        assert_eq!(slab.values().copied().collect::<Vec<_>>(), [30, 20, 40]);
        assert_eq!(slab.remove(&second), Some(20));
    }
}
