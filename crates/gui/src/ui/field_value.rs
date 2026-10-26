//! The editable value of a field: the edits of a pass over its text and the
//! drawing of the text, the cursor, the selection and the composition.

use super::{
    context::{UiContext, UiId},
    focus::Navigation,
    input::EditCommand,
    label::{Align, Truncation},
    text_edit::Motion,
};
use crate::{Color, Rect, Run};

/// The width of the text cursor.
const CURSOR_WIDTH: f32 = 1.5;

/// The height of the text cursor and of the selection highlight in units of
/// the text size.
const CURSOR_HEIGHT_SIZES: f32 = 1.3;

/// Applies the input of the pass to `text` of the focused field `id` through
/// the edit state of the context. The committed text goes to the cursor in
/// place of the selection, one character after the other, up to `limit`
/// characters. Control characters are dropped. Backspace and delete remove
/// the selection or the character before or after the cursor. Select all
/// selects the whole text. `navigation` steps the cursor, and the modifiers
/// of the input turn a step into a word step, a step to an edge or an
/// extension of the selection. `accept_insert` judges the whole candidate
/// text of an insert. A removal always happens. A field that is blocked or
/// that draws in a layer that opened in this pass keeps its text. Returns
/// whether an edit changed `text`.
pub(super) fn edit_text(
    context: &mut UiContext,
    id: UiId,
    text: &mut String,
    limit: usize,
    navigation: Option<Navigation>,
    accept_insert: impl Fn(&str) -> bool,
) -> bool {
    let input = context.input;
    let takes_input = context.takes_input();
    let edit = context.state.text_edit(id, text);
    if !takes_input {
        return false;
    }
    let mut changed = false;
    let mut buffer = [0; 4];
    for character in input
        .text
        .chars()
        .filter(|character| !character.is_control())
    {
        let inserted = character.encode_utf8(&mut buffer);
        changed |= edit
            .insert(text, inserted, limit, &accept_insert)
            .text_changed;
    }
    if input.backspace {
        changed |= edit.remove_backward(text).text_changed;
    }
    match input.edit {
        Some(EditCommand::DeleteForward) => {
            changed |= edit.remove_forward(text).text_changed;
        }
        Some(EditCommand::SelectAll) => {
            edit.select_all(text);
        }
        None => {}
    }
    let modifiers = input.modifiers;
    let motion = match navigation {
        Some(Navigation::Left) if modifiers.edge => Some(Motion::Start),
        Some(Navigation::Right) if modifiers.edge => Some(Motion::End),
        Some(Navigation::Left) if modifiers.word => Some(Motion::WordLeft),
        Some(Navigation::Right) if modifiers.word => Some(Motion::WordRight),
        Some(Navigation::Left) => Some(Motion::Left),
        Some(Navigation::Right) => Some(Motion::Right),
        Some(Navigation::Home) => Some(Motion::Start),
        Some(Navigation::End) => Some(Motion::End),
        Some(
            Navigation::Up
            | Navigation::Down
            | Navigation::PageUp
            | Navigation::PageDown
            | Navigation::Next
            | Navigation::Previous
            | Navigation::PreviousTab
            | Navigation::NextTab,
        )
        | None => None,
    };
    if let Some(motion) = motion {
        edit.go_to(text, motion, modifiers.extend);
    }
    changed
}

/// Places the cursor of the focused field `id` at the end of `text`
/// without a selection, for a text that the field replaced as a whole.
pub(super) fn place_cursor_at_end(context: &mut UiContext, id: UiId, text: &str) {
    context
        .state
        .text_edit(id, text)
        .go_to(text, Motion::End, false);
}

/// The index of the first glyph of `run` at or after the byte offset
/// `offset` of its text.
fn glyph_index(run: &Run, offset: usize) -> usize {
    run.glyphs.partition_point(|glyph| glyph.cluster < offset)
}

/// The x of the byte offset `offset` of the text of `run` in em: the origin
/// of the first glyph at or after it, or the end of the run.
fn offset_x(run: &Run, offset: usize) -> f32 {
    run.glyphs
        .get(glyph_index(run, offset))
        .map_or(run.advance, |glyph| glyph.x)
}

/// Draws the value of a field: `text` in `inner` with the color `color`,
/// or `placeholder` while the field is empty. `inner` spans the height of
/// the field. `focus` is the id of the field while it holds the focus.
///
/// A focused field draws the cursor at its position, the selection
/// highlight behind the selected text and the composition of the input
/// method at the cursor. The glyphs of the shaped text give the x of the
/// cursor and of the selection. The text scrolls when the cursor leaves
/// `inner` and keeps its scroll offset while the cursor stays visible.
/// Without the focus a text wider than `inner` is cut as `truncation` says.
/// A placeholder wider than `inner` is cut as `truncation` says.
pub(super) fn draw_value(
    context: &mut UiContext,
    inner: Rect,
    text: &str,
    placeholder: &str,
    focus: Option<UiId>,
    color: Color,
    truncation: Truncation,
) {
    let theme = context.theme;
    let composition = if focus.is_some() {
        context.input.composition.as_str()
    } else {
        ""
    };
    let style = theme.body().with_color(color);
    if text.is_empty() && composition.is_empty() {
        context.text_in(
            inner,
            &style.with_color(theme.colors.text_disabled),
            placeholder,
            Align::Start,
            truncation,
        );
    }
    let Some(id) = focus else {
        if !text.is_empty() {
            context.text_in(inner, &style, text, Align::Start, truncation);
        }
        return;
    };
    let edit = *context.state.text_edit(id, text);
    let baseline = context.centered_baseline(inner, &style);
    let cursor_width = context.snap(CURSOR_WIDTH).max(1.0 / context.scale());
    context.scene.push_clip(Rect::new(
        inner.x - cursor_width,
        inner.y + theme.stroke,
        inner.width + cursor_width * 2.0,
        inner.height - theme.stroke * 2.0,
    ));
    let run = context.fonts.shape(style.font, text);
    let size = style.size;
    let cursor = edit.cursor();
    let composition_width = context.measure(&style, composition);
    let before_cursor = offset_x(&run, cursor) * size;
    let cursor_offset = before_cursor + composition_width;
    let room = (inner.width - cursor_width).max(0.0);
    let mut scroll = context.state.text_scroll;
    if cursor_offset - scroll > room {
        scroll = cursor_offset - room;
    }
    if cursor_offset < scroll {
        scroll = cursor_offset;
    }
    let content = run.advance * size + composition_width;
    scroll = scroll.min(content - room).max(0.0);
    context.state.text_scroll = scroll;
    let x = context.snap(inner.x - scroll);
    let height = (theme.text_size * CURSOR_HEIGHT_SIZES).round();
    let top = context.snap(inner.y + (inner.height - height) * 0.5);
    if edit.has_selection() {
        let selection = edit.selection();
        let shift = |offset: usize, after: bool| {
            if after {
                offset_x(&run, offset) * size + composition_width
            } else {
                offset_x(&run, offset) * size
            }
        };
        let start = context.snap(x + shift(selection.start, selection.start >= cursor));
        let end = context.snap(x + shift(selection.end, selection.end > cursor));
        context.scene.fill(
            Rect::new(start, top, end - start, height),
            0.0,
            theme.colors.selection_focused,
        );
    }
    let split = glyph_index(&run, cursor);
    context
        .scene
        .run_glyphs((x, baseline), size, &run, 0..split, style.color);
    if !composition.is_empty() {
        let start = context.snap(x + before_cursor);
        context.text(start, baseline, &style, composition);
        let underline = (1.0 / context.scale()).max(context.snap(1.0));
        context.scene.fill(
            Rect::new(start, baseline + 2.0, composition_width, underline),
            0.0,
            theme.colors.text,
        );
    }
    context.scene.run_glyphs(
        (x + cursor_offset, baseline),
        size,
        &run,
        split..run.glyphs.len(),
        style.color,
    );
    let cursor_x = context.snap(x + cursor_offset);
    context.set_text_caret(cursor_x);
    context.scene.fill(
        Rect::new(cursor_x, top, cursor_width, height),
        0.0,
        theme.colors.text,
    );
    context.scene.pop_clip();
}

#[cfg(test)]
pub(super) mod tests {
    use std::ops::Range;

    use super::*;
    use crate::{
        Instance,
        ui::{EditModifiers, TEXT_FIELD_LIMIT, tests::Harness},
    };

    /// The area of the field.
    const FIELD: Rect = Rect::new(10.0, 10.0, 200.0, 32.0);

    /// The room of an icon before the value and of a clear button after it.
    const SIDE_ROOM: f32 = 30.0;

    /// No modifier keys.
    const PLAIN: EditModifiers = EditModifiers {
        extend: false,
        word: false,
        edge: false,
    };

    /// The modifier that extends the selection.
    pub(crate) const SHIFT: EditModifiers = EditModifiers {
        extend: true,
        ..PLAIN
    };

    /// The modifier of a word step.
    pub(crate) const WORD: EditModifiers = EditModifiers {
        word: true,
        ..PLAIN
    };

    /// The modifiers of a word step that extends the selection.
    pub(crate) const SHIFT_WORD: EditModifiers = EditModifiers {
        extend: true,
        word: true,
        ..PLAIN
    };

    /// The modifier of a step to an edge.
    const EDGE: EditModifiers = EditModifiers {
        edge: true,
        ..PLAIN
    };

    /// The value of the field `id` in `rect` over `text`: the edits of the
    /// pass while the field holds the focus, then the drawing between the
    /// side rooms. Returns whether the text changed.
    fn value(context: &mut UiContext, id: &str, rect: Rect, text: &mut String) -> bool {
        let id = UiId::from(id);
        let mut changed = false;
        if context.has_focus(id) {
            let navigation = context.navigation(id);
            changed = edit_text(context, id, text, TEXT_FIELD_LIMIT, navigation, |_| true);
        }
        context.text_focusable(id, rect, false);
        let focus = context.has_focus(id).then_some(id);
        let inner = Rect::new(
            rect.x + SIDE_ROOM,
            rect.y,
            rect.width - SIDE_ROOM * 2.0,
            rect.height,
        );
        let color = context.theme.colors.text;
        draw_value(
            context,
            inner,
            text,
            "Filter",
            focus,
            color,
            Truncation::End,
        );
        changed
    }

    /// Draws the value of the field over `text` and returns whether the
    /// text changed.
    fn draw(harness: &mut Harness, text: &mut String) -> bool {
        harness.frame(|context| value(context, "filter", FIELD, text))
    }

    /// Sends `navigation` with `modifiers` to the field over `text`.
    fn key(
        harness: &mut Harness,
        text: &mut String,
        navigation: Navigation,
        modifiers: EditModifiers,
    ) -> bool {
        harness.navigate_with(navigation, modifiers);
        draw(harness, text)
    }

    /// Types `typed` into the field over `text`.
    fn typing(harness: &mut Harness, text: &mut String, typed: &str) -> bool {
        harness.input.text.push_str(typed);
        draw(harness, text)
    }

    /// Sends the edit command `command` to the field over `text`.
    fn command(harness: &mut Harness, text: &mut String, command: EditCommand) -> bool {
        harness.input.edit = Some(command);
        draw(harness, text)
    }

    /// Sends backspace to the field over `text`.
    fn backspace(harness: &mut Harness, text: &mut String) -> bool {
        harness.input.backspace = true;
        draw(harness, text)
    }

    /// The anchor and the cursor of the edit state.
    fn ends(harness: &Harness) -> (usize, usize) {
        let edit = harness.state.text_edit;
        (edit.anchor(), edit.cursor())
    }

    /// The selected byte range of the edit state.
    fn selection(harness: &Harness) -> Range<usize> {
        harness.state.text_edit.selection()
    }

    /// The x of the glyphs of the last pass in physical pixels.
    fn glyph_xs(harness: &Harness) -> Vec<f32> {
        harness
            .scene
            .instances()
            .iter()
            .filter(|instance| instance.kind[0] == Instance::KIND_GLYPH)
            .map(|instance| instance.rect[0])
            .collect()
    }

    /// Asserts that the glyph positions `left` and `right` agree up to a
    /// rounding of the float sums.
    fn assert_same_glyphs(left: &[f32], right: &[f32]) {
        assert_eq!(left.len(), right.len(), "{left:?} {right:?}");
        for (left_x, right_x) in left.iter().zip(right) {
            assert!((left_x - right_x).abs() < 0.01, "{left:?} {right:?}");
        }
    }

    /// The fills of the selection highlight of the last pass.
    pub(crate) fn highlights(harness: &Harness) -> Vec<[f32; 4]> {
        let color = harness.theme.colors.selection_focused.to_linear();
        harness
            .scene
            .instances()
            .iter()
            .filter(|instance| {
                instance.kind[0] == Instance::KIND_RECTANGLE && instance.color == color
            })
            .map(|instance| instance.rect)
            .collect()
    }

    /// A harness with the focus on the field over `text` after one pass.
    fn focused(text: &mut String) -> Harness {
        let mut harness = Harness::new();
        harness.state.set_focus(UiId::from("filter"));
        draw(&mut harness, text);
        harness
    }

    /// The rectangle of the text cursor of the last pass, a fill of the
    /// cursor width in physical pixels.
    pub(crate) fn cursor(harness: &Harness) -> Option<[f32; 4]> {
        let width = (CURSOR_WIDTH * harness.viewport.scale).round().max(1.0);
        let height =
            (harness.theme.text_size * CURSOR_HEIGHT_SIZES).round() * harness.viewport.scale;
        harness
            .scene
            .instances()
            .iter()
            .find(|instance| {
                instance.kind[0] == Instance::KIND_RECTANGLE
                    && instance.rect[2] == width
                    && instance.rect[3] == height.round()
            })
            .map(|instance| instance.rect)
    }

    #[test]
    fn backspace_removes_whole_characters() {
        let mut harness = Harness::new();
        let mut text = String::from("aé漢");
        harness.state.set_focus(UiId::from("filter"));
        backspace(&mut harness, &mut text);
        assert_eq!(text, "aé");
        for _ in 0..2 {
            assert!(backspace(&mut harness, &mut text));
        }
        assert_eq!(text, "");
        assert!(!backspace(&mut harness, &mut text));
    }

    /// Verifies that a character and a backspace in one pass leave the text as it was, and that
    /// a pass without input allocates nothing.
    #[test]
    fn a_typed_character_and_a_backspace_in_one_pass_leave_the_text() {
        let mut harness = Harness::new();
        let mut text = String::from("ab");
        harness.state.set_focus(UiId::from("filter"));
        draw(&mut harness, &mut text);
        harness.input.text.push('漢');
        backspace(&mut harness, &mut text);
        assert_eq!(text, "ab");
        harness.input.text.push_str("cd");
        assert!(backspace(&mut harness, &mut text));
        assert_eq!(text, "abc");
        draw(&mut harness, &mut text);
        let (allocations, changed) =
            allocation_counter::count_allocations(|| draw(&mut harness, &mut text));
        assert!(!changed);
        assert_eq!(allocations, 0);
    }

    #[test]
    fn a_long_text_scrolls_and_keeps_the_cursor_inside() {
        let mut harness = Harness::new();
        let mut text = String::new();
        harness.state.set_focus(UiId::from("filter"));
        draw(&mut harness, &mut text);
        let empty = cursor(&harness).unwrap();
        harness.input.text = "The quick brown fox jumps over the lazy dog".into();
        draw(&mut harness, &mut text);
        let long = cursor(&harness).unwrap();
        assert!(long[0] > empty[0]);
        assert!(long[0] + long[2] <= FIELD.right() - 20.0, "{long:?}");
        let glyphs = glyph_xs(&harness).len();
        assert!(glyphs < text.chars().filter(|character| *character != ' ').count());
    }

    #[test]
    fn the_text_stops_at_the_limit() {
        let mut harness = Harness::new();
        let mut text = "a".repeat(TEXT_FIELD_LIMIT - 2);
        harness.state.set_focus(UiId::from("filter"));
        assert!(typing(&mut harness, &mut text, "漢字かな"));
        assert_eq!(text.chars().count(), TEXT_FIELD_LIMIT);
        assert!(text.ends_with("漢字"));
        assert!(!typing(&mut harness, &mut text, "b"));
    }

    #[test]
    fn the_composition_shows_at_the_cursor() {
        let mut harness = Harness::new();
        let mut text = String::from("ab");
        harness.state.set_focus(UiId::from("filter"));
        draw(&mut harness, &mut text);
        let plain = cursor(&harness).unwrap();
        harness.input.composition.push_str("かな");
        draw(&mut harness, &mut text);
        let composing = cursor(&harness).unwrap();
        assert!(composing[0] > plain[0] + 20.0);
        assert_eq!(text, "ab");
        harness.input.composition.clear();
        typing(&mut harness, &mut text, "かな");
        assert_eq!(text, "abかな");
        assert_eq!(cursor(&harness).unwrap()[0], composing[0]);
    }

    #[test]
    fn the_focus_places_the_cursor_at_the_end_without_a_selection() {
        let mut text = String::from("one two");
        let harness = focused(&mut text);
        assert_eq!(ends(&harness), (7, 7));
        assert!(highlights(&harness).is_empty());
    }

    #[test]
    fn left_right_home_and_end_step_the_cursor_and_typed_text_goes_to_it() {
        let mut text = String::from("one two");
        let mut harness = focused(&mut text);
        assert!(!key(&mut harness, &mut text, Navigation::Left, PLAIN));
        assert_eq!(ends(&harness), (6, 6));
        assert!(typing(&mut harness, &mut text, "X"));
        assert_eq!(text, "one twXo");
        key(&mut harness, &mut text, Navigation::Home, PLAIN);
        assert_eq!(ends(&harness), (0, 0));
        typing(&mut harness, &mut text, "Y");
        assert_eq!(text, "Yone twXo");
        key(&mut harness, &mut text, Navigation::Right, PLAIN);
        typing(&mut harness, &mut text, "日本");
        assert_eq!(text, "Yo日本ne twXo");
        assert_eq!(ends(&harness), (8, 8));
        key(&mut harness, &mut text, Navigation::End, PLAIN);
        typing(&mut harness, &mut text, "Z");
        assert_eq!(text, "Yo日本ne twXoZ");
        key(&mut harness, &mut text, Navigation::Left, EDGE);
        assert_eq!(ends(&harness), (0, 0));
        key(&mut harness, &mut text, Navigation::Right, EDGE);
        assert_eq!(ends(&harness), (text.len(), text.len()));
        harness.navigate_gamepad(Navigation::Left);
        draw(&mut harness, &mut text);
        assert_eq!(ends(&harness), (text.len() - 1, text.len() - 1));
    }

    #[test]
    fn word_steps_and_edges_extend_the_selection_with_shift() {
        let mut text = String::from("alpha beta gamma");
        let mut harness = focused(&mut text);
        key(&mut harness, &mut text, Navigation::Left, WORD);
        assert_eq!(ends(&harness), (11, 11));
        key(&mut harness, &mut text, Navigation::Left, SHIFT_WORD);
        assert_eq!(&text[selection(&harness)], "beta ");
        key(&mut harness, &mut text, Navigation::Home, SHIFT);
        assert_eq!(ends(&harness), (11, 0));
        key(&mut harness, &mut text, Navigation::End, SHIFT);
        assert_eq!(&text[selection(&harness)], "gamma");
        key(&mut harness, &mut text, Navigation::Left, PLAIN);
        assert_eq!(ends(&harness), (11, 11));
        key(&mut harness, &mut text, Navigation::Right, SHIFT);
        assert_eq!(&text[selection(&harness)], "g");
        key(&mut harness, &mut text, Navigation::Left, SHIFT);
        key(&mut harness, &mut text, Navigation::Left, SHIFT);
        assert_eq!(ends(&harness), (11, 10));
        key(&mut harness, &mut text, Navigation::Right, WORD);
        assert_eq!(ends(&harness), (16, 16));
        key(&mut harness, &mut text, Navigation::Left, WORD);
        key(&mut harness, &mut text, Navigation::Left, WORD);
        assert_eq!(ends(&harness), (6, 6));
        key(&mut harness, &mut text, Navigation::Right, SHIFT_WORD);
        assert_eq!(&text[selection(&harness)], "beta");
        key(
            &mut harness,
            &mut text,
            Navigation::Left,
            EditModifiers {
                extend: true,
                edge: true,
                ..PLAIN
            },
        );
        assert_eq!(ends(&harness), (6, 0));
        assert_eq!(text, "alpha beta gamma");
    }

    #[test]
    fn typed_text_replaces_the_selection() {
        let mut text = String::from("held");
        let mut harness = focused(&mut text);
        key(&mut harness, &mut text, Navigation::Left, SHIFT);
        key(&mut harness, &mut text, Navigation::Left, SHIFT);
        assert!(typing(&mut harness, &mut text, "X"));
        assert_eq!(text, "heX");
        assert_eq!(ends(&harness), (3, 3));
        key(&mut harness, &mut text, Navigation::Home, SHIFT);
        typing(&mut harness, &mut text, "ディスク");
        assert_eq!(text, "ディスク");
    }

    #[test]
    fn backspace_and_delete_remove_the_selection_or_one_character() {
        let mut text = String::from("abcde");
        let mut harness = focused(&mut text);
        key(&mut harness, &mut text, Navigation::Home, PLAIN);
        assert!(!backspace(&mut harness, &mut text));
        assert!(command(&mut harness, &mut text, EditCommand::DeleteForward));
        assert_eq!(text, "bcde");
        key(&mut harness, &mut text, Navigation::End, PLAIN);
        assert!(!command(
            &mut harness,
            &mut text,
            EditCommand::DeleteForward
        ));
        assert!(backspace(&mut harness, &mut text));
        assert_eq!(text, "bcd");
        key(&mut harness, &mut text, Navigation::Left, SHIFT);
        key(&mut harness, &mut text, Navigation::Left, SHIFT);
        command(&mut harness, &mut text, EditCommand::DeleteForward);
        assert_eq!(text, "b");
        key(&mut harness, &mut text, Navigation::Home, PLAIN);
        typing(&mut harness, &mut text, "xyz");
        key(&mut harness, &mut text, Navigation::Left, SHIFT);
        backspace(&mut harness, &mut text);
        assert_eq!(text, "xyb");
        assert_eq!(ends(&harness), (2, 2));
    }

    #[test]
    fn select_all_selects_the_whole_text() {
        let mut text = String::from("ディスク A");
        let mut harness = focused(&mut text);
        key(&mut harness, &mut text, Navigation::Home, PLAIN);
        assert!(!command(&mut harness, &mut text, EditCommand::SelectAll));
        assert_eq!(ends(&harness), (0, text.len()));
        assert_eq!(highlights(&harness).len(), 1);
        typing(&mut harness, &mut text, "B");
        assert_eq!(text, "B");
        command(&mut harness, &mut text, EditCommand::SelectAll);
        assert!(backspace(&mut harness, &mut text));
        assert!(text.is_empty());
    }

    #[test]
    fn an_insert_in_the_middle_keeps_the_limit() {
        let mut text = "a".repeat(TEXT_FIELD_LIMIT - 1);
        let mut harness = focused(&mut text);
        key(&mut harness, &mut text, Navigation::Home, PLAIN);
        assert!(typing(&mut harness, &mut text, "漢字"));
        assert_eq!(text.chars().count(), TEXT_FIELD_LIMIT);
        assert!(text.starts_with("漢a"));
        assert!(!typing(&mut harness, &mut text, "b"));
        key(&mut harness, &mut text, Navigation::Right, SHIFT);
        assert!(typing(&mut harness, &mut text, "字"));
        assert!(text.starts_with("漢字a"));
        assert_eq!(text.chars().count(), TEXT_FIELD_LIMIT);
    }

    /// Verifies that a long text scrolls to its start and back to its end with the cursor, and
    /// that a step that leaves the cursor visible keeps the scroll offset.
    #[test]
    fn a_long_text_scrolls_both_ways_with_the_cursor() {
        let mut text = String::from("The quick brown fox jumps over the lazy dog");
        let mut harness = focused(&mut text);
        let at_end = cursor(&harness).unwrap();
        let end_glyphs = glyph_xs(&harness);
        assert!(at_end[0] + at_end[2] <= FIELD.right() - 20.0, "{at_end:?}");
        key(&mut harness, &mut text, Navigation::Left, PLAIN);
        assert!(cursor(&harness).unwrap()[0] < at_end[0]);
        assert_same_glyphs(&glyph_xs(&harness), &end_glyphs);
        key(&mut harness, &mut text, Navigation::Home, PLAIN);
        let at_start = cursor(&harness).unwrap();
        assert!(at_start[0] < FIELD.x + 40.0, "{at_start:?}");
        let start_glyphs = glyph_xs(&harness);
        assert!(start_glyphs[0] > end_glyphs[0]);
        key(&mut harness, &mut text, Navigation::Right, PLAIN);
        assert_same_glyphs(&glyph_xs(&harness), &start_glyphs);
        key(&mut harness, &mut text, Navigation::End, PLAIN);
        assert_eq!(cursor(&harness).unwrap(), at_end);
        assert_same_glyphs(&glyph_xs(&harness), &end_glyphs);
    }

    /// Verifies that the composition of an input method shows at a cursor inside the text and
    /// that its commit goes to the cursor in place of the selection.
    #[test]
    fn the_composition_shows_at_a_cursor_inside_the_text() {
        let mut text = String::from("abcd");
        let mut harness = focused(&mut text);
        key(&mut harness, &mut text, Navigation::Home, PLAIN);
        key(&mut harness, &mut text, Navigation::Right, PLAIN);
        key(&mut harness, &mut text, Navigation::Right, PLAIN);
        let plain = cursor(&harness).unwrap();
        let plain_glyphs = glyph_xs(&harness);
        harness.input.composition.push_str("かな");
        draw(&mut harness, &mut text);
        let composing = cursor(&harness).unwrap();
        assert!(composing[0] > plain[0] + 20.0);
        let glyphs = glyph_xs(&harness);
        assert_eq!(glyphs.len(), plain_glyphs.len() + 2);
        assert_same_glyphs(&glyphs[..2], &plain_glyphs[..2]);
        let shift = glyphs[5] - plain_glyphs[3];
        assert!(shift > 20.0, "{shift}");
        assert!((glyphs[4] - plain_glyphs[2] - shift).abs() < 0.01);
        assert_eq!(
            harness.state.text_input_caret(),
            Some(composing[0] - FIELD.x)
        );
        harness.input.composition.clear();
        typing(&mut harness, &mut text, "かな");
        assert_eq!(text, "abかなcd");
        assert_eq!(cursor(&harness).unwrap()[0], composing[0]);

        key(&mut harness, &mut text, Navigation::Right, SHIFT);
        harness.input.composition.push('x');
        draw(&mut harness, &mut text);
        assert_eq!(text, "abかなcd");
        harness.input.composition.clear();
        typing(&mut harness, &mut text, "X");
        assert_eq!(text, "abかなXd");
    }

    /// Verifies that the selection highlight spans the selected glyphs and draws before them, and
    /// that a field without the focus draws no highlight and no cursor.
    #[test]
    fn the_selection_draws_behind_the_selected_text() {
        let mut text = String::from("abcd");
        let mut harness = focused(&mut text);
        let plain_glyphs = glyph_xs(&harness);
        key(&mut harness, &mut text, Navigation::Left, SHIFT);
        key(&mut harness, &mut text, Navigation::Left, SHIFT);
        let highlight = highlights(&harness);
        assert_eq!(highlight.len(), 1);
        let [x, _, width, height] = highlight[0];
        assert!(
            x <= plain_glyphs[2] && plain_glyphs[2] < x + width,
            "{highlight:?}"
        );
        assert!(x + width <= plain_glyphs[3] + 20.0, "{highlight:?}");
        assert_eq!(
            height,
            (harness.theme.text_size * CURSOR_HEIGHT_SIZES).round()
        );
        assert_same_glyphs(&glyph_xs(&harness), &plain_glyphs);
        let instances = harness.scene.instances();
        let color = harness.theme.colors.selection_focused.to_linear();
        let fill = instances
            .iter()
            .position(|instance| instance.color == color)
            .unwrap();
        let first_glyph = instances
            .iter()
            .position(|instance| instance.kind[0] == Instance::KIND_GLYPH)
            .unwrap();
        assert!(fill < first_glyph);

        harness.state.clear_focus();
        draw(&mut harness, &mut text);
        assert!(highlights(&harness).is_empty());
        assert!(cursor(&harness).is_none());
    }

    /// Verifies that a focus that leaves the field and comes back resets the cursor to the end
    /// without a selection.
    #[test]
    fn a_focus_change_resets_the_edit_state() {
        let mut first = String::from("first text");
        let mut second = String::from("second");
        let mut harness = Harness::new();
        let mut draw_both = |harness: &mut Harness| {
            harness.frame(|context| {
                value(context, "first", FIELD, &mut first);
                value(
                    context,
                    "second",
                    Rect::new(10.0, 60.0, 200.0, 32.0),
                    &mut second,
                );
            });
        };
        harness.state.set_focus(UiId::from("first"));
        draw_both(&mut harness);
        harness.navigate_with(Navigation::Home, SHIFT);
        draw_both(&mut harness);
        assert_eq!(ends(&harness), (10, 0));
        harness.navigate(Navigation::Next);
        draw_both(&mut harness);
        assert_eq!(harness.state.focused(), Some(UiId::from("second")));
        assert_eq!(ends(&harness), (6, 6));
        harness.navigate(Navigation::Previous);
        draw_both(&mut harness);
        assert_eq!(harness.state.focused(), Some(UiId::from("first")));
        assert_eq!(ends(&harness), (10, 10));
    }

    #[test]
    fn the_text_caret_follows_the_cursor() {
        let mut text = String::from("caret");
        let mut harness = focused(&mut text);
        let end = harness.state.text_input_caret().unwrap();
        assert_eq!(end, cursor(&harness).unwrap()[0] - FIELD.x);
        key(&mut harness, &mut text, Navigation::Home, PLAIN);
        let start = harness.state.text_input_caret().unwrap();
        assert!(start < end - 20.0, "{start} {end}");
        assert_eq!(start, cursor(&harness).unwrap()[0] - FIELD.x);
    }
}
