//! Native primitives backing the SRFI 1 (List Library) extension.
//!
//! Only the structural list operations that need no user callback live here. The
//! higher-order procedures of SRFI 1 (fold, filter, find, the map family, ...)
//! call back into a Scheme procedure argument, which a native cannot do, so they
//! are defined in the `(srfi 1)` Scheme wrapper (see `crate::embed::extensions`).
//! What remains native is the pointer-and-structure work where a tight Rust loop
//! or cycle-safe traversal pays off: the prefix/suffix selectors, the last-pair
//! walk, the cycle-aware length and predicates, and the reversing append.
//!
//! Traversals that could meet a circular list use a tortoise and hare and raise
//! (or, for `length+`, report `#f`) rather than spinning, because a native Rust
//! loop never reaches a fuel safe point.

use super::{NativeContext, index, type_error};
use crate::{Error, ErrorKind, Value};

/// The shared error for an operation that reached a non-pair where the list was
/// expected to continue.
fn improper_list(cx: &NativeContext<'_>, value: Value) -> Error {
    type_error("pair", value, cx.heap)
}

/// The shared error for a traversal that detected a circular list.
fn circular_list() -> Error {
    Error::plain(
        ErrorKind::TypeError,
        "expected a proper list, received a circular list",
    )
}

/// Counts the pairs of `list`, following cdrs until the first non-pair. Returns
/// the pair count and the final non-pair tail, or `None` if the chain is
/// circular. A `None` result never allocates and never raises, so each caller
/// decides how to report a cycle.
fn measure(cx: &NativeContext<'_>, list: Value) -> Option<(i64, Value)> {
    let mut hare = list;
    let mut tortoise = list;
    let mut count: i64 = 0;
    loop {
        for _ in 0..2 {
            let Some((_, next)) = cx.heap.pair(hare) else {
                return Some((count, hare));
            };
            hare = next;
            count += 1;
        }
        // The hare advanced two pairs, so step the tortoise one and compare.
        if let Some((_, next)) = cx.heap.pair(tortoise) {
            tortoise = next;
        }
        if tortoise == hare {
            return None;
        }
    }
}

/// Steps `k` pairs into `list` and returns the reached tail. A non-pair before
/// `k` steps raises the same error a `cdr` of that non-pair would. `k` is
/// bounded, so no cycle check is needed.
fn drop_pairs(cx: &NativeContext<'_>, list: Value, k: i64) -> Result<Value, Error> {
    let mut node = list;
    for _ in 0..k {
        let Some((_, next)) = cx.heap.pair(node) else {
            return Err(improper_list(cx, node));
        };
        node = next;
    }
    Ok(node)
}

/// `(xcons d a)`. Exchanged `cons`, building the pair `(a . d)`. Handy when the
/// tail is the first argument, as in a `fold`.
pub(crate) fn xcons(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    cx.pair(a[1], a[0])
}

/// `(cons* elt1 elt2 ... tail)`. Conses the leading elements onto the final
/// argument, which becomes the tail uncopied. With one argument it returns that
/// argument unchanged.
pub(crate) fn cons_star(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let (tail, prefix) = a
        .split_last()
        .expect("cons* is registered with arity of at least one");
    let mut result = *tail;
    for element in prefix.iter().rev() {
        result = cx.pair(*element, result)?;
    }
    Ok(result)
}

/// `(take list i)`. Returns a freshly allocated list of the first `i` elements.
/// It is an error for `list` to have fewer than `i` elements.
pub(crate) fn take(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let count = index(cx, a[1])?;
    // Every collected element stays reachable from the still-rooted input list,
    // so consing the prefix cannot leave a stale reference behind.
    let mut node = a[0];
    // Grow only for pairs that actually exist. Preallocating from an
    // untrusted count can overflow capacity before a short input list reports
    // its ordinary type error.
    let mut elements = Vec::new();
    for _ in 0..count {
        let Some((car, next)) = cx.heap.pair(node) else {
            return Err(improper_list(cx, node));
        };
        elements.push(car);
        node = next;
    }
    let mut result = Value::nil();
    for car in elements.into_iter().rev() {
        result = cx.pair(car, result)?;
    }
    Ok(result)
}

/// `(take-right list i)`. Returns the last `i` pairs of `list`, sharing
/// structure with it. It is an error for `list` to have fewer than `i` elements.
pub(crate) fn take_right(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let count = i64::try_from(index(cx, a[1])?).map_err(|_| length_out_of_range())?;
    let length = measure(cx, a[0]).ok_or_else(circular_list)?.0;
    if count > length {
        return Err(too_few_elements());
    }
    drop_pairs(cx, a[0], length - count)
}

/// `(drop-right list i)`. Returns a freshly allocated list of all but the last
/// `i` elements. It is an error for `list` to have fewer than `i` elements.
pub(crate) fn drop_right(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let count = i64::try_from(index(cx, a[1])?).map_err(|_| length_out_of_range())?;
    let length = measure(cx, a[0]).ok_or_else(circular_list)?.0;
    if count > length {
        return Err(too_few_elements());
    }
    let keep = usize::try_from(length - count).map_err(|_| length_out_of_range())?;
    let mut node = a[0];
    let mut elements = Vec::with_capacity(keep);
    for _ in 0..keep {
        let Some((car, next)) = cx.heap.pair(node) else {
            return Err(improper_list(cx, node));
        };
        elements.push(car);
        node = next;
    }
    let mut result = Value::nil();
    for car in elements.into_iter().rev() {
        result = cx.pair(car, result)?;
    }
    Ok(result)
}

/// `(last-pair list)`. Returns the last pair of the non-empty list `list`.
pub(crate) fn last_pair(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let Some((length, _)) = measure(cx, a[0]) else {
        return Err(circular_list());
    };
    if length == 0 {
        return Err(improper_list(cx, a[0]));
    }
    drop_pairs(cx, a[0], length - 1)
}

/// `(last list)`. Returns the last element of the non-empty list `list`.
pub(crate) fn last(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let pair = last_pair(cx, a)?;
    cx.heap
        .pair(pair)
        .map(|(car, _)| car)
        .ok_or_else(|| improper_list(cx, pair))
}

/// `(length+ clist)`. Returns the number of pairs in `clist`, or `#f` when it is
/// circular. Unlike `length`, a dotted tail is not an error: the pair count is
/// returned.
pub(crate) fn length_plus(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    match measure(cx, a[0]) {
        Some((count, _)) => Ok(Value::integer(count)),
        None => Ok(Value::boolean(false)),
    }
}

/// `(append-reverse rev-head tail)`. Reverses `rev-head` onto the front of
/// `tail` in a single pass, equivalent to `(append (reverse rev-head) tail)`.
pub(crate) fn append_reverse(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let mut node = a[0];
    let mut result = a[1];
    let mut hare = a[0];
    let mut step = false;
    while let Some((car, next)) = cx.heap.pair(node) {
        // The remaining input and the growing result are both rooted through the
        // register view, so an intervening collection keeps every operand live.
        result = cx.pair(car, result)?;
        node = next;
        if step {
            if let Some((_, hare_next)) = cx.heap.pair(hare) {
                hare = hare_next;
            }
            if hare == node {
                return Err(circular_list());
            }
        }
        step = !step;
    }
    if node != Value::nil() {
        return Err(improper_list(cx, node));
    }
    Ok(result)
}

/// `(circular-list? x)`. True only when following cdrs from `x` re-enters a pair
/// already visited.
pub(crate) fn circular_list_p(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    Ok(Value::boolean(measure(cx, a[0]).is_none()))
}

/// `(dotted-list? x)`. True when `x` is neither a proper list (nil-terminated)
/// nor circular, so its final tail is a non-nil, non-pair value. A non-pair atom
/// is itself a dotted list of length zero.
pub(crate) fn dotted_list_p(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let dotted = match measure(cx, a[0]) {
        Some((_, tail)) => tail != Value::nil(),
        None => false,
    };
    Ok(Value::boolean(dotted))
}

/// The error raised when a `take-right`/`drop-right` count exceeds the list.
fn too_few_elements() -> Error {
    Error::plain(
        ErrorKind::RangeError,
        "count exceeds the number of elements in the list",
    )
}

/// The error raised when a length or index does not fit the machine word.
fn length_out_of_range() -> Error {
    Error::plain(
        ErrorKind::RangeError,
        "list index must be a non-negative exact integer in range",
    )
}

#[cfg(test)]
mod tests {
    use crate::{Engine, EngineConfig, ErrorKind, Extension};

    fn engine() -> Engine {
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        engine.install_extension(Extension::Srfi1).unwrap();
        engine
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

    #[test]
    fn take_and_drop_split_a_list() {
        let mut engine = engine();
        assert_eq!(
            show(&mut engine, "(import (srfi 1)) (take '(a b c d e) 2)"),
            "(a b)"
        );
        assert_eq!(
            show(&mut engine, "(import (srfi 1)) (drop '(a b c d e) 2)"),
            "(c d e)"
        );
        assert_eq!(
            show(&mut engine, "(import (srfi 1)) (take-right '(a b c d e) 2)"),
            "(d e)"
        );
        assert_eq!(
            show(&mut engine, "(import (srfi 1)) (drop-right '(a b c d e) 2)"),
            "(a b c)"
        );
    }

    #[test]
    fn take_rejects_a_short_list() {
        let mut engine = engine();
        assert_eq!(
            error_kind(&mut engine, "(import (srfi 1)) (take '(a b) 5)"),
            ErrorKind::TypeError
        );
        assert_eq!(
            error_kind(&mut engine, "(import (srfi 1)) (take-right '(a b) 5)"),
            ErrorKind::RangeError
        );
    }

    #[test]
    fn last_and_last_pair() {
        let mut engine = engine();
        assert_eq!(show(&mut engine, "(import (srfi 1)) (last '(a b c))"), "c");
        assert_eq!(
            show(&mut engine, "(import (srfi 1)) (last-pair '(a b c))"),
            "(c)"
        );
        assert_eq!(
            error_kind(&mut engine, "(import (srfi 1)) (last '())"),
            ErrorKind::TypeError
        );
    }

    #[test]
    fn last_and_last_pair_reject_circular_lists() {
        let mut engine = engine();
        for procedure in ["last", "last-pair"] {
            let source = format!(
                r#"
                (import (srfi 1) (scheme base))
                (define ring (list 'a 'b 'c))
                (set-cdr! (cddr ring) ring)
                ({procedure} ring)
                "#
            );
            assert_eq!(error_kind(&mut engine, &source), ErrorKind::TypeError);
        }
    }

    #[test]
    fn cons_star_and_xcons_build_tails() {
        let mut engine = engine();
        assert_eq!(
            show(&mut engine, "(import (srfi 1)) (cons* 1 2 3 '(4 5))"),
            "(1 2 3 4 5)"
        );
        assert_eq!(show(&mut engine, "(import (srfi 1)) (cons* 7)"), "7");
        assert_eq!(
            show(&mut engine, "(import (srfi 1)) (xcons '(b c) 'a)"),
            "(a b c)"
        );
    }

    #[test]
    fn append_reverse_reverses_onto_a_tail() {
        let mut engine = engine();
        assert_eq!(
            show(
                &mut engine,
                "(import (srfi 1)) (append-reverse '(3 2 1) '(4 5))"
            ),
            "(1 2 3 4 5)"
        );
    }

    #[test]
    fn length_plus_reports_circular_lists() {
        let mut engine = engine();
        assert_eq!(
            show(&mut engine, "(import (srfi 1)) (length+ '(a b c))"),
            "3"
        );
        // A circular list yields #f rather than looping until fuel runs out.
        assert_eq!(
            show(
                &mut engine,
                r#"
                (import (srfi 1) (scheme base))
                (define ring (list 'a 'b 'c))
                (set-cdr! (cddr ring) ring)
                (length+ ring)
                "#,
            ),
            "#f"
        );
    }

    #[test]
    fn structural_predicates_classify_tails() {
        let mut engine = engine();
        assert_eq!(
            show(
                &mut engine,
                r#"
                (import (srfi 1) (scheme base))
                (define ring (list 1 2 3))
                (set-cdr! (cddr ring) ring)
                (list (circular-list? ring)
                      (circular-list? '(1 2 3))
                      (dotted-list? '(1 2 . 3))
                      (dotted-list? '(1 2 3))
                      (dotted-list? 7)
                      (proper-list? '(1 2 3))
                      (not-pair? 7)
                      (null-list? '()))
                "#,
            ),
            "(#t #f #t #f #t #t #t #t)"
        );
    }
}
