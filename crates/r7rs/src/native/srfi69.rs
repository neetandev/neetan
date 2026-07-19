//! Native primitives backing the SRFI 69 (Basic Hash Tables) extension.
//!
//! Only the four hash functions live here. The table operations of SRFI 69 take
//! a user comparison, a user hash function, or a callback, none of which a
//! native can invoke, so they are defined in the `(srfi 69)` Scheme wrapper. The
//! functions below turn a value into an exact integer with no callback, backed
//! by the fixed-seed foldhash core in [`crate::hash`].
//!
//! `hash` is acceptable for `equal?`, `string-hash` for `string=?`,
//! `string-ci-hash` for `string-ci=?`, and `hash-by-identity` for `eq?`. In each
//! case two keys the predicate deems equal produce the same integer, which is
//! what the specification requires of an acceptable hash function.

use super::{NativeContext, scalar::unicode_fold, type_error};
use crate::{
    Error, ErrorKind, Value,
    hash::{FoldHasher, hash_bytes},
    printer::{RuntimeWriteMode, write_value},
    value::ValueRepr,
};

/// The default hash bound, `2^29 - 3`, matching the SRFI 69 reference. It is a
/// prime comfortably inside the fixnum range, so a default-bounded hash never
/// needs a heap integer.
const DEFAULT_BOUND: i128 = 536_870_909;

/// An upper bound on the nodes visited while hashing one structure. A key with a
/// cycle has unspecified behaviour under the SRFI, so the traversal simply stops
/// once it has folded this many nodes rather than looping forever.
const MAX_NODES: u32 = 65_536;

/// Structure tags kept distinct so values of different shapes do not collide
/// through a shared byte content. Numbers are split by representation because an
/// exact and an inexact number are never `equal?`.
mod tag {
    pub(super) const NIL: u64 = 1;
    pub(super) const BOOLEAN: u64 = 2;
    pub(super) const CHARACTER: u64 = 3;
    pub(super) const FIXNUM: u64 = 4;
    pub(super) const FLOAT: u64 = 5;
    pub(super) const EOF: u64 = 6;
    pub(super) const UNSPECIFIED: u64 = 7;
    pub(super) const UNDEFINED: u64 = 8;
    pub(super) const PAIR: u64 = 9;
    pub(super) const VECTOR: u64 = 10;
    pub(super) const STRING: u64 = 11;
    pub(super) const SYMBOL: u64 = 12;
    pub(super) const BYTEVECTOR: u64 = 13;
    pub(super) const NUMBER: u64 = 14;
    pub(super) const OPAQUE: u64 = 15;
}

/// The canonical bit pattern for any NaN, so every NaN key hashes alike.
const CANONICAL_NAN: u64 = 0x7FF8_0000_0000_0000;

/// Reads the optional `bound` argument. Absent means [`DEFAULT_BOUND`]. A
/// present bound must be a positive exact integer.
fn read_bound(cx: &NativeContext<'_>, args: &[Value], name: &str) -> Result<u128, Error> {
    match args.get(1) {
        None => Ok(DEFAULT_BOUND as u128),
        Some(&value) => {
            let bound = cx.to_i128(value)?;
            if bound <= 0 {
                return Err(Error::plain(
                    ErrorKind::RangeError,
                    format!("{name}: bound must be a positive exact integer"),
                ));
            }
            Ok(bound as u128)
        }
    }
}

/// Reduces a raw 64-bit hash into `[0, bound)` and returns it as an exact
/// integer.
fn bounded(cx: &mut NativeContext<'_>, raw: u64, bound: u128) -> Result<Value, Error> {
    cx.integer((raw as u128 % bound) as i128)
}

/// Folds one value into `hasher`, treating every number by its representation
/// and every other opaque object by identity. Called for each node during the
/// structural walk.
fn fold_atom(hasher: &mut FoldHasher, value: Value) {
    match value.decode() {
        ValueRepr::Nil => hasher.write_u64(tag::NIL),
        ValueRepr::Boolean(flag) => {
            hasher.write_u64(tag::BOOLEAN);
            hasher.write_u64(flag as u64);
        }
        ValueRepr::Character(ch) => {
            hasher.write_u64(tag::CHARACTER);
            hasher.write_u64(ch as u64);
        }
        ValueRepr::Fixnum(number) => {
            hasher.write_u64(tag::FIXNUM);
            hasher.write_u64(number as u64);
        }
        ValueRepr::Float(number) => {
            hasher.write_u64(tag::FLOAT);
            let bits = if number.is_nan() {
                CANONICAL_NAN
            } else {
                number.to_bits()
            };
            hasher.write_u64(bits);
        }
        ValueRepr::Eof => hasher.write_u64(tag::EOF),
        ValueRepr::Unspecified => hasher.write_u64(tag::UNSPECIFIED),
        ValueRepr::Undefined => hasher.write_u64(tag::UNDEFINED),
        // Heap values are handled by the walk, which only routes leaf heap
        // objects here. Compound heap objects never reach this arm.
        ValueRepr::Heap(_) => {
            hasher.write_u64(tag::OPAQUE);
            hasher.write_u64(value.0 as u64);
            hasher.write_u64((value.0 >> 64) as u64);
        }
    }
}

/// Hashes a value deeply and in a way acceptable for `equal?`. Pairs, vectors,
/// strings, symbols, and bytevectors are walked by content. Numbers hash by
/// their canonical form. Anything else hashes by identity. The walk unfolds
/// shared structure so that two `equal?` values hash alike regardless of
/// sharing, and stops after [`MAX_NODES`] nodes to bound cyclic keys.
fn hash_value(cx: &NativeContext<'_>, root: Value) -> u64 {
    let mut hasher = FoldHasher::new();
    let mut stack = vec![root];
    let mut budget = MAX_NODES;
    while let Some(value) = stack.pop() {
        if budget == 0 {
            break;
        }
        budget -= 1;
        if value.heap_ref().is_none() {
            fold_atom(&mut hasher, value);
            continue;
        }
        if let Some((car, cdr)) = cx.heap.pair(value) {
            hasher.write_u64(tag::PAIR);
            stack.push(cdr);
            stack.push(car);
        } else if let Some(text) = cx.heap.string_slice(value) {
            hasher.write_u64(tag::STRING);
            hasher.write_bytes(text.as_bytes());
        } else if let Some(name) = cx.heap.symbol(value) {
            hasher.write_u64(tag::SYMBOL);
            hasher.write_bytes(name.as_bytes());
        } else if let Some(bytes) = cx.heap.bytevector_slice(value) {
            hasher.write_u64(tag::BYTEVECTOR);
            hasher.write_bytes(bytes);
        } else if let Some(elements) = cx.heap.vector(value) {
            hasher.write_u64(tag::VECTOR);
            hasher.write_u64(elements.len() as u64);
            for element in elements.into_iter().rev() {
                stack.push(element);
            }
        } else if cx.heap.number(value).is_some() {
            hasher.write_u64(tag::NUMBER);
            let text = write_value(cx.heap, value, RuntimeWriteMode::Write).unwrap_or_default();
            hasher.write_bytes(text.as_bytes());
        } else {
            fold_atom(&mut hasher, value);
        }
    }
    hasher.finish()
}

/// `(hash obj [bound])`. An acceptable hash function for `equal?`.
pub(crate) fn hash(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    let bound = read_bound(cx, args, "hash")?;
    let raw = hash_value(cx, args[0]);
    bounded(cx, raw, bound)
}

/// `(string-hash string [bound])`. An acceptable hash function for `string=?`.
pub(crate) fn string_hash(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    let raw = {
        let text = cx
            .heap
            .string_slice(args[0])
            .ok_or_else(|| type_error("string", args[0], cx.heap))?;
        hash_bytes(text.as_bytes())
    };
    let bound = read_bound(cx, args, "string-hash")?;
    bounded(cx, raw, bound)
}

/// `(string-ci-hash string [bound])`. An acceptable hash function for
/// `string-ci=?`, which folds case with the same `unicode_fold` that
/// `string-ci=?` compares through.
pub(crate) fn string_ci_hash(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    let raw = {
        let text = cx
            .heap
            .string_slice(args[0])
            .ok_or_else(|| type_error("string", args[0], cx.heap))?;
        hash_bytes(unicode_fold(text).as_bytes())
    };
    let bound = read_bound(cx, args, "string-ci-hash")?;
    bounded(cx, raw, bound)
}

/// `(hash-by-identity obj [bound])`. An acceptable hash function for `eq?`. It
/// folds the raw tagged word, which is object identity for heap values and the
/// value itself for immediates.
pub(crate) fn hash_by_identity(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    let value = args[0];
    let mut hasher = FoldHasher::new();
    hasher.write_u64(value.0 as u64);
    hasher.write_u64((value.0 >> 64) as u64);
    let bound = read_bound(cx, args, "hash-by-identity")?;
    bounded(cx, hasher.finish(), bound)
}

#[cfg(test)]
mod tests {
    use crate::{Engine, EngineConfig, ErrorKind, Extension, Value};

    fn engine() -> Engine {
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        engine.install_extension(Extension::Srfi69).unwrap();
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
    fn every_hash_lands_inside_its_bound() {
        let mut engine = engine();
        assert_eq!(
            run(
                &mut engine,
                r#"
                (import (srfi 69) (scheme base))
                (define (ok n) (and (exact-integer? n) (<= 0 n) (< n 100)))
                (and (ok (hash '(1 2 3) 100))
                     (ok (string-hash "abc" 100))
                     (ok (string-ci-hash "AbC" 100))
                     (ok (hash-by-identity 'sym 100)))
                "#,
            ),
            Value::boolean(true)
        );
    }

    #[test]
    fn equal_structures_hash_alike() {
        let mut engine = engine();
        // Two distinct but equal? lists, one built with shared substructure,
        // must hash the same for the default equal? table to work.
        assert_eq!(
            run(
                &mut engine,
                r#"
                (import (srfi 69) (scheme base))
                (define inner (list 1 2))
                (= (hash (list inner inner)) (hash (list (list 1 2) (list 1 2))))
                "#,
            ),
            Value::boolean(true)
        );
    }

    #[test]
    fn string_ci_hash_ignores_case() {
        let mut engine = engine();
        assert_eq!(
            run(
                &mut engine,
                r#"(import (srfi 69)) (= (string-ci-hash "Hello") (string-ci-hash "hELLO"))"#,
            ),
            Value::boolean(true)
        );
    }

    #[test]
    fn a_supplied_bound_is_respected() {
        let mut engine = engine();
        assert_eq!(
            run(
                &mut engine,
                r#"(import (srfi 69)) (< (string-hash "anything" 7) 7)"#,
            ),
            Value::boolean(true)
        );
    }

    #[test]
    fn a_non_positive_bound_is_a_range_error() {
        let mut engine = engine();
        assert_eq!(
            error_kind(&mut engine, r#"(import (srfi 69)) (hash 'x 0)"#),
            ErrorKind::RangeError
        );
        assert_eq!(
            error_kind(&mut engine, r#"(import (srfi 69)) (string-hash "x" -4)"#),
            ErrorKind::RangeError
        );
    }

    #[test]
    fn a_non_string_to_string_hash_is_a_type_error() {
        let mut engine = engine();
        assert_eq!(
            error_kind(&mut engine, r#"(import (srfi 69)) (string-hash 42)"#),
            ErrorKind::TypeError
        );
    }
}
