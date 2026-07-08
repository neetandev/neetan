mod common;

use common::harness::*;

#[allow(dead_code)]
mod golden {
    include!("golden/ym2151_fm.rs");
}

#[test]
fn silence_after_reset() {
    let mut chip = setup_ym2151();
    let samples = generate_2_opm(&mut chip, golden::SILENCE.len());
    assert_samples_2(&samples, golden::SILENCE);
}

#[test]
fn single_tone_algo7() {
    let mut chip = setup_ym2151();
    setup_ym2151_simple_tone(&mut chip, 0, 7, 0);
    key_on_ym2151(&mut chip, 0);
    let samples = generate_2_opm(&mut chip, golden::TONE_ALGO7.len());
    assert_samples_2(&samples, golden::TONE_ALGO7);
}

#[test]
fn tone_algo4_with_feedback() {
    let mut chip = setup_ym2151();
    setup_ym2151_simple_tone(&mut chip, 0, 4, 5);
    key_on_ym2151(&mut chip, 0);
    let samples = generate_2_opm(&mut chip, golden::TONE_ALGO4_FB.len());
    assert_samples_2(&samples, golden::TONE_ALGO4_FB);
}

#[test]
fn lfo_phase_modulation() {
    let mut chip = setup_ym2151();
    write_reg_ym2151(&mut chip, 0x18, 0x80); // LFO frequency
    write_reg_ym2151(&mut chip, 0x19, 0xFF); // PM depth (bit7=1)
    write_reg_ym2151(&mut chip, 0x1B, 0x00); // LFO waveform: sawtooth
    setup_ym2151_simple_tone(&mut chip, 0, 7, 0);
    write_reg_ym2151(&mut chip, 0x38, 0x70); // PMS=7, AMS=0
    key_on_ym2151(&mut chip, 0);
    let samples = generate_2_opm(&mut chip, golden::LFO_PM.len());
    assert_samples_2(&samples, golden::LFO_PM);
}

#[test]
fn noise_on_channel_7() {
    let mut chip = setup_ym2151();
    write_reg_ym2151(&mut chip, 0x0F, 0x90); // noise enable + frequency
    setup_ym2151_simple_tone(&mut chip, 7, 7, 0);
    key_on_ym2151(&mut chip, 7);
    let samples = generate_2_opm(&mut chip, golden::NOISE.len());
    assert_samples_2(&samples, golden::NOISE);
}
