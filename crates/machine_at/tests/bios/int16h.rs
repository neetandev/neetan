//! INT 16h keyboard services: reads, peeks, shift status, buffer push,
//! typematic and the enhanced/compatible entry filtering.

use common::{Cpu, Machine};

use super::{
    BDA_KEYBOARD_HEAD, RESULT, boot_and_run, create_machine_dx50, inject_and_run, read_ram_u16,
    seed_keyboard_buffer, write_bytes,
};

/// FLAGS bit 6: zero.
const FLAGS_ZERO: u16 = 0x0040;
/// Cycle budget for the injected service call programs.
const CALL_BUDGET: u64 = 2_000_000;

/// AH=00h: reads a keystroke, stores AX.
#[rustfmt::skip]
const READ_CODE: &[u8] = &[
    0xB4, 0x00,             // MOV AH, 0x00
    0xCD, 0x16,             // INT 16h
    0xA3, 0x00, 0x06,       // MOV [0x0600], AX
    0xF4,                   // HLT
];

/// AH=10h: reads an enhanced keystroke, stores AX.
#[rustfmt::skip]
const READ_ENHANCED_CODE: &[u8] = &[
    0xB4, 0x10,             // MOV AH, 0x10
    0xCD, 0x16,             // INT 16h
    0xA3, 0x00, 0x06,       // MOV [0x0600], AX
    0xF4,                   // HLT
];

/// AH=00h with interrupts enabled: blocks until a keystroke arrives.
#[rustfmt::skip]
const BLOCKING_READ_CODE: &[u8] = &[
    0xFB,                   // STI
    0xB4, 0x00,             // MOV AH, 0x00
    0xCD, 0x16,             // INT 16h
    0xA3, 0x00, 0x06,       // MOV [0x0600], AX
    0xFA,                   // CLI (the read forced IF on)
    0xF4,                   // HLT
];

/// AH=01h: peeks, stores AX and the returned FLAGS.
#[rustfmt::skip]
const PEEK_CODE: &[u8] = &[
    0xB4, 0x01,             // MOV AH, 0x01
    0xCD, 0x16,             // INT 16h
    0xA3, 0x00, 0x06,       // MOV [0x0600], AX
    0x9C, 0x58,             // PUSHF; POP AX
    0xA3, 0x02, 0x06,       // MOV [0x0602], AX
    0xF4,                   // HLT
];

/// AH=02h: stores the shift flags returned in AL.
#[rustfmt::skip]
const SHIFT_STATUS_CODE: &[u8] = &[
    0xB4, 0x02,             // MOV AH, 0x02
    0xCD, 0x16,             // INT 16h
    0xA3, 0x00, 0x06,       // MOV [0x0600], AX
    0xF4,                   // HLT
];

/// AH=05h then AH=10h: pushes CX, stores the push status and the read-back.
#[rustfmt::skip]
const PUSH_AND_READ_CODE: &[u8] = &[
    0xB8, 0x00, 0x05,       // MOV AX, 0x0500
    0xB9, 0x34, 0x12,       // MOV CX, 0x1234
    0xCD, 0x16,             // INT 16h
    0xA2, 0x00, 0x06,       // MOV [0x0600], AL
    0xB4, 0x10,             // MOV AH, 0x10
    0xCD, 0x16,             // INT 16h
    0xA3, 0x02, 0x06,       // MOV [0x0602], AX
    0xF4,                   // HLT
];

/// AH=05h into a full buffer: stores the failure status in AL.
#[rustfmt::skip]
const PUSH_FULL_CODE: &[u8] = &[
    0xB8, 0x00, 0x05,       // MOV AX, 0x0500
    0xB9, 0x34, 0x12,       // MOV CX, 0x1234
    0xCD, 0x16,             // INT 16h
    0xA2, 0x00, 0x06,       // MOV [0x0600], AL
    0xF4,                   // HLT
];

/// AH=09h: stores the functionality bitmap returned in AL.
#[rustfmt::skip]
const FUNCTIONALITY_CODE: &[u8] = &[
    0xB4, 0x09,             // MOV AH, 0x09
    0xCD, 0x16,             // INT 16h
    0xA3, 0x00, 0x06,       // MOV [0x0600], AX
    0xF4,                   // HLT
];

/// AH=12h: stores the extended shift states returned in AX.
#[rustfmt::skip]
const EXTENDED_SHIFT_STATUS_CODE: &[u8] = &[
    0xB4, 0x12,             // MOV AH, 0x12
    0xCD, 0x16,             // INT 16h
    0xA3, 0x00, 0x06,       // MOV [0x0600], AX
    0xF4,                   // HLT
];

#[test]
fn read_returns_seeded_entry_and_advances_head() {
    let mut machine = create_machine_dx50();
    boot_to_halt!(machine);
    seed_keyboard_buffer(&mut machine, &[0x1E61]);
    inject_and_run(&mut machine, READ_CODE, &[], CALL_BUDGET);

    assert_eq!(read_ram_u16(&machine, RESULT), 0x1E61);
    assert_eq!(read_ram_u16(&machine, BDA_KEYBOARD_HEAD), 0x0020);
}

#[test]
fn read_blocks_until_a_late_key_arrives() {
    let mut machine = create_machine_dx50();
    boot_to_halt!(machine);
    inject_and_run(&mut machine, BLOCKING_READ_CODE, &[], 300_000);

    assert_eq!(read_ram_u16(&machine, RESULT), 0, "still blocked");
    assert!(!machine.cpu.halted(), "spinning on the rewound INT 16h");

    machine.push_keyboard_scancode(0x1E);
    machine.push_keyboard_scancode(0x9E);
    machine.run_for(3_000_000);

    assert_eq!(read_ram_u16(&machine, RESULT), 0x1E61, "woken by the key");
    assert!(machine.cpu.halted(), "the program ran to completion");
}

#[test]
fn peek_reports_zero_flag_when_empty() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, PEEK_CODE, &[], CALL_BUDGET);

    assert_eq!(
        read_ram_u16(&machine, RESULT + 2) & FLAGS_ZERO,
        FLAGS_ZERO,
        "ZF set"
    );
}

#[test]
fn peek_returns_entry_without_consuming() {
    let mut machine = create_machine_dx50();
    boot_to_halt!(machine);
    seed_keyboard_buffer(&mut machine, &[0x1E61]);
    inject_and_run(&mut machine, PEEK_CODE, &[], CALL_BUDGET);

    assert_eq!(read_ram_u16(&machine, RESULT), 0x1E61);
    assert_eq!(
        read_ram_u16(&machine, RESULT + 2) & FLAGS_ZERO,
        0,
        "ZF clear"
    );
    assert_eq!(
        read_ram_u16(&machine, BDA_KEYBOARD_HEAD),
        0x001E,
        "head unmoved"
    );
}

#[test]
fn read_filters_extended_entry_to_compatible_form() {
    let mut machine = create_machine_dx50();
    boot_to_halt!(machine);
    seed_keyboard_buffer(&mut machine, &[0x48E0]);
    inject_and_run(&mut machine, READ_CODE, &[], CALL_BUDGET);

    assert_eq!(
        read_ram_u16(&machine, RESULT),
        0x4800,
        "grey up loses the E0 marker for AH=00h"
    );
}

#[test]
fn read_remaps_keypad_enter_to_classic_scan() {
    let mut machine = create_machine_dx50();
    boot_to_halt!(machine);
    seed_keyboard_buffer(&mut machine, &[0xE00D]);
    inject_and_run(&mut machine, READ_CODE, &[], CALL_BUDGET);

    assert_eq!(read_ram_u16(&machine, RESULT), 0x1C0D, "classic Enter scan");
}

#[test]
fn enhanced_read_passes_extended_entry_verbatim() {
    let mut machine = create_machine_dx50();
    boot_to_halt!(machine);
    seed_keyboard_buffer(&mut machine, &[0x48E0]);
    inject_and_run(&mut machine, READ_ENHANCED_CODE, &[], CALL_BUDGET);

    assert_eq!(read_ram_u16(&machine, RESULT), 0x48E0);
}

#[test]
fn peek_discards_enhanced_only_entry() {
    let mut machine = create_machine_dx50();
    boot_to_halt!(machine);
    seed_keyboard_buffer(&mut machine, &[0x8500]);
    inject_and_run(&mut machine, PEEK_CODE, &[], CALL_BUDGET);

    assert_eq!(
        read_ram_u16(&machine, RESULT + 2) & FLAGS_ZERO,
        FLAGS_ZERO,
        "F11 is invisible to AH=01h"
    );
    assert_eq!(
        read_ram_u16(&machine, BDA_KEYBOARD_HEAD),
        0x0020,
        "the incompatible entry was removed while checking"
    );
}

#[test]
fn shift_status_reads_the_bda_flags() {
    let mut machine = create_machine_dx50();
    boot_to_halt!(machine);
    write_bytes(&mut machine, 0x417, &[0x63]);
    inject_and_run(&mut machine, SHIFT_STATUS_CODE, &[], CALL_BUDGET);

    assert_eq!(read_ram_u16(&machine, RESULT) & 0x00FF, 0x63);
}

#[test]
fn push_key_then_enhanced_read_round_trips() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, PUSH_AND_READ_CODE, &[], CALL_BUDGET);

    assert_eq!(read_ram_u16(&machine, RESULT) & 0x00FF, 0, "push succeeded");
    assert_eq!(read_ram_u16(&machine, RESULT + 2), 0x1234, "read back");
}

#[test]
fn push_into_full_buffer_reports_failure() {
    let mut machine = create_machine_dx50();
    boot_to_halt!(machine);
    seed_keyboard_buffer(&mut machine, &[0x0101; 15]);
    inject_and_run(&mut machine, PUSH_FULL_CODE, &[], CALL_BUDGET);

    assert_eq!(read_ram_u16(&machine, RESULT) & 0x00FF, 1, "buffer full");
}

#[test]
fn functionality_reports_enhanced_support() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, FUNCTIONALITY_CODE, &[], CALL_BUDGET);

    assert_eq!(
        read_ram_u16(&machine, RESULT) & 0x00FF,
        0x24,
        "AH=10h-12h and AX=0305h supported"
    );
}

#[test]
fn extended_shift_status_composes_both_flag_bytes() {
    let mut machine = create_machine_dx50();
    boot_to_halt!(machine);
    // Flags 1, flags 2 (left ctrl, sysreq, caps pressed), accumulator.
    write_bytes(&mut machine, 0x417, &[0x03, 0x45]);
    // Keyboard mode: enhanced keyboard, right alt pressed.
    write_bytes(&mut machine, 0x496, &[0x18]);
    inject_and_run(&mut machine, EXTENDED_SHIFT_STATUS_CODE, &[], CALL_BUDGET);

    assert_eq!(
        read_ram_u16(&machine, RESULT),
        0xC903,
        "AL mirrors flags 1, AH packs left ctrl, right alt, caps, sysreq"
    );
}
