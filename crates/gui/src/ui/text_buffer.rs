//! The text buffer: a formatted text of a screen that refills only when its data or the catalog
//! generation changes.

#[cfg(any(test, feature = "fill-counter"))]
use std::cell::Cell;
use std::path::{Path, PathBuf};

use i18n::Catalog;

use super::context::UiContext;

#[cfg(any(test, feature = "fill-counter"))]
std::thread_local! {
    /// The number of fills of every text buffer on the current thread.
    static FILLS: Cell<usize> = const { Cell::new(0) };
}

/// Runs `function` and returns the number of fills of every [`TextBuffer`] on the current thread
/// during the run, together with the result of `function`.
#[cfg(any(test, feature = "fill-counter"))]
pub fn count_fills<T>(function: impl FnOnce() -> T) -> (usize, T) {
    let before = FILLS.with(Cell::get);
    let result = function();
    let after = FILLS.with(Cell::get);
    (after - before, result)
}

/// The data of a text in a [`TextBuffer`] that stores it as `D`.
///
/// The data is a borrowed form of the stored data, such as a `&str` of a stored `String` or a
/// tuple of such forms. The check of the data against the stored data allocates nothing.
pub trait TextData<D> {
    /// Whether the data equals the stored data `stored`.
    fn matches(&self, stored: &D) -> bool;

    /// Writes the data into `stored`. The stored data keeps its capacity.
    fn store_into(&self, stored: &mut D);

    /// Returns the data in its stored form.
    fn to_stored(&self) -> D;
}

/// A `Copy` value that a [`TextBuffer`] stores as itself.
pub trait TextValue: Copy + PartialEq {}

impl<T: TextValue> TextData<T> for T {
    fn matches(&self, stored: &T) -> bool {
        self == stored
    }

    fn store_into(&self, stored: &mut T) {
        *stored = *self;
    }

    fn to_stored(&self) -> T {
        *self
    }
}

/// Marks each listed type as a [`TextValue`].
macro_rules! text_values {
    ($($value:ty),*) => {
        $(impl TextValue for $value {})*
    };
}

text_values!(
    (),
    bool,
    char,
    u8,
    u16,
    u32,
    u64,
    u128,
    usize,
    i8,
    i16,
    i32,
    i64,
    i128,
    isize
);

impl TextData<String> for &str {
    fn matches(&self, stored: &String) -> bool {
        *self == stored
    }

    fn store_into(&self, stored: &mut String) {
        stored.clear();
        stored.push_str(self);
    }

    fn to_stored(&self) -> String {
        (*self).to_owned()
    }
}

impl TextData<PathBuf> for &Path {
    fn matches(&self, stored: &PathBuf) -> bool {
        *self == stored.as_path()
    }

    fn store_into(&self, stored: &mut PathBuf) {
        stored.as_mut_os_string().clear();
        stored.push(self);
    }

    fn to_stored(&self) -> PathBuf {
        self.to_path_buf()
    }
}

/// Implements [`TextData`] for a tuple whose parts are text data.
macro_rules! text_data_tuple {
    ($(($data:ident, $stored:ident, $index:tt)),+) => {
        impl<$($data, $stored),+> TextData<($($stored,)+)> for ($($data,)+)
        where
            $($data: TextData<$stored>),+
        {
            fn matches(&self, stored: &($($stored,)+)) -> bool {
                $(self.$index.matches(&stored.$index))&&+
            }

            fn store_into(&self, stored: &mut ($($stored,)+)) {
                $(self.$index.store_into(&mut stored.$index);)+
            }

            fn to_stored(&self) -> ($($stored,)+) {
                ($(self.$index.to_stored(),)+)
            }
        }
    };
}

text_data_tuple!((A, StoredA, 0), (B, StoredB, 1));
text_data_tuple!((A, StoredA, 0), (B, StoredB, 1), (C, StoredC, 2));

/// A formatted text together with the data and the catalog generation of its last fill.
///
/// A screen keeps the data of a text in its state, such as a game name, and the buffer keeps the
/// resolved text. The buffer fills its text again only when the data or the catalog generation
/// differs from the last fill. The text and the stored data keep their capacity between fills.
#[derive(Clone, Debug)]
pub struct TextBuffer<D> {
    /// The text of the last fill.
    text: String,
    /// The data and the catalog generation of the last fill. A buffer without a fill has none.
    filled: Option<(D, u64)>,
}

impl<D> Default for TextBuffer<D> {
    /// An empty buffer without a fill.
    fn default() -> Self {
        Self::new()
    }
}

impl<D> TextBuffer<D> {
    /// An empty buffer without a fill.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            text: String::new(),
            filled: None,
        }
    }

    /// Returns the text for `data` in the catalog of `context`.
    ///
    /// `fill` writes the text into the empty buffer when the data or the catalog generation of
    /// `context` differs from the last fill.
    pub fn text<Q: TextData<D>>(
        &mut self,
        context: &UiContext<'_>,
        data: Q,
        fill: impl FnOnce(&Catalog, Q, &mut String),
    ) -> &str {
        self.text_in(context.catalog, context.catalog_generation, data, fill)
    }

    /// Returns the text for `data` in `catalog` at the catalog generation `generation`.
    ///
    /// `fill` writes the text into the empty buffer when the data or the generation differs from
    /// the last fill.
    pub fn text_in<Q: TextData<D>>(
        &mut self,
        catalog: &Catalog,
        generation: u64,
        data: Q,
        fill: impl FnOnce(&Catalog, Q, &mut String),
    ) -> &str {
        let fresh = self
            .filled
            .as_ref()
            .is_some_and(|(filled, filled_generation)| {
                *filled_generation == generation && data.matches(filled)
            });
        if !fresh {
            match &mut self.filled {
                Some((filled, filled_generation)) => {
                    data.store_into(filled);
                    *filled_generation = generation;
                }
                None => self.filled = Some((data.to_stored(), generation)),
            }
            self.text.clear();
            fill(catalog, data, &mut self.text);
            #[cfg(any(test, feature = "fill-counter"))]
            FILLS.with(|fills| fills.set(fills.get() + 1));
        }
        &self.text
    }

    /// Forgets the last fill. The next call fills the text again.
    pub fn invalidate(&mut self) {
        self.filled = None;
    }

    /// The text of the last fill. A buffer without a fill has none.
    #[must_use]
    pub fn last_text(&self) -> Option<&str> {
        self.filled.as_ref().map(|_| self.text.as_str())
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use common::allocation_counter::count_allocations;

    use super::*;
    use crate::ui::tests::{Harness, test_catalog};

    /// Verifies that the same data and generation fill the text once.
    #[test]
    fn a_buffer_fills_once_for_the_same_data_and_generation() {
        let catalog = test_catalog();
        let mut buffer = TextBuffer::<String>::new();
        let mut fills = 0;
        let mut fill = |_: &Catalog, name: &str, out: &mut String| {
            fills += 1;
            let _ = write!(out, "Delete {name}?");
        };
        assert_eq!(
            buffer.text_in(catalog, 0, "Edge", &mut fill),
            "Delete Edge?"
        );
        assert_eq!(
            buffer.text_in(catalog, 0, "Edge", &mut fill),
            "Delete Edge?"
        );
        assert_eq!(fills, 1);
    }

    /// Verifies that new data, a new generation and an invalidation fill the text again, and that the last text follows the fills.
    #[test]
    fn new_data_or_a_new_generation_fills_the_buffer_again() {
        let catalog = test_catalog();
        let mut buffer = TextBuffer::<String>::new();
        let fills = Cell::new(0);
        let fill = |_: &Catalog, name: &str, out: &mut String| {
            fills.set(fills.get() + 1);
            out.push_str(name);
        };
        assert_eq!(buffer.text_in(catalog, 0, "Edge", fill), "Edge");
        assert_eq!(buffer.text_in(catalog, 0, "Rusty", fill), "Rusty");
        assert_eq!(buffer.text_in(catalog, 1, "Rusty", fill), "Rusty");
        assert_eq!(fills.get(), 3);
        assert_eq!(buffer.last_text(), Some("Rusty"));
        buffer.invalidate();
        assert_eq!(buffer.last_text(), None);
        assert_eq!(buffer.text_in(catalog, 1, "Rusty", fill), "Rusty");
        assert_eq!(fills.get(), 4);
    }

    /// Verifies that a refill keeps the capacity of the text and of the stored data.
    #[test]
    fn a_refill_keeps_the_capacity_of_the_text_and_the_data() {
        let catalog = test_catalog();
        let mut buffer = TextBuffer::<String>::new();
        let long = "A long name of a game";
        buffer.text_in(catalog, 0, long, |_, name, out| out.push_str(name));
        let text_capacity = buffer.text.capacity();
        let data_capacity = buffer.filled.as_ref().map(|(data, _)| data.capacity());
        buffer.text_in(catalog, 0, "Short", |_, name, out| out.push_str(name));
        assert_eq!(buffer.text, "Short");
        assert_eq!(buffer.text.capacity(), text_capacity);
        assert_eq!(
            buffer.filled.as_ref().map(|(data, _)| data.capacity()),
            data_capacity
        );
    }

    /// Verifies that a tuple of a name, a path and a count hits without an allocation and refills
    /// a shorter text into the kept capacity.
    #[test]
    fn a_tuple_key_hits_without_an_allocation() {
        let catalog = test_catalog();
        let mut buffer = TextBuffer::<(String, PathBuf, u32)>::new();
        let fills = Cell::new(0);
        let fill = |_: &Catalog, (name, folder, count): (&str, &Path, u32), out: &mut String| {
            fills.set(fills.get() + 1);
            let _ = write!(out, "{name} in {} ({count})", folder.display());
        };
        let long = ("Edge", Path::new("/games/edge"), 12);
        assert_eq!(
            buffer.text_in(catalog, 0, long, fill),
            "Edge in /games/edge (12)"
        );
        let (allocations, length) =
            count_allocations(|| buffer.text_in(catalog, 0, long, fill).len());
        assert_eq!((allocations, length, fills.get()), (0, 24, 1));
        let short = ("Ed", Path::new("/games/ed"), 3);
        let (allocations, length) =
            count_allocations(|| buffer.text_in(catalog, 0, short, fill).len());
        assert_eq!((allocations, length, fills.get()), (0, 19, 2));
        let different_path = ("Ed", Path::new("/games/ex"), 3);
        assert_eq!(
            buffer.text_in(catalog, 0, different_path, fill),
            "Ed in /games/ex (3)"
        );
        assert_eq!(fills.get(), 3);
    }

    /// Verifies that the buffer takes the catalog and the generation of the context.
    #[test]
    fn a_buffer_follows_the_catalog_generation_of_the_context() {
        let mut harness = Harness::new();
        let mut buffer = TextBuffer::<u32>::new();
        let mut fills = 0;
        for generation in [0, 0, 1] {
            harness.generation = generation;
            let text = harness.frame(|context| {
                buffer
                    .text(context, 3, |catalog, count, out| {
                        fills += 1;
                        let _ = write!(out, "{count} {}", catalog.name());
                    })
                    .to_owned()
            });
            assert_eq!(text, "3 Test");
        }
        assert_eq!(fills, 2);
    }

    /// Verifies that the fill counter counts the fills of every buffer of the run and no hit.
    #[test]
    fn the_fill_counter_counts_the_fills_of_every_buffer() {
        let catalog = test_catalog();
        let mut first = TextBuffer::<u32>::new();
        let mut second = TextBuffer::<u32>::new();
        let fill = |_: &Catalog, count: u32, out: &mut String| {
            let _ = write!(out, "{count}");
        };
        let (fills, ()) = count_fills(|| {
            first.text_in(catalog, 0, 1, fill);
            second.text_in(catalog, 0, 2, fill);
        });
        assert_eq!(fills, 2);
        let (fills, ()) = count_fills(|| {
            first.text_in(catalog, 0, 1, fill);
            second.text_in(catalog, 0, 2, fill);
        });
        assert_eq!(fills, 0);
        let (fills, text) = count_fills(|| first.text_in(catalog, 1, 1, fill).to_owned());
        assert_eq!((fills, text.as_str()), (1, "1"));
    }
}
