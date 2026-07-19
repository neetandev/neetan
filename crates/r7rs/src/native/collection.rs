//! Pair, list, vector, string, and bytevector procedures, plus the
//! string/symbol/UTF-8 conversions.

use super::{
    scalar::{equal_value, eqv_value},
    *,
};

pub(super) fn cons(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    cx.pair(a[0], a[1])
}

/// Deep-freezes a hoisted literal's object graph and returns the value, so
/// mutating an R7RS constant raises instead of succeeding. Registered as the
/// internal `%literal` native and emitted only by the expander around hoisted
/// literal initializers. The walk tracks visited slots because the native is
/// an ordinary global, so user code can reach it with shared or cyclic data.
pub(super) fn literal_freeze(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let mut pending = vec![a[0]];
    let mut visited = std::collections::HashSet::new();
    while let Some(value) = pending.pop() {
        let Some(reference) = value.heap_ref() else {
            continue;
        };
        if !visited.insert(reference) {
            continue;
        }
        match cx.heap.kind(value) {
            ValueKind::Pair => {
                if let Some((car, cdr)) = cx.heap.pair(value) {
                    pending.push(car);
                    pending.push(cdr);
                }
                cx.heap.make_immutable(value);
            }
            ValueKind::Vector => {
                if let Some(values) = cx.heap.vector_slice(value) {
                    pending.extend_from_slice(values);
                }
                cx.heap.make_immutable(value);
            }
            ValueKind::String | ValueKind::Bytevector => cx.heap.make_immutable(value),
            // Symbols are interned and already immutable. Everything else
            // cannot appear inside a literal datum and is left untouched.
            _ => {}
        }
    }
    Ok(a[0])
}

pub(super) fn car(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    cx.heap
        .pair(a[0])
        .map(|pair| pair.0)
        .ok_or_else(|| type_error("pair", a[0], cx.heap))
}

pub(super) fn cdr(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    cx.heap
        .pair(a[0])
        .map(|pair| pair.1)
        .ok_or_else(|| type_error("pair", a[0], cx.heap))
}

fn car_of(cx: &NativeContext<'_>, value: Value) -> Result<Value, Error> {
    cx.heap
        .pair(value)
        .map(|pair| pair.0)
        .ok_or_else(|| type_error("pair", value, cx.heap))
}

fn cdr_of(cx: &NativeContext<'_>, value: Value) -> Result<Value, Error> {
    cx.heap
        .pair(value)
        .map(|pair| pair.1)
        .ok_or_else(|| type_error("pair", value, cx.heap))
}

// Each cxr accessor composes car/cdr steps. The letters between the leading `c`
// and trailing `r` name the steps from outer to inner, so they are applied in
// reverse: the rightmost letter acts on the argument first. A non-pair at any
// step raises the same pair type error the plain car or cdr would.
macro_rules! cxr {
    ($name:ident, $($step:ident),+) => {
        pub(super) fn $name(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
            let steps: &[fn(&NativeContext<'_>, Value) -> Result<Value, Error>] = &[$($step),+];
            let mut value = a[0];
            for step in steps.iter().rev() {
                value = step(cx, value)?;
            }
            Ok(value)
        }
    };
}

cxr!(caar, car_of, car_of);
cxr!(cadr, car_of, cdr_of);
cxr!(cdar, cdr_of, car_of);
cxr!(cddr, cdr_of, cdr_of);
cxr!(caaar, car_of, car_of, car_of);
cxr!(caadr, car_of, car_of, cdr_of);
cxr!(cadar, car_of, cdr_of, car_of);
cxr!(caddr, car_of, cdr_of, cdr_of);
cxr!(cdaar, cdr_of, car_of, car_of);
cxr!(cdadr, cdr_of, car_of, cdr_of);
cxr!(cddar, cdr_of, cdr_of, car_of);
cxr!(cdddr, cdr_of, cdr_of, cdr_of);
cxr!(caaaar, car_of, car_of, car_of, car_of);
cxr!(caaadr, car_of, car_of, car_of, cdr_of);
cxr!(caadar, car_of, car_of, cdr_of, car_of);
cxr!(caaddr, car_of, car_of, cdr_of, cdr_of);
cxr!(cadaar, car_of, cdr_of, car_of, car_of);
cxr!(cadadr, car_of, cdr_of, car_of, cdr_of);
cxr!(caddar, car_of, cdr_of, cdr_of, car_of);
cxr!(cadddr, car_of, cdr_of, cdr_of, cdr_of);
cxr!(cdaaar, cdr_of, car_of, car_of, car_of);
cxr!(cdaadr, cdr_of, car_of, car_of, cdr_of);
cxr!(cdadar, cdr_of, car_of, cdr_of, car_of);
cxr!(cdaddr, cdr_of, car_of, cdr_of, cdr_of);
cxr!(cddaar, cdr_of, cdr_of, car_of, car_of);
cxr!(cddadr, cdr_of, cdr_of, car_of, cdr_of);
cxr!(cdddar, cdr_of, cdr_of, cdr_of, car_of);
cxr!(cddddr, cdr_of, cdr_of, cdr_of, cdr_of);

pub(super) fn set_car(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    if cx.heap.set_pair_car(a[0], a[1]) {
        Ok(Value::unspecified())
    } else {
        Err(pair_mutation_error(cx, a[0]))
    }
}

pub(super) fn set_cdr(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    if cx.heap.set_pair_cdr(a[0], a[1]) {
        Ok(Value::unspecified())
    } else {
        Err(pair_mutation_error(cx, a[0]))
    }
}

pub(super) fn list(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let mut result = Value::nil();
    for value in a.iter().rev() {
        result = cx.pair(*value, result)?;
    }
    Ok(result)
}

pub(super) fn list_predicate(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let mut slow = a[0];
    let mut fast = a[0];
    loop {
        match cx.heap.pair(fast) {
            None if fast == Value::nil() => return bool_value(true),
            None => return bool_value(false),
            Some((_, next)) => fast = next,
        };
        match cx.heap.pair(fast) {
            None if fast == Value::nil() => return bool_value(true),
            None => return bool_value(false),
            Some((_, next)) => fast = next,
        };
        slow = match cx.heap.pair(slow) {
            Some((_, next)) => next,
            None => return bool_value(false),
        };
        if slow == fast {
            return bool_value(false);
        }
    }
}

// The Scheme definitions these natives replace looped until fuel ran out on a
// circular list. A native Rust loop never reaches a fuel safe point, so the
// traversals below detect a cycle with a tortoise and hare and raise instead of
// spinning forever.
fn cycle_error() -> Error {
    Error::plain(
        ErrorKind::TypeError,
        "expected a proper list, received a circular list",
    )
}

// Walks a chain of pairs, collecting the car of each, and stops at the first
// non-pair tail. Returns the collected elements and that tail. Raises only on a
// circular chain.
fn collect_list(cx: &NativeContext<'_>, value: Value) -> Result<(Vec<Value>, Value), Error> {
    let mut elements = Vec::new();
    let mut hare = value;
    let mut tortoise = value;
    let mut step = false;
    loop {
        let Some((car, next)) = cx.heap.pair(hare) else {
            return Ok((elements, hare));
        };
        elements.push(car);
        hare = next;
        if step {
            if let Some((_, tortoise_next)) = cx.heap.pair(tortoise) {
                tortoise = tortoise_next;
            }
            if tortoise == hare {
                return Err(cycle_error());
            }
        }
        step = !step;
    }
}

pub(super) fn list_length(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let mut hare = a[0];
    let mut tortoise = a[0];
    let mut count: i64 = 0;
    loop {
        for _ in 0..2 {
            let Some((_, next)) = cx.heap.pair(hare) else {
                if hare == Value::nil() {
                    return Ok(Value::integer(count));
                }
                return Err(type_error("pair", hare, cx.heap));
            };
            hare = next;
            count += 1;
        }
        if let Some((_, tortoise_next)) = cx.heap.pair(tortoise) {
            tortoise = tortoise_next;
        }
        if tortoise == hare {
            return Err(cycle_error());
        }
    }
}

pub(super) fn list_reverse(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let mut hare = a[0];
    let mut tortoise = a[0];
    let mut result = Value::nil();
    loop {
        for _ in 0..2 {
            let Some((car, next)) = cx.heap.pair(hare) else {
                if hare == Value::nil() {
                    return Ok(result);
                }
                return Err(type_error("pair", hare, cx.heap));
            };
            // The input stays rooted through the VM register view and `result`
            // is rooted as each pair is allocated, so an intervening collection
            // keeps every operand live.
            result = cx.pair(car, result)?;
            hare = next;
        }
        if let Some((_, tortoise_next)) = cx.heap.pair(tortoise) {
            tortoise = tortoise_next;
        }
        if tortoise == hare {
            return Err(cycle_error());
        }
    }
}

pub(super) fn list_append(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let Some((last, prefix)) = a.split_last() else {
        return Ok(Value::nil());
    };
    // Every argument but the last must be a proper list and is copied. The last
    // argument becomes the shared tail and is returned uncopied, so it may be any
    // object including an improper tail.
    let mut elements: Vec<Value> = Vec::new();
    for list in prefix {
        let (chunk, tail) = collect_list(cx, *list)?;
        if tail != Value::nil() {
            return Err(type_error("pair", tail, cx.heap));
        }
        elements.extend(chunk);
    }
    let mut result = *last;
    for car in elements.into_iter().rev() {
        result = cx.pair(car, result)?;
    }
    Ok(result)
}

pub(super) fn list_copy(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let (elements, tail) = collect_list(cx, a[0])?;
    let mut result = tail;
    for car in elements.into_iter().rev() {
        result = cx.pair(car, result)?;
    }
    Ok(result)
}

// Steps `k` pairs into a chain. `k` is bounded, so no cycle check is needed. A
// non-pair before `k` steps raises the same error the underlying `cdr` would.
fn list_tail_value(cx: &NativeContext<'_>, list: Value, k: usize) -> Result<Value, Error> {
    let mut value = list;
    for _ in 0..k {
        let Some((_, next)) = cx.heap.pair(value) else {
            return Err(type_error("pair", value, cx.heap));
        };
        value = next;
    }
    Ok(value)
}

pub(super) fn list_tail(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let k = index(cx, a[1])?;
    list_tail_value(cx, a[0], k)
}

pub(super) fn list_ref(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let k = index(cx, a[1])?;
    let tail = list_tail_value(cx, a[0], k)?;
    car_of(cx, tail)
}

pub(super) fn list_set(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let k = index(cx, a[1])?;
    let tail = list_tail_value(cx, a[0], k)?;
    if cx.heap.set_pair_car(tail, a[2]) {
        Ok(Value::unspecified())
    } else {
        Err(pair_mutation_error(cx, tail))
    }
}

pub(super) fn make_list(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let count = index(cx, a[0])?;
    let fill = a.get(1).copied().unwrap_or(Value::boolean(false));
    let mut result = Value::nil();
    for _ in 0..count {
        result = cx.pair(fill, result)?;
    }
    Ok(result)
}

// Scans a list for the first element equal to `object` under `same`, returning
// the matching sublist or #f. A non-pair, non-nil tail raises the same pair type
// error the old `(car list)` step would, and a circular list is detected rather
// than spun on forever.
fn member_scan(
    cx: &NativeContext<'_>,
    object: Value,
    list: Value,
    same: fn(&Heap, Value, Value) -> bool,
) -> Result<Value, Error> {
    let mut hare = list;
    let mut tortoise = list;
    let mut step = false;
    loop {
        let Some((car, next)) = cx.heap.pair(hare) else {
            if hare == Value::nil() {
                return Ok(Value::boolean(false));
            }
            return Err(type_error("pair", hare, cx.heap));
        };
        if same(cx.heap, object, car) {
            return Ok(hare);
        }
        hare = next;
        if step {
            if let Some((_, tortoise_next)) = cx.heap.pair(tortoise) {
                tortoise = tortoise_next;
            }
            if tortoise == hare {
                return Err(cycle_error());
            }
        }
        step = !step;
    }
}

// Scans an association list for the first entry whose key is equal to `object`
// under `same`, returning that entry or #f. Each entry must be a pair, matching
// the old `(car (car alist))` step.
fn assoc_scan(
    cx: &NativeContext<'_>,
    object: Value,
    alist: Value,
    same: fn(&Heap, Value, Value) -> bool,
) -> Result<Value, Error> {
    let mut hare = alist;
    let mut tortoise = alist;
    let mut step = false;
    loop {
        let Some((entry, next)) = cx.heap.pair(hare) else {
            if hare == Value::nil() {
                return Ok(Value::boolean(false));
            }
            return Err(type_error("pair", hare, cx.heap));
        };
        let key = car_of(cx, entry)?;
        if same(cx.heap, object, key) {
            return Ok(entry);
        }
        hare = next;
        if step {
            if let Some((_, tortoise_next)) = cx.heap.pair(tortoise) {
                tortoise = tortoise_next;
            }
            if tortoise == hare {
                return Err(cycle_error());
            }
        }
        step = !step;
    }
}

/// Upper bound on the pairs a fast-path list scan may visit before deferring
/// to the canonical native. The bound keeps the fast arm short between safe
/// points and makes cycle handling fall out: a circular list always exhausts
/// the bound, and the canonical scan raises the cycle error.
pub(super) const FAST_LIST_SCAN_BOUND: usize = 128;

// Bounded member scan for the native fast path. Writes the matching sublist,
// or #f on a proper-list miss, into `out`. Any other shape (improper tail or
// bound exhaustion) defers to `member_scan`, which raises the canonical error.
// `#[inline(never)]` keeps the loop body out of the VM dispatch caller, and
// the out-parameter keeps the return in registers (see `FastProcedure::invoke`).
#[inline(never)]
pub(super) fn fast_member_scan(
    heap: &Heap,
    object: Value,
    list: Value,
    same: fn(&Heap, Value, Value) -> bool,
    out: &mut Value,
) -> bool {
    let mut current = list;
    for _ in 0..FAST_LIST_SCAN_BOUND {
        let Some((car, next)) = heap.pair(current) else {
            if current == Value::nil() {
                *out = Value::boolean(false);
                return true;
            }
            return false;
        };
        if same(heap, object, car) {
            *out = current;
            return true;
        }
        current = next;
    }
    false
}

// Bounded assoc scan for the native fast path. Writes the matching entry, or
// #f on a proper-list miss, into `out`. A non-pair entry, an improper tail,
// or bound exhaustion defers to `assoc_scan` for the canonical error.
#[inline(never)]
pub(super) fn fast_assoc_scan(
    heap: &Heap,
    object: Value,
    alist: Value,
    same: fn(&Heap, Value, Value) -> bool,
    out: &mut Value,
) -> bool {
    let mut current = alist;
    for _ in 0..FAST_LIST_SCAN_BOUND {
        let Some((entry, next)) = heap.pair(current) else {
            if current == Value::nil() {
                *out = Value::boolean(false);
                return true;
            }
            return false;
        };
        let Some((key, _)) = heap.pair(entry) else {
            return false;
        };
        if same(heap, object, key) {
            *out = entry;
            return true;
        }
        current = next;
    }
    false
}

// Bounded length count for the native fast path. Defers past the bound, so a
// circular list always reaches the canonical scan and its cycle error.
#[inline(never)]
pub(super) fn fast_length(heap: &Heap, list: Value, out: &mut Value) -> bool {
    let mut current = list;
    for count in 0..FAST_LIST_SCAN_BOUND {
        let Some((_, next)) = heap.pair(current) else {
            if current == Value::nil() {
                *out = Value::integer(count as i64);
                return true;
            }
            return false;
        };
        current = next;
    }
    false
}

// Bounded list-tail step for the native fast path. Defers on a non-fixnum or
// negative count and on a non-pair before `k` steps, so the canonical native
// raises its index or pair error. The value after `k` steps is written even
// when it is not a pair, matching `list_tail_value`.
#[inline(never)]
pub(super) fn fast_list_tail(heap: &Heap, list: Value, count: Value, out: &mut Value) -> bool {
    let Some(k) = count.as_fixnum().and_then(|k| usize::try_from(k).ok()) else {
        return false;
    };
    if k > FAST_LIST_SCAN_BOUND {
        return false;
    }
    let mut current = list;
    for _ in 0..k {
        let Some((_, next)) = heap.pair(current) else {
            return false;
        };
        current = next;
    }
    *out = current;
    true
}

// Bounded list-ref for the native fast path: the list-tail walk plus a final
// car, deferring on the same shapes.
#[inline(never)]
pub(super) fn fast_list_ref(heap: &Heap, list: Value, count: Value, out: &mut Value) -> bool {
    let Some(k) = count.as_fixnum().and_then(|k| usize::try_from(k).ok()) else {
        return false;
    };
    if k > FAST_LIST_SCAN_BOUND {
        return false;
    }
    let mut current = list;
    for _ in 0..k {
        let Some((_, next)) = heap.pair(current) else {
            return false;
        };
        current = next;
    }
    let Some((car, _)) = heap.pair(current) else {
        return false;
    };
    *out = car;
    true
}

// Bounded reverse for the native fast path. Builds the reversed chain with
// plain allocations: while the VM is active a collection defers to the next
// safe point, so the unrooted partial chain stays live by construction and the
// result lands in a register before that point. An allocation error, a
// non-proper shape, or bound exhaustion defers to the canonical native, which
// runs under the rooted region and raises the canonical error. An abandoned
// partial chain is ordinary garbage.
#[inline(never)]
pub(super) fn fast_reverse(heap: &mut Heap, list: Value, out: &mut Value) -> bool {
    let mut current = list;
    let mut result = Value::nil();
    for _ in 0..FAST_LIST_SCAN_BOUND {
        let Some((car, next)) = heap.pair(current) else {
            if current == Value::nil() {
                *out = result;
                return true;
            }
            return false;
        };
        let Ok(pair) = heap.alloc_pair(car, result) else {
            return false;
        };
        result = pair;
        current = next;
    }
    false
}

// Bounded two-argument append for the native fast path. The first list's cars
// are collected into a stack buffer and its tail must be nil. The result is
// then built in reverse onto the second argument uncopied, which preserves the
// R7RS tail sharing of the canonical append. The second argument may be any
// object, matching the canonical last-argument rule. Longer first lists, other
// arities, and allocation errors defer.
#[inline(never)]
pub(super) fn fast_append_two(
    heap: &mut Heap,
    first: Value,
    second: Value,
    out: &mut Value,
) -> bool {
    let mut buffer = [Value::nil(); FAST_LIST_SCAN_BOUND];
    let mut length = 0;
    let mut current = first;
    loop {
        let Some((car, next)) = heap.pair(current) else {
            if current == Value::nil() {
                break;
            }
            return false;
        };
        if length == FAST_LIST_SCAN_BOUND {
            return false;
        }
        buffer[length] = car;
        length += 1;
        current = next;
    }
    let mut result = second;
    for index in (0..length).rev() {
        let Ok(pair) = heap.alloc_pair(buffer[index], result) else {
            return false;
        };
        result = pair;
    }
    *out = result;
    true
}

// eq? and eqv? are the same operation in this engine, so memq and memv share one
// native, as do assq and assv.
pub(super) fn member_by_eqv(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    member_scan(cx, a[0], a[1], eqv_value)
}

pub(super) fn member_by_equal(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    member_scan(cx, a[0], a[1], equal_value)
}

pub(super) fn assoc_by_eqv(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    assoc_scan(cx, a[0], a[1], eqv_value)
}

pub(super) fn assoc_by_equal(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    assoc_scan(cx, a[0], a[1], equal_value)
}

fn out_of_range() -> Error {
    Error::plain(ErrorKind::RangeError, "index is outside the sequence")
}

pub(super) fn immutable_error(expected: &str) -> Error {
    Error::plain(
        ErrorKind::RuntimeError,
        format!("cannot mutate an immutable {expected}"),
    )
}

/// Explains a failed pair mutation: an immutable pair (a literal) gets the
/// immutability error, anything else the usual type error.
fn pair_mutation_error(cx: &NativeContext<'_>, value: Value) -> Error {
    if cx.heap.pair(value).is_some() && cx.heap.is_immutable(value) {
        immutable_error("pair")
    } else {
        type_error("pair", value, cx.heap)
    }
}

/// Explains a failed indexed mutation: a valid index into an immutable
/// sequence (a literal) gets the immutability error, anything else the
/// usual range-or-type error.
pub(super) fn sequence_mutation_error(
    cx: &NativeContext<'_>,
    length: Option<usize>,
    index: usize,
    expected: &str,
    value: Value,
) -> Error {
    if length.is_some_and(|length| index < length) && cx.heap.is_immutable(value) {
        immutable_error(expected)
    } else {
        range_or_type(length, expected, value)
    }
}

// Resolves optional (start, end) bounds against a sequence length, matching the
// old Scheme behavior: start defaults to 0 and end to the length, an empty or
// reversed range accesses nothing, and only a non-empty range reaching past the
// end raises. Callers iterate `start..end`, which is empty when start >= end.
fn resolve_bounds(
    cx: &NativeContext<'_>,
    bounds: &[Value],
    length: usize,
) -> Result<(usize, usize), Error> {
    let start = match bounds.first() {
        Some(value) => index(cx, *value)?,
        None => 0,
    };
    let end = match bounds.get(1) {
        Some(value) => index(cx, *value)?,
        None => length,
    };
    if start < end && end > length {
        return Err(out_of_range());
    }
    Ok((start, end))
}

fn string_chars(cx: &NativeContext<'_>, value: Value) -> Result<Vec<char>, Error> {
    cx.heap
        .string_slice(value)
        .map(|text| text.chars().collect())
        .ok_or_else(|| type_error("string", value, cx.heap))
}

fn vector_elements(cx: &NativeContext<'_>, value: Value) -> Result<Vec<Value>, Error> {
    cx.heap
        .vector(value)
        .ok_or_else(|| type_error("vector", value, cx.heap))
}

pub(super) fn string_to_list(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let chars = cx
        .heap
        .string_len(a[0])
        .ok_or_else(|| type_error("string", a[0], cx.heap))?;
    let (start, end) = resolve_bounds(cx, &a[1..], chars)?;
    let mut result = Value::nil();
    if start >= end {
        return Ok(result);
    }
    // Snapshot only the requested range as UTF-8 text. Pair allocation below
    // needs the mutable context, so the borrowed slice cannot be held.
    let text = cx
        .heap
        .string_range(a[0], start, end)
        .map(str::to_owned)
        .ok_or_else(|| type_error("string", a[0], cx.heap))?;
    for value in text.chars().rev() {
        result = cx.pair(Value::character(value), result)?;
    }
    Ok(result)
}

pub(super) fn list_to_string(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let (elements, tail) = collect_list(cx, a[0])?;
    if tail != Value::nil() {
        return Err(type_error("pair", tail, cx.heap));
    }
    let chars = elements
        .iter()
        .map(|value| character(cx, *value))
        .collect::<Result<Vec<_>, _>>()?;
    cx.string(chars)
}

pub(super) fn vector_to_list(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let elements = vector_elements(cx, a[0])?;
    let (start, end) = resolve_bounds(cx, &a[1..], elements.len())?;
    let mut result = Value::nil();
    // The source vector stays rooted through the register view, so its elements
    // stay live across the pair allocations below.
    for index in (start..end).rev() {
        result = cx.pair(elements[index], result)?;
    }
    Ok(result)
}

pub(super) fn list_to_vector(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let (elements, tail) = collect_list(cx, a[0])?;
    if tail != Value::nil() {
        return Err(type_error("pair", tail, cx.heap));
    }
    cx.vector(elements)
}

pub(super) fn string_to_vector(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let chars = string_chars(cx, a[0])?;
    let (start, end) = resolve_bounds(cx, &a[1..], chars.len())?;
    let elements = (start..end).map(|i| Value::character(chars[i])).collect();
    cx.vector(elements)
}

pub(super) fn vector_to_string(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let elements = vector_elements(cx, a[0])?;
    let (start, end) = resolve_bounds(cx, &a[1..], elements.len())?;
    let chars = (start..end)
        .map(|i| character(cx, elements[i]))
        .collect::<Result<Vec<_>, _>>()?;
    cx.string(chars)
}

pub(super) fn vector_append(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let mut total = 0usize;
    for value in a {
        let part = cx
            .heap
            .vector_len(*value)
            .ok_or_else(|| type_error("vector", *value, cx.heap))?;
        total = total
            .checked_add(part)
            .ok_or_else(|| Error::plain(ErrorKind::HeapLimitExceeded, "vector is too large"))?;
    }
    let mut elements: Vec<Value> = Vec::new();
    elements
        .try_reserve_exact(total)
        .map_err(|_| Error::plain(ErrorKind::HeapLimitExceeded, "vector is too large"))?;
    for value in a {
        elements.extend(vector_elements(cx, *value)?);
    }
    cx.vector(elements)
}

pub(super) fn string_copy(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let chars = cx
        .heap
        .string_len(a[0])
        .ok_or_else(|| type_error("string", a[0], cx.heap))?;
    let (start, end) = resolve_bounds(cx, &a[1..], chars)?;
    // The validated char range maps to one UTF-8 byte range, so the copy is
    // a single bulk byte copy with no per-char decode and re-encode. A
    // reversed or empty range copies nothing and must not reach
    // `string_range`, whose contract requires start <= end <= length.
    let (text, count) = if start < end {
        let text = cx
            .heap
            .string_range(a[0], start, end)
            .map(str::to_owned)
            .ok_or_else(|| type_error("string", a[0], cx.heap))?;
        (text, end - start)
    } else {
        (String::new(), 0)
    };
    cx.string_with_char_count(&text, count)
}

pub(super) fn substring(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    // (substring s start end) is exactly (string-copy s start end).
    string_copy(cx, a)
}

pub(super) fn vector_copy(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let length = cx
        .heap
        .vector_len(a[0])
        .ok_or_else(|| type_error("vector", a[0], cx.heap))?;
    let (start, end) = resolve_bounds(cx, &a[1..], length)?;
    // Copy the requested range directly instead of cloning the whole source
    // vector first. A reversed or empty range copies nothing.
    let out = if start < end {
        cx.heap
            .vector_slice(a[0])
            .and_then(|values| values.get(start..end))
            .ok_or_else(|| type_error("vector", a[0], cx.heap))?
            .to_vec()
    } else {
        Vec::new()
    };
    cx.vector(out)
}

pub(super) fn string_fill(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let length = cx
        .heap
        .string_len(a[0])
        .ok_or_else(|| type_error("string", a[0], cx.heap))?;
    let fill = character(cx, a[1])?;
    let (start, end) = resolve_bounds(cx, &a[2..], length)?;
    for index in start..end {
        if !cx.string_set(a[0], index, fill)? {
            return Err(sequence_mutation_error(
                cx,
                cx.heap.string_len(a[0]),
                index,
                "string",
                a[0],
            ));
        }
    }
    Ok(Value::unspecified())
}

pub(super) fn vector_fill(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let length = cx
        .heap
        .vector_len(a[0])
        .ok_or_else(|| type_error("vector", a[0], cx.heap))?;
    let fill = a[1];
    let (start, end) = resolve_bounds(cx, &a[2..], length)?;
    for index in start..end {
        if !cx.heap.vector_set(a[0], index, fill) {
            return Err(sequence_mutation_error(
                cx,
                cx.heap.vector_len(a[0]),
                index,
                "vector",
                a[0],
            ));
        }
    }
    Ok(Value::unspecified())
}

pub(super) fn string_copy_mut(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let target_length = cx
        .heap
        .string_len(a[0])
        .ok_or_else(|| type_error("string", a[0], cx.heap))?;
    let at = index(cx, a[1])?;
    let chars = cx
        .heap
        .string_len(a[2])
        .ok_or_else(|| type_error("string", a[2], cx.heap))?;
    let (start, end) = resolve_bounds(cx, &a[3..], chars)?;
    let target_end = at
        .checked_add(end - start)
        .ok_or_else(|| Error::plain(ErrorKind::RangeError, "string target range overflowed"))?;
    if target_end > target_length {
        return Err(Error::plain(
            ErrorKind::RangeError,
            "string target range is out of bounds",
        ));
    }
    if start >= end {
        return Ok(Value::unspecified());
    }
    // Snapshotting the source range keeps an overlapping to/from copy
    // memmove-safe: writes to the destination never disturb the read source.
    let from = cx
        .heap
        .string_range(a[2], start, end)
        .map(str::to_owned)
        .ok_or_else(|| type_error("string", a[2], cx.heap))?;
    for (offset, value) in from.chars().enumerate() {
        let target = at.checked_add(offset).ok_or_else(out_of_range)?;
        if !cx.string_set(a[0], target, value)? {
            return Err(sequence_mutation_error(
                cx,
                cx.heap.string_len(a[0]),
                target,
                "string",
                a[0],
            ));
        }
    }
    Ok(Value::unspecified())
}

pub(super) fn vector_copy_mut(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let target_length = cx
        .heap
        .vector_len(a[0])
        .ok_or_else(|| type_error("vector", a[0], cx.heap))?;
    let at = index(cx, a[1])?;
    let length = cx
        .heap
        .vector_len(a[2])
        .ok_or_else(|| type_error("vector", a[2], cx.heap))?;
    let (start, end) = resolve_bounds(cx, &a[3..], length)?;
    let target_end = at
        .checked_add(end - start)
        .ok_or_else(|| Error::plain(ErrorKind::RangeError, "vector target range overflowed"))?;
    if target_end > target_length {
        return Err(Error::plain(
            ErrorKind::RangeError,
            "vector target range is out of bounds",
        ));
    }
    if start >= end {
        return Ok(Value::unspecified());
    }
    // Snapshotting only the source range keeps an overlapping to/from copy
    // memmove-safe: writes to the destination never disturb the read source.
    let from = cx
        .heap
        .vector_slice(a[2])
        .and_then(|values| values.get(start..end))
        .ok_or_else(|| type_error("vector", a[2], cx.heap))?
        .to_vec();
    for (offset, value) in from.into_iter().enumerate() {
        let target = at.checked_add(offset).ok_or_else(out_of_range)?;
        if !cx.heap.vector_set(a[0], target, value) {
            return Err(sequence_mutation_error(
                cx,
                cx.heap.vector_len(a[0]),
                target,
                "vector",
                a[0],
            ));
        }
    }
    Ok(Value::unspecified())
}

pub(super) fn vector(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    cx.vector_from_slice(a)
}

pub(super) fn make_vector(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let len = index(cx, a[0])?;
    let fill = a.get(1).copied().unwrap_or(Value::unspecified());
    cx.vector_filled(fill, len)
}

pub(super) fn vector_length(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    length(cx.heap.vector_len(a[0]), "vector", a[0], cx.heap)
}

pub(super) fn vector_ref(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let i = index(cx, a[1])?;
    cx.heap
        .vector_ref(a[0], i)
        .ok_or_else(|| range_or_type(cx.heap.vector_len(a[0]), "vector", a[0]))
}

pub(super) fn vector_set(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let i = index(cx, a[1])?;
    if cx.heap.vector_set(a[0], i, a[2]) {
        Ok(Value::unspecified())
    } else {
        Err(sequence_mutation_error(
            cx,
            cx.heap.vector_len(a[0]),
            i,
            "vector",
            a[0],
        ))
    }
}

pub(super) fn string(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    cx.string(
        a.iter()
            .map(|value| character(cx, *value))
            .collect::<Result<Vec<_>, _>>()?,
    )
}

pub(super) fn make_string(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let len = index(cx, a[0])?;
    let fill = match a.get(1) {
        Some(value) => character(cx, *value)?,
        None => '\0',
    };
    cx.string_filled(fill, len)
}

pub(super) fn string_length(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    length(cx.heap.string_len(a[0]), "string", a[0], cx.heap)
}

pub(super) fn string_ref(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let i = index(cx, a[1])?;
    cx.heap
        .string_ref(a[0], i)
        .map(Value::character)
        .ok_or_else(|| range_or_type(cx.heap.string_len(a[0]), "string", a[0]))
}

pub(super) fn string_set(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let i = index(cx, a[1])?;
    let c = character(cx, a[2])?;
    if cx.string_set(a[0], i, c)? {
        Ok(Value::unspecified())
    } else {
        Err(sequence_mutation_error(
            cx,
            cx.heap.string_len(a[0]),
            i,
            "string",
            a[0],
        ))
    }
}

pub(super) fn string_append(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let mut total_bytes = 0usize;
    let mut total_chars = 0usize;
    for value in a {
        let (bytes, chars) = cx
            .heap
            .string_dimensions(*value)
            .ok_or_else(|| type_error("string", *value, cx.heap))?;
        total_bytes = total_bytes
            .checked_add(bytes)
            .ok_or_else(|| Error::plain(ErrorKind::HeapLimitExceeded, "string is too large"))?;
        total_chars += chars;
    }
    // Every argument was validated above, so the parts concatenate span to
    // span inside the arena with no temporary text.
    cx.string_concat(a, total_bytes, total_chars)
}

pub(super) fn string_argument(cx: &NativeContext<'_>, value: Value) -> Result<String, Error> {
    cx.heap
        .string(value)
        .ok_or_else(|| type_error("string", value, cx.heap))
}

pub(super) fn string_order(
    cx: &NativeContext<'_>,
    values: &[Value],
    fold: bool,
    allowed: impl Fn(std::cmp::Ordering) -> bool,
) -> Result<Value, Error> {
    let text = |value| -> Result<String, Error> {
        let value = string_argument(cx, value)?;
        Ok(if fold { unicode_fold(&value) } else { value })
    };
    let mut previous = text(values[0])?;
    for value in &values[1..] {
        let next = text(*value)?;
        if !allowed(previous.cmp(&next)) {
            return bool_value(false);
        }
        previous = next;
    }
    bool_value(true)
}

pub(super) fn string_equal(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    string_order(cx, a, false, |o| o == std::cmp::Ordering::Equal)
}

pub(super) fn string_less(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    string_order(cx, a, false, |o| o == std::cmp::Ordering::Less)
}

pub(super) fn string_greater(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    string_order(cx, a, false, |o| o == std::cmp::Ordering::Greater)
}

pub(super) fn string_less_equal(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    string_order(cx, a, false, |o| o != std::cmp::Ordering::Greater)
}

pub(super) fn string_greater_equal(
    cx: &mut NativeContext<'_>,
    a: &[Value],
) -> Result<Value, Error> {
    string_order(cx, a, false, |o| o != std::cmp::Ordering::Less)
}

pub(super) fn bytevector(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    cx.bytevector(
        a.iter()
            .map(|value| byte(cx, *value))
            .collect::<Result<Vec<_>, _>>()?,
    )
}

pub(super) fn make_bytevector(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let len = index(cx, a[0])?;
    let fill = match a.get(1) {
        Some(value) => fill_byte(cx, *value)?,
        None => 0,
    };
    cx.bytevector_filled(fill, len)
}

pub(super) fn bytevector_length(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    length(cx.heap.bytevector_len(a[0]), "bytevector", a[0], cx.heap)
}

pub(super) fn bytevector_ref(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let i = index(cx, a[1])?;
    cx.heap
        .bytevector_ref(a[0], i)
        .map(|value| Value::integer(i64::from(value)))
        .ok_or_else(|| range_or_type(cx.heap.bytevector_len(a[0]), "bytevector", a[0]))
}

pub(super) fn bytevector_set(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let i = index(cx, a[1])?;
    let b = byte(cx, a[2])?;
    if cx.heap.bytevector_set(a[0], i, b) {
        Ok(Value::unspecified())
    } else {
        Err(sequence_mutation_error(
            cx,
            cx.heap.bytevector_len(a[0]),
            i,
            "bytevector",
            a[0],
        ))
    }
}

pub(super) fn bytevector_bounds(
    cx: &NativeContext<'_>,
    value: Value,
    arguments: &[Value],
) -> Result<(Vec<u8>, usize, usize), Error> {
    let bytes = bytevector_argument(cx, value)?;
    let start = arguments
        .first()
        .map(|value| index(cx, *value))
        .transpose()?
        .unwrap_or(0);
    let end = arguments
        .get(1)
        .map(|value| index(cx, *value))
        .transpose()?
        .unwrap_or(bytes.len());
    if start > end || end > bytes.len() {
        return Err(Error::plain(
            ErrorKind::RangeError,
            "invalid bytevector range",
        ));
    }
    Ok((bytes, start, end))
}

pub(super) fn bytevector_copy(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let (bytes, start, end) = bytevector_bounds(cx, a[0], &a[1..])?;
    cx.bytevector(bytes[start..end].to_vec())
}

pub(super) fn bytevector_copy_mut(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let target_length = cx
        .heap
        .bytevector_len(a[0])
        .ok_or_else(|| type_error("bytevector", a[0], cx.heap))?;
    let at = index(cx, a[1])?;
    let (source, start, end) = bytevector_bounds(cx, a[2], &a[3..])?;
    let count = end - start;
    let target_end = at
        .checked_add(count)
        .ok_or_else(|| Error::plain(ErrorKind::RangeError, "bytevector target range overflowed"))?;
    if target_end > target_length {
        return Err(Error::plain(
            ErrorKind::RangeError,
            "bytevector target range is out of bounds",
        ));
    }
    for (offset, value) in source[start..end].iter().copied().enumerate() {
        if !cx.heap.bytevector_set(a[0], at + offset, value) {
            return Err(sequence_mutation_error(
                cx,
                cx.heap.bytevector_len(a[0]),
                at + offset,
                "bytevector",
                a[0],
            ));
        }
    }
    Ok(Value::unspecified())
}

pub(super) fn bytevector_append(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let mut output = Vec::new();
    for value in a {
        let bytes = bytevector_argument(cx, *value)?;
        output
            .try_reserve(bytes.len())
            .map_err(|_| Error::plain(ErrorKind::HeapLimitExceeded, "bytevector is too large"))?;
        output.extend(bytes);
    }
    cx.bytevector(output)
}

pub(super) fn string_to_utf8(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let len = cx
        .heap
        .string_len(a[0])
        .ok_or_else(|| type_error("string", a[0], cx.heap))?;
    let start = a.get(1).map(|v| index(cx, *v)).transpose()?.unwrap_or(0);
    let end = a.get(2).map(|v| index(cx, *v)).transpose()?.unwrap_or(len);
    if start > end || end > len {
        return Err(Error::plain(ErrorKind::RangeError, "invalid string range"));
    }
    let bytes = cx
        .heap
        .string_range(a[0], start, end)
        .ok_or_else(|| type_error("string", a[0], cx.heap))?
        .as_bytes()
        .to_vec();
    cx.bytevector(bytes)
}

pub(super) fn utf8_to_string(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let (bytes, start, end) = bytevector_bounds(cx, a[0], &a[1..])?;
    let text = std::str::from_utf8(&bytes[start..end]).map_err(|error| {
        Error::plain(
            ErrorKind::RuntimeError,
            format!("bytevector is not valid UTF-8: {error}"),
        )
    })?;
    cx.string_utf8(text.to_owned())
}

pub(super) fn string_to_symbol(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let text = string_argument(cx, a[0])?;
    cx.intern_symbol(&text)
}

pub(super) fn symbol_to_string(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let text = cx
        .heap
        .symbol(a[0])
        .ok_or_else(|| type_error("symbol", a[0], cx.heap))?;
    cx.string_utf8(text)
}
