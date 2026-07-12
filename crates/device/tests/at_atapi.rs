//! Secondary-channel ATAPI CD-ROM controller tests for the PC/AT.
//!
//! Exercises the `AtAtapiController` through its public register interface,
//! the same way the AT bus drives it at ports 0x170-0x177/0x376. The full
//! ATAPI command set is already covered by the PC-98 `IdeController` tests;
//! these focus on the AT wrapper: channel activation, the packet transport
//! reaching the shared routing, CD audio state, and isolation from a separate
//! primary HDD controller.

use device::{
    cdrom::CdImage,
    disk::{HddFormat, HddGeometry, HddImage},
    ide::{AtAtapiController, AtIdeController, IdeAction},
};

/// Builds a single-data-track CD image whose sector N starts with the bytes
/// `[N >> 8, N, ...]`, so reads can assert the exact sector they landed on.
fn make_test_cdimage() -> CdImage {
    let cue = r#"FILE "test.bin" BINARY
  TRACK 01 MODE1/2048
    INDEX 01 00:00:00
"#;
    let mut bin_data = vec![0u8; 2048 * 100];
    for i in 0..100u32 {
        let offset = i as usize * 2048;
        bin_data[offset] = (i >> 8) as u8;
        bin_data[offset + 1] = i as u8;
    }
    CdImage::from_cue(cue, bin_data).unwrap()
}

fn make_test_drive() -> HddImage {
    let geometry = HddGeometry {
        cylinders: 20,
        heads: 4,
        sectors_per_track: 17,
        sector_size: 512,
    };
    let data = vec![0u8; geometry.total_bytes() as usize];
    HddImage::from_raw(geometry, HddFormat::Hdi, data)
}

/// Issues a PACKET command with the given byte-count limit, ready for the CDB.
fn start_packet(controller: &mut AtAtapiController, byte_count_limit: u16) {
    controller.write_cylinder_low(byte_count_limit as u8);
    controller.write_cylinder_high((byte_count_limit >> 8) as u8);
    controller.write_command(0xA0);
}

/// Reads the INQUIRY response, which also clears the power-on media-change
/// UNIT_ATTENTION so following data commands proceed.
fn clear_media_attention(controller: &mut AtAtapiController) {
    start_packet(controller, 0xFFFE);
    controller.write_data_word(0x0012); // INQUIRY opcode 0x12.
    controller.write_data_word(0x0000);
    controller.write_data_word(0x0024); // allocation length 36.
    controller.write_data_word(0x0000);
    controller.write_data_word(0x0000);
    controller.write_data_word(0x0000);
    // Drain the 36-byte response.
    for _ in 0..18 {
        let (_, action) = controller.read_data_word();
        if action == IdeAction::ScheduleCompletion {
            break;
        }
    }
}

#[test]
fn atapi_signature_present_after_insert() {
    let mut controller = AtAtapiController::new(44100);
    controller.insert_cdrom(make_test_cdimage());

    // insert_cdrom activates the ATAPI channel, so the signature registers
    // are readable without any bank-select port.
    assert_eq!(controller.read_cylinder_low(), 0x14);
    assert_eq!(controller.read_cylinder_high(), 0xEB);
    assert!(controller.has_cdrom());
}

#[test]
fn identify_packet_device_returns_config_word() {
    let mut controller = AtAtapiController::new(44100);
    controller.insert_cdrom(make_test_cdimage());

    let action = controller.write_command(0xA1);
    assert_eq!(action, IdeAction::ScheduleCompletion);

    let mut data = [0u16; 256];
    for word in data.iter_mut() {
        *word = controller.read_data_word().0;
    }
    // Word 0: 0x8580 = ATAPI CD-ROM device.
    assert_eq!(data[0], 0x8580);
}

#[test]
fn inquiry_returns_cdrom_device_type() {
    let mut controller = AtAtapiController::new(44100);
    controller.insert_cdrom(make_test_cdimage());

    start_packet(&mut controller, 0xFFFE);
    controller.write_data_word(0x0012);
    controller.write_data_word(0x0000);
    controller.write_data_word(0x0024);
    controller.write_data_word(0x0000);
    controller.write_data_word(0x0000);
    controller.write_data_word(0x0000);

    let (first_word, _) = controller.read_data_word();
    assert_eq!(first_word & 0xFF, 0x05); // Device type: CD-ROM.
    assert_eq!(first_word >> 8, 0x80); // Removable.
}

#[test]
fn read10_returns_addressed_sector() {
    let mut controller = AtAtapiController::new(44100);
    controller.insert_cdrom(make_test_cdimage());
    clear_media_attention(&mut controller);

    start_packet(&mut controller, 0xFFFE);
    // READ(10): opcode 0x28, LBA 42, count 1.
    controller.write_data_word(0x0028);
    controller.write_data_word(0x0000);
    controller.write_data_word(0x2A00); // byte[5] = 0x2A (LBA = 42).
    controller.write_data_word(0x0000);
    controller.write_data_word(0x0001); // count = 1.
    controller.write_data_word(0x0000);

    let (first_word, _) = controller.read_data_word();
    assert_eq!(first_word & 0xFF, 0); // sector 42 byte[0].
    assert_eq!(first_word >> 8, 42); // sector 42 byte[1].
}

#[test]
fn play_audio_msf_then_pause_resume_drives_cd_audio() {
    use device::cd_audio::CdAudioState;

    let mut controller = AtAtapiController::new(44100);
    controller.insert_cdrom(make_test_cdimage());
    clear_media_attention(&mut controller);

    assert_eq!(controller.cd_audio_player().state(), CdAudioState::Stopped);

    // PLAY AUDIO MSF (0x47) from 00:02:00 (LBA 0) to 00:03:00 (LBA 75). Each
    // word fills two packet bytes low-byte-first, so the CDB bytes become
    // [0x47,0,0, 0,2,0, 0,3,0, 0,0,0]: start M:S:F = 0:2:0, end M:S:F = 0:3:0.
    start_packet(&mut controller, 0xFFFE);
    controller.write_data_word(0x0047); // byte[0]=0x47, byte[1]=0.
    controller.write_data_word(0x0000); // byte[2]=0, byte[3]=0 (start M).
    controller.write_data_word(0x0002); // byte[4]=2 (start S), byte[5]=0 (start F).
    controller.write_data_word(0x0300); // byte[6]=0 (end M), byte[7]=3 (end S).
    controller.write_data_word(0x0000); // byte[8]=0 (end F), byte[9]=0.
    controller.write_data_word(0x0000);
    assert_eq!(controller.cd_audio_player().state(), CdAudioState::Playing);

    // PAUSE (0x4B, resume bit clear).
    start_packet(&mut controller, 0xFFFE);
    controller.write_data_word(0x004B);
    controller.write_data_word(0x0000);
    controller.write_data_word(0x0000);
    controller.write_data_word(0x0000);
    controller.write_data_word(0x0000); // byte[8] = 0 -> pause.
    controller.write_data_word(0x0000);
    assert_eq!(controller.cd_audio_player().state(), CdAudioState::Paused);

    // RESUME (0x4B, resume bit set in byte[8]).
    start_packet(&mut controller, 0xFFFE);
    controller.write_data_word(0x004B);
    controller.write_data_word(0x0000);
    controller.write_data_word(0x0000);
    controller.write_data_word(0x0000);
    controller.write_data_word(0x0001); // byte[8] = 1 -> resume.
    controller.write_data_word(0x0000);
    assert_eq!(controller.cd_audio_player().state(), CdAudioState::Playing);
}

#[test]
fn eject_clears_media() {
    let mut controller = AtAtapiController::new(44100);
    controller.insert_cdrom(make_test_cdimage());
    assert!(controller.has_cdrom());

    controller.eject_cdrom();
    assert!(!controller.has_cdrom());
}

#[test]
fn secondary_atapi_isolated_from_primary_hdd() {
    // Two independent controllers, as the AT bus instantiates them.
    let mut primary = AtIdeController::new();
    primary.insert_drive(0, make_test_drive(), None);
    let mut secondary = AtAtapiController::new(44100);
    secondary.insert_cdrom(make_test_cdimage());

    // Primary IDENTIFY DEVICE returns the HDD config word.
    assert_eq!(primary.write_command(0xEC), IdeAction::ScheduleCompletion);
    let mut hdd_identify = [0u16; 256];
    for word in hdd_identify.iter_mut() {
        *word = primary.read_data_word().0;
    }
    assert_eq!(hdd_identify[0], 0x0040); // ATA HDD general configuration.

    // Drive an ATAPI IDENTIFY PACKET on the secondary; the primary is untouched.
    assert_eq!(secondary.write_command(0xA1), IdeAction::ScheduleCompletion);
    assert_eq!(secondary.read_data_word().0, 0x8580);

    // The primary still reports its own signature, not the ATAPI one.
    assert_eq!(primary.read_cylinder_low(), 0x00);
    assert_ne!(primary.read_cylinder_low(), 0x14);
}
