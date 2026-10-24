//! The settings table: one declarative entry for every key of `EmulatorConfig`.
//!
//! An entry holds the key, its label and help text, its scope, its value kind
//! with the choices, the families and models that read it, its section, and the
//! functions that apply, format and clear its value.

use std::path::{Path, PathBuf};

use common::CpuMode;
#[cfg(feature = "pc88")]
use common::{EightMhzWaitMode, MemoryWaitSwitch};
use device::gameport_towns::TownsPadType;

use super::{BootMode, EmulatorConfig, Target, model_name, resolve_model, systems::SYSTEMS};

/// Layer scope of a setting: the application, the emulated machine or the media of a game.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Scope {
    /// Setting of the application and its window.
    Host,
    /// Setting of the emulated machine.
    System,
    /// Media image of a game.
    Game,
}

/// Set of machine families, one bit for each family.
///
/// The constants exist in every build. A family that is not compiled in never
/// matches a `Target`.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Families(u16);

impl Families {
    /// The empty set.
    pub const NONE: Self = Self(0);
    /// The PC-9800 series.
    pub const PC98: Self = Self(1 << 0);
    /// The PC-8801 series.
    pub const PC88: Self = Self(1 << 1);
    /// The PC-88VA series.
    pub const PC88VA: Self = Self(1 << 2);
    /// The PC-6000 / PC-6600 series.
    pub const PC60: Self = Self(1 << 3);
    /// The MSX series.
    pub const MSX: Self = Self(1 << 4);
    /// The FM Towns series.
    pub const TOWNS: Self = Self(1 << 5);
    /// The Sharp X1 series.
    pub const X1: Self = Self(1 << 6);
    /// The Fujitsu FM-7 series.
    pub const FM7: Self = Self(1 << 7);
    /// The Sharp X68000 series.
    pub const X68K: Self = Self(1 << 8);
    /// The IBM PC/AT series.
    pub const AT: Self = Self(1 << 9);
    /// Every family.
    pub const ALL: Self = Self(
        Self::PC98.0
            | Self::PC88.0
            | Self::PC88VA.0
            | Self::PC60.0
            | Self::MSX.0
            | Self::TOWNS.0
            | Self::X1.0
            | Self::FM7.0
            | Self::X68K.0
            | Self::AT.0,
    );

    /// Returns the union of two sets.
    #[must_use]
    pub const fn with(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Returns this set without the families of `other`.
    #[must_use]
    pub const fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// Returns whether every family of `other` is in this set.
    #[must_use]
    pub const fn includes(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Returns whether this set and `other` share at least one family.
    #[must_use]
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    /// Returns whether this set holds the family of `target`.
    #[must_use]
    pub fn contains_target(self, target: Target) -> bool {
        self.includes(Self::of(target))
    }

    /// Returns the set that holds only the family of `target`.
    #[must_use]
    pub fn of(target: Target) -> Self {
        match target {
            #[cfg(feature = "pc98")]
            Target::Pc98 => Self::PC98,
            #[cfg(feature = "pc88")]
            Target::Pc88 => Self::PC88,
            #[cfg(feature = "pc88va")]
            Target::Pc88Va => Self::PC88VA,
            #[cfg(feature = "pc60")]
            Target::Pc60 => Self::PC60,
            #[cfg(feature = "msx")]
            Target::Msx => Self::MSX,
            #[cfg(feature = "towns")]
            Target::Towns => Self::TOWNS,
            #[cfg(feature = "x1")]
            Target::X1 => Self::X1,
            #[cfg(feature = "fm7")]
            Target::Fm7 => Self::FM7,
            #[cfg(feature = "x68k")]
            Target::X68k => Self::X68K,
            #[cfg(feature = "at")]
            Target::At => Self::AT,
        }
    }

    /// Returns the families compiled into this build.
    #[must_use]
    pub const fn compiled() -> Self {
        let mut families = Self::NONE;
        #[cfg(feature = "pc98")]
        {
            families = families.with(Self::PC98);
        }
        #[cfg(feature = "pc88")]
        {
            families = families.with(Self::PC88);
        }
        #[cfg(feature = "pc88va")]
        {
            families = families.with(Self::PC88VA);
        }
        #[cfg(feature = "pc60")]
        {
            families = families.with(Self::PC60);
        }
        #[cfg(feature = "msx")]
        {
            families = families.with(Self::MSX);
        }
        #[cfg(feature = "towns")]
        {
            families = families.with(Self::TOWNS);
        }
        #[cfg(feature = "x1")]
        {
            families = families.with(Self::X1);
        }
        #[cfg(feature = "fm7")]
        {
            families = families.with(Self::FM7);
        }
        #[cfg(feature = "x68k")]
        {
            families = families.with(Self::X68K);
        }
        #[cfg(feature = "at")]
        {
            families = families.with(Self::AT);
        }
        families
    }

    /// Returns the system names of the families in this set, in the order of the systems table.
    pub fn names(self) -> impl Iterator<Item = &'static str> {
        SYSTEMS
            .iter()
            .filter(move |system| self.includes(system.family))
            .map(|system| system.name)
    }
}

/// Models of one family that read a setting.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct ModelLimit {
    /// The family the limit applies to.
    pub family: Families,
    /// The model names of the family that read the setting, in the form of the `machine` values.
    pub models: &'static [&'static str],
}

/// One value of a choice setting.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Choice {
    /// The value as the configuration file writes it.
    pub value: &'static str,
    /// The human-readable name of the value.
    pub label: &'static str,
    /// The families that accept the value.
    pub families: Families,
}

/// Range of a number setting.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct NumberRange {
    /// The smallest accepted value.
    pub minimum: f64,
    /// The largest accepted value.
    pub maximum: f64,
    /// The increment between two adjacent values.
    pub step: f64,
    /// Whether the value is a whole number.
    pub integer: bool,
}

impl NumberRange {
    /// Returns whether `value` lies inside the range.
    #[must_use]
    pub fn contains(self, value: f64) -> bool {
        (self.minimum..=self.maximum).contains(&value)
    }
}

/// The kind of path that a path setting holds.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PathKind {
    /// A file, for example a disk image.
    File,
    /// A folder, for example a ROM set.
    Folder,
}

/// Value kind of a setting.
#[derive(Debug, Copy, Clone, PartialEq)]
pub enum ValueKind {
    /// One value of a fixed list.
    Choice(&'static [Choice]),
    /// The values `on` and `off`.
    OnOff,
    /// A number in a range.
    Number(NumberRange),
    /// One path.
    Path(PathKind),
    /// An ordered list of files. The key repeats and each value appends one file.
    PathList,
}

/// Group of related settings.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Section {
    /// Model, CPU, boot and memory settings.
    Machine,
    /// Disk, disc, cartridge and cassette images.
    Media,
    /// Sound boards and MIDI.
    Sound,
    /// Screen presentation and video hardware.
    Display,
    /// Printer, text extractor and other rarely used settings.
    Advanced,
    /// The config folder, the ROM folder and the GUI font.
    Folders,
    /// ROM folders.
    Roms,
    /// Rendering backend and audio output.
    VideoAndAudio,
}

/// Setting whose value marks an explicit choice over a derived default.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ExplicitKey {
    /// The CPU mode.
    CpuMode,
    /// The PC-88 memory wait.
    #[cfg(feature = "pc88")]
    Pc88MemoryWait,
}

/// One entry of the settings table.
#[derive(Debug, Copy, Clone)]
pub struct Setting {
    /// The key of the setting, for example `cpu-mode`.
    pub key: &'static str,
    /// The short human-readable name of the setting.
    pub label: &'static str,
    /// The one-line description of the setting.
    pub help: &'static str,
    /// The layer scope of the key.
    pub scope: Scope,
    /// The value kind and its choices.
    pub kind: ValueKind,
    /// The families that read the setting.
    pub families: Families,
    /// Families where only some models read the setting. A family without a limit reads it on every model.
    pub model_limits: &'static [ModelLimit],
    /// The group of the setting. `None` marks a debug setting outside every group.
    pub section: Option<Section>,
    /// The explicit choice that a value of this setting records.
    pub explicit: Option<ExplicitKey>,
    /// Whether only the interactive frontend reads the setting.
    pub frontend_only: bool,
    /// Empties a path list. Every other kind has `None`.
    pub clear: Option<fn(&mut EmulatorConfig)>,
    /// Applies one value to the configuration. A path list appends the value.
    pub apply: fn(&mut EmulatorConfig, &str) -> Result<(), String>,
    /// Formats the configured value. A path list returns one value for each file, an unset value returns none.
    pub format: fn(&EmulatorConfig) -> Vec<String>,
}

impl Setting {
    /// Returns whether the machine `model` of the family `target` reads this setting.
    #[must_use]
    pub fn is_read_by(&self, target: Target, model: &str) -> bool {
        if !self.families.contains_target(target) {
            return false;
        }
        self.model_limits
            .iter()
            .find(|limit| limit.family.contains_target(target))
            .is_none_or(|limit| {
                limit
                    .models
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(model))
            })
    }

    /// Returns the choices of a choice setting that the family `target` accepts.
    pub fn choices_for(&self, target: Target) -> impl Iterator<Item = &'static Choice> {
        let choices: &'static [Choice] = match self.kind {
            ValueKind::Choice(choices) => choices,
            ValueKind::OnOff | ValueKind::Number(_) | ValueKind::Path(_) | ValueKind::PathList => {
                &[]
            }
        };
        choices
            .iter()
            .filter(move |choice| choice.families.contains_target(target))
    }
}

/// Settings whose defaults derive from other settings unless a layer sets them.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ExplicitSettings {
    /// A layer set `cpu-mode`.
    pub cpu_mode: bool,
    /// A layer set `pc88-memory-wait`.
    #[cfg(feature = "pc88")]
    pub pc88_memory_wait: bool,
}

impl ExplicitSettings {
    /// Applies one value of `setting` to `config` and records an explicit value.
    pub fn apply(
        &mut self,
        config: &mut EmulatorConfig,
        setting: &Setting,
        value: &str,
    ) -> Result<(), String> {
        (setting.apply)(config, value)?;
        match setting.explicit {
            Some(ExplicitKey::CpuMode) => self.cpu_mode = true,
            #[cfg(feature = "pc88")]
            Some(ExplicitKey::Pc88MemoryWait) => self.pc88_memory_wait = true,
            None => {}
        }
        Ok(())
    }
}

/// Applies the defaults that derive from other settings.
///
/// The PC-88 boot modes V1 standard and the PC-8001 compatible N modes run at
/// 4 MHz with the real-hardware memory waits. An explicit `cpu-mode` or
/// `pc88-memory-wait` keeps its value.
#[cfg(feature = "pc88")]
pub fn apply_derived_defaults(config: &mut EmulatorConfig, explicit: ExplicitSettings) {
    if config.target != Target::Pc88 {
        return;
    }
    if !config
        .boot_mode
        .is_some_and(BootMode::wants_pc88_compatibility_speed)
    {
        return;
    }
    if !explicit.cpu_mode {
        config.cpu_mode = CpuMode::Low;
    }
    if !explicit.pc88_memory_wait {
        config.pc88_memory_wait = MemoryWaitSwitch::Compatible;
    }
}

/// Applies the defaults that derive from other settings. Only the PC-88 has such defaults.
#[cfg(not(feature = "pc88"))]
pub fn apply_derived_defaults(_config: &mut EmulatorConfig, _explicit: ExplicitSettings) {}

/// Key of the machine model.
pub const MACHINE_KEY: &str = "machine";
/// Key of the config folder.
pub const CONFIG_FOLDER_KEY: &str = "config-folder";
/// Key of the ROM folder.
pub const ROMS_FOLDER_KEY: &str = "roms-folder";
/// Key of the font file of the user interface.
pub const GUI_FONT_KEY: &str = "gui-font";
/// Key of the rendering backend.
pub const BACKEND_KEY: &str = "backend";
/// Key of the audio volume.
pub const AUDIO_VOLUME_KEY: &str = "audio-volume";
/// Key of the window mode.
pub const WINDOW_MODE_KEY: &str = "window-mode";

/// Returns the settings table entry of `key`.
#[must_use]
pub fn find_setting(key: &str) -> Option<&'static Setting> {
    SETTINGS.iter().find(|setting| setting.key == key)
}

/// Returns the model names of the families compiled into this build.
#[must_use]
pub fn compiled_model_names() -> Vec<&'static str> {
    let compiled = Families::compiled();
    MODEL_CHOICES
        .iter()
        .filter(|choice| compiled.includes(choice.families))
        .map(|choice| choice.value)
        .collect()
}

/// Range of the PC-6000 composite phase.
const COMPOSITE_PHASE_RANGE: NumberRange = NumberRange {
    minimum: 0.0,
    maximum: 3.0,
    step: 1.0,
    integer: true,
};

/// Range of the audio volume.
const AUDIO_VOLUME_RANGE: NumberRange = NumberRange {
    minimum: 0.0,
    maximum: 1.0,
    step: 0.1,
    integer: false,
};

/// Parses an on/off value.
fn parse_on_off(value: &str) -> Result<bool, String> {
    match value {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err(format!("invalid value '{value}', expected on or off")),
    }
}

/// Formats an on/off value.
fn format_on_off(value: bool) -> Vec<String> {
    vec![if value { "on" } else { "off" }.to_owned()]
}

/// Formats one displayable value.
fn format_value(value: impl std::fmt::Display) -> Vec<String> {
    vec![value.to_string()]
}

/// Formats a decimal number with at least one fractional digit.
fn format_decimal(value: f32) -> Vec<String> {
    vec![format!("{value:?}")]
}

/// Formats an optional displayable value. `None` formats as an unset key.
fn format_optional(value: Option<impl std::fmt::Display>) -> Vec<String> {
    value.map(|value| value.to_string()).into_iter().collect()
}

/// Formats an optional path. `None` formats as an unset key.
fn format_path(path: Option<&PathBuf>) -> Vec<String> {
    path.map(|path| path_text(path)).into_iter().collect()
}

/// Formats a path list, one value for each path.
fn format_path_list(paths: &[PathBuf]) -> Vec<String> {
    paths.iter().map(|path| path_text(path)).collect()
}

/// Parses an optional path. An empty value is unset.
fn parse_optional_path(value: &str) -> Option<PathBuf> {
    (!value.is_empty()).then(|| PathBuf::from(value))
}

/// Parses a required folder. An empty value is an error.
fn parse_folder(value: &str) -> Result<PathBuf, String> {
    if value.is_empty() {
        return Err("the value is empty, a folder is needed".to_owned());
    }
    Ok(PathBuf::from(value))
}

/// Parses a required file path. An empty value is an error.
fn parse_file(value: &str) -> Result<PathBuf, String> {
    if value.is_empty() {
        return Err("the value is empty, a file is needed".to_owned());
    }
    Ok(PathBuf::from(value))
}

/// Appends a path to a path list. An empty value appends nothing.
fn push_path(list: &mut Vec<PathBuf>, value: &str) {
    if !value.is_empty() {
        list.push(PathBuf::from(value));
    }
}

/// Returns the text of a path as a configuration file writes it.
fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Parses the PC-6000 composite phase.
fn parse_composite_phase(value: &str) -> Result<u32, String> {
    match value.parse::<u32>() {
        Ok(phase) if COMPOSITE_PHASE_RANGE.contains(f64::from(phase)) => Ok(phase),
        _ => Err(format!(
            "invalid composite phase '{value}', expected 0, 1, 2 or 3"
        )),
    }
}

/// Parses the audio volume.
pub fn parse_audio_volume(value: &str) -> Result<f32, String> {
    match value.parse::<f32>() {
        Ok(volume) if AUDIO_VOLUME_RANGE.contains(f64::from(volume)) => Ok(volume),
        _ => Err(format!(
            "invalid audio volume '{value}', expected a number from 0.0 to 1.0"
        )),
    }
}

/// Parses the FM Towns pad type.
fn parse_towns_pad(value: &str) -> Result<TownsPadType, String> {
    value.parse()
}

/// Formats the FM Towns pad type.
fn format_towns_pad(pad: TownsPadType) -> Vec<String> {
    let text = match pad {
        TownsPadType::TwoButton => "2",
        TownsPadType::SixButton => "6",
    };
    vec![text.to_owned()]
}

/// Families with a floppy drive on at least one model.
const FDD1_FAMILIES: Families = Families::ALL;
/// Families with a second floppy drive on at least one model.
const FDD2_FAMILIES: Families = Families::ALL.without(Families::MSX);
/// Families with a hard disk interface.
const HDD_FAMILIES: Families = Families::PC98
    .with(Families::TOWNS)
    .with(Families::X68K)
    .with(Families::AT);
/// Families with a CD-ROM drive on at least one model.
const CDROM_FAMILIES: Families = Families::PC98
    .with(Families::PC88)
    .with(Families::TOWNS)
    .with(Families::X68K)
    .with(Families::AT);
/// Families with a MIDI output.
const MIDI_FAMILIES: Families = Families::PC98
    .with(Families::TOWNS)
    .with(Families::X68K)
    .with(Families::AT);
/// Families with a boot device selection.
const BOOT_DEVICE_FAMILIES: Families = Families::PC98.with(Families::TOWNS).with(Families::AT);
/// Families with a boot device selection beyond the PC/AT boot orders.
const FULL_BOOT_DEVICE_FAMILIES: Families = Families::PC98.with(Families::TOWNS);

/// PC-98 models with a selectable CPU clock.
const PC98_CPU_MODE_MODELS: &[&str] = &["PC9801F", "PC9801VM", "PC9801VX"];
/// PC-98 models with EMS, XMS and the 5 MHz GDC clock.
const PC98_VX_AND_LATER_MODELS: &[&str] =
    &["PC9801VX", "PC9801RS", "PC9801RA", "PC9821AS", "PC9821AP"];
/// PC-98 models that boot a real BIOS.
const PC98_REAL_BIOS_MODELS: &[&str] = &["PC9801F", "PC9801VM", "PC9801VX", "PC9801RS", "PC9801RA"];
/// PC-98 models with a CD-ROM drive.
const PC98_CDROM_MODELS: &[&str] = &["PC9821AS", "PC9821AP"];
/// PC-6000 models with built-in floppy drives.
const PC60_FDD_MODELS: &[&str] = &["PC6601", "PC6601SR"];
/// MSX models with a floppy drive.
const MSX_FDD_MODELS: &[&str] = &["MSX2", "MSX2PLUS"];
/// FM Towns models with a selectable CPU clock.
const TOWNS_CPU_MODE_MODELS: &[&str] = &["FMTownsIICX", "FMTownsIIMX"];
/// X68000 models with a selectable CPU clock.
const X68K_CPU_MODE_MODELS: &[&str] = &["X68000XVI"];
/// X68000 models with a CD-ROM capable SCSI interface.
const X68K_CDROM_MODELS: &[&str] = &["X68000SUPER", "X68000XVI"];
/// X1 models with the turbo monitor and keyboard switches.
const X1_TURBO_MODELS: &[&str] = &["X1TURBO"];

/// Every machine model, with the family that builds it.
pub const MODEL_CHOICES: &[Choice] = &[
    Choice {
        value: "PC9801F",
        label: "PC-9801F",
        families: Families::PC98,
    },
    Choice {
        value: "PC9801VM",
        label: "PC-9801VM",
        families: Families::PC98,
    },
    Choice {
        value: "PC9801VX",
        label: "PC-9801VX",
        families: Families::PC98,
    },
    Choice {
        value: "PC9801RS",
        label: "PC-9801RS",
        families: Families::PC98,
    },
    Choice {
        value: "PC9801RA",
        label: "PC-9801RA",
        families: Families::PC98,
    },
    Choice {
        value: "PC9821AS",
        label: "PC-9821As",
        families: Families::PC98,
    },
    Choice {
        value: "PC9821AP",
        label: "PC-9821Ap",
        families: Families::PC98,
    },
    Choice {
        value: "PC8801MC",
        label: "PC-8801MC",
        families: Families::PC88,
    },
    Choice {
        value: "PC88VA2",
        label: "PC-88VA2",
        families: Families::PC88VA,
    },
    Choice {
        value: "PC6001",
        label: "PC-6001",
        families: Families::PC60,
    },
    Choice {
        value: "PC6001MK2",
        label: "PC-6001mkII",
        families: Families::PC60,
    },
    Choice {
        value: "PC6601",
        label: "PC-6601",
        families: Families::PC60,
    },
    Choice {
        value: "PC6001MK2SR",
        label: "PC-6001mkIISR",
        families: Families::PC60,
    },
    Choice {
        value: "PC6601SR",
        label: "PC-6601SR",
        families: Families::PC60,
    },
    Choice {
        value: "MSX",
        label: "MSX",
        families: Families::MSX,
    },
    Choice {
        value: "MSX2",
        label: "MSX2",
        families: Families::MSX,
    },
    Choice {
        value: "MSX2PLUS",
        label: "MSX2+",
        families: Families::MSX,
    },
    Choice {
        value: "FMTowns",
        label: "FM Towns",
        families: Families::TOWNS,
    },
    Choice {
        value: "FMTownsIICX",
        label: "FM Towns II CX",
        families: Families::TOWNS,
    },
    Choice {
        value: "FMTownsIIMX",
        label: "FM Towns II MX",
        families: Families::TOWNS,
    },
    Choice {
        value: "X68000",
        label: "X68000",
        families: Families::X68K,
    },
    Choice {
        value: "X68000SUPER",
        label: "X68000 SUPER",
        families: Families::X68K,
    },
    Choice {
        value: "X68000XVI",
        label: "X68000 XVI",
        families: Families::X68K,
    },
    Choice {
        value: "X1",
        label: "X1",
        families: Families::X1,
    },
    Choice {
        value: "X1TURBO",
        label: "X1 turbo",
        families: Families::X1,
    },
    Choice {
        value: "FM7",
        label: "FM-7",
        families: Families::FM7,
    },
    Choice {
        value: "FM77AV",
        label: "FM77AV",
        families: Families::FM7,
    },
    Choice {
        value: "AT486DX50",
        label: "PC/AT i486DX2-50",
        families: Families::AT,
    },
    Choice {
        value: "AT486DX66",
        label: "PC/AT i486DX2-66",
        families: Families::AT,
    },
];

/// CPU clock modes.
const CPU_MODE_CHOICES: &[Choice] = &[
    Choice {
        value: "low",
        label: "Low",
        families: Families::ALL,
    },
    Choice {
        value: "high",
        label: "High",
        families: Families::ALL,
    },
];

/// Boot modes of the PC-88 and the FM-7.
const BOOT_MODE_CHOICES: &[Choice] = &[
    Choice {
        value: "v1s",
        label: "N88-BASIC V1 standard speed",
        families: Families::PC88,
    },
    Choice {
        value: "v1h",
        label: "N88-BASIC V1 high speed",
        families: Families::PC88,
    },
    Choice {
        value: "v2",
        label: "N88-BASIC V2",
        families: Families::PC88,
    },
    Choice {
        value: "n",
        label: "N-BASIC",
        families: Families::PC88,
    },
    Choice {
        value: "n80",
        label: "N80-BASIC",
        families: Families::PC88,
    },
    Choice {
        value: "n80sr",
        label: "N80SR-BASIC",
        families: Families::PC88,
    },
    Choice {
        value: "basic",
        label: "F-BASIC",
        families: Families::FM7,
    },
    Choice {
        value: "dos",
        label: "Disk boot",
        families: Families::FM7,
    },
];

/// Boot devices.
const BOOT_DEVICE_CHOICES: &[Choice] = &[
    Choice {
        value: "auto",
        label: "Automatic",
        families: BOOT_DEVICE_FAMILIES,
    },
    Choice {
        value: "fdd1",
        label: "FDD 1",
        families: BOOT_DEVICE_FAMILIES,
    },
    Choice {
        value: "fdd2",
        label: "FDD 2",
        families: FULL_BOOT_DEVICE_FAMILIES,
    },
    Choice {
        value: "hdd1",
        label: "HDD 1",
        families: BOOT_DEVICE_FAMILIES,
    },
    Choice {
        value: "hdd2",
        label: "HDD 2",
        families: FULL_BOOT_DEVICE_FAMILIES,
    },
    Choice {
        value: "dos",
        label: "Neetan DOS",
        families: FULL_BOOT_DEVICE_FAMILIES,
    },
];

/// Monitor timings.
const MONITOR_CHOICES: &[Choice] = &[
    Choice {
        value: "auto",
        label: "Automatic",
        families: Families::ALL,
    },
    Choice {
        value: "15k",
        label: "15 kHz",
        families: Families::ALL,
    },
    Choice {
        value: "24k",
        label: "24 kHz",
        families: Families::ALL,
    },
];

/// PC-88 wait modes.
#[cfg(feature = "pc88")]
const WAIT_CHOICES: &[Choice] = &[
    Choice {
        value: "fast",
        label: "Fast",
        families: Families::ALL,
    },
    Choice {
        value: "compatible",
        label: "Compatible",
        families: Families::ALL,
    },
];

/// X1 turbo keyboard mode switch positions.
const X1_KEYBOARD_CHOICES: &[Choice] = &[
    Choice {
        value: "A",
        label: "Mode A",
        families: Families::ALL,
    },
    Choice {
        value: "B",
        label: "Mode B",
        families: Families::ALL,
    },
];

/// FM Towns pad types.
const TOWNS_PAD_CHOICES: &[Choice] = &[
    Choice {
        value: "2",
        label: "2 buttons",
        families: Families::ALL,
    },
    Choice {
        value: "6",
        label: "6 buttons",
        families: Families::ALL,
    },
];

/// PC-98 sound boards.
const SOUNDBOARD_CHOICES: &[Choice] = &[
    Choice {
        value: "none",
        label: "None",
        families: Families::ALL,
    },
    Choice {
        value: "14",
        label: "PC-9801-14",
        families: Families::ALL,
    },
    Choice {
        value: "26k",
        label: "PC-9801-26K",
        families: Families::ALL,
    },
    Choice {
        value: "86",
        label: "PC-9801-86",
        families: Families::ALL,
    },
    Choice {
        value: "86+26k",
        label: "PC-9801-86 and PC-9801-26K",
        families: Families::ALL,
    },
    Choice {
        value: "sb16",
        label: "Sound Blaster 16",
        families: Families::ALL,
    },
    Choice {
        value: "sb16+26k",
        label: "Sound Blaster 16 and PC-9801-26K",
        families: Families::ALL,
    },
];

/// MIDI devices.
const MIDI_CHOICES: &[Choice] = &[
    Choice {
        value: "none",
        label: "None",
        families: Families::ALL,
    },
    Choice {
        value: "mt32",
        label: "Roland MT-32",
        families: Families::ALL,
    },
    Choice {
        value: "sc55",
        label: "Roland SC-55",
        families: Families::ALL,
    },
];

/// Display aspect modes.
const ASPECT_MODE_CHOICES: &[Choice] = &[
    Choice {
        value: "4:3",
        label: "4:3",
        families: Families::ALL,
    },
    Choice {
        value: "1:1",
        label: "1:1",
        families: Families::ALL,
    },
];

/// Window modes.
const WINDOW_MODE_CHOICES: &[Choice] = &[
    Choice {
        value: "windowed",
        label: "Windowed",
        families: Families::ALL,
    },
    Choice {
        value: "fullscreen",
        label: "Fullscreen",
        families: Families::ALL,
    },
];

/// Scaling methods.
const SCALING_CHOICES: &[Choice] = &[
    Choice {
        value: "nearest",
        label: "Nearest",
        families: Families::ALL,
    },
    Choice {
        value: "bilinear",
        label: "Bilinear",
        families: Families::ALL,
    },
    Choice {
        value: "pixelart",
        label: "Pixel art",
        families: Families::ALL,
    },
];

/// Forced GDC clocks.
const GDC_CLOCK_CHOICES: &[Choice] = &[
    Choice {
        value: "2.5",
        label: "2.5 MHz",
        families: Families::ALL,
    },
    Choice {
        value: "5",
        label: "5 MHz",
        families: Families::ALL,
    },
];

/// PC-98 graphics boards.
const GRAPHICBOARD_CHOICES: &[Choice] = &[
    Choice {
        value: "none",
        label: "None",
        families: Families::ALL,
    },
    Choice {
        value: "ga1280a",
        label: "I-O DATA GA-1280A",
        families: Families::ALL,
    },
];

/// Rendering backends.
const BACKEND_CHOICES: &[Choice] = &[
    Choice {
        value: "modern",
        label: "Modern",
        families: Families::ALL,
    },
    Choice {
        value: "legacy",
        label: "Legacy",
        families: Families::ALL,
    },
];

/// Every setting, ordered by section.
pub static SETTINGS: &[Setting] = &[
    Setting {
        key: MACHINE_KEY,
        label: "Model",
        help: "Machine model. It selects the family.",
        scope: Scope::System,
        kind: ValueKind::Choice(MODEL_CHOICES),
        families: Families::ALL,
        model_limits: &[],
        section: Some(Section::Machine),
        explicit: None,
        frontend_only: false,
        clear: None,
        apply: resolve_model,
        format: |config| vec![model_name(config).to_owned()],
    },
    Setting {
        key: "cpu-mode",
        label: "CPU mode",
        help: "CPU speed mode. The PC-88 default derives from the boot mode. X68000 XVI 10 or 16.67 MHz, FM Towns II CX 16 or 20 MHz, II MX 33 or 66 MHz.",
        scope: Scope::System,
        kind: ValueKind::Choice(CPU_MODE_CHOICES),
        families: Families::PC98
            .with(Families::PC88)
            .with(Families::TOWNS)
            .with(Families::X68K)
            .with(Families::AT),
        model_limits: &[
            ModelLimit {
                family: Families::PC98,
                models: PC98_CPU_MODE_MODELS,
            },
            ModelLimit {
                family: Families::TOWNS,
                models: TOWNS_CPU_MODE_MODELS,
            },
            ModelLimit {
                family: Families::X68K,
                models: X68K_CPU_MODE_MODELS,
            },
        ],
        section: Some(Section::Machine),
        explicit: Some(ExplicitKey::CpuMode),
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.cpu_mode = value.parse::<CpuMode>()?;
            Ok(())
        },
        format: |config| format_value(config.cpu_mode),
    },
    Setting {
        key: "boot-mode",
        label: "Boot mode",
        help: "Boot mode. Each family accepts only its own values.",
        scope: Scope::System,
        kind: ValueKind::Choice(BOOT_MODE_CHOICES),
        families: Families::PC88.with(Families::FM7),
        model_limits: &[],
        section: Some(Section::Machine),
        explicit: None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.boot_mode = Some(value.parse::<BootMode>()?);
            Ok(())
        },
        format: |config| format_optional(config.boot_mode),
    },
    Setting {
        key: "boot-device",
        label: "Boot device",
        help: "Boot device.",
        scope: Scope::System,
        kind: ValueKind::Choice(BOOT_DEVICE_CHOICES),
        families: BOOT_DEVICE_FAMILIES,
        model_limits: &[],
        section: Some(Section::Machine),
        explicit: None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.boot_device = value.parse::<common::BootDevice>()?;
            Ok(())
        },
        format: |config| format_value(config.boot_device),
    },
    Setting {
        key: "bios",
        label: "Real BIOS",
        help: "Boot the real BIOS from the ROM folder of the family.",
        scope: Scope::System,
        kind: ValueKind::OnOff,
        families: Families::PC98.with(Families::AT),
        model_limits: &[ModelLimit {
            family: Families::PC98,
            models: PC98_REAL_BIOS_MODELS,
        }],
        section: Some(Section::Machine),
        explicit: None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.bios = parse_on_off(value)?;
            Ok(())
        },
        format: |config| format_on_off(config.bios),
    },
    Setting {
        key: "ems",
        label: "EMS memory",
        help: "EMS expanded memory of the HLE DOS.",
        scope: Scope::System,
        kind: ValueKind::OnOff,
        families: Families::PC98,
        model_limits: &[ModelLimit {
            family: Families::PC98,
            models: PC98_VX_AND_LATER_MODELS,
        }],
        section: Some(Section::Machine),
        explicit: None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.ems = parse_on_off(value)?;
            Ok(())
        },
        format: |config| format_on_off(config.ems),
    },
    Setting {
        key: "xms",
        label: "XMS memory",
        help: "XMS extended memory of the HLE DOS.",
        scope: Scope::System,
        kind: ValueKind::OnOff,
        families: Families::PC98,
        model_limits: &[ModelLimit {
            family: Families::PC98,
            models: PC98_VX_AND_LATER_MODELS,
        }],
        section: Some(Section::Machine),
        explicit: None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.xms = parse_on_off(value)?;
            Ok(())
        },
        format: |config| format_on_off(config.xms),
    },
    Setting {
        key: "monitor",
        label: "Monitor",
        help: "Monitor timing.",
        scope: Scope::System,
        kind: ValueKind::Choice(MONITOR_CHOICES),
        families: Families::PC88.with(Families::X1),
        model_limits: &[ModelLimit {
            family: Families::X1,
            models: X1_TURBO_MODELS,
        }],
        section: Some(Section::Machine),
        explicit: None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.monitor = value.parse::<common::MonitorTiming>()?;
            Ok(())
        },
        format: |config| format_value(config.monitor),
    },
    #[cfg(feature = "pc88")]
    Setting {
        key: "pc88-memory-wait",
        label: "Memory wait",
        help: "PC-8801 memory wait. The default derives from the boot mode.",
        scope: Scope::System,
        kind: ValueKind::Choice(WAIT_CHOICES),
        families: Families::PC88,
        model_limits: &[],
        section: Some(Section::Machine),
        explicit: Some(ExplicitKey::Pc88MemoryWait),
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.pc88_memory_wait = value.parse::<MemoryWaitSwitch>()?;
            Ok(())
        },
        format: |config| format_value(config.pc88_memory_wait),
    },
    #[cfg(feature = "pc88")]
    Setting {
        key: "pc88-8mhz-wait",
        label: "8 MHz wait",
        help: "PC-8801 8 MHz wait.",
        scope: Scope::System,
        kind: ValueKind::Choice(WAIT_CHOICES),
        families: Families::PC88,
        model_limits: &[],
        section: Some(Section::Machine),
        explicit: None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.pc88_8mhz_wait = value.parse::<EightMhzWaitMode>()?;
            Ok(())
        },
        format: |config| format_value(config.pc88_8mhz_wait),
    },
    Setting {
        key: "x1-keyboard",
        label: "Keyboard mode",
        help: "X1 turbo keyboard mode switch.",
        scope: Scope::System,
        kind: ValueKind::Choice(X1_KEYBOARD_CHOICES),
        families: Families::X1,
        model_limits: &[ModelLimit {
            family: Families::X1,
            models: X1_TURBO_MODELS,
        }],
        section: Some(Section::Machine),
        explicit: None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.x1_keyboard = value.parse()?;
            Ok(())
        },
        format: |config| format_value(config.x1_keyboard),
    },
    Setting {
        key: "towns-pad",
        label: "Game pad",
        help: "FM Towns game pad type.",
        scope: Scope::System,
        kind: ValueKind::Choice(TOWNS_PAD_CHOICES),
        families: Families::TOWNS,
        model_limits: &[],
        section: Some(Section::Machine),
        explicit: None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.towns_pad = parse_towns_pad(value)?;
            Ok(())
        },
        format: |config| format_towns_pad(config.towns_pad),
    },
    Setting {
        key: "fdd1",
        label: "FDD 1",
        help: "Floppy disk image for drive 1. Repeat the key for more images.",
        scope: Scope::Game,
        kind: ValueKind::PathList,
        families: FDD1_FAMILIES,
        model_limits: &[
            ModelLimit {
                family: Families::PC60,
                models: PC60_FDD_MODELS,
            },
            ModelLimit {
                family: Families::MSX,
                models: MSX_FDD_MODELS,
            },
        ],
        section: Some(Section::Media),
        explicit: None,
        frontend_only: false,
        clear: Some(|config| config.fdd1.clear()),
        apply: |config, value| {
            push_path(&mut config.fdd1, value);
            Ok(())
        },
        format: |config| format_path_list(&config.fdd1),
    },
    Setting {
        key: "fdd2",
        label: "FDD 2",
        help: "Floppy disk image for drive 2. Repeat the key for more images.",
        scope: Scope::Game,
        kind: ValueKind::PathList,
        families: FDD2_FAMILIES,
        model_limits: &[ModelLimit {
            family: Families::PC60,
            models: PC60_FDD_MODELS,
        }],
        section: Some(Section::Media),
        explicit: None,
        frontend_only: false,
        clear: Some(|config| config.fdd2.clear()),
        apply: |config, value| {
            push_path(&mut config.fdd2, value);
            Ok(())
        },
        format: |config| format_path_list(&config.fdd2),
    },
    Setting {
        key: "cdrom",
        label: "CD-ROM",
        help: "CD-ROM disc image, a .cue or .ccd file. Repeat the key for more discs.",
        scope: Scope::Game,
        kind: ValueKind::PathList,
        families: CDROM_FAMILIES,
        model_limits: &[
            ModelLimit {
                family: Families::PC98,
                models: PC98_CDROM_MODELS,
            },
            ModelLimit {
                family: Families::X68K,
                models: X68K_CDROM_MODELS,
            },
        ],
        section: Some(Section::Media),
        explicit: None,
        frontend_only: false,
        clear: Some(|config| config.cdrom.clear()),
        apply: |config, value| {
            push_path(&mut config.cdrom, value);
            Ok(())
        },
        format: |config| format_path_list(&config.cdrom),
    },
    Setting {
        key: "cdrom-compat",
        label: "Compatible CD-ROM timing",
        help: "Slow and compatible CD-ROM drive timing.",
        scope: Scope::System,
        kind: ValueKind::OnOff,
        families: Families::TOWNS,
        model_limits: &[],
        section: Some(Section::Media),
        explicit: None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.cdrom_compat = parse_on_off(value)?;
            Ok(())
        },
        format: |config| format_on_off(config.cdrom_compat),
    },
    Setting {
        key: "hdd1",
        label: "HDD 1",
        help: "Hard disk image for drive 1: PC-98 .hdi/.nhd/.thd, FM Towns .h0-.h4, X68000 .hdf, PC/AT .hdd.",
        scope: Scope::Game,
        kind: ValueKind::Path(PathKind::File),
        families: HDD_FAMILIES,
        model_limits: &[],
        section: Some(Section::Media),
        explicit: None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.hdd1 = parse_optional_path(value);
            Ok(())
        },
        format: |config| format_path(config.hdd1.as_ref()),
    },
    Setting {
        key: "hdd2",
        label: "HDD 2",
        help: "Hard disk image for drive 2: PC-98 .hdi/.nhd/.thd, FM Towns .h0-.h4, X68000 .hdf, PC/AT .hdd.",
        scope: Scope::Game,
        kind: ValueKind::Path(PathKind::File),
        families: HDD_FAMILIES,
        model_limits: &[],
        section: Some(Section::Media),
        explicit: None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.hdd2 = parse_optional_path(value);
            Ok(())
        },
        format: |config| format_path(config.hdd2.as_ref()),
    },
    Setting {
        key: "cartridge",
        label: "Cartridge",
        help: "Cartridge ROM image.",
        scope: Scope::Game,
        kind: ValueKind::Path(PathKind::File),
        families: Families::PC60.with(Families::MSX),
        model_limits: &[],
        section: Some(Section::Media),
        explicit: None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.cartridge = parse_optional_path(value);
            Ok(())
        },
        format: |config| format_path(config.cartridge.as_ref()),
    },
    Setting {
        key: "cassette",
        label: "Cassette",
        help: "Cassette tape image: MSX .cas, PC-6000 .cas/.p6/.p6t, X1 .tap, FM-7 .t77.",
        scope: Scope::Game,
        kind: ValueKind::Path(PathKind::File),
        families: Families::PC60
            .with(Families::MSX)
            .with(Families::X1)
            .with(Families::FM7),
        model_limits: &[],
        section: Some(Section::Media),
        explicit: None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.cassette = parse_optional_path(value);
            Ok(())
        },
        format: |config| format_path(config.cassette.as_ref()),
    },
    Setting {
        key: "soundboard",
        label: "Sound board",
        help: "PC-98 sound board.",
        scope: Scope::System,
        kind: ValueKind::Choice(SOUNDBOARD_CHOICES),
        families: Families::PC98,
        model_limits: &[],
        section: Some(Section::Sound),
        explicit: None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.soundboard = value.parse()?;
            Ok(())
        },
        format: |config| format_value(config.soundboard),
    },
    Setting {
        key: "adpcm-ram",
        label: "ADPCM RAM",
        help: "ADPCM RAM option of the PC-9801-86.",
        scope: Scope::System,
        kind: ValueKind::OnOff,
        families: Families::PC98,
        model_limits: &[],
        section: Some(Section::Sound),
        explicit: None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.adpcm_ram = parse_on_off(value)?;
            Ok(())
        },
        format: |config| format_on_off(config.adpcm_ram),
    },
    Setting {
        key: "midi",
        label: "MIDI device",
        help: "MIDI device. MT-32 and SC-55 need their ROM folders.",
        scope: Scope::System,
        kind: ValueKind::Choice(MIDI_CHOICES),
        families: MIDI_FAMILIES,
        model_limits: &[],
        section: Some(Section::Sound),
        explicit: None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.midi = value.parse()?;
            Ok(())
        },
        format: |config| format_value(config.midi),
    },
    Setting {
        key: "aspect-mode",
        label: "Aspect mode",
        help: "Display aspect mode.",
        scope: Scope::System,
        kind: ValueKind::Choice(ASPECT_MODE_CHOICES),
        families: Families::ALL,
        model_limits: &[],
        section: Some(Section::Display),
        explicit: None,
        frontend_only: true,
        clear: None,
        apply: |config, value| {
            config.aspect_mode = value.parse()?;
            Ok(())
        },
        format: |config| format_value(config.aspect_mode),
    },
    Setting {
        key: WINDOW_MODE_KEY,
        label: "Window mode",
        help: "Window mode.",
        scope: Scope::System,
        kind: ValueKind::Choice(WINDOW_MODE_CHOICES),
        families: Families::ALL,
        model_limits: &[],
        section: Some(Section::Display),
        explicit: None,
        frontend_only: true,
        clear: None,
        apply: |config, value| {
            config.window_mode = value.parse()?;
            Ok(())
        },
        format: |config| format_value(config.window_mode),
    },
    Setting {
        key: "crt",
        label: "CRT effect",
        help: "CRT effect. The modern backend draws it.",
        scope: Scope::System,
        kind: ValueKind::OnOff,
        families: Families::ALL,
        model_limits: &[],
        section: Some(Section::Display),
        explicit: None,
        frontend_only: true,
        clear: None,
        apply: |config, value| {
            config.crt = parse_on_off(value)?;
            Ok(())
        },
        format: |config| format_on_off(config.crt),
    },
    Setting {
        key: "scaling",
        label: "Scaling",
        help: "Scaling method.",
        scope: Scope::System,
        kind: ValueKind::Choice(SCALING_CHOICES),
        families: Families::ALL,
        model_limits: &[],
        section: Some(Section::Display),
        explicit: None,
        frontend_only: true,
        clear: None,
        apply: |config, value| {
            config.scaling = value.parse()?;
            Ok(())
        },
        format: |config| format_value(config.scaling),
    },
    Setting {
        key: "pc6000-phase",
        label: "Color phase",
        help: "Initial composite artifact color phase. Right Ctrl + P cycles it.",
        scope: Scope::System,
        kind: ValueKind::Number(COMPOSITE_PHASE_RANGE),
        families: Families::PC60,
        model_limits: &[],
        section: Some(Section::Display),
        explicit: None,
        frontend_only: true,
        clear: None,
        apply: |config, value| {
            config.pc60_composite_phase = parse_composite_phase(value)?;
            Ok(())
        },
        format: |config| format_value(config.pc60_composite_phase),
    },
    Setting {
        key: "force-gdc-clock",
        label: "GDC clock",
        help: "Force the GDC clock in MHz. Unset follows the machine.",
        scope: Scope::System,
        kind: ValueKind::Choice(GDC_CLOCK_CHOICES),
        families: Families::PC98,
        model_limits: &[ModelLimit {
            family: Families::PC98,
            models: PC98_VX_AND_LATER_MODELS,
        }],
        section: Some(Section::Display),
        explicit: None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.force_gdc_clock = Some(value.parse()?);
            Ok(())
        },
        format: |config| format_optional(config.force_gdc_clock),
    },
    Setting {
        key: "graphicboard",
        label: "Graphics board",
        help: "PC-98 graphics accelerator board.",
        scope: Scope::System,
        kind: ValueKind::Choice(GRAPHICBOARD_CHOICES),
        families: Families::PC98,
        model_limits: &[],
        section: Some(Section::Display),
        explicit: None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.graphicboard = value.parse()?;
            Ok(())
        },
        format: |config| format_value(config.graphicboard),
    },
    Setting {
        key: "printer",
        label: "Printer output",
        help: "Output file of the printer. The file must exist.",
        scope: Scope::System,
        kind: ValueKind::Path(PathKind::File),
        families: Families::PC98,
        model_limits: &[],
        section: Some(Section::Advanced),
        explicit: None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.printer = parse_optional_path(value);
            Ok(())
        },
        format: |config| format_path(config.printer.as_ref()),
    },
    Setting {
        key: "enable-extractor",
        label: "Text extractor",
        help: "Copy on-screen text to the system clipboard.",
        scope: Scope::System,
        kind: ValueKind::OnOff,
        families: Families::PC98,
        model_limits: &[],
        section: Some(Section::Advanced),
        explicit: None,
        frontend_only: true,
        clear: None,
        apply: |config, value| {
            config.enable_extractor = parse_on_off(value)?;
            Ok(())
        },
        format: |config| format_on_off(config.enable_extractor),
    },
    Setting {
        key: "debug-bios",
        label: "Debug BIOS",
        help: "PC-98 BIOS ROM image for hardware test programs.",
        scope: Scope::System,
        kind: ValueKind::Path(PathKind::File),
        families: Families::PC98,
        model_limits: &[],
        section: None,
        explicit: None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.debug_bios = parse_optional_path(value);
            Ok(())
        },
        format: |config| format_path(config.debug_bios.as_ref()),
    },
    Setting {
        key: BACKEND_KEY,
        label: "Backend",
        help: "Rendering backend. The legacy backend has no shader effects.",
        scope: Scope::Host,
        kind: ValueKind::Choice(BACKEND_CHOICES),
        families: Families::ALL,
        model_limits: &[],
        section: Some(Section::VideoAndAudio),
        explicit: None,
        frontend_only: true,
        clear: None,
        apply: |config, value| {
            config.backend = value.parse()?;
            Ok(())
        },
        format: |config| format_value(config.backend),
    },
    Setting {
        key: AUDIO_VOLUME_KEY,
        label: "Audio volume",
        help: "Audio volume.",
        scope: Scope::Host,
        kind: ValueKind::Number(AUDIO_VOLUME_RANGE),
        families: Families::ALL,
        model_limits: &[],
        section: Some(Section::VideoAndAudio),
        explicit: None,
        frontend_only: true,
        clear: None,
        apply: |config, value| {
            config.audio_volume = parse_audio_volume(value)?;
            Ok(())
        },
        format: |config| format_decimal(config.audio_volume),
    },
    Setting {
        key: CONFIG_FOLDER_KEY,
        label: "Config folder",
        help: "Folder of the system defaults and the game files. The default lies in the root.",
        scope: Scope::Host,
        kind: ValueKind::Path(PathKind::Folder),
        families: Families::ALL,
        model_limits: &[],
        section: Some(Section::Folders),
        explicit: None,
        frontend_only: true,
        clear: None,
        apply: |config, value| {
            config.config_folder = parse_folder(value)?;
            Ok(())
        },
        format: |config| format_path(Some(&config.config_folder)),
    },
    Setting {
        key: ROMS_FOLDER_KEY,
        label: "ROM folder",
        help: "Folder of the ROM folders of every system. The default lies in the root.",
        scope: Scope::Host,
        kind: ValueKind::Path(PathKind::Folder),
        families: Families::ALL,
        model_limits: &[],
        section: Some(Section::Folders),
        explicit: None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.roms_folder = parse_folder(value)?;
            Ok(())
        },
        format: |config| format_path(Some(&config.roms_folder)),
    },
    Setting {
        key: GUI_FONT_KEY,
        label: "GUI font",
        help: "Font file of the user interface. The default lies in the resource folder of the installation.",
        scope: Scope::Host,
        kind: ValueKind::Path(PathKind::File),
        families: Families::ALL,
        model_limits: &[],
        section: Some(Section::Folders),
        explicit: None,
        frontend_only: true,
        clear: None,
        apply: |config, value| {
            config.gui_font = parse_file(value)?;
            Ok(())
        },
        format: |config| format_path(Some(&config.gui_font)),
    },
    Setting {
        key: "at-roms",
        label: "PC/AT",
        help: "Folder with the PC/AT ROM set. Every ROM is optional.",
        scope: Scope::Host,
        kind: ValueKind::Path(PathKind::Folder),
        families: Families::AT,
        model_limits: &[],
        section: Some(Section::Roms),
        explicit: None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.at_roms = parse_optional_path(value);
            Ok(())
        },
        format: |config| format_path(config.at_roms.as_ref()),
    },
    Setting {
        key: "fm7-roms",
        label: "FM-7",
        help: "Folder with the FM-7 and FM77AV ROM set (required).",
        scope: Scope::Host,
        kind: ValueKind::Path(PathKind::Folder),
        families: Families::FM7,
        model_limits: &[],
        section: Some(Section::Roms),
        explicit: None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.fm7_roms = parse_optional_path(value);
            Ok(())
        },
        format: |config| format_path(config.fm7_roms.as_ref()),
    },
    Setting {
        key: "msx-roms",
        label: "MSX",
        help: "Folder with the MSX machine and FS-CA1 ROM sets (required).",
        scope: Scope::Host,
        kind: ValueKind::Path(PathKind::Folder),
        families: Families::MSX,
        model_limits: &[],
        section: Some(Section::Roms),
        explicit: None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.msx_roms = parse_optional_path(value);
            Ok(())
        },
        format: |config| format_path(config.msx_roms.as_ref()),
    },
    Setting {
        key: "mt32-roms",
        label: "MT-32",
        help: "Folder with the Roland MT-32 ROM set.",
        scope: Scope::Host,
        kind: ValueKind::Path(PathKind::Folder),
        families: MIDI_FAMILIES,
        model_limits: &[],
        section: Some(Section::Roms),
        explicit: None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.mt32_roms = parse_optional_path(value);
            Ok(())
        },
        format: |config| format_path(config.mt32_roms.as_ref()),
    },
    Setting {
        key: "pc6000-roms",
        label: "PC-6000",
        help: "Folder with the PC-6000 ROM set (required).",
        scope: Scope::Host,
        kind: ValueKind::Path(PathKind::Folder),
        families: Families::PC60,
        model_limits: &[],
        section: Some(Section::Roms),
        explicit: None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.pc6000_roms = parse_optional_path(value);
            Ok(())
        },
        format: |config| format_path(config.pc6000_roms.as_ref()),
    },
    Setting {
        key: "pc88-roms",
        label: "PC-88",
        help: "Folder with the PC-8801MC ROM set (required).",
        scope: Scope::Host,
        kind: ValueKind::Path(PathKind::Folder),
        families: Families::PC88,
        model_limits: &[],
        section: Some(Section::Roms),
        explicit: None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.pc88_roms = parse_optional_path(value);
            Ok(())
        },
        format: |config| format_path(config.pc88_roms.as_ref()),
    },
    Setting {
        key: "pc88va-roms",
        label: "PC-88VA",
        help: "Folder with the PC-88VA2 ROM set (required).",
        scope: Scope::Host,
        kind: ValueKind::Path(PathKind::Folder),
        families: Families::PC88VA,
        model_limits: &[],
        section: Some(Section::Roms),
        explicit: None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.pc88va_roms = parse_optional_path(value);
            Ok(())
        },
        format: |config| format_path(config.pc88va_roms.as_ref()),
    },
    Setting {
        key: "pc98-roms",
        label: "PC-98",
        help: "Folder with the PC-98 ROM set. Every ROM is optional.",
        scope: Scope::Host,
        kind: ValueKind::Path(PathKind::Folder),
        families: Families::PC98,
        model_limits: &[],
        section: Some(Section::Roms),
        explicit: None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.pc98_roms = parse_optional_path(value);
            Ok(())
        },
        format: |config| format_path(config.pc98_roms.as_ref()),
    },
    Setting {
        key: "sc55-roms",
        label: "SC-55",
        help: "Folder with the Roland SC-55 ROM set.",
        scope: Scope::Host,
        kind: ValueKind::Path(PathKind::Folder),
        families: MIDI_FAMILIES,
        model_limits: &[],
        section: Some(Section::Roms),
        explicit: None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.sc55_roms = parse_optional_path(value);
            Ok(())
        },
        format: |config| format_path(config.sc55_roms.as_ref()),
    },
    Setting {
        key: "towns-roms",
        label: "FM Towns",
        help: "Folder with the FM Towns ROM set (required).",
        scope: Scope::Host,
        kind: ValueKind::Path(PathKind::Folder),
        families: Families::TOWNS,
        model_limits: &[],
        section: Some(Section::Roms),
        explicit: None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.towns_roms = parse_optional_path(value);
            Ok(())
        },
        format: |config| format_path(config.towns_roms.as_ref()),
    },
    Setting {
        key: "x1-roms",
        label: "X1",
        help: "Folder with the Sharp X1 ROM set (required).",
        scope: Scope::Host,
        kind: ValueKind::Path(PathKind::Folder),
        families: Families::X1,
        model_limits: &[],
        section: Some(Section::Roms),
        explicit: None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.x1_roms = parse_optional_path(value);
            Ok(())
        },
        format: |config| format_path(config.x1_roms.as_ref()),
    },
    Setting {
        key: "x68k-roms",
        label: "X68000",
        help: "Folder with the ROM set of the selected X68000 model (required).",
        scope: Scope::Host,
        kind: ValueKind::Path(PathKind::Folder),
        families: Families::X68K,
        model_limits: &[],
        section: Some(Section::Roms),
        explicit: None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.x68k_roms = parse_optional_path(value);
            Ok(())
        },
        format: |config| format_path(config.x68k_roms.as_ref()),
    },
];

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{super::first_system_config, *};

    /// Returns sample values of a setting that its apply function accepts.
    fn sample_values(setting: &Setting) -> Vec<String> {
        match setting.kind {
            ValueKind::Choice(choices) => choices
                .iter()
                .filter(|choice| choice.families.intersects(Families::compiled()))
                .map(|choice| choice.value.to_owned())
                .collect(),
            ValueKind::OnOff => vec!["on".to_owned(), "off".to_owned()],
            ValueKind::Number(range) => [range.minimum, range.maximum]
                .iter()
                .map(|value| format_number(*value, range.integer))
                .collect(),
            ValueKind::Path(_) => vec!["images/disk.img".to_owned()],
            ValueKind::PathList => vec!["images/disk.img".to_owned()],
        }
    }

    /// Formats a sample number the way the format function of its setting writes it.
    fn format_number(value: f64, integer: bool) -> String {
        if integer {
            format!("{value:.0}")
        } else {
            format!("{value:?}")
        }
    }

    /// Returns whether `name` is a model of `family` in the model list.
    fn is_model_of(name: &str, family: Families) -> bool {
        MODEL_CHOICES
            .iter()
            .any(|choice| choice.value == name && choice.families == family)
    }

    /// Verifies that every key is unique and has a label, a help sentence and families.
    #[test]
    fn keys_are_unique_and_described() {
        let mut keys = BTreeSet::new();
        for setting in SETTINGS {
            assert!(keys.insert(setting.key), "{} is listed twice", setting.key);
            assert!(!setting.label.is_empty(), "{} has no label", setting.key);
            assert!(
                setting.help.ends_with('.'),
                "{} help is a sentence",
                setting.key
            );
            assert!(setting.help.is_ascii(), "{} help is ASCII", setting.key);
            assert_ne!(setting.families, Families::NONE, "{}", setting.key);
        }
    }

    /// Verifies that the format function writes back every applied sample value.
    #[test]
    fn format_round_trips_every_key() {
        for setting in SETTINGS {
            for value in sample_values(setting) {
                let mut config = first_system_config();
                (setting.apply)(&mut config, &value).expect("sample value applies");
                let formatted = (setting.format)(&config);
                assert_eq!(formatted, vec![value.clone()], "{} = {value}", setting.key);

                let mut reparsed = first_system_config();
                for formatted_value in &formatted {
                    (setting.apply)(&mut reparsed, formatted_value)
                        .expect("formatted value applies");
                }
                assert_eq!(reparsed, config, "{} = {value}", setting.key);
            }
        }
    }

    /// Verifies that path lists append each value and format every value in order.
    #[test]
    fn path_lists_append_and_format_every_value() {
        for setting in SETTINGS
            .iter()
            .filter(|setting| setting.kind == ValueKind::PathList)
        {
            let mut config = first_system_config();
            assert!((setting.format)(&config).is_empty(), "{}", setting.key);
            (setting.apply)(&mut config, "first.img").expect("path applies");
            (setting.apply)(&mut config, "second.img").expect("path applies");
            assert_eq!(
                (setting.format)(&config),
                vec!["first.img".to_owned(), "second.img".to_owned()],
                "{}",
                setting.key
            );
        }
    }

    /// Verifies that the clear function empties a path list before a later layer applies its values.
    #[test]
    fn clear_replaces_a_path_list() {
        for setting in SETTINGS {
            let Some(clear) = setting.clear else {
                assert_ne!(
                    setting.kind,
                    ValueKind::PathList,
                    "{} has no clear",
                    setting.key
                );
                continue;
            };
            assert_eq!(setting.kind, ValueKind::PathList, "{}", setting.key);
            let mut config = first_system_config();
            (setting.apply)(&mut config, "first.img").expect("path applies");
            (setting.apply)(&mut config, "second.img").expect("path applies");
            clear(&mut config);
            assert!((setting.format)(&config).is_empty(), "{}", setting.key);
            (setting.apply)(&mut config, "third.img").expect("path applies");
            assert_eq!(
                (setting.format)(&config),
                vec!["third.img".to_owned()],
                "{}",
                setting.key
            );
        }
    }

    /// Verifies the empty value of every path kind: unset, nothing appended or an error for a folder key.
    #[test]
    fn empty_path_values_unset_or_fail() {
        for setting in SETTINGS {
            let mut config = first_system_config();
            match setting.kind {
                ValueKind::Path(_)
                    if [CONFIG_FOLDER_KEY, ROMS_FOLDER_KEY, GUI_FONT_KEY]
                        .contains(&setting.key) =>
                {
                    assert!((setting.apply)(&mut config, "").is_err(), "{}", setting.key);
                }
                ValueKind::Path(_) => {
                    (setting.apply)(&mut config, "images/disk.img").expect("path applies");
                    (setting.apply)(&mut config, "").expect("empty path applies");
                    assert!((setting.format)(&config).is_empty(), "{}", setting.key);
                }
                ValueKind::PathList => {
                    (setting.apply)(&mut config, "").expect("empty path applies");
                    assert!((setting.format)(&config).is_empty(), "{}", setting.key);
                }
                ValueKind::Choice(_) | ValueKind::OnOff | ValueKind::Number(_) => {}
            }
        }
    }

    /// Verifies that keys outside the table are not found.
    #[test]
    fn unknown_keys_are_not_found() {
        assert!(find_setting("fdd3").is_none());
        assert!(find_setting("sound-board").is_none());
        assert!(find_setting("key.A").is_none());
        assert!(find_setting("").is_none());
    }

    /// Verifies that every choice, on/off and number setting rejects an invalid value.
    #[test]
    fn invalid_values_are_errors() {
        for setting in SETTINGS {
            let invalid = match setting.kind {
                ValueKind::Choice(_) | ValueKind::OnOff => "bogus",
                ValueKind::Number(_) => "99",
                ValueKind::Path(_) | ValueKind::PathList => continue,
            };
            let mut config = first_system_config();
            assert!(
                (setting.apply)(&mut config, invalid).is_err(),
                "{} accepts {invalid}",
                setting.key
            );
        }
        let mut config = first_system_config();
        let bios = find_setting("bios").expect("bios is a table key");
        assert_eq!(
            (bios.apply)(&mut config, "maybe"),
            Err("invalid value 'maybe', expected on or off".to_owned())
        );
        let volume = find_setting("audio-volume").expect("audio-volume is a table key");
        assert!((volume.apply)(&mut config, "-0.1").is_err());
        assert!((volume.apply)(&mut config, "NaN").is_err());
    }

    /// Verifies that every compiled model name resolves to a model with the same name.
    #[test]
    fn every_compiled_model_resolves_to_its_own_name() {
        for name in compiled_model_names() {
            let mut config = first_system_config();
            resolve_model(&mut config, name).expect("model name resolves");
            assert_eq!(model_name(&config), name);
            let lowercase = name.to_ascii_lowercase();
            let mut config = first_system_config();
            resolve_model(&mut config, &lowercase).expect("model names ignore case");
            assert_eq!(model_name(&config), name);
            let choice = MODEL_CHOICES
                .iter()
                .find(|choice| choice.value == name)
                .expect("model is a choice");
            assert!(choice.families.contains_target(config.target), "{name}");
        }
    }

    /// Verifies that the model limits and choice families match the families of each key.
    #[test]
    fn filter_data_is_consistent() {
        for setting in SETTINGS {
            for limit in setting.model_limits {
                assert!(
                    setting.families.includes(limit.family),
                    "{} limits a family it does not read",
                    setting.key
                );
                for model in limit.models {
                    assert!(
                        is_model_of(model, limit.family),
                        "{} lists the unknown model {model}",
                        setting.key
                    );
                }
            }
            if let ValueKind::Choice(choices) = setting.kind {
                for choice in choices {
                    assert!(
                        setting.families.intersects(choice.families),
                        "{} choice {} matches no family of the key",
                        setting.key,
                        choice.value
                    );
                }
            }
        }
    }

    /// Verifies that every family accepts every key at apply time.
    #[test]
    fn family_filter_rejects_nothing_at_apply_time() {
        let mut config = first_system_config();
        for setting in SETTINGS.iter().filter(|setting| setting.key != MACHINE_KEY) {
            for value in sample_values(setting) {
                (setting.apply)(&mut config, &value).expect("every family accepts every key");
            }
        }
    }

    #[cfg(feature = "pc98")]
    /// Verifies that the model limits select the PC-98 models that read a key.
    #[test]
    fn model_limits_name_the_models_that_read_a_key() {
        let cpu_mode = find_setting("cpu-mode").expect("cpu-mode is a table key");
        assert!(cpu_mode.is_read_by(Target::Pc98, "PC9801VM"));
        assert!(!cpu_mode.is_read_by(Target::Pc98, "PC9801RA"));
        let soundboard = find_setting("soundboard").expect("soundboard is a table key");
        assert!(soundboard.is_read_by(Target::Pc98, "PC9821AP"));
        let boot_device = find_setting("boot-device").expect("boot-device is a table key");
        assert_eq!(boot_device.choices_for(Target::Pc98).count(), 6);
    }

    #[cfg(feature = "at")]
    /// Verifies that the choice families limit the PC/AT boot devices.
    #[test]
    fn choice_families_limit_the_pc_at_boot_devices() {
        let boot_device = find_setting("boot-device").expect("boot-device is a table key");
        let values: Vec<&str> = boot_device
            .choices_for(Target::At)
            .map(|choice| choice.value)
            .collect();
        assert_eq!(values, ["auto", "fdd1", "hdd1"]);
        let soundboard = find_setting("soundboard").expect("soundboard is a table key");
        assert!(!soundboard.is_read_by(Target::At, "AT486DX66"));
    }

    #[cfg(feature = "pc88")]
    /// Verifies that an explicit CPU mode survives the PC-88 derived defaults.
    #[test]
    fn explicit_settings_keep_their_value_over_derived_defaults() {
        let mut config = first_system_config();
        let mut explicit = ExplicitSettings::default();
        for (key, value) in [
            (MACHINE_KEY, "PC8801MC"),
            ("boot-mode", "v1s"),
            ("cpu-mode", "high"),
        ] {
            let setting = find_setting(key).expect("table key");
            explicit
                .apply(&mut config, setting, value)
                .expect("value applies");
        }
        apply_derived_defaults(&mut config, explicit);
        assert_eq!(config.cpu_mode, CpuMode::High);
        assert_eq!(config.pc88_memory_wait, MemoryWaitSwitch::Compatible);
    }
}
