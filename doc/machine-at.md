# IBM PC/AT (DOS/V)

Select the PC/AT family with `--machine AT486DX50` or `--machine AT486DX66`.
Both are i486DX2 on the Chips & Technologies CS4031 chipset, period
correct for the DOS/V era (roughly 1990 to the release of Windows 95). They
differ only in clock:

| Machine      | `--machine` | CPU     | Bus / core clock | RAM    | VGA                    |
|--------------|-------------|---------|------------------|--------|------------------------|
| AT 486DX2-50 | `AT486DX50` | i486DX2 | 25 MHz / 50 MHz  | 16 MiB | Tseng ET4000AX (1 MiB) |
| AT 486DX2-66 | `AT486DX66` | i486DX2 | 33 MHz / 66 MHz  | 16 MiB | Tseng ET4000AX (1 MiB) |

RAM is fixed at 16 MiB on both. `--cpu-mode high` (the default) runs the doubled
core clock (50 MHz on the DX50, 66 MHz on the DX66); `--cpu-mode low` disables the
clock doubler and runs at the bus clock (25 and 33 MHz). Off-chip I/O and VGA VRAM
accesses are paced at the fixed ISA bus clock (about 8.33 MHz), so I/O-bound
timing loops run at credible speed on both variants.

DOS/V needs no Japanese-specific hardware: Japanese text is rendered entirely in
software into VGA graphics modes, so any good VGA PC/AT clone qualifies. The
machine boots a built-in HLE system BIOS and HLE VGA BIOS by default, so no ROM
files are needed. Pass `--bios` with `--at-roms` to boot a real ROM set instead
(see [ROM set](#rom-set)). There is no HLE DOS: you install a real DOS/V from
floppy images (IBM PC DOS J5.0x/V through PC DOS 7.0/V, and MS-DOS 5.0/V and
6.2/V).

## Installed hardware

| Card / device           | Notes                                                    |
|-------------------------|----------------------------------------------------------|
| Tseng ET4000AX SVGA     | 1 MiB, full VGA plus the Tseng extensions and SVGA modes |
| Sound Blaster 16        | CT1741 DSP, CT1745 mixer, YMF262 OPL3, at 0x220 / 0x388  |
| MPU-401 (0x330)         | Intelligent and UART MIDI, IRQ 9                         |
| Game port (0x200-0x207) | Analog, two 2-axis / 2-button sticks                     |
| Floppy controller       | uPD765, IRQ 6, DMA 2, 3-mode capable                     |
| IDE primary / secondary | Two hard disks; ATAPI CD-ROM on the secondary channel    |
| COM1 16450 UART (0x3F8) | Serial mouse, IRQ 4                                      |
| PC speaker              | PIT channel 2 through port 0x61                          |

## Platform options

| Option             | Description                                                        | Default   |
|--------------------|--------------------------------------------------------------------|-----------|
| `--at-roms <PATH>` | Directory with the `ct486` and `et4000` ROM sets, by content hash  | -         |
| `--bios`           | Boot the real BIOS from `--at-roms` instead of the HLE BIOS        | HLE BIOS  |

Floppy images attach with `--fdd1` / `--fdd2`, hard disks with `--hdd1` /
`--hdd2` (flat `.hdd` images with an MBR partition table), and CD-ROM images with
`--cdrom`. Both BIOSes expose two boot orders. `--boot-device auto` and
`--boot-device fdd1` select `A: then C:`, while `--boot-device hdd1` selects
`C: then A:`.

## Media

Floppy images are raw sector images (`.img`) whose geometry is detected from the
file size, plus the existing container formats where their geometry matches:

| Format                     | Geometry            | Size (bytes) |
|----------------------------|---------------------|--------------|
| 5.25" 2D (360 KB)          | 40x2x9x512          | 368,640      |
| 3.5" 2DD (720 KB)          | 80x2x9x512          | 737,280      |
| 5.25" 2HD (1.2 MB)         | 80x2x15x512         | 1,228,800    |
| 3.5" 2HD 3-mode (1.23 MB)  | 77x2x8x1024         | 1,261,568    |
| 3.5" 2HD (1.44 MB)         | 80x2x18x512         | 1,474,560    |
| 3.5" 2HD IBM XDF (1.84 MB) | mixed sectors       | 1,884,160    |

The 3-mode 1.23 MB format is the PC-98-style geometry that Japanese DOS/V machines
read through their 3-mode drives. IBM XDF is the high-capacity distribution format
of PC DOS 7.0. Note the extension clash with the X68000 raw 1.23 MB `.xdf`: the two
are told apart by exact file size.

Hard disks are raw flat images with an MBR partition table. For example, create a
100 MB image with:

```bash
./neetan create-hdd dosv.hdd --type at100
```

The available sizes are `at40`, `at100`, `at250`, and `at504`. CUE/BIN and
CloneCD images are exposed as an ATAPI device on the secondary IDE channel. The
guest uses a real ATAPI driver plus the real MSCDEX shipped with DOS/V.

## MIDI sound modules

The MPU-401 drives an external MT-32 or SC-55, exactly as on the other machines:

* Roland MT-32 - using a Rust port of [munt](https://github.com/munt/munt)
* Roland SC-55 - using a Rust port of [Nuked-SC55](https://github.com/nukeykt/Nuked-SC55)

Both require external ROM files. Set the device and point at the ROM directory:

```bash
neetan --machine AT486DX66 --midi mt32 --mt32-roms /path/to/mt32_roms [options...]
neetan --machine AT486DX66 --midi sc55 --sc55-roms /path/to/sc55_roms [options...]
```

See [MIDI: Roland MT-32](roms.md#midi-roland-mt-32) and
[MIDI: Roland SC-55](roms.md#midi-roland-sc-55) for the ROM files. Both modules
are optional build features; see [Build requirements](../README.md#build-requirements)
and [License](../README.md#license).

## ROM set

The PC/AT target runs without any ROM files: the built-in HLE system BIOS and HLE
VGA BIOS cover the POST, the boot sequence, INT 10h/13h/14h/15h/16h/17h/1Ah and
the VGA services, including the video parameter table published through
SAVE_PTR (BDA 40:A8).

A real ROM set is optional and selected with `--bios` plus `--at-roms`: the AMI
CS4031 system BIOS (`chips_1.ami`) and the Tseng ET4000AX VGA BIOS
(`et4000.bin`, or the alternate ColorImage `cvet4kax.bin`). Matching is by
content hash, so file names do not matter. See the
[PC/AT (DOS/V)](roms.md#pcat-dosv) ROM section for the exact files and hashes.
