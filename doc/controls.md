# Controls

## Emulator shortcuts

Right Ctrl is the shortcut modifier of the emulator. The emulated machine uses
Left Ctrl.

| Key                | Action                                                 |
|--------------------|--------------------------------------------------------|
| Right Ctrl + M     | Toggle mouse capture                                   |
| Right Ctrl + Esc   | Quit the emulator. A game of the manager returns to it |
| Right Ctrl + Enter | Toggle fullscreen                                      |
| Right Ctrl + R     | Hard reset                                             |
| Right Ctrl + F     | Fast forward 8x (hold)                                 |
| Right Ctrl + C     | Toggle CRT effect                                      |
| Right Ctrl + S     | Cycle scaling method                                   |
| Right Ctrl + P     | Cycle composite phase (PC-6000)                        |
| Right Ctrl + 1     | Open floppy selector for drive 1                       |
| Right Ctrl + 2     | Open floppy selector for drive 2                       |
| Right Ctrl + 3     | Open CD-ROM selector                                   |
| Right Ctrl + 5     | Capture quick save                                     |
| Right Ctrl + 9     | Restore quick save                                     |

## Mouse

Games with mouse support need a captured mouse pointer. `Right Ctrl + M` captures
the pointer and a second `Right Ctrl + M` releases it.

## Quick save

The quick save is one runtime snapshot kept only in memory. `Right Ctrl + 5`
replaces the previous snapshot and `Right Ctrl + 9` restores it. A restore switches
the removable drives back to the floppy, CD-ROM and cassette images that were mounted
at capture time. Guest writes to writable media remain on the media.

The snapshot is never written to disk and is lost when Neetan exits. A quick save
does not replace the save function of the emulated software or a save to disk.

## Key bindings

Keys are remapped in a configuration file. See
[Key bindings](configuration.md#key-bindings).
