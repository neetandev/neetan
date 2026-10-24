//! The game files of one system as the library shows them.

use std::path::{Path, PathBuf};

use machine_factory::config::systems::System;

use crate::catalog::{CatalogEntry, SystemCatalog};

/// One game file of the library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GameRow {
    /// The system of the game.
    pub(crate) system: &'static System,
    /// The path below the config folder, for example `pc98/edge.conf`.
    pub(crate) identity: String,
    /// The file name, for example `edge.conf`.
    pub(crate) file_name: String,
    /// The title to show: `meta.title`, else the file stem.
    pub(crate) title: String,
    /// The title in its original script, `meta.title-original`.
    pub(crate) title_original: String,
    /// The developer, `meta.developer`.
    pub(crate) developer: String,
    /// The release year as the file writes it, `meta.year`.
    pub(crate) year_text: String,
    /// The release year, when `meta.year` holds a valid year.
    pub(crate) year: Option<u16>,
    /// The absolute path of the game file.
    pub(crate) path: PathBuf,
    /// The path of the game file to show: below the root when the file lies there.
    pub(crate) shown_path: String,
    /// The first error of the file without its location. A file with an error cannot start.
    pub(crate) error: Option<String>,
    /// The first error of the file with the shown path and the line.
    pub(crate) located_error: Option<String>,
}

impl GameRow {
    /// Returns the row of `entry` of `system`. `root` shortens the shown path.
    pub(crate) fn new(system: &'static System, entry: &CatalogEntry, root: &Path) -> Self {
        let shown_path = shown_path(&entry.path, root);
        let first_error = entry.first_error();
        Self {
            system,
            identity: entry.identity.clone(),
            file_name: entry.file_name.clone(),
            title: entry.title.clone(),
            title_original: entry.metadata.title_original.clone(),
            developer: entry.metadata.developer.clone(),
            year_text: entry.metadata.year.trim().to_owned(),
            year: entry.metadata.year(),
            path: entry.path.clone(),
            error: first_error.map(ToString::to_string),
            located_error: first_error.map(|diagnostic| diagnostic.located(&shown_path)),
            shown_path,
        }
    }

    /// Returns whether the game can start.
    pub(crate) fn launchable(&self) -> bool {
        self.error.is_none()
    }

    /// The name of the row in a question: the title of a game, the file
    /// name of a file with an error.
    pub(crate) fn name(&self) -> &str {
        if self.launchable() {
            &self.title
        } else {
            &self.file_name
        }
    }
}

/// Returns `path` below `root` when it lies there, else the whole path.
pub(crate) fn shown_path(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

/// Returns the rows of the games and the files with errors of `catalog`.
pub(crate) fn system_rows(catalog: &SystemCatalog, root: &Path) -> Vec<GameRow> {
    catalog
        .games
        .iter()
        .chain(&catalog.files_with_errors)
        .map(|entry| GameRow::new(catalog.system, entry, root))
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use machine_factory::config::systems::compiled_systems;

    use super::*;

    /// Returns a launchable row of the first compiled system with the file
    /// stem `stem`, the title `title`, the original title `original`, the
    /// developer `developer` and the year `year`.
    pub(crate) fn game(
        stem: &str,
        title: &str,
        original: &str,
        developer: &str,
        year: &str,
    ) -> GameRow {
        let system = compiled_systems().next().expect("one system is compiled");
        let identity = format!("{}/{stem}.conf", system.name);
        GameRow {
            system,
            file_name: format!("{stem}.conf"),
            title: if title.is_empty() { stem } else { title }.to_owned(),
            title_original: original.to_owned(),
            developer: developer.to_owned(),
            year_text: year.to_owned(),
            year: year.parse().ok(),
            path: PathBuf::from("/root/config").join(&identity),
            shown_path: format!("config/{identity}"),
            identity,
            error: None,
            located_error: None,
        }
    }

    /// Returns a row of the first compiled system with the file stem `stem` and an error.
    pub(crate) fn broken(stem: &str) -> GameRow {
        let row = game(stem, "", "", "", "");
        GameRow {
            error: Some("line 1: unknown machine X68030".to_owned()),
            located_error: Some(format!("{} line 1: unknown machine X68030", row.shown_path)),
            ..row
        }
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

    /// Verifies that a question shows the title of a game and the file name of a file with an error.
    #[test]
    fn a_question_shows_the_title_or_the_file_name() {
        assert_eq!(game("edge", "Edge", "", "", "").name(), "Edge");
        assert_eq!(game("edge", "", "", "", "").name(), "edge");
        assert_eq!(broken("nemesis-90").name(), "nemesis-90.conf");
    }
}
