use crate::harness;

const IOSYS_CURSOR_Y: u32 = 0x0600 + 0x0110;
const IOSYS_CURSOR_X: u32 = 0x0600 + 0x011C;
const TEXT_CHAR_BASE: u32 = 0xA0000;
const TEXT_ATTR_BASE: u32 = 0xA2000;
const COLUMNS: u32 = 80;

fn write_ascii_text(machine: &mut machine::Pc9801Ra, row: u32, col: u32, text: &str) {
    for (index, byte) in text.bytes().enumerate() {
        let offset = (row * COLUMNS + col + index as u32) * 2;
        harness::write_bytes(&mut machine.bus, TEXT_CHAR_BASE + offset, &[byte, 0x00]);
        harness::write_bytes(&mut machine.bus, TEXT_ATTR_BASE + offset, &[0xE1, 0x00]);
    }
}

fn set_cursor(machine: &mut machine::Pc9801Ra, row: u8, col: u8) {
    harness::set_cursor_position(&mut machine.bus, row, col);
}

fn cursor(machine: &machine::Pc9801Ra) -> (u8, u8) {
    (
        machine.bus.read_byte_direct(IOSYS_CURSOR_Y),
        machine.bus.read_byte_direct(IOSYS_CURSOR_X),
    )
}

fn emit_int29_bytes(machine: &mut machine::Pc9801Ra, bytes: &[u8]) {
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
