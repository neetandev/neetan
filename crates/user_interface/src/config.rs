use std::path::PathBuf;

use common::{StringError, bail};
pub use machine_factory::config::{
    AspectMode, Backend, BootMode, EmulatorConfig, ForceGdcClock, GraphicboardType,
    MachineSelection, MidiDevice, ScalingMode, SoundboardType, Target, WindowMode,
};
use machine_factory::config::{
    settings::{
        Families, MACHINE_KEY, PathKind, SETTINGS, Scope, Setting, ValueKind, find_setting,
    },
    systems::{compiled_systems, rom_folders},
};

pub use self::{
    discovery::{
        Locations, SETTINGS_FILE_NAME, SettingsLocation, discover_settings, resolve_path,
        resolve_value,
    },
    document::{ConfigDocument, DocumentEntry, DocumentError, LineContent, parse_line},
    layers::{CONFIG_FILE_EXTENSION, HostLayers, LaunchRequest, ModelSource, build_configuration},
    metadata::GameMetadata,
    reading::{Diagnostic, Severity, check_value},
};
use crate::input::KeyOverrides;

mod atomic_file;
mod discovery;
pub mod document;
pub(crate) mod keymap;
pub(crate) mod layers;
pub mod metadata;
pub(crate) mod reading;
#[cfg(test)]
pub(crate) mod test_support;

fn next_value(flag: &str, args: &mut impl Iterator<Item = String>) -> crate::Result<String> {
    match args.next() {
        Some(val) => Ok(val),
        None => bail!("missing value for {flag}"),
    }
}

/// Returns the value of a flag: the inline `--flag=value` text or the next argument.
fn flag_value(
    flag: &str,
    inline_value: Option<String>,
    args: &mut impl Iterator<Item = String>,
) -> crate::Result<String> {
    match inline_value {
        Some(value) => Ok(value),
        None => next_value(flag, args),
    }
}

/// Returns the value of an on/off flag. A flag without `=value` takes the next
/// argument when it is `on` or `off` and turns the setting on otherwise.
fn on_off_flag_value<I: Iterator<Item = String>>(
    inline_value: Option<String>,
    args: &mut std::iter::Peekable<I>,
) -> String {
    if let Some(value) = inline_value {
        return value;
    }
    args.next_if(|next| next == "on" || next == "off")
        .unwrap_or_else(|| "on".to_owned())
}

/// Returns the value placeholder of a setting in the `--help` list.
fn value_placeholder(setting: &Setting) -> &'static str {
    match setting.kind {
        ValueKind::Choice(_) => "<VALUE>",
        ValueKind::OnOff => "[on|off]",
        ValueKind::Number(_) => "<NUMBER>",
        ValueKind::Path(PathKind::File) | ValueKind::PathList => "<PATH>",
        ValueKind::Path(PathKind::Folder) => "<FOLDER>",
    }
}

/// Returns the built-in configuration of the default model of each compiled system.
fn system_defaults() -> Vec<(&'static str, EmulatorConfig)> {
    compiled_systems()
        .filter_map(|system| {
            MachineSelection::of_system(system)
                .map(|model| (system.name, EmulatorConfig::for_model(model)))
        })
        .collect()
}

/// Returns the built-in default of a setting in the `--help` list.
///
/// A default that differs between the systems that read the setting lists
/// the value of each of them. A setting that no compiled system reads shows
/// the default of every compiled system.
fn default_description(setting: &Setting, defaults: &[(&'static str, EmulatorConfig)]) -> String {
    let reading: Vec<&(&'static str, EmulatorConfig)> = defaults
        .iter()
        .filter(|(_, config)| setting.families.contains_target(config.target))
        .collect();
    let shown: Vec<&(&'static str, EmulatorConfig)> = if reading.is_empty() {
        defaults.iter().collect()
    } else {
        reading
    };
    let values: Vec<(&str, String)> = shown
        .iter()
        .map(|(name, config)| (*name, (setting.format)(config).join(", ")))
        .collect();
    let Some((_, first)) = values.first() else {
        return String::new();
    };
    if values.iter().all(|(_, value)| value == first) {
        if !first.is_empty() {
            return format!(" Default: {first}.");
        }
        return match rom_folders().find(|folder| folder.key == setting.key) {
            Some(folder) => format!(" Default: <roms-folder>/{}.", folder.name),
            None => String::new(),
        };
    }
    let listed: Vec<String> = values
        .iter()
        .map(|(name, value)| format!("{value} ({name})"))
        .collect();
    format!(" Default: {}.", listed.join(", "))
}

/// Returns the description of a setting in the `--help` list: the help text,
/// the values, the families that read it and the built-in default.
fn setting_description(setting: &Setting, defaults: &[(&'static str, EmulatorConfig)]) -> String {
    let mut description = setting.help.to_owned();
    match setting.kind {
        ValueKind::Choice(choices) => {
            let compiled = Families::compiled();
            let values: Vec<&str> = choices
                .iter()
                .filter(|choice| choice.families.intersects(compiled))
                .map(|choice| choice.value)
                .collect();
            description.push_str(&format!(" Values: {}.", values.join(", ")));
        }
        ValueKind::Number(range) => {
            description.push_str(&format!(
                " Range {} to {}.",
                format_range_end(range.minimum, range.integer),
                format_range_end(range.maximum, range.integer)
            ));
        }
        ValueKind::OnOff | ValueKind::Path(_) | ValueKind::PathList => {}
    }
    if setting.families != Families::ALL {
        let families: Vec<&str> = setting.families.names().collect();
        description.push_str(&format!(" Families: {}.", families.join(", ")));
    }
    description.push_str(&default_description(setting, defaults));
    description
}

/// Formats one end of a number range: a whole number, or a decimal number with one fractional digit.
fn format_range_end(value: f64, integer: bool) -> String {
    if integer {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}

/// Builds the option list of `--help` from the settings table and the flags outside it.
fn option_lines() -> String {
    let defaults = system_defaults();
    let mut rows: Vec<(String, String)> = vec![
        (
            "-c, --config <PATH>".to_owned(),
            "Game file to load. Repeat the option for more files.".to_owned(),
        ),
        (
            "    --settings <PATH>".to_owned(),
            "Settings file to load. Its folder is the root.".to_owned(),
        ),
    ];
    for setting in SETTINGS {
        rows.push((
            format!("    --{} {}", setting.key, value_placeholder(setting)),
            setting_description(setting, &defaults),
        ));
    }
    rows.push(("-h, --help".to_owned(), "Print help".to_owned()));
    rows.push(("-V, --version".to_owned(), "Print version".to_owned()));
    let width = rows.iter().map(|(flag, _)| flag.len()).max().unwrap_or(0);
    rows.iter()
        .map(|(flag, description)| format!("  {flag:<width$}  {description}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn print_help() {
    println!(
        "\
{} - Japanese computer emulator

Usage: neetan [OPTIONS]
       neetan <COMMAND>

Commands:
  create-fdd <PATH>             Create an empty floppy disk image (D88 or raw XDF)
  create-hdd <PATH>             Create an empty hard disk image (HDI, raw SCSI, X68000 HDF or PC/AT flat .hdd)
  convert-hdd <INPUT> <OUTPUT>  Convert HDD image between SASI and IDE
  copy <SOURCE> <DEST>          Copy files between host and FAT disk images

Options:
{}

An option takes its value as --name value or --name=value.
An on/off option without a value turns the setting on.
Every option except --config, --settings, --help and --version has a configuration file key of the
same name.

Startup:
  With --machine or --config, neetan starts the game directly. Without either option, it opens
  the game manager. A media option such as --fdd1 needs --machine or --config.

Settings file:
  Neetan reads the first neetan.conf of: --settings <PATH>, the current directory, the folder of
  the executable and the SDL pref path. The folder of the settings file is the root. Without a
  settings file the root is the SDL pref path.
  A relative path in a configuration file resolves against the root. A relative path in an option
  resolves against the current directory.

Layers:
  A later layer overrides an earlier one: the built-in defaults, the settings file, the system
  defaults <config-folder>/<system>.conf, the --config files and the options.
  Host keys apply only from the settings file and the options.

Run 'neetan <COMMAND> --help' for more information on a command.",
        crate::GAME_NAME,
        option_lines(),
    );
}

fn print_create_fdd_help() {
    println!(
        "\
Create an empty floppy disk image in D88 or raw XDF format

Usage: neetan create-fdd <PATH> [OPTIONS]

Arguments:
  <PATH>  Output file path (.d88 for D88; .xdf or .2hd for raw XDF, 2hd type only)

Options:
      --type <TYPE>  Floppy type [default: 2hd]
  -h, --help         Print help

Floppy types:
  2hd     1232 KB  (77 cyl, 2 heads, 8 spt, 1024 B/sector)
  2hd144  1440 KB  (80 cyl, 2 heads, 18 spt, 512 B/sector)
  2dd      640 KB  (80 cyl, 2 heads, 16 spt, 256 B/sector)
  2d       320 KB  (40 cyl, 2 heads, 16 spt, 256 B/sector)"
    );
}

fn print_create_hdd_help() {
    println!(
        "\
Create an empty hard disk image (HDI for SASI/IDE, raw for SCSI, HDF for X68000,
HDD for the PC/AT)

Usage: neetan create-hdd <PATH> [OPTIONS]

Arguments:
  <PATH>  Output file path (.hdi for SASI/IDE, .h0-.h4 for SCSI, .hdf for
          X68000, .hdd for the PC/AT)

Options:
      --type <TYPE>  HDD size (required)
  -h, --help         Print help

SASI types:
  sasi5      5 MB  (153 cyl, 4 heads, 33 spt, 256 B/sector)
  sasi10    10 MB  (310 cyl, 4 heads, 33 spt, 256 B/sector)
  sasi15    15 MB  (310 cyl, 6 heads, 33 spt, 256 B/sector)
  sasi20    20 MB  (310 cyl, 8 heads, 33 spt, 256 B/sector)
  sasi30    30 MB  (615 cyl, 6 heads, 33 spt, 256 B/sector)
  sasi40    40 MB  (615 cyl, 8 heads, 33 spt, 256 B/sector)

IDE types:
  ide40     40 MB  (977 cyl, 5 heads, 17 spt, 512 B/sector)
  ide80     80 MB  (977 cyl, 10 heads, 17 spt, 512 B/sector)
  ide120   120 MB  (977 cyl, 15 heads, 17 spt, 512 B/sector)
  ide200   200 MB  (977 cyl, 15 heads, 28 spt, 512 B/sector)
  ide500   500 MB  (1015 cyl, 16 heads, 63 spt, 512 B/sector)

SCSI types (raw .h0-.h4 images):
  scsi20     20 MB  (160 cyl, 8 heads, 32 spt, 512 B/sector)
  scsi40     40 MB  (320 cyl, 8 heads, 32 spt, 512 B/sector)
  scsi100   100 MB  (800 cyl, 8 heads, 32 spt, 512 B/sector)
  scsi200   200 MB  (1600 cyl, 8 heads, 32 spt, 512 B/sector)
  scsi340   340 MB  (2720 cyl, 8 heads, 32 spt, 512 B/sector)
  scsi540   540 MB  (4320 cyl, 8 heads, 32 spt, 512 B/sector)

X68000 SASI types (headerless .hdf images):
  x68sasi10  10 MB  (309 cyl, 4 heads, 33 spt, 256 B/sector)
  x68sasi20  20 MB  (614 cyl, 4 heads, 33 spt, 256 B/sector)
  x68sasi40  40 MB  (614 cyl, 8 heads, 33 spt, 256 B/sector)

X68000 SCSI types (headerless .hdf images):
  x68scsi20  20 MB  (160 cyl, 8 heads, 32 spt, 512 B/sector)
  x68scsi40  40 MB  (320 cyl, 8 heads, 32 spt, 512 B/sector)

PC/AT IDE types (headerless flat .hdd images):
  at40       40 MB  (81 cyl, 16 heads, 63 spt, 512 B/sector)
  at100     100 MB  (203 cyl, 16 heads, 63 spt, 512 B/sector)
  at250     250 MB  (507 cyl, 16 heads, 63 spt, 512 B/sector)
  at504     504 MB  (1023 cyl, 16 heads, 63 spt, 512 B/sector)"
    );
}

fn print_copy_help() {
    println!(
        "\
Copy files and directories between the host filesystem and FAT-formatted
disk images.

Usage: neetan copy <SOURCE> <DEST>

Arguments:
  <SOURCE>  Source path. Either a host path, or IMAGE:DOSPATH
            (e.g. roms/dos620.hdi:A:\\PROGS\\FILE.EXE).
  <DEST>    Destination path with the same syntax.

Options:
  -h, --help  Print help

Examples:
  neetan copy ./readme.txt roms/disk.hdi:A:\\README.TXT
  neetan copy roms/disk.hdi:A:\\PROGS\\FOO.EXE ./extracted/
  neetan copy ./mydir roms/disk.hdi:A:\\BACKUP
  neetan copy roms/disk.hdi:A:\\DOCS ./local_docs
  neetan copy src.hdi:A:\\FOO.EXE dst.hdi:A:\\FOO.EXE

Image formats: HDI, NHD, THD, HDD (HDD);
               D88, D98, 88D, 98D, HDM, NFD, 2D, IMG, IMA, DSK (FDD).

Notes:
  - Directories are copied recursively (no -r flag).
  - DOS paths must use 8.3 ASCII filenames; longer names are rejected
    before any file is written.
  - The destination image file is rewritten atomically on success."
    );
}

fn print_convert_hdd_help() {
    println!(
        "\
Convert a hard disk image between SASI and IDE formats

Usage: neetan convert-hdd <INPUT> <OUTPUT>

Arguments:
  <INPUT>   Source HDD image (HDI, NHD, or THD)
  <OUTPUT>  Destination path (must have .hdi extension)

Options:
  -h, --help  Print help

The conversion direction is detected from the input image:
  256 B/sector (SASI) -> converts to IDE
  512 B/sector (IDE)  -> converts to SASI

The smallest compatible target geometry is chosen automatically.

SASI geometries:
  sasi5      5 MB  (153 cyl, 4 heads, 33 spt, 256 B/sector)
  sasi10    10 MB  (310 cyl, 4 heads, 33 spt, 256 B/sector)
  sasi15    15 MB  (310 cyl, 6 heads, 33 spt, 256 B/sector)
  sasi20    20 MB  (310 cyl, 8 heads, 33 spt, 256 B/sector)
  sasi30    30 MB  (615 cyl, 6 heads, 33 spt, 256 B/sector)
  sasi40    40 MB  (615 cyl, 8 heads, 33 spt, 256 B/sector)

IDE geometries:
  ide40     40 MB  (977 cyl, 5 heads, 17 spt, 512 B/sector)
  ide80     80 MB  (977 cyl, 10 heads, 17 spt, 512 B/sector)
  ide120   120 MB  (977 cyl, 15 heads, 17 spt, 512 B/sector)
  ide200   200 MB  (977 cyl, 15 heads, 28 spt, 512 B/sector)
  ide500   500 MB  (1015 cyl, 16 heads, 63 spt, 512 B/sector)"
    );
}

fn print_version() {
    println!("neetan {}", crate::CARGO_PKG_VERSION);
}

/// The action of a command line.
pub enum Action {
    /// Starts the game of the configuration directly.
    Run(Box<EmulatorConfig>, KeyOverrides),
    /// Opens the game manager.
    Manager(Box<ManagerStart>),
    CreateFdd {
        path: PathBuf,
        fdd_type: FddType,
    },
    CreateHdd {
        path: PathBuf,
        hdd_type: HddSizeType,
    },
    ConvertHdd {
        input: PathBuf,
        output: PathBuf,
    },
    Copy {
        source: CopyArg,
        dest: CopyArg,
    },
}

/// The inputs of the game manager: the command line of the launch and the
/// folders of the settings discovery.
#[derive(Debug, Clone)]
pub struct ManagerStart {
    /// The command line inputs. Their paths are absolute.
    pub request: LaunchRequest,
    /// The folders that the settings discovery searches.
    pub locations: Locations,
}

/// The start of the application that a command line asks for.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum StartupMode {
    /// The game of the command line starts directly.
    Direct,
    /// The game manager opens.
    Manager,
}

/// Returns the start of the application for `request`.
///
/// A `--machine` flag or a `--config` file starts the game directly. A media
/// flag without both is an error. Every other command line opens the
/// manager. The `machine` of the settings file does not count.
pub fn startup_mode(request: &LaunchRequest) -> Result<StartupMode, String> {
    let has_machine = request
        .flags
        .iter()
        .any(|(setting, _)| setting.key == MACHINE_KEY);
    if has_machine || !request.game_files.is_empty() {
        return Ok(StartupMode::Direct);
    }
    if let Some((setting, _)) = request
        .flags
        .iter()
        .find(|(setting, _)| setting.scope == Scope::Game)
    {
        return Err(format!("--{} needs --machine or --config", setting.key));
    }
    Ok(StartupMode::Manager)
}

#[derive(Debug, Clone)]
pub enum CopyArg {
    Host(PathBuf),
    Image {
        image_path: PathBuf,
        dos_path: Vec<u8>,
    },
}

fn parse_copy_arg(raw: &str) -> CopyArg {
    for (idx, byte) in raw.as_bytes().iter().enumerate() {
        if *byte != b':' {
            continue;
        }
        let head = &raw[..idx];
        let Some(dot) = head.rfind('.') else {
            continue;
        };
        let ext = &head[dot + 1..];
        if crate::copy::is_disk_image_extension(ext) {
            return CopyArg::Image {
                image_path: PathBuf::from(head),
                dos_path: raw.as_bytes()[idx + 1..].to_vec(),
            };
        }
    }
    CopyArg::Host(PathBuf::from(raw))
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum FddType {
    Hd2,
    Hd2Fmt144,
    Dd2,
    D2,
}

impl std::str::FromStr for FddType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "2hd" => Ok(Self::Hd2),
            "2hd144" => Ok(Self::Hd2Fmt144),
            "2dd" => Ok(Self::Dd2),
            "2d" => Ok(Self::D2),
            _ => Err(format!(
                "unknown floppy type '{s}', expected 2hd, 2hd144, 2dd or 2d"
            )),
        }
    }
}

pub use device::disk::HddSizeType;

fn parse_create_fdd_args(args: &mut impl Iterator<Item = String>) -> crate::Result<Action> {
    let mut path: Option<PathBuf> = None;
    let mut fdd_type = FddType::Hd2;

    while let Some(arg) = args.next() {
        let (flag, inline_value) = match arg.split_once('=') {
            Some((f, v)) => (f.to_owned(), Some(v.to_owned())),
            None => (arg, None),
        };

        match flag.as_str() {
            "--help" | "-h" => {
                print_create_fdd_help();
                std::process::exit(0);
            }
            "--type" => {
                let val = if let Some(v) = inline_value {
                    v
                } else {
                    next_value("--type", args)?
                };
                fdd_type = val.parse::<FddType>().map_err(StringError)?;
            }
            other if !other.starts_with('-') && path.is_none() => {
                path = Some(PathBuf::from(other));
            }
            other => bail!("unknown argument: {other}"),
        }
    }

    let path = path.ok_or_else(|| StringError("missing required argument: <PATH>".into()))?;
    Ok(Action::CreateFdd { path, fdd_type })
}

fn parse_create_hdd_args(args: &mut impl Iterator<Item = String>) -> crate::Result<Action> {
    let mut path: Option<PathBuf> = None;
    let mut hdd_type: Option<HddSizeType> = None;

    while let Some(arg) = args.next() {
        let (flag, inline_value) = match arg.split_once('=') {
            Some((f, v)) => (f.to_owned(), Some(v.to_owned())),
            None => (arg, None),
        };

        match flag.as_str() {
            "--help" | "-h" => {
                print_create_hdd_help();
                std::process::exit(0);
            }
            "--type" => {
                let val = if let Some(v) = inline_value {
                    v
                } else {
                    next_value("--type", args)?
                };
                hdd_type = Some(val.parse::<HddSizeType>().map_err(StringError)?);
            }
            other if !other.starts_with('-') && path.is_none() => {
                path = Some(PathBuf::from(other));
            }
            other => bail!("unknown argument: {other}"),
        }
    }

    let path = path.ok_or_else(|| StringError("missing required argument: <PATH>".into()))?;
    let hdd_type =
        hdd_type.ok_or_else(|| StringError("missing required option: --type <TYPE>".into()))?;
    Ok(Action::CreateHdd { path, hdd_type })
}

fn parse_copy_args(args: &mut impl Iterator<Item = String>) -> crate::Result<Action> {
    let mut positional: Vec<String> = Vec::new();
    for arg in args {
        match arg.as_str() {
            "--help" | "-h" => {
                print_copy_help();
                std::process::exit(0);
            }
            other if other.starts_with("--") => bail!("unknown argument: {other}"),
            other => positional.push(other.to_owned()),
        }
    }
    if positional.len() != 2 {
        bail!("copy expects exactly two arguments: <SOURCE> <DEST>");
    }
    let source = parse_copy_arg(&positional[0]);
    let dest = parse_copy_arg(&positional[1]);
    if matches!(&source, CopyArg::Host(_)) && matches!(&dest, CopyArg::Host(_)) {
        bail!("neither argument refers to a disk image; use a host filesystem copy tool instead");
    }
    Ok(Action::Copy { source, dest })
}

fn parse_convert_hdd_args(args: &mut impl Iterator<Item = String>) -> crate::Result<Action> {
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;

    for arg in args {
        match arg.as_str() {
            "--help" | "-h" => {
                print_convert_hdd_help();
                std::process::exit(0);
            }
            other if !other.starts_with('-') => {
                if input.is_none() {
                    input = Some(PathBuf::from(other));
                } else if output.is_none() {
                    output = Some(PathBuf::from(other));
                } else {
                    bail!("unexpected argument: {other}");
                }
            }
            other => bail!("unknown argument: {other}"),
        }
    }

    let input = input.ok_or_else(|| StringError("missing required argument: <INPUT>".into()))?;
    let output = output.ok_or_else(|| StringError("missing required argument: <OUTPUT>".into()))?;
    Ok(Action::ConvertHdd { input, output })
}

/// Parses the command line of the process into an action.
///
/// The process folders of the settings discovery are read only for a run.
pub fn parse_args() -> crate::Result<Action> {
    match parse_command_line(std::env::args().skip(1))? {
        CommandLine::Command(action) => Ok(action),
        CommandLine::Launch(request) => launch_action(request, &Locations::of_process()?),
    }
}

/// Parses `args` into an action. A run action reads its layers from the
/// settings file that the discovery finds in `locations`.
pub fn parse_args_from(
    args: impl IntoIterator<Item = String>,
    locations: &Locations,
) -> crate::Result<Action> {
    match parse_command_line(args)? {
        CommandLine::Command(action) => Ok(action),
        CommandLine::Launch(request) => launch_action(request, locations),
    }
}

/// A parsed command line: a command, or the inputs of a launch.
enum CommandLine {
    /// A command such as `create-fdd`.
    Command(Action),
    /// A launch. Its paths are still relative to the current directory.
    Launch(LaunchRequest),
}

/// Splits `args` into a command or the inputs of a launch.
///
/// A flag value is checked against the settings table.
fn parse_command_line(args: impl IntoIterator<Item = String>) -> crate::Result<CommandLine> {
    let mut request = LaunchRequest::default();
    let mut args = args.into_iter().peekable();

    while let Some(arg) = args.next() {
        let command = match arg.as_str() {
            "create-fdd" => Some(parse_create_fdd_args(&mut args)),
            "create-hdd" => Some(parse_create_hdd_args(&mut args)),
            "convert-hdd" => Some(parse_convert_hdd_args(&mut args)),
            "copy" => Some(parse_copy_args(&mut args)),
            _ => None,
        };
        if let Some(command) = command {
            return command.map(CommandLine::Command);
        }
        let (flag, inline_value) = match arg.split_once('=') {
            Some((f, v)) => (f.to_owned(), Some(v.to_owned())),
            None => (arg, None),
        };

        match flag.as_str() {
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            "--version" | "-V" => {
                print_version();
                std::process::exit(0);
            }
            "-c" | "--config" => {
                let path = flag_value(&flag, inline_value, &mut args)?;
                request.game_files.push(PathBuf::from(path));
            }
            "--settings" => {
                let path = flag_value(&flag, inline_value, &mut args)?;
                request.settings = Some(PathBuf::from(path));
            }
            other => {
                let Some(setting) = other.strip_prefix("--").and_then(find_setting) else {
                    bail!("unknown argument: {other}");
                };
                let value = match setting.kind {
                    ValueKind::OnOff => on_off_flag_value(inline_value, &mut args),
                    ValueKind::Choice(_)
                    | ValueKind::Number(_)
                    | ValueKind::Path(_)
                    | ValueKind::PathList => flag_value(&flag, inline_value, &mut args)?,
                };
                check_value(setting, &value)
                    .map_err(|message| StringError(format!("{flag}: {message}")))?;
                request.flags.push((setting, value));
            }
        }
    }
    Ok(CommandLine::Launch(request))
}

/// Resolves the relative paths of `request` against the current directory and
/// builds the run action or the manager action of its startup mode.
fn launch_action(mut request: LaunchRequest, locations: &Locations) -> crate::Result<Action> {
    let current_directory = &locations.current_directory;
    for file in &mut request.game_files {
        *file = resolve_path(current_directory, file);
    }
    for (setting, value) in &mut request.flags {
        *value = resolve_value(setting, current_directory, value);
    }
    match startup_mode(&request).map_err(StringError)? {
        StartupMode::Direct => {
            let (config, key_map) = build_configuration(&request, locations)?;
            Ok(Action::Run(Box::new(config), key_map))
        }
        StartupMode::Manager => Ok(Action::Manager(Box::new(ManagerStart {
            request,
            locations: locations.clone(),
        }))),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    #[cfg(feature = "pc98")]
    use common::MachineModel;
    use common::MonitorTiming;
    #[cfg(feature = "pc88")]
    use common::{CpuMode, MemoryWaitSwitch};
    #[cfg(feature = "pc60")]
    use machine_60::Pc6000Model;
    use machine_factory::config::settings::{MACHINE_KEY, NumberRange};
    #[cfg(feature = "msx")]
    use machine_msx::MsxModel;
    #[cfg(feature = "x68k")]
    use machine_x68k::X68kModel;

    #[cfg(feature = "pc98")]
    use super::test_support::write_file;
    use super::{
        test_support::{
            TestLocations, first_model, parse_error, parse_run_config, with_first_model,
        },
        *,
    };

    /// Keys that exist only in builds with the PC-88 family.
    const PC88_FEATURE_KEYS: &[&str] = &["pc88-memory-wait", "pc88-8mhz-wait"];

    /// Returns sample values of a number range: both ends and the middle.
    fn number_samples(range: NumberRange) -> Vec<String> {
        let middle = ((range.minimum + range.maximum) / 2.0 / range.step).round() * range.step;
        [range.minimum, middle, range.maximum]
            .iter()
            .map(|value| {
                if range.integer {
                    format!("{value:.0}")
                } else {
                    format!("{value:?}")
                }
            })
            .collect()
    }

    /// Returns the sample values of every setting. Path samples name `existing_path`.
    fn sample_values(setting: &Setting, existing_path: &Path) -> Vec<String> {
        let path = existing_path.to_string_lossy().into_owned();
        match setting.kind {
            ValueKind::Choice(choices) => {
                let compiled = Families::compiled();
                choices
                    .iter()
                    .filter(|choice| choice.families.intersects(compiled))
                    .map(|choice| choice.value.to_owned())
                    .collect()
            }
            ValueKind::OnOff => vec!["on".to_owned(), "off".to_owned()],
            ValueKind::Number(range) => number_samples(range),
            ValueKind::Path(_) | ValueKind::PathList => vec![path],
        }
    }

    /// Verifies that every key parses to the same configuration from a flag and from a file.
    ///
    /// The file is the settings file. The `machine` key reads from a game
    /// file, because the `--machine` of the command line outranks the settings file.
    #[test]
    fn every_setting_parses_the_same_from_a_flag_and_from_a_file() {
        let test = TestLocations::new("every_setting");
        let existing_path = test.file("existing.img", "");
        for setting in SETTINGS {
            for value in sample_values(setting, &existing_path) {
                let flag = format!("--{}", setting.key);
                let (separate, _) = test.parse(&with_first_model(&[&flag, &value]));
                let inline_flag = format!("--{}={value}", setting.key);
                let (inline, _) = test.parse(&with_first_model(&[&inline_flag]));
                let file_name = format!("{}.conf", setting.key);
                let file_line = format!("{} = {value}\n", setting.key);
                let from_file = if setting.key == MACHINE_KEY {
                    test.parse_game(&file_name, &file_line).0
                } else {
                    test.parse_settings(&file_name, &file_line)
                };
                assert_eq!(separate, from_file, "{} = {value}", setting.key);
                assert_eq!(inline, from_file, "{} = {value}", setting.key);
                assert_eq!(
                    (setting.format)(&from_file),
                    vec![value.clone()],
                    "{} formats its value",
                    setting.key
                );
            }
        }
    }

    /// Verifies that repeated media flags and file lines append their images in order.
    #[test]
    fn path_list_flags_and_file_lines_append_in_order() {
        let test = TestLocations::new("append");
        let first_text = test.file("first.d88", "").to_string_lossy().into_owned();
        let second_text = test.file("second.d88", "").to_string_lossy().into_owned();
        for key in ["fdd1", "fdd2", "cdrom"] {
            let flag = format!("--{key}");
            let (from_flags, _) = test.parse(&with_first_model(&[
                &flag,
                &first_text,
                &flag,
                &second_text,
            ]));
            let (from_file, _) = test.parse_game(
                &format!("{key}_list.conf"),
                &format!(
                    "machine = {}\n{key} = {first_text}\n{key} = {second_text}\n",
                    first_model()
                ),
            );
            assert_eq!(from_flags, from_file, "{key}");
            let setting = find_setting(key).expect("media key is a table key");
            assert_eq!(
                (setting.format)(&from_file),
                vec![first_text.clone(), second_text.clone()],
                "{key}"
            );
        }
    }

    /// Verifies that an on/off flag accepts the bare form and both value forms.
    #[test]
    fn on_off_flags_accept_the_bare_flag_and_both_value_forms() {
        assert!(parse_run_config(&["--bios"]).0.bios);
        assert!(parse_run_config(&["--bios", "on"]).0.bios);
        assert!(!parse_run_config(&["--bios", "off"]).0.bios);
        assert!(!parse_run_config(&["--bios=off"]).0.bios);
        assert!(parse_run_config(&["--enable-extractor"]).0.enable_extractor);

        let (config, _) = parse_run_config(&["--crt", "--bios", "--monitor", "15k"]);
        assert!(config.crt);
        assert!(config.bios);
        assert_eq!(config.monitor, MonitorTiming::Fixed15kHz);

        let (config, _) = parse_run_config(&["--crt", "off", "--adpcm-ram=off"]);
        assert!(!config.crt);
        assert!(!config.adpcm_ram);
    }

    /// Verifies that unknown flags and invalid flag values end with an error.
    #[test]
    fn unknown_flags_and_invalid_values_are_errors() {
        assert_eq!(parse_error(&["--foo"]), "unknown argument: --foo");
        assert_eq!(parse_error(&["--foo=bar"]), "unknown argument: --foo");
        assert_eq!(parse_error(&["foo"]), "unknown argument: foo");
        assert_eq!(parse_error(&["--cpu-mode"]), "missing value for --cpu-mode");
        assert_eq!(parse_error(&["--settings"]), "missing value for --settings");
        assert_eq!(
            parse_error(&["--bios=maybe"]),
            "--bios: invalid value 'maybe', expected on or off"
        );
        assert_eq!(
            parse_error(&["--pc6000-phase", "4"]),
            "--pc6000-phase: invalid composite phase '4', expected 0, 1, 2 or 3"
        );
        assert!(parse_error(&["--audio-volume", "1.5"]).starts_with("--audio-volume: "));
        assert!(parse_error(&["--force-gdc-clock", "3"]).starts_with("--force-gdc-clock: "));
        assert!(parse_error(&["--machine", "FOOBAR"]).starts_with("--machine: "));
        assert_eq!(
            parse_error(&["--config-folder="]),
            "--config-folder: the value is empty, a folder is needed"
        );
        assert!(parse_error(&["--fdd1", "missing.d88"]).starts_with("fdd1 image not found: "));
    }

    /// Verifies that every key parses on the default machine, whatever families read it.
    #[test]
    fn family_filter_rejects_nothing_at_parse_time() {
        let test = TestLocations::new("filter");
        let existing_path = test.file("filter.img", "");
        let mut args: Vec<String> = Vec::new();
        for setting in SETTINGS.iter().filter(|setting| setting.key != MACHINE_KEY) {
            if let Some(value) = sample_values(setting, &existing_path).into_iter().next() {
                args.push(format!("--{}={value}", setting.key));
            }
        }
        let arguments: Vec<&str> = args.iter().map(String::as_str).collect();
        let _ = test.parse(&with_first_model(&arguments));
    }

    /// Verifies that the reference configuration names table keys and documents every grouped setting.
    #[test]
    fn reference_configuration_documents_every_editor_setting() {
        let reference = include_str!("../../../configuration/default.conf");
        let mut documented: Vec<&str> = Vec::new();
        for line in reference.lines() {
            let line = line.trim_start_matches(';').trim();
            let Some((key, _)) = line.split_once(" = ") else {
                continue;
            };
            if key.starts_with("key.") || key.contains(' ') {
                continue;
            }
            if !cfg!(feature = "pc88") && PC88_FEATURE_KEYS.contains(&key) {
                continue;
            }
            assert!(find_setting(key).is_some(), "{key} is not a table key");
            documented.push(key);
        }
        for setting in SETTINGS.iter().filter(|setting| setting.section.is_some()) {
            assert!(
                documented.contains(&setting.key),
                "{} is missing from configuration/default.conf",
                setting.key
            );
        }
    }

    /// Verifies that the help lists every table key and the flags outside the table.
    #[test]
    fn help_lists_every_setting() {
        let options = option_lines();
        for setting in SETTINGS {
            assert!(
                options.contains(&format!("--{} ", setting.key)),
                "--{} is missing from the help",
                setting.key
            );
        }
        assert!(options.contains("--config <PATH>"));
        assert!(options.contains("--settings <PATH>"));
        assert!(options.contains("--help"));
        assert!(options.contains("--version"));
        assert!(options.contains("Default: <roms-folder>/pc6000."));
        assert!(options.contains("Families: pc6000."));
    }

    #[cfg(feature = "pc98")]
    /// Verifies that a relative `--config-folder` and a relative `-c` resolve against the current directory.
    #[test]
    fn relative_config_flags_resolve_against_the_current_directory() {
        let test = TestLocations::new("config_flags");
        let current = test.current().to_path_buf();
        let settings = test.file("elsewhere.conf", "soundboard = 26k\n");
        write_file(
            &current.join("games").join("pc98.conf"),
            "soundboard = 14\n",
        );
        write_file(&current.join("game.conf"), "machine = PC9801VM\n");
        let settings_text = settings.to_string_lossy().into_owned();

        let (config, _) = test.parse(&[
            "--settings",
            &settings_text,
            "--config-folder",
            "games",
            "-c",
            "game.conf",
        ]);
        assert_eq!(config.config_folder, current.join("games"));
        assert_eq!(config.soundboard, SoundboardType::Sb14);
        assert_eq!(config.machine, MachineModel::PC9801VM);
    }

    #[cfg(feature = "pc88")]
    #[test]
    fn machine_flag_selects_the_pc88_target() {
        let (config, _) = parse_run_config(&["--machine", "PC8801MC"]);
        assert_eq!(config.target, Target::Pc88);
    }

    #[cfg(feature = "pc60")]
    #[test]
    fn machine_flag_selects_the_pc60_target() {
        let (config, _) = parse_run_config(&["--machine", "PC6001MK2SR"]);
        assert_eq!(config.target, Target::Pc60);
        assert_eq!(config.pc60_model, Pc6000Model::Pc6001Mk2Sr);
    }

    /// Verifies the startup decision: a machine or a game file starts the game, media alone is an error, the rest opens the manager.
    #[test]
    fn startup_mode_follows_the_machine_and_the_game_files() {
        let setting = |key: &str| find_setting(key).expect("table key");
        let request =
            |flags: Vec<(&'static Setting, &str)>, game_files: Vec<PathBuf>| LaunchRequest {
                flags: flags
                    .into_iter()
                    .map(|(setting, value)| (setting, value.to_owned()))
                    .collect(),
                game_files,
                ..LaunchRequest::default()
            };
        assert_eq!(
            startup_mode(&request(Vec::new(), Vec::new())),
            Ok(StartupMode::Manager)
        );
        assert_eq!(
            startup_mode(&request(
                vec![
                    (setting("roms-folder"), "/roms"),
                    (setting("window-mode"), "fullscreen"),
                    (setting("crt"), "off"),
                ],
                Vec::new()
            )),
            Ok(StartupMode::Manager)
        );
        assert_eq!(
            startup_mode(&request(vec![(setting(MACHINE_KEY), "X1")], Vec::new())),
            Ok(StartupMode::Direct)
        );
        assert_eq!(
            startup_mode(&request(Vec::new(), vec![PathBuf::from("/game.conf")])),
            Ok(StartupMode::Direct)
        );
        assert_eq!(
            startup_mode(&request(
                vec![(setting("fdd1"), "/disk.d88")],
                vec![PathBuf::from("/game.conf")]
            )),
            Ok(StartupMode::Direct)
        );
        for key in [
            "fdd1",
            "fdd2",
            "cdrom",
            "hdd1",
            "hdd2",
            "cartridge",
            "cassette",
        ] {
            assert_eq!(
                startup_mode(&request(vec![(setting(key), "/image")], Vec::new())),
                Err(format!("--{key} needs --machine or --config")),
                "{key}"
            );
        }
    }

    /// Verifies that a command line without a machine opens the manager and that the machine of the settings file does not launch.
    #[test]
    fn a_command_line_without_a_machine_opens_the_manager() {
        let test = TestLocations::new("manager");
        let settings = test.file("neetan.conf", &format!("machine = {}\n", first_model()));
        let settings_text = settings.to_string_lossy().into_owned();
        let Action::Manager(start) =
            test.action(&["--settings", &settings_text, "--roms-folder", "roms"])
        else {
            panic!("expected the manager action");
        };
        assert_eq!(start.request.settings, Some(settings));
        assert_eq!(start.locations, test.locations);
        let (setting, value) = &start.request.flags[0];
        assert_eq!(setting.key, "roms-folder");
        assert_eq!(PathBuf::from(value), test.current().join("roms"));
        assert!(matches!(test.action(&[]), Action::Manager(_)));
        let disk = test.file("disk.d88", "").to_string_lossy().into_owned();
        assert_eq!(
            test.error(&["--fdd1", &disk]),
            "--fdd1 needs --machine or --config"
        );
    }

    #[test]
    fn copy_parser_recognizes_every_supported_disk_extension() {
        for extension in crate::copy::HDD_EXTENSIONS
            .iter()
            .chain(crate::copy::FDD_EXTENSIONS)
        {
            for extension in [
                extension.to_ascii_lowercase(),
                extension.to_ascii_uppercase(),
            ] {
                let argument = format!("disk.{extension}:A:\\FILE.TXT");
                let CopyArg::Image {
                    image_path,
                    dos_path,
                } = parse_copy_arg(&argument)
                else {
                    panic!("{extension} should identify a disk image");
                };
                assert_eq!(image_path, PathBuf::from(format!("disk.{extension}")));
                assert_eq!(dos_path, b"A:\\FILE.TXT");
            }
        }
    }

    #[cfg(feature = "msx")]
    #[test]
    fn machine_flag_selects_msx_and_its_rom_directory() {
        let test = TestLocations::new("msx_roms");
        let (config, _key_map) = test.parse(&["--machine", "msx", "--msx-roms", "roms/msx"]);
        assert_eq!(config.target, Target::Msx);
        assert_eq!(config.msx_model, MsxModel::Msx);
        assert_eq!(config.msx_roms, Some(test.current().join("roms/msx")));
    }

    #[cfg(feature = "msx")]
    #[test]
    fn machine_flag_exposes_msx2_and_msx2_plus() {
        let (config, _) = parse_run_config(&["--machine", "MSX2"]);
        assert_eq!(config.target, Target::Msx);
        assert_eq!(config.msx_model, MsxModel::Msx2);

        let (config, _) = parse_run_config(&["--machine", "MSX2", "--machine", "MSX2PLUS"]);
        assert_eq!(config.target, Target::Msx);
        assert_eq!(config.msx_model, MsxModel::Msx2Plus);
    }

    #[cfg(feature = "pc98")]
    #[test]
    fn machine_flag_selects_the_pc98_target() {
        let (config, _) = parse_run_config(&["--machine", "PC9801VX"]);
        assert_eq!(config.target, Target::Pc98);
        assert_eq!(config.machine, MachineModel::PC9801VX);
    }

    #[cfg(feature = "x68k")]
    #[test]
    fn machine_flag_selects_x68000_and_its_rom_directory() {
        let test = TestLocations::new("x68k_roms");
        let (config, _key_map) =
            test.parse(&["--machine", "x68000xvi", "--x68k-roms", "roms/x68kxvi"]);
        assert_eq!(config.target, Target::X68k);
        assert_eq!(config.x68k_model, X68kModel::X68000Xvi);
        assert_eq!(config.x68k_roms, Some(test.current().join("roms/x68kxvi")));
    }

    #[test]
    fn unknown_machine_is_rejected() {
        assert!(parse_error(&["--machine", "FOOBAR"]).starts_with("--machine: unknown machine"));
    }

    #[test]
    fn fdd_type_parses_2d() {
        assert_eq!("2d".parse::<FddType>(), Ok(FddType::D2));
        assert_eq!("2hd".parse::<FddType>(), Ok(FddType::Hd2));
        assert_eq!("2dd".parse::<FddType>(), Ok(FddType::Dd2));
        assert!("2xx".parse::<FddType>().is_err());
    }

    #[cfg(feature = "pc88")]
    #[test]
    fn pc88_n_family_boot_modes_derive_compatible_defaults() {
        for boot_mode in ["n", "n80", "n80sr"] {
            let (config, _key_map) =
                parse_run_config(&["--machine", "PC8801MC", "--boot-mode", boot_mode]);
            assert_eq!(config.cpu_mode, CpuMode::Low, "{boot_mode}");
            assert_eq!(
                config.pc88_memory_wait,
                MemoryWaitSwitch::Compatible,
                "{boot_mode}"
            );
        }
    }

    #[cfg(feature = "pc88")]
    #[test]
    fn pc88_v1s_boot_mode_derives_compatible_defaults() {
        let (config, _key_map) = parse_run_config(&["--machine", "PC8801MC", "--boot-mode", "v1s"]);
        assert_eq!(config.cpu_mode, CpuMode::Low);
        assert_eq!(config.pc88_memory_wait, MemoryWaitSwitch::Compatible);
    }

    #[cfg(feature = "pc88")]
    #[test]
    fn pc88_v2_boot_mode_keeps_fast_defaults() {
        let (config, _key_map) = parse_run_config(&["--machine", "PC8801MC", "--boot-mode", "v2"]);
        assert_eq!(config.cpu_mode, CpuMode::High);
        assert_eq!(config.pc88_memory_wait, MemoryWaitSwitch::Fast);
    }

    #[cfg(feature = "pc88")]
    #[test]
    fn explicit_pc88_cpu_mode_overrides_boot_mode_default() {
        let (config, _key_map) = parse_run_config(&[
            "--machine",
            "PC8801MC",
            "--boot-mode",
            "n",
            "--cpu-mode",
            "high",
        ]);
        assert_eq!(config.cpu_mode, CpuMode::High);
        assert_eq!(config.pc88_memory_wait, MemoryWaitSwitch::Compatible);
    }

    #[cfg(feature = "pc88")]
    #[test]
    fn explicit_pc88_memory_wait_overrides_boot_mode_default() {
        let (config, _key_map) = parse_run_config(&[
            "--machine",
            "PC8801MC",
            "--boot-mode",
            "v1s",
            "--pc88-memory-wait",
            "fast",
        ]);
        assert_eq!(config.cpu_mode, CpuMode::Low);
        assert_eq!(config.pc88_memory_wait, MemoryWaitSwitch::Fast);
    }

    #[cfg(feature = "pc88")]
    #[test]
    fn config_file_explicit_pc88_values_override_boot_mode_default() {
        let test = TestLocations::new("pc88_explicit");
        let (config, _key_map) = test.parse_game(
            "pc88_explicit.conf",
            "machine=PC8801MC\nboot-mode=n\ncpu-mode=high\npc88-memory-wait=fast\n",
        );
        assert_eq!(config.cpu_mode, CpuMode::High);
        assert_eq!(config.pc88_memory_wait, MemoryWaitSwitch::Fast);
    }

    #[test]
    fn cdrom_compat_flag_parses_on_and_off() {
        assert!(!parse_run_config(&[]).0.cdrom_compat);
        assert!(parse_run_config(&["--cdrom-compat", "on"]).0.cdrom_compat);
        assert!(!parse_run_config(&["--cdrom-compat", "off"]).0.cdrom_compat);
    }

    #[cfg(feature = "towns")]
    #[test]
    fn config_file_cdrom_compat_parses_on() {
        let test = TestLocations::new("cdrom_compat");
        let (config, _key_map) = test.parse_game(
            "cdrom_compat.conf",
            "machine=FMTownsIICX\ncdrom-compat=on\n",
        );
        assert!(config.cdrom_compat);
    }
}
