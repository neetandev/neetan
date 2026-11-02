//! Critical errors of guest block units and of native floppy media through
//! the `INT 24h` handler: the registers of the call, the allowed answers and
//! the answers retry, ignore, fail and abort.

use common::Cpu as _;

use crate::{
    harness::{FLOPPY_SECTOR_SIZE, GuestMachine},
    kernel_block_device::read_file,
    kernel_harness::{
        ANY_SECTOR, IOSYS_BLOCK_HEADER, IOSYS_SEGMENT, KERNEL_SEGMENT, RAMDISK_DATA_SECTOR,
        RAMDISK_HEADER, RAMDISK_SECTOR_SIZE, RAMDISK_SEGMENT, SYSINIT_SEGMENT, TAG_RAMDISK,
        abort_child_program, booted_kernel, clear_request_log, critical_error_record,
        exec_and_return_code, exec_command_com, extended_error, fail_ramdisk_requests,
        install_critical_error_handler, iso_image, linear, ramdisk_image, request_log, run,
        try_open, write_exec_arguments,
    },
};

/// Offset of the critical error stub of the 3.10 layout.
const CRITICAL_ERROR_STUB: u16 = 0x0A18;

/// Drive index of D: (the RAM disk).
const DRIVE_D: u8 = 3;

/// Command code of media check.
const MEDIA_CHECK: u8 = 0x01;
/// Command code of build BPB.
const BUILD_BPB: u8 = 0x02;
/// Command code of read.
const READ: u8 = 0x04;
/// Command code of write.
const WRITE: u8 = 0x08;

/// Driver error not ready.
const NOT_READY: u8 = 0x02;
/// Driver error write fault.
const WRITE_FAULT: u8 = 0x0A;

/// Answer ignore.
const IGNORE: u8 = 0x00;
/// Answer retry.
const RETRY: u8 = 0x01;
/// Answer abort.
const ABORT: u8 = 0x02;
/// Answer fail.
const FAIL: u8 = 0x03;

/// Content of `HELLO.TXT` on the RAM disk.
const HELLO: &[u8] = b"guest block data";

/// Returns a machine whose RAM disk holds `HELLO.TXT` and `CHILD.COM`, a
/// program that opens `D:\HELLO.TXT` with the answer abort.
fn machine() -> GuestMachine {
    booted_kernel(
        None,
        &iso_image(b"MENU", &[]),
        &ramdisk_image(&[
            (b"HELLO   TXT", HELLO),
            (b"CHILD   COM", &abort_child_program(b"D:\\HELLO.TXT")),
        ]),
    )
}

/// Returns the commands that the RAM disk received.
fn ramdisk_commands(machine: &mut GuestMachine) -> Vec<u8> {
    request_log(machine)
        .into_iter()
        .filter(|(device, _)| *device == TAG_RAMDISK)
        .map(|(_, command)| command)
        .collect()
}

/// Creates `D:\NEW.TXT`, writes `data` and closes it. Returns the number of
/// calls that succeeded (3 for all) and the carry of the last call.
fn create_and_write(machine: &mut GuestMachine, data: &[u8]) -> (u16, u16) {
    machine.write_bytes(linear(SYSINIT_SEGMENT, 0x0200), b"D:\\NEW.TXT\0");
    machine.write_bytes(linear(SYSINIT_SEGMENT, 0x0300), data);
    let [count_low, count_high] = (data.len() as u16).to_le_bytes();
    run(
        machine,
        &[
            0x31, 0xF6, // XOR SI,SI
            0xB4, 0x3C, // MOV AH,3Ch
            0x31, 0xC9, // XOR CX,CX
            0xBA, 0x00, 0x02, // MOV DX,0200h
            0xCD, 0x21, // INT 21h
            0x72, 0x17, // JC done
            0x46, // INC SI
            0x89, 0xC3, // MOV BX,AX
            0xB4, 0x40, // MOV AH,40h
            0xB9, count_low, count_high, // MOV CX,count
            0xBA, 0x00, 0x03, // MOV DX,0300h
            0xCD, 0x21, // INT 21h
            0x72, 0x08, // JC done
            0x46, // INC SI
            0xB4, 0x3E, // MOV AH,3Eh
            0xCD, 0x21, // INT 21h
            0x72, 0x01, // JC done
            0x46, // INC SI
            0xF4, // done: HLT
        ],
    );
    (machine.cpu.si(), machine.cpu.flags() & 1)
}

/// Returns the sectors of the RAM disk.
fn ramdisk(machine: &GuestMachine) -> &[u8] {
    let start = linear(RAMDISK_SEGMENT, 0) as usize;
    &machine.bus.ram[start..start + 128 * RAMDISK_SECTOR_SIZE]
}

/// Returns the size of the largest free memory block in paragraphs.
fn largest_free_block(machine: &mut GuestMachine) -> u16 {
    // MOV BX,FFFFh; MOV AH,48h; INT 21h; HLT
    run(machine, &[0xBB, 0xFF, 0xFF, 0xB4, 0x48, 0xCD, 0x21, 0xF4]);
    machine.cpu.bx()
}

/// Returns the critical error flag and the critical error drive of the
/// swappable data area.
fn critical_error_state(machine: &mut GuestMachine) -> (u8, u8) {
    // MOV AX,5D06h; INT 21h; MOV AL,[SI]; MOV DL,[SI+2]; HLT
    run(
        machine,
        &[
            0xB8, 0x06, 0x5D, 0xCD, 0x21, 0x8A, 0x04, 0x8A, 0x54, 0x02, 0xF4,
        ],
    );
    (machine.cpu.al(), machine.cpu.dl())
}

#[test]
fn media_check_error_asks_the_handler_and_fail_fails_the_call() {
    let mut machine = machine();
    assert_eq!(read_file(&mut machine, b"D:\\HELLO.TXT"), HELLO);
    install_critical_error_handler(&mut machine, &[FAIL]);
    fail_ramdisk_requests(&mut machine, MEDIA_CHECK, 0, 1, NOT_READY, ANY_SECTOR);

    let (carry, _) = try_open(&mut machine, b"D:\\HELLO.TXT");
    assert_eq!(carry, 1, "the open fails");
    assert_eq!(extended_error(&mut machine), 0x0053);
    let record = critical_error_record(&mut machine);
    assert_eq!(record.calls, 1);
    assert_eq!(
        record.ax,
        0x1A00 | u16::from(DRIVE_D),
        "FAT area, read, retry and fail"
    );
    assert_eq!(record.di, u16::from(NOT_READY));
    assert_eq!((record.bp, record.si), (IOSYS_SEGMENT, RAMDISK_HEADER));
    assert_eq!(record.extended_error, 0x0015);
    assert_eq!(machine.indos(), 0);

    assert_eq!(read_file(&mut machine, b"D:\\HELLO.TXT"), HELLO);
}

#[test]
fn media_check_retry_checks_the_medium_again() {
    let mut machine = machine();
    assert_eq!(read_file(&mut machine, b"D:\\HELLO.TXT"), HELLO);
    install_critical_error_handler(&mut machine, &[RETRY]);
    fail_ramdisk_requests(&mut machine, MEDIA_CHECK, 0, 1, NOT_READY, ANY_SECTOR);
    clear_request_log(&mut machine);

    assert_eq!(read_file(&mut machine, b"D:\\HELLO.TXT"), HELLO);
    assert_eq!(critical_error_record(&mut machine).calls, 1);
    assert_eq!(ramdisk_commands(&mut machine), [MEDIA_CHECK, MEDIA_CHECK]);
}

#[test]
fn build_bpb_error_reports_the_fat_area_and_retry_checks_the_medium() {
    let mut machine = machine();
    install_critical_error_handler(&mut machine, &[RETRY]);
    fail_ramdisk_requests(&mut machine, BUILD_BPB, 0, 1, NOT_READY, ANY_SECTOR);
    clear_request_log(&mut machine);

    assert_eq!(read_file(&mut machine, b"D:\\HELLO.TXT"), HELLO);
    let record = critical_error_record(&mut machine);
    assert_eq!(record.calls, 1);
    assert_eq!(record.ax, 0x1A00 | u16::from(DRIVE_D));
    assert_eq!(
        ramdisk_commands(&mut machine)[..5],
        [READ, BUILD_BPB, MEDIA_CHECK, READ, BUILD_BPB]
    );
}

#[test]
fn fat_read_error_allows_no_ignore() {
    let mut machine = machine();
    install_critical_error_handler(&mut machine, &[IGNORE]);
    // The read of the first FAT sector in front of build BPB passes, the
    // read of the FAT sectors of the load fails.
    fail_ramdisk_requests(&mut machine, READ, 1, 1, NOT_READY, 1);

    assert_eq!(try_open(&mut machine, b"D:\\HELLO.TXT").0, 1);
    assert_eq!(extended_error(&mut machine), 0x0053);
    let record = critical_error_record(&mut machine);
    assert_eq!(record.calls, 1);
    assert_eq!(record.ax, 0x1A00 | u16::from(DRIVE_D), "no ignore bit");
    assert_eq!(machine.indos(), 0);
}

#[test]
fn ignored_data_read_leaves_the_sectors_empty() {
    let mut machine = machine();
    install_critical_error_handler(&mut machine, &[IGNORE]);
    fail_ramdisk_requests(
        &mut machine,
        READ,
        0,
        1,
        NOT_READY,
        RAMDISK_DATA_SECTOR as u16,
    );

    assert_eq!(read_file(&mut machine, b"D:\\HELLO.TXT"), [0; HELLO.len()]);
    let record = critical_error_record(&mut machine);
    assert_eq!(record.calls, 1);
    assert_eq!(
        record.ax,
        0x3E00 | u16::from(DRIVE_D),
        "data area, read, ignore, retry and fail"
    );
    assert_eq!(machine.indos(), 0);
}

#[test]
fn ignored_data_read_keeps_the_other_sectors_of_the_request() {
    let mut machine = booted_kernel(
        None,
        &iso_image(b"MENU", &[]),
        &ramdisk_image(&[
            (b"FIRST   TXT", b"first sector"),
            (b"FAILED  TXT", b"failed sector"),
            (b"LAST    TXT", b"last sector"),
        ]),
    );
    install_critical_error_handler(&mut machine, &[IGNORE]);
    fail_ramdisk_requests(
        &mut machine,
        READ,
        0,
        1,
        NOT_READY,
        (RAMDISK_DATA_SECTOR + 1) as u16,
    );

    assert_eq!(read_file(&mut machine, b"D:\\FIRST.TXT"), b"first sector");
    assert_eq!(critical_error_record(&mut machine).calls, 1);
    assert_eq!(read_file(&mut machine, b"D:\\FAILED.TXT"), [0; 13]);
    assert_eq!(read_file(&mut machine, b"D:\\LAST.TXT"), b"last sector");
    assert_eq!(critical_error_record(&mut machine).calls, 1);
}

#[test]
fn retried_data_read_loads_the_sectors() {
    let mut machine = machine();
    install_critical_error_handler(&mut machine, &[RETRY]);
    fail_ramdisk_requests(
        &mut machine,
        READ,
        0,
        1,
        NOT_READY,
        RAMDISK_DATA_SECTOR as u16,
    );

    assert_eq!(read_file(&mut machine, b"D:\\HELLO.TXT"), HELLO);
    assert_eq!(critical_error_record(&mut machine).calls, 1);
}

#[test]
fn failed_data_read_fails_the_call() {
    let mut machine = machine();
    install_critical_error_handler(&mut machine, &[FAIL]);
    fail_ramdisk_requests(
        &mut machine,
        READ,
        0,
        1,
        NOT_READY,
        RAMDISK_DATA_SECTOR as u16,
    );

    assert_eq!(try_open(&mut machine, b"D:\\HELLO.TXT").0, 1);
    assert_eq!(extended_error(&mut machine), 0x0053);
    assert_eq!(machine.indos(), 0);
}

#[test]
fn retried_write_writes_the_sector_again() {
    let mut machine = machine();
    install_critical_error_handler(&mut machine, &[RETRY]);
    fail_ramdisk_requests(&mut machine, WRITE, 0, 1, WRITE_FAULT, ANY_SECTOR);

    assert_eq!(create_and_write(&mut machine, b"written"), (3, 0));
    let record = critical_error_record(&mut machine);
    assert_eq!(record.calls, 1);
    assert_eq!(record.ax & 0x1900, 0x1900, "write, retry and fail");
    assert_eq!(record.di, u16::from(WRITE_FAULT));
    assert_eq!(record.extended_error, 0x001D);
    assert!(
        ramdisk(&machine)
            .chunks(32)
            .any(|entry| &entry[..11] == b"NEW     TXT")
    );
    assert!(
        ramdisk(&machine)
            .windows(7)
            .any(|window| window == b"written")
    );
}

#[test]
fn ignored_data_write_counts_as_written() {
    let mut machine = machine();
    install_critical_error_handler(&mut machine, &[IGNORE]);
    // NEW.TXT takes cluster 4, the first free cluster behind the two files.
    let new_file_sector = (RAMDISK_DATA_SECTOR + 2) as u16;
    fail_ramdisk_requests(&mut machine, WRITE, 0, 0xFF, WRITE_FAULT, new_file_sector);

    assert_eq!(create_and_write(&mut machine, b"written"), (3, 0));
    let record = critical_error_record(&mut machine);
    assert!(record.calls >= 1, "calls {}", record.calls);
    assert_eq!(
        record.ax,
        0x3F00 | u16::from(DRIVE_D),
        "data area, write, ignore, retry and fail"
    );
    assert!(
        !ramdisk(&machine)
            .windows(7)
            .any(|window| window == b"written"),
        "the ignored sector stays unwritten"
    );
}

#[test]
fn directory_write_allows_no_ignore() {
    let mut machine = machine();
    install_critical_error_handler(&mut machine, &[IGNORE]);
    let root_sector = (RAMDISK_DATA_SECTOR - 1) as u16;
    fail_ramdisk_requests(&mut machine, WRITE, 0, 1, WRITE_FAULT, root_sector);

    let (succeeded, carry) = create_and_write(&mut machine, b"written");
    assert_eq!(carry, 1, "a call fails");
    assert!(succeeded < 3);
    let record = critical_error_record(&mut machine);
    assert_eq!(record.calls, 1);
    assert_eq!(
        record.ax,
        0x1D00 | u16::from(DRIVE_D),
        "directory area, write, retry and fail"
    );
    assert_eq!(extended_error(&mut machine), 0x0053);
}

#[test]
fn abort_ends_the_program_with_termination_type_2() {
    let mut machine = machine();
    let free_before = largest_free_block(&mut machine);
    // EXEC loads the volume, the open of the child checks the medium.
    fail_ramdisk_requests(&mut machine, MEDIA_CHECK, 0, 1, NOT_READY, ANY_SECTOR);

    assert_eq!(
        exec_and_return_code(&mut machine, b"D:\\CHILD.COM"),
        (0, 0x0200),
        "termination type 2 (critical error), exit code 0"
    );
    assert_eq!(machine.indos(), 0);
    assert_eq!(
        critical_error_state(&mut machine),
        (0, 0xFF),
        "critical error flag and drive of the swappable data area"
    );
    assert_eq!(
        largest_free_block(&mut machine),
        free_before,
        "the memory of the child is free"
    );
    assert_eq!(read_file(&mut machine, b"D:\\HELLO.TXT"), HELLO);
}

#[test]
fn abort_of_a_program_without_a_parent_counts_as_fail() {
    let mut machine = machine();
    assert_eq!(read_file(&mut machine, b"D:\\HELLO.TXT"), HELLO);
    install_critical_error_handler(&mut machine, &[ABORT]);
    fail_ramdisk_requests(&mut machine, MEDIA_CHECK, 0, 1, NOT_READY, ANY_SECTOR);

    assert_eq!(try_open(&mut machine, b"D:\\HELLO.TXT").0, 1);
    assert_eq!(extended_error(&mut machine), 0x0053);
    assert_eq!(critical_error_record(&mut machine).calls, 1);
    assert_eq!(machine.indos(), 0);
}

/// Returns a 2HD floppy image whose FAT ID `IO.SYS` does not know.
fn unknown_floppy() -> Vec<u8> {
    let mut image = vec![0u8; 77 * 2 * 8 * FLOPPY_SECTOR_SIZE];
    image[FLOPPY_SECTOR_SIZE..FLOPPY_SECTOR_SIZE + 3].copy_from_slice(&[0x55, 0xFF, 0xFF]);
    image
}

#[test]
fn unknown_floppy_medium_asks_the_handler_like_a_build_bpb_error() {
    let mut machine = machine();
    machine.drives.floppy = Some(unknown_floppy());
    install_critical_error_handler(&mut machine, &[RETRY, FAIL]);

    assert_eq!(try_open(&mut machine, b"A:\\GAME.DAT").0, 1);
    assert_eq!(extended_error(&mut machine), 0x0053);
    let record = critical_error_record(&mut machine);
    assert_eq!(record.calls, 2, "retry mounts the medium again");
    assert_eq!(
        record.ax, 0x1A00,
        "FAT area, read, retry and fail, drive A:"
    );
    assert_eq!(record.di, 0x0007, "unknown media");
    assert_eq!((record.bp, record.si), (IOSYS_SEGMENT, IOSYS_BLOCK_HEADER));
    assert_eq!(record.extended_error, 0x001A);
    assert_eq!(machine.indos(), 0);
}

#[test]
fn unknown_floppy_medium_ignore_counts_as_fail() {
    let mut machine = machine();
    machine.drives.floppy = Some(unknown_floppy());
    install_critical_error_handler(&mut machine, &[IGNORE]);

    assert_eq!(try_open(&mut machine, b"A:\\GAME.DAT").0, 1);
    assert_eq!(extended_error(&mut machine), 0x0053);
    assert_eq!(critical_error_record(&mut machine).calls, 1);
}

/// Runs `code` at `SYSINIT_SEGMENT:0100h` to its `HLT` and sends NeetanDos
/// through a save state round trip after every trap.
fn run_with_round_trips(machine: &mut GuestMachine, code: &[u8]) {
    machine.start_program_at(SYSINIT_SEGMENT, code);
    let mut traps = machine.trap_count;
    for _ in 0..5_000_000 {
        if machine.run_steps(1) {
            return;
        }
        if machine.trap_count != traps {
            traps = machine.trap_count;
            let mut encoded = Vec::new();
            save_state::StateEncode::encode_state(&machine.dos, &mut encoded);
            let mut decoder = save_state::StateDecoder::new(&encoded, 1 << 24);
            machine.dos =
                save_state::StateDecode::decode_state(&mut decoder).expect("state decodes");
        }
    }
    panic!("program did not halt");
}

#[test]
fn critical_error_survives_save_state_round_trips() {
    let mut machine = machine();
    machine.drives.floppy = Some(unknown_floppy());
    install_critical_error_handler(&mut machine, &[RETRY, FAIL]);
    machine.write_bytes(linear(SYSINIT_SEGMENT, 0x0200), b"A:\\GAME.DAT\0");
    run_with_round_trips(
        &mut machine,
        &[0xB8, 0x00, 0x3D, 0xBA, 0x00, 0x02, 0xCD, 0x21, 0xF4],
    );
    assert_eq!(machine.cpu.flags() & 1, 1, "the open fails");
    assert_eq!(critical_error_record(&mut machine).calls, 2);
    assert_eq!(extended_error(&mut machine), 0x0053);

    install_critical_error_handler(&mut machine, &[IGNORE]);
    fail_ramdisk_requests(
        &mut machine,
        READ,
        0,
        1,
        NOT_READY,
        RAMDISK_DATA_SECTOR as u16,
    );
    machine.write_bytes(linear(SYSINIT_SEGMENT, 0x0200), b"D:\\HELLO.TXT\0");
    run_with_round_trips(
        &mut machine,
        &[0xB8, 0x00, 0x3D, 0xBA, 0x00, 0x02, 0xCD, 0x21, 0xF4],
    );
    assert_eq!(machine.cpu.flags() & 1, 0, "the open succeeds");
    assert_eq!(critical_error_record(&mut machine).calls, 1);
    assert_eq!(machine.indos(), 0);
}

#[test]
fn absolute_disk_read_fails_without_the_handler() {
    let mut machine = machine();
    install_critical_error_handler(&mut machine, &[RETRY]);
    fail_ramdisk_requests(
        &mut machine,
        READ,
        0,
        1,
        NOT_READY,
        RAMDISK_DATA_SECTOR as u16,
    );
    let [sector_low, sector_high] = (RAMDISK_DATA_SECTOR as u16).to_le_bytes();
    run(
        &mut machine,
        &[
            0xB0,
            DRIVE_D, // MOV AL,3
            0xB9,
            0x01,
            0x00, // MOV CX,1
            0xBA,
            sector_low,
            sector_high, // MOV DX,sector
            0xBB,
            0x00,
            0x03, // MOV BX,0300h
            0xCD,
            0x25, // INT 25h
            0x9C, // PUSHF
            0x5E, // POP SI
            0x9D, // POPF
            0xF4, // HLT
        ],
    );
    assert_eq!(machine.cpu.si() & 1, 1, "the read fails");
    assert_eq!(
        critical_error_record(&mut machine).calls,
        0,
        "only a DOS call asks the handler"
    );
    assert_eq!(machine.indos(), 0);
}

#[test]
fn shell_installs_the_critical_error_stub_for_its_programs() {
    // MOV AX,3524h; INT 21h; XOR AX,AX; MOV DS,AX; MOV [04F0h],BX;
    // MOV [04F2h],ES; MOV AX,4C00h; INT 21h
    let vector_program: &[u8] = &[
        0xB8, 0x24, 0x35, 0xCD, 0x21, 0x31, 0xC0, 0x8E, 0xD8, 0x89, 0x1E, 0xF0, 0x04, 0x8C, 0x06,
        0xF2, 0x04, 0xB8, 0x00, 0x4C, 0xCD, 0x21,
    ];
    let mut machine = booted_kernel(
        None,
        &iso_image(b"MENU", &[]),
        &ramdisk_image(&[(b"VECTOR  COM", vector_program)]),
    );
    install_critical_error_handler(&mut machine, &[FAIL]);
    write_exec_arguments(&mut machine, b" /C D:VECTOR");
    run(&mut machine, &exec_command_com());
    assert_eq!(machine.cpu.flags() & 1, 0, "EXEC succeeds");
    assert_eq!(
        (machine.read_word(0x04F2), machine.read_word(0x04F0)),
        (KERNEL_SEGMENT, CRITICAL_ERROR_STUB)
    );
}
