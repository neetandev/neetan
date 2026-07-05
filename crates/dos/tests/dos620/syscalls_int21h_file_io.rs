use common::Bus;
use dos::tables;

use crate::harness;

/// Helper: open a file and return the handle.
fn open_file(machine: &mut machine::Pc9801Ra, filename: &[u8]) -> u16 {
    open_file_with_mode(machine, filename, 0x00)
}

fn open_file_raw(machine: &mut machine::Pc9801Ra, filename: &[u8], open_mode: u8) -> (u16, u16) {
    let path_addr = harness::INJECT_CODE_BASE + 0x200;
    harness::write_bytes(&mut machine.bus, path_addr, filename);
    #[rustfmt::skip]
    let mut code: Vec<u8> = vec![
        0xB4, 0x3D,                         // MOV AH, 3Dh (open)
        0xB0, 0x00,                         // MOV AL, imm8
        0xBA, 0x00, 0x02,                   // MOV DX, 0200h
        0xCD, 0x21,                         // INT 21h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x02, 0x01,                   // MOV [0x0102], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    code[3] = open_mode;
    harness::inject_and_run_with_budget(machine, &code, harness::INJECT_BUDGET_DISK_IO);
    (
        harness::result_word(&machine.bus, 0),
        harness::result_word(&machine.bus, 2),
    )
}

fn open_file_with_mode(machine: &mut machine::Pc9801Ra, filename: &[u8], open_mode: u8) -> u16 {
    let (handle, flags) = open_file_raw(machine, filename, open_mode);
    assert_eq!(
        flags & 0x0001,
        0,
        "open_file: CF should be 0, flags={:#06X}",
        flags
    );
    handle
}

fn sft_addr_for_handle(machine: &mut machine::Pc9801Ra, handle: u16) -> u32 {
    let psp_segment = harness::get_psp_segment(machine);
    let psp_base = harness::far_to_linear(psp_segment, 0);
    let sft_index = u16::from(harness::read_byte(
        &machine.bus,
        psp_base + tables::PSP_OFF_JFT + u32::from(handle),
    ));

    if sft_index < tables::SFT_INITIAL_COUNT {
        tables::SFT_BASE + tables::SFT_HEADER_SIZE + u32::from(sft_index) * tables::SFT_ENTRY_SIZE
    } else {
        let (segment, offset) = harness::read_far_ptr(&machine.bus, tables::SFT_BASE);
        let sft2_base = harness::far_to_linear(segment, offset);
        let local_index = u32::from(sft_index - tables::SFT_INITIAL_COUNT);
        sft2_base + tables::SFT_HEADER_SIZE + local_index * tables::SFT_ENTRY_SIZE
    }
}

fn open_file_ap(machine: &mut machine::Pc9821Ap, filename: &[u8]) -> u16 {
    let path_addr = harness::INJECT_CODE_BASE + 0x200;
    harness::write_bytes(&mut machine.bus, path_addr, filename);
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x3D,                         // MOV AH, 3Dh (open)
        0xB0, 0x00,                         // MOV AL, 00h (read-only)
        0xBA, 0x00, 0x02,                   // MOV DX, 0200h
        0xCD, 0x21,                         // INT 21h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX (handle)
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x02, 0x01,                   // MOV [0x0102], AX (flags)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_generic_with_budget(machine, code, harness::INJECT_BUDGET_DISK_IO);

    let flags = harness::result_word(&machine.bus, 2);
    assert_eq!(
        flags & 0x0001,
        0,
        "open_file_ap: CF should be 0, flags={:#06X}",
        flags
    );
    harness::result_word(&machine.bus, 0)
}

/// Helper: close a file handle.
fn close_file(machine: &mut machine::Pc9801Ra, handle: u16) {
    let handle_lo = (handle & 0xFF) as u8;
    let handle_hi = (handle >> 8) as u8;
    #[rustfmt::skip]
    let code: Vec<u8> = vec![
        0xBB, handle_lo, handle_hi,          // MOV BX, handle
        0xB4, 0x3E,                         // MOV AH, 3Eh (close)
        0xCD, 0x21,                         // INT 21h
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_with_budget(machine, &code, harness::INJECT_BUDGET_DISK_IO);
}

fn write_zero_bytes(machine: &mut machine::Pc9801Ra, handle: u16) {
    let handle_lo = (handle & 0xFF) as u8;
    let handle_hi = (handle >> 8) as u8;
    #[rustfmt::skip]
    let code: Vec<u8> = vec![
        0xBB, handle_lo, handle_hi,          // MOV BX, handle
        0xB4, 0x40,                         // MOV AH, 40h
        0x31, 0xC9,                         // XOR CX, CX
        0xBA, 0x00, 0x02,                   // MOV DX, 0200h
        0xCD, 0x21,                         // INT 21h
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX (flags)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_with_budget(machine, &code, harness::INJECT_BUDGET_DISK_IO);

    let flags = harness::result_word(&machine.bus, 0);
    assert_eq!(
        flags & 0x0001,
        0,
        "write_zero_bytes: CF should be 0, flags={:#06X}",
        flags
    );
}

/// Helper: create a file and return the handle.
fn create_file(machine: &mut machine::Pc9801Ra, filename: &[u8]) -> u16 {
    let path_addr = harness::INJECT_CODE_BASE + 0x200;
    harness::write_bytes(&mut machine.bus, path_addr, filename);
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x3C,                         // MOV AH, 3Ch (create)
        0xB9, 0x00, 0x00,                   // MOV CX, 0000h (normal)
        0xBA, 0x00, 0x02,                   // MOV DX, 0200h
        0xCD, 0x21,                         // INT 21h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX (handle)
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x02, 0x01,                   // MOV [0x0102], AX (flags)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_with_budget(machine, code, harness::INJECT_BUDGET_DISK_IO);

    let flags = harness::result_word(&machine.bus, 2);
    assert_eq!(
        flags & 0x0001,
        0,
        "create_file: CF should be 0, flags={:#06X}",
        flags
    );
    harness::result_word(&machine.bus, 0)
}

/// Helper: delete a file.
fn delete_file(machine: &mut machine::Pc9801Ra, filename: &[u8]) {
    let path_addr = harness::INJECT_CODE_BASE + 0x200;
    harness::write_bytes(&mut machine.bus, path_addr, filename);
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xBA, 0x00, 0x02,                   // MOV DX, 0200h
        0xB4, 0x41,                         // MOV AH, 41h (delete)
        0xCD, 0x21,                         // INT 21h
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX (flags)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_with_budget(machine, code, harness::INJECT_BUDGET_DISK_IO);

    let flags = harness::result_word(&machine.bus, 0);
    assert_eq!(
        flags & 0x0001,
        0,
        "delete_file: CF should be 0, flags={:#06X}",
        flags
    );
}

#[test]
fn open_existing_file() {
    let mut machine = harness::boot_hle_with_floppy();

    let handle = open_file(&mut machine, b"A:\\COMMAND.COM\0");
    assert!(
        handle >= 5,
        "File handle should be >= 5 (above standard handles), got {}",
        handle
    );
    close_file(&mut machine, handle);
}

#[test]
fn failed_open_does_not_consume_file_handle() {
    let mut machine = harness::boot_hle_with_floppy();

    for _ in 0..20 {
        let (ax, flags) = open_file_raw(&mut machine, b"A:\\MISSING.386\0", 0x00);
        assert_eq!(flags & 0x0001, 0x0001, "open should fail");
        assert_eq!(ax, 0x0002);
    }

    let handle = open_file(&mut machine, b"A:\\COMMAND.COM\0");
    close_file(&mut machine, handle);
}

#[test]
fn open_emmxxxx0_as_character_device() {
    let mut machine = harness::boot_hle_with_floppy();

    let handle = open_file_with_mode(&mut machine, b"EMMXXXX0\0", 0x02);
    let sft_addr = sft_addr_for_handle(&mut machine, handle);

    assert_eq!(
        harness::read_word(&machine.bus, sft_addr + tables::SFT_ENT_OPEN_MODE),
        0x0002
    );
    assert_eq!(
        harness::read_word(&machine.bus, sft_addr + tables::SFT_ENT_DEV_INFO),
        tables::SFT_DEVINFO_DRIVER_CHAR
            | tables::SFT_DEVINFO_CHAR
            | tables::SFT_DEVINFO_EOF
            | tables::SFT_DEVINFO_IOCTL
    );
    assert_eq!(
        harness::read_word(&machine.bus, sft_addr + tables::SFT_ENT_DEV_PTR),
        tables::DEV_EMS_OFFSET
    );
    assert_eq!(
        harness::read_word(&machine.bus, sft_addr + tables::SFT_ENT_DEV_PTR + 2),
        tables::DOS_DATA_SEGMENT
    );
    assert_eq!(
        harness::read_bytes(&machine.bus, sft_addr + tables::SFT_ENT_NAME, 11),
        b"EMMXXXX0   "
    );
}

#[test]
fn open_xmsxxxx0_as_character_device() {
    let mut machine = harness::boot_hle_with_floppy();

    let handle = open_file_with_mode(&mut machine, b"\\XMSXXXX0\0", 0x00);
    let sft_addr = sft_addr_for_handle(&mut machine, handle);

    assert_eq!(
        harness::read_word(&machine.bus, sft_addr + tables::SFT_ENT_OPEN_MODE),
        0x0000
    );
    assert_eq!(
        harness::read_word(&machine.bus, sft_addr + tables::SFT_ENT_DEV_INFO),
        tables::SFT_DEVINFO_DRIVER_CHAR
            | tables::SFT_DEVINFO_CHAR
            | tables::SFT_DEVINFO_EOF
            | tables::SFT_DEVINFO_IOCTL
    );
    assert_eq!(
        harness::read_word(&machine.bus, sft_addr + tables::SFT_ENT_DEV_PTR),
        tables::DEV_XMS_OFFSET
    );
    assert_eq!(
        harness::read_word(&machine.bus, sft_addr + tables::SFT_ENT_DEV_PTR + 2),
        tables::DOS_DATA_SEGMENT
    );
    assert_eq!(
        harness::read_bytes(&machine.bus, sft_addr + tables::SFT_ENT_NAME, 11),
        b"XMSXXXX0   "
    );
}

#[test]
fn ioctl_read_cdrom_control_channel_device_status() {
    let mut machine = harness::boot_hle_with_cdrom();

    let handle = open_file_ap(&mut machine, b"CD_101\0");
    let handle_lo = (handle & 0xFF) as u8;
    let handle_hi = (handle >> 8) as u8;
    harness::write_bytes(
        &mut machine.bus,
        harness::INJECT_CODE_BASE + 0x0200,
        &[6, 0, 0, 0, 0],
    );

    #[rustfmt::skip]
    let code: Vec<u8> = vec![
        0xBB, handle_lo, handle_hi,          // MOV BX, handle
        0xB8, 0x02, 0x44,                   // MOV AX, 4402h
        0xB9, 0x05, 0x00,                   // MOV CX, 5
        0xBA, 0x00, 0x02,                   // MOV DX, 0200h
        0xCD, 0x21,                         // INT 21h
        0xA3, 0x00, 0x01,                   // MOV [0100h], AX
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x02, 0x01,                   // MOV [0102h], AX
        0xA1, 0x01, 0x02,                   // MOV AX, [0201h]
        0xA3, 0x04, 0x01,                   // MOV [0104h], AX
        0xA1, 0x03, 0x02,                   // MOV AX, [0203h]
        0xA3, 0x06, 0x01,                   // MOV [0106h], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_generic_with_budget(
        &mut machine,
        &code,
        harness::INJECT_BUDGET_DISK_IO,
    );

    let bytes_read = harness::result_word(&machine.bus, 0);
    let flags = harness::result_word(&machine.bus, 2);
    assert_eq!(
        flags & 0x0001,
        0,
        "CD-ROM IOCTL read control channel should succeed, flags={flags:#06X}",
    );
    assert_eq!(bytes_read, 5, "IOCTL should report 5 bytes read");

    let flags_lo = harness::result_word(&machine.bus, 4);
    let flags_hi = harness::result_word(&machine.bus, 6);
    let device_status = u32::from(flags_lo) | (u32::from(flags_hi) << 16);
    assert!(
        device_status & 0x10 != 0,
        "Device status should report audio playback support, flags={device_status:#06X}",
    );
    assert_eq!(device_status & !0x0396, 0, "Reserved bits should be clear");
}

#[test]
fn ioctl_read_cdrom_control_channel_device_header_address() {
    let mut machine = harness::boot_hle_with_cdrom();

    let handle = open_file_ap(&mut machine, b"CD_101\0");
    let handle_lo = (handle & 0xFF) as u8;
    let handle_hi = (handle >> 8) as u8;
    harness::write_bytes(
        &mut machine.bus,
        harness::INJECT_CODE_BASE + 0x0200,
        &[0, 0, 0, 0, 0],
    );

    #[rustfmt::skip]
    let code: Vec<u8> = vec![
        0xBB, handle_lo, handle_hi,          // MOV BX, handle
        0xB8, 0x02, 0x44,                   // MOV AX, 4402h
        0xB9, 0x05, 0x00,                   // MOV CX, 5
        0xBA, 0x00, 0x02,                   // MOV DX, 0200h
        0xCD, 0x21,                         // INT 21h
        0xA3, 0x00, 0x01,                   // MOV [0100h], AX
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x02, 0x01,                   // MOV [0102h], AX
        0xA1, 0x01, 0x02,                   // MOV AX, [0201h]
        0xA3, 0x04, 0x01,                   // MOV [0104h], AX
        0xA1, 0x03, 0x02,                   // MOV AX, [0203h]
        0xA3, 0x06, 0x01,                   // MOV [0106h], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_generic_with_budget(
        &mut machine,
        &code,
        harness::INJECT_BUDGET_DISK_IO,
    );

    let bytes_read = harness::result_word(&machine.bus, 0);
    let flags = harness::result_word(&machine.bus, 2);
    assert_eq!(
        flags & 0x0001,
        0,
        "CD-ROM device-header IOCTL should succeed, flags={flags:#06X}",
    );
    assert_eq!(bytes_read, 5, "IOCTL should report 5 bytes read");
    let header_off = harness::result_word(&machine.bus, 4);
    let header_seg = harness::result_word(&machine.bus, 6);
    assert_eq!(
        header_off, 0,
        "CD-ROM header should be normalized to offset 0, got {header_off:#06X}"
    );
    assert_eq!(
        header_seg,
        tables::CDROM_MIRROR_HEADER_SEGMENT,
        "CD-ROM header segment should point at the mirror header, got {header_seg:#06X}"
    );
    let header_addr = (u32::from(header_seg) << 4) + u32::from(header_off);
    assert_eq!(
        harness::read_word(&machine.bus, header_addr + tables::DEVHDR_OFF_STRATEGY),
        tables::CDROM_MIRROR_STRATEGY_OFFSET
    );
    assert_eq!(
        harness::read_word(&machine.bus, header_addr + tables::DEVHDR_OFF_INTERRUPT),
        tables::CDROM_MIRROR_INTERRUPT_OFFSET
    );
}

#[test]
fn cdrom_device_strategy_interrupt_processes_request() {
    let mut machine = harness::boot_hle_with_cdrom();

    let base = harness::INJECT_CODE_BASE;
    let req_off: u16 = 0x0200;
    let req_addr = base + u32::from(req_off);
    let ctl_off: u16 = 0x0220;
    let ctl_addr = base + u32::from(ctl_off);
    let seg = harness::INJECT_CODE_SEGMENT;

    harness::write_bytes(&mut machine.bus, req_addr, &[26, 0, 3, 0, 0]);
    harness::write_bytes(&mut machine.bus, req_addr + 5, &[0; 8]);
    harness::write_bytes(
        &mut machine.bus,
        req_addr + 14,
        &[
            ctl_off as u8,
            (ctl_off >> 8) as u8,
            seg as u8,
            (seg >> 8) as u8,
        ],
    );
    harness::write_bytes(&mut machine.bus, ctl_addr, &[6, 0, 0, 0, 0]);

    let seg_lo = (seg & 0xFF) as u8;
    let seg_hi = (seg >> 8) as u8;
    let req_lo = (req_off & 0xFF) as u8;
    let req_hi = (req_off >> 8) as u8;
    let strategy = tables::CDROM_MIRROR_STRATEGY_OFFSET;
    let strategy_lo = (strategy & 0xFF) as u8;
    let strategy_hi = (strategy >> 8) as u8;
    let interrupt = tables::CDROM_MIRROR_INTERRUPT_OFFSET;
    let interrupt_lo = (interrupt & 0xFF) as u8;
    let interrupt_hi = (interrupt >> 8) as u8;
    let cdrom_seg = tables::CDROM_MIRROR_HEADER_SEGMENT;
    let cdrom_seg_lo = (cdrom_seg & 0xFF) as u8;
    let cdrom_seg_hi = (cdrom_seg >> 8) as u8;
    #[rustfmt::skip]
    let code: Vec<u8> = vec![
        0xB8, seg_lo, seg_hi,               // MOV AX, seg
        0x8E, 0xC0,                         // MOV ES, AX
        0xBB, req_lo, req_hi,               // MOV BX, req_off
        0x9A, strategy_lo, strategy_hi, cdrom_seg_lo, cdrom_seg_hi,
                                                // CALL FAR strategy
        0x9A, interrupt_lo, interrupt_hi, cdrom_seg_lo, cdrom_seg_hi,
                                                // CALL FAR interrupt
        0xA1, 0x03, 0x02,                   // MOV AX, [0203h]
        0xA3, 0x00, 0x01,                   // MOV [0100h], AX
        0xA1, 0x21, 0x02,                   // MOV AX, [0221h]
        0xA3, 0x02, 0x01,                   // MOV [0102h], AX
        0xA1, 0x23, 0x02,                   // MOV AX, [0223h]
        0xA3, 0x04, 0x01,                   // MOV [0104h], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_generic_with_budget(
        &mut machine,
        &code,
        harness::INJECT_BUDGET_DISK_IO,
    );

    let request_status = harness::result_word(&machine.bus, 0);
    assert_eq!(
        request_status & 0x8100,
        0x0100,
        "CD-ROM direct request should finish without error, status={request_status:#06X}",
    );

    let flags_lo = harness::result_word(&machine.bus, 2);
    let flags_hi = harness::result_word(&machine.bus, 4);
    let device_status = u32::from(flags_lo) | (u32::from(flags_hi) << 16);
    assert!(
        device_status & 0x10 != 0,
        "Device status should report audio playback support, flags={device_status:#06X}",
    );
    assert_eq!(device_status & !0x0396, 0, "Reserved bits should be clear");
}

#[test]
fn open_con_as_character_device() {
    let mut machine = harness::boot_hle_with_floppy();

    let handle = open_file_with_mode(&mut machine, b"CON\0", 0x02);
    let sft_addr = sft_addr_for_handle(&mut machine, handle);

    assert_eq!(
        harness::read_word(&machine.bus, sft_addr + tables::SFT_ENT_OPEN_MODE),
        0x0002
    );
    assert_eq!(
        harness::read_word(&machine.bus, sft_addr + tables::SFT_ENT_DEV_INFO),
        tables::SFT_DEVINFO_DRIVER_CHAR
            | tables::SFT_DEVINFO_CHAR
            | tables::SFT_DEVINFO_EOF
            | tables::SFT_DEVINFO_SPECIAL
            | tables::SFT_DEVINFO_STDIN
            | tables::SFT_DEVINFO_STDOUT
    );
    assert_eq!(
        harness::read_word(&machine.bus, sft_addr + tables::SFT_ENT_DEV_PTR),
        tables::DEV_CON_OFFSET
    );
    assert_eq!(
        harness::read_word(&machine.bus, sft_addr + tables::SFT_ENT_DEV_PTR + 2),
        tables::DOS_DATA_SEGMENT
    );
    assert_eq!(
        harness::read_bytes(&machine.bus, sft_addr + tables::SFT_ENT_NAME, 11),
        b"CON        "
    );
}

#[test]
fn open_emmxxxx0_fails_without_ems() {
    let mut machine = harness::boot_hle_without_ems();

    let (ax, flags) = open_file_raw(&mut machine, b"EMMXXXX0\0", 0x00);
    assert_eq!(
        flags & 0x0001,
        0x0001,
        "CF should be 1, flags={:#06X}",
        flags
    );
    assert_eq!(ax, 0x0002);
}

#[test]
fn open_xmsxxxx0_fails_when_xms_disabled() {
    let mut machine = harness::boot_hle_without_xms();

    let (ax, flags) = open_file_raw(&mut machine, b"XMSXXXX0\0", 0x00);
    assert_eq!(
        flags & 0x0001,
        0x0001,
        "CF should be 1, flags={:#06X}",
        flags
    );
    assert_eq!(ax, 0x0002);
}

#[test]
fn open_existing_file_with_dot_component() {
    let mut machine = harness::boot_hle_with_floppy();

    let handle = open_file(&mut machine, b"A:\\.\\COMMAND.COM\0");
    assert!(
        handle >= 5,
        "File handle should be >= 5 (above standard handles), got {}",
        handle
    );
    close_file(&mut machine, handle);
}

#[test]
fn read_from_file() {
    let mut machine = harness::boot_hle_with_floppy();

    let handle = open_file(&mut machine, b"A:\\COMMAND.COM\0");

    let handle_lo = (handle & 0xFF) as u8;
    let handle_hi = (handle >> 8) as u8;

    #[rustfmt::skip]
    let code: Vec<u8> = vec![
        0xBB, handle_lo, handle_hi,          // MOV BX, handle
        0xB9, 0x02, 0x00,                   // MOV CX, 0002h
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
    harness::inject_and_run_with_budget(&mut machine, &code, harness::INJECT_BUDGET_DISK_IO);

    let flags = harness::result_word(&machine.bus, 2);
    assert_eq!(
        flags & 0x0001,
        0,
        "Read should succeed (CF=0), flags={:#06X}",
        flags
    );

    let bytes_read = harness::result_word(&machine.bus, 0);
    assert_eq!(
        bytes_read, 2,
        "Should have read 2 bytes, got {}",
        bytes_read
    );

    // Verify the content matches known COMMAND.COM test data
    let read_back = harness::read_bytes(&machine.bus, harness::INJECT_CODE_BASE + 0x210, 2);
    assert_eq!(
        &read_back,
        &harness::TEST_COMMAND_COM[..2],
        "Read data should match first 2 bytes of COMMAND.COM"
    );

    close_file(&mut machine, handle);
}

#[test]
fn read_from_file_uses_vram_access_page() {
    let mut machine = harness::boot_hle_with_floppy();

    machine.bus.io_write_byte(0xA6, 0x00);
    machine.bus.write_byte(0xA8000, 0xAA);
    machine.bus.io_write_byte(0xA6, 0x01);
    machine.bus.write_byte(0xA8000, 0xAA);

    let handle = open_file(&mut machine, b"A:\\TESTFILE.TXT\0");
    let handle_lo = (handle & 0xFF) as u8;
    let handle_hi = (handle >> 8) as u8;

    #[rustfmt::skip]
    let code: Vec<u8> = vec![
        0xBB, handle_lo, handle_hi,         // MOV BX, handle
        0x1E,                               // PUSH DS
        0xB8, 0x00, 0xA8,                   // MOV AX, A800h
        0x8E, 0xD8,                         // MOV DS, AX
        0xB9, 0x02, 0x00,                   // MOV CX, 0002h
        0x31, 0xD2,                         // XOR DX, DX
        0xB4, 0x3F,                         // MOV AH, 3Fh
        0xCD, 0x21,                         // INT 21h
        0x9C,                               // PUSHF
        0x50,                               // PUSH AX
        0x0E,                               // PUSH CS
        0x1F,                               // POP DS
        0x58,                               // POP AX
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0x58,                               // POP AX
        0xA3, 0x02, 0x01,                   // MOV [0x0102], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_with_budget(&mut machine, &code, harness::INJECT_BUDGET_DISK_IO);

    let flags = harness::result_word(&machine.bus, 2);
    assert_eq!(flags & 0x0001, 0, "read should succeed");
    assert_eq!(harness::result_word(&machine.bus, 0), 2);
    assert_eq!(
        machine.bus.graphics_vram()[0],
        0xAA,
        "page 0 must be untouched"
    );
    assert_eq!(
        machine.bus.graphics_vram()[0x18000],
        harness::TEST_FILE_CONTENT[0],
        "file data should land on graphics access page 1"
    );

    close_file(&mut machine, handle);
}

#[test]
fn open_read_from_cdrom_file() {
    let mut machine = harness::boot_hle_with_cdrom();

    let handle = open_file_ap(&mut machine, b"Q:\\README.TXT\0");
    let handle_lo = (handle & 0xFF) as u8;
    let handle_hi = (handle >> 8) as u8;

    #[rustfmt::skip]
    let code: Vec<u8> = vec![
        0xBB, handle_lo, handle_hi,          // MOV BX, handle
        0xB9, 0x05, 0x00,                   // MOV CX, 0005h
        0xBA, 0x10, 0x02,                   // MOV DX, 0210h
        0xB4, 0x3F,                         // MOV AH, 3Fh (read)
        0xCD, 0x21,                         // INT 21h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX (bytes read)
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x02, 0x01,                   // MOV [0x0102], AX (flags)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_generic_with_budget(
        &mut machine,
        &code,
        harness::INJECT_BUDGET_DISK_IO,
    );

    let flags = harness::result_word(&machine.bus, 2);
    assert_eq!(
        flags & 0x0001,
        0,
        "CD-ROM read should succeed, flags={:#06X}",
        flags
    );
    assert_eq!(
        harness::result_word(&machine.bus, 0),
        5,
        "CD-ROM read should return 5 bytes"
    );

    let read_back = harness::read_bytes(&machine.bus, harness::INJECT_CODE_BASE + 0x210, 5);
    assert_eq!(&read_back, &harness::TEST_CDROM_README[..5]);
}

#[test]
fn open_read_from_multifile_mode2_cdrom_file() {
    let temp_cdrom_files = harness::write_temp_mode2_multi_file_cdrom("mode2_multifile");
    let mut machine = harness::boot_hle_with_cdrom_path(&temp_cdrom_files.cue_path);

    let handle = open_file_ap(&mut machine, b"Q:\\SETUP.EXE\0");
    let handle_lo = (handle & 0xFF) as u8;
    let handle_hi = (handle >> 8) as u8;

    #[rustfmt::skip]
    let code: Vec<u8> = vec![
        0xBB, handle_lo, handle_hi,
        0xB9, 0x02, 0x00,
        0xBA, 0x10, 0x02,
        0xB4, 0x3F,
        0xCD, 0x21,
        0xA3, 0x00, 0x01,
        0x9C,
        0x58,
        0xA3, 0x02, 0x01,
        0xFA,
        0xF4,
    ];
    harness::inject_and_run_generic_with_budget(
        &mut machine,
        &code,
        harness::INJECT_BUDGET_DISK_IO,
    );

    let flags = harness::result_word(&machine.bus, 2);
    assert_eq!(
        flags & 0x0001,
        0,
        "synthetic mode2 CD-ROM read should succeed, flags={:#06X}",
        flags
    );
    assert_eq!(
        harness::result_word(&machine.bus, 0),
        2,
        "synthetic mode2 CD-ROM read should return 2 bytes"
    );

    let read_back = harness::read_bytes(&machine.bus, harness::INJECT_CODE_BASE + 0x210, 2);
    assert_eq!(&read_back, b"MZ");
}

#[test]
fn lseek_file_size() {
    let mut machine = harness::boot_hle_with_floppy();

    let handle = open_file(&mut machine, b"A:\\COMMAND.COM\0");
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
    harness::inject_and_run_with_budget(&mut machine, &code, harness::INJECT_BUDGET_DISK_IO);

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
        harness::TEST_COMMAND_COM.len() as u32,
        "COMMAND.COM file size should match test data length"
    );

    close_file(&mut machine, handle);
}

#[test]
fn create_write_close_delete() {
    let mut machine = harness::boot_hle_with_floppy();

    let filename = b"A:\\TEST.TMP\0";
    let handle = create_file(&mut machine, filename);
    assert!(handle >= 5, "Created handle should be >= 5, got {}", handle);

    // Write 5 bytes.
    let write_data = b"HELLO";
    let data_addr = harness::INJECT_CODE_BASE + 0x210;
    harness::write_bytes(&mut machine.bus, data_addr, write_data);
    let handle_lo = (handle & 0xFF) as u8;
    let handle_hi = (handle >> 8) as u8;
    #[rustfmt::skip]
    let write_code: Vec<u8> = vec![
        0xBB, handle_lo, handle_hi,         // MOV BX, handle
        0xB9, 0x05, 0x00,                   // MOV CX, 0005h
        0xBA, 0x10, 0x02,                   // MOV DX, 0210h
        0xB4, 0x40,                         // MOV AH, 40h (write)
        0xCD, 0x21,                         // INT 21h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX (bytes written)
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x02, 0x01,                   // MOV [0x0102], AX (flags)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_with_budget(&mut machine, &write_code, harness::INJECT_BUDGET_DISK_IO);

    let write_flags = harness::result_word(&machine.bus, 2);
    assert_eq!(
        write_flags & 0x0001,
        0,
        "Write should succeed (CF=0), flags={:#06X}",
        write_flags
    );
    let bytes_written = harness::result_word(&machine.bus, 0);
    assert_eq!(
        bytes_written, 5,
        "Should have written 5 bytes, got {}",
        bytes_written
    );

    close_file(&mut machine, handle);
    delete_file(&mut machine, filename);
}

#[test]
fn create_write_seek_read_verify() {
    let mut machine = harness::boot_hle_with_floppy();

    let filename = b"A:\\TEST2.TMP\0";
    let handle = create_file(&mut machine, filename);
    let handle_lo = (handle & 0xFF) as u8;
    let handle_hi = (handle >> 8) as u8;

    // Write "HELLO".
    let write_data = b"HELLO";
    let data_addr = harness::INJECT_CODE_BASE + 0x210;
    harness::write_bytes(&mut machine.bus, data_addr, write_data);
    #[rustfmt::skip]
    let write_code: Vec<u8> = vec![
        0xBB, handle_lo, handle_hi,
        0xB9, 0x05, 0x00,
        0xBA, 0x10, 0x02,
        0xB4, 0x40,
        0xCD, 0x21,
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_with_budget(&mut machine, &write_code, harness::INJECT_BUDGET_DISK_IO);

    // Seek to start.
    #[rustfmt::skip]
    let seek_code: Vec<u8> = vec![
        0xBB, handle_lo, handle_hi,
        0xB4, 0x42,
        0xB0, 0x00,                         // AL=0 (from start)
        0xB9, 0x00, 0x00,
        0xBA, 0x00, 0x00,
        0xCD, 0x21,
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_with_budget(&mut machine, &seek_code, harness::INJECT_BUDGET_DISK_IO);

    // Read 5 bytes.
    #[rustfmt::skip]
    let read_code: Vec<u8> = vec![
        0xBB, handle_lo, handle_hi,
        0xB9, 0x05, 0x00,
        0xBA, 0x20, 0x02,                   // buffer at 0x0220
        0xB4, 0x3F,
        0xCD, 0x21,
        0xA3, 0x00, 0x01,                   // bytes read
        0x9C, 0x58,
        0xA3, 0x02, 0x01,                   // flags
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_with_budget(&mut machine, &read_code, harness::INJECT_BUDGET_DISK_IO);

    let read_flags = harness::result_word(&machine.bus, 2);
    assert_eq!(
        read_flags & 0x0001,
        0,
        "Read should succeed (CF=0), flags={:#06X}",
        read_flags
    );
    let bytes_read = harness::result_word(&machine.bus, 0);
    assert_eq!(
        bytes_read, 5,
        "Should read back 5 bytes, got {}",
        bytes_read
    );

    let read_back = harness::read_bytes(&machine.bus, harness::INJECT_CODE_BASE + 0x220, 5);
    assert_eq!(
        &read_back, b"HELLO",
        "Read-back data should match, got {:?}",
        read_back
    );

    close_file(&mut machine, handle);
    delete_file(&mut machine, filename);
}

#[test]
fn get_file_attributes() {
    let mut machine = harness::boot_hle_with_floppy();

    let path_addr = harness::INJECT_CODE_BASE + 0x200;
    harness::write_bytes(&mut machine.bus, path_addr, b"A:\\COMMAND.COM\0");

    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x43,                         // MOV AH, 43h
        0xB0, 0x00,                         // MOV AL, 00h (get)
        0xBA, 0x00, 0x02,                   // MOV DX, 0200h
        0xCD, 0x21,                         // INT 21h
        0x89, 0x0E, 0x00, 0x01,             // MOV [0x0100], CX
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x02, 0x01,                   // MOV [0x0102], AX (flags)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_with_budget(&mut machine, code, harness::INJECT_BUDGET_DISK_IO);

    let flags = harness::result_word(&machine.bus, 2);
    assert_eq!(
        flags & 0x0001,
        0,
        "Get attributes should succeed (CF=0), flags={:#06X}",
        flags
    );

    // INT 21h/43h returns attributes in CL only; CH is undefined.
    let attributes = harness::result_byte(&machine.bus, 0);
    assert_eq!(
        attributes & 0xC0,
        0,
        "File attributes should use only bits 0-5, got {:#04X}",
        attributes
    );
    assert_ne!(
        attributes & 0x20,
        0,
        "COMMAND.COM should have archive attribute set, got {:#04X}",
        attributes
    );
}

#[test]
fn create_directory_via_int21h_39h() {
    let mut machine = harness::boot_hle_with_floppy();

    let create_path_addr = harness::INJECT_CODE_BASE + 0x200;
    let attrs_path_addr = harness::INJECT_CODE_BASE + 0x220;
    harness::write_bytes(&mut machine.bus, create_path_addr, b"A:\\NEWDIR\0");
    harness::write_bytes(&mut machine.bus, attrs_path_addr, b"A:\\NEWDIR\0");

    #[rustfmt::skip]
    let code: &[u8] = &[
        0xBA, 0x00, 0x02,                   // MOV DX, 0200h
        0xB4, 0x39,                         // MOV AH, 39h
        0xCD, 0x21,                         // INT 21h
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX (create flags)
        0xBA, 0x20, 0x02,                   // MOV DX, 0220h
        0xB4, 0x43,                         // MOV AH, 43h
        0xB0, 0x00,                         // MOV AL, 00h
        0xCD, 0x21,                         // INT 21h
        0x89, 0x0E, 0x02, 0x01,             // MOV [0x0102], CX (attributes)
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x04, 0x01,                   // MOV [0x0104], AX (attr flags)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_with_budget(&mut machine, code, harness::INJECT_BUDGET_DISK_IO);

    let create_flags = harness::result_word(&machine.bus, 0);
    assert_eq!(
        create_flags & 0x0001,
        0,
        "AH=39h should succeed (CF=0), flags={:#06X}",
        create_flags
    );

    let attr_flags = harness::result_word(&machine.bus, 4);
    assert_eq!(
        attr_flags & 0x0001,
        0,
        "New directory should be queryable via AH=43h, flags={:#06X}",
        attr_flags
    );

    let attributes = harness::result_byte(&machine.bus, 2);
    assert_ne!(
        attributes & 0x10,
        0,
        "NEWDIR should have directory attribute set, got {:#04X}",
        attributes
    );
}

#[test]
fn create_directory_via_int21h_39h_existing_returns_access_denied() {
    let mut machine = harness::boot_hle_with_floppy();

    let path_addr = harness::INJECT_CODE_BASE + 0x200;
    harness::write_bytes(&mut machine.bus, path_addr, b"A:\\DUPDIR\0");

    #[rustfmt::skip]
    let code: &[u8] = &[
        0xBA, 0x00, 0x02,                   // MOV DX, 0200h
        0xB4, 0x39,                         // MOV AH, 39h
        0xCD, 0x21,                         // INT 21h
        0xBA, 0x00, 0x02,                   // MOV DX, 0200h
        0xB4, 0x39,                         // MOV AH, 39h
        0xCD, 0x21,                         // INT 21h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x02, 0x01,                   // MOV [0x0102], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_with_budget(&mut machine, code, harness::INJECT_BUDGET_DISK_IO);

    let error_code = harness::result_word(&machine.bus, 0);
    let flags = harness::result_word(&machine.bus, 2);
    assert_eq!(
        flags & 0x0001,
        1,
        "Second AH=39h should fail (CF=1), flags={:#06X}",
        flags
    );
    assert_eq!(
        error_code, 0x0005,
        "Creating an existing directory should return access denied, got {:#06X}",
        error_code
    );
}

#[test]
fn find_next_uses_restored_dta_directory() {
    let hdd = harness::create_test_hdd(256);
    let mut machine = harness::boot_hle_with_forced_os(None, Some(hdd));

    run_mkdir(&mut machine, b"A:\\ONE\0");
    run_mkdir(&mut machine, b"A:\\TWO\0");
    run_create_file_with_payload(&mut machine, b"A:\\ONE\\ONE1.TXT\0");
    run_create_file_with_payload(&mut machine, b"A:\\TWO\\TWO1.TXT\0");
    run_create_file_with_payload(&mut machine, b"A:\\TWO\\TWO2.TXT\0");

    harness::write_bytes(
        &mut machine.bus,
        harness::INJECT_CODE_BASE + 0x200,
        b"A:\\ONE\\*.TXT\0",
    );
    harness::write_bytes(
        &mut machine.bus,
        harness::INJECT_CODE_BASE + 0x240,
        b"A:\\TWO\\*.TXT\0",
    );
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xBA, 0x00, 0x03,                   // MOV DX, 0300h
        0xB4, 0x1A,                         // MOV AH, 1Ah
        0xCD, 0x21,                         // INT 21h
        0xBA, 0x00, 0x02,                   // MOV DX, 0200h
        0xB9, 0x00, 0x00,                   // MOV CX, 0000h
        0xB4, 0x4E,                         // MOV AH, 4Eh
        0xCD, 0x21,                         // INT 21h
        0xA3, 0x00, 0x01,                   // MOV [0100h], AX
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x02, 0x01,                   // MOV [0102h], AX
        0xBA, 0x00, 0x04,                   // MOV DX, 0400h
        0xB4, 0x1A,                         // MOV AH, 1Ah
        0xCD, 0x21,                         // INT 21h
        0xBA, 0x40, 0x02,                   // MOV DX, 0240h
        0xB9, 0x00, 0x00,                   // MOV CX, 0000h
        0xB4, 0x4E,                         // MOV AH, 4Eh
        0xCD, 0x21,                         // INT 21h
        0xA3, 0x04, 0x01,                   // MOV [0104h], AX
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x06, 0x01,                   // MOV [0106h], AX
        0xBA, 0x00, 0x03,                   // MOV DX, 0300h
        0xB4, 0x1A,                         // MOV AH, 1Ah
        0xCD, 0x21,                         // INT 21h
        0xB4, 0x4F,                         // MOV AH, 4Fh
        0xCD, 0x21,                         // INT 21h
        0xA3, 0x08, 0x01,                   // MOV [0108h], AX
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x0A, 0x01,                   // MOV [010Ah], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_with_budget(&mut machine, code, harness::INJECT_BUDGET_DISK_IO);

    let first_search_flags = harness::result_word(&machine.bus, 2);
    assert_eq!(
        first_search_flags & 0x0001,
        0,
        "first FINDFIRST should succeed, flags={first_search_flags:#06X}"
    );

    let second_search_flags = harness::result_word(&machine.bus, 6);
    assert_eq!(
        second_search_flags & 0x0001,
        0,
        "second FINDFIRST should succeed, flags={second_search_flags:#06X}"
    );

    let find_next_flags = harness::result_word(&machine.bus, 10);
    assert_ne!(
        find_next_flags & 0x0001,
        0,
        "restored DTA FINDNEXT should fail, flags={find_next_flags:#06X}"
    );

    let find_next_error = harness::result_word(&machine.bus, 8);
    assert_eq!(
        find_next_error, 0x0012,
        "restored DTA FINDNEXT should continue the original directory"
    );
}

fn run_mkdir(machine: &mut machine::Pc9801Ra, path: &[u8]) {
    harness::write_bytes(&mut machine.bus, harness::INJECT_CODE_BASE + 0x200, path);
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xBA, 0x00, 0x02,                   // MOV DX, 0200h
        0xB4, 0x39,                         // MOV AH, 39h
        0xCD, 0x21,                         // INT 21h
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x00, 0x01,                   // MOV [0100h], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_with_budget(machine, code, harness::INJECT_BUDGET_DISK_IO);

    let flags = harness::result_word(&machine.bus, 0);
    assert_eq!(
        flags & 0x0001,
        0,
        "MKDIR should succeed, flags={flags:#06X}"
    );
}

fn run_create_file_with_payload(machine: &mut machine::Pc9801Ra, path: &[u8]) {
    harness::write_bytes(&mut machine.bus, harness::INJECT_CODE_BASE + 0x200, path);
    harness::write_bytes(
        &mut machine.bus,
        harness::INJECT_CODE_BASE + 0x300,
        &[0xA5; 0x1200],
    );
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB9, 0x00, 0x00,                   // MOV CX, 0000h
        0xBA, 0x00, 0x02,                   // MOV DX, 0200h
        0xB4, 0x3C,                         // MOV AH, 3Ch
        0xCD, 0x21,                         // INT 21h
        0xA3, 0x00, 0x01,                   // MOV [0100h], AX
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x02, 0x01,                   // MOV [0102h], AX
        0x8B, 0x1E, 0x00, 0x01,             // MOV BX, [0100h]
        0xB9, 0x00, 0x12,                   // MOV CX, 1200h
        0xBA, 0x00, 0x03,                   // MOV DX, 0300h
        0xB4, 0x40,                         // MOV AH, 40h
        0xCD, 0x21,                         // INT 21h
        0xB4, 0x3E,                         // MOV AH, 3Eh
        0xCD, 0x21,                         // INT 21h
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_with_budget(machine, code, harness::INJECT_BUDGET_DISK_IO);

    let flags = harness::result_word(&machine.bus, 2);
    assert_eq!(
        flags & 0x0001,
        0,
        "create file {path:?} should succeed, flags={flags:#06X}"
    );
}

#[test]
fn create_directory_via_int21h_39h_missing_parent_returns_path_not_found() {
    let mut machine = harness::boot_hle_with_floppy();

    let path_addr = harness::INJECT_CODE_BASE + 0x200;
    harness::write_bytes(&mut machine.bus, path_addr, b"A:\\NOPE\\CHILD\0");

    #[rustfmt::skip]
    let code: &[u8] = &[
        0xBA, 0x00, 0x02,                   // MOV DX, 0200h
        0xB4, 0x39,                         // MOV AH, 39h
        0xCD, 0x21,                         // INT 21h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x02, 0x01,                   // MOV [0x0102], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_with_budget(&mut machine, code, harness::INJECT_BUDGET_DISK_IO);

    let error_code = harness::result_word(&machine.bus, 0);
    let flags = harness::result_word(&machine.bus, 2);
    assert_eq!(
        flags & 0x0001,
        1,
        "AH=39h with a missing parent should fail (CF=1), flags={:#06X}",
        flags
    );
    assert_eq!(
        error_code, 0x0003,
        "Missing parent path should return path not found, got {:#06X}",
        error_code
    );
}

#[test]
fn get_file_datetime() {
    let mut machine = harness::boot_hle_with_floppy();

    let handle = open_file(&mut machine, b"A:\\COMMAND.COM\0");
    let handle_lo = (handle & 0xFF) as u8;
    let handle_hi = (handle >> 8) as u8;

    #[rustfmt::skip]
    let code: Vec<u8> = vec![
        0xBB, handle_lo, handle_hi,         // MOV BX, handle
        0xB4, 0x57,                         // MOV AH, 57h
        0xB0, 0x00,                         // MOV AL, 00h (get)
        0xCD, 0x21,                         // INT 21h
        0x89, 0x0E, 0x00, 0x01,             // MOV [0x0100], CX (time)
        0x89, 0x16, 0x02, 0x01,             // MOV [0x0102], DX (date)
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x04, 0x01,                   // MOV [0x0104], AX (flags)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_with_budget(&mut machine, &code, harness::INJECT_BUDGET_DISK_IO);

    let flags = harness::result_word(&machine.bus, 4);
    assert_eq!(
        flags & 0x0001,
        0,
        "Get date/time should succeed (CF=0), flags={:#06X}",
        flags
    );

    let time = harness::result_word(&machine.bus, 0);
    let date = harness::result_word(&machine.bus, 2);
    assert_eq!(
        time,
        harness::TEST_FILE_TIME,
        "File time should match test floppy value"
    );
    assert_eq!(
        date,
        harness::TEST_FILE_DATE,
        "File date should match test floppy value"
    );

    close_file(&mut machine, handle);
}

#[test]
fn dup_file_handle() {
    let mut machine = harness::boot_hle_with_floppy();

    let handle = open_file(&mut machine, b"A:\\COMMAND.COM\0");
    let handle_lo = (handle & 0xFF) as u8;
    let handle_hi = (handle >> 8) as u8;

    // DUP the handle.
    #[rustfmt::skip]
    let dup_code: Vec<u8> = vec![
        0xBB, handle_lo, handle_hi,         // MOV BX, handle
        0xB4, 0x45,                         // MOV AH, 45h (DUP)
        0xCD, 0x21,                         // INT 21h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX (new handle)
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x02, 0x01,                   // MOV [0x0102], AX (flags)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_with_budget(&mut machine, &dup_code, harness::INJECT_BUDGET_DISK_IO);

    let flags = harness::result_word(&machine.bus, 2);
    assert_eq!(
        flags & 0x0001,
        0,
        "DUP should succeed (CF=0), flags={:#06X}",
        flags
    );

    let dup_handle = harness::result_word(&machine.bus, 0);
    assert_ne!(
        handle, dup_handle,
        "DUP'd handle ({}) should differ from original ({})",
        dup_handle, handle
    );

    // Read 1 byte from dup'd handle.
    let dup_lo = (dup_handle & 0xFF) as u8;
    let dup_hi = (dup_handle >> 8) as u8;
    #[rustfmt::skip]
    let read_code: Vec<u8> = vec![
        0xBB, dup_lo, dup_hi,
        0xB9, 0x01, 0x00,
        0xBA, 0x10, 0x02,
        0xB4, 0x3F,
        0xCD, 0x21,
        0xA3, 0x00, 0x01,                   // bytes read
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_with_budget(&mut machine, &read_code, harness::INJECT_BUDGET_DISK_IO);

    let bytes_read = harness::result_word(&machine.bus, 0);
    assert_eq!(
        bytes_read, 1,
        "Reading from DUP'd handle should return 1 byte, got {}",
        bytes_read
    );

    close_file(&mut machine, dup_handle);
    close_file(&mut machine, handle);
}

#[test]
fn ioctl_get_device_info_stdout() {
    let mut machine = harness::boot_hle_with_floppy();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x44,                         // MOV AH, 44h
        0xB0, 0x00,                         // MOV AL, 00h
        0xBB, 0x01, 0x00,                   // MOV BX, 0001h (stdout)
        0xCD, 0x21,                         // INT 21h
        0x89, 0x16, 0x00, 0x01,             // MOV [0x0100], DX
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x02, 0x01,                   // MOV [0x0102], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let flags = harness::result_word(&machine.bus, 2);
    assert_eq!(
        flags & 0x0001,
        0,
        "IOCTL on stdout should succeed (CF=0), flags={:#06X}",
        flags
    );

    let device_info = harness::result_word(&machine.bus, 0);
    assert_ne!(
        device_info & 0x0080,
        0,
        "Stdout IOCTL: bit 7 should be set (character device), got {:#06X}",
        device_info
    );
}

#[test]
fn ioctl_get_device_info_file() {
    let mut machine = harness::boot_hle_with_floppy();

    let handle = open_file(&mut machine, b"A:\\COMMAND.COM\0");
    let handle_lo = (handle & 0xFF) as u8;
    let handle_hi = (handle >> 8) as u8;

    #[rustfmt::skip]
    let code: Vec<u8> = vec![
        0xBB, handle_lo, handle_hi,         // MOV BX, handle
        0xB4, 0x44,                         // MOV AH, 44h
        0xB0, 0x00,                         // MOV AL, 00h
        0xCD, 0x21,                         // INT 21h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0x89, 0x16, 0x02, 0x01,             // MOV [0x0102], DX
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x04, 0x01,                   // MOV [0x0104], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_with_budget(&mut machine, &code, harness::INJECT_BUDGET_DISK_IO);

    let flags = harness::result_word(&machine.bus, 4);
    assert_eq!(
        flags & 0x0001,
        0,
        "IOCTL on file should succeed (CF=0), flags={:#06X}",
        flags
    );

    let device_info_ax = harness::result_word(&machine.bus, 0);
    let device_info_dx = harness::result_word(&machine.bus, 2);
    assert_eq!(
        device_info_ax, 0x0040,
        "File IOCTL should return the not-written bit for A: in AX"
    );
    assert_eq!(
        device_info_dx, 0x0040,
        "File IOCTL should return the not-written bit for A: in DX"
    );

    close_file(&mut machine, handle);
}

#[test]
fn ioctl_get_device_info_file_clears_not_written_after_zero_length_write() {
    let mut machine = harness::boot_hle_with_floppy();

    let handle = open_file_with_mode(&mut machine, b"A:\\TESTFILE.TXT\0", 0x02);
    let handle_lo = (handle & 0xFF) as u8;
    let handle_hi = (handle >> 8) as u8;

    #[rustfmt::skip]
    let ioctl_code: Vec<u8> = vec![
        0xBB, handle_lo, handle_hi,         // MOV BX, handle
        0xB4, 0x44,                         // MOV AH, 44h
        0xB0, 0x00,                         // MOV AL, 00h
        0xCD, 0x21,                         // INT 21h
        0x89, 0x16, 0x00, 0x01,             // MOV [0x0100], DX
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x02, 0x01,                   // MOV [0x0102], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_with_budget(&mut machine, &ioctl_code, harness::INJECT_BUDGET_DISK_IO);

    let flags = harness::result_word(&machine.bus, 2);
    assert_eq!(
        flags & 0x0001,
        0,
        "IOCTL on file should succeed (CF=0), flags={:#06X}",
        flags
    );

    let device_info = harness::result_word(&machine.bus, 0);
    assert_ne!(
        device_info & 0x0040,
        0,
        "File IOCTL should start with the not-written bit set, got {:#06X}",
        device_info
    );

    write_zero_bytes(&mut machine, handle);

    harness::inject_and_run_with_budget(&mut machine, &ioctl_code, harness::INJECT_BUDGET_DISK_IO);
    let flags_after_write = harness::result_word(&machine.bus, 2);
    assert_eq!(
        flags_after_write & 0x0001,
        0,
        "IOCTL after zero-length write should succeed (CF=0), flags={:#06X}",
        flags_after_write
    );

    let device_info_after_write = harness::result_word(&machine.bus, 0);
    assert_eq!(
        device_info_after_write, 0x0000,
        "File IOCTL should clear the not-written bit after a successful zero-length write on A:"
    );

    close_file(&mut machine, handle);
}

#[test]
fn findfirst_root_directory() {
    let mut machine = harness::boot_hle_with_floppy();

    let path_addr = harness::INJECT_CODE_BASE + 0x200;
    harness::write_bytes(&mut machine.bus, path_addr, b"A:\\*.*\0");

    #[rustfmt::skip]
    let code: &[u8] = &[
        // Set DTA to DS:0300h
        0xBA, 0x00, 0x03,
        0xB4, 0x1A,
        0xCD, 0x21,
        // FINDFIRST
        0xBA, 0x00, 0x02,
        0xB9, 0x00, 0x00,                   // CX=0 (normal files)
        0xB4, 0x4E,
        0xCD, 0x21,
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX (flags)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_with_budget(&mut machine, code, harness::INJECT_BUDGET_DISK_IO);

    let flags = harness::result_word(&machine.bus, 0);
    assert_eq!(
        flags & 0x0001,
        0,
        "FINDFIRST should find something (CF=0), flags={:#06X}",
        flags
    );

    let dta_base = harness::INJECT_CODE_BASE + 0x300;
    let attribute = harness::read_byte(&machine.bus, dta_base + 0x15);
    assert_eq!(
        attribute & 0xC0,
        0,
        "Found file attribute should use only bits 0-5, got {:#04X}",
        attribute
    );

    let filename = harness::read_string(&machine.bus, dta_base + 0x1E, 13);
    assert!(!filename.is_empty(), "Found filename should be non-empty");
}

#[test]
fn findfirst_writes_to_default_dta() {
    let mut machine = harness::boot_hle_with_floppy();

    let path_addr = harness::INJECT_CODE_BASE + 0x200;
    harness::write_bytes(&mut machine.bus, path_addr, b"A:\\*.*\0");

    #[rustfmt::skip]
    let code: &[u8] = &[
        // Read the default DTA pointer.
        0xB4, 0x2F,
        0xCD, 0x21,
        0x89, 0x1E, 0x00, 0x01,             // MOV [0100h], BX
        0x8C, 0x06, 0x02, 0x01,             // MOV [0102h], ES
        // FINDFIRST without changing the DTA.
        0xBA, 0x00, 0x02,
        0xB9, 0x00, 0x00,
        0xB4, 0x4E,
        0xCD, 0x21,
        0x9C,
        0x58,
        0xA3, 0x04, 0x01,                   // MOV [0104h], AX
        0xFA,
        0xF4,
    ];
    harness::inject_and_run_with_budget(&mut machine, code, harness::INJECT_BUDGET_DISK_IO);

    let dta_offset = harness::result_word(&machine.bus, 0);
    let dta_segment = harness::result_word(&machine.bus, 2);
    let flags = harness::result_word(&machine.bus, 4);
    assert_eq!(
        flags & 0x0001,
        0,
        "FINDFIRST should succeed (CF=0), flags={flags:#06X}"
    );

    let dta_base = harness::far_to_linear(dta_segment, dta_offset);
    let filename = harness::read_string(&machine.bus, dta_base + 0x1E, 13);
    assert!(
        !filename.is_empty(),
        "FINDFIRST should write the result filename at the default DTA"
    );
}

#[test]
fn findnext_after_findfirst() {
    let mut machine = harness::boot_hle_with_floppy();

    let path_addr = harness::INJECT_CODE_BASE + 0x200;
    harness::write_bytes(&mut machine.bus, path_addr, b"A:\\*.*\0");

    // FINDFIRST + FINDNEXT in single injection (both use same DTA).
    #[rustfmt::skip]
    let code: &[u8] = &[
        // Set DTA
        0xBA, 0x00, 0x03,
        0xB4, 0x1A,
        0xCD, 0x21,
        // FINDFIRST
        0xBA, 0x00, 0x02,
        0xB9, 0x00, 0x00,
        0xB4, 0x4E,
        0xCD, 0x21,
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x00, 0x01,                   // findfirst flags
        // FINDNEXT
        0xB4, 0x4F,
        0xCD, 0x21,
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x02, 0x01,                   // findnext flags
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run_with_budget(&mut machine, code, harness::INJECT_BUDGET_DISK_IO);

    let ff_flags = harness::result_word(&machine.bus, 0);
    assert_eq!(
        ff_flags & 0x0001,
        0,
        "FINDFIRST should succeed (CF=0), flags={:#06X}",
        ff_flags
    );

    let fn_flags = harness::result_word(&machine.bus, 2);
    assert_eq!(
        fn_flags & 0x0001,
        0,
        "FINDNEXT should find a second entry (CF=0), flags={:#06X}",
        fn_flags
    );
}

/// Runs INT 21h AX=4400h (IOCTL get device info) for the given handle and
/// returns (AX, DX, flags).
fn ioctl_get_device_info(machine: &mut machine::Pc9801Ra, handle: u8) -> (u16, u16, u16) {
    #[rustfmt::skip]
    let mut code: Vec<u8> = vec![
        0xB8, 0x00, 0x44,                   // MOV AX, 4400h
        0xBB, 0x00, 0x00,                   // MOV BX, handle
        0xCD, 0x21,                         // INT 21h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0x89, 0x16, 0x02, 0x01,             // MOV [0x0102], DX
        0x9C,                               // PUSHF
        0x58,                               // POP AX
        0xA3, 0x04, 0x01,                   // MOV [0x0104], AX (flags)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    code[4] = handle;
    harness::inject_and_run(machine, &code);
    (
        harness::result_word(&machine.bus, 0),
        harness::result_word(&machine.bus, 2),
        harness::result_word(&machine.bus, 4),
    )
}

#[test]
fn ioctl_get_device_info_standard_handles_set_bit15() {
    let mut machine = harness::boot_hle();

    // stdin(0), stdout(1), stderr(2) are all pre-opened to CON. Real DOS 6.20
    // returns the character-device SFT word 0x80D3 (bit 15 + bit 7 + bit 6 +
    // SPECIAL|STDIN|STDOUT). TurboC's conio checks bit 15 to detect the console.
    for handle in [0u8, 1, 2] {
        let (ax, dx, flags) = ioctl_get_device_info(&mut machine, handle);
        assert_eq!(
            ax, 0x80D3,
            "AX=4400h handle {handle} must return 0x80D3 (bit 15 set), got {ax:#06X}"
        );
        assert_eq!(dx, ax, "AX=4400h handle {handle} must return DX == AX");
        assert_eq!(
            flags & 0x0001,
            0,
            "AX=4400h handle {handle} must clear CF, flags={flags:#06X}"
        );
    }
}
