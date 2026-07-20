# (r7rs receive)

```scheme
(import (r7rs receive))   ; also available as (srfi 8)
```

A convenience binding form for procedures that return multiple values.
This is SRFI 8.

- `(receive formals expression body ...)` - (syntax) evaluate expression,
  bind the multiple values it produces to formals, and evaluate body in that
  scope -> the value of body.
