//! The rows of the game list and the view of the manager that outlives a
//! game.

use std::path::{Path, PathBuf};

use gui::ui::ListState;
use machine_factory::config::systems::System;

use crate::catalog::{Catalog, CatalogEntry};

/// One game file of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GameRow {
    /// The system of the game.
    pub(crate) system: &'static System,
    /// The path below the config folder, for example `pc98/edge.conf`.
    pub(crate) identity: String,
    /// The title to show.
    pub(crate) title: String,
    /// The absolute path of the game file.
    pub(crate) path: PathBuf,
    /// The path of the game file to show: below the root when the file lies there.
    pub(crate) shown_path: String,
    /// The first error of the file with its location. A file with an error cannot start.
    pub(crate) error: Option<String>,
}

impl GameRow {
    /// Returns the row of `entry` of `system`. `root` shortens the shown path.
    fn new(system: &'static System, entry: &CatalogEntry, root: &Path) -> Self {
        let shown_path = shown_path(&entry.path, root);
        Self {
            system,
            identity: entry.identity.clone(),
            title: entry.title.clone(),
            path: entry.path.clone(),
            error: entry
                .first_error()
                .map(|diagnostic| diagnostic.located(&shown_path)),
            shown_path,
        }
    }

    /// Returns whether the game can start.
    pub(crate) fn launchable(&self) -> bool {
        self.error.is_none()
    }
}

/// Returns `path` below `root` when it lies there, else the whole path.
pub(crate) fn shown_path(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

/// Returns the rows of every game file of `catalog`: the systems in the
/// order of the systems table, in each system the games and then the files
/// with errors.
pub(crate) fn catalog_rows(catalog: &Catalog, root: &Path) -> Vec<GameRow> {
    catalog
        .systems
        .iter()
        .flat_map(|system_catalog| {
            system_catalog
                .games
                .iter()
                .chain(&system_catalog.files_with_errors)
                .map(|entry| GameRow::new(system_catalog.system, entry, root))
        })
        .collect()
}

/// The view of the manager: the selected game and the scroll position of
/// the list. It stays in memory while a game runs.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct ManagerView {
    /// The selected row and the scroll offset of the list.
    pub(crate) list: ListState,
    /// The identity of the selected game.
    pub(crate) selected: Option<String>,
}

impl ManagerView {
    /// Selects the selected game again in the new `rows`. A game that is
    /// gone leaves the selection on the nearest row. The scroll position
    /// stays while the game keeps its row.
    pub(crate) fn restore(&mut self, rows: &[GameRow]) {
        let Some(last) = rows.len().checked_sub(1) else {
            *self = Self::default();
            return;
        };
        let found = self
            .selected
            .as_ref()
            .and_then(|identity| rows.iter().position(|row| &row.identity == identity));
        match found {
            Some(index) if index == self.list.selected => {}
            Some(index) => self.list.select(index),
            None => self.list.select(self.list.selected.min(last)),
        }
        self.record(rows);
    }

    /// Records the identity of the selected row of `rows`.
    pub(crate) fn record(&mut self, rows: &[GameRow]) {
        self.selected = rows.get(self.list.selected).map(|row| row.identity.clone());
    }

    /// Returns the selected row of `rows`.
    pub(crate) fn selected_row<'a>(&self, rows: &'a [GameRow]) -> Option<&'a GameRow> {
        rows.get(self.list.selected)
    }
}

#[cfg(test)]
mod tests {
    use machine_factory::config::systems::compiled_systems;

    use super::*;

    /// Returns a launchable row of the first compiled system.
    fn row(identity: &str) -> GameRow {
        GameRow {
            system: compiled_systems().next().expect("one system is compiled"),
            identity: identity.to_owned(),
            title: identity.to_owned(),
            path: PathBuf::from("/root/config").join(identity),
            shown_path: format!("config/{identity}"),
            error: None,
        }
    }

    /// Returns rows with the identities `identities`.
    fn rows(identities: &[&str]) -> Vec<GameRow> {
        identities.iter().map(|identity| row(identity)).collect()
    }

    /// Verifies that a rescan keeps the selected game and its scroll position.
    #[test]
    fn the_view_keeps_the_selected_game_and_its_scroll_position() {
        let before = rows(&["pc98/a.conf", "pc98/b.conf", "pc98/c.conf"]);
        let mut view = ManagerView::default();
        view.list.select(1);
        view.list.scroll = 12.0;
        view.record(&before);
        assert_eq!(view.selected.as_deref(), Some("pc98/b.conf"));

        let after = before.clone();
        view.restore(&after);
        assert_eq!(view.list.selected, 1);
        assert_eq!(view.list.scroll, 12.0);

        let moved = rows(&["pc98/0.conf", "pc98/a.conf", "pc98/b.conf"]);
        view.restore(&moved);
        assert_eq!(view.list.selected, 2);
        assert_eq!(view.selected.as_deref(), Some("pc98/b.conf"));
    }

    /// Verifies that a removed game selects the nearest row and that an empty list resets the view.
    #[test]
    fn a_removed_game_selects_the_nearest_row() {
        let before = rows(&["pc98/a.conf", "pc98/b.conf", "pc98/c.conf"]);
        let mut view = ManagerView::default();
        view.list.select(2);
        view.record(&before);

        view.restore(&rows(&["pc98/a.conf", "pc98/b.conf"]));
        assert_eq!(view.list.selected, 1);
        assert_eq!(view.selected.as_deref(), Some("pc98/b.conf"));

        view.restore(&rows(&["pc98/a.conf", "pc98/x.conf"]));
        assert_eq!(view.list.selected, 1);
        assert_eq!(view.selected.as_deref(), Some("pc98/x.conf"));

        view.restore(&[]);
        assert_eq!(view, ManagerView::default());
    }

    /// Verifies that a shown path is relative to the root when the file lies below it.
    #[test]
    fn shown_paths_lie_below_the_root() {
        assert_eq!(
            shown_path(Path::new("/root/config/pc98/a.conf"), Path::new("/root")),
            "config/pc98/a.conf"
        );
        assert_eq!(
            shown_path(Path::new("/elsewhere/a.conf"), Path::new("/root")),
            "/elsewhere/a.conf"
        );
    }
}
