//! MB8877 floppy controller PIO-path tests.

mod harness;

use std::path::PathBuf;

use device::floppy::{D88Disk, D88MediaType, D88Sector, FloppyImage};
use harness::{build_machine, run_bus_cycles};
use machine_x1::X1Model;

/// FDC register ports.
const FDC_STATUS_COMMAND: u16 = 0x0FF8;
const FDC_TRACK: u16 = 0x0FF9;
const FDC_SECTOR: u16 = 0x0FFA;
const FDC_DATA: u16 = 0x0FFB;
const FDC_CONTROL: u16 = 0x0FFC;
const FDC_CONTROL_MFM: u16 = 0x0FFD;

/// WD1793 command opcodes.
const CMD_RESTORE: u8 = 0x00;
const CMD_READ_SECTOR: u8 = 0x80;

/// Status register bits.
const STATUS_BUSY: u8 = 0x01;
const STATUS_DRQ: u8 = 0x02;

/// Builds a 256-byte sector whose data ramps from `first_value`.
fn make_sector(record: u8, sector_count: u16, first_value: u8) -> D88Sector {
    D88Sector {
        cylinder: 0,
        head: 0,
        record,
        size_code: 1,
        sector_count,
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

fn synthetic_disk(sectors: Vec<D88Sector>) -> FloppyImage {
    FloppyImage::from_d88(D88Disk::from_tracks(
        String::from("TEST"),
        false,
        D88MediaType::Disk2D,
        vec![Some(sectors)],
    ))
}

/// Selects drive 0 (motor on, side 0) and MFM density.
fn select_drive(bus: &mut machine_x1::X1Bus) {
    bus.io_write(FDC_CONTROL, 0x80); // drive 0, motor on
    bus.io_read(FDC_CONTROL_MFM); // MFM density
}

#[test]
fn read_sector_streams_the_data_over_pio() {
    let mut machine = build_machine(X1Model::X1);
    let bus = &mut machine.bus;
    bus.insert_floppy(
        0,
        synthetic_disk(vec![make_sector(1, 1, 0x10)]),
        PathBuf::from("test.d88"),
    );

    select_drive(bus);
    bus.io_write(FDC_TRACK, 0);
    bus.io_write(FDC_SECTOR, 1);
    bus.io_write(FDC_STATUS_COMMAND, CMD_READ_SECTOR);

    // Let the controller settle and stage the sector buffer.
    run_bus_cycles(bus, 100_000);

    let status = bus.io_read(FDC_STATUS_COMMAND);
    assert_eq!(status & STATUS_DRQ, STATUS_DRQ, "DRQ should be asserted");

    let data: Vec<u8> = (0..256).map(|_| bus.io_read(FDC_DATA)).collect();
    let expected: Vec<u8> = (0..256)
        .map(|index| 0x10u8.wrapping_add(index as u8))
        .collect();
    assert_eq!(data, expected);

    // The transfer completes: DRQ and BUSY drop once the last byte is read.
    let status = bus.io_read(FDC_STATUS_COMMAND);
    assert_eq!(status & (STATUS_DRQ | STATUS_BUSY), 0);
}

#[test]
fn restore_seeks_to_track_zero() {
    let mut machine = build_machine(X1Model::X1);
    let bus = &mut machine.bus;
    bus.insert_floppy(
        0,
        synthetic_disk(vec![make_sector(1, 1, 0x00)]),
        PathBuf::from("test.d88"),
    );

    select_drive(bus);
    // Pretend the head is elsewhere, then restore.
    bus.io_write(FDC_TRACK, 5);
    bus.io_write(FDC_STATUS_COMMAND, CMD_RESTORE);
    run_bus_cycles(bus, 200_000);

    // Restore clears busy and zeroes the track register.
    let status = bus.io_read(FDC_STATUS_COMMAND);
    assert_eq!(status & STATUS_BUSY, 0);
    assert_eq!(bus.io_read(FDC_TRACK), 0);
}

#[test]
fn read_without_media_reports_not_ready() {
    let mut machine = build_machine(X1Model::X1);
    let bus = &mut machine.bus;

    select_drive(bus);
    bus.io_write(FDC_SECTOR, 1);
    bus.io_write(FDC_STATUS_COMMAND, CMD_READ_SECTOR);
    run_bus_cycles(bus, 100_000);

    // With no disk the drive is not ready (status bit 7) and no data is staged.
    let status = bus.io_read(FDC_STATUS_COMMAND);
    assert_eq!(status & 0x80, 0x80);
    assert_eq!(status & STATUS_DRQ, 0);
}

/// Z80 DMA control port (mirror 0x1F80-0x1F8F).
const DMA_PORT: u16 = 0x1F80;
/// Z80 DMA WR6 command: load the running address/counter from the programmed values.
const DMA_CMD_LOAD: u8 = 0xCF;
/// Z80 DMA WR6 command: enable the controller.
const DMA_CMD_ENABLE: u8 = 0x87;

/// Programs and arms the DMA for a floppy read into memory the way real
/// loaders do: port A fixed on the FDC data register (0x0FFB) in I/O space as
/// the source, port B incrementing over `target` in memory as the destination,
/// byte operating mode, `block_len + 1` bytes. Optionally arms an end-of-block
/// interrupt vectored through `vector`.
fn program_dma(bus: &mut machine_x1::X1Bus, target: u16, block_len: u16, interrupt: Option<u8>) {
    bus.io_write(DMA_PORT, 0xC3); // WR6: reset
    bus.io_write(DMA_PORT, 0x7D); // WR0: transfer, port A source, addr/len follow
    bus.io_write(DMA_PORT, 0xFB); // port A = FDC data register 0x0FFB
    bus.io_write(DMA_PORT, 0x0F);
    bus.io_write(DMA_PORT, (block_len & 0xFF) as u8);
    bus.io_write(DMA_PORT, (block_len >> 8) as u8);
    bus.io_write(DMA_PORT, 0x2C); // WR1: port A addresses I/O, fixed
    bus.io_write(DMA_PORT, 0x10); // WR2: port B addresses memory, incrementing
    if let Some(vector) = interrupt {
        // WR4: byte mode, port B address low/high and interrupt control follow.
        bus.io_write(DMA_PORT, 0x9D);
        bus.io_write(DMA_PORT, (target & 0xFF) as u8);
        bus.io_write(DMA_PORT, (target >> 8) as u8);
        // Interrupt on end of block, and an interrupt vector follows.
        bus.io_write(DMA_PORT, 0x02 | 0x10);
        bus.io_write(DMA_PORT, vector);
        // WR3: enable interrupts.
        bus.io_write(DMA_PORT, 0xA0);
    } else {
        // WR4: byte mode, port B address low/high follow.
        bus.io_write(DMA_PORT, 0x8D);
        bus.io_write(DMA_PORT, (target & 0xFF) as u8);
        bus.io_write(DMA_PORT, (target >> 8) as u8);
    }
    bus.io_write(DMA_PORT, DMA_CMD_LOAD);
    bus.io_write(DMA_PORT, DMA_CMD_ENABLE);
}

#[test]
fn turbo_read_sector_transfers_over_dma() {
    let mut machine = build_machine(X1Model::X1Turbo);
    let bus = &mut machine.bus;
    bus.insert_floppy(
        0,
        synthetic_disk(vec![make_sector(1, 1, 0x10)]),
        PathBuf::from("test.d88"),
    );

    // Target the always-RAM upper half so the read-back is unambiguous.
    program_dma(bus, 0x8000, 0x00FF, None);

    select_drive(bus);
    bus.io_write(FDC_TRACK, 0);
    bus.io_write(FDC_SECTOR, 1);
    bus.io_write(FDC_STATUS_COMMAND, CMD_READ_SECTOR);
    run_bus_cycles(bus, 100_000);

    // The DMA moved the whole sector into work RAM; the bytes match the PIO test's
    // expected ramp.
    let data: Vec<u8> = (0..256)
        .map(|index| bus.peek_byte(0x8000 + index))
        .collect();
    let expected: Vec<u8> = (0..256)
        .map(|index| 0x10u8.wrapping_add(index as u8))
        .collect();
    assert_eq!(data, expected);

    // The DMA path never asserts the PIO DRQ handshake; the command completes.
    let status = bus.io_read(FDC_STATUS_COMMAND);
    assert_eq!(status & (STATUS_DRQ | STATUS_BUSY), 0);
}

/// Programs the DMA the way the Arcus loader does for a VRAM-bound read: port A
/// fixed on the FDC data register (0x0FFB) in I/O space as the source, port B
/// incrementing over `target` in I/O space as the destination.
fn program_dma_to_io(bus: &mut machine_x1::X1Bus, target: u16, block_len: u16) {
    bus.io_write(DMA_PORT, 0xC3); // WR6: reset
    bus.io_write(DMA_PORT, 0x83); // WR6: disable
    bus.io_write(DMA_PORT, 0x7D); // WR0: mode 1, port A source, addr/len follow
    bus.io_write(DMA_PORT, 0xFB);
    bus.io_write(DMA_PORT, 0x0F);
    bus.io_write(DMA_PORT, (block_len & 0xFF) as u8);
    bus.io_write(DMA_PORT, (block_len >> 8) as u8);
    bus.io_write(DMA_PORT, 0x2C); // WR1: port A addresses I/O, fixed
    bus.io_write(DMA_PORT, 0x18); // WR2: port B addresses I/O, incrementing
    bus.io_write(DMA_PORT, 0x8D); // WR4: port B address low/high follow
    bus.io_write(DMA_PORT, (target & 0xFF) as u8);
    bus.io_write(DMA_PORT, (target >> 8) as u8);
    bus.io_write(DMA_PORT, DMA_CMD_LOAD);
    bus.io_write(DMA_PORT, DMA_CMD_ENABLE);
}

#[test]
fn turbo_dma_with_io_destination_streams_into_bitmap_vram() {
    let mut machine = build_machine(X1Model::X1Turbo);
    let bus = &mut machine.bus;
    bus.insert_floppy(
        0,
        synthetic_disk(vec![make_sector(1, 1, 0x10)]),
        PathBuf::from("test.d88"),
    );

    // Games stream sector data straight into the I/O-mapped bitmap VRAM; the
    // blue-plane window starts at port 0x4000.
    program_dma_to_io(bus, 0x4000, 0x00FF);

    select_drive(bus);
    bus.io_write(FDC_TRACK, 0);
    bus.io_write(FDC_SECTOR, 1);
    bus.io_write(FDC_STATUS_COMMAND, CMD_READ_SECTOR);
    run_bus_cycles(bus, 100_000);

    let data: Vec<u8> = (0..256).map(|index| bus.io_read(0x4000 + index)).collect();
    let expected: Vec<u8> = (0..256)
        .map(|index| 0x10u8.wrapping_add(index as u8))
        .collect();
    assert_eq!(data, expected);

    // The bytes went to VRAM through I/O cycles, not to work RAM.
    assert!((0..256).all(|index| bus.peek_byte(0x4000 + index) == 0));

    let status = bus.io_read(FDC_STATUS_COMMAND);
    assert_eq!(status & (STATUS_DRQ | STATUS_BUSY), 0);
}

#[test]
fn turbo_dma_end_of_block_raises_its_interrupt() {
    let mut machine = build_machine(X1Model::X1Turbo);
    let bus = &mut machine.bus;
    bus.insert_floppy(
        0,
        synthetic_disk(vec![make_sector(1, 1, 0x00)]),
        PathBuf::from("test.d88"),
    );

    program_dma(bus, 0x8000, 0x00FF, Some(0x60));
    assert!(!bus.has_irq());

    select_drive(bus);
    bus.io_write(FDC_TRACK, 0);
    bus.io_write(FDC_SECTOR, 1);
    bus.io_write(FDC_STATUS_COMMAND, CMD_READ_SECTOR);
    run_bus_cycles(bus, 100_000);

    // The end-of-block interrupt vectors through the DMA's programmed vector.
    assert!(bus.has_irq());
    assert_eq!(bus.acknowledge_irq(), 0x60);
    assert!(!bus.has_irq());
}

#[test]
fn base_x1_uses_pio_not_dma() {
    // The base X1 has no DMA controller: 0x1F80 is open bus and the FDC still
    // streams over the PIO DRQ path.
    let mut machine = build_machine(X1Model::X1);
    let bus = &mut machine.bus;
    bus.insert_floppy(
        0,
        synthetic_disk(vec![make_sector(1, 1, 0x10)]),
        PathBuf::from("test.d88"),
    );
    assert_eq!(bus.io_read(DMA_PORT), 0xFF);

    select_drive(bus);
    bus.io_write(FDC_TRACK, 0);
    bus.io_write(FDC_SECTOR, 1);
    bus.io_write(FDC_STATUS_COMMAND, CMD_READ_SECTOR);
    run_bus_cycles(bus, 100_000);

    let status = bus.io_read(FDC_STATUS_COMMAND);
    assert_eq!(status & STATUS_DRQ, STATUS_DRQ);
}
