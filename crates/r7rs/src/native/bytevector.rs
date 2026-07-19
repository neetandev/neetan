//! Native primitives backing the R6RS bytevectors extension `(scheme bytevector)`.
//!
//! Everything representation-dependent lives here: the fixed-size and
//! arbitrary-size integer accessors, the IEEE-754 accessors, the list
//! conversions, and the UTF-16/UTF-32 transcoders. Only the `endianness` macro
//! is defined in the Scheme wrapper (see `crate::embed::extensions`), because
//! it must reject an unknown endianness symbol at expansion time.
//!
//! Exact integers in this engine are `i128`. `bytevector-uint-ref` and
//! `bytevector-sint-ref` therefore raise `ImplementationRestriction` when the
//! decoded value falls outside the `i128` window, matching the numeric-tower
//! contract. Redundant leading bytes (zero, or sign-matching `#xFF`) are fine
//! at any size.
//!
//! `Value::float` canonicalizes NaN, so the IEEE accessors do not preserve NaN
//! payload bits. Writing any NaN stores the canonical pattern and reading a
//! non-canonical NaN pattern yields the canonical NaN.
//!
//! List traversals use a tortoise and hare and raise on a circular list,
//! because a native Rust loop never reaches a fuel safe point.
//!
//! Multi-byte writes go through `Heap::bytevector_slice_mut`, which performs
//! the type, liveness, and immutability checks once. The returned slice
//! borrows the heap mutably, so the borrow checker guarantees no allocation
//! (and thus no arena compaction) can happen while it is held. Every native
//! converts and validates its arguments before taking the slice.

use super::{
    NativeContext, byte, bytevector_argument, fill_byte, immutable_error, index,
    number::{exact_integer, real_argument},
    range_or_type, sequence_mutation_error, string_argument, type_error,
};
use crate::{Error, ErrorKind, Value, value::ValueRepr};

/// A decoded endianness argument.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Endianness {
    Little,
    Big,
}

/// The endianness `native-endianness` reports and the `-native-` accessors use.
const NATIVE_ENDIANNESS: Endianness = if cfg!(target_endian = "big") {
    Endianness::Big
} else {
    Endianness::Little
};

/// A boolean argument is false only when it is `#f`, following Scheme's
/// general truth rule.
fn truthy(value: Value) -> bool {
    !matches!(value.decode(), ValueRepr::Boolean(false))
}

/// Decodes an endianness symbol argument. Only the symbols `little` and `big`
/// are supported.
fn endianness_argument(cx: &NativeContext<'_>, value: Value) -> Result<Endianness, Error> {
    match cx.heap.symbol_slice(value) {
        Some("little") => Ok(Endianness::Little),
        Some("big") => Ok(Endianness::Big),
        _ => Err(Error::plain(
            ErrorKind::TypeError,
            "expected an endianness symbol, little or big",
        )),
    }
}

/// The shared error for an access that reaches past the end of the payload.
fn out_of_bounds() -> Error {
    Error::plain(ErrorKind::RangeError, "index is outside the sequence")
}

/// The error for a `-native-` access whose index is not aligned to the width.
fn misaligned(size: usize) -> Error {
    Error::plain(
        ErrorKind::RangeError,
        format!("index must be a multiple of {size}"),
    )
}

/// The error for a decoded integer that does not fit the `i128` window.
fn unrepresentable() -> Error {
    Error::plain(
        ErrorKind::ImplementationRestriction,
        "value is outside the exact integer range",
    )
}

/// The error for a stored integer outside the representable range of its width.
fn value_out_of_range() -> Error {
    Error::plain(
        ErrorKind::RangeError,
        "value is outside the representable range for this width",
    )
}

/// Explains a failed multi-byte mutation, mirroring the error identity of the
/// base `bytevector-u8-set!`: an immutable target with a valid index gets the
/// immutability error, anything else the usual range-or-type error.
fn mutation_error(cx: &NativeContext<'_>, value: Value, index: usize) -> Error {
    sequence_mutation_error(
        cx,
        cx.heap.bytevector_len(value),
        index,
        "bytevector",
        value,
    )
}

/// Reads `N` bytes starting at `k`, with an alignment check for the native
/// accessors.
fn load<const N: usize>(
    cx: &NativeContext<'_>,
    value: Value,
    k: usize,
    aligned: bool,
) -> Result<[u8; N], Error> {
    if aligned && !k.is_multiple_of(N) {
        return Err(misaligned(N));
    }
    let bytes = cx
        .heap
        .bytevector_slice(value)
        .ok_or_else(|| type_error("bytevector", value, cx.heap))?;
    let end = k
        .checked_add(N)
        .filter(|end| *end <= bytes.len())
        .ok_or_else(out_of_bounds)?;
    let mut out = [0u8; N];
    out.copy_from_slice(&bytes[k..end]);
    Ok(out)
}

/// Writes `bytes` starting at `k`, with an alignment check for the native
/// accessors. The caller must have finished every argument conversion, so the
/// mutable payload borrow is the last thing this native does.
fn store(
    cx: &mut NativeContext<'_>,
    value: Value,
    k: usize,
    bytes: &[u8],
    aligned: bool,
) -> Result<Value, Error> {
    if aligned && !k.is_multiple_of(bytes.len()) {
        return Err(misaligned(bytes.len()));
    }
    let end = k.checked_add(bytes.len()).ok_or_else(out_of_bounds)?;
    let Some(target) = cx.heap.bytevector_slice_mut(value) else {
        return Err(mutation_error(cx, value, k));
    };
    match target.get_mut(k..end) {
        Some(slot) => {
            slot.copy_from_slice(bytes);
            Ok(Value::unspecified())
        }
        None => Err(out_of_bounds()),
    }
}

/// Converts an exact integer argument into the storage width, raising the
/// range error when it does not fit.
fn int_in_range<T: TryFrom<i128>>(cx: &NativeContext<'_>, value: Value) -> Result<T, Error> {
    T::try_from(exact_integer(cx, value)?).map_err(|_| value_out_of_range())
}

/// Generates the four accessors of one fixed integer width: the
/// endianness-taking ref and set! plus their aligned native-endianness forms.
macro_rules! fixed_int {
    ($t:ty, $size:literal, $ref_fn:ident, $native_ref_fn:ident, $set_fn:ident, $native_set_fn:ident) => {
        pub(crate) fn $ref_fn(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
            let k = index(cx, a[1])?;
            let endian = endianness_argument(cx, a[2])?;
            let bytes = load::<$size>(cx, a[0], k, false)?;
            let n = match endian {
                Endianness::Little => <$t>::from_le_bytes(bytes),
                Endianness::Big => <$t>::from_be_bytes(bytes),
            };
            cx.integer(i128::from(n))
        }

        pub(crate) fn $native_ref_fn(
            cx: &mut NativeContext<'_>,
            a: &[Value],
        ) -> Result<Value, Error> {
            let k = index(cx, a[1])?;
            let bytes = load::<$size>(cx, a[0], k, true)?;
            cx.integer(i128::from(<$t>::from_ne_bytes(bytes)))
        }

        pub(crate) fn $set_fn(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
            let k = index(cx, a[1])?;
            let n: $t = int_in_range(cx, a[2])?;
            let endian = endianness_argument(cx, a[3])?;
            let bytes = match endian {
                Endianness::Little => n.to_le_bytes(),
                Endianness::Big => n.to_be_bytes(),
            };
            store(cx, a[0], k, &bytes, false)
        }

        pub(crate) fn $native_set_fn(
            cx: &mut NativeContext<'_>,
            a: &[Value],
        ) -> Result<Value, Error> {
            let k = index(cx, a[1])?;
            let n: $t = int_in_range(cx, a[2])?;
            store(cx, a[0], k, &n.to_ne_bytes(), true)
        }
    };
}

fixed_int!(
    u16,
    2,
    bytevector_u16_ref,
    bytevector_u16_native_ref,
    bytevector_u16_set,
    bytevector_u16_native_set
);
fixed_int!(
    i16,
    2,
    bytevector_s16_ref,
    bytevector_s16_native_ref,
    bytevector_s16_set,
    bytevector_s16_native_set
);
fixed_int!(
    u32,
    4,
    bytevector_u32_ref,
    bytevector_u32_native_ref,
    bytevector_u32_set,
    bytevector_u32_native_set
);
fixed_int!(
    i32,
    4,
    bytevector_s32_ref,
    bytevector_s32_native_ref,
    bytevector_s32_set,
    bytevector_s32_native_set
);
fixed_int!(
    u64,
    8,
    bytevector_u64_ref,
    bytevector_u64_native_ref,
    bytevector_u64_set,
    bytevector_u64_native_set
);
fixed_int!(
    i64,
    8,
    bytevector_s64_ref,
    bytevector_s64_native_ref,
    bytevector_s64_set,
    bytevector_s64_native_set
);

/// Conversion glue between a stored IEEE-754 width and the engine's `f64`,
/// so the accessor macro expands without identity conversions at the `f64`
/// width.
trait IeeeWidth: Copy {
    fn widen(self) -> f64;
    fn narrow(x: f64) -> Self;
}

impl IeeeWidth for f32 {
    fn widen(self) -> f64 {
        f64::from(self)
    }

    fn narrow(x: f64) -> Self {
        x as f32
    }
}

impl IeeeWidth for f64 {
    fn widen(self) -> f64 {
        self
    }

    fn narrow(x: f64) -> Self {
        x
    }
}

/// Generates the four accessors of one IEEE-754 width. Reads widen to `f64`.
/// Writes accept any real number and store its `f64` value, rounded to `f32`
/// for the single-precision forms.
macro_rules! ieee {
    ($t:ty, $size:literal, $ref_fn:ident, $native_ref_fn:ident, $set_fn:ident, $native_set_fn:ident) => {
        pub(crate) fn $ref_fn(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
            let k = index(cx, a[1])?;
            let endian = endianness_argument(cx, a[2])?;
            let bytes = load::<$size>(cx, a[0], k, false)?;
            let x = match endian {
                Endianness::Little => <$t>::from_le_bytes(bytes),
                Endianness::Big => <$t>::from_be_bytes(bytes),
            };
            Ok(Value::float(x.widen()))
        }

        pub(crate) fn $native_ref_fn(
            cx: &mut NativeContext<'_>,
            a: &[Value],
        ) -> Result<Value, Error> {
            let k = index(cx, a[1])?;
            let bytes = load::<$size>(cx, a[0], k, true)?;
            Ok(Value::float(<$t>::from_ne_bytes(bytes).widen()))
        }

        pub(crate) fn $set_fn(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
            let k = index(cx, a[1])?;
            let x = <$t>::narrow(crate::number::to_f64(real_argument(cx, a[2])?));
            let endian = endianness_argument(cx, a[3])?;
            let bytes = match endian {
                Endianness::Little => x.to_le_bytes(),
                Endianness::Big => x.to_be_bytes(),
            };
            store(cx, a[0], k, &bytes, false)
        }

        pub(crate) fn $native_set_fn(
            cx: &mut NativeContext<'_>,
            a: &[Value],
        ) -> Result<Value, Error> {
            let k = index(cx, a[1])?;
            let x = <$t>::narrow(crate::number::to_f64(real_argument(cx, a[2])?));
            store(cx, a[0], k, &x.to_ne_bytes(), true)
        }
    };
}

ieee!(
    f32,
    4,
    bytevector_ieee_single_ref,
    bytevector_ieee_single_native_ref,
    bytevector_ieee_single_set,
    bytevector_ieee_single_native_set
);
ieee!(
    f64,
    8,
    bytevector_ieee_double_ref,
    bytevector_ieee_double_native_ref,
    bytevector_ieee_double_set,
    bytevector_ieee_double_native_set
);

/// `(native-endianness)`. The endianness symbol of the host architecture.
pub(crate) fn native_endianness(cx: &mut NativeContext<'_>, _: &[Value]) -> Result<Value, Error> {
    cx.intern_symbol(match NATIVE_ENDIANNESS {
        Endianness::Little => "little",
        Endianness::Big => "big",
    })
}

/// `(bytevector=? bytevector1 bytevector2)`. True when both have the same
/// length and equal bytes at every index.
pub(crate) fn bytevector_equal(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let left = cx
        .heap
        .bytevector_slice(a[0])
        .ok_or_else(|| type_error("bytevector", a[0], cx.heap))?;
    let right = cx
        .heap
        .bytevector_slice(a[1])
        .ok_or_else(|| type_error("bytevector", a[1], cx.heap))?;
    Ok(Value::boolean(left == right))
}

/// `(bytevector-fill! bytevector fill)`. Stores `fill` in every element. The
/// fill accepts the R6RS signed range -128 through 255.
pub(crate) fn bytevector_fill(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let fill = fill_byte(cx, a[1])?;
    match cx.heap.bytevector_slice_mut(a[0]) {
        Some(slice) => {
            slice.fill(fill);
            Ok(Value::unspecified())
        }
        None => Err(
            if cx.heap.bytevector_len(a[0]).is_some() && cx.heap.is_immutable(a[0]) {
                immutable_error("bytevector")
            } else {
                type_error("bytevector", a[0], cx.heap)
            },
        ),
    }
}

/// `(bytevector-s8-ref bytevector k)`. The byte at `k` as a signed byte.
pub(crate) fn bytevector_s8_ref(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let i = index(cx, a[1])?;
    cx.heap
        .bytevector_ref(a[0], i)
        .map(|value| Value::integer(i64::from(value as i8)))
        .ok_or_else(|| range_or_type(cx.heap.bytevector_len(a[0]), "bytevector", a[0]))
}

/// `(bytevector-s8-set! bytevector k byte)`. Stores the two's complement of
/// `byte`, which must lie in -128 through 127.
pub(crate) fn bytevector_s8_set(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let i = index(cx, a[1])?;
    let n = exact_integer(cx, a[2])?;
    if !(-128..=127).contains(&n) {
        return Err(Error::plain(
            ErrorKind::RangeError,
            "byte must be an exact integer from -128 through 127",
        ));
    }
    if cx.heap.bytevector_set(a[0], i, (n & 0xFF) as u8) {
        Ok(Value::unspecified())
    } else {
        Err(mutation_error(cx, a[0], i))
    }
}

/// Decodes one integer of `window.len()` bytes in the given byte order,
/// raising when the value does not fit the `i128` window. Redundant leading
/// bytes accumulate without overflow, so oversized encodings of small values
/// stay representable.
fn decode_int(window: &[u8], endian: Endianness, signed: bool) -> Result<i128, Error> {
    let mut acc: i128 = match (signed, endian) {
        (true, Endianness::Big) if window[0] >= 0x80 => -1,
        (true, Endianness::Little) if window[window.len() - 1] >= 0x80 => -1,
        _ => 0,
    };
    let mut step = |byte: u8| {
        acc = acc
            .checked_mul(256)
            .and_then(|shifted| shifted.checked_add(i128::from(byte)))
            .ok_or_else(unrepresentable)?;
        Ok::<(), Error>(())
    };
    match endian {
        Endianness::Big => {
            for &byte in window {
                step(byte)?;
            }
        }
        Endianness::Little => {
            for &byte in window.iter().rev() {
                step(byte)?;
            }
        }
    }
    Ok(acc)
}

/// Range-checks `n` for a `size`-byte encoding and appends its bytes in the
/// given byte order. For sizes of 16 bytes and up every signed `i128` (and
/// every non-negative one for unsigned) is encodable, the sign extension
/// filling the redundant leading bytes.
fn encode_int(
    n: i128,
    endian: Endianness,
    size: usize,
    signed: bool,
    out: &mut Vec<u8>,
) -> Result<(), Error> {
    let in_range = if size >= 16 {
        signed || n >= 0
    } else {
        let bound = 1i128 << (8 * size);
        if signed {
            (-bound / 2..bound / 2).contains(&n)
        } else {
            (0..bound).contains(&n)
        }
    };
    if !in_range {
        return Err(value_out_of_range());
    }
    let start = out.len();
    let mut value = n;
    for _ in 0..size {
        out.push(value as u8);
        // An arithmetic shift, so the sign extends into redundant bytes.
        value >>= 8;
    }
    if endian == Endianness::Big {
        out[start..].reverse();
    }
    Ok(())
}

/// Decodes a size argument, which must be a positive exact integer.
fn size_argument(cx: &NativeContext<'_>, value: Value) -> Result<usize, Error> {
    let size = index(cx, value)?;
    if size == 0 {
        return Err(Error::plain(
            ErrorKind::RangeError,
            "size must be a positive exact integer",
        ));
    }
    Ok(size)
}

/// The shared implementation of `bytevector-uint-ref` and
/// `bytevector-sint-ref`.
fn arbitrary_ref(cx: &mut NativeContext<'_>, a: &[Value], signed: bool) -> Result<Value, Error> {
    let k = index(cx, a[1])?;
    let endian = endianness_argument(cx, a[2])?;
    let size = size_argument(cx, a[3])?;
    let bytes = cx
        .heap
        .bytevector_slice(a[0])
        .ok_or_else(|| type_error("bytevector", a[0], cx.heap))?;
    let end = k
        .checked_add(size)
        .filter(|end| *end <= bytes.len())
        .ok_or_else(out_of_bounds)?;
    let value = decode_int(&bytes[k..end], endian, signed)?;
    cx.integer(value)
}

/// The shared implementation of `bytevector-uint-set!` and
/// `bytevector-sint-set!`.
fn arbitrary_set(cx: &mut NativeContext<'_>, a: &[Value], signed: bool) -> Result<Value, Error> {
    let k = index(cx, a[1])?;
    let n = exact_integer(cx, a[2])?;
    let endian = endianness_argument(cx, a[3])?;
    let size = size_argument(cx, a[4])?;
    // The bounds check runs before the encode buffer is sized, so a huge size
    // argument fails as out of range instead of attempting the allocation.
    let len = cx
        .heap
        .bytevector_len(a[0])
        .ok_or_else(|| type_error("bytevector", a[0], cx.heap))?;
    if k.checked_add(size).is_none_or(|end| end > len) {
        return Err(out_of_bounds());
    }
    let mut bytes = Vec::with_capacity(size);
    encode_int(n, endian, size, signed, &mut bytes)?;
    store(cx, a[0], k, &bytes, false)
}

/// `(bytevector-uint-ref bytevector k endianness size)`.
pub(crate) fn bytevector_uint_ref(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    arbitrary_ref(cx, a, false)
}

/// `(bytevector-sint-ref bytevector k endianness size)`.
pub(crate) fn bytevector_sint_ref(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    arbitrary_ref(cx, a, true)
}

/// `(bytevector-uint-set! bytevector k n endianness size)`.
pub(crate) fn bytevector_uint_set(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    arbitrary_set(cx, a, false)
}

/// `(bytevector-sint-set! bytevector k n endianness size)`.
pub(crate) fn bytevector_sint_set(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    arbitrary_set(cx, a, true)
}

/// Collects the elements of a proper list, raising on an improper or circular
/// argument. The walk uses a tortoise and hare because a native loop never
/// reaches a fuel safe point.
fn list_elements(cx: &NativeContext<'_>, list: Value) -> Result<Vec<Value>, Error> {
    let mut elements = Vec::new();
    let mut hare = list;
    let mut tortoise = list;
    loop {
        for _ in 0..2 {
            if hare == Value::nil() {
                return Ok(elements);
            }
            let Some((car, next)) = cx.heap.pair(hare) else {
                return Err(type_error("proper list", hare, cx.heap));
            };
            elements.push(car);
            hare = next;
        }
        if let Some((_, next)) = cx.heap.pair(tortoise) {
            tortoise = next;
        }
        if tortoise == hare {
            return Err(Error::plain(
                ErrorKind::TypeError,
                "expected a proper list, received a circular list",
            ));
        }
    }
}

/// Builds a list from decoded integers, consing back to front. The values may
/// heap-allocate beyond the inline fixnum range, and every intermediate stays
/// rooted through the context's root stack.
fn integer_list(cx: &mut NativeContext<'_>, values: &[i128]) -> Result<Value, Error> {
    let mut result = Value::nil();
    for &value in values.iter().rev() {
        let element = cx.integer(value)?;
        result = cx.pair(element, result)?;
    }
    Ok(result)
}

/// `(bytevector->u8-list bytevector)`. The octets in index order.
pub(crate) fn bytevector_to_u8_list(
    cx: &mut NativeContext<'_>,
    a: &[Value],
) -> Result<Value, Error> {
    let bytes = bytevector_argument(cx, a[0])?;
    let mut result = Value::nil();
    for &byte in bytes.iter().rev() {
        result = cx.pair(Value::integer(i64::from(byte)), result)?;
    }
    Ok(result)
}

/// `(u8-list->bytevector list)`. A bytevector of the listed octets.
pub(crate) fn u8_list_to_bytevector(
    cx: &mut NativeContext<'_>,
    a: &[Value],
) -> Result<Value, Error> {
    let elements = list_elements(cx, a[0])?;
    let mut bytes = Vec::with_capacity(elements.len());
    for element in elements {
        bytes.push(byte(cx, element)?);
    }
    cx.bytevector(bytes)
}

/// The shared implementation of `bytevector->uint-list` and
/// `bytevector->sint-list`.
fn to_int_list(cx: &mut NativeContext<'_>, a: &[Value], signed: bool) -> Result<Value, Error> {
    let bytes = bytevector_argument(cx, a[0])?;
    let endian = endianness_argument(cx, a[1])?;
    let size = size_argument(cx, a[2])?;
    if !bytes.len().is_multiple_of(size) {
        return Err(Error::plain(
            ErrorKind::RangeError,
            "bytevector length must be divisible by the element size",
        ));
    }
    let values = bytes
        .chunks_exact(size)
        .map(|window| decode_int(window, endian, signed))
        .collect::<Result<Vec<_>, _>>()?;
    integer_list(cx, &values)
}

/// The shared implementation of `uint-list->bytevector` and
/// `sint-list->bytevector`.
fn from_int_list(cx: &mut NativeContext<'_>, a: &[Value], signed: bool) -> Result<Value, Error> {
    let endian = endianness_argument(cx, a[1])?;
    let size = size_argument(cx, a[2])?;
    let elements = list_elements(cx, a[0])?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve(elements.len().saturating_mul(size))
        .map_err(|_| Error::plain(ErrorKind::HeapLimitExceeded, "bytevector is too large"))?;
    for element in elements {
        let value = exact_integer(cx, element)?;
        encode_int(value, endian, size, signed, &mut bytes)?;
    }
    cx.bytevector(bytes)
}

/// `(bytevector->uint-list bytevector endianness size)`.
pub(crate) fn bytevector_to_uint_list(
    cx: &mut NativeContext<'_>,
    a: &[Value],
) -> Result<Value, Error> {
    to_int_list(cx, a, false)
}

/// `(bytevector->sint-list bytevector endianness size)`.
pub(crate) fn bytevector_to_sint_list(
    cx: &mut NativeContext<'_>,
    a: &[Value],
) -> Result<Value, Error> {
    to_int_list(cx, a, true)
}

/// `(uint-list->bytevector list endianness size)`.
pub(crate) fn uint_list_to_bytevector(
    cx: &mut NativeContext<'_>,
    a: &[Value],
) -> Result<Value, Error> {
    from_int_list(cx, a, false)
}

/// `(sint-list->bytevector list endianness size)`.
pub(crate) fn sint_list_to_bytevector(
    cx: &mut NativeContext<'_>,
    a: &[Value],
) -> Result<Value, Error> {
    from_int_list(cx, a, true)
}

/// `(string->utf16 string)` or `(string->utf16 string endianness)`. UTF-16
/// without a byte order mark, big-endian by default.
pub(crate) fn string_to_utf16(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let text = string_argument(cx, a[0])?;
    let endian = match a.get(1) {
        Some(value) => endianness_argument(cx, *value)?,
        None => Endianness::Big,
    };
    let mut bytes = Vec::with_capacity(text.len() * 2);
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&match endian {
            Endianness::Little => unit.to_le_bytes(),
            Endianness::Big => unit.to_be_bytes(),
        });
    }
    cx.bytevector(bytes)
}

/// `(string->utf32 string)` or `(string->utf32 string endianness)`. UTF-32
/// without a byte order mark, big-endian by default.
pub(crate) fn string_to_utf32(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let text = string_argument(cx, a[0])?;
    let endian = match a.get(1) {
        Some(value) => endianness_argument(cx, *value)?,
        None => Endianness::Big,
    };
    let mut bytes = Vec::with_capacity(text.chars().count() * 4);
    for scalar in text.chars() {
        let scalar = scalar as u32;
        bytes.extend_from_slice(&match endian {
            Endianness::Little => scalar.to_le_bytes(),
            Endianness::Big => scalar.to_be_bytes(),
        });
    }
    cx.bytevector(bytes)
}

/// `(utf16->string bytevector endianness)` with an optional
/// endianness-mandatory flag. Without the flag a leading byte order mark
/// selects the endianness and is consumed. Decoding replaces invalid or
/// incomplete encodings with U+FFFD.
pub(crate) fn utf16_to_string(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let bytes = bytevector_argument(cx, a[0])?;
    let mut endian = endianness_argument(cx, a[1])?;
    let mandatory = a.get(2).copied().is_some_and(truthy);
    let mut start = 0;
    if !mandatory && bytes.len() >= 2 {
        if bytes[0] == 0xFE && bytes[1] == 0xFF {
            endian = Endianness::Big;
            start = 2;
        } else if bytes[0] == 0xFF && bytes[1] == 0xFE {
            endian = Endianness::Little;
            start = 2;
        }
    }
    let chunks = bytes[start..].chunks_exact(2);
    let partial_tail = !chunks.remainder().is_empty();
    let units = chunks.map(|pair| {
        let pair = [pair[0], pair[1]];
        match endian {
            Endianness::Little => u16::from_le_bytes(pair),
            Endianness::Big => u16::from_be_bytes(pair),
        }
    });
    let mut text = String::new();
    for result in char::decode_utf16(units) {
        text.push(result.unwrap_or(char::REPLACEMENT_CHARACTER));
    }
    if partial_tail {
        text.push(char::REPLACEMENT_CHARACTER);
    }
    cx.string_utf8(text)
}

/// `(utf32->string bytevector endianness)` with an optional
/// endianness-mandatory flag. Without the flag a leading byte order mark
/// selects the endianness and is consumed. Decoding replaces invalid or
/// incomplete encodings with U+FFFD.
pub(crate) fn utf32_to_string(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let bytes = bytevector_argument(cx, a[0])?;
    let mut endian = endianness_argument(cx, a[1])?;
    let mandatory = a.get(2).copied().is_some_and(truthy);
    let mut start = 0;
    if !mandatory && bytes.len() >= 4 {
        if bytes[..4] == [0x00, 0x00, 0xFE, 0xFF] {
            endian = Endianness::Big;
            start = 4;
        } else if bytes[..4] == [0xFF, 0xFE, 0x00, 0x00] {
            endian = Endianness::Little;
            start = 4;
        }
    }
    let chunks = bytes[start..].chunks_exact(4);
    let partial_tail = !chunks.remainder().is_empty();
    let mut text = String::new();
    for window in chunks {
        let window = [window[0], window[1], window[2], window[3]];
        let scalar = match endian {
            Endianness::Little => u32::from_le_bytes(window),
            Endianness::Big => u32::from_be_bytes(window),
        };
        text.push(char::from_u32(scalar).unwrap_or(char::REPLACEMENT_CHARACTER));
    }
    if partial_tail {
        text.push(char::REPLACEMENT_CHARACTER);
    }
    cx.string_utf8(text)
}
