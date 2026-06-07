mod common;

use common::harness::*;

#[allow(dead_code)]
mod golden {
    include!("golden/ym3526_fm.rs");
}

#[test]
fn silence_after_reset() {
    let mut chip = setup_ym3526();
    let samples = generate_1_opl(&mut chip, golden::SILENCE.len());
    assert_samples_1(&samples, golden::SILENCE);
}

#[test]
fn single_tone_algo1_additive() {
    let mut chip = setup_ym3526();
    setup_opl_simple_tone(&mut chip, 0, 1, 0);
    key_on_opl(&mut chip, 0);
    let samples = generate_1_opl(&mut chip, golden::SINGLE_TONE_ALGO1.len());
    assert_samples_1(&samples, golden::SINGLE_TONE_ALGO1);
}

#[test]
fn single_tone_algo0_fm() {
    let mut chip = setup_ym3526();
    setup_opl_simple_tone(&mut chip, 0, 0, 0);
    key_on_opl(&mut chip, 0);
    let samples = generate_1_opl(&mut chip, golden::SINGLE_TONE_ALGO0.len());
    assert_samples_1(&samples, golden::SINGLE_TONE_ALGO0);
}

#[test]
fn feedback_sweep() {
    let golden_fbs: [&[[i32; 1]]; 8] = [
        golden::FEEDBACK_0,
        golden::FEEDBACK_1,
        golden::FEEDBACK_2,
        golden::FEEDBACK_3,
        golden::FEEDBACK_4,
        golden::FEEDBACK_5,
        golden::FEEDBACK_6,
        golden::FEEDBACK_7,
    ];

    for (fb, expected) in golden_fbs.iter().enumerate() {
        let mut chip = setup_ym3526();
        setup_opl_simple_tone(&mut chip, 0, 0, fb as u8);
        key_on_opl(&mut chip, 0);
        let samples = generate_1_opl(&mut chip, expected.len());
        assert_samples_1(&samples, expected);
    }
}

#[test]
fn multiple_sweep() {
    let cases: [(u8, &[[i32; 1]]); 4] = [
        (0, golden::MULTIPLE_0),
        (1, golden::MULTIPLE_1),
        (2, golden::MULTIPLE_2),
        (4, golden::MULTIPLE_4),
    ];

    for (mul, expected) in &cases {
        let mut chip = setup_ym3526();
        setup_opl_simple_tone(&mut chip, 0, 1, 0);
        for op in 0..2u8 {
            let off = opl_op_offset(0, op);
            write_reg_opl(&mut chip, 0x20 + off, 0x20 | mul);
        }
        key_on_opl(&mut chip, 0);
        let samples = generate_1_opl(&mut chip, expected.len());
        assert_samples_1(&samples, expected);
    }
}

#[test]
fn total_level_sweep() {
    let cases: [(u8, &[[i32; 1]]); 4] = [
        (0x00, golden::TL_00),
        (0x10, golden::TL_10),
        (0x20, golden::TL_20),
        (0x3F, golden::TL_3F),
    ];

    for (tl, expected) in &cases {
        let mut chip = setup_ym3526();
        setup_opl_simple_tone(&mut chip, 0, 1, 0);
        let carrier_off = opl_op_offset(0, 1);
        write_reg_opl(&mut chip, 0x40 + carrier_off, *tl);
        key_on_opl(&mut chip, 0);
        let samples = generate_1_opl(&mut chip, expected.len());
        assert_samples_1(&samples, expected);
    }
}

#[test]
fn full_adsr_envelope_cycle() {
    let mut chip = setup_ym3526();
    setup_opl_simple_tone(&mut chip, 0, 1, 0);
    key_on_opl(&mut chip, 0);
    let sustain = generate_1_opl(&mut chip, golden::ADSR_SUSTAIN.len());
    assert_samples_1(&sustain, golden::ADSR_SUSTAIN);

    key_off_opl(&mut chip, 0);
    let release = generate_1_opl(&mut chip, golden::ADSR_RELEASE.len());
    assert_samples_1(&release, golden::ADSR_RELEASE);
}

#[test]
fn max_attack_rate() {
    let mut chip = setup_ym3526();
    setup_opl_simple_tone(&mut chip, 0, 1, 0);
    key_on_opl(&mut chip, 0);
    let samples = generate_1_opl(&mut chip, golden::MAX_ATTACK_RATE.len());
    assert_samples_1(&samples, golden::MAX_ATTACK_RATE);
}

#[test]
fn zero_attack_rate() {
    let mut chip = setup_ym3526();
    setup_opl_simple_tone(&mut chip, 0, 1, 0);
    for op in 0..2u8 {
        let off = opl_op_offset(0, op);
        write_reg_opl(&mut chip, 0x60 + off, 0x00);
    }
    key_on_opl(&mut chip, 0);
    let samples = generate_1_opl(&mut chip, golden::ZERO_ATTACK_RATE.len());
    assert_samples_1(&samples, golden::ZERO_ATTACK_RATE);
}

#[test]
fn key_reon_during_release() {
    let mut chip = setup_ym3526();
    setup_opl_simple_tone(&mut chip, 0, 1, 0);
    for op in 0..2u8 {
        let off = opl_op_offset(0, op);
        write_reg_opl(&mut chip, 0x80 + off, 0x84);
    }
    key_on_opl(&mut chip, 0);
    let sustain = generate_1_opl(&mut chip, golden::REON_SUSTAIN.len());
    assert_samples_1(&sustain, golden::REON_SUSTAIN);

    key_off_opl(&mut chip, 0);
    let release = generate_1_opl(&mut chip, golden::REON_RELEASE.len());
    assert_samples_1(&release, golden::REON_RELEASE);

    key_on_opl(&mut chip, 0);
    let reon = generate_1_opl(&mut chip, golden::REON_AFTER.len());
    assert_samples_1(&reon, golden::REON_AFTER);
}

#[test]
fn two_channels() {
    let mut chip = setup_ym3526();
    setup_opl_simple_tone(&mut chip, 0, 1, 0);
    setup_opl_simple_tone(&mut chip, 1, 1, 0);
    write_reg_opl(&mut chip, 0xA1, 0x81);
    key_on_opl(&mut chip, 0);
    key_on_opl(&mut chip, 1);
    let samples = generate_1_opl(&mut chip, golden::TWO_CHANNELS.len());
    assert_samples_1(&samples, golden::TWO_CHANNELS);
}

#[test]
fn all_9_channels() {
    let mut chip = setup_ym3526();
    for ch in 0..9u8 {
        setup_opl_simple_tone(&mut chip, ch, 1, 0);
        write_reg_opl(&mut chip, 0xA0 + ch, 0x41 + ch * 8);
        key_on_opl(&mut chip, ch);
    }
    let samples = generate_1_opl(&mut chip, golden::ALL_9_CHANNELS.len());
    assert_samples_1(&samples, golden::ALL_9_CHANNELS);
}

#[test]
fn frequency_zero() {
    let mut chip = setup_ym3526();
    setup_opl_simple_tone(&mut chip, 0, 1, 0);
    write_reg_opl(&mut chip, 0xA0, 0x00);
    write_reg_opl(&mut chip, 0xB0, 0x20);
    let samples = generate_1_opl(&mut chip, golden::FREQ_ZERO.len());
    assert_samples_1(&samples, golden::FREQ_ZERO);
}

#[test]
fn am_vib_depth() {
    let mut chip = setup_ym3526();
    setup_opl_simple_tone(&mut chip, 0, 1, 0);
    for op in 0..2u8 {
        let off = opl_op_offset(0, op);
        write_reg_opl(&mut chip, 0x20 + off, 0xE1);
    }
    write_reg_opl(&mut chip, 0xBD, 0xC0);
    key_on_opl(&mut chip, 0);
    let samples = generate_1_opl(&mut chip, golden::AM_VIB_DEPTH.len());
    assert_samples_1(&samples, golden::AM_VIB_DEPTH);
}

#[test]
fn ksl_sweep() {
    let cases: [(u8, &[[i32; 1]]); 4] = [
        (0, golden::KSL_0),
        (1, golden::KSL_1),
        (2, golden::KSL_2),
        (3, golden::KSL_3),
    ];

    for (ksl, expected) in &cases {
        let mut chip = setup_ym3526();
        setup_opl_simple_tone(&mut chip, 0, 1, 0);
        let carrier_off = opl_op_offset(0, 1);
        write_reg_opl(&mut chip, 0x40 + carrier_off, ksl << 6);
        key_on_opl(&mut chip, 0);
        let samples = generate_1_opl(&mut chip, expected.len());
        assert_samples_1(&samples, expected);
    }
}

#[test]
fn rhythm_mode() {
    let mut chip = setup_ym3526();
    for ch in 6..9u8 {
        setup_opl_simple_tone(&mut chip, ch, 1, 0);
    }
    write_reg_opl(&mut chip, 0xBD, 0x3F);
    let samples = generate_1_opl(&mut chip, golden::RHYTHM_MODE.len());
    assert_samples_1(&samples, golden::RHYTHM_MODE);
}

#[test]
fn rhythm_individual_instruments() {
    let cases: [(&str, u8, &[[i32; 1]]); 5] = [
        ("BD", 0x10, golden::RHYTHM_BD),
        ("SD", 0x08, golden::RHYTHM_SD),
        ("TOM", 0x04, golden::RHYTHM_TOM),
        ("CY", 0x02, golden::RHYTHM_CY),
        ("HH", 0x01, golden::RHYTHM_HH),
    ];

    for (name, bit, expected) in &cases {
        let mut chip = setup_ym3526();
        for ch in 6..9u8 {
            setup_opl_simple_tone(&mut chip, ch, 1, 0);
        }
        write_reg_opl(&mut chip, 0xBD, 0x20 | bit);
        let samples = generate_1_opl(&mut chip, expected.len());
        assert_samples_1(&samples, expected);
        let _ = name;
    }
}

#[test]
fn csm_mode_timer_a_triggers_keyon() {
    use ymfm_oxide::Ym3526;

    let mut chip = Ym3526::new();
    chip.reset();
    setup_opl_simple_tone(&mut chip, 0, 1, 0);
    write_reg_opl(&mut chip, 0x02, 0xFF);
    write_reg_opl(&mut chip, 0x08, 0x80);
    write_reg_opl(&mut chip, 0x04, 0x01);

    let before = generate_1_opl(&mut chip, golden::CSM_BEFORE_TRIGGER.len());
    assert_samples_1(&before, golden::CSM_BEFORE_TRIGGER);

    chip.timer_expired(0);

    let after = generate_1_opl(&mut chip, golden::CSM_AFTER_TRIGGER.len());
    assert_samples_1(&after, golden::CSM_AFTER_TRIGGER);
}
