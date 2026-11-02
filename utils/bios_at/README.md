# PC/AT HLE BIOS ROMs

This directory holds the sources of the HLE system BIOS ROM of the PC/AT (`bios.rom`, 64 KiB) and
of the HLE VGA BIOS ROM (`vgabios.rom`, 32 KiB).

## Files

- `bios.asm` - the system BIOS ROM image mapped at `F0000h`. It holds the HLE stubs of the BIOS
  interrupt handlers, the 8x8 font and the reset vector.
- `vgabios.asm` - the VGA BIOS option ROM image mapped at `C0000h`. It holds the fonts and the
  video parameter tables.
- `fonts/` - the `.bit` sources of the PC/AT fonts, see `fonts/README.md`.
- `Makefile` - renders the raw font tables and assembles `bios.rom` and `vgabios.rom`.

## System BIOS ROM layout

| Image offset      | Address  | Content                                                   |
|-------------------|----------|-----------------------------------------------------------|
| `00000h`-`00370h` | `F0000h` | metadata, HLE stubs, parameter tables, vector table       |
| `00371h`-`0FA6Dh` | `F0371h` | `FFh`                                                     |
| `0FA6Eh`-`0FE6Dh` | `FFA6Eh` | 8x8 font, codes `00h`-`7Fh`                               |
| `0FE6Eh`-`0FFEFh` | `FFE6Eh` | `FFh`                                                     |
| `0FFF0h`-`0FFFFh` | `FFFF0h` | reset vector, BIOS date at `FFF5h`, model byte at `FFFEh` |

## VGA BIOS ROM layout

| Image offset    | Address  | Content                                                   |
|-----------------|----------|-----------------------------------------------------------|
| `0000h`-`000Fh` | `C0000h` | option ROM header                                         |
| `0010h`-`004Ch` | `C0010h` | metadata words, ID string, functionality table at `003Dh` |
| `004Dh`-`084Ch` | `C004Dh` | 8x8 font                                                  |
| `084Dh`-`164Ch` | `C084Dh` | 8x14 font                                                 |
| `164Dh`-`264Ch` | `C164Dh` | 8x16 font                                                 |
| `264Dh`-`2779h` | `C264Dh` | 9x14 alternate glyphs                                     |
| `277Ah`-`28BDh` | `C277Ah` | 9x16 alternate glyphs                                     |
| `28BEh`-`28BFh` | `C28BEh` | `90h`, alignment padding                                  |
| `28C0h`-`301Bh` | `C28C0h` | video parameter table and video save pointer table        |
| `301Ch`-`7FFFh` | `C301Ch` | `00h`, checksum byte at `7FFFh`                           |

## Build

The build needs [nasm](https://www.nasm.us/) and cargo:

```bash
make -C utils/bios_at
```

The raw font tables (`vga_*.bin`, `system_8x8.bin`) are intermediate files.

## Licence

The MIT-0 licence in `LICENSE` covers the source files in this directory. It grants no rights in
the Tseng Labs and AMI typefaces. See `utils/LEGAL_FONT.md`.
