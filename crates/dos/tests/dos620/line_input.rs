use crate::harness;

/// Cursor row of the IO.SYS work area.
const IOSYS_CURSOR_Y: u32 = 0x0600 + 0x0110;
/// Cursor column of the IO.SYS work area.
const IOSYS_CURSOR_X: u32 = 0x0600 + 0x011C;
/// Offset of the line buffer of the injected code.
const BUFFER_OFFSET: u16 = 0x0200;
/// Linear address of the line buffer of the injected code.
const BUFFER_BASE: u32 = harness::INJECT_CODE_BASE + BUFFER_OFFSET as u32;
/// Longest line of the `INT 21h AH=0Ah` buffer.
const BUFFER_LIMIT: u8 = 60;
/// Label of row 0. A screen clear erases it.
const ROW_0_LABEL: &str = "ROW00 LABEL";

/// Returns the cursor as (row, column).
fn cursor(machine: &machine_98::Pc9801Ra) -> (u8, u8) {
    (
        machine.bus.read_byte_direct(IOSYS_CURSOR_Y),
        machine.bus.read_byte_direct(IOSYS_CURSOR_X),
    )
}

/// Returns the ASCII text of `row` without trailing spaces.
fn row_text(machine: &machine_98::Pc9801Ra, row: usize) -> String {
    harness::text_vram_row_to_string(&machine.bus, row)
        .trim_end()
        .to_string()
}

/// Boots, writes the label into row 0, clears row 10 and sets the cursor to
/// row 10, column 0.
fn prepare() -> machine_98::Pc9801Ra {
    let mut machine = harness::boot_hle();
    let label = format!("{ROW_0_LABEL:80}");
    for (column, byte) in label.bytes().enumerate() {
        harness::write_bytes(&mut machine.bus, 0xA0000 + column as u32 * 2, &[byte, 0x00]);
    }
    for column in 0..80 {
        harness::write_bytes(
            &mut machine.bus,
            0xA0000 + (800 + column) * 2,
            &[b' ', 0x00],
        );
    }
    harness::set_cursor_position(&mut machine.bus, 10, 0);
    machine
}

/// Types `keys` into `INT 21h AH=0Ah` and returns the stored bytes up to and
/// with the CR.
fn read_line_0ah(machine: &mut machine_98::Pc9801Ra, keys: &[u8]) -> Vec<u8> {
    harness::write_bytes(&mut machine.bus, BUFFER_BASE, &[BUFFER_LIMIT, 0]);
    harness::type_string(&mut machine.bus, keys);
    let [buffer_low, buffer_high] = BUFFER_OFFSET.to_le_bytes();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xBA, buffer_low, buffer_high, // MOV DX, BUFFER_OFFSET
        0xB4, 0x0A,                    // MOV AH, 0Ah
        0xCD, 0x21,                    // INT 21h
        0xFA,                          // CLI
        0xF4,                          // HLT
    ];
    harness::inject_and_run_with_budget(machine, code, harness::INJECT_BUDGET_DISK_IO);
    let count = machine.bus.read_byte_direct(BUFFER_BASE + 1);
    harness::read_bytes(&machine.bus, BUFFER_BASE + 2, usize::from(count) + 1)
}

/// Types `keys` into `INT 21h AH=3Fh` on handle 0 (CX=60) and returns the
/// bytes of the read.
fn read_line_3fh(machine: &mut machine_98::Pc9801Ra, keys: &[u8]) -> Vec<u8> {
    harness::type_string(&mut machine.bus, keys);
    let [buffer_low, buffer_high] = BUFFER_OFFSET.to_le_bytes();
    let [result_low, result_high] = harness::INJECT_RESULT_OFFSET.to_le_bytes();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x3F,                    // MOV AH, 3Fh
        0xBB, 0x00, 0x00,              // MOV BX, 0
        0xB9, BUFFER_LIMIT, 0x00,      // MOV CX, BUFFER_LIMIT
        0xBA, buffer_low, buffer_high, // MOV DX, BUFFER_OFFSET
        0xCD, 0x21,                    // INT 21h
        0xA3, result_low, result_high, // MOV [result], AX
        0xFA,                          // CLI
        0xF4,                          // HLT
    ];
    harness::inject_and_run_with_budget(machine, code, harness::INJECT_BUDGET_DISK_IO);
    let count = harness::result_word(&machine.bus, 0);
    harness::read_bytes(&machine.bus, BUFFER_BASE, usize::from(count))
}

#[test]
fn line_input_echoes_control_bytes_as_caret_and_keeps_them() {
    for byte in [0x01, 0x02, 0x04, 0x05, 0x07, 0x0B, 0x0C, 0x1A, 0x1E] {
        let mut machine = prepare();

        let line = read_line_0ah(&mut machine, &[b'A', b'B', byte, b'C', b'\r']);

        let letter = char::from(byte + 0x40);
        assert_eq!(line, [b'A', b'B', byte, b'C', b'\r'], "byte {byte:02X}h");
        assert_eq!(
            row_text(&machine, 10),
            format!("AB^{letter}C"),
            "byte {byte:02X}h"
        );
        assert_eq!(row_text(&machine, 0), ROW_0_LABEL, "byte {byte:02X}h");
    }
}

#[test]
fn line_input_enter_echoes_only_a_carriage_return() {
    let mut machine = prepare();

    read_line_0ah(&mut machine, b"AB\r");

    assert_eq!(cursor(&machine), (10, 0));
}

#[test]
fn line_input_backspace_erases_both_cells_of_a_caret_echo() {
    let mut machine = prepare();

    let line = read_line_0ah(&mut machine, b"AB\x1A\x08C\r");

    assert_eq!(line, b"ABC\r");
    assert_eq!(row_text(&machine, 10), "ABC");
}

#[test]
fn line_input_ctrl_u_erases_the_line() {
    let mut machine = prepare();

    let line = read_line_0ah(&mut machine, b"A\x1AB\x15C\r");

    assert_eq!(line, b"C\r");
    assert_eq!(row_text(&machine, 10), "C");
}

#[test]
fn line_input_drops_ctrl_f() {
    let mut machine = prepare();

    let line = read_line_0ah(&mut machine, b"AB\x06C\r");

    assert_eq!(line, b"ABC\r");
    assert_eq!(row_text(&machine, 10), "ABC");
}

#[test]
fn line_input_line_feed_echoes_a_new_line_and_keeps_nothing() {
    let mut machine = prepare();

    let line = read_line_0ah(&mut machine, b"AB\nC\r");

    assert_eq!(line, b"ABC\r");
    assert_eq!(row_text(&machine, 10), "AB");
    assert_eq!(row_text(&machine, 11), "C");
    assert_eq!(cursor(&machine), (11, 0));
}

#[test]
fn console_read_echoes_control_bytes_as_caret_and_keeps_them() {
    let mut machine = prepare();

    let line = read_line_3fh(&mut machine, b"AB\x1AC\r");

    assert_eq!(line, b"AB\x1AC\r\n");
    assert_eq!(row_text(&machine, 10), "AB^ZC");
    assert_eq!(row_text(&machine, 0), ROW_0_LABEL);
    assert_eq!(cursor(&machine), (11, 0));
}

#[test]
fn console_read_line_feed_echoes_a_new_line_and_keeps_nothing() {
    let mut machine = prepare();

    let line = read_line_3fh(&mut machine, b"AB\nC\r");

    assert_eq!(line, b"ABC\r\n");
    assert_eq!(row_text(&machine, 10), "AB");
    assert_eq!(row_text(&machine, 11), "C");
    assert_eq!(cursor(&machine), (12, 0));
}
