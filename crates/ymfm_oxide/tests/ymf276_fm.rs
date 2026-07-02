mod common;

#[allow(dead_code)]
mod golden {
    include!("golden/ymf276_fm.rs");
}

use common::harness::*;

const YMF276_CLOCK: u32 = 8_000_000;

#[test]
fn sample_rate() {
    // Fixed prescaler 6, 24 operators: native FM rate is clock / 144.
    let chip = setup_ymf276();
    assert_eq!(chip.sample_rate(YMF276_CLOCK), YMF276_CLOCK / 144);
}

#[test]
fn silence_after_reset() {
    let mut chip = setup_ymf276();
    let samples = generate_2(&mut chip, golden::SILENCE.len());
    assert_samples_2(&samples, golden::SILENCE);
}

#[test]
fn single_tone() {
    let mut chip = setup_ymf276();
    setup_ymf276_simple_tone(&mut chip, 0, 7, 0);
    key_on_ymf276(&mut chip, 0);
    let samples = generate_2(&mut chip, golden::SINGLE_TONE.len());
    assert_samples_2(&samples, golden::SINGLE_TONE);
}

#[test]
fn all_algorithms() {
    let goldens: [&[[i32; 2]]; 8] = [
        golden::ALGO_0,
        golden::ALGO_1,
        golden::ALGO_2,
        golden::ALGO_3,
        golden::ALGO_4,
        golden::ALGO_5,
        golden::ALGO_6,
        golden::ALGO_7,
    ];
    for (algo, expected) in goldens.iter().enumerate() {
        let mut chip = setup_ymf276();
        write_reg_ymf276(&mut chip, 0xB0, algo as u8);
        for (op_offset, tl) in [(0x00, 0x20), (0x04, 0x20), (0x08, 0x20), (0x0C, 0x00)] {
            write_reg_ymf276(&mut chip, 0x30 + op_offset, 0x01);
            write_reg_ymf276(&mut chip, 0x40 + op_offset, tl);
            write_reg_ymf276(&mut chip, 0x50 + op_offset, 0x1F);
            write_reg_ymf276(&mut chip, 0x60 + op_offset, 0x00);
            write_reg_ymf276(&mut chip, 0x70 + op_offset, 0x00);
            write_reg_ymf276(&mut chip, 0x80 + op_offset, 0x0F);
            write_reg_ymf276(&mut chip, 0x90 + op_offset, 0x00);
        }
        write_reg_ymf276(&mut chip, 0xA4, 0x22);
        write_reg_ymf276(&mut chip, 0xA0, 0x69);
        write_reg_ymf276(&mut chip, 0xB4, 0xC0);
        key_on_ymf276(&mut chip, 0);
        let samples = generate_2(&mut chip, expected.len());
        assert_samples_2(&samples, expected);
    }
}

#[test]
fn all_6_channels() {
    let mut chip = setup_ymf276();
    let freqs: [(u8, u8); 6] = [
        (0x22, 0x69),
        (0x24, 0x80),
        (0x26, 0xD5),
        (0x22, 0x40),
        (0x28, 0x50),
        (0x2A, 0xA0),
    ];
    for ch in 0..6u8 {
        setup_ymf276_simple_tone(&mut chip, ch, 7, 0);
        let (hi, lo) = freqs[ch as usize];
        if ch < 3 {
            write_reg_ymf276(&mut chip, 0xA4 + ch, hi);
            write_reg_ymf276(&mut chip, 0xA0 + ch, lo);
        } else {
            write_reg_ymf276_hi(&mut chip, 0xA4 + (ch - 3), hi);
            write_reg_ymf276_hi(&mut chip, 0xA0 + (ch - 3), lo);
        }
        key_on_ymf276(&mut chip, ch);
    }
    let samples = generate_2(&mut chip, golden::ALL_6_CHANNELS.len());
    assert_samples_2(&samples, golden::ALL_6_CHANNELS);
}

#[test]
fn lfo_off_and_on() {
    for (lfo, expected) in [(false, golden::LFO_OFF), (true, golden::LFO_ON)] {
        let mut chip = setup_ymf276();
        write_reg_ymf276(&mut chip, 0xB0, 0x00);
        for (op_offset, tl) in [(0x00, 0x20), (0x04, 0x20), (0x08, 0x20), (0x0C, 0x00)] {
            write_reg_ymf276(&mut chip, 0x30 + op_offset, 0x01);
            write_reg_ymf276(&mut chip, 0x40 + op_offset, tl);
            write_reg_ymf276(&mut chip, 0x50 + op_offset, 0x1F);
            write_reg_ymf276(&mut chip, 0x60 + op_offset, 0x00);
            write_reg_ymf276(&mut chip, 0x70 + op_offset, 0x00);
            write_reg_ymf276(&mut chip, 0x80 + op_offset, 0x0F);
            write_reg_ymf276(&mut chip, 0x90 + op_offset, 0x00);
        }
        write_reg_ymf276(&mut chip, 0xA4, 0x22);
        write_reg_ymf276(&mut chip, 0xA0, 0x69);
        if lfo {
            write_reg_ymf276(&mut chip, 0x22, 0x08);
            write_reg_ymf276(&mut chip, 0xB4, 0xC0 | 0x27);
            write_reg_ymf276(&mut chip, 0x60, 0x80);
        } else {
            write_reg_ymf276(&mut chip, 0xB4, 0xC0);
        }
        key_on_ymf276(&mut chip, 0);
        let samples = generate_2(&mut chip, expected.len());
        assert_samples_2(&samples, expected);
    }
}

#[test]
fn dac_mode() {
    let mut chip = setup_ymf276();
    write_reg_ymf276(&mut chip, 0x2B, 0x80); // DAC enable
    write_reg_ymf276(&mut chip, 0x2A, 0xC0); // DAC data (positive)
    write_reg_ymf276_hi(&mut chip, 0xB6, 0xC0); // channel 6 pan L+R
    let samples = generate_2(&mut chip, golden::DAC_MODE.len());
    assert_samples_2(&samples, golden::DAC_MODE);
    // With the DAC enabled and a fixed positive value, the output is a nonzero
    // constant on both channels.
    assert!(samples[0][0] != 0 && samples[0][0] == samples[0][1]);
}
