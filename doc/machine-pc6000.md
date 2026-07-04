# PC-6001 / PC-6601

Select the PC-6000 family with `--machine` set to one of `PC6001`, `PC6001MK2`,
`PC6601`, `PC6001MK2SR` or `PC6601SR`. All five are single-Z80 machines:

| Machine       | `--machine`   | CPU / Clock   | RAM    | Sound      | Voice   | Built-in FDD |
|---------------|---------------|---------------|--------|------------|---------|--------------|
| PC-6001       | `PC6001`      | Z80 ~4 MHz    | 16 KiB | AY-3-8910  | No      | No           |
| PC-6001mkII   | `PC6001MK2`   | Z80 ~4 MHz    | 64 KiB | AY-3-8910  | uPD7752 | No           |
| PC-6601       | `PC6601`      | Z80 ~4 MHz    | 64 KiB | AY-3-8910  | uPD7752 | 5.25"        |
| PC-6001mkIISR | `PC6001MK2SR` | Z80 ~3.58 MHz | 64 KiB | YM2203 OPN | uPD7752 | No           |
| PC-6601SR     | `PC6601SR`    | Z80 ~3.58 MHz | 64 KiB | YM2203 OPN | uPD7752 | 3.5"         |

The early models run at roughly 4 MHz with the AY-3-8910 PSG, while the SR models run
at the 3.58 MHz NTSC colorburst clock with a YM2203 (OPN) and 64 KiB of work RAM
reached through bank-switched 8 KiB paging. Every model except the PC-6001 adds the
uPD7752 voice synthesizer. The PC-6601 and PC-6601SR have a built-in floppy drive
driven directly by the main CPU.

Cartridge and cassette images can be inserted with `--cartridge` and `--cassette`
(`.cas`, `.p6`, `.p6t`); the floppy drives use the shared `--fdd1` / `--fdd2`
options. Like the PC-8801MC, none of these targets have HLE firmware, and they require
a real ROM set (see [ROM set](#rom-set)).

## Platform options

| Option                 | Description                                   | Default |
|------------------------|-----------------------------------------------|---------|
| `--pc6000-roms <PATH>` | Directory with the PC-6000 ROM set (required) | -       |
| `--cartridge <PATH>`   | Cartridge ROM image to insert                 | -       |
| `--cassette <PATH>`    | Cassette tape image (`.cas`, `.p6`, `.p6t`)   | -       |
| `--pc6000-phase <0-3>` | Initial composite artifact-color phase        | `0`     |

The `--pc6000-phase` option sets the initial composite artifact-color phase; it swaps
the fake-color pair that Mode 4 titles rely on. The phase can also be cycled at
runtime with `Right Ctrl + P`.

## ROM set

The PC-6000 targets need a real ROM set, pointed to by `--pc6000-roms`. ROMs are
identified by their BLAKE3 content hash rather than by file name, so any dump layout
works. Each model requires its boot ROM (BASIC or, on the SR models, the system ROM)
and its base character generator. The kanji, extended character generator, and voice
ROMs are loaded when present. Several dumps are bit-identical across models, so a
single file can satisfy more than one slot.

See the [PC-6001 / PC-6601 ROMs](roms.md#pc-6001--pc-6601) section for the exact ROM
slots, per-model requirements, and hashes.
