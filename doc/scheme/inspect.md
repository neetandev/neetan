# (neetan inspect 1)

```scheme
(import (neetan inspect 1))
```

Read-only inspection of the emulated machine: processor registers, protected
mode state, and memory or I/O address spaces. Requires the machine's `inspect`
capability; unsupported machines and unmapped access raise a stable error. A
peek never performs a side-effecting bus read. Every procedure takes a `machine`
handle from [`(neetan automation 1)`](automation.md).

## Processors and registers

- `(processors machine)` -> list of symbols. The stable processor identifiers.
- `(processor-info machine processor)` -> alist. Keys `id`, `architecture`,
  `protected-mode`, and `registers` (each with `name`, `bits`, and `writable`).
- `(registers machine processor)` -> alist. Each register name mapped to its
  current integer value.
- `(register-ref machine processor register)` -> integer. One register's value.
- `(protected-mode-state machine processor)` -> alist. x86 protected-mode view:
  `general`, `segments`, `control`, `debug`, `descriptor-tables`, `eip`, and
  `eflags`.

## Address spaces and memory

- `(address-spaces machine)` -> list of symbols. Stable address-space
  identifiers such as `cpu.main.memory`, `cpu.main.io`, `cpu.sub.memory`, and
  `cpu.sub.io`.
- `(address-space-info machine space)` -> alist. Keys `id`, `class` (`memory` or
  `io`), `address-bits`, `byte-order`, `peekable`, and `writable`.
- `(memory-read-bytevector machine space address length)` -> bytevector. Reads
  `length` bytes from `space` at `address`.
- `(memory-peek-unsigned machine space address width byte-order)` -> integer.
  Reads a `width`-byte unsigned value. `byte-order` is `little`, `big`, or
  `native` (the address-space descriptor's order).
- `(save-memory! machine space address length path)` -> alist with `path` and
  `bytes`. Writes `length` exact guest bytes beneath the artifact root, ready
  for tools such as `ndisasm`. Path traversal and absolute paths raise
  `neetan/path-escape`.

## Text surfaces

Decoded text-mode inspection without knowing the physical text VRAM layout or
performing JIS decoding in Scheme. Reads are side-effect-free and never render
a framebuffer. On the PC-98 the main surface is `display.main`.

- `(text-surfaces machine)` -> list of symbols. The inspectable text surfaces.
- `(text-surface-info machine surface)` -> alist. Keys `id`, `rows`, and
  `columns`.
- `(text-cell machine surface row column)` -> alist. One decoded cell with
  `row`, `column`, `raw-jis`, `unicode` (a character, or `#f` when the code has
  no mapping), `attribute` (the raw hardware attribute byte), and
  `display-width` (1 or 2 columns).
- `(text-screen machine surface)` -> list of strings. Every row decoded to
  text, top to bottom. Use `text-cell` when raw attributes matter, for example
  to check reverse video separately from the glyphs.
- `(wait-for-text machine surface predicate)` /
  `(wait-for-text machine surface predicate options)` -> string or `#f`. Runs
  the machine until the decoded surface matches, returning the matched text.
  The predicate is a bare string, shorthand for `((contains . string))`, or an
  alist with a required `contains` string and an optional `row`; options are
  `frames` (default `120`) and `ticks`. The live text plane is sampled once up
  front and then at each frame boundary, so text appearing and vanishing
  within a single frame is not observed.
- `(save-text-screen! machine surface path)` -> alist with `path` and `bytes`.
  Writes the decoded rows beneath the artifact root, one line per row. Path
  traversal and absolute paths raise `neetan/path-escape`.
