use crate::harness::*;

/// Message of NEC MS-DOS 3.x for a line without a known keyword.
const UNKNOWN_DIRECTIVE_MESSAGE: &str = "CONFIG.SYSに無効なコマンドかパラメータがあります.";

/// Boots with a floppy whose `CONFIG.SYS` holds `config`.
fn boot_with_config(config: &[u8]) -> Machine {
    boot_with_floppy(create_floppy(&[TestFile {
        name: *b"CONFIG  SYS",
        data: config,
    }]))
}

/// Returns the number of screen rows with the CONFIG.SYS message.
fn message_rows(machine: &Machine) -> usize {
    screen_rows(&machine.bus)
        .iter()
        .filter(|row| row.starts_with("CONFIG.SYS"))
        .count()
}

/// Returns `LASTDRIVE` from the list of lists as a 1-based drive count.
fn lastdrive(machine: &mut Machine) -> u8 {
    let list_of_lists = far_to_linear(list_of_lists_pointer(machine));
    read_byte(&machine.bus, list_of_lists + 0x21)
}

/// Returns the number of entries of the SFT chain.
fn file_count(machine: &mut Machine) -> u16 {
    let list_of_lists = far_to_linear(list_of_lists_pointer(machine));
    let mut sft = far_to_linear(read_far_pointer(&machine.bus, list_of_lists + 0x04));
    let mut entries = 0;
    while sft != 0 && sft < 0xA_0000 {
        entries += read_word(&machine.bus, sft + 4);
        let next = read_far_pointer(&machine.bus, sft);
        if next == (0xFFFF, 0xFFFF) {
            break;
        }
        sft = far_to_linear(next);
    }
    entries
}

#[test]
fn unknown_directive_prints_the_dos_3_message_once_for_each_line() {
    let machine = boot_with_config(b"FOOBAR=1\r\nFILES=30\r\nDOS=HIGH\r\nSTACKS=9,256\r\n");
    assert!(screen_contains(&machine.bus, UNKNOWN_DIRECTIVE_MESSAGE));
    assert!(!screen_contains(&machine.bus, "CONFIG.SYS のエラーです"));
    assert_eq!(message_rows(&machine), 3);
}

#[test]
fn prefix_directives_print_no_message() {
    let machine = boot_with_config(b"FILESXYZ=30\r\nBUFFERSX=5\r\n\r\nFCBS=4,0\r\n");
    assert!(!screen_contains(&machine.bus, UNKNOWN_DIRECTIVE_MESSAGE));
}

#[test]
fn remarks_and_comments_print_the_message() {
    let machine = boot_with_config(b"REM remark\r\nFILES=30\r\n; comment\r\n");
    assert_eq!(message_rows(&machine), 2);
}

#[test]
fn files_prefix_sets_the_file_count() {
    let mut machine = boot_with_config(b"FILESXYZ=30\r\nBUFFERSX=5\r\n");
    assert_eq!(file_count(&mut machine), 30);
}

#[test]
fn single_token_line_continues_with_the_next_line() {
    let mut machine = boot_with_config(
        b"REM\r\nLASTDRIVE=K\r\n;comment\r\nLASTDRIVE=L\r\n[MENU]\r\nLASTDRIVE=M\r\nFILES\r\nLASTDRIVE=N\r\n",
    );
    assert_eq!(message_rows(&machine), 3);
    assert_eq!(lastdrive(&mut machine), 26);
}

#[test]
fn delimiter_behind_the_keyword_keeps_the_next_line() {
    let mut machine = boot_with_config(b"FILES=\r\nLASTDRIVE=K\r\nBUFFERS   \r\nFILES=30\r\n");
    assert_eq!(message_rows(&machine), 0);
    assert_eq!(lastdrive(&mut machine), 11);
    assert_eq!(file_count(&mut machine), 30);
}

#[test]
fn line_with_two_words_keeps_the_next_line() {
    let mut machine = boot_with_config(b";two words\r\nLASTDRIVE=K\r\n");
    assert_eq!(message_rows(&machine), 1);
    assert_eq!(lastdrive(&mut machine), 11);
}

#[test]
fn joined_line_skips_empty_lines_and_takes_whitespace_lines() {
    let mut machine = boot_with_config(b"REM\r\n\r\nFILES=30\r\nREM\r\n   \r\nLASTDRIVE=M\r\n");
    assert_eq!(message_rows(&machine), 2);
    assert_eq!(file_count(&mut machine), 20);
    assert_eq!(lastdrive(&mut machine), 13);
}

#[test]
fn joined_lines_run_as_one_directive() {
    let mut machine = boot_with_config(b"FILES\r\n30\r\nLASTDRIVE\r\nM\r\nFILES=25\r\n");
    assert_eq!(message_rows(&machine), 0);
    assert_eq!(file_count(&mut machine), 25);
    assert_eq!(lastdrive(&mut machine), 26);
}

#[test]
fn leading_semicolons_start_a_directive() {
    let mut machine = boot_with_config(b";FILES=30\r\n;\r\nLASTDRIVE=K\r\n");
    assert_eq!(message_rows(&machine), 0);
    assert_eq!(file_count(&mut machine), 30);
    assert_eq!(lastdrive(&mut machine), 11);
}
