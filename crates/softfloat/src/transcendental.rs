//! x87 transcendental function implementations.
//!
//! These functions implement the x87 FPU's transcendental instructions using polynomial
//! approximation with 106-bit intermediate precision ([`DoubleF64`]).
//!
//! # Trigonometric instructions (FSIN, FCOS, FSINCOS, FPTAN)
//!
//! The x87 trigonometric instructions are rounded-period approximations. As described by
//! Ferguson, Cornea, Anderson & Schneider in "The Difference Between x87 Instructions FSIN,
//! FCOS, FSINCOS, and FPTAN and Mathematical Functions sin, cos, sincos, and tan" (Intel,
//! 2015), these instructions compute `sin(x · π/p)` rather than `sin(x)`, where `p` is a
//! 66-bit approximation of π rounded-to-nearest from its leading 68 bits:
//!
//! ```text
//! p ≅ (0.C90FDAA22168C234)₁₆ · 2²
//! ```
//!
//! The relative error `e = π/p − 1 ≅ 1.5 · 2⁻⁷⁰` causes results to diverge from true
//! sin/cos as |x| grows - most notably near multiples of π where the error in ulps grows
//! sharply. For |x| < 2⁶³ the error is less than 1 ulp in round-to-nearest-even mode.
//!
//! Each trigonometric evaluation follows a three-step process:
//!
//! 1. Reduction: Compute {N, r} such that x = N·(p/2) + r exactly, with |r| < p/4.
//!    This uses 106-bit intermediate arithmetic for precision.
//! 2. Approximation: Evaluate sin(r·π/p) and cos(r·π/p) via odd/even polynomial
//!    series (SIN_ARR: 11-term, COS_ARR: 11-term) on the reduced argument.
//! 3. Reconstruction: Use N mod 4 to select the correct quadrant combination:
//!
//! | N mod 4 | sin(x·π/p)   | cos(x·π/p)    |
//! |---------|--------------|---------------|
//! | 0       |  sin_poly(r) |  cos_poly(r)  |
//! | 1       |  cos_poly(r) | −sin_poly(r)  |
//! | 2       | −sin_poly(r) | −cos_poly(r)  |
//! | 3       | −cos_poly(r) |  sin_poly(r)  |
//!
//! # Exponential/logarithmic instructions (F2XM1, FYL2X, FYL2XP1)
//!
//! F2XM1 computes 2ˣ−1 via the identity 2ˣ = e^(x·ln2), then evaluates e^y−1 using
//! a 19-term Taylor polynomial: e^y−1 = y·(1/1! + y/2! + y²/3! + ⋯ + y¹⁸/19!).
//!
//! FYL2X computes y·log₂(x) by normalizing x's significand into [√2/2, √2), computing
//! log₂(significand) via the identity ln(f) = 2·artanh((f−1)/(f+1)) with a 12-term
//! polynomial, and converting to log₂ using the constant 2/ln(2). The integer part of
//! the exponent is added separately.
//!
//! FYL2XP1 computes y·log₂(x+1) using the substitution u = x/(x+2) to avoid
//! catastrophic cancellation when x is near zero. Uses the same 12-term artanh polynomial.
//!
//! # Inverse trigonometric (FPATAN)
//!
//! Computes atan2(y,x) with unrestricted domain. Uses argument reduction based on √3
//! thresholds, a 16-term odd polynomial for the core arctan approximation, and correction
//! terms (π/6, π/4, π/2, π, 3π/4) for quadrant reconstruction.

use core::cmp::Ordering;

use crate::{
    ExceptionFlags, Fp80,
    double_f64::{
        ATAN_ARR, DD_LN2, DD_LN2INV2, DD_PI, DD_PI2, DD_PI4, DD_PI6, DD_SQRT3, DoubleF64, EXP_ARR,
        LN_ARR, cos_poly, dd_3pi4, eval_poly, odd_poly, sin_poly, trig_reduce,
    },
    flag_denormal_operands, flag_unsupported_operands,
};

/// An operand of FSIN, FCOS, FPTAN or FSINCOS with a leading exponent below this limit takes
/// the first term of the series of the function, and FPATAN with an exponent difference
/// below it takes the quotient or a multiple of pi/2. The second term then lies below bit 0
/// of the 128-bit value the result rounds from.
const FIRST_TERM_EXPONENT_LIMIT: i32 = -67;

/// An operand of F2XM1 or FYL2XP1 with a leading exponent below this limit takes the first
/// term of the series of the function. The second term, about x/2 of the first, then lies
/// below bit 0 of the 128-bit value the result rounds from.
const F2XM1_FYL2XP1_FIRST_TERM_LIMIT: i32 = -128;

/// Returns the exponent of the leading significand bit of the finite nonzero `value`. A
/// pseudo-denormal reads with the exponent 00..01 (80486 PRM 16.1.7).
fn leading_exponent(value: Fp80) -> i32 {
    value.true_exponent() - value.significand().leading_zeros() as i32
}

/// Rounds the series with the first term `term`, multiplied by 2^`scale`. The remaining
/// terms lie below the rounding of a 64-bit significand, and `below` tells whether they
/// reduce the magnitude of the result. A nonzero result is inexact: PE is set, UE for a tiny
/// result, and C1 reports the direction of the rounding (80486 PRM table 15-1). A zero term
/// is the exact result.
fn first_term_result(term: DoubleF64, below: bool, scale: i32, ef: &mut ExceptionFlags) -> Fp80 {
    if term.is_zero() {
        return term.to_fp80_scaled(scale, ef);
    }
    term.to_fp80_with_remainder(scale, below, ef)
}

/// Returns the result of a function that equals its finite nonzero operand `value` to the
/// first term, such as the sine of a tiny operand. `below` tells whether the remaining terms
/// reduce the magnitude.
fn identity_result(value: Fp80, below: bool, ef: &mut ExceptionFlags) -> Fp80 {
    let exponent = leading_exponent(value);
    first_term_result(
        DoubleF64::from_fp80_scaled(value, exponent),
        below,
        exponent,
        ef,
    )
}

/// Returns whether the finite nonzero `value` takes the first term of a series with the
/// leading exponent limit `limit`.
fn is_first_term_operand(value: Fp80, limit: i32) -> bool {
    leading_exponent(value) < limit
}

fn handle_single_nan(v: Fp80, ef: &mut ExceptionFlags) -> Option<Fp80> {
    if v.is_signaling_nan() {
        ef.invalid = true;
        Some(v.quieten())
    } else if v.is_quiet_nan() {
        Some(v)
    } else {
        None
    }
}

fn handle_two_nans(a: Fp80, b: Fp80, ef: &mut ExceptionFlags) -> Option<Fp80> {
    let a_snan = a.is_signaling_nan();
    let b_snan = b.is_signaling_nan();
    let a_nan = a.is_nan();
    let b_nan = b.is_nan();

    if a_snan || b_snan {
        ef.invalid = true;
    }

    if a_snan {
        return Some(a.quieten());
    }
    if b_snan {
        return Some(b.quieten());
    }
    if a_nan {
        return Some(a);
    }
    if b_nan {
        return Some(b);
    }
    None
}

fn is_trig_out_of_range(v: Fp80) -> bool {
    let exp = v.true_exponent();
    exp >= 63
}

fn compute_pi_f80(sign: bool, ef: &mut ExceptionFlags) -> Fp80 {
    DD_PI.with_sign(sign).to_fp80(ef)
}

fn compute_pi2_f80(sign: bool, ef: &mut ExceptionFlags) -> Fp80 {
    DD_PI2.with_sign(sign).to_fp80(ef)
}

fn compute_pi4_f80(sign: bool, ef: &mut ExceptionFlags) -> Fp80 {
    DD_PI4.with_sign(sign).to_fp80(ef)
}

fn compute_3pi4_f80(sign: bool, ef: &mut ExceptionFlags) -> Fp80 {
    dd_3pi4().with_sign(sign).to_fp80(ef)
}

impl Fp80 {
    /// Compute 2^self − 1 (F2XM1). Domain: −1.0 ≤ self ≤ +1.0.
    pub fn f2xm1(self, ef: &mut ExceptionFlags) -> Fp80 {
        if flag_unsupported_operands(&[self], ef) {
            return Fp80::INDEFINITE;
        }
        if let Some(nan) = handle_single_nan(self, ef) {
            return nan;
        }
        flag_denormal_operands(&[self], ef);
        if self.is_zero() {
            return self;
        }

        if self.true_exponent() >= 0 {
            ef.precision = true;
            if self.sign() {
                return Fp80::from_bits(0xBFFE, 0x8000_0000_0000_0000);
            }
            return self;
        }

        if is_first_term_operand(self, F2XM1_FYL2XP1_FIRST_TERM_LIMIT) {
            let exponent = leading_exponent(self);
            let term = DoubleF64::from_fp80_scaled(self, exponent) * DD_LN2;
            return first_term_result(term, self.sign(), exponent, ef);
        }

        let x = DoubleF64::from_fp80(self);
        let y = x * DD_LN2;

        let mut result = eval_poly(y, &EXP_ARR);
        result = result * y;

        result.to_fp80(ef)
    }

    /// Compute y × log₂(self) (FYL2X). self = x (ST0), y = ST1.
    pub fn fyl2x(self, y: Fp80, ef: &mut ExceptionFlags) -> Fp80 {
        if flag_unsupported_operands(&[self, y], ef) {
            return Fp80::INDEFINITE;
        }
        if let Some(nan) = handle_two_nans(self, y, ef) {
            return nan;
        }

        let x = self;

        if x.is_negative() && !x.is_zero() {
            ef.invalid = true;
            return Fp80::INDEFINITE;
        }

        if x.is_zero() {
            if y.is_zero() {
                ef.invalid = true;
                return Fp80::INDEFINITE;
            }
            ef.zero_divide = true;
            return if y.sign() {
                Fp80::INFINITY
            } else {
                Fp80::NEG_INFINITY
            };
        }

        flag_denormal_operands(&[x, y], ef);

        if x.is_infinity() {
            if y.is_zero() {
                ef.invalid = true;
                return Fp80::INDEFINITE;
            }
            return if y.sign() {
                Fp80::NEG_INFINITY
            } else {
                Fp80::INFINITY
            };
        }

        if y.is_infinity() {
            if x == Fp80::ONE {
                ef.invalid = true;
                return Fp80::INDEFINITE;
            }
            let x_gt_1 = x.true_exponent() > 0
                || (x.true_exponent() == 0 && x.significand() > 0x8000_0000_0000_0000);
            let log_positive = x_gt_1;
            let result_sign = log_positive ^ y.sign();
            return if result_sign {
                Fp80::NEG_INFINITY
            } else {
                Fp80::INFINITY
            };
        }

        if y.is_zero() {
            return Fp80::ZERO;
        }

        if x == Fp80::ONE {
            return Fp80::ZERO;
        }

        let log2_x = compute_log2(x);
        let y_exponent = leading_exponent(y);
        let y_dd = DoubleF64::from_fp80_scaled(y, y_exponent);
        let result = y_dd * log2_x;
        result.to_fp80_scaled(y_exponent, ef)
    }

    /// Compute y × log₂(self + 1) (FYL2XP1). self = x (ST0), y = ST1. An operand more
    /// negative than -1 is an invalid operation (80486 PRM table 16-11).
    pub fn fyl2xp1(self, y: Fp80, ef: &mut ExceptionFlags) -> Fp80 {
        if flag_unsupported_operands(&[self, y], ef) {
            return Fp80::INDEFINITE;
        }
        if let Some(nan) = handle_two_nans(self, y, ef) {
            return nan;
        }

        let mut ignored = ExceptionFlags::default();
        if self.compare_quiet(Fp80::ONE.negate(), &mut ignored) == crate::FpOrdering::Less {
            ef.invalid = true;
            return Fp80::INDEFINITE;
        }

        flag_denormal_operands(&[self, y], ef);

        if self.is_zero() {
            let result_sign = self.sign() ^ y.sign();
            return if result_sign {
                Fp80::NEG_ZERO
            } else {
                Fp80::ZERO
            };
        }

        let y_exponent = leading_exponent(y);
        let y_dd = DoubleF64::from_fp80_scaled(y, y_exponent);

        if is_first_term_operand(self, F2XM1_FYL2XP1_FIRST_TERM_LIMIT) {
            let x_exponent = leading_exponent(self);
            let term = DoubleF64::from_fp80_scaled(self, x_exponent) * y_dd * DD_LN2INV2;
            return first_term_result(term, !self.sign(), x_exponent + y_exponent - 1, ef);
        }

        let x_dd = DoubleF64::from_fp80(self);
        let two = DoubleF64::ONE + DoubleF64::ONE;
        let u = x_dd / (x_dd + two);
        let log2_1px = odd_poly(u, &LN_ARR) * DD_LN2INV2;

        let result = y_dd * log2_1px;
        result.to_fp80_scaled(y_exponent, ef)
    }

    /// Compute sin(self) (FSIN). Returns (result, out_of_range).
    pub fn fsin(self, ef: &mut ExceptionFlags) -> (Fp80, bool) {
        if flag_unsupported_operands(&[self], ef) {
            return (Fp80::INDEFINITE, false);
        }
        if let Some(nan) = handle_single_nan(self, ef) {
            return (nan, false);
        }
        if self.is_infinity() {
            ef.invalid = true;
            return (Fp80::INDEFINITE, false);
        }
        flag_denormal_operands(&[self], ef);
        if self.is_zero() {
            return (self, false);
        }
        if is_first_term_operand(self, FIRST_TERM_EXPONENT_LIMIT) {
            return (identity_result(self, true, ef), false);
        }
        if is_trig_out_of_range(self) {
            return (self, true);
        }

        let x_dd = DoubleF64::from_fp80(self);
        let (r, n) = trig_reduce(x_dd);
        let n_mod4 = ((n % 4) + 4) % 4;

        let result = match n_mod4 {
            0 => sin_poly(r),
            1 => cos_poly(r),
            2 => sin_poly(r).negate(),
            3 => cos_poly(r).negate(),
            _ => unreachable!(),
        };

        (result.to_fp80(ef), false)
    }

    /// Compute cos(self) (FCOS). Returns (result, out_of_range).
    pub fn fcos(self, ef: &mut ExceptionFlags) -> (Fp80, bool) {
        if flag_unsupported_operands(&[self], ef) {
            return (Fp80::INDEFINITE, false);
        }
        if let Some(nan) = handle_single_nan(self, ef) {
            return (nan, false);
        }
        if self.is_infinity() {
            ef.invalid = true;
            return (Fp80::INDEFINITE, false);
        }
        flag_denormal_operands(&[self], ef);
        if self.is_zero() {
            return (Fp80::ONE, false);
        }
        if is_first_term_operand(self, FIRST_TERM_EXPONENT_LIMIT) {
            return (first_term_result(DoubleF64::ONE, true, 0, ef), false);
        }
        if is_trig_out_of_range(self) {
            return (self, true);
        }

        let x_dd = DoubleF64::from_fp80(self);
        let (r, n) = trig_reduce(x_dd);
        let n_mod4 = ((n % 4) + 4) % 4;

        let result = match n_mod4 {
            0 => cos_poly(r),
            1 => sin_poly(r).negate(),
            2 => cos_poly(r).negate(),
            3 => sin_poly(r),
            _ => unreachable!(),
        };

        (result.to_fp80(ef), false)
    }

    /// Compute (sin(self), cos(self)) simultaneously (FSINCOS).
    pub fn fsincos(self, ef: &mut ExceptionFlags) -> (Fp80, Fp80, bool) {
        if flag_unsupported_operands(&[self], ef) {
            return (Fp80::INDEFINITE, Fp80::INDEFINITE, false);
        }
        if let Some(nan) = handle_single_nan(self, ef) {
            return (nan, nan, false);
        }
        if self.is_infinity() {
            ef.invalid = true;
            return (Fp80::INDEFINITE, Fp80::INDEFINITE, false);
        }
        flag_denormal_operands(&[self], ef);
        if self.is_zero() {
            return (self, Fp80::ONE, false);
        }
        if is_first_term_operand(self, FIRST_TERM_EXPONENT_LIMIT) {
            let cosine = first_term_result(DoubleF64::ONE, true, 0, ef);
            return (identity_result(self, true, ef), cosine, false);
        }
        if is_trig_out_of_range(self) {
            return (self, Fp80::ONE, true);
        }

        let x_dd = DoubleF64::from_fp80(self);
        let (r, n) = trig_reduce(x_dd);
        let n_mod4 = ((n % 4) + 4) % 4;

        let s = sin_poly(r);
        let c = cos_poly(r);

        let (sin_result, cos_result) = match n_mod4 {
            0 => (s, c),
            1 => (c, s.negate()),
            2 => (s.negate(), c.negate()),
            3 => (c.negate(), s),
            _ => unreachable!(),
        };

        let mut ef_sin = ExceptionFlags::default();
        let mut ef_cos = ExceptionFlags::default();
        let sin_fp80 = sin_result.to_fp80(&mut ef_sin);
        let cos_fp80 = cos_result.to_fp80(&mut ef_cos);
        ef.merge(&ef_cos);
        ef.merge(&ef_sin);
        (sin_fp80, cos_fp80, false)
    }

    /// Compute tan(self) (FPTAN). Returns (tan, out_of_range).
    pub fn fptan(self, ef: &mut ExceptionFlags) -> (Fp80, bool) {
        if flag_unsupported_operands(&[self], ef) {
            return (Fp80::INDEFINITE, false);
        }
        if let Some(nan) = handle_single_nan(self, ef) {
            return (nan, false);
        }
        if self.is_infinity() {
            ef.invalid = true;
            return (Fp80::INDEFINITE, false);
        }
        flag_denormal_operands(&[self], ef);
        if self.is_zero() {
            return (self, false);
        }
        if is_first_term_operand(self, FIRST_TERM_EXPONENT_LIMIT) {
            return (identity_result(self, false, ef), false);
        }
        if is_trig_out_of_range(self) {
            return (self, true);
        }

        let x_dd = DoubleF64::from_fp80(self);
        let (r, n) = trig_reduce(x_dd);
        let n_mod4 = ((n % 4) + 4) % 4;

        let s = sin_poly(r);
        let c = cos_poly(r);

        let (num, den) = match n_mod4 {
            0 => (s, c),
            1 => (c, s.negate()),
            2 => (s.negate(), c.negate()),
            3 => (c.negate(), s),
            _ => unreachable!(),
        };

        let result = num / den;
        (result.to_fp80(ef), false)
    }

    /// Compute atan2(self, x) (FPATAN). self = y (ST1), x = ST0.
    pub fn fpatan(self, x: Fp80, ef: &mut ExceptionFlags) -> Fp80 {
        let y = self;

        if flag_unsupported_operands(&[y, x], ef) {
            return Fp80::INDEFINITE;
        }
        if let Some(nan) = handle_two_nans(y, x, ef) {
            return nan;
        }

        flag_denormal_operands(&[y, x], ef);

        let y_sign = y.sign();
        let x_sign = x.sign();

        if y.is_zero() {
            if x.is_zero() {
                if x_sign {
                    return compute_pi_f80(y_sign, ef);
                }
                return y;
            }
            if x.is_negative() {
                return compute_pi_f80(y_sign, ef);
            }
            return y;
        }

        if x.is_zero() {
            return compute_pi2_f80(y_sign, ef);
        }

        if y.is_infinity() && x.is_infinity() {
            if x_sign {
                return compute_3pi4_f80(y_sign, ef);
            }
            return compute_pi4_f80(y_sign, ef);
        }

        if y.is_infinity() {
            return compute_pi2_f80(y_sign, ef);
        }

        if x.is_infinity() {
            if x_sign {
                return compute_pi_f80(y_sign, ef);
            }
            return if y_sign { Fp80::NEG_ZERO } else { Fp80::ZERO };
        }

        let y_exponent = leading_exponent(y);
        let x_exponent = leading_exponent(x);
        if y_exponent - x_exponent < FIRST_TERM_EXPONENT_LIMIT {
            if x_sign {
                return compute_pi_f80(y_sign, ef);
            }
            let quotient = DoubleF64::from_fp80_scaled(y, y_exponent)
                / DoubleF64::from_fp80_scaled(x, x_exponent);
            return first_term_result(quotient, true, y_exponent - x_exponent, ef);
        }
        if x_exponent - y_exponent < FIRST_TERM_EXPONENT_LIMIT {
            return compute_pi2_f80(y_sign, ef);
        }

        let scale = y_exponent.max(x_exponent);
        let y_dd = DoubleF64::from_fp80_scaled(y.abs(), scale);
        let x_dd = DoubleF64::from_fp80_scaled(x.abs(), scale);

        let swapped = y_dd.compare_abs(x_dd) == Ordering::Greater;
        let (num, den) = if swapped { (x_dd, y_dd) } else { (y_dd, x_dd) };
        let ratio = num / den;

        let two_minus_sqrt3 = DD_SQRT3.negate() + DoubleF64::ONE + DoubleF64::ONE;

        let (reduced, correction) = if ratio.compare_abs(two_minus_sqrt3) == Ordering::Less {
            (ratio, DoubleF64::ZERO)
        } else if ratio.compare_abs(DoubleF64::ONE) != Ordering::Greater {
            let sqrt3 = DD_SQRT3;
            let numerator = ratio * sqrt3 - DoubleF64::ONE;
            let denominator = sqrt3 + ratio;
            (numerator / denominator, DD_PI6)
        } else {
            let numerator = ratio - DoubleF64::ONE;
            let denominator = ratio + DoubleF64::ONE;
            (numerator / denominator, DD_PI4)
        };

        let mut atan_approx = odd_poly(reduced, &ATAN_ARR);
        atan_approx = atan_approx + correction;

        if swapped {
            atan_approx = DD_PI2 - atan_approx;
        }

        if x_sign {
            atan_approx = DD_PI - atan_approx;
        }

        ef.precision = true;
        atan_approx.with_sign(y_sign).to_fp80(ef)
    }
}

fn compute_log2(x: Fp80) -> DoubleF64 {
    let exp = x.true_exponent();
    let mut significand = x.significand();
    let mut n = exp;

    if significand == 0 {
        return DoubleF64::ZERO;
    }

    let shift = significand.leading_zeros();
    significand <<= shift;
    n -= shift as i32;

    let sqrt2_sig: u64 = 0xB504_F333_F9DE_6484;
    if significand >= sqrt2_sig {
        significand >>= 1;
        n += 1;
    }

    let f = DoubleF64::from_significand(significand);

    let f_minus_1 = f - DoubleF64::ONE;
    let f_plus_1 = f + DoubleF64::ONE;

    if f_minus_1.is_zero() {
        return DoubleF64::from_i64(n as i64);
    }

    let u = f_minus_1 / f_plus_1;
    let log2_f = odd_poly(u, &LN_ARR) * DD_LN2INV2;

    let n_dd = DoubleF64::from_i64(n as i64);
    n_dd + log2_f
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEGATIVE_ONE: Fp80 = Fp80::from_bits(0xBFFF, 0x8000_0000_0000_0000);
    const NEGATIVE_HALF: Fp80 = Fp80::from_bits(0xBFFE, 0x8000_0000_0000_0000);
    const POSITIVE_SNAN: Fp80 = Fp80::from_bits(0x7FFF, 0x8000_0000_0000_0001);
    const POSITIVE_QNAN: Fp80 = Fp80::from_bits(0x7FFF, 0xC000_0000_0000_0001);

    // |x| >= 2^63: smallest out-of-range value for trig functions.
    const OUT_OF_RANGE: Fp80 = Fp80::from_bits(0x403E, 0x8000_0000_0000_0000);

    fn no_exceptions() -> ExceptionFlags {
        ExceptionFlags::default()
    }

    #[test]
    fn test_f2xm1_basic() {
        // f2xm1(+0) = +0.
        let mut ef = no_exceptions();
        let result = Fp80::ZERO.f2xm1(&mut ef);
        assert_eq!(result, Fp80::ZERO);

        // f2xm1(-0) = -0.
        let mut ef = no_exceptions();
        let result = Fp80::NEG_ZERO.f2xm1(&mut ef);
        assert_eq!(result, Fp80::NEG_ZERO);
    }

    #[test]
    fn test_f2xm1_edge_cases() {
        // f2xm1(-1.0) = -0.5 + PE.
        let mut ef = no_exceptions();
        let result = NEGATIVE_ONE.f2xm1(&mut ef);
        assert_eq!(result, NEGATIVE_HALF);
        assert!(ef.precision);

        // f2xm1(+1.0) = +1.0 + PE.
        let mut ef = no_exceptions();
        let result = Fp80::ONE.f2xm1(&mut ef);
        assert_eq!(result, Fp80::ONE);
        assert!(ef.precision);

        // SNaN -> QNaN + IE.
        let mut ef = no_exceptions();
        let result = POSITIVE_SNAN.f2xm1(&mut ef);
        assert!(result.is_quiet_nan());
        assert!(ef.invalid);

        // QNaN -> QNaN, no IE.
        let mut ef = no_exceptions();
        let result = POSITIVE_QNAN.f2xm1(&mut ef);
        assert!(result.is_quiet_nan());
        assert!(!ef.invalid);
    }

    #[test]
    fn test_fyl2x_basic() {
        // fyl2x(x=2, y=1) = 1 * log2(2) = 1.0.
        let two = Fp80::from_bits(0x4000, 0x8000_0000_0000_0000);
        let mut ef = no_exceptions();
        let result = two.fyl2x(Fp80::ONE, &mut ef);
        assert_eq!(result, Fp80::ONE);

        // fyl2x(x=1, y=1) = 1 * log2(1) = 0.
        let mut ef = no_exceptions();
        let result = Fp80::ONE.fyl2x(Fp80::ONE, &mut ef);
        assert!(result.is_zero());
    }

    #[test]
    fn test_fyl2x_edge_cases() {
        // x < 0 -> Indefinite + IE.
        let mut ef = no_exceptions();
        let result = NEGATIVE_ONE.fyl2x(Fp80::ONE, &mut ef);
        assert_eq!(result, Fp80::INDEFINITE);
        assert!(ef.invalid);

        // x = 0, y = 0 -> Indefinite + IE.
        let mut ef = no_exceptions();
        let result = Fp80::ZERO.fyl2x(Fp80::ZERO, &mut ef);
        assert_eq!(result, Fp80::INDEFINITE);
        assert!(ef.invalid);

        // x = 0, y = positive nonzero -> -inf + ZE (sign opposite to y).
        let mut ef = no_exceptions();
        let result = Fp80::ZERO.fyl2x(Fp80::ONE, &mut ef);
        assert_eq!(result, Fp80::NEG_INFINITY);
        assert!(ef.zero_divide);

        // x = 0, y = negative nonzero -> +inf + ZE.
        let mut ef = no_exceptions();
        let result = Fp80::ZERO.fyl2x(NEGATIVE_ONE, &mut ef);
        assert_eq!(result, Fp80::INFINITY);
        assert!(ef.zero_divide);

        // x = +inf, y = 0 -> Indefinite + IE.
        let mut ef = no_exceptions();
        let result = Fp80::INFINITY.fyl2x(Fp80::ZERO, &mut ef);
        assert_eq!(result, Fp80::INDEFINITE);
        assert!(ef.invalid);

        // x = 1, y = inf -> Indefinite + IE (0 * inf is indeterminate).
        let mut ef = no_exceptions();
        let result = Fp80::ONE.fyl2x(Fp80::INFINITY, &mut ef);
        assert_eq!(result, Fp80::INDEFINITE);
        assert!(ef.invalid);

        // x = +inf, y = positive -> +inf.
        let mut ef = no_exceptions();
        let result = Fp80::INFINITY.fyl2x(Fp80::ONE, &mut ef);
        assert_eq!(result, Fp80::INFINITY);

        // x = +inf, y = negative -> -inf.
        let mut ef = no_exceptions();
        let result = Fp80::INFINITY.fyl2x(NEGATIVE_ONE, &mut ef);
        assert_eq!(result, Fp80::NEG_INFINITY);

        // SNaN propagation.
        let mut ef = no_exceptions();
        let result = POSITIVE_SNAN.fyl2x(Fp80::ONE, &mut ef);
        assert!(result.is_quiet_nan());
        assert!(ef.invalid);

        let mut ef = no_exceptions();
        let result = Fp80::ONE.fyl2x(POSITIVE_SNAN, &mut ef);
        assert!(result.is_quiet_nan());
        assert!(ef.invalid);

        // QNaN propagation: QNaN as x, no IE.
        let mut ef = no_exceptions();
        let result = POSITIVE_QNAN.fyl2x(Fp80::ONE, &mut ef);
        assert!(result.is_quiet_nan());
        assert!(!ef.invalid);

        // QNaN propagation: QNaN as y, no IE.
        let mut ef = no_exceptions();
        let result = Fp80::ONE.fyl2x(POSITIVE_QNAN, &mut ef);
        assert!(result.is_quiet_nan());
        assert!(!ef.invalid);
    }

    #[test]
    fn test_fyl2xp1_basic() {
        // fyl2xp1(x=+0, y=ONE) = +0 (sign = sign(x) XOR sign(y) = 0 XOR 0 = +0).
        let mut ef = no_exceptions();
        let result = Fp80::ZERO.fyl2xp1(Fp80::ONE, &mut ef);
        assert_eq!(result, Fp80::ZERO);
        assert!(!result.sign());

        // fyl2xp1(x=-0, y=ONE) = -0 (sign = 1 XOR 0 = 1).
        let mut ef = no_exceptions();
        let result = Fp80::NEG_ZERO.fyl2xp1(Fp80::ONE, &mut ef);
        assert_eq!(result, Fp80::NEG_ZERO);
    }

    #[test]
    fn test_fyl2xp1_edge_cases() {
        // fyl2xp1(x=+0, y=negative) = -0 (sign = 0 XOR 1 = 1).
        let mut ef = no_exceptions();
        let result = Fp80::ZERO.fyl2xp1(NEGATIVE_ONE, &mut ef);
        assert_eq!(result, Fp80::NEG_ZERO);

        // fyl2xp1(x=-0, y=negative) = +0 (sign = 1 XOR 1 = 0).
        let mut ef = no_exceptions();
        let result = Fp80::NEG_ZERO.fyl2xp1(NEGATIVE_ONE, &mut ef);
        assert_eq!(result, Fp80::ZERO);
        assert!(!result.sign());

        // SNaN propagation.
        let mut ef = no_exceptions();
        let result = POSITIVE_SNAN.fyl2xp1(Fp80::ONE, &mut ef);
        assert!(result.is_quiet_nan());
        assert!(ef.invalid);

        let mut ef = no_exceptions();
        let result = Fp80::ZERO.fyl2xp1(POSITIVE_SNAN, &mut ef);
        assert!(result.is_quiet_nan());
        assert!(ef.invalid);

        // QNaN propagation: QNaN as x, no IE.
        let mut ef = no_exceptions();
        let result = POSITIVE_QNAN.fyl2xp1(Fp80::ONE, &mut ef);
        assert!(result.is_quiet_nan());
        assert!(!ef.invalid);

        // QNaN propagation: QNaN as y, no IE.
        let mut ef = no_exceptions();
        let result = Fp80::ZERO.fyl2xp1(POSITIVE_QNAN, &mut ef);
        assert!(result.is_quiet_nan());
        assert!(!ef.invalid);
    }

    #[test]
    fn test_fsin_basic() {
        // sin(+0) = +0.
        let mut ef = no_exceptions();
        let (result, out_of_range) = Fp80::ZERO.fsin(&mut ef);
        assert_eq!(result, Fp80::ZERO);
        assert!(!out_of_range);

        // sin(-0) = -0.
        let mut ef = no_exceptions();
        let (result, out_of_range) = Fp80::NEG_ZERO.fsin(&mut ef);
        assert_eq!(result, Fp80::NEG_ZERO);
        assert!(!out_of_range);
    }

    #[test]
    fn test_fsin_edge_cases() {
        // sin(+inf) = Indefinite + IE.
        let mut ef = no_exceptions();
        let (result, _) = Fp80::INFINITY.fsin(&mut ef);
        assert_eq!(result, Fp80::INDEFINITE);
        assert!(ef.invalid);

        // sin(-inf) = Indefinite + IE.
        let mut ef = no_exceptions();
        let (result, _) = Fp80::NEG_INFINITY.fsin(&mut ef);
        assert_eq!(result, Fp80::INDEFINITE);
        assert!(ef.invalid);

        // |x| >= 2^63 -> (x unchanged, C2=true).
        let mut ef = no_exceptions();
        let (result, out_of_range) = OUT_OF_RANGE.fsin(&mut ef);
        assert_eq!(result, OUT_OF_RANGE);
        assert!(out_of_range);

        // SNaN -> QNaN + IE.
        let mut ef = no_exceptions();
        let (result, _) = POSITIVE_SNAN.fsin(&mut ef);
        assert!(result.is_quiet_nan());
        assert!(ef.invalid);

        // QNaN -> QNaN, no IE.
        let mut ef = no_exceptions();
        let (result, _) = POSITIVE_QNAN.fsin(&mut ef);
        assert!(result.is_quiet_nan());
        assert!(!ef.invalid);
    }

    #[test]
    fn test_fcos_basic() {
        // cos(+0) = +1.0.
        let mut ef = no_exceptions();
        let (result, out_of_range) = Fp80::ZERO.fcos(&mut ef);
        assert_eq!(result, Fp80::ONE);
        assert!(!out_of_range);

        // cos(-0) = +1.0.
        let mut ef = no_exceptions();
        let (result, out_of_range) = Fp80::NEG_ZERO.fcos(&mut ef);
        assert_eq!(result, Fp80::ONE);
        assert!(!out_of_range);
    }

    #[test]
    fn test_fcos_edge_cases() {
        // cos(+inf) = Indefinite + IE.
        let mut ef = no_exceptions();
        let (result, _) = Fp80::INFINITY.fcos(&mut ef);
        assert_eq!(result, Fp80::INDEFINITE);
        assert!(ef.invalid);

        // cos(-inf) = Indefinite + IE.
        let mut ef = no_exceptions();
        let (result, _) = Fp80::NEG_INFINITY.fcos(&mut ef);
        assert_eq!(result, Fp80::INDEFINITE);
        assert!(ef.invalid);

        // |x| >= 2^63 -> (x unchanged, C2=true).
        let mut ef = no_exceptions();
        let (result, out_of_range) = OUT_OF_RANGE.fcos(&mut ef);
        assert_eq!(result, OUT_OF_RANGE);
        assert!(out_of_range);

        // SNaN -> QNaN + IE.
        let mut ef = no_exceptions();
        let (result, _) = POSITIVE_SNAN.fcos(&mut ef);
        assert!(result.is_quiet_nan());
        assert!(ef.invalid);

        // QNaN -> QNaN, no IE.
        let mut ef = no_exceptions();
        let (result, _) = POSITIVE_QNAN.fcos(&mut ef);
        assert!(result.is_quiet_nan());
        assert!(!ef.invalid);
    }

    #[test]
    fn test_fsincos_basic() {
        // fsincos(+0) = (sin=+0, cos=+1.0, false).
        let mut ef = no_exceptions();
        let (sin, cos, out_of_range) = Fp80::ZERO.fsincos(&mut ef);
        assert_eq!(sin, Fp80::ZERO);
        assert_eq!(cos, Fp80::ONE);
        assert!(!out_of_range);

        // fsincos(-0) = (sin=-0, cos=+1.0, false).
        let mut ef = no_exceptions();
        let (sin, cos, out_of_range) = Fp80::NEG_ZERO.fsincos(&mut ef);
        assert_eq!(sin, Fp80::NEG_ZERO);
        assert_eq!(cos, Fp80::ONE);
        assert!(!out_of_range);
    }

    #[test]
    fn test_fsincos_edge_cases() {
        // fsincos(+inf) -> Indefinite + IE.
        let mut ef = no_exceptions();
        let (sin, _, _) = Fp80::INFINITY.fsincos(&mut ef);
        assert_eq!(sin, Fp80::INDEFINITE);
        assert!(ef.invalid);

        // Out of range.
        let mut ef = no_exceptions();
        let (result, _, out_of_range) = OUT_OF_RANGE.fsincos(&mut ef);
        assert_eq!(result, OUT_OF_RANGE);
        assert!(out_of_range);

        // SNaN -> QNaN + IE.
        let mut ef = no_exceptions();
        let (result, _, _) = POSITIVE_SNAN.fsincos(&mut ef);
        assert!(result.is_quiet_nan());
        assert!(ef.invalid);

        // fsincos(-inf) -> Indefinite + IE.
        let mut ef = no_exceptions();
        let (sin, _, _) = Fp80::NEG_INFINITY.fsincos(&mut ef);
        assert_eq!(sin, Fp80::INDEFINITE);
        assert!(ef.invalid);

        // QNaN -> QNaN, no IE.
        let mut ef = no_exceptions();
        let (sin, cos, _) = POSITIVE_QNAN.fsincos(&mut ef);
        assert!(sin.is_quiet_nan());
        assert!(cos.is_quiet_nan());
        assert!(!ef.invalid);
    }

    #[test]
    fn test_fptan_basic() {
        // fptan(+0) = +0.
        let mut ef = no_exceptions();
        let (result, out_of_range) = Fp80::ZERO.fptan(&mut ef);
        assert_eq!(result, Fp80::ZERO);
        assert!(!out_of_range);

        // fptan(-0) = -0.
        let mut ef = no_exceptions();
        let (result, out_of_range) = Fp80::NEG_ZERO.fptan(&mut ef);
        assert_eq!(result, Fp80::NEG_ZERO);
        assert!(!out_of_range);
    }

    #[test]
    fn test_fptan_edge_cases() {
        // fptan(+inf) = Indefinite + IE.
        let mut ef = no_exceptions();
        let (result, _) = Fp80::INFINITY.fptan(&mut ef);
        assert_eq!(result, Fp80::INDEFINITE);
        assert!(ef.invalid);

        // fptan(-inf) = Indefinite + IE.
        let mut ef = no_exceptions();
        let (result, _) = Fp80::NEG_INFINITY.fptan(&mut ef);
        assert_eq!(result, Fp80::INDEFINITE);
        assert!(ef.invalid);

        // |x| >= 2^63 -> (x unchanged, C2=true).
        let mut ef = no_exceptions();
        let (result, out_of_range) = OUT_OF_RANGE.fptan(&mut ef);
        assert_eq!(result, OUT_OF_RANGE);
        assert!(out_of_range);

        // SNaN -> QNaN + IE.
        let mut ef = no_exceptions();
        let (result, _) = POSITIVE_SNAN.fptan(&mut ef);
        assert!(result.is_quiet_nan());
        assert!(ef.invalid);

        // QNaN -> QNaN, no IE.
        let mut ef = no_exceptions();
        let (result, _) = POSITIVE_QNAN.fptan(&mut ef);
        assert!(result.is_quiet_nan());
        assert!(!ef.invalid);
    }

    #[test]
    fn test_fpatan_basic() {
        // fpatan(y=+0, x=+ONE) = +0 (atan2(+0, positive) = +0).
        let mut ef = no_exceptions();
        let result = Fp80::ZERO.fpatan(Fp80::ONE, &mut ef);
        assert_eq!(result, Fp80::ZERO);
        assert!(!result.sign());

        // fpatan(y=-0, x=+ONE) = -0 (atan2(-0, positive) = -0).
        let mut ef = no_exceptions();
        let result = Fp80::NEG_ZERO.fpatan(Fp80::ONE, &mut ef);
        assert_eq!(result, Fp80::NEG_ZERO);
    }

    #[test]
    fn test_fpatan_edge_cases() {
        // fpatan(y=+0, x=-ONE) = +pi.
        let mut ef = no_exceptions();
        let result = Fp80::ZERO.fpatan(NEGATIVE_ONE, &mut ef);
        assert!(result.is_normal());
        assert!(!result.sign()); // positive pi

        // fpatan(y=-0, x=-ONE) = -pi.
        let mut ef = no_exceptions();
        let result = Fp80::NEG_ZERO.fpatan(NEGATIVE_ONE, &mut ef);
        assert!(result.is_normal());
        assert!(result.sign()); // negative pi

        // fpatan(y=+ONE, x=+0) = +pi/2.
        let mut ef = no_exceptions();
        let result = Fp80::ONE.fpatan(Fp80::ZERO, &mut ef);
        assert!(result.is_normal());
        assert!(!result.sign());

        // fpatan(y=-ONE, x=+0) = -pi/2.
        let mut ef = no_exceptions();
        let result = NEGATIVE_ONE.fpatan(Fp80::ZERO, &mut ef);
        assert!(result.is_normal());
        assert!(result.sign());

        // fpatan(y=+inf, x=+inf) = +pi/4.
        let mut ef = no_exceptions();
        let result = Fp80::INFINITY.fpatan(Fp80::INFINITY, &mut ef);
        assert!(result.is_normal());
        assert!(!result.sign());

        // fpatan(y=+inf, x=-inf) = +3pi/4.
        let mut ef = no_exceptions();
        let result = Fp80::INFINITY.fpatan(Fp80::NEG_INFINITY, &mut ef);
        assert!(result.is_normal());
        assert!(!result.sign());

        // fpatan(y=-inf, x=+inf) = -pi/4.
        let mut ef = no_exceptions();
        let result = Fp80::NEG_INFINITY.fpatan(Fp80::INFINITY, &mut ef);
        assert!(result.is_normal());
        assert!(result.sign());

        // fpatan(y=-inf, x=-inf) = -3pi/4.
        let mut ef = no_exceptions();
        let result = Fp80::NEG_INFINITY.fpatan(Fp80::NEG_INFINITY, &mut ef);
        assert!(result.is_normal());
        assert!(result.sign());

        // fpatan(y=+inf, x=finite) = +pi/2.
        let mut ef = no_exceptions();
        let result = Fp80::INFINITY.fpatan(Fp80::ONE, &mut ef);
        assert!(result.is_normal());
        assert!(!result.sign());

        // fpatan(y=-inf, x=finite) = -pi/2.
        let mut ef = no_exceptions();
        let result = Fp80::NEG_INFINITY.fpatan(Fp80::ONE, &mut ef);
        assert!(result.is_normal());
        assert!(result.sign());

        // fpatan(y=finite, x=+inf) = +0 (positive y) or -0 (negative y).
        let mut ef = no_exceptions();
        let result = Fp80::ONE.fpatan(Fp80::INFINITY, &mut ef);
        assert!(result.is_zero());
        assert!(!result.sign());

        let mut ef = no_exceptions();
        let result = NEGATIVE_ONE.fpatan(Fp80::INFINITY, &mut ef);
        assert!(result.is_zero());
        assert!(result.sign());

        // fpatan(y=finite, x=-inf) = +pi (positive y) or -pi (negative y).
        let mut ef = no_exceptions();
        let result = Fp80::ONE.fpatan(Fp80::NEG_INFINITY, &mut ef);
        assert!(result.is_normal());
        assert!(!result.sign());

        let mut ef = no_exceptions();
        let result = NEGATIVE_ONE.fpatan(Fp80::NEG_INFINITY, &mut ef);
        assert!(result.is_normal());
        assert!(result.sign());

        // SNaN propagation.
        let mut ef = no_exceptions();
        let result = POSITIVE_SNAN.fpatan(Fp80::ONE, &mut ef);
        assert!(result.is_quiet_nan());
        assert!(ef.invalid);

        let mut ef = no_exceptions();
        let result = Fp80::ONE.fpatan(POSITIVE_SNAN, &mut ef);
        assert!(result.is_quiet_nan());
        assert!(ef.invalid);

        // QNaN propagation, no IE.
        let mut ef = no_exceptions();
        let result = POSITIVE_QNAN.fpatan(Fp80::ONE, &mut ef);
        assert!(result.is_quiet_nan());
        assert!(!ef.invalid);

        let mut ef = no_exceptions();
        let result = Fp80::ONE.fpatan(POSITIVE_QNAN, &mut ef);
        assert!(result.is_quiet_nan());
        assert!(!ef.invalid);

        // fpatan(y=+ONE, x=-0) = +pi/2.
        let mut ef = no_exceptions();
        let result = Fp80::ONE.fpatan(Fp80::NEG_ZERO, &mut ef);
        assert!(result.is_normal());
        assert!(!result.sign());

        // fpatan(y=-ONE, x=-0) = -pi/2.
        let mut ef = no_exceptions();
        let result = NEGATIVE_ONE.fpatan(Fp80::NEG_ZERO, &mut ef);
        assert!(result.is_normal());
        assert!(result.sign());
    }

    #[test]
    fn test_f2xm1_intermediate_values() {
        let half = Fp80::from_bits(0x3FFE, 0x8000_0000_0000_0000);

        // f2xm1(0.5) = sqrt(2) - 1 ≈ 0.4142.
        // Result should be positive, in range [0.25, 0.5), so true_exponent = -2.
        let mut ef = no_exceptions();
        let result = half.f2xm1(&mut ef);
        assert!(!result.is_zero());
        assert!(!result.is_negative());
        assert_eq!(result.true_exponent(), -2);
        assert!(ef.precision);

        // f2xm1(-0.5) = 1/sqrt(2) - 1 ≈ -0.2929.
        // Result should be negative, in range (-0.5, -0.25], so true_exponent = -2.
        let neg_half = Fp80::from_bits(0xBFFE, 0x8000_0000_0000_0000);
        let mut ef = no_exceptions();
        let result = neg_half.f2xm1(&mut ef);
        assert!(!result.is_zero());
        assert!(result.is_negative());
        assert_eq!(result.true_exponent(), -2);
        assert!(ef.precision);
    }

    #[test]
    fn test_fyl2x_powers_of_two() {
        let two = Fp80::from_bits(0x4000, 0x8000_0000_0000_0000);
        let three = Fp80::from_bits(0x4000, 0xC000_0000_0000_0000);

        // fyl2x(x=4, y=1) = log2(4) = 2.0.
        let four = Fp80::from_bits(0x4001, 0x8000_0000_0000_0000);
        let mut ef = no_exceptions();
        let result = four.fyl2x(Fp80::ONE, &mut ef);
        assert_eq!(result, two);

        // fyl2x(x=0.5, y=1) = log2(0.5) = -1.0.
        let half = Fp80::from_bits(0x3FFE, 0x8000_0000_0000_0000);
        let mut ef = no_exceptions();
        let result = half.fyl2x(Fp80::ONE, &mut ef);
        assert_eq!(result, NEGATIVE_ONE);

        // fyl2x(x=8, y=1) = log2(8) = 3.0.
        let eight = Fp80::from_bits(0x4002, 0x8000_0000_0000_0000);
        let mut ef = no_exceptions();
        let result = eight.fyl2x(Fp80::ONE, &mut ef);
        assert_eq!(result, three);
    }

    #[test]
    fn test_fyl2x_non_power_of_two() {
        // fyl2x(x=3, y=1) = log2(3) ≈ 1.585.
        // Significand 0xC000... >= sqrt2_sig, so sqrt(2) normalization triggers.
        // Result must be in [1.0, 2.0): true_exponent = 0, positive.
        let three = Fp80::from_bits(0x4000, 0xC000_0000_0000_0000);
        let mut ef = no_exceptions();
        let result = three.fyl2x(Fp80::ONE, &mut ef);
        assert!(!result.is_zero());
        assert!(!result.is_negative());
        assert_eq!(result.true_exponent(), 0);

        // fyl2x(x=1.5, y=1) = log2(1.5) ≈ 0.585.
        // Significand 0xC000... also >= sqrt2_sig, normalization triggers.
        // Result must be in [0.5, 1.0): true_exponent = -1, positive.
        let one_and_half = Fp80::from_bits(0x3FFF, 0xC000_0000_0000_0000);
        let mut ef = no_exceptions();
        let result = one_and_half.fyl2x(Fp80::ONE, &mut ef);
        assert!(!result.is_zero());
        assert!(!result.is_negative());
        assert_eq!(result.true_exponent(), -1);
    }

    /// A pseudo-denormal: exponent field 0 with the J-bit set.
    const PSEUDO_DENORMAL: Fp80 = Fp80::from_bits(0x0000, 0x8000_0000_0000_0001);
    /// The value of `PSEUDO_DENORMAL` with the exponent field 1.
    const PSEUDO_DENORMAL_VALUE: Fp80 = Fp80::from_bits(0x0001, 0x8000_0000_0000_0001);
    /// A denormal.
    const DENORMAL: Fp80 = Fp80::from_bits(0x0000, 0x4000_0000_0000_0000);
    /// 2^-4095, a normal far below the f64 range.
    const TINY_NORMAL: Fp80 = Fp80::from_bits(0x3000, 0x8000_0000_0000_0000);

    #[test]
    fn trigonometric_functions_of_a_tiny_operand_take_the_first_term() {
        let mut ef = no_exceptions();
        assert_eq!(
            PSEUDO_DENORMAL.fsin(&mut ef),
            (PSEUDO_DENORMAL_VALUE, false)
        );
        assert!(ef.denormal && ef.precision && !ef.underflow && ef.rounded_up);

        let mut ef = no_exceptions();
        assert_eq!(PSEUDO_DENORMAL.fcos(&mut ef), (Fp80::ONE, false));
        assert!(ef.denormal && ef.precision && ef.rounded_up);

        let mut ef = no_exceptions();
        assert_eq!(
            PSEUDO_DENORMAL.fptan(&mut ef),
            (PSEUDO_DENORMAL_VALUE, false)
        );
        assert!(ef.denormal && ef.precision && !ef.rounded_up);

        let mut ef = no_exceptions();
        assert_eq!(
            PSEUDO_DENORMAL.fsincos(&mut ef),
            (PSEUDO_DENORMAL_VALUE, Fp80::ONE, false)
        );
        assert!(ef.denormal && ef.precision && ef.rounded_up);

        let mut ef = no_exceptions();
        assert_eq!(DENORMAL.fsin(&mut ef), (DENORMAL, false));
        assert!(ef.denormal && ef.precision && ef.underflow && ef.rounded_up);

        let mut ef = no_exceptions();
        let negative = TINY_NORMAL.negate();
        assert_eq!(negative.fsin(&mut ef), (negative, false));
        assert!(ef.precision && ef.rounded_up && !ef.denormal && !ef.underflow);

        let mut ef = no_exceptions();
        assert_eq!(negative.fptan(&mut ef), (negative, false));
        assert!(ef.precision && !ef.rounded_up);
    }

    #[test]
    fn trigonometric_functions_at_the_first_term_limit_report_the_rounding() {
        for exponent in [0x3FBD, 0x3FBC, 0x3FBB, 0x3F9B] {
            let operand = Fp80::from_bits(exponent, 0x8000_0000_0000_0000);
            let mut ef = no_exceptions();
            assert_eq!(operand.fsin(&mut ef), (operand, false));
            assert!(ef.precision && ef.rounded_up, "{exponent:04X}");

            let mut ef = no_exceptions();
            assert_eq!(operand.fcos(&mut ef), (Fp80::ONE, false));
            assert!(ef.precision && ef.rounded_up, "{exponent:04X}");

            let mut ef = no_exceptions();
            assert_eq!(operand.fptan(&mut ef), (operand, false));
            assert!(ef.precision && !ef.rounded_up, "{exponent:04X}");

            let mut ef = no_exceptions();
            assert_eq!(operand.fsincos(&mut ef), (operand, Fp80::ONE, false));
            assert!(ef.precision && ef.rounded_up, "{exponent:04X}");
        }
    }

    #[test]
    fn exponential_and_logarithm_above_the_first_term_limit_keep_the_second_term() {
        let mut ef = no_exceptions();
        let result = Fp80::from_bits(0xBFBB, 0xB0E7_4E9F_66B6_93E4).f2xm1(&mut ef);
        assert_eq!(result, Fp80::from_bits(0xBFBA, 0xF53D_89DA_8272_BBC3));
        assert!(ef.precision && !ef.rounded_up);

        let mut ef = no_exceptions();
        let x = Fp80::from_bits(0xBFB9, 0x82A0_649A_3B40_0000);
        let y = Fp80::from_bits(0x4021, 0xFF82_A9C9_B040_0000);
        let result = x.fyl2xp1(y, &mut ef);
        assert_eq!(result, Fp80::from_bits(0xBFDC, 0xBC18_05C4_45F3_3A8A));
        assert!(ef.precision && ef.rounded_up);
    }

    #[test]
    fn logarithm_of_a_tiny_operand_times_zero_is_exact() {
        for x in [TINY_NORMAL, TINY_NORMAL.negate()] {
            for y in [Fp80::ZERO, Fp80::NEG_ZERO] {
                let mut ef = no_exceptions();
                assert!(x.fyl2xp1(y, &mut ef).is_zero(), "{x:?} {y:?}");
                assert_eq!(ef, no_exceptions(), "{x:?} {y:?}");
            }
        }
    }

    #[test]
    fn exponential_and_logarithm_of_a_tiny_operand_take_the_first_term() {
        let mut ef = no_exceptions();
        let result = PSEUDO_DENORMAL.f2xm1(&mut ef);
        assert_eq!(result, Fp80::from_bits(0x0000, 0x58B9_0BFB_E8E7_BCD7));
        assert!(ef.denormal && ef.underflow && ef.precision && ef.rounded_up);

        let mut ef = no_exceptions();
        let result = TINY_NORMAL.f2xm1(&mut ef);
        assert_eq!(result, Fp80::from_bits(0x2FFF, 0xB172_17F7_D1CF_79AC));
        assert!(ef.precision && ef.rounded_up && !ef.underflow);

        let mut ef = no_exceptions();
        let result = PSEUDO_DENORMAL.fyl2xp1(Fp80::ONE, &mut ef);
        assert_eq!(result, Fp80::from_bits(0x0001, 0xB8AA_3B29_5C17_F0BD));
        assert!(ef.denormal && ef.precision && !ef.rounded_up && !ef.underflow);

        let mut ef = no_exceptions();
        let result = TINY_NORMAL.fyl2xp1(Fp80::ONE, &mut ef);
        assert_eq!(result, Fp80::from_bits(0x3000, 0xB8AA_3B29_5C17_F0BC));
        assert!(ef.precision && ef.rounded_up);
    }

    #[test]
    fn arctangent_of_operands_out_of_the_f64_range() {
        let mut ef = no_exceptions();
        let result = PSEUDO_DENORMAL.fpatan(Fp80::ONE, &mut ef);
        assert_eq!(result, PSEUDO_DENORMAL_VALUE);
        assert!(ef.denormal && ef.precision && ef.rounded_up && !ef.underflow);

        let mut ef = no_exceptions();
        let result = Fp80::ONE.fpatan(PSEUDO_DENORMAL, &mut ef);
        assert_eq!(result, Fp80::from_bits(0x3FFF, 0xC90F_DAA2_2168_C235));
        assert!(ef.denormal && ef.precision && ef.rounded_up);

        let mut ef = no_exceptions();
        let result = PSEUDO_DENORMAL.fpatan(NEGATIVE_ONE, &mut ef);
        assert_eq!(result, Fp80::PI_UP);

        let mut quarter_flags = no_exceptions();
        let quarter = Fp80::ONE.fpatan(Fp80::ONE, &mut quarter_flags);
        let mut ef = no_exceptions();
        assert_eq!(PSEUDO_DENORMAL.fpatan(PSEUDO_DENORMAL, &mut ef), quarter);
        let huge = Fp80::from_bits(0x7000, 0x8000_0000_0000_0000);
        assert_eq!(huge.fpatan(huge, &mut ef), quarter);
    }
}
