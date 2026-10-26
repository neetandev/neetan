//! The vertical scroll area.

use super::{
    context::{UiContext, UiId},
    focus::ScrollAnchor,
    scroll_bar::ScrollBar,
};
use crate::Rect;

/// The distance one wheel step scrolls, in rows of the theme.
const WHEEL_ROWS: f32 = 3.0;

/// The state of a [`scroll_area`] that outlives a pass.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollState {
    /// The scroll offset of the content in logical units.
    pub offset: f32,
    /// The height of the content in the last pass.
    content_height: f32,
    /// The focused widget that the area scrolled into view last.
    revealed: Option<UiId>,
}

impl ScrollState {
    /// An area at its top.
    pub fn new() -> Self {
        Self::default()
    }
}

/// A vertical scroll area in `rect` that clips its content and shows the
/// scroll bar of a list at its right edge while the content overflows.
///
/// `content` draws the content into the rectangle it receives: the left
/// edge and the width of the content, with its top at the scrolled
/// position and the height of the view. It returns the height of the
/// content. The content leaves the padding of the theme free at its right
/// for the scroll bar.
///
/// A focus change by a key or the gamepad to a widget of the content outside
/// the view scrolls the widget into view with the gap of the theme above or
/// below it. A widget that does not fit the view with the margin above and
/// below it shows its top. A press of the pointer on a widget keeps the
/// offset. The pressed widget receives its click. A press that gives a
/// widget the focus through [`UiContext::focus_on_press`] keeps the offset
/// while the whole widget shows. Else it scrolls the widget into view as a
/// key does, also when the widget held the focus before the press.
///
/// The area groups the arrows of its content. Up and Down from a widget of
/// the content take the nearest widget of the content in their direction,
/// also one outside the view. They leave the area only when the content has
/// no widget in that direction. Left and Right take the nearest widget of
/// the same row in the outermost scroll area around the widget. They leave
/// the area only when that area has no such widget.
///
/// The first pass keeps the offset of `state` until the content tells its
/// height. A restored offset survives this way. The wheel over the area
/// scrolls it unless a widget of the content scrolled, and so does a drag of
/// the scroll bar. A change after the content drew asks for another pass,
/// see [`UiContext::request_pass`].
pub fn scroll_area(
    context: &mut UiContext,
    id: impl Into<UiId>,
    rect: Rect,
    state: &mut ScrollState,
    content: impl FnOnce(&mut UiContext, Rect) -> f32,
) {
    let id = id.into();
    let theme = context.theme;
    let margin = theme.gap;
    let overflow_of = |state: &ScrollState| (state.content_height - rect.height).max(0.0);

    let by_key = context.input.has_key_event();
    if let Some(focus) = context.state.focus
        && state.revealed != Some(focus)
        && let Some(entry) = context
            .state
            .previous_ring
            .iter()
            .find(|entry| entry.id == focus)
        && let Some(anchor) = entry.scroll
        && anchor.id == id
    {
        if by_key {
            let top = entry.rect.y - anchor.origin;
            reveal(state, top, entry.rect.height, rect.height, margin);
        }
        state.revealed = Some(focus);
    }

    let scroll_bar = ScrollBar::new(context, id.child("scroll bar"), rect, state.content_height);
    let bar_hot = scroll_bar.interact(context, &mut state.offset);
    if state.content_height > 0.0 {
        state.offset = context.snap(state.offset.clamp(0.0, overflow_of(state)));
    }

    let area_hovered = context.hovered(rect);
    let outer_capture = context.mouse_captured;
    let (x, y) = context.mouse();
    if !rect.contains(x, y) {
        context.mouse_captured = true;
    }
    let origin = rect.y - state.offset;
    let outer_anchor = context.scroll;
    context.scroll = Some(ScrollAnchor::inside(id, origin, rect, outer_anchor));
    let ring_start = context.state.ring.len();
    let wheel_before = std::mem::take(&mut context.state.wheel_taken);
    context.scene.push_clip(rect);
    let height = content(
        context,
        Rect::new(
            rect.x,
            origin,
            (rect.width - theme.padding).max(0.0),
            rect.height,
        ),
    );
    context.scene.pop_clip();
    context.scroll = outer_anchor;
    let child_scrolled = context.state.wheel_taken;
    context.state.wheel_taken = wheel_before || child_scrolled;
    context.mouse_captured = outer_capture;

    let before = state.offset;
    state.content_height = height.max(0.0);
    if area_hovered && !child_scrolled && context.input.wheel != 0.0 {
        state.offset -= context.input.wheel * WHEEL_ROWS * theme.row_height;
        context.state.wheel_taken = true;
    }
    let inside = context.state.focus.and_then(|focus| {
        context.state.ring[ring_start..]
            .iter()
            .find(|entry| entry.id == focus && entry.scroll.is_some_and(|anchor| anchor.id == id))
            .copied()
    });
    match inside {
        Some(entry) if context.state.focus_pressed => {
            let top = entry.rect.y - origin;
            let shown =
                top >= state.offset && top + entry.rect.height <= state.offset + rect.height;
            if !shown {
                reveal(state, top, entry.rect.height, rect.height, margin);
            }
            state.revealed = Some(entry.id);
        }
        Some(entry) if state.revealed != Some(entry.id) => {
            if by_key {
                reveal(
                    state,
                    entry.rect.y - origin,
                    entry.rect.height,
                    rect.height,
                    margin,
                );
            }
            state.revealed = Some(entry.id);
        }
        Some(_) => {}
        None => state.revealed = None,
    }
    let overflow = overflow_of(state);
    state.offset = context.snap(state.offset.clamp(0.0, overflow));
    if state.offset != before {
        context.request_pass();
    }

    let scroll_bar = ScrollBar::new(context, id.child("scroll bar"), rect, state.content_height);
    scroll_bar.draw(context, state.offset, bar_hot);
    if context.hovered(rect) {
        context.mouse_captured = true;
    }
}

/// Scrolls `state` so the area from `top` with `height` in the content lies
/// in a view of `view_height` with `margin` above and below it. An area
/// that does not fit the view with the margin above and below it shows its
/// top.
fn reveal(state: &mut ScrollState, top: f32, height: f32, view_height: f32, margin: f32) {
    if top - margin < state.offset || height + margin * 2.0 > view_height {
        state.offset = top - margin;
    } else if top + height + margin > state.offset + view_height {
        state.offset = top + height + margin - view_height;
    }
    state.offset = state.offset.max(0.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{
        ActionButton, Navigation, NavigationSet,
        tests::{Harness, pixel, srgb},
    };

    /// The view of the area: five rows of 40 units.
    const VIEW: Rect = Rect::new(20.0, 20.0, 400.0, 200.0);

    /// The distance between two rows of the content.
    const PITCH: f32 = 40.0;

    /// Draws an area with `rows` focusable rows of 32 units at the pitch
    /// [`PITCH`]. Returns the rectangles of the rows.
    fn draw(harness: &mut Harness, state: &mut ScrollState, rows: usize) -> Vec<Rect> {
        harness.frame(|context| {
            let mut rects = Vec::new();
            scroll_area(context, "form", VIEW, state, |context, content| {
                for index in 0..rows {
                    let rect = Rect::new(
                        content.x,
                        content.y + index as f32 * PITCH + 4.0,
                        content.width,
                        32.0,
                    );
                    context.focusable(UiId::from("row").indexed(index), rect, NavigationSet::EMPTY);
                    rects.push(rect);
                }
                rows as f32 * PITCH
            });
            rects
        })
    }

    #[test]
    fn navigation_scrolls_the_focused_widget_into_view() {
        let mut harness = Harness::new();
        let mut state = ScrollState::new();
        draw(&mut harness, &mut state, 20);
        let margin = harness.theme.gap;
        for _ in 0..8 {
            harness.navigate(Navigation::Down);
            draw(&mut harness, &mut state, 20);
        }
        assert_eq!(harness.state.focused(), Some(UiId::from("row").indexed(7)));
        let rects = draw(&mut harness, &mut state, 20);
        let focused = rects[7];
        assert!(focused.y >= VIEW.y && focused.bottom() + margin <= VIEW.bottom() + 0.5);
        assert_eq!(
            state.offset,
            7.0 * PITCH + 4.0 + 32.0 + margin - VIEW.height
        );
        harness.navigate(Navigation::Home);
        draw(&mut harness, &mut state, 20);
        for _ in 0..7 {
            harness.navigate(Navigation::Up);
            draw(&mut harness, &mut state, 20);
        }
        assert_eq!(harness.state.focused(), Some(UiId::from("row").indexed(0)));
        assert_eq!(state.offset, 0.0);
    }

    #[test]
    fn a_focus_that_a_shortcut_sets_scrolls_and_asks_for_a_pass() {
        let mut harness = Harness::new();
        let mut state = ScrollState::new();
        draw(&mut harness, &mut state, 20);
        let target = UiId::from("row").indexed(15);
        harness.input.action = Some(ActionButton::Y);
        harness.frame(|context| {
            scroll_area(context, "form", VIEW, &mut state, |context, content| {
                context.set_focus(target);
                for index in 0..20 {
                    let rect = Rect::new(
                        content.x,
                        content.y + index as f32 * PITCH + 4.0,
                        content.width,
                        32.0,
                    );
                    context.focusable(UiId::from("row").indexed(index), rect, NavigationSet::EMPTY);
                }
                20.0 * PITCH
            });
        });
        assert!(harness.state.take_pass_request());
        let rects = draw(&mut harness, &mut state, 20);
        assert!(rects[15].y >= VIEW.y && rects[15].bottom() <= VIEW.bottom());
        assert!(!harness.state.take_pass_request());
    }

    #[test]
    fn the_wheel_scrolls_inside_the_content_height() {
        let mut harness = Harness::new();
        let mut state = ScrollState::new();
        draw(&mut harness, &mut state, 20);
        harness.mouse(100.0, 100.0);
        harness.input.wheel = -1.0;
        draw(&mut harness, &mut state, 20);
        assert_eq!(state.offset, WHEEL_ROWS * harness.theme.row_height);
        harness.input.wheel = -100.0;
        draw(&mut harness, &mut state, 20);
        assert_eq!(state.offset, 20.0 * PITCH - VIEW.height);
        harness.mouse(600.0, 100.0);
        harness.input.wheel = 10.0;
        draw(&mut harness, &mut state, 20);
        assert_eq!(state.offset, 20.0 * PITCH - VIEW.height);
    }

    #[test]
    fn the_content_outside_the_view_ignores_the_pointer() {
        let mut harness = Harness::new();
        let mut state = ScrollState::new();
        draw(&mut harness, &mut state, 20);
        harness.mouse(100.0, VIEW.bottom() + 20.0);
        let hovered = harness.frame(|context| {
            let mut hovered = false;
            scroll_area(context, "form", VIEW, &mut state, |context, content| {
                let below = Rect::new(content.x, content.y + 200.0, content.width, 60.0);
                hovered = context.hovered(below);
                800.0
            });
            hovered
        });
        assert!(!hovered);
    }

    #[test]
    fn a_short_content_does_not_scroll() {
        let mut harness = Harness::new();
        let mut state = ScrollState::new();
        draw(&mut harness, &mut state, 3);
        harness.mouse(100.0, 100.0);
        harness.input.wheel = -5.0;
        draw(&mut harness, &mut state, 3);
        assert_eq!(state.offset, 0.0);
        let thumbs = harness
            .scene
            .instances()
            .iter()
            .filter(|instance| instance.color == harness.theme.colors.scroll_thumb.to_linear())
            .count();
        assert_eq!(thumbs, 0);
    }

    #[test]
    fn a_press_on_a_partly_visible_row_does_not_scroll_and_clicks() {
        let mut harness = Harness::new();
        let mut state = ScrollState::new();
        let draw = |harness: &mut Harness, state: &mut ScrollState| {
            harness.frame(|context| {
                let mut clicked = false;
                scroll_area(context, "form", VIEW, state, |context, content| {
                    for index in 0..10 {
                        let rect = Rect::new(
                            content.x,
                            content.y + index as f32 * PITCH + 4.0,
                            200.0,
                            32.0,
                        );
                        let id = UiId::from("row").indexed(index);
                        context.focusable(id, rect, NavigationSet::EMPTY);
                        if context.button_behavior(id, rect).clicked && index == 5 {
                            clicked = true;
                        }
                    }
                    10.0 * PITCH
                });
                clicked
            })
        };
        draw(&mut harness, &mut state);
        state.offset = 10.0;
        draw(&mut harness, &mut state);
        harness.press(100.0, VIEW.bottom() - 2.0);
        draw(&mut harness, &mut state);
        assert_eq!(state.offset, 10.0);
        harness.state.take_pass_request();
        draw(&mut harness, &mut state);
        assert_eq!(state.offset, 10.0);
        assert!(!harness.state.take_pass_request());
        harness.release();
        assert!(draw(&mut harness, &mut state));
        assert_eq!(harness.state.focused(), Some(UiId::from("row").indexed(5)));
        assert_eq!(state.offset, 10.0);
    }

    #[test]
    fn a_drag_of_the_scroll_bar_scrolls_the_area() {
        let mut harness = Harness::new();
        let mut state = ScrollState::new();
        draw(&mut harness, &mut state, 20);
        let bar_x = VIEW.right() - 3.0;
        harness.press(bar_x, VIEW.y + 10.0);
        draw(&mut harness, &mut state, 20);
        harness.mouse(bar_x, VIEW.y + 1000.0);
        draw(&mut harness, &mut state, 20);
        assert_eq!(state.offset, 20.0 * PITCH - VIEW.height);
        harness.release();
        draw(&mut harness, &mut state, 20);
        assert_eq!(harness.state.focused(), None);
    }

    #[test]
    fn a_nested_list_keeps_the_wheel() {
        let mut harness = Harness::new();
        let mut state = ScrollState::new();
        let mut list = crate::ui::ListState::new();
        let draw =
            |harness: &mut Harness, state: &mut ScrollState, list: &mut crate::ui::ListState| {
                harness.frame(|context| {
                    scroll_area(context, "form", VIEW, state, |context, content| {
                        crate::ui::list(
                            context,
                            "media",
                            Rect::new(content.x, content.y, 200.0, 100.0),
                            30,
                            list,
                            |_, _| {},
                        );
                        800.0
                    });
                });
            };
        draw(&mut harness, &mut state, &mut list);
        harness.mouse(VIEW.x + 50.0, VIEW.y + 50.0);
        harness.input.wheel = -1.0;
        draw(&mut harness, &mut state, &mut list);
        assert_eq!(state.offset, 0.0);
        assert!(list.scroll > 0.0);
        harness.mouse(VIEW.x + 300.0, VIEW.y + 150.0);
        harness.input.wheel = -1.0;
        draw(&mut harness, &mut state, &mut list);
        assert_eq!(state.offset, WHEEL_ROWS * harness.theme.row_height);
    }

    #[test]
    fn the_area_clips_its_content_and_draws_the_thumb() {
        let mut harness = Harness::new();
        let mut state = ScrollState::new();
        for _ in 0..2 {
            harness.frame(|context| {
                context.fill(context.bounds(), context.theme.colors.panel);
                scroll_area(context, "form", VIEW, &mut state, |context, content| {
                    context.fill(
                        Rect::new(content.x, content.y, content.width, 800.0),
                        crate::Color::WHITE,
                    );
                    800.0
                });
            });
        }
        let pixels = harness.render();
        let width = harness.viewport.width;
        let theme = &harness.theme;
        assert_eq!(
            pixel(&pixels, width, VIEW.x + 10.0, VIEW.y + 10.0),
            srgb(crate::Color::WHITE)
        );
        assert_eq!(
            pixel(&pixels, width, VIEW.x + 10.0, VIEW.bottom() + 2.0),
            srgb(theme.colors.panel)
        );
        let thumb = pixel(&pixels, width, VIEW.right() - 4.0, VIEW.y + 20.0);
        assert_ne!(thumb, srgb(theme.colors.panel));
    }

    /// Verifies that Down from a tall row of a short area takes the next row below the view,
    /// that the area scrolls the next row into view, and that the focus leaves the area for
    /// the button below the area from the last row and for the button above the area from
    /// the first row.
    #[test]
    fn the_arrows_walk_the_rows_of_a_short_area_before_they_leave_it() {
        /// The view of the area.
        const SHORT_VIEW: Rect = Rect::new(20.0, 60.0, 400.0, 100.0);
        /// The heights of the rows. The second row is taller than the view, and its bottom lies
        /// between the top and the bottom of the button below the area.
        const HEIGHTS: [f32; 4] = [32.0, 160.0, 32.0, 32.0];
        let mut harness = Harness::new();
        let mut state = ScrollState::new();
        let header = UiId::from("header");
        let footer = UiId::from("footer");
        let row = UiId::from("row");
        let draw = |harness: &mut Harness, state: &mut ScrollState| {
            harness.frame(|context| {
                context.focusable(
                    header,
                    Rect::new(20.0, 10.0, 100.0, 30.0),
                    NavigationSet::EMPTY,
                );
                let mut rects = [Rect::new(0.0, 0.0, 0.0, 0.0); HEIGHTS.len()];
                scroll_area(context, "form", SHORT_VIEW, state, |context, content| {
                    let mut y = content.y;
                    for (index, height) in HEIGHTS.iter().enumerate() {
                        let rect = Rect::new(content.x, y + 4.0, content.width, *height);
                        context.focusable(row.indexed(index), rect, NavigationSet::EMPTY);
                        rects[index] = rect;
                        y += height + 8.0;
                    }
                    y - content.y
                });
                context.focusable(
                    footer,
                    Rect::new(320.0, 200.0, 100.0, 30.0),
                    NavigationSet::EMPTY,
                );
                rects
            })
        };
        draw(&mut harness, &mut state);
        harness.state.set_focus(row.indexed(0));
        draw(&mut harness, &mut state);
        for index in 1..HEIGHTS.len() {
            harness.navigate(Navigation::Down);
            draw(&mut harness, &mut state);
            assert_eq!(harness.state.focused(), Some(row.indexed(index)));
            let rects = draw(&mut harness, &mut state);
            let focused = rects[index];
            assert!(
                focused.y >= SHORT_VIEW.y && focused.y < SHORT_VIEW.bottom(),
                "row {index} at {focused:?}"
            );
        }
        harness.navigate(Navigation::Down);
        draw(&mut harness, &mut state);
        assert_eq!(harness.state.focused(), Some(footer));
        harness.navigate(Navigation::Up);
        draw(&mut harness, &mut state);
        assert_eq!(
            harness.state.focused(),
            Some(row.indexed(HEIGHTS.len() - 1))
        );
        for index in (0..HEIGHTS.len() - 1).rev() {
            harness.navigate(Navigation::Up);
            draw(&mut harness, &mut state);
            assert_eq!(harness.state.focused(), Some(row.indexed(index)));
            let rects = draw(&mut harness, &mut state);
            assert!(rects[index].bottom() > SHORT_VIEW.y, "row {index}");
        }
        harness.navigate(Navigation::Up);
        draw(&mut harness, &mut state);
        assert_eq!(harness.state.focused(), Some(header));
    }

    /// Draws an area of 20 rows of 32 units at the pitch [`PITCH`]. A press on the area of a
    /// row gives the control of the row the focus. The control spans the height of the row.
    /// Returns the rectangles of the controls.
    fn draw_pressable(harness: &mut Harness, state: &mut ScrollState) -> Vec<Rect> {
        harness.frame(|context| {
            let mut controls = Vec::new();
            scroll_area(context, "form", VIEW, state, |context, content| {
                for index in 0..20 {
                    let y = content.y + index as f32 * PITCH + 4.0;
                    let control = Rect::new(content.x + 200.0, y, 150.0, 32.0);
                    let id = UiId::from("control").indexed(index);
                    context.focusable(id, control, NavigationSet::EMPTY);
                    context.focus_on_press(id, Rect::new(content.x, y, content.width, 32.0));
                    controls.push(control);
                }
                20.0 * PITCH
            });
            controls
        })
    }

    /// Verifies that a press that gives a control the focus through
    /// [`UiContext::focus_on_press`] keeps the offset while the whole control shows, also with
    /// the control nearer to the top or the bottom edge of the view than the gap of the theme.
    #[test]
    fn a_press_on_a_fully_visible_row_keeps_the_offset() {
        /// The offset and the row of each press: the top of row 5 one unit below the top of
        /// the view and the bottom of row 9 one unit above the bottom of the view.
        const PRESSES: [(f32, usize); 2] = [(203.0, 5), (197.0, 9)];
        for (offset, index) in PRESSES {
            let mut harness = Harness::new();
            let mut state = ScrollState::new();
            draw_pressable(&mut harness, &mut state);
            state.offset = offset;
            let controls = draw_pressable(&mut harness, &mut state);
            let control = controls[index];
            assert!(
                control.y > VIEW.y
                    && control.y < VIEW.y + harness.theme.gap
                    && control.bottom() <= VIEW.bottom()
                    || control.bottom() < VIEW.bottom()
                        && control.bottom() > VIEW.bottom() - harness.theme.gap
                        && control.y >= VIEW.y,
                "{control:?}"
            );
            harness.press(VIEW.x + 10.0, control.y + 16.0);
            draw_pressable(&mut harness, &mut state);
            harness.release();
            draw_pressable(&mut harness, &mut state);
            draw_pressable(&mut harness, &mut state);
            assert_eq!(
                harness.state.focused(),
                Some(UiId::from("control").indexed(index))
            );
            assert_eq!(state.offset, offset, "row {index}");
        }
    }

    /// Verifies that a press on the visible lower part of a tall row scrolls the control of the
    /// row into view when the control lies above the view and already holds the focus.
    #[test]
    fn a_press_reveals_a_hidden_control_that_holds_the_focus() {
        /// The height of the tall row, taller than the view.
        const TALL: f32 = 300.0;
        let mut harness = Harness::new();
        let mut state = ScrollState::new();
        let control_id = UiId::from("control");
        let draw = |harness: &mut Harness, state: &mut ScrollState| {
            harness.frame(|context| {
                let mut control = Rect::new(0.0, 0.0, 0.0, 0.0);
                scroll_area(context, "form", VIEW, state, |context, content| {
                    let first = Rect::new(content.x, content.y + 4.0, 150.0, 32.0);
                    context.focusable(UiId::from("first"), first, NavigationSet::EMPTY);
                    let row = Rect::new(content.x, content.y + PITCH + 4.0, content.width, TALL);
                    control = Rect::new(row.x + 200.0, row.y, 150.0, 32.0);
                    context.focusable(control_id, control, NavigationSet::EMPTY);
                    context.focus_on_press(control_id, row);
                    PITCH + TALL + PITCH
                });
                control
            })
        };
        draw(&mut harness, &mut state);
        harness.state.set_focus(control_id);
        draw(&mut harness, &mut state);
        draw(&mut harness, &mut state);
        state.offset = 100.0;
        let control = draw(&mut harness, &mut state);
        assert!(control.bottom() < VIEW.y, "{control:?}");
        assert_eq!(harness.state.focused(), Some(control_id));
        harness.press(VIEW.x + 10.0, VIEW.y + 50.0);
        draw(&mut harness, &mut state);
        harness.release();
        draw(&mut harness, &mut state);
        let control = draw(&mut harness, &mut state);
        assert_eq!(harness.state.focused(), Some(control_id));
        assert!(
            control.y >= VIEW.y && control.bottom() <= VIEW.bottom(),
            "{control:?}"
        );
    }

    /// Verifies that Down from a widget of an outer area enters a nested area at its first
    /// widget, walks the nested area, and leaves it to the next widgets of the outer area,
    /// also to one below the view of the outer area, before it leaves the outer area for the
    /// button below. Up walks back the same way to the button above the outer area. The
    /// focused widget of the nested area shows inside its view after every step.
    #[test]
    fn the_arrows_walk_a_nested_area_inside_its_outer_area() {
        /// The view of the outer area.
        const OUTER_VIEW: Rect = Rect::new(20.0, 60.0, 400.0, 160.0);
        /// The top of the view of the nested area in the content of the outer area, and its
        /// height.
        const NESTED: (f32, f32) = (44.0, 100.0);
        /// The number of rows of the nested area.
        const ROWS: usize = 5;
        let mut harness = Harness::new();
        let mut outer = ScrollState::new();
        let mut nested = ScrollState::new();
        let header = UiId::from("header");
        let top = UiId::from("top");
        let row = UiId::from("row");
        let after = UiId::from("after");
        let last = UiId::from("last");
        let footer = UiId::from("footer");
        let draw = |harness: &mut Harness, outer: &mut ScrollState, nested: &mut ScrollState| {
            harness.frame(|context| {
                context.focusable(
                    header,
                    Rect::new(20.0, 10.0, 100.0, 30.0),
                    NavigationSet::EMPTY,
                );
                let mut view = Rect::new(0.0, 0.0, 0.0, 0.0);
                let mut rows = [view; ROWS];
                scroll_area(context, "outer", OUTER_VIEW, outer, |context, content| {
                    let widget =
                        |offset: f32| Rect::new(content.x, content.y + offset, 300.0, 32.0);
                    context.focusable(top, widget(4.0), NavigationSet::EMPTY);
                    view = Rect::new(content.x, content.y + NESTED.0, content.width, NESTED.1);
                    scroll_area(context, "nested", view, nested, |context, inner| {
                        for (index, rect) in rows.iter_mut().enumerate() {
                            *rect = Rect::new(
                                inner.x,
                                inner.y + index as f32 * PITCH + 4.0,
                                inner.width,
                                32.0,
                            );
                            context.focusable(row.indexed(index), *rect, NavigationSet::EMPTY);
                        }
                        ROWS as f32 * PITCH
                    });
                    context.focusable(after, widget(152.0), NavigationSet::EMPTY);
                    context.focusable(last, widget(192.0), NavigationSet::EMPTY);
                    232.0
                });
                context.focusable(
                    footer,
                    Rect::new(320.0, 230.0, 100.0, 30.0),
                    NavigationSet::EMPTY,
                );
                (view, rows)
            })
        };
        draw(&mut harness, &mut outer, &mut nested);
        harness.state.set_focus(top);
        draw(&mut harness, &mut outer, &mut nested);
        let mut step = |harness: &mut Harness, navigation: Navigation, expected: UiId| {
            harness.navigate(navigation);
            draw(harness, &mut outer, &mut nested);
            assert_eq!(harness.state.focused(), Some(expected), "{navigation:?}");
            let (view, rows) = draw(harness, &mut outer, &mut nested);
            if let Some(index) = (0..ROWS).find(|&index| expected == row.indexed(index)) {
                let rect = rows[index];
                assert!(
                    rect.y >= view.y && rect.bottom() <= view.bottom(),
                    "row {index} at {rect:?} in {view:?}"
                );
            }
        };
        for index in 0..ROWS {
            step(&mut harness, Navigation::Down, row.indexed(index));
        }
        step(&mut harness, Navigation::Down, after);
        step(&mut harness, Navigation::Down, last);
        step(&mut harness, Navigation::Down, footer);
        step(&mut harness, Navigation::Up, last);
        step(&mut harness, Navigation::Up, after);
        for index in (0..ROWS).rev() {
            step(&mut harness, Navigation::Up, row.indexed(index));
        }
        step(&mut harness, Navigation::Up, top);
        step(&mut harness, Navigation::Up, header);
    }
}
