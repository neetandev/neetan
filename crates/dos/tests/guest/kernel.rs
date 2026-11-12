//! The kernel entry: DOSINIT under a synthetic `IO.SYS`, the DOS 3.10 tables,
//! the DOS calls of SYSINIT, EXEC of COMMAND.COM and the save state.

use common::Cpu as _;

use crate::{
    harness::GuestMachine,
    kernel_harness::{
        AUX_HEADER, CLOCK_HEADER, CLOCK_RECORD, CON_HEADER, IOSYS_BLOCK_HEADER, IOSYS_SEGMENT,
        KERNEL_SEGMENT, LIST_OF_LISTS_OFFSET, MEMORY_TOP, RAMDISK_HEADER, SYSINIT_SEGMENT, TAG_AUX,
        TAG_CLOCK, TAG_CON, TAG_IOSYS_BLOCK, TAG_MENU_DRV, TAG_RAMDISK, TAG_TOWNS_CD,
        booted_kernel, console_output, driver_address, exec_command_com, iso_image, kernel_machine,
        linear, list_of_lists, parameter_block, ramdisk_image, request_log, run, run_dosinit,
        start_dosinit, towns_platform, type_input, write_exec_arguments,
    },
};

/// Offset of the first disk buffer of the FM Towns data area.
const BUFFER_OFFSET: u16 = 0x6AA0;
/// Offset of the first DPB with a largest sector of 400h.
const FIRST_DPB_OFFSET: u16 = 0x6EB0;
/// Size of a 3.10 DPB.
const DPB_SIZE: u16 = 0x20;
/// First MCB behind four DPBs: (6EB0h + 4 * 20h) / 10h paragraphs.
const FIRST_MCB: u16 = KERNEL_SEGMENT + 0x06F3;

/// Returns a machine after DOSINIT without CD images and with an empty RAM disk.
fn machine() -> GuestMachine {
    booted_kernel(None, &iso_image(b"MENU", &[]), &ramdisk_image(&[]))
}

/// Returns the linear address of an offset in the DOS data segment.
fn dos(offset: u16) -> u32 {
    linear(KERNEL_SEGMENT, offset)
}

/// Reads the far pointer at `address` as `(segment, offset)`.
fn far_pointer(machine: &mut GuestMachine, address: u32) -> (u16, u16) {
    (machine.read_word(address + 2), machine.read_word(address))
}

#[test]
fn dosinit_leaves_the_registers_and_flags_of_the_dos_310_kernel() {
    let mut machine = kernel_machine(None, &iso_image(b"MENU", &[]), &ramdisk_image(&[]));
    let result = run_dosinit(&mut machine);
    assert_eq!(
        (result.ax, result.bx, result.cx, result.si),
        (0x0003, 0xF01D, 0x0004, 0x0094)
    );
    assert_eq!(result.bp, KERNEL_SEGMENT, "IO.SYS closes BP as a handle");
    assert_eq!(result.flags & 0x0041, 0x0001, "CF set, ZF clear");
}

#[test]
fn dosinit_returns_the_list_of_lists_and_the_first_psp() {
    let mut machine = kernel_machine(None, &iso_image(b"MENU", &[]), &ramdisk_image(&[]));
    let result = run_dosinit(&mut machine);
    assert_eq!(result.es, KERNEL_SEGMENT);
    assert_eq!(result.di, LIST_OF_LISTS_OFFSET);
    assert_eq!(result.ds, FIRST_MCB + 1);
    assert_eq!(result.dx, FIRST_MCB + 1);
    assert_eq!((result.ss, result.sp), (SYSINIT_SEGMENT, 0xFFFE));
    assert_ne!(result.flags & 0x0200, 0, "interrupts stay enabled");

    assert_eq!(machine.read_word(dos(0x0024)), FIRST_MCB);
    let mcb = linear(FIRST_MCB, 0);
    assert_eq!(machine.read_byte(mcb), b'Z');
    assert_eq!(machine.read_word(mcb + 1), 0x0000);
    assert_eq!(machine.read_word(mcb + 3), MEMORY_TOP - FIRST_MCB - 1);

    let psp = linear(FIRST_MCB + 1, 0);
    assert_eq!(machine.read_word(psp + 0x32), 20);
    assert_eq!(
        far_pointer(&mut machine, psp + 0x34),
        (FIRST_MCB + 1, 0x0018)
    );
    let handles: Vec<u8> = (0..5)
        .map(|handle| machine.read_byte(psp + 0x18 + handle))
        .collect();
    assert_eq!(handles, [0x00, 0x00, 0x00, 0xFF, 0xFF]);
    assert_eq!(machine.read_word(dos(0x02CE + 0x10)), FIRST_MCB + 1);
}

#[test]
fn dosinit_sends_init_to_every_device_in_chain_order() {
    let mut machine = kernel_machine(None, &iso_image(b"MENU", &[]), &ramdisk_image(&[]));
    run_dosinit(&mut machine);
    assert_eq!(
        request_log(&mut machine),
        [
            (TAG_CON, 0x00),
            (TAG_AUX, 0x00),
            (TAG_TOWNS_CD, 0x00),
            (TAG_MENU_DRV, 0x00),
            (TAG_CLOCK, 0x00),
            (TAG_IOSYS_BLOCK, 0x00),
            (TAG_RAMDISK, 0x00),
        ]
    );
    assert_eq!(
        machine.read_byte(linear(IOSYS_SEGMENT, IOSYS_BLOCK_HEADER + 0x0A)),
        3
    );
    assert_eq!(
        machine.read_byte(linear(IOSYS_SEGMENT, RAMDISK_HEADER + 0x0A)),
        1
    );
}

#[test]
fn dosinit_links_the_chain_behind_nul_and_installs_con_and_clock() {
    let mut machine = machine();
    let lol = list_of_lists();
    assert_eq!(
        far_pointer(&mut machine, dos(0x0048)),
        (IOSYS_SEGMENT, CON_HEADER)
    );
    assert_eq!(
        &machine.bus.ram[dos(0x0048 + 0x0A) as usize..][..8],
        b"NUL     "
    );
    assert_eq!(
        far_pointer(&mut machine, lol + 0x0C),
        (IOSYS_SEGMENT, CON_HEADER)
    );
    assert_eq!(
        far_pointer(&mut machine, lol + 0x08),
        (IOSYS_SEGMENT, CLOCK_HEADER)
    );

    let sft = dos(0x0098);
    assert_eq!(
        far_pointer(&mut machine, lol + 0x04),
        (KERNEL_SEGMENT, 0x0098)
    );
    assert_eq!(machine.read_word(sft + 4), 5);
    let entry = sft + 6;
    assert_eq!(machine.read_word(entry), 3);
    assert_eq!(machine.read_word(entry + 2), 2);
    assert_eq!(machine.read_word(entry + 5), 0x00C3);
    assert_eq!(
        far_pointer(&mut machine, entry + 7),
        (IOSYS_SEGMENT, CON_HEADER)
    );
    assert_eq!(
        &machine.bus.ram[entry as usize + 0x20..][..11],
        b"CON        "
    );
}

#[test]
fn dosinit_builds_the_buffer_and_the_parameter_blocks_of_every_unit() {
    let mut machine = machine();
    let lol = list_of_lists();
    assert_eq!(machine.read_byte(lol + 0x20), 4);
    assert_eq!(machine.read_word(lol + 0x10), 0x0400);
    assert_eq!(
        far_pointer(&mut machine, lol + 0x12),
        (KERNEL_SEGMENT, BUFFER_OFFSET)
    );
    assert_eq!(
        far_pointer(&mut machine, dos(BUFFER_OFFSET)),
        (0xFFFF, 0xFFFF)
    );
    assert_eq!(machine.read_byte(dos(BUFFER_OFFSET) + 4), 0xFF);

    let expected = [
        (0x0400, IOSYS_BLOCK_HEADER, 0u8),
        (0x0400, IOSYS_BLOCK_HEADER, 1),
        (0x0200, IOSYS_BLOCK_HEADER, 2),
        (0x0200, RAMDISK_HEADER, 0),
    ];
    for (drive_index, (bytes_per_sector, driver, unit)) in expected.into_iter().enumerate() {
        let offset = FIRST_DPB_OFFSET + drive_index as u16 * DPB_SIZE;
        assert_eq!(
            parameter_block(&mut machine, drive_index as u8),
            Some((KERNEL_SEGMENT, offset))
        );
        let dpb = dos(offset);
        assert_eq!(machine.read_byte(dpb + 1), unit);
        assert_eq!(machine.read_word(dpb + 2), bytes_per_sector);
        assert_eq!(
            far_pointer(&mut machine, dpb + 0x12),
            (IOSYS_SEGMENT, driver)
        );
    }
    assert_eq!(
        machine.read_byte(dos(FIRST_DPB_OFFSET + 2 * DPB_SIZE) + 0x16),
        0xFB
    );
}

#[test]
fn dosinit_installs_the_kernel_vectors_and_leaves_int_29h() {
    let mut machine = machine();
    for vector in [
        0x00u32, 0x20, 0x21, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x2A, 0x2F, 0x3F,
    ] {
        assert_eq!(
            machine.read_word(vector * 4 + 2),
            KERNEL_SEGMENT,
            "vector {vector:02X}"
        );
    }
    assert_eq!(far_pointer(&mut machine, 0x29 * 4), (IOSYS_SEGMENT, 0x0440));
    assert_eq!(far_pointer(&mut machine, 0x33 * 4), (0, 0));
    assert_eq!(far_pointer(&mut machine, 0x22 * 4), (FIRST_MCB + 1, 0x0100));
    assert_eq!(machine.read_byte(0x00C0), 0xEA);
    assert_eq!(machine.read_word(0x00C3), KERNEL_SEGMENT);
}

#[test]
fn version_list_of_lists_and_parameter_block_calls_follow_dos_310() {
    let mut machine = machine();
    // MOV AH,30h; INT 21h; HLT
    run(&mut machine, &[0xB4, 0x30, 0xCD, 0x21, 0xF4]);
    assert_eq!(machine.cpu.ax(), 0x0A03);
    assert_eq!(machine.cpu.bx(), 0xFF00);
    assert_eq!(machine.cpu.cx(), 0x0000);

    // MOV AH,52h; INT 21h; HLT
    run(&mut machine, &[0xB4, 0x52, 0xCD, 0x21, 0xF4]);
    assert_eq!(machine.cpu.es(), KERNEL_SEGMENT);
    assert_eq!(machine.cpu.bx(), LIST_OF_LISTS_OFFSET);

    // BPB at 0200h, DPB at 0300h: MOV SI,0200h; MOV BP,0300h; MOV AH,53h; INT 21h; HLT
    let bpb = [
        0x00, 0x04, 0x01, 0x01, 0x00, 0x02, 0xC0, 0x00, 0xD0, 0x04, 0x01, 0x02, 0x00,
    ];
    machine.write_bytes(linear(SYSINIT_SEGMENT, 0x0200), &bpb);
    run(
        &mut machine,
        &[
            0xBE, 0x00, 0x02, 0xBD, 0x00, 0x03, 0xB4, 0x53, 0xCD, 0x21, 0xF4,
        ],
    );
    let dpb = linear(SYSINIT_SEGMENT, 0x0300);
    assert_eq!(machine.read_word(dpb + 2), 0x0400);
    assert_eq!(machine.read_byte(dpb + 0x0F), 2, "byte sectors per FAT");
    assert_eq!(machine.read_word(dpb + 0x10), 5, "first root sector");
    assert_eq!(machine.read_byte(dpb + 0x16), 0x01, "media");
}

#[test]
fn ioctl_is_drive_remote_reports_zero_for_a_local_drive_like_dos_310() {
    let mut machine = machine();
    // MOV DX,1234h; MOV AX,4409h; MOV BL,01h; INT 21h; HLT
    run(
        &mut machine,
        &[
            0xBA, 0x34, 0x12, 0xB8, 0x09, 0x44, 0xB3, 0x01, 0xCD, 0x21, 0xF4,
        ],
    );
    assert_eq!(machine.cpu.flags() & 1, 0);
    assert_eq!(machine.cpu.dx(), 0x0000);
    // MOV AX,4409h; MOV BL,19h; INT 21h; HLT
    run(
        &mut machine,
        &[0xB8, 0x09, 0x44, 0xB3, 0x19, 0xCD, 0x21, 0xF4],
    );
    assert_eq!(machine.cpu.flags() & 1, 1);
    assert_eq!(machine.cpu.ax(), 0x000F);
}

#[test]
fn memory_calls_use_the_free_block_behind_the_parameter_blocks() {
    let mut machine = machine();
    // MOV BX,0094h; MOV AH,48h; INT 21h; HLT
    run(
        &mut machine,
        &[0xBB, 0x94, 0x00, 0xB4, 0x48, 0xCD, 0x21, 0xF4],
    );
    assert_eq!(machine.cpu.ax(), FIRST_MCB + 1);
    // MOV BX,FFFFh; MOV AH,48h; INT 21h; HLT
    run(
        &mut machine,
        &[0xBB, 0xFF, 0xFF, 0xB4, 0x48, 0xCD, 0x21, 0xF4],
    );
    assert_eq!(machine.cpu.ax(), 0x0008);
    assert_eq!(machine.cpu.bx(), MEMORY_TOP - (FIRST_MCB + 1 + 0x94) - 1);
    // MOV AX,<block>; MOV ES,AX; MOV AH,49h; INT 21h; HLT
    let [low, high] = (FIRST_MCB + 1).to_le_bytes();
    run(
        &mut machine,
        &[0xB8, low, high, 0x8E, 0xC0, 0xB4, 0x49, 0xCD, 0x21, 0xF4],
    );
    assert_eq!(machine.cpu.flags() & 1, 0);
    let mcb = linear(FIRST_MCB, 0);
    assert_eq!(machine.read_byte(mcb), b'M', "AH=49h merges no blocks");
    assert_eq!(machine.read_word(mcb + 1), 0, "the block is free");
    assert_eq!(machine.read_word(mcb + 3), 0x0094);
    let last_mcb = linear(FIRST_MCB + 1 + 0x94, 0);
    assert_eq!(machine.read_byte(last_mcb), b'Z');
}

#[test]
fn exec_of_command_com_runs_the_shell_on_the_guest_console() {
    let mut machine = machine();
    write_exec_arguments(&mut machine, b" /C ECHO HELLO");
    run(&mut machine, &exec_command_com());
    assert_eq!(machine.cpu.flags() & 1, 0, "EXEC succeeds");
    let output = String::from_utf8_lossy(&console_output(&mut machine)).into_owned();
    assert!(output.contains("HELLO\r\n"), "{output:?}");
}

#[test]
fn load_of_command_com_keeps_the_block_of_the_resident_shell() {
    let mut machine = machine();
    write_exec_arguments(&mut machine, b" /C ECHO HELLO");
    let mut code = crate::kernel_harness::own_psp();
    code.extend_from_slice(&[
        0xBA, 0x80, 0x02, // MOV DX, 0280h (path)
        0xBB, 0x00, 0x02, // MOV BX, 0200h (parameter block)
        0xB8, 0x01, 0x4B, // MOV AX, 4B01h
        0xCD, 0x21, // INT 21h
        0xB4, 0x62, // MOV AH, 62h
        0xCD, 0x21, // INT 21h
        0xF4, // HLT
    ]);
    run(&mut machine, &code);
    let child_psp = machine.cpu.bx();
    assert_ne!(child_psp, SYSINIT_SEGMENT, "the loaded program is current");
    let block_paragraphs = machine.read_word(linear(child_psp - 1, 3));
    assert_eq!(block_paragraphs, dos::tables::COMMAND_BLOCK_PARAGRAPHS);
}

#[test]
fn shell_runs_a_program_named_with_a_drive_and_no_directory() {
    let mut machine = machine();
    write_exec_arguments(&mut machine, b" /C C:MSCDEX /D:TOWNS_CD /L:Q > NUL");
    run(&mut machine, &exec_command_com());
    assert_eq!(machine.cpu.flags() & 1, 0, "EXEC succeeds");
    let output = String::from_utf8_lossy(&console_output(&mut machine)).into_owned();
    assert!(!output.contains("Bad command"), "{output:?}");
}

#[test]
fn permanent_shell_runs_autoexec_of_the_current_drive() {
    let ramdisk = ramdisk_image(&[(b"AUTOEXECBAT", b"@ECHO AUTOSTART\r\n")]);
    let mut machine = booted_kernel(None, &iso_image(b"MENU", &[]), &ramdisk);
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
    assert!(output.contains("AUTOSTART\r\n"), "{output:?}");
    assert!(output.contains("D:\\>"), "{output:?}");
}

#[test]
fn batch_line_on_a_guest_drive_shows_its_echo_and_output_once() {
    let ramdisk = ramdisk_image(&[
        (b"AUTOEXECBAT", b"TYPE D:\\HELLO.TXT\r\nGREET\r\n"),
        (b"HELLO   TXT", b"HI THERE"),
    ]);
    let mut machine = booted_kernel(None, &iso_image(b"MENU", &[]), &ramdisk);
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
    assert_eq!(
        output.matches("TYPE D:\\HELLO.TXT").count(),
        1,
        "{output:?}"
    );
    assert_eq!(output.matches("HI THERE").count(), 1, "{output:?}");
    assert_eq!(output.matches("D:\\>GREET").count(), 1, "{output:?}");
    assert_eq!(output.matches("Bad command").count(), 1, "{output:?}");
}

#[test]
fn permanent_shell_with_the_nul_device_shows_nothing_on_the_console() {
    let ramdisk = ramdisk_image(&[(b"AUTOEXECBAT", b"ECHO OFF\r\nECHO AUTOSTART\r\n")]);
    let mut machine = booted_kernel(None, &iso_image(b"MENU", &[]), &ramdisk);
    write_exec_arguments(&mut machine, b" C:\\ NUL /P");
    // MOV DL,3; MOV AH,0Eh; INT 21h (current drive D:)
    let mut code = vec![0xB2, 0x03, 0xB4, 0x0E, 0xCD, 0x21];
    code.extend(exec_command_com());
    machine.start_program_at(SYSINIT_SEGMENT, &code);
    assert!(
        !machine.run_steps(3_000_000),
        "the permanent shell does not return"
    );
    let output = String::from_utf8_lossy(&console_output(&mut machine)).into_owned();
    assert!(!output.contains("ECHO OFF"), "{output:?}");
    assert!(!output.contains("AUTOSTART"), "{output:?}");
    assert!(!output.contains("D:\\>"), "{output:?}");
}

#[test]
fn read_from_con_returns_one_cooked_line() {
    let mut machine = machine();
    type_input(&mut machine, b"AB\x08C\rXYZ");
    // MOV AH,3Fh; MOV BX,0; MOV CX,10; MOV DX,0400h; INT 21h; HLT
    let code = [
        0xB4, 0x3F, 0xBB, 0x00, 0x00, 0xB9, 0x0A, 0x00, 0xBA, 0x00, 0x04, 0xCD, 0x21, 0xF4,
    ];
    run(&mut machine, &code);
    assert_eq!(machine.cpu.flags() & 1, 0);
    assert_eq!(machine.cpu.ax(), 4);
    assert_eq!(
        &machine.bus.ram[linear(SYSINIT_SEGMENT, 0x0400) as usize..][..4],
        b"AC\r\n"
    );
    assert_eq!(console_output(&mut machine), b"AB\x08 \x08C\r\n");

    // A read of two bytes takes the next line in parts.
    type_input(&mut machine, b"XY\r");
    let code = [
        0xB4, 0x3F, 0xBB, 0x00, 0x00, 0xB9, 0x02, 0x00, 0xBA, 0x00, 0x04, 0xCD, 0x21, 0xF4,
    ];
    run(&mut machine, &code);
    assert_eq!(machine.cpu.ax(), 2);
    assert_eq!(
        &machine.bus.ram[linear(SYSINIT_SEGMENT, 0x0400) as usize..][..2],
        b"XY"
    );
    run(&mut machine, &code);
    assert_eq!(machine.cpu.ax(), 2);
    assert_eq!(
        &machine.bus.ram[linear(SYSINIT_SEGMENT, 0x0400) as usize..][..2],
        b"\r\n"
    );
}

#[test]
fn raw_read_from_con_returns_the_keys_without_echo_and_editing() {
    let mut machine = machine();
    type_input(&mut machine, b"AB\x08C\r");
    // MOV AX,4400h; MOV BX,0; INT 21h; OR DL,20h; XOR DH,DH; MOV AX,4401h;
    // INT 21h; MOV AH,3Fh; MOV BX,0; MOV CX,4; MOV DX,0400h; INT 21h; HLT
    let code = [
        0xB8, 0x00, 0x44, 0xBB, 0x00, 0x00, 0xCD, 0x21, 0x80, 0xCA, 0x20, 0x30, 0xF6, 0xB8, 0x01,
        0x44, 0xCD, 0x21, 0xB4, 0x3F, 0xBB, 0x00, 0x00, 0xB9, 0x04, 0x00, 0xBA, 0x00, 0x04, 0xCD,
        0x21, 0xF4,
    ];
    run(&mut machine, &code);
    assert_eq!(machine.cpu.flags() & 1, 0);
    assert_eq!(machine.cpu.ax(), 4);
    assert_eq!(
        &machine.bus.ram[linear(SYSINIT_SEGMENT, 0x0400) as usize..][..4],
        b"AB\x08C"
    );
    assert_eq!(console_output(&mut machine), b"");
}

#[test]
fn open_finds_the_character_devices_of_the_chain() {
    let mut machine = machine();
    machine.write_bytes(linear(SYSINIT_SEGMENT, 0x0280), b"AUX\0");
    // MOV DX,0280h; MOV AX,3D02h; INT 21h; HLT
    run(
        &mut machine,
        &[0xBA, 0x80, 0x02, 0xB8, 0x02, 0x3D, 0xCD, 0x21, 0xF4],
    );
    assert_eq!(machine.cpu.flags() & 1, 0);
    let handle = machine.cpu.ax();
    let psp = linear(FIRST_MCB + 1, 0);
    let sft_index = machine.read_byte(psp + 0x18 + u32::from(handle));
    let entry = dos(0x0098) + 6 + u32::from(sft_index) * 0x35;
    assert_eq!(
        far_pointer(&mut machine, entry + 7),
        (IOSYS_SEGMENT, AUX_HEADER)
    );
}

#[test]
fn initialized_kernel_survives_a_save_state_round_trip() {
    let ramdisk = ramdisk_image(&[(b"HELLO   TXT", b"saved state")]);
    let mut machine = booted_kernel(None, &iso_image(b"MENU", &[]), &ramdisk);
    let mut encoded = Vec::new();
    save_state::StateEncode::encode_state(&machine.dos, &mut encoded);
    let mut decoder = save_state::StateDecoder::new(&encoded, 1 << 24);
    let decoded: dos::NeetanDos =
        save_state::StateDecode::decode_state(&mut decoder).expect("state decodes");
    assert!(
        decoded
            .validate_state(&towns_platform(), true, true, false, 0)
            .is_ok()
    );
    machine.dos = decoded;

    run(&mut machine, &[0xB4, 0x30, 0xCD, 0x21, 0xF4]);
    assert_eq!(machine.cpu.ax(), 0x0A03);
    assert_eq!(
        crate::kernel_block_device::read_file(&mut machine, b"D:\\HELLO.TXT"),
        b"saved state"
    );
}

#[test]
fn dosinit_continues_after_a_save_state_round_trip_inside_init() {
    let mut machine = kernel_machine(None, &iso_image(b"MENU", &[]), &ramdisk_image(&[]));
    start_dosinit(&mut machine);
    while machine.trap_count < 4 {
        assert!(!machine.run_steps(1), "DOSINIT ended early");
    }
    let mut encoded = Vec::new();
    save_state::StateEncode::encode_state(&machine.dos, &mut encoded);
    let mut decoder = save_state::StateDecoder::new(&encoded, 1 << 24);
    machine.dos = save_state::StateDecode::decode_state(&mut decoder).expect("state decodes");
    assert!(machine.run_steps(5_000_000), "DOSINIT did not return");
    assert_eq!(
        machine.read_word(linear(SYSINIT_SEGMENT, 0x04)),
        FIRST_MCB + 1
    );
    assert_eq!(request_log(&mut machine).len(), 7);
}

#[test]
fn date_comes_from_the_clock_device_of_the_chain() {
    let mut machine = machine();
    // 1995-07-17 (day 5676), 13:45:30.25
    machine.write_bytes(driver_address(CLOCK_RECORD), &[0x2C, 0x16, 45, 13, 25, 30]);
    // MOV AH,2Ah; INT 21h; HLT
    run(&mut machine, &[0xB4, 0x2A, 0xCD, 0x21, 0xF4]);
    assert_eq!(machine.cpu.cx(), 1995);
    assert_eq!(machine.cpu.dx(), 0x0711);
}
