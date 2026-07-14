use crate::harness::*;

static EXPECTED_STUB: &[u8] = include_bytes!("../../../../utils/dos/dos.rom");

fn open_file_raw(machine: &mut machine_98::Pc9801Ra, filename: &[u8]) -> (u16, u16) {
    let path_addr = INJECT_CODE_BASE + 0x200;
    write_bytes(&mut machine.bus, path_addr, filename);
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x3D,
        0xB0, 0x00,
        0xBA, 0x00, 0x02,
        0xCD, 0x21,
        0xA3, 0x00, 0x01,
        0x9C,
        0x58,
        0xA3, 0x02, 0x01,
        0xFA,
        0xF4,
    ];
    inject_and_run_with_budget(machine, code, INJECT_BUDGET_DISK_IO);
    (result_word(&machine.bus, 0), result_word(&machine.bus, 2))
}

fn read_file(machine: &mut machine_98::Pc9801Ra, filename: &[u8], count: u16) -> Vec<u8> {
    let path_addr = INJECT_CODE_BASE + 0x200;
    write_bytes(&mut machine.bus, path_addr, filename);
    let count_lo = (count & 0xFF) as u8;
    let count_hi = (count >> 8) as u8;
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x3D,
        0xB0, 0x00,
        0xBA, 0x00, 0x02,
        0xCD, 0x21,
        0x89, 0xC3,
        0xB9, count_lo, count_hi,
        0xBA, 0x00, 0x03,
        0xB4, 0x3F,
        0xCD, 0x21,
        0xA3, 0x00, 0x01,
        0x53,
        0xBB, 0x00, 0x00,
        0x58,
        0x93,
        0xB4, 0x3E,
        0xCD, 0x21,
        0xFA,
        0xF4,
    ];
    inject_and_run_with_budget(machine, code, INJECT_BUDGET_DISK_IO);
    let bytes_read = result_word(&machine.bus, 0) as usize;
    read_bytes(&machine.bus, INJECT_CODE_BASE + 0x300, bytes_read)
}

fn get_file_attributes(machine: &mut machine_98::Pc9801Ra, filename: &[u8]) -> (u8, u16) {
    let path_addr = INJECT_CODE_BASE + 0x200;
    write_bytes(&mut machine.bus, path_addr, filename);
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x43,
        0xB0, 0x00,
        0xBA, 0x00, 0x02,
        0xCD, 0x21,
        0x89, 0x0E, 0x00, 0x01,
        0x9C,
        0x58,
        0xA3, 0x02, 0x01,
        0xFA,
        0xF4,
    ];
    inject_and_run_with_budget(machine, code, INJECT_BUDGET_DISK_IO);
    (result_byte(&machine.bus, 0), result_word(&machine.bus, 2))
}

#[test]
fn dosmock_creates_mock_dos_files_on_formatted_hdd() {
    let mut machine = boot_hle_with_empty_sasi_hdd();

    type_string_long(&mut machine, b"FORMAT A:\r");
    machine.run_for(10_000_000);
    type_string(&mut machine.bus, b"Y");
    run_until_prompt(&mut machine);

    type_string_long(&mut machine, b"DOSMOCK A:\r");
    run_until_prompt(&mut machine);

    let (io_attributes, io_flags) = get_file_attributes(&mut machine, b"A:\\IO.SYS\0");
    assert_eq!(
        io_flags & 0x0001,
        0,
        "IO.SYS attribute query should succeed"
    );
    assert_eq!(
        io_attributes & (0x01 | 0x02 | 0x04),
        0x01 | 0x02 | 0x04,
        "IO.SYS should be read-only, hidden, and system"
    );
    let (io_open_result, io_open_flags) = open_file_raw(&mut machine, b"A:\\IO.SYS\0");
    assert_eq!(
        io_open_flags & 0x0001,
        0,
        "IO.SYS should exist and open successfully, flags={:#06X}, AX={:#06X}",
        io_open_flags,
        io_open_result
    );

    let (msdos_attributes, msdos_flags) = get_file_attributes(&mut machine, b"A:\\MSDOS.SYS\0");
    assert_eq!(
        msdos_flags & 0x0001,
        0,
        "MSDOS.SYS attribute query should succeed"
    );
    assert_eq!(
        msdos_attributes & (0x01 | 0x02 | 0x04),
        0x01 | 0x02 | 0x04,
        "MSDOS.SYS should be read-only, hidden, and system"
    );
    let (msdos_open_result, msdos_open_flags) = open_file_raw(&mut machine, b"A:\\MSDOS.SYS\0");
    assert_eq!(
        msdos_open_flags & 0x0001,
        0,
        "MSDOS.SYS should exist and open successfully, flags={:#06X}, AX={:#06X}",
        msdos_open_flags,
        msdos_open_result
    );

    let (command_attributes, command_flags) =
        get_file_attributes(&mut machine, b"A:\\COMMAND.COM\0");
    assert_eq!(
        command_flags & 0x0001,
        0,
        "COMMAND.COM attribute query should succeed"
    );
    assert_eq!(
        command_attributes & (0x01 | 0x20),
        0x01 | 0x20,
        "COMMAND.COM should be read-only and archive"
    );

    let io_contents = read_file(&mut machine, b"A:\\IO.SYS\0", 1);
    assert!(io_contents.is_empty(), "IO.SYS should be zero-byte");

    let msdos_contents = read_file(&mut machine, b"A:\\MSDOS.SYS\0", 1);
    assert!(msdos_contents.is_empty(), "MSDOS.SYS should be zero-byte");

    let command_contents = read_file(
        &mut machine,
        b"A:\\COMMAND.COM\0",
        EXPECTED_STUB.len() as u16,
    );
    assert_eq!(
        command_contents, EXPECTED_STUB,
        "COMMAND.COM should use the same stub as the virtual drive"
    );
}

#[test]
fn dosmock_aborts_before_writing_if_any_target_exists() {
    let mut machine = boot_hle_with_empty_sasi_hdd();

    type_string_long(&mut machine, b"FORMAT A:\r");
    machine.run_for(10_000_000);
    type_string(&mut machine.bus, b"Y");
    run_until_prompt(&mut machine);

    type_string_long(&mut machine, b"ECHO X > A:\\IO.SYS\r");
    run_until_prompt(&mut machine);

    type_string_long(&mut machine, b"DOSMOCK A:\r");
    run_until_prompt(&mut machine);

    let exists = [
        0x0061, 0x006C, 0x0072, 0x0065, 0x0061, 0x0064, 0x0079, 0x0020, 0x0065, 0x0078, 0x0069,
        0x0073, 0x0074,
    ];
    assert!(
        find_string_in_text_vram(&machine.bus, &exists),
        "DOSMOCK should report that mock files already exist"
    );

    let io_contents = read_file(&mut machine, b"A:\\IO.SYS\0", 8);
    assert_eq!(
        io_contents, b"X\r\n",
        "Existing IO.SYS should remain unchanged after DOSMOCK aborts"
    );

    let (_, msdos_flags) = open_file_raw(&mut machine, b"A:\\MSDOS.SYS\0");
    assert_ne!(
        msdos_flags & 0x0001,
        0,
        "MSDOS.SYS should not be created when DOSMOCK aborts"
    );

    let (_, command_flags) = open_file_raw(&mut machine, b"A:\\COMMAND.COM\0");
    assert_ne!(
        command_flags & 0x0001,
        0,
        "COMMAND.COM should not be created when DOSMOCK aborts"
    );
}
