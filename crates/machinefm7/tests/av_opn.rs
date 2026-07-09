//! FM-77AV YM2203 (OPN) command-latch and timer-IRQ tests.

mod harness;

use harness::{build_av_bus_with_synthetic_roms, run_bus_cycles};
use machinefm7::{BootMode, Fm7Bus};

/// `0xFD0D` PSG-aliased command port.
const PSG_COMMAND_PORT: u16 = 0xFD0D;
/// `0xFD0E` PSG-aliased data port.
const PSG_DATA_PORT: u16 = 0xFD0E;
/// `0xFD15` native OPN command port.
const OPN_COMMAND_PORT: u16 = 0xFD15;
/// `0xFD16` native OPN data port.
const OPN_DATA_PORT: u16 = 0xFD16;
/// `0xFD17` OPN external status port.
const OPN_EXT_PORT: u16 = 0xFD17;

/// Command latching the register address from the data byte.
const OPN_LATCH_ADDRESS: u8 = 3;
/// Command writing the data byte to the latched register.
const OPN_WRITE_DATA: u8 = 2;
/// Command selecting a chip status read on the next data read.
const OPN_READ_STATUS: u8 = 4;

/// `0xFD17` bit 3 (active low) reporting a pending OPN IRQ.
const FD17_OPN_IRQ_BIT: u8 = 0x08;

/// Writes an OPN register through the given command/data port pair.
fn write_opn_register(bus: &mut Fm7Bus, command_port: u16, data_port: u16, address: u8, value: u8) {
    bus.write_byte(command_port, OPN_LATCH_ADDRESS);
    bus.write_byte(data_port, address);
    bus.write_byte(command_port, OPN_WRITE_DATA);
    bus.write_byte(data_port, value);
}

/// Programs OPN timer A with a short period and enables its IRQ.
fn program_timer_a(bus: &mut Fm7Bus, command_port: u16, data_port: u16) {
    // Near-maximum count (NNN = 0x3FF) so the timer expires in a few dozen cycles.
    write_opn_register(bus, command_port, data_port, 0x24, 0xFF);
    write_opn_register(bus, command_port, data_port, 0x25, 0x03);
    // Load timer A (bit 0) and enable its IRQ (bit 2).
    write_opn_register(bus, command_port, data_port, 0x27, 0x05);
}

#[test]
fn timer_a_raises_the_opn_irq_via_the_native_ports() {
    let mut bus = build_av_bus_with_synthetic_roms(BootMode::Basic, |_| {});
    assert_eq!(bus.read_byte(OPN_EXT_PORT), 0xFF);
    assert!(!bus.has_irq());

    program_timer_a(&mut bus, OPN_COMMAND_PORT, OPN_DATA_PORT);
    run_bus_cycles(&mut bus, 4_000);

    assert!(bus.has_irq());
    assert_eq!(bus.read_byte(OPN_EXT_PORT) & FD17_OPN_IRQ_BIT, 0);
}

#[test]
fn timer_a_raises_the_opn_irq_via_the_psg_alias_ports() {
    let mut bus = build_av_bus_with_synthetic_roms(BootMode::Basic, |_| {});

    program_timer_a(&mut bus, PSG_COMMAND_PORT, PSG_DATA_PORT);
    run_bus_cycles(&mut bus, 4_000);

    assert!(bus.has_irq());
    assert_eq!(bus.read_byte(OPN_EXT_PORT) & FD17_OPN_IRQ_BIT, 0);
}

#[test]
fn opn_status_reports_the_timer_a_overflow() {
    let mut bus = build_av_bus_with_synthetic_roms(BootMode::Basic, |_| {});

    program_timer_a(&mut bus, OPN_COMMAND_PORT, OPN_DATA_PORT);
    run_bus_cycles(&mut bus, 4_000);

    // Command 4 selects the status register on the next data-port read.
    bus.write_byte(OPN_COMMAND_PORT, OPN_READ_STATUS);
    let status = bus.read_byte(OPN_DATA_PORT);
    assert_ne!(status & 0x01, 0, "timer A overflow flag should be set");
}
