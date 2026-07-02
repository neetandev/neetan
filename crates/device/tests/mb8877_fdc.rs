//! Tests for the MB8877 (WD1793 family) floppy disk controller.

use device::{
    floppy::{
        FloppyImage, MountedFloppy,
        d88::{D88Disk, D88MediaType, D88Sector},
    },
    mb8877_fdc::Mb8877Fdc,
};

const CPU_CLOCK_HZ: u32 = 16_000_000;
const SECTORS_PER_TRACK: u8 = 8;
const SECTOR_SIZE: usize = 256;
const SIZE_CODE: u8 = 1; // 128 << 1 = 256 bytes

// Port offsets.
const STATUS_COMMAND: u16 = 0x0200;
const TRACK: u16 = 0x0202;
const SECTOR: u16 = 0x0204;
const DATA: u16 = 0x0206;
const DRIVE_CONTROL: u16 = 0x0208;

// Drive-control bits.
const IRQ_ENABLE: u8 = 0x01;
const SIDE_ONE: u8 = 0x04;
const MOTOR: u8 = 0x10;

// Status bits.
const STATUS_BUSY: u8 = 0x01;
const STATUS_TRACK00: u8 = 0x04;
const STATUS_LOST_DATA: u8 = 0x04;
const STATUS_RECORD_NOT_FOUND: u8 = 0x10;

fn sector_fill(record: u8) -> u8 {
    0x10u8.wrapping_add(record)
}

/// Builds a single-track (cylinder 0, head 0) 2HD disk with 8 known sectors.
fn make_disk() -> D88Disk {
    let mut sectors = Vec::new();
    for record in 1..=SECTORS_PER_TRACK {
        sectors.push(D88Sector {
            cylinder: 0,
            head: 0,
            record,
            size_code: SIZE_CODE,
            sector_count: u16::from(SECTORS_PER_TRACK),
            mfm_flag: 0x40,
            deleted: 0x00,
            status: 0x00,
            reserved: [0u8; 5],
            data: vec![sector_fill(record); SECTOR_SIZE],
            source_offset: None,
        });
    }
    D88Disk::from_tracks(
        String::new(),
        false,
        D88MediaType::Disk2HD,
        vec![Some(sectors)],
    )
}

fn make_controller() -> Mb8877Fdc {
    let mut fdc = Mb8877Fdc::new(CPU_CLOCK_HZ);
    let image = FloppyImage::from_d88(make_disk());
    fdc.insert(0, MountedFloppy::new(image, None));
    // Enable IRQ and spin up the motor; select side 0.
    fdc.io_write(DRIVE_CONTROL, IRQ_ENABLE | MOTOR, 0);
    fdc
}

#[test]
fn restore_seeks_to_track_zero() {
    let mut fdc = make_controller();
    // Move off track 0 first.
    fdc.io_write(DATA, 5, 0);
    fdc.io_write(STATUS_COMMAND, 0x10, 0); // Seek to 5
    fdc.run_task(0);

    fdc.io_write(STATUS_COMMAND, 0x00, 0); // Restore
    fdc.run_task(0);
    assert!(fdc.irq_line(), "Restore should raise IRQ when enabled");

    let status = fdc.io_read(STATUS_COMMAND, 0);
    assert_eq!(status & STATUS_BUSY, 0, "BUSY must clear after Restore");
    assert_ne!(status & STATUS_TRACK00, 0, "TRACK0 must be set at track 0");
    assert!(!fdc.irq_line(), "reading status clears the IRQ");
    assert_eq!(fdc.io_read(TRACK, 0), 0);
}

#[test]
fn seek_updates_track_register() {
    let mut fdc = make_controller();
    fdc.io_write(DATA, 12, 0);
    fdc.io_write(STATUS_COMMAND, 0x10, 0); // Seek
    fdc.run_task(0);
    assert_eq!(fdc.io_read(TRACK, 0), 12);
}

#[test]
fn read_sector_returns_data_and_completes() {
    let mut fdc = make_controller();
    fdc.io_write(SECTOR, 3, 0);
    fdc.io_write(STATUS_COMMAND, 0x80, 0); // Read single sector
    let outcome = fdc.run_task(0);
    let data = outcome.dma_read.expect("read sector yields DMA data");
    assert_eq!(data.len(), SECTOR_SIZE);
    assert!(data.iter().all(|&b| b == sector_fill(3)));

    fdc.on_read_dma_complete(0, data.len());
    assert!(fdc.irq_line());
    let status = fdc.io_read(STATUS_COMMAND, 0);
    assert_eq!(status & STATUS_BUSY, 0);
    assert_eq!(status & STATUS_LOST_DATA, 0);
    assert_eq!(status & STATUS_RECORD_NOT_FOUND, 0);
}

#[test]
fn short_dma_read_sets_lost_data() {
    let mut fdc = make_controller();
    fdc.io_write(SECTOR, 1, 0);
    fdc.io_write(STATUS_COMMAND, 0x80, 0);
    let outcome = fdc.run_task(0);
    let data = outcome.dma_read.unwrap();
    // The DMA channel accepted fewer bytes than the sector length.
    fdc.on_read_dma_complete(0, data.len() - 1);
    let status = fdc.io_read(STATUS_COMMAND, 0);
    assert_ne!(
        status & STATUS_LOST_DATA,
        0,
        "partial read must flag LOST DATA"
    );
}

#[test]
fn missing_sector_reports_record_not_found() {
    let mut fdc = make_controller();
    fdc.io_write(SECTOR, 99, 0); // No such record on the track.
    fdc.io_write(STATUS_COMMAND, 0x80, 0);
    let outcome = fdc.run_task(0);
    assert!(outcome.dma_read.is_none());
    let status = fdc.io_read(STATUS_COMMAND, 0);
    assert_ne!(status & STATUS_RECORD_NOT_FOUND, 0);
}

#[test]
fn write_sector_round_trips() {
    let mut fdc = make_controller();
    fdc.io_write(SECTOR, 4, 0);
    fdc.io_write(STATUS_COMMAND, 0xA0, 0); // Write single sector
    let outcome = fdc.run_task(0);
    assert_eq!(outcome.dma_write_len, Some(SECTOR_SIZE));

    let new_data = vec![0xABu8; SECTOR_SIZE];
    fdc.on_write_dma_complete(0, &new_data);
    assert!(fdc.irq_line());

    // Read it back.
    fdc.io_write(SECTOR, 4, 0);
    fdc.io_write(STATUS_COMMAND, 0x80, 0);
    let read = fdc.run_task(0).dma_read.unwrap();
    assert_eq!(read, new_data);
}

#[test]
fn read_address_returns_chrn() {
    let mut fdc = make_controller();
    fdc.io_write(STATUS_COMMAND, 0xC0, 0); // Read Address
    let id = fdc
        .run_task(0)
        .dma_read
        .expect("read address yields ID bytes");
    assert_eq!(id.len(), 6);
    assert_eq!(id[0], 0, "cylinder");
    assert_eq!(id[1], 0, "head");
    assert_eq!(id[2], 1, "first record");
    assert_eq!(id[3], SIZE_CODE, "size code");
    // Read Address copies the track address into the sector register.
    assert_eq!(fdc.io_read(SECTOR, 0), 0);
}

#[test]
fn force_interrupt_clears_busy() {
    let mut fdc = make_controller();
    fdc.io_write(SECTOR, 1, 0);
    fdc.io_write(STATUS_COMMAND, 0x80, 0); // start a command (BUSY)
    fdc.io_write(STATUS_COMMAND, 0xD0, 0); // Force Interrupt, no IRQ
    let status = fdc.io_read(STATUS_COMMAND, 0);
    assert_eq!(status & STATUS_BUSY, 0);
    assert!(!fdc.irq_line());
}

#[test]
fn force_interrupt_with_immediate_flag_raises_irq() {
    let mut fdc = make_controller();
    fdc.io_write(STATUS_COMMAND, 0xD8, 0); // Force Interrupt, immediate IRQ
    fdc.run_task(0);
    assert!(fdc.irq_line());
}

#[test]
fn command_fe_is_a_noop() {
    let mut fdc = make_controller();
    fdc.io_write(STATUS_COMMAND, 0xFE, 0);
    let status = fdc.io_read(STATUS_COMMAND, 0);
    assert_eq!(status & STATUS_BUSY, 0);
    assert_eq!(fdc.next_task_cycle(), None);
}

#[test]
fn irq_mask_disabled_suppresses_interrupt() {
    let mut fdc = Mb8877Fdc::new(CPU_CLOCK_HZ);
    let image = FloppyImage::from_d88(make_disk());
    fdc.insert(0, MountedFloppy::new(image, None));
    // Motor on but IRQ mask cleared (bit0 = 0 -> disabled per errata).
    fdc.io_write(DRIVE_CONTROL, MOTOR, 0);
    fdc.io_write(STATUS_COMMAND, 0x00, 0); // Restore
    fdc.run_task(0);
    assert!(
        !fdc.irq_line(),
        "IRQ must stay low when the mask bit is clear"
    );
}

#[test]
fn drive_status_reports_ready_and_no_change() {
    let mut fdc = make_controller();
    let status = fdc.io_read(DRIVE_CONTROL, 0);
    // Disk present from power-on -> DSKCHG clear (bit 0). Motor on + ready -> bit 1.
    assert_eq!(status & 0x01, 0, "no media change for a power-on disk");
    assert_ne!(
        status & 0x02,
        0,
        "FREADY set with motor on and disk present"
    );

    fdc.eject(0);
    let status = fdc.io_read(DRIVE_CONTROL, 0);
    assert_ne!(status & 0x01, 0, "DSKCHG latches after ejecting the disk");
}

#[test]
fn side_select_bit_sets_head_one() {
    let mut fdc = make_controller();
    // Side bit high should select side 1 (errata: 0 = side 0, 1 = side 1).
    fdc.io_write(DRIVE_CONTROL, IRQ_ENABLE | MOTOR | SIDE_ONE, 0);
    // The single-track fixture has no head-1 track, so a read finds no sector.
    fdc.io_write(SECTOR, 1, 0);
    fdc.io_write(STATUS_COMMAND, 0x80, 0);
    let outcome = fdc.run_task(0);
    assert!(
        outcome.dma_read.is_none(),
        "selecting side 1 must address head 1, which has no sectors here"
    );
}
