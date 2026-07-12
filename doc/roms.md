# ROMs

- [How ROM loading works](#how-rom-loading-works)
- [Which MAME ROM set do I need?](#which-mame-rom-set-do-i-need)
- [Example directory layout](#example-directory-layout)
- [Example configuration](#example-configuration)
- [Build script](#build-script)
- [PC-9801 / PC-9821](#pc-9801--pc-9821)
- [PC-8001 / PC-8801](#pc-8001--pc-8801)
- [PC-88VA2](#pc-88va2)
- [PC-6001 / PC-6601](#pc-6001--pc-6601)
- [FM Towns](#fm-towns)
- [Sharp X68000](#sharp-x68000)
- [Sharp X1](#sharp-x1)
- [Fujitsu FM-7 / FM-77AV](#fujitsu-fm-7--fm-77av)
- [MIDI: Roland MT-32](#midi-roland-mt-32)
- [MIDI: Roland SC-55](#midi-roland-sc-55)

## How ROM loading works

neetan loads its ROMs from officially MAME ROM sets or ROMS of real machine
that were correctly dumped and have the same BLAKE3 hash. You do not need to
merge, split, or rename anything: extract the set(s) for your machine into a
directory and point the matching `--*-roms` option at it.

Every machine points its ROM option at a single directory. neetan scans that
directory (non-recursively), reads each file, and matches it by BLAKE3 content hash
against the tables below. File names do not matter and stray or unrelated files
are simply ignored, so you can drop a whole MAME set (or several sets) into one
directory and neetan picks out the files it needs.

There is one exception where file names are used: every Roland SC-55 set is matched
by file name (see that section).

| Machine            | Option           | Required?                        |
|--------------------|------------------|----------------------------------|
| PC-9801 / PC-9821  | `--pc98-roms`    | Optional (HLE BIOS by default)   |
| PC-8001 / PC-8801  | `--pc88-roms`    | Required                         |
| PC-88VA2           | `--pc88va-roms`  | Required                         |
| PC-6001 / PC-6601  | `--pc6000-roms`  | Required                         |
| FM Towns           | `--towns-roms`   | Required                         |
| Sharp X68000       | `--x68k-roms`    | Required                         |
| Sharp X1           | `--x1-roms`      | Required                         |
| FM-7 / FM-77AV     | `--fm7-roms`     | Required                         |
| Roland MT-32       | `--mt32-roms`    | Required for `--midi mt32`       |
| Roland SC-55       | `--sc55-roms`    | Required for `--midi sc55`       |

## Which MAME ROM set do I need?

MAME ROM sets are named by a short identifier (the set below). Get the set from a
MAME ROM collection matching your MAME version, then extract it into the directory
for that machine.

| Machine / model       | MAME set(s)                               | Notes                                                                        |
|-----------------------|-------------------------------------------|------------------------------------------------------------------------------|
| PC-9801F              | `pc9801f`                                 | BIOS assembled from `urm01-02`..`urm06-02`                                   |
| PC-9801VM             | `pc9801vm`                                | BIOS assembled from the `cpu_board_*` chips                                  |
| PC-9801VX             | `pc9801vx`                                | BIOS assembled from the four `..._yll0x` chips; ships the font ROM           |
| PC-9801RS / PC-9801RA | `pc9801rs`                                | The `pc9801rs` set supplies the BIOS for both models; ships the font ROM     |
| PC-9821AS / PC-9821AP | -                                         | HLE BIOS only; their fonts are not part of any MAME set (built-in font used) |
| PC-8801MC             | `pc8801mc` (+ `pc8001mk2`, `pc8001mk2sr`) | The two `pc8001*` sets only add the optional N80 boot-mode ROMs              |
| PC-88VA2              | `pc88va2`                                 | The sub-CPU ROM is NO_DUMP in MAME and must be sourced separately            |
| PC-6001               | `pc6001`                                  |                                                                              |
| PC-6001mkII           | `pc6001mk2`                               |                                                                              |
| PC-6601               | `pc6601`                                  |                                                                              |
| PC-6001mkIISR         | `pc6001mk2sr`                             |                                                                              |
| PC-6601SR             | `pc6601sr`                                |                                                                              |
| FM Towns / II CX / MX | `fmtownsmx`                               | The base FM Towns and CX targets boot the shared MX ROM set until CX dumps exist |
| Sharp X68000          | `x68000`                                  | Original CZ-600C split IPL                                                   |
| Sharp X68000 SUPER    | `x68ksupr`                                | IPL V1.0 and internal SCSI ROM                                               |
| Sharp X68000 XVI      | `x68kxvi`                                 | IPL V1.1 with the compatible Compact-XVI SCSI ROM                            |
| Sharp X1              | `x1`                                      | Both X1 sets share the `--x1-roms` directory                                 |
| Sharp X1 turbo        | `x1turbo`                                 | The ANK font is byte-identical with the `x1` set                             |
| Fujitsu FM-7          | `fm7`                                     | Both FM-7 sets share the `--fm7-roms` directory; kanji ROM optional          |
| Fujitsu FM-77AV       | `fm77av`                                  | The sub monitor C and kanji ROMs are byte-identical with the `fm7` set       |
| Roland MT-32          | MT-32 v1.07 (recommended)                 | Any supported control + PCM pair works; see the MT-32 section                |
| Roland SC-55          | SC-55 v1.21 (recommended)                 | Any supported model works; see the SC-55 section                             |

The PC-98 models all use the single `--pc98-roms` directory. Because matching is by
content hash, you can extract the `pc9801f`, `pc9801vm`, `pc9801vx`, and `pc9801rs`
sets into that one directory and neetan will pick the right chips for whichever model
you boot.

## Example directory layout

One sub-directory per `--*-roms` option. The PC-98 directory holds every PC-98 set
together; the others hold their single set.

```
roms/
|-- pc98/          --pc98-roms                     pc9801f + pc9801vm + pc9801vx + pc9801rs sets
|   |-- urm01-02.bin ... urm06-02.bin              (PC-9801F BIOS chips)
|   |-- cpu_board_1a_23128e.bin ...                (PC-9801VM BIOS chips)
|   |-- nec_d27c256d-15_cpu_extboard_yll01.bin ... (PC-9801VX BIOS chips)
|   |-- itf_rs.rom, bios_rs.rom                    (PC-9801RS / PC-9801RA BIOS, from the RS set)
|   |-- font_ux.rom, font_rs.rom                   (shared V98 fonts)
|   `-- sound.rom                                  (PC-9801-26K sound BIOS)
|-- pc88/          --pc88-roms                     pc8801mc [+ pc8001mk2, pc8001mk2sr]
|   |-- mc_n88.rom, mc_n88_0.rom ... cdbios.rom, kanji1.rom, mc_kanji2.rom, ...
|   `-- n80_2.rom, n80_2sr.rom, n80_3.rom          (optional N80 boot modes)
|-- pc88va/        --pc88va-roms   pc88va2
|   |-- varom00_va2.rom, varom08_va2.rom, varom1_va2.rom, vafont_va2.rom, vadic_va2.rom
|   `-- (sub-CPU ROM: not in the MAME set, source separately)
|-- pc6000/        --pc6000-roms   pc6001 ... pc6601sr
|   `-- basicrom.60, systemrom1.64, cgrom*.*, kanjirom.*, voicerom.*, ...
|-- fmtowns/       --towns-roms    fmtownsmx
|   |-- fmtownsiimxbios.m79
|   `-- mytownsmx.rom
|-- x68k/          --x68k-roms                     one selected X68000 model set
|   |-- cgrom.dat                                  (shared character generator)
|   |-- rh-ix0897cezz.ic12, rh-ix0898cezz.ic11     (original split IPL)
|   |-- iplrom.dat, scsiinsu.bin                   (SUPER)
|   `-- iplromxv.dat, scsiinco.bin                 (XVI compatibility set)
|-- x1/            --x1-roms                       x1 + x1turbo sets
|   |-- ipl.x1, ipl.x1t                            (IPL ROMs: X1, X1turbo)
|   |-- fnt0808.x1, fnt0808_turbo.x1, ank.fnt      (8x8 CG fonts + shared 8x16 ANK font)
|   `-- kanji1.rom ... kanji4.rom                  (turbo kanji ROMs)
|-- fm7/           --fm7-roms                      fm7 + fm77av sets
|   |-- fbasic300.rom, boot_bas.rom, boot_dos_a.rom (FM-7 F-BASIC + boot ROMs)
|   |-- initiate.rom, fbasic30.rom                 (FM-77AV initiator + F-BASIC)
|   |-- subsys_c.rom, subsys_a.rom, subsys_b.rom, subsyscg.rom (sub monitors + CG)
|   `-- kanji.rom                                  (shared JIS level-1 kanji ROM)
|-- mt32/          --mt32-roms     MT-32 v1.07
|   |-- MT32_CONTROL.ROM
|   `-- MT32_PCM.ROM
`-- sc55/          --sc55-roms     SC-55 v1.21
    |-- sc55_rom1.bin, sc55_rom2.bin
    `-- sc55_waverom1.bin, sc55_waverom2.bin, sc55_waverom3.bin
```

## Example configuration

A `.conf` file (see `configuration/default.conf` for the full template) that points
every ROM option at the layout above:

```ini
; PC-98: one directory holding all PC-9801 MAME sets.
; Boot the real BIOS with bios=on for games that need it.
; 98% of games will work with the HLE BIOS just fine
; (and booting is faster on the HLE BIOS).
pc98-roms = roms/pc98

; PC-88 family.
pc88-roms   = roms/pc88
pc88va-roms = roms/pc88va
pc6000-roms = roms/pc6000

; FM Towns.
towns-roms = roms/fmtowns

; Sharp X68000.
x68k-roms = roms/x68k

; Sharp X1.
x1-roms = roms/x1

; Fujitsu FM-7 / FM-77AV.
fm7-roms = roms/fm7

; MIDI modules.
mt32-roms = roms/mt32
sc55-roms = roms/sc55
```

## Build script

The bash script below turns a set of extracted MAME sets into the `roms/` layout
above. Run it from a directory that contains the extracted sets (any depth, any
names); it hashes every file once, then verifies and copies each ROM neetan needs
with `b3sum` + `cp`, printing `OK` / `MISS` per file. It requires `b3sum`.

```bash
#!/usr/bin/env bash
# Build a neetan ROM directory tree from extracted MAME ROM sets.
# Usage: run from a directory containing the extracted sets. Output goes to ./roms.
set -euo pipefail

DEST="${1:-roms}"

echo "Hashing source files ..."
declare -A SRC
while IFS= read -r -d '' file; do
    SRC["$(b3sum --no-names "$file")"]="$file"
done < <(find . -type f -not -path "./$DEST/*" -print0)

status=0
place() { # <blake3> <dest-relative-path>
    local want="$1" dest="$2" src="${SRC[$1]:-}"
    if [ -z "$src" ]; then
        echo "MISS  $dest  (no source file with b3sum $want)" >&2
        status=1
        return
    fi
    mkdir -p "$DEST/$(dirname "$dest")"
    cp -f "$src" "$DEST/$dest"
    echo "OK    $dest"
}

while read -r hash dest; do
    [ -z "${hash:-}" ] && continue
    case "$hash" in \#*) continue ;; esac
    place "$hash" "$dest"
done <<'ROMS'
# --- PC-9801 / PC-9821 (all sets share roms/pc98) ---
cbac44179293aa4ad530c72fa19f2e3ac8278f3e6816a8691db82fa81d82e11c  pc98/urm01-02.bin
e2717e5f6145218f2ddfa53d57c51df4da117b3b97ce044f81e90032bda4db69  pc98/urm02-02.bin
a3ae6097b2203e5a5434dedb83f3eaeb9417551649e96af0e07cae1b8e8d4a7f  pc98/urm03-02.bin
6b171375a77c20d515babda27c6189be6c5caa32c84825b003e616932c2d99bb  pc98/urm04-02.bin
76f8963dd66a65b05b862f6193a8bd05b2ce64b4c388394136544a297bcc2757  pc98/urm05-02.bin
8c14048b85a320340a07a44fadb1d54b04fa3da50e38f36ed19ae2cc1870886f  pc98/urm06-02.bin
eb16e6050452c218497e6cf28591e8c049cca0a313bb5d9b8f30e2b22a58a939  pc98/cpu_board_1a_23128e.bin
f8b7cda3cf40c9feca6899cc1045cdd65cd85a90511560eace8028253e1ce1f3  pc98/cpu_board_2a_d23c256ec.bin
13218482d54793a10a25ea712a5be362d1d490c568c3e0228939af3f2c244b9c  pc98/cpu_board_3a_23c256e.bin
00b3558c12b28dff9ab823354b289ad559c59d3695e80c1586f07e23890d45a6  pc98/cpu_board_4a_d23128ec.bin
04de20aabdf46d943cd5148e4bbfdd7ba843fe19d8fec7afbaa48630f6887e52  pc98/nec_d27c256d-15_cpu_extboard_yll01.bin
3de5476a72aadd3e32870f3525bc4fc945e941f8c3cfe81aeb0fd73c955294ee  pc98/nec_d27c256d-15_cpu_extboard_yll02.bin
05e3220b53a61c9325ceb694629bddc20c593dbf2218ee78a9e5635e8a7bf5f6  pc98/nec_d27c256d-15_cpu_extboard_yll03.bin
a0f6e1e87afa336c21648f972c96e20ea88dda792a1fc3d0acf26d8c50546158  pc98/nec_d27c256d-15_cpu_extboard_yll04.bin
c1881b44dc07a7f20ceff00a24fe4467a933fd2c94e64213c9a8526d60e4d3d1  pc98/itf_rs.rom
ac5b46fbec4a5ac6b3185066d86af8e3d76cd1b66955301dad3cae8736b31f2d  pc98/bios_rs.rom
3c1efa858b80fc11bb7482bdc5e15004dd9a015d7d22d48159cd43ed63f540dc  pc98/font_ux.rom
4b6f751f34e633e072ded2a109c25ddb90ac70350792dc55914a4cefa4dbe005  pc98/font_rs.rom
93816a6e42ed9a10135af634ed500e10b1d266e0b4158d3f8471910609255e24  pc98/sound.rom
# --- PC-8801MC (roms/pc88) ---
40457b507b82dd57cce0fcecf6bc65543a60bd46558ca947b0f69dd3658cdad8  pc88/mc_n88.rom
6a50a88231062ec871c65f63266fa7062a303ab870aed81c49f1f333f594a518  pc88/mc_n88_0.rom
d5583fcce4eabf078d17666a1fddefa6a0d8bdc7f56d4499d526818728777252  pc88/mc_n88_1.rom
ca200799765cb02a001bd55215b0daaf6d0593118a05e8d85754bddd92e5e8f7  pc88/mc_n88_2.rom
ac31c1fbabfada9890669bebd471d60fac0be0e88ddfde81f17c600d5b0a1757  pc88/mc_n88_3.rom
652eacc1ed6073bc3da1856c9c4f74ac14abef3f966f0d0fc89c40386de3d1a1  pc88/mc_n80.rom
283dcd1c4a69f8049d19021d34d1cc2094f10de8b4e1ddf85da6a4b258dd8d12  pc88/mc_jisyo.rom
10fd26424ae9e28be721846491d2d7b10e946da2d2ff39542248e819bc2339ba  pc88/kanji1.rom
f528e78bbe43e3d36c3def6ef30140e22ba9e69f422736605c2c4570c7d3fbe7  pc88/mc_kanji2.rom
081d2ca8ad7066de207b7360e45b5d6f3bab01769aefb9057141becbbaec5aa5  pc88/ma_disk.rom
de4d49437344806850b22356f9e5537e413e6113902fb8fbc803f902a5728827  pc88/cdbios.rom
9e4ec9c53f4432a88583dccd04ae3186f4d7849f80ea7774ac1efbdb93c992f2  pc88/n80_2.rom
56406a79fd664a197c458cb3feeeb6994c34266a1e02728877b6ea5ef86e15ba  pc88/n80_2sr.rom
7b81e27b831ad00f264170d1d98c645298fa688b07d5a9f0c19c1d6a73fe4273  pc88/n80_3.rom
# --- PC-88VA2 (roms/pc88va) ---
bba5011412fb266b3c15ff08d2508716ba2ac54fec3aa172b59e441486807eab  pc88va/varom00_va2.rom
4cdf3da9a1423e874f9618a8d8859107fa5e3d20a91f4dcf908e042763c41bbb  pc88va/varom08_va2.rom
1239bf390d444ff205f70c700527cb50bc90107904050fa8713a415a17bf0e42  pc88va/varom1_va2.rom
b47ec9f55ff199ac71f453385aec0f370afbb958fd47ad9bb5161bdf4e2bb3ee  pc88va/vafont_va2.rom
21fcd88c97b881e55f015f22d62002022189572e171f1c5e485b751c84379b30  pc88va/vadic_va2.rom
# --- PC-6001 / PC-6601 (roms/pc6000) ---
13bc0696487984f7836f094312b64fb0702dcb5ac3b941a79bd6f174e657697d  pc6000/basicrom.60
d951eae886dec98a063e5fb11e12b0385f5dd4617c0546fe7cf9fd77b17ae41c  pc6000/basicrom.62
d9eaf3e5e6cb1f71db527e6eeadf7a1968f8a558234b74c6812198c588ae46d1  pc6000/basicrom.66
c4901a2149f3c8e65d3db78bbf3776fc2d963f270152923ba920274d44a0224b  pc6000/basicrom.68
6ca4e747c8b17307a77150441e5d8721d5c242fcc8b8ef35737d3f5edf6e2d74  pc6000/systemrom1.64
998a90c4bd0bf4ae4a600a0d94f3eca96c3b8db754311ce1c8029126dbcf0a9a  pc6000/systemrom2.64
becb7c1502d41a9f160b651e142044610ffa172a8bbf47eaa11aa0086953a080  pc6000/sysrom2.68
f537afe76997ec4f8b377a29771f45c39414a25f7e071d2d38b143cdd8bee7bc  pc6000/cgrom60.60
581f6d2db80386732ed09706ad3b8961f8b77b7ea024e65cec37e56ad2adf07c  pc6000/cgrom60.62
63829a1c32924a77f85716f445c445ab7be178c4438cfd8cf6ffaff5731a0965  pc6000/cgrom60.66
24e524d4938809a87720f98abfba71c8e9162d742c67a167d8b87566cc1d4258  pc6000/cgrom60.68
ba0dd650539dd3fdbf63da36982b41bfda8f4c2ea0dcda2c1c2ac56427ee26ed  pc6000/cgrom66.66
067c732525260eadfcfecbb9fc4ef9535c0c2f77caa049453bf2ab992ec3fca3  pc6000/cgrom66.68
b49b056ca06bd0c2253e6db0806969787a6fca4fc78228728422c9cf63f1e472  pc6000/cgrom68.64
f0af53e54b1b09b229d03efc9f65e65597a0c4f6aa9e3e7c0e553274ccd481fb  pc6000/kanjirom.62
633e73f55479bee65ed344d818a35b15ab109f188ad5c09826c066d6ec2596c5  pc6000/voicerom.62
88a747147725fd618668e07744b05f34288b4454698d6182c4db2e680c7b76d0  pc6000/voicerom.66
8ed4a9a3e9ae2e4aa0fccc0f170081f3f61c09e293812b7973a7ab9c23e22b68  pc6000/voicerom.68
# --- FM Towns II CX / MX (roms/fmtowns) ---
f5c2cc7c2876a4b30f320fe6fb721bd32f3ba43bbb9b0b42c398fa6b59d72ce8  fmtowns/fmtownsiimxbios.m79
d5dc70e34d072889c28bed51ef3ccaac7f6f3fdd9e448d89297847247a901538  fmtowns/mytownsmx.rom
# --- Sharp X68000 (roms/x68k) ---
095cfc5c21d704cce7340982b717dadc9fa20bfb86637ce9a594af88c87dc6b8  x68k/cgrom.dat
50f6e84f88feb32e1cf2421ea6376fed44851c269f8bd48706c2e8061ceba313  x68k/rh-ix0897cezz.ic12
2bc789c7b172ebbe70d5099a9b8820653234e26cb5f7b4a171b4d73ee647ddaa  x68k/rh-ix0898cezz.ic11
10ecab1df03426f4823de6cca28a26818b471b9ca20943441ba73c8fd0cd710f  x68k/iplrom.dat
7ac5c8fa53d2693ee61ada293efd1f681b1390ef50c1117ddcf52d2280468c20  x68k/scsiinsu.bin
06d3d6365d2b4079abf37d362a393f9224e472b8321e1826fef0a263d9e26590  x68k/iplromxv.dat
08e08002db7e47bdf6f2f60066f7253eb94791fb2aa17b392e26d23d72e0c19f  x68k/scsiinco.bin
# --- Sharp X1 / X1 turbo (both sets share roms/x1) ---
194f351bc1024188162856e2374d92bc608d9c742ca007d8c19a4b4eed44abbc  x1/ipl.x1
871c77226a6e65bf1820c0a3e6f63a330cb1d2eb6c135fc9e4da9741ce38106c  x1/ipl.x1t
61440d736fdec066b825428f4d26fbdb04b3a4fcc7f05bbdd4b5bbe9e55318c3  x1/fnt0808.x1
f26c67af04f3b4819e0bd474ded7b083e3d370a62ea0672f09787b8ca4ebc4a6  x1/fnt0808_turbo.x1
a8695470e98492a2d969ba3fdeee76ee9b3573f525eee20f98627fb5e98279a0  x1/ank.fnt
212d081a600377a1068d56f4049d03916ea705465eb2feca950b6df186a12ba4  x1/kanji1.rom
0bd59d087b3197c8136e5664e311234930ec566b61d184204144f04a84ba769b  x1/kanji2.rom
f2495255441c15bfce5c7441f6d94809d4f0e0dba1c7f43f9153991e326b881a  x1/kanji3.rom
84e0afa27e1f4ef01b6e5dac452835f487c98968e14fceaac3c93331524b51d7  x1/kanji4.rom
# --- Fujitsu FM-7 / FM-77AV (both sets share roms/fm7) ---
059a5c926109fc156f07d91aaad05307ff0bd9d3eb5bffa805d554863f4a01bc  fm7/fbasic300.rom
d6a8dda5482a337e28aaf7b838be0543411277ba17f260ae62f9f1af46592b2d  fm7/boot_bas.rom
fbc9e9240f810deb8e28207b7a3362486f5f57294fb7ff8225628286479d26f3  fm7/boot_dos_a.rom
276f3953b3f8fe975d29d13463261d9e70ce9c339d2af12536cf2010ae0f2a8d  fm7/fbasic30.rom
4ac5111f650f4415763c1e0d9f6b997432f80c5ba9b60a38b68b308dcea9f404  fm7/initiate.rom
55b0e4f72561ea0fafe6353376642d70595b08989a2d76c2b6423c7d85a9d1d2  fm7/subsys_c.rom
413b20a42227ddf95e153685cc989dcf03b193aaf79f3429848db899bd6635e3  fm7/subsys_a.rom
edf5fc537af21d93c73d3446e44654fbab0106edaf85f564abfad99bd28590e1  fm7/subsys_b.rom
7b430d28aebaf260a823e8585c31dacc2aaca9d4f69ab34672a1ded0b37cfd23  fm7/subsyscg.rom
482b314f15b6a063e06a8c3e6e7426d4de9b8513086ab0e72ff0ea1623ac51f6  fm7/kanji.rom
# --- Roland MT-32 v1.07 (roms/mt32) ---
8f123c1f38104a2a7eb1df35fd5b26ca1b857185086a87233b355510264602bf  mt32/MT32_CONTROL.ROM
7805996b758fab5469e96d9a28588eb2e991440242372f7546345cdc66c8d97a  mt32/MT32_PCM.ROM
# --- Roland SC-55 v1.21 (roms/sc55; names matter for SC-55) ---
4755d6b3b0455b13c9176f409f4d9c2d9953f61fb5e68df3094886b7b2795f9e  sc55/sc55_rom1.bin
593282df20a4a9ed9b595711fce19b84999316fa8bc6bf2ff63df66871ca81ce  sc55/sc55_rom2.bin
482581a5042a54ed6d8c46df929a898902feddee248ee0becb5e228dbab638e3  sc55/sc55_waverom1.bin
3b14ffc7e6803fa7cb0f5cb2cca3eb07094b49b9563f5a55b656360df7a90965  sc55/sc55_waverom2.bin
cd0b796db0b37467bb64cb2f89c66befe841125620daa008531bdd3723880cd3  sc55/sc55_waverom3.bin
ROMS

exit $status
```

## PC-9801 / PC-9821

The PC-98 targets run on a built-in HLE BIOS and a built-in font by default, so a ROM
set is optional. Point `--pc98-roms` at a directory of MAME dumps and pass `--bios`
to boot the model's real BIOS instead of the HLE BIOS. neetan assembles the model's
BIOS from its individual mask-ROM chips (following MAME's ROM layout) into the
192 KiB dual-bank image the emulator uses.

With `--bios` the model's BIOS is required. The PC-9821 targets are the exception:
they have no real-BIOS boot path and always fall back to HLE with a warning. The 26K
sound ROM (`sound.rom`) is loaded when a PC-9801-26K board is selected. A font ROM is
best-effort: any V98 font in the directory is used, otherwise the built-in font is
kept.

BIOS chips (assembled per model). Extract the model's MAME set into `--pc98-roms`:

PC-9801F (MAME set `pc9801f`):

| File           | Size   | BLAKE3                                                             |
|----------------|--------|--------------------------------------------------------------------|
| `urm01-02.bin` | 16 KiB | `cbac44179293aa4ad530c72fa19f2e3ac8278f3e6816a8691db82fa81d82e11c` |
| `urm02-02.bin` | 16 KiB | `e2717e5f6145218f2ddfa53d57c51df4da117b3b97ce044f81e90032bda4db69` |
| `urm03-02.bin` | 16 KiB | `a3ae6097b2203e5a5434dedb83f3eaeb9417551649e96af0e07cae1b8e8d4a7f` |
| `urm04-02.bin` | 16 KiB | `6b171375a77c20d515babda27c6189be6c5caa32c84825b003e616932c2d99bb` |
| `urm05-02.bin` | 16 KiB | `76f8963dd66a65b05b862f6193a8bd05b2ce64b4c388394136544a297bcc2757` |
| `urm06-02.bin` | 16 KiB | `8c14048b85a320340a07a44fadb1d54b04fa3da50e38f36ed19ae2cc1870886f` |

PC-9801VM (MAME set `pc9801vm`):

| File                          | Size   | BLAKE3                                                             |
|-------------------------------|--------|--------------------------------------------------------------------|
| `cpu_board_1a_23128e.bin`     | 16 KiB | `eb16e6050452c218497e6cf28591e8c049cca0a313bb5d9b8f30e2b22a58a939` |
| `cpu_board_2a_d23c256ec.bin`  | 32 KiB | `f8b7cda3cf40c9feca6899cc1045cdd65cd85a90511560eace8028253e1ce1f3` |
| `cpu_board_3a_23c256e.bin`    | 32 KiB | `13218482d54793a10a25ea712a5be362d1d490c568c3e0228939af3f2c244b9c` |
| `cpu_board_4a_d23128ec.bin`   | 16 KiB | `00b3558c12b28dff9ab823354b289ad559c59d3695e80c1586f07e23890d45a6` |

PC-9801VX (MAME set `pc9801vx`):

| File                                       | Size   | BLAKE3                                                             |
|--------------------------------------------|--------|--------------------------------------------------------------------|
| `nec_d27c256d-15_cpu_extboard_yll01.bin`   | 32 KiB | `04de20aabdf46d943cd5148e4bbfdd7ba843fe19d8fec7afbaa48630f6887e52` |
| `nec_d27c256d-15_cpu_extboard_yll02.bin`   | 32 KiB | `3de5476a72aadd3e32870f3525bc4fc945e941f8c3cfe81aeb0fd73c955294ee` |
| `nec_d27c256d-15_cpu_extboard_yll03.bin`   | 32 KiB | `05e3220b53a61c9325ceb694629bddc20c593dbf2218ee78a9e5635e8a7bf5f6` |
| `nec_d27c256d-15_cpu_extboard_yll04.bin`   | 32 KiB | `a0f6e1e87afa336c21648f972c96e20ea88dda792a1fc3d0acf26d8c50546158` |

PC-9801RS / PC-9801RA (MAME set `pc9801rs`, shared by both models):

| File          | Size   | BLAKE3                                                             |
|---------------|--------|--------------------------------------------------------------------|
| `itf_rs.rom`  | 32 KiB | `c1881b44dc07a7f20ceff00a24fe4467a933fd2c94e64213c9a8526d60e4d3d1` |
| `bios_rs.rom` | 96 KiB | `ac5b46fbec4a5ac6b3185066d86af8e3d76cd1b66955301dad3cae8736b31f2d` |

The PC-9821AS and PC-9821AP have no real-BIOS boot path and always run the
HLE BIOS.

Font ROM (V98 format, 282 KiB). Best-effort: any of these dumps is accepted for any
model, otherwise the built-in font is used. The `pc9801vx` and `pc9801rs` sets ship a
font (`font_ux.rom` / `font_rs.rom`); the `pc9801f` and `pc9801vm` sets do not, so
those models fall back to the built-in font unless one of the other fonts is in the
same directory.

| Dump          | Source     | BLAKE3                                                             |
|---------------|------------|--------------------------------------------------------------------|
| `font_rs.rom` | `pc9801rs` | `4b6f751f34e633e072ded2a109c25ddb90ac70350792dc55914a4cefa4dbe005` |
| `font_ux.rom` | `pc9801vx` | `3c1efa858b80fc11bb7482bdc5e15004dd9a015d7d22d48159cd43ed63f540dc` |
| PC-9821As     | -          | `a567134a3d5c2a215b9573ee07b5204fff243631052e7a40be340e863aff8eef` |
| PC-9821Ap2    | -          | `7fb96af345c33f9bd7be5c22f75c650ac41da9b543ca5f9ca7b3d3906f2abb40` |
| PC-9821Ce2    | -          | `b38096265c76cf9f54cb47df905cfb6c8b4d4f27019a04835bbc3dc8782d33e1` |

Sound ROM (loaded when a PC-9801-26K board is selected; present in every PC-98 set):

| File        | Size   | BLAKE3                                                             |
|-------------|--------|--------------------------------------------------------------------|
| `sound.rom` | 16 KiB | `93816a6e42ed9a10135af634ed500e10b1d266e0b4158d3f8471910609255e24` |

## PC-8001 / PC-8801

The PC-8801MC needs a real ROM set (MAME set `pc8801mc`), pointed to by `--pc88-roms`.
The optional N80 boot-mode ROMs come from the `pc8001mk2` and `pc8001mk2sr` sets.

These ROMs are always required:

| Slot       | Size    | Contents                         | BLAKE3                                                             |
|------------|---------|----------------------------------|--------------------------------------------------------------------|
| `n88`      | 32 KiB  | N88-BASIC main ROM               | `40457b507b82dd57cce0fcecf6bc65543a60bd46558ca947b0f69dd3658cdad8` |
| `n88_ext0` | 8 KiB   | N88-BASIC extension bank 0       | `6a50a88231062ec871c65f63266fa7062a303ab870aed81c49f1f333f594a518` |
| `n88_ext1` | 8 KiB   | N88-BASIC extension bank 1       | `d5583fcce4eabf078d17666a1fddefa6a0d8bdc7f56d4499d526818728777252` |
| `n88_ext2` | 8 KiB   | N88-BASIC extension bank 2       | `ca200799765cb02a001bd55215b0daaf6d0593118a05e8d85754bddd92e5e8f7` |
| `n88_ext3` | 8 KiB   | N88-BASIC extension bank 3       | `ac31c1fbabfada9890669bebd471d60fac0be0e88ddfde81f17c600d5b0a1757` |
| `n_basic`  | 32 KiB  | N-BASIC ROM (PC-8001, 1979)      | `652eacc1ed6073bc3da1856c9c4f74ac14abef3f966f0d0fc89c40386de3d1a1` |
| `jisyo`    | 512 KiB | Kanji dictionary ROM             | `283dcd1c4a69f8049d19021d34d1cc2094f10de8b4e1ddf85da6a4b258dd8d12` |
| `kanji1`   | 128 KiB | Level-1 kanji ROM                | `10fd26424ae9e28be721846491d2d7b10e946da2d2ff39542248e819bc2339ba` |
| `kanji2`   | 128 KiB | Level-2 kanji ROM                | `f528e78bbe43e3d36c3def6ef30140e22ba9e69f422736605c2c4570c7d3fbe7` |
| `disk`     | 8 KiB   | PC80S31K disk sub-CPU ROM        | `081d2ca8ad7066de207b7360e45b5d6f3bab01769aefb9057141becbbaec5aa5` |
| `cdbios`   | 64 KiB  | PC-8801-31 CD-ROM interface BIOS | `de4d49437344806850b22356f9e5537e413e6113902fb8fbc803f902a5728827` |

These ROMs are required only when the matching boot mode is selected:

| Slot         | Size   | Required by boot mode | MAME set       | BLAKE3                                                             |
|--------------|--------|-----------------------|----------------|--------------------------------------------------------------------|
| `n80_mkii`   | 32 KiB | `n80`                 | `pc8001mk2`    | `9e4ec9c53f4432a88583dccd04ae3186f4d7849f80ea7774ac1efbdb93c992f2` |
| `n80_mkiisr` | 32 KiB | `n80sr`               | `pc8001mk2sr`  | `56406a79fd664a197c458cb3feeeb6994c34266a1e02728877b6ea5ef86e15ba` |
| `n80sr`      | 40 KiB | `n80sr`               | `pc8001mk2sr`  | `7b81e27b831ad00f264170d1d98c645298fa688b07d5a9f0c19c1d6a73fe4273` |

## PC-88VA2

The PC-88VA2 needs a real ROM set (MAME set `pc88va2`), pointed to by `--pc88va-roms`.
The floppy sub-CPU ROM (`subsys`) is marked `NO_DUMP` in MAME and is not part of the
`pc88va2` set; it must be sourced separately (sometimes called `disk.rom` (8 KiB) in
older dumps; a PC-88VA's disk.rom should be the same). All slots are required:

| Slot         | Size    | Contents                  | BLAKE3                                                             |
|--------------|---------|---------------------------|--------------------------------------------------------------------|
| `rom00`      | 512 KiB | ROM0 low image (varom00)  | `bba5011412fb266b3c15ff08d2508716ba2ac54fec3aa172b59e441486807eab` |
| `rom08`      | 128 KiB | ROM0 high image (varom08) | `4cdf3da9a1423e874f9618a8d8859107fa5e3d20a91f4dcf908e042763c41bbb` |
| `rom1`       | 128 KiB | ROM1 image (varom1)       | `1239bf390d444ff205f70c700527cb50bc90107904050fa8713a415a17bf0e42` |
| `font`       | 320 KiB | Kanji / font ROM          | `b47ec9f55ff199ac71f453385aec0f370afbb958fd47ad9bb5161bdf4e2bb3ee` |
| `dictionary` | 512 KiB | Dictionary (jisyo) ROM    | `21fcd88c97b881e55f015f22d62002022189572e171f1c5e485b751c84379b30` |
| `subsys`     | 8 KiB   | Floppy sub-CPU (Z80) ROM  | `531ab2aa2c7d7c4deb2ddd8303c6637ea7e273648825fb51e17c8660d7496565` |

## PC-6001 / PC-6601

The PC-6000 targets need a real ROM set, pointed to by `--pc6000-roms`. Each model has
its own MAME set (`pc6001`, `pc6001mk2`, `pc6601`, `pc6001mk2sr`, `pc6601sr`); extract
whichever you need into the directory (they can share it). Each model requires its boot
ROM (BASIC or, on the SR models, the system ROM) and its base character generator; the
kanji, extended character generator, and voice ROMs are loaded when present. Several
dumps are bit-identical across models, so a single file can satisfy more than one slot.

| Slot          | Size   | Contents                          | Required for                            | BLAKE3                                                             |
|---------------|--------|-----------------------------------|-----------------------------------------|--------------------------------------------------------------------|
| `basic60`     | 16 KiB | PC-6001 BASIC                     | `PC6001`                                | `13bc0696487984f7836f094312b64fb0702dcb5ac3b941a79bd6f174e657697d` |
| `basic62`     | 32 KiB | PC-6001mkII BASIC                 | `PC6001MK2`                             | `d951eae886dec98a063e5fb11e12b0385f5dd4617c0546fe7cf9fd77b17ae41c` |
| `basic66`     | 32 KiB | PC-6601 BASIC                     | `PC6601`                                | `d9eaf3e5e6cb1f71db527e6eeadf7a1968f8a558234b74c6812198c588ae46d1` |
| `basic68`     | 32 KiB | PC-6601SR mkII-compat BASIC       | `PC6601SR` (optional)                   | `c4901a2149f3c8e65d3db78bbf3776fc2d963f270152923ba920274d44a0224b` |
| `system1`     | 64 KiB | SR system ROM, first half         | `PC6001MK2SR`, `PC6601SR`               | `6ca4e747c8b17307a77150441e5d8721d5c242fcc8b8ef35737d3f5edf6e2d74` |
| `system2`     | 64 KiB | SR system ROM, second half        | `PC6001MK2SR`, `PC6601SR`               | `998a90c4bd0bf4ae4a600a0d94f3eca96c3b8db754311ce1c8029126dbcf0a9a` |
| `subsys`      | 8 KiB  | SR sub / disk ROM                 | `PC6601SR` (optional)                   | `becb7c1502d41a9f160b651e142044610ffa172a8bbf47eaa11aa0086953a080` |
| `cg60`        | 4 KiB  | PC-6001 base character generator  | `PC6001`                                | `f537afe76997ec4f8b377a29771f45c39414a25f7e071d2d38b143cdd8bee7bc` |
| `cg62`        | 8 KiB  | PC-6001mkII base CG               | `PC6001MK2`                             | `581f6d2db80386732ed09706ad3b8961f8b77b7ea024e65cec37e56ad2adf07c` |
| `cg66`        | 8 KiB  | PC-6601 base CG                   | `PC6601`                                | `63829a1c32924a77f85716f445c445ab7be178c4438cfd8cf6ffaff5731a0965` |
| `cg68base`    | 8 KiB  | PC-6601SR base CG                 | `PC6001MK2SR`, `PC6601SR` (optional)    | `24e524d4938809a87720f98abfba71c8e9162d742c67a167d8b87566cc1d4258` |
| `cgext`       | 8 KiB  | Extended CG (mkII / 6601)         | `PC6001MK2`, `PC6601` (optional)        | `ba0dd650539dd3fdbf63da36982b41bfda8f4c2ea0dcda2c1c2ac56427ee26ed` |
| `cg68ext`     | 8 KiB  | PC-6601SR extended CG             | `PC6001MK2SR`, `PC6601SR` (optional)    | `067c732525260eadfcfecbb9fc4ef9535c0c2f77caa049453bf2ab992ec3fca3` |
| `cg68`        | 16 KiB | SR native CG                      | `PC6001MK2SR`, `PC6601SR`               | `b49b056ca06bd0c2253e6db0806969787a6fca4fc78228728422c9cf63f1e472` |
| `kanji`       | 32 KiB | Kanji font ROM                    | mkII and later (optional)               | `f0af53e54b1b09b229d03efc9f65e65597a0c4f6aa9e3e7c0e553274ccd481fb` |
| `voice62`     | 16 KiB | uPD7752 voice data (PC-6001mkII)  | `PC6001MK2` (optional)                  | `633e73f55479bee65ed344d818a35b15ab109f188ad5c09826c066d6ec2596c5` |
| `voice66`     | 16 KiB | uPD7752 voice data (PC-6601)      | `PC6601` (optional)                     | `88a747147725fd618668e07744b05f34288b4454698d6182c4db2e680c7b76d0` |
| `voice68`     | 16 KiB | uPD7752 voice data (SR models)    | `PC6001MK2SR`, `PC6601SR` (optional)    | `8ed4a9a3e9ae2e4aa0fccc0f170081f3f61c09e293812b7973a7ab9c23e22b68` |

## FM Towns

The FM Towns targets need a real ROM set, pointed to by `--towns-roms`. The base
FM Towns, the FM Towns II CX, and the MX all use the FM Towns II MX ROM dump (MAME
set `fmtownsmx`). Two layouts are accepted.

The merged set is the packed 2 MiB MAME BIOS image plus the 32-byte serial ROM (this
is what the `fmtownsmx` set contains):

| File                  | Size     | BLAKE3                                                             |
|-----------------------|----------|--------------------------------------------------------------------|
| `fmtownsiimxbios.m79` | 2 MiB    | `f5c2cc7c2876a4b30f320fe6fb721bd32f3ba43bbb9b0b42c398fa6b59d72ce8` |
| `mytownsmx.rom`       | 32 bytes | `d5dc70e34d072889c28bed51ef3ccaac7f6f3fdd9e448d89297847247a901538` |

The split set provides the five images individually plus the serial ROM:

| File            | Size     | BLAKE3                                                             |
|-----------------|----------|--------------------------------------------------------------------|
| `FMT_SYS.ROM`   | 256 KiB  | `fba6e75d9727b6a192bf6b3e351f6ed7ae118162a0f71fea9c825a6b5f143022` |
| `FMT_DOS.ROM`   | 512 KiB  | `7f07a3c51743b51b02f347251057cfd1bfff9ff718b6c0fd3540e0da77c8a4da` |
| `FMT_FNT.ROM`   | 256 KiB  | `0c365fb76a886c9f426893949d73390456ed6fc6c83f3109f699b0ded8b1ef24` |
| `FMT_F20.ROM`   | 512 KiB  | `1dde131510456c9660c2217774853822674459412d8e6f98312fff0ee83ca9a7` |
| `FMT_DIC.ROM`   | 512 KiB  | `0fbcbecb5b62c8fa4e9a60885f887b0a2cafd680a1174b0f7ddf57f49c65ab60` |
| `mytownsmx.rom` | 32 bytes | `d5dc70e34d072889c28bed51ef3ccaac7f6f3fdd9e448d89297847247a901538` |

## Sharp X68000

The X68000 targets require a real ROM set selected with `--x68k-roms`. The
loader scans one directory non-recursively and selects only the slots required
by the chosen model. A directory may contain additional IPL variants; they are
ignored rather than substituted for the requested model.

| Slot                                      | Size    | Required for  | BLAKE3                                                             |
|-------------------------------------------|---------|---------------|--------------------------------------------------------------------|
| `cgrom`                                   | 768 KiB | All models    | `095cfc5c21d704cce7340982b717dadc9fa20bfb86637ce9a594af88c87dc6b8` |
| IPL even half (`rh-ix0897cezz.ic12`)      | 64 KiB  | `X68000`      | `50f6e84f88feb32e1cf2421ea6376fed44851c269f8bd48706c2e8061ceba313` |
| IPL odd half (`rh-ix0898cezz.ic11`)       | 64 KiB  | `X68000`      | `2bc789c7b172ebbe70d5099a9b8820653234e26cb5f7b4a171b4d73ee647ddaa` |
| IPL V1.0 (`iplrom.dat`)                   | 128 KiB | `X68000SUPER` | `10ecab1df03426f4823de6cca28a26818b471b9ca20943441ba73c8fd0cd710f` |
| Internal SCSI (`scsiinsu.bin`)            | 8 KiB   | `X68000SUPER` | `7ac5c8fa53d2693ee61ada293efd1f681b1390ef50c1117ddcf52d2280468c20` |
| IPL V1.1 (`iplromxv.dat`)                 | 128 KiB | `X68000XVI`   | `06d3d6365d2b4079abf37d362a393f9224e472b8321e1826fef0a263d9e26590` |
| Compatible internal SCSI (`scsiinco.bin`) | 8 KiB   | `X68000XVI`   | `08e08002db7e47bdf6f2f60066f7253eb94791fb2aa17b392e26d23d72e0c19f` |

For the original model, the IC12 image supplies even bytes and IC11 supplies
odd bytes. Their assembled 128 KiB IPL has BLAKE3
`fe7832b87d5bb5f8d56d9f1d697ef9bb94c446334e17105e574c8314b7602d32`.
The XVI SCSI image is accepted as a named compatibility substitute and produces
a startup warning.

## Sharp X1

The Sharp X1 targets need a real ROM set, pointed to by `--x1-roms`. Each model has
its own MAME set (`x1`, `x1turbo`); extract whichever you need into the directory
(they can share it, and the files that appear in both sets are byte-identical). ROMs
are identified by their BLAKE3 content hash rather than by file name, so any dump
layout works.

Each model requires its IPL boot ROM, its 8x8 character generator, and the 8x16 ANK
font. The turbo additionally requires the four kanji ROMs. The ANK font is
byte-identical across both sets, so a single file can satisfy the matching slot for
both models.

| Slot              | Size   | Contents                    | Required for            | BLAKE3                                                             |
|-------------------|--------|-----------------------------|-------------------------|--------------------------------------------------------------------|
| `ipl` (X1)        | 4 KiB  | IPL boot ROM                | `X1`                    | `194f351bc1024188162856e2374d92bc608d9c742ca007d8c19a4b4eed44abbc` |
| `ipl` (X1turbo)   | 32 KiB | IPL boot ROM                | `X1TURBO`               | `871c77226a6e65bf1820c0a3e6f63a330cb1d2eb6c135fc9e4da9741ce38106c` |
| `cgrom` (X1)      | 2 KiB  | 8x8 character generator     | `X1`                    | `61440d736fdec066b825428f4d26fbdb04b3a4fcc7f05bbdd4b5bbe9e55318c3` |
| `cgrom` (turbo)   | 2 KiB  | 8x8 character generator     | `X1TURBO`               | `f26c67af04f3b4819e0bd474ded7b083e3d370a62ea0672f09787b8ca4ebc4a6` |
| `ank`             | 8 KiB  | 8x16 ANK font               | both models             | `a8695470e98492a2d969ba3fdeee76ee9b3573f525eee20f98627fb5e98279a0` |
| `kanji1`          | 32 KiB | Kanji ROM, first quarter    | `X1TURBO`               | `212d081a600377a1068d56f4049d03916ea705465eb2feca950b6df186a12ba4` |
| `kanji2`          | 32 KiB | Kanji ROM, second quarter   | `X1TURBO`               | `0bd59d087b3197c8136e5664e311234930ec566b61d184204144f04a84ba769b` |
| `kanji3`          | 32 KiB | Kanji ROM, third quarter    | `X1TURBO`               | `f2495255441c15bfce5c7441f6d94809d4f0e0dba1c7f43f9153991e326b881a` |
| `kanji4`          | 32 KiB | Kanji ROM, fourth quarter   | `X1TURBO`               | `84e0afa27e1f4ef01b6e5dac452835f487c98968e14fceaac3c93331524b51d7` |

## Fujitsu FM-7 / FM-77AV

The FM-7 targets need a real ROM set, pointed to by `--fm7-roms`. Each model has
its own MAME set (`fm7`, `fm77av`); extract whichever you need into the directory
(they can share it, and the files that appear in both sets are byte-identical).
ROMs are identified by their BLAKE3 content hash rather than by file name, so any
dump layout works.

The FM-7 requires the F-BASIC 3.0 ROM, the BASIC and DOS boot ROMs, and the
type-C sub monitor; its kanji ROM slot is optional (kanji reads return open bus
when it is absent). The FM-77AV requires the initiator ROM, its F-BASIC 3.0 ROM,
all three sub monitors plus the sub CG font, and the kanji ROM. The sub monitor C
and kanji ROMs are byte-identical across both sets, so a single file satisfies
the matching slot for both models.

| Slot               | Size    | Contents                | Required for                 | BLAKE3                                                             |
|--------------------|---------|-------------------------|------------------------------|--------------------------------------------------------------------|
| `fbasic` (FM-7)    | 31 KiB  | F-BASIC v3.0 ROM        | `FM7`                        | `059a5c926109fc156f07d91aaad05307ff0bd9d3eb5bffa805d554863f4a01bc` |
| `fbasic` (FM-77AV) | 31 KiB  | F-BASIC v3.0 ROM        | `FM77AV`                     | `276f3953b3f8fe975d29d13463261d9e70ce9c339d2af12536cf2010ae0f2a8d` |
| `boot_bas`         | 512 B   | Boot ROM, BASIC mode    | `FM7`                        | `d6a8dda5482a337e28aaf7b838be0543411277ba17f260ae62f9f1af46592b2d` |
| `boot_dos`         | 512 B   | Boot ROM, DOS mode      | `FM7`                        | `fbc9e9240f810deb8e28207b7a3362486f5f57294fb7ff8225628286479d26f3` |
| `initiate`         | 8 KiB   | Initiator ROM           | `FM77AV`                     | `4ac5111f650f4415763c1e0d9f6b997432f80c5ba9b60a38b68b308dcea9f404` |
| `subsys_c`         | 10 KiB  | Sub monitor type C + CG | both models                  | `55b0e4f72561ea0fafe6353376642d70595b08989a2d76c2b6423c7d85a9d1d2` |
| `subsys_a`         | 8 KiB   | Sub monitor type A      | `FM77AV`                     | `413b20a42227ddf95e153685cc989dcf03b193aaf79f3429848db899bd6635e3` |
| `subsys_b`         | 8 KiB   | Sub monitor type B      | `FM77AV`                     | `edf5fc537af21d93c73d3446e44654fbab0106edaf85f564abfad99bd28590e1` |
| `subsyscg`         | 8 KiB   | Sub CG font ROM         | `FM77AV`                     | `7b430d28aebaf260a823e8585c31dacc2aaca9d4f69ab34672a1ded0b37cfd23` |
| `kanji`            | 128 KiB | JIS level-1 kanji ROM   | `FM77AV` (optional on `FM7`) | `482b314f15b6a063e06a8c3e6e7426d4de9b8513086ab0e72ff0ea1623ac51f6` |

## MIDI: Roland MT-32

Place your MT-32 ROM files into a single directory and point `--mt32-roms` at it.
ROMs are identified by size and BLAKE3 hash, so file names do not matter. You need one
control ROM and one PCM ROM. Split ROM pairs (two halves) are also supported and merged
automatically.

Recommended: MT-32 control ROM v1.07 (`MT32_CONTROL.ROM`
`8f123c1f...`) with the MT-32 PCM ROM (`MT32_PCM.ROM` `7805996b...`). The control ROM
version determines the emulated model:

| Model                     | Control ROM versions                  |
|---------------------------|---------------------------------------|
| MT-32                     | v1.04, v1.05, v1.06, v1.07, BlueRidge |
| MT-32 (new / "old" v2)    | v2.04, v2.06, v2.07                   |
| CM-32L / LAPC-I           | v1.00, v1.02                          |
| CM-32LN / CM-500 / LAPC-N | v1.00                                 |

The MT-32 control ROM versions v1.04, v1.05, v1.06 and v1.07 have the best
compatibility; v1.07 is recommended.

Full ROMs (a single control ROM plus a single PCM ROM is enough):

| Description                             | Type    | Size    | BLAKE3                                                             |
|-----------------------------------------|---------|---------|--------------------------------------------------------------------|
| MT-32 Control v1.04                     | Control | 64 KiB  | `9102699229706ff459a718924884559d50a6a8749a2d27fe58548f3c0606f66a` |
| MT-32 Control v1.05                     | Control | 64 KiB  | `6b05c40c21d67c6780c39dac669dc7869d2b9fbde62bfc73a03ec3634282658f` |
| MT-32 Control v1.06                     | Control | 64 KiB  | `93e8a9bd5fdea0f3e92d9a9949e307bc98dc7d9ff7650b28d9dbfd2e863054bb` |
| MT-32 Control v1.07 (recommended)       | Control | 64 KiB  | `8f123c1f38104a2a7eb1df35fd5b26ca1b857185086a87233b355510264602bf` |
| MT-32 Control BlueRidge                 | Control | 64 KiB  | `af3cc9fe2f9844adde07377af66b4e1b0636df499abf4f2cdba716bb886642ad` |
| MT-32 Control v2.04                     | Control | 128 KiB | `788364d4f8dbe7577f092ef944418461b65bdbd449e2808a3403e28e90c4ee5d` |
| MT-32 Control v2.06                     | Control | 128 KiB | `3bd5adf2aba6f5bd9a85d52dc164b2c0efd3c8e69b7cf058d4dcc644c85d98b3` |
| MT-32 Control v2.07                     | Control | 128 KiB | `eb32a5640adba7da5e5cc2b8a455cf709d9f8998f3a5b5f2f2aa948c0ff3a9e0` |
| CM-32L / LAPC-I Control v1.00           | Control | 64 KiB  | `d88dcc0e94864040bd5933d89a29afd5a156eb43fec416ae1add5c02e565b9ff` |
| CM-32L / LAPC-I Control v1.02           | Control | 64 KiB  | `136741df33c185e809b057ee82b71ad94a07e82925fb0b7941bdd5912be6f549` |
| CM-32LN / CM-500 / LAPC-N Control v1.00 | Control | 64 KiB  | `0037be2e04ee72b01de1577b996887cd4258ddb538a433b52de8f60829e06ce1` |
| MT-32 PCM ROM                           | PCM     | 512 KiB | `7805996b758fab5469e96d9a28588eb2e991440242372f7546345cdc66c8d97a` |
| CM-32L / CM-64 / LAPC-I PCM ROM         | PCM     | 1 MiB   | `5e4839e75ec9e9b03eca0c0eacf4d4b551e76504c72c10325a311bd9ea1309e7` |

Half dumps (merged automatically into the corresponding full ROM):

| Description                 | Half   | Size    | BLAKE3                                                             |
|-----------------------------|--------|---------|--------------------------------------------------------------------|
| MT-32 Control v1.04 (a)     | Mux0   | 32 KiB  | `3b0bdc08828f383711334a5db13252b98df79cbd9fa7a21cd37e55355dd41963` |
| MT-32 Control v1.04 (b)     | Mux1   | 32 KiB  | `a3feacf1522d04d283fcb20c262f8cdfe469a667eb4e4689899b168660923993` |
| MT-32 Control v1.05 (a)     | Mux0   | 32 KiB  | `2d970225f29d20dc38ef47e48db1ded49ee223f27a6b8c0e9072b55ebe85aa0f` |
| MT-32 Control v1.05 (b)     | Mux1   | 32 KiB  | `a6d5c9d616cf23b8fdf06f86a8c1a3116b4bf71985ca337cca21ba517614dd04` |
| MT-32 Control v1.06 (a)     | Mux0   | 32 KiB  | `ad9dd4a7eec18b561ca9bfdf446730ff55019dc8e9a20b0e6de3a9c721282e68` |
| MT-32 Control v1.06 (b)     | Mux1   | 32 KiB  | `7bd393b0b2dec1ee98b06357eb3849aa903bdd57595b0d1b409530e2a963269a` |
| MT-32 Control v1.07 (a)     | Mux0   | 32 KiB  | `d8f51c813aebfa8f47a20ec8d5dc1bd870720b19d2ae43a20eea19612fb249d1` |
| MT-32 Control v1.07 (b)     | Mux1   | 32 KiB  | `31f0fa94dda0bb836106b53c21d78e016bf970ea794a6cd84dbf4bd852ada51e` |
| MT-32 Control BlueRidge (a) | Mux0   | 32 KiB  | `848350fb882dbffafaa18fa4c100c2c63fec6ddc99ac62243dcf7acf86594397` |
| MT-32 Control BlueRidge (b) | Mux1   | 32 KiB  | `46a2c0b8ee01ed06a73bb3cfaee40199e8fcb51162e1504c32bb33dc32935dbb` |
| MT-32 PCM (low)             | First  | 256 KiB | `5ced158f0131b5170219cd69d438288321810004f06148eda275c11d3c488bfb` |
| MT-32 PCM (high)            | Second | 256 KiB | `22a2f889408003c128a28a9672c11655444b2c777955114ba87d0fba5822d035` |
| CM-32L PCM (low)            | First  | 512 KiB | `5ced158f0131b5170219cd69d438288321810004f06148eda275c11d3c488bfb` |
| CM-32L PCM (high)           | Second | 512 KiB | `991388440296b3ae2664f9f620667b64120ba862a1cda23e0701859693830397` |

## MIDI: Roland SC-55

Place the ROM files for your device model into a single directory and point
`--sc55-roms` at it. Unlike every other ROM set, the SC-55 is matched by file name
(and size), not content hash; the emulator auto-detects the model from the file names
present and the first matching set wins.

Recommended: SC-55 v1.21 (mk1) - place `sc55_rom1.bin`, `sc55_rom2.bin`,
`sc55_waverom1.bin`, `sc55_waverom2.bin`, and `sc55_waverom3.bin`.

| Model                    | Required files                                                                                       |
|--------------------------|------------------------------------------------------------------------------------------------------|
| SC-55mk2 / SC-155mk2     | `rom1.bin`, `rom2.bin`, `rom_sm.bin`, `waverom1.bin`, `waverom2.bin`                                 |
| SC-55st                  | `rom1.bin`, `rom2_st.bin`, `rom_sm.bin`, `waverom1.bin`, `waverom2.bin`                              |
| SC-55 (mk1, recommended) | `sc55_rom1.bin`, `sc55_rom2.bin`, `sc55_waverom1.bin`, `sc55_waverom2.bin`, `sc55_waverom3.bin`      |
| CM-300 / SCC-1 / SCC-1A  | `cm300_rom1.bin`, `cm300_rom2.bin`, `cm300_waverom1.bin`, `cm300_waverom2.bin`, `cm300_waverom3.bin` |
| JV-880                   | `jv880_rom1.bin`, `jv880_rom2.bin`, `jv880_waverom1.bin`, `jv880_waverom2.bin`                       |
| SCB-55 / RLP-3194        | `scb55_rom1.bin`, `scb55_rom2.bin`, `scb55_waverom1.bin`, `scb55_waverom2.bin`                       |
| RLP-3237                 | `rlp3237_rom1.bin`, `rlp3237_rom2.bin`, `rlp3237_waverom1.bin`                                       |
| SC-155                   | `sc155_rom1.bin`, `sc155_rom2.bin`, `sc155_waverom1.bin`, `sc155_waverom2.bin`, `sc155_waverom3.bin` |

The MT-32 and SC-55 emulations are optional build features. See
[Build requirements](../README.md#build-requirements) for how to enable or disable them
and [License](../README.md#license) for the licensing implications.
