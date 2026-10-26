//! The count beside the system label of the library: the games and the
//! files with errors of the system, with the shown numbers while a filter
//! applies.

use std::fmt::{self, Write as _};

use gui::ui::TextValue;
use i18n::Catalog;

use crate::texts::library;

/// The counts beside the system label: the shown and the total number of
/// games and of files with errors, and whether a filter applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct GameCounts {
    /// The number of games that the table shows.
    pub(super) shown_games: usize,
    /// The number of games of the system.
    pub(super) games: usize,
    /// The number of files with errors that the table shows.
    pub(super) shown_errors: usize,
    /// The number of files with errors of the system.
    pub(super) errors: usize,
    /// Whether a filter applies.
    pub(super) filtering: bool,
}

impl TextValue for GameCounts {}

/// One part of the count beside the system label: the games or the files
/// with errors, with the shown number while a filter applies.
struct CountPart<'a> {
    /// The catalog of the texts.
    catalog: &'a Catalog,
    /// The number of the system.
    total: usize,
    /// The number that the table shows.
    shown: usize,
    /// Whether a filter applies.
    filtering: bool,
    /// Whether the part counts the files with errors.
    errors: bool,
}

impl fmt::Display for CountPart<'_> {
    /// Writes the count of the part in the catalog.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let total = self.total as u64;
        let shown = &self.shown;
        match (self.errors, self.filtering) {
            (false, false) => self.catalog.write(formatter, library::games(total)),
            (false, true) => self
                .catalog
                .write(formatter, library::shown_games(total, shown)),
            (true, false) => self.catalog.write(formatter, library::error_files(total)),
            (true, true) => self
                .catalog
                .write(formatter, library::shown_error_files(total, shown)),
        }
    }
}

/// Writes the count beside the system label in `catalog`: the total number
/// of games, and of the files with errors. While a filter applies the count
/// tells the shown number of the total, for example `3 of 18 games`.
pub(super) fn write_counts(catalog: &Catalog, counts: GameCounts, out: &mut String) {
    let games = CountPart {
        catalog,
        total: counts.games,
        shown: counts.shown_games,
        filtering: counts.filtering,
        errors: false,
    };
    let _ = if counts.errors == 0 {
        write!(out, "{games}")
    } else {
        let errors = CountPart {
            catalog,
            total: counts.errors,
            shown: counts.shown_errors,
            filtering: counts.filtering,
            errors: true,
        };
        catalog.write(out, library::games_and_errors(&games, &errors))
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::texts::english;

    /// Verifies the count beside the system label with and without a filter.
    #[test]
    fn the_count_tells_the_games_and_the_files_with_errors() {
        let count_text = |games: (usize, usize), errors: (usize, usize), filtering: bool| {
            let mut text = String::new();
            let counts = GameCounts {
                shown_games: games.0,
                games: games.1,
                shown_errors: errors.0,
                errors: errors.1,
                filtering,
            };
            write_counts(english(), counts, &mut text);
            text
        };
        assert_eq!(count_text((18, 18), (0, 0), false), "18 games");
        assert_eq!(count_text((1, 1), (0, 0), false), "1 game");
        assert_eq!(count_text((0, 0), (0, 0), false), "0 games");
        assert_eq!(
            count_text((5, 5), (2, 2), false),
            "5 games, 2 files with errors"
        );
        assert_eq!(count_text((0, 18), (0, 0), true), "0 of 18 games");
        assert_eq!(
            count_text((1, 5), (1, 1), true),
            "1 of 5 games, 1 of 1 file with errors"
        );
    }
}
