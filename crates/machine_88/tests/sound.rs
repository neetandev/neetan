//! Sound tests: the OPNA (Sound Board II) timer interrupt path, the SINTM mask
//! gate, the beeper tone, and OPNA audio generation.
//!
//! The interrupt tests hand-assemble a ROM that installs an IM 2 vector at the
//! INT4 slot (`level 4 * 2`), programs OPNA timer A to overflow, and counts the
//! interrupt service routine runs, proving the OPNA -> i8214 INT4 -> Z80 IM 2
//! delivery and the port 0x32 SINTM mask.

use machine_88::Pc8801Machine;

mod harness;
use harness::{build_machine_with, build_machine_with_rom};

/// Page holding the IM 2 vector table and the counter (main RAM).
const VECTOR_PAGE: u8 = 0x90;
/// Address of the ISR-incremented counter (main RAM).
const COUNTER_ADDRESS: u16 = 0x9100;
const ISR_ADDRESS: u16 = 0x0050;

/// Builds a ROM that programs OPNA timer A for an INT4 interrupt. When `mask` is
/// true the ROM sets SINTM (port 0x32 bit 7), which must suppress INT4.
fn build_opna_irq_rom(mask: bool, target: u8) -> Vec<u8> {
    let mut rom = vec![0u8; 0x8000];

    let mut init: Vec<u8> = vec![
        0xF3, // DI
        0x31,
        0x00,
        0xB0, // LD SP, 0xB000
        0x3E,
        VECTOR_PAGE, // LD A, VECTOR_PAGE
        0xED,
        0x47, // LD I, A
        0xED,
        0x5E, // IM 2
        0x21,
        0x50,
        0x00, // LD HL, ISR_ADDRESS (0x0050)
        0x22,
        0x08,
        0x90, // LD (0x9008), HL  (INT4 vector)
        0xAF, // XOR A
        0x32,
        0x00,
        0x91, // LD (0x9100), A   (counter = 0)
    ];

    // LD A, value ; OUT (0x32), A - bit 7 is SINTM, the OPNA IRQ mask.
    init.extend_from_slice(&[0x3E, if mask { 0x80 } else { 0x00 }, 0xD3, 0x32]);

    // Program OPNA timer A (address via 0x44, data via 0x45) and load+enable it.
    let opna_writes: [(u8, u8); 3] = [
        (0x24, 0x80), // timer A period high 8 bits
        (0x25, 0x00), // timer A period low 2 bits
        (0x27, 0x05), // load timer A (bit 0) + enable timer A IRQ (bit 2)
    ];
    for (reg, val) in opna_writes {
        init.extend_from_slice(&[0x3E, reg, 0xD3, 0x44, 0x3E, val, 0xD3, 0x45]);
    }

    // OUT (0xE4), 0x08 -- enable all i8214 priority levels.
    init.extend_from_slice(&[0x3E, 0x08, 0xD3, 0xE4]);

    // loop: EI; HALT; LD A,(counter); CP target; JR C, loop ; done: JR done
    init.extend_from_slice(&[
        0xFB, // EI
        0x76, // HALT
        0x3A, 0x00, 0x91, // LD A, (0x9100)
        0xFE, target, // CP target
        0x38, 0xF7, // JR C, -9 (back to EI)
        0x18, 0xFE, // JR done (-2)
    ]);
    rom[..init.len()].copy_from_slice(&init);

    let isr: &[u8] = &[
        0xF5, // PUSH AF
        // Reset timer A overflow flag, then reload + re-enable, so the timer
        // keeps firing (address latch stays at 0x27 between the data writes).
        0x3E, 0x27, 0xD3, 0x44, // LD A,0x27; OUT (0x44),A
        0x3E, 0x10, 0xD3, 0x45, // LD A,0x10; OUT (0x45),A  (reset timer A flag)
        0x3E, 0x05, 0xD3, 0x45, // LD A,0x05; OUT (0x45),A  (load + enable A)
        0x3A, 0x00, 0x91, // LD A, (0x9100)
        0x3C, // INC A
        0x32, 0x00, 0x91, // LD (0x9100), A
        0x3E, 0x08, 0xD3, 0xE4, // LD A,0x08; OUT (0xE4),A  (re-arm priority)
        0xF1, // POP AF
        0xFB, // EI
        0xED, 0x4D, // RETI
    ];
    rom[ISR_ADDRESS as usize..ISR_ADDRESS as usize + isr.len()].copy_from_slice(isr);

    rom
}

fn run_until_counter(machine: &mut Pc8801Machine, target: u8) -> u8 {
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

#[test]
fn opna_timer_raises_int4() {
    let target = 3;
    let rom = build_opna_irq_rom(false, target);
    let mut machine = build_machine_with_rom(&rom);

    let count = run_until_counter(&mut machine, target);
    assert!(
        count >= target,
        "OPNA timer INT4 ISR ran {count} times, expected at least {target}"
    );
}

#[test]
fn sintm_masks_opna_timer_int4() {
    let target = 1;
    let rom = build_opna_irq_rom(true, target);
    let mut machine = build_machine_with_rom(&rom);

    let count = run_until_counter(&mut machine, target);
    assert_eq!(count, 0, "SINTM did not mask the OPNA timer INT4");
}

#[test]
fn beeper_enabled_produces_tone() {
    let mut machine = build_machine_with(|_| {});

    // Port 0x40 bit 5 set enables the fixed-tone beeper.
    machine.bus.io_write(0x40, 0x20);
    machine.bus.set_current_cycle(200_000);
    let mut output = vec![0.0f32; 1024 * 2];
    machine.bus.generate_audio_samples(1.0, &mut output);
    assert!(
        output.iter().any(|&sample| sample != 0.0),
        "enabled beeper produced silence"
    );
}

#[test]
fn beeper_disabled_is_silent() {
    let mut machine = build_machine_with(|_| {});

    // Beeper gate left clear (port 0x40 bit 5 = 0).
    machine.bus.set_current_cycle(200_000);
    let mut output = vec![0.0f32; 1024 * 2];
    machine.bus.generate_audio_samples(1.0, &mut output);
    assert!(
        output.iter().all(|&sample| sample == 0.0),
        "disabled beeper produced output"
    );
}

#[test]
fn opn_compatible_programming_via_port_44() {
    // SR/MR-class software targets the OPN (YM2203) at the address/data pair
    // 0x44/0x45 only, never the extended 0x46/0x47 bank. The MA's OPNA is a
    // superset, so the same low-bank-only programming drives an SSG tone and
    // the status read at 0x44 responds rather than returning open bus.
    let mut machine = build_machine_with(|_| {});

    let tone: [(u8, u8); 4] = [
        (0x00, 0xFE), // channel A period, fine
        (0x01, 0x00), // channel A period, coarse
        (0x07, 0x3E), // mixer: tone A enabled (bit 0 clear), others off
        (0x08, 0x0F), // channel A amplitude, fixed maximum
    ];
    for (register, value) in tone {
        machine.bus.io_write(0x44, register);
        machine.bus.io_write(0x45, value);
    }

    machine.bus.set_current_cycle(400_000);

    // The status read at 0x44 must respond (the busy bit is clear once idle).
    let status = machine.bus.io_read(0x44).0;
    assert_eq!(status & 0x80, 0, "OPN status busy bit is clear when idle");

    let mut output = vec![0.0f32; 1024 * 2];
    machine.bus.generate_audio_samples(1.0, &mut output);
    assert!(
        output.iter().any(|&sample| sample != 0.0),
        "OPN-compatible SSG programming produced silence"
    );
}

#[test]
fn opna_note_produces_audio() {
    let mut machine = build_machine_with(|_| {});

    // Program an SSG square-wave tone on channel A: enable tone A in the mixer,
    // set a period, and drive the amplitude to its fixed maximum.
    let tone: [(u8, u8); 4] = [
        (0x00, 0xFE), // channel A period, fine
        (0x01, 0x00), // channel A period, coarse
        (0x07, 0x3E), // mixer: tone A enabled (bit 0 clear), others off
        (0x08, 0x0F), // channel A amplitude, fixed maximum
    ];
    for (reg, val) in tone {
        machine.bus.io_write(0x44, reg);
        machine.bus.io_write(0x45, val);
    }

    machine.bus.set_current_cycle(400_000);
    let mut output = vec![0.0f32; 1024 * 2];
    machine.bus.generate_audio_samples(1.0, &mut output);
    assert!(
        output.iter().any(|&sample| sample != 0.0),
        "OPNA SSG tone produced silence"
    );
}
