use crate::harness;

#[test]
fn switch_char_get_availdev() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, 0x02, 0x37,                       // MOV AX, 3702h
        0xCD, 0x21,                             // INT 21h
        0xA2, 0x00, 0x01,                       // MOV [0100h], AL
        0x88, 0x16, 0x01, 0x01,                 // MOV [0101h], DL
        0xFA, 0xF4,                             // CLI; HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let al = harness::result_byte(&machine.bus, 0);
    let dl = harness::result_byte(&machine.bus, 1);
    assert_eq!(al, 0x00, "AX=3702h: AL should be 00h, got {al:#04X}");
    assert_eq!(
        dl, 0xFF,
        "AX=3702h: DL should be FFh (availdev true), got {dl:#04X}"
    );
}

#[test]
fn switch_char_set_availdev_ignored() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB2, 0x00,                             // MOV DL, 00h
        0xB8, 0x03, 0x37,                       // MOV AX, 3703h
        0xCD, 0x21,                             // INT 21h
        0xA2, 0x00, 0x01,                       // MOV [0100h], AL
        0xFA, 0xF4,                             // CLI; HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let al = harness::result_byte(&machine.bus, 0);
    assert_eq!(al, 0x00, "AX=3703h: AL should be 00h, got {al:#04X}");
}

#[test]
fn swap_break_flag() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        // First get current break state (should be 0).
        0xB8, 0x00, 0x33,                       // MOV AX, 3300h
        0xCD, 0x21,                             // INT 21h
        0x88, 0x16, 0x00, 0x01,                 // MOV [0100h], DL  (initial state)
        // Now swap: set DL=1, call AX=3302h.
        0xB2, 0x01,                             // MOV DL, 01h
        0xB8, 0x02, 0x33,                       // MOV AX, 3302h
        0xCD, 0x21,                             // INT 21h
        0x88, 0x16, 0x01, 0x01,                 // MOV [0101h], DL  (old value returned)
        // Now verify new state is 1.
        0xB8, 0x00, 0x33,                       // MOV AX, 3300h
        0xCD, 0x21,                             // INT 21h
        0x88, 0x16, 0x02, 0x01,                 // MOV [0102h], DL  (new state)
        0xFA, 0xF4,                             // CLI; HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let initial = harness::result_byte(&machine.bus, 0);
    let old_from_swap = harness::result_byte(&machine.bus, 1);
    let new_state = harness::result_byte(&machine.bus, 2);
    assert_eq!(initial, 0x00, "Initial break state should be 0");
    assert_eq!(old_from_swap, 0x00, "Swap should return old value 0 in DL");
    assert_eq!(new_state, 0x01, "After swap, break state should be 1");
}

#[test]
fn code_page_reserved() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, 0x03, 0x33,                       // MOV AX, 3303h
        0xCD, 0x21,                             // INT 21h
        0xA2, 0x00, 0x01,                       // MOV [0100h], AL
        0xB8, 0x04, 0x33,                       // MOV AX, 3304h
        0xCD, 0x21,                             // INT 21h
        0xA2, 0x01, 0x01,                       // MOV [0101h], AL
        0xFA, 0xF4,                             // CLI; HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let al_03 = harness::result_byte(&machine.bus, 0);
    let al_04 = harness::result_byte(&machine.bus, 1);
    assert_eq!(al_03, 0xFF, "AX=3303h should return AL=FFh");
    assert_eq!(al_04, 0xFF, "AX=3304h should return AL=FFh");
}

#[test]
fn file_datetime_extended_attr_stubs() {
    let mut machine = harness::boot_hle();
    // Use handle 1 (stdout) which is always open.
    #[rustfmt::skip]
    let code: &[u8] = &[
        // AX=5702h: Get extended attributes.
        0xBB, 0x01, 0x00,                       // MOV BX, 0001h (stdout)
        0xB8, 0x02, 0x57,                       // MOV AX, 5702h
        0xCD, 0x21,                             // INT 21h
        0x9C,                                   // PUSHF
        0x58,                                   // POP AX
        0xA3, 0x00, 0x01,                       // MOV [0100h], AX  (flags for 5702h)
        // AX=5703h: Get extended attributes name list.
        0xBB, 0x01, 0x00,                       // MOV BX, 0001h
        0xB8, 0x03, 0x57,                       // MOV AX, 5703h
        0xCD, 0x21,                             // INT 21h
        0x9C,                                   // PUSHF
        0x58,                                   // POP AX
        0xA3, 0x02, 0x01,                       // MOV [0102h], AX  (flags for 5703h)
        // AX=5704h: Set extended attributes.
        0xBB, 0x01, 0x00,                       // MOV BX, 0001h
        0xB8, 0x04, 0x57,                       // MOV AX, 5704h
        0xCD, 0x21,                             // INT 21h
        0x9C,                                   // PUSHF
        0x58,                                   // POP AX
        0xA3, 0x04, 0x01,                       // MOV [0104h], AX  (flags for 5704h)
        0xFA, 0xF4,                             // CLI; HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let flags_02 = harness::result_word(&machine.bus, 0);
    let flags_03 = harness::result_word(&machine.bus, 2);
    let flags_04 = harness::result_word(&machine.bus, 4);
    assert_eq!(flags_02 & 1, 0, "AX=5702h should return CF=0");
    assert_eq!(flags_03 & 1, 0, "AX=5703h should return CF=0");
    assert_eq!(flags_04 & 1, 0, "AX=5704h should return CF=0");
}

#[test]
fn path_parse_noop() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, 0x00, 0x61,                       // MOV AX, 6100h
        0xCD, 0x21,                             // INT 21h
        0xA2, 0x00, 0x01,                       // MOV [0100h], AL
        0xFA, 0xF4,                             // CLI; HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let al = harness::result_byte(&machine.bus, 0);
    assert_eq!(al, 0x00, "AH=61h should return AL=00h (no-op)");
}

#[test]
fn ifs_ioctl_not_supported() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, 0x00, 0x6B,                       // MOV AX, 6B00h
        0xCD, 0x21,                             // INT 21h
        0x9C,                                   // PUSHF
        0x58,                                   // POP AX
        0xA3, 0x00, 0x01,                       // MOV [0100h], AX
        0xFA, 0xF4,                             // CLI; HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let flags = harness::result_word(&machine.bus, 0);
    assert_ne!(flags & 1, 0, "AH=6Bh should return CF=1");
}

#[test]
fn set_wait_external_noop() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, 0x00, 0x64,                       // MOV AX, 6400h
        0xCD, 0x21,                             // INT 21h
        // If it returns at all, the test passes.
        0xC6, 0x06, 0x00, 0x01, 0x01,           // MOV BYTE [0100h], 01h
        0xFA, 0xF4,                             // CLI; HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let marker = harness::result_byte(&machine.bus, 0);
    assert_eq!(marker, 0x01, "AH=64h should return without error");
}

#[test]
fn uppercase_char() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB2, 0x61,                             // MOV DL, 'a'
        0xB8, 0x20, 0x65,                       // MOV AX, 6520h
        0xCD, 0x21,                             // INT 21h
        0x88, 0x16, 0x00, 0x01,                 // MOV [0100h], DL
        0xB2, 0x7A,                             // MOV DL, 'z'
        0xB8, 0x20, 0x65,                       // MOV AX, 6520h
        0xCD, 0x21,                             // INT 21h
        0x88, 0x16, 0x01, 0x01,                 // MOV [0101h], DL
        0xB2, 0x31,                             // MOV DL, '1' (non-alpha)
        0xB8, 0x20, 0x65,                       // MOV AX, 6520h
        0xCD, 0x21,                             // INT 21h
        0x88, 0x16, 0x02, 0x01,                 // MOV [0102h], DL
        0xFA, 0xF4,                             // CLI; HLT
    ];
    harness::inject_and_run(&mut machine, code);

    assert_eq!(harness::result_byte(&machine.bus, 0), b'A');
    assert_eq!(harness::result_byte(&machine.bus, 1), b'Z');
    assert_eq!(harness::result_byte(&machine.bus, 2), b'1');
}

#[test]
fn uppercase_asciiz() {
    let mut machine = harness::boot_hle();
    let base = harness::INJECT_CODE_BASE;

    // Write test string at +0x0200 in the segment.
    harness::write_bytes(&mut machine.bus, base + 0x0200, b"hello\0");

    #[rustfmt::skip]
    let code: &[u8] = &[
        0xBA, 0x00, 0x02,                       // MOV DX, 0200h
        0xB8, 0x22, 0x65,                       // MOV AX, 6522h
        0xCD, 0x21,                             // INT 21h
        // Copy result to result area.
        0xBE, 0x00, 0x02,                       // MOV SI, 0200h
        0xBF, 0x00, 0x01,                       // MOV DI, 0100h
        0xB9, 0x06, 0x00,                       // MOV CX, 6
        0xF3, 0xA4,                             // REP MOVSB
        0xFA, 0xF4,                             // CLI; HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let result = harness::read_bytes(&machine.bus, harness::INJECT_RESULT_BASE, 5);
    assert_eq!(&result, b"HELLO");
}

#[test]
fn yesno_check() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        // 'Y' should be yes/no char -> AX=0
        0xB2, 0x59,                             // MOV DL, 'Y'
        0xB8, 0x23, 0x65,                       // MOV AX, 6523h
        0xCD, 0x21,                             // INT 21h
        0xA3, 0x00, 0x01,                       // MOV [0100h], AX
        // 'X' should not be -> AX=2
        0xB2, 0x58,                             // MOV DL, 'X'
        0xB8, 0x23, 0x65,                       // MOV AX, 6523h
        0xCD, 0x21,                             // INT 21h
        0xA3, 0x02, 0x01,                       // MOV [0102h], AX
        0xFA, 0xF4,                             // CLI; HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let ax_y = harness::result_word(&machine.bus, 0);
    let ax_x = harness::result_word(&machine.bus, 2);
    assert_eq!(ax_y, 0x0000, "'Y' should be a yes/no char (AX=0)");
    assert_eq!(ax_x, 0x0002, "'X' should not be a yes/no char (AX=2)");
}

#[test]
fn country_lowercase_table() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB9, 0x07, 0x00,                       // MOV CX, 7 (buffer size)
        0x8C, 0xC0,                             // MOV AX, ES
        0x8E, 0xC0,                             // MOV ES, AX
        0xBF, 0x00, 0x01,                       // MOV DI, 0100h
        0xB8, 0x03, 0x65,                       // MOV AX, 6503h
        0xCD, 0x21,                             // INT 21h
        0x9C,                                   // PUSHF
        0x58,                                   // POP AX
        0xA3, 0x08, 0x01,                       // MOV [0108h], AX (flags)
        0xFA, 0xF4,                             // CLI; HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let info_id = harness::result_byte(&machine.bus, 0);
    let flags = harness::result_word(&machine.bus, 8);
    assert_eq!(flags & 1, 0, "AX=6503h should return CF=0");
    assert_eq!(info_id, 0x03, "Info ID should be 03h");
}

#[test]
fn country_filename_char_table() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB9, 0x07, 0x00,                      // MOV CX, 7 (buffer size)
        0x8C, 0xC0,                             // MOV AX, ES
        0x8E, 0xC0,                             // MOV ES, AX
        0xBF, 0x00, 0x01,                       // MOV DI, 0100h
        0xB8, 0x05, 0x65,                       // MOV AX, 6505h
        0xCD, 0x21,                             // INT 21h
        0x9C,                                   // PUSHF
        0x58,                                   // POP AX
        0xA3, 0x08, 0x01,                       // MOV [0108h], AX (flags)
        0xFA, 0xF4,                             // CLI; HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let info_id = harness::result_byte(&machine.bus, 0);
    let flags = harness::result_word(&machine.bus, 8);
    assert_eq!(flags & 1, 0, "AX=6505h should return CF=0");
    assert_eq!(info_id, 0x05, "Info ID should be 05h");
}

#[test]
fn get_dpb_default_drive() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0x06,                                   // PUSH ES
        0x0E,                                   // PUSH CS
        0x07,                                   // POP ES  (ES = code segment)
        0xB4, 0x1F,                             // MOV AH, 1Fh
        0xCD, 0x21,                             // INT 21h
        0x26, 0xA2, 0x00, 0x01,                 // MOV ES:[0100h], AL
        0x26, 0x89, 0x1E, 0x01, 0x01,           // MOV ES:[0101h], BX
        0x26, 0x8C, 0x1E, 0x03, 0x01,           // MOV ES:[0103h], DS
        0x07,                                   // POP ES
        0xFA, 0xF4,                             // CLI; HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let al = harness::result_byte(&machine.bus, 0);
    let bx = harness::result_word(&machine.bus, 1);
    let ds = harness::result_word(&machine.bus, 3);
    assert_eq!(al, 0x00, "AH=1Fh: AL should be 00h, got {al:#04X}");
    assert!(
        ds != 0 || bx != 0,
        "AH=1Fh: DS:BX should be a non-null pointer"
    );
}

#[test]
fn get_dpb_specified_drive() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0x06,                                   // PUSH ES
        0x0E,                                   // PUSH CS
        0x07,                                   // POP ES
        0xB2, 0x01,                             // MOV DL, 01h (A:)
        0xB4, 0x32,                             // MOV AH, 32h
        0xCD, 0x21,                             // INT 21h
        0x26, 0xA2, 0x00, 0x01,                 // MOV ES:[0100h], AL
        0x26, 0x89, 0x1E, 0x01, 0x01,           // MOV ES:[0101h], BX
        0x26, 0x8C, 0x1E, 0x03, 0x01,           // MOV ES:[0103h], DS
        0x07,                                   // POP ES
        0xFA, 0xF4,                             // CLI; HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let al = harness::result_byte(&machine.bus, 0);
    // The boot drive may not be A:, so AL could be FFh.
    // Just verify that the function returns without crashing.
    assert!(
        al == 0x00 || al == 0xFF,
        "AH=32h: AL should be 00h or FFh, got {al:#04X}"
    );
}

#[test]
fn get_dpb_invalid_drive() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB2, 0x1B,                             // MOV DL, 1Bh (27, invalid)
        0xB4, 0x32,                             // MOV AH, 32h
        0xCD, 0x21,                             // INT 21h
        0xA2, 0x00, 0x01,                       // MOV [0100h], AL
        0xFA, 0xF4,                             // CLI; HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let al = harness::result_byte(&machine.bus, 0);
    assert_eq!(al, 0xFF, "AH=32h with invalid drive should return AL=FFh");
}

#[test]
fn truename_simple_path() {
    let mut machine = harness::boot_hle();
    let base = harness::INJECT_CODE_BASE;

    // Write input path at +0x0200.
    harness::write_bytes(&mut machine.bus, base + 0x0200, b"A:\\\0");

    #[rustfmt::skip]
    let code: &[u8] = &[
        0xBE, 0x00, 0x02,                       // MOV SI, 0200h
        0xBF, 0x00, 0x01,                       // MOV DI, 0100h
        0xB4, 0x60,                             // MOV AH, 60h
        0xCD, 0x21,                             // INT 21h
        0x9C,                                   // PUSHF
        0x58,                                   // POP AX
        0xA3, 0x80, 0x01,                       // MOV [0180h], AX (flags)
        0xFA, 0xF4,                             // CLI; HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let flags = harness::result_word(&machine.bus, 0x80);
    assert_eq!(flags & 1, 0, "AH=60h should return CF=0");

    let result = harness::read_string(&machine.bus, harness::INJECT_RESULT_BASE, 128);
    assert_eq!(&result, b"A:\\", "TRUENAME of 'A:\\' should be 'A:\\'");
}

#[test]
fn truename_dotdot() {
    let mut machine = harness::boot_hle();
    let base = harness::INJECT_CODE_BASE;

    // Write input path at +0x0200.
    harness::write_bytes(&mut machine.bus, base + 0x0200, b"A:\\DIR\\..\0");

    #[rustfmt::skip]
    let code: &[u8] = &[
        0xBE, 0x00, 0x02,                       // MOV SI, 0200h
        0xBF, 0x00, 0x01,                       // MOV DI, 0100h
        0xB4, 0x60,                             // MOV AH, 60h
        0xCD, 0x21,                             // INT 21h
        0xFA, 0xF4,                             // CLI; HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let result = harness::read_string(&machine.bus, harness::INJECT_RESULT_BASE, 128);
    assert_eq!(
        &result, b"A:\\",
        "TRUENAME of 'A:\\DIR\\..' should be 'A:\\'"
    );
}

#[test]
fn truename_relative() {
    let mut machine = harness::boot_hle();
    let base = harness::INJECT_CODE_BASE;

    // Write a relative path at +0x0200.
    harness::write_bytes(&mut machine.bus, base + 0x0200, b"test.txt\0");

    #[rustfmt::skip]
    let code: &[u8] = &[
        0xBE, 0x00, 0x02,                       // MOV SI, 0200h
        0xBF, 0x00, 0x01,                       // MOV DI, 0100h
        0xB4, 0x60,                             // MOV AH, 60h
        0xCD, 0x21,                             // INT 21h
        0xFA, 0xF4,                             // CLI; HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let result = harness::read_string(&machine.bus, harness::INJECT_RESULT_BASE, 128);
    // Should be uppercased and have the current drive + CWD prepended.
    assert!(
        result.len() >= 4,
        "TRUENAME result should have at least 'X:\\' prefix, got {:?}",
        String::from_utf8_lossy(&result)
    );
    assert_eq!(result[1], b':', "Should have drive letter colon");
    assert_eq!(result[2], b'\\', "Should have root backslash");
    assert!(
        result.ends_with(b"TEST.TXT"),
        "Should end with uppercased filename, got {:?}",
        String::from_utf8_lossy(&result)
    );
}

#[test]
fn create_child_psp() {
    let mut machine = harness::boot_hle();
    // Use segment 0x3000 for the child PSP (well into free memory).
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xBA, 0x00, 0x30,                       // MOV DX, 3000h (child segment)
        0xBE, 0x00, 0xA0,                       // MOV SI, A000h (mem top)
        0xB4, 0x55,                             // MOV AH, 55h
        0xCD, 0x21,                             // INT 21h
        // Read PSP bytes at 3000:0000 (INT 20h opcode = CD 20).
        0x1E,                                   // PUSH DS
        0xB8, 0x00, 0x30,                       // MOV AX, 3000h
        0x8E, 0xD8,                             // MOV DS, AX
        0xA0, 0x00, 0x00,                       // MOV AL, [0000h] (should be 0xCD)
        0x1F,                                   // POP DS
        0xA2, 0x00, 0x01,                       // MOV [0100h], AL
        // Read PSP byte at +0x01 (should be 0x20).
        0x1E,                                   // PUSH DS
        0xB8, 0x00, 0x30,                       // MOV AX, 3000h
        0x8E, 0xD8,                             // MOV DS, AX
        0xA0, 0x01, 0x00,                       // MOV AL, [0001h]
        0x1F,                                   // POP DS
        0xA2, 0x01, 0x01,                       // MOV [0101h], AL
        // Read memory top at PSP+0x02.
        0x1E,                                   // PUSH DS
        0xB8, 0x00, 0x30,                       // MOV AX, 3000h
        0x8E, 0xD8,                             // MOV DS, AX
        0xA1, 0x02, 0x00,                       // MOV AX, [0002h]
        0x1F,                                   // POP DS
        0xA3, 0x02, 0x01,                       // MOV [0102h], AX
        0xFA, 0xF4,                             // CLI; HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let int20_cd = harness::result_byte(&machine.bus, 0);
    let int20_20 = harness::result_byte(&machine.bus, 1);
    let mem_top = harness::result_word(&machine.bus, 2);
    assert_eq!(int20_cd, 0xCD, "PSP+0x00 should be INT opcode (0xCD)");
    assert_eq!(int20_20, 0x20, "PSP+0x01 should be 0x20");
    assert_eq!(mem_top, 0xA000, "PSP+0x02 memory top should be A000h");
}

#[test]
fn commit_file_68h() {
    let mut machine = harness::boot_hle();
    // Use handle 1 (stdout).
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xBB, 0x01, 0x00,                       // MOV BX, 0001h
        0xB4, 0x68,                             // MOV AH, 68h
        0xCD, 0x21,                             // INT 21h
        0x9C,                                   // PUSHF
        0x58,                                   // POP AX
        0xA3, 0x00, 0x01,                       // MOV [0100h], AX
        0xFA, 0xF4,                             // CLI; HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let flags = harness::result_word(&machine.bus, 0);
    assert_eq!(flags & 1, 0, "AH=68h should return CF=0 for valid handle");
}

#[test]
fn commit_file_6ah() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xBB, 0x01, 0x00,                       // MOV BX, 0001h
        0xB4, 0x6A,                             // MOV AH, 6Ah
        0xCD, 0x21,                             // INT 21h
        0x9C,                                   // PUSHF
        0x58,                                   // POP AX
        0xA3, 0x00, 0x01,                       // MOV [0100h], AX
        0xFA, 0xF4,                             // CLI; HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let flags = harness::result_word(&machine.bus, 0);
    assert_eq!(flags & 1, 0, "AH=6Ah should return CF=0 for valid handle");
}

#[test]
fn get_media_info() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB3, 0x00,                             // MOV BL, 00h (default drive)
        0xBA, 0x00, 0x01,                       // MOV DX, 0100h (buffer)
        0xB8, 0x00, 0x69,                       // MOV AX, 6900h
        0xCD, 0x21,                             // INT 21h
        0x9C,                                   // PUSHF
        0x58,                                   // POP AX
        0xA3, 0x80, 0x01,                       // MOV [0180h], AX (flags)
        0xFA, 0xF4,                             // CLI; HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let flags = harness::result_word(&machine.bus, 0x80);
    assert_eq!(flags & 1, 0, "AH=69h AL=00h should return CF=0");

    // Verify file system type at buffer+17 (8 bytes).
    let fs_type = harness::read_bytes(&machine.bus, harness::INJECT_RESULT_BASE + 17, 8);
    assert_eq!(&fs_type, b"FAT12   ");
}

#[test]
fn extended_country_info_invalid_subfunction() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, 0x99, 0x65,                       // MOV AX, 6599h (invalid subfunction)
        0xCD, 0x21,                             // INT 21h
        0x9C,                                   // PUSHF
        0x58,                                   // POP AX
        0xA3, 0x00, 0x01,                       // MOV [0100h], AX (flags)
        0xFA, 0xF4,                             // CLI; HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let flags = harness::result_word(&machine.bus, 0);
    assert_ne!(
        flags & 1,
        0,
        "AX=6599h should return CF=1 (invalid function)"
    );
}

#[test]
fn exec_load_only_com() {
    let mut machine = harness::boot_hle_with_floppy();

    // Data area layout (relative to INJECT_CODE_BASE):
    //   +0x0200: filename ASCIIZ
    //   +0x0210: parameter block (22 bytes for AX=4B01h)
    //   +0x0230: command tail
    let base = harness::INJECT_CODE_BASE;

    harness::write_bytes(&mut machine.bus, base + 0x0200, b"A:\\TEST.COM\0");
    harness::write_bytes(&mut machine.bus, base + 0x0230, &[0x00, 0x0D]);

    let seg = harness::INJECT_CODE_SEGMENT;
    let seg_lo = (seg & 0xFF) as u8;
    let seg_hi = (seg >> 8) as u8;

    #[rustfmt::skip]
    harness::write_bytes(
        &mut machine.bus,
        base + 0x0210,
        &[
            0x00, 0x00,                             // env_seg = 0 (inherit)
            0x30, 0x02, seg_lo, seg_hi,             // cmd_tail far ptr
            0xFF, 0xFF, seg_lo, seg_hi,             // FCB1 far ptr
            0xFF, 0xFF, seg_lo, seg_hi,             // FCB2 far ptr
            0x00, 0x00, 0x00, 0x00,                 // SS:SP output
            0x00, 0x00, 0x00, 0x00,                 // CS:IP output
        ],
    );

    // The x86 code:
    // 1. Calls INT 21h AX=4B01h to load without executing
    // 2. Reads back the parameter block outputs (SS:SP, CS:IP)
    // 3. Uses the returned CS (= PSP segment for .COM) to read loaded memory:
    //    - PSP+0x00: should be 0xCD (INT opcode)
    //    - PSP+0x01: should be 0x20
    //    - PSP+0x100..0x106: should be the TEST_COM_PROGRAM bytes
    #[rustfmt::skip]
    let code: &[u8] = &[
        // Call AX=4B01h: DS:DX=filename, ES:BX=param block.
        0xBA, 0x00, 0x02,                       // MOV DX, 0200h
        0xBB, 0x10, 0x02,                       // MOV BX, 0210h
        0xB8, 0x01, 0x4B,                       // MOV AX, 4B01h
        0xCD, 0x21,                             // INT 21h
        // Save CF result.
        0x9C,                                   // PUSHF
        0x58,                                   // POP AX
        0xA3, 0x00, 0x01,                       // MOV [0100h], AX  (flags)
        // Read param block: SP, SS, IP, CS.
        0xA1, 0x1E, 0x02,                       // MOV AX, [021Eh]
        0xA3, 0x02, 0x01,                       // MOV [0102h], AX  (SP)
        0xA1, 0x20, 0x02,                       // MOV AX, [0220h]
        0xA3, 0x04, 0x01,                       // MOV [0104h], AX  (SS)
        0xA1, 0x22, 0x02,                       // MOV AX, [0222h]
        0xA3, 0x06, 0x01,                       // MOV [0106h], AX  (IP)
        0xA1, 0x24, 0x02,                       // MOV AX, [0224h]
        0xA3, 0x08, 0x01,                       // MOV [0108h], AX  (CS = PSP seg)
        // Now read from the loaded segment to verify memory contents.
        // Use FS (or ES trick) -- simpler: push DS, load DS from CS result.
        0x1E,                                   // PUSH DS
        0x8E, 0x1E, 0x08, 0x01,                 // MOV DS, [0108h]  (DS = loaded CS)
        // Read PSP: byte at PSP+0x00 and PSP+0x01.
        0xA0, 0x00, 0x00,                       // MOV AL, [0000h]
        0x2E, 0xA2, 0x0A, 0x01,                 // MOV CS:[010Ah], AL  (PSP byte 0)
        0xA0, 0x01, 0x00,                       // MOV AL, [0001h]
        0x2E, 0xA2, 0x0B, 0x01,                 // MOV CS:[010Bh], AL  (PSP byte 1)
        // Read 6 bytes of program code at PSP:0100h.
        0xA0, 0x00, 0x01,                       // MOV AL, [0100h]
        0x2E, 0xA2, 0x10, 0x01,                 // MOV CS:[0110h], AL
        0xA0, 0x01, 0x01,                       // MOV AL, [0101h]
        0x2E, 0xA2, 0x11, 0x01,                 // MOV CS:[0111h], AL
        0xA0, 0x02, 0x01,                       // MOV AL, [0102h]
        0x2E, 0xA2, 0x12, 0x01,                 // MOV CS:[0112h], AL
        0xA0, 0x03, 0x01,                       // MOV AL, [0103h]
        0x2E, 0xA2, 0x13, 0x01,                 // MOV CS:[0113h], AL
        0xA0, 0x04, 0x01,                       // MOV AL, [0104h]
        0x2E, 0xA2, 0x14, 0x01,                 // MOV CS:[0114h], AL
        0xA0, 0x05, 0x01,                       // MOV AL, [0105h]
        0x2E, 0xA2, 0x15, 0x01,                 // MOV CS:[0115h], AL
        0x1F,                                   // POP DS
        0xC3,                                   // RET
    ];
    harness::inject_and_run_via_int28(&mut machine, code, harness::INJECT_BUDGET_DISK_IO);

    let flags = harness::result_word(&machine.bus, 0);
    let sp = harness::result_word(&machine.bus, 2);
    let ss = harness::result_word(&machine.bus, 4);
    let ip = harness::result_word(&machine.bus, 6);
    let cs = harness::result_word(&machine.bus, 8);

    assert_eq!(
        flags & 1,
        0,
        "AX=4B01h should return CF=0, got flags={flags:#06X}"
    );
    assert_eq!(ip, 0x0100, "COM entry IP should be 0100h, got {ip:#06X}");
    assert_eq!(cs, ss, "COM: CS should equal SS (both = PSP segment)");
    assert_ne!(ss, 0, "SS should be non-zero (allocated PSP segment)");
    assert_ne!(sp, 0, "SP should be non-zero");

    // Verify PSP: INT 20h opcode at PSP:0000.
    let psp_byte0 = harness::result_byte(&machine.bus, 0x0A);
    let psp_byte1 = harness::result_byte(&machine.bus, 0x0B);
    assert_eq!(
        psp_byte0, 0xCD,
        "PSP+0 should be INT opcode (0xCD), got {psp_byte0:#04X}"
    );
    assert_eq!(
        psp_byte1, 0x20,
        "PSP+1 should be 0x20, got {psp_byte1:#04X}"
    );

    // Verify program bytes at PSP:0100h match TEST_COM_PROGRAM.
    let loaded_code = harness::read_bytes(&machine.bus, harness::INJECT_RESULT_BASE + 0x10, 6);
    assert_eq!(
        loaded_code,
        harness::TEST_COM_PROGRAM,
        "Loaded COM code at PSP:0100h should match TEST_COM_PROGRAM"
    );
}

#[test]
fn exec_load_only_exe() {
    let exe_code: &[u8] = &[
        0xB4, 0x4C, // MOV AH, 4Ch
        0xB0, 0x00, // MOV AL, 00h
        0xCD, 0x21, // INT 21h
    ];

    let header_paragraphs: u16 = 2;
    let header_size = (header_paragraphs as usize) * 16;
    let stack_size: u16 = 256;
    let image_size = exe_code.len() + stack_size as usize;
    let file_size = header_size + image_size;
    let total_pages = file_size.div_ceil(512) as u16;
    let bytes_last_page = (file_size % 512) as u16;
    let init_sp = (exe_code.len() as u16) + stack_size;

    let mut exe = vec![0u8; file_size];
    exe[0] = 0x4D;
    exe[1] = 0x5A;
    exe[2..4].copy_from_slice(&bytes_last_page.to_le_bytes());
    exe[4..6].copy_from_slice(&total_pages.to_le_bytes());
    exe[6..8].copy_from_slice(&0u16.to_le_bytes());
    exe[8..10].copy_from_slice(&header_paragraphs.to_le_bytes());
    exe[10..12].copy_from_slice(&0u16.to_le_bytes());
    exe[12..14].copy_from_slice(&0xFFFFu16.to_le_bytes());
    exe[14..16].copy_from_slice(&0u16.to_le_bytes()); // init_ss = 0
    exe[16..18].copy_from_slice(&init_sp.to_le_bytes());
    exe[20..22].copy_from_slice(&0u16.to_le_bytes()); // init_ip = 0
    exe[22..24].copy_from_slice(&0u16.to_le_bytes()); // init_cs = 0
    exe[24..26].copy_from_slice(&(header_size as u16).to_le_bytes());
    exe[header_size..header_size + exe_code.len()].copy_from_slice(exe_code);

    let floppy = harness::create_test_floppy_with_program(b"LOADTESTEXE", &exe);
    let mut machine = harness::boot_hle_with_floppy_image(floppy);

    let base = harness::INJECT_CODE_BASE;

    harness::write_bytes(&mut machine.bus, base + 0x0200, b"A:\\LOADTEST.EXE\0");
    harness::write_bytes(&mut machine.bus, base + 0x0230, &[0x00, 0x0D]);

    let seg = harness::INJECT_CODE_SEGMENT;
    let seg_lo = (seg & 0xFF) as u8;
    let seg_hi = (seg >> 8) as u8;

    #[rustfmt::skip]
    harness::write_bytes(
        &mut machine.bus,
        base + 0x0210,
        &[
            0x00, 0x00,                             // env_seg = 0 (inherit)
            0x30, 0x02, seg_lo, seg_hi,             // cmd_tail far ptr
            0xFF, 0xFF, seg_lo, seg_hi,             // FCB1 far ptr
            0xFF, 0xFF, seg_lo, seg_hi,             // FCB2 far ptr
            0x00, 0x00, 0x00, 0x00,                 // SS:SP output
            0x00, 0x00, 0x00, 0x00,                 // CS:IP output
        ],
    );

    // After 4B01h, read param block outputs, then use CS:IP to verify loaded code.
    // For EXE, CS points to load_segment + init_cs (= load_segment for init_cs=0).
    // The PSP segment is at CS - 0x10 (PSP is 0x10 paragraphs before the load segment).
    #[rustfmt::skip]
    let test_code: &[u8] = &[
        0xBA, 0x00, 0x02,                       // MOV DX, 0200h
        0xBB, 0x10, 0x02,                       // MOV BX, 0210h
        0xB8, 0x01, 0x4B,                       // MOV AX, 4B01h
        0xCD, 0x21,                             // INT 21h
        0x9C,                                   // PUSHF
        0x58,                                   // POP AX
        0xA3, 0x00, 0x01,                       // MOV [0100h], AX  (flags)
        0xA1, 0x1E, 0x02,                       // MOV AX, [021Eh]
        0xA3, 0x02, 0x01,                       // MOV [0102h], AX  (SP)
        0xA1, 0x20, 0x02,                       // MOV AX, [0220h]
        0xA3, 0x04, 0x01,                       // MOV [0104h], AX  (SS)
        0xA1, 0x22, 0x02,                       // MOV AX, [0222h]
        0xA3, 0x06, 0x01,                       // MOV [0106h], AX  (IP)
        0xA1, 0x24, 0x02,                       // MOV AX, [0224h]
        0xA3, 0x08, 0x01,                       // MOV [0108h], AX  (CS)
        // Read PSP (CS - 0x10 for EXE).
        0xA1, 0x08, 0x01,                       // MOV AX, [0108h]  (CS)
        0x2D, 0x10, 0x00,                       // SUB AX, 0010h    (PSP segment)
        0x1E,                                   // PUSH DS
        0x8E, 0xD8,                             // MOV DS, AX
        0xA0, 0x00, 0x00,                       // MOV AL, [0000h]
        0x2E, 0xA2, 0x0A, 0x01,                 // MOV CS:[010Ah], AL  (PSP byte 0)
        0xA0, 0x01, 0x00,                       // MOV AL, [0001h]
        0x2E, 0xA2, 0x0B, 0x01,                 // MOV CS:[010Bh], AL  (PSP byte 1)
        0x1F,                                   // POP DS
        // Read code at CS:IP (CS = loaded CS, IP = 0).
        0x1E,                                   // PUSH DS
        0x8E, 0x1E, 0x08, 0x01,                 // MOV DS, [0108h]  (DS = loaded CS)
        0x8B, 0x36, 0x06, 0x01,                 // MOV SI, [0106h]  (SI = loaded IP)
        // Copy 6 bytes from DS:SI to CS:0110h.
        0x8A, 0x04,                             // MOV AL, [SI]
        0x2E, 0xA2, 0x10, 0x01,                 // MOV CS:[0110h], AL
        0x8A, 0x44, 0x01,                       // MOV AL, [SI+1]
        0x2E, 0xA2, 0x11, 0x01,                 // MOV CS:[0111h], AL
        0x8A, 0x44, 0x02,                       // MOV AL, [SI+2]
        0x2E, 0xA2, 0x12, 0x01,                 // MOV CS:[0112h], AL
        0x8A, 0x44, 0x03,                       // MOV AL, [SI+3]
        0x2E, 0xA2, 0x13, 0x01,                 // MOV CS:[0113h], AL
        0x8A, 0x44, 0x04,                       // MOV AL, [SI+4]
        0x2E, 0xA2, 0x14, 0x01,                 // MOV CS:[0114h], AL
        0x8A, 0x44, 0x05,                       // MOV AL, [SI+5]
        0x2E, 0xA2, 0x15, 0x01,                 // MOV CS:[0115h], AL
        0x1F,                                   // POP DS
        0xC3,                                   // RET
    ];
    harness::inject_and_run_via_int28(&mut machine, test_code, harness::INJECT_BUDGET_DISK_IO);

    let flags = harness::result_word(&machine.bus, 0);
    let sp = harness::result_word(&machine.bus, 2);
    let ss = harness::result_word(&machine.bus, 4);
    let ip = harness::result_word(&machine.bus, 6);
    let cs = harness::result_word(&machine.bus, 8);

    assert_eq!(
        flags & 1,
        0,
        "AX=4B01h for EXE should return CF=0, got flags={flags:#06X}"
    );
    assert_eq!(ip, 0x0000, "EXE entry IP should be 0000h, got {ip:#06X}");
    assert_eq!(
        sp, init_sp,
        "EXE entry SP should be {init_sp:#06X}, got {sp:#06X}"
    );
    assert_ne!(ss, 0, "SS should be non-zero");
    assert_ne!(cs, 0, "CS should be non-zero");

    // Verify PSP at CS-0x10: INT 20h opcode.
    let psp_byte0 = harness::result_byte(&machine.bus, 0x0A);
    let psp_byte1 = harness::result_byte(&machine.bus, 0x0B);
    assert_eq!(
        psp_byte0, 0xCD,
        "PSP+0 should be INT opcode (0xCD), got {psp_byte0:#04X}"
    );
    assert_eq!(
        psp_byte1, 0x20,
        "PSP+1 should be 0x20, got {psp_byte1:#04X}"
    );

    // Verify program code at CS:IP matches what we put in the EXE.
    let loaded_code = harness::read_bytes(&machine.bus, harness::INJECT_RESULT_BASE + 0x10, 6);
    assert_eq!(
        loaded_code, exe_code,
        "Loaded EXE code at CS:IP should match the original code bytes"
    );
}

#[test]
fn dbcs_get_table_subfunction_00() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0x06,                                   // PUSH ES
        0x0E,                                   // PUSH CS
        0x07,                                   // POP ES  (ES = code segment)
        0xB8, 0x00, 0x63,                       // MOV AX, 6300h
        0xCD, 0x21,                             // INT 21h
        0x26, 0x89, 0x36, 0x00, 0x01,           // MOV ES:[0100h], SI
        0x26, 0x8C, 0x1E, 0x02, 0x01,           // MOV ES:[0102h], DS
        0x07,                                   // POP ES
        0xFA, 0xF4,                             // CLI; HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let si = harness::result_word(&machine.bus, 0);
    let ds = harness::result_word(&machine.bus, 2);
    assert!(
        ds != 0 || si != 0,
        "AX=6300h: DS:SI should be a non-null DBCS table pointer"
    );
}

#[test]
fn dbcs_set_interim_console_flag() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        // Set interim console flag to 1.
        0xB2, 0x01,                             // MOV DL, 01h
        0xB8, 0x01, 0x63,                       // MOV AX, 6301h
        0xCD, 0x21,                             // INT 21h
        // Get interim console flag.
        0xB8, 0x02, 0x63,                       // MOV AX, 6302h
        0xCD, 0x21,                             // INT 21h
        0x88, 0x16, 0x00, 0x01,                 // MOV [0100h], DL
        0xFA, 0xF4,                             // CLI; HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let dl = harness::result_byte(&machine.bus, 0);
    assert_eq!(dl, 0x01, "Interim console flag should be 01h after set");
}

#[test]
fn dbcs_interim_console_flag_roundtrip() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        // Set flag to 0x00, read back.
        0xB2, 0x00,                             // MOV DL, 00h
        0xB8, 0x01, 0x63,                       // MOV AX, 6301h
        0xCD, 0x21,                             // INT 21h
        0xB8, 0x02, 0x63,                       // MOV AX, 6302h
        0xCD, 0x21,                             // INT 21h
        0x88, 0x16, 0x00, 0x01,                 // MOV [0100h], DL
        // Set flag to 0x01, read back.
        0xB2, 0x01,                             // MOV DL, 01h
        0xB8, 0x01, 0x63,                       // MOV AX, 6301h
        0xCD, 0x21,                             // INT 21h
        0xB8, 0x02, 0x63,                       // MOV AX, 6302h
        0xCD, 0x21,                             // INT 21h
        0x88, 0x16, 0x01, 0x01,                 // MOV [0101h], DL
        // Set flag to 0xFF, read back.
        0xB2, 0xFF,                             // MOV DL, FFh
        0xB8, 0x01, 0x63,                       // MOV AX, 6301h
        0xCD, 0x21,                             // INT 21h
        0xB8, 0x02, 0x63,                       // MOV AX, 6302h
        0xCD, 0x21,                             // INT 21h
        0x88, 0x16, 0x02, 0x01,                 // MOV [0102h], DL
        0xFA, 0xF4,                             // CLI; HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let val_00 = harness::result_byte(&machine.bus, 0);
    let val_01 = harness::result_byte(&machine.bus, 1);
    let val_ff = harness::result_byte(&machine.bus, 2);
    assert_eq!(val_00, 0x00, "Flag should read back 00h");
    assert_eq!(val_01, 0x01, "Flag should read back 01h");
    assert_eq!(val_ff, 0xFF, "Flag should read back FFh");
}

#[test]
fn network_functions_return_error() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        // AH=5Eh: Network set machine name.
        0xB8, 0x00, 0x5E,                       // MOV AX, 5E00h
        0xCD, 0x21,                             // INT 21h
        0xA3, 0x00, 0x01,                       // MOV [0100h], AX  (error code)
        0x9C,                                   // PUSHF
        0x58,                                   // POP AX
        0xA3, 0x02, 0x01,                       // MOV [0102h], AX  (flags)
        // AH=5Fh: Network redirection.
        0xB8, 0x00, 0x5F,                       // MOV AX, 5F00h
        0xCD, 0x21,                             // INT 21h
        0xA3, 0x04, 0x01,                       // MOV [0104h], AX  (error code)
        0x9C,                                   // PUSHF
        0x58,                                   // POP AX
        0xA3, 0x06, 0x01,                       // MOV [0106h], AX  (flags)
        0xFA, 0xF4,                             // CLI; HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let ax_5e = harness::result_word(&machine.bus, 0);
    let flags_5e = harness::result_word(&machine.bus, 2);
    let ax_5f = harness::result_word(&machine.bus, 4);
    let flags_5f = harness::result_word(&machine.bus, 6);
    assert_eq!(ax_5e, 0x0001, "AH=5Eh should return AX=0001h");
    assert_ne!(flags_5e & 1, 0, "AH=5Eh should return CF=1");
    assert_eq!(ax_5f, 0x0001, "AH=5Fh should return AX=0001h");
    assert_ne!(flags_5f & 1, 0, "AH=5Fh should return CF=1");
}

#[test]
fn server_call_5d0a_set_error_noop() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, 0x0A, 0x5D,                       // MOV AX, 5D0Ah
        0xCD, 0x21,                             // INT 21h
        0x9C,                                   // PUSHF
        0x58,                                   // POP AX
        0xA3, 0x00, 0x01,                       // MOV [0100h], AX  (flags)
        0xFA, 0xF4,                             // CLI; HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let flags = harness::result_word(&machine.bus, 0);
    assert_eq!(flags & 1, 0, "AX=5D0Ah should return CF=0 (no-op success)");
}

#[test]
fn server_call_5d06_returns_error() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, 0x06, 0x5D,                       // MOV AX, 5D06h
        0xCD, 0x21,                             // INT 21h
        0xA3, 0x00, 0x01,                       // MOV [0100h], AX  (error code)
        0x9C,                                   // PUSHF
        0x58,                                   // POP AX
        0xA3, 0x02, 0x01,                       // MOV [0102h], AX  (flags)
        0xFA, 0xF4,                             // CLI; HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let ax = harness::result_word(&machine.bus, 0);
    let flags = harness::result_word(&machine.bus, 2);
    assert_eq!(ax, 0x0001, "AX=5D06h should return AX=0001h");
    assert_ne!(flags & 1, 0, "AX=5D06h should return CF=1");
}

#[test]
fn create_dpb_from_bpb_stub() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x53,                             // MOV AH, 53h
        0xCD, 0x21,                             // INT 21h
        0xC6, 0x06, 0x00, 0x01, 0x01,           // MOV BYTE [0100h], 01h
        0xFA, 0xF4,                             // CLI; HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let marker = harness::result_byte(&machine.bus, 0);
    assert_eq!(marker, 0x01, "AH=53h should return without crashing");
}
