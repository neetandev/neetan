//! Integration tests for the original X68000 SASI hard-disk path: register
//! decode at 0xE96000, the bus-phase walk, IOC interrupt delivery, and DMA
//! transfers over HD63450 channel 1.

#[path = "common/harness.rs"]
mod harness;

use common::Bus;
use harness::{
    machine, patterned_sasi_hdf, patterned_scsi_hdf, program_storage_dma, read_byte,
    run_pending_events, write_byte,
};
use machinex68k::{X68kMachine, X68kModel};

/// SASI data register.
const SASI_DATA: u32 = 0xE96001;
/// SASI bus-status read / command-phase start write register.
const SASI_STATUS: u32 = 0xE96003;
/// SASI selection register.
const SASI_SELECT: u32 = 0xE96007;
/// IOC status and mask register.
const IOC_STATUS: u32 = 0xE9C001;
/// IOC vector-base register.
const IOC_VECTOR: u32 = 0xE9C003;

/// Bus status: command phase (C/D + BSY + REQ).
const STATUS_COMMAND: u8 = 0x0B;
/// Bus status: data-in phase (I/O + BSY + REQ).
const STATUS_DATA_IN: u8 = 0x07;
/// Bus status: status phase (C/D + I/O + BSY + REQ).
const STATUS_STATUS: u8 = 0x0F;
/// Bus status: message phase (MSG + C/D + I/O + BSY + REQ).
const STATUS_MESSAGE: u8 = 0x1F;

/// Builds the SASI machine with one patterned 10 MB drive and an unmasked
/// IOC programmed to vector base 0x40.
fn sasi_machine_with_drive() -> X68kMachine {
    let mut machine = machine(X68kModel::X68000);
    machine.insert_hdd(0, patterned_sasi_hdf(), None).unwrap();
    write_byte(&mut machine, IOC_STATUS, 0x0F);
    write_byte(&mut machine, IOC_VECTOR, 0x40);
    machine
}

/// Selects a SASI ID and delivers a six-byte command.
fn select_and_command(machine: &mut X68kMachine, id: u8, cdb: &[u8; 6]) {
    write_byte(machine, SASI_SELECT, 1 << id);
    assert_ne!(
        read_byte(machine, SASI_STATUS) & 0x02,
        0,
        "BSY after select"
    );
    write_byte(machine, SASI_STATUS, 0);
    assert_eq!(read_byte(machine, SASI_STATUS), STATUS_COMMAND);
    for &byte in cdb {
        write_byte(machine, SASI_DATA, byte);
    }
}

/// Reads the status and message bytes, returning the status byte.
fn read_status_and_message(machine: &mut X68kMachine) -> u8 {
    assert_eq!(read_byte(machine, SASI_STATUS), STATUS_STATUS);
    let status = read_byte(machine, SASI_DATA);
    assert_eq!(read_byte(machine, SASI_STATUS), STATUS_MESSAGE);
    assert_eq!(read_byte(machine, SASI_DATA), 0);
    assert_eq!(read_byte(machine, SASI_STATUS), 0, "bus free after message");
    status
}

/// The expected contents of one patterned 256-byte SASI sector.
fn expected_sector(lba: u32) -> Vec<u8> {
    let mut sector = vec![(lba as u8) ^ 0x5A; 256];
    sector[..4].copy_from_slice(&lba.to_le_bytes());
    sector
}

#[test]
fn selecting_an_empty_id_leaves_the_bus_free() {
    let mut machine = machine(X68kModel::X68000);
    for id in 0..2 {
        write_byte(&mut machine, SASI_SELECT, 1 << id);
        assert_eq!(read_byte(&mut machine, SASI_STATUS), 0, "SASI ID {id}");
    }
}

#[test]
fn selecting_the_unattached_second_id_times_out() {
    let mut machine = sasi_machine_with_drive();
    write_byte(&mut machine, SASI_SELECT, 1 << 1);
    assert_eq!(read_byte(&mut machine, SASI_STATUS), 0);
}

#[test]
fn test_drive_ready_raises_the_ioc_hdc_interrupt() {
    let mut machine = sasi_machine_with_drive();
    select_and_command(&mut machine, 0, &[0x00, 0, 0, 0, 0, 0]);
    assert_eq!(read_status_and_message(&mut machine), 0);

    assert_ne!(
        read_byte(&mut machine, IOC_STATUS) & 0x10,
        0,
        "IOC status reports the HDC request"
    );
    assert_eq!(machine.bus.m68000_interrupt_level(), 1);
    assert_eq!(
        machine.bus.m68000_acknowledge_interrupt(1),
        0x42,
        "HDC uses vector base | 2"
    );
    assert_eq!(machine.bus.m68000_interrupt_level(), 0);
}

#[test]
fn pio_read_returns_the_patterned_sector() {
    let mut machine = sasi_machine_with_drive();
    select_and_command(&mut machine, 0, &[0x08, 0x00, 0x00, 0x05, 1, 0]);
    assert_eq!(read_byte(&mut machine, SASI_STATUS), STATUS_DATA_IN);
    let data: Vec<u8> = (0..256)
        .map(|_| read_byte(&mut machine, SASI_DATA))
        .collect();
    assert_eq!(data, expected_sector(5));
    assert_eq!(read_status_and_message(&mut machine), 0);
}

#[test]
fn dma_read_lands_the_sector_in_ram() {
    let mut machine = sasi_machine_with_drive();
    let buffer = 0x2000u32;
    program_storage_dma(&mut machine, buffer, SASI_DATA, 256, true);
    select_and_command(&mut machine, 0, &[0x08, 0x00, 0x00, 0x07, 1, 0]);
    run_pending_events(&mut machine, 16);

    let expected = expected_sector(7);
    for (index, &byte) in expected.iter().enumerate() {
        assert_eq!(
            machine.bus.ram_byte(buffer + index as u32),
            Some(byte),
            "sector byte {index}"
        );
    }
    assert_ne!(
        read_byte(&mut machine, harness::DMAC_CHANNEL1_BASE) & 0x80,
        0,
        "channel 1 reports operation complete"
    );
    assert_eq!(read_status_and_message(&mut machine), 0);
}

#[test]
fn dma_write_round_trips_through_the_drive() {
    let mut machine = sasi_machine_with_drive();
    let buffer = 0x3000u32;
    let payload: Vec<u8> = (0..256)
        .map(|index| (index as u8).wrapping_mul(3))
        .collect();
    for (index, &byte) in payload.iter().enumerate() {
        write_byte(&mut machine, buffer + index as u32, byte);
    }

    // Command first, DMAC armed afterwards: the settling delay before the
    // data-phase request keeps the transfer from being missed.
    select_and_command(&mut machine, 0, &[0x0A, 0x00, 0x00, 0x09, 1, 0]);
    program_storage_dma(&mut machine, buffer, SASI_DATA, 256, false);
    run_pending_events(&mut machine, 16);
    assert_eq!(read_status_and_message(&mut machine), 0);
    assert_ne!(
        read_byte(&mut machine, harness::DMAC_CHANNEL1_BASE) & 0x80,
        0,
        "channel 1 reports operation complete"
    );

    select_and_command(&mut machine, 0, &[0x08, 0x00, 0x00, 0x09, 1, 0]);
    let data: Vec<u8> = (0..256)
        .map(|_| read_byte(&mut machine, SASI_DATA))
        .collect();
    assert_eq!(data, payload);
    assert_eq!(read_status_and_message(&mut machine), 0);
}

#[test]
fn out_of_range_read_reports_invalid_sector_sense() {
    let mut machine = sasi_machine_with_drive();
    // 10 MB drive: 40788 sectors; LBA 40788 is one past the end.
    select_and_command(&mut machine, 0, &[0x08, 0x00, 0x9F, 0x54, 1, 0]);
    assert_ne!(read_status_and_message(&mut machine), 0);

    select_and_command(&mut machine, 0, &[0x03, 0, 0, 0, 0, 0]);
    let sense: Vec<u8> = (0..4).map(|_| read_byte(&mut machine, SASI_DATA)).collect();
    assert_eq!(sense[0], 0x21, "INVALID SECTOR ADDRESS");
    assert_eq!(read_status_and_message(&mut machine), 0);
}

#[test]
fn format_block_zero_fills_the_track() {
    let mut machine = sasi_machine_with_drive();
    select_and_command(&mut machine, 0, &[0x06, 0x00, 0x00, 33, 0, 0]);
    assert_eq!(read_status_and_message(&mut machine), 0);

    // The 33 formatted sectors read back as zeroes.
    select_and_command(&mut machine, 0, &[0x08, 0x00, 0x00, 33, 33, 0]);
    for index in 0..33 * 256 {
        assert_eq!(read_byte(&mut machine, SASI_DATA), 0, "byte {index}");
    }
    assert_eq!(read_status_and_message(&mut machine), 0);

    // The following sector is untouched.
    select_and_command(&mut machine, 0, &[0x08, 0x00, 0x00, 66, 1, 0]);
    let data: Vec<u8> = (0..256)
        .map(|_| read_byte(&mut machine, SASI_DATA))
        .collect();
    assert_eq!(data, expected_sector(66));
    assert_eq!(read_status_and_message(&mut machine), 0);
}

#[test]
fn second_drive_answers_at_sasi_id_one() {
    let mut machine = sasi_machine_with_drive();
    machine.insert_hdd(1, patterned_sasi_hdf(), None).unwrap();
    assert_eq!(
        machine.bus.sram_data()[machinex68k::SASI_HDMAX_OFFSET],
        2,
        "the SRAM boot scan covers both units"
    );

    select_and_command(&mut machine, 1, &[0x00, 0, 0, 0, 0, 0]);
    assert_eq!(read_status_and_message(&mut machine), 0);
}

#[test]
fn insert_hdd_validates_slot_and_sector_size() {
    let mut machine = machine(X68kModel::X68000);
    assert!(machine.insert_hdd(2, patterned_sasi_hdf(), None).is_err());
    assert!(
        machine.insert_hdd(0, patterned_scsi_hdf(2), None).is_err(),
        "the SASI model rejects 512-byte-sector images"
    );
    assert_eq!(machine.bus.sram_data()[machinex68k::SASI_HDMAX_OFFSET], 0);
}
