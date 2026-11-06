use softfloat::{ExceptionFlags, Fp80, Precision, RoundingMode};

const RC: RoundingMode = RoundingMode::NearestEven;
const PC: Precision = Precision::Extended;

fn sign_exp(fp: Fp80) -> u16 {
    let bytes = fp.to_le_bytes();
    u16::from_le_bytes([bytes[8], bytes[9]])
}

fn ulp_distance(computed: Fp80, expected: Fp80) -> u64 {
    let c_sig = computed.significand();
    let e_sig = expected.significand();

    // Both zero: match regardless of sign.
    if c_sig == 0 && e_sig == 0 {
        return 0;
    }

    // Sign/exponent mismatch: infinite ULP.
    if sign_exp(computed) != sign_exp(expected) {
        return u64::MAX;
    }

    c_sig.abs_diff(e_sig)
}

fn check(name: &str, computed: Fp80, exp_sign_exp: u16, sig: u64, max_ulp: u64) {
    let expected = Fp80::from_bits(exp_sign_exp, sig);
    let ulp = ulp_distance(computed, expected);
    assert!(
        ulp <= max_ulp,
        "{name}: ULP {ulp} exceeds threshold {max_ulp} \
         (computed: 0x{:04X}_{:016X}, expected: 0x{exp_sign_exp:04X}_{sig:016X})",
        sign_exp(computed),
        computed.significand(),
    );
}

#[test]
fn fpu_constant_pi() {
    check("FLDPI", Fp80::PI_UP, 0x4000, 0xC90FDAA22168C235, 0);
}

#[test]
fn fpu_constant_one() {
    check("FLD1", Fp80::ONE, 0x3FFF, 0x8000000000000000, 0);
}

#[test]
fn fpu_constant_zero() {
    check("FLDZ", Fp80::ZERO, 0x0000, 0x0000000000000000, 0);
}

#[test]
fn fpu_constant_log2_10() {
    check("FLDL2T", Fp80::LOG2_10_DOWN, 0x4000, 0xD49A784BCD1B8AFE, 0);
}

#[test]
fn fpu_constant_log2_e() {
    check("FLDL2E", Fp80::LOG2_E_UP, 0x3FFF, 0xB8AA3B295C17F0BC, 0);
}

#[test]
fn fpu_constant_log10_2() {
    check("FLDLG2", Fp80::LOG10_2_UP, 0x3FFD, 0x9A209A84FBCFF799, 0);
}

#[test]
fn fpu_constant_ln_2() {
    check("FLDLN2", Fp80::LN_2_UP, 0x3FFE, 0xB17217F7D1CF79AC, 0);
}

#[test]
fn arithmetic_add() {
    let mut ef = ExceptionFlags::default();
    let a = Fp80::from_f64(3.0, &mut ef);
    let b = Fp80::from_f64(4.0, &mut ef);
    let result = a.add(b, RC, PC, &mut ef);
    check("3+4", result, 0x4001, 0xE000000000000000, 0);
}

#[test]
fn arithmetic_sub() {
    let mut ef = ExceptionFlags::default();
    let a = Fp80::from_f64(10.0, &mut ef);
    let b = Fp80::from_f64(3.0, &mut ef);
    let result = a.sub(b, RC, PC, &mut ef);
    check("10-3", result, 0x4001, 0xE000000000000000, 0);
}

#[test]
fn arithmetic_mul() {
    let mut ef = ExceptionFlags::default();
    let a = Fp80::from_f64(6.0, &mut ef);
    let b = Fp80::from_f64(7.0, &mut ef);
    let result = a.mul(b, RC, PC, &mut ef);
    check("6*7", result, 0x4004, 0xA800000000000000, 0);
}

#[test]
fn arithmetic_div() {
    let mut ef = ExceptionFlags::default();
    let a = Fp80::from_f64(355.0, &mut ef);
    let b = Fp80::from_f64(113.0, &mut ef);
    let result = a.div(b, RC, PC, &mut ef);
    // Bytes from mpmath: 0x09,0xBC,0xFD,0x90,0xC0,0xDB,0x0F,0xC9 -> sig = 0xC90FDBC090FDBC09
    check("355/113", result, 0x4000, 0xC90FDBC090FDBC09, 0);
}

#[test]
fn arithmetic_neg_add() {
    let mut ef = ExceptionFlags::default();
    let a = Fp80::from_f64(-5.0, &mut ef);
    let b = Fp80::from_f64(5.0, &mut ef);
    let result = a.add(b, RC, PC, &mut ef);
    check("-5+5", result, 0x0000, 0x0000000000000000, 0);
}

#[test]
fn transcendental_sqrt2() {
    let mut ef = ExceptionFlags::default();
    let two = Fp80::from_f64(2.0, &mut ef);
    let result = two.sqrt(RC, PC, &mut ef);
    // IEEE 754 mandates correctly-rounded sqrt.
    check("SQRT(2)", result, 0x3FFF, 0xB504F333F9DE6484, 0);
}

#[test]
fn transcendental_sin_pi6() {
    let mut ef = ExceptionFlags::default();
    let six = Fp80::from_f64(6.0, &mut ef);
    let pi_over_6 = Fp80::PI_UP.div(six, RC, PC, &mut ef);
    let (result, _) = pi_over_6.fsin(RC, &mut ef);
    check("SIN(PI/6)", result, 0x3FFE, 0x8000000000000000, 0);
}

#[test]
fn transcendental_cos_pi3() {
    let mut ef = ExceptionFlags::default();
    let three = Fp80::from_f64(3.0, &mut ef);
    let pi_over_3 = Fp80::PI_UP.div(three, RC, PC, &mut ef);
    let (result, _) = pi_over_3.fcos(RC, &mut ef);
    check("COS(PI/3)", result, 0x3FFE, 0x8000000000000000, 0);
}

#[test]
fn transcendental_tan_pi4() {
    let mut ef = ExceptionFlags::default();
    let four = Fp80::from_f64(4.0, &mut ef);
    let pi_over_4 = Fp80::PI_UP.div(four, RC, PC, &mut ef);
    let (result, _) = pi_over_4.fptan(RC, &mut ef);
    check("TAN(PI/4)", result, 0x3FFF, 0x8000000000000000, 0);
}

#[test]
fn transcendental_4atan1() {
    let mut ef = ExceptionFlags::default();
    let four = Fp80::from_f64(4.0, &mut ef);
    let atan1 = Fp80::ONE.fpatan(Fp80::ONE, RC, &mut ef);
    let result = atan1.mul(four, RC, PC, &mut ef);
    check("4*ATAN(1)", result, 0x4000, 0xC90FDAA22168C235, 0);
}

#[test]
fn transcendental_f2xm1() {
    let mut ef = ExceptionFlags::default();
    let half = Fp80::from_f64(0.5, &mut ef);
    let result = half.f2xm1(RC, &mut ef);
    check("F2XM1(0.5)", result, 0x3FFD, 0xD413CCCFE7799211, 0);
}

#[test]
fn transcendental_f2xm1_near_boundary() {
    let mut ef = ExceptionFlags::default();
    // x = 15/16 = 0.9375 (exact in f64, near the |x| < 1 domain boundary).
    let x = Fp80::from_f64(0.9375, &mut ef);
    let result = x.f2xm1(RC, &mut ef);
    check("F2XM1(0.9375)", result, 0x3FFE, 0xEA4AFA2A490D9859, 0);

    let mut ef = ExceptionFlags::default();
    // x = -15/16 = -0.9375.
    let neg_x = Fp80::from_f64(-0.9375, &mut ef);
    let result = neg_x.f2xm1(RC, &mut ef);
    check("F2XM1(-0.9375)", result, 0xBFFD, 0xF4AA7930676F09D6, 0);
}

#[test]
fn transcendental_fyl2x() {
    let mut ef = ExceptionFlags::default();
    let eight = Fp80::from_f64(8.0, &mut ef);
    let result = eight.fyl2x(Fp80::ONE, RC, &mut ef);
    check("FYL2X(1,8)", result, 0x4000, 0xC000000000000000, 0);
}

#[test]
fn transcendental_fyl2xp1() {
    let mut ef = ExceptionFlags::default();
    // FYL2XP1(y=2, x=0.25) = 2 * log2(1.25). x=0.25 is within valid domain |x| < 1-sqrt(2)/2.
    let x = Fp80::from_f64(0.25, &mut ef);
    let y = Fp80::from_f64(2.0, &mut ef);
    let result = x.fyl2xp1(y, RC, &mut ef);
    check("FYL2XP1(2,0.25)", result, 0x3FFE, 0xA4D3C25E68DC57F2, 0);
}

#[test]
fn transcendental_fscale() {
    let mut ef = ExceptionFlags::default();
    let one_point_five = Fp80::from_f64(1.5, &mut ef);
    let two = Fp80::from_f64(2.0, &mut ef);
    let result = one_point_five.scale(two, RoundingMode::NearestEven, &mut ef);
    check("FSCALE(1.5,2)", result, 0x4001, 0xC000000000000000, 0);
}

#[test]
fn transcendental_machin_pi() {
    let mut ef = ExceptionFlags::default();
    let four = Fp80::from_f64(4.0, &mut ef);
    let five = Fp80::from_f64(5.0, &mut ef);
    let sixteen = Fp80::from_f64(16.0, &mut ef);
    let two_three_nine = Fp80::from_f64(239.0, &mut ef);

    let atan_1_5 = Fp80::ONE.fpatan(five, RC, &mut ef);
    let term1 = atan_1_5.mul(sixteen, RC, PC, &mut ef);

    let atan_1_239 = Fp80::ONE.fpatan(two_three_nine, RC, &mut ef);
    let term2 = atan_1_239.mul(four, RC, PC, &mut ef);

    let result = term1.sub(term2, RC, PC, &mut ef);
    check("MACHIN PI", result, 0x4000, 0xC90FDAA22168C235, 0);
}

#[test]
fn golden_fsin() {
    let cases: &[((u16, u64), &str, u16, u64)] = &[
        (
            (0x3FFF, 0x8000_0000_0000_0000),
            "FSIN(1.0)",
            0x3FFE,
            0xD76AA47848677021,
        ),
        (
            (0x3FFE, 0x8000_0000_0000_0000),
            "FSIN(0.5)",
            0x3FFD,
            0xF57743A2582F7F44,
        ),
        (
            (0x3FFD, 0x8000_0000_0000_0000),
            "FSIN(0.25)",
            0x3FFC,
            0xFD5776A798ABB5D4,
        ),
        (
            (0x3FFE, 0xC90F_DAA2_2168_C235),
            "FSIN(pi/4)",
            0x3FFE,
            0xB504F333F9DE6485,
        ),
        (
            (0x3FFF, 0xC90F_DAA2_2168_C235),
            "FSIN(pi/2)",
            0x3FFF,
            0x8000_0000_0000_0000,
        ),
        (
            (0x4000, 0xC90F_DAA2_2168_C235),
            "FSIN(pi)",
            0xBFBF,
            0x8000_0000_0000_0000,
        ),
        (
            (0x4001, 0xC90F_DAA2_2168_C235),
            "FSIN(2pi)",
            0x3FC0,
            0x8000_0000_0000_0000,
        ),
        (
            (0x4002, 0x96CB_E3F9_990E_91A8),
            "FSIN(3pi)",
            0xBFC1,
            0xE000_0000_0000_0000,
        ),
        (
            (0x4005, 0xC800_0000_0000_0000),
            "FSIN(100)",
            0xBFFE,
            0x81A12DBC626DC036,
        ),
        (
            (0x4008, 0xFA00_0000_0000_0000),
            "FSIN(1000)",
            0x3FFE,
            0xD3AE60A851035AD7,
        ),
        (
            (0xBFFF, 0x8000_0000_0000_0000),
            "FSIN(-1.0)",
            0xBFFE,
            0xD76AA47848677021,
        ),
        (
            (0xC000, 0xC90F_DAA2_2168_C235),
            "FSIN(-pi)",
            0x3FBF,
            0x8000_0000_0000_0000,
        ),
    ];

    for &((in_exp, in_sig), name, exp_se, exp_sig) in cases {
        let mut ef = ExceptionFlags::default();
        let input = Fp80::from_bits(in_exp, in_sig);
        let (result, _) = input.fsin(RC, &mut ef);
        check(name, result, exp_se, exp_sig, 0);
    }
}

#[test]
fn golden_fcos() {
    let cases: &[((u16, u64), &str, u16, u64)] = &[
        (
            (0x3FFF, 0x8000_0000_0000_0000),
            "FCOS(1.0)",
            0x3FFE,
            0x8A51407DA8345C92,
        ),
        (
            (0x3FFE, 0x8000_0000_0000_0000),
            "FCOS(0.5)",
            0x3FFE,
            0xE0A94032DBEA7CEE,
        ),
        (
            (0x3FFD, 0x8000_0000_0000_0000),
            "FCOS(0.25)",
            0x3FFE,
            0xF80AA4FBEF750BA8,
        ),
        (
            (0x3FFE, 0xC90F_DAA2_2168_C235),
            "FCOS(pi/4)",
            0x3FFE,
            0xB504F333F9DE6484,
        ),
        (
            (0x3FFF, 0xC90F_DAA2_2168_C235),
            "FCOS(pi/2)",
            0xBFBE,
            0x8000_0000_0000_0000,
        ),
        (
            (0x4000, 0xC90F_DAA2_2168_C235),
            "FCOS(pi)",
            0xBFFF,
            0x8000_0000_0000_0000,
        ),
        (
            (0x4001, 0xC90F_DAA2_2168_C235),
            "FCOS(2pi)",
            0x3FFF,
            0x8000_0000_0000_0000,
        ),
        (
            (0x4002, 0x96CB_E3F9_990E_91A8),
            "FCOS(3pi)",
            0xBFFF,
            0x8000_0000_0000_0000,
        ),
        (
            (0x4005, 0xC800_0000_0000_0000),
            "FCOS(100)",
            0x3FFE,
            0xDCC0EDFB32FEFB21,
        ),
        (
            (0x4008, 0xFA00_0000_0000_0000),
            "FCOS(1000)",
            0x3FFE,
            0x8FF8133C9F8DDBA4,
        ),
        (
            (0xBFFF, 0x8000_0000_0000_0000),
            "FCOS(-1.0)",
            0x3FFE,
            0x8A51407DA8345C92,
        ),
        (
            (0xC000, 0xC90F_DAA2_2168_C235),
            "FCOS(-pi)",
            0xBFFF,
            0x8000_0000_0000_0000,
        ),
    ];

    for &((in_exp, in_sig), name, exp_se, exp_sig) in cases {
        let mut ef = ExceptionFlags::default();
        let input = Fp80::from_bits(in_exp, in_sig);
        let (result, _) = input.fcos(RC, &mut ef);
        check(name, result, exp_se, exp_sig, 0);
    }
}

#[test]
fn golden_fptan() {
    let cases: &[((u16, u64), &str, u16, u64)] = &[
        (
            (0x3FFF, 0x8000_0000_0000_0000),
            "FPTAN(1.0)",
            0x3FFF,
            0xC75922E5F71D2DC6,
        ),
        (
            (0x3FFE, 0x8000_0000_0000_0000),
            "FPTAN(0.5)",
            0x3FFE,
            0x8BDA7ADF9A3A5219,
        ),
        (
            (0x3FFD, 0x8000_0000_0000_0000),
            "FPTAN(0.25)",
            0x3FFD,
            0x82BC2D21E262AF32,
        ),
        (
            (0x3FFE, 0xC90F_DAA2_2168_C235),
            "FPTAN(pi/4)",
            0x3FFF,
            0x8000_0000_0000_0000,
        ),
        (
            (0x3FFF, 0xC90F_DAA2_2168_C235),
            "FPTAN(pi/2)",
            0xC040,
            0x8000_0000_0000_0000,
        ),
        (
            (0x4000, 0xC90F_DAA2_2168_C235),
            "FPTAN(pi)",
            0x3FBF,
            0x8000_0000_0000_0000,
        ),
        (
            (0x4001, 0xC90F_DAA2_2168_C235),
            "FPTAN(2pi)",
            0x3FC0,
            0x8000_0000_0000_0000,
        ),
        (
            (0x4002, 0x96CB_E3F9_990E_91A8),
            "FPTAN(3pi)",
            0x3FC1,
            0xE000_0000_0000_0000,
        ),
        (
            (0x4005, 0xC800_0000_0000_0000),
            "FPTAN(100)",
            0xBFFE,
            0x9653A6B15AE9BD79,
        ),
        (
            (0x4008, 0xFA00_0000_0000_0000),
            "FPTAN(1000)",
            0x3FFF,
            0xBC3394F9A188CFA0,
        ),
        (
            (0xBFFF, 0x8000_0000_0000_0000),
            "FPTAN(-1.0)",
            0xBFFF,
            0xC75922E5F71D2DC6,
        ),
        (
            (0xC000, 0xC90F_DAA2_2168_C235),
            "FPTAN(-pi)",
            0xBFBF,
            0x8000_0000_0000_0000,
        ),
    ];

    for &((in_exp, in_sig), name, exp_se, exp_sig) in cases {
        let mut ef = ExceptionFlags::default();
        let input = Fp80::from_bits(in_exp, in_sig);
        let (result, _) = input.fptan(RC, &mut ef);
        check(name, result, exp_se, exp_sig, 1);
    }
}

#[test]
fn golden_fsincos() {
    fn case(
        in_exp: u16,
        in_sig: u64,
        name: &str,
        sin_se: u16,
        sin_sig: u64,
        cos_se: u16,
        cos_sig: u64,
    ) {
        let mut ef = ExceptionFlags::default();
        let input = Fp80::from_bits(in_exp, in_sig);
        let (sin_result, cos_result, _) = input.fsincos(RC, &mut ef);
        check(&format!("{name} sin"), sin_result, sin_se, sin_sig, 0);
        check(&format!("{name} cos"), cos_result, cos_se, cos_sig, 0);
    }

    case(
        0x3FFF,
        0x8000_0000_0000_0000,
        "FSINCOS(1.0)",
        0x3FFE,
        0xD76AA47848677021,
        0x3FFE,
        0x8A51407DA8345C92,
    );
    case(
        0x3FFE,
        0x8000_0000_0000_0000,
        "FSINCOS(0.5)",
        0x3FFD,
        0xF57743A2582F7F44,
        0x3FFE,
        0xE0A94032DBEA7CEE,
    );
    case(
        0x4000,
        0xC90F_DAA2_2168_C235,
        "FSINCOS(pi)",
        0xBFBF,
        0x8000_0000_0000_0000,
        0xBFFF,
        0x8000_0000_0000_0000,
    );
    case(
        0xBFFF,
        0x8000_0000_0000_0000,
        "FSINCOS(-1.0)",
        0xBFFE,
        0xD76AA47848677021,
        0x3FFE,
        0x8A51407DA8345C92,
    );
}
