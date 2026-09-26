mod common;

#[allow(dead_code)]
mod golden {
    include!("golden/ym2610_fm.rs");
}

use common::harness::*;
use ymfm_oxide::{Ym2610, Ym2610b, YmfmOpnFidelity};

const YM2610_CLOCK: u32 = 8_000_000;

#[test]
fn sample_rate() {
    let mut chip: Ym2610 = setup_ym2610(YmfmOpnFidelity::Max, false);
    assert_eq!(chip.sample_rate(YM2610_CLOCK), 500_000);
    chip.set_fidelity(YmfmOpnFidelity::Med);
    assert_eq!(chip.sample_rate(YM2610_CLOCK), 55_555);
    chip.set_fidelity(YmfmOpnFidelity::Min);
    assert_eq!(chip.sample_rate(YM2610_CLOCK), 55_555);
}

#[test]
fn silence_after_reset() {
    let mut chip: Ym2610 = setup_ym2610(YmfmOpnFidelity::Max, false);
    let samples = generate_3_2610(&mut chip, golden::SILENCE.len());
    assert_samples_3(&samples, golden::SILENCE);
}

#[test]
fn tone_on_channel_1() {
    let mut chip: Ym2610 = setup_ym2610(YmfmOpnFidelity::Max, false);
    add_ssg_tone_2610(&mut chip);
    setup_ym2610_simple_tone(&mut chip, 1, 7, 0);
    key_on_2610(&mut chip, 1);
    let samples = generate_3_2610(&mut chip, golden::TONE_CHANNEL_1.len());
    assert_samples_3(&samples, golden::TONE_CHANNEL_1);
}

#[test]
fn channel_0_is_silent_on_ym2610() {
    let mut chip: Ym2610 = setup_ym2610(YmfmOpnFidelity::Max, false);
    setup_ym2610_simple_tone(&mut chip, 0, 4, 3);
    key_on_2610(&mut chip, 0);
    let samples = generate_3_2610(&mut chip, golden::CHANNEL_0_YM2610.len());
    assert_samples_3(&samples, golden::CHANNEL_0_YM2610);
    assert!(samples.iter().all(|sample| *sample == [0, 0, 0]));
}

#[test]
fn channel_0_plays_on_ym2610b() {
    let mut chip: Ym2610b = setup_ym2610(YmfmOpnFidelity::Max, false);
    setup_ym2610_simple_tone(&mut chip, 0, 4, 3);
    key_on_2610(&mut chip, 0);
    let samples = generate_3_2610(&mut chip, golden::CHANNEL_0_YM2610B.len());
    assert_samples_3(&samples, golden::CHANNEL_0_YM2610B);
    assert!(samples.iter().any(|sample| sample[0] != 0));
}

#[test]
fn tone_on_channel_4_through_high_port() {
    let mut chip: Ym2610 = setup_ym2610(YmfmOpnFidelity::Max, false);
    setup_ym2610_simple_tone(&mut chip, 4, 7, 0);
    key_on_2610(&mut chip, 4);
    let samples = generate_3_2610(&mut chip, golden::TONE_CHANNEL_4.len());
    assert_samples_3(&samples, golden::TONE_CHANNEL_4);
}

#[test]
fn ssg_at_minimum_fidelity() {
    let mut chip: Ym2610 = setup_ym2610(YmfmOpnFidelity::Min, false);
    add_ssg_tone_2610(&mut chip);
    let samples = generate_3_2610(&mut chip, golden::SSG_MIN.len());
    assert_samples_3(&samples, golden::SSG_MIN);
}

#[test]
fn ssg_and_tone_at_medium_fidelity() {
    let mut chip: Ym2610 = setup_ym2610(YmfmOpnFidelity::Med, false);
    add_ssg_tone_2610(&mut chip);
    setup_ym2610_simple_tone(&mut chip, 2, 7, 0);
    key_on_2610(&mut chip, 2);
    let samples = generate_3_2610(&mut chip, golden::SSG_TONE_MED.len());
    assert_samples_3(&samples, golden::SSG_TONE_MED);
}

#[test]
fn read_data_returns_id_and_missing_io_ports() {
    let mut chip: Ym2610 = setup_ym2610(YmfmOpnFidelity::Max, false);
    write_reg_2610(&mut chip, 0x00, 0x5A);

    chip.write_address(0x00);
    assert_eq!(chip.read_data(), 0x5A);
    chip.write_address(0x0E);
    assert_eq!(chip.read_data(), 0xFF);
    chip.write_address(0x0F);
    assert_eq!(chip.read_data(), 0xFF);
    chip.write_address(0xFF);
    assert_eq!(chip.read_data(), 0x01);
    assert_eq!(chip.read_data_hi(), 0x00);
}

#[test]
fn data_write_reports_busy_duration() {
    let mut chip: Ym2610 = setup_ym2610(YmfmOpnFidelity::Max, false);
    assert_eq!(chip.write_address(0x30), 0);
    assert_eq!(chip.write_data(0x01), 32 * 6);
    assert_eq!(chip.write_address_hi(0x30), 0);
    assert_eq!(chip.write_data_hi(0x01), 32 * 6);
}

#[test]
fn data_write_to_other_bank_is_ignored() {
    let mut chip: Ym2610 = setup_ym2610(YmfmOpnFidelity::Max, false);
    chip.write_address_hi(0x00);
    assert_eq!(chip.write_data(0x10), 0);
    chip.write_address(0x00);
    assert_eq!(chip.write_data_hi(0x10), 0);
    chip.write_address(0x00);
    assert_eq!(chip.read_data(), 0x00);
}
