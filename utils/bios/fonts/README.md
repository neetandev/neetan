# PC-98 Open Source Font ROM

The V98-format font ROM (`font.rom`) that the PC-98 HLE BIOS uses when no
external font ROM is configured.

This font was created using the [Shinonome Gothic](https://github.com/code4fukui/shinonome-font) bitmap font (public
domain), which provides 8x16 and 16x16 glyphs for JIS X 0201 and JIS X 0208 character sets with some additional
modifications to align them with the original NEC fonts.

NEC-specific characters use glyph data from NP21W and are located under `patches` (BSD 3-Clause).

## Generate the font ROM yourself

The `.bit` files are the committed source. The `utils/bios/Makefile` renders
them into `fonts/font.rom`. To run the generator by hand from the root folder of
this project:

```sh
cargo run --release -p create_font --bin create_font -- \
    --output utils/bios/fonts/font.rom
```

The source paths default to this directory, so only `--output` is required.
