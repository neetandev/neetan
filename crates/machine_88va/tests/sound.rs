//! Sound board integration tests: OPNA FM synthesis and the Timer A interrupt
//! gated by the FM interrupt mask, driven through the public bus surface.

use common::{Bus, Machine};
use machine_88va::Pc88VaMachine;

#[path = "common/harness.rs"]
mod harness;
use harness::*;

/// Writes one OPNA low-bank register through ports 0x044/0x045.
fn opn_write(machine: &mut Pc88VaMachine, register: u8, value: u8) {
    machine.bus.io_write_byte(0x044, register);
    machine.bus.io_write_byte(0x045, value);
}

#[test]
fn fm_register_sequence_produces_non_silent_audio() {
    let mut machine = machine();
    // A minimal FM key-on on channel 0: set an operator total level and key it.
    opn_write(&mut machine, 0x30, 0x00); // DT/MULT
    opn_write(&mut machine, 0x40, 0x00); // TL = 0 (loudest)
    opn_write(&mut machine, 0x5C, 0x1F); // RR
    opn_write(&mut machine, 0xA0, 0x00); // F-number low
    opn_write(&mut machine, 0xA4, 0x22); // block / F-number high
    opn_write(&mut machine, 0x28, 0xF0); // key on, channel 0, all operators

    // Advance so the chip accrues samples, then render a buffer.
    machine.run_for(200_000);
    let mut output = vec![0.0f32; 2048];
    let written = machine.generate_audio_samples(1.0, &mut output);
    assert_eq!(written, output.len());
    assert!(
        output.iter().any(|sample| sample.abs() > 0.0),
        "the FM voice should produce non-silent output"
    );
}

#[test]
fn opna_timer_a_raises_irq12_when_unmasked() {
    let mut machine = machine();

    // Unmask the cascade (master IR7) and the sound line (slave IR4).
    machine.bus.io_write_byte(0x18A, 0x7F);
    machine.bus.io_write_byte(0x186, 0xEF);
    // FM interrupt mask open: system port 0x032 bit 7 clear (the reset default).

    // Timer A: shortest period (NA = 0x3FF), load + enable timer A IRQ.
    opn_write(&mut machine, 0x24, 0xFF);
    opn_write(&mut machine, 0x25, 0x03);
    opn_write(&mut machine, 0x27, 0x05);

    let mut cycle = machine.bus.current_cycle();
    let limit = cycle + 5_000_000;
    let mut fired = false;
    while cycle < limit {
        cycle += 64;
        machine.bus.set_current_cycle(cycle);
        if machine.bus.has_irq() {
            fired = true;
            break;
        }
    }
    assert!(fired, "timer A should raise the OPNA interrupt");
    // Slave IR4 acknowledges as vector 0x14.
    assert_eq!(machine.bus.acknowledge_irq(), 0x14);
}

#[test]
fn fm_interrupt_mask_gates_irq12() {
    let mut machine = machine();
    machine.bus.io_write_byte(0x18A, 0x7F);
    machine.bus.io_write_byte(0x186, 0xEF);
    // Mask the FM interrupt (port 0x032 bit 7 set).
    machine.bus.io_write_byte(0x032, 0x80);

    opn_write(&mut machine, 0x24, 0xFF);
    opn_write(&mut machine, 0x25, 0x03);
    opn_write(&mut machine, 0x27, 0x05);

    let mut cycle = machine.bus.current_cycle();
    for _ in 0..20_000 {
        cycle += 64;
        machine.bus.set_current_cycle(cycle);
    }
    assert!(
        !machine.bus.has_irq(),
        "a masked FM interrupt must not surface"
    );
    // Opening the mask now surfaces the pending timer interrupt.
    machine.bus.io_write_byte(0x032, 0x00);
    assert!(machine.bus.has_irq());
    assert_eq!(machine.bus.acknowledge_irq(), 0x14);
}
