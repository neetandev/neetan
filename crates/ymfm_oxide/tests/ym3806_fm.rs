mod common;

#[allow(dead_code)]
mod golden {
    include!("golden/ym3806_fm.rs");
}

use common::harness::*;

const CLOCK: u32 = 3_579_545;

fn check(name: &str, expected: &[[i32; 2]]) {
    assert_samples_2(&ym3806_scenario(name), expected);
}

fn check_long(name: &str, expected: &[u64]) {
    let samples = ym3806_long_scenario(name);
    assert_eq!(
        block_checksums(&samples, YM3806_CHECKSUM_BLOCK),
        expected,
        "{name} block checksums differ"
    );
}

#[test]
fn sample_rate() {
    let chip = setup_ym3806();
    assert_eq!(chip.sample_rate(CLOCK), CLOCK / 64);
}

#[test]
fn multiple_write_keeps_the_detune() {
    let mut detune_first = setup_ym3806();
    setup_ym3806_tone(&mut detune_first, 0, 7, 0);
    write_reg_ym3806(&mut detune_first, 0x40, 0x3A);
    write_reg_ym3806(&mut detune_first, 0x40, 0x85);
    key_on_ym3806(&mut detune_first, 0, 0x01);

    let mut multiple_first = setup_ym3806();
    setup_ym3806_tone(&mut multiple_first, 0, 7, 0);
    write_reg_ym3806(&mut multiple_first, 0x40, 0x85);
    write_reg_ym3806(&mut multiple_first, 0x40, 0x3A);
    key_on_ym3806(&mut multiple_first, 0, 0x01);

    let mut plain = setup_ym3806();
    setup_ym3806_tone(&mut plain, 0, 7, 0);
    key_on_ym3806(&mut plain, 0, 0x01);

    let expected = generate_2_ym3806(&mut detune_first, 256);
    assert_eq!(generate_2_ym3806(&mut multiple_first, 256), expected);
    assert_ne!(generate_2_ym3806(&mut plain, 256), expected);
}

#[test]
fn bus_read_returns_status_only_at_offset_0() {
    let mut chip = setup_ym3806();
    assert_eq!(chip.read(0, false), 0x00);
    assert_eq!(chip.read(0, true), 0x80);
    assert_eq!(chip.read(1, true), 0xFF);
    assert_eq!(chip.read(2, false), 0xFF);
}

#[test]
fn silence() {
    check("SILENCE", golden::SILENCE);
}

#[test]
fn algorithm_0() {
    check("ALGORITHM_0", golden::ALGORITHM_0);
}

#[test]
fn algorithm_1() {
    check("ALGORITHM_1", golden::ALGORITHM_1);
}

#[test]
fn algorithm_2() {
    check("ALGORITHM_2", golden::ALGORITHM_2);
}

#[test]
fn algorithm_3() {
    check("ALGORITHM_3", golden::ALGORITHM_3);
}

#[test]
fn algorithm_4() {
    check("ALGORITHM_4", golden::ALGORITHM_4);
}

#[test]
fn algorithm_5() {
    check("ALGORITHM_5", golden::ALGORITHM_5);
}

#[test]
fn algorithm_6() {
    check("ALGORITHM_6", golden::ALGORITHM_6);
}

#[test]
fn algorithm_7() {
    check("ALGORITHM_7", golden::ALGORITHM_7);
}

#[test]
fn feedback() {
    check("FEEDBACK", golden::FEEDBACK);
}

#[test]
fn half_sine() {
    check("HALF_SINE", golden::HALF_SINE);
}

#[test]
fn detune_min() {
    check("DETUNE_MIN", golden::DETUNE_MIN);
}

#[test]
fn detune_zero() {
    check("DETUNE_ZERO", golden::DETUNE_ZERO);
}

#[test]
fn detune_max() {
    check("DETUNE_MAX", golden::DETUNE_MAX);
}

#[test]
fn multiples() {
    check("MULTIPLES", golden::MULTIPLES);
}

#[test]
fn frequency_registers() {
    check("FREQUENCY_REGISTERS", golden::FREQUENCY_REGISTERS);
}

#[test]
fn blocks() {
    check("BLOCKS", golden::BLOCKS);
}

#[test]
fn pan() {
    check("PAN", golden::PAN);
}

#[test]
fn partial_key_on() {
    check("PARTIAL_KEY_ON", golden::PARTIAL_KEY_ON);
}

#[test]
fn key_scale_rate() {
    check("KEY_SCALE_RATE", golden::KEY_SCALE_RATE);
}

#[test]
fn envelope_rates() {
    check("ENVELOPE_RATES", golden::ENVELOPE_RATES);
}

#[test]
fn release() {
    check_long("RELEASE", golden::RELEASE);
}

#[test]
fn reverb() {
    check_long("REVERB", golden::REVERB);
}

#[test]
fn lfo_rate_0() {
    check_long("LFO_RATE_0", golden::LFO_RATE_0);
}

#[test]
fn lfo_rate_1() {
    check_long("LFO_RATE_1", golden::LFO_RATE_1);
}

#[test]
fn lfo_rate_2() {
    check_long("LFO_RATE_2", golden::LFO_RATE_2);
}

#[test]
fn lfo_rate_3() {
    check_long("LFO_RATE_3", golden::LFO_RATE_3);
}

#[test]
fn lfo_rate_4() {
    check_long("LFO_RATE_4", golden::LFO_RATE_4);
}

#[test]
fn lfo_rate_5() {
    check_long("LFO_RATE_5", golden::LFO_RATE_5);
}

#[test]
fn lfo_rate_6() {
    check_long("LFO_RATE_6", golden::LFO_RATE_6);
}

#[test]
fn lfo_rate_7() {
    check_long("LFO_RATE_7", golden::LFO_RATE_7);
}

#[test]
fn lfo_pm_sensitivity_1() {
    check_long("LFO_PM_SENSITIVITY_1", golden::LFO_PM_SENSITIVITY_1);
}

#[test]
fn lfo_pm_sensitivity_2() {
    check_long("LFO_PM_SENSITIVITY_2", golden::LFO_PM_SENSITIVITY_2);
}

#[test]
fn lfo_pm_sensitivity_3() {
    check_long("LFO_PM_SENSITIVITY_3", golden::LFO_PM_SENSITIVITY_3);
}

#[test]
fn lfo_pm_sensitivity_4() {
    check_long("LFO_PM_SENSITIVITY_4", golden::LFO_PM_SENSITIVITY_4);
}

#[test]
fn lfo_pm_sensitivity_5() {
    check_long("LFO_PM_SENSITIVITY_5", golden::LFO_PM_SENSITIVITY_5);
}

#[test]
fn lfo_pm_sensitivity_6() {
    check_long("LFO_PM_SENSITIVITY_6", golden::LFO_PM_SENSITIVITY_6);
}

#[test]
fn lfo_pm_sensitivity_7() {
    check_long("LFO_PM_SENSITIVITY_7", golden::LFO_PM_SENSITIVITY_7);
}

#[test]
fn lfo_am_sensitivity_1() {
    check_long("LFO_AM_SENSITIVITY_1", golden::LFO_AM_SENSITIVITY_1);
}

#[test]
fn lfo_am_sensitivity_2() {
    check_long("LFO_AM_SENSITIVITY_2", golden::LFO_AM_SENSITIVITY_2);
}

#[test]
fn lfo_am_sensitivity_3() {
    check_long("LFO_AM_SENSITIVITY_3", golden::LFO_AM_SENSITIVITY_3);
}

#[test]
fn lfo_disabled() {
    check_long("LFO_DISABLED", golden::LFO_DISABLED);
}

#[test]
fn lfo_restart() {
    check_long("LFO_RESTART", golden::LFO_RESTART);
}
