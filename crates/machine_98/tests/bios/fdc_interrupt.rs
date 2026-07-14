use super::{
    boot_and_run_f, boot_and_run_ra, boot_and_run_vm, boot_and_run_vx, create_machine_f,
    create_machine_ra, create_machine_vm, create_machine_vx, read_ivt_vector,
};

const FDC_BUDGET: u64 = 500_000;

const DISK_INTL: usize = 0x055E;
const DISK_INTH: usize = 0x055F;
const DISK_RESULT_1MB: usize = 0x0564;
const DISK_RESULT_640K: usize = 0x05D8;

// RECALIBRATE drive 0 on 1MB FDC (port 0x92), then wait for INT 13h.
#[rustfmt::skip]
const RECALIBRATE_1MB_CODE: &[u8] = &[
    0xC6, 0x06, 0x5E, 0x05, 0x00,  // MOV BYTE [0x055E], 0x00  (clear DISK_INTL)
    0xB0, 0x07,                     // MOV AL, 0x07             (RECALIBRATE command)
    0xE6, 0x92,                     // OUT 0x92, AL             (1MB FDC data port)
    0xB0, 0x00,                     // MOV AL, 0x00             (drive 0)
    0xE6, 0x92,                     // OUT 0x92, AL
    0xFB,                           // STI
    0xF4,                           // HLT                      (wait for INT 13h)
    0xF4,                           // HLT                      (final halt)
];

// Switch to 640KB FDC mode, unmask IRQ 10, RECALIBRATE drive 0, wait for INT 12h.
#[rustfmt::skip]
const RECALIBRATE_640K_CODE: &[u8] = &[
    0xC6, 0x06, 0x5F, 0x05, 0x00,  // MOV BYTE [0x055F], 0x00  (clear DISK_INTH)
    0xB0, 0x02,                     // MOV AL, 0x02             (PORT EXC=0, FDD EXC=1)
    0xE6, 0xBE,                     // OUT 0xBE, AL             (switch to 640KB FDC mode)
    0xE4, 0x0A,                     // IN AL, 0x0A              (read slave PIC IMR)
    0x24, 0xFB,                     // AND AL, 0xFB             (clear bit 2, unmask IRQ 10)
    0xE6, 0x0A,                     // OUT 0x0A, AL             (write slave PIC IMR)
    0xB0, 0x07,                     // MOV AL, 0x07             (RECALIBRATE command)
    0xE6, 0xCA,                     // OUT 0xCA, AL             (640KB FDC data port)
    0xB0, 0x00,                     // MOV AL, 0x00             (drive 0)
    0xE6, 0xCA,                     // OUT 0xCA, AL
    0xFB,                           // STI
    0xF4,                           // HLT                      (wait for INT 12h)
    0xF4,                           // HLT                      (final halt)
];

// ============================================================================
// §8.2 INT 13h - 1MB FDC Vector Setup
// ============================================================================

#[test]
fn int13h_vector_f() {
    let mut machine = create_machine_f();
    let _cycles = boot_to_halt!(machine);
    let state = machine.save_state();

    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x13);
    assert!(
        segment >= 0xFD80,
        "INT 13h segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

#[test]
fn int13h_vector_vm() {
    let mut machine = create_machine_vm();
    let _cycles = boot_to_halt!(machine);
    let state = machine.save_state();

    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x13);
    assert!(
        segment >= 0xFD80,
        "INT 13h segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

#[test]
fn int13h_vector_vx() {
    let mut machine = create_machine_vx();
    let _cycles = boot_to_halt!(machine);
    let state = machine.save_state();

    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x13);
    assert!(
        segment >= 0xFD80,
        "INT 13h segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

#[test]
fn int13h_vector_ra() {
    let mut machine = create_machine_ra();
    let _cycles = boot_to_halt!(machine);
    let state = machine.save_state();

    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x13);
    assert!(
        segment >= 0xFD80,
        "INT 13h segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

// ============================================================================
// §8.2 INT 13h - 1MB FDC RECALIBRATE Sets DISK_INTL Flag
// ============================================================================

#[test]
fn fdc_1mb_recalibrate_sets_intl_flag_f() {
    let (machine, _cycles) = boot_and_run_f(RECALIBRATE_1MB_CODE, &[], FDC_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[DISK_INTL] & 0x01,
        0,
        "F BIOS leaves DISK_INTL bit 0 clear after direct 1MB FDC RECALIBRATE"
    );
}

#[test]
fn fdc_1mb_recalibrate_sets_intl_flag_vm() {
    let (machine, _cycles) = boot_and_run_vm(RECALIBRATE_1MB_CODE, &[], FDC_BUDGET);
    let state = machine.save_state();

    assert_ne!(
        state.memory.ram[DISK_INTL] & 0x01,
        0,
        "DISK_INTL bit 0 should be set after 1MB FDC RECALIBRATE drive 0"
    );
}

#[test]
fn fdc_1mb_recalibrate_sets_intl_flag_vx() {
    let (machine, _cycles) = boot_and_run_vx(RECALIBRATE_1MB_CODE, &[], FDC_BUDGET);
    let state = machine.save_state();

    assert_ne!(
        state.memory.ram[DISK_INTL] & 0x01,
        0,
        "DISK_INTL bit 0 should be set after 1MB FDC RECALIBRATE drive 0"
    );
}

#[test]
fn fdc_1mb_recalibrate_sets_intl_flag_ra() {
    let (machine, _cycles) = boot_and_run_ra(RECALIBRATE_1MB_CODE, &[], FDC_BUDGET);
    let state = machine.save_state();

    assert_ne!(
        state.memory.ram[DISK_INTL] & 0x01,
        0,
        "DISK_INTL bit 0 should be set after 1MB FDC RECALIBRATE drive 0"
    );
}

// ============================================================================
// §8.2 INT 13h - 1MB FDC RECALIBRATE Stores Result
// ============================================================================

#[test]
fn fdc_1mb_recalibrate_stores_result_f() {
    let (machine, _cycles) = boot_and_run_f(RECALIBRATE_1MB_CODE, &[], FDC_BUDGET);
    let state = machine.save_state();

    assert_eq!(state.memory.ram[DISK_RESULT_1MB], 0x00, "ST0");
    assert_eq!(
        state.memory.ram[DISK_RESULT_1MB + 1],
        0x00,
        "PCN should be 0x00 (track 0)"
    );
}

#[test]
fn fdc_1mb_recalibrate_stores_result_vm() {
    let (machine, _cycles) = boot_and_run_vm(RECALIBRATE_1MB_CODE, &[], FDC_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[DISK_RESULT_1MB], 0x20,
        "ST0 should be 0x20 (seek end, drive 0)"
    );
    assert_eq!(
        state.memory.ram[DISK_RESULT_1MB + 1],
        0x00,
        "PCN should be 0x00 (track 0)"
    );
}

#[test]
fn fdc_1mb_recalibrate_stores_result_vx() {
    let (machine, _cycles) = boot_and_run_vx(RECALIBRATE_1MB_CODE, &[], FDC_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[DISK_RESULT_1MB], 0x20,
        "ST0 should be 0x20 (seek end, drive 0)"
    );
    assert_eq!(
        state.memory.ram[DISK_RESULT_1MB + 1],
        0x00,
        "PCN should be 0x00 (track 0)"
    );
}

#[test]
fn fdc_1mb_recalibrate_stores_result_ra() {
    let (machine, _cycles) = boot_and_run_ra(RECALIBRATE_1MB_CODE, &[], FDC_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[DISK_RESULT_1MB], 0x20,
        "ST0 should be 0x20 (seek end, drive 0)"
    );
    assert_eq!(
        state.memory.ram[DISK_RESULT_1MB + 1],
        0x00,
        "PCN should be 0x00 (track 0)"
    );
}

// ============================================================================
// §8.2 INT 13h - 1MB FDC RECALIBRATE Sends EOI
// ============================================================================

#[test]
fn fdc_1mb_recalibrate_sends_eoi_f() {
    let (machine, _cycles) = boot_and_run_f(RECALIBRATE_1MB_CODE, &[], FDC_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.pic.chips[1].isr & 0x08,
        0,
        "Slave IR3 should not be in-service after INT 13h (EOI was sent)"
    );
}

#[test]
fn fdc_1mb_recalibrate_sends_eoi_vm() {
    let (machine, _cycles) = boot_and_run_vm(RECALIBRATE_1MB_CODE, &[], FDC_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.pic.chips[1].isr & 0x08,
        0,
        "Slave IR3 should not be in-service after INT 13h (EOI was sent)"
    );
}

#[test]
fn fdc_1mb_recalibrate_sends_eoi_vx() {
    let (machine, _cycles) = boot_and_run_vx(RECALIBRATE_1MB_CODE, &[], FDC_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.pic.chips[1].isr & 0x08,
        0,
        "Slave IR3 should not be in-service after INT 13h (EOI was sent)"
    );
}

#[test]
fn fdc_1mb_recalibrate_sends_eoi_ra() {
    let (machine, _cycles) = boot_and_run_ra(RECALIBRATE_1MB_CODE, &[], FDC_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.pic.chips[1].isr & 0x08,
        0,
        "Slave IR3 should not be in-service after INT 13h (EOI was sent)"
    );
}

// ============================================================================
// §8.1 INT 12h - 640KB FDC Vector Setup
// ============================================================================

#[test]
fn int12h_vector_f() {
    let mut machine = create_machine_f();
    let _cycles = boot_to_halt!(machine);
    let state = machine.save_state();

    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x12);
    assert!(
        segment >= 0xFD80,
        "INT 12h segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

#[test]
fn int12h_vector_vm() {
    let mut machine = create_machine_vm();
    let _cycles = boot_to_halt!(machine);
    let state = machine.save_state();

    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x12);
    assert!(
        segment >= 0xFD80,
        "INT 12h segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

#[test]
fn int12h_vector_vx() {
    let mut machine = create_machine_vx();
    let _cycles = boot_to_halt!(machine);
    let state = machine.save_state();

    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x12);
    assert!(
        segment >= 0xFD80,
        "INT 12h segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

#[test]
fn int12h_vector_ra() {
    let mut machine = create_machine_ra();
    let _cycles = boot_to_halt!(machine);
    let state = machine.save_state();

    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x12);
    assert!(
        segment >= 0xFD80,
        "INT 12h segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

// ============================================================================
// §8.1 INT 12h - 640KB FDC RECALIBRATE Sets DISK_INTH Flag
// ============================================================================

#[test]
fn fdc_640k_recalibrate_sets_inth_flag_f() {
    let (machine, _cycles) = boot_and_run_f(RECALIBRATE_640K_CODE, &[], FDC_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[DISK_INTH] & 0x10,
        0,
        "F BIOS leaves DISK_INTH bit 4 clear after direct 640KB FDC RECALIBRATE"
    );
}

#[test]
fn fdc_640k_recalibrate_sets_inth_flag_vm() {
    let (machine, _cycles) = boot_and_run_vm(RECALIBRATE_640K_CODE, &[], FDC_BUDGET);
    let state = machine.save_state();

    assert_ne!(
        state.memory.ram[DISK_INTH] & 0x10,
        0,
        "DISK_INTH bit 4 should be set after 640KB FDC RECALIBRATE drive 0"
    );
}

#[test]
fn fdc_640k_recalibrate_sets_inth_flag_vx() {
    let (machine, _cycles) = boot_and_run_vx(RECALIBRATE_640K_CODE, &[], FDC_BUDGET);
    let state = machine.save_state();

    assert_ne!(
        state.memory.ram[DISK_INTH] & 0x10,
        0,
        "DISK_INTH bit 4 should be set after 640KB FDC RECALIBRATE drive 0"
    );
}

#[test]
fn fdc_640k_recalibrate_sets_inth_flag_ra() {
    let (machine, _cycles) = boot_and_run_ra(RECALIBRATE_640K_CODE, &[], FDC_BUDGET);
    let state = machine.save_state();

    assert_ne!(
        state.memory.ram[DISK_INTH] & 0x10,
        0,
        "DISK_INTH bit 4 should be set after 640KB FDC RECALIBRATE drive 0"
    );
}

// ============================================================================
// §8.1 INT 12h - 640KB FDC RECALIBRATE Stores Result
// ============================================================================

#[test]
fn fdc_640k_recalibrate_stores_result_f() {
    let (machine, _cycles) = boot_and_run_f(RECALIBRATE_640K_CODE, &[], FDC_BUDGET);
    let state = machine.save_state();

    assert_eq!(state.memory.ram[DISK_RESULT_640K], 0x00, "ST0");
}

#[test]
fn fdc_640k_recalibrate_stores_result_vm() {
    let (machine, _cycles) = boot_and_run_vm(RECALIBRATE_640K_CODE, &[], FDC_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[DISK_RESULT_640K], 0x20,
        "ST0 should be 0x20 (seek end, drive 0)"
    );
}

#[test]
fn fdc_640k_recalibrate_stores_result_vx() {
    let (machine, _cycles) = boot_and_run_vx(RECALIBRATE_640K_CODE, &[], FDC_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[DISK_RESULT_640K], 0x20,
        "ST0 should be 0x20 (seek end, drive 0)"
    );
}

#[test]
fn fdc_640k_recalibrate_stores_result_ra() {
    let (machine, _cycles) = boot_and_run_ra(RECALIBRATE_640K_CODE, &[], FDC_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[DISK_RESULT_640K], 0x20,
        "ST0 should be 0x20 (seek end, drive 0)"
    );
}

// ============================================================================
// §8.1 INT 12h - 640KB FDC RECALIBRATE Sends EOI
// ============================================================================

#[test]
fn fdc_640k_recalibrate_sends_eoi_f() {
    let (machine, _cycles) = boot_and_run_f(RECALIBRATE_640K_CODE, &[], FDC_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.pic.chips[1].isr & 0x04,
        0,
        "Slave IR2 should not be in-service after INT 12h (EOI was sent)"
    );
}

#[test]
fn fdc_640k_recalibrate_sends_eoi_vm() {
    let (machine, _cycles) = boot_and_run_vm(RECALIBRATE_640K_CODE, &[], FDC_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.pic.chips[1].isr & 0x04,
        0,
        "Slave IR2 should not be in-service after INT 12h (EOI was sent)"
    );
}

#[test]
fn fdc_640k_recalibrate_sends_eoi_vx() {
    let (machine, _cycles) = boot_and_run_vx(RECALIBRATE_640K_CODE, &[], FDC_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.pic.chips[1].isr & 0x04,
        0,
        "Slave IR2 should not be in-service after INT 12h (EOI was sent)"
    );
}

#[test]
fn fdc_640k_recalibrate_sends_eoi_ra() {
    let (machine, _cycles) = boot_and_run_ra(RECALIBRATE_640K_CODE, &[], FDC_BUDGET);
    let state = machine.save_state();

    assert_eq!(
        state.pic.chips[1].isr & 0x04,
        0,
        "Slave IR2 should not be in-service after INT 12h (EOI was sent)"
    );
}
