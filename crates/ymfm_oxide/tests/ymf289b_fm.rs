mod common;

#[allow(dead_code)]
mod golden {
    include!("golden/ymf289b_fm.rs");
}

use common::harness::*;

const CLOCK: u32 = 33_868_800;

fn check(name: &str, expected: &[[i32; 2]]) {
    let mut chip = ymf289b_scenario(name);
    let samples = generate_2_ymf289b(&mut chip, expected.len());
    assert_samples_2(&samples, expected);
}

#[test]
fn sample_rate() {
    let chip = setup_ymf289b();
    assert_eq!(chip.sample_rate(CLOCK), CLOCK / (8 * 36));
}

#[test]
fn silence() {
    check("SILENCE", golden::SILENCE);
}

#[test]
fn tone_output_a() {
    check("TONE_OUTPUT_A", golden::TONE_OUTPUT_A);
}

#[test]
fn tone_output_b() {
    check("TONE_OUTPUT_B", golden::TONE_OUTPUT_B);
}

#[test]
fn tone_outputs_c_and_d() {
    check("TONE_OUTPUTS_C_AND_D", golden::TONE_OUTPUTS_C_AND_D);
}

#[test]
fn high_bank_tone() {
    check("HIGH_BANK_TONE", golden::HIGH_BANK_TONE);
}

#[test]
fn four_operator() {
    check("FOUR_OPERATOR", golden::FOUR_OPERATOR);
}

#[test]
fn waveforms() {
    check("WAVEFORMS", golden::WAVEFORMS);
}

#[test]
fn loud_clamped() {
    check("LOUD_CLAMPED", golden::LOUD_CLAMPED);
}

#[test]
fn register_clear() {
    check("REGISTER_CLEAR", golden::REGISTER_CLEAR);
}

#[test]
fn ymf289b_mode_tone() {
    check("YMF289B_MODE_TONE", golden::YMF289B_MODE_TONE);
}
