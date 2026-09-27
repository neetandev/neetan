mod common;

#[allow(dead_code)]
mod golden {
    include!("golden/ym3438_fm.rs");
}

use common::harness::*;
use ymfm_oxide::{OPN2_VARIANT_YM3438, Ym3438};

const CLOCK: u32 = 7_670_454;

fn check(name: &str, expected: &[[i32; 2]]) {
    let mut chip = opn2_scenario::<OPN2_VARIANT_YM3438>(name);
    let samples = generate_2_opn2(&mut chip, expected.len());
    assert_samples_2(&samples, expected);
}

#[test]
fn sample_rate() {
    // Fixed prescaler 6, 24 operators: native FM rate is clock / 144.
    let chip: Ym3438 = setup_opn2();
    assert_eq!(chip.sample_rate(CLOCK), CLOCK / 144);
}

#[test]
fn data_write_reports_busy_duration() {
    let mut chip: Ym3438 = setup_opn2();
    chip.write_address(0x30);
    assert_eq!(chip.write_data(0x01), 32 * 6);
    chip.write_address_hi(0x30);
    assert_eq!(chip.write_data_hi(0x01), 32 * 6);
}

#[test]
fn data_write_to_other_bank_is_ignored() {
    let mut chip: Ym3438 = setup_opn2();
    chip.write_address_hi(0x30);
    assert_eq!(chip.write_data(0x01), 0);
    chip.write_address(0x30);
    assert_eq!(chip.write_data_hi(0x01), 0);
}

#[test]
fn data_port_reads_zero() {
    let mut chip: Ym3438 = setup_opn2();
    chip.write_address(0x30);
    assert_eq!(chip.read_data(), 0);
}

#[test]
fn status_reports_timer_flags_and_busy() {
    let mut chip: Ym3438 = setup_opn2();
    write_reg_opn2(&mut chip, 0x24, 0xFF);
    write_reg_opn2(&mut chip, 0x25, 0x03);
    write_reg_opn2(&mut chip, 0x26, 0xFF);
    write_reg_opn2(&mut chip, 0x27, 0x0F);
    assert_eq!(chip.read_status(false), 0x00);
    chip.timer_expired(0);
    chip.timer_expired(1);
    assert_eq!(chip.read_status(false), 0x03);
    assert_eq!(chip.read_status(true), 0x83);
}

#[test]
fn silence() {
    check("SILENCE", golden::SILENCE);
}

#[test]
fn single_tone() {
    check("SINGLE_TONE", golden::SINGLE_TONE);
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
fn all_6_channels() {
    check("ALL_6_CHANNELS", golden::ALL_6_CHANNELS);
}

#[test]
fn lfo_off() {
    check("LFO_OFF", golden::LFO_OFF);
}

#[test]
fn lfo_on() {
    check("LFO_ON", golden::LFO_ON);
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
fn dac_mode() {
    check("DAC_MODE", golden::DAC_MODE);
}

#[test]
fn dac_negative() {
    check("DAC_NEGATIVE", golden::DAC_NEGATIVE);
}

#[test]
fn dac_low_bit() {
    check("DAC_LOW_BIT", golden::DAC_LOW_BIT);
}

#[test]
fn dac_left_only() {
    check("DAC_LEFT_ONLY", golden::DAC_LEFT_ONLY);
}

#[test]
fn dac_with_fm() {
    check("DAC_WITH_FM", golden::DAC_WITH_FM);
}

#[test]
fn dac_kept_by_reset() {
    check("DAC_KEPT_BY_RESET", golden::DAC_KEPT_BY_RESET);
}

#[test]
fn csm_key_on() {
    check("CSM_KEY_ON", golden::CSM_KEY_ON);
}
