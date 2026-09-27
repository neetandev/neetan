mod common;

#[allow(dead_code)]
mod golden {
    include!("golden/ym2413_rom_fm.rs");
}

use common::harness::*;
use ymfm_oxide::{OPLL_VARIANT_YM2413, Ym2413};

fn check(name: &str, expected: &[[i32; 2]]) {
    let mut chip = opll_rom_scenario::<OPLL_VARIANT_YM2413>(name);
    generate_2_opll(&mut chip, OPLL_ROM_WARM_UP);
    let samples = generate_2_opll(&mut chip, expected.len());
    assert_samples_2(&samples, expected);
}

#[test]
fn register_writes_report_busy_duration() {
    let mut chip = Ym2413::new();
    chip.reset();
    assert_eq!(chip.write_address(0x10), 12);
    assert_eq!(chip.write_data(0x80), 84);
}

#[test]
fn reset_keeps_the_address_latch() {
    let mut latched = Ym2413::new();
    latched.reset();
    latched.write_address(0x20);
    latched.reset();
    latched.write_data(0x15);
    write_reg_opll(&mut latched, 0x30, 0x40);
    write_reg_opll(&mut latched, 0x10, 0x80);

    let mut direct = Ym2413::new();
    direct.reset();
    write_reg_opll(&mut direct, 0x20, 0x15);
    write_reg_opll(&mut direct, 0x30, 0x40);
    write_reg_opll(&mut direct, 0x10, 0x80);

    let expected = generate_2_opll(&mut direct, 1024);
    assert!(expected.iter().any(|sample| sample[0] != 0));
    assert_eq!(generate_2_opll(&mut latched, 1024), expected);
}

#[test]
fn silence() {
    check("SILENCE", golden::SILENCE);
}

#[test]
fn instrument_1() {
    check("INSTRUMENT_1", golden::INSTRUMENT_1);
}

#[test]
fn instrument_2() {
    check("INSTRUMENT_2", golden::INSTRUMENT_2);
}

#[test]
fn instrument_3() {
    check("INSTRUMENT_3", golden::INSTRUMENT_3);
}

#[test]
fn instrument_4() {
    check("INSTRUMENT_4", golden::INSTRUMENT_4);
}

#[test]
fn instrument_5() {
    check("INSTRUMENT_5", golden::INSTRUMENT_5);
}

#[test]
fn instrument_6() {
    check("INSTRUMENT_6", golden::INSTRUMENT_6);
}

#[test]
fn instrument_7() {
    check("INSTRUMENT_7", golden::INSTRUMENT_7);
}

#[test]
fn instrument_8() {
    check("INSTRUMENT_8", golden::INSTRUMENT_8);
}

#[test]
fn instrument_9() {
    check("INSTRUMENT_9", golden::INSTRUMENT_9);
}

#[test]
fn instrument_10() {
    check("INSTRUMENT_10", golden::INSTRUMENT_10);
}

#[test]
fn instrument_11() {
    check("INSTRUMENT_11", golden::INSTRUMENT_11);
}

#[test]
fn instrument_12() {
    check("INSTRUMENT_12", golden::INSTRUMENT_12);
}

#[test]
fn instrument_13() {
    check("INSTRUMENT_13", golden::INSTRUMENT_13);
}

#[test]
fn instrument_14() {
    check("INSTRUMENT_14", golden::INSTRUMENT_14);
}

#[test]
fn instrument_15() {
    check("INSTRUMENT_15", golden::INSTRUMENT_15);
}

#[test]
fn user_instrument() {
    check("USER_INSTRUMENT", golden::USER_INSTRUMENT);
}

#[test]
fn rhythm_bass_drum() {
    check("RHYTHM_BASS_DRUM", golden::RHYTHM_BASS_DRUM);
}

#[test]
fn rhythm_all() {
    check("RHYTHM_ALL", golden::RHYTHM_ALL);
}
