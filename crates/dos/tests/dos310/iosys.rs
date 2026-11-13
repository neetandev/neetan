use dos::tables::{
    IOSYS_BASE, IOSYS_OFF_DAUA_PTR, IOSYS_OFF_EXT_DAUA_TABLE, IOSYS_OFF_INTERNAL_REVISION,
    IOSYS_OFF_PRODUCT_NUMBER, IOSYS_OFF_SPECIAL_INPUT,
};

use crate::harness::*;

/// `BX` in front of an `INT DCh` call.
const BX_PRESET: u16 = 0x1234;
/// `DX` in front of an `INT DCh` call.
const DX_PRESET: u16 = 0x5678;

/// Calls `INT DCh` with `cl` and `ax` and returns `[AX, BX, CX, DX]`.
fn call_intdch(machine: &mut Machine, cl: u8, ax: u16) -> [u16; 4] {
    let [ax_low, ax_high] = ax.to_le_bytes();
    let [bx_low, bx_high] = BX_PRESET.to_le_bytes();
    let [dx_low, dx_high] = DX_PRESET.to_le_bytes();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, ax_low, ax_high,  // MOV AX, ax
        0xBB, bx_low, bx_high,  // MOV BX, BX_PRESET
        0xB9, cl, 0x00,         // MOV CX, cl
        0xBA, dx_low, dx_high,  // MOV DX, DX_PRESET
        0xCD, 0xDC,             // INT DCh
        0xA3, 0x00, 0x01,       // MOV [0100h], AX
        0x89, 0x1E, 0x02, 0x01, // MOV [0102h], BX
        0x89, 0x0E, 0x04, 0x01, // MOV [0104h], CX
        0x89, 0x16, 0x06, 0x01, // MOV [0106h], DX
        0xFA,                   // CLI
        0xF4,                   // HLT
    ];
    inject_and_run(machine, code);
    [0, 2, 4, 6].map(|offset| result_word(&machine.bus, offset))
}

#[test]
fn system_identification_returns_the_ps98_011_values() {
    let mut machine = boot();
    assert_eq!(
        call_intdch(&mut machine, 0x12, 0x0000),
        [0x0001, BX_PRESET, 0x0012, 0x0003]
    );
}

#[test]
fn drive_table_buffer_lists_the_drives() {
    let mut machine = boot_with_floppy(create_floppy(&standard_files()));
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB9, 0x13, 0x00,       // MOV CX, 0013h
        0xBA, 0x00, 0x02,       // MOV DX, 0200h
        0xCD, 0xDC,             // INT DCh
        0xFA,                   // CLI
        0xF4,                   // HLT
    ];
    inject_and_run(&mut machine, code);
    let buffer = read_bytes(&machine.bus, INJECT_DATA_BASE, 0x60);
    assert_eq!(buffer[0x00], 0x90, "A: is the first 2HD floppy");
    assert_eq!(buffer[0x1A..0x1C], [0x00, 0x90]);
}

#[test]
fn younger_intdch_functions_change_no_register() {
    let mut machine = boot();
    for (cl, ax) in [
        (0x0F, 0x8000),
        (0x0F, 0x8002),
        (0x15, 0xAB00),
        (0x81, 0xAB00),
    ] {
        assert_eq!(
            call_intdch(&mut machine, cl, ax),
            [ax, BX_PRESET, u16::from(cl), DX_PRESET],
            "CL={cl:02X}h AX={ax:04X}h"
        );
    }
}

#[test]
fn work_area_has_no_product_number_and_no_revision() {
    let machine = boot();
    assert_eq!(
        read_word(&machine.bus, IOSYS_BASE + IOSYS_OFF_PRODUCT_NUMBER),
        0
    );
    assert_eq!(
        read_byte(&machine.bus, IOSYS_BASE + IOSYS_OFF_INTERNAL_REVISION),
        0
    );
}

#[test]
fn drive_table_pointer_points_to_the_table_of_all_drives() {
    let machine = boot();
    assert_eq!(
        read_far_pointer(&machine.bus, IOSYS_BASE + IOSYS_OFF_DAUA_PTR),
        (0x0060, IOSYS_OFF_EXT_DAUA_TABLE as u16)
    );
}

/// Size of the buffer of `INT DCh CL=80h`.
const DISK_INFO_BUFFER_SIZE: usize = 0x100;
/// First physical sector of the partition of the test hard disk. The
/// partition starts at cylinder 1 of a disk with 8 heads and 17 sectors per
/// track.
const TEST_HDD_PARTITION_START: u32 = 136;

/// Fills the data area with FFh, calls `INT DCh CL=80h` with `ax` and DS:DX
/// at the data area. Returns `[AX, BX, CX, DX]` and the buffer.
fn call_disk_information(machine: &mut Machine, ax: u16) -> ([u16; 4], Vec<u8>) {
    write_bytes(
        &mut machine.bus,
        INJECT_DATA_BASE,
        &[0xFF; DISK_INFO_BUFFER_SIZE],
    );
    let [ax_low, ax_high] = ax.to_le_bytes();
    let [bx_low, bx_high] = BX_PRESET.to_le_bytes();
    let [dx_low, dx_high] = INJECT_DATA_OFFSET.to_le_bytes();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, ax_low, ax_high,  // MOV AX, ax
        0xBB, bx_low, bx_high,  // MOV BX, BX_PRESET
        0xB9, 0x80, 0x00,       // MOV CX, 0080h
        0xBA, dx_low, dx_high,  // MOV DX, INJECT_DATA_OFFSET
        0xCD, 0xDC,             // INT DCh
        0xA3, 0x00, 0x01,       // MOV [0100h], AX
        0x89, 0x1E, 0x02, 0x01, // MOV [0102h], BX
        0x89, 0x0E, 0x04, 0x01, // MOV [0104h], CX
        0x89, 0x16, 0x06, 0x01, // MOV [0106h], DX
        0xFA,                   // CLI
        0xF4,                   // HLT
    ];
    inject_and_run(machine, code);
    let registers = [0, 2, 4, 6].map(|offset| result_word(&machine.bus, offset));
    (
        registers,
        read_bytes(&machine.bus, INJECT_DATA_BASE, DISK_INFO_BUFFER_SIZE),
    )
}

#[test]
fn disk_information_lists_the_drives_and_the_partition_start() {
    let mut machine = boot_with_floppy_and_hdd(
        create_floppy(&standard_files()),
        create_hdd(&standard_files()),
    );
    let (registers, buffer) = call_disk_information(&mut machine, 0x0000);
    assert_eq!(registers, [0x0000, BX_PRESET, 0x0080, INJECT_DATA_OFFSET]);
    let mut expected = vec![0u8; DISK_INFO_BUFFER_SIZE];
    expected[0x00] = 1;
    expected[0x0C] = 1;
    expected[0x0E] = 1;
    expected[0x10..0x14].copy_from_slice(&TEST_HDD_PARTITION_START.to_le_bytes());
    assert_eq!(buffer, expected);
}

#[test]
fn disk_information_subfunctions_report_no_branch_4670_drive_and_no_fep() {
    let mut machine = boot();
    for ax in [0x8000, 0x8001, 0x8002] {
        let (registers, buffer) = call_disk_information(&mut machine, ax);
        assert_eq!(
            registers,
            [0x0000, BX_PRESET, 0x0080, INJECT_DATA_OFFSET],
            "AX={ax:04X}h"
        );
        assert!(buffer.iter().all(|&byte| byte == 0xFF), "AX={ax:04X}h");
    }
}

/// Bytes of the work area from `0060:0106h` that CL=80h AX=0001h copies.
const CONSOLE_STATE_SIZE: usize = 0x1A;

#[test]
fn disk_information_copies_the_console_state() {
    let mut machine = boot();
    let (registers, buffer) = call_disk_information(&mut machine, 0x0001);
    assert_eq!(registers, [0x0001, BX_PRESET, 0x0080, INJECT_DATA_OFFSET]);
    let state = read_bytes(
        &machine.bus,
        IOSYS_BASE + IOSYS_OFF_SPECIAL_INPUT,
        CONSOLE_STATE_SIZE,
    );
    assert_eq!(buffer[..CONSOLE_STATE_SIZE], state[..]);
    assert!(buffer[CONSOLE_STATE_SIZE..].iter().all(|&byte| byte == 0));
}
