//! Interrupt-delivery tests: the IM 2 timer path on the mkII, the timer mask,
//! the reprogrammable timer vector, and the SR programmable vector table with
//! its vertical-retrace source.

use common::CpuZ80;
use machine_60::{Pc6000Bus, Pc6000Machine, Pc6000Model};

mod harness;
use harness::{build_machine, build_machine_with_synthetic_roms, fire_next_event, run_frames};

/// Vector-table page held in the I register; the timer vector 0x06 lands the ISR
/// pointer at 0xE006 in work RAM.
const VECTOR_PAGE: u8 = 0xE0;
const COUNTER_ADDRESS: u16 = 0xE100;
const ISR_ADDRESS: u16 = 0x0050;

/// Builds a mkII BASIC image that installs an IM 2 timer ISR which increments a
/// counter. When `mask` is set it also raises the port 0xF3 timer mask, which
/// must suppress the interrupt.
fn build_timer_irq_basic(mask: bool, target: u8) -> Vec<u8> {
    let mut rom = vec![0u8; 0x8000];

    let init: Vec<u8> = vec![
        0xF3, // DI
        0x31,
        0x00,
        0xF0, // LD SP, 0xF000
        0x3E,
        VECTOR_PAGE, // LD A, VECTOR_PAGE
        0xED,
        0x47, // LD I, A
        0xED,
        0x5E, // IM 2
        0x21,
        (ISR_ADDRESS & 0xFF) as u8,
        (ISR_ADDRESS >> 8) as u8, // LD HL, ISR_ADDRESS
        0x22,
        0x06,
        0xE0, // LD (0xE006), HL  (timer vector entry)
        0xAF, // XOR A
        0x32,
        (COUNTER_ADDRESS & 0xFF) as u8,
        (COUNTER_ADDRESS >> 8) as u8, // LD (counter), A
        0x3E,
        if mask { 0x04 } else { 0x00 }, // LD A, mask  (port 0xF3 bit 2)
        0xD3,
        0xF3, // OUT (0xF3), A
        0x3E,
        0x00, // LD A, 0x00
        0xD3,
        0xB0, // OUT (0xB0), A   (system latch: enable timer)
        // loop: EI; HALT; LD A,(counter); CP target; JR C, loop; done: JR done
        0xFB, // EI
        0x76, // HALT
        0x3A,
        (COUNTER_ADDRESS & 0xFF) as u8,
        (COUNTER_ADDRESS >> 8) as u8, // LD A,(counter)
        0xFE,
        target, // CP target
        0x38,
        0xF7, // JR C, loop (-9)
        0x18,
        0xFE, // JR done (-2)
    ];
    rom[..init.len()].copy_from_slice(&init);

    let isr: &[u8] = &[
        0xF5, // PUSH AF
        0x3A,
        (COUNTER_ADDRESS & 0xFF) as u8,
        (COUNTER_ADDRESS >> 8) as u8, // LD A,(counter)
        0x3C,                         // INC A
        0x32,
        (COUNTER_ADDRESS & 0xFF) as u8,
        (COUNTER_ADDRESS >> 8) as u8, // LD (counter),A
        0xF1,                         // POP AF
        0xFB,                         // EI
        0xED,
        0x4D, // RETI
    ];
    rom[ISR_ADDRESS as usize..ISR_ADDRESS as usize + isr.len()].copy_from_slice(isr);

    rom
}

fn run_until_counter(machine: &mut Pc6000Machine, target: u8) -> u8 {
    const STEP: u64 = 50_000;
    const CAP: u64 = 8_000_000;
    let mut total = 0u64;
    while total < CAP {
        machine.run_for(STEP);
        total += STEP;
        if machine.bus.peek_byte(COUNTER_ADDRESS) >= target {
            break;
        }
    }
    machine.bus.peek_byte(COUNTER_ADDRESS)
}

/// Pumps scheduled events until an interrupt is acknowledged, returning its vector.
fn next_irq_vector(bus: &mut Pc6000Bus) -> u8 {
    for _ in 0..100_000 {
        if let Some(vector) = fire_next_event(bus) {
            return vector;
        }
    }
    panic!("no interrupt was delivered");
}

#[test]
fn mkii_timer_irq_wakes_the_halted_cpu() {
    let target = 3;
    let rom = build_timer_irq_basic(false, target);
    let mut machine =
        build_machine_with_synthetic_roms(Pc6000Model::Pc6001Mk2, |roms| roms.basic = Some(rom));

    assert_eq!(machine.main_cpu.pc(), 0x0000);
    let count = run_until_counter(&mut machine, target);
    assert!(
        count >= target,
        "timer ISR ran {count} times, expected at least {target}"
    );
}

#[test]
fn mkii_masked_timer_never_interrupts() {
    let target = 1;
    let rom = build_timer_irq_basic(true, target);
    let mut machine =
        build_machine_with_synthetic_roms(Pc6000Model::Pc6001Mk2, |roms| roms.basic = Some(rom));

    let count = run_until_counter(&mut machine, target);
    assert_eq!(
        count, 0,
        "the port 0xF3 mask did not suppress the timer IRQ"
    );
}

#[test]
fn mkii_timer_vector_is_reprogrammable_through_io() {
    let mut machine = build_machine(Pc6000Model::Pc6001Mk2);
    let bus = &mut machine.bus;

    // Enable the timer through the system latch (bit 0 clear).
    bus.io_write(0xB0, 0x00);
    assert_eq!(next_irq_vector(bus), 0x06, "default timer vector");

    bus.io_write(0xF7, 0x22);
    assert_eq!(next_irq_vector(bus), 0x22, "reprogrammed timer vector");
}

#[test]
fn non_sr_vertical_retrace_raises_no_interrupt() {
    let mut machine = build_machine(Pc6000Model::Pc6001Mk2);
    // The timer stays disabled and no keys are pending, so a few frames worth of
    // vertical-retrace events must leave the controller idle on the mkII.
    run_frames(&mut machine, 4);
    assert!(!machine.bus.has_irq(), "the mkII frame event raised an IRQ");
}

#[test]
fn sr_vector_table_round_trips_per_source() {
    let mut machine = build_machine(Pc6000Model::Pc6001Mk2Sr);
    let bus = &mut machine.bus;

    for index in 0..8u16 {
        let vector = 0x80 + index as u8;
        bus.io_write(0xB8 + index, vector);
    }
    for index in 0..8u16 {
        assert_eq!(bus.io_read(0xB8 + index), 0x80 + index as u8);
    }
}

#[test]
fn sr_hardware_revision_distinguishes_models() {
    let mut mk2sr = build_machine(Pc6000Model::Pc6001Mk2Sr);
    let mut pc6601sr = build_machine(Pc6000Model::Pc6601Sr);

    assert_eq!(mk2sr.bus.io_read(0xB2), 0x01);
    assert_eq!(pc6601sr.bus.io_read(0xB2), 0x03);
}

#[test]
fn sr_joystick_trigger_uses_fixed_vector() {
    let mut machine = build_machine(Pc6000Model::Pc6001Mk2Sr);
    let bus = &mut machine.bus;

    bus.io_write(0xB9, 0x80);
    bus.io_write(0x90, 0x06);

    assert_eq!(bus.acknowledge_irq(), 0x16);
}

#[test]
fn sr_vertical_retrace_raises_its_programmed_vector() {
    let mut machine = build_machine(Pc6000Model::Pc6001Mk2Sr);
    let bus = &mut machine.bus;

    // Program the vertical-retrace slot (source index 4 -> port 0xBC).
    bus.io_write(0xBC, 0x86);
    assert_eq!(next_irq_vector(bus), 0x86);
}
