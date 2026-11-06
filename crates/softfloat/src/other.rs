//! Other FPU operations: scale, extract, partial remainder, IEEE remainder.

use crate::{
    BIAS, ExceptionFlags, Fp80, Precision, RoundingMode, flag_denormal_operands,
    flag_unsupported_operands,
};

/// The magnitude an FSCALE scale factor is clamped to. Every factor of this magnitude or
/// more gives the same infinity or zero, also after the bias of the unmasked responses.
const SCALE_LIMIT: i64 = 0x10_0000;

impl Fp80 {
    /// Compute self * 2^trunc(scale) (FSCALE). The result takes the rounding mode `rc` at
    /// the extended precision for an overflow (80486 PRM table 16-12) and a denormal result.
    pub fn scale(self, scale: Fp80, rc: RoundingMode, ef: &mut ExceptionFlags) -> Fp80 {
        if flag_unsupported_operands(&[self, scale], ef) {
            return Fp80::INDEFINITE;
        }
        if self.is_nan() || scale.is_nan() {
            return Fp80::propagate_nan(self, scale, ef);
        }

        flag_denormal_operands(&[self, scale], ef);

        if self.is_infinity() {
            if scale.is_infinity() && scale.sign() {
                ef.invalid = true;
                return Fp80::INDEFINITE;
            }
            return self;
        }

        if self.is_zero() {
            if scale.is_infinity() && !scale.sign() {
                ef.invalid = true;
                return Fp80::INDEFINITE;
            }
            return self;
        }

        if scale.is_infinity() {
            if scale.sign() {
                return if self.sign() {
                    Fp80::NEG_ZERO
                } else {
                    Fp80::ZERO
                };
            }
            return if self.sign() {
                Fp80::NEG_INFINITY
            } else {
                Fp80::INFINITY
            };
        }

        let scale_int = truncate_to_integer(scale).clamp(-SCALE_LIMIT, SCALE_LIMIT);
        let exponent = self.true_exponent() as i64 + scale_int;
        Fp80::round_and_pack(
            self.sign(),
            exponent as i32,
            (self.significand() as u128) << 64,
            rc,
            Precision::Extended,
            ef,
        )
    }

    /// Separate exponent and significand (FXTRACT).
    /// Returns (significand with exponent 0x3FFF, exponent as Fp80).
    /// After execution: ST(0) = significand (first element), ST(1) = exponent (second element).
    pub fn extract(self, ef: &mut ExceptionFlags) -> (Fp80, Fp80) {
        if flag_unsupported_operands(&[self], ef) {
            return (Fp80::INDEFINITE, Fp80::INDEFINITE);
        }
        if self.is_nan() {
            if self.is_signaling_nan() {
                ef.invalid = true;
            }
            let nan = if self.is_signaling_nan() {
                self.quieten()
            } else {
                self
            };
            return (nan, nan);
        }

        if self.is_infinity() {
            return (self, Fp80::INFINITY);
        }

        if self.is_zero() {
            ef.zero_divide = true;
            return (self, Fp80::NEG_INFINITY);
        }

        flag_denormal_operands(&[self], ef);

        let sign = self.sign();
        let mut exponent = self.true_exponent();
        let mut significand = self.significand();

        if self.exponent() == 0 && significand != 0 {
            let (norm_sig, shift) = Fp80::normalize_significand(significand);
            significand = norm_sig;
            exponent -= shift as i32;
        }

        let sig_se = if sign { 0xBFFF_u16 } else { 0x3FFF_u16 };
        let sig_result = Fp80::from_bits(sig_se, significand);

        let exp_result = Fp80::from_i64(exponent as i64);

        (sig_result, exp_result)
    }

    /// Partial remainder with truncation (FPREM).
    /// Returns (remainder, quotient_low_3_bits, complete).
    /// `complete = false` means C2=1 (re-execute needed).
    /// `quotient_low_3_bits` encodes Q2:Q1:Q0.
    pub fn partial_remainder(self, divisor: Fp80, ef: &mut ExceptionFlags) -> (Fp80, u8, bool) {
        remainder_impl(self, divisor, false, ef)
    }

    /// IEEE partial remainder with round-to-nearest (FPREM1).
    /// Returns (remainder, quotient_low_3_bits, complete).
    /// Same semantics as `partial_remainder`.
    pub fn ieee_remainder(self, divisor: Fp80, ef: &mut ExceptionFlags) -> (Fp80, u8, bool) {
        remainder_impl(self, divisor, true, ef)
    }
}

/// Returns `value` truncated toward zero, saturated at a magnitude of 2^62.
fn truncate_to_integer(value: Fp80) -> i64 {
    if value.is_zero() {
        return 0;
    }

    let sign = value.sign();
    let exp = value.true_exponent();
    let sig = value.significand();

    if exp < 0 {
        return 0;
    }

    if exp >= 62 {
        return if sign { -(1 << 62) } else { 1 << 62 };
    }

    let shift = 63 - exp as u32;
    let integer = sig >> shift;

    if sign {
        -(integer as i64)
    } else {
        integer as i64
    }
}

fn remainder_impl(
    dividend: Fp80,
    divisor: Fp80,
    round_nearest: bool,
    ef: &mut ExceptionFlags,
) -> (Fp80, u8, bool) {
    if flag_unsupported_operands(&[dividend, divisor], ef) {
        return (Fp80::INDEFINITE, 0, true);
    }

    if dividend.is_nan() || divisor.is_nan() {
        let nan = Fp80::propagate_nan(dividend, divisor, ef);
        return (nan, 0, true);
    }

    if dividend.is_infinity() {
        ef.invalid = true;
        return (Fp80::INDEFINITE, 0, true);
    }

    if divisor.is_zero() {
        ef.invalid = true;
        return (Fp80::INDEFINITE, 0, true);
    }

    flag_denormal_operands(&[dividend, divisor], ef);

    if dividend.is_zero() {
        return (dividend, 0, true);
    }

    if divisor.is_infinity() {
        return (dividend.canonical(), 0, true);
    }

    let dividend_sign = dividend.sign();

    let mut dividend_exp = dividend.true_exponent();
    let mut dividend_sig: u64 = dividend.significand();
    if dividend.exponent() == 0 && dividend_sig != 0 {
        let (norm, shift) = Fp80::normalize_significand(dividend_sig);
        dividend_sig = norm;
        dividend_exp -= shift as i32;
    }

    let mut divisor_exp = divisor.true_exponent();
    let mut divisor_sig: u64 = divisor.significand();
    if divisor.exponent() == 0 && divisor_sig != 0 {
        let (norm, shift) = Fp80::normalize_significand(divisor_sig);
        divisor_sig = norm;
        divisor_exp -= shift as i32;
    }

    let exp_diff = dividend_exp - divisor_exp;

    if exp_diff < 0 {
        if round_nearest && exp_diff == -1 {
            let doubled_dividend = (dividend_sig as u128) << 1;
            let divisor_128 = divisor_sig as u128;
            if doubled_dividend > divisor_128 {
                let result_sign = !dividend_sign;
                let result = pack_remainder_u64(
                    result_sign,
                    divisor_exp,
                    (divisor_128 - doubled_dividend) as u64,
                );
                return (result, 1, true);
            }
        }
        return (dividend.canonical(), 0, true);
    }

    if exp_diff >= 64 {
        let mut current_sig = dividend_sig as u128;
        let divisor_128 = divisor_sig as u128;
        let current_exp = dividend_exp;

        let reduce_bits = 63i32.min(exp_diff);

        for i in (0..reduce_bits).rev() {
            let target_exp = divisor_exp + i;
            let shift = current_exp - target_exp;
            if !(0..128).contains(&shift) {
                continue;
            }
            let shifted_divisor = divisor_128 << (shift as u32);
            if current_sig >= shifted_divisor {
                current_sig -= shifted_divisor;
            }
        }

        if current_sig == 0 {
            let result = if dividend_sign {
                Fp80::NEG_ZERO
            } else {
                Fp80::ZERO
            };
            return (result, 0, true);
        }

        let sig64 = current_sig as u64;
        let leading = sig64.leading_zeros();
        let norm_sig = sig64 << leading;
        let new_exp = current_exp - leading as i32;

        let result = pack_remainder_u64(dividend_sign, new_exp, norm_sig);
        return (result, 0, false);
    }

    // exp_diff in [0, 63]: we can compute the full quotient.
    // Shift dividend significand left by exp_diff to align with divisor scale,
    // then do integer division.
    let mut remainder = (dividend_sig as u128) << (exp_diff as u32);
    let divisor_128 = divisor_sig as u128;
    let mut quotient: u64 = (remainder / divisor_128) as u64;
    remainder %= divisor_128;

    let mut remainder_u64 = remainder as u64;

    if round_nearest && remainder_u64 != 0 {
        let doubled_remainder = (remainder_u64 as u128) << 1;
        if doubled_remainder > divisor_128
            || (doubled_remainder == divisor_128 && quotient & 1 != 0)
        {
            remainder_u64 = divisor_sig - remainder_u64;
            quotient = quotient.wrapping_add(1);
            let result = pack_remainder_u64(!dividend_sign, divisor_exp, remainder_u64);
            return (result, (quotient & 7) as u8, true);
        }
    }

    if remainder_u64 == 0 {
        let result = if dividend_sign {
            Fp80::NEG_ZERO
        } else {
            Fp80::ZERO
        };
        return (result, (quotient & 7) as u8, true);
    }

    let result = pack_remainder_u64(dividend_sign, divisor_exp, remainder_u64);
    (result, (quotient & 7) as u8, true)
}

fn pack_remainder_u64(sign: bool, exponent: i32, significand: u64) -> Fp80 {
    if significand == 0 {
        return if sign { Fp80::NEG_ZERO } else { Fp80::ZERO };
    }

    let leading = significand.leading_zeros();
    let norm_sig = significand << leading;
    let adj_exp = exponent - leading as i32;

    let biased = adj_exp + BIAS;
    if biased <= 0 {
        let shift_right = (1 - biased) as u32;
        if shift_right >= 64 {
            return if sign { Fp80::NEG_ZERO } else { Fp80::ZERO };
        }
        let denorm_sig = norm_sig >> shift_right;
        let se = if sign { 0x8000u16 } else { 0x0000u16 };
        return Fp80::from_bits(se, denorm_sig);
    }

    let se = if sign {
        0x8000 | biased as u16
    } else {
        biased as u16
    };
    Fp80::from_bits(se, norm_sig)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEGATIVE_ONE: Fp80 = Fp80::from_bits(0xBFFF, 0x8000_0000_0000_0000);
    const TWO: Fp80 = Fp80::from_bits(0x4000, 0x8000_0000_0000_0000);
    const THREE: Fp80 = Fp80::from_bits(0x4000, 0xC000_0000_0000_0000);
    const FOUR: Fp80 = Fp80::from_bits(0x4001, 0x8000_0000_0000_0000);
    const FIVE: Fp80 = Fp80::from_bits(0x4001, 0xA000_0000_0000_0000);
    const SEVEN: Fp80 = Fp80::from_bits(0x4001, 0xE000_0000_0000_0000);
    const POSITIVE_SNAN: Fp80 = Fp80::from_bits(0x7FFF, 0x8000_0000_0000_0001);
    const POSITIVE_QNAN: Fp80 = Fp80::from_bits(0x7FFF, 0xC000_0000_0000_0001);
    const NEGATIVE_FOUR: Fp80 = Fp80::from_bits(0xC001, 0x8000_0000_0000_0000);
    const NEGATIVE_THREE: Fp80 = Fp80::from_bits(0xC000, 0xC000_0000_0000_0000);
    const NEGATIVE_SEVEN: Fp80 = Fp80::from_bits(0xC001, 0xE000_0000_0000_0000);
    const HALF: Fp80 = Fp80::from_bits(0x3FFE, 0x8000_0000_0000_0000);
    const ONE_AND_HALF: Fp80 = Fp80::from_bits(0x3FFF, 0xC000_0000_0000_0000);
    // 2^100: exponent = 0x3FFF + 100 = 0x4063.
    const TWO_POW_100: Fp80 = Fp80::from_bits(0x4063, 0x8000_0000_0000_0000);

    fn no_exceptions() -> ExceptionFlags {
        ExceptionFlags::default()
    }

    #[test]
    fn test_scale_basic() {
        // 1.0 * 2^1 = 2.0.
        let mut ef = no_exceptions();
        let result = Fp80::ONE.scale(Fp80::ONE, RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, TWO);
        assert_eq!(ef, no_exceptions());

        // 1.0 * 2^0 = 1.0.
        let mut ef = no_exceptions();
        let result = Fp80::ONE.scale(Fp80::ZERO, RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::ONE);

        // 1.0 * 2^2 = 4.0.
        let mut ef = no_exceptions();
        let result = Fp80::ONE.scale(TWO, RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, FOUR);
    }

    #[test]
    fn test_scale_edge_cases() {
        // +0 * 2^(+inf) = Indefinite + IE.
        let mut ef = no_exceptions();
        let result = Fp80::ZERO.scale(Fp80::INFINITY, RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::INDEFINITE);
        assert!(ef.invalid);

        // +inf * 2^(-inf) = Indefinite + IE.
        let mut ef = no_exceptions();
        let result = Fp80::INFINITY.scale(Fp80::NEG_INFINITY, RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::INDEFINITE);
        assert!(ef.invalid);

        // +0 * 2^(-inf) = +0.
        let mut ef = no_exceptions();
        let result = Fp80::ZERO.scale(Fp80::NEG_INFINITY, RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::ZERO);

        // +inf * 2^(+inf) = +inf.
        let mut ef = no_exceptions();
        let result = Fp80::INFINITY.scale(Fp80::INFINITY, RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::INFINITY);

        // SNaN propagation.
        let mut ef = no_exceptions();
        let result = POSITIVE_SNAN.scale(Fp80::ONE, RoundingMode::NearestEven, &mut ef);
        assert!(result.is_quiet_nan());
        assert!(ef.invalid);

        // QNaN propagation, no IE.
        let mut ef = no_exceptions();
        let result = POSITIVE_QNAN.scale(Fp80::ONE, RoundingMode::NearestEven, &mut ef);
        assert!(result.is_quiet_nan());
        assert!(!ef.invalid);

        // 0 * 2^(finite) = 0 with sign preserved.
        let mut ef = no_exceptions();
        let result = Fp80::NEG_ZERO.scale(TWO, RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::NEG_ZERO);
    }

    #[test]
    fn test_extract_basic() {
        // extract(1.0) = (significand=1.0, exponent=0.0).
        // 1.0 has true exponent 0 (biased 0x3FFF), significand = 1.0.
        let mut ef = no_exceptions();
        let (sig, exp) = Fp80::ONE.extract(&mut ef);
        assert_eq!(sig, Fp80::ONE); // significand with exp=0x3FFF, value in [1,2)
        assert_eq!(exp, Fp80::ZERO); // true exponent = 0
        assert_eq!(ef, no_exceptions());

        // extract(4.0): 4.0 = 1.0 * 2^2. Significand = 1.0, exponent = 2.0.
        let mut ef = no_exceptions();
        let (sig, exp) = FOUR.extract(&mut ef);
        assert_eq!(sig, Fp80::ONE);
        assert_eq!(exp, TWO);
    }

    #[test]
    fn test_extract_edge_cases() {
        // extract(+0) = (+0, -inf) + ZE.
        let mut ef = no_exceptions();
        let (sig, exp) = Fp80::ZERO.extract(&mut ef);
        assert_eq!(sig, Fp80::ZERO);
        assert_eq!(exp, Fp80::NEG_INFINITY);
        assert!(ef.zero_divide);

        // extract(-0) = (-0, -inf) + ZE.
        let mut ef = no_exceptions();
        let (sig, exp) = Fp80::NEG_ZERO.extract(&mut ef);
        assert_eq!(sig, Fp80::NEG_ZERO);
        assert_eq!(exp, Fp80::NEG_INFINITY);
        assert!(ef.zero_divide);

        // extract(+inf) = (+inf, +inf).
        let mut ef = no_exceptions();
        let (sig, exp) = Fp80::INFINITY.extract(&mut ef);
        assert_eq!(sig, Fp80::INFINITY);
        assert_eq!(exp, Fp80::INFINITY);

        // extract(-inf) = (-inf, +inf).
        let mut ef = no_exceptions();
        let (sig, exp) = Fp80::NEG_INFINITY.extract(&mut ef);
        assert_eq!(sig, Fp80::NEG_INFINITY);
        assert_eq!(exp, Fp80::INFINITY);

        // extract(SNaN) -> NaN, IE.
        let mut ef = no_exceptions();
        let (sig, exp) = POSITIVE_SNAN.extract(&mut ef);
        assert!(sig.is_nan());
        assert!(exp.is_nan());
        assert!(ef.invalid);
    }

    #[test]
    fn test_partial_remainder_basic() {
        // 5 % 3 = 2 (quotient = 1).
        let mut ef = no_exceptions();
        let (rem, q_bits, complete) = FIVE.partial_remainder(THREE, &mut ef);
        assert_eq!(rem, TWO);
        assert_eq!(q_bits & 0x07, 1); // low 3 bits of quotient
        assert!(complete);

        // 7 % 4 = 3 (quotient = 1).
        let mut ef = no_exceptions();
        let (rem, q_bits, complete) = SEVEN.partial_remainder(FOUR, &mut ef);
        assert_eq!(rem, THREE);
        assert_eq!(q_bits & 0x07, 1);
        assert!(complete);
    }

    #[test]
    fn test_partial_remainder_edge_cases() {
        // inf % any = Indefinite + IE.
        let mut ef = no_exceptions();
        let (rem, _, _) = Fp80::INFINITY.partial_remainder(Fp80::ONE, &mut ef);
        assert_eq!(rem, Fp80::INDEFINITE);
        assert!(ef.invalid);

        // any % 0 = Indefinite + IE.
        let mut ef = no_exceptions();
        let (rem, _, _) = Fp80::ONE.partial_remainder(Fp80::ZERO, &mut ef);
        assert_eq!(rem, Fp80::INDEFINITE);
        assert!(ef.invalid);

        // 0 % nonzero = +0.
        let mut ef = no_exceptions();
        let (rem, _, complete) = Fp80::ZERO.partial_remainder(Fp80::ONE, &mut ef);
        assert!(rem.is_zero());
        assert!(complete);

        // finite % inf = finite (unchanged).
        let mut ef = no_exceptions();
        let (rem, _, complete) = Fp80::ONE.partial_remainder(Fp80::INFINITY, &mut ef);
        assert_eq!(rem, Fp80::ONE);
        assert!(complete);

        // SNaN propagation.
        let mut ef = no_exceptions();
        let (rem, _, _) = POSITIVE_SNAN.partial_remainder(Fp80::ONE, &mut ef);
        assert!(rem.is_nan());
        assert!(ef.invalid);
    }

    #[test]
    fn test_ieee_remainder_basic() {
        // 5 FPREM1 3: round_nearest(5/3) = round_nearest(1.666...) = 2. rem = 5 - 2*3 = -1.
        let mut ef = no_exceptions();
        let (rem, q_bits, complete) = FIVE.ieee_remainder(THREE, &mut ef);
        assert_eq!(rem, NEGATIVE_ONE);
        assert_eq!(q_bits & 0x07, 2); // quotient = 2
        assert!(complete);

        // 7 FPREM1 4: round_nearest(7/4) = round_nearest(1.75) = 2. rem = 7 - 2*4 = -1.
        let mut ef = no_exceptions();
        let (rem, q_bits, complete) = SEVEN.ieee_remainder(FOUR, &mut ef);
        assert_eq!(rem, NEGATIVE_ONE);
        assert_eq!(q_bits & 0x07, 2);
        assert!(complete);
    }

    #[test]
    fn test_ieee_remainder_edge_cases() {
        // inf % any = Indefinite + IE.
        let mut ef = no_exceptions();
        let (rem, _, _) = Fp80::INFINITY.ieee_remainder(Fp80::ONE, &mut ef);
        assert_eq!(rem, Fp80::INDEFINITE);
        assert!(ef.invalid);

        // any % 0 = Indefinite + IE.
        let mut ef = no_exceptions();
        let (rem, _, _) = Fp80::ONE.ieee_remainder(Fp80::ZERO, &mut ef);
        assert_eq!(rem, Fp80::INDEFINITE);
        assert!(ef.invalid);

        // 0 % nonzero = +0.
        let mut ef = no_exceptions();
        let (rem, _, complete) = Fp80::ZERO.ieee_remainder(Fp80::ONE, &mut ef);
        assert!(rem.is_zero());
        assert!(complete);

        // finite % inf = finite (unchanged).
        let mut ef = no_exceptions();
        let (rem, _, complete) = Fp80::ONE.ieee_remainder(Fp80::INFINITY, &mut ef);
        assert_eq!(rem, Fp80::ONE);
        assert!(complete);
    }

    #[test]
    fn test_scale_nan_in_scale_param() {
        // SNaN in scale parameter -> QNaN + IE.
        let mut ef = no_exceptions();
        let result = Fp80::ONE.scale(POSITIVE_SNAN, RoundingMode::NearestEven, &mut ef);
        assert!(result.is_quiet_nan());
        assert!(ef.invalid);

        // QNaN in scale parameter -> QNaN, no IE.
        let mut ef = no_exceptions();
        let result = Fp80::ONE.scale(POSITIVE_QNAN, RoundingMode::NearestEven, &mut ef);
        assert!(result.is_quiet_nan());
        assert!(!ef.invalid);
    }

    #[test]
    fn test_scale_non_integer_truncation() {
        // scale(1.0, 1.5) -> 1.0 * 2^floor(1.5) = 1.0 * 2^1 = 2.0.
        let mut ef = no_exceptions();
        let result = Fp80::ONE.scale(ONE_AND_HALF, RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, TWO);
    }

    #[test]
    fn test_scale_negative() {
        // scale(1.0, -1) = 1.0 * 2^(-1) = 0.5.
        let mut ef = no_exceptions();
        let result = Fp80::ONE.scale(NEGATIVE_ONE, RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, HALF);
    }

    #[test]
    fn test_scale_neg_inf_cases() {
        // -inf * 2^(-inf) = Indefinite + IE.
        let mut ef = no_exceptions();
        let result =
            Fp80::NEG_INFINITY.scale(Fp80::NEG_INFINITY, RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::INDEFINITE);
        assert!(ef.invalid);

        // -inf * 2^(+inf) = -inf.
        let mut ef = no_exceptions();
        let result = Fp80::NEG_INFINITY.scale(Fp80::INFINITY, RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::NEG_INFINITY);
    }

    /// Verifies that an FSCALE overflow takes the masked result of the rounding mode
    /// (80486 PRM table 16-12) and records the biased unmasked response.
    #[test]
    fn test_scale_overflow_follows_rounding_mode() {
        let large_scale = Fp80::from_i32(100_000);
        let mut ef = no_exceptions();
        let result = Fp80::ONE.scale(large_scale, RoundingMode::Zero, &mut ef);
        assert_eq!(result, Fp80::from_bits(0x7FFE, u64::MAX));
        assert!(ef.overflow && ef.precision && !ef.rounded_up);

        let mut ef = no_exceptions();
        let result = Fp80::ONE.scale(large_scale, RoundingMode::Down, &mut ef);
        assert_eq!(result, Fp80::from_bits(0x7FFE, u64::MAX));

        let mut ef = no_exceptions();
        let result = NEGATIVE_ONE.scale(large_scale, RoundingMode::Down, &mut ef);
        assert_eq!(result, Fp80::NEG_INFINITY);
        assert!(ef.rounded_up);
        let biased = ef.biased.expect("biased response");
        assert_eq!(biased.value, Fp80::NEG_INFINITY);

        let mut ef = no_exceptions();
        let huge = Fp80::from_bits(0x7FFE, 0xC000_0000_0000_0000);
        let result = huge.scale(Fp80::from_i32(20_000), RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::INFINITY);
        let biased = ef.biased.expect("biased response");
        assert_eq!(biased.value, Fp80::from_bits(0x6E1E, 0xC000_0000_0000_0000));
        assert!(!biased.precision && !biased.rounded_up);

        let mut ef = no_exceptions();
        let scale = Fp80::from_bits(0x4063, 0x8000_0000_0000_0000);
        let result = Fp80::ONE.scale(scale, RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::INFINITY);
    }

    /// Verifies that a denormal FSCALE result rounds with the rounding mode.
    #[test]
    fn test_scale_denormal_result_rounds() {
        let value = Fp80::from_bits(0x3FFF, 0xC000_0000_0000_0001);
        let scale = Fp80::from_i32(-16400);
        let mut ef = no_exceptions();
        let result = value.scale(scale, RoundingMode::Up, &mut ef);
        assert_eq!(result, Fp80::from_bits(0x0000, 0x0000_3000_0000_0001));
        assert!(ef.underflow && ef.precision && ef.rounded_up && ef.tiny);

        let mut ef = no_exceptions();
        let result = value.scale(scale, RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::from_bits(0x0000, 0x0000_3000_0000_0000));
        assert!(!ef.rounded_up);
        let biased = ef.biased.expect("biased response");
        assert_eq!(biased.value, Fp80::from_bits(0x5FEF, 0xC000_0000_0000_0001));
        assert!(!biased.precision);

        let mut ef = no_exceptions();
        let result = value.scale(Fp80::from_i32(-16500), RoundingMode::Up, &mut ef);
        assert_eq!(result, Fp80::from_bits(0x0000, 1));
    }

    /// Verifies that FSCALE reports a denormal operand.
    #[test]
    fn test_scale_denormal_operand() {
        let mut ef = no_exceptions();
        let denormal = Fp80::from_bits(0x0000, 0x1234);
        let result = Fp80::ONE.scale(denormal, RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, Fp80::ONE);
        assert!(ef.denormal);
    }

    #[test]
    fn test_extract_qnan() {
        // extract(QNaN) = (QNaN, QNaN), no IE.
        let mut ef = no_exceptions();
        let (sig, exp) = POSITIVE_QNAN.extract(&mut ef);
        assert!(sig.is_quiet_nan());
        assert!(exp.is_quiet_nan());
        assert!(!ef.invalid);
    }

    #[test]
    fn test_extract_negative_normal() {
        // extract(-4.0): -4.0 = -1.0 * 2^2. Significand = -1.0, exponent = 2.0.
        let mut ef = no_exceptions();
        let (sig, exp) = NEGATIVE_FOUR.extract(&mut ef);
        assert!(sig.sign());
        assert_eq!(sig.exponent(), 0x3FFF);
        assert_eq!(exp, TWO);
    }

    #[test]
    fn test_partial_remainder_negative_operand() {
        // (-7) % 4 = -3 (quotient = 1, sign from dividend).
        let mut ef = no_exceptions();
        let (rem, q_bits, complete) = NEGATIVE_SEVEN.partial_remainder(FOUR, &mut ef);
        assert_eq!(rem, NEGATIVE_THREE);
        assert_eq!(q_bits & 0x07, 1);
        assert!(complete);
    }

    #[test]
    fn test_partial_remainder_incomplete() {
        // Large exponent difference (>= 64): should return complete=false (C2=1).
        let mut ef = no_exceptions();
        let (_, _, complete) = TWO_POW_100.partial_remainder(Fp80::ONE, &mut ef);
        assert!(!complete);
    }

    #[test]
    fn test_partial_remainder_quotient_bits() {
        // 7 % 2 = 1 (quotient = 3). q_bits low 3 = 3 (binary 011).
        let mut ef = no_exceptions();
        let (rem, q_bits, complete) = SEVEN.partial_remainder(TWO, &mut ef);
        assert_eq!(rem, Fp80::ONE);
        assert_eq!(q_bits & 0x07, 3);
        assert!(complete);
    }

    #[test]
    fn test_partial_remainder_qnan() {
        // QNaN % anything = QNaN, no IE.
        let mut ef = no_exceptions();
        let (rem, _, _) = POSITIVE_QNAN.partial_remainder(Fp80::ONE, &mut ef);
        assert!(rem.is_quiet_nan());
        assert!(!ef.invalid);
    }

    #[test]
    fn test_ieee_remainder_snan() {
        // SNaN propagation.
        let mut ef = no_exceptions();
        let (rem, _, _) = POSITIVE_SNAN.ieee_remainder(Fp80::ONE, &mut ef);
        assert!(rem.is_nan());
        assert!(ef.invalid);
    }

    #[test]
    fn test_ieee_remainder_halfway() {
        // 3 % 2: round_nearest(3/2) = round_nearest(1.5) = 2 (round to even).
        // rem = 3 - 2*2 = -1.
        let mut ef = no_exceptions();
        let (rem, q_bits, complete) = THREE.ieee_remainder(TWO, &mut ef);
        assert_eq!(rem, NEGATIVE_ONE);
        assert_eq!(q_bits & 0x07, 2);
        assert!(complete);
    }

    #[test]
    fn test_ieee_remainder_qnan() {
        // QNaN % anything = QNaN, no IE.
        let mut ef = no_exceptions();
        let (rem, _, _) = POSITIVE_QNAN.ieee_remainder(Fp80::ONE, &mut ef);
        assert!(rem.is_quiet_nan());
        assert!(!ef.invalid);
    }
}
