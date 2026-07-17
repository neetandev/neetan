# MSX / MSX2 / MSX2+

Select the MSX family with `--machine` set to `MSX`, `MSX2`, or `MSX2PLUS`.
All three are single-Z80 machines running at the fixed Japanese NTSC clock.

| Machine | `--machine` | Model         | CPU / Clock   | Main RAM | VRAM    | Video   | Sound                   | Drives |
|---------|-------------|---------------|---------------|----------|---------|---------|-------------------------|--------|
| MSX     | `MSX`       | Sony HB-201   | Z80 ~3.58 MHz | 64 KiB   | 16 KiB  | TMS9118 | YM2149 + Y8950          | None   |
| MSX2    | `MSX2`      | Sony HB-F1XD  | Z80 ~3.58 MHz | 512 KiB  | 128 KiB | V9938   | YM2149 + Y8950          | 1      |
| MSX2+   | `MSX2PLUS`  | Sony HB-F1XDJ | Z80 ~3.58 MHz | 512 KiB  | 128 KiB | V9958   | YM2149 + YM2413 + Y8950 | 1      |

The HB-201 provides the first-generation MSX memory and slot layout, 64 KiB of
linear RAM, TMS9118 video, a Japanese keyboard, the YM2149 PSG, and the Sony
Personal Data Bank ROM.

The HB-F1XD adds the V9938 with 128 KiB of VRAM, 512 KiB of memory-mapper RAM,
expanded slots, the S1985 system controller, an RP5C01 real-time clock, a numeric
keypad, and a Sony WD2793-class 3.5-inch floppy drive.

The HB-F1XDJ adds the V9958 and its horizontal scroll and YJK/YAE video modes,
built-in MSX-MUSIC through the YM2413, a 256 KiB Kanji ROM, and Sony's banked
firmware mapper. It retains the HB-F1XD memory size and disk system.

All three targets install the Panasonic FS-CA1 MSX-AUDIO expansion internally.
It provides a Y8950 with FM, ADPCM RAM, timers, and interrupt support.

To use the mouse, capture the pointer with `Right Ctrl + M` (see
[Emulator controls](../README.md#emulator-controls)).

## Platform options

| Option               | Description                                | Default |
|----------------------|--------------------------------------------|---------|
| `--msx-roms <PATH>`  | Directory with the MSX ROM sets (required) | -       |
| `--cartridge <PATH>` | One cartridge ROM image                    | -       |
| `--cassette <PATH>`  | Cassette image (`.cas`)                    | -       |

MSX2 and MSX2+ floppy images use the shared `--fdd1` option with raw `.dsk`
images. The emulated Sony machines have one built-in drive. Supplying
several `--fdd1` values registers all disks for runtime swapping. The MSX has
no disk controller.

Cassette images use the fMSX `.cas` block format. BASIC cassette programs are
normally loaded with `CLOAD` and started with `RUN`. Binary programs may specify
their own loading command.

## Cartridges

The front end exposes one game cartridge through `--cartridge`. Mapper selection
is automatic and cannot be overridden from the command line or configuration
file. Exact BLAKE3 identities are used where known, followed by ROM-header and
conservative write-address heuristics.

Supported layouts include plain and mirrored ROMs, Konami and Konami SCC,
ASCII8 and ASCII16, their SRAM variants, Koei and Wizardry SRAM, Game Master 2,
R-Type, Cross Blaim, Harry Fox, Super Lode Runner, Super Swangi, Majutsushi,
Synthesizer, FM-PAC, MSX-DOS2, Halnote, MSX-Write, Nettou Yakyuu, PlayBall, and
SCC+ sound cartridges. Known Snatcher and SD Snatcher disks select their
required SCC+ sound cartridge internally and map it to physical cartridge slot
2. Select SCC+ and cartridge slot 2 when Snatcher asks for the sound cartridge
configuration.

Battery-backed cartridge data is stored beside the ROM by replacing its
extension with `.sav`. The save is loaded automatically on insertion and flushed
when the cartridge is ejected or the emulator exits.

## ROM set

The MSX targets need real ROM sets, pointed to by `--msx-roms`. ROMs are
identified by their BLAKE3 content hash rather than by file name, so the four
MAME sets can share one flat directory:

| Target | MAME ROM sets      |
|--------|--------------------|
| MSX    | `hb201`, `fsca1`   |
| MSX2   | `hbf1xd`, `fsca1`  |
| MSX2+  | `hbf1xdj`, `fsca1` |

See the [MSX ROMs](roms.md#msx) section for the exact ROM slots, per-model
requirements, accepted FS-CA1 variants, and BLAKE3 digests.
