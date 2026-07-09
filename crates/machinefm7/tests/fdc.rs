//! MB8877 floppy controller tests for the FM-7 (0xFD18-0xFD1F, PIO path).

mod harness;

use std::path::PathBuf;

use device::floppy::{D88Disk, D88MediaType, D88Sector, FloppyImage};
use harness::{build_bus_with_synthetic_roms, run_bus_cycles, synthetic_roms};
use machinefm7::{BootMode, Fm7Bus, Fm7Model};

/// FDC register ports on the main CPU I/O page.
const FDC_STATUS_COMMAND: u16 = 0xFD18;
const FDC_TRACK: u16 = 0xFD19;
const FDC_SECTOR: u16 = 0xFD1A;
const FDC_DATA: u16 = 0xFD1B;
const FDC_SIDE: u16 = 0xFD1C;
const FDC_DRIVE_MOTOR: u16 = 0xFD1D;
const FDC_UNUSED: u16 = 0xFD1E;
const FDC_DRQ_IRQ: u16 = 0xFD1F;

/// `0xFD02` IRQ mask register and `0xFD03` IRQ status register.
const IRQ_MASK: u16 = 0xFD02;
const IRQ_STATUS: u16 = 0xFD03;
/// `0xFD02` bit 4 enabling the FDC IRQ to reach the CPU.
const IRQ_MASK_FDC: u8 = 0x10;
/// `0xFD03` active-low external (FDC/OPN) IRQ pending bit.
const IRQ_STATUS_EXTERNAL: u8 = 0x08;

/// WD1793 command opcodes.
const CMD_RESTORE: u8 = 0x00;
const CMD_READ_SECTOR: u8 = 0x80;

/// Status register bits.
const STATUS_BUSY: u8 = 0x01;
const STATUS_DRQ: u8 = 0x02;

/// `0xFD1D` bit 7: motor request (write) / motor spinning (read).
const MOTOR_BIT: u8 = 0x80;
/// `0xFD1F` DRQ and IRQ status bits.
const DRQ_BIT: u8 = 0x80;
const IRQ_BIT: u8 = 0x40;

/// Builds a 256-byte sector whose data ramps from `first_value`.
fn make_sector(record: u8, first_value: u8) -> D88Sector {
    D88Sector {
        cylinder: 0,
        head: 0,
        record,
        size_code: 1,
        sector_count: 1,
        mfm_flag: 0x40,
        deleted: 0x00,
        status: 0x00,
        reserved: [0; 5],
        data: (0..256)
            .map(|index| first_value.wrapping_add(index as u8))
            .collect(),
        source_offset: None,
    }
}

/// A single-track 2D disk carrying `sectors` on cylinder 0, side 0.
fn synthetic_disk(sectors: Vec<D88Sector>) -> FloppyImage {
    FloppyImage::from_d88(D88Disk::from_tracks(
        String::from("TEST"),
        false,
        D88MediaType::Disk2D,
        vec![Some(sectors)],
    ))
}

/// Mounts a one-sector disk in drive 0 and selects it with the motor on.
fn mount_and_select(bus: &mut Fm7Bus, first_value: u8) {
    bus.insert_floppy(
        0,
        synthetic_disk(vec![make_sector(1, first_value)]),
        PathBuf::from("test.d88"),
    );
    bus.write_byte(FDC_DRIVE_MOTOR, MOTOR_BIT); // drive 0, motor on
    bus.write_byte(FDC_SIDE, 0x00); // side 0
}

#[test]
fn read_sector_streams_the_data_over_pio() {
    let mut bus = build_bus_with_synthetic_roms(BootMode::Dos, |_| {});
    mount_and_select(&mut bus, 0x10);

    bus.write_byte(FDC_TRACK, 0);
    bus.write_byte(FDC_SECTOR, 1);
    bus.write_byte(FDC_STATUS_COMMAND, CMD_READ_SECTOR);

    // Let the controller settle and stage the sector buffer.
    run_bus_cycles(&mut bus, 100_000);

    let status = bus.read_byte(FDC_STATUS_COMMAND);
    assert_eq!(status & STATUS_DRQ, STATUS_DRQ, "DRQ should be asserted");

    let data: Vec<u8> = (0..256).map(|_| bus.read_byte(FDC_DATA)).collect();
    let expected: Vec<u8> = (0..256)
        .map(|index| 0x10u8.wrapping_add(index as u8))
        .collect();
    assert_eq!(data, expected);

    // The transfer completes: DRQ and BUSY drop once the last byte is read.
    let status = bus.read_byte(FDC_STATUS_COMMAND);
    assert_eq!(status & (STATUS_DRQ | STATUS_BUSY), 0);
}

#[test]
fn restore_seeks_to_track_zero() {
    let mut bus = build_bus_with_synthetic_roms(BootMode::Dos, |_| {});
    mount_and_select(&mut bus, 0x00);

    // Pretend the head is elsewhere, then restore.
    bus.write_byte(FDC_TRACK, 5);
    bus.write_byte(FDC_STATUS_COMMAND, CMD_RESTORE);
    run_bus_cycles(&mut bus, 200_000);

    let status = bus.read_byte(FDC_STATUS_COMMAND);
    assert_eq!(status & STATUS_BUSY, 0);
    assert_eq!(bus.read_byte(FDC_TRACK), 0);
}

#[test]
fn drq_and_irq_mirror_into_fd1f() {
    let mut bus = build_bus_with_synthetic_roms(BootMode::Dos, |_| {});
    mount_and_select(&mut bus, 0x20);

    bus.write_byte(FDC_TRACK, 0);
    bus.write_byte(FDC_SECTOR, 1);
    bus.write_byte(FDC_STATUS_COMMAND, CMD_READ_SECTOR);
    run_bus_cycles(&mut bus, 100_000);

    // Mid-transfer: DRQ is set, IRQ is not, and the low six bits read as one.
    let status = bus.read_byte(FDC_DRQ_IRQ);
    assert_eq!(status & DRQ_BIT, DRQ_BIT);
    assert_eq!(status & IRQ_BIT, 0);
    assert_eq!(status & 0x3F, 0x3F);

    // Drain the sector; completion raises the controller IRQ.
    for _ in 0..256 {
        bus.read_byte(FDC_DATA);
    }
    let status = bus.read_byte(FDC_DRQ_IRQ);
    assert_eq!(status & IRQ_BIT, IRQ_BIT, "IRQ should mirror on completion");
    assert_eq!(status & DRQ_BIT, 0);

    // 0xFD1E is an AV40-only register and reads open bus on the FM-7.
    assert_eq!(bus.read_byte(FDC_UNUSED), 0xFF);
}

#[test]
fn side_select_reads_back_with_the_high_bits_set() {
    let mut bus = build_bus_with_synthetic_roms(BootMode::Dos, |_| {});

    bus.write_byte(FDC_SIDE, 0x01);
    assert_eq!(bus.read_byte(FDC_SIDE), 0xFF);
    bus.write_byte(FDC_SIDE, 0x00);
    assert_eq!(bus.read_byte(FDC_SIDE), 0xFE);
}

#[test]
fn motor_readback_follows_the_control_latch_immediately() {
    let mut bus = build_bus_with_synthetic_roms(BootMode::Dos, |_| {});
    bus.insert_floppy(
        0,
        synthetic_disk(vec![make_sector(1, 0x00)]),
        PathBuf::from("test.d88"),
    );

    // The boot ROM checks the bit a few microseconds after switching the motor
    // on, so the readback reflects the written latch without the spin-up delay.
    bus.write_byte(FDC_DRIVE_MOTOR, MOTOR_BIT);
    assert_eq!(bus.read_byte(FDC_DRIVE_MOTOR) & MOTOR_BIT, MOTOR_BIT);
    run_bus_cycles(&mut bus, 600_000);
    assert_eq!(bus.read_byte(FDC_DRIVE_MOTOR) & MOTOR_BIT, MOTOR_BIT);

    // Releasing the motor clears the latch readback immediately as well.
    bus.write_byte(FDC_DRIVE_MOTOR, 0x00);
    assert_eq!(bus.read_byte(FDC_DRIVE_MOTOR) & MOTOR_BIT, 0);
}

#[test]
fn drive_select_reads_back_and_gates_the_motor_on_fitted_drives() {
    let mut bus = build_bus_with_synthetic_roms(BootMode::Dos, |_| {});

    // Selecting drive 2 (fitted on the FM-7's four selects) reads the bits back.
    bus.write_byte(FDC_DRIVE_MOTOR, 0x02);
    assert_eq!(bus.read_byte(FDC_DRIVE_MOTOR), 0x3E);
}

#[test]
fn unfitted_drive_never_reports_the_motor() {
    // The FM-77AV fits only two of the four drive selects, so an unfitted select
    // never reports the motor even after spin-up.
    let mut bus = Fm7Bus::new(Fm7Model::Fm77Av, BootMode::Basic, 48_000);
    bus.load_roms(&synthetic_roms(Fm7Model::Fm77Av));
    bus.insert_floppy(
        2,
        synthetic_disk(vec![make_sector(1, 0x00)]),
        PathBuf::from("test.d88"),
    );

    bus.write_byte(FDC_DRIVE_MOTOR, MOTOR_BIT | 0x02); // drive 2, motor on
    run_bus_cycles(&mut bus, 2_000_000);
    assert_eq!(bus.read_byte(FDC_DRIVE_MOTOR) & MOTOR_BIT, 0);
    assert_eq!(bus.read_byte(FDC_DRIVE_MOTOR) & 0x03, 0x02);
}

#[test]
fn fdc_irq_reaches_the_cpu_only_when_unmasked() {
    let mut bus = build_bus_with_synthetic_roms(BootMode::Dos, |_| {});
    mount_and_select(&mut bus, 0x00);

    // A completed command raises the controller IRQ.
    bus.write_byte(FDC_TRACK, 5);
    bus.write_byte(FDC_STATUS_COMMAND, CMD_RESTORE);
    run_bus_cycles(&mut bus, 200_000);

    // Masked (default): the IRQ is pending but does not reach the CPU line.
    bus.write_byte(IRQ_MASK, 0x00);
    assert!(!bus.has_irq());

    // 0xFD03 bit 3 reports the external IRQ pending active-low regardless of mask.
    assert_eq!(bus.read_byte(IRQ_STATUS) & IRQ_STATUS_EXTERNAL, 0);

    // Enabling the FDC IRQ mask lets it reach the CPU.
    bus.write_byte(IRQ_MASK, IRQ_MASK_FDC);
    assert!(bus.has_irq());
}

#[test]
fn fdc_ports_are_not_traced_as_unhandled() {
    use common::Tracing;

    #[derive(Default)]
    struct CountingTracer {
        unhandled: u32,
    }

    impl Tracing for CountingTracer {
        fn trace_io_unhandled_read(&mut self, _port: u16) {
            self.unhandled += 1;
        }

        fn trace_io_unhandled_write(&mut self, _port: u16, _value: u8) {
            self.unhandled += 1;
        }
    }

    let mut bus = Fm7Bus::<CountingTracer>::new(Fm7Model::Fm7, BootMode::Dos, 48_000);
    bus.load_roms(&synthetic_roms(Fm7Model::Fm7));

    for port in FDC_STATUS_COMMAND..=FDC_DRQ_IRQ {
        bus.read_byte(port);
        bus.write_byte(port, 0x00);
    }
    assert_eq!(bus.tracer().unhandled, 0);
}
