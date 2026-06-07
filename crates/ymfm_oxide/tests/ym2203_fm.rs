mod common;

use common::harness::*;
use ymfm_oxide::{Ym2203, YmfmOpnFidelity};

#[allow(dead_code)]
mod golden {
    include!("golden/ym2203_fm.rs");
}

#[test]
fn silence_after_reset() {
    let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
    let samples = generate_4(&mut chip, golden::SILENCE.len());
    assert_samples_4(&samples, golden::SILENCE);
}

#[test]
fn single_tone_algo7_produces_sound() {
    let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
    add_ssg_bg_2203(&mut chip);
    setup_ym2203_simple_tone(&mut chip, 0, 7, 0);
    key_on_2203(&mut chip, 0);
    let samples = generate_4(&mut chip, golden::SINGLE_TONE_ALGO7.len());
    assert_samples_4(&samples, golden::SINGLE_TONE_ALGO7);
}

#[test]
fn all_8_algorithms_produce_distinct_output() {
    let golden_algos: [&[[i32; 4]]; 8] = [
        golden::ALGO_0,
        golden::ALGO_1,
        golden::ALGO_2,
        golden::ALGO_3,
        golden::ALGO_4,
        golden::ALGO_5,
        golden::ALGO_6,
        golden::ALGO_7,
    ];

    for (algo, expected) in golden_algos.iter().enumerate() {
        let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
        add_ssg_bg_2203(&mut chip);

        write_reg(&mut chip, 0xB0, algo as u8);
        for (op_offset, tl) in [(0x00, 0x20), (0x04, 0x20), (0x08, 0x20), (0x0C, 0x00)] {
            write_reg(&mut chip, 0x30 + op_offset, 0x01);
            write_reg(&mut chip, 0x40 + op_offset, tl);
            write_reg(&mut chip, 0x50 + op_offset, 0x1F);
            write_reg(&mut chip, 0x60 + op_offset, 0x00);
            write_reg(&mut chip, 0x70 + op_offset, 0x00);
            write_reg(&mut chip, 0x80 + op_offset, 0x0F);
            write_reg(&mut chip, 0x90 + op_offset, 0x00);
        }
        write_reg(&mut chip, 0xA4, 0x22);
        write_reg(&mut chip, 0xA0, 0x69);
        key_on_2203(&mut chip, 0);
        let samples = generate_4(&mut chip, expected.len());
        assert_samples_4(&samples, expected);
    }
}

#[test]
fn feedback_sweep_produces_distinct_output() {
    let golden_fbs: [&[[i32; 4]]; 8] = [
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
        let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
        add_ssg_bg_2203(&mut chip);
        setup_ym2203_simple_tone(&mut chip, 0, 0, fb as u8);
        key_on_2203(&mut chip, 0);
        let samples = generate_4(&mut chip, expected.len());
        assert_samples_4(&samples, expected);
    }
}

#[test]
fn detune_produces_different_pitch() {
    let golden_detunes: [&[[i32; 4]]; 8] = [
        golden::DETUNE_0,
        golden::DETUNE_1,
        golden::DETUNE_2,
        golden::DETUNE_3,
        golden::DETUNE_4,
        golden::DETUNE_5,
        golden::DETUNE_6,
        golden::DETUNE_7,
    ];

    for (dt, expected) in golden_detunes.iter().enumerate() {
        let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
        add_ssg_bg_2203(&mut chip);
        write_reg(&mut chip, 0xB0, 0x07);
        for op_offset in [0x00, 0x04, 0x08, 0x0C] {
            write_reg(&mut chip, 0x30 + op_offset, ((dt as u8) << 4) | 0x01);
            write_reg(&mut chip, 0x40 + op_offset, 0x10);
            write_reg(&mut chip, 0x50 + op_offset, 0x1F);
            write_reg(&mut chip, 0x60 + op_offset, 0x00);
            write_reg(&mut chip, 0x70 + op_offset, 0x00);
            write_reg(&mut chip, 0x80 + op_offset, 0x0F);
            write_reg(&mut chip, 0x90 + op_offset, 0x00);
        }
        write_reg(&mut chip, 0xA4, 0x22);
        write_reg(&mut chip, 0xA0, 0x69);
        key_on_2203(&mut chip, 0);
        let samples = generate_4(&mut chip, expected.len());
        assert_samples_4(&samples, expected);
    }
}

#[test]
fn multiple_zero_is_half() {
    let mut chip0 = setup_ym2203(YmfmOpnFidelity::Max);
    add_ssg_bg_2203(&mut chip0);
    setup_ym2203_simple_tone(&mut chip0, 0, 7, 0);
    for op_offset in [0x00, 0x04, 0x08, 0x0C] {
        write_reg(&mut chip0, 0x30 + op_offset, 0x00);
    }
    key_on_2203(&mut chip0, 0);
    let samples0 = generate_4(&mut chip0, golden::MULTIPLE_0.len());
    assert_samples_4(&samples0, golden::MULTIPLE_0);

    let mut chip1 = setup_ym2203(YmfmOpnFidelity::Max);
    add_ssg_bg_2203(&mut chip1);
    setup_ym2203_simple_tone(&mut chip1, 0, 7, 0);
    for op_offset in [0x00, 0x04, 0x08, 0x0C] {
        write_reg(&mut chip1, 0x30 + op_offset, 0x01);
    }
    key_on_2203(&mut chip1, 0);
    let samples1 = generate_4(&mut chip1, golden::MULTIPLE_1.len());
    assert_samples_4(&samples1, golden::MULTIPLE_1);
}

#[test]
fn full_adsr_envelope_cycle() {
    let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
    add_ssg_bg_2203(&mut chip);

    write_reg(&mut chip, 0xB0, 0x07);
    for op_offset in [0x00, 0x04, 0x08, 0x0C] {
        write_reg(&mut chip, 0x30 + op_offset, 0x01);
        write_reg(&mut chip, 0x40 + op_offset, 0x00);
        write_reg(&mut chip, 0x50 + op_offset, 0x1F);
        write_reg(&mut chip, 0x60 + op_offset, 0x00);
        write_reg(&mut chip, 0x70 + op_offset, 0x00);
        write_reg(&mut chip, 0x80 + op_offset, 0x0F);
        write_reg(&mut chip, 0x90 + op_offset, 0x00);
    }
    write_reg(&mut chip, 0xA4, 0x22);
    write_reg(&mut chip, 0xA0, 0x69);

    key_on_2203(&mut chip, 0);
    let sustain_samples = generate_4(&mut chip, golden::ADSR_SUSTAIN.len());
    assert_samples_4(&sustain_samples, golden::ADSR_SUSTAIN);

    key_off_2203(&mut chip, 0);
    let release_samples = generate_4(&mut chip, golden::ADSR_RELEASE.len());
    assert_samples_4(&release_samples, golden::ADSR_RELEASE);
}

#[test]
fn max_attack_rate_immediate_volume() {
    let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
    add_ssg_bg_2203(&mut chip);
    setup_ym2203_simple_tone(&mut chip, 0, 7, 0);
    key_on_2203(&mut chip, 0);
    let samples = generate_4(&mut chip, golden::MAX_ATTACK_RATE.len());
    assert_samples_4(&samples, golden::MAX_ATTACK_RATE);
}

#[test]
fn zero_attack_rate_no_sound() {
    let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
    add_ssg_bg_2203(&mut chip);

    write_reg(&mut chip, 0xB0, 0x07);
    for op_offset in [0x00, 0x04, 0x08, 0x0C] {
        write_reg(&mut chip, 0x30 + op_offset, 0x01);
        write_reg(&mut chip, 0x40 + op_offset, 0x00);
        write_reg(&mut chip, 0x50 + op_offset, 0x00);
        write_reg(&mut chip, 0x60 + op_offset, 0x00);
        write_reg(&mut chip, 0x70 + op_offset, 0x00);
        write_reg(&mut chip, 0x80 + op_offset, 0x0F);
        write_reg(&mut chip, 0x90 + op_offset, 0x00);
    }
    write_reg(&mut chip, 0xA4, 0x22);
    write_reg(&mut chip, 0xA0, 0x69);

    key_on_2203(&mut chip, 0);
    let samples = generate_4(&mut chip, golden::ZERO_ATTACK_RATE.len());
    assert_samples_4(&samples, golden::ZERO_ATTACK_RATE);
}

#[test]
fn key_on_individual_operators() {
    let mut chip_all = setup_ym2203(YmfmOpnFidelity::Max);
    add_ssg_bg_2203(&mut chip_all);
    setup_ym2203_simple_tone(&mut chip_all, 0, 7, 0);
    key_on_2203(&mut chip_all, 0);
    let samples_all = generate_4(&mut chip_all, golden::KEY_ON_ALL_OPS.len());
    assert_samples_4(&samples_all, golden::KEY_ON_ALL_OPS);

    let mut chip_op1 = setup_ym2203(YmfmOpnFidelity::Max);
    add_ssg_bg_2203(&mut chip_op1);
    setup_ym2203_simple_tone(&mut chip_op1, 0, 7, 0);
    write_reg(&mut chip_op1, 0x28, 0x10);
    let samples_op1 = generate_4(&mut chip_op1, golden::KEY_ON_OP1_ONLY.len());
    assert_samples_4(&samples_op1, golden::KEY_ON_OP1_ONLY);
}

#[test]
fn multi_channel_produces_combined_output() {
    let mut chip1 = setup_ym2203(YmfmOpnFidelity::Max);
    add_ssg_bg_2203(&mut chip1);
    setup_ym2203_simple_tone(&mut chip1, 0, 7, 0);
    key_on_2203(&mut chip1, 0);
    let samples_1ch = generate_4(&mut chip1, golden::ONE_CHANNEL.len());
    assert_samples_4(&samples_1ch, golden::ONE_CHANNEL);

    let mut chip2 = setup_ym2203(YmfmOpnFidelity::Max);
    add_ssg_bg_2203(&mut chip2);
    setup_ym2203_simple_tone(&mut chip2, 0, 7, 0);
    setup_ym2203_simple_tone(&mut chip2, 1, 7, 0);
    write_reg(&mut chip2, 0xA5, 0x26);
    write_reg(&mut chip2, 0xA1, 0xD5);
    key_on_2203(&mut chip2, 0);
    key_on_2203(&mut chip2, 1);
    let samples_2ch = generate_4(&mut chip2, golden::TWO_CHANNELS.len());
    assert_samples_4(&samples_2ch, golden::TWO_CHANNELS);
}

#[test]
fn channel2_multi_freq_mode() {
    let mut chip = Ym2203::new();
    chip.reset();
    chip.set_fidelity(YmfmOpnFidelity::Max);
    add_ssg_bg_2203(&mut chip);

    write_reg(&mut chip, 0x27, 0x40);
    setup_ym2203_simple_tone(&mut chip, 2, 7, 0);

    write_reg(&mut chip, 0xAD, 0x22);
    write_reg(&mut chip, 0xA9, 0x69);
    write_reg(&mut chip, 0xAE, 0x26);
    write_reg(&mut chip, 0xAA, 0xD5);
    write_reg(&mut chip, 0xAC, 0x2A);
    write_reg(&mut chip, 0xA8, 0x40);

    key_on_2203(&mut chip, 2);
    let samples = generate_4(&mut chip, golden::CH2_MULTI_FREQ.len());
    assert_samples_4(&samples, golden::CH2_MULTI_FREQ);
}

#[test]
fn csm_mode_timer_a_triggers_keyon() {
    let mut chip = Ym2203::new();
    chip.reset();
    chip.set_fidelity(YmfmOpnFidelity::Max);
    add_ssg_bg_2203(&mut chip);

    setup_ym2203_simple_tone(&mut chip, 2, 7, 0);

    write_reg(&mut chip, 0x24, 0xFF);
    write_reg(&mut chip, 0x25, 0x03);
    write_reg(&mut chip, 0x27, 0x85);

    let before = generate_4(&mut chip, golden::CSM_BEFORE_TRIGGER.len());
    assert_samples_4(&before, golden::CSM_BEFORE_TRIGGER);

    chip.timer_expired(0);

    let after = generate_4(&mut chip, golden::CSM_AFTER_TRIGGER.len());
    assert_samples_4(&after, golden::CSM_AFTER_TRIGGER);
}

#[test]
fn ssg_eg_modes_produce_different_envelopes() {
    let golden_ssg: [&[[i32; 4]]; 8] = [
        golden::SSG_EG_MODE_0,
        golden::SSG_EG_MODE_1,
        golden::SSG_EG_MODE_2,
        golden::SSG_EG_MODE_3,
        golden::SSG_EG_MODE_4,
        golden::SSG_EG_MODE_5,
        golden::SSG_EG_MODE_6,
        golden::SSG_EG_MODE_7,
    ];

    for (mode, expected) in golden_ssg.iter().enumerate() {
        let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
        add_ssg_bg_2203(&mut chip);

        write_reg(&mut chip, 0xB0, 0x07);
        for op_offset in [0x00, 0x04, 0x08, 0x0C] {
            write_reg(&mut chip, 0x30 + op_offset, 0x01);
            write_reg(&mut chip, 0x40 + op_offset, 0x10);
            write_reg(&mut chip, 0x50 + op_offset, 0x1F);
            write_reg(&mut chip, 0x60 + op_offset, 0x1A);
            write_reg(&mut chip, 0x70 + op_offset, 0x00);
            write_reg(&mut chip, 0x80 + op_offset, 0x0F);
            write_reg(&mut chip, 0x90 + op_offset, 0x08 | mode as u8);
        }
        write_reg(&mut chip, 0xA4, 0x22);
        write_reg(&mut chip, 0xA0, 0x69);

        key_on_2203(&mut chip, 0);
        let samples = generate_4(&mut chip, expected.len());
        assert_samples_4(&samples, expected);
    }
}

#[test]
fn key_reon_during_release() {
    let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
    add_ssg_bg_2203(&mut chip);

    write_reg(&mut chip, 0xB0, 0x07);
    for op_offset in [0x00, 0x04, 0x08, 0x0C] {
        write_reg(&mut chip, 0x30 + op_offset, 0x01);
        write_reg(&mut chip, 0x40 + op_offset, 0x00);
        write_reg(&mut chip, 0x50 + op_offset, 0x1F);
        write_reg(&mut chip, 0x60 + op_offset, 0x00);
        write_reg(&mut chip, 0x70 + op_offset, 0x00);
        write_reg(&mut chip, 0x80 + op_offset, 0x84);
        write_reg(&mut chip, 0x90 + op_offset, 0x00);
    }
    write_reg(&mut chip, 0xA4, 0x22);
    write_reg(&mut chip, 0xA0, 0x69);

    key_on_2203(&mut chip, 0);
    let sustain = generate_4(&mut chip, golden::REON_SUSTAIN.len());
    assert_samples_4(&sustain, golden::REON_SUSTAIN);

    key_off_2203(&mut chip, 0);
    let release = generate_4(&mut chip, golden::REON_RELEASE.len());
    assert_samples_4(&release, golden::REON_RELEASE);

    key_on_2203(&mut chip, 0);
    let reon = generate_4(&mut chip, golden::REON_AFTER.len());
    assert_samples_4(&reon, golden::REON_AFTER);
}

#[test]
fn frequency_zero_no_oscillation() {
    let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
    add_ssg_bg_2203(&mut chip);
    setup_ym2203_simple_tone(&mut chip, 0, 7, 0);
    write_reg(&mut chip, 0xA4, 0x00);
    write_reg(&mut chip, 0xA0, 0x00);
    key_on_2203(&mut chip, 0);
    let samples = generate_4(&mut chip, golden::FREQ_ZERO.len());
    assert_samples_4(&samples, golden::FREQ_ZERO);
}

#[test]
fn freq_write_order_high_then_low() {
    let mut chip_normal = setup_ym2203(YmfmOpnFidelity::Max);
    add_ssg_bg_2203(&mut chip_normal);
    setup_ym2203_simple_tone(&mut chip_normal, 0, 7, 0);
    key_on_2203(&mut chip_normal, 0);
    let samples_normal = generate_4(&mut chip_normal, golden::FREQ_ORDER_NORMAL.len());
    assert_samples_4(&samples_normal, golden::FREQ_ORDER_NORMAL);

    let mut chip_reverse = setup_ym2203(YmfmOpnFidelity::Max);
    add_ssg_bg_2203(&mut chip_reverse);
    write_reg(&mut chip_reverse, 0xB0, 0x07);
    for op_offset in [0x00, 0x04, 0x08, 0x0C] {
        let reg = op_offset;
        write_reg(&mut chip_reverse, 0x30 + reg, 0x01);
        write_reg(&mut chip_reverse, 0x40 + reg, 0x00);
        write_reg(&mut chip_reverse, 0x50 + reg, 0x1F);
        write_reg(&mut chip_reverse, 0x60 + reg, 0x00);
        write_reg(&mut chip_reverse, 0x70 + reg, 0x00);
        write_reg(&mut chip_reverse, 0x80 + reg, 0x0F);
        write_reg(&mut chip_reverse, 0x90 + reg, 0x00);
    }
    write_reg(&mut chip_reverse, 0xA0, 0x69);
    write_reg(&mut chip_reverse, 0xA4, 0x22);
    write_reg(&mut chip_reverse, 0xA0, 0x69);
    key_on_2203(&mut chip_reverse, 0);
    let samples_reverse = generate_4(&mut chip_reverse, golden::FREQ_ORDER_REVERSE.len());
    assert_samples_4(&samples_reverse, golden::FREQ_ORDER_REVERSE);
}
