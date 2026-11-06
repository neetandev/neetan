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
//!    This uses integer arithmetic on the 64-bit significand and the 68-bit significand of p/2.
//! 2. Approximation: Evaluate sin(r) and cos(r) via odd/even polynomial
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
    ExceptionFlags, Fp80, Precision, RoundingMode,
    double_f64::{
        ATAN_ARR, COS_ARR, DD_LN2, DD_LN2INV2, DD_PI, DD_PI2, DD_PI4, DD_PI6, DD_SQRT3, DoubleF64,
        EXP_ARR, LN_ARR, SIN_ARR, cos_poly, dd_3pi4, eval_poly, odd_poly, sin_poly,
    },
    flag_denormal_operands, flag_unsupported_operands,
};

/// FPATAN with a negative x and an exponent difference of y and x below this limit takes pi.
/// FPATAN with an exponent difference of x and y below this limit takes pi/2. The quotient
/// then lies below 1/16 ulp of the result and cannot change its rounding.
const FIRST_TERM_EXPONENT_LIMIT: i32 = -67;

/// FPATAN with a positive x and an exponent difference below this limit computes atan(y/x)
/// from the exact quotient of the significands and the series of the remaining terms, which
/// lie below 2^-46 of the quotient.
const FPATAN_SMALL_RATIO_LIMIT: i32 = -24;

/// Below this exponent difference the terms of atan(y/x) beyond the quotient lie below
/// 2^-64 of the lowest bit of the 127-bit quotient and only decide the sticky remainder.
const FPATAN_STICKY_TAIL_LIMIT: i32 = -100;

/// The largest shift of one division step of the FPATAN quotient. A remainder below 2^64
/// shifted by it stays below 2^127.
const QUOTIENT_STEP_BITS: u32 = 63;

/// An operand of FSIN, FCOS, FPTAN or FSINCOS with a leading exponent below this limit takes
/// the first term of the series of the function, the operand or 1. The second term then
/// lies below half the distance from the first term to its neighbors at 64 bits and only
/// decides the direction of the rounding.
const TRIG_FIRST_TERM_EXPONENT_LIMIT: i32 = -32;

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
fn first_term_result(
    term: DoubleF64,
    below: bool,
    scale: i32,
    rc: RoundingMode,
    ef: &mut ExceptionFlags,
) -> Fp80 {
    if term.is_zero() {
        return term.to_fp80_scaled(scale, rc, ef);
    }
    term.to_fp80_with_remainder(scale, below, rc, ef)
}

/// Returns the result of a function that equals its finite nonzero operand `value` to the
/// first term, such as the sine of a tiny operand. `below` tells whether the remaining terms
/// reduce the magnitude.
fn identity_result(value: Fp80, below: bool, rc: RoundingMode, ef: &mut ExceptionFlags) -> Fp80 {
    let exponent = leading_exponent(value);
    first_term_result(
        DoubleF64::from_fp80_scaled(value, exponent),
        below,
        exponent,
        rc,
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

/// The significand of p/2, half the 66-bit internal pi p = 4 x 0.C90FDAA22168C234C hex of
/// the trigonometric argument reduction (80486 PRM 17.5). p/2 is this value x 2^-67.
const HALF_P_SIGNIFICAND: u128 = 0xC_90FD_AA22_168C_234C;

/// The exponent of the lowest bit of [`HALF_P_SIGNIFICAND`], the unit of the reduction.
const HALF_P_UNIT_EXPONENT: i32 = -67;

/// The number of fraction bits of an Fp80 significand.
const SIGNIFICAND_FRACTION_BITS: i32 = 63;

/// The largest shift of one division step of the reduction. A remainder below 2^68 shifted
/// by it stays below 2^128.
const REDUCTION_STEP_BITS: i32 = 60;

/// Reduces the finite operand `x` with |`x`| below 2^63 to {r, N} with x = N x p/2 + r
/// exactly and N the quotient x/(p/2) rounded to nearest with ties to even (80486 PRM
/// 17.5). An operand below 1/2 is its own remainder with N = 0.
fn trig_reduce(x: Fp80) -> (DoubleF64, i64) {
    if leading_exponent(x) < -1 {
        let significand = i128::from(x.significand());
        let value = if x.sign() { -significand } else { significand };
        let exponent = x.true_exponent() - SIGNIFICAND_FRACTION_BITS;
        return (DoubleF64::from_i128_scaled(value, exponent), 0);
    }
    let (units, n) = trig_reduce_units(x);
    (DoubleF64::from_i128_scaled(units, HALF_P_UNIT_EXPONENT), n)
}

/// Reduces the finite operand `x` with 1/2 <= |`x`| < 2^63 to {R, N} with x = N x p/2 +
/// R x 2^-67 exactly and N the quotient x/(p/2) rounded to nearest with ties to even.
fn trig_reduce_units(x: Fp80) -> (i128, i64) {
    let mut quotient = 0_u64;
    let mut remainder = u128::from(x.significand());
    let mut shift = x.true_exponent() - SIGNIFICAND_FRACTION_BITS - HALF_P_UNIT_EXPONENT;
    while shift > 0 {
        let step = shift.min(REDUCTION_STEP_BITS);
        remainder <<= step;
        quotient = (quotient << step) | (remainder / HALF_P_SIGNIFICAND) as u64;
        remainder %= HALF_P_SIGNIFICAND;
        shift -= step;
    }
    let mut units = remainder as i128;
    let twice_remainder = 2 * remainder;
    if twice_remainder > HALF_P_SIGNIFICAND
        || (twice_remainder == HALF_P_SIGNIFICAND && quotient & 1 != 0)
    {
        quotient += 1;
        units -= HALF_P_SIGNIFICAND as i128;
    }
    let n = quotient as i64;
    if x.sign() { (-units, -n) } else { (units, n) }
}

/// Rounds the double-double result `value` of a function in the mode `rc`. A value on a
/// 64-bit value or halfway between two of them can hide terms below its precision, so the
/// sign of `residual`, the exact result minus `value`, then decides the rounding as a
/// remainder below every bit of the value.
fn round_with_residual(
    value: DoubleF64,
    residual: impl FnOnce() -> DoubleF64,
    rc: RoundingMode,
    ef: &mut ExceptionFlags,
) -> Fp80 {
    if value.is_exact_or_halfway() {
        let residual = residual();
        if !residual.is_zero() {
            let below = residual.is_sign_negative() != value.is_sign_negative();
            return value.to_fp80_with_remainder(0, below, rc, ef);
        }
    }
    value.to_fp80(rc, ef)
}

/// Returns sin(`r`) minus `sine`, a double-double value near sin(`r`), from the exact
/// leading term `r` and the series of the remaining terms.
fn sine_residual(r: DoubleF64, sine: DoubleF64) -> DoubleF64 {
    let square = r * r;
    (r - sine) + r * square * eval_poly(square, &SIN_ARR[1..])
}

/// Returns cos(`r`) minus `cosine`, a double-double value near cos(`r`), from the exact
/// leading term 1 and the series of the remaining terms.
fn cosine_residual(r: DoubleF64, cosine: DoubleF64) -> DoubleF64 {
    let square = r * r;
    (DoubleF64::ONE - cosine) + square * eval_poly(square, &COS_ARR[1..])
}

/// Returns tan(`r`) - `r` = (sin(r) - r cos(r)) / cos(r) from the series of sin(r) and
/// cos(r) beyond their leading terms, with `cosine` near cos(`r`).
fn tangent_tail(r: DoubleF64, cosine: DoubleF64) -> DoubleF64 {
    let square = r * r;
    let difference = eval_poly(square, &SIN_ARR[1..]) - eval_poly(square, &COS_ARR[1..]);
    r * square * difference / cosine
}

/// Rounds sin(`r` + `quadrant` x pi/2) in the mode `rc`, with `r` the exactly reduced
/// argument and `quadrant` in 0 to 3.
fn sine_in_quadrant(
    r: DoubleF64,
    quadrant: i64,
    rc: RoundingMode,
    ef: &mut ExceptionFlags,
) -> Fp80 {
    let cosine_term = quadrant % 2 == 1;
    let negative = quadrant >= 2;
    let term = if cosine_term {
        cos_poly(r)
    } else {
        sin_poly(r)
    };
    let value = if negative { term.negate() } else { term };
    let residual = || {
        let residual = if cosine_term {
            cosine_residual(r, term)
        } else {
            sine_residual(r, term)
        };
        if negative {
            residual.negate()
        } else {
            residual
        }
    };
    round_with_residual(value, residual, rc, ef)
}

/// Returns atan(`y`/`x`) of the finite nonzero `y` and the positive finite `x` with an
/// exponent difference below [`FPATAN_SMALL_RATIO_LIMIT`], rounded in the mode `rc`. The
/// quotient of the significands is exact to 127 bits with its remainder, and the remaining
/// terms q^3/3 - q^5/5 + ... are subtracted in units of the lowest quotient bit. The
/// fractional parts of the remainder and of those terms decide the sticky remainder.
fn small_ratio_arctangent(y: Fp80, x: Fp80, rc: RoundingMode, ef: &mut ExceptionFlags) -> Fp80 {
    let y_exponent = leading_exponent(y);
    let x_exponent = leading_exponent(x);
    let difference = y_exponent - x_exponent;
    let dividend = u128::from(y.significand() << y.significand().leading_zeros());
    let divisor = u128::from(x.significand() << x.significand().leading_zeros());
    let quotient_shift: u32 = if dividend >= divisor { 126 } else { 127 };
    let mut quotient = dividend / divisor;
    let mut remainder = dividend % divisor;
    let mut shift = quotient_shift;
    while shift > 0 {
        let step = shift.min(QUOTIENT_STEP_BITS);
        remainder <<= step;
        quotient = (quotient << step) | (remainder / divisor);
        remainder %= divisor;
        shift -= step;
    }
    let (tail_integer, tail_fraction) = if difference < FPATAN_STICKY_TAIL_LIMIT {
        (0, 0.0)
    } else {
        let ratio =
            DoubleF64::from_fp80_scaled(y, y_exponent) / DoubleF64::from_fp80_scaled(x, x_exponent);
        let square = ratio * ratio * DoubleF64::from_i128_scaled(1, 2 * difference);
        let tail_over_quotient = (square * eval_poly(square, &ATAN_ARR[1..])).negate();
        (DoubleF64::from_i128_scaled(quotient as i128, 0) * tail_over_quotient).floor_and_fraction()
    };
    let remainder_fraction = remainder as f64 / divisor as f64;
    let sticky = if remainder_fraction > tail_fraction {
        1
    } else {
        -1
    };
    let combined = ((quotient - tail_integer as u128) << 1).wrapping_add_signed(sticky);
    let exponent = difference - quotient_shift as i32 + 126;
    Fp80::round_and_pack(y.sign(), exponent, combined, rc, Precision::Extended, ef)
}

fn compute_pi_f80(sign: bool, rc: RoundingMode, ef: &mut ExceptionFlags) -> Fp80 {
    DD_PI.with_sign(sign).to_fp80(rc, ef)
}

fn compute_pi2_f80(sign: bool, rc: RoundingMode, ef: &mut ExceptionFlags) -> Fp80 {
    DD_PI2.with_sign(sign).to_fp80(rc, ef)
}

fn compute_pi4_f80(sign: bool, rc: RoundingMode, ef: &mut ExceptionFlags) -> Fp80 {
    DD_PI4.with_sign(sign).to_fp80(rc, ef)
}

fn compute_3pi4_f80(sign: bool, rc: RoundingMode, ef: &mut ExceptionFlags) -> Fp80 {
    dd_3pi4().with_sign(sign).to_fp80(rc, ef)
}

impl Fp80 {
    /// Compute 2^self − 1 (F2XM1). Domain: −1.0 ≤ self ≤ +1.0.
    /// The result rounds in the mode `rc` (80486 PRM 15.2.3).
    pub fn f2xm1(self, rc: RoundingMode, ef: &mut ExceptionFlags) -> Fp80 {
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
            return first_term_result(term, self.sign(), exponent, rc, ef);
        }

        let x = DoubleF64::from_fp80(self);
        let y = x * DD_LN2;

        let mut result = eval_poly(y, &EXP_ARR);
        result = result * y;

        result.to_fp80(rc, ef)
    }

    /// Compute y × log₂(self) (FYL2X). self = x (ST0), y = ST1.
    /// The result rounds in the mode `rc` (80486 PRM 15.2.3).
    pub fn fyl2x(self, y: Fp80, rc: RoundingMode, ef: &mut ExceptionFlags) -> Fp80 {
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
        result.to_fp80_scaled(y_exponent, rc, ef)
    }

    /// Compute y × log₂(self + 1) (FYL2XP1). self = x (ST0), y = ST1. An operand more
    /// negative than -1 is an invalid operation (80486 PRM table 16-11). The result rounds in
    /// the mode `rc` (80486 PRM 15.2.3).
    pub fn fyl2xp1(self, y: Fp80, rc: RoundingMode, ef: &mut ExceptionFlags) -> Fp80 {
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
            return first_term_result(term, !self.sign(), x_exponent + y_exponent - 1, rc, ef);
        }

        let x_dd = DoubleF64::from_fp80(self);
        let two = DoubleF64::ONE + DoubleF64::ONE;
        let u = x_dd / (x_dd + two);
        let log2_1px = odd_poly(u, &LN_ARR) * DD_LN2INV2;

        let result = y_dd * log2_1px;
        result.to_fp80_scaled(y_exponent, rc, ef)
    }

    /// Compute sin(self) (FSIN). Returns (result, out_of_range). The result rounds in the mode
    /// `rc` (80486 PRM 15.2.3).
    pub fn fsin(self, rc: RoundingMode, ef: &mut ExceptionFlags) -> (Fp80, bool) {
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
        if is_first_term_operand(self, TRIG_FIRST_TERM_EXPONENT_LIMIT) {
            return (identity_result(self, true, rc, ef), false);
        }
        if is_trig_out_of_range(self) {
            return (self, true);
        }

        let (r, n) = trig_reduce(self);
        (sine_in_quadrant(r, n.rem_euclid(4), rc, ef), false)
    }

    /// Compute cos(self) (FCOS). Returns (result, out_of_range). The result rounds in the mode
    /// `rc` (80486 PRM 15.2.3).
    pub fn fcos(self, rc: RoundingMode, ef: &mut ExceptionFlags) -> (Fp80, bool) {
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
        if is_first_term_operand(self, TRIG_FIRST_TERM_EXPONENT_LIMIT) {
            return (first_term_result(DoubleF64::ONE, true, 0, rc, ef), false);
        }
        if is_trig_out_of_range(self) {
            return (self, true);
        }

        let (r, n) = trig_reduce(self);
        (sine_in_quadrant(r, (n + 1).rem_euclid(4), rc, ef), false)
    }

    /// Compute (sin(self), cos(self)) simultaneously (FSINCOS). Both results round in the
    /// mode `rc` (80486 PRM 15.2.3). Observed behaviour: C1 is set when either result rounds
    /// up.
    pub fn fsincos(self, rc: RoundingMode, ef: &mut ExceptionFlags) -> (Fp80, Fp80, bool) {
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
        if is_first_term_operand(self, TRIG_FIRST_TERM_EXPONENT_LIMIT) {
            let cosine = first_term_result(DoubleF64::ONE, true, 0, rc, ef);
            return (identity_result(self, true, rc, ef), cosine, false);
        }
        if is_trig_out_of_range(self) {
            return (self, Fp80::ONE, true);
        }

        let (r, n) = trig_reduce(self);

        let mut ef_sin = ExceptionFlags::default();
        let mut ef_cos = ExceptionFlags::default();
        let sin_fp80 = sine_in_quadrant(r, n.rem_euclid(4), rc, &mut ef_sin);
        let cos_fp80 = sine_in_quadrant(r, (n + 1).rem_euclid(4), rc, &mut ef_cos);
        ef.merge(&ef_cos);
        ef.merge(&ef_sin);
        (sin_fp80, cos_fp80, false)
    }

    /// Compute tan(self) (FPTAN). Returns (tan, out_of_range). The result rounds in the mode
    /// `rc` (80486 PRM 15.2.3).
    pub fn fptan(self, rc: RoundingMode, ef: &mut ExceptionFlags) -> (Fp80, bool) {
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
        if is_first_term_operand(self, TRIG_FIRST_TERM_EXPONENT_LIMIT) {
            return (identity_result(self, false, rc, ef), false);
        }
        if is_trig_out_of_range(self) {
            return (self, true);
        }

        let (r, n) = trig_reduce(self);
        let c = cos_poly(r);
        if n.rem_euclid(2) == 0 {
            let tangent = r + tangent_tail(r, c);
            let residual = || (r - tangent) + tangent_tail(r, c);
            return (round_with_residual(tangent, residual, rc, ef), false);
        }

        let s = sin_poly(r);
        let (num, den) = if n.rem_euclid(4) == 1 {
            (c, s.negate())
        } else {
            (c.negate(), s)
        };
        let result = num / den;
        let residual = || (num - result * den) / den;
        (round_with_residual(result, residual, rc, ef), false)
    }

    /// Compute atan2(self, x) (FPATAN). self = y (ST1), x = ST0. The result rounds in the mode
    /// `rc` (80486 PRM 15.2.3).
    pub fn fpatan(self, x: Fp80, rc: RoundingMode, ef: &mut ExceptionFlags) -> Fp80 {
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
                    return compute_pi_f80(y_sign, rc, ef);
                }
                return y;
            }
            if x.is_negative() {
                return compute_pi_f80(y_sign, rc, ef);
            }
            return y;
        }

        if x.is_zero() {
            return compute_pi2_f80(y_sign, rc, ef);
        }

        if y.is_infinity() && x.is_infinity() {
            if x_sign {
                return compute_3pi4_f80(y_sign, rc, ef);
            }
            return compute_pi4_f80(y_sign, rc, ef);
        }

        if y.is_infinity() {
            return compute_pi2_f80(y_sign, rc, ef);
        }

        if x.is_infinity() {
            if x_sign {
                return compute_pi_f80(y_sign, rc, ef);
            }
            return if y_sign { Fp80::NEG_ZERO } else { Fp80::ZERO };
        }

        let y_exponent = leading_exponent(y);
        let x_exponent = leading_exponent(x);
        if x_sign && y_exponent - x_exponent < FIRST_TERM_EXPONENT_LIMIT {
            return compute_pi_f80(y_sign, rc, ef);
        }
        if !x_sign && y_exponent - x_exponent < FPATAN_SMALL_RATIO_LIMIT {
            return small_ratio_arctangent(y, x, rc, ef);
        }
        if x_exponent - y_exponent < FIRST_TERM_EXPONENT_LIMIT {
            return compute_pi2_f80(y_sign, rc, ef);
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
        atan_approx.with_sign(y_sign).to_fp80(rc, ef)
    }
}

/// Returns log2 of the finite positive `x` as n + log2(f) with x = f x 2^n and f in
/// [t/2, t), where t = 0xB504_F333_F9DE_6484 x 2^-63 is sqrt(2) truncated to 64 bits. The
/// scaling by 2^-n is exact and f keeps every significand bit of `x`.
fn compute_log2(x: Fp80) -> DoubleF64 {
    let significand = x.significand();

    if significand == 0 {
        return DoubleF64::ZERO;
    }

    let mut n = leading_exponent(x);
    let sqrt2_sig: u64 = 0xB504_F333_F9DE_6484;
    if significand << significand.leading_zeros() >= sqrt2_sig {
        n += 1;
    }

    let f = DoubleF64::from_fp80_scaled(x, n);

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
        let result = Fp80::ZERO.f2xm1(RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::ZERO);

        // f2xm1(-0) = -0.
        let mut ef = no_exceptions();
        let result = Fp80::NEG_ZERO.f2xm1(RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::NEG_ZERO);
    }

    #[test]
    fn test_f2xm1_edge_cases() {
        // f2xm1(-1.0) = -0.5 + PE.
        let mut ef = no_exceptions();
        let result = NEGATIVE_ONE.f2xm1(RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, NEGATIVE_HALF);
        assert!(ef.precision);

        // f2xm1(+1.0) = +1.0 + PE.
        let mut ef = no_exceptions();
        let result = Fp80::ONE.f2xm1(RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::ONE);
        assert!(ef.precision);

        // SNaN -> QNaN + IE.
        let mut ef = no_exceptions();
        let result = POSITIVE_SNAN.f2xm1(RoundingMode::NearestEven, &mut ef);
        assert!(result.is_quiet_nan());
        assert!(ef.invalid);

        // QNaN -> QNaN, no IE.
        let mut ef = no_exceptions();
        let result = POSITIVE_QNAN.f2xm1(RoundingMode::NearestEven, &mut ef);
        assert!(result.is_quiet_nan());
        assert!(!ef.invalid);
    }

    #[test]
    fn test_fyl2x_basic() {
        // fyl2x(x=2, y=1) = 1 * log2(2) = 1.0.
        let two = Fp80::from_bits(0x4000, 0x8000_0000_0000_0000);
        let mut ef = no_exceptions();
        let result = two.fyl2x(Fp80::ONE, RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::ONE);

        // fyl2x(x=1, y=1) = 1 * log2(1) = 0.
        let mut ef = no_exceptions();
        let result = Fp80::ONE.fyl2x(Fp80::ONE, RoundingMode::NearestEven, &mut ef);
        assert!(result.is_zero());
    }

    #[test]
    fn test_fyl2x_edge_cases() {
        // x < 0 -> Indefinite + IE.
        let mut ef = no_exceptions();
        let result = NEGATIVE_ONE.fyl2x(Fp80::ONE, RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::INDEFINITE);
        assert!(ef.invalid);

        // x = 0, y = 0 -> Indefinite + IE.
        let mut ef = no_exceptions();
        let result = Fp80::ZERO.fyl2x(Fp80::ZERO, RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::INDEFINITE);
        assert!(ef.invalid);

        // x = 0, y = positive nonzero -> -inf + ZE (sign opposite to y).
        let mut ef = no_exceptions();
        let result = Fp80::ZERO.fyl2x(Fp80::ONE, RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::NEG_INFINITY);
        assert!(ef.zero_divide);

        // x = 0, y = negative nonzero -> +inf + ZE.
        let mut ef = no_exceptions();
        let result = Fp80::ZERO.fyl2x(NEGATIVE_ONE, RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::INFINITY);
        assert!(ef.zero_divide);

        // x = +inf, y = 0 -> Indefinite + IE.
        let mut ef = no_exceptions();
        let result = Fp80::INFINITY.fyl2x(Fp80::ZERO, RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::INDEFINITE);
        assert!(ef.invalid);

        // x = 1, y = inf -> Indefinite + IE (0 * inf is indeterminate).
        let mut ef = no_exceptions();
        let result = Fp80::ONE.fyl2x(Fp80::INFINITY, RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::INDEFINITE);
        assert!(ef.invalid);

        // x = +inf, y = positive -> +inf.
        let mut ef = no_exceptions();
        let result = Fp80::INFINITY.fyl2x(Fp80::ONE, RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::INFINITY);

        // x = +inf, y = negative -> -inf.
        let mut ef = no_exceptions();
        let result = Fp80::INFINITY.fyl2x(NEGATIVE_ONE, RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::NEG_INFINITY);

        // SNaN propagation.
        let mut ef = no_exceptions();
        let result = POSITIVE_SNAN.fyl2x(Fp80::ONE, RoundingMode::NearestEven, &mut ef);
        assert!(result.is_quiet_nan());
        assert!(ef.invalid);

        let mut ef = no_exceptions();
        let result = Fp80::ONE.fyl2x(POSITIVE_SNAN, RoundingMode::NearestEven, &mut ef);
        assert!(result.is_quiet_nan());
        assert!(ef.invalid);

        // QNaN propagation: QNaN as x, no IE.
        let mut ef = no_exceptions();
        let result = POSITIVE_QNAN.fyl2x(Fp80::ONE, RoundingMode::NearestEven, &mut ef);
        assert!(result.is_quiet_nan());
        assert!(!ef.invalid);

        // QNaN propagation: QNaN as y, no IE.
        let mut ef = no_exceptions();
        let result = Fp80::ONE.fyl2x(POSITIVE_QNAN, RoundingMode::NearestEven, &mut ef);
        assert!(result.is_quiet_nan());
        assert!(!ef.invalid);
    }

    #[test]
    fn test_fyl2xp1_basic() {
        // fyl2xp1(x=+0, y=ONE) = +0 (sign = sign(x) XOR sign(y) = 0 XOR 0 = +0).
        let mut ef = no_exceptions();
        let result = Fp80::ZERO.fyl2xp1(Fp80::ONE, RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::ZERO);
        assert!(!result.sign());

        // fyl2xp1(x=-0, y=ONE) = -0 (sign = 1 XOR 0 = 1).
        let mut ef = no_exceptions();
        let result = Fp80::NEG_ZERO.fyl2xp1(Fp80::ONE, RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::NEG_ZERO);
    }

    #[test]
    fn test_fyl2xp1_edge_cases() {
        // fyl2xp1(x=+0, y=negative) = -0 (sign = 0 XOR 1 = 1).
        let mut ef = no_exceptions();
        let result = Fp80::ZERO.fyl2xp1(NEGATIVE_ONE, RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::NEG_ZERO);

        // fyl2xp1(x=-0, y=negative) = +0 (sign = 1 XOR 1 = 0).
        let mut ef = no_exceptions();
        let result = Fp80::NEG_ZERO.fyl2xp1(NEGATIVE_ONE, RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::ZERO);
        assert!(!result.sign());

        // SNaN propagation.
        let mut ef = no_exceptions();
        let result = POSITIVE_SNAN.fyl2xp1(Fp80::ONE, RoundingMode::NearestEven, &mut ef);
        assert!(result.is_quiet_nan());
        assert!(ef.invalid);

        let mut ef = no_exceptions();
        let result = Fp80::ZERO.fyl2xp1(POSITIVE_SNAN, RoundingMode::NearestEven, &mut ef);
        assert!(result.is_quiet_nan());
        assert!(ef.invalid);

        // QNaN propagation: QNaN as x, no IE.
        let mut ef = no_exceptions();
        let result = POSITIVE_QNAN.fyl2xp1(Fp80::ONE, RoundingMode::NearestEven, &mut ef);
        assert!(result.is_quiet_nan());
        assert!(!ef.invalid);

        // QNaN propagation: QNaN as y, no IE.
        let mut ef = no_exceptions();
        let result = Fp80::ZERO.fyl2xp1(POSITIVE_QNAN, RoundingMode::NearestEven, &mut ef);
        assert!(result.is_quiet_nan());
        assert!(!ef.invalid);
    }

    #[test]
    fn test_fsin_basic() {
        // sin(+0) = +0.
        let mut ef = no_exceptions();
        let (result, out_of_range) = Fp80::ZERO.fsin(RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::ZERO);
        assert!(!out_of_range);

        // sin(-0) = -0.
        let mut ef = no_exceptions();
        let (result, out_of_range) = Fp80::NEG_ZERO.fsin(RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::NEG_ZERO);
        assert!(!out_of_range);
    }

    #[test]
    fn test_fsin_edge_cases() {
        // sin(+inf) = Indefinite + IE.
        let mut ef = no_exceptions();
        let (result, _) = Fp80::INFINITY.fsin(RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::INDEFINITE);
        assert!(ef.invalid);

        // sin(-inf) = Indefinite + IE.
        let mut ef = no_exceptions();
        let (result, _) = Fp80::NEG_INFINITY.fsin(RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::INDEFINITE);
        assert!(ef.invalid);

        // |x| >= 2^63 -> (x unchanged, C2=true).
        let mut ef = no_exceptions();
        let (result, out_of_range) = OUT_OF_RANGE.fsin(RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, OUT_OF_RANGE);
        assert!(out_of_range);

        // SNaN -> QNaN + IE.
        let mut ef = no_exceptions();
        let (result, _) = POSITIVE_SNAN.fsin(RoundingMode::NearestEven, &mut ef);
        assert!(result.is_quiet_nan());
        assert!(ef.invalid);

        // QNaN -> QNaN, no IE.
        let mut ef = no_exceptions();
        let (result, _) = POSITIVE_QNAN.fsin(RoundingMode::NearestEven, &mut ef);
        assert!(result.is_quiet_nan());
        assert!(!ef.invalid);
    }

    #[test]
    fn test_fcos_basic() {
        // cos(+0) = +1.0.
        let mut ef = no_exceptions();
        let (result, out_of_range) = Fp80::ZERO.fcos(RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::ONE);
        assert!(!out_of_range);

        // cos(-0) = +1.0.
        let mut ef = no_exceptions();
        let (result, out_of_range) = Fp80::NEG_ZERO.fcos(RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::ONE);
        assert!(!out_of_range);
    }

    #[test]
    fn test_fcos_edge_cases() {
        // cos(+inf) = Indefinite + IE.
        let mut ef = no_exceptions();
        let (result, _) = Fp80::INFINITY.fcos(RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::INDEFINITE);
        assert!(ef.invalid);

        // cos(-inf) = Indefinite + IE.
        let mut ef = no_exceptions();
        let (result, _) = Fp80::NEG_INFINITY.fcos(RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::INDEFINITE);
        assert!(ef.invalid);

        // |x| >= 2^63 -> (x unchanged, C2=true).
        let mut ef = no_exceptions();
        let (result, out_of_range) = OUT_OF_RANGE.fcos(RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, OUT_OF_RANGE);
        assert!(out_of_range);

        // SNaN -> QNaN + IE.
        let mut ef = no_exceptions();
        let (result, _) = POSITIVE_SNAN.fcos(RoundingMode::NearestEven, &mut ef);
        assert!(result.is_quiet_nan());
        assert!(ef.invalid);

        // QNaN -> QNaN, no IE.
        let mut ef = no_exceptions();
        let (result, _) = POSITIVE_QNAN.fcos(RoundingMode::NearestEven, &mut ef);
        assert!(result.is_quiet_nan());
        assert!(!ef.invalid);
    }

    #[test]
    fn test_fsincos_basic() {
        // fsincos(+0) = (sin=+0, cos=+1.0, false).
        let mut ef = no_exceptions();
        let (sin, cos, out_of_range) = Fp80::ZERO.fsincos(RoundingMode::NearestEven, &mut ef);
        assert_eq!(sin, Fp80::ZERO);
        assert_eq!(cos, Fp80::ONE);
        assert!(!out_of_range);

        // fsincos(-0) = (sin=-0, cos=+1.0, false).
        let mut ef = no_exceptions();
        let (sin, cos, out_of_range) = Fp80::NEG_ZERO.fsincos(RoundingMode::NearestEven, &mut ef);
        assert_eq!(sin, Fp80::NEG_ZERO);
        assert_eq!(cos, Fp80::ONE);
        assert!(!out_of_range);
    }

    #[test]
    fn test_fsincos_edge_cases() {
        // fsincos(+inf) -> Indefinite + IE.
        let mut ef = no_exceptions();
        let (sin, _, _) = Fp80::INFINITY.fsincos(RoundingMode::NearestEven, &mut ef);
        assert_eq!(sin, Fp80::INDEFINITE);
        assert!(ef.invalid);

        // Out of range.
        let mut ef = no_exceptions();
        let (result, _, out_of_range) = OUT_OF_RANGE.fsincos(RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, OUT_OF_RANGE);
        assert!(out_of_range);

        // SNaN -> QNaN + IE.
        let mut ef = no_exceptions();
        let (result, _, _) = POSITIVE_SNAN.fsincos(RoundingMode::NearestEven, &mut ef);
        assert!(result.is_quiet_nan());
        assert!(ef.invalid);

        // fsincos(-inf) -> Indefinite + IE.
        let mut ef = no_exceptions();
        let (sin, _, _) = Fp80::NEG_INFINITY.fsincos(RoundingMode::NearestEven, &mut ef);
        assert_eq!(sin, Fp80::INDEFINITE);
        assert!(ef.invalid);

        // QNaN -> QNaN, no IE.
        let mut ef = no_exceptions();
        let (sin, cos, _) = POSITIVE_QNAN.fsincos(RoundingMode::NearestEven, &mut ef);
        assert!(sin.is_quiet_nan());
        assert!(cos.is_quiet_nan());
        assert!(!ef.invalid);
    }

    #[test]
    fn test_fptan_basic() {
        // fptan(+0) = +0.
        let mut ef = no_exceptions();
        let (result, out_of_range) = Fp80::ZERO.fptan(RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::ZERO);
        assert!(!out_of_range);

        // fptan(-0) = -0.
        let mut ef = no_exceptions();
        let (result, out_of_range) = Fp80::NEG_ZERO.fptan(RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::NEG_ZERO);
        assert!(!out_of_range);
    }

    #[test]
    fn test_fptan_edge_cases() {
        // fptan(+inf) = Indefinite + IE.
        let mut ef = no_exceptions();
        let (result, _) = Fp80::INFINITY.fptan(RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::INDEFINITE);
        assert!(ef.invalid);

        // fptan(-inf) = Indefinite + IE.
        let mut ef = no_exceptions();
        let (result, _) = Fp80::NEG_INFINITY.fptan(RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::INDEFINITE);
        assert!(ef.invalid);

        // |x| >= 2^63 -> (x unchanged, C2=true).
        let mut ef = no_exceptions();
        let (result, out_of_range) = OUT_OF_RANGE.fptan(RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, OUT_OF_RANGE);
        assert!(out_of_range);

        // SNaN -> QNaN + IE.
        let mut ef = no_exceptions();
        let (result, _) = POSITIVE_SNAN.fptan(RoundingMode::NearestEven, &mut ef);
        assert!(result.is_quiet_nan());
        assert!(ef.invalid);

        // QNaN -> QNaN, no IE.
        let mut ef = no_exceptions();
        let (result, _) = POSITIVE_QNAN.fptan(RoundingMode::NearestEven, &mut ef);
        assert!(result.is_quiet_nan());
        assert!(!ef.invalid);
    }

    #[test]
    fn test_fpatan_basic() {
        // fpatan(y=+0, x=+ONE) = +0 (atan2(+0, positive) = +0).
        let mut ef = no_exceptions();
        let result = Fp80::ZERO.fpatan(Fp80::ONE, RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::ZERO);
        assert!(!result.sign());

        // fpatan(y=-0, x=+ONE) = -0 (atan2(-0, positive) = -0).
        let mut ef = no_exceptions();
        let result = Fp80::NEG_ZERO.fpatan(Fp80::ONE, RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::NEG_ZERO);
    }

    #[test]
    fn test_fpatan_edge_cases() {
        // fpatan(y=+0, x=-ONE) = +pi.
        let mut ef = no_exceptions();
        let result = Fp80::ZERO.fpatan(NEGATIVE_ONE, RoundingMode::NearestEven, &mut ef);
        assert!(result.is_normal());
        assert!(!result.sign()); // positive pi

        // fpatan(y=-0, x=-ONE) = -pi.
        let mut ef = no_exceptions();
        let result = Fp80::NEG_ZERO.fpatan(NEGATIVE_ONE, RoundingMode::NearestEven, &mut ef);
        assert!(result.is_normal());
        assert!(result.sign()); // negative pi

        // fpatan(y=+ONE, x=+0) = +pi/2.
        let mut ef = no_exceptions();
        let result = Fp80::ONE.fpatan(Fp80::ZERO, RoundingMode::NearestEven, &mut ef);
        assert!(result.is_normal());
        assert!(!result.sign());

        // fpatan(y=-ONE, x=+0) = -pi/2.
        let mut ef = no_exceptions();
        let result = NEGATIVE_ONE.fpatan(Fp80::ZERO, RoundingMode::NearestEven, &mut ef);
        assert!(result.is_normal());
        assert!(result.sign());

        // fpatan(y=+inf, x=+inf) = +pi/4.
        let mut ef = no_exceptions();
        let result = Fp80::INFINITY.fpatan(Fp80::INFINITY, RoundingMode::NearestEven, &mut ef);
        assert!(result.is_normal());
        assert!(!result.sign());

        // fpatan(y=+inf, x=-inf) = +3pi/4.
        let mut ef = no_exceptions();
        let result = Fp80::INFINITY.fpatan(Fp80::NEG_INFINITY, RoundingMode::NearestEven, &mut ef);
        assert!(result.is_normal());
        assert!(!result.sign());

        // fpatan(y=-inf, x=+inf) = -pi/4.
        let mut ef = no_exceptions();
        let result = Fp80::NEG_INFINITY.fpatan(Fp80::INFINITY, RoundingMode::NearestEven, &mut ef);
        assert!(result.is_normal());
        assert!(result.sign());

        // fpatan(y=-inf, x=-inf) = -3pi/4.
        let mut ef = no_exceptions();
        let result =
            Fp80::NEG_INFINITY.fpatan(Fp80::NEG_INFINITY, RoundingMode::NearestEven, &mut ef);
        assert!(result.is_normal());
        assert!(result.sign());

        // fpatan(y=+inf, x=finite) = +pi/2.
        let mut ef = no_exceptions();
        let result = Fp80::INFINITY.fpatan(Fp80::ONE, RoundingMode::NearestEven, &mut ef);
        assert!(result.is_normal());
        assert!(!result.sign());

        // fpatan(y=-inf, x=finite) = -pi/2.
        let mut ef = no_exceptions();
        let result = Fp80::NEG_INFINITY.fpatan(Fp80::ONE, RoundingMode::NearestEven, &mut ef);
        assert!(result.is_normal());
        assert!(result.sign());

        // fpatan(y=finite, x=+inf) = +0 (positive y) or -0 (negative y).
        let mut ef = no_exceptions();
        let result = Fp80::ONE.fpatan(Fp80::INFINITY, RoundingMode::NearestEven, &mut ef);
        assert!(result.is_zero());
        assert!(!result.sign());

        let mut ef = no_exceptions();
        let result = NEGATIVE_ONE.fpatan(Fp80::INFINITY, RoundingMode::NearestEven, &mut ef);
        assert!(result.is_zero());
        assert!(result.sign());

        // fpatan(y=finite, x=-inf) = +pi (positive y) or -pi (negative y).
        let mut ef = no_exceptions();
        let result = Fp80::ONE.fpatan(Fp80::NEG_INFINITY, RoundingMode::NearestEven, &mut ef);
        assert!(result.is_normal());
        assert!(!result.sign());

        let mut ef = no_exceptions();
        let result = NEGATIVE_ONE.fpatan(Fp80::NEG_INFINITY, RoundingMode::NearestEven, &mut ef);
        assert!(result.is_normal());
        assert!(result.sign());

        // SNaN propagation.
        let mut ef = no_exceptions();
        let result = POSITIVE_SNAN.fpatan(Fp80::ONE, RoundingMode::NearestEven, &mut ef);
        assert!(result.is_quiet_nan());
        assert!(ef.invalid);

        let mut ef = no_exceptions();
        let result = Fp80::ONE.fpatan(POSITIVE_SNAN, RoundingMode::NearestEven, &mut ef);
        assert!(result.is_quiet_nan());
        assert!(ef.invalid);

        // QNaN propagation, no IE.
        let mut ef = no_exceptions();
        let result = POSITIVE_QNAN.fpatan(Fp80::ONE, RoundingMode::NearestEven, &mut ef);
        assert!(result.is_quiet_nan());
        assert!(!ef.invalid);

        let mut ef = no_exceptions();
        let result = Fp80::ONE.fpatan(POSITIVE_QNAN, RoundingMode::NearestEven, &mut ef);
        assert!(result.is_quiet_nan());
        assert!(!ef.invalid);

        // fpatan(y=+ONE, x=-0) = +pi/2.
        let mut ef = no_exceptions();
        let result = Fp80::ONE.fpatan(Fp80::NEG_ZERO, RoundingMode::NearestEven, &mut ef);
        assert!(result.is_normal());
        assert!(!result.sign());

        // fpatan(y=-ONE, x=-0) = -pi/2.
        let mut ef = no_exceptions();
        let result = NEGATIVE_ONE.fpatan(Fp80::NEG_ZERO, RoundingMode::NearestEven, &mut ef);
        assert!(result.is_normal());
        assert!(result.sign());
    }

    #[test]
    fn test_f2xm1_intermediate_values() {
        let half = Fp80::from_bits(0x3FFE, 0x8000_0000_0000_0000);

        // f2xm1(0.5) = sqrt(2) - 1 ≈ 0.4142.
        // Result should be positive, in range [0.25, 0.5), so true_exponent = -2.
        let mut ef = no_exceptions();
        let result = half.f2xm1(RoundingMode::NearestEven, &mut ef);
        assert!(!result.is_zero());
        assert!(!result.is_negative());
        assert_eq!(result.true_exponent(), -2);
        assert!(ef.precision);

        // f2xm1(-0.5) = 1/sqrt(2) - 1 ≈ -0.2929.
        // Result should be negative, in range (-0.5, -0.25], so true_exponent = -2.
        let neg_half = Fp80::from_bits(0xBFFE, 0x8000_0000_0000_0000);
        let mut ef = no_exceptions();
        let result = neg_half.f2xm1(RoundingMode::NearestEven, &mut ef);
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
        let result = four.fyl2x(Fp80::ONE, RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, two);

        // fyl2x(x=0.5, y=1) = log2(0.5) = -1.0.
        let half = Fp80::from_bits(0x3FFE, 0x8000_0000_0000_0000);
        let mut ef = no_exceptions();
        let result = half.fyl2x(Fp80::ONE, RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, NEGATIVE_ONE);

        // fyl2x(x=8, y=1) = log2(8) = 3.0.
        let eight = Fp80::from_bits(0x4002, 0x8000_0000_0000_0000);
        let mut ef = no_exceptions();
        let result = eight.fyl2x(Fp80::ONE, RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, three);
    }

    #[test]
    fn test_fyl2x_non_power_of_two() {
        // fyl2x(x=3, y=1) = log2(3) ≈ 1.585.
        // Significand 0xC000... >= sqrt2_sig, so sqrt(2) normalization triggers.
        // Result must be in [1.0, 2.0): true_exponent = 0, positive.
        let three = Fp80::from_bits(0x4000, 0xC000_0000_0000_0000);
        let mut ef = no_exceptions();
        let result = three.fyl2x(Fp80::ONE, RoundingMode::NearestEven, &mut ef);
        assert!(!result.is_zero());
        assert!(!result.is_negative());
        assert_eq!(result.true_exponent(), 0);

        // fyl2x(x=1.5, y=1) = log2(1.5) ≈ 0.585.
        // Significand 0xC000... also >= sqrt2_sig, normalization triggers.
        // Result must be in [0.5, 1.0): true_exponent = -1, positive.
        let one_and_half = Fp80::from_bits(0x3FFF, 0xC000_0000_0000_0000);
        let mut ef = no_exceptions();
        let result = one_and_half.fyl2x(Fp80::ONE, RoundingMode::NearestEven, &mut ef);
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
            PSEUDO_DENORMAL.fsin(RoundingMode::NearestEven, &mut ef),
            (PSEUDO_DENORMAL_VALUE, false)
        );
        assert!(ef.denormal && ef.precision && !ef.underflow && ef.rounded_up);

        let mut ef = no_exceptions();
        assert_eq!(
            PSEUDO_DENORMAL.fcos(RoundingMode::NearestEven, &mut ef),
            (Fp80::ONE, false)
        );
        assert!(ef.denormal && ef.precision && ef.rounded_up);

        let mut ef = no_exceptions();
        assert_eq!(
            PSEUDO_DENORMAL.fptan(RoundingMode::NearestEven, &mut ef),
            (PSEUDO_DENORMAL_VALUE, false)
        );
        assert!(ef.denormal && ef.precision && !ef.rounded_up);

        let mut ef = no_exceptions();
        assert_eq!(
            PSEUDO_DENORMAL.fsincos(RoundingMode::NearestEven, &mut ef),
            (PSEUDO_DENORMAL_VALUE, Fp80::ONE, false)
        );
        assert!(ef.denormal && ef.precision && ef.rounded_up);

        let mut ef = no_exceptions();
        assert_eq!(
            DENORMAL.fsin(RoundingMode::NearestEven, &mut ef),
            (DENORMAL, false)
        );
        assert!(ef.denormal && ef.precision && ef.underflow && ef.rounded_up);

        let mut ef = no_exceptions();
        let negative = TINY_NORMAL.negate();
        assert_eq!(
            negative.fsin(RoundingMode::NearestEven, &mut ef),
            (negative, false)
        );
        assert!(ef.precision && ef.rounded_up && !ef.denormal && !ef.underflow);

        let mut ef = no_exceptions();
        assert_eq!(
            negative.fptan(RoundingMode::NearestEven, &mut ef),
            (negative, false)
        );
        assert!(ef.precision && !ef.rounded_up);
    }

    #[test]
    fn trigonometric_functions_at_the_first_term_limit_report_the_rounding() {
        for exponent in [0x3FDF, 0x3FDE, 0x3FBC, 0x3F9B] {
            let operand = Fp80::from_bits(exponent, 0x8000_0000_0000_0000);
            let mut ef = no_exceptions();
            assert_eq!(
                operand.fsin(RoundingMode::NearestEven, &mut ef),
                (operand, false)
            );
            assert!(ef.precision && ef.rounded_up, "{exponent:04X}");

            let mut ef = no_exceptions();
            assert_eq!(
                operand.fcos(RoundingMode::NearestEven, &mut ef),
                (Fp80::ONE, false)
            );
            assert!(ef.precision && ef.rounded_up, "{exponent:04X}");

            let mut ef = no_exceptions();
            assert_eq!(
                operand.fptan(RoundingMode::NearestEven, &mut ef),
                (operand, false)
            );
            assert!(ef.precision && !ef.rounded_up, "{exponent:04X}");

            let mut ef = no_exceptions();
            assert_eq!(
                operand.fsincos(RoundingMode::NearestEven, &mut ef),
                (operand, Fp80::ONE, false)
            );
            assert!(ef.precision && ef.rounded_up, "{exponent:04X}");
        }
    }

    #[test]
    fn exponential_and_logarithm_above_the_first_term_limit_keep_the_second_term() {
        let mut ef = no_exceptions();
        let result = Fp80::from_bits(0xBFBB, 0xB0E7_4E9F_66B6_93E4)
            .f2xm1(RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::from_bits(0xBFBA, 0xF53D_89DA_8272_BBC3));
        assert!(ef.precision && !ef.rounded_up);

        let mut ef = no_exceptions();
        let x = Fp80::from_bits(0xBFB9, 0x82A0_649A_3B40_0000);
        let y = Fp80::from_bits(0x4021, 0xFF82_A9C9_B040_0000);
        let result = x.fyl2xp1(y, RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::from_bits(0xBFDC, 0xBC18_05C4_45F3_3A8A));
        assert!(ef.precision && ef.rounded_up);
    }

    #[test]
    fn logarithm_of_a_tiny_operand_times_zero_is_exact() {
        for x in [TINY_NORMAL, TINY_NORMAL.negate()] {
            for y in [Fp80::ZERO, Fp80::NEG_ZERO] {
                let mut ef = no_exceptions();
                assert!(
                    x.fyl2xp1(y, RoundingMode::NearestEven, &mut ef).is_zero(),
                    "{x:?} {y:?}"
                );
                assert_eq!(ef, no_exceptions(), "{x:?} {y:?}");
            }
        }
    }

    #[test]
    fn exponential_and_logarithm_of_a_tiny_operand_take_the_first_term() {
        let mut ef = no_exceptions();
        let result = PSEUDO_DENORMAL.f2xm1(RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::from_bits(0x0000, 0x58B9_0BFB_E8E7_BCD7));
        assert!(ef.denormal && ef.underflow && ef.precision && ef.rounded_up);

        let mut ef = no_exceptions();
        let result = TINY_NORMAL.f2xm1(RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::from_bits(0x2FFF, 0xB172_17F7_D1CF_79AC));
        assert!(ef.precision && ef.rounded_up && !ef.underflow);

        let mut ef = no_exceptions();
        let result = PSEUDO_DENORMAL.fyl2xp1(Fp80::ONE, RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::from_bits(0x0001, 0xB8AA_3B29_5C17_F0BD));
        assert!(ef.denormal && ef.precision && !ef.rounded_up && !ef.underflow);

        let mut ef = no_exceptions();
        let result = TINY_NORMAL.fyl2xp1(Fp80::ONE, RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::from_bits(0x3000, 0xB8AA_3B29_5C17_F0BC));
        assert!(ef.precision && ef.rounded_up);
    }

    #[test]
    fn arctangent_of_operands_out_of_the_f64_range() {
        let mut ef = no_exceptions();
        let result = PSEUDO_DENORMAL.fpatan(Fp80::ONE, RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, PSEUDO_DENORMAL_VALUE);
        assert!(ef.denormal && ef.precision && ef.rounded_up && !ef.underflow);

        let mut ef = no_exceptions();
        let result = Fp80::ONE.fpatan(PSEUDO_DENORMAL, RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::from_bits(0x3FFF, 0xC90F_DAA2_2168_C235));
        assert!(ef.denormal && ef.precision && ef.rounded_up);

        let mut ef = no_exceptions();
        let result = PSEUDO_DENORMAL.fpatan(NEGATIVE_ONE, RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::PI_UP);

        let mut quarter_flags = no_exceptions();
        let quarter = Fp80::ONE.fpatan(Fp80::ONE, RoundingMode::NearestEven, &mut quarter_flags);
        let mut ef = no_exceptions();
        assert_eq!(
            PSEUDO_DENORMAL.fpatan(PSEUDO_DENORMAL, RoundingMode::NearestEven, &mut ef),
            quarter
        );
        let huge = Fp80::from_bits(0x7000, 0x8000_0000_0000_0000);
        assert_eq!(
            huge.fpatan(huge, RoundingMode::NearestEven, &mut ef),
            quarter
        );
    }

    /// A wide unsigned integer as its high and low 128 bits.
    type Wide = (u128, u128);

    /// Returns `significand` x 2^`shift` for a shift of 1 to 127.
    fn wide_shifted(significand: u64, shift: u32) -> Wide {
        let value = u128::from(significand);
        (value >> (128 - shift), value << shift)
    }

    /// Returns the full product `multiplier` x `multiplicand` for a multiplicand below 2^127.
    fn wide_product(multiplier: u64, multiplicand: u128) -> Wide {
        let lower = u128::from(multiplier) * (multiplicand & u128::from(u64::MAX));
        let upper = u128::from(multiplier) * (multiplicand >> 64);
        let (low, carry) = lower.overflowing_add(upper << 64);
        ((upper >> 64) + u128::from(carry), low)
    }

    /// Returns `minuend` - `subtrahend`, which must fit in an i128.
    fn wide_difference(minuend: Wide, subtrahend: Wide) -> i128 {
        let (low, borrow) = minuend.1.overflowing_sub(subtrahend.1);
        let high = minuend
            .0
            .wrapping_sub(subtrahend.0)
            .wrapping_sub(u128::from(borrow));
        let difference = low as i128;
        let sign_extension = if difference < 0 { u128::MAX } else { 0 };
        assert_eq!(high, sign_extension, "the difference exceeds 128 bits");
        difference
    }

    /// Returns `value` / 2^`shift` rounded down for a shift of 1 to 127. The result must fit
    /// in a u128.
    fn wide_shifted_down(value: Wide, shift: u32) -> u128 {
        assert_eq!(value.0 >> shift, 0);
        (value.1 >> shift) | (value.0 << (128 - shift))
    }

    /// The shift of a significand with the leading exponent `exponent` to units of 2^-67.
    fn units_shift(exponent: i32) -> u32 {
        (exponent - SIGNIFICAND_FRACTION_BITS - HALF_P_UNIT_EXPONENT) as u32
    }

    /// Checks the reduction of the operand with the sign `sign`, the leading exponent
    /// `exponent` and the normalized `significand` against the wide integer identity
    /// |x| x 2^67 = |N| x HALF_P_SIGNIFICAND + R with |R| <= HALF_P_SIGNIFICAND / 2 and N even
    /// on a tie.
    fn check_reduction(sign: bool, exponent: i32, significand: u64) {
        let sign_bit = if sign { 0x8000 } else { 0x0000 };
        let operand = Fp80::from_bits(sign_bit | (0x3FFF + exponent) as u16, significand);
        let (units, quotient) = trig_reduce_units(operand);
        let (units_magnitude, quotient_magnitude) = if sign {
            (-units, -quotient)
        } else {
            (units, quotient)
        };
        assert!(quotient_magnitude >= 0, "{operand:?}: N {quotient}");
        let numerator = wide_shifted(significand, units_shift(exponent));
        let product = wide_product(quotient_magnitude as u64, HALF_P_SIGNIFICAND);
        assert_eq!(
            wide_difference(numerator, product),
            units_magnitude,
            "{operand:?}: x - N x p/2 differs from r"
        );
        let twice_remainder = 2 * units.unsigned_abs();
        assert!(
            twice_remainder <= HALF_P_SIGNIFICAND,
            "{operand:?}: |r| exceeds p/4"
        );
        if twice_remainder == HALF_P_SIGNIFICAND {
            assert_eq!(quotient % 2, 0, "{operand:?}: N of a tie is odd");
        }
        assert_eq!(trig_reduce(operand).1, quotient);
    }

    /// The seed of the operand generator of the reduction tests.
    const REDUCTION_SEED: u64 = 0x9E37_79B9_7F4A_7C15;
    /// The random operands of each exponent of the reduction tests.
    const REDUCTION_RANDOM_OPERANDS: usize = 64;
    /// The operands near multiples of p/4 of each exponent of the reduction tests.
    const REDUCTION_BOUNDARY_TARGETS: usize = 16;

    /// Returns the next value of the xorshift generator `state`.
    fn next_random(state: &mut u64) -> u64 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state
    }

    #[test]
    fn trigonometric_reduction_is_exact_from_one_half_to_two_to_63() {
        let mut state = REDUCTION_SEED;
        for exponent in -1..=62 {
            for _ in 0..REDUCTION_RANDOM_OPERANDS {
                let significand = next_random(&mut state) | (1 << 63);
                check_reduction(state & 1 != 0, exponent, significand);
            }
            let shift = units_shift(exponent);
            for _ in 0..REDUCTION_BOUNDARY_TARGETS {
                let significand = next_random(&mut state) | (1 << 63);
                let quotient = (significand as f64 * 2_f64.powi(shift as i32)
                    / HALF_P_SIGNIFICAND as f64) as u64;
                let quarter_multiples = [2 * quotient, 2 * quotient + 1];
                for multiple in quarter_multiples {
                    let boundary = wide_product(multiple, HALF_P_SIGNIFICAND);
                    let nearest = wide_shifted_down(boundary, shift + 1);
                    for offset in -2_i128..=2 {
                        let candidate = nearest.wrapping_add_signed(offset);
                        if candidate >> 63 == 1 {
                            check_reduction(state & 2 != 0, exponent, candidate as u64);
                        }
                    }
                }
            }
        }
        check_reduction(false, 62, u64::MAX);
        check_reduction(true, 62, u64::MAX);
        check_reduction(false, -1, 1 << 63);
    }

    #[test]
    fn trigonometric_reduction_below_one_half_keeps_the_operand() {
        let operand = Fp80::from_bits(0xBFFD, 0xFFFF_FFFF_FFFF_FFFF);
        let (remainder, quotient) = trig_reduce(operand);
        assert_eq!(quotient, 0);
        let mut ef = no_exceptions();
        assert_eq!(
            remainder.to_fp80(RoundingMode::NearestEven, &mut ef),
            operand
        );
        assert!(!ef.precision);
    }

    /// An operand of FSIN, FCOS, FSINCOS and FPTAN with the quotient N and the results of the
    /// 66-bit pi model of 80486 PRM 17.5, each with its C1 bit.
    struct TrigModelCase {
        operand: Fp80,
        quotient: i64,
        sine: (Fp80, bool),
        cosine: (Fp80, bool),
        tangent: (Fp80, bool),
    }

    /// Operands from 2^30 to 2^62.9 with the results of the 66-bit pi model of 80486 PRM 17.5:
    /// f(r + N x pi/2) with N = x/(p/2) rounded to nearest with ties to even, r = x - N x p/2
    /// exact and p/2 = 0xC90FDAA22168C234C x 2^-67. Each result is the exact value of the model
    /// rounded to nearest at 64 bits, evaluated with 300-bit arithmetic. C1 tells whether the
    /// rounding increased the magnitude. The last operand, 1.0030 x 2^41, is the smallest
    /// operand found where an inexact reduction breaks the relative error bound 2^-62.
    const TRIG_MODEL_CASES: [TrigModelCase; 16] = [
        TrigModelCase {
            operand: Fp80::from_bits(0x401D, 0x862F_B283_4C23_2521),
            quotient: 716602400,
            sine: (Fp80::from_bits(0xBFFA, 0xB2F1_DA7A_8B25_DC7D), true),
            cosine: (Fp80::from_bits(0x3FFE, 0xFFC1_6DBE_428D_3FEF), false),
            tangent: (Fp80::from_bits(0xBFFA, 0xB31D_A1FC_ABCC_DD69), true),
        },
        TrigModelCase {
            operand: Fp80::from_bits(0x4025, 0x9119_40DC_2D3B_12EF),
            quotient: 198368791790,
            sine: (Fp80::from_bits(0xBFFC, 0x90A6_45A6_7170_9420), true),
            cosine: (Fp80::from_bits(0xBFFE, 0xFD6E_D8B7_DFAE_1F79), false),
            tangent: (Fp80::from_bits(0x3FFC, 0x921D_5964_7FAE_1C4A), false),
        },
        TrigModelCase {
            operand: Fp80::from_bits(0x4028, 0x80FA_3AC0_785C_6E9F),
            quotient: 1410632196674,
            sine: (Fp80::from_bits(0x3FFE, 0xA32D_4B17_2383_E275), false),
            cosine: (Fp80::from_bits(0xBFFE, 0xC541_1F0E_773A_71B2), true),
            tangent: (Fp80::from_bits(0xBFFE, 0xD3C6_1255_288E_C093), false),
        },
        TrigModelCase {
            operand: Fp80::from_bits(0x402C, 0xECC2_B03B_3C2D_47E9),
            quotient: 41431361921659,
            sine: (Fp80::from_bits(0xBFFE, 0xD53E_4726_1A5C_81D6), false),
            cosine: (Fp80::from_bits(0xBFFE, 0x8DA5_1ED1_FBEC_7C1C), true),
            tangent: (Fp80::from_bits(0x3FFF, 0xC0B3_774A_913B_17CC), true),
        },
        TrigModelCase {
            operand: Fp80::from_bits(0x4031, 0xB3F2_A96E_A66F_95F9),
            quotient: 1007666252758902,
            sine: (Fp80::from_bits(0x3FFD, 0xD866_DAAA_8FA1_FB16), false),
            cosine: (Fp80::from_bits(0xBFFE, 0xE802_852E_DE2F_82C9), false),
            tangent: (Fp80::from_bits(0xBFFD, 0xEEC7_2D1D_CF5F_B362), true),
        },
        TrigModelCase {
            operand: Fp80::from_bits(0x4036, 0xA3A7_4EA5_FF02_2BD3),
            quotient: 29325493478883238,
            sine: (Fp80::from_bits(0x3FFE, 0x9764_32D8_5737_89F6), false),
            cosine: (Fp80::from_bits(0xBFFE, 0xCE70_201E_6307_FD46), true),
            tangent: (Fp80::from_bits(0xBFFE, 0xBBBC_DB8B_ADDD_B08D), true),
        },
        TrigModelCase {
            operand: Fp80::from_bits(0x403B, 0xD5C1_3249_C35B_EBFB),
            quotient: 1225703742604511255,
            sine: (Fp80::from_bits(0xBFFE, 0xFBF8_4A49_5B6C_1851), false),
            cosine: (Fp80::from_bits(0xBFFC, 0xB4FB_A59C_7520_9678), false),
            tangent: (Fp80::from_bits(0x4001, 0xB234_9458_C933_D1B5), false),
        },
        TrigModelCase {
            operand: Fp80::from_bits(0x403D, 0xEED9_1687_2B02_02F8),
            quotient: 5478371679124213529,
            sine: (Fp80::from_bits(0x3FFE, 0xECC8_A33A_ADF0_DDDA), true),
            cosine: (Fp80::from_bits(0xBFFD, 0xC29F_4F36_1024_3A3A), true),
            tangent: (Fp80::from_bits(0xC000, 0x9BBA_99C6_69B4_0323), false),
        },
        TrigModelCase {
            operand: Fp80::from_bits(0x4026, 0x8330_3E52_A61F_7F39),
            quotient: 358703478562,
            sine: (Fp80::from_bits(0xBFFD, 0xBE9E_865B_6F55_80C0), true),
            cosine: (Fp80::from_bits(0xBFFE, 0xED98_B16E_5EF2_2BCA), false),
            tangent: (Fp80::from_bits(0x3FFD, 0xCD62_51B1_950D_BF5B), false),
        },
        TrigModelCase {
            operand: Fp80::from_bits(0x402E, 0x8E7C_AF85_E7BB_8BAD),
            quotient: 99736782678232,
            sine: (Fp80::from_bits(0xBFFC, 0x9A93_0F3B_CA23_53D3), true),
            cosine: (Fp80::from_bits(0x3FFE, 0xFD11_0871_D24E_25B8), true),
            tangent: (Fp80::from_bits(0xBFFC, 0x9C5D_C11C_D9E6_C746), true),
        },
        TrigModelCase {
            operand: Fp80::from_bits(0x4034, 0xA2E5_C7D7_370E_DF19),
            quotient: 7297507626696211,
            sine: (Fp80::from_bits(0xBFFE, 0xFE0A_AFCD_7CB6_29DA), false),
            cosine: (Fp80::from_bits(0xBFFB, 0xFCD4_4F74_F262_46E9), false),
            tangent: (Fp80::from_bits(0x4002, 0x809D_2264_96D2_EF93), false),
        },
        TrigModelCase {
            operand: Fp80::from_bits(0x4039, 0x9192_9AA5_F2B6_8F64),
            quotient: 208684290029722327,
            sine: (Fp80::from_bits(0xBFFE, 0xE75C_6BAF_5201_2E58), false),
            cosine: (Fp80::from_bits(0x3FFD, 0xDB29_AAE7_6734_D62A), true),
            tangent: (Fp80::from_bits(0xC000, 0x871F_D0B3_FAA0_0E4C), true),
        },
        TrigModelCase {
            operand: Fp80::from_bits(0x403C, 0xA792_F176_2BD8_B2BD),
            quotient: 1921792602070816163,
            sine: (Fp80::from_bits(0xBFFE, 0xF127_DB9C_1D87_0820), false),
            cosine: (Fp80::from_bits(0x3FFD, 0xABD0_23F0_7642_E0E6), false),
            tangent: (Fp80::from_bits(0xC000, 0xB3A8_F47B_997C_19E2), true),
        },
        TrigModelCase {
            operand: Fp80::from_bits(0x403D, 0x9112_8354_941F_A932),
            quotient: 3327472157289710967,
            sine: (Fp80::from_bits(0xBFFE, 0xFFEB_1037_BC38_CFA0), false),
            cosine: (Fp80::from_bits(0x3FF9, 0xCF0D_DAE5_3E28_DBB3), true),
            tangent: (Fp80::from_bits(0xC004, 0x9E35_2407_8F93_928A), true),
        },
        TrigModelCase {
            operand: Fp80::from_bits(0xC033, 0xF1F0_99A8_56C7_1B74),
            quotient: -5419226777728039,
            sine: (Fp80::from_bits(0x3FFE, 0xFEC0_60B9_9672_B68B), true),
            cosine: (Fp80::from_bits(0x3FFB, 0xCA04_FD2A_4ACF_9A21), false),
            tangent: (Fp80::from_bits(0x4002, 0xA169_3E37_F299_0733), false),
        },
        TrigModelCase {
            operand: Fp80::from_bits(0x4028, 0x8061_1A9D_A802_833C),
            quotient: 1404090240677,
            sine: (Fp80::from_bits(0x3FFE, 0xFFBC_B432_8049_AA91), false),
            cosine: (Fp80::from_bits(0xBFFA, 0xB993_15F2_FDCD_11E8), true),
            tangent: (Fp80::from_bits(0xC003, 0xB065_057D_114C_8A6B), true),
        },
    ];

    #[test]
    fn trigonometric_functions_follow_the_66_bit_pi_model() {
        for case in &TRIG_MODEL_CASES {
            let operand = case.operand;
            assert_eq!(trig_reduce(operand).1, case.quotient, "N of {operand:?}");

            let mut ef = no_exceptions();
            let (sine, out_of_range) = operand.fsin(RoundingMode::NearestEven, &mut ef);
            assert!(!out_of_range);
            assert_eq!(sine, case.sine.0, "FSIN {operand:?}");
            assert!(ef.precision);
            assert_eq!(ef.rounded_up, case.sine.1, "FSIN C1 {operand:?}");

            let mut ef = no_exceptions();
            let (cosine, out_of_range) = operand.fcos(RoundingMode::NearestEven, &mut ef);
            assert!(!out_of_range);
            assert_eq!(cosine, case.cosine.0, "FCOS {operand:?}");
            assert!(ef.precision);
            assert_eq!(ef.rounded_up, case.cosine.1, "FCOS C1 {operand:?}");

            let mut ef = no_exceptions();
            let (tangent, out_of_range) = operand.fptan(RoundingMode::NearestEven, &mut ef);
            assert!(!out_of_range);
            assert_eq!(tangent, case.tangent.0, "FPTAN {operand:?}");
            assert!(ef.precision);
            assert_eq!(ef.rounded_up, case.tangent.1, "FPTAN C1 {operand:?}");

            let mut ef = no_exceptions();
            let (sine, cosine, out_of_range) = operand.fsincos(RoundingMode::NearestEven, &mut ef);
            assert!(!out_of_range);
            assert_eq!(
                (sine, cosine),
                (case.sine.0, case.cosine.0),
                "FSINCOS {operand:?}"
            );
            assert!(ef.precision);
        }
    }

    /// The rounding modes in the order of the RC field values 00 to 11 (80486 PRM table 15-5).
    const ROUNDING_MODES: [RoundingMode; 4] = [
        RoundingMode::NearestEven,
        RoundingMode::Down,
        RoundingMode::Up,
        RoundingMode::Zero,
    ];

    /// A transcendental instruction with one result.
    enum Transcendental {
        Sine,
        Cosine,
        Tangent,
        TwoToXMinusOne,
        YLog2X,
        YLog2XPlusOne,
        Arctangent,
    }

    /// An operation with `first` as the receiver and `second` as the argument of the binary
    /// operations, and the result with its C1 bit in each of the [`ROUNDING_MODES`].
    struct RoundingCase {
        function: Transcendental,
        first: Fp80,
        second: Fp80,
        results: [(Fp80, bool); 4],
    }

    /// The results of the exact functions rounded at 64 bits in each rounding mode (80486 PRM
    /// table 15-5), evaluated with 320-bit arithmetic or, for an arctangent of a ratio below
    /// 2^-60, with the exact series. FSIN, FCOS and FPTAN follow the 66-bit pi reduction of
    /// 80486 PRM 17.5. The flag is C1, set when the rounding increased the magnitude. The last
    /// seven results have a leading part exact at 64 bits and terms far below it.
    const ROUNDING_CASES: [RoundingCase; 24] = [
        RoundingCase {
            function: Transcendental::Sine,
            first: Fp80::from_bits(0x3FFE, 0x8000_0000_0000_0000),
            second: Fp80::ZERO,
            results: [
                (Fp80::from_bits(0x3FFD, 0xF577_43A2_582F_7F44), true),
                (Fp80::from_bits(0x3FFD, 0xF577_43A2_582F_7F43), false),
                (Fp80::from_bits(0x3FFD, 0xF577_43A2_582F_7F44), true),
                (Fp80::from_bits(0x3FFD, 0xF577_43A2_582F_7F43), false),
            ],
        },
        RoundingCase {
            function: Transcendental::Sine,
            first: Fp80::from_bits(0xBFFE, 0xE666_6666_6666_6800),
            second: Fp80::ZERO,
            results: [
                (Fp80::from_bits(0xBFFE, 0xC888_1CC2_ED42_0723), false),
                (Fp80::from_bits(0xBFFE, 0xC888_1CC2_ED42_0724), true),
                (Fp80::from_bits(0xBFFE, 0xC888_1CC2_ED42_0723), false),
                (Fp80::from_bits(0xBFFE, 0xC888_1CC2_ED42_0723), false),
            ],
        },
        RoundingCase {
            function: Transcendental::Sine,
            first: Fp80::from_bits(0x4020, 0x9502_F900_0000_0000),
            second: Fp80::ZERO,
            results: [
                (Fp80::from_bits(0xBFFD, 0xF99A_63C4_9C6F_2B1A), false),
                (Fp80::from_bits(0xBFFD, 0xF99A_63C4_9C6F_2B1B), true),
                (Fp80::from_bits(0xBFFD, 0xF99A_63C4_9C6F_2B1A), false),
                (Fp80::from_bits(0xBFFD, 0xF99A_63C4_9C6F_2B1A), false),
            ],
        },
        RoundingCase {
            function: Transcendental::Cosine,
            first: Fp80::from_bits(0x3FFE, 0x8000_0000_0000_0000),
            second: Fp80::ZERO,
            results: [
                (Fp80::from_bits(0x3FFE, 0xE0A9_4032_DBEA_7CEE), true),
                (Fp80::from_bits(0x3FFE, 0xE0A9_4032_DBEA_7CED), false),
                (Fp80::from_bits(0x3FFE, 0xE0A9_4032_DBEA_7CEE), true),
                (Fp80::from_bits(0x3FFE, 0xE0A9_4032_DBEA_7CED), false),
            ],
        },
        RoundingCase {
            function: Transcendental::Cosine,
            first: Fp80::from_bits(0xBFFE, 0xE666_6666_6666_6800),
            second: Fp80::ZERO,
            results: [
                (Fp80::from_bits(0x3FFE, 0x9F21_D4B4_9710_E35B), false),
                (Fp80::from_bits(0x3FFE, 0x9F21_D4B4_9710_E35B), false),
                (Fp80::from_bits(0x3FFE, 0x9F21_D4B4_9710_E35C), true),
                (Fp80::from_bits(0x3FFE, 0x9F21_D4B4_9710_E35B), false),
            ],
        },
        RoundingCase {
            function: Transcendental::Tangent,
            first: Fp80::from_bits(0x3FFE, 0x8000_0000_0000_0000),
            second: Fp80::ZERO,
            results: [
                (Fp80::from_bits(0x3FFE, 0x8BDA_7ADF_9A3A_5219), true),
                (Fp80::from_bits(0x3FFE, 0x8BDA_7ADF_9A3A_5218), false),
                (Fp80::from_bits(0x3FFE, 0x8BDA_7ADF_9A3A_5219), true),
                (Fp80::from_bits(0x3FFE, 0x8BDA_7ADF_9A3A_5218), false),
            ],
        },
        RoundingCase {
            function: Transcendental::Tangent,
            first: Fp80::from_bits(0x3FFF, 0x8CCC_CCCC_CCCC_D000),
            second: Fp80::ZERO,
            results: [
                (Fp80::from_bits(0x3FFF, 0xFB7D_3E94_310A_727C), false),
                (Fp80::from_bits(0x3FFF, 0xFB7D_3E94_310A_727C), false),
                (Fp80::from_bits(0x3FFF, 0xFB7D_3E94_310A_727D), true),
                (Fp80::from_bits(0x3FFF, 0xFB7D_3E94_310A_727C), false),
            ],
        },
        RoundingCase {
            function: Transcendental::TwoToXMinusOne,
            first: Fp80::from_bits(0x3FFE, 0x8000_0000_0000_0000),
            second: Fp80::ZERO,
            results: [
                (Fp80::from_bits(0x3FFD, 0xD413_CCCF_E779_9211), false),
                (Fp80::from_bits(0x3FFD, 0xD413_CCCF_E779_9211), false),
                (Fp80::from_bits(0x3FFD, 0xD413_CCCF_E779_9212), true),
                (Fp80::from_bits(0x3FFD, 0xD413_CCCF_E779_9211), false),
            ],
        },
        RoundingCase {
            function: Transcendental::TwoToXMinusOne,
            first: Fp80::from_bits(0xBFFE, 0xE666_6666_6666_6800),
            second: Fp80::ZERO,
            results: [
                (Fp80::from_bits(0xBFFD, 0xEDA0_411D_AF99_BECE), true),
                (Fp80::from_bits(0xBFFD, 0xEDA0_411D_AF99_BECE), true),
                (Fp80::from_bits(0xBFFD, 0xEDA0_411D_AF99_BECD), false),
                (Fp80::from_bits(0xBFFD, 0xEDA0_411D_AF99_BECD), false),
            ],
        },
        RoundingCase {
            function: Transcendental::TwoToXMinusOne,
            first: Fp80::from_bits(0x3FEE, 0xA7C5_AC47_1B47_8800),
            second: Fp80::ZERO,
            results: [
                (Fp80::from_bits(0x3FED, 0xE895_1885_AF47_98D4), false),
                (Fp80::from_bits(0x3FED, 0xE895_1885_AF47_98D4), false),
                (Fp80::from_bits(0x3FED, 0xE895_1885_AF47_98D5), true),
                (Fp80::from_bits(0x3FED, 0xE895_1885_AF47_98D4), false),
            ],
        },
        RoundingCase {
            function: Transcendental::YLog2X,
            first: Fp80::from_bits(0x4000, 0xC000_0000_0000_0000),
            second: Fp80::from_bits(0x3FFE, 0xB333_3333_3333_3000),
            results: [
                (Fp80::from_bits(0x3FFF, 0x8E03_3C61_1824_ACEF), true),
                (Fp80::from_bits(0x3FFF, 0x8E03_3C61_1824_ACEE), false),
                (Fp80::from_bits(0x3FFF, 0x8E03_3C61_1824_ACEF), true),
                (Fp80::from_bits(0x3FFF, 0x8E03_3C61_1824_ACEE), false),
            ],
        },
        RoundingCase {
            function: Transcendental::YLog2X,
            first: Fp80::from_bits(0x4000, 0xA000_0000_0000_0000),
            second: Fp80::from_bits(0x3FFD, 0x9999_9999_9999_9800),
            results: [
                (Fp80::from_bits(0x3FFD, 0xCB0C_53E9_1F75_4B78), false),
                (Fp80::from_bits(0x3FFD, 0xCB0C_53E9_1F75_4B78), false),
                (Fp80::from_bits(0x3FFD, 0xCB0C_53E9_1F75_4B79), true),
                (Fp80::from_bits(0x3FFD, 0xCB0C_53E9_1F75_4B78), false),
            ],
        },
        RoundingCase {
            function: Transcendental::YLog2XPlusOne,
            first: Fp80::from_bits(0x3FFD, 0x8000_0000_0000_0000),
            second: Fp80::from_bits(0x3FFF, 0xA666_6666_6666_6800),
            results: [
                (Fp80::from_bits(0x3FFD, 0xD646_7CAD_EEB8_0DFE), true),
                (Fp80::from_bits(0x3FFD, 0xD646_7CAD_EEB8_0DFD), false),
                (Fp80::from_bits(0x3FFD, 0xD646_7CAD_EEB8_0DFE), true),
                (Fp80::from_bits(0x3FFD, 0xD646_7CAD_EEB8_0DFD), false),
            ],
        },
        RoundingCase {
            function: Transcendental::YLog2XPlusOne,
            first: Fp80::from_bits(0x3FEE, 0xA7C5_AC47_1B47_8800),
            second: Fp80::from_bits(0x3FFE, 0x8000_0000_0000_0000),
            results: [
                (Fp80::from_bits(0x3FED, 0xF20A_F86A_5B0F_A11F), true),
                (Fp80::from_bits(0x3FED, 0xF20A_F86A_5B0F_A11E), false),
                (Fp80::from_bits(0x3FED, 0xF20A_F86A_5B0F_A11F), true),
                (Fp80::from_bits(0x3FED, 0xF20A_F86A_5B0F_A11E), false),
            ],
        },
        RoundingCase {
            function: Transcendental::Arctangent,
            first: Fp80::from_bits(0x3FFF, 0x8CCC_CCCC_CCCC_D000),
            second: Fp80::from_bits(0x4000, 0xC000_0000_0000_0000),
            results: [
                (Fp80::from_bits(0x3FFD, 0xB3F0_9271_307E_0A68), true),
                (Fp80::from_bits(0x3FFD, 0xB3F0_9271_307E_0A67), false),
                (Fp80::from_bits(0x3FFD, 0xB3F0_9271_307E_0A68), true),
                (Fp80::from_bits(0x3FFD, 0xB3F0_9271_307E_0A67), false),
            ],
        },
        RoundingCase {
            function: Transcendental::Arctangent,
            first: Fp80::from_bits(0xBFFE, 0xE666_6666_6666_6800),
            second: Fp80::from_bits(0x3FFD, 0x9999_9999_9999_9800),
            results: [
                (Fp80::from_bits(0xBFFF, 0x9FE0_BB5B_D42B_0096), false),
                (Fp80::from_bits(0xBFFF, 0x9FE0_BB5B_D42B_0097), true),
                (Fp80::from_bits(0xBFFF, 0x9FE0_BB5B_D42B_0096), false),
                (Fp80::from_bits(0xBFFF, 0x9FE0_BB5B_D42B_0096), false),
            ],
        },
        RoundingCase {
            function: Transcendental::Arctangent,
            first: Fp80::from_bits(0x4005, 0xC800_0000_0000_0000),
            second: Fp80::from_bits(0x3FEE, 0xA7C5_AC47_1B47_8800),
            results: [
                (Fp80::from_bits(0x3FFF, 0xC90F_D9CB_61D3_EC4F), false),
                (Fp80::from_bits(0x3FFF, 0xC90F_D9CB_61D3_EC4F), false),
                (Fp80::from_bits(0x3FFF, 0xC90F_D9CB_61D3_EC50), true),
                (Fp80::from_bits(0x3FFF, 0xC90F_D9CB_61D3_EC4F), false),
            ],
        },
        RoundingCase {
            function: Transcendental::Arctangent,
            first: Fp80::from_bits(0x3FC3, 0xD3C2_1BCE_CCED_A100),
            second: Fp80::ONE,
            results: [
                (Fp80::from_bits(0x3FC3, 0xD3C2_1BCE_CCED_A100), true),
                (Fp80::from_bits(0x3FC3, 0xD3C2_1BCE_CCED_A0FF), false),
                (Fp80::from_bits(0x3FC3, 0xD3C2_1BCE_CCED_A100), true),
                (Fp80::from_bits(0x3FC3, 0xD3C2_1BCE_CCED_A0FF), false),
            ],
        },
        RoundingCase {
            function: Transcendental::Arctangent,
            first: Fp80::from_bits(0xBFC9, 0xA000_0000_0000_0000),
            second: Fp80::from_bits(0x4005, 0x8000_0000_0000_0000),
            results: [
                (Fp80::from_bits(0xBFC3, 0xA000_0000_0000_0000), true),
                (Fp80::from_bits(0xBFC3, 0xA000_0000_0000_0000), true),
                (Fp80::from_bits(0xBFC3, 0x9FFF_FFFF_FFFF_FFFF), false),
                (Fp80::from_bits(0xBFC3, 0x9FFF_FFFF_FFFF_FFFF), false),
            ],
        },
        RoundingCase {
            function: Transcendental::Arctangent,
            first: Fp80::from_bits(0x3FAF, 0x8000_0000_0000_0000),
            second: Fp80::from_bits(0x3FFB, 0xFFFF_FFFF_FFFF_FFFF),
            results: [
                (Fp80::from_bits(0x3FB2, 0x8000_0000_0000_0001), true),
                (Fp80::from_bits(0x3FB2, 0x8000_0000_0000_0000), false),
                (Fp80::from_bits(0x3FB2, 0x8000_0000_0000_0001), true),
                (Fp80::from_bits(0x3FB2, 0x8000_0000_0000_0000), false),
            ],
        },
        RoundingCase {
            function: Transcendental::Arctangent,
            first: Fp80::from_bits(0x3F37, 0x8000_0000_0000_0000),
            second: Fp80::ONE,
            results: [
                (Fp80::from_bits(0x3F37, 0x8000_0000_0000_0000), true),
                (Fp80::from_bits(0x3F36, 0xFFFF_FFFF_FFFF_FFFF), false),
                (Fp80::from_bits(0x3F37, 0x8000_0000_0000_0000), true),
                (Fp80::from_bits(0x3F36, 0xFFFF_FFFF_FFFF_FFFF), false),
            ],
        },
        RoundingCase {
            function: Transcendental::Cosine,
            first: Fp80::from_bits(0x3FE0, 0x8000_0000_0000_0000),
            second: Fp80::ZERO,
            results: [
                (Fp80::from_bits(0x3FFE, 0xFFFF_FFFF_FFFF_FFFE), false),
                (Fp80::from_bits(0x3FFE, 0xFFFF_FFFF_FFFF_FFFE), false),
                (Fp80::from_bits(0x3FFE, 0xFFFF_FFFF_FFFF_FFFF), true),
                (Fp80::from_bits(0x3FFE, 0xFFFF_FFFF_FFFF_FFFE), false),
            ],
        },
        RoundingCase {
            function: Transcendental::Sine,
            first: Fp80::from_bits(0x3FE4, 0xC000_0000_0000_0000),
            second: Fp80::ZERO,
            results: [
                (Fp80::from_bits(0x3FE4, 0xBFFF_FFFF_FFFF_FEE0), false),
                (Fp80::from_bits(0x3FE4, 0xBFFF_FFFF_FFFF_FEE0), false),
                (Fp80::from_bits(0x3FE4, 0xBFFF_FFFF_FFFF_FEE1), true),
                (Fp80::from_bits(0x3FE4, 0xBFFF_FFFF_FFFF_FEE0), false),
            ],
        },
        RoundingCase {
            function: Transcendental::Tangent,
            first: Fp80::from_bits(0x3FE4, 0xC000_0000_0000_0000),
            second: Fp80::ZERO,
            results: [
                (Fp80::from_bits(0x3FE4, 0xC000_0000_0000_0240), false),
                (Fp80::from_bits(0x3FE4, 0xC000_0000_0000_0240), false),
                (Fp80::from_bits(0x3FE4, 0xC000_0000_0000_0241), true),
                (Fp80::from_bits(0x3FE4, 0xC000_0000_0000_0240), false),
            ],
        },
    ];

    /// FYL2X operands x with an odd significand next to the sqrt(2) bound of the normalization,
    /// 1 - 2^-64 and 2 - 2^-63, with y = 1, and y x log2(x) with its C1 bit in each of the
    /// [`ROUNDING_MODES`]. The results are the exact function of the full 64-bit significand
    /// rounded at 64 bits, evaluated with 400-bit arithmetic (80486 PRM 17.5).
    const LOGARITHM_SIGNIFICAND_CASES: [RoundingCase; 6] = [
        RoundingCase {
            function: Transcendental::YLog2X,
            first: Fp80::from_bits(0x3FFE, 0xFFFF_FFFF_FFFF_FFFF),
            second: Fp80::ONE,
            results: [
                (Fp80::from_bits(0xBFBF, 0xB8AA_3B29_5C17_F0BC), false),
                (Fp80::from_bits(0xBFBF, 0xB8AA_3B29_5C17_F0BD), true),
                (Fp80::from_bits(0xBFBF, 0xB8AA_3B29_5C17_F0BC), false),
                (Fp80::from_bits(0xBFBF, 0xB8AA_3B29_5C17_F0BC), false),
            ],
        },
        RoundingCase {
            function: Transcendental::YLog2X,
            first: Fp80::from_bits(0x3FFF, 0xB504_F333_F9DE_6485),
            second: Fp80::ONE,
            results: [
                (Fp80::from_bits(0x3FFE, 0x8000_0000_0000_0001), false),
                (Fp80::from_bits(0x3FFE, 0x8000_0000_0000_0001), false),
                (Fp80::from_bits(0x3FFE, 0x8000_0000_0000_0002), true),
                (Fp80::from_bits(0x3FFE, 0x8000_0000_0000_0001), false),
            ],
        },
        RoundingCase {
            function: Transcendental::YLog2X,
            first: Fp80::from_bits(0x3FFF, 0xB504_F333_F9DE_6483),
            second: Fp80::ONE,
            results: [
                (Fp80::from_bits(0x3FFD, 0xFFFF_FFFF_FFFF_FFFA), false),
                (Fp80::from_bits(0x3FFD, 0xFFFF_FFFF_FFFF_FFFA), false),
                (Fp80::from_bits(0x3FFD, 0xFFFF_FFFF_FFFF_FFFB), true),
                (Fp80::from_bits(0x3FFD, 0xFFFF_FFFF_FFFF_FFFA), false),
            ],
        },
        RoundingCase {
            function: Transcendental::YLog2X,
            first: Fp80::from_bits(0x3FFE, 0xB504_F333_F9DE_6485),
            second: Fp80::ONE,
            results: [
                (Fp80::from_bits(0xBFFD, 0xFFFF_FFFF_FFFF_FFFD), false),
                (Fp80::from_bits(0xBFFD, 0xFFFF_FFFF_FFFF_FFFE), true),
                (Fp80::from_bits(0xBFFD, 0xFFFF_FFFF_FFFF_FFFD), false),
                (Fp80::from_bits(0xBFFD, 0xFFFF_FFFF_FFFF_FFFD), false),
            ],
        },
        RoundingCase {
            function: Transcendental::YLog2X,
            first: Fp80::from_bits(0x3FFE, 0xB504_F333_F9DE_6483),
            second: Fp80::ONE,
            results: [
                (Fp80::from_bits(0xBFFE, 0x8000_0000_0000_0003), true),
                (Fp80::from_bits(0xBFFE, 0x8000_0000_0000_0003), true),
                (Fp80::from_bits(0xBFFE, 0x8000_0000_0000_0002), false),
                (Fp80::from_bits(0xBFFE, 0x8000_0000_0000_0002), false),
            ],
        },
        RoundingCase {
            function: Transcendental::YLog2X,
            first: Fp80::from_bits(0x3FFF, 0xFFFF_FFFF_FFFF_FFFF),
            second: Fp80::ONE,
            results: [
                (Fp80::from_bits(0x3FFE, 0xFFFF_FFFF_FFFF_FFFF), true),
                (Fp80::from_bits(0x3FFE, 0xFFFF_FFFF_FFFF_FFFE), false),
                (Fp80::from_bits(0x3FFE, 0xFFFF_FFFF_FFFF_FFFF), true),
                (Fp80::from_bits(0x3FFE, 0xFFFF_FFFF_FFFF_FFFE), false),
            ],
        },
    ];

    /// The sine, the cosine and C1 of FSINCOS in each of the [`ROUNDING_MODES`].
    type SineCosineResults = [(Fp80, Fp80, bool); 4];

    /// FSINCOS operands with the sine, the cosine and C1 in each of the [`ROUNDING_MODES`]. The
    /// results are the exact functions rounded at 64 bits, evaluated with 320-bit arithmetic.
    /// Observed behaviour: C1 is set when either result rounds up in magnitude.
    const SINE_COSINE_CASES: [(Fp80, SineCosineResults); 4] = [
        (
            Fp80::from_bits(0x3FFF, 0x8CCC_CCCC_CCCC_D000),
            [
                (
                    Fp80::from_bits(0x3FFE, 0xE426_2A61_6B19_BD67),
                    Fp80::from_bits(0x3FFD, 0xE83D_C036_3B08_872B),
                    true,
                ),
                (
                    Fp80::from_bits(0x3FFE, 0xE426_2A61_6B19_BD66),
                    Fp80::from_bits(0x3FFD, 0xE83D_C036_3B08_872A),
                    false,
                ),
                (
                    Fp80::from_bits(0x3FFE, 0xE426_2A61_6B19_BD67),
                    Fp80::from_bits(0x3FFD, 0xE83D_C036_3B08_872B),
                    true,
                ),
                (
                    Fp80::from_bits(0x3FFE, 0xE426_2A61_6B19_BD66),
                    Fp80::from_bits(0x3FFD, 0xE83D_C036_3B08_872A),
                    false,
                ),
            ],
        ),
        (
            Fp80::from_bits(0x4000, 0xA000_0000_0000_0000),
            [
                (
                    Fp80::from_bits(0x3FFE, 0x9935_786E_7E55_8405),
                    Fp80::from_bits(0xBFFE, 0xCD17_BF7C_2C5B_E959),
                    true,
                ),
                (
                    Fp80::from_bits(0x3FFE, 0x9935_786E_7E55_8405),
                    Fp80::from_bits(0xBFFE, 0xCD17_BF7C_2C5B_E959),
                    true,
                ),
                (
                    Fp80::from_bits(0x3FFE, 0x9935_786E_7E55_8406),
                    Fp80::from_bits(0xBFFE, 0xCD17_BF7C_2C5B_E958),
                    true,
                ),
                (
                    Fp80::from_bits(0x3FFE, 0x9935_786E_7E55_8405),
                    Fp80::from_bits(0xBFFE, 0xCD17_BF7C_2C5B_E958),
                    false,
                ),
            ],
        ),
        (
            Fp80::from_bits(0x3FE4, 0xC000_0000_0000_0000),
            [
                (
                    Fp80::from_bits(0x3FE4, 0xBFFF_FFFF_FFFF_FEE0),
                    Fp80::from_bits(0x3FFE, 0xFFFF_FFFF_FFFF_FB80),
                    false,
                ),
                (
                    Fp80::from_bits(0x3FE4, 0xBFFF_FFFF_FFFF_FEE0),
                    Fp80::from_bits(0x3FFE, 0xFFFF_FFFF_FFFF_FB80),
                    false,
                ),
                (
                    Fp80::from_bits(0x3FE4, 0xBFFF_FFFF_FFFF_FEE1),
                    Fp80::from_bits(0x3FFE, 0xFFFF_FFFF_FFFF_FB81),
                    true,
                ),
                (
                    Fp80::from_bits(0x3FE4, 0xBFFF_FFFF_FFFF_FEE0),
                    Fp80::from_bits(0x3FFE, 0xFFFF_FFFF_FFFF_FB80),
                    false,
                ),
            ],
        ),
        (
            Fp80::from_bits(0x3FE0, 0x8000_0000_0000_0000),
            [
                (
                    Fp80::from_bits(0x3FDF, 0xFFFF_FFFF_FFFF_FFFF),
                    Fp80::from_bits(0x3FFE, 0xFFFF_FFFF_FFFF_FFFE),
                    false,
                ),
                (
                    Fp80::from_bits(0x3FDF, 0xFFFF_FFFF_FFFF_FFFF),
                    Fp80::from_bits(0x3FFE, 0xFFFF_FFFF_FFFF_FFFE),
                    false,
                ),
                (
                    Fp80::from_bits(0x3FE0, 0x8000_0000_0000_0000),
                    Fp80::from_bits(0x3FFE, 0xFFFF_FFFF_FFFF_FFFF),
                    true,
                ),
                (
                    Fp80::from_bits(0x3FDF, 0xFFFF_FFFF_FFFF_FFFF),
                    Fp80::from_bits(0x3FFE, 0xFFFF_FFFF_FFFF_FFFE),
                    false,
                ),
            ],
        ),
    ];

    /// Runs the operation of `case` in the rounding mode `rc`.
    fn run_rounding_case(case: &RoundingCase, rc: RoundingMode, ef: &mut ExceptionFlags) -> Fp80 {
        match case.function {
            Transcendental::Sine => case.first.fsin(rc, ef).0,
            Transcendental::Cosine => case.first.fcos(rc, ef).0,
            Transcendental::Tangent => case.first.fptan(rc, ef).0,
            Transcendental::TwoToXMinusOne => case.first.f2xm1(rc, ef),
            Transcendental::YLog2X => case.first.fyl2x(case.second, rc, ef),
            Transcendental::YLog2XPlusOne => case.first.fyl2xp1(case.second, rc, ef),
            Transcendental::Arctangent => case.first.fpatan(case.second, rc, ef),
        }
    }

    /// Checks the result, PE and C1 of each of `cases` in each of the [`ROUNDING_MODES`].
    fn check_rounding_cases(cases: &[RoundingCase]) {
        for case in cases {
            for (rc, (expected, rounded_up)) in ROUNDING_MODES.into_iter().zip(case.results) {
                let mut ef = no_exceptions();
                let result = run_rounding_case(case, rc, &mut ef);
                assert_eq!(
                    result, expected,
                    "{:?} {:?} {rc:?}",
                    case.first, case.second
                );
                assert!(ef.precision, "{:?} {:?} {rc:?}", case.first, case.second);
                assert_eq!(
                    ef.rounded_up, rounded_up,
                    "C1 {:?} {:?} {rc:?}",
                    case.first, case.second
                );
            }
        }
    }

    #[test]
    fn transcendental_results_follow_the_rounding_control() {
        check_rounding_cases(&ROUNDING_CASES);
    }

    #[test]
    fn logarithm_keeps_every_significand_bit_of_the_operand() {
        check_rounding_cases(&LOGARITHM_SIGNIFICAND_CASES);
    }

    #[test]
    fn sine_and_cosine_follow_the_rounding_control() {
        for (operand, results) in SINE_COSINE_CASES {
            for (rc, (expected_sine, expected_cosine, rounded_up)) in
                ROUNDING_MODES.into_iter().zip(results)
            {
                let mut ef = no_exceptions();
                let (sine, cosine, out_of_range) = operand.fsincos(rc, &mut ef);
                assert!(!out_of_range);
                assert_eq!(
                    (sine, cosine),
                    (expected_sine, expected_cosine),
                    "{operand:?} {rc:?}"
                );
                assert!(ef.precision, "{operand:?} {rc:?}");
                assert_eq!(ef.rounded_up, rounded_up, "C1 {operand:?} {rc:?}");
            }
        }
    }

    #[test]
    fn first_term_results_follow_the_rounding_control() {
        let operand = Fp80::from_bits(0x3FDE, 0x8000_0000_0000_0000);
        let below = Fp80::from_bits(0x3FDD, 0xFFFF_FFFF_FFFF_FFFF);
        let below_one = Fp80::from_bits(0x3FFE, 0xFFFF_FFFF_FFFF_FFFF);
        let above = Fp80::from_bits(0x3FDE, 0x8000_0000_0000_0001);
        let expectations = [
            (RoundingMode::NearestEven, operand, Fp80::ONE, operand),
            (RoundingMode::Down, below, below_one, operand),
            (RoundingMode::Up, operand, Fp80::ONE, above),
            (RoundingMode::Zero, below, below_one, operand),
        ];
        for (rc, sine, cosine, tangent) in expectations {
            let mut ef = no_exceptions();
            assert_eq!(operand.fsin(rc, &mut ef), (sine, false), "{rc:?}");
            assert!(ef.precision);
            assert_eq!(ef.rounded_up, sine == operand, "{rc:?}");

            let mut ef = no_exceptions();
            assert_eq!(operand.fcos(rc, &mut ef), (cosine, false), "{rc:?}");
            assert_eq!(ef.rounded_up, cosine == Fp80::ONE, "{rc:?}");

            let mut ef = no_exceptions();
            assert_eq!(operand.fptan(rc, &mut ef), (tangent, false), "{rc:?}");
            assert_eq!(ef.rounded_up, tangent == above, "{rc:?}");

            let mut ef = no_exceptions();
            assert_eq!(
                operand.fsincos(rc, &mut ef),
                (sine, cosine, false),
                "{rc:?}"
            );
        }
    }
}
