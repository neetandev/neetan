mod common;

#[allow(dead_code)]
mod golden {
    include!("golden/ymf288_fm.rs");
}

use common::harness::*;
use ymfm_oxide::YmfmOpnFidelity;

const CLOCK: u32 = 33_868_800;

fn check(name: &str, expected: &[[i32; 3]]) {
    let mut chip = ymf288_fm_scenario(name);
    let samples = generate_3_ymf288(&mut chip, expected.len());
    assert_samples_3(&samples, expected);
}

#[test]
fn sample_rate_follows_fidelity() {
    assert_eq!(
        setup_ymf288(YmfmOpnFidelity::Max).sample_rate(CLOCK),
        CLOCK / 16
    );
    assert_eq!(
        setup_ymf288(YmfmOpnFidelity::Med).sample_rate(CLOCK),
        CLOCK / 144
    );
    assert_eq!(
        setup_ymf288(YmfmOpnFidelity::Min).sample_rate(CLOCK),
        CLOCK / 144
    );
    assert_eq!(
        setup_ymf288(YmfmOpnFidelity::Max).ssg_effective_clock(CLOCK),
        CLOCK / 4
    );
}

#[test]
fn silence() {
    check("SILENCE", golden::SILENCE);
}

#[test]
fn tone_ch0() {
    check("TONE_CH0", golden::TONE_CH0);
}

#[test]
fn three_channel_mode() {
    check("THREE_CHANNEL_MODE", golden::THREE_CHANNEL_MODE);
}

#[test]
fn six_channel_mode() {
    check("SIX_CHANNEL_MODE", golden::SIX_CHANNEL_MODE);
}

#[test]
fn algo_0() {
    check("ALGO_0", golden::ALGO_0);
}

#[test]
fn algo_1() {
    check("ALGO_1", golden::ALGO_1);
}

#[test]
fn algo_2() {
    check("ALGO_2", golden::ALGO_2);
}

#[test]
fn algo_3() {
    check("ALGO_3", golden::ALGO_3);
}

#[test]
fn algo_4() {
    check("ALGO_4", golden::ALGO_4);
}

#[test]
fn algo_5() {
    check("ALGO_5", golden::ALGO_5);
}

#[test]
fn algo_6() {
    check("ALGO_6", golden::ALGO_6);
}

#[test]
fn algo_7() {
    check("ALGO_7", golden::ALGO_7);
}

#[test]
fn pan_left() {
    check("PAN_LEFT", golden::PAN_LEFT);
}

#[test]
fn pan_right() {
    check("PAN_RIGHT", golden::PAN_RIGHT);
}

#[test]
fn lfo_on() {
    check("LFO_ON", golden::LFO_ON);
}

#[test]
fn ssg_tone() {
    check("SSG_TONE", golden::SSG_TONE);
}

#[test]
fn ssg_envelope() {
    check("SSG_ENVELOPE", golden::SSG_ENVELOPE);
}

#[test]
fn fidelity_min() {
    check("FIDELITY_MIN", golden::FIDELITY_MIN);
}

#[test]
fn fidelity_med() {
    check("FIDELITY_MED", golden::FIDELITY_MED);
}

#[test]
fn ymf288_mode_tone() {
    check("YMF288_MODE_TONE", golden::YMF288_MODE_TONE);
}

#[test]
fn csm_ignored() {
    check("CSM_IGNORED", golden::CSM_IGNORED);
}

#[test]
fn prescaler_ignored() {
    check("PRESCALER_IGNORED", golden::PRESCALER_IGNORED);
}
