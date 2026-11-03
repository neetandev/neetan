//! Far-pointer loads via LDS/LES/LFS/LGS/LSS, derived from the 80486 PRM
//! Chapter 26.
//!
//! Each instruction reads a 32- or 48-bit pointer (offset + selector) from
//! memory and loads the selector into its target segment register. A failed
//! segment load preserves both the destination register and the segment
//! cache. LSS additionally suppresses interrupts for one instruction after a
//! successful SS load, just like MOV SS and POP SS.

use common::Cpu as _;
use cpu::{I386State, SegReg32};

use super::setup::{
    ACCESS_DESCRIPTOR_CODE_OR_DATA, ACCESS_DPL_RING0, ACCESS_DPL_RING3, ACCESS_PRESENT,
    ACCESS_TYPE_CODE, ACCESS_TYPE_DATA_WRITABLE, GLOBAL_DESCRIPTOR_TABLE_BASE,
    HANDLER_GENERAL_PROTECTION_IP, HANDLER_INVALID_OPCODE_IP, HANDLER_SEGMENT_NOT_PRESENT_IP,
    HANDLER_STACK_FAULT_IP, INTERRUPT_DESCRIPTOR_TABLE_BASE, RIGHTS_RING0_CODE_READABLE_ACCESSED,
    RIGHTS_RING0_DATA_WRITABLE_ACCESSED, RING0_CODE_BASE, RING3_CODE_BASE, SELECTOR_RING0_CODE,
    SELECTOR_RING0_DATA, SELECTOR_RING0_STACK, SELECTOR_RING3_DATA, SELECTOR_RING3_STACK,
    SHARED_DATA_BASE, TestBus, make_cpu_386, make_cpu_486, place_at, place_code, promote_to_ring3,
    setup_protected_mode_with_handlers, write_dword_at, write_interrupt_gate_386,
    write_segment_descriptor_16bit, write_word_at,
};

const TEST_SLOT_DATA_RING0_NOT_PRESENT: u16 = 10;
const TEST_SLOT_DATA_RING0_READ_ONLY: u16 = 11;
const TEST_SLOT_TSS_PROBE: u16 = 12;
const TEST_SLOT_CODE_NON_READABLE: u16 = 13;
const TEST_SLOT_DATA_RING3_DPL3: u16 = 14;

const SELECTOR_DATA_RING0_NOT_PRESENT: u16 = TEST_SLOT_DATA_RING0_NOT_PRESENT * 8;
const SELECTOR_DATA_RING0_READ_ONLY: u16 = TEST_SLOT_DATA_RING0_READ_ONLY * 8;
const SELECTOR_TSS_PROBE: u16 = TEST_SLOT_TSS_PROBE * 8;
const SELECTOR_CODE_NON_READABLE: u16 = TEST_SLOT_CODE_NON_READABLE * 8;
const SELECTOR_DATA_RING3_DPL3: u16 = TEST_SLOT_DATA_RING3_DPL3 * 8;

const EXTENDED_GDT_LIMIT: u16 = 16 * 8 - 1;

const POINTER_OFFSET: u32 = 0x40;
const POINTER_LINEAR_ADDRESS: u32 = SHARED_DATA_BASE + POINTER_OFFSET;
const PROBE_OFFSET_16: u16 = 0x1234;
const PROBE_OFFSET_32: u32 = 0xDEAD_BEEF;
const SENTINEL_REG: u32 = 0xCAFE_BABE;

const IRQ_TEST_VECTOR: u8 = 0x40;
const IRQ_HANDLER_IP: u16 = 0x9000;

// LDS/LES use one-byte primary opcodes; LFS/LGS/LSS use a 0F-prefixed two-byte opcode.
// Destination register field encodes BX (reg=011); rm=110 is disp16 in 16-bit
// addressing mode, so the operand is at [DS:disp16].
const LDS_BX_DS_DISP16: [u8; 4] = [0xC5, 0x1E, POINTER_OFFSET as u8, 0x00];
const LES_BX_DS_DISP16: [u8; 4] = [0xC4, 0x1E, POINTER_OFFSET as u8, 0x00];
const LFS_BX_DS_DISP16: [u8; 5] = [0x0F, 0xB4, 0x1E, POINTER_OFFSET as u8, 0x00];
const LGS_BX_DS_DISP16: [u8; 5] = [0x0F, 0xB5, 0x1E, POINTER_OFFSET as u8, 0x00];
const LSS_BX_DS_DISP16: [u8; 5] = [0x0F, 0xB2, 0x1E, POINTER_OFFSET as u8, 0x00];
const LDS_EBX_DS_DISP16_OPSIZE: [u8; 5] = [0x66, 0xC5, 0x1E, POINTER_OFFSET as u8, 0x00];
const LES_EBX_DS_DISP16_OPSIZE: [u8; 5] = [0x66, 0xC4, 0x1E, POINTER_OFFSET as u8, 0x00];
const LSS_EBX_DS_DISP16_OPSIZE: [u8; 6] = [0x66, 0x0F, 0xB2, 0x1E, POINTER_OFFSET as u8, 0x00];

// Register-to-register (modrm>=C0) forms must raise #UD.
const LDS_BX_AX_INVALID: [u8; 2] = [0xC5, 0xD8];
const LES_BX_AX_INVALID: [u8; 2] = [0xC4, 0xD8];
const LFS_BX_AX_INVALID: [u8; 3] = [0x0F, 0xB4, 0xD8];
const LGS_BX_AX_INVALID: [u8; 3] = [0x0F, 0xB5, 0xD8];
const LSS_BX_AX_INVALID: [u8; 3] = [0x0F, 0xB2, 0xD8];

const NOP: u8 = 0x90;
const HLT: u8 = 0xF4;

fn install_protected_mode_irq_handler(bus: &mut TestBus) {
    write_interrupt_gate_386(
        bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        IRQ_TEST_VECTOR,
        IRQ_HANDLER_IP as u32,
        SELECTOR_RING0_CODE,
        0,
    );
    bus.ram[(RING0_CODE_BASE + IRQ_HANDLER_IP as u32) as usize] = HLT;
}

fn install_extended_test_descriptors(bus: &mut TestBus) {
    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_DATA_RING0_NOT_PRESENT,
        SHARED_DATA_BASE,
        0xFFFF,
        ACCESS_DPL_RING0 | ACCESS_DESCRIPTOR_CODE_OR_DATA | ACCESS_TYPE_DATA_WRITABLE,
    );
    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_DATA_RING0_READ_ONLY,
        SHARED_DATA_BASE,
        0xFFFF,
        ACCESS_PRESENT | ACCESS_DPL_RING0 | ACCESS_DESCRIPTOR_CODE_OR_DATA,
    );
    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_TSS_PROBE,
        SHARED_DATA_BASE,
        0x0067,
        ACCESS_PRESENT | ACCESS_DPL_RING0 | 0x09,
    );
    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_CODE_NON_READABLE,
        RING0_CODE_BASE,
        0xFFFF,
        ACCESS_PRESENT | ACCESS_DPL_RING0 | ACCESS_DESCRIPTOR_CODE_OR_DATA | ACCESS_TYPE_CODE,
    );
    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_DATA_RING3_DPL3,
        SHARED_DATA_BASE,
        0xFFFF,
        ACCESS_PRESENT
            | ACCESS_DPL_RING3
            | ACCESS_DESCRIPTOR_CODE_OR_DATA
            | ACCESS_TYPE_DATA_WRITABLE,
    );
}

fn standard_protected_mode(bus: &mut TestBus) -> I386State {
    let mut state = setup_protected_mode_with_handlers(bus);
    install_extended_test_descriptors(bus);
    state.gdt_limit = EXTENDED_GDT_LIMIT;
    state
}

fn make_real_mode_state() -> I386State {
    let mut state = I386State::default();
    state.set_cs(0x1000);
    state.seg_bases[SegReg32::CS as usize] = 0x0001_0000;
    state.set_ss(0x2000);
    state.seg_bases[SegReg32::SS as usize] = 0x0002_0000;
    state.set_ds(0x4000);
    state.seg_bases[SegReg32::DS as usize] = 0x0004_0000;
    state.set_es(0x3000);
    state.seg_bases[SegReg32::ES as usize] = 0x0003_0000;
    state.set_esp(0x1000);
    state.seg_limits = [0xFFFF; 6];
    state.seg_rights[SegReg32::CS as usize] = RIGHTS_RING0_CODE_READABLE_ACCESSED;
    state.seg_rights[SegReg32::SS as usize] = RIGHTS_RING0_DATA_WRITABLE_ACCESSED;
    state.seg_rights[SegReg32::DS as usize] = RIGHTS_RING0_DATA_WRITABLE_ACCESSED;
    state.seg_rights[SegReg32::ES as usize] = RIGHTS_RING0_DATA_WRITABLE_ACCESSED;
    state.seg_valid = [true, true, true, true, false, false];
    state
}

fn write_far_pointer_16(bus: &mut TestBus, linear: u32, offset: u16, segment: u16) {
    write_word_at(bus, linear, offset);
    write_word_at(bus, linear + 2, segment);
}

fn write_far_pointer_32(bus: &mut TestBus, linear: u32, offset: u32, segment: u16) {
    write_dword_at(bus, linear, offset);
    write_word_at(bus, linear + 4, segment);
}

#[test]
fn lds_real_mode_loads_ds_and_offset() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = make_real_mode_state();
    cpu.load_state(&state);

    let linear = state.seg_bases[SegReg32::DS as usize] + POINTER_OFFSET;
    write_far_pointer_16(&mut bus, linear, PROBE_OFFSET_16, 0x5000);

    place_code(&mut bus, 0x1000, 0, &LDS_BX_DS_DISP16);

    cpu.step(&mut bus);

    assert_eq!(cpu.ds(), 0x5000);
    assert_eq!(cpu.state.seg_bases[SegReg32::DS as usize], 0x0005_0000);
    assert_eq!(cpu.ebx() & 0xFFFF, PROBE_OFFSET_16 as u32);
}

#[test]
fn les_real_mode_loads_es_and_offset() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = make_real_mode_state();
    cpu.load_state(&state);

    let linear = state.seg_bases[SegReg32::DS as usize] + POINTER_OFFSET;
    write_far_pointer_16(&mut bus, linear, PROBE_OFFSET_16, 0x5000);

    place_code(&mut bus, 0x1000, 0, &LES_BX_DS_DISP16);

    cpu.step(&mut bus);

    assert_eq!(cpu.es(), 0x5000);
    assert_eq!(cpu.state.seg_bases[SegReg32::ES as usize], 0x0005_0000);
    assert_eq!(cpu.ebx() & 0xFFFF, PROBE_OFFSET_16 as u32);
}

#[test]
fn lfs_real_mode_loads_fs_and_offset() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = make_real_mode_state();
    cpu.load_state(&state);

    let linear = state.seg_bases[SegReg32::DS as usize] + POINTER_OFFSET;
    write_far_pointer_16(&mut bus, linear, PROBE_OFFSET_16, 0x5000);

    place_code(&mut bus, 0x1000, 0, &LFS_BX_DS_DISP16);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.sregs[SegReg32::FS as usize], 0x5000);
    assert_eq!(cpu.state.seg_bases[SegReg32::FS as usize], 0x0005_0000);
    assert_eq!(cpu.ebx() & 0xFFFF, PROBE_OFFSET_16 as u32);
}

#[test]
fn lgs_real_mode_loads_gs_and_offset() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = make_real_mode_state();
    cpu.load_state(&state);

    let linear = state.seg_bases[SegReg32::DS as usize] + POINTER_OFFSET;
    write_far_pointer_16(&mut bus, linear, PROBE_OFFSET_16, 0x5000);

    place_code(&mut bus, 0x1000, 0, &LGS_BX_DS_DISP16);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.sregs[SegReg32::GS as usize], 0x5000);
    assert_eq!(cpu.ebx() & 0xFFFF, PROBE_OFFSET_16 as u32);
}

#[test]
fn lss_real_mode_loads_ss_and_offset() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = make_real_mode_state();
    cpu.load_state(&state);

    let linear = state.seg_bases[SegReg32::DS as usize] + POINTER_OFFSET;
    write_far_pointer_16(&mut bus, linear, PROBE_OFFSET_16, 0x5000);

    place_code(&mut bus, 0x1000, 0, &LSS_BX_DS_DISP16);

    cpu.step(&mut bus);

    assert_eq!(cpu.ss(), 0x5000);
    assert_eq!(cpu.state.seg_bases[SegReg32::SS as usize], 0x0005_0000);
    assert_eq!(cpu.ebx() & 0xFFFF, PROBE_OFFSET_16 as u32);
}

#[test]
fn lds_real_mode_32bit_operand_loads_full_eoffset() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = make_real_mode_state();
    cpu.load_state(&state);

    let linear = state.seg_bases[SegReg32::DS as usize] + POINTER_OFFSET;
    write_far_pointer_32(&mut bus, linear, PROBE_OFFSET_32, 0x5000);

    place_code(&mut bus, 0x1000, 0, &LDS_EBX_DS_DISP16_OPSIZE);

    cpu.step(&mut bus);

    assert_eq!(cpu.ds(), 0x5000);
    assert_eq!(cpu.ebx(), PROBE_OFFSET_32);
}

#[test]
fn les_real_mode_32bit_operand_loads_full_eoffset() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = make_real_mode_state();
    cpu.load_state(&state);

    let linear = state.seg_bases[SegReg32::DS as usize] + POINTER_OFFSET;
    write_far_pointer_32(&mut bus, linear, PROBE_OFFSET_32, 0x5000);

    place_code(&mut bus, 0x1000, 0, &LES_EBX_DS_DISP16_OPSIZE);

    cpu.step(&mut bus);

    assert_eq!(cpu.es(), 0x5000);
    assert_eq!(cpu.ebx(), PROBE_OFFSET_32);
}

#[test]
fn lds_register_form_raises_invalid_opcode() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = standard_protected_mode(&mut bus);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LDS_BX_AX_INVALID);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_INVALID_OPCODE_IP as u32 + 1);
}

#[test]
fn les_register_form_raises_invalid_opcode() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = standard_protected_mode(&mut bus);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LES_BX_AX_INVALID);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_INVALID_OPCODE_IP as u32 + 1);
}

#[test]
fn lfs_register_form_raises_invalid_opcode() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = standard_protected_mode(&mut bus);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LFS_BX_AX_INVALID);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_INVALID_OPCODE_IP as u32 + 1);
}

#[test]
fn lgs_register_form_raises_invalid_opcode() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = standard_protected_mode(&mut bus);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LGS_BX_AX_INVALID);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_INVALID_OPCODE_IP as u32 + 1);
}

#[test]
fn lss_register_form_raises_invalid_opcode() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = standard_protected_mode(&mut bus);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LSS_BX_AX_INVALID);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_INVALID_OPCODE_IP as u32 + 1);
}

#[test]
fn lds_protected_mode_valid_loads_ds_and_offset() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = standard_protected_mode(&mut bus);
    cpu.load_state(&state);

    write_far_pointer_16(
        &mut bus,
        POINTER_LINEAR_ADDRESS,
        PROBE_OFFSET_16,
        SELECTOR_RING0_DATA,
    );

    place_at(&mut bus, RING0_CODE_BASE, &LDS_BX_DS_DISP16);

    cpu.step(&mut bus);

    assert!(!cpu.halted());
    assert_eq!(cpu.ds(), SELECTOR_RING0_DATA);
    assert_eq!(cpu.ebx() & 0xFFFF, PROBE_OFFSET_16 as u32);
}

#[test]
fn les_protected_mode_valid_loads_es_and_offset() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = standard_protected_mode(&mut bus);
    cpu.load_state(&state);

    write_far_pointer_16(
        &mut bus,
        POINTER_LINEAR_ADDRESS,
        PROBE_OFFSET_16,
        SELECTOR_RING0_DATA,
    );

    place_at(&mut bus, RING0_CODE_BASE, &LES_BX_DS_DISP16);

    cpu.step(&mut bus);

    assert_eq!(cpu.es(), SELECTOR_RING0_DATA);
    assert_eq!(cpu.ebx() & 0xFFFF, PROBE_OFFSET_16 as u32);
}

#[test]
fn lfs_protected_mode_valid_loads_fs_and_offset() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = standard_protected_mode(&mut bus);
    cpu.load_state(&state);

    write_far_pointer_16(
        &mut bus,
        POINTER_LINEAR_ADDRESS,
        PROBE_OFFSET_16,
        SELECTOR_RING0_DATA,
    );

    place_at(&mut bus, RING0_CODE_BASE, &LFS_BX_DS_DISP16);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.sregs[SegReg32::FS as usize], SELECTOR_RING0_DATA);
    assert_eq!(cpu.ebx() & 0xFFFF, PROBE_OFFSET_16 as u32);
}

#[test]
fn lgs_protected_mode_valid_loads_gs_and_offset() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = standard_protected_mode(&mut bus);
    cpu.load_state(&state);

    write_far_pointer_16(
        &mut bus,
        POINTER_LINEAR_ADDRESS,
        PROBE_OFFSET_16,
        SELECTOR_RING0_DATA,
    );

    place_at(&mut bus, RING0_CODE_BASE, &LGS_BX_DS_DISP16);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.sregs[SegReg32::GS as usize], SELECTOR_RING0_DATA);
    assert_eq!(cpu.ebx() & 0xFFFF, PROBE_OFFSET_16 as u32);
}

#[test]
fn lss_protected_mode_valid_loads_ss_and_takes_the_next_irq() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    install_protected_mode_irq_handler(&mut bus);
    state.flags.if_flag = true;
    cpu.load_state(&state);

    write_far_pointer_16(
        &mut bus,
        POINTER_LINEAR_ADDRESS,
        PROBE_OFFSET_16,
        SELECTOR_RING0_STACK,
    );

    place_at(
        &mut bus,
        RING0_CODE_BASE,
        &[
            LSS_BX_DS_DISP16[0],
            LSS_BX_DS_DISP16[1],
            LSS_BX_DS_DISP16[2],
            LSS_BX_DS_DISP16[3],
            LSS_BX_DS_DISP16[4],
            NOP,
            NOP,
        ],
    );

    cpu.step(&mut bus);
    assert_eq!(cpu.ss(), SELECTOR_RING0_STACK);
    assert_eq!(cpu.ebx() & 0xFFFF, PROBE_OFFSET_16 as u32);

    bus.irq_vector = IRQ_TEST_VECTOR;
    cpu.signal_irq();

    // 80486 PRM 3.10.3: LSS does not inhibit interrupts after it.
    cpu.step(&mut bus);
    assert!(cpu.halted(), "the IRQ is delivered right after LSS");
    assert_eq!(cpu.ip(), IRQ_HANDLER_IP as u32 + 1);
}

#[test]
fn lds_protected_mode_data_dpl_below_cpl_preserves_destination_register() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    promote_to_ring3(&mut state);
    state.set_ebx(SENTINEL_REG);
    cpu.load_state(&state);

    write_far_pointer_16(
        &mut bus,
        POINTER_LINEAR_ADDRESS,
        PROBE_OFFSET_16,
        SELECTOR_RING0_DATA,
    );

    place_at(&mut bus, RING3_CODE_BASE, &LDS_BX_DS_DISP16);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
    assert_eq!(
        cpu.ebx(),
        SENTINEL_REG,
        "Destination register must be unchanged when segment load fails"
    );
}

#[test]
fn les_protected_mode_non_readable_code_preserves_destination_register() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx(SENTINEL_REG);
    cpu.load_state(&state);

    write_far_pointer_16(
        &mut bus,
        POINTER_LINEAR_ADDRESS,
        PROBE_OFFSET_16,
        SELECTOR_CODE_NON_READABLE,
    );

    place_at(&mut bus, RING0_CODE_BASE, &LES_BX_DS_DISP16);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
    assert_eq!(cpu.ebx(), SENTINEL_REG);
}

#[test]
fn lfs_protected_mode_tss_descriptor_preserves_destination_register() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx(SENTINEL_REG);
    cpu.load_state(&state);

    write_far_pointer_16(
        &mut bus,
        POINTER_LINEAR_ADDRESS,
        PROBE_OFFSET_16,
        SELECTOR_TSS_PROBE,
    );

    place_at(&mut bus, RING0_CODE_BASE, &LFS_BX_DS_DISP16);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
    assert_eq!(cpu.ebx(), SENTINEL_REG);
}

#[test]
fn lgs_protected_mode_descriptor_present_zero_raises_segment_not_present() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx(SENTINEL_REG);
    cpu.load_state(&state);

    write_far_pointer_16(
        &mut bus,
        POINTER_LINEAR_ADDRESS,
        PROBE_OFFSET_16,
        SELECTOR_DATA_RING0_NOT_PRESENT,
    );

    place_at(&mut bus, RING0_CODE_BASE, &LGS_BX_DS_DISP16);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_SEGMENT_NOT_PRESENT_IP as u32 + 1);
    assert_eq!(cpu.ebx(), SENTINEL_REG);
}

#[test]
fn lss_protected_mode_read_only_data_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx(SENTINEL_REG);
    cpu.load_state(&state);

    write_far_pointer_16(
        &mut bus,
        POINTER_LINEAR_ADDRESS,
        PROBE_OFFSET_16,
        SELECTOR_DATA_RING0_READ_ONLY,
    );

    place_at(&mut bus, RING0_CODE_BASE, &LSS_BX_DS_DISP16);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
    assert_eq!(cpu.ebx(), SENTINEL_REG);
}

#[test]
fn lss_protected_mode_dpl_not_equal_cpl_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = standard_protected_mode(&mut bus);
    cpu.load_state(&state);

    write_far_pointer_16(
        &mut bus,
        POINTER_LINEAR_ADDRESS,
        PROBE_OFFSET_16,
        SELECTOR_DATA_RING3_DPL3 | 3,
    );

    place_at(&mut bus, RING0_CODE_BASE, &LSS_BX_DS_DISP16);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn lss_protected_mode_rpl_not_equal_cpl_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = standard_protected_mode(&mut bus);
    cpu.load_state(&state);

    write_far_pointer_16(
        &mut bus,
        POINTER_LINEAR_ADDRESS,
        PROBE_OFFSET_16,
        SELECTOR_RING0_STACK | 3,
    );

    place_at(&mut bus, RING0_CODE_BASE, &LSS_BX_DS_DISP16);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn lss_protected_mode_descriptor_present_zero_raises_stack_fault() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = standard_protected_mode(&mut bus);
    cpu.load_state(&state);

    write_far_pointer_16(
        &mut bus,
        POINTER_LINEAR_ADDRESS,
        PROBE_OFFSET_16,
        SELECTOR_DATA_RING0_NOT_PRESENT,
    );

    place_at(&mut bus, RING0_CODE_BASE, &LSS_BX_DS_DISP16);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_STACK_FAULT_IP as u32 + 1);
}

#[test]
fn lss_protected_mode_null_at_cpl0_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = standard_protected_mode(&mut bus);
    cpu.load_state(&state);

    write_far_pointer_16(&mut bus, POINTER_LINEAR_ADDRESS, PROBE_OFFSET_16, 0);

    place_at(&mut bus, RING0_CODE_BASE, &LSS_BX_DS_DISP16);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn lss_protected_mode_null_at_cpl3_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    promote_to_ring3(&mut state);
    cpu.load_state(&state);

    write_far_pointer_16(&mut bus, POINTER_LINEAR_ADDRESS, PROBE_OFFSET_16, 0);

    place_at(&mut bus, RING3_CODE_BASE, &LSS_BX_DS_DISP16);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn lds_protected_mode_null_succeeds_with_invalid_segment_cache() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = standard_protected_mode(&mut bus);
    cpu.load_state(&state);

    write_far_pointer_16(&mut bus, POINTER_LINEAR_ADDRESS, PROBE_OFFSET_16, 0);

    place_at(&mut bus, RING0_CODE_BASE, &LDS_BX_DS_DISP16);

    cpu.step(&mut bus);

    assert!(!cpu.halted(), "Null DS via LDS does not fault");
    assert_eq!(cpu.ds(), 0);
    assert!(!cpu.state.seg_valid[SegReg32::DS as usize]);
    assert_eq!(cpu.ebx() & 0xFFFF, PROBE_OFFSET_16 as u32);
}

#[test]
fn les_protected_mode_null_succeeds_with_invalid_segment_cache() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = standard_protected_mode(&mut bus);
    cpu.load_state(&state);

    write_far_pointer_16(&mut bus, POINTER_LINEAR_ADDRESS, PROBE_OFFSET_16, 0);

    place_at(&mut bus, RING0_CODE_BASE, &LES_BX_DS_DISP16);

    cpu.step(&mut bus);

    assert!(!cpu.halted());
    assert_eq!(cpu.es(), 0);
    assert!(!cpu.state.seg_valid[SegReg32::ES as usize]);
    assert_eq!(cpu.ebx() & 0xFFFF, PROBE_OFFSET_16 as u32);
}

#[test]
fn lfs_protected_mode_null_succeeds_with_invalid_segment_cache() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = standard_protected_mode(&mut bus);
    cpu.load_state(&state);

    write_far_pointer_16(&mut bus, POINTER_LINEAR_ADDRESS, PROBE_OFFSET_16, 0);

    place_at(&mut bus, RING0_CODE_BASE, &LFS_BX_DS_DISP16);

    cpu.step(&mut bus);

    assert!(!cpu.halted());
    assert!(!cpu.state.seg_valid[SegReg32::FS as usize]);
    assert_eq!(cpu.ebx() & 0xFFFF, PROBE_OFFSET_16 as u32);
}

#[test]
fn lgs_protected_mode_null_succeeds_with_invalid_segment_cache() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = standard_protected_mode(&mut bus);
    cpu.load_state(&state);

    write_far_pointer_16(&mut bus, POINTER_LINEAR_ADDRESS, PROBE_OFFSET_16, 0);

    place_at(&mut bus, RING0_CODE_BASE, &LGS_BX_DS_DISP16);

    cpu.step(&mut bus);

    assert!(!cpu.halted());
    assert!(!cpu.state.seg_valid[SegReg32::GS as usize]);
    assert_eq!(cpu.ebx() & 0xFFFF, PROBE_OFFSET_16 as u32);
}

#[test]
fn lds_protected_mode_selector_beyond_gdt_limit_preserves_destination() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx(SENTINEL_REG);
    cpu.load_state(&state);

    write_far_pointer_16(&mut bus, POINTER_LINEAR_ADDRESS, PROBE_OFFSET_16, 0xFFF8);

    place_at(&mut bus, RING0_CODE_BASE, &LDS_BX_DS_DISP16);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
    assert_eq!(cpu.ebx(), SENTINEL_REG);
}

#[test]
fn lds_protected_mode_pointer_fetch_beyond_segment_limit_preserves_ds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.seg_limits[SegReg32::DS as usize] = 0x10;
    cpu.load_state(&state);

    let original_ds = cpu.ds();

    place_at(&mut bus, RING0_CODE_BASE, &LDS_BX_DS_DISP16);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted(), "DS limit overrun must fault");
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
    assert_eq!(
        cpu.ds(),
        original_ds,
        "DS register unchanged when pointer fetch faults"
    );
}

#[test]
fn lds_protected_mode_sets_accessed_bit_on_success() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = standard_protected_mode(&mut bus);
    let descriptor_address =
        GLOBAL_DESCRIPTOR_TABLE_BASE + (TEST_SLOT_DATA_RING0_READ_ONLY as u32) * 8;
    bus.ram[(descriptor_address + 5) as usize] =
        ACCESS_PRESENT | ACCESS_DPL_RING0 | ACCESS_DESCRIPTOR_CODE_OR_DATA;
    cpu.load_state(&state);

    write_far_pointer_16(
        &mut bus,
        POINTER_LINEAR_ADDRESS,
        PROBE_OFFSET_16,
        SELECTOR_DATA_RING0_READ_ONLY,
    );

    place_at(&mut bus, RING0_CODE_BASE, &LDS_BX_DS_DISP16);

    cpu.step(&mut bus);

    assert!(!cpu.halted());
    assert_eq!(
        bus.ram[(descriptor_address + 5) as usize] & 0x01,
        0x01,
        "LDS sets the accessed bit on successful load"
    );
}

#[test]
fn lss_protected_mode_at_cpl3_with_ring3_stack_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    promote_to_ring3(&mut state);
    cpu.load_state(&state);

    let pointer_linear = state.seg_bases[SegReg32::DS as usize] + POINTER_OFFSET;
    write_far_pointer_16(
        &mut bus,
        pointer_linear,
        PROBE_OFFSET_16,
        SELECTOR_RING3_STACK,
    );

    place_at(&mut bus, RING3_CODE_BASE, &LSS_BX_DS_DISP16);

    cpu.step(&mut bus);

    assert!(!cpu.halted());
    assert_eq!(cpu.ss(), SELECTOR_RING3_STACK);
}

#[test]
fn lds_protected_mode_at_cpl3_with_ring3_data_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    promote_to_ring3(&mut state);
    cpu.load_state(&state);

    let pointer_linear = state.seg_bases[SegReg32::DS as usize] + POINTER_OFFSET;
    write_far_pointer_16(
        &mut bus,
        pointer_linear,
        PROBE_OFFSET_16,
        SELECTOR_RING3_DATA,
    );

    place_at(&mut bus, RING3_CODE_BASE, &LDS_BX_DS_DISP16);

    cpu.step(&mut bus);

    assert!(!cpu.halted());
    assert_eq!(cpu.ds(), SELECTOR_RING3_DATA);
}

#[test]
fn lss_protected_mode_32bit_operand_loads_full_eoffset() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = standard_protected_mode(&mut bus);
    cpu.load_state(&state);

    write_far_pointer_32(
        &mut bus,
        POINTER_LINEAR_ADDRESS,
        PROBE_OFFSET_32,
        SELECTOR_RING0_STACK,
    );

    place_at(&mut bus, RING0_CODE_BASE, &LSS_EBX_DS_DISP16_OPSIZE);

    cpu.step(&mut bus);

    assert_eq!(cpu.ss(), SELECTOR_RING0_STACK);
    assert_eq!(cpu.ebx(), PROBE_OFFSET_32);
}

#[test]
fn les_protected_mode_failed_load_preserves_ebx_full_dword() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx(0xAAAA_BBBB);
    cpu.load_state(&state);

    write_far_pointer_16(&mut bus, POINTER_LINEAR_ADDRESS, PROBE_OFFSET_16, 0xFFF8);

    place_at(&mut bus, RING0_CODE_BASE, &LES_BX_DS_DISP16);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ebx(), 0xAAAA_BBBB);
}

#[test]
fn lds_486_protected_mode_matches_386_semantics() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let state = standard_protected_mode(&mut bus);
    cpu.load_state(&state);

    write_far_pointer_16(
        &mut bus,
        POINTER_LINEAR_ADDRESS,
        PROBE_OFFSET_16,
        SELECTOR_RING0_DATA,
    );

    place_at(&mut bus, RING0_CODE_BASE, &LDS_BX_DS_DISP16);

    cpu.step(&mut bus);

    assert_eq!(cpu.ds(), SELECTOR_RING0_DATA);
    assert_eq!(cpu.ebx() & 0xFFFF, PROBE_OFFSET_16 as u32);
}

#[test]
fn lss_486_protected_mode_takes_the_following_irq() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    install_protected_mode_irq_handler(&mut bus);
    state.flags.if_flag = true;
    cpu.load_state(&state);

    write_far_pointer_16(
        &mut bus,
        POINTER_LINEAR_ADDRESS,
        PROBE_OFFSET_16,
        SELECTOR_RING0_STACK,
    );

    place_at(
        &mut bus,
        RING0_CODE_BASE,
        &[
            LSS_BX_DS_DISP16[0],
            LSS_BX_DS_DISP16[1],
            LSS_BX_DS_DISP16[2],
            LSS_BX_DS_DISP16[3],
            LSS_BX_DS_DISP16[4],
            NOP,
            NOP,
        ],
    );

    cpu.step(&mut bus);

    bus.irq_vector = IRQ_TEST_VECTOR;
    cpu.signal_irq();

    // 80486 PRM 3.10.3 and 9.3.4: only MOV SS and POP SS inhibit interrupts.
    cpu.step(&mut bus);
    assert!(cpu.halted());
    assert_eq!(cpu.ip(), IRQ_HANDLER_IP as u32 + 1);
}

#[test]
fn lds_protected_mode_pointer_at_segment_limit_boundary_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.seg_limits[SegReg32::DS as usize] = POINTER_OFFSET + 3;
    cpu.load_state(&state);

    write_far_pointer_16(
        &mut bus,
        POINTER_LINEAR_ADDRESS,
        PROBE_OFFSET_16,
        SELECTOR_RING0_DATA,
    );

    place_at(&mut bus, RING0_CODE_BASE, &LDS_BX_DS_DISP16);

    cpu.step(&mut bus);

    assert!(!cpu.halted(), "Pointer fetch within DS limit must succeed");
    assert_eq!(cpu.ds(), SELECTOR_RING0_DATA);
}

#[test]
fn lss_real_mode_takes_the_next_irq() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = make_real_mode_state();
    state.flags.if_flag = true;
    cpu.load_state(&state);

    let pointer_linear = state.seg_bases[SegReg32::DS as usize] + POINTER_OFFSET;
    write_far_pointer_16(&mut bus, pointer_linear, PROBE_OFFSET_16, 0x7000);

    bus.ram[IRQ_TEST_VECTOR as usize * 4] = 0x00;
    bus.ram[IRQ_TEST_VECTOR as usize * 4 + 1] = 0x00;
    bus.ram[IRQ_TEST_VECTOR as usize * 4 + 2] = 0x00;
    bus.ram[IRQ_TEST_VECTOR as usize * 4 + 3] = 0xA0;
    bus.ram[0x000A_0000] = HLT;

    place_code(
        &mut bus,
        0x1000,
        0,
        &[
            LSS_BX_DS_DISP16[0],
            LSS_BX_DS_DISP16[1],
            LSS_BX_DS_DISP16[2],
            LSS_BX_DS_DISP16[3],
            LSS_BX_DS_DISP16[4],
            NOP,
            NOP,
        ],
    );

    cpu.step(&mut bus);
    assert_eq!(cpu.ss(), 0x7000);

    bus.irq_vector = IRQ_TEST_VECTOR;
    cpu.signal_irq();

    // 80486 PRM 3.10.3: LSS does not inhibit interrupts in real mode either.
    cpu.step(&mut bus);
    assert!(cpu.halted(), "the IRQ is delivered right after LSS");
    assert_eq!(cpu.cs(), 0xA000);
}
