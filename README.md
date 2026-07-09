# Neetan (ねーたん)

An emulator for the PC-6001, PC-6601, PC-8001, PC-8801, PC-88VA, PC-9801, PC-9821,
FM Towns, Sharp X1 and Fujitsu FM-7 written in Rust.

## Documentation

Each machine family has its own detailed guide:

* [PC-9801 / PC-9821](doc/machine-pc98.md)
* [PC-8001 / PC-8801](doc/machine-pc88.md)
* [PC-88VA2](doc/machine-pc88va.md)
* [PC-6001 / PC-6601](doc/machine-pc6000.md)
* [FM Towns](doc/machine-towns.md)
* [Sharp X1](doc/machine-x1.md)
* [Fujitsu FM-7 / FM-77AV](doc/machine-fm7.md)

The [ROMs](doc/roms.md) page is the unified reference for every ROM file the
emulator can load.

## Game compatibility

Verified per-title compatibility lists.

* [PC-98 game compatibility](doc/games-pc98.md)
* [PC-88 VA game compatibility](doc/games-88va.md)
* [FM-7 game compatibility](doc/games-fm7.md)

Updates to the compatibility list is always greatly welcome!

## Supported systems

Neetan emulates seven distinct families, selected through the `--machine` option. See
each family's guide for the detailed target list, sound options, ROM requirements,
and platform-specific flags.

| Family                                     | Targets                                                         | Firmware                 |
|--------------------------------------------|-----------------------------------------------------------------|--------------------------|
| [PC-9801 / PC-9821](doc/machine-pc98.md)   | PC-9801F, PC-9801VM, PC-9801VX, PC-9801RA, PC-9821AS, PC-9821AP | HLE BIOS (ROMs optional) |
| [PC-8001 / PC-8801](doc/machine-pc88.md)   | PC-8801MC (plus PC-8001 personalities via `--boot-mode`)        | Real ROM set required    |
| [PC-88VA2](doc/machine-pc88va.md)          | PC-88VA2                                                        | Real ROM set required    |
| [PC-6001 / PC-6601](doc/machine-pc6000.md) | PC-6001, PC-6001mkII, PC-6601, PC-6001mkIISR, PC-6601SR         | Real ROM set required    |
| [FM Towns](doc/machine-towns.md)           | FM Towns II CX, FM Towns II MX                                  | Real ROM set required    |
| [Sharp X1](doc/machine-x1.md)              | X1, X1 turbo                                                    | Real ROM set required    |
| [Fujitsu FM-7](doc/machine-fm7.md)         | FM-7, FM-77AV                                                   | Real ROM set required    |

The default machine is `PC9801RA`. Games of the PC-98 normally do not require any ROM
files. The other families need a real ROM set. See [ROMs](doc/roms.md) for details.

## Usage

```bash
neetan [OPTIONS]
neetan <COMMAND>
```

### Options

The `System` column shows where an option applies: `All` (every family), `PC-98`
(PC-9801 / PC-9821 only), `PC-9821` (PC-9821 only), `PC-88` (PC-8001 / PC-8801
only), `PC-88VA` (PC-88VA only), `PC-6000` (PC-6001 / PC-6601 only), `FM Towns`
(FM Towns only), `X1` (Sharp X1 only), or `FM-7` (FM-7 / FM-77AV only). Options
that apply to one family are ignored on the others.

| Option                       | System            | Description                                                                                      | Default           |
|------------------------------|-------------------|--------------------------------------------------------------------------------------------------|-------------------|
| `-c, --config <PATH>`        | All               | Load configuration from file                                                                     | -                 |
| `--machine <TYPE>`           | All               | Machine type (see the list of values below the table)                                            | `PC9801RA`        |
| `--cpu-mode <MODE>`          | All               | CPU speed mode: `low` or `high` (PC-88 default derives from the boot mode)                       | `high` (PC-98)    |
| `--boot-mode <MODE>`         | PC-88, FM-7       | Boot mode; PC-88: `v1s`, `v1h`, `v2`, `n`, `n80`, `n80sr`; FM-7: `basic`, `dos`                  | `v2` / `basic`    |
| `--monitor <MODE>`           | PC-88, X1         | Monitor timing: `auto`, `15k`, `24k`                                                             | `auto`            |
| `--pc88-memory-wait <MODE>`  | PC-88             | Memory wait: `fast` or `compatible`                                                              | derives from mode |
| `--pc88-8mhz-wait <MODE>`    | PC-88             | 8 MHz wait: `fast` or `compatible`                                                               | `fast`            |
| `--pc88-roms <PATH>`         | PC-88             | Directory with the PC-8801MC ROM set (required for `PC8801MC`)                                   | -                 |
| `--pc88va-roms <PATH>`       | PC-88VA           | Directory with the PC-88VA2 ROM set (required for `PC88VA2`)                                     | -                 |
| `--pc6000-roms <PATH>`       | PC-6000           | Directory with the PC-6000 ROM set (required for the PC-6000 targets)                            | -                 |
| `--pc6000-phase <0-3>`       | PC-6000           | Initial composite artifact-color phase; swaps the fake-color pair Mode 4 titles rely on.         | `0`               |
| `--towns-roms <PATH>`        | FM Towns          | Directory with the FM Towns ROM set (required for the FM Towns targets)                          | -                 |
| `--towns-pad <2\|6>`         | FM Towns          | FM Towns game pad type: `2` (2-button) or `6` (6-button)                                         | `6`               |
| `--x1-roms <PATH>`           | X1                | Directory with the Sharp X1 ROM set (required for the X1 targets)                                | -                 |
| `--x1-keyboard <A\|B>`       | X1                | X1 turbo keyboard mode switch: `A` (standard) or `B` (game-key matrix, mode-B kana layout)       | `A`               |
| `--fm7-roms <PATH>`          | FM-7              | Directory with the FM-7 / FM-77AV ROM set (required for the FM-7 targets)                        | -                 |
| `--fdd1 <PATH>`              | All               | Floppy disk image for drive 1 (repeatable)                                                       | -                 |
| `--fdd2 <PATH>`              | All               | Floppy disk image for drive 2 (repeatable)                                                       | -                 |
| `--hdd1 <PATH>`              | All               | Hard disk image for hard disk drive 1                                                            | -                 |
| `--hdd2 <PATH>`              | All               | Hard disk image for hard disk drive 2                                                            | -                 |
| `--cdrom <PATH>`             | PC-9821, FM Towns | CD-ROM disc image .cue or .ccd file (repeatable)                                                 | -                 |
| `--cdrom-compat <on\|off>`   | FM Towns          | Slow/compatible CD-ROM drive timing                                                              | `off`             |
| `--cartridge <PATH>`         | PC-6000           | Cartridge ROM image to insert                                                                    | -                 |
| `--cassette <PATH>`          | PC-6000, X1, FM-7 | Cassette tape image to insert (`.cas`, `.p6`, `.p6t`; X1 `.tap`; FM-7 `.t77`)                    | -                 |
| `--audio-volume <FLOAT>`     | All               | Audio volume 0.0-1.0                                                                             | `1.0`             |
| `--aspect-mode <MODE>`       | All               | Display aspect mode: `4:3` or `1:1`                                                              | `4:3`             |
| `--crt <on\|off>`            | All               | Enable the CRT effect. Not available when using the legacy backend.                              | `on`              |
| `--scaling <MODE>`           | All               | Scaling method: `nearest`, `bilinear`, `pixelart`                                                | `pixelart`        |
| `--backend <BACKEND>`        | All               | Rendering backend: `modern` or `legacy`                                                          | `modern`          |
| `--window-mode <MODE>`       | All               | Window mode: `windowed` or `fullscreen`                                                          | `windowed`        |
| `--force-gdc-clock <2.5\|5>` | PC-98             | Force GDC clock to 2.5 or 5 MHz. VX and later only                                               | auto              |
| `--graphicboard <TYPE>`      | PC-98             | Graphics accelerator board: `none`, `ga1280a`                                                    | `none`            |
| `--pc98-roms <PATH>`         | PC-98             | Directory with the PC-98 ROM set (BIOS, font, sound), matched by content hash. All ROMs optional | -                 |
| `--bios`                     | PC-98             | Boot the real BIOS from `--pc98-roms` instead of the HLE BIOS. Ignored (warns) on PC-9821        | HLE BIOS          |
| `--soundboard <TYPE>`        | PC-98             | Sound board: `none`, `14`, `26k`, `86`, `86+26k`, `sb16`, `sb16+26k`                             | `86+26k`          |
| `--adpcm-ram <on\|off>`      | PC-98             | ADPCM RAM option for the PC-9801-86                                                              | `on`              |
| `--ems <on\|off>`            | PC-98             | Enable EMS expanded memory                                                                       | `on`              |
| `--xms <on\|off>`            | PC-98             | Enable XMS extended memory                                                                       | `on`              |
| `--midi <DEVICE>`            | PC-98, FM Towns   | MIDI device: `none`, `mt32`, `sc55`                                                              | `none`            |
| `--mt32-roms <PATH>`         | PC-98, FM Towns   | Path to MT-32 ROM directory (requires `mt32` feature)                                            | -                 |
| `--sc55-roms <PATH>`         | PC-98, FM Towns   | Path to SC-55 ROM directory (requires `sc55` feature)                                            | -                 |
| `--boot-device <DEVICE>`     | All               | Boot device: `auto`, `fdd1`, `fdd2`, `hdd1`, `hdd2`, `dos`                                       | `auto`            |
| `--printer <PATH>`           | All               | Output file for printer (must exist)                                                             | -                 |
| `--enable-extractor`         | PC-98             | Copy on-screen Japanese text to the system clipboard, one line at a time                         | off               |
| `-h, --help`                 | All               | Print help                                                                                       | -                 |
| `-V, --version`              | All               | Print version                                                                                    | -                 |

The `--machine <TYPE>` values are: `PC9801F`, `PC9801VM`, `PC9801VX`, `PC9801RA`,
`PC9821AS`, `PC9821AP`, `PC8801MC`, `PC88VA2`, `PC6001`, `PC6001MK2`, `PC6601`,
`PC6001MK2SR`, `PC6601SR`, `FMTownsIICX`, `FMTownsIIMX`, `X1`, `X1TURBO`, `FM7`
and `FM77AV`. The default is `PC9801RA`.

### Commands

`create-fdd <PATH> [OPTIONS]` - Create an empty floppy disk image (D88 format). The
path must have a `.d88` extension.

| Option          | Description                                                          | Default |
|-----------------|----------------------------------------------------------------------|---------|
| `--type <TYPE>` | `2hd` (1232 KB), `2hd144` (1440 KB), `2dd` (640 KB) or `2d` (320 KB) | `2hd`   |

`create-hdd <PATH> --type <TYPE>` - Create an empty hard disk image. Use a `.hdi`
path for SASI/IDE images and a `.h0`-`.h4` path for raw SCSI images. `--type` is
required.

| Interface | `--type` values                                                     |
|-----------|---------------------------------------------------------------------|
| SASI      | `sasi5`, `sasi10`, `sasi15`, `sasi20`, `sasi30`, `sasi40`           |
| IDE       | `ide40`, `ide80`, `ide120`, `ide200`, `ide500`                      |
| SCSI      | `scsi20`, `scsi40`, `scsi100`, `scsi200`, `scsi340`, `scsi540`      |

`convert-hdd <INPUT> <OUTPUT>` - Convert a hard disk image between SASI and IDE
formats. The input may be an HDI, NHD, or THD image; the output must be `.hdi`.

The conversion direction is auto-detected from the input image's sector size (256
bytes = SASI, 512 bytes = IDE). The smallest compatible target geometry is chosen
automatically. Output is always in HDI format.

SASI to IDE conversion always succeeds (all SASI sizes fit within ide40). IDE to SASI
conversion will fail if the IDE image exceeds the largest SASI capacity (sasi40 at
~40 MB).

`copy <SOURCE> <DEST>` - Copy files and directories between the host filesystem and
FAT-formatted disk images. Each argument is either a host path or an `IMAGE:DOSPATH`
reference (e.g. `roms/dos620.hdi:A:\PROGS\FILE.EXE`). At least one side must be an
image. Image-to-image copies are supported.

```bash
neetan copy ./readme.txt roms/disk.hdi:A:\README.TXT
neetan copy roms/disk.hdi:A:\PROGS\FOO.EXE ./extracted/
neetan copy src.hdi:A:\FOO.EXE dst.hdi:A:\FOO.EXE
```

Directories are copied recursively (there is no `-r` flag). DOS paths must use 8.3
ASCII filenames. Longer names are rejected before any file is written. Recognized
image extensions are `hdi`, `nhd`, `thd` (hard disks) and `d88`, `d98`, `88d`, `98d`,
`hdm`, `nfd`, `2d` (floppies).

## Configuration file

Instead of passing all options on the command line, you can use a configuration file
with `-c` or `--config`:

```bash
neetan --config my_game.cfg
```

The file uses a simple `key = value` format. Lines starting with `#` or `;` are
comments. See [`configuration/default.conf`](configuration/default.conf) for a
complete reference with all options and their defaults.

```ini
# Example configuration
machine = PC9801RA
soundboard = 86+26k
force-gdc-clock = 2.5
audio-volume = 0.8
aspect-mode = 4:3
crt = on
scaling = pixelart
fdd1 = /path/to/disk_a.d88
fdd1 = /path/to/disk_b.d88
fdd2 = /path/to/save_game.d88
hdd1 = /path/to/harddrive.hdi
cdrom = /path/to/game.cue
midi = mt32
mt32-roms = /path/to/mt32_roms
```

Command-line arguments override values from the configuration file.

### Global configuration

neetan automatically loads a global configuration file from the OS data directory if
it exists. This is useful for setting persistent defaults like your preferred machine
type, sound card, or keyboard mapping without needing to pass `--config` or CLI flags
every time.

The global config file uses the same `key = value` format as regular configuration
files.

#### File location

| OS      | Path                                                         |
|---------|--------------------------------------------------------------|
| Linux   | `~/.local/share/neetan/neetan/global.conf`                   |
| Windows | `C:\Users\<user>\AppData\Roaming\neetan\neetan\global.conf`  |
| macOS   | `~/Library/Application Support/neetan/neetan/global.conf`    |

The directory is created automatically. The configuration file must be created
manually.

#### Layering order

Settings are applied in this order, with later layers overriding earlier ones:

1. Built-in defaults
2. Global configuration file (`global.conf` in OS data directory)
3. Per-invocation configuration file (`--config`)
4. Command-line arguments

For example, if your global config sets `machine = PC9801RA` and you run
`neetan --config game.cfg --soundboard sb16`, the machine will be PC9801RA (from
global config) unless `game.cfg` or CLI args override it.

## Emulator controls

| Key                | Action                           |
|--------------------|----------------------------------|
| Right Ctrl + M     | Toggle mouse capture             |
| Right Ctrl + Q     | Quit the emulator                |
| Right Ctrl + Enter | Toggle fullscreen                |
| Right Ctrl + R     | Hard reset                       |
| Right Ctrl + F     | Fast forward 8x (hold)           |
| Right Ctrl + C     | Toggle CRT effect                |
| Right Ctrl + S     | Cycle scaling method             |
| Right Ctrl + P     | Cycle composite phase (PC-6000)  |
| Right Ctrl + 1     | Open floppy selector for drive 1 |
| Right Ctrl + 2     | Open floppy selector for drive 2 |
| Right Ctrl + 3     | Open CD-ROM selector             |

Right Ctrl is reserved as the emulator's shortcut modifier. The emulated machine uses
Left Ctrl.

### How do I rebind my keys?

You can remap keys in the configuration file using `key.<HostKey> = <PC-98 Key>`
entries. See [`configuration/default.conf`](configuration/default.conf) for a
complete reference of all available host key names, PC-98 key names, and the default
mappings.

## Disk and disc images

### Supported floppy disk image formats

| Format  | Extensions                     | Writable | Description                                        |
|---------|--------------------------------|----------|----------------------------------------------------|
| D88     | `.d88`, `.d98`, `.88d`, `.98d` | Yes      | Standard PC-98 disk image with per-sector metadata |
| HDM     | `.hdm`                         | Yes      | Headerless raw sector image (2HD only)             |
| NFD     | `.nfd`                         | Partial  | T98Next format with per-sector metadata            |
| 2D      | `.2d`                          | Yes      | Headerless raw sector image (Sharp X1, 2D only)    |
| D77     | `.d77`                         | Yes      | Fujitsu FM-7 disk image; byte-compatible D88       |

Sector writes made by the emulated software (e.g. game saves) are persisted back to the
source file for all formats. Full track reformatting (`FORMAT TRACK`) is re-emitted for
D88, HDM, and 2D, but not for NFD, whose full-image serialization cannot preserve all
per-sector metadata.

### Supported CD-ROM disc image formats

CD-ROM discs apply to the PC-8801 and PC-9821 targets.

| Format  | Extensions | Description                                                                                          |
|---------|------------|------------------------------------------------------------------------------------------------------|
| CUE/BIN | `.cue`     | CUE sheet referencing a raw BIN image                                                                |
| CloneCD | `.ccd`     | CloneCD control file with sibling `.img` (raw 2352-byte sectors) and optional `.sub` subchannel data |

### Multiple disk images

Many games ship on multiple floppy disks and ask you to swap disks during gameplay.
Some CD-ROM games also come as multiple disc images. neetan handles this by letting
you assign several disk images to each drive up front, then swap between them at
runtime.

Use the `--fdd1` / `--fdd2` / `--cdrom` flags more than once to register all images
for a drive:

```bash
neetan --fdd1 floppy_disk1.d88 --fdd1 floppy_disk2.d88 --fdd1 floppy_disk3.d88
neetan --cdrom disc1.cue --cdrom disc2.cue
```

Or equivalently in a configuration file:

```ini
fdd1 = floppy_disk1.d88
fdd1 = floppy_disk2.d88
fdd1 = floppy_disk3.d88
cdrom = disc1.cue
cdrom = disc2.cue
```

The first image in each list is automatically inserted at startup.

Press `Right Ctrl + 1` (drive 1), `Right Ctrl + 2` (drive 2), or `Right Ctrl + 3`
(CD-ROM) to open the image selector and swap disks at runtime.

## FAQ

### How can I use my mouse?

In games that support a mouse, you first need to capture the mouse pointer via
`Right Ctrl + M`. You can release the mouse pointer by pressing `Right Ctrl + M`
again.

### 日本語も分かりますか？

もちろん！IssueやPRの作成には日本語をご利用いただけますが、ソースコードのコメントについては英語での記述を推奨しております。

## Build requirements

* [Rust 1.95](https://rustup.rs/)
* [SDL3](https://github.com/libsdl-org/SDL) (See [sdl3_sys description](https://docs.rs/sdl3-sys/latest/sdl3_sys/#usage))

Build a release binary with:

```bash
cargo build --release
```

The resulting binary is `target/release/neetan`. Run it with any of the options
documented under [Usage](#usage).

### Optional MIDI features

The Roland MT-32 and SC-55 emulations are optional build features, both enabled by
default. Distributions that cannot comply with their licenses (see
[License](#license)) can disable one or both at build time:

```bash
cargo build --release --no-default-features                       # neither
cargo build --release --no-default-features --features mt32       # MT-32 only
cargo build --release --no-default-features --features sc55       # SC-55 only
```

When built without a feature, the corresponding `--midi` option is still accepted but
the emulator will print a warning and continue without audio for that module.

## Acknowledgement

Following projects provided references for our implementation and test vectors. They
were invaluable for developing neetan:

- [MAME](https://www.mamedev.org/)
- [MartyPC](https://github.com/dbalsom/martypc)
- [NP21W](https://simk98.github.io/np21w/)
- [Tsugaru](https://github.com/captainys/TOWNSEMU)
- [Common Source Code Project](https://takeda-toshiya.my.coocan.jp/common/index.html)
- [SingleStepTests](https://github.com/SingleStepTests)
- [undoc98](https://www.webtech.co.jp/company/doc/undocumented_mem/index.html)

We ported the Yamaha OPN and OPL emulation from the amazing YMFM project to our own
Rust port:

- [ymfm](https://github.com/aaronsgiles/ymfm)

We ported the Roland SC-55 emulator from the incredible Nuked SC55 project to our own
Rust port:

- [Nuked-SC55](https://github.com/nukeykt/Nuked-SC55)

We ported the Roland MT-32 emulator from the outstanding munt project to our own Rust
port:

- [munt](https://github.com/munt/munt)

## Rules for AI coding assistants

AI agents MUST NOT add Signed-off-by tags. Only humans can legally certify the origin
of the source code. AI agents are only tools and do not absolve the human submitter
of their responsibility:

* Reviewing all AI-generated code
* Ensuring compliance with licensing requirements
* Taking full responsibility for the contribution

An `Assisted-by` tag MUST NOT be added, since that information is irrelevant to the
contribution of the human contributor.

## License

This project is licensed under the [3-clause BSD](https://opensource.org/license/bsd-3-clause) license.

When optional features are enabled, the license terms of the resulting binary change:

| Build configuration             | Binary license                |
|---------------------------------|-------------------------------|
| Default (no optional features)  | BSD 3-Clause                  |
| `sc55` feature enabled          | BSD 3-Clause + non-commercial |
| `mt32` feature enabled          | LGPL 2.1                      |
| `sc55` + `mt32` enabled         | LGPL 2.1 + non-commercial     |

The `sc55` feature links the Nuked-SC55 port, which is licensed under the original
MAME license (non-commercial use only). The `mt32` feature links the munt port, which
is licensed under [LGPL 2.1](https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html).

The source code of the BSD 3-Clause licensed components remains available under BSD
3-Clause regardless of the build configuration.
