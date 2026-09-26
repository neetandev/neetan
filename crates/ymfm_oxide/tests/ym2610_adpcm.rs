mod common;

#[allow(dead_code)]
mod golden {
    include!("golden/ym2610_adpcm.rs");
}

use common::harness::*;
use ymfm_oxide::{Ym2610, YmfmOpnFidelity};

#[test]
fn adpcm_a_plays_from_high_rom_addresses() {
    let mut chip: Ym2610 = setup_ym2610(YmfmOpnFidelity::Min, true);
    start_ym2610_adpcm_a(&mut chip);
    let samples = generate_3_2610(&mut chip, golden::ADPCM_A.len());
    assert_samples_3(&samples, golden::ADPCM_A);
}

#[test]
fn adpcm_a_end_flags_are_cleared_and_masked_by_register_1c() {
    let mut chip: Ym2610 = setup_ym2610(YmfmOpnFidelity::Min, true);
    start_ym2610_adpcm_a(&mut chip);
    assert_eq!(chip.read_status_hi(), 0x00);

    generate_3_2610(&mut chip, golden::ADPCM_A.len() + 2048);
    assert_eq!(chip.read_status_hi(), golden::ADPCM_A_STATUS_END);
    assert_eq!(chip.read_status_hi(), 0x21);

    write_reg_2610(&mut chip, 0x1C, 0x01);
    assert_eq!(chip.read_status_hi(), golden::ADPCM_A_STATUS_MASKED);

    write_reg_2610(&mut chip, 0x1C, 0x00);
    assert_eq!(chip.read_status_hi(), golden::ADPCM_A_STATUS_UNMASKED);
}

#[test]
fn adpcm_b_plays_from_rom_with_record_bit_ignored() {
    let mut chip: Ym2610 = setup_ym2610(YmfmOpnFidelity::Min, true);
    start_ym2610_adpcm_b(&mut chip);
    let samples = generate_3_2610(&mut chip, golden::ADPCM_B.len());
    assert_samples_3(&samples, golden::ADPCM_B);
    assert!(samples.iter().any(|sample| sample[0] != 0));
}

#[test]
fn adpcm_b_end_sets_status_bit_7() {
    let mut chip: Ym2610 = setup_ym2610(YmfmOpnFidelity::Min, true);
    start_ym2610_adpcm_b(&mut chip);
    generate_3_2610(&mut chip, golden::ADPCM_B.len() + 8192);
    assert_eq!(chip.read_status_hi(), golden::ADPCM_B_STATUS_END);
    assert_eq!(chip.read_status_hi(), 0x80);

    write_reg_2610(&mut chip, 0x1C, 0x80);
    assert_eq!(chip.read_status_hi(), golden::ADPCM_B_STATUS_CLEARED);
}

#[test]
fn missing_roms_read_as_zero_bytes() {
    let mut without_roms: Ym2610 = setup_ym2610(YmfmOpnFidelity::Min, true);
    without_roms.clear_adpcm_a_rom();
    without_roms.clear_adpcm_b_rom();
    start_ym2610_adpcm_a(&mut without_roms);
    start_ym2610_adpcm_b(&mut without_roms);

    let mut zero_roms: Ym2610 = setup_ym2610(YmfmOpnFidelity::Min, false);
    zero_roms.set_adpcm_a_rom(vec![0; 0x30000]);
    zero_roms.set_adpcm_b_rom(vec![0; 0x20000]);
    start_ym2610_adpcm_a(&mut zero_roms);
    start_ym2610_adpcm_b(&mut zero_roms);

    let expected = generate_3_2610(&mut zero_roms, 512);
    let actual = generate_3_2610(&mut without_roms, 512);
    assert_samples_3(&actual, &expected);
}

#[test]
fn adpcm_status_does_not_raise_irq() {
    let mut chip: Ym2610 = setup_ym2610(YmfmOpnFidelity::Min, true);
    start_ym2610_adpcm_a(&mut chip);
    start_ym2610_adpcm_b(&mut chip);
    generate_3_2610(&mut chip, 8192);
    assert_ne!(chip.read_status_hi(), 0x00);
    assert!(!chip.irq_asserted());
}
