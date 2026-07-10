# Sharp X68000

Select the Sharp X68000 family with `--machine X68000`, `--machine X68000SUPER` or
`--machine X68000XVI`. All three are MC68000-based machines sharing the X68000
chipset; they differ in CPU clock, internal storage controller and IPL version:

| Machine      | `--machine`   | Model   | CPU clock         | Interface | CD-ROM |
|--------------|---------------|---------|-------------------|-----------|--------|
| X68000       | `X68000`      | CZ-600C | 10 MHz            | SASI      | No     |
| X68000 SUPER | `X68000SUPER` | CZ-604C | 10 MHz            | SCSI      | Yes    |
| X68000 XVI   | `X68000XVI`   | CZ-634C | 16.7 MHz / 10 MHz | SCSI      | Yes    |

On the XVI, `--cpu-mode high` (the default) selects the 16.7 MHz clock and
`--cpu-mode low` the 10 MHz front-panel setting. The other two models always run
at 10 MHz. All three configurations use the platform maximum of 12 MiB main RAM.

Sound is provided by the built-in YM2151 (OPM) FM synthesizer and the MSM6258
ADPCM voice source. All targets have two 5.25-inch 2HD floppy drives and a hard
disk interface: the original X68000 attaches SASI disks, while the SUPER and XVI
attach SCSI disks and a SCSI CD-ROM drive with CDDA playback. The keyboard, the
Sharp mouse (capture with `Right Ctrl + M`), and two two-button joystick ports
are supported. MIDI sound modules can be used as well; see
[MIDI sound modules](#midi-sound-modules).

Floppy images use the D88 (`.d88`), DIM (`.dim`) or raw XDF (`.xdf`, `.2hd`)
containers through the shared `--fdd1` / `--fdd2` options. Hard-disk images are
headerless X68000 `.hdf` files (created with `./neetan create-hdd`) supplied
with `--hdd1` / `--hdd2`, and CD-ROM disc images use `--cdrom` (SUPER/XVI only,
repeatable).

The X68000 targets have no HLE firmware and require a real ROM set (see
[ROM set](#rom-set)).

## Platform options

| Option               | Description                                        | Default |
|----------------------|----------------------------------------------------|---------|
| `--x68k-roms <PATH>` | Directory with the X68000 ROM set (required)       | -       |
| `--cpu-mode <MODE>`  | XVI clock: `high` (16.7 MHz) or `low` (10 MHz)     | `high`  |

## ROM set

The X68000 targets need a real ROM set, pointed to by `--x68k-roms`. Each model
selects only its own IPL and SCSI boot images; there is no fallback to another
model's IPL. The XVI accepts the Compact-XVI SCSI ROM dump as a named
compatibility substitute and prints a startup warning when it is used.

See the [Sharp X68000 ROMs](roms.md#sharp-x68000) section for the exact files,
sizes and hashes.

## MIDI sound modules

The X68000 reaches a MIDI sound module through an emulated CZ-6BM1 (YM3802)
MIDI board, installed automatically when a MIDI device is selected. Two modules
are supported:

* Roland MT-32 - using a Rust port of [munt](https://github.com/munt/munt)
* Roland SC-55 - using a Rust port of [Nuked-SC55](https://github.com/nukeykt/Nuked-SC55)

Both require external ROM files. Set the device and point at the ROM directory:

```bash
neetan --machine X68000 --midi mt32 --mt32-roms /path/to/mt32_roms [options...]
neetan --machine X68000 --midi sc55 --sc55-roms /path/to/sc55_roms [options...]
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
[MIDI: Roland SC-55](roms.md#midi-roland-sc-55) for the ROM files. Both modules
are optional build features; see
[Build requirements](../README.md#build-requirements) and
[License](../README.md#license).
