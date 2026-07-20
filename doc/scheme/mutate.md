# (neetan mutate 1)

```scheme
(import (neetan mutate 1))
```

Writes into the emulated machine's registers and address spaces. Requires the
machine's `mutate` capability (and the per-register or per-address-space
mutability reported by [`(neetan inspect 1)`](inspect.md)). Every operation is
validated in full before any state changes, so a write never happens partially.
Each procedure takes a `machine` handle from [`(neetan automation 1)`](automation.md).

- `(register-set! machine processor register value)` -> unspecified. Sets one
  register to a non-negative integer `value`.
- `(memory-write-bytevector! machine space address bytes)` -> unspecified.
  Writes the bytevector `bytes` into `space` starting at `address`.
- `(memory-poke-unsigned! machine space address width byte-order value)` ->
  unspecified. Writes a `width`-byte unsigned `value`. `byte-order` is `little`,
  `big`, or `native`.
