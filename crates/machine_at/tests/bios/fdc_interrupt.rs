//! INT 0Eh (IRQ 6) diskette completion flag for direct hardware users.

use super::{create_machine_dx50, inject_and_run, make_pattern_floppy, read_ram_u8};
use crate::FLOPPY_1440K_SIZE;

/// BIOS data area: diskette recalibrate and interrupt status.
const BDA_FLOPPY_RECALIBRATE: u32 = 0x43E;
/// Cycle budget for the polled reset round trip.
const FDC_IRQ_BUDGET: u64 = 20_000_000;

/// Pulses the FDC reset through the DOR with interrupts enabled and polls
/// BDA 40:3E until the completion flag appears, then stores the byte.
#[rustfmt::skip]
const RESET_POLL_CODE: &[u8] = &[
    0xFB,                   // STI
    0xBA, 0xF2, 0x03,       // MOV DX, 0x3F2
    0xB0, 0x08,             // MOV AL, 0x08 (assert reset, gates open)
    0xEE,                   // OUT DX, AL
    0xB0, 0x0C,             // MOV AL, 0x0C (release reset)
    0xEE,                   // OUT DX, AL
    0xA0, 0x3E, 0x04,       // MOV AL, [0x043E]
    0xA8, 0x80,             // TEST AL, 0x80
    0x74, 0xF9,             // JZ back to the poll
    0xA2, 0x00, 0x06,       // MOV [0x0600], AL
    0xF4,                   // HLT
];

/// Like `RESET_POLL_CODE`, but consumes the completion flag before storing
/// the byte, the way a guest FDC driver acknowledges the wait.
#[rustfmt::skip]
const RESET_POLL_CONSUME_CODE: &[u8] = &[
    0xFB,                         // STI
    0xBA, 0xF2, 0x03,             // MOV DX, 0x3F2
    0xB0, 0x08,                   // MOV AL, 0x08
    0xEE,                         // OUT DX, AL
    0xB0, 0x0C,                   // MOV AL, 0x0C
    0xEE,                         // OUT DX, AL
    0xA0, 0x3E, 0x04,             // MOV AL, [0x043E]
    0xA8, 0x80,                   // TEST AL, 0x80
    0x74, 0xF9,                   // JZ back to the poll
    0x80, 0x26, 0x3E, 0x04, 0x7F, // AND BYTE [0x043E], 0x7F
    0xA0, 0x3E, 0x04,             // MOV AL, [0x043E]
    0xA2, 0x00, 0x06,             // MOV [0x0600], AL
    0xF4,                         // HLT
];

#[test]
fn irq6_sets_completion_flag_and_eois() {
    let mut machine = create_machine_dx50();
    machine
        .bus
        .insert_floppy(0, make_pattern_floppy(FLOPPY_1440K_SIZE), None)
        .expect("insert pattern floppy");
    boot_to_halt!(machine);
    inject_and_run(&mut machine, RESET_POLL_CODE, &[], FDC_IRQ_BUDGET);

    let stored = read_ram_u8(&machine, super::RESULT);
    assert_ne!(stored & 0x80, 0, "completion flag observed by the poller");
    assert_ne!(
        read_ram_u8(&machine, BDA_FLOPPY_RECALIBRATE) & 0x80,
        0,
        "completion flag set in the BDA"
    );
    let state = machine.inspection_state();
    assert_eq!(
        state.pic.chips[0].isr & 0x40,
        0,
        "IRQ 6 in-service bit cleared by the stub EOI"
    );
}

#[test]
fn completion_flag_visible_to_poller() {
    let mut machine = create_machine_dx50();
    machine
        .bus
        .insert_floppy(0, make_pattern_floppy(FLOPPY_1440K_SIZE), None)
        .expect("insert pattern floppy");
    boot_to_halt!(machine);
    inject_and_run(&mut machine, RESET_POLL_CONSUME_CODE, &[], FDC_IRQ_BUDGET);

    // The boot read left the drive 0 recalibrate bit; the poller consumed
    // the interrupt flag on top of it.
    assert_eq!(read_ram_u8(&machine, super::RESULT), 0x01, "consumed flag");
    assert_eq!(
        read_ram_u8(&machine, BDA_FLOPPY_RECALIBRATE),
        0x01,
        "40:3E after the acknowledge"
    );
}
