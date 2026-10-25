# Neetan (ねーたん)

An emulator for the PC-9821, PC-9801, PC-88VA2, PC-8801, PC-8001, PC-6601, PC-6001,
MSX, Fujitsu FM Towns, Fujitsu FM-7, Sharp X68000, Sharp X1 and the IBM PC/AT
(DOS/V) written in Rust.

## Documentation

Each machine family has its own detailed guide:

* [NEC PC-9801 / PC-9821](doc/machine-pc98.md)
* [NEC PC-88VA2](doc/machine-pc88va.md)
* [NEC PC-8001 / PC-8801](doc/machine-pc88.md)
* [NEC PC-6001 / PC-6601](doc/machine-pc6000.md)
* [MSX / MSX2 / MSX2+](doc/machine-msx.md)
* [Fujitsu FM Towns](doc/machine-towns.md)
* [Fujitsu FM-7 / FM-77AV](doc/machine-fm7.md)
* [Sharp X68000](doc/machine-x68k.md)
* [Sharp X1](doc/machine-x1.md)
* [IBM PC/AT (DOS/V)](doc/machine-at.md)

The [ROMs](doc/roms.md) page is the unified reference for every ROM file the
emulator can load.

## Game compatibility

Verified per-title compatibility lists.

* [PC-98 game compatibility](doc/games-pc98.md)
* [PC-88 VA game compatibility](doc/games-88va.md)
* [MSX game compatibility](doc/games-msx.md)
* [FM Towns game compatibility](doc/games-towns.md)
* [FM-7 game compatibility](doc/games-fm7.md)
* [X68000 game compatibility](doc/games-x68k.md)
* [PC/AT (DOS/V)](doc/games-at.md)

Updates to the compatibility list is always greatly welcome!

## Supported systems

Neetan emulates ten distinct families, selected through the `--machine` option. See
each family's guide for the detailed target list, sound options, ROM requirements,
and platform-specific flags.

| Family                                         | Targets                                                                    | Firmware                 |
|------------------------------------------------|----------------------------------------------------------------------------|--------------------------|
| [NEC PC-9801 / PC-9821](doc/machine-pc98.md)   | PC-9801F, PC-9801VM, PC-9801VX, PC-9801RS, PC-9801RA, PC-9821AS, PC-9821AP | HLE BIOS (ROMs optional) |
| [NEC PC-88VA2](doc/machine-pc88va.md)          | PC-88VA2                                                                   | Real ROM set required    |
| [NEC PC-8001 / PC-8801](doc/machine-pc88.md)   | PC-8801MC (plus PC-8001 personalities via `--boot-mode`)                   | Real ROM set required    |
| [NEC PC-6001 / PC-6601](doc/machine-pc6000.md) | PC-6001, PC-6001mkII, PC-6601, PC-6001mkIISR, PC-6601SR                    | Real ROM set required    |
| [MSX / MSX2 / MSX2+](doc/machine-msx.md)       | MSX, MSX2, MSX2+                                                           | Real ROM sets required   |
| [Fujitsu FM Towns](doc/machine-towns.md)       | FM Towns, FM Towns II CX, FM Towns II MX                                   | Real ROM set required    |
| [Sharp X68000](doc/machine-x68k.md)            | X68000, X68000 SUPER, X68000 XVI                                           | Real ROM set required    |
| [Sharp X1](doc/machine-x1.md)                  | X1, X1 turbo                                                               | Real ROM set required    |
| [Fujitsu FM-7](doc/machine-fm7.md)             | FM-7, FM-77AV                                                              | Real ROM set required    |
| [IBM PC/AT (DOS/V)](doc/machine-at.md)         | AT 486DX2-50, AT 486DX2-66                                                 | Real ROM set required    |

Each system has a default model, for example `PC9801RA` for the PC-98. Games of the
PC-98 normally do not require any ROM files. The other families need a real ROM set.
See [ROMs](doc/roms.md) for details.

## Usage

```bash
neetan [OPTIONS]
neetan <COMMAND>
```

### Startup

`neetan` without arguments opens the game manager. A `--machine` option or a
`--config` file starts the game directly. A media option such as `--fdd1`, `--cdrom`
or `--cartridge` needs `--machine` or `--config`. Host options such as `--settings`,
`--roms-folder` or `--window-mode` alone open the manager with these settings. A
`machine` key in the settings file does not start a game.

```bash
neetan                                        # opens the game manager
neetan --settings ~/neetan/neetan.conf        # opens the game manager of that root
neetan --config config/pc98/albatross.conf    # starts the game directly
neetan --machine PC9801RA --fdd1 disk.d88     # starts the machine directly
```

### Options

The `System` column shows where an option applies: `All` (every family), `PC-98`
(PC-9801 / PC-9821 only), `PC-9821` (PC-9821 only), `PC-88` (PC-8001 / PC-8801
only), `PC-88VA` (PC-88VA only), `PC-6000` (PC-6001 / PC-6601 only), `MSX`
(MSX / MSX2 / MSX2+ only), `FM Towns` (FM Towns only), `X68000` (Sharp X68000
only), `X1` (Sharp X1 only), `FM-7` (FM-7 / FM-77AV only), or `PC/AT` (IBM
PC/AT DOS/V only). Options that apply to one family are ignored on the others.

An option takes its value as `--name value` or `--name=value`. An on/off option
without a value turns the setting on, so `--bios` equals `--bios on`. Every option
except `--config`, `--settings`, `--help` and `--version` has a configuration file
key of the same name. A relative path in an option resolves against the current
directory.

| Option                         | System                                                            | Description                                                                                      | Default                      |
|--------------------------------|-------------------------------------------------------------------|--------------------------------------------------------------------------------------------------|------------------------------|
| `-c, --config <PATH>`          | All                                                               | Load a game file (repeatable)                                                                    | -                            |
| `--settings <PATH>`            | All                                                               | Load this settings file. Its folder is the root                                                  | discovered                   |
| `--config-folder <PATH>`       | All                                                               | Folder of the system defaults and the game files                                                 | `config`                     |
| `--roms-folder <PATH>`         | All                                                               | Folder of the ROM folders of every system                                                        | `roms`                       |
| `--gui-font <PATH>`            | All                                                               | Font file of the user interface                                                                  | `font/SarasaUiJ-Regular.ttf` |
| `--gui-theme <THEME>`          | All                                                               | Colors of the user interface: `dark` or `light`                                                  | `dark`                       |
| `--gui-language <LANGUAGE>`    | All                                                               | Language of the user interface: `auto` or a language tag such as `ja`                            | `auto`                       |
| `--machine <TYPE>`             | All                                                               | Machine type (see the list of values below the table). Starts the game directly                  | per system                   |
| `--cpu-mode <MODE>`            | PC-98, PC-88, FM Towns, X68000, PC/AT                             | CPU speed mode: `low` or `high`                                                                  | `high`                       |
| `--boot-mode <MODE>`           | PC-88, FM-7                                                       | Boot mode; PC-88: `v1s`, `v1h`, `v2`, `n`, `n80`, `n80sr`; FM-7: `basic`, `dos`                  | `v2` / `basic`               |
| `--monitor <MODE>`             | PC-88, X1                                                         | Monitor timing: `auto`, `15k`, `24k`                                                             | `auto`                       |
| `--pc88-memory-wait <MODE>`    | PC-88                                                             | Memory wait: `fast` or `compatible`                                                              | derives from mode            |
| `--pc88-8mhz-wait <MODE>`      | PC-88                                                             | 8 MHz wait: `fast` or `compatible`                                                               | `fast`                       |
| `--pc88-roms <PATH>`           | PC-88                                                             | Directory with the PC-8801MC ROM set (required for `PC8801MC`)                                   | `roms/pc88`                  |
| `--pc88va-roms <PATH>`         | PC-88VA                                                           | Directory with the PC-88VA2 ROM set (required for `PC88VA2`)                                     | `roms/pc88va`                |
| `--pc6000-roms <PATH>`         | PC-6000                                                           | Directory with the PC-6000 ROM set (required for the PC-6000 targets)                            | `roms/pc6000`                |
| `--pc6000-phase <0-3>`         | PC-6000                                                           | Initial composite artifact-color phase; swaps the fake-color pair Mode 4 titles rely on.         | `0`                          |
| `--msx-roms <PATH>`            | MSX                                                               | Directory with the MSX machine and FS-CA1 ROM sets                                               | `roms/msx`                   |
| `--towns-roms <PATH>`          | FM Towns                                                          | Directory with the FM Towns ROM set (required for the FM Towns targets)                          | `roms/towns`                 |
| `--towns-pad <2\|6>`           | FM Towns                                                          | FM Towns game pad type: `2` (2-button) or `6` (6-button)                                         | `6`                          |
| `--x68k-roms <PATH>`           | X68000                                                            | Directory with the selected X68000 model ROM set                                                 | `roms/x68k`                  |
| `--x1-roms <PATH>`             | X1                                                                | Directory with the Sharp X1 ROM set (required for the X1 targets)                                | `roms/x1`                    |
| `--x1-keyboard <A\|B>`         | X1                                                                | X1 turbo keyboard mode switch: `A` (standard) or `B` (game-key matrix, mode-B kana layout)       | `A`                          |
| `--fm7-roms <PATH>`            | FM-7                                                              | Directory with the FM-7 / FM-77AV ROM set (required for the FM-7 targets)                        | `roms/fm7`                   |
| `--at-roms <PATH>`             | PC/AT                                                             | Directory with the PC/AT ROM set, matched by content hash. All ROMs optional                     | `roms/at`                    |
| `--fdd1 <PATH>`                | All                                                               | Floppy disk image for drive 1 (repeatable)                                                       | -                            |
| `--fdd2 <PATH>`                | PC-98, PC-88, PC-88VA, PC-6000, FM Towns, X68000, X1, FM-7, PC/AT | Floppy disk image for drive 2 (repeatable)                                                       | -                            |
| `--hdd1 <PATH>`                | PC-98, FM Towns, X68000, PC/AT                                    | Hard disk image for hard disk drive 1                                                            | -                            |
| `--hdd2 <PATH>`                | PC-98, FM Towns, X68000, PC/AT                                    | Hard disk image for hard disk drive 2                                                            | -                            |
| `--cdrom <PATH>`               | PC-9821, PC-88, PC/AT, FM Towns, X68000                           | CD-ROM disc image .cue or .ccd file (repeatable)                                                 | -                            |
| `--cdrom-compat [on\|off]`     | FM Towns                                                          | Slow/compatible CD-ROM drive timing                                                              | `off`                        |
| `--cartridge <PATH>`           | PC-6000, MSX                                                      | One cartridge ROM image to insert; MSX mapper selection is automatic                             | -                            |
| `--cassette <PATH>`            | PC-6000, MSX, X1, FM-7                                            | Cassette tape image (`MSX .cas`; PC-6000 `.cas`/`.p6`/`.p6t`; X1 `.tap`; FM-7 `.t77`)            | -                            |
| `--audio-volume <FLOAT>`       | All                                                               | Audio volume 0.0-1.0                                                                             | `1.0`                        |
| `--aspect-mode <MODE>`         | All                                                               | Display aspect mode: `4:3` or `1:1`                                                              | `4:3`                        |
| `--crt [on\|off]`              | All                                                               | Enable the CRT effect. Not available when using the legacy backend.                              | `on`                         |
| `--scaling <MODE>`             | All                                                               | Scaling method: `nearest`, `bilinear`, `pixelart`                                                | `pixelart`                   |
| `--backend <BACKEND>`          | All                                                               | Rendering backend: `modern` or `legacy`                                                          | `modern`                     |
| `--window-mode <MODE>`         | All                                                               | Window mode: `windowed` or `fullscreen`                                                          | `windowed`                   |
| `--force-gdc-clock <2.5\|5>`   | PC-98                                                             | Force GDC clock to 2.5 or 5 MHz. VX and later only                                               | auto                         |
| `--graphicboard <TYPE>`        | PC-98                                                             | Graphics accelerator board: `none`, `ga1280a`                                                    | `none`                       |
| `--pc98-roms <PATH>`           | PC-98                                                             | Directory with the PC-98 ROM set (BIOS, font, sound), matched by content hash. All ROMs optional | `roms/pc98`                  |
| `--bios [on\|off]`             | PC-98, PC/AT                                                      | Boot the real BIOS from `--pc98-roms` / `--at-roms` instead of the HLE BIOS. Ignored on PC-9821  | HLE BIOS                     |
| `--soundboard <TYPE>`          | PC-98                                                             | Sound board: `none`, `14`, `26k`, `86`, `86+26k`, `sb16`, `sb16+26k`                             | `86+26k`                     |
| `--adpcm-ram [on\|off]`        | PC-98                                                             | ADPCM RAM option for the PC-9801-86                                                              | `on`                         |
| `--ems [on\|off]`              | PC-98                                                             | Enable EMS expanded memory                                                                       | `on`                         |
| `--xms [on\|off]`              | PC-98                                                             | Enable XMS extended memory                                                                       | `on`                         |
| `--midi <DEVICE>`              | PC-98, PC/AT, FM Towns, X68000                                    | MIDI device: `none`, `mt32`, `sc55`                                                              | `none`                       |
| `--mt32-roms <PATH>`           | PC-98, PC/AT, FM Towns, X68000                                    | Path to MT-32 ROM directory (requires `mt32` feature)                                            | `roms/mt32`                  |
| `--sc55-roms <PATH>`           | PC-98, PC/AT, FM Towns, X68000                                    | Path to SC-55 ROM directory (requires `sc55` feature)                                            | `roms/sc55`                  |
| `--boot-device <DEVICE>`       | PC-98, FM Towns, PC/AT                                            | Boot device: `auto`, `fdd1`, `fdd2`, `hdd1`, `hdd2`, `dos`                                       | `auto`                       |
| `--printer <PATH>`             | PC-98                                                             | Output file for printer (must exist)                                                             | -                            |
| `--enable-extractor [on\|off]` | PC-98                                                             | Copy on-screen Japanese text to the system clipboard, one line at a time                         | off                          |
| `--debug-bios <PATH>`          | PC-98                                                             | PC-98 BIOS ROM image for hardware test programs. Excludes `--bios`                               | -                            |
| `-h, --help`                   | All                                                               | Print help                                                                                       | -                            |
| `-V, --version`                | All                                                               | Print version                                                                                    | -                            |

The `--machine <TYPE>` values are: `PC9801F`, `PC9801VM`, `PC9801VX`, `PC9801RS`,
`PC9801RA`, `PC9821AS`, `PC9821AP`, `PC8801MC`, `PC88VA2`, `PC6001`, `PC6001MK2`, `PC6601`,
`PC6001MK2SR`, `PC6601SR`, `MSX`, `MSX2`, `MSX2PLUS`, `FM7`, `FM77AV`, `FMTowns`,
`FMTownsIICX`, `FMTownsIIMX`, `X68000`, `X68000SUPER`, `X68000XVI`, `X1`, `X1TURBO`,
`AT486DX50`, `AT486DX66`. A game file without a `machine` key in a system folder of the
config folder runs the `machine` of `<config-folder>/<system>.conf`, else a `machine` of
the same system in the settings file, else the default model of the system. For a
`--config` file outside the system folders, the `machine` of the settings file selects
the system. The `machine` of `<config-folder>/<system>.conf` then wins over it.

On the PC/AT, the real AMI BIOS exposes only `A: then C:` or `C: then A:`.
`auto` and `fdd1` select the first order, while `hdd1` selects the second.
Requests for `fdd2`, `hdd2`, or `dos` fall back to the closest supported order
with a warning.

### Commands

`create-fdd <PATH> [OPTIONS]` - Create an empty floppy disk image. A `.d88` path
produces a D88 image; a `.xdf` or `.2hd` path produces a headerless raw XDF image
(X68000, `2hd` type only).

| Option          | Description                                                          | Default |
|-----------------|----------------------------------------------------------------------|---------|
| `--type <TYPE>` | `2hd` (1232 KB), `2hd144` (1440 KB), `2dd` (640 KB) or `2d` (320 KB) | `2hd`   |

`create-hdd <PATH> --type <TYPE>` - Create an empty hard disk image. Use a `.hdi`
path for SASI/IDE images, a `.h0`-`.h4` path for raw SCSI images, a `.hdf` path
for the headerless X68000 types, and a `.hdd` path for the PC/AT flat images.
`--type` is required.

| Interface   | `--type` values                                                |
|-------------|----------------------------------------------------------------|
| SASI        | `sasi5`, `sasi10`, `sasi15`, `sasi20`, `sasi30`, `sasi40`      |
| IDE         | `ide40`, `ide80`, `ide120`, `ide200`, `ide500`                 |
| SCSI        | `scsi20`, `scsi40`, `scsi100`, `scsi200`, `scsi340`, `scsi540` |
| X68000 SASI | `x68sasi10`, `x68sasi20`, `x68sasi40`                          |
| X68000 SCSI | `x68scsi20`, `x68scsi40`                                       |
| PC/AT IDE   | `at40`, `at100`, `at250`, `at504`                              |

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
image extensions are `hdi`, `nhd`, `thd`, `hdd` (hard disks) and `d88`, `d98`, `88d`,
`98d`, `hdm`, `nfd`, `2d`, `img`, `ima`, `dsk` (floppies).

## Configuration files & portable setup

Neetan stores its settings in plain `key = value` files. Lines starting with `#` or
`;` are comments. See [`configuration/default.conf`](configuration/default.conf) for
a complete reference with all keys and their defaults.

### Folder layout

The folder that holds the settings file `neetan.conf` is the root. The settings file of
the SDL pref path uses the current directory as the root:

```
<root>/
  neetan.conf              ; host settings: config-folder, roms-folder, backend, audio-volume, ...
  config/
    pc98.conf              ; PC-98 system defaults
    pc98/
      albatross.conf       ; one file for each game
    pc88.conf
    pc88/
    ...
  roms/
    at/ fm7/ msx/ mt32/ pc6000/ pc88/ pc88va/ pc98/ sc55/ towns/ x1/ x68k/
```

The system names are `at`, `fm7`, `msx`, `pc6000`, `pc88`, `pc88va`, `pc98`, `towns`,
`x1` and `x68k`. `config-folder` selects the config folder and defaults to `config`.
`roms-folder` selects the ROM folder and defaults to `roms`. Every ROM key defaults to
the folder of its system below the ROM folder, for example `pc98-roms` to
`roms/pc98`. The derived folders of the PC-98, the PC/AT, the MT-32 and the SC-55
apply only when they exist. See [ROMs](doc/roms.md) for the ROM sets of each folder.
`gui-font` selects the font file of the user interface. Its default
`font/SarasaUiJ-Regular.ttf` is searched below the root and then in the resource folder
of the installation: the folder of the executable, `../share/neetan` on Linux and
`../Resources` on macOS.

A relative path inside a configuration file resolves against the root. A relative
path given as a command line option resolves against the current directory.

### Settings file

The settings file is the first match of:

1. `--settings <PATH>` on the command line. The file must exist.
2. `neetan.conf` in the current directory.
3. `neetan.conf` beside the executable, for a portable installation.
4. `neetan.conf` in the SDL pref path.

| OS      | SDL pref path                                    |
|---------|--------------------------------------------------|
| Linux   | `~/.local/share/neetan/neetan/`                  |
| Windows | `C:\Users\<user>\AppData\Roaming\neetan\neetan\` |
| macOS   | `~/Library/Application Support/neetan/neetan/`   |

Without a settings file the root is the current directory and every setting keeps its
built-in default. The log shows the settings file and the root at startup.

### Layers

A launch builds its configuration from these layers. A later layer overrides an
earlier one:

1. The built-in defaults.
2. The settings file.
3. The system defaults `<config-folder>/<system>.conf`.
4. The game files of `--config`, in command line order.
5. The command line options.

The `machine` of the latest layer that sets it selects the system, and with it the
system defaults file. A missing system defaults file is fine. A `machine` key of
another system inside a system defaults file is ignored with a warning.

The first `fdd1`, `fdd2` or `cdrom` value of a layer replaces the images of the
earlier layers. Further values in the same layer append images. An empty value such
as `fdd1 =` drops the images of the earlier layers. An empty single image key such as
`hdd1 =` drops the image of the earlier layers. An empty ROM key falls back to its
derived folder.

The host keys `config-folder`, `roms-folder`, `gui-font`, the ROM keys, `backend`,
`gui-theme`, `gui-language` and `audio-volume` apply only from the settings file and the
command line.
A system defaults file or a game file that sets one gets a warning and the line is
ignored.
A media key such as `fdd1` or `hdd1` in a system defaults file applies to every game
of the system that does not set the key.

```ini
# A game file, for example config/pc98/albatross.conf
machine = PC9801RA
soundboard = 86+26k
force-gdc-clock = 2.5
fdd1 = games/98/disk_a.d88
fdd1 = games/98/disk_b.d88
fdd2 = games/98/save_game.d88
hdd1 = games/98/harddrive.hdi
midi = mt32
```

```bash
neetan --config config/pc98/albatross.conf --soundboard sb16
```

This command runs the PC9801RA of the game file with the sb16 sound board of the
option. The images resolve against the root.

## Emulator controls

| Key                | Action                                                       |
|--------------------|--------------------------------------------------------------|
| Right Ctrl + M     | Toggle mouse capture                                         |
| Right Ctrl + Q     | Quit the emulator. A game of the manager returns to it       |
| Right Ctrl + Esc   | Return to the game manager. Only in a game of the manager    |
| Right Ctrl + Enter | Toggle fullscreen                                            |
| Right Ctrl + R     | Hard reset                                                   |
| Right Ctrl + F     | Fast forward 8x (hold)                                       |
| Right Ctrl + C     | Toggle CRT effect                                            |
| Right Ctrl + S     | Cycle scaling method                                         |
| Right Ctrl + P     | Cycle composite phase (PC-6000)                              |
| Right Ctrl + 1     | Open floppy selector for drive 1                             |
| Right Ctrl + 2     | Open floppy selector for drive 2                             |
| Right Ctrl + 3     | Open CD-ROM selector                                         |
| Right Ctrl + 5     | Capture quick save                                           |
| Right Ctrl + 9     | Restore quick save                                           |

Right Ctrl is reserved as the emulator's shortcut modifier. The emulated machine uses
Left Ctrl.

The quick save is one runtime snapshot kept only in memory.
Right Ctrl + 5 replaces the previous snapshot and Right Ctrl + 9 restores it.
Quick load switches removable drives back to the floppy, CD-ROM and cassette images
mounted when the snapshot was captured. Guest writes to writable media remain persistent.
The snapshot is not written to disk and is lost when Neetan exits. Save normally inside
the emulated software because quick saves do not replace disk or application saves.

## Automation

`neetan-auto` is a deterministic, headless frontend for writing automated scripts
and tests for the emulator as R7RS-small Scheme programs. It builds a machine,
drives it at exact frame boundaries, injects input, reads the screen, and
reports one pass or fail result.

The [Automation API reference](doc/api.md) documents how to run `neetan-auto`,
explains it's general usage and has an API reference.

### How do I rebind my keys?

You can remap keys in the configuration file using `key.<HostKey> = <PC-98 Key>`
entries. See [`configuration/default.conf`](configuration/default.conf) for a
complete reference of all available host key names, PC-98 key names, and the default
mappings.

## Disk and disc images

### Supported floppy disk image formats

| Format  | Extensions                     | Description                                        |
|---------|--------------------------------|----------------------------------------------------|
| D88     | `.d88`, `.d98`, `.88d`, `.98d` | Standard PC-98 disk image with per-sector metadata |
| HDM     | `.hdm`                         | Headerless raw sector image (2HD only)             |
| NFD     | `.nfd`                         | T98Next format with per-sector metadata            |
| 2D      | `.2d`                          | Headerless raw sector image (Sharp X1, 2D only)    |
| D77     | `.d77`                         | Fujitsu FM-7 disk image; byte-compatible D88       |
| MSX DSK | `.dsk`                         | Headerless raw MSX floppy image                    |
| DIM     | `.dim`                         | X68000 DIFC.X container                            |
| XDF     | `.xdf`, `.2hd`                 | Headerless raw sector image (X68000, 2HD only)     |
| IMG     | `.img`, `.ima`                 | Headerless raw IBM PC floppy image                 |
| IBM XDF | `.xdf`, `.img`, `.ima`         | IBM Extended Density Format, detected by file size |

Writes made by the emulated software (e.g. game saves) are persisted back to the source file for
all formats.

### Supported CD-ROM disc image formats

CD-ROM discs apply to the PC-9821, PC-8801, PC/AT, FM Towns, and X68000 SUPER/XVI targets.

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

* [Rust 1.97](https://rustup.rs/)
* [SDL3](https://github.com/libsdl-org/SDL) (See [sdl3_sys description](https://docs.rs/sdl3-sys/latest/sdl3_sys/#usage))
* Linux: [The mold linker]([https://github.com/wild-linker/wild](https://github.com/rui314/mold))

Build a release binary with:

```bash
cargo build --release
```

The resulting binary is `target/release/neetan`. Run it with any of the options
documented under [Usage](#usage).

### Build features

Every machine family and both frontends are selectable at build time. All of them are
on by default, so a plain `cargo build` produces the full emulator.

| Feature        | Builds                                              |
|----------------|-----------------------------------------------------|
| `gui`          | The `neetan` binary and its SDL3 fronted            |
| `auto`         | The `neetan-auto` binary and its Scheme interpreter |
| `pc98`         | NEC PC-9801 / PC-9821                               |
| `pc88`         | NEC PC-8001 / PC-8801                               |
| `pc88va`       | NEC PC-88VA2                                        |
| `pc60`         | NEC PC-6001 / PC-6601                               |
| `msx`          | MSX / MSX2 / MSX2+                                  |
| `towns`        | Fujitsu FM Towns                                    |
| `fm7`          | Fujitsu FM-7 / FM-77AV                              |
| `x68k`         | Sharp X68000                                        |
| `x1`           | Sharp X1                                            |
| `at`           | IBM PC/AT (DOS/V)                                   |
| `all-machines` | All ten machine families                            |

Examples:

```bash
cargo build --no-default-features --features gui,msx        # MSX only, GUI only
cargo build --no-default-features --features auto,pc98      # PC-98 only, headless only
```

### Optional MIDI features

The Roland MT-32 and SC-55 emulations are optional build features, both enabled by
default. Both use copyleft licenses (see [License](#license)). A build that must stay
BSD 3-Clause licensed can disable one or both:

```bash
# neither
cargo build --release --no-default-features --features gui,auto,all-machines
# MT-32 only
cargo build --release --no-default-features --features gui,auto,all-machines,mt32
# SC-55 only
cargo build --release --no-default-features --features gui,auto,all-machines,sc55
```

When built without a feature, the corresponding `--midi` option is still accepted but
the emulator will print a warning and continue without audio for that module.

## Acknowledgement

Following projects provided references for our implementation and test vectors. They
were invaluable for developing neetan:

- [Common Source Code Project](https://takeda-toshiya.my.coocan.jp/common/index.html)
- [MAME](https://www.mamedev.org/)
- [MartyPC](https://github.com/dbalsom/martypc)
- [NP21W](https://simk98.github.io/np21w/)
- [OpenMSX](https://openmsx.org/)
- [Tsugaru](https://github.com/captainys/TOWNSEMU)
- [XEiJ](https://stdkmd.net/xeij/)
- [SingleStepTests](https://github.com/SingleStepTests)
- [undoc98](https://www.webtech.co.jp/company/doc/undocumented_mem/index.html)

We ported the Yamaha OPM, OPN and OPL emulation from the amazing YMFM project to our own
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

| Build configuration             | Binary license   |
|---------------------------------|------------------|
| Default (no optional features)  | BSD 3-Clause     |
| `mt32` feature enabled          | LGPL 2.1         |
| `sc55` feature enabled          | GPL 2.0 or later |
| `sc55` + `mt32` enabled         | GPL 2.0 or later |

The `sc55` feature links the Nuked-SC55 port, which is licensed under
[GPL 2.0 or later](https://www.gnu.org/licenses/old-licenses/gpl-2.0.html). The `mt32`
feature links the munt port, which is licensed under
[LGPL 2.1](https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html).

The source code of the BSD 3-Clause licensed components remains available under BSD
3-Clause regardless of the build configuration.

The binary embeds third-party components; their license notices are collected in the
[`licenses/`](licenses/) folder, which is included in every release.
