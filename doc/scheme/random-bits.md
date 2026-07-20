# (r7rs random-bits)

```scheme
(import (r7rs random-bits))   ; also available as (srfi 27)
```

Sources of random bits, giving both a convenient default source and the ability
to create, seed, and snapshot independent random sources. This is SRFI 27.

Note: results are deterministic only if the script seeds a source explicitly
(for example with `random-source-pseudo-randomize!`). Reproducibility is the
script author's responsibility.

## Convenience procedures

- `(random-integer n)` - a random integer in the range 0 to n-1 from the
  default source -> integer.
- `(random-real)` - a random real strictly between 0 and 1 from the default
  source -> real.
- `default-random-source` - the random source backing random-integer and
  random-real.

## Sources

- `(make-random-source [seed])` - create a new random source -> random source.
  With no seed the source is seeded from the host wall clock. Pass an exact
  integer seed for a reproducible source.
- `(random-source? object)` - `#t` if object is a random source.
- `(random-source-state-ref source)` - return an externalizable snapshot of the
  source's internal state.
- `(random-source-state-set! source state)` - restore a source to a previously
  snapshotted state.
- `(random-source-randomize! source [seed])` - reseed the source. With no seed
  it is reseeded from the host wall clock. Pass an exact integer seed for a
  reproducible source.
- `(random-source-pseudo-randomize! source i j)` - reseed the source
  deterministically from the two indices i and j.

## Source-bound generators

- `(random-source-make-integers source)` - return a procedure `(lambda (n) ...)`
  drawing integers in 0 to n-1 from source.
- `(random-source-make-reals source [unit])` - return a procedure
  `(lambda () ...)` drawing reals strictly between 0 and 1 from source. The
  optional unit, in the open interval (0, 1), sets the resolution of the drawn
  reals.
