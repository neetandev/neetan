//! The game table of the library: the columns title, original title,
//! developer and year, the files with errors under a caption and the hints
//! of an empty table.

use gui::{
    Rect,
    ui::{
        Align, CellTone, Column, ColumnWidth, EmptyHint, Icon, RowKind, Table, TableCell,
        TableResponse, TableRow, TableState, Truncation, UiContext, UiId, caption_row,
    },
};
use i18n::Catalog;

use super::{
    entries::{DEVELOPER_COLUMN, TITLE_COLUMN, TITLE_ORIGINAL_COLUMN, TableEntry, YEAR_COLUMN},
    rows::GameRow,
};
use crate::texts::library;

/// The id of the game table.
pub(crate) const GAME_TABLE_ID: &str = "games";

/// The width of the year column in logical units.
const YEAR_COLUMN_WIDTH: f32 = 64.0;

/// The share of the title column of the width that the year column leaves.
const TITLE_WEIGHT: f32 = 3.0;

/// The share of the original title column.
const TITLE_ORIGINAL_WEIGHT: f32 = 3.0;

/// The share of the developer column.
const DEVELOPER_WEIGHT: f32 = 2.0;

/// The columns of the game table with their captions in `catalog`.
fn columns(catalog: &Catalog) -> [Column<'_>; 4] {
    [
        Column {
            caption: catalog.text(library::COLUMN_TITLE),
            width: ColumnWidth::Weight(TITLE_WEIGHT),
            align: Align::Start,
        },
        Column {
            caption: catalog.text(library::COLUMN_ORIGINAL_TITLE),
            width: ColumnWidth::Weight(TITLE_ORIGINAL_WEIGHT),
            align: Align::Start,
        },
        Column {
            caption: catalog.text(library::COLUMN_DEVELOPER),
            width: ColumnWidth::Weight(DEVELOPER_WEIGHT),
            align: Align::Start,
        },
        Column {
            caption: catalog.text(library::COLUMN_YEAR),
            width: ColumnWidth::Fixed(YEAR_COLUMN_WIDTH),
            align: Align::End,
        },
    ]
}

/// The id of the column header of the game table.
pub(crate) fn header_id() -> UiId {
    Table::new(GAME_TABLE_ID, &[], 0).header_id()
}

/// Draws the cells of the game `row` into `table_row`.
fn draw_game(context: &mut UiContext, table_row: &TableRow, row: &GameRow) {
    for (column, text, tone) in [
        (TITLE_COLUMN, &row.title, CellTone::Normal),
        (TITLE_ORIGINAL_COLUMN, &row.title_original, CellTone::Normal),
        (DEVELOPER_COLUMN, &row.developer, CellTone::Dim),
        (YEAR_COLUMN, &row.year_text, CellTone::Dim),
    ] {
        TableCell::new(text)
            .tone(tone)
            .show(context, table_row, column, column);
    }
}

/// Draws the file name and the error of the file `row` into `table_row`.
/// The error spans the columns of the original title and the developer.
fn draw_broken(context: &mut UiContext, table_row: &TableRow, row: &GameRow) {
    TableCell::new(&row.file_name)
        .icon(Icon::Alert, CellTone::Error)
        .truncation(Truncation::Middle)
        .show(context, table_row, TITLE_COLUMN, TITLE_COLUMN);
    TableCell::new(row.error.as_deref().unwrap_or_default())
        .tone(CellTone::Error)
        .show(context, table_row, TITLE_ORIGINAL_COLUMN, DEVELOPER_COLUMN);
}

/// Draws the game table of `entries` of `rows` into `area` with the
/// selection, the scroll offset and the sort in `state`. `kinds` holds the
/// row kind of each entry. A table without entries shows `hint`. Returns
/// what the table did.
pub(crate) fn show_game_table(
    context: &mut UiContext,
    area: Rect,
    entries: &[TableEntry],
    kinds: &[RowKind],
    rows: &[GameRow],
    state: &mut TableState,
    hint: EmptyHint,
) -> TableResponse {
    let catalog = context.catalog;
    let columns = columns(catalog);
    Table::new(GAME_TABLE_ID, &columns, entries.len())
        .kinds(kinds)
        .empty_hint(hint)
        .show(context, area, state, |context, table_row| {
            match entries[table_row.row.index] {
                TableEntry::Game(index) => draw_game(context, table_row, &rows[index]),
                TableEntry::Broken(index) => draw_broken(context, table_row, &rows[index]),
                TableEntry::Caption => caption_row(
                    context,
                    &table_row.row,
                    catalog.text(library::ERRORS_CAPTION),
                ),
            }
        })
}

#[cfg(test)]
mod tests {
    use gui::ui::Navigation;

    use super::*;
    use crate::{
        gui_test_support::GuiHarness,
        manager::rows::tests::{broken, game},
    };

    /// The area of the table in the tests.
    const AREA: Rect = Rect::new(0.0, 0.0, 700.0, 400.0);

    /// Verifies that the selection skips the caption over the files with errors and reaches them.
    #[test]
    fn the_selection_skips_the_caption_and_reaches_the_files_with_errors() {
        let rows = vec![game("edge", "Edge", "エッジ", "TGL", "1993"), broken("bad")];
        let entries = [
            TableEntry::Game(0),
            TableEntry::Caption,
            TableEntry::Broken(1),
        ];
        let kinds = entries.map(TableEntry::kind);
        let mut harness = GuiHarness::new(960, 600);
        let mut state = TableState::new();
        harness.state.set_focus(UiId::from(GAME_TABLE_ID));
        let draw = |harness: &mut GuiHarness, state: &mut TableState| {
            harness.frame(|context| {
                show_game_table(
                    context,
                    AREA,
                    &entries,
                    &kinds,
                    &rows,
                    state,
                    EmptyHint::None,
                )
            })
        };
        let danger = harness.theme.colors.danger_text.to_linear();
        let has_danger = |harness: &GuiHarness| {
            harness
                .scene
                .instances()
                .iter()
                .any(|instance| instance.color == danger)
        };
        draw(&mut harness, &mut state);
        assert!(has_danger(&harness));
        harness.input.navigation = Some(Navigation::Down);
        draw(&mut harness, &mut state);
        assert_eq!(state.list.selected, 2);
        assert!(!has_danger(&harness));
    }

    /// Verifies that a table without entries leaves the rows out of the focus ring and keeps the header.
    #[test]
    fn an_empty_table_keeps_only_its_header_in_the_ring() {
        let mut harness = GuiHarness::new(960, 600);
        let mut state = TableState::new();
        let draw = |harness: &mut GuiHarness, state: &mut TableState| {
            harness.frame(|context| {
                show_game_table(
                    context,
                    AREA,
                    &[],
                    &[],
                    &[],
                    state,
                    EmptyHint::Top("No games match the filter"),
                )
            })
        };
        draw(&mut harness, &mut state);
        harness.input.navigation = Some(Navigation::Next);
        draw(&mut harness, &mut state);
        assert_eq!(harness.state.focused(), Some(header_id()));
        harness.input.navigation = Some(Navigation::Next);
        draw(&mut harness, &mut state);
        assert_eq!(harness.state.focused(), Some(header_id()));
    }
}
