//! Integration tests for the internal SCSI CD-ROM at ID 6 on the SUPER/XVI:
//! insertion through the machine interface, SCSI command dispatch through
//! the SPC, CDDA playback mixed into the motherboard audio, and media-change
//! sense reporting.

#[path = "common/harness.rs"]
mod harness;
#[path = "common/spc.rs"]
mod spc;

use common::Machine;
use harness::machine;
use machine_x68k::{X68kMachine, X68kModel};
use spc::{read_data_in, read_status_and_message, select, send_command};

/// SCSI status: command completed successfully.
const STATUS_GOOD: u8 = 0x00;
/// SCSI status: sense data is pending.
const STATUS_CHECK_CONDITION: u8 = 0x02;
/// Sense key: the drive has no medium.
const SENSE_KEY_NOT_READY: u8 = 0x02;
/// Sense key: the medium changed or the drive was reset.
const SENSE_KEY_UNIT_ATTENTION: u8 = 0x06;
/// Additional sense code: medium not present.
const ASC_MEDIUM_NOT_PRESENT: u8 = 0x3A;

/// Sectors in the MODE1 data track (LBA 0 to 15).
const DATA_TRACK_SECTORS: usize = 16;
/// Sectors in the audio track starting at LBA 16.
const AUDIO_TRACK_SECTORS: usize = 75;

/// Writes a mixed-mode CUE/BIN disc into a per-test temporary directory and
/// returns the CUE path. The data track stamps each sector's first byte with
/// its number plus one; the audio track holds a constant nonzero PCM level.
fn write_mixed_cd(test_name: &str) -> std::path::PathBuf {
    let directory = std::env::temp_dir().join(format!("neetan_x68k_cdrom_{test_name}"));
    std::fs::create_dir_all(&directory).unwrap();
    let mut bin = vec![0u8; DATA_TRACK_SECTORS * 2048 + AUDIO_TRACK_SECTORS * 2352];
    for sector in 0..DATA_TRACK_SECTORS {
        bin[sector * 2048] = sector as u8 + 1;
    }
    for byte in &mut bin[DATA_TRACK_SECTORS * 2048..] {
        *byte = 0x40;
    }
    std::fs::write(directory.join("disc.bin"), &bin).unwrap();
    let cue = "FILE \"disc.bin\" BINARY\n  TRACK 01 MODE1/2048\n    INDEX 01 00:00:00\n  TRACK 02 AUDIO\n    INDEX 01 00:00:16\n";
    let cue_path = directory.join("disc.cue");
    std::fs::write(&cue_path, cue).unwrap();
    cue_path
}

/// Builds the SUPER machine with the mixed-mode disc inserted at SCSI ID 6.
fn super_machine_with_cdrom(test_name: &str) -> X68kMachine {
    let mut machine = machine(X68kModel::X68000Super);
    let description = machine.insert_cdrom(&write_mixed_cd(test_name)).unwrap();
    assert!(!description.is_empty());
    machine
}

/// Reads the 18-byte sense data, returning it after a GOOD status.
fn request_sense(machine: &mut X68kMachine) -> Vec<u8> {
    select(machine, 6);
    send_command(machine, &[0x03, 0, 0, 0, 18, 0]);
    let sense = read_data_in(machine, 18);
    assert_eq!(read_status_and_message(machine), STATUS_GOOD);
    sense
}

/// Acknowledges the insertion unit attention with TEST UNIT READY plus
/// REQUEST SENSE.
fn acknowledge_unit_attention(machine: &mut X68kMachine) {
    select(machine, 6);
    send_command(machine, &[0x00, 0, 0, 0, 0, 0]);
    assert_eq!(read_status_and_message(machine), STATUS_CHECK_CONDITION);
    let sense = request_sense(machine);
    assert_eq!(sense[2] & 0x0F, SENSE_KEY_UNIT_ATTENTION);
}

#[test]
fn plain_x68000_rejects_cdrom_insertion() {
    let mut machine = machine(X68kModel::X68000);
    let error = machine
        .insert_cdrom(&write_mixed_cd("sasi_reject"))
        .unwrap_err();
    assert!(error.contains("CD-ROM"), "{error}");
    machine.eject_cdrom();
}

#[test]
fn first_test_unit_ready_reports_unit_attention_then_good() {
    let mut machine = super_machine_with_cdrom("unit_attention");
    acknowledge_unit_attention(&mut machine);

    select(&mut machine, 6);
    send_command(&mut machine, &[0x00, 0, 0, 0, 0, 0]);
    assert_eq!(read_status_and_message(&mut machine), STATUS_GOOD);
}

#[test]
fn inquiry_at_id_6_reports_a_removable_cdrom() {
    let mut machine = super_machine_with_cdrom("inquiry");
    acknowledge_unit_attention(&mut machine);

    select(&mut machine, 6);
    send_command(&mut machine, &[0x12, 0, 0, 0, 36, 0]);
    let data = read_data_in(&mut machine, 36);
    assert_eq!(data[0], 0x05, "CD-ROM device type");
    assert_ne!(data[1] & 0x80, 0, "removable medium");
    assert_eq!(read_status_and_message(&mut machine), STATUS_GOOD);
}

#[test]
fn read_toc_lists_both_tracks_and_the_lead_out() {
    let mut machine = super_machine_with_cdrom("read_toc");
    acknowledge_unit_attention(&mut machine);

    select(&mut machine, 6);
    send_command(&mut machine, &[0x43, 0, 0, 0, 0, 0, 0, 0, 28, 0]);
    let toc = read_data_in(&mut machine, 28);
    assert_eq!(toc[2], 1, "first track");
    assert_eq!(toc[3], 2, "last track");
    assert_eq!(toc[4 + 1], 0x14, "track 1 is a data track");
    assert_eq!(toc[4 + 2], 1);
    assert_eq!(toc[12 + 1], 0x10, "track 2 is an audio track");
    assert_eq!(toc[12 + 2], 2);
    assert_eq!(toc[20 + 2], 0xAA, "lead-out entry");
    assert_eq!(read_status_and_message(&mut machine), STATUS_GOOD);
}

#[test]
fn read_10_returns_the_stamped_data_sector() {
    let mut machine = super_machine_with_cdrom("read_10");
    acknowledge_unit_attention(&mut machine);

    select(&mut machine, 6);
    send_command(&mut machine, &[0x28, 0, 0, 0, 0, 2, 0, 0, 1, 0]);
    let data = read_data_in(&mut machine, 2048);
    assert_eq!(data[0], 3, "sector 2 carries its stamp");
    assert!(data[1..].iter().all(|&byte| byte == 0));
    assert_eq!(read_status_and_message(&mut machine), STATUS_GOOD);
}

#[test]
fn play_audio_mixes_cdda_and_pause_silences_it() {
    let mut machine = super_machine_with_cdrom("play_audio");
    acknowledge_unit_attention(&mut machine);

    // PLAY AUDIO(10) over the whole audio track.
    select(&mut machine, 6);
    send_command(&mut machine, &[0x45, 0, 0, 0, 0, 16, 0, 0, 75, 0]);
    assert_eq!(read_status_and_message(&mut machine), STATUS_GOOD);

    // READ SUB-CHANNEL reports audio playback in the audio track.
    select(&mut machine, 6);
    send_command(&mut machine, &[0x42, 0, 0x40, 0x01, 0, 0, 0, 0, 16, 0]);
    let sub_channel = read_data_in(&mut machine, 16);
    assert_eq!(sub_channel[1], 0x11, "audio status: playing");
    assert_eq!(sub_channel[6], 2, "track number");
    assert_eq!(read_status_and_message(&mut machine), STATUS_GOOD);

    // The playing track mixes nonzero CDDA into the motherboard output.
    let mut output = vec![0.0f32; 2048];
    machine.generate_audio_samples(1.0, &mut output);
    assert!(output.iter().any(|&sample| sample != 0.0));

    // PAUSE stops the mix immediately.
    select(&mut machine, 6);
    send_command(&mut machine, &[0x4B, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(read_status_and_message(&mut machine), STATUS_GOOD);
    let mut output = vec![0.0f32; 2048];
    machine.generate_audio_samples(1.0, &mut output);
    assert!(output.iter().all(|&sample| sample == 0.0));
}

#[test]
fn eject_and_reinsert_signal_media_change() {
    let mut machine = super_machine_with_cdrom("media_change");
    acknowledge_unit_attention(&mut machine);

    machine.eject_cdrom();
    select(&mut machine, 6);
    send_command(&mut machine, &[0x00, 0, 0, 0, 0, 0]);
    assert_eq!(
        read_status_and_message(&mut machine),
        STATUS_CHECK_CONDITION
    );
    let sense = request_sense(&mut machine);
    assert_eq!(sense[2] & 0x0F, SENSE_KEY_NOT_READY);
    assert_eq!(sense[12], ASC_MEDIUM_NOT_PRESENT);

    machine
        .insert_cdrom(&write_mixed_cd("media_change"))
        .unwrap();
    acknowledge_unit_attention(&mut machine);
    select(&mut machine, 6);
    send_command(&mut machine, &[0x00, 0, 0, 0, 0, 0]);
    assert_eq!(read_status_and_message(&mut machine), STATUS_GOOD);
}
