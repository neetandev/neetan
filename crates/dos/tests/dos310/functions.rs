use crate::harness::*;

/// `BX`, `CX`, `DX` and `SI` in front of a call.
const PRESETS: [u16; 4] = [0x1234, 0x5678, 0x9ABC, 0x1111];
/// Carry flag in the flags word.
const CARRY_FLAG: u16 = 0x0001;

/// Calls `INT 21h` with `ax`, the presets and the carry flag `carry`.
/// Returns `[AX, BX, CX, DX, SI, DS, flags]` after the call.
fn call_int21h(machine: &mut Machine, ax: u16, carry: bool) -> [u16; 7] {
    let [ax_low, ax_high] = ax.to_le_bytes();
    let [bx_low, bx_high] = PRESETS[0].to_le_bytes();
    let [cx_low, cx_high] = PRESETS[1].to_le_bytes();
    let [dx_low, dx_high] = PRESETS[2].to_le_bytes();
    let [si_low, si_high] = PRESETS[3].to_le_bytes();
    let carry_instruction = if carry { 0xF9 } else { 0xF8 };
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, ax_low, ax_high,  // MOV AX, ax
        0xBB, bx_low, bx_high,  // MOV BX, PRESETS[0]
        0xB9, cx_low, cx_high,  // MOV CX, PRESETS[1]
        0xBA, dx_low, dx_high,  // MOV DX, PRESETS[2]
        0xBE, si_low, si_high,  // MOV SI, PRESETS[3]
        carry_instruction,      // STC or CLC
        0xCD, 0x21,             // INT 21h
        0x9C,                   // PUSHF
        0x2E, 0xA3, 0x00, 0x01, // MOV CS:[0100h], AX
        0x2E, 0x89, 0x1E, 0x02, 0x01, // MOV CS:[0102h], BX
        0x2E, 0x89, 0x0E, 0x04, 0x01, // MOV CS:[0104h], CX
        0x2E, 0x89, 0x16, 0x06, 0x01, // MOV CS:[0106h], DX
        0x2E, 0x89, 0x36, 0x08, 0x01, // MOV CS:[0108h], SI
        0x2E, 0x8C, 0x1E, 0x0A, 0x01, // MOV CS:[010Ah], DS
        0x58,                   // POP AX
        0x2E, 0xA3, 0x0C, 0x01, // MOV CS:[010Ch], AX
        0xFA,                   // CLI
        0xF4,                   // HLT
    ];
    inject_and_run(machine, code);
    [0, 2, 4, 6, 8, 10, 12].map(|offset| result_word(&machine.bus, offset))
}

#[test]
fn dbcs_functions_answer_as_an_undefined_function() {
    let mut machine = boot();
    for ax in [0x6300, 0x6301, 0x6302, 0x63FF] {
        for carry in [false, true] {
            let result = call_int21h(&mut machine, ax, carry);
            assert_eq!(result[0], 0x6300, "AX={ax:04X}h");
            assert_eq!(result[1..5], PRESETS, "AX={ax:04X}h");
            assert_eq!(result[5], INJECT_CODE_SEGMENT, "AX={ax:04X}h");
            assert_eq!(result[6] & CARRY_FLAG != 0, carry, "AX={ax:04X}h");
        }
    }
}

/// Cursor row of the IO.SYS work area.
const IOSYS_CURSOR_ROW: u32 = 0x0710;
/// Cursor column of the IO.SYS work area.
const IOSYS_CURSOR_COLUMN: u32 = 0x071C;
/// Address of the ASCII text cells of the text VRAM.
const TEXT_CHAR_BASE: u32 = 0xA0000;

/// Writes `text` at `row`, column 0 of the text VRAM and sets the cursor to
/// `row`, `column`.
fn prepare_screen(machine: &mut Machine, row: u32, text: &[u8], column: u8) {
    for (index, &byte) in text.iter().enumerate() {
        let address = TEXT_CHAR_BASE + (row * 80 + index as u32) * 2;
        write_bytes(&mut machine.bus, address, &[byte, 0x00]);
    }
    write_bytes(&mut machine.bus, IOSYS_CURSOR_ROW, &[row as u8]);
    write_bytes(&mut machine.bus, IOSYS_CURSOR_COLUMN, &[column]);
}

/// Returns the cursor as (row, column).
fn cursor(machine: &Machine) -> (u8, u8) {
    (
        read_byte(&machine.bus, IOSYS_CURSOR_ROW),
        read_byte(&machine.bus, IOSYS_CURSOR_COLUMN),
    )
}

#[test]
fn ctrl_z_through_direct_console_output_clears_the_screen_and_homes_the_cursor() {
    let mut machine = boot();
    prepare_screen(&mut machine, 11, b"INSTALLER BOX", 12);
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x06, // MOV AH, 06h
        0xB2, 0x1A, // MOV DL, 1Ah
        0xCD, 0x21, // INT 21h
        0xFA,       // CLI
        0xF4,       // HLT
    ];
    inject_and_run(&mut machine, code);

    for (row, text) in screen_rows(&machine.bus).iter().enumerate() {
        assert_eq!(text.trim_end(), "", "row {row}");
    }
    assert_eq!(cursor(&machine), (0, 0));
}

#[test]
fn ctrl_z_ends_a_write_to_con() {
    let mut machine = boot();
    prepare_screen(&mut machine, 11, b"INSTALLER BOX", 13);
    write_bytes(&mut machine.bus, INJECT_DATA_BASE, b"!\x1A?");
    let [data_low, data_high] = INJECT_DATA_OFFSET.to_le_bytes();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x40,                // MOV AH, 40h
        0xBB, 0x01, 0x00,          // MOV BX, 1
        0xB9, 0x03, 0x00,          // MOV CX, 3
        0xBA, data_low, data_high, // MOV DX, INJECT_DATA_OFFSET
        0xCD, 0x21,                // INT 21h
        0x2E, 0xA3, 0x00, 0x01,    // MOV CS:[0100h], AX
        0xFA,                      // CLI
        0xF4,                      // HLT
    ];
    inject_and_run(&mut machine, code);

    assert_eq!(result_word(&machine.bus, 0), 1);
    assert_eq!(screen_rows(&machine.bus)[11].trim_end(), "INSTALLER BOX!");
    assert_eq!(cursor(&machine), (11, 14));
}
