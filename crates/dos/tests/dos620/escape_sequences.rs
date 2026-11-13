use crate::harness;

const IOSYS_CURSOR_Y: u32 = 0x0600 + 0x0110;
const IOSYS_CURSOR_X: u32 = 0x0600 + 0x011C;
const TEXT_CHAR_BASE: u32 = 0xA0000;
const TEXT_ATTR_BASE: u32 = 0xA2000;
const COLUMNS: u32 = 80;

fn write_ascii_text(machine: &mut machine_98::Pc9801Ra, row: u32, col: u32, text: &str) {
    for (index, byte) in text.bytes().enumerate() {
        let offset = (row * COLUMNS + col + index as u32) * 2;
        harness::write_bytes(&mut machine.bus, TEXT_CHAR_BASE + offset, &[byte, 0x00]);
        harness::write_bytes(&mut machine.bus, TEXT_ATTR_BASE + offset, &[0xE1, 0x00]);
    }
}

fn set_cursor(machine: &mut machine_98::Pc9801Ra, row: u8, col: u8) {
    harness::set_cursor_position(&mut machine.bus, row, col);
}

fn cursor(machine: &machine_98::Pc9801Ra) -> (u8, u8) {
    (
        machine.bus.read_byte_direct(IOSYS_CURSOR_Y),
        machine.bus.read_byte_direct(IOSYS_CURSOR_X),
    )
}

fn emit_int29_bytes(machine: &mut machine_98::Pc9801Ra, bytes: &[u8]) {
    let mut code = Vec::with_capacity(bytes.len() * 4 + 2);
    for &byte in bytes {
        code.extend_from_slice(&[0xB0, byte, 0xCD, 0x29]);
    }
    code.extend_from_slice(&[0xFA, 0xF4]);
    harness::inject_and_run(machine, &code);
}

#[test]
fn esc_d_moves_cursor_down_preserving_column_via_int29h() {
    let mut machine = harness::boot_hle();
    set_cursor(&mut machine, 5, 10);

    emit_int29_bytes(&mut machine, &[0x1B, b'D']);

    assert_eq!(cursor(&machine), (6, 10));
}

#[test]
fn esc_e_moves_cursor_to_next_line_start_via_int29h() {
    let mut machine = harness::boot_hle();
    set_cursor(&mut machine, 5, 10);

    emit_int29_bytes(&mut machine, &[0x1B, b'E']);

    assert_eq!(cursor(&machine), (6, 0));
}

#[test]
fn esc_m_moves_cursor_up_preserving_column_via_int29h() {
    let mut machine = harness::boot_hle();
    set_cursor(&mut machine, 5, 10);

    emit_int29_bytes(&mut machine, &[0x1B, b'M']);

    assert_eq!(cursor(&machine), (4, 10));
}

#[test]
fn esc_d_scrolls_at_bottom_via_int29h() {
    let mut machine = harness::boot_hle();
    write_ascii_text(&mut machine, 23, 0, "BOTTOM-23");
    write_ascii_text(&mut machine, 24, 0, "BOTTOM-24");
    set_cursor(&mut machine, 24, 7);

    emit_int29_bytes(&mut machine, &[0x1B, b'D']);

    assert_eq!(cursor(&machine), (24, 7));
    assert!(harness::text_vram_row_to_string(&machine.bus, 23).contains("BOTTOM-24"));
    assert!(
        harness::text_vram_row_to_string(&machine.bus, 24)
            .trim()
            .is_empty()
    );
}

#[test]
fn esc_e_scrolls_at_bottom_and_resets_column_via_int29h() {
    let mut machine = harness::boot_hle();
    write_ascii_text(&mut machine, 24, 0, "BOTTOM-E");
    set_cursor(&mut machine, 24, 7);

    emit_int29_bytes(&mut machine, &[0x1B, b'E']);

    assert_eq!(cursor(&machine), (24, 0));
    assert!(harness::text_vram_row_to_string(&machine.bus, 23).contains("BOTTOM-E"));
    assert!(
        harness::text_vram_row_to_string(&machine.bus, 24)
            .trim()
            .is_empty()
    );
}

#[test]
fn esc_m_scrolls_at_top_via_int29h() {
    let mut machine = harness::boot_hle();
    write_ascii_text(&mut machine, 0, 0, "TOP-M");
    set_cursor(&mut machine, 0, 7);

    emit_int29_bytes(&mut machine, &[0x1B, b'M']);

    assert_eq!(cursor(&machine), (0, 7));
    assert!(
        harness::text_vram_row_to_string(&machine.bus, 0)
            .trim()
            .is_empty()
    );
    assert!(harness::text_vram_row_to_string(&machine.bus, 1).contains("TOP-M"));
}

fn text_cell(machine: &machine_98::Pc9801Ra, row: u32, col: u32) -> (u8, u8) {
    let offset = (row * COLUMNS + col) * 2;
    (
        machine.bus.read_byte_direct(TEXT_CHAR_BASE + offset),
        machine.bus.read_byte_direct(TEXT_ATTR_BASE + offset),
    )
}

#[test]
fn sgr_reads_raw_value_bytes_as_parameter_digits_via_int29h() {
    let mut machine = harness::boot_hle();
    set_cursor(&mut machine, 5, 0);

    emit_int29_bytes(
        &mut machine,
        &[
            0x1B, b'[', b'3', 0x06, b'm', b'A', 0x1B, b'[', b'3', 0x13, b'm', b'B', 0x1B, b'[',
            b'4', b'&', b'm', b'C', 0x1B, b'[', 0x00, b';', 0x07, b'm', b'D',
        ],
    );

    assert_eq!(text_cell(&machine, 5, 0), (b'A', 0xA1));
    assert_eq!(text_cell(&machine, 5, 1), (b'B', 0xC1));
    assert_eq!(text_cell(&machine, 5, 2), (b'C', 0xA5));
    assert_eq!(text_cell(&machine, 5, 3), (b'D', 0xE5));
    assert_eq!(cursor(&machine), (5, 4));
}

#[test]
fn csi_parameter_with_non_digit_low_nibble_aborts_sequence_via_int29h() {
    let mut machine = harness::boot_hle();
    set_cursor(&mut machine, 5, 0);

    emit_int29_bytes(&mut machine, &[0x1B, b'[', b'3', 0x0C, b'm', b'A']);

    assert_eq!(text_cell(&machine, 5, 0), (b'm', 0xE1));
    assert_eq!(text_cell(&machine, 5, 1), (b'A', 0xE1));
}

#[test]
fn each_sgr_sequence_builds_attribute_from_white_via_int29h() {
    let mut machine = harness::boot_hle();
    set_cursor(&mut machine, 5, 0);

    emit_int29_bytes(
        &mut machine,
        &[
            0x1B, b'[', b'4', b'3', b'm', b'A', 0x1B, b'[', b'3', b'7', b'm', b'B', 0x1B, b'[',
            b'7', b';', b'3', b'6', b'm', b'C', 0x1B, b'[', b'4', b'3', b'm', 0x1B, b'[', b'7',
            b'm', b'D', 0x1B, b'[', b'5', b'm', 0x1B, b'[', b'3', b'6', b'm', b'E',
        ],
    );

    assert_eq!(text_cell(&machine, 5, 0), (b'A', 0xC5));
    assert_eq!(text_cell(&machine, 5, 1), (b'B', 0xE1));
    assert_eq!(text_cell(&machine, 5, 2), (b'C', 0xA5));
    assert_eq!(text_cell(&machine, 5, 3), (b'D', 0xE5));
    assert_eq!(text_cell(&machine, 5, 4), (b'E', 0xA1));
}

fn fill_rows_with_labels(machine: &mut machine_98::Pc9801Ra) {
    for row in 0..25 {
        write_ascii_text(machine, row, 0, &format!("{:<80}", format!("ROW-{row:02}")));
    }
}

fn row_text(machine: &machine_98::Pc9801Ra, row: usize) -> String {
    harness::text_vram_row_to_string(&machine.bus, row)
        .trim()
        .to_string()
}

#[test]
fn esc_l_inserts_blank_rows_at_the_cursor_row_via_int29h() {
    let mut machine = harness::boot_hle();
    fill_rows_with_labels(&mut machine);
    set_cursor(&mut machine, 10, 7);

    emit_int29_bytes(&mut machine, &[0x1B, b'[', b'2', b'L']);

    assert_eq!(cursor(&machine), (10, 0));
    assert_eq!(row_text(&machine, 0), "ROW-00");
    assert_eq!(row_text(&machine, 9), "ROW-09");
    assert_eq!(row_text(&machine, 10), "");
    assert_eq!(row_text(&machine, 11), "");
    assert_eq!(row_text(&machine, 12), "ROW-10");
    assert_eq!(row_text(&machine, 24), "ROW-22");
}

#[test]
fn esc_m_deletes_rows_at_the_cursor_row_via_int29h() {
    let mut machine = harness::boot_hle();
    fill_rows_with_labels(&mut machine);
    set_cursor(&mut machine, 10, 7);

    emit_int29_bytes(&mut machine, &[0x1B, b'[', b'2', b'M']);

    assert_eq!(cursor(&machine), (10, 0));
    assert_eq!(row_text(&machine, 0), "ROW-00");
    assert_eq!(row_text(&machine, 9), "ROW-09");
    assert_eq!(row_text(&machine, 10), "ROW-12");
    assert_eq!(row_text(&machine, 22), "ROW-24");
    assert_eq!(row_text(&machine, 23), "");
    assert_eq!(row_text(&machine, 24), "");
}

#[test]
fn echo_of_esc_l_leaves_a_blank_row_between_lines_via_int29h() {
    let mut machine = harness::boot_hle();

    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"\x1b[2J\x1b[L\r\nLINE-A\r\nLINE-B\r\n\x1b[L\r\nLINE-C\r\n");
    emit_int29_bytes(&mut machine, &bytes);

    assert_eq!(row_text(&machine, 0), "");
    assert_eq!(row_text(&machine, 1), "LINE-A");
    assert_eq!(row_text(&machine, 2), "LINE-B");
    assert_eq!(row_text(&machine, 3), "");
    assert_eq!(row_text(&machine, 4), "LINE-C");
    assert_eq!(cursor(&machine), (5, 0));
}
