# Disk images

## Floppy disk image formats

| Format  | Extensions                     | Description                                        |
|---------|--------------------------------|----------------------------------------------------|
| D88     | `.d88`, `.d98`, `.88d`, `.98d` | Standard PC-98 disk image with per-sector metadata |
| HDM     | `.hdm`                         | Headerless raw sector image (2HD only)             |
| NFD     | `.nfd`                         | T98Next format with per-sector metadata            |
| 2D      | `.2d`                          | Headerless raw sector image (Sharp X1, 2D only)    |
| D77     | `.d77`                         | Fujitsu FM-7 disk image, byte-compatible with D88  |
| MSX DSK | `.dsk`                         | Headerless raw MSX floppy image                    |
| DIM     | `.dim`                         | X68000 DIFC.X container                            |
| XDF     | `.xdf`, `.2hd`                 | Headerless raw sector image (X68000, 2HD only)     |
| IMG     | `.img`, `.ima`                 | Headerless raw IBM PC floppy image                 |
| IBM XDF | `.xdf`, `.img`, `.ima`         | IBM Extended Density Format, detected by file size |

Writes made by the emulated software, for example game saves, are written back to the
source file for all formats.

## CD-ROM disc image formats

CD-ROM discs apply to the PC-9821, PC-8801, PC/AT, FM Towns, and X68000 SUPER/XVI targets.

| Format  | Extensions | Description                                                                                          |
|---------|------------|------------------------------------------------------------------------------------------------------|
| CUE/BIN | `.cue`     | CUE sheet referencing a raw BIN image                                                                |
| CloneCD | `.ccd`     | CloneCD control file with sibling `.img` (raw 2352-byte sectors) and optional `.sub` subchannel data |

## Cartridge and cassette images

The PC-6000 and the MSX take cartridge ROM images with `--cartridge`. The MSX selects
the cartridge mapper automatically. The cassette formats are `.cas` on the MSX,
`.cas`, `.p6` and `.p6t` on the PC-6000, `.tap` on the X1 and `.t77` on the FM-7.

## Multiple disk images

Many games ship on several floppy disks and ask for a disk swap during play. Some
CD-ROM games come as several disc images. Each drive takes a list of images. The
first image of each list is inserted at startup.

Use the `--fdd1`, `--fdd2` and `--cdrom` options more than once to register all
images of a drive:

```bash
neetan --fdd1 floppy_disk1.d88 --fdd1 floppy_disk2.d88 --fdd1 floppy_disk3.d88
neetan --cdrom disc1.cue --cdrom disc2.cue
```

The same in a configuration file:

```ini
fdd1 = floppy_disk1.d88
fdd1 = floppy_disk2.d88
fdd1 = floppy_disk3.d88
cdrom = disc1.cue
cdrom = disc2.cue
```

Press `Right Ctrl + 1` (drive 1), `Right Ctrl + 2` (drive 2), or `Right Ctrl + 3`
(CD-ROM) to open the image selector and swap disks at runtime.

## Disk image tools

The game editor of the manager creates blank floppy and hard disk images. The
command line offers the same and more.

### create-fdd

`create-fdd <PATH> [OPTIONS]` creates an empty floppy disk image. A `.d88` path
produces a D88 image of every type. A raw extension produces a headerless image:
`.xdf` or `.2hd` for `2hd` (X68000), `.img` or `.ima` for `2hd144` (PC/AT), and
`.dsk`, `.img` or `.ima` for `2dd720` (MSX and PC/AT).

| Option          | Description                                                                             | Default |
|-----------------|-----------------------------------------------------------------------------------------|---------|
| `--type <TYPE>` | `2hd` (1232 KB), `2hd144` (1440 KB), `2dd` (640 KB), `2dd720` (720 KB) or `2d` (320 KB) | `2hd`   |

### create-hdd

`create-hdd <PATH> --type <TYPE>` creates an empty hard disk image. Use a `.hdi`
path for SASI/IDE images, a `.h0`-`.h4` path for raw SCSI images, a `.hdf` path
for the headerless X68000 types, and a `.hdd` path for the PC/AT flat images.
`--type` is required.

| Interface   | `--type` values                                                |
|-------------|----------------------------------------------------------------|
| SASI        | `sasi5`, `sasi10`, `sasi15`, `sasi20`, `sasi30`, `sasi40`      |
| IDE         | `ide40`, `ide80`, `ide120`, `ide200`, `ide500`                 |
| SCSI        | `scsi20`, `scsi40`, `scsi100`, `scsi200`, `scsi340`, `scsi540` |
| X68000 SASI | `x68sasi10`, `x68sasi20`, `x68sasi40`                          |
| X68000 SCSI | `x68scsi20`, `x68scsi40`                                       |
| PC/AT IDE   | `at40`, `at100`, `at250`, `at504`                              |

### convert-hdd

`convert-hdd <INPUT> <OUTPUT>` converts a PC-98 hard disk image between the SASI and
IDE formats. The input may be an HDI, NHD, or THD image. The output is always an HDI
image and its path must end in `.hdi`.

The sector size of the input selects the direction: 256 bytes is SASI and 512 bytes
is IDE. The command picks the smallest target geometry that holds the data. Every
SASI image fits into `ide40`. An IDE image larger than `sasi40` (about 40 MB) cannot
be converted to SASI.

### copy

`copy <SOURCE> <DEST>` copies files and directories between the host filesystem and
FAT-formatted disk images. Each argument is either a host path or an `IMAGE:DOSPATH`
reference such as `roms/dos620.hdi:A:\PROGS\FILE.EXE`. At least one side must be an
image. Image-to-image copies are supported.

```bash
neetan copy ./readme.txt roms/disk.hdi:A:\README.TXT
neetan copy roms/disk.hdi:A:\PROGS\FOO.EXE ./extracted/
neetan copy src.hdi:A:\FOO.EXE dst.hdi:A:\FOO.EXE
```

Directories are copied recursively. There is no `-r` flag. DOS paths must use 8.3
ASCII filenames. Longer names are rejected before any file is written. The recognized
image extensions are `hdi`, `nhd`, `thd`, `hdd` (hard disks) and `d88`, `d98`, `88d`,
`98d`, `hdm`, `nfd`, `2d`, `img`, `ima`, `dsk` (floppies).
