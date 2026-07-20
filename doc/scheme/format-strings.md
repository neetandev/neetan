# (r7rs intermediate-format-strings)

```scheme
(import (r7rs intermediate-format-strings))   ; also available as (srfi 48)
```

Intermediate-level formatted output driven by a format string with tilde
directives. This is SRFI 48. It is also reachable as `(srfi 48)`, and a
`(srfi 28)` compatibility library exporting the same `format` procedure exists.

## Procedure

- `(format destination format-string arg ...)` - build or emit a string from
  format-string and the arguments; also callable as `(format format-string
  arg ...)`. A destination of `#f` returns the formatted string, `#t` writes it
  to the current output port, and a port writes it to that port.

## Directives

- `~a` - display the next argument (as by `display`).
- `~s` - write the next argument (as by `write`).
- `~d` - the next argument as a decimal integer.
- `~b` - the next argument as a binary integer.
- `~o` - the next argument as an octal integer.
- `~x` - the next argument as a hexadecimal integer.
- `~w` - write the next argument with datum labels for shared structure.
- `~y` - pretty-print the next argument.
- `~?` / `~k` - indirection: take a format string and an argument list from the
  next two arguments and process them.
- `~~` - a literal tilde character.
- `~%` - output a newline.
- `~&` - output a newline only if not already at the start of a line.
- `~_` - output a single space.
- `~t` - output a tab.
- `~h` - output the built-in directive help text.
- `~c` - output the next argument as a character.
- `~f` - the next argument as a fixed-point real.

Numeric parameters set the width and decimal places of the fixed-point `~f`
directive only: `~8f` pads the value to a field width of 8, and `~8,2f` uses a
width of 8 with 2 decimal places. Field widths on other directives and custom
padding characters are not supported.
