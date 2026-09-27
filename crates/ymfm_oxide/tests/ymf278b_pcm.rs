mod common;

#[allow(dead_code)]
mod golden {
    include!("golden/ymf278b_pcm.rs");
}

use common::harness::*;

fn check(name: &str, expected: &[[i32; 6]]) {
    assert_samples_6(&ymf278b_pcm_scenario(name), expected);
}

fn check_long(name: &str, expected: &[u64]) {
    let samples = ymf278b_long_scenario(name);
    assert_eq!(
        block_checksums(&samples, YMF278B_CHECKSUM_BLOCK),
        expected,
        "{name} block checksums differ"
    );
}

#[test]
fn memory_port_writes_ram_with_auto_increment() {
    let mut chip = setup_ymf278b();
    let address = YMF278B_RAM_BASE + YMF278B_RAM_UPLOAD_OFFSET + 0xFE;
    upload_ymf278b_memory(&mut chip, address, &[0x11, 0x22, 0x33, 0x44]);
    let offset = (address - YMF278B_RAM_BASE) as usize;
    assert_eq!(
        &chip.pcm_ram()[offset..offset + 4],
        &[0x11, 0x22, 0x33, 0x44]
    );

    chip.write_address_pcm(0x03);
    assert_eq!(chip.read_data_pcm(), 0x20);
    chip.write_address_pcm(0x04);
    assert_eq!(chip.read_data_pcm(), 0x0D);
    chip.write_address_pcm(0x05);
    assert_eq!(chip.read_data_pcm(), 0x02);
}

#[test]
fn memory_port_reads_rom_and_ram() {
    let mut chip = setup_ymf278b();
    let rom = create_ymf278b_rom();
    let ram = create_ymf278b_ram();
    write_pcm_ymf278b(&mut chip, 0x02, 0x03);
    write_pcm_ymf278b(&mut chip, 0x03, 0x00);
    write_pcm_ymf278b(&mut chip, 0x04, 0x20);
    write_pcm_ymf278b(&mut chip, 0x05, 0x10);
    chip.write_address_pcm(0x06);
    for index in 0..8 {
        assert_eq!(chip.read_data_pcm(), rom[0x2010 + index]);
    }
    write_pcm_ymf278b(&mut chip, 0x03, 0x20);
    write_pcm_ymf278b(&mut chip, 0x04, 0x04);
    write_pcm_ymf278b(&mut chip, 0x05, 0x00);
    chip.write_address_pcm(0x06);
    for index in 0..8 {
        assert_eq!(chip.read_data_pcm(), ram[0x400 + index]);
    }
}

#[test]
fn memory_port_ignores_rom_and_unmapped_writes() {
    let mut chip = setup_ymf278b();
    upload_ymf278b_memory(&mut chip, 0x2000, &[0xAA; 4]);
    upload_ymf278b_memory(&mut chip, 0x30_0000, &[0xAA; 4]);
    assert_eq!(chip.pcm_ram(), create_ymf278b_ram().as_slice());

    write_pcm_ymf278b(&mut chip, 0x02, 0x03);
    write_pcm_ymf278b(&mut chip, 0x03, 0x00);
    write_pcm_ymf278b(&mut chip, 0x04, 0x20);
    write_pcm_ymf278b(&mut chip, 0x05, 0x00);
    chip.write_address_pcm(0x06);
    assert_eq!(chip.read_data_pcm(), create_ymf278b_rom()[0x2000]);
    write_pcm_ymf278b(&mut chip, 0x03, 0x30);
    write_pcm_ymf278b(&mut chip, 0x04, 0x00);
    write_pcm_ymf278b(&mut chip, 0x05, 0x00);
    chip.write_address_pcm(0x06);
    assert_eq!(chip.read_data_pcm(), 0x00);
}

#[test]
fn memory_data_register_holds_the_value_outside_access_mode() {
    let mut chip = setup_ymf278b();
    write_pcm_ymf278b(&mut chip, 0x06, 0x5A);
    chip.write_address_pcm(0x06);
    assert_eq!(chip.read_data_pcm(), 0x5A);
    assert_eq!(chip.pcm_ram(), create_ymf278b_ram().as_slice());
}

#[test]
fn wave_header_load_sets_the_channel_registers() {
    let mut chip = setup_ymf278b();
    load_ymf278b_pcm(&mut chip, 9, 5, 0, 0);
    for (register, expected) in [
        (0x80, 0x09),
        (0x98, 0xD3),
        (0xB0, 0x21),
        (0xC8, 0xE8),
        (0xE0, 0x01),
    ] {
        chip.write_address_pcm(register + 9);
        assert_eq!(chip.read_data_pcm(), expected);
    }
}

#[test]
fn pcm_8bit() {
    check("PCM_8BIT", golden::PCM_8BIT);
}

#[test]
fn pcm_12bit() {
    check("PCM_12BIT", golden::PCM_12BIT);
}

#[test]
fn pcm_16bit() {
    check("PCM_16BIT", golden::PCM_16BIT);
}

#[test]
fn pcm_format_3() {
    check("PCM_FORMAT_3", golden::PCM_FORMAT_3);
}

#[test]
fn pcm_short_loop() {
    check("PCM_SHORT_LOOP", golden::PCM_SHORT_LOOP);
}

#[test]
fn pcm_odd_loop() {
    check("PCM_ODD_LOOP", golden::PCM_ODD_LOOP);
}

#[test]
fn pcm_octave_minus_8() {
    check("PCM_OCTAVE_MINUS_8", golden::PCM_OCTAVE_MINUS_8);
}

#[test]
fn pcm_octave_minus_3() {
    check("PCM_OCTAVE_MINUS_3", golden::PCM_OCTAVE_MINUS_3);
}

#[test]
fn pcm_octave_plus_2() {
    check("PCM_OCTAVE_PLUS_2", golden::PCM_OCTAVE_PLUS_2);
}

#[test]
fn pcm_octave_plus_7() {
    check("PCM_OCTAVE_PLUS_7", golden::PCM_OCTAVE_PLUS_7);
}

#[test]
fn pcm_fnumber() {
    check("PCM_FNUMBER", golden::PCM_FNUMBER);
}

#[test]
fn pcm_pan_0() {
    check("PCM_PAN_0", golden::PCM_PAN_0);
}

#[test]
fn pcm_pan_1() {
    check("PCM_PAN_1", golden::PCM_PAN_1);
}

#[test]
fn pcm_pan_2() {
    check("PCM_PAN_2", golden::PCM_PAN_2);
}

#[test]
fn pcm_pan_3() {
    check("PCM_PAN_3", golden::PCM_PAN_3);
}

#[test]
fn pcm_pan_4() {
    check("PCM_PAN_4", golden::PCM_PAN_4);
}

#[test]
fn pcm_pan_5() {
    check("PCM_PAN_5", golden::PCM_PAN_5);
}

#[test]
fn pcm_pan_6() {
    check("PCM_PAN_6", golden::PCM_PAN_6);
}

#[test]
fn pcm_pan_7() {
    check("PCM_PAN_7", golden::PCM_PAN_7);
}

#[test]
fn pcm_pan_8() {
    check("PCM_PAN_8", golden::PCM_PAN_8);
}

#[test]
fn pcm_pan_9() {
    check("PCM_PAN_9", golden::PCM_PAN_9);
}

#[test]
fn pcm_pan_10() {
    check("PCM_PAN_10", golden::PCM_PAN_10);
}

#[test]
fn pcm_pan_11() {
    check("PCM_PAN_11", golden::PCM_PAN_11);
}

#[test]
fn pcm_pan_12() {
    check("PCM_PAN_12", golden::PCM_PAN_12);
}

#[test]
fn pcm_pan_13() {
    check("PCM_PAN_13", golden::PCM_PAN_13);
}

#[test]
fn pcm_pan_14() {
    check("PCM_PAN_14", golden::PCM_PAN_14);
}

#[test]
fn pcm_pan_15() {
    check("PCM_PAN_15", golden::PCM_PAN_15);
}

#[test]
fn pcm_output_channel() {
    check("PCM_OUTPUT_CHANNEL", golden::PCM_OUTPUT_CHANNEL);
}

#[test]
fn mix_0() {
    check("MIX_0", golden::MIX_0);
}

#[test]
fn mix_1() {
    check("MIX_1", golden::MIX_1);
}

#[test]
fn mix_2() {
    check("MIX_2", golden::MIX_2);
}

#[test]
fn mix_3() {
    check("MIX_3", golden::MIX_3);
}

#[test]
fn mix_4() {
    check("MIX_4", golden::MIX_4);
}

#[test]
fn mix_5() {
    check("MIX_5", golden::MIX_5);
}

#[test]
fn mix_6() {
    check("MIX_6", golden::MIX_6);
}

#[test]
fn mix_7() {
    check("MIX_7", golden::MIX_7);
}

#[test]
fn pcm_attack_8() {
    check("PCM_ATTACK_8", golden::PCM_ATTACK_8);
}

#[test]
fn pcm_attack_12() {
    check("PCM_ATTACK_12", golden::PCM_ATTACK_12);
}

#[test]
fn pcm_attack_14() {
    check("PCM_ATTACK_14", golden::PCM_ATTACK_14);
}

#[test]
fn pcm_decay_sustain() {
    check("PCM_DECAY_SUSTAIN", golden::PCM_DECAY_SUSTAIN);
}

#[test]
fn pcm_release() {
    check("PCM_RELEASE", golden::PCM_RELEASE);
}

#[test]
fn pcm_damp() {
    check("PCM_DAMP", golden::PCM_DAMP);
}

#[test]
fn pcm_reverb() {
    check("PCM_REVERB", golden::PCM_REVERB);
}

#[test]
fn pcm_rate_correction() {
    check("PCM_RATE_CORRECTION", golden::PCM_RATE_CORRECTION);
}

#[test]
fn pcm_level_direct() {
    check("PCM_LEVEL_DIRECT", golden::PCM_LEVEL_DIRECT);
}

#[test]
fn pcm_level_interpolation() {
    check("PCM_LEVEL_INTERPOLATION", golden::PCM_LEVEL_INTERPOLATION);
}

#[test]
fn pcm_vibrato() {
    check("PCM_VIBRATO", golden::PCM_VIBRATO);
}

#[test]
fn pcm_tremolo() {
    check("PCM_TREMOLO", golden::PCM_TREMOLO);
}

#[test]
fn pcm_lfo_reset() {
    check("PCM_LFO_RESET", golden::PCM_LFO_RESET);
}

#[test]
fn pcm_header_defaults() {
    check("PCM_HEADER_DEFAULTS", golden::PCM_HEADER_DEFAULTS);
}

#[test]
fn pcm_bank_rom_header() {
    check("PCM_BANK_ROM_HEADER", golden::PCM_BANK_ROM_HEADER);
}

#[test]
fn pcm_bank_ram_header() {
    check("PCM_BANK_RAM_HEADER", golden::PCM_BANK_RAM_HEADER);
}

#[test]
fn pcm_bank_ram_reformat() {
    check("PCM_BANK_RAM_REFORMAT", golden::PCM_BANK_RAM_REFORMAT);
}

#[test]
fn pcm_ram_sample() {
    check("PCM_RAM_SAMPLE", golden::PCM_RAM_SAMPLE);
}

#[test]
fn pcm_memory_upload() {
    check("PCM_MEMORY_UPLOAD", golden::PCM_MEMORY_UPLOAD);
}

#[test]
fn pcm_key_on_off_pending() {
    check("PCM_KEY_ON_OFF_PENDING", golden::PCM_KEY_ON_OFF_PENDING);
}

#[test]
fn pcm_retrigger() {
    check("PCM_RETRIGGER", golden::PCM_RETRIGGER);
}

#[test]
fn pcm_wave_change() {
    check("PCM_WAVE_CHANGE", golden::PCM_WAVE_CHANGE);
}

#[test]
fn pcm_all_channels() {
    check("PCM_ALL_CHANNELS", golden::PCM_ALL_CHANNELS);
}

#[test]
fn pcm_and_fm() {
    check("PCM_AND_FM", golden::PCM_AND_FM);
}

#[test]
fn pcm_new2_off() {
    check("PCM_NEW2_OFF", golden::PCM_NEW2_OFF);
}

#[test]
fn pcm_full_envelope() {
    check_long("PCM_FULL_ENVELOPE", golden::PCM_FULL_ENVELOPE);
}

#[test]
fn pcm_reverb_long() {
    check_long("PCM_REVERB_LONG", golden::PCM_REVERB_LONG);
}

#[test]
fn pcm_damp_long() {
    check_long("PCM_DAMP_LONG", golden::PCM_DAMP_LONG);
}

#[test]
fn pcm_level_sweep() {
    check_long("PCM_LEVEL_SWEEP", golden::PCM_LEVEL_SWEEP);
}

#[test]
fn pcm_lfo_speeds() {
    check_long("PCM_LFO_SPEEDS", golden::PCM_LFO_SPEEDS);
}

#[test]
fn pcm_rate_corrections() {
    check_long("PCM_RATE_CORRECTIONS", golden::PCM_RATE_CORRECTIONS);
}

#[test]
fn pcm_prepare_sweep() {
    check_long("PCM_PREPARE_SWEEP", golden::PCM_PREPARE_SWEEP);
}

#[test]
fn fm_resampling() {
    check_long("FM_RESAMPLING", golden::FM_RESAMPLING);
}
