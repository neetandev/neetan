# Configuration

Neetan stores its settings in plain `key = value` files. Lines starting with `#` or
`;` are comments. See [`configuration/default.conf`](../configuration/default.conf)
for a complete reference with all keys and their defaults. The game manager reads
and writes the same files.

## Folder layout

The folder that holds the settings file `neetan.conf` is the root. The settings file of
the SDL pref path uses the current directory as the root:

```
<root>/
  neetan.conf              ; host settings: config-folder, roms-folder, backend, audio-volume, ...
  config/
    pc98.conf              ; PC-98 system defaults
    pc98/
      albatross.conf       ; one file for each game
    pc88.conf
    pc88/
    ...
  roms/
    at/ fm7/ msx/ mt32/ pc6000/ pc88/ pc88va/ pc98/ sc55/ towns/ x1/ x68k/
```

The release archives ship this layout with an empty `neetan.conf`. The unpacked
folder is a portable installation.

The system names are `at`, `fm7`, `msx`, `pc6000`, `pc88`, `pc88va`, `pc98`, `towns`,
`x1` and `x68k`. `config-folder` selects the config folder and defaults to `config`.
`roms-folder` selects the ROM folder and defaults to `roms`. Every ROM key defaults to
the folder of its system below the ROM folder, for example `pc98-roms` to
`roms/pc98`. The derived folders of the PC-98, the PC/AT, the MT-32 and the SC-55
apply only when they exist. See [ROMs](roms.md) for the ROM sets of each folder.

`gui-font` selects the font file of the user interface. Its default
`font/SarasaUiJ-Regular.ttf` is searched below the root and then in the resource folder
of the installation: the folder of the executable, `../share/neetan` on Linux and
`../Resources` on macOS.

A relative path inside a configuration file resolves against the root. A relative
path given as a command line option resolves against the current directory.

## Settings file

The settings file is the first match of:

1. `--settings <PATH>` on the command line. The file must exist.
2. `neetan.conf` in the current directory.
3. `neetan.conf` beside the executable, for a portable installation.
    For an AppImage this is the folder of the `.AppImage` file.
4. `neetan.conf` in the SDL pref path.

| OS      | SDL pref path                                    |
|---------|--------------------------------------------------|
| Linux   | `~/.local/share/neetan/neetan/`                  |
| Windows | `C:\Users\<user>\AppData\Roaming\neetan\neetan\` |
| macOS   | `~/Library/Application Support/neetan/neetan/`   |

Without a settings file the root is the current directory and every setting keeps its
built-in default. The log shows the settings file and the root at startup.

## Layers

A launch builds its configuration from these layers. A later layer overrides an
earlier one:

1. The built-in defaults.
2. The settings file.
3. The system defaults `<config-folder>/<system>.conf`.
4. The game files of `--config`, in command line order.
5. The command line options.

The `machine` of the latest layer that sets it selects the system, and with it the
system defaults file. A missing system defaults file is fine. A `machine` key of
another system inside a system defaults file is ignored with a warning.

The first `fdd1`, `fdd2` or `cdrom` value of a layer replaces the images of the
earlier layers. Further values in the same layer append images. An empty value such
as `fdd1 =` drops the images of the earlier layers. An empty single image key such as
`hdd1 =` drops the image of the earlier layers. An empty ROM key falls back to its
derived folder.

The host keys `config-folder`, `roms-folder`, `gui-font`, the ROM keys, `backend`,
`gui-theme`, `gui-language` and `audio-volume` apply only from the settings file and the
command line. A system defaults file or a game file that sets one gets a warning and
the line is ignored. A media key such as `fdd1` or `hdd1` in a system defaults file
applies to every game of the system that does not set the key.

```ini
# A game file, for example config/pc98/albatross.conf
machine = PC9801RA
soundboard = 86+26k
force-gdc-clock = 2.5
fdd1 = games/98/disk_a.d88
fdd1 = games/98/disk_b.d88
fdd2 = games/98/save_game.d88
hdd1 = games/98/harddrive.hdi
midi = mt32
```

```bash
neetan --config config/pc98/albatross.conf --soundboard sb16
```

This command runs the PC9801RA of the game file with the sb16 sound board of the
option. The images resolve against the root.

## Machine selection

Each system has a default model, for example `PC9801RA` for the PC-98. A game file
without a `machine` key in a system folder of the config folder runs the `machine` of
`<config-folder>/<system>.conf`, else a `machine` of the same system in the settings
file, else the default model of the system. For a `--config` file outside the system
folders, the `machine` of the settings file selects the system. The `machine` of
`<config-folder>/<system>.conf` then wins over it.

## Key bindings

Keys are remapped in a configuration file with `key.<HostKey> = <Machine Key>`
entries. Family-specific bindings belong in the system defaults
`<config-folder>/<system>.conf` or in the game file. See
[`configuration/default.conf`](../configuration/default.conf) for the host key names,
the machine key names and the default mappings.
