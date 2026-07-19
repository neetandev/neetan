//! Native primitives backing the SRFI 152 (String Library) extension.
//!
//! Only the representation-dependent primitives that take no Scheme callback
//! live here: the prefix and suffix scans, the substring search, the bulk
//! concatenation, and the replicated substring. Every procedure that accepts a
//! predicate or mapper (`string-every`, `string-index`, `string-fold`,
//! `string-map`, and the rest) lives in the `(srfi 152)` Scheme wrapper, because
//! a native cannot call back into a Scheme argument. The procedures that
//! R7RS-small already provides are re-exported by the wrapper unchanged.
//!
//! The scans work over `Vec<char>` snapshots of the operand ranges. The SRFI
//! makes no performance guarantee and is intended for short strings, so the
//! straightforward char-indexed algorithms are used rather than byte-level
//! search.

use super::{NativeContext, character, index, type_error};
use crate::{Error, ErrorKind, Value};

/// A resolved half-open `[start, end)` char range into a string.
type Range = (usize, usize);

/// The error raised when an index or bound falls outside a string.
fn range_error(message: &str) -> Error {
    Error::plain(ErrorKind::RangeError, message.to_owned())
}

/// The error raised when a result string would exceed the heap limit.
fn too_large() -> Error {
    Error::plain(ErrorKind::HeapLimitExceeded, "string is too large")
}

/// Snapshots a string argument as a vector of its characters, type-checking it.
fn chars(cx: &NativeContext<'_>, value: Value) -> Result<Vec<char>, Error> {
    cx.heap
        .string_slice(value)
        .map(|text| text.chars().collect())
        .ok_or_else(|| type_error("string", value, cx.heap))
}

/// Resolves optional `[start end]` bounds against a length. SRFI 152 requires
/// `0 <= start <= end <= length`, so a reversed or overshooting range is an
/// error here rather than the lenient empty range used elsewhere in the engine.
fn bounds(cx: &NativeContext<'_>, rest: &[Value], length: usize) -> Result<Range, Error> {
    let start = match rest.first() {
        Some(value) => index(cx, *value)?,
        None => 0,
    };
    let end = match rest.get(1) {
        Some(value) => index(cx, *value)?,
        None => length,
    };
    if start > end || end > length {
        return Err(range_error("string index range is out of bounds"));
    }
    Ok((start, end))
}

/// Resolves the optional `[start1 end1 start2 end2]` prefix of arguments shared
/// by the two-string procedures. Any prefix of the four may be supplied.
fn two_ranges(
    cx: &NativeContext<'_>,
    rest: &[Value],
    length1: usize,
    length2: usize,
) -> Result<(Range, Range), Error> {
    let (first, second) = rest.split_at(rest.len().min(2));
    let range1 = bounds(cx, first, length1)?;
    let range2 = bounds(cx, second, length2)?;
    Ok((range1, range2))
}

pub(crate) fn string_null_p(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let length = cx
        .heap
        .string_len(a[0])
        .ok_or_else(|| type_error("string", a[0], cx.heap))?;
    Ok(Value::boolean(length == 0))
}

pub(crate) fn reverse_list_to_string(
    cx: &mut NativeContext<'_>,
    a: &[Value],
) -> Result<Value, Error> {
    let mut collected = Vec::new();
    let mut node = a[0];
    while let Some((car, cdr)) = cx.heap.pair(node) {
        collected.push(character(cx, car)?);
        node = cdr;
    }
    if node != Value::nil() {
        return Err(type_error("list", a[0], cx.heap));
    }
    collected.reverse();
    cx.string(collected)
}

pub(crate) fn string_prefix_length(
    cx: &mut NativeContext<'_>,
    a: &[Value],
) -> Result<Value, Error> {
    let s1 = chars(cx, a[0])?;
    let s2 = chars(cx, a[1])?;
    let ((start1, end1), (start2, end2)) = two_ranges(cx, &a[2..], s1.len(), s2.len())?;
    let mut n = 0;
    while start1 + n < end1 && start2 + n < end2 && s1[start1 + n] == s2[start2 + n] {
        n += 1;
    }
    Ok(Value::integer(n as i64))
}

pub(crate) fn string_suffix_length(
    cx: &mut NativeContext<'_>,
    a: &[Value],
) -> Result<Value, Error> {
    let s1 = chars(cx, a[0])?;
    let s2 = chars(cx, a[1])?;
    let ((start1, end1), (start2, end2)) = two_ranges(cx, &a[2..], s1.len(), s2.len())?;
    let max = (end1 - start1).min(end2 - start2);
    let mut n = 0;
    while n < max && s1[end1 - 1 - n] == s2[end2 - 1 - n] {
        n += 1;
    }
    Ok(Value::integer(n as i64))
}

pub(crate) fn string_prefix_p(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let s1 = chars(cx, a[0])?;
    let s2 = chars(cx, a[1])?;
    let ((start1, end1), (start2, end2)) = two_ranges(cx, &a[2..], s1.len(), s2.len())?;
    let mut n = 0;
    while start1 + n < end1 && start2 + n < end2 && s1[start1 + n] == s2[start2 + n] {
        n += 1;
    }
    Ok(Value::boolean(n == end1 - start1))
}

pub(crate) fn string_suffix_p(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let s1 = chars(cx, a[0])?;
    let s2 = chars(cx, a[1])?;
    let ((start1, end1), (start2, end2)) = two_ranges(cx, &a[2..], s1.len(), s2.len())?;
    let max = (end1 - start1).min(end2 - start2);
    let mut n = 0;
    while n < max && s1[end1 - 1 - n] == s2[end2 - 1 - n] {
        n += 1;
    }
    Ok(Value::boolean(n == end1 - start1))
}

pub(crate) fn string_contains(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let s1 = chars(cx, a[0])?;
    let s2 = chars(cx, a[1])?;
    let ((start1, end1), (start2, end2)) = two_ranges(cx, &a[2..], s1.len(), s2.len())?;
    let haystack = &s1[start1..end1];
    let needle = &s2[start2..end2];
    // An empty needle matches at the start of the search range.
    if needle.is_empty() {
        return Ok(Value::integer(start1 as i64));
    }
    if needle.len() <= haystack.len() {
        for offset in 0..=haystack.len() - needle.len() {
            if haystack[offset..offset + needle.len()] == *needle {
                return Ok(Value::integer((start1 + offset) as i64));
            }
        }
    }
    Ok(Value::boolean(false))
}

pub(crate) fn string_contains_right(
    cx: &mut NativeContext<'_>,
    a: &[Value],
) -> Result<Value, Error> {
    let s1 = chars(cx, a[0])?;
    let s2 = chars(cx, a[1])?;
    let ((start1, end1), (start2, end2)) = two_ranges(cx, &a[2..], s1.len(), s2.len())?;
    let haystack = &s1[start1..end1];
    let needle = &s2[start2..end2];
    // An empty needle matches at the end of the search range.
    if needle.is_empty() {
        return Ok(Value::integer(end1 as i64));
    }
    if needle.len() <= haystack.len() {
        for offset in (0..=haystack.len() - needle.len()).rev() {
            if haystack[offset..offset + needle.len()] == *needle {
                return Ok(Value::integer((start1 + offset) as i64));
            }
        }
    }
    Ok(Value::boolean(false))
}

pub(crate) fn string_concatenate(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let mut parts = Vec::new();
    let mut total_bytes = 0usize;
    let mut total_chars = 0usize;
    let mut node = a[0];
    while let Some((car, cdr)) = cx.heap.pair(node) {
        let (bytes, count) = cx
            .heap
            .string_dimensions(car)
            .ok_or_else(|| type_error("string", car, cx.heap))?;
        total_bytes = total_bytes.checked_add(bytes).ok_or_else(too_large)?;
        total_chars += count;
        parts.push(car);
        node = cdr;
    }
    if node != Value::nil() {
        return Err(type_error("list", a[0], cx.heap));
    }
    // Every part was type-checked above, so the spans concatenate directly.
    cx.string_concat(&parts, total_bytes, total_chars)
}

pub(crate) fn string_replicate(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let source = chars(cx, a[0])?;
    // `from` and `to` index the replicated space and may be negative, so they
    // are read as signed integers rather than through the non-negative helper.
    let from = cx.to_i128(a[1])?;
    let to = cx.to_i128(a[2])?;
    let (start, end) = bounds(cx, &a[3..], source.len())?;
    if from > to {
        return Err(range_error("string-replicate: from is greater than to"));
    }
    let span = end - start;
    if span == 0 {
        // A zero-length window can only produce a zero-length result.
        if from == to {
            return cx.string(std::iter::empty::<char>());
        }
        return Err(range_error(
            "string-replicate: empty substring cannot be replicated",
        ));
    }
    let out_len = usize::try_from(to - from).map_err(|_| too_large())?;
    let mut out: Vec<char> = Vec::new();
    out.try_reserve(out_len).map_err(|_| too_large())?;
    let span = span as i128;
    let mut k = from;
    while k < to {
        let offset = k.rem_euclid(span) as usize;
        out.push(source[start + offset]);
        k += 1;
    }
    cx.string(out)
}

#[cfg(test)]
mod tests {
    use crate::{Engine, EngineConfig, ErrorKind, Extension, Value};

    fn engine() -> Engine {
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        engine.install_extension(Extension::Srfi152).unwrap();
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

    #[test]
    fn prefix_and_suffix_scans_follow_the_spec() {
        let mut engine = engine();
        assert_eq!(
            run(
                &mut engine,
                r#"
                (import (srfi 152) (scheme base))
                (and (= (string-prefix-length "cool" "court") 2)
                     (= (string-suffix-length "place" "space") 3)
                     (string-prefix? "abc" "abcdef")
                     (not (string-prefix? "abz" "abcdef"))
                     (string-suffix? "def" "abcdef")
                     (not (string-suffix? "xef" "abcdef")))
                "#,
            ),
            Value::boolean(true)
        );
    }

    #[test]
    fn contains_returns_the_match_index_or_false() {
        let mut engine = engine();
        // The spec example searches the substring "a geek" of "eek -- what a geek."
        assert_eq!(
            run(
                &mut engine,
                r#"
                (import (srfi 152) (scheme base))
                (string-contains "eek -- what a geek." "ee" 12 18)
                "#,
            ),
            Value::integer(15)
        );
        assert_eq!(
            run(
                &mut engine,
                r#"
                (import (srfi 152) (scheme base))
                (string-contains "abcdef" "xyz")
                "#,
            ),
            Value::boolean(false)
        );
        // The rightmost match wins for the -right variant.
        assert_eq!(
            run(
                &mut engine,
                r#"
                (import (srfi 152) (scheme base))
                (string-contains-right "abcabc" "bc")
                "#,
            ),
            Value::integer(4)
        );
    }

    #[test]
    fn concatenate_and_replicate_build_new_strings() {
        let mut engine = engine();
        assert_eq!(
            show(
                &mut engine,
                r#"
                (import (srfi 152) (scheme base))
                (string-concatenate '("foo" "bar" "baz"))
                "#,
            ),
            "\"foobarbaz\""
        );
        // Rotate left, rotate right, and replicate, from the spec examples.
        assert_eq!(
            show(
                &mut engine,
                r#"
                (import (srfi 152) (scheme base))
                (list (string-replicate "abcdef" 2 8)
                      (string-replicate "abcdef" -2 4)
                      (string-replicate "abc" 0 7))
                "#,
            ),
            "(\"cdefab\" \"efabcd\" \"abcabca\")"
        );
    }

    #[test]
    fn reverse_list_to_string_matches_the_idiom() {
        let mut engine = engine();
        assert_eq!(
            show(
                &mut engine,
                r#"
                (import (srfi 152) (scheme base))
                (reverse-list->string '(#\a #\B #\c))
                "#,
            ),
            "\"cBa\""
        );
    }

    #[test]
    fn an_out_of_range_bound_raises() {
        let mut engine = engine();
        assert_eq!(
            error_kind(
                &mut engine,
                r#"
                (import (srfi 152) (scheme base))
                (string-prefix-length "abc" "abcdef" 0 9)
                "#,
            ),
            ErrorKind::RangeError
        );
    }
}
