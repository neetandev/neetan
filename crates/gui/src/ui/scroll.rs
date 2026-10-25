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
/// below it. A press of the pointer keeps the offset. The pressed widget
/// receives its click.
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
    let outer_anchor = context.scroll.replace(ScrollAnchor { id, origin });
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
/// in a view of `view_height` with `margin` above and below it.
fn reveal(state: &mut ScrollState, top: f32, height: f32, view_height: f32, margin: f32) {
    if top - margin < state.offset {
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
}
