# (neetan test 1)

```scheme
(import (neetan test 1))
```

A small test framework layered over `execution-result` from
[`(neetan automation 1)`](automation.md). A script declares one root
`test-suite` containing one or more `test-case` forms. The suite owns the
script's single pass or fail result. Checks abort the current case on failure
and record it, and execution continues with the next case.

## Structure

- `(test-suite name body ...)` -> test-summary alist. (syntax) The single root
  suite. Not nestable and may appear only once. Requires at least one case. On
  completion it publishes `(execution-result 'OK)` if every case passed, or
  `(execution-result 'ERROR summary)` otherwise. The returned alist has keys
  `suite`, `passed`, `test-count`, `passed-count`, `failure-count`, `failures`,
  and `summary`.
- `(test-case name body ...)` -> unspecified. (syntax) One test case. Must be
  inside a suite and may not nest. A failing check or a raised condition is
  recorded as a failure (with `kind` `assertion` or `error`) and the next case
  runs. Each failure alist has `test-case`, `kind`, and `message`.

A misuse of these forms (nesting, no active case, a second root) raises
`neetan/test-state`.

## Checks

Each check is a syntax form that returns the checked value on success and aborts
the case with a `neetan/assertion` error on failure. Failure messages include a
normalized written representation of the complete check form so multiple checks
in one case can be distinguished.

- `(check-true value)` -> value. Fails if `value` is `#f`.
- `(check-false value)` -> value. Fails if `value` is not `#f`.
- `(check-equal expected actual)` -> actual. Fails unless `(equal? expected
  actual)`.
- `(check-near expected actual tolerance)` -> actual. Reals; fails when
  `|expected - actual| > tolerance`. `tolerance` must be non-negative.
- `(check-screen machine expected-path)` /
  `(check-screen machine expected-path options)` -> unspecified. Compares the
  screen against an expected PNG (the optional third argument is the same
  options alist as `screen-matches?`, so it accepts `tolerance`). On failure it
  writes a side-by-side comparison image beneath the artifact root and aborts
  the case.

## Reporting

- `(fail message)` -> aborts the case. Records a `message` string assertion
  failure. Requires an active case.
- `(note message)` -> unspecified. Emits a diagnostic `message` line to the
  script output.
- `(artifact! artifact-path)` -> alist `((path . artifact-path))`. Records an
  existing artifact-root-relative path in the output; grants no extra filesystem
  authority.
