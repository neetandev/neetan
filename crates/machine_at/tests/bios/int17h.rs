//! INT 17h printer services and INT 05h print screen.
//!
//! The golden values were captured from the real AMI BIOS on the same
//! LPT-less machine (see the tempt_real_bios_probes.rs capture runs): INT 17h
//! returns with all registers untouched because every BDA LPT base word is 0,
//! and INT 05h leaves 0xFF (no printer) in the status byte at 50:00.

use super::{RESULT, boot_and_run, create_machine_dx50, read_ram_u8, read_ram_u16, write_bytes};

/// FLAGS bit 0: carry.
const FLAGS_CARRY: u16 = 0x0001;

/// INT 17h AH=00h/01h/02h on LPT1 and AH=02h on an out-of-range port, with
/// poisoned AL: stores AX and FLAGS per call.
#[rustfmt::skip]
const PRINTER_FUNCTIONS_CODE: &[u8] = &[
    0xB8, 0x41, 0x00,       // MOV AX, 0x0041 (AH=00h print, AL='A')
    0xBA, 0x00, 0x00,       // MOV DX, 0
    0xCD, 0x17,             // INT 17h
    0xA3, 0x00, 0x06,       // MOV [0x0600], AX
    0x9C, 0x58,             // PUSHF; POP AX
    0xA3, 0x02, 0x06,       // MOV [0x0602], AX
    0xB8, 0x41, 0x01,       // MOV AX, 0x0141 (AH=01h initialize)
    0xBA, 0x00, 0x00,       // MOV DX, 0
    0xCD, 0x17,             // INT 17h
    0xA3, 0x04, 0x06,       // MOV [0x0604], AX
    0xB8, 0x41, 0x02,       // MOV AX, 0x0241 (AH=02h status)
    0xBA, 0x00, 0x00,       // MOV DX, 0
    0xCD, 0x17,             // INT 17h
    0xA3, 0x06, 0x06,       // MOV [0x0606], AX
    0xB8, 0x34, 0x02,       // MOV AX, 0x0234 (AH=02h, poisoned AL)
    0xBA, 0x03, 0x00,       // MOV DX, 3 (no such port)
    0xCD, 0x17,             // INT 17h
    0xA3, 0x08, 0x06,       // MOV [0x0608], AX
    0xF4,                   // HLT
];

/// INT 05h with poisoned registers: stores AX afterwards.
#[rustfmt::skip]
const PRINT_SCREEN_CODE: &[u8] = &[
    0xB8, 0x5A, 0xA5,       // MOV AX, 0xA55A
    0xCD, 0x05,             // INT 05h
    0xA3, 0x00, 0x06,       // MOV [0x0600], AX
    0xF4,                   // HLT
];

#[test]
fn printer_functions_return_untouched() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, PRINTER_FUNCTIONS_CODE, &[], 1_000_000);

    assert_eq!(read_ram_u16(&machine, RESULT), 0x0041, "AH=00h preserved");
    assert_eq!(
        read_ram_u16(&machine, RESULT + 2) & FLAGS_CARRY,
        0,
        "carry untouched"
    );
    assert_eq!(
        read_ram_u16(&machine, RESULT + 4),
        0x0141,
        "AH=01h preserved"
    );
    assert_eq!(
        read_ram_u16(&machine, RESULT + 6),
        0x0241,
        "AH=02h preserved"
    );
    assert_eq!(
        read_ram_u16(&machine, RESULT + 8),
        0x0234,
        "out-of-range port preserved"
    );
}

#[test]
fn print_screen_reports_no_printer() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, &[0xF4], &[], 1_000_000);
    write_bytes(&mut machine, 0x500, &[0xAA]);
    super::inject_and_run(&mut machine, PRINT_SCREEN_CODE, &[], 1_000_000);

    assert_eq!(
        read_ram_u8(&machine, 0x500),
        0xFF,
        "print screen status: failed, no printer"
    );
    assert_eq!(
        read_ram_u16(&machine, RESULT),
        0xA55A,
        "registers preserved"
    );
}
