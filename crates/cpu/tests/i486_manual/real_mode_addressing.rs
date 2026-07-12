//! Real-Address Mode addressing/limit edge cases.
//!
//! 80486 PRM Chapter 22 ("Real-Address Mode") differences from 8086:
//!   - Sequential execution past offset 65,535 raises #GP (item 8).
//!   - INT vector beyond IDTR limit raises #DF / shutdown (Table 22-2 / 22.6).
//!   - LIDT controls both base and limit (22.5); reset defaults base=0,
//!     limit=0x3FF (item, 22.5).

use common::Cpu as _;
use cpu::I386State;

use super::setup::{
    REAL_MODE_HANDLER_DOUBLE_FAULT_OFFSET, REAL_MODE_HANDLER_GENERAL_PROTECTION_OFFSET,
    REAL_MODE_HANDLER_SEGMENT, RIGHTS_RING0_CODE_READABLE_ACCESSED,
    RIGHTS_RING0_DATA_WRITABLE_ACCESSED, TestBus, install_real_mode_ivt_entry, make_cpu_486,
    place_at, setup_real_mode_with_ivt_handlers,
};

const HLT_OPCODE: u8 = 0xF4;
const NOP_OPCODE: u8 = 0x90;

const TEST_CS_SELECTOR: u16 = 0xF000;
const TEST_CS_BASE: u32 = 0x000F_0000;
const TEST_DS_SELECTOR: u16 = 0x1000;
const TEST_DS_BASE: u32 = 0x0001_0000;
const TEST_SS_SELECTOR: u16 = 0x3000;
const TEST_SS_BASE: u32 = 0x0003_0000;

fn place_in_test_code(bus: &mut TestBus, offset: u16, code: &[u8]) {
    place_at(bus, TEST_CS_BASE + offset as u32, code);
}

fn assert_general_protection_dispatched(cpu: &cpu::I386<{ cpu::CPU_MODEL_486_DX }>) {
    assert!(cpu.halted(), "expected #GP handler HLT");
    assert_eq!(cpu.cs(), REAL_MODE_HANDLER_SEGMENT);
    assert_eq!(
        cpu.ip() as u16,
        REAL_MODE_HANDLER_GENERAL_PROTECTION_OFFSET + 1
    );
}

fn assert_double_fault_dispatched(cpu: &cpu::I386<{ cpu::CPU_MODEL_486_DX }>) {
    assert!(cpu.halted(), "expected #DF handler HLT");
    assert_eq!(cpu.cs(), REAL_MODE_HANDLER_SEGMENT);
    assert_eq!(cpu.ip() as u16, REAL_MODE_HANDLER_DOUBLE_FAULT_OFFSET + 1);
}

fn make_real_mode_state_for_addressing_tests() -> I386State {
    let mut state = I386State::default();
    state.set_cs(TEST_CS_SELECTOR);
    state.seg_bases[cpu::SegReg32::CS as usize] = TEST_CS_BASE;
    state.set_ds(TEST_DS_SELECTOR);
    state.seg_bases[cpu::SegReg32::DS as usize] = TEST_DS_BASE;
    state.set_ss(TEST_SS_SELECTOR);
    state.seg_bases[cpu::SegReg32::SS as usize] = TEST_SS_BASE;
    state.set_esp(0x1000);
    state.seg_limits = [0xFFFF; 6];
    state.seg_rights[cpu::SegReg32::CS as usize] = RIGHTS_RING0_CODE_READABLE_ACCESSED;
    state.seg_rights[cpu::SegReg32::DS as usize] = RIGHTS_RING0_DATA_WRITABLE_ACCESSED;
    state.seg_rights[cpu::SegReg32::ES as usize] = RIGHTS_RING0_DATA_WRITABLE_ACCESSED;
    state.seg_rights[cpu::SegReg32::SS as usize] = RIGHTS_RING0_DATA_WRITABLE_ACCESSED;
    state.seg_valid = [true; 6];
    state.idt_base = 0;
    state.idt_limit = 0x03FF;
    state
}

#[test]
fn real_mode_sequential_execution_past_offset_ffff_raises_general_protection() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let _ = setup_real_mode_with_ivt_handlers(&mut bus);
    let mut state = make_real_mode_state_for_addressing_tests();
    state.ip = 0xFFFF;
    cpu.load_state(&state);

    // NOP at 0xFFFF; after execution IP advances to 0x10000 which crosses
    // the segment limit. The next fetch must raise #GP instead of fetching
    // from offset 0 (8086 behaviour).
    place_in_test_code(&mut bus, 0xFFFF, &[NOP_OPCODE]);
    place_in_test_code(&mut bus, 0x0000, &[HLT_OPCODE]);

    cpu.step(&mut bus); // execute NOP at 0xFFFF
    cpu.step(&mut bus); // fetch at 0x10000 raises #GP and dispatches handler
    cpu.step(&mut bus); // execute the handler's HLT

    assert_general_protection_dispatched(&cpu);
}

#[test]
fn real_mode_int_n_with_vector_beyond_idtr_limit_raises_double_fault() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let _ = setup_real_mode_with_ivt_handlers(&mut bus);
    install_real_mode_ivt_entry(
        &mut bus,
        8,
        REAL_MODE_HANDLER_SEGMENT,
        REAL_MODE_HANDLER_DOUBLE_FAULT_OFFSET,
    );
    let mut state = make_real_mode_state_for_addressing_tests();
    // IDTR limit covers vectors 0..15 only (16 entries * 4 bytes - 1 = 63).
    state.idt_limit = 63;
    cpu.load_state(&state);

    // INT 0x40 = vector 64; entry at byte 256, far beyond limit 63.
    place_in_test_code(&mut bus, 0x0000, &[0xCD, 0x40]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_double_fault_dispatched(&cpu);
}

#[test]
fn real_mode_lidt_loads_both_base_and_limit() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let _ = setup_real_mode_with_ivt_handlers(&mut bus);
    let state = make_real_mode_state_for_addressing_tests();
    cpu.load_state(&state);

    // Build the IDT image at DS:[0x0000]: limit=0x01FF, base=0x0001_2345.
    let idt_image_offset: u16 = 0x0000;
    let idt_image_address = TEST_DS_BASE + idt_image_offset as u32;
    bus.ram[idt_image_address as usize] = 0xFF;
    bus.ram[idt_image_address as usize + 1] = 0x01;
    bus.ram[idt_image_address as usize + 2] = 0x45;
    bus.ram[idt_image_address as usize + 3] = 0x23;
    bus.ram[idt_image_address as usize + 4] = 0x01;
    bus.ram[idt_image_address as usize + 5] = 0x00;

    // LIDT m16:24/32 = 0x0F 0x01 /3. ModR/M=0x1E disp16 -> [DS:disp16].
    place_in_test_code(
        &mut bus,
        0x0000,
        &[
            0x0F,
            0x01,
            0x1E,
            idt_image_offset as u8,
            (idt_image_offset >> 8) as u8,
            HLT_OPCODE,
        ],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    // Real-mode default LIDT loads only the low 24 bits of the base; verify
    // base AND limit both updated.
    assert_eq!(cpu.state.idt_limit, 0x01FF);
    assert_eq!(cpu.state.idt_base, 0x0001_2345);
}

#[test]
fn real_mode_default_idtr_base_is_zero_and_limit_is_3ff_after_reset() {
    let cpu = make_cpu_486();

    // The 80486 PRM (22.5) specifies that real-mode IDTR is initialised to
    // base=0, limit=0x03FF on reset. The default I386 state mirrors this.
    assert_eq!(cpu.state.idt_base, 0);
    assert_eq!(cpu.state.idt_limit, 0x03FF);
}

#[test]
fn real_mode_word_push_at_sp_zero_wraps_sp_without_fault() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let _ = setup_real_mode_with_ivt_handlers(&mut bus);
    let mut state = make_real_mode_state_for_addressing_tests();
    state.set_esp(0x0000);
    state.set_eax(0x1234);
    cpu.load_state(&state);

    // PUSH AX with SP=0: SP decrements (16-bit wrap) to 0xFFFE and the word
    // write hits offsets 0xFFFE and 0xFFFF, both still within the 64K
    // segment limit. 80286 PRM 8 documents that only PUSHA/POPA wrap
    // raises #SS in real mode (saved SP one of 0x0000, 0x0001, 0xFFFE,
    // 0xFFFF); a regular PUSH/POP simply wraps SP without faulting.
    place_in_test_code(&mut bus, 0x0000, &[0x50, HLT_OPCODE]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.esp() as u16, 0xFFFE);
    assert_eq!(
        super::setup::read_word_at(&bus, TEST_SS_BASE + 0xFFFE),
        0x1234
    );
    assert!(!cpu.halted());
}
