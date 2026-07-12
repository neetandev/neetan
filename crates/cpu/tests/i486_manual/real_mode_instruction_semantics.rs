//! Real-mode instruction-behavior edge cases.
//!
//! 80486 PRM Chapter 22 quirks that are about instruction semantics rather
//! than addressing/limit faults:
//!   - DIV/IDIV exception CS:IP points to the faulting instruction (item 2).
//!   - IDIV permits 0x80 / 0x8000 / 0x80000000 quotients (item 11).
//!   - LOCK prefix is restricted to a fixed instruction whitelist with a
//!     memory destination; misuse raises #UD (item 9 and 22.7.2).
//!   - Instruction encoded with more than 15 bytes raises #GP (item 6).
//!   - PUSH SP pushes the value of SP BEFORE the decrement (item 4).
//!   - Shift / rotate counts are masked to the low 5 bits (item 5).

use common::Cpu as _;
use cpu::I386State;

use super::setup::{
    REAL_MODE_HANDLER_DIVIDE_ERROR_OFFSET, REAL_MODE_HANDLER_GENERAL_PROTECTION_OFFSET,
    REAL_MODE_HANDLER_INVALID_OPCODE_OFFSET, REAL_MODE_HANDLER_SEGMENT,
    RIGHTS_RING0_CODE_READABLE_ACCESSED, RIGHTS_RING0_DATA_WRITABLE_ACCESSED, TestBus,
    make_cpu_486, place_at, read_word_at, setup_real_mode_with_ivt_handlers,
};

const HLT_OPCODE: u8 = 0xF4;
const NOP_OPCODE: u8 = 0x90;
const SS_OVERRIDE_PREFIX: u8 = 0x36;

const TEST_CS_SELECTOR: u16 = 0xF000;
const TEST_CS_BASE: u32 = 0x000F_0000;
const TEST_DS_SELECTOR: u16 = 0x1000;
const TEST_DS_BASE: u32 = 0x0001_0000;
const TEST_SS_SELECTOR: u16 = 0x3000;
const TEST_SS_BASE: u32 = 0x0003_0000;
const TEST_INITIAL_SP: u16 = 0x1000;

fn place_in_test_code(bus: &mut TestBus, offset: u16, code: &[u8]) {
    place_at(bus, TEST_CS_BASE + offset as u32, code);
}

fn make_real_mode_state_for_semantics() -> I386State {
    let mut state = I386State::default();
    state.set_cs(TEST_CS_SELECTOR);
    state.seg_bases[cpu::SegReg32::CS as usize] = TEST_CS_BASE;
    state.set_ds(TEST_DS_SELECTOR);
    state.seg_bases[cpu::SegReg32::DS as usize] = TEST_DS_BASE;
    state.set_ss(TEST_SS_SELECTOR);
    state.seg_bases[cpu::SegReg32::SS as usize] = TEST_SS_BASE;
    state.set_esp(TEST_INITIAL_SP as u32);
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

fn assert_invalid_opcode_dispatched(cpu: &cpu::I386<{ cpu::CPU_MODEL_486_DX }>) {
    assert!(cpu.halted(), "expected #UD handler HLT");
    assert_eq!(cpu.cs(), REAL_MODE_HANDLER_SEGMENT);
    assert_eq!(cpu.ip() as u16, REAL_MODE_HANDLER_INVALID_OPCODE_OFFSET + 1);
}

fn assert_general_protection_dispatched(cpu: &cpu::I386<{ cpu::CPU_MODEL_486_DX }>) {
    assert!(cpu.halted(), "expected #GP handler HLT");
    assert_eq!(cpu.cs(), REAL_MODE_HANDLER_SEGMENT);
    assert_eq!(
        cpu.ip() as u16,
        REAL_MODE_HANDLER_GENERAL_PROTECTION_OFFSET + 1
    );
}

fn read_real_mode_interrupt_frame_return_ip(bus: &TestBus, sp_after_fault: u16) -> u16 {
    // Real-mode interrupt pushes FLAGS, CS, IP; SP points at IP.
    read_word_at(bus, TEST_SS_BASE + sp_after_fault as u32)
}

#[test]
fn real_mode_div_byte_by_zero_csip_points_to_div_instruction() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let _ = setup_real_mode_with_ivt_handlers(&mut bus);
    let mut state = make_real_mode_state_for_semantics();
    state.set_eax(0x0010);
    state.set_ebx(0x0000);
    cpu.load_state(&state);

    // DIV BL = 0xF6 /6, ModR/M = 0xF3.
    let div_offset: u16 = 0x0040;
    place_in_test_code(&mut bus, div_offset, &[0xF6, 0xF3]);
    cpu.state.ip = div_offset;

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted(), "expected #DE handler HLT");
    assert_eq!(cpu.cs(), REAL_MODE_HANDLER_SEGMENT);
    assert_eq!(cpu.ip() as u16, REAL_MODE_HANDLER_DIVIDE_ERROR_OFFSET + 1);

    let saved_ip = read_real_mode_interrupt_frame_return_ip(&bus, cpu.state.esp() as u16);
    assert_eq!(
        saved_ip, div_offset,
        "saved CS:IP must point to the faulting DIV, not past it"
    );
}

#[test]
fn real_mode_idiv_byte_overflow_csip_points_to_idiv_instruction() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let _ = setup_real_mode_with_ivt_handlers(&mut bus);
    let mut state = make_real_mode_state_for_semantics();
    // AX = 0x7FFF, BL = 1: signed quotient 32767, does not fit in i8.
    state.set_eax(0x7FFF);
    state.set_ebx(0x0001);
    cpu.load_state(&state);

    let idiv_offset: u16 = 0x0040;
    place_in_test_code(&mut bus, idiv_offset, &[0xF6, 0xFB]);
    cpu.state.ip = idiv_offset;

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip() as u16, REAL_MODE_HANDLER_DIVIDE_ERROR_OFFSET + 1);

    let saved_ip = read_real_mode_interrupt_frame_return_ip(&bus, cpu.state.esp() as u16);
    assert_eq!(saved_ip, idiv_offset);
}

#[test]
fn real_mode_idiv_word_quotient_0x8000_succeeds_on_i486() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let _ = setup_real_mode_with_ivt_handlers(&mut bus);
    let mut state = make_real_mode_state_for_semantics();
    // DX:AX = 0xFFFF_8000 (signed -32768), BX = 1.
    // Signed quotient: -32768 / 1 = -32768 = 0x8000 (fits in i16 on i486).
    state.set_edx(0xFFFF);
    state.set_eax(0x8000);
    state.set_ebx(0x0001);
    cpu.load_state(&state);

    // F7 FB = IDIV BX (16-bit operand size by real-mode default).
    place_in_test_code(&mut bus, 0x0000, &[0xF7, 0xFB, HLT_OPCODE]);

    cpu.step(&mut bus);

    assert!(!cpu.halted(), "i486 must NOT fault on 0x8000 quotient");
    assert_eq!((cpu.state.eax() & 0xFFFF) as u16, 0x8000);
}

#[test]
fn real_mode_idiv_dword_quotient_0x80000000_succeeds_on_i486() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let _ = setup_real_mode_with_ivt_handlers(&mut bus);
    let mut state = make_real_mode_state_for_semantics();
    // EDX:EAX = 0x80000000_00000000, EBX = 0x0000_0001_0000_0000 won't fit
    // in 32 bits. Use a simpler 0x80000000 in EAX with EDX = 0xFFFFFFFF:
    // signed dividend = -2^31; divide by 1 -> quotient = -2^31 = 0x80000000.
    state.set_edx(0xFFFF_FFFF);
    state.set_eax(0x8000_0000);
    state.set_ebx(0x0000_0001);
    cpu.load_state(&state);

    // 66 F7 FB = IDIV EBX (66H promotes operand to 32-bit in real mode).
    place_in_test_code(&mut bus, 0x0000, &[0x66, 0xF7, 0xFB, HLT_OPCODE]);

    cpu.step(&mut bus);

    assert!(!cpu.halted(), "i486 must NOT fault on 0x80000000 quotient");
    assert_eq!(cpu.state.eax(), 0x8000_0000);
}

#[test]
fn real_mode_idiv_byte_quotient_0x80_succeeds_on_i486() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let _ = setup_real_mode_with_ivt_handlers(&mut bus);
    let mut state = make_real_mode_state_for_semantics();
    // AX = 0xFF80 (signed -128), BL = 1: quotient -128 = 0x80, fits in i8.
    state.set_eax(0xFF80);
    state.set_ebx(0x0001);
    cpu.load_state(&state);

    place_in_test_code(&mut bus, 0x0000, &[0xF6, 0xFB, HLT_OPCODE]);

    cpu.step(&mut bus);

    assert!(!cpu.halted(), "i486 must NOT fault on 0x80 quotient");
    assert_eq!(cpu.state.eax() as u8, 0x80);
}

#[test]
fn real_mode_lock_prefix_on_mov_register_to_register_raises_invalid_opcode() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let _ = setup_real_mode_with_ivt_handlers(&mut bus);
    let state = make_real_mode_state_for_semantics();
    cpu.load_state(&state);

    // LOCK MOV AX, BX = F0 89 D8 (89 /r is MOV r/m, r; ModR/M D8 = mod=11,
    // r/m=AX, r=BX -> register-only destination, which is illegal under LOCK).
    place_in_test_code(&mut bus, 0x0000, &[0xF0, 0x89, 0xD8]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_invalid_opcode_dispatched(&cpu);
}

#[test]
fn real_mode_lock_prefix_on_add_with_register_destination_raises_invalid_opcode() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let _ = setup_real_mode_with_ivt_handlers(&mut bus);
    let state = make_real_mode_state_for_semantics();
    cpu.load_state(&state);

    // LOCK ADD AX, BX = F0 01 D8 (ADD is on the LOCK whitelist, but the
    // destination must be memory; reg-reg form is illegal).
    place_in_test_code(&mut bus, 0x0000, &[0xF0, 0x01, 0xD8]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_invalid_opcode_dispatched(&cpu);
}

#[test]
fn real_mode_lock_prefix_on_inc_register_raises_invalid_opcode() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let _ = setup_real_mode_with_ivt_handlers(&mut bus);
    let state = make_real_mode_state_for_semantics();
    cpu.load_state(&state);

    // LOCK INC AX = F0 40. Even though INC is on the whitelist, the short
    // form 40+rw uses a register, not memory.
    place_in_test_code(&mut bus, 0x0000, &[0xF0, 0x40]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_invalid_opcode_dispatched(&cpu);
}

#[test]
fn real_mode_lock_prefix_on_disallowed_opcode_nop_raises_invalid_opcode() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let _ = setup_real_mode_with_ivt_handlers(&mut bus);
    let state = make_real_mode_state_for_semantics();
    cpu.load_state(&state);

    // LOCK NOP = F0 90. NOP is not on the LOCK whitelist at all.
    place_in_test_code(&mut bus, 0x0000, &[0xF0, NOP_OPCODE]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_invalid_opcode_dispatched(&cpu);
}

#[test]
fn real_mode_instruction_with_15_legal_prefix_bytes_executes() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let _ = setup_real_mode_with_ivt_handlers(&mut bus);
    let state = make_real_mode_state_for_semantics();
    cpu.load_state(&state);

    let mut encoding = vec![SS_OVERRIDE_PREFIX; 14];
    encoding.push(NOP_OPCODE);
    place_in_test_code(&mut bus, 0x0000, &encoding);
    place_in_test_code(&mut bus, encoding.len() as u16, &[HLT_OPCODE]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(
        cpu.halted(),
        "15-byte instruction should execute and reach the trailing HLT"
    );
    assert_eq!(cpu.ip() as u16, encoding.len() as u16 + 1);
}

#[test]
fn real_mode_instruction_exceeding_15_bytes_raises_general_protection() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let _ = setup_real_mode_with_ivt_handlers(&mut bus);
    let state = make_real_mode_state_for_semantics();
    cpu.load_state(&state);

    // 15 prefixes + NOP = 16 bytes total.
    let mut encoding = vec![SS_OVERRIDE_PREFIX; 15];
    encoding.push(NOP_OPCODE);
    place_in_test_code(&mut bus, 0x0000, &encoding);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_dispatched(&cpu);
}

#[test]
fn real_mode_push_sp_pushes_pre_decrement_value() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let _ = setup_real_mode_with_ivt_handlers(&mut bus);
    let mut state = make_real_mode_state_for_semantics();
    state.set_esp(TEST_INITIAL_SP as u32);
    cpu.load_state(&state);

    // PUSH SP = 0x54 (16-bit operand by default in real mode).
    place_in_test_code(&mut bus, 0x0000, &[0x54, HLT_OPCODE]);

    cpu.step(&mut bus);

    let post_sp = cpu.state.esp() as u16;
    let pushed_value = read_word_at(&bus, TEST_SS_BASE + post_sp as u32);
    assert_eq!(post_sp, TEST_INITIAL_SP - 2);
    assert_eq!(
        pushed_value, TEST_INITIAL_SP,
        "i486 PUSH SP pushes pre-decrement value"
    );
}

#[test]
fn real_mode_shl_word_count_33_masks_to_1() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let _ = setup_real_mode_with_ivt_handlers(&mut bus);
    let mut state = make_real_mode_state_for_semantics();
    state.set_eax(0x0000_1234);
    state.set_ecx(0x0000_0021); // 33
    cpu.load_state(&state);

    // SHL AX, CL = D3 E0 (16-bit operand by default in real mode).
    place_in_test_code(&mut bus, 0x0000, &[0xD3, 0xE0, HLT_OPCODE]);

    cpu.step(&mut bus);

    assert_eq!((cpu.state.eax() & 0xFFFF) as u16, 0x2468);
}

#[test]
fn real_mode_shr_word_count_32_masks_to_0_no_change() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let _ = setup_real_mode_with_ivt_handlers(&mut bus);
    let mut state = make_real_mode_state_for_semantics();
    state.set_eax(0x0000_F00F);
    state.set_ecx(0x0000_0020); // 32
    cpu.load_state(&state);

    // SHR AX, CL = D3 E8.
    place_in_test_code(&mut bus, 0x0000, &[0xD3, 0xE8, HLT_OPCODE]);

    cpu.step(&mut bus);

    // 32 & 0x1F = 0; no shift, AX unchanged.
    assert_eq!((cpu.state.eax() & 0xFFFF) as u16, 0xF00F);
}

#[test]
fn real_mode_sar_dword_count_with_high_bits_set_uses_low_5_bits() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let _ = setup_real_mode_with_ivt_handlers(&mut bus);
    let mut state = make_real_mode_state_for_semantics();
    state.set_eax(0x8000_0000);
    state.set_ecx(0x0000_FF1F); // low 5 bits = 31, ignore high bits
    cpu.load_state(&state);

    // 66 D3 F8 = SAR EAX, CL.
    place_in_test_code(&mut bus, 0x0000, &[0x66, 0xD3, 0xF8, HLT_OPCODE]);

    cpu.step(&mut bus);

    // 0x8000_0000 SAR 31 = 0xFFFF_FFFF.
    assert_eq!(cpu.state.eax(), 0xFFFF_FFFF);
}

#[test]
fn real_mode_rol_byte_count_8_is_no_op_after_masking() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let _ = setup_real_mode_with_ivt_handlers(&mut bus);
    let mut state = make_real_mode_state_for_semantics();
    state.set_eax(0x0000_00A5);
    state.set_ecx(0x0000_0008); // 8
    cpu.load_state(&state);

    // D2 C0 = ROL AL, CL.
    place_in_test_code(&mut bus, 0x0000, &[0xD2, 0xC0, HLT_OPCODE]);

    cpu.step(&mut bus);

    // For byte rotates the modulus is 8 (count & 0x1F then % 8); 8 % 8 = 0,
    // so AL unchanged at 0xA5.
    assert_eq!(cpu.state.eax() as u8, 0xA5);
}

#[test]
fn real_mode_ror_word_count_45_equivalent_to_count_13() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let _ = setup_real_mode_with_ivt_handlers(&mut bus);
    let mut state = make_real_mode_state_for_semantics();
    state.set_eax(0x0000_8001);
    state.set_ecx(0x0000_002D); // 45
    cpu.load_state(&state);

    // D3 C8 = ROR AX, CL.
    place_in_test_code(&mut bus, 0x0000, &[0xD3, 0xC8, HLT_OPCODE]);

    cpu.step(&mut bus);

    // 45 & 0x1F = 13. ROR 0x8001 by 13 -> 0x0_C008 (truncated to 16 bits).
    let expected = 0x8001u16.rotate_right(13);
    assert_eq!((cpu.state.eax() & 0xFFFF) as u16, expected);
}

#[test]
fn real_mode_rcl_dword_count_above_31_uses_low_5_bits() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let _ = setup_real_mode_with_ivt_handlers(&mut bus);
    let mut state = make_real_mode_state_for_semantics();
    state.set_eax(0x0000_0001);
    state.set_ecx(0x0000_0040); // 64; & 0x1F = 0
    state.flags.carry_val = 0;
    cpu.load_state(&state);

    // 66 D3 D0 = RCL EAX, CL.
    place_in_test_code(&mut bus, 0x0000, &[0x66, 0xD3, 0xD0, HLT_OPCODE]);

    cpu.step(&mut bus);

    // Count masked to 0 -> no rotation, EAX unchanged.
    assert_eq!(cpu.state.eax(), 0x0000_0001);
}

#[test]
fn real_mode_rcr_word_count_above_31_uses_low_5_bits() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let _ = setup_real_mode_with_ivt_handlers(&mut bus);
    let mut state = make_real_mode_state_for_semantics();
    state.set_eax(0x0000_0001);
    state.set_ecx(0x0000_0021); // 33; & 0x1F = 1
    state.flags.carry_val = 1;
    cpu.load_state(&state);

    // D3 D8 = RCR AX, CL.
    place_in_test_code(&mut bus, 0x0000, &[0xD3, 0xD8, HLT_OPCODE]);

    cpu.step(&mut bus);

    // Count = 1. 17-bit RCR of value 0x0001 with CF=1:
    //   bits = (CF=1, AX=0x0001) = 1_00000000_00000001
    //   rotate right by 1 within 17-bit window:
    //     new CF = AX bit 0 (=1)
    //     new AX = (CF in -> bit 15) | (AX >> 1)
    //          = 0x8000 | 0x0000 = 0x8000.
    assert_eq!((cpu.state.eax() & 0xFFFF) as u16, 0x8000);
    assert!(cpu.state.flags.cf());
}
