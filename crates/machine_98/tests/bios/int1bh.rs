use common::Bus;
use device::{
    disk::{HddFormat, HddGeometry, HddImage},
    floppy::FloppyImage,
};

use super::{
    TEST_CODE, boot_and_run_f, boot_and_run_ra, boot_and_run_vm, boot_and_run_vx, build_2hd_d88,
    create_machine_f, create_machine_pc9821as, create_machine_ra, create_machine_vm,
    create_machine_vx, read_ivt_vector, read_ram_u16, write_bytes,
};

const RESULT: u32 = 0x0600;
const DATA_BUFFER: u32 = 0x3000;
const INT1BH_BUDGET: u64 = 10_000_000;

const DISK_EQUIP: usize = 0x055C;

const DA_FDD_1MB_DRIVE0: u8 = 0x90;
const DA_FDD_1MB_DRIVE1: u8 = 0x91;
const DA_FDD_2DD_DRIVE0: u8 = 0x30;
const DA_FDD_TYPE_B0_DRIVE0: u8 = 0xB0;
const DA_FDD_TYPE_10_DRIVE0: u8 = 0x10;
const DA_INVALID: u8 = 0x40;

const DA_SASI_CHS_DRIVE0: u8 = 0x80;
const DA_SASI_LBA_DRIVE0: u8 = 0x00;

fn make_standard_2hd_disk(write_protected: bool) -> FloppyImage {
    let s1 = {
        let mut d = vec![0xA5u8; 1024];
        d[0] = 0x01;
        d
    };
    let s2 = {
        let mut d = vec![0xA5u8; 1024];
        d[0] = 0x02;
        d
    };
    let s3 = {
        let mut d = vec![0xA5u8; 1024];
        d[0] = 0x03;
        d
    };
    let s4 = {
        let mut d = vec![0xA5u8; 1024];
        d[0] = 0x04;
        d
    };
    let s5 = {
        let mut d = vec![0xA5u8; 1024];
        d[0] = 0x05;
        d
    };
    let s6 = {
        let mut d = vec![0xA5u8; 1024];
        d[0] = 0x06;
        d
    };
    let s7 = {
        let mut d = vec![0xA5u8; 1024];
        d[0] = 0x07;
        d
    };
    let s8 = {
        let mut d = vec![0xA5u8; 1024];
        d[0] = 0x08;
        d
    };

    let sectors: Vec<(u8, &[u8])> = vec![
        (1, &s1),
        (2, &s2),
        (3, &s3),
        (4, &s4),
        (5, &s5),
        (6, &s6),
        (7, &s7),
        (8, &s8),
    ];
    let tracks = vec![(0u8, 0u8, sectors.as_slice())];
    let d88_bytes = build_2hd_d88(&tracks, write_protected);
    FloppyImage::from_d88_bytes(&d88_bytes).expect("floppy image parse failed")
}

fn make_sasi_test_drive() -> HddImage {
    let geometry = HddGeometry {
        cylinders: 153,
        heads: 4,
        sectors_per_track: 33,
        sector_size: 256,
    };
    let total = geometry.total_bytes() as usize;
    let mut data = vec![0u8; total];
    for lba in 0..geometry.total_sectors() {
        let offset = lba as usize * 256;
        data[offset] = (lba >> 8) as u8;
        data[offset + 1] = lba as u8;
    }
    data[0] = 0xFA; // CLI
    data[1] = 0xF4; // HLT
    HddImage::from_raw(geometry, HddFormat::Thd, data)
}

/// INT 1Bh call with AH and AL only. Stores result AX to [RESULT], then HLT.
#[rustfmt::skip]
fn make_int1bh_simple(ah: u8, al: u8) -> Vec<u8> {
    vec![
        0xB8, al, ah,           // MOV AX, ah:al
        0xCD, 0x1B,             // INT 0x1B
        0xA3, 0x00, 0x06,       // MOV [RESULT], AX
        0xF4,                   // HLT
    ]
}

#[rustfmt::skip]
fn make_int1bh_hle_trap(ah: u8, al: u8, bx: u16, cx: u16, dx: u16, bp: u16) -> Vec<u8> {
    vec![
        0xB8, al, ah,                     // MOV AX, ah:al
        0xBB, bx as u8, (bx >> 8) as u8, // MOV BX, imm16
        0xB9, cx as u8, (cx >> 8) as u8, // MOV CX, imm16
        0xBA, dx as u8, (dx >> 8) as u8, // MOV DX, imm16
        0xBD, bp as u8, (bp >> 8) as u8, // MOV BP, imm16
        0x50,                             // PUSH AX
        0x52,                             // PUSH DX
        0xBA, 0xF0, 0x07,                 // MOV DX, 0x07F0
        0xB0, 0x1B,                       // MOV AL, 0x1B
        0xEE,                             // OUT DX, AL
        0xF4,                             // HLT
    ]
}

/// INT 1Bh FDD read/write/verify call with full parameters.
/// Stores result AX to [RESULT], then HLT.
#[allow(clippy::too_many_arguments)]
#[rustfmt::skip]
fn make_int1bh_rw(
    ah: u8,
    al: u8,
    transfer_size: u16,
    cylinder: u8,
    head: u8,
    sector: u8,
    length_code: u8,
    buffer_segment: u16,
    buffer_offset: u16,
) -> Vec<u8> {
    vec![
        // ES = buffer_segment
        0xB8, (buffer_segment & 0xFF) as u8, (buffer_segment >> 8) as u8,  // MOV AX, seg
        0x8E, 0xC0,                                                         // MOV ES, AX
        // BP = buffer_offset
        0xBD, (buffer_offset & 0xFF) as u8, (buffer_offset >> 8) as u8,     // MOV BP, off
        // BX = transfer_size
        0xBB, (transfer_size & 0xFF) as u8, (transfer_size >> 8) as u8,     // MOV BX, size
        // CX: CH=length_code, CL=cylinder
        0xB9, cylinder, length_code,                                         // MOV CX, len:cyl
        // DX: DH=head, DL=sector
        0xBA, sector, head,                                                  // MOV DX, head:sec
        // AX: AH=function, AL=DA/UA
        0xB8, al, ah,                                                        // MOV AX, func:da
        // INT 1Bh
        0xCD, 0x1B,                                                          // INT 0x1B
        // Store result AX to [RESULT]
        0xA3, 0x00, 0x06,                                                    // MOV [RESULT], AX
        // HLT
        0xF4,
    ]
}

/// INT 1Bh SASI read/write call. CX and DX carry CHS or LBA info.
/// Stores result AX to [RESULT], then HLT.
#[rustfmt::skip]
fn make_int1bh_sasi_rw(
    ah: u8,
    al: u8,
    transfer_size: u16,
    cx: u16,
    dx: u16,
    buffer_segment: u16,
    buffer_offset: u16,
) -> Vec<u8> {
    vec![
        // ES = buffer_segment
        0xB8, (buffer_segment & 0xFF) as u8, (buffer_segment >> 8) as u8,  // MOV AX, seg
        0x8E, 0xC0,                                                         // MOV ES, AX
        // BP = buffer_offset
        0xBD, (buffer_offset & 0xFF) as u8, (buffer_offset >> 8) as u8,     // MOV BP, off
        // BX = transfer_size
        0xBB, (transfer_size & 0xFF) as u8, (transfer_size >> 8) as u8,     // MOV BX, size
        // CX
        0xB9, (cx & 0xFF) as u8, (cx >> 8) as u8,                           // MOV CX, cx
        // DX
        0xBA, (dx & 0xFF) as u8, (dx >> 8) as u8,                           // MOV DX, dx
        // AX: AH=function, AL=DA/UA
        0xB8, al, ah,                                                        // MOV AX, func:da
        // INT 1Bh
        0xCD, 0x1B,                                                          // INT 0x1B
        // Store result AX to [RESULT]
        0xA3, 0x00, 0x06,                                                    // MOV [RESULT], AX
        // HLT
        0xF4,
    ]
}

/// INT 1Bh SASI new sense (AH=0x84). Stores AX, BX, CX, DX to [RESULT..RESULT+8].
#[rustfmt::skip]
fn make_int1bh_sasi_sense_new(al: u8) -> Vec<u8> {
    vec![
        0xB8, al, 0x84,                // MOV AX, 0x84:al
        0xCD, 0x1B,                     // INT 0x1B
        0xA3, 0x00, 0x06,              // MOV [RESULT], AX
        0x89, 0x1E, 0x02, 0x06,        // MOV [RESULT+2], BX
        0x89, 0x0E, 0x04, 0x06,        // MOV [RESULT+4], CX
        0x89, 0x16, 0x06, 0x06,        // MOV [RESULT+6], DX
        0xF4,                           // HLT
    ]
}

fn boot_and_run_fdd_f(
    code: &[u8],
    disk: Option<(usize, FloppyImage)>,
    budget: u64,
) -> machine_98::Pc9801F {
    let mut machine = create_machine_f();
    boot_to_halt!(machine);
    machine.bus.eject_floppy(0);
    if let Some((drive, image)) = disk {
        machine.bus.insert_floppy(drive, image, None);
    }
    write_bytes(&mut machine.bus, TEST_CODE, code);
    machine.cpu.load_state(&{
        let mut s = cpu::I8086State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.initialize_cold_frontend();
        s.set_sp(0x4000);
        s
    });
    machine.run_for(budget);
    machine
}

fn boot_and_run_fdd_vm(
    code: &[u8],
    disk: Option<(usize, FloppyImage)>,
    budget: u64,
) -> machine_98::Pc9801Vm {
    let mut machine = create_machine_vm();
    boot_to_halt!(machine);
    machine.bus.eject_floppy(0);
    if let Some((drive, image)) = disk {
        machine.bus.insert_floppy(drive, image, None);
    }
    write_bytes(&mut machine.bus, TEST_CODE, code);
    machine.cpu.load_state(&{
        let mut s = cpu::V30State::default();
        s.ip = TEST_CODE as u16;
        s.initialize_cold_frontend();
        s.set_sp(0x4000);
        s
    });
    machine.run_for(budget);
    machine
}

fn boot_and_run_fdd_vx(
    code: &[u8],
    disk: Option<(usize, FloppyImage)>,
    budget: u64,
) -> machine_98::Pc9801Vx {
    let mut machine = create_machine_vx();
    boot_to_halt!(machine);
    machine.bus.eject_floppy(0);
    if let Some((drive, image)) = disk {
        machine.bus.insert_floppy(drive, image, None);
    }
    write_bytes(&mut machine.bus, TEST_CODE, code);
    machine.cpu.load_state(&{
        let mut s = cpu::I286State::default();
        s.ip = TEST_CODE as u16;
        s.set_sp(0x4000);
        s
    });
    machine.run_for(budget);
    machine
}

fn boot_and_run_fdd_ra(
    code: &[u8],
    disk: Option<(usize, FloppyImage)>,
    budget: u64,
) -> machine_98::Pc9801Ra {
    let mut machine = create_machine_ra();
    boot_to_halt!(machine);
    machine.bus.eject_floppy(0);
    if let Some((drive, image)) = disk {
        machine.bus.insert_floppy(drive, image, None);
    }
    write_bytes(&mut machine.bus, TEST_CODE, code);
    machine.cpu.load_state(&{
        let mut s = cpu::I386State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_esp(0x4000);
        s
    });
    machine.run_for(budget);
    machine
}

fn boot_and_run_sasi_f(
    code: &[u8],
    hdd: Option<(usize, HddImage)>,
    budget: u64,
) -> machine_98::Pc9801F {
    let mut machine = create_machine_f();
    if let Some((drive, image)) = hdd {
        machine.bus.insert_hdd(drive, image, None);
    }
    boot_to_halt!(machine);
    write_bytes(&mut machine.bus, TEST_CODE, code);
    machine.cpu.load_state(&{
        let mut s = cpu::I8086State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.initialize_cold_frontend();
        s.set_sp(0x4000);
        s
    });
    machine.run_for(budget);
    machine
}

fn boot_and_run_sasi_vm(
    code: &[u8],
    hdd: Option<(usize, HddImage)>,
    budget: u64,
) -> machine_98::Pc9801Vm {
    let mut machine = create_machine_vm();
    if let Some((drive, image)) = hdd {
        machine.bus.insert_hdd(drive, image, None);
    }
    boot_to_halt!(machine);
    write_bytes(&mut machine.bus, TEST_CODE, code);
    machine.cpu.load_state(&{
        let mut s = cpu::V30State::default();
        s.ip = TEST_CODE as u16;
        s.initialize_cold_frontend();
        s.set_sp(0x4000);
        s
    });
    machine.run_for(budget);
    machine
}

fn boot_and_run_sasi_vx(
    code: &[u8],
    hdd: Option<(usize, HddImage)>,
    budget: u64,
) -> machine_98::Pc9801Vx {
    let mut machine = create_machine_vx();
    if let Some((drive, image)) = hdd {
        machine.bus.insert_hdd(drive, image, None);
    }
    boot_to_halt!(machine);
    write_bytes(&mut machine.bus, TEST_CODE, code);
    machine.cpu.load_state(&{
        let mut s = cpu::I286State::default();
        s.ip = TEST_CODE as u16;
        s.set_sp(0x4000);
        s
    });
    machine.run_for(budget);
    machine
}

fn boot_and_run_sasi_ra(
    code: &[u8],
    hdd: Option<(usize, HddImage)>,
    budget: u64,
) -> machine_98::Pc9801Ra {
    let mut machine = create_machine_ra();
    if let Some((drive, image)) = hdd {
        machine.bus.insert_hdd(drive, image, None);
    }
    boot_to_halt!(machine);
    write_bytes(&mut machine.bus, TEST_CODE, code);
    machine.cpu.load_state(&{
        let mut s = cpu::I386State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_esp(0x4000);
        s
    });
    machine.run_for(budget);
    machine
}

fn assert_result_ah(ram: &[u8; 0xA0000], expected: u8, label: &str) {
    let ax = read_ram_u16(ram, RESULT as usize);
    let ah = (ax >> 8) as u8;
    assert_eq!(
        ah, expected,
        "{label}: AH should be {expected:#04X} (got {ah:#04X})"
    );
}

// ============================================================================
// §12 INT 1Bh - IVT Vector Setup
// ============================================================================

#[test]
fn int1bh_vector_f() {
    let mut machine = create_machine_f();
    boot_to_halt!(machine);
    let state = machine.inspection_state();
    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x1B);
    assert!(
        segment >= 0xFD80,
        "INT 1Bh segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

#[test]
fn int1bh_vector_vm() {
    let mut machine = create_machine_vm();
    boot_to_halt!(machine);
    let state = machine.inspection_state();
    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x1B);
    assert!(
        segment >= 0xFD80,
        "INT 1Bh segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

#[test]
fn int1bh_vector_vx() {
    let mut machine = create_machine_vx();
    boot_to_halt!(machine);
    let state = machine.inspection_state();
    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x1B);
    assert!(
        segment >= 0xFD80,
        "INT 1Bh segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

#[test]
fn int1bh_vector_ra() {
    let mut machine = create_machine_ra();
    boot_to_halt!(machine);
    let state = machine.inspection_state();
    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x1B);
    assert!(
        segment >= 0xFD80,
        "INT 1Bh segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

// ============================================================================
// §12.2 FDD No-Op (AH=0x00) - Valid DA
// ============================================================================

#[test]
fn int1bh_fdd_noop_valid_da_f() {
    let code = make_int1bh_simple(0x00, DA_FDD_1MB_DRIVE0);
    let (machine, _) = boot_and_run_f(&code, &[], INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "FDD no-op valid DA");
}

#[test]
fn int1bh_fdd_noop_valid_da_vm() {
    let code = make_int1bh_simple(0x00, DA_FDD_1MB_DRIVE0);
    let (machine, _) = boot_and_run_vm(&code, &[], INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "FDD no-op valid DA");
}

#[test]
fn int1bh_fdd_noop_valid_da_vx() {
    let code = make_int1bh_simple(0x00, DA_FDD_1MB_DRIVE0);
    let (machine, _) = boot_and_run_vx(&code, &[], INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "FDD no-op valid DA");
}

#[test]
fn int1bh_fdd_noop_valid_da_ra() {
    let code = make_int1bh_simple(0x00, DA_FDD_1MB_DRIVE0);
    let (machine, _) = boot_and_run_ra(&code, &[], INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "FDD no-op valid DA");
}

// ============================================================================
// §12.2 FDD No-Op (AH=0x00) - Invalid DA
// ============================================================================

#[test]
fn int1bh_fdd_noop_invalid_da_f() {
    let code = make_int1bh_simple(0x00, DA_INVALID);
    let (machine, _) = boot_and_run_f(&code, &[], INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x40, "FDD no-op invalid DA");
}

#[test]
fn int1bh_fdd_noop_invalid_da_vm() {
    let code = make_int1bh_simple(0x00, DA_INVALID);
    let (machine, _) = boot_and_run_vm(&code, &[], INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x40, "FDD no-op invalid DA");
}

#[test]
fn int1bh_fdd_noop_invalid_da_vx() {
    let code = make_int1bh_simple(0x00, DA_INVALID);
    let (machine, _) = boot_and_run_vx(&code, &[], INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x40, "FDD no-op invalid DA");
}

#[test]
fn int1bh_fdd_noop_invalid_da_ra() {
    let code = make_int1bh_simple(0x00, DA_INVALID);
    let (machine, _) = boot_and_run_ra(&code, &[], INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x40, "FDD no-op invalid DA");
}

// ============================================================================
// §12.2 FDD Initialize (AH=0x03)
// ============================================================================

#[test]
fn int1bh_fdd_initialize_f() {
    let code = make_int1bh_simple(0x03, DA_FDD_1MB_DRIVE0);
    let (machine, _) = boot_and_run_f(&code, &[], INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "FDD initialize");
}

#[test]
fn int1bh_fdd_initialize_vm() {
    let code = make_int1bh_simple(0x03, DA_FDD_1MB_DRIVE0);
    let (machine, _) = boot_and_run_vm(&code, &[], INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "FDD initialize");
}

#[test]
fn int1bh_fdd_initialize_vx() {
    let code = make_int1bh_simple(0x03, DA_FDD_1MB_DRIVE0);
    let (machine, _) = boot_and_run_vx(&code, &[], INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "FDD initialize");
}

#[test]
fn int1bh_fdd_initialize_ra() {
    let code = make_int1bh_simple(0x03, DA_FDD_1MB_DRIVE0);
    let (machine, _) = boot_and_run_ra(&code, &[], INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "FDD initialize");
}

// ============================================================================
// §12.2 FDD Set Density (AH=0x4E - MF + 0x0E)
// ============================================================================

#[test]
fn int1bh_fdd_set_density_f() {
    let code = make_int1bh_simple(0x4E, DA_FDD_1MB_DRIVE0);
    let (machine, _) = boot_and_run_f(&code, &[], INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "FDD set density");
}

#[test]
fn int1bh_fdd_set_density_vm() {
    let code = make_int1bh_simple(0x4E, DA_FDD_1MB_DRIVE0);
    let (machine, _) = boot_and_run_vm(&code, &[], INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "FDD set density");
}

#[test]
fn int1bh_fdd_set_density_vx() {
    let code = make_int1bh_simple(0x4E, DA_FDD_1MB_DRIVE0);
    let (machine, _) = boot_and_run_vx(&code, &[], INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "FDD set density");
}

#[test]
fn int1bh_fdd_set_density_ra() {
    let code = make_int1bh_simple(0x4E, DA_FDD_1MB_DRIVE0);
    let (machine, _) = boot_and_run_ra(&code, &[], INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "FDD set density");
}

// ============================================================================
// §12.2 FDD Recalibrate (AH=0x07) - With Disk
// ============================================================================

#[test]
fn int1bh_fdd_recalibrate_with_disk_f() {
    let code = make_int1bh_simple(0x07, DA_FDD_1MB_DRIVE0);
    let machine = boot_and_run_fdd_f(
        &code,
        Some((0, make_standard_2hd_disk(false))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "FDD recalibrate with disk");
}

#[test]
fn int1bh_fdd_recalibrate_with_disk_vm() {
    let code = make_int1bh_simple(0x07, DA_FDD_1MB_DRIVE0);
    let machine = boot_and_run_fdd_vm(
        &code,
        Some((0, make_standard_2hd_disk(false))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "FDD recalibrate with disk");
}

#[test]
fn int1bh_fdd_recalibrate_with_disk_vx() {
    let code = make_int1bh_simple(0x07, DA_FDD_1MB_DRIVE0);
    let machine = boot_and_run_fdd_vx(
        &code,
        Some((0, make_standard_2hd_disk(false))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "FDD recalibrate with disk");
}

#[test]
fn int1bh_fdd_recalibrate_with_disk_ra() {
    let code = make_int1bh_simple(0x07, DA_FDD_1MB_DRIVE0);
    let machine = boot_and_run_fdd_ra(
        &code,
        Some((0, make_standard_2hd_disk(false))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "FDD recalibrate with disk");
}

// ============================================================================
// §12.2 FDD Recalibrate (AH=0x07) - No Disk
// ============================================================================

#[test]
fn int1bh_fdd_recalibrate_no_disk_f() {
    let code = make_int1bh_simple(0x07, DA_FDD_1MB_DRIVE0);
    let machine = boot_and_run_fdd_f(&code, None, INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "FDD recalibrate no disk");
}

#[test]
fn int1bh_fdd_recalibrate_no_disk_vm() {
    let code = make_int1bh_simple(0x07, DA_FDD_1MB_DRIVE0);
    let machine = boot_and_run_fdd_vm(&code, None, INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "FDD recalibrate no disk");
}

#[test]
fn int1bh_fdd_recalibrate_no_disk_vx() {
    let code = make_int1bh_simple(0x07, DA_FDD_1MB_DRIVE0);
    let machine = boot_and_run_fdd_vx(&code, None, INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "FDD recalibrate no disk");
}

#[test]
fn int1bh_fdd_recalibrate_no_disk_ra() {
    let code = make_int1bh_simple(0x07, DA_FDD_1MB_DRIVE0);
    let machine = boot_and_run_fdd_ra(&code, None, INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "FDD recalibrate no disk");
}

// ============================================================================
// §12.2 FDD Sense (AH=0x04) - With Disk
// ============================================================================

#[test]
fn int1bh_fdd_sense_with_disk_f() {
    let code = make_int1bh_simple(0x04, DA_FDD_1MB_DRIVE0);
    let machine = boot_and_run_fdd_f(
        &code,
        Some((0, make_standard_2hd_disk(false))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x01, "FDD sense with disk");
}

#[test]
fn int1bh_fdd_sense_with_disk_vm() {
    let code = make_int1bh_simple(0x04, DA_FDD_1MB_DRIVE0);
    let machine = boot_and_run_fdd_vm(
        &code,
        Some((0, make_standard_2hd_disk(false))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x01, "FDD sense with disk");
}

#[test]
fn int1bh_fdd_sense_with_disk_vx() {
    let code = make_int1bh_simple(0x04, DA_FDD_1MB_DRIVE0);
    let machine = boot_and_run_fdd_vx(
        &code,
        Some((0, make_standard_2hd_disk(false))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x01, "FDD sense with disk");
}

#[test]
fn int1bh_fdd_sense_with_disk_ra() {
    let code = make_int1bh_simple(0x04, DA_FDD_1MB_DRIVE0);
    let machine = boot_and_run_fdd_ra(
        &code,
        Some((0, make_standard_2hd_disk(false))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x01, "FDD sense with disk");
}

// ============================================================================
// §12.2 FDD Sense (AH=0x04) - No Disk
// ============================================================================

#[test]
fn int1bh_fdd_sense_no_disk_f() {
    let code = make_int1bh_simple(0x04, DA_FDD_1MB_DRIVE0);
    let machine = boot_and_run_fdd_f(&code, None, INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x60, "FDD sense no disk");
}

#[test]
fn int1bh_fdd_sense_no_disk_vm() {
    let code = make_int1bh_simple(0x04, DA_FDD_1MB_DRIVE0);
    let machine = boot_and_run_fdd_vm(&code, None, INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x60, "FDD sense no disk");
}

#[test]
fn int1bh_fdd_sense_no_disk_vx() {
    let code = make_int1bh_simple(0x04, DA_FDD_1MB_DRIVE0);
    let machine = boot_and_run_fdd_vx(&code, None, INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x60, "FDD sense no disk");
}

#[test]
fn int1bh_fdd_sense_no_disk_ra() {
    let code = make_int1bh_simple(0x04, DA_FDD_1MB_DRIVE0);
    let machine = boot_and_run_fdd_ra(&code, None, INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x60, "FDD sense no disk");
}

// ============================================================================
// §12.2 FDD Sense (AH=0x04) - Write Protected
// ============================================================================

#[test]
fn int1bh_fdd_sense_write_protected_f() {
    let code = make_int1bh_simple(0x04, DA_FDD_1MB_DRIVE0);
    let machine = boot_and_run_fdd_f(
        &code,
        Some((0, make_standard_2hd_disk(true))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x11, "FDD sense write protected");
}

#[test]
fn int1bh_fdd_sense_write_protected_vm() {
    let code = make_int1bh_simple(0x04, DA_FDD_1MB_DRIVE0);
    let machine = boot_and_run_fdd_vm(
        &code,
        Some((0, make_standard_2hd_disk(true))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x11, "FDD sense write protected");
}

#[test]
fn int1bh_fdd_sense_write_protected_vx() {
    let code = make_int1bh_simple(0x04, DA_FDD_1MB_DRIVE0);
    let machine = boot_and_run_fdd_vx(
        &code,
        Some((0, make_standard_2hd_disk(true))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x11, "FDD sense write protected");
}

#[test]
fn int1bh_fdd_sense_write_protected_ra() {
    let code = make_int1bh_simple(0x04, DA_FDD_1MB_DRIVE0);
    let machine = boot_and_run_fdd_ra(
        &code,
        Some((0, make_standard_2hd_disk(true))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x11, "FDD sense write protected");
}

// ============================================================================
// §12.2 FDD Diagnostic Read (AH=0x02 aliases AH=0x06)
// ============================================================================

#[test]
fn int1bh_fdd_diagnostic_read_f() {
    let code = make_int1bh_rw(
        0x02,
        DA_FDD_1MB_DRIVE0,
        1024,
        0,
        0,
        1,
        3,
        0x0000,
        DATA_BUFFER as u16,
    );
    let machine = boot_and_run_fdd_f(
        &code,
        Some((0, make_standard_2hd_disk(false))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "FDD diagnostic read");
    assert_eq!(
        state.memory.ram[DATA_BUFFER as usize], 0x00,
        "Diagnostic read buffer stays zero (DMA transfer does not complete during INT)"
    );
}

#[test]
fn int1bh_fdd_diagnostic_read_vm() {
    let code = make_int1bh_rw(
        0x02,
        DA_FDD_1MB_DRIVE0,
        1024,
        0,
        0,
        1,
        3,
        0x0000,
        DATA_BUFFER as u16,
    );
    let machine = boot_and_run_fdd_vm(
        &code,
        Some((0, make_standard_2hd_disk(false))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "FDD diagnostic read");
    assert_eq!(
        state.memory.ram[DATA_BUFFER as usize], 0x00,
        "Diagnostic read buffer stays zero (DMA transfer does not complete during INT)"
    );
}

// ============================================================================
// §12.2 FDD Read Single Sector (AH=0x56 - MF+SEEK+Read)
// ============================================================================

fn assert_fdd_read_single_sector(ram: &[u8; 0xA0000]) {
    assert_result_ah(ram, 0x00, "FDD read single sector");
    let buf_start = DATA_BUFFER as usize;
    assert_eq!(
        ram[buf_start], 0x01,
        "First byte of sector 1 should be 0x01 (got {:#04X})",
        ram[buf_start]
    );
    assert_eq!(
        ram[buf_start + 1],
        0xA5,
        "Second byte of sector 1 should be 0xA5 (got {:#04X})",
        ram[buf_start + 1]
    );
}

#[test]
fn int1bh_fdd_read_single_sector_f() {
    // AH=0x56: MF(bit6) + SEEK(bit4) + Read(0x06)
    let code = make_int1bh_rw(
        0x56,
        DA_FDD_1MB_DRIVE0,
        1024,
        0,
        0,
        1,
        3,
        0x0000,
        DATA_BUFFER as u16,
    );
    let machine = boot_and_run_fdd_f(
        &code,
        Some((0, make_standard_2hd_disk(false))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_fdd_read_single_sector(&state.memory.ram);
}

#[test]
fn int1bh_fdd_read_single_sector_vm() {
    // AH=0x56: MF(bit6) + SEEK(bit4) + Read(0x06)
    let code = make_int1bh_rw(
        0x56,
        DA_FDD_1MB_DRIVE0,
        1024,
        0,
        0,
        1,
        3,
        0x0000,
        DATA_BUFFER as u16,
    );
    let machine = boot_and_run_fdd_vm(
        &code,
        Some((0, make_standard_2hd_disk(false))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_fdd_read_single_sector(&state.memory.ram);
}

#[test]
fn int1bh_fdd_read_single_sector_vx() {
    let code = make_int1bh_rw(
        0x56,
        DA_FDD_1MB_DRIVE0,
        1024,
        0,
        0,
        1,
        3,
        0x0000,
        DATA_BUFFER as u16,
    );
    let machine = boot_and_run_fdd_vx(
        &code,
        Some((0, make_standard_2hd_disk(false))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_fdd_read_single_sector(&state.memory.ram);
}

#[test]
fn int1bh_fdd_read_single_sector_ra() {
    let code = make_int1bh_rw(
        0x56,
        DA_FDD_1MB_DRIVE0,
        1024,
        0,
        0,
        1,
        3,
        0x0000,
        DATA_BUFFER as u16,
    );
    let machine = boot_and_run_fdd_ra(
        &code,
        Some((0, make_standard_2hd_disk(false))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_fdd_read_single_sector(&state.memory.ram);
}

// ============================================================================
// §12.2 FDD Read Multiple Sectors (AH=0x56, 2048 bytes)
// ============================================================================

fn assert_fdd_read_multiple_sectors(ram: &[u8; 0xA0000]) {
    assert_result_ah(ram, 0x00, "FDD read multiple sectors");
    let buf_start = DATA_BUFFER as usize;
    assert_eq!(
        ram[buf_start], 0x01,
        "First byte of sector 1 should be 0x01"
    );
    assert_eq!(
        ram[buf_start + 1024],
        0x02,
        "First byte of sector 2 should be 0x02"
    );
}

#[test]
fn int1bh_fdd_read_multiple_sectors_f() {
    let code = make_int1bh_rw(
        0x56,
        DA_FDD_1MB_DRIVE0,
        2048,
        0,
        0,
        1,
        3,
        0x0000,
        DATA_BUFFER as u16,
    );
    let machine = boot_and_run_fdd_f(
        &code,
        Some((0, make_standard_2hd_disk(false))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_fdd_read_multiple_sectors(&state.memory.ram);
}

#[test]
fn int1bh_fdd_read_multiple_sectors_vm() {
    let code = make_int1bh_rw(
        0x56,
        DA_FDD_1MB_DRIVE0,
        2048,
        0,
        0,
        1,
        3,
        0x0000,
        DATA_BUFFER as u16,
    );
    let machine = boot_and_run_fdd_vm(
        &code,
        Some((0, make_standard_2hd_disk(false))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_fdd_read_multiple_sectors(&state.memory.ram);
}

#[test]
fn int1bh_fdd_read_multiple_sectors_vx() {
    let code = make_int1bh_rw(
        0x56,
        DA_FDD_1MB_DRIVE0,
        2048,
        0,
        0,
        1,
        3,
        0x0000,
        DATA_BUFFER as u16,
    );
    let machine = boot_and_run_fdd_vx(
        &code,
        Some((0, make_standard_2hd_disk(false))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_fdd_read_multiple_sectors(&state.memory.ram);
}

#[test]
fn int1bh_fdd_read_multiple_sectors_ra() {
    let code = make_int1bh_rw(
        0x56,
        DA_FDD_1MB_DRIVE0,
        2048,
        0,
        0,
        1,
        3,
        0x0000,
        DATA_BUFFER as u16,
    );
    let machine = boot_and_run_fdd_ra(
        &code,
        Some((0, make_standard_2hd_disk(false))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_fdd_read_multiple_sectors(&state.memory.ram);
}

// ============================================================================
// §12.2 FDD Read - No Disk
// ============================================================================

#[test]
fn int1bh_fdd_read_no_disk_f() {
    let code = make_int1bh_rw(
        0x56,
        DA_FDD_1MB_DRIVE0,
        1024,
        0,
        0,
        1,
        3,
        0x0000,
        DATA_BUFFER as u16,
    );
    let machine = boot_and_run_fdd_f(&code, None, INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x60, "FDD read no disk");
}

#[test]
fn int1bh_fdd_read_no_disk_vm() {
    let code = make_int1bh_rw(
        0x56,
        DA_FDD_1MB_DRIVE0,
        1024,
        0,
        0,
        1,
        3,
        0x0000,
        DATA_BUFFER as u16,
    );
    let machine = boot_and_run_fdd_vm(&code, None, INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x60, "FDD read no disk");
}

#[test]
fn int1bh_fdd_read_no_disk_vx() {
    let code = make_int1bh_rw(
        0x56,
        DA_FDD_1MB_DRIVE0,
        1024,
        0,
        0,
        1,
        3,
        0x0000,
        DATA_BUFFER as u16,
    );
    let machine = boot_and_run_fdd_vx(&code, None, INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x60, "FDD read no disk");
}

#[test]
fn int1bh_fdd_read_no_disk_ra() {
    let code = make_int1bh_rw(
        0x56,
        DA_FDD_1MB_DRIVE0,
        1024,
        0,
        0,
        1,
        3,
        0x0000,
        DATA_BUFFER as u16,
    );
    let machine = boot_and_run_fdd_ra(&code, None, INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x60, "FDD read no disk");
}

// ============================================================================
// §12.2 FDD Read - Invalid DA
// ============================================================================

#[test]
fn int1bh_fdd_read_invalid_da_f() {
    let code = make_int1bh_rw(
        0x56,
        DA_INVALID,
        1024,
        0,
        0,
        1,
        3,
        0x0000,
        DATA_BUFFER as u16,
    );
    let (machine, _) = boot_and_run_f(&code, &[], INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x40, "FDD read invalid DA");
}

#[test]
fn int1bh_fdd_read_invalid_da_vm() {
    let code = make_int1bh_rw(
        0x56,
        DA_INVALID,
        1024,
        0,
        0,
        1,
        3,
        0x0000,
        DATA_BUFFER as u16,
    );
    let (machine, _) = boot_and_run_vm(&code, &[], INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x40, "FDD read invalid DA");
}

#[test]
fn int1bh_fdd_read_invalid_da_vx() {
    let code = make_int1bh_rw(
        0x56,
        DA_INVALID,
        1024,
        0,
        0,
        1,
        3,
        0x0000,
        DATA_BUFFER as u16,
    );
    let (machine, _) = boot_and_run_vx(&code, &[], INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x40, "FDD read invalid DA");
}

#[test]
fn int1bh_fdd_read_invalid_da_ra() {
    let code = make_int1bh_rw(
        0x56,
        DA_INVALID,
        1024,
        0,
        0,
        1,
        3,
        0x0000,
        DATA_BUFFER as u16,
    );
    let (machine, _) = boot_and_run_ra(&code, &[], INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x40, "FDD read invalid DA");
}

// ============================================================================
// §12.2 FDD Read - Sector Not Found (AH=0x76: MF+no-retry+SEEK+Read)
// ============================================================================

#[test]
fn int1bh_fdd_read_sector_not_found_f() {
    // Sector 10 does not exist on an 8-sector track. bit5=1 disables retry.
    let code = make_int1bh_rw(
        0x76,
        DA_FDD_1MB_DRIVE0,
        1024,
        0,
        0,
        10,
        3,
        0x0000,
        DATA_BUFFER as u16,
    );
    let machine = boot_and_run_fdd_f(
        &code,
        Some((0, make_standard_2hd_disk(false))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0xE0, "FDD read sector not found");
}

#[test]
fn int1bh_fdd_read_sector_not_found_vm() {
    // Sector 10 does not exist on an 8-sector track. bit5=1 disables retry.
    let code = make_int1bh_rw(
        0x76,
        DA_FDD_1MB_DRIVE0,
        1024,
        0,
        0,
        10,
        3,
        0x0000,
        DATA_BUFFER as u16,
    );
    let machine = boot_and_run_fdd_vm(
        &code,
        Some((0, make_standard_2hd_disk(false))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0xE0, "FDD read sector not found");
}

#[test]
fn int1bh_fdd_read_sector_not_found_vx() {
    let code = make_int1bh_rw(
        0x76,
        DA_FDD_1MB_DRIVE0,
        1024,
        0,
        0,
        10,
        3,
        0x0000,
        DATA_BUFFER as u16,
    );
    let machine = boot_and_run_fdd_vx(
        &code,
        Some((0, make_standard_2hd_disk(false))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0xE0, "FDD read sector not found");
}

#[test]
fn int1bh_fdd_read_sector_not_found_ra() {
    let code = make_int1bh_rw(
        0x76,
        DA_FDD_1MB_DRIVE0,
        1024,
        0,
        0,
        10,
        3,
        0x0000,
        DATA_BUFFER as u16,
    );
    let machine = boot_and_run_fdd_ra(
        &code,
        Some((0, make_standard_2hd_disk(false))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0xE0, "FDD read sector not found");
}

// ============================================================================
// §12.2 FDD Write Single Sector (AH=0x55 - MF+SEEK+Write)
// ============================================================================

/// Generates code that fills DATA_BUFFER with 0xBB, then calls INT 1Bh write.
#[rustfmt::skip]
fn make_fdd_write_code() -> Vec<u8> {
    let buf_lo = (DATA_BUFFER & 0xFF) as u8;
    let buf_hi = ((DATA_BUFFER >> 8) & 0xFF) as u8;
    vec![
        // Fill DATA_BUFFER with 0xBB (1024 bytes using REP STOSB)
        0x31, 0xC0,                     // XOR AX, AX
        0x8E, 0xC0,                     // MOV ES, AX
        0xBF, buf_lo, buf_hi,           // MOV DI, DATA_BUFFER
        0xB0, 0xBB,                     // MOV AL, 0xBB
        0xB9, 0x00, 0x04,              // MOV CX, 1024
        0xFC,                           // CLD
        0xF3, 0xAA,                     // REP STOSB
        // Now set up INT 1Bh write call
        // ES already 0x0000
        0xBD, buf_lo, buf_hi,           // MOV BP, DATA_BUFFER
        0xBB, 0x00, 0x04,              // MOV BX, 1024
        0xB9, 0x00, 0x03,              // MOV CX, cyl=0, len_code=3
        0xBA, 0x01, 0x00,              // MOV DX, head=0, sector=1
        0xB8, 0x90, 0x55,              // MOV AX, 0x55:0x90 (MF+SEEK+Write, drive 0)
        0xCD, 0x1B,                     // INT 0x1B
        0xA3, 0x00, 0x06,              // MOV [RESULT], AX
        0xF4,                           // HLT
    ]
}

#[test]
fn int1bh_fdd_write_single_sector_f() {
    let code = make_fdd_write_code();
    let machine = boot_and_run_fdd_f(
        &code,
        Some((0, make_standard_2hd_disk(false))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "FDD write single sector");

    let disk = machine.bus.floppy_disk(0).expect("disk should be inserted");
    let sector = disk
        .find_sector_near_track_index(0, 0, 0, 1, 3)
        .expect("sector 1 should exist");
    assert!(
        sector.data.iter().all(|&b| b == 0xBB),
        "sector 1 data should be all 0xBB after write"
    );
}

#[test]
fn int1bh_fdd_write_single_sector_vm() {
    let code = make_fdd_write_code();
    let machine = boot_and_run_fdd_vm(
        &code,
        Some((0, make_standard_2hd_disk(false))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "FDD write single sector");

    let disk = machine.bus.floppy_disk(0).expect("disk should be inserted");
    let sector = disk
        .find_sector_near_track_index(0, 0, 0, 1, 3)
        .expect("sector 1 should exist");
    assert!(
        sector.data.iter().all(|&b| b == 0xBB),
        "sector 1 data should be all 0xBB after write"
    );
}

#[test]
fn int1bh_fdd_write_single_sector_vx() {
    let code = make_fdd_write_code();
    let machine = boot_and_run_fdd_vx(
        &code,
        Some((0, make_standard_2hd_disk(false))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "FDD write single sector");

    let disk = machine.bus.floppy_disk(0).expect("disk should be inserted");
    let sector = disk
        .find_sector_near_track_index(0, 0, 0, 1, 3)
        .expect("sector 1 should exist");
    assert!(
        sector.data.iter().all(|&b| b == 0xBB),
        "sector 1 data should be all 0xBB after write"
    );
}

#[test]
fn int1bh_fdd_write_single_sector_ra() {
    let code = make_fdd_write_code();
    let machine = boot_and_run_fdd_ra(
        &code,
        Some((0, make_standard_2hd_disk(false))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "FDD write single sector");

    let disk = machine.bus.floppy_disk(0).expect("disk should be inserted");
    let sector = disk
        .find_sector_near_track_index(0, 0, 0, 1, 3)
        .expect("sector 1 should exist");
    assert!(
        sector.data.iter().all(|&b| b == 0xBB),
        "sector 1 data should be all 0xBB after write"
    );
}

// ============================================================================
// §12.2 FDD Write - Write Protected
// ============================================================================

#[test]
fn int1bh_fdd_write_protected_f() {
    let code = make_int1bh_rw(
        0x55,
        DA_FDD_1MB_DRIVE0,
        1024,
        0,
        0,
        1,
        3,
        0x0000,
        DATA_BUFFER as u16,
    );
    let machine = boot_and_run_fdd_f(
        &code,
        Some((0, make_standard_2hd_disk(true))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x70, "FDD write protected");
}

#[test]
fn int1bh_fdd_write_protected_vm() {
    let code = make_int1bh_rw(
        0x55,
        DA_FDD_1MB_DRIVE0,
        1024,
        0,
        0,
        1,
        3,
        0x0000,
        DATA_BUFFER as u16,
    );
    let machine = boot_and_run_fdd_vm(
        &code,
        Some((0, make_standard_2hd_disk(true))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x70, "FDD write protected");
}

#[test]
fn int1bh_fdd_write_protected_vx() {
    let code = make_int1bh_rw(
        0x55,
        DA_FDD_1MB_DRIVE0,
        1024,
        0,
        0,
        1,
        3,
        0x0000,
        DATA_BUFFER as u16,
    );
    let machine = boot_and_run_fdd_vx(
        &code,
        Some((0, make_standard_2hd_disk(true))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x70, "FDD write protected");
}

#[test]
fn int1bh_fdd_write_protected_ra() {
    let code = make_int1bh_rw(
        0x55,
        DA_FDD_1MB_DRIVE0,
        1024,
        0,
        0,
        1,
        3,
        0x0000,
        DATA_BUFFER as u16,
    );
    let machine = boot_and_run_fdd_ra(
        &code,
        Some((0, make_standard_2hd_disk(true))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x70, "FDD write protected");
}

// ============================================================================
// §12.2 FDD Verify (AH=0x51 - MF+SEEK+Verify)
// ============================================================================

#[test]
fn int1bh_fdd_verify_f() {
    let code = make_int1bh_rw(
        0x51,
        DA_FDD_1MB_DRIVE0,
        1024,
        0,
        0,
        1,
        3,
        0x0000,
        DATA_BUFFER as u16,
    );
    let machine = boot_and_run_fdd_f(
        &code,
        Some((0, make_standard_2hd_disk(false))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "FDD verify");
}

#[test]
fn int1bh_fdd_verify_vm() {
    let code = make_int1bh_rw(
        0x51,
        DA_FDD_1MB_DRIVE0,
        1024,
        0,
        0,
        1,
        3,
        0x0000,
        DATA_BUFFER as u16,
    );
    let machine = boot_and_run_fdd_vm(
        &code,
        Some((0, make_standard_2hd_disk(false))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "FDD verify");
}

#[test]
fn int1bh_fdd_verify_vx() {
    let code = make_int1bh_rw(
        0x51,
        DA_FDD_1MB_DRIVE0,
        1024,
        0,
        0,
        1,
        3,
        0x0000,
        DATA_BUFFER as u16,
    );
    let machine = boot_and_run_fdd_vx(
        &code,
        Some((0, make_standard_2hd_disk(false))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "FDD verify");
}

#[test]
fn int1bh_fdd_verify_ra() {
    let code = make_int1bh_rw(
        0x51,
        DA_FDD_1MB_DRIVE0,
        1024,
        0,
        0,
        1,
        3,
        0x0000,
        DATA_BUFFER as u16,
    );
    let machine = boot_and_run_fdd_ra(
        &code,
        Some((0, make_standard_2hd_disk(false))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "FDD verify");
}

// ============================================================================
// §12.2 FDD Read ID (AH=0x5A - MF+SEEK+ReadID)
// ============================================================================

/// Read ID: AH=0x5A, AL=DA, CL=cylinder, DH=head. Stores AX to RESULT.
#[rustfmt::skip]
fn make_int1bh_read_id(al: u8, cylinder: u8, head: u8) -> Vec<u8> {
    vec![
        // CX: CH=0, CL=cylinder
        0xB9, cylinder, 0x00,           // MOV CX, 0:cyl
        // DX: DH=head, DL=0
        0xBA, 0x00, head,               // MOV DX, head:0
        // AX: AH=0x5A (MF+SEEK+ReadID), AL=DA
        0xB8, al, 0x5A,                 // MOV AX, 0x5A:al
        // INT 1Bh
        0xCD, 0x1B,                     // INT 0x1B
        // Store result AX
        0xA3, 0x00, 0x06,              // MOV [RESULT], AX
        0xF4,                           // HLT
    ]
}

/// Four Read ID calls. Stores AX, CX, DX tuples to RESULT.
#[rustfmt::skip]
fn make_int1bh_read_id_sequence(al: u8, cylinder: u8, head: u8) -> Vec<u8> {
    let mut code = Vec::new();
    for call in 0..4u16 {
        let result = RESULT as u16 + call * 6;
        code.extend_from_slice(&[
            // CX: CH=0, CL=cylinder
            0xB9, cylinder, 0x00,                              // MOV CX, 0:cyl
            // DX: DH=head, DL=0
            0xBA, 0x00, head,                                  // MOV DX, head:0
            // AX: AH=0x7A (MF+NR+SEEK+ReadID), AL=DA
            0xB8, al, 0x7A,                                    // MOV AX, 0x7A:al
            0xCD, 0x1B,                                        // INT 1Bh
            0xA3, result as u8, (result >> 8) as u8,           // MOV [result], AX
            0x89, 0x0E, (result + 2) as u8, ((result + 2) >> 8) as u8, // MOV [result+2], CX
            0x89, 0x16, (result + 4) as u8, ((result + 4) >> 8) as u8, // MOV [result+4], DX
        ]);
    }
    code.push(0xF4); // HLT
    code
}

fn assert_fdd_read_id(ram: &[u8; 0xA0000]) {
    assert_result_ah(ram, 0x00, "FDD read ID");
}

#[test]
fn int1bh_fdd_read_id_f() {
    let code = make_int1bh_read_id(DA_FDD_1MB_DRIVE0, 0, 0);
    let machine = boot_and_run_fdd_f(
        &code,
        Some((0, make_standard_2hd_disk(false))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_fdd_read_id(&state.memory.ram);
}

#[test]
fn int1bh_fdd_read_id_vm() {
    let code = make_int1bh_read_id(DA_FDD_1MB_DRIVE0, 0, 0);
    let machine = boot_and_run_fdd_vm(
        &code,
        Some((0, make_standard_2hd_disk(false))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_fdd_read_id(&state.memory.ram);
}

#[test]
fn int1bh_fdd_read_id_vx() {
    let code = make_int1bh_read_id(DA_FDD_1MB_DRIVE0, 0, 0);
    let machine = boot_and_run_fdd_vx(
        &code,
        Some((0, make_standard_2hd_disk(false))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_fdd_read_id(&state.memory.ram);
}

#[test]
fn int1bh_fdd_read_id_ra() {
    let code = make_int1bh_read_id(DA_FDD_1MB_DRIVE0, 0, 0);
    let machine = boot_and_run_fdd_ra(
        &code,
        Some((0, make_standard_2hd_disk(false))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_fdd_read_id(&state.memory.ram);
}

#[test]
fn int1bh_fdd_read_id_sequence_advances_record_ra() {
    let code = make_int1bh_read_id_sequence(DA_FDD_1MB_DRIVE0, 0, 0);
    let machine = boot_and_run_fdd_ra(
        &code,
        Some((0, make_standard_2hd_disk(false))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();

    for call in 0..4usize {
        let result = RESULT as usize + call * 6;
        let ax = read_ram_u16(&state.memory.ram, result);
        let cx = read_ram_u16(&state.memory.ram, result + 2);
        let dx = read_ram_u16(&state.memory.ram, result + 4);
        assert_eq!((ax >> 8) as u8, 0x00, "read ID call {call} AH");
        assert_eq!(cx, 0x0300, "read ID call {call} CX");
        assert_eq!(dx, (call as u16) + 1, "read ID call {call} DX");
    }
}

// ============================================================================
// §12.2 FDD Write - Sector Not Found (AH=0x55: MF+SEEK+Write)
// ============================================================================

#[test]
fn int1bh_fdd_write_sector_not_found_vx() {
    let code = make_int1bh_rw(
        0x55,
        DA_FDD_1MB_DRIVE0,
        1024,
        0,
        0,
        10,
        3,
        0x0000,
        DATA_BUFFER as u16,
    );
    let machine = boot_and_run_fdd_vx(
        &code,
        Some((0, make_standard_2hd_disk(false))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0xE0, "FDD write sector not found");
}

// ============================================================================
// §12.2 FDD Format - N=0 Preservation
// ============================================================================

#[test]
#[rustfmt::skip]
fn int1bh_fdd_format_preserves_n_zero_vx() {
    let buf_lo = (DATA_BUFFER & 0xFF) as u8;
    let buf_hi = ((DATA_BUFFER >> 8) & 0xFF) as u8;
    let code = vec![
        // Set up CHRN table at DATA_BUFFER: [C=0, H=0, R=1, N=0]
        0x31, 0xC0,             // XOR AX, AX
        0x8E, 0xC0,             // MOV ES, AX
        0xC6, 0x06, buf_lo, buf_hi, 0x00,           // MOV BYTE [DATA_BUFFER+0], 0 (C)
        0xC6, 0x06, buf_lo.wrapping_add(1), buf_hi, 0x00,  // MOV BYTE [DATA_BUFFER+1], 0 (H)
        0xC6, 0x06, buf_lo.wrapping_add(2), buf_hi, 0x01,  // MOV BYTE [DATA_BUFFER+2], 1 (R)
        0xC6, 0x06, buf_lo.wrapping_add(3), buf_hi, 0x00,  // MOV BYTE [DATA_BUFFER+3], 0 (N)
        // ES:BP = DATA_BUFFER
        0xBD, buf_lo, buf_hi,   // MOV BP, DATA_BUFFER
        // BX = 4 (1 entry)
        0xBB, 0x04, 0x00,       // MOV BX, 4
        // CX: CH=3 (data size code), CL=0 (cylinder)
        0xB9, 0x00, 0x03,       // MOV CX, 0x0300
        // DX: DH=0 (head), DL=0xE5 (fill byte)
        0xBA, 0xE5, 0x00,       // MOV DX, 0x00E5
        // AX: AH=0x5D (MF+SEEK+Format), AL=DA_FDD_1MB_DRIVE0
        0xB8, DA_FDD_1MB_DRIVE0, 0x5D,
        0xCD, 0x1B,             // INT 0x1B
        0xA3, 0x00, 0x06,       // MOV [RESULT], AX
        0xF4,                   // HLT
    ];
    let machine = boot_and_run_fdd_vx(
        &code,
        Some((0, make_standard_2hd_disk(false))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "FDD format N=0");

    let disk = machine.bus.floppy_disk(0).expect("disk should be inserted");
    let sector = disk
        .find_sector_near_track_index(0, 0, 0, 1, 0)
        .expect("sector with N=0 should exist in header");
    assert_eq!(
        sector.data.len(),
        1024,
        "sector data should be 1024 bytes (128 << 3 from CH register)"
    );
    assert!(
        sector.data.iter().all(|&b| b == 0xE5),
        "sector data should be filled with 0xE5"
    );
}

#[test]
#[rustfmt::skip]
fn int1bh_fdd_format_uses_chrn_n_vx() {
    let buf_lo = (DATA_BUFFER & 0xFF) as u8;
    let buf_hi = ((DATA_BUFFER >> 8) & 0xFF) as u8;
    let code = vec![
        // Set up CHRN table at DATA_BUFFER: [C=0, H=0, R=1, N=3]
        0x31, 0xC0,             // XOR AX, AX
        0x8E, 0xC0,             // MOV ES, AX
        0xC6, 0x06, buf_lo, buf_hi, 0x00,           // MOV BYTE [DATA_BUFFER+0], 0 (C)
        0xC6, 0x06, buf_lo.wrapping_add(1), buf_hi, 0x00,  // MOV BYTE [DATA_BUFFER+1], 0 (H)
        0xC6, 0x06, buf_lo.wrapping_add(2), buf_hi, 0x01,  // MOV BYTE [DATA_BUFFER+2], 1 (R)
        0xC6, 0x06, buf_lo.wrapping_add(3), buf_hi, 0x03,  // MOV BYTE [DATA_BUFFER+3], 3 (N)
        // ES:BP = DATA_BUFFER
        0xBD, buf_lo, buf_hi,   // MOV BP, DATA_BUFFER
        // BX = 4 (1 entry)
        0xBB, 0x04, 0x00,       // MOV BX, 4
        // CX: CH=3 (data size code), CL=0 (cylinder)
        0xB9, 0x00, 0x03,       // MOV CX, 0x0300
        // DX: DH=0 (head), DL=0xAA (fill byte)
        0xBA, 0xAA, 0x00,       // MOV DX, 0x00AA
        // AX: AH=0x5D (MF+SEEK+Format), AL=DA_FDD_1MB_DRIVE0
        0xB8, DA_FDD_1MB_DRIVE0, 0x5D,
        0xCD, 0x1B,             // INT 0x1B
        0xA3, 0x00, 0x06,       // MOV [RESULT], AX
        0xF4,                   // HLT
    ];
    let machine = boot_and_run_fdd_vx(
        &code,
        Some((0, make_standard_2hd_disk(false))),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "FDD format CHRN N=3");

    let disk = machine.bus.floppy_disk(0).expect("disk should be inserted");
    let sector = disk
        .find_sector_near_track_index(0, 0, 0, 1, 3)
        .expect("sector with N=3 should exist");
    assert_eq!(sector.data.len(), 1024, "sector data should be 1024 bytes");
    assert!(
        sector.data.iter().all(|&b| b == 0xAA),
        "sector data should be filled with 0xAA"
    );
}

// ============================================================================
// §12.2 FDD Write - Multi-Track (MT)
// ============================================================================

fn make_two_head_disk() -> FloppyImage {
    let sectors_h0: Vec<(u8, &[u8])> = (1..=8).map(|r| (r, [0xAAu8; 1024].as_slice())).collect();
    let sectors_h1: Vec<(u8, &[u8])> = (1..=8).map(|r| (r, [0xBBu8; 1024].as_slice())).collect();
    let tracks = vec![
        (0u8, 0u8, sectors_h0.as_slice()),
        (0, 1, sectors_h1.as_slice()),
    ];
    let d88_bytes = build_2hd_d88(&tracks, false);
    FloppyImage::from_d88_bytes(&d88_bytes).expect("two-head disk parse failed")
}

#[test]
#[rustfmt::skip]
fn int1bh_fdd_write_multi_track_vx() {
    let buf_lo = (DATA_BUFFER & 0xFF) as u8;
    let buf_hi = ((DATA_BUFFER >> 8) & 0xFF) as u8;
    let code = vec![
        // Fill DATA_BUFFER with 0xCC (16 * 1024 = 16384 bytes using REP STOSB)
        0x31, 0xC0,                     // XOR AX, AX
        0x8E, 0xC0,                     // MOV ES, AX
        0xBF, buf_lo, buf_hi,           // MOV DI, DATA_BUFFER
        0xB0, 0xCC,                     // MOV AL, 0xCC
        0xB9, 0x00, 0x40,              // MOV CX, 16384
        0xFC,                           // CLD
        0xF3, 0xAA,                     // REP STOSB
        // ES:BP = DATA_BUFFER
        0xBD, buf_lo, buf_hi,           // MOV BP, DATA_BUFFER
        // BX = 16*1024
        0xBB, 0x00, 0x40,              // MOV BX, 16384
        // CX: CH=3 (N), CL=0 (cylinder)
        0xB9, 0x00, 0x03,              // MOV CX, 0x0300
        // DX: DH=0 (head), DL=1 (start sector)
        0xBA, 0x01, 0x00,              // MOV DX, 0x0001
        // AX: AH=0xD5 (MT+MF+SEEK+Write), AL=DA_FDD_1MB_DRIVE0
        0xB8, DA_FDD_1MB_DRIVE0, 0xD5,
        0xCD, 0x1B,                     // INT 0x1B
        0xA3, 0x00, 0x06,              // MOV [RESULT], AX
        0xF4,                           // HLT
    ];
    let machine = boot_and_run_fdd_vx(
        &code,
        Some((0, make_two_head_disk())),
        INT1BH_BUDGET,
    );
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "FDD write multi-track");

    let disk = machine.bus.floppy_disk(0).expect("disk should be inserted");
    let sector_h0 = disk
        .find_sector_near_track_index(0, 0, 0, 1, 3)
        .expect("sector 1 head 0 should exist");
    assert!(
        sector_h0.data.iter().all(|&b| b == 0xCC),
        "head 0 sector 1 should be 0xCC after MT write"
    );
    let sector_h1 = disk
        .find_sector_near_track_index(1, 0, 1, 1, 3)
        .expect("sector 1 head 1 should exist");
    assert!(
        sector_h1.data.iter().all(|&b| b == 0xCC),
        "head 1 sector 1 should be 0xCC after MT write"
    );
}

#[test]
fn int1bh_fdd_write_single_head_without_mt_vx() {
    let code = make_int1bh_rw(
        0x55,
        DA_FDD_1MB_DRIVE0,
        9 * 1024,
        0,
        0,
        1,
        3,
        0x0000,
        DATA_BUFFER as u16,
    );
    let machine = boot_and_run_fdd_vx(&code, Some((0, make_two_head_disk())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(
        &state.memory.ram,
        0x00,
        "FDD write past EOT without MT returns success",
    );
}

// ============================================================================
// §12.3 SASI Initialize (AH=0x03)
// ============================================================================
// All BIOSes detect the SASI expansion ROM at 0xD7000 and dispatch to it.

#[test]
fn int1bh_sasi_initialize_f() {
    let code = make_int1bh_simple(0x03, DA_SASI_CHS_DRIVE0);
    let machine = boot_and_run_sasi_f(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "SASI initialize");
    let disk_equip = read_ram_u16(&state.memory.ram, DISK_EQUIP);
    assert_ne!(
        disk_equip & 0x0100,
        0,
        "DISK_EQUIP bit 8 should be set for SASI drive 0 (got {disk_equip:#06X})"
    );
}

#[test]
fn int1bh_sasi_initialize_vm() {
    let code = make_int1bh_simple(0x03, DA_SASI_CHS_DRIVE0);
    let machine = boot_and_run_sasi_vm(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "SASI initialize");
    let disk_equip = read_ram_u16(&state.memory.ram, DISK_EQUIP);
    assert_ne!(
        disk_equip & 0x0100,
        0,
        "DISK_EQUIP bit 8 should be set for SASI drive 0 (got {disk_equip:#06X})"
    );
}

#[test]
fn int1bh_sasi_initialize_vx() {
    let code = make_int1bh_simple(0x03, DA_SASI_CHS_DRIVE0);
    let machine = boot_and_run_sasi_vx(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "SASI initialize");
    let disk_equip = read_ram_u16(&state.memory.ram, DISK_EQUIP);
    assert_ne!(
        disk_equip & 0x0100,
        0,
        "DISK_EQUIP bit 8 should be set for SASI drive 0 (got {disk_equip:#06X})"
    );
}

#[test]
fn int1bh_sasi_initialize_ra() {
    let code = make_int1bh_simple(0x03, DA_SASI_CHS_DRIVE0);
    let machine = boot_and_run_sasi_ra(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "SASI initialize");
    let disk_equip = read_ram_u16(&state.memory.ram, DISK_EQUIP);
    assert_ne!(
        disk_equip & 0x0100,
        0,
        "DISK_EQUIP bit 8 should be set for SASI drive 0 (got {disk_equip:#06X})"
    );
}

// ============================================================================
// §12.3 SASI Verify (AH=0x01) - No-op
// ============================================================================

#[test]
fn int1bh_sasi_verify_f() {
    let code = make_int1bh_simple(0x01, DA_SASI_CHS_DRIVE0);
    let machine = boot_and_run_sasi_f(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "SASI verify");
}

#[test]
fn int1bh_sasi_verify_vm() {
    let code = make_int1bh_simple(0x01, DA_SASI_CHS_DRIVE0);
    let machine = boot_and_run_sasi_vm(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "SASI verify");
}

#[test]
fn int1bh_sasi_verify_vx() {
    let code = make_int1bh_simple(0x01, DA_SASI_CHS_DRIVE0);
    let machine = boot_and_run_sasi_vx(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "SASI verify");
}

#[test]
fn int1bh_sasi_verify_ra() {
    let code = make_int1bh_simple(0x01, DA_SASI_CHS_DRIVE0);
    let machine = boot_and_run_sasi_ra(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "SASI verify");
}

// ============================================================================
// §12.3 SASI Sense Legacy (AH=0x04)
// ============================================================================

#[test]
fn int1bh_sasi_sense_f() {
    let code = make_int1bh_simple(0x04, DA_SASI_CHS_DRIVE0);
    let machine = boot_and_run_sasi_f(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    // 5 MB drive = type index 0, legacy sense code 0x00.
    assert_result_ah(&state.memory.ram, 0x00, "SASI sense legacy");
}

#[test]
fn int1bh_sasi_sense_vm() {
    let code = make_int1bh_simple(0x04, DA_SASI_CHS_DRIVE0);
    let machine = boot_and_run_sasi_vm(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    // 5 MB drive = type index 0, legacy sense code 0x00.
    assert_result_ah(&state.memory.ram, 0x00, "SASI sense legacy");
}

#[test]
fn int1bh_sasi_sense_vx() {
    let code = make_int1bh_simple(0x04, DA_SASI_CHS_DRIVE0);
    let machine = boot_and_run_sasi_vx(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "SASI sense legacy");
}

#[test]
fn int1bh_sasi_sense_ra() {
    let code = make_int1bh_simple(0x04, DA_SASI_CHS_DRIVE0);
    let machine = boot_and_run_sasi_ra(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "SASI sense legacy");
}

// ============================================================================
// §12.3 SASI Sense New (AH=0x84) - Returns Geometry
// ============================================================================

fn assert_sasi_sense_new(ram: &[u8; 0xA0000]) {
    assert_result_ah(ram, 0x00, "SASI sense new");
    let bx = read_ram_u16(ram, RESULT as usize + 2);
    let cx = read_ram_u16(ram, RESULT as usize + 4);
    let dx = read_ram_u16(ram, RESULT as usize + 6);
    assert_eq!(bx, 256, "BX should be sector size 256 (got {bx})");
    assert_eq!(cx, 152, "CX should be cylinders-1 = 152 (got {cx})");
    assert_eq!(dx >> 8, 4, "DH should be heads = 4 (got {})", dx >> 8);
    assert_eq!(
        dx & 0xFF,
        33,
        "DL should be sectors_per_track = 33 (got {})",
        dx & 0xFF
    );
}

#[test]
fn int1bh_sasi_sense_new_f() {
    let code = make_int1bh_sasi_sense_new(DA_SASI_CHS_DRIVE0);
    let machine = boot_and_run_sasi_f(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_sasi_sense_new(&state.memory.ram);
}

#[test]
fn int1bh_sasi_sense_new_vm() {
    let code = make_int1bh_sasi_sense_new(DA_SASI_CHS_DRIVE0);
    let machine = boot_and_run_sasi_vm(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_sasi_sense_new(&state.memory.ram);
}

#[test]
fn int1bh_sasi_sense_new_vx() {
    let code = make_int1bh_sasi_sense_new(DA_SASI_CHS_DRIVE0);
    let machine = boot_and_run_sasi_vx(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_sasi_sense_new(&state.memory.ram);
}

#[test]
fn int1bh_sasi_sense_new_ra() {
    let code = make_int1bh_sasi_sense_new(DA_SASI_CHS_DRIVE0);
    let machine = boot_and_run_sasi_ra(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_sasi_sense_new(&state.memory.ram);
}

// ============================================================================
// §12.3 SASI Sense - No Drive
// ============================================================================

#[test]
fn int1bh_sasi_sense_no_drive_f() {
    // Drive 1 (0x81) not present, only drive 0 inserted.
    let code = make_int1bh_simple(0x04, 0x81);
    let machine = boot_and_run_sasi_f(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x60, "SASI sense no drive");
}

#[test]
fn int1bh_sasi_sense_no_drive_vm() {
    // Drive 1 (0x81) not present, only drive 0 inserted.
    let code = make_int1bh_simple(0x04, 0x81);
    let machine = boot_and_run_sasi_vm(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x60, "SASI sense no drive");
}

#[test]
fn int1bh_sasi_sense_no_drive_vx() {
    let code = make_int1bh_simple(0x04, 0x81);
    let machine = boot_and_run_sasi_vx(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x60, "SASI sense no drive");
}

#[test]
fn int1bh_sasi_sense_no_drive_ra() {
    let code = make_int1bh_simple(0x04, 0x81);
    let machine = boot_and_run_sasi_ra(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x60, "SASI sense no drive");
}

// ============================================================================
// §12.3 SASI Read - CHS Mode (AH=0x06, AL=0x80)
// ============================================================================

fn assert_sasi_read_chs(ram: &[u8; 0xA0000]) {
    assert_result_ah(ram, 0x00, "SASI read CHS");
    let buf_start = DATA_BUFFER as usize;
    // LBA 0 bytes 0-1 are CLI+HLT (boot sector stub).
    assert_eq!(
        ram[buf_start], 0xFA,
        "LBA 0 byte 0 should be 0xFA (CLI, got {:#04X})",
        ram[buf_start]
    );
    assert_eq!(
        ram[buf_start + 1],
        0xF4,
        "LBA 0 byte 1 should be 0xF4 (HLT, got {:#04X})",
        ram[buf_start + 1]
    );
}

#[test]
fn int1bh_sasi_read_chs_f() {
    // CHS: C=0, H=0, S=0 -> LBA 0. Read 256 bytes.
    let code = make_int1bh_sasi_rw(
        0x06,
        DA_SASI_CHS_DRIVE0,
        256,
        0x0000, // CX: cylinder = 0
        0x0000, // DX: DH=head=0, DL=sector=0
        0x0000,
        DATA_BUFFER as u16,
    );
    let machine = boot_and_run_sasi_f(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_sasi_read_chs(&state.memory.ram);
}

#[test]
fn int1bh_sasi_read_chs_vm() {
    // CHS: C=0, H=0, S=0 -> LBA 0. Read 256 bytes.
    let code = make_int1bh_sasi_rw(
        0x06,
        DA_SASI_CHS_DRIVE0,
        256,
        0x0000, // CX: cylinder = 0
        0x0000, // DX: DH=head=0, DL=sector=0
        0x0000,
        DATA_BUFFER as u16,
    );
    let machine = boot_and_run_sasi_vm(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_sasi_read_chs(&state.memory.ram);
}

#[test]
fn int1bh_sasi_read_chs_vx() {
    let code = make_int1bh_sasi_rw(
        0x06,
        DA_SASI_CHS_DRIVE0,
        256,
        0x0000,
        0x0000,
        0x0000,
        DATA_BUFFER as u16,
    );
    let machine = boot_and_run_sasi_vx(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_sasi_read_chs(&state.memory.ram);
}

#[test]
fn int1bh_sasi_read_chs_ra() {
    let code = make_int1bh_sasi_rw(
        0x06,
        DA_SASI_CHS_DRIVE0,
        256,
        0x0000,
        0x0000,
        0x0000,
        DATA_BUFFER as u16,
    );
    let machine = boot_and_run_sasi_ra(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_sasi_read_chs(&state.memory.ram);
}

// ============================================================================
// §12.3 SASI Read - LBA Mode (AH=0x06, AL=0x00)
// ============================================================================

fn assert_sasi_read_lba(ram: &[u8; 0xA0000]) {
    assert_result_ah(ram, 0x00, "SASI read LBA");
    let buf_start = DATA_BUFFER as usize;
    // LBA 42 marker: first two bytes are (0x00, 0x2A).
    assert_eq!(
        ram[buf_start], 0x00,
        "LBA 42 marker high byte should be 0x00 (got {:#04X})",
        ram[buf_start]
    );
    assert_eq!(
        ram[buf_start + 1],
        0x2A,
        "LBA 42 marker low byte should be 0x2A (got {:#04X})",
        ram[buf_start + 1]
    );
}

#[test]
fn int1bh_sasi_read_lba_f() {
    // LBA mode: AL=0x00, CX=LBA low word (42), DL=LBA high byte (0).
    let code = make_int1bh_sasi_rw(
        0x06,
        DA_SASI_LBA_DRIVE0,
        256,
        42,     // CX: LBA low word
        0x0000, // DX: DL=LBA high byte=0, DH=0
        0x0000,
        DATA_BUFFER as u16,
    );
    let machine = boot_and_run_sasi_f(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_sasi_read_lba(&state.memory.ram);
}

#[test]
fn int1bh_sasi_read_lba_vm() {
    // LBA mode: AL=0x00, CX=LBA low word (42), DL=LBA high byte (0).
    let code = make_int1bh_sasi_rw(
        0x06,
        DA_SASI_LBA_DRIVE0,
        256,
        42,     // CX: LBA low word
        0x0000, // DX: DL=LBA high byte=0, DH=0
        0x0000,
        DATA_BUFFER as u16,
    );
    let machine = boot_and_run_sasi_vm(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_sasi_read_lba(&state.memory.ram);
}

#[test]
fn int1bh_sasi_read_lba_vx() {
    let code = make_int1bh_sasi_rw(
        0x06,
        DA_SASI_LBA_DRIVE0,
        256,
        42,
        0x0000,
        0x0000,
        DATA_BUFFER as u16,
    );
    let machine = boot_and_run_sasi_vx(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_sasi_read_lba(&state.memory.ram);
}

#[test]
fn int1bh_sasi_read_lba_ra() {
    let code = make_int1bh_sasi_rw(
        0x06,
        DA_SASI_LBA_DRIVE0,
        256,
        42,
        0x0000,
        0x0000,
        DATA_BUFFER as u16,
    );
    let machine = boot_and_run_sasi_ra(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_sasi_read_lba(&state.memory.ram);
}

// ============================================================================
// §12.3 SASI Read - No Drive
// ============================================================================

#[test]
fn int1bh_sasi_read_no_drive_f() {
    // Drive 1 (0x81) not present.
    let code = make_int1bh_sasi_rw(0x06, 0x81, 256, 0, 0, 0x0000, DATA_BUFFER as u16);
    let machine = boot_and_run_sasi_f(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x60, "SASI read no drive");
}

#[test]
fn int1bh_sasi_read_no_drive_vm() {
    // Drive 1 (0x81) not present.
    let code = make_int1bh_sasi_rw(0x06, 0x81, 256, 0, 0, 0x0000, DATA_BUFFER as u16);
    let machine = boot_and_run_sasi_vm(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x60, "SASI read no drive");
}

#[test]
fn int1bh_sasi_read_no_drive_vx() {
    let code = make_int1bh_sasi_rw(0x06, 0x81, 256, 0, 0, 0x0000, DATA_BUFFER as u16);
    let machine = boot_and_run_sasi_vx(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x60, "SASI read no drive");
}

#[test]
fn int1bh_sasi_read_no_drive_ra() {
    let code = make_int1bh_sasi_rw(0x06, 0x81, 256, 0, 0, 0x0000, DATA_BUFFER as u16);
    let machine = boot_and_run_sasi_ra(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x60, "SASI read no drive");
}

// ============================================================================
// §12.3 SASI Write - CHS Mode (AH=0x05, AL=0x80)
// ============================================================================

/// Generates code that fills DATA_BUFFER with 0xCC (256 bytes), then calls INT 1Bh SASI write.
/// Writes to C=0, H=0, S=1 (LBA 1) to avoid overwriting the LBA 0 marker used in read tests.
/// After the write, reads the same sector back to verify via INT 1Bh read.
#[rustfmt::skip]
fn make_sasi_write_and_readback_code() -> Vec<u8> {
    let buf_lo = (DATA_BUFFER & 0xFF) as u8;
    let buf_hi = ((DATA_BUFFER >> 8) & 0xFF) as u8;
    let result_lo = (RESULT & 0xFF) as u8;
    let result_hi = ((RESULT >> 8) & 0xFF) as u8;
    vec![
        // Fill DATA_BUFFER with 0xCC (256 bytes using REP STOSB)
        0x31, 0xC0,                     // XOR AX, AX
        0x8E, 0xC0,                     // MOV ES, AX
        0xBF, buf_lo, buf_hi,           // MOV DI, DATA_BUFFER
        0xB0, 0xCC,                     // MOV AL, 0xCC
        0xB9, 0x00, 0x01,              // MOV CX, 256
        0xFC,                           // CLD
        0xF3, 0xAA,                     // REP STOSB
        // INT 1Bh SASI write: AH=0x05, AL=0x80 (CHS drive 0)
        0x31, 0xC0,                     // XOR AX, AX
        0x8E, 0xC0,                     // MOV ES, AX
        0xBD, buf_lo, buf_hi,           // MOV BP, DATA_BUFFER
        0xBB, 0x00, 0x01,              // MOV BX, 256
        0xB9, 0x00, 0x00,              // MOV CX, cylinder=0
        0xBA, 0x01, 0x00,              // MOV DX, head=0, sector=1
        0xB8, 0x80, 0x05,              // MOV AX, 0x05:0x80
        0xCD, 0x1B,                     // INT 0x1B
        0xA3, result_lo, result_hi,     // MOV [RESULT], AX
        // Clear buffer before readback
        0x31, 0xC0,                     // XOR AX, AX
        0x8E, 0xC0,                     // MOV ES, AX
        0xBF, buf_lo, buf_hi,           // MOV DI, DATA_BUFFER
        0xB0, 0x00,                     // MOV AL, 0x00
        0xB9, 0x00, 0x01,              // MOV CX, 256
        0xFC,                           // CLD
        0xF3, 0xAA,                     // REP STOSB
        // INT 1Bh SASI read: AH=0x06, AL=0x80 (CHS drive 0), same C/H/S
        0x31, 0xC0,                     // XOR AX, AX
        0x8E, 0xC0,                     // MOV ES, AX
        0xBD, buf_lo, buf_hi,           // MOV BP, DATA_BUFFER
        0xBB, 0x00, 0x01,              // MOV BX, 256
        0xB9, 0x00, 0x00,              // MOV CX, cylinder=0
        0xBA, 0x01, 0x00,              // MOV DX, head=0, sector=1
        0xB8, 0x80, 0x06,              // MOV AX, 0x06:0x80
        0xCD, 0x1B,                     // INT 0x1B
        // Store readback result AH to RESULT+2
        0x89, 0x06, (result_lo + 2), result_hi,  // MOV [RESULT+2], AX
        0xF4,                           // HLT
    ]
}

fn assert_sasi_write_and_readback(ram: &[u8; 0xA0000]) {
    // Check write result
    let write_ax = read_ram_u16(ram, RESULT as usize);
    let write_ah = (write_ax >> 8) as u8;
    assert_eq!(
        write_ah, 0x00,
        "SASI write AH should be 0x00 (got {write_ah:#04X})"
    );

    // Check readback result
    let read_ax = read_ram_u16(ram, RESULT as usize + 2);
    let read_ah = (read_ax >> 8) as u8;
    assert_eq!(
        read_ah, 0x00,
        "SASI readback AH should be 0x00 (got {read_ah:#04X})"
    );

    // Verify data: buffer should contain 0xCC pattern from write
    let buf_start = DATA_BUFFER as usize;
    assert_eq!(
        ram[buf_start], 0xCC,
        "Readback data[0] should be 0xCC (got {:#04X})",
        ram[buf_start]
    );
    assert_eq!(
        ram[buf_start + 255],
        0xCC,
        "Readback data[255] should be 0xCC (got {:#04X})",
        ram[buf_start + 255]
    );
}

#[test]
fn int1bh_sasi_write_chs_f() {
    let code = make_sasi_write_and_readback_code();
    let machine = boot_and_run_sasi_f(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_sasi_write_and_readback(&state.memory.ram);
}

#[test]
fn int1bh_sasi_write_chs_vm() {
    let code = make_sasi_write_and_readback_code();
    let machine = boot_and_run_sasi_vm(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_sasi_write_and_readback(&state.memory.ram);
}

#[test]
fn int1bh_sasi_write_chs_vx() {
    let code = make_sasi_write_and_readback_code();
    let machine = boot_and_run_sasi_vx(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_sasi_write_and_readback(&state.memory.ram);
}

#[test]
fn int1bh_sasi_write_chs_ra() {
    let code = make_sasi_write_and_readback_code();
    let machine = boot_and_run_sasi_ra(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_sasi_write_and_readback(&state.memory.ram);
}

// ============================================================================
// §12.3 SASI Retract (AH=0x07) - No-op
// ============================================================================

#[test]
fn int1bh_sasi_retract_f() {
    let code = make_int1bh_simple(0x07, DA_SASI_CHS_DRIVE0);
    let machine = boot_and_run_sasi_f(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "SASI retract");
}

#[test]
fn int1bh_sasi_retract_vm() {
    let code = make_int1bh_simple(0x07, DA_SASI_CHS_DRIVE0);
    let machine = boot_and_run_sasi_vm(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "SASI retract");
}

#[test]
fn int1bh_sasi_retract_vx() {
    let code = make_int1bh_simple(0x07, DA_SASI_CHS_DRIVE0);
    let machine = boot_and_run_sasi_vx(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "SASI retract");
}

#[test]
fn int1bh_sasi_retract_ra() {
    let code = make_int1bh_simple(0x07, DA_SASI_CHS_DRIVE0);
    let machine = boot_and_run_sasi_ra(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "SASI retract");
}

// ============================================================================
// §12.3 SASI Mode Set (AH=0x0E)
// ============================================================================

#[test]
fn int1bh_sasi_mode_set_f() {
    let code = make_int1bh_simple(0x0E, DA_SASI_CHS_DRIVE0);
    let machine = boot_and_run_sasi_f(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "SASI mode set");
}

#[test]
fn int1bh_sasi_mode_set_vm() {
    let code = make_int1bh_simple(0x0E, DA_SASI_CHS_DRIVE0);
    let machine = boot_and_run_sasi_vm(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "SASI mode set");
}

#[test]
fn int1bh_sasi_mode_set_vx() {
    let code = make_int1bh_simple(0x0E, DA_SASI_CHS_DRIVE0);
    let machine = boot_and_run_sasi_vx(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "SASI mode set");
}

#[test]
fn int1bh_sasi_mode_set_ra() {
    let code = make_int1bh_simple(0x0E, DA_SASI_CHS_DRIVE0);
    let machine = boot_and_run_sasi_ra(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "SASI mode set");
}

// ============================================================================
// §12.3 SASI Format (AH=0x0D)
// ============================================================================

#[test]
fn int1bh_sasi_format_f() {
    // Format track at C=1, H=0, S=0.
    let code = make_int1bh_sasi_rw(
        0x0D,
        DA_SASI_CHS_DRIVE0,
        0,      // BX not used for format
        0x0001, // CX: cylinder=1
        0x0000, // DX: head=0, sector=0
        0x0000,
        0x0000,
    );
    let machine = boot_and_run_sasi_f(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "SASI format");
}

#[test]
fn int1bh_sasi_format_vm() {
    // Format track at C=1, H=0, S=0.
    let code = make_int1bh_sasi_rw(
        0x0D,
        DA_SASI_CHS_DRIVE0,
        0,      // BX not used for format
        0x0001, // CX: cylinder=1
        0x0000, // DX: head=0, sector=0
        0x0000,
        0x0000,
    );
    let machine = boot_and_run_sasi_vm(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "SASI format");
}

#[test]
fn int1bh_sasi_format_vx() {
    let code = make_int1bh_sasi_rw(0x0D, DA_SASI_CHS_DRIVE0, 0, 0x0001, 0x0000, 0x0000, 0x0000);
    let machine = boot_and_run_sasi_vx(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "SASI format");
}

#[test]
fn int1bh_sasi_format_ra() {
    let code = make_int1bh_sasi_rw(0x0D, DA_SASI_CHS_DRIVE0, 0, 0x0001, 0x0000, 0x0000, 0x0000);
    let machine = boot_and_run_sasi_ra(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "SASI format");
}

// ============================================================================
// §12.4 INT 1Bh - 2DD Device Type Dispatch
// ============================================================================

// 2DD (DA=0x70) is not supported on VM/VX/RS/RA - returns error 0x40.

const DA_FDD_640KB_DRIVE0: u8 = 0x70;

#[test]
fn int1bh_fdd_640kb_sense_f() {
    let disk = make_standard_2hd_disk(false);
    let code = make_int1bh_simple(0x04, DA_FDD_640KB_DRIVE0);
    let machine = boot_and_run_fdd_f(&code, Some((0, disk)), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x01, "DA=0x01 (640KB) sense");
}

#[test]
fn int1bh_fdd_640kb_sense_vm() {
    let disk = make_standard_2hd_disk(false);
    let code = make_int1bh_simple(0x04, DA_FDD_640KB_DRIVE0);
    let machine = boot_and_run_fdd_vm(&code, Some((0, disk)), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x01, "DA=0x70 (640KB) sense");
}

#[test]
fn int1bh_fdd_640kb_sense_vx() {
    let disk = make_standard_2hd_disk(false);
    let code = make_int1bh_simple(0x04, DA_FDD_640KB_DRIVE0);
    let machine = boot_and_run_fdd_vx(&code, Some((0, disk)), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x01, "DA=0x70 (640KB) sense");
}

#[test]
fn int1bh_fdd_640kb_sense_ra() {
    let disk = make_standard_2hd_disk(false);
    let code = make_int1bh_simple(0x04, DA_FDD_640KB_DRIVE0);
    let machine = boot_and_run_fdd_ra(&code, Some((0, disk)), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x01, "DA=0x70 (640KB) sense");
}

// ============================================================================
// §12.1 FDD Initialize (AH=0x03) - DISK_EQUIP Update
// ============================================================================

#[test]
fn int1bh_fdd_init_updates_disk_equip_1mb_f() {
    let code = make_int1bh_simple(0x03, DA_FDD_1MB_DRIVE0);
    let (machine, _) = boot_and_run_f(&code, &[], INT1BH_BUDGET);
    let state = machine.inspection_state();
    let disk_equip = read_ram_u16(&state.memory.ram, DISK_EQUIP);
    // 1MB FDD init writes drive_equipped bits into the low nibble of DISK_EQUIP.
    assert_ne!(
        disk_equip & 0x000F,
        0,
        "AH=03h with 1MB DA should update DISK_EQUIP low nibble (got {disk_equip:#06X})"
    );
}

#[test]
fn int1bh_fdd_init_updates_disk_equip_1mb_vm() {
    let code = make_int1bh_simple(0x03, DA_FDD_1MB_DRIVE0);
    let (machine, _) = boot_and_run_vm(&code, &[], INT1BH_BUDGET);
    let state = machine.inspection_state();
    let disk_equip = read_ram_u16(&state.memory.ram, DISK_EQUIP);
    // 1MB FDD init writes drive_equipped bits into the low nibble of DISK_EQUIP.
    assert_ne!(
        disk_equip & 0x000F,
        0,
        "AH=03h with 1MB DA should update DISK_EQUIP low nibble (got {disk_equip:#06X})"
    );
}

#[test]
fn int1bh_fdd_init_updates_disk_equip_1mb_vx() {
    let code = make_int1bh_simple(0x03, DA_FDD_1MB_DRIVE0);
    let (machine, _) = boot_and_run_vx(&code, &[], INT1BH_BUDGET);
    let state = machine.inspection_state();
    let disk_equip = read_ram_u16(&state.memory.ram, DISK_EQUIP);
    assert_ne!(
        disk_equip & 0x000F,
        0,
        "AH=03h with 1MB DA should update DISK_EQUIP low nibble (got {disk_equip:#06X})"
    );
}

#[test]
fn int1bh_fdd_init_updates_disk_equip_1mb_ra() {
    let code = make_int1bh_simple(0x03, DA_FDD_1MB_DRIVE0);
    let (machine, _) = boot_and_run_ra(&code, &[], INT1BH_BUDGET);
    let state = machine.inspection_state();
    let disk_equip = read_ram_u16(&state.memory.ram, DISK_EQUIP);
    assert_ne!(
        disk_equip & 0x000F,
        0,
        "AH=03h with 1MB DA should update DISK_EQUIP low nibble (got {disk_equip:#06X})"
    );
}

/// Build a disk with data on two different physical tracks but identical
/// sector headers (C=0). This mimics games like Ys that use the BIOS SEEK
/// command to position the head and then issue READ with a different CL.
///
/// Track index 0 (physical cyl 0, head 0): C=0 H=0 R=1 N=3, data=0xAA
/// Track index 2 (physical cyl 1, head 0): C=0 H=0 R=1 N=3, data=0xBB
///
/// Test disk with standard 2HD layout: cylinder 0 head 0 has data 0xAA,
/// cylinder 1 head 0 has data 0xBB. Sector headers match physical position.
fn make_seek_test_disk() -> FloppyImage {
    const HEADER_SIZE: usize = 0x2B0;
    const SEC_HDR: usize = 16;
    const SEC_SIZE: usize = 1024;

    let mut image = vec![0u8; HEADER_SIZE];
    image[..4].copy_from_slice(b"TEST");
    image[0x1B] = 0x20; // 2HD

    for &(track_index, cyl, fill_byte) in &[(0usize, 0u8, 0xAAu8), (2usize, 1u8, 0xBBu8)] {
        let track_offset = image.len() as u32;
        let ptr = 0x20 + track_index * 4;
        image[ptr..ptr + 4].copy_from_slice(&track_offset.to_le_bytes());

        let mut header = [0u8; SEC_HDR];
        header[0] = cyl;
        header[1] = 0; // H
        header[2] = 1; // R
        header[3] = 3; // N (1024 bytes)
        header[4..6].copy_from_slice(&1u16.to_le_bytes());
        header[0x0E..0x10].copy_from_slice(&(SEC_SIZE as u16).to_le_bytes());
        image.extend_from_slice(&header);
        image.extend_from_slice(&vec![fill_byte; SEC_SIZE]);
    }

    let disk_size = image.len() as u32;
    image[0x1C..0x20].copy_from_slice(&disk_size.to_le_bytes());
    FloppyImage::from_d88_bytes(&image).expect("seek test disk")
}

/// Generates code: SEEK to `seek_cyl` (MFM), then READ-with-seek CL=`read_cyl` (MFM).
/// The read result AX is stored at [RESULT], first byte of data at [DATA_BUFFER].
#[rustfmt::skip]
fn make_seek_then_read_code(seek_cyl: u8, read_cyl: u8, da: u8) -> Vec<u8> {
    vec![
        // SEEK: AH=0x50 (seek + MFM), AL=DA, CL=seek_cyl
        0xB1, seek_cyl,                                                     // MOV CL, seek_cyl
        0xB8, da, 0x50,                                                     // MOV AX, 0x50:DA
        0xCD, 0x1B,                                                         // INT 1Bh
        // READ with seek + MFM: AH=0x56, AL=DA
        0xB8, 0x00, 0x00,                                                   // MOV AX, 0
        0x8E, 0xC0,                                                         // MOV ES, AX
        0xBD, (DATA_BUFFER & 0xFF) as u8, (DATA_BUFFER >> 8) as u8,         // MOV BP, buf
        0xBB, 0x00, 0x04,                                                   // MOV BX, 1024
        0xB9, read_cyl, 0x03,                                               // MOV CX, 03:read_cyl
        0xBA, 0x01, 0x00,                                                   // MOV DX, 00:01
        0xB8, da, 0x56,                                                     // MOV AX, 0x56:DA
        0xCD, 0x1B,                                                         // INT 1Bh
        0xA3, (RESULT & 0xFF) as u8, (RESULT >> 8) as u8,                   // MOV [RESULT], AX
        0xF4,                                                               // HLT
    ]
}

fn assert_seek_then_read(ram: &[u8; 0xA0000]) {
    assert_result_ah(ram, 0x00, "SEEK then READ");
    assert_eq!(
        ram[DATA_BUFFER as usize], 0xBB,
        "Data should come from track_index=2 (0xBB), not track_index=0 (0xAA). \
         Got {:#04X}",
        ram[DATA_BUFFER as usize]
    );
}

#[test]
fn int1bh_fdd_seek_then_read_uses_stored_cylinder_f() {
    let code = make_seek_then_read_code(1, 1, DA_FDD_1MB_DRIVE0);
    let machine = boot_and_run_fdd_f(&code, Some((0, make_seek_test_disk())), INT1BH_BUDGET);
    assert_seek_then_read(&machine.inspection_state().memory.ram);
}

#[test]
fn int1bh_fdd_seek_then_read_uses_stored_cylinder_vm() {
    let code = make_seek_then_read_code(1, 1, DA_FDD_1MB_DRIVE0);
    let machine = boot_and_run_fdd_vm(&code, Some((0, make_seek_test_disk())), INT1BH_BUDGET);
    assert_seek_then_read(&machine.inspection_state().memory.ram);
}

#[test]
fn int1bh_fdd_seek_then_read_uses_stored_cylinder_vx() {
    let code = make_seek_then_read_code(1, 1, DA_FDD_1MB_DRIVE0);
    let machine = boot_and_run_fdd_vx(&code, Some((0, make_seek_test_disk())), INT1BH_BUDGET);
    assert_seek_then_read(&machine.inspection_state().memory.ram);
}

#[test]
fn int1bh_fdd_seek_then_read_uses_stored_cylinder_ra() {
    let code = make_seek_then_read_code(1, 1, DA_FDD_1MB_DRIVE0);
    let machine = boot_and_run_fdd_ra(&code, Some((0, make_seek_test_disk())), INT1BH_BUDGET);
    assert_seek_then_read(&machine.inspection_state().memory.ram);
}

/// Verify that READ with implicit seek (AH=0x56, bit 4 set) stores CL
/// and uses it for the track index.
#[rustfmt::skip]
fn make_read_with_seek_code(cylinder: u8, da: u8) -> Vec<u8> {
    vec![
        0xB8, 0x00, 0x00,                                                   // MOV AX, 0
        0x8E, 0xC0,                                                         // MOV ES, AX
        0xBD, (DATA_BUFFER & 0xFF) as u8, (DATA_BUFFER >> 8) as u8,         // MOV BP, buf
        0xBB, 0x00, 0x04,                                                   // MOV BX, 1024
        0xB9, cylinder, 0x03,                                               // MOV CX, 03:cyl
        0xBA, 0x01, 0x00,                                                   // MOV DX, 00:01
        0xB8, da, 0x56,                                                     // MOV AX, 0x56:DA
        0xCD, 0x1B,                                                         // INT 1Bh
        0xA3, (RESULT & 0xFF) as u8, (RESULT >> 8) as u8,                   // MOV [RESULT], AX
        0xF4,                                                               // HLT
    ]
}

#[test]
fn int1bh_fdd_read_with_seek_reads_correct_track_f() {
    let code = make_read_with_seek_code(1, DA_FDD_1MB_DRIVE0);
    let machine = boot_and_run_fdd_f(&code, Some((0, make_seek_test_disk())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "READ with seek");
    assert_eq!(
        state.memory.ram[DATA_BUFFER as usize], 0xBB,
        "READ with seek CL=1 should access track_index=2 (data=0xBB)"
    );
}

#[test]
fn int1bh_fdd_read_with_seek_reads_correct_track_vm() {
    let code = make_read_with_seek_code(1, DA_FDD_1MB_DRIVE0);
    let machine = boot_and_run_fdd_vm(&code, Some((0, make_seek_test_disk())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "READ with seek");
    assert_eq!(
        state.memory.ram[DATA_BUFFER as usize], 0xBB,
        "READ with seek CL=1 should access track_index=2 (data=0xBB)"
    );
}

#[test]
fn int1bh_fdd_read_with_seek_reads_correct_track_vx() {
    let code = make_read_with_seek_code(1, DA_FDD_1MB_DRIVE0);
    let machine = boot_and_run_fdd_vx(&code, Some((0, make_seek_test_disk())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "READ with seek");
    assert_eq!(
        state.memory.ram[DATA_BUFFER as usize], 0xBB,
        "READ with seek CL=1 should access track_index=2 (data=0xBB)"
    );
}

#[test]
fn int1bh_fdd_read_with_seek_reads_correct_track_ra() {
    let code = make_read_with_seek_code(1, DA_FDD_1MB_DRIVE0);
    let machine = boot_and_run_fdd_ra(&code, Some((0, make_seek_test_disk())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "READ with seek");
    assert_eq!(
        state.memory.ram[DATA_BUFFER as usize], 0xBB,
        "READ with seek CL=1 should access track_index=2 (data=0xBB)"
    );
}

/// Verify SEEK on drive 0 does not affect drive 1's seek cylinder.
#[rustfmt::skip]
fn make_seek_per_drive_code() -> Vec<u8> {
    vec![
        // SEEK drive 0 to cylinder 1 (MFM mode)
        0xB1, 0x01,                                                     // MOV CL, 1
        0xB8, DA_FDD_1MB_DRIVE0, 0x50,                                 // MOV AX, 0x50:90
        0xCD, 0x1B,                                                     // INT 1Bh
        // READ drive 1, CL=0 (MFM mode, no seek)
        0xB8, 0x00, 0x00,                                              // MOV AX, 0
        0x8E, 0xC0,                                                    // MOV ES, AX
        0xBD, (DATA_BUFFER & 0xFF) as u8, (DATA_BUFFER >> 8) as u8,   // MOV BP, buf
        0xBB, 0x00, 0x04,                                              // MOV BX, 1024
        0xB9, 0x00, 0x03,                                              // MOV CX, 03:00
        0xBA, 0x01, 0x00,                                              // MOV DX, 00:01
        0xB8, DA_FDD_1MB_DRIVE1, 0x46,                                 // MOV AX, 0x46:91
        0xCD, 0x1B,                                                    // INT 1Bh
        0xA3, (RESULT & 0xFF) as u8, (RESULT >> 8) as u8,             // MOV [RESULT], AX
        0xF4,                                                          // HLT
    ]
}

#[test]
fn int1bh_fdd_seek_per_drive_isolation_f() {
    let mut machine = create_machine_f();
    boot_to_halt!(machine);
    machine.bus.eject_floppy(0);
    machine.bus.insert_floppy(0, make_seek_test_disk(), None);
    machine.bus.insert_floppy(1, make_seek_test_disk(), None);

    let code = make_seek_per_drive_code();
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I8086State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.initialize_cold_frontend();
        s.set_sp(0x4000);
        s
    });
    machine.run_for(INT1BH_BUDGET);
    let state = machine.inspection_state();

    assert_result_ah(&state.memory.ram, 0x00, "Drive isolation read");
    assert_eq!(
        state.memory.ram[DATA_BUFFER as usize], 0xAA,
        "Drive 1 should read from track_index=0 (data=0xAA) since only drive 0 was seeked"
    );
}

#[test]
fn int1bh_fdd_seek_per_drive_isolation_vm() {
    let mut machine = create_machine_vm();
    boot_to_halt!(machine);
    machine.bus.eject_floppy(0);
    machine.bus.insert_floppy(0, make_seek_test_disk(), None);
    machine.bus.insert_floppy(1, make_seek_test_disk(), None);

    let code = make_seek_per_drive_code();
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::V30State::default();
        s.ip = TEST_CODE as u16;
        s.initialize_cold_frontend();
        s.set_sp(0x4000);
        s
    });
    machine.run_for(INT1BH_BUDGET);
    let state = machine.inspection_state();

    assert_result_ah(&state.memory.ram, 0x00, "Drive isolation read");
    assert_eq!(
        state.memory.ram[DATA_BUFFER as usize], 0xAA,
        "Drive 1 should read from track_index=0 (data=0xAA) since only drive 0 was seeked"
    );
}

/// Verify accepted FDD device type codes (DA=0x90, 0x10) dispatch to the FDD handler.
fn assert_fdd_da_accepted(da: u8, label: &str) {
    let code = make_int1bh_simple(0x00, da);
    let (machine, _) = boot_and_run_vm(&code, &[], INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, label);
}

/// Verify rejected FDD device type codes return error 0x40.
#[test]
fn int1bh_fdd_da_type_90_accepted() {
    assert_fdd_da_accepted(DA_FDD_1MB_DRIVE0, "DA=0x90 (1MB 2HD)");
}

#[test]
fn int1bh_fdd_da_type_10_accepted() {
    assert_fdd_da_accepted(DA_FDD_TYPE_10_DRIVE0, "DA=0x10");
}

#[test]
fn int1bh_fdd_da_type_30_accepted() {
    assert_fdd_da_accepted(DA_FDD_2DD_DRIVE0, "DA=0x30 (2DD)");
}

#[test]
fn int1bh_fdd_da_type_b0_accepted() {
    assert_fdd_da_accepted(DA_FDD_TYPE_B0_DRIVE0, "DA=0xB0");
}

/// Verify WRITE+READ with seek+MFM to a non-zero cylinder.
#[rustfmt::skip]
fn make_seek_then_write_readback_code( write_cyl: u8) -> Vec<u8> {
    let pattern: u8 = 0xCC;
    vec![
        // Fill write buffer at DATA_BUFFER with pattern
        0xBF, (DATA_BUFFER & 0xFF) as u8, (DATA_BUFFER >> 8) as u8,    // MOV DI, DATA_BUFFER
        0xB0, pattern,                                                 // MOV AL, pattern
        0xB9, 0x00, 0x04,                                              // MOV CX, 1024
        0xFC,                                                          // CLD
        0xF3, 0xAA,                                                    // REP STOSB

        // WRITE with seek + MFM: AH=0x55, AL=0x90
        0xB8, 0x00, 0x00,                                              // MOV AX, 0
        0x8E, 0xC0,                                                    // MOV ES, AX
        0xBD, (DATA_BUFFER & 0xFF) as u8, (DATA_BUFFER >> 8) as u8,    // MOV BP, buf
        0xBB, 0x00, 0x04,                                              // MOV BX, 1024
        0xB9, write_cyl, 0x03,                                         // MOV CX, 03:write_cyl
        0xBA, 0x01, 0x00,                                              // MOV DX, 00:01
        0xB8, DA_FDD_1MB_DRIVE0, 0x55,                                 // MOV AX, 0x55:90
        0xCD, 0x1B,                                                    // INT 1Bh
        0xA3, (RESULT & 0xFF) as u8, (RESULT >> 8) as u8,              // MOV [RESULT], AX

        // Clear buffer
        0xBF, (DATA_BUFFER & 0xFF) as u8, (DATA_BUFFER >> 8) as u8,    // MOV DI, DATA_BUFFER
        0xB0, 0x00,                                                    // MOV AL, 0
        0xB9, 0x00, 0x04,                                              // MOV CX, 1024
        0xF3, 0xAA,                                                    // REP STOSB

        // READ back with seek + MFM: AH=0x56, AL=0x90
        0xB8, 0x00, 0x00,                                              // MOV AX, 0
        0x8E, 0xC0,                                                    // MOV ES, AX
        0xBD, (DATA_BUFFER & 0xFF) as u8, (DATA_BUFFER >> 8) as u8,   // MOV BP, buf
        0xBB, 0x00, 0x04,                                              // MOV BX, 1024
        0xB9, write_cyl, 0x03,                                         // MOV CX, 03:write_cyl
        0xBA, 0x01, 0x00,                                              // MOV DX, 00:01
        0xB8, DA_FDD_1MB_DRIVE0, 0x56,                                 // MOV AX, 0x56:90
        0xCD, 0x1B,                                                     // INT 1Bh
        0x89, 0x06, 0x02, 0x06,                                        // MOV [RESULT+2], AX
        0xF4,                                                           // HLT
    ]
}

#[test]
fn int1bh_fdd_write_uses_seek_cylinder_f() {
    let code = make_seek_then_write_readback_code(1);
    let machine = boot_and_run_fdd_f(&code, Some((0, make_seek_test_disk())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "WRITE with seek cylinder");
    assert_eq!(
        state.memory.ram[DATA_BUFFER as usize], 0xCC,
        "Readback after WRITE should return written pattern 0xCC"
    );
}

#[test]
fn int1bh_fdd_write_uses_seek_cylinder_vm() {
    let code = make_seek_then_write_readback_code(1);
    let machine = boot_and_run_fdd_vm(&code, Some((0, make_seek_test_disk())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "WRITE with seek cylinder");
    assert_eq!(
        state.memory.ram[DATA_BUFFER as usize], 0xCC,
        "Readback after WRITE should return written pattern 0xCC"
    );
}

#[test]
fn int1bh_fdd_write_uses_seek_cylinder_vx() {
    let code = make_seek_then_write_readback_code(1);
    let machine = boot_and_run_fdd_vx(&code, Some((0, make_seek_test_disk())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "WRITE with seek cylinder");
    assert_eq!(
        state.memory.ram[DATA_BUFFER as usize], 0xCC,
        "Readback after WRITE should return written pattern 0xCC"
    );
}

#[test]
fn int1bh_fdd_write_uses_seek_cylinder_ra() {
    let code = make_seek_then_write_readback_code(1);
    let machine = boot_and_run_fdd_ra(&code, Some((0, make_seek_test_disk())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "WRITE with seek cylinder");
    assert_eq!(
        state.memory.ram[DATA_BUFFER as usize], 0xCC,
        "Readback after WRITE should return written pattern 0xCC"
    );
}

#[rustfmt::skip]
fn make_fdd_write_drive1_code() -> Vec<u8> {
    let buf_lo = (DATA_BUFFER & 0xFF) as u8;
    let buf_hi = ((DATA_BUFFER >> 8) & 0xFF) as u8;
    vec![
        // Fill DATA_BUFFER with 0xBB (1024 bytes using REP STOSB)
        0x31, 0xC0,                     // XOR AX, AX
        0x8E, 0xC0,                     // MOV ES, AX
        0xBF, buf_lo, buf_hi,           // MOV DI, DATA_BUFFER
        0xB0, 0xBB,                     // MOV AL, 0xBB
        0xB9, 0x00, 0x04,              // MOV CX, 1024
        0xFC,                           // CLD
        0xF3, 0xAA,                     // REP STOSB
        // Now set up INT 1Bh write call
        // ES already 0x0000
        0xBD, buf_lo, buf_hi,           // MOV BP, DATA_BUFFER
        0xBB, 0x00, 0x04,              // MOV BX, 1024
        0xB9, 0x00, 0x03,              // MOV CX, cyl=0, len_code=3
        0xBA, 0x01, 0x00,              // MOV DX, head=0, sector=1
        0xB8, DA_FDD_1MB_DRIVE1, 0x55, // MOV AX, 0x55:0x91 (MF+SEEK+Write, drive 1)
        0xCD, 0x1B,                     // INT 0x1B
        0xA3, 0x00, 0x06,              // MOV [RESULT], AX
        0xF4,                           // HLT
    ]
}

#[test]
fn int1bh_fdd_write_single_sector_drive1_vx() {
    let code = make_fdd_write_drive1_code();
    let mut machine = create_machine_vx();
    boot_to_halt!(machine);
    machine.bus.eject_floppy(0);
    machine
        .bus
        .insert_floppy(1, make_standard_2hd_disk(false), None);
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I286State::default();
        s.ip = TEST_CODE as u16;
        s.set_sp(0x4000);
        s
    });
    machine.run_for(INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "FDD write single sector drive 1");

    let disk = machine
        .bus
        .floppy_disk(1)
        .expect("disk should be inserted in drive 1");
    let sector = disk
        .find_sector_near_track_index(0, 0, 0, 1, 3)
        .expect("sector 1 should exist");
    assert!(
        sector.data.iter().all(|&b| b == 0xBB),
        "sector 1 data should be all 0xBB after write"
    );
}

const DA_IDE_CHS_DRIVE0: u8 = 0x80;

fn make_ide_test_drive() -> HddImage {
    let geometry = HddGeometry {
        cylinders: 20,
        heads: 4,
        sectors_per_track: 17,
        sector_size: 512,
    };
    let total = geometry.total_bytes() as usize;
    let mut data = vec![0u8; total];
    for lba in 0..geometry.total_sectors() {
        let offset = lba as usize * 512;
        data[offset] = (lba >> 8) as u8;
        data[offset + 1] = lba as u8;
    }
    data[0] = 0xFA; // CLI
    data[1] = 0xF4; // HLT
    HddImage::from_raw(geometry, HddFormat::Hdi, data)
}

fn boot_and_run_ide_pc9821(
    code: &[u8],
    hdd: Option<(usize, HddImage)>,
    budget: u64,
) -> machine_98::Pc9821As {
    let mut machine = create_machine_pc9821as();
    if let Some((drive, image)) = hdd {
        machine.bus.insert_hdd(drive, image, None);
    }
    boot_to_halt!(machine);
    write_bytes(&mut machine.bus, TEST_CODE, code);
    machine.cpu.load_state(&{
        let mut s = cpu::I386State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_esp(0x4000);
        s
    });
    machine.run_for(budget);
    machine
}

#[rustfmt::skip]
fn make_int1bh_ide_sense_new(al: u8) -> Vec<u8> {
    vec![
        0xB8, al, 0x84,                // MOV AX, 0x84:al
        0xCD, 0x1B,                     // INT 0x1B
        0xA3, 0x00, 0x06,              // MOV [RESULT], AX
        0x89, 0x1E, 0x02, 0x06,        // MOV [RESULT+2], BX
        0x89, 0x0E, 0x04, 0x06,        // MOV [RESULT+4], CX
        0x89, 0x16, 0x06, 0x06,        // MOV [RESULT+6], DX
        0xF4,                           // HLT
    ]
}

#[test]
fn int1bh_ide_initialize_pc9821() {
    let code = make_int1bh_simple(0x03, DA_IDE_CHS_DRIVE0);
    let machine = boot_and_run_ide_pc9821(&code, Some((0, make_ide_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "IDE init");
    let disk_equip = read_ram_u16(&state.memory.ram, DISK_EQUIP);
    assert_eq!(
        disk_equip & 0x0100,
        0x0100,
        "IDE drive 0 should be present in equipment word (got {disk_equip:#06X})"
    );
}

#[test]
fn int1bh_ide_verify_pc9821() {
    let code = make_int1bh_simple(0x01, DA_IDE_CHS_DRIVE0);
    let machine = boot_and_run_ide_pc9821(&code, Some((0, make_ide_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "IDE verify");
}

#[test]
fn int1bh_ide_sense_pc9821() {
    let code = make_int1bh_simple(0x04, DA_IDE_CHS_DRIVE0);
    let machine = boot_and_run_ide_pc9821(&code, Some((0, make_ide_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x0F, "IDE sense");
}

#[test]
fn int1bh_ide_sense_new_pc9821() {
    let code = make_int1bh_ide_sense_new(DA_IDE_CHS_DRIVE0);
    let machine = boot_and_run_ide_pc9821(&code, Some((0, make_ide_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x0F, "IDE new sense");
    let bx = read_ram_u16(&state.memory.ram, RESULT as usize + 2);
    assert_eq!(bx, 0x0200, "BX should be sector size (512)");
    let cx = read_ram_u16(&state.memory.ram, RESULT as usize + 4);
    assert_eq!(cx, 19, "CX should be cylinders - 1 (20 - 1 = 19)");
    let dx = read_ram_u16(&state.memory.ram, RESULT as usize + 6);
    assert_eq!(dx, 0x0411, "DX should encode DH=heads(4), DL=sectors(17)");
}

#[test]
fn int1bh_ide_read_uses_segment_base_in_protected_mode_pc9821() {
    const ES_SELECTOR: u16 = 0x1234;
    const WRONG_RESULT: u32 = (ES_SELECTOR as u32) << 4;

    let code = make_int1bh_hle_trap(0x06, DA_IDE_CHS_DRIVE0, 0x0200, 0x0000, 0x0005, 0x0000);
    let mut machine = create_machine_pc9821as();
    machine.bus.insert_hdd(0, make_ide_test_drive(), None);
    boot_to_halt!(machine);
    write_bytes(&mut machine.bus, TEST_CODE, &code);

    for i in 0..512u32 {
        machine.bus.write_byte(RESULT + i, 0x00);
        machine.bus.write_byte(WRONG_RESULT + i, 0x00);
    }

    machine.cpu.load_state(&{
        let mut s = cpu::I386State {
            ip: TEST_CODE as u16,
            cr0: 0x0000_0001,
            ..Default::default()
        };
        s.set_esp(0x4000);

        s.set_cs(0x0008);
        s.seg_bases[cpu::SegReg32::CS as usize] = 0x0000_0000;
        s.seg_limits[cpu::SegReg32::CS as usize] = 0x0000_FFFF;
        s.seg_rights[cpu::SegReg32::CS as usize] = 0x9B;

        s.set_ss(0x0010);
        s.seg_bases[cpu::SegReg32::SS as usize] = 0x0000_0000;
        s.seg_limits[cpu::SegReg32::SS as usize] = 0x0000_FFFF;
        s.seg_rights[cpu::SegReg32::SS as usize] = 0x93;

        s.set_ds(0x0010);
        s.seg_bases[cpu::SegReg32::DS as usize] = 0x0000_0000;
        s.seg_limits[cpu::SegReg32::DS as usize] = 0x0000_FFFF;
        s.seg_rights[cpu::SegReg32::DS as usize] = 0x93;

        s.set_es(ES_SELECTOR);
        s.seg_bases[cpu::SegReg32::ES as usize] = RESULT;
        s.seg_limits[cpu::SegReg32::ES as usize] = 0x0000_FFFF;
        s.seg_rights[cpu::SegReg32::ES as usize] = 0x93;

        s.seg_valid = [true, true, true, true, false, false];
        s
    });
    machine.run_for(INT1BH_BUDGET);

    assert_eq!(machine.bus.read_byte(RESULT), 0x00, "sector 5 byte 0");
    assert_eq!(machine.bus.read_byte(RESULT + 1), 0x05, "sector 5 byte 1");
    assert_eq!(
        machine.bus.read_byte(WRONG_RESULT),
        0x00,
        "INT 1Bh IDE read must not use selector<<4 in protected mode"
    );
    assert_eq!(
        machine.bus.read_byte(WRONG_RESULT + 1),
        0x00,
        "INT 1Bh IDE read must not use selector<<4 in protected mode"
    );
}

#[test]
fn int1bh_ide_sense_no_drive_pc9821() {
    let code = make_int1bh_simple(0x04, 0x81);
    let machine = boot_and_run_ide_pc9821(&code, Some((0, make_ide_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x60, "IDE sense no drive");
}

#[test]
fn int1bh_ide_read_chs_pc9821() {
    let code = make_int1bh_sasi_rw(
        0x06,
        DA_IDE_CHS_DRIVE0,
        512,
        0x0000,
        0x0005,
        0x0000,
        DATA_BUFFER as u16,
    );
    let machine = boot_and_run_ide_pc9821(&code, Some((0, make_ide_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "IDE read CHS");
    assert_eq!(
        state.memory.ram[DATA_BUFFER as usize], 0x00,
        "sector 5 byte 0 (LBA high)"
    );
    assert_eq!(
        state.memory.ram[DATA_BUFFER as usize + 1],
        0x05,
        "sector 5 byte 1 (LBA low)"
    );
}

#[test]
fn int1bh_ide_read_no_drive_pc9821() {
    let code = make_int1bh_sasi_rw(0x06, 0x81, 512, 0, 0, 0x0000, DATA_BUFFER as u16);
    let machine = boot_and_run_ide_pc9821(&code, Some((0, make_ide_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x60, "IDE read no drive");
}

fn make_ide_write_and_readback_code() -> Vec<u8> {
    let buf_lo = (DATA_BUFFER & 0xFF) as u8;
    let buf_hi = ((DATA_BUFFER >> 8) & 0xFF) as u8;
    let result_lo = (RESULT & 0xFF) as u8;
    let result_hi = ((RESULT >> 8) & 0xFF) as u8;
    vec![
        // Fill DATA_BUFFER with 0xCC (512 bytes using REP STOSB)
        0x31,
        0xC0, // XOR AX, AX
        0x8E,
        0xC0, // MOV ES, AX
        0xBF,
        buf_lo,
        buf_hi, // MOV DI, DATA_BUFFER
        0xB0,
        0xCC, // MOV AL, 0xCC
        0xB9,
        0x00,
        0x02, // MOV CX, 512
        0xFC, // CLD
        0xF3,
        0xAA, // REP STOSB
        // INT 1Bh IDE write: AH=0x05, AL=0x80 (CHS drive 0)
        0x31,
        0xC0, // XOR AX, AX
        0x8E,
        0xC0, // MOV ES, AX
        0xBD,
        buf_lo,
        buf_hi, // MOV BP, DATA_BUFFER
        0xBB,
        0x00,
        0x02, // MOV BX, 512
        0xB9,
        0x00,
        0x00, // MOV CX, cylinder=0
        0xBA,
        0x01,
        0x00, // MOV DX, head=0, sector=1
        0xB8,
        0x80,
        0x05, // MOV AX, 0x05:0x80
        0xCD,
        0x1B, // INT 0x1B
        0xA3,
        result_lo,
        result_hi, // MOV [RESULT], AX
        // Clear buffer before readback
        0x31,
        0xC0, // XOR AX, AX
        0x8E,
        0xC0, // MOV ES, AX
        0xBF,
        buf_lo,
        buf_hi, // MOV DI, DATA_BUFFER
        0xB0,
        0x00, // MOV AL, 0x00
        0xB9,
        0x00,
        0x02, // MOV CX, 512
        0xFC, // CLD
        0xF3,
        0xAA, // REP STOSB
        // INT 1Bh IDE read: AH=0x06, AL=0x80 (CHS drive 0), same C/H/S
        0x31,
        0xC0, // XOR AX, AX
        0x8E,
        0xC0, // MOV ES, AX
        0xBD,
        buf_lo,
        buf_hi, // MOV BP, DATA_BUFFER
        0xBB,
        0x00,
        0x02, // MOV BX, 512
        0xB9,
        0x00,
        0x00, // MOV CX, cylinder=0
        0xBA,
        0x01,
        0x00, // MOV DX, head=0, sector=1
        0xB8,
        0x80,
        0x06, // MOV AX, 0x06:0x80
        0xCD,
        0x1B, // INT 0x1B
        0x89,
        0x06,
        (result_lo + 2),
        result_hi, // MOV [RESULT+2], AX
        0xF4,      // HLT
    ]
}

#[test]
fn int1bh_ide_write_chs_pc9821() {
    let code = make_ide_write_and_readback_code();
    let machine = boot_and_run_ide_pc9821(&code, Some((0, make_ide_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    let write_ax = read_ram_u16(&state.memory.ram, RESULT as usize);
    let write_ah = (write_ax >> 8) as u8;
    assert_eq!(
        write_ah, 0x00,
        "IDE write AH should be 0x00 (got {write_ah:#04X})"
    );
    let read_ax = read_ram_u16(&state.memory.ram, RESULT as usize + 2);
    let read_ah = (read_ax >> 8) as u8;
    assert_eq!(
        read_ah, 0x00,
        "IDE readback AH should be 0x00 (got {read_ah:#04X})"
    );
    for i in 0..512usize {
        assert_eq!(
            state.memory.ram[DATA_BUFFER as usize + i],
            0xCC,
            "readback mismatch at offset {i}"
        );
    }
}

#[test]
fn int1bh_ide_retract_pc9821() {
    let code = make_int1bh_simple(0x07, DA_IDE_CHS_DRIVE0);
    let machine = boot_and_run_ide_pc9821(&code, Some((0, make_ide_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "IDE retract");
}

#[test]
fn int1bh_ide_mode_set_pc9821() {
    let code = make_int1bh_simple(0x0E, DA_IDE_CHS_DRIVE0);
    let machine = boot_and_run_ide_pc9821(&code, Some((0, make_ide_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "IDE mode set");
}

#[test]
fn int1bh_ide_format_pc9821() {
    let code = make_int1bh_sasi_rw(0x0D, DA_IDE_CHS_DRIVE0, 0, 0x0001, 0x0000, 0x0000, 0x0000);
    let machine = boot_and_run_ide_pc9821(&code, Some((0, make_ide_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "IDE format");
}

// INT 1Bh function codes with high nibble set must be dispatched correctly
// via the lower nibble mask (e.g. AH=0x8E -> mode set, AH=0x21 -> verify).

#[test]
fn int1bh_ide_mode_set_high_nibble_pc9821() {
    // AH=0x8E: lower nibble 0x0E = mode set. Must succeed like AH=0x0E.
    let code = make_int1bh_simple(0x8E, DA_IDE_CHS_DRIVE0);
    let machine = boot_and_run_ide_pc9821(&code, Some((0, make_ide_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "IDE mode set AH=0x8E");
}

#[test]
fn int1bh_ide_verify_high_nibble_pc9821() {
    // AH=0x21: lower nibble 0x01 = verify. Must succeed like AH=0x01.
    let code = make_int1bh_simple(0x21, DA_IDE_CHS_DRIVE0);
    let machine = boot_and_run_ide_pc9821(&code, Some((0, make_ide_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "IDE verify AH=0x21");
}

#[test]
fn int1bh_sasi_mode_set_high_nibble_ra() {
    // AH=0x8E: lower nibble 0x0E = mode set. Must succeed like AH=0x0E.
    let code = make_int1bh_simple(0x8E, DA_SASI_CHS_DRIVE0);
    let machine = boot_and_run_sasi_ra(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "SASI mode set AH=0x8E");
}

#[test]
fn int1bh_sasi_verify_high_nibble_ra() {
    // AH=0x21: lower nibble 0x01 = verify. Must succeed like AH=0x01.
    let code = make_int1bh_simple(0x21, DA_SASI_CHS_DRIVE0);
    let machine = boot_and_run_sasi_ra(&code, Some((0, make_sasi_test_drive())), INT1BH_BUDGET);
    let state = machine.inspection_state();
    assert_result_ah(&state.memory.ram, 0x00, "SASI verify AH=0x21");
}
