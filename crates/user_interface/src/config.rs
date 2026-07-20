use std::path::{Path, PathBuf};

use common::{Context, CpuMode, MonitorTiming, StringError, bail, info, warn};
use machine_88::{EightMhzWaitMode, MemoryWaitSwitch};
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
  create-hdd <PATH>             Create an empty hard disk image (HDI, raw SCSI, X68000 HDF or PC/AT flat .hdd)
  convert-hdd <INPUT> <OUTPUT>  Convert HDD image between SASI and IDE
  copy <SOURCE> <DEST>          Copy files between host and FAT disk images

Options:
  -c, --config <PATH>           Load configuration from file
      --machine <TYPE>          Machine type: PC9801F, PC9801VM, PC9801VX, PC9801RS, PC9801RA, PC9821AS, PC9821AP, PC8801MC, PC88VA2, PC6001, PC6001MK2, PC6601, PC6001MK2SR, PC6601SR, MSX, MSX2, MSX2PLUS, FMTowns, FMTownsIICX, FMTownsIIMX, X68000, X68000SUPER, X68000XVI, X1, X1TURBO, FM7, FM77AV, AT486DX50, AT486DX66
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
      --msx-roms <PATH>         Directory with the MSX ROM set (required)
      --x1-roms <PATH>          Directory with the Sharp X1 ROM set (required)
      --x1-keyboard <A|B>       X1 turbo keyboard mode switch (default: A)
      --fm7-roms <PATH>         Directory with the FM-7 / FM-77AV ROM set (required)
      --towns-roms <PATH>       Directory with the FM Towns ROM set (required)
      --x68k-roms <PATH>        Directory with the X68000 ROM set (required)
      --at-roms <PATH>          Directory with the PC/AT (ct486) ROM set (required)
      --towns-pad <2|6>         FM Towns game pad type (default 6-button)
      --pc6000-phase <0-3>      Initial composite artifact-color phase; cycle with Right Ctrl + P (PC-6000 only)
      --fdd1 <PATH>             Floppy disk image for drive 1 (repeatable)
      --fdd2 <PATH>             Floppy disk image for drive 2 (repeatable)
      --hdd1 <PATH>             Hard disk image for drive 1 (PC-98 .hdi/.nhd/.thd, FM Towns .h0-.h4, X68000 .hdf, PC/AT .hdd)
      --hdd2 <PATH>             Hard disk image for drive 2 (PC-98 .hdi/.nhd/.thd, FM Towns .h0-.h4, X68000 .hdf, PC/AT .hdd)
      --cdrom <PATH>            CD-ROM disc image .cue or .ccd file (repeatable, PC-9821, PC/AT, FM Towns and X68000 SUPER/XVI)
      --cdrom-compat <on|off>   Slow/compatible CD-ROM drive timing (default: off; FM Towns only)
      --cartridge <PATH>        Cartridge ROM image to insert
      --cassette <PATH>         Cassette image (MSX .cas, PC-6000 .cas/.p6/.p6t, X1 .tap, FM-7 .t77)
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
    AtMb40,
    AtMb100,
    AtMb250,
    AtMb504,
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
            "at40" => Ok(Self::AtMb40),
            "at100" => Ok(Self::AtMb100),
            "at250" => Ok(Self::AtMb250),
            "at504" => Ok(Self::AtMb504),
            _ => Err(format!(
                "unknown HDD size '{s}', expected sasi5, sasi10, sasi15, sasi20, sasi30, sasi40, \
                 ide40, ide80, ide120, ide200, ide500, scsi20, scsi40, scsi100, scsi200, scsi340, \
                 scsi540, x68sasi10, x68sasi20, x68sasi40, x68scsi20, x68scsi40, at40, at100, \
                 at250, or at504"
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

    /// Whether this size denotes an AT headerless flat .hdd image.
    pub fn is_at_flat(self) -> bool {
        matches!(
            self,
            Self::AtMb40 | Self::AtMb100 | Self::AtMb250 | Self::AtMb504
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
    let mut key_map = KeyOverrides::new();

    if load_global_config
        && let Some(global_path) = global_config_path()
        && global_path.exists()
    {
        apply_config_file(&mut config, &mut explicit, &mut key_map, &global_path)?;
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
                apply_config_file(&mut config, &mut explicit, &mut key_map, Path::new(&path))?;
            }
            "--machine" => {
                let val = value(&flag)?;
                apply_machine_selection(&mut config, &mut key_map, &val).map_err(StringError)?;
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
            "--at-roms" => config.at_roms = Some(PathBuf::from(value(&flag)?)),
            "--towns-pad" => config.towns_pad = value(&flag)?.parse().map_err(StringError)?,
            "--pc6000-roms" => config.pc60_roms = Some(PathBuf::from(value(&flag)?)),
            "--msx-roms" => config.msx_roms = Some(PathBuf::from(value(&flag)?)),
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
                config.boot_device = val.parse::<machine_98::BootDevice>().map_err(StringError)?;
            }
            "--enable-extractor" => config.enable_extractor = true,
            other => bail!("unknown argument: {other}"),
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

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct ExplicitSettings {
    cpu_mode: bool,
    pc88_memory_wait: bool,
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
        match key {
            "machine" => {
                if let Err(message) = apply_machine_selection(config, key_map, val) {
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
            "at-roms" => config.at_roms = Some(PathBuf::from(val)),
            "towns-pad" => match val.parse() {
                Ok(pad) => config.towns_pad = pad,
                Err(error) => warn!("Invalid towns-pad in config: {error}"),
            },
            "pc6000-roms" => config.pc60_roms = Some(PathBuf::from(val)),
            "msx-roms" => config.msx_roms = Some(PathBuf::from(val)),
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
            "boot-device" => match val.parse::<machine_98::BootDevice>() {
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
                match parse_key_binding(config.target, host_name, val) {
                    Some((host, code)) => key_map.set(host, code),
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
        .unwrap_or(machine_88::BootMode::V2);
    if !(matches!(boot_mode, machine_88::BootMode::V1S) || boot_mode.is_n_family()) {
        return;
    }
    if !explicit.cpu_mode {
        config.cpu_mode = CpuMode::Low;
    }
    if !explicit.pc88_memory_wait {
        config.pc88_memory_wait = MemoryWaitSwitch::Compatible;
    }
}

/// Resolves a `--machine` / `machine=` value to a family and model, then clears
/// the native key overrides when the family changed so later `key.*` overrides
/// are parsed against the new target.
fn apply_machine_selection(
    config: &mut EmulatorConfig,
    key_map: &mut KeyOverrides,
    value: &str,
) -> Result<(), String> {
    let previous = config.target;
    machine_factory::config::resolve_model(config, value)?;
    if config.target != previous {
        // Native override codes are target specific, so a family change clears
        // any bindings parsed against the previous target.
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
    use common::MachineModel;
    use machine_60::Pc6000Model;
    use machine_msx::MsxModel;
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

    #[test]
    fn machine_flag_selects_the_pc88_target() {
        let mut config = EmulatorConfig::default();
        let mut key_map = KeyOverrides::new();
        apply_machine_selection(&mut config, &mut key_map, "PC8801MC").expect("PC8801MC is valid");
        assert_eq!(config.target, Target::Pc88);
    }

    #[test]
    fn machine_flag_selects_the_pc60_target() {
        let mut config = EmulatorConfig::default();
        let mut key_map = KeyOverrides::new();
        apply_machine_selection(&mut config, &mut key_map, "PC6001MK2SR")
            .expect("PC6001MK2SR is valid");
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

    #[test]
    fn machine_flag_selects_msx_and_its_rom_directory() {
        let (config, key_map) = parse_run_config(&["--machine", "msx", "--msx-roms", "roms/msx"]);
        assert_eq!(config.target, Target::Msx);
        assert_eq!(config.msx_model, MsxModel::Msx);
        assert_eq!(config.msx_roms, Some(PathBuf::from("roms/msx")));
        // The host-to-native key table now lives in each machine crate's
        // `translate_host_key`; the keyboard.rs cross-check test guards it.
        let _ = key_map;
    }

    #[test]
    fn machine_flag_exposes_msx2_and_msx2_plus() {
        let mut config = EmulatorConfig::default();
        let mut key_map = KeyOverrides::new();
        apply_machine_selection(&mut config, &mut key_map, "MSX2").expect("MSX2 is valid");
        assert_eq!(config.target, Target::Msx);
        assert_eq!(config.msx_model, MsxModel::Msx2);

        apply_machine_selection(&mut config, &mut key_map, "MSX2PLUS").expect("MSX2PLUS is valid");
        assert_eq!(config.target, Target::Msx);
        assert_eq!(config.msx_model, MsxModel::Msx2Plus);
    }

    #[test]
    fn machine_flag_selects_the_pc98_target() {
        let mut config = EmulatorConfig::default();
        let mut key_map = KeyOverrides::new();
        apply_machine_selection(&mut config, &mut key_map, "PC9801VX").expect("PC9801VX is valid");
        assert_eq!(config.target, Target::Pc98);
        assert_eq!(config.machine, MachineModel::PC9801VX);
    }

    #[test]
    fn machine_flag_selects_x68000_and_its_rom_directory() {
        let (config, key_map) =
            parse_run_config(&["--machine", "x68000xvi", "--x68k-roms", "roms/x68kxvi"]);
        assert_eq!(config.target, Target::X68k);
        assert_eq!(config.x68k_model, X68kModel::X68000Xvi);
        assert_eq!(config.x68k_roms, Some(PathBuf::from("roms/x68kxvi")));
        let _ = key_map;
    }

    #[test]
    fn config_file_applies_named_pc_at_key_binding() {
        let path = std::env::temp_dir().join(format!(
            "neetan_config_test_{}_at_key.conf",
            std::process::id()
        ));
        std::fs::write(&path, "machine=AT486DX66\nkey.B=A\n")
            .expect("config file should be written");

        let (config, key_map) =
            parse_run_config(&["--config", path.to_str().expect("path is UTF-8")]);
        let _ = std::fs::remove_file(path);

        assert_eq!(config.target, Target::At);
        assert_eq!(key_map.get(sdl3::keyboard::Scancode::B), Some(0x1E));
    }

    #[test]
    fn unknown_machine_is_rejected() {
        let mut config = EmulatorConfig::default();
        let mut key_map = KeyOverrides::new();
        assert!(apply_machine_selection(&mut config, &mut key_map, "FOOBAR").is_err());
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

    #[test]
    fn pc88_v1s_boot_mode_derives_compatible_defaults() {
        let (config, _key_map) = parse_run_config(&["--machine", "PC8801MC", "--boot-mode", "v1s"]);
        assert_eq!(config.cpu_mode, CpuMode::Low);
        assert_eq!(config.pc88_memory_wait, MemoryWaitSwitch::Compatible);
    }

    #[test]
    fn pc88_v2_boot_mode_keeps_fast_defaults() {
        let (config, _key_map) = parse_run_config(&["--machine", "PC8801MC", "--boot-mode", "v2"]);
        assert_eq!(config.cpu_mode, CpuMode::High);
        assert_eq!(config.pc88_memory_wait, MemoryWaitSwitch::Fast);
    }

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
        let path = std::env::temp_dir().join(format!(
            "neetan_config_test_{}_cdrom_compat.conf",
            std::process::id()
        ));
        std::fs::write(&path, "machine=FMTownsIICX\ncdrom-compat=on\n")
            .expect("config file should be written");

        let (config, _key_map) =
            parse_run_config(&["--config", path.to_str().expect("path is UTF-8")]);
        let _ = std::fs::remove_file(path);

        assert!(config.cdrom_compat);
    }
}
