use std::{cell::RefCell, rc::Rc};

/// The public category of a Scheme value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValueKind {
    /// An uninitialized internal binding sentinel.
    Undefined,
    /// The Scheme unspecified value.
    Unspecified,
    /// The empty list.
    Nil,
    /// A boolean.
    Boolean,
    /// A character.
    Character,
    /// An inline signed 64-bit exact integer.
    Fixnum,
    /// An inexact floating-point number.
    Float,
    /// The distinguished end-of-file object.
    Eof,
    /// A heap-backed exact integer, rational, or complex number.
    Number,
    /// A mutable pair.
    Pair,
    /// A mutable vector.
    Vector,
    /// A mutable bytevector.
    Bytevector,
    /// A mutable string.
    String,
    /// An interned symbol.
    Symbol,
    /// A Scheme closure.
    Procedure,
    /// A registered host procedure.
    NativeProcedure,
    /// A delayed computation.
    Promise,
    /// A dynamically scoped parameter object.
    Parameter,
    /// An evaluation environment.
    Environment,
    /// An R7RS record instance.
    Record,
    /// An R7RS record-type descriptor.
    RecordType,
    /// A textual, binary, input, or output port.
    Port,
    /// A SRFI 27 random source.
    RandomSource,
    /// Any heap object not yet exposed as a public data type.
    Heap,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct GcRef(pub(crate) u32);

// 128-bit tagged layout. `Value` is a `u128`. Since an `f64` needs only 64 bits,
// the low half of the word carries the primary payload (float bits, fixnum bits,
// or a heap slot) and a small tag rides in the very top bits, so no NaN-punning
// is needed:
//
//   * bits 113..=0   : payload (an `f64`'s raw bits occupy the low 64 of these).
//   * bits 127..=114 : a 14-bit tag selecting the value kind.
//
// A genuine `f64` is stored as its raw bits zero-extended to 128, landing it in
// tag 0 (`TAG_FLOAT`) automatically; every other kind sets a non-zero tag, so
// floats and boxed values can never collide.
//
// The payload field is 114 bits wide, so fixnums inline the full i64 range
// (the low 64 payload bits) and keep cheap native `i64` arithmetic.
const TAG_SHIFT: u32 = 114;

const TAG_FLOAT: u128 = 0;
const TAG_FIXNUM: u128 = 1;
const TAG_HEAP: u128 = 2;
const TAG_CHAR: u128 = 3;
const TAG_BOOL: u128 = 4;
const TAG_SINGLETON: u128 = 5;

// `Value::pair_key` combines two tags at a 3-bit stride, which is
// collision-free only while every tag stays below 8.
const _: () = assert!(TAG_SINGLETON < 8, "pair_key requires all tags below 8");

// Program NaNs are stored with these canonical bits so distinct NaN payloads
// never leak through bitwise value comparisons. The tag field keeps floats and
// boxed values apart, so this concerns only NaN identity, not disambiguation.
const CANON_NAN_BITS: u64 = 0x7FF8_0000_0000_0000;

const SINGLETON_UNDEFINED: u64 = 0;
const SINGLETON_UNSPECIFIED: u64 = 1;
const SINGLETON_NIL: u64 = 2;
const SINGLETON_EOF: u64 = 3;

#[inline(always)]
const fn box_fixnum(value: i64) -> u128 {
    (TAG_FIXNUM << TAG_SHIFT) | (value as u64 as u128)
}

#[inline(always)]
const fn box_other(tag: u128, payload: u64) -> u128 {
    (tag << TAG_SHIFT) | payload as u128
}

/// A compact opaque Scheme value (a 128-bit tagged word).
#[derive(Clone, Copy)]
pub struct Value(pub(crate) u128);

const _: () = assert!(size_of::<Value>() == 16, "Value must stay 16 bytes");

/// The decoded view of a [`Value`]. This is *not* the storage representation
/// (that is a 128-bit tagged word); `Value::decode` materializes it on demand so
/// in-crate code can pattern-match one variant per value kind.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ValueRepr {
    Undefined,
    Unspecified,
    Nil,
    Boolean(bool),
    Character(char),
    Fixnum(i64),
    Float(f64),
    Eof,
    Heap(GcRef),
}

impl Value {
    /// The unspecified value.
    pub const fn unspecified() -> Self {
        Self(box_other(TAG_SINGLETON, SINGLETON_UNSPECIFIED))
    }
    /// The empty list.
    pub const fn nil() -> Self {
        Self(box_other(TAG_SINGLETON, SINGLETON_NIL))
    }
    /// A boolean value.
    pub const fn boolean(value: bool) -> Self {
        Self(box_other(TAG_BOOL, value as u64))
    }
    /// A character value.
    pub const fn character(value: char) -> Self {
        Self(box_other(TAG_CHAR, value as u64))
    }
    /// An inline exact integer.
    ///
    /// The inline fixnum range is the full i64, so every `i64` argument is
    /// stored inline. Exact integers beyond i64 (only reachable past i64
    /// overflow) are heap-backed; construct those through
    /// [`crate::Engine::root_integer`] rather than this constructor.
    pub const fn integer(value: i64) -> Self {
        Self(box_fixnum(value))
    }
    /// An inexact floating-point value. Any NaN argument is canonicalized.
    pub const fn float(value: f64) -> Self {
        if value.is_nan() {
            Self(CANON_NAN_BITS as u128)
        } else {
            Self(value.to_bits() as u128)
        }
    }

    /// An inexact float without NaN canonicalization, for arithmetic results
    /// on the VM's hot paths. Any `f64` bit pattern is a valid float `Value`
    /// (`TAG_FLOAT` is zero with the payload in the low limb), so this only
    /// forgoes the per-result NaN branch of [`Self::float`]; NaN sign/payload
    /// bits are normalized at the observation sites instead (`eqv?` via
    /// `real_eqv` and the three `+nan.0` printers).
    #[inline(always)]
    pub(crate) const fn float_raw(value: f64) -> Self {
        Self(value.to_bits() as u128)
    }
    /// The distinguished end-of-file object.
    pub const fn eof() -> Self {
        Self(box_other(TAG_SINGLETON, SINGLETON_EOF))
    }

    /// The uninitialized internal binding sentinel.
    pub(crate) const fn undefined() -> Self {
        Self(box_other(TAG_SINGLETON, SINGLETON_UNDEFINED))
    }

    /// Wraps a heap slot index.
    pub(crate) const fn heap(reference: GcRef) -> Self {
        Self(box_other(TAG_HEAP, reference.0 as u64))
    }

    /// The 14-bit kind tag riding in the top bits of the word.
    #[inline(always)]
    const fn tag(self) -> u128 {
        self.0 >> TAG_SHIFT
    }

    /// Returns the inline fixnum value, if this is one.
    #[inline(always)]
    pub(crate) const fn as_fixnum(self) -> Option<i64> {
        if self.tag() == TAG_FIXNUM {
            Some(self.0 as u64 as i64)
        } else {
            None
        }
    }

    /// Returns the inexact float value, if this is one.
    #[inline(always)]
    pub(crate) const fn as_float(self) -> Option<f64> {
        if self.tag() == TAG_FLOAT {
            Some(f64::from_bits(self.0 as u64))
        } else {
            None
        }
    }

    /// The high 64-bit limb of the tagged word. Every kind stores its payload
    /// in the low 64 bits (an `f64`'s raw bits, a fixnum's `i64`, a heap
    /// slot's `u32`, a char/bool/singleton's small scalar), so bits 64..=113
    /// are always zero and the high limb is exactly `tag << (TAG_SHIFT - 64)`,
    /// a complete kind discriminant with no shifting. [`Self::pair_key`]
    /// builds the arithmetic fast path's two-operand classifier from it; the
    /// unit test `high_limb_is_a_pure_tag` pins the invariant.
    #[inline(always)]
    const fn high_limb(self) -> u64 {
        (self.0 >> 64) as u64
    }

    /// Raw tagged-word identity: true only when both values carry identical
    /// bits. Unlike `==` (which follows IEEE float semantics), this
    /// distinguishes `-0.0` from `0.0` and reports identical-bit NaNs as the
    /// same. Callers use it as an eqv-true witness: identical bits always
    /// denote operationally indistinguishable values.
    #[inline(always)]
    pub(crate) const fn same_bits(left: Self, right: Self) -> bool {
        left.0 == right.0
    }

    /// A single scalar classifying a pair of operands by kind: the two high
    /// limbs combined as `hi(a) + (hi(b) << 3)`. Because every tag is below 8
    /// (compile-time-asserted next to the tag constants), the combination is
    /// collision-free, so [`Self::PAIR_BOTH_FIXNUM`] and
    /// [`Self::PAIR_BOTH_FLOAT`] each match exactly one kind pair. Costs one
    /// shift and one add. Cheaper than extracting and packing both tags.
    #[inline(always)]
    pub(crate) const fn pair_key(a: Self, b: Self) -> u64 {
        a.high_limb() + (b.high_limb() << 3)
    }

    /// [`Self::pair_key`] result for two inline fixnums.
    pub(crate) const PAIR_BOTH_FIXNUM: u64 = {
        let fixnum_limb = (TAG_FIXNUM as u64) << (TAG_SHIFT - 64);
        fixnum_limb + (fixnum_limb << 3)
    };

    /// [`Self::pair_key`] result for two inline floats (zero: a float's high
    /// limb is zero because `TAG_FLOAT` is zero and its payload is 64 bits).
    pub(crate) const PAIR_BOTH_FLOAT: u64 = 0;

    /// [`Self::pair_key`] result for an inline fixnum paired with an inline
    /// float (the float's high limb contributes zero).
    pub(crate) const PAIR_FIXNUM_FLOAT: u64 = (TAG_FIXNUM as u64) << (TAG_SHIFT - 64);

    /// [`Self::pair_key`] result for an inline float paired with an inline
    /// fixnum.
    pub(crate) const PAIR_FLOAT_FIXNUM: u64 = ((TAG_FIXNUM as u64) << (TAG_SHIFT - 64)) << 3;

    /// The raw inline fixnum payload. The caller must have established the
    /// fixnum tag (e.g. via [`Self::pair_key`]).
    #[inline(always)]
    pub(crate) const fn fixnum_payload(self) -> i64 {
        self.0 as u64 as i64
    }

    /// The raw inline float payload. The caller must have established the
    /// float tag (e.g. via [`Self::pair_key`]).
    #[inline(always)]
    pub(crate) const fn float_payload(self) -> f64 {
        f64::from_bits(self.0 as u64)
    }

    /// Returns the heap slot reference, if this is a heap value.
    #[inline(always)]
    pub(crate) const fn heap_ref(self) -> Option<GcRef> {
        if self.tag() == TAG_HEAP {
            Some(GcRef(self.0 as u32))
        } else {
            None
        }
    }

    /// Materializes the decoded view for pattern-matching.
    #[inline(always)]
    pub(crate) fn decode(self) -> ValueRepr {
        let tag = self.tag();
        if tag == TAG_FLOAT {
            return ValueRepr::Float(f64::from_bits(self.0 as u64));
        }
        if tag == TAG_FIXNUM {
            return ValueRepr::Fixnum(self.0 as u64 as i64);
        }
        let payload = self.0 as u64;
        match tag {
            TAG_HEAP => ValueRepr::Heap(GcRef(payload as u32)),
            TAG_CHAR => ValueRepr::Character(char::from_u32(payload as u32).unwrap_or('\u{0}')),
            TAG_BOOL => ValueRepr::Boolean(payload & 1 != 0),
            _ => match payload {
                SINGLETON_UNSPECIFIED => ValueRepr::Unspecified,
                SINGLETON_NIL => ValueRepr::Nil,
                SINGLETON_EOF => ValueRepr::Eof,
                _ => ValueRepr::Undefined,
            },
        }
    }

    /// Returns the value category.
    pub const fn kind(self) -> ValueKind {
        let tag = self.tag();
        if tag == TAG_FLOAT {
            return ValueKind::Float;
        }
        if tag == TAG_FIXNUM {
            return ValueKind::Fixnum;
        }
        match tag {
            TAG_HEAP => ValueKind::Heap,
            TAG_CHAR => ValueKind::Character,
            TAG_BOOL => ValueKind::Boolean,
            _ => match self.0 as u64 {
                SINGLETON_UNSPECIFIED => ValueKind::Unspecified,
                SINGLETON_NIL => ValueKind::Nil,
                SINGLETON_EOF => ValueKind::Eof,
                _ => ValueKind::Undefined,
            },
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        // Preserve IEEE float semantics (NaN != NaN, -0.0 == 0.0). Because every
        // program NaN is canonicalized to identical bits, raw bit-equality would
        // wrongly report NaN == NaN, so floats are compared as `f64`. All other
        // values are distinguished purely by their bits.
        match (self.as_float(), other.as_float()) {
            (Some(left), Some(right)) => left == right,
            (None, None) => self.0 == other.0,
            _ => false,
        }
    }
}

impl std::fmt::Debug for Value {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.decode().fmt(formatter)
    }
}

pub(crate) type HostRoots = crate::slab::Slab<Value>;

/// An RAII root that keeps a Scheme value alive in its owning engine.
pub struct Root {
    pub(crate) roots: Rc<RefCell<HostRoots>>,
    key: crate::slab::Key,
    pub(crate) value: Value,
}

impl Root {
    /// Registers and constructs a root through the single allocation path.
    pub(crate) fn new(roots: Rc<RefCell<HostRoots>>, value: Value) -> Self {
        let key = roots.borrow_mut().insert(value);
        Self { roots, key, value }
    }

    /// Returns the rooted value.
    #[must_use]
    pub const fn value(&self) -> Value {
        self.value
    }
}

impl Clone for Root {
    fn clone(&self) -> Self {
        Self::new(self.roots.clone(), self.value)
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        self.roots.borrow_mut().remove(&self.key);
    }
}

impl std::fmt::Debug for Root {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_tuple("Root").field(&self.value).finish()
    }
}

/// A rooted packet of zero or more values returned by Scheme evaluation.
///
/// Scheme procedures may return any number of values. Keeping every member
/// rooted makes the packet safe to retain and inspect across collection.
#[derive(Debug)]
pub struct Values {
    values: Vec<Root>,
}

/// The non-error result of evaluating a Scheme module.
#[derive(Debug)]
pub enum EvalOutcome {
    /// Evaluation returned normally with zero or more values.
    Values(Values),
    /// Evaluation terminated through `exit` or `emergency-exit`.
    Exited(crate::ExitStatus),
}

impl EvalOutcome {
    /// Returns a terminal exit status, if evaluation exited.
    #[must_use]
    pub const fn exit_status(&self) -> Option<crate::ExitStatus> {
        match self {
            Self::Exited(status) => Some(*status),
            Self::Values(_) => None,
        }
    }

    /// Returns the number of normally returned values.
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Self::Values(values) => values.len(),
            Self::Exited(_) => 0,
        }
    }

    /// Returns whether evaluation returned no values or exited.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns normally produced rooted values, or an empty slice after exit.
    #[must_use]
    pub fn as_slice(&self) -> &[Root] {
        match self {
            Self::Values(values) => values.as_slice(),
            Self::Exited(_) => &[],
        }
    }

    /// Extracts the sole normally returned value.
    pub fn into_one(self) -> Result<Root, crate::Error> {
        match self {
            Self::Values(values) => values.into_one(),
            Self::Exited(_) => Err(crate::Error::plain(
                crate::ErrorKind::RuntimeError,
                "evaluation exited without returning values",
            )),
        }
    }
}

impl Values {
    pub(crate) fn new(values: Vec<Root>) -> Self {
        Self { values }
    }

    /// Returns the number of values in this result packet.
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Returns whether this packet contains no values.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Returns the rooted values in result order.
    #[must_use]
    pub fn as_slice(&self) -> &[Root] {
        &self.values
    }

    /// Consumes this packet and returns its rooted values.
    #[must_use]
    pub fn into_vec(self) -> Vec<Root> {
        self.values
    }

    /// Extracts the only value, reporting a structured error otherwise.
    pub fn into_one(mut self) -> Result<Root, crate::Error> {
        if self.values.len() == 1 {
            Ok(self.values.pop().expect("length checked"))
        } else {
            Err(crate::Error::plain(
                crate::ErrorKind::RuntimeError,
                format!("expected exactly one value, received {}", self.values.len()),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn high_limb_is_a_pure_tag() {
        // `pair_key` relies on every kind keeping its payload strictly in the
        // low 64 bits, so the high limb is exactly `tag << (TAG_SHIFT - 64)`.
        let limb = |tag: u128| (tag << (TAG_SHIFT - 64)) as u64;
        for value in [Value::integer(i64::MIN), Value::integer(i64::MAX)] {
            assert_eq!(value.high_limb(), limb(TAG_FIXNUM));
        }
        for value in [
            Value::float(f64::MIN),
            Value::float(f64::MAX),
            Value::float(f64::NAN),
            Value::float(f64::NEG_INFINITY),
            Value::float(-0.0),
        ] {
            assert_eq!(value.high_limb(), limb(TAG_FLOAT));
        }
        assert_eq!(Value::heap(GcRef(u32::MAX)).high_limb(), limb(TAG_HEAP));
        assert_eq!(Value::character('\u{10FFFF}').high_limb(), limb(TAG_CHAR));
        assert_eq!(Value::boolean(true).high_limb(), limb(TAG_BOOL));
        for value in [
            Value::nil(),
            Value::eof(),
            Value::unspecified(),
            Value::undefined(),
        ] {
            assert_eq!(value.high_limb(), limb(TAG_SINGLETON));
        }

        // The pair classifier keys are unique to their kind pairs.
        let fx = Value::integer(7);
        let fl = Value::float(7.0);
        assert_eq!(Value::pair_key(fx, fx), Value::PAIR_BOTH_FIXNUM);
        assert_eq!(Value::pair_key(fl, fl), Value::PAIR_BOTH_FLOAT);
        for (a, b) in [
            (fx, fl),
            (fl, fx),
            (fx, Value::nil()),
            (Value::boolean(false), fl),
        ] {
            let key = Value::pair_key(a, b);
            assert_ne!(key, Value::PAIR_BOTH_FIXNUM);
            assert_ne!(key, Value::PAIR_BOTH_FLOAT);
        }
    }
}
