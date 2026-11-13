//! Screen rows of batch files against the captures of the real NEC MS-DOS 3.10.
//!
//! Each case clears the screen with CLS and runs one batch file from the
//! prompt of drive A:. Row 0 is empty and row 1 holds the typed command line.
//! The prompt of the HLE DOS is `A:\>`. The real DOS 3.10 shows `A>`.

use crate::harness::*;

/// Cycles between two screen checks.
const ROW_CHECK_INTERVAL: u64 = 1_000_000;

/// Cycles after which a screen check gives up.
const ROW_CHECK_BUDGET: u64 = 300_000_000;

/// Linear address of the cursor row in the IO.SYS work area.
const IOSYS_CURSOR_Y: u32 = 0x0600 + 0x0110;

/// Linear address of the cursor column in the IO.SYS work area.
const IOSYS_CURSOR_X: u32 = 0x0600 + 0x011C;

/// Number of text rows of the screen.
const SCREEN_ROWS: usize = 25;

/// Boots the HLE DOS 3.10 from a floppy with the batch files of the capture.
fn boot_with_batch_files() -> Machine {
    boot_with_floppy(create_floppy(&[
        TestFile {
            name: *b"B1      BAT",
            data: b"echo off\r\necho LINE-1\r\n",
        },
        TestFile {
            name: *b"B2      BAT",
            data: b"echo off\r\necho LINE-1\r\nrem last\r\n",
        },
        TestFile {
            name: *b"B3      BAT",
            data: b"echo LINE-1\r\n",
        },
        TestFile {
            name: *b"B4      BAT",
            data: b"@echo off\r\necho LINE-1\r\n",
        },
        TestFile {
            name: *b"B5      BAT",
            data: b"echo off\r\necho LINE-1\r\necho on\r\n",
        },
        TestFile {
            name: *b"B6      BAT",
            data: b"echo off\r\n",
        },
    ]))
}

/// Returns the rows of the screen without trailing spaces.
fn trimmed_rows(machine: &Machine) -> Vec<String> {
    screen_rows(&machine.bus)
        .iter()
        .map(|row| row.trim_end().to_string())
        .collect()
}

/// Returns `rows` padded with empty rows to the full screen.
fn full_screen(rows: &[&str]) -> Vec<String> {
    let mut screen: Vec<String> = rows.iter().map(|row| row.to_string()).collect();
    screen.resize(SCREEN_ROWS, String::new());
    screen
}

/// Runs the machine until the screen shows `rows` or the budget ends.
fn run_until_rows(machine: &mut Machine, rows: &[&str]) {
    let expected = full_screen(rows);
    let mut total_cycles = 0;
    while trimmed_rows(machine) != expected && total_cycles < ROW_CHECK_BUDGET {
        total_cycles += machine.run_for(ROW_CHECK_INTERVAL);
    }
    assert_eq!(trimmed_rows(machine), expected);
}

/// Clears the screen, runs the batch file `name` and checks the rows and the
/// cursor position behind the last prompt.
fn run_case(machine: &mut Machine, name: &str, rows: &[&str]) {
    type_string(&mut machine.bus, b"CLS\r");
    run_until_rows(machine, &["", "A:\\>"]);
    type_string(&mut machine.bus, format!("{name}\r").as_bytes());
    run_until_rows(machine, rows);
    let cursor = (
        read_byte(&machine.bus, IOSYS_CURSOR_Y),
        read_byte(&machine.bus, IOSYS_CURSOR_X),
    );
    assert_eq!(cursor, (rows.len() as u8 - 1, 4), "cursor after {name}");
}

#[test]
fn batch_rows_match_nec_ms_dos_310() {
    let mut machine = boot_with_batch_files();
    run_case(
        &mut machine,
        "B1",
        &["", "A:\\>B1", "", "A:\\>echo off", "LINE-1", "", "A:\\>"],
    );
    run_case(
        &mut machine,
        "B2",
        &["", "A:\\>B2", "", "A:\\>echo off", "LINE-1", "", "A:\\>"],
    );
    run_case(
        &mut machine,
        "B3",
        &[
            "",
            "A:\\>B3",
            "",
            "A:\\>echo LINE-1",
            "LINE-1",
            "",
            "A:\\>",
            "A:\\>",
        ],
    );
    run_case(
        &mut machine,
        "B4",
        &[
            "",
            "A:\\>B4",
            "",
            "A:\\>@echo off",
            "Bad command or file name",
            "",
            "A:\\>echo LINE-1",
            "LINE-1",
            "",
            "A:\\>",
            "A:\\>",
        ],
    );
    run_case(
        &mut machine,
        "B5",
        &[
            "",
            "A:\\>B5",
            "",
            "A:\\>echo off",
            "LINE-1",
            "",
            "A:\\>",
            "A:\\>",
        ],
    );
    run_case(
        &mut machine,
        "B6",
        &["", "A:\\>B6", "", "A:\\>echo off", "", "A:\\>"],
    );
}
