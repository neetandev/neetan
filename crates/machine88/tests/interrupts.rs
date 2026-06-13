//! Interrupt-delivery tests for the main Z80.
//!
//! A hand-assembled ROM installs an IM 2 vector table, enables the 600 Hz
//! CLOCK interrupt through ports 0xE4/0xE6, and loops `EI; HALT`. The CLOCK
//! interrupt service routine increments a RAM counter, proving vector
//! arithmetic (`logical_level * 2`), mask gating, and HLT wakeup. A decoy
//! vector installed at the level the raw (reversed) i8214 wiring would use must
//! never run.

use machine88::Pc8801Machine;

mod harness;
use harness::build_machine_with_rom;

/// Page holding the IM 2 vector table and the test counters (main RAM).
const VECTOR_PAGE: u8 = 0x90;
/// CLOCK interrupt vector slot: `level 2 * 2` within the table page.
const CLOCK_VECTOR_ADDRESS: u16 = 0x9004;
/// Decoy slot the raw i8214 mapping `(7 - level) * 2` would use for CLOCK.
const DECOY_VECTOR_ADDRESS: u16 = 0x900A;
/// Address of the ISR-incremented CLOCK counter (main RAM).
const CLOCK_COUNTER_ADDRESS: u16 = 0x9100;
/// Address of the decoy counter that must stay zero (main RAM).
const DECOY_COUNTER_ADDRESS: u16 = 0x9101;

const ISR_ADDRESS: u16 = 0x0050;
const DECOY_ISR_ADDRESS: u16 = 0x0070;

/// Builds the test ROM. `mask_value` is written to port 0xE6 (bit 0 unmasks
/// CLOCK); `target` is the count the main loop waits for before spinning.
fn build_rom(mask_value: u8, target: u8) -> Vec<u8> {
    let mut rom = vec![0u8; 0x8000];

    let init: &[u8] = &[
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
        0x04,
        0x90, // LD (0x9004), HL
        0x21,
        0x70,
        0x00, // LD HL, DECOY_ISR_ADDRESS (0x0070)
        0x22,
        0x0A,
        0x90, // LD (0x900A), HL
        0xAF, // XOR A
        0x32,
        0x00,
        0x91, // LD (0x9100), A
        0x32,
        0x01,
        0x91, // LD (0x9101), A
        0x3E,
        0x08, // LD A, 0x08 (enable all priority levels)
        0xD3,
        0xE4, // OUT (0xE4), A
        0x3E,
        mask_value, // LD A, mask_value
        0xD3,
        0xE6, // OUT (0xE6), A
        // loop (0x0025):
        0xFB, // EI
        0x76, // HALT
        0x3A,
        0x00,
        0x91, // LD A, (0x9100)
        0xFE,
        target, // CP target
        0x38,
        0xF7, // JR C, loop (-9)
        // done (0x002E):
        0x18,
        0xFE, // JR done (-2)
    ];
    rom[..init.len()].copy_from_slice(init);

    let isr: &[u8] = &[
        0xF5, // PUSH AF
        0x3A, 0x00, 0x91, // LD A, (0x9100)
        0x3C, // INC A
        0x32, 0x00, 0x91, // LD (0x9100), A
        0x3E, 0x08, // LD A, 0x08
        0xD3, 0xE4, // OUT (0xE4), A (re-arm priority)
        0xF1, // POP AF
        0xFB, // EI
        0xED, 0x4D, // RETI
    ];
    rom[ISR_ADDRESS as usize..ISR_ADDRESS as usize + isr.len()].copy_from_slice(isr);

    let decoy: &[u8] = &[
        0xF5, // PUSH AF
        0x3A, 0x01, 0x91, // LD A, (0x9101)
        0x3C, // INC A
        0x32, 0x01, 0x91, // LD (0x9101), A
        0x3E, 0x08, // LD A, 0x08
        0xD3, 0xE4, // OUT (0xE4), A
        0xF1, // POP AF
        0xFB, // EI
        0xED, 0x4D, // RETI
    ];
    rom[DECOY_ISR_ADDRESS as usize..DECOY_ISR_ADDRESS as usize + decoy.len()]
        .copy_from_slice(decoy);

    rom
}

/// Runs `machine` in fixed slices until `counter` reaches `target` or the cycle
/// cap is exhausted. Returns the final counter value.
fn run_until_counter(machine: &mut Pc8801Machine, counter: u16, target: u8) -> u8 {
    const STEP: u64 = 50_000;
    const CAP: u64 = 4_000_000;

    let mut total = 0u64;
    while total < CAP {
        machine.run_for(STEP);
        total += STEP;
        if machine.bus.peek_byte(counter) >= target {
            break;
        }
    }
    machine.bus.peek_byte(counter)
}

#[test]
fn clock_interrupt_wakes_halted_cpu() {
    let target = 5;
    let rom = build_rom(0x01, target);
    let mut machine = build_machine_with_rom(&rom);

    let count = run_until_counter(&mut machine, CLOCK_COUNTER_ADDRESS, target);
    assert!(
        count >= target,
        "CLOCK ISR ran {count} times, expected at least {target}"
    );

    // The decoy at the raw-i8214 vector slot must never have run, locking the
    // normalized `logical_level * 2` vector arithmetic.
    assert_eq!(
        machine.bus.peek_byte(DECOY_COUNTER_ADDRESS),
        0,
        "decoy ISR ran: CLOCK vector was not logical_level * 2"
    );
}

#[test]
fn masked_clock_interrupt_never_fires() {
    let target = 1;
    // Port 0xE6 = 0x00 leaves CLOCK masked.
    let rom = build_rom(0x00, target);
    let mut machine = build_machine_with_rom(&rom);

    let count = run_until_counter(&mut machine, CLOCK_COUNTER_ADDRESS, target);
    assert_eq!(count, 0, "masked CLOCK interrupt was delivered");
}

#[test]
fn vector_table_uses_clock_slot() {
    let rom = build_rom(0x01, 5);
    let mut machine = build_machine_with_rom(&rom);

    // Run long enough for the ROM's init code to install the vector table.
    machine.run_for(50_000);

    // The CLOCK slot holds the real ISR; the decoy slot holds the decoy ISR.
    assert_eq!(
        machine.bus.peek_byte(CLOCK_VECTOR_ADDRESS),
        (ISR_ADDRESS & 0xFF) as u8
    );
    assert_eq!(
        machine.bus.peek_byte(CLOCK_VECTOR_ADDRESS + 1),
        (ISR_ADDRESS >> 8) as u8
    );
    assert_eq!(
        machine.bus.peek_byte(DECOY_VECTOR_ADDRESS),
        (DECOY_ISR_ADDRESS & 0xFF) as u8
    );
}
