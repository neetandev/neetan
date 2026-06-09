# Neetan (ねーたん)

An emulator for the PC-98 written in Rust.

[Game Compatibility](https://github.com/hasenbanck/neetan/wiki/Game-Compatibility)

## Supported systems

Currently, we aim to support all 16-bit era DOS games and emulate them accurately for 6 idealized machine targets:

| Machine   | CPU      | CPU Speed   | FPU (x87) | Extended RAM | Graphics | Interface | CD-ROM |
|-----------|----------|-------------|-----------|--------------|----------|-----------|--------|
| PC-9801F  | 8086     | 5 / 8 MHz   | No        | None         | GDC      | SASI      | No     |
| PC-9801VM | V30      | 8 / 10 MHz  | No        | None         | GRCG     | SASI      | No     |
| PC-9801VX | 80286    | 8 / 10 MHz  | No        | 4 MiB        | EGC      | SASI      | No     |
| PC-9801RA | 80386DX  | 16 / 20 MHz | Yes       | 12 MiB       | EGC      | SASI      | No     |
| PC-9821AS | 80486DX  | 33 MHz      | Yes       | 14 MiB       | PEGC     | IDE       | Yes    |
| PC-9821AP | 80486DX2 | 66 MHz      | Yes       | 14 MiB       | PEGC     | IDE       | Yes    |

All targets have the full 640 KiB conventional RAM. The 8086 and V30 are cycle-count accurately emulated.
The emulated 286 is calibrated to run at the same speed as the original 286 using trace data. The 386 and the 486
are optimized for emulation speed and most likely run a bit fast compared to their real counterparts.

We also support the following sound cards:

* PC beeper
* PC-9801-14 Music Generator (TMS3631 8-channel synth)
* PC-9801-26k
* PC-9801-86
* PC-9801-86 + PC-9801-26k combo
* Sound Blaster 16
* Sound Blaster 16 + PC-9801-26k combo
* Roland MT-32 using the MPU-PC98II interface
* Roland SC-55 using the MPU-PC98II interface

The default for the CLI is the PC-9801RA machine with the PC-9801-86 + PC-9801-26k combo soundboards.

## Usage

```bash
neetan [OPTIONS]
neetan <COMMAND>
```

### Options

| Option                       | Description                                                                         | Default    |
|------------------------------|-------------------------------------------------------------------------------------|------------|
| `-c, --config <PATH>`        | Load configuration from file                                                        | -          |
| `--machine <TYPE>`           | Machine type: `PC9801F`, `PC9801VM`, `PC9801VX`, `PC9801RA`, `PC9821AS`, `PC9821AP` | `PC9801RA` |
| `--cpu-mode <MODE>`          | CPU speed mode: low or high (default: high; PC-9801 only)                           | high       |
| `--fdd1 <PATH>`              | Floppy disk image for drive 1 (repeatable)                                          | -          |
| `--fdd2 <PATH>`              | Floppy disk image for drive 2 (repeatable)                                          | -          |
| `--hdd1 <PATH>`              | Hard disk image for hard disk drive 1                                               | -          |
| `--hdd2 <PATH>`              | Hard disk image for hard disk drive 2                                               | -          |
| `--cdrom <PATH>`             | CD-ROM disc image .cue or .ccd file (repeatable, PC-9821 only)                      | -          |
| `--audio-volume <FLOAT>`     | Audio volume 0.0–1.0                                                                | `1.0`      |
| `--aspect-mode <MODE>`       | Display aspect mode: `4:3` or `1:1`                                                 | `4:3`      |
| `--crt <on\|off>`            | Enable the CRT effect. Not avialable when using the legacy backend.                 | `on`       |
| `--scaling <MODE>`           | Scaling method: `nearest`, `bilinear`, `pixelart`                                   | `pixelart` |
| `--backend <BACKEND>`        | Rendering backend: `modern` or `legacy`                                             | `modern`   |
| `--window-mode <MODE>`       | Window mode: `windowed` or `fullscreen`                                             | `windowed` |
| `--force-gdc-clock <2.5\|5>` | Force GDC clock to 2.5 or 5 MHz. VX and later only                                  | auto       |
| `--bios-rom <PATH>`          | Path to BIOS ROM file                                                               | HLE BIOS   |
| `--font-rom <PATH>`          | Path to font ROM file                                                               | Built-in   |
| `--soundboard <TYPE>`        | Sound board: `none`, `14`, `26k`, `86`, `86+26k`, `sb16`, `sb16+26k`                | `86+26k`   |
| `--graphicboard <TYPE>`      | Graphics accelerator board: `none`, `ga1280a`                                       | `none`     |
| `--midi <DEVICE>`            | MIDI device: `none`, `mt32`, `sc55`                                                 | `none`     |
| `--mt32-roms <PATH>`         | Path to MT-32 ROM directory (requires `mt32` feature)                               | -          |
| `--sc55-roms <PATH>`         | Path to SC-55 ROM directory (requires `sc55` feature)                               | -          |
| `--boot-device <DEVICE>`     | Boot device: `auto`, `fdd1`, `fdd2`, `hdd1`, `hdd2`, `dos`                          | `auto`     |
| `--printer <PATH>`           | Output file for printer (must exist)                                                | -          |
| `--enable-extractor`         | Copy on-screen Japanese text to the system clipboard, one line at a time            | off        |
| `-h, --help`                 | Print help                                                                          | -          |
| `-V, --version`              | Print version                                                                       | -          |

### Commands

`create-fdd <PATH> [OPTIONS]` - Create an empty floppy disk image (D88 format).

| Option          | Description                         | Default |
|-----------------|-------------------------------------|---------|
| `--type <TYPE>` | `2hd` (1232 KiB) or `2dd` (640 KiB) | `2hd`   |

`create-hdd <PATH> [OPTIONS]` - Create an empty hard disk image (HDI format).

| Option          | Description                                                                                                          |
|-----------------|----------------------------------------------------------------------------------------------------------------------|
| `--type <TYPE>` | SASI: `sasi5`, `sasi10`, `sasi15`, `sasi20`, `sasi30`, `sasi40`. IDE: `ide40`, `ide80`, `ide120`, `ide200`, `ide500` |

`convert-hdd <INPUT> <OUTPUT>` - Convert a hard disk image between SASI and IDE formats.

The conversion direction is auto-detected from the input image's sector size (256 bytes = SASI, 512 bytes = IDE).
The smallest compatible target geometry is chosen automatically. Output is always in HDI format.

SASI to IDE conversion always succeeds (all SASI sizes fit within ide40).
IDE to SASI conversion will fail if the IDE image exceeds the largest SASI capacity (sasi40 at ~40 MB).

### Configuration file

Instead of passing all options on the command line, you can use a configuration file with `-c`:

```bash
neetan -c my_game.cfg
```

The file uses a simple `key = value` format. Lines starting with `#` or `;` are comments.
See [`configuration/default.conf`](configuration/default.conf) for a complete reference with all
options and their defaults.

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

neetan automatically loads a global configuration file from the OS data directory if it exists.
This is useful for setting persistent defaults like your preferred machine type, sound card, or keyboard mapping
without needing to pass `--config` or CLI flags every time.

The global config file uses the same `key = value` format as regular configuration files.

#### File location

| OS      | Path                                                         |
|---------|--------------------------------------------------------------|
| Linux   | `~/.local/share/neetan/neetan/global.conf`                   |
| Windows | `C:\Users\<user>\AppData\Roaming\neetan\neetan\global.conf`  |
| macOS   | `~/Library/Application Support/neetan/neetan/global.conf`    |

The directory is created automatically. The configuration file must be created manually.

#### Layering order 

Settings are applied in this order, with later layers overriding earlier ones:

1. Built-in defaults
2. Global configuration file (`global.conf` in OS data directory)
3. Per-invocation configuration file (`--config`)
4. Command-line arguments

For example, if your global config sets `machine = PC9801RA` and you run
`neetan --config game.cfg --soundboard sb16`, the machine will be PC9801RA
(from global config) unless `game.cfg` or CLI args override it.

### Emulator controls

| Key                | Action                           |
|--------------------|----------------------------------|
| Right Ctrl         | Toggle mouse capture             |
| GUI + Alt + Enter  | Toggle fullscreen                |
| GUI + Alt + Escape | Quit the emulator                |
| GUI + Alt + F1     | Toggle CRT effect                |
| GUI + Alt + F2     | Cycle scaling method             |
| GUI + Alt + F9     | Open floppy selector for drive 1 |
| GUI + Alt + F10    | Open floppy selector for drive 2 |
| GUI + Alt + F11    | Open CD-ROM selector             |
| GUI + Alt + F12    | Log HLE DOS memory overview      |

(GUI is the Windows / Command key)

### Text extraction

When started with `--enable-extractor` (or `enable-extractor = on` in a
configuration file), Neetan observes glyph fetches from the CGROM and
copies on-screen text to the system clipboard, one completed line at a time.
This is intended for use with external machine-translation tools such as
Textractor, Translator++, or other translation tools that watch
the clipboard.

Limitations:

- This iteration covers games that render text by reading the CGROM
  (visual novels and similar engines). Titles that draw text solely from
  the text VRAM (most DOS prompts and menus) are not captured yet.
- Characters that the JIS-to-Unicode conversion cannot map (custom
  user-defined glyphs, etc.) are silently dropped.

### Supported floppy disk image formats

| Format  | Extensions                     | Writable | Description                                        |
|---------|--------------------------------|----------|----------------------------------------------------|
| D88     | `.d88`, `.d98`, `.88d`, `.98d` | Yes      | Standard PC-98 disk image with per-sector metadata |
| HDM     | `.hdm`                         | No       | Headerless raw sector image (2HD only)             |
| NFD     | `.nfd`                         | No       | T98Next format with per-sector metadata            |

Only D88 images preserve modifications written by the emulated software. HDM and NFD images are currently read-only.

### Supported CD-ROM disc image formats

| Format  | Extensions | Description                                                                                       |
|---------|------------|---------------------------------------------------------------------------------------------------|
| CUE/BIN | `.cue`     | CUE sheet referencing a raw BIN image (PC-9821)                                                   |
| CloneCD | `.ccd`     | CloneCD control file with sibling `.img` (raw 2352-byte sectors) and optional `.sub` subchannel data (PC-9821) |

## Multiple disk images

Many PC-98 games ship on multiple floppy disks and ask you to swap disks during gameplay.
Some CD-ROM games also come as multiple disc images.
neetan handles this by letting you assign several disk images to each drive up front, then swap
between them at runtime.

### Providing multiple disks

Use the `--fdd1` / `--fdd2` / `--cdrom` flags more than once to register all images for a drive:

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

### Swapping disks at runtime

Press **GUI + Alt + F9** (drive 1), **GUI + Alt + F10** (drive 2), or **GUI + Alt + F11** (CD-ROM) to open the image selector.

## MIDI sound modules

neetan can emulate MIDI sound modules connected via the MPU-PC98II interface. Two modules are supported:

* Roland MT-32 - using a Rust port of [munt](https://github.com/munt/munt)
* Roland SC-55 - using a Rust port of [Nuked-SC55](https://github.com/nukeykt/Nuked-SC55)

Both features are optional, but enabled by default and require external ROM files to work.

### Why they are optional

The munt code is licensed under [LGPL 2.1](https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html),
and the Nuked-SC55 code is licensed under the original MAME license (non-commercial use only).
See the [License](#license) section for details. Distributions that cannot comply with these
licenses can disable one or both at build time:

```bash
cargo build --release --no-default-features                       # neither
cargo build --release --no-default-features --features mt32       # MT-32 only
cargo build --release --no-default-features --features sc55       # SC-55 only
```

When built without a feature, the corresponding `--midi` option is still accepted but the
emulator will print a warning and continue without audio for that module.

### Graphic acceleration board

neetan can emulate the I-O DATA GA-1280A graphics accelerator board, the
high-end variant to the GA-1024A. All GA-1024A software is compatible with the
GA-1280A.

The board is primarily useful for Windows 3.1, where it unlocks higher
resolutions than the stock EGC/PEGC video paths (up to 1600x1024 pixel).

Enable it on the CLI with `--graphicboard ga1280a`, or in a configuration file
with `graphicboard = ga1280a`. The default is `none`.

Official I-O DATA drivers for MS-DOS, Windows 3.1, and Windows 95 are still
available from the manufacturer:
<https://www.iodata.jp/lib/software/g/106.htm#MS-DOS>

### MT-32 ROM files

Place your MT-32 ROM files (`.rom` extension) into a single directory and point `--mt32-roms`
at it. The emulator identifies ROMs by BLAKE3 hash, so filenames do not matter. You need one
control ROM and one PCM ROM. Split ROM pairs (two halves) are also supported and merged
automatically.

| Model                     | Control ROM versions                  |
|---------------------------|---------------------------------------|
| MT-32                     | v1.04, v1.05, v1.06, v1.07, BlueRidge |
| MT-32 (new / "old" v2)    | v2.04, v2.06, v2.07                   |
| CM-32L / LAPC-I           | v1.00, v1.02                          |
| CM-32LN / CM-500 / LAPC-N | v1.00                                 |

It seems that currently the control ROM version v1.04, v1.05, v1.06 and v1.07 of the MT-32 have the best compatibility.

### SC-55 ROM files

Place the ROM files for your device model into a single directory and point `--sc55-roms` at it.
The emulator auto-detects the model from the filenames present.

| Model                   | Required files                                                                                       |
|-------------------------|------------------------------------------------------------------------------------------------------|
| SC-55mk2 / SC-155mk2    | `rom1.bin`, `rom2.bin`, `rom_sm.bin`, `waverom1.bin`, `waverom2.bin`                                 |
| SC-55st                 | `rom1.bin`, `rom2_st.bin`, `rom_sm.bin`, `waverom1.bin`, `waverom2.bin`                              |
| SC-55 (mk1)             | `sc55_rom1.bin`, `sc55_rom2.bin`, `sc55_waverom1.bin`, `sc55_waverom2.bin`, `sc55_waverom3.bin`      |
| CM-300 / SCC-1 / SCC-1A | `cm300_rom1.bin`, `cm300_rom2.bin`, `cm300_waverom1.bin`, `cm300_waverom2.bin`, `cm300_waverom3.bin` |
| JV-880                  | `jv880_rom1.bin`, `jv880_rom2.bin`, `jv880_waverom1.bin`, `jv880_waverom2.bin`                       |
| SCB-55 / RLP-3194       | `scb55_rom1.bin`, `scb55_rom2.bin`, `scb55_waverom1.bin`, `scb55_waverom2.bin`                       |
| RLP-3237                | `rlp3237_rom1.bin`, `rlp3237_rom2.bin`, `rlp3237_waverom1.bin`                                       |
| SC-155                  | `sc155_rom1.bin`, `sc155_rom2.bin`, `sc155_waverom1.bin`, `sc155_waverom2.bin`, `sc155_waverom3.bin` |

### Usage

Set the MIDI device and provide the path to the ROM directory:

```bash
neetan --midi mt32 --mt32-roms /path/to/mt32_roms [other options...]
neetan --midi sc55 --sc55-roms /path/to/sc55_roms [other options...]
```

Or in a configuration file:

```ini
midi = mt32
mt32-roms = /path/to/mt32_roms
```

```ini
midi = sc55
sc55-roms = /path/to/sc55_roms
```

Both ROM paths can be set in the global configuration file so they only need to be specified once.
MIDI emulation is only activated when both the `--midi` device and the corresponding ROM path are
set, so you can keep ROM paths in your global config and toggle per-game by changing only `--midi`.

## FAQ

### Which ROM files do I need for this emulation?

You don't need any rom files. If you have the correct rom files, you CAN use them, but there is not a particular reason
to use them, since our HLE BIOS and HLE SASI BIOS has very good compatibility.
We also include a self created open source font ROM and also provide the tools to re-create / change it.
With these systems in place we are able to run th fast majority of PC-98 games and applications.

There are some BIOS extensions, mainly the sound API and LIO API that we currently haven't implemented, but outside
some odd BASIC based games, they should not be used by games, which interface with the hardware I/O port directly.

The only exceptions are the optional MT-32 and SC-55 MIDI modules, which need external ROM files to work correctly
as described in the "MIDI sound modules" section.

### How can I use my mouse?

In games that support a mouse, you first need to capture the mouse pointer via the right CTRL key. You can release
the mouse pointer by clicking the right CTRL key again.

### How do I rebind my keys?

You can remap keys in the configuration file using `key.<HostKey> = <PC-98 Key>` entries.
See [`configuration/default.conf`](configuration/default.conf) for a complete reference of all
available host key names, PC-98 key names, and the default mappings.

### 日本語も分かりますか？

もちろん！IssueやPRの作成には日本語をご利用いただけますが、ソースコードのコメントについては英語での記述を推奨しております。

## Build requirements

* [Rust 1.95](https://rustup.rs/)
* [SDL3](https://github.com/libsdl-org/SDL) (See [sdl3_sys descriptio](https://docs.rs/sdl3-sys/latest/sdl3_sys/#usage))

## Acknowledgement

Following projects provided references for our implementation and test vectors. They were invaluable for developing
neetan:

- [MAME](https://www.mamedev.org/) 
- [MartyPC](https://github.com/dbalsom/martypc)
- [NP21W](https://simk98.github.io/np21w/)
- [SingleStepTests](https://github.com/SingleStepTests)
- [undoc98](https://www.webtech.co.jp/company/doc/undocumented_mem/index.html)

We ported the Yamaha OPN and OPL emulation from the amazing YMFM project to our own Rust port:

- [ymfm](https://github.com/aaronsgiles/ymfm)

We ported the Roland SC-55 emulator from the incredible Nuked SC55 project to our own Rust port:

- [Nuked-SC55](https://github.com/nukeykt/Nuked-SC55)

We ported the Roland MT-32 emulator from the outstanding munt project to our own Rust port:

- [munt](https://github.com/munt/munt)

## Rules for AI coding assistants

AI agents MUST NOT add Signed-off-by tags. Only humans can legally certify the origin of the source code.
AI agents are only tools and do not absolve the human submitter of their responsibility:

* Reviewing all AI-generated code
* Ensuring compliance with licensing requirements
* Taking full responsibility for the contribution

An `Assisted-by` tag MUST NOT be added, since that information is irrelevant to the contribution of the human
contributor.

## License

This project is licensed under the [3-clause BSD](https://opensource.org/license/bsd-3-clause) license.

When optional features are enabled, the license terms of the resulting binary change:

| Build configuration             | Binary license                |
|---------------------------------|-------------------------------|
| Default (no optional features)  | BSD 3-Clause                  |
| `sc55` feature enabled          | BSD 3-Clause + non-commercial |
| `mt32` feature enabled          | LGPL 2.1                      |
| `sc55` + `mt32` enabled         | LGPL 2.1 + non-commercial     |

The `sc55` feature links the Nuked-SC55 port, which is licensed under the original MAME license
(non-commercial use only). The `mt32` feature links the munt port, which is licensed under
[LGPL 2.1](https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html).

The source code of the BSD 3-Clause licensed components remains available under BSD 3-Clause
regardless of the build configuration.
