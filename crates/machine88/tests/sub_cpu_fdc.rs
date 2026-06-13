//! Disk sub-system integration tests: the PPI mailbox and the FDC PIO data path.
//!
//! These drive the disk side through the public sub-CPU I/O surface (no CPU code
//! required), so the protocol is deterministic.

mod harness;

use std::path::PathBuf;

use common::Machine;
use device::floppy::{D88Disk, D88MediaType, D88Sector, FloppyImage};
use harness::build_machine_with;
use machine88::Pc8801Machine;

const SECTOR_SIZE: usize = 256;
const SIZE_CODE_256: u8 = 1;

/// Builds a single-track 2D image whose sector 1 holds a known ramp pattern.
fn synthetic_2d_image() -> FloppyImage {
    let mut sectors = Vec::new();
    for record in 1..=16u8 {
        let data: Vec<u8> = (0..SECTOR_SIZE)
            .map(|index| (index as u8).wrapping_add(record))
            .collect();
        sectors.push(D88Sector {
            cylinder: 0,
            head: 0,
            record,
            size_code: SIZE_CODE_256,
            sector_count: 16,
            mfm_flag: 0x00,
            deleted: 0x00,
            status: 0x00,
            reserved: [0; 5],
            data,
            source_offset: None,
        });
    }
    let disk = D88Disk::from_tracks(
        String::from("PIO-TEST"),
        false,
        D88MediaType::Disk2D,
        vec![Some(sectors)],
    );
    FloppyImage::from_d88(disk)
}

fn temp_path(suffix: &str) -> PathBuf {
    let unique = format!(
        "neetan_pc88_fdc_{}_{}{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        suffix
    );
    std::env::temp_dir().join(unique)
}

/// Advances the main clock until the FDC asserts RQM, then returns true. Reading
/// the main status register has no side effects, so it is safe to poll.
fn wait_for_rqm(machine: &mut Pc8801Machine) -> bool {
    for _ in 0..4096 {
        if machine.bus.sub_io_read(0xFA) & 0x80 != 0 {
            return true;
        }
        let cycle = machine.bus.current_cycle();
        machine.bus.set_current_cycle(cycle + 64);
    }
    false
}

#[test]
fn mailbox_carries_data_both_directions() {
    let mut machine = build_machine_with(|_| {});

    // Host writes port A; the sub reads it on port B.
    machine.bus.io_write(0xFC, 0x3C);
    assert_eq!(machine.bus.sub_io_read(0xFD), 0x3C);

    // Sub writes port A; the host reads it on port B.
    machine.bus.sub_io_write(0xFC, 0xC3);
    assert_eq!(machine.bus.io_read(0xFD), 0xC3);
}

#[test]
fn fdc_reads_mounted_sector_over_pio() {
    let image = synthetic_2d_image();
    let path = temp_path(".d88");
    std::fs::write(&path, image.to_bytes()).expect("write temp image");

    let mut machine = build_machine_with(|_| {});
    machine
        .insert_floppy(0, &path)
        .expect("mount synthetic image");

    // Drive 0 is a 2D disk; select 2D so the density check matches.
    machine.bus.sub_io_write(0xF4, 0x00);

    // SPECIFY with non-DMA mode (ND = 1).
    machine.bus.sub_io_write(0xFB, 0x03);
    machine.bus.sub_io_write(0xFB, 0xCF);
    machine.bus.sub_io_write(0xFB, 0x01);

    // READ DATA (MFM): HD/US=0, C=0, H=0, R=1, N=1, EOT=1, GPL, DTL.
    machine.bus.sub_io_write(0xFB, 0x46);
    for &byte in &[0x00, 0x00, 0x00, 0x01, SIZE_CODE_256, 0x01, 0x1B, 0xFF] {
        machine.bus.sub_io_write(0xFB, byte);
    }

    // Stream the sector one byte at a time, paced by the FDC DRQ events.
    let mut received = Vec::with_capacity(SECTOR_SIZE);
    for _ in 0..SECTOR_SIZE {
        assert!(
            wait_for_rqm(&mut machine),
            "FDC should signal each data byte"
        );
        received.push(machine.bus.sub_io_read(0xFB));
    }

    let expected: Vec<u8> = (0..SECTOR_SIZE)
        .map(|index| (index as u8).wrapping_add(1))
        .collect();
    assert_eq!(received, expected, "PIO read returns the sector ramp");

    // After the sector, the FDC enters the result phase with a normal status.
    let st0 = machine.bus.sub_io_read(0xFB);
    assert_eq!(st0 & 0xC0, 0x00, "ST0 reports normal termination");

    std::fs::remove_file(&path).ok();
}

#[test]
fn missing_drive_reports_not_ready() {
    // No disk mounted: a READ DATA must fail with the not-ready status, not hang.
    let mut machine = build_machine_with(|_| {});
    machine.bus.sub_io_write(0xFB, 0x46);
    for &byte in &[0x00, 0x00, 0x00, 0x01, SIZE_CODE_256, 0x01, 0x1B, 0xFF] {
        machine.bus.sub_io_write(0xFB, byte);
    }
    // The command fails immediately into the result phase; ST0 has IC=01 and NR.
    let st0 = machine.bus.sub_io_read(0xFB);
    assert_eq!(st0 & 0xC0, 0x40, "abnormal termination");
    assert_eq!(st0 & 0x08, 0x08, "not ready");
}
