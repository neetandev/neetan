# cpu_68k

A port of the microcode based 68k core from MAME, originally written by Olivier Galibert.

The checked-in `src/m68000_generated.rs` is generated from the vendored MAME 68000 tables.

From this directory, regenerate it with Python 3:

```bash
python3 codegen/m68000gen.py decode codegen/m68000.lst /tmp/m68000-decode.cpp
python3 codegen/m68000gen.py sif codegen/m68000.lst /tmp/m68000-sif.cpp
python3 codegen/transpile_m68000.py /tmp/m68000-sif.cpp /tmp/m68000-decode.cpp src/m68000_generated.rs
cargo +nightly fmt --all
```

## License

This project is licensed under [3-clause BSD](https://opensource.org/license/bsd-3-clause) license.
