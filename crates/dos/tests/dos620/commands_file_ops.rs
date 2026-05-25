use common::JisChar;

use crate::harness::*;

#[test]
fn type_file_content() {
    let mut machine = boot_hle_with_floppy();
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);

    type_string_long(&mut machine, b"TYPE TESTFILE.TXT\r");
    run_until_prompt(&mut machine);

    let hello = [
        0x0048, 0x0045, 0x004C, 0x004C, 0x004F, 0x0020, 0x0057, 0x004F, 0x0052, 0x004C, 0x0044,
    ]; // "HELLO WORLD"
    assert!(
        find_string_in_text_vram(&machine.bus, &hello),
        "TYPE should display 'HELLO WORLD'"
    );
}

#[test]
fn type_nonexistent() {
    let mut machine = boot_hle_with_floppy();
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);

    type_string_long(&mut machine, b"TYPE NOFILE.TXT\r");
    run_until_prompt(&mut machine);

    let not_found = [
        0x006E, 0x006F, 0x0074, 0x0020, 0x0066, 0x006F, 0x0075, 0x006E, 0x0064,
    ]; // "not found"
    assert!(
        find_string_in_text_vram(&machine.bus, &not_found),
        "TYPE of nonexistent file should show 'not found'"
    );
}

#[test]
fn type_shift_jis_file_content() {
    let floppy = create_test_floppy_with_program(b"JAPAN   TXT", b"\x82\xA0\x82\xA2\r\n");
    let mut machine = boot_hle_with_floppy_image(floppy);
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);

    type_string_long(&mut machine, b"TYPE JAPAN.TXT\r");
    run_until_prompt(&mut machine);

    let chars = [JisChar::from_u16(0x2422), JisChar::from_u16(0x2424)];
    assert!(
        find_jis_string_in_text_vram(&machine.bus, &chars),
        "TYPE should display Shift-JIS text as full-width JIS glyphs"
    );
}

#[test]
fn del_file() {
    let mut machine = boot_hle_with_floppy();
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    type_string_long(&mut machine, b"DEL TESTFILE.TXT\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"DIR\r");
    run_until_prompt(&mut machine);

    let testfile_txt = [
        0x0054, 0x0045, 0x0053, 0x0054, 0x0046, 0x0049, 0x004C, 0x0045, 0x0020, 0x0054, 0x0058,
        0x0054,
    ]; // "TESTFILE TXT"
    assert!(
        !find_string_in_text_vram(&machine.bus, &testfile_txt),
        "TESTFILE.TXT should be gone after DEL"
    );

    let command = [0x0043, 0x004F, 0x004D, 0x004D, 0x0041, 0x004E, 0x0044]; // "COMMAND"
    assert!(
        find_string_in_text_vram(&machine.bus, &command),
        "COMMAND.COM should still exist after DEL TESTFILE.TXT"
    );
}

#[test]
fn del_wildcard_deletes_large_directory() {
    let hdd = create_test_hdd_with_many_txt_files(256, 300);
    let mut machine = boot_hle_with_forced_os(None, Some(hdd));

    type_string(&mut machine.bus, b"C:\r");
    run_until_prompt(&mut machine);

    type_string_long(&mut machine, b"DEL *.*\r");
    machine.run_for(50_000_000);
    type_string(&mut machine.bus, b"Y");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);
    type_string_long(&mut machine, b"DIR F0000000.TXT\r");
    run_until_prompt(&mut machine);

    let file_name = [
        0x0046, 0x0030, 0x0030, 0x0030, 0x0030, 0x0030, 0x0030, 0x0030, 0x0020, 0x0054, 0x0058,
        0x0054,
    ]; // "F0000000 TXT"
    assert!(
        !find_string_in_text_vram(&machine.bus, &file_name),
        "DEL *.* should remove generated files even in large directories"
    );
}

#[test]
fn md_rd_directory() {
    let mut machine = boot_hle_with_floppy();
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"MD TESTDIR\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);
    type_string(&mut machine.bus, b"DIR\r");
    run_until_prompt(&mut machine);

    let testdir = [0x0054, 0x0045, 0x0053, 0x0054, 0x0044, 0x0049, 0x0052]; // "TESTDIR"
    assert!(
        find_string_in_text_vram(&machine.bus, &testdir),
        "MD TESTDIR should create a directory visible in DIR"
    );

    let dir_marker = [0x003C, 0x0044, 0x0049, 0x0052, 0x003E]; // "<DIR>"
    assert!(
        find_string_in_text_vram(&machine.bus, &dir_marker),
        "TESTDIR should show <DIR> marker"
    );

    type_string(&mut machine.bus, b"RD TESTDIR\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);
    type_string(&mut machine.bus, b"DIR\r");
    run_until_prompt(&mut machine);

    let testdir_padded = [
        0x0054, 0x0045, 0x0053, 0x0054, 0x0044, 0x0049, 0x0052, 0x0020, 0x0020, 0x0020,
    ]; // "TESTDIR   "
    assert!(
        !find_string_in_text_vram(&machine.bus, &testdir_padded),
        "TESTDIR should be gone after RD"
    );
}

/// Fixed test time: 1995-01-01 12:00:00, Sunday.
fn test_time() -> [u8; 6] {
    [0x95, 0x10, 0x01, 0x12, 0x00, 0x00]
}

#[test]
fn date_command() {
    let mut machine = boot_hle_with_time(Some(test_time));
    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"DATE\r");
    run_until_prompt(&mut machine);

    let year = [0x0031, 0x0039, 0x0039, 0x0035]; // "1995"
    assert!(
        find_string_in_text_vram(&machine.bus, &year),
        "DATE should display year '1995'"
    );
}

#[test]
fn time_command() {
    let mut machine = boot_hle_with_time(Some(test_time));
    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"TIME\r");
    run_until_prompt(&mut machine);

    let time = [0x0031, 0x0032, 0x003A, 0x0030, 0x0030]; // "12:00"
    assert!(
        find_string_in_text_vram(&machine.bus, &time),
        "TIME should display '12:00'"
    );
}

#[test]
fn ren_file() {
    let mut machine = boot_hle_with_floppy();
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    type_string_long(&mut machine, b"REN TESTFILE.TXT RENAMED.TXT\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);
    type_string(&mut machine.bus, b"DIR\r");
    run_until_prompt(&mut machine);

    let renamed = [0x0052, 0x0045, 0x004E, 0x0041, 0x004D, 0x0045, 0x0044]; // "RENAMED"
    assert!(
        find_string_in_text_vram(&machine.bus, &renamed),
        "RENAMED.TXT should appear after REN"
    );

    let testfile_txt = [
        0x0054, 0x0045, 0x0053, 0x0054, 0x0046, 0x0049, 0x004C, 0x0045, 0x0020, 0x0054, 0x0058,
        0x0054,
    ]; // "TESTFILE TXT"
    assert!(
        !find_string_in_text_vram(&machine.bus, &testfile_txt),
        "TESTFILE.TXT should be gone after REN"
    );
}

#[test]
fn ren_wildcard() {
    let mut machine = boot_hle_with_floppy();
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    type_string_long(&mut machine, b"COPY TESTFILE.TXT SECOND.TXT\r");
    run_until_prompt(&mut machine);

    type_string_long(&mut machine, b"REN *.TXT *.BAK\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);
    type_string(&mut machine.bus, b"DIR\r");
    run_until_prompt(&mut machine);

    let bak = [0x0042, 0x0041, 0x004B]; // "BAK"
    assert!(
        find_string_in_text_vram(&machine.bus, &bak),
        ".BAK files should appear after REN *.TXT *.BAK"
    );
}

#[test]
fn del_with_prompt_yes() {
    let mut machine = boot_hle_with_floppy();
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    // DEL /P should prompt per file; answer Y to delete
    type_string_long(&mut machine, b"DEL /P TESTFILE.TXT\r");
    // Wait for the prompt to appear, then press Y
    machine.run_for(50_000_000);
    type_string(&mut machine.bus, b"Y");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);
    type_string(&mut machine.bus, b"DIR\r");
    run_until_prompt(&mut machine);

    let testfile_txt = [
        0x0054, 0x0045, 0x0053, 0x0054, 0x0046, 0x0049, 0x004C, 0x0045, 0x0020, 0x0054, 0x0058,
        0x0054,
    ]; // "TESTFILE TXT"
    assert!(
        !find_string_in_text_vram(&machine.bus, &testfile_txt),
        "TESTFILE.TXT should be gone after DEL /P + Y"
    );
}

#[test]
fn del_with_prompt_no() {
    let mut machine = boot_hle_with_floppy();
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    // DEL /P should prompt per file; answer N to skip
    type_string_long(&mut machine, b"DEL /P TESTFILE.TXT\r");
    machine.run_for(50_000_000);
    type_string(&mut machine.bus, b"N");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);
    type_string(&mut machine.bus, b"DIR\r");
    run_until_prompt(&mut machine);

    let testfile_txt = [
        0x0054, 0x0045, 0x0053, 0x0054, 0x0046, 0x0049, 0x004C, 0x0045, 0x0020, 0x0054, 0x0058,
        0x0054,
    ]; // "TESTFILE TXT"
    assert!(
        find_string_in_text_vram(&machine.bus, &testfile_txt),
        "TESTFILE.TXT should still exist after DEL /P + N"
    );
}

#[test]
fn type_reads_from_cdrom() {
    let mut machine = boot_hle_with_cdrom_image(create_test_cdimage());
    type_string(&mut machine.bus, b"Q:\r");
    run_until_prompt_ap(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt_ap(&mut machine);
    type_string_long_ap(&mut machine, b"TYPE README.TXT\r");
    run_until_prompt_ap(&mut machine);

    let neetan = [
        0x004E, 0x0045, 0x0045, 0x0054, 0x0041, 0x004E, 0x0020, 0x0043, 0x0044, 0x0020, 0x0052,
        0x0045, 0x0041, 0x0044, 0x004D, 0x0045,
    ]; // "NEETAN CD README"
    assert!(
        find_string_in_text_vram(&machine.bus, &neetan),
        "TYPE should display README.TXT contents from the CD-ROM"
    );
}

#[test]
fn more_reads_from_floppy() {
    let mut machine = boot_hle_with_floppy();
    type_string(&mut machine.bus, b"A:\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);
    type_string_long(&mut machine, b"MORE TESTFILE.TXT\r");
    run_until_prompt(&mut machine);

    let hello = [
        0x0048, 0x0045, 0x004C, 0x004C, 0x004F, 0x0020, 0x0057, 0x004F, 0x0052, 0x004C, 0x0044,
    ]; // "HELLO WORLD"
    assert!(
        find_string_in_text_vram(&machine.bus, &hello),
        "MORE should display 'HELLO WORLD' from a FAT floppy"
    );
}

#[test]
fn more_reads_from_cdrom() {
    let mut machine = boot_hle_with_cdrom_image(create_test_cdimage());
    type_string(&mut machine.bus, b"Q:\r");
    run_until_prompt_ap(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt_ap(&mut machine);
    type_string_long_ap(&mut machine, b"MORE README.TXT\r");
    run_until_prompt_ap(&mut machine);

    let neetan = [
        0x004E, 0x0045, 0x0045, 0x0054, 0x0041, 0x004E, 0x0020, 0x0043, 0x0044, 0x0020, 0x0052,
        0x0045, 0x0041, 0x0044, 0x004D, 0x0045,
    ]; // "NEETAN CD README"
    assert!(
        find_string_in_text_vram(&machine.bus, &neetan),
        "MORE should display README.TXT contents from the CD-ROM"
    );
}
