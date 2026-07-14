use crate::harness::*;

#[test]
fn dir_lists_standard_2hd_floppy_without_bpb() {
    let floppy = create_test_floppy_without_bpb();
    let mut machine = boot_hle_with_floppy_image(floppy);

    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"DIR\r");
    run_until_prompt(&mut machine);

    assert!(
        find_row_containing(&machine.bus, "TESTFILE").is_some(),
        "DIR should list files from a standard 2HD FAT12 floppy without a BPB"
    );
}

fn row_containing(machine: &machine_98::Pc9801Ra, needle: &str) -> usize {
    find_row_containing(&machine.bus, needle)
        .unwrap_or_else(|| panic!("expected to find row containing {needle:?}"))
}

fn format_with_commas(value: usize) -> String {
    let digits = value.to_string();
    let mut formatted = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, byte) in digits.bytes().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            formatted.push(',');
        }
        formatted.push(byte as char);
    }
    formatted
}

#[test]
fn dir_basic_listing() {
    let mut machine = boot_hle_with_floppy();
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"DIR\r");
    run_until_prompt(&mut machine);

    let testfile = [
        0x0054, 0x0045, 0x0053, 0x0054, 0x0046, 0x0049, 0x004C, 0x0045,
    ]; // "TESTFILE"
    assert!(
        find_string_in_text_vram(&machine.bus, &testfile),
        "DIR should list TESTFILE"
    );

    let command = [0x0043, 0x004F, 0x004D, 0x004D, 0x0041, 0x004E, 0x0044]; // "COMMAND"
    assert!(
        find_string_in_text_vram(&machine.bus, &command),
        "DIR should list COMMAND"
    );
}

#[test]
fn dir_lists_cdrom_root() {
    let mut machine = boot_hle_with_cdrom();
    type_string(&mut machine.bus, b"Q:\r");
    run_until_prompt_ap(&mut machine);

    type_string(&mut machine.bus, b"DIR\r");
    run_until_prompt_ap(&mut machine);

    let readme = [0x0052, 0x0045, 0x0041, 0x0044, 0x004D, 0x0045]; // "README"
    assert!(
        find_string_in_text_vram(&machine.bus, &readme),
        "DIR on Q: should list README.TXT from the synthetic CD-ROM"
    );
}

#[test]
fn dir_on_cdrom_subdirectory_suppresses_self_and_parent_entries() {
    let mut machine = boot_hle_with_cdrom_image(create_test_cdimage_with_xcopy_tree());
    type_string(&mut machine.bus, b"Q:\r");
    run_until_prompt_ap(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt_ap(&mut machine);
    type_string_long_ap(&mut machine, b"DIR Q:\\YOURFOLD\r");
    run_until_prompt_ap(&mut machine);

    assert!(
        find_row_containing(&machine.bus, "5 file(s)").is_some(),
        "DIR on a CD-ROM subdirectory should count only real entries (3 dirs + 2 files), \
         ISO9660 . and .. records must be suppressed"
    );
    assert!(
        find_row_containing(&machine.bus, "7 file(s)").is_none(),
        "DIR on a CD-ROM subdirectory must not include ISO9660 self/parent records in the count"
    );
}

#[test]
fn dir_wildcard_txt() {
    let mut machine = boot_hle_with_floppy();
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"DIR *.TXT\r");
    run_until_prompt(&mut machine);

    let testfile = [
        0x0054, 0x0045, 0x0053, 0x0054, 0x0046, 0x0049, 0x004C, 0x0045,
    ]; // "TESTFILE"
    assert!(
        find_string_in_text_vram(&machine.bus, &testfile),
        "DIR *.TXT should list TESTFILE"
    );

    let command_com = [
        0x0043, 0x004F, 0x004D, 0x004D, 0x0041, 0x004E, 0x0044, 0x0020, 0x0020, 0x0043, 0x004F,
        0x004D,
    ]; // "COMMAND  COM"
    assert!(
        !find_string_in_text_vram(&machine.bus, &command_com),
        "DIR *.TXT should not list COMMAND.COM"
    );
}

#[test]
fn dir_bare_format() {
    let mut machine = boot_hle_with_floppy();
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"DIR /B\r");
    run_until_prompt(&mut machine);

    let testfile_txt = [
        0x0054, 0x0045, 0x0053, 0x0054, 0x0046, 0x0049, 0x004C, 0x0045, 0x002E, 0x0054, 0x0058,
        0x0054,
    ]; // "TESTFILE.TXT"
    assert!(
        find_string_in_text_vram(&machine.bus, &testfile_txt),
        "DIR /B should show TESTFILE.TXT"
    );
}

#[test]
fn dir_file_count() {
    let mut machine = boot_hle_with_floppy();
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"DIR\r");
    run_until_prompt(&mut machine);

    let files = [0x0066, 0x0069, 0x006C, 0x0065, 0x0028, 0x0073, 0x0029]; // "file(s)"
    assert!(
        find_string_in_text_vram(&machine.bus, &files),
        "DIR should show file(s) count in summary"
    );
}

#[test]
fn dir_nonexistent() {
    let mut machine = boot_hle_with_floppy();
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    type_string_long(&mut machine, b"DIR NOFILE.XYZ\r");
    run_until_prompt(&mut machine);

    let not_found = [
        0x004E, 0x006F, 0x0074, 0x0020, 0x0046, 0x006F, 0x0075, 0x006E, 0x0064,
    ]; // "Not Found"
    assert!(
        find_string_in_text_vram(&machine.bus, &not_found),
        "DIR NOFILE.XYZ should show 'Not Found'"
    );
}

#[test]
fn dir_sorted_by_name() {
    let mut machine = boot_hle_with_floppy();
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"MD SUBDIR\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"DIR /ON\r");
    run_until_prompt(&mut machine);

    let subdir_row = row_containing(&machine, "SUBDIR");
    let command_row = row_containing(&machine, "COMMAND");
    let test_row = row_containing(&machine, "TEST     COM");
    let testfile_row = row_containing(&machine, "TESTFILE TXT");

    assert!(
        subdir_row < command_row,
        "DIR /ON should list directories before files"
    );
    assert!(command_row < test_row, "DIR /ON should sort files by name");
    assert!(
        test_row < testfile_row,
        "DIR /ON should keep TEST.COM before TESTFILE.TXT"
    );
}

#[test]
fn dir_sorted_by_extension() {
    let mut machine = boot_hle_with_floppy();
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"MD SUBDIR\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"DIR /OE\r");
    run_until_prompt(&mut machine);

    let subdir_row = row_containing(&machine, "SUBDIR");
    let command_row = row_containing(&machine, "COMMAND");
    let test_row = row_containing(&machine, "TEST     COM");
    let testfile_row = row_containing(&machine, "TESTFILE TXT");

    assert!(
        subdir_row < command_row,
        "DIR /OE should keep directories before files"
    );
    assert!(
        command_row < test_row,
        "DIR /OE should sort matching COM entries by name"
    );
    assert!(
        test_row < testfile_row,
        "DIR /OE should place COM files before TXT files"
    );
}

#[test]
fn dir_sorted_by_size() {
    let mut machine = boot_hle_with_floppy();
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"MD SUBDIR\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"DIR /OS\r");
    run_until_prompt(&mut machine);

    let subdir_row = row_containing(&machine, "SUBDIR");
    let test_row = row_containing(&machine, "TEST     COM");
    let testfile_row = row_containing(&machine, "TESTFILE TXT");
    let command_row = row_containing(&machine, "COMMAND");

    assert!(
        subdir_row < test_row,
        "DIR /OS should keep directories before files"
    );
    assert!(
        test_row < testfile_row,
        "DIR /OS should place smaller files first"
    );
    assert!(
        testfile_row < command_row,
        "DIR /OS should place larger files later"
    );
}

#[test]
fn dir_default_sorts_directories_before_files_by_name() {
    let mut machine = boot_hle_with_floppy();
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"MD SUBDIR\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"DIR\r");
    run_until_prompt(&mut machine);

    let subdir_row = row_containing(&machine, "SUBDIR");
    let command_row = row_containing(&machine, "COMMAND");
    let test_row = row_containing(&machine, "TEST     COM");
    let testfile_row = row_containing(&machine, "TESTFILE TXT");

    assert!(
        subdir_row < command_row,
        "DIR should list directories before files by default"
    );
    assert!(
        command_row < test_row,
        "DIR should sort files by name by default"
    );
    assert!(
        test_row < testfile_row,
        "DIR should keep TEST.COM before TESTFILE.TXT by default"
    );
}

#[test]
fn dir_attr_filter_dirs_only() {
    let mut machine = boot_hle_with_floppy();
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    // Create a subdirectory first
    type_string(&mut machine.bus, b"MD SUBDIR\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);

    // /AD shows only directories
    type_string(&mut machine.bus, b"DIR /AD\r");
    run_until_prompt(&mut machine);

    let subdir = [0x0053, 0x0055, 0x0042, 0x0044, 0x0049, 0x0052]; // "SUBDIR"
    assert!(
        find_string_in_text_vram(&machine.bus, &subdir),
        "DIR /AD should show SUBDIR"
    );

    // Regular files should NOT appear
    let testfile_txt = [
        0x0054, 0x0045, 0x0053, 0x0054, 0x0046, 0x0049, 0x004C, 0x0045, 0x0020, 0x0054, 0x0058,
        0x0054,
    ]; // "TESTFILE TXT"
    assert!(
        !find_string_in_text_vram(&machine.bus, &testfile_txt),
        "DIR /AD should not show regular files"
    );
}

#[test]
fn dir_recursive() {
    let mut machine = boot_hle_with_floppy();
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    // Create subdirectory and copy a file into it
    type_string(&mut machine.bus, b"MD SUB\r");
    run_until_prompt(&mut machine);

    type_string_long(&mut machine, b"COPY TESTFILE.TXT SUB\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);

    // DIR /S should list files in root and in SUB
    type_string(&mut machine.bus, b"DIR /S\r");
    run_until_prompt(&mut machine);

    // Should show TESTFILE in both root and subdirectory listing
    let testfile = [
        0x0054, 0x0045, 0x0053, 0x0054, 0x0046, 0x0049, 0x004C, 0x0045,
    ]; // "TESTFILE"
    assert!(
        find_string_in_text_vram(&machine.bus, &testfile),
        "DIR /S should show TESTFILE"
    );

    // Should show the SUB directory marker too
    let sub = [0x0053, 0x0055, 0x0042]; // "SUB"
    assert!(
        find_string_in_text_vram(&machine.bus, &sub),
        "DIR /S should show SUB directory"
    );
}

#[test]
fn dir_in_subdirectory_shows_subdir_content() {
    let mut machine = boot_hle_with_floppy();
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    // Create a subdirectory and change into it.
    type_string(&mut machine.bus, b"MD SUB\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"CD SUB\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"DIR\r");
    run_until_prompt(&mut machine);

    // COMMAND.COM lives in the root directory only. It must NOT appear
    // when listing the subdirectory (regression: resolve_file_path used to
    // always start from root instead of the current directory).
    let command_com = [
        0x0043, 0x004F, 0x004D, 0x004D, 0x0041, 0x004E, 0x0044, 0x0020, 0x0020, 0x0043, 0x004F,
        0x004D,
    ]; // "COMMAND  COM"
    assert!(
        !find_string_in_text_vram(&machine.bus, &command_com),
        "DIR inside subdirectory must not list COMMAND.COM from root"
    );

    // The prompt should confirm we are in A:\SUB.
    let sub_prompt: [u16; 7] = [0x0041, 0x003A, 0x005C, 0x0053, 0x0055, 0x0042, 0x003E]; // "A:\SUB>"
    assert!(
        find_string_in_text_vram(&machine.bus, &sub_prompt),
        "Prompt should show A:\\SUB>"
    );
}

#[test]
fn dir_formats_sizes_with_commas_and_dates_as_ymd() {
    let large_file = vec![0x41; 1024];
    let expected_total_bytes = TEST_COMMAND_COM.len() + TEST_FILE_CONTENT.len() + large_file.len();
    let floppy = create_test_floppy_with_program(b"BIGFILE BIN", &large_file);
    let mut machine = boot_hle_with_floppy_image(floppy);
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"DIR\r");
    run_until_prompt(&mut machine);

    let bigfile_row = row_containing(&machine, "BIGFILE  BIN");
    let bigfile_line = text_vram_row_to_string(&machine.bus, bigfile_row);
    assert!(
        bigfile_line.contains("1,024"),
        "DIR should format file sizes with commas, line was {:?}",
        bigfile_line.trim_end()
    );
    assert!(
        bigfile_line.contains("1995-01-01"),
        "DIR should format dates as YYYY-MM-DD, line was {:?}",
        bigfile_line.trim_end()
    );
    assert!(
        bigfile_line.contains("12:00"),
        "DIR should keep the expected time field, line was {:?}",
        bigfile_line.trim_end()
    );

    let summary_row = row_containing(&machine, "file(s)");
    let summary_line = text_vram_row_to_string(&machine.bus, summary_row);
    let expected_total_bytes = format!("{} bytes", format_with_commas(expected_total_bytes));
    assert!(
        summary_line.contains(&expected_total_bytes),
        "DIR summary should format total bytes with commas, line was {:?}",
        summary_line.trim_end()
    );

    let free_row = row_containing(&machine, "bytes free");
    let free_line = text_vram_row_to_string(&machine.bus, free_row);
    assert!(
        free_line.contains(','),
        "DIR free space line should include comma separators, line was {:?}",
        free_line.trim_end()
    );
}
