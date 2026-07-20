# R7RS standard libraries

These are the remaining R7RS-small libraries beyond `(scheme base)`. All of them
are registered and active by default in neetan-auto, so you only need to import
them by name to use their exports. The large `(scheme base)` library is documented
separately in [base.md](base.md).

## (scheme case-lambda)

```scheme
(import (scheme case-lambda))
```

Procedures that dispatch on the number of arguments they receive.

- `(case-lambda (formals body ...) ...)` - create a procedure whose active clause
  is chosen by the count of arguments at each call.

## (scheme char)

```scheme
(import (scheme char))
```

Unicode-aware character and string case and classification operations.

- `(char-alphabetic? char)` - `#t` if the character is alphabetic.
- `(char-ci<=? char1 char2 ...)` - case-insensitive monotonic <= comparison.
- `(char-ci<? char1 char2 ...)` - case-insensitive monotonic < comparison.
- `(char-ci=? char1 char2 ...)` - case-insensitive equality comparison.
- `(char-ci>=? char1 char2 ...)` - case-insensitive monotonic >= comparison.
- `(char-ci>? char1 char2 ...)` - case-insensitive monotonic > comparison.
- `(char-downcase char)` - lowercase equivalent of the character -> char.
- `(char-foldcase char)` - case-folded equivalent of the character -> char.
- `(char-lower-case? char)` - `#t` if the character is a lowercase letter.
- `(char-numeric? char)` - `#t` if the character is a numeric digit.
- `(char-upcase char)` - uppercase equivalent of the character -> char.
- `(char-upper-case? char)` - `#t` if the character is an uppercase letter.
- `(char-whitespace? char)` - `#t` if the character is whitespace.
- `(digit-value char)` - numeric value of a digit character, else `#f`.
- `(string-ci<=? string1 string2 ...)` - case-insensitive string <= comparison.
- `(string-ci<? string1 string2 ...)` - case-insensitive string < comparison.
- `(string-ci=? string1 string2 ...)` - case-insensitive string equality.
- `(string-ci>=? string1 string2 ...)` - case-insensitive string >= comparison.
- `(string-ci>? string1 string2 ...)` - case-insensitive string > comparison.
- `(string-downcase string)` - lowercased copy of the string -> string.
- `(string-foldcase string)` - case-folded copy of the string -> string.
- `(string-upcase string)` - uppercased copy of the string -> string.

## (scheme complex)

```scheme
(import (scheme complex))
```

Constructors and accessors for complex numbers.

- `(angle z)` - argument (phase angle) of the complex number -> real.
- `(imag-part z)` - imaginary part of the number -> real.
- `(magnitude z)` - absolute value (modulus) of the number -> real.
- `(make-polar magnitude angle)` - complex number from polar coordinates.
- `(make-rectangular real imag)` - complex number from real and imaginary parts.
- `(real-part z)` - real part of the number -> real.

## (scheme cxr)

```scheme
(import (scheme cxr))
```

The compositions of `car` and `cdr` three and four levels deep.

- `(caaaar pair)` - `(car (car (car (car pair))))`.
- `(caaadr pair)` - `(car (car (car (cdr pair))))`.
- `(caaar pair)` - `(car (car (car pair)))`.
- `(caadar pair)` - `(car (car (cdr (car pair))))`.
- `(caaddr pair)` - `(car (car (cdr (cdr pair))))`.
- `(caadr pair)` - `(car (car (cdr pair)))`.
- `(cadaar pair)` - `(car (cdr (car (car pair))))`.
- `(cadadr pair)` - `(car (cdr (car (cdr pair))))`.
- `(cadar pair)` - `(car (cdr (car pair)))`.
- `(caddar pair)` - `(car (cdr (cdr (car pair))))`.
- `(cadddr pair)` - `(car (cdr (cdr (cdr pair))))`.
- `(caddr pair)` - `(car (cdr (cdr pair)))`.
- `(cdaaar pair)` - `(cdr (car (car (car pair))))`.
- `(cdaadr pair)` - `(cdr (car (car (cdr pair))))`.
- `(cdaar pair)` - `(cdr (car (car pair)))`.
- `(cdadar pair)` - `(cdr (car (cdr (car pair))))`.
- `(cdaddr pair)` - `(cdr (car (cdr (cdr pair))))`.
- `(cdadr pair)` - `(cdr (car (cdr pair)))`.
- `(cddaar pair)` - `(cdr (cdr (car (car pair))))`.
- `(cddadr pair)` - `(cdr (cdr (car (cdr pair))))`.
- `(cddar pair)` - `(cdr (cdr (car pair)))`.
- `(cdddar pair)` - `(cdr (cdr (cdr (car pair))))`.
- `(cddddr pair)` - `(cdr (cdr (cdr (cdr pair))))`.
- `(cdddr pair)` - `(cdr (cdr (cdr pair)))`.

## (scheme eval)

```scheme
(import (scheme eval))
```

Runtime evaluation of Scheme datums in a specified environment.

- `(eval expr environment)` - evaluate the expression in the environment
  -> result.
- `(environment list ...)` - build an environment from the named library import
  sets -> environment specifier.

*In the neetan-auto sandbox `eval` is available but rarely needed inside tests.*

## (scheme file)

```scheme
(import (scheme file))
```

Opening files and running procedures over file ports.

- `(call-with-input-file string proc)` - open the file for input, call `proc`
  with the port, close it -> result of `proc`.
- `(call-with-output-file string proc)` - open the file for output, call `proc`
  with the port, close it -> result of `proc`.
- `(delete-file string)` - delete the named file.
- `(file-exists? string)` - `#t` if the named file exists.
- `(open-binary-input-file string)` - open the file -> binary input port.
- `(open-binary-output-file string)` - open the file -> binary output port.
- `(open-input-file string)` - open the file -> textual input port.
- `(open-output-file string)` - open the file -> textual output port.
- `(with-input-from-file string thunk)` - call `thunk` with the current input
  port bound to the file -> result of `thunk`.
- `(with-output-to-file string thunk)` - call `thunk` with the current output
  port bound to the file -> result of `thunk`.

*In the neetan-auto sandbox file procedures resolve paths beneath the script
directory (data and source) or the artifact root. Absolute paths, `..` escapes,
and symlink escapes are rejected. Prefer the Neetan media and artifact APIs for
real I/O.*

## (scheme inexact)

```scheme
(import (scheme inexact))
```

Transcendental and inexact-only numeric procedures.

- `(acos z)` - arc cosine -> number.
- `(asin z)` - arc sine -> number.
- `(atan z)` / `(atan y x)` - arc tangent, two-argument form gives the angle of
  `(x, y)` -> number.
- `(cos z)` - cosine -> number.
- `(exp z)` - `e` raised to the power `z` -> number.
- `(finite? z)` - `#t` if the number is neither infinite nor NaN.
- `(infinite? z)` - `#t` if the number is an infinity.
- `(log z)` / `(log z base)` - natural logarithm, or logarithm in `base`
  -> number.
- `(nan? z)` - `#t` if the number is a NaN.
- `(sin z)` - sine -> number.
- `(sqrt z)` - principal square root -> number.
- `(tan z)` - tangent -> number.

## (scheme lazy)

```scheme
(import (scheme lazy))
```

Promises for lazy evaluation.

- `(delay expr)` - create a promise that evaluates `expr` at most once when forced.
- `(delay-force expr)` - like `delay` but supports tail-recursive promise chains
  in bounded space.
- `(force promise)` - force a promise and return its value.
- `(make-promise obj)` - wrap an already-computed value in a promise.
- `(promise? obj)` - `#t` if the object is a promise.

## (scheme load)

```scheme
(import (scheme load))
```

Loading and evaluating source files.

- `(load string)` / `(load string environment)` - read and evaluate the source
  file, optionally in the given environment.

*In the neetan-auto sandbox `load` resolves the file under the script directory.*

## (scheme process-context)

```scheme
(import (scheme process-context))
```

Access to the process command line, environment, and exit status.

- `(command-line)` - list of the command-line arguments -> list of strings.
- `(emergency-exit)` / `(emergency-exit obj)` - terminate immediately without
  running outstanding cleanup.
- `(exit)` / `(exit obj)` - terminate after running outstanding cleanup;
  the argument sets the exit status.
- `(get-environment-variable name)` - value of the named variable -> string or
  `#f`.
- `(get-environment-variables)` - all environment variables -> alist of
  name/value string pairs.

*In the neetan-auto sandbox `command-line` is backed by the script arguments
passed after `--`. `get-environment-variable` and `get-environment-variables`
work, and `exit`/`emergency-exit` work as well, but `exit` produces an orderly
executor outcome rather than tearing down the host process.*

## (scheme read)

```scheme
(import (scheme read))
```

Reading external datum representations.

- `(read)` / `(read port)` - read one datum from the current input port or
  `port` -> datum or the end-of-file object.

*In the neetan-auto sandbox `read` is available but rarely needed inside tests.*

## (scheme repl)

```scheme
(import (scheme repl))
```

Access to the interactive top-level environment.

- `(interaction-environment)` - the mutable interaction environment
  -> environment specifier.

*In the neetan-auto sandbox `interaction-environment` is available but rarely
needed inside tests.*

## (scheme time)

```scheme
(import (scheme time))
```

Access to the system clock and a monotonic tick counter.

- `(current-jiffy)` - current value of the monotonic jiffy counter -> integer.
- `(current-second)` - current time in TAI seconds -> inexact real.
- `(jiffies-per-second)` - number of jiffies in one second -> integer.

*In the neetan-auto sandbox `current-second` and `current-jiffy` come from the
fixed deterministic clock, not wall time, so tests stay reproducible.*

## (scheme write)

```scheme
(import (scheme write))
```

Writing external representations of Scheme objects.

- `(display obj)` / `(display obj port)` - write a human-readable representation,
  without quoting strings or characters.
- `(write obj)` / `(write obj port)` - write a machine-readable representation
  using datum labels for cyclic structure.
- `(write-shared obj)` / `(write-shared obj port)` - like `write` but labels all
  shared structure, not just cycles.
- `(write-simple obj)` / `(write-simple obj port)` - like `write` but assumes no
  shared structure and uses no datum labels.

## (scheme r5rs)

```scheme
(import (scheme r5rs))
```

The historical R5RS surface, both procedures and syntax, provided for
compatibility with older Scheme code. It equals the R7RS compatibility surface
(the shared procedures and syntax also reachable through `(scheme base)` and the
libraries above) plus a small set of R5RS-era names. See [base.md](base.md) for
the shared procedures.

The notable additions beyond the R7RS surface are:

- `(exact->inexact z)` - convert a number to an inexact value -> inexact number.
- `(inexact->exact z)` - convert a number to an exact value -> exact number.
- `(null-environment version)` - environment with only the R5RS syntactic
  keywords -> environment specifier.
- `(scheme-report-environment version)` - environment with the R5RS bindings
  -> environment specifier.
