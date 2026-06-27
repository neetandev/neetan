//! Interrupt and timer integration tests: the 8259 PIC, PIT channel 0 driving
//! IRQ0, and the uPD9002 clock divider that scales the timer, all driven through
//! the public bus surface.

use common::{Bus, Cpu};
use machine88va::Pc88VaMachine;

#[path = "common/harness.rs"]
mod harness;
use harness::*;

/// Programs PIT channel 0 as a rate generator with the given 16-bit reload.
fn program_timer0(machine: &mut Pc88VaMachine, reload: u16) {
    // Control word 0x34: SC=0, RL=both, mode 2 (rate generator), binary.
    machine.bus.io_write_byte(0x1A6, 0x34);
    machine.bus.io_write_byte(0x1A0, (reload & 0xFF) as u8);
    machine.bus.io_write_byte(0x1A0, (reload >> 8) as u8);
}

#[test]
fn upd9002_timer_clock_port_round_trips() {
    let mut machine = machine();
    machine.bus.io_write_byte(0xFFF0, 0x6A);
    assert_eq!(machine.bus.io_read_byte(0xFFF0), 0x6A);
}

#[test]
fn pic_imr_round_trips_through_va_ports() {
    let mut machine = machine();
    // Master IMR at 0x18A, slave IMR at 0x186.
    machine.bus.io_write_byte(0x18A, 0x5A);
    machine.bus.io_write_byte(0x186, 0xA5);
    assert_eq!(machine.bus.io_read_byte(0x18A), 0x5A);
    assert_eq!(machine.bus.io_read_byte(0x186), 0xA5);
}

#[test]
fn timer0_raises_irq0_at_expected_cycle() {
    let mut machine = machine();
    // Unmask IRQ0 on the master PIC (default IMR masks it).
    machine.bus.io_write_byte(0x18A, 0xFE);

    let reload = 0x1000u16;
    program_timer0(&mut machine, reload);
    // tcks = 0 -> CPU/PIT ratio = 4.
    let period = u64::from(reload) * 4;

    machine.bus.set_current_cycle(period - 1);
    assert!(!machine.bus.has_irq());

    machine.bus.set_current_cycle(period);
    assert!(machine.bus.has_irq());
    assert_eq!(machine.bus.acknowledge_irq(), 0x08);
}

#[test]
fn masked_irq0_is_suppressed_until_unmasked() {
    let mut machine = machine();
    let reload = 0x0800u16;
    program_timer0(&mut machine, reload);
    let period = u64::from(reload) * 4;

    // IRQ0 still masked by the default master IMR.
    machine.bus.set_current_cycle(period);
    assert!(!machine.bus.has_irq());

    // Unmask IRQ0; the latched request now surfaces.
    machine.bus.io_write_byte(0x18A, 0xFE);
    assert!(machine.bus.has_irq());
}

#[test]
fn upd9002_divider_scales_timer0_period() {
    let mut machine = machine();
    machine.bus.io_write_byte(0x18A, 0xFE);
    // Divider bits = 2 -> CPU/PIT ratio = 4 << 2 = 16.
    machine.bus.io_write_byte(0xFFF0, 0x02);

    let reload = 0x0400u16;
    program_timer0(&mut machine, reload);
    let period = u64::from(reload) * 16;

    machine.bus.set_current_cycle(period - 1);
    assert!(!machine.bus.has_irq());
    machine.bus.set_current_cycle(period);
    assert!(machine.bus.has_irq());
}

#[test]
fn halted_v30_wakes_on_timer_irq() {
    let mut machine = machine();

    // INT 8 vector -> 0x0000:0x0100.
    machine.bus.write_byte(0x20, 0x00);
    machine.bus.write_byte(0x21, 0x01);
    machine.bus.write_byte(0x22, 0x00);
    machine.bus.write_byte(0x23, 0x00);

    // Handler at 0x0100: MOV AL, 0x99 ; HLT.
    machine.bus.write_byte(0x0100, 0xB0);
    machine.bus.write_byte(0x0101, 0x99);
    machine.bus.write_byte(0x0102, 0xF4);

    // Main program at 0x0400: STI ; HLT.
    machine.bus.write_byte(0x0400, 0xFB);
    machine.bus.write_byte(0x0401, 0xF4);

    machine.bus.io_write_byte(0x18A, 0xFE);
    program_timer0(&mut machine, 0x0040);

    machine.cpu.set_cs(0x0000);
    machine.cpu.set_ip(0x0400);

    machine.run_for(100_000);

    // The interrupt handler ran, so the halted CPU was woken by the timer IRQ.
    assert_eq!(machine.cpu.ax() & 0xFF, 0x99);
}
