//! Main/sub handshake tests: HALT gate, BUSY flag with the CLR quirk, ATTENTION
//! FIRQ, and the CANCEL interrupt.

mod harness;

use common::Bus;
use harness::{build_bus_with_synthetic_roms, build_machine_with_synthetic_roms, run_bus_cycles};
use machine_fm7::{BootMode, SubBusView};

/// `0xFD05` write bit requesting the sub CPU halt.
const FD05_HALT: u8 = 0x80;
/// `0xFD05` write bit raising the sub CANCEL interrupt.
const FD05_CANCEL: u8 = 0x40;
/// Bit 7 reporting the sub busy / halted state on `0xFD04`/`0xFD05`.
const SUB_BUSY_BIT: u8 = 0x80;
/// `0xFD04` active-low sub-attention bit.
const FD04_ATTENTION: u8 = 0x01;

#[test]
fn halt_gate_controls_shared_ram_access() {
    let mut machine = build_machine_with_synthetic_roms(BootMode::Basic, |roms| {
        harness::park_main_cpu(roms);
        harness::park_sub_cpu(roms);
    });

    // While the sub runs, the shared window is closed: reads float, writes drop.
    machine.run_for(400);
    assert!(!machine.bus.is_sub_halted());
    machine.bus.write_byte(0xFC80, 0x11);
    assert_eq!(machine.bus.read_byte(0xFC80).0, 0xFF);

    // Requesting HALT stops the sub at an instruction boundary and folds into busy.
    machine.bus.write_byte(0xFD05, FD05_HALT);
    machine.run_for(400);
    assert!(machine.bus.is_sub_halted());
    assert!(machine.bus.sub_busy());
    assert_eq!(machine.bus.read_byte(0xFD05).0 & SUB_BUSY_BIT, SUB_BUSY_BIT);

    // Now the window aliases the sub shared RAM.
    machine.bus.write_byte(0xFC80, 0x42);
    assert_eq!(machine.bus.read_byte(0xFC80).0, 0x42);
    assert_eq!(machine.bus.sub_peek_byte(0xD380), 0x42);

    // Releasing HALT lets the sub run again and closes the window.
    machine.bus.write_byte(0xFD05, 0x00);
    machine.run_for(400);
    assert!(!machine.bus.is_sub_halted());
    assert_eq!(machine.bus.read_byte(0xFC80).0, 0xFF);
}

#[test]
fn busy_flag_sets_on_write_and_clears_on_read() {
    let mut bus = build_bus_with_synthetic_roms(BootMode::Basic, |_| {});

    sub_write(&mut bus, 0xD40A, 0x00);
    assert!(bus.sub_busy());

    sub_read(&mut bus, 0xD40A);
    assert!(!bus.sub_busy());
}

#[test]
fn clr_read_modify_write_delays_the_busy_reassert() {
    let mut bus = build_bus_with_synthetic_roms(BootMode::Basic, |_| {});

    sub_write(&mut bus, 0xD40A, 0x00);
    assert!(bus.sub_busy());

    // A CLR is a read then an immediate write. The read clears busy; the paired
    // write keeps it cleared and re-asserts it only after a short delay.
    sub_read(&mut bus, 0xD40A);
    assert!(!bus.sub_busy());
    sub_write(&mut bus, 0xD40A, 0x00);
    assert!(
        !bus.sub_busy(),
        "busy stays cleared immediately after a CLR write"
    );

    run_bus_cycles(&mut bus, 64);
    assert!(
        bus.sub_busy(),
        "busy is re-asserted after the CLR delay elapses"
    );
}

#[test]
fn lone_read_disarms_so_a_later_write_sets_busy_immediately() {
    let mut bus = build_bus_with_synthetic_roms(BootMode::Basic, |_| {});

    sub_read(&mut bus, 0xD40A);
    assert!(!bus.sub_busy());

    // With no paired write, the CLR window disarms after its delay.
    run_bus_cycles(&mut bus, 64);
    sub_write(&mut bus, 0xD40A, 0x00);
    assert!(bus.sub_busy(), "an unpaired write sets busy immediately");
}

#[test]
fn attention_raises_and_read_clears_main_firq() {
    let mut bus = build_bus_with_synthetic_roms(BootMode::Basic, |_| {});
    assert!(!bus.firq_active());

    sub_read(&mut bus, 0xD404);
    assert!(bus.firq_active());
    assert_eq!(bus.read_byte(0xFD04).0 & FD04_ATTENTION, 0x00);

    // Reading 0xFD04 acknowledges the attention FIRQ.
    assert!(!bus.firq_active());
    assert_eq!(bus.read_byte(0xFD04).0 & FD04_ATTENTION, FD04_ATTENTION);
}

#[test]
fn cancel_raises_sub_irq_until_acknowledged() {
    let mut bus = build_bus_with_synthetic_roms(BootMode::Basic, |_| {});
    assert!(!sub_irq(&mut bus));

    bus.write_byte(0xFD05, FD05_CANCEL);
    assert!(sub_irq(&mut bus));

    // The sub acknowledges by reading its cancel port.
    sub_read(&mut bus, 0xD402);
    assert!(!sub_irq(&mut bus));
}

/// Reads a byte from the sub address space through a sub bus view.
fn sub_read(bus: &mut machine_fm7::Fm7Bus, address: u32) -> u8 {
    let mut view = SubBusView { bus };
    view.read_byte(address)
}

/// Writes a byte to the sub address space through a sub bus view.
fn sub_write(bus: &mut machine_fm7::Fm7Bus, address: u32, value: u8) {
    let mut view = SubBusView { bus };
    view.write_byte(address, value);
}

/// Whether the sub CPU IRQ line is asserted, sampled through a sub bus view.
fn sub_irq(bus: &mut machine_fm7::Fm7Bus) -> bool {
    let view = SubBusView { bus };
    view.has_irq()
}
