# Command line

```bash
neetan [OPTIONS]
neetan <COMMAND>
```

## Startup

`neetan` without arguments opens the game manager. A `--machine` option or a
`--config` file starts the game directly. A media option such as `--fdd1`, `--cdrom`
or `--cartridge` needs `--machine` or `--config`. Host options such as `--settings`,
`--roms-folder` or `--window-mode` alone open the manager with these settings.

```bash
neetan                                        # opens the game manager
neetan --settings ~/neetan/neetan.conf        # opens the game manager of that root
neetan --config config/pc98/albatross.conf    # starts a game directly
neetan --machine PC9801RA --fdd1 disk.d88     # starts a machine directly
```

## Option syntax

An option takes its value as `--name value` or `--name=value`. An on/off option
without a value turns the setting on. `--bios` equals `--bios on`. Every option
except `--config`, `--settings`, `--help` and `--version` has a configuration file
key of the same name. A relative path in an option resolves against the current
directory. See [Configuration](configuration.md) for the configuration files.

## Shared options

These options apply to every family or to several families. The `System` column
names the families that read an option. The other families ignore it. Options of a
single family are listed on the page of that family:

* [NEC PC-9801 / PC-9821](machine-pc98.md#platform-options)
* [NEC PC-88VA2](machine-pc88va.md#platform-options)
* [NEC PC-8001 / PC-8801](machine-pc88.md#platform-options)
* [NEC PC-6001 / PC-6601](machine-pc6000.md#platform-options)
* [MSX / MSX2 / MSX2+](machine-msx.md#platform-options)
* [Fujitsu FM Towns](machine-towns.md#platform-options)
* [Fujitsu FM-7 / FM-77AV](machine-fm7.md#platform-options)
* [Sharp X68000](machine-x68k.md#platform-options)
* [Sharp X1](machine-x1.md#platform-options)
* [IBM PC/AT (DOS/V)](machine-at.md#platform-options)

| Option                      | System                                                            | Description                                                                     | Default                      |
|-----------------------------|-------------------------------------------------------------------|---------------------------------------------------------------------------------|------------------------------|
| `-c, --config <PATH>`       | All                                                               | Load a game file (repeatable)                                                   | -                            |
| `--settings <PATH>`         | All                                                               | Load this settings file. Its folder is the root                                 | discovered                   |
| `--config-folder <PATH>`    | All                                                               | Folder of the system defaults and the game files                                | `config`                     |
| `--roms-folder <PATH>`      | All                                                               | Folder of the ROM folders of every system                                       | `roms`                       |
| `--gui-font <PATH>`         | All                                                               | Font file of the user interface                                                 | `font/SarasaUiJ-Regular.ttf` |
| `--gui-theme <THEME>`       | All                                                               | Colors of the user interface: `dark` or `light`                                 | `dark`                       |
| `--gui-language <LANGUAGE>` | All                                                               | Language of the user interface: `auto` or a language tag such as `ja`           | `auto`                       |
| `--machine <TYPE>`          | All                                                               | Machine type (see [Machine types](#machine-types)). Starts the game directly    | per system                   |
| `--cpu-mode <MODE>`         | PC-98, PC-88, FM Towns, X68000, PC/AT                             | CPU speed mode: `low` or `high`                                                 | `high`                       |
| `--boot-mode <MODE>`        | PC-88, FM-7                                                       | Boot mode. PC-88: `v1s`, `v1h`, `v2`, `n`, `n80`, `n80sr`. FM-7: `basic`, `dos` | `v2` / `basic`               |
| `--monitor <MODE>`          | PC-88, X1                                                         | Monitor timing: `auto`, `15k`, `24k`                                            | `auto`                       |
| `--boot-device <DEVICE>`    | PC-98, FM Towns, PC/AT                                            | Boot device: `auto`, `fdd1`, `fdd2`, `hdd1`, `hdd2`, `dos`                      | `auto`                       |
| `--bios [on\|off]`          | PC-98, PC/AT                                                      | Boot the real BIOS of the ROM set instead of the HLE BIOS                       | HLE BIOS                     |
| `--fdd1 <PATH>`             | All                                                               | Floppy disk image for drive 1 (repeatable)                                      | -                            |
| `--fdd2 <PATH>`             | PC-98, PC-88, PC-88VA, PC-6000, FM Towns, X68000, X1, FM-7, PC/AT | Floppy disk image for drive 2 (repeatable)                                      | -                            |
| `--hdd1 <PATH>`             | PC-98, FM Towns, X68000, PC/AT                                    | Hard disk image for hard disk drive 1                                           | -                            |
| `--hdd2 <PATH>`             | PC-98, FM Towns, X68000, PC/AT                                    | Hard disk image for hard disk drive 2                                           | -                            |
| `--cdrom <PATH>`            | PC-9821, PC-88, PC/AT, FM Towns, X68000                           | CD-ROM disc image `.cue` or `.ccd` file (repeatable)                            | -                            |
| `--cartridge <PATH>`        | PC-6000, MSX                                                      | Cartridge ROM image                                                             | -                            |
| `--cassette <PATH>`         | PC-6000, MSX, X1, FM-7                                            | Cassette tape image                                                             | -                            |
| `--midi <DEVICE>`           | PC-98, PC/AT, FM Towns, X68000                                    | MIDI device: `none`, `mt32`, `sc55`                                             | `none`                       |
| `--mt32-roms <PATH>`        | PC-98, PC/AT, FM Towns, X68000                                    | Folder of the MT-32 ROM set (requires the `mt32` feature)                       | `roms/mt32`                  |
| `--sc55-roms <PATH>`        | PC-98, PC/AT, FM Towns, X68000                                    | Folder of the SC-55 ROM set (requires the `sc55` feature)                       | `roms/sc55`                  |
| `--audio-volume <FLOAT>`    | All                                                               | Audio volume 0.0-1.0                                                            | `1.0`                        |
| `--aspect-mode <MODE>`      | All                                                               | Display aspect mode: `4:3` or `1:1`                                             | `4:3`                        |
| `--crt [on\|off]`           | All                                                               | Enable the CRT effect. The legacy backend has no CRT effect                     | `on`                         |
| `--scaling <MODE>`          | All                                                               | Scaling method: `nearest`, `bilinear`, `pixelart`                               | `pixelart`                   |
| `--backend <BACKEND>`       | All                                                               | Rendering backend: `modern` or `legacy`                                         | `modern`                     |
| `--window-mode <MODE>`      | All                                                               | Window mode: `windowed` or `fullscreen`                                         | `windowed`                   |
| `-h, --help`                | All                                                               | Print help                                                                      | -                            |
| `-V, --version`             | All                                                               | Print version                                                                   | -                            |

The family pages describe the accepted media formats and the boot devices of each
family. [Disk images](disk-images.md) lists the image formats.

## ROM folder options

Each family reads its ROM set from its own folder. Every ROM option defaults to the
folder of its system below `--roms-folder`. See [ROMs](roms.md) for the files of
each set.

| Option                 | System   | Default       |
|------------------------|----------|---------------|
| `--pc98-roms <PATH>`   | PC-98    | `roms/pc98`   |
| `--pc88-roms <PATH>`   | PC-88    | `roms/pc88`   |
| `--pc88va-roms <PATH>` | PC-88VA  | `roms/pc88va` |
| `--pc6000-roms <PATH>` | PC-6000  | `roms/pc6000` |
| `--msx-roms <PATH>`    | MSX      | `roms/msx`    |
| `--towns-roms <PATH>`  | FM Towns | `roms/towns`  |
| `--fm7-roms <PATH>`    | FM-7     | `roms/fm7`    |
| `--x68k-roms <PATH>`   | X68000   | `roms/x68k`   |
| `--x1-roms <PATH>`     | X1       | `roms/x1`     |
| `--at-roms <PATH>`     | PC/AT    | `roms/at`     |

## Machine types

The `--machine <TYPE>` values are:

| System   | Values                                                                            |
|----------|-----------------------------------------------------------------------------------|
| PC-98    | `PC9801F`, `PC9801VM`, `PC9801VX`, `PC9801RS`, `PC9801RA`, `PC9821AS`, `PC9821AP` |
| PC-88    | `PC8801MC`                                                                        |
| PC-88VA  | `PC88VA2`                                                                         |
| PC-6000  | `PC6001`, `PC6001MK2`, `PC6601`, `PC6001MK2SR`, `PC6601SR`                        |
| MSX      | `MSX`, `MSX2`, `MSX2PLUS`                                                         |
| FM Towns | `FMTowns`, `FMTownsIICX`, `FMTownsIIMX`                                           |
| FM-7     | `FM7`, `FM77AV`                                                                   |
| X68000   | `X68000`, `X68000SUPER`, `X68000XVI`                                              |
| X1       | `X1`, `X1TURBO`                                                                   |
| PC/AT    | `AT486DX50`, `AT486DX66`                                                          |

[Machine selection](configuration.md#machine-selection) describes how a game file
without a `machine` key selects its machine.

## Commands

| Command                           | Description                                               |
|-----------------------------------|-----------------------------------------------------------|
| `create-fdd <PATH> [OPTIONS]`     | Create an empty floppy disk image                         |
| `create-hdd <PATH> --type <TYPE>` | Create an empty hard disk image                           |
| `convert-hdd <INPUT> <OUTPUT>`    | Convert a hard disk image between SASI and IDE formats    |
| `copy <SOURCE> <DEST>`            | Copy files between the host and FAT-formatted disk images |

[Disk image tools](disk-images.md#disk-image-tools) documents each command.
