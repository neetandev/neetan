//! Settings discovery, the root folder and the resolution of relative paths.

use std::path::{Component, Path, PathBuf};

use common::{Context, bail};
use machine_factory::config::settings::{Setting, ValueKind};

/// File name of the settings file.
pub const SETTINGS_FILE_NAME: &str = "neetan.conf";

/// Folders that the settings discovery searches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locations {
    /// The current directory of the process.
    pub current_directory: PathBuf,
    /// The folder of the executable after following symbolic links to the
    /// executable file, when it is known.
    pub executable_directory: Option<PathBuf>,
    /// The SDL pref path, when SDL provides one. It holds the global
    /// settings file.
    pub pref_path: Option<PathBuf>,
}

impl Locations {
    /// Returns the folders of the running process.
    pub fn of_process() -> crate::Result<Self> {
        let current_directory =
            std::env::current_dir().context("Failed to read the current directory")?;
        let executable_directory = std::env::current_exe()
            .and_then(std::fs::canonicalize)
            .ok()
            .and_then(|executable| executable.parent().map(Path::to_path_buf));
        let pref_path = sdl3::filesystem::get_pref_path(crate::COMPANY_NAME, crate::GAME_NAME);
        Ok(Self {
            current_directory,
            executable_directory,
            pref_path,
        })
    }
}

/// The settings file of a launch and its root folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsLocation {
    /// The settings file. `None` when the discovery found no file.
    pub file: Option<PathBuf>,
    /// The root folder. It is the folder of a settings file from `--settings`,
    /// the current directory or the folder of the executable. For the settings
    /// file of the SDL pref path and without a file it is the current directory.
    pub root: PathBuf,
}

/// Finds the settings file and the root folder.
///
/// The settings file is the first match of the explicit file, `neetan.conf` in
/// the current directory, `neetan.conf` beside the executable and `neetan.conf`
/// in the SDL pref path. An explicit file must exist. The root is the folder of
/// the settings file. For the settings file of the SDL pref path and without a
/// match the root is the current directory.
pub fn discover_settings(
    explicit: Option<&Path>,
    locations: &Locations,
) -> crate::Result<SettingsLocation> {
    if let Some(file) = explicit {
        let file = resolve_path(&locations.current_directory, file);
        if !file.is_file() {
            bail!("settings file not found: {}", file.display());
        }
        return Ok(SettingsLocation {
            root: parent_folder(&file),
            file: Some(file),
        });
    }
    let candidates = [
        Some(&locations.current_directory),
        locations.executable_directory.as_ref(),
    ];
    for folder in candidates.into_iter().flatten() {
        let file = folder.join(SETTINGS_FILE_NAME);
        if file.is_file() {
            return Ok(SettingsLocation {
                root: resolve_path(&locations.current_directory, folder),
                file: Some(file),
            });
        }
    }
    let file = locations
        .pref_path
        .as_ref()
        .map(|folder| folder.join(SETTINGS_FILE_NAME))
        .filter(|file| file.is_file());
    Ok(SettingsLocation {
        file,
        root: locations.current_directory.clone(),
    })
}

/// Returns the folder that holds `file`.
fn parent_folder(file: &Path) -> PathBuf {
    file.parent().map_or_else(PathBuf::new, Path::to_path_buf)
}

/// The resource folder of an installation relative to the folder of the
/// executable: `share/neetan` beside the `bin` folder on Linux.
#[cfg(target_os = "linux")]
const RESOURCE_FOLDER: &str = "../share/neetan";
/// The resource folder of an installation relative to the folder of the
/// executable: the `Resources` folder of the application bundle on macOS.
#[cfg(target_os = "macos")]
const RESOURCE_FOLDER: &str = "../Resources";
/// The resource folder of an installation relative to the folder of the
/// executable: the folder of the executable itself.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
const RESOURCE_FOLDER: &str = ".";

/// Returns the resource folder of the installation whose executable lies in
/// `executable_directory`. A symbolic link to the folder is followed first.
#[must_use]
pub fn resource_folder(executable_directory: &Path) -> PathBuf {
    let directory = std::fs::canonicalize(executable_directory)
        .unwrap_or_else(|_| executable_directory.to_path_buf());
    let folder = resolve_path(&directory, Path::new(RESOURCE_FOLDER));
    let mut normalized = PathBuf::new();
    for component in folder.components() {
        match component {
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other),
        }
    }
    normalized
}

/// Returns `path` as an absolute path. A relative path lies below `base`.
#[must_use]
pub fn resolve_path(base: &Path, path: &Path) -> PathBuf {
    base.join(path)
        .components()
        .filter(|component| !matches!(component, Component::CurDir))
        .collect()
}

/// Returns the value of `setting` with a relative path resolved against `base`.
///
/// An empty value and a value of a kind without a path stay as written.
#[must_use]
pub fn resolve_value(setting: &Setting, base: &Path, value: &str) -> String {
    match setting.kind {
        ValueKind::Path(_) | ValueKind::PathList if !value.is_empty() => {
            resolve_path(base, Path::new(value))
                .to_string_lossy()
                .into_owned()
        }
        ValueKind::Choice(_)
        | ValueKind::OnOff
        | ValueKind::Number(_)
        | ValueKind::Path(_)
        | ValueKind::PathList => value.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use common::temporary::TemporaryFolder;

    use super::*;
    use crate::config::test_support::TestLocations;

    /// Verifies that the resource folder follows a symbolic link to the folder of the executable.
    #[cfg(unix)]
    #[test]
    fn the_resource_folder_follows_a_linked_executable_folder() {
        let temporary = TemporaryFolder::new("resource_folder_test");
        let base = temporary.path();
        let binaries = base.join("install").join("usr").join("bin");
        std::fs::create_dir_all(&binaries).expect("folder is created");
        let link = base.join("linked-bin");
        std::os::unix::fs::symlink(&binaries, &link).expect("link is created");
        let real = std::fs::canonicalize(&binaries).expect("folder exists");
        let folder = resource_folder(&link);
        assert_eq!(folder, resource_folder(&real));
        assert!(folder.starts_with(real.parent().expect("bin has a parent")));
    }

    /// Verifies that the resource folder of an installation follows the layout of the platform.
    #[test]
    fn the_resource_folder_follows_the_platform_layout() {
        let folder = resource_folder(Path::new("/nonexistent/neetan/usr/bin"));
        #[cfg(target_os = "linux")]
        assert_eq!(
            folder,
            PathBuf::from("/nonexistent/neetan/usr/share/neetan")
        );
        #[cfg(target_os = "macos")]
        assert_eq!(folder, PathBuf::from("/nonexistent/neetan/usr/Resources"));
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        assert_eq!(folder, PathBuf::from("/nonexistent/neetan/usr/bin"));
    }

    /// Verifies that the root is the current directory without a settings file.
    #[test]
    fn a_missing_settings_file_gives_the_current_directory_as_the_root() {
        let test = TestLocations::new("discovery_missing_test");
        let locations = &test.locations;
        let found = discover_settings(None, locations).expect("discovery succeeds");
        assert_eq!(found.file, None);
        assert_eq!(found.root, locations.current_directory);

        let without_pref = Locations {
            pref_path: None,
            ..locations.clone()
        };
        let found = discover_settings(None, &without_pref).expect("discovery succeeds");
        assert_eq!(found.file, None);
        assert_eq!(found.root, locations.current_directory);
    }

    /// Verifies that the settings file of the pref path gives the current directory as the root.
    #[test]
    fn the_pref_path_settings_file_gives_the_current_directory_as_the_root() {
        let test = TestLocations::new("discovery_pref_test");
        let locations = &test.locations;
        let pref = locations
            .pref_path
            .clone()
            .expect("locations have a pref path");
        std::fs::write(pref.join(SETTINGS_FILE_NAME), "").expect("file is written");
        let found = discover_settings(None, locations).expect("discovery succeeds");
        assert_eq!(found.file, Some(pref.join(SETTINGS_FILE_NAME)));
        assert_eq!(found.root, locations.current_directory);
    }

    /// Verifies the discovery order: the explicit file, the current directory, the executable folder and the pref path.
    #[test]
    fn settings_discovery_takes_the_first_match() {
        let test = TestLocations::new("discovery_order_test");
        let locations = &test.locations;
        let current = locations.current_directory.clone();
        let executable = locations
            .executable_directory
            .clone()
            .expect("locations have an executable folder");
        let pref = locations
            .pref_path
            .clone()
            .expect("locations have a pref path");

        for (folder, root) in [
            (&pref, &current),
            (&executable, &executable),
            (&current, &current),
        ] {
            std::fs::write(folder.join(SETTINGS_FILE_NAME), "").expect("file is written");
            let found = discover_settings(None, locations).expect("discovery succeeds");
            assert_eq!(found.file, Some(folder.join(SETTINGS_FILE_NAME)));
            assert_eq!(&found.root, root);
        }

        let explicit = current.join("custom").join("other.conf");
        std::fs::create_dir_all(current.join("custom")).expect("folder is created");
        std::fs::write(&explicit, "").expect("file is written");
        let found = discover_settings(Some(Path::new("custom/other.conf")), locations)
            .expect("an explicit file wins");
        assert_eq!(found.file, Some(explicit));
        assert_eq!(found.root, current.join("custom"));

        let missing = discover_settings(Some(Path::new("missing.conf")), locations)
            .expect_err("a missing explicit file is an error");
        assert_eq!(
            missing.to_string(),
            format!(
                "settings file not found: {}",
                current.join("missing.conf").display()
            )
        );
    }

    /// Verifies that a relative path lies below the base and that `.` components go.
    #[test]
    fn relative_paths_resolve_below_the_base() {
        let base = Path::new("/games/neetan");
        assert_eq!(
            resolve_path(base, Path::new("./config/pc98")),
            PathBuf::from("/games/neetan/config/pc98")
        );
        assert_eq!(
            resolve_path(base, Path::new("/absolute/roms")),
            PathBuf::from("/absolute/roms")
        );
        assert_eq!(
            resolve_path(base, Path::new(".")),
            PathBuf::from("/games/neetan")
        );
    }
}
