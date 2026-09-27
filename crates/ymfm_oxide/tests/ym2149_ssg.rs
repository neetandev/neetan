mod common;

#[allow(dead_code)]
mod golden {
    include!("golden/ym2149_ssg.rs");
}

use common::harness::*;

fn check(name: &str, expected: &[[i32; 3]]) {
    let samples = ym2149_scenario(name);
    assert_samples_3(&samples, expected);
}

#[test]
fn silence() {
    check("SILENCE", golden::SILENCE);
}

#[test]
fn tone_a() {
    check("TONE_A", golden::TONE_A);
}

#[test]
fn three_channels() {
    check("THREE_CHANNELS", golden::THREE_CHANNELS);
}

#[test]
fn tone_period_zero() {
    check("TONE_PERIOD_ZERO", golden::TONE_PERIOD_ZERO);
}

#[test]
fn noise() {
    check("NOISE", golden::NOISE);
}

#[test]
fn noise_period_zero() {
    check("NOISE_PERIOD_ZERO", golden::NOISE_PERIOD_ZERO);
}

#[test]
fn tone_and_noise() {
    check("TONE_AND_NOISE", golden::TONE_AND_NOISE);
}

#[test]
fn amplitudes() {
    check("AMPLITUDES", golden::AMPLITUDES);
}

#[test]
fn envelope_shape_0() {
    check("ENVELOPE_SHAPE_0", golden::ENVELOPE_SHAPE_0);
}

#[test]
fn envelope_shape_1() {
    check("ENVELOPE_SHAPE_1", golden::ENVELOPE_SHAPE_1);
}

#[test]
fn envelope_shape_2() {
    check("ENVELOPE_SHAPE_2", golden::ENVELOPE_SHAPE_2);
}

#[test]
fn envelope_shape_3() {
    check("ENVELOPE_SHAPE_3", golden::ENVELOPE_SHAPE_3);
}

#[test]
fn envelope_shape_4() {
    check("ENVELOPE_SHAPE_4", golden::ENVELOPE_SHAPE_4);
}

#[test]
fn envelope_shape_5() {
    check("ENVELOPE_SHAPE_5", golden::ENVELOPE_SHAPE_5);
}

#[test]
fn envelope_shape_6() {
    check("ENVELOPE_SHAPE_6", golden::ENVELOPE_SHAPE_6);
}

#[test]
fn envelope_shape_7() {
    check("ENVELOPE_SHAPE_7", golden::ENVELOPE_SHAPE_7);
}

#[test]
fn envelope_shape_8() {
    check("ENVELOPE_SHAPE_8", golden::ENVELOPE_SHAPE_8);
}

#[test]
fn envelope_shape_9() {
    check("ENVELOPE_SHAPE_9", golden::ENVELOPE_SHAPE_9);
}

#[test]
fn envelope_shape_a() {
    check("ENVELOPE_SHAPE_A", golden::ENVELOPE_SHAPE_A);
}

#[test]
fn envelope_shape_b() {
    check("ENVELOPE_SHAPE_B", golden::ENVELOPE_SHAPE_B);
}

#[test]
fn envelope_shape_c() {
    check("ENVELOPE_SHAPE_C", golden::ENVELOPE_SHAPE_C);
}

#[test]
fn envelope_shape_d() {
    check("ENVELOPE_SHAPE_D", golden::ENVELOPE_SHAPE_D);
}

#[test]
fn envelope_shape_e() {
    check("ENVELOPE_SHAPE_E", golden::ENVELOPE_SHAPE_E);
}

#[test]
fn envelope_shape_f() {
    check("ENVELOPE_SHAPE_F", golden::ENVELOPE_SHAPE_F);
}

#[test]
fn envelope_period_zero() {
    check("ENVELOPE_PERIOD_ZERO", golden::ENVELOPE_PERIOD_ZERO);
}

#[test]
fn envelope_tone() {
    check("ENVELOPE_TONE", golden::ENVELOPE_TONE);
}

#[test]
fn envelope_restart() {
    check("ENVELOPE_RESTART", golden::ENVELOPE_RESTART);
}

#[test]
fn bus_interface() {
    check("BUS_INTERFACE", golden::BUS_INTERFACE);
}
