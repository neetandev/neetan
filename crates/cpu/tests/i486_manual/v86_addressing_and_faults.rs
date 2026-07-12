//! Virtual-8086 mode addressing/limit edge cases and instruction quirks.
//!
//! 80486 PRM Chapter 23. The V86 environment from `setup_vm86_with_iopl`
//! has CS=0x1000 (base 0x0001_0000), SS=0x2000 (base 0x0002_0000), all
//! with limit 0xFFFF, and IOPL set per call. The chapter calls out:
//!   - Sequential execution past 0xFFFF -> #GP (23.6 difference 8).
//!   - DIV/IDIV exception saved CS:IP points to faulting insn (23.6 #2).
//!   - LOCK prefix restricted whitelist -> #UD when misused (23.7.2).
//!   - Instruction longer than 15 bytes -> #GP (23.6 difference 6).
//!   - BOUND raises #BR when index out of range.
//!   - INT n always uses IDT (not 8086-style table at linear 0).
//!   - Interrupt gate target must be non-conforming code at PL 0 (23.3.2).
//!   - LIDT/LGDT/LMSW/MOV CRn/CLTS/HLT/INVD/WBINVD/INVLPG -> #GP (23.7.1).

use common::Cpu as _;

use super::setup::{
    HANDLER_BOUND_RANGE_IP, HANDLER_GENERAL_PROTECTION_IP, HANDLER_INVALID_OPCODE_IP,
    INTERRUPT_DESCRIPTOR_TABLE_BASE, RING0_CODE_BASE, SELECTOR_RING0_CODE, TestBus, make_cpu_486,
    place_at, read_dword_at, read_word_at, setup_vm86, setup_vm86_with_iopl,
    write_interrupt_gate_386,
};

const HLT_OPCODE: u8 = 0xF4;
const NOP_OPCODE: u8 = 0x90;
const SS_OVERRIDE_PREFIX: u8 = 0x36;

const VM86_CS_BASE: u32 = 0x0001_0000;

fn install_v86_invalid_opcode_handler(bus: &mut TestBus) {
    write_interrupt_gate_386(
        bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        6,
        HANDLER_INVALID_OPCODE_IP as u32,
        SELECTOR_RING0_CODE,
        0,
    );
    bus.ram[(RING0_CODE_BASE + HANDLER_INVALID_OPCODE_IP as u32) as usize] = HLT_OPCODE;
}

fn install_v86_general_protection_handler(bus: &mut TestBus) {
    write_interrupt_gate_386(
        bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        13,
        HANDLER_GENERAL_PROTECTION_IP as u32,
        SELECTOR_RING0_CODE,
        0,
    );
    bus.ram[(RING0_CODE_BASE + HANDLER_GENERAL_PROTECTION_IP as u32) as usize] = HLT_OPCODE;
}

fn install_v86_divide_error_handler(bus: &mut TestBus) {
    write_interrupt_gate_386(
        bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        0,
        super::setup::HANDLER_DIVIDE_ERROR_IP as u32,
        SELECTOR_RING0_CODE,
        0,
    );
    bus.ram[(RING0_CODE_BASE + super::setup::HANDLER_DIVIDE_ERROR_IP as u32) as usize] = HLT_OPCODE;
}

fn install_v86_bound_range_handler(bus: &mut TestBus) {
    write_interrupt_gate_386(
        bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        5,
        HANDLER_BOUND_RANGE_IP as u32,
        SELECTOR_RING0_CODE,
        0,
    );
    bus.ram[(RING0_CODE_BASE + HANDLER_BOUND_RANGE_IP as u32) as usize] = HLT_OPCODE;
}

fn place_in_v86_code(bus: &mut TestBus, offset: u16, code: &[u8]) {
    place_at(bus, VM86_CS_BASE + offset as u32, code);
}

fn assert_at_handler(cpu: &cpu::I386<{ cpu::CPU_MODEL_486_DX }>, handler_ip: u16) {
    assert!(cpu.halted(), "expected handler HLT");
    assert_eq!(cpu.ip(), handler_ip as u32 + 1);
}

fn read_saved_eip_from_pl0_stack(bus: &TestBus, cpu: &cpu::I386<{ cpu::CPU_MODEL_486_DX }>) -> u32 {
    let stack_linear =
        cpu.state.seg_bases[cpu::SegReg32::SS as usize] + cpu.state.regs.dword(cpu::DwordReg::ESP);
    read_dword_at(bus, stack_linear)
}

#[test]
fn v86_sequential_execution_past_offset_ffff_raises_general_protection() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_vm86(&mut bus);
    install_v86_general_protection_handler(&mut bus);
    state.gdt_limit = 5 * 8 - 1;
    state.ip = 0xFFFF;
    cpu.load_state(&state);

    place_in_v86_code(&mut bus, 0xFFFF, &[NOP_OPCODE]);
    place_in_v86_code(&mut bus, 0x0000, &[HLT_OPCODE]);

    cpu.step(&mut bus); // execute NOP at 0xFFFF
    cpu.step(&mut bus); // fetch at 0x10000 raises #GP and dispatches handler
    cpu.step(&mut bus); // execute the handler's HLT

    assert_at_handler(&cpu, HANDLER_GENERAL_PROTECTION_IP);
}

#[test]
fn v86_div_byte_by_zero_csip_points_to_div_instruction() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_vm86(&mut bus);
    install_v86_divide_error_handler(&mut bus);
    state.gdt_limit = 5 * 8 - 1;
    state.set_eax(0x0010);
    state.set_ebx(0x0000);
    cpu.load_state(&state);

    let div_offset: u16 = 0x0040;
    place_in_v86_code(&mut bus, div_offset, &[0xF6, 0xF3]);
    cpu.state.ip = div_offset;

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_at_handler(&cpu, super::setup::HANDLER_DIVIDE_ERROR_IP);

    let saved_eip = read_saved_eip_from_pl0_stack(&bus, &cpu);
    assert_eq!(
        saved_eip, div_offset as u32,
        "saved CS:IP must point to the faulting DIV"
    );
}

#[test]
fn v86_idiv_word_overflow_csip_points_to_idiv_instruction() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_vm86(&mut bus);
    install_v86_divide_error_handler(&mut bus);
    state.gdt_limit = 5 * 8 - 1;
    // DX:AX = 0x0001_0000, BX = 1: signed quotient 65536, does not fit in i16.
    state.set_edx(0x0001);
    state.set_eax(0x0000);
    state.set_ebx(0x0001);
    cpu.load_state(&state);

    let idiv_offset: u16 = 0x0040;
    place_in_v86_code(&mut bus, idiv_offset, &[0xF7, 0xFB]);
    cpu.state.ip = idiv_offset;

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_at_handler(&cpu, super::setup::HANDLER_DIVIDE_ERROR_IP);

    let saved_eip = read_saved_eip_from_pl0_stack(&bus, &cpu);
    assert_eq!(saved_eip, idiv_offset as u32);
}

#[test]
fn v86_lock_prefix_on_mov_register_destination_raises_invalid_opcode() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_vm86(&mut bus);
    install_v86_invalid_opcode_handler(&mut bus);
    state.gdt_limit = 5 * 8 - 1;
    cpu.load_state(&state);

    place_in_v86_code(&mut bus, 0x0000, &[0xF0, 0x89, 0xD8]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_at_handler(&cpu, HANDLER_INVALID_OPCODE_IP);
}

#[test]
fn v86_lock_prefix_on_disallowed_opcode_raises_invalid_opcode() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_vm86(&mut bus);
    install_v86_invalid_opcode_handler(&mut bus);
    state.gdt_limit = 5 * 8 - 1;
    cpu.load_state(&state);

    // LOCK NOP.
    place_in_v86_code(&mut bus, 0x0000, &[0xF0, NOP_OPCODE]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_at_handler(&cpu, HANDLER_INVALID_OPCODE_IP);
}

#[test]
fn v86_instruction_exceeding_15_bytes_raises_general_protection() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_vm86(&mut bus);
    install_v86_general_protection_handler(&mut bus);
    state.gdt_limit = 5 * 8 - 1;
    cpu.load_state(&state);

    let mut encoding = vec![SS_OVERRIDE_PREFIX; 15];
    encoding.push(NOP_OPCODE);
    place_in_v86_code(&mut bus, 0x0000, &encoding);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_at_handler(&cpu, HANDLER_GENERAL_PROTECTION_IP);
}

#[test]
fn v86_bound_with_index_below_lower_bound_raises_bound_range() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_vm86(&mut bus);
    install_v86_bound_range_handler(&mut bus);
    state.gdt_limit = 5 * 8 - 1;
    state.set_eax(0xFFF0u32 as i16 as i32 as u32); // -16 sign-extended to 32 bits
    cpu.load_state(&state);

    // Bounds at DS:[0x40] = lower=0x0010, DS:[0x42] = upper=0x0050.
    let bounds_offset: u16 = 0x0040;
    let bounds_address = 0x0004_0000u32 + bounds_offset as u32; // V86 DS base = 0x40000
    bus.ram[bounds_address as usize] = 0x10;
    bus.ram[bounds_address as usize + 1] = 0x00;
    bus.ram[bounds_address as usize + 2] = 0x50;
    bus.ram[bounds_address as usize + 3] = 0x00;

    // 62 06 disp16 = BOUND AX, [disp16].
    place_in_v86_code(
        &mut bus,
        0x0000,
        &[0x62, 0x06, bounds_offset as u8, (bounds_offset >> 8) as u8],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_at_handler(&cpu, HANDLER_BOUND_RANGE_IP);
}

#[test]
fn v86_bound_with_index_above_upper_bound_raises_bound_range() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_vm86(&mut bus);
    install_v86_bound_range_handler(&mut bus);
    state.gdt_limit = 5 * 8 - 1;
    state.set_eax(0x00FF);
    cpu.load_state(&state);

    let bounds_offset: u16 = 0x0040;
    let bounds_address = 0x0004_0000u32 + bounds_offset as u32;
    bus.ram[bounds_address as usize] = 0x10;
    bus.ram[bounds_address as usize + 1] = 0x00;
    bus.ram[bounds_address as usize + 2] = 0x50;
    bus.ram[bounds_address as usize + 3] = 0x00;

    place_in_v86_code(
        &mut bus,
        0x0000,
        &[0x62, 0x06, bounds_offset as u8, (bounds_offset >> 8) as u8],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_at_handler(&cpu, HANDLER_BOUND_RANGE_IP);
}

#[test]
fn v86_int_n_dispatches_via_idt_not_real_mode_ivt_at_linear_zero() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    // Plant a fake IVT entry at linear 0:0x42*4 to detect any incorrect
    // real-mode-style dispatch. The bytes installed here would route the
    // dispatch into a HLT at a recognisable address; if the V86 INT n
    // dispatched via this table the assertion below would fail.
    let ivt_offset_address = (0x42u32 * 4) as usize;
    bus.ram[ivt_offset_address] = 0x00;
    bus.ram[ivt_offset_address + 1] = 0x00;
    bus.ram[ivt_offset_address + 2] = 0x00;
    bus.ram[ivt_offset_address + 3] = 0x80; // segment 0x8000 -> linear 0x80000

    let trap_address = 0x0008_0000u32;
    bus.ram[trap_address as usize] = HLT_OPCODE;

    let state = setup_vm86_with_iopl(&mut bus, 3);
    cpu.load_state(&state);

    place_in_v86_code(&mut bus, 0x0000, &[0xCD, 0x42]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_ne!(
        cpu.cs(),
        0x8000,
        "INT n in V86 must NOT use the linear-0 IVT"
    );
    // Setup_vm86 installs the IDT entry handler at HANDLER_VM86_IP in
    // RING0_CODE_BASE (selector SELECTOR_RING0_CODE).
    assert_eq!(cpu.cs(), SELECTOR_RING0_CODE);
}

#[test]
fn v86_idt_entry_with_conforming_code_segment_target_raises_general_protection() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_vm86_with_iopl(&mut bus, 3);
    install_v86_general_protection_handler(&mut bus);

    // Add a conforming ring-0 code descriptor to the GDT (slot 5).
    super::setup::write_segment_descriptor_16bit(
        &mut bus,
        super::setup::GLOBAL_DESCRIPTOR_TABLE_BASE,
        5,
        RING0_CODE_BASE,
        0xFFFF,
        super::setup::RIGHTS_RING0_CODE_CONFORMING_READABLE_ACCESSED,
    );
    state.gdt_limit = 6 * 8 - 1;

    // Install INT 0x50 gate pointing at the conforming segment (selector 0x28).
    write_interrupt_gate_386(
        &mut bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        0x50,
        HANDLER_GENERAL_PROTECTION_IP as u32,
        0x0028,
        3,
    );
    cpu.load_state(&state);

    place_in_v86_code(&mut bus, 0x0000, &[0xCD, 0x50]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_at_handler(&cpu, HANDLER_GENERAL_PROTECTION_IP);
}

#[test]
fn v86_idt_entry_with_target_dpl_nonzero_raises_general_protection() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_vm86_with_iopl(&mut bus, 3);
    install_v86_general_protection_handler(&mut bus);

    // Add a non-conforming ring-3 code descriptor (DPL=3) at slot 5.
    super::setup::write_segment_descriptor_16bit(
        &mut bus,
        super::setup::GLOBAL_DESCRIPTOR_TABLE_BASE,
        5,
        RING0_CODE_BASE,
        0xFFFF,
        super::setup::RIGHTS_RING3_CODE_READABLE_ACCESSED,
    );
    state.gdt_limit = 6 * 8 - 1;

    // INT 0x50 -> selector 0x002B (slot 5 RPL=3, ring-3 code).
    write_interrupt_gate_386(
        &mut bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        0x50,
        HANDLER_GENERAL_PROTECTION_IP as u32,
        0x002B,
        3,
    );
    cpu.load_state(&state);

    place_in_v86_code(&mut bus, 0x0000, &[0xCD, 0x50]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_at_handler(&cpu, HANDLER_GENERAL_PROTECTION_IP);
}

#[test]
fn v86_mov_from_cr0_at_iopl_3_raises_general_protection() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_vm86_with_iopl(&mut bus, 3);
    install_v86_general_protection_handler(&mut bus);
    state.gdt_limit = 5 * 8 - 1;
    cpu.load_state(&state);

    // MOV EAX, CR0 = 0F 20 C0.
    place_in_v86_code(&mut bus, 0x0000, &[0x0F, 0x20, 0xC0]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_at_handler(&cpu, HANDLER_GENERAL_PROTECTION_IP);
}

#[test]
fn v86_mov_to_cr0_raises_general_protection() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_vm86_with_iopl(&mut bus, 3);
    install_v86_general_protection_handler(&mut bus);
    state.gdt_limit = 5 * 8 - 1;
    cpu.load_state(&state);

    // MOV CR0, EAX = 0F 22 C0.
    place_in_v86_code(&mut bus, 0x0000, &[0x0F, 0x22, 0xC0]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_at_handler(&cpu, HANDLER_GENERAL_PROTECTION_IP);
}

#[test]
fn v86_mov_from_dr0_raises_general_protection() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_vm86_with_iopl(&mut bus, 3);
    install_v86_general_protection_handler(&mut bus);
    state.gdt_limit = 5 * 8 - 1;
    cpu.load_state(&state);

    // MOV EAX, DR0 = 0F 21 C0.
    place_in_v86_code(&mut bus, 0x0000, &[0x0F, 0x21, 0xC0]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_at_handler(&cpu, HANDLER_GENERAL_PROTECTION_IP);
}

#[test]
fn v86_sldt_to_register_raises_invalid_opcode() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_vm86_with_iopl(&mut bus, 3);
    install_v86_invalid_opcode_handler(&mut bus);
    state.gdt_limit = 5 * 8 - 1;
    cpu.load_state(&state);

    // SLDT AX = 0F 00 C0.
    place_in_v86_code(&mut bus, 0x0000, &[0x0F, 0x00, 0xC0]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_at_handler(&cpu, HANDLER_INVALID_OPCODE_IP);
}

#[test]
fn v86_str_to_register_raises_invalid_opcode() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_vm86_with_iopl(&mut bus, 3);
    install_v86_invalid_opcode_handler(&mut bus);
    state.gdt_limit = 5 * 8 - 1;
    cpu.load_state(&state);

    // STR AX = 0F 00 C8.
    place_in_v86_code(&mut bus, 0x0000, &[0x0F, 0x00, 0xC8]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_at_handler(&cpu, HANDLER_INVALID_OPCODE_IP);
}

#[test]
fn v86_sgdt_to_memory_succeeds() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_vm86_with_iopl(&mut bus, 3);
    state.gdt_limit = 5 * 8 - 1;
    cpu.load_state(&state);

    // SGDT [DS:0x0040] = 0F 01 06 disp16. Write into V86 DS (base 0x40000).
    let memory_offset: u16 = 0x0040;
    place_in_v86_code(
        &mut bus,
        0x0000,
        &[
            0x0F,
            0x01,
            0x06,
            memory_offset as u8,
            (memory_offset >> 8) as u8,
            HLT_OPCODE,
        ],
    );

    cpu.step(&mut bus);

    let stored_limit = read_word_at(&bus, 0x0004_0000 + memory_offset as u32);
    assert_eq!(stored_limit, state.gdt_limit);
}

#[test]
fn v86_sidt_to_memory_succeeds() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_vm86_with_iopl(&mut bus, 3);
    state.gdt_limit = 5 * 8 - 1;
    cpu.load_state(&state);

    // SIDT [DS:0x0040] = 0F 01 0E disp16.
    let memory_offset: u16 = 0x0040;
    place_in_v86_code(
        &mut bus,
        0x0000,
        &[
            0x0F,
            0x01,
            0x0E,
            memory_offset as u8,
            (memory_offset >> 8) as u8,
            HLT_OPCODE,
        ],
    );

    cpu.step(&mut bus);

    let stored_limit = read_word_at(&bus, 0x0004_0000 + memory_offset as u32);
    assert_eq!(stored_limit, state.idt_limit);
}
