mod common;

use common::{harness::*, signals::*};
use ymfm_oxide::Y8950;

#[allow(dead_code)]
mod golden {
    include!("golden/y8950_adpcm.rs");
}

#[test]
fn adpcm_b_playback() {
    let adpcm_data = create_y8950_adpcm_data();
    let mut chip = setup_y8950_with_adpcm_data(adpcm_data);
    chip.reset();
    write_reg_y8950(&mut chip, 0x09, 0x20);
    write_reg_y8950(&mut chip, 0x0A, 0x00);
    write_reg_y8950(&mut chip, 0x0B, 0x23);
    write_reg_y8950(&mut chip, 0x0C, 0x00);
    write_reg_y8950(&mut chip, 0x10, 0x49);
    write_reg_y8950(&mut chip, 0x11, 0x0C);
    write_reg_y8950(&mut chip, 0x12, 0xFF);
    write_reg_y8950(&mut chip, 0x08, 0xC0);
    write_reg_y8950(&mut chip, 0x07, 0xA0);
    let samples = generate_1_y8950(&mut chip, golden::ADPCM_B_PLAYBACK.len());
    assert_samples_1(&samples, golden::ADPCM_B_PLAYBACK);
}

#[test]
fn adpcm_b_start_stop() {
    let adpcm_data = create_y8950_adpcm_data();
    let mut chip = setup_y8950_with_adpcm_data(adpcm_data);
    chip.reset();
    write_reg_y8950(&mut chip, 0x09, 0x20);
    write_reg_y8950(&mut chip, 0x0A, 0x00);
    write_reg_y8950(&mut chip, 0x0B, 0x23);
    write_reg_y8950(&mut chip, 0x0C, 0x00);
    write_reg_y8950(&mut chip, 0x10, 0x49);
    write_reg_y8950(&mut chip, 0x11, 0x0C);
    write_reg_y8950(&mut chip, 0x12, 0xFF);
    write_reg_y8950(&mut chip, 0x08, 0xC0);
    write_reg_y8950(&mut chip, 0x07, 0xA0);
    let on = generate_1_y8950(&mut chip, golden::ADPCM_B_ON.len());
    assert_samples_1(&on, golden::ADPCM_B_ON);

    write_reg_y8950(&mut chip, 0x07, 0x01);
    let off = generate_1_y8950(&mut chip, golden::ADPCM_B_OFF.len());
    assert_samples_1(&off, golden::ADPCM_B_OFF);
}

#[test]
fn adpcm_b_different_rates() {
    // Low rate
    {
        let adpcm_data = create_y8950_adpcm_data();
        let mut chip = setup_y8950_with_adpcm_data(adpcm_data);
        chip.reset();
        write_reg_y8950(&mut chip, 0x09, 0x20);
        write_reg_y8950(&mut chip, 0x0A, 0x00);
        write_reg_y8950(&mut chip, 0x0B, 0x23);
        write_reg_y8950(&mut chip, 0x0C, 0x00);
        write_reg_y8950(&mut chip, 0x10, 0x00);
        write_reg_y8950(&mut chip, 0x11, 0x01);
        write_reg_y8950(&mut chip, 0x12, 0xFF);
        write_reg_y8950(&mut chip, 0x08, 0xC0);
        write_reg_y8950(&mut chip, 0x07, 0xA0);
        let samples = generate_1_y8950(&mut chip, golden::ADPCM_B_RATE_LOW.len());
        assert_samples_1(&samples, golden::ADPCM_B_RATE_LOW);
    }

    // High rate
    {
        let adpcm_data = create_y8950_adpcm_data();
        let mut chip = setup_y8950_with_adpcm_data(adpcm_data);
        chip.reset();
        write_reg_y8950(&mut chip, 0x09, 0x20);
        write_reg_y8950(&mut chip, 0x0A, 0x00);
        write_reg_y8950(&mut chip, 0x0B, 0x23);
        write_reg_y8950(&mut chip, 0x0C, 0x00);
        write_reg_y8950(&mut chip, 0x10, 0xFF);
        write_reg_y8950(&mut chip, 0x11, 0xFF);
        write_reg_y8950(&mut chip, 0x12, 0xFF);
        write_reg_y8950(&mut chip, 0x08, 0xC0);
        write_reg_y8950(&mut chip, 0x07, 0xA0);
        let samples = generate_1_y8950(&mut chip, golden::ADPCM_B_RATE_HIGH.len());
        assert_samples_1(&samples, golden::ADPCM_B_RATE_HIGH);
    }
}

#[test]
fn adpcm_b_end_of_sample() {
    let adpcm_data = create_y8950_adpcm_data();
    let mut chip = setup_y8950_with_adpcm_data(adpcm_data);
    chip.reset();
    write_reg_y8950(&mut chip, 0x09, 0x20);
    write_reg_y8950(&mut chip, 0x0A, 0x00);
    write_reg_y8950(&mut chip, 0x0B, 0x20);
    write_reg_y8950(&mut chip, 0x0C, 0x00);
    write_reg_y8950(&mut chip, 0x10, 0xFF);
    write_reg_y8950(&mut chip, 0x11, 0xFF);
    write_reg_y8950(&mut chip, 0x12, 0xFF);
    write_reg_y8950(&mut chip, 0x08, 0xC0);
    write_reg_y8950(&mut chip, 0x07, 0xA0);
    let samples = generate_1_y8950(&mut chip, golden::ADPCM_B_SHORT_SAMPLE.len());
    assert_samples_1(&samples, golden::ADPCM_B_SHORT_SAMPLE);
}

#[test]
fn external_write_updates_adpcm_memory() {
    let mut chip = Y8950::new();
    chip.reset();
    chip.set_adpcm_memory(vec![0; 256 * 1024]);
    chip.take_signals();
    write_reg_y8950(&mut chip, 0x07, 0x01);
    write_reg_y8950(&mut chip, 0x08, 0xC0);
    write_reg_y8950(&mut chip, 0x09, 0x00);
    write_reg_y8950(&mut chip, 0x0A, 0x00);
    write_reg_y8950(&mut chip, 0x0B, 0xFF);
    write_reg_y8950(&mut chip, 0x0C, 0xFF);
    write_reg_y8950(&mut chip, 0x07, 0x60);
    write_reg_y8950(&mut chip, 0x0F, 0xAB);
    assert_eq!(chip.adpcm_memory()[0], 0xAB);
    let samples = generate_1_y8950(&mut chip, golden::EXTERNAL_WRITE.len());
    assert_samples_1(&samples, golden::EXTERNAL_WRITE);
}

#[test]
#[should_panic(expected = "ADPCM memory must not be empty")]
fn empty_adpcm_memory_panics() {
    let mut chip = Y8950::new();
    chip.set_adpcm_memory(Vec::new());
}

#[test]
fn external_read_uses_adpcm_memory() {
    let adpcm_data = create_y8950_adpcm_data();
    let mut chip = setup_y8950_with_adpcm_data(adpcm_data);
    chip.reset();
    chip.take_signals();

    // Start ADPCM-B playback to trigger external reads
    write_reg_y8950(&mut chip, 0x09, 0x20);
    write_reg_y8950(&mut chip, 0x0A, 0x00);
    write_reg_y8950(&mut chip, 0x0B, 0x23);
    write_reg_y8950(&mut chip, 0x0C, 0x00);
    write_reg_y8950(&mut chip, 0x10, 0x49);
    write_reg_y8950(&mut chip, 0x11, 0x0C);
    write_reg_y8950(&mut chip, 0x12, 0xFF);
    write_reg_y8950(&mut chip, 0x08, 0xC0);
    write_reg_y8950(&mut chip, 0x07, 0xA0);

    // Generate some samples to cause reads.
    let samples = generate_1_y8950(&mut chip, 64);
    assert!(
        samples.iter().any(|sample| sample[0] != 0),
        "ADPCM-B playback should read non-zero sample data"
    );
}

#[test]
fn read_data_readback() {
    let adpcm_data = create_y8950_adpcm_data();
    let mut chip = setup_y8950_with_adpcm_data(adpcm_data);
    chip.reset();

    // The Y8950 supports read_data for ADPCM readback
    // Just verify the function doesn't panic
    let _data = chip.read_data();
}
