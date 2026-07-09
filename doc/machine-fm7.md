# Fujitsu FM-7 / FM-77AV

Select the Fujitsu FM-7 family with `--machine` set to `FM7` or `FM77AV`. Both are
dual-CPU machines with two MC68B09 processors (`--cpu-mode` is ignored):

| Machine | `--machine` | CPU / Clock                         | Main RAM | Sound              | Drive default |
|---------|-------------|-------------------------------------|----------|--------------------|---------------|
| FM-7    | `FM7`       | 2x MC68B09, 1.8 MHz main, 2 MHz sub | 64 KiB   | AY-3-8910 + buzzer | 5.25" 2D      |
| FM-77AV | `FM77AV`    | 2x MC68B09, 1.8 MHz main, 2 MHz sub | 128 KiB  | YM2203 + buzzer    | 5.25" 2D      |

The FM-7 (1982) pairs a main CPU running F-BASIC 3.0 from ROM with a dedicated
sub CPU that owns the video hardware: 640x200 graphics in three bit planes with
a digital 8-color palette. The two processors communicate through a 128-byte
shared RAM window and a HALT/BUSY handshake. Sound comes from the AY-3-8910 PSG
and the 1200 Hz buzzer; the JIS level-1 kanji ROM is fitted.

The FM-77AV (1985) runs the same software while adding a memory management
register (MMR) with a 256 KiB address space, a second VRAM page with an analog
palette and a 320x200 4096-color mode, the MB61VH010 graphics ALU with hardware
line drawing, bankable sub monitors, a serial keyboard encoder with a real-time
clock, and the YM2203 OPN (FM 3ch + SSG 3ch) in place of the PSG. It boots
through an initiator ROM that seeds the boot RAM before starting F-BASIC.

Cassette tape images are inserted with `--cassette` (`.t77`); the floppy drives
use the shared `--fdd1` / `--fdd2` options with D77/D88 images. These targets
have no HLE firmware and require a real ROM set (see [ROM set](#rom-set)).

## Platform options

| Option               | Description                                          | Default |
|----------------------|------------------------------------------------------|---------|
| `--fm7-roms <PATH>`  | Directory with the FM-7 / FM-77AV ROM set (required) | -       |
| `--boot-mode <MODE>` | Boot mode: `basic`, `dos`                            | `basic` |
| `--cassette <PATH>`  | Cassette tape image (`.t77`)                         | -       |

`--boot-mode` selects the boot path. `basic` boots the BASIC-mode boot ROM;
`dos` boots the DOS-mode boot ROM for DOS-style IPL media. On the FM-77AV the
same modes choose the initiator's boot path instead of a physical ROM.

Floppy images are supplied with the shared `--fdd1` / `--fdd2` options.

## ROM set

The FM-7 targets need a real ROM set, pointed to by `--fm7-roms`. ROMs are
identified by their BLAKE3 content hash rather than by file name, so any dump
layout works. The FM-7 requires the F-BASIC 3.0 ROM, both boot ROMs, and the
type-C sub monitor; its kanji ROM slot is optional. The FM-77AV requires the
initiator ROM, the F-BASIC 3.0 ROM, all three sub monitors plus the sub CG
font, and the kanji ROM. Both MAME sets (`fm7`, `fm77av`) can share the
directory.

See the [Fujitsu FM-7 ROMs](roms.md#fujitsu-fm-7--fm-77av) section for the
exact ROM slots, per-model requirements, and hashes.
