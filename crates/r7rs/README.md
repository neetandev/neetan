# r7rs

An embeddable Rust implementation of the [R7RS-small](https://r7rs.org/)
Scheme dialect. Internally it is a register based bytecode interpreter.

## Values and type system

The runtime implements the R7RS-small value families: booleans, Unicode
characters, numbers, the empty list and pairs, symbols, mutable strings,
mutable bytevectors, mutable vectors, procedures and continuations, promises,
parameters, records, ports, environments, error objects, the end-of-file
object, and the unspecified value. A procedure may return zero, one, or
several values.

### Numeric tower

R7RS numeric predicates form an inclusion hierarchy. This implementation
supports every level of that hierarchy, within the fixed ranges described
below:

```text
number
└── complex
    ├── non-real complex
    └── real
        └── rational
            └── integer
```

Every implemented number satisfies `complex?`. A complex number with a zero
imaginary component is normalized to a real number. Every finite implemented
real satisfies `rational?`, and a finite inexact real with no fractional part
also satisfies `integer?`. Exactness is orthogonal to this hierarchy:

- Exact integers are signed `i128` values. Values in the `i64` range are stored
  inline. Wider values are stored on the heap. Staying in the `i64` range
  provides the best performance.
- Non-integral exact rationals are normalized fractions with an `i64`
  numerator and a positive `i64` denominator. A fraction that reduces to an
  integer may use the full `i128` integer range.
- Inexact reals are IEEE-754 `f64` values, including infinities, NaNs, and
  signed zero.
- Complex numbers use two real components. They are exact only when both
  components are exact. Rectangular exact complex numbers are supported.
  Polar construction and transcendental operations produce inexact results.

The runtime deliberately does not implement bignums, arbitrary-precision
rationals, arbitrary-precision inexact reals, or exact irrational numbers.
Checked exact arithmetic never wraps. If an exact integer result exceeds
`i128`, or a non-integral rational result cannot fit its `i64` components, the
engine reports an `ImplementationRestriction`. Conversion from an inexact
number to an exact number can fail for the same reason. Mixing an exact number
with an inexact number generally produces an inexact result.

Accordingly, the engine advertises the `ieee-float` and `exact-complex`
features, but not `ratios` or `exact-closed`.

## Embedding

Add the crate to your application, create an isolated `Engine`, then compile and
evaluate Scheme source:

```rust
use r7rs::{Engine, EngineConfig};

fn main() -> Result<(), r7rs::Error> {
    let mut scheme = Engine::new(EngineConfig::default())?;
    let module = scheme.compile("embedded.scm", "(+ 20 22)")?;
    let value = scheme.eval(&module)?.into_one()?;

    assert_eq!(scheme.write_root(&value)?, "42");
    Ok(())
}
```

Compiled modules belong to the engine that created them. Returned `Root`s keep
Scheme values alive across garbage collections.

Expose Rust functions through an engine-local library, then wrap that private
interface in an ordinary Scheme library:

```rust
use r7rs::{Engine, EngineConfig, LibraryName, LibraryNameComponent, Value};

fn main() -> Result<(), r7rs::Error> {
    let mut scheme = Engine::new(EngineConfig::default())?;
    let internal = LibraryName::new([
        LibraryNameComponent::identifier("example"),
        LibraryNameComponent::identifier("internal"),
        LibraryNameComponent::number(1),
    ])?;
    scheme.register_library_fn(&internal, "%host-inc", 1..=1, |cx, args| {
        let value = cx.to_i128(args[0])? + 1;
        cx.integer(value)
    })?;
    let public = LibraryName::new([
        LibraryNameComponent::identifier("example"),
        LibraryNameComponent::identifier("host"),
    ])?;
    scheme.register_library_source(
        public,
        "example-host.sld",
        "(define-library (example host)
           (export inc)
           (import (scheme base) (only (example internal 1) %host-inc))
           (begin (define (inc value) (%host-inc value))))",
    )?;

    let module = scheme.compile("callback.scm", "(import (example host)) (inc 41)")?;
    let value = scheme.eval(&module)?.into_one()?;

    assert_eq!(scheme.write_root(&value)?, "42");
    Ok(())
}
```

Names and bindings in a host's private native libraries are implementation
details and carry no compatibility guarantee.

## Extensions

Optional extension libraries, SRFIs and other standard Scheme libraries, are
enabled per engine at runtime, so each engine opts in independently. Install
one with `install_extension`:

```rust
use r7rs::{Engine, EngineConfig, Extension};

fn main() -> Result<(), r7rs::Error> {
    let mut scheme = Engine::new(EngineConfig::default())?;
    scheme.install_extension(Extension::Srfi27)?;

    let module = scheme.compile("dice.scm", "(import (srfi 27)) (random-integer 6)")?;
    let value = scheme.eval(&module)?.into_one()?;

    assert!((0..6).contains(&value.value().as_fixnum().unwrap()));
    Ok(())
}
```

Installing an extension also enables its `cond-expand` feature identifier
(`srfi-1` for SRFI 1), so guest code can detect it through `cond-expand` and
`features`. `Extension::ALL` lists every extension this build offers, which is
convenient for enabling them all in a loop.

Each installed extension is importable under its canonical name and under a
discoverable alias in the `(r7rs ...)` namespace. Both names provide the
identical library.

| Extension                             | Canonical name        | Alias                                |
|---------------------------------------|-----------------------|--------------------------------------|
| SRFI 1 (List Library)                 | `(srfi 1)`            | `(r7rs lists)`                       |
| SRFI 2 (AND-LET*)                     | `(srfi 2)`            | `(r7rs and-let*)`                    |
| SRFI 8 (Receive)                      | `(srfi 8)`            | `(r7rs receive)`                     |
| SRFI 26 (cut/cute)                    | `(srfi 26)`           | `(r7rs cut)`                         |
| SRFI 27 (Sources of Random Bits)      | `(srfi 27)`           | `(r7rs random-bits)`                 |
| SRFI 48 (Intermediate Format Strings) | `(srfi 48)`           | `(r7rs intermediate-format-strings)` |
| SRFI 69 (Basic Hash Tables)           | `(srfi 69)`           | `(r7rs basic-hash-table)`            |
| SRFI 132 (Sort Libraries)             | `(srfi 132)`          | `(r7rs sorting)`                     |
| SRFI 151 (Bitwise Operations)         | `(srfi 151)`          | `(r7rs bitwise-operations)`          |
| SRFI 152 (String Library)             | `(srfi 152)`          | `(r7rs strings)`                     |
| SRFI 175 (ASCII Character Library)    | `(srfi 175)`          | `(r7rs ascii)`                       |
| Bytevectors (R6RS)                    | `(scheme bytevector)` | `(r7rs bytevector)`                  |

`EngineConfig::default()` grants no filesystem, source-loading, process, or
clock authority. Its standard ports are engine-local: `current-input-port` is
at end of file and the output ports accumulate silently, so guest output never
reaches the host unless the embedder installs a port resource through
`set_standard_input`, `set_standard_output`, or `set_standard_error`. Install
capabilities explicitly for a sandboxed embed, or use
`EngineConfig::standalone()` (with the default `host-capabilities` feature) for
conventional host access, which connects the standard ports to the process
standard streams. Resource limits and interruption are configured through
`Limits` and `InterruptToken`.

### SRFI 1 (List Library)

SRFI 1 (List Library) provides the full list-processing vocabulary the
R7RS-small base lacks.

### SRFI 2 (AND-LET*)

SRFI 2 (AND-LET*) provides `and-let*`, a short-circuiting `and` whose clauses can
bind their non-`#f` results for use in later clauses and the body.

### SRFI 8 (Receive)

SRFI 8 (Receive) provides `receive`, a concise syntax for binding the multiple
values of an expression to variables before evaluating a body.

### SRFI 26 (cut/cute)

SRFI 26 (cut/cute) provides `cut` and `cute`, a compact notation for specializing
some of a procedure's arguments without writing a `lambda`.

### SRFI 27 (Sources of Random Bits)

SRFI 27 (Sources of Random Bits) is backed by a non-cryptographic random number
generators. One deliberate deviation from the specification: `(make-random-source)`
with no argument, `(random-source-randomize! s)` with no seed, and
`default-random-source` are seeded from the host wall clock rather than a fixed
state. This wall-clock access is part of installing the extension. Pass an
explicit exact integer seed to `make-random-source` or `random-source-randomize!`,
or restore a captured state with `random-source-state-set!`, for a reproducible stream.

Explicit seeds and the two indices passed to `random-source-pseudo-randomize!` use the
full exact `i128` range. Their 128-bit representations are reinterpreted as `u128`, so
negative values address the upper half of the unsigned seed space. For example, `-1`
maps to `2^128 - 1`.

### SRFI 48 (Intermediate Format Strings)

SRFI 48 (Intermediate Format Strings) provides a single procedure, `format`,
that renders a template with display, write, radix, fixed-width, and character
directives to a string or an output port.

### SRFI 69 (Basic Hash Tables)

SRFI 69 (Basic Hash Tables) provides mutable hash tables.
As an implementation-specific extension, `make-hash-table` accepts a third
optional `sizehint` argument after the equivalence predicate and hash function.
A positive hint is the initial number of bucket slots. If the hint is omitted
or is zero or negative, the table starts with the default 64 bucket slots.

### SRFI 132 (Sort Libraries)

SRFI 132 (Sort Libraries) provides stable and non-stable sorting and merging for
lists and vectors, along with neighbor-duplicate deletion, median finding, and
selection.

### SRFI 151 (Bitwise Operations)

SRFI 151 (Bitwise Operations) provides two's-complement bitwise operations on
exact integers, covering the logical family, shifting, bit counting, and the
single-bit, bit-field, and bit-sequence procedures.

### SRFI 152 (String Library)

SRFI 152 (String Library) provides a comprehensive set of string-processing
procedures: predicates, constructors, selection, padding and trimming,
comparison, prefix and suffix tests, searching, folding and mapping, and
splitting and joining. One deliberate deviation from the specification:
`string-map` is the R7RS-small procedure, so its mapper must return a character
rather than a character or a string.

### SRFI 175 (ASCII Character Library)

SRFI 175 provides ASCII-only character classification, case folding,
comparison, control-character conversion, bracket mirroring, and numeric
transformations. Procedures whose argument is named `char` accept either a
character or an exact integer. The full `i128` exact-integer range is the
implementation's `char-fix` range.

### Bytevectors (R6RS)

Bytevectors (R6RS) provides the R6RS bytevectors library under its R7RS-large
name `(scheme bytevector)`: endianness-aware integer and IEEE-754 accessors of
every width, list conversions, UTF-16 and UTF-32 string transcoding, and a
signed fill, with `bytevector-copy!` keeping the R7RS-small argument order.

## License

This crate is licensed under the [MIT No Attribution (MIT-0)](https://opensource.org/license/mit-0) license.
