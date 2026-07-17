//! Tests for the WD17xx-family floppy disk controller.

use device::{
    floppy::{
        FloppyImage, MountedFloppy,
        d88::{D88Disk, D88MediaType, D88Sector},
    },
    wd17xx_fdc::{WD17XX_PLATFORM_FM_TOWNS, WD17XX_PLATFORM_MSX, WD17XX_PLATFORM_X1, Wd17xxFdc},
};

type TownsWd17xxFdc = Wd17xxFdc<WD17XX_PLATFORM_FM_TOWNS>;
type X1Wd17xxFdc = Wd17xxFdc<WD17XX_PLATFORM_X1>;
type MsxWd17xxFdc = Wd17xxFdc<WD17XX_PLATFORM_MSX>;

/// Controller clock used by the tests.
const CPU_CLOCK_HZ: u32 = 16_000_000;
/// Number of sectors in the fixture track.
const SECTORS_PER_TRACK: u8 = 8;
/// Fixture sector size in bytes.
const SECTOR_SIZE: usize = 256;
/// Fixture sector size code.
const SIZE_CODE: u8 = 1;

/// Drive-control IRQ-enable bit.
const IRQ_ENABLE: u8 = 0x01;
/// Drive-control side-one bit.
const SIDE_ONE: u8 = 0x04;
/// Drive-control motor bit.
const MOTOR: u8 = 0x10;

/// Status busy bit.
const STATUS_BUSY: u8 = 0x01;
/// Status data-request bit.
const STATUS_DRQ: u8 = 0x02;
/// Status track-zero bit.
const STATUS_TRACK00: u8 = 0x04;
/// Status lost-data bit.
const STATUS_LOST_DATA: u8 = 0x04;
/// Status CRC-error bit.
const STATUS_CRC_ERROR: u8 = 0x08;
/// Status record-not-found bit.
const STATUS_RECORD_NOT_FOUND: u8 = 0x10;
/// Status write-protect bit.
const STATUS_WRITE_PROTECT: u8 = 0x40;
/// Status not-ready bit.
const STATUS_NOT_READY: u8 = 0x80;

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
            mfm_flag: 0x00,
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

fn make_controller() -> TownsWd17xxFdc {
    let mut fdc = TownsWd17xxFdc::new(CPU_CLOCK_HZ);
    let image = FloppyImage::from_d88(make_disk());
    fdc.insert(0, MountedFloppy::new(image, None));
    // Enable IRQ and spin up the motor; select side 0.
    fdc.write_drive_control(IRQ_ENABLE | MOTOR);
    fdc
}

#[test]
fn restore_seeks_to_track_zero() {
    let mut fdc = make_controller();
    // Move off track 0 first.
    fdc.write_data_register(5);
    fdc.write_command(0x10, 0); // Seek to 5
    fdc.run_task(0);

    fdc.write_command(0x00, 0); // Restore
    fdc.run_task(0);
    assert!(fdc.irq_line(), "Restore should raise IRQ when enabled");

    let status = fdc.read_status(0);
    assert_eq!(status & STATUS_BUSY, 0, "BUSY must clear after Restore");
    assert_ne!(status & STATUS_TRACK00, 0, "TRACK0 must be set at track 0");
    assert!(!fdc.irq_line(), "reading status clears the IRQ");
    assert_eq!(fdc.read_track_register(), 0);
}

#[test]
fn seek_updates_track_register() {
    let mut fdc = make_controller();
    fdc.write_data_register(12);
    fdc.write_command(0x10, 0); // Seek
    fdc.run_task(0);
    assert_eq!(fdc.read_track_register(), 12);
}

#[test]
fn read_sector_returns_data_and_completes() {
    let mut fdc = make_controller();
    fdc.write_sector_register(3);
    fdc.write_command(0x80, 0); // Read single sector
    let outcome = fdc.run_task(0);
    let data = outcome.dma_read.expect("read sector yields DMA data");
    assert_eq!(data.len(), SECTOR_SIZE);
    assert!(data.iter().all(|&b| b == sector_fill(3)));

    fdc.on_read_dma_complete(0, data.len());
    assert!(fdc.irq_line());
    let status = fdc.read_status(0);
    assert_eq!(status & STATUS_BUSY, 0);
    assert_eq!(status & STATUS_LOST_DATA, 0);
    assert_eq!(status & STATUS_RECORD_NOT_FOUND, 0);
}

#[test]
fn short_dma_read_sets_lost_data() {
    let mut fdc = make_controller();
    fdc.write_sector_register(1);
    fdc.write_command(0x80, 0);
    let outcome = fdc.run_task(0);
    let data = outcome.dma_read.unwrap();
    // The DMA channel accepted fewer bytes than the sector length.
    fdc.on_read_dma_complete(0, data.len() - 1);
    let status = fdc.read_status(0);
    assert_ne!(
        status & STATUS_LOST_DATA,
        0,
        "partial read must flag LOST DATA"
    );
}

#[test]
fn missing_sector_reports_record_not_found() {
    let mut fdc = make_controller();
    fdc.write_sector_register(99); // No such record on the track.
    fdc.write_command(0x80, 0);
    let outcome = fdc.run_task(0);
    assert!(outcome.dma_read.is_none());
    let status = fdc.read_status(0);
    assert_ne!(status & STATUS_RECORD_NOT_FOUND, 0);
}

/// Builds a single-track disk mixing 1024-byte and 512-byte records, like the
/// Arcus data disks (five N=3 records plus one N=2 record per track).
fn make_mixed_size_disk() -> D88Disk {
    let mut sectors = Vec::new();
    for record in 1..=5u8 {
        sectors.push(D88Sector {
            cylinder: 0,
            head: 0,
            record,
            size_code: 3,
            sector_count: 6,
            mfm_flag: 0x00,
            deleted: 0x00,
            status: 0x00,
            reserved: [0u8; 5],
            data: vec![sector_fill(record); 1024],
            source_offset: None,
        });
    }
    sectors.push(D88Sector {
        cylinder: 0,
        head: 0,
        record: 6,
        size_code: 2,
        sector_count: 6,
        mfm_flag: 0x00,
        deleted: 0x00,
        status: 0x00,
        reserved: [0u8; 5],
        data: vec![sector_fill(6); 512],
        source_offset: None,
    });
    D88Disk::from_tracks(
        String::new(),
        false,
        D88MediaType::Disk2HD,
        vec![Some(sectors)],
    )
}

#[test]
fn read_sector_matches_by_id_and_ignores_the_size_code() {
    // The WD179x compares only the ID's track and record (and optionally the
    // side) when searching for a sector; the size code is taken from the found
    // ID, so a track mixing sector sizes delivers each record's own length.
    let mut fdc = TownsWd17xxFdc::new(CPU_CLOCK_HZ);
    let image = FloppyImage::from_d88(make_mixed_size_disk());
    fdc.insert(0, MountedFloppy::new(image, None));
    fdc.write_drive_control(IRQ_ENABLE | MOTOR);

    fdc.write_sector_register(6);
    fdc.write_command(0x80, 0); // Read single sector
    let outcome = fdc.run_task(0);
    let data = outcome.dma_read.expect("the 512-byte record is found");
    assert_eq!(data.len(), 512);
    assert!(data.iter().all(|&b| b == sector_fill(6)));

    fdc.on_read_dma_complete(0, data.len());
    let status = fdc.read_status(0);
    assert_eq!(status & STATUS_BUSY, 0);
    assert_eq!(status & STATUS_RECORD_NOT_FOUND, 0);
}

#[test]
fn write_sector_round_trips() {
    let mut fdc = make_controller();
    fdc.write_sector_register(4);
    fdc.write_command(0xA0, 0); // Write single sector
    let outcome = fdc.run_task(0);
    assert_eq!(outcome.dma_write_len, Some(SECTOR_SIZE));

    let new_data = vec![0xABu8; SECTOR_SIZE];
    fdc.on_write_dma_complete(0, &new_data);
    assert!(fdc.irq_line());

    // Read it back.
    fdc.write_sector_register(4);
    fdc.write_command(0x80, 0);
    let read = fdc.run_task(0).dma_read.unwrap();
    assert_eq!(read, new_data);
}

#[test]
fn read_address_returns_chrn() {
    let mut fdc = make_controller();
    fdc.write_command(0xC0, 0); // Read Address
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
    assert_eq!(fdc.read_sector_register(), 0);
}

#[test]
fn force_interrupt_clears_busy() {
    let mut fdc = make_controller();
    fdc.write_sector_register(1);
    fdc.write_command(0x80, 0); // start a command (BUSY)
    fdc.write_command(0xD0, 0); // Force Interrupt, no IRQ
    let status = fdc.read_status(0);
    assert_eq!(status & STATUS_BUSY, 0);
    assert!(!fdc.irq_line());
}

#[test]
fn force_interrupt_with_immediate_flag_raises_irq() {
    let mut fdc = make_controller();
    fdc.write_command(0xD8, 0); // Force Interrupt, immediate IRQ
    fdc.run_task(0);
    assert!(fdc.irq_line());
}

#[test]
fn command_fe_is_a_noop() {
    let mut fdc = make_controller();
    fdc.write_command(0xFE, 0);
    let status = fdc.read_status(0);
    assert_eq!(status & STATUS_BUSY, 0);
    assert_eq!(fdc.next_task_cycle(), None);
}

#[test]
fn irq_mask_disabled_suppresses_interrupt() {
    let mut fdc = TownsWd17xxFdc::new(CPU_CLOCK_HZ);
    let image = FloppyImage::from_d88(make_disk());
    fdc.insert(0, MountedFloppy::new(image, None));
    // Motor on but IRQ mask cleared (bit0 = 0 -> disabled per errata).
    fdc.write_drive_control(MOTOR);
    fdc.write_command(0x00, 0); // Restore
    fdc.run_task(0);
    assert!(
        !fdc.irq_line(),
        "IRQ must stay low when the mask bit is clear"
    );
}

#[test]
fn drive_status_reports_ready_and_no_change() {
    let mut fdc = make_controller();
    let status = fdc.read_drive_status();
    // Disk present from power-on -> DSKCHG clear (bit 0). Motor on + ready -> bit 1.
    assert_eq!(status & 0x01, 0, "no media change for a power-on disk");
    assert_ne!(
        status & 0x02,
        0,
        "FREADY set with motor on and disk present"
    );

    fdc.eject(0);
    let status = fdc.read_drive_status();
    assert_ne!(status & 0x01, 0, "DSKCHG latches after ejecting the disk");
}

#[test]
fn side_select_bit_sets_head_one() {
    let mut fdc = make_controller();
    // Side bit high should select side 1 (errata: 0 = side 0, 1 = side 1).
    fdc.write_drive_control(IRQ_ENABLE | MOTOR | SIDE_ONE);
    // The single-track fixture has no head-1 track, so a read finds no sector.
    fdc.write_sector_register(1);
    fdc.write_command(0x80, 0);
    let outcome = fdc.run_task(0);
    assert!(
        outcome.dma_read.is_none(),
        "selecting side 1 must address head 1, which has no sectors here"
    );
}

fn make_pio_controller() -> X1Wd17xxFdc {
    let mut fdc = X1Wd17xxFdc::new(CPU_CLOCK_HZ);
    let image = FloppyImage::from_d88(make_disk());
    fdc.insert(0, MountedFloppy::new(image, None));
    fdc.set_motor(true);
    fdc.set_irq_enable(true);
    fdc.set_side(0);
    fdc.select_drive(0);
    fdc
}

#[test]
fn pio_read_sector_streams_bytes_via_data_register() {
    let mut fdc = make_pio_controller();
    fdc.write_sector_register(3);
    fdc.write_command(0x80, 0); // Read single sector
    let outcome = fdc.run_task(0);
    // PIO mode stages the data internally instead of requesting a DMA block.
    assert!(outcome.dma_read.is_none());

    // DRQ holds off until the first data-rate slot so the host observes the
    // initial data latency.
    assert_eq!(fdc.read_status(0) & STATUS_DRQ, 0);

    // Bytes arrive off the rotating disk at the fixed data rate; step a full
    // FM byte period (the slower rate) per byte.
    let byte_period = u64::from(CPU_CLOCK_HZ) / 15_625;
    let mut now = 0u64;
    let mut bytes = Vec::new();
    for _ in 0..SECTOR_SIZE {
        now += byte_period;
        assert_ne!(
            fdc.read_status(now) & STATUS_DRQ,
            0,
            "DRQ asserts at each data-rate slot"
        );
        bytes.push(fdc.read_data_pio(now));
    }
    assert!(bytes.iter().all(|&b| b == sector_fill(3)));
    assert!(!fdc.drq(), "DRQ drops after the final byte");
    assert!(fdc.irq_line(), "the command completes with an IRQ");
    let status = fdc.read_status(now);
    assert_eq!(status & STATUS_BUSY, 0);
    assert_eq!(status & STATUS_DRQ, 0);
}

#[test]
fn pio_write_sector_round_trips_via_data_register() {
    let mut fdc = make_pio_controller();
    fdc.write_sector_register(4);
    fdc.write_command(0xA0, 0); // Write single sector
    let outcome = fdc.run_task(0);
    assert!(outcome.dma_write_len.is_none(), "PIO does not request DMA");
    // DRQ holds off until the first data-rate slot, then paces the bytes.
    assert!(!fdc.drq());

    let byte_period = u64::from(CPU_CLOCK_HZ) / 15_625;
    let mut now = 0u64;
    let new_data = vec![0xABu8; SECTOR_SIZE];
    for &byte in &new_data {
        now += byte_period;
        assert_ne!(
            fdc.read_status(now) & STATUS_DRQ,
            0,
            "DRQ asserts at each data-rate slot"
        );
        fdc.write_data_pio(byte, now);
        assert!(!fdc.drq(), "accepting a byte drops DRQ until the next slot");
    }
    assert!(fdc.irq_line());

    // Read it back over the DMA-free PIO path.
    fdc.write_sector_register(4);
    fdc.write_command(0x80, now);
    fdc.run_task(now);
    let mut read = Vec::new();
    for _ in 0..SECTOR_SIZE {
        now += byte_period;
        read.push(fdc.read_data_pio(now));
    }
    assert_eq!(read, new_data);
}

#[test]
fn pio_data_register_write_latches_seek_target() {
    let mut fdc = make_pio_controller();

    // Leave a stale, nonzero value in the data register by reading a sector.
    fdc.write_sector_register(3);
    fdc.write_command(0x80, 0);
    fdc.run_task(0);
    let stale = fdc.read_data_pio(0);
    assert_ne!(stale, 5, "the stale value must differ from the seek target");

    // The IPL loads the target track into the data register, then issues SEEK.
    fdc.write_data_pio(5, 0);
    fdc.write_command(0x10, 0); // Seek
    fdc.run_task(0);

    assert_eq!(
        fdc.read_track_register(),
        5,
        "SEEK must use the freshly written data-register target, not a stale byte"
    );
}

fn make_msx_controller() -> MsxWd17xxFdc {
    let mut controller = MsxWd17xxFdc::new(CPU_CLOCK_HZ);
    controller.insert(
        0,
        MountedFloppy::new(FloppyImage::from_d88(make_disk()), None),
    );
    controller.set_irq_enable(true);
    controller.set_double_density(true);
    controller.set_motor(true);
    controller
}

#[test]
fn msx_ready_motor_and_density_wiring_match_sony() {
    let mut controller = make_msx_controller();
    controller.set_motor(false);
    controller.write_command(0x80, 0);
    controller.run_task(0);
    let status = controller.read_status(0);
    assert_eq!(status & STATUS_NOT_READY, 0);
    assert_ne!(status & STATUS_RECORD_NOT_FOUND, 0);

    controller.set_motor(true);
    controller.set_double_density(false);
    controller.write_command(0x80, 0);
    controller.run_task(0);
    assert_ne!(controller.read_status(0) & STATUS_RECORD_NOT_FOUND, 0);
}

#[test]
fn msx_unserviced_drq_sets_lost_data() {
    let mut controller = make_msx_controller();
    controller.write_command(0x80, 0);
    controller.run_task(0);

    let assertion = controller
        .next_pio_event_cycle()
        .expect("the first byte requests service");
    controller.run_pio_event(assertion);
    assert!(controller.drq());

    let deadline = controller
        .next_pio_event_cycle()
        .expect("an asserted request has a deadline");
    controller.run_pio_event(deadline);
    assert_ne!(controller.read_status(deadline) & STATUS_LOST_DATA, 0);
}

#[test]
fn msx_propagates_sector_crc_errors() {
    let mut disk = make_disk();
    disk.find_sector_on_track_index_mut(0, 0, 0, 1, SIZE_CODE)
        .expect("fixture sector exists")
        .status = 0xB0;
    let mut controller = MsxWd17xxFdc::new(CPU_CLOCK_HZ);
    controller.insert(0, MountedFloppy::new(FloppyImage::from_d88(disk), None));
    controller.set_irq_enable(true);
    controller.set_motor(true);
    controller.write_command(0x80, 0);
    controller.run_task(0);

    let byte_period = u64::from(CPU_CLOCK_HZ) / 31_250;
    let mut now = 0;
    for _ in 0..SECTOR_SIZE {
        now += byte_period;
        let _ = controller.read_data_pio(now);
    }
    assert_ne!(controller.read_status(now) & STATUS_CRC_ERROR, 0);
}

#[test]
fn write_protected_media_rejects_sector_writes() {
    let disk = D88Disk::from_tracks(
        String::new(),
        true,
        D88MediaType::Disk2HD,
        vec![Some(vec![D88Sector {
            cylinder: 0,
            head: 0,
            record: 1,
            size_code: SIZE_CODE,
            sector_count: 1,
            mfm_flag: 0x00,
            deleted: 0,
            status: 0,
            reserved: [0; 5],
            data: vec![0; SECTOR_SIZE],
            source_offset: None,
        }])],
    );
    let mut controller = MsxWd17xxFdc::new(CPU_CLOCK_HZ);
    controller.insert(0, MountedFloppy::new(FloppyImage::from_d88(disk), None));
    controller.set_motor(true);
    controller.write_command(0xA0, 0);
    controller.run_task(0);
    assert_ne!(controller.read_status(0) & STATUS_WRITE_PROTECT, 0);
}

#[test]
fn multi_sector_read_advances_until_the_end_of_the_track() {
    let mut controller = make_controller();
    controller.write_sector_register(1);
    controller.write_command(0x90, 0);
    for record in 1..=SECTORS_PER_TRACK {
        let outcome = controller.run_task(0);
        let data = outcome.dma_read.expect("each record produces data");
        assert!(data.iter().all(|byte| *byte == sector_fill(record)));
        controller.on_read_dma_complete(0, data.len());
    }
    assert_eq!(controller.read_sector_register(), SECTORS_PER_TRACK);
    assert_eq!(controller.read_status(0) & STATUS_BUSY, 0);
}

#[test]
fn read_and_write_track_use_hardware_commands() {
    let mut controller = make_controller();
    controller.write_command(0xE0, 0);
    let track = controller
        .run_task(0)
        .dma_read
        .expect("Read Track returns a DMA stream");
    assert_eq!(track.len(), usize::from(SECTORS_PER_TRACK) * SECTOR_SIZE);
    controller.on_read_dma_complete(0, track.len());

    controller.write_command(0xF0, 0);
    let length = controller
        .run_task(0)
        .dma_write_len
        .expect("Write Track requests a DMA stream");
    let mut format_stream = vec![0x4E; length];
    format_stream[..5].copy_from_slice(&[0xFE, 0, 0, 1, SIZE_CODE]);
    controller.on_write_dma_complete(0, &format_stream);

    controller.write_sector_register(1);
    controller.write_command(0x80, 0);
    let data = controller
        .run_task(0)
        .dma_read
        .expect("formatted sector can be read");
    assert!(data.iter().all(|byte| *byte == 0xE5));
}
