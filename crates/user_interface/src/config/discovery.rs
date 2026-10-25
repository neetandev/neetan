//! Settings discovery, the root folder and the resolution of relative paths.

use std::path::{Component, Path, PathBuf};

use common::{Context, bail, warn};
use machine_factory::config::{
    is_language_tag,
    settings::{Setting, ValueKind},
};

use crate::texts::ENGLISH_TAG;

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

/// The step of the settings discovery that found the settings file.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum SettingsSource {
    /// The `--settings` file of the command line.
    CommandLine,
    /// `neetan.conf` in the current directory.
    CurrentDirectory,
    /// `neetan.conf` beside the executable.
    ExecutableFolder,
    /// `neetan.conf` in the SDL pref path.
    PrefPath,
    /// No step found a file.
    NotFound,
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
    /// The step of the discovery that found the file.
    pub source: SettingsSource,
}

impl SettingsLocation {
    /// The file that a write of the host settings writes: the settings
    /// file, or `neetan.conf` in the root without one.
    #[must_use]
    pub fn writable_file(&self) -> PathBuf {
        self.file
            .clone()
            .unwrap_or_else(|| self.root.join(SETTINGS_FILE_NAME))
    }
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
            source: SettingsSource::CommandLine,
        });
    }
    let candidates = [
        (
            Some(&locations.current_directory),
            SettingsSource::CurrentDirectory,
        ),
        (
            locations.executable_directory.as_ref(),
            SettingsSource::ExecutableFolder,
        ),
    ];
    for (folder, source) in candidates {
        let Some(folder) = folder else {
            continue;
        };
        let file = folder.join(SETTINGS_FILE_NAME);
        if file.is_file() {
            return Ok(SettingsLocation {
                root: resolve_path(&locations.current_directory, folder),
                file: Some(file),
                source,
            });
        }
    }
    let file = locations
        .pref_path
        .as_ref()
        .map(|folder| folder.join(SETTINGS_FILE_NAME))
        .filter(|file| file.is_file());
    let source = if file.is_some() {
        SettingsSource::PrefPath
    } else {
        SettingsSource::NotFound
    };
    Ok(SettingsLocation {
        file,
        root: locations.current_directory.clone(),
        source,
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

/// The folder of the language files below the root and below the resource
/// folder of the installation.
pub const LANGUAGE_FOLDER: &str = "language";

/// The file extension of a language file.
const LANGUAGE_FILE_EXTENSION: &str = "ini";

/// A language file of the user interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageFile {
    /// The language tag: the file name without its extension, for example `ja` or `pt-BR`.
    pub tag: String,
    /// The path of the file.
    pub path: PathBuf,
}

/// Returns the language files of `folders` in the order of the folders. The
/// files of one folder follow the order of their tags without regard to case.
///
/// A file is a `*.ini` file whose name is a language tag. A tag that an
/// earlier file has, compared without regard to case, and the tag `en` of the
/// embedded English texts are left out. A folder that does not read gives no
/// files.
#[must_use]
pub fn find_language_files(folders: &[PathBuf]) -> Vec<LanguageFile> {
    let mut files: Vec<LanguageFile> = Vec::new();
    for folder in folders {
        let Ok(entries) = std::fs::read_dir(folder) else {
            continue;
        };
        let mut found: Vec<LanguageFile> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_file()
                    && path.extension().is_some_and(|extension| {
                        extension.eq_ignore_ascii_case(LANGUAGE_FILE_EXTENSION)
                    })
            })
            .filter_map(|path| {
                let tag = path.file_stem()?.to_str()?.to_owned();
                if !is_language_tag(&tag) {
                    warn!(
                        "The language file {} has no language tag as its name",
                        path.display()
                    );
                    return None;
                }
                Some(LanguageFile { tag, path })
            })
            .filter(|file| !file.tag.eq_ignore_ascii_case(ENGLISH_TAG))
            .collect();
        found.sort_by(|first, second| compare_tags(&first.tag, &second.tag));
        for file in found {
            if !files
                .iter()
                .any(|known| known.tag.eq_ignore_ascii_case(&file.tag))
            {
                files.push(file);
            }
        }
    }
    files
}

/// Compares two language tags without regard to case.
#[must_use]
pub fn compare_tags(first: &str, second: &str) -> std::cmp::Ordering {
    first
        .bytes()
        .map(|byte| byte.to_ascii_lowercase())
        .cmp(second.bytes().map(|byte| byte.to_ascii_lowercase()))
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
        | ValueKind::PathList
        | ValueKind::Language => value.to_owned(),
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
        assert_eq!(found.source, SettingsSource::NotFound);
        assert_eq!(
            found.writable_file(),
            locations.current_directory.join(SETTINGS_FILE_NAME)
        );

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
        assert_eq!(found.source, SettingsSource::PrefPath);
        assert_eq!(found.writable_file(), pref.join(SETTINGS_FILE_NAME));
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

        for (folder, root, source) in [
            (&pref, &current, SettingsSource::PrefPath),
            (&executable, &executable, SettingsSource::ExecutableFolder),
            (&current, &current, SettingsSource::CurrentDirectory),
        ] {
            std::fs::write(folder.join(SETTINGS_FILE_NAME), "").expect("file is written");
            let found = discover_settings(None, locations).expect("discovery succeeds");
            assert_eq!(found.file, Some(folder.join(SETTINGS_FILE_NAME)));
            assert_eq!(&found.root, root);
            assert_eq!(found.source, source);
        }

        let explicit = current.join("custom").join("other.conf");
        std::fs::create_dir_all(current.join("custom")).expect("folder is created");
        std::fs::write(&explicit, "").expect("file is written");
        let found = discover_settings(Some(Path::new("custom/other.conf")), locations)
            .expect("an explicit file wins");
        assert_eq!(found.file, Some(explicit));
        assert_eq!(found.root, current.join("custom"));
        assert_eq!(found.source, SettingsSource::CommandLine);

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

    /// Verifies that the language files of the first folder come first, sorted by tag, and that a later folder adds only new tags.
    #[test]
    fn language_files_follow_the_folder_order() {
        let temporary = TemporaryFolder::new("language_discovery_test");
        let root = temporary.path().join("root");
        let resource = temporary.path().join("resource");
        for (folder, file) in [
            (&root, "ja.ini"),
            (&root, "de.ini"),
            (&root, "EN.ini"),
            (&root, "notes.txt"),
            (&root, "pt_BR.ini"),
            (&resource, "JA.ini"),
            (&resource, "fr.INI"),
            (&resource, "en.ini"),
        ] {
            temporary.write_file(folder.join(file), b"");
        }
        std::fs::create_dir(root.join("ko.ini")).expect("folder is created");
        let missing = temporary.path().join("missing");
        let files = find_language_files(&[missing, root.clone(), resource.clone()]);
        assert_eq!(
            files,
            [
                LanguageFile {
                    tag: "de".to_owned(),
                    path: root.join("de.ini"),
                },
                LanguageFile {
                    tag: "ja".to_owned(),
                    path: root.join("ja.ini"),
                },
                LanguageFile {
                    tag: "fr".to_owned(),
                    path: resource.join("fr.INI"),
                },
            ]
        );
        assert_eq!(compare_tags("pt-br", "PT-BR"), std::cmp::Ordering::Equal);
        assert_eq!(compare_tags("De", "fr"), std::cmp::Ordering::Less);
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
