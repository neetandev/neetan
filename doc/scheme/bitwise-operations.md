# (r7rs bitwise-operations)

```scheme
(import (r7rs bitwise-operations))   ; also available as (srfi 151)
```

The SRFI 151 bitwise operations library: boolean bit combinators, shifts, bit
counting, single-bit and bit-field manipulation, and conversions between
integers and bit sequences. This is SRFI 151.

Integers are treated as two's-complement bit strings of unbounded width, so
negative integers act as an infinite run of leading one bits. Bit 0 is the least
significant bit.

## Bitwise boolean operations

- `(bitwise-not i)` - bitwise complement -> integer.
- `(bitwise-and i ...)` - bitwise AND of the arguments -> integer.
- `(bitwise-ior i ...)` - bitwise inclusive OR of the arguments -> integer.
- `(bitwise-xor i ...)` - bitwise exclusive OR of the arguments -> integer.
- `(bitwise-eqv i ...)` - bitwise equivalence (complement of XOR) -> integer.
- `(bitwise-nand i j)` - bitwise NAND of the two arguments -> integer.
- `(bitwise-nor i j)` - bitwise NOR of the two arguments -> integer.
- `(bitwise-andc1 i j)` - AND of `(bitwise-not i)` and `j` -> integer.
- `(bitwise-andc2 i j)` - AND of `i` and `(bitwise-not j)` -> integer.
- `(bitwise-orc1 i j)` - OR of `(bitwise-not i)` and `j` -> integer.
- `(bitwise-orc2 i j)` - OR of `i` and `(bitwise-not j)` -> integer.

## Integer operations

- `(arithmetic-shift i count)` - shift left by `count`, or right if negative,
  with sign extension -> integer.
- `(bit-count i)` - population count of set bits (of clear bits, for negatives)
  -> integer.
- `(integer-length i)` - number of bits needed to represent `i` -> integer.
- `(bitwise-if mask i j)` - take masked bits from `i` and the rest from `j`
  -> integer.

## Single-bit operations

- `(bit-set? index i)` - `#t` if bit `index` of `i` is set.
- `(copy-bit index i boolean)` - `i` with bit `index` set to `boolean`
  -> integer.
- `(bit-swap index1 index2 i)` - `i` with bits `index1` and `index2` exchanged
  -> integer.
- `(any-bit-set? test-bits i)` - `#t` if any bit set in `test-bits` is set in `i`.
- `(every-bit-set? test-bits i)` - `#t` if every bit set in `test-bits` is set in
  `i`.
- `(first-set-bit i)` - index of the least significant set bit, or `-1` if none.

## Bit fields

- `(bit-field i start end)` - the bits in `[start, end)` shifted down to bit 0
  -> integer.
- `(bit-field-any? i start end)` - `#t` if any bit in `[start, end)` is set.
- `(bit-field-every? i start end)` - `#t` if every bit in `[start, end)` is set.
- `(bit-field-clear i start end)` - `i` with the bits in `[start, end)` cleared
  -> integer.
- `(bit-field-set i start end)` - `i` with the bits in `[start, end)` set
  -> integer.
- `(bit-field-replace dest source start end)` - replace `[start, end)` of `dest`
  with the low bits of `source` -> integer.
- `(bit-field-replace-same dest source start end)` - copy `[start, end)` from
  `source` into `dest` in place -> integer.
- `(bit-field-rotate i count start end)` - rotate the field `[start, end)` by
  `count` bits -> integer.
- `(bit-field-reverse i start end)` - reverse the bit order within `[start, end)`
  -> integer.

## Bits conversion

- `(bits->list i)` / `(bits->list i len)` - list of booleans for the bits, least
  significant first -> list.
- `(bits->vector i)` / `(bits->vector i len)` - vector of booleans for the bits,
  least significant first -> vector.
- `(list->bits list)` - integer from a list of booleans, least significant first
  -> integer.
- `(vector->bits vec)` - integer from a vector of booleans, least significant
  first -> integer.
- `(bits bool ...)` - integer from boolean arguments, least significant first
  -> integer.

## Fold, for-each, and generators

- `(bitwise-fold proc seed i)` - fold `proc` over the bits of `i` from bit 0
  -> value.
- `(bitwise-for-each proc i)` - apply `proc` to each bit of `i` from bit 0 for
  effect.
- `(bitwise-unfold stop? mapper successor seed)` - build an integer bit by bit
  from a seed -> integer.
- `(make-bitwise-generator i)` - generator yielding the successive bits of `i` as
  booleans -> procedure.
