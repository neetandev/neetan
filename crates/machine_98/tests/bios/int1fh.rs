use common::Bus;

use super::{
    TEST_CODE, boot_and_run_f, boot_and_run_ra, boot_and_run_vm, boot_and_run_vx, create_machine_f,
    create_machine_ra, create_machine_vm, create_machine_vx, read_ivt_vector, write_bytes,
};

const RESULT: u32 = 0x0600;
const DESCRIPTOR_BASE: u32 = 0x0700;
const DESCRIPTOR_DATA: u32 = DESCRIPTOR_BASE + 0x10;
const SRC_DATA: u32 = 0x5000;
const DST_DATA: u32 = 0x6000;
const INT1FH_BUDGET: u64 = 2_000_000;

#[rustfmt::skip]
fn make_int1fh_call_with_cf_set(ah: u8) -> Vec<u8> {
    vec![
        0xF9,                 // STC (set CF=1)
        0xB4, ah,             // MOV AH, ah
        0xCD, 0x1F,           // INT 0x1F
        0x9C,                 // PUSHF
        0x58,                 // POP AX
        0x25, 0x01, 0x00,     // AND AX, 0x0001
        0xA3, 0x00, 0x06,     // MOV [RESULT], AX
        0xF4,                 // HLT
    ]
}

#[rustfmt::skip]
fn make_memcpy_code(si: u16, di: u16, cx: u16) -> Vec<u8> {
    let bx = DESCRIPTOR_BASE as u16;
    vec![
        0x31, 0xC0,                                       // XOR AX, AX
        0x8E, 0xC0,                                       // MOV ES, AX
        0xBB, (bx & 0xFF) as u8, (bx >> 8) as u8,         // MOV BX, DESCRIPTOR_BASE
        0xBE, (si & 0xFF) as u8, (si >> 8) as u8,         // MOV SI, si
        0xBF, (di & 0xFF) as u8, (di >> 8) as u8,         // MOV DI, di
        0xB9, (cx & 0xFF) as u8, (cx >> 8) as u8,         // MOV CX, cx
        0xB4, 0x90,                                       // MOV AH, 0x90
        0xCD, 0x1F,                                       // INT 0x1F
        0x9C,                                             // PUSHF
        0x58,                                             // POP AX
        0x25, 0x01, 0x00,                                 // AND AX, 0x0001
        0xA3, 0x00, 0x06,                                 // MOV [RESULT], AX
        0xF4,                                             // HLT
    ]
}

/// Writes a pair of 80286-format GDT entries at ES:BX+0x10.
/// Access rights byte is set to 0x93 (present, DPL=0, data, writable).
fn write_descriptor_table(
    bus: &mut impl Bus,
    src_base: u32,
    src_limit: u16,
    dst_base: u32,
    dst_limit: u16,
) {
    #[rustfmt::skip]
    let desc: [u8; 16] = [
        // Source descriptor (GDT entry 2)
        (src_limit & 0xFF) as u8, (src_limit >> 8) as u8,
        (src_base & 0xFF) as u8, ((src_base >> 8) & 0xFF) as u8, ((src_base >> 16) & 0xFF) as u8,
        0x93, 0x00, 0x00,
        // Destination descriptor (GDT entry 3)
        (dst_limit & 0xFF) as u8, (dst_limit >> 8) as u8,
        (dst_base & 0xFF) as u8, ((dst_base >> 8) & 0xFF) as u8, ((dst_base >> 16) & 0xFF) as u8,
        0x93, 0x00, 0x00,
    ];
    write_bytes(bus, DESCRIPTOR_DATA, &desc);
}

fn write_test_pattern(bus: &mut impl Bus, addr: u32, count: usize) {
    for i in 0..count {
        bus.write_byte(addr + i as u32, 0xA0u8.wrapping_add(i as u8));
    }
}

/// §14 INT 1Fh - Vector Setup
///
/// The VM BIOS maps INT 1Fh to an expansion ROM (segment 0xD800), not the
/// main BIOS ROM area. VX and RA map it to the BIOS ROM (>= 0xFD80).
#[test]
fn int1fh_vector_f() {
    let mut machine = create_machine_f();
    let _cycles = boot_to_halt!(machine);
    let state = machine.inspection_state();

    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x1F);
    assert!(
        segment != 0 || offset != 0,
        "INT 1Fh vector should be non-zero (got {segment:#06X}:{offset:#06X})"
    );
}

#[test]
fn int1fh_vector_vm() {
    let mut machine = create_machine_vm();
    let _cycles = boot_to_halt!(machine);
    let state = machine.inspection_state();

    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x1F);
    assert!(
        segment != 0 || offset != 0,
        "INT 1Fh vector should be non-zero (got {segment:#06X}:{offset:#06X})"
    );
}

#[test]
fn int1fh_vector_vx() {
    let mut machine = create_machine_vx();
    let _cycles = boot_to_halt!(machine);
    let state = machine.inspection_state();

    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x1F);
    assert!(
        segment >= 0xFD80,
        "INT 1Fh segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

#[test]
fn int1fh_vector_ra() {
    let mut machine = create_machine_ra();
    let _cycles = boot_to_halt!(machine);
    let state = machine.inspection_state();

    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x1F);
    assert!(
        segment >= 0xFD80,
        "INT 1Fh segment should be in BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );
}

/// §14 INT 1Fh - Dispatch: AH Below 0x80
///
/// The VM BIOS handler (expansion ROM at 0xD800) clears CF even for AH < 0x80.
/// VX and RA preserve CF (handler returns without modifying flags on stack).
#[test]
fn int1fh_ah_below_80h_clears_cf_f() {
    let code = make_int1fh_call_with_cf_set(0x00);
    let (mut machine, _) = boot_and_run_f(&code, &[], INT1FH_BUDGET);
    assert_eq!(
        machine.bus.read_word(RESULT),
        0,
        "F: CF should be 0 (VM handler clears CF for all calls)"
    );
}

#[test]
fn int1fh_ah_below_80h_clears_cf_vm() {
    let code = make_int1fh_call_with_cf_set(0x00);
    let (mut machine, _) = boot_and_run_vm(&code, &[], INT1FH_BUDGET);
    assert_eq!(
        machine.bus.read_word(RESULT),
        0,
        "VM: CF should be 0 (VM handler clears CF for all calls)"
    );
}

#[test]
fn int1fh_ah_below_80h_preserves_cf_vx() {
    let code = make_int1fh_call_with_cf_set(0x00);
    let (mut machine, _) = boot_and_run_vx(&code, &[], INT1FH_BUDGET);
    assert_eq!(
        machine.bus.read_word(RESULT),
        1,
        "CF should remain 1 when AH < 0x80 (handler does not modify flags)"
    );
}

#[test]
fn int1fh_ah_below_80h_preserves_cf_ra() {
    let code = make_int1fh_call_with_cf_set(0x00);
    let (mut machine, _) = boot_and_run_ra(&code, &[], INT1FH_BUDGET);
    assert_eq!(
        machine.bus.read_word(RESULT),
        1,
        "CF should remain 1 when AH < 0x80 (handler does not modify flags)"
    );
}

/// §14 INT 1Fh - Dispatch: AH=0x80 Clears CF
#[test]
fn int1fh_ah_80h_clears_cf_f() {
    let code = make_int1fh_call_with_cf_set(0x80);
    let (mut machine, _) = boot_and_run_f(&code, &[], INT1FH_BUDGET);
    assert_eq!(
        machine.bus.read_word(RESULT),
        0,
        "CF should be cleared when AH=0x80 (bit 7 set, bit 4 clear)"
    );
}

#[test]
fn int1fh_ah_80h_clears_cf_vm() {
    let code = make_int1fh_call_with_cf_set(0x80);
    let (mut machine, _) = boot_and_run_vm(&code, &[], INT1FH_BUDGET);
    assert_eq!(
        machine.bus.read_word(RESULT),
        0,
        "CF should be cleared when AH=0x80 (bit 7 set, bit 4 clear)"
    );
}

#[test]
fn int1fh_ah_80h_clears_cf_vx() {
    let code = make_int1fh_call_with_cf_set(0x80);
    let (mut machine, _) = boot_and_run_vx(&code, &[], INT1FH_BUDGET);
    assert_eq!(
        machine.bus.read_word(RESULT),
        0,
        "CF should be cleared when AH=0x80 (bit 7 set, bit 4 clear)"
    );
}

#[test]
fn int1fh_ah_80h_clears_cf_ra() {
    let code = make_int1fh_call_with_cf_set(0x80);
    let (mut machine, _) = boot_and_run_ra(&code, &[], INT1FH_BUDGET);
    assert_eq!(
        machine.bus.read_word(RESULT),
        0,
        "CF should be cleared when AH=0x80 (bit 7 set, bit 4 clear)"
    );
}

/// §14.1 AH=0x90 - Basic Memory Copy
///
/// The VM BIOS does not implement AH=0x90 (the INT 1Fh vector on VM points to
/// an expansion ROM at 0xD800, not to a memory copy handler). AH=0x90 is a
/// 80286+ feature that uses protected mode for the transfer.
#[test]
fn int1fh_memcpy_basic_vx() {
    let mut machine = create_machine_vx();
    boot_to_halt!(machine);

    write_test_pattern(&mut machine.bus, SRC_DATA, 16);
    write_descriptor_table(&mut machine.bus, SRC_DATA, 0xFFFF, DST_DATA, 0xFFFF);

    let code = make_memcpy_code(0, 0, 16);
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I286State::default();
        s.ip = TEST_CODE as u16;
        s.set_sp(0x4000);
        s
    });
    machine.run_for(INT1FH_BUDGET);

    assert_eq!(
        machine.bus.read_word(RESULT),
        0,
        "CF should be 0 on successful copy"
    );
    for i in 0..16u32 {
        assert_eq!(
            machine.bus.read_byte(DST_DATA + i),
            0xA0u8.wrapping_add(i as u8),
            "Byte {i} at destination should match source pattern"
        );
    }
}

#[test]
fn int1fh_memcpy_basic_ra() {
    let mut machine = create_machine_ra();
    boot_to_halt!(machine);

    write_test_pattern(&mut machine.bus, SRC_DATA, 16);
    write_descriptor_table(&mut machine.bus, SRC_DATA, 0xFFFF, DST_DATA, 0xFFFF);

    let code = make_memcpy_code(0, 0, 16);
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I386State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_esp(0x4000);
        s
    });
    machine.run_for(INT1FH_BUDGET);

    assert_eq!(
        machine.bus.read_word(RESULT),
        0,
        "CF should be 0 on successful copy"
    );
    for i in 0..16u32 {
        assert_eq!(
            machine.bus.read_byte(DST_DATA + i),
            0xA0u8.wrapping_add(i as u8),
            "Byte {i} at destination should match source pattern"
        );
    }
}

/// §14.1 AH=0x90 - Copy With Non-Zero Offsets.
#[test]
fn int1fh_memcpy_with_offsets_vx() {
    let mut machine = create_machine_vx();
    boot_to_halt!(machine);

    write_test_pattern(&mut machine.bus, SRC_DATA, 32);
    write_descriptor_table(&mut machine.bus, SRC_DATA, 0xFFFF, DST_DATA, 0xFFFF);

    // Copy 8 bytes from SRC_DATA+4 to DST_DATA+8.
    let code = make_memcpy_code(4, 8, 8);
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I286State::default();
        s.ip = TEST_CODE as u16;
        s.set_sp(0x4000);
        s
    });
    machine.run_for(INT1FH_BUDGET);

    assert_eq!(
        machine.bus.read_word(RESULT),
        0,
        "CF should be 0 on successful copy"
    );
    for i in 0..8u32 {
        assert_eq!(
            machine.bus.read_byte(DST_DATA + 8 + i),
            0xA0u8.wrapping_add(4 + i as u8),
            "Byte {i} at DST+8 should match SRC+4 pattern"
        );
    }
}

/// §14.1 AH=0x90 - Copy With Non-Zero Offsets.
#[test]
fn int1fh_memcpy_with_offsets_ra() {
    let mut machine = create_machine_ra();
    boot_to_halt!(machine);

    write_test_pattern(&mut machine.bus, SRC_DATA, 32);
    write_descriptor_table(&mut machine.bus, SRC_DATA, 0xFFFF, DST_DATA, 0xFFFF);

    let code = make_memcpy_code(4, 8, 8);
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I386State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_esp(0x4000);
        s
    });
    machine.run_for(INT1FH_BUDGET);

    assert_eq!(
        machine.bus.read_word(RESULT),
        0,
        "CF should be 0 on successful copy"
    );
    for i in 0..8u32 {
        assert_eq!(
            machine.bus.read_byte(DST_DATA + 8 + i),
            0xA0u8.wrapping_add(4 + i as u8),
            "Byte {i} at DST+8 should match SRC+4 pattern"
        );
    }
}

/// §14.1 AH=0x90 - BIOS Ignores Descriptor Limits (VX and later only)
///
/// The real BIOS does NOT validate SI/DI against the descriptor limit fields.
/// NP21W adds this validation, but the original ROM performs the copy regardless.
/// The descriptor limit fields are part of the GDT entry format but are not
/// checked by the BIOS copy routine - only the base address is used.
#[test]
fn int1fh_memcpy_ignores_src_limit_vx() {
    let mut machine = create_machine_vx();
    boot_to_halt!(machine);

    // Write pattern at SRC_DATA+0x0010 (beyond the descriptor limit of 0x000F).
    write_test_pattern(&mut machine.bus, SRC_DATA, 0x20);
    write_descriptor_table(&mut machine.bus, SRC_DATA, 0x000F, DST_DATA, 0xFFFF);

    let code = make_memcpy_code(0x0010, 0, 8);
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I286State::default();
        s.ip = TEST_CODE as u16;
        s.set_sp(0x4000);
        s
    });
    machine.run_for(INT1FH_BUDGET);

    assert_eq!(
        machine.bus.read_word(RESULT),
        0,
        "CF should be 0 (real BIOS ignores descriptor limits)"
    );
}

#[test]
fn int1fh_memcpy_ignores_src_limit_ra() {
    let mut machine = create_machine_ra();
    boot_to_halt!(machine);

    write_test_pattern(&mut machine.bus, SRC_DATA, 0x20);
    write_descriptor_table(&mut machine.bus, SRC_DATA, 0x000F, DST_DATA, 0xFFFF);

    let code = make_memcpy_code(0x0010, 0, 8);
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I386State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_esp(0x4000);
        s
    });
    machine.run_for(INT1FH_BUDGET);

    assert_eq!(
        machine.bus.read_word(RESULT),
        0,
        "CF should be 0 (real BIOS ignores descriptor limits)"
    );
}

#[test]
fn int1fh_memcpy_ignores_dst_limit_vx() {
    let mut machine = create_machine_vx();
    boot_to_halt!(machine);

    write_test_pattern(&mut machine.bus, SRC_DATA, 16);
    write_descriptor_table(&mut machine.bus, SRC_DATA, 0xFFFF, DST_DATA, 0x000F);

    let code = make_memcpy_code(0, 0x0010, 8);
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I286State::default();
        s.ip = TEST_CODE as u16;
        s.set_sp(0x4000);
        s
    });
    machine.run_for(INT1FH_BUDGET);

    assert_eq!(
        machine.bus.read_word(RESULT),
        0,
        "CF should be 0 (real BIOS ignores descriptor limits)"
    );
}

#[test]
fn int1fh_memcpy_ignores_dst_limit_ra() {
    let mut machine = create_machine_ra();
    boot_to_halt!(machine);

    write_test_pattern(&mut machine.bus, SRC_DATA, 16);
    write_descriptor_table(&mut machine.bus, SRC_DATA, 0xFFFF, DST_DATA, 0x000F);

    let code = make_memcpy_code(0, 0x0010, 8);
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I386State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_esp(0x4000);
        s
    });
    machine.run_for(INT1FH_BUDGET);

    assert_eq!(
        machine.bus.read_word(RESULT),
        0,
        "CF should be 0 (real BIOS ignores descriptor limits)"
    );
}

const A20_RESULT: u32 = 0x0610;

#[rustfmt::skip]
fn make_a20_memcpy_code(enable_a20: bool) -> Vec<u8> {
    let bx = DESCRIPTOR_BASE as u16;
    let a20_cmd: u8 = if enable_a20 { 0x02 } else { 0x03 };
    vec![
        0xB0, a20_cmd,                                       // MOV AL, a20_cmd
        0xE6, 0xF6,                                          // OUT 0xF6, AL
        0x31, 0xC0,                                          // XOR AX, AX
        0x8E, 0xC0,                                          // MOV ES, AX
        0xBB, (bx & 0xFF) as u8, (bx >> 8) as u8,           // MOV BX, DESCRIPTOR_BASE
        0xBE, 0x00, 0x00,                                    // MOV SI, 0
        0xBF, 0x00, 0x00,                                    // MOV DI, 0
        0xB9, 0x01, 0x00,                                    // MOV CX, 1
        0xB4, 0x90,                                          // MOV AH, 0x90
        0xCD, 0x1F,                                          // INT 0x1F
        0xE4, 0xF6,                                          // IN AL, 0xF6
        0x25, 0x01, 0x00,                                    // AND AX, 0x0001
        0xA3,
            (A20_RESULT & 0xFF) as u8,
            (A20_RESULT >> 8) as u8,                          // MOV [A20_RESULT], AX
        0xF4,                                                 // HLT
    ]
}

#[test]
fn int1fh_block_move_a20_disabled_after_enabled_ra() {
    let mut machine = create_machine_ra();
    boot_to_halt!(machine);

    write_test_pattern(&mut machine.bus, SRC_DATA, 1);
    write_descriptor_table(&mut machine.bus, SRC_DATA, 0xFFFF, DST_DATA, 0xFFFF);

    let code = make_a20_memcpy_code(true);
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I386State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_esp(0x4000);
        s
    });
    machine.run_for(INT1FH_BUDGET);

    let a20_after = machine.bus.read_word(A20_RESULT);
    assert_eq!(
        a20_after, 1,
        "A20 should be disabled after INT 1Fh AH=0x90 (bit 0 = 1)"
    );
}

#[test]
fn int1fh_block_move_a20_disabled_after_disabled_ra() {
    let mut machine = create_machine_ra();
    boot_to_halt!(machine);

    write_test_pattern(&mut machine.bus, SRC_DATA, 1);
    write_descriptor_table(&mut machine.bus, SRC_DATA, 0xFFFF, DST_DATA, 0xFFFF);

    let code = make_a20_memcpy_code(false);
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I386State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_esp(0x4000);
        s
    });
    machine.run_for(INT1FH_BUDGET);

    let a20_after = machine.bus.read_word(A20_RESULT);
    assert_eq!(
        a20_after, 1,
        "A20 should be disabled after INT 1Fh AH=0x90 (bit 0 = 1)"
    );
}
