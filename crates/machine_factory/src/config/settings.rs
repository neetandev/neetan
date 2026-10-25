//! The settings table: one declarative entry for every key of `EmulatorConfig`.
//!
//! An entry holds the key, its scope, its value kind with the choices, the
//! families and models that read it, its section, and the functions that apply,
//! format and clear its value. The language files hold the label and the help
//! text of each key.

use std::path::{Path, PathBuf};

use common::CpuMode;
#[cfg(feature = "pc88")]
use common::MemoryWaitSwitch;
use device::gameport_towns::TownsPadType;

pub mod file_types;
pub mod model_limits;
pub mod value_error;

use self::{
    file_types::{
        CARTRIDGE_FILES, CASSETTE_FILES, CDROM_FILES, FLOPPY_FILES, FileTypes, HARD_DISK_FILES,
    },
    model_limits::{
        ChoiceFact, FactData, LimitReason, MSX_FDD_MODELS, ModelLimit, PC60_FDD_MODELS,
        PC98_CDROM_MODELS, PC98_CPU_MODE_MODELS, PC98_REAL_BIOS_MODELS, PC98_VX_AND_LATER_MODELS,
        TOWNS_CPU_MODE_MODELS, X1_TURBO_MODELS, X68K_CDROM_MODELS, X68K_CPU_MODE_MODELS,
    },
    value_error::ValueError,
};
#[cfg(feature = "pc88")]
use super::BootMode;
use super::{
    EmulatorConfig, MachineSelection, Target, model_name, phrases::Megahertz, resolve_model,
    systems::SYSTEMS,
};

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

/// The value that the machines of a set of families take for an optional setting that no layer
/// sets.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct UnsetValue {
    /// The families that take the value.
    pub families: Families,
    /// The value as the file writes it, or the key of the word for a value that the machine
    /// picks.
    pub text: UnsetText,
}

/// The text of an unset value.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum UnsetText {
    /// The value as the configuration file writes it.
    Value(&'static str),
    /// A word for a value that the machine picks. The word is a key of the section `[unset]` of
    /// the language files.
    Word(&'static str),
}

/// One value of a choice setting.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Choice {
    /// The value as the configuration file writes it.
    pub value: &'static str,
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
    /// `auto` or a language tag of the language files of the user interface.
    Language,
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
    /// The key of the setting, for example `cpu-mode`. The section `[setting.<key>]` of the
    /// language files holds its label and its help text.
    pub key: &'static str,
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
    /// The files that a path setting takes, one entry for each set of families.
    pub file_types: &'static [FileTypes],
    /// The values that the machines take while no layer sets an optional setting.
    pub unset_values: &'static [UnsetValue],
    /// The fact that the note of a choice setting adds.
    pub fact: ChoiceFact,
    /// Whether only the interactive frontend reads the setting.
    pub frontend_only: bool,
    /// Empties a path list. Every other kind has `None`.
    pub clear: Option<fn(&mut EmulatorConfig)>,
    /// Applies one value to the configuration. A path list appends the value.
    pub apply: fn(&mut EmulatorConfig, &str) -> Result<(), ValueError>,
    /// Formats the configured value. A path list returns one value for each file, an unset value returns none.
    pub format: fn(&EmulatorConfig) -> Vec<String>,
}

/// The families whose machines derive the defaults of the settings with an [`ExplicitKey`].
const DERIVED_DEFAULT_FAMILIES: Families = Families::PC88;

impl Setting {
    /// Returns whether the machine `model` of the family `target` reads this setting.
    #[must_use]
    pub fn is_read_by(&self, target: Target, model: &str) -> bool {
        if !self.families.contains_target(target) {
            return false;
        }
        self.limit_for(target).is_none_or(|limit| {
            limit
                .models
                .iter()
                .any(|name| name.eq_ignore_ascii_case(model))
        })
    }

    /// Returns the model limit of the family `target`. A family without a limit reads the setting
    /// on every model.
    #[must_use]
    pub fn limit_for(&self, target: Target) -> Option<&'static ModelLimit> {
        self.model_limits
            .iter()
            .find(|limit| limit.family.contains_target(target))
    }

    /// Returns the files that the path setting takes on the family `target`.
    #[must_use]
    pub fn file_types_for(&self, target: Target) -> Option<&'static FileTypes> {
        self.file_types
            .iter()
            .find(|types| types.families.contains_target(target))
    }

    /// Returns the value that the machines of the family `target` take while no layer sets
    /// the setting.
    #[must_use]
    pub fn unset_text(&self, target: Target) -> Option<UnsetText> {
        self.unset_values
            .iter()
            .find(|unset| unset.families.contains_target(target))
            .map(|unset| unset.text)
    }

    /// Returns the choices of a choice setting that the family `target` accepts.
    pub fn choices_for(&self, target: Target) -> impl Iterator<Item = &'static Choice> {
        self.choices_for_families(Families::of(target))
    }

    /// Returns the choices of a choice setting that one of the families `families` accepts.
    pub fn choices_for_families(
        &self,
        families: Families,
    ) -> impl Iterator<Item = &'static Choice> {
        let choices: &'static [Choice] = match self.kind {
            ValueKind::Choice(choices) => choices,
            ValueKind::OnOff
            | ValueKind::Number(_)
            | ValueKind::Path(_)
            | ValueKind::PathList
            | ValueKind::Language => &[],
        };
        choices
            .iter()
            .filter(move |choice| choice.families.intersects(families))
    }

    /// Returns the data of the fact about the values of the choice setting on `model` while the
    /// file holds `value`.
    #[must_use]
    pub fn choice_fact(&self, model: MachineSelection, value: Option<&str>) -> Option<FactData> {
        match self.fact {
            ChoiceFact::None => None,
            ChoiceFact::Choices => {
                let values: Vec<&'static str> = self
                    .choices_for(model.target())
                    .map(|choice| choice.value)
                    .collect();
                (!values.is_empty()).then_some(FactData::Choices(values))
            }
            ChoiceFact::CpuClock => {
                let mode = value?.parse::<CpuMode>().ok()?;
                let hertz = model.cpu_clock_hz(mode)?;
                Some(FactData::CpuClock {
                    mode,
                    clock: Megahertz(hertz),
                })
            }
        }
    }

    /// Returns whether the default of the setting derives from other settings on the family
    /// `target`.
    #[must_use]
    pub fn has_derived_default(&self, target: Target) -> bool {
        self.explicit.is_some() && DERIVED_DEFAULT_FAMILIES.contains_target(target)
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
    ) -> Result<(), ValueError> {
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
    if !DERIVED_DEFAULT_FAMILIES.contains_target(config.target) {
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
/// Key of the theme of the user interface.
pub const GUI_THEME_KEY: &str = "gui-theme";
/// Key of the language of the user interface.
pub const GUI_LANGUAGE_KEY: &str = "gui-language";
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
fn parse_on_off(value: &str) -> Result<bool, ValueError> {
    match value {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err(ValueError::OnOff {
            value: value.to_owned(),
        }),
    }
}

/// Parses a value of a choice setting with the choices `choices`.
fn parse_choice<T: std::str::FromStr>(
    value: &str,
    choices: &'static [Choice],
) -> Result<T, ValueError> {
    value
        .parse()
        .map_err(|_| ValueError::unknown_choice(value, choices))
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
fn parse_folder(value: &str) -> Result<PathBuf, ValueError> {
    if value.is_empty() {
        return Err(ValueError::EmptyFolder);
    }
    Ok(PathBuf::from(value))
}

/// Parses a required file path. An empty value is an error.
fn parse_file(value: &str) -> Result<PathBuf, ValueError> {
    if value.is_empty() {
        return Err(ValueError::EmptyFile);
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

/// Returns the error of `value` outside `range`.
fn out_of_range(value: &str, range: NumberRange) -> ValueError {
    ValueError::OutOfRange {
        value: value.to_owned(),
        range,
    }
}

/// Parses the PC-6000 composite phase.
fn parse_composite_phase(value: &str) -> Result<u32, ValueError> {
    match value.parse::<u32>() {
        Ok(phase) if COMPOSITE_PHASE_RANGE.contains(f64::from(phase)) => Ok(phase),
        _ => Err(out_of_range(value, COMPOSITE_PHASE_RANGE)),
    }
}

/// Parses the audio volume.
pub fn parse_audio_volume(value: &str) -> Result<f32, ValueError> {
    match value.parse::<f32>() {
        Ok(volume) if AUDIO_VOLUME_RANGE.contains(f64::from(volume)) => Ok(volume),
        _ => Err(out_of_range(value, AUDIO_VOLUME_RANGE)),
    }
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

/// The boot modes that the machines take without `boot-mode`.
const BOOT_MODE_UNSET: &[UnsetValue] = &[
    UnsetValue {
        families: Families::PC88,
        text: UnsetText::Value("v2"),
    },
    UnsetValue {
        families: Families::FM7,
        text: UnsetText::Value("basic"),
    },
];

/// The GDC clock without `force-gdc-clock`: the machine picks it.
const GDC_CLOCK_UNSET: &[UnsetValue] = &[UnsetValue {
    families: Families::PC98,
    text: UnsetText::Word("auto"),
}];

/// Every machine model, with the family that builds it.
pub const MODEL_CHOICES: &[Choice] = &[
    Choice {
        value: "PC9801F",
        families: Families::PC98,
    },
    Choice {
        value: "PC9801VM",
        families: Families::PC98,
    },
    Choice {
        value: "PC9801VX",
        families: Families::PC98,
    },
    Choice {
        value: "PC9801RS",
        families: Families::PC98,
    },
    Choice {
        value: "PC9801RA",
        families: Families::PC98,
    },
    Choice {
        value: "PC9821AS",
        families: Families::PC98,
    },
    Choice {
        value: "PC9821AP",
        families: Families::PC98,
    },
    Choice {
        value: "PC8801MC",
        families: Families::PC88,
    },
    Choice {
        value: "PC88VA2",
        families: Families::PC88VA,
    },
    Choice {
        value: "PC6001",
        families: Families::PC60,
    },
    Choice {
        value: "PC6001MK2",
        families: Families::PC60,
    },
    Choice {
        value: "PC6601",
        families: Families::PC60,
    },
    Choice {
        value: "PC6001MK2SR",
        families: Families::PC60,
    },
    Choice {
        value: "PC6601SR",
        families: Families::PC60,
    },
    Choice {
        value: "MSX",
        families: Families::MSX,
    },
    Choice {
        value: "MSX2",
        families: Families::MSX,
    },
    Choice {
        value: "MSX2PLUS",
        families: Families::MSX,
    },
    Choice {
        value: "FMTowns",
        families: Families::TOWNS,
    },
    Choice {
        value: "FMTownsIICX",
        families: Families::TOWNS,
    },
    Choice {
        value: "FMTownsIIMX",
        families: Families::TOWNS,
    },
    Choice {
        value: "X68000",
        families: Families::X68K,
    },
    Choice {
        value: "X68000SUPER",
        families: Families::X68K,
    },
    Choice {
        value: "X68000XVI",
        families: Families::X68K,
    },
    Choice {
        value: "X1",
        families: Families::X1,
    },
    Choice {
        value: "X1TURBO",
        families: Families::X1,
    },
    Choice {
        value: "FM7",
        families: Families::FM7,
    },
    Choice {
        value: "FM77AV",
        families: Families::FM7,
    },
    Choice {
        value: "AT486DX50",
        families: Families::AT,
    },
    Choice {
        value: "AT486DX66",
        families: Families::AT,
    },
];

/// CPU clock modes.
const CPU_MODE_CHOICES: &[Choice] = &[
    Choice {
        value: "low",
        families: Families::ALL,
    },
    Choice {
        value: "high",
        families: Families::ALL,
    },
];

/// Boot modes of the PC-88 and the FM-7.
const BOOT_MODE_CHOICES: &[Choice] = &[
    Choice {
        value: "v1s",
        families: Families::PC88,
    },
    Choice {
        value: "v1h",
        families: Families::PC88,
    },
    Choice {
        value: "v2",
        families: Families::PC88,
    },
    Choice {
        value: "n",
        families: Families::PC88,
    },
    Choice {
        value: "n80",
        families: Families::PC88,
    },
    Choice {
        value: "n80sr",
        families: Families::PC88,
    },
    Choice {
        value: "basic",
        families: Families::FM7,
    },
    Choice {
        value: "dos",
        families: Families::FM7,
    },
];

/// Boot devices.
const BOOT_DEVICE_CHOICES: &[Choice] = &[
    Choice {
        value: "auto",
        families: BOOT_DEVICE_FAMILIES,
    },
    Choice {
        value: "fdd1",
        families: BOOT_DEVICE_FAMILIES,
    },
    Choice {
        value: "fdd2",
        families: FULL_BOOT_DEVICE_FAMILIES,
    },
    Choice {
        value: "hdd1",
        families: BOOT_DEVICE_FAMILIES,
    },
    Choice {
        value: "hdd2",
        families: FULL_BOOT_DEVICE_FAMILIES,
    },
    Choice {
        value: "dos",
        families: FULL_BOOT_DEVICE_FAMILIES,
    },
];

/// Monitor timings.
const MONITOR_CHOICES: &[Choice] = &[
    Choice {
        value: "auto",
        families: Families::ALL,
    },
    Choice {
        value: "15k",
        families: Families::ALL,
    },
    Choice {
        value: "24k",
        families: Families::ALL,
    },
];

/// PC-88 wait modes.
#[cfg(feature = "pc88")]
const WAIT_CHOICES: &[Choice] = &[
    Choice {
        value: "fast",
        families: Families::ALL,
    },
    Choice {
        value: "compatible",
        families: Families::ALL,
    },
];

/// X1 turbo keyboard mode switch positions.
const X1_KEYBOARD_CHOICES: &[Choice] = &[
    Choice {
        value: "A",
        families: Families::ALL,
    },
    Choice {
        value: "B",
        families: Families::ALL,
    },
];

/// FM Towns pad types.
const TOWNS_PAD_CHOICES: &[Choice] = &[
    Choice {
        value: "2",
        families: Families::ALL,
    },
    Choice {
        value: "6",
        families: Families::ALL,
    },
];

/// PC-98 sound boards.
const SOUNDBOARD_CHOICES: &[Choice] = &[
    Choice {
        value: "none",
        families: Families::ALL,
    },
    Choice {
        value: "14",
        families: Families::ALL,
    },
    Choice {
        value: "26k",
        families: Families::ALL,
    },
    Choice {
        value: "86",
        families: Families::ALL,
    },
    Choice {
        value: "86+26k",
        families: Families::ALL,
    },
    Choice {
        value: "sb16",
        families: Families::ALL,
    },
    Choice {
        value: "sb16+26k",
        families: Families::ALL,
    },
];

/// MIDI devices.
const MIDI_CHOICES: &[Choice] = &[
    Choice {
        value: "none",
        families: Families::ALL,
    },
    Choice {
        value: "mt32",
        families: Families::ALL,
    },
    Choice {
        value: "sc55",
        families: Families::ALL,
    },
];

/// Display aspect modes.
const ASPECT_MODE_CHOICES: &[Choice] = &[
    Choice {
        value: "4:3",
        families: Families::ALL,
    },
    Choice {
        value: "1:1",
        families: Families::ALL,
    },
];

/// Window modes.
const WINDOW_MODE_CHOICES: &[Choice] = &[
    Choice {
        value: "windowed",
        families: Families::ALL,
    },
    Choice {
        value: "fullscreen",
        families: Families::ALL,
    },
];

/// Scaling methods.
const SCALING_CHOICES: &[Choice] = &[
    Choice {
        value: "nearest",
        families: Families::ALL,
    },
    Choice {
        value: "bilinear",
        families: Families::ALL,
    },
    Choice {
        value: "pixelart",
        families: Families::ALL,
    },
];

/// Forced GDC clocks.
const GDC_CLOCK_CHOICES: &[Choice] = &[
    Choice {
        value: "2.5",
        families: Families::ALL,
    },
    Choice {
        value: "5",
        families: Families::ALL,
    },
];

/// PC-98 graphics boards.
const GRAPHICBOARD_CHOICES: &[Choice] = &[
    Choice {
        value: "none",
        families: Families::ALL,
    },
    Choice {
        value: "ga1280a",
        families: Families::ALL,
    },
];

/// Rendering backends.
const BACKEND_CHOICES: &[Choice] = &[
    Choice {
        value: "modern",
        families: Families::ALL,
    },
    Choice {
        value: "legacy",
        families: Families::ALL,
    },
];

/// Themes of the user interface.
const GUI_THEME_CHOICES: &[Choice] = &[
    Choice {
        value: "dark",
        families: Families::ALL,
    },
    Choice {
        value: "light",
        families: Families::ALL,
    },
];

/// Every setting, ordered by section.
pub static SETTINGS: &[Setting] = &[
    Setting {
        key: MACHINE_KEY,
        scope: Scope::System,
        kind: ValueKind::Choice(MODEL_CHOICES),
        families: Families::ALL,
        model_limits: &[],
        section: Some(Section::Machine),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
        frontend_only: false,
        clear: None,
        apply: resolve_model,
        format: |config| vec![model_name(config).to_owned()],
    },
    Setting {
        key: "cpu-mode",
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
                reason: LimitReason::FixedClock,
            },
            ModelLimit {
                family: Families::TOWNS,
                models: TOWNS_CPU_MODE_MODELS,
                reason: LimitReason::FixedClock,
            },
            ModelLimit {
                family: Families::X68K,
                models: X68K_CPU_MODE_MODELS,
                reason: LimitReason::FixedClock,
            },
        ],
        section: Some(Section::Machine),
        explicit: Some(ExplicitKey::CpuMode),
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::CpuClock,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.cpu_mode = parse_choice(value, CPU_MODE_CHOICES)?;
            Ok(())
        },
        format: |config| format_value(config.cpu_mode),
    },
    Setting {
        key: "boot-mode",
        scope: Scope::System,
        kind: ValueKind::Choice(BOOT_MODE_CHOICES),
        families: Families::PC88.with(Families::FM7),
        model_limits: &[],
        section: Some(Section::Machine),
        explicit: None,
        file_types: &[],
        unset_values: BOOT_MODE_UNSET,
        fact: ChoiceFact::Choices,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.boot_mode = Some(parse_choice(value, BOOT_MODE_CHOICES)?);
            Ok(())
        },
        format: |config| format_optional(config.boot_mode),
    },
    Setting {
        key: "boot-device",
        scope: Scope::System,
        kind: ValueKind::Choice(BOOT_DEVICE_CHOICES),
        families: BOOT_DEVICE_FAMILIES,
        model_limits: &[],
        section: Some(Section::Machine),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.boot_device = parse_choice(value, BOOT_DEVICE_CHOICES)?;
            Ok(())
        },
        format: |config| format_value(config.boot_device),
    },
    Setting {
        key: "bios",
        scope: Scope::System,
        kind: ValueKind::OnOff,
        families: Families::PC98.with(Families::AT),
        model_limits: &[ModelLimit {
            family: Families::PC98,
            models: PC98_REAL_BIOS_MODELS,
            reason: LimitReason::Phrase("builtin-bios"),
        }],
        section: Some(Section::Machine),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
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
        scope: Scope::System,
        kind: ValueKind::OnOff,
        families: Families::PC98,
        model_limits: &[ModelLimit {
            family: Families::PC98,
            models: PC98_VX_AND_LATER_MODELS,
            reason: LimitReason::Phrase("no-ems"),
        }],
        section: Some(Section::Machine),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
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
        scope: Scope::System,
        kind: ValueKind::OnOff,
        families: Families::PC98,
        model_limits: &[ModelLimit {
            family: Families::PC98,
            models: PC98_VX_AND_LATER_MODELS,
            reason: LimitReason::Phrase("no-xms"),
        }],
        section: Some(Section::Machine),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
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
        scope: Scope::System,
        kind: ValueKind::Choice(MONITOR_CHOICES),
        families: Families::PC88.with(Families::X1),
        model_limits: &[ModelLimit {
            family: Families::X1,
            models: X1_TURBO_MODELS,
            reason: LimitReason::Phrase("fixed-monitor"),
        }],
        section: Some(Section::Machine),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::Choices,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.monitor = parse_choice(value, MONITOR_CHOICES)?;
            Ok(())
        },
        format: |config| format_value(config.monitor),
    },
    #[cfg(feature = "pc88")]
    Setting {
        key: "pc88-memory-wait",
        scope: Scope::System,
        kind: ValueKind::Choice(WAIT_CHOICES),
        families: Families::PC88,
        model_limits: &[],
        section: Some(Section::Machine),
        explicit: Some(ExplicitKey::Pc88MemoryWait),
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.pc88_memory_wait = parse_choice(value, WAIT_CHOICES)?;
            Ok(())
        },
        format: |config| format_value(config.pc88_memory_wait),
    },
    #[cfg(feature = "pc88")]
    Setting {
        key: "pc88-8mhz-wait",
        scope: Scope::System,
        kind: ValueKind::Choice(WAIT_CHOICES),
        families: Families::PC88,
        model_limits: &[],
        section: Some(Section::Machine),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.pc88_8mhz_wait = parse_choice(value, WAIT_CHOICES)?;
            Ok(())
        },
        format: |config| format_value(config.pc88_8mhz_wait),
    },
    Setting {
        key: "x1-keyboard",
        scope: Scope::System,
        kind: ValueKind::Choice(X1_KEYBOARD_CHOICES),
        families: Families::X1,
        model_limits: &[ModelLimit {
            family: Families::X1,
            models: X1_TURBO_MODELS,
            reason: LimitReason::Phrase("no-keyboard-switch"),
        }],
        section: Some(Section::Machine),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.x1_keyboard = parse_choice(value, X1_KEYBOARD_CHOICES)?;
            Ok(())
        },
        format: |config| format_value(config.x1_keyboard),
    },
    Setting {
        key: "towns-pad",
        scope: Scope::System,
        kind: ValueKind::Choice(TOWNS_PAD_CHOICES),
        families: Families::TOWNS,
        model_limits: &[],
        section: Some(Section::Machine),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.towns_pad = parse_choice(value, TOWNS_PAD_CHOICES)?;
            Ok(())
        },
        format: |config| format_towns_pad(config.towns_pad),
    },
    Setting {
        key: "fdd1",
        scope: Scope::Game,
        kind: ValueKind::PathList,
        families: FDD1_FAMILIES,
        model_limits: &[
            ModelLimit {
                family: Families::PC60,
                models: PC60_FDD_MODELS,
                reason: LimitReason::Phrase("no-floppy"),
            },
            ModelLimit {
                family: Families::MSX,
                models: MSX_FDD_MODELS,
                reason: LimitReason::Phrase("no-floppy"),
            },
        ],
        section: Some(Section::Media),
        explicit: None,
        file_types: FLOPPY_FILES,
        unset_values: &[],
        fact: ChoiceFact::None,
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
        scope: Scope::Game,
        kind: ValueKind::PathList,
        families: FDD2_FAMILIES,
        model_limits: &[ModelLimit {
            family: Families::PC60,
            models: PC60_FDD_MODELS,
            reason: LimitReason::Phrase("no-floppy"),
        }],
        section: Some(Section::Media),
        explicit: None,
        file_types: FLOPPY_FILES,
        unset_values: &[],
        fact: ChoiceFact::None,
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
        scope: Scope::Game,
        kind: ValueKind::PathList,
        families: CDROM_FAMILIES,
        model_limits: &[
            ModelLimit {
                family: Families::PC98,
                models: PC98_CDROM_MODELS,
                reason: LimitReason::Phrase("no-cdrom"),
            },
            ModelLimit {
                family: Families::X68K,
                models: X68K_CDROM_MODELS,
                reason: LimitReason::Phrase("no-cdrom"),
            },
        ],
        section: Some(Section::Media),
        explicit: None,
        file_types: CDROM_FILES,
        unset_values: &[],
        fact: ChoiceFact::None,
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
        scope: Scope::System,
        kind: ValueKind::OnOff,
        families: Families::TOWNS,
        model_limits: &[],
        section: Some(Section::Media),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
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
        scope: Scope::Game,
        kind: ValueKind::Path(PathKind::File),
        families: HDD_FAMILIES,
        model_limits: &[],
        section: Some(Section::Media),
        explicit: None,
        file_types: HARD_DISK_FILES,
        unset_values: &[],
        fact: ChoiceFact::None,
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
        scope: Scope::Game,
        kind: ValueKind::Path(PathKind::File),
        families: HDD_FAMILIES,
        model_limits: &[],
        section: Some(Section::Media),
        explicit: None,
        file_types: HARD_DISK_FILES,
        unset_values: &[],
        fact: ChoiceFact::None,
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
        scope: Scope::Game,
        kind: ValueKind::Path(PathKind::File),
        families: Families::PC60.with(Families::MSX),
        model_limits: &[],
        section: Some(Section::Media),
        explicit: None,
        file_types: CARTRIDGE_FILES,
        unset_values: &[],
        fact: ChoiceFact::None,
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
        scope: Scope::Game,
        kind: ValueKind::Path(PathKind::File),
        families: Families::PC60
            .with(Families::MSX)
            .with(Families::X1)
            .with(Families::FM7),
        model_limits: &[],
        section: Some(Section::Media),
        explicit: None,
        file_types: CASSETTE_FILES,
        unset_values: &[],
        fact: ChoiceFact::None,
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
        scope: Scope::System,
        kind: ValueKind::Choice(SOUNDBOARD_CHOICES),
        families: Families::PC98,
        model_limits: &[],
        section: Some(Section::Sound),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.soundboard = parse_choice(value, SOUNDBOARD_CHOICES)?;
            Ok(())
        },
        format: |config| format_value(config.soundboard),
    },
    Setting {
        key: "adpcm-ram",
        scope: Scope::System,
        kind: ValueKind::OnOff,
        families: Families::PC98,
        model_limits: &[],
        section: Some(Section::Sound),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
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
        scope: Scope::System,
        kind: ValueKind::Choice(MIDI_CHOICES),
        families: MIDI_FAMILIES,
        model_limits: &[],
        section: Some(Section::Sound),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.midi = parse_choice(value, MIDI_CHOICES)?;
            Ok(())
        },
        format: |config| format_value(config.midi),
    },
    Setting {
        key: "aspect-mode",
        scope: Scope::System,
        kind: ValueKind::Choice(ASPECT_MODE_CHOICES),
        families: Families::ALL,
        model_limits: &[],
        section: Some(Section::Display),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
        frontend_only: true,
        clear: None,
        apply: |config, value| {
            config.aspect_mode = parse_choice(value, ASPECT_MODE_CHOICES)?;
            Ok(())
        },
        format: |config| format_value(config.aspect_mode),
    },
    Setting {
        key: WINDOW_MODE_KEY,
        scope: Scope::System,
        kind: ValueKind::Choice(WINDOW_MODE_CHOICES),
        families: Families::ALL,
        model_limits: &[],
        section: Some(Section::Display),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
        frontend_only: true,
        clear: None,
        apply: |config, value| {
            config.window_mode = parse_choice(value, WINDOW_MODE_CHOICES)?;
            Ok(())
        },
        format: |config| format_value(config.window_mode),
    },
    Setting {
        key: "crt",
        scope: Scope::System,
        kind: ValueKind::OnOff,
        families: Families::ALL,
        model_limits: &[],
        section: Some(Section::Display),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
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
        scope: Scope::System,
        kind: ValueKind::Choice(SCALING_CHOICES),
        families: Families::ALL,
        model_limits: &[],
        section: Some(Section::Display),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
        frontend_only: true,
        clear: None,
        apply: |config, value| {
            config.scaling = parse_choice(value, SCALING_CHOICES)?;
            Ok(())
        },
        format: |config| format_value(config.scaling),
    },
    Setting {
        key: "pc6000-phase",
        scope: Scope::System,
        kind: ValueKind::Number(COMPOSITE_PHASE_RANGE),
        families: Families::PC60,
        model_limits: &[],
        section: Some(Section::Display),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
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
        scope: Scope::System,
        kind: ValueKind::Choice(GDC_CLOCK_CHOICES),
        families: Families::PC98,
        model_limits: &[ModelLimit {
            family: Families::PC98,
            models: PC98_VX_AND_LATER_MODELS,
            reason: LimitReason::Phrase("fixed-gdc-clock"),
        }],
        section: Some(Section::Display),
        explicit: None,
        file_types: &[],
        unset_values: GDC_CLOCK_UNSET,
        fact: ChoiceFact::None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.force_gdc_clock = Some(parse_choice(value, GDC_CLOCK_CHOICES)?);
            Ok(())
        },
        format: |config| format_optional(config.force_gdc_clock),
    },
    Setting {
        key: "graphicboard",
        scope: Scope::System,
        kind: ValueKind::Choice(GRAPHICBOARD_CHOICES),
        families: Families::PC98,
        model_limits: &[],
        section: Some(Section::Display),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
        frontend_only: false,
        clear: None,
        apply: |config, value| {
            config.graphicboard = parse_choice(value, GRAPHICBOARD_CHOICES)?;
            Ok(())
        },
        format: |config| format_value(config.graphicboard),
    },
    Setting {
        key: "printer",
        scope: Scope::System,
        kind: ValueKind::Path(PathKind::File),
        families: Families::PC98,
        model_limits: &[],
        section: Some(Section::Advanced),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
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
        scope: Scope::System,
        kind: ValueKind::OnOff,
        families: Families::PC98,
        model_limits: &[],
        section: Some(Section::Advanced),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
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
        scope: Scope::System,
        kind: ValueKind::Path(PathKind::File),
        families: Families::PC98,
        model_limits: &[],
        section: None,
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
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
        scope: Scope::Host,
        kind: ValueKind::Choice(BACKEND_CHOICES),
        families: Families::ALL,
        model_limits: &[],
        section: Some(Section::VideoAndAudio),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
        frontend_only: true,
        clear: None,
        apply: |config, value| {
            config.backend = parse_choice(value, BACKEND_CHOICES)?;
            Ok(())
        },
        format: |config| format_value(config.backend),
    },
    Setting {
        key: GUI_THEME_KEY,
        scope: Scope::Host,
        kind: ValueKind::Choice(GUI_THEME_CHOICES),
        families: Families::ALL,
        model_limits: &[],
        section: Some(Section::VideoAndAudio),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
        frontend_only: true,
        clear: None,
        apply: |config, value| {
            config.gui_theme = parse_choice(value, GUI_THEME_CHOICES)?;
            Ok(())
        },
        format: |config| format_value(config.gui_theme),
    },
    Setting {
        key: GUI_LANGUAGE_KEY,
        scope: Scope::Host,
        kind: ValueKind::Language,
        families: Families::ALL,
        model_limits: &[],
        section: Some(Section::VideoAndAudio),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
        frontend_only: true,
        clear: None,
        apply: |config, value| {
            config.gui_language = value.parse().map_err(|_| ValueError::Language {
                value: value.to_owned(),
            })?;
            Ok(())
        },
        format: |config| format_value(&config.gui_language),
    },
    Setting {
        key: AUDIO_VOLUME_KEY,
        scope: Scope::Host,
        kind: ValueKind::Number(AUDIO_VOLUME_RANGE),
        families: Families::ALL,
        model_limits: &[],
        section: Some(Section::VideoAndAudio),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
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
        scope: Scope::Host,
        kind: ValueKind::Path(PathKind::Folder),
        families: Families::ALL,
        model_limits: &[],
        section: Some(Section::Folders),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
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
        scope: Scope::Host,
        kind: ValueKind::Path(PathKind::Folder),
        families: Families::ALL,
        model_limits: &[],
        section: Some(Section::Folders),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
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
        scope: Scope::Host,
        kind: ValueKind::Path(PathKind::File),
        families: Families::ALL,
        model_limits: &[],
        section: Some(Section::Folders),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
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
        scope: Scope::Host,
        kind: ValueKind::Path(PathKind::Folder),
        families: Families::AT,
        model_limits: &[],
        section: Some(Section::Roms),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
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
        scope: Scope::Host,
        kind: ValueKind::Path(PathKind::Folder),
        families: Families::FM7,
        model_limits: &[],
        section: Some(Section::Roms),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
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
        scope: Scope::Host,
        kind: ValueKind::Path(PathKind::Folder),
        families: Families::MSX,
        model_limits: &[],
        section: Some(Section::Roms),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
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
        scope: Scope::Host,
        kind: ValueKind::Path(PathKind::Folder),
        families: MIDI_FAMILIES,
        model_limits: &[],
        section: Some(Section::Roms),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
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
        scope: Scope::Host,
        kind: ValueKind::Path(PathKind::Folder),
        families: Families::PC60,
        model_limits: &[],
        section: Some(Section::Roms),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
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
        scope: Scope::Host,
        kind: ValueKind::Path(PathKind::Folder),
        families: Families::PC88,
        model_limits: &[],
        section: Some(Section::Roms),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
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
        scope: Scope::Host,
        kind: ValueKind::Path(PathKind::Folder),
        families: Families::PC88VA,
        model_limits: &[],
        section: Some(Section::Roms),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
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
        scope: Scope::Host,
        kind: ValueKind::Path(PathKind::Folder),
        families: Families::PC98,
        model_limits: &[],
        section: Some(Section::Roms),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
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
        scope: Scope::Host,
        kind: ValueKind::Path(PathKind::Folder),
        families: MIDI_FAMILIES,
        model_limits: &[],
        section: Some(Section::Roms),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
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
        scope: Scope::Host,
        kind: ValueKind::Path(PathKind::Folder),
        families: Families::TOWNS,
        model_limits: &[],
        section: Some(Section::Roms),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
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
        scope: Scope::Host,
        kind: ValueKind::Path(PathKind::Folder),
        families: Families::X1,
        model_limits: &[],
        section: Some(Section::Roms),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
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
        scope: Scope::Host,
        kind: ValueKind::Path(PathKind::Folder),
        families: Families::X68K,
        model_limits: &[],
        section: Some(Section::Roms),
        explicit: None,
        file_types: &[],
        unset_values: &[],
        fact: ChoiceFact::None,
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
            ValueKind::Choice(_) => setting
                .choices_for_families(Families::compiled())
                .map(|choice| choice.value.to_owned())
                .collect(),
            ValueKind::OnOff => vec!["on".to_owned(), "off".to_owned()],
            ValueKind::Number(range) => [range.minimum, range.maximum]
                .iter()
                .map(|value| format_number(*value, range.integer))
                .collect(),
            ValueKind::Path(_) => vec!["images/disk.img".to_owned()],
            ValueKind::PathList => vec!["images/disk.img".to_owned()],
            ValueKind::Language => vec!["auto".to_owned(), "ja".to_owned(), "pt-BR".to_owned()],
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

    /// Returns whether `name` is a name of a language file section: lower case letters, digits
    /// and `-`.
    fn is_section_name(name: &str) -> bool {
        !name.is_empty()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    }

    /// Verifies that every key is unique, has families and gives the name of a language file
    /// section.
    #[test]
    fn keys_are_unique_section_names() {
        let mut keys = BTreeSet::new();
        for setting in SETTINGS {
            assert!(keys.insert(setting.key), "{} is listed twice", setting.key);
            assert!(is_section_name(setting.key), "{}", setting.key);
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
                ValueKind::Choice(_)
                | ValueKind::OnOff
                | ValueKind::Number(_)
                | ValueKind::Language => {}
            }
        }
    }

    /// Verifies that the GUI language takes auto and language tags, writes them back as written
    /// and rejects other text.
    #[test]
    fn the_gui_language_takes_auto_and_language_tags() {
        use super::super::GuiLanguage;

        let setting = find_setting(GUI_LANGUAGE_KEY).expect("gui-language is a table key");
        assert_eq!(setting.scope, Scope::Host);
        assert_eq!(setting.kind, ValueKind::Language);
        assert_eq!(setting.section, Some(Section::VideoAndAudio));
        let mut config = first_system_config();
        assert_eq!(config.gui_language, GuiLanguage::Auto);
        assert_eq!((setting.format)(&config), ["auto"]);
        for (value, language, written) in [
            ("ja", GuiLanguage::Tag("ja".to_owned()), "ja"),
            ("pt-BR", GuiLanguage::Tag("pt-BR".to_owned()), "pt-BR"),
            (
                "zh-Hant-TW",
                GuiLanguage::Tag("zh-Hant-TW".to_owned()),
                "zh-Hant-TW",
            ),
            ("AUTO", GuiLanguage::Auto, "auto"),
        ] {
            (setting.apply)(&mut config, value).expect("the value applies");
            assert_eq!(config.gui_language, language);
            assert_eq!((setting.format)(&config), [written]);
        }
        for invalid in ["", "pt_BR", "ja-", "-ja", "ja--JP", "日本語", "en US"] {
            assert_eq!(
                (setting.apply)(&mut config, invalid),
                Err(ValueError::Language {
                    value: invalid.to_owned()
                }),
                "{invalid}"
            );
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
                ValueKind::Language => "pt_BR",
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
            Err(ValueError::OnOff {
                value: "maybe".to_owned()
            })
        );
        let volume = find_setting("audio-volume").expect("audio-volume is a table key");
        assert_eq!(
            (volume.apply)(&mut config, "-0.1"),
            Err(ValueError::OutOfRange {
                value: "-0.1".to_owned(),
                range: AUDIO_VOLUME_RANGE
            })
        );
        assert!((volume.apply)(&mut config, "NaN").is_err());
        let scaling = find_setting("scaling").expect("scaling is a table key");
        assert_eq!(
            (scaling.apply)(&mut config, "sharp"),
            Err(ValueError::unknown_choice("sharp", SCALING_CHOICES))
        );
        let machine = find_setting(MACHINE_KEY).expect("machine is a table key");
        assert_eq!(
            (machine.apply)(&mut config, "X68030"),
            Err(ValueError::unknown_choice("X68030", MODEL_CHOICES))
        );
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

    /// Verifies that the choices of a set of families hold the choices of each family of the set.
    #[test]
    fn choices_of_families_hold_the_choices_of_each_family() {
        let boot_device = find_setting("boot-device").expect("boot-device is a table key");
        let compiled: Vec<&str> = boot_device
            .choices_for_families(Families::compiled())
            .map(|choice| choice.value)
            .collect();
        for system in super::super::systems::compiled_systems() {
            let target = MachineSelection::of_system(system)
                .expect("compiled system")
                .target();
            for choice in boot_device.choices_for(target) {
                assert!(compiled.contains(&choice.value), "{}", choice.value);
            }
        }
        assert_eq!(boot_device.choices_for_families(Families::NONE).count(), 0);
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

    /// Verifies that every phrase reason of a model limit is a key of a language file section.
    #[test]
    fn model_limits_give_a_reason_key() {
        for setting in SETTINGS {
            for limit in setting.model_limits {
                match limit.reason {
                    LimitReason::Phrase(key) => assert!(is_section_name(key), "{}", setting.key),
                    LimitReason::FixedClock => {}
                }
            }
        }
    }

    /// Verifies that every image setting of a game lists its files for each family that reads it.
    #[test]
    fn image_settings_list_their_files_for_every_family() {
        for setting in SETTINGS {
            let images = setting.scope == Scope::Game
                && matches!(
                    setting.kind,
                    ValueKind::PathList | ValueKind::Path(PathKind::File)
                );
            if !images {
                assert!(setting.file_types.is_empty(), "{}", setting.key);
                continue;
            }
            let mut covered = Families::NONE;
            for types in setting.file_types {
                assert!(
                    setting.families.intersects(types.families),
                    "{}",
                    setting.key
                );
                assert!(!covered.intersects(types.families), "{}", setting.key);
                covered = covered.with(types.families);
            }
            assert!(covered.includes(setting.families), "{}", setting.key);
        }
    }

    /// Verifies that the unset values belong to optional settings of their families.
    #[test]
    fn unset_values_belong_to_optional_settings() {
        for setting in SETTINGS {
            let mut covered = Families::NONE;
            for unset in setting.unset_values {
                assert!(setting.families.includes(unset.families), "{}", setting.key);
                assert!(!covered.intersects(unset.families), "{}", setting.key);
                covered = covered.with(unset.families);
            }
            if !setting.unset_values.is_empty() {
                let config = first_system_config();
                assert!((setting.format)(&config).is_empty(), "{}", setting.key);
            }
        }
    }

    #[cfg(feature = "pc88")]
    /// Verifies the unset boot mode of the PC-88 and that only the PC-88 derives defaults.
    #[test]
    fn the_pc88_derives_its_defaults_and_boots_v2_without_a_boot_mode() {
        let boot_mode = find_setting("boot-mode").expect("boot-mode is a table key");
        assert_eq!(
            boot_mode.unset_text(Target::Pc88),
            Some(UnsetText::Value("v2"))
        );
        let cpu_mode = find_setting("cpu-mode").expect("cpu-mode is a table key");
        assert!(cpu_mode.has_derived_default(Target::Pc88));
        assert!(!boot_mode.has_derived_default(Target::Pc88));
        let model = MachineSelection::default_of(Target::Pc88);
        let Some(FactData::CpuClock { mode, clock }) = cpu_mode.choice_fact(model, Some("low"))
        else {
            panic!("the CPU mode gives a clock");
        };
        assert_eq!((mode, clock.to_string().as_str()), (CpuMode::Low, "4 MHz"));
        assert_eq!(
            boot_mode.choice_fact(model, None),
            Some(FactData::Choices(vec![
                "v1s", "v1h", "v2", "n", "n80", "n80sr"
            ]))
        );
    }

    #[cfg(feature = "pc98")]
    /// Verifies the model limit and the reason of the PC-98 CD-ROM drive.
    #[test]
    fn the_pc98_cdrom_limit_gives_its_models_and_reason() {
        let cdrom = find_setting("cdrom").expect("cdrom is a table key");
        let limit = cdrom
            .limit_for(Target::Pc98)
            .expect("the PC-98 limits cdrom");
        assert_eq!(limit.models, ["PC9821AS", "PC9821AP"]);
        let model: MachineSelection = "PC9801RA".parse().expect("a PC-98 model");
        assert_eq!(limit.reason, LimitReason::Phrase("no-cdrom"));
        assert!(!cdrom.has_derived_default(Target::Pc98));
        let cpu_mode = find_setting("cpu-mode").expect("cpu-mode is a table key");
        let limit = cpu_mode
            .limit_for(Target::Pc98)
            .expect("the PC-98 limits cpu-mode");
        assert_eq!(
            limit
                .reason
                .fixed_clock(model)
                .map(|clock| clock.to_string()),
            Some("20 MHz".to_owned())
        );
        let vm: MachineSelection = "PC9801VM".parse().expect("a PC-98 model");
        let Some(FactData::CpuClock { mode, clock }) = cpu_mode.choice_fact(vm, Some("low")) else {
            panic!("the CPU mode gives a clock");
        };
        assert_eq!((mode, clock.to_string().as_str()), (CpuMode::Low, "8 MHz"));
        assert_eq!(cpu_mode.choice_fact(vm, None), None);
        let gdc = find_setting("force-gdc-clock").expect("force-gdc-clock is a table key");
        assert_eq!(gdc.unset_text(Target::Pc98), Some(UnsetText::Word("auto")));
    }
}
