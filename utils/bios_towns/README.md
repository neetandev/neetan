# FM Towns HLE system ROM

This directory holds the sources of the HLE system ROM of the FM Towns (`sysrom.rom`, 256 KiB)
and of the FM Towns font ROM (`fonts/font.rom`, 256 KiB).

## Files

- `sysrom.asm` - the system ROM image mapped at `FFFC0000h`. It holds the 12-dot font area, the
  system icons and the code segment `FC00h` with the HLE entry points.
- `icons.txt` - the art of the 128 system icons (Neetan art).
- `fonts/` - the `.bit` sources of the FM Towns fonts and the generated font ROM, see
  `fonts/README.md`.
- `Makefile` - renders `icons.bin` and `font12.bin` and assembles `sysrom.rom`. It also builds
  `fonts/font.rom`.

## System ROM layout

| Image offset      | Address     | Content                                                     |
|-------------------|-------------|-------------------------------------------------------------|
| `00000h`-`1AFFFh` | `FFFC0000h` | 12-dot font area: 8x12 ANK font, 12x12 JIS font from `C00h` |
| `1B000h`-`27FFFh` | `FFFDB000h` | `FFh`                                                       |
| `28000h`-`2FFFFh` | `FFFE8000h` | system icons, 256 bytes per icon                            |
| `30000h`-`3BFFFh` | `FFFF0000h` | `FFh`                                                       |
| `3C000h`-`3FFFFh` | `FFFFC000h` | code segment `FC00h`                                        |

## Build

The build needs [nasm](https://www.nasm.us/) and cargo:

```bash
make -C utils/bios_towns
```

`icons.bin` and `font12.bin` are intermediate files.

## Licence

The MIT-0 licence in `LICENSE` covers the source files in this directory. It grants no rights in
the FM Towns hardware typeface. See `utils/LEGAL_FONT.md`.
