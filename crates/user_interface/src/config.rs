use std::path::{Path, PathBuf};

use common::{Context, StringError, bail, info, warn};
use machine_factory::config::settings::{
    ExplicitSettings, Families, PathKind, SETTINGS, Setting, ValueKind, apply_derived_defaults,
    find_setting,
};
pub use machine_factory::config::{
    AspectMode, Backend, BootMode, EmulatorConfig, ForceGdcClock, GraphicboardType, MidiDevice,
    ScalingMode, SoundboardType, Target, WindowMode,
};

use crate::{config::keymap::parse_key_binding, input::KeyOverrides};

mod keymap;

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

/// Returns the description of a setting in the `--help` list: the help text,
/// the values, the families that read it and the built-in default.
fn setting_description(setting: &Setting, defaults: &EmulatorConfig) -> String {
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
    let default = (setting.format)(defaults);
    if !default.is_empty() {
        description.push_str(&format!(" Default: {}.", default.join(", ")));
    }
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
    let defaults = EmulatorConfig::default();
    let mut rows: Vec<(String, String)> = vec![(
        "-c, --config <PATH>".to_owned(),
        "Load configuration from file".to_owned(),
    )];
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
Every option except --config, --help and --version has a configuration file key of the same name.

Global configuration:
  A global config is loaded from the OS data directory if it exists.
  Layering: defaults -> global config -> --config file -> CLI arguments

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

pub enum Action {
    Run(Box<EmulatorConfig>, KeyOverrides),
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

pub fn parse_args() -> crate::Result<Action> {
    parse_args_from(std::env::args().skip(1), true)
}

fn parse_args_from(
    args: impl IntoIterator<Item = String>,
    load_global_config: bool,
) -> crate::Result<Action> {
    let mut config = EmulatorConfig::default();
    let mut explicit = ExplicitSettings::default();
    let mut key_map = KeyOverrides::new();

    if load_global_config
        && let Some(global_path) = global_config_path()
        && global_path.exists()
    {
        apply_config_file(&mut config, &mut explicit, &mut key_map, &global_path)?;
        info!("Loaded global config: {}", global_path.display());
    }

    let mut args = args.into_iter().peekable();

    while let Some(arg) = args.next() {
        if arg == "create-fdd" {
            return parse_create_fdd_args(&mut args);
        }
        if arg == "create-hdd" {
            return parse_create_hdd_args(&mut args);
        }
        if arg == "convert-hdd" {
            return parse_convert_hdd_args(&mut args);
        }
        if arg == "copy" {
            return parse_copy_args(&mut args);
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
                apply_config_file(&mut config, &mut explicit, &mut key_map, Path::new(&path))?;
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
                apply_setting(&mut config, &mut explicit, &mut key_map, setting, &value)
                    .map_err(|message| StringError(format!("{flag}: {message}")))?;
            }
        }
    }

    apply_derived_defaults(&mut config, explicit);
    validate_paths(&config)?;

    Ok(Action::Run(Box::new(config), key_map))
}

fn validate_paths(config: &EmulatorConfig) -> crate::Result<()> {
    for path in &config.fdd1 {
        if !path.exists() {
            bail!("fdd1 image not found: {}", path.display());
        }
    }
    for path in &config.fdd2 {
        if !path.exists() {
            bail!("fdd2 image not found: {}", path.display());
        }
    }
    for path in &config.cdrom {
        if !path.exists() {
            bail!("cdrom image not found: {}", path.display());
        }
    }
    if let Some(ref path) = config.hdd1
        && !path.exists()
    {
        bail!("hdd1 image not found: {}", path.display());
    }
    if let Some(ref path) = config.hdd2
        && !path.exists()
    {
        bail!("hdd2 image not found: {}", path.display());
    }
    Ok(())
}

pub fn parse_config_file(path: &Path) -> crate::Result<EmulatorConfig> {
    let mut config = EmulatorConfig::default();
    let mut explicit = ExplicitSettings::default();
    let mut key_map = KeyOverrides::new();
    apply_config_file(&mut config, &mut explicit, &mut key_map, path)?;
    apply_derived_defaults(&mut config, explicit);
    Ok(config)
}

fn apply_config_file(
    config: &mut EmulatorConfig,
    explicit: &mut ExplicitSettings,
    key_map: &mut KeyOverrides,
    path: &Path,
) -> crate::Result<()> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;

    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        let Some((key, val)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let val = val.trim();
        if let Some(host_name) = key.strip_prefix("key.") {
            match parse_key_binding(config.target, host_name, val) {
                Some((host, code)) => key_map.set(host, code),
                None => warn!("Invalid key binding: {key}={val}"),
            }
            continue;
        }
        match find_setting(key) {
            Some(setting) => {
                if let Err(message) = apply_setting(config, explicit, key_map, setting, val) {
                    warn!("Invalid {key} in config: {message}");
                }
            }
            None => warn!("Unknown config key: {key}"),
        }
    }

    Ok(())
}

/// Applies one value of `setting` and records an explicit value. A family
/// change clears the key overrides, since native override codes belong to one
/// family and later `key.*` lines parse against the new family.
fn apply_setting(
    config: &mut EmulatorConfig,
    explicit: &mut ExplicitSettings,
    key_map: &mut KeyOverrides,
    setting: &Setting,
    value: &str,
) -> Result<(), String> {
    let previous = config.target;
    explicit.apply(config, setting, value)?;
    if config.target != previous {
        *key_map = KeyOverrides::new();
    }
    Ok(())
}

fn global_config_path() -> Option<PathBuf> {
    let pref_path = sdl3::filesystem::get_pref_path(crate::COMPANY_NAME, crate::GAME_NAME)?;
    Some(pref_path.join("global.conf"))
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "pc98")]
    use common::MachineModel;
    use common::MonitorTiming;
    #[cfg(feature = "pc88")]
    use common::{CpuMode, MemoryWaitSwitch};
    #[cfg(feature = "pc60")]
    use machine_60::Pc6000Model;
    use machine_factory::config::settings::NumberRange;
    #[cfg(feature = "msx")]
    use machine_msx::MsxModel;
    #[cfg(feature = "x68k")]
    use machine_x68k::X68kModel;

    use super::*;

    fn parse_run_config(args: &[&str]) -> (EmulatorConfig, KeyOverrides) {
        match parse_args_from(args.iter().map(|arg| (*arg).to_owned()), false)
            .expect("arguments should parse")
        {
            Action::Run(config, key_map) => (*config, key_map),
            _ => panic!("expected run action"),
        }
    }

    /// Returns the error of an argument list that must fail to parse.
    fn parse_error(args: &[&str]) -> String {
        match parse_args_from(args.iter().map(|arg| (*arg).to_owned()), false) {
            Ok(_) => panic!("{args:?} should fail to parse"),
            Err(error) => error.to_string(),
        }
    }

    /// Writes `contents` to a unique temporary file and returns its path.
    fn temporary_file(name: &str, contents: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("neetan_config_test_{}_{name}", std::process::id()));
        std::fs::write(&path, contents).expect("temporary file should be written");
        path
    }

    /// Parses a configuration file holding `contents`.
    fn parse_file_contents(name: &str, contents: &str) -> EmulatorConfig {
        let path = temporary_file(name, contents);
        let config = parse_config_file(&path).expect("config file should parse");
        let _ = std::fs::remove_file(path);
        config
    }

    /// Selects a machine through the settings table.
    fn select_machine(
        config: &mut EmulatorConfig,
        key_map: &mut KeyOverrides,
        value: &str,
    ) -> Result<(), String> {
        let setting = find_setting("machine").expect("machine is a table key");
        apply_setting(
            config,
            &mut ExplicitSettings::default(),
            key_map,
            setting,
            value,
        )
    }

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
    #[test]
    fn every_setting_parses_the_same_from_a_flag_and_from_a_file() {
        let existing_path = temporary_file("existing.img", "");
        for setting in SETTINGS {
            for value in sample_values(setting, &existing_path) {
                let flag = format!("--{}", setting.key);
                let (separate, _) = parse_run_config(&[&flag, &value]);
                let inline_flag = format!("--{}={value}", setting.key);
                let (inline, _) = parse_run_config(&[&inline_flag]);
                let from_file = parse_file_contents(
                    &format!("{}.conf", setting.key),
                    &format!("{} = {value}\n", setting.key),
                );
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
        let _ = std::fs::remove_file(existing_path);
    }

    /// Verifies that repeated media flags and file lines append their images in order.
    #[test]
    fn path_list_flags_and_file_lines_append_in_order() {
        let first = temporary_file("first.d88", "");
        let second = temporary_file("second.d88", "");
        let first_text = first.to_string_lossy().into_owned();
        let second_text = second.to_string_lossy().into_owned();
        for key in ["fdd1", "fdd2", "cdrom"] {
            let flag = format!("--{key}");
            let (from_flags, _) = parse_run_config(&[&flag, &first_text, &flag, &second_text]);
            let from_file = parse_file_contents(
                &format!("{key}_list.conf"),
                &format!("{key} = {first_text}\n{key} = {second_text}\n"),
            );
            assert_eq!(from_flags, from_file, "{key}");
            let setting = find_setting(key).expect("media key is a table key");
            assert_eq!(
                (setting.format)(&from_file),
                vec![first_text.clone(), second_text.clone()],
                "{key}"
            );
        }
        let _ = std::fs::remove_file(first);
        let _ = std::fs::remove_file(second);
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
    }

    /// Verifies that the file parser skips unknown keys and invalid values.
    #[test]
    fn config_file_skips_unknown_keys_and_invalid_values() {
        let config = parse_file_contents(
            "invalid.conf",
            "fdd3 = disk.d88\ncpu-mode = turbo\ncrt = maybe\nscaling = nearest\n",
        );
        let expected = EmulatorConfig {
            scaling: ScalingMode::Nearest,
            ..EmulatorConfig::default()
        };
        assert_eq!(config, expected);
    }

    /// Verifies that every key parses on the default machine, whatever families read it.
    #[test]
    fn family_filter_rejects_nothing_at_parse_time() {
        let existing_path = temporary_file("filter.img", "");
        let mut args: Vec<String> = Vec::new();
        for setting in SETTINGS.iter().filter(|setting| setting.key != "machine") {
            if let Some(value) = sample_values(setting, &existing_path).into_iter().next() {
                args.push(format!("--{}={value}", setting.key));
            }
        }
        let arguments: Vec<&str> = args.iter().map(String::as_str).collect();
        let _ = parse_run_config(&arguments);
        let _ = std::fs::remove_file(existing_path);
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
        assert!(options.contains("--help"));
        assert!(options.contains("--version"));
    }

    #[cfg(feature = "pc88")]
    #[test]
    fn machine_flag_selects_the_pc88_target() {
        let mut config = EmulatorConfig::default();
        let mut key_map = KeyOverrides::new();
        select_machine(&mut config, &mut key_map, "PC8801MC").expect("PC8801MC is valid");
        assert_eq!(config.target, Target::Pc88);
    }

    #[cfg(feature = "pc60")]
    #[test]
    fn machine_flag_selects_the_pc60_target() {
        let mut config = EmulatorConfig::default();
        let mut key_map = KeyOverrides::new();
        select_machine(&mut config, &mut key_map, "PC6001MK2SR").expect("PC6001MK2SR is valid");
        assert_eq!(config.target, Target::Pc60);
        assert_eq!(config.pc60_model, Pc6000Model::Pc6001Mk2Sr);
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
        let (config, _key_map) = parse_run_config(&["--machine", "msx", "--msx-roms", "roms/msx"]);
        assert_eq!(config.target, Target::Msx);
        assert_eq!(config.msx_model, MsxModel::Msx);
        assert_eq!(config.msx_roms, Some(PathBuf::from("roms/msx")));
    }

    #[cfg(feature = "msx")]
    #[test]
    fn machine_flag_exposes_msx2_and_msx2_plus() {
        let mut config = EmulatorConfig::default();
        let mut key_map = KeyOverrides::new();
        select_machine(&mut config, &mut key_map, "MSX2").expect("MSX2 is valid");
        assert_eq!(config.target, Target::Msx);
        assert_eq!(config.msx_model, MsxModel::Msx2);

        select_machine(&mut config, &mut key_map, "MSX2PLUS").expect("MSX2PLUS is valid");
        assert_eq!(config.target, Target::Msx);
        assert_eq!(config.msx_model, MsxModel::Msx2Plus);
    }

    #[cfg(feature = "pc98")]
    #[test]
    fn machine_flag_selects_the_pc98_target() {
        let mut config = EmulatorConfig::default();
        let mut key_map = KeyOverrides::new();
        select_machine(&mut config, &mut key_map, "PC9801VX").expect("PC9801VX is valid");
        assert_eq!(config.target, Target::Pc98);
        assert_eq!(config.machine, MachineModel::PC9801VX);
    }

    #[cfg(feature = "x68k")]
    #[test]
    fn machine_flag_selects_x68000_and_its_rom_directory() {
        let (config, _key_map) =
            parse_run_config(&["--machine", "x68000xvi", "--x68k-roms", "roms/x68kxvi"]);
        assert_eq!(config.target, Target::X68k);
        assert_eq!(config.x68k_model, X68kModel::X68000Xvi);
        assert_eq!(config.x68k_roms, Some(PathBuf::from("roms/x68kxvi")));
    }

    #[cfg(feature = "at")]
    #[test]
    fn config_file_applies_named_pc_at_key_binding() {
        let path = temporary_file("at_key.conf", "machine=AT486DX66\nkey.B=A\n");
        let (config, key_map) =
            parse_run_config(&["--config", path.to_str().expect("path is UTF-8")]);
        let _ = std::fs::remove_file(path);

        assert_eq!(config.target, Target::At);
        assert_eq!(key_map.get(sdl3::keyboard::Scancode::B), Some(0x1E));
    }

    #[cfg(all(feature = "at", feature = "msx"))]
    /// Verifies that a family change clears the key overrides of the previous family.
    #[test]
    fn family_change_clears_key_overrides() {
        let path = temporary_file(
            "family_change.conf",
            "machine=AT486DX66\nkey.B=A\nmachine=MSX\n",
        );
        let (config, key_map) =
            parse_run_config(&["--config", path.to_str().expect("path is UTF-8")]);
        let _ = std::fs::remove_file(path);

        assert_eq!(config.target, Target::Msx);
        assert_eq!(key_map.get(sdl3::keyboard::Scancode::B), None);
    }

    #[test]
    fn unknown_machine_is_rejected() {
        let mut config = EmulatorConfig::default();
        let mut key_map = KeyOverrides::new();
        assert!(select_machine(&mut config, &mut key_map, "FOOBAR").is_err());
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
        let path = temporary_file(
            "pc88_explicit.conf",
            "machine=PC8801MC\nboot-mode=n\ncpu-mode=high\npc88-memory-wait=fast\n",
        );
        let (config, _key_map) =
            parse_run_config(&["--config", path.to_str().expect("path is UTF-8")]);
        let _ = std::fs::remove_file(path);

        assert_eq!(config.cpu_mode, CpuMode::High);
        assert_eq!(config.pc88_memory_wait, MemoryWaitSwitch::Fast);
    }

    #[test]
    fn cdrom_compat_flag_parses_on_and_off() {
        assert!(!parse_run_config(&[]).0.cdrom_compat);
        assert!(parse_run_config(&["--cdrom-compat", "on"]).0.cdrom_compat);
        assert!(!parse_run_config(&["--cdrom-compat", "off"]).0.cdrom_compat);
    }

    #[test]
    fn config_file_cdrom_compat_parses_on() {
        let path = temporary_file(
            "cdrom_compat.conf",
            "machine=FMTownsIICX\ncdrom-compat=on\n",
        );
        let (config, _key_map) =
            parse_run_config(&["--config", path.to_str().expect("path is UTF-8")]);
        let _ = std::fs::remove_file(path);

        assert!(config.cdrom_compat);
    }
}
