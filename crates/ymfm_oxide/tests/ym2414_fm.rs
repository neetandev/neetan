mod common;

#[allow(dead_code)]
mod golden {
    include!("golden/ym2414_fm.rs");
}

use common::harness::*;

const CLOCK: u32 = 3_579_545;

fn check(name: &str, expected: &[[i32; 2]]) {
    assert_samples_2(&ym2414_scenario(name), expected);
}

fn check_long(name: &str, expected: &[u64]) {
    let samples = ym2414_long_scenario(name);
    assert_eq!(
        block_checksums(&samples, YM2414_CHECKSUM_BLOCK),
        expected,
        "{name} block checksums differ"
    );
}

#[test]
fn sample_rate() {
    let chip = setup_ym2414();
    assert_eq!(chip.sample_rate(CLOCK), CLOCK / 64);
}

#[test]
fn waveform_write_keeps_the_multiple_and_detune() {
    let mut waveform_last = setup_ym2414();
    select_ym2414_channel(&mut waveform_last, 0);
    setup_ym2414_tone(&mut waveform_last, 0, 7, 0);
    write_reg_ym2414(&mut waveform_last, 0x40, 0x53);
    write_reg_ym2414(&mut waveform_last, 0x40, 0xA4);
    key_ym2414(&mut waveform_last, 0, 7, 0, true);

    let mut waveform_first = setup_ym2414();
    select_ym2414_channel(&mut waveform_first, 0);
    setup_ym2414_tone(&mut waveform_first, 0, 7, 0);
    write_reg_ym2414(&mut waveform_first, 0x40, 0xA4);
    write_reg_ym2414(&mut waveform_first, 0x40, 0x53);
    key_ym2414(&mut waveform_first, 0, 7, 0, true);

    let expected = generate_2_ym2414(&mut waveform_last, 256);
    assert!(expected.iter().any(|sample| *sample != [0; 2]));
    assert_eq!(generate_2_ym2414(&mut waveform_first, 256), expected);
}

#[test]
fn key_on_needs_the_selected_channel() {
    let mut chip = setup_ym2414();
    select_ym2414_channel(&mut chip, 2);
    setup_ym2414_tone(&mut chip, 1, 7, 0);
    key_ym2414(&mut chip, 1, 7, 0, true);
    assert!(
        generate_2_ym2414(&mut chip, 64)
            .iter()
            .all(|sample| *sample == [0; 2])
    );
}

#[test]
fn bus_interface_decodes_the_low_offset_bit() {
    let mut chip = setup_ym2414();
    assert_eq!(chip.write(2, 0x12), 0);
    assert_eq!(chip.write(3, 0xFE), 64);
    assert_eq!(chip.write(0, 0x14), 0);
    assert_eq!(chip.write(1, 0x0A), 64);
    assert_eq!(chip.read(0, true), 0xFF);
    assert_eq!(chip.read(1, true), 0x80);
    chip.timer_expired(1);
    assert_eq!(chip.read(3, false), 0x02);
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
fn waveform_0() {
    check("WAVEFORM_0", golden::WAVEFORM_0);
}

#[test]
fn waveform_1() {
    check("WAVEFORM_1", golden::WAVEFORM_1);
}

#[test]
fn waveform_2() {
    check("WAVEFORM_2", golden::WAVEFORM_2);
}

#[test]
fn waveform_3() {
    check("WAVEFORM_3", golden::WAVEFORM_3);
}

#[test]
fn waveform_4() {
    check("WAVEFORM_4", golden::WAVEFORM_4);
}

#[test]
fn waveform_5() {
    check("WAVEFORM_5", golden::WAVEFORM_5);
}

#[test]
fn waveform_6() {
    check("WAVEFORM_6", golden::WAVEFORM_6);
}

#[test]
fn waveform_7() {
    check("WAVEFORM_7", golden::WAVEFORM_7);
}

#[test]
fn fine_multiple() {
    check("FINE_MULTIPLE", golden::FINE_MULTIPLE);
}

#[test]
fn half_multiple() {
    check("HALF_MULTIPLE", golden::HALF_MULTIPLE);
}

#[test]
fn detune() {
    check("DETUNE", golden::DETUNE);
}

#[test]
fn detune2() {
    check("DETUNE2", golden::DETUNE2);
}

#[test]
fn fixed_frequency_low() {
    check("FIXED_FREQUENCY_LOW", golden::FIXED_FREQUENCY_LOW);
}

#[test]
fn fixed_frequency_mid() {
    check("FIXED_FREQUENCY_MID", golden::FIXED_FREQUENCY_MID);
}

#[test]
fn fixed_frequency_high() {
    check("FIXED_FREQUENCY_HIGH", golden::FIXED_FREQUENCY_HIGH);
}

#[test]
fn fixed_frequency_zero() {
    check("FIXED_FREQUENCY_ZERO", golden::FIXED_FREQUENCY_ZERO);
}

#[test]
fn fixed_and_keyed() {
    check("FIXED_AND_KEYED", golden::FIXED_AND_KEYED);
}

#[test]
fn eg_shift() {
    check("EG_SHIFT", golden::EG_SHIFT);
}

#[test]
fn pan() {
    check("PAN", golden::PAN);
}

#[test]
fn noise() {
    check("NOISE", golden::NOISE);
}

#[test]
fn key_scale_rate() {
    check("KEY_SCALE_RATE", golden::KEY_SCALE_RATE);
}

#[test]
fn key_on_channel_match() {
    check("KEY_ON_CHANNEL_MATCH", golden::KEY_ON_CHANNEL_MATCH);
}

#[test]
fn preset_load() {
    check("PRESET_LOAD", golden::PRESET_LOAD);
}

#[test]
fn release() {
    check_long("RELEASE", golden::RELEASE);
}

#[test]
fn reverb_rates() {
    check_long("REVERB_RATES", golden::REVERB_RATES);
}

#[test]
fn lfo_waveform_0() {
    check_long("LFO_WAVEFORM_0", golden::LFO_WAVEFORM_0);
}

#[test]
fn lfo_waveform_1() {
    check_long("LFO_WAVEFORM_1", golden::LFO_WAVEFORM_1);
}

#[test]
fn lfo_waveform_2() {
    check_long("LFO_WAVEFORM_2", golden::LFO_WAVEFORM_2);
}

#[test]
fn lfo_waveform_3() {
    check_long("LFO_WAVEFORM_3", golden::LFO_WAVEFORM_3);
}

#[test]
fn lfo2_waveform_0() {
    check_long("LFO2_WAVEFORM_0", golden::LFO2_WAVEFORM_0);
}

#[test]
fn lfo2_waveform_1() {
    check_long("LFO2_WAVEFORM_1", golden::LFO2_WAVEFORM_1);
}

#[test]
fn lfo2_waveform_2() {
    check_long("LFO2_WAVEFORM_2", golden::LFO2_WAVEFORM_2);
}

#[test]
fn lfo2_waveform_3() {
    check_long("LFO2_WAVEFORM_3", golden::LFO2_WAVEFORM_3);
}

#[test]
fn lfo_pm_sensitivities() {
    check_long("LFO_PM_SENSITIVITIES", golden::LFO_PM_SENSITIVITIES);
}

#[test]
fn lfo2_pm_sensitivities() {
    check_long("LFO2_PM_SENSITIVITIES", golden::LFO2_PM_SENSITIVITIES);
}

#[test]
fn both_lfos() {
    check_long("BOTH_LFOS", golden::BOTH_LFOS);
}

#[test]
fn lfo_sync() {
    check_long("LFO_SYNC", golden::LFO_SYNC);
}

#[test]
fn lfo2_sync() {
    check_long("LFO2_SYNC", golden::LFO2_SYNC);
}

#[test]
fn fixed_frequency_sweep() {
    check_long("FIXED_FREQUENCY_SWEEP", golden::FIXED_FREQUENCY_SWEEP);
}

#[test]
fn fixed_frequency_with_lfo() {
    check_long("FIXED_FREQUENCY_WITH_LFO", golden::FIXED_FREQUENCY_WITH_LFO);
}

#[test]
fn csm() {
    check_long("CSM", golden::CSM);
}
