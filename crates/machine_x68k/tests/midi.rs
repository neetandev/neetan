//! Integration tests for the CZ-6BM1 MIDI board path.
//!
//! X68000 software emits MIDI through the YM3802 on the CZ-6BM1 expansion
//! card at odd addresses 0xEAFA01-0xEAFA0F. These tests drive the real bus
//! dispatch, transmit pacing, and interrupt wiring through the public
//! machine API with a synthetic ROM set and check that transmitted bytes
//! are captured verbatim at the 31.25 kbit/s byte cadence.

#[path = "common/harness.rs"]
mod harness;

use common::{
    Bus, M68000AccessSize, M68000BusAccess, M68000BusError, M68000CycleKind, M68000FunctionCode,
};
use harness::{machine, read_byte, write_byte};
use machine_x68k::{X68kMachine, X68kModel};

/// Interrupt-vector register address of the primary card.
const IVR_ADDRESS: u32 = 0xEAFA01;
/// System-control register address of the primary card.
const RGR_ADDRESS: u32 = 0xEAFA03;
/// Interrupt-clear register address of the primary card.
const ICR_ADDRESS: u32 = 0xEAFA07;
/// Banked offset-4 register address of the primary card.
const BANKED_4_ADDRESS: u32 = 0xEAFA09;
/// Banked offset-5 register address of the primary card.
const BANKED_5_ADDRESS: u32 = 0xEAFA0B;
/// Banked offset-6 register address of the primary card.
const BANKED_6_ADDRESS: u32 = 0xEAFA0D;
/// CPU cycles per MIDI byte at the CLKM/32 rate on a 10 MHz model.
const MIDI_BYTE_CYCLES: u64 = 3200;

/// Builds a supervisor byte access for probing bus errors.
fn probe(address: u32) -> M68000BusAccess {
    M68000BusAccess {
        address,
        size: M68000AccessSize::Byte,
        function_code: M68000FunctionCode::SupervisorData,
        cycle_kind: M68000CycleKind::Normal,
    }
}

/// Programs the CLKM/32 MIDI rate and enables the transmitter.
fn enable_transmitter_at_midi_rate(machine: &mut X68kMachine) {
    write_byte(machine, RGR_ADDRESS, 0x04);
    write_byte(machine, BANKED_4_ADDRESS, 0x08);
    write_byte(machine, RGR_ADDRESS, 0x05);
    write_byte(machine, BANKED_5_ADDRESS, 0x01);
}

#[test]
fn midi_bytes_captured_verbatim_and_in_order() {
    let mut machine = machine(X68kModel::X68000);
    machine.install_midi_card();
    enable_transmitter_at_midi_rate(&mut machine);

    // Note on, note off, program change.
    let stream = [0x90, 0x40, 0x7F, 0x80, 0x40, 0x00, 0xC0, 0x30];
    for &byte in &stream {
        write_byte(&mut machine, BANKED_6_ADDRESS, byte);
    }
    machine.run_for(MIDI_BYTE_CYCLES * (stream.len() as u64 + 1));

    let mut drained = Vec::new();
    machine.flush_midi_into(&mut drained);
    assert_eq!(drained, stream);

    // Draining again yields nothing.
    let mut again = Vec::new();
    machine.flush_midi_into(&mut again);
    assert!(again.is_empty());
}

#[test]
fn transmission_is_paced_at_the_midi_byte_rate() {
    let mut machine = machine(X68kModel::X68000);
    machine.install_midi_card();
    enable_transmitter_at_midi_rate(&mut machine);
    for byte in [0xF8, 0xFE, 0xFA] {
        write_byte(&mut machine, BANKED_6_ADDRESS, byte);
    }

    let mut drained = Vec::new();
    machine.run_for(MIDI_BYTE_CYCLES * 2 - 100);
    machine.flush_midi_into(&mut drained);
    assert_eq!(
        drained.len(),
        1,
        "only the first byte fits the elapsed time"
    );
    machine.run_for(MIDI_BYTE_CYCLES * 2);
    machine.flush_midi_into(&mut drained);
    assert_eq!(drained, [0xF8, 0xFE, 0xFA]);
}

#[test]
fn absent_card_probe_raises_a_bus_error() {
    let mut machine = machine(X68kModel::X68000);
    assert_eq!(
        machine.bus.m68000_read(probe(IVR_ADDRESS)),
        Err(M68000BusError),
        "the Human68k MIDI probe expects a bus error without a card"
    );
    machine.install_midi_card();
    assert_eq!(read_byte(&mut machine, IVR_ADDRESS), 0x10);
}

#[test]
fn transmit_empty_interrupt_reaches_the_cpu_interface() {
    let mut machine = machine(X68kModel::X68000);
    machine.install_midi_card();
    write_byte(&mut machine, RGR_ADDRESS, 0x00);
    write_byte(&mut machine, BANKED_4_ADDRESS, 0x40);
    write_byte(&mut machine, BANKED_6_ADDRESS, 0x40);
    enable_transmitter_at_midi_rate(&mut machine);
    write_byte(&mut machine, BANKED_6_ADDRESS, 0x90);

    assert_eq!(machine.bus.m68000_interrupt_level(), 4);
    assert_eq!(machine.bus.m68000_acknowledge_interrupt(4), 0x4C);
    write_byte(&mut machine, ICR_ADDRESS, 0x40);
    assert_eq!(machine.bus.m68000_interrupt_level(), 0);
}

#[test]
fn no_bytes_are_captured_without_an_installed_card() {
    let mut machine = machine(X68kModel::X68000);
    let mut drained = Vec::new();
    machine.flush_midi_into(&mut drained);
    assert!(drained.is_empty());
}
