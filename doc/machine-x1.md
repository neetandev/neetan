# Sharp X1 / X1 turbo

Select the Sharp X1 family with `--machine` set to `X1` or `X1TURBO`. Both are
single-Z80 machines running at a fixed 4 MHz (`--cpu-mode` is ignored):

| Machine    | `--machine` | CPU / Clock | RAM              | Sound              | Drive default |
|------------|-------------|-------------|------------------|--------------------|---------------|
| Sharp X1   | `X1`        | Z80A 4 MHz  | 64 KiB           | AY-3-8910          | 5.25" 2D      |
| X1 turbo   | `X1TURBO`   | Z80A 4 MHz  | 16 x 64 KiB      | AY-3-8910 + YM2151 | 5.25" 2D      |

The base X1 (CZ-800C) offers 640x200 and 320x200 graphics with a digital 8-color
palette, a programmable character generator (PCG), text attributes, and the
AY-3-8910 PSG. The keyboard, cassette transport, and real-time clock are handled by
the machine's sub-CPU, which the emulator provides as a high-level emulation.

The X1 turbo (CZ-850C) is a strict hardware superset of the base X1 and still runs
all base-X1 software. It adds sixteen 64 KiB work-RAM banks, a Z80 DMA controller
that drives the floppy controller, a Z80 SIO (RS-232C on channel 0 and the mouse on
channel 1), a kanji ROM with a kanji text VRAM plane, the 400-line 24 kHz hi-res
video mode, and a hi-speed PCG mode. The turbo is fitted with the CZ-8BS1 FM sound
board (YM2151 OPM), which software detects through the board's detection port; the
base X1 has the PSG only.

To use the mouse, capture the pointer with `Right Ctrl + M` (see
[Emulator controls](../README.md#emulator-controls)).

Cassette tape images are inserted with `--cassette` (`.tap`); the floppy drives use
the shared `--fdd1` / `--fdd2` options with D88 or raw `.2d` images, and a SASI hard
disk attaches through the shared `--hdd1` option. None of these targets have HLE
firmware, and they require a real ROM set (see [ROM set](#rom-set)).

## Platform options

| Option                  | Description                                    | Default |
|-------------------------|------------------------------------------------|---------|
| `--x1-roms <PATH>`      | Directory with the Sharp X1 ROM set (required) | -       |
| `--x1-keyboard <A\|B>`  | X1 turbo keyboard mode switch                  | `A`     |
| `--monitor <MODE>`      | Monitor timing: `auto`, `15k`, `24k` (turbo)   | `auto`  |
| `--cassette <PATH>`     | Cassette tape image (`.tap`)                   | -       |

The X1 turbo keyboard has a physical A/B mode switch. Mode A is the standard
layout; mode B rearranges the kana assignments and lets games read the key matrix
directly through the sub-CPU's game-key command. The base X1 keyboard has no
switch and always behaves like mode A.

`--monitor` selects the monitor timing reported to software. In `auto`, the X1
turbo reports a 24 kHz monitor so hi-res software runs at 400 lines; `15k` forces
a 15 kHz (200-line) monitor and `24k` forces a 24 kHz (400-line) one. The base X1
has no hi-res mode, so the setting only matters on the turbo.

Floppy images are supplied with the shared `--fdd1` / `--fdd2` options and a SASI
hard disk image with `--hdd1`.

## ROM set

The Sharp X1 targets need a real ROM set, pointed to by `--x1-roms`. ROMs are
identified by their BLAKE3 content hash rather than by file name, so any dump layout
works. Each model requires its IPL boot ROM, its 8x8 character generator, and the
8x16 ANK font; the turbo additionally requires the four kanji ROMs. Both MAME sets
(`x1`, `x1turbo`) can share the directory.

See the [Sharp X1 ROMs](roms.md#sharp-x1) section for the exact ROM slots,
per-model requirements, and hashes.
