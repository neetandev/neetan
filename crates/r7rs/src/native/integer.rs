//! Integer division and remainder, gcd/lcm, powers, roots, and transcendental
//! procedures.

use super::{number::*, *};

pub(super) fn quotient(cx: &mut NativeContext<'_>, values: &[Value]) -> Result<Value, Error> {
    if values
        .iter()
        .any(|value| matches!(real_argument(cx, *value), Ok(Real::Inexact(_))))
    {
        return Ok(Value::float(inexact_integer_division(cx, values, false)?.0));
    }
    let left = exact_integer_number(cx, values[0])?;
    let right = exact_integer_number(cx, values[1])?;
    if right == 0 {
        return Err(Error::plain(ErrorKind::RangeError, "division by zero"));
    }
    let result = left.checked_div(right).ok_or_else(|| {
        Error::plain(
            ErrorKind::ImplementationRestriction,
            "exact division overflow",
        )
    })?;
    cx.integer(result)
}

pub(super) fn remainder(cx: &mut NativeContext<'_>, values: &[Value]) -> Result<Value, Error> {
    if values
        .iter()
        .any(|value| matches!(real_argument(cx, *value), Ok(Real::Inexact(_))))
    {
        return Ok(Value::float(inexact_integer_division(cx, values, false)?.1));
    }
    let left = exact_integer_number(cx, values[0])?;
    let right = exact_integer_number(cx, values[1])?;
    if right == 0 {
        return Err(Error::plain(ErrorKind::RangeError, "division by zero"));
    }
    let result = left.checked_rem(right).ok_or_else(|| {
        Error::plain(
            ErrorKind::ImplementationRestriction,
            "exact division overflow",
        )
    })?;
    cx.integer(result)
}

pub(super) fn modulo(cx: &mut NativeContext<'_>, values: &[Value]) -> Result<Value, Error> {
    if values
        .iter()
        .any(|value| matches!(real_argument(cx, *value), Ok(Real::Inexact(_))))
    {
        return Ok(Value::float(inexact_integer_division(cx, values, true)?.1));
    }
    let left = exact_integer_number(cx, values[0])?;
    let right = exact_integer_number(cx, values[1])?;
    if right == 0 {
        return Err(Error::plain(ErrorKind::RangeError, "division by zero"));
    }
    let mut result = left.checked_rem(right).ok_or_else(|| {
        Error::plain(
            ErrorKind::ImplementationRestriction,
            "exact division overflow",
        )
    })?;
    if result != 0 && (result < 0) != (right < 0) {
        result = result
            .checked_add(right)
            .ok_or_else(|| Error::plain(ErrorKind::ImplementationRestriction, "modulo overflow"))?;
    }
    cx.integer(result)
}

pub(super) fn integer_division(
    cx: &NativeContext<'_>,
    values: &[Value],
    floor: bool,
) -> Result<(i128, i128), Error> {
    let dividend = exact_integer_number(cx, values[0])?;
    let divisor = exact_integer_number(cx, values[1])?;
    if divisor == 0 {
        return Err(Error::plain(ErrorKind::RangeError, "division by zero"));
    }
    let mut quotient = dividend.checked_div(divisor).ok_or_else(|| {
        Error::plain(
            ErrorKind::ImplementationRestriction,
            "exact division overflow",
        )
    })?;
    let remainder = dividend % divisor;
    if floor && remainder != 0 && (dividend < 0) != (divisor < 0) {
        quotient = quotient.checked_sub(1).ok_or_else(|| {
            Error::plain(
                ErrorKind::ImplementationRestriction,
                "exact division overflow",
            )
        })?;
    }
    let remainder = dividend
        .checked_sub(divisor.checked_mul(quotient).ok_or_else(|| {
            Error::plain(
                ErrorKind::ImplementationRestriction,
                "exact division overflow",
            )
        })?)
        .ok_or_else(|| {
            Error::plain(
                ErrorKind::ImplementationRestriction,
                "exact division overflow",
            )
        })?;
    Ok((quotient, remainder))
}

pub(super) fn floor_divide(
    cx: &mut NativeContext<'_>,
    values: &[Value],
) -> Result<NativeValues, Error> {
    if values
        .iter()
        .any(|value| matches!(real_argument(cx, *value), Ok(Real::Inexact(_))))
    {
        let (quotient, remainder) = inexact_integer_division(cx, values, true)?;
        return Ok(NativeValues::many([
            Value::float(quotient),
            Value::float(remainder),
        ]));
    }
    let (quotient, remainder) = integer_division(cx, values, true)?;
    Ok(NativeValues::many([
        cx.integer(quotient)?,
        cx.integer(remainder)?,
    ]))
}

pub(super) fn floor_quotient(cx: &mut NativeContext<'_>, values: &[Value]) -> Result<Value, Error> {
    if values
        .iter()
        .any(|value| matches!(real_argument(cx, *value), Ok(Real::Inexact(_))))
    {
        return Ok(Value::float(inexact_integer_division(cx, values, true)?.0));
    }
    let quotient = integer_division(cx, values, true)?.0;
    cx.integer(quotient)
}

pub(super) fn floor_remainder(
    cx: &mut NativeContext<'_>,
    values: &[Value],
) -> Result<Value, Error> {
    if values
        .iter()
        .any(|value| matches!(real_argument(cx, *value), Ok(Real::Inexact(_))))
    {
        return Ok(Value::float(inexact_integer_division(cx, values, true)?.1));
    }
    let remainder = integer_division(cx, values, true)?.1;
    cx.integer(remainder)
}

pub(super) fn truncate_divide(
    cx: &mut NativeContext<'_>,
    values: &[Value],
) -> Result<NativeValues, Error> {
    if values
        .iter()
        .any(|value| matches!(real_argument(cx, *value), Ok(Real::Inexact(_))))
    {
        let (quotient, remainder) = inexact_integer_division(cx, values, false)?;
        return Ok(NativeValues::many([
            Value::float(quotient),
            Value::float(remainder),
        ]));
    }
    let (quotient, remainder) = integer_division(cx, values, false)?;
    Ok(NativeValues::many([
        cx.integer(quotient)?,
        cx.integer(remainder)?,
    ]))
}

pub(super) fn truncate_quotient(
    cx: &mut NativeContext<'_>,
    values: &[Value],
) -> Result<Value, Error> {
    if values
        .iter()
        .any(|value| matches!(real_argument(cx, *value), Ok(Real::Inexact(_))))
    {
        return Ok(Value::float(inexact_integer_division(cx, values, false)?.0));
    }
    let quotient = integer_division(cx, values, false)?.0;
    cx.integer(quotient)
}

pub(super) fn truncate_remainder(
    cx: &mut NativeContext<'_>,
    values: &[Value],
) -> Result<Value, Error> {
    if values
        .iter()
        .any(|value| matches!(real_argument(cx, *value), Ok(Real::Inexact(_))))
    {
        return Ok(Value::float(inexact_integer_division(cx, values, false)?.1));
    }
    let remainder = integer_division(cx, values, false)?.1;
    cx.integer(remainder)
}

pub(super) fn inexact_integer_division(
    cx: &NativeContext<'_>,
    values: &[Value],
    floor: bool,
) -> Result<(f64, f64), Error> {
    let dividend = real_argument(cx, values[0])?;
    let divisor = real_argument(cx, values[1])?;
    if !dividend.is_integer() || !divisor.is_integer() {
        return Err(Error::plain(
            ErrorKind::TypeError,
            "division operands must be integers",
        ));
    }
    let dividend = crate::number::to_f64(dividend);
    let divisor = crate::number::to_f64(divisor);
    if divisor == 0.0 {
        return Err(Error::plain(ErrorKind::RangeError, "division by zero"));
    }
    let quotient = if floor {
        (dividend / divisor).floor()
    } else {
        (dividend / divisor).trunc()
    };
    Ok((quotient, dividend - divisor * quotient))
}

pub(super) fn gcd_procedure(cx: &mut NativeContext<'_>, values: &[Value]) -> Result<Value, Error> {
    let mut result = 0_u128;
    let mut inexact = false;
    for value in values {
        let (value, value_inexact) = integer_valued_number(cx, *value)?;
        inexact |= value_inexact;
        let value = value.unsigned_abs();
        result = gcd_u128(result, value);
    }
    let result = i128::try_from(result).map_err(|_| {
        Error::plain(
            ErrorKind::ImplementationRestriction,
            "gcd exceeds i128 range",
        )
    })?;
    Ok(if inexact {
        Value::float(result as f64)
    } else {
        cx.integer(result)?
    })
}

pub(super) fn lcm_procedure(cx: &mut NativeContext<'_>, values: &[Value]) -> Result<Value, Error> {
    let mut result = 1_u128;
    let mut inexact = false;
    for value in values {
        let (value, value_inexact) = integer_valued_number(cx, *value)?;
        inexact |= value_inexact;
        let value = value.unsigned_abs();
        result = if result == 0 || value == 0 {
            0
        } else {
            result
                .checked_div(gcd_u128(result, value))
                .and_then(|v| v.checked_mul(value))
                .ok_or_else(|| {
                    Error::plain(
                        ErrorKind::ImplementationRestriction,
                        "lcm exceeds i128 range",
                    )
                })?
        };
    }
    let result = i128::try_from(result).map_err(|_| {
        Error::plain(
            ErrorKind::ImplementationRestriction,
            "lcm exceeds i128 range",
        )
    })?;
    Ok(if inexact {
        Value::float(result as f64)
    } else {
        cx.integer(result)?
    })
}

pub(super) fn integer_valued_number(
    cx: &NativeContext<'_>,
    value: Value,
) -> Result<(i128, bool), Error> {
    match number_argument(cx, value)? {
        RuntimeNumber::Real(Real::ExactInteger(value)) => Ok((value, false)),
        RuntimeNumber::Real(Real::Inexact(number))
            if number.is_finite()
                && number.fract() == 0.0
                && number >= i128::MIN as f64
                && number < -(i128::MIN as f64) =>
        {
            Ok((number as i128, true))
        }
        _ => Err(type_error("integer", value, cx.heap)),
    }
}

pub(super) fn gcd_u128(mut left: u128, mut right: u128) -> u128 {
    while right != 0 {
        (left, right) = (right, left % right);
    }
    left
}

pub(super) fn square(cx: &mut NativeContext<'_>, values: &[Value]) -> Result<Value, Error> {
    let value = number_argument(cx, values[0])?;
    numeric_value(
        cx,
        crate::number::number_mul(value, value).map_err(numeric_error)?,
    )
}

pub(super) fn expt(cx: &mut NativeContext<'_>, values: &[Value]) -> Result<Value, Error> {
    let base = number_argument(cx, values[0])?;
    if let RuntimeNumber::Real(Real::ExactInteger(exponent)) = number_argument(cx, values[1])?
        && exponent >= 0
    {
        // Exponentiation by squaring keeps the native call logarithmic in the
        // exponent. A linear loop would let guest code monopolize the engine
        // without reaching VM fuel or interrupt safe points.
        let mut exponent = exponent as u128;
        let mut factor = base;
        let mut result = RuntimeNumber::Real(Real::ExactInteger(1));
        while exponent != 0 {
            if exponent & 1 != 0 {
                result = crate::number::number_mul(result, factor).map_err(numeric_error)?;
            }
            exponent >>= 1;
            if exponent != 0 {
                factor = crate::number::number_mul(factor, factor).map_err(numeric_error)?;
            }
        }
        return numeric_value(cx, result);
    }
    let exponent = crate::number::to_f64(real_argument(cx, values[1])?);
    let base = crate::number::to_f64(real_argument(cx, values[0])?);
    numeric_value(cx, RuntimeNumber::Real(Real::Inexact(base.powf(exponent))))
}

pub(super) fn exact_integer_sqrt(
    cx: &mut NativeContext<'_>,
    values: &[Value],
) -> Result<NativeValues, Error> {
    let value = exact_integer_number(cx, values[0])?;
    if value < 0 {
        return Err(Error::plain(
            ErrorKind::RangeError,
            "exact-integer-sqrt requires a non-negative integer",
        ));
    }
    let root = integer_sqrt(value as u128) as i128;
    Ok(NativeValues::many([
        cx.integer(root)?,
        cx.integer(value - root * root)?,
    ]))
}

pub(super) fn integer_sqrt(value: u128) -> u128 {
    if value < 2 {
        return value;
    }
    let mut current = 1_u128 << value.ilog2().div_ceil(2);
    loop {
        let next = (current + value / current) / 2;
        if next >= current {
            return current;
        }
        current = next;
    }
}

pub(super) fn sqrt(cx: &mut NativeContext<'_>, values: &[Value]) -> Result<Value, Error> {
    let number = number_argument(cx, values[0])?;
    if let RuntimeNumber::Complex { real, imaginary } = number {
        let real = crate::number::to_f64(real);
        let imaginary = crate::number::to_f64(imaginary);
        let magnitude = real.hypot(imaginary);
        let result_real = ((magnitude + real) / 2.0).sqrt();
        let imaginary_sign = if imaginary == 0.0 { 1.0 } else { imaginary };
        let result_imaginary = ((magnitude - real) / 2.0).sqrt().copysign(imaginary_sign);
        return numeric_value(
            cx,
            RuntimeNumber::Complex {
                real: Real::Inexact(result_real),
                imaginary: Real::Inexact(result_imaginary),
            },
        );
    }
    let RuntimeNumber::Real(value) = number else {
        unreachable!()
    };
    if crate::number::to_f64(value) < 0.0 {
        return numeric_value(
            cx,
            RuntimeNumber::complex(
                Real::Inexact(0.0),
                Real::Inexact((-crate::number::to_f64(value)).sqrt()),
            ),
        );
    }
    numeric_value(
        cx,
        RuntimeNumber::Real(Real::Inexact(crate::number::to_f64(value).sqrt())),
    )
}

pub(super) fn unary_transcendental(
    cx: &mut NativeContext<'_>,
    value: Value,
    operation: fn(f64) -> f64,
) -> Result<Value, Error> {
    numeric_value(
        cx,
        RuntimeNumber::Real(Real::Inexact(operation(crate::number::to_f64(
            real_argument(cx, value)?,
        )))),
    )
}

pub(super) fn sin(cx: &mut NativeContext<'_>, values: &[Value]) -> Result<Value, Error> {
    unary_transcendental(cx, values[0], f64::sin)
}

pub(super) fn cos(cx: &mut NativeContext<'_>, values: &[Value]) -> Result<Value, Error> {
    unary_transcendental(cx, values[0], f64::cos)
}

pub(super) fn tan(cx: &mut NativeContext<'_>, values: &[Value]) -> Result<Value, Error> {
    unary_transcendental(cx, values[0], f64::tan)
}

pub(super) fn exp(cx: &mut NativeContext<'_>, values: &[Value]) -> Result<Value, Error> {
    unary_transcendental(cx, values[0], f64::exp)
}

pub(super) fn log(cx: &mut NativeContext<'_>, values: &[Value]) -> Result<Value, Error> {
    if values.len() == 1 {
        unary_transcendental(cx, values[0], f64::ln)
    } else {
        let value = crate::number::to_f64(real_argument(cx, values[0])?);
        let base = crate::number::to_f64(real_argument(cx, values[1])?);
        numeric_value(
            cx,
            RuntimeNumber::Real(Real::Inexact(value.ln() / base.ln())),
        )
    }
}

pub(super) fn asin(cx: &mut NativeContext<'_>, values: &[Value]) -> Result<Value, Error> {
    unary_transcendental(cx, values[0], f64::asin)
}

pub(super) fn acos(cx: &mut NativeContext<'_>, values: &[Value]) -> Result<Value, Error> {
    unary_transcendental(cx, values[0], f64::acos)
}

pub(super) fn atan(cx: &mut NativeContext<'_>, values: &[Value]) -> Result<Value, Error> {
    let y = crate::number::to_f64(real_argument(cx, values[0])?);
    let result = if values.len() == 2 {
        y.atan2(crate::number::to_f64(real_argument(cx, values[1])?))
    } else {
        y.atan()
    };
    numeric_value(cx, RuntimeNumber::Real(Real::Inexact(result)))
}
