# (r7rs ascii)

```scheme
(import (r7rs ascii))   ; also available as (srfi 175)
```

An ASCII character library operating on codepoints, characters, bytevectors, and
strings restricted to the ASCII range. This is SRFI 175.

## Predicates

- `(ascii-codepoint? object)` - `#t` if object is an integer in the ASCII range
  0 to 127.
- `(ascii-bytevector? bytevector)` - `#t` if every byte is an ASCII codepoint.
- `(ascii-char? char)` - `#t` if char is an ASCII character.
- `(ascii-string? string)` - `#t` if every character of string is ASCII.
- `(ascii-control? x)` - `#t` if x is an ASCII control character (including
  delete).
- `(ascii-non-control? x)` - `#t` if x is a printable non-control ASCII
  character.
- `(ascii-whitespace? x)` - `#t` if x is an ASCII whitespace character.
- `(ascii-space-or-tab? x)` - `#t` if x is a space or tab.
- `(ascii-other-graphic? x)` - `#t` if x is a graphic character that is neither
  alphanumeric nor whitespace.
- `(ascii-upper-case? x)` - `#t` if x is an uppercase ASCII letter.
- `(ascii-lower-case? x)` - `#t` if x is a lowercase ASCII letter.
- `(ascii-alphabetic? x)` - `#t` if x is an ASCII letter.
- `(ascii-alphanumeric? x)` - `#t` if x is an ASCII letter or digit.
- `(ascii-numeric? x)` - `#t` if x is an ASCII decimal digit.

## Case and value conversions

- `(ascii-digit-value x limit)` - numeric value of digit x below limit, else
  `#f`.
- `(ascii-upper-case-value x offset limit)` - index of uppercase letter x, or
  `#f`, with the given offset and limit.
- `(ascii-lower-case-value x offset limit)` - index of lowercase letter x, or
  `#f`, with the given offset and limit.
- `(ascii-nth-digit n)` - the digit character for value n -> char.
- `(ascii-nth-upper-case n)` - the nth uppercase letter (wrapping) -> char.
- `(ascii-nth-lower-case n)` - the nth lowercase letter (wrapping) -> char.
- `(ascii-upcase x)` - uppercase equivalent of x, ASCII only.
- `(ascii-downcase x)` - lowercase equivalent of x, ASCII only.
- `(ascii-control->graphic x)` - the graphic character matching control x, else
  `#f`.
- `(ascii-graphic->control x)` - the control character matching graphic x, else
  `#f`.
- `(ascii-mirror-bracket x)` - the mirror-image bracket of x, else `#f`.

## Comparisons

- `(ascii-ci=? a b)` - case-insensitive ASCII equality.
- `(ascii-ci<? a b)` - case-insensitive ASCII less-than.
- `(ascii-ci>? a b)` - case-insensitive ASCII greater-than.
- `(ascii-ci<=? a b)` - case-insensitive ASCII less-or-equal.
- `(ascii-ci>=? a b)` - case-insensitive ASCII greater-or-equal.
- `(ascii-string-ci=? a b)` - case-insensitive ASCII string equality.
- `(ascii-string-ci<? a b)` - case-insensitive ASCII string less-than.
- `(ascii-string-ci>? a b)` - case-insensitive ASCII string greater-than.
- `(ascii-string-ci<=? a b)` - case-insensitive ASCII string less-or-equal.
- `(ascii-string-ci>=? a b)` - case-insensitive ASCII string greater-or-equal.
