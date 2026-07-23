//! INT 10h text services: teletype output, scroll windows, character and
//! attribute access, display pages, cursor shape and write string.

use common::Bus;

use super::{
    RESULT, boot_and_run, create_machine_dx50,
    harness::{framebuffer_hash, render_frame, text_cell},
    inject_and_run, read_ram_u8, read_ram_u16, write_bytes,
};

/// Cycle budget for one injected INT 10h program.
const BUDGET: u64 = 8_000_000;

/// BIOS data area: active page regen start offset (word).
const BDA_VIDEO_PAGE_START: u32 = 0x44E;
/// BIOS data area: cursor position of page zero (word).
const BDA_CURSOR_PAGE_0: u32 = 0x450;
/// BIOS data area: cursor shape (word).
const BDA_CURSOR_SHAPE: u32 = 0x460;
/// BIOS data area: active display page.
const BDA_ACTIVE_PAGE: u32 = 0x462;
/// Mode 03h display page size in bytes.
const PAGE_SIZE: u16 = 0x1000;
/// Mode 03h text columns.
const COLUMNS: u16 = 80;

/// Builds `MOV AX, mode; INT 10h; HLT`.
fn set_mode_code(mode: u8) -> [u8; 6] {
    [0xB8, mode, 0x00, 0xCD, 0x10, 0xF4]
}

/// Mode 03h, then teletype 'A', 'B', CR, LF, 'C', BS, 'D'.
#[rustfmt::skip]
const TELETYPE_CONTROL_CODE: &[u8] = &[
    0xB8, 0x03, 0x00,       // MOV AX, 0x0003
    0xCD, 0x10,             // INT 10h
    0xBB, 0x00, 0x00,       // MOV BX, 0x0000
    0xB8, 0x41, 0x0E,       // MOV AX, 0x0E41 'A'
    0xCD, 0x10,             // INT 10h
    0xB8, 0x42, 0x0E,       // MOV AX, 0x0E42 'B'
    0xCD, 0x10,             // INT 10h
    0xB8, 0x0D, 0x0E,       // MOV AX, 0x0E0D CR
    0xCD, 0x10,             // INT 10h
    0xB8, 0x0A, 0x0E,       // MOV AX, 0x0E0A LF
    0xCD, 0x10,             // INT 10h
    0xB8, 0x43, 0x0E,       // MOV AX, 0x0E43 'C'
    0xCD, 0x10,             // INT 10h
    0xB8, 0x08, 0x0E,       // MOV AX, 0x0E08 BS
    0xCD, 0x10,             // INT 10h
    0xB8, 0x44, 0x0E,       // MOV AX, 0x0E44 'D'
    0xCD, 0x10,             // INT 10h
    0xF4,                   // HLT
];

#[test]
fn teletype_advances_and_handles_control_codes() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, TELETYPE_CONTROL_CODE, &[], BUDGET);
    assert_eq!(
        text_cell(&machine, 0, PAGE_SIZE, COLUMNS, 0, 0),
        (b'A', 0x07)
    );
    assert_eq!(
        text_cell(&machine, 0, PAGE_SIZE, COLUMNS, 0, 1),
        (b'B', 0x07)
    );
    // BS moved the cursor back over 'C', so 'D' overwrote it.
    assert_eq!(
        text_cell(&machine, 0, PAGE_SIZE, COLUMNS, 1, 0),
        (b'D', 0x07)
    );
    assert_eq!(read_ram_u16(&machine, BDA_CURSOR_PAGE_0), 0x0101);
}

/// Mode 03h, cursor to (24,79), teletype 'X' (wraps and scrolls), then 'Y'.
#[rustfmt::skip]
const TELETYPE_BOTTOM_SCROLL_CODE: &[u8] = &[
    0xB8, 0x03, 0x00,       // MOV AX, 0x0003
    0xCD, 0x10,             // INT 10h
    0xB4, 0x02,             // MOV AH, 0x02
    0xBB, 0x00, 0x00,       // MOV BX, 0x0000
    0xBA, 0x4F, 0x18,       // MOV DX, 0x184F
    0xCD, 0x10,             // INT 10h
    0xB8, 0x58, 0x0E,       // MOV AX, 0x0E58 'X'
    0xCD, 0x10,             // INT 10h
    0xB8, 0x59, 0x0E,       // MOV AX, 0x0E59 'Y'
    0xCD, 0x10,             // INT 10h
    0xF4,                   // HLT
];

#[test]
fn teletype_scrolls_the_bottom_line() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, TELETYPE_BOTTOM_SCROLL_CODE, &[], BUDGET);
    // 'X' was written at (24,79), then the wrap scrolled everything up.
    assert_eq!(
        text_cell(&machine, 0, PAGE_SIZE, COLUMNS, 23, 79),
        (b'X', 0x07)
    );
    assert_eq!(
        text_cell(&machine, 0, PAGE_SIZE, COLUMNS, 24, 0),
        (b'Y', 0x07)
    );
    // The freed bottom line was blanked with the attribute under the cursor.
    assert_eq!(
        text_cell(&machine, 0, PAGE_SIZE, COLUMNS, 24, 1),
        (0x20, 0x07)
    );
    assert_eq!(read_ram_u16(&machine, BDA_CURSOR_PAGE_0), 0x1801);
}

/// AH=06h AL=01h: scrolls rows 0-4 up one line, fill attribute 0x20.
#[rustfmt::skip]
const SCROLL_UP_CODE: &[u8] = &[
    0xB8, 0x01, 0x06,       // MOV AX, 0x0601
    0xB7, 0x20,             // MOV BH, 0x20
    0xB9, 0x00, 0x00,       // MOV CX, 0x0000
    0xBA, 0x4F, 0x04,       // MOV DX, 0x044F
    0xCD, 0x10,             // INT 10h
    0xF4,                   // HLT
];

/// AH=07h AL=01h: scrolls rows 0-4 down one line, fill attribute 0x30.
#[rustfmt::skip]
const SCROLL_DOWN_CODE: &[u8] = &[
    0xB8, 0x01, 0x07,       // MOV AX, 0x0701
    0xB7, 0x30,             // MOV BH, 0x30
    0xB9, 0x00, 0x00,       // MOV CX, 0x0000
    0xBA, 0x4F, 0x04,       // MOV DX, 0x044F
    0xCD, 0x10,             // INT 10h
    0xF4,                   // HLT
];

/// AH=06h AL=00h: clears rows 0-4 with attribute 0x07.
#[rustfmt::skip]
const SCROLL_CLEAR_CODE: &[u8] = &[
    0xB8, 0x00, 0x06,       // MOV AX, 0x0600
    0xB7, 0x07,             // MOV BH, 0x07
    0xB9, 0x00, 0x00,       // MOV CX, 0x0000
    0xBA, 0x4F, 0x04,       // MOV DX, 0x044F
    0xCD, 0x10,             // INT 10h
    0xF4,                   // HLT
];

#[test]
fn scroll_window_up_down_and_clear() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, &set_mode_code(0x03), &[], BUDGET);
    for row in 0..6u8 {
        let address = 0xB8000 + u32::from(row) * u32::from(COLUMNS) * 2;
        machine.bus.write_byte(address, b'0' + row);
        machine.bus.write_byte(address + 1, 0x10 + row);
    }

    inject_and_run(&mut machine, SCROLL_UP_CODE, &[], BUDGET);
    assert_eq!(
        text_cell(&machine, 0, PAGE_SIZE, COLUMNS, 0, 0),
        (b'1', 0x11)
    );
    assert_eq!(
        text_cell(&machine, 0, PAGE_SIZE, COLUMNS, 3, 0),
        (b'4', 0x14)
    );
    assert_eq!(
        text_cell(&machine, 0, PAGE_SIZE, COLUMNS, 4, 0),
        (0x20, 0x20)
    );
    // Row 5 sits outside the window and stays untouched.
    assert_eq!(
        text_cell(&machine, 0, PAGE_SIZE, COLUMNS, 5, 0),
        (b'5', 0x15)
    );

    inject_and_run(&mut machine, SCROLL_DOWN_CODE, &[], BUDGET);
    assert_eq!(
        text_cell(&machine, 0, PAGE_SIZE, COLUMNS, 0, 0),
        (0x20, 0x30)
    );
    assert_eq!(
        text_cell(&machine, 0, PAGE_SIZE, COLUMNS, 1, 0),
        (b'1', 0x11)
    );
    assert_eq!(
        text_cell(&machine, 0, PAGE_SIZE, COLUMNS, 4, 0),
        (b'4', 0x14)
    );
    assert_eq!(
        text_cell(&machine, 0, PAGE_SIZE, COLUMNS, 5, 0),
        (b'5', 0x15)
    );

    inject_and_run(&mut machine, SCROLL_CLEAR_CODE, &[], BUDGET);
    for row in 0..5u8 {
        assert_eq!(
            text_cell(&machine, 0, PAGE_SIZE, COLUMNS, row, 0),
            (0x20, 0x07),
            "row {row}"
        );
    }
    assert_eq!(
        text_cell(&machine, 0, PAGE_SIZE, COLUMNS, 5, 0),
        (b'5', 0x15)
    );
}

/// Mode 03h, cursor (1,2), AH=09h '*' three times, AH=0Ah '+', AH=08h read.
#[rustfmt::skip]
const WRITE_CHAR_ATTR_CODE: &[u8] = &[
    0xB8, 0x03, 0x00,       // MOV AX, 0x0003
    0xCD, 0x10,             // INT 10h
    0xB4, 0x02,             // MOV AH, 0x02
    0xBB, 0x00, 0x00,       // MOV BX, 0x0000
    0xBA, 0x02, 0x01,       // MOV DX, 0x0102
    0xCD, 0x10,             // INT 10h
    0xB8, 0x2A, 0x09,       // MOV AX, 0x092A '*'
    0xBB, 0x4E, 0x00,       // MOV BX, 0x004E
    0xB9, 0x03, 0x00,       // MOV CX, 0x0003
    0xCD, 0x10,             // INT 10h
    0xB8, 0x2B, 0x0A,       // MOV AX, 0x0A2B '+'
    0xB9, 0x01, 0x00,       // MOV CX, 0x0001
    0xCD, 0x10,             // INT 10h
    0xB4, 0x08,             // MOV AH, 0x08
    0xB7, 0x00,             // MOV BH, 0x00
    0xCD, 0x10,             // INT 10h
    0xA3, 0x00, 0x06,       // MOV [0x0600], AX
    0xF4,                   // HLT
];

#[test]
fn write_char_attr_count_and_read_back() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, WRITE_CHAR_ATTR_CODE, &[], BUDGET);
    // AH=0Ah replaced the character at the cursor but kept the attribute.
    assert_eq!(
        text_cell(&machine, 0, PAGE_SIZE, COLUMNS, 1, 2),
        (b'+', 0x4E)
    );
    assert_eq!(
        text_cell(&machine, 0, PAGE_SIZE, COLUMNS, 1, 3),
        (b'*', 0x4E)
    );
    assert_eq!(
        text_cell(&machine, 0, PAGE_SIZE, COLUMNS, 1, 4),
        (b'*', 0x4E)
    );
    assert_eq!(read_ram_u16(&machine, RESULT), 0x4E2B);
    // Neither service moved the cursor.
    assert_eq!(read_ram_u16(&machine, BDA_CURSOR_PAGE_0), 0x0102);
}

/// Mode 03h, page 1 active, cursor (3,5) on page 1, teletype 'Q', read back
/// the page 1 cursor.
#[rustfmt::skip]
const PAGE_SWITCH_CODE: &[u8] = &[
    0xB8, 0x03, 0x00,       // MOV AX, 0x0003
    0xCD, 0x10,             // INT 10h
    0xB8, 0x01, 0x05,       // MOV AX, 0x0501
    0xCD, 0x10,             // INT 10h
    0xB4, 0x02,             // MOV AH, 0x02
    0xB7, 0x01,             // MOV BH, 0x01
    0xBA, 0x05, 0x03,       // MOV DX, 0x0305
    0xCD, 0x10,             // INT 10h
    0xB8, 0x51, 0x0E,       // MOV AX, 0x0E51 'Q'
    0xBB, 0x00, 0x01,       // MOV BX, 0x0100
    0xCD, 0x10,             // INT 10h
    0xB4, 0x03,             // MOV AH, 0x03
    0xB7, 0x01,             // MOV BH, 0x01
    0xCD, 0x10,             // INT 10h
    0x89, 0x16, 0x00, 0x06, // MOV [0x0600], DX
    0x89, 0x0E, 0x02, 0x06, // MOV [0x0602], CX
    0xF4,                   // HLT
];

#[test]
fn active_page_switch_and_per_page_cursor() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, PAGE_SWITCH_CODE, &[], BUDGET);
    assert_eq!(read_ram_u8(&machine, BDA_ACTIVE_PAGE), 1);
    assert_eq!(read_ram_u16(&machine, BDA_VIDEO_PAGE_START), 0x1000);
    // The CRTC start address counts character cells, so page 1 starts at
    // 0x1000 / 2 cells.
    assert_eq!(machine.bus.vga().crtc[0x0C], 0x08);
    assert_eq!(machine.bus.vga().crtc[0x0D], 0x00);
    assert_eq!(
        text_cell(&machine, 1, PAGE_SIZE, COLUMNS, 3, 5),
        (b'Q', 0x07)
    );
    // The CRTC cursor location tracks the page 1 cursor after the teletype.
    let cursor_cell = 0x1000 / 2 + 3 * u32::from(COLUMNS) + 6;
    assert_eq!(u32::from(machine.bus.vga().crtc[0x0E]), cursor_cell >> 8);
    assert_eq!(u32::from(machine.bus.vga().crtc[0x0F]), cursor_cell & 0xFF);
    assert_eq!(read_ram_u16(&machine, RESULT), 0x0306);
    assert_eq!(read_ram_u16(&machine, RESULT + 2), 0x0D0E);

    inject_and_run(
        &mut machine,
        &[0xB8, 0x00, 0x05, 0xCD, 0x10, 0xF4],
        &[],
        BUDGET,
    );
    assert_eq!(read_ram_u8(&machine, BDA_ACTIVE_PAGE), 0);
    assert_eq!(read_ram_u16(&machine, BDA_VIDEO_PAGE_START), 0x0000);
    assert_eq!(machine.bus.vga().crtc[0x0C], 0x00);
    assert_eq!(machine.bus.vga().crtc[0x0D], 0x00);
}

#[test]
fn cursor_shape_and_invisible_bit() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, &set_mode_code(0x03), &[], BUDGET);

    #[rustfmt::skip]
    let shape_code: &[u8] = &[
        0xB4, 0x01,             // MOV AH, 0x01
        0xB9, 0x0C, 0x0B,       // MOV CX, 0x0B0C
        0xCD, 0x10,             // INT 10h
        0xF4,                   // HLT
    ];
    inject_and_run(&mut machine, shape_code, &[], BUDGET);
    assert_eq!(read_ram_u16(&machine, BDA_CURSOR_SHAPE), 0x0B0C);
    assert_eq!(machine.bus.vga().crtc[0x0A], 0x0B);
    assert_eq!(machine.bus.vga().crtc[0x0B], 0x0C);

    #[rustfmt::skip]
    let hide_code: &[u8] = &[
        0xB4, 0x01,             // MOV AH, 0x01
        0xB9, 0x00, 0x20,       // MOV CX, 0x2000
        0xCD, 0x10,             // INT 10h
        0xF4,                   // HLT
    ];
    inject_and_run(&mut machine, hide_code, &[], BUDGET);
    assert_eq!(read_ram_u16(&machine, BDA_CURSOR_SHAPE), 0x2000);
    assert_eq!(machine.bus.vga().crtc[0x0A], 0x20);
}

/// Mode 03h, then AH=13h in all four sub-modes from the buffer at 0x3000.
#[rustfmt::skip]
const WRITE_STRING_CODE: &[u8] = &[
    0xB8, 0x03, 0x00,       // MOV AX, 0x0003
    0xCD, 0x10,             // INT 10h
    0xB8, 0x00, 0x03,       // MOV AX, 0x0300
    0x8E, 0xC0,             // MOV ES, AX
    // AL=00h: characters with the BL attribute, cursor restored.
    0xBD, 0x00, 0x00,       // MOV BP, 0x0000
    0xB8, 0x00, 0x13,       // MOV AX, 0x1300
    0xBB, 0x1F, 0x00,       // MOV BX, 0x001F
    0xB9, 0x02, 0x00,       // MOV CX, 0x0002
    0xBA, 0x00, 0x05,       // MOV DX, 0x0500
    0xCD, 0x10,             // INT 10h
    // AL=01h: cursor moves to the string end.
    0xBD, 0x00, 0x00,       // MOV BP, 0x0000
    0xB8, 0x01, 0x13,       // MOV AX, 0x1301
    0xBB, 0x1F, 0x00,       // MOV BX, 0x001F
    0xB9, 0x02, 0x00,       // MOV CX, 0x0002
    0xBA, 0x00, 0x06,       // MOV DX, 0x0600
    0xCD, 0x10,             // INT 10h
    // AL=02h: character/attribute pairs, cursor restored.
    0xBD, 0x10, 0x00,       // MOV BP, 0x0010
    0xB8, 0x02, 0x13,       // MOV AX, 0x1302
    0xBB, 0x00, 0x00,       // MOV BX, 0x0000
    0xB9, 0x02, 0x00,       // MOV CX, 0x0002
    0xBA, 0x00, 0x07,       // MOV DX, 0x0700
    0xCD, 0x10,             // INT 10h
    // AL=03h: character/attribute pairs, cursor moves.
    0xBD, 0x10, 0x00,       // MOV BP, 0x0010
    0xB8, 0x03, 0x13,       // MOV AX, 0x1303
    0xBB, 0x00, 0x00,       // MOV BX, 0x0000
    0xB9, 0x02, 0x00,       // MOV CX, 0x0002
    0xBA, 0x00, 0x08,       // MOV DX, 0x0800
    0xCD, 0x10,             // INT 10h
    0xF4,                   // HLT
];

#[test]
fn write_string_sub_modes() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, &[0xF4], &[], 1_000_000);
    write_bytes(&mut machine, 0x3000, b"HI");
    write_bytes(&mut machine, 0x3010, &[b'X', 0x2E, b'Y', 0x4A]);
    inject_and_run(&mut machine, WRITE_STRING_CODE, &[], BUDGET);

    assert_eq!(
        text_cell(&machine, 0, PAGE_SIZE, COLUMNS, 5, 0),
        (b'H', 0x1F)
    );
    assert_eq!(
        text_cell(&machine, 0, PAGE_SIZE, COLUMNS, 5, 1),
        (b'I', 0x1F)
    );
    assert_eq!(
        text_cell(&machine, 0, PAGE_SIZE, COLUMNS, 6, 0),
        (b'H', 0x1F)
    );
    assert_eq!(
        text_cell(&machine, 0, PAGE_SIZE, COLUMNS, 6, 1),
        (b'I', 0x1F)
    );
    assert_eq!(
        text_cell(&machine, 0, PAGE_SIZE, COLUMNS, 7, 0),
        (b'X', 0x2E)
    );
    assert_eq!(
        text_cell(&machine, 0, PAGE_SIZE, COLUMNS, 7, 1),
        (b'Y', 0x4A)
    );
    assert_eq!(
        text_cell(&machine, 0, PAGE_SIZE, COLUMNS, 8, 0),
        (b'X', 0x2E)
    );
    assert_eq!(
        text_cell(&machine, 0, PAGE_SIZE, COLUMNS, 8, 1),
        (b'Y', 0x4A)
    );
    // The last sub-mode moved the cursor to the string end.
    assert_eq!(read_ram_u16(&machine, BDA_CURSOR_PAGE_0), 0x0802);
}

/// Mode 03h, then teletype BEL, then an interrupt-serving idle loop.
#[rustfmt::skip]
const TELETYPE_BEL_CODE: &[u8] = &[
    0xB8, 0x03, 0x00,       // MOV AX, 0x0003
    0xCD, 0x10,             // INT 10h
    0xB8, 0x07, 0x0E,       // MOV AX, 0x0E07 BEL
    0xBB, 0x00, 0x00,       // MOV BX, 0x0000
    0xCD, 0x10,             // INT 10h
    0xFB,                   // STI
    0xF4,                   // HLT
    0xEB, 0xFD,             // JMP to the HLT
];

#[test]
fn teletype_bel_beeps_and_stops() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, &[0xF4], &[], 1_000_000);
    // A short budget: the BEL just started, no timer tick fired yet.
    inject_and_run(&mut machine, TELETYPE_BEL_CODE, &[], 500_000);
    assert_eq!(machine.bus.io_read_byte(0x0061) & 0x03, 0x03);
    // Two timer ticks (about 110 ms) later the gate bits are cleared.
    machine.run_for(25_000_000);
    assert_eq!(machine.bus.io_read_byte(0x0061) & 0x03, 0x00);
}

#[test]
fn scrolled_text_scene_matches_golden_hash() {
    let mut machine = create_machine_dx50();
    // Mode 03h, hide the cursor, then thirty teletype lines (five scrolls).
    let mut code: Vec<u8> = vec![0xB8, 0x03, 0x00, 0xCD, 0x10];
    code.extend_from_slice(&[0xB4, 0x01, 0xB9, 0x00, 0x20, 0xCD, 0x10]);
    code.extend_from_slice(&[0xBB, 0x00, 0x00]);
    for line in 0..30u8 {
        let character = b'A' + (line % 26);
        code.extend_from_slice(&[0xB8, character, 0x0E, 0xCD, 0x10]);
        code.extend_from_slice(&[0xB8, 0x0D, 0x0E, 0xCD, 0x10]);
        code.extend_from_slice(&[0xB8, 0x0A, 0x0E, 0xCD, 0x10]);
    }
    code.push(0xF4);
    boot_and_run(&mut machine, &code, &[], BUDGET);
    render_frame(&mut machine);
    assert_eq!(
        framebuffer_hash(&machine),
        "4f293e14afd355ded5ac1472860f862265c7ee6a151e3b1060448bed81092e6f"
    );
}
