//! Native primitives backing the SRFI 27 (Sources of Random Bits) extension.
//!
//! The draw procedures on the default source are registered as closures that
//! capture the default source (see `crate::embed::extensions`), so they call
//! [`draw_integer`] and [`draw_real`] with no per-draw Scheme frame. The
//! procedures returned by `random-source-make-integers` and
//! `random-source-make-reals` for an arbitrary source reach the same helpers
//! through [`random_integer_on`] and [`random_real_on`].

use std::time::{SystemTime, UNIX_EPOCH};

use super::{NativeContext, type_error};
use crate::{Error, ErrorKind, Value, heap::Object, random::SquaresRng};

/// The leading symbol of a random source's external state representation.
const STATE_TAG: &str = "squares-state";

/// Derives a 128-bit seed from the host wall clock. A clock reading before the
/// Unix epoch degrades to a zero seed rather than failing.
pub(crate) fn system_time_seed() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default()
}

/// Reads an exact integer seed argument. Any exact integer is accepted, its
/// 128 bits reinterpreted as the unsigned seed.
fn seed_from_value(cx: &NativeContext<'_>, value: Value) -> Result<u128, Error> {
    Ok(cx.to_i128(value)? as u128)
}

/// Allocates a fresh random source holding the given generator state.
fn alloc_source(cx: &mut NativeContext<'_>, rng: SquaresRng) -> Result<Value, Error> {
    cx.alloc(Object::RandomSource(rng))
}

/// Loads the generator state of a source argument, or reports a type error.
fn load_source(cx: &NativeContext<'_>, source: Value) -> Result<SquaresRng, Error> {
    cx.heap
        .random_source(source)
        .ok_or_else(|| type_error("random source", source, cx.heap))
}

/// `(make-random-source)` and `(make-random-source seed)`. With no argument the
/// source is seeded from the host wall clock. With an exact integer it is seeded deterministically.
pub(crate) fn make_random_source(
    cx: &mut NativeContext<'_>,
    args: &[Value],
) -> Result<Value, Error> {
    let seed = match args.first() {
        None => system_time_seed(),
        Some(&value) => seed_from_value(cx, value)?,
    };
    alloc_source(cx, SquaresRng::from_seed(seed))
}

/// `(random-source? obj)`.
pub(crate) fn random_source_p(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    Ok(Value::boolean(cx.heap.random_source(args[0]).is_some()))
}

/// Draws one integer in `{0, ..., n - 1}` from `source` and advances its state.
/// `n` must be a positive exact integer.
pub(crate) fn draw_integer(
    cx: &mut NativeContext<'_>,
    source: Value,
    n_value: Value,
) -> Result<Value, Error> {
    let mut rng = load_source(cx, source)?;
    let n = cx.to_i128(n_value)?;
    if n <= 0 {
        return Err(Error::plain(
            ErrorKind::RangeError,
            "random-integer: bound must be a positive exact integer",
        ));
    }
    let result = if n <= u64::MAX as i128 {
        i128::from(rng.next_below_u64(n as u64))
    } else {
        rng.next_below_u128(n as u128) as i128
    };
    cx.heap.set_random_source(source, rng);
    cx.integer(result)
}

/// Draws one real in the open interval `(0, 1)` from `source` and advances it.
pub(crate) fn draw_real(cx: &mut NativeContext<'_>, source: Value) -> Result<Value, Error> {
    let mut rng = load_source(cx, source)?;
    let value = rng.next_open_f64();
    cx.heap.set_random_source(source, rng);
    Ok(Value::float(value))
}

/// `(%random-integer-on source n)`, the backend of `random-source-make-integers`.
pub(crate) fn random_integer_on(
    cx: &mut NativeContext<'_>,
    args: &[Value],
) -> Result<Value, Error> {
    draw_integer(cx, args[0], args[1])
}

/// `(%random-real-on source)`, the backend of `random-source-make-reals`.
pub(crate) fn random_real_on(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    draw_real(cx, args[0])
}

/// `(random-source-state-ref source)`. Builds the external representation
/// `(squares-state counter key)`.
pub(crate) fn random_source_state_ref(
    cx: &mut NativeContext<'_>,
    args: &[Value],
) -> Result<Value, Error> {
    let rng = cx
        .heap
        .random_source(args[0])
        .ok_or_else(|| type_error("random source", args[0], cx.heap))?;
    let tag = cx.intern_symbol(STATE_TAG)?;
    let counter_value = cx.integer(rng.counter as i128)?;
    let key_value = cx.integer(rng.key as i128)?;
    let tail = cx.pair(key_value, Value::nil())?;
    let tail = cx.pair(counter_value, tail)?;
    cx.pair(tag, tail)
}

/// Builds the shared shape error for a malformed state representation.
fn state_shape_error() -> Error {
    Error::plain(
        ErrorKind::TypeError,
        "random source state must be a list (squares-state counter key)",
    )
}

/// Reads a stored word field of the state list. Both fields are exact integers
/// in the `u64` range.
fn state_word(cx: &NativeContext<'_>, value: Value) -> Result<u64, Error> {
    let word = cx.to_i128(value)?;
    u64::try_from(word).map_err(|_| {
        Error::plain(
            ErrorKind::RangeError,
            "random source state fields must be exact integers in [0, 2^64)",
        )
    })
}

/// Parses `(squares-state counter key)` into validated generator state.
fn parse_state(cx: &NativeContext<'_>, state: Value) -> Result<SquaresRng, Error> {
    let (tag, rest) = cx.heap.pair(state).ok_or_else(state_shape_error)?;
    match cx.heap.symbol(tag) {
        Some(name) if name == STATE_TAG => {}
        _ => return Err(state_shape_error()),
    }
    let (counter_value, rest) = cx.heap.pair(rest).ok_or_else(state_shape_error)?;
    let (key_value, rest) = cx.heap.pair(rest).ok_or_else(state_shape_error)?;
    if !Value::same_bits(rest, Value::nil()) {
        return Err(state_shape_error());
    }
    let counter = state_word(cx, counter_value)?;
    let key = state_word(cx, key_value)?;
    if !SquaresRng::key_is_valid(key) {
        return Err(Error::plain(
            ErrorKind::RangeError,
            "random source state carries an invalid squares key",
        ));
    }
    Ok(SquaresRng::from_parts(counter, key))
}

/// `(random-source-state-set! source state)`.
pub(crate) fn random_source_state_set(
    cx: &mut NativeContext<'_>,
    args: &[Value],
) -> Result<Value, Error> {
    let source = args[0];
    if cx.heap.random_source(source).is_none() {
        return Err(type_error("random source", source, cx.heap));
    }
    let rng = parse_state(cx, args[1])?;
    cx.heap.set_random_source(source, rng);
    Ok(Value::unspecified())
}

/// `(random-source-randomize! source)` and the deterministic
/// `(random-source-randomize! source seed)` extension.
pub(crate) fn random_source_randomize(
    cx: &mut NativeContext<'_>,
    args: &[Value],
) -> Result<Value, Error> {
    let source = args[0];
    if cx.heap.random_source(source).is_none() {
        return Err(type_error("random source", source, cx.heap));
    }
    let seed = match args.get(1) {
        None => system_time_seed(),
        Some(&value) => seed_from_value(cx, value)?,
    };
    let rng = SquaresRng::from_seed(seed);
    cx.heap.set_random_source(source, rng);
    Ok(Value::unspecified())
}

/// Reads an exact integer stream index. Any exact integer is accepted, its
/// 128 bits reinterpreted as the unsigned index.
fn stream_index(cx: &NativeContext<'_>, value: Value) -> Result<u128, Error> {
    Ok(cx.to_i128(value)? as u128)
}

/// `(random-source-pseudo-randomize! source i j)`.
pub(crate) fn random_source_pseudo_randomize(
    cx: &mut NativeContext<'_>,
    args: &[Value],
) -> Result<Value, Error> {
    let source = args[0];
    if cx.heap.random_source(source).is_none() {
        return Err(type_error("random source", source, cx.heap));
    }
    let i = stream_index(cx, args[1])?;
    let j = stream_index(cx, args[2])?;
    let rng = SquaresRng::pseudo_randomize(i, j);
    cx.heap.set_random_source(source, rng);
    Ok(Value::unspecified())
}

#[cfg(test)]
mod tests {
    use crate::{Engine, EngineConfig, ErrorKind, Extension, Value};

    fn engine() -> Engine {
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        engine.install_extension(Extension::Srfi27).unwrap();
        engine
    }

    fn run(engine: &mut Engine, source: &str) -> Value {
        let module = engine.compile("test.scm", source).unwrap();
        engine.eval(&module).unwrap().into_one().unwrap().value()
    }

    fn show(engine: &mut Engine, source: &str) -> String {
        let module = engine.compile("test.scm", source).unwrap();
        let root = engine.eval(&module).unwrap().into_one().unwrap();
        engine.write_root(&root).unwrap()
    }

    fn error_kind(engine: &mut Engine, source: &str) -> ErrorKind {
        let module = engine.compile("test.scm", source).unwrap();
        engine.eval(&module).unwrap_err().kind()
    }

    fn is_error(engine: &mut Engine, source: &str) -> bool {
        let module = engine.compile("test.scm", source).unwrap();
        engine.eval(&module).is_err()
    }

    #[test]
    fn random_integer_rejects_non_positive_bounds() {
        let mut engine = engine();
        assert_eq!(
            error_kind(&mut engine, "(import (srfi 27)) (random-integer 0)"),
            ErrorKind::RangeError
        );
        assert_eq!(
            error_kind(&mut engine, "(import (srfi 27)) (random-integer -3)"),
            ErrorKind::RangeError
        );
    }

    #[test]
    fn random_integer_rejects_non_integer_bounds() {
        let mut engine = engine();
        assert_eq!(
            error_kind(&mut engine, "(import (srfi 27)) (random-integer 1.5)"),
            ErrorKind::TypeError
        );
        assert_eq!(
            error_kind(&mut engine, "(import (srfi 27)) (random-integer \"x\")"),
            ErrorKind::TypeError
        );
    }

    #[test]
    fn random_integer_of_one_is_always_zero() {
        let mut engine = engine();
        assert_eq!(
            run(
                &mut engine,
                r#"
                (import (srfi 27))
                (let loop ((i 0) (ok #t))
                  (if (= i 1000)
                      ok
                      (loop (+ i 1) (and ok (= 0 (random-integer 1))))))
                "#,
            ),
            Value::boolean(true)
        );
    }

    #[test]
    fn random_integer_handles_bounds_above_u64() {
        let mut engine = engine();
        // 2^64 + 1 exceeds u64::MAX and exercises the u128 sampling path.
        assert_eq!(
            run(
                &mut engine,
                r#"
                (import (srfi 27))
                (define n 18446744073709551617)
                (let loop ((i 0) (ok #t))
                  (if (= i 1000)
                      ok
                      (let ((x (random-integer n)))
                        (loop (+ i 1) (and ok (<= 0 x) (< x n))))))
                "#,
            ),
            Value::boolean(true)
        );
    }

    #[test]
    fn state_set_rejects_malformed_states() {
        let mut engine = engine();
        let prelude = "(import (srfi 27)) (define s (make-random-source 1))";
        let cases = [
            (
                format!("{prelude} (random-source-state-set! s 5)"),
                ErrorKind::TypeError,
            ),
            (
                format!("{prelude} (random-source-state-set! s '(bogus 0 1))"),
                ErrorKind::TypeError,
            ),
            (
                format!("{prelude} (random-source-state-set! s '(squares-state 0))"),
                ErrorKind::TypeError,
            ),
            (
                format!("{prelude} (random-source-state-set! s '(squares-state 0 1 2))"),
                ErrorKind::TypeError,
            ),
            (
                format!("{prelude} (random-source-state-set! s '(squares-state 1.5 1))"),
                ErrorKind::TypeError,
            ),
            (
                format!("{prelude} (random-source-state-set! s '(squares-state -1 1))"),
                ErrorKind::RangeError,
            ),
            (
                format!(
                    "{prelude} (random-source-state-set! s '(squares-state 0 18446744073709551616))"
                ),
                ErrorKind::RangeError,
            ),
            (
                format!("{prelude} (random-source-state-set! s '(squares-state 0 2))"),
                ErrorKind::RangeError,
            ),
        ];
        for (source, expected) in cases {
            assert_eq!(error_kind(&mut engine, &source), expected, "for: {source}");
        }
    }

    #[test]
    fn state_set_rejects_non_sources() {
        let mut engine = engine();
        assert_eq!(
            error_kind(
                &mut engine,
                "(import (srfi 27)) (random-source-state-set! 5 '(squares-state 0 1))"
            ),
            ErrorKind::TypeError
        );
    }

    #[test]
    fn a_source_is_a_disjoint_type() {
        let mut engine = engine();
        assert_eq!(
            show(
                &mut engine,
                r#"
                (import (srfi 27))
                (define s (make-random-source 1))
                (list (random-source? s) (random-source? 5) (random-source? '(1))
                      (procedure? s) (pair? s) (number? s))
                "#,
            ),
            "(#t #f #f #f #f #f)"
        );
    }

    #[test]
    fn a_source_writes_as_an_opaque_object() {
        let mut engine = engine();
        assert_eq!(
            show(&mut engine, "(import (srfi 27)) (make-random-source 1)"),
            "#<random-source>"
        );
    }

    #[test]
    fn generators_share_the_source_state() {
        let mut engine = engine();
        // An integer generator and a real generator over one source advance the
        // same underlying state, so each draw changes what state-ref reports.
        assert_eq!(
            run(
                &mut engine,
                r#"
                (import (srfi 27))
                (define s (make-random-source 4))
                (define g (random-source-make-integers s))
                (define r (random-source-make-reals s))
                (define before (random-source-state-ref s))
                (g 1000000000)
                (define middle (random-source-state-ref s))
                (r)
                (define after (random-source-state-ref s))
                (and (not (equal? before middle)) (not (equal? middle after)))
                "#,
            ),
            Value::boolean(true)
        );
    }

    #[test]
    fn generator_constructors_reject_non_sources() {
        let mut engine = engine();
        assert!(is_error(
            &mut engine,
            "(import (srfi 27)) (random-source-make-integers 5)"
        ));
        assert!(is_error(
            &mut engine,
            "(import (srfi 27)) (random-source-make-reals 5)"
        ));
    }

    #[test]
    fn make_reals_rejects_invalid_units() {
        let mut engine = engine();
        for unit in ["0", "1", "2", "-1/2"] {
            let source = format!(
                "(import (srfi 27)) (random-source-make-reals (make-random-source 1) {unit})"
            );
            assert!(
                is_error(&mut engine, &source),
                "unit {unit} should be rejected"
            );
        }
    }

    #[test]
    fn sources_survive_collection() {
        let mut engine = engine();
        // A live source keeps drawing correctly after churning many throwaway
        // sources through the heap.
        assert_eq!(
            run(
                &mut engine,
                r#"
                (import (srfi 27))
                (define keeper (make-random-source 123))
                (define g (random-source-make-integers keeper))
                (let churn ((i 0))
                  (if (< i 5000)
                      (begin (make-random-source i) (churn (+ i 1)))
                      #t))
                (let check ((i 0) (ok #t))
                  (if (= i 1000)
                      ok
                      (let ((x (g 6)))
                        (check (+ i 1) (and ok (<= 0 x) (< x 6))))))
                "#,
            ),
            Value::boolean(true)
        );
    }
}
