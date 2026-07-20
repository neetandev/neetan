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
