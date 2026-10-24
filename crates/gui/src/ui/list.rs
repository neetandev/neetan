//! The scrolling list.

use super::{
    context::{UiContext, UiId},
    focus::{Navigation, NavigationSet},
    icons::Icon,
    label::{Align, Truncation},
    scroll_bar::ScrollBar,
};
use crate::{Rect, RectStyle};

/// The gap between the edge of a list and its rows.
pub(super) const LIST_PADDING: f32 = 4.0;

/// The rows one wheel step scrolls.
const WHEEL_ROWS: f32 = 3.0;

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

/// The kind of a row of a [`List`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RowKind {
    /// A row that the user selects.
    #[default]
    Normal,
    /// A row that draws dimmed and ignores the input. The selection skips
    /// it.
    Disabled,
    /// A caption that groups the rows below it. The selection skips it.
    Caption,
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
    /// The kind of the row.
    pub kind: RowKind,
    /// Whether the row and its list react to input. A disabled row or a row
    /// of a disabled list draws dimmed.
    pub enabled: bool,
}

/// What a [`list`] did in a pass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ListResponse {
    /// The row that a double click or confirm activated.
    pub activated: Option<usize>,
    /// The row that a click selected: a press and a release over the row.
    pub clicked: Option<usize>,
    /// Whether the selected row changed.
    pub selection_changed: bool,
}

/// The vertical positions of the rows of a list: rows of one height with an
/// optional separator after one row.
#[derive(Clone, Copy, Debug)]
struct RowLayout {
    /// The number of rows.
    rows: usize,
    /// The height of a row.
    row_height: f32,
    /// The row that the separator follows.
    separator: Option<usize>,
    /// The height of the separator with its gaps.
    separator_height: f32,
}

impl RowLayout {
    /// The top of the row `index` below the top padding of the list.
    fn top(&self, index: usize) -> f32 {
        let below = self.separator.is_some_and(|separator| index > separator);
        index as f32 * self.row_height + if below { self.separator_height } else { 0.0 }
    }

    /// The height of the rows and the separator.
    fn height(&self) -> f32 {
        self.top(self.rows)
    }

    /// The first row whose bottom lies below `offset`.
    fn first_below(&self, offset: f32) -> usize {
        let estimate = (offset / self.row_height).floor().max(0.0) as usize;
        let mut index = estimate.saturating_sub(1).min(self.rows);
        while index < self.rows && self.top(index) + self.row_height <= offset {
            index += 1;
        }
        index
    }
}

/// A list with options: row kinds, a disabled state, a separator and a
/// look without its own frame. [`list`] draws a list without options.
///
/// A press selects a row and gives the list the focus. A double click or
/// confirm activates the selected row. A press on a row that the edge of the
/// list cuts scrolls the row into view. While the list holds the focus, up,
/// down, page up, page down, home and end select another row and scroll it
/// into view. Up on the first row and down on the last row let the focus
/// leave the list. The wheel scrolls the list, and so does a drag of the
/// scroll bar. A press on the scroll bar track centers the thumb under the
/// pointer and drags it.
#[derive(Clone, Copy, Debug)]
pub struct List<'a> {
    /// The id of the list.
    id: UiId,
    /// The number of rows.
    rows: usize,
    /// The kinds of the rows. Rows past its end are normal rows.
    kinds: &'a [RowKind],
    /// Whether the list reacts to input.
    enabled: bool,
    /// Whether the list draws its well, its edge and its focus ring.
    framed: bool,
    /// The row that a separator follows.
    separator: Option<usize>,
}

impl<'a> List<'a> {
    /// An enabled, framed list of `rows` normal rows.
    pub fn new(id: impl Into<UiId>, rows: usize) -> Self {
        Self {
            id: id.into(),
            rows,
            kinds: &[],
            enabled: true,
            framed: true,
            separator: None,
        }
    }

    /// The same list with the row kinds `kinds`. Rows past the end of
    /// `kinds` are normal rows.
    pub fn kinds(self, kinds: &'a [RowKind]) -> Self {
        Self { kinds, ..self }
    }

    /// The same list, disabled unless `enabled`. A disabled list draws its
    /// edge and its rows dimmed, ignores input and leaves the focus ring.
    pub fn enabled(self, enabled: bool) -> Self {
        Self { enabled, ..self }
    }

    /// The same list with or without its well, its edge and its focus ring,
    /// for a list inside another box such as a popup or a table. A list
    /// without a frame places its rows in the whole `rect`.
    pub fn framed(self, framed: bool) -> Self {
        Self { framed, ..self }
    }

    /// The same list with a separator line after the row `index`.
    pub fn separator_after(self, index: usize) -> Self {
        Self {
            separator: Some(index),
            ..self
        }
    }

    /// The kind of the row `index`.
    fn kind(&self, index: usize) -> RowKind {
        self.kinds.get(index).copied().unwrap_or_default()
    }

    /// Whether the row `index` takes the selection.
    fn selectable(&self, index: usize) -> bool {
        self.enabled && index < self.rows && self.kind(index) == RowKind::Normal
    }

    /// The selectable row nearest to `index`: the first one in the
    /// direction `forward`, else the first one the other way.
    fn nearest(&self, index: usize, forward: bool) -> Option<usize> {
        let index = index.min(self.rows.saturating_sub(1));
        let after = || (index..self.rows).find(|&row| self.selectable(row));
        let before = || (0..=index).rev().find(|&row| self.selectable(row));
        if forward {
            after().or_else(before)
        } else {
            before().or_else(after)
        }
    }

    /// The selection after `navigation` from `selected` with `page` rows in
    /// a page.
    fn navigate(&self, selected: usize, navigation: Navigation, page: usize) -> usize {
        let target = match navigation {
            Navigation::Up => (0..selected).rev().find(|&row| self.selectable(row)),
            Navigation::Down => (selected + 1..self.rows).find(|&row| self.selectable(row)),
            Navigation::PageUp => self.nearest(selected.saturating_sub(page), false),
            Navigation::PageDown => self.nearest(selected + page, true),
            Navigation::Home => self.nearest(0, true),
            Navigation::End => self.nearest(self.rows, false),
            Navigation::Left
            | Navigation::Right
            | Navigation::Next
            | Navigation::Previous
            | Navigation::PreviousTab
            | Navigation::NextTab => None,
        };
        target.unwrap_or(selected)
    }

    /// Draws the list in `rect`, clipped to `rect`. `state` holds the
    /// selection and the scroll offset between passes. `draw_row` draws the
    /// content of each visible row over the selection fill.
    pub fn show(
        self,
        context: &mut UiContext,
        rect: Rect,
        state: &mut ListState,
        mut draw_row: impl FnMut(&mut UiContext, &ListRow),
    ) -> ListResponse {
        let id = self.id;
        let theme = context.theme;
        let rows = self.rows;
        let layout = RowLayout {
            rows,
            row_height: theme.row_height,
            separator: self.separator.filter(|&separator| separator + 1 < rows),
            separator_height: LIST_PADDING * 2.0 + theme.stroke,
        };
        let mut before = state.selected;
        let view = if self.framed {
            rect.inset(theme.stroke)
        } else {
            rect
        };
        let page =
            (((view.height - LIST_PADDING * 2.0) / theme.row_height).floor() as usize).max(2) - 1;
        let mut response = ListResponse::default();
        let any_selectable = (0..rows).any(|row| self.selectable(row));
        let clamped = state.selected.min(rows.saturating_sub(1));
        if any_selectable && !self.selectable(clamped) {
            let corrected = self.nearest(clamped, true).unwrap_or(0);
            if clamped == before {
                before = corrected;
            }
            state.selected = corrected;
            state.reveal = true;
        } else {
            state.selected = clamped;
        }
        if let Some(navigation) = context.navigation(id) {
            state.selected = self.navigate(state.selected, navigation, page);
        }
        if self.selectable(state.selected) && context.confirmed(id) {
            response.activated = Some(state.selected);
        }
        if state.selected != before {
            state.reveal = true;
        }

        let content = layout.height() + LIST_PADDING * 2.0;
        let scroll_bar = ScrollBar::new(context, id.child("scroll bar"), view, content);
        let overflow = scroll_bar.overflow();
        let mut bar_hot = false;
        if self.enabled {
            let list_hovered = context.hovered(rect);
            bar_hot = scroll_bar.interact(context, &mut state.scroll);
            if list_hovered && context.input.wheel != 0.0 && overflow > 0.0 {
                state.scroll -= context.input.wheel * WHEEL_ROWS * theme.row_height;
                context.state.wheel_taken = true;
            }
        }
        settle_scroll(context, state, &layout, view.height, overflow);

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
                view.y + LIST_PADDING + layout.top(index) - scroll,
                row_width,
                theme.row_height,
            )
        };
        let visible_rows = |scroll: f32| {
            let first = layout.first_below(scroll - LIST_PADDING);
            (first..rows).take_while(move |&index| row_area(scroll, index).y < view.bottom())
        };
        let scroll = state.scroll;
        for index in visible_rows(scroll) {
            if !self.selectable(index) {
                continue;
            }
            let row = row_area(scroll, index);
            let top = row.y.max(view.y);
            let visible = Rect::new(row.x, top, row.width, row.bottom().min(view.bottom()) - top);
            let pointer = context.button_behavior(id.indexed(index), visible);
            if pointer.pressed {
                if context.input.left_double && state.last_pressed == Some(index) {
                    response.activated = Some(index);
                }
                state.last_pressed = Some(index);
                state.selected = index;
                state.reveal = true;
                context.set_focus(id);
            }
            if pointer.clicked {
                response.clicked = Some(index);
            }
        }
        settle_scroll(context, state, &layout, view.height, overflow);
        let captures = if self.enabled && any_selectable {
            NavigationSet::of(&[
                Navigation::PageUp,
                Navigation::PageDown,
                Navigation::Home,
                Navigation::End,
            ])
            .with_if(
                (0..state.selected).any(|row| self.selectable(row)),
                Navigation::Up,
            )
            .with_if(
                (state.selected + 1..rows).any(|row| self.selectable(row)),
                Navigation::Down,
            )
        } else {
            NavigationSet::EMPTY
        };
        if self.enabled {
            context.focusable(id, rect, captures);
        }

        if self.framed {
            let edge = if self.enabled {
                theme.well_edge
            } else {
                theme.well_edge.with_alpha(0.5)
            };
            context.rect(
                rect,
                &RectStyle::filled(theme.well, theme.radius).with_stroke(theme.stroke, edge),
            );
        }
        let focused = context.has_focus(id);
        context.scene.push_clip(view);
        if let Some(separator) = layout.separator {
            let y = view.y + LIST_PADDING + layout.top(separator + 1) - layout.separator_height
                + LIST_PADDING
                - state.scroll;
            context.fill(
                Rect::new(view.x, y, view.width, theme.stroke),
                theme.panel_edge,
            );
        }
        for index in visible_rows(state.scroll) {
            let selectable = self.selectable(index);
            let kind = self.kind(index);
            let row = ListRow {
                index,
                rect: row_area(state.scroll, index),
                selected: selectable && index == state.selected,
                hovered: selectable && context.is_hot(id.indexed(index)),
                focused,
                kind,
                enabled: self.enabled && kind != RowKind::Disabled,
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
                context.scene.fill(row.rect, theme.radius * 2.0 / 3.0, fill);
            }
            draw_row(context, &row);
        }
        context.scene.pop_clip();

        scroll_bar.draw(context, state.scroll, bar_hot);
        if focused && self.framed {
            context.focus_ring(rect, theme.radius);
        }
        if context.hovered(rect) {
            context.mouse_captured = true;
        }
        response.selection_changed = state.selected != before;
        response
    }
}

/// A list of `rows` rows of the theme row height in `rect`, clipped to
/// `rect`, with the behavior of [`List`]. `state` holds the selection and
/// the scroll offset between passes. `draw_row` draws the content of each
/// visible row over the selection fill.
pub fn list(
    context: &mut UiContext,
    id: impl Into<UiId>,
    rect: Rect,
    rows: usize,
    state: &mut ListState,
    draw_row: impl FnMut(&mut UiContext, &ListRow),
) -> ListResponse {
    List::new(id, rows).show(context, rect, state, draw_row)
}

/// Scrolls the selected row of `state` into a view of `view_height` when
/// the state asks for it, then limits the scroll offset to `overflow` and
/// rounds it to whole physical pixels.
fn settle_scroll(
    context: &UiContext,
    state: &mut ListState,
    layout: &RowLayout,
    view_height: f32,
    overflow: f32,
) {
    if state.reveal && layout.rows > 0 {
        let top = LIST_PADDING + layout.top(state.selected);
        if top - LIST_PADDING < state.scroll {
            state.scroll = top - LIST_PADDING;
        }
        if top + layout.row_height + LIST_PADDING > state.scroll + view_height {
            state.scroll = top + layout.row_height + LIST_PADDING - view_height;
        }
    }
    state.reveal = false;
    state.scroll = context.snap(state.scroll.clamp(0.0, overflow));
}

/// Draws the standard content of `row`: an optional `icon`, `text` cut in
/// the middle when it does not fit, and an optional `marker` icon in the
/// marker color at the end. A disabled row draws its icon and its text in
/// the disabled color. Returns whether the text was cut.
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
    let (icon_color, text_color) = if !row.enabled {
        (theme.text_disabled, theme.text_disabled)
    } else if row.selected {
        (theme.text, theme.text)
    } else {
        (theme.text_dim, theme.text)
    };
    if let Some(icon) = icon {
        let (slot, rest) = area.cut_left(theme.icon_size);
        context.icon(slot, icon, theme.icon_size, icon_color);
        area = rest.cut_left(theme.gap).1;
    }
    if let Some(marker) = marker {
        let (slot, rest) = area.cut_right(theme.icon_size);
        let color = if row.enabled {
            theme.marker
        } else {
            theme.text_disabled
        };
        context.icon(slot, marker, theme.icon_size, color);
        area = rest.cut_right(theme.gap).1;
    }
    let style = theme.body().with_color(text_color);
    context.text_in(area, &style, text, Align::Start, Truncation::Middle)
}

/// Draws `text` as the caption of the caption row `row`: small secondary
/// text inside the padding of the row.
pub fn caption_row(context: &mut UiContext, row: &ListRow, text: &str) {
    let theme = context.theme;
    let area = Rect::new(
        row.rect.x + theme.gap,
        row.rect.y,
        (row.rect.width - theme.gap * 2.0).max(0.0),
        row.rect.height,
    );
    context.text_left(area, &theme.small(), text);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{
        scroll_bar::MINIMUM_THUMB,
        tests::{Harness, pixel, srgb},
    };

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
            clicked: pressed.clicked.or(released.clicked),
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

    /// The kinds of a list with a caption, a disabled row and normal rows.
    const KINDS: [RowKind; 6] = [
        RowKind::Caption,
        RowKind::Normal,
        RowKind::Disabled,
        RowKind::Normal,
        RowKind::Caption,
        RowKind::Normal,
    ];

    /// Draws a list of [`KINDS`] with `state` and returns its response.
    fn draw_kinds(harness: &mut Harness, state: &mut ListState) -> ListResponse {
        harness.frame(|context| {
            List::new("files", KINDS.len()).kinds(&KINDS).show(
                context,
                LIST,
                state,
                |context, row| {
                    if row.kind == RowKind::Caption {
                        caption_row(context, row, "Caption");
                    } else {
                        list_row(context, row, None, "image.d88", None);
                    }
                },
            )
        })
    }

    #[test]
    fn the_selection_skips_captions_and_disabled_rows() {
        let mut harness = Harness::new();
        let mut state = ListState::new();
        harness.state.set_focus(UiId::from("files"));
        draw_kinds(&mut harness, &mut state);
        assert_eq!(state.selected, 1);
        harness.navigate(Navigation::Down);
        draw_kinds(&mut harness, &mut state);
        assert_eq!(state.selected, 3);
        harness.navigate(Navigation::Down);
        draw_kinds(&mut harness, &mut state);
        assert_eq!(state.selected, 5);
        harness.navigate(Navigation::Home);
        draw_kinds(&mut harness, &mut state);
        assert_eq!(state.selected, 1);
        harness.navigate(Navigation::End);
        draw_kinds(&mut harness, &mut state);
        assert_eq!(state.selected, 5);
        harness.navigate(Navigation::PageUp);
        draw_kinds(&mut harness, &mut state);
        assert_eq!(state.selected, 1);
        click_kinds(&mut harness, 2, &mut state);
        click_kinds(&mut harness, 0, &mut state);
        assert_eq!(state.selected, 1);
        assert_eq!(click_kinds(&mut harness, 3, &mut state).clicked, Some(3));
        assert_eq!(state.selected, 3);
    }

    /// Clicks the row `index` of the list of [`KINDS`] in two passes.
    fn click_kinds(harness: &mut Harness, index: usize, state: &mut ListState) -> ListResponse {
        let point = row_center(&harness.theme, index, 0.0);
        harness.press(point.0, point.1);
        let pressed = draw_kinds(harness, state);
        harness.release();
        let released = draw_kinds(harness, state);
        ListResponse {
            activated: pressed.activated.or(released.activated),
            clicked: pressed.clicked.or(released.clicked),
            selection_changed: pressed.selection_changed || released.selection_changed,
        }
    }

    #[test]
    fn the_correction_to_the_first_selectable_row_is_no_change() {
        let mut harness = Harness::new();
        let mut state = ListState::new();
        let response = draw_kinds(&mut harness, &mut state);
        assert_eq!(state.selected, 1);
        assert!(!response.selection_changed);
        let response = draw_kinds(&mut harness, &mut state);
        assert!(!response.selection_changed);
    }

    #[test]
    fn up_on_the_first_selectable_row_leaves_the_list() {
        let mut harness = Harness::new();
        let mut state = ListState::new();
        let above = UiId::from("filter");
        harness.state.set_focus(UiId::from("files"));
        for _ in 0..2 {
            harness.frame(|context| {
                context.focusable(
                    above,
                    Rect::new(20.0, 0.0, 300.0, 16.0),
                    NavigationSet::EMPTY,
                );
                List::new("files", KINDS.len()).kinds(&KINDS).show(
                    context,
                    LIST,
                    &mut state,
                    |_, _| {},
                );
            });
            harness.navigate(Navigation::Up);
        }
        assert_eq!(harness.state.focused(), Some(above));
        assert_eq!(state.selected, 1);
    }

    #[test]
    fn a_disabled_list_ignores_input_and_leaves_the_ring() {
        let mut harness = Harness::new();
        let mut state = ListState::new();
        let draw = |harness: &mut Harness, state: &mut ListState| {
            harness.frame(|context| {
                let mut enabled = Vec::new();
                let response =
                    List::new("files", 5)
                        .enabled(false)
                        .show(context, LIST, state, |_, row| enabled.push(row.enabled));
                (response, enabled)
            })
        };
        harness.navigate(Navigation::Next);
        draw(&mut harness, &mut state);
        assert_eq!(harness.state.focused(), None);
        let point = row_center(&harness.theme, 3, 0.0);
        harness.press(point.0, point.1);
        let (response, enabled) = draw(&mut harness, &mut state);
        assert_eq!(state.selected, 0);
        assert!(!response.selection_changed);
        assert!(enabled.iter().all(|enabled| !enabled));
    }

    #[test]
    fn a_separator_opens_a_gap_after_its_row() {
        let mut harness = Harness::new();
        let mut state = ListState::new();
        let tops = harness.frame(|context| {
            let mut tops = Vec::new();
            List::new("files", 3)
                .separator_after(0)
                .show(context, LIST, &mut state, |_, row| tops.push(row.rect.y));
            tops
        });
        let stroke = harness.theme.stroke;
        let row_height = harness.theme.row_height;
        let first = LIST.y + stroke + LIST_PADDING;
        assert_eq!(
            tops,
            [
                first,
                first + row_height + LIST_PADDING * 2.0 + stroke,
                first + row_height * 2.0 + LIST_PADDING * 2.0 + stroke,
            ]
        );
        let layout = RowLayout {
            rows: 3,
            row_height,
            separator: Some(0),
            separator_height: LIST_PADDING * 2.0 + stroke,
        };
        assert_eq!(layout.first_below(0.0), 0);
        assert_eq!(layout.first_below(row_height + 1.0), 1);
        assert_eq!(layout.first_below(row_height * 2.0 + 10.0), 2);
        assert_eq!(layout.first_below(1000.0), 3);
    }

    #[test]
    fn rows_draw_their_kinds_in_the_image() {
        let mut harness = Harness::new();
        let mut state = ListState::new();
        state.select(3);
        harness.state.set_focus(UiId::from("files"));
        draw_kinds(&mut harness, &mut state);
        let theme = &harness.theme;
        let row_left = LIST.x + theme.stroke + LIST_PADDING + 2.0;
        let (_, selected_y) = row_center(theme, 3, 0.0);
        let (_, caption_y) = row_center(theme, 0, 0.0);
        let (_, disabled_y) = row_center(theme, 2, 0.0);
        let pixels = harness.render();
        let width = harness.viewport.width;
        assert_eq!(
            pixel(&pixels, width, row_left, selected_y),
            srgb(theme.selection_focused)
        );
        assert_eq!(pixel(&pixels, width, row_left, caption_y), srgb(theme.well));
        assert_eq!(
            pixel(&pixels, width, row_left, disabled_y),
            srgb(theme.well)
        );
    }

    #[test]
    fn a_disabled_list_draws_a_dimmed_edge() {
        let mut harness = Harness::new();
        let mut state = ListState::new();
        harness.frame(|context| {
            List::new("files", 3)
                .enabled(false)
                .show(context, LIST, &mut state, |_, _| {});
        });
        let pixels = harness.render();
        let edge = pixel(
            &pixels,
            harness.viewport.width,
            LIST.center().0,
            LIST.y + 0.5,
        );
        let theme = &harness.theme;
        assert_ne!(edge, srgb(theme.well_edge));
        assert!(edge[0] < theme.well_edge.r && edge[0] > theme.panel.r / 4);
    }
}
