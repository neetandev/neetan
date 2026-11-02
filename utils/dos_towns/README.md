# FM Towns HLE OS ROM

This directory holds the sources of the HLE OS ROM of the FM Towns (`osrom.rom`, 512 KiB).
The real `IO.SYS` of a boot disc loads its DOS kernel and its programs from this ROM. The
image holds only stubs of Neetan. NeetanDos (`crates/dos`) does the work behind them.

## Files

- `msdos.asm` - the `MSDOS.SYS` stub (`7800h` bytes). `IO.SYS` reads it from the fixed OS ROM
  offset `14800h` and SYSINIT far-calls it as the DOS kernel. It traps into NeetanDos and returns
  to SYSINIT. It also carries the `INT 21h` entry fragment that CTL.SYS searches at `11D2h`.
- `mscdex.asm` - the `MSCDEX.EXE` stub. It traps into the HLE MSCDEX of NeetanDos with its command
  tail and stays resident with its PSP, or ends with the exit code of NeetanDos.
- `src/` - `create_osrom`, the image builder (`dos_towns` crate). It writes the FAT12 volume
  with the layout of the real OS ROM: the `IPL4` boot record, the volume label `OS-ROM`,
  `MSDOS.SYS` from cluster `4Eh`, `COMMAND.COM` and `MSCDEX.EXE`.
- `osrom.rom` - the generated image, checked in like the other BIOS images.

`COMMAND.COM` is the shell stub `utils/dos/dos.rom`. NeetanDos runs its Rust shell for every
EXEC of `COMMAND.COM`.

## Build

The build needs [nasm](https://www.nasm.us/) and cargo:

```bash
make -C utils/dos_towns
```

The Makefile assembles `msdos.sys` and `mscdex.exe` and runs the builder:

```bash
cargo run --release -p dos_towns --bin create_osrom -- \
    --msdos msdos.sys --command ../dos/dos.rom --mscdex mscdex.exe --output osrom.rom
```

`msdos.sys` and `mscdex.exe` are intermediate files. Only the sources and `osrom.rom` are
checked in.

## Licence

The sources in this directory (the assembly sources, the builder, the Makefile) are licensed
under the [MIT No Attribution (MIT-0)](https://opensource.org/license/mit-0) licence, see
`LICENSE`.
