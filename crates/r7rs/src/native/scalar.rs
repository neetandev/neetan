//! Boolean, record-type, character, type-predicate, `values`, equivalence, and
//! character/string case procedures.

use super::{collection::*, number::*, *};

pub(super) fn predicate_boolean(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    bool_value(cx.kind(a[0]) == ValueKind::Boolean)
}

pub(super) fn not(_cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    bool_value(a[0] == Value::boolean(false))
}

pub(super) fn boolean_equal(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    for &value in a {
        if cx.kind(value) != ValueKind::Boolean {
            return Err(type_error("boolean", value, cx.heap));
        }
    }
    let first = a[0];
    bool_value(a[1..].iter().all(|value| eqv_value(cx.heap, first, *value)))
}

pub(super) fn symbol_equal(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    for &value in a {
        if cx.kind(value) != ValueKind::Symbol {
            return Err(type_error("symbol", value, cx.heap));
        }
    }
    let first = a[0];
    bool_value(a[1..].iter().all(|value| eqv_value(cx.heap, first, *value)))
}

pub(super) fn make_record_type(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let fields = index(cx, a[0])?;
    cx.alloc(Object::RecordType(crate::heap::RecordType { fields }))
}

pub(super) fn record_type_argument(
    cx: &NativeContext<'_>,
    value: Value,
) -> Result<crate::heap::RecordType, Error> {
    cx.heap
        .record_type(value)
        .ok_or_else(|| type_error("record type", value, cx.heap))
}

pub(super) fn make_record_constructor(
    cx: &mut NativeContext<'_>,
    a: &[Value],
) -> Result<Value, Error> {
    let record_type = record_type_argument(cx, a[0])?;
    let mapping = a[1..]
        .iter()
        .map(|value| index(cx, *value))
        .collect::<Result<Vec<_>, _>>()?;
    if mapping.iter().any(|field| *field >= record_type.fields) {
        return Err(Error::plain(
            ErrorKind::RangeError,
            "record field is out of range",
        ));
    }
    cx.alloc(Object::RecordProcedure(Box::new(
        crate::heap::RecordProcedure::Constructor {
            record_type: a[0],
            fields: record_type.fields,
            mapping,
        },
    )))
}

pub(super) fn make_record_predicate(
    cx: &mut NativeContext<'_>,
    a: &[Value],
) -> Result<Value, Error> {
    record_type_argument(cx, a[0])?;
    cx.alloc(Object::RecordProcedure(Box::new(
        crate::heap::RecordProcedure::Predicate { record_type: a[0] },
    )))
}

pub(super) fn record_field_argument(cx: &NativeContext<'_>, a: &[Value]) -> Result<usize, Error> {
    let record_type = record_type_argument(cx, a[0])?;
    let field = index(cx, a[1])?;
    if field >= record_type.fields {
        Err(Error::plain(
            ErrorKind::RangeError,
            "record field is out of range",
        ))
    } else {
        Ok(field)
    }
}

pub(super) fn make_record_accessor(
    cx: &mut NativeContext<'_>,
    a: &[Value],
) -> Result<Value, Error> {
    let field = record_field_argument(cx, a)?;
    cx.alloc(Object::RecordProcedure(Box::new(
        crate::heap::RecordProcedure::Accessor {
            record_type: a[0],
            field,
        },
    )))
}

pub(super) fn make_record_mutator(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let field = record_field_argument(cx, a)?;
    cx.alloc(Object::RecordProcedure(Box::new(
        crate::heap::RecordProcedure::Mutator {
            record_type: a[0],
            field,
        },
    )))
}

pub(super) fn predicate_char(_: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    bool_value(a[0].kind() == ValueKind::Character)
}

pub(super) fn char_order(
    cx: &NativeContext<'_>,
    values: &[Value],
    transform: fn(char) -> char,
    allowed: impl Fn(std::cmp::Ordering) -> bool,
) -> Result<Value, Error> {
    let mut previous = transform(character(cx, values[0])?);
    for value in &values[1..] {
        let next = transform(character(cx, *value)?);
        if !allowed(previous.cmp(&next)) {
            return bool_value(false);
        }
        previous = next;
    }
    bool_value(true)
}

pub(super) fn identity_char(value: char) -> char {
    value
}

pub(super) fn char_equal(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    char_order(cx, a, identity_char, |o| o == std::cmp::Ordering::Equal)
}

pub(super) fn char_less(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    char_order(cx, a, identity_char, |o| o == std::cmp::Ordering::Less)
}

pub(super) fn char_greater(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    char_order(cx, a, identity_char, |o| o == std::cmp::Ordering::Greater)
}

pub(super) fn char_less_equal(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    char_order(cx, a, identity_char, |o| o != std::cmp::Ordering::Greater)
}

pub(super) fn char_greater_equal(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    char_order(cx, a, identity_char, |o| o != std::cmp::Ordering::Less)
}

pub(super) fn char_to_integer(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    Ok(Value::integer(i64::from(character(cx, a[0])? as u32)))
}

pub(super) fn integer_to_char(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let value = exact_integer(cx, a[0])?;
    let scalar = u32::try_from(value)
        .ok()
        .and_then(char::from_u32)
        .ok_or_else(|| {
            Error::plain(
                ErrorKind::RangeError,
                "integer is not a Unicode scalar value",
            )
        })?;
    Ok(Value::character(scalar))
}

pub(super) fn predicate_null(_: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    bool_value(a[0] == Value::nil())
}

pub(super) fn predicate_pair(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    bool_value(cx.kind(a[0]) == ValueKind::Pair)
}

pub(super) fn predicate_vector(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    bool_value(cx.kind(a[0]) == ValueKind::Vector)
}

pub(super) fn predicate_string(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    bool_value(cx.kind(a[0]) == ValueKind::String)
}

pub(super) fn predicate_bytevector(
    cx: &mut NativeContext<'_>,
    a: &[Value],
) -> Result<Value, Error> {
    bool_value(cx.kind(a[0]) == ValueKind::Bytevector)
}

pub(super) fn predicate_symbol(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    bool_value(cx.kind(a[0]) == ValueKind::Symbol)
}

pub(super) fn predicate_procedure(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    bool_value(matches!(
        cx.kind(a[0]),
        ValueKind::Procedure | ValueKind::NativeProcedure | ValueKind::Parameter
    ))
}

pub(super) fn predicate_promise(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    bool_value(cx.heap.promise_state(a[0]).is_some())
}

pub(super) fn make_parameter(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    cx.alloc(Object::Parameter(Box::new(crate::heap::Parameter {
        value: a[0],
        converter: None,
    })))
}

pub(super) fn make_promise(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    if cx.heap.promise_state(a[0]).is_some() {
        return Ok(a[0]);
    }
    cx.alloc(Object::Promise(crate::heap::Promise {
        state: crate::heap::PromiseState::Done(vec![a[0]]),
    }))
}

/// Creates the immutable base environment placeholder used by the evaluator.
/// Import-set validation remains in expansion, where syntax bindings can be
/// resolved before code is compiled.
pub(super) fn environment(cx: &mut NativeContext<'_>, _: &[Value]) -> Result<Value, Error> {
    cx.alloc(Object::Environment { mutable: false })
}

pub(super) fn interaction_environment(
    cx: &mut NativeContext<'_>,
    _: &[Value],
) -> Result<Value, Error> {
    cx.alloc(Object::Environment { mutable: true })
}

pub(super) fn error_object_predicate(
    cx: &mut NativeContext<'_>,
    a: &[Value],
) -> Result<Value, Error> {
    bool_value(cx.heap.error_object(a[0]).is_some())
}

pub(super) fn error_object_message(
    cx: &mut NativeContext<'_>,
    a: &[Value],
) -> Result<Value, Error> {
    cx.heap
        .error_object(a[0])
        .map(|error| error.message)
        .ok_or_else(|| type_error("error object", a[0], cx.heap))
}

pub(super) fn error_object_irritants(
    cx: &mut NativeContext<'_>,
    a: &[Value],
) -> Result<Value, Error> {
    let irritants = cx
        .heap
        .error_object(a[0])
        .ok_or_else(|| type_error("error object", a[0], cx.heap))?
        .irritants;
    let mut result = Value::nil();
    for irritant in irritants.into_iter().rev() {
        result = cx.pair(irritant, result)?;
    }
    Ok(result)
}

pub(super) fn read_error_predicate(
    cx: &mut NativeContext<'_>,
    a: &[Value],
) -> Result<Value, Error> {
    bool_value(
        cx.heap
            .error_object(a[0])
            .is_some_and(|error| error.kind == crate::heap::ConditionKind::Read),
    )
}

pub(super) fn file_error_predicate(
    cx: &mut NativeContext<'_>,
    a: &[Value],
) -> Result<Value, Error> {
    bool_value(
        cx.heap
            .error_object(a[0])
            .is_some_and(|error| error.kind == crate::heap::ConditionKind::File),
    )
}

pub(super) fn values_procedure(
    _: &mut NativeContext<'_>,
    values: &[Value],
) -> Result<NativeValues, Error> {
    Ok(NativeValues::many(values.iter().copied()))
}

pub(super) fn eqv(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    bool_value(eqv_value(cx.heap, a[0], a[1]))
}

pub(super) fn eqv_value(heap: &Heap, left: Value, right: Value) -> bool {
    // Bit-identical tagged words are always eqv. For immediates that is the
    // value itself (equal fixnums, the same symbol, char, boolean, or float
    // bit pattern, all of which `number_eqv` would accept). For heap values it
    // is object identity, and a number is trivially eqv to itself (the NaN
    // class includes identical bits). This prefix keeps the dominant fixnum
    // and symbol comparisons out of the numeric tower below. Raw bit identity
    // is required here: `==` follows IEEE float semantics and would wrongly
    // identify 0.0 with -0.0.
    if Value::same_bits(left, right) {
        return true;
    }
    // Two inline fixnums with different bits are different exact integers.
    if Value::pair_key(left, right) == Value::PAIR_BOTH_FIXNUM {
        return false;
    }
    // Numbers follow R7RS `eqv?`: same exactness, exact operands numerically
    // equal, inexact operands bit-identical (with all NaNs one class). This
    // covers mixed fixnum/float pairs, distinct heap-backed numbers, and
    // differing NaN bit patterns.
    if let (Some(left), Some(right)) = (runtime_number(heap, left), runtime_number(heap, right)) {
        return crate::number::number_eqv(left, right);
    }
    // Everything else is distinguished purely by its tagged-word bits, which
    // already compared unequal above.
    false
}

pub(super) fn equal(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    bool_value(equal_value(cx.heap, a[0], a[1]))
}

pub(super) fn equal_value(heap: &Heap, left: Value, right: Value) -> bool {
    // Non-allocating prefix. Most equal? calls compare scalars or flat
    // strings and bytevectors, none of which need the worklist and cycle
    // set below. Only pair or vector operands can recurse.
    if eqv_value(heap, left, right) {
        return true;
    }
    if left.heap_ref().is_none() || right.heap_ref().is_none() {
        return false;
    }
    if let (Some(a), Some(b)) = (heap.string_slice(left), heap.string_slice(right)) {
        return a == b;
    }
    if let (Some(a), Some(b)) = (heap.bytevector_slice(left), heap.bytevector_slice(right)) {
        return a == b;
    }
    let pairs = heap.pair(left).is_some() && heap.pair(right).is_some();
    let vectors = heap.vector_len(left).is_some() && heap.vector_len(right).is_some();
    if !pairs && !vectors {
        return false;
    }
    equal_value_worklist(heap, left, right)
}

// The allocating recursive compare. The seen set terminates cyclic
// structures.
fn equal_value_worklist(heap: &Heap, left: Value, right: Value) -> bool {
    let mut todo = vec![(left, right)];
    let mut seen = std::collections::HashSet::new();
    while let Some((left, right)) = todo.pop() {
        if eqv_value(heap, left, right) {
            continue;
        }
        let key = match (left.heap_ref(), right.heap_ref()) {
            (Some(a), Some(b)) => (a, b),
            _ => return false,
        };
        if !seen.insert(key) {
            continue;
        }
        if let (Some((a, b)), Some((c, d))) = (heap.pair(left), heap.pair(right)) {
            todo.extend([(a, c), (b, d)]);
        } else if let (Some(a), Some(b)) = (heap.vector(left), heap.vector(right)) {
            if a.len() != b.len() {
                return false;
            }
            todo.extend(a.into_iter().zip(b));
        } else if let (Some(a), Some(b)) = (heap.string_slice(left), heap.string_slice(right)) {
            if a != b {
                return false;
            }
        } else if let (Some(a), Some(b)) =
            (heap.bytevector_slice(left), heap.bytevector_slice(right))
        {
            if a != b {
                return false;
            }
        } else {
            return false;
        }
    }
    true
}

pub(super) fn simple_upcase(value: char) -> char {
    value.to_uppercase().next().unwrap_or(value)
}

pub(super) fn simple_downcase(value: char) -> char {
    value.to_lowercase().next().unwrap_or(value)
}

pub(super) fn simple_fold(value: char) -> char {
    match value {
        'ß' | 'ẞ' => 'ß',
        'ς' => 'σ',
        'K' => 'k',
        value => simple_downcase(value),
    }
}

pub(super) fn unicode_fold(value: &str) -> String {
    value
        .chars()
        .flat_map(|value| match value {
            'ß' | 'ẞ' => "ss".chars().collect::<Vec<_>>(),
            'ς' => vec!['σ'],
            'İ' => vec!['i', '\u{307}'],
            'ſ' => vec!['s'],
            value => value.to_lowercase().collect(),
        })
        .collect()
}

pub(super) fn char_alphabetic(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    bool_value(character(cx, a[0])?.is_alphabetic())
}

pub(super) fn char_numeric(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    bool_value(decimal_digit(character(cx, a[0])?).is_some())
}

pub(super) fn char_whitespace(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    bool_value(character(cx, a[0])?.is_whitespace())
}

pub(super) fn char_upper_case(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    bool_value(character(cx, a[0])?.is_uppercase())
}

pub(super) fn char_lower_case(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    bool_value(character(cx, a[0])?.is_lowercase())
}

pub(super) fn digit_value(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    Ok(decimal_digit(character(cx, a[0])?)
        .map(|value| Value::integer(value.into()))
        .unwrap_or(Value::boolean(false)))
}

pub(super) fn char_upcase(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    Ok(Value::character(simple_upcase(character(cx, a[0])?)))
}

pub(super) fn char_downcase(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    Ok(Value::character(simple_downcase(character(cx, a[0])?)))
}

pub(super) fn char_foldcase(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    Ok(Value::character(simple_fold(character(cx, a[0])?)))
}

pub(super) fn char_ci_equal(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    char_order(cx, a, simple_fold, |o| o == std::cmp::Ordering::Equal)
}

pub(super) fn char_ci_less(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    char_order(cx, a, simple_fold, |o| o == std::cmp::Ordering::Less)
}

pub(super) fn char_ci_greater(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    char_order(cx, a, simple_fold, |o| o == std::cmp::Ordering::Greater)
}

pub(super) fn char_ci_less_equal(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    char_order(cx, a, simple_fold, |o| o != std::cmp::Ordering::Greater)
}

pub(super) fn char_ci_greater_equal(
    cx: &mut NativeContext<'_>,
    a: &[Value],
) -> Result<Value, Error> {
    char_order(cx, a, simple_fold, |o| o != std::cmp::Ordering::Less)
}

pub(super) fn string_upcase(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let input = string_argument(cx, a[0])?;
    cx.string(input.chars().flat_map(char::to_uppercase))
}

pub(super) fn string_downcase(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let input = string_argument(cx, a[0])?;
    cx.string(input.chars().flat_map(char::to_lowercase))
}

pub(super) fn string_foldcase(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let input = string_argument(cx, a[0])?;
    cx.string_utf8(unicode_fold(&input))
}

pub(super) fn string_ci_equal(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    string_order(cx, a, true, |o| o == std::cmp::Ordering::Equal)
}

pub(super) fn string_ci_less(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    string_order(cx, a, true, |o| o == std::cmp::Ordering::Less)
}

pub(super) fn string_ci_greater(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    string_order(cx, a, true, |o| o == std::cmp::Ordering::Greater)
}

pub(super) fn string_ci_less_equal(
    cx: &mut NativeContext<'_>,
    a: &[Value],
) -> Result<Value, Error> {
    string_order(cx, a, true, |o| o != std::cmp::Ordering::Greater)
}

pub(super) fn string_ci_greater_equal(
    cx: &mut NativeContext<'_>,
    a: &[Value],
) -> Result<Value, Error> {
    string_order(cx, a, true, |o| o != std::cmp::Ordering::Less)
}

pub(super) fn decimal_digit(value: char) -> Option<u8> {
    let value = value as u32;
    const ZEROES: &[u32] = &[
        0x0030, 0x0660, 0x06F0, 0x07C0, 0x0966, 0x09E6, 0x0A66, 0x0AE6, 0x0B66, 0x0BE6, 0x0C66,
        0x0CE6, 0x0D66, 0x0DE6, 0x0E50, 0x0ED0, 0x0F20, 0x1040, 0x1090, 0x17E0, 0x1810, 0x1946,
        0x19D0, 0x1A80, 0x1A90, 0x1B50, 0x1BB0, 0x1C40, 0x1C50, 0xA620, 0xA8D0, 0xA900, 0xA9D0,
        0xA9F0, 0xAA50, 0xABF0, 0xFF10, 0x104A0, 0x11066, 0x110F0, 0x11136, 0x111D0, 0x112F0,
        0x11450, 0x114D0, 0x11650, 0x116C0, 0x11730, 0x118E0, 0x11950, 0x11C50, 0x11D50, 0x11DA0,
        0x16A60, 0x16AC0, 0x16B50, 0x1D7CE, 0x1D7D8, 0x1D7E2, 0x1D7EC, 0x1D7F6, 0x1E140, 0x1E2F0,
        0x1E950, 0x1FBF0,
    ];
    ZEROES
        .iter()
        .find_map(|zero| (value >= *zero && value < *zero + 10).then(|| (value - *zero) as u8))
}
