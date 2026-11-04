//! Far CALL / JMP edge cases derived from the 80486 PRM Chapters 6 and 26.
//!
//! Covers call gate parameter-count truncation, 16-bit vs 32-bit parameter
//! copy across privilege change, ring-3 -> ring-0 stack-overflow on the new
//! SS, gate selector LDT/GDT and TI-mismatch handling, target type confusion,
//! conforming/non-conforming target DPL rules, JMP-through-call-gate
//! restrictions, and JMP/CALL to a busy TSS without a task gate.

use common::Cpu as _;

use super::setup::{
    ACCESS_DESCRIPTOR_CODE_OR_DATA, ACCESS_DESCRIPTOR_SYSTEM, ACCESS_DPL_RING0, ACCESS_DPL_RING3,
    ACCESS_PRESENT, ACCESS_TYPE_CODE, ACCESS_TYPE_CODE_CONFORMING, ACCESS_TYPE_CODE_READABLE,
    ACCESS_TYPE_DATA_WRITABLE, GLOBAL_DESCRIPTOR_TABLE_BASE, HANDLER_GENERAL_PROTECTION_IP,
    HANDLER_SEGMENT_NOT_PRESENT_IP, RING0_CODE_BASE, RING0_STACK_BASE, RING3_CODE_BASE,
    SELECTOR_PRIMARY_TSS, SELECTOR_RING0_CODE, SELECTOR_RING0_STACK, SHARED_DATA_BASE,
    SYSTEM_TYPE_LDT, SYSTEM_TYPE_TSS_286_BUSY, TASK_STATE_SEGMENT_BASE,
    TASK_STATE_SEGMENT_SECONDARY_BASE, TSS_MINIMUM_LIMIT, TSS_OFFSET_ESP0, TestBus, make_cpu_486,
    place_at, promote_to_ring3, read_dword_at, read_word_at, setup_protected_mode_with_handlers,
    write_call_gate_286, write_call_gate_386, write_dword_at, write_segment_descriptor_16bit,
    write_word_at,
};

const TEST_SLOT_CALL_GATE_286_DPL3: u16 = 10;
const TEST_SLOT_CALL_GATE_386_DPL3: u16 = 11;
const TEST_SLOT_LDT_DESCRIPTOR: u16 = 12;
const TEST_SLOT_DATA_AS_TARGET: u16 = 13;
const TEST_SLOT_NOT_PRESENT_TARGET: u16 = 14;
const TEST_SLOT_CONFORMING_RING0_CODE: u16 = 15;
const TEST_SLOT_RING0_NON_CONFORMING_DPL2: u16 = 16;
const TEST_SLOT_TSS_286_BUSY: u16 = 17;

const TEST_SLOT_GATE_NOT_PRESENT: u16 = 18;
const EXTENDED_GDT_SLOT_COUNT: u16 = 19;

const SELECTOR_CALL_GATE_286_DPL3: u16 = (TEST_SLOT_CALL_GATE_286_DPL3 << 3) | 3;
const SELECTOR_CALL_GATE_386_DPL3: u16 = (TEST_SLOT_CALL_GATE_386_DPL3 << 3) | 3;
const SELECTOR_LDT_DESCRIPTOR: u16 = TEST_SLOT_LDT_DESCRIPTOR << 3;
const SELECTOR_DATA_AS_TARGET: u16 = TEST_SLOT_DATA_AS_TARGET << 3;
const SELECTOR_NOT_PRESENT_TARGET: u16 = TEST_SLOT_NOT_PRESENT_TARGET << 3;
const SELECTOR_CONFORMING_RING0_CODE: u16 = TEST_SLOT_CONFORMING_RING0_CODE << 3;
const SELECTOR_RING0_NON_CONFORMING_DPL2: u16 = TEST_SLOT_RING0_NON_CONFORMING_DPL2 << 3;
const SELECTOR_TSS_286_BUSY: u16 = TEST_SLOT_TSS_286_BUSY << 3;
const SELECTOR_GATE_NOT_PRESENT: u16 = (TEST_SLOT_GATE_NOT_PRESENT << 3) | 3;

const LDT_BASE: u32 = SHARED_DATA_BASE + 0x4000;
const LDT_LIMIT: u32 = 0x00FF;
const LDT_SLOT_CALL_GATE_286_DPL3: u16 = 1;
const SELECTOR_LDT_CALL_GATE_286_DPL3: u16 = (LDT_SLOT_CALL_GATE_286_DPL3 << 3) | 3 | 4;

const GATE_TARGET_IP: u16 = 0x0100;

fn extend_gdt_for_tests(state: &mut cpu::I386State) {
    state.gdt_limit = EXTENDED_GDT_SLOT_COUNT * 8 - 1;
}

fn install_test_descriptors(bus: &mut TestBus) {
    // Data segment masquerading as a CALL target: gate target points here
    // to verify type-confusion -> #GP.
    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_DATA_AS_TARGET,
        SHARED_DATA_BASE,
        0xFFFF,
        ACCESS_PRESENT
            | ACCESS_DPL_RING0
            | ACCESS_DESCRIPTOR_CODE_OR_DATA
            | ACCESS_TYPE_DATA_WRITABLE,
    );

    // Code descriptor with present=0 used as gate target.
    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_NOT_PRESENT_TARGET,
        RING0_CODE_BASE,
        0xFFFF,
        ACCESS_DPL_RING0
            | ACCESS_DESCRIPTOR_CODE_OR_DATA
            | ACCESS_TYPE_CODE
            | ACCESS_TYPE_CODE_READABLE,
    );

    // Conforming ring-0 code: target DPL <= CPL is allowed without PL change.
    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_CONFORMING_RING0_CODE,
        RING0_CODE_BASE,
        0xFFFF,
        ACCESS_PRESENT
            | ACCESS_DPL_RING0
            | ACCESS_DESCRIPTOR_CODE_OR_DATA
            | ACCESS_TYPE_CODE
            | ACCESS_TYPE_CODE_CONFORMING
            | ACCESS_TYPE_CODE_READABLE,
    );

    // Non-conforming DPL=2 code segment used to exercise mismatched target
    // DPL during CALL through a DPL=3 gate.
    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_RING0_NON_CONFORMING_DPL2,
        RING0_CODE_BASE,
        0xFFFF,
        ACCESS_PRESENT
            | (2 << 5)
            | ACCESS_DESCRIPTOR_CODE_OR_DATA
            | ACCESS_TYPE_CODE
            | ACCESS_TYPE_CODE_READABLE,
    );

    // 286 busy TSS for "JMP/CALL to busy TSS" tests.
    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_TSS_286_BUSY,
        TASK_STATE_SEGMENT_SECONDARY_BASE,
        TSS_MINIMUM_LIMIT as u16,
        ACCESS_PRESENT | ACCESS_DPL_RING0 | ACCESS_DESCRIPTOR_SYSTEM | SYSTEM_TYPE_TSS_286_BUSY,
    );

    // LDT descriptor in GDT.
    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_LDT_DESCRIPTOR,
        LDT_BASE,
        LDT_LIMIT as u16,
        ACCESS_PRESENT | ACCESS_DPL_RING0 | ACCESS_DESCRIPTOR_SYSTEM | SYSTEM_TYPE_LDT,
    );

    // 286 call gate at DPL=3 with target = ring-0 code, in LDT.
    super::setup::write_gate_descriptor(
        bus,
        LDT_BASE,
        LDT_SLOT_CALL_GATE_286_DPL3,
        GATE_TARGET_IP as u32,
        SELECTOR_RING0_CODE,
        0,
        super::setup::SYSTEM_TYPE_CALL_GATE_286,
        3,
    );

    // Gate descriptor with present=0 (286 type used).
    let gate_addr = GLOBAL_DESCRIPTOR_TABLE_BASE + (TEST_SLOT_GATE_NOT_PRESENT as u32) * 8;
    bus.ram[gate_addr as usize] = GATE_TARGET_IP as u8;
    bus.ram[gate_addr as usize + 1] = (GATE_TARGET_IP >> 8) as u8;
    bus.ram[gate_addr as usize + 2] = SELECTOR_RING0_CODE as u8;
    bus.ram[gate_addr as usize + 3] = (SELECTOR_RING0_CODE >> 8) as u8;
    bus.ram[gate_addr as usize + 4] = 0;
    bus.ram[gate_addr as usize + 5] =
        ACCESS_DPL_RING3 | ACCESS_DESCRIPTOR_SYSTEM | super::setup::SYSTEM_TYPE_CALL_GATE_286;
    bus.ram[gate_addr as usize + 6] = 0;
    bus.ram[gate_addr as usize + 7] = 0;
}

fn install_handler_byte_at(bus: &mut TestBus, ip: u16) {
    bus.ram[(RING0_CODE_BASE + ip as u32) as usize] = 0xF4;
}

fn install_call_gate_target_handler(bus: &mut TestBus) {
    install_handler_byte_at(bus, GATE_TARGET_IP);
}

fn standard_protected_mode_at_ring3(bus: &mut TestBus) -> cpu::I386State {
    let mut state = setup_protected_mode_with_handlers(bus);
    install_test_descriptors(bus);
    install_call_gate_target_handler(bus);
    extend_gdt_for_tests(&mut state);
    state.ldtr = SELECTOR_LDT_DESCRIPTOR;
    state.ldtr_base = LDT_BASE;
    state.ldtr_limit = LDT_LIMIT;
    promote_to_ring3(&mut state);
    state
}

fn standard_protected_mode_at_ring0(bus: &mut TestBus) -> cpu::I386State {
    let mut state = setup_protected_mode_with_handlers(bus);
    install_test_descriptors(bus);
    install_call_gate_target_handler(bus);
    extend_gdt_for_tests(&mut state);
    state.ldtr = SELECTOR_LDT_DESCRIPTOR;
    state.ldtr_base = LDT_BASE;
    state.ldtr_limit = LDT_LIMIT;
    state
}

fn place_call_far(bus: &mut TestBus, code_base: u32, target_offset: u16, target_selector: u16) {
    place_at(
        bus,
        code_base,
        &[
            0x9A,
            target_offset as u8,
            (target_offset >> 8) as u8,
            target_selector as u8,
            (target_selector >> 8) as u8,
        ],
    );
}

fn place_call_far_32bit(bus: &mut TestBus, code_base: u32, target_selector: u16) {
    place_at(
        bus,
        code_base,
        &[
            0x66,
            0x9A,
            0x00,
            0x00,
            0x00,
            0x00,
            target_selector as u8,
            (target_selector >> 8) as u8,
        ],
    );
}

fn place_jmp_far(bus: &mut TestBus, code_base: u32, target_offset: u16, target_selector: u16) {
    place_at(
        bus,
        code_base,
        &[
            0xEA,
            target_offset as u8,
            (target_offset >> 8) as u8,
            target_selector as u8,
            (target_selector >> 8) as u8,
        ],
    );
}

#[test]
fn call_through_286_call_gate_same_privilege_pushes_return_frame() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode_at_ring0(&mut bus);
    write_call_gate_286(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_CALL_GATE_286_DPL3,
        GATE_TARGET_IP as u32,
        SELECTOR_RING0_CODE,
        0,
        0,
    );
    state.set_esp(0x1000);
    cpu.load_state(&state);

    place_call_far(
        &mut bus,
        RING0_CODE_BASE,
        0,
        TEST_SLOT_CALL_GATE_286_DPL3 << 3,
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), GATE_TARGET_IP as u32 + 1);
    let new_sp = cpu.state.esp();
    assert_eq!(read_word_at(&bus, RING0_STACK_BASE + new_sp), 5);
    assert_eq!(
        read_word_at(&bus, RING0_STACK_BASE + new_sp + 2),
        SELECTOR_RING0_CODE
    );
}

#[test]
fn call_through_386_call_gate_same_privilege_pushes_dword_return_frame() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode_at_ring0(&mut bus);
    write_call_gate_386(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_CALL_GATE_386_DPL3,
        GATE_TARGET_IP as u32,
        SELECTOR_RING0_CODE,
        0,
        0,
    );
    state.set_esp(0x1000);
    cpu.load_state(&state);

    place_call_far_32bit(&mut bus, RING0_CODE_BASE, TEST_SLOT_CALL_GATE_386_DPL3 << 3);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), GATE_TARGET_IP as u32 + 1);
    let new_sp = cpu.state.esp();
    assert_eq!(read_dword_at(&bus, RING0_STACK_BASE + new_sp), 8);
    assert_eq!(
        read_dword_at(&bus, RING0_STACK_BASE + new_sp + 4),
        SELECTOR_RING0_CODE as u32
    );
}

#[test]
fn call_through_286_call_gate_ring3_to_ring0_pushes_outer_stack_frame() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode_at_ring3(&mut bus);
    write_call_gate_286(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_CALL_GATE_286_DPL3,
        GATE_TARGET_IP as u32,
        SELECTOR_RING0_CODE,
        0,
        3,
    );
    state.set_esp(0xF000);
    cpu.load_state(&state);

    place_call_far(&mut bus, RING3_CODE_BASE, 0, SELECTOR_CALL_GATE_286_DPL3);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), GATE_TARGET_IP as u32 + 1);
    assert_eq!(cpu.cs() & 3, 0, "new CPL must be the gate target's DPL");
    let new_sp = cpu.state.esp();
    let new_ss_base = cpu.state.seg_bases[cpu::SegReg32::SS as usize];
    // Stack layout (low to high): [old_eip][old_cs][saved_esp][saved_ss]
    // for a 286 gate with zero parameters.
    assert_eq!(read_word_at(&bus, new_ss_base + new_sp), 5);
    let pushed_cs = read_word_at(&bus, new_ss_base + new_sp + 2);
    assert_eq!(
        pushed_cs & 0xFFFC,
        super::setup::SELECTOR_RING3_CODE & 0xFFFC,
        "pushed CS must be the caller's CS"
    );
    assert_eq!(read_word_at(&bus, new_ss_base + new_sp + 4), 0xF000);
    let saved_ss = read_word_at(&bus, new_ss_base + new_sp + 6);
    assert_eq!(
        saved_ss & 0xFFFC,
        super::setup::SELECTOR_RING3_STACK & 0xFFFC
    );
}

#[test]
fn call_through_386_call_gate_ring3_to_ring0_pushes_dword_outer_stack_frame() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode_at_ring3(&mut bus);
    write_call_gate_386(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_CALL_GATE_386_DPL3,
        GATE_TARGET_IP as u32,
        SELECTOR_RING0_CODE,
        0,
        3,
    );
    state.set_esp(0xF000);
    cpu.load_state(&state);

    place_call_far_32bit(&mut bus, RING3_CODE_BASE, SELECTOR_CALL_GATE_386_DPL3);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), GATE_TARGET_IP as u32 + 1);
    assert_eq!(cpu.cs() & 3, 0);
    let new_sp = cpu.state.esp();
    let new_ss_base = cpu.state.seg_bases[cpu::SegReg32::SS as usize];
    assert_eq!(read_dword_at(&bus, new_ss_base + new_sp), 8);
    let pushed_cs = read_dword_at(&bus, new_ss_base + new_sp + 4);
    assert_eq!(
        pushed_cs & 0xFFFC,
        super::setup::SELECTOR_RING3_CODE as u32 & 0xFFFC,
        "pushed CS must be the caller's CS"
    );
    assert_eq!(read_dword_at(&bus, new_ss_base + new_sp + 8), 0xF000);
}

// 80486 PRM 6.5.1: the old SS is pushed zero-extended to 32 bits. The CS slot of the same
// frame is written as a dword as well.
#[test]
fn call_through_386_call_gate_ring3_to_ring0_zero_extends_selector_slots() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode_at_ring3(&mut bus);
    write_call_gate_386(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_CALL_GATE_386_DPL3,
        GATE_TARGET_IP as u32,
        SELECTOR_RING0_CODE,
        0,
        3,
    );
    let inner_stack_top = super::setup::RING0_STACK_BASE + 0xFFF0;
    for address in inner_stack_top - 16..inner_stack_top {
        bus.ram[address as usize] = 0xA5;
    }
    state.set_esp(0xF000);
    cpu.load_state(&state);

    place_call_far_32bit(&mut bus, RING3_CODE_BASE, SELECTOR_CALL_GATE_386_DPL3);

    cpu.step(&mut bus);

    assert_eq!(
        read_dword_at(&bus, inner_stack_top - 4),
        super::setup::SELECTOR_RING3_STACK as u32
    );
    assert_eq!(read_dword_at(&bus, inner_stack_top - 8), 0xF000);
    assert_eq!(
        read_dword_at(&bus, inner_stack_top - 12),
        super::setup::SELECTOR_RING3_CODE as u32
    );
}

#[test]
fn call_through_286_call_gate_ring3_to_ring0_copies_word_parameters() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode_at_ring3(&mut bus);
    let parameter_count: u8 = 4;
    write_call_gate_286(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_CALL_GATE_286_DPL3,
        GATE_TARGET_IP as u32,
        SELECTOR_RING0_CODE,
        parameter_count,
        3,
    );
    state.set_esp(0xF000);
    cpu.load_state(&state);

    let outer_ss_base = cpu.state.seg_bases[cpu::SegReg32::SS as usize];
    write_word_at(&mut bus, outer_ss_base + 0xF000, 0x1111);
    write_word_at(&mut bus, outer_ss_base + 0xF002, 0x2222);
    write_word_at(&mut bus, outer_ss_base + 0xF004, 0x3333);
    write_word_at(&mut bus, outer_ss_base + 0xF006, 0x4444);

    place_call_far(&mut bus, RING3_CODE_BASE, 0, SELECTOR_CALL_GATE_286_DPL3);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    let new_sp = cpu.state.esp();
    let new_ss_base = cpu.state.seg_bases[cpu::SegReg32::SS as usize];
    // Stack layout: [old_eip][old_cs][param0][param1][param2][param3][saved_esp][saved_ss].
    assert_eq!(read_word_at(&bus, new_ss_base + new_sp), 5);
    assert_eq!(read_word_at(&bus, new_ss_base + new_sp + 4), 0x1111);
    assert_eq!(read_word_at(&bus, new_ss_base + new_sp + 6), 0x2222);
    assert_eq!(read_word_at(&bus, new_ss_base + new_sp + 8), 0x3333);
    assert_eq!(read_word_at(&bus, new_ss_base + new_sp + 10), 0x4444);
}

#[test]
fn call_through_386_call_gate_ring3_to_ring0_copies_dword_parameters() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode_at_ring3(&mut bus);
    let parameter_count: u8 = 3;
    write_call_gate_386(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_CALL_GATE_386_DPL3,
        GATE_TARGET_IP as u32,
        SELECTOR_RING0_CODE,
        parameter_count,
        3,
    );
    state.set_esp(0xF000);
    cpu.load_state(&state);

    let outer_ss_base = cpu.state.seg_bases[cpu::SegReg32::SS as usize];
    write_dword_at(&mut bus, outer_ss_base + 0xF000, 0x1111_1111);
    write_dword_at(&mut bus, outer_ss_base + 0xF004, 0x2222_2222);
    write_dword_at(&mut bus, outer_ss_base + 0xF008, 0x3333_3333);

    place_call_far_32bit(&mut bus, RING3_CODE_BASE, SELECTOR_CALL_GATE_386_DPL3);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    let new_sp = cpu.state.esp();
    let new_ss_base = cpu.state.seg_bases[cpu::SegReg32::SS as usize];
    // Stack layout: [old_eip][old_cs][param0..param2][saved_esp][saved_ss].
    assert_eq!(read_dword_at(&bus, new_ss_base + new_sp), 8);
    assert_eq!(read_dword_at(&bus, new_ss_base + new_sp + 8), 0x1111_1111);
    assert_eq!(read_dword_at(&bus, new_ss_base + new_sp + 12), 0x2222_2222);
    assert_eq!(read_dword_at(&bus, new_ss_base + new_sp + 16), 0x3333_3333);
}

#[test]
fn call_gate_parameter_count_truncated_to_lower_five_bits_286() {
    // parameter_count=33 -> 33 & 0x1F = 1 -> exactly one word param copied.
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode_at_ring3(&mut bus);
    write_call_gate_286(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_CALL_GATE_286_DPL3,
        GATE_TARGET_IP as u32,
        SELECTOR_RING0_CODE,
        33,
        3,
    );
    state.set_esp(0xF000);
    cpu.load_state(&state);

    let outer_ss_base = cpu.state.seg_bases[cpu::SegReg32::SS as usize];
    write_word_at(&mut bus, outer_ss_base + 0xF000, 0xCAFE);
    write_word_at(&mut bus, outer_ss_base + 0xF002, 0xBABE);

    place_call_far(&mut bus, RING3_CODE_BASE, 0, SELECTOR_CALL_GATE_286_DPL3);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    let new_sp = cpu.state.esp();
    let new_ss_base = cpu.state.seg_bases[cpu::SegReg32::SS as usize];
    // Exactly one parameter word was copied at +4 (immediately above the
    // 4-byte 286 return frame); the +6 slot holds saved_esp, not param 1.
    assert_eq!(read_word_at(&bus, new_ss_base + new_sp + 4), 0xCAFE);
    assert_eq!(
        read_word_at(&bus, new_ss_base + new_sp + 6),
        0xF000,
        "+6 holds saved_esp when only one param was copied"
    );
}

#[test]
fn call_gate_parameter_count_truncated_to_lower_five_bits_386() {
    // parameter_count=32 -> 32 & 0x1F = 0 -> no parameters copied.
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode_at_ring3(&mut bus);
    write_call_gate_386(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_CALL_GATE_386_DPL3,
        GATE_TARGET_IP as u32,
        SELECTOR_RING0_CODE,
        32,
        3,
    );
    state.set_esp(0xF000);
    cpu.load_state(&state);

    let outer_ss_base = cpu.state.seg_bases[cpu::SegReg32::SS as usize];
    write_dword_at(&mut bus, outer_ss_base + 0xF000, 0xDEAD_BEEF);

    place_call_far_32bit(&mut bus, RING3_CODE_BASE, SELECTOR_CALL_GATE_386_DPL3);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    let new_sp = cpu.state.esp();
    let new_ss_base = cpu.state.seg_bases[cpu::SegReg32::SS as usize];
    // 386 return frame is 8 bytes; with zero copied params the dword at +8
    // is saved_esp, not a parameter. The original ESP was 0xF000.
    assert_eq!(read_dword_at(&bus, new_ss_base + new_sp + 8), 0xF000);
}

#[test]
fn call_gate_parameter_count_max_31_286_copies_31_words() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode_at_ring3(&mut bus);
    write_call_gate_286(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_CALL_GATE_286_DPL3,
        GATE_TARGET_IP as u32,
        SELECTOR_RING0_CODE,
        63, // 63 & 0x1F = 31
        3,
    );
    state.set_esp(0xF000);
    cpu.load_state(&state);

    let outer_ss_base = cpu.state.seg_bases[cpu::SegReg32::SS as usize];
    for index in 0..31u32 {
        write_word_at(
            &mut bus,
            outer_ss_base + 0xF000 + index * 2,
            (0x1000 + index) as u16,
        );
    }

    place_call_far(&mut bus, RING3_CODE_BASE, 0, SELECTOR_CALL_GATE_286_DPL3);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    let new_sp = cpu.state.esp();
    let new_ss_base = cpu.state.seg_bases[cpu::SegReg32::SS as usize];
    // Params live at [esp+4..esp+4+31*2].
    for index in 0..31u32 {
        assert_eq!(
            read_word_at(&bus, new_ss_base + new_sp + 4 + index * 2),
            (0x1000 + index) as u16,
            "parameter {index} mismatch"
        );
    }
}

#[test]
fn call_through_286_gate_ring3_with_tiny_inner_esp_wraps_within_the_stack_limit() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode_at_ring3(&mut bus);
    write_dword_at(&mut bus, TASK_STATE_SEGMENT_BASE + TSS_OFFSET_ESP0, 0x0004);
    write_call_gate_286(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_CALL_GATE_286_DPL3,
        GATE_TARGET_IP as u32,
        SELECTOR_RING0_CODE,
        0,
        3,
    );
    state.set_esp(0xF000);
    cpu.load_state(&state);

    place_call_far(&mut bus, RING3_CODE_BASE, 0, SELECTOR_CALL_GATE_286_DPL3);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    // 80486 PRM chapter 26, CALL: the 16-bit stack offsets wrap within the 64 KiB limit,
    // so the new stack has room and the call reaches the gate target.
    assert!(cpu.halted());
    assert_eq!(cpu.ip(), GATE_TARGET_IP as u32 + 1);
    assert_eq!(cpu.ss(), SELECTOR_RING0_STACK);
    assert_eq!(cpu.esp(), 0xFFFC);
}

#[test]
fn call_through_386_gate_ring3_with_tiny_inner_esp_wraps_within_the_stack_limit() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode_at_ring3(&mut bus);
    write_dword_at(&mut bus, TASK_STATE_SEGMENT_BASE + TSS_OFFSET_ESP0, 0x0008);
    write_call_gate_386(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_CALL_GATE_386_DPL3,
        GATE_TARGET_IP as u32,
        SELECTOR_RING0_CODE,
        0,
        3,
    );
    state.set_esp(0xF000);
    cpu.load_state(&state);

    place_call_far_32bit(&mut bus, RING3_CODE_BASE, SELECTOR_CALL_GATE_386_DPL3);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    // 80486 PRM chapter 26, CALL: the 16-bit stack offsets wrap within the 64 KiB limit,
    // so the new stack has room and the call reaches the gate target.
    assert!(cpu.halted());
    assert_eq!(cpu.ip(), GATE_TARGET_IP as u32 + 1);
    assert_eq!(cpu.ss(), SELECTOR_RING0_STACK);
    assert_eq!(cpu.esp(), 0xFFF8);
}

#[test]
fn call_through_286_gate_with_31_parameters_wraps_within_the_stack_limit() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode_at_ring3(&mut bus);
    write_dword_at(&mut bus, TASK_STATE_SEGMENT_BASE + TSS_OFFSET_ESP0, 0x0010);
    write_call_gate_286(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_CALL_GATE_286_DPL3,
        GATE_TARGET_IP as u32,
        SELECTOR_RING0_CODE,
        31,
        3,
    );
    state.set_esp(0xF000);
    cpu.load_state(&state);

    place_call_far(&mut bus, RING3_CODE_BASE, 0, SELECTOR_CALL_GATE_286_DPL3);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    // 80486 PRM chapter 26, CALL: the 16-bit stack offsets wrap within the 64 KiB limit,
    // so the new stack has room and the call reaches the gate target.
    assert!(cpu.halted());
    assert_eq!(cpu.ip(), GATE_TARGET_IP as u32 + 1);
    assert_eq!(cpu.ss(), SELECTOR_RING0_STACK);
    assert_eq!(cpu.esp(), 0xFFCA);
}

#[test]
fn call_through_call_gate_in_ldt_succeeds() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode_at_ring3(&mut bus);
    state.set_esp(0xF000);
    cpu.load_state(&state);

    place_call_far(
        &mut bus,
        RING3_CODE_BASE,
        0,
        SELECTOR_LDT_CALL_GATE_286_DPL3,
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), GATE_TARGET_IP as u32 + 1);
}

#[test]
fn call_through_null_gate_selector_raises_general_protection() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let state = standard_protected_mode_at_ring3(&mut bus);
    cpu.load_state(&state);

    place_call_far(&mut bus, RING3_CODE_BASE, 0, 0);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn call_through_gate_selector_beyond_gdt_limit_raises_general_protection() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let state = standard_protected_mode_at_ring3(&mut bus);
    cpu.load_state(&state);

    let beyond_limit = (EXTENDED_GDT_SLOT_COUNT << 3) | 3;
    place_call_far(&mut bus, RING3_CODE_BASE, 0, beyond_limit);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn call_through_call_gate_with_dpl_below_cpl_raises_general_protection() {
    // Gate DPL=0; current CPL=3 fails max(CPL,RPL) <= DPL.
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode_at_ring3(&mut bus);
    write_call_gate_286(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_CALL_GATE_286_DPL3,
        GATE_TARGET_IP as u32,
        SELECTOR_RING0_CODE,
        0,
        0,
    );
    state.set_esp(0xF000);
    cpu.load_state(&state);

    place_call_far(&mut bus, RING3_CODE_BASE, 0, SELECTOR_CALL_GATE_286_DPL3);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn call_through_call_gate_with_data_target_raises_general_protection() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode_at_ring3(&mut bus);
    write_call_gate_286(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_CALL_GATE_286_DPL3,
        GATE_TARGET_IP as u32,
        SELECTOR_DATA_AS_TARGET,
        0,
        3,
    );
    state.set_esp(0xF000);
    cpu.load_state(&state);

    place_call_far(&mut bus, RING3_CODE_BASE, 0, SELECTOR_CALL_GATE_286_DPL3);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn call_through_call_gate_with_system_target_raises_general_protection() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode_at_ring3(&mut bus);
    // Use the LDT slot itself as the gate target -- a system descriptor.
    write_call_gate_286(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_CALL_GATE_286_DPL3,
        GATE_TARGET_IP as u32,
        SELECTOR_LDT_DESCRIPTOR,
        0,
        3,
    );
    state.set_esp(0xF000);
    cpu.load_state(&state);

    place_call_far(&mut bus, RING3_CODE_BASE, 0, SELECTOR_CALL_GATE_286_DPL3);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn call_through_call_gate_with_not_present_target_raises_segment_not_present() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode_at_ring3(&mut bus);
    write_call_gate_286(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_CALL_GATE_286_DPL3,
        GATE_TARGET_IP as u32,
        SELECTOR_NOT_PRESENT_TARGET,
        0,
        3,
    );
    state.set_esp(0xF000);
    cpu.load_state(&state);

    place_call_far(&mut bus, RING3_CODE_BASE, 0, SELECTOR_CALL_GATE_286_DPL3);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_SEGMENT_NOT_PRESENT_IP as u32 + 1);
}

#[test]
fn call_through_not_present_gate_raises_segment_not_present() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode_at_ring3(&mut bus);
    state.set_esp(0xF000);
    cpu.load_state(&state);

    place_call_far(&mut bus, RING3_CODE_BASE, 0, SELECTOR_GATE_NOT_PRESENT);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_SEGMENT_NOT_PRESENT_IP as u32 + 1);
}

#[test]
fn call_through_call_gate_to_conforming_code_keeps_cpl() {
    // Conforming target with DPL=0: from CPL=3 to a conforming DPL=0
    // segment, the executing CPL stays at 3 (no inter-privilege transition,
    // no stack switch).
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode_at_ring3(&mut bus);
    write_call_gate_286(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_CALL_GATE_286_DPL3,
        GATE_TARGET_IP as u32,
        SELECTOR_CONFORMING_RING0_CODE,
        0,
        3,
    );
    state.set_esp(0xF000);
    cpu.load_state(&state);
    let saved_ss = cpu.ss();

    place_call_far(&mut bus, RING3_CODE_BASE, 0, SELECTOR_CALL_GATE_286_DPL3);

    cpu.step(&mut bus);

    assert!(!cpu.halted());
    assert_eq!(cpu.ip(), GATE_TARGET_IP as u32);
    assert_eq!(cpu.cs() & 3, 3, "conforming target keeps CPL");
    assert_eq!(cpu.ss(), saved_ss, "no stack switch on conforming target");
}

#[test]
fn jmp_through_286_call_gate_same_privilege_does_not_push_return_frame() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode_at_ring0(&mut bus);
    write_call_gate_286(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_CALL_GATE_286_DPL3,
        GATE_TARGET_IP as u32,
        SELECTOR_RING0_CODE,
        0,
        0,
    );
    state.set_esp(0x1000);
    cpu.load_state(&state);
    let initial_sp = cpu.state.esp();

    place_jmp_far(
        &mut bus,
        RING0_CODE_BASE,
        0,
        TEST_SLOT_CALL_GATE_286_DPL3 << 3,
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), GATE_TARGET_IP as u32 + 1);
    assert_eq!(
        cpu.state.esp(),
        initial_sp,
        "JMP through gate must not push a return frame"
    );
}

#[test]
fn jmp_through_386_call_gate_same_privilege_does_not_push_return_frame() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode_at_ring0(&mut bus);
    write_call_gate_386(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_CALL_GATE_386_DPL3,
        GATE_TARGET_IP as u32,
        SELECTOR_RING0_CODE,
        0,
        0,
    );
    state.set_esp(0x1000);
    cpu.load_state(&state);
    let initial_sp = cpu.state.esp();

    place_at(
        &mut bus,
        RING0_CODE_BASE,
        &[
            0x66,
            0xEA,
            0x00,
            0x00,
            0x00,
            0x00,
            (TEST_SLOT_CALL_GATE_386_DPL3 << 3) as u8,
            ((TEST_SLOT_CALL_GATE_386_DPL3 << 3) >> 8) as u8,
        ],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), GATE_TARGET_IP as u32 + 1);
    assert_eq!(cpu.state.esp(), initial_sp);
}

#[test]
fn jmp_through_286_call_gate_inter_privilege_raises_general_protection() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode_at_ring3(&mut bus);
    write_call_gate_286(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_CALL_GATE_286_DPL3,
        GATE_TARGET_IP as u32,
        SELECTOR_RING0_CODE,
        0,
        3,
    );
    state.set_esp(0xF000);
    cpu.load_state(&state);

    place_jmp_far(&mut bus, RING3_CODE_BASE, 0, SELECTOR_CALL_GATE_286_DPL3);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn jmp_through_386_call_gate_inter_privilege_raises_general_protection() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode_at_ring3(&mut bus);
    write_call_gate_386(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_CALL_GATE_386_DPL3,
        GATE_TARGET_IP as u32,
        SELECTOR_RING0_CODE,
        0,
        3,
    );
    state.set_esp(0xF000);
    cpu.load_state(&state);

    place_at(
        &mut bus,
        RING3_CODE_BASE,
        &[
            0x66,
            0xEA,
            0x00,
            0x00,
            0x00,
            0x00,
            SELECTOR_CALL_GATE_386_DPL3 as u8,
            (SELECTOR_CALL_GATE_386_DPL3 >> 8) as u8,
        ],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn jmp_far_to_busy_386_tss_raises_general_protection() {
    // SELECTOR_PRIMARY_TSS is the busy TSS at GDT slot 7. JMP/CALL FAR to a
    // busy TSS without a task gate is an error per 80486 PRM Chapter 7.
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let state = standard_protected_mode_at_ring0(&mut bus);
    cpu.load_state(&state);

    place_jmp_far(&mut bus, RING0_CODE_BASE, 0, SELECTOR_PRIMARY_TSS);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn jmp_far_to_busy_286_tss_raises_general_protection() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let state = standard_protected_mode_at_ring0(&mut bus);
    cpu.load_state(&state);

    place_jmp_far(&mut bus, RING0_CODE_BASE, 0, SELECTOR_TSS_286_BUSY);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn call_far_to_busy_386_tss_raises_general_protection() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let state = standard_protected_mode_at_ring0(&mut bus);
    cpu.load_state(&state);

    place_call_far(&mut bus, RING0_CODE_BASE, 0, SELECTOR_PRIMARY_TSS);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn call_far_to_busy_286_tss_raises_general_protection() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let state = standard_protected_mode_at_ring0(&mut bus);
    cpu.load_state(&state);

    place_call_far(&mut bus, RING0_CODE_BASE, 0, SELECTOR_TSS_286_BUSY);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn call_far_to_non_conforming_code_with_higher_dpl_raises_general_protection() {
    // Direct far CALL (no gate) to a non-conforming code segment with
    // DPL=2 from CPL=0: per 80486 PRM Chapter 6, RPL <= CPL is required
    // and DPL must equal CPL for non-conforming segments -> #GP(sel).
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let state = standard_protected_mode_at_ring0(&mut bus);
    cpu.load_state(&state);

    place_call_far(
        &mut bus,
        RING0_CODE_BASE,
        0,
        SELECTOR_RING0_NON_CONFORMING_DPL2,
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn call_through_call_gate_to_non_conforming_target_with_dpl_above_target_dpl_succeeds() {
    // Conforming target CPL stays at original; non-conforming target
    // requires target_dpl <= cpl and falls through to inter-priv if <.
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode_at_ring3(&mut bus);
    write_call_gate_286(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_CALL_GATE_286_DPL3,
        GATE_TARGET_IP as u32,
        SELECTOR_RING0_CODE,
        0,
        3,
    );
    state.set_esp(0xF000);
    cpu.load_state(&state);

    place_call_far(&mut bus, RING3_CODE_BASE, 0, SELECTOR_CALL_GATE_286_DPL3);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), GATE_TARGET_IP as u32 + 1);
    assert_eq!(
        cpu.cs() & 3,
        0,
        "non-conforming target switches CPL to its DPL"
    );
}

#[test]
fn jmp_far_through_call_gate_to_conforming_code_succeeds() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode_at_ring3(&mut bus);
    write_call_gate_286(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_CALL_GATE_286_DPL3,
        GATE_TARGET_IP as u32,
        SELECTOR_CONFORMING_RING0_CODE,
        0,
        3,
    );
    state.set_esp(0xF000);
    cpu.load_state(&state);

    place_jmp_far(&mut bus, RING3_CODE_BASE, 0, SELECTOR_CALL_GATE_286_DPL3);

    cpu.step(&mut bus);

    assert!(!cpu.halted());
    assert_eq!(cpu.ip(), GATE_TARGET_IP as u32);
    assert_eq!(cpu.cs() & 3, 3, "JMP to conforming keeps CPL");
}

#[test]
fn call_through_286_gate_zero_parameters_pushes_only_return_frame() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = standard_protected_mode_at_ring3(&mut bus);
    write_call_gate_286(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        TEST_SLOT_CALL_GATE_286_DPL3,
        GATE_TARGET_IP as u32,
        SELECTOR_RING0_CODE,
        0,
        3,
    );
    state.set_esp(0xF000);
    cpu.load_state(&state);

    place_call_far(&mut bus, RING3_CODE_BASE, 0, SELECTOR_CALL_GATE_286_DPL3);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    let new_sp = cpu.state.esp();
    let new_ss_base = cpu.state.seg_bases[cpu::SegReg32::SS as usize];
    // Return frame is exactly 8 bytes (IP, CS, SP, SS) at [esp..esp+8].
    assert_eq!(
        new_sp,
        0xFFF0 - 8,
        "286 gate ring3->ring0 return frame is 8 bytes"
    );
    assert_eq!(read_word_at(&bus, new_ss_base + new_sp), 5);
}

#[test]
fn call_far_direct_to_ring0_code_from_ring3_raises_general_protection() {
    // Without a gate, far CALL from ring 3 to a DPL=0 non-conforming code
    // segment raises #GP(sel).
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let state = standard_protected_mode_at_ring3(&mut bus);
    cpu.load_state(&state);

    place_call_far(&mut bus, RING3_CODE_BASE, 0, SELECTOR_RING0_CODE);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}
