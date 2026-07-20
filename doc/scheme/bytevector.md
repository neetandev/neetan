# (r7rs bytevector)

```scheme
(import (r7rs bytevector))   ; the R6RS bytevector library, R7RS-large name (scheme bytevector)
```

The R6RS bytevector operations exposed under their R7RS-large name
`(scheme bytevector)`: general access, integer and float encode/decode at
various widths and endianness, and string transcoders.

Two deviations from R6RS apply. The R6RS-argument-order `bytevector-copy!` is
not provided here. The R7RS-small ordering from `(scheme base)` is used instead.
`bytevector-append` is not part of this library.

## Syntax

- `(endianness symbol)` - (syntax) an endianness object; accepts only `little`
  or `big`.

## General operations

- `(bytevector? object)` - `#t` if object is a bytevector.
- `(make-bytevector length [fill])` - a new bytevector of length bytes ->
  bytevector.
- `(bytevector-length bytevector)` - length in bytes -> integer.
- `(bytevector-u8-ref bytevector index)` - unsigned byte at index -> integer.
- `(bytevector-u8-set! bytevector index value)` - store an unsigned byte at
  index.
- `(bytevector-copy bytevector [start end])` - a copy of a range -> bytevector.
- `(bytevector-copy! to at from [start end])` - copy a range of from into to at
  index at (R7RS-small argument order).
- `(bytevector=? a b)` - `#t` if the two bytevectors are byte-for-byte equal.
- `(bytevector-fill! bytevector value)` - set every byte to value.
- `(bytevector-s8-ref bytevector index)` - signed byte at index -> integer.
- `(bytevector-s8-set! bytevector index value)` - store a signed byte at index.
- `(bytevector->u8-list bytevector)` - list of unsigned bytes -> list.
- `(u8-list->bytevector list)` - bytevector from a list of bytes -> bytevector.
- `(string->utf8 string [start end])` - UTF-8 encoding of string -> bytevector.
- `(utf8->string bytevector [start end])` - decode UTF-8 bytes -> string.
- `(native-endianness)` - the platform's native endianness object.

## Arbitrary-size integers

- `(bytevector-uint-ref bytevector index endianness size)` - unsigned integer of
  size bytes -> integer.
- `(bytevector-sint-ref bytevector index endianness size)` - signed integer of
  size bytes -> integer.
- `(bytevector-uint-set! bytevector index value endianness size)` - store an
  unsigned integer of size bytes.
- `(bytevector-sint-set! bytevector index value endianness size)` - store a
  signed integer of size bytes.
- `(bytevector->uint-list bytevector endianness size)` - list of unsigned
  integers -> list.
- `(bytevector->sint-list bytevector endianness size)` - list of signed integers
  -> list.
- `(uint-list->bytevector list endianness size)` - bytevector from unsigned
  integers -> bytevector.
- `(sint-list->bytevector list endianness size)` - bytevector from signed
  integers -> bytevector.

## Fixed-size integers (16/32/64, native and endian)

- `(bytevector-u16-ref bytevector index endianness)` - unsigned 16-bit at index
  -> integer.
- `(bytevector-s16-ref bytevector index endianness)` - signed 16-bit at index ->
  integer.
- `(bytevector-u16-native-ref bytevector index)` - unsigned 16-bit, native
  endianness -> integer.
- `(bytevector-s16-native-ref bytevector index)` - signed 16-bit, native
  endianness -> integer.
- `(bytevector-u16-set! bytevector index value endianness)` - store unsigned
  16-bit.
- `(bytevector-s16-set! bytevector index value endianness)` - store signed
  16-bit.
- `(bytevector-u16-native-set! bytevector index value)` - store unsigned 16-bit,
  native endianness.
- `(bytevector-s16-native-set! bytevector index value)` - store signed 16-bit,
  native endianness.
- `(bytevector-u32-ref bytevector index endianness)` - unsigned 32-bit at index
  -> integer.
- `(bytevector-s32-ref bytevector index endianness)` - signed 32-bit at index ->
  integer.
- `(bytevector-u32-native-ref bytevector index)` - unsigned 32-bit, native
  endianness -> integer.
- `(bytevector-s32-native-ref bytevector index)` - signed 32-bit, native
  endianness -> integer.
- `(bytevector-u32-set! bytevector index value endianness)` - store unsigned
  32-bit.
- `(bytevector-s32-set! bytevector index value endianness)` - store signed
  32-bit.
- `(bytevector-u32-native-set! bytevector index value)` - store unsigned 32-bit,
  native endianness.
- `(bytevector-s32-native-set! bytevector index value)` - store signed 32-bit,
  native endianness.
- `(bytevector-u64-ref bytevector index endianness)` - unsigned 64-bit at index
  -> integer.
- `(bytevector-s64-ref bytevector index endianness)` - signed 64-bit at index ->
  integer.
- `(bytevector-u64-native-ref bytevector index)` - unsigned 64-bit, native
  endianness -> integer.
- `(bytevector-s64-native-ref bytevector index)` - signed 64-bit, native
  endianness -> integer.
- `(bytevector-u64-set! bytevector index value endianness)` - store unsigned
  64-bit.
- `(bytevector-s64-set! bytevector index value endianness)` - store signed
  64-bit.
- `(bytevector-u64-native-set! bytevector index value)` - store unsigned 64-bit,
  native endianness.
- `(bytevector-s64-native-set! bytevector index value)` - store signed 64-bit,
  native endianness.

## IEEE-754 floats

- `(bytevector-ieee-single-ref bytevector index endianness)` - 32-bit float at
  index -> real.
- `(bytevector-ieee-single-native-ref bytevector index)` - 32-bit float, native
  endianness -> real.
- `(bytevector-ieee-single-set! bytevector index value endianness)` - store a
  32-bit float.
- `(bytevector-ieee-single-native-set! bytevector index value)` - store a 32-bit
  float, native endianness.
- `(bytevector-ieee-double-ref bytevector index endianness)` - 64-bit float at
  index -> real.
- `(bytevector-ieee-double-native-ref bytevector index)` - 64-bit float, native
  endianness -> real.
- `(bytevector-ieee-double-set! bytevector index value endianness)` - store a
  64-bit float.
- `(bytevector-ieee-double-native-set! bytevector index value)` - store a 64-bit
  float, native endianness.

## String transcoders

- `(string->utf16 string [endianness])` - UTF-16 encoding of string ->
  bytevector.
- `(string->utf32 string [endianness])` - UTF-32 encoding of string ->
  bytevector.
- `(utf16->string bytevector endianness [endianness-mandatory])` - decode UTF-16
  bytes -> string.
- `(utf32->string bytevector endianness [endianness-mandatory])` - decode UTF-32
  bytes -> string.
