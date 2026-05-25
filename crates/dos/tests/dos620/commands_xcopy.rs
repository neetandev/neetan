use std::fs;

use crate::{file_copy_harness::*, harness::*};

#[test]
fn xcopy_copies_multicluster_file() {
    let source_bytes = prng_bytes(RANDOM_FILE_SIZE);
    let floppy = create_random_file_floppy(&source_bytes);

    let hdd_path = make_temp_hdd_path("xcopy-repro");
    let hdd = create_empty_hdd(256);
    fs::write(&hdd_path, hdd.to_bytes()).expect("write temp HDD image");

    let mut machine = boot_hle_with_temp_hdd_and_floppy(&hdd_path, floppy);

    type_string_long(&mut machine, b"FORMAT A:\r");
    machine.run_for(10_000_000);
    type_string(&mut machine.bus, b"Y");
    run_until_prompt(&mut machine);

    type_string_long(&mut machine, b"XCOPY C:RAND.BIN A:\\\r");
    run_until_prompt(&mut machine);
    machine.bus.flush_hdd(0);

    let copied = [0x0063, 0x006F, 0x0070, 0x0069, 0x0065, 0x0064]; // "copied"
    assert!(
        find_string_in_text_vram(&machine.bus, &copied),
        "XCOPY should report success before the on-disk bytes are checked"
    );

    let copied_bytes = extract_root_file(&hdd_path, &RANDOM_FILE_FCB)
        .expect("destination file should be readable");
    let mismatches = mismatch_offsets(&source_bytes, &copied_bytes, 8);

    assert_eq!(
        copied_bytes.len(),
        source_bytes.len(),
        "copied size mismatch"
    );
    assert!(
        mismatches.is_empty(),
        "XCOPY corrupted data: first mismatches at {mismatches:?}"
    );
    assert_eq!(
        copied_bytes, source_bytes,
        "XCOPY should preserve file contents"
    );
}

#[test]
fn xcopy_broken_source_chain_reports_read_error() {
    let broken_name = *b"BROKEN  BIN";
    let floppy = create_broken_chain_floppy_with_name(&broken_name, 4096);

    let hdd_path = make_temp_hdd_path("xcopy-broken-chain");
    let hdd = create_empty_hdd(256);
    fs::write(&hdd_path, hdd.to_bytes()).expect("write temp HDD image");

    let mut machine = boot_hle_with_temp_hdd_and_floppy(&hdd_path, floppy);

    type_string_long(&mut machine, b"FORMAT A:\r");
    machine.run_for(10_000_000);
    type_string(&mut machine.bus, b"Y");
    run_until_prompt(&mut machine);

    type_string_long(&mut machine, b"XCOPY C:BROKEN.BIN A:\\\r");
    run_until_prompt(&mut machine);
    machine.bus.flush_hdd(0);

    let read_error = [
        0x0052, 0x0065, 0x0061, 0x0064, 0x0020, 0x0065, 0x0072, 0x0072, 0x006F, 0x0072,
    ]; // "Read error"
    assert!(
        find_string_in_text_vram(&machine.bus, &read_error),
        "XCOPY should report a read error for a truncated source chain"
    );
    assert!(
        extract_root_file(&hdd_path, &broken_name).is_err(),
        "XCOPY should not create a destination file after a read error"
    );
}

#[test]
fn xcopy_recursive_wildcard_path_copies_directory_tree() {
    let hdd_path = make_temp_hdd_path("xcopy-wildcard-tree");
    let hdd = create_empty_hdd(256);
    fs::write(&hdd_path, hdd.to_bytes()).expect("write temp HDD image");

    let mut machine = boot_hle_with_temp_hdd_and_floppy(&hdd_path, create_test_floppy());

    type_string_long(&mut machine, b"FORMAT A:\r");
    machine.run_for(10_000_000);
    type_string(&mut machine.bus, b"Y");
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"C:\r");
    run_until_prompt(&mut machine);

    type_string_long(&mut machine, b"MD YOURFOLDER\r");
    run_until_prompt(&mut machine);
    type_string_long(&mut machine, b"MD YOURFOLDER\\SUBONE\r");
    run_until_prompt(&mut machine);
    type_string_long(&mut machine, b"MD YOURFOLDER\\SUBTWO\r");
    run_until_prompt(&mut machine);
    type_string_long(&mut machine, b"MD YOURFOLDER\\EMPTY\r");
    run_until_prompt(&mut machine);

    type_string_long(&mut machine, b"COPY TESTFILE.TXT YOURFOLDER\\ROOT1.TXT\r");
    run_until_prompt(&mut machine);
    type_string_long(&mut machine, b"COPY TESTFILE.TXT YOURFOLDER\\ROOT2.TXT\r");
    run_until_prompt(&mut machine);
    type_string_long(
        &mut machine,
        b"COPY TESTFILE.TXT YOURFOLDER\\SUBONE\\CHILD1.TXT\r",
    );
    run_until_prompt(&mut machine);
    type_string_long(
        &mut machine,
        b"COPY TESTFILE.TXT YOURFOLDER\\SUBTWO\\CHILD2.TXT\r",
    );
    run_until_prompt(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);
    type_string_long(&mut machine, b"XCOPY C:\\YOURFOLDER\\*.* A:\\ /S /E /Y\r");
    run_until_prompt(&mut machine);

    let file_not_found = [
        0x0046, 0x0069, 0x006C, 0x0065, 0x0020, 0x006E, 0x006F, 0x0074, 0x0020, 0x0066, 0x006F,
        0x0075, 0x006E, 0x0064,
    ]; // "File not found"
    assert!(
        !find_string_in_text_vram(&machine.bus, &file_not_found),
        "XCOPY should resolve wildcard paths rooted in a subdirectory"
    );

    let copied = [0x0063, 0x006F, 0x0070, 0x0069, 0x0065, 0x0064]; // "copied"
    assert!(
        find_string_in_text_vram(&machine.bus, &copied),
        "XCOPY should report that files were copied"
    );

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);
    type_string_long(&mut machine, b"DIR A:\\\r");
    run_until_prompt(&mut machine);

    let root1 = [0x0052, 0x004F, 0x004F, 0x0054, 0x0031]; // "ROOT1"
    let root2 = [0x0052, 0x004F, 0x004F, 0x0054, 0x0032]; // "ROOT2"
    let subone = [0x0053, 0x0055, 0x0042, 0x004F, 0x004E, 0x0045]; // "SUBONE"
    let subtwo = [0x0053, 0x0055, 0x0042, 0x0054, 0x0057, 0x004F]; // "SUBTWO"
    let empty = [0x0045, 0x004D, 0x0050, 0x0054, 0x0059]; // "EMPTY"
    assert!(find_string_in_text_vram(&machine.bus, &root1));
    assert!(find_string_in_text_vram(&machine.bus, &root2));
    assert!(find_string_in_text_vram(&machine.bus, &subone));
    assert!(find_string_in_text_vram(&machine.bus, &subtwo));
    assert!(find_string_in_text_vram(&machine.bus, &empty));

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);
    type_string_long(&mut machine, b"DIR A:\\SUBONE\r");
    run_until_prompt(&mut machine);

    let child1 = [0x0043, 0x0048, 0x0049, 0x004C, 0x0044, 0x0031]; // "CHILD1"
    assert!(
        find_string_in_text_vram(&machine.bus, &child1),
        "XCOPY should copy nested files into the destination subdirectory"
    );

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);
    type_string_long(&mut machine, b"DIR A:\\SUBTWO\r");
    run_until_prompt(&mut machine);

    let child2 = [0x0043, 0x0048, 0x0049, 0x004C, 0x0044, 0x0032]; // "CHILD2"
    assert!(
        find_string_in_text_vram(&machine.bus, &child2),
        "XCOPY should copy files from every matching subdirectory"
    );
}

#[test]
fn xcopy_copies_directory_tree_from_cdrom_source() {
    let mut machine = boot_hle_with_cdrom_image(create_test_cdimage_with_xcopy_tree());
    machine.bus.insert_floppy(0, create_blank_floppy(), None);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt_ap(&mut machine);
    type_string_long_ap(&mut machine, b"DIR Q:\\YOURFOLD\\SUBONE\r");
    run_until_prompt_ap(&mut machine);

    let source_child1 = [0x0043, 0x0048, 0x0049, 0x004C, 0x0044, 0x0031]; // "CHILD1"
    assert!(
        find_string_in_text_vram(&machine.bus, &source_child1),
        "the synthetic CD-ROM source should expose CHILD1.TXT before XCOPY runs"
    );

    type_string_long_ap(&mut machine, b"FORMAT A:\r");
    machine.run_for(10_000_000);
    type_string(&mut machine.bus, b"Y");
    run_until_prompt_ap(&mut machine);

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt_ap(&mut machine);
    type_string_long_ap(&mut machine, b"XCOPY Q:\\YOURFOLD\\*.* A:\\ /S /E /Y\r");
    run_until_prompt_ap(&mut machine);

    let file_not_found = [
        0x0046, 0x0069, 0x006C, 0x0065, 0x0020, 0x006E, 0x006F, 0x0074, 0x0020, 0x0066, 0x006F,
        0x0075, 0x006E, 0x0064,
    ]; // "File not found"
    assert!(
        !find_string_in_text_vram(&machine.bus, &file_not_found),
        "XCOPY should resolve wildcard directories on a CD-ROM source"
    );

    let read_error = [
        0x0052, 0x0065, 0x0061, 0x0064, 0x0020, 0x0065, 0x0072, 0x0072, 0x006F, 0x0072,
    ]; // "Read error"
    assert!(
        !find_string_in_text_vram(&machine.bus, &read_error),
        "XCOPY should not hit a read error while copying from the CD-ROM source"
    );

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt_ap(&mut machine);
    type_string_long_ap(&mut machine, b"DIR A:\\\r");
    run_until_prompt_ap(&mut machine);

    let root1 = [0x0052, 0x004F, 0x004F, 0x0054, 0x0031]; // "ROOT1"
    let root2 = [0x0052, 0x004F, 0x004F, 0x0054, 0x0032]; // "ROOT2"
    let subone = [0x0053, 0x0055, 0x0042, 0x004F, 0x004E, 0x0045]; // "SUBONE"
    let subtwo = [0x0053, 0x0055, 0x0042, 0x0054, 0x0057, 0x004F]; // "SUBTWO"
    let empty = [0x0045, 0x004D, 0x0050, 0x0054, 0x0059]; // "EMPTY"
    assert!(find_string_in_text_vram(&machine.bus, &root1));
    assert!(find_string_in_text_vram(&machine.bus, &root2));
    assert!(find_string_in_text_vram(&machine.bus, &subone));
    assert!(find_string_in_text_vram(&machine.bus, &subtwo));
    assert!(find_string_in_text_vram(&machine.bus, &empty));

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt_ap(&mut machine);
    type_string_long_ap(&mut machine, b"DIR A:\\SUBONE\r");
    run_until_prompt_ap(&mut machine);

    let child1 = [0x0043, 0x0048, 0x0049, 0x004C, 0x0044, 0x0031]; // "CHILD1"
    assert!(
        find_string_in_text_vram(&machine.bus, &child1),
        "XCOPY should copy nested files from the CD-ROM source"
    );

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt_ap(&mut machine);
    type_string_long_ap(&mut machine, b"DIR A:\\SUBTWO\r");
    run_until_prompt_ap(&mut machine);

    let child2 = [0x0043, 0x0048, 0x0049, 0x004C, 0x0044, 0x0032]; // "CHILD2"
    assert!(
        find_string_in_text_vram(&machine.bus, &child2),
        "XCOPY should copy every matching file from CD-ROM subdirectories"
    );

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt_ap(&mut machine);
    type_string_long_ap(&mut machine, b"DEL A:\\ROOT1.TXT\r");
    run_until_prompt_ap(&mut machine);

    let access_denied = [
        0x0041, 0x0063, 0x0063, 0x0065, 0x0073, 0x0073, 0x0020, 0x0064, 0x0065, 0x006E, 0x0069,
        0x0065, 0x0064,
    ]; // "Access denied"
    assert!(
        !find_string_in_text_vram(&machine.bus, &access_denied),
        "files copied from CD-ROM should not inherit the read-only attribute"
    );

    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt_ap(&mut machine);
    type_string_long_ap(&mut machine, b"DIR A:\\\r");
    run_until_prompt_ap(&mut machine);

    let root1_padded = [
        0x0052, 0x004F, 0x004F, 0x0054, 0x0031, 0x0020, 0x0020, 0x0020, 0x0054, 0x0058, 0x0054,
    ]; // "ROOT1   TXT"
    assert!(
        !find_string_in_text_vram(&machine.bus, &root1_padded),
        "the copied destination file should be deletable after XCOPY from CD-ROM"
    );
}
