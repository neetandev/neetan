mod common;

#[allow(dead_code)]
mod golden {
    include!("golden/ymf278b_fm.rs");
}

use common::harness::*;

const CLOCK: u32 = 33_868_800;

fn check(name: &str, expected: &[[i32; 6]]) {
    assert_samples_6(&ymf278b_fm_scenario(name), expected);
}

#[test]
fn sample_rate() {
    let chip = setup_ymf278b();
    assert_eq!(chip.sample_rate(CLOCK), 44_100);
}

#[test]
fn silence() {
    check("SILENCE", golden::SILENCE);
}

#[test]
fn fm_tone() {
    check("FM_TONE", golden::FM_TONE);
}

#[test]
fn fm_feedback() {
    check("FM_FEEDBACK", golden::FM_FEEDBACK);
}

#[test]
fn fm_outputs_2_3() {
    check("FM_OUTPUTS_2_3", golden::FM_OUTPUTS_2_3);
}

#[test]
fn fm_all_outputs() {
    check("FM_ALL_OUTPUTS", golden::FM_ALL_OUTPUTS);
}

#[test]
fn fm_high_bank() {
    check("FM_HIGH_BANK", golden::FM_HIGH_BANK);
}

#[test]
fn fm_four_op() {
    check("FM_FOUR_OP", golden::FM_FOUR_OP);
}

#[test]
fn fm_waveform_5() {
    check("FM_WAVEFORM_5", golden::FM_WAVEFORM_5);
}

#[test]
fn fm_compatibility_mode() {
    check("FM_COMPATIBILITY_MODE", golden::FM_COMPATIBILITY_MODE);
}

#[test]
fn fm_release() {
    check("FM_RELEASE", golden::FM_RELEASE);
}
