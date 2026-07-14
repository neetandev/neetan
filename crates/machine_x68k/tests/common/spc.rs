//! Shared MB89352 SPC protocol helpers for the internal-SCSI integration
//! tests: register addresses, chip constants, and program-transfer phase
//! walks driven through the supervisor bus.

#![allow(dead_code)]

use machine_x68k::X68kMachine;

use super::harness::{read_byte, run_pending_events, write_byte};

/// SPC bus device ID register.
pub const SPC_BDID: u32 = 0xE96021;
/// SPC control register.
pub const SPC_SCTL: u32 = 0xE96023;
/// SPC command register.
pub const SPC_SCMD: u32 = 0xE96025;
/// SPC interrupt sense register.
pub const SPC_INTS: u32 = 0xE96029;
/// SPC phase sense register.
pub const SPC_PSNS: u32 = 0xE9602B;
/// SPC status register.
pub const SPC_SSTS: u32 = 0xE9602D;
/// SPC phase control register.
pub const SPC_PCTL: u32 = 0xE96031;
/// SPC data register.
pub const SPC_DREG: u32 = 0xE96035;
/// SPC temporary register.
pub const SPC_TEMP: u32 = 0xE96037;
/// SPC transfer counter bits 16-23.
pub const SPC_TCH: u32 = 0xE96039;
/// SPC transfer counter bits 8-15.
pub const SPC_TCM: u32 = 0xE9603B;
/// SPC transfer counter bits 0-7.
pub const SPC_TCL: u32 = 0xE9603D;

/// SCTL: hold the controller in reset while set.
pub const SCTL_RESET_AND_DISABLE: u8 = 0x80;
/// SCTL: enable the interrupt line.
pub const SCTL_INTERRUPT_ENABLE: u8 = 0x01;
/// SCMD command: start a selection.
pub const SCMD_SELECT: u8 = 0x20;
/// SCMD command: start a transfer.
pub const SCMD_TRANSFER: u8 = 0x80;
/// SCMD: program (CPU) transfer; clear means DMA transfer with DREQ.
pub const SCMD_PROGRAM_TRANSFER: u8 = 0x04;
/// INTS: a Select or Transfer command completed.
pub const INTS_COMMAND_COMPLETE: u8 = 0x10;
/// INTS: the selection received no response.
pub const INTS_TIME_OUT: u8 = 0x04;
/// PSNS: the target requests a byte handshake.
pub const PSNS_REQUEST: u8 = 0x80;
/// PSNS: the SEL line (held through a selection timeout).
pub const PSNS_SELECT: u8 = 0x10;
/// SSTS: the SPC is connected as the initiator.
pub const SSTS_CONNECTED_INITIATOR: u8 = 0x80;
/// SSTS: the transfer counter reached zero.
pub const SSTS_TRANSFER_COUNTER_ZERO: u8 = 0x04;
/// SSTS: the DREG FIFO is empty.
pub const SSTS_FIFO_EMPTY: u8 = 0x01;
/// Bus phase code: data out (initiator to target).
pub const PHASE_DATA_OUT: u8 = 0x00;
/// Bus phase code: data in (target to initiator).
pub const PHASE_DATA_IN: u8 = 0x01;
/// Bus phase code: command.
pub const PHASE_COMMAND: u8 = 0x02;
/// Bus phase code: status.
pub const PHASE_STATUS: u8 = 0x03;
/// Bus phase code: message in.
pub const PHASE_MESSAGE_IN: u8 = 0x07;

/// Runs device events until the INTS bit latches.
pub fn wait_for_interrupt_bit(machine: &mut X68kMachine, bit: u8) {
    for _ in 0..64 {
        if read_byte(machine, SPC_INTS) & bit != 0 {
            return;
        }
        run_pending_events(machine, 1);
    }
    panic!("SPC interrupt bit {bit:#04X} never latched");
}

/// Loads the 24-bit transfer counter.
pub fn set_transfer_counter(machine: &mut X68kMachine, count: u32) {
    write_byte(machine, SPC_TCH, (count >> 16) as u8);
    write_byte(machine, SPC_TCM, (count >> 8) as u8);
    write_byte(machine, SPC_TCL, count as u8);
}

/// Selects the target at the given ID and clears the completion event.
pub fn select(machine: &mut X68kMachine, id: u8) {
    write_byte(machine, SPC_SCTL, SCTL_INTERRUPT_ENABLE);
    write_byte(machine, SPC_PCTL, 0);
    write_byte(machine, SPC_TEMP, 0x80 | (1 << id));
    write_byte(machine, SPC_SCMD, SCMD_SELECT);
    wait_for_interrupt_bit(machine, INTS_COMMAND_COMPLETE);
    write_byte(machine, SPC_INTS, 0xFF);
    assert_eq!(read_byte(machine, SPC_PSNS), PSNS_REQUEST | PHASE_COMMAND);
}

/// Delivers a CDB with a program transfer through DREG.
pub fn send_command(machine: &mut X68kMachine, cdb: &[u8]) {
    assert_eq!(read_byte(machine, SPC_PSNS), PSNS_REQUEST | PHASE_COMMAND);
    write_byte(machine, SPC_PCTL, PHASE_COMMAND);
    set_transfer_counter(machine, cdb.len() as u32);
    write_byte(machine, SPC_SCMD, SCMD_TRANSFER | SCMD_PROGRAM_TRANSFER);
    for &byte in cdb {
        write_byte(machine, SPC_DREG, byte);
    }
    write_byte(machine, SPC_INTS, 0xFF);
}

/// Drains a DATA IN payload of the given length with a program transfer.
pub fn read_data_in(machine: &mut X68kMachine, length: usize) -> Vec<u8> {
    assert_eq!(read_byte(machine, SPC_PSNS), PSNS_REQUEST | PHASE_DATA_IN);
    write_byte(machine, SPC_PCTL, PHASE_DATA_IN);
    set_transfer_counter(machine, length as u32);
    write_byte(machine, SPC_SCMD, SCMD_TRANSFER | SCMD_PROGRAM_TRANSFER);
    let data: Vec<u8> = (0..length).map(|_| read_byte(machine, SPC_DREG)).collect();
    write_byte(machine, SPC_INTS, 0xFF);
    data
}

/// Reads the status and message bytes, returning the status.
pub fn read_status_and_message(machine: &mut X68kMachine) -> u8 {
    assert_eq!(read_byte(machine, SPC_PSNS), PSNS_REQUEST | PHASE_STATUS);
    write_byte(machine, SPC_PCTL, PHASE_STATUS);
    set_transfer_counter(machine, 1);
    write_byte(machine, SPC_SCMD, SCMD_TRANSFER | SCMD_PROGRAM_TRANSFER);
    let status_byte = read_byte(machine, SPC_DREG);
    write_byte(machine, SPC_INTS, 0xFF);
    assert_eq!(
        read_byte(machine, SPC_PSNS),
        PSNS_REQUEST | PHASE_MESSAGE_IN
    );
    write_byte(machine, SPC_PCTL, PHASE_MESSAGE_IN);
    set_transfer_counter(machine, 1);
    write_byte(machine, SPC_SCMD, SCMD_TRANSFER | SCMD_PROGRAM_TRANSFER);
    assert_eq!(read_byte(machine, SPC_DREG), 0x00);
    write_byte(machine, SPC_INTS, 0xFF);
    assert_eq!(read_byte(machine, SPC_PSNS), 0, "bus free after message");
    status_byte
}
