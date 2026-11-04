//! Selector inspection tests derived from the 80486 PRM Chapter 26.
//!
//! Covers LAR, LSL, VERR, and VERW. ARPL is exercised in detail in
//! system_control.rs and is not duplicated here.
//!
//! Each test names the mode and the specific edge being verified. The
//! expected results follow the manual pseudocode for the instruction.

use common::Cpu as _;

use super::setup::{
    ACCESS_DESCRIPTOR_CODE_OR_DATA, ACCESS_DESCRIPTOR_SYSTEM, ACCESS_DPL_RING0, ACCESS_DPL_RING3,
    ACCESS_PRESENT, ACCESS_TYPE_CODE, GLOBAL_DESCRIPTOR_TABLE_BASE, GRANULARITY_BIG_OR_DEFAULT32,
    GRANULARITY_PAGE, HANDLER_INVALID_OPCODE_IP, INTERRUPT_DESCRIPTOR_TABLE_BASE,
    RIGHTS_RING0_CODE_CONFORMING_READABLE_ACCESSED, RIGHTS_RING0_CODE_READABLE_ACCESSED,
    RIGHTS_RING0_DATA_WRITABLE_ACCESSED, RIGHTS_RING3_CODE_READABLE_ACCESSED,
    RIGHTS_RING3_DATA_WRITABLE_ACCESSED, RIGHTS_TSS_386_AVAILABLE, RIGHTS_TSS_386_BUSY,
    RING0_CODE_BASE, RING3_CODE_BASE, SELECTOR_RING0_CODE, SHARED_DATA_BASE,
    SYSTEM_TYPE_CALL_GATE_286, SYSTEM_TYPE_CALL_GATE_386, SYSTEM_TYPE_INTERRUPT_GATE_286,
    SYSTEM_TYPE_INTERRUPT_GATE_386, SYSTEM_TYPE_LDT, SYSTEM_TYPE_TASK_GATE,
    SYSTEM_TYPE_TRAP_GATE_286, SYSTEM_TYPE_TRAP_GATE_386, SYSTEM_TYPE_TSS_286_AVAILABLE, TestBus,
    make_cpu_386, make_cpu_486, place_at, place_code, promote_to_ring3, read_word_at,
    setup_protected_mode_with_handlers, setup_vm86, write_gate_descriptor,
    write_interrupt_gate_386, write_segment_descriptor, write_segment_descriptor_16bit,
};

// Test-only GDT slots layered on top of the slots that
// setup_protected_mode_with_handlers already populates (0..=9). Each slot is
// referenced by selector = slot * 8 (RPL=0); tests OR an RPL into the low
// two bits where needed.

const TEST_SLOT_CODE_RING0: u16 = 10;
const TEST_SLOT_CODE_RING0_CONFORMING: u16 = 11;
const TEST_SLOT_CODE_RING3: u16 = 12;
const TEST_SLOT_CODE_NON_READABLE: u16 = 13;
const TEST_SLOT_DATA_RING0: u16 = 14;
const TEST_SLOT_DATA_RING3: u16 = 15;
const TEST_SLOT_DATA_READ_ONLY: u16 = 16;
const TEST_SLOT_DATA_RING0_GRANULAR: u16 = 17;
const TEST_SLOT_TSS_286: u16 = 18;
const TEST_SLOT_TSS_386_AVAILABLE_DPL0: u16 = 19;
const TEST_SLOT_LDT_DESCRIPTOR: u16 = 20;
const TEST_SLOT_CALL_GATE_286: u16 = 21;
const TEST_SLOT_CALL_GATE_386: u16 = 22;
const TEST_SLOT_TASK_GATE: u16 = 23;
const TEST_SLOT_INT_GATE_286: u16 = 24;
const TEST_SLOT_INT_GATE_386: u16 = 25;
const TEST_SLOT_TRAP_GATE_286: u16 = 26;
const TEST_SLOT_TRAP_GATE_386: u16 = 27;
const TEST_SLOT_RESERVED_TYPE_ZERO: u16 = 28;
const TEST_SLOT_TSS_386_AVAILABLE_DPL3: u16 = 29;

const EXTENDED_GDT_ENTRIES: u16 = 32;
const EXTENDED_GDT_LIMIT: u32 = (EXTENDED_GDT_ENTRIES as u32) * 8 - 1;

// LDT placed inside the shared data region, well clear of any test data
// the instruction encoding accesses via DS.
const LDT_BASE: u32 = SHARED_DATA_BASE + 0x4000;
const LDT_LIMIT: u32 = 0xFF;
const LDT_SLOT_RING0_CODE: u16 = 1;
const SELECTOR_LDT_RING0_CODE: u16 = (LDT_SLOT_RING0_CODE * 8) | 0x04;
const SELECTOR_LDT_DESCRIPTOR: u16 = TEST_SLOT_LDT_DESCRIPTOR * 8;

const SENTINEL_EAX: u32 = 0xDEAD_BEEF;
const SENTINEL_EBX: u32 = 0xCAFE_BABE;

fn extend_gdt(state: &mut cpu::I386State) {
    state.gdt_limit = EXTENDED_GDT_LIMIT as u16;
}

fn install_test_descriptors(bus: &mut TestBus) {
    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_CODE_RING0,
        RING0_CODE_BASE,
        0xFFFF,
        RIGHTS_RING0_CODE_READABLE_ACCESSED,
    );
    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_CODE_RING0_CONFORMING,
        RING0_CODE_BASE,
        0xFFFF,
        RIGHTS_RING0_CODE_CONFORMING_READABLE_ACCESSED,
    );
    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_CODE_RING3,
        RING3_CODE_BASE,
        0xFFFF,
        RIGHTS_RING3_CODE_READABLE_ACCESSED,
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
        TEST_SLOT_DATA_RING0,
        SHARED_DATA_BASE,
        0xFFFF,
        RIGHTS_RING0_DATA_WRITABLE_ACCESSED,
    );
    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_DATA_RING3,
        SHARED_DATA_BASE,
        0xFFFF,
        RIGHTS_RING3_DATA_WRITABLE_ACCESSED,
    );
    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_DATA_READ_ONLY,
        SHARED_DATA_BASE,
        0xFFFF,
        ACCESS_PRESENT | ACCESS_DPL_RING0 | ACCESS_DESCRIPTOR_CODE_OR_DATA,
    );
    write_segment_descriptor(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_DATA_RING0_GRANULAR,
        SHARED_DATA_BASE,
        0x000F_FFFF,
        RIGHTS_RING0_DATA_WRITABLE_ACCESSED,
        GRANULARITY_PAGE | GRANULARITY_BIG_OR_DEFAULT32,
    );
    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_TSS_286,
        SHARED_DATA_BASE,
        0x002B,
        ACCESS_PRESENT
            | ACCESS_DPL_RING0
            | ACCESS_DESCRIPTOR_SYSTEM
            | SYSTEM_TYPE_TSS_286_AVAILABLE,
    );
    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_TSS_386_AVAILABLE_DPL0,
        SHARED_DATA_BASE,
        0x0067,
        RIGHTS_TSS_386_AVAILABLE,
    );
    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_TSS_386_AVAILABLE_DPL3,
        SHARED_DATA_BASE,
        0x0067,
        ACCESS_PRESENT
            | ACCESS_DPL_RING3
            | ACCESS_DESCRIPTOR_SYSTEM
            | (RIGHTS_TSS_386_AVAILABLE & 0x0F),
    );
    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_LDT_DESCRIPTOR,
        LDT_BASE,
        LDT_LIMIT as u16,
        ACCESS_PRESENT | ACCESS_DPL_RING0 | ACCESS_DESCRIPTOR_SYSTEM | SYSTEM_TYPE_LDT,
    );

    write_gate_descriptor(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_CALL_GATE_286,
        0x0000_1234,
        SELECTOR_RING0_CODE,
        0,
        SYSTEM_TYPE_CALL_GATE_286,
        0,
    );
    write_gate_descriptor(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_CALL_GATE_386,
        0x0000_5678,
        SELECTOR_RING0_CODE,
        0,
        SYSTEM_TYPE_CALL_GATE_386,
        0,
    );
    write_gate_descriptor(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_TASK_GATE,
        0,
        TEST_SLOT_TSS_386_AVAILABLE_DPL0 * 8,
        0,
        SYSTEM_TYPE_TASK_GATE,
        0,
    );
    write_gate_descriptor(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_INT_GATE_286,
        0x0000_1000,
        SELECTOR_RING0_CODE,
        0,
        SYSTEM_TYPE_INTERRUPT_GATE_286,
        0,
    );
    write_gate_descriptor(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_INT_GATE_386,
        0x0000_2000,
        SELECTOR_RING0_CODE,
        0,
        SYSTEM_TYPE_INTERRUPT_GATE_386,
        0,
    );
    write_gate_descriptor(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_TRAP_GATE_286,
        0x0000_3000,
        SELECTOR_RING0_CODE,
        0,
        SYSTEM_TYPE_TRAP_GATE_286,
        0,
    );
    write_gate_descriptor(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_TRAP_GATE_386,
        0x0000_4000,
        SELECTOR_RING0_CODE,
        0,
        SYSTEM_TYPE_TRAP_GATE_386,
        0,
    );
    // Reserved type 0 (S=0, type field=0): not present, type 0 is invalid
    // for LAR and LSL per the manual matrix.
    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_RESERVED_TYPE_ZERO,
        0,
        0,
        ACCESS_PRESENT | ACCESS_DPL_RING0 | ACCESS_DESCRIPTOR_SYSTEM,
    );
}

fn install_local_descriptor_table(bus: &mut TestBus) {
    write_segment_descriptor_16bit(
        bus,
        LDT_BASE,
        LDT_SLOT_RING0_CODE,
        RING0_CODE_BASE,
        0xFFFF,
        RIGHTS_RING0_CODE_READABLE_ACCESSED,
    );
}

fn standard_protected_mode(bus: &mut TestBus) -> cpu::I386State {
    let mut state = setup_protected_mode_with_handlers(bus);
    install_test_descriptors(bus);
    install_local_descriptor_table(bus);
    extend_gdt(&mut state);
    state.ldtr = SELECTOR_LDT_DESCRIPTOR;
    state.ldtr_base = LDT_BASE;
    state.ldtr_limit = LDT_LIMIT;
    state.set_eax(SENTINEL_EAX);
    state.set_ebx(SENTINEL_EBX);
    state
}

fn install_real_mode_invalid_opcode_handler(bus: &mut TestBus, handler_cs: u16, handler_ip: u16) {
    bus.ram[6 * 4] = handler_ip as u8;
    bus.ram[6 * 4 + 1] = (handler_ip >> 8) as u8;
    bus.ram[6 * 4 + 2] = handler_cs as u8;
    bus.ram[6 * 4 + 3] = (handler_cs >> 8) as u8;
}

// LAR encoding: 0F 02 modrm. The destination register is the reg field;
// the source selector comes from the r/m operand. Tests use BX (rm=011) as
// the selector source and AX (reg=000) as the destination, giving modrm=C3.
const LAR_AX_BX: [u8; 3] = [0x0F, 0x02, 0xC3];
const LAR_EAX_EBX: [u8; 4] = [0x66, 0x0F, 0x02, 0xC3];
/// LAR ECX,EBX with a 32-bit operand.
const LAR_ECX_EBX: [u8; 4] = [0x66, 0x0F, 0x02, 0xCB];
// LAR AX, [DS:0x0040]: modrm=06 disp16=0x0040.
const LAR_AX_MEM_0040: [u8; 5] = [0x0F, 0x02, 0x06, 0x40, 0x00];
const LSL_AX_BX: [u8; 3] = [0x0F, 0x03, 0xC3];
const LSL_EAX_EBX: [u8; 4] = [0x66, 0x0F, 0x03, 0xC3];
const VERR_BX: [u8; 3] = [0x0F, 0x00, 0xE3]; // /4, rm=BX, mod=11
const VERW_BX: [u8; 3] = [0x0F, 0x00, 0xEB]; // /5, rm=BX, mod=11
const VERR_MEM_0040: [u8; 5] = [0x0F, 0x00, 0x26, 0x40, 0x00];
const VERW_MEM_0040: [u8; 5] = [0x0F, 0x00, 0x2E, 0x40, 0x00];

#[test]
fn lar_real_mode_raises_invalid_opcode_on_386() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let handler_cs: u16 = 0x2000;
    install_real_mode_invalid_opcode_handler(&mut bus, handler_cs, 0);

    place_code(&mut bus, cs, 0, &LAR_AX_BX);

    let mut state = cpu::I386State::default();
    state.set_cs(cs);
    state.set_ss(0x3000);
    state.set_esp(0x1000);
    cpu.load_state(&state);

    cpu.step(&mut bus);

    assert_eq!(cpu.cs(), handler_cs);
}

#[test]
fn lar_real_mode_raises_invalid_opcode_on_486() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let handler_cs: u16 = 0x2000;
    install_real_mode_invalid_opcode_handler(&mut bus, handler_cs, 0);

    place_code(&mut bus, cs, 0, &LAR_AX_BX);

    let mut state = cpu::I386State::default();
    state.set_cs(cs);
    state.set_ss(0x3000);
    state.set_esp(0x1000);
    cpu.load_state(&state);

    cpu.step(&mut bus);

    assert_eq!(cpu.cs(), handler_cs);
}

#[test]
fn lar_vm86_raises_invalid_opcode() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_vm86(&mut bus);
    write_interrupt_gate_386(
        &mut bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        6,
        HANDLER_INVALID_OPCODE_IP as u32,
        SELECTOR_RING0_CODE,
        0,
    );
    bus.ram[(RING0_CODE_BASE + HANDLER_INVALID_OPCODE_IP as u32) as usize] = 0xF4;
    state.gdt_limit = 5 * 8 - 1;
    cpu.load_state(&state);

    place_code(&mut bus, 0x1000, 0x0000, &LAR_AX_BX);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_INVALID_OPCODE_IP as u32 + 1);
}

#[test]
fn lar_protected_mode_null_selector_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx(0x0000_0000);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LAR_AX_BX);

    cpu.step(&mut bus);

    assert!(!cpu.halted(), "LAR with null selector must not fault");
    assert!(!cpu.state.flags.zf(), "LAR(null) sets ZF=0");
    assert_eq!(cpu.eax(), SENTINEL_EAX, "Destination preserved on failure");
}

#[test]
fn lar_protected_mode_selector_beyond_gdt_limit_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx(0x0000_FFF8);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LAR_AX_BX);

    cpu.step(&mut bus);

    assert!(!cpu.halted());
    assert!(!cpu.state.flags.zf());
    assert_eq!(cpu.eax(), SENTINEL_EAX);
}

#[test]
fn lar_protected_mode_selector_beyond_ldt_limit_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    let ldt_index_beyond_limit = ((LDT_LIMIT + 1) & 0xFFFF_FFF8) as u16;
    state.set_ebx((ldt_index_beyond_limit | 0x04) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LAR_AX_BX);

    cpu.step(&mut bus);

    assert!(!cpu.halted());
    assert!(!cpu.state.flags.zf());
    assert_eq!(cpu.eax(), SENTINEL_EAX);
}

#[test]
fn lar_protected_mode_ldt_selector_with_zero_ldtr_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.ldtr = 0;
    state.ldtr_base = 0;
    state.ldtr_limit = 0;
    state.set_ebx(0x0000_000C); // selector 0x000C: index=1 in LDT, RPL=0, TI=1
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LAR_AX_BX);

    cpu.step(&mut bus);

    assert!(!cpu.halted());
    assert!(!cpu.state.flags.zf());
}

#[test]
fn lar_protected_mode_ring0_data_returns_rights_byte() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(0);
    state.set_ebx((TEST_SLOT_DATA_RING0 * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LAR_AX_BX);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf(), "LAR on accessible data must set ZF=1");
    assert_eq!(
        cpu.eax() & 0xFFFF,
        (RIGHTS_RING0_DATA_WRITABLE_ACCESSED as u32) << 8,
        "AH gets the access-rights byte; AL is cleared"
    );
}

#[test]
fn lar_protected_mode_ring0_code_returns_rights_byte() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(0);
    state.set_ebx((TEST_SLOT_CODE_RING0 * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LAR_AX_BX);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf());
    assert_eq!(
        cpu.eax() & 0xFFFF,
        (RIGHTS_RING0_CODE_READABLE_ACCESSED as u32) << 8
    );
}

#[test]
fn lar_protected_mode_ring0_code_from_cpl3_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx((TEST_SLOT_CODE_RING0 * 8) as u32);
    promote_to_ring3(&mut state);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &LAR_AX_BX);

    cpu.step(&mut bus);

    assert!(!cpu.halted());
    assert!(
        !cpu.state.flags.zf(),
        "LAR(ring0 code) from CPL=3 fails the privilege check"
    );
}

#[test]
fn lar_protected_mode_conforming_code_from_cpl3_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(0);
    state.set_ebx((TEST_SLOT_CODE_RING0_CONFORMING * 8) as u32);
    promote_to_ring3(&mut state);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &LAR_AX_BX);

    cpu.step(&mut bus);

    assert!(!cpu.halted());
    assert!(
        cpu.state.flags.zf(),
        "Conforming code is accessible from any CPL"
    );
    assert_eq!(
        cpu.eax() & 0xFFFF,
        (RIGHTS_RING0_CODE_CONFORMING_READABLE_ACCESSED as u32) << 8
    );
}

#[test]
fn lar_protected_mode_ring3_data_from_cpl3_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(0);
    state.set_ebx(((TEST_SLOT_DATA_RING3 * 8) | 3) as u32);
    promote_to_ring3(&mut state);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &LAR_AX_BX);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf());
    assert_eq!(
        cpu.eax() & 0xFFFF,
        (RIGHTS_RING3_DATA_WRITABLE_ACCESSED as u32) << 8
    );
}

#[test]
fn lar_protected_mode_ring0_data_from_cpl0_with_rpl3_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx(((TEST_SLOT_DATA_RING0 * 8) | 3) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LAR_AX_BX);

    cpu.step(&mut bus);

    assert!(
        !cpu.state.flags.zf(),
        "RPL>DPL fails the LAR privilege check"
    );
}

#[test]
fn lar_protected_mode_tss_286_available_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(0);
    state.set_ebx((TEST_SLOT_TSS_286 * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LAR_AX_BX);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf());
    let expected_rights = ACCESS_PRESENT
        | ACCESS_DPL_RING0
        | ACCESS_DESCRIPTOR_SYSTEM
        | SYSTEM_TYPE_TSS_286_AVAILABLE;
    assert_eq!(cpu.eax() & 0xFFFF, (expected_rights as u32) << 8);
}

#[test]
fn lar_protected_mode_tss_386_available_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(0);
    state.set_ebx((TEST_SLOT_TSS_386_AVAILABLE_DPL0 * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LAR_AX_BX);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf());
    assert_eq!(cpu.eax() & 0xFFFF, (RIGHTS_TSS_386_AVAILABLE as u32) << 8);
}

#[test]
fn lar_protected_mode_tss_386_busy_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    write_segment_descriptor_16bit(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_TSS_386_AVAILABLE_DPL0,
        SHARED_DATA_BASE,
        0x0067,
        RIGHTS_TSS_386_BUSY,
    );
    state.set_eax(0);
    state.set_ebx((TEST_SLOT_TSS_386_AVAILABLE_DPL0 * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LAR_AX_BX);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf());
    assert_eq!(cpu.eax() & 0xFFFF, (RIGHTS_TSS_386_BUSY as u32) << 8);
}

#[test]
fn lar_protected_mode_ldt_descriptor_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(0);
    state.set_ebx(SELECTOR_LDT_DESCRIPTOR as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LAR_AX_BX);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf());
    let expected_rights =
        ACCESS_PRESENT | ACCESS_DPL_RING0 | ACCESS_DESCRIPTOR_SYSTEM | SYSTEM_TYPE_LDT;
    assert_eq!(cpu.eax() & 0xFFFF, (expected_rights as u32) << 8);
}

#[test]
fn lar_protected_mode_call_gate_286_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(0);
    state.set_ebx((TEST_SLOT_CALL_GATE_286 * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LAR_AX_BX);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf());
    let expected_rights =
        ACCESS_PRESENT | ACCESS_DPL_RING0 | ACCESS_DESCRIPTOR_SYSTEM | SYSTEM_TYPE_CALL_GATE_286;
    assert_eq!(cpu.eax() & 0xFFFF, (expected_rights as u32) << 8);
}

#[test]
fn lar_protected_mode_call_gate_386_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(0);
    state.set_ebx((TEST_SLOT_CALL_GATE_386 * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LAR_AX_BX);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf());
    let expected_rights =
        ACCESS_PRESENT | ACCESS_DPL_RING0 | ACCESS_DESCRIPTOR_SYSTEM | SYSTEM_TYPE_CALL_GATE_386;
    assert_eq!(cpu.eax() & 0xFFFF, (expected_rights as u32) << 8);
}

#[test]
fn lar_protected_mode_task_gate_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(0);
    state.set_ebx((TEST_SLOT_TASK_GATE * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LAR_AX_BX);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf());
    let expected_rights =
        ACCESS_PRESENT | ACCESS_DPL_RING0 | ACCESS_DESCRIPTOR_SYSTEM | SYSTEM_TYPE_TASK_GATE;
    assert_eq!(cpu.eax() & 0xFFFF, (expected_rights as u32) << 8);
}

#[test]
fn lar_protected_mode_interrupt_gate_286_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx((TEST_SLOT_INT_GATE_286 * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LAR_AX_BX);

    cpu.step(&mut bus);

    assert!(
        !cpu.state.flags.zf(),
        "Interrupt gate type 6 is invalid for LAR"
    );
}

#[test]
fn lar_protected_mode_interrupt_gate_386_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx((TEST_SLOT_INT_GATE_386 * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LAR_AX_BX);

    cpu.step(&mut bus);

    assert!(
        !cpu.state.flags.zf(),
        "Interrupt gate type 14 is invalid for LAR"
    );
}

#[test]
fn lar_protected_mode_trap_gate_286_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx((TEST_SLOT_TRAP_GATE_286 * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LAR_AX_BX);

    cpu.step(&mut bus);

    assert!(!cpu.state.flags.zf(), "Trap gate type 7 is invalid for LAR");
}

#[test]
fn lar_protected_mode_trap_gate_386_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx((TEST_SLOT_TRAP_GATE_386 * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LAR_AX_BX);

    cpu.step(&mut bus);

    assert!(
        !cpu.state.flags.zf(),
        "Trap gate type 15 is invalid for LAR"
    );
}

#[test]
fn lar_protected_mode_reserved_type_zero_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx((TEST_SLOT_RESERVED_TYPE_ZERO * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LAR_AX_BX);

    cpu.step(&mut bus);

    assert!(!cpu.state.flags.zf(), "Reserved type 0 is invalid for LAR");
}

#[test]
fn lar_protected_mode_dpl3_tss_from_cpl0_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx((TEST_SLOT_TSS_386_AVAILABLE_DPL3 * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LAR_AX_BX);

    cpu.step(&mut bus);

    assert!(
        cpu.state.flags.zf(),
        "DPL=3 system descriptor accessible from CPL=0"
    );
}

// No hardware data covers LAR on the 386. The 386 is assumed to return the limit bits
// 19:16 like the 486.
#[test]
fn lar_protected_mode_32bit_operand_returns_limit_bits_386_assumed_like_486() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(0);
    state.set_ebx((TEST_SLOT_DATA_RING0_GRANULAR * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LAR_EAX_EBX);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf());
    let expected_rights = RIGHTS_RING0_DATA_WRITABLE_ACCESSED;
    let expected_byte_6 = GRANULARITY_PAGE | GRANULARITY_BIG_OR_DEFAULT32 | 0x0F;
    let expected = ((expected_rights as u32) << 8) | ((expected_byte_6 as u32) << 16);
    assert_eq!(cpu.eax(), expected);
}

#[test]
fn lar_protected_mode_32bit_operand_returns_byte_granular_limit_bits_486() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    write_segment_descriptor(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_DATA_RING0_GRANULAR,
        SHARED_DATA_BASE,
        0x0002_571F,
        RIGHTS_RING3_DATA_WRITABLE_ACCESSED,
        0,
    );
    state.set_ecx(0);
    state.set_ebx((TEST_SLOT_DATA_RING0_GRANULAR * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LAR_ECX_EBX);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf());
    assert_eq!(
        cpu.ecx(),
        0x0002_0000 | ((RIGHTS_RING3_DATA_WRITABLE_ACCESSED as u32) << 8)
    );
}

#[test]
fn lar_protected_mode_via_ldt_selector_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(0);
    state.set_ebx(SELECTOR_LDT_RING0_CODE as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LAR_AX_BX);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf(), "LDT-table selector resolves via LDTR");
    assert_eq!(
        cpu.eax() & 0xFFFF,
        (RIGHTS_RING0_CODE_READABLE_ACCESSED as u32) << 8
    );
}

#[test]
fn lar_protected_mode_memory_operand_loads_from_ds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(0);
    write_segment_descriptor_16bit(&mut bus, SHARED_DATA_BASE, 0x0040 / 8, 0, 0, 0);
    super::setup::write_word_at(&mut bus, SHARED_DATA_BASE + 0x40, TEST_SLOT_DATA_RING0 * 8);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LAR_AX_MEM_0040);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf());
    assert_eq!(
        cpu.eax() & 0xFFFF,
        (RIGHTS_RING0_DATA_WRITABLE_ACCESSED as u32) << 8
    );
}

#[test]
fn lar_protected_mode_failure_does_not_modify_destination() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(0xAAAA_BBBB);
    state.set_ebx((TEST_SLOT_INT_GATE_386 * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LAR_AX_BX);

    cpu.step(&mut bus);

    assert!(!cpu.state.flags.zf());
    assert_eq!(
        cpu.eax(),
        0xAAAA_BBBB,
        "Destination register must be unchanged when ZF=0"
    );
}

#[test]
fn lsl_real_mode_raises_invalid_opcode_on_386() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let handler_cs: u16 = 0x2000;
    install_real_mode_invalid_opcode_handler(&mut bus, handler_cs, 0);

    place_code(&mut bus, cs, 0, &LSL_AX_BX);

    let mut state = cpu::I386State::default();
    state.set_cs(cs);
    state.set_ss(0x3000);
    state.set_esp(0x1000);
    cpu.load_state(&state);

    cpu.step(&mut bus);

    assert_eq!(cpu.cs(), handler_cs);
}

#[test]
fn lsl_real_mode_raises_invalid_opcode_on_486() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let handler_cs: u16 = 0x2000;
    install_real_mode_invalid_opcode_handler(&mut bus, handler_cs, 0);

    place_code(&mut bus, cs, 0, &LSL_AX_BX);

    let mut state = cpu::I386State::default();
    state.set_cs(cs);
    state.set_ss(0x3000);
    state.set_esp(0x1000);
    cpu.load_state(&state);

    cpu.step(&mut bus);

    assert_eq!(cpu.cs(), handler_cs);
}

#[test]
fn lsl_vm86_raises_invalid_opcode() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_vm86(&mut bus);
    write_interrupt_gate_386(
        &mut bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        6,
        HANDLER_INVALID_OPCODE_IP as u32,
        SELECTOR_RING0_CODE,
        0,
    );
    bus.ram[(RING0_CODE_BASE + HANDLER_INVALID_OPCODE_IP as u32) as usize] = 0xF4;
    state.gdt_limit = 5 * 8 - 1;
    cpu.load_state(&state);

    place_code(&mut bus, 0x1000, 0x0000, &LSL_AX_BX);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_INVALID_OPCODE_IP as u32 + 1);
}

#[test]
fn lsl_protected_mode_null_selector_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx(0);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LSL_AX_BX);

    cpu.step(&mut bus);

    assert!(!cpu.halted());
    assert!(!cpu.state.flags.zf());
    assert_eq!(cpu.eax(), SENTINEL_EAX);
}

#[test]
fn lsl_protected_mode_selector_beyond_gdt_limit_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx(0xFFF8);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LSL_AX_BX);

    cpu.step(&mut bus);

    assert!(!cpu.state.flags.zf());
}

#[test]
fn lsl_protected_mode_data_segment_returns_byte_limit() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(0);
    state.set_ebx((TEST_SLOT_DATA_RING0 * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LSL_AX_BX);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf());
    assert_eq!(cpu.eax() & 0xFFFF, 0xFFFF);
}

#[test]
fn lsl_protected_mode_code_segment_returns_byte_limit() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(0);
    state.set_ebx((TEST_SLOT_CODE_RING0 * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LSL_AX_BX);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf());
    assert_eq!(cpu.eax() & 0xFFFF, 0xFFFF);
}

#[test]
fn lsl_protected_mode_granular_segment_returns_byte_scaled_limit_via_32bit() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(0);
    state.set_ebx((TEST_SLOT_DATA_RING0_GRANULAR * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LSL_EAX_EBX);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf());
    let expected_byte_limit: u32 = (0x000F_FFFFu32 << 12) | 0xFFF;
    assert_eq!(cpu.eax(), expected_byte_limit);
}

#[test]
fn lsl_protected_mode_tss_386_available_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(0);
    state.set_ebx((TEST_SLOT_TSS_386_AVAILABLE_DPL0 * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LSL_AX_BX);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf());
    assert_eq!(cpu.eax() & 0xFFFF, 0x0067);
}

#[test]
fn lsl_protected_mode_tss_286_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(0);
    state.set_ebx((TEST_SLOT_TSS_286 * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LSL_AX_BX);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf());
    assert_eq!(cpu.eax() & 0xFFFF, 0x002B);
}

#[test]
fn lsl_protected_mode_ldt_descriptor_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(0);
    state.set_ebx(SELECTOR_LDT_DESCRIPTOR as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LSL_AX_BX);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf());
    assert_eq!(cpu.eax() & 0xFFFF, LDT_LIMIT as u16 as u32);
}

#[test]
fn lsl_protected_mode_call_gate_286_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx((TEST_SLOT_CALL_GATE_286 * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LSL_AX_BX);

    cpu.step(&mut bus);

    assert!(!cpu.state.flags.zf(), "Call gate is invalid for LSL");
}

#[test]
fn lsl_protected_mode_call_gate_386_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx((TEST_SLOT_CALL_GATE_386 * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LSL_AX_BX);

    cpu.step(&mut bus);

    assert!(!cpu.state.flags.zf());
}

#[test]
fn lsl_protected_mode_task_gate_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx((TEST_SLOT_TASK_GATE * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LSL_AX_BX);

    cpu.step(&mut bus);

    assert!(!cpu.state.flags.zf(), "Task gate is invalid for LSL");
}

#[test]
fn lsl_protected_mode_interrupt_gate_386_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx((TEST_SLOT_INT_GATE_386 * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LSL_AX_BX);

    cpu.step(&mut bus);

    assert!(!cpu.state.flags.zf());
}

#[test]
fn lsl_protected_mode_trap_gate_386_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx((TEST_SLOT_TRAP_GATE_386 * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LSL_AX_BX);

    cpu.step(&mut bus);

    assert!(!cpu.state.flags.zf());
}

#[test]
fn lsl_protected_mode_ring0_code_from_cpl3_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx((TEST_SLOT_CODE_RING0 * 8) as u32);
    promote_to_ring3(&mut state);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &LSL_AX_BX);

    cpu.step(&mut bus);

    assert!(!cpu.halted());
    assert!(!cpu.state.flags.zf());
}

#[test]
fn lsl_protected_mode_conforming_code_from_cpl3_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(0);
    state.set_ebx((TEST_SLOT_CODE_RING0_CONFORMING * 8) as u32);
    promote_to_ring3(&mut state);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &LSL_AX_BX);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf());
    assert_eq!(cpu.eax() & 0xFFFF, 0xFFFF);
}

#[test]
fn lsl_protected_mode_data_with_high_rpl_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx(((TEST_SLOT_DATA_RING0 * 8) | 3) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LSL_AX_BX);

    cpu.step(&mut bus);

    assert!(
        !cpu.state.flags.zf(),
        "RPL>DPL fails the LSL privilege check"
    );
}

#[test]
fn lsl_protected_mode_failure_does_not_modify_destination() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(0x1122_3344);
    state.set_ebx((TEST_SLOT_CALL_GATE_386 * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LSL_AX_BX);

    cpu.step(&mut bus);

    assert!(!cpu.state.flags.zf());
    assert_eq!(cpu.eax(), 0x1122_3344);
}

#[test]
fn lsl_protected_mode_via_ldt_selector_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(0);
    state.set_ebx(SELECTOR_LDT_RING0_CODE as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LSL_AX_BX);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf());
    assert_eq!(cpu.eax() & 0xFFFF, 0xFFFF);
}

#[test]
fn lsl_protected_mode_reserved_type_zero_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx((TEST_SLOT_RESERVED_TYPE_ZERO * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LSL_AX_BX);

    cpu.step(&mut bus);

    assert!(!cpu.state.flags.zf());
}

#[test]
fn verr_real_mode_raises_invalid_opcode_on_386() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let handler_cs: u16 = 0x2000;
    install_real_mode_invalid_opcode_handler(&mut bus, handler_cs, 0);

    place_code(&mut bus, cs, 0, &VERR_BX);

    let mut state = cpu::I386State::default();
    state.set_cs(cs);
    state.set_ss(0x3000);
    state.set_esp(0x1000);
    cpu.load_state(&state);

    cpu.step(&mut bus);

    assert_eq!(cpu.cs(), handler_cs);
}

#[test]
fn verr_real_mode_raises_invalid_opcode_on_486() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let handler_cs: u16 = 0x2000;
    install_real_mode_invalid_opcode_handler(&mut bus, handler_cs, 0);

    place_code(&mut bus, cs, 0, &VERR_BX);

    let mut state = cpu::I386State::default();
    state.set_cs(cs);
    state.set_ss(0x3000);
    state.set_esp(0x1000);
    cpu.load_state(&state);

    cpu.step(&mut bus);

    assert_eq!(cpu.cs(), handler_cs);
}

#[test]
fn verr_vm86_raises_invalid_opcode() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_vm86(&mut bus);
    write_interrupt_gate_386(
        &mut bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        6,
        HANDLER_INVALID_OPCODE_IP as u32,
        SELECTOR_RING0_CODE,
        0,
    );
    bus.ram[(RING0_CODE_BASE + HANDLER_INVALID_OPCODE_IP as u32) as usize] = 0xF4;
    state.gdt_limit = 5 * 8 - 1;
    cpu.load_state(&state);

    place_code(&mut bus, 0x1000, 0x0000, &VERR_BX);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_INVALID_OPCODE_IP as u32 + 1);
}

#[test]
fn verr_protected_mode_null_selector_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx(0);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &VERR_BX);

    cpu.step(&mut bus);

    assert!(!cpu.halted());
    assert!(!cpu.state.flags.zf());
}

#[test]
fn verr_protected_mode_selector_beyond_limit_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx(0xFFF8);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &VERR_BX);

    cpu.step(&mut bus);

    assert!(!cpu.state.flags.zf());
}

#[test]
fn verr_protected_mode_system_descriptor_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx((TEST_SLOT_TSS_386_AVAILABLE_DPL0 * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &VERR_BX);

    cpu.step(&mut bus);

    assert!(!cpu.state.flags.zf(), "VERR rejects all system descriptors");
}

#[test]
fn verr_protected_mode_data_segment_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx((TEST_SLOT_DATA_RING0 * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &VERR_BX);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf(), "Writable data is always readable");
}

#[test]
fn verr_protected_mode_read_only_data_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx((TEST_SLOT_DATA_READ_ONLY * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &VERR_BX);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf(), "Read-only data is readable for VERR");
}

#[test]
fn verr_protected_mode_readable_code_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx((TEST_SLOT_CODE_RING0 * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &VERR_BX);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf(), "Readable code passes VERR");
}

#[test]
fn verr_protected_mode_non_readable_code_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx((TEST_SLOT_CODE_NON_READABLE * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &VERR_BX);

    cpu.step(&mut bus);

    assert!(!cpu.state.flags.zf(), "Execute-only code fails VERR");
}

#[test]
fn verr_protected_mode_conforming_code_from_lower_privilege_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx((TEST_SLOT_CODE_RING0_CONFORMING * 8) as u32);
    promote_to_ring3(&mut state);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &VERR_BX);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf(), "Conforming code skips VERR DPL gate");
}

#[test]
fn verr_protected_mode_non_conforming_code_dpl_below_cpl_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx((TEST_SLOT_CODE_RING0 * 8) as u32);
    promote_to_ring3(&mut state);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &VERR_BX);

    cpu.step(&mut bus);

    assert!(!cpu.state.flags.zf());
}

#[test]
fn verr_protected_mode_data_dpl_below_cpl_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx((TEST_SLOT_DATA_RING0 * 8) as u32);
    promote_to_ring3(&mut state);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &VERR_BX);

    cpu.step(&mut bus);

    assert!(!cpu.state.flags.zf());
}

#[test]
fn verr_protected_mode_high_rpl_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx(((TEST_SLOT_DATA_RING0 * 8) | 3) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &VERR_BX);

    cpu.step(&mut bus);

    assert!(!cpu.state.flags.zf(), "RPL>DPL trips the privilege check");
}

#[test]
fn verr_protected_mode_memory_operand_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = standard_protected_mode(&mut bus);
    super::setup::write_word_at(&mut bus, SHARED_DATA_BASE + 0x40, TEST_SLOT_DATA_RING0 * 8);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &VERR_MEM_0040);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf());
}

#[test]
fn verw_real_mode_raises_invalid_opcode_on_386() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let handler_cs: u16 = 0x2000;
    install_real_mode_invalid_opcode_handler(&mut bus, handler_cs, 0);

    place_code(&mut bus, cs, 0, &VERW_BX);

    let mut state = cpu::I386State::default();
    state.set_cs(cs);
    state.set_ss(0x3000);
    state.set_esp(0x1000);
    cpu.load_state(&state);

    cpu.step(&mut bus);

    assert_eq!(cpu.cs(), handler_cs);
}

#[test]
fn verw_real_mode_raises_invalid_opcode_on_486() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let handler_cs: u16 = 0x2000;
    install_real_mode_invalid_opcode_handler(&mut bus, handler_cs, 0);

    place_code(&mut bus, cs, 0, &VERW_BX);

    let mut state = cpu::I386State::default();
    state.set_cs(cs);
    state.set_ss(0x3000);
    state.set_esp(0x1000);
    cpu.load_state(&state);

    cpu.step(&mut bus);

    assert_eq!(cpu.cs(), handler_cs);
}

#[test]
fn verw_vm86_raises_invalid_opcode() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_vm86(&mut bus);
    write_interrupt_gate_386(
        &mut bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        6,
        HANDLER_INVALID_OPCODE_IP as u32,
        SELECTOR_RING0_CODE,
        0,
    );
    bus.ram[(RING0_CODE_BASE + HANDLER_INVALID_OPCODE_IP as u32) as usize] = 0xF4;
    state.gdt_limit = 5 * 8 - 1;
    cpu.load_state(&state);

    place_code(&mut bus, 0x1000, 0x0000, &VERW_BX);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_INVALID_OPCODE_IP as u32 + 1);
}

#[test]
fn verw_protected_mode_null_selector_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx(0);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &VERW_BX);

    cpu.step(&mut bus);

    assert!(!cpu.state.flags.zf());
}

#[test]
fn verw_protected_mode_selector_beyond_limit_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx(0xFFF8);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &VERW_BX);

    cpu.step(&mut bus);

    assert!(!cpu.state.flags.zf());
}

#[test]
fn verw_protected_mode_system_descriptor_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx((TEST_SLOT_LDT_DESCRIPTOR * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &VERW_BX);

    cpu.step(&mut bus);

    assert!(!cpu.state.flags.zf(), "VERW rejects all system descriptors");
}

#[test]
fn verw_protected_mode_writable_data_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx((TEST_SLOT_DATA_RING0 * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &VERW_BX);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf());
}

#[test]
fn verw_protected_mode_read_only_data_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx((TEST_SLOT_DATA_READ_ONLY * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &VERW_BX);

    cpu.step(&mut bus);

    assert!(
        !cpu.state.flags.zf(),
        "Read-only data is not writable per VERW"
    );
}

#[test]
fn verw_protected_mode_readable_code_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx((TEST_SLOT_CODE_RING0 * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &VERW_BX);

    cpu.step(&mut bus);

    assert!(
        !cpu.state.flags.zf(),
        "Code segments are never writable for VERW"
    );
}

#[test]
fn verw_protected_mode_conforming_code_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx((TEST_SLOT_CODE_RING0_CONFORMING * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &VERW_BX);

    cpu.step(&mut bus);

    assert!(!cpu.state.flags.zf());
}

#[test]
fn verw_protected_mode_data_dpl_below_cpl_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx((TEST_SLOT_DATA_RING0 * 8) as u32);
    promote_to_ring3(&mut state);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &VERW_BX);

    cpu.step(&mut bus);

    assert!(!cpu.state.flags.zf());
}

#[test]
fn verw_protected_mode_data_dpl_equal_cpl_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx(((TEST_SLOT_DATA_RING3 * 8) | 3) as u32);
    promote_to_ring3(&mut state);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &VERW_BX);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf());
}

#[test]
fn verw_protected_mode_high_rpl_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx(((TEST_SLOT_DATA_RING0 * 8) | 3) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &VERW_BX);

    cpu.step(&mut bus);

    assert!(!cpu.state.flags.zf());
}

#[test]
fn verw_protected_mode_memory_operand_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = standard_protected_mode(&mut bus);
    super::setup::write_word_at(&mut bus, SHARED_DATA_BASE + 0x40, TEST_SLOT_DATA_RING0 * 8);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &VERW_MEM_0040);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf());
}

#[test]
fn verw_protected_mode_via_ldt_selector_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    write_segment_descriptor_16bit(
        &mut bus,
        LDT_BASE,
        2,
        SHARED_DATA_BASE,
        0xFFFF,
        RIGHTS_RING0_DATA_WRITABLE_ACCESSED,
    );
    state.set_ebx(((2u16 * 8) | 0x04) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &VERW_BX);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf());
}

#[test]
fn verr_protected_mode_data_dpl_equal_cpl_with_rpl_zero_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx(((TEST_SLOT_DATA_RING3 * 8) | 3) as u32);
    promote_to_ring3(&mut state);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &VERR_BX);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf());
}

#[test]
fn lar_protected_mode_at_cpl3_does_not_fault() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx((TEST_SLOT_DATA_RING3 * 8) as u32);
    promote_to_ring3(&mut state);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &LAR_AX_BX);

    cpu.step(&mut bus);

    assert!(!cpu.halted(), "LAR has no CPL gate in protected mode");
}

#[test]
fn lsl_protected_mode_at_cpl3_does_not_fault() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx((TEST_SLOT_DATA_RING3 * 8) as u32);
    promote_to_ring3(&mut state);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &LSL_AX_BX);

    cpu.step(&mut bus);

    assert!(!cpu.halted(), "LSL has no CPL gate in protected mode");
}

#[test]
fn lar_protected_mode_does_not_modify_destination_when_invalid_type() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(0xCC11_DD22);
    state.set_ebx((TEST_SLOT_RESERVED_TYPE_ZERO * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LAR_AX_BX);

    cpu.step(&mut bus);

    assert_eq!(cpu.eax(), 0xCC11_DD22);
}

#[test]
fn lar_protected_mode_does_not_modify_destination_when_privilege_fails() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(0x9999_8888);
    state.set_ebx(((TEST_SLOT_DATA_RING0 * 8) | 3) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LAR_AX_BX);

    cpu.step(&mut bus);

    assert!(!cpu.state.flags.zf());
    assert_eq!(cpu.eax(), 0x9999_8888);
}

#[test]
fn lsl_protected_mode_via_ldt_with_high_rpl_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx(((LDT_SLOT_RING0_CODE * 8) | 0x04 | 0x03) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LSL_AX_BX);

    cpu.step(&mut bus);

    assert!(
        !cpu.state.flags.zf(),
        "RPL>DPL on LDT-resolved selector fails LSL privilege"
    );
}

#[test]
fn lar_does_not_set_accessed_bit_in_descriptor() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    write_segment_descriptor_16bit(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_DATA_RING0,
        SHARED_DATA_BASE,
        0xFFFF,
        RIGHTS_RING0_DATA_WRITABLE_ACCESSED & !0x01,
    );
    state.set_ebx((TEST_SLOT_DATA_RING0 * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LAR_AX_BX);

    cpu.step(&mut bus);

    let descriptor_address = GLOBAL_DESCRIPTOR_TABLE_BASE + (TEST_SLOT_DATA_RING0 as u32) * 8;
    let rights_after = bus.ram[(descriptor_address + 5) as usize];
    assert_eq!(
        rights_after & 0x01,
        0,
        "LAR is a non-modifying inspection instruction"
    );
}

#[test]
fn lsl_protected_mode_call_gate_286_with_table_indicator_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    write_gate_descriptor(
        &mut bus,
        LDT_BASE,
        3,
        0,
        SELECTOR_RING0_CODE,
        0,
        SYSTEM_TYPE_CALL_GATE_286,
        0,
    );
    state.set_ebx(((3u16 * 8) | 0x04) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LSL_AX_BX);

    cpu.step(&mut bus);

    assert!(!cpu.state.flags.zf());
}

#[test]
fn lar_486_protected_mode_matches_386_semantics() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(0);
    state.set_ebx((TEST_SLOT_DATA_RING0 * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LAR_AX_BX);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf());
    assert_eq!(
        cpu.eax() & 0xFFFF,
        (RIGHTS_RING0_DATA_WRITABLE_ACCESSED as u32) << 8
    );
}

#[test]
fn lsl_486_protected_mode_matches_386_semantics() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(0);
    state.set_ebx((TEST_SLOT_DATA_RING0 * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LSL_AX_BX);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf());
    assert_eq!(cpu.eax() & 0xFFFF, 0xFFFF);
}

#[test]
fn verr_486_protected_mode_matches_386_semantics() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx((TEST_SLOT_CODE_RING0 * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &VERR_BX);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf());
}

#[test]
fn verw_486_protected_mode_matches_386_semantics() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_ebx((TEST_SLOT_DATA_RING0 * 8) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &VERW_BX);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf());
}

#[test]
fn verr_protected_mode_does_not_modify_memory() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = standard_protected_mode(&mut bus);
    let selector = TEST_SLOT_DATA_RING0 * 8;
    super::setup::write_word_at(&mut bus, SHARED_DATA_BASE + 0x40, selector);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &VERR_MEM_0040);

    cpu.step(&mut bus);

    assert_eq!(read_word_at(&bus, SHARED_DATA_BASE + 0x40), selector);
}

#[test]
fn lar_protected_mode_via_ldt_with_zero_ldtr_after_ldt_load_clears_zf() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.ldtr = 0;
    state.ldtr_base = 0;
    state.ldtr_limit = 0;
    state.set_ebx(SELECTOR_LDT_RING0_CODE as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LAR_AX_BX);

    cpu.step(&mut bus);

    assert!(
        !cpu.state.flags.zf(),
        "LDT-resolved selectors fail when LDTR has zero limit"
    );
}

#[test]
fn lar_protected_mode_high_rpl_with_dpl_3_data_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(0);
    state.set_ebx(((TEST_SLOT_DATA_RING3 * 8) | 3) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LAR_AX_BX);

    cpu.step(&mut bus);

    assert!(
        cpu.state.flags.zf(),
        "DPL=3 admits selectors with any RPL up to 3"
    );
    assert_eq!(
        cpu.eax() & 0xFFFF,
        (RIGHTS_RING3_DATA_WRITABLE_ACCESSED as u32) << 8
    );
}

#[test]
fn lsl_protected_mode_high_rpl_with_dpl_3_data_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    state.set_eax(0);
    state.set_ebx(((TEST_SLOT_DATA_RING3 * 8) | 3) as u32);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &LSL_AX_BX);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf());
    assert_eq!(cpu.eax() & 0xFFFF, 0xFFFF);
}

/// The EFLAGS.ZF bit.
const EFLAGS_ZERO: u32 = 1 << 6;

/// Runs `code` with BX selecting a descriptor on a not-present LDT page and EFLAGS.ZF set,
/// and checks that the #PF of the descriptor read ends the instruction with EAX and ZF
/// unchanged, both live and in the pushed EFLAGS image.
fn assert_descriptor_page_fault_keeps_state(code: &[u8]) {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode(&mut bus);
    super::setup::enable_identity_paging(&mut bus, &mut state);
    super::setup::set_identity_page_flags(&mut bus, LDT_BASE, 0);
    state.set_ebx(SELECTOR_LDT_RING0_CODE as u32);
    state.set_eflags(state.eflags() | EFLAGS_ZERO);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, code);

    cpu.step(&mut bus);

    assert_eq!(cpu.ip(), u32::from(super::setup::HANDLER_PAGE_FAULT_IP));
    assert_eq!(cpu.state.cr2, LDT_BASE + 8);
    assert_eq!(cpu.eax(), SENTINEL_EAX);
    assert!(cpu.state.flags.zf(), "live ZF after the #PF delivery");
    let stack = cpu.state.seg_bases[cpu::SegReg32::SS as usize].wrapping_add(cpu.esp());
    let pushed_eflags = super::setup::read_dword_at(&bus, stack + 12);
    assert_ne!(
        pushed_eflags & EFLAGS_ZERO,
        0,
        "ZF in the pushed EFLAGS image"
    );
}

#[test]
fn lar_descriptor_page_fault_keeps_destination_and_zf() {
    assert_descriptor_page_fault_keeps_state(&LAR_EAX_EBX);
}

#[test]
fn lsl_descriptor_page_fault_keeps_destination_and_zf() {
    assert_descriptor_page_fault_keeps_state(&LSL_EAX_EBX);
}

#[test]
fn verr_descriptor_page_fault_keeps_zf() {
    assert_descriptor_page_fault_keeps_state(&VERR_BX);
}

#[test]
fn verw_descriptor_page_fault_keeps_zf() {
    assert_descriptor_page_fault_keeps_state(&VERW_BX);
}
