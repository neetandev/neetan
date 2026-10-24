//! The systems table: the folder name, display name, ROM key and default model
//! of each machine family, plus the ROM folders of the MIDI modules.

use std::path::{Path, PathBuf};

use super::{
    EmulatorConfig, Target,
    settings::{Families, find_setting},
};

/// The reader of a ROM folder.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum RomOwner {
    /// The machines of one family.
    System(Families),
    /// The Roland MT-32 sound module.
    Mt32,
    /// The Roland SC-55 sound module.
    Sc55,
}

/// One folder below the ROM folder and the key that names it.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct RomFolder {
    /// The reader of the ROM set.
    pub owner: RomOwner,
    /// The folder name below the ROM folder, for example `pc98`.
    pub name: &'static str,
    /// The display name of the system or module that reads the ROM set.
    pub label: &'static str,
    /// The settings key of the folder, for example `pc98-roms`.
    pub key: &'static str,
    /// Whether the machine needs the ROM set to start.
    pub required: bool,
}

/// One emulated system: a machine family with its folder name and default model.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct System {
    /// The family of the system.
    pub family: Families,
    /// The folder name of the system below the config folder and the ROM folder.
    pub name: &'static str,
    /// The display name of the system.
    pub label: &'static str,
    /// The settings key of the ROM folder of the system.
    pub rom_key: &'static str,
    /// Whether the machines of the system need their ROM set to start.
    pub roms_required: bool,
    /// The built-in model of the system in the form of the `machine` values.
    pub default_model: &'static str,
}

impl System {
    /// Returns the ROM folder of the system.
    #[must_use]
    pub const fn rom_folder(&self) -> RomFolder {
        RomFolder {
            owner: RomOwner::System(self.family),
            name: self.name,
            label: self.label,
            key: self.rom_key,
            required: self.roms_required,
        }
    }
}

/// The PC-9800 series.
pub const PC98_SYSTEM: System = System {
    family: Families::PC98,
    name: "pc98",
    label: "PC-98",
    rom_key: "pc98-roms",
    roms_required: false,
    default_model: "PC9801RA",
};

/// The PC-8801 series.
pub const PC88_SYSTEM: System = System {
    family: Families::PC88,
    name: "pc88",
    label: "PC-88",
    rom_key: "pc88-roms",
    roms_required: true,
    default_model: "PC8801MC",
};

/// The PC-88VA series.
pub const PC88VA_SYSTEM: System = System {
    family: Families::PC88VA,
    name: "pc88va",
    label: "PC-88VA",
    rom_key: "pc88va-roms",
    roms_required: true,
    default_model: "PC88VA2",
};

/// The PC-6000 / PC-6600 series.
pub const PC6000_SYSTEM: System = System {
    family: Families::PC60,
    name: "pc6000",
    label: "PC-6000",
    rom_key: "pc6000-roms",
    roms_required: true,
    default_model: "PC6001",
};

/// The MSX series.
pub const MSX_SYSTEM: System = System {
    family: Families::MSX,
    name: "msx",
    label: "MSX",
    rom_key: "msx-roms",
    roms_required: true,
    default_model: "MSX",
};

/// The FM Towns series.
pub const TOWNS_SYSTEM: System = System {
    family: Families::TOWNS,
    name: "towns",
    label: "FM Towns",
    rom_key: "towns-roms",
    roms_required: true,
    default_model: "FMTownsIIMX",
};

/// The Sharp X1 series.
pub const X1_SYSTEM: System = System {
    family: Families::X1,
    name: "x1",
    label: "X1",
    rom_key: "x1-roms",
    roms_required: true,
    default_model: "X1",
};

/// The Fujitsu FM-7 series.
pub const FM7_SYSTEM: System = System {
    family: Families::FM7,
    name: "fm7",
    label: "FM-7",
    rom_key: "fm7-roms",
    roms_required: true,
    default_model: "FM7",
};

/// The Sharp X68000 series.
pub const X68K_SYSTEM: System = System {
    family: Families::X68K,
    name: "x68k",
    label: "X68000",
    rom_key: "x68k-roms",
    roms_required: true,
    default_model: "X68000",
};

/// The IBM PC/AT series.
pub const AT_SYSTEM: System = System {
    family: Families::AT,
    name: "at",
    label: "PC/AT",
    rom_key: "at-roms",
    roms_required: false,
    default_model: "AT486DX66",
};

/// Every system, in display order.
pub const SYSTEMS: [System; 10] = [
    PC98_SYSTEM,
    PC88_SYSTEM,
    PC88VA_SYSTEM,
    PC6000_SYSTEM,
    X68K_SYSTEM,
    TOWNS_SYSTEM,
    X1_SYSTEM,
    FM7_SYSTEM,
    MSX_SYSTEM,
    AT_SYSTEM,
];

/// The ROM folders of the MIDI modules. A machine uses them only with the matching `midi` value.
pub const MIDI_ROM_FOLDERS: [RomFolder; 2] = [
    RomFolder {
        owner: RomOwner::Mt32,
        name: "mt32",
        label: "MT-32",
        key: "mt32-roms",
        required: false,
    },
    RomFolder {
        owner: RomOwner::Sc55,
        name: "sc55",
        label: "SC-55",
        key: "sc55-roms",
        required: false,
    },
];

/// Returns the system of the family `target`.
#[must_use]
pub fn system_of(target: Target) -> &'static System {
    match target {
        #[cfg(feature = "pc98")]
        Target::Pc98 => &PC98_SYSTEM,
        #[cfg(feature = "pc88")]
        Target::Pc88 => &PC88_SYSTEM,
        #[cfg(feature = "pc88va")]
        Target::Pc88Va => &PC88VA_SYSTEM,
        #[cfg(feature = "pc60")]
        Target::Pc60 => &PC6000_SYSTEM,
        #[cfg(feature = "msx")]
        Target::Msx => &MSX_SYSTEM,
        #[cfg(feature = "towns")]
        Target::Towns => &TOWNS_SYSTEM,
        #[cfg(feature = "x1")]
        Target::X1 => &X1_SYSTEM,
        #[cfg(feature = "fm7")]
        Target::Fm7 => &FM7_SYSTEM,
        #[cfg(feature = "x68k")]
        Target::X68k => &X68K_SYSTEM,
        #[cfg(feature = "at")]
        Target::At => &AT_SYSTEM,
    }
}

/// Returns the systems compiled into this build, in table order.
pub fn compiled_systems() -> impl Iterator<Item = &'static System> {
    let compiled = Families::compiled();
    SYSTEMS
        .iter()
        .filter(move |system| compiled.includes(system.family))
}

/// Returns every ROM folder: the folder of each system, then the folders of the MIDI modules.
pub fn rom_folders() -> impl Iterator<Item = RomFolder> {
    SYSTEMS
        .iter()
        .map(System::rom_folder)
        .chain(MIDI_ROM_FOLDERS)
}

/// Returns the derived folder of `folder` below `roms_folder`.
///
/// The folder of an optional ROM set counts only when it exists. The folder of
/// a required ROM set always counts. A missing required set then names the
/// folder that the machine searched.
#[must_use]
pub fn derived_rom_folder(folder: &RomFolder, roms_folder: &Path) -> Option<PathBuf> {
    let path = roms_folder.join(folder.name);
    (folder.required || path.is_dir()).then_some(path)
}

/// Sets every unset ROM key of `config` to its derived folder below `roms_folder`.
pub fn apply_rom_folder_defaults(
    config: &mut EmulatorConfig,
    roms_folder: &Path,
) -> Result<(), String> {
    for folder in rom_folders() {
        let Some(setting) = find_setting(folder.key) else {
            return Err(format!("{} is not a settings key", folder.key));
        };
        if !(setting.format)(config).is_empty() {
            continue;
        }
        if let Some(path) = derived_rom_folder(&folder, roms_folder) {
            (setting.apply)(config, &path.to_string_lossy())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{
        super::{
            MachineSelection, first_system_config, model_name, resolve_model,
            settings::{MODEL_CHOICES, PathKind, SETTINGS, Scope, Section, ValueKind},
        },
        *,
    };

    /// A temporary folder. Dropping the value removes the folder.
    struct TemporaryFolder(PathBuf);

    impl TemporaryFolder {
        /// Creates an empty and unique temporary folder.
        fn new(name: &str) -> Self {
            let folder = std::env::temp_dir()
                .join(format!("neetan_systems_test_{}_{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&folder);
            std::fs::create_dir_all(&folder).expect("temporary folder is created");
            Self(folder)
        }
    }

    impl Drop for TemporaryFolder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Verifies that the names, families and ROM keys of the systems are unique and complete.
    #[test]
    fn systems_cover_every_family_once() {
        let mut families = Families::NONE;
        let mut names = BTreeSet::new();
        for system in SYSTEMS {
            assert!(!families.intersects(system.family), "{}", system.name);
            families = families.with(system.family);
            assert!(names.insert(system.name), "{} is listed twice", system.name);
            assert_eq!(system.rom_key, format!("{}-roms", system.name));
        }
        for folder in MIDI_ROM_FOLDERS {
            assert!(names.insert(folder.name), "{} is listed twice", folder.name);
            assert_eq!(folder.key, format!("{}-roms", folder.name));
        }
        assert_eq!(families, Families::ALL);
        assert_eq!(rom_folders().count(), 12);
    }

    /// Verifies that the ROM folders and the ROM keys of the settings table match one to one.
    #[test]
    fn rom_folders_match_the_rom_keys() {
        let keys: BTreeSet<&str> = rom_folders().map(|folder| folder.key).collect();
        let table_keys: BTreeSet<&str> = SETTINGS
            .iter()
            .filter(|setting| setting.section == Some(Section::Roms))
            .map(|setting| setting.key)
            .collect();
        assert_eq!(keys, table_keys);
        for key in keys {
            let setting = find_setting(key).expect("ROM key is a table key");
            assert_eq!(setting.scope, Scope::Host, "{key}");
            assert_eq!(setting.kind, ValueKind::Path(PathKind::Folder), "{key}");
        }
    }

    /// Verifies that the default model of each system names a model of its family.
    #[test]
    fn default_models_belong_to_their_family() {
        for system in SYSTEMS {
            assert!(
                MODEL_CHOICES.iter().any(|choice| {
                    choice.value == system.default_model && choice.families == system.family
                }),
                "{}",
                system.name
            );
        }
    }

    /// Verifies that the configuration of the default model of each compiled system holds that model.
    #[test]
    fn default_models_build_their_configuration() {
        for system in compiled_systems() {
            let model = MachineSelection::of_system(system).expect("default model resolves");
            let config = EmulatorConfig::for_model(model);
            assert_eq!(system_of(config.target), system);
            assert_eq!(model_name(&config), system.default_model);
            let mut resolved = first_system_config();
            resolve_model(&mut resolved, system.default_model).expect("default model resolves");
            assert_eq!(resolved.target, config.target);
        }
    }

    /// Verifies that an explicit ROM key wins and that a missing optional folder stays unset.
    #[test]
    fn derived_rom_folders_fill_only_unset_keys() {
        let folder = TemporaryFolder::new("derived");
        let roms = &folder.0;
        std::fs::create_dir_all(roms.join("at")).expect("folder is created");
        let mut config = EmulatorConfig {
            x1_roms: Some(PathBuf::from("/explicit/x1")),
            ..first_system_config()
        };
        apply_rom_folder_defaults(&mut config, roms).expect("derivation applies");
        assert_eq!(config.x1_roms, Some(PathBuf::from("/explicit/x1")));
        assert_eq!(config.at_roms, Some(roms.join("at")));
        assert_eq!(config.pc98_roms, None);
        assert_eq!(config.mt32_roms, None);
        assert_eq!(config.sc55_roms, None);
        assert_eq!(config.pc88_roms, Some(roms.join("pc88")));
        assert_eq!(config.towns_roms, Some(roms.join("towns")));
        assert_eq!(config.pc6000_roms, Some(roms.join("pc6000")));
    }
}
