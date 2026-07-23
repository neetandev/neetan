# PC/AT HLE BIOS CP437 Fonts

The three CP437 bitmap fonts embedded in the HLE VGA BIOS stub ROM
(`utils/bios_at/vgabios.asm`):

- `vga_8x16.bit` - 8x16 VGA text mode font (Px437 IBM VGA 8x16)
- `ega_8x14.bit` - 8x14 EGA font (Px437 IBM EGA 8x14)
- `bios_8x8.bit` - 8x8 BIOS/CGA font (Px437 IBM BIOS 8x8)

The glyphs come from "The Ultimate Oldschool PC Font Pack" v2.2 by VileR
(https://int10h.org/oldschool-pc-fonts/), rendered to bitmaps by the pcface
project by Susam Pal (https://github.com/susam/pcface). They are faithful
recreations of the IBM PC ROM fonts, licensed under CC BY-SA 4.0 (see
`LICENSE`).

## Generate the raw glyph binaries yourself

The `.bit` files are the committed source. The `utils/bios_at/Makefile`
renders them into the raw glyph binaries (`fonts/*.bin`) that `vgabios.asm`
includes, using:

```sh
cargo run --release -p create_font --bin bios_at_fonts -- \
    --input utils/bios_at/fonts/vga_8x16.bit --height 16 \
    --output utils/bios_at/fonts/font_8x16.bin
```
