use crate::harness;

/// The expected content of Z:\COMMAND.COM (the HLE stub from dos.rom).
static EXPECTED_STUB: &[u8] = include_bytes!("../../../../utils/dos/dos.rom");

fn open_file_raw(machine: &mut machine_98::Pc9801Ra, filename: &[u8], mode: u8) -> (u16, u16) {
    let path_addr = harness::INJECT_CODE_BASE + 0x200;
    harness::write_bytes(&mut machine.bus, path_addr, filename);
    #[rustfmt::skip]
    let code: Vec<u8> = vec![
        0xB4, 0x3D,                         // MOV AH, 3Dh (open)
        0xB0, mode,                         // MOV AL, mode
        0xBA, 0x00, 0x02,                   // MOV DX, 0200h
        0xCD, 0x21,                         // INT 21h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x02, 0x01,                   // MOV [0x0102], AX (flags)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_with_budget(machine, &code, harness::INJECT_BUDGET);
    let ax = harness::result_word(&machine.bus, 0);
    let flags = harness::result_word(&machine.bus, 2);
    (ax, flags)
}

fn open_file(machine: &mut machine_98::Pc9801Ra, filename: &[u8]) -> u16 {
    let (handle, flags) = open_file_raw(machine, filename, 0x00);
    assert_eq!(
        flags & 0x0001,
        0,
        "open_file: CF should be 0, flags={:#06X}",
        flags
    );
    handle
}

fn close_file(machine: &mut machine_98::Pc9801Ra, handle: u16) {
    let handle_lo = (handle & 0xFF) as u8;
    let handle_hi = (handle >> 8) as u8;
    #[rustfmt::skip]
    let code: Vec<u8> = vec![
        0xBB, handle_lo, handle_hi,         // MOV BX, handle
        0xB4, 0x3E,                         // MOV AH, 3Eh (close)
        0xCD, 0x21,                         // INT 21h
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_with_budget(machine, &code, harness::INJECT_BUDGET);
}

#[test]
fn open_virtual_file_read_only() {
    let mut machine = harness::boot_hle();
    let handle = open_file(&mut machine, b"Z:\\COMMAND.COM\0");
    assert!(
        handle >= 5,
        "File handle should be >= 5 (above standard handles), got {}",
        handle
    );
    close_file(&mut machine, handle);
}

#[test]
fn open_virtual_file_write_denied() {
    let mut machine = harness::boot_hle();
    let (error_code, flags) = open_file_raw(&mut machine, b"Z:\\COMMAND.COM\0", 0x01);
    assert_ne!(flags & 0x0001, 0, "Opening Z: file for write should set CF");
    assert_eq!(error_code, 0x0005, "Error should be 0x0005 (access denied)");
}

#[test]
fn read_virtual_file_full_content() {
    let mut machine = harness::boot_hle();
    let handle = open_file(&mut machine, b"Z:\\COMMAND.COM\0");

    let handle_lo = (handle & 0xFF) as u8;
    let handle_hi = (handle >> 8) as u8;
    #[rustfmt::skip]
    let code: Vec<u8> = vec![
        0xBB, handle_lo, handle_hi,         // MOV BX, handle
        0xB9, 0xFF, 0x00,                   // MOV CX, 00FFh (request more than file size)
        0xBA, 0x10, 0x02,                   // MOV DX, 0210h (read buffer)
        0xB4, 0x3F,                         // MOV AH, 3Fh (read)
        0xCD, 0x21,                         // INT 21h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX (bytes read)
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x02, 0x01,                   // MOV [0x0102], AX (flags)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_with_budget(&mut machine, &code, harness::INJECT_BUDGET);

    let flags = harness::result_word(&machine.bus, 2);
    assert_eq!(
        flags & 0x0001,
        0,
        "Read should succeed (CF=0), flags={:#06X}",
        flags
    );

    let bytes_read = harness::result_word(&machine.bus, 0);
    assert_eq!(
        bytes_read,
        EXPECTED_STUB.len() as u16,
        "Should read exactly {} bytes (full file), got {}",
        EXPECTED_STUB.len(),
        bytes_read
    );

    let read_back = harness::read_bytes(
        &machine.bus,
        harness::INJECT_CODE_BASE + 0x210,
        bytes_read as usize,
    );
    assert_eq!(
        &read_back, EXPECTED_STUB,
        "Read data should match COMMAND.COM stub"
    );

    close_file(&mut machine, handle);
}

#[test]
fn read_virtual_file_returns_zero_at_eof() {
    let mut machine = harness::boot_hle();
    let handle = open_file(&mut machine, b"Z:\\COMMAND.COM\0");

    let handle_lo = (handle & 0xFF) as u8;
    let handle_hi = (handle >> 8) as u8;

    // First read: consume entire file.
    #[rustfmt::skip]
    let code: Vec<u8> = vec![
        0xBB, handle_lo, handle_hi,
        0xB9, 0xFF, 0x00,
        0xBA, 0x10, 0x02,
        0xB4, 0x3F,
        0xCD, 0x21,
        0xFA,
        0xF4,
    ];
    harness::inject_and_run_with_budget(&mut machine, &code, harness::INJECT_BUDGET);

    // Second read: should return 0 bytes (EOF).
    #[rustfmt::skip]
    let code2: Vec<u8> = vec![
        0xBB, handle_lo, handle_hi,
        0xB9, 0x10, 0x00,                   // MOV CX, 10h
        0xBA, 0x10, 0x02,                   // MOV DX, 0210h
        0xB4, 0x3F,                         // MOV AH, 3Fh
        0xCD, 0x21,                         // INT 21h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x02, 0x01,                   // MOV [0x0102], AX
        0xFA,
        0xF4,
    ];
    harness::inject_and_run_with_budget(&mut machine, &code2, harness::INJECT_BUDGET);

    let flags = harness::result_word(&machine.bus, 2);
    assert_eq!(flags & 0x0001, 0, "EOF read should succeed (CF=0)");

    let bytes_read = harness::result_word(&machine.bus, 0);
    assert_eq!(bytes_read, 0, "Read at EOF should return 0 bytes");

    close_file(&mut machine, handle);
}

#[test]
fn lseek_virtual_file_size() {
    let mut machine = harness::boot_hle();
    let handle = open_file(&mut machine, b"Z:\\COMMAND.COM\0");

    let handle_lo = (handle & 0xFF) as u8;
    let handle_hi = (handle >> 8) as u8;
    #[rustfmt::skip]
    let code: Vec<u8> = vec![
        0xBB, handle_lo, handle_hi,         // MOV BX, handle
        0xB4, 0x42,                         // MOV AH, 42h (lseek)
        0xB0, 0x02,                         // MOV AL, 02h (from end)
        0xB9, 0x00, 0x00,                   // MOV CX, 0000h
        0xBA, 0x00, 0x00,                   // MOV DX, 0000h
        0xCD, 0x21,                         // INT 21h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX (low word)
        0x89, 0x16, 0x02, 0x01,             // MOV [0x0102], DX (high word)
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x04, 0x01,                   // MOV [0x0104], AX (flags)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_with_budget(&mut machine, &code, harness::INJECT_BUDGET);

    let flags = harness::result_word(&machine.bus, 4);
    assert_eq!(
        flags & 0x0001,
        0,
        "LSEEK should succeed (CF=0), flags={:#06X}",
        flags
    );

    let size_low = harness::result_word(&machine.bus, 0) as u32;
    let size_high = harness::result_word(&machine.bus, 2) as u32;
    let file_size = (size_high << 16) | size_low;
    assert_eq!(
        file_size,
        EXPECTED_STUB.len() as u32,
        "LSEEK to end should report {} bytes",
        EXPECTED_STUB.len()
    );

    close_file(&mut machine, handle);
}

#[test]
fn seek_and_read_virtual_file() {
    let mut machine = harness::boot_hle();
    let handle = open_file(&mut machine, b"Z:\\COMMAND.COM\0");

    let handle_lo = (handle & 0xFF) as u8;
    let handle_hi = (handle >> 8) as u8;

    // Seek to offset 5 from start, then read remaining bytes.
    #[rustfmt::skip]
    let code: Vec<u8> = vec![
        // LSEEK to offset 5
        0xBB, handle_lo, handle_hi,         // MOV BX, handle
        0xB4, 0x42,                         // MOV AH, 42h
        0xB0, 0x00,                         // MOV AL, 00h (from start)
        0xB9, 0x00, 0x00,                   // MOV CX, 0000h
        0xBA, 0x05, 0x00,                   // MOV DX, 0005h
        0xCD, 0x21,                         // INT 21h
        // Read remaining bytes
        0xBB, handle_lo, handle_hi,         // MOV BX, handle
        0xB9, 0xFF, 0x00,                   // MOV CX, 00FFh
        0xBA, 0x10, 0x02,                   // MOV DX, 0210h
        0xB4, 0x3F,                         // MOV AH, 3Fh (read)
        0xCD, 0x21,                         // INT 21h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX (bytes read)
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x02, 0x01,                   // MOV [0x0102], AX (flags)
        0xFA,
        0xF4,
    ];
    harness::inject_and_run_with_budget(&mut machine, &code, harness::INJECT_BUDGET);

    let flags = harness::result_word(&machine.bus, 2);
    assert_eq!(flags & 0x0001, 0, "Read after seek should succeed (CF=0)");

    let bytes_read = harness::result_word(&machine.bus, 0);
    let expected_remaining = EXPECTED_STUB.len() - 5;
    assert_eq!(
        bytes_read, expected_remaining as u16,
        "Should read {} bytes after seeking to offset 5",
        expected_remaining
    );

    let read_back = harness::read_bytes(
        &machine.bus,
        harness::INJECT_CODE_BASE + 0x210,
        bytes_read as usize,
    );
    assert_eq!(
        &read_back,
        &EXPECTED_STUB[5..],
        "Read data after seek should match expected bytes"
    );

    close_file(&mut machine, handle);
}

#[test]
fn get_attributes_virtual_file() {
    let mut machine = harness::boot_hle();

    let path_addr = harness::INJECT_CODE_BASE + 0x200;
    harness::write_bytes(&mut machine.bus, path_addr, b"Z:\\COMMAND.COM\0");
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x43,                         // MOV AH, 43h (get/set attributes)
        0xB0, 0x00,                         // MOV AL, 00h (get)
        0xBA, 0x00, 0x02,                   // MOV DX, 0200h
        0xCD, 0x21,                         // INT 21h
        0x89, 0x0E, 0x00, 0x01,             // MOV [0x0100], CX (attributes)
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x02, 0x01,                   // MOV [0x0102], AX (flags)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_with_budget(&mut machine, code, harness::INJECT_BUDGET);

    let flags = harness::result_word(&machine.bus, 2);
    assert_eq!(
        flags & 0x0001,
        0,
        "Get attributes should succeed (CF=0), flags={:#06X}",
        flags
    );

    let attrs = harness::result_word(&machine.bus, 0);
    assert_ne!(
        attrs & 0x01,
        0,
        "COMMAND.COM should have read-only attribute"
    );
    assert_ne!(attrs & 0x20, 0, "COMMAND.COM should have archive attribute");
}

#[test]
fn findfirst_virtual_file() {
    let mut machine = harness::boot_hle();

    // Set DTA to a known location, then call FINDFIRST.
    let path_addr = harness::INJECT_CODE_BASE + 0x200;
    harness::write_bytes(&mut machine.bus, path_addr, b"Z:\\*.*\0");
    #[rustfmt::skip]
    let code: &[u8] = &[
        // Set DTA to 0x0300
        0xBA, 0x00, 0x03,                   // MOV DX, 0300h
        0xB4, 0x1A,                         // MOV AH, 1Ah (set DTA)
        0xCD, 0x21,                         // INT 21h
        // FINDFIRST with Z:\*.*
        0xBA, 0x00, 0x02,                   // MOV DX, 0200h (path)
        0xB9, 0x27, 0x00,                   // MOV CX, 0027h (all attributes)
        0xB4, 0x4E,                         // MOV AH, 4Eh (findfirst)
        0xCD, 0x21,                         // INT 21h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x02, 0x01,                   // MOV [0x0102], AX (flags)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_with_budget(&mut machine, code, harness::INJECT_BUDGET);

    let flags = harness::result_word(&machine.bus, 2);
    assert_eq!(
        flags & 0x0001,
        0,
        "FINDFIRST Z:\\*.* should succeed (CF=0), flags={:#06X}",
        flags
    );

    // DTA+0x15 = attribute, DTA+0x1A = size (dword), DTA+0x1E = filename (ASCIIZ).
    let dta_base = harness::INJECT_CODE_BASE + 0x300;
    let found_name = harness::read_string(&machine.bus, dta_base + 0x1E, 13);
    assert_eq!(
        found_name, b"COMMAND.COM",
        "FINDFIRST should find COMMAND.COM"
    );

    let found_size = harness::read_word(&machine.bus, dta_base + 0x1A) as u32
        | ((harness::read_word(&machine.bus, dta_base + 0x1C) as u32) << 16);
    assert_eq!(
        found_size,
        EXPECTED_STUB.len() as u32,
        "FINDFIRST should report correct file size"
    );
}

#[test]
fn findfirst_nonexistent_virtual_file() {
    let mut machine = harness::boot_hle();

    let path_addr = harness::INJECT_CODE_BASE + 0x200;
    harness::write_bytes(&mut machine.bus, path_addr, b"Z:\\NOSUCHFILE.EXE\0");
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xBA, 0x00, 0x03,
        0xB4, 0x1A,
        0xCD, 0x21,
        0xBA, 0x00, 0x02,
        0xB9, 0x27, 0x00,
        0xB4, 0x4E,
        0xCD, 0x21,
        0xA3, 0x00, 0x01,
        0x9C,
        0x58,
        0xA3, 0x02, 0x01,
        0xFA,
        0xF4,
    ];
    harness::inject_and_run_with_budget(&mut machine, code, harness::INJECT_BUDGET);

    let flags = harness::result_word(&machine.bus, 2);
    assert_ne!(
        flags & 0x0001,
        0,
        "FINDFIRST for nonexistent file should set CF"
    );

    let error = harness::result_word(&machine.bus, 0);
    assert_eq!(error, 0x0012, "Error should be 0x0012 (no more files)");
}

#[test]
fn create_on_virtual_drive_denied() {
    let mut machine = harness::boot_hle();

    let path_addr = harness::INJECT_CODE_BASE + 0x200;
    harness::write_bytes(&mut machine.bus, path_addr, b"Z:\\NEWFILE.TXT\0");
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x3C,                         // MOV AH, 3Ch (create)
        0xB9, 0x00, 0x00,                   // MOV CX, 0000h (normal attr)
        0xBA, 0x00, 0x02,                   // MOV DX, 0200h
        0xCD, 0x21,                         // INT 21h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x02, 0x01,                   // MOV [0x0102], AX (flags)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_with_budget(&mut machine, code, harness::INJECT_BUDGET);

    let flags = harness::result_word(&machine.bus, 2);
    assert_ne!(flags & 0x0001, 0, "Create on Z: should set CF");

    let error = harness::result_word(&machine.bus, 0);
    assert_eq!(error, 0x0005, "Error should be 0x0005 (access denied)");
}

#[test]
fn dir_z_drive_lists_command_com() {
    let mut machine = harness::boot_hle();

    harness::type_string(&mut machine.bus, b"CLS\r");
    harness::run_until_prompt(&mut machine);

    harness::type_string(&mut machine.bus, b"DIR Z:\\\r");
    harness::run_until_prompt(&mut machine);

    let command = [0x0043, 0x004F, 0x004D, 0x004D, 0x0041, 0x004E, 0x0044]; // "COMMAND"
    assert!(
        harness::find_string_in_text_vram(&machine.bus, &command),
        "DIR Z:\\ should list COMMAND"
    );
}

#[test]
fn dir_z_drive_wildcard() {
    let mut machine = harness::boot_hle();

    harness::type_string(&mut machine.bus, b"CLS\r");
    harness::run_until_prompt(&mut machine);

    harness::type_string_long(&mut machine, b"DIR Z:\\*.COM\r");
    harness::run_until_prompt(&mut machine);

    let command = [0x0043, 0x004F, 0x004D, 0x004D, 0x0041, 0x004E, 0x0044]; // "COMMAND"
    assert!(
        harness::find_string_in_text_vram(&machine.bus, &command),
        "DIR Z:\\*.COM should list COMMAND"
    );
}

#[test]
fn dir_z_drive_shows_correct_size() {
    let mut machine = harness::boot_hle();

    harness::type_string(&mut machine.bus, b"CLS\r");
    harness::run_until_prompt(&mut machine);

    harness::type_string(&mut machine.bus, b"DIR Z:\\\r");
    harness::run_until_prompt(&mut machine);

    // The file size "15" should appear in the listing.
    let size_15 = [0x0031, 0x0035]; // "15"
    assert!(
        harness::find_string_in_text_vram(&machine.bus, &size_15),
        "DIR Z:\\ should show file size 15"
    );
}
