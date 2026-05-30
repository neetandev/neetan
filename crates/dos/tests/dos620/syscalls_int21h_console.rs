use common::JisChar;

use crate::harness::{self, *};

#[test]
fn display_character_02h() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x02,                         // MOV AH, 02h
        0xB2, 0x58,                         // MOV DL, 58h ('X')
        0xCD, 0x21,                         // INT 21h
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);

    assert!(
        harness::find_char_in_text_vram(&machine.bus, 0x0058),
        "Text VRAM should contain character 'X' (0x0058) after INT 21h/02h"
    );
}

#[test]
fn display_string_09h() {
    let mut machine = harness::boot_hle();
    // Place '$'-terminated string at 0x9000:0200.
    let string = b"TEST$";
    harness::write_bytes(&mut machine.bus, harness::INJECT_CODE_BASE + 0x200, string);

    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x09,                         // MOV AH, 09h
        0xBA, 0x00, 0x02,                   // MOV DX, 0200h
        0xCD, 0x21,                         // INT 21h
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let chars: Vec<u16> = b"TEST".iter().map(|&c| c as u16).collect();
    assert!(
        harness::find_string_in_text_vram(&machine.bus, &chars),
        "Text VRAM should contain consecutive 'T','E','S','T' after INT 21h/09h"
    );
}

#[test]
fn display_string_09h_shift_jis() {
    let mut machine = boot_hle();
    let string = b"\x82\xA0\x82\xA2$";
    write_bytes(&mut machine.bus, INJECT_CODE_BASE + 0x200, string);

    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x09,                         // MOV AH, 09h
        0xBA, 0x00, 0x02,                   // MOV DX, 0200h
        0xCD, 0x21,                         // INT 21h
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    inject_and_run(&mut machine, code);

    let chars = [JisChar::from_u16(0x2422), JisChar::from_u16(0x2424)];
    assert!(
        find_jis_string_in_text_vram(&machine.bus, &chars),
        "INT 21h/09h should display Shift-JIS strings as full-width glyphs"
    );
}

#[test]
fn direct_console_output_06h() {
    let mut machine = harness::boot_hle();
    // Use a rare character to avoid false matches from the DOS prompt.
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x06,                         // MOV AH, 06h
        0xB2, 0x7E,                         // MOV DL, 7Eh ('~')
        0xCD, 0x21,                         // INT 21h
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);

    assert!(
        harness::find_char_in_text_vram(&machine.bus, 0x007E),
        "Text VRAM should contain character '~' (0x007E) after INT 21h/06h"
    );
}

#[test]
fn fast_console_output_int29h() {
    let mut machine = harness::boot_hle();
    // Use a rare character to avoid false matches.
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB0, 0x7C,                         // MOV AL, 7Ch ('|')
        0xCD, 0x29,                         // INT 29h
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);

    assert!(
        harness::find_char_in_text_vram(&machine.bus, 0x007C),
        "Text VRAM should contain character '|' (0x007C) after INT 29h"
    );
}

#[test]
fn direct_console_input_06h_key_available() {
    let mut machine = harness::boot_hle();
    type_string(&mut machine.bus, b"Q");

    const RES_LO: u8 = (INJECT_RESULT_OFFSET & 0xFF) as u8;
    const RES_HI: u8 = (INJECT_RESULT_OFFSET >> 8) as u8;
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x06,                         // MOV AH, 06h
        0xB2, 0xFF,                         // MOV DL, FFh (input request)
        0xCD, 0x21,                         // INT 21h
        0x89, 0x06, RES_LO, RES_HI,         // MOV [result+0], AX
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0x89, 0x06, RES_LO + 2, RES_HI,     // MOV [result+2], AX (flags)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    inject_and_run(&mut machine, code);

    let ax = result_word(&machine.bus, 0);
    assert_eq!(ax & 0xFF, b'Q' as u16, "AL should be 'Q'");

    let flags = result_word(&machine.bus, 2);
    assert_eq!(flags & 0x0040, 0, "ZF should be clear when key available");
}

#[test]
fn direct_console_input_06h_no_key() {
    let mut machine = harness::boot_hle();

    const RES_LO: u8 = (INJECT_RESULT_OFFSET & 0xFF) as u8;
    const RES_HI: u8 = (INJECT_RESULT_OFFSET >> 8) as u8;
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x06,                         // MOV AH, 06h
        0xB2, 0xFF,                         // MOV DL, FFh (input request)
        0xCD, 0x21,                         // INT 21h
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0x89, 0x06, RES_LO, RES_HI,         // MOV [result+0], AX (flags)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    inject_and_run(&mut machine, code);

    let flags = result_word(&machine.bus, 0);
    assert_ne!(flags & 0x0040, 0, "ZF should be set when no key available");
}

#[test]
fn direct_char_input_07h() {
    let mut machine = harness::boot_hle();
    type_string(&mut machine.bus, b"Z");

    const RES_LO: u8 = (INJECT_RESULT_OFFSET & 0xFF) as u8;
    const RES_HI: u8 = (INJECT_RESULT_OFFSET >> 8) as u8;
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x07,                         // MOV AH, 07h
        0xCD, 0x21,                         // INT 21h
        0x89, 0x06, RES_LO, RES_HI,         // MOV [result+0], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    inject_and_run(&mut machine, code);

    let ax = result_word(&machine.bus, 0);
    assert_eq!(ax & 0xFF, b'Z' as u16, "AL should be 'Z'");
}

#[test]
fn char_input_no_echo_08h() {
    let mut machine = harness::boot_hle();
    type_string(&mut machine.bus, b"W");

    const RES_LO: u8 = (INJECT_RESULT_OFFSET & 0xFF) as u8;
    const RES_HI: u8 = (INJECT_RESULT_OFFSET >> 8) as u8;
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x08,                         // MOV AH, 08h
        0xCD, 0x21,                         // INT 21h
        0x89, 0x06, RES_LO, RES_HI,         // MOV [result+0], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    inject_and_run(&mut machine, code);

    let ax = result_word(&machine.bus, 0);
    assert_eq!(ax & 0xFF, b'W' as u16, "AL should be 'W'");
}

#[test]
fn keyboard_input_with_echo_01h() {
    let mut machine = harness::boot_hle();
    type_string(&mut machine.bus, b"#");

    const RES_LO: u8 = (INJECT_RESULT_OFFSET & 0xFF) as u8;
    const RES_HI: u8 = (INJECT_RESULT_OFFSET >> 8) as u8;
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x01,                         // MOV AH, 01h
        0xCD, 0x21,                         // INT 21h
        0x89, 0x06, RES_LO, RES_HI,         // MOV [result+0], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    inject_and_run(&mut machine, code);

    let ax = result_word(&machine.bus, 0);
    assert_eq!(ax & 0xFF, b'#' as u16, "AL should be '#'");

    assert!(
        harness::find_char_in_text_vram(&machine.bus, 0x0023),
        "Character '#' should be echoed to VRAM by AH=01h"
    );
}

#[test]
fn check_keyboard_status_0bh_available() {
    let mut machine = harness::boot_hle();
    type_string(&mut machine.bus, b"A");

    const RES_LO: u8 = (INJECT_RESULT_OFFSET & 0xFF) as u8;
    const RES_HI: u8 = (INJECT_RESULT_OFFSET >> 8) as u8;
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x0B,                         // MOV AH, 0Bh
        0xCD, 0x21,                         // INT 21h
        0x89, 0x06, RES_LO, RES_HI,         // MOV [result+0], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    inject_and_run(&mut machine, code);

    let ax = result_word(&machine.bus, 0);
    assert_eq!(ax & 0xFF, 0xFF, "AL should be FFh when key available");
}

#[test]
fn check_keyboard_status_0bh_empty() {
    let mut machine = harness::boot_hle();

    const RES_LO: u8 = (INJECT_RESULT_OFFSET & 0xFF) as u8;
    const RES_HI: u8 = (INJECT_RESULT_OFFSET >> 8) as u8;
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x0B,                         // MOV AH, 0Bh
        0xCD, 0x21,                         // INT 21h
        0x89, 0x06, RES_LO, RES_HI,         // MOV [result+0], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    inject_and_run(&mut machine, code);

    let ax = result_word(&machine.bus, 0);
    assert_eq!(ax & 0xFF, 0x00, "AL should be 00h when no key available");
}

#[test]
fn buffered_input_0ah() {
    let mut machine = harness::boot_hle();

    let base = INJECT_CODE_BASE;

    // Set up input buffer at +0x0200: byte[0]=10 (max), byte[1]=0 (count, output)
    write_bytes(&mut machine.bus, base + 0x0200, &[10, 0]);

    // Inject "HI\r" into keyboard buffer
    type_string(&mut machine.bus, b"HI\r");

    const RES_LO: u8 = (INJECT_RESULT_OFFSET & 0xFF) as u8;
    const RES_HI: u8 = (INJECT_RESULT_OFFSET >> 8) as u8;
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xBA, 0x00, 0x02,                   // MOV DX, 0200h (buffer)
        0xB4, 0x0A,                         // MOV AH, 0Ah
        0xCD, 0x21,                         // INT 21h
        0xC6, 0x06, RES_LO, RES_HI, 0x01,   // MOV BYTE [result+0], 01h (done marker)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    inject_and_run_with_budget(&mut machine, code, INJECT_BUDGET_DISK_IO);

    assert_eq!(
        result_byte(&machine.bus, 0),
        0x01,
        "code should have completed"
    );

    let actual_count = machine.bus.read_byte_direct(base + 0x0201);
    assert_eq!(actual_count, 2, "byte[1] should be 2 (chars read)");

    let ch0 = machine.bus.read_byte_direct(base + 0x0202);
    let ch1 = machine.bus.read_byte_direct(base + 0x0203);
    let ch2 = machine.bus.read_byte_direct(base + 0x0204);
    assert_eq!(ch0, b'H', "byte[2] should be 'H'");
    assert_eq!(ch1, b'I', "byte[3] should be 'I'");
    assert_eq!(ch2, 0x0D, "byte[4] should be CR");
}

#[test]
fn flush_and_invoke_0ch() {
    let mut machine = harness::boot_hle();

    // Inject a key that should be flushed
    type_string(&mut machine.bus, b"X");

    const RES_LO: u8 = (INJECT_RESULT_OFFSET & 0xFF) as u8;
    const RES_HI: u8 = (INJECT_RESULT_OFFSET >> 8) as u8;
    #[rustfmt::skip]
    let code: &[u8] = &[
        // AH=0Ch, AL=06h (flush then direct console input DL=FFh)
        0xB8, 0x06, 0x0C,                   // MOV AX, 0C06h
        0xB2, 0xFF,                         // MOV DL, FFh
        0xCD, 0x21,                         // INT 21h
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0x89, 0x06, RES_LO, RES_HI,         // MOV [result+0], AX (flags)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    inject_and_run(&mut machine, code);

    let flags = result_word(&machine.bus, 0);
    assert_ne!(flags & 0x0040, 0, "ZF should be set (no key after flush)");
}

#[test]
fn flush_and_invoke_0ch_blocks_without_repeated_flush() {
    let mut machine = harness::boot_hle();

    const RES_LO: u8 = (INJECT_RESULT_OFFSET & 0xFF) as u8;
    const RES_HI: u8 = (INJECT_RESULT_OFFSET >> 8) as u8;
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, 0x07, 0x0C,                   // MOV AX, 0C07h
        0xCD, 0x21,                         // INT 21h
        0x89, 0x06, RES_LO, RES_HI,         // MOV [result+0], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];

    inject_and_run_with_budget(&mut machine, code, 1_000_000);
    type_string(&mut machine.bus, b"2");
    machine.run_for(INJECT_BUDGET_DISK_IO);

    let al = result_word(&machine.bus, 0) & 0xFF;
    assert_eq!(
        al, b'2' as u16,
        "AH=0Ch/AL=07h should read the key typed while blocked"
    );
}

/// Extended keys (arrows) are expanded into function key map sequences by the
/// HLE DOS, matching NEC DOS IO.SYS behavior. Arrow UP (hardware scan 0x3A)
/// produces 0x0B (VT control character). This is a single byte, so only one
/// INT 21h AH=07h call is needed.
///
/// Default values verified against real MS-DOS 6.20 via oracle test
/// (machine crate `fnkey_oracle::read_dos620_default_fnkey_map`).
#[test]
fn extended_key_07h_arrow_up_returns_vt() {
    let mut machine = harness::boot_hle();

    // Inject arrow-up key using real PC-98 hardware scan code (0x3A).
    const PC98_SCAN_UP: u8 = 0x3A;
    machine.bus.push_keyboard_scancode(PC98_SCAN_UP);
    machine.bus.push_keyboard_scancode(PC98_SCAN_UP | 0x80);

    const RES_LO: u8 = (INJECT_RESULT_OFFSET & 0xFF) as u8;
    const RES_HI: u8 = (INJECT_RESULT_OFFSET >> 8) as u8;
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x07,                         // MOV AH, 07h
        0xCD, 0x21,                         // INT 21h
        0x89, 0x06, RES_LO, RES_HI,         // MOV [result+0], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    inject_and_run_with_budget(&mut machine, code, INJECT_BUDGET_DISK_IO);

    let al = result_word(&machine.bus, 0) & 0xFF;
    assert_eq!(al, 0x0B, "arrow-up should produce 0x0B (VT), got {al:#04X}");
}
