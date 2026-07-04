# ROMs

- [How ROM loading works](#how-rom-loading-works)
- [PC-9801 / PC-9821](#pc-9801--pc-9821)
- [PC-8001 / PC-8801](#pc-8001--pc-8801)
- [PC-88VA2](#pc-88va2)
- [PC-6001 / PC-6601](#pc-6001--pc-6601)
- [FM Towns](#fm-towns)
- [MIDI: Roland MT-32](#midi-roland-mt-32)
- [MIDI: Roland SC-55](#midi-roland-sc-55)

## How ROM loading works

Every machine points its ROM option at a single directory. neetan scans that
directory (non-recursively), reads each file, and matches it by size and BLAKE3
content hash against the tables below. File names do not matter and stray or
wrong-sized files are simply ignored, so any dump layout works.

```
TODO: We do not want to have these exceptions. We need to use blake3 everywhere. 
      I collected the SC-55 Rom files we have access to here: 
[Roland SC-55 v1.21 ROMs.zip](../reference/roms/Roland%20SC-55%20v1.21%20ROMs.zip)
[Roland SC-55mkII ROMs.zip](../reference/roms/Roland%20SC-55mkII%20ROMs.zip)
[Roland SC-155 Rev1.zip](../reference/roms/Roland%20SC-155%20Rev1.zip)
```

There are two exceptions where file names are used: the FM Towns loose ROM set
and every Roland SC-55 set are matched by file name (see those sections).

| Machine            | Option           | Required?                        |
|--------------------|------------------|----------------------------------|
| PC-9801 / PC-9821  | `--pc98-roms`    | Optional (HLE BIOS by default)   |
| PC-8001 / PC-8801  | `--pc88-roms`    | Required                         |
| PC-88VA2           | `--pc88va-roms`  | Required                         |
| PC-6001 / PC-6601  | `--pc6000-roms`  | Required                         |
| FM Towns           | `--towns-roms`   | Required                         |
| Roland MT-32       | `--mt32-roms`    | Required for `--midi mt32`       |
| Roland SC-55       | `--sc55-roms`    | Required for `--midi sc55`       |

## PC-9801 / PC-9821

The PC-98 targets run on a built-in HLE BIOS and a built-in font by default, so a
ROM set is optional. Point `--pc98-roms` at a directory of dumps and pass `--bios`
to boot the model's real BIOS instead of the HLE BIOS.

With `--bios` the model's BIOS is required. The PC-9821 targets are the exception:
they have no real-BIOS boot path and always fall back to HLE with a warning. The 26K
sound ROM is loaded when a PC-9801-26K board is selected. A font ROM is best-effort:
the model's preferred dump is used when present, otherwise the built-in font is kept.

BIOS ROM (192 KiB dual-bank ITF + BIOS image, one per model):

| Model      | Size    | BLAKE3                                                             |
|------------|---------|--------------------------------------------------------------------|
| `PC9801F`  | 192 KiB | `5587b89b968b005e81ea2bb4c2ef6fc762154d589e627920e3d9be9cd3e01b06` |
| `PC9801VM` | 192 KiB | `4377eeba8410c57f9a313ed2d24cd929cbfb7cac40244d5c6cafd1a27bf3495e` |
| `PC9801VX` | 192 KiB | `89ff271aa046bb6428761cdc3ec92d82e87350c5a4941974293c5b7fe2238aed` |
| `PC9801RA` | 192 KiB | `f18e91e8097661efe4543f30558383a02021047acfaa6d0a78e06d025094aa5e` |
| `PC9821AS` | -       | HLE only (no real BIOS)                                            |
| `PC9821AP` | -       | HLE only (no real BIOS)                                            |

Font ROM (V98 format, 282 KiB). If none are provided, uses the open source fallback font.
Any of these dumps is accepted for any model. Each model just prefers the one matching its family:

| Dump         | Preferred by     | BLAKE3                                                             |
|--------------|------------------|--------------------------------------------------------------------|
| standard     | F / VM / VX / RA | `4b6f751f34e633e072ded2a109c25ddb90ac70350792dc55914a4cefa4dbe005` |
| PC-9821As    | `PC9821AS`       | `a567134a3d5c2a215b9573ee07b5204fff243631052e7a40be340e863aff8eef` |
| PC-9821Ap2   | `PC9821AP`       | `7fb96af345c33f9bd7be5c22f75c650ac41da9b543ca5f9ca7b3d3906f2abb40` |
| PC-9801UX    | fallback         | `3c1efa858b80fc11bb7482bdc5e15004dd9a015d7d22d48159cd43ed63f540dc` |
| PC-9821Ce2   | fallback         | `b38096265c76cf9f54cb47df905cfb6c8b4d4f27019a04835bbc3dc8782d33e1` |

Sound ROM (loaded when a PC-9801-26K board is selected):

| Slot    | Size   | BLAKE3                                                             |
|---------|--------|--------------------------------------------------------------------|
| `sound` | 16 KiB | `93816a6e42ed9a10135af634ed500e10b1d266e0b4158d3f8471910609255e24` |

## PC-8001 / PC-8801

The PC-8801MC needs a real ROM set, pointed to by `--pc88-roms`.

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

| Slot         | Size   | Required by boot mode | BLAKE3                                                             |
|--------------|--------|-----------------------|--------------------------------------------------------------------|
| `n80_mkii`   | 32 KiB | `n80`                 | `9e4ec9c53f4432a88583dccd04ae3186f4d7849f80ea7774ac1efbdb93c992f2` |
| `n80_mkiisr` | 32 KiB | `n80sr`               | `56406a79fd664a197c458cb3feeeb6994c34266a1e02728877b6ea5ef86e15ba` |
| `n80sr`      | 40 KiB | `n80sr`               | `7b81e27b831ad00f264170d1d98c645298fa688b07d5a9f0c19c1d6a73fe4273` |

## PC-88VA2

The PC-88VA2 needs a real ROM set, pointed to by `--pc88va-roms`. All slots are
required:

| Slot         | Size    | Contents                  | BLAKE3                                                             |
|--------------|---------|---------------------------|--------------------------------------------------------------------|
| `rom00`      | 512 KiB | ROM0 low image (varom00)  | `bba5011412fb266b3c15ff08d2508716ba2ac54fec3aa172b59e441486807eab` |
| `rom08`      | 128 KiB | ROM0 high image (varom08) | `4cdf3da9a1423e874f9618a8d8859107fa5e3d20a91f4dcf908e042763c41bbb` |
| `rom1`       | 128 KiB | ROM1 image (varom1)       | `1239bf390d444ff205f70c700527cb50bc90107904050fa8713a415a17bf0e42` |
| `font`       | 320 KiB | Kanji / font ROM          | `b47ec9f55ff199ac71f453385aec0f370afbb958fd47ad9bb5161bdf4e2bb3ee` |
| `dictionary` | 512 KiB | Dictionary (jisyo) ROM    | `21fcd88c97b881e55f015f22d62002022189572e171f1c5e485b751c84379b30` |
| `subsys`     | 8 KiB   | Floppy sub-CPU (Z80) ROM  | `531ab2aa2c7d7c4deb2ddd8303c6637ea7e273648825fb51e17c8660d7496565` |

## PC-6001 / PC-6601

The PC-6000 targets need a real ROM set, pointed to by `--pc6000-roms`. Each model
requires its boot ROM (BASIC or, on the SR models, the system ROM) and its base
character generator; the kanji, extended character generator, and voice ROMs are
loaded when present. Several dumps are bit-identical across models (the kanji ROM,
the SR system ROM halves), so a single file can satisfy more than one slot.

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

The FM Towns targets need a real ROM set, pointed to by `--towns-roms`. Both the
FM Towns II CX and MX use the FM Towns II MX ROM dump. Two layouts are accepted.

The merged set is the packed 2 MiB MAME BIOS image plus the 32-byte serial ROM.
These two files are matched by name:

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

## MIDI: Roland MT-32

Place your MT-32 ROM files into a single directory and point `--mt32-roms` at it.
ROMs are identified by size and BLAKE3 hash, so file names do not matter. You need
one control ROM and one PCM ROM. Split ROM pairs (two halves) are also supported and
merged automatically.

The control ROM version determines the emulated model:

| Model                     | Control ROM versions                  |
|---------------------------|---------------------------------------|
| MT-32                     | v1.04, v1.05, v1.06, v1.07, BlueRidge |
| MT-32 (new / "old" v2)    | v2.04, v2.06, v2.07                   |
| CM-32L / LAPC-I           | v1.00, v1.02                          |
| CM-32LN / CM-500 / LAPC-N | v1.00                                 |

Currently the MT-32 control ROM versions v1.04, v1.05, v1.06 and v1.07 have the best
compatibility.

Full ROMs (a single control ROM plus a single PCM ROM is enough):

| Description                             | Type    | Size    | BLAKE3                                                             |
|-----------------------------------------|---------|---------|--------------------------------------------------------------------|
| MT-32 Control v1.04                     | Control | 64 KiB  | `9102699229706ff459a718924884559d50a6a8749a2d27fe58548f3c0606f66a` |
| MT-32 Control v1.05                     | Control | 64 KiB  | `6b05c40c21d67c6780c39dac669dc7869d2b9fbde62bfc73a03ec3634282658f` |
| MT-32 Control v1.06                     | Control | 64 KiB  | `93e8a9bd5fdea0f3e92d9a9949e307bc98dc7d9ff7650b28d9dbfd2e863054bb` |
| MT-32 Control v1.07                     | Control | 64 KiB  | `8f123c1f38104a2a7eb1df35fd5b26ca1b857185086a87233b355510264602bf` |
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
`--sc55-roms` at it. Unlike every other ROM set, the SC-55 is matched by **file
name** (and size), not content hash; the emulator auto-detects the model from the
file names present and the first matching set wins.

| Model                   | Required files                                                                                       |
|-------------------------|------------------------------------------------------------------------------------------------------|
| SC-55mk2 / SC-155mk2    | `rom1.bin`, `rom2.bin`, `rom_sm.bin`, `waverom1.bin`, `waverom2.bin`                                 |
| SC-55st                 | `rom1.bin`, `rom2_st.bin`, `rom_sm.bin`, `waverom1.bin`, `waverom2.bin`                              |
| SC-55 (mk1)             | `sc55_rom1.bin`, `sc55_rom2.bin`, `sc55_waverom1.bin`, `sc55_waverom2.bin`, `sc55_waverom3.bin`      |
| CM-300 / SCC-1 / SCC-1A | `cm300_rom1.bin`, `cm300_rom2.bin`, `cm300_waverom1.bin`, `cm300_waverom2.bin`, `cm300_waverom3.bin` |
| JV-880                  | `jv880_rom1.bin`, `jv880_rom2.bin`, `jv880_waverom1.bin`, `jv880_waverom2.bin`                       |
| SCB-55 / RLP-3194       | `scb55_rom1.bin`, `scb55_rom2.bin`, `scb55_waverom1.bin`, `scb55_waverom2.bin`                       |
| RLP-3237                | `rlp3237_rom1.bin`, `rlp3237_rom2.bin`, `rlp3237_waverom1.bin`                                       |
| SC-155                  | `sc155_rom1.bin`, `sc155_rom2.bin`, `sc155_waverom1.bin`, `sc155_waverom2.bin`, `sc155_waverom3.bin` |

The MT-32 and SC-55 emulations are optional build features. See
[Build requirements](../README.md#build-requirements) for how to enable or disable
them and [License](../README.md#license) for the licensing implications.
