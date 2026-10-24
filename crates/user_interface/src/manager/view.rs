//! The view of the library that outlives a game: the selected system, the
//! selected game, the sort, the filter text and the scroll position.

use gui::ui::{ListState, TableState};

use super::{
    entries::{DEFAULT_SORT, TableEntry},
    rows::GameRow,
};

/// The view of the library: the selection and the scroll offset of the
/// system list and of the game table, the selected system and game, the
/// sort and the filter text. It is runtime state and is never written to
/// disk.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ManagerView {
    /// The selection and the scroll offset of the system list.
    pub(crate) systems: ListState,
    /// The folder name of the selected system, for example `pc98`.
    pub(crate) system: Option<&'static str>,
    /// The selected entry, the scroll offset, the sort and the picked column of the game table.
    pub(crate) table: TableState,
    /// The identity of the selected game, for example `pc98/edge.conf`.
    pub(crate) selected: Option<String>,
    /// The text of the filter field.
    pub(crate) filter: String,
}

impl Default for ManagerView {
    fn default() -> Self {
        Self {
            systems: ListState::new(),
            system: None,
            table: TableState {
                sort: Some(DEFAULT_SORT),
                ..TableState::new()
            },
            selected: None,
            filter: String::new(),
        }
    }
}

/// The identity of the row of the entry `index` of `entries`.
fn identity<'a>(entries: &[TableEntry], rows: &'a [GameRow], index: usize) -> Option<&'a str> {
    let row = entries.get(index)?.row()?;
    rows.get(row).map(|row| row.identity.as_str())
}

impl ManagerView {
    /// Selects the system with the folder name `name` at `index` of the
    /// system list. The game table starts at its top.
    pub(crate) fn select_system(&mut self, index: usize, name: &'static str) {
        if self.systems.selected != index {
            self.systems.select(index);
        }
        self.system = Some(name);
        self.table.list = ListState::new();
        self.selected = None;
    }

    /// Restores the selected system among the folder names `names` of the
    /// system list. A system that is gone leaves the selection on the
    /// nearest system. Returns the index of the selected system.
    pub(crate) fn restore_system(&mut self, names: &[&'static str]) -> usize {
        let last = names.len().saturating_sub(1);
        let index = self
            .system
            .and_then(|system| names.iter().position(|name| *name == system))
            .unwrap_or_else(|| self.systems.selected.min(last));
        if index != self.systems.selected {
            self.systems.select(index);
        }
        self.system = names.get(index).copied();
        index
    }

    /// Selects the selected game again in the new `entries` of `rows`. A
    /// game that is gone leaves the selection on the nearest entry. On the
    /// caption the selection takes the game before it, or the entry after
    /// it without one. The scroll position stays while the game keeps its
    /// entry.
    pub(crate) fn restore(&mut self, entries: &[TableEntry], rows: &[GameRow]) {
        let Some(last) = entries.len().checked_sub(1) else {
            self.table.list = ListState::new();
            self.selected = None;
            return;
        };
        let found = self.selected_entry(entries, rows);
        match found {
            Some(index) if index == self.table.list.selected => {}
            Some(index) => self.table.list.select(index),
            None => {
                let nearest = self.table.list.selected.min(last);
                let index = match entries[nearest] {
                    TableEntry::Caption if nearest > 0 => nearest - 1,
                    TableEntry::Caption => (nearest + 1).min(last),
                    TableEntry::Game(_) | TableEntry::Broken(_) => nearest,
                };
                self.table.list.select(index);
            }
        }
        self.record(entries, rows);
    }

    /// Selects the selected game again in the new `entries` of `rows` after
    /// a change of the filter. A game that the filter hides leaves the
    /// selection on the first entry.
    pub(crate) fn refilter(&mut self, entries: &[TableEntry], rows: &[GameRow]) {
        let found = self.selected_entry(entries, rows);
        match found {
            Some(index) => self.table.list.select(index),
            None => {
                self.table.list = ListState::new();
                self.selected = None;
            }
        }
        self.record(entries, rows);
    }

    /// The index of the entry of `entries` of `rows` that holds the
    /// selected game.
    fn selected_entry(&self, entries: &[TableEntry], rows: &[GameRow]) -> Option<usize> {
        let selected = self.selected.as_deref()?;
        (0..entries.len()).find(|&index| identity(entries, rows, index) == Some(selected))
    }

    /// Records the identity of the selected entry of `entries` of `rows`.
    /// A caption keeps the recorded game.
    pub(crate) fn record(&mut self, entries: &[TableEntry], rows: &[GameRow]) {
        match entries.get(self.table.list.selected) {
            Some(TableEntry::Caption) => {}
            Some(_) => {
                self.selected =
                    identity(entries, rows, self.table.list.selected).map(str::to_owned);
            }
            None => self.selected = None,
        }
    }

    /// The row of the selected entry of `entries` of `rows`.
    pub(crate) fn selected_row<'a>(
        &self,
        entries: &[TableEntry],
        rows: &'a [GameRow],
    ) -> Option<&'a GameRow> {
        let row = entries.get(self.table.list.selected)?.row()?;
        rows.get(row)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::{
        entries::table_entries,
        filter::Filter,
        rows::tests::{broken, game},
    };

    /// Returns rows with the file stems `stems`.
    fn rows(stems: &[&str]) -> Vec<GameRow> {
        stems
            .iter()
            .map(|stem| game(stem, "", "", "", ""))
            .collect()
    }

    /// Returns the entries of `rows` in the default order without a filter.
    fn entries(rows: &[GameRow]) -> Vec<TableEntry> {
        table_entries(rows, DEFAULT_SORT, &Filter::new(""))
    }

    /// Verifies that a new view sorts by title and that a rescan keeps the selected game and its scroll position.
    #[test]
    fn the_view_keeps_the_selected_game_and_its_scroll_position() {
        let mut view = ManagerView::default();
        assert_eq!(view.table.sort, Some(DEFAULT_SORT));
        let before = rows(&["a", "b", "c"]);
        view.table.list.select(1);
        view.table.list.scroll = 12.0;
        view.record(&entries(&before), &before);
        assert!(
            view.selected
                .as_deref()
                .is_some_and(|id| id.ends_with("/b.conf"))
        );

        view.restore(&entries(&before), &before);
        assert_eq!(view.table.list.selected, 1);
        assert_eq!(view.table.list.scroll, 12.0);

        let shifted = rows(&["0", "a", "b"]);
        view.restore(&entries(&shifted), &shifted);
        assert_eq!(view.table.list.selected, 2);
        assert!(
            view.selected
                .as_deref()
                .is_some_and(|id| id.ends_with("/b.conf"))
        );
    }

    /// Verifies that a deleted game selects the nearest entry and that an empty table resets the selection.
    #[test]
    fn a_deleted_game_selects_the_nearest_entry() {
        let before = rows(&["a", "b", "c"]);
        let mut view = ManagerView::default();
        view.table.list.select(2);
        view.record(&entries(&before), &before);

        let after = rows(&["a", "b"]);
        view.restore(&entries(&after), &after);
        assert_eq!(view.table.list.selected, 1);
        assert!(
            view.selected
                .as_deref()
                .is_some_and(|id| id.ends_with("/b.conf"))
        );

        let mut with_errors = rows(&["a"]);
        with_errors.push(broken("x"));
        view.restore(&entries(&with_errors), &with_errors);
        assert_eq!(view.table.list.selected, 0);
        assert!(
            view.selected
                .as_deref()
                .is_some_and(|id| id.ends_with("/a.conf"))
        );

        let only_errors = vec![broken("x"), broken("y")];
        view.selected = Some("gone".to_owned());
        view.table.list.select(0);
        view.restore(&entries(&only_errors), &only_errors);
        assert_eq!(view.table.list.selected, 1);
        assert!(
            view.selected
                .as_deref()
                .is_some_and(|id| id.ends_with("/x.conf"))
        );

        view.restore(&[], &[]);
        assert_eq!(view.table.list.selected, 0);
        assert_eq!(view.selected, None);
    }

    /// Verifies that a game that a changed filter result hides leaves the selection on the nearest entry after a rescan and on the first entry after a filter change.
    #[test]
    fn a_hidden_game_gives_the_selection_to_another_entry() {
        let mut list = rows(&["a", "b", "c", "d"]);
        let mut view = ManagerView::default();
        let filtered = |list: &[GameRow], view: &ManagerView| {
            table_entries(list, DEFAULT_SORT, &Filter::new(&view.filter))
        };
        view.table.list.select(2);
        view.record(&filtered(&list, &view), &list);
        assert!(
            view.selected
                .as_deref()
                .is_some_and(|id| id.ends_with("/c.conf"))
        );

        list[2].title = "xyz".to_owned();
        view.filter = "b".to_owned();
        view.refilter(&filtered(&list, &view), &list);
        assert_eq!(view.table.list.selected, 0);
        assert!(
            view.selected
                .as_deref()
                .is_some_and(|id| id.ends_with("/b.conf"))
        );

        view.filter = String::new();
        view.table.list.select(3);
        view.record(&filtered(&list, &view), &list);
        assert!(
            view.selected
                .as_deref()
                .is_some_and(|id| id.ends_with("/c.conf"))
        );
        view.filter = "d".to_owned();
        let shown = filtered(&list, &view);
        assert_eq!(shown.len(), 1);
        view.restore(&shown, &list);
        assert_eq!(view.table.list.selected, 0);
        assert!(
            view.selected
                .as_deref()
                .is_some_and(|id| id.ends_with("/d.conf"))
        );
    }

    /// Verifies that a system that is gone selects the nearest system and that a known system keeps its place.
    #[test]
    fn the_selected_system_returns_by_name() {
        let mut view = ManagerView::default();
        view.select_system(2, "pc88va");
        assert_eq!(view.restore_system(&["pc98", "pc88", "pc88va"]), 2);
        assert_eq!(view.restore_system(&["pc88va", "x1"]), 0);
        assert_eq!(view.systems.selected, 0);
        view.system = Some("gone");
        view.systems.select(5);
        assert_eq!(view.restore_system(&["pc98", "pc88"]), 1);
        assert_eq!(view.system, Some("pc88"));
    }
}
