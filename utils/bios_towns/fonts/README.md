# FM Towns fonts

The `.bit` files are the sources of the FM Towns font ROM (`font.rom`) and of the 12-dot font
area of the HLE system ROM (`../sysrom.rom`, offsets `00000h`-`1AFFFh`).

The glyphs reproduce the original Fujitsu FM Towns hardware typeface. `utils/LEGAL_FONT.md`
explains why Japanese law does not protect typefaces of this kind.

## Files

| File                    | Cell  | ENCODING       | Content                                              |
|-------------------------|-------|----------------|------------------------------------------------------|
| `towns_16_nonkanji.bit` | 16x16 | JIS code       | rows `21h`-`28h`                                     |
| `towns_16_kanji1.bit`   | 16x16 | JIS code       | level 1 kanji, rows `30h`-`4Fh`                      |
| `towns_16_kanji2.bit`   | 16x16 | JIS code       | level 2 kanji, rows `50h`-`74h`                      |
| `towns_16_fujitsu.bit`  | 16x16 | ROM cell index | glyphs at the cells `300h`-`3FFh`                    |
| `towns_ank_8x8.bit`     | 8x8   | ANK code       | 8x8 ANK font                                         |
| `towns_ank_8x16.bit`    | 8x16  | ANK code       | 8x16 ANK font                                        |
| `towns_12_ank.bit`      | 8x12  | ANK code       | 8x12 ANK font of the 12-dot area                     |
| `towns_12_jis.bit`      | 12x12 | JIS code       | 12x12 non-kanji and level 1 kanji of the 12-dot area |

`ENCODING` is decimal. A JIS code is `H * 256 + L`.

## Licence

The MIT-0 licence of `../LICENSE` covers the `.bit` files as a format and a container. It grants
no rights in the FM Towns hardware typeface.
