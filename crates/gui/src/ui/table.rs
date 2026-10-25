//! The table: a list with a column header, sort indicators and rows of
//! cells.

use super::{
    context::{UiContext, UiId},
    focus::{Navigation, NavigationSet},
    icons::Icon,
    label::{Align, Truncation, wrapped_line_height},
    list::{LIST_PADDING, List, ListRow, ListState, RowKind},
};
use crate::{Rect, RectStyle};

/// The size of the sort chevron beside a column caption.
pub(super) const SORT_ICON_SIZE: f32 = 12.0;

/// The width of a column of a [`Table`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColumnWidth {
    /// A share of the width that the fixed columns leave.
    Weight(f32),
    /// A fixed width in logical units.
    Fixed(f32),
}

/// A column of a [`Table`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Column<'a> {
    /// The caption in the header.
    pub caption: &'a str,
    /// The width.
    pub width: ColumnWidth,
    /// The placement of the caption and the cells.
    pub align: Align,
}

impl<'a> Column<'a> {
    /// A left aligned column with the caption `caption` and the width
    /// `width`.
    pub fn new(caption: &'a str, width: ColumnWidth) -> Self {
        Self {
            caption,
            width,
            align: Align::Start,
        }
    }

    /// The same column placed by `align`.
    pub fn align(self, align: Align) -> Self {
        Self { align, ..self }
    }
}

/// The direction of a sort.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SortDirection {
    /// The smallest value first.
    #[default]
    Ascending,
    /// The largest value first.
    Descending,
}

impl SortDirection {
    /// The other direction.
    pub fn reversed(self) -> Self {
        match self {
            Self::Ascending => Self::Descending,
            Self::Descending => Self::Ascending,
        }
    }
}

/// The column and the direction a table sorts by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Sort {
    /// The index of the column.
    pub column: usize,
    /// The direction.
    pub direction: SortDirection,
}

/// The state of a [`Table`] that outlives a pass.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TableState {
    /// The selected row and the scroll offset of the rows.
    pub list: ListState,
    /// The sort of the rows. The caller sorts its rows by it.
    pub sort: Option<Sort>,
    /// The column that the keys picked in the header.
    pub picked: usize,
}

impl TableState {
    /// A table at its top with the first row selected and without a sort.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sorts by `column`: ascending for a new column, the reverse direction
    /// for the sorted column.
    pub fn sort_by(&mut self, column: usize) {
        let direction = match self.sort {
            Some(sort) if sort.column == column => sort.direction.reversed(),
            _ => SortDirection::Ascending,
        };
        self.sort = Some(Sort { column, direction });
    }
}

/// What a [`Table`] did in a pass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TableResponse {
    /// The row that a double click or confirm activated.
    pub activated: Option<usize>,
    /// The row that a click selected.
    pub clicked: Option<usize>,
    /// Whether the selected row changed.
    pub selection_changed: bool,
    /// Whether the sort changed. The caller sorts its rows again.
    pub sort_changed: bool,
}

/// A row of a [`Table`] that the caller draws.
#[derive(Clone, Copy, Debug)]
pub struct TableRow<'a> {
    /// The row of the list of the table.
    pub row: ListRow,
    /// The columns of the table.
    pub columns: &'a [Column<'a>],
    /// The left edge of each column from the inner edge of a row, and its
    /// width.
    spans: &'a [(f32, f32)],
    /// The padding inside a row.
    padding: f32,
}

impl TableRow<'_> {
    /// The area of the cell of `column`.
    pub fn cell(&self, column: usize) -> Rect {
        self.cells(column, column)
    }

    /// The area from the cell of `first` to the cell of `last`, for a cell
    /// that spans several columns.
    pub fn cells(&self, first: usize, last: usize) -> Rect {
        let rect = self.row.rect;
        let left = self.spans.get(first).map_or(0.0, |&(offset, _)| offset);
        let right = self
            .spans
            .get(last.max(first))
            .map_or(left, |&(offset, width)| offset + width);
        Rect::new(
            rect.x + self.padding + left,
            rect.y,
            (right - left).max(0.0),
            rect.height,
        )
    }
}

/// The look of the text or the icon of a cell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CellTone {
    /// Primary text.
    #[default]
    Normal,
    /// Secondary text. A selected row shows it as primary text.
    Dim,
    /// The error color. A selected row shows it as primary text.
    Error,
}

/// The content of a cell of a [`TableRow`]: a text with an optional icon
/// before it.
///
/// The cell is placed as its first column says. A text that does not fit
/// gives way at the end, or in the middle for a file name. A cell can span
/// several columns.
#[derive(Clone, Copy, Debug)]
pub struct TableCell<'a> {
    /// The text.
    text: &'a str,
    /// The look of the text.
    tone: CellTone,
    /// The icon before the text and its look.
    icon: Option<(Icon, CellTone)>,
    /// The cut of a text that does not fit.
    truncation: Truncation,
}

impl<'a> TableCell<'a> {
    /// A cell with `text` in primary text, cut at the end.
    pub fn new(text: &'a str) -> Self {
        Self {
            text,
            tone: CellTone::Normal,
            icon: None,
            truncation: Truncation::End,
        }
    }

    /// The same cell with the text in `tone`.
    pub fn tone(self, tone: CellTone) -> Self {
        Self { tone, ..self }
    }

    /// The same cell with `icon` in `tone` before the text.
    pub fn icon(self, icon: Icon, tone: CellTone) -> Self {
        Self {
            icon: Some((icon, tone)),
            ..self
        }
    }

    /// The same cell, cut as `truncation` says when the text does not fit.
    pub fn truncation(self, truncation: Truncation) -> Self {
        Self { truncation, ..self }
    }

    /// Draws the cell into the columns `first` to `last` of `row`.
    pub fn show(self, context: &mut UiContext, row: &TableRow, first: usize, last: usize) {
        let theme = context.theme;
        let color = |tone: CellTone| match tone {
            CellTone::Dim if !row.row.selected => theme.colors.text_dim,
            CellTone::Error if !row.row.selected => theme.colors.danger_text,
            CellTone::Normal | CellTone::Dim | CellTone::Error => theme.colors.text,
        };
        let align = row
            .columns
            .get(first)
            .map_or(Align::Start, |column| column.align);
        let mut area = row.cells(first, last);
        if let Some((icon, tone)) = self.icon {
            let (slot, rest) = area.cut_left(theme.icon_size);
            context.icon(slot, icon, theme.icon_size, color(tone));
            area = rest.cut_left(theme.gap).1;
        }
        context.text_in(
            area,
            &theme.body().with_color(color(self.tone)),
            self.text,
            align,
            self.truncation,
        );
    }
}

/// The hint that a [`Table`] without rows shows in the area of its rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EmptyHint<'a> {
    /// No hint.
    #[default]
    None,
    /// A line of primary text over a line of small secondary text, both in
    /// the middle of the area, for a table that has no rows at all.
    Centered {
        /// The line of primary text.
        title: &'a str,
        /// The line of small secondary text.
        detail: &'a str,
    },
    /// A line of small secondary text one row below the top of the area,
    /// for a filter without a match.
    Top(&'a str),
}

/// A table: a column header over a list of rows with cells.
///
/// A click on a column caption sorts by that column, a second click
/// reverses the order. The sorted column shows its caption in the primary
/// color and a chevron in the accent color: up for ascending, down for
/// descending. The header is one stop of the focus ring above the rows.
/// While it holds the focus, left and right pick a column and confirm sorts
/// by it. The rows behave like a [`List`]: the selection skips caption rows,
/// and up on the first row goes to the header.
#[derive(Clone, Copy, Debug)]
pub struct Table<'a> {
    /// The id of the table and of its rows.
    id: UiId,
    /// The columns.
    columns: &'a [Column<'a>],
    /// The number of rows.
    rows: usize,
    /// The kinds of the rows.
    kinds: &'a [RowKind],
    /// The hint of a table without rows.
    empty_hint: EmptyHint<'a>,
}

impl<'a> Table<'a> {
    /// A table of `rows` normal rows with `columns`.
    pub fn new(id: impl Into<UiId>, columns: &'a [Column<'a>], rows: usize) -> Self {
        Self {
            id: id.into(),
            columns,
            rows,
            kinds: &[],
            empty_hint: EmptyHint::None,
        }
    }

    /// The same table with the row kinds `kinds`, for caption rows. Rows past
    /// the end of `kinds` are normal rows.
    pub fn kinds(self, kinds: &'a [RowKind]) -> Self {
        Self { kinds, ..self }
    }

    /// The same table, which shows `hint` while it has no rows.
    pub fn empty_hint(self, hint: EmptyHint<'a>) -> Self {
        Self {
            empty_hint: hint,
            ..self
        }
    }

    /// The id of the header.
    pub fn header_id(&self) -> UiId {
        self.id.child("header")
    }

    /// The left edge of each column from the inner edge of a row, and its
    /// width, in a row of the inner width `width`.
    fn spans(&self, context: &UiContext, width: f32) -> Vec<(f32, f32)> {
        let gap = context.theme.padding;
        let gaps = gap * self.columns.len().saturating_sub(1) as f32;
        let (fixed, weights) = self
            .columns
            .iter()
            .fold((0.0, 0.0), |(fixed, weights), column| match column.width {
                ColumnWidth::Fixed(width) => (fixed + width, weights),
                ColumnWidth::Weight(weight) => (fixed, weights + weight.max(0.0)),
            });
        let shared = (width - gaps - fixed).max(0.0);
        let mut offset = 0.0;
        self.columns
            .iter()
            .map(|column| {
                let width = match column.width {
                    ColumnWidth::Fixed(width) => width,
                    ColumnWidth::Weight(weight) if weights > 0.0 => {
                        shared * weight.max(0.0) / weights
                    }
                    ColumnWidth::Weight(_) => 0.0,
                };
                let span = (offset, width);
                offset += width + gap;
                span
            })
            .collect()
    }

    /// Draws the table in `rect`. `state` holds the selection, the scroll
    /// offset, the sort and the picked column between passes. `draw_row`
    /// draws the cells of each visible row over the selection fill.
    pub fn show(
        self,
        context: &mut UiContext,
        rect: Rect,
        state: &mut TableState,
        mut draw_row: impl FnMut(&mut UiContext, &TableRow),
    ) -> TableResponse {
        let theme = context.theme;
        let header_id = self.header_id();
        let count = self.columns.len();
        let mut response = TableResponse::default();
        state.picked = state.picked.min(count.saturating_sub(1));

        let view = rect.inset(theme.stroke);
        let header_top = view.y + LIST_PADDING;
        let rule_top = header_top + theme.row_height + LIST_PADDING;
        let body = Rect::new(
            view.x,
            rule_top + theme.stroke,
            view.width,
            (view.bottom() - rule_top - theme.stroke).max(0.0),
        );
        let content = self.rows as f32 * theme.row_height + LIST_PADDING * 2.0;
        let right_padding = if content > body.height {
            theme.scroll_bar_width
        } else {
            LIST_PADDING
        };
        let header = Rect::new(
            view.x + LIST_PADDING,
            header_top,
            (view.width - LIST_PADDING - right_padding).max(0.0),
            theme.row_height,
        );
        let inner_left = header.x + theme.gap;
        let spans = self.spans(context, (header.width - theme.gap * 2.0).max(0.0));
        let column_area = |index: usize| {
            let (offset, width) = spans[index];
            Rect::new(
                inner_left + offset - theme.gap * 0.75,
                header.y,
                width + theme.gap * 1.5,
                header.height,
            )
        };

        if count > 0 {
            if let Some(navigation) = context.navigation(header_id) {
                state.picked = match navigation {
                    Navigation::Left => state.picked.saturating_sub(1),
                    Navigation::Right => (state.picked + 1).min(count - 1),
                    Navigation::Up
                    | Navigation::Down
                    | Navigation::PageUp
                    | Navigation::PageDown
                    | Navigation::Home
                    | Navigation::End
                    | Navigation::Next
                    | Navigation::Previous
                    | Navigation::PreviousTab
                    | Navigation::NextTab => state.picked,
                };
            }
            if context.confirmed(header_id) {
                state.sort_by(state.picked);
                response.sort_changed = true;
            }
            for index in 0..count {
                let pointer = context.button_behavior(header_id.indexed(index), column_area(index));
                if pointer.clicked {
                    state.picked = index;
                    state.sort_by(index);
                    response.sort_changed = true;
                    context.set_focus(header_id);
                }
            }
            let captures = NavigationSet::EMPTY
                .with_if(state.picked > 0, Navigation::Left)
                .with_if(state.picked + 1 < count, Navigation::Right);
            context.focusable(header_id, header, captures);
        }

        context.rect(
            rect,
            &RectStyle::filled(theme.colors.well, theme.radius)
                .with_stroke(theme.stroke, theme.colors.well_edge),
        );
        let header_focused = context.shows_focus(header_id);
        let picking = header_focused && context.focus_visible();
        let caption_style = theme.small();
        for (index, column) in self.columns.iter().enumerate() {
            let area = column_area(index);
            let picked = picking && index == state.picked;
            let hot = context.is_hot(header_id.indexed(index));
            if picked || hot {
                let fill = if picked {
                    theme.colors.selection_focused
                } else {
                    theme.colors.row_hover
                };
                context.scene.fill(
                    Rect::new(
                        area.x,
                        area.y + theme.gap * 0.375,
                        area.width,
                        area.height - theme.gap * 0.75,
                    ),
                    theme.radius * 2.0 / 3.0,
                    fill,
                );
            }
            let sorted = state.sort.filter(|sort| sort.column == index);
            let text_color = if sorted.is_some() || picked {
                theme.colors.text
            } else {
                theme.colors.text_dim
            };
            let style = caption_style.with_color(text_color);
            let (offset, width) = spans[index];
            let cell = Rect::new(inner_left + offset, header.y, width, header.height);
            let Some(sort) = sorted else {
                context.text_in(cell, &style, column.caption, column.align, Truncation::End);
                continue;
            };
            let spacing = theme.gap * 0.5;
            let caption_width = context
                .measure(&style, column.caption)
                .min((width - SORT_ICON_SIZE - spacing).max(0.0));
            let group = caption_width + spacing + SORT_ICON_SIZE;
            let left = match column.align {
                Align::Start => cell.x,
                Align::Center => cell.x + (cell.width - group) * 0.5,
                Align::End => cell.right() - group,
            };
            context.text_in(
                Rect::new(left, cell.y, caption_width + 1.0, cell.height),
                &style,
                column.caption,
                Align::Start,
                Truncation::End,
            );
            let icon = match sort.direction {
                SortDirection::Ascending => Icon::Up,
                SortDirection::Descending => Icon::Down,
            };
            let icon_color = if picked {
                theme.colors.text
            } else {
                theme.colors.accent
            };
            context.icon(
                Rect::new(
                    left + caption_width + spacing,
                    cell.y,
                    SORT_ICON_SIZE,
                    cell.height,
                ),
                icon,
                SORT_ICON_SIZE,
                icon_color,
            );
        }
        context.fill(
            Rect::new(view.x, rule_top, view.width, theme.stroke),
            theme.colors.well_edge,
        );

        let columns = self.columns;
        let padding = theme.gap;
        let list = List::new(self.id, self.rows)
            .kinds(self.kinds)
            .framed(false)
            .show(context, body, &mut state.list, |context, row| {
                draw_row(
                    context,
                    &TableRow {
                        row: *row,
                        columns,
                        spans: &spans,
                        padding,
                    },
                );
            });
        if self.rows == 0 {
            draw_empty_hint(context, body, self.empty_hint);
        }
        if header_focused {
            context.focus_ring(header, theme.radius);
        }
        if context.shows_focus(self.id) {
            context.focus_ring(rect, theme.radius);
        }
        response.activated = list.activated;
        response.clicked = list.clicked;
        response.selection_changed = list.selection_changed;
        response
    }
}

/// Draws `hint` into the area `body` of the rows of a table.
fn draw_empty_hint(context: &mut UiContext, body: Rect, hint: EmptyHint) {
    let theme = context.theme;
    let detail_style = theme.small();
    let detail_height = wrapped_line_height(&detail_style);
    match hint {
        EmptyHint::None => {}
        EmptyHint::Centered { title, detail } => {
            let title_style = theme.body();
            let title_height = wrapped_line_height(&title_style);
            let spacing = theme.gap * 0.5;
            let top = body.y + (body.height - title_height - spacing - detail_height) * 0.5;
            context.text_in(
                Rect::new(body.x, top, body.width, title_height),
                &title_style,
                title,
                Align::Center,
                Truncation::End,
            );
            context.text_in(
                Rect::new(
                    body.x,
                    top + title_height + spacing,
                    body.width,
                    detail_height,
                ),
                &detail_style,
                detail,
                Align::Center,
                Truncation::Middle,
            );
        }
        EmptyHint::Top(text) => {
            context.text_in(
                Rect::new(
                    body.x,
                    body.y + theme.row_height + theme.gap,
                    body.width,
                    detail_height,
                ),
                &detail_style,
                text,
                Align::Center,
                Truncation::End,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{
        caption_row,
        tests::{Harness, pixel, srgb},
    };

    /// The columns of the game table.
    const COLUMNS: [Column; 4] = [
        Column {
            caption: "Title",
            width: ColumnWidth::Weight(3.0),
            align: Align::Start,
        },
        Column {
            caption: "Original title",
            width: ColumnWidth::Weight(3.0),
            align: Align::Start,
        },
        Column {
            caption: "Developer",
            width: ColumnWidth::Weight(2.0),
            align: Align::Start,
        },
        Column {
            caption: "Year",
            width: ColumnWidth::Fixed(64.0),
            align: Align::End,
        },
    ];

    /// The games of the table.
    const GAMES: [[&str; 4]; 5] = [
        ["Brandish", "ブランディッシュ", "Nihon Falcom", "1991"],
        ["Dokyusei", "同級生", "Elf", "1992"],
        [
            "Dragon Slayer: The Legend of Heroes",
            "ドラゴンスレイヤー英雄伝説",
            "Nihon Falcom",
            "1989",
        ],
        ["", "Before 1992", "", ""],
        ["Edge", "エッジ", "TGL", "1993"],
    ];

    /// The kinds of the rows: the fourth row is a caption.
    const KINDS: [RowKind; 5] = [
        RowKind::Normal,
        RowKind::Normal,
        RowKind::Normal,
        RowKind::Caption,
        RowKind::Normal,
    ];

    /// The area of the table.
    const TABLE: Rect = Rect::new(20.0, 20.0, 640.0, 250.0);

    /// Draws the table with `state`.
    fn draw(harness: &mut Harness, state: &mut TableState) -> TableResponse {
        harness.frame(|context| {
            Table::new("games", &COLUMNS, GAMES.len())
                .kinds(&KINDS)
                .show(context, TABLE, state, |context, row| {
                    if row.row.kind == RowKind::Caption {
                        caption_row(context, &row.row, GAMES[row.row.index][1]);
                        return;
                    }
                    let game = GAMES[row.row.index];
                    for (column, text) in game.iter().enumerate() {
                        let tone = if column >= 2 {
                            CellTone::Dim
                        } else {
                            CellTone::Normal
                        };
                        TableCell::new(text)
                            .tone(tone)
                            .show(context, row, column, column);
                    }
                })
        })
    }

    /// The center of the header caption of `column`.
    fn header_point(harness: &mut Harness, column: usize) -> (f32, f32) {
        let theme = &harness.theme;
        let header = Rect::new(
            TABLE.x + theme.stroke + LIST_PADDING,
            TABLE.y + theme.stroke + LIST_PADDING,
            TABLE.width - theme.stroke * 2.0 - LIST_PADDING * 2.0,
            theme.row_height,
        );
        let gap = theme.gap;
        let spans = harness.frame(|context| {
            Table::new("games", &COLUMNS, 0).spans(context, header.width - gap * 2.0)
        });
        let (offset, width) = spans[column];
        (header.x + gap + offset + width * 0.5, header.center().1)
    }

    #[test]
    fn columns_share_the_width_by_weight_after_the_fixed_columns() {
        let mut harness = Harness::new();
        let spans = harness.frame(|context| Table::new("games", &COLUMNS, 0).spans(context, 664.0));
        let gap = harness.theme.padding;
        let shared = 664.0 - gap * 3.0 - 64.0;
        assert_eq!(spans[0], (0.0, shared * 3.0 / 8.0));
        assert_eq!(spans[3].1, 64.0);
        assert_eq!(spans[3].0 + spans[3].1, 664.0);
    }

    #[test]
    fn a_click_on_a_caption_sorts_and_a_second_reverses() {
        let mut harness = Harness::new();
        let mut state = TableState::new();
        draw(&mut harness, &mut state);
        let (x, y) = header_point(&mut harness, 3);
        for direction in [SortDirection::Ascending, SortDirection::Descending] {
            harness.press(x, y);
            draw(&mut harness, &mut state);
            harness.release();
            assert!(draw(&mut harness, &mut state).sort_changed);
            assert_eq!(
                state.sort,
                Some(Sort {
                    column: 3,
                    direction
                })
            );
        }
    }

    #[test]
    fn the_header_is_a_stop_above_the_rows() {
        let mut harness = Harness::new();
        let mut state = TableState::new();
        harness.state.set_focus(UiId::from("games"));
        draw(&mut harness, &mut state);
        harness.navigate(Navigation::Up);
        draw(&mut harness, &mut state);
        let header = Table::new("games", &COLUMNS, 0).header_id();
        assert_eq!(harness.state.focused(), Some(header));
        for _ in 0..5 {
            harness.navigate(Navigation::Right);
            draw(&mut harness, &mut state);
        }
        assert_eq!(state.picked, 3);
        assert_eq!(harness.state.focused(), Some(header));
        harness.navigate(Navigation::Left);
        draw(&mut harness, &mut state);
        harness.input.confirm = true;
        assert!(draw(&mut harness, &mut state).sort_changed);
        assert_eq!(
            state.sort,
            Some(Sort {
                column: 2,
                direction: SortDirection::Ascending
            })
        );
        harness.navigate(Navigation::Down);
        draw(&mut harness, &mut state);
        assert_eq!(harness.state.focused(), Some(UiId::from("games")));
    }

    #[test]
    fn the_selection_skips_the_caption_row() {
        let mut harness = Harness::new();
        let mut state = TableState::new();
        state.list.select(2);
        harness.state.set_focus(UiId::from("games"));
        draw(&mut harness, &mut state);
        harness.navigate(Navigation::Down);
        draw(&mut harness, &mut state);
        assert_eq!(state.list.selected, 4);
    }

    #[test]
    fn the_picked_column_and_the_selected_row_draw_their_fills() {
        let mut harness = Harness::new();
        let mut state = TableState::new();
        state.list.select(1);
        state.sort = Some(Sort {
            column: 0,
            direction: SortDirection::Ascending,
        });
        let header = Table::new("games", &COLUMNS, 0).header_id();
        let (x, y) = header_point(&mut harness, 2);
        harness.state.set_focus(UiId::from("games"));
        draw(&mut harness, &mut state);
        let theme = &harness.theme;
        let row_y = TABLE.y
            + theme.stroke
            + LIST_PADDING * 3.0
            + theme.row_height * 1.5
            + theme.stroke
            + theme.row_height;
        let pixels = harness.render();
        let width = harness.viewport.width;
        assert_eq!(
            pixel(&pixels, width, TABLE.x + 10.0, row_y),
            srgb(theme.colors.selection_focused)
        );
        let chevron: Vec<[f32; 4]> = harness
            .scene
            .instances()
            .iter()
            .filter(|instance| instance.color == harness.theme.colors.accent.to_linear())
            .map(|instance| instance.rect)
            .collect();
        assert_eq!(chevron.len(), 2);
        let left = chevron.iter().map(|rect| rect[0]).fold(f32::MAX, f32::min);
        let right = chevron
            .iter()
            .map(|rect| rect[0] + rect[2])
            .fold(f32::MIN, f32::max);
        let unit = SORT_ICON_SIZE / 16.0;
        let stroke = (1.75 * unit).max(1.25);
        assert!(
            (right - left - (8.0 * unit + stroke)).abs() < 0.01,
            "{chevron:?}"
        );

        harness.state.set_focus(header);
        state.picked = 1;
        harness.navigate(Navigation::Right);
        draw(&mut harness, &mut state);
        assert_eq!(state.picked, 2);
        let pixels = harness.render();
        let theme = &harness.theme;
        let fill = pixel(&pixels, harness.viewport.width, x - 30.0, y - 9.0);
        assert_eq!(fill, srgb(theme.colors.selection_focused));
    }

    #[test]
    fn page_keys_and_a_double_click_reach_the_rows() {
        let mut harness = Harness::new();
        let mut state = TableState::new();
        harness.state.set_focus(UiId::from("games"));
        draw(&mut harness, &mut state);
        harness.navigate(Navigation::PageDown);
        draw(&mut harness, &mut state);
        assert_eq!(state.list.selected, 4);
        harness.navigate(Navigation::PageUp);
        draw(&mut harness, &mut state);
        assert_eq!(state.list.selected, 0);
        harness.navigate(Navigation::End);
        draw(&mut harness, &mut state);
        assert_eq!(state.list.selected, 4);
        let theme = &harness.theme;
        let row_y = TABLE.y
            + theme.stroke
            + LIST_PADDING * 3.0
            + theme.row_height
            + theme.stroke
            + theme.row_height * 1.5;
        harness.press(TABLE.x + 100.0, row_y);
        draw(&mut harness, &mut state);
        harness.release();
        draw(&mut harness, &mut state);
        harness.input.left_double = true;
        harness.press(TABLE.x + 100.0, row_y);
        assert_eq!(draw(&mut harness, &mut state).activated, Some(1));
        harness.release();
        draw(&mut harness, &mut state);
        harness.input.confirm = true;
        assert_eq!(draw(&mut harness, &mut state).activated, Some(1));
    }

    #[test]
    fn disabled_rows_draw_dimmed_and_take_no_selection() {
        let mut harness = Harness::new();
        let kinds = [RowKind::Normal, RowKind::Disabled, RowKind::Normal];
        let mut state = TableState::new();
        harness.state.set_focus(UiId::from("games"));
        let mut enabled = Vec::new();
        let frame = |harness: &mut Harness, state: &mut TableState, enabled: &mut Vec<bool>| {
            harness.frame(|context| {
                enabled.clear();
                Table::new("games", &COLUMNS, 3).kinds(&kinds).show(
                    context,
                    TABLE,
                    state,
                    |context, row| {
                        enabled.push(row.row.enabled);
                        TableCell::new("Game")
                            .tone(CellTone::Normal)
                            .show(context, row, 0, 0);
                    },
                )
            })
        };
        frame(&mut harness, &mut state, &mut enabled);
        harness.navigate(Navigation::Down);
        frame(&mut harness, &mut state, &mut enabled);
        assert_eq!(state.list.selected, 2);
        assert_eq!(enabled, [true, false, true]);
    }

    #[test]
    fn an_empty_table_activates_nothing_and_keeps_its_header() {
        let mut harness = Harness::new();
        let mut state = TableState::new();
        let header = Table::new("games", &COLUMNS, 0).header_id();
        let frame = |harness: &mut Harness, state: &mut TableState| {
            harness.frame(|context| {
                Table::new("games", &COLUMNS, 0).show(context, TABLE, state, |_, _| {})
            })
        };
        frame(&mut harness, &mut state);
        harness.navigate(Navigation::Next);
        frame(&mut harness, &mut state);
        assert_eq!(harness.state.focused(), Some(header));
        harness.input.confirm = true;
        assert!(frame(&mut harness, &mut state).sort_changed);
        harness.navigate(Navigation::Down);
        frame(&mut harness, &mut state);
        harness.input.confirm = true;
        let response = frame(&mut harness, &mut state);
        assert_eq!(response.activated, None);
        assert!(!response.selection_changed);
        assert_eq!(state.list.selected, 0);
    }

    /// Draws one broken row with an alert icon in the first column and the
    /// error over the second and the third column.
    fn draw_broken(harness: &mut Harness, state: &mut TableState) {
        harness.frame(|context| {
            Table::new("games", &COLUMNS, 2).show(context, TABLE, state, |context, row| {
                TableCell::new("nemesis-90.conf")
                    .icon(Icon::Alert, CellTone::Error)
                    .truncation(Truncation::Middle)
                    .show(context, row, 0, 0);
                TableCell::new("line 4: unknown machine X68030")
                    .tone(CellTone::Error)
                    .show(context, row, 1, 2);
            });
        });
    }

    /// Verifies that an error cell draws in the error color.
    #[test]
    fn an_error_cell_draws_in_the_error_color_and_a_selected_one_in_text() {
        let mut harness = Harness::new();
        let mut state = TableState::new();
        harness.state.set_focus(UiId::from("games"));
        draw_broken(&mut harness, &mut state);
        let theme = &harness.theme;
        let colors: Vec<_> = harness
            .scene
            .instances()
            .iter()
            .map(|instance| instance.color)
            .collect();
        assert!(colors.contains(&theme.colors.danger_text.to_linear()));
        assert!(colors.contains(&theme.colors.text.to_linear()));
    }

    /// Verifies that a cell over two columns spans both and the gap between them.
    #[test]
    fn a_spanning_cell_covers_its_columns_and_the_gaps_between() {
        let mut harness = Harness::new();
        let mut spans = Vec::new();
        let mut state = TableState::new();
        harness.frame(|context| {
            Table::new("games", &COLUMNS, 1).show(context, TABLE, &mut state, |_, row| {
                spans.push((row.cell(1), row.cell(2), row.cells(1, 2)));
            });
        });
        let (first, last, both) = spans[0];
        assert_eq!(both.x, first.x);
        assert_eq!(both.right(), last.right());
    }

    /// Draws an empty table with `hint` and returns the rendered pixels.
    fn draw_empty(harness: &mut Harness, hint: EmptyHint) -> Vec<u8> {
        let mut state = TableState::new();
        harness.frame(|context| {
            Table::new("games", &COLUMNS, 0).empty_hint(hint).show(
                context,
                TABLE,
                &mut state,
                |_, _| {},
            );
        });
        harness.render()
    }

    /// Whether a pixel of the row band from `top` to `bottom` in the middle
    /// of the table differs from the well.
    fn band_lit(harness: &Harness, pixels: &[u8], top: f32, bottom: f32) -> bool {
        let width = harness.viewport.width;
        let well = srgb(harness.theme.colors.well);
        let mut y = top;
        while y < bottom {
            let mut x = TABLE.center().0 - 60.0;
            while x < TABLE.center().0 + 60.0 {
                if pixel(pixels, width, x, y) != well {
                    return true;
                }
                x += 1.0;
            }
            y += 1.0;
        }
        false
    }

    /// Verifies where the hints of an empty table draw.
    #[test]
    fn the_hints_of_an_empty_table_draw_in_the_middle_or_at_the_top() {
        let mut harness = Harness::new();
        let theme = &harness.theme;
        let body_top =
            TABLE.y + theme.stroke + LIST_PADDING * 2.0 + theme.row_height + theme.stroke;
        let row_height = theme.row_height;
        let pixels = draw_empty(&mut harness, EmptyHint::None);
        assert!(!band_lit(
            &harness,
            &pixels,
            body_top + 2.0,
            TABLE.bottom() - 4.0
        ));

        let pixels = draw_empty(&mut harness, EmptyHint::Top("No games match the filter"));
        assert!(band_lit(
            &harness,
            &pixels,
            body_top + row_height,
            body_top + row_height * 2.0
        ));
        assert!(!band_lit(
            &harness,
            &pixels,
            body_top + row_height * 2.0,
            TABLE.bottom() - 4.0
        ));

        let pixels = draw_empty(
            &mut harness,
            EmptyHint::Centered {
                title: "No games for PC-6000",
                detail: "Place game files in config/pc6000/",
            },
        );
        let middle = body_top + (TABLE.bottom() - body_top) * 0.5;
        assert!(band_lit(&harness, &pixels, middle - 20.0, middle));
        assert!(band_lit(&harness, &pixels, middle, middle + 20.0));
        assert!(!band_lit(&harness, &pixels, body_top + 2.0, middle - 30.0));
    }

    /// Verifies that a table with rows shows no hint.
    #[test]
    fn a_table_with_rows_shows_no_hint() {
        let mut harness = Harness::new();
        let mut state = TableState::new();
        let hint = EmptyHint::Top("No games match the filter");
        harness.frame(|context| {
            Table::new("games", &COLUMNS, 1).empty_hint(hint).show(
                context,
                TABLE,
                &mut state,
                |_, _| {},
            );
        });
        let pixels = harness.render();
        let theme = &harness.theme;
        let body_top =
            TABLE.y + theme.stroke + LIST_PADDING * 2.0 + theme.row_height + theme.stroke;
        assert!(!band_lit(
            &harness,
            &pixels,
            body_top + theme.row_height + LIST_PADDING + 2.0,
            body_top + theme.row_height * 2.0
        ));
    }
}
