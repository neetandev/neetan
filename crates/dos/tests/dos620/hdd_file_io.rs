use crate::harness;

fn open_file_generic<const M: u8>(
    machine: &mut machine::Machine<cpu::I386<M>>,
    filename: &[u8],
) -> u16 {
    let path_addr = harness::INJECT_CODE_BASE + 0x200;
    harness::write_bytes(&mut machine.bus, path_addr, filename);
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x3D,                         // MOV AH, 3Dh (open)
        0xB0, 0x00,                         // MOV AL, 00h (read-only)
        0xBA, 0x00, 0x02,                   // MOV DX, 0200h
        0xCD, 0x21,                         // INT 21h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x02, 0x01,                   // MOV [0x0102], AX
        0xFA, 0xF4,                         // CLI; HLT
    ];
    harness::inject_and_run_generic_with_budget(machine, code, harness::INJECT_BUDGET_DISK_IO);

    let flags = harness::result_word(&machine.bus, 2);
    let ax = harness::result_word(&machine.bus, 0);
    assert_eq!(
        flags & 0x0001,
        0,
        "open failed, flags={:#06X}, AX(error)={:#06X}",
        flags,
        ax
    );
    ax
}

fn close_file_generic<const M: u8>(machine: &mut machine::Machine<cpu::I386<M>>, handle: u16) {
    let handle_lo = (handle & 0xFF) as u8;
    let handle_hi = (handle >> 8) as u8;
    #[rustfmt::skip]
    let code: Vec<u8> = vec![
        0xBB, handle_lo, handle_hi,
        0xB4, 0x3E,
        0xCD, 0x21,
        0xFA, 0xF4,
    ];
    harness::inject_and_run_generic_with_budget(machine, &code, harness::INJECT_BUDGET_DISK_IO);
}

fn create_file_generic<const M: u8>(
    machine: &mut machine::Machine<cpu::I386<M>>,
    filename: &[u8],
) -> u16 {
    let path_addr = harness::INJECT_CODE_BASE + 0x200;
    harness::write_bytes(&mut machine.bus, path_addr, filename);
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x3C,
        0xB9, 0x00, 0x00,
        0xBA, 0x00, 0x02,
        0xCD, 0x21,
        0xA3, 0x00, 0x01,
        0x9C, 0x58,
        0xA3, 0x02, 0x01,
        0xFA, 0xF4,
    ];
    harness::inject_and_run_generic_with_budget(machine, code, harness::INJECT_BUDGET_DISK_IO);

    let flags = harness::result_word(&machine.bus, 2);
    assert_eq!(flags & 0x0001, 0, "create failed, flags={:#06X}", flags);
    harness::result_word(&machine.bus, 0)
}

fn delete_file_generic<const M: u8>(machine: &mut machine::Machine<cpu::I386<M>>, filename: &[u8]) {
    let path_addr = harness::INJECT_CODE_BASE + 0x200;
    harness::write_bytes(&mut machine.bus, path_addr, filename);
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xBA, 0x00, 0x02,
        0xB4, 0x41,
        0xCD, 0x21,
        0x9C, 0x58,
        0xA3, 0x00, 0x01,
        0xFA, 0xF4,
    ];
    harness::inject_and_run_generic_with_budget(machine, code, harness::INJECT_BUDGET_DISK_IO);

    let flags = harness::result_word(&machine.bus, 0);
    assert_eq!(flags & 0x0001, 0, "delete failed, flags={:#06X}", flags);
}

/// Runs the open/read/lseek/close test sequence on a given machine.
fn run_hdd_file_io_tests<const M: u8>(
    machine: &mut machine::Machine<cpu::I386<M>>,
    drive_letter: &[u8],
) {
    let mut path = Vec::new();
    path.extend_from_slice(drive_letter);
    path.extend_from_slice(b":\\COMMAND.COM\0");

    // Open
    let handle = open_file_generic(machine, &path);
    assert!(handle >= 5, "handle should be >= 5, got {}", handle);

    let handle_lo = (handle & 0xFF) as u8;
    let handle_hi = (handle >> 8) as u8;

    // Read 2 bytes
    #[rustfmt::skip]
    let read_code: Vec<u8> = vec![
        0xBB, handle_lo, handle_hi,
        0xB9, 0x02, 0x00,
        0xBA, 0x10, 0x02,
        0xB4, 0x3F,
        0xCD, 0x21,
        0xA3, 0x00, 0x01,
        0x9C, 0x58,
        0xA3, 0x02, 0x01,
        0xFA, 0xF4,
    ];
    harness::inject_and_run_generic_with_budget(
        machine,
        &read_code,
        harness::INJECT_BUDGET_DISK_IO,
    );

    let flags = harness::result_word(&machine.bus, 2);
    assert_eq!(flags & 0x0001, 0, "read failed");
    let bytes_read = harness::result_word(&machine.bus, 0);
    assert_eq!(bytes_read, 2);
    let read_back = harness::read_bytes(&machine.bus, harness::INJECT_CODE_BASE + 0x210, 2);
    assert_eq!(&read_back, &harness::TEST_COMMAND_COM[..2]);

    // LSEEK to end
    #[rustfmt::skip]
    let seek_code: Vec<u8> = vec![
        0xBB, handle_lo, handle_hi,
        0xB4, 0x42,
        0xB0, 0x02,             // from end
        0xB9, 0x00, 0x00,
        0xBA, 0x00, 0x00,
        0xCD, 0x21,
        0xA3, 0x00, 0x01,      // AX = low word
        0x89, 0x16, 0x02, 0x01, // DX = high word
        0x9C, 0x58,
        0xA3, 0x04, 0x01,
        0xFA, 0xF4,
    ];
    harness::inject_and_run_generic_with_budget(
        machine,
        &seek_code,
        harness::INJECT_BUDGET_DISK_IO,
    );

    let seek_flags = harness::result_word(&machine.bus, 4);
    assert_eq!(seek_flags & 0x0001, 0, "lseek failed");
    let size_lo = harness::result_word(&machine.bus, 0) as u32;
    let size_hi = harness::result_word(&machine.bus, 2) as u32;
    let file_size = (size_hi << 16) | size_lo;
    assert_eq!(
        file_size,
        harness::TEST_COMMAND_COM.len() as u32,
        "file size mismatch"
    );

    close_file_generic(machine, handle);
}

/// Runs create/write/seek/read/verify/delete on a given machine.
fn run_hdd_write_tests<const M: u8>(
    machine: &mut machine::Machine<cpu::I386<M>>,
    drive_letter: &[u8],
) {
    let mut path = Vec::new();
    path.extend_from_slice(drive_letter);
    path.extend_from_slice(b":\\WTEST.TMP\0");

    let handle = create_file_generic(machine, &path);
    let handle_lo = (handle & 0xFF) as u8;
    let handle_hi = (handle >> 8) as u8;

    // Write "HELLO"
    let data_addr = harness::INJECT_CODE_BASE + 0x210;
    harness::write_bytes(&mut machine.bus, data_addr, b"HELLO");
    #[rustfmt::skip]
    let write_code: Vec<u8> = vec![
        0xBB, handle_lo, handle_hi,
        0xB9, 0x05, 0x00,
        0xBA, 0x10, 0x02,
        0xB4, 0x40,
        0xCD, 0x21,
        0xA3, 0x00, 0x01,
        0x9C, 0x58,
        0xA3, 0x02, 0x01,
        0xFA, 0xF4,
    ];
    harness::inject_and_run_generic_with_budget(
        machine,
        &write_code,
        harness::INJECT_BUDGET_DISK_IO,
    );

    let wflags = harness::result_word(&machine.bus, 2);
    assert_eq!(wflags & 0x0001, 0, "write failed");
    assert_eq!(harness::result_word(&machine.bus, 0), 5);

    // Seek to start
    #[rustfmt::skip]
    let seek_code: Vec<u8> = vec![
        0xBB, handle_lo, handle_hi,
        0xB4, 0x42, 0xB0, 0x00,
        0xB9, 0x00, 0x00, 0xBA, 0x00, 0x00,
        0xCD, 0x21, 0xFA, 0xF4,
    ];
    harness::inject_and_run_generic_with_budget(
        machine,
        &seek_code,
        harness::INJECT_BUDGET_DISK_IO,
    );

    // Read back
    #[rustfmt::skip]
    let read_code: Vec<u8> = vec![
        0xBB, handle_lo, handle_hi,
        0xB9, 0x05, 0x00,
        0xBA, 0x20, 0x02,
        0xB4, 0x3F,
        0xCD, 0x21,
        0xA3, 0x00, 0x01,
        0x9C, 0x58,
        0xA3, 0x02, 0x01,
        0xFA, 0xF4,
    ];
    harness::inject_and_run_generic_with_budget(
        machine,
        &read_code,
        harness::INJECT_BUDGET_DISK_IO,
    );

    let rflags = harness::result_word(&machine.bus, 2);
    assert_eq!(rflags & 0x0001, 0, "read-back failed");
    assert_eq!(harness::result_word(&machine.bus, 0), 5);
    let read_back = harness::read_bytes(&machine.bus, harness::INJECT_CODE_BASE + 0x220, 5);
    assert_eq!(&read_back, b"HELLO");

    close_file_generic(machine, handle);
    delete_file_generic(machine, &path);
}

fn run_hdd_multicluster_write_tests<const M: u8>(
    machine: &mut machine::Machine<cpu::I386<M>>,
    drive_letter: &[u8],
) {
    let mut path = Vec::new();
    path.extend_from_slice(drive_letter);
    path.extend_from_slice(b":\\WBIG.TMP\0");

    let payload = (0..4096)
        .map(|index| ((index * 5) % 251) as u8)
        .collect::<Vec<_>>();

    let handle = create_file_generic(machine, &path);
    let handle_lo = (handle & 0xFF) as u8;
    let handle_hi = (handle >> 8) as u8;

    let write_addr = harness::INJECT_CODE_BASE + 0x400;
    let read_addr = harness::INJECT_CODE_BASE + 0x1800;
    harness::write_bytes(&mut machine.bus, write_addr, &payload);

    #[rustfmt::skip]
    let write_code: Vec<u8> = vec![
        0xBB, handle_lo, handle_hi,
        0xB9, 0x00, 0x10,
        0xBA, 0x00, 0x04,
        0xB4, 0x40,
        0xCD, 0x21,
        0xA3, 0x00, 0x01,
        0x9C, 0x58,
        0xA3, 0x02, 0x01,
        0xFA, 0xF4,
    ];
    harness::inject_and_run_generic_with_budget(
        machine,
        &write_code,
        harness::INJECT_BUDGET_DISK_IO,
    );

    let write_flags = harness::result_word(&machine.bus, 2);
    assert_eq!(write_flags & 0x0001, 0, "multicluster write failed");
    assert_eq!(harness::result_word(&machine.bus, 0), payload.len() as u16);

    #[rustfmt::skip]
    let seek_code: Vec<u8> = vec![
        0xBB, handle_lo, handle_hi,
        0xB4, 0x42, 0xB0, 0x00,
        0xB9, 0x00, 0x00, 0xBA, 0x00, 0x00,
        0xCD, 0x21, 0xFA, 0xF4,
    ];
    harness::inject_and_run_generic_with_budget(
        machine,
        &seek_code,
        harness::INJECT_BUDGET_DISK_IO,
    );

    #[rustfmt::skip]
    let read_code: Vec<u8> = vec![
        0xBB, handle_lo, handle_hi,
        0xB9, 0x00, 0x10,
        0xBA, 0x00, 0x18,
        0xB4, 0x3F,
        0xCD, 0x21,
        0xA3, 0x00, 0x01,
        0x9C, 0x58,
        0xA3, 0x02, 0x01,
        0xFA, 0xF4,
    ];
    harness::inject_and_run_generic_with_budget(
        machine,
        &read_code,
        harness::INJECT_BUDGET_DISK_IO,
    );

    let read_flags = harness::result_word(&machine.bus, 2);
    assert_eq!(read_flags & 0x0001, 0, "multicluster read failed");
    assert_eq!(harness::result_word(&machine.bus, 0), payload.len() as u16);
    let read_back = harness::read_bytes(&machine.bus, read_addr, payload.len());
    assert_eq!(read_back, payload, "multicluster read-back mismatch");

    close_file_generic(machine, handle);
    delete_file_generic(machine, &path);
}

#[test]
fn sasi_hdd_256_open_read_lseek() {
    let mut machine = harness::boot_hle_with_sasi_hdd(256);
    run_hdd_file_io_tests(&mut machine, b"A");
}

#[test]
fn sasi_hdd_256_create_write_read_delete() {
    let mut machine = harness::boot_hle_with_sasi_hdd(256);
    run_hdd_write_tests(&mut machine, b"A");
}

#[test]
fn ide_hdd_512_open_read_lseek() {
    let mut machine = harness::boot_hle_with_ide_hdd(512);
    run_hdd_file_io_tests(&mut machine, b"A");
}

#[test]
fn ide_hdd_512_create_write_read_delete() {
    let mut machine = harness::boot_hle_with_ide_hdd(512);
    run_hdd_write_tests(&mut machine, b"A");
}

#[test]
fn ide_hdd_512_create_write_read_multicluster_file() {
    let mut machine = harness::boot_hle_with_ide_hdd(512);
    run_hdd_multicluster_write_tests(&mut machine, b"A");
}

#[test]
fn ide_hdd_256_open_read_lseek() {
    let mut machine = harness::boot_hle_with_ide_hdd(256);
    run_hdd_file_io_tests(&mut machine, b"A");
}

#[test]
fn ide_hdd_256_create_write_read_delete() {
    let mut machine = harness::boot_hle_with_ide_hdd(256);
    run_hdd_write_tests(&mut machine, b"A");
}

#[test]
fn sasi_hdd_mismatched_sectors_open_read_lseek() {
    let mut machine = harness::boot_hle_with_sasi_hdd_mismatched_sectors();
    run_hdd_file_io_tests(&mut machine, b"A");
}

#[test]
fn sasi_hdd_mismatched_sectors_create_write_read_delete() {
    let mut machine = harness::boot_hle_with_sasi_hdd_mismatched_sectors();
    run_hdd_write_tests(&mut machine, b"A");
}

#[test]
fn sasi_hdd_256_create_write_read_multicluster_file() {
    let mut machine = harness::boot_hle_with_sasi_hdd(256);
    run_hdd_multicluster_write_tests(&mut machine, b"A");
}
