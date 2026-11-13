//! Screen rows of batch files against the captures of the real NEC MS-DOS 6.20.
//!
//! Each case clears the screen with CLS and runs one batch file from the
//! prompt of drive A:. Row 0 is empty and row 1 holds the typed command line.

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

/// AUTOEXEC.BAT that turns echo off for the rest of the boot.
const AUTOEXEC_ECHO_OFF: &[u8] = b"@ECHO OFF\r\nPATH A:\\\r\n";

/// The batch files of the capture and the batch files that turn echo off
/// through the general command dispatch.
const BATCH_FILES: [([u8; 11], &[u8]); 9] = [
    (*b"B1      BAT", b"echo off\r\necho LINE-1\r\n"),
    (*b"B2      BAT", b"echo off\r\necho LINE-1\r\nrem last\r\n"),
    (*b"B3      BAT", b"echo LINE-1\r\n"),
    (*b"B4      BAT", b"@echo off\r\necho LINE-1\r\n"),
    (*b"B5      BAT", b"echo off\r\necho LINE-1\r\necho on\r\n"),
    (*b"B6      BAT", b"echo off\r\n"),
    (*b"C1      BAT", b"IF A==A ECHO OFF\r\necho X\r\n"),
    (*b"C2      BAT", b"ECHO  OFF\r\necho X\r\n"),
    (*b"C3      BAT", b"ECHO OFF >NUL\r\necho X\r\n"),
];

/// Boots the HLE DOS 6.20 from a floppy with the batch files and an
/// AUTOEXEC.BAT that runs `@ECHO OFF`.
fn boot_with_batch_files() -> machine_98::Pc9801Ra {
    let mut files: Vec<([u8; 11], &[u8])> = vec![(*b"AUTOEXECBAT", AUTOEXEC_ECHO_OFF)];
    files.extend_from_slice(&BATCH_FILES);
    boot_hle_with_forced_os(Some(create_test_floppy_with_files(&files)), None)
}

/// Returns the rows of the screen without trailing spaces.
fn screen_rows(machine: &machine_98::Pc9801Ra) -> Vec<String> {
    (0..SCREEN_ROWS)
        .map(|row| {
            text_vram_row_to_string(&machine.bus, row)
                .trim_end()
                .to_string()
        })
        .collect()
}

/// Returns `rows` padded with empty rows to the full screen.
fn full_screen(rows: &[&str]) -> Vec<String> {
    let mut screen: Vec<String> = rows.iter().map(|row| row.to_string()).collect();
    screen.resize(SCREEN_ROWS, String::new());
    screen
}

/// Runs the machine until the screen shows `rows` or the budget ends.
fn run_until_rows(machine: &mut machine_98::Pc9801Ra, rows: &[&str]) {
    let expected = full_screen(rows);
    let mut total_cycles = 0;
    while screen_rows(machine) != expected && total_cycles < ROW_CHECK_BUDGET {
        total_cycles += machine.run_for(ROW_CHECK_INTERVAL);
    }
    assert_eq!(screen_rows(machine), expected);
}

/// Clears the screen, runs the batch file `name` and checks the rows and the
/// cursor position behind the last prompt.
fn run_case(machine: &mut machine_98::Pc9801Ra, name: &str, rows: &[&str]) {
    type_string(&mut machine.bus, b"CLS\r");
    run_until_rows(machine, &["", "A:\\>"]);
    type_string(&mut machine.bus, format!("{name}\r").as_bytes());
    run_until_rows(machine, rows);
    let cursor = (
        machine.bus.read_byte_direct(IOSYS_CURSOR_Y),
        machine.bus.read_byte_direct(IOSYS_CURSOR_X),
    );
    assert_eq!(cursor, (rows.len() as u8 - 1, 4), "cursor after {name}");
}

#[test]
fn batch_rows_match_nec_ms_dos_620() {
    let mut machine = boot_with_batch_files();
    run_case(
        &mut machine,
        "B1",
        &["", "A:\\>B1", "", "A:\\>echo off", "LINE-1", "A:\\>"],
    );
    run_case(
        &mut machine,
        "B2",
        &["", "A:\\>B2", "", "A:\\>echo off", "LINE-1", "A:\\>"],
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
    run_case(&mut machine, "B4", &["", "A:\\>B4", "LINE-1", "A:\\>"]);
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
        &["", "A:\\>B6", "", "A:\\>echo off", "A:\\>"],
    );
}

#[test]
fn batch_after_autoexec_with_echo_off_starts_with_echo_on() {
    let mut machine = boot_with_batch_files();
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
}

#[test]
fn echo_off_through_the_command_dispatch_turns_batch_echo_off() {
    let mut machine = boot_with_batch_files();
    run_case(
        &mut machine,
        "C1",
        &["", "A:\\>C1", "", "A:\\>IF A==A ECHO OFF", "X", "A:\\>"],
    );
    run_case(
        &mut machine,
        "C2",
        &["", "A:\\>C2", "", "A:\\>ECHO  OFF", "X", "A:\\>"],
    );
    run_case(
        &mut machine,
        "C3",
        &["", "A:\\>C3", "", "A:\\>ECHO OFF >NUL", "X", "A:\\>"],
    );
}
