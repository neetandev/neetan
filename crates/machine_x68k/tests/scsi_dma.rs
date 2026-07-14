//! Integration tests for the SUPER/XVI internal MB89352 SCSI path: register
//! decode at 0xE96020, selection interrupts through the IOC with the fixed
//! vector 0x6C, program transfers through DREG, and DMA transfers over
//! HD63450 channel 1.

#[path = "common/harness.rs"]
mod harness;
#[path = "common/spc.rs"]
mod spc;

use common::Bus;
use harness::{
    machine, patterned_scsi_hdf, program_storage_dma, read_byte, run_pending_events, write_byte,
};
use machine_x68k::{X68kMachine, X68kModel};
use spc::{
    INTS_COMMAND_COMPLETE, INTS_TIME_OUT, PHASE_COMMAND, PHASE_DATA_IN, PHASE_DATA_OUT,
    PSNS_REQUEST, PSNS_SELECT, SCMD_SELECT, SCMD_TRANSFER, SCTL_INTERRUPT_ENABLE,
    SCTL_RESET_AND_DISABLE, SPC_BDID, SPC_DREG, SPC_INTS, SPC_PCTL, SPC_PSNS, SPC_SCMD, SPC_SCTL,
    SPC_SSTS, SPC_TCH, SPC_TCL, SPC_TCM, SPC_TEMP, SSTS_CONNECTED_INITIATOR, SSTS_FIFO_EMPTY,
    SSTS_TRANSFER_COUNTER_ZERO, read_data_in, read_status_and_message, select, send_command,
    set_transfer_counter, wait_for_interrupt_bit,
};

/// IOC status and mask register.
const IOC_STATUS: u32 = 0xE9C001;
/// IOC vector-base register.
const IOC_VECTOR: u32 = 0xE9C003;

/// Bytes in one SCSI hard-disk sector.
const SECTOR_SIZE: usize = 512;

/// Builds a SCSI-model machine with one patterned 2 MiB disk at SCSI ID 0
/// and the IOC vector base programmed away from the fixed SPC vector.
fn machine_with_disk(model: X68kModel) -> X68kMachine {
    let mut machine = machine(model);
    machine.insert_hdd(0, patterned_scsi_hdf(2), None).unwrap();
    write_byte(&mut machine, IOC_STATUS, 0x0F);
    write_byte(&mut machine, IOC_VECTOR, 0x40);
    machine
}

/// Builds the SUPER machine with the patterned disk.
fn super_machine_with_disk() -> X68kMachine {
    machine_with_disk(X68kModel::X68000Super)
}

/// The expected contents of one patterned 512-byte SCSI sector.
fn expected_sector(lba: u32) -> Vec<u8> {
    let mut sector = vec![(lba as u8) ^ 0xA5; SECTOR_SIZE];
    sector[..4].copy_from_slice(&lba.to_le_bytes());
    sector
}

#[test]
fn register_defaults_and_temp_loopback_through_the_bus() {
    let mut machine = machine(X68kModel::X68000Super);
    assert_eq!(read_byte(&mut machine, SPC_BDID), 0x80);
    assert_eq!(read_byte(&mut machine, SPC_SCTL), SCTL_RESET_AND_DISABLE);
    assert_eq!(read_byte(&mut machine, SPC_INTS), 0);
    assert_eq!(read_byte(&mut machine, SPC_PSNS), 0);
    assert_eq!(
        read_byte(&mut machine, SPC_SSTS),
        SSTS_TRANSFER_COUNTER_ZERO | SSTS_FIFO_EMPTY
    );

    write_byte(&mut machine, SPC_TEMP, 0xA5);
    assert_eq!(read_byte(&mut machine, SPC_TEMP), 0xA5);
    write_byte(&mut machine, SPC_BDID, 3);
    assert_eq!(read_byte(&mut machine, SPC_BDID), 0x08);

    set_transfer_counter(&mut machine, 0x123456);
    assert_eq!(read_byte(&mut machine, SPC_TCH), 0x12);
    assert_eq!(read_byte(&mut machine, SPC_TCM), 0x34);
    assert_eq!(read_byte(&mut machine, SPC_TCL), 0x56);
}

#[test]
fn selection_timeout_latches_ints_and_gates_the_ioc_interrupt() {
    let mut machine = machine(X68kModel::X68000Super);
    write_byte(&mut machine, IOC_STATUS, 0x0F);
    write_byte(&mut machine, IOC_VECTOR, 0x40);

    // Interrupt enable clear: INTS latches, no IOC request appears.
    write_byte(&mut machine, SPC_SCTL, 0);
    write_byte(&mut machine, SPC_PCTL, 0);
    write_byte(&mut machine, SPC_TEMP, 0x80 | 0x01);
    write_byte(&mut machine, SPC_SCMD, SCMD_SELECT);
    wait_for_interrupt_bit(&mut machine, INTS_TIME_OUT);
    assert_ne!(read_byte(&mut machine, SPC_PSNS) & PSNS_SELECT, 0);
    assert_eq!(machine.bus.m68000_interrupt_level(), 0);

    // Setting interrupt enable raises the line for the latched event; the
    // internal SPC always answers with the fixed vector 0x6C, never with
    // the programmed IOC base.
    write_byte(&mut machine, SPC_SCTL, SCTL_INTERRUPT_ENABLE);
    assert_eq!(machine.bus.m68000_interrupt_level(), 1);
    assert_eq!(machine.bus.m68000_acknowledge_interrupt(1), 0x6C);
    assert_eq!(machine.bus.m68000_interrupt_level(), 0);

    // Clearing the timeout releases SEL and recovers the bus.
    write_byte(&mut machine, SPC_INTS, INTS_TIME_OUT);
    assert_eq!(read_byte(&mut machine, SPC_INTS), 0);
    assert_eq!(read_byte(&mut machine, SPC_PSNS) & PSNS_SELECT, 0);
}

#[test]
fn selection_success_raises_the_fixed_spc_vector() {
    let mut machine = super_machine_with_disk();
    write_byte(&mut machine, SPC_SCTL, SCTL_INTERRUPT_ENABLE);
    write_byte(&mut machine, SPC_PCTL, 0);
    write_byte(&mut machine, SPC_TEMP, 0x80 | 0x01);
    write_byte(&mut machine, SPC_SCMD, SCMD_SELECT);
    wait_for_interrupt_bit(&mut machine, INTS_COMMAND_COMPLETE);

    assert_eq!(machine.bus.m68000_interrupt_level(), 1);
    assert_eq!(machine.bus.m68000_acknowledge_interrupt(1), 0x6C);
    assert_ne!(
        read_byte(&mut machine, SPC_SSTS) & SSTS_CONNECTED_INITIATOR,
        0
    );
    assert_eq!(
        read_byte(&mut machine, SPC_PSNS),
        PSNS_REQUEST | PHASE_COMMAND
    );

    // Clearing INTS drops the line without a stale IOC request.
    write_byte(&mut machine, SPC_INTS, 0xFF);
    assert_eq!(machine.bus.m68000_interrupt_level(), 0);
}

#[test]
fn inquiry_identifies_the_scsi_disk() {
    let mut machine = super_machine_with_disk();
    select(&mut machine, 0);
    send_command(&mut machine, &[0x12, 0, 0, 0, 36, 0]);
    let data = read_data_in(&mut machine, 36);
    assert_eq!(data[0], 0x00, "direct-access device type");
    assert_eq!(read_status_and_message(&mut machine), 0);
}

#[test]
fn program_transfer_read_returns_the_patterned_sector() {
    let mut machine = super_machine_with_disk();
    select(&mut machine, 0);
    send_command(&mut machine, &[0x28, 0, 0, 0, 0, 3, 0, 0, 1, 0]);
    let data = read_data_in(&mut machine, SECTOR_SIZE);
    assert_eq!(data, expected_sector(3));
    assert_eq!(read_status_and_message(&mut machine), 0);
}

#[test]
fn dma_read_lands_the_sector_in_ram_on_both_scsi_models() {
    for model in [X68kModel::X68000Super, X68kModel::X68000Xvi] {
        let mut machine = machine_with_disk(model);
        let buffer = 0x2000u32;
        program_storage_dma(&mut machine, buffer, SPC_DREG, SECTOR_SIZE as u16, true);

        select(&mut machine, 0);
        send_command(&mut machine, &[0x28, 0, 0, 0, 0, 5, 0, 0, 1, 0]);
        assert_eq!(
            read_byte(&mut machine, SPC_PSNS),
            PSNS_REQUEST | PHASE_DATA_IN
        );
        write_byte(&mut machine, SPC_PCTL, PHASE_DATA_IN);
        set_transfer_counter(&mut machine, SECTOR_SIZE as u32);
        // Transfer without the program-transfer bit: the bytes flow over DMAC
        // channel 1 from DREG into memory.
        write_byte(&mut machine, SPC_SCMD, SCMD_TRANSFER);
        run_pending_events(&mut machine, 16);

        let expected = expected_sector(5);
        for (index, &byte) in expected.iter().enumerate() {
            assert_eq!(
                machine.bus.ram_byte(buffer + index as u32),
                Some(byte),
                "{model}: sector byte {index}"
            );
        }
        assert_ne!(
            read_byte(&mut machine, harness::DMAC_CHANNEL1_BASE) & 0x80,
            0,
            "{model}: channel 1 reports operation complete"
        );
        write_byte(&mut machine, SPC_INTS, 0xFF);
        assert_eq!(read_status_and_message(&mut machine), 0);
    }
}

#[test]
fn dma_write_round_trips_through_the_drive() {
    let mut machine = super_machine_with_disk();
    let buffer = 0x3000u32;
    let payload: Vec<u8> = (0..SECTOR_SIZE)
        .map(|index| (index as u8).wrapping_mul(7))
        .collect();
    for (index, &byte) in payload.iter().enumerate() {
        write_byte(&mut machine, buffer + index as u32, byte);
    }

    select(&mut machine, 0);
    send_command(&mut machine, &[0x2A, 0, 0, 0, 0, 7, 0, 0, 1, 0]);
    assert_eq!(
        read_byte(&mut machine, SPC_PSNS),
        PSNS_REQUEST | PHASE_DATA_OUT
    );
    write_byte(&mut machine, SPC_PCTL, PHASE_DATA_OUT);
    set_transfer_counter(&mut machine, SECTOR_SIZE as u32);
    write_byte(&mut machine, SPC_SCMD, SCMD_TRANSFER);
    // The DMAC is armed after the Transfer command; arming pumps the
    // pending request immediately.
    program_storage_dma(&mut machine, buffer, SPC_DREG, SECTOR_SIZE as u16, false);
    run_pending_events(&mut machine, 16);
    assert_ne!(
        read_byte(&mut machine, harness::DMAC_CHANNEL1_BASE) & 0x80,
        0,
        "channel 1 reports operation complete"
    );
    write_byte(&mut machine, SPC_INTS, 0xFF);
    assert_eq!(read_status_and_message(&mut machine), 0);

    select(&mut machine, 0);
    send_command(&mut machine, &[0x28, 0, 0, 0, 0, 7, 0, 0, 1, 0]);
    let data = read_data_in(&mut machine, SECTOR_SIZE);
    assert_eq!(data, payload);
    assert_eq!(read_status_and_message(&mut machine), 0);
}

#[test]
fn sctl_reset_releases_the_bus_mid_phase() {
    let mut machine = super_machine_with_disk();
    select(&mut machine, 0);
    send_command(&mut machine, &[0x28, 0, 0, 0, 0, 3, 0, 0, 1, 0]);
    assert_eq!(
        read_byte(&mut machine, SPC_PSNS),
        PSNS_REQUEST | PHASE_DATA_IN
    );

    write_byte(&mut machine, SPC_SCTL, SCTL_RESET_AND_DISABLE);
    assert_eq!(read_byte(&mut machine, SPC_PSNS), 0);
    assert_eq!(read_byte(&mut machine, SPC_INTS), 0);
    assert_eq!(machine.bus.m68000_interrupt_level(), 0);

    // The controller recovers after the reset is released.
    select(&mut machine, 0);
    send_command(&mut machine, &[0x00, 0, 0, 0, 0, 0]);
    assert_eq!(read_status_and_message(&mut machine), 0);
}
