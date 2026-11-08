# (neetan automation 1)

```scheme
(import (neetan automation 1))
```

The core Neetan automation library. It builds and drives an emulated machine,
advances emulated time at exact frame boundaries, injects input, mounts media,
reads the screen, and reports the one pass or fail result of a script. Almost
every procedure takes an opaque `machine` handle as its first argument, obtained
from `call-with-machine` or `with-machine`.

## Conventions

- The `machine` argument is an opaque record. Passing anything else raises
  `neetan/argument`; using a handle after its scope has closed raises
  `neetan/stale-handle`.
- Counts, indices, coordinates, and tick/frame limits are non-negative exact
  integers. Keys, controls, buttons, media types, and reset kinds are symbols.
  Paths and messages are strings.
- Procedures ending in `!` mutate machine or session state. Procedures ending
  in `?` are predicates.
- Several procedures take a trailing optional association list of options.
  Unknown keys, duplicate keys, malformed entries, or a non-list raise
  `neetan/argument`.
- The default per-frame tick budget is `50000000`. When an options alist gives a
  `frames` limit but no `ticks` limit, the tick limit is derived as
  `frames * 50000000`.
- A run outcome is an alist with keys `stop-reason`, `ticks`, `frames`,
  `overshoot-ticks`, `epoch`, `current-tick`, and `current-frame`.
- Calling an operation the machine does not support raises `neetan/unsupported`.

## Versioning and host configuration

- `(neetan-api-version)` -> two values `major minor`. The automation API version
  (`1 0`). Destructure with `receive` from `(r7rs receive)`.
- `(host-config)` -> alist. All common host settings exposed by `--config`: the
  per-family ROM directories, `mt32-roms`, `sc55-roms`, `roms-folder`, `artifact-root`,
  `guest-time`, `timeout`, and `audio-sample-rate`. Read-only.
- `(config-value key)` / `(config-value key default)` -> value. Looks up one
  host-config `key`; returns `default` (or `#f`) when absent.
- `(alist-ref alist key)` / `(alist-ref alist key default)` -> value. Checked
  lookup in a symbol-keyed alist; rejects malformed entries, non-symbol keys,
  and duplicate keys. Raises `neetan/argument` on a missing key with no default.
- `(alist-key? alist key)` -> boolean. Whether `key` is present in `alist`.

## Machine construction and lifecycle

- `(call-with-machine specification procedure)` -> the procedure's values.
  Builds a machine from the `specification` alist, calls
  `(procedure machine)`, and closes the machine on every unwind via
  `dynamic-wind`. The specification takes `target`, `model`, `media` and
  the machine settings of the configuration file under their file keys, for
  example `(soundboard . |26k|)`. See the machine-spec keys table in the API
  index.
- `(with-machine (machine specification) body ...)` -> the body's values.
  (syntax) Sugar over `call-with-machine` that binds `machine` over `body`.
- `(machine? value)` -> boolean. Whether `value` is a machine handle.
- `(machine-state? value)` -> boolean. Whether `value` is a save-state handle.

## Machine information and capabilities

- `(machine-info machine)` -> alist. Keys `target`, `model`, `api-version`,
  `epoch`, `timebase` (`ticks-per-second-numerator` and
  `ticks-per-second-denominator`), `audio-sample-rate`, and `capabilities`.
- `(machine-capabilities machine)` -> alist. Capability descriptor: `keyboard`,
  `mouse`, `mouse-buttons`, `joystick`, `joystick-ports`, `cartridge`,
  `cassette`, `hard-disk`, `printer`, `mt32`, `sc55`, `floppy-drives`,
  `cdrom`, `trace`, and `trace-schema-version`.
- `(machine-capability? machine capability)` -> boolean. Convenience predicate
  over the descriptor. Recognized symbols: `keyboard`, `mouse`, `joystick`,
  `cartridge`, `cassette`, `hard-disk`, `printer`, `mt32`, `sc55`,
  `floppy-drives` (at least one floppy drive), `cdrom`, `save-state`,
  `inspect`, `mutate`, `trace`.

## Timeline counters

- `(machine-epoch machine)` -> integer. The epoch, incremented on hard reset and
  startup restoration.
- `(machine-tick machine)` -> integer. Session-total machine ticks.
- `(machine-frame machine)` -> integer. Session-total presented frames.
- `(machine-epoch-tick machine)` -> integer. Ticks since the last epoch change.
- `(machine-epoch-frame machine)` -> integer. Frames since the last epoch change.
- `(machine-emulated-time-ns machine)` -> integer. Emulated time in nanoseconds.
- `(machine-shutdown-requested? machine)` -> boolean. Whether the guest has
  requested shutdown.

## Bounded execution

- `(run-ticks! machine count)` -> run outcome. Advances exactly `count` machine
  ticks; may overshoot by one indivisible CPU operation, reported in
  `overshoot-ticks`.
- `(run-frames! machine count)` / `(run-frames! machine count maximum-ticks)`
  -> run outcome. Advances to `count` exact presentation boundaries.
  `maximum-ticks` defaults to `count * 50000000`. A tick-limit stop raises
  `neetan/timeout`; guest shutdown raises `neetan/guest-shutdown`.
- `(run-until-frame! machine frame)` / `(run-until-frame! machine frame options)`
  -> run outcome. Advances until the absolute session `frame`. Options key
  `ticks` overrides the derived limit.
- `(wait-until machine predicate)` / `(wait-until machine predicate options)`
  -> boolean. Checks `(predicate)` before advancing and after each frame,
  returning `#t` on a match or `#f` when a bound is exhausted. Options keys
  `frames` (default `1800`) and `ticks`. Predicate exceptions propagate.

## Reset and restore

- `(reset! machine kind)` -> unspecified. `kind` is `hard` or `soft`. A hard
  reset reconstructs volatile hardware while keeping mounted media; a soft reset
  asserts the machine's documented reset line. Hard reset advances the epoch and
  invalidates save states.
- `(restore-startup! machine)` -> unspecified. Reconstructs the machine from its
  original specification, re-reads the startup media from their fixture files,
  releases all controls, and discards runtime media changes. Advances the epoch
  and invalidates save states.

## Save states

- `(save-state machine)` -> opaque state. Captures an in-memory runtime state.
  Unsupported machines raise `neetan/unsupported`.
- `(restore-state! machine state)` -> unspecified. Restores `state` into the same
  machine instance without advancing the epoch. A state from another machine
  raises `neetan/stale-handle`.
- `(discard-state! state)` -> unspecified. Releases a save state.
- `(call-with-saved-state machine procedure)` -> the procedure's values. Saves a
  state, calls `(procedure state)`, and discards the state on every unwind.

## Result

- `(execution-result 'OK)` / `(execution-result 'ERROR message)` -> unspecified.
  Sets the one authoritative script result. The first call wins and is terminal;
  a second call raises `neetan/result-state`. The `(neetan test 1)` library sets
  this for you.

## Keyboard and text input

- `(key-down! machine key)` -> unspecified. Presses and holds a logical `key`.
- `(key-up! machine key)` -> unspecified. Releases a logical `key`.
- `(key-tap! machine key)` / `(key-tap! machine key options)` -> run outcome.
  Presses `key`, advances the requested frames, then releases it. Options keys
  `frames` (default `2`) and `ticks`. The key is always released on unwind.
- `(type-text! machine text)` / `(type-text! machine text options)` ->
  unspecified. Types an ASCII string (printable ASCII plus carriage return, tab,
  and backspace). Options keys `frames` (default `2`, applied per character) and
  `ticks` (per character). Unsupported characters fail before any injection.

See the logical key-name table in the API index for the full `key` vocabulary.

## Joystick and mouse

- `(joystick-set! machine index control pressed?)` -> unspecified. Sets one
  joystick `control` on port `index` to `pressed?`.
- `(joystick-clear! machine index)` -> unspecified. Releases all controls on
  port `index`.
- `(mouse-move! machine delta-x delta-y)` -> unspecified. Accumulates relative
  mouse motion (signed integers) until the next execution chunk.
- `(mouse-button! machine button pressed?)` -> unspecified. Sets a mouse button
  (`left`, `right`, or `middle`).

See the joystick-control table in the API index for the `control` vocabulary.

## Media

- `(media-insert! machine type slot path)` -> media alist. Mounts media at a
  zero-based `slot`. Floppy and hard disk images are read into memory. The guest
  writes to that copy and the fixture file stays unchanged.
- `(media-eject! machine type slot)` -> unspecified. Ejects media from a slot.
- `(media-save! machine type slot artifact-path)` -> artifact alist (`path`,
  `bytes`). Writes the current image of the floppy or hard disk in `slot`,
  guest writes included, to `artifact-path` beneath the artifact root. `type` is
  `floppy` or `hdd`. The file keeps the format of the mounted image, and the
  extension of `artifact-path` must name that format (`.d88` for a D88 floppy,
  `.hdi` for an HDI hard disk). An empty slot or a wrong extension raises
  `neetan/argument`. Other media types and a machine that cannot read back its
  image raise `neetan/unsupported`. A failed write raises `neetan/io`.
- `(media-info machine type slot)` -> media alist or `#f`. Reports `type`,
  `slot`, `format`, `description`, `source`, `private` (the printer output path,
  otherwise `#f`), and `write-protected`. `#f` when the slot is empty.
- `(media-create! machine type slot format)` -> media alist. Builds a zeroed
  floppy or hard disk in memory and mounts it in `slot`. No host file is read
  or written. `type` is `floppy` or `hdd`. The `format` symbol names the disk:
  - For `hdd` it is a size from the `create-hdd` table of the
    [disk image tooling](../disk-images.md#create-hdd), such as `sasi20`,
    `ide40` or `at100`. Dashes are optional. `sasi-20` names the same size.
  - For `floppy` it is a type from the `create-fdd` table of the
    [disk image tooling](../disk-images.md#create-fdd): `|2hd|`, `|2hd144|`,
    `|2dd|`, `|2dd720|` or `|2d|`. These names start with a digit. Scheme
    reads `2hd` as a number, and the vertical bars make it a symbol.

  The format must be one of the blank images that the drive of the model
  takes in the game editor. A format of another model raises
  `neetan/argument`. A drive without blank images raises
  `neetan/unsupported`. The media alist reports `source` as `blank:<size>`
  for a hard disk and as `blank:<type>.<extension>` for a floppy. It reports
  `format` as the image format: `HDI` for SASI and IDE, `H0` for SCSI,
  `HDF` for the X68000 sizes and `HDD` for the PC/AT sizes. A floppy takes the
  container of the first blank floppy image of the model:

  | Family                                              | Floppy format |
  |-----------------------------------------------------|---------------|
  | PC-98, PC-88, PC-88VA, PC-6000, FM Towns, X1, FM-7  | `D88`         |
  | MSX                                                 | `DSK`         |
  | X68000                                              | `XDF`         |
  | PC/AT                                               | `IMG`         |

- `(media-format! machine type slot)` / `(media-format! machine type slot table)`
  -> `#t`. Formats the floppy or hard disk in `slot` in place. The guest sees
  one empty FAT volume. `type` is `floppy` or `hdd`. An empty slot raises
  `neetan/argument`. Call it before the guest boots from or reads the disk.
  - A hard disk gets the partition table, a boot sector, both FATs and an
    empty root directory. The machine family selects the partition table:
    `pc98` on the PC-98 and `at` on the PC/AT. The optional `table` symbol
    names it explicitly. A table of another family raises `neetan/argument`.
    Other machine families raise `neetan/unsupported`.
  - A floppy gets a boot sector, both FATs and an empty root directory in the
    FAT12 layout of the machine family. A floppy takes no `table`, and a
    `table` raises `neetan/argument`. A geometry outside the table below and
    the other machine families raise `neetan/unsupported`.

  | Family   | Floppy geometries                                              |
  |----------|----------------------------------------------------------------|
  | PC-98    | 2HD 1.2 MB (`\|2hd\|`), 2DD 640 KB (8 sectors of 512 bytes)     |
  | FM Towns | 2HD 1.2 MB (`\|2hd\|`), 2HC 1.2 MB, 2HD 1.44 MB (`\|2hd144\|`) |
  | PC/AT    | 2HD 1.44 MB (`\|2hd144\|`), 2DD 720 KB (`\|2dd720\|`)          |
  | MSX      | 2DD 720 KB (`\|2dd720\|`)                                      |

See the media-type and slot-count table in the API index.

### Media checkpoints

`media-save!` turns a slow setup step into a file that later scripts mount
directly. A game installation runs once onto a fresh hard disk and saves it:

```scheme
(with-machine (machine '((model . pc9801vm)
                         (media . ((floppy 0 "install-a.d88")))))
  (media-create! machine 'hdd 0 'sasi40)
  (media-format! machine 'hdd 0)
  ;; ... drive the installer to its end ...
  (media-save! machine 'hdd 0 "installed.hdi"))
```

The default artifact root `<script-dir>/artifacts/<stem>` lies beneath the
script directory. A test script next to `install.scm` mounts the result with a
path relative to its own directory:

```scheme
(with-machine (machine '((model . pc9801vm)
                         (media . ((hdd 0 "artifacts/install/installed.hdi")))))
  (check-true (wait-for-screen machine "title.png")))
```

A save disk for a game works the same way with a floppy:

```scheme
(with-machine (machine '((model . pc9801vm)))
  (media-create! machine 'floppy 1 '|2hd|)
  (media-format! machine 'floppy 1)
  ;; ... let the game write its save data to drive 2 ...
  (media-save! machine 'floppy 1 "save-disk.d88"))
```

## Screen and display

- `(screen-available? machine)` -> boolean. Whether a framebuffer has been
  presented yet. Other screen reads fail with `neetan/argument` before this.
- `(screen-size machine)` -> two values `width height`.
- `(screen-rgba machine)` -> bytevector. A copy of tightly packed RGBA8 pixels.
- `(screen-pixel machine x y)` -> four values `red green blue alpha`.
- `(screen-hash machine)` -> string. 64 lowercase hex BLAKE3 characters over a
  format tag, dimensions, and the RGBA bytes.
- `(save-screenshot! machine artifact-path)` -> artifact alist (`path`, `bytes`).
  Encodes the current framebuffer to a PNG beneath the artifact root.
- `(screen-matches? machine expected-path)` /
  `(screen-matches? machine expected-path options)` -> boolean. Compares the
  screen against an expected PNG. Options key `tolerance` (default `0.0`, exact).
  Writes nothing, so it is safe inside a `wait-until` loop.
- `(screen-region-matches? machine expected-path x y width height)` /
  `(screen-region-matches? machine expected-path x y width height options)`
  -> boolean. Region variant of `screen-matches?`. Options key `tolerance`.
- `(wait-for-screen machine expected-path)` /
  `(wait-for-screen machine expected-path options)` -> boolean. Advances at
  presentation boundaries until the framebuffer matches (`#t`) or a bound is
  exhausted (`#f`, and writes `<expected-stem>-compare.png`). Options keys
  `tolerance` (default `0.0`), `frames` (default `1800`), and `ticks`.

Tolerance is a real in `[0.0, 1.0]`; a match is normalized RGB RMSE
`<= tolerance`. Image matching requires exact dimensions.
