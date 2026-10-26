//! The edit state of a line of text and the edits over it.
//!
//! [`TextEdit`] holds the cursor and the anchor of the selection as byte
//! offsets of the text. The text itself belongs to the caller. The model
//! holds two offsets and allocates nothing.
//!
//! A position is a byte offset at a character boundary. A position never
//! sits before a combining mark, a variation selector or a zero width joiner
//! (see [`is_mark`]), and never after a zero width joiner. The start and the
//! end of the text are always positions.
//!
//! A word is a run of letters and digits of one script, or a run of Han
//! characters, of hiragana, of katakana or of Hangul. Spaces and punctuation
//! separate words, also `/`, `\`, `.`, `-`, `_` and `:`. A digit joins the
//! letters of any script beside it. A digit beside a Han, kana or Hangul
//! character starts a new word. Two adjacent letters of different scripts
//! start a new word. A word step to the left goes to the start of
//! the word before the cursor. A word step to the right goes to the end of
//! the word after the cursor.
//!
//! Every edit reports in an [`EditOutcome`] whether the text and the state
//! changed. Every operation first clamps the state to the text, so the text
//! may change between operations.

use std::ops::Range;

use crate::fonts::wrap::is_mark;

/// The cursor and the anchor of the selection in a text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct TextEdit {
    /// The byte offset of the cursor.
    cursor: usize,
    /// The byte offset of the end of the selection that stays while the
    /// selection extends. It equals the cursor without a selection.
    anchor: usize,
}

/// The result of an operation of a [`TextEdit`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct EditOutcome {
    /// Whether the text changed.
    pub(super) text_changed: bool,
    /// Whether the cursor or the anchor changed.
    pub(super) state_changed: bool,
    /// Whether the acceptance rule rejected the candidate text. The text and
    /// the state then keep their values.
    pub(super) rejected: bool,
}

/// A target of the cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Motion {
    /// One position to the left.
    Left,
    /// One position to the right.
    Right,
    /// The start of the word before the cursor.
    WordLeft,
    /// The end of the word after the cursor.
    WordRight,
    /// The start of the text.
    Start,
    /// The end of the text.
    End,
}

/// The script of a letter, as far as the word rule tells scripts apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Script {
    /// Latin letters, also the full width forms.
    Latin,
    /// Greek letters.
    Greek,
    /// Cyrillic letters.
    Cyrillic,
    /// The letters of every other script.
    Other,
}

/// The part that a character takes in a word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WordClass {
    /// A space or punctuation between words.
    Separator,
    /// A letter of an alphabetic script outside Han, kana and Hangul.
    Letter(Script),
    /// A digit.
    Digit,
    /// A Han character.
    Han,
    /// A hiragana character.
    Hiragana,
    /// A katakana character.
    Katakana,
    /// A Hangul character.
    Hangul,
}

impl TextEdit {
    /// The byte offset of the cursor.
    pub(super) fn cursor(self) -> usize {
        self.cursor
    }

    /// The byte offset of the anchor of the selection.
    #[cfg(test)]
    pub(super) fn anchor(self) -> usize {
        self.anchor
    }

    /// Whether the state holds a selection of at least one position.
    pub(super) fn has_selection(self) -> bool {
        self.cursor != self.anchor
    }

    /// The byte range of the selection, from its smaller end to its larger
    /// end. It is empty without a selection.
    pub(super) fn selection(self) -> Range<usize> {
        self.cursor.min(self.anchor)..self.cursor.max(self.anchor)
    }

    /// The selected part of `text`.
    pub(super) fn selected_text(self, text: &str) -> &str {
        let mut edit = self;
        edit.clamp(text);
        &text[edit.selection()]
    }

    /// Clamps the cursor and the anchor to positions of `text`. An offset
    /// past the end goes to the end. An offset inside a character, before a
    /// mark or after a zero width joiner goes to the position before it.
    pub(super) fn clamp(&mut self, text: &str) -> EditOutcome {
        let previous = *self;
        self.cursor = floor_position(text, self.cursor);
        self.anchor = floor_position(text, self.anchor);
        self.outcome(previous, false)
    }

    /// Places the cursor at the position at or before the byte offset
    /// `offset`. With `extend` the anchor keeps its value, else it follows
    /// the cursor.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "only the tests place the cursor at an offset")
    )]
    pub(super) fn place(&mut self, text: &str, offset: usize, extend: bool) -> EditOutcome {
        let previous = *self;
        self.clamp(text);
        self.set_cursor(floor_position(text, offset), extend);
        self.outcome(previous, false)
    }

    /// Selects the whole `text` with the cursor at its end.
    pub(super) fn select_all(&mut self, text: &str) -> EditOutcome {
        let previous = *self;
        self.anchor = 0;
        self.cursor = text.len();
        self.outcome(previous, false)
    }

    /// Places the cursor at the target of `motion` in `text`. With `extend`
    /// the anchor keeps its value and the selection grows or shrinks.
    /// Without `extend` the selection collapses. [`Motion::Left`] and
    /// [`Motion::Right`] with a selection go to its start and its end. Every
    /// other motion goes from the cursor.
    pub(super) fn go_to(&mut self, text: &str, motion: Motion, extend: bool) -> EditOutcome {
        let previous = *self;
        self.clamp(text);
        let selection = self.selection();
        let target = match motion {
            Motion::Left if !extend && self.has_selection() => selection.start,
            Motion::Right if !extend && self.has_selection() => selection.end,
            Motion::Left => previous_position(text, self.cursor).unwrap_or(0),
            Motion::Right => next_position(text, self.cursor).unwrap_or(text.len()),
            Motion::WordLeft => word_start_before(text, self.cursor),
            Motion::WordRight => word_end_after(text, self.cursor),
            Motion::Start => 0,
            Motion::End => text.len(),
        };
        self.set_cursor(target, extend);
        self.outcome(previous, false)
    }

    /// Inserts `inserted` at the cursor of `text` in place of the selection
    /// and places the cursor after it. The text keeps at most `limit`
    /// characters. A longer insert is cut at the last position that fits.
    /// With no room left for the first position of `inserted` the insert
    /// does nothing, also for a text at or past the limit with a selection
    /// too small to make room. The edit happens only when `accept` takes
    /// the whole candidate text.
    pub(super) fn insert(
        &mut self,
        text: &mut String,
        inserted: &str,
        limit: usize,
        accept: impl Fn(&str) -> bool,
    ) -> EditOutcome {
        let previous = *self;
        self.clamp(text);
        let selection = self.selection();
        let kept = text.chars().count() - text[selection.clone()].chars().count();
        let fitting = cut_to_positions(inserted, limit.saturating_sub(kept));
        if fitting.is_empty() && !inserted.is_empty() {
            return self.outcome(previous, false);
        }
        self.replace(text, selection, fitting, previous, accept)
    }

    /// Removes the selection of `text`, or else the position before the
    /// cursor.
    pub(super) fn remove_backward(&mut self, text: &mut String) -> EditOutcome {
        let previous = *self;
        self.clamp(text);
        let range = if self.has_selection() {
            self.selection()
        } else {
            previous_position(text, self.cursor).unwrap_or(self.cursor)..self.cursor
        };
        self.replace(text, range, "", previous, |_| true)
    }

    /// Removes the selection of `text`, or else the position after the
    /// cursor.
    pub(super) fn remove_forward(&mut self, text: &mut String) -> EditOutcome {
        let previous = *self;
        self.clamp(text);
        let range = if self.has_selection() {
            self.selection()
        } else {
            self.cursor..next_position(text, self.cursor).unwrap_or(self.cursor)
        };
        self.replace(text, range, "", previous, |_| true)
    }

    /// Replaces `range` of `text` with `inserted` when `accept` takes the
    /// candidate text, and places the cursor at the first position at or
    /// after the end of `inserted`. `previous` is the state before the
    /// operation. The candidate forms inside `text`. A rejected candidate
    /// leaves `text` as it was.
    fn replace(
        &mut self,
        text: &mut String,
        range: Range<usize>,
        inserted: &str,
        previous: TextEdit,
        accept: impl Fn(&str) -> bool,
    ) -> EditOutcome {
        if text[range.clone()] == *inserted {
            if !range.is_empty() || !inserted.is_empty() {
                self.set_cursor(range.start + inserted.len(), false);
            }
            return self.outcome(previous, false);
        }
        let candidate_length = text.len() - range.len() + inserted.len();
        let inserted_end = range.start + inserted.len();
        text.extend_from_within(range.clone());
        text.replace_range(range.clone(), inserted);
        if !accept(&text[..candidate_length]) {
            text.extend_from_within(inserted_end..candidate_length);
            text.drain(range.start..candidate_length);
            *self = previous;
            return EditOutcome {
                rejected: true,
                ..EditOutcome::default()
            };
        }
        text.truncate(candidate_length);
        self.set_cursor(ceil_position(text, inserted_end), false);
        self.outcome(previous, true)
    }

    /// Sets the cursor to `position`. Without `extend` the anchor follows.
    fn set_cursor(&mut self, position: usize, extend: bool) {
        self.cursor = position;
        if !extend {
            self.anchor = position;
        }
    }

    /// The outcome of an operation that started in the state `previous` and
    /// changed the text when `text_changed` holds.
    fn outcome(self, previous: TextEdit, text_changed: bool) -> EditOutcome {
        EditOutcome {
            text_changed,
            state_changed: self != previous,
            rejected: false,
        }
    }
}

/// The zero width joiner. It joins the characters on both of its sides.
const ZERO_WIDTH_JOINER: char = '\u{200D}';

/// Whether the byte offset `offset` is a position of `text`.
fn is_position(text: &str, offset: usize) -> bool {
    offset == 0
        || offset == text.len()
        || (text.is_char_boundary(offset)
            && !text[offset..].starts_with(is_mark)
            && !text[..offset].ends_with(ZERO_WIDTH_JOINER))
}

/// The position of `text` at or before the byte offset `offset`.
fn floor_position(text: &str, offset: usize) -> usize {
    let mut position = text.floor_char_boundary(offset);
    while !is_position(text, position) {
        position = text.floor_char_boundary(position - 1);
    }
    position
}

/// The position of `text` at or after the byte offset `offset`.
fn ceil_position(text: &str, offset: usize) -> usize {
    let mut position = text.ceil_char_boundary(offset);
    while !is_position(text, position) {
        position = text.ceil_char_boundary(position + 1);
    }
    position
}

/// The position of `text` before `position`, or `None` at the start.
fn previous_position(text: &str, position: usize) -> Option<usize> {
    (position > 0).then(|| floor_position(text, position - 1))
}

/// The position of `text` after `position`, or `None` at the end.
fn next_position(text: &str, position: usize) -> Option<usize> {
    let character = text[position..].chars().next()?;
    Some(ceil_position(text, position + character.len_utf8()))
}

/// The longest start of `text` that ends at a position and holds at most
/// `room` characters.
fn cut_to_positions(text: &str, room: usize) -> &str {
    let end = text
        .char_indices()
        .nth(room)
        .map_or(text.len(), |(offset, _)| offset);
    &text[..floor_position(text, end)]
}

/// The word class of the position of `text` that starts at `position`.
fn class_at(text: &str, position: usize) -> WordClass {
    text[position..]
        .chars()
        .next()
        .map_or(WordClass::Separator, word_class)
}

/// The start of the word before `position` of `text`, or the start of the
/// text without one.
fn word_start_before(text: &str, mut position: usize) -> usize {
    let mut right = WordClass::Separator;
    while let Some(start) = previous_position(text, position) {
        let class = class_at(text, start);
        if right != WordClass::Separator && !joins(class, right) {
            break;
        }
        right = class;
        position = start;
    }
    position
}

/// The end of the word after `position` of `text`, or the end of the text
/// without one.
fn word_end_after(text: &str, mut position: usize) -> usize {
    let mut left = WordClass::Separator;
    while let Some(end) = next_position(text, position) {
        let class = class_at(text, position);
        if left != WordClass::Separator && !joins(left, class) {
            break;
        }
        left = class;
        position = end;
    }
    position
}

/// Whether the word class `left` and the word class `right` after it belong
/// to one word.
fn joins(left: WordClass, right: WordClass) -> bool {
    match (left, right) {
        (WordClass::Separator, _) | (_, WordClass::Separator) => false,
        (WordClass::Letter(left), WordClass::Letter(right)) => left == right,
        (WordClass::Letter(_) | WordClass::Digit, WordClass::Letter(_) | WordClass::Digit) => true,
        (left, right) => left == right,
    }
}

/// The word class of `character`.
fn word_class(character: char) -> WordClass {
    match character {
        '\u{2E80}'..='\u{2FDF}'
        | '\u{3005}'..='\u{3007}'
        | '\u{3021}'..='\u{3029}'
        | '\u{3038}'..='\u{303B}'
        | '\u{3400}'..='\u{4DBF}'
        | '\u{4E00}'..='\u{9FFF}'
        | '\u{F900}'..='\u{FAFF}'
        | '\u{20000}'..='\u{3FFFF}' => WordClass::Han,
        '\u{3041}'..='\u{309F}' => WordClass::Hiragana,
        '\u{30A1}'..='\u{30FA}'
        | '\u{30FC}'..='\u{30FF}'
        | '\u{31F0}'..='\u{31FF}'
        | '\u{FF66}'..='\u{FF9F}' => WordClass::Katakana,
        '\u{1100}'..='\u{11FF}'
        | '\u{3131}'..='\u{318E}'
        | '\u{A960}'..='\u{A97F}'
        | '\u{AC00}'..='\u{D7A3}'
        | '\u{D7B0}'..='\u{D7FF}'
        | '\u{FFA0}'..='\u{FFDC}' => WordClass::Hangul,
        _ if character.is_alphabetic() => WordClass::Letter(script(character)),
        _ if character.is_numeric() => WordClass::Digit,
        _ => WordClass::Separator,
    }
}

/// The script of the letter `character`.
fn script(character: char) -> Script {
    match character {
        '\u{0000}'..='\u{02AF}'
        | '\u{1D00}'..='\u{1DBF}'
        | '\u{1E00}'..='\u{1EFF}'
        | '\u{2C60}'..='\u{2C7F}'
        | '\u{A720}'..='\u{A7FF}'
        | '\u{AB30}'..='\u{AB6F}'
        | '\u{FF21}'..='\u{FF3A}'
        | '\u{FF41}'..='\u{FF5A}' => Script::Latin,
        '\u{0370}'..='\u{03FF}' | '\u{1F00}'..='\u{1FFF}' => Script::Greek,
        '\u{0400}'..='\u{052F}'
        | '\u{1C80}'..='\u{1C8F}'
        | '\u{2DE0}'..='\u{2DFF}'
        | '\u{A640}'..='\u{A69F}' => Script::Cyrillic,
        _ => Script::Other,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use allocation_counter::count_allocations;

    use super::*;

    /// A limit that no text of the tests reaches.
    const NO_LIMIT: usize = 1000;

    /// An acceptance rule that takes every candidate.
    fn any(_: &str) -> bool {
        true
    }

    /// A state with the anchor at `anchor` and the cursor at `cursor` of
    /// `text`.
    fn state(text: &str, anchor: usize, cursor: usize) -> TextEdit {
        let mut edit = TextEdit::default();
        edit.place(text, anchor, false);
        edit.place(text, cursor, true);
        edit
    }

    /// The positions that `motion` visits from `start` of `text` until the
    /// cursor keeps its value.
    fn walk(text: &str, start: usize, motion: Motion) -> Vec<usize> {
        let mut edit = state(text, start, start);
        let mut positions = Vec::new();
        while edit.go_to(text, motion, false).state_changed {
            positions.push(edit.cursor());
        }
        positions
    }

    /// The outcome of an operation that changed only the state.
    const STATE: EditOutcome = EditOutcome {
        text_changed: false,
        state_changed: true,
        rejected: false,
    };

    /// The outcome of an operation that changed the text and the state.
    const TEXT_AND_STATE: EditOutcome = EditOutcome {
        text_changed: true,
        state_changed: true,
        rejected: false,
    };

    /// The outcome of an operation that the acceptance rule rejected.
    const REJECTED: EditOutcome = EditOutcome {
        text_changed: false,
        state_changed: false,
        rejected: true,
    };

    #[test]
    fn every_operation_on_an_empty_text_changes_nothing() {
        let mut text = String::new();
        let mut edit = TextEdit::default();
        for motion in [
            Motion::Left,
            Motion::Right,
            Motion::WordLeft,
            Motion::WordRight,
            Motion::Start,
            Motion::End,
        ] {
            for extend in [false, true] {
                assert_eq!(edit.go_to(&text, motion, extend), EditOutcome::default());
            }
        }
        assert_eq!(edit.select_all(&text), EditOutcome::default());
        assert_eq!(edit.remove_backward(&mut text), EditOutcome::default());
        assert_eq!(edit.remove_forward(&mut text), EditOutcome::default());
        assert_eq!(
            edit.insert(&mut text, "", NO_LIMIT, any),
            EditOutcome::default()
        );
        assert_eq!(edit.clamp(&text), EditOutcome::default());
        assert_eq!(edit.selected_text(&text), "");
        assert!(!edit.has_selection());
        assert_eq!(edit.selection(), 0..0);
        assert_eq!(edit.insert(&mut text, "ab", NO_LIMIT, any), TEXT_AND_STATE);
        assert_eq!(text, "ab");
        assert_eq!(edit.cursor(), 2);
    }

    #[test]
    fn an_insert_goes_to_the_cursor_and_replaces_the_selection() {
        let mut text = String::from("held");
        let mut edit = state(&text, 2, 2);
        assert_eq!(edit.insert(&mut text, "xy", NO_LIMIT, any), TEXT_AND_STATE);
        assert_eq!(text, "hexyld");
        assert_eq!((edit.anchor(), edit.cursor()), (4, 4));

        let mut edit = state(&text, 5, 1);
        assert_eq!(edit.insert(&mut text, "Q", NO_LIMIT, any), TEXT_AND_STATE);
        assert_eq!(text, "hQd");
        assert_eq!((edit.anchor(), edit.cursor()), (2, 2));
    }

    #[test]
    fn an_insert_equal_to_the_selection_changes_only_the_state() {
        let mut text = String::from("abc");
        let mut edit = state(&text, 0, 2);
        assert_eq!(edit.insert(&mut text, "ab", NO_LIMIT, |_| false), STATE);
        assert_eq!(text, "abc");
        assert_eq!((edit.anchor(), edit.cursor()), (2, 2));
    }

    #[test]
    fn an_insert_keeps_the_limit_and_cuts_at_a_position() {
        let mut text = String::from("abc");
        let mut edit = state(&text, 3, 3);
        assert_eq!(edit.insert(&mut text, "d", 3, any), EditOutcome::default());
        assert_eq!(text, "abc");

        assert_eq!(edit.insert(&mut text, "defgh", 5, any), TEXT_AND_STATE);
        assert_eq!(text, "abcde");
        assert_eq!(edit.cursor(), 5);

        let mut edit = state(&text, 1, 4);
        assert_eq!(edit.insert(&mut text, "WXYZ", 5, any), TEXT_AND_STATE);
        assert_eq!(text, "aWXYe");
        assert_eq!(edit.cursor(), 4);

        let mut text = String::from("ab");
        let mut edit = state(&text, 2, 2);
        assert_eq!(edit.insert(&mut text, "ce\u{301}", 4, any), TEXT_AND_STATE);
        assert_eq!(text, "abc");
        assert_eq!(edit.cursor(), 3);

        let mut text = String::from("日本");
        let mut edit = state(&text, 6, 6);
        assert_eq!(edit.insert(&mut text, "語の文字", 4, any), TEXT_AND_STATE);
        assert_eq!(text, "日本語の");
        assert_eq!(edit.cursor(), text.len());
    }

    #[test]
    fn a_text_past_the_limit_takes_no_insert() {
        let mut text = String::from("abcdef");
        let mut edit = state(&text, 1, 3);
        assert_eq!(edit.insert(&mut text, "x", 3, any), EditOutcome::default());
        assert_eq!(text, "abcdef");
        assert_eq!((edit.anchor(), edit.cursor()), (1, 3));
    }

    #[test]
    fn a_rejected_candidate_leaves_the_text_and_the_state() {
        let digits = |candidate: &str| candidate.bytes().all(|byte| byte.is_ascii_digit());
        let mut text = String::from("1234");
        let mut edit = state(&text, 1, 3);
        assert_eq!(edit.insert(&mut text, "x", NO_LIMIT, digits), REJECTED);
        assert_eq!(text, "1234");
        assert_eq!((edit.anchor(), edit.cursor()), (1, 3));
        assert_eq!(
            edit.insert(&mut text, "9", NO_LIMIT, digits),
            TEXT_AND_STATE
        );
        assert_eq!(text, "194");

        let mut text = String::from("ab日本cd");
        let mut edit = state(&text, 2, 8);
        assert_eq!(edit.insert(&mut text, "語", NO_LIMIT, |_| false), REJECTED);
        assert_eq!(text, "ab日本cd");
        assert_eq!((edit.anchor(), edit.cursor()), (2, 8));
    }

    #[test]
    fn the_rule_judges_the_whole_candidate() {
        let mut text = String::from("ab|cd");
        let mut edit = state(&text, 2, 3);
        let seen = RefCell::new(String::new());
        let outcome = edit.insert(&mut text, "XY", NO_LIMIT, |candidate| {
            seen.borrow_mut().push_str(candidate);
            false
        });
        assert_eq!(outcome, REJECTED);
        assert_eq!(*seen.borrow(), "abXYcd");
        assert_eq!(text, "ab|cd");
    }

    #[test]
    fn a_selection_extends_in_both_directions() {
        let text = "one two";
        let mut edit = state(text, 3, 3);
        assert_eq!(edit.go_to(text, Motion::Right, true), STATE);
        assert_eq!(edit.go_to(text, Motion::Right, true), STATE);
        assert_eq!(edit.selection(), 3..5);
        assert_eq!(edit.selected_text(text), " t");
        edit.go_to(text, Motion::Left, true);
        edit.go_to(text, Motion::Left, true);
        assert!(!edit.has_selection());
        edit.go_to(text, Motion::Left, true);
        assert_eq!((edit.anchor(), edit.cursor()), (3, 2));
        assert_eq!(edit.selection(), 2..3);
        assert_eq!(edit.selected_text(text), "e");
        edit.go_to(text, Motion::Start, true);
        assert_eq!(edit.selected_text(text), "one");
        edit.go_to(text, Motion::End, true);
        assert_eq!(edit.selected_text(text), " two");
        edit.go_to(text, Motion::WordLeft, true);
        assert_eq!(edit.selected_text(text), " ");
        edit.go_to(text, Motion::WordLeft, true);
        assert_eq!(edit.selected_text(text), "one");
        edit.go_to(text, Motion::WordRight, true);
        edit.go_to(text, Motion::WordRight, true);
        assert_eq!(edit.selected_text(text), " two");
    }

    #[test]
    fn a_step_without_extending_collapses_the_selection() {
        let text = "alpha beta gamma";
        let mut forward = state(text, 2, 8);
        let mut backward = state(text, 8, 2);
        assert_eq!(forward.go_to(text, Motion::Left, false), STATE);
        assert_eq!(backward.go_to(text, Motion::Left, false), STATE);
        assert_eq!((forward.anchor(), forward.cursor()), (2, 2));
        assert_eq!((backward.anchor(), backward.cursor()), (2, 2));

        let mut forward = state(text, 2, 8);
        let mut backward = state(text, 8, 2);
        forward.go_to(text, Motion::Right, false);
        backward.go_to(text, Motion::Right, false);
        assert_eq!((forward.anchor(), forward.cursor()), (8, 8));
        assert_eq!((backward.anchor(), backward.cursor()), (8, 8));

        let mut forward = state(text, 2, 8);
        forward.go_to(text, Motion::WordRight, false);
        assert_eq!((forward.anchor(), forward.cursor()), (10, 10));
        let mut backward = state(text, 8, 2);
        backward.go_to(text, Motion::WordRight, false);
        assert_eq!((backward.anchor(), backward.cursor()), (5, 5));
    }

    #[test]
    fn a_removal_takes_the_selection_in_both_directions() {
        for (anchor, cursor) in [(1, 4), (4, 1)] {
            let mut text = String::from("abcdef");
            let mut edit = state(&text, anchor, cursor);
            assert_eq!(edit.remove_backward(&mut text), TEXT_AND_STATE);
            assert_eq!(text, "aef");
            assert_eq!((edit.anchor(), edit.cursor()), (1, 1));

            let mut text = String::from("abcdef");
            let mut edit = state(&text, anchor, cursor);
            assert_eq!(edit.remove_forward(&mut text), TEXT_AND_STATE);
            assert_eq!(text, "aef");
            assert_eq!((edit.anchor(), edit.cursor()), (1, 1));
        }
    }

    #[test]
    fn a_removal_without_a_selection_takes_one_position() {
        let mut text = String::from("abc");
        let mut edit = state(&text, 1, 1);
        assert_eq!(edit.remove_backward(&mut text), TEXT_AND_STATE);
        assert_eq!(text, "bc");
        assert_eq!(edit.cursor(), 0);
        assert_eq!(edit.remove_backward(&mut text), EditOutcome::default());
        let outcome = edit.remove_forward(&mut text);
        assert_eq!((outcome.text_changed, outcome.state_changed), (true, false));
        assert_eq!(text, "c");
        edit.go_to(&text, Motion::End, false);
        assert_eq!(edit.remove_forward(&mut text), EditOutcome::default());
        assert_eq!(text, "c");
    }

    #[test]
    fn select_all_covers_the_text_with_the_cursor_at_the_end() {
        let text = "日本語";
        let mut edit = state(text, 3, 3);
        assert_eq!(edit.select_all(text), STATE);
        assert_eq!((edit.anchor(), edit.cursor()), (0, 9));
        assert_eq!(edit.selected_text(text), "日本語");
        assert_eq!(edit.select_all(text), EditOutcome::default());
    }

    #[test]
    fn japanese_text_steps_by_character_and_by_script_run() {
        let text = "日本語のテキストを入力、ＯＫ２つ";
        assert_eq!(walk(text, 0, Motion::Right).len(), text.chars().count());
        assert_eq!(walk(text, 0, Motion::Right)[..3], [3, 6, 9]);
        assert_eq!(walk(text, text.len(), Motion::Left).last(), Some(&0));

        let words: Vec<&str> = walk(text, 0, Motion::WordRight)
            .iter()
            .scan(0, |start, &end| {
                let word = &text[*start..end];
                *start = end;
                Some(word.trim_start_matches('、'))
            })
            .collect();
        assert_eq!(
            words,
            ["日本語", "の", "テキスト", "を", "入力", "ＯＫ２", "つ"]
        );
        let starts: Vec<&str> = walk(text, text.len(), Motion::WordLeft)
            .iter()
            .map(|&start| &text[start..])
            .collect();
        assert_eq!(starts[0], "つ");
        assert_eq!(starts[1], "ＯＫ２つ");
        assert_eq!(starts[2], "入力、ＯＫ２つ");
        assert_eq!(starts.last(), Some(&text));

        let mut text = String::from(text);
        let mut edit = state(&text, 9, 9);
        edit.remove_backward(&mut text);
        assert!(text.starts_with("日本の"));
        assert_eq!(edit.cursor(), 6);
    }

    #[test]
    fn french_text_with_no_break_spaces_steps_by_word() {
        let text = "Prix\u{202F}: 12\u{A0}€ « très » l’été";
        let ends: Vec<&str> = walk(text, 0, Motion::WordRight)
            .iter()
            .map(|&end| &text[..end])
            .collect();
        assert_eq!(
            ends,
            [
                "Prix",
                "Prix\u{202F}: 12",
                "Prix\u{202F}: 12\u{A0}€ « très",
                "Prix\u{202F}: 12\u{A0}€ « très » l",
                text,
            ]
        );
        let starts: Vec<&str> = walk(text, text.len(), Motion::WordLeft)
            .iter()
            .map(|&start| &text[start..])
            .collect();
        assert_eq!(
            starts,
            [
                "été",
                "l’été",
                "très » l’été",
                "12\u{A0}€ « très » l’été",
                text,
            ]
        );
        let after_twelve = text
            .find('\u{A0}')
            .expect("the text holds a no-break space");
        let mut edit = state(text, after_twelve, after_twelve);
        edit.go_to(text, Motion::Right, false);
        assert_eq!(&text[edit.cursor()..], "€ « très » l’été");
    }

    #[test]
    fn a_combining_mark_stays_with_its_base() {
        let mut text = String::from("e\u{301}a\u{308}\u{304}b");
        assert_eq!(walk(&text, 0, Motion::Right), [3, 8, 9]);
        assert_eq!(walk(&text, 9, Motion::Left), [8, 3, 0]);
        assert_eq!(walk(&text, 0, Motion::WordRight), [9]);

        let mut edit = TextEdit::default();
        edit.place(&text, 2, false);
        assert_eq!(edit.cursor(), 0);
        edit.place(&text, 5, false);
        assert_eq!(edit.cursor(), 3);

        let mut edit = state(&text, 8, 8);
        assert_eq!(edit.remove_backward(&mut text), TEXT_AND_STATE);
        assert_eq!(text, "e\u{301}b");
        assert_eq!(edit.cursor(), 3);
        let mut edit = state(&text, 0, 0);
        edit.remove_forward(&mut text);
        assert_eq!(text, "b");

        let text = "\u{2764}\u{FE0F}\u{1F468}\u{200D}\u{1F469}";
        assert_eq!(walk(text, 0, Motion::Right), [6, text.len()]);
        assert_eq!(walk(text, text.len(), Motion::Left), [6, 0]);
        let mut edit = TextEdit::default();
        edit.place(text, 13, false);
        assert_eq!(edit.cursor(), 6);
    }

    #[test]
    fn backspace_removes_a_joined_emoji_sequence_whole() {
        let mut text = String::from("a\u{1F468}\u{200D}\u{1F469}");
        let mut edit = state(&text, text.len(), text.len());
        assert_eq!(edit.cursor(), text.len());
        assert_eq!(edit.remove_backward(&mut text), TEXT_AND_STATE);
        assert_eq!(text, "a");
        assert_eq!(edit.cursor(), 1);

        let mut text = String::from("\u{1F468}\u{200D}\u{1F469}b");
        let mut edit = state(&text, 0, 0);
        assert!(edit.remove_forward(&mut text).text_changed);
        assert_eq!(text, "b");
    }

    #[test]
    fn typing_before_a_leading_mark_keeps_the_order() {
        let mut text = String::from("\u{301}abc");
        let mut edit = TextEdit::default();
        assert_eq!(edit.insert(&mut text, "e", NO_LIMIT, any), TEXT_AND_STATE);
        assert_eq!(edit.cursor(), 3);
        assert_eq!(edit.insert(&mut text, "x", NO_LIMIT, any), TEXT_AND_STATE);
        assert_eq!(text, "e\u{301}xabc");
        assert_eq!((edit.anchor(), edit.cursor()), (4, 4));
    }

    #[test]
    fn an_insert_that_ends_in_a_joiner_places_the_cursor_after_the_joined_character() {
        let mut text = String::from("\u{1F469}b");
        let mut edit = TextEdit::default();
        assert_eq!(
            edit.insert(&mut text, "\u{1F468}\u{200D}", NO_LIMIT, any),
            TEXT_AND_STATE
        );
        assert_eq!(text, "\u{1F468}\u{200D}\u{1F469}b");
        assert_eq!(edit.cursor(), text.len() - 1);
    }

    #[test]
    fn a_path_steps_by_word_between_separators() {
        let text = "C:\\Games\\pc-98/disk_01.hdi";
        let ends: Vec<&str> = walk(text, 0, Motion::WordRight)
            .iter()
            .map(|&end| &text[..end])
            .collect();
        assert_eq!(
            ends,
            [
                "C",
                "C:\\Games",
                "C:\\Games\\pc",
                "C:\\Games\\pc-98",
                "C:\\Games\\pc-98/disk",
                "C:\\Games\\pc-98/disk_01",
                text,
            ]
        );
        let starts: Vec<&str> = walk(text, text.len(), Motion::WordLeft)
            .iter()
            .map(|&start| &text[start..])
            .collect();
        assert_eq!(starts[0], "hdi");
        assert_eq!(starts[1], "01.hdi");
        assert_eq!(starts[2], "disk_01.hdi");
        assert_eq!(starts.last(), Some(&text));
        assert_eq!(walk("/", 0, Motion::WordRight), [1]);
        assert_eq!(walk("a // ", 1, Motion::WordRight), [5]);
    }

    #[test]
    fn letters_of_another_script_start_a_new_word() {
        assert_eq!(walk("abcΩμέγα", 0, Motion::WordRight), [3, 13]);
        assert_eq!(walk("file2Ωμ", 0, Motion::WordRight), [9]);
        assert_eq!(walk("Москваmoscow", 0, Motion::WordRight), [12, 18]);
        assert_eq!(walk("第2章", 0, Motion::WordRight), [3, 4, 7]);
        assert_eq!(walk("한국어 text", 0, Motion::WordRight), [9, 14]);
    }

    #[test]
    fn a_clamp_follows_a_change_outside_the_model() {
        let mut text = String::from("abcdefgh");
        let mut edit = state(&text, 2, 7);
        text.truncate(4);
        assert_eq!(edit.clamp(&text), STATE);
        assert_eq!((edit.anchor(), edit.cursor()), (2, 4));
        assert_eq!(edit.clamp(&text), EditOutcome::default());

        text.clear();
        text.push_str("日e\u{301}");
        let mut edit = state("abcdef", 4, 4);
        assert_eq!(edit.clamp(&text), STATE);
        assert_eq!(edit.cursor(), 3);
        let mut edit = state("abcdef", 2, 2);
        edit.clamp(&text);
        assert_eq!(edit.cursor(), 0);

        let mut edit = state("abcdefgh", 1, 8);
        text.clear();
        assert_eq!(edit.selected_text(&text), "");
        assert_eq!(edit.insert(&mut text, "x", NO_LIMIT, any), TEXT_AND_STATE);
        assert_eq!(text, "x");
        assert_eq!((edit.anchor(), edit.cursor()), (1, 1));
    }

    #[test]
    fn steps_and_selections_allocate_nothing() {
        let text = "日本語のテキスト and e\u{301}t\u{E9}";
        let mut edit = TextEdit::default();
        let (allocations, ()) = count_allocations(|| {
            for motion in [
                Motion::End,
                Motion::WordLeft,
                Motion::Left,
                Motion::WordRight,
                Motion::Right,
                Motion::Start,
            ] {
                edit.go_to(text, motion, true);
                edit.go_to(text, motion, false);
            }
            edit.select_all(text);
            edit.clamp(text);
            assert!(!edit.selected_text(text).is_empty());
        });
        assert_eq!(allocations, 0);
    }
}
