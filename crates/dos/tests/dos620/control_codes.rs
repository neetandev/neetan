use crate::harness;

/// Cursor row of the IO.SYS work area.
const IOSYS_CURSOR_Y: u32 = 0x0600 + 0x0110;
/// Cursor column of the IO.SYS work area.
const IOSYS_CURSOR_X: u32 = 0x0600 + 0x011C;
/// Address of the character cells of the text VRAM.
const TEXT_CHAR_BASE: u32 = 0xA0000;
/// Address of the attribute cells of the text VRAM.
const TEXT_ATTR_BASE: u32 = 0xA2000;
/// Columns of a text row.
const COLUMNS: u32 = 80;
/// Rows of the text screen.
const ROWS: usize = 25;
/// Offset of the data area of the injected code.
const DATA_OFFSET: u16 = 0x0200;
/// Rows that hold a label before each output.
const LABEL_ROWS: [u32; 4] = [0, 10, 23, 24];

/// Writes `text` at `row`, `col` with the attribute E1h.
fn write_ascii_text(machine: &mut machine_98::Pc9801Ra, row: u32, col: u32, text: &str) {
    for (index, byte) in text.bytes().enumerate() {
        let offset = (row * COLUMNS + col + index as u32) * 2;
        harness::write_bytes(&mut machine.bus, TEXT_CHAR_BASE + offset, &[byte, 0x00]);
        harness::write_bytes(&mut machine.bus, TEXT_ATTR_BASE + offset, &[0xE1, 0x00]);
    }
}

/// Writes a label into each row of `LABEL_ROWS`.
fn label_rows(machine: &mut machine_98::Pc9801Ra) {
    for row in LABEL_ROWS {
        write_ascii_text(
            machine,
            row,
            0,
            &format!("ROW{row:02} ABCDEFGHIJKLMNOPQRSTUVWXYZ"),
        );
    }
}

/// Returns the ASCII text of `row` without trailing spaces.
fn row_text(machine: &machine_98::Pc9801Ra, row: usize) -> String {
    harness::text_vram_row_to_string(&machine.bus, row)
        .trim_end()
        .to_string()
}

/// Returns the cursor as (row, column).
fn cursor(machine: &machine_98::Pc9801Ra) -> (u8, u8) {
    (
        machine.bus.read_byte_direct(IOSYS_CURSOR_Y),
        machine.bus.read_byte_direct(IOSYS_CURSOR_X),
    )
}

/// Asserts that every row is blank.
fn assert_screen_blank(machine: &machine_98::Pc9801Ra, path: &str) {
    for row in 0..ROWS {
        assert_eq!(row_text(machine, row), "", "{path}: row {row} is not blank");
    }
}

/// Builder of code that outputs one byte.
type OutputCode = fn(u8) -> Vec<u8>;

/// Code that outputs `byte` through `INT 21h AH=02h`.
fn ah02(byte: u8) -> Vec<u8> {
    vec![0xB4, 0x02, 0xB2, byte, 0xCD, 0x21, 0xFA, 0xF4]
}

/// Code that outputs `byte` through `INT 21h AH=06h`.
fn ah06(byte: u8) -> Vec<u8> {
    vec![0xB4, 0x06, 0xB2, byte, 0xCD, 0x21, 0xFA, 0xF4]
}

/// Code that outputs `byte` through `INT 29h`.
fn int29(byte: u8) -> Vec<u8> {
    vec![0xB0, byte, 0xCD, 0x29, 0xFA, 0xF4]
}

/// Code that outputs the string `byte`, '$' through `INT 21h AH=09h`.
fn ah09(byte: u8) -> Vec<u8> {
    let [offset_low, offset_high] = DATA_OFFSET.to_le_bytes();
    let mut code = vec![
        0xB4,
        0x09, // MOV AH, 09h
        0xBA,
        offset_low,
        offset_high, // MOV DX, DATA_OFFSET
        0xCD,
        0x21, // INT 21h
        0xFA,
        0xF4, // CLI, HLT
    ];
    code.resize(usize::from(DATA_OFFSET), 0x90);
    code.extend_from_slice(&[byte, b'$']);
    code
}

/// Code that writes `data` to handle 1 through `INT 21h AH=40h` and stores
/// AX and the flags at the result area. With `raw` the code sets handle 1 to
/// raw mode first and back to cooked mode after the write.
fn ah40(data: &[u8], raw: bool) -> Vec<u8> {
    let [offset_low, offset_high] = DATA_OFFSET.to_le_bytes();
    let [count_low, count_high] = (data.len() as u16).to_le_bytes();
    let [result_low, result_high] = harness::INJECT_RESULT_OFFSET.to_le_bytes();
    let [flags_low, flags_high] = (harness::INJECT_RESULT_OFFSET + 2).to_le_bytes();
    let mut code = Vec::new();
    if raw {
        #[rustfmt::skip]
        code.extend_from_slice(&[
            0xB8, 0x00, 0x44, // MOV AX, 4400h
            0xBB, 0x01, 0x00, // MOV BX, 1
            0xCD, 0x21,       // INT 21h
            0x80, 0xCA, 0x20, // OR DL, 20h
            0x32, 0xF6,       // XOR DH, DH
            0xB8, 0x01, 0x44, // MOV AX, 4401h
            0xCD, 0x21,       // INT 21h
        ]);
    }
    #[rustfmt::skip]
    code.extend_from_slice(&[
        0xB4, 0x40,                    // MOV AH, 40h
        0xBB, 0x01, 0x00,              // MOV BX, 1
        0xB9, count_low, count_high,   // MOV CX, count
        0xBA, offset_low, offset_high, // MOV DX, DATA_OFFSET
        0xCD, 0x21,                    // INT 21h
        0x9C,                          // PUSHF
        0xA3, result_low, result_high, // MOV [result], AX
        0x58,                          // POP AX
        0xA3, flags_low, flags_high,   // MOV [result + 2], AX
    ]);
    if raw {
        #[rustfmt::skip]
        code.extend_from_slice(&[
            0xB8, 0x00, 0x44, // MOV AX, 4400h
            0xBB, 0x01, 0x00, // MOV BX, 1
            0xCD, 0x21,       // INT 21h
            0x80, 0xE2, 0xDF, // AND DL, DFh
            0x32, 0xF6,       // XOR DH, DH
            0xB8, 0x01, 0x44, // MOV AX, 4401h
            0xCD, 0x21,       // INT 21h
        ]);
    }
    code.extend_from_slice(&[0xFA, 0xF4]);
    code.resize(usize::from(DATA_OFFSET), 0x90);
    code.extend_from_slice(data);
    code
}

/// Returns AX and the carry flag that the `ah40` code stored.
fn ah40_result(machine: &machine_98::Pc9801Ra) -> (u16, bool) {
    let result = harness::read_bytes(&machine.bus, harness::INJECT_RESULT_BASE, 4);
    (
        u16::from_le_bytes([result[0], result[1]]),
        result[2] & 0x01 != 0,
    )
}

#[test]
fn ctrl_z_clears_the_screen_and_homes_the_cursor_on_every_output_path() {
    let paths: [(&str, Vec<u8>); 4] = [
        ("AH=02h", ah02(0x1A)),
        ("AH=06h", ah06(0x1A)),
        ("INT 29h", int29(0x1A)),
        ("AH=09h", ah09(0x1A)),
    ];
    for (path, code) in paths {
        let mut machine = harness::boot_hle();
        label_rows(&mut machine);
        harness::set_cursor_position(&mut machine.bus, 10, 20);

        harness::inject_and_run(&mut machine, &code);

        assert_screen_blank(&machine, path);
        assert_eq!(cursor(&machine), (0, 0), "{path}");
    }
}

#[test]
fn ctrl_z_ends_a_cooked_write_to_con() {
    let mut machine = harness::boot_hle();
    label_rows(&mut machine);
    harness::set_cursor_position(&mut machine.bus, 10, 20);

    harness::inject_and_run(&mut machine, &ah40(b"Q\x1AR", false));

    assert_eq!(ah40_result(&machine), (1, false));
    assert_eq!(row_text(&machine, 0), "ROW00 ABCDEFGHIJKLMNOPQRSTUVWXYZ");
    assert_eq!(row_text(&machine, 10), "ROW10 ABCDEFGHIJKLMNQPQRSTUVWXYZ");
    assert_eq!(cursor(&machine), (10, 21));
}

#[test]
fn ctrl_z_reaches_the_console_in_a_raw_write_to_con() {
    let mut machine = harness::boot_hle();
    label_rows(&mut machine);
    harness::set_cursor_position(&mut machine.bus, 10, 20);

    harness::inject_and_run(&mut machine, &ah40(b"Q\x1AR", true));

    assert_eq!(ah40_result(&machine), (3, false));
    assert_eq!(row_text(&machine, 0), "R");
    for row in 1..ROWS {
        assert_eq!(row_text(&machine, row), "", "row {row}");
    }
    assert_eq!(cursor(&machine), (0, 1));
}

#[test]
fn rs_homes_the_cursor_without_clearing() {
    let mut machine = harness::boot_hle();
    label_rows(&mut machine);
    harness::set_cursor_position(&mut machine.bus, 10, 20);

    harness::inject_and_run(&mut machine, &ah06(0x1E));

    assert_eq!(cursor(&machine), (0, 0));
    for row in LABEL_ROWS {
        assert_eq!(
            row_text(&machine, row as usize),
            format!("ROW{row:02} ABCDEFGHIJKLMNOPQRSTUVWXYZ")
        );
    }
}

#[test]
fn vt_sets_the_cursor_one_row_up_and_stops_at_row_0() {
    let mut machine = harness::boot_hle();
    label_rows(&mut machine);
    harness::set_cursor_position(&mut machine.bus, 10, 20);

    harness::inject_and_run(&mut machine, &int29(0x0B));

    assert_eq!(cursor(&machine), (9, 20));
    assert_eq!(row_text(&machine, 10), "ROW10 ABCDEFGHIJKLMNOPQRSTUVWXYZ");

    harness::set_cursor_position(&mut machine.bus, 0, 20);
    harness::inject_and_run(&mut machine, &int29(0x0B));

    assert_eq!(cursor(&machine), (0, 20));
    assert_eq!(row_text(&machine, 0), "ROW00 ABCDEFGHIJKLMNOPQRSTUVWXYZ");
}

#[test]
fn ff_advances_the_cursor_without_erasing_and_wraps_at_the_last_column() {
    let mut machine = harness::boot_hle();
    label_rows(&mut machine);
    write_ascii_text(&mut machine, 10, 79, "Z");
    harness::set_cursor_position(&mut machine.bus, 10, 20);

    harness::inject_and_run(&mut machine, &ah02(0x0C));

    assert_eq!(cursor(&machine), (10, 21));
    assert!(row_text(&machine, 10).starts_with("ROW10 ABCDEFGHIJKLMNOPQRSTUVWXYZ"));

    harness::set_cursor_position(&mut machine.bus, 10, 79);
    harness::inject_and_run(&mut machine, &ah02(0x0C));

    assert_eq!(cursor(&machine), (11, 0));
    assert!(row_text(&machine, 10).ends_with('Z'));
}

#[test]
fn bs_at_column_0_goes_to_the_last_column_of_the_row_above() {
    let mut machine = harness::boot_hle();
    label_rows(&mut machine);
    harness::set_cursor_position(&mut machine.bus, 10, 0);

    harness::inject_and_run(&mut machine, &ah06(0x08));

    assert_eq!(cursor(&machine), (9, 79));

    harness::set_cursor_position(&mut machine.bus, 0, 0);
    harness::inject_and_run(&mut machine, &ah06(0x08));

    assert_eq!(cursor(&machine), (0, 0));
    assert_eq!(row_text(&machine, 0), "ROW00 ABCDEFGHIJKLMNOPQRSTUVWXYZ");
}

#[test]
fn tab_through_the_console_driver_keeps_the_cells_and_wraps_at_the_row_end() {
    let paths: [(&str, OutputCode); 2] = [("AH=06h", ah06), ("INT 29h", int29)];
    for (path, output) in paths {
        let mut machine = harness::boot_hle();
        label_rows(&mut machine);
        harness::set_cursor_position(&mut machine.bus, 10, 20);

        harness::inject_and_run(&mut machine, &output(0x09));

        assert_eq!(cursor(&machine), (10, 24), "{path}");
        assert_eq!(
            row_text(&machine, 10),
            "ROW10 ABCDEFGHIJKLMNOPQRSTUVWXYZ",
            "{path}"
        );

        for column in [76, 79] {
            harness::set_cursor_position(&mut machine.bus, 10, column);
            harness::inject_and_run(&mut machine, &output(0x09));

            assert_eq!(cursor(&machine), (11, 0), "{path} column {column}");
        }
    }
}

#[test]
fn tab_through_display_character_writes_spaces() {
    let mut machine = harness::boot_hle();
    label_rows(&mut machine);
    harness::set_cursor_position(&mut machine.bus, 10, 20);

    harness::inject_and_run(&mut machine, &ah02(0x09));

    assert_eq!(cursor(&machine), (10, 24));
    assert_eq!(row_text(&machine, 10), "ROW10 ABCDEFGHIJKLMN    STUVWXYZ");
}
