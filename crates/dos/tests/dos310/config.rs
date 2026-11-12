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

#[test]
fn unknown_directive_prints_the_dos_3_message_once_for_each_line() {
    let machine = boot_with_config(b"FOOBAR=1\r\nFILES=30\r\nDOS=HIGH\r\nSTACKS=9,256\r\n");
    assert!(screen_contains(&machine.bus, UNKNOWN_DIRECTIVE_MESSAGE));
    assert!(!screen_contains(&machine.bus, "CONFIG.SYS のエラーです"));
    let message_rows = screen_rows(&machine.bus)
        .iter()
        .filter(|row| row.starts_with("CONFIG.SYS"))
        .count();
    assert_eq!(message_rows, 3);
}

#[test]
fn prefix_directives_print_no_message() {
    let machine = boot_with_config(b"FILESXYZ=30\r\nBUFFERSX=5\r\n\r\nFCBS=4,0\r\n");
    assert!(!screen_contains(&machine.bus, UNKNOWN_DIRECTIVE_MESSAGE));
}

#[test]
fn remarks_and_comments_print_the_message() {
    let machine = boot_with_config(b"REM remark\r\nFILES=30\r\n; comment\r\n");
    let message_rows = screen_rows(&machine.bus)
        .iter()
        .filter(|row| row.starts_with("CONFIG.SYS"))
        .count();
    assert_eq!(message_rows, 2);
}

#[test]
fn files_prefix_sets_the_file_count() {
    let mut machine = boot_with_config(b"FILESXYZ=30\r\nBUFFERSX=5\r\n");
    let (segment, offset) = list_of_lists_pointer(&mut machine);
    let list_of_lists = far_to_linear((segment, offset));
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
    assert_eq!(entries, 30);
}
