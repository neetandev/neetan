# PC-98 fonts

The `.bit` files are the sources of the PC-98 font ROM (`font.rom`) in the V98 format.

The glyphs reproduce the original NEC PC-98 hardware typeface. `utils/LEGAL_FONT.md` explains
why Japanese law does not protect typefaces of this kind.

## Files

| File                    | Cell  | ENCODING | Content                                       |
|-------------------------|-------|----------|-----------------------------------------------|
| `pc98_ank_8x8.bit`      | 8x8   | ANK code | 8x8 ANK font                                  |
| `pc98_ank_8x16.bit`     | 8x16  | ANK code | 8x16 ANK font                                 |
| `pc98_16_nonkanji.bit`  | 16x16 | JIS code | rows `21h`-`27h` and `2Ch`-`2Dh`              |
| `pc98_16_halfwidth.bit` | 8x16  | JIS code | rows `29h`-`2Bh`, bytes `00h`-`0Fh` of a cell |
| `pc98_16_kanji1.bit`    | 16x16 | JIS code | level 1 kanji, rows `30h`-`4Fh`               |
| `pc98_16_kanji2.bit`    | 16x16 | JIS code | level 2 kanji, rows `50h`-`73h`               |
| `pc98_16_extension.bit` | 16x16 | JIS code | NEC selected IBM extension, rows `79h`-`7Ch`  |

`ENCODING` is decimal. A JIS code is `H * 256 + L`.

## Licence

The MIT-0 licence of `../LICENSE` covers the `.bit` files as a format and a container. It grants
no rights in the NEC PC-98 hardware typeface.
