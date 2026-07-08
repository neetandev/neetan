# PC-8001 / PC-8801

Select the PC-88 family with `--machine PC8801MC`. The emulated machine is always
the PC-8801MC; the `--boot-mode` option chooses which BASIC personality it powers up
with, which is how the earlier PC-8001 family is emulated. Unlike the PC-98 targets,
the PC-8801MC has no HLE firmware and requires a real ROM set (see [ROM set](#rom-set)).

Sound is provided by the machine's built-in OPNA (Sound Board II, which should be
fully compatible with the Sound Board I).

## Boot modes

The boot mode is selected with `--boot-mode` and defaults to `v2`. The `v1s`, `v1h`
and `v2` modes are the PC-8801's own modes; the `n`, `n80` and `n80sr` modes emulate
the three generations of the PC-8001 line:

| `--boot-mode` | Alias   | BASIC dialect                 | Emulated machine |
|---------------|---------|-------------------------------|------------------|
| `v1s`         | -       | N88-BASIC V1 (standard speed) | PC-8801          |
| `v1h`         | -       | N88-BASIC V1 (high speed)     | PC-8801          |
| `v2`          | -       | N88-BASIC V2 (default)        | PC-8801          |
| `n`           | -       | N-BASIC                       | PC-8001          |
| `n80`         | `n80v1` | N80-BASIC                     | PC-8001mkII      |
| `n80sr`       | `n80v2` | N80SR-BASIC                   | PC-8001mkIISR    |

## N / N80 / N80SR are not interchangeable

The three N-family modes correspond to three different generations of real hardware:

* `n` - the original PC-8001 (1979) running plain N-BASIC.
* `n80` - the PC-8001mkII (1983) running N80-BASIC.
* `n80sr` - the PC-8001mkIISR (1985) running N80SR-BASIC.

While each generation broadly builds on the previous one, they are not a clean
superset chain and are not fully compatible with each other. Pick the boot mode that
matches the machine the software was written for.

The PC-8801 modes (`v1s`, `v1h`, `v2`) are separate modes again and are not
compatible with the N-family modes. The vast majority of PC-88 games will target the
`v2` mode, which is the default.

## Platform options

| Option                      | Description                                                 | Default           |
|-----------------------------|-------------------------------------------------------------|-------------------|
| `--boot-mode <MODE>`        | Boot mode: `v1s`, `v1h`, `v2`, `n`, `n80`, `n80sr`          | `v2`              |
| `--pc88-roms <PATH>`        | Directory with the PC-8801MC ROM set (required)             | -                 |
| `--monitor <MODE>`          | Monitor timing: `auto`, `15k` (200-line), `24k` (400-line)  | `auto`            |
| `--pc88-memory-wait <MODE>` | Memory wait states: `fast` or `compatible`                  | derives from mode |
| `--pc88-8mhz-wait <MODE>`   | 8 MHz wait mode: `fast` or `compatible`                     | `fast`            |
| `--cpu-mode <MODE>`         | CPU speed: `low` or `high`                                  | derives from mode |

The defaults of `--cpu-mode` and `--pc88-memory-wait` derive from the boot mode: the
`v1s` and N-family modes (`n`, `n80`, `n80sr`) default to the slower, compatible
behavior of the machines they emulate, while `v1h` and `v2` keep the fast defaults.
Pass the flags explicitly to override.

## ROM set

The PC-8801MC needs a real ROM set, pointed to by `--pc88-roms`. ROMs are identified
by their BLAKE3 content hash rather than by file name, so any dump layout works.

The N88-BASIC ROM and its extension banks, the N-BASIC ROM, the dictionary and kanji
ROMs, the disk sub-CPU ROM, and the CD-ROM interface BIOS are always required. The
`n80_mkii`, `n80_mkiisr` and `n80sr` ROMs are needed only when the matching
`--boot-mode` is selected.

See the [PC-8001 / PC-8801 ROMs](roms.md#pc-8001--pc-8801) section for the exact ROM
slots and hashes.
