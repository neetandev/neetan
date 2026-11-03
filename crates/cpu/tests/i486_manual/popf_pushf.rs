//! PUSHF / POPF / PUSHFD / POPFD privilege-level boundary tests.
//!
//! 80486 PRM Chapter 24 ("Mixing 16-Bit and 32-Bit Code") and the per-
//! instruction reference for PUSHF/POPF. Coverage of the IOPL/IF mask rules:
//!
//! - Real mode: every flag bit writable.
//! - PM CPL=0: full IOPL/NT/IF writable.
//! - PM 0 < CPL <= IOPL: IOPL preserved, IF writable, VM read as zero on push.
//! - PM CPL > IOPL: IF preserved (popped value cannot lower or set it), IOPL
//!   preserved.
//! - VM86 IOPL=3: PUSHF/POPF behave as the protected-mode CPL=3 forms; IF
//!   writable, IOPL preserved.
//! - VM86 IOPL<3: PUSHF and POPF (and PUSHFD/POPFD) raise #GP(0).
//! - PUSHFD: bit 16 (RF) and bit 17 (VM) are cleared in the pushed image; POPF/POPFD do not
//!   modify RF and do not perform the normal post-instruction RF clear.
//!   VM (bit 17) is not modifiable via POPFD - only IRET at CPL=0 sets it.

use common::Cpu as _;
use cpu::I386State;

use super::setup::{
    HANDLER_GENERAL_PROTECTION_IP, INTERRUPT_DESCRIPTOR_TABLE_BASE,
    RIGHTS_RING0_CODE_READABLE_ACCESSED, RIGHTS_RING0_DATA_WRITABLE_ACCESSED, RING0_CODE_BASE,
    RING0_STACK_BASE, RING3_CODE_BASE, RING3_STACK_BASE, SELECTOR_RING0_CODE, TestBus,
    make_cpu_386, place_at, promote_to_ring3, read_dword_at, read_word_at,
    setup_protected_mode_with_handlers, setup_vm86, setup_vm86_with_iopl, write_interrupt_gate_386,
};

const PUSHF_OPCODE: u8 = 0x9C;
const POPF_OPCODE: u8 = 0x9D;
const OPERAND_SIZE_PREFIX: u8 = 0x66;

fn install_vm86_gp_handler(bus: &mut TestBus, state: &mut I386State) {
    write_interrupt_gate_386(
        bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        13,
        HANDLER_GENERAL_PROTECTION_IP as u32,
        SELECTOR_RING0_CODE,
        0,
    );
    bus.ram[(RING0_CODE_BASE + HANDLER_GENERAL_PROTECTION_IP as u32) as usize] = 0xF4;
    state.gdt_limit = 5 * 8 - 1;
}

fn make_real_mode_state_at(cs_segment: u16, cs_base: u32) -> I386State {
    let mut state = I386State::default();
    state.set_cs(cs_segment);
    state.seg_bases[cpu::SegReg32::CS as usize] = cs_base;
    state.set_ss(0x2000);
    state.seg_bases[cpu::SegReg32::SS as usize] = 0x0002_0000;
    state.set_esp(0x1000);
    state.seg_limits = [0xFFFF; 6];
    state.seg_rights[cpu::SegReg32::CS as usize] = RIGHTS_RING0_CODE_READABLE_ACCESSED;
    state.seg_rights[cpu::SegReg32::SS as usize] = RIGHTS_RING0_DATA_WRITABLE_ACCESSED;
    state.seg_valid = [true; 6];
    state
}

// PUSHF tests.

#[test]
fn pushf_real_mode_pushes_low_16_flags() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = make_real_mode_state_at(0x1000, 0x0001_0000);
    state.flags.if_flag = true;
    state.flags.df = true;
    cpu.load_state(&state);

    place_at(&mut bus, 0x0001_0000, &[PUSHF_OPCODE]);

    cpu.step(&mut bus);

    let pushed = read_word_at(&bus, 0x0002_0000 + 0x0FFE);
    // bit 1 always 1, IF (bit 9) and DF (bit 10) set.
    assert_eq!(pushed & 0x0602, 0x0602);
    assert_eq!(cpu.esp() & 0xFFFF, 0x0FFE, "PUSHF decrements SP by 2");
}

#[test]
fn pushfd_real_mode_pushes_full_eflags() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = make_real_mode_state_at(0x1000, 0x0001_0000);
    state.flags.if_flag = true;
    cpu.load_state(&state);

    place_at(&mut bus, 0x0001_0000, &[OPERAND_SIZE_PREFIX, PUSHF_OPCODE]);

    cpu.step(&mut bus);

    let pushed = read_dword_at(&bus, 0x0002_0000 + 0x0FFC);
    assert_ne!(pushed & 0x0200, 0, "IF set in pushed image");
    assert_eq!(cpu.esp() & 0xFFFF, 0x0FFC, "PUSHFD decrements SP by 4");
}

#[test]
fn pushfd_real_mode_clears_rf_bit() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = make_real_mode_state_at(0x1000, 0x0001_0000);
    state.eflags_upper = 0x0001_0000; // RF set
    cpu.load_state(&state);

    place_at(&mut bus, 0x0001_0000, &[OPERAND_SIZE_PREFIX, PUSHF_OPCODE]);

    cpu.step(&mut bus);

    let pushed = read_dword_at(&bus, 0x0002_0000 + 0x0FFC);
    assert_eq!(pushed & 0x0001_0000, 0, "PUSHFD clears RF in pushed image");
}

#[test]
fn pushfd_pm_pushes_vm_bit_clear() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    cpu.load_state(&state);

    place_at(
        &mut bus,
        RING0_CODE_BASE,
        &[OPERAND_SIZE_PREFIX, PUSHF_OPCODE],
    );

    cpu.step(&mut bus);

    let sp = cpu.esp();
    let pushed = read_dword_at(&bus, RING0_STACK_BASE + sp);
    assert_eq!(pushed & 0x0002_0000, 0, "PM PUSHFD shows VM bit zero");
}

#[test]
fn pushfd_pm_clears_rf_bit() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.eflags_upper |= 0x0001_0000;
    cpu.load_state(&state);

    place_at(
        &mut bus,
        RING0_CODE_BASE,
        &[OPERAND_SIZE_PREFIX, PUSHF_OPCODE],
    );

    cpu.step(&mut bus);

    let sp = cpu.esp();
    let pushed = read_dword_at(&bus, RING0_STACK_BASE + sp);
    assert_eq!(pushed & 0x0001_0000, 0, "PM PUSHFD clears RF");
}

#[test]
fn pushf_pm_cpl0_pushes_current_iopl() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.flags.iopl = 2;
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[PUSHF_OPCODE]);

    cpu.step(&mut bus);

    let sp = cpu.esp();
    let pushed = read_word_at(&bus, RING0_STACK_BASE + sp);
    assert_eq!(pushed & 0x3000, 0x2000, "PUSHF reflects current IOPL");
}

#[test]
fn pushf_pm_cpl3_succeeds_outside_vm86() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    promote_to_ring3(&mut state);
    state.set_esp(0xFFE0);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[PUSHF_OPCODE]);

    cpu.step(&mut bus);

    // PUSHF outside VM86 never faults regardless of CPL/IOPL.
    let sp = cpu.esp();
    let pushed = read_word_at(&bus, RING3_STACK_BASE + sp);
    assert_eq!(pushed & 0x0002, 0x0002, "bit 1 always 1");
}

#[test]
fn pushf_pm_cpl3_iopl0_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    promote_to_ring3(&mut state);
    state.set_esp(0xFFE0);
    state.flags.iopl = 0;
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[PUSHF_OPCODE]);

    cpu.step(&mut bus);

    // No fault expected (PUSHF only checks IOPL when in VM86).
    assert_eq!(cpu.esp(), 0xFFE0 - 2);
}

#[test]
fn pushf_vm86_iopl3_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_vm86(&mut bus);
    cpu.load_state(&state);

    place_at(
        &mut bus,
        state.seg_bases[cpu::SegReg32::CS as usize],
        &[PUSHF_OPCODE],
    );

    cpu.step(&mut bus);

    let sp = cpu.esp();
    let pushed = read_word_at(&bus, state.seg_bases[cpu::SegReg32::SS as usize] + sp);
    assert_eq!(pushed & 0x3000, 0x3000, "VM86 IOPL=3 pushed in image");
}

#[test]
fn pushf_vm86_iopl2_raises_gp() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_vm86_with_iopl(&mut bus, 2);
    install_vm86_gp_handler(&mut bus, &mut state);
    cpu.load_state(&state);

    place_at(
        &mut bus,
        state.seg_bases[cpu::SegReg32::CS as usize],
        &[PUSHF_OPCODE],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn pushf_vm86_iopl0_raises_gp() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_vm86_with_iopl(&mut bus, 0);
    install_vm86_gp_handler(&mut bus, &mut state);
    cpu.load_state(&state);

    place_at(
        &mut bus,
        state.seg_bases[cpu::SegReg32::CS as usize],
        &[PUSHF_OPCODE],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn pushfd_vm86_iopl3_stores_vm_bit_clear() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_vm86(&mut bus);
    cpu.load_state(&state);

    place_at(
        &mut bus,
        state.seg_bases[cpu::SegReg32::CS as usize],
        &[OPERAND_SIZE_PREFIX, PUSHF_OPCODE],
    );

    cpu.step(&mut bus);

    let sp = cpu.esp();
    let pushed = read_dword_at(&bus, state.seg_bases[cpu::SegReg32::SS as usize] + sp);
    assert_eq!(pushed & 0x0002_0000, 0, "VM bit clear in pushed eflags");
}

#[test]
fn pushfd_vm86_iopl0_raises_gp() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_vm86_with_iopl(&mut bus, 0);
    install_vm86_gp_handler(&mut bus, &mut state);
    cpu.load_state(&state);

    place_at(
        &mut bus,
        state.seg_bases[cpu::SegReg32::CS as usize],
        &[OPERAND_SIZE_PREFIX, PUSHF_OPCODE],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

// POPF tests.

fn push_word_at_sp(bus: &mut TestBus, ss_base: u32, sp: u32, value: u16) {
    super::setup::write_word_at(bus, ss_base + sp, value);
}

fn push_dword_at_sp(bus: &mut TestBus, ss_base: u32, sp: u32, value: u32) {
    super::setup::write_dword_at(bus, ss_base + sp, value);
}

#[test]
fn popf_real_mode_loads_full_lower_flags() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = make_real_mode_state_at(0x1000, 0x0001_0000);
    cpu.load_state(&state);

    // CF=1, bit 1=1, IF=1, DF=1, IOPL=3, NT=1.
    push_word_at_sp(&mut bus, 0x0002_0000, 0x1000, 0x7603);
    place_at(&mut bus, 0x0001_0000, &[POPF_OPCODE]);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.cf());
    assert!(cpu.state.flags.if_flag);
    assert!(cpu.state.flags.df);
    assert_eq!(cpu.state.flags.iopl, 3, "real-mode POPF can write IOPL");
    assert!(cpu.state.flags.nt, "real-mode POPF can write NT");
}

#[test]
fn popf_real_mode_increments_sp_by_2() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = make_real_mode_state_at(0x1000, 0x0001_0000);
    cpu.load_state(&state);

    push_word_at_sp(&mut bus, 0x0002_0000, 0x1000, 0x0202);
    place_at(&mut bus, 0x0001_0000, &[POPF_OPCODE]);

    cpu.step(&mut bus);

    assert_eq!(cpu.esp() & 0xFFFF, 0x1002);
}

#[test]
fn popfd_real_mode_loads_eflags_lower_16_bits() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = make_real_mode_state_at(0x1000, 0x0001_0000);
    cpu.load_state(&state);

    push_dword_at_sp(&mut bus, 0x0002_0000, 0x1000, 0x0000_3242);
    place_at(&mut bus, 0x0001_0000, &[OPERAND_SIZE_PREFIX, POPF_OPCODE]);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.if_flag);
    assert_eq!(cpu.state.flags.iopl, 3);
}

#[test]
fn popfd_real_mode_increments_sp_by_4() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = make_real_mode_state_at(0x1000, 0x0001_0000);
    cpu.load_state(&state);

    push_dword_at_sp(&mut bus, 0x0002_0000, 0x1000, 0x0000_0202);
    place_at(&mut bus, 0x0001_0000, &[OPERAND_SIZE_PREFIX, POPF_OPCODE]);

    cpu.step(&mut bus);

    assert_eq!(cpu.esp() & 0xFFFF, 0x1004);
}

#[test]
fn popf_pm_cpl0_writes_iopl() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.flags.iopl = 0;
    cpu.load_state(&state);

    let sp = state.esp();
    push_word_at_sp(&mut bus, RING0_STACK_BASE, sp - 2, 0x3202);
    state.set_esp(sp - 2);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[POPF_OPCODE]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.flags.iopl, 3, "CPL=0 POPF writes IOPL");
}

#[test]
fn popf_pm_cpl0_writes_nt() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.flags.nt = false;
    cpu.load_state(&state);

    let sp = state.esp();
    push_word_at_sp(&mut bus, RING0_STACK_BASE, sp - 2, 0x4202);
    state.set_esp(sp - 2);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[POPF_OPCODE]);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.nt, "CPL=0 POPF writes NT");
}

#[test]
fn popf_pm_cpl3_iopl3_writes_if() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    promote_to_ring3(&mut state);
    state.set_esp(0xFFE0);
    state.flags.if_flag = false;
    state.flags.iopl = 3;
    cpu.load_state(&state);

    let sp = cpu.esp();
    push_word_at_sp(&mut bus, RING3_STACK_BASE, sp - 2, 0x0202);
    cpu.state.set_esp(sp - 2);

    place_at(&mut bus, RING3_CODE_BASE, &[POPF_OPCODE]);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.if_flag, "CPL<=IOPL POPF writes IF");
}

#[test]
fn popf_pm_cpl3_iopl0_preserves_if() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    promote_to_ring3(&mut state);
    state.set_esp(0xFFE0);
    state.flags.if_flag = true;
    state.flags.iopl = 0;
    cpu.load_state(&state);

    let sp = cpu.esp();
    // Pop image clears IF (bit 9), but CPL>IOPL must preserve it.
    push_word_at_sp(&mut bus, RING3_STACK_BASE, sp - 2, 0x0002);
    cpu.state.set_esp(sp - 2);

    place_at(&mut bus, RING3_CODE_BASE, &[POPF_OPCODE]);

    cpu.step(&mut bus);

    assert!(
        cpu.state.flags.if_flag,
        "CPL>IOPL POPF preserves current IF"
    );
}

#[test]
fn popf_pm_cpl3_preserves_iopl() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    promote_to_ring3(&mut state);
    state.set_esp(0xFFE0);
    state.flags.iopl = 1;
    cpu.load_state(&state);

    let sp = cpu.esp();
    // Pop image attempts to set IOPL to 3.
    push_word_at_sp(&mut bus, RING3_STACK_BASE, sp - 2, 0x3002);
    cpu.state.set_esp(sp - 2);

    place_at(&mut bus, RING3_CODE_BASE, &[POPF_OPCODE]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.flags.iopl, 1, "CPL>0 POPF cannot raise IOPL");
}

#[test]
fn popf_pm_cpl3_iopl3_cannot_lower_iopl() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    promote_to_ring3(&mut state);
    state.set_esp(0xFFE0);
    state.flags.iopl = 3;
    cpu.load_state(&state);

    let sp = cpu.esp();
    // Pop image attempts to set IOPL to 0.
    push_word_at_sp(&mut bus, RING3_STACK_BASE, sp - 2, 0x0202);
    cpu.state.set_esp(sp - 2);

    place_at(&mut bus, RING3_CODE_BASE, &[POPF_OPCODE]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.flags.iopl, 3, "CPL>0 POPF cannot lower IOPL");
}

#[test]
fn popf_pm_cpl1_iopl1_writes_if_keeps_iopl() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.flags.iopl = 1;
    state.flags.if_flag = false;
    // Forge a fake CPL=1 by setting CS RPL=1.
    state.set_cs(SELECTOR_RING0_CODE | 1);
    state.stored_cpl = 1;
    cpu.load_state(&state);

    let sp = state.esp();
    push_word_at_sp(&mut bus, RING0_STACK_BASE, sp - 2, 0x0202);
    cpu.state.set_esp(sp - 2);

    place_at(&mut bus, RING0_CODE_BASE, &[POPF_OPCODE]);

    cpu.step(&mut bus);

    assert!(
        cpu.state.flags.if_flag,
        "CPL=IOPL boundary still permits IF write"
    );
    assert_eq!(cpu.state.flags.iopl, 1, "IOPL preserved at CPL>0");
}

#[test]
fn popf_vm86_iopl3_writes_if() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_vm86(&mut bus);
    state.flags.if_flag = false;
    cpu.load_state(&state);

    let sp = cpu.esp();
    let ss_base = state.seg_bases[cpu::SegReg32::SS as usize];
    push_word_at_sp(&mut bus, ss_base, sp - 2, 0x0202);
    cpu.state.set_esp(sp - 2);

    place_at(
        &mut bus,
        state.seg_bases[cpu::SegReg32::CS as usize],
        &[POPF_OPCODE],
    );

    cpu.step(&mut bus);

    assert!(cpu.state.flags.if_flag, "VM86 IOPL=3 POPF writes IF");
}

#[test]
fn popf_vm86_iopl3_preserves_iopl() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_vm86(&mut bus);
    cpu.load_state(&state);

    let sp = cpu.esp();
    let ss_base = state.seg_bases[cpu::SegReg32::SS as usize];
    // Pop image with IOPL=0.
    push_word_at_sp(&mut bus, ss_base, sp - 2, 0x0002);
    cpu.state.set_esp(sp - 2);

    place_at(
        &mut bus,
        state.seg_bases[cpu::SegReg32::CS as usize],
        &[POPF_OPCODE],
    );

    cpu.step(&mut bus);

    assert_eq!(cpu.state.flags.iopl, 3, "VM86 POPF cannot lower IOPL");
}

#[test]
fn popf_vm86_iopl2_raises_gp() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_vm86_with_iopl(&mut bus, 2);
    install_vm86_gp_handler(&mut bus, &mut state);
    cpu.load_state(&state);

    place_at(
        &mut bus,
        state.seg_bases[cpu::SegReg32::CS as usize],
        &[POPF_OPCODE],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn popf_vm86_iopl0_raises_gp() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_vm86_with_iopl(&mut bus, 0);
    install_vm86_gp_handler(&mut bus, &mut state);
    cpu.load_state(&state);

    place_at(
        &mut bus,
        state.seg_bases[cpu::SegReg32::CS as usize],
        &[POPF_OPCODE],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn popfd_pm_preserves_rf_bit() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.eflags_upper |= 0x0001_0000;
    cpu.load_state(&state);

    let sp = state.esp();
    // Push RF=0 in the image; POPFD must leave the current RF set.
    push_dword_at_sp(&mut bus, RING0_STACK_BASE, sp - 4, 0x0000_0202);
    cpu.state.set_esp(sp - 4);

    place_at(
        &mut bus,
        RING0_CODE_BASE,
        &[OPERAND_SIZE_PREFIX, POPF_OPCODE],
    );

    cpu.step(&mut bus);

    assert_eq!(
        cpu.state.eflags_upper & 0x0001_0000,
        0x0001_0000,
        "POPFD leaves RF unchanged"
    );
}

#[test]
fn popfd_pm_does_not_set_vm_bit() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    cpu.load_state(&state);

    let sp = state.esp();
    // Push VM=1 in image; POPFD must NOT enter VM86.
    push_dword_at_sp(&mut bus, RING0_STACK_BASE, sp - 4, 0x0002_0202);
    cpu.state.set_esp(sp - 4);

    place_at(
        &mut bus,
        RING0_CODE_BASE,
        &[OPERAND_SIZE_PREFIX, POPF_OPCODE],
    );

    cpu.step(&mut bus);

    assert_eq!(
        cpu.state.eflags_upper & 0x0002_0000,
        0,
        "POPFD does not enter VM86"
    );
}

#[test]
fn popfd_vm86_iopl3_writes_if() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_vm86(&mut bus);
    state.flags.if_flag = false;
    cpu.load_state(&state);

    let sp = cpu.esp();
    let ss_base = state.seg_bases[cpu::SegReg32::SS as usize];
    push_dword_at_sp(&mut bus, ss_base, sp - 4, 0x0000_0202);
    cpu.state.set_esp(sp - 4);

    place_at(
        &mut bus,
        state.seg_bases[cpu::SegReg32::CS as usize],
        &[OPERAND_SIZE_PREFIX, POPF_OPCODE],
    );

    cpu.step(&mut bus);

    assert!(cpu.state.flags.if_flag);
}

#[test]
fn popfd_vm86_iopl0_raises_gp() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_vm86_with_iopl(&mut bus, 0);
    install_vm86_gp_handler(&mut bus, &mut state);
    cpu.load_state(&state);

    place_at(
        &mut bus,
        state.seg_bases[cpu::SegReg32::CS as usize],
        &[OPERAND_SIZE_PREFIX, POPF_OPCODE],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

// Round-trip and cross-check tests.

#[test]
fn pushf_then_popf_round_trip_in_pm_cpl0() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.flags.df = true;
    state.flags.if_flag = true;
    state.flags.iopl = 2;
    state.flags.nt = true;
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[PUSHF_OPCODE, POPF_OPCODE]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.state.flags.df);
    assert!(cpu.state.flags.if_flag);
    assert_eq!(cpu.state.flags.iopl, 2);
    assert!(cpu.state.flags.nt);
}

#[test]
fn pushf_pm_cpl0_pushed_value_has_bit_1_set() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[PUSHF_OPCODE]);

    cpu.step(&mut bus);

    let sp = cpu.esp();
    let pushed = read_word_at(&bus, RING0_STACK_BASE + sp);
    assert_eq!(pushed & 0x0002, 0x0002);
}

#[test]
fn pushfd_pm_pushes_iopl_in_low_word() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.flags.iopl = 3;
    cpu.load_state(&state);

    place_at(
        &mut bus,
        RING0_CODE_BASE,
        &[OPERAND_SIZE_PREFIX, PUSHF_OPCODE],
    );

    cpu.step(&mut bus);

    let sp = cpu.esp();
    let pushed = read_dword_at(&bus, RING0_STACK_BASE + sp);
    assert_eq!(pushed & 0x3000, 0x3000);
}

#[test]
fn popf_does_not_change_unrelated_eflags_upper_bits() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.eflags_upper = 0x0004_0000; // AC=1
    cpu.load_state(&state);

    let sp = state.esp();
    push_word_at_sp(&mut bus, RING0_STACK_BASE, sp - 2, 0x0202);
    cpu.state.set_esp(sp - 2);

    place_at(&mut bus, RING0_CODE_BASE, &[POPF_OPCODE]);

    cpu.step(&mut bus);

    // 16-bit POPF only touches lower 16 bits; AC unchanged.
    assert_eq!(cpu.state.eflags_upper & 0x0004_0000, 0x0004_0000);
}

// Helper sanity tests.

#[test]
fn popf_pushf_helper_pushf_at_real_mode_resolves_address() {
    let mut bus = TestBus::new();
    let state = make_real_mode_state_at(0x1000, 0x0001_0000);
    place_at(&mut bus, 0x0001_0000, &[PUSHF_OPCODE]);
    assert_eq!(bus.ram[0x0001_0000], PUSHF_OPCODE);
    assert_eq!(state.cs(), 0x1000);
}

#[test]
fn popf_pushf_helper_install_vm86_gp_handler_writes_idt_slot() {
    let mut bus = TestBus::new();
    let mut state = setup_vm86_with_iopl(&mut bus, 0);
    install_vm86_gp_handler(&mut bus, &mut state);
    let address = (INTERRUPT_DESCRIPTOR_TABLE_BASE + 13 * 8) as usize;
    assert_eq!(bus.ram[address + 5] & 0x80, 0x80, "GP gate present");
}
