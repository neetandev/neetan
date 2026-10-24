//! Shared helpers of the configuration tests: temporary locations and file writers.

use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

use super::{
    Action, EmulatorConfig, LaunchRequest, Locations, build_configuration, parse_args_from,
};
use crate::input::KeyOverrides;

/// Counter that keeps the temporary folders of the tests apart.
static FOLDER_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// The temporary locations of one test. Dropping the value removes its folders.
pub(crate) struct TestLocations {
    /// The folder that holds every folder of the locations.
    pub(crate) base: PathBuf,
    /// The current directory, the executable folder and the pref path below `base`.
    pub(crate) locations: Locations,
}

impl TestLocations {
    /// Creates empty folders for a new set of locations.
    pub(crate) fn new(name: &str) -> Self {
        let base = std::env::temp_dir().join(format!(
            "neetan_config_test_{}_{}_{name}",
            std::process::id(),
            FOLDER_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&base);
        let locations = Locations {
            current_directory: base.join("current"),
            executable_directory: Some(base.join("executable")),
            pref_path: Some(base.join("pref")),
        };
        for folder in [
            Some(&locations.current_directory),
            locations.executable_directory.as_ref(),
            locations.pref_path.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            std::fs::create_dir_all(folder).expect("temporary folder is created");
        }
        Self { base, locations }
    }

    /// Returns the current directory.
    pub(crate) fn current(&self) -> &Path {
        &self.locations.current_directory
    }

    /// Returns the pref path.
    pub(crate) fn pref(&self) -> &Path {
        self.locations
            .pref_path
            .as_deref()
            .expect("test locations have a pref path")
    }

    /// Parses `args` into a run configuration.
    pub(crate) fn parse(&self, args: &[&str]) -> (EmulatorConfig, KeyOverrides) {
        match parse_args_from(args.iter().map(|arg| (*arg).to_owned()), &self.locations)
            .expect("arguments should parse")
        {
            Action::Run(config, key_map) => (*config, key_map),
            _ => panic!("expected run action"),
        }
    }

    /// Returns the error of an argument list that must fail to parse.
    pub(crate) fn error(&self, args: &[&str]) -> String {
        match parse_args_from(args.iter().map(|arg| (*arg).to_owned()), &self.locations) {
            Ok(_) => panic!("{args:?} should fail to parse"),
            Err(error) => error.to_string(),
        }
    }

    /// Writes `contents` to `name` in the pref path and returns its path.
    pub(crate) fn file(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.pref().join(name);
        write_file(&path, contents);
        path
    }

    /// Parses a settings file in the pref path that holds `contents`.
    pub(crate) fn parse_settings(&self, name: &str, contents: &str) -> EmulatorConfig {
        let path = self.file(name, contents);
        self.parse(&["--settings", &path.to_string_lossy()]).0
    }

    /// Parses a game file in the pref path that holds `contents`.
    pub(crate) fn parse_game(&self, name: &str, contents: &str) -> (EmulatorConfig, KeyOverrides) {
        let path = self.file(name, contents);
        self.parse(&["--config", &path.to_string_lossy()])
    }

    /// Builds the layered configuration of `request`.
    pub(crate) fn build(&self, request: &LaunchRequest) -> EmulatorConfig {
        build_configuration(request, &self.locations)
            .expect("configuration builds")
            .0
    }
}

impl Drop for TestLocations {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

/// Parses `args` with fresh locations into a run configuration.
pub(crate) fn parse_run_config(args: &[&str]) -> (EmulatorConfig, KeyOverrides) {
    TestLocations::new("run").parse(args)
}

/// Returns the error of an argument list that must fail to parse with fresh locations.
pub(crate) fn parse_error(args: &[&str]) -> String {
    TestLocations::new("error").error(args)
}

/// Writes `contents` to `path`, creating its folder.
pub(crate) fn write_file(path: &Path, contents: &str) {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).expect("folder is created");
    }
    std::fs::write(path, contents).expect("file is written");
}

/// Creates an empty file at each path.
pub(crate) fn touch(paths: &[&Path]) {
    for path in paths {
        write_file(path, "");
    }
}
