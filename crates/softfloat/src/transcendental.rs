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
    flag_denormal_operands,
};

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

        let x = DoubleF64::from_fp80(self);
        let y = x * DD_LN2;

        let mut result = eval_poly(y, &EXP_ARR);
        result = result * y;

        result.to_fp80(ef)
    }

    /// Compute y × log₂(self) (FYL2X). self = x (ST0), y = ST1.
    pub fn fyl2x(self, y: Fp80, ef: &mut ExceptionFlags) -> Fp80 {
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
        let y_dd = DoubleF64::from_fp80(y);
        let result = y_dd * log2_x;
        result.to_fp80(ef)
    }

    /// Compute y × log₂(self + 1) (FYL2XP1). self = x (ST0), y = ST1. An operand more
    /// negative than -1 is an invalid operation (80486 PRM table 16-11).
    pub fn fyl2xp1(self, y: Fp80, ef: &mut ExceptionFlags) -> Fp80 {
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

        let x_dd = DoubleF64::from_fp80(self);
        let two = DoubleF64::ONE + DoubleF64::ONE;
        let u = x_dd / (x_dd + two);
        let log2_1px = odd_poly(u, &LN_ARR) * DD_LN2INV2;

        let y_dd = DoubleF64::from_fp80(y);
        let result = y_dd * log2_1px;
        result.to_fp80(ef)
    }

    /// Compute sin(self) (FSIN). Returns (result, out_of_range).
    pub fn fsin(self, ef: &mut ExceptionFlags) -> (Fp80, bool) {
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

        let y_dd = DoubleF64::from_fp80(y.abs());
        let x_dd = DoubleF64::from_fp80(x.abs());

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
}
