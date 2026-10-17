//! The scrolling list.

use super::{
    context::{Drag, UiContext, UiId},
    focus::{Navigation, NavigationSet},
    icons::Icon,
    label::{Align, Truncation},
};
use crate::{Rect, RectStyle};

/// The gap between the edge of a list and its rows.
pub(super) const LIST_PADDING: f32 = 4.0;

/// The rows one wheel step scrolls.
const WHEEL_ROWS: f32 = 3.0;

/// The smallest height of the scroll bar thumb.
const MINIMUM_THUMB: f32 = 24.0;

/// The width of the thumb inside the scroll bar while the pointer is away.
const THUMB_WIDTH: f32 = 4.0;

/// The width of the thumb inside the scroll bar under the pointer.
const THUMB_WIDTH_HOVER: f32 = 6.0;

/// The state of a [`list`] that outlives a pass.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ListState {
    /// The selected row.
    pub selected: usize,
    /// The scroll offset of the content in logical units.
    pub scroll: f32,
    /// Whether the next pass scrolls the selected row into view.
    reveal: bool,
    /// The row of the last press, for a double click.
    last_pressed: Option<usize>,
}

impl ListState {
    /// A list at its top with the first row selected.
    pub fn new() -> Self {
        Self::default()
    }

    /// Selects the row `index` and scrolls it into view in the next pass.
    pub fn select(&mut self, index: usize) {
        self.selected = index;
        self.reveal = true;
    }
}

/// A row of a [`list`] that the caller draws.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ListRow {
    /// The index of the row.
    pub index: usize,
    /// The area of the row in logical units. It can reach past the list.
    pub rect: Rect,
    /// Whether the row is selected.
    pub selected: bool,
    /// Whether the pointer is over the row.
    pub hovered: bool,
    /// Whether the list holds the focus.
    pub focused: bool,
}

/// What a [`list`] did in a pass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ListResponse {
    /// The row that a double click or confirm activated.
    pub activated: Option<usize>,
    /// Whether the selected row changed.
    pub selection_changed: bool,
}

/// A list of `rows` rows of the theme row height in `rect`, clipped to
/// `rect`. `state` holds the selection and the scroll offset between passes.
/// `draw_row` draws the content of each visible row over the selection fill.
///
/// A press selects a row and gives the list the focus. A double click or
/// confirm activates the selected row. A press on a row that the edge of the
/// list cuts scrolls the row into view. While the list holds the focus, up,
/// down, page up, page down, home and end select another row and scroll it
/// into view. Up on the first row and down on the last row let the focus
/// leave the list. The wheel scrolls the list, and so does a drag of the
/// scroll bar. A press on the scroll bar track centers the thumb under the
/// pointer and drags it.
pub fn list(
    context: &mut UiContext,
    id: impl Into<UiId>,
    rect: Rect,
    rows: usize,
    state: &mut ListState,
    mut draw_row: impl FnMut(&mut UiContext, &ListRow),
) -> ListResponse {
    let id = id.into();
    let theme = context.theme;
    let row_height = theme.row_height;
    let before = state.selected;
    let last = rows.saturating_sub(1);
    let view = rect.inset(theme.stroke);
    let page = (((view.height - LIST_PADDING * 2.0) / row_height).floor() as usize).max(2) - 1;
    let mut response = ListResponse::default();

    if let Some(navigation) = context.navigation(id) {
        state.selected = match navigation {
            Navigation::Up => state.selected.saturating_sub(1),
            Navigation::Down => state.selected + 1,
            Navigation::PageUp => state.selected.saturating_sub(page),
            Navigation::PageDown => state.selected + page,
            Navigation::Home => 0,
            Navigation::End => last,
            _ => state.selected,
        };
    }
    state.selected = state.selected.min(last);
    if rows > 0 && context.confirmed(id) {
        response.activated = Some(state.selected);
    }
    if state.selected != before {
        state.reveal = true;
    }

    let content = rows as f32 * row_height + LIST_PADDING * 2.0;
    let overflow = (content - view.height).max(0.0);
    let bar = Rect::new(
        view.right() - theme.scroll_bar_width,
        view.y,
        theme.scroll_bar_width,
        view.height,
    );
    let thumb_height = (view.height * view.height / content)
        .max(MINIMUM_THUMB)
        .min(view.height);
    let travel = (view.height - thumb_height).max(1.0);
    let bar_id = id.child("scroll bar");
    let list_hovered = context.hovered(rect);
    let mut bar_hot = false;
    if overflow > 0.0 {
        let dragging = context.is_active(bar_id);
        let pointer = context.button_behavior(bar_id, bar);
        let (_, mouse_y) = context.mouse();
        if pointer.pressed {
            let thumb_y = view.y + state.scroll / overflow * travel;
            let offset = if mouse_y >= thumb_y && mouse_y < thumb_y + thumb_height {
                mouse_y - thumb_y
            } else {
                thumb_height * 0.5
            };
            context.set_drag(Some(Drag { offset }));
        }
        if pointer.held {
            if let Some(drag) = context.drag() {
                state.scroll = (mouse_y - drag.offset - view.y) / travel * overflow;
            }
            context.mouse_captured = true;
        } else if dragging {
            context.set_drag(None);
        }
        bar_hot = pointer.hovered || pointer.held;
    }
    if list_hovered && context.input.wheel != 0.0 {
        state.scroll -= context.input.wheel * WHEEL_ROWS * row_height;
    }
    settle_scroll(context, state, rows, view.height, overflow);

    let row_width = view.width
        - LIST_PADDING * 2.0
        - if overflow > 0.0 {
            theme.scroll_bar_width - LIST_PADDING
        } else {
            0.0
        };
    let row_area = |scroll: f32, index: usize| {
        Rect::new(
            view.x + LIST_PADDING,
            view.y + LIST_PADDING + index as f32 * row_height - scroll,
            row_width,
            row_height,
        )
    };
    let visible_rows = |scroll: f32| {
        let first = ((scroll - LIST_PADDING) / row_height).floor().max(0.0) as usize;
        (first..rows).take_while(move |&index| row_area(scroll, index).y < view.bottom())
    };
    let scroll = state.scroll;
    for index in visible_rows(scroll) {
        let row = row_area(scroll, index);
        let top = row.y.max(view.y);
        let visible = Rect::new(row.x, top, row.width, row.bottom().min(view.bottom()) - top);
        if context.button_behavior(id.indexed(index), visible).pressed {
            if context.input.left_double && state.last_pressed == Some(index) {
                response.activated = Some(index);
            }
            state.last_pressed = Some(index);
            state.selected = index;
            state.reveal = true;
            context.set_focus(id);
        }
    }
    settle_scroll(context, state, rows, view.height, overflow);
    let captures = if rows > 0 {
        NavigationSet::of(&[
            Navigation::PageUp,
            Navigation::PageDown,
            Navigation::Home,
            Navigation::End,
        ])
        .with_if(state.selected > 0, Navigation::Up)
        .with_if(state.selected < last, Navigation::Down)
    } else {
        NavigationSet::EMPTY
    };
    context.focusable(id, rect, captures);

    context.rect(
        rect,
        &RectStyle::filled(theme.well, theme.radius).with_stroke(theme.stroke, theme.well_edge),
    );
    let focused = context.has_focus(id);
    context.scene.push_clip(view);
    for index in visible_rows(state.scroll) {
        let row = ListRow {
            index,
            rect: row_area(state.scroll, index),
            selected: index == state.selected,
            hovered: context.is_hot(id.indexed(index)),
            focused,
        };
        let fill = if row.selected {
            Some(if focused {
                theme.selection_focused
            } else {
                theme.selection
            })
        } else if row.hovered {
            Some(theme.row_hover)
        } else {
            None
        };
        if let Some(fill) = fill {
            context.scene.fill(row.rect, theme.radius - 2.0, fill);
        }
        draw_row(context, &row);
    }
    context.scene.pop_clip();

    if overflow > 0.0 {
        let width = if bar_hot {
            THUMB_WIDTH_HOVER
        } else {
            THUMB_WIDTH
        };
        let thumb = Rect::new(
            bar.right() - LIST_PADDING * 0.5 - width,
            view.y + state.scroll / overflow * travel,
            width,
            thumb_height,
        );
        let thumb = Rect::new(thumb.x, thumb.y + 2.0, thumb.width, thumb.height - 4.0);
        let color = if bar_hot {
            theme.scroll_thumb_hover
        } else {
            theme.scroll_thumb
        };
        context.scene.fill(thumb, width * 0.5, color);
    }
    if focused {
        context.focus_ring(rect, theme.radius);
    }
    if context.hovered(rect) {
        context.mouse_captured = true;
    }
    response.selection_changed = state.selected != before;
    response
}

/// Scrolls the selected row of `state` into a view of `view_height` when
/// the state asks for it, then limits the scroll offset to `overflow` and
/// rounds it to whole physical pixels.
fn settle_scroll(
    context: &UiContext,
    state: &mut ListState,
    rows: usize,
    view_height: f32,
    overflow: f32,
) {
    if state.reveal && rows > 0 {
        let row_height = context.theme.row_height;
        let top = LIST_PADDING + state.selected as f32 * row_height;
        if top - LIST_PADDING < state.scroll {
            state.scroll = top - LIST_PADDING;
        }
        if top + row_height + LIST_PADDING > state.scroll + view_height {
            state.scroll = top + row_height + LIST_PADDING - view_height;
        }
    }
    state.reveal = false;
    state.scroll = context.snap(state.scroll.clamp(0.0, overflow));
}

/// Draws the standard content of `row`: an optional `icon`, `text` cut in
/// the middle when it does not fit, and an optional `marker` icon in the
/// marker color at the end. Returns whether the text was cut.
pub fn list_row(
    context: &mut UiContext,
    row: &ListRow,
    icon: Option<Icon>,
    text: &str,
    marker: Option<Icon>,
) -> bool {
    let theme = context.theme;
    let mut area = Rect::new(
        row.rect.x + theme.gap,
        row.rect.y,
        row.rect.width - theme.gap * 2.0,
        row.rect.height,
    );
    let icon_color = if row.selected {
        theme.text
    } else {
        theme.text_dim
    };
    if let Some(icon) = icon {
        let (slot, rest) = area.cut_left(theme.icon_size);
        context.icon(slot, icon, theme.icon_size, icon_color);
        area = rest.cut_left(theme.gap).1;
    }
    if let Some(marker) = marker {
        let (slot, rest) = area.cut_right(theme.icon_size);
        context.icon(slot, marker, theme.icon_size, theme.marker);
        area = rest.cut_right(theme.gap).1;
    }
    let style = theme.body();
    context.text_in(area, &style, text, Align::Start, Truncation::Middle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::tests::Harness;

    /// The area of the list: 10 rows of 30 units fit, and 4 units of padding
    /// at the top and the bottom.
    const LIST: Rect = Rect::new(20.0, 20.0, 300.0, 310.0);

    /// Draws a list of `rows` rows with `state` and returns its response.
    fn draw(harness: &mut Harness, rows: usize, state: &mut ListState) -> ListResponse {
        harness.frame(|context| {
            list(context, "files", LIST, rows, state, |context, row| {
                list_row(context, row, Some(Icon::Disk), "image.d88", None);
            })
        })
    }

    /// The center of the row `index` of [`LIST`] at the scroll offset
    /// `scroll` with the row height of `theme`.
    fn row_center(theme: &crate::ui::Theme, index: usize, scroll: f32) -> (f32, f32) {
        let row_height = theme.row_height;
        (
            LIST.center().0,
            LIST.y + theme.stroke + LIST_PADDING + (index as f32 + 0.5) * row_height - scroll,
        )
    }

    /// Clicks at `point` in two passes.
    fn click(
        harness: &mut Harness,
        point: (f32, f32),
        rows: usize,
        state: &mut ListState,
    ) -> ListResponse {
        harness.press(point.0, point.1);
        let pressed = draw(harness, rows, state);
        harness.release();
        let released = draw(harness, rows, state);
        ListResponse {
            activated: pressed.activated.or(released.activated),
            selection_changed: pressed.selection_changed || released.selection_changed,
        }
    }

    /// Clicks the row `index` of [`LIST`] at the scroll offset `scroll` in
    /// two passes.
    fn click_row(
        harness: &mut Harness,
        index: usize,
        scroll: f32,
        rows: usize,
        state: &mut ListState,
    ) -> ListResponse {
        let point = row_center(&harness.theme, index, scroll);
        click(harness, point, rows, state)
    }

    #[test]
    fn a_click_selects_a_row_and_focuses_the_list() {
        let mut harness = Harness::new();
        let mut state = ListState::new();
        draw(&mut harness, 5, &mut state);
        let response = click_row(&mut harness, 3, 0.0, 5, &mut state);
        assert_eq!(state.selected, 3);
        assert!(response.selection_changed);
        assert_eq!(response.activated, None);
        assert_eq!(harness.state.focused(), Some(UiId::from("files")));
    }

    #[test]
    fn a_double_click_activates_the_row() {
        let mut harness = Harness::new();
        let mut state = ListState::new();
        draw(&mut harness, 5, &mut state);
        click_row(&mut harness, 2, 0.0, 5, &mut state);
        harness.input.left_double = true;
        let response = click_row(&mut harness, 2, 0.0, 5, &mut state);
        assert_eq!(response.activated, Some(2));
        harness.input.left_double = true;
        let response = click_row(&mut harness, 4, 0.0, 5, &mut state);
        assert_eq!(response.activated, None);
        assert_eq!(state.selected, 4);
    }

    #[test]
    fn a_double_click_needs_both_presses_on_the_same_row() {
        let mut harness = Harness::new();
        let mut state = ListState::new();
        state.select(2);
        draw(&mut harness, 5, &mut state);
        click_row(&mut harness, 1, 0.0, 5, &mut state);
        state.selected = 2;
        harness.input.left_double = true;
        let response = click_row(&mut harness, 2, 0.0, 5, &mut state);
        assert_eq!(response.activated, None);
    }

    #[test]
    fn a_press_on_a_cut_row_scrolls_it_into_view() {
        let mut harness = Harness::new();
        let mut state = ListState::new();
        draw(&mut harness, 40, &mut state);
        let view = LIST.inset(harness.theme.stroke);
        let point = (LIST.center().0, view.bottom() - 3.0);
        click(&mut harness, point, 40, &mut state);
        assert_eq!(state.selected, 10);
        let bottom = LIST_PADDING + 11.0 * harness.theme.row_height + LIST_PADDING;
        assert_eq!(state.scroll, bottom - view.height);
    }

    #[test]
    fn keys_select_rows_and_keep_the_selection_visible() {
        let mut harness = Harness::new();
        let mut state = ListState::new();
        harness.state.set_focus(UiId::from("files"));
        draw(&mut harness, 40, &mut state);
        let mut changes = 0;
        for _ in 0..12 {
            harness.navigate(Navigation::Down);
            changes += usize::from(draw(&mut harness, 40, &mut state).selection_changed);
        }
        assert_eq!(changes, 12);
        assert_eq!(state.selected, 12);
        let view = LIST.inset(1.0);
        let top = LIST_PADDING + 12.0 * 30.0;
        assert!(top >= state.scroll);
        assert!(top + 30.0 <= state.scroll + view.height);
        assert_eq!(state.scroll, top + 30.0 + LIST_PADDING - view.height);

        harness.navigate(Navigation::PageDown);
        draw(&mut harness, 40, &mut state);
        assert_eq!(state.selected, 21);
        harness.navigate(Navigation::End);
        draw(&mut harness, 40, &mut state);
        assert_eq!(state.selected, 39);
        assert_eq!(state.scroll, 40.0 * 30.0 + 8.0 - view.height);
        harness.navigate(Navigation::Home);
        draw(&mut harness, 40, &mut state);
        assert_eq!((state.selected, state.scroll), (0, 0.0));
        harness.navigate(Navigation::PageUp);
        draw(&mut harness, 40, &mut state);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn confirm_activates_the_selected_row() {
        let mut harness = Harness::new();
        let mut state = ListState::new();
        state.select(3);
        harness.state.set_focus(UiId::from("files"));
        harness.input.confirm = true;
        assert_eq!(draw(&mut harness, 5, &mut state).activated, Some(3));
        harness.input.confirm = true;
        assert_eq!(draw(&mut harness, 0, &mut state).activated, None);
    }

    #[test]
    fn the_focus_leaves_at_the_first_and_the_last_row() {
        let mut harness = Harness::new();
        let mut state = ListState::new();
        let above = UiId::from("filter");
        let below = UiId::from("insert");
        let frame = |harness: &mut Harness, state: &mut ListState| {
            harness.frame(|context| {
                context.focusable(
                    above,
                    Rect::new(20.0, 0.0, 300.0, 16.0),
                    NavigationSet::EMPTY,
                );
                list(context, "files", LIST, 3, state, |_, _| {});
                context.focusable(
                    below,
                    Rect::new(20.0, 340.0, 300.0, 30.0),
                    NavigationSet::EMPTY,
                );
            });
        };
        harness.state.set_focus(UiId::from("files"));
        frame(&mut harness, &mut state);
        harness.navigate(Navigation::Up);
        frame(&mut harness, &mut state);
        assert_eq!(harness.state.focused(), Some(above));
        harness.navigate(Navigation::Down);
        frame(&mut harness, &mut state);
        harness.navigate(Navigation::Down);
        frame(&mut harness, &mut state);
        assert_eq!(state.selected, 1);
        harness.navigate(Navigation::Down);
        frame(&mut harness, &mut state);
        harness.navigate(Navigation::Down);
        frame(&mut harness, &mut state);
        assert_eq!(state.selected, 2);
        assert_eq!(harness.state.focused(), Some(below));
    }

    #[test]
    fn the_wheel_scrolls_without_changing_the_selection() {
        let mut harness = Harness::new();
        let mut state = ListState::new();
        harness.mouse(100.0, 100.0);
        harness.input.wheel = -1.0;
        draw(&mut harness, 40, &mut state);
        assert_eq!(state.scroll, WHEEL_ROWS * harness.theme.row_height);
        assert_eq!(state.selected, 0);
        harness.input.wheel = -100.0;
        draw(&mut harness, 40, &mut state);
        let overflow = 40.0 * 30.0 + 8.0 - LIST.inset(1.0).height;
        assert_eq!(state.scroll, overflow);
        harness.mouse(500.0, 100.0);
        harness.input.wheel = 1.0;
        draw(&mut harness, 40, &mut state);
        assert_eq!(state.scroll, overflow);
        let response = click_row(&mut harness, 30, overflow, 40, &mut state);
        assert_eq!(state.selected, 30);
        assert!(response.selection_changed);
    }

    #[test]
    fn up_after_a_click_on_the_first_row_leaves_the_list() {
        let mut harness = Harness::new();
        let mut state = ListState::new();
        state.select(3);
        let above = UiId::from("filter");
        let frame = |harness: &mut Harness, state: &mut ListState| {
            harness.frame(|context| {
                context.focusable(
                    above,
                    Rect::new(20.0, 0.0, 300.0, 16.0),
                    NavigationSet::EMPTY,
                );
                list(context, "files", LIST, 5, state, |_, _| {});
            });
        };
        frame(&mut harness, &mut state);
        let point = row_center(&harness.theme, 0, 0.0);
        harness.press(point.0, point.1);
        frame(&mut harness, &mut state);
        harness.release();
        harness.navigate(Navigation::Up);
        frame(&mut harness, &mut state);
        assert_eq!(state.selected, 0);
        assert_eq!(harness.state.focused(), Some(above));
    }

    #[test]
    fn the_wheel_scrolls_over_the_scroll_bar() {
        let mut harness = Harness::new();
        let mut state = ListState::new();
        draw(&mut harness, 40, &mut state);
        let view = LIST.inset(harness.theme.stroke);
        harness.mouse(view.right() - 2.0, view.y + 100.0);
        harness.input.wheel = -1.0;
        draw(&mut harness, 40, &mut state);
        assert_eq!(state.scroll, WHEEL_ROWS * harness.theme.row_height);
    }

    #[test]
    fn a_drag_of_the_scroll_bar_scrolls_the_list() {
        let mut harness = Harness::new();
        let mut state = ListState::new();
        let view = LIST.inset(1.0);
        let overflow = 40.0 * 30.0 + 8.0 - view.height;
        let thumb_height = (view.height * view.height / (40.0 * 30.0 + 8.0)).max(MINIMUM_THUMB);
        let travel = view.height - thumb_height;
        let bar_x = view.right() - 4.0;
        draw(&mut harness, 40, &mut state);
        harness.press(bar_x, view.y + 10.0);
        draw(&mut harness, 40, &mut state);
        assert_eq!(state.scroll, 0.0);
        harness.mouse(bar_x, view.y + 10.0 + travel * 0.5);
        draw(&mut harness, 40, &mut state);
        assert!(
            (state.scroll - overflow * 0.5).abs() <= 0.5,
            "{}",
            state.scroll
        );
        harness.mouse(bar_x + 200.0, view.y + 1000.0);
        draw(&mut harness, 40, &mut state);
        assert_eq!(state.scroll, overflow);
        harness.release();
        draw(&mut harness, 40, &mut state);
        assert_eq!(state.selected, 0);
        assert_eq!(harness.state.focused(), None);

        harness.press(bar_x, view.y + view.height * 0.5);
        draw(&mut harness, 40, &mut state);
        let expected = (view.height * 0.5 - thumb_height * 0.5) / travel * overflow;
        assert!((state.scroll - expected).abs() <= 0.5);
        harness.release();
        draw(&mut harness, 40, &mut state);
    }

    #[test]
    fn a_shrinking_list_keeps_a_valid_selection() {
        let mut harness = Harness::new();
        let mut state = ListState::new();
        state.select(30);
        draw(&mut harness, 40, &mut state);
        assert!(state.scroll > 0.0);
        let response = draw(&mut harness, 4, &mut state);
        assert_eq!(state.selected, 3);
        assert!(response.selection_changed);
        assert_eq!(state.scroll, 0.0);
        draw(&mut harness, 0, &mut state);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn the_list_works_at_twice_the_scale() {
        let mut harness = Harness::with_viewport(crate::ui::Viewport::new(800, 800, 2.0));
        let mut state = ListState::new();
        draw(&mut harness, 5, &mut state);
        let (x, y) = row_center(&harness.theme, 3, 0.0);
        click(&mut harness, (x * 2.0, y * 2.0), 5, &mut state);
        assert_eq!(state.selected, 3);
    }
}
