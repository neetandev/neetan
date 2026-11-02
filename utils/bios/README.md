# PC-98 HLE BIOS ROM

This directory holds the sources of the HLE BIOS ROM of the PC-98 (`bios.rom`, 96 KiB) and of
the PC-98 font ROM (`fonts/font.rom`, 282 KiB).

## Files

- `bios.asm` - the BIOS ROM image mapped at `E8000h`. It holds the HLE stubs of the BIOS
  interrupt handlers and the reset vector.
- `fonts/` - the `.bit` sources of the PC-98 fonts and the generated font ROM, see
  `fonts/README.md`.
- `Makefile` - assembles `bios.rom` and builds `fonts/font.rom`.

## BIOS ROM layout

| Image offset      | Address  | Content                                                          |
|-------------------|----------|------------------------------------------------------------------|
| `00000h`-`00DD7h` | `E8000h` | N88-BASIC entry traps                                            |
| `00DD8h`-`157FFh` | `E8DD8h` | model-specific ROM data area, DOS 3.3 probe code at `0D38Eh`     |
| `15800h`-`17FEFh` | `FD800h` | code segment `FD80h`: entry point table, HLE stubs, vector table |
| `17FF0h`-`17FFFh` | `FFFF0h` | reset vector                                                     |

## Build

The build needs [nasm](https://www.nasm.us/) and cargo:

```bash
make -C utils/bios
```

## Licence

The MIT-0 licence in `LICENSE` covers the source files in this directory. It grants no rights in
the NEC PC-98 hardware typeface. See `utils/LEGAL_FONT.md`.
