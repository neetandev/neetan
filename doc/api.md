# Neetan automation Scheme API

This is the reference for the Scheme API used to write automation scripts and
tests for the Neetan emulator. Scripts run under `neetan-auto`, a deterministic,
headless frontend that builds a machine, drives it at exact frame boundaries, injects
input, reads the screen, and reports one pass or fail result.

Scripts are R7RS-small Scheme programs.

## Running neetan-auto

```text
neetan-auto run <SCRIPT> [OPTIONS] [-- <SCRIPT-ARG>...]
neetan-auto orchestrate <DIR> [OPTIONS]
```

`run` executes one `.scm` script and streams its output. `orchestrate`
discovers every `.scm` script under a directory and runs them in parallel.
Options shared by both:

```text
--config <PATH>          Common host settings (ROM locations, etc.)
--global-config <PATH>   Explicit optional configuration layer
--artifacts <PATH>       Artifact root (default: <script-dir>/artifacts/<stem>)
--timeout <SECONDS>      Per-script wall-clock deadline (default: 600)
--guest-time <DATETIME>  Fixed guest RTC value (default: 2000-01-01T00:00:00)
--help / --version
```

`orchestrate` additionally accepts `--jobs <N>` (default: CPU count). The
`--config` layer supplies only common host settings (chiefly the per-family ROM
directories). Each script builds its own machine. Configuration order is
`defaults -> --global-config -> --config`.

### Exit codes (run)

```text
0    script set (execution-result 'OK)
1    script set (execution-result 'ERROR ...) or a bounded wait timed out
2    command-line, configuration, or machine-construction error
3    Scheme compile or runtime error
4    internal error, or the script set no result
124  cooperative timeout or external cancellation
```

## Libraries

Every library below is registered and active by default.
Import the ones a script uses.

Neetan machine libraries:

- [(neetan automation 1)](scheme/automation.md) - build and drive a machine,
  input, media, screen, save states. The core library.
- [(neetan test 1)](scheme/test.md) - `test-suite` / `test-case` and checks.
- [(neetan inspect 1)](scheme/inspect.md) - read registers and memory.
- [(neetan mutate 1)](scheme/mutate.md) - write registers and memory.
- [(neetan trace 1)](scheme/trace.md) - trace hardware and firmware events.

R7RS standard library:

- [(scheme base)](scheme/base.md) - core data types, syntax, and procedures.
- [R7RS standard libraries](scheme/standard-libraries.md) - `(scheme char)`,
  `(scheme inexact)`, `(scheme file)`, `(scheme write)`, and the rest.

SRFI extensions (importable by their `(r7rs ...)` alias or `(srfi N)` name):

- [(r7rs lists)](scheme/lists.md) - SRFI 1 list library.
- [(r7rs sorting)](scheme/sorting.md) - SRFI 132 sort library.
- [(r7rs strings)](scheme/strings.md) - SRFI 152 string library.
- [(r7rs bitwise-operations)](scheme/bitwise-operations.md) - SRFI 151.
- [(r7rs basic-hash-table)](scheme/basic-hash-table.md) - SRFI 69 hash tables.
- [(r7rs random-bits)](scheme/random-bits.md) - SRFI 27 random sources.
- [(r7rs intermediate-format-strings)](scheme/format-strings.md) - SRFI 48
  `format`.
- [(r7rs ascii)](scheme/ascii.md) - SRFI 175 ASCII library.
- [(r7rs bytevector)](scheme/bytevector.md) - R6RS bytevectors.
- [(r7rs and-let*)](scheme/and-let-star.md) - SRFI 2.
- [(r7rs receive)](scheme/receive.md) - SRFI 8.
- [(r7rs cut)](scheme/cut.md) - SRFI 26.

## Quick start

A minimal script builds a machine, waits for an expected screen, and lets the
test suite set the result:

```scheme
(import (scheme base)
        (neetan automation 1)
        (neetan test 1))

(test-suite "My game"
  (with-machine (machine '((model . pc9801vm)
                           (media . ((floppy 0 "game.d88")))))
    (test-case "title screen renders"
      (check-true (wait-for-screen machine "title.png")))))
```

Run it with `neetan-auto run my-game.scm --config roms.conf`. Expected PNGs and
other data resolve beneath the script's directory. Screenshots and comparison
images are written beneath the artifact root.

## Full example

It selects a model and its media, waits for two title screens, and confirms
music by matching a traced I/O write:

```scheme
(import (scheme base)
        (neetan automation 1)
        (neetan trace 1)
        (neetan test 1)
        (r7rs receive)                      ; receive, for multiple values
        (r7rs intermediate-format-strings)) ; format, for readable messages

;; The suite owns the result. The ROM directory comes from --config.
;; This script only selects the model and its media.
(test-suite "PC-98 Dragon Knight"
  (with-machine (machine '((target . pc98)
                           (model . pc9801vm)
                           (sound-board . |26k|)
                           (media . ((floppy 0 "disk-a.hdm")
                                     (floppy 1 "disk-b.hdm")))))

    (test-case "company and game titles render"
      (check-true (wait-for-screen machine "company-title.png"
                    '((tolerance . 0.01) (frames . 1800))))
      (receive (width height) (screen-size machine)
        (note (format #f "title reached at ~ax~a" width height)))
      (check-true (wait-for-screen machine "game-title.png")))

    (test-case "26K music plays"
      ;; I/O port 0x0188 (392) selects YM2203 register 0x28 (FM key-on); 0x018A
      ;; writes the key-on data. Matching either write confirms FM output.
      (check-true
        (pair?
          (wait-for-event machine
            '((class . access)
              (data . ((space . cpu.main.io)
                       (operation . write)
                       (address . 392)
                       (value . 40)
                       (handled? . #t))))))))))
```

## Argument vocabulary

### Machine specification keys

A specification is an alist passed to `call-with-machine` / `with-machine`.

```text
target        machine family symbol (see targets below)
model         concrete model symbol. Selects the family on its own
media         list of (type slot path) entries
cpu-mode      family-specific CPU mode
sound-board   family-specific sound board (e.g. |26k|, |86|)
graphic-board family-specific graphics board
boot-device   family-specific boot device
boot-mode     family-specific boot mode
bios          on or off
midi          family-specific MIDI device
```

`target` and `model` must agree when both are given. Valid values for the
family-specific keys are documented per machine in the `doc/machine-*.md` files.
Targets: `pc98`, `pc88`, `pc88va`, `pc60`, `msx`, `towns`, `x1`, `fm7`, `x68k`,
`at`.

### Logical key names

Used by `key-down!`, `key-up!`, and `key-tap!`. Text conversion for `type-text!`
covers printable ASCII plus carriage return, tab, and backspace. Names are
symbols. Several accept aliases (shown with `/`).

```text
letters      a b c ... z
digits       0 1 2 ... 9
punctuation  space  minus  equals/caret  backslash
             nonusbackslash/underscore  grave/at  leftbracket  rightbracket
             semicolon  comma  period  slash  apostrophe/colon
editing      esc/escape  bs/backspace  tab  return/enter  ins/insert
             del/delete  home  end/help
navigation   up  down  left  right  pageup/rollup  pagedown/rolldown
             pause/stop  printscreen/copy  application/nfer/muhenkan
keypad       kp0 ... kp9  kpminus  kpdivide  kpmultiply  kpplus
             kpcomma  kpperiod  kpenter
function     f1 ... f10  f11/vf1/xf1  f12/vf2/xf2  f13/vf3/xf3
             f14/vf4/xf4  f15/vf5/xf5
modifiers    shift/leftshift  rightshift  ctrl/control/leftcontrol
             rightcontrol  alt/leftalt/grph/graph  rightalt/xfer/henkan/convert
             caps/capslock  numlock/kana
japanese     international1/ro  international2/katakana  international3/yen
             international4  international5
```

### Joystick controls

Used by `joystick-set!`.

```text
up  down  left  right
trigger1/button-a  trigger2/button-b  button-c  button-x  button-y  button-z
run/start  select
```

### Mouse buttons

Used by `mouse-button!`: `left`, `right`, `middle`.

### Media types and slot counts

Used by `media-insert!`, `media-eject!`, and `media-info`. Slots are zero-based.

```text
floppy/fdd        2 slots
hdd/hard-disk     2 slots
cdrom/cd          1 slot
cartridge/cart    1 slot
cassette/tape     1 slot
printer           1 slot
```

### Reset kinds and byte orders

- `reset!` kinds: `hard`, `soft`.
- Byte orders for `memory-peek-unsigned` / `memory-poke-unsigned!`: `little`,
  `big`, `native`.

### Error symbols

Expected failures raise a Scheme error object whose first irritant is one of
these stable symbols, so they can be caught with `guard`:

```text
neetan/argument        neetan/range          neetan/no-machine
neetan/machine-state   neetan/stale-handle   neetan/result-state
neetan/timeout         neetan/guest-shutdown neetan/unsupported
neetan/path-escape     neetan/io             neetan/trace-overflow
neetan/trace-state     neetan/assertion      neetan/test-state
```

Catch one like this:

```scheme
(guard (condition
        ((and (error-object? condition)
              (memq 'neetan/timeout (error-object-irritants condition)))
         (note "timed out")))
  (run-frames! machine 600))
```

## Cookbook

### Wait for a title, then press start

```scheme
(check-true (wait-for-screen machine "title.png" '((frames . 1800))))
(key-tap! machine 'return)
(check-true (wait-for-screen machine "menu.png"))
```

### Type a BASIC command

```scheme
(type-text! machine "RUN\r")
```

### Compare only part of the screen

```scheme
(check-true
  (screen-region-matches? machine "hud.png" 0 0 320 32 '((tolerance . 0.02))))
```

### Poll a condition without touching the screen result

```scheme
;; screen-matches? writes nothing, so it is safe inside wait-until.
(check-true
  (wait-until machine
    (lambda () (screen-matches? machine "ready.png" '((tolerance . 0.03))))
    '((frames . 1200))))
```

### Read and write memory

```scheme
(import (neetan inspect 1) (neetan mutate 1))

(define lives (memory-peek-unsigned machine 'cpu.main.memory #xC000 1 'little))
(memory-poke-unsigned! machine 'cpu.main.memory #xC000 1 'little 99)
```

### Trace an I/O write

```scheme
(import (neetan trace 1))

(define event
  (wait-for-event machine
    '((class . access)
      (data . ((space . cpu.main.io) (operation . write))))
    '((frames . 300))))
```

### Save and restore a state around an experiment

```scheme
(call-with-saved-state machine
  (lambda (state)
    (key-tap! machine 'space)
    (run-frames! machine 60)
    (restore-state! machine state)))  ; back to the saved point
```
