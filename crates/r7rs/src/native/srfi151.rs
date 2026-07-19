//! Native primitives backing the SRFI 151 (Bitwise Operations) extension.
//!
//! Every procedure here is a pure computation over exact integers: it reads
//! integers and returns an integer, a boolean, or a bit index, with no callback
//! and no heap aggregate. The five aggregate conversions (bits and the
//! list/vector conversions) and the four callback or generator procedures live
//! in the `(srfi 151)` Scheme wrapper because they build heap sequences or call
//! a user procedure, neither of which a native does.
//!
//! SRFI 151 interprets an integer as a semi-infinite two's-complement bitstring.
//! This engine stores exact integers as `i128`, whose two's-complement encoding
//! is exactly that bitstring truncated to 128 bits with correct sign extension,
//! so the logical operators are plain `i128` operators. A result that would fall
//! outside the `i128` exact-integer range raises `ImplementationRestriction`,
//! matching the numeric-tower contract. Bit indices and field bounds at or above
//! bit 127 fold into the sign bit, and field mutations whose field reaches bit
//! 127 are refused for the same reason.

use super::{NativeContext, index};
use crate::{Error, ErrorKind, Value, value::ValueRepr};

/// The number of bits in the `i128` two's-complement window. Positions at or
/// above the top bit are the sign region.
const WIDTH: usize = 128;

/// The error raised when an exact result falls outside the `i128` range.
fn overflow(op: &str) -> Error {
    Error::plain(
        ErrorKind::ImplementationRestriction,
        format!("{op}: result is outside the exact integer range"),
    )
}

/// Reads a required boolean argument.
fn boolean(value: Value) -> Result<bool, Error> {
    match value.decode() {
        ValueRepr::Boolean(value) => Ok(value),
        _ => Err(Error::plain(ErrorKind::TypeError, "expected boolean")),
    }
}

/// A mask with the low `width` bits set. `width` must be at most [`WIDTH`]. The
/// wrapping subtraction keeps `width == 127` (which yields `i128::MAX`) and
/// `width == 128` (which yields `-1`) correct without overflowing.
fn low_mask(width: usize) -> i128 {
    if width >= WIDTH {
        return -1;
    }
    (1i128 << width).wrapping_sub(1)
}

/// Reads bit `idx` of `i`, folding every index at or above the top bit into the
/// sign bit so a semi-infinite index never shifts past the window.
fn get_bit(i: i128, idx: usize) -> bool {
    (i >> idx.min(WIDTH - 1)) & 1 == 1
}

/// Sets bit `idx` of `i` to `flag`, or returns `None` when the change would move
/// a bit in the sign region and escape the `i128` range.
fn set_bit(i: i128, idx: usize, flag: bool) -> Option<i128> {
    if idx < WIDTH - 1 {
        let mask = 1i128 << idx;
        Some(if flag { i | mask } else { i & !mask })
    } else if flag == (i < 0) {
        // The bit already equals the sign, so setting it changes nothing.
        Some(i)
    } else {
        None
    }
}

/// Reads the field `[start, end)` of `i` shifted down to bit 0. Returns `None`
/// when the extracted value would exceed the `i128` range, which happens only
/// for a wide field of a negative integer.
fn read_field(i: i128, start: usize, end: usize) -> Option<i128> {
    if end <= start {
        return Some(0);
    }
    let width = end - start;
    let shifted = if start >= WIDTH {
        if i < 0 { -1 } else { 0 }
    } else {
        i >> start
    };
    if width >= WIDTH {
        // The mask covers the whole window. A negative shifted value would carry
        // ones into the sign region and above, which cannot be represented.
        if shifted < 0 { None } else { Some(shifted) }
    } else {
        Some(shifted & low_mask(width))
    }
}

/// Reports whether any bit of the field `[start, end)` is set in `i`.
fn field_any(i: i128, start: usize, end: usize) -> bool {
    if end <= start {
        return false;
    }
    let width = end - start;
    let shifted = if start >= WIDTH {
        if i < 0 { -1 } else { 0 }
    } else {
        i >> start
    };
    if width >= WIDTH {
        shifted != 0
    } else {
        shifted & low_mask(width) != 0
    }
}

/// Reports whether every bit of the field `[start, end)` is set in `i`.
fn field_every(i: i128, start: usize, end: usize) -> bool {
    if end <= start {
        return true;
    }
    let width = end - start;
    let shifted = if start >= WIDTH {
        if i < 0 { -1 } else { 0 }
    } else {
        i >> start
    };
    if width >= WIDTH {
        shifted == -1
    } else {
        let mask = low_mask(width);
        shifted & mask == mask
    }
}

/// Reads a field's `[start, end)` bounds as non-negative indices, requiring
/// `start <= end` and a field that stays below the sign bit. A field whose
/// exclusive end is 127 reaches only bit 126 and remains representable.
fn mut_bounds(cx: &NativeContext<'_>, start: Value, end: Value) -> Result<(usize, usize), Error> {
    let start = index(cx, start)?;
    let end = index(cx, end)?;
    if end < start {
        return Err(Error::plain(
            ErrorKind::RangeError,
            "bit field end must be at least start",
        ));
    }
    if end > WIDTH - 1 {
        return Err(Error::plain(
            ErrorKind::ImplementationRestriction,
            "bit field reaches the sign bit and cannot be represented",
        ));
    }
    Ok((start, end))
}

/// The mask selecting the field `[start, end)`. The caller guarantees
/// `end <= 127` through [`mut_bounds`], so the mask never touches the sign bit
/// and stays a non-negative `i128`.
fn field_mask(start: usize, end: usize) -> i128 {
    low_mask(end - start) << start
}

/// `(bitwise-not i)`.
pub(crate) fn bitwise_not(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    let i = cx.to_i128(args[0])?;
    cx.integer(!i)
}

/// `(bitwise-and i ...)`. Associative, identity `-1`.
pub(crate) fn bitwise_and(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    let mut acc: i128 = -1;
    for &arg in args {
        acc &= cx.to_i128(arg)?;
    }
    cx.integer(acc)
}

/// `(bitwise-ior i ...)`. Associative, identity `0`.
pub(crate) fn bitwise_ior(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    let mut acc: i128 = 0;
    for &arg in args {
        acc |= cx.to_i128(arg)?;
    }
    cx.integer(acc)
}

/// `(bitwise-xor i ...)`. Associative, identity `0`.
pub(crate) fn bitwise_xor(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    let mut acc: i128 = 0;
    for &arg in args {
        acc ^= cx.to_i128(arg)?;
    }
    cx.integer(acc)
}

/// `(bitwise-eqv i ...)`. Associative, identity `-1`. It is the complement of
/// the running `bitwise-xor`.
pub(crate) fn bitwise_eqv(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    let mut acc: i128 = -1;
    for &arg in args {
        acc = !(acc ^ cx.to_i128(arg)?);
    }
    cx.integer(acc)
}

/// `(bitwise-nand i j)`.
pub(crate) fn bitwise_nand(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    let i = cx.to_i128(args[0])?;
    let j = cx.to_i128(args[1])?;
    cx.integer(!(i & j))
}

/// `(bitwise-nor i j)`.
pub(crate) fn bitwise_nor(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    let i = cx.to_i128(args[0])?;
    let j = cx.to_i128(args[1])?;
    cx.integer(!(i | j))
}

/// `(bitwise-andc1 i j)`. And of the complement of the first argument with the
/// second.
pub(crate) fn bitwise_andc1(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    let i = cx.to_i128(args[0])?;
    let j = cx.to_i128(args[1])?;
    cx.integer(!i & j)
}

/// `(bitwise-andc2 i j)`. And of the first argument with the complement of the
/// second.
pub(crate) fn bitwise_andc2(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    let i = cx.to_i128(args[0])?;
    let j = cx.to_i128(args[1])?;
    cx.integer(i & !j)
}

/// `(bitwise-orc1 i j)`. Or of the complement of the first argument with the
/// second.
pub(crate) fn bitwise_orc1(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    let i = cx.to_i128(args[0])?;
    let j = cx.to_i128(args[1])?;
    cx.integer(!i | j)
}

/// `(bitwise-orc2 i j)`. Or of the first argument with the complement of the
/// second.
pub(crate) fn bitwise_orc2(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    let i = cx.to_i128(args[0])?;
    let j = cx.to_i128(args[1])?;
    cx.integer(i | !j)
}

/// `(arithmetic-shift i count)`. Left shift for a positive count, arithmetic
/// (sign-preserving) right shift for a negative count. A left shift that would
/// drop a value bit off the top raises `ImplementationRestriction`.
pub(crate) fn arithmetic_shift(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    let i = cx.to_i128(args[0])?;
    let count = cx.to_i128(args[1])?;
    let result = shift(i, count).ok_or_else(|| overflow("arithmetic-shift"))?;
    cx.integer(result)
}

/// Shifts `i` by `count`, returning `None` when a left shift overflows the
/// `i128` range. Shifts of magnitude at least the window size saturate a right
/// shift to `0` or `-1` by sign.
fn shift(i: i128, count: i128) -> Option<i128> {
    use std::cmp::Ordering::{Equal, Greater, Less};
    match count.cmp(&0) {
        Equal => Some(i),
        Greater => {
            if i == 0 {
                return Some(0);
            }
            if count >= WIDTH as i128 {
                return None;
            }
            let n = count as u32;
            let shifted = i.checked_shl(n)?;
            // The value overflowed if shifting back does not reproduce the input.
            (shifted >> n == i).then_some(shifted)
        }
        Less => {
            let n = count.unsigned_abs();
            if n >= WIDTH as u128 {
                Some(if i < 0 { -1 } else { 0 })
            } else {
                Some(i >> n as u32)
            }
        }
    }
}

/// `(bit-count i)`. The population count of ones for a non-negative `i`, or of
/// zeros for a negative `i`. Always non-negative.
pub(crate) fn bit_count(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    let i = cx.to_i128(args[0])?;
    let count = if i >= 0 {
        i.count_ones()
    } else {
        i.count_zeros()
    };
    cx.integer(i128::from(count))
}

/// `(integer-length i)`. The number of bits needed to represent `i` without its
/// sign. Always non-negative.
pub(crate) fn integer_length(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    let i = cx.to_i128(args[0])?;
    let magnitude = if i >= 0 { i } else { !i };
    let length = WIDTH as u32 - magnitude.leading_zeros();
    cx.integer(i128::from(length))
}

/// `(bitwise-if mask i j)`. Takes each bit from `i` where `mask` is set and from
/// `j` where it is clear.
pub(crate) fn bitwise_if(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    let mask = cx.to_i128(args[0])?;
    let i = cx.to_i128(args[1])?;
    let j = cx.to_i128(args[2])?;
    cx.integer((mask & i) | (!mask & j))
}

/// `(bit-set? index i)`.
pub(crate) fn bit_set_p(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    let idx = index(cx, args[0])?;
    let i = cx.to_i128(args[1])?;
    Ok(Value::boolean(get_bit(i, idx)))
}

/// `(copy-bit index i boolean)`. `i` with bit `index` set to `boolean`.
pub(crate) fn copy_bit(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    let idx = index(cx, args[0])?;
    let i = cx.to_i128(args[1])?;
    let flag = boolean(args[2])?;
    let result = set_bit(i, idx, flag).ok_or_else(|| overflow("copy-bit"))?;
    cx.integer(result)
}

/// `(bit-swap index1 index2 i)`. `i` with the two named bits exchanged.
pub(crate) fn bit_swap(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    let idx1 = index(cx, args[0])?;
    let idx2 = index(cx, args[1])?;
    let i = cx.to_i128(args[2])?;
    let bit1 = get_bit(i, idx1);
    let bit2 = get_bit(i, idx2);
    if bit1 == bit2 {
        return cx.integer(i);
    }
    let swapped = set_bit(i, idx1, bit2)
        .and_then(|i| set_bit(i, idx2, bit1))
        .ok_or_else(|| overflow("bit-swap"))?;
    cx.integer(swapped)
}

/// `(any-bit-set? test-bits i)`.
pub(crate) fn any_bit_set_p(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    let test = cx.to_i128(args[0])?;
    let i = cx.to_i128(args[1])?;
    Ok(Value::boolean(test & i != 0))
}

/// `(every-bit-set? test-bits i)`.
pub(crate) fn every_bit_set_p(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    let test = cx.to_i128(args[0])?;
    let i = cx.to_i128(args[1])?;
    Ok(Value::boolean(test & i == test))
}

/// `(first-set-bit i)`. The index of the lowest set bit, or `-1` when `i` is
/// zero.
pub(crate) fn first_set_bit(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    let i = cx.to_i128(args[0])?;
    let result = if i == 0 {
        -1
    } else {
        i128::from(i.trailing_zeros())
    };
    cx.integer(result)
}

/// `(bit-field i start end)`.
pub(crate) fn bit_field(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    let i = cx.to_i128(args[0])?;
    let start = index(cx, args[1])?;
    let end = index(cx, args[2])?;
    let result = read_field(i, start, end).ok_or_else(|| overflow("bit-field"))?;
    cx.integer(result)
}

/// `(bit-field-any? i start end)`.
pub(crate) fn bit_field_any_p(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    let i = cx.to_i128(args[0])?;
    let start = index(cx, args[1])?;
    let end = index(cx, args[2])?;
    Ok(Value::boolean(field_any(i, start, end)))
}

/// `(bit-field-every? i start end)`.
pub(crate) fn bit_field_every_p(
    cx: &mut NativeContext<'_>,
    args: &[Value],
) -> Result<Value, Error> {
    let i = cx.to_i128(args[0])?;
    let start = index(cx, args[1])?;
    let end = index(cx, args[2])?;
    Ok(Value::boolean(field_every(i, start, end)))
}

/// `(bit-field-clear i start end)`. `i` with the field cleared to zeros.
pub(crate) fn bit_field_clear(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    let i = cx.to_i128(args[0])?;
    let (start, end) = mut_bounds(cx, args[1], args[2])?;
    cx.integer(i & !field_mask(start, end))
}

/// `(bit-field-set i start end)`. `i` with the field set to ones.
pub(crate) fn bit_field_set(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    let i = cx.to_i128(args[0])?;
    let (start, end) = mut_bounds(cx, args[1], args[2])?;
    cx.integer(i | field_mask(start, end))
}

/// `(bit-field-replace dest source start end)`. `dest` with the field replaced
/// by the least-significant `end - start` bits of `source`.
pub(crate) fn bit_field_replace(
    cx: &mut NativeContext<'_>,
    args: &[Value],
) -> Result<Value, Error> {
    let dest = cx.to_i128(args[0])?;
    let source = cx.to_i128(args[1])?;
    let (start, end) = mut_bounds(cx, args[2], args[3])?;
    let mask = field_mask(start, end);
    let field = (source & low_mask(end - start)) << start;
    cx.integer((dest & !mask) | field)
}

/// `(bit-field-replace-same dest source start end)`. `dest` with the field
/// replaced by the same field of `source`.
pub(crate) fn bit_field_replace_same(
    cx: &mut NativeContext<'_>,
    args: &[Value],
) -> Result<Value, Error> {
    let dest = cx.to_i128(args[0])?;
    let source = cx.to_i128(args[1])?;
    let (start, end) = mut_bounds(cx, args[2], args[3])?;
    let mask = field_mask(start, end);
    cx.integer((dest & !mask) | (source & mask))
}

/// `(bit-field-rotate i count start end)`. `i` with the field cyclically rotated
/// by `count` bits toward the high order end.
pub(crate) fn bit_field_rotate(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    let i = cx.to_i128(args[0])?;
    let count = cx.to_i128(args[1])?;
    let (start, end) = mut_bounds(cx, args[2], args[3])?;
    let width = end - start;
    let mask = field_mask(start, end);
    let field = (i >> start) & low_mask(width);
    let rotated = rotate_field(field, count, width);
    cx.integer((i & !mask) | (rotated << start))
}

/// Rotates the low `width` bits of `field` toward the high order end by `count`,
/// wrapping the top bits back to the bottom. `field` holds only its low `width`
/// bits, so the shifts stay within the window.
fn rotate_field(field: i128, count: i128, width: usize) -> i128 {
    if width <= 1 {
        return field;
    }
    let steps = count.rem_euclid(width as i128) as u32;
    if steps == 0 {
        return field;
    }
    let bits = field as u128;
    let mask = low_mask(width) as u128;
    let rotated = (bits << steps | bits >> (width as u32 - steps)) & mask;
    rotated as i128
}

/// `(bit-field-reverse i start end)`. `i` with the order of the field's bits
/// reversed.
pub(crate) fn bit_field_reverse(
    cx: &mut NativeContext<'_>,
    args: &[Value],
) -> Result<Value, Error> {
    let i = cx.to_i128(args[0])?;
    let (start, end) = mut_bounds(cx, args[1], args[2])?;
    let width = end - start;
    let mask = field_mask(start, end);
    let field = ((i >> start) & low_mask(width)) as u128;
    let mut reversed: u128 = 0;
    for bit in 0..width {
        if field >> bit & 1 == 1 {
            reversed |= 1u128 << (width - 1 - bit);
        }
    }
    cx.integer((i & !mask) | ((reversed as i128) << start))
}

#[cfg(test)]
mod tests {
    use crate::{Engine, EngineConfig, ErrorKind, Extension, Value};

    fn engine() -> Engine {
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        engine.install_extension(Extension::Srfi151).unwrap();
        engine
    }

    fn run(engine: &mut Engine, source: &str) -> Value {
        let module = engine.compile("test.scm", source).unwrap();
        engine.eval(&module).unwrap().into_one().unwrap().value()
    }

    fn error_kind(engine: &mut Engine, source: &str) -> ErrorKind {
        let module = engine.compile("test.scm", source).unwrap();
        engine.eval(&module).unwrap_err().kind()
    }

    #[test]
    fn logical_operators_follow_the_spec_examples() {
        let mut engine = engine();
        assert_eq!(
            run(
                &mut engine,
                r#"
                (import (srfi 151) (scheme base))
                (and (= (bitwise-not 10) -11)
                     (= (bitwise-not -37) 36)
                     (= (bitwise-ior 3 10) 11)
                     (= (bitwise-and 11 26) 10)
                     (= (bitwise-xor 3 10) 9)
                     (= (bitwise-eqv 37 12) -42)
                     (= (bitwise-and) -1)
                     (= (bitwise-ior) 0)
                     (= (bitwise-nand 11 26) -11)
                     (= (bitwise-nor 11 26) -28)
                     (= (bitwise-andc1 11 26) 16)
                     (= (bitwise-andc2 11 26) 1)
                     (= (bitwise-orc1 11 26) -2)
                     (= (bitwise-orc2 11 26) -17))
                "#,
            ),
            Value::boolean(true)
        );
    }

    #[test]
    fn integer_operations_follow_the_spec_examples() {
        let mut engine = engine();
        assert_eq!(
            run(
                &mut engine,
                r#"
                (import (srfi 151) (scheme base))
                (and (= (arithmetic-shift 8 2) 32)
                     (= (arithmetic-shift 8 -1) 4)
                     (= (arithmetic-shift -100000000000000000000000000000000 -100) -79)
                     (= (bit-count 0) 0)
                     (= (bit-count -1) 0)
                     (= (bit-count 13) 3)
                     (= (bit-count -13) 2)
                     (= (bit-count (expt 2 100)) 1)
                     (= (bit-count (- (expt 2 100))) 100)
                     (= (integer-length 0) 0)
                     (= (integer-length -1) 0)
                     (= (integer-length 7) 3)
                     (= (integer-length -8) 3)
                     (= (bitwise-if 3 1 8) 9)
                     (= (bitwise-if #b00111100 #b11110000 #b00001111) #b00110011))
                "#,
            ),
            Value::boolean(true)
        );
    }

    #[test]
    fn single_bit_operations_follow_the_spec_examples() {
        let mut engine = engine();
        assert_eq!(
            run(
                &mut engine,
                r#"
                (import (srfi 151) (scheme base))
                (and (eq? (bit-set? 1 1) #f)
                     (eq? (bit-set? 0 1) #t)
                     (eq? (bit-set? 3 10) #t)
                     (eq? (bit-set? 1000000 -1) #t)
                     (= (copy-bit 0 0 #t) 1)
                     (= (copy-bit 2 0 #t) 4)
                     (= (copy-bit 2 #b1111 #f) #b1011)
                     (= (bit-swap 0 2 4) 1)
                     (eq? (any-bit-set? 3 6) #t)
                     (eq? (any-bit-set? 3 12) #f)
                     (eq? (every-bit-set? 4 6) #t)
                     (eq? (every-bit-set? 7 6) #f)
                     (= (first-set-bit 1) 0)
                     (= (first-set-bit 0) -1)
                     (= (first-set-bit 40) 3)
                     (= (first-set-bit -28) 2))
                "#,
            ),
            Value::boolean(true)
        );
    }

    #[test]
    fn bit_field_operations_follow_the_spec_examples() {
        let mut engine = engine();
        assert_eq!(
            run(
                &mut engine,
                r#"
                (import (srfi 151) (scheme base))
                (and (= (bit-field #b1101101010 0 4) #b1010)
                     (= (bit-field #b1101101010 3 9) #b101101)
                     (= (bit-field 6 2 999) 1)
                     (eq? (bit-field-any? #b1001001 1 6) #t)
                     (eq? (bit-field-any? #b1000001 1 6) #f)
                     (eq? (bit-field-every? #b1011110 1 5) #t)
                     (eq? (bit-field-every? #b1011010 1 5) #f)
                     (= (bit-field-clear #b101010 1 4) #b100000)
                     (= (bit-field-set #b101010 1 4) #b101110)
                     (= (bit-field-replace #b101010 #b010 1 4) #b100100)
                     (= (bit-field-replace-same #b1111 #b0000 1 3) #b1001)
                     (= (bit-field-rotate #b110 1 2 4) #b1010)
                     (= (bit-field-rotate #b0111 -1 1 4) #b1011)
                     (= (bit-field-reverse 6 1 4) 12)
                     (= (bit-field-reverse 1 0 32) #x80000000))
                "#,
            ),
            Value::boolean(true)
        );
    }

    #[test]
    fn a_non_integer_argument_is_a_type_error() {
        let mut engine = engine();
        assert_eq!(
            error_kind(&mut engine, r#"(import (srfi 151)) (bitwise-and 3 "x")"#),
            ErrorKind::TypeError
        );
    }

    #[test]
    fn a_negative_bit_index_is_a_range_error() {
        let mut engine = engine();
        assert_eq!(
            error_kind(&mut engine, r#"(import (srfi 151)) (bit-set? -1 5)"#),
            ErrorKind::RangeError
        );
    }

    #[test]
    fn an_overflowing_left_shift_is_an_implementation_restriction() {
        let mut engine = engine();
        assert_eq!(
            error_kind(
                &mut engine,
                r#"(import (srfi 151)) (arithmetic-shift 1 200)"#
            ),
            ErrorKind::ImplementationRestriction
        );
    }
}
