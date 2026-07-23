//! INT 13h floppy disk services. The per-call results and the diskette BDA
//! state asserted here are the ones the real AMI BIOS produces, taken from
//! side-by-side captures against the real ct486 ROM set.

use common::NoTrace;
use machine_at::AtMachine;

use super::{
    FLOPPY_360K_SIZE, FLOPPY_720K_SIZE, FLOPPY_1200K_SIZE, FLOPPY_1232K_SIZE, FLOPPY_1440K_SIZE,
    RESULT, boot_and_run, create_machine_dx50, create_machine_dx66, inject_and_run,
    make_pattern_floppy, read_ram_u8, read_ram_u16, write_bytes,
};

/// FLAGS bit 0: carry.
const FLAGS_CARRY: u16 = 0x0001;
/// Guest scratch buffer for sector transfers.
const DATA_BUFFER: u32 = 0x8000;
/// Cycle budget for one injected INT 13h script against the HLE BIOS.
const INT13H_BUDGET: u64 = 20_000_000;

/// BIOS data area: diskette recalibrate and interrupt status.
const BDA_FLOPPY_RECALIBRATE: u32 = 0x43E;
/// BIOS data area: diskette motor status.
const BDA_FLOPPY_MOTOR: u32 = 0x43F;
/// BIOS data area: diskette motor shutoff counter.
const BDA_FLOPPY_MOTOR_COUNT: u32 = 0x440;
/// BIOS data area: diskette status of the last operation.
const BDA_FLOPPY_STATUS: u32 = 0x441;
/// BIOS data area: drive 0 media state.
const BDA_FLOPPY_MEDIA_STATE_0: u32 = 0x490;
/// BIOS data area: drive 0 current track.
const BDA_FLOPPY_TRACK_0: u32 = 0x494;

/// The 11 diskette parameter table bytes the stub ROM publishes.
const DISKETTE_PARAMETER_TABLE: [u8; 11] = [
    0xAF, 0x02, 0x25, 0x02, 0x12, 0x1B, 0xFF, 0x6C, 0xF6, 0x0F, 0x08,
];

/// One INT 13h call of a script: the register file loaded before the INT
/// instruction.
#[derive(Clone, Copy)]
struct Int13hCall {
    ax: u16,
    cx: u16,
    dx: u16,
    es: u16,
    bx: u16,
}

/// Builds a guest program that runs every call in order, storing AX and the
/// returned FLAGS of call `n` at RESULT + n * 4, then halts. Interrupts stay
/// disabled so the motor countdown at 40:40 is deterministic.
fn int13h_script(calls: &[Int13hCall]) -> Vec<u8> {
    let mut code = Vec::new();
    for (index, call) in calls.iter().enumerate() {
        let result_offset = RESULT as u16 + index as u16 * 4;
        let flags_offset = result_offset + 2;
        code.extend_from_slice(&[0xB8, call.es as u8, (call.es >> 8) as u8]);
        code.extend_from_slice(&[0x8E, 0xC0]);
        code.extend_from_slice(&[0xBB, call.bx as u8, (call.bx >> 8) as u8]);
        code.extend_from_slice(&[0xB9, call.cx as u8, (call.cx >> 8) as u8]);
        code.extend_from_slice(&[0xBA, call.dx as u8, (call.dx >> 8) as u8]);
        code.extend_from_slice(&[0xB8, call.ax as u8, (call.ax >> 8) as u8]);
        code.extend_from_slice(&[0xCD, 0x13]);
        code.extend_from_slice(&[0xA3, result_offset as u8, (result_offset >> 8) as u8]);
        code.extend_from_slice(&[0x9C, 0x58]);
        code.extend_from_slice(&[0xA3, flags_offset as u8, (flags_offset >> 8) as u8]);
    }
    code.push(0xF4);
    code
}

/// Asserts the AX and carry stored in script slot `slot`.
fn assert_slot(machine: &AtMachine<NoTrace>, slot: u32, ax: u16, carry: bool, label: &str) {
    assert_eq!(read_ram_u16(machine, RESULT + slot * 4), ax, "{label}: AX");
    assert_eq!(
        read_ram_u16(machine, RESULT + slot * 4 + 2) & FLAGS_CARRY != 0,
        carry,
        "{label}: carry"
    );
}

/// Asserts the pattern stamp of image unit `unit` at `address`.
fn assert_pattern_unit(machine: &AtMachine<NoTrace>, address: u32, unit: u16, label: &str) {
    assert_eq!(read_ram_u16(machine, address), unit, "{label}: unit stamp");
    assert_eq!(read_ram_u8(machine, address + 2), 0xA5, "{label}: marker");
    assert_eq!(
        read_ram_u8(machine, address + 3),
        unit as u8 ^ 0x5A,
        "{label}: fill"
    );
}

/// Boots a pattern floppy of `size` and reads one sector, asserting the
/// transfer result and the diskette BDA state.
fn read_single_sector_case(size: usize, media_state: u8) {
    let mut machine = create_machine_dx50();
    machine
        .bus
        .insert_floppy(0, make_pattern_floppy(size), None)
        .expect("insert pattern floppy");
    let script = int13h_script(&[Int13hCall {
        ax: 0x0201,
        cx: 0x0002,
        dx: 0x0000,
        es: (DATA_BUFFER >> 4) as u16,
        bx: 0,
    }]);
    boot_and_run(&mut machine, &script, &[], INT13H_BUDGET);

    assert_slot(&machine, 0, 0x0001, false, "read CHS 0/0/2");
    assert_pattern_unit(&machine, DATA_BUFFER, 1, "sector 2 content");
    assert_eq!(read_ram_u8(&machine, BDA_FLOPPY_STATUS), 0x00, "40:41");
    assert_eq!(
        read_ram_u8(&machine, BDA_FLOPPY_MEDIA_STATE_0),
        media_state,
        "40:90"
    );
    assert_eq!(read_ram_u8(&machine, BDA_FLOPPY_MOTOR), 0x01, "40:3F");
    assert_eq!(read_ram_u8(&machine, BDA_FLOPPY_MOTOR_COUNT), 0x25, "40:40");
    assert_eq!(read_ram_u8(&machine, BDA_FLOPPY_TRACK_0), 0, "40:94");
    assert_eq!(read_ram_u8(&machine, BDA_FLOPPY_RECALIBRATE), 0x01, "40:3E");
}

#[test]
fn read_single_sector_360k() {
    read_single_sector_case(FLOPPY_360K_SIZE, 0x93);
}

#[test]
fn read_single_sector_720k() {
    read_single_sector_case(FLOPPY_720K_SIZE, 0x97);
}

#[test]
fn read_single_sector_1200k() {
    read_single_sector_case(FLOPPY_1200K_SIZE, 0x15);
}

#[test]
fn read_single_sector_1440k() {
    read_single_sector_case(FLOPPY_1440K_SIZE, 0x17);
}

#[test]
fn read_multi_sector_stops_at_track_end() {
    let mut machine = create_machine_dx50();
    machine
        .bus
        .insert_floppy(0, make_pattern_floppy(FLOPPY_1440K_SIZE), None)
        .expect("insert pattern floppy");
    // CHS 2/0/17, four sectors: 17 and 18 of head 0 transfer, then the
    // controller cannot find record 19 and the run stops without switching
    // heads (matched to the real AMI BIOS).
    let script = int13h_script(&[Int13hCall {
        ax: 0x0204,
        cx: 0x0211,
        dx: 0x0000,
        es: (DATA_BUFFER >> 4) as u16,
        bx: 0,
    }]);
    boot_and_run(&mut machine, &script, &[], INT13H_BUDGET);

    assert_slot(&machine, 0, 0x0202, true, "read stops at track end");
    assert_eq!(read_ram_u8(&machine, BDA_FLOPPY_STATUS), 0x02, "40:41");
    for (index, unit) in [88u16, 89].into_iter().enumerate() {
        assert_pattern_unit(
            &machine,
            DATA_BUFFER + index as u32 * 512,
            unit,
            "transferred sectors",
        );
    }
    for offset in 0..4u32 {
        assert_eq!(
            read_ram_u8(&machine, DATA_BUFFER + 1024 + offset),
            0,
            "sectors past the track end untouched"
        );
    }
    assert_eq!(read_ram_u8(&machine, BDA_FLOPPY_TRACK_0), 2, "40:94");
}

#[test]
fn read_past_cylinder_end_returns_address_mark() {
    let mut machine = create_machine_dx50();
    machine
        .bus
        .insert_floppy(0, make_pattern_floppy(FLOPPY_1440K_SIZE), None)
        .expect("insert pattern floppy");
    // CHS 2/1/18, two sectors: sector 18 transfers, record 19 is never
    // found. No continuation onto the next cylinder.
    let script = int13h_script(&[Int13hCall {
        ax: 0x0202,
        cx: 0x0212,
        dx: 0x0100,
        es: (DATA_BUFFER >> 4) as u16,
        bx: 0,
    }]);
    boot_and_run(&mut machine, &script, &[], INT13H_BUDGET);

    assert_slot(&machine, 0, 0x0201, true, "read past cylinder end");
    assert_eq!(read_ram_u8(&machine, BDA_FLOPPY_STATUS), 0x02, "40:41");
}

#[test]
fn read_invalid_chs_returns_address_mark() {
    let mut machine = create_machine_dx50();
    machine
        .bus
        .insert_floppy(0, make_pattern_floppy(FLOPPY_1440K_SIZE), None)
        .expect("insert pattern floppy");
    let buffer_segment = (DATA_BUFFER >> 4) as u16;
    let script = int13h_script(&[
        // Sector 19 of a 18-sector track.
        Int13hCall {
            ax: 0x0201,
            cx: 0x0013,
            dx: 0x0000,
            es: buffer_segment,
            bx: 0,
        },
        // Sector 0.
        Int13hCall {
            ax: 0x0201,
            cx: 0x0000,
            dx: 0x0000,
            es: buffer_segment,
            bx: 0,
        },
        // Head 2.
        Int13hCall {
            ax: 0x0201,
            cx: 0x0001,
            dx: 0x0200,
            es: buffer_segment,
            bx: 0,
        },
        // Cylinder 80 of an 80-cylinder disk.
        Int13hCall {
            ax: 0x0201,
            cx: 0x5001,
            dx: 0x0000,
            es: buffer_segment,
            bx: 0,
        },
    ]);
    boot_and_run(&mut machine, &script, &[], INT13H_BUDGET);

    assert_slot(&machine, 0, 0x0200, true, "sector 19");
    assert_slot(&machine, 1, 0x0200, true, "sector 0");
    assert_slot(&machine, 2, 0x0200, true, "head 2");
    assert_slot(&machine, 3, 0x0200, true, "cylinder 80");
}

#[test]
fn read_invalid_drive_returns_bad_command() {
    let mut machine = create_machine_dx50();
    machine
        .bus
        .insert_floppy(0, make_pattern_floppy(FLOPPY_1440K_SIZE), None)
        .expect("insert pattern floppy");
    let script = int13h_script(&[Int13hCall {
        ax: 0x0201,
        cx: 0x0002,
        dx: 0x0002,
        es: (DATA_BUFFER >> 4) as u16,
        bx: 0,
    }]);
    boot_and_run(&mut machine, &script, &[], INT13H_BUDGET);

    assert_slot(&machine, 0, 0x0100, true, "drive 2");
    assert_eq!(read_ram_u8(&machine, BDA_FLOPPY_STATUS), 0x01, "40:41");
}

#[test]
fn read_no_media_returns_timeout() {
    let mut machine = create_machine_dx50();
    machine
        .bus
        .insert_floppy(0, make_pattern_floppy(FLOPPY_1440K_SIZE), None)
        .expect("insert pattern floppy");
    let script = int13h_script(&[Int13hCall {
        ax: 0x0201,
        cx: 0x0002,
        dx: 0x0001,
        es: (DATA_BUFFER >> 4) as u16,
        bx: 0,
    }]);
    boot_and_run(&mut machine, &script, &[], INT13H_BUDGET);

    assert_slot(&machine, 0, 0x8000, true, "empty drive B");
    assert_eq!(read_ram_u8(&machine, BDA_FLOPPY_STATUS), 0x80, "40:41");
}

#[test]
fn read_after_media_change_reports_06_then_succeeds() {
    let mut machine = create_machine_dx50();
    machine
        .bus
        .insert_floppy(0, make_pattern_floppy(FLOPPY_1440K_SIZE), None)
        .expect("insert pattern floppy");
    boot_to_halt!(machine);
    machine
        .bus
        .insert_floppy(0, make_pattern_floppy(FLOPPY_1440K_SIZE), None)
        .expect("swap pattern floppy");

    let call = Int13hCall {
        ax: 0x0201,
        cx: 0x0002,
        dx: 0x0000,
        es: (DATA_BUFFER >> 4) as u16,
        bx: 0,
    };
    let script = int13h_script(&[call, call]);
    inject_and_run(&mut machine, &script, &[], INT13H_BUDGET);

    assert_slot(&machine, 0, 0x0600, true, "changed media");
    assert_slot(&machine, 1, 0x0001, false, "retry succeeds");
    assert_eq!(
        read_ram_u8(&machine, BDA_FLOPPY_MEDIA_STATE_0),
        0x17,
        "40:90 re-established"
    );
    assert_pattern_unit(&machine, DATA_BUFFER, 1, "retried sector content");
}

#[test]
fn dma_boundary_violation_returns_09() {
    let mut machine = create_machine_dx50();
    machine
        .bus
        .insert_floppy(0, make_pattern_floppy(FLOPPY_1440K_SIZE), None)
        .expect("insert pattern floppy");
    // ES:BX = 1000:FE01, so the 512-byte transfer would cross 0x20000.
    let script = int13h_script(&[Int13hCall {
        ax: 0x0201,
        cx: 0x0002,
        dx: 0x0000,
        es: 0x1000,
        bx: 0xFE01,
    }]);
    boot_and_run(&mut machine, &script, &[], INT13H_BUDGET);

    assert_slot(&machine, 0, 0x0900, true, "boundary violation");
    assert_eq!(read_ram_u8(&machine, BDA_FLOPPY_STATUS), 0x09, "40:41");
    for offset in 0..4u32 {
        assert_eq!(
            read_ram_u8(&machine, 0x1FE01 + offset),
            0,
            "buffer untouched"
        );
    }
}

#[test]
fn write_sector_persists_and_reads_back() {
    let mut machine = create_machine_dx50();
    machine
        .bus
        .insert_floppy(0, make_pattern_floppy(FLOPPY_1440K_SIZE), None)
        .expect("insert pattern floppy");
    boot_to_halt!(machine);

    let payload: Vec<u8> = (0..512u32).map(|index| (index * 7 + 3) as u8).collect();
    write_bytes(&mut machine, DATA_BUFFER, &payload);

    let buffer_segment = (DATA_BUFFER >> 4) as u16;
    let script = int13h_script(&[
        // Write CHS 0/0/5 from the payload.
        Int13hCall {
            ax: 0x0301,
            cx: 0x0005,
            dx: 0x0000,
            es: buffer_segment,
            bx: 0,
        },
        // Read it back into the second half of the buffer.
        Int13hCall {
            ax: 0x0201,
            cx: 0x0005,
            dx: 0x0000,
            es: buffer_segment,
            bx: 0x0400,
        },
    ]);
    inject_and_run(&mut machine, &script, &[], INT13H_BUDGET);

    assert_slot(&machine, 0, 0x0001, false, "write");
    assert_slot(&machine, 1, 0x0001, false, "read back");
    for (index, &byte) in payload.iter().enumerate() {
        assert_eq!(
            read_ram_u8(&machine, DATA_BUFFER + 0x400 + index as u32),
            byte,
            "read back content"
        );
    }
    let image_bytes = machine
        .bus
        .floppy_image_bytes(0)
        .expect("mounted image bytes");
    assert_eq!(
        &image_bytes[4 * 512..5 * 512],
        &payload[..],
        "image content"
    );
}

#[test]
fn write_protected_returns_03() {
    let mut machine = create_machine_dx50();
    let mut image = make_pattern_floppy(FLOPPY_1440K_SIZE);
    image.write_protected = true;
    machine
        .bus
        .insert_floppy(0, image, None)
        .expect("insert protected floppy");
    let script = int13h_script(&[Int13hCall {
        ax: 0x0301,
        cx: 0x0005,
        dx: 0x0000,
        es: (DATA_BUFFER >> 4) as u16,
        bx: 0,
    }]);
    boot_and_run(&mut machine, &script, &[], INT13H_BUDGET);

    assert_slot(&machine, 0, 0x0300, true, "write protected");
    assert_eq!(read_ram_u8(&machine, BDA_FLOPPY_STATUS), 0x03, "40:41");
}

#[test]
fn verify_valid_and_invalid() {
    let mut machine = create_machine_dx50();
    machine
        .bus
        .insert_floppy(0, make_pattern_floppy(FLOPPY_1440K_SIZE), None)
        .expect("insert pattern floppy");
    let buffer_segment = (DATA_BUFFER >> 4) as u16;
    let script = int13h_script(&[
        Int13hCall {
            ax: 0x0402,
            cx: 0x0003,
            dx: 0x0000,
            es: buffer_segment,
            bx: 0,
        },
        Int13hCall {
            ax: 0x0401,
            cx: 0x0013,
            dx: 0x0000,
            es: buffer_segment,
            bx: 0,
        },
    ]);
    boot_and_run(&mut machine, &script, &[], INT13H_BUDGET);

    assert_slot(&machine, 0, 0x0002, false, "verify two sectors");
    assert_slot(&machine, 1, 0x0200, true, "verify missing sector");
    for offset in 0..8u32 {
        assert_eq!(
            read_ram_u8(&machine, DATA_BUFFER + offset),
            0,
            "verify moves no data"
        );
    }
}

#[test]
fn format_track_rewrites_sector_ids() {
    let mut machine = create_machine_dx50();
    machine
        .bus
        .insert_floppy(0, make_pattern_floppy(FLOPPY_1440K_SIZE), None)
        .expect("insert pattern floppy");
    boot_to_halt!(machine);

    // Five C/H/R/N identifiers for cylinder 1 head 0, records 1-5.
    let mut identifiers = Vec::new();
    for record in 1..=5u8 {
        identifiers.extend_from_slice(&[1, 0, record, 2]);
    }
    write_bytes(&mut machine, DATA_BUFFER, &identifiers);

    let buffer_segment = (DATA_BUFFER >> 4) as u16;
    let script = int13h_script(&[
        // Format cylinder 1 head 0 down to five sectors.
        Int13hCall {
            ax: 0x0505,
            cx: 0x0100,
            dx: 0x0000,
            es: buffer_segment,
            bx: 0,
        },
        // A surviving sector reads fine.
        Int13hCall {
            ax: 0x0201,
            cx: 0x0103,
            dx: 0x0000,
            es: buffer_segment,
            bx: 0x0400,
        },
        // A formatted-away sector is gone.
        Int13hCall {
            ax: 0x0201,
            cx: 0x0107,
            dx: 0x0000,
            es: buffer_segment,
            bx: 0x0400,
        },
    ]);
    inject_and_run(&mut machine, &script, &[], INT13H_BUDGET);

    assert_slot(&machine, 0, 0x0005, false, "format");
    assert_slot(&machine, 1, 0x0001, false, "surviving sector");
    assert_slot(&machine, 2, 0x0200, true, "formatted-away sector");
    for offset in 0..8u32 {
        assert_eq!(
            read_ram_u8(&machine, DATA_BUFFER + 0x400 + offset),
            0xF6,
            "format fill byte"
        );
    }
}

#[test]
fn reset_clears_status_and_recalibrate_bits() {
    let mut machine = create_machine_dx50();
    machine
        .bus
        .insert_floppy(0, make_pattern_floppy(FLOPPY_1440K_SIZE), None)
        .expect("insert pattern floppy");
    boot_to_halt!(machine);
    write_bytes(&mut machine, BDA_FLOPPY_RECALIBRATE, &[0x8F]);
    write_bytes(&mut machine, BDA_FLOPPY_STATUS, &[0x04]);

    let script = int13h_script(&[Int13hCall {
        ax: 0x0000,
        cx: 0,
        dx: 0x0000,
        es: 0,
        bx: 0,
    }]);
    inject_and_run(&mut machine, &script, &[], INT13H_BUDGET);

    assert_slot(&machine, 0, 0x0000, false, "reset");
    assert_eq!(read_ram_u8(&machine, BDA_FLOPPY_STATUS), 0x00, "40:41");
    assert_eq!(read_ram_u8(&machine, BDA_FLOPPY_RECALIBRATE), 0x00, "40:3E");
}

#[test]
fn last_status_returns_without_clearing() {
    let mut machine = create_machine_dx50();
    machine
        .bus
        .insert_floppy(0, make_pattern_floppy(FLOPPY_1440K_SIZE), None)
        .expect("insert pattern floppy");
    let buffer_segment = (DATA_BUFFER >> 4) as u16;
    let script = int13h_script(&[
        // Provoke an address-mark error.
        Int13hCall {
            ax: 0x0201,
            cx: 0x0013,
            dx: 0x0000,
            es: buffer_segment,
            bx: 0,
        },
        // Both status reads return it; 40:41 is not consumed.
        Int13hCall {
            ax: 0x0100,
            cx: 0,
            dx: 0x0000,
            es: 0,
            bx: 0,
        },
        Int13hCall {
            ax: 0x0100,
            cx: 0,
            dx: 0x0000,
            es: 0,
            bx: 0,
        },
    ]);
    boot_and_run(&mut machine, &script, &[], INT13H_BUDGET);

    assert_slot(&machine, 0, 0x0200, true, "provoked error");
    assert_slot(&machine, 1, 0x0200, true, "status returned");
    assert_slot(&machine, 2, 0x0200, true, "status returned again");
    assert_eq!(read_ram_u8(&machine, BDA_FLOPPY_STATUS), 0x02, "40:41");
}

/// Boots a pattern floppy of `size` and checks the AH=08h register file for
/// its CMOS drive type.
fn drive_parameters_case(size: usize, drive_type: u8, max_cylinder: u8, sectors_per_track: u8) {
    let mut machine = create_machine_dx50();
    machine
        .bus
        .insert_floppy(0, make_pattern_floppy(size), None)
        .expect("insert pattern floppy");
    let code = int13h_call_full(Int13hCall {
        ax: 0x0800,
        cx: 0,
        dx: 0x0000,
        es: 0,
        bx: 0,
    });
    boot_and_run(&mut machine, &code, &[], INT13H_BUDGET);

    assert_eq!(read_ram_u16(&machine, RESULT), 0x0000, "AX");
    assert_eq!(read_ram_u16(&machine, RESULT + 2) & FLAGS_CARRY, 0, "carry");
    assert_eq!(
        read_ram_u16(&machine, RESULT + 4),
        (u16::from(max_cylinder) << 8) | u16::from(sectors_per_track),
        "CX"
    );
    assert_eq!(read_ram_u16(&machine, RESULT + 6), 0x0101, "DX");
    assert_eq!(read_ram_u16(&machine, RESULT + 8), 0xF000, "ES");
    assert_eq!(
        read_ram_u16(&machine, RESULT + 0x0C),
        u16::from(drive_type),
        "BX"
    );

    let table = 0xF_0000 + u32::from(read_ram_u16(&machine, RESULT + 0x0A));
    for (offset, &expected) in DISKETTE_PARAMETER_TABLE.iter().enumerate() {
        assert_eq!(
            read_ram_u8(&machine, table + offset as u32),
            expected,
            "DPT byte {offset}"
        );
    }
}

/// Builds a guest program that runs one call and stores the full register
/// file: AX, FLAGS, CX, DX, ES, DI and BX at RESULT + 0/2/4/6/8/0Ah/0Ch.
#[rustfmt::skip]
fn int13h_call_full(call: Int13hCall) -> Vec<u8> {
    let mut code = Vec::new();
    code.extend_from_slice(&[0xB8, call.es as u8, (call.es >> 8) as u8]);
    code.extend_from_slice(&[0x8E, 0xC0]);
    code.extend_from_slice(&[0xBB, call.bx as u8, (call.bx >> 8) as u8]);
    code.extend_from_slice(&[0xB9, call.cx as u8, (call.cx >> 8) as u8]);
    code.extend_from_slice(&[0xBA, call.dx as u8, (call.dx >> 8) as u8]);
    code.extend_from_slice(&[0xBF, 0x99, 0x88]);
    code.extend_from_slice(&[0xB8, call.ax as u8, (call.ax >> 8) as u8]);
    code.extend_from_slice(&[0xCD, 0x13]);
    code.extend_from_slice(&[
        0xA3, 0x00, 0x06,       // MOV [0x0600], AX
        0x9C, 0x58,             // PUSHF; POP AX
        0xA3, 0x02, 0x06,       // MOV [0x0602], AX
        0x89, 0x0E, 0x04, 0x06, // MOV [0x0604], CX
        0x89, 0x16, 0x06, 0x06, // MOV [0x0606], DX
        0x8C, 0xC0,             // MOV AX, ES
        0xA3, 0x08, 0x06,       // MOV [0x0608], AX
        0x89, 0x3E, 0x0A, 0x06, // MOV [0x060A], DI
        0x89, 0x1E, 0x0C, 0x06, // MOV [0x060C], BX
        0xF4,                   // HLT
    ]);
    code
}

#[test]
fn drive_parameters_360k() {
    drive_parameters_case(FLOPPY_360K_SIZE, 0x01, 39, 9);
}

#[test]
fn drive_parameters_720k() {
    drive_parameters_case(FLOPPY_720K_SIZE, 0x03, 79, 9);
}

#[test]
fn drive_parameters_1200k() {
    drive_parameters_case(FLOPPY_1200K_SIZE, 0x02, 79, 15);
}

#[test]
fn drive_parameters_1440k() {
    drive_parameters_case(FLOPPY_1440K_SIZE, 0x04, 79, 18);
}

#[test]
fn drive_parameters_invalid_drive() {
    let mut machine = create_machine_dx50();
    machine
        .bus
        .insert_floppy(0, make_pattern_floppy(FLOPPY_1440K_SIZE), None)
        .expect("insert pattern floppy");
    let code = int13h_call_full(Int13hCall {
        ax: 0x0800,
        cx: 0x1234,
        dx: 0x0005,
        es: 0x4321,
        bx: 0x5678,
    });
    boot_and_run(&mut machine, &code, &[], INT13H_BUDGET);

    // A floppy drive number of 2 to 7Fh is out of range. The real AMI BIOS
    // has no defined behavior here (it indexes its two-entry drive table
    // unchecked and runs away), so instead of replicating that hang the HLE
    // returns safely: not an error, everything zeroed except the drive count
    // in DL.
    assert_eq!(read_ram_u16(&machine, RESULT), 0x0000, "AX");
    assert_eq!(read_ram_u16(&machine, RESULT + 2) & FLAGS_CARRY, 0, "carry");
    assert_eq!(read_ram_u16(&machine, RESULT + 4), 0, "CX");
    assert_eq!(read_ram_u16(&machine, RESULT + 6), 0x0001, "DX");
    assert_eq!(read_ram_u16(&machine, RESULT + 8), 0, "ES");
    assert_eq!(read_ram_u16(&machine, RESULT + 0x0A), 0, "DI");
    assert_eq!(read_ram_u16(&machine, RESULT + 0x0C), 0, "BX");
}

#[test]
fn drive_type_reports_change_line() {
    let mut machine = create_machine_dx50();
    machine
        .bus
        .insert_floppy(0, make_pattern_floppy(FLOPPY_1440K_SIZE), None)
        .expect("insert pattern floppy");
    let script = int13h_script(&[
        Int13hCall {
            ax: 0x1500,
            cx: 0,
            dx: 0x0000,
            es: 0,
            bx: 0,
        },
        Int13hCall {
            ax: 0x1500,
            cx: 0,
            dx: 0x0001,
            es: 0,
            bx: 0,
        },
        Int13hCall {
            ax: 0x1500,
            cx: 0,
            dx: 0x0080,
            es: 0,
            bx: 0,
        },
    ]);
    boot_and_run(&mut machine, &script, &[], INT13H_BUDGET);

    assert_slot(&machine, 0, 0x0200, false, "drive A has a change line");
    assert_slot(&machine, 1, 0x0000, false, "no drive B");
    assert_slot(&machine, 2, 0x0100, true, "no hard disk service yet");
}

#[test]
fn media_change_status_reports_and_persists() {
    let mut machine = create_machine_dx50();
    machine
        .bus
        .insert_floppy(0, make_pattern_floppy(FLOPPY_1440K_SIZE), None)
        .expect("insert pattern floppy");
    boot_to_halt!(machine);
    machine
        .bus
        .insert_floppy(0, make_pattern_floppy(FLOPPY_1440K_SIZE), None)
        .expect("swap pattern floppy");

    let status_call = Int13hCall {
        ax: 0x1600,
        cx: 0,
        dx: 0x0000,
        es: 0,
        bx: 0,
    };
    let read_call = Int13hCall {
        ax: 0x0201,
        cx: 0x0002,
        dx: 0x0000,
        es: (DATA_BUFFER >> 4) as u16,
        bx: 0,
    };
    // AH=16h reports without consuming; the read consumes; then it is clean.
    let script = int13h_script(&[status_call, status_call, read_call, status_call]);
    inject_and_run(&mut machine, &script, &[], INT13H_BUDGET);

    assert_slot(&machine, 0, 0x0600, true, "change reported");
    assert_slot(&machine, 1, 0x0600, true, "change persists");
    assert_slot(&machine, 2, 0x0600, true, "read consumes the change");
    assert_slot(&machine, 3, 0x0000, false, "change cleared");
}

#[test]
fn set_dasd_type_updates_media_state() {
    let mut machine = create_machine_dx50();
    machine
        .bus
        .insert_floppy(0, make_pattern_floppy(FLOPPY_1440K_SIZE), None)
        .expect("insert pattern floppy");
    let script = int13h_script(&[
        Int13hCall {
            ax: 0x1703,
            cx: 0,
            dx: 0x0000,
            es: 0,
            bx: 0,
        },
        Int13hCall {
            ax: 0x1709,
            cx: 0,
            dx: 0x0000,
            es: 0,
            bx: 0,
        },
    ]);
    boot_and_run(&mut machine, &script, &[], INT13H_BUDGET);

    assert_slot(&machine, 0, 0x0003, false, "set 1.2M DASD type");
    assert_eq!(
        read_ram_u8(&machine, BDA_FLOPPY_MEDIA_STATE_0),
        0x15,
        "40:90"
    );
    assert_slot(&machine, 1, 0x0109, true, "invalid DASD type");
}

#[test]
fn set_media_type_for_format() {
    let mut machine = create_machine_dx50();
    machine
        .bus
        .insert_floppy(0, make_pattern_floppy(FLOPPY_1440K_SIZE), None)
        .expect("insert pattern floppy");
    let script = int13h_script(&[
        Int13hCall {
            ax: 0x1800,
            cx: 0x4F12,
            dx: 0x0000,
            es: 0,
            bx: 0,
        },
        Int13hCall {
            ax: 0x1800,
            cx: 0x4F63,
            dx: 0x0000,
            es: 0,
            bx: 0,
        },
    ]);
    boot_and_run(&mut machine, &script, &[], INT13H_BUDGET);

    assert_slot(&machine, 0, 0x0000, false, "80 tracks, 18 sectors");
    assert_eq!(
        read_ram_u8(&machine, BDA_FLOPPY_MEDIA_STATE_0),
        0x17,
        "40:90"
    );
    assert_slot(&machine, 1, 0x0C00, true, "80 tracks, 99 sectors");
}

#[test]
fn int40h_aliases_floppy_services() {
    let mut machine = create_machine_dx66();
    machine
        .bus
        .insert_floppy(0, make_pattern_floppy(FLOPPY_1440K_SIZE), None)
        .expect("insert pattern floppy");
    // The single-call script with INT 13h replaced by INT 40h.
    let mut code = int13h_script(&[Int13hCall {
        ax: 0x0201,
        cx: 0x0002,
        dx: 0x0000,
        es: (DATA_BUFFER >> 4) as u16,
        bx: 0,
    }]);
    let int_position = code
        .windows(2)
        .position(|window| window == [0xCD, 0x13])
        .expect("INT 13h in script");
    code[int_position + 1] = 0x40;
    boot_and_run(&mut machine, &code, &[], INT13H_BUDGET);

    assert_slot(&machine, 0, 0x0001, false, "read through INT 40h");
    assert_pattern_unit(&machine, DATA_BUFFER, 1, "sector content");
}

#[test]
fn mode_1232k_reads_fail_per_sector() {
    let mut machine = create_machine_dx50();
    machine
        .bus
        .insert_floppy(0, make_pattern_floppy(FLOPPY_1232K_SIZE), None)
        .expect("insert 3-mode floppy");
    // The 1024-byte boot sector cannot be read, so the bootstrap falls
    // through to the failure halt loop; the script then runs from there.
    let call = Int13hCall {
        ax: 0x0201,
        cx: 0x0002,
        dx: 0x0000,
        es: (DATA_BUFFER >> 4) as u16,
        bx: 0,
    };
    let script = int13h_script(&[call, call]);
    boot_and_run(&mut machine, &script, &[], INT13H_BUDGET);

    // The insert latch reports the change first; the retry establishes the
    // media by the drive type (1.44M class) and then cannot address the
    // 1024-byte sectors, exactly like the real controller.
    assert_slot(&machine, 0, 0x0600, true, "changed media");
    assert_slot(&machine, 1, 0x0200, true, "3-mode media is not addressable");
    assert_eq!(
        read_ram_u8(&machine, BDA_FLOPPY_MEDIA_STATE_0),
        0x17,
        "40:90 establishes by drive type"
    );
}
