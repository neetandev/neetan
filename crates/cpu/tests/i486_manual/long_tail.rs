//! Long-tail instruction tests derived from the 80486 PRM.
//!
//! Covers ENTER/LEAVE, PUSHA/POPA/PUSHAD/POPAD, BT/BTS/BTR/BTC, XCHG with
//! memory operand (implicit LOCK), XLAT, AAA/AAS/AAD/AAM, DAA/DAS,
//! CWD/CDQ/CBW/CWDE, the SETcc matrix, and REP-prefixed string operations
//! with segment-override / segment-limit edges.

use common::Cpu as _;

use super::setup::{
    GLOBAL_DESCRIPTOR_TABLE_BASE, HANDLER_GENERAL_PROTECTION_IP,
    RIGHTS_RING0_DATA_WRITABLE_ACCESSED, RING0_CODE_BASE, SHARED_DATA_BASE, TestBus, make_cpu_486,
    place_at, read_byte_at, read_dword_at, read_word_at, setup_protected_mode_with_handlers,
    write_segment_descriptor_16bit,
};

const HALT_OPCODE: u8 = 0xF4;

fn place_then_halt(bus: &mut TestBus, code: &[u8]) {
    place_at(bus, RING0_CODE_BASE, code);
    bus.ram[(RING0_CODE_BASE + code.len() as u32) as usize] = HALT_OPCODE;
}

#[test]
fn enter_with_zero_alloc_zero_level_saves_bp() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_esp(0x1000);
    state.set_ebp(0xAAAA);
    cpu.load_state(&state);

    // ENTER 0, 0 = C8 00 00 00.
    place_then_halt(&mut bus, &[0xC8, 0x00, 0x00, 0x00]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.esp(), 0x0FFE);
    assert_eq!(cpu.state.ebp(), 0x0FFE);
    assert_eq!(read_word_at(&bus, 0x0FFE), 0xAAAA);
}

#[test]
fn enter_allocates_local_frame_bytes() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_esp(0x1000);
    state.set_ebp(0xBEEF);
    cpu.load_state(&state);

    // ENTER 0x0010, 0 = C8 10 00 00.
    place_then_halt(&mut bus, &[0xC8, 0x10, 0x00, 0x00]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.ebp(), 0x0FFE);
    assert_eq!(cpu.state.esp(), 0x0FEE);
}

#[test]
fn enter_level_one_pushes_frame_pointer() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_esp(0x1000);
    state.set_ebp(0x1234);
    cpu.load_state(&state);

    // ENTER 0, 1 = C8 00 00 01.
    place_then_halt(&mut bus, &[0xC8, 0x00, 0x00, 0x01]);

    cpu.step(&mut bus);

    // Per 80486 PRM ENTER pseudocode: FRAME-PTR := SP after pushing old BP;
    // for level 1 the same FRAME-PTR is pushed once more, then BP is set to
    // FRAME-PTR. So pushed BP at [0xFFE], frame_ptr at [0xFFC], and the
    // architectural BP register holds FRAME-PTR (0x0FFE), not the inner SP.
    assert_eq!(read_word_at(&bus, 0x0FFE), 0x1234);
    assert_eq!(read_word_at(&bus, 0x0FFC), 0x0FFE);
    assert_eq!(cpu.state.ebp() & 0xFFFF, 0x0FFE);
    assert_eq!(cpu.state.esp() & 0xFFFF, 0x0FFC);
}

#[test]
fn enter_level_field_truncated_to_lower_five_bits() {
    // ENTER level operand is masked to bits [4:0] per 80486 PRM. A value of
    // 0x20 (32) collapses to 0, identical to ENTER 0,0.
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_esp(0x1000);
    state.set_ebp(0xAAAA);
    cpu.load_state(&state);

    place_then_halt(&mut bus, &[0xC8, 0x00, 0x00, 0x20]);

    cpu.step(&mut bus);

    // Only one push (the original BP) -> SP -= 2 once.
    assert_eq!(cpu.state.esp(), 0x0FFE);
    assert_eq!(cpu.state.ebp(), 0x0FFE);
}

#[test]
fn leave_restores_esp_from_ebp_and_pops_ebp() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_esp(0x0FF0);
    state.set_ebp(0x0FFE);
    cpu.load_state(&state);

    super::setup::write_word_at(&mut bus, 0x0FFE, 0xCAFE);

    // LEAVE = C9.
    place_then_halt(&mut bus, &[0xC9]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.esp() & 0xFFFF, 0x1000);
    assert_eq!(cpu.state.ebp() & 0xFFFF, 0xCAFE);
}

#[test]
fn pusha_pushes_16bit_registers_in_documented_order() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_eax(0x1111);
    state.set_ecx(0x2222);
    state.set_edx(0x3333);
    state.set_ebx(0x4444);
    state.set_esp(0x1000);
    state.set_ebp(0x6666);
    state.set_esi(0x7777);
    state.set_edi(0x8888);
    cpu.load_state(&state);

    // PUSHA = 0x60.
    place_then_halt(&mut bus, &[0x60]);

    cpu.step(&mut bus);

    let new_sp = cpu.state.esp() & 0xFFFF;
    assert_eq!(new_sp, 0x1000 - 16);
    // Last pushed (top of stack) is DI (0x8888); first pushed is AX (0x1111).
    assert_eq!(read_word_at(&bus, new_sp), 0x8888);
    assert_eq!(read_word_at(&bus, new_sp + 2), 0x7777);
    assert_eq!(read_word_at(&bus, new_sp + 4), 0x6666);
    assert_eq!(read_word_at(&bus, new_sp + 6), 0x1000);
    assert_eq!(read_word_at(&bus, new_sp + 8), 0x4444);
    assert_eq!(read_word_at(&bus, new_sp + 10), 0x3333);
    assert_eq!(read_word_at(&bus, new_sp + 12), 0x2222);
    assert_eq!(read_word_at(&bus, new_sp + 14), 0x1111);
}

#[test]
fn popa_restores_registers_skipping_sp_slot() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_esp(0x0F00);
    cpu.load_state(&state);

    let stack_base = 0x0F00u32;
    super::setup::write_word_at(&mut bus, stack_base, 0xDDDD); // DI
    super::setup::write_word_at(&mut bus, stack_base + 2, 0xCCCC); // SI
    super::setup::write_word_at(&mut bus, stack_base + 4, 0xBBBB); // BP
    super::setup::write_word_at(&mut bus, stack_base + 6, 0xAAAA); // (SP slot, discarded)
    super::setup::write_word_at(&mut bus, stack_base + 8, 0x4444); // BX
    super::setup::write_word_at(&mut bus, stack_base + 10, 0x3333); // DX
    super::setup::write_word_at(&mut bus, stack_base + 12, 0x2222); // CX
    super::setup::write_word_at(&mut bus, stack_base + 14, 0x1111); // AX

    place_then_halt(&mut bus, &[0x61]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.eax() & 0xFFFF, 0x1111);
    assert_eq!(cpu.state.ecx() & 0xFFFF, 0x2222);
    assert_eq!(cpu.state.edx() & 0xFFFF, 0x3333);
    assert_eq!(cpu.state.ebx() & 0xFFFF, 0x4444);
    assert_eq!(cpu.state.ebp() & 0xFFFF, 0xBBBB);
    assert_eq!(cpu.state.esi() & 0xFFFF, 0xCCCC);
    assert_eq!(cpu.state.edi() & 0xFFFF, 0xDDDD);
    assert_eq!(cpu.state.esp() & 0xFFFF, 0x0F00 + 16);
}

#[test]
fn pushad_pushes_32bit_registers() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_eax(0x1111_1111);
    state.set_ecx(0x2222_2222);
    state.set_edx(0x3333_3333);
    state.set_ebx(0x4444_4444);
    state.set_esp(0x1000);
    state.set_ebp(0x6666_6666);
    state.set_esi(0x7777_7777);
    state.set_edi(0x8888_8888);
    cpu.load_state(&state);

    place_then_halt(&mut bus, &[0x66, 0x60]);

    cpu.step(&mut bus);

    let new_sp = cpu.state.esp() & 0xFFFF;
    assert_eq!(new_sp, 0x1000 - 32);
    assert_eq!(read_dword_at(&bus, new_sp), 0x8888_8888);
    assert_eq!(read_dword_at(&bus, new_sp + 4), 0x7777_7777);
    assert_eq!(read_dword_at(&bus, new_sp + 8), 0x6666_6666);
    assert_eq!(read_dword_at(&bus, new_sp + 12), 0x0000_1000);
    assert_eq!(read_dword_at(&bus, new_sp + 16), 0x4444_4444);
    assert_eq!(read_dword_at(&bus, new_sp + 20), 0x3333_3333);
    assert_eq!(read_dword_at(&bus, new_sp + 24), 0x2222_2222);
    assert_eq!(read_dword_at(&bus, new_sp + 28), 0x1111_1111);
}

#[test]
fn popad_restores_registers_skipping_esp_slot() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_esp(0x0F00);
    cpu.load_state(&state);

    let base = 0x0F00u32;
    super::setup::write_dword_at(&mut bus, base, 0x0000_DDDD);
    super::setup::write_dword_at(&mut bus, base + 4, 0x0000_CCCC);
    super::setup::write_dword_at(&mut bus, base + 8, 0x0000_BBBB);
    super::setup::write_dword_at(&mut bus, base + 12, 0xDEAD_BEEF);
    super::setup::write_dword_at(&mut bus, base + 16, 0x0000_4444);
    super::setup::write_dword_at(&mut bus, base + 20, 0x0000_3333);
    super::setup::write_dword_at(&mut bus, base + 24, 0x0000_2222);
    super::setup::write_dword_at(&mut bus, base + 28, 0x0000_1111);

    place_then_halt(&mut bus, &[0x66, 0x61]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.eax(), 0x0000_1111);
    assert_eq!(cpu.state.ecx(), 0x0000_2222);
    assert_eq!(cpu.state.edx(), 0x0000_3333);
    assert_eq!(cpu.state.ebx(), 0x0000_4444);
    assert_eq!(cpu.state.ebp(), 0x0000_BBBB);
    assert_eq!(cpu.state.esi(), 0x0000_CCCC);
    assert_eq!(cpu.state.edi(), 0x0000_DDDD);
}

#[test]
fn bt_register_reads_bit_and_sets_carry_flag() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    // BT AX, CX with AX=0x0F00, CX=8 -> bit 8 of 0x0F00 is 1 -> CF=1.
    state.set_eax(0x0F00);
    state.set_ecx(0x0008);
    cpu.load_state(&state);

    // BT r/m16, r16 = 0F A3 /r. ModR/M with r=CX(1), r/m=AX(0).
    place_then_halt(&mut bus, &[0x0F, 0xA3, 0xC8]);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.cf());
}

#[test]
fn bt_register_clears_carry_when_bit_is_zero() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_eax(0x0F00);
    state.set_ecx(0x0007);
    cpu.load_state(&state);

    place_then_halt(&mut bus, &[0x0F, 0xA3, 0xC8]);

    cpu.step(&mut bus);

    assert!(!cpu.state.flags.cf());
}

#[test]
fn bts_register_sets_bit_and_returns_old_value_in_carry() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_eax(0x0000);
    state.set_ecx(0x0005);
    cpu.load_state(&state);

    // BTS r/m16, r16 = 0F AB /r.
    place_then_halt(&mut bus, &[0x0F, 0xAB, 0xC8]);

    cpu.step(&mut bus);

    assert!(!cpu.state.flags.cf());
    assert_eq!(cpu.state.eax() & 0xFFFF, 1 << 5);
}

#[test]
fn btr_register_clears_bit_and_returns_old_value_in_carry() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_eax(0xFFFF);
    state.set_ecx(0x0007);
    cpu.load_state(&state);

    // BTR r/m16, r16 = 0F B3 /r.
    place_then_halt(&mut bus, &[0x0F, 0xB3, 0xC8]);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.cf());
    assert_eq!(cpu.state.eax() & 0xFFFF, 0xFF7F);
}

#[test]
fn btc_register_toggles_bit() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_eax(0x00FF);
    state.set_ecx(0x0003);
    cpu.load_state(&state);

    // BTC r/m16, r16 = 0F BB /r.
    place_then_halt(&mut bus, &[0x0F, 0xBB, 0xC8]);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.cf());
    assert_eq!(cpu.state.eax() & 0xFFFF, 0x00F7);
}

#[test]
fn bt_immediate_form_dword_masks_offset_to_31() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_eax(0x8000_0000);
    cpu.load_state(&state);

    // BT EAX, 31 = 0x66 0x0F 0xBA /4 imm8. ModR/M /4=0x20|EAX=0xE0.
    place_then_halt(&mut bus, &[0x66, 0x0F, 0xBA, 0xE0, 31]);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.cf());
}

#[test]
fn bt_memory_form_word_handles_bit_offset_above_15() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    // bit_offset=17 with 16-bit operand: byte_delta = 17/16 = 1 word stride
    // = 2 bytes. Effective bit index = 17 & 15 = 1.
    state.set_ecx(17);
    cpu.load_state(&state);

    let memory_offset: u16 = 0x100;
    // Write bit 1 at SHARED_DATA_BASE + 0x102 (offset by one word).
    super::setup::write_word_at(
        &mut bus,
        SHARED_DATA_BASE + memory_offset as u32 + 2,
        0x0002,
    );

    // BT [0x100], CX = 0F A3 /r: r/m disp16, reg=CX(1) -> ModR/M = 0x0E.
    place_then_halt(
        &mut bus,
        &[
            0x0F,
            0xA3,
            0x0E,
            memory_offset as u8,
            (memory_offset >> 8) as u8,
        ],
    );

    cpu.step(&mut bus);

    assert!(cpu.state.flags.cf());
}

#[test]
fn xchg_byte_register_swaps_values() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_eax(0x12);
    state.set_ecx(0x34);
    cpu.load_state(&state);

    // XCHG AL, CL = 86 C1 (r/m=AL, reg=CL).
    place_then_halt(&mut bus, &[0x86, 0xC1]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.eax() & 0xFF, 0x34);
    assert_eq!(cpu.state.ecx() & 0xFF, 0x12);
}

#[test]
fn xchg_word_register_with_eax_short_form() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_eax(0xAAAA);
    state.set_ecx(0xBBBB);
    cpu.load_state(&state);

    // XCHG AX, CX = 0x91.
    place_then_halt(&mut bus, &[0x91]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.eax() & 0xFFFF, 0xBBBB);
    assert_eq!(cpu.state.ecx() & 0xFFFF, 0xAAAA);
}

#[test]
fn xchg_with_memory_swaps_register_and_memory() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_eax(0x1234);
    cpu.load_state(&state);

    let memory_offset: u16 = 0x100;
    super::setup::write_word_at(&mut bus, SHARED_DATA_BASE + memory_offset as u32, 0xDEAD);

    // XCHG AX, [disp16] = 87 06 disp16 (reg=AX, r/m disp16).
    place_then_halt(
        &mut bus,
        &[0x87, 0x06, memory_offset as u8, (memory_offset >> 8) as u8],
    );

    cpu.step(&mut bus);

    assert_eq!(cpu.state.eax() & 0xFFFF, 0xDEAD);
    assert_eq!(
        read_word_at(&bus, SHARED_DATA_BASE + memory_offset as u32),
        0x1234
    );
}

#[test]
fn xchg_with_lock_prefix_succeeds() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_eax(0x1234);
    cpu.load_state(&state);

    let memory_offset: u16 = 0x100;
    super::setup::write_word_at(&mut bus, SHARED_DATA_BASE + memory_offset as u32, 0xDEAD);

    // LOCK XCHG AX, [disp16] = F0 87 06 disp16.
    place_then_halt(
        &mut bus,
        &[
            0xF0,
            0x87,
            0x06,
            memory_offset as u8,
            (memory_offset >> 8) as u8,
        ],
    );

    cpu.step(&mut bus);

    assert_eq!(cpu.state.eax() & 0xFFFF, 0xDEAD);
}

#[test]
fn xlat_reads_byte_at_ds_bx_plus_al() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_eax(0x0003);
    state.set_ebx(0x0100);
    cpu.load_state(&state);

    // Place table at SHARED_DATA_BASE + 0x100.
    bus.ram[(SHARED_DATA_BASE + 0x100) as usize] = 0xA0;
    bus.ram[(SHARED_DATA_BASE + 0x101) as usize] = 0xA1;
    bus.ram[(SHARED_DATA_BASE + 0x102) as usize] = 0xA2;
    bus.ram[(SHARED_DATA_BASE + 0x103) as usize] = 0xA3;

    place_then_halt(&mut bus, &[0xD7]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.eax() & 0xFF, 0xA3);
}

#[test]
fn xlat_with_address_size_override_reads_via_full_ebx() {
    // Verify the 0x67 address-size prefix selects EBX (32-bit) instead of BX
    // (16-bit) for the operand. To keep the access within the default DS
    // limit (0xFFFF), use a 32-bit EBX whose low 16 bits already cover the
    // full effective offset.
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_eax(0x05);
    state.set_ebx(0x0000_0200);
    cpu.load_state(&state);

    bus.ram[(SHARED_DATA_BASE + 0x205) as usize] = 0x77;

    place_then_halt(&mut bus, &[0x67, 0xD7]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.eax() & 0xFF, 0x77);
}

#[test]
fn aaa_adjusts_when_low_nibble_above_9() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_eax(0x000A);
    cpu.load_state(&state);

    place_then_halt(&mut bus, &[0x37]);

    cpu.step(&mut bus);

    // AL+6 = 0x10 -> AL=(low nibble of 0x10) & 0x0F = 0; AH += 1.
    assert_eq!(cpu.state.eax() & 0xFFFF, 0x0100);
    assert!(cpu.state.flags.cf());
    assert!(cpu.state.flags.af());
}

#[test]
fn aaa_no_adjustment_when_low_nibble_below_10() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_eax(0x0007);
    cpu.load_state(&state);

    place_then_halt(&mut bus, &[0x37]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.eax() & 0xFFFF, 0x0007);
    assert!(!cpu.state.flags.cf());
    assert!(!cpu.state.flags.af());
}

#[test]
fn aas_adjusts_when_low_nibble_above_9() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_eax(0x010A);
    cpu.load_state(&state);

    place_then_halt(&mut bus, &[0x3F]);

    cpu.step(&mut bus);

    // AX = 0x010A - 0x0106 = 0x0004; AL = 0x04 & 0x0F = 0x04.
    assert_eq!(cpu.state.eax() & 0xFFFF, 0x0004);
    assert!(cpu.state.flags.cf());
    assert!(cpu.state.flags.af());
}

#[test]
fn aam_with_default_base_10_splits_unpacked_decimal() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_eax(0x0035);
    cpu.load_state(&state);

    // AAM = D4 0A.
    place_then_halt(&mut bus, &[0xD4, 0x0A]);

    cpu.step(&mut bus);

    // AH = 53/10 = 5; AL = 53%10 = 3.
    assert_eq!(cpu.state.eax() & 0xFFFF, 0x0503);
}

#[test]
fn aad_with_default_base_10_collapses_unpacked_decimal() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_eax(0x0503);
    cpu.load_state(&state);

    // AAD = D5 0A.
    place_then_halt(&mut bus, &[0xD5, 0x0A]);

    cpu.step(&mut bus);

    // AL = 3 + 5*10 = 53.
    assert_eq!(cpu.state.eax() & 0xFFFF, 0x0035);
}

#[test]
fn daa_adjusts_when_low_nibble_above_9() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_eax(0x000A);
    cpu.load_state(&state);

    place_then_halt(&mut bus, &[0x27]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.eax() & 0xFF, 0x10);
    assert!(cpu.state.flags.af());
}

#[test]
fn daa_adjusts_high_nibble_above_9_with_carry() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_eax(0x009A);
    cpu.load_state(&state);

    place_then_halt(&mut bus, &[0x27]);

    cpu.step(&mut bus);

    // (0x9A & 0x0F)=0xA > 9 -> AL+=6 = 0xA0; old_al=0x9A>0x99 -> AL+=0x60=0x100->0x00; CF=1.
    assert_eq!(cpu.state.eax() & 0xFF, 0x00);
    assert!(cpu.state.flags.cf());
}

#[test]
fn das_adjusts_when_low_nibble_above_9() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_eax(0x000F);
    cpu.load_state(&state);

    place_then_halt(&mut bus, &[0x2F]);

    cpu.step(&mut bus);

    // 0x0F - 6 = 0x09; AF=1.
    assert_eq!(cpu.state.eax() & 0xFF, 0x09);
    assert!(cpu.state.flags.af());
}

#[test]
fn cbw_sign_extends_al_to_ax() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_eax(0x0080);
    cpu.load_state(&state);

    place_then_halt(&mut bus, &[0x98]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.eax() & 0xFFFF, 0xFF80);
}

#[test]
fn cbw_sign_extends_positive_al() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_eax(0x0042);
    cpu.load_state(&state);

    place_then_halt(&mut bus, &[0x98]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.eax() & 0xFFFF, 0x0042);
}

#[test]
fn cwde_sign_extends_ax_to_eax() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_eax(0x0000_8000);
    cpu.load_state(&state);

    // CWDE = 66 98.
    place_then_halt(&mut bus, &[0x66, 0x98]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.eax(), 0xFFFF_8000);
}

#[test]
fn cwd_sign_extends_ax_to_dx_ax() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_eax(0x0000_8000);
    state.set_edx(0x1234_5678);
    cpu.load_state(&state);

    place_then_halt(&mut bus, &[0x99]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.eax() & 0xFFFF, 0x8000);
    assert_eq!(cpu.state.edx() & 0xFFFF, 0xFFFF);
}

#[test]
fn cdq_sign_extends_eax_to_edx_eax() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_eax(0x8000_0000);
    cpu.load_state(&state);

    // CDQ = 66 99.
    place_then_halt(&mut bus, &[0x66, 0x99]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.edx(), 0xFFFF_FFFF);
}

#[test]
fn setz_writes_one_when_zero_flag_set() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.flags.zero_val = 0; // ZF=1
    state.set_eax(0x00FF);
    cpu.load_state(&state);

    // SETZ AL = 0F 94 C0.
    place_then_halt(&mut bus, &[0x0F, 0x94, 0xC0]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.eax() & 0xFF, 0x01);
}

#[test]
fn setnz_writes_zero_when_zero_flag_set() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.flags.zero_val = 0; // ZF=1
    state.set_eax(0x00FF);
    cpu.load_state(&state);

    // SETNZ AL = 0F 95 C0.
    place_then_halt(&mut bus, &[0x0F, 0x95, 0xC0]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.eax() & 0xFF, 0x00);
}

#[test]
fn setc_writes_one_when_carry_set() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.flags.carry_val = 1;
    cpu.load_state(&state);

    place_then_halt(&mut bus, &[0x0F, 0x92, 0xC0]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.eax() & 0xFF, 0x01);
}

#[test]
fn seto_writes_one_when_overflow_set() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.flags.overflow_val = 1;
    cpu.load_state(&state);

    place_then_halt(&mut bus, &[0x0F, 0x90, 0xC0]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.eax() & 0xFF, 0x01);
}

#[test]
fn sets_writes_one_when_sign_set() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.flags.sign_val = -1;
    cpu.load_state(&state);

    place_then_halt(&mut bus, &[0x0F, 0x98, 0xC0]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.eax() & 0xFF, 0x01);
}

#[test]
fn setp_writes_one_when_parity_even() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.flags.parity_val = 0; // 0 has 0 set bits -> even parity (PF=1).
    cpu.load_state(&state);

    place_then_halt(&mut bus, &[0x0F, 0x9A, 0xC0]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.eax() & 0xFF, 0x01);
}

#[test]
fn setl_writes_one_when_sign_differs_from_overflow() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.flags.sign_val = -1; // SF=1
    state.flags.overflow_val = 0; // OF=0
    cpu.load_state(&state);

    // SETL AL = 0F 9C C0.
    place_then_halt(&mut bus, &[0x0F, 0x9C, 0xC0]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.eax() & 0xFF, 0x01);
}

#[test]
fn setle_writes_one_when_zero_or_signs_differ() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.flags.zero_val = 0; // ZF=1
    state.flags.sign_val = 0;
    state.flags.overflow_val = 0;
    cpu.load_state(&state);

    // SETLE AL = 0F 9E C0.
    place_then_halt(&mut bus, &[0x0F, 0x9E, 0xC0]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.eax() & 0xFF, 0x01);
}

#[test]
fn setcc_to_memory_operand_writes_to_memory() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.flags.carry_val = 1;
    cpu.load_state(&state);

    let memory_offset: u16 = 0x100;
    bus.ram[(SHARED_DATA_BASE + memory_offset as u32) as usize] = 0x55;

    // SETC [disp16] = 0F 92 06 disp16.
    place_then_halt(
        &mut bus,
        &[
            0x0F,
            0x92,
            0x06,
            memory_offset as u8,
            (memory_offset >> 8) as u8,
        ],
    );

    cpu.step(&mut bus);

    assert_eq!(
        read_byte_at(&bus, SHARED_DATA_BASE + memory_offset as u32),
        1
    );
}

#[test]
fn rep_movsb_copies_ecx_bytes_then_clears_ecx() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_ecx(4);
    state.set_esi(0x100);
    state.set_edi(0x200);
    cpu.load_state(&state);

    bus.ram[(SHARED_DATA_BASE + 0x100) as usize] = 0xAA;
    bus.ram[(SHARED_DATA_BASE + 0x101) as usize] = 0xBB;
    bus.ram[(SHARED_DATA_BASE + 0x102) as usize] = 0xCC;
    bus.ram[(SHARED_DATA_BASE + 0x103) as usize] = 0xDD;

    // REP MOVSB = F3 A4. Then HLT.
    place_then_halt(&mut bus, &[0xF3, 0xA4]);

    // Loop until REP completes (cpu may take multiple steps).
    for _ in 0..10 {
        cpu.step(&mut bus);
        if cpu.state.ecx() == 0 {
            break;
        }
    }

    assert_eq!(cpu.state.ecx() & 0xFFFF, 0);
    assert_eq!(cpu.state.esi() & 0xFFFF, 0x104);
    assert_eq!(cpu.state.edi() & 0xFFFF, 0x204);
    for index in 0..4 {
        assert_eq!(
            bus.ram[(SHARED_DATA_BASE + 0x200 + index) as usize],
            [0xAA, 0xBB, 0xCC, 0xDD][index as usize]
        );
    }
}

#[test]
fn rep_stosb_writes_al_ecx_times() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_ecx(4);
    state.set_eax(0x77);
    state.set_edi(0x100);
    cpu.load_state(&state);

    place_then_halt(&mut bus, &[0xF3, 0xAA]);

    for _ in 0..10 {
        cpu.step(&mut bus);
        if cpu.state.ecx() == 0 {
            break;
        }
    }

    for index in 0..4 {
        assert_eq!(bus.ram[(SHARED_DATA_BASE + 0x100 + index) as usize], 0x77);
    }
    assert_eq!(cpu.state.edi() & 0xFFFF, 0x104);
}

#[test]
fn repe_cmpsb_stops_on_first_mismatch() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_ecx(4);
    state.set_esi(0x100);
    state.set_edi(0x200);
    cpu.load_state(&state);

    bus.ram[(SHARED_DATA_BASE + 0x100) as usize] = 0xAA;
    bus.ram[(SHARED_DATA_BASE + 0x101) as usize] = 0xBB;
    bus.ram[(SHARED_DATA_BASE + 0x102) as usize] = 0xCC;
    bus.ram[(SHARED_DATA_BASE + 0x103) as usize] = 0xDD;

    bus.ram[(SHARED_DATA_BASE + 0x200) as usize] = 0xAA;
    bus.ram[(SHARED_DATA_BASE + 0x201) as usize] = 0xBB;
    bus.ram[(SHARED_DATA_BASE + 0x202) as usize] = 0xFF; // mismatch
    bus.ram[(SHARED_DATA_BASE + 0x203) as usize] = 0xDD;

    // REPE CMPSB = F3 A6.
    place_then_halt(&mut bus, &[0xF3, 0xA6]);

    for _ in 0..10 {
        cpu.step(&mut bus);
        if cpu.state.ecx() == 0 || !cpu.state.flags.zf() {
            break;
        }
    }

    // Stops after 3 iterations: ECX = 4-3 = 1.
    assert_eq!(cpu.state.ecx() & 0xFFFF, 1);
    assert!(!cpu.state.flags.zf());
}

#[test]
fn repne_scasb_stops_on_first_match() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_ecx(8);
    state.set_eax(0x33);
    state.set_edi(0x100);
    cpu.load_state(&state);

    bus.ram[(SHARED_DATA_BASE + 0x100) as usize] = 0x11;
    bus.ram[(SHARED_DATA_BASE + 0x101) as usize] = 0x22;
    bus.ram[(SHARED_DATA_BASE + 0x102) as usize] = 0x33; // match
    bus.ram[(SHARED_DATA_BASE + 0x103) as usize] = 0x44;

    // REPNE SCASB = F2 AE.
    place_then_halt(&mut bus, &[0xF2, 0xAE]);

    for _ in 0..16 {
        cpu.step(&mut bus);
        if cpu.state.ecx() == 0 || cpu.state.flags.zf() {
            break;
        }
    }

    assert_eq!(cpu.state.ecx() & 0xFFFF, 5);
    assert!(cpu.state.flags.zf());
}

#[test]
fn rep_movsb_with_segment_override_uses_overridden_segment() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    // Use FS as the source segment override; set it to point at the
    // SHARED_DATA_BASE area but with a unique base so the test verifies
    // the override is honored.
    state.set_fs(super::setup::SELECTOR_RING0_DATA);
    state.seg_bases[cpu::SegReg32::FS as usize] = SHARED_DATA_BASE + 0x1000;
    state.seg_limits[cpu::SegReg32::FS as usize] = 0xFFFF;
    state.seg_rights[cpu::SegReg32::FS as usize] = RIGHTS_RING0_DATA_WRITABLE_ACCESSED;
    state.seg_valid[cpu::SegReg32::FS as usize] = true;
    state.set_ecx(2);
    state.set_esi(0x100);
    state.set_edi(0x200);
    cpu.load_state(&state);

    // Source data at FS:0x100 = SHARED_DATA_BASE + 0x1000 + 0x100.
    bus.ram[(SHARED_DATA_BASE + 0x1100) as usize] = 0xAA;
    bus.ram[(SHARED_DATA_BASE + 0x1101) as usize] = 0xBB;

    // FS-prefix REP MOVSB = 64 F3 A4.
    place_then_halt(&mut bus, &[0x64, 0xF3, 0xA4]);

    for _ in 0..10 {
        cpu.step(&mut bus);
        if cpu.state.ecx() == 0 {
            break;
        }
    }

    assert_eq!(bus.ram[(SHARED_DATA_BASE + 0x200) as usize], 0xAA);
    assert_eq!(bus.ram[(SHARED_DATA_BASE + 0x201) as usize], 0xBB);
}

#[test]
fn rep_movsb_crossing_source_segment_limit_raises_general_protection() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    // Shrink DS to limit 4 so the 6th byte read (offset 5) raises #GP.
    write_segment_descriptor_16bit(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        super::setup::SELECTOR_RING0_DATA >> 3,
        SHARED_DATA_BASE,
        4,
        RIGHTS_RING0_DATA_WRITABLE_ACCESSED,
    );
    state.seg_limits[cpu::SegReg32::DS as usize] = 4;

    state.set_ecx(8);
    state.set_esi(0);
    state.set_edi(0x200);
    cpu.load_state(&state);

    place_then_halt(&mut bus, &[0xF3, 0xA4]);

    for _ in 0..50 {
        cpu.step(&mut bus);
        if cpu.halted() {
            break;
        }
    }

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn lodsw_loads_word_into_ax_and_advances_esi() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_esi(0x100);
    cpu.load_state(&state);

    super::setup::write_word_at(&mut bus, SHARED_DATA_BASE + 0x100, 0xCAFE);

    // LODSW = AD.
    place_then_halt(&mut bus, &[0xAD]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.eax() & 0xFFFF, 0xCAFE);
    assert_eq!(cpu.state.esi() & 0xFFFF, 0x102);
}

#[test]
fn stosw_stores_ax_at_es_edi_and_advances() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_eax(0xBABE);
    state.set_edi(0x200);
    cpu.load_state(&state);

    place_then_halt(&mut bus, &[0xAB]);

    cpu.step(&mut bus);

    assert_eq!(read_word_at(&bus, SHARED_DATA_BASE + 0x200), 0xBABE);
    assert_eq!(cpu.state.edi() & 0xFFFF, 0x202);
}

#[test]
fn rep_stosw_with_direction_flag_set_decrements_edi() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.flags.df = true;
    state.set_eax(0x1234);
    state.set_edi(0x200);
    state.set_ecx(2);
    cpu.load_state(&state);

    place_then_halt(&mut bus, &[0xF3, 0xAB]);

    for _ in 0..10 {
        cpu.step(&mut bus);
        if cpu.state.ecx() == 0 {
            break;
        }
    }

    // Each iteration decrements EDI by 2. Two iterations: 0x200 -> 0x1FE -> 0x1FC.
    assert_eq!(cpu.state.edi() & 0xFFFF, 0x1FC);
    assert_eq!(read_word_at(&bus, SHARED_DATA_BASE + 0x200), 0x1234);
    assert_eq!(read_word_at(&bus, SHARED_DATA_BASE + 0x1FE), 0x1234);
}

// 80486 PRM Chapter 26: each iteration of a string operation must satisfy
// the segment limit / read-write permission check before performing the
// memory access. The tests below verify that every string mnemonic (MOVS,
// CMPS, STOS, LODS, SCAS, INS, OUTS) raises #GP(0) on the iteration where
// the indexed offset exceeds the limit of its source or destination
// segment. Each test shrinks DS or ES to a tight limit and runs the
// instruction until it either completes or faults.

fn shrink_only_data_segment_limit(state: &mut cpu::I386State, limit: u16) {
    state.seg_limits[cpu::SegReg32::DS as usize] = limit as u32;
}

fn shrink_only_extra_segment_limit(state: &mut cpu::I386State, limit: u16) {
    state.seg_limits[cpu::SegReg32::ES as usize] = limit as u32;
}

fn run_until_halted(cpu: &mut cpu::I386<{ cpu::CPU_MODEL_486_DX }>, bus: &mut TestBus) {
    for _ in 0..64 {
        cpu.step(bus);
        if cpu.halted() {
            break;
        }
    }
}

#[test]
fn rep_movsb_crossing_destination_segment_limit_raises_general_protection() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    shrink_only_extra_segment_limit(&mut state, 4);
    state.set_ecx(8);
    state.set_esi(0x100);
    state.set_edi(0); // ES:0..7 writes, but ES limit 4 -> fault on 6th byte.
    cpu.load_state(&state);

    place_then_halt(&mut bus, &[0xF3, 0xA4]);
    run_until_halted(&mut cpu, &mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn rep_movsw_crossing_source_segment_limit_raises_general_protection() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    shrink_only_data_segment_limit(&mut state, 5);
    state.set_ecx(4);
    state.set_esi(0); // word reads at offsets 0,2,4 OK; offset 6 (end=7>5) faults.
    state.set_edi(0x200);
    cpu.load_state(&state);

    place_then_halt(&mut bus, &[0xF3, 0xA5]);
    run_until_halted(&mut cpu, &mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn rep_movsw_crossing_destination_segment_limit_raises_general_protection() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    shrink_only_extra_segment_limit(&mut state, 5);
    state.set_ecx(4);
    state.set_esi(0x200);
    state.set_edi(0);
    cpu.load_state(&state);

    place_then_halt(&mut bus, &[0xF3, 0xA5]);
    run_until_halted(&mut cpu, &mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn rep_cmpsb_crossing_source_segment_limit_raises_general_protection() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    shrink_only_data_segment_limit(&mut state, 4);
    state.set_ecx(8);
    state.set_esi(0);
    state.set_edi(0x200);
    cpu.load_state(&state);

    place_then_halt(&mut bus, &[0xF3, 0xA6]);
    run_until_halted(&mut cpu, &mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn rep_cmpsb_crossing_destination_segment_limit_raises_general_protection() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    shrink_only_extra_segment_limit(&mut state, 4);
    state.set_ecx(8);
    state.set_esi(0x200);
    state.set_edi(0);
    cpu.load_state(&state);

    place_then_halt(&mut bus, &[0xF3, 0xA6]);
    run_until_halted(&mut cpu, &mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn rep_cmpsw_crossing_source_segment_limit_raises_general_protection() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    shrink_only_data_segment_limit(&mut state, 5);
    state.set_ecx(4);
    state.set_esi(0);
    state.set_edi(0x200);
    cpu.load_state(&state);

    place_then_halt(&mut bus, &[0xF3, 0xA7]);
    run_until_halted(&mut cpu, &mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn rep_cmpsw_crossing_destination_segment_limit_raises_general_protection() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    shrink_only_extra_segment_limit(&mut state, 5);
    state.set_ecx(4);
    state.set_esi(0x200);
    state.set_edi(0);
    cpu.load_state(&state);

    place_then_halt(&mut bus, &[0xF3, 0xA7]);
    run_until_halted(&mut cpu, &mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn rep_stosb_crossing_destination_segment_limit_raises_general_protection() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    shrink_only_extra_segment_limit(&mut state, 4);
    state.set_eax(0x77);
    state.set_ecx(8);
    state.set_edi(0);
    cpu.load_state(&state);

    place_then_halt(&mut bus, &[0xF3, 0xAA]);
    run_until_halted(&mut cpu, &mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn rep_stosw_crossing_destination_segment_limit_raises_general_protection() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    shrink_only_extra_segment_limit(&mut state, 5);
    state.set_eax(0x1234);
    state.set_ecx(4);
    state.set_edi(0);
    cpu.load_state(&state);

    place_then_halt(&mut bus, &[0xF3, 0xAB]);
    run_until_halted(&mut cpu, &mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn rep_lodsb_crossing_source_segment_limit_raises_general_protection() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    shrink_only_data_segment_limit(&mut state, 4);
    state.set_ecx(8);
    state.set_esi(0);
    cpu.load_state(&state);

    place_then_halt(&mut bus, &[0xF3, 0xAC]);
    run_until_halted(&mut cpu, &mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn rep_lodsw_crossing_source_segment_limit_raises_general_protection() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    shrink_only_data_segment_limit(&mut state, 5);
    state.set_ecx(4);
    state.set_esi(0);
    cpu.load_state(&state);

    place_then_halt(&mut bus, &[0xF3, 0xAD]);
    run_until_halted(&mut cpu, &mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn repne_scasb_crossing_destination_segment_limit_raises_general_protection() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    shrink_only_extra_segment_limit(&mut state, 4);
    state.set_eax(0x42); // search target absent in scanned bytes
    state.set_ecx(8);
    state.set_edi(0);
    cpu.load_state(&state);

    // REPNE SCASB = F2 AE.
    place_then_halt(&mut bus, &[0xF2, 0xAE]);
    run_until_halted(&mut cpu, &mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn repne_scasw_crossing_destination_segment_limit_raises_general_protection() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    shrink_only_extra_segment_limit(&mut state, 5);
    state.set_eax(0x9999); // not in memory
    state.set_ecx(4);
    state.set_edi(0);
    cpu.load_state(&state);

    place_then_halt(&mut bus, &[0xF2, 0xAF]);
    run_until_halted(&mut cpu, &mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn rep_insb_crossing_destination_segment_limit_raises_general_protection() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    super::setup::install_io_permission_bitmap(
        &mut bus,
        &mut state,
        super::setup::DEFAULT_IO_MAP_BASE_OFFSET,
        16,
        &[0x80],
    );
    shrink_only_extra_segment_limit(&mut state, 4);
    state.set_ecx(8);
    state.set_edi(0);
    state.set_edx(0x80);
    cpu.load_state(&state);

    // REP INSB = F3 6C.
    place_then_halt(&mut bus, &[0xF3, 0x6C]);
    run_until_halted(&mut cpu, &mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn rep_outsb_crossing_source_segment_limit_raises_general_protection() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    super::setup::install_io_permission_bitmap(
        &mut bus,
        &mut state,
        super::setup::DEFAULT_IO_MAP_BASE_OFFSET,
        16,
        &[0x80],
    );
    shrink_only_data_segment_limit(&mut state, 4);
    state.set_ecx(8);
    state.set_esi(0);
    state.set_edx(0x80);
    cpu.load_state(&state);

    // REP OUTSB = F3 6E.
    place_then_halt(&mut bus, &[0xF3, 0x6E]);
    run_until_halted(&mut cpu, &mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn rep_movsb_with_segment_override_crossing_overridden_source_limit_raises_general_protection() {
    // Verify the segment-override path also enforces the override segment's
    // limit (not the default DS limit).
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_fs(super::setup::SELECTOR_RING0_DATA);
    state.seg_bases[cpu::SegReg32::FS as usize] = SHARED_DATA_BASE + 0x1000;
    state.seg_limits[cpu::SegReg32::FS as usize] = 4;
    state.seg_rights[cpu::SegReg32::FS as usize] = RIGHTS_RING0_DATA_WRITABLE_ACCESSED;
    state.seg_valid[cpu::SegReg32::FS as usize] = true;
    state.set_ecx(8);
    state.set_esi(0);
    state.set_edi(0x300);
    cpu.load_state(&state);

    // FS-prefix REP MOVSB = 64 F3 A4.
    place_then_halt(&mut bus, &[0x64, 0xF3, 0xA4]);
    run_until_halted(&mut cpu, &mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}
