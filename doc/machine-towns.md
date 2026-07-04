# FM Towns

Select the FM Towns family with `--machine FMTownsIICX` or `--machine FMTownsIIMX`.
Both are 32-bit machines sharing the Towns chipset; they differ mainly in CPU and
clock:

| Machine        | `--machine`   | CPU      | CPU Speed   | Sound              | CD-ROM |
|----------------|---------------|----------|-------------|--------------------|--------|
| FM Towns II CX | `FMTownsIICX` | 80386DX  | 16 / 20 MHz | YMF276 OPN2 + PCM  | Yes    |
| FM Towns II MX | `FMTownsIIMX` | 80486DX2 | 33 / 66 MHz | YMF276 OPN2 + PCM  | Yes    |

For both targets `--cpu-mode low` selects the lower clock and `--cpu-mode high` (the
default) the higher one: 16 / 20 MHz on the CX and 33 / 66 MHz on the MX. The 386 and
486 cores are optimized for emulation speed and most likely run a bit fast compared
to their real counterparts.

Sound is provided by the built-in YMF276 (OPN2) FM synthesizer and the RF5C68 PCM
sound source. Both targets have a built-in CD-ROM drive, a SCSI interface, a floppy
drive, and support the 2-button and 6-button Towns game pads. MIDI sound modules can
be used as well; see [MIDI sound modules](#midi-sound-modules).

Like the PC-8801MC, the FM Towns targets have no HLE firmware and require a real ROM
set (see [ROM set](#rom-set)).

## Platform options

| Option                     | Description                                                     | Default |
|----------------------------|-----------------------------------------------------------------|---------|
| `--towns-roms <PATH>`      | Directory with the FM Towns ROM set (required)                  | -       |
| `--towns-pad <2\|6>`       | Game pad type: `2` (2-button) or `6` (6-button)                 | `6`     |
| `--cdrom-compat <on\|off>` | Slow/compatible CD-ROM drive timing                             | `off`   |

CD-ROM disc images are supplied with the shared `--cdrom` option (repeatable), and
floppy images with `--fdd1` / `--fdd2`.

## ROM set

The FM Towns targets need a real ROM set, pointed to by `--towns-roms`. Both targets
use the FM Towns II MX ROM dump. Two layouts are accepted: a merged set (the packed
2 MiB MAME BIOS image plus the 32-byte serial ROM) or a split set (the five
individual images plus the serial ROM).

See the [FM Towns ROMs](roms.md#fm-towns) section for the exact files and hashes.

## MIDI sound modules

The FM Towns reaches a MIDI sound module through RS-MIDI over its RS-232C port.
Two modules are supported:

* Roland MT-32 - using a Rust port of [munt](https://github.com/munt/munt)
* Roland SC-55 - using a Rust port of [Nuked-SC55](https://github.com/nukeykt/Nuked-SC55)

Both require external ROM files. Set the device and point at the ROM directory:

```bash
neetan --machine FMTownsIIMX --midi mt32 --mt32-roms /path/to/mt32_roms [options...]
neetan --machine FMTownsIIMX --midi sc55 --sc55-roms /path/to/sc55_roms [options...]
```

Or in a configuration file:

```ini
midi = sc55
sc55-roms = /path/to/sc55_roms
```

MIDI emulation is only activated when both the `--midi` device and the corresponding
ROM path are set, so you can keep the ROM paths in your global config and toggle
per-game by changing only `--midi`.

See [MIDI: Roland MT-32](roms.md#midi-roland-mt-32) and
[MIDI: Roland SC-55](roms.md#midi-roland-sc-55) for the ROM files. Both modules are
optional build features; see [Build requirements](../README.md#build-requirements)
and [License](../README.md#license).
