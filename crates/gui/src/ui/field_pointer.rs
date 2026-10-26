//! The pointer over the value of a field: the press that places the
//! cursor, the drag that extends the selection and the edge repeat that
//! extends it past an edge of the value.

use std::{
    iter,
    time::{Duration, Instant},
};

use super::{
    context::{PointerResponse, UiContext, UiId},
    field_value::offset_x,
    label::Truncation,
    text_edit::{Motion, is_position},
};
use crate::{Rect, Run};

/// The interval between two steps of the edge repeat. Each step extends the
/// selection by one position.
pub(super) const EDGE_REPEAT_INTERVAL: Duration = Duration::from_millis(50);

/// An edge of the value area of a field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Edge {
    /// The left edge, before the start of the text.
    Start,
    /// The right edge, after the end of the text.
    End,
}

/// The edge repeat of a drag in a text widget: the edge that the drag holds
/// the pointer past and the time of the next step.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct EdgeRepeat {
    /// The edge that the drag held the pointer past in the last pass.
    pub(super) edge: Option<Edge>,
    /// Whether the cursor is at the end of the text in the direction of the
    /// edge: the start for the left edge and the end for the right edge.
    pub(super) at_text_end: bool,
    /// The time of the next step.
    deadline: Option<Instant>,
}

impl EdgeRepeat {
    /// Starts the steps one interval after `now` while the drag holds the
    /// pointer past an edge. A running repeat keeps its deadline. Without an
    /// edge, or with the cursor at the end of the text in the direction of
    /// the edge, the repeat stops.
    pub(super) fn update(&mut self, now: Instant) {
        if self.edge.is_some() && !self.at_text_end {
            self.deadline.get_or_insert(now + EDGE_REPEAT_INTERVAL);
        } else {
            self.deadline = None;
        }
    }

    /// The time of the next step.
    pub(super) fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    /// The steps that are due at `now`: one for the deadline and one for
    /// each further interval that passed. The next deadline follows the
    /// last due step by one interval.
    pub(super) fn take_steps(&mut self, now: Instant) -> u32 {
        let Some(deadline) = self.deadline.filter(|&deadline| deadline <= now) else {
            return 0;
        };
        let intervals = (now - deadline).as_nanos() / EDGE_REPEAT_INTERVAL.as_nanos();
        let steps = u32::try_from(intervals + 1).unwrap_or(u32::MAX);
        self.deadline = Some(deadline + EDGE_REPEAT_INTERVAL * steps);
        steps
    }
}

/// Applies the left button of the pass to the edit state of the focused
/// field `id` over `text`, whose value draws in `inner`. A field without the
/// focus draws its text cut as `truncation` says. `pointer` is the response
/// of the field to the pointer in this pass.
///
/// A press places the cursor at the position nearest to the pointer x,
/// clamped to `inner`, in the layout of the last pass. A press on a field
/// that the last pass drew without the focus finds the position in the cut
/// text and scrolls the text so the position keeps its drawn x. With the
/// extend modifier the press extends the selection to the position. The
/// second press of a double click selects the whole text.
///
/// While the button stays held after a press that placed the cursor, the
/// pointer x extends the selection. A pointer past an edge of `inner`
/// extends it to the visible text at that edge, and each step of the edge
/// repeat in the input extends it by one more position. The drawing of the
/// value scrolls the cursor into view. The held button takes the pointer
/// from the widgets after the field.
pub(super) fn point_value(
    context: &mut UiContext,
    id: UiId,
    inner: Rect,
    text: &str,
    truncation: Truncation,
    pointer: PointerResponse,
) {
    if !pointer.held {
        context.state.text_drag = false;
        context.state.text_edge.edge = None;
        return;
    }
    context.mouse_captured = true;
    let pointer_x = context.mouse().0;
    if pointer.pressed {
        press(context, id, inner, text, truncation, pointer_x);
    } else if context.state.text_drag {
        drag(context, id, inner, text, pointer_x);
    }
}

/// Applies a press at `pointer_x` to the edit state of the field `id` over
/// `text` with its value in `inner`, as [`point_value`] says.
fn press(
    context: &mut UiContext,
    id: UiId,
    inner: Rect,
    text: &str,
    truncation: Truncation,
    pointer_x: f32,
) {
    context.state.text_edge.edge = None;
    let drawn_without_focus = context.state.text_owner != Some(id);
    if context.input.left_double {
        context.state.text_drag = false;
        context.state.text_edit(id, text).select_all(text);
        return;
    }
    let extend = context.input.modifiers.extend;
    let x = pointer_x.clamp(inner.x, inner.right());
    if drawn_without_focus {
        let (position, drawn_x) = cut_position(context, inner, text, truncation, x);
        context
            .state
            .text_edit(id, text)
            .place(text, position, extend);
        let style = context.theme.body();
        let run = context.fonts.shape(style.font, text);
        context.state.text_scroll = offset_x(&run, position) * style.size - (drawn_x - inner.x);
    } else {
        let cursor = context.state.text_edit(id, text).cursor();
        let position = pointer_position(context, inner, text, cursor, x);
        context
            .state
            .text_edit(id, text)
            .place(text, position, extend);
    }
    context.state.text_drag = true;
}

/// Applies the held button with the pointer at `pointer_x` to the edit
/// state of the field `id` over `text` with its value in `inner`, as
/// [`point_value`] says. A pointer that stays past the same edge keeps the
/// cursor, so only the steps of the edge repeat extend the selection. The
/// repeat stops at the end of the text in the direction of the edge.
fn drag(context: &mut UiContext, id: UiId, inner: Rect, text: &str, pointer_x: f32) {
    let edge = if pointer_x < inner.x {
        Some(Edge::Start)
    } else if pointer_x > inner.right() {
        Some(Edge::End)
    } else {
        None
    };
    let held_edge = context.state.text_edge.edge;
    context.state.text_edge.edge = edge;
    let cursor = context.state.text_edit(id, text).cursor();
    let position = match edge {
        Some(edge) if held_edge == Some(edge) => cursor,
        Some(_) | None => {
            let x = pointer_x.clamp(inner.x, inner.right());
            pointer_position(context, inner, text, cursor, x)
        }
    };
    let steps = context.input.edge_steps as usize;
    let edit = context.state.text_edit(id, text);
    edit.place(text, position, true);
    let (motion, text_end) = match edge {
        Some(Edge::Start) => (Motion::Left, 0),
        Some(Edge::End) => (Motion::Right, text.len()),
        None => return,
    };
    for _ in 0..steps.min(text.len()) {
        edit.go_to(text, motion, true);
    }
    let at_text_end = edit.cursor() == text_end;
    context.state.text_edge.at_text_end = at_text_end;
}

/// The position of `text` nearest to `x` in the text that a field without
/// the focus draws in `inner`: the text cut to the width of `inner` as
/// `truncation` says. A position in the ellipsis gives the start or the end
/// of the cut part. Returns the position and its drawn x in logical units.
fn cut_position(
    context: &mut UiContext,
    inner: Rect,
    text: &str,
    truncation: Truncation,
    x: f32,
) -> (usize, f32) {
    let style = context.theme.body();
    let width = inner.width.max(0.0);
    let run = match truncation {
        Truncation::End => context.fonts.fit(style.font, style.size, text, width),
        Truncation::Middle => context
            .fonts
            .fit_middle(style.font, style.size, text, width),
    };
    let origin = context.snap(inner.x);
    let (position, position_x) = nearest_position(&run, text, (x - origin) / style.size);
    (position, origin + position_x * style.size)
}

/// The position of `text` nearest to `x` in the layout of the focused value
/// in `inner` with the cursor at `cursor` and the scroll offset of the last
/// pass. An x over the composition of an input method gives the cursor.
fn pointer_position(
    context: &mut UiContext,
    inner: Rect,
    text: &str,
    cursor: usize,
    x: f32,
) -> usize {
    let style = context.theme.body();
    let input = context.input;
    let composition_width = context.measure(&style, &input.composition);
    let run = context.fonts.shape(style.font, text);
    let origin = context.snap(inner.x - context.state.text_scroll);
    let cursor_x = offset_x(&run, cursor) * style.size;
    let mut x = x - origin;
    if x > cursor_x + composition_width {
        x -= composition_width;
    } else if x > cursor_x {
        x = cursor_x;
    }
    nearest_position(&run, text, x / style.size).0
}

/// The position of `text` nearest to `x` in em from the origin of `run`, a
/// shaped line of `text` whose glyphs hold the byte offsets of `text`, and
/// the x of that position in `run` in em. The candidates are the origins of
/// the glyphs at the positions of the text and the end of the run. An x
/// before the first or past the last candidate gives the first or the last
/// candidate. Of two candidates at the same distance the first one wins.
fn nearest_position(run: &Run, text: &str, x: f32) -> (usize, f32) {
    let position = run
        .glyphs
        .iter()
        .map(|glyph| (glyph.cluster, glyph.x))
        .chain(iter::once((text.len(), run.advance)))
        .filter(|&(offset, _)| is_position(text, offset))
        .min_by(|left, right| (left.1 - x).abs().total_cmp(&(right.1 - x).abs()))
        .map_or(0, |(offset, _)| offset);
    (position, offset_x(run, position))
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::ui::{
        Navigation,
        field_value::tests::{
            FIELD, PLAIN, SIDE_ROOM, assert_same_glyphs, cursor, draw, ends, focused, glyph_xs,
            key, selection, typing,
        },
        tests::Harness,
    };

    /// The x in logical units of the byte offset `offset` of `text` in a
    /// value area that starts at `inner_x`, with the scroll offset of the
    /// last pass.
    pub(crate) fn position_x(
        harness: &mut Harness,
        inner_x: f32,
        text: &str,
        offset: usize,
    ) -> f32 {
        let style = harness.theme.body();
        let run = harness.fonts.shape(style.font, text);
        (inner_x - harness.state.text_scroll).round() + offset_x(&run, offset) * style.size
    }

    /// Presses the left button at `(x, y)` and runs the pass `draw`.
    pub(crate) fn press_at<R>(
        harness: &mut Harness,
        x: f32,
        y: f32,
        draw: impl FnOnce(&mut Harness) -> R,
    ) -> R {
        harness.press(x, y);
        draw(harness)
    }

    /// The x in logical units of the byte offset `offset` of `text` in the
    /// value area of the field.
    fn field_x(harness: &mut Harness, text: &str, offset: usize) -> f32 {
        position_x(harness, FIELD.x + SIDE_ROOM, text, offset)
    }

    /// Presses the left button at `x` over the field over `text`.
    fn press_field(harness: &mut Harness, text: &mut String, x: f32) {
        press_at(harness, x, FIELD.center().1, |harness| draw(harness, text));
    }

    /// Moves the pointer to `x` with the button as it is and draws the
    /// field over `text`.
    fn pointer_to(harness: &mut Harness, text: &mut String, x: f32) {
        harness.mouse(x, FIELD.center().1);
        draw(harness, text);
    }

    /// Runs a pass over the field over `text` with `steps` steps of the edge
    /// repeat and the pointer where it is.
    fn edge_steps(harness: &mut Harness, text: &mut String, steps: u32) {
        harness.input.edge_steps = steps;
        draw(harness, text);
    }

    /// Presses and releases the left button at `x` over the field over
    /// `text`.
    fn click_at(harness: &mut Harness, text: &mut String, x: f32) {
        press_field(harness, text, x);
        harness.release();
        draw(harness, text);
    }

    /// The byte offsets of `text` whose x lies inside the value area of the
    /// field in the last pass.
    fn visible_offsets(harness: &mut Harness, text: &str) -> Vec<usize> {
        let inner = FIELD.x + SIDE_ROOM;
        let right = FIELD.right() - SIDE_ROOM;
        (0..=text.len())
            .filter(|&offset| text.is_char_boundary(offset))
            .filter(|&offset| {
                let x = field_x(harness, text, offset);
                x >= inner && x <= right
            })
            .collect()
    }

    /// Verifies that the nearest position of an x is the glyph boundary at the shortest distance
    /// and comes with the x of that boundary, that an x before the start or past the end clamps,
    /// and that a mark, a joiner and the inside of a character never give a position.
    #[test]
    fn the_nearest_position_is_the_closest_glyph_boundary() {
        let mut harness = Harness::new();
        let font = harness.theme.body().font;
        for text in [
            "abc",
            "ディスク",
            "ae\u{301}b",
            "a\u{1F469}\u{200D}\u{1F4BB}b",
            "",
        ] {
            let run = harness.fonts.shape(font, text);
            let positions: Vec<(usize, f32)> = (0..=text.len())
                .filter(|&offset| is_position(text, offset))
                .map(|offset| (offset, offset_x(&run, offset)))
                .collect();
            assert_eq!(nearest_position(&run, text, -5.0), (0, 0.0), "{text:?}");
            assert_eq!(
                nearest_position(&run, text, run.advance + 5.0),
                (text.len(), run.advance)
            );
            for &(offset, x) in &positions {
                assert_eq!(nearest_position(&run, text, x), (offset, x), "{text:?}");
            }
            let mut x = -0.5;
            while x < run.advance + 0.5 {
                let (position, position_x) = nearest_position(&run, text, x);
                assert!(is_position(text, position), "{text:?} {x}");
                assert_eq!(position_x, offset_x(&run, position), "{text:?} {x}");
                let distance = (offset_x(&run, position) - x).abs();
                for &(_, other) in &positions {
                    assert!(distance <= (other - x).abs() + 1e-6, "{text:?} {x}");
                }
                x += 0.01;
            }
        }
        let run = harness.fonts.shape(font, "abc");
        let middle = (run.glyphs[1].x + run.glyphs[2].x) * 0.5;
        assert_eq!(nearest_position(&run, "abc", middle - 0.01).0, 1);
        assert_eq!(nearest_position(&run, "abc", middle + 0.01).0, 2);
    }

    /// Verifies that a press places the cursor at the nearest position at the start, in the
    /// middle and at the end of the visible part of a scrolled text and keeps the scroll offset,
    /// and that a press in a short text beyond either end places it at that end.
    #[test]
    fn a_press_places_the_cursor_at_the_nearest_position_of_a_scrolled_text() {
        let mut text = String::from("The quick brown fox jumps over the lazy dog");
        let mut harness = focused(&mut text);
        let glyphs = glyph_xs(&harness);
        assert!(harness.state.text_scroll > 0.0);
        let visible = visible_offsets(&mut harness, &text);
        let start = visible[0];
        let middle = visible[visible.len() / 2];
        assert!(start > 0 && middle > start && middle < text.len());
        assert_eq!(visible.last(), Some(&text.len()));
        for (offset, shift) in [
            (start, 0.0),
            (start, 1.0),
            (middle, -1.0),
            (middle, 1.0),
            (text.len(), 0.0),
            (text.len(), -1.0),
        ] {
            let x = field_x(&mut harness, &text, offset) + shift;
            click_at(&mut harness, &mut text, x);
            assert_eq!(ends(&harness), (offset, offset), "{offset} {shift}");
            assert_same_glyphs(&glyph_xs(&harness), &glyphs);
        }

        let mut short = String::from("disk");
        let mut harness = focused(&mut short);
        click_at(&mut harness, &mut short, FIELD.x + 2.0);
        assert_eq!(ends(&harness), (0, 0));
        click_at(&mut harness, &mut short, FIELD.right() - 2.0);
        assert_eq!(ends(&harness), (4, 4));
        click_at(&mut harness, &mut short, FIELD.x + 2.0);
        click_at(&mut harness, &mut short, 500.0);
        assert_eq!(ends(&harness), (0, 0));
        assert_eq!(short, "disk");
    }

    /// Verifies that a press left or right of the value area of a scrolled text places the cursor
    /// at the visible text at that edge and leaves the scroll offset.
    #[test]
    fn a_press_beside_the_value_area_keeps_the_scroll() {
        let mut text = String::from("The quick brown fox jumps over the lazy dog");
        let mut harness = focused(&mut text);
        key(&mut harness, &mut text, Navigation::Left, PLAIN);
        let scroll = harness.state.text_scroll;
        assert!(scroll > 0.0);
        let glyphs = glyph_xs(&harness);
        let visible = visible_offsets(&mut harness, &text);
        click_at(&mut harness, &mut text, FIELD.x + 2.0);
        assert_eq!(ends(&harness), (visible[0], visible[0]));
        assert_eq!(harness.state.text_scroll, scroll);
        assert_same_glyphs(&glyph_xs(&harness), &glyphs);
        click_at(&mut harness, &mut text, FIELD.right() - 2.0);
        assert_eq!(ends(&harness), (text.len(), text.len()));
        assert_eq!(harness.state.text_scroll, scroll);
        assert_same_glyphs(&glyph_xs(&harness), &glyphs);
    }

    /// Verifies that a drag extends the selection from the press position in both directions,
    /// that a pointer past an edge extends it to the visible text at that edge, that each step
    /// of the edge repeat extends it by one position and scrolls the text with the cursor up to
    /// the end of the text, that a pass without a step and without motion changes nothing, and
    /// that the release ends the drag.
    #[test]
    fn a_drag_extends_the_selection_and_the_edge_repeat_scrolls_past_the_edges() {
        let mut text = String::from("The quick brown fox jumps over the lazy dog");
        let mut harness = focused(&mut text);
        let end_scroll = harness.state.text_scroll;
        let visible = visible_offsets(&mut harness, &text);
        let middle = visible[visible.len() / 2];
        let x = field_x(&mut harness, &text, middle);
        press_field(&mut harness, &mut text, x);
        assert_eq!(ends(&harness), (middle, middle));
        let x = field_x(&mut harness, &text, middle - 3);
        pointer_to(&mut harness, &mut text, x);
        assert_eq!(ends(&harness), (middle, middle - 3));
        let x = field_x(&mut harness, &text, middle + 2);
        pointer_to(&mut harness, &mut text, x);
        assert_eq!(ends(&harness), (middle, middle + 2));
        assert_eq!(harness.state.text_scroll, end_scroll);

        pointer_to(&mut harness, &mut text, 0.0);
        assert_eq!(ends(&harness), (middle, visible[0]));
        assert_eq!(harness.state.text_scroll, end_scroll);
        for _ in 0..3 {
            draw(&mut harness, &mut text);
            pointer_to(&mut harness, &mut text, 1.0);
        }
        assert_eq!(ends(&harness), (middle, visible[0]));
        assert_eq!(harness.state.text_scroll, end_scroll);
        let mut scroll = end_scroll;
        for expected in (0..visible[0]).rev() {
            edge_steps(&mut harness, &mut text, 1);
            assert_eq!(ends(&harness), (middle, expected));
            assert!(harness.state.text_scroll < scroll);
            scroll = harness.state.text_scroll;
            let start = cursor(&harness).unwrap();
            assert!(start[0] >= FIELD.x + SIDE_ROOM - 2.0, "{start:?}");
        }
        assert_eq!(harness.state.text_scroll, 0.0);
        edge_steps(&mut harness, &mut text, 3);
        assert_eq!(ends(&harness), (middle, 0));

        let shown = visible_offsets(&mut harness, &text);
        let edge = shown[shown.len() - 1];
        pointer_to(&mut harness, &mut text, 700.0);
        assert_eq!(ends(&harness), (middle, edge));
        assert!(harness.state.text_scroll < 1.0);
        edge_steps(&mut harness, &mut text, 2);
        assert_eq!(ends(&harness), (middle, edge + 2));
        while ends(&harness).1 < text.len() {
            edge_steps(&mut harness, &mut text, 1);
        }
        assert_eq!(ends(&harness), (middle, text.len()));
        assert_eq!(harness.state.text_scroll, end_scroll);
        assert_eq!(&text[selection(&harness)], &text[middle..]);

        harness.release();
        draw(&mut harness, &mut text);
        assert_eq!(harness.state.text_edge.edge, None);
        pointer_to(&mut harness, &mut text, 0.0);
        edge_steps(&mut harness, &mut text, 1);
        assert_eq!(ends(&harness), (middle, text.len()));
        assert_eq!(text, "The quick brown fox jumps over the lazy dog");
    }

    /// Verifies that a step of the edge repeat with the pointer back inside the value area does
    /// nothing beyond the position of the pointer.
    #[test]
    fn an_edge_step_inside_the_value_area_follows_the_pointer() {
        let mut text = String::from("The quick brown fox jumps over the lazy dog");
        let mut harness = focused(&mut text);
        let visible = visible_offsets(&mut harness, &text);
        let middle = visible[visible.len() / 2];
        let x = field_x(&mut harness, &text, middle);
        press_field(&mut harness, &mut text, x);
        let x = field_x(&mut harness, &text, middle - 2);
        harness.input.edge_steps = 4;
        pointer_to(&mut harness, &mut text, x);
        assert_eq!(ends(&harness), (middle, middle - 2));
    }

    /// Verifies that the edge repeat runs while a drag holds the pointer past an edge: the first
    /// step comes one interval after the pass that saw the edge, each further interval gives one
    /// step, and the release stops the repeat.
    #[test]
    fn the_edge_repeat_steps_once_for_each_interval() {
        let mut text = String::from("The quick brown fox jumps over the lazy dog");
        let mut harness = focused(&mut text);
        let visible = visible_offsets(&mut harness, &text);
        let start = Instant::now();
        let x = field_x(&mut harness, &text, visible[visible.len() / 2]);
        press_field(&mut harness, &mut text, x);
        harness.state.update_edge_repeat(start);
        assert_eq!(harness.state.next_edge_repeat(), None);
        assert_eq!(
            harness.state.take_edge_steps(start + EDGE_REPEAT_INTERVAL),
            0
        );

        pointer_to(&mut harness, &mut text, 0.0);
        assert_eq!(harness.state.text_edge.edge, Some(Edge::Start));
        harness.state.update_edge_repeat(start);
        let first = start + EDGE_REPEAT_INTERVAL;
        assert_eq!(harness.state.next_edge_repeat(), Some(first));
        harness
            .state
            .update_edge_repeat(start + EDGE_REPEAT_INTERVAL / 2);
        assert_eq!(harness.state.next_edge_repeat(), Some(first));
        let before = first - Duration::from_millis(1);
        assert_eq!(harness.state.take_edge_steps(before), 0);
        assert_eq!(harness.state.take_edge_steps(first), 1);
        assert_eq!(
            harness.state.next_edge_repeat(),
            Some(first + EDGE_REPEAT_INTERVAL)
        );
        let late = first + EDGE_REPEAT_INTERVAL * 3 + EDGE_REPEAT_INTERVAL / 2;
        assert_eq!(harness.state.take_edge_steps(late), 3);
        assert_eq!(
            harness.state.next_edge_repeat(),
            Some(first + EDGE_REPEAT_INTERVAL * 4)
        );

        harness.release();
        draw(&mut harness, &mut text);
        harness.state.update_edge_repeat(late);
        assert_eq!(harness.state.next_edge_repeat(), None);
        assert_eq!(
            harness.state.take_edge_steps(late + EDGE_REPEAT_INTERVAL),
            0
        );
    }

    /// Verifies that the edge repeat stops while the cursor is at the end of the text in the
    /// direction of the held edge, at the end for the right edge and at the start for the left
    /// edge, and that it runs at an edge with text left beyond it.
    #[test]
    fn the_edge_repeat_stops_at_the_end_of_the_text() {
        let mut text = String::from("The quick brown fox jumps over the lazy dog");
        let mut harness = focused(&mut text);
        let visible = visible_offsets(&mut harness, &text);
        let middle = visible[visible.len() / 2];
        let start = Instant::now();
        let x = field_x(&mut harness, &text, middle);
        press_field(&mut harness, &mut text, x);
        pointer_to(&mut harness, &mut text, 700.0);
        assert_eq!(ends(&harness), (middle, text.len()));
        harness.state.update_edge_repeat(start);
        assert_eq!(harness.state.next_edge_repeat(), None);

        pointer_to(&mut harness, &mut text, 0.0);
        assert_eq!(ends(&harness), (middle, visible[0]));
        harness.state.update_edge_repeat(start);
        assert_eq!(
            harness.state.next_edge_repeat(),
            Some(start + EDGE_REPEAT_INTERVAL)
        );
        let steps = u32::try_from(visible[0]).unwrap();
        edge_steps(&mut harness, &mut text, steps);
        assert_eq!(ends(&harness), (middle, 0));
        harness.state.update_edge_repeat(start);
        assert_eq!(harness.state.next_edge_repeat(), None);
        pointer_to(&mut harness, &mut text, 1.0);
        harness.state.update_edge_repeat(start);
        assert_eq!(harness.state.next_edge_repeat(), None);
        assert_eq!(
            harness
                .state
                .take_edge_steps(start + EDGE_REPEAT_INTERVAL * 4),
            0
        );

        pointer_to(&mut harness, &mut text, 700.0);
        harness.state.update_edge_repeat(start);
        assert!(ends(&harness).1 < text.len());
        assert_eq!(
            harness.state.next_edge_repeat(),
            Some(start + EDGE_REPEAT_INTERVAL)
        );
    }

    /// Verifies that a press in Japanese text places the cursor between two characters, and that
    /// a press over a combining mark never places it between the mark and its base.
    #[test]
    fn a_press_in_japanese_text_and_over_a_mark_lands_on_a_position() {
        let mut text = String::from("ディスク");
        let mut harness = focused(&mut text);
        for offset in [0, 3, 6, 9, 12] {
            let x = field_x(&mut harness, &text, offset) + 1.0;
            click_at(&mut harness, &mut text, x);
            assert_eq!(ends(&harness), (offset, offset));
        }
        typing(&mut harness, &mut text, "!");
        assert_eq!(text, "ディスク!");
        let x = field_x(&mut harness, &text, 6);
        click_at(&mut harness, &mut text, x);
        typing(&mut harness, &mut text, "-");
        assert_eq!(text, "ディ-スク!");

        let mut text = String::from("ae\u{301}b");
        let mut harness = focused(&mut text);
        let mark = field_x(&mut harness, &text, 2);
        for shift in [-0.5, 0.0, 0.5] {
            click_at(&mut harness, &mut text, mark + shift);
            let (anchor, cursor) = ends(&harness);
            assert_eq!(anchor, cursor);
            assert!(cursor == 1 || cursor == 4, "{cursor}");
        }
        let x = field_x(&mut harness, &text, 4);
        click_at(&mut harness, &mut text, x);
        assert_eq!(ends(&harness), (4, 4));
        typing(&mut harness, &mut text, "x");
        assert_eq!(text, "ae\u{301}xb");
    }

    /// Verifies that a press while an input method composes at the cursor finds the positions
    /// after the composition at their drawn x and that a press over the composition keeps the
    /// cursor.
    #[test]
    fn a_press_beside_the_composition_finds_the_drawn_positions() {
        let mut text = String::from("abcd");
        let mut harness = focused(&mut text);
        key(&mut harness, &mut text, Navigation::Home, PLAIN);
        key(&mut harness, &mut text, Navigation::Right, PLAIN);
        key(&mut harness, &mut text, Navigation::Right, PLAIN);
        harness.input.composition.push_str("かな");
        draw(&mut harness, &mut text);
        let style = harness.theme.body();
        let composition = harness.fonts.measure(style.font, style.size, "かな");
        let x = field_x(&mut harness, &text, 3) + composition;
        click_at(&mut harness, &mut text, x);
        assert_eq!(ends(&harness), (3, 3));
        let x = field_x(&mut harness, &text, 3) + composition * 0.5;
        click_at(&mut harness, &mut text, x);
        assert_eq!(ends(&harness), (3, 3));
        let x = field_x(&mut harness, &text, 1);
        click_at(&mut harness, &mut text, x);
        assert_eq!(ends(&harness), (1, 1));
    }
}
