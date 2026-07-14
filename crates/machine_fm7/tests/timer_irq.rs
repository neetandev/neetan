//! Main timer IRQ tests.

mod harness;

use common::Cpu6809;
use harness::{build_bus_with_synthetic_roms, build_machine_with_synthetic_roms, run_bus_cycles};
use machine_fm7::BootMode;

#[test]
fn timer_event_sets_pending_irq_when_unmasked() {
    let mut bus = build_bus_with_synthetic_roms(BootMode::Basic, |_| {});
    let period = bus.timer_irq_period_cycles();

    run_bus_cycles(&mut bus, period);
    assert!(!bus.has_irq(), "timer is pending but masked");

    bus.write_byte(0xFD02, 0x04);
    assert!(bus.has_irq(), "unmasking exposes the pending timer IRQ");

    let status = bus.read_byte(0xFD03);
    assert_eq!(status & 0x04, 0x00);
    assert!(!bus.has_irq(), "reading FD03 acknowledges the timer");
}

#[test]
fn cpu_takes_irq_vector_after_timer_fires() {
    let mut machine = build_machine_with_synthetic_roms(BootMode::Basic, |roms| {
        let boot = roms.boot_bas.as_mut().expect("basic boot ROM exists");
        // ANDCC #~I ; BRA $
        boot[..4].copy_from_slice(&[0x1C, 0xEF, 0x20, 0xFE]);

        let handler_offset = 0x0100;
        // LDA #0x77 ; STA 0x0120 ; LDA 0xFD03 ; RTI
        roms.fbasic[handler_offset..handler_offset + 9]
            .copy_from_slice(&[0x86, 0x77, 0xB7, 0x01, 0x20, 0xB6, 0xFD, 0x03, 0x3B]);
    });
    machine.main_cpu.set_s(0x0200);
    machine.bus.poke_byte(0xFFF8, 0x81);
    machine.bus.poke_byte(0xFFF9, 0x00);
    machine.bus.write_byte(0xFD02, 0x04);

    let budget = machine.bus.timer_irq_period_cycles() + 2_000;
    machine.run_for(budget);

    assert_eq!(machine.bus.peek_byte(0x0120), 0x77);
}
