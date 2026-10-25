//! The writer of the sentences of a note and the cached text of a note.
//!
//! Each sentence of the catalog ends with its own sentence mark. The catalog
//! pattern `note.join` joins the sentences of a note.

use std::fmt::{self, Write as _};

use gui::ui::{TextBuffer, TextData, WrappedText};
use i18n::{Catalog, Key, Message};

use super::notes::NoteLook;
use crate::texts::note;

/// The number of sentence ends that a note keeps on the stack. A note with
/// more sentences keeps their ends on the heap.
const INLINE_SENTENCES: usize = 8;

/// Writes the sentences of a note. Each sentence goes into a scratch text,
/// and [`NoteWriter::finish`] joins them with the catalog pattern
/// `note.join`. An empty sentence leaves the note.
pub(crate) struct NoteWriter<'c, 's> {
    /// The texts of the sentences.
    catalog: &'c Catalog,
    /// The sentences back to back.
    scratch: &'s mut String,
    /// The end of each of the first sentences in the scratch text.
    inline_ends: [usize; INLINE_SENTENCES],
    /// The end of every sentence of a note with more than
    /// [`INLINE_SENTENCES`] sentences. Empty for a shorter note.
    spilled_ends: Vec<usize>,
    /// The number of sentences.
    count: usize,
}

impl<'c, 's> NoteWriter<'c, 's> {
    /// A writer of a note with the texts of `catalog` over the emptied
    /// scratch text `scratch`.
    pub(crate) fn new(catalog: &'c Catalog, scratch: &'s mut String) -> Self {
        scratch.clear();
        Self {
            catalog,
            scratch,
            inline_ends: [0; INLINE_SENTENCES],
            spilled_ends: Vec::new(),
            count: 0,
        }
    }

    /// The catalog of the sentences.
    pub(crate) fn catalog(&self) -> &'c Catalog {
        self.catalog
    }

    /// The number of sentences of the note.
    pub(crate) fn sentence_count(&self) -> usize {
        self.count
    }

    /// Adds `sentence`, which ends with its own sentence mark.
    pub(crate) fn sentence(&mut self, sentence: impl fmt::Display) {
        let start = self.scratch.len();
        let _ = write!(self.scratch, "{sentence}");
        let end = self.scratch.len();
        if end == start {
            return;
        }
        if self.count < INLINE_SENTENCES {
            self.inline_ends[self.count] = end;
        } else {
            if self.spilled_ends.is_empty() {
                self.spilled_ends.extend_from_slice(&self.inline_ends);
            }
            self.spilled_ends.push(end);
        }
        self.count += 1;
    }

    /// Adds the fixed text `key` as a sentence.
    pub(crate) fn text(&mut self, key: Key) {
        self.sentence(self.catalog.text(key));
    }

    /// Adds `message` as a sentence.
    pub(crate) fn message<const N: usize>(&mut self, message: Message<'_, N>) {
        self.sentence(self.catalog.display(message));
    }

    /// The end of each sentence in the scratch text.
    fn ends(&self) -> &[usize] {
        if self.spilled_ends.is_empty() {
            &self.inline_ends[..self.count]
        } else {
            &self.spilled_ends
        }
    }

    /// Writes the sentences joined with the catalog pattern into `out`.
    pub(crate) fn finish(&self, out: &mut String) {
        let _ = write!(
            out,
            "{}",
            Joined {
                catalog: self.catalog,
                text: self.scratch,
                ends: self.ends(),
                start: 0,
            }
        );
    }
}

/// The sentences of a note from `start` that end at `ends` in `text`,
/// joined with the catalog pattern `note.join`.
struct Joined<'a> {
    /// The catalog of the pattern.
    catalog: &'a Catalog,
    /// The sentences back to back.
    text: &'a str,
    /// The end of each sentence.
    ends: &'a [usize],
    /// The start of the first sentence.
    start: usize,
}

impl fmt::Display for Joined<'_> {
    /// Writes the sentences joined with the join pattern of the catalog.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.ends {
            [] => Ok(()),
            [end] => formatter.write_str(&self.text[self.start..*end]),
            [end, rest @ ..] => {
                let first = &self.text[self.start..*end];
                let rest = Joined {
                    catalog: self.catalog,
                    text: self.text,
                    ends: rest,
                    start: *end,
                };
                self.catalog.write(formatter, note::join(&first, &rest))
            }
        }
    }
}

/// A note of a pass: its text, its look and the lines that its text wraps
/// into, kept between passes.
#[derive(Debug)]
pub(crate) struct NoteView<'a> {
    /// The joined sentences.
    pub(crate) text: &'a str,
    /// The look of the note.
    pub(crate) look: NoteLook,
    /// The wrapped lines of the text.
    pub(crate) lines: &'a mut WrappedText,
}

/// The text and the look of a note that refill only when the data of the
/// note or the catalog generation changes, and the lines of its text.
#[derive(Debug, Clone)]
pub(crate) struct NoteText<D> {
    /// The joined sentences.
    text: TextBuffer<D>,
    /// The sentences back to back while the note refills.
    scratch: String,
    /// The look of the last fill.
    look: NoteLook,
    /// The wrapped lines of the text.
    lines: WrappedText,
}

impl<D> Default for NoteText<D> {
    /// An empty note without a fill.
    fn default() -> Self {
        Self {
            text: TextBuffer::new(),
            scratch: String::new(),
            look: NoteLook::default(),
            lines: WrappedText::new(),
        }
    }
}

impl<D> NoteText<D> {
    /// The note for `data` in `catalog` at the catalog generation
    /// `generation` with its look and its lines. `build` writes the
    /// sentences and returns the look when the data or the generation
    /// differs from the last fill.
    pub(crate) fn note<Q: TextData<D>>(
        &mut self,
        catalog: &Catalog,
        generation: u64,
        data: Q,
        build: impl FnOnce(&mut NoteWriter<'_, '_>, Q) -> NoteLook,
    ) -> NoteView<'_> {
        let Self {
            text,
            scratch,
            look,
            lines,
        } = self;
        let text = text.text_in(catalog, generation, data, |catalog, data, out| {
            let mut writer = NoteWriter::new(catalog, scratch);
            *look = build(&mut writer, data);
            writer.finish(out);
        });
        NoteView {
            text,
            look: *look,
            lines,
        }
    }

    /// The text of the last fill. A note without a fill has none.
    #[cfg(test)]
    pub(crate) fn last_text(&self) -> Option<&str> {
        self.text.last_text()
    }

    /// The number of lines of the last wrap of the text.
    #[cfg(test)]
    pub(crate) fn line_count(&self) -> usize {
        self.lines.line_count()
    }
}

#[cfg(test)]
mod tests {
    use allocation_counter::count_allocations;

    use super::*;
    use crate::texts::{english, test_catalog};

    /// The German texts of the note join.
    const GERMAN: &str = "[note]\njoin = {first} | {second}\n";

    /// The sentences of `sentences` joined in `catalog`.
    fn joined(catalog: &Catalog, sentences: &[&str]) -> String {
        let mut scratch = String::new();
        let mut writer = NoteWriter::new(catalog, &mut scratch);
        for sentence in sentences {
            writer.sentence(sentence);
        }
        let mut text = String::new();
        writer.finish(&mut text);
        text
    }

    /// Verifies that the note joins its sentences with the catalog pattern and that empty sentences leave the note.
    #[test]
    fn notes_join_their_sentences_with_the_catalog_pattern() {
        assert_eq!(
            joined(
                english(),
                &[
                    "Built-in default: low.",
                    "",
                    "At low the CPU runs at 4 MHz."
                ]
            ),
            "Built-in default: low. At low the CPU runs at 4 MHz."
        );
        assert_eq!(joined(english(), &[]), "");
        let german = test_catalog(GERMAN);
        assert_eq!(
            joined(&german, &["Eins.", "Zwei.", "Drei."]),
            "Eins. | Zwei. | Drei."
        );
    }

    /// Verifies that a note joins every sentence past the sentences on the stack with the catalog pattern.
    #[test]
    fn a_note_joins_every_sentence_past_the_inline_ends() {
        for count in [INLINE_SENTENCES, INLINE_SENTENCES + 1, INLINE_SENTENCES * 3] {
            let sentences = vec!["A."; count];
            assert_eq!(joined(english(), &sentences), sentences.join(" "));
        }
        let german = test_catalog(GERMAN);
        let sentences = vec!["B."; INLINE_SENTENCES + 2];
        assert_eq!(joined(&german, &sentences), sentences.join(" | "));
    }

    /// Verifies that a note of the sentences on the stack writes without an allocation into buffers with room.
    #[test]
    fn a_note_of_inline_sentences_allocates_nothing() {
        let mut scratch = String::with_capacity(64);
        let mut text = String::with_capacity(64);
        let catalog = english();
        let (allocations, ()) = count_allocations(|| {
            let mut writer = NoteWriter::new(catalog, &mut scratch);
            for _ in 0..INLINE_SENTENCES {
                writer.sentence("A.");
            }
            writer.finish(&mut text);
        });
        assert_eq!(allocations, 0);
        assert_eq!(text, ["A."; INLINE_SENTENCES].join(" "));
    }
}
