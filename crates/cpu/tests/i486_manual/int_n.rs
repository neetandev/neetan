//! INT n (opcode 0xCD) software interrupt dispatch tests.
//!
//! 80486 PRM Chapter 9 ("Exceptions and Interrupts") and the INT n
//! reference. Covers gate-DPL gating, gate type matrix, present-bit, target
//! descriptor checks, real-mode IVT path, and the VM86 IOPL gate (only
//! relevant to INT n - INT 3 / INTO are exempt and live in int3_into_bound.rs).
//!
//! Selector error code layout (80486 PRM Figure 9-6):
//!   bit 0: EXT (1 if external/hardware-delivered)
//!   bit 1: IDT (1 if the lookup was through the IDT)
//!   bit 2: TI  (1 if descriptor came from the LDT)
//!   bits 15..3: selector index (i.e. selector & 0xFFF8)

use common::Cpu as _;
use cpu::I386State;

use super::setup::{
    GLOBAL_DESCRIPTOR_TABLE_BASE, HANDLER_GENERAL_PROTECTION_IP, HANDLER_SEGMENT_NOT_PRESENT_IP,
    HANDLER_STACK_FAULT_IP, INTERRUPT_DESCRIPTOR_TABLE_BASE, RIGHTS_RING0_DATA_WRITABLE_ACCESSED,
    RING0_CODE_BASE, RING0_STACK_BASE, RING3_CODE_BASE, SELECTOR_RING0_CODE, SELECTOR_RING0_STACK,
    SELECTOR_RING3_CODE, SELECTOR_RING3_STACK, TASK_STATE_SEGMENT_BASE, TSS_OFFSET_ESP0, TestBus,
    make_cpu_386, make_cpu_486, place_at, place_code, promote_to_ring3, read_word_at,
    setup_protected_mode_with_handlers, setup_vm86, setup_vm86_with_iopl, write_dword_at,
    write_interrupt_gate_286, write_interrupt_gate_386, write_segment_descriptor_16bit,
    write_trap_gate_386,
};

const TEST_VECTOR: u8 = 0x42;
const HIGH_VECTOR: u8 = 0xFF;
const LOW_VECTOR: u8 = 0x10;
const TEST_VECTOR_HANDLER_IP: u16 = 0x9100;
const TEST_VECTOR_TRAP_HANDLER_IP: u16 = 0x9200;
const TEST_VECTOR_286_HANDLER_IP: u16 = 0x9300;

fn idt_selector_error_code(vector: u8, ext: u16) -> u16 {
    (vector as u16) * 8 + 2 + ext
}

fn install_int_handler_at(bus: &mut TestBus, handler_ip: u16) {
    bus.ram[(RING0_CODE_BASE + handler_ip as u32) as usize] = 0xF4;
}

#[test]
fn int_n_pm_cpl0_via_dpl0_interrupt_gate_succeeds_and_clears_if() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.flags.if_flag = true;
    cpu.load_state(&state);

    write_interrupt_gate_386(
        &mut bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        TEST_VECTOR,
        TEST_VECTOR_HANDLER_IP as u32,
        SELECTOR_RING0_CODE,
        0,
    );
    install_int_handler_at(&mut bus, TEST_VECTOR_HANDLER_IP);

    place_at(&mut bus, RING0_CODE_BASE, &[0xCD, TEST_VECTOR]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), TEST_VECTOR_HANDLER_IP as u32 + 1);
    assert!(
        !cpu.state.flags.if_flag,
        "Interrupt gate must clear EFLAGS.IF on dispatch"
    );
}

#[test]
fn int_n_pm_cpl0_via_dpl0_trap_gate_preserves_if() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.flags.if_flag = true;
    cpu.load_state(&state);

    write_trap_gate_386(
        &mut bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        TEST_VECTOR,
        TEST_VECTOR_TRAP_HANDLER_IP as u32,
        SELECTOR_RING0_CODE,
        0,
    );
    install_int_handler_at(&mut bus, TEST_VECTOR_TRAP_HANDLER_IP);

    place_at(&mut bus, RING0_CODE_BASE, &[0xCD, TEST_VECTOR]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), TEST_VECTOR_TRAP_HANDLER_IP as u32 + 1);
    assert!(
        cpu.state.flags.if_flag,
        "Trap gate must NOT clear EFLAGS.IF"
    );
}

#[test]
fn int_n_pm_cpl0_via_286_interrupt_gate_dispatches() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    cpu.load_state(&state);

    write_interrupt_gate_286(
        &mut bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        TEST_VECTOR,
        TEST_VECTOR_286_HANDLER_IP as u32,
        SELECTOR_RING0_CODE,
        0,
    );
    install_int_handler_at(&mut bus, TEST_VECTOR_286_HANDLER_IP);

    place_at(&mut bus, RING0_CODE_BASE, &[0xCD, TEST_VECTOR]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), TEST_VECTOR_286_HANDLER_IP as u32 + 1);
}

#[test]
fn int_n_pm_with_not_present_gate_raises_segment_not_present() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    cpu.load_state(&state);

    // Construct a gate manually with the present bit clear.
    let address = (INTERRUPT_DESCRIPTOR_TABLE_BASE + (TEST_VECTOR as u32) * 8) as usize;
    bus.ram[address] = TEST_VECTOR_HANDLER_IP as u8;
    bus.ram[address + 1] = (TEST_VECTOR_HANDLER_IP >> 8) as u8;
    bus.ram[address + 2] = SELECTOR_RING0_CODE as u8;
    bus.ram[address + 3] = (SELECTOR_RING0_CODE >> 8) as u8;
    bus.ram[address + 4] = 0;
    // present=0, type=interrupt 386
    bus.ram[address + 5] = super::setup::SYSTEM_TYPE_INTERRUPT_GATE_386;
    bus.ram[address + 6] = 0;
    bus.ram[address + 7] = 0;

    place_at(&mut bus, RING0_CODE_BASE, &[0xCD, TEST_VECTOR]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_SEGMENT_NOT_PRESENT_IP as u32 + 1);
}

#[test]
fn int_n_pm_with_bad_gate_type_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    cpu.load_state(&state);

    // Install a "data segment" descriptor at IDT[TEST_VECTOR].
    let address = (INTERRUPT_DESCRIPTOR_TABLE_BASE + (TEST_VECTOR as u32) * 8) as usize;
    bus.ram[address] = 0;
    bus.ram[address + 1] = 0;
    bus.ram[address + 2] = SELECTOR_RING0_CODE as u8;
    bus.ram[address + 3] = (SELECTOR_RING0_CODE >> 8) as u8;
    bus.ram[address + 4] = 0;
    // present=1, S=1 (code/data), type=data — invalid for IDT
    bus.ram[address + 5] = 0x92;
    bus.ram[address + 6] = 0;
    bus.ram[address + 7] = 0;

    place_at(&mut bus, RING0_CODE_BASE, &[0xCD, TEST_VECTOR]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn int_n_pm_with_idt_limit_too_small_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    // Shrink IDT so vector 0x42 is past its end (need offset+7 <= limit;
    // for vector 0x42 we need 0x217 <= limit, set limit smaller).
    state.idt_limit = 0x100;
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[0xCD, TEST_VECTOR]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn int_n_pm_with_target_cs_null_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    cpu.load_state(&state);

    write_interrupt_gate_386(
        &mut bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        TEST_VECTOR,
        0,
        0,
        0,
    );

    place_at(&mut bus, RING0_CODE_BASE, &[0xCD, TEST_VECTOR]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn int_n_pm_with_target_cs_data_descriptor_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    cpu.load_state(&state);

    // Use SELECTOR_RING0_DATA which points to a data descriptor (not code).
    write_interrupt_gate_386(
        &mut bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        TEST_VECTOR,
        TEST_VECTOR_HANDLER_IP as u32,
        super::setup::SELECTOR_RING0_DATA,
        0,
    );

    place_at(&mut bus, RING0_CODE_BASE, &[0xCD, TEST_VECTOR]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn int_n_pm_high_vector_dispatches() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    cpu.load_state(&state);

    write_interrupt_gate_386(
        &mut bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        HIGH_VECTOR,
        TEST_VECTOR_HANDLER_IP as u32,
        SELECTOR_RING0_CODE,
        0,
    );
    install_int_handler_at(&mut bus, TEST_VECTOR_HANDLER_IP);

    place_at(&mut bus, RING0_CODE_BASE, &[0xCD, HIGH_VECTOR]);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), TEST_VECTOR_HANDLER_IP as u32 + 1);
}

#[test]
fn int_n_pm_low_vector_dispatches() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    cpu.load_state(&state);

    write_interrupt_gate_386(
        &mut bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        LOW_VECTOR,
        TEST_VECTOR_HANDLER_IP as u32,
        SELECTOR_RING0_CODE,
        0,
    );
    install_int_handler_at(&mut bus, TEST_VECTOR_HANDLER_IP);

    place_at(&mut bus, RING0_CODE_BASE, &[0xCD, LOW_VECTOR]);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), TEST_VECTOR_HANDLER_IP as u32 + 1);
}

#[test]
fn int_n_pm_at_cpl3_via_dpl3_gate_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    write_interrupt_gate_386(
        &mut bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        TEST_VECTOR,
        TEST_VECTOR_HANDLER_IP as u32,
        SELECTOR_RING0_CODE,
        3,
    );
    install_int_handler_at(&mut bus, TEST_VECTOR_HANDLER_IP);
    promote_to_ring3(&mut state);
    cpu.load_state(&state);

    place_at(
        &mut bus,
        super::setup::RING3_CODE_BASE,
        &[0xCD, TEST_VECTOR],
    );
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), TEST_VECTOR_HANDLER_IP as u32 + 1);
    assert_eq!(cpu.cs() & 3, 0, "Inter-priv int adjusts CPL to target");
}

#[test]
fn int_n_pm_at_cpl3_via_dpl0_gate_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    write_interrupt_gate_386(
        &mut bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        TEST_VECTOR,
        TEST_VECTOR_HANDLER_IP as u32,
        SELECTOR_RING0_CODE,
        0,
    );
    install_int_handler_at(&mut bus, TEST_VECTOR_HANDLER_IP);
    promote_to_ring3(&mut state);
    cpu.load_state(&state);

    place_at(
        &mut bus,
        super::setup::RING3_CODE_BASE,
        &[0xCD, TEST_VECTOR],
    );
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(
        cpu.ip(),
        HANDLER_GENERAL_PROTECTION_IP as u32 + 1,
        "Software INT with gate DPL < CPL must raise #GP"
    );
}

#[test]
fn int_n_pm_at_cpl3_via_dpl2_gate_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    write_interrupt_gate_386(
        &mut bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        TEST_VECTOR,
        TEST_VECTOR_HANDLER_IP as u32,
        SELECTOR_RING0_CODE,
        2,
    );
    install_int_handler_at(&mut bus, TEST_VECTOR_HANDLER_IP);
    promote_to_ring3(&mut state);
    cpu.load_state(&state);

    place_at(
        &mut bus,
        super::setup::RING3_CODE_BASE,
        &[0xCD, TEST_VECTOR],
    );
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn int_n_pm_at_cpl3_inter_priv_uses_tss_esp0_ss0_for_handler_stack() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    write_interrupt_gate_386(
        &mut bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        TEST_VECTOR,
        TEST_VECTOR_HANDLER_IP as u32,
        SELECTOR_RING0_CODE,
        3,
    );
    install_int_handler_at(&mut bus, TEST_VECTOR_HANDLER_IP);
    promote_to_ring3(&mut state);
    cpu.load_state(&state);

    place_at(
        &mut bus,
        super::setup::RING3_CODE_BASE,
        &[0xCD, TEST_VECTOR],
    );
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(
        cpu.ss(),
        super::setup::SELECTOR_RING0_STACK,
        "Inter-priv int loads SS from TSS.SS0"
    );
}

#[test]
fn int_n_pm_at_cpl0_pushes_3_dword_frame_on_same_priv_dispatch() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_esp(0x0000_FFF0);
    state.flags.if_flag = true;
    cpu.load_state(&state);

    write_interrupt_gate_386(
        &mut bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        TEST_VECTOR,
        TEST_VECTOR_HANDLER_IP as u32,
        SELECTOR_RING0_CODE,
        0,
    );
    install_int_handler_at(&mut bus, TEST_VECTOR_HANDLER_IP);

    place_at(&mut bus, RING0_CODE_BASE, &[0xCD, TEST_VECTOR]);
    cpu.step(&mut bus);

    // After dispatch, ESP should have been decremented by 12 (3 dwords:
    // EFLAGS, CS, EIP). The pushed return EIP is the byte after the
    // 2-byte INT n instruction.
    let pushed_eip = super::setup::read_dword_at(&bus, super::setup::RING0_STACK_BASE + cpu.esp());
    assert_eq!(
        pushed_eip, 2,
        "Pushed return EIP must point at the byte after INT n"
    );
}

#[test]
fn int_n_pm_at_cpl3_inter_priv_pushes_5_dword_frame_no_error_code() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    write_interrupt_gate_386(
        &mut bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        TEST_VECTOR,
        TEST_VECTOR_HANDLER_IP as u32,
        SELECTOR_RING0_CODE,
        3,
    );
    install_int_handler_at(&mut bus, TEST_VECTOR_HANDLER_IP);
    promote_to_ring3(&mut state);
    state.set_esp(0xFFE0);
    cpu.load_state(&state);

    place_at(
        &mut bus,
        super::setup::RING3_CODE_BASE,
        &[0xCD, TEST_VECTOR],
    );
    cpu.step(&mut bus);

    // After dispatch via inter-priv interrupt gate, the new SS:ESP comes
    // from TSS.SS0/ESP0. Five dwords were pushed: SS, ESP_old, EFLAGS,
    // CS, EIP. ESP at 0xFFF0 - 20 = 0xFFDC.
    let new_esp = cpu.esp();
    let pushed_old_ss =
        super::setup::read_dword_at(&bus, super::setup::RING0_STACK_BASE + new_esp + 16);
    assert_eq!(
        pushed_old_ss & 0xFFFF,
        super::setup::SELECTOR_RING3_STACK as u32
    );
}

#[test]
fn int_n_vm86_at_iopl_3_dispatches_through_idt() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_vm86(&mut bus);
    cpu.load_state(&state);

    place_code(&mut bus, 0x1000, 0x0000, &[0xCD, TEST_VECTOR]);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(
        cpu.ip(),
        super::setup::VM86_HANDLER_IP as u32 + 1,
        "VM86 + IOPL=3 dispatches INT n through the protected-mode IDT"
    );
}

#[test]
fn int_n_vm86_at_iopl_2_raises_general_protection_with_zero_error_code() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_vm86_with_iopl(&mut bus, 2);
    write_interrupt_gate_386(
        &mut bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        13,
        HANDLER_GENERAL_PROTECTION_IP as u32,
        SELECTOR_RING0_CODE,
        0,
    );
    install_int_handler_at(&mut bus, HANDLER_GENERAL_PROTECTION_IP);
    state.gdt_limit = 5 * 8 - 1;
    cpu.load_state(&state);

    place_code(&mut bus, 0x1000, 0x0000, &[0xCD, TEST_VECTOR]);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn int_n_vm86_at_iopl_0_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_vm86_with_iopl(&mut bus, 0);
    write_interrupt_gate_386(
        &mut bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        13,
        HANDLER_GENERAL_PROTECTION_IP as u32,
        SELECTOR_RING0_CODE,
        0,
    );
    install_int_handler_at(&mut bus, HANDLER_GENERAL_PROTECTION_IP);
    state.gdt_limit = 5 * 8 - 1;
    cpu.load_state(&state);

    place_code(&mut bus, 0x1000, 0x0000, &[0xCD, TEST_VECTOR]);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn int_n_vm86_target_dpl_nonzero_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_vm86(&mut bus);

    // Add a CPL=3 code descriptor at slot 5, then point INT 0x42 at it
    // through a DPL=3 gate. Per 80486 PRM Section 9.9.13, VM86 -> non-zero
    // target DPL is illegal: VM86 can only escape to ring 0.
    super::setup::write_segment_descriptor_16bit(
        &mut bus,
        super::setup::GLOBAL_DESCRIPTOR_TABLE_BASE,
        5,
        super::setup::RING3_CODE_BASE,
        0xFFFF,
        super::setup::RIGHTS_RING3_CODE_READABLE_ACCESSED,
    );
    write_interrupt_gate_386(
        &mut bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        TEST_VECTOR,
        TEST_VECTOR_HANDLER_IP as u32,
        super::setup::SELECTOR_RING3_CODE,
        3,
    );
    write_interrupt_gate_386(
        &mut bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        13,
        HANDLER_GENERAL_PROTECTION_IP as u32,
        SELECTOR_RING0_CODE,
        0,
    );
    install_int_handler_at(&mut bus, HANDLER_GENERAL_PROTECTION_IP);
    state.gdt_limit = 6 * 8 - 1;
    cpu.load_state(&state);

    place_code(&mut bus, 0x1000, 0x0000, &[0xCD, TEST_VECTOR]);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(
        cpu.ip(),
        HANDLER_GENERAL_PROTECTION_IP as u32 + 1,
        "VM86 INT n must transition only to a ring-0 target"
    );
}

#[test]
fn int_n_real_mode_dispatches_via_ivt() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let handler_segment: u16 = 0x2000;
    let handler_offset: u16 = 0x0100;
    bus.ram[(TEST_VECTOR as usize) * 4] = handler_offset as u8;
    bus.ram[(TEST_VECTOR as usize) * 4 + 1] = (handler_offset >> 8) as u8;
    bus.ram[(TEST_VECTOR as usize) * 4 + 2] = handler_segment as u8;
    bus.ram[(TEST_VECTOR as usize) * 4 + 3] = (handler_segment >> 8) as u8;
    let handler_linear = ((handler_segment as u32) << 4) + handler_offset as u32;
    bus.ram[handler_linear as usize] = 0xF4;

    place_code(&mut bus, 0xFFFF, 0x0000, &[0xCD, TEST_VECTOR]);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.cs(), handler_segment);
    assert_eq!(cpu.ip() as u16, handler_offset + 1);
}

#[test]
fn int_n_real_mode_pushes_flags_cs_ip_three_word_frame() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let handler_segment: u16 = 0x2000;
    let handler_offset: u16 = 0x0100;
    bus.ram[(TEST_VECTOR as usize) * 4] = handler_offset as u8;
    bus.ram[(TEST_VECTOR as usize) * 4 + 1] = (handler_offset >> 8) as u8;
    bus.ram[(TEST_VECTOR as usize) * 4 + 2] = handler_segment as u8;
    bus.ram[(TEST_VECTOR as usize) * 4 + 3] = (handler_segment >> 8) as u8;

    let mut state = I386State::default();
    state.set_cs(0xFFFF);
    state.seg_bases[cpu::SegReg32::CS as usize] = 0x000F_FFF0;
    state.set_ss(0x3000);
    state.seg_bases[cpu::SegReg32::SS as usize] = 0x0003_0000;
    state.set_esp(0x1000);
    state.seg_limits = [0xFFFF; 6];
    state.seg_rights[cpu::SegReg32::CS as usize] = 0x9B;
    state.seg_rights[cpu::SegReg32::SS as usize] = 0x93;
    state.seg_valid = [true; 6];
    cpu.load_state(&state);

    place_at(&mut bus, 0x000F_FFF0, &[0xCD, TEST_VECTOR]);
    cpu.step(&mut bus);

    let new_sp = cpu.esp() as u16;
    let pushed_ip = super::setup::read_word_at(&bus, 0x0003_0000 + new_sp as u32);
    let pushed_cs = super::setup::read_word_at(&bus, 0x0003_0000 + new_sp as u32 + 2);
    assert_eq!(
        pushed_ip, 2,
        "Real-mode INT n pushes IP pointing past INT n"
    );
    assert_eq!(pushed_cs, 0xFFFF);
    assert_eq!(
        new_sp,
        0x1000 - 6,
        "Real-mode INT n pushes 3 words = 6 bytes"
    );
}

#[test]
fn int_n_real_mode_clears_if() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let handler_segment: u16 = 0x2000;
    let handler_offset: u16 = 0x0100;
    bus.ram[(TEST_VECTOR as usize) * 4] = handler_offset as u8;
    bus.ram[(TEST_VECTOR as usize) * 4 + 1] = (handler_offset >> 8) as u8;
    bus.ram[(TEST_VECTOR as usize) * 4 + 2] = handler_segment as u8;
    bus.ram[(TEST_VECTOR as usize) * 4 + 3] = (handler_segment >> 8) as u8;

    let mut state = I386State::default();
    state.set_cs(0xFFFF);
    state.seg_bases[cpu::SegReg32::CS as usize] = 0x000F_FFF0;
    state.set_ss(0x3000);
    state.seg_bases[cpu::SegReg32::SS as usize] = 0x0003_0000;
    state.set_esp(0x1000);
    state.seg_limits = [0xFFFF; 6];
    state.seg_rights[cpu::SegReg32::CS as usize] = 0x9B;
    state.seg_rights[cpu::SegReg32::SS as usize] = 0x93;
    state.seg_valid = [true; 6];
    state.flags.if_flag = true;
    cpu.load_state(&state);

    place_at(&mut bus, 0x000F_FFF0, &[0xCD, TEST_VECTOR]);
    cpu.step(&mut bus);

    assert!(
        !cpu.state.flags.if_flag,
        "Real-mode INT n clears EFLAGS.IF (no gate type distinction)"
    );
}

#[test]
fn int_n_pm_via_dpl3_gate_with_target_limit_too_small_raises_gp_with_zero_code() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    let target_offset_past_limit: u32 = 0x0001_0000;
    write_interrupt_gate_386(
        &mut bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        TEST_VECTOR,
        target_offset_past_limit,
        SELECTOR_RING0_CODE,
        0,
    );
    state.seg_limits[cpu::SegReg32::CS as usize] = 0xFFFF;
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[0xCD, TEST_VECTOR]);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn int_n_pm_via_286_trap_gate_preserves_if() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.flags.if_flag = true;
    cpu.load_state(&state);

    super::setup::write_trap_gate_286(
        &mut bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        TEST_VECTOR,
        TEST_VECTOR_HANDLER_IP as u32,
        SELECTOR_RING0_CODE,
        0,
    );
    install_int_handler_at(&mut bus, TEST_VECTOR_HANDLER_IP);

    place_at(&mut bus, RING0_CODE_BASE, &[0xCD, TEST_VECTOR]);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert!(cpu.state.flags.if_flag, "286 trap gate preserves IF");
}

#[test]
fn int_n_pm_via_286_interrupt_gate_clears_if() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.flags.if_flag = true;
    cpu.load_state(&state);

    write_interrupt_gate_286(
        &mut bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        TEST_VECTOR,
        TEST_VECTOR_HANDLER_IP as u32,
        SELECTOR_RING0_CODE,
        0,
    );
    install_int_handler_at(&mut bus, TEST_VECTOR_HANDLER_IP);

    place_at(&mut bus, RING0_CODE_BASE, &[0xCD, TEST_VECTOR]);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert!(
        !cpu.state.flags.if_flag,
        "286 interrupt gate also clears IF"
    );
}

#[test]
fn int_n_pm_target_descriptor_not_present_raises_segment_not_present() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    cpu.load_state(&state);

    // Slot 8 is unused (between TSS slot 7 and TSS slot 9).
    let target_selector: u16 = 0x0040;
    let access_rights_not_present: u8 = super::setup::ACCESS_DPL_RING0
        | super::setup::ACCESS_DESCRIPTOR_CODE_OR_DATA
        | super::setup::ACCESS_TYPE_CODE
        | super::setup::ACCESS_TYPE_CODE_READABLE
        | super::setup::ACCESS_TYPE_ACCESSED;
    super::setup::write_segment_descriptor_16bit(
        &mut bus,
        super::setup::GLOBAL_DESCRIPTOR_TABLE_BASE,
        target_selector >> 3,
        RING0_CODE_BASE,
        0xFFFF,
        access_rights_not_present,
    );

    write_interrupt_gate_386(
        &mut bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        TEST_VECTOR,
        TEST_VECTOR_HANDLER_IP as u32,
        target_selector,
        0,
    );

    place_at(&mut bus, RING0_CODE_BASE, &[0xCD, TEST_VECTOR]);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_SEGMENT_NOT_PRESENT_IP as u32 + 1);
}

#[test]
fn int_n_real_mode_with_high_vector_dispatches_correctly() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let handler_segment: u16 = 0x2000;
    let handler_offset: u16 = 0x0123;
    bus.ram[(HIGH_VECTOR as usize) * 4] = handler_offset as u8;
    bus.ram[(HIGH_VECTOR as usize) * 4 + 1] = (handler_offset >> 8) as u8;
    bus.ram[(HIGH_VECTOR as usize) * 4 + 2] = handler_segment as u8;
    bus.ram[(HIGH_VECTOR as usize) * 4 + 3] = (handler_segment >> 8) as u8;
    let handler_linear = ((handler_segment as u32) << 4) + handler_offset as u32;
    bus.ram[handler_linear as usize] = 0xF4;

    place_code(&mut bus, 0xFFFF, 0x0000, &[0xCD, HIGH_VECTOR]);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.cs(), handler_segment);
    assert_eq!(cpu.ip() as u16, handler_offset + 1);
}

#[test]
fn int_n_pm_dispatches_to_handler_after_load_state() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    cpu.load_state(&state);

    write_interrupt_gate_386(
        &mut bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        TEST_VECTOR,
        TEST_VECTOR_HANDLER_IP as u32,
        SELECTOR_RING0_CODE,
        0,
    );
    install_int_handler_at(&mut bus, TEST_VECTOR_HANDLER_IP);

    place_at(&mut bus, RING0_CODE_BASE, &[0xCD, TEST_VECTOR]);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.cs(), SELECTOR_RING0_CODE);
}

#[test]
fn idt_selector_error_code_for_software_int_zeroes_ext_bit() {
    // Sanity check on the helper: software INT n delivers EXT=0.
    let code = idt_selector_error_code(TEST_VECTOR, 0);
    assert_eq!(code & 0x1, 0, "EXT bit clear for software INT");
    assert_eq!(code & 0x2, 0x2, "IDT bit set for IDT-sourced fault");
    assert_eq!(code & 0xFFF8, (TEST_VECTOR as u16) * 8);
}

#[test]
fn idt_selector_error_code_for_external_int_sets_ext_bit() {
    let code = idt_selector_error_code(TEST_VECTOR, 1);
    assert_eq!(code & 0x1, 0x1, "EXT bit set for external interrupt");
    assert_eq!(code & 0x2, 0x2);
}

#[test]
fn int_n_pm_at_cpl0_with_dpl3_gate_succeeds_within_ring0() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    cpu.load_state(&state);

    write_interrupt_gate_386(
        &mut bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        TEST_VECTOR,
        TEST_VECTOR_HANDLER_IP as u32,
        SELECTOR_RING0_CODE,
        3,
    );
    install_int_handler_at(&mut bus, TEST_VECTOR_HANDLER_IP);

    place_at(&mut bus, RING0_CODE_BASE, &[0xCD, TEST_VECTOR]);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(
        cpu.ip(),
        TEST_VECTOR_HANDLER_IP as u32 + 1,
        "Gate DPL >= CPL is sufficient at any CPL"
    );
}

#[test]
fn int_n_vm86_dispatch_pushes_full_segment_frame_on_pl0_stack() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_vm86(&mut bus);
    cpu.load_state(&state);

    place_code(&mut bus, 0x1000, 0x0000, &[0xCD, TEST_VECTOR]);
    cpu.step(&mut bus);

    // After dispatch, the new SS:ESP comes from TSS.SS0/ESP0. setup_vm86
    // configures SS0 = 0x0018 (slot 3, the 32-bit ring-0 stack) and
    // ESP0 = 0x1000. VM86 dispatch pushes 9 dwords (GS, FS, DS, ES, SS,
    // ESP, EFLAGS, CS, EIP), so ESP ends at 0x1000 - 36.
    assert_eq!(
        cpu.esp(),
        0x1000 - 36,
        "VM86 dispatch through 386 gate pushes 9-dword frame"
    );
    assert_eq!(cpu.ss() & 0xFFF8, 0x0018);
}

/// Limit of the ring 0 stack segment in the new stack overflow test.
const SMALL_RING0_STACK_LIMIT: u16 = 0x0FFF;
/// TSS ESP0 of the new stack overflow test. The fifth slot of a 386 frame wraps to
/// offset 0xFFFC, beyond the limit. A 286 frame with an error code fits.
const SMALL_RING0_STACK_POINTER: u32 = 0x0010;

#[test]
fn int_n_pm_inter_priv_new_stack_overflow_raises_stack_fault_with_new_ss_selector() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    write_interrupt_gate_386(
        &mut bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        TEST_VECTOR,
        TEST_VECTOR_HANDLER_IP as u32,
        SELECTOR_RING0_CODE,
        3,
    );
    install_int_handler_at(&mut bus, TEST_VECTOR_HANDLER_IP);
    write_interrupt_gate_286(
        &mut bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        12,
        HANDLER_STACK_FAULT_IP as u32,
        SELECTOR_RING0_CODE,
        0,
    );
    write_segment_descriptor_16bit(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        SELECTOR_RING0_STACK >> 3,
        RING0_STACK_BASE,
        SMALL_RING0_STACK_LIMIT,
        RIGHTS_RING0_DATA_WRITABLE_ACCESSED,
    );
    write_dword_at(
        &mut bus,
        TASK_STATE_SEGMENT_BASE + TSS_OFFSET_ESP0,
        SMALL_RING0_STACK_POINTER,
    );
    promote_to_ring3(&mut state);
    state.set_esp(0xFFE0);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[0xCD, TEST_VECTOR]);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    // 80486 PRM 9.9.12: an overflow of the new stack pushes the selector of the new
    // stack segment. The check comes before SS:ESP is committed, so the #SS frame holds
    // the ring 3 stack and the address of INT n.
    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_STACK_FAULT_IP as u32 + 1);
    let frame = RING0_STACK_BASE + cpu.esp();
    assert_eq!(read_word_at(&bus, frame), SELECTOR_RING0_STACK & 0xFFFC);
    assert_eq!(
        read_word_at(&bus, frame + 2),
        0,
        "the frame points at INT n"
    );
    assert_eq!(read_word_at(&bus, frame + 4), SELECTOR_RING3_CODE);
    assert_eq!(read_word_at(&bus, frame + 8), 0xFFE0);
    assert_eq!(read_word_at(&bus, frame + 10), SELECTOR_RING3_STACK);
}
