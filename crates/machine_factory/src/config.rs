//! Pure-data emulator configuration types and the machine model resolver.

use std::path::PathBuf;

use common::{CpuMode, MachineModel, MonitorTiming};
#[cfg(feature = "pc88")]
use common::{EightMhzWaitMode, MemoryWaitSwitch};
use device::{gameport_towns::TownsPadType, subcontroller_x1::X1KeyboardMode};
#[cfg(feature = "pc60")]
use machine_60::Pc6000Model;
#[cfg(feature = "pc88")]
use machine_88::Pc8801Model;
#[cfg(feature = "pc88va")]
use machine_88va::Pc88VaModel;
#[cfg(feature = "at")]
use machine_at::AtModel;
#[cfg(feature = "fm7")]
use machine_fm7::Fm7Model;
#[cfg(feature = "msx")]
use machine_msx::MsxModel;
#[cfg(feature = "towns")]
use machine_towns::TownsModel;
#[cfg(feature = "x1")]
use machine_x1::X1Model;
#[cfg(feature = "x68k")]
use machine_x68k::X68kModel;

/// Selected machine family. Each family is constructed from its own crate, so
/// the family is tracked explicitly while the model and per-family settings
/// stay on `EmulatorConfig`. A family is present only when its cargo feature is
/// enabled.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Target {
    /// PC-9800 series (the `machine_98` crate).
    #[cfg(feature = "pc98")]
    Pc98,
    /// PC-8801 series (the `machine_88` crate).
    #[cfg(feature = "pc88")]
    Pc88,
    /// PC-88VA series (the `machine_88va` crate).
    #[cfg(feature = "pc88va")]
    Pc88Va,
    /// PC-6000/PC-6600 series (the `machine_60` crate).
    #[cfg(feature = "pc60")]
    Pc60,
    /// MSX series (the `machine_msx` crate).
    #[cfg(feature = "msx")]
    Msx,
    /// FM Towns series (the `machine_towns` crate).
    #[cfg(feature = "towns")]
    Towns,
    /// Sharp X1 series (the `machine_x1` crate).
    #[cfg(feature = "x1")]
    X1,
    /// Fujitsu FM-7 series (the `machine_fm7` crate).
    #[cfg(feature = "fm7")]
    Fm7,
    /// Sharp X68000 series (the `machine_x68k` crate).
    #[cfg(feature = "x68k")]
    X68k,
    /// IBM PC/AT series (the `machine_at` crate).
    #[cfg(feature = "at")]
    At,
}

/// Returns the default machine family: the first one compiled in, in the
/// declaration order of [`Target`].
#[must_use]
pub const fn default_target() -> Target {
    #[cfg(feature = "pc98")]
    return Target::Pc98;
    #[cfg(all(not(feature = "pc98"), feature = "pc88"))]
    return Target::Pc88;
    #[cfg(all(not(any(feature = "pc98", feature = "pc88")), feature = "pc88va"))]
    return Target::Pc88Va;
    #[cfg(all(
        not(any(feature = "pc98", feature = "pc88", feature = "pc88va")),
        feature = "pc60"
    ))]
    return Target::Pc60;
    #[cfg(all(
        not(any(
            feature = "pc98",
            feature = "pc88",
            feature = "pc88va",
            feature = "pc60"
        )),
        feature = "msx"
    ))]
    return Target::Msx;
    #[cfg(all(
        not(any(
            feature = "pc98",
            feature = "pc88",
            feature = "pc88va",
            feature = "pc60",
            feature = "msx"
        )),
        feature = "towns"
    ))]
    return Target::Towns;
    #[cfg(all(
        not(any(
            feature = "pc98",
            feature = "pc88",
            feature = "pc88va",
            feature = "pc60",
            feature = "msx",
            feature = "towns"
        )),
        feature = "x1"
    ))]
    return Target::X1;
    #[cfg(all(
        not(any(
            feature = "pc98",
            feature = "pc88",
            feature = "pc88va",
            feature = "pc60",
            feature = "msx",
            feature = "towns",
            feature = "x1"
        )),
        feature = "fm7"
    ))]
    return Target::Fm7;
    #[cfg(all(
        not(any(
            feature = "pc98",
            feature = "pc88",
            feature = "pc88va",
            feature = "pc60",
            feature = "msx",
            feature = "towns",
            feature = "x1",
            feature = "fm7"
        )),
        feature = "x68k"
    ))]
    return Target::X68k;
    #[cfg(all(
        not(any(
            feature = "pc98",
            feature = "pc88",
            feature = "pc88va",
            feature = "pc60",
            feature = "msx",
            feature = "towns",
            feature = "x1",
            feature = "fm7",
            feature = "x68k"
        )),
        feature = "at"
    ))]
    return Target::At;
}

impl Default for Target {
    fn default() -> Self {
        default_target()
    }
}

impl Target {
    /// Returns whether this family needs a render surface larger than the
    /// PC-98 sized default.
    #[must_use]
    pub fn wants_large_native_surface(self) -> bool {
        match self {
            #[cfg(feature = "pc98")]
            Target::Pc98 => false,
            #[cfg(feature = "pc88")]
            Target::Pc88 => false,
            #[cfg(feature = "pc88va")]
            Target::Pc88Va => false,
            #[cfg(feature = "pc60")]
            Target::Pc60 => false,
            #[cfg(feature = "msx")]
            Target::Msx => false,
            #[cfg(feature = "towns")]
            Target::Towns => true,
            #[cfg(feature = "x1")]
            Target::X1 => false,
            #[cfg(feature = "fm7")]
            Target::Fm7 => false,
            #[cfg(feature = "x68k")]
            Target::X68k => true,
            #[cfg(feature = "at")]
            Target::At => true,
        }
    }

    /// Returns whether this family's font ROM already uses the PC-98 CG-ROM
    /// layout, which the host image selector renders with.
    #[must_use]
    pub fn has_pc98_cgrom_layout(self) -> bool {
        match self {
            #[cfg(feature = "pc98")]
            Target::Pc98 => true,
            #[cfg(feature = "pc88")]
            Target::Pc88 => false,
            #[cfg(feature = "pc88va")]
            Target::Pc88Va => false,
            #[cfg(feature = "pc60")]
            Target::Pc60 => false,
            #[cfg(feature = "msx")]
            Target::Msx => false,
            #[cfg(feature = "towns")]
            Target::Towns => false,
            #[cfg(feature = "x1")]
            Target::X1 => false,
            #[cfg(feature = "fm7")]
            Target::Fm7 => false,
            #[cfg(feature = "x68k")]
            Target::X68k => false,
            #[cfg(feature = "at")]
            Target::At => false,
        }
    }

    /// Returns whether this family drives a composite monitor whose artifact
    /// colours the renderer has to emulate.
    #[must_use]
    pub fn has_composite_video(self) -> bool {
        match self {
            #[cfg(feature = "pc98")]
            Target::Pc98 => false,
            #[cfg(feature = "pc88")]
            Target::Pc88 => false,
            #[cfg(feature = "pc88va")]
            Target::Pc88Va => false,
            #[cfg(feature = "pc60")]
            Target::Pc60 => true,
            #[cfg(feature = "msx")]
            Target::Msx => false,
            #[cfg(feature = "towns")]
            Target::Towns => false,
            #[cfg(feature = "x1")]
            Target::X1 => false,
            #[cfg(feature = "fm7")]
            Target::Fm7 => false,
            #[cfg(feature = "x68k")]
            Target::X68k => false,
            #[cfg(feature = "at")]
            Target::At => false,
        }
    }
}

/// Boot mode requested on the command line, spanning every machine family that
/// exposes one. Each machine accepts only the subset it understands; the
/// conversion methods reject an out-of-subset value so a wrong `--boot-mode`
/// choice fails cleanly at machine initialization instead of being silently
/// ignored.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum BootMode {
    /// PC-88 N88-BASIC V1 standard speed.
    Pc88V1S,
    /// PC-88 N88-BASIC V1 high speed.
    Pc88V1H,
    /// PC-88 N88-BASIC V2.
    Pc88V2,
    /// PC-88 plain N-BASIC.
    Pc88N,
    /// PC-88 N80-BASIC (PC-8001mkII compatibility).
    Pc88N80,
    /// PC-88 N80SR-BASIC (PC-8001mkIISR compatibility).
    Pc88N80Sr,
    /// FM-7 F-BASIC from ROM.
    Fm7Basic,
    /// FM-7 disk (DOS) boot.
    Fm7Dos,
}

impl BootMode {
    /// Returns whether this boot mode makes the PC-88 start at its
    /// compatibility speed: V1 standard and the PC-8001-compatible N modes run
    /// at 4 MHz with the real-hardware memory waits.
    #[must_use]
    pub const fn wants_pc88_compatibility_speed(self) -> bool {
        matches!(
            self,
            BootMode::Pc88V1S | BootMode::Pc88N | BootMode::Pc88N80 | BootMode::Pc88N80Sr
        )
    }

    /// Maps this value to a PC-88 boot mode, erroring when the value belongs to
    /// another machine family.
    #[cfg(feature = "pc88")]
    pub fn to_pc88(self) -> Result<machine_88::BootMode, String> {
        match self {
            BootMode::Pc88V1S => Ok(machine_88::BootMode::V1S),
            BootMode::Pc88V1H => Ok(machine_88::BootMode::V1H),
            BootMode::Pc88V2 => Ok(machine_88::BootMode::V2),
            BootMode::Pc88N => Ok(machine_88::BootMode::N),
            BootMode::Pc88N80 => Ok(machine_88::BootMode::N80),
            BootMode::Pc88N80Sr => Ok(machine_88::BootMode::N80SR),
            BootMode::Fm7Basic | BootMode::Fm7Dos => Err(format!(
                "boot mode '{self}' is not supported by the PC-8801, expected v1s, v1h, v2, n, n80 or n80sr"
            )),
        }
    }

    /// Maps this value to an FM-7 boot mode, erroring when the value belongs to
    /// another machine family.
    #[cfg(feature = "fm7")]
    pub fn to_fm7(self) -> Result<machine_fm7::BootMode, String> {
        match self {
            BootMode::Fm7Basic => Ok(machine_fm7::BootMode::Basic),
            BootMode::Fm7Dos => Ok(machine_fm7::BootMode::Dos),
            BootMode::Pc88V1S
            | BootMode::Pc88V1H
            | BootMode::Pc88V2
            | BootMode::Pc88N
            | BootMode::Pc88N80
            | BootMode::Pc88N80Sr => Err(format!(
                "boot mode '{self}' is not supported by the FM-7, expected basic or dos"
            )),
        }
    }
}

impl std::fmt::Display for BootMode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            BootMode::Pc88V1S => "v1s",
            BootMode::Pc88V1H => "v1h",
            BootMode::Pc88V2 => "v2",
            BootMode::Pc88N => "n",
            BootMode::Pc88N80 => "n80",
            BootMode::Pc88N80Sr => "n80sr",
            BootMode::Fm7Basic => "basic",
            BootMode::Fm7Dos => "dos",
        };
        formatter.write_str(text)
    }
}

impl std::str::FromStr for BootMode {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text.to_ascii_lowercase().as_str() {
            "v1s" => Ok(BootMode::Pc88V1S),
            "v1h" => Ok(BootMode::Pc88V1H),
            "v2" => Ok(BootMode::Pc88V2),
            "n" => Ok(BootMode::Pc88N),
            "n80" | "n80v1" => Ok(BootMode::Pc88N80),
            "n80sr" | "n80v2" => Ok(BootMode::Pc88N80Sr),
            "basic" => Ok(BootMode::Fm7Basic),
            "dos" => Ok(BootMode::Fm7Dos),
            _ => Err(format!(
                "unknown boot mode '{text}', expected v1s, v1h, v2, n, n80, n80sr (PC-88) or basic, dos (FM-7)"
            )),
        }
    }
}

/// The complete startup configuration for building a machine.
#[derive(Clone)]
pub struct EmulatorConfig {
    pub target: Target,
    pub machine: MachineModel,
    pub cpu_mode: CpuMode,
    pub fdd1: Vec<PathBuf>,
    pub fdd2: Vec<PathBuf>,
    pub hdd1: Option<PathBuf>,
    pub hdd2: Option<PathBuf>,
    pub cdrom: Vec<PathBuf>,
    pub cdrom_compat: bool,
    pub aspect_mode: AspectMode,
    pub crt: bool,
    pub scaling: ScalingMode,
    pub window_mode: WindowMode,
    pub audio_volume: f32,
    pub pc98_roms: Option<PathBuf>,
    pub bios: bool,
    pub debug_bios: Option<PathBuf>,
    pub soundboard: SoundboardType,
    pub adpcm_ram: bool,
    pub force_gdc_clock: Option<ForceGdcClock>,
    pub graphicboard: GraphicboardType,
    pub printer: Option<PathBuf>,
    pub mt32_roms: Option<PathBuf>,
    pub sc55_roms: Option<PathBuf>,
    pub midi: MidiDevice,
    pub boot_device: common::BootDevice,
    pub ems: bool,
    pub xms: bool,
    pub backend: Backend,
    pub enable_extractor: bool,
    /// Boot mode selected via `--boot-mode`, shared across machine families.
    /// `None` means each machine uses its own default.
    pub boot_mode: Option<BootMode>,
    pub monitor: MonitorTiming,
    #[cfg(feature = "pc88")]
    pub pc88_memory_wait: MemoryWaitSwitch,
    #[cfg(feature = "pc88")]
    pub pc88_8mhz_wait: EightMhzWaitMode,
    pub pc88_roms: Option<PathBuf>,
    #[cfg(feature = "pc88va")]
    pub pc88va_model: Pc88VaModel,
    pub pc88va_roms: Option<PathBuf>,
    #[cfg(feature = "pc60")]
    pub pc60_model: Pc6000Model,
    pub pc6000_roms: Option<PathBuf>,
    #[cfg(feature = "msx")]
    pub msx_model: MsxModel,
    pub msx_roms: Option<PathBuf>,
    #[cfg(feature = "x1")]
    pub x1_model: X1Model,
    pub x1_roms: Option<PathBuf>,
    pub x1_keyboard: X1KeyboardMode,
    #[cfg(feature = "fm7")]
    pub fm7_model: Fm7Model,
    pub fm7_roms: Option<PathBuf>,
    #[cfg(feature = "towns")]
    pub towns_model: TownsModel,
    pub towns_roms: Option<PathBuf>,
    #[cfg(feature = "x68k")]
    pub x68k_model: X68kModel,
    pub x68k_roms: Option<PathBuf>,
    #[cfg(feature = "at")]
    pub at_model: AtModel,
    pub at_roms: Option<PathBuf>,
    pub towns_pad: TownsPadType,
    pub cartridge: Option<PathBuf>,
    pub cassette: Option<PathBuf>,
    /// Initial composite subcarrier phase select (0..3). Swaps the PC-6001
    /// artifact-color pair; also cycled at runtime with Right Ctrl + P.
    pub pc60_composite_phase: u32,
}

impl Default for EmulatorConfig {
    fn default() -> Self {
        Self {
            target: default_target(),
            machine: MachineModel::PC9801RA,
            cpu_mode: CpuMode::High,
            fdd1: Vec::new(),
            fdd2: Vec::new(),
            hdd1: None,
            hdd2: None,
            cdrom: Vec::new(),
            cdrom_compat: false,
            aspect_mode: AspectMode::Aspect4By3,
            crt: true,
            scaling: ScalingMode::Pixelart,
            window_mode: WindowMode::Windowed,
            audio_volume: 1.0,
            pc98_roms: None,
            bios: false,
            debug_bios: None,
            soundboard: SoundboardType::Sb86And26k,
            adpcm_ram: true,
            force_gdc_clock: None,
            graphicboard: GraphicboardType::None,
            printer: None,
            mt32_roms: None,
            sc55_roms: None,
            midi: MidiDevice::default(),
            boot_device: common::BootDevice::Auto,
            ems: true,
            xms: true,
            backend: Backend::Modern,
            enable_extractor: false,
            boot_mode: None,
            monitor: MonitorTiming::Auto,
            #[cfg(feature = "pc88")]
            pc88_memory_wait: MemoryWaitSwitch::Fast,
            #[cfg(feature = "pc88")]
            pc88_8mhz_wait: EightMhzWaitMode::Fast,
            pc88_roms: None,
            #[cfg(feature = "pc88va")]
            pc88va_model: Pc88VaModel::PC88VA2,
            pc88va_roms: None,
            #[cfg(feature = "pc60")]
            pc60_model: Pc6000Model::Pc6001,
            pc6000_roms: None,
            #[cfg(feature = "msx")]
            msx_model: MsxModel::Msx,
            msx_roms: None,
            #[cfg(feature = "x1")]
            x1_model: X1Model::X1,
            x1_roms: None,
            x1_keyboard: X1KeyboardMode::ModeA,
            #[cfg(feature = "fm7")]
            fm7_model: Fm7Model::Fm7,
            fm7_roms: None,
            #[cfg(feature = "towns")]
            towns_model: TownsModel::FmTownsIIMx,
            towns_roms: None,
            #[cfg(feature = "x68k")]
            x68k_model: X68kModel::X68000,
            x68k_roms: None,
            #[cfg(feature = "at")]
            at_model: AtModel::default(),
            at_roms: None,
            towns_pad: TownsPadType::SixButton,
            cartridge: None,
            cassette: None,
            pc60_composite_phase: 0,
        }
    }
}

/// Machine model names accepted by the families compiled into this build.
const COMPILED_MODEL_NAMES: &[&str] = &[
    #[cfg(feature = "pc98")]
    "PC9801F",
    #[cfg(feature = "pc98")]
    "PC9801VM",
    #[cfg(feature = "pc98")]
    "PC9801VX",
    #[cfg(feature = "pc98")]
    "PC9801RS",
    #[cfg(feature = "pc98")]
    "PC9801RA",
    #[cfg(feature = "pc98")]
    "PC9821AS",
    #[cfg(feature = "pc98")]
    "PC9821AP",
    #[cfg(feature = "pc88")]
    "PC8801MC",
    #[cfg(feature = "pc88va")]
    "PC88VA2",
    #[cfg(feature = "pc60")]
    "PC6001",
    #[cfg(feature = "pc60")]
    "PC6001MK2",
    #[cfg(feature = "pc60")]
    "PC6601",
    #[cfg(feature = "pc60")]
    "PC6001MK2SR",
    #[cfg(feature = "pc60")]
    "PC6601SR",
    #[cfg(feature = "msx")]
    "MSX",
    #[cfg(feature = "msx")]
    "MSX2",
    #[cfg(feature = "msx")]
    "MSX2PLUS",
    #[cfg(feature = "towns")]
    "FMTowns",
    #[cfg(feature = "towns")]
    "FMTownsIICX",
    #[cfg(feature = "towns")]
    "FMTownsIIMX",
    #[cfg(feature = "x68k")]
    "X68000",
    #[cfg(feature = "x68k")]
    "X68000SUPER",
    #[cfg(feature = "x68k")]
    "X68000XVI",
    #[cfg(feature = "x1")]
    "X1",
    #[cfg(feature = "x1")]
    "X1TURBO",
    #[cfg(feature = "fm7")]
    "FM7",
    #[cfg(feature = "fm7")]
    "FM77AV",
    #[cfg(feature = "at")]
    "AT486DX50",
    #[cfg(feature = "at")]
    "AT486DX66",
];

/// Family symbol names for the families compiled into this build.
const COMPILED_TARGET_NAMES: &[&str] = &[
    #[cfg(feature = "pc98")]
    "pc98",
    #[cfg(feature = "pc88")]
    "pc88",
    #[cfg(feature = "pc88va")]
    "pc88va",
    #[cfg(feature = "pc60")]
    "pc60",
    #[cfg(feature = "msx")]
    "msx",
    #[cfg(feature = "towns")]
    "towns",
    #[cfg(feature = "x1")]
    "x1",
    #[cfg(feature = "fm7")]
    "fm7",
    #[cfg(feature = "x68k")]
    "x68k",
    #[cfg(feature = "at")]
    "at",
];

/// Renders a name list as `a, b or c` for an error message.
fn join_alternatives(names: &[&str]) -> String {
    match names {
        [] => "(none)".to_owned(),
        [only] => (*only).to_owned(),
        [rest @ .., last] => format!("{} or {last}", rest.join(", ")),
    }
}

/// Returns the machine model names this build accepts.
#[must_use]
pub fn compiled_model_names() -> &'static [&'static str] {
    COMPILED_MODEL_NAMES
}

/// Returns the family symbol names this build accepts.
#[must_use]
pub fn compiled_target_names() -> &'static [&'static str] {
    COMPILED_TARGET_NAMES
}

/// Resolves a `--machine` / `machine=` value to a family and model. Each
/// compiled-in family is offered the value in turn; the PC-98 models are tried
/// last. Returns a human-readable error if no compiled-in family recognises it.
pub fn resolve_model(config: &mut EmulatorConfig, value: &str) -> Result<(), String> {
    #[cfg(feature = "pc88")]
    if value.parse::<Pc8801Model>().is_ok() {
        config.target = Target::Pc88;
        return Ok(());
    }
    #[cfg(feature = "pc88va")]
    if let Ok(model) = value.parse::<Pc88VaModel>() {
        config.target = Target::Pc88Va;
        config.pc88va_model = model;
        return Ok(());
    }
    #[cfg(feature = "pc60")]
    if let Ok(model) = value.parse::<Pc6000Model>() {
        config.target = Target::Pc60;
        config.pc60_model = model;
        return Ok(());
    }
    #[cfg(feature = "msx")]
    if let Ok(model) = value.parse::<MsxModel>() {
        config.target = Target::Msx;
        config.msx_model = model;
        return Ok(());
    }
    #[cfg(feature = "x1")]
    if let Ok(model) = value.parse::<X1Model>() {
        config.target = Target::X1;
        config.x1_model = model;
        return Ok(());
    }
    #[cfg(feature = "fm7")]
    if let Ok(model) = value.parse::<Fm7Model>() {
        config.target = Target::Fm7;
        config.fm7_model = model;
        return Ok(());
    }
    #[cfg(feature = "towns")]
    if let Ok(model) = value.parse::<TownsModel>() {
        config.target = Target::Towns;
        config.towns_model = model;
        return Ok(());
    }
    #[cfg(feature = "x68k")]
    if let Ok(model) = value.parse::<X68kModel>() {
        config.target = Target::X68k;
        config.x68k_model = model;
        return Ok(());
    }
    #[cfg(feature = "at")]
    if let Ok(model) = value.parse::<AtModel>() {
        config.target = Target::At;
        config.at_model = model;
        return Ok(());
    }
    #[cfg(feature = "pc98")]
    if let Ok(model) = value.parse::<MachineModel>() {
        config.target = Target::Pc98;
        config.machine = model;
        return Ok(());
    }
    Err(format!(
        "unknown machine type '{value}', expected {}",
        join_alternatives(COMPILED_MODEL_NAMES)
    ))
}

/// Returns the stable lowercase symbol name for a target family.
#[must_use]
pub fn target_symbol_name(target: Target) -> &'static str {
    match target {
        #[cfg(feature = "pc98")]
        Target::Pc98 => "pc98",
        #[cfg(feature = "pc88")]
        Target::Pc88 => "pc88",
        #[cfg(feature = "pc88va")]
        Target::Pc88Va => "pc88va",
        #[cfg(feature = "pc60")]
        Target::Pc60 => "pc60",
        #[cfg(feature = "msx")]
        Target::Msx => "msx",
        #[cfg(feature = "towns")]
        Target::Towns => "towns",
        #[cfg(feature = "x1")]
        Target::X1 => "x1",
        #[cfg(feature = "fm7")]
        Target::Fm7 => "fm7",
        #[cfg(feature = "x68k")]
        Target::X68k => "x68k",
        #[cfg(feature = "at")]
        Target::At => "at",
    }
}

/// Resolves a target family symbol name to a `Target`.
pub fn target_from_name(name: &str) -> Result<Target, String> {
    match name {
        #[cfg(feature = "pc98")]
        "pc98" => Ok(Target::Pc98),
        #[cfg(feature = "pc88")]
        "pc88" => Ok(Target::Pc88),
        #[cfg(feature = "pc88va")]
        "pc88va" => Ok(Target::Pc88Va),
        #[cfg(feature = "pc60")]
        "pc60" => Ok(Target::Pc60),
        #[cfg(feature = "msx")]
        "msx" => Ok(Target::Msx),
        #[cfg(feature = "towns")]
        "towns" => Ok(Target::Towns),
        #[cfg(feature = "x1")]
        "x1" => Ok(Target::X1),
        #[cfg(feature = "fm7")]
        "fm7" => Ok(Target::Fm7),
        #[cfg(feature = "x68k")]
        "x68k" => Ok(Target::X68k),
        #[cfg(feature = "at")]
        "at" => Ok(Target::At),
        _ => Err(format!(
            "unknown target '{name}', expected {}",
            join_alternatives(COMPILED_TARGET_NAMES)
        )),
    }
}

/// Applies one scalar specification setting, given as a key and a value string.
///
/// Handles the non-media automation machine specification keys. The caller resolves `target`
/// and `model` separately because a model selects the family. An unknown key or
/// an invalid value is returned as a human-readable error.
pub fn apply_spec_setting(
    config: &mut EmulatorConfig,
    key: &str,
    value: &str,
) -> Result<(), String> {
    match key {
        "cpu-mode" => config.cpu_mode = value.parse()?,
        "sound-board" => config.soundboard = value.parse()?,
        "bios" => {
            config.bios = match value {
                "on" => true,
                "off" => false,
                _ => {
                    return Err(format!(
                        "unknown BIOS setting '{value}', expected on or off"
                    ));
                }
            };
        }
        "boot-device" => config.boot_device = value.parse()?,
        "boot-mode" => config.boot_mode = Some(value.parse()?),
        "graphic-board" => config.graphicboard = value.parse()?,
        "midi" => config.midi = value.parse()?,
        _ => return Err(format!("unknown specification key '{key}'")),
    }
    Ok(())
}

/// Forced GDC clock speed.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ForceGdcClock {
    /// Force 2.5 MHz (200-line compatibility mode).
    Force2_5,
    /// Force 5 MHz (400-line graphics mode).
    Force5,
}

impl std::fmt::Display for ForceGdcClock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Force2_5 => f.write_str("2.5"),
            Self::Force5 => f.write_str("5"),
        }
    }
}

impl std::str::FromStr for ForceGdcClock {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "2.5" => Ok(Self::Force2_5),
            "5" => Ok(Self::Force5),
            _ => Err(format!("unknown GDC clock mode '{s}', expected 2.5 or 5")),
        }
    }
}

/// Display aspect mode.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum AspectMode {
    Aspect4By3,
    Aspect1By1,
}

impl std::fmt::Display for AspectMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Aspect4By3 => f.write_str("4:3"),
            Self::Aspect1By1 => f.write_str("1:1"),
        }
    }
}

impl std::str::FromStr for AspectMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "4:3" => Ok(Self::Aspect4By3),
            "1:1" => Ok(Self::Aspect1By1),
            _ => Err(format!("unknown aspect mode '{s}', expected 4:3 or 1:1")),
        }
    }
}

/// Sound board type.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum SoundboardType {
    /// No sound board installed.
    None,
    /// PC-9801-14 Music Generator (TMS3631 8-channel synth).
    Sb14,
    /// PC-9801-26K only (YM2203 OPN).
    Sb26k,
    /// PC-9801-86 only (YM2608 OPNA + PCM86).
    Sb86,
    /// PC-9801-86 + PC-9801-26K (both boards).
    Sb86And26k,
    /// Creative Sound Blaster 16 (CT2720).
    Sb16,
    /// Creative Sound Blaster 16 (CT2720) + .
    Sb16And26k,
}

impl std::fmt::Display for SoundboardType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => f.write_str("none"),
            Self::Sb14 => f.write_str("14"),
            Self::Sb26k => f.write_str("26k"),
            Self::Sb86 => f.write_str("86"),
            Self::Sb86And26k => f.write_str("86+26k"),
            Self::Sb16 => f.write_str("sb16"),
            Self::Sb16And26k => f.write_str("sb16+26k"),
        }
    }
}

impl std::str::FromStr for SoundboardType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "none" => Ok(Self::None),
            "14" => Ok(Self::Sb14),
            "26k" => Ok(Self::Sb26k),
            "86" => Ok(Self::Sb86),
            "86+26k" => Ok(Self::Sb86And26k),
            "sb16" => Ok(Self::Sb16),
            "sb16+26k" => Ok(Self::Sb16And26k),
            _ => Err(format!(
                "unknown soundboard type '{s}', expected none, 14, 26k, 86, 86+26k, sb16 or sb16+26k"
            )),
        }
    }
}

/// Graphics accelerator board type.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum GraphicboardType {
    /// No graphics accelerator board installed.
    None,
    /// I-O DATA GA-1280A
    Ga1280a,
}

impl std::fmt::Display for GraphicboardType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => f.write_str("none"),
            Self::Ga1280a => f.write_str("ga1280a"),
        }
    }
}

impl std::str::FromStr for GraphicboardType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "none" => Ok(Self::None),
            "ga1280a" => Ok(Self::Ga1280a),
            _ => Err(format!(
                "unknown graphicboard type '{s}', expected none or ga1280a"
            )),
        }
    }
}

/// Scaling method used.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ScalingMode {
    Nearest,
    Bilinear,
    Pixelart,
}

impl std::fmt::Display for ScalingMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Nearest => f.write_str("nearest"),
            Self::Bilinear => f.write_str("bilinear"),
            Self::Pixelart => f.write_str("pixelart"),
        }
    }
}

impl std::str::FromStr for ScalingMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "nearest" => Ok(Self::Nearest),
            "bilinear" => Ok(Self::Bilinear),
            "pixelart" => Ok(Self::Pixelart),
            _ => Err(format!(
                "unknown scaling '{s}', expected nearest, bilinear or pixelart"
            )),
        }
    }
}

/// Window mode selection.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum WindowMode {
    Windowed,
    Fullscreen,
}

impl std::fmt::Display for WindowMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Windowed => f.write_str("windowed"),
            Self::Fullscreen => f.write_str("fullscreen"),
        }
    }
}

impl std::str::FromStr for WindowMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "windowed" => Ok(Self::Windowed),
            "fullscreen" => Ok(Self::Fullscreen),
            _ => Err(format!(
                "unknown window mode '{s}', expected windowed or fullscreen"
            )),
        }
    }
}

/// Rendering backend selection.
#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
pub enum Backend {
    /// SDL3 GPU API renderer (Vulkan / D3D12 / Metal under the hood).
    #[default]
    Modern,
    /// SDL3 2D renderer fallback. Used automatically when the GPU API is
    /// unavailable or fails to initialize.
    Legacy,
}

impl std::fmt::Display for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Modern => f.write_str("modern"),
            Self::Legacy => f.write_str("legacy"),
        }
    }
}

impl std::str::FromStr for Backend {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "modern" => Ok(Self::Modern),
            "legacy" => Ok(Self::Legacy),
            _ => Err(format!("unknown backend '{s}', expected modern or legacy")),
        }
    }
}

/// MIDI output device.
#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
pub enum MidiDevice {
    /// No MIDI output.
    #[default]
    None,
    /// Roland MT-32 (requires MT-32 ROMs).
    Mt32,
    /// Roland SC-55 (requires SC-55 ROMs).
    Sc55,
}

impl std::fmt::Display for MidiDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => f.write_str("none"),
            Self::Mt32 => f.write_str("mt32"),
            Self::Sc55 => f.write_str("sc55"),
        }
    }
}

impl std::str::FromStr for MidiDevice {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "none" => Ok(Self::None),
            "mt32" => Ok(Self::Mt32),
            "sc55" => Ok(Self::Sc55),
            _ => Err(format!(
                "unknown MIDI device '{s}', expected none, mt32 or sc55"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{EmulatorConfig, apply_spec_setting};

    /// Verifies that the automation BIOS setting accepts its public symbols.
    #[test]
    fn applies_bios_setting() {
        let mut config = EmulatorConfig::default();

        apply_spec_setting(&mut config, "bios", "on").expect("enable BIOS");
        assert!(config.bios);

        apply_spec_setting(&mut config, "bios", "off").expect("disable BIOS");
        assert!(!config.bios);
    }

    /// Verifies that the automation BIOS setting rejects unknown symbols.
    #[test]
    fn rejects_unknown_bios_setting() {
        let mut config = EmulatorConfig::default();
        let error = apply_spec_setting(&mut config, "bios", "maybe").unwrap_err();

        assert_eq!(error, "unknown BIOS setting 'maybe', expected on or off");
    }
}
