//! Integration tests for the FM Towns SCSI + uPD71071 DMA path: the IPL and the
//! OS transfer disk sectors by programming the DMAC over its I/O ports and
//! driving the MB89352-class SCSI controller, so these exercise that whole chain
//! against an in-memory disk image.

#[path = "common/harness.rs"]
mod harness;

use common::Bus;
use device::{disk::HddImage, scsi::command::opcode};
use harness::{machine_mx, program_dma_channel};

/// The SCSI SPC drives main-DMA channel 1.
const SCSI_DMA_CHANNEL: u8 = 1;
/// SCSI host-interface ports.
const SCSI_DATA: u16 = 0x0C30;
const SCSI_CONTROL: u16 = 0x0C32;
const INITIATOR_MASK: u8 = 1 << 7;

/// Selects target 0, sends a CDB, and advances the clock past the SCSI task so
/// its scheduled data transfer runs.
fn run_command(machine: &mut machine_towns::TownsMachine<{ cpu::CPU_MODEL_486_DX }>, cdb: &[u8]) {
    machine
        .bus
        .io_write_byte(SCSI_DATA, (1 << 0) | INITIATOR_MASK);
    machine.bus.io_write_byte(SCSI_CONTROL, 0x04); // SEL asserted
    machine.bus.io_write_byte(SCSI_CONTROL, 0x00); // SEL released -> COMMAND
    for &byte in cdb {
        machine.bus.io_write_byte(SCSI_DATA, byte);
    }
    let deadline = machine.bus.current_cycle() + 1_000_000;
    machine.bus.set_current_cycle(deadline);
}

/// Reads back STATUS and MESSAGE IN so the bus returns to BUS FREE.
fn drain_status(machine: &mut machine_towns::TownsMachine<{ cpu::CPU_MODEL_486_DX }>) {
    machine.bus.io_read_byte(SCSI_DATA);
    machine.bus.io_read_byte(SCSI_DATA);
}

#[test]
fn read10_dmas_sector_from_disk_to_memory() {
    let mut machine = machine_mx();
    // A 128 KiB raw image with a recognizable pattern at LBA 3.
    let mut data = vec![0u8; 128 * 1024];
    for (index, byte) in data[3 * 512..4 * 512].iter_mut().enumerate() {
        *byte = (index as u8) ^ 0x5A;
    }
    machine.insert_hdd(0, HddImage::from_raw_flat(data).unwrap(), None);

    let buffer = 0x0000_2000u32;
    program_dma_channel(&mut machine.bus, SCSI_DMA_CHANNEL, buffer, 512);
    run_command(&mut machine, &[opcode::READ10, 0, 0, 0, 0, 3, 0, 0, 1, 0]);

    for index in 0..512u32 {
        assert_eq!(machine.bus.read_byte(buffer + index), (index as u8) ^ 0x5A);
    }
}

#[test]
fn write10_then_read10_round_trips_through_memory() {
    let mut machine = machine_mx();
    machine.insert_hdd(
        0,
        HddImage::from_raw_flat(vec![0u8; 128 * 1024]).unwrap(),
        None,
    );

    // Fill a source buffer and DMA it to LBA 7 via WRITE(10).
    let source = 0x0000_4000u32;
    for index in 0..512u32 {
        machine
            .bus
            .write_byte(source + index, (index as u8).wrapping_add(1));
    }
    program_dma_channel(&mut machine.bus, SCSI_DMA_CHANNEL, source, 512);
    run_command(&mut machine, &[opcode::WRITE10, 0, 0, 0, 0, 7, 0, 0, 1, 0]);
    drain_status(&mut machine);

    // Read LBA 7 back into a different buffer and compare.
    let destination = 0x0000_6000u32;
    program_dma_channel(&mut machine.bus, SCSI_DMA_CHANNEL, destination, 512);
    run_command(&mut machine, &[opcode::READ10, 0, 0, 0, 0, 7, 0, 0, 1, 0]);

    for index in 0..512u32 {
        assert_eq!(
            machine.bus.read_byte(destination + index),
            (index as u8).wrapping_add(1)
        );
    }
}
