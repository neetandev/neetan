# (r7rs cut)

```scheme
(import (r7rs cut))   ; also available as (srfi 26)
```

Syntax for building specialized procedures by partially applying an operator
without writing an explicit lambda. This is SRFI 26.

- `(cut slot-or-expr ...)` - (syntax) build a procedure from the given
  subforms; each `<>` marks a positional argument slot and a trailing `<...>`
  marks a rest slot, while every other subform is treated as fixed. The
  non-slot subforms are evaluated once each time the resulting procedure is
  called -> a procedure.
- `(cute slot-or-expr ...)` - (syntax) like cut, but the non-slot subforms are
  evaluated once at the time the procedure is constructed rather than on each
  call -> a procedure.
