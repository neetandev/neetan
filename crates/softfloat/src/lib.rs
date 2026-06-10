//! Software floating-point library for the 80-bit extended precision format used by the x87 FPU.
//!
//! This crate implements the [`Fp80`] type - an IEEE 754 extended double (80-bit) soft-float -
//! providing all operations required by the 387 and 486DX FPU: basic arithmetic (add, sub, mul,
//! div, sqrt), integer/float/BCD conversions, ordered and unordered comparisons, and the full set
//! of x87 transcendentals (F2XM1, FYL2X, FYL2XP1, FSIN, FCOS, FSINCOS, FPTAN, FPATAN).
//!
//! Arithmetic operations respect both precision control (24/53/64-bit significand) and all four
//! IEEE 754 rounding modes. The six x87 exception flags (IE, DE, ZE, OE, UE, PE) are accumulated
//! through an [`ExceptionFlags`] parameter rather than global state.
//!
//! Transcendental functions use double-double (`f64 × 2`) intermediate arithmetic, providing
//! approximately 106 bits of significand precision - well above the 62-bit relative error
//! guarantee of the real 486DX (Intel i486 Programmer's Reference, §17.5: |relative error| <
//! 2⁻⁶²). Trigonometric functions intentionally use the same 66-bit π approximation as the
//! hardware (`p = 4 × 0.C90FDAA2_2168C234_C`) so that results match real silicon rather than
//! being mathematically superior. Polynomial coefficients for exp, ln, sin, cos, and atan series
//! are computed at compile time via `const fn` Dekker two-sum/two-product transformations.

#![warn(missing_docs)]
#![forbid(unsafe_code)]
#![cfg_attr(not(test), no_std)]

mod arithmetic;
mod compare;
mod convert;
mod double_f64;
mod other;
mod transcendental;

/// Maps to CW bits 11–10 (RC field).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoundingMode {
    /// RC=00 - round to nearest, ties to even
    NearestEven,
    /// RC=01 - round toward −∞
    Down,
    /// RC=10 - round toward +∞
    Up,
    /// RC=11 - round toward zero (truncate)
    Zero,
}

/// Maps to CW bits 9–8 (PC field).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Precision {
    /// PC=00 - 24-bit significand
    Single,
    /// PC=10 - 53-bit significand
    Double,
    /// PC=11 - 64-bit significand (default)
    Extended,
}

/// Accumulator for exceptions raised during operations. Operations OR new exceptions into the flags
/// and never clear them. This matches how the x87 status word accumulates exception bits.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExceptionFlags {
    /// IE - invalid operation
    pub invalid: bool,
    /// DE - denormalized operand
    pub denormal: bool,
    /// ZE - division by zero
    pub zero_divide: bool,
    /// OE - result too large
    pub overflow: bool,
    /// UE - result too small
    pub underflow: bool,
    /// PE - result was rounded
    pub precision: bool,
}

/// Result of floating-point comparison operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FpOrdering {
    /// Lhs is less.
    Less,
    /// Lhs and Rhs are equal.
    Equal,
    /// Lhs is greater.
    Greater,
    /// One or both operands are NaN.
    Unordered,
}

/// Value class for the FXAM instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FpClass {
    /// unnormal, pseudo-infinity, pseudo-NaN
    Unsupported,
    /// quiet or signaling NaN
    Nan,
    /// normalized finite number
    Normal,
    /// +∞ or −∞
    Infinity,
    /// +0 or −0
    Zero,
    /// register tagged as empty (not a property of the value itself)
    Empty,
    /// denormalized or pseudo-denormal
    Denormal,
}

/// Custom 80-bit floating point for x87.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fp80 {
    /// bits 63..0 (J-bit at bit 63, fraction at bits 62..0)
    significand: u64,
    /// bit 15 = sign, bits 14..0 = biased exponent
    sign_exponent: u16,
}

impl Fp80 {
    /// +0.0
    pub const ZERO: Fp80 = Fp80 {
        sign_exponent: 0x0000,
        significand: 0x0000_0000_0000_0000,
    };

    /// −0.0
    pub const NEG_ZERO: Fp80 = Fp80 {
        sign_exponent: 0x8000,
        significand: 0x0000_0000_0000_0000,
    };

    /// +1.0
    pub const ONE: Fp80 = Fp80 {
        sign_exponent: 0x3FFF,
        significand: 0x8000_0000_0000_0000,
    };

    /// +∞
    pub const INFINITY: Fp80 = Fp80 {
        sign_exponent: 0x7FFF,
        significand: 0x8000_0000_0000_0000,
    };

    /// −∞
    pub const NEG_INFINITY: Fp80 = Fp80 {
        sign_exponent: 0xFFFF,
        significand: 0x8000_0000_0000_0000,
    };

    /// Default NaN (indefinite): negative quiet NaN with zero payload.
    pub const INDEFINITE: Fp80 = Fp80 {
        sign_exponent: 0xFFFF,
        significand: 0xC000_0000_0000_0000,
    };

    /// log₂(10) higher significand - used when RC = Up.
    pub const LOG2_10_UP: Fp80 = Fp80 {
        sign_exponent: 0x4000,
        significand: 0xD49A_784B_CD1B_8AFF,
    };

    /// log₂(10) lower significand - used when RC ∈ {NearestEven, Down, Zero}.
    pub const LOG2_10_DOWN: Fp80 = Fp80 {
        sign_exponent: 0x4000,
        significand: 0xD49A_784B_CD1B_8AFE,
    };

    /// log₂(e) higher significand - used when RC ∈ {Up, NearestEven}.
    pub const LOG2_E_UP: Fp80 = Fp80 {
        sign_exponent: 0x3FFF,
        significand: 0xB8AA_3B29_5C17_F0BC,
    };

    /// log₂(e) lower significand - used when RC ∈ {Down, Zero}.
    pub const LOG2_E_DOWN: Fp80 = Fp80 {
        sign_exponent: 0x3FFF,
        significand: 0xB8AA_3B29_5C17_F0BB,
    };

    /// π higher significand - used when RC ∈ {Up, NearestEven}.
    pub const PI_UP: Fp80 = Fp80 {
        sign_exponent: 0x4000,
        significand: 0xC90F_DAA2_2168_C235,
    };

    /// π lower significand - used when RC ∈ {Down, Zero}.
    pub const PI_DOWN: Fp80 = Fp80 {
        sign_exponent: 0x4000,
        significand: 0xC90F_DAA2_2168_C234,
    };

    /// log₁₀(2) higher significand - used when RC ∈ {Up, NearestEven}.
    pub const LOG10_2_UP: Fp80 = Fp80 {
        sign_exponent: 0x3FFD,
        significand: 0x9A20_9A84_FBCF_F799,
    };

    /// log₁₀(2) lower significand - used when RC ∈ {Down, Zero}.
    pub const LOG10_2_DOWN: Fp80 = Fp80 {
        sign_exponent: 0x3FFD,
        significand: 0x9A20_9A84_FBCF_F798,
    };

    /// ln(2) higher significand - used when RC ∈ {Up, NearestEven}.
    pub const LN_2_UP: Fp80 = Fp80 {
        sign_exponent: 0x3FFE,
        significand: 0xB172_17F7_D1CF_79AC,
    };

    /// ln(2) lower significand - used when RC ∈ {Down, Zero}.
    pub const LN_2_DOWN: Fp80 = Fp80 {
        sign_exponent: 0x3FFE,
        significand: 0xB172_17F7_D1CF_79AB,
    };
}

impl Fp80 {
    /// Construct from raw bit fields.
    pub const fn from_bits(sign_exponent: u16, significand: u64) -> Fp80 {
        Fp80 {
            significand,
            sign_exponent,
        }
    }

    /// Load from 10-byte little-endian memory representation
    /// (8 bytes significand + 2 bytes sign/exponent).
    pub const fn from_le_bytes(bytes: [u8; 10]) -> Fp80 {
        let significand = u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]);
        let sign_exponent = u16::from_le_bytes([bytes[8], bytes[9]]);
        Fp80 {
            significand,
            sign_exponent,
        }
    }

    /// Store as 10-byte little-endian memory representation.
    pub const fn to_le_bytes(self) -> [u8; 10] {
        let sig = self.significand.to_le_bytes();
        let se = self.sign_exponent.to_le_bytes();
        [
            sig[0], sig[1], sig[2], sig[3], sig[4], sig[5], sig[6], sig[7], se[0], se[1],
        ]
    }
}

impl Fp80 {
    /// The sign of the floating point. `true` if negative.
    pub const fn sign(self) -> bool {
        self.sign_exponent & 0x8000 != 0
    }

    /// The biased exponent (0x0000–0x7FFF).
    pub const fn exponent(self) -> u16 {
        self.sign_exponent & 0x7FFF
    }

    /// The full 64-bit significand.
    pub const fn significand(self) -> u64 {
        self.significand
    }

    /// The explicit integer bit (bit 63 of the significand).
    pub const fn j_bit(self) -> bool {
        self.significand & (1 << 63) != 0
    }

    /// The fractional part (bits 62..0 of the significand).
    pub const fn fraction(self) -> u64 {
        self.significand & 0x7FFF_FFFF_FFFF_FFFF
    }

    /// `true` if the value is +0 or −0.
    pub const fn is_zero(self) -> bool {
        self.exponent() == 0 && self.significand == 0
    }

    /// `true` if the value is a denormal (exponent=0, J=0, fraction≠0).
    pub const fn is_denormal(self) -> bool {
        self.exponent() == 0 && !self.j_bit() && self.fraction() != 0
    }

    /// `true` if the value is a pseudo-denormal (exponent=0, J=1).
    pub const fn is_pseudo_denormal(self) -> bool {
        self.exponent() == 0 && self.j_bit()
    }

    /// `true` if the value is a normalized finite number (exponent ∈ [1, 0x7FFE], J=1).
    pub const fn is_normal(self) -> bool {
        let exp = self.exponent();
        exp >= 0x0001 && exp <= 0x7FFE && self.j_bit()
    }

    /// `true` if the value is an unnormal (exponent ∈ [1, 0x7FFE], J=0). Unsupported on 387 and 486 FPU.
    pub const fn is_unnormal(self) -> bool {
        let exp = self.exponent();
        exp >= 0x0001 && exp <= 0x7FFE && !self.j_bit()
    }

    /// `true` if the value is +∞ or −∞.
    pub const fn is_infinity(self) -> bool {
        self.exponent() == 0x7FFF && self.significand == 0x8000_0000_0000_0000
    }

    /// `true` if the value is any NaN (quiet or signaling).
    pub const fn is_nan(self) -> bool {
        self.exponent() == 0x7FFF && (self.significand & 0x7FFF_FFFF_FFFF_FFFF) != 0
    }

    /// `true` if the value is a signaling NaN (exponent=0x7FFF, J=1, bit62=0, fraction≠0).
    pub const fn is_signaling_nan(self) -> bool {
        self.exponent() == 0x7FFF
            && self.j_bit()
            && (self.significand & (1 << 62)) == 0
            && self.fraction() != 0
    }

    /// `true` if the value is a quiet NaN (exponent=0x7FFF, J=1, bit62=1).
    pub const fn is_quiet_nan(self) -> bool {
        self.exponent() == 0x7FFF && self.j_bit() && (self.significand & (1 << 62)) != 0
    }

    /// `true` if the value is in an unsupported format (unnormal, pseudo-infinity, or pseudo-NaN).
    pub const fn is_unsupported(self) -> bool {
        if self.is_unnormal() {
            return true;
        }
        // pseudo-infinity: exponent=0x7FFF, J=0, fraction=0
        if self.exponent() == 0x7FFF && !self.j_bit() && self.fraction() == 0 {
            return true;
        }
        // pseudo-NaN: exponent=0x7FFF, J=0, fraction≠0
        if self.exponent() == 0x7FFF && !self.j_bit() && self.fraction() != 0 {
            return true;
        }
        false
    }

    /// `true` if the sign bit is set.
    pub const fn is_negative(self) -> bool {
        self.sign()
    }

    /// Classify the value for FXAM. Does not distinguish Empty (that is a tag word property).
    pub const fn classify(self) -> FpClass {
        let exp = self.exponent();

        if exp == 0x7FFF {
            if self.significand == 0x8000_0000_0000_0000 {
                return FpClass::Infinity;
            }
            if !self.j_bit() {
                // pseudo-infinity (fraction=0) or pseudo-NaN (fraction≠0)
                return FpClass::Unsupported;
            }
            // J=1: quiet or signaling NaN
            return FpClass::Nan;
        }

        if exp >= 0x0001 {
            // exp ∈ [1, 0x7FFE]
            if self.j_bit() {
                return FpClass::Normal;
            }
            return FpClass::Unsupported; // unnormal
        }

        // exp == 0
        if self.significand == 0 {
            return FpClass::Zero;
        }
        FpClass::Denormal // denormal or pseudo-denormal
    }
}

impl Fp80 {
    /// Flip sign bit (FCHS). Works on any value including NaN and zero.
    pub const fn negate(self) -> Fp80 {
        Fp80 {
            sign_exponent: self.sign_exponent ^ 0x8000,
            significand: self.significand,
        }
    }

    /// Clear sign bit (FABS). Works on any value including NaN and zero.
    pub const fn abs(self) -> Fp80 {
        Fp80 {
            sign_exponent: self.sign_exponent & 0x7FFF,
            significand: self.significand,
        }
    }
}

impl Fp80 {
    /// Convert SNaN to QNaN by setting bit 62 of the significand. No-op on QNaN or non-NaN.
    pub const fn quieten(self) -> Fp80 {
        Fp80 {
            sign_exponent: self.sign_exponent,
            significand: self.significand | (1 << 62),
        }
    }

    /// Apply x86 NaN propagation rules. Sets `ef.invalid` if either operand is SNaN.
    pub fn propagate_nan(a: Fp80, b: Fp80, ef: &mut ExceptionFlags) -> Fp80 {
        let a_is_snan = a.is_signaling_nan();
        let b_is_snan = b.is_signaling_nan();
        let a_is_qnan = a.is_quiet_nan();
        let b_is_qnan = b.is_quiet_nan();

        if a_is_snan || b_is_snan {
            ef.invalid = true;
        }

        let a_nan = a_is_snan || a_is_qnan;
        let b_nan = b_is_snan || b_is_qnan;

        match (a_nan, b_nan, a_is_snan, b_is_snan) {
            // Both are SNaN: quieten both, return the one with larger significand magnitude.
            (_, _, true, true) => {
                let qa = a.quieten();
                let qb = b.quieten();
                if qa.significand >= qb.significand {
                    qa
                } else {
                    qb
                }
            }
            // SNaN + QNaN: return the QNaN (discard SNaN payload).
            (_, _, true, false) if b_is_qnan => b,
            (_, _, false, true) if a_is_qnan => a,
            // SNaN + non-NaN: quieten the SNaN.
            (_, _, true, false) => a.quieten(),
            (_, _, false, true) => b.quieten(),
            // Two QNaN: return the one with larger significand. Tie-break: positive sign, then b.
            (true, true, false, false) => {
                if a.significand > b.significand {
                    a
                } else if b.significand > a.significand {
                    b
                } else if !a.sign() && b.sign() {
                    a
                } else {
                    b
                }
            }
            // One QNaN, one non-NaN.
            (true, false, false, false) => a,
            (false, true, false, false) => b,
            // Neither is NaN - should not be called, but return indefinite as fallback.
            _ => Fp80::INDEFINITE,
        }
    }
}

const BIAS: i32 = 16383;

impl Fp80 {
    /// True exponent (unbiased). For denormals, returns -16382 (the effective exponent).
    pub(crate) fn true_exponent(self) -> i32 {
        let biased = self.exponent() as i32;
        if biased == 0 {
            1 - BIAS // denormals: effective exponent is -16382
        } else {
            biased - BIAS
        }
    }

    /// Normalize a denormal/pseudo-denormal by shifting the significand left until J-bit is set.
    /// Returns (normalized significand, shift count). If significand is zero, returns (0, 0).
    pub(crate) fn normalize_significand(significand: u64) -> (u64, u32) {
        if significand == 0 {
            return (0, 0);
        }
        let shift = significand.leading_zeros();
        (significand << shift, shift)
    }

    /// Round and pack a result into Fp80.
    ///
    /// - `sign`: result sign
    /// - `exponent`: unbiased exponent (maybe out of range)
    /// - `significand`: 128-bit significand with J-bit at bit 127 and extra precision in lower bits
    /// - `rc`: rounding mode
    /// - `pc`: precision control
    /// - `ef`: exception flags accumulator
    pub(crate) fn round_and_pack(
        sign: bool,
        exponent: i32,
        significand: u128,
        rc: RoundingMode,
        pc: Precision,
        ef: &mut ExceptionFlags,
    ) -> Fp80 {
        let precision_bits: u32 = match pc {
            Precision::Single => 24,
            Precision::Double => 53,
            Precision::Extended => 64,
        };

        // The significand has the J-bit at bit 127. We need to round to `precision_bits` bits.
        // Bits to discard from the 128-bit significand: 128 - precision_bits
        let discard_bits = 128 - precision_bits;

        // Extract the kept bits, guard bit, round bit, and sticky bits
        let round_mask = if discard_bits < 128 {
            (1u128 << discard_bits) - 1
        } else {
            u128::MAX
        };
        let discarded = significand & round_mask;
        let halfway = 1u128 << (discard_bits - 1);

        let mut result_sig = significand >> discard_bits;

        let inexact = discarded != 0;
        if inexact {
            ef.precision = true;

            let round_up = match rc {
                RoundingMode::NearestEven => {
                    if discarded > halfway {
                        true
                    } else if discarded == halfway {
                        // Tie: round to even (round up if LSB is odd)
                        result_sig & 1 != 0
                    } else {
                        false
                    }
                }
                RoundingMode::Up => !sign,
                RoundingMode::Down => sign,
                RoundingMode::Zero => false,
            };

            if round_up {
                result_sig += 1;
                // Check for carry out of the precision bits
                if result_sig >= (1u128 << precision_bits) {
                    result_sig >>= 1;
                    exponent_add_one_overflow(sign, exponent + 1, result_sig as u64, ef)
                } else {
                    Self::pack_result(sign, exponent, result_sig as u64, precision_bits, ef)
                }
            } else {
                Self::pack_result(sign, exponent, result_sig as u64, precision_bits, ef)
            }
        } else {
            Self::pack_result(sign, exponent, result_sig as u64, precision_bits, ef)
        }
    }

    fn pack_result(
        sign: bool,
        exponent: i32,
        significand: u64,
        precision_bits: u32,
        ef: &mut ExceptionFlags,
    ) -> Fp80 {
        // Place the significand bits at the top of the 64-bit field
        let shift_up = 64 - precision_bits;
        let sig64 = significand << shift_up;

        let biased = exponent + BIAS;

        if biased >= 0x7FFF {
            // Overflow
            ef.overflow = true;
            ef.precision = true;
            return if sign {
                Fp80::NEG_INFINITY
            } else {
                Fp80::INFINITY
            };
        }

        if biased <= 0 {
            // Underflow: denormalize
            let shift_right = (1 - biased) as u32;
            if shift_right >= 64 || sig64 == 0 {
                ef.underflow = true;
                if sig64 != 0 {
                    ef.precision = true;
                }
                return if sign { Fp80::NEG_ZERO } else { Fp80::ZERO };
            }
            let denorm_sig = sig64 >> shift_right;
            if denorm_sig << shift_right != sig64 {
                ef.underflow = true;
                ef.precision = true;
            } else if denorm_sig != sig64 {
                ef.underflow = true;
            }
            let se = if sign { 0x8000u16 } else { 0x0000u16 };
            return Fp80::from_bits(se, denorm_sig);
        }

        let se = if sign {
            0x8000 | biased as u16
        } else {
            biased as u16
        };
        Fp80::from_bits(se, sig64)
    }
}

fn exponent_add_one_overflow(
    sign: bool,
    exponent: i32,
    significand: u64,
    ef: &mut ExceptionFlags,
) -> Fp80 {
    Fp80::pack_result(sign, exponent, significand, 64, ef)
}

#[cfg(test)]
mod tests {
    use super::*;

    const POSITIVE_DENORMAL: Fp80 = Fp80::from_bits(0x0000, 0x0000_0000_0000_0001);
    const NEGATIVE_DENORMAL: Fp80 = Fp80::from_bits(0x8000, 0x0000_0000_0000_0001);
    const MAX_DENORMAL: Fp80 = Fp80::from_bits(0x0000, 0x3FFF_FFFF_FFFF_FFFF);
    const POSITIVE_PSEUDO_DENORMAL: Fp80 = Fp80::from_bits(0x0000, 0x8000_0000_0000_0000);
    const NEGATIVE_PSEUDO_DENORMAL: Fp80 = Fp80::from_bits(0x8000, 0x8000_0000_0000_0000);
    const SMALLEST_NORMAL: Fp80 = Fp80::from_bits(0x0001, 0x8000_0000_0000_0000);
    const LARGEST_NORMAL: Fp80 = Fp80::from_bits(0x7FFE, 0xFFFF_FFFF_FFFF_FFFF);
    const UNNORMAL: Fp80 = Fp80::from_bits(0x0001, 0x0000_0000_0000_0001);
    const UNNORMAL_MAX_EXP: Fp80 = Fp80::from_bits(0x7FFE, 0x0000_0000_0000_0001);
    const PSEUDO_INFINITY: Fp80 = Fp80::from_bits(0x7FFF, 0x0000_0000_0000_0000);
    const NEGATIVE_PSEUDO_INFINITY: Fp80 = Fp80::from_bits(0xFFFF, 0x0000_0000_0000_0000);
    const POSITIVE_SNAN: Fp80 = Fp80::from_bits(0x7FFF, 0x8000_0000_0000_0001);
    const NEGATIVE_SNAN: Fp80 = Fp80::from_bits(0xFFFF, 0x8000_0000_0000_0001);
    const MAX_SNAN: Fp80 = Fp80::from_bits(0x7FFF, 0xBFFF_FFFF_FFFF_FFFF);
    const POSITIVE_QNAN: Fp80 = Fp80::from_bits(0x7FFF, 0xC000_0000_0000_0001);
    const PSEUDO_NAN: Fp80 = Fp80::from_bits(0x7FFF, 0x0000_0000_0000_0001);
    const NEGATIVE_ONE: Fp80 = Fp80::from_bits(0xBFFF, 0x8000_0000_0000_0000);

    fn no_exceptions() -> ExceptionFlags {
        ExceptionFlags::default()
    }

    #[test]
    fn test_from_bits_basic() {
        let v = Fp80::from_bits(0x4000, 0xA000_0000_0000_0000);
        assert_eq!(v.sign_exponent, 0x4000);
        assert_eq!(v.significand, 0xA000_0000_0000_0000);

        assert_eq!(Fp80::from_bits(0x0000, 0x0000_0000_0000_0000), Fp80::ZERO);
        assert_eq!(Fp80::from_bits(0x3FFF, 0x8000_0000_0000_0000), Fp80::ONE);
    }

    #[test]
    fn test_from_bits_edge_cases() {
        assert_eq!(
            Fp80::from_bits(0x8000, 0x0000_0000_0000_0000),
            Fp80::NEG_ZERO
        );
        assert_eq!(
            Fp80::from_bits(0x7FFF, 0x8000_0000_0000_0000),
            Fp80::INFINITY
        );
        assert_eq!(
            Fp80::from_bits(0xFFFF, 0x8000_0000_0000_0000),
            Fp80::NEG_INFINITY
        );
        assert_eq!(
            Fp80::from_bits(0xFFFF, 0xC000_0000_0000_0000),
            Fp80::INDEFINITE
        );

        let max = Fp80::from_bits(0xFFFF, 0xFFFF_FFFF_FFFF_FFFF);
        assert_eq!(max.sign_exponent, 0xFFFF);
        assert_eq!(max.significand, 0xFFFF_FFFF_FFFF_FFFF);
    }

    #[test]
    fn test_serialization_basic() {
        // Round-trip for ONE.
        let bytes = Fp80::ONE.to_le_bytes();
        assert_eq!(Fp80::from_le_bytes(bytes), Fp80::ONE);

        // ZERO is all zeros.
        assert_eq!(Fp80::ZERO.to_le_bytes(), [0u8; 10]);
        assert_eq!(Fp80::from_le_bytes([0u8; 10]), Fp80::ZERO);

        // ONE: sig=0x8000000000000000 LE, se=0x3FFF LE.
        assert_eq!(
            Fp80::ONE.to_le_bytes(),
            [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0xFF, 0x3F]
        );
    }

    #[test]
    fn test_serialization_edge_cases() {
        // Round-trip special values.
        for v in [
            Fp80::INFINITY,
            Fp80::NEG_INFINITY,
            Fp80::INDEFINITE,
            Fp80::NEG_ZERO,
        ] {
            assert_eq!(Fp80::from_le_bytes(v.to_le_bytes()), v);
        }

        // NEG_ZERO: first 8 bytes zero, last 2 = 0x8000 LE.
        assert_eq!(
            Fp80::NEG_ZERO.to_le_bytes(),
            [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80]
        );

        // INDEFINITE: sig=0xC000000000000000 LE, se=0xFFFF LE.
        assert_eq!(
            Fp80::INDEFINITE.to_le_bytes(),
            [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0xFF, 0xFF]
        );

        // All-ones byte pattern.
        let all_ones = Fp80::from_le_bytes([0xFF; 10]);
        assert_eq!(all_ones.sign_exponent, 0xFFFF);
        assert_eq!(all_ones.significand, 0xFFFF_FFFF_FFFF_FFFF);
    }

    #[test]
    fn test_accessors_basic() {
        // sign
        assert!(!Fp80::ZERO.sign());
        assert!(Fp80::NEG_ZERO.sign());
        assert!(!Fp80::ONE.sign());

        // exponent
        assert_eq!(Fp80::ZERO.exponent(), 0);
        assert_eq!(Fp80::ONE.exponent(), 0x3FFF);
        assert_eq!(Fp80::INFINITY.exponent(), 0x7FFF);

        // significand
        assert_eq!(Fp80::ZERO.significand(), 0);
        assert_eq!(Fp80::ONE.significand(), 0x8000_0000_0000_0000);

        // j_bit
        assert!(!Fp80::ZERO.j_bit());
        assert!(Fp80::ONE.j_bit());
        assert!(Fp80::INFINITY.j_bit());

        // fraction
        assert_eq!(Fp80::ZERO.fraction(), 0);
        assert_eq!(Fp80::ONE.fraction(), 0); // significand is exactly 0x8000..., J-bit only
        assert_eq!(Fp80::INFINITY.fraction(), 0);
    }

    #[test]
    fn test_accessors_edge_cases() {
        // sign: NEG_INFINITY has sign_exponent 0xFFFF, sign should be true.
        assert!(Fp80::NEG_INFINITY.sign());
        // INDEFINITE is negative (sign_exponent 0xFFFF).
        assert!(Fp80::INDEFINITE.sign());
        // Positive SNaN.
        assert!(!POSITIVE_SNAN.sign());

        // exponent masks out sign bit.
        assert_eq!(Fp80::NEG_ZERO.exponent(), 0);
        assert_eq!(Fp80::NEG_INFINITY.exponent(), 0x7FFF);
        assert_eq!(SMALLEST_NORMAL.exponent(), 1);
        assert_eq!(LARGEST_NORMAL.exponent(), 0x7FFE);

        // significand returns full 64 bits.
        assert_eq!(Fp80::INDEFINITE.significand(), 0xC000_0000_0000_0000);
        assert_eq!(POSITIVE_DENORMAL.significand(), 1);

        // j_bit for various classes.
        assert!(!POSITIVE_DENORMAL.j_bit()); // denormal: J=0
        assert!(POSITIVE_PSEUDO_DENORMAL.j_bit()); // pseudo-denormal: J=1
        assert!(!UNNORMAL.j_bit()); // unnormal: J=0

        // fraction for INDEFINITE: sig=0xC000..., masked = 0x4000...
        assert_eq!(Fp80::INDEFINITE.fraction(), 0x4000_0000_0000_0000);
        // fraction for max significand.
        assert_eq!(
            Fp80::from_bits(0x3FFF, 0xFFFF_FFFF_FFFF_FFFF).fraction(),
            0x7FFF_FFFF_FFFF_FFFF
        );
    }

    #[test]
    fn test_is_zero_basic() {
        assert!(Fp80::ZERO.is_zero());
        assert!(Fp80::NEG_ZERO.is_zero());
        assert!(!Fp80::ONE.is_zero());
    }

    #[test]
    fn test_is_zero_edge_cases() {
        assert!(!POSITIVE_DENORMAL.is_zero()); // exp=0, sig!=0
        assert!(!POSITIVE_PSEUDO_DENORMAL.is_zero()); // exp=0, sig=0x8000...
        assert!(!Fp80::INFINITY.is_zero());
        assert!(!Fp80::INDEFINITE.is_zero());
    }

    #[test]
    fn test_is_denormal_basic() {
        assert!(POSITIVE_DENORMAL.is_denormal());
        assert!(MAX_DENORMAL.is_denormal());
        assert!(!Fp80::ZERO.is_denormal());
        assert!(!Fp80::ONE.is_denormal());
    }

    #[test]
    fn test_is_denormal_edge_cases() {
        assert!(NEGATIVE_DENORMAL.is_denormal());
        assert!(!POSITIVE_PSEUDO_DENORMAL.is_denormal()); // J=1 -> not denormal
        assert!(!Fp80::NEG_ZERO.is_denormal()); // sig=0
        assert!(!UNNORMAL.is_denormal()); // exp!=0
    }

    #[test]
    fn test_is_pseudo_denormal_basic() {
        assert!(POSITIVE_PSEUDO_DENORMAL.is_pseudo_denormal());
        // Pseudo-denormal with nonzero fraction.
        assert!(Fp80::from_bits(0x0000, 0x8000_0000_0000_0001).is_pseudo_denormal());
        assert!(!Fp80::ZERO.is_pseudo_denormal());
    }

    #[test]
    fn test_is_pseudo_denormal_edge_cases() {
        assert!(NEGATIVE_PSEUDO_DENORMAL.is_pseudo_denormal());
        assert!(!POSITIVE_DENORMAL.is_pseudo_denormal()); // J=0
        assert!(!Fp80::ONE.is_pseudo_denormal()); // exp!=0
    }

    #[test]
    fn test_is_normal_basic() {
        assert!(Fp80::ONE.is_normal());
        assert!(SMALLEST_NORMAL.is_normal());
        assert!(LARGEST_NORMAL.is_normal());
    }

    #[test]
    fn test_is_normal_edge_cases() {
        assert!(NEGATIVE_ONE.is_normal());
        assert!(!Fp80::ZERO.is_normal());
        assert!(!Fp80::INFINITY.is_normal()); // exp=0x7FFF
        assert!(!UNNORMAL.is_normal()); // J=0
        assert!(!POSITIVE_DENORMAL.is_normal()); // exp=0
    }

    #[test]
    fn test_is_unnormal_basic() {
        assert!(UNNORMAL.is_unnormal());
        assert!(UNNORMAL_MAX_EXP.is_unnormal());
        assert!(!Fp80::ONE.is_unnormal());
    }

    #[test]
    fn test_is_unnormal_edge_cases() {
        // exp=0, J=0, frac!=0 is denormal, not unnormal.
        assert!(!POSITIVE_DENORMAL.is_unnormal());
        // exp=0x7FFF, J=0, frac=0 is pseudo-infinity, not unnormal.
        assert!(!PSEUDO_INFINITY.is_unnormal());
        // Unnormal with J=0, zero fraction is still unnormal.
        assert!(Fp80::from_bits(0x4000, 0x0000_0000_0000_0000).is_unnormal());
    }

    #[test]
    fn test_is_infinity_basic() {
        assert!(Fp80::INFINITY.is_infinity());
        assert!(Fp80::NEG_INFINITY.is_infinity());
        assert!(!Fp80::ONE.is_infinity());
    }

    #[test]
    fn test_is_infinity_edge_cases() {
        assert!(!PSEUDO_INFINITY.is_infinity()); // J=0
        assert!(!Fp80::INDEFINITE.is_infinity()); // QNaN
        assert!(!Fp80::ZERO.is_infinity());
    }

    #[test]
    fn test_is_nan_basic() {
        assert!(Fp80::INDEFINITE.is_nan()); // QNaN
        assert!(POSITIVE_SNAN.is_nan()); // SNaN
        assert!(!Fp80::ONE.is_nan());
        assert!(!Fp80::INFINITY.is_nan());
    }

    #[test]
    fn test_is_nan_edge_cases() {
        // Pseudo-NaN (J=0, frac!=0): is_nan uses mask 0x7FFF..., so fraction is nonzero -> true.
        assert!(PSEUDO_NAN.is_nan());
        // Pseudo-infinity (J=0, frac=0): masked significand is 0 -> false.
        assert!(!PSEUDO_INFINITY.is_nan());
        assert!(POSITIVE_QNAN.is_nan());
        assert!(NEGATIVE_SNAN.is_nan());
    }

    #[test]
    fn test_is_signaling_nan_basic() {
        // exp=0x7FFF, J=1, bit62=0, frac!=0 -> SNaN.
        assert!(POSITIVE_SNAN.is_signaling_nan());
        assert!(!Fp80::INDEFINITE.is_signaling_nan()); // QNaN
        assert!(!Fp80::ONE.is_signaling_nan());
    }

    #[test]
    fn test_is_signaling_nan_edge_cases() {
        assert!(NEGATIVE_SNAN.is_signaling_nan());
        assert!(MAX_SNAN.is_signaling_nan()); // J=1, bit62=0, max fraction
        // Infinity has frac=0, so not SNaN.
        assert!(!Fp80::INFINITY.is_signaling_nan());
        // Pseudo-NaN (J=0): fails J-bit check.
        assert!(!PSEUDO_NAN.is_signaling_nan());
    }

    #[test]
    fn test_is_quiet_nan_basic() {
        assert!(Fp80::INDEFINITE.is_quiet_nan());
        assert!(POSITIVE_QNAN.is_quiet_nan());
        assert!(!POSITIVE_SNAN.is_quiet_nan());
    }

    #[test]
    fn test_is_quiet_nan_edge_cases() {
        // Positive indefinite-like (no sign): exp=0x7FFF, sig=0xC000...
        let positive_indef = Fp80::from_bits(0x7FFF, 0xC000_0000_0000_0000);
        assert!(positive_indef.is_quiet_nan());
        // Pseudo-NaN with bit62 set but J=0: fails J-bit check.
        assert!(!Fp80::from_bits(0x7FFF, 0x4000_0000_0000_0001).is_quiet_nan());
        assert!(!Fp80::INFINITY.is_quiet_nan());
    }

    #[test]
    fn test_is_unsupported_basic() {
        assert!(UNNORMAL.is_unsupported());
        assert!(PSEUDO_INFINITY.is_unsupported());
        assert!(PSEUDO_NAN.is_unsupported());
    }

    #[test]
    fn test_is_unsupported_edge_cases() {
        assert!(NEGATIVE_PSEUDO_INFINITY.is_unsupported());
        assert!(UNNORMAL_MAX_EXP.is_unsupported());
        // Supported formats.
        assert!(!Fp80::ONE.is_unsupported());
        assert!(!Fp80::ZERO.is_unsupported());
        assert!(!Fp80::INFINITY.is_unsupported());
        assert!(!Fp80::INDEFINITE.is_unsupported()); // QNaN is supported
        assert!(!POSITIVE_SNAN.is_unsupported()); // SNaN is supported
        assert!(!POSITIVE_DENORMAL.is_unsupported());
    }

    #[test]
    fn test_is_negative_basic() {
        assert!(Fp80::NEG_ZERO.is_negative());
        assert!(Fp80::NEG_INFINITY.is_negative());
        assert!(!Fp80::ZERO.is_negative());
        assert!(!Fp80::ONE.is_negative());
    }

    #[test]
    fn test_is_negative_edge_cases() {
        assert!(Fp80::INDEFINITE.is_negative()); // sign_exponent 0xFFFF
        assert!(NEGATIVE_SNAN.is_negative());
        assert!(!POSITIVE_SNAN.is_negative());
        assert!(!POSITIVE_QNAN.is_negative());
    }

    #[test]
    fn test_classify_basic() {
        assert_eq!(Fp80::ZERO.classify(), FpClass::Zero);
        assert_eq!(Fp80::ONE.classify(), FpClass::Normal);
        assert_eq!(Fp80::INFINITY.classify(), FpClass::Infinity);
        assert_eq!(Fp80::INDEFINITE.classify(), FpClass::Nan);
        assert_eq!(POSITIVE_DENORMAL.classify(), FpClass::Denormal);
    }

    #[test]
    fn test_classify_edge_cases() {
        assert_eq!(Fp80::NEG_ZERO.classify(), FpClass::Zero);
        assert_eq!(Fp80::NEG_INFINITY.classify(), FpClass::Infinity);
        assert_eq!(NEGATIVE_ONE.classify(), FpClass::Normal);
        assert_eq!(POSITIVE_SNAN.classify(), FpClass::Nan);
        // Pseudo-denormal is classified as Denormal.
        assert_eq!(POSITIVE_PSEUDO_DENORMAL.classify(), FpClass::Denormal);
        // Unsupported formats.
        assert_eq!(UNNORMAL.classify(), FpClass::Unsupported);
        assert_eq!(PSEUDO_INFINITY.classify(), FpClass::Unsupported);
        assert_eq!(PSEUDO_NAN.classify(), FpClass::Unsupported);
    }

    #[test]
    fn test_negate_basic() {
        let neg_one = Fp80::ONE.negate();
        assert!(neg_one.sign());
        assert_eq!(neg_one.exponent(), Fp80::ONE.exponent());
        assert_eq!(neg_one.significand(), Fp80::ONE.significand());

        // Double negate is identity.
        assert_eq!(Fp80::ONE.negate().negate(), Fp80::ONE);
        assert_eq!(Fp80::ZERO.negate(), Fp80::NEG_ZERO);
    }

    #[test]
    fn test_negate_edge_cases() {
        assert_eq!(Fp80::NEG_ZERO.negate(), Fp80::ZERO);
        assert_eq!(Fp80::INFINITY.negate(), Fp80::NEG_INFINITY);
        assert_eq!(Fp80::NEG_INFINITY.negate(), Fp80::INFINITY);

        // NaN negate flips sign, preserves significand.
        let neg_indef = Fp80::INDEFINITE.negate();
        assert_eq!(neg_indef.sign_exponent, 0x7FFF);
        assert_eq!(neg_indef.significand, Fp80::INDEFINITE.significand);
    }

    #[test]
    fn test_abs_basic() {
        assert_eq!(Fp80::ONE.abs(), Fp80::ONE);
        assert_eq!(NEGATIVE_ONE.abs(), Fp80::ONE);
        assert_eq!(Fp80::NEG_ZERO.abs(), Fp80::ZERO);
    }

    #[test]
    fn test_abs_edge_cases() {
        assert_eq!(Fp80::NEG_INFINITY.abs(), Fp80::INFINITY);

        // abs of INDEFINITE: sign_exponent becomes 0x7FFF, significand unchanged.
        let abs_indef = Fp80::INDEFINITE.abs();
        assert_eq!(abs_indef.sign_exponent, 0x7FFF);
        assert_eq!(abs_indef.significand, 0xC000_0000_0000_0000);

        // Already positive: no change.
        assert_eq!(Fp80::INFINITY.abs(), Fp80::INFINITY);
        assert_eq!(POSITIVE_SNAN.abs(), POSITIVE_SNAN);
    }

    #[test]
    fn test_quieten_basic() {
        // SNaN -> QNaN: sets bit 62.
        let q = POSITIVE_SNAN.quieten();
        assert_eq!(q.significand, 0xC000_0000_0000_0001);
        assert_eq!(q.sign_exponent, POSITIVE_SNAN.sign_exponent);
        assert!(q.is_quiet_nan());
    }

    #[test]
    fn test_quieten_edge_cases() {
        // QNaN: bit 62 already set, no-op.
        assert_eq!(Fp80::INDEFINITE.quieten(), Fp80::INDEFINITE);

        // Non-NaN: quieten unconditionally sets bit 62 (it does not check for NaN).
        let q_one = Fp80::ONE.quieten();
        assert_eq!(q_one.significand, 0x8000_0000_0000_0000 | (1 << 62));

        let q_zero = Fp80::ZERO.quieten();
        assert_eq!(q_zero.significand, 1 << 62);
    }

    #[test]
    fn test_propagate_nan_basic() {
        // Case 5: QNaN + non-NaN -> return QNaN, no IE.
        let mut ef = no_exceptions();
        let result = Fp80::propagate_nan(Fp80::INDEFINITE, Fp80::ONE, &mut ef);
        assert_eq!(result, Fp80::INDEFINITE);
        assert_eq!(ef, no_exceptions());

        // Case 5 reversed: non-NaN + QNaN -> return QNaN.
        let mut ef = no_exceptions();
        let result = Fp80::propagate_nan(Fp80::ONE, Fp80::INDEFINITE, &mut ef);
        assert_eq!(result, Fp80::INDEFINITE);
        assert_eq!(ef, no_exceptions());

        // Case 6: neither NaN -> return INDEFINITE (fallback).
        let mut ef = no_exceptions();
        let result = Fp80::propagate_nan(Fp80::ONE, Fp80::ZERO, &mut ef);
        assert_eq!(result, Fp80::INDEFINITE);
    }

    #[test]
    fn test_propagate_nan_edge_cases() {
        // Case 1: SNaN + non-NaN -> quieten SNaN, IE set.
        let mut ef = no_exceptions();
        let result = Fp80::propagate_nan(POSITIVE_SNAN, Fp80::ONE, &mut ef);
        assert_eq!(result, Fp80::from_bits(0x7FFF, 0xC000_0000_0000_0001));
        assert!(ef.invalid);

        // Case 1 reversed: non-NaN + SNaN -> quieten SNaN.
        let mut ef = no_exceptions();
        let result = Fp80::propagate_nan(Fp80::ONE, POSITIVE_SNAN, &mut ef);
        assert_eq!(result, Fp80::from_bits(0x7FFF, 0xC000_0000_0000_0001));
        assert!(ef.invalid);

        // Case 2: SNaN + QNaN -> return QNaN, IE set.
        let mut ef = no_exceptions();
        let result = Fp80::propagate_nan(POSITIVE_SNAN, Fp80::INDEFINITE, &mut ef);
        assert_eq!(result, Fp80::INDEFINITE);
        assert!(ef.invalid);

        // Case 2 reversed: QNaN + SNaN -> return QNaN.
        let mut ef = no_exceptions();
        let result = Fp80::propagate_nan(Fp80::INDEFINITE, POSITIVE_SNAN, &mut ef);
        assert_eq!(result, Fp80::INDEFINITE);
        assert!(ef.invalid);

        // Case 3: two SNaNs -> quieten both, return larger significand, IE set.
        let snan_larger = Fp80::from_bits(0x7FFF, 0x8000_0000_0000_0002);
        let snan_smaller = Fp80::from_bits(0x7FFF, 0x8000_0000_0000_0001);
        let mut ef = no_exceptions();
        let result = Fp80::propagate_nan(snan_larger, snan_smaller, &mut ef);
        // After quieten: 0xC000_0000_0000_0002 >= 0xC000_0000_0000_0001, return a.
        assert_eq!(result, Fp80::from_bits(0x7FFF, 0xC000_0000_0000_0002));
        assert!(ef.invalid);

        // Case 4: two QNaNs -> return larger significand, no IE.
        let qnan_a = Fp80::from_bits(0x7FFF, 0xC000_0000_0000_0002);
        let qnan_b = Fp80::from_bits(0x7FFF, 0xC000_0000_0000_0001);
        let mut ef = no_exceptions();
        let result = Fp80::propagate_nan(qnan_a, qnan_b, &mut ef);
        assert_eq!(result, qnan_a);
        assert_eq!(ef, no_exceptions());

        // Case 4 tie-break: same significand -> prefer positive.
        let qnan_pos = Fp80::from_bits(0x7FFF, 0xC000_0000_0000_0000);
        let qnan_neg = Fp80::from_bits(0xFFFF, 0xC000_0000_0000_0000);
        let mut ef = no_exceptions();
        let result = Fp80::propagate_nan(qnan_pos, qnan_neg, &mut ef);
        assert_eq!(result, qnan_pos);

        // Case 4 tie-break: same significand, same sign -> return b.
        let mut ef = no_exceptions();
        let result = Fp80::propagate_nan(qnan_pos, qnan_pos, &mut ef);
        // Both identical, so !a.sign()&&b.sign() is false (both positive), returns b.
        assert_eq!(result, qnan_pos);

        // Case 3 tie: two SNaN with equal significand -> quieten, return a (>= is true).
        let snan_same = Fp80::from_bits(0x7FFF, 0x8000_0000_0000_0001);
        let mut ef = no_exceptions();
        let result = Fp80::propagate_nan(snan_same, snan_same, &mut ef);
        assert_eq!(result, Fp80::from_bits(0x7FFF, 0xC000_0000_0000_0001));
        assert!(ef.invalid);

        // Case 4 tie-break reversed: same significand -> prefer positive regardless of order.
        let qnan_pos = Fp80::from_bits(0x7FFF, 0xC000_0000_0000_0000);
        let qnan_neg = Fp80::from_bits(0xFFFF, 0xC000_0000_0000_0000);
        let mut ef = no_exceptions();
        let result = Fp80::propagate_nan(qnan_neg, qnan_pos, &mut ef);
        assert_eq!(result, qnan_pos);
        assert_eq!(ef, no_exceptions());
    }

    #[test]
    fn test_constants() {
        // ZERO.
        assert!(!Fp80::ZERO.sign());
        assert_eq!(Fp80::ZERO.exponent(), 0);
        assert_eq!(Fp80::ZERO.significand(), 0);
        assert!(Fp80::ZERO.is_zero());

        // NEG_ZERO.
        assert!(Fp80::NEG_ZERO.sign());
        assert_eq!(Fp80::NEG_ZERO.exponent(), 0);
        assert_eq!(Fp80::NEG_ZERO.significand(), 0);
        assert!(Fp80::NEG_ZERO.is_zero());

        // ONE.
        assert!(!Fp80::ONE.sign());
        assert_eq!(Fp80::ONE.exponent(), 0x3FFF);
        assert_eq!(Fp80::ONE.significand(), 0x8000_0000_0000_0000);
        assert!(Fp80::ONE.is_normal());

        // INFINITY.
        assert!(!Fp80::INFINITY.sign());
        assert_eq!(Fp80::INFINITY.exponent(), 0x7FFF);
        assert_eq!(Fp80::INFINITY.significand(), 0x8000_0000_0000_0000);
        assert!(Fp80::INFINITY.is_infinity());

        // NEG_INFINITY.
        assert!(Fp80::NEG_INFINITY.sign());
        assert!(Fp80::NEG_INFINITY.is_infinity());

        // INDEFINITE.
        assert!(Fp80::INDEFINITE.sign());
        assert_eq!(Fp80::INDEFINITE.exponent(), 0x7FFF);
        assert_eq!(Fp80::INDEFINITE.significand(), 0xC000_0000_0000_0000);
        assert!(Fp80::INDEFINITE.is_quiet_nan());

        // Transcendental constant pairs: all normal, positive.
        let pairs = [
            (Fp80::LOG2_10_UP, Fp80::LOG2_10_DOWN),
            (Fp80::LOG2_E_UP, Fp80::LOG2_E_DOWN),
            (Fp80::PI_UP, Fp80::PI_DOWN),
            (Fp80::LOG10_2_UP, Fp80::LOG10_2_DOWN),
            (Fp80::LN_2_UP, Fp80::LN_2_DOWN),
        ];
        for (up, down) in pairs {
            assert!(up.is_normal());
            assert!(down.is_normal());
            assert!(!up.sign());
            assert!(!down.sign());
            assert_eq!(up.exponent(), down.exponent());
            // UP significand is exactly 1 more than DOWN.
            assert_eq!(up.significand(), down.significand() + 1);
        }
    }
}
