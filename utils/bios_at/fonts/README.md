# PC/AT fonts

The `.bit` files are the sources of the font tables of the HLE VGA BIOS (`../vgabios.rom`) and of
the HLE system BIOS (`../bios.rom`).

The glyphs reproduce the original typefaces of the Tseng Labs ET4000 VGA BIOS and of the AMI
system BIOS. `utils/LEGAL_FONT.md` explains why the law of the United States does not protect
typefaces of this kind.

## Files

| File                  | Cell | Content                                  |
|-----------------------|------|------------------------------------------|
| `at_vga_8x8.bit`      | 8x8  | 8x8 font of the VGA BIOS                 |
| `at_vga_8x14.bit`     | 8x14 | 8x14 font of the VGA BIOS                |
| `at_vga_8x16.bit`     | 8x16 | 8x16 font of the VGA BIOS                |
| `at_vga_9x14_alt.bit` | 8x14 | 9x14 alternate glyphs of the VGA BIOS    |
| `at_vga_9x16_alt.bit` | 8x16 | 9x16 alternate glyphs of the VGA BIOS    |
| `at_system_8x8.bit`   | 8x8  | 8x8 font of the system BIOS, `00h`-`7Fh` |

`ENCODING` is decimal and gives the character code. An alternate file lists its glyphs in
ascending code order. Its table ends with the code byte `00h`.

## Licence

The MIT-0 licence of `../LICENSE` covers the `.bit` files as a format and a container. It grants
no rights in the Tseng Labs and AMI typefaces.
