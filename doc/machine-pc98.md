# PC-9801 / PC-9821

Select a PC-98 machine with `--machine`; the default is `PC9801RA`. neetan aims to
support all 16-bit era DOS games and emulates them accurately for 6 idealized
machine targets:

| Machine   | `--machine` | CPU      | CPU Speed   | FPU (x87) | Extended RAM | Graphics | Interface | CD-ROM |
|-----------|-------------|----------|-------------|-----------|--------------|----------|-----------|--------|
| PC-9801F  | `PC9801F`   | 8086     | 5 / 8 MHz   | No        | None         | GDC      | SASI      | No     |
| PC-9801VM | `PC9801VM`  | V30      | 8 / 10 MHz  | No        | None         | GRCG     | SASI      | No     |
| PC-9801VX | `PC9801VX`  | 80286    | 8 / 10 MHz  | No        | 4 MiB        | EGC      | SASI      | No     |
| PC-9801RA | `PC9801RA`  | 80386DX  | 16 / 20 MHz | Yes       | 12 MiB       | EGC      | SASI      | No     |
| PC-9821AS | `PC9821AS`  | 80486DX  | 33 MHz      | Yes       | 14 MiB       | PEGC     | IDE       | Yes    |
| PC-9821AP | `PC9821AP`  | 80486DX2 | 66 MHz      | Yes       | 14 MiB       | PEGC     | IDE       | Yes    |

All targets have the full 640 KiB conventional RAM. The 8086 and V30 are cycle-count
accurately emulated. The emulated 286 is calibrated to run at the same speed as the
original 286 using trace data. The 386 and the 486 are optimized for emulation speed
and most likely run a bit fast compared to their real counterparts.

The default for the CLI is the PC-9801RA machine with the PC-9801-86 + PC-9801-26k
combo soundboards.

Games of the PC-98 normally do not require any ROM files, with the rare exception of
games that used the PC-98's N88 (86) BASIC ROM.

## Sound cards

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

## Platform options

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

## ROM set

Unlike the other families, the PC-98 targets run on a built-in HLE BIOS and a
built-in font by default, so a ROM set is mostly optional. Point `--pc98-roms` at a
directory of dumps and pass `--bios` to boot the model's real BIOS instead of the
HLE BIOS. ROMs are identified by their BLAKE3 content hash rather than by file name,
so any dump layout works.

If a game doesn't boot or has strange errors, then you might need to use a real
PC-98 BIOS file. Please open an issue in such cases, since outside BASIC games we
want to have full compatibility with the HLE BIOS.

With `--bios` the model's BIOS is required. The PC-9821 targets are the exception:
they have no real-BIOS boot path and always fall back to HLE with a warning. The 26K
sound ROM is loaded when a PC-9801-26K board is selected; a font ROM is best-effort.

See the [PC-9801 / PC-9821 ROMs](roms.md#pc-9801--pc-9821) section for the exact
BIOS, font, and sound ROM hashes.

## Graphic acceleration board

neetan can emulate the I-O DATA GA-1280A graphics accelerator board, the high-end
variant to the GA-1024A. All GA-1024A software is compatible with the GA-1280A.

The board is primarily useful for Windows 3.1, where it unlocks higher resolutions
than the stock EGC/PEGC video paths (up to 1600x1024 pixel).

Enable it on the CLI with `--graphicboard ga1280a`, or in a configuration file with
`graphicboard = ga1280a`. The default is `none`.

Official I-O DATA drivers for MS-DOS, Windows 3.1, and Windows 95 are still
available from the manufacturer:
<https://www.iodata.jp/lib/software/g/106.htm#MS-DOS>

## MIDI sound modules

The PC-98 reaches a MIDI sound module through the emulated MPU-PC98II interface (a
C-Bus MPU-401). Two modules are supported:

* Roland MT-32 - using a Rust port of [munt](https://github.com/munt/munt)
* Roland SC-55 - using a Rust port of [Nuked-SC55](https://github.com/nukeykt/Nuked-SC55)

Both require external ROM files. Set the device and point at the ROM directory:

```bash
neetan --midi mt32 --mt32-roms /path/to/mt32_roms [other options...]
neetan --midi sc55 --sc55-roms /path/to/sc55_roms [other options...]
```

Or in a configuration file:

```ini
midi = mt32
mt32-roms = /path/to/mt32_roms
```

MIDI emulation is only activated when both the `--midi` device and the corresponding
ROM path are set, so you can keep the ROM paths in your global config and toggle
per-game by changing only `--midi`.

See [MIDI: Roland MT-32](roms.md#midi-roland-mt-32) and
[MIDI: Roland SC-55](roms.md#midi-roland-sc-55) for the ROM files. Both modules are
optional build features; see [Build requirements](../README.md#build-requirements)
and [License](../README.md#license).

## Text extraction

When started with `--enable-extractor` (or `enable-extractor = on` in a
configuration file), neetan observes glyph fetches from the CGROM and copies
on-screen text to the system clipboard, one completed line at a time. This is
intended for use with external machine-translation tools such as Textractor,
Translator++, or other translation tools that watch the clipboard.

Text extraction is supported by the PC-98 targets only.

Limitations:

- This covers games that render text by reading the CGROM (visual novels and similar
  engines). Titles that draw text solely from the text VRAM (most DOS prompts and
  menus) are not captured yet.
- Characters that the JIS-to-Unicode conversion cannot map (custom user-defined
  glyphs, etc.) are silently dropped.
