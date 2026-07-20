# (r7rs and-let*)

```scheme
(import (r7rs and-let*))   ; also available as (srfi 2)
```

A short-circuiting form that combines `and` with `let*`, threading bindings and
tests left to right and stopping at the first false result. This is SRFI 2.

- `(and-let* (clause ...) body ...)` - (syntax) evaluate each clause in order,
  stopping and returning `#f` as soon as one yields a false value. A clause may
  be `(var expr)` to bind var to expr, `(expr)` to test expr without binding, or
  `(bound-var)` to test an already-bound variable. If every clause succeeds,
  body is evaluated and its value is returned; with no body the value of the
  last clause is returned.
