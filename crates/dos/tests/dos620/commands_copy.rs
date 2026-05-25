use std::fs;

use crate::{file_copy_harness::*, harness::*};

fn run_command(machine: &mut machine::Pc9801Ra, command: &[u8]) {
    type_string_long(machine, command);
    run_until_prompt(machine);
}

fn run_command_ap(machine: &mut machine::Pc9821Ap, command: &[u8]) {
    type_string_long_ap(machine, command);
    run_until_prompt_ap(machine);
}

fn format_drive_a(machine: &mut machine::Pc9801Ra) {
    type_string_long(machine, b"FORMAT A:\r");
    machine.run_for(10_000_000);
    type_string(&mut machine.bus, b"Y");
    run_until_prompt(machine);
}

fn format_drive_a_ap(machine: &mut machine::Pc9821Ap) {
    type_string_long_ap(machine, b"FORMAT A:\r");
    machine.run_for(10_000_000);
    type_string(&mut machine.bus, b"Y");
    run_until_prompt_ap(machine);
}

#[test]
fn copy_single_file() {
    let mut machine = boot_hle_with_floppy();
    run_command(&mut machine, b"A:\r");

    run_command(&mut machine, b"COPY TESTFILE.TXT COPIED.TXT\r");

    run_command(&mut machine, b"CLS\r");

    run_command(&mut machine, b"TYPE COPIED.TXT\r");

    let hello = [
        0x0048, 0x0045, 0x004C, 0x004C, 0x004F, 0x0020, 0x0057, 0x004F, 0x0052, 0x004C, 0x0044,
    ]; // "HELLO WORLD"
    assert!(
        find_string_in_text_vram(&machine.bus, &hello),
        "TYPE of copied file should show 'HELLO WORLD'"
    );
}

#[test]
fn copy_file_count() {
    let mut machine = boot_hle_with_floppy();
    run_command(&mut machine, b"A:\r");

    run_command(&mut machine, b"CLS\r");

    run_command(&mut machine, b"COPY TESTFILE.TXT COPY2.TXT\r");

    let copied = [0x0063, 0x006F, 0x0070, 0x0069, 0x0065, 0x0064]; // "copied"
    assert!(
        find_string_in_text_vram(&machine.bus, &copied),
        "COPY should show 'copied' message"
    );
}

#[test]
fn copy_nonexistent_source() {
    let mut machine = boot_hle_with_floppy();
    run_command(&mut machine, b"A:\r");

    run_command(&mut machine, b"CLS\r");

    run_command(&mut machine, b"COPY NOFILE.TXT DEST.TXT\r");

    let not_found = [
        0x006E, 0x006F, 0x0074, 0x0020, 0x0066, 0x006F, 0x0075, 0x006E, 0x0064,
    ]; // "not found"
    assert!(
        find_string_in_text_vram(&machine.bus, &not_found),
        "COPY of nonexistent file should show error"
    );
}

#[test]
fn copy_with_verify() {
    let mut machine = boot_hle_with_floppy();
    run_command(&mut machine, b"A:\r");

    // COPY /V should copy and verify
    run_command(&mut machine, b"COPY /V TESTFILE.TXT VERIFIED.TXT\r");

    let copied = [0x0063, 0x006F, 0x0070, 0x0069, 0x0065, 0x0064]; // "copied"
    assert!(
        find_string_in_text_vram(&machine.bus, &copied),
        "COPY /V should show 'copied' message"
    );

    // Verify the file content is correct
    run_command(&mut machine, b"CLS\r");

    run_command(&mut machine, b"TYPE VERIFIED.TXT\r");

    let hello = [
        0x0048, 0x0045, 0x004C, 0x004C, 0x004F, 0x0020, 0x0057, 0x004F, 0x0052, 0x004C, 0x0044,
    ]; // "HELLO WORLD"
    assert!(
        find_string_in_text_vram(&machine.bus, &hello),
        "Verified copy should contain 'HELLO WORLD'"
    );
}

#[test]
fn copy_overwrite_with_y_flag() {
    let mut machine = boot_hle_with_floppy();
    run_command(&mut machine, b"A:\r");

    // First copy
    run_command(&mut machine, b"COPY TESTFILE.TXT DEST.TXT\r");

    run_command(&mut machine, b"CLS\r");

    // Copy again with /Y -- should overwrite without prompting
    run_command(&mut machine, b"COPY /Y TESTFILE.TXT DEST.TXT\r");

    let copied = [0x0063, 0x006F, 0x0070, 0x0069, 0x0065, 0x0064]; // "copied"
    assert!(
        find_string_in_text_vram(&machine.bus, &copied),
        "COPY /Y should overwrite and show 'copied'"
    );
}

#[test]
fn copy_concatenation() {
    let mut machine = boot_hle_with_floppy();
    run_command(&mut machine, b"A:\r");

    // Create a second file
    run_command(&mut machine, b"COPY TESTFILE.TXT PART2.TXT\r");

    run_command(&mut machine, b"CLS\r");

    // Concatenate: COPY TESTFILE.TXT+PART2.TXT JOINED.TXT
    run_command(&mut machine, b"COPY TESTFILE.TXT+PART2.TXT JOINED.TXT\r");

    let copied = [0x0063, 0x006F, 0x0070, 0x0069, 0x0065, 0x0064]; // "copied"
    assert!(
        find_string_in_text_vram(&machine.bus, &copied),
        "COPY with + concatenation should show 'copied'"
    );

    // Verify the joined file exists via DIR
    run_command(&mut machine, b"CLS\r");
    run_command(&mut machine, b"DIR\r");

    let joined = [0x004A, 0x004F, 0x0049, 0x004E, 0x0045, 0x0044]; // "JOINED"
    assert!(
        find_string_in_text_vram(&machine.bus, &joined),
        "JOINED.TXT should appear in DIR after concatenation"
    );
}

#[test]
fn xcopy_recursive() {
    let mut machine = boot_hle_with_floppy();
    run_command(&mut machine, b"A:\r");

    // Create source structure: SRCDIR with a file
    run_command(&mut machine, b"MD SRCDIR\r");

    run_command(&mut machine, b"COPY TESTFILE.TXT SRCDIR\r");

    // Create dest dir
    run_command(&mut machine, b"MD DSTDIR\r");

    run_command(&mut machine, b"CLS\r");

    // XCOPY /S SRCDIR DSTDIR should copy files recursively
    run_command(&mut machine, b"XCOPY SRCDIR DSTDIR\r");

    // Should show "File(s) copied"
    let copied_msg = [0x0063, 0x006F, 0x0070, 0x0069, 0x0065, 0x0064]; // "copied"
    assert!(
        find_string_in_text_vram(&machine.bus, &copied_msg),
        "XCOPY should show 'copied' message"
    );

    // Verify file is in DSTDIR
    run_command(&mut machine, b"CLS\r");
    run_command(&mut machine, b"DIR DSTDIR\r");

    let testfile = [
        0x0054, 0x0045, 0x0053, 0x0054, 0x0046, 0x0049, 0x004C, 0x0045,
    ]; // "TESTFILE"
    assert!(
        find_string_in_text_vram(&machine.bus, &testfile),
        "XCOPY should have copied TESTFILE.TXT into DSTDIR"
    );
}

#[test]
fn copy_multicluster_file_from_floppy_to_formatted_hdd_without_corruption() {
    let source_bytes = prng_bytes(RANDOM_FILE_SIZE);
    let floppy = create_random_file_floppy(&source_bytes);

    let hdd_path = make_temp_hdd_path("copy-repro");
    let hdd = create_empty_hdd(256);
    fs::write(&hdd_path, hdd.to_bytes()).expect("write temp HDD image");

    let mut machine = boot_hle_with_temp_hdd_and_floppy(&hdd_path, floppy);

    format_drive_a(&mut machine);

    run_command(&mut machine, b"COPY /V C:RAND.BIN A:\\\r");
    machine.bus.flush_hdd(0);

    let copied = [0x0063, 0x006F, 0x0070, 0x0069, 0x0065, 0x0064]; // "copied"
    assert!(
        find_string_in_text_vram(&machine.bus, &copied),
        "COPY should report success before the on-disk bytes are checked"
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
        "COPY corrupted data: first mismatches at {mismatches:?}"
    );
    assert_eq!(
        copied_bytes, source_bytes,
        "COPY should preserve file contents"
    );
}

#[test]
fn copy_explicit_destination_path_uses_basename() {
    let source_bytes = prng_bytes(RANDOM_FILE_SIZE);
    let source_name = *b"COPYONE BIN";
    let destination_name = *b"TARGET  BIN";
    let floppy = create_random_file_floppy_with_name(&source_name, &source_bytes);

    let hdd_path = make_temp_hdd_path("copy-destination-path");
    let hdd = create_empty_hdd(256);
    fs::write(&hdd_path, hdd.to_bytes()).expect("write temp HDD image");

    let mut machine = boot_hle_with_temp_hdd_and_floppy(&hdd_path, floppy);

    format_drive_a(&mut machine);

    run_command(&mut machine, b"COPY /V C:COPYONE.BIN A:\\TARGET.BIN\r");
    machine.bus.flush_hdd(0);

    let copied_bytes = extract_root_file(&hdd_path, &destination_name)
        .expect("destination file should be created with the explicit basename");
    let mismatches = mismatch_offsets(&source_bytes, &copied_bytes, 8);

    assert_eq!(
        copied_bytes.len(),
        source_bytes.len(),
        "copy with explicit destination path should preserve file size"
    );
    assert!(
        mismatches.is_empty(),
        "copy with explicit destination path mismatches at {mismatches:?}"
    );
    assert_eq!(
        copied_bytes, source_bytes,
        "copy with explicit destination path should preserve file contents"
    );
}

#[test]
fn copy_broken_source_chain_reports_read_error() {
    let broken_name = *b"BROKEN  BIN";
    let floppy = create_broken_chain_floppy_with_name(&broken_name, 4096);

    let hdd_path = make_temp_hdd_path("copy-broken-chain");
    let hdd = create_empty_hdd(256);
    fs::write(&hdd_path, hdd.to_bytes()).expect("write temp HDD image");

    let mut machine = boot_hle_with_temp_hdd_and_floppy(&hdd_path, floppy);

    format_drive_a(&mut machine);

    run_command(&mut machine, b"COPY C:BROKEN.BIN A:\\BROKEN.BIN\r");
    machine.bus.flush_hdd(0);

    let read_error = [
        0x0052, 0x0065, 0x0061, 0x0064, 0x0020, 0x0065, 0x0072, 0x0072, 0x006F, 0x0072,
    ]; // "Read error"
    assert!(
        find_string_in_text_vram(&machine.bus, &read_error),
        "COPY should report a read error for a truncated source chain"
    );
    assert!(
        extract_root_file(&hdd_path, &broken_name).is_err(),
        "COPY should not create a destination file after a read error"
    );
}

#[test]
fn copy_from_cdrom_to_floppy() {
    let mut machine = boot_hle_with_cdrom_image(create_test_cdimage());
    machine.bus.insert_floppy(0, create_blank_floppy(), None);

    format_drive_a_ap(&mut machine);

    run_command_ap(&mut machine, b"CLS\r");
    run_command_ap(&mut machine, b"COPY Q:\\README.TXT A:\\README.TXT\r");

    let copied = [0x0063, 0x006F, 0x0070, 0x0069, 0x0065, 0x0064]; // "copied"
    assert!(
        find_string_in_text_vram(&machine.bus, &copied),
        "COPY from CD-ROM should report 'copied'"
    );

    run_command_ap(&mut machine, b"CLS\r");
    run_command_ap(&mut machine, b"TYPE A:\\README.TXT\r");

    let neetan = [
        0x004E, 0x0045, 0x0045, 0x0054, 0x0041, 0x004E, 0x0020, 0x0043, 0x0044, 0x0020, 0x0052,
        0x0045, 0x0041, 0x0044, 0x004D, 0x0045,
    ]; // "NEETAN CD README"
    assert!(
        find_string_in_text_vram(&machine.bus, &neetan),
        "COPY should write the CD-ROM file bytes exactly to the FAT destination"
    );
}

fn make_host_temp_path(prefix: &str, suffix: &str) -> std::path::PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "neetan-{prefix}-{}-{nanos}{suffix}",
        std::process::id()
    ))
}

fn make_short_host_temp_path(prefix: &str, suffix: &str) -> std::path::PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let base = if std::path::Path::new("/tmp").is_dir() {
        std::path::PathBuf::from("/tmp")
    } else {
        std::env::temp_dir()
    };
    base.join(format!(
        "nt-{prefix}-{}-{nanos}{suffix}",
        std::process::id()
    ))
}

fn text_codes(text: &str) -> Vec<u16> {
    text.bytes().map(u16::from).collect()
}

#[test]
fn copy_recursive_dos_to_dos() {
    let mut machine = boot_hle_with_floppy();
    run_command(&mut machine, b"A:\r");

    run_command(&mut machine, b"MD SRCDIR\r");
    run_command(&mut machine, b"MD SRCDIR\\SUB\r");

    run_command(&mut machine, b"COPY TESTFILE.TXT SRCDIR\\ROOT.TXT\r");
    run_command(&mut machine, b"COPY TESTFILE.TXT SRCDIR\\SUB\\CHILD.TXT\r");

    run_command(&mut machine, b"MD DSTDIR\r");

    run_command(&mut machine, b"CLS\r");
    run_command(&mut machine, b"COPY SRCDIR DSTDIR\r");

    let copied = [0x0063, 0x006F, 0x0070, 0x0069, 0x0065, 0x0064]; // "copied"
    assert!(
        find_string_in_text_vram(&machine.bus, &copied),
        "Recursive COPY should report copied files"
    );

    run_command(&mut machine, b"CLS\r");
    run_command(&mut machine, b"DIR DSTDIR\\SRCDIR\\SUB\r");

    let child = [
        0x0043, 0x0048, 0x0049, 0x004C, 0x0044, // "CHILD"
    ];
    assert!(
        find_string_in_text_vram(&machine.bus, &child),
        "CHILD.TXT should appear in the recursively copied tree"
    );
}

#[test]
fn copy_host_to_dos_single_file() {
    let host_src = make_host_temp_path("copy-h2d", ".txt");
    fs::write(&host_src, b"HELLO FROM HOST").expect("write host source");

    let mut machine = boot_hle_with_floppy();
    run_command(&mut machine, b"A:\r");

    let mut cmd = b"COPY host:".to_vec();
    cmd.extend_from_slice(host_src.to_str().unwrap().as_bytes());
    cmd.extend_from_slice(b" FROMHOST.TXT\r");
    run_command(&mut machine, &cmd);

    run_command(&mut machine, b"CLS\r");
    run_command(&mut machine, b"TYPE FROMHOST.TXT\r");

    let hello = [
        0x0048, 0x0045, 0x004C, 0x004C, 0x004F, 0x0020, 0x0046, 0x0052, 0x004F, 0x004D,
    ]; // "HELLO FROM"
    assert!(
        find_string_in_text_vram(&machine.bus, &hello),
        "Host-to-DOS COPY should write the host file contents"
    );

    let _ = fs::remove_file(&host_src);
}

#[test]
fn copy_host_to_dos_multicluster_binary_with_verify() {
    let source_bytes = prng_bytes(RANDOM_FILE_SIZE);
    let host_source = make_short_host_temp_path("h2dm", ".bin");
    fs::write(&host_source, &source_bytes).expect("write host binary source");

    let hard_disk_path = make_temp_hdd_path("copy-host-to-dos-multi");
    let hard_disk = create_empty_hdd(256);
    fs::write(&hard_disk_path, hard_disk.to_bytes()).expect("write temp HDD image");

    let mut machine = boot_hle_with_temp_hdd_and_floppy(&hard_disk_path, create_test_floppy());

    format_drive_a(&mut machine);

    let mut command = b"COPY /V host:".to_vec();
    command.extend_from_slice(host_source.to_str().unwrap().as_bytes());
    command.extend_from_slice(b" A:\\HOSTBIN.BIN\r");
    run_command(&mut machine, &command);
    machine.bus.flush_hdd(0);

    assert!(
        find_string_in_text_vram(&machine.bus, &text_codes("copied")),
        "Host-to-DOS binary copy should report success"
    );
    let copied_bytes = extract_root_file(&hard_disk_path, b"HOSTBIN BIN")
        .expect("HOSTBIN.BIN should be readable from the HDD image");
    assert_eq!(copied_bytes, source_bytes);

    let _ = fs::remove_file(&host_source);
    let _ = fs::remove_file(&hard_disk_path);
}

#[test]
fn copy_dos_to_host_single_file() {
    let host_dst = make_host_temp_path("copy-d2h", ".txt");
    let _ = fs::remove_file(&host_dst);

    let mut machine = boot_hle_with_floppy();
    run_command(&mut machine, b"A:\r");

    let mut cmd = b"COPY TESTFILE.TXT host:".to_vec();
    cmd.extend_from_slice(host_dst.to_str().unwrap().as_bytes());
    cmd.push(b'\r');
    run_command(&mut machine, &cmd);

    let copied = [0x0063, 0x006F, 0x0070, 0x0069, 0x0065, 0x0064]; // "copied"
    assert!(
        find_string_in_text_vram(&machine.bus, &copied),
        "DOS-to-host COPY should report 'copied'"
    );

    let written = fs::read(&host_dst).expect("host destination should exist");
    assert!(
        written.starts_with(b"HELLO WORLD"),
        "Host file should contain the DOS file's bytes, got {written:?}"
    );

    let _ = fs::remove_file(&host_dst);
}

#[test]
fn copy_dos_to_host_multicluster_binary() {
    let source_bytes = prng_bytes(RANDOM_FILE_SIZE);
    let floppy = create_random_file_floppy(&source_bytes);
    let host_destination = make_host_temp_path("copy-d2h-multi", ".bin");
    let _ = fs::remove_file(&host_destination);

    let mut machine = boot_hle_with_floppy_image(floppy);
    run_command(&mut machine, b"A:\r");

    let mut command = b"COPY RAND.BIN host:".to_vec();
    command.extend_from_slice(host_destination.to_str().unwrap().as_bytes());
    command.push(b'\r');
    run_command(&mut machine, &command);

    assert!(
        find_string_in_text_vram(&machine.bus, &text_codes("copied")),
        "DOS-to-host binary copy should report success"
    );
    let written = fs::read(&host_destination).expect("host binary destination should exist");
    assert_eq!(written, source_bytes);

    let _ = fs::remove_file(&host_destination);
}

#[test]
fn copy_dos_to_host_existing_directory_uses_source_basename() {
    let host_directory = make_host_temp_path("copy-d2h-dir", "");
    fs::create_dir_all(&host_directory).expect("create host destination directory");

    let mut machine = boot_hle_with_floppy();
    run_command(&mut machine, b"A:\r");

    let mut command = b"COPY TESTFILE.TXT host:".to_vec();
    command.extend_from_slice(host_directory.to_str().unwrap().as_bytes());
    command.push(b'\r');
    run_command(&mut machine, &command);

    let copied_file = host_directory.join("TESTFILE.TXT");
    let written = fs::read(&copied_file).expect("host directory should receive TESTFILE.TXT");
    assert_eq!(written, TEST_FILE_CONTENT);

    let _ = fs::remove_dir_all(&host_directory);
}

#[test]
fn copy_dos_to_host_existing_destination_prompt_decline_preserves_file() {
    let host_destination = make_host_temp_path("copy-d2h-decline", ".txt");
    fs::write(&host_destination, b"keep me").expect("seed host destination");

    let mut machine = boot_hle_with_floppy();
    run_command(&mut machine, b"A:\r");

    let mut command = b"COPY TESTFILE.TXT host:".to_vec();
    command.extend_from_slice(host_destination.to_str().unwrap().as_bytes());
    command.push(b'\r');
    type_string_long(&mut machine, &command);
    machine.run_for(5_000_000);

    assert!(
        find_string_in_text_vram(&machine.bus, &text_codes("Overwrite")),
        "COPY should prompt before overwriting an existing host file"
    );

    type_string(&mut machine.bus, b"N");
    machine.run_for(5_000_000);

    assert_eq!(fs::read(&host_destination).unwrap(), b"keep me");

    let _ = fs::remove_file(&host_destination);
}

#[test]
fn copy_dos_to_host_existing_destination_prompt_yes_overwrites() {
    let host_destination = make_host_temp_path("copy-d2h-yes", ".txt");
    fs::write(&host_destination, b"replace me").expect("seed host destination");

    let mut machine = boot_hle_with_floppy();
    run_command(&mut machine, b"A:\r");

    let mut command = b"COPY TESTFILE.TXT host:".to_vec();
    command.extend_from_slice(host_destination.to_str().unwrap().as_bytes());
    command.push(b'\r');
    type_string_long(&mut machine, &command);
    machine.run_for(5_000_000);

    assert!(
        find_string_in_text_vram(&machine.bus, &text_codes("Overwrite")),
        "COPY should prompt before overwriting an existing host file"
    );

    type_string(&mut machine.bus, b"Y");
    machine.run_for(5_000_000);

    assert_eq!(fs::read(&host_destination).unwrap(), TEST_FILE_CONTENT);

    let _ = fs::remove_file(&host_destination);
}

#[test]
fn copy_host_to_host_is_rejected() {
    let host_src = make_host_temp_path("copy-h2h-src", ".txt");
    let host_dst = make_host_temp_path("copy-h2h-dst", ".txt");
    fs::write(&host_src, b"NOPE").expect("write host source");

    let mut machine = boot_hle_with_floppy();
    run_command(&mut machine, b"A:\r");

    run_command(&mut machine, b"CLS\r");

    let mut cmd = b"COPY host:".to_vec();
    cmd.extend_from_slice(host_src.to_str().unwrap().as_bytes());
    cmd.extend_from_slice(b" host:");
    cmd.extend_from_slice(host_dst.to_str().unwrap().as_bytes());
    cmd.push(b'\r');
    run_command(&mut machine, &cmd);

    let copied = [0x0063, 0x006F, 0x0070, 0x0069, 0x0065, 0x0064];
    assert!(
        !find_string_in_text_vram(&machine.bus, &copied),
        "Host-to-host COPY must not succeed"
    );
    assert!(
        !host_dst.exists(),
        "Host-to-host COPY must not create destination"
    );

    let _ = fs::remove_file(&host_src);
}

#[test]
fn copy_host_dir_to_dos_recursive_preserves_tree() {
    let host_root = make_host_temp_path("copy-h2d-tree", "");
    fs::create_dir_all(host_root.join("sub")).expect("create host subdir");
    fs::create_dir_all(host_root.join("empty")).expect("create host empty dir");
    fs::write(host_root.join("mixed.txt"), b"Root text\r\n").expect("write root text");
    fs::write(host_root.join("sub").join("deep.txt"), b"Deep text\r\n").expect("write nested text");
    let binary_bytes = prng_bytes(4099);
    fs::write(host_root.join("data.bin"), &binary_bytes).expect("write host binary");

    let hard_disk_path = make_temp_hdd_path("copy-host-tree");
    let hard_disk = create_empty_hdd(256);
    fs::write(&hard_disk_path, hard_disk.to_bytes()).expect("write temp HDD image");

    let mut machine = boot_hle_with_temp_hdd_and_floppy(&hard_disk_path, create_test_floppy());

    format_drive_a(&mut machine);

    let mut command = b"COPY host:".to_vec();
    command.extend_from_slice(host_root.to_str().unwrap().as_bytes());
    command.extend_from_slice(b" A:\\HOSTTREE\r");
    run_command(&mut machine, &command);
    machine.bus.flush_hdd(0);

    assert_eq!(
        extract_hard_disk_file(&hard_disk_path, &["HOSTTREE", "MIXED.TXT"]).unwrap(),
        b"Root text\r\n"
    );
    assert_eq!(
        extract_hard_disk_file(&hard_disk_path, &["HOSTTREE", "SUB", "DEEP.TXT"]).unwrap(),
        b"Deep text\r\n"
    );
    assert_eq!(
        extract_hard_disk_file(&hard_disk_path, &["HOSTTREE", "DATA.BIN"]).unwrap(),
        binary_bytes
    );
    assert!(
        hard_disk_directory_exists(&hard_disk_path, &["HOSTTREE", "EMPTY"]).unwrap(),
        "empty host directories should be represented in DOS"
    );

    let _ = fs::remove_dir_all(&host_root);
    let _ = fs::remove_file(&hard_disk_path);
}

#[test]
fn copy_host_dir_to_dos_rejects_long_filename_preflight() {
    let host_root = make_host_temp_path("copy-pref", "");
    fs::create_dir_all(&host_root).expect("create host dir");
    fs::write(host_root.join("OK.TXT"), b"ok").expect("write ok file");
    fs::write(
        host_root.join("toolongname.txt"),
        b"would corrupt 8.3 if copied",
    )
    .expect("write bad file");

    let mut machine = boot_hle_with_floppy();
    run_command(&mut machine, b"A:\r");

    run_command(&mut machine, b"MD HOSTDST\r");

    run_command(&mut machine, b"CLS\r");

    let mut cmd = b"COPY host:".to_vec();
    cmd.extend_from_slice(host_root.to_str().unwrap().as_bytes());
    cmd.extend_from_slice(b" HOSTDST\r");
    run_command(&mut machine, &cmd);

    let long_msg = [
        0x004C, 0x006F, 0x006E, 0x0067, // "Long"
    ];
    assert!(
        find_string_in_text_vram(&machine.bus, &long_msg),
        "Pre-flight should reject long host filenames with 'Long' diagnostic"
    );

    run_command(&mut machine, b"CLS\r");
    run_command(&mut machine, b"DIR HOSTDST\r");

    let ok_marker = [
        0x004F, 0x004B, 0x0020, 0x0020, // "OK  " padded
    ];
    assert!(
        !find_string_in_text_vram(&machine.bus, &ok_marker),
        "pre-flight failure should keep valid earlier entries out of HOSTDST"
    );

    let _ = fs::remove_dir_all(&host_root);
}

#[test]
fn copy_dos_to_host_dir_recursive() {
    let host_root = make_host_temp_path("copy-d2hdir", "");
    fs::create_dir_all(&host_root).expect("create host dst dir");

    let mut machine = boot_hle_with_floppy();
    run_command(&mut machine, b"A:\r");

    run_command(&mut machine, b"MD TREE\r");
    run_command(&mut machine, b"MD TREE\\SUB\r");
    run_command(&mut machine, b"COPY TESTFILE.TXT TREE\\TOP.TXT\r");
    run_command(&mut machine, b"COPY TESTFILE.TXT TREE\\SUB\\DEEP.TXT\r");

    run_command(&mut machine, b"CLS\r");

    let mut cmd = b"COPY TREE host:".to_vec();
    cmd.extend_from_slice(host_root.to_str().unwrap().as_bytes());
    cmd.push(b'\r');
    run_command(&mut machine, &cmd);

    let copied = [0x0063, 0x006F, 0x0070, 0x0069, 0x0065, 0x0064];
    assert!(
        find_string_in_text_vram(&machine.bus, &copied),
        "Recursive DOS-to-host COPY should report 'copied'"
    );

    let top = host_root.join("TREE").join("TOP.TXT");
    let deep = host_root.join("TREE").join("SUB").join("DEEP.TXT");
    let top_bytes = fs::read(&top).expect("TREE/TOP.TXT should exist on host");
    let deep_bytes = fs::read(&deep).expect("TREE/SUB/DEEP.TXT should exist on host");
    assert!(top_bytes.starts_with(b"HELLO WORLD"), "top contents");
    assert!(deep_bytes.starts_with(b"HELLO WORLD"), "deep contents");

    let _ = fs::remove_dir_all(&host_root);
}

#[test]
fn copy_dos_wildcard_to_host_directory_copies_all_matches() {
    let host_directory = make_host_temp_path("copy-wild-host", "");
    fs::create_dir_all(&host_directory).expect("create host destination directory");

    let mut machine = boot_hle_with_floppy();
    run_command(&mut machine, b"A:\r");

    run_command(&mut machine, b"COPY TESTFILE.TXT ONE.TXT\r");
    run_command(&mut machine, b"COPY TESTFILE.TXT TWO.TXT\r");

    let mut command = b"COPY *.TXT host:".to_vec();
    command.extend_from_slice(host_directory.to_str().unwrap().as_bytes());
    command.push(b'\r');
    run_command(&mut machine, &command);

    assert_eq!(
        fs::read(host_directory.join("TESTFILE.TXT")).unwrap(),
        TEST_FILE_CONTENT
    );
    assert_eq!(
        fs::read(host_directory.join("ONE.TXT")).unwrap(),
        TEST_FILE_CONTENT
    );
    assert_eq!(
        fs::read(host_directory.join("TWO.TXT")).unwrap(),
        TEST_FILE_CONTENT
    );

    let _ = fs::remove_dir_all(&host_directory);
}

#[test]
fn copy_cdrom_to_host_preserves_exact_bytes() {
    let host_destination = make_host_temp_path("copy-cdrom-host", ".txt");
    let _ = fs::remove_file(&host_destination);

    let mut machine = boot_hle_with_cdrom_image(create_test_cdimage());

    let mut command = b"COPY Q:\\README.TXT host:".to_vec();
    command.extend_from_slice(host_destination.to_str().unwrap().as_bytes());
    command.push(b'\r');
    run_command_ap(&mut machine, &command);

    assert_eq!(fs::read(&host_destination).unwrap(), TEST_CDROM_README);

    let _ = fs::remove_file(&host_destination);
}

#[test]
fn copy_rejects_cdrom_destination() {
    let mut machine = boot_hle_with_cdrom_image(create_test_cdimage());
    machine.bus.insert_floppy(0, create_blank_floppy(), None);

    format_drive_a_ap(&mut machine);

    run_command_ap(&mut machine, b"COPY Q:\\README.TXT Q:\\WRITE.TXT\r");

    let copied = [0x0063, 0x006F, 0x0070, 0x0069, 0x0065, 0x0064]; // "copied"
    assert!(
        !find_string_in_text_vram(&machine.bus, &copied),
        "COPY must not report success when the destination is a read-only CD-ROM"
    );
}
