mod common;

use common::harness::*;

#[allow(dead_code)]
mod golden {
    include!("golden/ymf262_fm.rs");
}

#[test]
fn silence_after_reset() {
    let mut chip = setup_ymf262();
    let samples = generate_4_opl3(&mut chip, golden::SILENCE.len());
    assert_samples_4(&samples, golden::SILENCE);
}

#[test]
fn single_tone_2op_additive() {
    let mut chip = setup_ymf262();
    setup_opl3_simple_tone(&mut chip, 0, 1, 0);
    key_on_opl3(&mut chip, 0);
    let samples = generate_4_opl3(&mut chip, golden::SINGLE_TONE_2OP.len());
    assert_samples_4(&samples, golden::SINGLE_TONE_2OP);
}

#[test]
fn single_tone_algo0_fm() {
    let mut chip = setup_ymf262();
    setup_opl3_simple_tone(&mut chip, 0, 0, 0);
    key_on_opl3(&mut chip, 0);
    let samples = generate_4_opl3(&mut chip, golden::SINGLE_TONE_ALGO0_2OP.len());
    assert_samples_4(&samples, golden::SINGLE_TONE_ALGO0_2OP);
}

#[test]
fn feedback_sweep() {
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
        let mut chip = setup_ymf262();
        setup_opl3_simple_tone(&mut chip, 0, 0, fb as u8);
        key_on_opl3(&mut chip, 0);
        let samples = generate_4_opl3(&mut chip, expected.len());
        assert_samples_4(&samples, expected);
    }
}

#[test]
fn waveform_select() {
    let golden_wfs: [&[[i32; 4]]; 8] = [
        golden::WAVEFORM_0,
        golden::WAVEFORM_1,
        golden::WAVEFORM_2,
        golden::WAVEFORM_3,
        golden::WAVEFORM_4,
        golden::WAVEFORM_5,
        golden::WAVEFORM_6,
        golden::WAVEFORM_7,
    ];

    for (wf, expected) in golden_wfs.iter().enumerate() {
        let mut chip = setup_ymf262();
        setup_opl3_simple_tone(&mut chip, 0, 1, 0);
        for op in 0..2u8 {
            let off = opl_op_offset(0, op);
            write_reg_opl3(&mut chip, 0xE0 + off, wf as u8);
        }
        key_on_opl3(&mut chip, 0);
        let samples = generate_4_opl3(&mut chip, expected.len());
        assert_samples_4(&samples, expected);
    }
}

#[test]
fn four_op_algorithms() {
    let golden_4op: [&[[i32; 4]]; 4] = [
        golden::FOUR_OP_ALGO_0,
        golden::FOUR_OP_ALGO_1,
        golden::FOUR_OP_ALGO_2,
        golden::FOUR_OP_ALGO_3,
    ];

    for (algo, expected) in golden_4op.iter().enumerate() {
        let algo = algo as u8;
        let mut chip = setup_ymf262();
        write_reg_opl3_hi(&mut chip, 0x04, 0x01);
        let ch0_algo = algo & 0x01;
        write_reg_opl3(&mut chip, 0xC0, ch0_algo | 0x30);
        let ch3_algo = (algo >> 1) & 0x01;
        write_reg_opl3(&mut chip, 0xC3, ch3_algo | 0x30);
        for op in 0..2u8 {
            let off = opl_op_offset(0, op);
            write_reg_opl3(&mut chip, 0x20 + off, 0x21);
            write_reg_opl3(&mut chip, 0x40 + off, 0x00);
            write_reg_opl3(&mut chip, 0x60 + off, 0xF0);
            write_reg_opl3(&mut chip, 0x80 + off, 0x0F);
            write_reg_opl3(&mut chip, 0xE0 + off, 0x00);
        }
        for op in 0..2u8 {
            let off = opl_op_offset(3, op);
            write_reg_opl3(&mut chip, 0x20 + off, 0x21);
            write_reg_opl3(&mut chip, 0x40 + off, 0x00);
            write_reg_opl3(&mut chip, 0x60 + off, 0xF0);
            write_reg_opl3(&mut chip, 0x80 + off, 0x0F);
            write_reg_opl3(&mut chip, 0xE0 + off, 0x00);
        }
        write_reg_opl3(&mut chip, 0xA0, 0x41);
        write_reg_opl3(&mut chip, 0xB0, 0x31);
        let samples = generate_4_opl3(&mut chip, expected.len());
        assert_samples_4(&samples, expected);
    }
}

#[test]
fn four_op_all_pairs() {
    let mut chip = setup_ymf262();
    write_reg_opl3_hi(&mut chip, 0x04, 0x3F);
    write_reg_opl3(&mut chip, 0xC0, 0x31);
    write_reg_opl3(&mut chip, 0xC3, 0x31);
    for ch_base in [0u8, 3] {
        for op in 0..2u8 {
            let off = opl_op_offset(ch_base, op);
            write_reg_opl3(&mut chip, 0x20 + off, 0x21);
            write_reg_opl3(&mut chip, 0x40 + off, 0x00);
            write_reg_opl3(&mut chip, 0x60 + off, 0xF0);
            write_reg_opl3(&mut chip, 0x80 + off, 0x0F);
            write_reg_opl3(&mut chip, 0xE0 + off, 0x00);
        }
    }
    write_reg_opl3(&mut chip, 0xA0, 0x41);
    write_reg_opl3(&mut chip, 0xB0, 0x31);
    let samples = generate_4_opl3(&mut chip, golden::FOUR_OP_ALL_PAIRS.len());
    assert_samples_4(&samples, golden::FOUR_OP_ALL_PAIRS);
}

#[test]
fn low_bank_3_channels() {
    let mut chip = setup_ymf262();
    for ch in 0..3u8 {
        setup_opl3_simple_tone(&mut chip, ch, 1, 0);
        write_reg_opl3(&mut chip, 0xA0 + ch, 0x41 + ch * 8);
        key_on_opl3(&mut chip, ch);
    }
    let samples = generate_4_opl3(&mut chip, golden::LOW_BANK_3CH.len());
    assert_samples_4(&samples, golden::LOW_BANK_3CH);
}

#[test]
fn high_bank_3_channels() {
    let mut chip = setup_ymf262();
    for ch in 9..12u8 {
        setup_opl3_simple_tone(&mut chip, ch, 1, 0);
        let ch_off = ch - 9;
        write_reg_opl3_hi(&mut chip, 0xA0 + ch_off, 0x41 + ch_off * 8);
        key_on_opl3(&mut chip, ch);
    }
    let samples = generate_4_opl3(&mut chip, golden::HIGH_BANK_3CH.len());
    assert_samples_4(&samples, golden::HIGH_BANK_3CH);
}

#[test]
fn all_18_channels() {
    let mut chip = setup_ymf262();
    for ch in 0..18u8 {
        setup_opl3_simple_tone(&mut chip, ch, 1, 0);
        if ch < 9 {
            write_reg_opl3(&mut chip, 0xA0 + ch, 0x20 + ch * 5);
        } else {
            let ch_off = ch - 9;
            write_reg_opl3_hi(&mut chip, 0xA0 + ch_off, 0x20 + ch_off * 5);
        }
        key_on_opl3(&mut chip, ch);
    }
    let samples = generate_4_opl3(&mut chip, golden::ALL_18_CHANNELS.len());
    assert_samples_4(&samples, golden::ALL_18_CHANNELS);
}

#[test]
fn full_adsr_envelope_cycle() {
    let mut chip = setup_ymf262();
    setup_opl3_simple_tone(&mut chip, 0, 1, 0);
    key_on_opl3(&mut chip, 0);
    let sustain = generate_4_opl3(&mut chip, golden::ADSR_SUSTAIN.len());
    assert_samples_4(&sustain, golden::ADSR_SUSTAIN);

    key_off_opl3(&mut chip, 0);
    let release = generate_4_opl3(&mut chip, golden::ADSR_RELEASE.len());
    assert_samples_4(&release, golden::ADSR_RELEASE);
}

#[test]
fn rhythm_mode() {
    let mut chip = setup_ymf262();
    for ch in 6..9u8 {
        setup_opl3_simple_tone(&mut chip, ch, 1, 0);
    }
    write_reg_opl3(&mut chip, 0xBD, 0x3F);
    let samples = generate_4_opl3(&mut chip, golden::RHYTHM_MODE.len());
    assert_samples_4(&samples, golden::RHYTHM_MODE);
}

#[test]
fn frequency_zero() {
    let mut chip = setup_ymf262();
    setup_opl3_simple_tone(&mut chip, 0, 1, 0);
    write_reg_opl3(&mut chip, 0xA0, 0x00);
    write_reg_opl3(&mut chip, 0xB0, 0x20);
    let samples = generate_4_opl3(&mut chip, golden::FREQ_ZERO.len());
    assert_samples_4(&samples, golden::FREQ_ZERO);
}

#[test]
fn new_mode_off_vs_on() {
    // NEW mode off
    {
        let mut chip = ymfm_oxide::Ymf262::new();
        chip.reset();
        setup_opl3_simple_tone(&mut chip, 0, 1, 0);
        key_on_opl3(&mut chip, 0);
        let samples = generate_4_opl3(&mut chip, golden::NEW_MODE_OFF.len());
        assert_samples_4(&samples, golden::NEW_MODE_OFF);
    }

    // NEW mode on
    {
        let mut chip = setup_ymf262();
        setup_opl3_simple_tone(&mut chip, 0, 1, 0);
        key_on_opl3(&mut chip, 0);
        let samples = generate_4_opl3(&mut chip, golden::NEW_MODE_ON.len());
        assert_samples_4(&samples, golden::NEW_MODE_ON);
    }
}
