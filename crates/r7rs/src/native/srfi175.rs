//! Native primitives backing the SRFI 175 ASCII character library.
//!
//! Every procedure is native. The library has no callbacks into Scheme, and
//! its scans and comparisons benefit from direct access to strings and
//! bytevectors. The implementation's `char-fix` range is the full supported
//! exact-integer range, `i128`.

use std::cmp::Ordering;

use super::{NativeContext, type_error};
use crate::{Error, ErrorKind, Value, value::ValueRepr};

/// Converts a character or exact integer into its numeric value.
fn char_fix(cx: &NativeContext<'_>, value: Value) -> Result<i128, Error> {
    match value.decode() {
        ValueRepr::Character(character) => Ok(i128::from(character as u32)),
        _ => cx
            .to_i128(value)
            .map_err(|_| type_error("character or exact integer", value, cx.heap)),
    }
}

/// Returns `mapped` with the same character-or-integer representation as
/// `original`. Every mapped value produced by this library is ASCII.
fn mapped_like(cx: &mut NativeContext<'_>, original: Value, mapped: i128) -> Result<Value, Error> {
    match original.decode() {
        ValueRepr::Character(_) => Ok(Value::character(char::from(mapped as u8))),
        _ => cx.integer(mapped),
    }
}

/// Returns a Scheme boolean after applying an ASCII class predicate.
fn class_predicate(
    cx: &mut NativeContext<'_>,
    value: Value,
    predicate: impl FnOnce(i128) -> bool,
) -> Result<Value, Error> {
    Ok(Value::boolean(predicate(char_fix(cx, value)?)))
}

/// Reports an exact result outside the implementation's numeric tower.
fn overflow(operation: &str) -> Error {
    Error::plain(
        ErrorKind::ImplementationRestriction,
        format!("{operation}: result is outside the exact integer range"),
    )
}

/// Validates the SRFI requirement that `offset + limit - 1` fit in char-fix.
fn validate_offset_limit(offset: i128, limit: i128, operation: &str) -> Result<(), Error> {
    let direct = offset
        .checked_add(limit)
        .and_then(|value| value.checked_sub(1));
    let regrouped = limit
        .checked_sub(1)
        .and_then(|value| offset.checked_add(value));
    if direct.is_some() || regrouped.is_some() {
        Ok(())
    } else {
        Err(overflow(operation))
    }
}

/// Converts ASCII upper case to lower case for comparisons.
fn fold_ascii(value: i128) -> i128 {
    if (0x41..=0x5A).contains(&value) {
        value + 0x20
    } else {
        value
    }
}

/// Compares two character-or-integer values after ASCII-only case folding.
fn compare_chars(cx: &NativeContext<'_>, left: Value, right: Value) -> Result<Ordering, Error> {
    Ok(fold_ascii(char_fix(cx, left)?).cmp(&fold_ascii(char_fix(cx, right)?)))
}

/// Compares two strings by Unicode scalar value after ASCII-only case folding.
fn compare_strings(cx: &NativeContext<'_>, left: Value, right: Value) -> Result<Ordering, Error> {
    let left = cx
        .heap
        .string_slice(left)
        .ok_or_else(|| type_error("string", left, cx.heap))?;
    let right = cx
        .heap
        .string_slice(right)
        .ok_or_else(|| type_error("string", right, cx.heap))?;
    Ok(left
        .chars()
        .map(|character| character.to_ascii_lowercase())
        .cmp(
            right
                .chars()
                .map(|character| character.to_ascii_lowercase()),
        ))
}

pub(crate) fn ascii_codepoint_p(
    cx: &mut NativeContext<'_>,
    args: &[Value],
) -> Result<Value, Error> {
    Ok(Value::boolean(
        cx.to_i128(args[0])
            .is_ok_and(|value| (0..=0x7F).contains(&value)),
    ))
}

pub(crate) fn ascii_bytevector_p(
    cx: &mut NativeContext<'_>,
    args: &[Value],
) -> Result<Value, Error> {
    Ok(Value::boolean(
        cx.heap
            .bytevector_slice(args[0])
            .is_some_and(|bytes| bytes.is_ascii()),
    ))
}

pub(crate) fn ascii_char_p(_cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    Ok(Value::boolean(matches!(
        args[0].decode(),
        ValueRepr::Character(character) if character.is_ascii()
    )))
}

pub(crate) fn ascii_string_p(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    Ok(Value::boolean(
        cx.heap.string_slice(args[0]).is_some_and(str::is_ascii),
    ))
}

pub(crate) fn ascii_control_p(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    class_predicate(cx, args[0], |value| {
        (0..=0x1F).contains(&value) || value == 0x7F
    })
}

pub(crate) fn ascii_non_control_p(
    cx: &mut NativeContext<'_>,
    args: &[Value],
) -> Result<Value, Error> {
    class_predicate(cx, args[0], |value| (0x20..=0x7E).contains(&value))
}

pub(crate) fn ascii_whitespace_p(
    cx: &mut NativeContext<'_>,
    args: &[Value],
) -> Result<Value, Error> {
    class_predicate(cx, args[0], |value| {
        (0x09..=0x0D).contains(&value) || value == 0x20
    })
}

pub(crate) fn ascii_space_or_tab_p(
    cx: &mut NativeContext<'_>,
    args: &[Value],
) -> Result<Value, Error> {
    class_predicate(cx, args[0], |value| matches!(value, 0x09 | 0x20))
}

pub(crate) fn ascii_other_graphic_p(
    cx: &mut NativeContext<'_>,
    args: &[Value],
) -> Result<Value, Error> {
    class_predicate(cx, args[0], |value| {
        (0x21..=0x2F).contains(&value)
            || (0x3A..=0x40).contains(&value)
            || (0x5B..=0x60).contains(&value)
            || (0x7B..=0x7E).contains(&value)
    })
}

pub(crate) fn ascii_upper_case_p(
    cx: &mut NativeContext<'_>,
    args: &[Value],
) -> Result<Value, Error> {
    class_predicate(cx, args[0], |value| (0x41..=0x5A).contains(&value))
}

pub(crate) fn ascii_lower_case_p(
    cx: &mut NativeContext<'_>,
    args: &[Value],
) -> Result<Value, Error> {
    class_predicate(cx, args[0], |value| (0x61..=0x7A).contains(&value))
}

pub(crate) fn ascii_alphabetic_p(
    cx: &mut NativeContext<'_>,
    args: &[Value],
) -> Result<Value, Error> {
    class_predicate(cx, args[0], |value| {
        (0x41..=0x5A).contains(&value) || (0x61..=0x7A).contains(&value)
    })
}

pub(crate) fn ascii_alphanumeric_p(
    cx: &mut NativeContext<'_>,
    args: &[Value],
) -> Result<Value, Error> {
    class_predicate(cx, args[0], |value| {
        (0x30..=0x39).contains(&value)
            || (0x41..=0x5A).contains(&value)
            || (0x61..=0x7A).contains(&value)
    })
}

pub(crate) fn ascii_numeric_p(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    class_predicate(cx, args[0], |value| (0x30..=0x39).contains(&value))
}

pub(crate) fn ascii_digit_value(
    cx: &mut NativeContext<'_>,
    args: &[Value],
) -> Result<Value, Error> {
    let value = char_fix(cx, args[0])?;
    let limit = cx.to_i128(args[1])?;
    if (0x30..=0x39).contains(&value) && value - 0x30 < limit {
        let distance = value - 0x30;
        cx.integer(distance)
    } else {
        Ok(Value::boolean(false))
    }
}

fn ascii_letter_value(
    cx: &mut NativeContext<'_>,
    args: &[Value],
    base: i128,
    operation: &str,
) -> Result<Value, Error> {
    let value = char_fix(cx, args[0])?;
    let offset = cx.to_i128(args[1])?;
    let limit = cx.to_i128(args[2])?;
    validate_offset_limit(offset, limit, operation)?;
    if (base..=base + 25).contains(&value) && value - base < limit {
        let distance = value - base;
        cx.integer(
            offset
                .checked_add(distance)
                .ok_or_else(|| overflow(operation))?,
        )
    } else {
        Ok(Value::boolean(false))
    }
}

pub(crate) fn ascii_upper_case_value(
    cx: &mut NativeContext<'_>,
    args: &[Value],
) -> Result<Value, Error> {
    ascii_letter_value(cx, args, 0x41, "ascii-upper-case-value")
}

pub(crate) fn ascii_lower_case_value(
    cx: &mut NativeContext<'_>,
    args: &[Value],
) -> Result<Value, Error> {
    ascii_letter_value(cx, args, 0x61, "ascii-lower-case-value")
}

pub(crate) fn ascii_nth_digit(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    let value = cx.to_i128(args[0])?;
    if let Ok(digit) = u8::try_from(value)
        && digit <= 9
    {
        Ok(Value::character(char::from(b'0' + digit)))
    } else {
        Ok(Value::boolean(false))
    }
}

fn ascii_nth_letter(cx: &NativeContext<'_>, value: Value, base: u8) -> Result<Value, Error> {
    let index = cx.to_i128(value)?.rem_euclid(26) as u8;
    Ok(Value::character(char::from(base + index)))
}

pub(crate) fn ascii_nth_upper_case(
    cx: &mut NativeContext<'_>,
    args: &[Value],
) -> Result<Value, Error> {
    ascii_nth_letter(cx, args[0], b'A')
}

pub(crate) fn ascii_nth_lower_case(
    cx: &mut NativeContext<'_>,
    args: &[Value],
) -> Result<Value, Error> {
    ascii_nth_letter(cx, args[0], b'a')
}

pub(crate) fn ascii_upcase(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    let value = char_fix(cx, args[0])?;
    if (0x61..=0x7A).contains(&value) {
        mapped_like(cx, args[0], value - 0x20)
    } else {
        Ok(args[0])
    }
}

pub(crate) fn ascii_downcase(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    let value = char_fix(cx, args[0])?;
    if (0x41..=0x5A).contains(&value) {
        mapped_like(cx, args[0], value + 0x20)
    } else {
        Ok(args[0])
    }
}

pub(crate) fn ascii_control_to_graphic(
    cx: &mut NativeContext<'_>,
    args: &[Value],
) -> Result<Value, Error> {
    let value = char_fix(cx, args[0])?;
    let mapped = if (0..=0x1F).contains(&value) {
        Some(value + 0x40)
    } else if value == 0x7F {
        Some(0x3F)
    } else {
        None
    };
    match mapped {
        Some(mapped) => mapped_like(cx, args[0], mapped),
        None => Ok(Value::boolean(false)),
    }
}

pub(crate) fn ascii_graphic_to_control(
    cx: &mut NativeContext<'_>,
    args: &[Value],
) -> Result<Value, Error> {
    let value = char_fix(cx, args[0])?;
    let mapped = if (0x40..=0x5F).contains(&value) {
        Some(value - 0x40)
    } else if value == 0x3F {
        Some(0x7F)
    } else {
        None
    };
    match mapped {
        Some(mapped) => mapped_like(cx, args[0], mapped),
        None => Ok(Value::boolean(false)),
    }
}

pub(crate) fn ascii_mirror_bracket(
    cx: &mut NativeContext<'_>,
    args: &[Value],
) -> Result<Value, Error> {
    let value = char_fix(cx, args[0])?;
    let mapped = match value {
        0x28 => Some(0x29),
        0x29 => Some(0x28),
        0x3C => Some(0x3E),
        0x3E => Some(0x3C),
        0x5B => Some(0x5D),
        0x5D => Some(0x5B),
        0x7B => Some(0x7D),
        0x7D => Some(0x7B),
        _ => None,
    };
    match mapped {
        Some(mapped) => mapped_like(cx, args[0], mapped),
        None => Ok(Value::boolean(false)),
    }
}

fn character_comparison(
    cx: &mut NativeContext<'_>,
    args: &[Value],
    accepted: impl FnOnce(Ordering) -> bool,
) -> Result<Value, Error> {
    Ok(Value::boolean(accepted(compare_chars(
        cx, args[0], args[1],
    )?)))
}

pub(crate) fn ascii_ci_equal(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    character_comparison(cx, args, |ordering| ordering == Ordering::Equal)
}

pub(crate) fn ascii_ci_less(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    character_comparison(cx, args, |ordering| ordering == Ordering::Less)
}

pub(crate) fn ascii_ci_greater(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    character_comparison(cx, args, |ordering| ordering == Ordering::Greater)
}

pub(crate) fn ascii_ci_less_equal(
    cx: &mut NativeContext<'_>,
    args: &[Value],
) -> Result<Value, Error> {
    character_comparison(cx, args, |ordering| ordering != Ordering::Greater)
}

pub(crate) fn ascii_ci_greater_equal(
    cx: &mut NativeContext<'_>,
    args: &[Value],
) -> Result<Value, Error> {
    character_comparison(cx, args, |ordering| ordering != Ordering::Less)
}

fn string_comparison(
    cx: &mut NativeContext<'_>,
    args: &[Value],
    accepted: impl FnOnce(Ordering) -> bool,
) -> Result<Value, Error> {
    Ok(Value::boolean(accepted(compare_strings(
        cx, args[0], args[1],
    )?)))
}

pub(crate) fn ascii_string_ci_equal(
    cx: &mut NativeContext<'_>,
    args: &[Value],
) -> Result<Value, Error> {
    string_comparison(cx, args, |ordering| ordering == Ordering::Equal)
}

pub(crate) fn ascii_string_ci_less(
    cx: &mut NativeContext<'_>,
    args: &[Value],
) -> Result<Value, Error> {
    string_comparison(cx, args, |ordering| ordering == Ordering::Less)
}

pub(crate) fn ascii_string_ci_greater(
    cx: &mut NativeContext<'_>,
    args: &[Value],
) -> Result<Value, Error> {
    string_comparison(cx, args, |ordering| ordering == Ordering::Greater)
}

pub(crate) fn ascii_string_ci_less_equal(
    cx: &mut NativeContext<'_>,
    args: &[Value],
) -> Result<Value, Error> {
    string_comparison(cx, args, |ordering| ordering != Ordering::Greater)
}

pub(crate) fn ascii_string_ci_greater_equal(
    cx: &mut NativeContext<'_>,
    args: &[Value],
) -> Result<Value, Error> {
    string_comparison(cx, args, |ordering| ordering != Ordering::Less)
}
