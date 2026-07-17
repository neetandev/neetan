mod common;

use common::harness::{
    EMU2413_YM2413_PRESET_CAPTURE_STARTS, assert_samples_2, generate_2_ym2413, setup_ym2413,
    setup_ym2413_channel, setup_ym2413_rhythm, write_reg_ym2413,
};

mod golden {
    include!("golden/ym2413_fm.rs");
}

/// Returns whether a sample sequence contains audible output.
fn contains_audio(samples: &[[i32; 2]]) -> bool {
    samples.iter().any(|sample| *sample != [0, 0])
}

#[test]
fn reset_is_silent_and_high_registers_are_ignored() {
    let mut chip = setup_ym2413();
    write_reg_ym2413(&mut chip, 0x40, 0xFF);
    let samples = generate_2_ym2413(&mut chip, golden::SILENCE.len());
    assert_samples_2(&samples, golden::SILENCE);
}

#[test]
fn preset_one_key_on_matches_ymfm() {
    let mut chip = setup_ym2413();
    setup_ym2413_channel(&mut chip, 0, 1, 0x80, 0x15, 0);
    let samples = generate_2_ym2413(&mut chip, golden::PRESET_ONE_KEY_ON.len());
    assert_samples_2(&samples, golden::PRESET_ONE_KEY_ON);
}

#[test]
fn user_instrument_key_on_matches_ymfm() {
    let mut chip = setup_ym2413();
    for (address, value) in [0xF1, 0xF1, 0x1E, 0x17, 0xF0, 0xF0, 0x00, 0x07]
        .into_iter()
        .enumerate()
    {
        write_reg_ym2413(&mut chip, address as u8, value);
    }
    setup_ym2413_channel(&mut chip, 0, 0, 0x80, 0x15, 0);
    let samples = generate_2_ym2413(&mut chip, golden::USER_INSTRUMENT_KEY_ON.len());
    assert_samples_2(&samples, golden::USER_INSTRUMENT_KEY_ON);
}

#[test]
fn all_preset_instruments_match_ymfm() {
    let mut samples = Vec::new();
    for (instrument, first_sample) in (1_u8..=15).zip(EMU2413_YM2413_PRESET_CAPTURE_STARTS) {
        let mut chip = setup_ym2413();
        setup_ym2413_channel(&mut chip, 0, instrument, 0x58 + instrument * 7, 0x15, 0);
        generate_2_ym2413(&mut chip, first_sample);
        samples.extend(generate_2_ym2413(&mut chip, 96));
    }
    assert_samples_2(&samples, golden::ALL_PRESET_INSTRUMENTS);
    for instrument in golden::ALL_PRESET_INSTRUMENTS.chunks_exact(96) {
        assert!(contains_audio(instrument));
    }
}

#[test]
fn key_off_release_matches_ymfm() {
    let mut chip = setup_ym2413();
    setup_ym2413_channel(&mut chip, 0, 8, 0x80, 0x15, 0);
    generate_2_ym2413(&mut chip, 768);
    write_reg_ym2413(&mut chip, 0x20, 0x05);
    let samples = generate_2_ym2413(&mut chip, golden::KEY_OFF_RELEASE.len());
    assert_samples_2(&samples, golden::KEY_OFF_RELEASE);
}

#[test]
fn pitch_change_matches_ymfm() {
    let mut chip = setup_ym2413();
    setup_ym2413_channel(&mut chip, 0, 4, 0x80, 0x15, 0);
    generate_2_ym2413(&mut chip, 512);
    let mut samples = generate_2_ym2413(&mut chip, 128);
    write_reg_ym2413(&mut chip, 0x10, 0xD0);
    write_reg_ym2413(&mut chip, 0x20, 0x17);
    samples.extend(generate_2_ym2413(&mut chip, 256));
    assert_samples_2(&samples, golden::PITCH_CHANGE);
}

#[test]
fn volume_change_matches_ymfm() {
    let mut chip = setup_ym2413();
    setup_ym2413_channel(&mut chip, 0, 6, 0x80, 0x15, 0);
    generate_2_ym2413(&mut chip, 512);
    let mut samples = generate_2_ym2413(&mut chip, 128);
    write_reg_ym2413(&mut chip, 0x30, 0x6A);
    samples.extend(generate_2_ym2413(&mut chip, 128));
    assert_samples_2(&samples, golden::VOLUME_CHANGE);
}

#[test]
fn three_channel_mix_matches_ymfm() {
    let mut chip = setup_ym2413();
    setup_ym2413_channel(&mut chip, 0, 1, 0x70, 0x15, 0);
    setup_ym2413_channel(&mut chip, 1, 7, 0x98, 0x17, 0);
    setup_ym2413_channel(&mut chip, 2, 12, 0xC0, 0x13, 0);
    let samples = generate_2_ym2413(&mut chip, golden::THREE_CHANNEL_MIX.len());
    assert_samples_2(&samples, golden::THREE_CHANNEL_MIX);
}

#[test]
fn sustained_key_off_matches_ymfm() {
    let mut chip = setup_ym2413();
    setup_ym2413_channel(&mut chip, 0, 9, 0x90, 0x35, 0);
    generate_2_ym2413(&mut chip, 768);
    write_reg_ym2413(&mut chip, 0x20, 0x25);
    let samples = generate_2_ym2413(&mut chip, golden::SUSTAINED_KEY_OFF.len());
    assert_samples_2(&samples, golden::SUSTAINED_KEY_OFF);
}

#[test]
fn all_rhythm_voices_match_ymfm() {
    let mut chip = setup_ym2413();
    setup_ym2413_rhythm(&mut chip);
    write_reg_ym2413(&mut chip, 0x0E, 0x3F);
    let samples = generate_2_ym2413(&mut chip, golden::RHYTHM_ALL_KEY_ON.len());
    assert_samples_2(&samples, golden::RHYTHM_ALL_KEY_ON);
}

#[test]
fn isolated_rhythm_voices_match_ymfm() {
    let mut samples = Vec::new();
    for key in 0..5 {
        let mut chip = setup_ym2413();
        setup_ym2413_rhythm(&mut chip);
        write_reg_ym2413(&mut chip, 0x0E, 0x20 | (1 << key));
        samples.extend(generate_2_ym2413(&mut chip, 192));
    }
    assert_samples_2(&samples, golden::RHYTHM_VOICES);
    for voice in golden::RHYTHM_VOICES.chunks_exact(192) {
        assert!(contains_audio(voice));
    }
}

#[test]
fn rhythm_key_off_matches_ymfm() {
    let mut chip = setup_ym2413();
    setup_ym2413_rhythm(&mut chip);
    write_reg_ym2413(&mut chip, 0x0E, 0x3F);
    generate_2_ym2413(&mut chip, 96);
    write_reg_ym2413(&mut chip, 0x0E, 0x20);
    let samples = generate_2_ym2413(&mut chip, golden::RHYTHM_KEY_OFF.len());
    assert_samples_2(&samples, golden::RHYTHM_KEY_OFF);
}

#[test]
fn behavioral_goldens_contain_audio() {
    for samples in [
        golden::PRESET_ONE_KEY_ON,
        golden::USER_INSTRUMENT_KEY_ON,
        golden::ALL_PRESET_INSTRUMENTS,
        golden::KEY_OFF_RELEASE,
        golden::PITCH_CHANGE,
        golden::VOLUME_CHANGE,
        golden::THREE_CHANNEL_MIX,
        golden::SUSTAINED_KEY_OFF,
        golden::RHYTHM_ALL_KEY_ON,
        golden::RHYTHM_VOICES,
        golden::RHYTHM_KEY_OFF,
    ] {
        assert!(contains_audio(samples));
    }
}
