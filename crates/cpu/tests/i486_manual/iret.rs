//! IRET / IRETD return-from-interrupt dispatch tests.
//!
//! 80486 PRM Chapter 9 ("Exceptions and Interrupts") and the IRET
//! reference. Covers same-privilege return, inter-privilege return, NT=1
//! task-switch return, the CPL=0 -> VM86 transition, the VM86 IOPL gate, and
//! real-mode 3-word frame handling.
//!
//! IRET selects 16-bit (3-word IP/CS/FLAGS) or 32-bit (3-dword EIP/CS/EFLAGS)
//! frames based on the operand-size override prefix (0x66). For inter-
//! privilege returns the implementation pops two extra slots (ESP/SS); for the
//! CPL=0 -> VM86 transition the 32-bit form pops nine dwords total.

use common::Cpu as _;
use cpu::I386State;

use super::setup::{
    ACCESS_DESCRIPTOR_CODE_OR_DATA, ACCESS_DPL_RING0, ACCESS_DPL_RING3, ACCESS_PRESENT,
    ACCESS_TYPE_ACCESSED, ACCESS_TYPE_CODE, ACCESS_TYPE_CODE_CONFORMING, ACCESS_TYPE_CODE_READABLE,
    GLOBAL_DESCRIPTOR_TABLE_BASE, HANDLER_GENERAL_PROTECTION_IP, HANDLER_INVALID_TSS_IP,
    HANDLER_SEGMENT_NOT_PRESENT_IP, HANDLER_STACK_FAULT_IP, RIGHTS_RING0_CODE_READABLE_ACCESSED,
    RIGHTS_RING0_DATA_WRITABLE_ACCESSED, RIGHTS_RING3_DATA_WRITABLE_ACCESSED,
    RIGHTS_TSS_386_AVAILABLE, RIGHTS_TSS_386_BUSY, RING0_CODE_BASE, RING0_STACK_BASE,
    RING3_CODE_BASE, RING3_STACK_BASE, SELECTOR_RING0_CODE, SELECTOR_RING0_STACK,
    SELECTOR_RING3_CODE, SELECTOR_RING3_DATA, SELECTOR_RING3_STACK, SELECTOR_SECONDARY_TSS,
    SHARED_DATA_BASE, TASK_STATE_SEGMENT_BASE, TASK_STATE_SEGMENT_SECONDARY_BASE,
    TSS_MINIMUM_LIMIT, TSS_OFFSET_LINK, TestBus, Tss386Image, make_cpu_386, place_at,
    promote_to_ring3, read_dword_at, read_word_at, setup_protected_mode_with_handlers, setup_vm86,
    setup_vm86_with_iopl, write_dword_at, write_segment_descriptor_16bit, write_tss_386,
    write_word_at,
};

const RETURN_IP: u16 = 0x2000;
const RETURN_EIP: u32 = 0x0000_2000;
const SECONDARY_TASK_EIP: u32 = 0x0000_3000;

fn install_target_hlt(bus: &mut TestBus, segment_base: u32, ip: u32) {
    bus.ram[(segment_base + ip) as usize] = 0xF4;
}

fn make_pm_state_with_32bit_stack(bus: &mut TestBus) -> I386State {
    let mut state = setup_protected_mode_with_handlers(bus);
    state.seg_granularity[cpu::SegReg32::SS as usize] = 0x40;
    state.seg_limits[cpu::SegReg32::SS as usize] = 0xFFFF_FFFF;
    state
}

#[test]
fn iret_pm_same_privilege_16bit_pops_ip_cs_flags() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    cpu.load_state(&state);

    let sp = cpu.esp();
    write_word_at(&mut bus, RING0_STACK_BASE + sp, RETURN_IP);
    write_word_at(&mut bus, RING0_STACK_BASE + sp + 2, SELECTOR_RING0_CODE);
    write_word_at(&mut bus, RING0_STACK_BASE + sp + 4, 0x0202);

    place_at(&mut bus, RING0_CODE_BASE, &[0xCF]);
    install_target_hlt(&mut bus, RING0_CODE_BASE, RETURN_IP as u32);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), RETURN_IP as u32 + 1);
    assert_eq!(cpu.cs(), SELECTOR_RING0_CODE);
}

#[test]
fn iret_pm_same_privilege_16bit_increments_sp_by_six() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    cpu.load_state(&state);

    let initial_sp = cpu.esp();
    write_word_at(&mut bus, RING0_STACK_BASE + initial_sp, RETURN_IP);
    write_word_at(
        &mut bus,
        RING0_STACK_BASE + initial_sp + 2,
        SELECTOR_RING0_CODE,
    );
    write_word_at(&mut bus, RING0_STACK_BASE + initial_sp + 4, 0x0202);

    place_at(&mut bus, RING0_CODE_BASE, &[0xCF]);
    install_target_hlt(&mut bus, RING0_CODE_BASE, RETURN_IP as u32);

    cpu.step(&mut bus);

    assert_eq!(
        cpu.esp() & 0xFFFF,
        (initial_sp + 6) & 0xFFFF,
        "16-bit IRET pops three words"
    );
}

#[test]
fn iret_pm_same_privilege_32bit_pops_eip_cs_eflags() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = make_pm_state_with_32bit_stack(&mut bus);
    cpu.load_state(&state);

    let sp = cpu.esp();
    write_dword_at(&mut bus, RING0_STACK_BASE + sp, RETURN_EIP);
    write_dword_at(
        &mut bus,
        RING0_STACK_BASE + sp + 4,
        SELECTOR_RING0_CODE as u32,
    );
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 8, 0x0000_0202);

    place_at(&mut bus, RING0_CODE_BASE, &[0x66, 0xCF]);
    install_target_hlt(&mut bus, RING0_CODE_BASE, RETURN_EIP);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), RETURN_EIP + 1);
    assert_eq!(cpu.cs(), SELECTOR_RING0_CODE);
}

#[test]
fn iret_pm_same_privilege_32bit_increments_esp_by_twelve() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = make_pm_state_with_32bit_stack(&mut bus);
    cpu.load_state(&state);

    let initial_sp = cpu.esp();
    write_dword_at(&mut bus, RING0_STACK_BASE + initial_sp, RETURN_EIP);
    write_dword_at(
        &mut bus,
        RING0_STACK_BASE + initial_sp + 4,
        SELECTOR_RING0_CODE as u32,
    );
    write_dword_at(&mut bus, RING0_STACK_BASE + initial_sp + 8, 0x0000_0202);

    place_at(&mut bus, RING0_CODE_BASE, &[0x66, 0xCF]);
    install_target_hlt(&mut bus, RING0_CODE_BASE, RETURN_EIP);

    cpu.step(&mut bus);

    assert_eq!(cpu.esp(), initial_sp + 12, "32-bit IRETD pops three dwords");
}

#[test]
fn iret_pm_with_null_cs_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    cpu.load_state(&state);

    let sp = cpu.esp();
    write_word_at(&mut bus, RING0_STACK_BASE + sp, RETURN_IP);
    write_word_at(&mut bus, RING0_STACK_BASE + sp + 2, 0);
    write_word_at(&mut bus, RING0_STACK_BASE + sp + 4, 0x0202);

    place_at(&mut bus, RING0_CODE_BASE, &[0xCF]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn iret_pm_with_data_segment_cs_raises_gp_with_selector() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    cpu.load_state(&state);

    // Slot 2 is the ring-0 data segment; using it as CS must fault.
    let bad_cs: u16 = 0x0010;
    let sp = cpu.esp();
    write_word_at(&mut bus, RING0_STACK_BASE + sp, RETURN_IP);
    write_word_at(&mut bus, RING0_STACK_BASE + sp + 2, bad_cs);
    write_word_at(&mut bus, RING0_STACK_BASE + sp + 4, 0x0202);

    place_at(&mut bus, RING0_CODE_BASE, &[0xCF]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn iret_pm_with_not_present_cs_raises_segment_not_present() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    cpu.load_state(&state);

    // Slot 8 is unused. Install a code descriptor at slot 8 with present=0.
    let target_selector: u16 = 0x0040;
    let access_rights_not_present: u8 = ACCESS_DPL_RING0
        | ACCESS_DESCRIPTOR_CODE_OR_DATA
        | ACCESS_TYPE_CODE
        | ACCESS_TYPE_CODE_READABLE
        | ACCESS_TYPE_ACCESSED;
    write_segment_descriptor_16bit(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        target_selector >> 3,
        RING0_CODE_BASE,
        0xFFFF,
        access_rights_not_present,
    );

    let sp = cpu.esp();
    write_word_at(&mut bus, RING0_STACK_BASE + sp, RETURN_IP);
    write_word_at(&mut bus, RING0_STACK_BASE + sp + 2, target_selector);
    write_word_at(&mut bus, RING0_STACK_BASE + sp + 4, 0x0202);

    place_at(&mut bus, RING0_CODE_BASE, &[0xCF]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_SEGMENT_NOT_PRESENT_IP as u32 + 1);
}

#[test]
fn iret_pm_with_eip_past_cs_limit_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = make_pm_state_with_32bit_stack(&mut bus);
    cpu.load_state(&state);

    // Target EIP = 0x10000 but ring-0 code segment limit is 0xFFFF.
    let target_eip: u32 = 0x0001_0000;
    let sp = cpu.esp();
    write_dword_at(&mut bus, RING0_STACK_BASE + sp, target_eip);
    write_dword_at(
        &mut bus,
        RING0_STACK_BASE + sp + 4,
        SELECTOR_RING0_CODE as u32,
    );
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 8, 0x0000_0202);

    place_at(&mut bus, RING0_CODE_BASE, &[0x66, 0xCF]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn iret_pm_with_nonconforming_cs_dpl_ne_rpl_raises_gp() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    cpu.load_state(&state);

    // Install a non-conforming DPL=3 code segment at slot 8, then try to
    // return to it with RPL=0 (mismatch -> #GP).
    let target_selector_rpl0: u16 = 0x0040;
    let rights_dpl3_nonconforming: u8 = ACCESS_PRESENT
        | ACCESS_DPL_RING3
        | ACCESS_DESCRIPTOR_CODE_OR_DATA
        | ACCESS_TYPE_CODE
        | ACCESS_TYPE_CODE_READABLE
        | ACCESS_TYPE_ACCESSED;
    write_segment_descriptor_16bit(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        target_selector_rpl0 >> 3,
        RING0_CODE_BASE,
        0xFFFF,
        rights_dpl3_nonconforming,
    );

    let sp = cpu.esp();
    write_word_at(&mut bus, RING0_STACK_BASE + sp, RETURN_IP);
    write_word_at(&mut bus, RING0_STACK_BASE + sp + 2, target_selector_rpl0);
    write_word_at(&mut bus, RING0_STACK_BASE + sp + 4, 0x0202);

    place_at(&mut bus, RING0_CODE_BASE, &[0xCF]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn iret_pm_with_conforming_cs_dpl_greater_than_rpl_raises_gp() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = make_pm_state_with_32bit_stack(&mut bus);
    cpu.load_state(&state);

    // Install a conforming DPL=3 code descriptor at slot 8; return with
    // RPL=0 from CPL=0 yields rpl < dpl on a conforming target -> #GP.
    let target_selector: u16 = 0x0040;
    let rights_conforming_dpl3: u8 = ACCESS_PRESENT
        | ACCESS_DPL_RING3
        | ACCESS_DESCRIPTOR_CODE_OR_DATA
        | ACCESS_TYPE_CODE
        | ACCESS_TYPE_CODE_CONFORMING
        | ACCESS_TYPE_CODE_READABLE
        | ACCESS_TYPE_ACCESSED;
    write_segment_descriptor_16bit(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        target_selector >> 3,
        RING0_CODE_BASE,
        0xFFFF,
        rights_conforming_dpl3,
    );

    let sp = cpu.esp();
    write_dword_at(&mut bus, RING0_STACK_BASE + sp, RETURN_EIP);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 4, target_selector as u32);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 8, 0x0000_0202);

    place_at(&mut bus, RING0_CODE_BASE, &[0x66, 0xCF]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn iret_pm_at_cpl0_can_change_iopl() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = make_pm_state_with_32bit_stack(&mut bus);
    state.flags.iopl = 0;
    cpu.load_state(&state);

    let sp = cpu.esp();
    write_dword_at(&mut bus, RING0_STACK_BASE + sp, RETURN_EIP);
    write_dword_at(
        &mut bus,
        RING0_STACK_BASE + sp + 4,
        SELECTOR_RING0_CODE as u32,
    );
    // EFLAGS with IOPL=3 (bits 12-13 = 11) and IF=1.
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 8, 0x0000_3202);

    place_at(&mut bus, RING0_CODE_BASE, &[0x66, 0xCF]);

    cpu.step(&mut bus);

    assert_eq!(
        cpu.state.flags.iopl, 3,
        "CPL=0 IRET writes IOPL from popped EFLAGS"
    );
}

#[test]
fn iret_pm_at_cpl_nonzero_preserves_iopl() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = make_pm_state_with_32bit_stack(&mut bus);
    promote_to_ring3(&mut state);
    state.set_esp(0xFFE0);
    state.flags.iopl = 1;
    state.flags.if_flag = true;
    cpu.load_state(&state);

    let sp = cpu.esp();
    // Pop EIP/CS/EFLAGS targeting ring-3 code. EFLAGS asks for IOPL=3,
    // which CPL=3 must NOT honour.
    write_dword_at(&mut bus, RING3_STACK_BASE + sp, RETURN_EIP);
    write_dword_at(
        &mut bus,
        RING3_STACK_BASE + sp + 4,
        SELECTOR_RING3_CODE as u32,
    );
    write_dword_at(&mut bus, RING3_STACK_BASE + sp + 8, 0x0000_3202);

    place_at(&mut bus, RING3_CODE_BASE, &[0x66, 0xCF]);

    cpu.step(&mut bus);

    assert_eq!(
        cpu.state.flags.iopl, 1,
        "CPL>0 IRET must not lower or raise IOPL"
    );
}

#[test]
fn iret_pm_at_cpl_above_iopl_preserves_if() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = make_pm_state_with_32bit_stack(&mut bus);
    promote_to_ring3(&mut state);
    state.set_esp(0xFFE0);
    state.flags.iopl = 0;
    state.flags.if_flag = true;
    cpu.load_state(&state);

    let sp = cpu.esp();
    // EFLAGS image clears IF (bit 9), but CPL=3 > IOPL=0 must preserve IF.
    write_dword_at(&mut bus, RING3_STACK_BASE + sp, RETURN_EIP);
    write_dword_at(
        &mut bus,
        RING3_STACK_BASE + sp + 4,
        SELECTOR_RING3_CODE as u32,
    );
    write_dword_at(&mut bus, RING3_STACK_BASE + sp + 8, 0x0000_0002);

    place_at(&mut bus, RING3_CODE_BASE, &[0x66, 0xCF]);

    cpu.step(&mut bus);

    assert!(
        cpu.state.flags.if_flag,
        "CPL>IOPL: IF must remain set across IRET"
    );
}

#[test]
fn iret_pm_at_cpl0_clears_nt_when_loaded_zero() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = make_pm_state_with_32bit_stack(&mut bus);
    state.flags.nt = false;
    cpu.load_state(&state);

    let sp = cpu.esp();
    write_dword_at(&mut bus, RING0_STACK_BASE + sp, RETURN_EIP);
    write_dword_at(
        &mut bus,
        RING0_STACK_BASE + sp + 4,
        SELECTOR_RING0_CODE as u32,
    );
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 8, 0x0000_0202);

    place_at(&mut bus, RING0_CODE_BASE, &[0x66, 0xCF]);

    cpu.step(&mut bus);

    assert!(
        !cpu.state.flags.nt,
        "EFLAGS image with NT=0 must keep NT clear"
    );
}

#[test]
fn iret_pm_eflags_bit1_remains_set() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = make_pm_state_with_32bit_stack(&mut bus);
    cpu.load_state(&state);

    let sp = cpu.esp();
    // EFLAGS image with bit 1 cleared; processor enforces it as always 1.
    write_dword_at(&mut bus, RING0_STACK_BASE + sp, RETURN_EIP);
    write_dword_at(
        &mut bus,
        RING0_STACK_BASE + sp + 4,
        SELECTOR_RING0_CODE as u32,
    );
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 8, 0x0000_0000);

    place_at(&mut bus, RING0_CODE_BASE, &[0x66, 0xCF]);

    cpu.step(&mut bus);

    let eflags = cpu.state.flags.compress() as u32 | cpu.state.eflags_upper;
    assert_eq!(
        eflags & 0x0000_0002,
        0x0000_0002,
        "EFLAGS bit 1 is fixed at 1"
    );
}

#[test]
fn iret_pm_at_cpl_nonzero_silently_strips_vm_bit() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = make_pm_state_with_32bit_stack(&mut bus);
    promote_to_ring3(&mut state);
    state.set_esp(0xFFE0);
    cpu.load_state(&state);

    let sp = cpu.esp();
    // VM=1 in popped EFLAGS at CPL=3 must NOT enter VM86; it is silently
    // stripped. (Manual: only CPL=0 IRETD can enter VM86.)
    write_dword_at(&mut bus, RING3_STACK_BASE + sp, RETURN_EIP);
    write_dword_at(
        &mut bus,
        RING3_STACK_BASE + sp + 4,
        SELECTOR_RING3_CODE as u32,
    );
    write_dword_at(&mut bus, RING3_STACK_BASE + sp + 8, 0x0002_0002);

    place_at(&mut bus, RING3_CODE_BASE, &[0x66, 0xCF]);

    cpu.step(&mut bus);

    assert_eq!(
        cpu.state.eflags_upper & 0x0002_0000,
        0,
        "CPL>0 IRETD must mask VM bit out of EFLAGS"
    );
}

// Inter-privilege returns (CPL=0 -> CPL=3).

fn pm_inter_privilege_setup_with_32bit_stack(bus: &mut TestBus) -> I386State {
    let mut state = setup_protected_mode_with_handlers(bus);
    state.seg_granularity[cpu::SegReg32::SS as usize] = 0x40;
    state.seg_limits[cpu::SegReg32::SS as usize] = 0xFFFF_FFFF;
    state
}

#[test]
fn iret_pm_inner_to_outer_loads_new_cs_eip_ss_esp() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = pm_inter_privilege_setup_with_32bit_stack(&mut bus);
    cpu.load_state(&state);

    let target_eip: u32 = 0x0000_0500;
    let target_esp: u32 = 0x0000_FF00;
    let sp = cpu.esp();
    write_dword_at(&mut bus, RING0_STACK_BASE + sp, target_eip);
    write_dword_at(
        &mut bus,
        RING0_STACK_BASE + sp + 4,
        SELECTOR_RING3_CODE as u32,
    );
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 8, 0x0000_0202);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 12, target_esp);
    write_dword_at(
        &mut bus,
        RING0_STACK_BASE + sp + 16,
        SELECTOR_RING3_STACK as u32,
    );

    place_at(&mut bus, RING0_CODE_BASE, &[0x66, 0xCF]);

    cpu.step(&mut bus);

    assert_eq!(cpu.cs(), SELECTOR_RING3_CODE);
    assert_eq!(cpu.cs() & 3, 3, "Now running at CPL=3");
    assert_eq!(cpu.ip(), target_eip);
    assert_eq!(cpu.ss(), SELECTOR_RING3_STACK);
    assert_eq!(cpu.esp(), target_esp);
}

#[test]
fn iret_pm_inner_to_outer_16bit_pops_5_words() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = pm_inter_privilege_setup_with_32bit_stack(&mut bus);
    cpu.load_state(&state);

    let target_ip: u16 = 0x0500;
    let target_sp: u16 = 0xFF00;
    let sp = cpu.esp();
    write_word_at(&mut bus, RING0_STACK_BASE + sp, target_ip);
    write_word_at(&mut bus, RING0_STACK_BASE + sp + 2, SELECTOR_RING3_CODE);
    write_word_at(&mut bus, RING0_STACK_BASE + sp + 4, 0x0202);
    write_word_at(&mut bus, RING0_STACK_BASE + sp + 6, target_sp);
    write_word_at(&mut bus, RING0_STACK_BASE + sp + 8, SELECTOR_RING3_STACK);

    place_at(&mut bus, RING0_CODE_BASE, &[0xCF]);

    cpu.step(&mut bus);

    assert_eq!(cpu.cs(), SELECTOR_RING3_CODE);
    assert_eq!(cpu.ip() as u16, target_ip);
    assert_eq!(cpu.ss(), SELECTOR_RING3_STACK);
    assert_eq!(cpu.esp() as u16, target_sp);
}

#[test]
fn iret_pm_outer_to_inner_attempt_raises_gp() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = pm_inter_privilege_setup_with_32bit_stack(&mut bus);
    promote_to_ring3(&mut state);
    state.set_esp(0xFFE0);
    cpu.load_state(&state);

    // CPL=3 returning to RPL=0 (lower numerical = higher privilege) must
    // fault with #GP per Intel 80486 PRM.
    let sp = cpu.esp();
    write_dword_at(&mut bus, RING3_STACK_BASE + sp, RETURN_EIP);
    write_dword_at(
        &mut bus,
        RING3_STACK_BASE + sp + 4,
        SELECTOR_RING0_CODE as u32,
    );
    write_dword_at(&mut bus, RING3_STACK_BASE + sp + 8, 0x0000_0202);

    place_at(&mut bus, RING3_CODE_BASE, &[0x66, 0xCF]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn iret_pm_inner_to_outer_with_null_ss_raises_gp() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = pm_inter_privilege_setup_with_32bit_stack(&mut bus);
    cpu.load_state(&state);

    let sp = cpu.esp();
    write_dword_at(&mut bus, RING0_STACK_BASE + sp, RETURN_EIP);
    write_dword_at(
        &mut bus,
        RING0_STACK_BASE + sp + 4,
        SELECTOR_RING3_CODE as u32,
    );
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 8, 0x0000_0202);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 12, 0x0000_FF00);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 16, 0);

    place_at(&mut bus, RING0_CODE_BASE, &[0x66, 0xCF]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn iret_pm_inner_to_outer_with_ss_not_present_raises_stack_fault() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = pm_inter_privilege_setup_with_32bit_stack(&mut bus);
    cpu.load_state(&state);

    // Install a not-present DPL=3 stack descriptor at slot 8.
    let bad_ss: u16 = 0x0040 | 3;
    let rights_ring3_data_not_present: u8 = ACCESS_DPL_RING3
        | ACCESS_DESCRIPTOR_CODE_OR_DATA
        | super::setup::ACCESS_TYPE_DATA_WRITABLE
        | ACCESS_TYPE_ACCESSED;
    write_segment_descriptor_16bit(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        bad_ss >> 3,
        RING3_STACK_BASE,
        0xFFFF,
        rights_ring3_data_not_present,
    );

    let sp = cpu.esp();
    write_dword_at(&mut bus, RING0_STACK_BASE + sp, RETURN_EIP);
    write_dword_at(
        &mut bus,
        RING0_STACK_BASE + sp + 4,
        SELECTOR_RING3_CODE as u32,
    );
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 8, 0x0000_0202);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 12, 0x0000_FF00);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 16, bad_ss as u32);

    place_at(&mut bus, RING0_CODE_BASE, &[0x66, 0xCF]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_STACK_FAULT_IP as u32 + 1);
}

#[test]
fn iret_pm_inner_to_outer_with_wrong_ss_dpl_raises_fault() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = pm_inter_privilege_setup_with_32bit_stack(&mut bus);
    cpu.load_state(&state);

    // SS RPL=3 but descriptor DPL=0 must raise a fault during inter-priv
    // return.
    let mismatched_ss: u16 = 0x0040 | 3;
    let rights_ring0_data: u8 = ACCESS_PRESENT
        | ACCESS_DPL_RING0
        | ACCESS_DESCRIPTOR_CODE_OR_DATA
        | super::setup::ACCESS_TYPE_DATA_WRITABLE
        | ACCESS_TYPE_ACCESSED;
    write_segment_descriptor_16bit(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        mismatched_ss >> 3,
        RING3_STACK_BASE,
        0xFFFF,
        rights_ring0_data,
    );

    let sp = cpu.esp();
    write_dword_at(&mut bus, RING0_STACK_BASE + sp, RETURN_EIP);
    write_dword_at(
        &mut bus,
        RING0_STACK_BASE + sp + 4,
        SELECTOR_RING3_CODE as u32,
    );
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 8, 0x0000_0202);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 12, 0x0000_FF00);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 16, mismatched_ss as u32);

    place_at(&mut bus, RING0_CODE_BASE, &[0x66, 0xCF]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    let landed = cpu.ip();
    assert!(
        landed == HANDLER_GENERAL_PROTECTION_IP as u32 + 1
            || landed == HANDLER_STACK_FAULT_IP as u32 + 1,
        "SS DPL mismatch must raise #GP or #SS"
    );
}

#[test]
fn iret_pm_inner_to_outer_revalidates_data_segments() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = pm_inter_privilege_setup_with_32bit_stack(&mut bus);
    // Pretend ring-0 code had DS pointing to ring-0 data: after IRET to
    // ring-3 the loader must zero-out (mark invalid) DS because DPL<RPL.
    state.set_ds(super::setup::SELECTOR_RING0_DATA);
    state.seg_rights[cpu::SegReg32::DS as usize] = RIGHTS_RING0_DATA_WRITABLE_ACCESSED;
    state.seg_valid[cpu::SegReg32::DS as usize] = true;
    cpu.load_state(&state);

    let sp = cpu.esp();
    write_dword_at(&mut bus, RING0_STACK_BASE + sp, RETURN_EIP);
    write_dword_at(
        &mut bus,
        RING0_STACK_BASE + sp + 4,
        SELECTOR_RING3_CODE as u32,
    );
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 8, 0x0000_0202);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 12, 0x0000_FF00);
    write_dword_at(
        &mut bus,
        RING0_STACK_BASE + sp + 16,
        SELECTOR_RING3_STACK as u32,
    );

    place_at(&mut bus, RING0_CODE_BASE, &[0x66, 0xCF]);

    cpu.step(&mut bus);

    assert!(
        !cpu.state.seg_valid[cpu::SegReg32::DS as usize] || cpu.state.ds() == 0,
        "DS targeting a higher-privileged segment must be invalidated"
    );
}

#[test]
fn iret_pm_inner_to_outer_keeps_ring3_data_segment() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = pm_inter_privilege_setup_with_32bit_stack(&mut bus);
    // DS at ring-3 already; revalidation must leave it alone.
    state.set_ds(SELECTOR_RING3_DATA);
    state.seg_bases[cpu::SegReg32::DS as usize] = SHARED_DATA_BASE;
    state.seg_rights[cpu::SegReg32::DS as usize] = RIGHTS_RING3_DATA_WRITABLE_ACCESSED;
    state.seg_valid[cpu::SegReg32::DS as usize] = true;
    cpu.load_state(&state);

    let sp = cpu.esp();
    write_dword_at(&mut bus, RING0_STACK_BASE + sp, RETURN_EIP);
    write_dword_at(
        &mut bus,
        RING0_STACK_BASE + sp + 4,
        SELECTOR_RING3_CODE as u32,
    );
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 8, 0x0000_0202);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 12, 0x0000_FF00);
    write_dword_at(
        &mut bus,
        RING0_STACK_BASE + sp + 16,
        SELECTOR_RING3_STACK as u32,
    );

    place_at(&mut bus, RING0_CODE_BASE, &[0x66, 0xCF]);

    cpu.step(&mut bus);

    assert_eq!(
        cpu.ds(),
        SELECTOR_RING3_DATA,
        "ring-3-accessible DS survives inter-priv return"
    );
}

#[test]
fn iret_pm_inner_to_outer_loads_low_16_bits_of_ss_only_for_16bit_iret() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = pm_inter_privilege_setup_with_32bit_stack(&mut bus);
    cpu.load_state(&state);

    // 16-bit IRET frame puts SS in two bytes (rather than a dword).
    let target_ip: u16 = 0x0500;
    let target_sp: u16 = 0xFF00;
    let sp = cpu.esp();
    write_word_at(&mut bus, RING0_STACK_BASE + sp, target_ip);
    write_word_at(&mut bus, RING0_STACK_BASE + sp + 2, SELECTOR_RING3_CODE);
    write_word_at(&mut bus, RING0_STACK_BASE + sp + 4, 0x0202);
    write_word_at(&mut bus, RING0_STACK_BASE + sp + 6, target_sp);
    write_word_at(&mut bus, RING0_STACK_BASE + sp + 8, SELECTOR_RING3_STACK);

    place_at(&mut bus, RING0_CODE_BASE, &[0xCF]);
    cpu.step(&mut bus);
    assert_eq!(cpu.ss(), SELECTOR_RING3_STACK);
    assert_eq!(cpu.esp(), target_sp as u32);
}

#[test]
fn iret_pm_inner_to_outer_does_not_pop_esp_ss_for_same_priv() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = make_pm_state_with_32bit_stack(&mut bus);
    cpu.load_state(&state);

    // Same-priv: only 3 dwords are popped. Stack pointer should advance
    // by exactly 12, no SS reload.
    let initial_sp = cpu.esp();
    write_dword_at(&mut bus, RING0_STACK_BASE + initial_sp, RETURN_EIP);
    write_dword_at(
        &mut bus,
        RING0_STACK_BASE + initial_sp + 4,
        SELECTOR_RING0_CODE as u32,
    );
    write_dword_at(&mut bus, RING0_STACK_BASE + initial_sp + 8, 0x0000_0202);
    // Sentinels in the 4th/5th dword: must not be consumed.
    write_dword_at(&mut bus, RING0_STACK_BASE + initial_sp + 12, 0xCAFE_BABE);
    write_dword_at(&mut bus, RING0_STACK_BASE + initial_sp + 16, 0xDEAD_BEEF);

    place_at(&mut bus, RING0_CODE_BASE, &[0x66, 0xCF]);
    cpu.step(&mut bus);

    assert_eq!(
        cpu.esp(),
        initial_sp + 12,
        "same-priv 32-bit IRETD pops only 3 dwords"
    );
    assert_eq!(cpu.ss(), SELECTOR_RING0_STACK, "SS unchanged in same-priv");
}

// NT=1 task-switch IRET tests.

fn pm_with_nt_secondary_tss_setup(bus: &mut TestBus) -> I386State {
    let state = setup_protected_mode_with_handlers(bus);

    // Configure secondary TSS as busy 386 TSS so an NT-IRET can return to it.
    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        SELECTOR_SECONDARY_TSS >> 3,
        TASK_STATE_SEGMENT_SECONDARY_BASE,
        TSS_MINIMUM_LIMIT as u16,
        RIGHTS_TSS_386_BUSY,
    );

    // Populate secondary TSS with a complete resumable 386 image.
    let secondary_image = Tss386Image {
        backlink: 0,
        esp0: 0xFFF0,
        ss0: SELECTOR_RING0_STACK,
        eip: SECONDARY_TASK_EIP,
        eflags: 0x0000_0202,
        esp: 0xFF00,
        cs: SELECTOR_RING0_CODE,
        ss: SELECTOR_RING0_STACK,
        ds: super::setup::SELECTOR_RING0_DATA,
        es: super::setup::SELECTOR_RING0_DATA,
        ldt: 0,
        ..Tss386Image::default()
    };
    write_tss_386(bus, TASK_STATE_SEGMENT_SECONDARY_BASE, &secondary_image);

    // Primary TSS back-link points at secondary, current state has NT=1.
    write_word_at(
        bus,
        TASK_STATE_SEGMENT_BASE + TSS_OFFSET_LINK,
        SELECTOR_SECONDARY_TSS,
    );

    state
}

#[test]
fn iret_pm_nt_set_switches_via_back_link_to_busy_tss() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = pm_with_nt_secondary_tss_setup(&mut bus);
    state.flags.nt = true;
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[0xCF]);
    install_target_hlt(&mut bus, RING0_CODE_BASE, SECONDARY_TASK_EIP);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), SECONDARY_TASK_EIP + 1);
    assert_eq!(
        cpu.state.tr, SELECTOR_SECONDARY_TSS,
        "TR now points at the resumed task's TSS"
    );
}

/// 80486 PRM 6.8.3: the back-link read of an IRET with NT=1 is a privilege level 0
/// access. At CPL 3 with the TSS in a supervisor page the task switch takes no page fault.
#[test]
fn iret_pm_nt_at_cpl3_reads_back_link_from_supervisor_page() {
    let mut cpu = super::setup::make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = pm_with_nt_secondary_tss_setup(&mut bus);
    super::setup::promote_to_ring3(&mut state);
    state.flags.nt = true;
    super::setup::enable_user_identity_paging(
        &mut bus,
        &mut state,
        &[
            GLOBAL_DESCRIPTOR_TABLE_BASE,
            TASK_STATE_SEGMENT_BASE,
            TASK_STATE_SEGMENT_SECONDARY_BASE,
        ],
    );
    cpu.load_state(&state);

    place_at(&mut bus, super::setup::RING3_CODE_BASE, &[0xCF]);
    install_target_hlt(&mut bus, RING0_CODE_BASE, SECONDARY_TASK_EIP);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.tr, SELECTOR_SECONDARY_TSS);
    assert_eq!(cpu.ip(), SECONDARY_TASK_EIP);
}

#[test]
fn iret_pm_nt_back_link_to_non_busy_tss_raises_gp() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = pm_with_nt_secondary_tss_setup(&mut bus);
    // Mark secondary TSS as available (not busy). NT IRET requires busy.
    write_segment_descriptor_16bit(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        SELECTOR_SECONDARY_TSS >> 3,
        TASK_STATE_SEGMENT_SECONDARY_BASE,
        TSS_MINIMUM_LIMIT as u16,
        RIGHTS_TSS_386_AVAILABLE,
    );
    state.flags.nt = true;
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[0xCF]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn iret_pm_nt_back_link_to_non_tss_raises_gp() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = pm_with_nt_secondary_tss_setup(&mut bus);
    // Replace secondary descriptor with a code segment.
    write_segment_descriptor_16bit(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        SELECTOR_SECONDARY_TSS >> 3,
        RING0_CODE_BASE,
        0xFFFF,
        RIGHTS_RING0_CODE_READABLE_ACCESSED,
    );
    state.flags.nt = true;
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[0xCF]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn iret_pm_nt_back_link_with_ti_set_raises_invalid_tss() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = pm_with_nt_secondary_tss_setup(&mut bus);
    // Back-link into the LDT (TI=1) is invalid.
    write_word_at(
        &mut bus,
        TASK_STATE_SEGMENT_BASE + TSS_OFFSET_LINK,
        SELECTOR_SECONDARY_TSS | 0x0004,
    );
    state.flags.nt = true;
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[0xCF]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_INVALID_TSS_IP as u32 + 1);
}

#[test]
fn iret_pm_nt_set_clears_nt_after_switch() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = pm_with_nt_secondary_tss_setup(&mut bus);
    state.flags.nt = true;
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[0xCF]);
    install_target_hlt(&mut bus, RING0_CODE_BASE, SECONDARY_TASK_EIP);

    cpu.step(&mut bus);

    assert!(
        !cpu.state.flags.nt,
        "Old EFLAGS NT bit must be cleared when saved before switch"
    );
}

// PM IRET to VM86 (CPL=0 with EFLAGS.VM=1 in popped value).

#[test]
fn iret_pm_at_cpl0_with_eflags_vm_enters_vm86() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = make_pm_state_with_32bit_stack(&mut bus);
    cpu.load_state(&state);

    // 9-dword frame: EIP, CS, EFLAGS, ESP, SS, ES, DS, FS, GS.
    let target_eip: u32 = 0x0000_0100;
    let target_cs: u16 = 0x1000;
    let target_eflags: u32 = 0x0002_3202; // VM=1, IOPL=3, IF=1
    let target_esp: u32 = 0x0000_FF00;
    let target_ss: u16 = 0x2000;
    let target_es: u16 = 0x3000;
    let target_ds: u16 = 0x4000;
    let target_fs: u16 = 0x5000;
    let target_gs: u16 = 0x6000;

    let sp = cpu.esp();
    write_dword_at(&mut bus, RING0_STACK_BASE + sp, target_eip);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 4, target_cs as u32);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 8, target_eflags);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 12, target_esp);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 16, target_ss as u32);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 20, target_es as u32);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 24, target_ds as u32);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 28, target_fs as u32);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 32, target_gs as u32);

    place_at(&mut bus, RING0_CODE_BASE, &[0x66, 0xCF]);

    cpu.step(&mut bus);

    assert_ne!(
        cpu.state.eflags_upper & 0x0002_0000,
        0,
        "CPL=0 IRETD with VM=1 enters VM86"
    );
    assert_eq!(cpu.cs(), target_cs);
    assert_eq!(cpu.ip(), target_eip);
    assert_eq!(cpu.ss(), target_ss);
    assert_eq!(cpu.esp(), target_esp);
}

#[test]
fn iret_pm_at_cpl0_to_vm86_preserves_rf_until_next_instruction() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = make_pm_state_with_32bit_stack(&mut bus);
    state.set_eax(0x0000_4300);
    cpu.load_state(&state);

    let target_eip: u32 = 0x0000_0100;
    let target_cs: u16 = 0x1000;
    let target_eflags: u32 = 0x0003_3202; // VM=1, RF=1, IOPL=3, IF=1
    let target_esp: u32 = 0x0000_FF00;
    let target_ss: u16 = 0x2000;

    let sp = cpu.esp();
    write_dword_at(&mut bus, RING0_STACK_BASE + sp, target_eip);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 4, target_cs as u32);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 8, target_eflags);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 12, target_esp);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 16, target_ss as u32);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 20, 0x3000);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 24, 0x4000);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 28, 0x5000);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 32, 0x6000);

    place_at(&mut bus, RING0_CODE_BASE, &[0x66, 0xCF]);
    place_at(
        &mut bus,
        ((target_cs as u32) << 4) + target_eip,
        &[0x80, 0xFC, 0x43],
    );

    cpu.step(&mut bus);

    assert_ne!(
        cpu.state.eflags_upper & 0x0001_0000,
        0,
        "IRETD loads RF from the VM86 return frame"
    );
    assert_ne!(
        cpu.state.eflags_upper & 0x0002_0000,
        0,
        "IRETD enters VM86 before the guest instruction runs"
    );

    cpu.step(&mut bus);

    assert_eq!(cpu.ip(), target_eip + 3);
    assert_eq!(
        cpu.state.eflags_upper & 0x0001_0000,
        0,
        "the instruction after IRETD clears RF when it completes"
    );
}

#[test]
fn iret_pm_at_cpl0_to_vm86_loads_segment_caches_via_real_mode() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = make_pm_state_with_32bit_stack(&mut bus);
    cpu.load_state(&state);

    let target_es: u16 = 0x3000;
    let target_ds: u16 = 0x4000;
    let target_fs: u16 = 0x5000;
    let target_gs: u16 = 0x6000;

    let sp = cpu.esp();
    write_dword_at(&mut bus, RING0_STACK_BASE + sp, 0x100);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 4, 0x1000);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 8, 0x0002_0202);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 12, 0xFF00);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 16, 0x2000);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 20, target_es as u32);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 24, target_ds as u32);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 28, target_fs as u32);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 32, target_gs as u32);

    place_at(&mut bus, RING0_CODE_BASE, &[0x66, 0xCF]);
    cpu.step(&mut bus);

    assert_eq!(cpu.es(), target_es);
    assert_eq!(cpu.ds(), target_ds);
    assert_eq!(cpu.fs(), target_fs);
    assert_eq!(cpu.gs(), target_gs);
    // VM86 caches base = selector*16.
    assert_eq!(
        cpu.state.seg_bases[cpu::SegReg32::ES as usize],
        (target_es as u32) << 4
    );
    assert_eq!(
        cpu.state.seg_bases[cpu::SegReg32::DS as usize],
        (target_ds as u32) << 4
    );
    assert_eq!(
        cpu.state.seg_bases[cpu::SegReg32::FS as usize],
        (target_fs as u32) << 4
    );
    assert_eq!(
        cpu.state.seg_bases[cpu::SegReg32::GS as usize],
        (target_gs as u32) << 4
    );
}

#[test]
fn iret_pm_at_cpl0_to_vm86_keeps_eflags_vm_bit() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = make_pm_state_with_32bit_stack(&mut bus);
    cpu.load_state(&state);

    let sp = cpu.esp();
    write_dword_at(&mut bus, RING0_STACK_BASE + sp, 0x100);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 4, 0x1000);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 8, 0x0002_0202);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 12, 0xFF00);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 16, 0x2000);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 20, 0x3000);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 24, 0x4000);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 28, 0x5000);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 32, 0x6000);

    place_at(&mut bus, RING0_CODE_BASE, &[0x66, 0xCF]);
    cpu.step(&mut bus);

    assert_ne!(
        cpu.state.eflags_upper & 0x0002_0000,
        0,
        "EFLAGS.VM remains set after IRETD into VM86"
    );
}

#[test]
fn iret_pm_at_cpl0_to_vm86_uses_pl0_stack_for_9_dwords() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = make_pm_state_with_32bit_stack(&mut bus);
    cpu.load_state(&state);

    let initial_sp = cpu.esp();
    let sp = initial_sp;
    write_dword_at(&mut bus, RING0_STACK_BASE + sp, 0x0100);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 4, 0x1000);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 8, 0x0002_0202);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 12, 0xFF00);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 16, 0x2000);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 20, 0x3000);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 24, 0x4000);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 28, 0x5000);
    write_dword_at(&mut bus, RING0_STACK_BASE + sp + 32, 0x6000);

    place_at(&mut bus, RING0_CODE_BASE, &[0x66, 0xCF]);
    cpu.step(&mut bus);

    // After dispatch, ESP comes from popped image, which was 0xFF00.
    assert_eq!(cpu.esp(), 0xFF00);
    assert_eq!(cpu.ss(), 0x2000);
}

// VM86 IRET tests.

#[test]
fn iret_vm86_iopl_3_pops_ip_cs_flags_16bit() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_vm86(&mut bus);
    cpu.load_state(&state);

    let target_ip: u16 = 0x0500;
    let target_cs: u16 = 0x7000;
    let sp = cpu.esp();
    let ss_base = state.seg_bases[cpu::SegReg32::SS as usize];
    write_word_at(&mut bus, ss_base + sp, target_ip);
    write_word_at(&mut bus, ss_base + sp + 2, target_cs);
    write_word_at(&mut bus, ss_base + sp + 4, 0x0202);

    place_at(
        &mut bus,
        state.seg_bases[cpu::SegReg32::CS as usize],
        &[0xCF],
    );

    cpu.step(&mut bus);

    assert_eq!(cpu.cs(), target_cs);
    assert_eq!(cpu.ip() as u16, target_ip);
}

#[test]
fn iret_vm86_iopl_3_pops_eip_cs_eflags_for_iretd() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_vm86(&mut bus);
    cpu.load_state(&state);

    let target_eip: u32 = 0x0000_0500;
    let target_cs: u16 = 0x7000;
    let sp = cpu.esp();
    let ss_base = state.seg_bases[cpu::SegReg32::SS as usize];
    write_dword_at(&mut bus, ss_base + sp, target_eip);
    write_dword_at(&mut bus, ss_base + sp + 4, target_cs as u32);
    write_dword_at(&mut bus, ss_base + sp + 8, 0x0000_0202);

    place_at(
        &mut bus,
        state.seg_bases[cpu::SegReg32::CS as usize],
        &[0x66, 0xCF],
    );

    cpu.step(&mut bus);

    assert_eq!(cpu.cs(), target_cs);
    assert_eq!(cpu.ip(), target_eip);
}

#[test]
fn iret_vm86_iopl_2_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_vm86_with_iopl(&mut bus, 2);
    super::setup::write_interrupt_gate_386(
        &mut bus,
        super::setup::INTERRUPT_DESCRIPTOR_TABLE_BASE,
        13,
        HANDLER_GENERAL_PROTECTION_IP as u32,
        SELECTOR_RING0_CODE,
        0,
    );
    bus.ram[(RING0_CODE_BASE + HANDLER_GENERAL_PROTECTION_IP as u32) as usize] = 0xF4;
    state.gdt_limit = 5 * 8 - 1;
    cpu.load_state(&state);

    place_at(
        &mut bus,
        state.seg_bases[cpu::SegReg32::CS as usize],
        &[0xCF],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn iret_vm86_iopl_0_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_vm86_with_iopl(&mut bus, 0);
    super::setup::write_interrupt_gate_386(
        &mut bus,
        super::setup::INTERRUPT_DESCRIPTOR_TABLE_BASE,
        13,
        HANDLER_GENERAL_PROTECTION_IP as u32,
        SELECTOR_RING0_CODE,
        0,
    );
    bus.ram[(RING0_CODE_BASE + HANDLER_GENERAL_PROTECTION_IP as u32) as usize] = 0xF4;
    state.gdt_limit = 5 * 8 - 1;
    cpu.load_state(&state);

    place_at(
        &mut bus,
        state.seg_bases[cpu::SegReg32::CS as usize],
        &[0xCF],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn iret_vm86_iretd_at_iopl_3_preserves_ip_upper_zero() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_vm86(&mut bus);
    cpu.load_state(&state);

    let target_eip: u32 = 0x0000_3456;
    let sp = cpu.esp();
    let ss_base = state.seg_bases[cpu::SegReg32::SS as usize];
    write_dword_at(&mut bus, ss_base + sp, target_eip);
    write_dword_at(&mut bus, ss_base + sp + 4, 0x7000);
    write_dword_at(&mut bus, ss_base + sp + 8, 0x0000_0202);

    place_at(
        &mut bus,
        state.seg_bases[cpu::SegReg32::CS as usize],
        &[0x66, 0xCF],
    );

    cpu.step(&mut bus);

    assert_eq!(cpu.ip(), target_eip);
    // IP upper bits zero - target_eip fits in 16 bits.
    assert_eq!(cpu.state.ip_upper, 0);
}

#[test]
fn iret_vm86_iret_keeps_ip_upper_zero() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_vm86(&mut bus);
    cpu.load_state(&state);

    let target_ip: u16 = 0x0010;
    let sp = cpu.esp();
    let ss_base = state.seg_bases[cpu::SegReg32::SS as usize];
    write_word_at(&mut bus, ss_base + sp, target_ip);
    write_word_at(&mut bus, ss_base + sp + 2, 0x7000);
    write_word_at(&mut bus, ss_base + sp + 4, 0x0202);

    place_at(
        &mut bus,
        state.seg_bases[cpu::SegReg32::CS as usize],
        &[0xCF],
    );

    cpu.step(&mut bus);

    assert_eq!(cpu.ip(), target_ip as u32);
    assert_eq!(
        cpu.state.ip_upper, 0,
        "16-bit IRET keeps IP upper bits zero"
    );
}

#[test]
fn iret_vm86_iopl_3_preserves_iopl_in_eflags() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_vm86(&mut bus);
    cpu.load_state(&state);

    let sp = cpu.esp();
    let ss_base = state.seg_bases[cpu::SegReg32::SS as usize];
    // Pop EFLAGS image with IOPL=0; in VM86, IOPL must NOT change because
    // VM86 is effectively at CPL=3 with respect to flag rules.
    write_word_at(&mut bus, ss_base + sp, 0x0010);
    write_word_at(&mut bus, ss_base + sp + 2, 0x7000);
    write_word_at(&mut bus, ss_base + sp + 4, 0x0002);

    place_at(
        &mut bus,
        state.seg_bases[cpu::SegReg32::CS as usize],
        &[0xCF],
    );

    cpu.step(&mut bus);

    assert_eq!(
        cpu.state.flags.iopl, 3,
        "VM86 IRET cannot lower IOPL from popped image"
    );
}

#[test]
fn iret_vm86_iret_keeps_eflags_vm_bit_set() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_vm86(&mut bus);
    cpu.load_state(&state);

    let sp = cpu.esp();
    let ss_base = state.seg_bases[cpu::SegReg32::SS as usize];
    write_word_at(&mut bus, ss_base + sp, 0x0010);
    write_word_at(&mut bus, ss_base + sp + 2, 0x7000);
    write_word_at(&mut bus, ss_base + sp + 4, 0x0202);

    place_at(
        &mut bus,
        state.seg_bases[cpu::SegReg32::CS as usize],
        &[0xCF],
    );

    cpu.step(&mut bus);

    assert_ne!(
        cpu.state.eflags_upper & 0x0002_0000,
        0,
        "VM86 IRET stays in VM86 (cannot exit)"
    );
}

// Real-mode IRET tests.

fn make_real_mode_state_for_iret() -> I386State {
    let mut state = I386State::default();
    state.set_cs(0x1000);
    state.seg_bases[cpu::SegReg32::CS as usize] = 0x0001_0000;
    state.set_ss(0x2000);
    state.seg_bases[cpu::SegReg32::SS as usize] = 0x0002_0000;
    state.set_esp(0x1000);
    state.seg_limits = [0xFFFF; 6];
    state.seg_rights[cpu::SegReg32::CS as usize] = RIGHTS_RING0_CODE_READABLE_ACCESSED;
    state.seg_rights[cpu::SegReg32::SS as usize] = RIGHTS_RING0_DATA_WRITABLE_ACCESSED;
    state.seg_valid = [true; 6];
    state
}

#[test]
fn iret_real_mode_pops_three_words() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = make_real_mode_state_for_iret();
    cpu.load_state(&state);

    let target_ip: u16 = 0x0500;
    let target_cs: u16 = 0x3000;
    let sp = state.esp();
    write_word_at(&mut bus, 0x0002_0000 + sp, target_ip);
    write_word_at(&mut bus, 0x0002_0000 + sp + 2, target_cs);
    write_word_at(&mut bus, 0x0002_0000 + sp + 4, 0x0202);

    place_at(&mut bus, 0x0001_0000, &[0xCF]);

    cpu.step(&mut bus);

    assert_eq!(cpu.cs(), target_cs);
    assert_eq!(cpu.ip() as u16, target_ip);
}

#[test]
fn iret_real_mode_increments_sp_by_six() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = make_real_mode_state_for_iret();
    cpu.load_state(&state);

    let initial_sp = state.esp();
    write_word_at(&mut bus, 0x0002_0000 + initial_sp, 0x0500);
    write_word_at(&mut bus, 0x0002_0000 + initial_sp + 2, 0x3000);
    write_word_at(&mut bus, 0x0002_0000 + initial_sp + 4, 0x0202);

    place_at(&mut bus, 0x0001_0000, &[0xCF]);

    cpu.step(&mut bus);

    assert_eq!(
        cpu.esp() & 0xFFFF,
        (initial_sp + 6) & 0xFFFF,
        "real-mode 16-bit IRET pops three words"
    );
}

#[test]
fn iret_real_mode_iretd_pops_three_dwords() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = make_real_mode_state_for_iret();
    state.seg_granularity[cpu::SegReg32::SS as usize] = 0x40;
    state.seg_limits[cpu::SegReg32::SS as usize] = 0xFFFF_FFFF;
    cpu.load_state(&state);

    let target_eip: u32 = 0x0000_0500;
    let target_cs: u16 = 0x3000;
    let initial_sp = state.esp();
    write_dword_at(&mut bus, 0x0002_0000 + initial_sp, target_eip);
    write_dword_at(&mut bus, 0x0002_0000 + initial_sp + 4, target_cs as u32);
    write_dword_at(&mut bus, 0x0002_0000 + initial_sp + 8, 0x0000_0202);

    place_at(&mut bus, 0x0001_0000, &[0x66, 0xCF]);

    cpu.step(&mut bus);

    assert_eq!(cpu.cs(), target_cs);
    assert_eq!(cpu.ip(), target_eip);
    assert_eq!(cpu.esp(), initial_sp + 12);
}

#[test]
fn iret_real_mode_iret_keeps_ip_upper_zero() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = make_real_mode_state_for_iret();
    cpu.load_state(&state);

    write_word_at(&mut bus, 0x0002_0000 + 0x1000, 0x0500);
    write_word_at(&mut bus, 0x0002_0000 + 0x1002, 0x3000);
    write_word_at(&mut bus, 0x0002_0000 + 0x1004, 0x0202);

    place_at(&mut bus, 0x0001_0000, &[0xCF]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.ip_upper, 0);
}

#[test]
fn iret_real_mode_iretd_loads_full_eip_upper() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = make_real_mode_state_for_iret();
    state.seg_granularity[cpu::SegReg32::SS as usize] = 0x40;
    state.seg_limits[cpu::SegReg32::SS as usize] = 0xFFFF_FFFF;
    cpu.load_state(&state);

    let target_eip: u32 = 0x1234_5678;
    write_dword_at(&mut bus, 0x0002_0000 + 0x1000, target_eip);
    write_dword_at(&mut bus, 0x0002_0000 + 0x1004, 0x3000);
    write_dword_at(&mut bus, 0x0002_0000 + 0x1008, 0x0000_0202);

    place_at(&mut bus, 0x0001_0000, &[0x66, 0xCF]);

    cpu.step(&mut bus);

    assert_eq!(cpu.ip(), target_eip);
}

#[test]
fn iret_real_mode_loads_eflags_lower_16_bits() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = make_real_mode_state_for_iret();
    cpu.load_state(&state);

    write_word_at(&mut bus, 0x0002_0000 + 0x1000, 0x0500);
    write_word_at(&mut bus, 0x0002_0000 + 0x1002, 0x3000);
    // EFLAGS image with CF=1, ZF=1, IF=1. Bit 1 always 1.
    write_word_at(&mut bus, 0x0002_0000 + 0x1004, 0x0243);

    place_at(&mut bus, 0x0001_0000, &[0xCF]);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.cf());
    assert!(cpu.state.flags.zf());
    assert!(cpu.state.flags.if_flag);
}

// Sanity checks on the test helpers themselves.

#[test]
fn iret_helpers_make_pm_state_with_32bit_stack_marks_ss_granularity() {
    let mut bus = TestBus::new();
    let state = make_pm_state_with_32bit_stack(&mut bus);
    assert_eq!(state.seg_granularity[cpu::SegReg32::SS as usize], 0x40);
    assert_eq!(state.seg_limits[cpu::SegReg32::SS as usize], 0xFFFF_FFFF);
}

#[test]
fn iret_helpers_secondary_tss_is_busy_after_setup() {
    let mut bus = TestBus::new();
    let _state = pm_with_nt_secondary_tss_setup(&mut bus);
    let descriptor_address =
        (GLOBAL_DESCRIPTOR_TABLE_BASE + (SELECTOR_SECONDARY_TSS as u32 / 8) * 8) as usize;
    assert_eq!(
        bus.ram[descriptor_address + 5],
        RIGHTS_TSS_386_BUSY,
        "secondary TSS must be busy for NT IRET tests"
    );
}

#[test]
fn iret_helpers_back_link_points_at_secondary_tss() {
    let mut bus = TestBus::new();
    let _state = pm_with_nt_secondary_tss_setup(&mut bus);
    let backlink = read_word_at(&bus, TASK_STATE_SEGMENT_BASE + TSS_OFFSET_LINK);
    assert_eq!(backlink, SELECTOR_SECONDARY_TSS);
}

#[test]
fn iret_helpers_secondary_tss_eip_persists() {
    let mut bus = TestBus::new();
    let _state = pm_with_nt_secondary_tss_setup(&mut bus);
    let stored_eip = read_dword_at(
        &bus,
        TASK_STATE_SEGMENT_SECONDARY_BASE + super::setup::TSS_OFFSET_EIP,
    );
    assert_eq!(stored_eip, SECONDARY_TASK_EIP);
}
