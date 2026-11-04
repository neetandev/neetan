//! Segment-register loads via MOV sreg and POP sreg, derived from the 80486
//! PRM Chapter 26.
//!
//! Edges: null SS at CPL>=0 -> #GP(0); null DS/ES/FS/GS at any CPL succeeds
//! with the cache marked invalid; not-present descriptor -> #NP/#SS;
//! type and DPL faults; the one-instruction interrupt-inhibit window after
//! a successful SS load.

use common::Cpu as _;
use cpu::{I386State, SegReg32};

use super::setup::{
    ACCESS_DESCRIPTOR_CODE_OR_DATA, ACCESS_DPL_RING0, ACCESS_DPL_RING3, ACCESS_PRESENT,
    ACCESS_TYPE_CODE, ACCESS_TYPE_CODE_CONFORMING, ACCESS_TYPE_CODE_READABLE,
    ACCESS_TYPE_DATA_WRITABLE, GLOBAL_DESCRIPTOR_TABLE_BASE, HANDLER_GENERAL_PROTECTION_IP,
    HANDLER_INVALID_OPCODE_IP, HANDLER_SEGMENT_NOT_PRESENT_IP, HANDLER_STACK_FAULT_IP,
    INTERRUPT_DESCRIPTOR_TABLE_BASE, RIGHTS_RING0_CODE_READABLE_ACCESSED,
    RIGHTS_RING0_DATA_WRITABLE_ACCESSED, RING0_CODE_BASE, RING3_CODE_BASE, SELECTOR_RING0_CODE,
    SELECTOR_RING0_DATA, SELECTOR_RING0_STACK, SELECTOR_RING3_DATA, SELECTOR_RING3_STACK,
    SHARED_DATA_BASE, TestBus, make_cpu_386, make_cpu_486, place_at, place_code, promote_to_ring3,
    setup_protected_mode_with_handlers, write_interrupt_gate_386, write_segment_descriptor_16bit,
};

const TEST_SLOT_DATA_RING0_NOT_PRESENT: u16 = 10;
const TEST_SLOT_DATA_RING0_READ_ONLY: u16 = 11;
const TEST_SLOT_TSS_PROBE: u16 = 12;
const TEST_SLOT_CODE_NON_READABLE: u16 = 13;
const TEST_SLOT_CODE_RING0_CONFORMING: u16 = 14;
const TEST_SLOT_CODE_RING0_NON_CONFORMING: u16 = 15;
const TEST_SLOT_DATA_RING3_DPL3: u16 = 16;

const SELECTOR_DATA_RING0_NOT_PRESENT: u16 = TEST_SLOT_DATA_RING0_NOT_PRESENT * 8;
const SELECTOR_DATA_RING0_READ_ONLY: u16 = TEST_SLOT_DATA_RING0_READ_ONLY * 8;
const SELECTOR_TSS_PROBE: u16 = TEST_SLOT_TSS_PROBE * 8;
const SELECTOR_CODE_NON_READABLE: u16 = TEST_SLOT_CODE_NON_READABLE * 8;
const SELECTOR_CODE_RING0_CONFORMING: u16 = TEST_SLOT_CODE_RING0_CONFORMING * 8;
const SELECTOR_CODE_RING0_NON_CONFORMING: u16 = TEST_SLOT_CODE_RING0_NON_CONFORMING * 8;
const SELECTOR_DATA_RING3_DPL3: u16 = TEST_SLOT_DATA_RING3_DPL3 * 8;

const EXTENDED_GDT_ENTRIES: u16 = 18;
const EXTENDED_GDT_LIMIT: u16 = EXTENDED_GDT_ENTRIES * 8 - 1;

const IRQ_TEST_VECTOR: u8 = 0x40;
const IRQ_HANDLER_IP: u16 = 0x9000;

const MOV_ES_AX: [u8; 2] = [0x8E, 0xC0];
const MOV_CS_AX: [u8; 2] = [0x8E, 0xC8];
const MOV_SS_AX: [u8; 2] = [0x8E, 0xD0];
const MOV_DS_AX: [u8; 2] = [0x8E, 0xD8];
const MOV_FS_AX: [u8; 2] = [0x8E, 0xE0];
const MOV_GS_AX: [u8; 2] = [0x8E, 0xE8];
const POP_ES: u8 = 0x07;
const POP_SS: u8 = 0x17;
const POP_DS: u8 = 0x1F;
const POP_FS: [u8; 2] = [0x0F, 0xA1];
const POP_GS: [u8; 2] = [0x0F, 0xA9];
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
        TEST_SLOT_CODE_RING0_CONFORMING,
        RING0_CODE_BASE,
        0xFFFF,
        ACCESS_PRESENT
            | ACCESS_DPL_RING0
            | ACCESS_DESCRIPTOR_CODE_OR_DATA
            | ACCESS_TYPE_CODE
            | ACCESS_TYPE_CODE_CONFORMING
            | ACCESS_TYPE_CODE_READABLE,
    );
    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_CODE_RING0_NON_CONFORMING,
        RING0_CODE_BASE,
        0xFFFF,
        RIGHTS_RING0_CODE_READABLE_ACCESSED,
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

fn make_real_mode_state(cs_segment: u16, cs_base: u32) -> I386State {
    let mut state = I386State::default();
    state.set_cs(cs_segment);
    state.seg_bases[SegReg32::CS as usize] = cs_base;
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

/// MOV DS with a descriptor whose accessed bit is already set writes nothing to the
/// descriptor table, so the PTE of the GDT page keeps D=0. A clear accessed bit is set
/// and marks the page dirty (observed behaviour).
#[test]
fn mov_ds_sets_the_gdt_page_dirty_only_when_it_writes_the_accessed_bit() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();
    let mut state = super::setup::setup_protected_mode(&mut bus, 0xFFFF);
    super::setup::enable_identity_paging(&mut bus, &mut state);
    state.set_eax(u32::from(SELECTOR_RING0_DATA));
    cpu.load_state(&state);
    let gdt_entry_address =
        super::setup::PAGE_TABLE_BASE + (GLOBAL_DESCRIPTOR_TABLE_BASE >> 12) * 4;
    let page_dirty = 0x40;

    place_at(&mut bus, RING0_CODE_BASE, &[0x8E, 0xD8, 0x8E, 0xC0]);
    cpu.step(&mut bus);
    assert_eq!(
        super::setup::read_dword_at(&bus, gdt_entry_address) & page_dirty,
        0
    );

    let rights_address = GLOBAL_DESCRIPTOR_TABLE_BASE + 2 * 8 + 5;
    bus.ram[rights_address as usize] &= !0x01;
    cpu.step(&mut bus);
    assert_eq!(bus.ram[rights_address as usize] & 0x01, 0x01);
    assert_eq!(
        super::setup::read_dword_at(&bus, gdt_entry_address) & page_dirty,
        page_dirty
    );
}

#[test]
fn mov_es_real_mode_loads_segment_register_and_base() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = make_real_mode_state(0x1000, 0x0001_0000);
    state.set_eax(0x5000);
    cpu.load_state(&state);

    place_at(&mut bus, 0x0001_0000, &MOV_ES_AX);

    cpu.step(&mut bus);

    assert_eq!(cpu.es(), 0x5000);
    assert_eq!(cpu.state.seg_bases[SegReg32::ES as usize], 0x0005_0000);
}

#[test]
fn mov_ds_real_mode_loads_segment_register_and_base() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = make_real_mode_state(0x1000, 0x0001_0000);
    state.set_eax(0x6000);
    cpu.load_state(&state);

    place_at(&mut bus, 0x0001_0000, &MOV_DS_AX);

    cpu.step(&mut bus);

    assert_eq!(cpu.ds(), 0x6000);
    assert_eq!(cpu.state.seg_bases[SegReg32::DS as usize], 0x0006_0000);
}

#[test]
fn mov_ss_real_mode_loads_segment_and_inhibits_one_following_irq() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = make_real_mode_state(0x1000, 0x0001_0000);
    state.set_eax(0x7000);
    state.flags.if_flag = true;
    cpu.load_state(&state);

    bus.ram[IRQ_TEST_VECTOR as usize * 4] = 0x00;
    bus.ram[IRQ_TEST_VECTOR as usize * 4 + 1] = 0x00;
    bus.ram[IRQ_TEST_VECTOR as usize * 4 + 2] = 0x00;
    bus.ram[IRQ_TEST_VECTOR as usize * 4 + 3] = 0xA0;
    bus.ram[0x000A_0000] = HLT;

    place_at(
        &mut bus,
        0x0001_0000,
        &[MOV_SS_AX[0], MOV_SS_AX[1], NOP, NOP],
    );

    cpu.step(&mut bus);
    assert_eq!(cpu.ss(), 0x7000);

    bus.irq_vector = IRQ_TEST_VECTOR;
    cpu.signal_irq();

    cpu.step(&mut bus);
    assert!(!cpu.halted(), "IRQ deferred during MOV SS inhibit window");

    cpu.step(&mut bus);
    assert!(
        cpu.halted(),
        "IRQ delivered after the inhibit window expires"
    );
    assert_eq!(cpu.cs(), 0xA000);
}

#[test]
fn mov_fs_real_mode_loads_segment_register_and_base() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = make_real_mode_state(0x1000, 0x0001_0000);
    state.set_eax(0x8000);
    cpu.load_state(&state);

    place_at(&mut bus, 0x0001_0000, &MOV_FS_AX);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.sregs[SegReg32::FS as usize], 0x8000);
    assert_eq!(cpu.state.seg_bases[SegReg32::FS as usize], 0x0008_0000);
}

#[test]
fn mov_gs_real_mode_loads_segment_register_and_base() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = make_real_mode_state(0x1000, 0x0001_0000);
    state.set_eax(0x9000);
    cpu.load_state(&state);

    place_at(&mut bus, 0x0001_0000, &MOV_GS_AX);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.sregs[SegReg32::GS as usize], 0x9000);
    assert_eq!(cpu.state.seg_bases[SegReg32::GS as usize], 0x0009_0000);
}

#[test]
fn mov_cs_real_mode_raises_invalid_opcode() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let handler_cs: u16 = 0x2000;
    bus.ram[6 * 4] = 0;
    bus.ram[6 * 4 + 1] = 0;
    bus.ram[6 * 4 + 2] = handler_cs as u8;
    bus.ram[6 * 4 + 3] = (handler_cs >> 8) as u8;

    place_code(&mut bus, cs, 0, &MOV_CS_AX);

    let mut state = I386State::default();
    state.set_cs(cs);
    state.set_ss(0x3000);
    state.set_esp(0x1000);
    cpu.load_state(&state);

    cpu.step(&mut bus);

    assert_eq!(cpu.cs(), handler_cs);
}

#[test]
fn mov_cs_protected_mode_raises_invalid_opcode() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = standard_protected_mode(&mut bus);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &MOV_CS_AX);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_INVALID_OPCODE_IP as u32 + 1);
}

#[test]
fn mov_es_protected_mode_null_succeeds_with_invalid_segment_cache() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(0);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &MOV_ES_AX);

    cpu.step(&mut bus);

    assert!(!cpu.halted(), "Null DS/ES/FS/GS must not fault on load");
    assert_eq!(cpu.es(), 0);
    assert!(!cpu.state.seg_valid[SegReg32::ES as usize]);
    assert_eq!(cpu.state.seg_bases[SegReg32::ES as usize], 0);
    assert_eq!(cpu.state.seg_limits[SegReg32::ES as usize], 0);
}

#[test]
fn mov_ds_protected_mode_null_succeeds_with_invalid_segment_cache() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(0);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &MOV_DS_AX);

    cpu.step(&mut bus);

    assert!(!cpu.halted());
    assert_eq!(cpu.ds(), 0);
    assert!(!cpu.state.seg_valid[SegReg32::DS as usize]);
}

#[test]
fn mov_fs_protected_mode_null_succeeds_with_invalid_segment_cache() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(0);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &MOV_FS_AX);

    cpu.step(&mut bus);

    assert!(!cpu.halted());
    assert!(!cpu.state.seg_valid[SegReg32::FS as usize]);
}

#[test]
fn mov_gs_protected_mode_null_succeeds_with_invalid_segment_cache() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(0);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &MOV_GS_AX);

    cpu.step(&mut bus);

    assert!(!cpu.halted());
    assert!(!cpu.state.seg_valid[SegReg32::GS as usize]);
}

#[test]
fn mov_ss_protected_mode_null_at_cpl0_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(0);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &MOV_SS_AX);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn mov_ss_protected_mode_null_at_cpl3_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    promote_to_ring3(&mut state);
    state.set_eax(0);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &MOV_SS_AX);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn mov_ds_protected_mode_selector_beyond_gdt_limit_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(0xFFF8);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &MOV_DS_AX);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn mov_ds_protected_mode_valid_data_segment_loads_and_sets_accessed_bit() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    let descriptor_address =
        GLOBAL_DESCRIPTOR_TABLE_BASE + (TEST_SLOT_DATA_RING0_READ_ONLY as u32) * 8;
    bus.ram[(descriptor_address + 5) as usize] =
        ACCESS_PRESENT | ACCESS_DPL_RING0 | ACCESS_DESCRIPTOR_CODE_OR_DATA;
    state.set_eax(SELECTOR_DATA_RING0_READ_ONLY as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &MOV_DS_AX);

    cpu.step(&mut bus);

    assert!(!cpu.halted());
    assert_eq!(cpu.ds(), SELECTOR_DATA_RING0_READ_ONLY);
    assert_eq!(
        bus.ram[(descriptor_address + 5) as usize] & 0x01,
        0x01,
        "Accessed bit must be set on successful load"
    );
}

#[test]
fn mov_ds_protected_mode_data_dpl_below_cpl_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    promote_to_ring3(&mut state);
    state.set_eax(SELECTOR_RING0_DATA as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &MOV_DS_AX);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn mov_ds_protected_mode_executable_readable_code_segment_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(SELECTOR_CODE_RING0_NON_CONFORMING as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &MOV_DS_AX);

    cpu.step(&mut bus);

    assert!(!cpu.halted());
    assert_eq!(cpu.ds(), SELECTOR_CODE_RING0_NON_CONFORMING);
}

#[test]
fn mov_ds_protected_mode_non_readable_code_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(SELECTOR_CODE_NON_READABLE as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &MOV_DS_AX);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn mov_ds_protected_mode_system_descriptor_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(SELECTOR_TSS_PROBE as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &MOV_DS_AX);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn mov_ds_protected_mode_descriptor_present_zero_raises_segment_not_present() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(SELECTOR_DATA_RING0_NOT_PRESENT as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &MOV_DS_AX);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_SEGMENT_NOT_PRESENT_IP as u32 + 1);
}

#[test]
fn mov_ss_protected_mode_read_only_data_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(SELECTOR_DATA_RING0_READ_ONLY as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &MOV_SS_AX);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn mov_ss_protected_mode_dpl_not_equal_cpl_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax((SELECTOR_DATA_RING3_DPL3 | 3) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &MOV_SS_AX);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn mov_ss_protected_mode_rpl_not_equal_cpl_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax((SELECTOR_RING0_DATA | 3) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &MOV_SS_AX);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn mov_ss_protected_mode_descriptor_present_zero_raises_stack_fault() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(SELECTOR_DATA_RING0_NOT_PRESENT as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &MOV_SS_AX);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_STACK_FAULT_IP as u32 + 1);
}

#[test]
fn mov_ss_protected_mode_valid_load_inhibits_one_following_irq() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    install_protected_mode_irq_handler(&mut bus);
    state.flags.if_flag = true;
    state.set_eax(SELECTOR_RING0_STACK as u32);
    cpu.load_state(&state);

    place_at(
        &mut bus,
        RING0_CODE_BASE,
        &[MOV_SS_AX[0], MOV_SS_AX[1], NOP, NOP],
    );

    cpu.step(&mut bus);
    assert_eq!(cpu.ss(), SELECTOR_RING0_STACK);

    bus.irq_vector = IRQ_TEST_VECTOR;
    cpu.signal_irq();

    cpu.step(&mut bus);
    assert!(
        !cpu.halted(),
        "IRQ blocked across the MOV SS inhibit window"
    );

    cpu.step(&mut bus);
    assert!(
        cpu.halted(),
        "IRQ dispatches after the inhibit window expires"
    );
    assert_eq!(cpu.ip(), IRQ_HANDLER_IP as u32 + 1);
}

#[test]
fn mov_ds_protected_mode_conforming_code_from_cpl3_with_dpl0_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    promote_to_ring3(&mut state);
    state.set_eax(SELECTOR_CODE_RING0_CONFORMING as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &MOV_DS_AX);

    cpu.step(&mut bus);

    assert!(!cpu.halted(), "Conforming code skips DS DPL gate");
}

#[test]
fn mov_ds_protected_mode_non_conforming_code_dpl_below_cpl_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    promote_to_ring3(&mut state);
    state.set_eax(SELECTOR_CODE_RING0_NON_CONFORMING as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &MOV_DS_AX);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn mov_es_protected_mode_data_dpl_below_max_cpl_rpl_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax((SELECTOR_RING0_DATA | 3) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &MOV_ES_AX);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn pop_es_real_mode_loads_segment_register_and_base() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = make_real_mode_state(0x1000, 0x0001_0000);
    cpu.load_state(&state);

    let stack_top: u32 = state.seg_bases[SegReg32::SS as usize] + state.esp();
    bus.ram[stack_top as usize] = 0xCD;
    bus.ram[stack_top as usize + 1] = 0xAB;

    place_at(&mut bus, 0x0001_0000, &[POP_ES]);

    cpu.step(&mut bus);

    assert_eq!(cpu.es(), 0xABCD);
    assert_eq!(cpu.state.seg_bases[SegReg32::ES as usize], 0x000A_BCD0);
}

#[test]
fn pop_ds_real_mode_loads_segment_register() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = make_real_mode_state(0x1000, 0x0001_0000);
    cpu.load_state(&state);

    let stack_top: u32 = state.seg_bases[SegReg32::SS as usize] + state.esp();
    bus.ram[stack_top as usize] = 0x34;
    bus.ram[stack_top as usize + 1] = 0x12;

    place_at(&mut bus, 0x0001_0000, &[POP_DS]);

    cpu.step(&mut bus);

    assert_eq!(cpu.ds(), 0x1234);
}

#[test]
fn pop_ss_real_mode_inhibits_one_following_irq() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = make_real_mode_state(0x1000, 0x0001_0000);
    state.flags.if_flag = true;
    cpu.load_state(&state);

    let stack_top: u32 = state.seg_bases[SegReg32::SS as usize] + state.esp();
    bus.ram[stack_top as usize] = 0x00;
    bus.ram[stack_top as usize + 1] = 0x70;

    bus.ram[IRQ_TEST_VECTOR as usize * 4] = 0x00;
    bus.ram[IRQ_TEST_VECTOR as usize * 4 + 1] = 0x00;
    bus.ram[IRQ_TEST_VECTOR as usize * 4 + 2] = 0x00;
    bus.ram[IRQ_TEST_VECTOR as usize * 4 + 3] = 0xA0;
    bus.ram[0x000A_0000] = HLT;

    place_at(&mut bus, 0x0001_0000, &[POP_SS, NOP, NOP]);

    cpu.step(&mut bus);
    assert_eq!(cpu.ss(), 0x7000);

    bus.irq_vector = IRQ_TEST_VECTOR;
    cpu.signal_irq();

    cpu.step(&mut bus);
    assert!(!cpu.halted(), "IRQ deferred across POP SS inhibit window");

    cpu.step(&mut bus);
    assert!(cpu.halted());
    assert_eq!(cpu.cs(), 0xA000);
}

#[test]
fn pop_fs_real_mode_loads_segment_register() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = make_real_mode_state(0x1000, 0x0001_0000);
    cpu.load_state(&state);

    let stack_top: u32 = state.seg_bases[SegReg32::SS as usize] + state.esp();
    bus.ram[stack_top as usize] = 0x00;
    bus.ram[stack_top as usize + 1] = 0x80;

    place_at(&mut bus, 0x0001_0000, &POP_FS);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.sregs[SegReg32::FS as usize], 0x8000);
}

#[test]
fn pop_gs_real_mode_loads_segment_register() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = make_real_mode_state(0x1000, 0x0001_0000);
    cpu.load_state(&state);

    let stack_top: u32 = state.seg_bases[SegReg32::SS as usize] + state.esp();
    bus.ram[stack_top as usize] = 0x00;
    bus.ram[stack_top as usize + 1] = 0x90;

    place_at(&mut bus, 0x0001_0000, &POP_GS);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.sregs[SegReg32::GS as usize], 0x9000);
}

#[test]
fn pop_es_protected_mode_null_succeeds_with_invalid_segment_cache() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = standard_protected_mode(&mut bus);
    cpu.load_state(&state);

    let stack_top: u32 = state.seg_bases[SegReg32::SS as usize] + state.esp();
    bus.ram[stack_top as usize] = 0x00;
    bus.ram[stack_top as usize + 1] = 0x00;

    place_at(&mut bus, RING0_CODE_BASE, &[POP_ES]);

    cpu.step(&mut bus);

    assert!(!cpu.halted());
    assert_eq!(cpu.es(), 0);
    assert!(!cpu.state.seg_valid[SegReg32::ES as usize]);
}

#[test]
fn pop_ss_protected_mode_null_at_cpl0_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = standard_protected_mode(&mut bus);
    cpu.load_state(&state);

    let stack_top: u32 = state.seg_bases[SegReg32::SS as usize] + state.esp();
    bus.ram[stack_top as usize] = 0x00;
    bus.ram[stack_top as usize + 1] = 0x00;

    place_at(&mut bus, RING0_CODE_BASE, &[POP_SS]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn pop_ss_protected_mode_valid_inhibits_one_following_irq() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    install_protected_mode_irq_handler(&mut bus);
    state.flags.if_flag = true;
    cpu.load_state(&state);

    let stack_top: u32 = state.seg_bases[SegReg32::SS as usize] + state.esp();
    bus.ram[stack_top as usize] = SELECTOR_RING0_STACK as u8;
    bus.ram[stack_top as usize + 1] = (SELECTOR_RING0_STACK >> 8) as u8;

    place_at(&mut bus, RING0_CODE_BASE, &[POP_SS, NOP, NOP]);

    cpu.step(&mut bus);
    assert_eq!(cpu.ss(), SELECTOR_RING0_STACK);

    bus.irq_vector = IRQ_TEST_VECTOR;
    cpu.signal_irq();

    cpu.step(&mut bus);
    assert!(!cpu.halted(), "POP SS inhibit window blocks IRQ");

    cpu.step(&mut bus);
    assert!(cpu.halted());
    assert_eq!(cpu.ip(), IRQ_HANDLER_IP as u32 + 1);
}

#[test]
fn pop_ds_protected_mode_valid_loads_and_sets_accessed_bit() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = standard_protected_mode(&mut bus);
    let descriptor_address =
        GLOBAL_DESCRIPTOR_TABLE_BASE + (TEST_SLOT_DATA_RING0_READ_ONLY as u32) * 8;
    bus.ram[(descriptor_address + 5) as usize] =
        ACCESS_PRESENT | ACCESS_DPL_RING0 | ACCESS_DESCRIPTOR_CODE_OR_DATA;
    cpu.load_state(&state);

    let stack_top: u32 = state.seg_bases[SegReg32::SS as usize] + state.esp();
    bus.ram[stack_top as usize] = SELECTOR_DATA_RING0_READ_ONLY as u8;
    bus.ram[stack_top as usize + 1] = (SELECTOR_DATA_RING0_READ_ONLY >> 8) as u8;

    place_at(&mut bus, RING0_CODE_BASE, &[POP_DS]);

    cpu.step(&mut bus);

    assert!(!cpu.halted());
    assert_eq!(cpu.ds(), SELECTOR_DATA_RING0_READ_ONLY);
    assert_eq!(
        bus.ram[(descriptor_address + 5) as usize] & 0x01,
        0x01,
        "POP DS sets the accessed bit on success"
    );
}

#[test]
fn pop_ds_protected_mode_high_rpl_data_below_cpl_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    promote_to_ring3(&mut state);
    cpu.load_state(&state);

    let stack_top: u32 =
        state.seg_bases[SegReg32::SS as usize] + state.regs.dword(cpu::DwordReg::ESP);
    bus.ram[stack_top as usize] = SELECTOR_RING0_DATA as u8;
    bus.ram[stack_top as usize + 1] = (SELECTOR_RING0_DATA >> 8) as u8;

    place_at(&mut bus, RING3_CODE_BASE, &[POP_DS]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn mov_es_at_cpl3_with_ring3_data_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    promote_to_ring3(&mut state);
    state.set_eax(SELECTOR_RING3_DATA as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &MOV_ES_AX);

    cpu.step(&mut bus);

    assert!(!cpu.halted());
    assert_eq!(cpu.es(), SELECTOR_RING3_DATA);
}

#[test]
fn mov_es_protected_mode_null_clears_segment_base_and_limit() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(0);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &MOV_ES_AX);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.seg_bases[SegReg32::ES as usize], 0);
    assert_eq!(cpu.state.seg_limits[SegReg32::ES as usize], 0);
    assert_eq!(cpu.state.seg_rights[SegReg32::ES as usize], 0);
}

#[test]
fn mov_ss_protected_mode_failure_does_not_modify_ss_register() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    let original_ss = state.ss();
    state.set_eax(0);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &MOV_SS_AX);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(
        cpu.state.sregs[SegReg32::SS as usize],
        original_ss,
        "Failed SS load must leave SS register intact"
    );
}

#[test]
fn mov_ss_at_cpl3_with_ring3_stack_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    promote_to_ring3(&mut state);
    state.set_eax(SELECTOR_RING3_STACK as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &MOV_SS_AX);

    cpu.step(&mut bus);

    assert!(!cpu.halted());
    assert_eq!(cpu.ss(), SELECTOR_RING3_STACK);
}

#[test]
fn mov_es_486_protected_mode_matches_386_semantics() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(SELECTOR_RING0_DATA as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &MOV_ES_AX);

    cpu.step(&mut bus);

    assert!(!cpu.halted());
    assert_eq!(cpu.es(), SELECTOR_RING0_DATA);
}

#[test]
fn pop_ds_486_protected_mode_matches_386_semantics() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let state = standard_protected_mode(&mut bus);
    cpu.load_state(&state);

    let stack_top: u32 = state.seg_bases[SegReg32::SS as usize] + state.esp();
    bus.ram[stack_top as usize] = SELECTOR_RING0_DATA as u8;
    bus.ram[stack_top as usize + 1] = (SELECTOR_RING0_DATA >> 8) as u8;

    place_at(&mut bus, RING0_CODE_BASE, &[POP_DS]);

    cpu.step(&mut bus);

    assert!(!cpu.halted());
    assert_eq!(cpu.ds(), SELECTOR_RING0_DATA);
}
