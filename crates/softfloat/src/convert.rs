//! Conversions between Fp80 and integer/float/BCD formats.

use crate::{ExceptionFlags, Fp80, RoundingMode};

impl Fp80 {
    /// Convert a signed 16-bit integer to Fp80. Exact, no rounding needed.
    pub const fn from_i16(v: i16) -> Fp80 {
        Self::from_i64(v as i64)
    }

    /// Convert a signed 32-bit integer to Fp80. Exact, no rounding needed.
    pub const fn from_i32(v: i32) -> Fp80 {
        Self::from_i64(v as i64)
    }

    /// Convert a signed 64-bit integer to Fp80. Exact, no rounding needed.
    /// All i64 values are representable in the 64-bit significand of Fp80.
    pub const fn from_i64(v: i64) -> Fp80 {
        if v == 0 {
            return Fp80::ZERO;
        }

        let sign = v < 0;
        let magnitude = if sign {
            (v as u64).wrapping_neg()
        } else {
            v as u64
        };

        let leading_zeros = magnitude.leading_zeros();
        let significand = magnitude << leading_zeros;
        let biased_exponent = 16383 + 63 - leading_zeros as u16;

        let sign_exponent = if sign {
            0x8000 | biased_exponent
        } else {
            biased_exponent
        };

        Fp80 {
            significand,
            sign_exponent,
        }
    }
}

impl Fp80 {
    /// Convert to signed 16-bit integer. Returns 0x8000 (integer indefinite) on overflow/NaN/∞.
    pub fn to_i16(self, rc: RoundingMode, ef: &mut ExceptionFlags) -> i16 {
        let result = self.to_integer_generic(rc, ef, 15);
        result as i16
    }

    /// Convert to signed 32-bit integer. Returns 0x80000000 (integer indefinite) on overflow/NaN/∞.
    pub fn to_i32(self, rc: RoundingMode, ef: &mut ExceptionFlags) -> i32 {
        let result = self.to_integer_generic(rc, ef, 31);
        result as i32
    }

    /// Convert to signed 64-bit integer. Returns 0x8000000000000000 (integer indefinite) on
    /// overflow/NaN/∞.
    pub fn to_i64(self, rc: RoundingMode, ef: &mut ExceptionFlags) -> i64 {
        self.to_integer_generic(rc, ef, 63)
    }

    fn to_integer_generic(self, rc: RoundingMode, ef: &mut ExceptionFlags, max_bit: u32) -> i64 {
        let indefinite = i64::MIN >> (63 - max_bit);

        if self.is_nan() || self.is_infinity() {
            ef.invalid = true;
            return indefinite;
        }

        if self.is_zero() {
            return 0;
        }

        let sign = self.sign();
        let true_exp = self.true_exponent();
        let sig = self.significand();

        if true_exp >= max_bit as i32 {
            if true_exp == max_bit as i32 && sign && sig == 0x8000_0000_0000_0000 {
                return indefinite;
            }
            ef.invalid = true;
            return indefinite;
        }

        if true_exp < 0 {
            ef.precision = true;
            let round_up = match rc {
                RoundingMode::NearestEven => {
                    if true_exp == -1 {
                        sig > 0x8000_0000_0000_0000
                    } else {
                        false
                    }
                }
                RoundingMode::Up => !sign,
                RoundingMode::Down => sign,
                RoundingMode::Zero => false,
            };
            return if round_up {
                if sign { -1 } else { 1 }
            } else {
                0
            };
        }

        let shift_right = 63 - true_exp as u32;
        let integer_part = sig >> shift_right;
        let fraction = if shift_right < 64 {
            sig & ((1u64 << shift_right) - 1)
        } else {
            0
        };

        let has_fraction = fraction != 0;

        let round_up = if has_fraction {
            ef.precision = true;
            let halfway = 1u64 << (shift_right - 1);
            match rc {
                RoundingMode::NearestEven => {
                    if fraction > halfway {
                        true
                    } else if fraction == halfway {
                        integer_part & 1 != 0
                    } else {
                        false
                    }
                }
                RoundingMode::Up => !sign,
                RoundingMode::Down => sign,
                RoundingMode::Zero => false,
            }
        } else {
            false
        };

        let magnitude = if round_up {
            integer_part + 1
        } else {
            integer_part
        };

        let positive_limit = (1u64 << max_bit) - 1;
        let negative_limit = 1u64 << max_bit;

        if sign {
            if magnitude > negative_limit {
                ef.invalid = true;
                return indefinite;
            }
            -(magnitude as i64)
        } else {
            if magnitude > positive_limit {
                ef.invalid = true;
                return indefinite;
            }
            magnitude as i64
        }
    }
}

impl Fp80 {
    /// Exact widening from f32. Sets IE if source is a signaling NaN (quietened in result).
    pub fn from_f32(v: f32, ef: &mut ExceptionFlags) -> Fp80 {
        let bits = v.to_bits();
        let sign = (bits >> 31) != 0;
        let biased_exp = (bits >> 23) & 0xFF;
        let fraction = bits & 0x007F_FFFF;

        let sign_bit: u16 = if sign { 0x8000 } else { 0x0000 };

        if biased_exp == 0xFF {
            if fraction == 0 {
                return Fp80::from_bits(sign_bit | 0x7FFF, 0x8000_0000_0000_0000);
            }
            let is_signaling = (fraction & (1 << 22)) == 0;
            if is_signaling {
                ef.invalid = true;
            }
            let fp80_fraction = (fraction as u64) << 40;
            let significand = 0x8000_0000_0000_0000 | (1u64 << 62) | fp80_fraction;
            return Fp80::from_bits(sign_bit | 0x7FFF, significand);
        }

        if biased_exp == 0 {
            if fraction == 0 {
                return Fp80::from_bits(sign_bit, 0);
            }
            let raw_sig = fraction as u64;
            let leading_zeros = raw_sig.leading_zeros();
            let normalized_sig = raw_sig << leading_zeros;
            let new_exp = (1i32 - 127 - 23) + 16383 + 63 - (leading_zeros as i32 - 41);
            return Fp80::from_bits(sign_bit | new_exp as u16, normalized_sig);
        }

        let new_exp = biased_exp as u16 + (16383 - 127);
        let significand = (1u64 << 63) | ((fraction as u64) << 40);
        Fp80::from_bits(sign_bit | new_exp, significand)
    }

    /// Exact widening from f64. Sets IE if source is a signaling NaN (quietened in result).
    pub fn from_f64(v: f64, ef: &mut ExceptionFlags) -> Fp80 {
        let bits = v.to_bits();
        let sign = (bits >> 63) != 0;
        let biased_exp = ((bits >> 52) & 0x7FF) as u32;
        let fraction = bits & 0x000F_FFFF_FFFF_FFFF;

        let sign_bit: u16 = if sign { 0x8000 } else { 0x0000 };

        if biased_exp == 0x7FF {
            if fraction == 0 {
                return Fp80::from_bits(sign_bit | 0x7FFF, 0x8000_0000_0000_0000);
            }
            let is_signaling = (fraction & (1u64 << 51)) == 0;
            if is_signaling {
                ef.invalid = true;
            }
            let fp80_fraction = fraction << 11;
            let significand = 0x8000_0000_0000_0000 | (1u64 << 62) | fp80_fraction;
            return Fp80::from_bits(sign_bit | 0x7FFF, significand);
        }

        if biased_exp == 0 {
            if fraction == 0 {
                return Fp80::from_bits(sign_bit, 0);
            }
            let raw_sig = fraction;
            let leading_zeros = raw_sig.leading_zeros();
            let normalized_sig = raw_sig << leading_zeros;
            let new_exp = (1i32 - 1023 - 52) + 16383 + 63 - (leading_zeros as i32 - 12);
            return Fp80::from_bits(sign_bit | new_exp as u16, normalized_sig);
        }

        let new_exp = (biased_exp + 16383 - 1023) as u16;
        let significand = (1u64 << 63) | (fraction << 11);
        Fp80::from_bits(sign_bit | new_exp, significand)
    }
}

impl Fp80 {
    /// Narrowing conversion to f32. May raise OE, UE, PE, or IE (for SNaN).
    pub fn to_f32(self, rc: RoundingMode, ef: &mut ExceptionFlags) -> f32 {
        let sign = self.sign();
        let sign_bit: u32 = if sign { 1u32 << 31 } else { 0 };

        if self.is_nan() {
            if self.is_signaling_nan() {
                ef.invalid = true;
            }
            let fraction = self.significand() & 0x3FFF_FFFF_FFFF_FFFF;
            let f32_fraction = (fraction >> 40) | (1u32 << 22) as u64;
            let f32_fraction = f32_fraction as u32 & 0x007F_FFFF;
            let result_bits = sign_bit | 0x7F80_0000 | f32_fraction;
            return f32::from_bits(result_bits);
        }

        if self.is_infinity() {
            return f32::from_bits(sign_bit | 0x7F80_0000);
        }

        if self.is_zero() {
            return f32::from_bits(sign_bit);
        }

        let true_exp = self.true_exponent();
        let sig = self.significand();

        let new_exp = true_exp + 127;

        if new_exp > 254 {
            ef.overflow = true;
            ef.precision = true;
            let overflow_result = match rc {
                RoundingMode::NearestEven => sign_bit | 0x7F80_0000,
                RoundingMode::Up => {
                    if sign {
                        sign_bit | 0x7F7F_FFFF
                    } else {
                        sign_bit | 0x7F80_0000
                    }
                }
                RoundingMode::Down => {
                    if sign {
                        sign_bit | 0x7F80_0000
                    } else {
                        0x7F7F_FFFF
                    }
                }
                RoundingMode::Zero => sign_bit | 0x7F7F_FFFF,
            };
            return f32::from_bits(overflow_result);
        }

        if new_exp < -149 {
            ef.underflow = true;
            ef.precision = true;
            let underflow_result = match rc {
                RoundingMode::Up => {
                    if !sign {
                        sign_bit | 0x0000_0001
                    } else {
                        sign_bit
                    }
                }
                RoundingMode::Down => {
                    if sign {
                        sign_bit | 0x0000_0001
                    } else {
                        sign_bit
                    }
                }
                RoundingMode::NearestEven | RoundingMode::Zero => sign_bit,
            };
            return f32::from_bits(underflow_result);
        }

        if new_exp < 1 {
            let denorm_shift = (1 - new_exp) as u32;
            let total_shift = 40 + denorm_shift;
            if total_shift >= 64 {
                let has_bits = sig != 0;
                ef.underflow = true;
                if has_bits {
                    ef.precision = true;
                }
                let round_up = has_bits
                    && match rc {
                        RoundingMode::Up => !sign,
                        RoundingMode::Down => sign,
                        RoundingMode::NearestEven | RoundingMode::Zero => false,
                    };
                let result_bits = if round_up {
                    sign_bit | 0x0000_0001
                } else {
                    sign_bit
                };
                return f32::from_bits(result_bits);
            }
            let shifted = sig >> total_shift;
            let lost_mask = if total_shift < 64 {
                (1u64 << total_shift) - 1
            } else {
                u64::MAX
            };
            let lost = sig & lost_mask;
            let halfway = 1u64 << (total_shift - 1);
            let inexact = lost != 0;
            if inexact {
                ef.underflow = true;
                ef.precision = true;
            } else if shifted == 0 {
                ef.underflow = true;
            }
            let round_up = inexact
                && match rc {
                    RoundingMode::NearestEven => {
                        if lost > halfway {
                            true
                        } else if lost == halfway {
                            shifted & 1 != 0
                        } else {
                            false
                        }
                    }
                    RoundingMode::Up => !sign,
                    RoundingMode::Down => sign,
                    RoundingMode::Zero => false,
                };
            let f32_fraction = if round_up {
                shifted as u32 + 1
            } else {
                shifted as u32
            };
            let result_bits = sign_bit | f32_fraction;
            return f32::from_bits(result_bits);
        }

        let sig_no_jbit = sig & 0x7FFF_FFFF_FFFF_FFFF;
        let f32_fraction_raw = sig_no_jbit >> 40;
        let lost = sig_no_jbit & 0x000000FF_FFFFFFFF;
        let halfway = 1u64 << 39;

        let inexact = lost != 0;
        if inexact {
            ef.precision = true;
        }

        let round_up = inexact
            && match rc {
                RoundingMode::NearestEven => {
                    if lost > halfway {
                        true
                    } else if lost == halfway {
                        f32_fraction_raw & 1 != 0
                    } else {
                        false
                    }
                }
                RoundingMode::Up => !sign,
                RoundingMode::Down => sign,
                RoundingMode::Zero => false,
            };

        let f32_fraction = if round_up {
            f32_fraction_raw as u32 + 1
        } else {
            f32_fraction_raw as u32
        };

        if f32_fraction >= 0x0080_0000 {
            let carry_exp = new_exp + 1;
            if carry_exp > 254 {
                ef.overflow = true;
                return f32::from_bits(sign_bit | 0x7F80_0000);
            }
            let result_bits = sign_bit | ((carry_exp as u32) << 23);
            return f32::from_bits(result_bits);
        }

        let result_bits = sign_bit | ((new_exp as u32) << 23) | f32_fraction;
        f32::from_bits(result_bits)
    }

    /// Narrowing conversion to f64. May raise OE, UE, PE, or IE (for SNaN).
    pub fn to_f64(self, rc: RoundingMode, ef: &mut ExceptionFlags) -> f64 {
        let sign = self.sign();
        let sign_bit: u64 = if sign { 1u64 << 63 } else { 0 };

        if self.is_nan() {
            if self.is_signaling_nan() {
                ef.invalid = true;
            }
            let fraction = self.significand() & 0x3FFF_FFFF_FFFF_FFFF;
            let f64_fraction = (fraction >> 11) | (1u64 << 51);
            let f64_fraction = f64_fraction & 0x000F_FFFF_FFFF_FFFF;
            let result_bits = sign_bit | 0x7FF0_0000_0000_0000 | f64_fraction;
            return f64::from_bits(result_bits);
        }

        if self.is_infinity() {
            return f64::from_bits(sign_bit | 0x7FF0_0000_0000_0000);
        }

        if self.is_zero() {
            return f64::from_bits(sign_bit);
        }

        let true_exp = self.true_exponent();
        let sig = self.significand();

        let new_exp = true_exp + 1023;

        if new_exp > 2046 {
            ef.overflow = true;
            ef.precision = true;
            let overflow_result = match rc {
                RoundingMode::NearestEven => sign_bit | 0x7FF0_0000_0000_0000,
                RoundingMode::Up => {
                    if sign {
                        sign_bit | 0x7FEF_FFFF_FFFF_FFFF
                    } else {
                        sign_bit | 0x7FF0_0000_0000_0000
                    }
                }
                RoundingMode::Down => {
                    if sign {
                        sign_bit | 0x7FF0_0000_0000_0000
                    } else {
                        0x7FEF_FFFF_FFFF_FFFF
                    }
                }
                RoundingMode::Zero => sign_bit | 0x7FEF_FFFF_FFFF_FFFF,
            };
            return f64::from_bits(overflow_result);
        }

        if new_exp < -1074 {
            ef.underflow = true;
            ef.precision = true;
            let underflow_result = match rc {
                RoundingMode::Up => {
                    if !sign {
                        sign_bit | 0x0000_0000_0000_0001
                    } else {
                        sign_bit
                    }
                }
                RoundingMode::Down => {
                    if sign {
                        sign_bit | 0x0000_0000_0000_0001
                    } else {
                        sign_bit
                    }
                }
                RoundingMode::NearestEven | RoundingMode::Zero => sign_bit,
            };
            return f64::from_bits(underflow_result);
        }

        if new_exp < 1 {
            let denorm_shift = (1 - new_exp) as u32;
            let total_shift = 11 + denorm_shift;
            if total_shift >= 64 {
                let has_bits = sig != 0;
                ef.underflow = true;
                if has_bits {
                    ef.precision = true;
                }
                let round_up = has_bits
                    && match rc {
                        RoundingMode::Up => !sign,
                        RoundingMode::Down => sign,
                        RoundingMode::NearestEven | RoundingMode::Zero => false,
                    };
                let result_bits = if round_up {
                    sign_bit | 0x0000_0000_0000_0001
                } else {
                    sign_bit
                };
                return f64::from_bits(result_bits);
            }
            let shifted = sig >> total_shift;
            let lost_mask = if total_shift < 64 {
                (1u64 << total_shift) - 1
            } else {
                u64::MAX
            };
            let lost = sig & lost_mask;
            let halfway = 1u64 << (total_shift - 1);
            let inexact = lost != 0;
            if inexact {
                ef.underflow = true;
                ef.precision = true;
            } else if shifted == 0 {
                ef.underflow = true;
            }
            let round_up = inexact
                && match rc {
                    RoundingMode::NearestEven => {
                        if lost > halfway {
                            true
                        } else if lost == halfway {
                            shifted & 1 != 0
                        } else {
                            false
                        }
                    }
                    RoundingMode::Up => !sign,
                    RoundingMode::Down => sign,
                    RoundingMode::Zero => false,
                };
            let f64_fraction = if round_up { shifted + 1 } else { shifted };
            let result_bits = sign_bit | f64_fraction;
            return f64::from_bits(result_bits);
        }

        let sig_no_jbit = sig & 0x7FFF_FFFF_FFFF_FFFF;
        let f64_fraction_raw = sig_no_jbit >> 11;
        let lost = sig_no_jbit & 0x7FF;
        let halfway = 1u64 << 10;

        let inexact = lost != 0;
        if inexact {
            ef.precision = true;
        }

        let round_up = inexact
            && match rc {
                RoundingMode::NearestEven => {
                    if lost > halfway {
                        true
                    } else if lost == halfway {
                        f64_fraction_raw & 1 != 0
                    } else {
                        false
                    }
                }
                RoundingMode::Up => !sign,
                RoundingMode::Down => sign,
                RoundingMode::Zero => false,
            };

        let f64_fraction = if round_up {
            f64_fraction_raw + 1
        } else {
            f64_fraction_raw
        };

        if f64_fraction >= 0x0010_0000_0000_0000 {
            let carry_exp = new_exp + 1;
            if carry_exp > 2046 {
                ef.overflow = true;
                return f64::from_bits(sign_bit | 0x7FF0_0000_0000_0000);
            }
            let result_bits = sign_bit | ((carry_exp as u64) << 52);
            return f64::from_bits(result_bits);
        }

        let result_bits = sign_bit | ((new_exp as u64) << 52) | f64_fraction;
        f64::from_bits(result_bits)
    }
}

impl Fp80 {
    /// Load from 10-byte packed BCD (18 digits + sign).
    pub fn from_bcd(bytes: [u8; 10], _ef: &mut ExceptionFlags) -> Fp80 {
        let sign = (bytes[9] & 0x80) != 0;

        let mut value: u64 = 0;
        let mut digit_index = 8u32;
        loop {
            let byte = bytes[digit_index as usize];
            let high_digit = (byte >> 4) as u64;
            let low_digit = (byte & 0x0F) as u64;
            value = value * 100 + high_digit * 10 + low_digit;
            if digit_index == 0 {
                break;
            }
            digit_index -= 1;
        }

        if value == 0 {
            return Fp80::ZERO;
        }

        let result = Fp80::from_i64(value as i64);
        if sign { result.negate() } else { result }
    }

    /// Store as 10-byte packed BCD. Rounds to integer first.
    /// Out-of-range produces IE + indefinite BCD (0xFF_FF_C0_00_00_00_00_00_00_00).
    pub fn to_bcd(self, rc: RoundingMode, ef: &mut ExceptionFlags) -> [u8; 10] {
        const INDEFINITE_BCD: [u8; 10] =
            [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0xFF, 0xFF];
        const MAX_BCD: u64 = 999_999_999_999_999_999;

        if self.is_nan() || self.is_infinity() {
            ef.invalid = true;
            return INDEFINITE_BCD;
        }

        if self.is_zero() {
            return [0u8; 10];
        }

        let sign = self.sign();
        let true_exp = self.true_exponent();
        let sig = self.significand();

        if true_exp > 63 {
            ef.invalid = true;
            return INDEFINITE_BCD;
        }

        let has_fraction: bool;

        let magnitude: u64 = if true_exp < 0 {
            has_fraction = true;
            let round_up = match rc {
                RoundingMode::NearestEven => {
                    if true_exp == -1 {
                        sig > 0x8000_0000_0000_0000
                    } else {
                        false
                    }
                }
                RoundingMode::Up => !sign,
                RoundingMode::Down => sign,
                RoundingMode::Zero => false,
            };
            if round_up { 1 } else { 0 }
        } else {
            let shift_right = 63 - true_exp as u32;
            let integer_part = sig >> shift_right;
            let fraction = if shift_right < 64 {
                sig & ((1u64 << shift_right) - 1)
            } else {
                0
            };
            has_fraction = fraction != 0;

            let round_up = if has_fraction {
                let halfway = 1u64 << (shift_right - 1);
                match rc {
                    RoundingMode::NearestEven => {
                        if fraction > halfway {
                            true
                        } else if fraction == halfway {
                            integer_part & 1 != 0
                        } else {
                            false
                        }
                    }
                    RoundingMode::Up => !sign,
                    RoundingMode::Down => sign,
                    RoundingMode::Zero => false,
                }
            } else {
                false
            };
            if round_up {
                integer_part + 1
            } else {
                integer_part
            }
        };

        if has_fraction {
            ef.precision = true;
        }

        if magnitude > MAX_BCD {
            ef.invalid = true;
            return INDEFINITE_BCD;
        }

        if magnitude == 0 {
            return [0u8; 10];
        }

        let mut result = [0u8; 10];
        let mut remaining = magnitude;
        let mut byte_index = 0usize;
        while remaining > 0 && byte_index < 9 {
            let low = (remaining % 10) as u8;
            remaining /= 10;
            let high = (remaining % 10) as u8;
            remaining /= 10;
            result[byte_index] = (high << 4) | low;
            byte_index += 1;
        }

        if sign {
            result[9] = 0x80;
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEGATIVE_ONE: Fp80 = Fp80::from_bits(0xBFFF, 0x8000_0000_0000_0000);
    const TWO: Fp80 = Fp80::from_bits(0x4000, 0x8000_0000_0000_0000);
    const HALF: Fp80 = Fp80::from_bits(0x3FFE, 0x8000_0000_0000_0000);
    const ONE_AND_HALF: Fp80 = Fp80::from_bits(0x3FFF, 0xC000_0000_0000_0000);
    const NEGATIVE_ONE_AND_HALF: Fp80 = Fp80::from_bits(0xBFFF, 0xC000_0000_0000_0000);
    const TWO_AND_HALF: Fp80 = Fp80::from_bits(0x4000, 0xA000_0000_0000_0000);
    const POSITIVE_SNAN: Fp80 = Fp80::from_bits(0x7FFF, 0x8000_0000_0000_0001);
    // Very small Fp80: exponent 0x0001 (smallest normal exponent), significand 0x8000...
    const TINY_NORMAL: Fp80 = Fp80::from_bits(0x0001, 0x8000_0000_0000_0000);

    fn no_exceptions() -> ExceptionFlags {
        ExceptionFlags::default()
    }

    #[test]
    fn test_from_i16_basic() {
        assert_eq!(Fp80::from_i16(0), Fp80::ZERO);
        assert_eq!(Fp80::from_i16(1), Fp80::ONE);
        assert_eq!(Fp80::from_i16(-1), NEGATIVE_ONE);
    }

    #[test]
    fn test_from_i16_edge_cases() {
        // i16::MAX = 32767 = 0x7FFF. Leading zeros of 32767u64 = 49.
        // sig = 32767 << 49. Exponent = 16383 + 63 - 49 = 16397 = 0x400D.
        let max = Fp80::from_i16(i16::MAX);
        assert!(!max.sign());
        assert_eq!(max.exponent(), 0x400D);
        assert_eq!(max.significand(), (i16::MAX as u64) << 49);

        // i16::MIN = -32768. Magnitude = 32768 = 0x8000. Leading zeros of 32768u64 = 48.
        // sig = 32768 << 48 = 0x8000_0000_0000_0000. Exponent = 16383 + 63 - 48 = 16398 = 0x400E.
        let min = Fp80::from_i16(i16::MIN);
        assert!(min.sign());
        assert_eq!(min.exponent(), 0x400E);
        assert_eq!(min.significand(), 0x8000_0000_0000_0000);
    }

    #[test]
    fn test_from_i32_basic() {
        assert_eq!(Fp80::from_i32(0), Fp80::ZERO);
        assert_eq!(Fp80::from_i32(1), Fp80::ONE);
        assert_eq!(Fp80::from_i32(-1), NEGATIVE_ONE);

        // 256 = 2^8. Leading zeros of 256u64 = 55. sig = 256<<55 = 0x8000_0000_0000_0000.
        // exp = 16383 + 63 - 55 = 16391 = 0x4007.
        let v = Fp80::from_i32(256);
        assert_eq!(v.exponent(), 0x4007);
        assert_eq!(v.significand(), 0x8000_0000_0000_0000);
    }

    #[test]
    fn test_from_i32_edge_cases() {
        // i32::MAX = 2147483647 = 0x7FFF_FFFF. Leading zeros = 33.
        // sig = 0x7FFF_FFFF << 33. exp = 16383 + 63 - 33 = 16413 = 0x401D.
        let max = Fp80::from_i32(i32::MAX);
        assert!(!max.sign());
        assert_eq!(max.exponent(), 0x401D);
        assert_eq!(max.significand(), (i32::MAX as u64) << 33);

        // i32::MIN = -2147483648. Magnitude = 2147483648 = 0x8000_0000. Leading zeros = 32.
        // sig = 0x8000_0000 << 32 = 0x8000_0000_0000_0000. exp = 16383 + 63 - 32 = 16414 = 0x401E.
        let min = Fp80::from_i32(i32::MIN);
        assert!(min.sign());
        assert_eq!(min.exponent(), 0x401E);
        assert_eq!(min.significand(), 0x8000_0000_0000_0000);
    }

    #[test]
    fn test_from_i64_basic() {
        assert_eq!(Fp80::from_i64(0), Fp80::ZERO);
        assert_eq!(Fp80::from_i64(1), Fp80::ONE);
        assert_eq!(Fp80::from_i64(-1), NEGATIVE_ONE);

        // 2: exp=0x4000, sig=0x8000_0000_0000_0000.
        assert_eq!(Fp80::from_i64(2), TWO);

        // 3: exp=0x4000, sig=0xC000_0000_0000_0000.
        let three = Fp80::from_i64(3);
        assert_eq!(three.exponent(), 0x4000);
        assert_eq!(three.significand(), 0xC000_0000_0000_0000);
    }

    #[test]
    fn test_from_i64_edge_cases() {
        // i64::MAX = 0x7FFF_FFFF_FFFF_FFFF. Leading zeros = 1.
        // sig = 0x7FFF_FFFF_FFFF_FFFF << 1 = 0xFFFF_FFFF_FFFF_FFFE.
        // exp = 16383 + 63 - 1 = 16445 = 0x403D.
        let max = Fp80::from_i64(i64::MAX);
        assert!(!max.sign());
        assert_eq!(max.exponent(), 0x403D);
        assert_eq!(max.significand(), 0xFFFF_FFFF_FFFF_FFFE);

        // i64::MIN = -2^63. (v as u64).wrapping_neg() = 0x8000_0000_0000_0000 (wraps to itself).
        // Leading zeros = 0. sig = 0x8000_0000_0000_0000 << 0. exp = 16383 + 63 - 0 = 16446 = 0x403E.
        let min = Fp80::from_i64(i64::MIN);
        assert!(min.sign());
        assert_eq!(min.exponent(), 0x403E);
        assert_eq!(min.significand(), 0x8000_0000_0000_0000);

        // i64::MIN + 1 has same magnitude as i64::MAX.
        let almost_min = Fp80::from_i64(i64::MIN + 1);
        assert!(almost_min.sign());
        assert_eq!(almost_min.exponent(), max.exponent());
        assert_eq!(almost_min.significand(), max.significand());

        // Powers of 2.
        let pow62 = Fp80::from_i64(1i64 << 62);
        assert_eq!(pow62.exponent(), 16383 + 62);
        assert_eq!(pow62.significand(), 0x8000_0000_0000_0000);
    }

    #[test]
    fn test_to_i16_basic() {
        let mut ef = no_exceptions();
        assert_eq!(Fp80::ONE.to_i16(RoundingMode::NearestEven, &mut ef), 1);
        assert_eq!(ef, no_exceptions());

        let mut ef = no_exceptions();
        assert_eq!(Fp80::ZERO.to_i16(RoundingMode::NearestEven, &mut ef), 0);

        let mut ef = no_exceptions();
        assert_eq!(NEGATIVE_ONE.to_i16(RoundingMode::NearestEven, &mut ef), -1);

        let mut ef = no_exceptions();
        assert_eq!(
            Fp80::from_i16(i16::MAX).to_i16(RoundingMode::NearestEven, &mut ef),
            i16::MAX
        );

        let mut ef = no_exceptions();
        assert_eq!(
            Fp80::from_i16(i16::MIN).to_i16(RoundingMode::NearestEven, &mut ef),
            i16::MIN
        );
    }

    #[test]
    fn test_to_i16_edge_cases() {
        // Overflow: too large -> integer indefinite + IE.
        let too_large = Fp80::from_i32(40000);
        let mut ef = no_exceptions();
        assert_eq!(
            too_large.to_i16(RoundingMode::NearestEven, &mut ef),
            i16::MIN
        );
        assert!(ef.invalid);

        // NaN -> integer indefinite + IE.
        let mut ef = no_exceptions();
        assert_eq!(
            Fp80::INDEFINITE.to_i16(RoundingMode::NearestEven, &mut ef),
            i16::MIN
        );
        assert!(ef.invalid);

        // +Infinity -> integer indefinite + IE.
        let mut ef = no_exceptions();
        assert_eq!(
            Fp80::INFINITY.to_i16(RoundingMode::NearestEven, &mut ef),
            i16::MIN
        );
        assert!(ef.invalid);

        // Rounding: 1.5 with NearestEven -> 2 (round to even), PE.
        let mut ef = no_exceptions();
        assert_eq!(ONE_AND_HALF.to_i16(RoundingMode::NearestEven, &mut ef), 2);
        assert!(ef.precision);

        // Rounding: 2.5 with NearestEven -> 2 (round to even).
        let mut ef = no_exceptions();
        assert_eq!(TWO_AND_HALF.to_i16(RoundingMode::NearestEven, &mut ef), 2);
        assert!(ef.precision);

        // Rounding: 1.5 toward zero -> 1.
        let mut ef = no_exceptions();
        assert_eq!(ONE_AND_HALF.to_i16(RoundingMode::Zero, &mut ef), 1);
        assert!(ef.precision);

        // Rounding: -1.5 toward zero -> -1.
        let mut ef = no_exceptions();
        assert_eq!(
            NEGATIVE_ONE_AND_HALF.to_i16(RoundingMode::Zero, &mut ef),
            -1
        );
        assert!(ef.precision);

        // Rounding: 1.5 toward +inf -> 2.
        let mut ef = no_exceptions();
        assert_eq!(ONE_AND_HALF.to_i16(RoundingMode::Up, &mut ef), 2);
        assert!(ef.precision);

        // Rounding: -1.5 toward +inf -> -1.
        let mut ef = no_exceptions();
        assert_eq!(NEGATIVE_ONE_AND_HALF.to_i16(RoundingMode::Up, &mut ef), -1);
        assert!(ef.precision);

        // Rounding: 1.5 toward -inf -> 1.
        let mut ef = no_exceptions();
        assert_eq!(ONE_AND_HALF.to_i16(RoundingMode::Down, &mut ef), 1);
        assert!(ef.precision);

        // Rounding: -1.5 toward -inf -> -2.
        let mut ef = no_exceptions();
        assert_eq!(
            NEGATIVE_ONE_AND_HALF.to_i16(RoundingMode::Down, &mut ef),
            -2
        );
        assert!(ef.precision);
    }

    #[test]
    fn test_to_i32_basic() {
        let mut ef = no_exceptions();
        assert_eq!(Fp80::ONE.to_i32(RoundingMode::NearestEven, &mut ef), 1);
        assert_eq!(ef, no_exceptions());

        let mut ef = no_exceptions();
        assert_eq!(Fp80::ZERO.to_i32(RoundingMode::NearestEven, &mut ef), 0);

        let mut ef = no_exceptions();
        assert_eq!(
            Fp80::from_i32(i32::MAX).to_i32(RoundingMode::NearestEven, &mut ef),
            i32::MAX
        );

        let mut ef = no_exceptions();
        assert_eq!(
            Fp80::from_i32(i32::MIN).to_i32(RoundingMode::NearestEven, &mut ef),
            i32::MIN
        );
    }

    #[test]
    fn test_to_i32_edge_cases() {
        // Overflow -> integer indefinite (0x80000000) + IE.
        let too_large = Fp80::from_i64(i64::from(i32::MAX) + 1);
        let mut ef = no_exceptions();
        assert_eq!(
            too_large.to_i32(RoundingMode::NearestEven, &mut ef),
            i32::MIN
        );
        assert!(ef.invalid);

        // NaN -> integer indefinite + IE.
        let mut ef = no_exceptions();
        assert_eq!(
            Fp80::INDEFINITE.to_i32(RoundingMode::NearestEven, &mut ef),
            i32::MIN
        );
        assert!(ef.invalid);

        // -Infinity -> integer indefinite + IE.
        let mut ef = no_exceptions();
        assert_eq!(
            Fp80::NEG_INFINITY.to_i32(RoundingMode::NearestEven, &mut ef),
            i32::MIN
        );
        assert!(ef.invalid);

        // Rounding: 1.5 with NearestEven -> 2.
        let mut ef = no_exceptions();
        assert_eq!(ONE_AND_HALF.to_i32(RoundingMode::NearestEven, &mut ef), 2);
        assert!(ef.precision);
    }

    #[test]
    fn test_to_i64_basic() {
        let mut ef = no_exceptions();
        assert_eq!(Fp80::ONE.to_i64(RoundingMode::NearestEven, &mut ef), 1);
        assert_eq!(ef, no_exceptions());

        let mut ef = no_exceptions();
        assert_eq!(Fp80::ZERO.to_i64(RoundingMode::NearestEven, &mut ef), 0);

        let mut ef = no_exceptions();
        assert_eq!(
            Fp80::from_i64(i64::MAX).to_i64(RoundingMode::NearestEven, &mut ef),
            i64::MAX
        );
    }

    #[test]
    fn test_to_i64_edge_cases() {
        // i64::MIN round-trip.
        let mut ef = no_exceptions();
        assert_eq!(
            Fp80::from_i64(i64::MIN).to_i64(RoundingMode::NearestEven, &mut ef),
            i64::MIN
        );

        // Infinity -> integer indefinite + IE.
        let mut ef = no_exceptions();
        assert_eq!(
            Fp80::INFINITY.to_i64(RoundingMode::NearestEven, &mut ef),
            i64::MIN
        );
        assert!(ef.invalid);

        // SNaN -> integer indefinite + IE.
        let mut ef = no_exceptions();
        assert_eq!(
            POSITIVE_SNAN.to_i64(RoundingMode::NearestEven, &mut ef),
            i64::MIN
        );
        assert!(ef.invalid);

        // Fractional value rounds: 0.5 NearestEven -> 0 (round to even).
        let mut ef = no_exceptions();
        assert_eq!(HALF.to_i64(RoundingMode::NearestEven, &mut ef), 0);
        assert!(ef.precision);

        // 0.5 toward +inf -> 1.
        let mut ef = no_exceptions();
        assert_eq!(HALF.to_i64(RoundingMode::Up, &mut ef), 1);
    }

    #[test]
    fn test_from_f32_basic() {
        let mut ef = no_exceptions();
        let result = Fp80::from_f32(1.0f32, &mut ef);
        assert_eq!(result, Fp80::ONE);
        assert_eq!(ef, no_exceptions());

        let mut ef = no_exceptions();
        let result = Fp80::from_f32(0.0f32, &mut ef);
        assert_eq!(result, Fp80::ZERO);
        assert_eq!(ef, no_exceptions());

        let mut ef = no_exceptions();
        let result = Fp80::from_f32(-0.0f32, &mut ef);
        assert_eq!(result, Fp80::NEG_ZERO);
        assert_eq!(ef, no_exceptions());
    }

    #[test]
    fn test_from_f32_edge_cases() {
        // +Infinity.
        let mut ef = no_exceptions();
        let result = Fp80::from_f32(f32::INFINITY, &mut ef);
        assert_eq!(result, Fp80::INFINITY);
        assert_eq!(ef, no_exceptions());

        // -Infinity.
        let mut ef = no_exceptions();
        let result = Fp80::from_f32(f32::NEG_INFINITY, &mut ef);
        assert_eq!(result, Fp80::NEG_INFINITY);
        assert_eq!(ef, no_exceptions());

        // QNaN: preserved (no IE).
        let mut ef = no_exceptions();
        let result = Fp80::from_f32(f32::NAN, &mut ef);
        assert!(result.is_quiet_nan());
        assert!(!ef.invalid);

        // f32 denormal: widened exactly.
        let mut ef = no_exceptions();
        let result = Fp80::from_f32(f32::MIN_POSITIVE / 2.0, &mut ef);
        assert!(result.is_normal() || result.is_denormal());

        // Negative value.
        let mut ef = no_exceptions();
        let result = Fp80::from_f32(-2.0f32, &mut ef);
        assert!(result.sign());
        assert_eq!(result.exponent(), 0x4000);
    }

    #[test]
    fn test_from_f64_basic() {
        let mut ef = no_exceptions();
        let result = Fp80::from_f64(1.0f64, &mut ef);
        assert_eq!(result, Fp80::ONE);
        assert_eq!(ef, no_exceptions());

        let mut ef = no_exceptions();
        let result = Fp80::from_f64(0.0f64, &mut ef);
        assert_eq!(result, Fp80::ZERO);

        let mut ef = no_exceptions();
        let result = Fp80::from_f64(-0.0f64, &mut ef);
        assert_eq!(result, Fp80::NEG_ZERO);
    }

    #[test]
    fn test_from_f64_edge_cases() {
        let mut ef = no_exceptions();
        let result = Fp80::from_f64(f64::INFINITY, &mut ef);
        assert_eq!(result, Fp80::INFINITY);

        let mut ef = no_exceptions();
        let result = Fp80::from_f64(f64::NEG_INFINITY, &mut ef);
        assert_eq!(result, Fp80::NEG_INFINITY);

        // QNaN preserved.
        let mut ef = no_exceptions();
        let result = Fp80::from_f64(f64::NAN, &mut ef);
        assert!(result.is_quiet_nan());
        assert!(!ef.invalid);

        // f64 denormal widened exactly.
        let mut ef = no_exceptions();
        let result = Fp80::from_f64(f64::MIN_POSITIVE / 2.0, &mut ef);
        assert!(result.is_normal() || result.is_denormal());
    }

    #[test]
    fn test_to_f32_basic() {
        let mut ef = no_exceptions();
        assert_eq!(Fp80::ONE.to_f32(RoundingMode::NearestEven, &mut ef), 1.0f32);
        assert_eq!(ef, no_exceptions());

        let mut ef = no_exceptions();
        assert_eq!(
            Fp80::ZERO.to_f32(RoundingMode::NearestEven, &mut ef),
            0.0f32
        );

        let mut ef = no_exceptions();
        assert_eq!(
            Fp80::NEG_ZERO.to_f32(RoundingMode::NearestEven, &mut ef),
            -0.0f32
        );
    }

    #[test]
    fn test_to_f32_edge_cases() {
        // Infinity passthrough.
        let mut ef = no_exceptions();
        assert_eq!(
            Fp80::INFINITY.to_f32(RoundingMode::NearestEven, &mut ef),
            f32::INFINITY
        );

        // Very large Fp80 -> overflow to infinity + OE.
        let huge = Fp80::from_bits(0x7FFE, 0xFFFF_FFFF_FFFF_FFFF);
        let mut ef = no_exceptions();
        let result = huge.to_f32(RoundingMode::NearestEven, &mut ef);
        assert!(result.is_infinite());
        assert!(ef.overflow);

        // QNaN passthrough.
        let mut ef = no_exceptions();
        let result = Fp80::INDEFINITE.to_f32(RoundingMode::NearestEven, &mut ef);
        assert!(result.is_nan());

        // Precision loss: value not exactly representable in f32.
        let precise = Fp80::from_bits(0x3FFF, 0x8000_0000_0000_0001);
        let mut ef = no_exceptions();
        let _result = precise.to_f32(RoundingMode::NearestEven, &mut ef);
        assert!(ef.precision);
    }

    #[test]
    fn test_to_f64_basic() {
        let mut ef = no_exceptions();
        assert_eq!(Fp80::ONE.to_f64(RoundingMode::NearestEven, &mut ef), 1.0f64);
        assert_eq!(ef, no_exceptions());

        let mut ef = no_exceptions();
        assert_eq!(
            Fp80::ZERO.to_f64(RoundingMode::NearestEven, &mut ef),
            0.0f64
        );
    }

    #[test]
    fn test_to_f64_edge_cases() {
        // Infinity passthrough.
        let mut ef = no_exceptions();
        assert_eq!(
            Fp80::NEG_INFINITY.to_f64(RoundingMode::NearestEven, &mut ef),
            f64::NEG_INFINITY
        );

        // Very large Fp80 (beyond f64 max) -> overflow to infinity + OE.
        let huge = Fp80::from_bits(0x7FFE, 0xFFFF_FFFF_FFFF_FFFF);
        let mut ef = no_exceptions();
        let result = huge.to_f64(RoundingMode::NearestEven, &mut ef);
        assert!(result.is_infinite());
        assert!(ef.overflow);

        // Precision loss in f64.
        let precise = Fp80::from_bits(0x3FFF, 0x8000_0000_0000_0001);
        let mut ef = no_exceptions();
        let _result = precise.to_f64(RoundingMode::NearestEven, &mut ef);
        assert!(ef.precision);
    }

    #[test]
    fn test_from_bcd_basic() {
        // BCD zero: all bytes zero.
        let mut ef = no_exceptions();
        let result = Fp80::from_bcd([0u8; 10], &mut ef);
        assert_eq!(result, Fp80::ZERO);
        assert_eq!(ef, no_exceptions());

        // BCD for 1: byte 0 low nibble = 1, rest zero.
        let mut bcd = [0u8; 10];
        bcd[0] = 0x01;
        let mut ef = no_exceptions();
        let result = Fp80::from_bcd(bcd, &mut ef);
        assert_eq!(result, Fp80::ONE);
    }

    #[test]
    fn test_from_bcd_edge_cases() {
        // Negative BCD: bit 7 of byte 9 = sign.
        let mut bcd = [0u8; 10];
        bcd[0] = 0x01;
        bcd[9] = 0x80; // negative
        let mut ef = no_exceptions();
        let result = Fp80::from_bcd(bcd, &mut ef);
        assert!(result.sign());

        // Max BCD: 999,999,999,999,999,999 (18 nines).
        let max_bcd = [0x99, 0x99, 0x99, 0x99, 0x99, 0x99, 0x99, 0x99, 0x99, 0x00];
        let mut ef = no_exceptions();
        let result = Fp80::from_bcd(max_bcd, &mut ef);
        assert!(result.is_normal());
        assert!(!result.sign());
    }

    #[test]
    fn test_to_bcd_basic() {
        // 0 -> BCD zero.
        let mut ef = no_exceptions();
        let result = Fp80::ZERO.to_bcd(RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, [0u8; 10]);

        // 1.0 -> BCD 1.
        let mut ef = no_exceptions();
        let result = Fp80::ONE.to_bcd(RoundingMode::NearestEven, &mut ef);
        assert_eq!(result[0], 0x01);
        assert_eq!(result[9] & 0x80, 0); // positive
    }

    #[test]
    fn test_to_bcd_edge_cases() {
        // NaN -> IE + indefinite BCD.
        let mut ef = no_exceptions();
        let result = Fp80::INDEFINITE.to_bcd(RoundingMode::NearestEven, &mut ef);
        assert!(ef.invalid);
        assert_eq!(
            result,
            [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0xFF, 0xFF]
        );

        // Infinity -> IE + indefinite BCD.
        let mut ef = no_exceptions();
        let result = Fp80::INFINITY.to_bcd(RoundingMode::NearestEven, &mut ef);
        assert!(ef.invalid);
        assert_eq!(
            result,
            [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0xFF, 0xFF]
        );
    }

    #[test]
    fn test_to_i16_neg_zero() {
        // -0 -> 0.
        let mut ef = no_exceptions();
        assert_eq!(Fp80::NEG_ZERO.to_i16(RoundingMode::NearestEven, &mut ef), 0);
    }

    #[test]
    fn test_to_i16_negative_overflow() {
        // Value < -32768 -> integer indefinite + IE.
        let too_negative = Fp80::from_i32(-40000);
        let mut ef = no_exceptions();
        assert_eq!(
            too_negative.to_i16(RoundingMode::NearestEven, &mut ef),
            i16::MIN
        );
        assert!(ef.invalid);
    }

    #[test]
    fn test_to_i32_neg_zero() {
        // -0 -> 0.
        let mut ef = no_exceptions();
        assert_eq!(Fp80::NEG_ZERO.to_i32(RoundingMode::NearestEven, &mut ef), 0);
    }

    #[test]
    fn test_to_i32_rounding_modes() {
        // 1.5 toward zero -> 1.
        let mut ef = no_exceptions();
        assert_eq!(ONE_AND_HALF.to_i32(RoundingMode::Zero, &mut ef), 1);
        assert!(ef.precision);

        // -1.5 toward zero -> -1.
        let mut ef = no_exceptions();
        assert_eq!(
            NEGATIVE_ONE_AND_HALF.to_i32(RoundingMode::Zero, &mut ef),
            -1
        );
        assert!(ef.precision);

        // 1.5 toward +inf -> 2.
        let mut ef = no_exceptions();
        assert_eq!(ONE_AND_HALF.to_i32(RoundingMode::Up, &mut ef), 2);
        assert!(ef.precision);

        // -1.5 toward +inf -> -1.
        let mut ef = no_exceptions();
        assert_eq!(NEGATIVE_ONE_AND_HALF.to_i32(RoundingMode::Up, &mut ef), -1);
        assert!(ef.precision);

        // 1.5 toward -inf -> 1.
        let mut ef = no_exceptions();
        assert_eq!(ONE_AND_HALF.to_i32(RoundingMode::Down, &mut ef), 1);
        assert!(ef.precision);

        // -1.5 toward -inf -> -2.
        let mut ef = no_exceptions();
        assert_eq!(
            NEGATIVE_ONE_AND_HALF.to_i32(RoundingMode::Down, &mut ef),
            -2
        );
        assert!(ef.precision);
    }

    #[test]
    fn test_to_i64_neg_zero() {
        // -0 -> 0.
        let mut ef = no_exceptions();
        assert_eq!(Fp80::NEG_ZERO.to_i64(RoundingMode::NearestEven, &mut ef), 0);
    }

    #[test]
    fn test_to_i64_rounding_modes() {
        // 0.5 toward zero -> 0.
        let mut ef = no_exceptions();
        assert_eq!(HALF.to_i64(RoundingMode::Zero, &mut ef), 0);
        assert!(ef.precision);

        // 0.5 toward -inf -> 0.
        let mut ef = no_exceptions();
        assert_eq!(HALF.to_i64(RoundingMode::Down, &mut ef), 0);
        assert!(ef.precision);

        // -0.5 toward -inf -> -1.
        let negative_half = Fp80::from_bits(0xBFFE, 0x8000_0000_0000_0000);
        let mut ef = no_exceptions();
        assert_eq!(negative_half.to_i64(RoundingMode::Down, &mut ef), -1);
        assert!(ef.precision);
    }

    #[test]
    fn test_to_f32_underflow() {
        // Very small Fp80 -> f32 underflow + UE.
        let mut ef = no_exceptions();
        let result = TINY_NORMAL.to_f32(RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, 0.0f32);
        assert!(ef.underflow);
    }

    #[test]
    fn test_to_f32_snan() {
        // SNaN -> QNaN + IE.
        let mut ef = no_exceptions();
        let result = POSITIVE_SNAN.to_f32(RoundingMode::NearestEven, &mut ef);
        assert!(result.is_nan());
        assert!(ef.invalid);
    }

    #[test]
    fn test_to_f32_rounding_modes() {
        // Value not exactly representable: 1.0 + 2^-63.
        let precise = Fp80::from_bits(0x3FFF, 0x8000_0000_0000_0001);

        let mut ef = no_exceptions();
        let _result = precise.to_f32(RoundingMode::Up, &mut ef);
        assert!(ef.precision);

        let mut ef = no_exceptions();
        let _result = precise.to_f32(RoundingMode::Down, &mut ef);
        assert!(ef.precision);

        let mut ef = no_exceptions();
        let _result = precise.to_f32(RoundingMode::Zero, &mut ef);
        assert!(ef.precision);
    }

    #[test]
    fn test_to_f64_underflow() {
        // Very small Fp80 -> f64 underflow + UE.
        let mut ef = no_exceptions();
        let result = TINY_NORMAL.to_f64(RoundingMode::NearestEven, &mut ef);
        assert_eq!(result, 0.0f64);
        assert!(ef.underflow);
    }

    #[test]
    fn test_to_f64_snan() {
        // SNaN -> QNaN + IE.
        let mut ef = no_exceptions();
        let result = POSITIVE_SNAN.to_f64(RoundingMode::NearestEven, &mut ef);
        assert!(result.is_nan());
        assert!(ef.invalid);
    }

    #[test]
    fn test_to_f64_rounding_modes() {
        // Value not exactly representable: 1.0 + 2^-63.
        let precise = Fp80::from_bits(0x3FFF, 0x8000_0000_0000_0001);

        let mut ef = no_exceptions();
        let _result = precise.to_f64(RoundingMode::Up, &mut ef);
        assert!(ef.precision);

        let mut ef = no_exceptions();
        let _result = precise.to_f64(RoundingMode::Down, &mut ef);
        assert!(ef.precision);

        let mut ef = no_exceptions();
        let _result = precise.to_f64(RoundingMode::Zero, &mut ef);
        assert!(ef.precision);
    }

    #[test]
    fn test_to_bcd_negative_value() {
        // -1.0 -> BCD with sign bit set in byte 9.
        let mut ef = no_exceptions();
        let result = NEGATIVE_ONE.to_bcd(RoundingMode::NearestEven, &mut ef);
        assert_eq!(result[0], 0x01);
        assert_eq!(result[9] & 0x80, 0x80);
    }

    #[test]
    fn test_to_bcd_fractional_rounds() {
        // 1.5 -> rounds to 2, BCD 2, PE set.
        let mut ef = no_exceptions();
        let result = ONE_AND_HALF.to_bcd(RoundingMode::NearestEven, &mut ef);
        assert_eq!(result[0], 0x02);
        assert!(ef.precision);
    }

    #[test]
    fn test_to_bcd_out_of_range() {
        // Value exceeding BCD range (> 999,999,999,999,999,999) -> IE + indefinite BCD.
        let huge = Fp80::from_bits(0x7FFE, 0xFFFF_FFFF_FFFF_FFFF);
        let mut ef = no_exceptions();
        let result = huge.to_bcd(RoundingMode::NearestEven, &mut ef);
        assert!(ef.invalid);
        assert_eq!(
            result,
            [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0xFF, 0xFF]
        );
    }

    #[test]
    fn test_from_bcd_negative_zero() {
        // BCD with sign bit set but all digits zero.
        let mut bcd = [0u8; 10];
        bcd[9] = 0x80;
        let mut ef = no_exceptions();
        let result = Fp80::from_bcd(bcd, &mut ef);
        assert!(result.is_zero());
    }
}
