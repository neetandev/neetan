//! Numeric procedures: predicates, comparison, core arithmetic, sign helpers,
//! rationals, number/string formatting, complex accessors, and rounding.

use super::*;

pub(super) fn runtime_number(heap: &Heap, value: Value) -> Option<RuntimeNumber> {
    match value.decode() {
        crate::value::ValueRepr::Fixnum(value) => {
            Some(RuntimeNumber::Real(Real::ExactInteger(i128::from(value))))
        }
        crate::value::ValueRepr::Float(value) => Some(RuntimeNumber::Real(Real::Inexact(value))),
        _ => heap.number(value),
    }
}

pub(super) fn numeric_value(
    cx: &mut NativeContext<'_>,
    value: RuntimeNumber,
) -> Result<Value, Error> {
    match value {
        RuntimeNumber::Real(Real::ExactInteger(value)) => cx.integer(value),
        RuntimeNumber::Real(Real::Inexact(value)) => Ok(Value::float(value)),
        value => cx.alloc(Object::Number(Box::new(value))),
    }
}

pub(super) fn number_argument(
    cx: &NativeContext<'_>,
    value: Value,
) -> Result<RuntimeNumber, Error> {
    runtime_number(cx.heap, value).ok_or_else(|| type_error("number", value, cx.heap))
}

pub(super) fn predicate_number(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    bool_value(runtime_number(cx.heap, a[0]).is_some())
}

pub(super) fn predicate_exact(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    bool_value(runtime_number(cx.heap, a[0]).is_some_and(RuntimeNumber::is_exact))
}

pub(super) fn predicate_inexact(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    bool_value(runtime_number(cx.heap, a[0]).is_some_and(|value| !value.is_exact()))
}

pub(super) fn predicate_exact_integer(
    cx: &mut NativeContext<'_>,
    a: &[Value],
) -> Result<Value, Error> {
    bool_value(
        runtime_number(cx.heap, a[0]).is_some_and(|value| value.is_exact() && value.is_integer()),
    )
}

pub(super) fn predicate_integer(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    bool_value(runtime_number(cx.heap, a[0]).is_some_and(RuntimeNumber::is_integer))
}

pub(super) fn predicate_rational(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    bool_value(runtime_number(cx.heap, a[0]).is_some_and(RuntimeNumber::is_rational))
}

pub(super) fn predicate_real(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    bool_value(runtime_number(cx.heap, a[0]).is_some_and(RuntimeNumber::is_real))
}

pub(super) fn exact_integer(cx: &NativeContext<'_>, value: Value) -> Result<i128, Error> {
    cx.to_i128(value)
}

pub(super) fn numeric_equal(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let mut previous = number_argument(cx, a[0])?;
    for value in &a[1..] {
        let next = number_argument(cx, *value)?;
        if !crate::number::number_equal(previous, next) {
            return bool_value(false);
        }
        previous = next;
    }
    bool_value(true)
}

pub(super) fn numeric_less(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    numeric_real_compare(cx, a, std::cmp::Ordering::Less)
}

pub(super) fn numeric_greater(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    numeric_real_compare(cx, a, std::cmp::Ordering::Greater)
}

pub(super) fn numeric_less_equal(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    numeric_real_compare_inclusive(cx, a, std::cmp::Ordering::Less)
}

pub(super) fn numeric_greater_equal(
    cx: &mut NativeContext<'_>,
    a: &[Value],
) -> Result<Value, Error> {
    numeric_real_compare_inclusive(cx, a, std::cmp::Ordering::Greater)
}

pub(super) fn numeric_error(message: String) -> Error {
    Error::plain(ErrorKind::ImplementationRestriction, message)
}

pub(super) fn numeric_add(cx: &mut NativeContext<'_>, values: &[Value]) -> Result<Value, Error> {
    let mut result = RuntimeNumber::Real(Real::ExactInteger(0));
    for value in values {
        result = crate::number::number_add(result, number_argument(cx, *value)?)
            .map_err(numeric_error)?;
    }
    numeric_value(cx, result)
}

pub(super) fn numeric_multiply(
    cx: &mut NativeContext<'_>,
    values: &[Value],
) -> Result<Value, Error> {
    let mut result = RuntimeNumber::Real(Real::ExactInteger(1));
    for value in values {
        result = crate::number::number_mul(result, number_argument(cx, *value)?)
            .map_err(numeric_error)?;
    }
    numeric_value(cx, result)
}

pub(super) fn numeric_subtract(
    cx: &mut NativeContext<'_>,
    values: &[Value],
) -> Result<Value, Error> {
    let mut result = number_argument(cx, values[0])?;
    if values.len() == 1 {
        result = crate::number::number_neg(result).map_err(numeric_error)?;
    }
    for value in &values[1..] {
        result = crate::number::number_sub(result, number_argument(cx, *value)?)
            .map_err(numeric_error)?;
    }
    numeric_value(cx, result)
}

pub(super) fn numeric_divide(cx: &mut NativeContext<'_>, values: &[Value]) -> Result<Value, Error> {
    let mut result = RuntimeNumber::Real(Real::ExactInteger(1));
    let start = if values.len() == 1 {
        0
    } else {
        result = number_argument(cx, values[0])?;
        1
    };
    for value in &values[start..] {
        result = crate::number::number_div(result, number_argument(cx, *value)?)
            .map_err(numeric_error)?;
    }
    numeric_value(cx, result)
}

pub(super) fn real_argument(cx: &NativeContext<'_>, value: Value) -> Result<Real, Error> {
    let value = number_argument(cx, value)?;
    if !value.is_real() {
        return Err(Error::plain(
            ErrorKind::TypeError,
            format!("expected real number, received {value:?}"),
        ));
    }
    Ok(value.components().0)
}

pub(super) fn numeric_real_compare(
    cx: &NativeContext<'_>,
    values: &[Value],
    wanted: std::cmp::Ordering,
) -> Result<Value, Error> {
    let mut previous = real_argument(cx, values[0])?;
    for value in &values[1..] {
        let next = real_argument(cx, *value)?;
        if crate::number::real_compare(previous, next) != Some(wanted) {
            return bool_value(false);
        }
        previous = next;
    }
    bool_value(true)
}

pub(super) fn numeric_real_compare_inclusive(
    cx: &NativeContext<'_>,
    values: &[Value],
    wanted: std::cmp::Ordering,
) -> Result<Value, Error> {
    let mut previous = real_argument(cx, values[0])?;
    for value in &values[1..] {
        let next = real_argument(cx, *value)?;
        let Some(ordering) = crate::number::real_compare(previous, next) else {
            return bool_value(false);
        };
        if ordering != wanted && ordering != std::cmp::Ordering::Equal {
            return bool_value(false);
        }
        previous = next;
    }
    bool_value(true)
}

pub(super) fn numeric_exact(cx: &mut NativeContext<'_>, values: &[Value]) -> Result<Value, Error> {
    numeric_value(
        cx,
        number_argument(cx, values[0])?
            .exact()
            .map_err(numeric_error)?,
    )
}

pub(super) fn numeric_inexact(
    cx: &mut NativeContext<'_>,
    values: &[Value],
) -> Result<Value, Error> {
    numeric_value(cx, number_argument(cx, values[0])?.inexact())
}

pub(super) fn numeric_zero(cx: &mut NativeContext<'_>, values: &[Value]) -> Result<Value, Error> {
    bool_value(crate::number::number_equal(
        number_argument(cx, values[0])?,
        RuntimeNumber::Real(Real::ExactInteger(0)),
    ))
}

pub(super) fn numeric_positive(
    cx: &mut NativeContext<'_>,
    values: &[Value],
) -> Result<Value, Error> {
    bool_value(
        crate::number::real_compare(real_argument(cx, values[0])?, Real::ExactInteger(0))
            == Some(std::cmp::Ordering::Greater),
    )
}

pub(super) fn numeric_negative(
    cx: &mut NativeContext<'_>,
    values: &[Value],
) -> Result<Value, Error> {
    bool_value(
        crate::number::real_compare(real_argument(cx, values[0])?, Real::ExactInteger(0))
            == Some(std::cmp::Ordering::Less),
    )
}

pub(super) fn exact_integer_number(cx: &NativeContext<'_>, value: Value) -> Result<i128, Error> {
    let number = number_argument(cx, value)?;
    match number {
        RuntimeNumber::Real(Real::ExactInteger(value)) => Ok(value),
        _ => Err(type_error("exact integer", value, cx.heap)),
    }
}

pub(super) fn numeric_odd(cx: &mut NativeContext<'_>, values: &[Value]) -> Result<Value, Error> {
    Ok(Value::boolean(
        exact_integer_number(cx, values[0])? % 2 != 0,
    ))
}

pub(super) fn numeric_even(cx: &mut NativeContext<'_>, values: &[Value]) -> Result<Value, Error> {
    Ok(Value::boolean(
        exact_integer_number(cx, values[0])? % 2 == 0,
    ))
}

pub(super) fn numeric_abs(cx: &mut NativeContext<'_>, values: &[Value]) -> Result<Value, Error> {
    let value = real_argument(cx, values[0])?;
    let result = match value {
        Real::ExactInteger(v) => Real::ExactInteger(v.checked_abs().ok_or_else(|| {
            numeric_error("exact numeric result exceeds the supported i128 range".into())
        })?),
        Real::ExactRational(v) if v.numerator() >= 0 => Real::ExactRational(v),
        Real::ExactRational(_) => crate::number::real_neg(value).map_err(numeric_error)?,
        Real::Inexact(v) => Real::Inexact(v.abs()),
    };
    numeric_value(cx, RuntimeNumber::Real(result))
}

pub(super) fn numeric_numerator(
    cx: &mut NativeContext<'_>,
    values: &[Value],
) -> Result<Value, Error> {
    let real = real_argument(cx, values[0])?;
    let value = match real {
        Real::ExactInteger(v) => Real::ExactInteger(v),
        Real::ExactRational(v) => Real::ExactInteger(i128::from(v.numerator())),
        Real::Inexact(v) => match Real::Inexact(v).exact().map_err(numeric_error)? {
            Real::ExactInteger(value) => Real::Inexact(value as f64),
            Real::ExactRational(value) => Real::Inexact(value.numerator() as f64),
            Real::Inexact(_) => unreachable!(),
        },
    };
    numeric_value(cx, RuntimeNumber::Real(value))
}

pub(super) fn numeric_denominator(
    cx: &mut NativeContext<'_>,
    values: &[Value],
) -> Result<Value, Error> {
    let real = real_argument(cx, values[0])?;
    let value = match real {
        Real::ExactInteger(_) => Real::ExactInteger(1),
        Real::ExactRational(v) => Real::ExactInteger(i128::from(v.denominator())),
        Real::Inexact(v) => match Real::Inexact(v).exact().map_err(numeric_error)? {
            Real::ExactInteger(_) => Real::Inexact(1.0),
            Real::ExactRational(value) => Real::Inexact(value.denominator() as f64),
            Real::Inexact(_) => unreachable!(),
        },
    };
    numeric_value(cx, RuntimeNumber::Real(value))
}

pub(super) fn rationalize(cx: &mut NativeContext<'_>, values: &[Value]) -> Result<Value, Error> {
    let value = real_argument(cx, values[0])?;
    let tolerance = real_argument(cx, values[1])?;
    let tolerance_value = crate::number::to_f64(tolerance).abs();
    let value_f64 = crate::number::to_f64(value);
    if !value_f64.is_finite() || !tolerance_value.is_finite() {
        return numeric_value(cx, RuntimeNumber::Real(Real::Inexact(value_f64)));
    }
    let (numerator, denominator) =
        simplest_rational(value_f64 - tolerance_value, value_f64 + tolerance_value, 0)?;
    let denominator = i64::try_from(denominator).map_err(|_| {
        Error::plain(
            ErrorKind::ImplementationRestriction,
            "rationalize result exceeds the supported exact range",
        )
    })?;
    let exact = crate::number::rational(numerator, denominator).map_err(numeric_error)?;
    let result = if value.is_exact() && tolerance.is_exact() {
        exact
    } else {
        Real::Inexact(crate::number::to_f64(exact))
    };
    numeric_value(cx, RuntimeNumber::Real(result))
}

pub(super) fn simplest_rational(low: f64, high: f64, depth: usize) -> Result<(i128, i128), Error> {
    if depth >= 128 {
        return Err(Error::plain(
            ErrorKind::ImplementationRestriction,
            "rationalize continued fraction is too deep",
        ));
    }
    if low > high {
        return simplest_rational(high, low, depth);
    }
    if low <= 0.0 && high >= 0.0 {
        return Ok((0, 1));
    }
    if high < 0.0 {
        let (numerator, denominator) = simplest_rational(-high, -low, depth + 1)?;
        return Ok((-numerator, denominator));
    }
    let low_floor = low.floor();
    let high_floor = high.floor();
    if low_floor < high_floor {
        return Ok((low_floor as i128 + 1, 1));
    }
    if low == low_floor {
        return Ok((low_floor as i128, 1));
    }
    let integer = low_floor as i128;
    let (numerator, denominator) =
        simplest_rational(1.0 / (high - low_floor), 1.0 / (low - low_floor), depth + 1)?;
    let combined = integer
        .checked_mul(numerator)
        .and_then(|value| value.checked_add(denominator))
        .ok_or_else(|| {
            Error::plain(
                ErrorKind::ImplementationRestriction,
                "rationalize result exceeds the supported exact range",
            )
        })?;
    Ok((combined, numerator))
}

pub(super) fn number_to_string(
    cx: &mut NativeContext<'_>,
    values: &[Value],
) -> Result<Value, Error> {
    let radix = if values.len() == 2 {
        u32::try_from(exact_integer(cx, values[1])?)
            .ok()
            .filter(|radix| matches!(radix, 2 | 8 | 10 | 16))
            .ok_or_else(|| Error::plain(ErrorKind::RangeError, "radix must be 2, 8, 10, or 16"))?
    } else {
        10
    };
    let text = format_runtime_number(number_argument(cx, values[0])?, radix);
    cx.string_utf8(text)
}

pub(super) fn string_to_number(
    cx: &mut NativeContext<'_>,
    values: &[Value],
) -> Result<Value, Error> {
    let text = string_argument(cx, values[0])?;
    let text = if values.len() == 2 {
        let prefix = match exact_integer(cx, values[1])? {
            2 => "#b",
            8 => "#o",
            10 => "#d",
            16 => "#x",
            _ => {
                return Err(Error::plain(
                    ErrorKind::RangeError,
                    "radix must be 2, 8, 10, or 16",
                ));
            }
        };
        format!("{prefix}{text}")
    } else {
        text
    };
    match crate::number::parse(&text) {
        Some(Ok(number)) => numeric_value(cx, RuntimeNumber::from_literal(number)),
        Some(Err(_)) | None => Ok(Value::boolean(false)),
    }
}

pub(super) fn format_runtime_number(value: RuntimeNumber, radix: u32) -> String {
    let (real, imaginary) = value.components();
    if crate::number::is_zero(imaginary) {
        return format_real(real, radix);
    }
    let mut output = format_real(real, radix);
    if matches!(imaginary, Real::Inexact(value) if value.is_finite() && !value.is_sign_negative())
        || matches!(imaginary, Real::ExactInteger(value) if value >= 0)
        || matches!(imaginary, Real::ExactRational(value) if value.numerator() >= 0)
    {
        output.push('+');
    }
    output.push_str(&format_real(imaginary, radix));
    output.push('i');
    output
}

pub(super) fn format_real(value: Real, radix: u32) -> String {
    match value {
        Real::ExactInteger(value) => format_integer(value, radix),
        Real::ExactRational(value) => format!(
            "{}/{}",
            format_integer(i128::from(value.numerator()), radix),
            format_integer(i128::from(value.denominator()), radix)
        ),
        // Canonical for any NaN bit pattern: runtime arithmetic skips
        // per-result NaN canonicalization, so the bits carry no meaning.
        Real::Inexact(value) if value.is_nan() => "+nan.0".into(),
        Real::Inexact(value) if value == f64::INFINITY => "+inf.0".into(),
        Real::Inexact(value) if value == f64::NEG_INFINITY => "-inf.0".into(),
        Real::Inexact(value) if value == 0.0 && value.is_sign_negative() => "-0.0".into(),
        Real::Inexact(value) => {
            let mut output = format!("{value:?}");
            if let Some(index) = output.find('e') {
                let mut exponent_index = index;
                if !output[..index].contains('.') {
                    output.insert_str(index, ".0");
                    exponent_index += 2;
                }
                if !matches!(
                    output.as_bytes().get(exponent_index + 1),
                    Some(b'+') | Some(b'-')
                ) {
                    output.insert(exponent_index + 1, '+');
                }
            } else if !output.contains('.') {
                output.push_str(".0");
            }
            output
        }
    }
}

pub(super) fn format_integer(value: i128, radix: u32) -> String {
    match radix {
        2 if value < 0 => format!("-{:b}", value.unsigned_abs()),
        2 => format!("{value:b}"),
        8 if value < 0 => format!("-{:o}", value.unsigned_abs()),
        8 => format!("{value:o}"),
        10 => value.to_string(),
        16 if value < 0 => format!("-{:x}", value.unsigned_abs()),
        16 => format!("{value:x}"),
        _ => unreachable!("validated radix"),
    }
}

pub(super) fn make_rectangular(
    cx: &mut NativeContext<'_>,
    values: &[Value],
) -> Result<Value, Error> {
    numeric_value(
        cx,
        RuntimeNumber::complex(real_argument(cx, values[0])?, real_argument(cx, values[1])?),
    )
}

pub(super) fn make_polar(cx: &mut NativeContext<'_>, values: &[Value]) -> Result<Value, Error> {
    let magnitude = crate::number::to_f64(real_argument(cx, values[0])?);
    let angle = crate::number::to_f64(real_argument(cx, values[1])?);
    numeric_value(
        cx,
        RuntimeNumber::complex(
            Real::Inexact(magnitude * angle.cos()),
            Real::Inexact(magnitude * angle.sin()),
        ),
    )
}

pub(super) fn real_part(cx: &mut NativeContext<'_>, values: &[Value]) -> Result<Value, Error> {
    numeric_value(
        cx,
        RuntimeNumber::Real(number_argument(cx, values[0])?.components().0),
    )
}

pub(super) fn imag_part(cx: &mut NativeContext<'_>, values: &[Value]) -> Result<Value, Error> {
    numeric_value(
        cx,
        RuntimeNumber::Real(number_argument(cx, values[0])?.components().1),
    )
}

pub(super) fn magnitude(cx: &mut NativeContext<'_>, values: &[Value]) -> Result<Value, Error> {
    let (a, b) = number_argument(cx, values[0])?.components();
    numeric_value(
        cx,
        RuntimeNumber::Real(Real::Inexact(
            crate::number::to_f64(a).hypot(crate::number::to_f64(b)),
        )),
    )
}

pub(super) fn angle(cx: &mut NativeContext<'_>, values: &[Value]) -> Result<Value, Error> {
    let (a, b) = number_argument(cx, values[0])?.components();
    numeric_value(
        cx,
        RuntimeNumber::Real(Real::Inexact(
            crate::number::to_f64(b).atan2(crate::number::to_f64(a)),
        )),
    )
}

pub(super) fn numeric_finite(cx: &mut NativeContext<'_>, values: &[Value]) -> Result<Value, Error> {
    bool_value(number_argument(cx, values[0])?.is_finite())
}

pub(super) fn numeric_infinite(
    cx: &mut NativeContext<'_>,
    values: &[Value],
) -> Result<Value, Error> {
    bool_value(number_argument(cx, values[0])?.is_infinite())
}

pub(super) fn numeric_nan(cx: &mut NativeContext<'_>, values: &[Value]) -> Result<Value, Error> {
    bool_value(number_argument(cx, values[0])?.is_nan())
}

pub(super) fn numeric_max(cx: &mut NativeContext<'_>, values: &[Value]) -> Result<Value, Error> {
    let mut result = real_argument(cx, values[0])?;
    let mut inexact = !result.is_exact();
    for value in &values[1..] {
        let next = real_argument(cx, *value)?;
        inexact |= !next.is_exact();
        if crate::number::real_compare(result, next) == Some(std::cmp::Ordering::Less) {
            result = next;
        }
    }
    if inexact && result.is_exact() {
        result = Real::Inexact(crate::number::to_f64(result));
    }
    numeric_value(cx, RuntimeNumber::Real(result))
}

pub(super) fn numeric_min(cx: &mut NativeContext<'_>, values: &[Value]) -> Result<Value, Error> {
    let mut result = real_argument(cx, values[0])?;
    let mut inexact = !result.is_exact();
    for value in &values[1..] {
        let next = real_argument(cx, *value)?;
        inexact |= !next.is_exact();
        if crate::number::real_compare(result, next) == Some(std::cmp::Ordering::Greater) {
            result = next;
        }
    }
    if inexact && result.is_exact() {
        result = Real::Inexact(crate::number::to_f64(result));
    }
    numeric_value(cx, RuntimeNumber::Real(result))
}

#[derive(Clone, Copy)]
enum Rounding {
    Floor,
    Ceiling,
    Truncate,
    Round,
}

impl Rounding {
    fn inexact(self, value: f64) -> f64 {
        match self {
            Self::Floor => value.floor(),
            Self::Ceiling => value.ceil(),
            Self::Truncate => value.trunc(),
            Self::Round => value.round_ties_even(),
        }
    }

    fn exact_rational(self, value: crate::ExactRational) -> i128 {
        let numerator = i128::from(value.numerator());
        let denominator = i128::from(value.denominator());
        let quotient = numerator / denominator;
        let remainder = numerator % denominator;
        match self {
            Self::Floor if remainder < 0 => quotient - 1,
            Self::Ceiling if remainder > 0 => quotient + 1,
            Self::Round => {
                let twice_remainder = remainder.unsigned_abs() * 2;
                let denominator = denominator as u128;
                if twice_remainder > denominator
                    || (twice_remainder == denominator && quotient % 2 != 0)
                {
                    quotient + if remainder < 0 { -1 } else { 1 }
                } else {
                    quotient
                }
            }
            Self::Floor | Self::Ceiling | Self::Truncate => quotient,
        }
    }
}

fn rounded(cx: &mut NativeContext<'_>, value: Value, rounding: Rounding) -> Result<Value, Error> {
    let real = real_argument(cx, value)?;
    let result = match real {
        Real::ExactInteger(value) => Real::ExactInteger(value),
        Real::ExactRational(value) => Real::ExactInteger(rounding.exact_rational(value)),
        Real::Inexact(value) => Real::Inexact(rounding.inexact(value)),
    };
    numeric_value(cx, RuntimeNumber::Real(result))
}

pub(super) fn numeric_floor(cx: &mut NativeContext<'_>, values: &[Value]) -> Result<Value, Error> {
    rounded(cx, values[0], Rounding::Floor)
}

pub(super) fn numeric_ceiling(
    cx: &mut NativeContext<'_>,
    values: &[Value],
) -> Result<Value, Error> {
    rounded(cx, values[0], Rounding::Ceiling)
}

pub(super) fn numeric_truncate(
    cx: &mut NativeContext<'_>,
    values: &[Value],
) -> Result<Value, Error> {
    rounded(cx, values[0], Rounding::Truncate)
}

pub(super) fn numeric_round(cx: &mut NativeContext<'_>, values: &[Value]) -> Result<Value, Error> {
    rounded(cx, values[0], Rounding::Round)
}
