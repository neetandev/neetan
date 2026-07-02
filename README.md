# Neetan (ねーたん)

An emulator for the PC-6001, PC-6601, PC-8001, PC-8801, PC-88VA, PC-9801 and PC-9821
written in Rust.

[Game Compatibility](https://github.com/neetandev/neetan/wiki)

## Supported systems

Neetan emulates four distinct families, selected through the `--machine` option:

* PC-9801 / PC-9821 line
* PC-8001 / PC-8801 line
* PC-88VA line
* PC-6001 / PC-6601 line

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

### PC-6001 / PC-6601

Neetan emulates five PC-6000 targets, all built around a single Z80:

| Machine       | `--machine`   | CPU / Clock   | RAM    | Sound      | Voice   | Built-in FDD |
|---------------|---------------|---------------|--------|------------|---------|--------------|
| PC-6001       | `PC6001`      | Z80 ~4 MHz    | 16 KiB | AY-3-8910  | No      | No           |
| PC-6001mkII   | `PC6001MK2`   | Z80 ~4 MHz    | 64 KiB | AY-3-8910  | uPD7752 | No           |
| PC-6601       | `PC6601`      | Z80 ~4 MHz    | 64 KiB | AY-3-8910  | uPD7752 | 5.25"        |
| PC-6001mkIISR | `PC6001MK2SR` | Z80 ~3.58 MHz | 64 KiB | YM2203 OPN | uPD7752 | No           |
| PC-6601SR     | `PC6601SR`    | Z80 ~3.58 MHz | 64 KiB | YM2203 OPN | uPD7752 | 3.5"         |

The early models use the AY-3-8910 PSG, while the SR models use a YM2203 (OPN). All
but the PC-6001 add the uPD7752 voice synthesizer. The PC-6601 and PC-6601SR have a
built-in floppy drive. Cartridge and cassette images can be inserted as well. Like
the PC-8801MC, none of the PC-6000 targets have HLE firmware; they require a real ROM
set supplied via `--pc6000-roms`.

See [PC-6001 / PC-6601 systems](#pc-6001--pc-6601-systems) for the ROM set and
platform-specific options.

## Usage

```bash
neetan [OPTIONS]
neetan <COMMAND>
```

### Options

The `System` column shows where an option applies: `All` (every family), `PC-98`
(PC-9801 / PC-9821 only), `PC-9821` (PC-9821 only), `PC-88` (PC-8001 / PC-8801
only), `PC-88VA` (PC-88VA only), or `PC-6000` (PC-6001 / PC-6601 only). Options that
apply to one family are ignored on the others.

| Option                       | System  | Description                                                                                                                                              | Default           |
|------------------------------|---------|----------------------------------------------------------------------------------------------------------------------------------------------------------|-------------------|
| `-c, --config <PATH>`        | All     | Load configuration from file                                                                                                                             | -                 |
| `--machine <TYPE>`           | All     | `PC9801F`, `PC9801VM`, `PC9801VX`, `PC9801RA`, `PC9821AS`, `PC9821AP`, `PC8801MC`, `PC88VA2`, `PC6001`, `PC6001MK2`, `PC6601`, `PC6001MK2SR`, `PC6601SR` | `PC9801RA`        |
| `--cpu-mode <MODE>`          | All     | CPU speed mode: `low` or `high` (PC-88 default derives from the boot mode)                                                                               | `high` (PC-98)    |
| `--boot-mode <MODE>`         | PC-88   | BASIC boot mode: `v1s`, `v1h`, `v2`, `n`, `n80`, `n80sr`                                                                                                 | `v2`              |
| `--pc88-monitor <MODE>`      | PC-88   | Monitor timing: `auto`, `15k`, `24k`                                                                                                                     | `auto`            |
| `--pc88-memory-wait <MODE>`  | PC-88   | Memory wait: `fast` or `compatible`                                                                                                                      | derives from mode |
| `--pc88-8mhz-wait <MODE>`    | PC-88   | 8 MHz wait: `fast` or `compatible`                                                                                                                       | `fast`            |
| `--pc88-roms <PATH>`         | PC-88   | Directory with the PC-8801MC ROM set (required for `PC8801MC`)                                                                                           | -                 |
| `--pc88va-roms <PATH>`       | PC-88VA | Directory with the PC-88VA2 ROM set (required for `PC88VA2`)                                                                                             | -                 |
| `--pc6000-roms <PATH>`       | PC-6000 | Directory with the PC-6000 ROM set (required for the PC-6000 targets)                                                                                    | -                 |
| `--pc6000-phase <0-3>`       | PC-6000 | Initial composite artifact-color phase; swaps the fake-color pair Mode 4 titles rely on.                                                                 | `0`               |
| `--fdd1 <PATH>`              | All     | Floppy disk image for drive 1 (repeatable)                                                                                                               | -                 |
| `--fdd2 <PATH>`              | All     | Floppy disk image for drive 2 (repeatable)                                                                                                               | -                 |
| `--hdd1 <PATH>`              | All     | Hard disk image for hard disk drive 1                                                                                                                    | -                 |
| `--hdd2 <PATH>`              | All     | Hard disk image for hard disk drive 2                                                                                                                    | -                 |
| `--cdrom <PATH>`             | PC-9821 | CD-ROM disc image .cue or .ccd file (repeatable)                                                                                                         | -                 |
| `--cartridge <PATH>`         | PC-6000 | Cartridge ROM image to insert                                                                                                                            | -                 |
| `--cassette <PATH>`          | PC-6000 | Cassette tape image to insert (`.cas`, `.p6`, `.p6t`)                                                                                                    | -                 |
| `--audio-volume <FLOAT>`     | All     | Audio volume 0.0-1.0                                                                                                                                     | `1.0`             |
| `--aspect-mode <MODE>`       | All     | Display aspect mode: `4:3` or `1:1`                                                                                                                      | `4:3`             |
| `--crt <on\|off>`            | All     | Enable the CRT effect. Not available when using the legacy backend.                                                                                      | `on`              |
| `--scaling <MODE>`           | All     | Scaling method: `nearest`, `bilinear`, `pixelart`                                                                                                        | `pixelart`        |
| `--backend <BACKEND>`        | All     | Rendering backend: `modern` or `legacy`                                                                                                                  | `modern`          |
| `--window-mode <MODE>`       | All     | Window mode: `windowed` or `fullscreen`                                                                                                                  | `windowed`        |
| `--force-gdc-clock <2.5\|5>` | PC-98   | Force GDC clock to 2.5 or 5 MHz. VX and later only                                                                                                       | auto              |
| `--graphicboard <TYPE>`      | PC-98   | Graphics accelerator board: `none`, `ga1280a`                                                                                                            | `none`            |
| `--pc98-roms <PATH>`         | PC-98   | Directory with the PC-98 ROM set (BIOS, font, sound), matched by content hash. All ROMs optional                                                         | -                 |
| `--bios`                     | PC-98   | Boot the real BIOS from `--pc98-roms` instead of the HLE BIOS. Ignored (warns) on PC-9821                                                                | HLE BIOS          |
| `--soundboard <TYPE>`        | PC-98   | Sound board: `none`, `14`, `26k`, `86`, `86+26k`, `sb16`, `sb16+26k`                                                                                     | `86+26k`          |
| `--adpcm-ram <on\|off>`      | PC-98   | ADPCM RAM option for the PC-9801-86                                                                                                                      | `on`              |
| `--ems <on\|off>`            | PC-98   | Enable EMS expanded memory                                                                                                                               | `on`              |
| `--xms <on\|off>`            | PC-98   | Enable XMS extended memory                                                                                                                               | `on`              |
| `--midi <DEVICE>`            | PC-98   | MIDI device: `none`, `mt32`, `sc55`                                                                                                                      | `none`            |
| `--mt32-roms <PATH>`         | PC-98   | Path to MT-32 ROM directory (requires `mt32` feature)                                                                                                    | -                 |
| `--sc55-roms <PATH>`         | PC-98   | Path to SC-55 ROM directory (requires `sc55` feature)                                                                                                    | -                 |
| `--boot-device <DEVICE>`     | All     | Boot device: `auto`, `fdd1`, `fdd2`, `hdd1`, `hdd2`, `dos`                                                                                               | `auto`            |
| `--printer <PATH>`           | All     | Output file for printer (must exist)                                                                                                                     | -                 |
| `--enable-extractor`         | All     | Copy on-screen Japanese text to the system clipboard, one line at a time                                                                                 | off               |
| `-h, --help`                 | All     | Print help                                                                                                                                               | -                 |
| `-V, --version`              | All     | Print version                                                                                                                                            | -                 |

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

| Option                       | Description                                                           | Default  |
|------------------------------|-----------------------------------------------------------------------|----------|
| `--soundboard <TYPE>`        | Sound board: `none`, `14`, `26k`, `86`, `86+26k`, `sb16`, `sb16+26k`  | `86+26k` |
| `--adpcm-ram <on\|off>`      | ADPCM RAM option for the PC-9801-86                                   | `on`     |
| `--ems <on\|off>`            | Enable EMS expanded memory                                            | `on`     |
| `--xms <on\|off>`            | Enable XMS extended memory                                            | `on`     |
| `--force-gdc-clock <2.5\|5>` | Force GDC clock to 2.5 or 5 MHz (VX and later only)                   | auto     |
| `--graphicboard <TYPE>`      | Graphics accelerator board: `none`, `ga1280a`                         | `none`   |
| `--pc98-roms <PATH>`         | Directory with the PC-98 ROM set (BIOS, font, sound), by content hash | -        |
| `--bios`                     | Boot the real BIOS from `--pc98-roms` instead of the HLE BIOS         | HLE BIOS |

CD-ROM disc images (`--cdrom`) are supported on the PC-9821 targets only.

### ROM set

Unlike the other families, the PC-98 targets run on a built-in HLE BIOS and a
built-in font by default, so a ROM set is mostly optional. Point `--pc98-roms` at
a directory of dumps and pass `--bios` to boot the model's real BIOS instead of the
HLE BIOS. ROMs are identified by their BLAKE3 content hash rather than by file name,
so any dump layout works regardless of how the files are named; the directory is
scanned non-recursively.

If a game doesn't boot or has strange errors, then you might need to use a real PC-98
BIOS file. Please open an issue in such cases, since outside BASIC games, we want to
have full compatibility with the HLE BIOS. 

With `--bios` the model's BIOS is required. The PC-9821 targets are the exception:
they have currently no real-BIOS boot path and always fall back to HLE with a warning.
The 26K sound ROM is loaded when a PC-9801-26K board is selected. A font ROM is best-effort:
the model's preferred dump is used when present, otherwise the built-in font is kept.

BIOS ROM (192 KiB dual-bank image, one per model):

| Model      | Size    | BLAKE3                                                             |
|------------|---------|--------------------------------------------------------------------|
| `PC9801F`  | 192 KiB | `5587b89b968b005e81ea2bb4c2ef6fc762154d589e627920e3d9be9cd3e01b06` |
| `PC9801VM` | 192 KiB | `4377eeba8410c57f9a313ed2d24cd929cbfb7cac40244d5c6cafd1a27bf3495e` |
| `PC9801VX` | 192 KiB | `89ff271aa046bb6428761cdc3ec92d82e87350c5a4941974293c5b7fe2238aed` |
| `PC9801RA` | 192 KiB | `f18e91e8097661efe4543f30558383a02021047acfaa6d0a78e06d025094aa5e` |
| `PC9821AS` | -       | HLE only (no real BIOS)                                            |
| `PC9821AP` | -       | HLE only (no real BIOS)                                            |

Font ROM (V98 format, 282 KiB). Any of these dumps is accepted for any model; each
model just prefers the one matching its family:

| Dump         | Preferred by     | BLAKE3                                                             |
|--------------|------------------|--------------------------------------------------------------------|
| standard     | F / VM / VX / RA | `4b6f751f34e633e072ded2a109c25ddb90ac70350792dc55914a4cefa4dbe005` |
| PC-9821As    | `PC9821AS`       | `a567134a3d5c2a215b9573ee07b5204fff243631052e7a40be340e863aff8eef` |
| PC-9821Ap2   | `PC9821AP`       | `7fb96af345c33f9bd7be5c22f75c650ac41da9b543ca5f9ca7b3d3906f2abb40` |
| PC-9801UX    | fallback         | `3c1efa858b80fc11bb7482bdc5e15004dd9a015d7d22d48159cd43ed63f540dc` |
| PC-9821Ce2   | fallback         | `b38096265c76cf9f54cb47df905cfb6c8b4d4f27019a04835bbc3dc8782d33e1` |

Sound ROM (loaded when a PC-9801-26K board is selected):

| Slot    | Size   | BLAKE3                                                             |
|---------|--------|--------------------------------------------------------------------|
| `sound` | 16 KiB | `93816a6e42ed9a10135af634ed500e10b1d266e0b4158d3f8471910609255e24` |

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
[ROM set](#rom-set-1)).

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

Like the PC-8801MC, it has no HLE firmware and requires a real ROM set (see [ROM set](#rom-set-2)).

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

## PC-6001 / PC-6601 systems

Select the PC-6000 family with `--machine` set to one of `PC6001`, `PC6001MK2`,
`PC6601`, `PC6001MK2SR` or `PC6601SR`. All five are single-Z80 machines: the early
models run at roughly 4 MHz with the AY-3-8910 PSG, while the SR models run at the
3.58 MHz NTSC colorburst clock with a YM2203 (OPN) and 64 KiB of work RAM reached
through bank-switched 8 KiB paging.
Every model except the PC-6001 adds the uPD7752 voice synthesizer. The PC-6601 and
PC-6601SR have a built-in floppy drive driven directly by the main CPU.

Cartridge and cassette images can be inserted with `--cartridge` and `--cassette`
(`.cas`, `.p6`, `.p6t`); the floppy drives use the shared `--fdd1` / `--fdd2`
options. Like the PC-8801MC, none of these targets have HLE firmware and they require
a real ROM set (see [ROM set](#rom-set-3)).

### Platform options

| Option                 | Description                                   | Default |
|------------------------|-----------------------------------------------|---------|
| `--pc6000-roms <PATH>` | Directory with the PC-6000 ROM set (required) | -       |
| `--cartridge <PATH>`   | Cartridge ROM image to insert                 | -       |
| `--cassette <PATH>`    | Cassette tape image (`.cas`, `.p6`, `.p6t`)   | -       |
| `--pc6000-phase <0-3>` | Initial composite artifact-color phase        | `0`     |

### ROM set

The PC-6000 targets need a real ROM set, pointed to by `--pc6000-roms`. ROMs are
identified by their BLAKE3 content hash rather than by file name, so any dump layout
works regardless of how the files are named. Each model requires its boot ROM (BASIC
or, on the SR models, the system ROM) and its base character generator; the kanji,
extended character generator, and voice ROMs are loaded when present. Several dumps
are bit-identical across models (the kanji ROM, the SR system ROM halves), so a single
file can satisfy more than one slot.

| Slot          | Size   | Contents                          | Required for                            | BLAKE3                                                             |
|---------------|--------|-----------------------------------|-----------------------------------------|--------------------------------------------------------------------|
| `basic60`     | 16 KiB | PC-6001 BASIC                     | `PC6001`                                | `13bc0696487984f7836f094312b64fb0702dcb5ac3b941a79bd6f174e657697d` |
| `basic62`     | 32 KiB | PC-6001mkII BASIC                 | `PC6001MK2`                             | `d951eae886dec98a063e5fb11e12b0385f5dd4617c0546fe7cf9fd77b17ae41c` |
| `basic66`     | 32 KiB | PC-6601 BASIC                     | `PC6601`                                | `d9eaf3e5e6cb1f71db527e6eeadf7a1968f8a558234b74c6812198c588ae46d1` |
| `basic68`     | 32 KiB | PC-6601SR mkII-compat BASIC       | `PC6601SR` (optional)                   | `c4901a2149f3c8e65d3db78bbf3776fc2d963f270152923ba920274d44a0224b` |
| `system1`     | 64 KiB | SR system ROM, first half         | `PC6001MK2SR`, `PC6601SR`               | `6ca4e747c8b17307a77150441e5d8721d5c242fcc8b8ef35737d3f5edf6e2d74` |
| `system2`     | 64 KiB | SR system ROM, second half        | `PC6001MK2SR`, `PC6601SR`               | `998a90c4bd0bf4ae4a600a0d94f3eca96c3b8db754311ce1c8029126dbcf0a9a` |
| `subsys`      | 8 KiB  | SR sub / disk ROM                 | `PC6601SR` (optional)                   | `becb7c1502d41a9f160b651e142044610ffa172a8bbf47eaa11aa0086953a080` |
| `cg60`        | 4 KiB  | PC-6001 base character generator  | `PC6001`                                | `f537afe76997ec4f8b377a29771f45c39414a25f7e071d2d38b143cdd8bee7bc` |
| `cg62`        | 8 KiB  | PC-6001mkII base CG               | `PC6001MK2`                             | `581f6d2db80386732ed09706ad3b8961f8b77b7ea024e65cec37e56ad2adf07c` |
| `cg66`        | 8 KiB  | PC-6601 base CG                   | `PC6601`                                | `63829a1c32924a77f85716f445c445ab7be178c4438cfd8cf6ffaff5731a0965` |
| `cg68base`    | 8 KiB  | PC-6601SR base CG                 | `PC6001MK2SR`, `PC6601SR` (optional)    | `24e524d4938809a87720f98abfba71c8e9162d742c67a167d8b87566cc1d4258` |
| `cgext`       | 8 KiB  | Extended CG (mkII / 6601)         | `PC6001MK2`, `PC6601` (optional)        | `ba0dd650539dd3fdbf63da36982b41bfda8f4c2ea0dcda2c1c2ac56427ee26ed` |
| `cg68ext`     | 8 KiB  | PC-6601SR extended CG             | `PC6001MK2SR`, `PC6601SR` (optional)    | `067c732525260eadfcfecbb9fc4ef9535c0c2f77caa049453bf2ab992ec3fca3` |
| `cg68`        | 16 KiB | SR native CG                      | `PC6001MK2SR`, `PC6601SR`               | `b49b056ca06bd0c2253e6db0806969787a6fca4fc78228728422c9cf63f1e472` |
| `kanji`       | 32 KiB | Kanji font ROM                    | mkII and later (optional)               | `f0af53e54b1b09b229d03efc9f65e65597a0c4f6aa9e3e7c0e553274ccd481fb` |
| `voice62`     | 16 KiB | uPD7752 voice data (PC-6001mkII)  | `PC6001MK2` (optional)                  | `633e73f55479bee65ed344d818a35b15ab109f188ad5c09826c066d6ec2596c5` |
| `voice66`     | 16 KiB | uPD7752 voice data (PC-6601)      | `PC6601` (optional)                     | `88a747147725fd618668e07744b05f34288b4454698d6182c4db2e680c7b76d0` |
| `voice68`     | 16 KiB | uPD7752 voice data (SR models)    | `PC6001MK2SR`, `PC6601SR` (optional)    | `8ed4a9a3e9ae2e4aa0fccc0f170081f3f61c09e293812b7973a7ab9c23e22b68` |

## Configuration file

Instead of passing all options on the command line, you can use a configuration file with `-c` or `--config`:

```bash
neetan --config my_game.cfg
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

Right Ctrl is reserved as the emulator's shortcut modifier.
The emulated machine uses Left Ctrl.

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

Press `Right Ctrl + 1` (drive 1), `Right Ctrl + 2` (drive 2), or `Right Ctrl + 3` (CD-ROM)
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

### How can I use my mouse?

In games that support a mouse, you first need to capture the mouse pointer via
`Right Ctrl + M`. You can release the mouse pointer by pressing `Right Ctrl + M` again.

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
