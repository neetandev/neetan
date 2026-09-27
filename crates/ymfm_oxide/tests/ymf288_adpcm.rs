mod common;

#[allow(dead_code)]
mod golden {
    include!("golden/ymf288_adpcm.rs");
}

use common::harness::*;

fn check(name: &str, expected: &[[i32; 3]]) {
    let mut chip = ymf288_rhythm_scenario(name);
    let samples = generate_3_ymf288(&mut chip, expected.len());
    assert_samples_3(&samples, expected);
}

#[test]
fn rhythm_write_reports_prescaled_busy_duration() {
    let mut chip = ymf288_rhythm_scenario("RHYTHM_BASS_DRUM");
    write_reg_ymf288(&mut chip, 0x20, 0x02);
    assert_eq!(chip.write_address(0x11), 16);
    assert_eq!(chip.write_data(0x3F), 32 * 6);
}

#[test]
fn rhythm_bass_drum() {
    check("RHYTHM_BASS_DRUM", golden::RHYTHM_BASS_DRUM);
}

#[test]
fn rhythm_all() {
    check("RHYTHM_ALL", golden::RHYTHM_ALL);
}

#[test]
fn rhythm_panned() {
    check("RHYTHM_PANNED", golden::RHYTHM_PANNED);
}

#[test]
fn rhythm_with_fm() {
    check("RHYTHM_WITH_FM", golden::RHYTHM_WITH_FM);
}

#[test]
fn rhythm_missing_rom() {
    check("RHYTHM_MISSING_ROM", golden::RHYTHM_MISSING_ROM);
}
