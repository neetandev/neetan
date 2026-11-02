//! The HLE MSCDEX under a synthetic `IO.SYS`: installation from the command
//! tail, the machine CD-ROM drive for TOWNS_CD, MENU_DRV as a guest CD-ROM
//! drive, the exit codes and the INT 2Fh calls of Fujitsu code.

use common::Cpu as _;
use dos::DosTrapCode;

use crate::{
    harness::GuestMachine,
    kernel_block_device::read_file,
    kernel_harness::{
        CDS_ENTRY_SIZE, CDS_SEGMENT, CRITICAL_ERROR_HANDLER, CRITICAL_ERROR_RECORD, IOSYS_SEGMENT,
        MENU_HEADER, MENU_ISO_SEGMENT, MENU_MEDIA_CHANGE, MENU_NOT_READY, SYSINIT_SEGMENT,
        TAG_MENU_DRV, TOWNS_CD_HEADER, abort_child_program, booted_kernel, clear_request_log,
        console_output, critical_error_record, driver_address, exec_and_return_code,
        exec_command_com, extended_error, install_critical_error_handler, iso_image, linear,
        ramdisk_image, request_log, run, try_open, write_exec_arguments,
    },
};

/// Command tail of the MSCDEX call of `IO.SYS`.
const IO_SYS_TAIL: &[u8] = b" /L:Q /D:TOWNS_CD /D:MENU_DRV /M:8";

/// Drive index of Q:.
const DRIVE_Q: u8 = 16;

/// Drive index of R:.
const DRIVE_R: u8 = 17;

/// Offset of the command tail in a PSP.
const PSP_COMMAND_TAIL: u32 = 0x80;

/// Offset of the flags word in a CDS entry.
const CDS_FLAGS: u32 = 0x43;

/// Offset of the root length word in a CDS entry.
const CDS_ROOT_LENGTH: u32 = 0x4F;

/// Program buffer for paths.
const PATH_BUFFER: u16 = 0x0200;

/// Program buffer for results.
const RESULT_BUFFER: u16 = 0x0500;

/// Returns a machine after DOSINIT with a CD in the machine CD-ROM drive and
/// `RUN386.EXE` on the MENU_DRV volume.
fn machine_with_cdrom(cdrom: Option<Vec<u8>>) -> GuestMachine {
    booted_kernel(
        cdrom,
        &iso_image(b"MENU", &[(b"RUN386.EXE", b"menu volume")]),
        &ramdisk_image(&[]),
    )
}

/// Returns a machine with the game CD and the MENU_DRV volume.
fn machine() -> GuestMachine {
    machine_with_cdrom(Some(iso_image(b"GAME", &[(b"README.TXT", b"game disc")])))
}

/// Writes `tail` into the current PSP, runs the MSCDEX stub and returns the
/// exit code in `AL`.
fn install(machine: &mut GuestMachine, tail: &[u8]) -> u8 {
    // MOV AH,62h; INT 21h; HLT
    run(machine, &[0xB4, 0x62, 0xCD, 0x21, 0xF4]);
    let psp = machine.cpu.bx();
    let mut command_tail = vec![tail.len() as u8];
    command_tail.extend_from_slice(tail);
    command_tail.push(0x0D);
    machine.write_bytes(linear(psp, 0) + PSP_COMMAND_TAIL, &command_tail);

    // PUSHF; PUSH CS; CALL trap; HLT; trap: interrupt trap stub
    let code = [
        0x9C,
        0x0E,
        0xE8,
        0x01,
        0x00,
        0xF4,
        0x50,
        0x52,
        0xBA,
        0xF0,
        0x07,
        0xB0,
        DosTrapCode::MscdexInstall.code(),
        0xEE,
        0xCF,
    ];
    run(machine, &code);
    machine.cpu.ax() as u8
}

/// Runs `INT 2Fh AX=1500h` and returns `BX` and `CX`.
fn drive_count_and_first_drive(machine: &mut GuestMachine) -> (u16, u16) {
    // MOV AX,1500h; XOR BX,BX; INT 2Fh; HLT
    run(machine, &[0xB8, 0x00, 0x15, 0x31, 0xDB, 0xCD, 0x2F, 0xF4]);
    (machine.cpu.bx(), machine.cpu.cx())
}

/// Runs `INT 2Fh AX=150Dh` and returns the first `count` drive letters.
fn drive_letters(machine: &mut GuestMachine, count: usize) -> Vec<u8> {
    let [low, high] = RESULT_BUFFER.to_le_bytes();
    // MOV AX,150Dh; MOV BX,RESULT_BUFFER; INT 2Fh; HLT
    run(
        machine,
        &[0xB8, 0x0D, 0x15, 0xBB, low, high, 0xCD, 0x2F, 0xF4],
    );
    let start = linear(SYSINIT_SEGMENT, RESULT_BUFFER) as usize;
    machine.bus.ram[start..start + count].to_vec()
}

/// Runs `INT 2Fh AX=150Fh` for `path` on `drive_index`. Returns the carry
/// flag, `AX` and the directory record at the result buffer.
fn directory_entry(
    machine: &mut GuestMachine,
    drive_index: u8,
    path: &[u8],
) -> (bool, u16, Vec<u8>) {
    let mut name = path.to_vec();
    name.push(0);
    machine.write_bytes(linear(SYSINIT_SEGMENT, PATH_BUFFER), &name);
    machine.write_bytes(linear(SYSINIT_SEGMENT, RESULT_BUFFER), &[0; 64]);
    let [path_low, path_high] = PATH_BUFFER.to_le_bytes();
    let [segment_low, segment_high] = SYSINIT_SEGMENT.to_le_bytes();
    let [result_low, result_high] = RESULT_BUFFER.to_le_bytes();
    let code = [
        0xB8,
        0x0F,
        0x15, // MOV AX,150Fh
        0xB9,
        drive_index,
        0x00, // MOV CX,drive
        0xBB,
        path_low,
        path_high, // MOV BX,PATH_BUFFER
        0xBE,
        segment_low,
        segment_high, // MOV SI,SYSINIT_SEGMENT
        0xBF,
        result_low,
        result_high, // MOV DI,RESULT_BUFFER
        0xCD,
        0x2F, // INT 2Fh
        0xF4, // HLT
    ];
    run(machine, &code);
    let start = linear(SYSINIT_SEGMENT, RESULT_BUFFER) as usize;
    let length = usize::from(machine.bus.ram[start]);
    (
        machine.cpu.flags() & 1 != 0,
        machine.cpu.ax(),
        machine.bus.ram[start..start + length].to_vec(),
    )
}

/// Returns the linear address of the CDS entry of `drive_index`.
fn cds_entry(drive_index: u8) -> u32 {
    linear(CDS_SEGMENT, 0) + u32::from(drive_index) * CDS_ENTRY_SIZE
}

#[test]
fn installation_maps_towns_cd_to_q_and_menu_drv_to_r() {
    let mut machine = machine();
    assert_eq!(install(&mut machine, IO_SYS_TAIL), 0x00);
    assert_eq!(
        drive_count_and_first_drive(&mut machine),
        (2, u16::from(DRIVE_Q))
    );
    assert_eq!(drive_letters(&mut machine, 2), [DRIVE_Q, DRIVE_R]);
    for drive_index in [DRIVE_Q, DRIVE_R] {
        let entry = cds_entry(drive_index);
        assert_eq!(
            &machine.bus.ram[entry as usize..][..4],
            [b'A' + drive_index, b':', b'\\', 0]
        );
        assert_eq!(machine.read_word(entry + CDS_FLAGS), 0xC000);
        assert_eq!(machine.read_word(entry + CDS_ROOT_LENGTH), 2);
    }
}

#[test]
fn device_list_names_the_driver_headers() {
    let mut machine = machine();
    install(&mut machine, IO_SYS_TAIL);
    let [low, high] = RESULT_BUFFER.to_le_bytes();
    // MOV AX,1501h; MOV BX,RESULT_BUFFER; INT 2Fh; HLT
    run(
        &mut machine,
        &[0xB8, 0x01, 0x15, 0xBB, low, high, 0xCD, 0x2F, 0xF4],
    );
    let list = linear(SYSINIT_SEGMENT, RESULT_BUFFER);
    for (index, header) in [TOWNS_CD_HEADER, MENU_HEADER].into_iter().enumerate() {
        let entry = list + index as u32 * 5;
        assert_eq!(machine.read_byte(entry), 0);
        assert_eq!(machine.read_word(entry + 1), header);
        assert_eq!(machine.read_word(entry + 3), IOSYS_SEGMENT);
    }
}

#[test]
fn files_of_both_drives_read_through_dos() {
    let mut machine = machine();
    install(&mut machine, IO_SYS_TAIL);
    assert_eq!(read_file(&mut machine, b"Q:\\README.TXT"), b"game disc");
    clear_request_log(&mut machine);
    assert_eq!(read_file(&mut machine, b"R:\\RUN386.EXE"), b"menu volume");
    let menu_requests: Vec<u8> = request_log(&mut machine)
        .into_iter()
        .filter(|(device, _)| *device == TAG_MENU_DRV)
        .map(|(_, command)| command)
        .collect();
    assert_eq!(menu_requests.first(), Some(&0x0D), "open of MENU_DRV");
    assert_eq!(menu_requests.last(), Some(&0x0E), "close of MENU_DRV");
    assert!(menu_requests.contains(&0x80), "read long of MENU_DRV");
    assert_eq!(machine.indos(), 0);
}

/// Opens `path` for reading and returns the handle.
fn open_for_reading(machine: &mut GuestMachine, path: &[u8]) -> u16 {
    let mut name = path.to_vec();
    name.push(0);
    machine.write_bytes(linear(SYSINIT_SEGMENT, PATH_BUFFER), &name);
    // MOV AX,3D00h; MOV DX,0200h; INT 21h; HLT
    run(
        machine,
        &[0xB8, 0x00, 0x3D, 0xBA, 0x00, 0x02, 0xCD, 0x21, 0xF4],
    );
    assert_eq!(machine.cpu.flags() & 1, 0, "open {path:?}");
    machine.cpu.ax()
}

/// Reads up to 16 bytes from `handle` and returns the byte count.
fn read_count(machine: &mut GuestMachine, handle: u16) -> u16 {
    let [low, high] = handle.to_le_bytes();
    // MOV AH,3Fh; MOV BX,handle; MOV CX,16; MOV DX,0500h; INT 21h; HLT
    run(
        machine,
        &[
            0xB4, 0x3F, 0xBB, low, high, 0xB9, 0x10, 0x00, 0xBA, 0x00, 0x05, 0xCD, 0x21, 0xF4,
        ],
    );
    machine.cpu.ax()
}

#[test]
fn disc_change_drops_only_the_files_of_the_machine_drive() {
    let mut machine = machine();
    install(&mut machine, IO_SYS_TAIL);
    let game_file = open_for_reading(&mut machine, b"Q:\\README.TXT");
    let menu_file = open_for_reading(&mut machine, b"R:\\RUN386.EXE");
    let memory = crate::harness::RamAccess(&mut machine.bus.ram);
    machine.dos.invalidate_cdrom_caches(&memory);
    assert_eq!(read_count(&mut machine, game_file), 0);
    assert_eq!(read_count(&mut machine, menu_file), 11);
}

#[test]
fn failed_menu_load_asks_the_critical_error_handler_and_reports_fail() {
    let mut machine = machine();
    install(&mut machine, IO_SYS_TAIL);
    install_critical_error_handler(&mut machine, &[0x03]);
    machine.write_bytes(driver_address(MENU_MEDIA_CHANGE), &[0xFF]);
    machine.write_bytes(driver_address(MENU_NOT_READY), &[0x01]);

    assert_eq!(try_open(&mut machine, b"R:\\RUN386.EXE"), (1, 0x0005));
    assert_eq!(extended_error(&mut machine), 0x0053);
    let record = critical_error_record(&mut machine);
    assert_eq!(record.ax, 0x1E00 | u16::from(DRIVE_R));
    assert_eq!(record.di, 0x0002);
    assert_eq!((record.bp, record.si), (IOSYS_SEGMENT, MENU_HEADER));
    assert_eq!(
        record.extended_error, 0x0015,
        "not ready inside the handler"
    );
    assert_eq!(machine.indos(), 0);

    let (carry, _) = try_open(&mut machine, b"R:\\RUN386.EXE");
    assert_eq!(carry, 0, "the second load succeeds");
    assert_eq!(critical_error_record(&mut machine).calls, 1);
}

#[test]
fn critical_error_handler_with_a_dos_call_fails_the_load_once() {
    let mut machine = machine();
    install(&mut machine, IO_SYS_TAIL);
    install_critical_error_handler(&mut machine, &[0x03]);
    machine.write_bytes(driver_address(MENU_NOT_READY), &[0x01]);

    assert_eq!(try_open(&mut machine, b"R:\\RUN386.EXE"), (1, 0x0005));
    assert_eq!(critical_error_record(&mut machine).calls, 1);
    assert_eq!(extended_error(&mut machine), 0x0053);
    assert_eq!(machine.indos(), 0);
}

#[test]
fn critical_error_inside_the_handler_fails_without_a_second_call() {
    let mut machine = machine();
    install(&mut machine, IO_SYS_TAIL);
    // The handler opens the failing file again before it answers retry.
    let [path_low, path_high] = 0x0200u16.to_le_bytes();
    #[rustfmt::skip]
    let code = [
        0x2E, 0xFE, 0x06, 0x48, 0x06,       // INC BYTE [CS:0648h]
        0x1E,                               // PUSH DS
        0x0E,                               // PUSH CS
        0x1F,                               // POP DS
        0xB8, 0x00, 0x3D,                   // MOV AX,3D00h
        0xBA, path_low, path_high,          // MOV DX,0200h
        0xCD, 0x21,                         // INT 21h
        0x2E, 0xA3, 0x4A, 0x06,             // MOV [CS:064Ah],AX
        0x1F,                               // POP DS
        0xB0, 0x01,                         // MOV AL,01h
        0xCF,                               // IRET
    ];
    machine.write_bytes(linear(SYSINIT_SEGMENT, CRITICAL_ERROR_HANDLER), &code);
    machine.write_bytes(linear(SYSINIT_SEGMENT, CRITICAL_ERROR_RECORD), &[0; 12]);
    machine.set_vector(0x24, SYSINIT_SEGMENT, CRITICAL_ERROR_HANDLER);
    machine.write_bytes(driver_address(MENU_NOT_READY), &[0x02]);

    assert_eq!(read_file(&mut machine, b"R:\\RUN386.EXE"), b"menu volume");
    let record = critical_error_record(&mut machine);
    assert_eq!(record.calls, 1, "the inner error fails at once");
    assert_eq!(record.extended_error, 0x0005, "the inner open fails");
    assert_eq!(machine.indos(), 0);
}

#[test]
fn retry_of_the_critical_error_handler_loads_the_menu_volume() {
    let mut machine = machine();
    install(&mut machine, IO_SYS_TAIL);
    install_critical_error_handler(&mut machine, &[0x01]);
    machine.write_bytes(driver_address(MENU_NOT_READY), &[0x01]);
    assert_eq!(read_file(&mut machine, b"R:\\RUN386.EXE"), b"menu volume");
    assert_eq!(critical_error_record(&mut machine).calls, 1);
}

#[test]
fn ignore_of_a_menu_load_error_counts_as_fail() {
    let mut machine = machine();
    install(&mut machine, IO_SYS_TAIL);
    install_critical_error_handler(&mut machine, &[0x00]);
    machine.write_bytes(driver_address(MENU_NOT_READY), &[0x01]);
    assert_eq!(try_open(&mut machine, b"R:\\RUN386.EXE"), (1, 0x0005));
    assert_eq!(extended_error(&mut machine), 0x0053);
    assert_eq!(critical_error_record(&mut machine).calls, 1);
}

#[test]
fn abort_of_a_menu_load_error_ends_the_program() {
    let mut machine = booted_kernel(
        Some(iso_image(b"GAME", &[(b"README.TXT", b"game disc")])),
        &iso_image(b"MENU", &[(b"RUN386.EXE", b"menu volume")]),
        &ramdisk_image(&[(b"CHILD   COM", &abort_child_program(b"R:\\RUN386.EXE"))]),
    );
    install(&mut machine, IO_SYS_TAIL);
    machine.write_bytes(driver_address(MENU_NOT_READY), &[0x01]);

    assert_eq!(
        exec_and_return_code(&mut machine, b"D:\\CHILD.COM"),
        (0, 0x0200),
        "termination type 2 (critical error), exit code 0"
    );
    assert_eq!(machine.indos(), 0);
    assert_eq!(
        try_open(&mut machine, b"R:\\RUN386.EXE").0,
        0,
        "the next load succeeds"
    );
}

#[test]
fn every_entry_checks_the_medium_of_the_menu_volume() {
    let mut machine = machine();
    install(&mut machine, IO_SYS_TAIL);
    assert_eq!(read_file(&mut machine, b"R:\\RUN386.EXE"), b"menu volume");

    clear_request_log(&mut machine);
    assert_eq!(read_file(&mut machine, b"R:\\RUN386.EXE"), b"menu volume");
    let requests: Vec<u8> = request_log(&mut machine)
        .into_iter()
        .filter(|(device, _)| *device == TAG_MENU_DRV)
        .map(|(_, command)| command)
        .collect();
    assert_eq!(requests, [0x03], "media changed query, no reload");

    machine.write_bytes(
        linear(MENU_ISO_SEGMENT, 0),
        &iso_image(b"MENU", &[(b"RUN386.EXE", b"new volume!")]),
    );
    machine.write_bytes(driver_address(MENU_MEDIA_CHANGE), &[0xFF]);
    assert_eq!(read_file(&mut machine, b"R:\\RUN386.EXE"), b"new volume!");
}

#[test]
fn directory_entry_returns_the_iso_record() {
    let mut machine = machine();
    install(&mut machine, IO_SYS_TAIL);
    for (drive_index, path, size) in [
        (DRIVE_Q, b"Q:\\README.TXT".as_slice(), 9u32),
        (DRIVE_R, b"\\RUN386.EXE".as_slice(), 11),
    ] {
        let (carry, ax, record) = directory_entry(&mut machine, drive_index, path);
        assert!(!carry, "drive {drive_index}");
        assert_eq!(ax, 0x0001, "ISO 9660");
        assert_eq!(u32::from_le_bytes(record[10..14].try_into().unwrap()), size);
        let name_length = usize::from(record[32]);
        let name = &record[33..33 + name_length];
        assert!(name.ends_with(b";1"), "{name:?}");
    }
    let (carry, ax, _) = directory_entry(&mut machine, DRIVE_R, b"\\MISSING.EXE");
    assert!(carry);
    assert_eq!(ax, 0x0002);
}

#[test]
fn second_installation_ends_with_exit_code_5() {
    let mut machine = machine();
    assert_eq!(install(&mut machine, IO_SYS_TAIL), 0x00);
    assert_eq!(install(&mut machine, IO_SYS_TAIL), 0x05);
    assert_eq!(drive_count_and_first_drive(&mut machine).0, 2);
}

#[test]
fn installation_without_a_found_device_ends_with_exit_code_1() {
    let mut machine = machine();
    assert_eq!(install(&mut machine, b" /L:Q /D:NOCDROM /M:8"), 0x01);
    assert_eq!(drive_count_and_first_drive(&mut machine).0, 0);
}

#[test]
fn drives_end_at_lastdrive() {
    let mut machine = machine();
    assert_eq!(
        install(&mut machine, b" /L:R /D:TOWNS_CD /D:MENU_DRV"),
        0x00
    );
    assert_eq!(
        drive_count_and_first_drive(&mut machine),
        (1, u16::from(DRIVE_R))
    );
    assert_eq!(read_file(&mut machine, b"R:\\README.TXT"), b"game disc");
}

#[test]
fn towns_cd_without_a_machine_cdrom_drive_is_a_guest_drive() {
    let mut machine = machine_with_cdrom(None);
    assert_eq!(install(&mut machine, b" /L:Q /D:TOWNS_CD"), 0x00);
    assert_eq!(
        drive_count_and_first_drive(&mut machine),
        (1, u16::from(DRIVE_Q))
    );
    // AX=150Bh, CX=Q: reports an MSCDEX drive.
    run(
        &mut machine,
        &[0xB8, 0x0B, 0x15, 0xB9, DRIVE_Q, 0x00, 0xCD, 0x2F, 0xF4],
    );
    assert_eq!(machine.cpu.bx(), 0xADAD);
    assert_ne!(machine.cpu.ax() & 0x00FF, 0);
}

#[test]
fn autoexec_changes_to_the_guest_drive_before_its_volume_loads() {
    let mut machine = booted_kernel(
        Some(iso_image(b"GAME", &[])),
        &iso_image(b"MENU", &[(b"RUN386.EXE", b"menu volume")]),
        &ramdisk_image(&[(b"AUTOEXECBAT", b"@R:\r\n")]),
    );
    install(&mut machine, IO_SYS_TAIL);
    write_exec_arguments(&mut machine, b" C:\\ /P");
    // MOV DL,3; MOV AH,0Eh; INT 21h (current drive D:)
    let mut code = vec![0xB2, 0x03, 0xB4, 0x0E, 0xCD, 0x21];
    code.extend(exec_command_com());
    machine.start_program_at(SYSINIT_SEGMENT, &code);
    assert!(
        !machine.run_steps(3_000_000),
        "the permanent shell does not return"
    );
    let output = String::from_utf8_lossy(&console_output(&mut machine)).into_owned();
    assert!(!output.contains("No media"), "{output:?}");
    assert!(output.contains("R:\\>"), "{output:?}");
}
