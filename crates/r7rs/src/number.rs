use crate::ExactRational;

/// A real component of a parsed Scheme number.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Real {
    /// An exact integer in the implementation's supported range.
    ExactInteger(i128),
    /// An exact normalized rational in the implementation's supported range.
    ExactRational(ExactRational),
    /// An IEEE-754 inexact real, including infinities, NaNs, and signed zero.
    Inexact(f64),
}

/// A parsed Scheme number.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Number {
    /// A real number.
    Real(Real),
    /// A rectangular complex number.
    Rectangular {
        /// The real component.
        real: Real,
        /// The imaginary component.
        imaginary: Real,
    },
    /// A polar complex literal retained without premature rounding.
    Polar {
        /// The magnitude component.
        magnitude: Real,
        /// The angle component in radians.
        angle: Real,
    },
}

impl Number {
    pub(crate) fn real(real: Real) -> Self {
        Self::Real(real)
    }
}

/// The normalized representation used by the evaluator. Polar notation is a
/// reader convenience; evaluated values are always real or rectangular.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum RuntimeNumber {
    Real(Real),
    Complex { real: Real, imaginary: Real },
}

impl RuntimeNumber {
    pub(crate) fn from_literal(value: Number) -> Self {
        match value {
            Number::Real(real) => Self::Real(real),
            Number::Rectangular { real, imaginary } => Self::complex(real, imaginary),
            Number::Polar { magnitude, angle } => {
                let magnitude = to_f64(magnitude);
                let angle = to_f64(angle);
                Self::complex(
                    Real::Inexact(magnitude * angle.cos()),
                    Real::Inexact(magnitude * angle.sin()),
                )
            }
        }
    }

    pub(crate) fn complex(real: Real, imaginary: Real) -> Self {
        if matches!(imaginary, Real::ExactInteger(0))
            || matches!(imaginary, Real::ExactRational(value) if value.numerator() == 0)
        {
            Self::Real(real)
        } else {
            Self::Complex { real, imaginary }
        }
    }

    pub(crate) fn components(self) -> (Real, Real) {
        match self {
            Self::Real(real) => (real, Real::ExactInteger(0)),
            Self::Complex { real, imaginary } => (real, imaginary),
        }
    }

    pub(crate) fn is_exact(self) -> bool {
        let (real, imaginary) = self.components();
        real.is_exact() && imaginary.is_exact()
    }

    pub(crate) fn is_real(self) -> bool {
        matches!(self, Self::Real(_))
    }
    pub(crate) fn is_rational(self) -> bool {
        self.is_real() && self.components().0.is_rational()
    }
    pub(crate) fn is_integer(self) -> bool {
        self.is_real() && self.components().0.is_integer()
    }
    pub(crate) fn is_finite(self) -> bool {
        let (real, imaginary) = self.components();
        real.is_finite() && imaginary.is_finite()
    }
    pub(crate) fn is_infinite(self) -> bool {
        let (real, imaginary) = self.components();
        real.is_infinite() || imaginary.is_infinite()
    }
    pub(crate) fn is_nan(self) -> bool {
        let (real, imaginary) = self.components();
        real.is_nan() || imaginary.is_nan()
    }

    pub(crate) fn inexact(self) -> Self {
        match self {
            Self::Real(real) => Self::Real(real.inexact()),
            Self::Complex { real, imaginary } => Self::Complex {
                real: real.inexact(),
                imaginary: imaginary.inexact(),
            },
        }
    }

    pub(crate) fn exact(self) -> Result<Self, String> {
        match self {
            Self::Real(real) => Ok(Self::Real(real.exact()?)),
            Self::Complex { real, imaginary } => {
                Ok(Self::complex(real.exact()?, imaginary.exact()?))
            }
        }
    }
}

impl Real {
    pub(crate) fn is_exact(self) -> bool {
        !matches!(self, Self::Inexact(_))
    }
    pub(crate) fn is_rational(self) -> bool {
        !matches!(self, Self::Inexact(value) if !value.is_finite())
    }
    pub(crate) fn is_integer(self) -> bool {
        match self {
            Self::ExactInteger(_) => true,
            Self::ExactRational(value) => value.denominator() == 1,
            Self::Inexact(value) => value.is_finite() && value.fract() == 0.0,
        }
    }
    pub(crate) fn is_finite(self) -> bool {
        !matches!(self, Self::Inexact(value) if !value.is_finite())
    }
    pub(crate) fn is_infinite(self) -> bool {
        matches!(self, Self::Inexact(value) if value.is_infinite())
    }
    pub(crate) fn is_nan(self) -> bool {
        matches!(self, Self::Inexact(value) if value.is_nan())
    }
    pub(crate) fn inexact(self) -> Self {
        inexact(self)
    }
    pub(crate) fn exact(self) -> Result<Self, String> {
        match self {
            Self::ExactInteger(_) | Self::ExactRational(_) => Ok(self),
            Self::Inexact(value) if value.is_finite() => exact_float(value),
            Self::Inexact(_) => Err("cannot convert infinity or NaN to an exact number".into()),
        }
    }
}

fn exact_float(value: f64) -> Result<Real, String> {
    if value == 0.0 {
        return Ok(Real::ExactInteger(0));
    }
    let bits = value.to_bits();
    let negative = bits >> 63 != 0;
    let exponent_bits = ((bits >> 52) & 0x7FF) as i32;
    let fraction = bits & ((1_u64 << 52) - 1);
    let (mut significand, mut exponent) = if exponent_bits == 0 {
        (fraction as u128, -1022 - 52)
    } else {
        (
            ((1_u64 << 52) | fraction) as u128,
            exponent_bits - 1023 - 52,
        )
    };
    if exponent < 0 {
        let removable = significand.trailing_zeros().min((-exponent) as u32);
        significand >>= removable;
        exponent += removable as i32;
    }
    if exponent >= 0 {
        let factor = 1_u128
            .checked_shl(exponent as u32)
            .ok_or("exact floating-point value exceeds the supported range")?;
        let magnitude = significand
            .checked_mul(factor)
            .ok_or("exact floating-point value exceeds the supported range")?;
        checked_rational(signed_magnitude(magnitude, negative)?, 1)
    } else {
        let numerator = signed_magnitude(significand, negative)?;
        let denominator = 1_u128
            .checked_shl((-exponent) as u32)
            .and_then(|value| i128::try_from(value).ok())
            .ok_or("exact floating-point value exceeds the supported range")?;
        checked_rational(numerator, denominator)
    }
}

fn signed_magnitude(magnitude: u128, negative: bool) -> Result<i128, String> {
    if negative && magnitude == 1_u128 << 127 {
        return Ok(i128::MIN);
    }
    let value = i128::try_from(magnitude)
        .map_err(|_| "exact floating-point value exceeds the supported range")?;
    Ok(if negative { -value } else { value })
}

pub(crate) fn to_f64(value: Real) -> f64 {
    match value {
        Real::ExactInteger(value) => value as f64,
        Real::ExactRational(value) => value.numerator() as f64 / value.denominator() as f64,
        Real::Inexact(value) => value,
    }
}

pub(crate) fn is_zero(value: Real) -> bool {
    match value {
        Real::ExactInteger(value) => value == 0,
        Real::ExactRational(value) => value.numerator() == 0,
        Real::Inexact(value) => value == 0.0,
    }
}

fn fraction(value: Real) -> Option<(i128, i128)> {
    match value {
        Real::ExactInteger(value) => Some((value, 1)),
        Real::ExactRational(value) => Some((
            i128::from(value.numerator()),
            i128::from(value.denominator()),
        )),
        Real::Inexact(_) => None,
    }
}

fn checked_rational(numerator: i128, denominator: i128) -> Result<Real, String> {
    if denominator == 0 {
        return Err("division by exact zero".into());
    }
    let (numerator, denominator) = if denominator < 0 {
        (
            numerator
                .checked_neg()
                .ok_or("exact numeric result exceeds the supported i128 range")?,
            denominator
                .checked_neg()
                .ok_or("exact numeric result exceeds the supported i128 range")?,
        )
    } else {
        (numerator, denominator)
    };
    let divisor = gcd128(numerator.unsigned_abs(), denominator.unsigned_abs()) as i128;
    let numerator = numerator / divisor;
    let denominator = denominator / divisor;
    if denominator == 1 {
        return Ok(Real::ExactInteger(numerator));
    }
    Ok(Real::ExactRational(ExactRational::new(
        i64::try_from(numerator)
            .map_err(|_| "exact rational numerator exceeds the supported i64 range")?,
        i64::try_from(denominator)
            .map_err(|_| "exact rational denominator exceeds the supported i64 range")?,
    )))
}

fn gcd128(mut left: u128, mut right: u128) -> u128 {
    while right != 0 {
        (left, right) = (right, left % right);
    }
    left
}

pub(crate) fn real_add(left: Real, right: Real) -> Result<Real, String> {
    if let (Real::ExactInteger(left), Real::ExactInteger(right)) = (left, right) {
        return left
            .checked_add(right)
            .map(Real::ExactInteger)
            .ok_or_else(|| "exact numeric result exceeds the supported i128 range".into());
    }
    match (fraction(left), fraction(right)) {
        (Some((a, b)), Some((c, d))) => checked_rational(
            a.checked_mul(d)
                .and_then(|left| c.checked_mul(b).and_then(|right| left.checked_add(right)))
                .ok_or("exact numeric result exceeds the supported i128 range")?,
            b.checked_mul(d)
                .ok_or("exact numeric result exceeds the supported i128 range")?,
        ),
        _ => Ok(Real::Inexact(to_f64(left) + to_f64(right))),
    }
}
pub(crate) fn real_sub(left: Real, right: Real) -> Result<Real, String> {
    if let (Real::ExactInteger(left), Real::ExactInteger(right)) = (left, right) {
        return left
            .checked_sub(right)
            .map(Real::ExactInteger)
            .ok_or_else(|| "exact numeric result exceeds the supported i128 range".into());
    }
    real_add(left, real_neg(right)?)
}
pub(crate) fn real_neg(value: Real) -> Result<Real, String> {
    match value {
        Real::ExactInteger(value) => value
            .checked_neg()
            .map(Real::ExactInteger)
            .ok_or_else(|| "exact numeric result exceeds the supported i128 range".into()),
        Real::ExactRational(value) => {
            checked_rational(-(value.numerator() as i128), value.denominator() as i128)
        }
        Real::Inexact(value) => Ok(Real::Inexact(-value)),
    }
}
pub(crate) fn real_mul(left: Real, right: Real) -> Result<Real, String> {
    if let (Real::ExactInteger(left), Real::ExactInteger(right)) = (left, right) {
        return left
            .checked_mul(right)
            .map(Real::ExactInteger)
            .ok_or_else(|| "exact numeric result exceeds the supported i128 range".into());
    }
    match (fraction(left), fraction(right)) {
        (Some((a, b)), Some((c, d))) => {
            let left_divisor = gcd128(a.unsigned_abs(), d as u128) as i128;
            let right_divisor = gcd128(c.unsigned_abs(), b as u128) as i128;
            checked_rational(
                (a / left_divisor)
                    .checked_mul(c / right_divisor)
                    .ok_or("exact numeric result exceeds the supported i128 range")?,
                (b / right_divisor)
                    .checked_mul(d / left_divisor)
                    .ok_or("exact numeric result exceeds the supported i128 range")?,
            )
        }
        _ => Ok(Real::Inexact(to_f64(left) * to_f64(right))),
    }
}
pub(crate) fn real_div(left: Real, right: Real) -> Result<Real, String> {
    match (fraction(left), fraction(right)) {
        (_, Some((0, _))) => Err("division by exact zero".into()),
        (Some((a, b)), Some((c, d))) => {
            let numerator_divisor = gcd128(a.unsigned_abs(), c.unsigned_abs()) as i128;
            let denominator_divisor = gcd128(d as u128, b as u128) as i128;
            checked_rational(
                (a / numerator_divisor)
                    .checked_mul(d / denominator_divisor)
                    .ok_or("exact numeric result exceeds the supported i128 range")?,
                (b / denominator_divisor)
                    .checked_mul(c / numerator_divisor)
                    .ok_or("exact numeric result exceeds the supported i128 range")?,
            )
        }
        _ => Ok(Real::Inexact(to_f64(left) / to_f64(right))),
    }
}

pub(crate) fn number_add(
    left: RuntimeNumber,
    right: RuntimeNumber,
) -> Result<RuntimeNumber, String> {
    if let (RuntimeNumber::Real(left), RuntimeNumber::Real(right)) = (left, right) {
        return Ok(RuntimeNumber::Real(real_add(left, right)?));
    }
    let (a, b) = left.components();
    let (c, d) = right.components();
    Ok(RuntimeNumber::complex(real_add(a, c)?, real_add(b, d)?))
}
pub(crate) fn number_neg(value: RuntimeNumber) -> Result<RuntimeNumber, String> {
    if let RuntimeNumber::Real(value) = value {
        return Ok(RuntimeNumber::Real(real_neg(value)?));
    }
    let (real, imaginary) = value.components();
    Ok(RuntimeNumber::complex(
        real_neg(real)?,
        real_neg(imaginary)?,
    ))
}
pub(crate) fn number_sub(
    left: RuntimeNumber,
    right: RuntimeNumber,
) -> Result<RuntimeNumber, String> {
    if let (RuntimeNumber::Real(left), RuntimeNumber::Real(right)) = (left, right) {
        return Ok(RuntimeNumber::Real(real_sub(left, right)?));
    }
    number_add(left, number_neg(right)?)
}
pub(crate) fn number_mul(
    left: RuntimeNumber,
    right: RuntimeNumber,
) -> Result<RuntimeNumber, String> {
    if let (RuntimeNumber::Real(left), RuntimeNumber::Real(right)) = (left, right) {
        return Ok(RuntimeNumber::Real(real_mul(left, right)?));
    }
    let (a, b) = left.components();
    let (c, d) = right.components();
    Ok(RuntimeNumber::complex(
        real_sub(real_mul(a, c)?, real_mul(b, d)?)?,
        real_add(real_mul(a, d)?, real_mul(b, c)?)?,
    ))
}
pub(crate) fn number_div(
    left: RuntimeNumber,
    right: RuntimeNumber,
) -> Result<RuntimeNumber, String> {
    if let (RuntimeNumber::Real(left), RuntimeNumber::Real(right)) = (left, right) {
        return Ok(RuntimeNumber::Real(real_div(left, right)?));
    }
    let (a, b) = left.components();
    let (c, d) = right.components();
    let denominator = real_add(real_mul(c, c)?, real_mul(d, d)?)?;
    Ok(RuntimeNumber::complex(
        real_div(real_add(real_mul(a, c)?, real_mul(b, d)?)?, denominator)?,
        real_div(real_sub(real_mul(b, c)?, real_mul(a, d)?)?, denominator)?,
    ))
}

pub(crate) fn number_equal(left: RuntimeNumber, right: RuntimeNumber) -> bool {
    let (a, b) = left.components();
    let (c, d) = right.components();
    real_equal(a, c) && real_equal(b, d)
}
pub(crate) fn real_equal(left: Real, right: Real) -> bool {
    if let (Real::ExactInteger(left), Real::ExactInteger(right)) = (left, right) {
        return left == right;
    }
    if let (Real::Inexact(left), Real::Inexact(right)) = (left, right) {
        return left == right;
    }
    match (comparison_fraction(left), comparison_fraction(right)) {
        (Some(left), Some(right)) => compare_fractions(left, right).is_eq(),
        _ => to_f64(left) == to_f64(right),
    }
}

/// R7RS `eqv?` for two numbers.
///
/// Unlike [`number_equal`] (which implements the coercing `=`), `eqv?` returns
/// `#t` only when the operands are operationally indistinguishable: they must
/// share exactness, exact operands must be numerically equal, and inexact
/// operands must be bit-for-bit identical (so `0.0` and `-0.0` are distinct)
/// with all NaNs forming one equivalence class regardless of their bits.
/// Complex numbers are compared component-wise under the same rule. In
/// particular `(eqv? 5 5.0)` is `#f` because the operands differ in exactness.
pub(crate) fn number_eqv(left: RuntimeNumber, right: RuntimeNumber) -> bool {
    let (a, b) = left.components();
    let (c, d) = right.components();
    real_eqv(a, c) && real_eqv(b, d)
}

fn real_eqv(left: Real, right: Real) -> bool {
    match (left, right) {
        // Both inexact: operationally identical iff bit-for-bit identical,
        // except that all NaNs are one operational equivalence class. The VM's
        // arithmetic fast paths skip per-result NaN canonicalization, so two
        // NaNs may carry different sign/payload bits yet must stay `eqv?` (no
        // R7RS-observable procedure distinguishes them).
        (Real::Inexact(left), Real::Inexact(right)) => {
            (left.is_nan() && right.is_nan()) || left.to_bits() == right.to_bits()
        }
        // Exactly one inexact: exactness differs, so never `eqv?`.
        (Real::Inexact(_), _) | (_, Real::Inexact(_)) => false,
        // Both exact: exact reals are canonical, so `=` coincides with `eqv?`.
        _ => real_equal(left, right),
    }
}

pub(crate) fn real_compare(left: Real, right: Real) -> Option<std::cmp::Ordering> {
    if let (Real::ExactInteger(left), Real::ExactInteger(right)) = (left, right) {
        return Some(left.cmp(&right));
    }
    if let (Real::Inexact(left), Real::Inexact(right)) = (left, right) {
        return left.partial_cmp(&right);
    }
    match (comparison_fraction(left), comparison_fraction(right)) {
        (Some(left), Some(right)) => Some(compare_fractions(left, right)),
        _ => to_f64(left).partial_cmp(&to_f64(right)),
    }
}

fn comparison_fraction(value: Real) -> Option<(i128, i128)> {
    fraction(value).or_else(|| value.exact().ok().and_then(fraction))
}

fn compare_fractions(left: (i128, i128), right: (i128, i128)) -> std::cmp::Ordering {
    let (left_numerator, left_denominator) = left;
    let (right_numerator, right_denominator) = right;
    match (left_numerator.is_negative(), right_numerator.is_negative()) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        (negative, _) => {
            let ordering = compare_unsigned_fractions(
                left_numerator.unsigned_abs(),
                left_denominator as u128,
                right_numerator.unsigned_abs(),
                right_denominator as u128,
            );
            if negative {
                ordering.reverse()
            } else {
                ordering
            }
        }
    }
}

fn compare_unsigned_fractions(
    mut left_numerator: u128,
    mut left_denominator: u128,
    mut right_numerator: u128,
    mut right_denominator: u128,
) -> std::cmp::Ordering {
    let mut reverse = false;
    loop {
        let left_quotient = left_numerator / left_denominator;
        let right_quotient = right_numerator / right_denominator;
        let ordering = left_quotient.cmp(&right_quotient);
        if !ordering.is_eq() {
            return if reverse {
                ordering.reverse()
            } else {
                ordering
            };
        }
        let left_remainder = left_numerator % left_denominator;
        let right_remainder = right_numerator % right_denominator;
        match (left_remainder == 0, right_remainder == 0) {
            (true, true) => return std::cmp::Ordering::Equal,
            (true, false) => {
                return if reverse {
                    std::cmp::Ordering::Greater
                } else {
                    std::cmp::Ordering::Less
                };
            }
            (false, true) => {
                return if reverse {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                };
            }
            (false, false) => {
                (left_numerator, left_denominator) = (left_denominator, left_remainder);
                (right_numerator, right_denominator) = (right_denominator, right_remainder);
                reverse = !reverse;
            }
        }
    }
}

pub(crate) fn parse(input: &str) -> Option<Result<Number, String>> {
    let lower = input.to_ascii_lowercase();
    let mut rest = lower.as_str();
    let mut radix = 10_u32;
    let mut has_radix = false;
    let mut exactness = None;
    for _ in 0..2 {
        if rest.len() < 2 || !rest.starts_with('#') {
            break;
        }
        let prefix = rest.as_bytes()[1] as char;
        match prefix {
            'b' | 'o' | 'd' | 'x' if !has_radix => {
                radix = match prefix {
                    'b' => 2,
                    'o' => 8,
                    'd' => 10,
                    _ => 16,
                };
                has_radix = true;
                rest = &rest[2..];
            }
            'e' | 'i' if exactness.is_none() => {
                exactness = Some(prefix == 'e');
                rest = &rest[2..];
            }
            _ => return Some(Err("invalid numeric prefix".into())),
        }
    }
    if rest.starts_with('#') {
        return Some(Err("duplicate or invalid numeric prefix".into()));
    }
    let unsigned_decimal = |text: &str| {
        text.starts_with(|c: char| c.is_ascii_digit())
            || text.starts_with('.') && text[1..].starts_with(|c: char| c.is_ascii_digit())
    };
    let looks_numeric = input.starts_with('#')
        || unsigned_decimal(rest)
        || rest.strip_prefix(['+', '-']).is_some_and(unsigned_decimal)
        || matches!(rest, "+i" | "-i")
        || rest.starts_with("+inf.")
        || rest.starts_with("-inf.")
        || rest.starts_with("+nan.")
        || rest.starts_with("-nan.");
    if !looks_numeric {
        return None;
    }
    let result = (|| -> Result<Number, String> {
        if let Some(index) = rest.find('@') {
            if rest[index + 1..].contains('@') {
                Err("multiple polar separators".into())
            } else {
                let magnitude = parse_real(&rest[..index], radix, exactness)?;
                let angle = parse_real(&rest[index + 1..], radix, exactness)?;
                let (magnitude, angle) = coerce_components(magnitude, angle, exactness);
                Ok(Number::Polar { magnitude, angle })
            }
        } else if let Some(body) = rest.strip_suffix('i') {
            parse_rectangular(body, radix, exactness)
        } else {
            Ok(Number::real(parse_real(rest, radix, exactness)?))
        }
    })();
    Some(result)
}

fn parse_rectangular(body: &str, radix: u32, exactness: Option<bool>) -> Result<Number, String> {
    if body == "+" || body == "-" || body.is_empty() {
        let imaginary = parse_real(if body == "-" { "-1" } else { "1" }, radix, exactness)?;
        return Ok(Number::Rectangular {
            real: zero_like(&imaginary),
            imaginary,
        });
    }
    let mut split = None;
    for (index, c) in body.char_indices() {
        if index != 0
            && (c == '+' || c == '-')
            && !matches!(body.as_bytes().get(index.saturating_sub(1)), Some(b'e'))
        {
            split = Some(index);
        }
    }
    let (real_text, imaginary_text) = match split {
        Some(index) => (&body[..index], &body[index..]),
        None => ("0", body),
    };
    let real = parse_real(real_text, radix, exactness)?;
    let imaginary = parse_real(
        if imaginary_text == "+" {
            "1"
        } else if imaginary_text == "-" {
            "-1"
        } else {
            imaginary_text
        },
        radix,
        exactness,
    )?;
    Ok(Number::Rectangular { real, imaginary })
}

fn coerce_components(left: Real, right: Real, forced_exact: Option<bool>) -> (Real, Real) {
    if forced_exact.is_none()
        && (matches!(left, Real::Inexact(_)) || matches!(right, Real::Inexact(_)))
    {
        (inexact(left), inexact(right))
    } else {
        (left, right)
    }
}

fn inexact(value: Real) -> Real {
    match value {
        Real::ExactInteger(value) => Real::Inexact(value as f64),
        Real::ExactRational(value) => {
            Real::Inexact(value.numerator() as f64 / value.denominator() as f64)
        }
        value => value,
    }
}

fn zero_like(value: &Real) -> Real {
    match value {
        Real::Inexact(_) => Real::Inexact(0.0),
        _ => Real::ExactInteger(0),
    }
}

fn parse_real(input: &str, radix: u32, forced_exact: Option<bool>) -> Result<Real, String> {
    if matches!(input, "+inf.0" | "-inf.0" | "+nan.0" | "-nan.0") {
        if forced_exact == Some(true) {
            return Err("an exact number cannot be infinite or NaN".into());
        }
        return Ok(Real::Inexact(match input {
            "+inf.0" => f64::INFINITY,
            "-inf.0" => f64::NEG_INFINITY,
            "+nan.0" => f64::NAN,
            _ => -f64::NAN,
        }));
    }
    let normalized;
    let input = if radix == 10 && input.chars().any(|c| matches!(c, 's' | 'f' | 'd' | 'l')) {
        normalized = input
            .chars()
            .map(|c| {
                if matches!(c, 's' | 'f' | 'd' | 'l') {
                    'e'
                } else {
                    c
                }
            })
            .collect::<String>();
        normalized.as_str()
    } else {
        input
    };
    let has_decimal_point = input.contains('.');
    if radix != 10 && has_decimal_point {
        return Err("decimal notation requires radix 10".into());
    }
    // `e` is an ordinary digit in radices that include it.
    let decimal = has_decimal_point || (radix == 10 && input.contains('e'));
    let exact = forced_exact.unwrap_or(!decimal);
    if input.matches('/').count() > 1 {
        return Err("invalid rational literal".into());
    }
    if let Some((numerator, denominator)) = input.split_once('/') {
        if decimal {
            return Err("a rational literal cannot contain a decimal point".into());
        }
        let numerator = parse_integer(numerator, radix)?;
        let denominator = parse_unsigned(denominator, radix)?;
        if denominator == 0 {
            return Err("a rational denominator cannot be zero".into());
        }
        if exact {
            return rational(numerator, denominator);
        }
        return Ok(Real::Inexact((numerator as f64) / (denominator as f64)));
    }
    if exact && decimal {
        return exact_decimal(input);
    }
    if exact {
        return Ok(Real::ExactInteger(parse_integer(input, radix)?));
    }
    let value = if decimal {
        input
            .parse::<f64>()
            .map_err(|_| "invalid inexact decimal literal")?
    } else {
        parse_integer(input, radix)? as f64
    };
    Ok(Real::Inexact(value))
}

fn parse_integer(input: &str, radix: u32) -> Result<i128, String> {
    if input.is_empty() {
        return Err("missing digits".into());
    }
    i128::from_str_radix(input, radix)
        .map_err(|_| "exact integer exceeds the supported i128 range".into())
}

fn parse_unsigned(input: &str, radix: u32) -> Result<i64, String> {
    if input.starts_with(['+', '-']) {
        return Err("a rational denominator must be unsigned".into());
    }
    i64::try_from(parse_integer(input, radix)?)
        .map_err(|_| "exact rational denominator exceeds the supported i64 range".into())
}

pub(crate) fn rational(numerator: i128, denominator: i64) -> Result<Real, String> {
    checked_rational(numerator, i128::from(denominator))
}

fn exact_decimal(input: &str) -> Result<Real, String> {
    let (mantissa, exponent) = match input.find('e') {
        Some(index) => (
            &input[..index],
            input[index + 1..]
                .parse::<i32>()
                .map_err(|_| "invalid exponent")?,
        ),
        None => (input, 0),
    };
    let (negative, mantissa) = match mantissa.strip_prefix('-') {
        Some(value) => (true, value),
        None => (false, mantissa.strip_prefix('+').unwrap_or(mantissa)),
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if whole.is_empty() && fraction.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return Err("invalid exact decimal literal".into());
    }
    let digits = format!("{}{whole}{fraction}", if negative { "-" } else { "" });
    let mut numerator = digits
        .parse::<i128>()
        .map_err(|_| "exact decimal exceeds the supported range")?;
    let scale =
        exponent - i32::try_from(fraction.len()).map_err(|_| "decimal scale is too large")?;
    if scale >= 0 {
        numerator = numerator
            .checked_mul(
                10_i128
                    .checked_pow(scale as u32)
                    .ok_or("exact decimal exceeds the supported range")?,
            )
            .ok_or("exact decimal exceeds the supported range")?;
        return Ok(Real::ExactInteger(numerator));
    }
    let denominator = 10_i128
        .checked_pow((-scale) as u32)
        .ok_or("exact decimal exceeds the supported range")?;
    checked_rational(numerator, denominator)
}
