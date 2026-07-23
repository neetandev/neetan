//! INT 13h hard disk services. The per-call register files and the hard disk
//! BDA state asserted here are the ones the real AMI BIOS produces, taken from
//! side-by-side captures against the real ct486 ROM set.

use common::NoTrace;
use device::disk::{HddSizeType, blank_hdd_image};
use machine_at::AtMachine;

use super::{
    RESULT, boot_and_run, create_machine_dx50, create_machine_dx66, inject_and_run,
    make_pattern_hdd, read_ram_u8, read_ram_u16, write_bytes,
};

/// FLAGS bit 0: carry.
const FLAGS_CARRY: u16 = 0x0001;
/// Guest scratch buffer for sector transfers.
const DATA_BUFFER: u32 = 0x8000;
/// Second guest scratch buffer for read-back comparisons.
const READ_BACK_BUFFER: u32 = 0xA000;
/// Cycle budget for one injected INT 13h script against the HLE BIOS.
const INT13H_BUDGET: u64 = 40_000_000;

/// Byte stride of one snapshot slot in the script result block.
const SNAPSHOT_STRIDE: u32 = 20;

/// One INT 13h call of a script: the register file loaded before the INT
/// instruction.
#[derive(Clone, Copy)]
struct Int13hCall {
    ax: u16,
    cx: u16,
    dx: u16,
    es: u16,
    bx: u16,
    di: u16,
}

impl Int13hCall {
    /// A call with poisoned pointer registers, so untouched registers stay
    /// visible in the snapshot.
    const fn poisoned(ax: u16, cx: u16, dx: u16) -> Self {
        Self {
            ax,
            cx,
            dx,
            es: 0x4321,
            bx: 0x5678,
            di: 0x8899,
        }
    }
}

/// Builds a guest program that runs every call in order, storing the full
/// result of call `n` in a 20-byte slot at RESULT + n * 20, then halts.
/// Slot layout: +0 AX, +2 FLAGS, +4 CX, +6 DX, +8 ES, +10 DI, +12 BX,
/// +14 BDA 40:74, +15 40:75, +16 40:8E.
fn int13h_snapshot_script(calls: &[Int13hCall]) -> Vec<u8> {
    let mut code = Vec::new();
    for (index, call) in calls.iter().enumerate() {
        let slot = RESULT as u16 + index as u16 * SNAPSHOT_STRIDE as u16;
        let word = |value: u16| [value as u8, (value >> 8) as u8];
        code.extend_from_slice(&[0xB8, call.es as u8, (call.es >> 8) as u8]);
        code.extend_from_slice(&[0x8E, 0xC0]);
        code.extend_from_slice(&[0xBB, call.bx as u8, (call.bx >> 8) as u8]);
        code.extend_from_slice(&[0xB9, call.cx as u8, (call.cx >> 8) as u8]);
        code.extend_from_slice(&[0xBA, call.dx as u8, (call.dx >> 8) as u8]);
        code.extend_from_slice(&[0xBF, call.di as u8, (call.di >> 8) as u8]);
        code.extend_from_slice(&[0xB8, call.ax as u8, (call.ax >> 8) as u8]);
        code.extend_from_slice(&[0xCD, 0x13]);
        code.push(0xA3);
        code.extend_from_slice(&word(slot));
        code.extend_from_slice(&[0x9C, 0x58]);
        code.push(0xA3);
        code.extend_from_slice(&word(slot + 2));
        code.extend_from_slice(&[0x89, 0x0E]);
        code.extend_from_slice(&word(slot + 4));
        code.extend_from_slice(&[0x89, 0x16]);
        code.extend_from_slice(&word(slot + 6));
        code.extend_from_slice(&[0x8C, 0xC0]);
        code.push(0xA3);
        code.extend_from_slice(&word(slot + 8));
        code.extend_from_slice(&[0x89, 0x3E]);
        code.extend_from_slice(&word(slot + 10));
        code.extend_from_slice(&[0x89, 0x1E]);
        code.extend_from_slice(&word(slot + 12));
        for (bda, offset) in [(0x0474u16, 14u16), (0x0475, 15), (0x048E, 16)] {
            code.extend_from_slice(&[0xA0, bda as u8, (bda >> 8) as u8]);
            code.push(0xA2);
            code.extend_from_slice(&word(slot + offset));
        }
    }
    // Park in CLI;HLT with a jump back: the real BIOS returns from INT 13h
    // with interrupts enabled, so a bare HLT would fall through on the next
    // timer tick and run off into uninitialized memory.
    code.extend_from_slice(&[0xFA, 0xF4, 0xEB, 0xFD]);
    code
}

/// Linear address of script slot `slot`.
fn slot_base(slot: u32) -> u32 {
    RESULT + slot * SNAPSHOT_STRIDE
}

/// Asserts the AX and carry stored in script slot `slot`.
fn assert_slot(machine: &AtMachine<NoTrace>, slot: u32, ax: u16, carry: bool, label: &str) {
    assert_eq!(read_ram_u16(machine, slot_base(slot)), ax, "{label}: AX");
    assert_eq!(
        read_ram_u16(machine, slot_base(slot) + 2) & FLAGS_CARRY != 0,
        carry,
        "{label}: carry"
    );
}

/// Asserts the BDA 40:74 status byte stored in script slot `slot`.
fn assert_slot_status(machine: &AtMachine<NoTrace>, slot: u32, status: u8, label: &str) {
    assert_eq!(
        read_ram_u8(machine, slot_base(slot) + 14),
        status,
        "{label}: 40:74"
    );
}

/// Asserts the 24-bit pattern stamp of image sector `unit` at `address`.
fn assert_pattern_sector(machine: &AtMachine<NoTrace>, address: u32, unit: u32, label: &str) {
    let stored = u32::from(read_ram_u16(machine, address))
        | (u32::from(read_ram_u8(machine, address + 2)) << 16);
    assert_eq!(stored, unit, "{label}: unit stamp");
    assert_eq!(read_ram_u8(machine, address + 3), 0xA5, "{label}: marker");
    assert_eq!(
        read_ram_u8(machine, address + 4),
        unit as u8 ^ 0x5A,
        "{label}: fill"
    );
}

/// Packs a CHS address into the CX register file value.
fn chs_cx(cylinder: u16, sector: u8) -> u16 {
    (cylinder << 8) | u16::from(sector & 0x3F) | ((cylinder >> 2) & 0xC0)
}

/// Boots a blank disk of `size` and checks the AH=08h and AH=15h register
/// files against the image geometry.
fn drive_parameters_case(size: HddSizeType) {
    let (geometry, _) = size.geometry();
    let mut machine = create_machine_dx50();
    machine
        .bus
        .insert_hdd(0, blank_hdd_image(size), None)
        .expect("insert blank hard disk");
    let script = int13h_snapshot_script(&[
        Int13hCall::poisoned(0x0800, 0x1234, 0x0080),
        Int13hCall::poisoned(0x1500, 0x1234, 0x0080),
    ]);
    boot_and_run(&mut machine, &script, &[], INT13H_BUDGET);

    // AH=08h: the last cylinder is reserved for diagnostics, so the maximum
    // usable cylinder number is the count minus two. AL carries the sectors
    // per track like the probed AMI handler.
    let maximum_cylinder = geometry.cylinders - 2;
    assert_slot(
        &machine,
        0,
        u16::from(geometry.sectors_per_track),
        false,
        "AH=08h",
    );
    assert_eq!(
        read_ram_u16(&machine, slot_base(0) + 4),
        chs_cx(maximum_cylinder, geometry.sectors_per_track),
        "AH=08h: CX"
    );
    assert_eq!(
        read_ram_u16(&machine, slot_base(0) + 6),
        (u16::from(geometry.heads - 1) << 8) | 0x0001,
        "AH=08h: DX"
    );
    assert_eq!(read_ram_u16(&machine, slot_base(0) + 8), 0x4321, "ES kept");
    assert_eq!(read_ram_u16(&machine, slot_base(0) + 10), 0x8899, "DI kept");
    assert_eq!(read_ram_u16(&machine, slot_base(0) + 12), 0x5678, "BX kept");
    assert_slot_status(&machine, 0, 0x00, "AH=08h");

    // AH=15h: fixed disk, CX:DX = the sector count of the usable cylinders,
    // the count's low byte left in AL like the probed AMI handler.
    let sectors = u32::from(geometry.cylinders - 1)
        * u32::from(geometry.heads)
        * u32::from(geometry.sectors_per_track);
    assert_slot(
        &machine,
        1,
        0x0300 | u16::from(sectors as u8),
        false,
        "AH=15h",
    );
    assert_eq!(
        read_ram_u16(&machine, slot_base(1) + 4),
        (sectors >> 16) as u16,
        "AH=15h: CX"
    );
    assert_eq!(
        read_ram_u16(&machine, slot_base(1) + 6),
        sectors as u16,
        "AH=15h: DX"
    );
}

#[test]
fn drive_parameters_at40() {
    drive_parameters_case(HddSizeType::AtMb40);
}

#[test]
fn drive_parameters_at100() {
    drive_parameters_case(HddSizeType::AtMb100);
}

#[test]
fn drive_parameters_at250() {
    drive_parameters_case(HddSizeType::AtMb250);
}

#[test]
fn drive_parameters_at504() {
    drive_parameters_case(HddSizeType::AtMb504);
}

#[test]
fn read_single_sector() {
    let mut machine = create_machine_dx66();
    machine
        .bus
        .insert_hdd(0, make_pattern_hdd(2), None)
        .expect("insert pattern hard disk");
    let script = int13h_snapshot_script(&[Int13hCall {
        ax: 0x0201,
        cx: chs_cx(0, 2),
        dx: 0x0080,
        es: (DATA_BUFFER >> 4) as u16,
        bx: 0,
        di: 0,
    }]);
    boot_and_run(&mut machine, &script, &[], INT13H_BUDGET);

    // Successful transfers return AL=0 like the probed AMI handler.
    assert_slot(&machine, 0, 0x0000, false, "read CHS 0/0/2");
    assert_slot_status(&machine, 0, 0x00, "read CHS 0/0/2");
    assert_pattern_sector(&machine, DATA_BUFFER, 1, "sector 2 content");
}

#[test]
fn read_high_cylinder_uses_the_cl_high_bits() {
    let mut machine = create_machine_dx50();
    machine
        .bus
        .insert_hdd(0, make_pattern_hdd(320), None)
        .expect("insert pattern hard disk");
    let script = int13h_snapshot_script(&[Int13hCall {
        ax: 0x0201,
        cx: chs_cx(300, 7),
        dx: 0x0580,
        es: (DATA_BUFFER >> 4) as u16,
        bx: 0,
        di: 0,
    }]);
    boot_and_run(&mut machine, &script, &[], INT13H_BUDGET);

    let unit = (300 * 16 + 5) * 63 + 6;
    assert_slot(&machine, 0, 0x0000, false, "read CHS 300/5/7");
    assert_pattern_sector(&machine, DATA_BUFFER, unit, "cylinder 300 content");
}

#[test]
fn multi_sector_read_crosses_head_and_cylinder() {
    let mut machine = create_machine_dx50();
    machine
        .bus
        .insert_hdd(0, make_pattern_hdd(3), None)
        .expect("insert pattern hard disk");
    // CHS 0/15/62 for four sectors: the run crosses from the last head of
    // cylinder 0 into cylinder 1 head 0.
    let script = int13h_snapshot_script(&[Int13hCall {
        ax: 0x0204,
        cx: chs_cx(0, 62),
        dx: 0x0F80,
        es: (DATA_BUFFER >> 4) as u16,
        bx: 0,
        di: 0,
    }]);
    boot_and_run(&mut machine, &script, &[], INT13H_BUDGET);

    assert_slot(&machine, 0, 0x0000, false, "read CHS 0/15/62 x4");
    let first_unit = 15 * 63 + 61;
    for index in 0..4u32 {
        assert_pattern_sector(
            &machine,
            DATA_BUFFER + index * 512,
            first_unit + index,
            &format!("sector {index} of the run"),
        );
    }
}

#[test]
fn write_then_read_back() {
    let mut machine = create_machine_dx50();
    machine
        .bus
        .insert_hdd(0, make_pattern_hdd(2), None)
        .expect("insert pattern hard disk");
    boot_to_halt!(machine);

    let payload: Vec<u8> = (0..1024u32).map(|index| (index * 7 + 3) as u8).collect();
    write_bytes(&mut machine, DATA_BUFFER, &payload);
    // Write two sectors at CHS 0/0/3, then read them back to a second
    // buffer.
    let script = int13h_snapshot_script(&[
        Int13hCall {
            ax: 0x0302,
            cx: chs_cx(0, 3),
            dx: 0x0080,
            es: (DATA_BUFFER >> 4) as u16,
            bx: 0,
            di: 0,
        },
        Int13hCall {
            ax: 0x0202,
            cx: chs_cx(0, 3),
            dx: 0x0080,
            es: (READ_BACK_BUFFER >> 4) as u16,
            bx: 0,
            di: 0,
        },
    ]);
    inject_and_run(&mut machine, &script, &[], INT13H_BUDGET);

    assert_slot(&machine, 0, 0x0000, false, "write CHS 0/0/3 x2");
    assert_slot(&machine, 1, 0x0000, false, "read back CHS 0/0/3 x2");
    for (index, &byte) in payload.iter().enumerate() {
        assert_eq!(
            read_ram_u8(&machine, READ_BACK_BUFFER + index as u32),
            byte,
            "read-back byte {index}"
        );
    }
    let image = machine.bus.hdd_image_bytes(0).expect("mounted image");
    assert_eq!(&image[2 * 512..4 * 512], payload.as_slice(), "image bytes");
}

#[test]
fn verify_moves_no_memory() {
    let mut machine = create_machine_dx50();
    machine
        .bus
        .insert_hdd(0, make_pattern_hdd(2), None)
        .expect("insert pattern hard disk");
    boot_to_halt!(machine);

    let canary = vec![0xEEu8; 1024];
    write_bytes(&mut machine, DATA_BUFFER, &canary);
    let script = int13h_snapshot_script(&[
        Int13hCall {
            ax: 0x0402,
            cx: chs_cx(0, 5),
            dx: 0x0080,
            es: (DATA_BUFFER >> 4) as u16,
            bx: 0,
            di: 0,
        },
        // Verify past the end of the cylinders.
        Int13hCall {
            ax: 0x0401,
            cx: chs_cx(2, 1),
            dx: 0x0080,
            es: (DATA_BUFFER >> 4) as u16,
            bx: 0,
            di: 0,
        },
    ]);
    inject_and_run(&mut machine, &script, &[], INT13H_BUDGET);

    assert_slot(&machine, 0, 0x0000, false, "verify CHS 0/0/5 x2");
    assert_slot(&machine, 1, 0x0100, true, "verify CHS 2/0/1");
    for index in 0..1024u32 {
        assert_eq!(
            read_ram_u8(&machine, DATA_BUFFER + index),
            0xEE,
            "canary byte {index}"
        );
    }
}

#[test]
fn error_paths_and_status_byte() {
    let mut machine = create_machine_dx50();
    machine
        .bus
        .insert_hdd(0, make_pattern_hdd(2), None)
        .expect("insert pattern hard disk");
    let buffer_segment = (DATA_BUFFER >> 4) as u16;
    let read = |ax: u16, cx: u16, dx: u16| Int13hCall {
        ax,
        cx,
        dx,
        es: buffer_segment,
        bx: 0,
        di: 0,
    };
    let script = int13h_snapshot_script(&[
        // Cylinder past the end fails with a bad command against the FDPT.
        read(0x0201, chs_cx(2, 1), 0x0080),
        // AH=01h returns the stored status in AL and consumes it: the
        // second read returns zero.
        read(0x0100, 0, 0x0080),
        read(0x0100, 0, 0x0080),
        // Zero sector count is a successful no-op.
        read(0x0200, chs_cx(0, 1), 0x0080),
        // Sector number 0 is never found by the controller.
        read(0x0201, chs_cx(0, 0), 0x0080),
        // Head 16 wraps in the 4-bit device register and reads head 0.
        read(0x0201, chs_cx(0, 1), 0x1080),
        // Drive 81h is not installed.
        read(0x0201, chs_cx(0, 1), 0x0081),
        // EDD check.
        Int13hCall {
            ax: 0x4100,
            cx: 0,
            dx: 0x0080,
            es: 0,
            bx: 0x55AA,
            di: 0,
        },
        // Unknown function.
        read(0x5500, 0, 0x0080),
        // Reset and alternate reset clear the stored status.
        read(0x0000, 0, 0x0080),
        read(0x0D00, 0, 0x0080),
        // Seek passes the address to the controller unvalidated.
        read(0x0C00, chs_cx(1, 1), 0x0080),
        read(0x0C00, chs_cx(9, 1), 0x0080),
        // Format track: valid, then past the end.
        read(0x0500, chs_cx(1, 0), 0x0080),
        read(0x0500, chs_cx(9, 0), 0x0080),
        // Set parameters, drive ready, recalibrate, diagnostics.
        read(0x0900, 0, 0x0080),
        read(0x1000, 0, 0x0080),
        read(0x1100, 0, 0x0080),
        read(0x1400, 0, 0x0080),
        // Disk type of the absent second drive is not an error, but the
        // probed AMI handler still stores a bad command status in the BDA.
        read(0x1500, 0, 0x0081),
    ]);
    boot_and_run(&mut machine, &script, &[], INT13H_BUDGET);

    assert_slot(&machine, 0, 0x0100, true, "cylinder 2");
    assert_slot_status(&machine, 0, 0x01, "cylinder 2");
    assert_slot(&machine, 1, 0x0001, false, "first status read");
    assert_slot_status(&machine, 1, 0x00, "first status read clears 40:74");
    assert_slot(&machine, 2, 0x0000, false, "second status read");
    assert_slot(&machine, 3, 0x0000, false, "zero count");
    assert_slot_status(&machine, 3, 0x00, "zero count");
    assert_slot(&machine, 4, 0x0400, true, "sector 0");
    assert_slot(&machine, 5, 0x0000, false, "head 16 wraps to head 0");
    assert_slot(&machine, 6, 0x0101, true, "absent drive 81h keeps AL");
    assert_slot(&machine, 7, 0x0100, true, "EDD check");
    assert_slot(&machine, 8, 0x0100, true, "unknown function");
    assert_slot(&machine, 9, 0x0000, false, "reset");
    assert_slot_status(&machine, 9, 0x00, "reset");
    assert_slot(&machine, 10, 0x0000, false, "alternate reset");
    assert_slot(&machine, 11, 0x0000, false, "seek cylinder 1");
    assert_slot(
        &machine,
        12,
        0x0000,
        false,
        "seek cylinder 9 is unvalidated",
    );
    assert_slot(&machine, 13, 0x0000, false, "format cylinder 1");
    assert_slot(&machine, 14, 0x0100, true, "format cylinder 9");
    assert_slot(&machine, 15, 0x0000, false, "set parameters");
    assert_slot(
        &machine,
        16,
        0x0050,
        false,
        "drive ready returns the IDE status",
    );
    assert_slot(&machine, 17, 0x0000, false, "recalibrate");
    assert_slot(&machine, 18, 0x0001, false, "diagnostics returns the code");
    assert_slot(&machine, 19, 0x0000, false, "disk type of the absent drive");
    assert_slot_status(&machine, 19, 0x01, "disk type of the absent drive");
}

#[test]
fn second_drive_is_addressed_through_dl_81h() {
    let mut machine = create_machine_dx50();
    machine
        .bus
        .insert_hdd(0, make_pattern_hdd(2), None)
        .expect("insert first hard disk");
    machine
        .bus
        .insert_hdd(1, make_pattern_hdd(3), None)
        .expect("insert second hard disk");
    let script = int13h_snapshot_script(&[
        Int13hCall::poisoned(0x0800, 0, 0x0081),
        Int13hCall {
            ax: 0x0201,
            cx: chs_cx(1, 4),
            dx: 0x0281,
            es: (DATA_BUFFER >> 4) as u16,
            bx: 0,
            di: 0,
        },
    ]);
    boot_and_run(&mut machine, &script, &[], INT13H_BUDGET);

    assert_slot(&machine, 0, 0x003F, false, "AH=08h drive 81h");
    assert_eq!(
        read_ram_u16(&machine, slot_base(0) + 4),
        chs_cx(1, 63),
        "AH=08h drive 81h: CX"
    );
    assert_eq!(
        read_ram_u16(&machine, slot_base(0) + 6),
        0x0F02,
        "AH=08h drive 81h: DX (two drives)"
    );
    assert_eq!(read_ram_u8(&machine, slot_base(0) + 15), 2, "40:75");

    let unit = (16 + 2) * 63 + 3;
    assert_slot(&machine, 1, 0x0000, false, "read drive 81h CHS 1/2/4");
    assert_pattern_sector(&machine, DATA_BUFFER, unit, "drive 81h content");
}
