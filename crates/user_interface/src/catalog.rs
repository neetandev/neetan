//! The game catalog: the game files of every system below the config folder.
//!
//! A game file lives in `<config-folder>/<system>/<game>.conf`. The catalog
//! reads the metadata and the machine of each file and checks its lines with
//! the layer reader of a launch. A file with an error cannot start its game
//! and joins the files with errors of its system. The catalog reads only the
//! configuration files. It never touches ROMs or images.

use std::path::PathBuf;

use machine_factory::config::systems::{System, compiled_systems};

pub use self::file_names::{
    COPY_TITLE_SUFFIX, FALLBACK_FILE_STEM, MAXIMUM_FILE_STEM_LENGTH, copy_title,
    new_game_file_name, title_file_stem, unique_file_name,
};
pub use crate::config::ModelSource;
use crate::config::{Diagnostic, GameMetadata, HostLayers, Severity};

mod file_names;
mod scan;

/// One game file of the catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntry {
    /// The path below the config folder with `/` separators, for example `pc98/edge.conf`.
    pub identity: String,
    /// The absolute path of the file.
    pub path: PathBuf,
    /// The file name, for example `edge.conf`.
    pub file_name: String,
    /// The title to show: `meta.title`, else the file stem.
    pub title: String,
    /// The metadata of the file.
    pub metadata: GameMetadata,
    /// The machine model, in the form of the `machine` values.
    pub model: &'static str,
    /// The source of the machine model.
    pub model_source: ModelSource,
    /// The findings about the file in line order. A finding without a line comes first.
    pub diagnostics: Vec<Diagnostic>,
}

impl CatalogEntry {
    /// Returns whether a finding about the file is an error.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.first_error().is_some()
    }

    /// Returns the first error about the file.
    #[must_use]
    pub fn first_error(&self) -> Option<&Diagnostic> {
        self.diagnostics
            .iter()
            .find(|diagnostic| diagnostic.severity == Severity::Error)
    }
}

/// The games of one system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemCatalog {
    /// The system.
    pub system: &'static System,
    /// The folder of the game files, `<config-folder>/<system>`.
    pub folder: PathBuf,
    /// The system defaults file, `<config-folder>/<system>.conf`. It may be absent.
    pub defaults_file: PathBuf,
    /// The findings about the system defaults file.
    pub defaults_diagnostics: Vec<Diagnostic>,
    /// The model of a game file without `machine`.
    pub default_model: &'static str,
    /// The source of `default_model`: the system defaults, the settings file or the built-in default.
    pub default_model_source: ModelSource,
    /// The games that can start, sorted by title and file name.
    pub games: Vec<CatalogEntry>,
    /// The files with an error, sorted by title and file name.
    pub files_with_errors: Vec<CatalogEntry>,
}

impl SystemCatalog {
    /// Returns the number of games that can start.
    #[must_use]
    pub fn game_count(&self) -> usize {
        self.games.len()
    }

    /// Returns the game with the identity `identity`, among the games and the files with errors.
    #[must_use]
    pub fn find(&self, identity: &str) -> Option<&CatalogEntry> {
        self.games
            .iter()
            .chain(&self.files_with_errors)
            .find(|entry| entry.identity == identity)
    }
}

/// The games of every system compiled into this build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalog {
    /// The config folder.
    pub config_folder: PathBuf,
    /// One catalog for each compiled system, in the order of the systems table.
    pub systems: Vec<SystemCatalog>,
}

impl Catalog {
    /// Scans the game files below the config folder of `host`.
    ///
    /// The model of each game comes from the same layers as a launch: the game
    /// file, the system defaults, the settings file and the command line.
    /// Relative paths in the files resolve against the root. A missing config
    /// folder or system folder gives a system without games.
    #[must_use]
    pub fn scan(host: &HostLayers) -> Self {
        let config_folder = host.config_folder();
        Self {
            systems: compiled_systems()
                .filter_map(|system| scan::scan_system(system, &config_folder, host))
                .collect(),
            config_folder,
        }
    }

    /// Returns the catalog of the system with the folder name `name`.
    #[must_use]
    pub fn system(&self, name: &str) -> Option<&SystemCatalog> {
        self.systems
            .iter()
            .find(|catalog| catalog.system.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Returns an entry of `pc98` with the diagnostics `diagnostics`.
    fn entry(identity: &str, diagnostics: Vec<Diagnostic>) -> CatalogEntry {
        CatalogEntry {
            identity: identity.to_owned(),
            path: PathBuf::from(identity),
            file_name: identity.to_owned(),
            title: identity.to_owned(),
            metadata: GameMetadata::default(),
            model: "PC9801RA",
            model_source: ModelSource::BuiltIn,
            diagnostics,
        }
    }

    /// Verifies the error lookup of an entry and the identity lookup of a system.
    #[test]
    fn entries_report_their_first_error() {
        let warning = Diagnostic::warning(1, "unknown key fdd3".to_owned());
        let error = Diagnostic::error(2, "unknown machine X68030".to_owned());
        let good = entry("pc98/good.conf", vec![warning.clone()]);
        let bad = entry("pc98/bad.conf", vec![warning, error.clone()]);
        assert!(!good.has_errors());
        assert_eq!(bad.first_error(), Some(&error));

        let system = compiled_systems().next().expect("one system is compiled");
        let catalog = SystemCatalog {
            system,
            folder: PathBuf::from("config/pc98"),
            defaults_file: PathBuf::from("config/pc98.conf"),
            defaults_diagnostics: Vec::new(),
            default_model: system.default_model,
            default_model_source: ModelSource::BuiltIn,
            games: vec![good],
            files_with_errors: vec![bad],
        };
        assert_eq!(catalog.game_count(), 1);
        assert_eq!(
            catalog
                .find("pc98/bad.conf")
                .map(|entry| entry.identity.as_str()),
            Some("pc98/bad.conf")
        );
        assert!(catalog.find("pc98/absent.conf").is_none());
        let full = Catalog {
            config_folder: PathBuf::from("config"),
            systems: vec![catalog],
        };
        assert!(full.system(system.name).is_some());
        assert!(full.system("absent").is_none());
    }
}
