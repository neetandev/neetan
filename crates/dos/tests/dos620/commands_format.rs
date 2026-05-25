use std::fs;

use crate::{file_copy_harness::*, harness::*};

#[test]
fn format_floppy_shows_complete() {
    let mut machine = boot_hle_with_two_floppies();

    type_string_long(&mut machine, b"FORMAT B:\r");
    // Wait for the confirmation prompt to appear
    machine.run_for(10_000_000);
    // Confirm with Y
    type_string(&mut machine.bus, b"Y");
    run_until_prompt(&mut machine);

    let complete = [
        0x0046, 0x006F, 0x0072, 0x006D, 0x0061, 0x0074, 0x0020, 0x0063, 0x006F, 0x006D, 0x0070,
        0x006C, 0x0065, 0x0074, 0x0065,
    ]; // "Format complete"
    assert!(
        find_string_in_text_vram(&machine.bus, &complete),
        "FORMAT should show 'Format complete'"
    );
}

#[test]
fn format_floppy_quick() {
    let mut machine = boot_hle_with_two_floppies();

    type_string_long(&mut machine, b"FORMAT B: /Q\r");
    machine.run_for(10_000_000);
    type_string(&mut machine.bus, b"Y");
    run_until_prompt(&mut machine);

    let complete = [
        0x0046, 0x006F, 0x0072, 0x006D, 0x0061, 0x0074, 0x0020, 0x0063, 0x006F, 0x006D, 0x0070,
        0x006C, 0x0065, 0x0074, 0x0065,
    ]; // "Format complete"
    assert!(
        find_string_in_text_vram(&machine.bus, &complete),
        "FORMAT /Q should show 'Format complete'"
    );

    // Verify the disk is mountable (DIR B: shows directory header)
    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"DIR B:\r");
    run_until_prompt(&mut machine);

    // "Directory of" should appear (volume mounted successfully)
    let directory_of = [
        0x0044, 0x0069, 0x0072, 0x0065, 0x0063, 0x0074, 0x006F, 0x0072, 0x0079, 0x0020, 0x006F,
        0x0066,
    ]; // "Directory of"
    assert!(
        find_string_in_text_vram(&machine.bus, &directory_of),
        "DIR B: after FORMAT /Q should show 'Directory of' (volume mounts successfully)"
    );
}

#[test]
fn format_no_drive_argument() {
    let mut machine = boot_hle_with_floppy();

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"FORMAT\r");
    run_until_prompt(&mut machine);

    let help = [
        0x0046, 0x004F, 0x0052, 0x004D, 0x0041, 0x0054, 0x0020, 0x0064, 0x0072, 0x0069, 0x0076,
        0x0065, 0x003A,
    ]; // "FORMAT drive:"
    assert!(
        find_string_in_text_vram(&machine.bus, &help),
        "FORMAT with no args should show help text"
    );
}

#[test]
fn format_floppy_then_dir() {
    let mut machine = boot_hle_with_two_floppies();

    // Full format B:
    type_string_long(&mut machine, b"FORMAT B:\r");
    machine.run_for(10_000_000);
    type_string(&mut machine.bus, b"Y");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);

    // DIR B: should work (volume remounts successfully)
    type_string(&mut machine.bus, b"DIR B:\r");
    run_until_prompt(&mut machine);

    // "Directory of" should appear (volume mounted successfully)
    let directory_of = [
        0x0044, 0x0069, 0x0072, 0x0065, 0x0063, 0x0074, 0x006F, 0x0072, 0x0079, 0x0020, 0x006F,
        0x0066,
    ]; // "Directory of"
    assert!(
        find_string_in_text_vram(&machine.bus, &directory_of),
        "DIR B: after FORMAT should show 'Directory of' (volume remounts)"
    );
}

#[test]
fn format_parsed_empty_d88_floppy_then_dir() {
    let floppy_a = create_test_floppy();
    let floppy_b = create_parsed_empty_d88_floppy();
    let mut machine = boot_hle_with_two_floppy_images(floppy_a, floppy_b);

    type_string_long(&mut machine, b"FORMAT B:\r");
    machine.run_for(10_000_000);
    type_string(&mut machine.bus, b"Y");
    run_until_prompt(&mut machine);

    let complete = [
        0x0046, 0x006F, 0x0072, 0x006D, 0x0061, 0x0074, 0x0020, 0x0063, 0x006F, 0x006D, 0x0070,
        0x006C, 0x0065, 0x0074, 0x0065,
    ]; // "Format complete"
    assert!(
        find_string_in_text_vram(&machine.bus, &complete),
        "FORMAT should show 'Format complete' for a parsed empty D88 floppy"
    );

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"DIR B:\r");
    run_until_prompt(&mut machine);

    let directory_of = [
        0x0044, 0x0069, 0x0072, 0x0065, 0x0063, 0x0074, 0x006F, 0x0072, 0x0079, 0x0020, 0x006F,
        0x0066,
    ]; // "Directory of"
    assert!(
        find_string_in_text_vram(&machine.bus, &directory_of),
        "DIR B: after FORMAT should show 'Directory of' for a parsed empty D88 floppy"
    );
}

#[test]
fn format_sasi_hdd_then_dir() {
    let mut machine = boot_hle_with_empty_sasi_hdd();

    // HDD is drive A: (no floppies present)
    type_string_long(&mut machine, b"FORMAT A:\r");
    machine.run_for(10_000_000);
    type_string(&mut machine.bus, b"Y");
    run_until_prompt(&mut machine);

    let complete = [
        0x0046, 0x006F, 0x0072, 0x006D, 0x0061, 0x0074, 0x0020, 0x0063, 0x006F, 0x006D, 0x0070,
        0x006C, 0x0065, 0x0074, 0x0065,
    ]; // "Format complete"
    assert!(
        find_string_in_text_vram(&machine.bus, &complete),
        "FORMAT on SASI HDD should show 'Format complete'"
    );

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"DIR A:\r");
    run_until_prompt(&mut machine);

    let directory_of = [
        0x0044, 0x0069, 0x0072, 0x0065, 0x0063, 0x0074, 0x006F, 0x0072, 0x0079, 0x0020, 0x006F,
        0x0066,
    ]; // "Directory of"
    assert!(
        find_string_in_text_vram(&machine.bus, &directory_of),
        "DIR A: after FORMAT on SASI HDD should show 'Directory of'"
    );
}

#[test]
fn format_ide_hdd_then_dir() {
    let mut machine = boot_hle_with_empty_ide_hdd();

    // HDD is drive A: (no floppies present)
    type_string_long_ap(&mut machine, b"FORMAT A:\r");
    machine.run_for(10_000_000);
    type_string(&mut machine.bus, b"Y");
    run_until_prompt_ap(&mut machine);

    let complete = [
        0x0046, 0x006F, 0x0072, 0x006D, 0x0061, 0x0074, 0x0020, 0x0063, 0x006F, 0x006D, 0x0070,
        0x006C, 0x0065, 0x0074, 0x0065,
    ]; // "Format complete"
    assert!(
        find_string_in_text_vram(&machine.bus, &complete),
        "FORMAT on IDE HDD should show 'Format complete'"
    );

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt_ap(&mut machine);

    type_string(&mut machine.bus, b"DIR A:\r");
    run_until_prompt_ap(&mut machine);

    let directory_of = [
        0x0044, 0x0069, 0x0072, 0x0065, 0x0063, 0x0074, 0x006F, 0x0072, 0x0079, 0x0020, 0x006F,
        0x0066,
    ]; // "Directory of"
    assert!(
        find_string_in_text_vram(&machine.bus, &directory_of),
        "DIR A: after FORMAT on IDE HDD should show 'Directory of'"
    );
}

#[test]
fn format_hdd_then_copy_file() {
    let mut machine = boot_hle_with_empty_ide_hdd();

    type_string_long_ap(&mut machine, b"FORMAT A:\r");
    machine.run_for(10_000_000);
    type_string(&mut machine.bus, b"Y");
    run_until_prompt_ap(&mut machine);

    // Create a file on the formatted disk
    type_string_long_ap(&mut machine, b"ECHO test > A:\\TEST.TXT\r");
    run_until_prompt_ap(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt_ap(&mut machine);

    type_string(&mut machine.bus, b"DIR A:\r");
    run_until_prompt_ap(&mut machine);

    // "TEST" should appear in directory listing
    let test_txt = [0x0054, 0x0045, 0x0053, 0x0054]; // "TEST"
    assert!(
        find_string_in_text_vram(&machine.bus, &test_txt),
        "DIR A: should show TEST file after creating it on formatted HDD"
    );
}

#[test]
fn format_sasi_hdd_supports_multicluster_file_copy_after_format() {
    let source_bytes = prng_bytes(RANDOM_FILE_SIZE);
    let floppy = create_random_file_floppy(&source_bytes);

    let hdd_path = make_temp_hdd_path("format-repro");
    let hdd = create_empty_hdd(256);
    fs::write(&hdd_path, hdd.to_bytes()).expect("write temp HDD image");

    let mut machine = boot_hle_with_temp_hdd_and_floppy(&hdd_path, floppy);

    type_string_long(&mut machine, b"FORMAT A:\r");
    machine.run_for(10_000_000);
    type_string(&mut machine.bus, b"Y");
    run_until_prompt(&mut machine);

    type_string_long(&mut machine, b"COPY C:RAND.BIN A:\\\r");
    run_until_prompt(&mut machine);
    machine.bus.flush_hdd(0);

    let copied_bytes = extract_root_file(&hdd_path, &RANDOM_FILE_FCB)
        .expect("destination file should be readable");
    let mismatches = mismatch_offsets(&source_bytes, &copied_bytes, 8);

    assert_eq!(
        copied_bytes.len(),
        source_bytes.len(),
        "formatted HDD should preserve copied file size"
    );
    assert!(
        mismatches.is_empty(),
        "formatted HDD corrupted copied data: first mismatches at {mismatches:?}"
    );
    assert_eq!(
        copied_bytes, source_bytes,
        "formatted HDD should preserve copied file contents"
    );
}
