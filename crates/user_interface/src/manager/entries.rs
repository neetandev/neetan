//! The entries of the game table: the games of a system that pass the filter
//! in the order of the sort, then a caption and the files with errors.

use std::cmp::Ordering;

use gui::ui::{RowKind, Sort, SortDirection};

use super::{
    filter::{Filter, fold},
    rows::GameRow,
};

/// The column of the title.
pub(crate) const TITLE_COLUMN: usize = 0;

/// The column of the original title.
pub(crate) const TITLE_ORIGINAL_COLUMN: usize = 1;

/// The column of the developer.
pub(crate) const DEVELOPER_COLUMN: usize = 2;

/// The column of the year.
pub(crate) const YEAR_COLUMN: usize = 3;

/// A column of the game table that the sort compares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortColumn {
    /// The title.
    Title,
    /// The original title.
    TitleOriginal,
    /// The developer.
    Developer,
    /// The year.
    Year,
}

/// The sort columns in the order of the columns of the game table.
const SORT_COLUMNS: [SortColumn; 4] = [
    SortColumn::Title,
    SortColumn::TitleOriginal,
    SortColumn::Developer,
    SortColumn::Year,
];

/// The sort of a new library: by title, ascending.
pub(crate) const DEFAULT_SORT: Sort = Sort {
    column: TITLE_COLUMN,
    direction: SortDirection::Ascending,
};

/// One entry of the game table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TableEntry {
    /// The game at this index of the rows of the system.
    Game(usize),
    /// The caption over the files with errors.
    Caption,
    /// The file with an error at this index of the rows of the system.
    Broken(usize),
}

impl TableEntry {
    /// The index of the row of the entry in the rows of the system.
    pub(crate) fn row(self) -> Option<usize> {
        match self {
            Self::Game(index) | Self::Broken(index) => Some(index),
            Self::Caption => None,
        }
    }

    /// The kind of the entry in the table.
    pub(crate) fn kind(self) -> RowKind {
        match self {
            Self::Game(_) | Self::Broken(_) => RowKind::Normal,
            Self::Caption => RowKind::Caption,
        }
    }
}

/// The keys of a game that the sort compares, in their folded form.
struct SortKeys {
    /// The title.
    title: String,
    /// The original title. Empty is unset.
    title_original: String,
    /// The developer. Empty is unset.
    developer: String,
    /// The year.
    year: Option<u16>,
    /// The file name, the last tie break.
    file_name: String,
}

impl SortKeys {
    /// The keys of `row`.
    fn new(row: &GameRow) -> Self {
        Self {
            title: fold(&row.title),
            title_original: fold(row.title_original.trim()),
            developer: fold(row.developer.trim()),
            year: row.year,
            file_name: fold(&row.file_name),
        }
    }
}

/// Compares the set values `first` and `second` in `direction`. An unset
/// value follows every set value in both directions.
fn compare_set<T: Ord>(first: Option<T>, second: Option<T>, direction: SortDirection) -> Ordering {
    match (first, second) {
        (Some(first), Some(second)) => match direction {
            SortDirection::Ascending => first.cmp(&second),
            SortDirection::Descending => second.cmp(&first),
        },
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// The text `text`, or `None` when it is empty.
fn set_text(text: &str) -> Option<&str> {
    (!text.is_empty()).then_some(text)
}

/// Compares the games with the keys `first` and `second` by `sort`. Equal
/// values follow the title and then the file name, both ascending.
fn compare(first: &SortKeys, second: &SortKeys, sort: Sort) -> Ordering {
    let direction = sort.direction;
    let column = SORT_COLUMNS
        .get(sort.column)
        .copied()
        .unwrap_or(SortColumn::Title);
    let primary = match column {
        SortColumn::Title => compare_set(Some(&first.title), Some(&second.title), direction),
        SortColumn::TitleOriginal => compare_set(
            set_text(&first.title_original),
            set_text(&second.title_original),
            direction,
        ),
        SortColumn::Developer => compare_set(
            set_text(&first.developer),
            set_text(&second.developer),
            direction,
        ),
        SortColumn::Year => compare_set(first.year, second.year, direction),
    };
    primary
        .then_with(|| first.title.cmp(&second.title))
        .then_with(|| first.file_name.cmp(&second.file_name))
}

/// Returns the entries of `rows` that pass `filter`: the games in the order
/// of `sort`, then the caption and the files with errors by file name. The
/// caption shows only with a file with an error.
pub(crate) fn table_entries(rows: &[GameRow], sort: Sort, filter: &Filter) -> Vec<TableEntry> {
    let passing = |launchable: bool| {
        rows.iter()
            .enumerate()
            .filter(move |(_, row)| row.launchable() == launchable && filter.matches(row))
            .map(|(index, row)| (index, SortKeys::new(row)))
            .collect::<Vec<_>>()
    };
    let mut games = passing(true);
    games.sort_by(|(_, first), (_, second)| compare(first, second, sort));
    let mut broken = passing(false);
    broken.sort_by(|(_, first), (_, second)| first.file_name.cmp(&second.file_name));
    let mut entries: Vec<TableEntry> = games
        .into_iter()
        .map(|(index, _)| TableEntry::Game(index))
        .collect();
    if !broken.is_empty() {
        entries.push(TableEntry::Caption);
        entries.extend(
            broken
                .into_iter()
                .map(|(index, _)| TableEntry::Broken(index)),
        );
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::rows::tests::{broken, game};

    /// The sample games: a full row, Japanese titles, a game without
    /// metadata and games that share values.
    fn sample() -> Vec<GameRow> {
        vec![
            game("edge", "Edge", "エッジ", "TGL", "1993"),
            game(
                "brandish",
                "Brandish",
                "ブランディッシュ",
                "Nihon Falcom",
                "1991",
            ),
            game("dokyusei", "同級生", "どうきゅうせい", "Elf", "1992"),
            game("plain", "", "", "", ""),
            game("ys-2", "Ys II", "イースII", "Nihon Falcom", "1988"),
            game("akumajou", "悪魔城ドラキュラ", "", "Konami", "bad"),
        ]
    }

    /// The file stems of the games of `entries` in table order.
    fn stems(rows: &[GameRow], entries: &[TableEntry]) -> Vec<String> {
        entries
            .iter()
            .map(|entry| match entry {
                TableEntry::Game(index) | TableEntry::Broken(index) => {
                    rows[*index].file_name.trim_end_matches(".conf").to_owned()
                }
                TableEntry::Caption => "|".to_owned(),
            })
            .collect()
    }

    /// Returns the stems of `rows` sorted by `column` in `direction`.
    fn sorted(rows: &[GameRow], column: usize, direction: SortDirection) -> Vec<String> {
        let entries = table_entries(rows, Sort { column, direction }, &Filter::new(""));
        stems(rows, &entries)
    }

    /// Verifies the title order in both directions with Japanese titles and a game without metadata.
    #[test]
    fn titles_sort_without_regard_to_case_in_both_directions() {
        let rows = sample();
        assert_eq!(
            sorted(&rows, TITLE_COLUMN, SortDirection::Ascending),
            ["brandish", "edge", "plain", "ys-2", "dokyusei", "akumajou"]
        );
        assert_eq!(
            sorted(&rows, TITLE_COLUMN, SortDirection::Descending),
            ["akumajou", "dokyusei", "ys-2", "plain", "edge", "brandish"]
        );
    }

    /// Verifies the original title order in both directions. Games without it follow.
    #[test]
    fn original_titles_sort_with_the_unset_ones_last() {
        let rows = sample();
        assert_eq!(
            sorted(&rows, TITLE_ORIGINAL_COLUMN, SortDirection::Ascending),
            ["ys-2", "edge", "dokyusei", "brandish", "plain", "akumajou"]
        );
        assert_eq!(
            sorted(&rows, TITLE_ORIGINAL_COLUMN, SortDirection::Descending),
            ["brandish", "dokyusei", "edge", "ys-2", "plain", "akumajou"]
        );
    }

    /// Verifies the developer order in both directions. Equal developers follow the title.
    #[test]
    fn developers_sort_with_the_title_as_tie_break() {
        let rows = sample();
        assert_eq!(
            sorted(&rows, DEVELOPER_COLUMN, SortDirection::Ascending),
            ["dokyusei", "akumajou", "brandish", "ys-2", "edge", "plain"]
        );
        assert_eq!(
            sorted(&rows, DEVELOPER_COLUMN, SortDirection::Descending),
            ["edge", "brandish", "ys-2", "akumajou", "dokyusei", "plain"]
        );
    }

    /// Verifies the year order in both directions. Games without a valid year follow.
    #[test]
    fn years_sort_as_numbers_with_the_unset_ones_last() {
        let rows = sample();
        assert_eq!(
            sorted(&rows, YEAR_COLUMN, SortDirection::Ascending),
            ["ys-2", "brandish", "dokyusei", "edge", "plain", "akumajou"]
        );
        assert_eq!(
            sorted(&rows, YEAR_COLUMN, SortDirection::Descending),
            ["edge", "dokyusei", "brandish", "ys-2", "plain", "akumajou"]
        );
    }

    /// Verifies that the files with errors follow the games under a caption, by file name, and pass the filter.
    #[test]
    fn files_with_errors_follow_under_a_caption() {
        let mut rows = sample();
        rows.push(broken("zeta"));
        rows.push(broken("alpha"));
        let entries = table_entries(&rows, DEFAULT_SORT, &Filter::new(""));
        assert_eq!(
            stems(&rows, &entries),
            [
                "brandish", "edge", "plain", "ys-2", "dokyusei", "akumajou", "|", "alpha", "zeta"
            ]
        );
        assert_eq!(entries[6].kind(), RowKind::Caption);
        assert_eq!(entries[6].row(), None);
        assert_eq!(entries[7].kind(), RowKind::Normal);

        let entries = table_entries(&rows, DEFAULT_SORT, &Filter::new("ZETA"));
        assert_eq!(stems(&rows, &entries), ["|", "zeta"]);
        let entries = table_entries(&rows, DEFAULT_SORT, &Filter::new("falcom"));
        assert_eq!(stems(&rows, &entries), ["brandish", "ys-2"]);
        assert!(table_entries(&rows, DEFAULT_SORT, &Filter::new("nothing")).is_empty());
    }

    /// Verifies that the filter keeps the order of the sort.
    #[test]
    fn the_filter_keeps_the_order() {
        let rows = sample();
        let sort = Sort {
            column: YEAR_COLUMN,
            direction: SortDirection::Descending,
        };
        let entries = table_entries(&rows, sort, &Filter::new("ニホン"));
        assert!(entries.is_empty());
        let entries = table_entries(&rows, sort, &Filter::new("nihon"));
        assert_eq!(stems(&rows, &entries), ["brandish", "ys-2"]);
        let entries = table_entries(&rows, sort, &Filter::new("ドウキュウ"));
        assert_eq!(stems(&rows, &entries), ["dokyusei"]);
    }
}
