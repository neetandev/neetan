use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

use common::{Bus, CdAudioState, CpuMode, Machine, MachineModel, NoTrace};
use device::cdrom::CdImage;
use machine_98::Pc9801Bus;

/// Builds a test CdImage with the given number of 2048-byte data sectors.
/// Each sector's first two bytes contain the sector index (big-endian).
fn make_test_cdimage(sector_count: u32) -> CdImage {
    let cue = "FILE \"test.bin\" BINARY\n  TRACK 01 MODE1/2048\n    INDEX 01 00:00:00\n";
    let mut bin_data = vec![0u8; sector_count as usize * 2048];
    for i in 0..sector_count {
        let offset = i as usize * 2048;
        bin_data[offset] = (i >> 8) as u8;
        bin_data[offset + 1] = i as u8;
    }
    CdImage::from_cue(cue, bin_data).expect("test CdImage creation failed")
}

/// Builds a multi-track CdImage: data track + audio track.
fn make_multi_track_cdimage() -> CdImage {
    let cue = "FILE \"test.bin\" BINARY\n  TRACK 01 MODE1/2048\n    INDEX 01 00:00:00\n  TRACK 02 AUDIO\n    INDEX 01 00:02:00\n";
    let mut bin_data = vec![0x11u8; 2048 * 150];
    bin_data.extend_from_slice(&vec![0xAAu8; 2352 * 50]);
    CdImage::from_cue(cue, bin_data).expect("multi-track CdImage creation failed")
}

/// Builds an audio-only CdImage with the given number of raw CD-DA sectors.
fn make_audio_cdimage(sector_count: u32) -> CdImage {
    let cue = "FILE \"test.bin\" BINARY\n  TRACK 01 AUDIO\n    INDEX 01 00:00:00\n";
    let bin_data = vec![0xAAu8; sector_count as usize * 2352];
    CdImage::from_cue(cue, bin_data).expect("audio CdImage creation failed")
}

/// Creates a PC-9821AS bus (IDE-equipped).
fn make_ide_bus() -> Pc9801Bus<NoTrace> {
    Pc9801Bus::<NoTrace>::new(MachineModel::PC9821AS, CpuMode::High, 48000)
}

/// Creates a PC-9821AP machine.
fn create_machine_ap() -> machine_98::Pc9821Ap {
    machine_98::Pc9821Ap::new(
        cpu::I386::new(),
        Pc9801Bus::<NoTrace>::new(MachineModel::PC9821AP, CpuMode::High, 48000),
    )
}

/// Writes a CUE file and BIN file to temp directory, returns the CUE path.
fn write_temp_cue_bin(name: &str, sector_count: u32) -> PathBuf {
    let cue_path = std::env::temp_dir().join(format!("neetan_test_{name}.cue"));
    let bin_path = std::env::temp_dir().join(format!("neetan_test_{name}.bin"));

    let cue_content = format!(
        "FILE \"neetan_test_{name}.bin\" BINARY\n  TRACK 01 MODE1/2048\n    INDEX 01 00:00:00\n"
    );
    std::fs::write(&cue_path, &cue_content).expect("failed to write temp CUE");

    let mut bin_data = vec![0u8; sector_count as usize * 2048];
    for i in 0..sector_count {
        let offset = i as usize * 2048;
        bin_data[offset] = (i >> 8) as u8;
        bin_data[offset + 1] = i as u8;
    }
    std::fs::write(&bin_path, &bin_data).expect("failed to write temp BIN");

    cue_path
}

fn cleanup_temp_cue_bin(name: &str) {
    let cue_path = std::env::temp_dir().join(format!("neetan_test_{name}.cue"));
    let bin_path = std::env::temp_dir().join(format!("neetan_test_{name}.bin"));
    let _ = std::fs::remove_file(cue_path);
    let _ = std::fs::remove_file(bin_path);
}

static TEMP_CDROM_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

struct TempCdromCueFiles {
    cue_path: PathBuf,
    bin_paths: Vec<PathBuf>,
}

impl Drop for TempCdromCueFiles {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.cue_path);
        for bin_path in &self.bin_paths {
            let _ = std::fs::remove_file(bin_path);
        }
    }
}

fn next_temp_cdrom_stem(name: &str) -> String {
    let sequence = TEMP_CDROM_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("neetan_test_{name}_{}_{}", std::process::id(), sequence)
}

fn write_temp_mode2_multi_file_cdrom(name: &str) -> TempCdromCueFiles {
    const ROOT_DIRECTORY_LBA: u32 = 20;
    const README_LBA: u32 = 21;
    const SETUP_LBA: u32 = 22;
    const DATA_TRACK_SECTOR_COUNT: u32 = 150;
    const AUDIO_TRACK_TOTAL_SECTOR_COUNT: u32 = 152;

    fn write_both_endian_u16(destination: &mut [u8], value: u16) {
        destination[..2].copy_from_slice(&value.to_le_bytes());
        destination[2..4].copy_from_slice(&value.to_be_bytes());
    }

    fn write_both_endian_u32(destination: &mut [u8], value: u32) {
        destination[..4].copy_from_slice(&value.to_le_bytes());
        destination[4..8].copy_from_slice(&value.to_be_bytes());
    }

    fn recording_time() -> [u8; 7] {
        [95, 1, 1, 12, 0, 0, 0]
    }

    fn write_directory_record(
        buffer: &mut [u8],
        offset: &mut usize,
        identifier: &[u8],
        extent_lba: u32,
        data_length: u32,
        is_directory: bool,
    ) {
        let padding = usize::from(identifier.len().is_multiple_of(2));
        let length = 33 + identifier.len() + padding;
        let record = &mut buffer[*offset..*offset + length];
        record.fill(0);
        record[0] = length as u8;
        write_both_endian_u32(&mut record[2..10], extent_lba);
        write_both_endian_u32(&mut record[10..18], data_length);
        record[18..25].copy_from_slice(&recording_time());
        record[25] = if is_directory { 0x02 } else { 0x00 };
        write_both_endian_u16(&mut record[28..32], 1);
        record[32] = identifier.len() as u8;
        record[33..33 + identifier.len()].copy_from_slice(identifier);
        *offset += length;
    }

    fn make_mode2_raw_sector(user_data: &[u8; 2048]) -> Vec<u8> {
        let mut sector = vec![0u8; 2352];
        sector[0] = 0x00;
        for byte in &mut sector[1..11] {
            *byte = 0xFF;
        }
        sector[11] = 0x00;
        sector[13] = 0x02;
        sector[15] = 0x02;
        sector[24..24 + 2048].copy_from_slice(user_data);
        sector
    }

    let temp_stem = next_temp_cdrom_stem(name);
    let cue_path = std::env::temp_dir().join(format!("{temp_stem}.cue"));
    let data_track_path = std::env::temp_dir().join(format!("{temp_stem}_track01.bin"));
    let audio_track_path = std::env::temp_dir().join(format!("{temp_stem}_track02.bin"));

    let cue_content = format!(
        "FILE \"{}\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\nFILE \"{}\" BINARY\n  TRACK 02 AUDIO\n    INDEX 00 00:00:00\n    INDEX 01 00:02:00\n",
        data_track_path
            .file_name()
            .expect("track 1 file name")
            .to_string_lossy(),
        audio_track_path
            .file_name()
            .expect("track 2 file name")
            .to_string_lossy(),
    );
    std::fs::write(&cue_path, cue_content).expect("failed to write temp multi-file CUE");

    let mut data_track = Vec::with_capacity(DATA_TRACK_SECTOR_COUNT as usize * 2352);
    for sector_index in 0..DATA_TRACK_SECTOR_COUNT {
        let mut user_data = [0u8; 2048];
        match sector_index {
            16 => {
                user_data[0] = 1;
                user_data[1..6].copy_from_slice(b"CD001");
                user_data[6] = 1;
                user_data[8..40].fill(b' ');
                user_data[8..28].copy_from_slice(b"NEETAN MODE2 TEST CD");
                user_data[40..72].fill(b' ');
                user_data[40..51].copy_from_slice(b"MODE2_MULTI");
                write_both_endian_u32(&mut user_data[80..88], DATA_TRACK_SECTOR_COUNT);
                write_both_endian_u16(&mut user_data[120..124], 1);
                write_both_endian_u16(&mut user_data[124..128], 1);
                write_both_endian_u16(&mut user_data[128..132], 2048);
                write_both_endian_u32(&mut user_data[132..140], 10);
                let mut root_record_offset = 156usize;
                write_directory_record(
                    &mut user_data,
                    &mut root_record_offset,
                    &[0],
                    ROOT_DIRECTORY_LBA,
                    2048,
                    true,
                );
            }
            17 => {
                user_data[0] = 0xFF;
                user_data[1..6].copy_from_slice(b"CD001");
                user_data[6] = 1;
            }
            ROOT_DIRECTORY_LBA => {
                let mut offset = 0usize;
                write_directory_record(
                    &mut user_data,
                    &mut offset,
                    &[0],
                    ROOT_DIRECTORY_LBA,
                    2048,
                    true,
                );
                write_directory_record(
                    &mut user_data,
                    &mut offset,
                    &[1],
                    ROOT_DIRECTORY_LBA,
                    2048,
                    true,
                );
                write_directory_record(
                    &mut user_data,
                    &mut offset,
                    b"README.TXT;1",
                    README_LBA,
                    5,
                    false,
                );
                write_directory_record(
                    &mut user_data,
                    &mut offset,
                    b"SETUP.EXE;1",
                    SETUP_LBA,
                    4,
                    false,
                );
            }
            README_LBA => {
                user_data[..5].copy_from_slice(b"HELLO");
            }
            SETUP_LBA => {
                user_data[..4].copy_from_slice(b"MZ\x90\x00");
            }
            _ => {
                user_data.fill(0x11);
            }
        }
        data_track.extend_from_slice(&make_mode2_raw_sector(&user_data));
    }

    let audio_track = vec![0xAAu8; AUDIO_TRACK_TOTAL_SECTOR_COUNT as usize * 2352];

    std::fs::write(&data_track_path, data_track).expect("failed to write temp data track");
    std::fs::write(&audio_track_path, audio_track).expect("failed to write temp audio track");

    TempCdromCueFiles {
        cue_path,
        bin_paths: vec![data_track_path, audio_track_path],
    }
}

/// Switches the IDE bank register to the given channel (0 = HDD, 1 = CD-ROM).
fn select_ide_channel(bus: &mut Pc9801Bus<NoTrace>, channel: u8) {
    bus.io_write_byte(0x0432, channel);
}

/// Writes a value to the IDE command register (port 0x064E).
fn ide_write_command(bus: &mut Pc9801Bus<NoTrace>, command: u8) {
    bus.io_write_byte(0x064E, command);
}

/// Reads the IDE alternate status register (port 0x074C).
fn ide_read_alt_status(bus: &mut Pc9801Bus<NoTrace>) -> u8 {
    bus.io_read_byte(0x074C)
}

/// Reads the IDE cylinder low register (port 0x0648).
fn ide_read_cylinder_low(bus: &mut Pc9801Bus<NoTrace>) -> u8 {
    bus.io_read_byte(0x0648)
}

/// Reads the IDE cylinder high register (port 0x064A).
fn ide_read_cylinder_high(bus: &mut Pc9801Bus<NoTrace>) -> u8 {
    bus.io_read_byte(0x064A)
}

/// Reads a 16-bit word from the IDE data register (port 0x0640).
fn ide_read_data_word(bus: &mut Pc9801Bus<NoTrace>) -> u16 {
    bus.io_read_word(0x0640)
}

/// Writes a 16-bit word to the IDE data register (port 0x0640).
fn ide_write_data_word(bus: &mut Pc9801Bus<NoTrace>, value: u16) {
    bus.io_write_word(0x0640, value);
}

/// Sets the byte count limit in cylinder low/high before a PACKET command.
fn ide_set_byte_count_limit(bus: &mut Pc9801Bus<NoTrace>, limit: u16) {
    bus.io_write_byte(0x0648, limit as u8);
    bus.io_write_byte(0x064A, (limit >> 8) as u8);
}

/// Sends a PACKET command and writes a 12-byte CDB.
fn send_atapi_packet(bus: &mut Pc9801Bus<NoTrace>, cdb: &[u8; 12]) {
    ide_set_byte_count_limit(bus, 0xFFFE);
    ide_write_command(bus, 0xA0);
    // Complete the PACKET setup event.
    bus.set_current_cycle(bus.current_cycle() + 1024);

    // Write 6 words (12 bytes).
    for i in (0..12).step_by(2) {
        let word = u16::from(cdb[i]) | (u16::from(cdb[i + 1]) << 8);
        ide_write_data_word(bus, word);
    }
    // Complete the command execution event.
    bus.set_current_cycle(bus.current_cycle() + 1024);
}

/// Reads `word_count` 16-bit words from the IDE data register.
fn read_atapi_data(bus: &mut Pc9801Bus<NoTrace>, word_count: usize) -> Vec<u16> {
    let mut data = Vec::with_capacity(word_count);
    for _ in 0..word_count {
        data.push(ide_read_data_word(bus));
    }
    data
}

fn words_to_bytes(words: &[u16]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(words.len() * 2);
    for word in words {
        bytes.push(*word as u8);
        bytes.push((word >> 8) as u8);
    }
    bytes
}

/// Clears the UNIT_ATTENTION state after CD-ROM insertion by sending
/// TEST UNIT READY (to trigger it) then REQUEST SENSE (to clear it).
fn acknowledge_media_change(bus: &mut Pc9801Bus<NoTrace>) {
    // TEST UNIT READY - will return CHECK CONDITION (UNIT_ATTENTION).
    send_atapi_packet(bus, &[0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    // REQUEST SENSE - clears the attention.
    send_atapi_packet(bus, &[0x03, 0, 0, 0, 18, 0, 0, 0, 0, 0, 0, 0]);
    // Read and discard the 18-byte sense data (9 words).
    read_atapi_data(bus, 9);
}

#[test]
fn cdrom_insert_sets_presence() {
    let mut bus = make_ide_bus();
    assert!(!bus.has_cdrom());

    bus.insert_cdrom(make_test_cdimage(100));
    assert!(bus.has_cdrom());
}

#[test]
fn cdrom_eject_clears_presence() {
    let mut bus = make_ide_bus();
    bus.insert_cdrom(make_test_cdimage(100));
    assert!(bus.has_cdrom());

    bus.eject_cdrom();
    assert!(!bus.has_cdrom());
}

#[test]
fn cdrom_insert_installs_ide_rom() {
    let mut bus = make_ide_bus();

    // Before insertion, ROM area should be unmapped.
    assert_eq!(bus.read_byte(0xD8000), 0xFF);

    bus.insert_cdrom(make_test_cdimage(100));

    // After insertion, ROM should be mapped (expansion ROM signature).
    assert_eq!(bus.read_byte(0xD8009), 0x55);
    assert_eq!(bus.read_byte(0xD800A), 0xAA);
}

#[test]
fn cdrom_presence_register_reflects_cdrom() {
    let mut bus = make_ide_bus();

    // Channel 1 not selected: returns 0x00 regardless of devices.
    assert_eq!(bus.io_read_byte(0x0433) & 0x02, 0x00);

    // Insert CD-ROM and select channel 1: returns 0x02.
    bus.insert_cdrom(make_test_cdimage(100));
    select_ide_channel(&mut bus, 1);
    assert_eq!(bus.io_read_byte(0x0433) & 0x02, 0x02);

    // Switch back to channel 0: returns 0x00.
    select_ide_channel(&mut bus, 0);
    assert_eq!(bus.io_read_byte(0x0433) & 0x02, 0x00);
}

#[test]
fn atapi_identify_device_returns_signature() {
    let mut bus = make_ide_bus();
    bus.insert_cdrom(make_test_cdimage(100));

    select_ide_channel(&mut bus, 1);
    ide_write_command(&mut bus, 0xEC); // IDENTIFY DEVICE
    bus.set_current_cycle(bus.current_cycle() + 1024);

    // ATAPI signature: cylinder_low=0x14, cylinder_high=0xEB.
    assert_eq!(ide_read_cylinder_low(&mut bus), 0x14);
    assert_eq!(ide_read_cylinder_high(&mut bus), 0xEB);

    // Status should have ERR/CHK bit set (abort).
    assert_ne!(ide_read_alt_status(&mut bus) & 0x01, 0);
}

#[test]
fn atapi_identify_packet_device() {
    let mut bus = make_ide_bus();
    bus.insert_cdrom(make_test_cdimage(100));

    select_ide_channel(&mut bus, 1);
    ide_write_command(&mut bus, 0xA1); // IDENTIFY PACKET DEVICE
    bus.set_current_cycle(bus.current_cycle() + 1024);

    let data = read_atapi_data(&mut bus, 256);

    // Word 0: 0x8580 (ATAPI, CD-ROM, removable, 12-byte packets).
    assert_eq!(data[0], 0x8580);
    // Word 49: LBA supported.
    assert_eq!(data[49], 0x0200);
}

#[test]
fn atapi_inquiry_returns_cdrom_device_type() {
    let mut bus = make_ide_bus();
    bus.insert_cdrom(make_test_cdimage(100));

    select_ide_channel(&mut bus, 1);
    acknowledge_media_change(&mut bus);

    // INQUIRY: allocation length 36.
    send_atapi_packet(&mut bus, &[0x12, 0, 0, 0, 36, 0, 0, 0, 0, 0, 0, 0]);
    let data = read_atapi_data(&mut bus, 18);

    // Byte 0: device type = 0x05 (CD-ROM).
    assert_eq!(data[0] & 0xFF, 0x05);
    // Byte 1: RMB = 0x80 (removable).
    assert_eq!(data[0] >> 8, 0x80);
}

#[test]
fn atapi_read_capacity() {
    let mut bus = make_ide_bus();
    bus.insert_cdrom(make_test_cdimage(100));

    select_ide_channel(&mut bus, 1);
    acknowledge_media_change(&mut bus);

    // READ CAPACITY.
    send_atapi_packet(&mut bus, &[0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let data = read_atapi_data(&mut bus, 4);

    // Last LBA (big-endian in bytes 0-3): 99 (100 sectors - 1).
    let b0 = data[0] as u8;
    let b1 = (data[0] >> 8) as u8;
    let b2 = data[1] as u8;
    let b3 = (data[1] >> 8) as u8;
    let last_lba = u32::from(b0) << 24 | u32::from(b1) << 16 | u32::from(b2) << 8 | u32::from(b3);
    assert_eq!(last_lba, 99);

    // Block size (bytes 4-7): 2048.
    let b4 = data[2] as u8;
    let b5 = (data[2] >> 8) as u8;
    let b6 = data[3] as u8;
    let b7 = (data[3] >> 8) as u8;
    let block_size = u32::from(b4) << 24 | u32::from(b5) << 16 | u32::from(b6) << 8 | u32::from(b7);
    assert_eq!(block_size, 2048);
}

#[test]
fn atapi_read_sector_data() {
    let mut bus = make_ide_bus();
    bus.insert_cdrom(make_test_cdimage(100));

    select_ide_channel(&mut bus, 1);
    acknowledge_media_change(&mut bus);

    // READ(10): LBA=42, count=1.
    send_atapi_packet(&mut bus, &[0x28, 0, 0, 0, 0, 42, 0, 0, 1, 0, 0, 0]);
    let data = read_atapi_data(&mut bus, 1024); // 2048 bytes = 1024 words.

    // First two bytes: sector 42 marker (0x00, 0x2A).
    assert_eq!(data[0] & 0xFF, 0x00);
    assert_eq!(data[0] >> 8, 42);
}

#[test]
fn atapi_read_toc() {
    let mut bus = make_ide_bus();
    bus.insert_cdrom(make_multi_track_cdimage());

    select_ide_channel(&mut bus, 1);
    acknowledge_media_change(&mut bus);

    // READ TOC: format 0, allocation 1024.
    send_atapi_packet(&mut bus, &[0x43, 0, 0, 0, 0, 0, 0, 0x04, 0x00, 0, 0, 0]);
    let data = read_atapi_data(&mut bus, 16);

    // Header bytes 2-3: first track = 1, last track = 2.
    let byte2 = data[1] as u8;
    let byte3 = (data[1] >> 8) as u8;
    assert_eq!(byte2, 1, "first track should be 1");
    assert_eq!(byte3, 2, "last track should be 2");

    // Track 1 descriptor: ADR/CTL = 0x14 (data), track number = 1.
    let byte5 = (data[2] >> 8) as u8;
    let byte6 = data[3] as u8;
    assert_eq!(byte5, 0x14, "track 1 should be data");
    assert_eq!(byte6, 1, "track number should be 1");

    // Track 2 descriptor: ADR/CTL = 0x10 (audio), track number = 2.
    let byte13 = (data[6] >> 8) as u8;
    let byte14 = data[7] as u8;
    assert_eq!(byte13, 0x10, "track 2 should be audio");
    assert_eq!(byte14, 2, "track number should be 2");
}

#[test]
fn atapi_media_change_unit_attention() {
    let mut bus = make_ide_bus();
    bus.insert_cdrom(make_test_cdimage(100));

    select_ide_channel(&mut bus, 1);

    // TEST UNIT READY - should fail with CHECK CONDITION (UNIT_ATTENTION).
    send_atapi_packet(&mut bus, &[0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let status = ide_read_alt_status(&mut bus);
    assert_ne!(
        status & 0x01,
        0,
        "should have CHK bit set after media change"
    );

    // REQUEST SENSE - should return UNIT_ATTENTION (sense key 0x06).
    send_atapi_packet(&mut bus, &[0x03, 0, 0, 0, 18, 0, 0, 0, 0, 0, 0, 0]);
    let sense_data = read_atapi_data(&mut bus, 9);
    let sense_key = (sense_data[1] as u8) & 0x0F; // Byte 2 of sense data.
    assert_eq!(sense_key, 0x06, "sense key should be UNIT_ATTENTION");
    // ASC = 0x28 (NOT_READY_TO_READY_TRANSITION).
    let asc = sense_data[6] as u8; // Byte 12 of sense data.
    assert_eq!(asc, 0x28, "ASC should be NOT_READY_TO_READY_TRANSITION");

    // TEST UNIT READY - should now succeed (attention cleared by REQUEST SENSE).
    send_atapi_packet(&mut bus, &[0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let status = ide_read_alt_status(&mut bus);
    assert_eq!(status & 0x01, 0, "should succeed after clearing attention");
}

#[test]
fn atapi_media_not_present() {
    let mut bus = make_ide_bus();
    // No CD-ROM inserted.

    select_ide_channel(&mut bus, 1);

    // TEST UNIT READY - should fail with NOT_READY.
    send_atapi_packet(&mut bus, &[0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let status = ide_read_alt_status(&mut bus);
    assert_ne!(status & 0x01, 0, "should have CHK bit set without media");

    // REQUEST SENSE - should return NOT_READY (sense key 0x02), ASC 0x3A.
    send_atapi_packet(&mut bus, &[0x03, 0, 0, 0, 18, 0, 0, 0, 0, 0, 0, 0]);
    let sense_data = read_atapi_data(&mut bus, 9);
    let sense_key = (sense_data[1] as u8) & 0x0F;
    assert_eq!(sense_key, 0x02, "sense key should be NOT_READY");
    let asc = sense_data[6] as u8;
    assert_eq!(asc, 0x3A, "ASC should be MEDIUM_NOT_PRESENT");
}

#[test]
fn atapi_eject_makes_not_ready() {
    let mut bus = make_ide_bus();
    bus.insert_cdrom(make_test_cdimage(100));

    select_ide_channel(&mut bus, 1);
    acknowledge_media_change(&mut bus);

    // Verify unit is ready.
    send_atapi_packet(&mut bus, &[0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(
        ide_read_alt_status(&mut bus) & 0x01,
        0,
        "should be ready before eject"
    );

    // Eject via bus.
    bus.eject_cdrom();

    // TEST UNIT READY - should fail with NOT_READY.
    send_atapi_packet(&mut bus, &[0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert_ne!(
        ide_read_alt_status(&mut bus) & 0x01,
        0,
        "should be not ready after eject"
    );
}

#[test]
fn atapi_reinsert_after_eject() {
    let mut bus = make_ide_bus();
    bus.insert_cdrom(make_test_cdimage(50));

    select_ide_channel(&mut bus, 1);
    acknowledge_media_change(&mut bus);

    // Eject.
    bus.eject_cdrom();
    assert!(!bus.has_cdrom());

    // Re-insert with a different image.
    bus.insert_cdrom(make_test_cdimage(200));
    assert!(bus.has_cdrom());

    // Should get UNIT_ATTENTION again.
    send_atapi_packet(&mut bus, &[0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert_ne!(
        ide_read_alt_status(&mut bus) & 0x01,
        0,
        "should get attention after re-insert"
    );

    acknowledge_media_change(&mut bus);

    // READ CAPACITY should reflect new image size.
    send_atapi_packet(&mut bus, &[0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let data = read_atapi_data(&mut bus, 4);
    let b0 = data[0] as u8;
    let b1 = (data[0] >> 8) as u8;
    let b2 = data[1] as u8;
    let b3 = (data[1] >> 8) as u8;
    let last_lba = u32::from(b0) << 24 | u32::from(b1) << 16 | u32::from(b2) << 8 | u32::from(b3);
    assert_eq!(
        last_lba, 199,
        "last LBA should reflect new 200-sector image"
    );
}

#[test]
fn atapi_mode_sense_page_0f_nec_vendor() {
    let mut bus = make_ide_bus();
    bus.insert_cdrom(make_test_cdimage(100));

    select_ide_channel(&mut bus, 1);
    acknowledge_media_change(&mut bus);

    // MODE SENSE(10): page 0x0F, allocation 256.
    send_atapi_packet(&mut bus, &[0x5A, 0, 0x0F, 0, 0, 0, 0, 0x01, 0x00, 0, 0, 0]);
    let data = read_atapi_data(&mut bus, 13);

    // Mode parameter header is 8 bytes (4 words).
    // Page 0x0F should start at byte offset 8.
    // data[4] contains bytes 8 and 9: page_code=0x0F, page_length=0x10.
    let page_code = data[4] as u8;
    let page_length = (data[4] >> 8) as u8;
    assert_eq!(page_code, 0x0F, "page code should be 0x0F (NEC vendor)");
    assert_eq!(
        page_length, 0x10,
        "page length should be 16 in the compact NEC vendor layout"
    );
    let nec_capability_byte = data[6] as u8;
    assert_eq!(
        nec_capability_byte, 0x81,
        "page 0x0F byte 4: bit 0 (audio play) | bit 7 (lock state)"
    );
}

#[test]
fn atapi_mode_sense_all_pages_includes_nec() {
    let mut bus = make_ide_bus();
    bus.insert_cdrom(make_test_cdimage(100));

    select_ide_channel(&mut bus, 1);
    acknowledge_media_change(&mut bus);

    // MODE SENSE(10): page 0x3F (all pages), large allocation.
    send_atapi_packet(&mut bus, &[0x5A, 0, 0x3F, 0, 0, 0, 0, 0x04, 0x00, 0, 0, 0]);
    // Read enough data to cover all pages.
    let data = read_atapi_data(&mut bus, 32);

    // Reconstruct as bytes for page scanning.
    let mut bytes = Vec::new();
    for word in &data {
        bytes.push(*word as u8);
        bytes.push((*word >> 8) as u8);
    }

    // Scan pages after 8-byte header.
    let mut found_pages = Vec::new();
    let mut offset = 8;
    while offset + 1 < bytes.len() {
        let page_code = bytes[offset] & 0x3F;
        let page_length = bytes[offset + 1] as usize;
        if page_length == 0 {
            break;
        }
        found_pages.push(page_code);
        offset += if page_code == 0x0F {
            16
        } else {
            2 + page_length
        };
    }

    assert!(
        found_pages.contains(&0x0F),
        "all-pages response should include NEC vendor page 0x0F, found: {found_pages:?}"
    );
    assert!(
        found_pages.contains(&0x2A),
        "all-pages response should include capabilities page 0x2A, found: {found_pages:?}"
    );
}

#[test]
fn atapi_play_audio_msf_uses_nec_bcd_mode() {
    let mut bus = make_ide_bus();
    bus.insert_cdrom(make_audio_cdimage(1400));

    select_ide_channel(&mut bus, 1);
    acknowledge_media_change(&mut bus);

    // MODE SENSE(10) page 0x0F enables the NEC CD-ROM BCD MSF address mode.
    send_atapi_packet(&mut bus, &[0x5A, 0, 0x0F, 0, 0, 0, 0, 0x01, 0x00, 0, 0, 0]);
    read_atapi_data(&mut bus, 13);

    // PLAY AUDIO MSF: start 0:12:34, end 0:12:35, encoded as NEC BCD.
    // The correct LBA range is 784..785. Binary parsing would select 1252..1253.
    send_atapi_packet(
        &mut bus,
        &[0x47, 0, 0, 0x00, 0x12, 0x34, 0x00, 0x12, 0x35, 0, 0, 0],
    );

    let status = bus
        .cd_audio_status()
        .expect("CD audio status should be available after inserting a CD-ROM");
    assert_eq!(status.state, CdAudioState::Playing);
    assert_eq!(status.start_lba, 784);
    assert_eq!(status.end_lba, 785);
}

#[test]
fn hdd_and_cdrom_coexist_on_separate_channels() {
    let mut bus = make_ide_bus();

    let geometry = device::disk::HddGeometry {
        cylinders: 20,
        heads: 4,
        sectors_per_track: 17,
        sector_size: 512,
    };
    let hdd_data = vec![0u8; geometry.total_bytes() as usize];
    let hdd = device::disk::HddImage::from_raw(geometry, device::disk::HddFormat::Hdi, hdd_data);
    bus.insert_hdd(0, hdd, None);
    bus.insert_cdrom(make_test_cdimage(100));

    // Channel 0 (HDD): IDENTIFY DEVICE should return HDD general config.
    select_ide_channel(&mut bus, 0);
    ide_write_command(&mut bus, 0xEC);
    bus.set_current_cycle(bus.current_cycle() + 1024);
    let data = read_atapi_data(&mut bus, 256);
    assert_eq!(data[0], 0x0040, "channel 0 should be HDD (word 0 = 0x0040)");

    // Channel 1 (CD-ROM): IDENTIFY DEVICE should abort with ATAPI signature.
    select_ide_channel(&mut bus, 1);
    ide_write_command(&mut bus, 0xEC);
    bus.set_current_cycle(bus.current_cycle() + 1024);
    assert_eq!(ide_read_cylinder_low(&mut bus), 0x14);
    assert_eq!(ide_read_cylinder_high(&mut bus), 0xEB);

    // Switch back to channel 0: HDD should still work.
    select_ide_channel(&mut bus, 0);
    ide_write_command(&mut bus, 0xEC);
    bus.set_current_cycle(bus.current_cycle() + 1024);
    let data = read_atapi_data(&mut bus, 256);
    assert_eq!(
        data[0], 0x0040,
        "channel 0 should still be HDD after switching"
    );
}

#[test]
fn machine_insert_cdrom_from_file() {
    let cue_path = write_temp_cue_bin("cdrom_machine", 100);

    let mut machine = create_machine_ap();
    let result = machine.insert_cdrom(&cue_path);
    cleanup_temp_cue_bin("cdrom_machine");

    let desc = result.expect("insert_cdrom should succeed");
    assert!(
        desc.contains("100 sectors"),
        "description should contain sector count: {desc}"
    );
    assert!(machine.bus.has_cdrom());
}

#[test]
fn machine_eject_cdrom() {
    let cue_path = write_temp_cue_bin("cdrom_eject", 50);

    let mut machine = create_machine_ap();
    machine
        .insert_cdrom(&cue_path)
        .expect("insert should succeed");
    cleanup_temp_cue_bin("cdrom_eject");

    assert!(machine.bus.has_cdrom());
    machine.eject_cdrom();
    assert!(!machine.bus.has_cdrom());
}

#[test]
fn machine_insert_cdrom_nonexistent_file() {
    let mut machine = create_machine_ap();
    let result = machine.insert_cdrom(Path::new("/tmp/neetan_nonexistent.cue"));
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.contains("Failed to read"),
        "error should mention read failure: {err}"
    );
}

#[test]
fn machine_insert_cdrom_invalid_cue() {
    let cue_path = std::env::temp_dir().join("neetan_test_invalid_cdrom.cue");
    std::fs::write(&cue_path, "not a valid cue sheet").expect("write temp");

    let mut machine = create_machine_ap();
    let result = machine.insert_cdrom(&cue_path);
    let _ = std::fs::remove_file(&cue_path);

    assert!(result.is_err());
}

#[test]
fn machine_swap_cdrom() {
    let cue_path_a = write_temp_cue_bin("cdrom_swap_a", 50);
    let cue_path_b = write_temp_cue_bin("cdrom_swap_b", 200);

    let mut machine = create_machine_ap();

    // Insert disc A.
    let desc_a = machine
        .insert_cdrom(&cue_path_a)
        .expect("insert A should succeed");
    assert!(desc_a.contains("50 sectors"));

    // Eject disc A.
    machine.eject_cdrom();
    assert!(!machine.bus.has_cdrom());

    // Insert disc B.
    let desc_b = machine
        .insert_cdrom(&cue_path_b)
        .expect("insert B should succeed");
    assert!(desc_b.contains("200 sectors"));
    assert!(machine.bus.has_cdrom());

    cleanup_temp_cue_bin("cdrom_swap_a");
    cleanup_temp_cue_bin("cdrom_swap_b");
}

#[test]
fn multifile_mode2_cdrom_pvd_is_readable_via_atapi() {
    let temp_cdrom_files = write_temp_mode2_multi_file_cdrom("mode2_multifile");
    let mut machine = create_machine_ap();
    machine
        .insert_cdrom(&temp_cdrom_files.cue_path)
        .expect("synthetic mode2 CD-ROM should mount");

    select_ide_channel(&mut machine.bus, 1);
    acknowledge_media_change(&mut machine.bus);

    send_atapi_packet(&mut machine.bus, &[0x28, 0, 0, 0, 0, 16, 0, 0, 1, 0, 0, 0]);
    let data = read_atapi_data(&mut machine.bus, 1024);
    let bytes = words_to_bytes(&data);

    assert_eq!(bytes[0], 0x01, "PVD type byte should be 1");
    assert_eq!(&bytes[1..6], b"CD001", "PVD should expose ISO9660 magic");
    assert_eq!(bytes[6], 0x01, "PVD version byte should be 1");
}
