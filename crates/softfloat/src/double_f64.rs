//! Double-double float type.
//!
//! A [`DoubleF64`] represents a value as the unevaluated sum of two f64 values (high, low),
//! where `|low| ≤ 0.5 ulp(high)`. This gives approximately 106 bits of significand precision
//! (2 × 53) using only hardware f64 arithmetic.
//!
//! The key algorithms are Dekker's error-free transformations:
//! - TwoSum / QuickTwoSum: compute `a + b` as an exact (high, low) pair.
//! - TwoProd (via Dekker split): compute `a × b` as an exact (high, low) pair without FMA.
//!
//! Polynomial coefficient arrays ([`EXP_ARR`], [`LN_ARR`], [`SIN_ARR`], [`COS_ARR`],
//! [`ATAN_ARR`]) are computed at compile time from their mathematical definitions using
//! const double-double arithmetic with Dekker's error-free transformations.
//!
//! References:
//! - T.J. Dekker, "A Floating-Point Technique for Extending the Available Precision" (1971)
//! - D.E. Knuth, "The Art of Computer Programming", Vol. 2

use core::cmp::Ordering;

use crate::{ExceptionFlags, Fp80, Precision, RoundingMode, shift_right_sticky};

/// The exponent bias of f64.
const F64_EXPONENT_BIAS: i32 = 1023;

#[derive(Clone, Copy, Debug)]
pub(crate) struct DoubleF64 {
    high: f64,
    low: f64,
}

/// Dekker's fast two-sum. Requires |a| >= |b|.
#[inline(always)]
const fn quick_two_sum(a: f64, b: f64) -> DoubleF64 {
    let high = a + b;
    let low = b - (high - a);
    DoubleF64 { high, low }
}

/// Knuth's two-sum. No requirement on input magnitudes.
#[inline(always)]
const fn two_sum(a: f64, b: f64) -> DoubleF64 {
    let high = a + b;
    let v = high - a;
    let low = (a - (high - v)) + (b - v);
    DoubleF64 { high, low }
}

/// Dekker split: splits a into high (26 bits) and low (27 bits) halves.
#[inline(always)]
const fn split(a: f64) -> (f64, f64) {
    let c = 134217729.0_f64; // 2^27 + 1
    let t = c * a;
    let hi = t - (t - a);
    let lo = a - hi;
    (hi, lo)
}

/// Dekker's two-product via split (no FMA required, const-compatible).
#[inline(always)]
const fn two_prod(a: f64, b: f64) -> DoubleF64 {
    let p = a * b;
    let (a_hi, a_lo) = split(a);
    let (b_hi, b_lo) = split(b);
    let err = ((a_hi * b_hi - p) + a_hi * b_lo + a_lo * b_hi) + a_lo * b_lo;
    DoubleF64 { high: p, low: err }
}

pub(crate) const fn dd_add(a: DoubleF64, b: DoubleF64) -> DoubleF64 {
    let s = two_sum(a.high, b.high);
    let e = two_sum(a.low, b.low);
    let c = quick_two_sum(s.high, s.low + e.high);
    let d = quick_two_sum(c.low, e.low);
    quick_two_sum(c.high, d.high + d.low)
}

pub(crate) const fn dd_sub(a: DoubleF64, b: DoubleF64) -> DoubleF64 {
    dd_add(
        a,
        DoubleF64 {
            high: -b.high,
            low: -b.low,
        },
    )
}

pub(crate) const fn dd_mul(a: DoubleF64, b: DoubleF64) -> DoubleF64 {
    let p = two_prod(a.high, b.high);
    let cross = a.high * b.low + a.low * b.high;
    let t = quick_two_sum(p.high, p.low + cross);
    quick_two_sum(t.high, t.low)
}

pub(crate) const fn dd_div(a: DoubleF64, b: DoubleF64) -> DoubleF64 {
    let q1 = a.high / b.high;
    // residual = a - b * q1
    let p = two_prod(q1, b.high);
    let r_high = a.high - p.high;
    let r = dd_sub(
        DoubleF64 {
            high: r_high,
            low: a.low - p.low,
        },
        DoubleF64 {
            high: 0.0,
            low: q1 * b.low,
        },
    );
    let q2 = r.high / b.high;
    quick_two_sum(q1, q2)
}

const fn dd_negate(a: DoubleF64) -> DoubleF64 {
    DoubleF64 {
        high: -a.high,
        low: -a.low,
    }
}

impl core::ops::Add for DoubleF64 {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self {
        dd_add(self, rhs)
    }
}

impl core::ops::Sub for DoubleF64 {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self {
        dd_sub(self, rhs)
    }
}

impl core::ops::Mul for DoubleF64 {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: Self) -> Self {
        dd_mul(self, rhs)
    }
}

impl core::ops::Div for DoubleF64 {
    type Output = Self;
    #[inline]
    fn div(self, rhs: Self) -> Self {
        dd_div(self, rhs)
    }
}

impl DoubleF64 {
    pub(crate) const ZERO: DoubleF64 = DoubleF64 {
        high: 0.0,
        low: 0.0,
    };

    pub(crate) const ONE: DoubleF64 = DoubleF64 {
        high: 1.0,
        low: 0.0,
    };

    #[inline]
    pub(crate) const fn from_parts(high: f64, low: f64) -> Self {
        Self { high, low }
    }

    #[inline]
    pub(crate) const fn new(value: f64) -> Self {
        Self {
            high: value,
            low: 0.0,
        }
    }

    #[inline]
    pub(crate) fn is_zero(self) -> bool {
        self.high == 0.0 && self.low == 0.0
    }

    #[inline]
    pub(crate) const fn negate(self) -> Self {
        dd_negate(self)
    }

    pub(crate) fn with_sign(self, sign: bool) -> Self {
        let abs_high = self.high.abs();
        let abs_low = self.low.abs();
        // Ensure low has same sign relationship as the original
        let (h, l) =
            if self.high.is_sign_negative() == self.low.is_sign_negative() || self.low == 0.0 {
                if sign {
                    (-abs_high, -abs_low)
                } else {
                    (abs_high, abs_low)
                }
            } else {
                // high and low have different signs - preserve the relative sign
                if sign {
                    (-abs_high, abs_low)
                } else {
                    (abs_high, -abs_low)
                }
            };
        DoubleF64 { high: h, low: l }
    }

    pub(crate) fn compare_abs(self, other: DoubleF64) -> Ordering {
        let a = DoubleF64 {
            high: self.high.abs(),
            low: self.low,
        };
        let b = DoubleF64 {
            high: other.high.abs(),
            low: other.low,
        };
        match a.high.total_cmp(&b.high) {
            Ordering::Equal => {
                // Compare |low| with sign consideration
                let a_total = a.high + a.low.abs() * a.low.signum();
                let b_total = b.high + b.low.abs() * b.low.signum();
                a_total.total_cmp(&b_total)
            }
            ord => ord,
        }
    }

    /// Convert an Fp80 value to DoubleF64 with full 64-bit significand preserved.
    pub(crate) fn from_fp80(v: Fp80) -> DoubleF64 {
        DoubleF64::from_fp80_scaled(v, 0)
    }

    /// Converts the finite value `v` multiplied by 2^-`scale` to DoubleF64. The scaled value
    /// must lie in the exponent range of f64.
    pub(crate) fn from_fp80_scaled(v: Fp80, scale: i32) -> DoubleF64 {
        if v.is_zero() {
            return if v.sign() {
                DoubleF64::new(-0.0)
            } else {
                DoubleF64::ZERO
            };
        }

        let sign = v.sign();
        let mut sig = v.significand();
        let mut exp = v.true_exponent() - scale;

        // Normalize denormals
        if v.exponent() == 0 && sig != 0 {
            let shift = sig.leading_zeros();
            sig <<= shift;
            exp -= shift as i32;
        }

        let dd = sig_exp_to_dd(sig, exp);
        if sign { dd.negate() } else { dd }
    }

    /// Rounds the value to Fp80 in the rounding mode `rc`.
    pub(crate) fn to_fp80(self, rc: RoundingMode, ef: &mut ExceptionFlags) -> Fp80 {
        self.to_fp80_scaled(0, rc, ef)
    }

    /// Rounds the value multiplied by 2^`scale` to Fp80 in the rounding mode `rc`. The scale
    /// applies before the rounding, so a result out of the f64 exponent range rounds once.
    pub(crate) fn to_fp80_scaled(
        self,
        scale: i32,
        rc: RoundingMode,
        ef: &mut ExceptionFlags,
    ) -> Fp80 {
        self.round_scaled(scale, 0, rc, ef)
    }

    /// Rounds the nonzero value multiplied by 2^`scale` to Fp80 in the rounding mode `rc`, as
    /// the leading part of an exact value whose remainder lies below every bit of the value.
    /// `below` tells whether the remainder reduces the magnitude. The result is inexact.
    pub(crate) fn to_fp80_with_remainder(
        self,
        scale: i32,
        below: bool,
        rc: RoundingMode,
        ef: &mut ExceptionFlags,
    ) -> Fp80 {
        self.round_scaled(scale, if below { -1 } else { 1 }, rc, ef)
    }

    /// Rounds the value multiplied by 2^`scale` to Fp80 in the rounding mode `rc` at 64 bits.
    /// A nonzero `remainder` adds a sticky unit of that sign to the magnitude below every bit
    /// of the value. The bits of the value below the 64-bit significand decide the direction
    /// of the rounding and C1.
    fn round_scaled(
        self,
        scale: i32,
        remainder: i8,
        rc: RoundingMode,
        ef: &mut ExceptionFlags,
    ) -> Fp80 {
        let Some((sign, exp, combined)) = self.combined_significand() else {
            return if self.high.is_sign_negative() {
                Fp80::NEG_ZERO
            } else {
                Fp80::ZERO
            };
        };
        let combined = combined.wrapping_add_signed(i128::from(remainder));
        Fp80::round_and_pack(sign, exp + scale, combined, rc, Precision::Extended, ef)
    }

    /// Returns whether the nonzero value lies on a 64-bit value or halfway between two 64-bit
    /// values, the cases where the rounding depends on terms below the value.
    pub(crate) fn is_exact_or_halfway(self) -> bool {
        let Some((_, _, combined)) = self.combined_significand() else {
            return false;
        };
        let fraction = (combined << combined.leading_zeros()) as u64;
        fraction == 0 || fraction == 1 << 63
    }

    /// Returns whether the value is negative.
    pub(crate) fn is_sign_negative(self) -> bool {
        let value = quick_two_sum(self.high, self.low);
        value.high.is_sign_negative()
    }

    /// Splits the nonnegative value below 2^126 into its integer part and its fraction.
    pub(crate) fn floor_and_fraction(self) -> (i128, f64) {
        let (high_integer, high_fraction) = f64_floor_and_fraction(self.high);
        let (low_integer, low_fraction) = f64_floor_and_fraction(self.low);
        let mut integer = high_integer + low_integer;
        let mut fraction = high_fraction + low_fraction;
        if fraction >= 1.0 {
            integer += 1;
            fraction -= 1.0;
        }
        (integer, fraction)
    }

    /// Returns the sign, the exponent of bit 127 and the 128-bit significand of the nonzero
    /// value with the J-bit at bit 127 before a normalization. Bits below bit 0 act as a sticky
    /// bit. Returns None for a zero value or a value below the normal f64 range.
    fn combined_significand(self) -> Option<(bool, i32, u128)> {
        let val = quick_two_sum(self.high, self.low);
        if val.high == 0.0 {
            return None;
        }

        let sign = val.high.is_sign_negative();
        let abs_high = val.high.abs();
        let abs_low = if sign { -val.low } else { val.low };

        let bits_h = abs_high.to_bits();
        let biased_h = ((bits_h >> 52) & 0x7FF) as i32;
        let frac_h = bits_h & ((1u64 << 52) - 1);
        if biased_h == 0 {
            return None;
        }

        let exp = biased_h - 1023;
        let sig_h = (1u128 << 52) | frac_h as u128;
        let mut combined = sig_h << 75;

        if abs_low != 0.0 {
            let low_abs = abs_low.abs();
            let bits_l = low_abs.to_bits();
            let biased_l = ((bits_l >> 52) & 0x7FF) as i32;
            let frac_l = bits_l & ((1u64 << 52) - 1);

            let shifted = if biased_l != 0 {
                let exp_l = biased_l - 1023;
                let sig_l = (1u128 << 52) | frac_l as u128;
                let sig_l_at_127 = sig_l << 75;
                shift_right_sticky(sig_l_at_127, (exp - exp_l) as u32)
            } else {
                1
            };
            if abs_low >= 0.0 {
                combined = combined.wrapping_add(shifted);
            } else {
                combined = combined.wrapping_sub(shifted);
            }
        }
        Some((sign, exp, combined))
    }

    /// Convert from i64 to DoubleF64, preserving full precision.
    pub(crate) fn from_i64(n: i64) -> DoubleF64 {
        let high = n as f64;
        let low = (n - high as i64) as f64;
        DoubleF64::from_parts(high, low)
    }

    /// Converts `value` x 2^`exponent` to DoubleF64. The conversion is exact for |`value`| below
    /// 2^106 and a result with both parts in the normal f64 range.
    pub(crate) fn from_i128_scaled(value: i128, exponent: i32) -> DoubleF64 {
        let scale = f64::from_bits(((F64_EXPONENT_BIAS + exponent) as u64) << 52);
        let high = value as f64;
        let low = (value - high as i128) as f64;
        DoubleF64::from_parts(high * scale, low * scale)
    }
}

/// Splits the finite `value` with a magnitude below 2^126 into its integer part rounded down
/// and the fraction in [0, 1). Both parts are exact.
fn f64_floor_and_fraction(value: f64) -> (i128, f64) {
    let mut integer = value as i128;
    if integer as f64 > value {
        integer -= 1;
    }
    (integer, value - integer as f64)
}

/// Convert a raw (significand, true_exponent) pair to DoubleF64.
/// The significand need not be normalized (J-bit at position 63); normalization is automatic.
/// The represented value is: `significand × 2^(true_exponent - 63)`.
fn sig_exp_to_dd(mut sig: u64, mut true_exp: i32) -> DoubleF64 {
    if sig == 0 {
        return DoubleF64::ZERO;
    }

    // Normalize: ensure J-bit at position 63
    let lz = sig.leading_zeros();
    sig <<= lz;
    true_exp -= lz as i32;

    // sig now has J-bit at position 63
    // value = sig × 2^(true_exp - 63) = (1.frac) × 2^true_exp
    let high_sig = sig >> 11; // 53 bits, implicit 1 at bit 52
    let low_sig = sig & 0x7FF; // bottom 11 bits

    // Build high f64
    let f64_biased = (true_exp + 1023) as u64;
    let high_frac = high_sig & ((1u64 << 52) - 1);
    let high = f64::from_bits((f64_biased << 52) | high_frac);

    // Build low f64: low_sig × 2^(true_exp - 63)
    let low = if low_sig == 0 {
        0.0
    } else {
        // Normalize low_sig to put its MSB at bit 52 for valid f64 representation
        let low_lz = low_sig.leading_zeros() - 11;
        let low_norm = low_sig << low_lz;
        // value = low_sig × 2^(true_exp - 63)
        //       = low_norm × 2^(true_exp - 63 - low_lz)
        //       = (1.frac) × 2^(52 + true_exp - 63 - low_lz)
        //       = (1.frac) × 2^(true_exp - 11 - low_lz)
        let low_biased = (true_exp - 11 - low_lz as i32 + 1023) as u64;
        let low_frac = low_norm & ((1u64 << 52) - 1);
        f64::from_bits((low_biased << 52) | low_frac)
    };

    DoubleF64::from_parts(high, low)
}

pub(crate) const DD_LN2: DoubleF64 = DoubleF64::from_parts(
    core::f64::consts::LN_2, // 0x1.62e42fefa39efp-1
    2.3190468138462996e-17,  // 0x1.abc9e3b39803fp-56 (ln2 - high)
);

pub(crate) const DD_LN2INV2: DoubleF64 = DoubleF64::from_parts(
    2.8853900817779268,     // 0x1.71547652b82fep+1  (2/ln2)
    4.0710547481862066e-17, // 0x1.777d0ffda0d24p-55
);

pub(crate) const DD_SQRT3: DoubleF64 = DoubleF64::from_parts(
    1.7320508075688772,     // 0x1.bb67ae8584caap+0
    1.0035084221806903e-16, // 0x1.cec95d0b5c1e3p-53
);

pub(crate) const DD_PI: DoubleF64 = DoubleF64::from_parts(
    core::f64::consts::PI,  // 0x1.921fb54442d18p+1
    1.2246467991473532e-16, // 0x1.1a62633145c07p-53
);

pub(crate) const DD_PI2: DoubleF64 = DoubleF64::from_parts(
    core::f64::consts::FRAC_PI_2, // 0x1.921fb54442d18p+0
    6.123233995736766e-17,        // 0x1.1a62633145c07p-54
);

pub(crate) const DD_PI4: DoubleF64 = DoubleF64::from_parts(
    core::f64::consts::FRAC_PI_4, // 0x1.921fb54442d18p-1
    3.061616997868383e-17,        // 0x1.1a62633145c07p-55
);

pub(crate) const DD_PI6: DoubleF64 = DoubleF64::from_parts(
    core::f64::consts::FRAC_PI_6, // 0x1.0c152382d7366p-1
    -5.360408832255454e-17,       // -0x1.ee6913347c2a5p-55
);

pub(crate) fn dd_3pi4() -> DoubleF64 {
    dd_add(DD_PI4, DD_PI2)
}

const fn const_dd_from_u64(n: u64) -> DoubleF64 {
    DoubleF64::new(n as f64)
}

const fn const_dd_recip(n: u64) -> DoubleF64 {
    dd_div(DoubleF64::ONE, const_dd_from_u64(n))
}

const fn compute_exp_arr() -> [DoubleF64; 19] {
    let mut arr = [DoubleF64::ZERO; 19];
    // 1/n! computed by successive division: arr[i] = 1/(i+1)!
    let mut recip_fact = DoubleF64::ONE;
    let mut i = 0;
    while i < 19 {
        recip_fact = dd_div(recip_fact, const_dd_from_u64((i + 1) as u64));
        arr[i] = recip_fact;
        i += 1;
    }
    arr
}

const fn compute_ln_arr() -> [DoubleF64; 12] {
    let mut arr = [DoubleF64::ZERO; 12];
    let mut i = 0;
    while i < 12 {
        arr[i] = const_dd_recip((2 * i + 1) as u64);
        i += 1;
    }
    arr
}

const fn compute_sin_arr() -> [DoubleF64; 11] {
    let mut arr = [DoubleF64::ZERO; 11];
    // arr[i] = (-1)^i / (2i+1)!
    let mut recip_fact = DoubleF64::ONE; // starts at 1/0! = 1
    let mut i = 0;
    while i < 11 {
        // (2i+1)! = (2i)! * (2i+1), and (2i)! = (2i-1)! * 2i
        // Successive: from (2(i-1)+1)! to (2i+1)! multiply by (2i)*(2i+1)
        if i == 0 {
            recip_fact = DoubleF64::ONE; // 1/1! = 1
        } else {
            let two_i = (2 * i) as u64;
            recip_fact = dd_div(recip_fact, const_dd_from_u64(two_i));
            recip_fact = dd_div(recip_fact, const_dd_from_u64(two_i + 1));
        }
        arr[i] = if i % 2 == 1 {
            dd_negate(recip_fact)
        } else {
            recip_fact
        };
        i += 1;
    }
    arr
}

const fn compute_cos_arr() -> [DoubleF64; 11] {
    let mut arr = [DoubleF64::ZERO; 11];
    // arr[i] = (-1)^i / (2i)!
    let mut recip_fact = DoubleF64::ONE; // 1/0! = 1
    let mut i = 0;
    while i < 11 {
        if i == 0 {
            recip_fact = DoubleF64::ONE; // 1/0! = 1
        } else {
            let two_i = (2 * i) as u64;
            recip_fact = dd_div(recip_fact, const_dd_from_u64(two_i - 1));
            recip_fact = dd_div(recip_fact, const_dd_from_u64(two_i));
        }
        arr[i] = if i % 2 == 1 {
            dd_negate(recip_fact)
        } else {
            recip_fact
        };
        i += 1;
    }
    arr
}

const fn compute_atan_arr() -> [DoubleF64; 16] {
    let mut arr = [DoubleF64::ZERO; 16];
    let mut i = 0;
    while i < 16 {
        let val = const_dd_recip((2 * i + 1) as u64);
        arr[i] = if i % 2 == 1 { dd_negate(val) } else { val };
        i += 1;
    }
    arr
}

pub(crate) static EXP_ARR: [DoubleF64; 19] = compute_exp_arr();
pub(crate) static LN_ARR: [DoubleF64; 12] = compute_ln_arr();
pub(crate) static SIN_ARR: [DoubleF64; 11] = compute_sin_arr();
pub(crate) static COS_ARR: [DoubleF64; 11] = compute_cos_arr();
pub(crate) static ATAN_ARR: [DoubleF64; 16] = compute_atan_arr();

pub(crate) fn eval_poly(x: DoubleF64, coeffs: &[DoubleF64]) -> DoubleF64 {
    let n = coeffs.len();
    if n == 0 {
        return DoubleF64::ZERO;
    }
    let mut result = coeffs[n - 1];
    for i in (0..n - 1).rev() {
        result = result * x + coeffs[i];
    }
    result
}

pub(crate) fn even_poly(x: DoubleF64, coeffs: &[DoubleF64]) -> DoubleF64 {
    let x2 = x * x;
    eval_poly(x2, coeffs)
}

pub(crate) fn odd_poly(x: DoubleF64, coeffs: &[DoubleF64]) -> DoubleF64 {
    let x2 = x * x;
    x * eval_poly(x2, coeffs)
}

pub(crate) fn sin_poly(r: DoubleF64) -> DoubleF64 {
    odd_poly(r, &SIN_ARR)
}

pub(crate) fn cos_poly(r: DoubleF64) -> DoubleF64 {
    even_poly(r, &COS_ARR)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Returns `value` x 2^-`exponent` of an f64 that holds an integer multiple of 2^`exponent`.
    fn scaled_integer(value: f64, exponent: i32) -> i128 {
        (value * f64::from_bits(((F64_EXPONENT_BIAS - exponent) as u64) << 52)) as i128
    }

    #[test]
    fn integer_conversion_is_exact_up_to_106_bits() {
        let values = [
            1_i128,
            -1,
            (1 << 53) + 1,
            (1 << 54) + 2,
            (1 << 67) - 1,
            -((1 << 67) - 1),
            0x6_487E_D511_0B46_11A6,
            -0x6_487E_D511_0B46_11A5,
            (1 << 105) + 1,
            (1 << 106) - 1,
        ];
        for value in values {
            for exponent in [-67, 0, 20] {
                let result = DoubleF64::from_i128_scaled(value, exponent);
                assert_eq!(
                    scaled_integer(result.high, exponent) + scaled_integer(result.low, exponent),
                    value,
                    "{value:#X} x 2^{exponent}"
                );
                assert_eq!(result.high, result.high + result.low);
            }
        }
    }
}
