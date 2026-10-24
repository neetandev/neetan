//! The ROM folder of every ROM key and its ROM status.

use std::path::PathBuf;

use machine_factory::{
    config::systems::{RomFolder, rom_folders},
    rom_status::{RomCheck, check_rom_folder},
};

use crate::config::HostLayers;

/// The folder of one ROM key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RomFolderPath {
    /// The absolute folder.
    pub path: PathBuf,
    /// Whether a host layer names the folder. Otherwise it derives from the ROM folder.
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

/// Returns the folder of `folder`: the value of its key, else `<roms-folder>/<name>`.
///
/// The command line outranks the settings file. An empty value derives the folder.
#[must_use]
pub fn rom_folder_path(host: &HostLayers, folder: &RomFolder) -> RomFolderPath {
    match host
        .host_value(folder.key)
        .filter(|value| !value.is_empty())
    {
        Some(value) => RomFolderPath {
            path: PathBuf::from(value),
            explicit: true,
        },
        None => RomFolderPath {
            path: host.roms_folder().join(folder.name),
            explicit: false,
        },
    }
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
    use machine_factory::{
        config::{
            settings::find_setting,
            systems::{RomOwner, SYSTEMS},
        },
        rom_status::RomStatus,
    };

    use super::*;
    use crate::config::{LaunchRequest, Locations};

    /// Verifies the folders of the settings file and the command line, and the status of absent folders.
    #[test]
    fn host_layers_name_the_folders() {
        let root =
            std::env::temp_dir().join(format!("neetan_rom_folders_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("folder is created");
        let file = root.join("neetan.conf");
        std::fs::write(
            &file,
            "roms-folder = firmware\nx1-roms = /archive/x1\npc98-roms = /old\npc98-roms =\n\
             config-folder=games\nmeta.title = ignored\n",
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
                "/flag/config".to_owned(),
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
        assert_eq!(host.config_folder(), PathBuf::from("/flag/config"));
        assert_eq!(host.roms_folder(), root.join("firmware"));

        let x1 = SYSTEMS
            .iter()
            .find(|system| system.name == "x1")
            .expect("x1 system")
            .rom_folder();
        assert_eq!(
            rom_folder_path(&host, &x1),
            RomFolderPath {
                path: PathBuf::from("/archive/x1"),
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
        let _ = std::fs::remove_dir_all(&root);
    }
}
