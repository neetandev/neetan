//! Floating-point comparison operations.

use crate::{ExceptionFlags, Fp80, FpOrdering, flag_unsupported_operands};

impl Fp80 {
    /// Ordered comparison (FCOM). Raises IE on any NaN (quiet or signaling) and on an
    /// unsupported encoding.
    pub fn compare(self, other: Fp80, ef: &mut ExceptionFlags) -> FpOrdering {
        if flag_unsupported_operands(&[self, other], ef) {
            return FpOrdering::Unordered;
        }
        if self.is_nan() || other.is_nan() {
            ef.invalid = true;
            return FpOrdering::Unordered;
        }
        if self.is_denormal()
            || self.is_pseudo_denormal()
            || other.is_denormal()
            || other.is_pseudo_denormal()
        {
            ef.denormal = true;
        }
        self.compare_values(other)
    }

    /// Unordered comparison (FUCOM). Raises IE on a signaling NaN and on an unsupported
    /// encoding, not on a quiet NaN.
    pub fn compare_quiet(self, other: Fp80, ef: &mut ExceptionFlags) -> FpOrdering {
        if flag_unsupported_operands(&[self, other], ef) {
            return FpOrdering::Unordered;
        }
        if self.is_signaling_nan() || other.is_signaling_nan() {
            ef.invalid = true;
        }
        if self.is_nan() || other.is_nan() {
            return FpOrdering::Unordered;
        }
        if self.is_denormal()
            || self.is_pseudo_denormal()
            || other.is_denormal()
            || other.is_pseudo_denormal()
        {
            ef.denormal = true;
        }
        self.compare_values(other)
    }

    fn compare_values(self, other: Fp80) -> FpOrdering {
        if self.is_zero() && other.is_zero() {
            return FpOrdering::Equal;
        }

        let a_sign = self.sign();
        let b_sign = other.sign();

        if a_sign != b_sign {
            return if a_sign {
                FpOrdering::Less
            } else {
                FpOrdering::Greater
            };
        }

        let order = match (
            self.exponent().cmp(&other.exponent()),
            self.significand().cmp(&other.significand()),
        ) {
            (core::cmp::Ordering::Greater, _) => FpOrdering::Greater,
            (core::cmp::Ordering::Less, _) => FpOrdering::Less,
            (core::cmp::Ordering::Equal, core::cmp::Ordering::Greater) => FpOrdering::Greater,
            (core::cmp::Ordering::Equal, core::cmp::Ordering::Less) => FpOrdering::Less,
            (core::cmp::Ordering::Equal, core::cmp::Ordering::Equal) => FpOrdering::Equal,
        };

        if a_sign {
            match order {
                FpOrdering::Greater => FpOrdering::Less,
                FpOrdering::Less => FpOrdering::Greater,
                other => other,
            }
        } else {
            order
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEGATIVE_ONE: Fp80 = Fp80::from_bits(0xBFFF, 0x8000_0000_0000_0000);
    const TWO: Fp80 = Fp80::from_bits(0x4000, 0x8000_0000_0000_0000);
    const POSITIVE_SNAN: Fp80 = Fp80::from_bits(0x7FFF, 0x8000_0000_0000_0001);
    const POSITIVE_QNAN: Fp80 = Fp80::from_bits(0x7FFF, 0xC000_0000_0000_0001);
    const POSITIVE_DENORMAL: Fp80 = Fp80::from_bits(0x0000, 0x0000_0000_0000_0001);

    fn no_exceptions() -> ExceptionFlags {
        ExceptionFlags::default()
    }

    #[test]
    fn test_compare_basic() {
        let mut ef = no_exceptions();
        assert_eq!(Fp80::ONE.compare(Fp80::ZERO, &mut ef), FpOrdering::Greater);
        assert_eq!(ef, no_exceptions());

        let mut ef = no_exceptions();
        assert_eq!(Fp80::ZERO.compare(Fp80::ONE, &mut ef), FpOrdering::Less);

        let mut ef = no_exceptions();
        assert_eq!(Fp80::ONE.compare(Fp80::ONE, &mut ef), FpOrdering::Equal);

        // +0 == -0.
        let mut ef = no_exceptions();
        assert_eq!(
            Fp80::ZERO.compare(Fp80::NEG_ZERO, &mut ef),
            FpOrdering::Equal
        );
    }

    #[test]
    fn test_compare_edge_cases() {
        // QNaN -> Unordered + IE (ordered comparison raises IE on ANY NaN).
        let mut ef = no_exceptions();
        assert_eq!(
            POSITIVE_QNAN.compare(Fp80::ONE, &mut ef),
            FpOrdering::Unordered
        );
        assert!(ef.invalid);

        let mut ef = no_exceptions();
        assert_eq!(
            Fp80::ONE.compare(POSITIVE_QNAN, &mut ef),
            FpOrdering::Unordered
        );
        assert!(ef.invalid);

        // SNaN -> Unordered + IE.
        let mut ef = no_exceptions();
        assert_eq!(
            POSITIVE_SNAN.compare(Fp80::ONE, &mut ef),
            FpOrdering::Unordered
        );
        assert!(ef.invalid);

        // Infinity comparisons.
        let mut ef = no_exceptions();
        assert_eq!(
            Fp80::INFINITY.compare(Fp80::ONE, &mut ef),
            FpOrdering::Greater
        );

        let mut ef = no_exceptions();
        assert_eq!(
            Fp80::NEG_INFINITY.compare(Fp80::ONE, &mut ef),
            FpOrdering::Less
        );

        let mut ef = no_exceptions();
        assert_eq!(
            Fp80::INFINITY.compare(Fp80::INFINITY, &mut ef),
            FpOrdering::Equal
        );

        let mut ef = no_exceptions();
        assert_eq!(
            Fp80::NEG_INFINITY.compare(Fp80::INFINITY, &mut ef),
            FpOrdering::Less
        );

        // Negative values.
        let mut ef = no_exceptions();
        assert_eq!(NEGATIVE_ONE.compare(Fp80::ONE, &mut ef), FpOrdering::Less);

        // Denormal comparison.
        let mut ef = no_exceptions();
        assert_eq!(
            POSITIVE_DENORMAL.compare(Fp80::ZERO, &mut ef),
            FpOrdering::Greater
        );

        let mut ef = no_exceptions();
        assert_eq!(
            POSITIVE_DENORMAL.compare(Fp80::ONE, &mut ef),
            FpOrdering::Less
        );

        // Two QNaNs -> Unordered + IE (ordered comparison raises IE on ANY NaN).
        let mut ef = no_exceptions();
        assert_eq!(
            POSITIVE_QNAN.compare(Fp80::INDEFINITE, &mut ef),
            FpOrdering::Unordered
        );
        assert!(ef.invalid);

        // Two SNaNs -> Unordered + IE.
        let mut ef = no_exceptions();
        assert_eq!(
            POSITIVE_SNAN.compare(POSITIVE_SNAN, &mut ef),
            FpOrdering::Unordered
        );
        assert!(ef.invalid);

        // QNaN + SNaN -> Unordered + IE.
        let mut ef = no_exceptions();
        assert_eq!(
            POSITIVE_QNAN.compare(POSITIVE_SNAN, &mut ef),
            FpOrdering::Unordered
        );
        assert!(ef.invalid);

        // -inf == -inf.
        let mut ef = no_exceptions();
        assert_eq!(
            Fp80::NEG_INFINITY.compare(Fp80::NEG_INFINITY, &mut ef),
            FpOrdering::Equal
        );
        assert_eq!(ef, no_exceptions());
    }

    #[test]
    fn test_compare_quiet_basic() {
        let mut ef = no_exceptions();
        assert_eq!(TWO.compare_quiet(Fp80::ONE, &mut ef), FpOrdering::Greater);
        assert_eq!(ef, no_exceptions());

        let mut ef = no_exceptions();
        assert_eq!(
            Fp80::ONE.compare_quiet(Fp80::ONE, &mut ef),
            FpOrdering::Equal
        );

        // +0 == -0.
        let mut ef = no_exceptions();
        assert_eq!(
            Fp80::ZERO.compare_quiet(Fp80::NEG_ZERO, &mut ef),
            FpOrdering::Equal
        );
    }

    #[test]
    fn test_compare_quiet_edge_cases() {
        // QNaN -> Unordered, NO IE (key difference from ordered compare).
        let mut ef = no_exceptions();
        assert_eq!(
            POSITIVE_QNAN.compare_quiet(Fp80::ONE, &mut ef),
            FpOrdering::Unordered
        );
        assert!(!ef.invalid);

        let mut ef = no_exceptions();
        assert_eq!(
            Fp80::ONE.compare_quiet(POSITIVE_QNAN, &mut ef),
            FpOrdering::Unordered
        );
        assert!(!ef.invalid);

        // SNaN -> Unordered + IE (even in unordered comparison).
        let mut ef = no_exceptions();
        assert_eq!(
            POSITIVE_SNAN.compare_quiet(Fp80::ONE, &mut ef),
            FpOrdering::Unordered
        );
        assert!(ef.invalid);

        let mut ef = no_exceptions();
        assert_eq!(
            Fp80::ONE.compare_quiet(POSITIVE_SNAN, &mut ef),
            FpOrdering::Unordered
        );
        assert!(ef.invalid);

        // Two QNaNs -> Unordered, no IE.
        let mut ef = no_exceptions();
        assert_eq!(
            POSITIVE_QNAN.compare_quiet(Fp80::INDEFINITE, &mut ef),
            FpOrdering::Unordered
        );
        assert!(!ef.invalid);

        // Two SNaNs -> Unordered + IE (even in unordered comparison).
        let mut ef = no_exceptions();
        assert_eq!(
            POSITIVE_SNAN.compare_quiet(POSITIVE_SNAN, &mut ef),
            FpOrdering::Unordered
        );
        assert!(ef.invalid);

        // QNaN + SNaN -> Unordered + IE (SNaN triggers IE even with QNaN present).
        let mut ef = no_exceptions();
        assert_eq!(
            POSITIVE_QNAN.compare_quiet(POSITIVE_SNAN, &mut ef),
            FpOrdering::Unordered
        );
        assert!(ef.invalid);
    }

    #[test]
    fn test_compare_denormal_flag() {
        let mut ef = no_exceptions();
        let result = POSITIVE_DENORMAL.compare(Fp80::ONE, &mut ef);
        assert_eq!(result, FpOrdering::Less);
        assert!(ef.denormal);
        assert!(!ef.invalid);

        let mut ef = no_exceptions();
        let result = Fp80::ONE.compare(POSITIVE_DENORMAL, &mut ef);
        assert_eq!(result, FpOrdering::Greater);
        assert!(ef.denormal);

        let mut ef = no_exceptions();
        Fp80::ONE.compare(TWO, &mut ef);
        assert!(!ef.denormal);

        let mut ef = no_exceptions();
        let result = POSITIVE_DENORMAL.compare(Fp80::ZERO, &mut ef);
        assert_eq!(result, FpOrdering::Greater);
        assert!(ef.denormal);
    }

    #[test]
    fn test_compare_quiet_denormal_flag() {
        let mut ef = no_exceptions();
        let result = POSITIVE_DENORMAL.compare_quiet(Fp80::ONE, &mut ef);
        assert_eq!(result, FpOrdering::Less);
        assert!(ef.denormal);
        assert!(!ef.invalid);

        let mut ef = no_exceptions();
        let result = Fp80::ONE.compare_quiet(POSITIVE_DENORMAL, &mut ef);
        assert_eq!(result, FpOrdering::Greater);
        assert!(ef.denormal);

        let mut ef = no_exceptions();
        Fp80::ONE.compare_quiet(TWO, &mut ef);
        assert!(!ef.denormal);
    }
}
