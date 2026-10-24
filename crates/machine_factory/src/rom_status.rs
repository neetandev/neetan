//! The ROM status of a ROM folder: whether the machines of a system or a MIDI
//! module find their ROM set in it.
//!
//! A system checks the ROM set of each of its models with the ROM loader of its
//! machine crate. The loaders identify the files by their content hash. The
//! check reads and hashes every file of the folder once.

use std::path::Path;

use crate::config::{
    EmulatorConfig, MachineSelection, Target,
    settings::{Families, MODEL_CHOICES},
    systems::{RomFolder, RomOwner},
};

/// The status of one ROM folder.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum RomStatus {
    /// Every model finds its ROM set.
    Found,
    /// Some models find their ROM set and the others lack it.
    Partial,
    /// No model finds the ROM set that it needs to start.
    Missing,
    /// No model finds its ROM set. The machines run without it, for example on the HLE BIOS.
    NotFound,
    /// This build has no support for the reader of the folder.
    Unsupported,
}

/// The result of a ROM folder check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RomCheck {
    /// The status of the folder.
    pub status: RomStatus,
    /// Whether the folder exists.
    pub folder_exists: bool,
    /// The models that lack their ROM set, in the form of the `machine` values. Empty for a MIDI module.
    pub models_without_set: Vec<&'static str>,
}

/// One model of a family with its resolved configuration.
type ResolvedModel = (&'static str, EmulatorConfig);

/// Checks whether the reader of `folder` finds its ROM set in `path`.
///
/// A system is `Found` when every model of its family finds its ROM set and
/// `Partial` when only some models find it. When no model finds it, the
/// folder is `Missing` for a system that needs the set and `NotFound` for a
/// system that runs without it. A MIDI module is `Found` or `NotFound`.
#[must_use]
pub fn check_rom_folder(folder: &RomFolder, path: &Path) -> RomCheck {
    let folder_exists = path.is_dir();
    let midi_present = match folder.owner {
        RomOwner::System(family) => return check_system_folder(folder, family, path),
        RomOwner::Mt32 => mt32_rom_set_present(path),
        RomOwner::Sc55 => sc55_rom_set_present(path),
    };
    let status = match midi_present {
        Some(true) => RomStatus::Found,
        Some(false) => RomStatus::NotFound,
        None => RomStatus::Unsupported,
    };
    RomCheck {
        status,
        folder_exists,
        models_without_set: Vec::new(),
    }
}

/// Checks the ROM folder of the system of `family`.
fn check_system_folder(folder: &RomFolder, family: Families, path: &Path) -> RomCheck {
    let folder_exists = path.is_dir();
    let models: Vec<ResolvedModel> = MODEL_CHOICES
        .iter()
        .filter(|choice| choice.families == family)
        .filter_map(|choice| {
            let model = choice.value.parse::<MachineSelection>().ok()?;
            Some((choice.value, EmulatorConfig::for_model(model)))
        })
        .collect();
    let Some((_, first)) = models.first() else {
        return RomCheck {
            status: RomStatus::Unsupported,
            folder_exists,
            models_without_set: Vec::new(),
        };
    };
    let models_without_set = if folder_exists {
        models_without_set(first.target, &models, path)
    } else {
        models.iter().map(|(name, _)| *name).collect()
    };
    let status = system_status(models_without_set.len(), models.len(), folder.required);
    RomCheck {
        status,
        folder_exists,
        models_without_set,
    }
}

/// Returns the models whose ROM set the scan `index` lacks. A failed scan lacks every set.
fn lacking<Index, Error>(
    models: &[ResolvedModel],
    index: Result<Index, Error>,
    finds: impl Fn(&EmulatorConfig, &Index) -> bool,
) -> Vec<&'static str> {
    models
        .iter()
        .filter(|(_, config)| match &index {
            Ok(index) => !finds(config, index),
            Err(_) => true,
        })
        .map(|(name, _)| *name)
        .collect()
}

/// Returns the status of a system folder in which `lacking` of `models` models lack their set.
fn system_status(lacking: usize, models: usize, required: bool) -> RomStatus {
    if lacking == 0 {
        RomStatus::Found
    } else if lacking < models {
        RomStatus::Partial
    } else if required {
        RomStatus::Missing
    } else {
        RomStatus::NotFound
    }
}

/// Returns the models of the family `target` that lack their ROM set in `path`.
///
/// A family whose models share one ROM set loads it once. A PC-98 model needs
/// its font ROM and, when it boots a real BIOS, its BIOS chips.
fn models_without_set(target: Target, models: &[ResolvedModel], path: &Path) -> Vec<&'static str> {
    match target {
        #[cfg(feature = "pc98")]
        Target::Pc98 => lacking(
            models,
            machine_98::scan_rom_directory(path),
            |config, index| {
                machine_98::load_rom_set_from_index(config.machine, index).is_ok_and(|roms| {
                    roms.font.is_some()
                        && (roms.bios.is_some()
                            || machine_98::required_mame_set(config.machine).is_none())
                })
            },
        ),
        #[cfg(feature = "pc88")]
        Target::Pc88 => lacking(models, machine_88::load_rom_set(path), |_, _| true),
        #[cfg(feature = "pc88va")]
        Target::Pc88Va => lacking(models, machine_88va::load_rom_set(path), |_, _| true),
        #[cfg(feature = "pc60")]
        Target::Pc60 => lacking(
            models,
            machine_60::scan_rom_directory(path),
            |config, index| machine_60::load_rom_set_from_index(config.pc60_model, index).is_ok(),
        ),
        #[cfg(feature = "msx")]
        Target::Msx => lacking(
            models,
            machine_msx::scan_firmware_directory(path),
            |config, index| machine_msx::load_firmware_from_index(config.msx_model, index).is_ok(),
        ),
        #[cfg(feature = "towns")]
        Target::Towns => lacking(
            models,
            machine_towns::scan_rom_directory(path),
            |config, index| {
                machine_towns::load_rom_set_from_index(config.towns_model, index).is_ok()
            },
        ),
        #[cfg(feature = "x1")]
        Target::X1 => lacking(
            models,
            machine_x1::scan_rom_directory(path),
            |config, index| machine_x1::load_rom_set_from_index(config.x1_model, index).is_ok(),
        ),
        #[cfg(feature = "fm7")]
        Target::Fm7 => lacking(
            models,
            machine_fm7::scan_rom_directory(path),
            |config, index| machine_fm7::load_rom_set_from_index(config.fm7_model, index).is_ok(),
        ),
        #[cfg(feature = "x68k")]
        Target::X68k => lacking(
            models,
            machine_x68k::scan_rom_directory(path),
            |config, index| machine_x68k::load_rom_set_from_index(config.x68k_model, index).is_ok(),
        ),
        #[cfg(feature = "at")]
        Target::At => lacking(models, machine_at::load_rom_set(path), |_, _| true),
    }
}

/// Returns whether `path` holds an MT-32 ROM set. `None` when the build lacks MT-32 support.
#[cfg(feature = "mt32")]
fn mt32_rom_set_present(path: &Path) -> Option<bool> {
    Some(path.is_dir() && device::mt32::rom_set_present(path))
}

/// Returns whether `path` holds an MT-32 ROM set. `None` when the build lacks MT-32 support.
#[cfg(not(feature = "mt32"))]
fn mt32_rom_set_present(_path: &Path) -> Option<bool> {
    None
}

/// Returns whether `path` holds an SC-55 ROM set. `None` when the build lacks SC-55 support.
#[cfg(feature = "sc55")]
fn sc55_rom_set_present(path: &Path) -> Option<bool> {
    Some(path.is_dir() && device::sc55::rom_set_present(path))
}

/// Returns whether `path` holds an SC-55 ROM set. `None` when the build lacks SC-55 support.
#[cfg(not(feature = "sc55"))]
fn sc55_rom_set_present(_path: &Path) -> Option<bool> {
    None
}

#[cfg(test)]
mod tests {
    use common::temporary::TemporaryFolder;

    use super::*;
    use crate::config::systems::{MIDI_ROM_FOLDERS, SYSTEMS, compiled_systems};

    /// Verifies the status rule on folders without a ROM set.
    #[test]
    fn folders_without_a_set_follow_the_required_flag() {
        let empty = TemporaryFolder::new("rom_status_test_empty");
        std::fs::write(empty.path().join("stray.bin"), [0u8; 16]).expect("file is written");
        let absent = empty.path().join("absent");
        for system in compiled_systems() {
            let folder = system.rom_folder();
            for (path, exists) in [(empty.path(), true), (absent.as_path(), false)] {
                let check = check_rom_folder(&folder, path);
                let expected = if folder.required {
                    RomStatus::Missing
                } else {
                    RomStatus::NotFound
                };
                assert_eq!(check.status, expected, "{}", system.name);
                assert_eq!(check.folder_exists, exists, "{}", system.name);
                let models = MODEL_CHOICES
                    .iter()
                    .filter(|choice| choice.families == system.family)
                    .count();
                assert_eq!(check.models_without_set.len(), models, "{}", system.name);
            }
        }
        for folder in MIDI_ROM_FOLDERS {
            let check = check_rom_folder(&folder, empty.path());
            assert!(
                matches!(check.status, RomStatus::NotFound | RomStatus::Unsupported),
                "{}",
                folder.name
            );
            assert!(check.models_without_set.is_empty());
        }
    }

    /// Verifies the status rule of a system folder.
    #[test]
    fn status_counts_the_models_without_a_set() {
        assert_eq!(system_status(0, 3, true), RomStatus::Found);
        assert_eq!(system_status(0, 3, false), RomStatus::Found);
        assert_eq!(system_status(1, 3, true), RomStatus::Partial);
        assert_eq!(system_status(2, 3, false), RomStatus::Partial);
        assert_eq!(system_status(3, 3, true), RomStatus::Missing);
        assert_eq!(system_status(3, 3, false), RomStatus::NotFound);
    }

    /// Verifies that the SC-55 check finds a set by its file names.
    #[cfg(feature = "sc55")]
    #[test]
    fn sc55_set_is_found_by_its_file_names() {
        let folder = TemporaryFolder::new("rom_status_test_sc55");
        let sc55 = MIDI_ROM_FOLDERS
            .iter()
            .find(|folder| folder.owner == RomOwner::Sc55)
            .expect("SC-55 folder");
        assert_eq!(
            check_rom_folder(sc55, folder.path()).status,
            RomStatus::NotFound
        );
        for name in [
            "sc55_rom1.bin",
            "sc55_rom2.bin",
            "sc55_waverom1.bin",
            "sc55_waverom2.bin",
            "sc55_waverom3.bin",
        ] {
            std::fs::write(folder.path().join(name), [0u8; 4]).expect("file is written");
        }
        let check = check_rom_folder(sc55, folder.path());
        assert_eq!(check.status, RomStatus::Found);
        assert!(check.folder_exists);
    }

    /// Verifies that a system outside the build reports no support.
    #[test]
    fn systems_outside_the_build_are_unsupported() {
        let empty = TemporaryFolder::new("rom_status_test_unsupported");
        let compiled = Families::compiled();
        for system in SYSTEMS
            .iter()
            .filter(|system| !compiled.includes(system.family))
        {
            let check = check_rom_folder(&system.rom_folder(), empty.path());
            assert_eq!(check.status, RomStatus::Unsupported, "{}", system.name);
        }
    }
}
