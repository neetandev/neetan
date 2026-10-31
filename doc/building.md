# Building

## Requirements

* [Rust 1.97](https://rustup.rs/)
* [SDL3](https://github.com/libsdl-org/SDL) (see the
  [sdl3_sys description](https://docs.rs/sdl3-sys/latest/sdl3_sys/#usage))
* Linux: the [mold linker](https://github.com/rui314/mold)

Build a release binary with:

```bash
cargo build --release
```

The binaries are `target/release/neetan` and `target/release/neetan-auto`.

## SDL3

The `sdl3-sys` crate finds SDL3 in one of several ways. The build features
`sdl3-build-from-source`, `sdl3-use-pkg-config` and `sdl3-use-vcpkg` select the
method. The release builds use `sdl3-build-from-source`:

```bash
cargo build --release --features sdl3-build-from-source
```

## Build features

Every machine family and both frontends are selectable at build time. All of them are
on by default. A plain `cargo build` produces the full emulator.

| Feature        | Builds                                              |
|----------------|-----------------------------------------------------|
| `gui`          | The `neetan` binary and its SDL3 frontend           |
| `auto`         | The `neetan-auto` binary and its Scheme interpreter |
| `all-machines` | All ten machine families                            |
| `pc98`         | NEC PC-9801 / PC-9821                               |
| `pc88`         | NEC PC-8001 / PC-8801                               |
| `pc88va`       | NEC PC-88VA2                                        |
| `pc60`         | NEC PC-6001 / PC-6601                               |
| `msx`          | MSX / MSX2 / MSX2+                                  |
| `towns`        | Fujitsu FM Towns                                    |
| `fm7`          | Fujitsu FM-7 / FM-77AV                              |
| `x68k`         | Sharp X68000                                        |
| `x1`           | Sharp X1                                            |
| `at`           | IBM PC/AT (DOS/V)                                   |
| `mt32`         | Roland MT-32 emulation                              |
| `sc55`         | Roland SC-55 emulation                              |

Examples:

```bash
cargo build --no-default-features --features gui,msx        # MSX only, GUI only
cargo build --no-default-features --features auto,pc98      # PC-98 only, headless only
```

## MIDI features and binary license

The Roland MT-32 and SC-55 emulations are optional build features. Both are enabled
by default. Both use a copyleft licenses and change the license of the resulting
binary:

| Build configuration       | Binary license   |
|---------------------------|------------------|
| Without `mt32` and `sc55` | BSD 3-Clause     |
| `mt32` enabled            | LGPL 2.1         |
| `sc55` enabled            | GPL 2.0 or later |
| `mt32` and `sc55` enabled | GPL 2.0 or later |

Examples:

```bash
# neither
cargo build --release --no-default-features --features gui,auto,all-machines
# MT-32 only
cargo build --release --no-default-features --features gui,auto,all-machines,mt32
# SC-55 only
cargo build --release --no-default-features --features gui,auto,all-machines,sc55
```
