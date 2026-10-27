//! The ROM folder of every ROM key and its ROM status.

use std::path::{Path, PathBuf};

use machine_factory::{
    config::{
        DEFAULT_ROMS_FOLDER,
        settings::ROMS_FOLDER_KEY,
        systems::{RomFolder, rom_folders},
    },
    rom_status::{RomCheck, RomStatus, check_rom_folder},
};

use crate::config::{HostLayers, layers::folder_location, resolve_path};

/// The folder of one ROM key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RomFolderPath {
    /// The absolute folder.
    pub path: PathBuf,
    /// Whether a host layer sets the folder. Otherwise it derives from the ROM folder.
    pub explicit: bool,
}

/// The ROM status of one ROM folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RomFolderStatus {
    /// The ROM folder entry of the systems table.
    pub folder: RomFolder,
    /// The folder that the check read.
    pub location: RomFolderPath,
    /// The result of the check.
    pub check: RomCheck,
}

/// The severity of a ROM status: the mark and the color that show it.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) enum RomSeverity {
    /// Every model finds its ROM set: a check in the marker color.
    Found,
    /// Some or every model of a system that needs its ROM set lacks it: the
    /// alert icon and the danger text color.
    Warning,
    /// A ROM set that the system runs without, or a folder that this build
    /// cannot check: no mark.
    Unmarked,
}

/// Returns the severity of the ROM status `status` of a folder whose system
/// needs its ROM set when `required`.
#[must_use]
pub(crate) fn rom_severity(status: RomStatus, required: bool) -> RomSeverity {
    match status {
        RomStatus::Found => RomSeverity::Found,
        RomStatus::Partial | RomStatus::Missing if required => RomSeverity::Warning,
        RomStatus::Partial | RomStatus::Missing | RomStatus::NotFound | RomStatus::Unsupported => {
            RomSeverity::Unmarked
        }
    }
}

/// Returns the folder of `folder` below `root` with the `roms-folder` value
/// `roms_folder` and the value `value` of its ROM key: the value of its key,
/// else `<roms-folder>/<name>`. An empty value counts as unset.
#[must_use]
pub fn rom_folder_location(
    root: &Path,
    roms_folder: Option<&str>,
    value: Option<&str>,
    folder: &RomFolder,
) -> RomFolderPath {
    match value.filter(|value| !value.is_empty()) {
        Some(value) => RomFolderPath {
            path: resolve_path(root, Path::new(value)),
            explicit: true,
        },
        None => RomFolderPath {
            path: folder_location(root, roms_folder, DEFAULT_ROMS_FOLDER).join(folder.name),
            explicit: false,
        },
    }
}

/// Returns the folder of `folder`: the value of its key, else `<roms-folder>/<name>`.
///
/// The command line outranks the settings file. An empty value derives the folder.
#[must_use]
pub fn rom_folder_path(host: &HostLayers, folder: &RomFolder) -> RomFolderPath {
    rom_folder_location(
        host.root(),
        host.host_value(ROMS_FOLDER_KEY),
        host.host_value(folder.key),
        folder,
    )
}

/// Checks the ROM set of `folder`. The check reads and hashes the files of the folder once.
#[must_use]
pub fn check_folder(host: &HostLayers, folder: &RomFolder) -> RomFolderStatus {
    let location = rom_folder_path(host, folder);
    let check = check_rom_folder(folder, &location.path);
    RomFolderStatus {
        folder: *folder,
        location,
        check,
    }
}

/// Checks every ROM folder: the folder of each system, then the folders of the MIDI modules.
#[must_use]
pub fn check_all_folders(host: &HostLayers) -> Vec<RomFolderStatus> {
    rom_folders()
        .map(|folder| check_folder(host, &folder))
        .collect()
}

#[cfg(test)]
mod tests {
    use common::temporary::TemporaryFolder;
    use machine_factory::{
        config::{
            settings::find_setting,
            systems::{RomOwner, SYSTEMS},
        },
        rom_status::RomStatus,
    };

    use super::*;
    use crate::config::{LaunchRequest, Locations};

    /// Verifies the severity of each ROM status for a system that needs its ROM set and for one that runs without it.
    #[test]
    fn rom_severity_follows_the_status_and_the_need() {
        for (status, required, optional) in [
            (RomStatus::Found, RomSeverity::Found, RomSeverity::Found),
            (
                RomStatus::Partial,
                RomSeverity::Warning,
                RomSeverity::Unmarked,
            ),
            (
                RomStatus::Missing,
                RomSeverity::Warning,
                RomSeverity::Unmarked,
            ),
            (
                RomStatus::NotFound,
                RomSeverity::Unmarked,
                RomSeverity::Unmarked,
            ),
            (
                RomStatus::Unsupported,
                RomSeverity::Unmarked,
                RomSeverity::Unmarked,
            ),
        ] {
            assert_eq!(rom_severity(status, true), required, "{status:?}");
            assert_eq!(rom_severity(status, false), optional, "{status:?}");
        }
    }

    /// Verifies that an empty value derives the folder and that a relative value resolves against the root.
    #[test]
    fn empty_values_derive_and_relative_values_resolve() {
        let root = Path::new("/neetan");
        let folder = SYSTEMS[0].rom_folder();
        assert_eq!(
            rom_folder_location(root, Some(""), Some(""), &folder),
            RomFolderPath {
                path: root.join("roms").join(folder.name),
                explicit: false
            }
        );
        assert_eq!(
            rom_folder_location(root, Some("firmware"), Some("./sets/a"), &folder),
            RomFolderPath {
                path: root.join("sets").join("a"),
                explicit: true
            }
        );
        assert_eq!(
            rom_folder_location(root, Some("/archive"), None, &folder).path,
            Path::new("/archive").join(folder.name)
        );
    }

    /// Verifies the folders of the settings file and the command line, and the status of absent folders.
    #[test]
    fn host_layers_name_the_folders() {
        let folder = TemporaryFolder::new("rom_folders_test");
        let root = folder.path().to_path_buf();
        let file = root.join("neetan.conf");
        let archive = root.join("archive").join("x1");
        let flag_config = root.join("flag").join("config");
        std::fs::write(
            &file,
            format!(
                "roms-folder = firmware\nx1-roms = {}\npc98-roms = old\npc98-roms =\n\
                 config-folder=games\nmeta.title = ignored\n",
                archive.display()
            ),
        )
        .expect("file is written");
        let locations = Locations {
            current_directory: root.clone(),
            executable_directory: None,
            pref_path: None,
        };
        let request = LaunchRequest {
            settings: Some(file),
            flags: vec![(
                find_setting("config-folder").expect("config-folder is a key"),
                flag_config.to_string_lossy().into_owned(),
            )],
            ..LaunchRequest::default()
        };
        let host = HostLayers::read(&request, &locations).expect("settings read");
        assert_eq!(
            host.settings_diagnostics()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            vec!["line 4: duplicate key pc98-roms. It overrides line 3."]
        );
        assert_eq!(host.config_folder(), flag_config);
        assert_eq!(host.roms_folder(), root.join("firmware"));

        let x1 = SYSTEMS
            .iter()
            .find(|system| system.name == "x1")
            .expect("x1 system")
            .rom_folder();
        assert_eq!(
            rom_folder_path(&host, &x1),
            RomFolderPath {
                path: archive,
                explicit: true
            }
        );
        assert_eq!(
            rom_folder_path(&host, &SYSTEMS[0].rom_folder()),
            RomFolderPath {
                path: root.join("firmware").join("pc98"),
                explicit: false
            }
        );

        let statuses = check_all_folders(&host);
        assert_eq!(statuses.len(), rom_folders().count());
        for status in statuses {
            assert!(!status.check.folder_exists);
            let expected = match (status.folder.owner, status.folder.required) {
                (RomOwner::System(_), true) => [RomStatus::Missing, RomStatus::Unsupported],
                (RomOwner::System(_), false) | (RomOwner::Mt32 | RomOwner::Sc55, _) => {
                    [RomStatus::NotFound, RomStatus::Unsupported]
                }
            };
            assert!(
                expected.contains(&status.check.status),
                "{}",
                status.folder.name
            );
        }
    }
}
