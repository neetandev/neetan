use std::path::{Path, PathBuf};

use common::{Context, CpuMode, MachineModel, MonitorTiming, StringError, bail, info, warn};
use machine60::Pc6000Model;
use machine88::{EightMhzWaitMode, MemoryWaitSwitch, Pc8801Model};
use machine88va::Pc88VaModel;
use machinefm7::Fm7Model;
use machinetowns::{TownsModel, TownsPadType};
use machinex1::{X1KeyboardMode, X1Model};
use machinex68k::X68kModel;

use crate::keyboard::{
    KeyMap, parse_key_binding, parse_key_binding_pc60, parse_key_binding_pc88,
    parse_key_binding_pc88va, parse_key_binding_towns, parse_key_binding_x68k,
};

fn next_value(flag: &str, args: &mut impl Iterator<Item = String>) -> crate::Result<String> {
    match args.next() {
        Some(val) => Ok(val),
        None => bail!("missing value for {flag}"),
    }
}

fn parse_on_off(val: &str, flag: &str) -> crate::Result<bool> {
    match val {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => bail!("invalid value '{val}' for {flag}, expected on or off"),
    }
}

fn parse_composite_phase(val: &str) -> crate::Result<u32> {
    match val.parse::<u32>() {
        Ok(phase) if phase <= 3 => Ok(phase),
        _ => bail!("invalid composite phase '{val}', expected 0, 1, 2 or 3"),
    }
}

fn print_help() {
    println!(
        "\
{} - Japanese computer emulator

Usage: neetan [OPTIONS]
       neetan <COMMAND>

Commands:
  create-fdd <PATH>             Create an empty floppy disk image (D88 or raw XDF)
  create-hdd <PATH>             Create an empty hard disk image (HDI, raw SCSI or X68000 HDF)
  convert-hdd <INPUT> <OUTPUT>  Convert HDD image between SASI and IDE
  copy <SOURCE> <DEST>          Copy files between host and FAT disk images

Options:
  -c, --config <PATH>           Load configuration from file
      --machine <TYPE>          Machine type: PC9801F, PC9801VM, PC9801VX, PC9801RS, PC9801RA, PC9821AS, PC9821AP, PC8801MC, PC88VA2, PC6001, PC6001MK2, PC6601, PC6001MK2SR, PC6601SR, FMTowns, FMTownsIICX, FMTownsIIMX, X68000, X68000SUPER, X68000XVI, X1, X1TURBO, FM7, FM77AV
      --cpu-mode <MODE>         CPU speed mode: low or high (PC-88 derives from boot mode; X68000 XVI 10/16.67 MHz; FM Towns base fixed 16 MHz, CX 16/20 MHz, MX 33/66 MHz)
      --boot-mode <MODE>        Boot mode; each machine accepts only its own values: PC-8801 v1s, v1h, v2 (default), n, n80, n80sr; FM-7 basic (default), dos
      --monitor <MODE>          Monitor timing: auto, 15k, 24k (default: auto; PC-8801 and X1 turbo)
      --pc88-memory-wait <MODE> PC-8801 memory wait: fast or compatible (default derives from boot mode)
      --pc88-8mhz-wait <MODE>   PC-8801 8 MHz wait: fast or compatible (default: fast; PC-8801 only)
      --pc98-roms <PATH>        Directory with the PC-98 ROM set (optional)
      --bios                    Use the real BIOS from --pc98-roms instead of HLE
      --pc88-roms <PATH>        Directory with the PC-8801MC ROM set (required)
      --pc88va-roms <PATH>      Directory with the PC-88VA2 ROM set (required)
      --pc6000-roms <PATH>      Directory with the PC-6000 ROM set (required)
      --x1-roms <PATH>          Directory with the Sharp X1 ROM set (required)
      --x1-keyboard <A|B>       X1 turbo keyboard mode switch (default: A)
      --fm7-roms <PATH>         Directory with the FM-7 / FM-77AV ROM set (required)
      --towns-roms <PATH>       Directory with the FM Towns ROM set (required)
      --x68k-roms <PATH>        Directory with the X68000 ROM set (required)
      --towns-pad <2|6>         FM Towns game pad type (default 6-button)
      --pc6000-phase <0-3>      Initial composite artifact-color phase; cycle with Right Ctrl + P (PC-6000 only)
      --fdd1 <PATH>             Floppy disk image for drive 1 (repeatable)
      --fdd2 <PATH>             Floppy disk image for drive 2 (repeatable)
      --hdd1 <PATH>             Hard disk image for drive 1 (PC-98 .hdi/.nhd/.thd, FM Towns .h0-.h4, X68000 .hdf)
      --hdd2 <PATH>             Hard disk image for drive 2 (PC-98 .hdi/.nhd/.thd, FM Towns .h0-.h4, X68000 .hdf)
      --cdrom <PATH>            CD-ROM disc image .cue or .ccd file (repeatable, PC-9821, FM Towns and X68000 SUPER/XVI)
      --cdrom-compat <on|off>   Slow/compatible CD-ROM drive timing (default: off; FM Towns only)
      --cartridge <PATH>        Cartridge ROM image to insert
      --cassette <PATH>         Cassette tape image to insert (.cas/.p6/.p6t, X1 .tap, FM-7 .t77)
      --audio-volume <FLOAT>    Audio volume 0.0-1.0
      --aspect-mode <MODE>      Display aspect mode: 4:3 or 1:1
      --crt <on|off>            Enable CRT effect (default: on; modern backend only)
      --scaling <MODE>          Scaling method: nearest, bilinear, pixelart (default: pixelart)
      --backend <BACKEND>       Rendering backend: modern or legacy (default: modern)
      --window-mode <MODE>      Window mode: windowed or fullscreen
      --force-gdc-clock <2.5|5> Force GDC clock to 2.5 or 5 MHz (default: auto)
      --graphicboard <TYPE>     Graphics accelerator board: none, ga1280a
      --soundboard <TYPE>       Sound board type: none, 14, 26k, 86, 86+26k, sb16, sb16+26k
      --adpcm-ram <on|off>      ADPCM RAM option for PC-9801-86 (default: on)
      --ems <on|off>            Enable EMS expanded memory (default: on)
      --xms <on|off>            Enable XMS extended memory (default: on)
      --midi <DEVICE>           MIDI device: none, mt32, sc55 (default: none)
      --mt32-roms <PATH>        Path to MT-32 ROM directory
      --sc55-roms <PATH>        Path to SC55 ROM directory
      --boot-device <DEVICE>    Boot device: auto, fdd1, fdd2, hdd1, hdd2, dos (default: auto)
      --printer <PATH>          Output file for printer (must exist)
      --enable-extractor        Copy on-screen text to the system clipboard
  -h, --help                    Print help
  -V, --version                 Print version

Global configuration:
  A global config is loaded from the OS data directory if it exists.
  Layering: defaults -> global config -> --config file -> CLI arguments

Run 'neetan <COMMAND> --help' for more information on a command.",
        crate::GAME_NAME,
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
Create an empty hard disk image (HDI for SASI/IDE, raw for SCSI, HDF for X68000)

Usage: neetan create-hdd <PATH> [OPTIONS]

Arguments:
  <PATH>  Output file path (.hdi for SASI/IDE, .h0-.h4 for SCSI, .hdf for X68000)

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
  x68scsi40  40 MB  (320 cyl, 8 heads, 32 spt, 512 B/sector)"
    );
}

fn print_copy_help() {
    println!(
        "\
Copy files and directories between the host filesystem and FAT-formatted
PC-98 disk images.

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

Image formats: HDI, NHD, THD (HDD); D88, D98, 88D, 98D, HDM, NFD, 2D (FDD).

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
    Run(Box<EmulatorConfig>),
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

/// File extensions that identify a disk image when looking for the
/// `IMAGE:DOSPATH` separator. The substring up to a colon must end with one
/// of these (case-insensitive) for the argument to be treated as an image
/// reference; otherwise the colon is part of a host path.
const IMAGE_EXTENSIONS: &[&str] = &[
    "hdi", "nhd", "thd", "d88", "d98", "88d", "98d", "hdm", "nfd", "2d",
];

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
        if IMAGE_EXTENSIONS
            .iter()
            .any(|known| ext.eq_ignore_ascii_case(known))
        {
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

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum HddSizeType {
    Mb5,
    Mb10,
    Mb15,
    Mb20,
    Mb30,
    Mb40,
    IdeMb40,
    IdeMb80,
    IdeMb120,
    IdeMb200,
    IdeMb500,
    ScsiMb20,
    ScsiMb40,
    ScsiMb100,
    ScsiMb200,
    ScsiMb340,
    ScsiMb540,
    X68kSasiMb10,
    X68kSasiMb20,
    X68kSasiMb40,
    X68kScsiMb20,
    X68kScsiMb40,
}

impl std::str::FromStr for HddSizeType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "sasi5" => Ok(Self::Mb5),
            "sasi10" => Ok(Self::Mb10),
            "sasi15" => Ok(Self::Mb15),
            "sasi20" => Ok(Self::Mb20),
            "sasi30" => Ok(Self::Mb30),
            "sasi40" => Ok(Self::Mb40),
            "ide40" => Ok(Self::IdeMb40),
            "ide80" => Ok(Self::IdeMb80),
            "ide120" => Ok(Self::IdeMb120),
            "ide200" => Ok(Self::IdeMb200),
            "ide500" => Ok(Self::IdeMb500),
            "scsi20" => Ok(Self::ScsiMb20),
            "scsi40" => Ok(Self::ScsiMb40),
            "scsi100" => Ok(Self::ScsiMb100),
            "scsi200" => Ok(Self::ScsiMb200),
            "scsi340" => Ok(Self::ScsiMb340),
            "scsi540" => Ok(Self::ScsiMb540),
            "x68sasi10" => Ok(Self::X68kSasiMb10),
            "x68sasi20" => Ok(Self::X68kSasiMb20),
            "x68sasi40" => Ok(Self::X68kSasiMb40),
            "x68scsi20" => Ok(Self::X68kScsiMb20),
            "x68scsi40" => Ok(Self::X68kScsiMb40),
            _ => Err(format!(
                "unknown HDD size '{s}', expected sasi5, sasi10, sasi15, sasi20, sasi30, sasi40, \
                 ide40, ide80, ide120, ide200, ide500, scsi20, scsi40, scsi100, scsi200, scsi340, \
                 scsi540, x68sasi10, x68sasi20, x68sasi40, x68scsi20, or x68scsi40"
            )),
        }
    }
}

impl HddSizeType {
    /// Whether this size denotes an FM Towns raw SCSI image (.h0-.h4) rather
    /// than a PC-98 SASI/IDE header format (.hdi).
    pub fn is_scsi_raw(self) -> bool {
        matches!(
            self,
            Self::ScsiMb20
                | Self::ScsiMb40
                | Self::ScsiMb100
                | Self::ScsiMb200
                | Self::ScsiMb340
                | Self::ScsiMb540
        )
    }

    /// Whether this size denotes an X68000 headerless .hdf image.
    pub fn is_x68k_hdf(self) -> bool {
        matches!(
            self,
            Self::X68kSasiMb10
                | Self::X68kSasiMb20
                | Self::X68kSasiMb40
                | Self::X68kScsiMb20
                | Self::X68kScsiMb40
        )
    }
}

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

    if load_global_config
        && let Some(global_path) = global_config_path()
        && global_path.exists()
    {
        apply_config_file(&mut config, &mut explicit, &global_path)?;
        info!("Loaded global config: {}", global_path.display());
    }

    let mut args = args.into_iter();

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

        let mut value = |flag: &str| -> crate::Result<String> {
            if let Some(v) = inline_value.clone() {
                Ok(v)
            } else {
                next_value(flag, &mut args)
            }
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
                let path = value(&flag)?;
                apply_config_file(&mut config, &mut explicit, Path::new(&path))?;
            }
            "--machine" => {
                let val = value(&flag)?;
                apply_machine_selection(&mut config, &val).map_err(StringError)?;
            }
            "--cpu-mode" => {
                let val = value(&flag)?;
                config.cpu_mode = val.parse::<CpuMode>().map_err(StringError)?;
                explicit.cpu_mode = true;
            }
            "--boot-mode" => {
                let val = value(&flag)?;
                config.boot_mode = Some(val.parse::<BootMode>().map_err(StringError)?);
            }
            "--monitor" => {
                let val = value(&flag)?;
                config.monitor = val.parse::<MonitorTiming>().map_err(StringError)?;
            }
            "--pc88-memory-wait" => {
                let val = value(&flag)?;
                config.pc88_memory_wait = val.parse::<MemoryWaitSwitch>().map_err(StringError)?;
                explicit.pc88_memory_wait = true;
            }
            "--pc88-8mhz-wait" => {
                let val = value(&flag)?;
                config.pc88_8mhz_wait = val.parse::<EightMhzWaitMode>().map_err(StringError)?;
            }
            "--pc98-roms" => config.pc98_roms = Some(PathBuf::from(value(&flag)?)),
            "--bios" => config.bios = true,
            "--debug-bios" => config.debug_bios = Some(PathBuf::from(value(&flag)?)),
            "--pc88-roms" => config.pc88_roms = Some(PathBuf::from(value(&flag)?)),
            "--pc88va-roms" => config.pc88va_roms = Some(PathBuf::from(value(&flag)?)),
            "--towns-roms" => config.towns_roms = Some(PathBuf::from(value(&flag)?)),
            "--x68k-roms" => config.x68k_roms = Some(PathBuf::from(value(&flag)?)),
            "--towns-pad" => config.towns_pad = value(&flag)?.parse().map_err(StringError)?,
            "--pc6000-roms" => config.pc60_roms = Some(PathBuf::from(value(&flag)?)),
            "--x1-roms" => config.x1_roms = Some(PathBuf::from(value(&flag)?)),
            "--fm7-roms" => config.fm7_roms = Some(PathBuf::from(value(&flag)?)),
            "--x1-keyboard" => {
                config.x1_keyboard = value(&flag)?.parse().map_err(StringError)?;
            }
            "--cartridge" => config.cartridge = Some(PathBuf::from(value(&flag)?)),
            "--cassette" => config.cassette = Some(PathBuf::from(value(&flag)?)),
            "--pc6000-phase" => {
                let val = value(&flag)?;
                config.pc60_composite_phase = parse_composite_phase(&val)?;
            }
            "--fdd1" => config.fdd1.push(PathBuf::from(value(&flag)?)),
            "--fdd2" => config.fdd2.push(PathBuf::from(value(&flag)?)),
            "--hdd1" => config.hdd1 = Some(PathBuf::from(value(&flag)?)),
            "--hdd2" => config.hdd2 = Some(PathBuf::from(value(&flag)?)),
            "--cdrom" => config.cdrom.push(PathBuf::from(value(&flag)?)),
            "--cdrom-compat" => {
                let val = value(&flag)?;
                config.cdrom_compat = parse_on_off(&val, &flag)?;
            }
            "--audio-volume" => {
                let val = value(&flag)?;
                config.audio_volume = val
                    .parse::<f32>()
                    .map_err(|e| StringError(format!("invalid audio volume '{val}': {e}")))?;
            }
            "--aspect-mode" => {
                let val = value(&flag)?;
                config.aspect_mode = val.parse::<AspectMode>().map_err(StringError)?;
            }
            "--crt" => {
                let val = value(&flag)?;
                config.crt = parse_on_off(&val, &flag)?;
            }
            "--scaling" => {
                let val = value(&flag)?;
                config.scaling = val.parse::<ScalingMode>().map_err(StringError)?;
            }
            "--window-mode" => {
                let val = value(&flag)?;
                config.window_mode = val.parse::<WindowMode>().map_err(StringError)?;
            }
            "--soundboard" => {
                let val = value(&flag)?;
                config.soundboard = val.parse::<SoundboardType>().map_err(StringError)?;
            }
            "--adpcm-ram" => {
                let val = value(&flag)?;
                config.adpcm_ram = parse_on_off(&val, &flag)?;
            }
            "--ems" => {
                let val = value(&flag)?;
                config.ems = parse_on_off(&val, &flag)?;
            }
            "--xms" => {
                let val = value(&flag)?;
                config.xms = parse_on_off(&val, &flag)?;
            }
            "--backend" => {
                let val = value(&flag)?;
                config.backend = val.parse::<Backend>().map_err(StringError)?;
            }
            "--force-gdc-clock" => {
                let val = value(&flag)?;
                config.force_gdc_clock = Some(val.parse::<ForceGdcClock>().map_err(StringError)?);
            }
            "--graphicboard" => {
                let val = value(&flag)?;
                config.graphicboard = val.parse::<GraphicboardType>().map_err(StringError)?;
            }
            "--printer" => config.printer = Some(PathBuf::from(value(&flag)?)),
            "--mt32-roms" => config.mt32_roms = Some(PathBuf::from(value(&flag)?)),
            "--sc55-roms" => config.sc55_roms = Some(PathBuf::from(value(&flag)?)),
            "--midi" => {
                let val = value(&flag)?;
                config.midi = val.parse::<MidiDevice>().map_err(StringError)?;
            }
            "--boot-device" => {
                let val = value(&flag)?;
                config.boot_device = val.parse::<machine::BootDevice>().map_err(StringError)?;
            }
            "--enable-extractor" => config.enable_extractor = true,
            other => bail!("unknown argument: {other}"),
        }
    }

    apply_derived_defaults(&mut config, explicit);
    validate_paths(&config)?;

    Ok(Action::Run(Box::new(config)))
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

/// Selected machine family. The PC-98 and PC-88 targets are constructed from
/// different crates, so the family is tracked explicitly while the model and
/// per-family settings stay on `EmulatorConfig`.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub enum Target {
    /// PC-9800 series (the `machine` crate).
    #[default]
    Pc98,
    /// PC-8801 series (the `machine88` crate).
    Pc88,
    /// PC-88VA series (the `machine88va` crate).
    Pc88Va,
    /// PC-6000/PC-6600 series (the `machine60` crate).
    Pc60,
    /// FM Towns series (the `machinetowns` crate).
    Towns,
    /// Sharp X1 series (the `machinex1` crate).
    X1,
    /// Fujitsu FM-7 series (the `machinefm7` crate).
    Fm7,
    /// Sharp X68000 series (the `machinex68k` crate).
    X68k,
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
    /// Maps this value to a PC-88 boot mode, erroring when the value belongs to
    /// another machine family.
    pub(crate) fn to_pc88(self) -> Result<machine88::BootMode, String> {
        match self {
            BootMode::Pc88V1S => Ok(machine88::BootMode::V1S),
            BootMode::Pc88V1H => Ok(machine88::BootMode::V1H),
            BootMode::Pc88V2 => Ok(machine88::BootMode::V2),
            BootMode::Pc88N => Ok(machine88::BootMode::N),
            BootMode::Pc88N80 => Ok(machine88::BootMode::N80),
            BootMode::Pc88N80Sr => Ok(machine88::BootMode::N80SR),
            BootMode::Fm7Basic | BootMode::Fm7Dos => Err(format!(
                "boot mode '{self}' is not supported by the PC-8801, expected v1s, v1h, v2, n, n80 or n80sr"
            )),
        }
    }

    /// Maps this value to an FM-7 boot mode, erroring when the value belongs to
    /// another machine family.
    pub(crate) fn to_fm7(self) -> Result<machinefm7::BootMode, String> {
        match self {
            BootMode::Fm7Basic => Ok(machinefm7::BootMode::Basic),
            BootMode::Fm7Dos => Ok(machinefm7::BootMode::Dos),
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
    pub boot_device: machine::BootDevice,
    pub key_map: KeyMap,
    pub ems: bool,
    pub xms: bool,
    pub backend: Backend,
    pub enable_extractor: bool,
    /// Boot mode selected via `--boot-mode`, shared across machine families.
    /// `None` means each machine uses its own default.
    pub boot_mode: Option<BootMode>,
    pub monitor: MonitorTiming,
    pub pc88_memory_wait: MemoryWaitSwitch,
    pub pc88_8mhz_wait: EightMhzWaitMode,
    pub pc88_roms: Option<PathBuf>,
    pub pc88va_model: Pc88VaModel,
    pub pc88va_roms: Option<PathBuf>,
    pub pc60_model: Pc6000Model,
    pub pc60_roms: Option<PathBuf>,
    pub x1_model: X1Model,
    pub x1_roms: Option<PathBuf>,
    pub x1_keyboard: X1KeyboardMode,
    pub fm7_model: Fm7Model,
    pub fm7_roms: Option<PathBuf>,
    pub towns_model: TownsModel,
    pub towns_roms: Option<PathBuf>,
    pub x68k_model: X68kModel,
    pub x68k_roms: Option<PathBuf>,
    pub towns_pad: TownsPadType,
    pub cartridge: Option<PathBuf>,
    pub cassette: Option<PathBuf>,
    /// Initial composite subcarrier phase select (0..3). Swaps the PC-6001
    /// artifact-color pair; also cycled at runtime with Right Ctrl + P.
    pub pc60_composite_phase: u32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct ExplicitSettings {
    cpu_mode: bool,
    pc88_memory_wait: bool,
}

impl Default for EmulatorConfig {
    fn default() -> Self {
        Self {
            target: Target::Pc98,
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
            boot_device: machine::BootDevice::Auto,
            key_map: KeyMap::new(),
            ems: true,
            xms: true,
            backend: Backend::Modern,
            enable_extractor: false,
            boot_mode: None,
            monitor: MonitorTiming::Auto,
            pc88_memory_wait: MemoryWaitSwitch::Fast,
            pc88_8mhz_wait: EightMhzWaitMode::Fast,
            pc88_roms: None,
            pc88va_model: Pc88VaModel::PC88VA2,
            pc88va_roms: None,
            pc60_model: Pc6000Model::Pc6001,
            pc60_roms: None,
            x1_model: X1Model::X1,
            x1_roms: None,
            x1_keyboard: X1KeyboardMode::ModeA,
            fm7_model: Fm7Model::Fm7,
            fm7_roms: None,
            towns_model: TownsModel::FmTownsIIMx,
            towns_roms: None,
            x68k_model: X68kModel::X68000,
            x68k_roms: None,
            towns_pad: TownsPadType::SixButton,
            cartridge: None,
            cassette: None,
            pc60_composite_phase: 0,
        }
    }
}

pub fn parse_config_file(path: &Path) -> crate::Result<EmulatorConfig> {
    let mut config = EmulatorConfig::default();
    let mut explicit = ExplicitSettings::default();
    apply_config_file(&mut config, &mut explicit, path)?;
    apply_derived_defaults(&mut config, explicit);
    Ok(config)
}

fn apply_config_file(
    config: &mut EmulatorConfig,
    explicit: &mut ExplicitSettings,
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
        match key {
            "machine" => {
                if let Err(message) = apply_machine_selection(config, val) {
                    warn!("Unknown machine type in config: {val} ({message})");
                }
            }
            "cpu-mode" => match val.parse::<CpuMode>() {
                Ok(mode) => {
                    config.cpu_mode = mode;
                    explicit.cpu_mode = true;
                }
                Err(_) => warn!("Unknown CPU mode in config: {val}"),
            },
            "boot-mode" => match val.parse::<BootMode>() {
                Ok(mode) => config.boot_mode = Some(mode),
                Err(_) => warn!("Unknown boot mode in config: {val}"),
            },
            "monitor" => match val.parse::<MonitorTiming>() {
                Ok(timing) => config.monitor = timing,
                Err(_) => warn!("Unknown monitor timing in config: {val}"),
            },
            "pc88-memory-wait" => match val.parse::<MemoryWaitSwitch>() {
                Ok(switch) => {
                    config.pc88_memory_wait = switch;
                    explicit.pc88_memory_wait = true;
                }
                Err(_) => warn!("Unknown PC-88 memory wait in config: {val}"),
            },
            "pc88-8mhz-wait" => match val.parse::<EightMhzWaitMode>() {
                Ok(mode) => config.pc88_8mhz_wait = mode,
                Err(_) => warn!("Unknown PC-88 8 MHz wait in config: {val}"),
            },
            "pc98-roms" => config.pc98_roms = Some(PathBuf::from(val)),
            "bios" => match val {
                "on" => config.bios = true,
                "off" => config.bios = false,
                _ => warn!("Invalid bios in config: {val}, expected on or off"),
            },
            "debug-bios" => config.debug_bios = Some(PathBuf::from(val)),
            "pc88-roms" => config.pc88_roms = Some(PathBuf::from(val)),
            "pc88va-roms" => config.pc88va_roms = Some(PathBuf::from(val)),
            "towns-roms" => config.towns_roms = Some(PathBuf::from(val)),
            "x68k-roms" => config.x68k_roms = Some(PathBuf::from(val)),
            "towns-pad" => match val.parse() {
                Ok(pad) => config.towns_pad = pad,
                Err(error) => warn!("Invalid towns-pad in config: {error}"),
            },
            "pc6000-roms" => config.pc60_roms = Some(PathBuf::from(val)),
            "x1-roms" => config.x1_roms = Some(PathBuf::from(val)),
            "fm7-roms" => config.fm7_roms = Some(PathBuf::from(val)),
            "x1-keyboard" => match val.parse() {
                Ok(mode) => config.x1_keyboard = mode,
                Err(error) => warn!("Invalid x1-keyboard in config: {error}"),
            },
            "cartridge" => config.cartridge = Some(PathBuf::from(val)),
            "cassette" => config.cassette = Some(PathBuf::from(val)),
            "pc6000-phase" => match parse_composite_phase(val) {
                Ok(phase) => config.pc60_composite_phase = phase,
                Err(error) => warn!("Invalid PC-6000 composite phase in config: {error}"),
            },
            "fdd1" => config.fdd1.push(PathBuf::from(val)),
            "fdd2" => config.fdd2.push(PathBuf::from(val)),
            "hdd1" => config.hdd1 = Some(PathBuf::from(val)),
            "hdd2" => config.hdd2 = Some(PathBuf::from(val)),
            "cdrom" => config.cdrom.push(PathBuf::from(val)),
            "cdrom-compat" => match val {
                "on" => config.cdrom_compat = true,
                "off" => config.cdrom_compat = false,
                _ => warn!("Invalid cdrom-compat in config: {val}, expected on or off"),
            },
            "aspect-mode" => match val.parse::<AspectMode>() {
                Ok(mode) => config.aspect_mode = mode,
                Err(_) => warn!("Unknown aspect mode in config: {val}"),
            },
            "crt" => match val {
                "on" => config.crt = true,
                "off" => config.crt = false,
                _ => warn!("Invalid crt in config: {val}, expected on or off"),
            },
            "scaling" => match val.parse::<ScalingMode>() {
                Ok(mode) => config.scaling = mode,
                Err(_) => warn!("Unknown scaling in config: {val}"),
            },
            "window-mode" => match val.parse::<WindowMode>() {
                Ok(mode) => config.window_mode = mode,
                Err(_) => warn!("Unknown window mode in config: {val}"),
            },
            "audio-volume" => match val.parse::<f32>() {
                Ok(v) => config.audio_volume = v,
                Err(_) => warn!("Invalid audio-volume in config: {val}"),
            },
            "soundboard" => match val.parse::<SoundboardType>() {
                Ok(sb) => config.soundboard = sb,
                Err(_) => warn!("Unknown soundboard type in config: {val}"),
            },
            "adpcm-ram" => match val {
                "on" => config.adpcm_ram = true,
                "off" => config.adpcm_ram = false,
                _ => warn!("Invalid adpcm-ram in config: {val}, expected on or off"),
            },
            "ems" => match val {
                "on" => config.ems = true,
                "off" => config.ems = false,
                _ => warn!("Invalid ems in config: {val}, expected on or off"),
            },
            "xms" => match val {
                "on" => config.xms = true,
                "off" => config.xms = false,
                _ => warn!("Invalid xms in config: {val}, expected on or off"),
            },
            "backend" => match val.parse::<Backend>() {
                Ok(backend) => config.backend = backend,
                Err(_) => warn!("Invalid backend in config: {val}, expected modern or legacy"),
            },
            "force-gdc-clock" => match val.parse::<ForceGdcClock>() {
                Ok(mode) => config.force_gdc_clock = Some(mode),
                Err(_) => warn!("Invalid force-gdc-clock in config: {val}, expected 2.5 or 5"),
            },
            "graphicboard" => match val.parse::<GraphicboardType>() {
                Ok(gb) => config.graphicboard = gb,
                Err(_) => warn!("Unknown graphicboard type in config: {val}"),
            },
            "printer" => config.printer = Some(PathBuf::from(val)),
            "mt32-roms" => config.mt32_roms = Some(PathBuf::from(val)),
            "sc55-roms" => config.sc55_roms = Some(PathBuf::from(val)),
            "midi" => match val.parse::<MidiDevice>() {
                Ok(device) => config.midi = device,
                Err(_) => warn!("Unknown MIDI device in config: {val}"),
            },
            "boot-device" => match val.parse::<machine::BootDevice>() {
                Ok(device) => config.boot_device = device,
                Err(_) => warn!("Unknown boot device in config: {val}"),
            },
            "enable-extractor" => match val {
                "on" => config.enable_extractor = true,
                "off" => config.enable_extractor = false,
                _ => warn!("Invalid enable-extractor in config: {val}, expected on or off"),
            },
            key if key.starts_with("key.") => {
                let host_name = &key[4..];
                let binding = match config.target {
                    Target::Pc88 => parse_key_binding_pc88(host_name, val),
                    Target::Pc88Va => parse_key_binding_pc88va(host_name, val),
                    Target::Pc98 => parse_key_binding(host_name, val),
                    Target::Pc60 => parse_key_binding_pc60(host_name, val),
                    Target::Towns => parse_key_binding_towns(host_name, val),
                    Target::X68k => parse_key_binding_x68k(host_name, val),
                    // Placeholder until the X1 key map lands in a later phase.
                    Target::X1 => parse_key_binding(host_name, val),
                    // FM-7 reuses the generic parser; its key map yields
                    // physical scancodes directly.
                    Target::Fm7 => parse_key_binding(host_name, val),
                };
                match binding {
                    Some((host, code)) => config.key_map.set(host, code),
                    None => warn!("Invalid key binding: {key}={val}"),
                }
            }
            _ => warn!("Unknown config key: {key}"),
        }
    }

    Ok(())
}

fn apply_derived_defaults(config: &mut EmulatorConfig, explicit: ExplicitSettings) {
    if config.target != Target::Pc88 {
        return;
    }
    // An FM-7-only value here resolves to None and derives from V2; the real
    // error for a mismatched value is raised when the PC-88 machine is built.
    let boot_mode = config
        .boot_mode
        .and_then(|mode| mode.to_pc88().ok())
        .unwrap_or(machine88::BootMode::V2);
    if !(matches!(boot_mode, machine88::BootMode::V1S) || boot_mode.is_n_family()) {
        return;
    }
    if !explicit.cpu_mode {
        config.cpu_mode = CpuMode::Low;
    }
    if !explicit.pc88_memory_wait {
        config.pc88_memory_wait = MemoryWaitSwitch::Compatible;
    }
}

/// Resolves a `--machine` / `machine=` value to a family and model. A PC-88,
/// PC-88VA or PC-6000 model name selects that family's target; anything else is
/// parsed as a PC-98 model. Returns a human-readable error if no family
/// recognises the value.
fn apply_machine_selection(config: &mut EmulatorConfig, value: &str) -> Result<(), String> {
    if let Ok(_model) = value.parse::<Pc8801Model>() {
        // Switch the default key map to the PC-88 matrix so later `key.*`
        // overrides layer onto the right base.
        if config.target != Target::Pc88 {
            config.key_map = KeyMap::new_pc88();
        }
        config.target = Target::Pc88;
        return Ok(());
    }
    if let Ok(model) = value.parse::<machine88va::Pc88VaModel>() {
        if config.target != Target::Pc88Va {
            config.key_map = KeyMap::new_pc88va();
        }
        config.target = Target::Pc88Va;
        config.pc88va_model = model;
        return Ok(());
    }
    if let Ok(model) = value.parse::<Pc6000Model>() {
        if config.target != Target::Pc60 {
            config.key_map = KeyMap::new_pc60();
        }
        config.target = Target::Pc60;
        config.pc60_model = model;
        return Ok(());
    }
    if let Ok(model) = value.parse::<X1Model>() {
        if config.target != Target::X1 {
            config.key_map = KeyMap::new_x1();
        }
        config.target = Target::X1;
        config.x1_model = model;
        return Ok(());
    }
    if let Ok(model) = value.parse::<Fm7Model>() {
        if config.target != Target::Fm7 {
            config.key_map = KeyMap::new_fm7();
        }
        config.target = Target::Fm7;
        config.fm7_model = model;
        return Ok(());
    }
    if let Ok(model) = value.parse::<machinetowns::TownsModel>() {
        if config.target != Target::Towns {
            config.key_map = KeyMap::new_towns();
        }
        config.target = Target::Towns;
        config.towns_model = model;
        return Ok(());
    }
    if let Ok(model) = value.parse::<X68kModel>() {
        if config.target != Target::X68k {
            config.key_map = KeyMap::new_x68k();
        }
        config.target = Target::X68k;
        config.x68k_model = model;
        return Ok(());
    }
    let Ok(model) = value.parse::<MachineModel>() else {
        return Err(format!(
            "unknown machine type '{value}', expected PC9801F, PC9801VM, PC9801VX, PC9801RS, \
             PC9801RA, PC9821AS, PC9821AP, PC8801MC, PC88VA2, PC6001, PC6001MK2, PC6601, PC6001MK2SR, \
             PC6601SR, FMTowns, FMTownsIICX, FMTownsIIMX, X68000, X68000SUPER, X68000XVI, X1, \
             X1TURBO, FM7 or FM77AV"
        ));
    };
    if config.target != Target::Pc98 {
        config.key_map = KeyMap::new();
    }
    config.target = Target::Pc98;
    config.machine = model;
    Ok(())
}

fn global_config_path() -> Option<PathBuf> {
    let pref_path = sdl3::filesystem::get_pref_path(crate::COMPANY_NAME, crate::GAME_NAME)?;
    Some(pref_path.join("global.conf"))
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
    use super::*;

    fn parse_run_config(args: &[&str]) -> EmulatorConfig {
        match parse_args_from(args.iter().map(|arg| (*arg).to_owned()), false)
            .expect("arguments should parse")
        {
            Action::Run(config) => *config,
            _ => panic!("expected run action"),
        }
    }

    #[test]
    fn machine_flag_selects_the_pc88_target() {
        let mut config = EmulatorConfig::default();
        apply_machine_selection(&mut config, "PC8801MC").expect("PC8801MC is valid");
        assert_eq!(config.target, Target::Pc88);
    }

    #[test]
    fn machine_flag_selects_the_pc60_target() {
        let mut config = EmulatorConfig::default();
        apply_machine_selection(&mut config, "PC6001MK2SR").expect("PC6001MK2SR is valid");
        assert_eq!(config.target, Target::Pc60);
        assert_eq!(config.pc60_model, Pc6000Model::Pc6001Mk2Sr);
    }

    #[test]
    fn machine_flag_selects_the_pc98_target() {
        let mut config = EmulatorConfig::default();
        apply_machine_selection(&mut config, "PC9801VX").expect("PC9801VX is valid");
        assert_eq!(config.target, Target::Pc98);
        assert_eq!(config.machine, MachineModel::PC9801VX);
    }

    #[test]
    fn machine_flag_selects_x68000_and_its_rom_directory() {
        let config = parse_run_config(&["--machine", "x68000xvi", "--x68k-roms", "roms/x68kxvi"]);
        assert_eq!(config.target, Target::X68k);
        assert_eq!(config.x68k_model, X68kModel::X68000Xvi);
        assert_eq!(config.x68k_roms, Some(PathBuf::from("roms/x68kxvi")));
        assert_eq!(config.key_map.lookup(sdl3::keyboard::Scancode::F1), 0x63);
    }

    #[test]
    fn unknown_machine_is_rejected() {
        let mut config = EmulatorConfig::default();
        assert!(apply_machine_selection(&mut config, "FOOBAR").is_err());
    }

    #[test]
    fn fdd_type_parses_2d() {
        assert_eq!("2d".parse::<FddType>(), Ok(FddType::D2));
        assert_eq!("2hd".parse::<FddType>(), Ok(FddType::Hd2));
        assert_eq!("2dd".parse::<FddType>(), Ok(FddType::Dd2));
        assert!("2xx".parse::<FddType>().is_err());
    }

    #[test]
    fn pc88_n_family_boot_modes_derive_compatible_defaults() {
        for boot_mode in ["n", "n80", "n80sr"] {
            let config = parse_run_config(&["--machine", "PC8801MC", "--boot-mode", boot_mode]);
            assert_eq!(config.cpu_mode, CpuMode::Low, "{boot_mode}");
            assert_eq!(
                config.pc88_memory_wait,
                MemoryWaitSwitch::Compatible,
                "{boot_mode}"
            );
        }
    }

    #[test]
    fn pc88_v1s_boot_mode_derives_compatible_defaults() {
        let config = parse_run_config(&["--machine", "PC8801MC", "--boot-mode", "v1s"]);
        assert_eq!(config.cpu_mode, CpuMode::Low);
        assert_eq!(config.pc88_memory_wait, MemoryWaitSwitch::Compatible);
    }

    #[test]
    fn pc88_v2_boot_mode_keeps_fast_defaults() {
        let config = parse_run_config(&["--machine", "PC8801MC", "--boot-mode", "v2"]);
        assert_eq!(config.cpu_mode, CpuMode::High);
        assert_eq!(config.pc88_memory_wait, MemoryWaitSwitch::Fast);
    }

    #[test]
    fn explicit_pc88_cpu_mode_overrides_boot_mode_default() {
        let config = parse_run_config(&[
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

    #[test]
    fn explicit_pc88_memory_wait_overrides_boot_mode_default() {
        let config = parse_run_config(&[
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

    #[test]
    fn config_file_explicit_pc88_values_override_boot_mode_default() {
        let path = std::env::temp_dir().join(format!(
            "neetan_config_test_{}_pc88_explicit.conf",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "machine=PC8801MC\nboot-mode=n\ncpu-mode=high\npc88-memory-wait=fast\n",
        )
        .expect("config file should be written");

        let config = parse_run_config(&["--config", path.to_str().expect("path is UTF-8")]);
        let _ = std::fs::remove_file(path);

        assert_eq!(config.cpu_mode, CpuMode::High);
        assert_eq!(config.pc88_memory_wait, MemoryWaitSwitch::Fast);
    }

    #[test]
    fn cdrom_compat_flag_parses_on_and_off() {
        assert!(!parse_run_config(&[]).cdrom_compat);
        assert!(parse_run_config(&["--cdrom-compat", "on"]).cdrom_compat);
        assert!(!parse_run_config(&["--cdrom-compat", "off"]).cdrom_compat);
    }

    #[test]
    fn config_file_cdrom_compat_parses_on() {
        let path = std::env::temp_dir().join(format!(
            "neetan_config_test_{}_cdrom_compat.conf",
            std::process::id()
        ));
        std::fs::write(&path, "machine=FMTownsIICX\ncdrom-compat=on\n")
            .expect("config file should be written");

        let config = parse_run_config(&["--config", path.to_str().expect("path is UTF-8")]);
        let _ = std::fs::remove_file(path);

        assert!(config.cdrom_compat);
    }
}
