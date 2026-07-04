# PC-88VA2

Select the PC-88VA family with `--machine PC88VA2`. The emulated machine is the
PC-88VA2: an NEC V30 at roughly 8 MHz with 512 KiB of main RAM, a built-in OPNA, and
a custom graphics chipset with the Super Graphic Processor (SGP) blitter. A Z80
sub-CPU at 4 MHz drives the floppy unit.

Only the V3 graphic mode is emulated. This target is not backwards compatible with
the PC-8801 software. The original could run V1 and V2 BASIC games, since its CPU had
an integrated Z80. We don't implement that hybrid nature; if you need to run PC-8801
games, please use the [PC-8801MC target](machine-pc88.md).

Like the PC-8801MC, it has no HLE firmware and requires a real ROM set (see
[ROM set](#rom-set)).

## Platform options

| Option                  | Description                                       | Default |
|-------------------------|---------------------------------------------------|---------|
| `--pc88va-roms <PATH>`  | Directory with the PC-88VA2 ROM set (required)    | -       |

## ROM set

The PC-88VA2 needs a real ROM set, pointed to by `--pc88va-roms`. ROMs are
identified by their BLAKE3 content hash rather than by file name, so any dump layout
works. All six slots (the ROM0 low/high and ROM1 system images, the kanji/font ROM,
the dictionary ROM, and the floppy sub-CPU ROM) are required.

See the [PC-88VA2 ROMs](roms.md#pc-88va2) section for the exact ROM slots and hashes.
