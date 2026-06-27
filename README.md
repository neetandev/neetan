# Neetan (ねーたん)

An emulator for the PC-8001, PC-8801, PC-88VA, PC-9801 and PC-9821 written in Rust.

[Game Compatibility](https://github.com/neetandev/neetan/wiki)

## Supported systems

Neetan emulates three distinct families, selected through the `--machine` option:

* PC-9801 / PC-9821 line (six idealized targets).
* PC-8001 / PC-8801 line (one idealized target).
* PC-88VA line (one idealized target).

### PC-9801 / PC-9821

We aim to support all 16-bit era DOS games and emulate them accurately for 6 idealized
machine targets:

| Machine   | CPU      | CPU Speed   | FPU (x87) | Extended RAM | Graphics | Interface | CD-ROM |
|-----------|----------|-------------|-----------|--------------|----------|-----------|--------|
| PC-9801F  | 8086     | 5 / 8 MHz   | No        | None         | GDC      | SASI      | No     |
| PC-9801VM | V30      | 8 / 10 MHz  | No        | None         | GRCG     | SASI      | No     |
| PC-9801VX | 80286    | 8 / 10 MHz  | No        | 4 MiB        | EGC      | SASI      | No     |
| PC-9801RA | 80386DX  | 16 / 20 MHz | Yes       | 12 MiB       | EGC      | SASI      | No     |
| PC-9821AS | 80486DX  | 33 MHz      | Yes       | 14 MiB       | PEGC     | IDE       | Yes    |
| PC-9821AP | 80486DX2 | 66 MHz      | Yes       | 14 MiB       | PEGC     | IDE       | Yes    |

All targets have the full 640 KiB conventional RAM. The 8086 and V30 are cycle-count
accurately emulated. The emulated 286 is calibrated to run at the same speed as the
original 286 using trace data. The 386 and the 486 are optimized for emulation speed
and most likely run a bit fast compared to their real counterparts.

The default for the CLI is the PC-9801RA machine with the PC-9801-86 + PC-9801-26k
combo soundboards. See [PC-9801 / PC-9821 systems](#pc-9801--pc-9821-systems) for the
sound cards and platform-specific options.

Games of the PC-98 normally do not require any ROM files, with the rare exception of
games that used the PC-98's N88 (86) BASIC ROM.

### PC-8001 / PC-8801

Neetan emulates a single PC-8801 target, the PC-8801MC. Through its `--boot-mode`
setting (the equivalent of the machine's DIP switches) it also takes on the
personalities of the earlier PC-8001 family. Unlike the PC-98 line, the PC-8801MC
requires a real ROM set supplied via `--pc88-roms`, since of the deep integration
of the NEC BASIC variants (N, N80, N80 SR and N88).

See [PC-8001 / PC-8801 systems](#pc-8001--pc-8801-systems) for the boot modes, their
hardware mapping and compatibility caveats, the ROM set, and platform-specific
options.

### PC-88VA

Neetan emulates a single PC-88VA target, the PC-88VA2: an NEC V30 running at roughly
8 MHz with 512 KiB of main RAM, a built-in OPNA, and a custom graphics chipset with
the Super Graphic Processor (SGP) blitter. A Z80 sub-CPU at 4 MHz drives the floppy
unit. Like the PC-8801MC it has no HLE firmware and requires a real ROM set,
supplied via `--pc88va-roms`.

See [PC-88VA systems](#pc-88va-systems) for the ROM set and platform-specific options.

## Usage

```bash
neetan [OPTIONS]
neetan <COMMAND>
```

### Options

The `System` column shows where an option applies: `All` (every family), `PC-98`
(PC-9801 / PC-9821 only), `PC-9821` (PC-9821 only), `PC-88` (PC-8001 / PC-8801
only), or `PC-88VA` (PC-88VA only). Options that apply to one family are ignored on
the others.

| Option                       | System  | Description                                                                                  | Default           |
|------------------------------|---------|----------------------------------------------------------------------------------------------|-------------------|
| `-c, --config <PATH>`        | All     | Load configuration from file                                                                 | -                 |
| `--machine <TYPE>`           | All     | `PC9801F`, `PC9801VM`, `PC9801VX`, `PC9801RA`, `PC9821AS`, `PC9821AP`, `PC8801MC`, `PC88VA2` | `PC9801RA`        |
| `--cpu-mode <MODE>`          | All     | CPU speed mode: `low` or `high` (PC-88 default derives from the boot mode)                   | `high` (PC-98)    |
| `--boot-mode <MODE>`         | PC-88   | BASIC boot mode: `v1s`, `v1h`, `v2`, `n`, `n80`, `n80sr`                                     | `v2`              |
| `--pc88-monitor <MODE>`      | PC-88   | Monitor timing: `auto`, `15k`, `24k`                                                         | `auto`            |
| `--pc88-memory-wait <MODE>`  | PC-88   | Memory wait: `fast` or `compatible`                                                          | derives from mode |
| `--pc88-8mhz-wait <MODE>`    | PC-88   | 8 MHz wait: `fast` or `compatible`                                                           | `fast`            |
| `--pc88-roms <PATH>`         | PC-88   | Directory with the PC-8801MC ROM set (required for `PC8801MC`)                               | -                 |
| `--pc88va-roms <PATH>`       | PC-88VA | Directory with the PC-88VA2 ROM set (required for `PC88VA2`)                                 | -                 |
| `--fdd1 <PATH>`              | All     | Floppy disk image for drive 1 (repeatable)                                                   | -                 |
| `--fdd2 <PATH>`              | All     | Floppy disk image for drive 2 (repeatable)                                                   | -                 |
| `--hdd1 <PATH>`              | All     | Hard disk image for hard disk drive 1                                                        | -                 |
| `--hdd2 <PATH>`              | All     | Hard disk image for hard disk drive 2                                                        | -                 |
| `--cdrom <PATH>`             | PC-9821 | CD-ROM disc image .cue or .ccd file (repeatable)                                             | -                 |
| `--audio-volume <FLOAT>`     | All     | Audio volume 0.0-1.0                                                                         | `1.0`             |
| `--aspect-mode <MODE>`       | All     | Display aspect mode: `4:3` or `1:1`                                                          | `4:3`             |
| `--crt <on\|off>`            | All     | Enable the CRT effect. Not available when using the legacy backend.                          | `on`              |
| `--scaling <MODE>`           | All     | Scaling method: `nearest`, `bilinear`, `pixelart`                                            | `pixelart`        |
| `--backend <BACKEND>`        | All     | Rendering backend: `modern` or `legacy`                                                      | `modern`          |
| `--window-mode <MODE>`       | All     | Window mode: `windowed` or `fullscreen`                                                      | `windowed`        |
| `--force-gdc-clock <2.5\|5>` | PC-98   | Force GDC clock to 2.5 or 5 MHz. VX and later only                                           | auto              |
| `--graphicboard <TYPE>`      | PC-98   | Graphics accelerator board: `none`, `ga1280a`                                                | `none`            |
| `--bios-rom <PATH>`          | PC-98   | Path to BIOS ROM file                                                                        | HLE BIOS          |
| `--font-rom <PATH>`          | PC-98   | Path to font ROM file                                                                        | Built-in          |
| `--soundboard <TYPE>`        | PC-98   | Sound board: `none`, `14`, `26k`, `86`, `86+26k`, `sb16`, `sb16+26k`                         | `86+26k`          |
| `--adpcm-ram <on\|off>`      | PC-98   | ADPCM RAM option for the PC-9801-86                                                          | `on`              |
| `--ems <on\|off>`            | PC-98   | Enable EMS expanded memory                                                                   | `on`              |
| `--xms <on\|off>`            | PC-98   | Enable XMS extended memory                                                                   | `on`              |
| `--midi <DEVICE>`            | PC-98   | MIDI device: `none`, `mt32`, `sc55`                                                          | `none`            |
| `--mt32-roms <PATH>`         | PC-98   | Path to MT-32 ROM directory (requires `mt32` feature)                                        | -                 |
| `--sc55-roms <PATH>`         | PC-98   | Path to SC-55 ROM directory (requires `sc55` feature)                                        | -                 |
| `--boot-device <DEVICE>`     | All     | Boot device: `auto`, `fdd1`, `fdd2`, `hdd1`, `hdd2`, `dos`                                   | `auto`            |
| `--printer <PATH>`           | All     | Output file for printer (must exist)                                                         | -                 |
| `--enable-extractor`         | All     | Copy on-screen Japanese text to the system clipboard, one line at a time                     | off               |
| `-h, --help`                 | All     | Print help                                                                                   | -                 |
| `-V, --version`              | All     | Print version                                                                                | -                 |

### Commands

`create-fdd <PATH> [OPTIONS]` - Create an empty floppy disk image (D88 format).

| Option          | Description                                      | Default |
|-----------------|--------------------------------------------------|---------|
| `--type <TYPE>` | `2hd` (1232 KB), `2dd` (640 KB) or `2d` (320 KB) | `2hd`   |

`create-hdd <PATH> [OPTIONS]` - Create an empty hard disk image (HDI format).

| Option          | Description                                                                                                          |
|-----------------|----------------------------------------------------------------------------------------------------------------------|
| `--type <TYPE>` | SASI: `sasi5`, `sasi10`, `sasi15`, `sasi20`, `sasi30`, `sasi40`. IDE: `ide40`, `ide80`, `ide120`, `ide200`, `ide500` |

`convert-hdd <INPUT> <OUTPUT>` - Convert a hard disk image between SASI and IDE formats.

The conversion direction is auto-detected from the input image's sector size (256 bytes = SASI, 512 bytes = IDE).
The smallest compatible target geometry is chosen automatically. Output is always in HDI format.

SASI to IDE conversion always succeeds (all SASI sizes fit within ide40).
IDE to SASI conversion will fail if the IDE image exceeds the largest SASI capacity (sasi40 at ~40 MB).

## PC-9801 / PC-9821 systems

Select a PC-98 machine with `--machine`; the default is `PC9801RA`. See the
[PC-9801 / PC-9821 table](#pc-9801--pc-9821) above for the per-target hardware
summary.

### Sound cards

We support the following sound cards via `--soundboard`:

* PC beeper
* PC-9801-14 Music Generator (TMS3631 8-channel synth)
* PC-9801-26k
* PC-9801-86
* PC-9801-86 + PC-9801-26k combo
* Sound Blaster 16
* Sound Blaster 16 + PC-9801-26k combo
* Roland MT-32 using the MPU-PC98II interface
* Roland SC-55 using the MPU-PC98II interface

The MT-32 and SC-55 modules are configured separately via `--midi`; see
[MIDI sound modules](#midi-sound-modules).

### Platform options

| Option                       | Description                                                          | Default  |
|------------------------------|----------------------------------------------------------------------|----------|
| `--soundboard <TYPE>`        | Sound board: `none`, `14`, `26k`, `86`, `86+26k`, `sb16`, `sb16+26k` | `86+26k` |
| `--adpcm-ram <on\|off>`      | ADPCM RAM option for the PC-9801-86                                  | `on`     |
| `--ems <on\|off>`            | Enable EMS expanded memory                                           | `on`     |
| `--xms <on\|off>`            | Enable XMS extended memory                                           | `on`     |
| `--force-gdc-clock <2.5\|5>` | Force GDC clock to 2.5 or 5 MHz (VX and later only)                  | auto     |
| `--graphicboard <TYPE>`      | Graphics accelerator board: `none`, `ga1280a`                        | `none`   |
| `--bios-rom <PATH>`          | Path to BIOS ROM file                                                | HLE BIOS |
| `--font-rom <PATH>`          | Path to font ROM file                                                | Built-in |

CD-ROM disc images (`--cdrom`) are supported on the PC-9821 targets only.

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

## PC-8001 / PC-8801 systems

Select the PC-88 family with `--machine PC8801MC`. The emulated machine is always
the PC-8801MC; the `--boot-mode` option chooses which BASIC personality it powers up
with, which is how the earlier PC-8001 family is emulated. Unlike the PC-98 targets,
the PC-8801MC has no HLE firmware and **requires a real ROM set** (see
[ROM set](#rom-set)).

Sound is provided by the machine's built-in OPNA (Sound Board II, which should be
fully compatible to the Sound Board I).

### Boot modes

The boot mode is selected with `--boot-mode` and defaults to `v2`. The `v1s`, `v1h`
and `v2` modes are the PC-8801's own modes; the `n`, `n80` and `n80sr`
modes emulate the three generations of the PC-8001 line:

| `--boot-mode` | Alias   | BASIC dialect                 | Emulated machine |
|---------------|---------|-------------------------------|------------------|
| `v1s`         | -       | N88-BASIC V1 (standard speed) | PC-8801          |
| `v1h`         | -       | N88-BASIC V1 (high speed)     | PC-8801          |
| `v2`          | -       | N88-BASIC V2 (default)        | PC-8801          |
| `n`           | -       | N-BASIC                       | PC-8001          |
| `n80`         | `n80v1` | N80-BASIC                     | PC-8001mkII      |
| `n80sr`       | `n80v2` | N80SR-BASIC                   | PC-8001mkIISR    |

### N / N80 / N80SR are not interchangeable

The three N-family modes correspond to three different generations of real hardware:

* `n` - the original PC-8001 (1979) running plain N-BASIC.
* `n80` - the PC-8001mkII (1983) running N80-BASIC.
* `n80sr` - the PC-8001mkIISR (1985) running N80SR-BASIC.

While each generation broadly builds on the previous one, they are not a clean
superset chain and are not fully compatible with each other. Pick the boot mode
that matches the machine the software was written for.

The PC-8801 modes (`v1s`, `v1h`, `v2`) are a separate modes again and are
not compatible with the N-family modes. The vast majority of PC-88 games will
target the `v2` mode, which is the default config value.

### Platform options

| Option                      | Description                                                | Default           |
|-----------------------------|------------------------------------------------------------|-------------------|
| `--boot-mode <MODE>`        | Boot mode: `v1s`, `v1h`, `v2`, `n`, `n80`, `n80sr`         | `v2`              |
| `--pc88-roms <PATH>`        | Directory with the PC-8801MC ROM set (required)            | -                 |
| `--pc88-monitor <MODE>`     | Monitor timing: `auto`, `15k` (200-line), `24k` (400-line) | `auto`            |
| `--pc88-memory-wait <MODE>` | Memory wait states: `fast` or `compatible`                 | derives from mode |
| `--pc88-8mhz-wait <MODE>`   | 8 MHz wait mode: `fast` or `compatible`                    | `fast`            |
| `--cpu-mode <MODE>`         | CPU speed: `low` or `high`                                 | derives from mode |

The defaults of `--cpu-mode` and `--pc88-memory-wait` derive from the boot mode: the
`v1s` and N-family modes (`n`, `n80`, `n80sr`) default to the slower, compatible
behavior of the machines they emulate, while `v1h` and `v2` keep the fast defaults.
Pass the flags explicitly to override.

### ROM set

The PC-8801MC needs a real ROM set, pointed to by `--pc88-roms`. ROMs are identified
by their BLAKE3 content hash rather than by file name, so any dump layout works
regardless of how the files are named.

These ROMs are always required:

| Slot       | Size    | Contents                         | BLAKE3                                                             |
|------------|---------|----------------------------------|--------------------------------------------------------------------|
| `n88`      | 32 KiB  | N88-BASIC main ROM               | `40457b507b82dd57cce0fcecf6bc65543a60bd46558ca947b0f69dd3658cdad8` |
| `n88_ext0` | 8 KiB   | N88-BASIC extension bank 0       | `6a50a88231062ec871c65f63266fa7062a303ab870aed81c49f1f333f594a518` |
| `n88_ext1` | 8 KiB   | N88-BASIC extension bank 1       | `d5583fcce4eabf078d17666a1fddefa6a0d8bdc7f56d4499d526818728777252` |
| `n88_ext2` | 8 KiB   | N88-BASIC extension bank 2       | `ca200799765cb02a001bd55215b0daaf6d0593118a05e8d85754bddd92e5e8f7` |
| `n88_ext3` | 8 KiB   | N88-BASIC extension bank 3       | `ac31c1fbabfada9890669bebd471d60fac0be0e88ddfde81f17c600d5b0a1757` |
| `n_basic`  | 32 KiB  | N-BASIC ROM (PC-8001, 1979)      | `652eacc1ed6073bc3da1856c9c4f74ac14abef3f966f0d0fc89c40386de3d1a1` |
| `jisyo`    | 512 KiB | Kanji dictionary ROM             | `283dcd1c4a69f8049d19021d34d1cc2094f10de8b4e1ddf85da6a4b258dd8d12` |
| `kanji1`   | 128 KiB | Level-1 kanji ROM                | `10fd26424ae9e28be721846491d2d7b10e946da2d2ff39542248e819bc2339ba` |
| `kanji2`   | 128 KiB | Level-2 kanji ROM                | `f528e78bbe43e3d36c3def6ef30140e22ba9e69f422736605c2c4570c7d3fbe7` |
| `disk`     | 8 KiB   | PC80S31K disk sub-CPU ROM        | `081d2ca8ad7066de207b7360e45b5d6f3bab01769aefb9057141becbbaec5aa5` |
| `cdbios`   | 64 KiB  | PC-8801-31 CD-ROM interface BIOS | `de4d49437344806850b22356f9e5537e413e6113902fb8fbc803f902a5728827` |

These ROMs are required only when the matching boot mode is selected:

| Slot         | Size   | Required by boot mode | BLAKE3                                                             |
|--------------|--------|-----------------------|--------------------------------------------------------------------|
| `n80_mkii`   | 32 KiB | `n80`                 | `9e4ec9c53f4432a88583dccd04ae3186f4d7849f80ea7774ac1efbdb93c992f2` |
| `n80_mkiisr` | 32 KiB | `n80sr`               | `56406a79fd664a197c458cb3feeeb6994c34266a1e02728877b6ea5ef86e15ba` |
| `n80sr`      | 40 KiB | `n80sr`               | `7b81e27b831ad00f264170d1d98c645298fa688b07d5a9f0c19c1d6a73fe4273` |

## PC-88VA systems

Select the PC-88VA family with `--machine PC88VA2`. The emulated machine is the
PC-88VA2: an NEC V30 at roughly 8 MHz with 512 KiB of main RAM and a Z80 sub-CPU at
4 MHz for the floppy unit. Sound is provided by the machine's built-in OPNA.

Only the V3 graphic mode is emulated. This target is not backwards compatible with
the PC-8801 software. The original could run V1 and V2 BASIC games, since it's CPU
had an integrated Z80. We don't implement th hybrid nature and if you need to run
PC-8801 games, then please use the PC-8801MC target.

Like the PC-8801MC, it has no HLE firmware and requires a real ROM set (see [ROM set](#rom-set-1)).

### Platform options

| Option                  | Description                                       | Default |
|-------------------------|---------------------------------------------------|---------|
| `--pc88va-roms <PATH>`  | Directory with the PC-88VA2 ROM set (required)    | -       |

### ROM set

The PC-88VA2 needs a real ROM set, pointed to by `--pc88va-roms`. ROMs are identified by their
BLAKE3 content hash rather than by file name, so any dump layout works regardless of how the
files are named. All slots are required:

| Slot         | Size    | Contents                  | BLAKE3                                                             |
|--------------|---------|---------------------------|--------------------------------------------------------------------|
| `rom00`      | 512 KiB | ROM0 low image (varom00)  | `bba5011412fb266b3c15ff08d2508716ba2ac54fec3aa172b59e441486807eab` |
| `rom08`      | 128 KiB | ROM0 high image (varom08) | `4cdf3da9a1423e874f9618a8d8859107fa5e3d20a91f4dcf908e042763c41bbb` |
| `rom1`       | 128 KiB | ROM1 image (varom1)       | `1239bf390d444ff205f70c700527cb50bc90107904050fa8713a415a17bf0e42` |
| `font`       | 320 KiB | Kanji / font ROM          | `b47ec9f55ff199ac71f453385aec0f370afbb958fd47ad9bb5161bdf4e2bb3ee` |
| `dictionary` | 512 KiB | Dictionary (jisyo) ROM    | `21fcd88c97b881e55f015f22d62002022189572e171f1c5e485b751c84379b30` |
| `subsys`     | 8 KiB   | Floppy sub-CPU (Z80) ROM  | `531ab2aa2c7d7c4deb2ddd8303c6637ea7e273648825fb51e17c8660d7496565` |

## Configuration file

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

## Emulator controls

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

### How do I rebind my keys?

You can remap keys in the configuration file using `key.<HostKey> = <PC-98 Key>` entries.
See [`configuration/default.conf`](configuration/default.conf) for a complete reference of all
available host key names, PC-98 key names, and the default mappings.

## Disk and disc images

### Supported floppy disk image formats

| Format  | Extensions                     | Writable | Description                                        |
|---------|--------------------------------|----------|----------------------------------------------------|
| D88     | `.d88`, `.d98`, `.88d`, `.98d` | Yes      | Standard PC-98 disk image with per-sector metadata |
| HDM     | `.hdm`                         | No       | Headerless raw sector image (2HD only)             |
| NFD     | `.nfd`                         | No       | T98Next format with per-sector metadata            |

Only D88 images preserve modifications written by the emulated software. HDM and NFD images are currently read-only.

### Supported CD-ROM disc image formats

CD-ROM discs apply to the PC-8801 and PC-9821 targets.

| Format  | Extensions | Description                                                                                          |
|---------|------------|------------------------------------------------------------------------------------------------------|
| CUE/BIN | `.cue`     | CUE sheet referencing a raw BIN image                                                                |
| CloneCD | `.ccd`     | CloneCD control file with sibling `.img` (raw 2352-byte sectors) and optional `.sub` subchannel data |

### Multiple disk images

Many games ship on multiple floppy disks and ask you to swap disks during gameplay.
Some CD-ROM games also come as multiple disc images.
neetan handles this by letting you assign several disk images to each drive up front, then swap
between them at runtime.

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

Press `GUI + Alt + F9` (drive 1), `GUI + Alt + F10` (drive 2), or `GUI + Alt + F11` (CD-ROM)
to open the image selector and swap disks at runtime.

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

## Text extraction

When started with `--enable-extractor` (or `enable-extractor = on` in a
configuration file), Neetan observes glyph fetches from the CGROM and
copies on-screen text to the system clipboard, one completed line at a time.
This is intended for use with external machine-translation tools such as
Textractor, Translator++, or other translation tools that watch
the clipboard.

Limitations:

- Only supported by the PC-98 targets only.
- This covers games that render text by reading the CGROM (visual novels
  and similar engines). Titles that draw text solely from the text VRAM
  (most DOS prompts and menus) are not captured yet.
- Characters that the JIS-to-Unicode conversion cannot map (custom
  user-defined glyphs, etc.) are silently dropped.

## FAQ

### Which ROM files do I need for this emulation?

It depends on the emulated family:

- PC-9801 / PC-9821: none. If you have the correct ROM files you CAN use them,
  but there is no particular reason to, since our HLE BIOS has very good
  compatibility.
  We also include a self-created open source font ROM and provide the tools to
  re-create / change it. With these systems in place we can run the vast majority
  of PC-98 games and applications. There are some BIOS extensions, mainly the
  sound API and LIO API that we currently haven't implemented, but outside
  some odd BASIC-based games they should not be used by titles that interface with
  the hardware I/O ports directly.
- PC-8001 / PC-8801: a full ROM set is required and must be supplied via
  `--pc88-roms`. The ROMs are matched by content hash, so file names do not matter;
  see [ROM set](#rom-set) for the list of required ROMs and the extra ROMs that the
  `n80` / `n80sr` boot modes need.
- PC-88VA: a full ROM set is required and must be supplied via `--pc88va-roms`. The
  ROMs are matched by content hash, so file names do not matter; see
  [ROM set](#rom-set-1) for the list of required ROMs.

The optional MT-32 and SC-55 MIDI modules, also need ROM files to function correctly,
as described in the [MIDI sound modules](#midi-sound-modules) section.

### How can I use my mouse?

In games that support a mouse, you first need to capture the mouse pointer via the
right CTRL key. You can release the mouse pointer by clicking the right CTRL key again.

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
