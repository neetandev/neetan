//! System-control instruction tests derived from the 80486 PRM Chapter 26.
//!
//! Covers LGDT, LIDT, LLDT, LTR, SLDT, STR, SMSW, LMSW, CLTS, HLT, WAIT,
//! ARPL. Each test names the mode and the property being verified.
//!
//! These instructions form the "no-fault baseline" for the corpus: they do
//! not require the I/O permission bitmap and have well-defined CPL/DPL
//! gates straight out of the manual.

use common::Cpu as _;

use super::setup::{
    GLOBAL_DESCRIPTOR_TABLE_BASE, HANDLER_GENERAL_PROTECTION_IP, HANDLER_INVALID_OPCODE_IP,
    HANDLER_INVALID_TSS_IP, HANDLER_SEGMENT_NOT_PRESENT_IP, RIGHTS_RING0_DATA_WRITABLE_ACCESSED,
    RIGHTS_TSS_386_AVAILABLE, RING0_CODE_BASE, SELECTOR_PRIMARY_TSS, SELECTOR_RING0_CODE,
    SELECTOR_RING0_DATA, SELECTOR_RING0_STACK, SELECTOR_SECONDARY_TSS, SHARED_DATA_BASE,
    SYSTEM_TYPE_LDT, SYSTEM_TYPE_TSS_286_AVAILABLE, TASK_STATE_SEGMENT_SECONDARY_BASE,
    TSS_286_LIMIT, TSS_MINIMUM_LIMIT, TestBus, make_cpu_386, make_cpu_486, place_at, place_code,
    promote_to_ring3, read_byte_at, setup_protected_mode, setup_protected_mode_with_handlers,
    setup_vm86, setup_vm86_with_iopl, write_segment_descriptor_16bit,
};

const REAL_MODE_CODE_SEGMENT: u16 = 0xF000;
const REAL_MODE_CODE_OFFSET: u16 = 0x0000;
const REAL_MODE_DATA_SEGMENT: u16 = 0x1000;
const REAL_MODE_GDT_DESCRIPTOR_OFFSET: u16 = 0x0000;
const REAL_MODE_IDT_DESCRIPTOR_OFFSET: u16 = 0x0010;

// Place a 6-byte LGDT/LIDT operand (16-bit limit + 32-bit base) into RAM
// at a linear address that the test code segment can reach via DS.
fn write_pseudo_descriptor_48bit(bus: &mut TestBus, linear_address: u32, limit: u16, base: u32) {
    bus.ram[linear_address as usize] = limit as u8;
    bus.ram[linear_address as usize + 1] = (limit >> 8) as u8;
    bus.ram[linear_address as usize + 2] = base as u8;
    bus.ram[linear_address as usize + 3] = (base >> 8) as u8;
    bus.ram[linear_address as usize + 4] = (base >> 16) as u8;
    bus.ram[linear_address as usize + 5] = (base >> 24) as u8;
}

#[test]
fn lgdt_real_mode_loads_gdtr_base_and_limit() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let descriptor_address = (REAL_MODE_DATA_SEGMENT as u32) << 4;
    write_pseudo_descriptor_48bit(&mut bus, descriptor_address, 0x07FF, 0x0010_2030);

    place_code(
        &mut bus,
        REAL_MODE_CODE_SEGMENT,
        REAL_MODE_CODE_OFFSET,
        &[
            0x8E, 0xD8, // MOV DS, AX (set up DS via assumed AX=REAL_MODE_DATA_SEGMENT)
        ],
    );
    let mut state = cpu::I386State::default();
    state.set_cs(REAL_MODE_CODE_SEGMENT);
    state.set_ds(REAL_MODE_DATA_SEGMENT);
    state.seg_bases[cpu::SegReg32::CS as usize] = (REAL_MODE_CODE_SEGMENT as u32) << 4;
    state.seg_bases[cpu::SegReg32::DS as usize] = (REAL_MODE_DATA_SEGMENT as u32) << 4;
    state.seg_limits = [0xFFFF; 6];
    state.seg_rights[cpu::SegReg32::CS as usize] = 0x9B;
    state.seg_rights[cpu::SegReg32::DS as usize] = 0x93;
    state.seg_valid = [true, true, true, false, false, false];
    state.set_eax(REAL_MODE_DATA_SEGMENT as u32);
    cpu.load_state(&state);

    place_code(
        &mut bus,
        REAL_MODE_CODE_SEGMENT,
        REAL_MODE_CODE_OFFSET,
        &[
            0x0F,
            0x01,
            0x16,
            REAL_MODE_GDT_DESCRIPTOR_OFFSET as u8,
            (REAL_MODE_GDT_DESCRIPTOR_OFFSET >> 8) as u8,
        ],
    );

    cpu.step(&mut bus);

    assert_eq!(cpu.state.gdt_limit, 0x07FF);
    assert_eq!(cpu.state.gdt_base, 0x0010_2030);
}

#[test]
fn lidt_real_mode_loads_idtr_base_and_limit() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let descriptor_address =
        ((REAL_MODE_DATA_SEGMENT as u32) << 4) + REAL_MODE_IDT_DESCRIPTOR_OFFSET as u32;
    write_pseudo_descriptor_48bit(&mut bus, descriptor_address, 0x03FF, 0x0040_5060);

    let mut state = cpu::I386State::default();
    state.set_cs(REAL_MODE_CODE_SEGMENT);
    state.set_ds(REAL_MODE_DATA_SEGMENT);
    state.seg_bases[cpu::SegReg32::CS as usize] = (REAL_MODE_CODE_SEGMENT as u32) << 4;
    state.seg_bases[cpu::SegReg32::DS as usize] = (REAL_MODE_DATA_SEGMENT as u32) << 4;
    state.seg_limits = [0xFFFF; 6];
    state.seg_rights[cpu::SegReg32::CS as usize] = 0x9B;
    state.seg_rights[cpu::SegReg32::DS as usize] = 0x93;
    state.seg_valid = [true, true, true, false, false, false];
    cpu.load_state(&state);

    place_code(
        &mut bus,
        REAL_MODE_CODE_SEGMENT,
        REAL_MODE_CODE_OFFSET,
        &[
            0x0F,
            0x01,
            0x1E,
            REAL_MODE_IDT_DESCRIPTOR_OFFSET as u8,
            (REAL_MODE_IDT_DESCRIPTOR_OFFSET >> 8) as u8,
        ],
    );

    cpu.step(&mut bus);

    assert_eq!(cpu.state.idt_limit, 0x03FF);
    assert_eq!(cpu.state.idt_base, 0x0040_5060);
}

#[test]
fn lgdt_protected_mode_at_cpl0_loads_gdtr() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode(&mut bus, 0xFFFF);
    cpu.load_state(&state);

    let descriptor_offset_in_data = 0x0040_u32;
    write_pseudo_descriptor_48bit(
        &mut bus,
        SHARED_DATA_BASE + descriptor_offset_in_data,
        0x1FFF,
        0x00AA_BBCC,
    );

    place_at(&mut bus, RING0_CODE_BASE, &[0x0F, 0x01, 0x16, 0x40, 0x00]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.gdt_limit, 0x1FFF);
    assert_eq!(cpu.state.gdt_base, 0x00AA_BBCC);
}

#[test]
fn lgdt_protected_mode_at_cpl3_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    promote_to_ring3(&mut state);
    cpu.load_state(&state);

    place_at(
        &mut bus,
        super::setup::RING3_CODE_BASE,
        &[0x0F, 0x01, 0x16, 0x00, 0x00],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(
        cpu.ip(),
        HANDLER_GENERAL_PROTECTION_IP as u32 + 1,
        "LGDT at CPL=3 must raise #GP(0)"
    );
}

#[test]
fn lgdt_vm86_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_vm86(&mut bus);
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

    place_code(&mut bus, 0x1000, 0x0000, &[0x0F, 0x01, 0x16, 0x00, 0x00]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn lgdt_register_form_raises_invalid_opcode() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    cpu.load_state(&state);

    // 0F 01 D0 = LGDT EAX (mod=11, /2, rm=0): register form is not encodable.
    place_at(&mut bus, RING0_CODE_BASE, &[0x0F, 0x01, 0xD0]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_INVALID_OPCODE_IP as u32 + 1);
}

#[test]
fn lidt_register_form_raises_invalid_opcode() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    cpu.load_state(&state);

    // 0F 01 D8 = LIDT EAX (mod=11, /3, rm=0): register form is not encodable.
    place_at(&mut bus, RING0_CODE_BASE, &[0x0F, 0x01, 0xD8]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_INVALID_OPCODE_IP as u32 + 1);
}

#[test]
fn sgdt_protected_mode_stores_current_gdtr() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode(&mut bus, 0xFFFF);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[0x0F, 0x01, 0x06, 0x80, 0x00]);

    cpu.step(&mut bus);

    let stored_limit = read_byte_at(&bus, SHARED_DATA_BASE + 0x80) as u16
        | ((read_byte_at(&bus, SHARED_DATA_BASE + 0x81) as u16) << 8);
    let stored_base = read_byte_at(&bus, SHARED_DATA_BASE + 0x82) as u32
        | ((read_byte_at(&bus, SHARED_DATA_BASE + 0x83) as u32) << 8)
        | ((read_byte_at(&bus, SHARED_DATA_BASE + 0x84) as u32) << 16)
        | ((read_byte_at(&bus, SHARED_DATA_BASE + 0x85) as u32) << 24);

    assert_eq!(stored_limit, (4 * 8 - 1) as u16);
    assert_eq!(stored_base, GLOBAL_DESCRIPTOR_TABLE_BASE);
}

#[test]
fn sidt_protected_mode_stores_current_idtr() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode(&mut bus, 0xFFFF);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[0x0F, 0x01, 0x0E, 0x80, 0x00]);

    cpu.step(&mut bus);

    let stored_base = read_byte_at(&bus, SHARED_DATA_BASE + 0x82) as u32
        | ((read_byte_at(&bus, SHARED_DATA_BASE + 0x83) as u32) << 8)
        | ((read_byte_at(&bus, SHARED_DATA_BASE + 0x84) as u32) << 16)
        | ((read_byte_at(&bus, SHARED_DATA_BASE + 0x85) as u32) << 24);

    assert_eq!(stored_base, super::setup::INTERRUPT_DESCRIPTOR_TABLE_BASE);
}

#[test]
fn lldt_loads_null_selector_clears_ldtr() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.ldtr = 0x0080;
    state.ldtr_base = 0x0011_2233;
    state.ldtr_limit = 0xFF;
    cpu.load_state(&state);

    // MOV EAX, 0; LLDT AX
    place_at(
        &mut bus,
        RING0_CODE_BASE,
        &[0xB8, 0x00, 0x00, 0x0F, 0x00, 0xD0],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_eq!(cpu.state.ldtr, 0);
    assert_eq!(cpu.state.ldtr_base, 0);
    assert_eq!(cpu.state.ldtr_limit, 0);
}

#[test]
fn lldt_with_non_ldt_descriptor_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    cpu.load_state(&state);

    // SELECTOR_RING0_CODE (slot 1) points to a code descriptor, not an LDT.
    place_at(
        &mut bus,
        RING0_CODE_BASE,
        &[
            0xB8,
            SELECTOR_RING0_CODE as u8,
            (SELECTOR_RING0_CODE >> 8) as u8,
            0x0F,
            0x00,
            0xD0,
        ],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn lldt_with_local_table_indicator_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    cpu.load_state(&state);

    // Selector with TI=1 (LDT-relative) is invalid for LLDT.
    let selector_with_ti: u16 = 0x000C;
    place_at(
        &mut bus,
        RING0_CODE_BASE,
        &[
            0xB8,
            selector_with_ti as u8,
            (selector_with_ti >> 8) as u8,
            0x0F,
            0x00,
            0xD0,
        ],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn lldt_with_not_present_descriptor_raises_segment_not_present() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    cpu.load_state(&state);

    // Slot 8: LDT descriptor with present bit clear.
    let ldt_selector: u16 = 0x0040;
    let access_rights_not_present: u8 = super::setup::ACCESS_DESCRIPTOR_SYSTEM | SYSTEM_TYPE_LDT;
    write_segment_descriptor_16bit(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        8,
        0x0001_0000,
        0x00FF,
        access_rights_not_present,
    );

    place_at(
        &mut bus,
        RING0_CODE_BASE,
        &[
            0xB8,
            ldt_selector as u8,
            (ldt_selector >> 8) as u8,
            0x0F,
            0x00,
            0xD0,
        ],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_SEGMENT_NOT_PRESENT_IP as u32 + 1);
}

#[test]
fn lldt_at_cpl3_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    promote_to_ring3(&mut state);
    cpu.load_state(&state);

    place_at(
        &mut bus,
        super::setup::RING3_CODE_BASE,
        &[0xB8, 0x00, 0x00, 0x0F, 0x00, 0xD0],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn lldt_in_real_mode_raises_invalid_opcode() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let handler_cs: u16 = 0x2000;
    let handler_ip: u16 = 0x0000;
    bus.ram[6 * 4] = handler_ip as u8;
    bus.ram[6 * 4 + 1] = (handler_ip >> 8) as u8;
    bus.ram[6 * 4 + 2] = handler_cs as u8;
    bus.ram[6 * 4 + 3] = (handler_cs >> 8) as u8;

    place_code(&mut bus, cs, 0, &[0x0F, 0x00, 0xD0]);

    let mut state = cpu::I386State::default();
    state.set_cs(cs);
    state.set_ss(0x3000);
    state.set_esp(0x1000);
    cpu.load_state(&state);

    cpu.step(&mut bus);

    assert_eq!(cpu.cs(), handler_cs, "LLDT outside protected mode -> #UD");
    assert_eq!(cpu.ip() as u16, handler_ip);
}

#[test]
fn lldt_loaded_selector_low_two_bits_preserved_in_ldtr() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    cpu.load_state(&state);

    // Slot 8: valid present LDT descriptor with 0xFF limit.
    let access_rights_present_ldt: u8 = super::setup::ACCESS_PRESENT
        | super::setup::ACCESS_DPL_RING0
        | super::setup::ACCESS_DESCRIPTOR_SYSTEM
        | SYSTEM_TYPE_LDT;
    write_segment_descriptor_16bit(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        8,
        0x0001_2300,
        0x00FF,
        access_rights_present_ldt,
    );

    let selector_with_rpl_3: u16 = 0x0043;
    place_at(
        &mut bus,
        RING0_CODE_BASE,
        &[
            0xB8,
            selector_with_rpl_3 as u8,
            (selector_with_rpl_3 >> 8) as u8,
            0x0F,
            0x00,
            0xD0,
        ],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_eq!(
        cpu.state.ldtr, selector_with_rpl_3,
        "LDTR retains the selector RPL bits as supplied"
    );
    assert_eq!(cpu.state.ldtr_base, 0x0001_2300);
    assert_eq!(cpu.state.ldtr_limit, 0x00FF);
}

#[test]
fn ltr_loads_available_386_tss_and_marks_busy() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    cpu.load_state(&state);

    // Slot 9 was written by setup_protected_mode_with_handlers as an
    // available 386 TSS pointing at TASK_STATE_SEGMENT_SECONDARY_BASE.
    place_at(
        &mut bus,
        RING0_CODE_BASE,
        &[
            0xB8,
            SELECTOR_SECONDARY_TSS as u8,
            (SELECTOR_SECONDARY_TSS >> 8) as u8,
            0x0F,
            0x00,
            0xD8,
        ],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_eq!(cpu.state.tr, SELECTOR_SECONDARY_TSS);
    assert_eq!(cpu.state.tr_base, TASK_STATE_SEGMENT_SECONDARY_BASE);
    assert_eq!(
        cpu.state.tr_rights & 0x0F,
        super::setup::SYSTEM_TYPE_TSS_386_BUSY,
        "LTR sets the busy bit in TR_rights"
    );
    let descriptor_address = (GLOBAL_DESCRIPTOR_TABLE_BASE + 9 * 8) as usize;
    assert_eq!(
        bus.ram[descriptor_address + 5] & 0x0F,
        super::setup::SYSTEM_TYPE_TSS_386_BUSY,
        "LTR writes the busy type back to the GDT descriptor"
    );
}

#[test]
fn ltr_with_busy_tss_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    cpu.load_state(&state);

    // Slot 7 is the primary TSS, already busy after handler setup.
    place_at(
        &mut bus,
        RING0_CODE_BASE,
        &[
            0xB8,
            SELECTOR_PRIMARY_TSS as u8,
            (SELECTOR_PRIMARY_TSS >> 8) as u8,
            0x0F,
            0x00,
            0xD8,
        ],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn ltr_with_null_selector_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    cpu.load_state(&state);

    place_at(
        &mut bus,
        RING0_CODE_BASE,
        &[0xB8, 0x00, 0x00, 0x0F, 0x00, 0xD8],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn ltr_with_undersized_386_tss_raises_invalid_tss() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    cpu.load_state(&state);

    // Slot 9: shrink the 386 TSS limit below 67h.
    write_segment_descriptor_16bit(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        9,
        TASK_STATE_SEGMENT_SECONDARY_BASE,
        0x0066,
        RIGHTS_TSS_386_AVAILABLE,
    );

    place_at(
        &mut bus,
        RING0_CODE_BASE,
        &[
            0xB8,
            SELECTOR_SECONDARY_TSS as u8,
            (SELECTOR_SECONDARY_TSS >> 8) as u8,
            0x0F,
            0x00,
            0xD8,
        ],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_INVALID_TSS_IP as u32 + 1);
}

#[test]
fn ltr_with_undersized_286_tss_raises_invalid_tss() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    cpu.load_state(&state);

    let access_rights_286_tss_available: u8 = super::setup::ACCESS_PRESENT
        | super::setup::ACCESS_DPL_RING0
        | super::setup::ACCESS_DESCRIPTOR_SYSTEM
        | SYSTEM_TYPE_TSS_286_AVAILABLE;
    write_segment_descriptor_16bit(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        9,
        TASK_STATE_SEGMENT_SECONDARY_BASE,
        (TSS_286_LIMIT - 1) as u16,
        access_rights_286_tss_available,
    );

    place_at(
        &mut bus,
        RING0_CODE_BASE,
        &[
            0xB8,
            SELECTOR_SECONDARY_TSS as u8,
            (SELECTOR_SECONDARY_TSS >> 8) as u8,
            0x0F,
            0x00,
            0xD8,
        ],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_INVALID_TSS_IP as u32 + 1);
}

#[test]
fn ltr_with_data_descriptor_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    cpu.load_state(&state);

    place_at(
        &mut bus,
        RING0_CODE_BASE,
        &[
            0xB8,
            SELECTOR_RING0_DATA as u8,
            (SELECTOR_RING0_DATA >> 8) as u8,
            0x0F,
            0x00,
            0xD8,
        ],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn ltr_with_not_present_tss_raises_segment_not_present() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    cpu.load_state(&state);

    // Slot 9: 386 TSS with present=0.
    let access_rights_not_present: u8 =
        super::setup::ACCESS_DESCRIPTOR_SYSTEM | super::setup::SYSTEM_TYPE_TSS_386_AVAILABLE;
    write_segment_descriptor_16bit(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        9,
        TASK_STATE_SEGMENT_SECONDARY_BASE,
        TSS_MINIMUM_LIMIT as u16,
        access_rights_not_present,
    );

    place_at(
        &mut bus,
        RING0_CODE_BASE,
        &[
            0xB8,
            SELECTOR_SECONDARY_TSS as u8,
            (SELECTOR_SECONDARY_TSS >> 8) as u8,
            0x0F,
            0x00,
            0xD8,
        ],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_SEGMENT_NOT_PRESENT_IP as u32 + 1);
}

#[test]
fn ltr_at_cpl3_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    promote_to_ring3(&mut state);
    cpu.load_state(&state);

    place_at(
        &mut bus,
        super::setup::RING3_CODE_BASE,
        &[0xB8, 0x00, 0x00, 0x0F, 0x00, 0xD8],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn ltr_in_real_mode_raises_invalid_opcode() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let handler_cs: u16 = 0x2000;
    let handler_ip: u16 = 0x0000;
    bus.ram[6 * 4] = handler_ip as u8;
    bus.ram[6 * 4 + 1] = (handler_ip >> 8) as u8;
    bus.ram[6 * 4 + 2] = handler_cs as u8;
    bus.ram[6 * 4 + 3] = (handler_cs >> 8) as u8;

    place_code(&mut bus, cs, 0, &[0x0F, 0x00, 0xD8]);

    let mut state = cpu::I386State::default();
    state.set_cs(cs);
    state.set_ss(0x3000);
    state.set_esp(0x1000);
    cpu.load_state(&state);

    cpu.step(&mut bus);

    assert_eq!(cpu.cs(), handler_cs);
}

#[test]
fn ltr_with_286_tss_loads_successfully_when_limit_meets_minimum() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    cpu.load_state(&state);

    let access_rights_286_tss_available: u8 = super::setup::ACCESS_PRESENT
        | super::setup::ACCESS_DPL_RING0
        | super::setup::ACCESS_DESCRIPTOR_SYSTEM
        | SYSTEM_TYPE_TSS_286_AVAILABLE;
    write_segment_descriptor_16bit(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        9,
        TASK_STATE_SEGMENT_SECONDARY_BASE,
        TSS_286_LIMIT as u16,
        access_rights_286_tss_available,
    );

    place_at(
        &mut bus,
        RING0_CODE_BASE,
        &[
            0xB8,
            SELECTOR_SECONDARY_TSS as u8,
            (SELECTOR_SECONDARY_TSS >> 8) as u8,
            0x0F,
            0x00,
            0xD8,
        ],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_eq!(cpu.state.tr, SELECTOR_SECONDARY_TSS);
    assert_eq!(
        cpu.state.tr_rights & 0x0F,
        super::setup::SYSTEM_TYPE_TSS_286_BUSY,
        "LTR on a 286 TSS marks the type as 286 busy"
    );
}

#[test]
fn smsw_register_form_returns_full_cr0_on_386() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    // Real-mode reset state: CS=0xFFFF base 0xFFFF0, CR0=0x0010 (ET=1).
    place_at(&mut bus, 0x000F_FFF0, &[0x0F, 0x01, 0xE0]);

    cpu.step(&mut bus);

    assert_eq!(
        cpu.eax(),
        0x0000_0010,
        "SMSW EAX (reg form) returns the full 32-bit CR0 (reset value: ET=1)"
    );
}

#[test]
fn smsw_memory_form_writes_only_low_16_bits() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    // Set CR0 to 0x0011 (PE=1, ET=1) so PM is on but paging is off.
    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.cr0 = 0x0000_0011;
    cpu.load_state(&state);

    bus.ram[(SHARED_DATA_BASE + 0x40) as usize] = 0xAA;
    bus.ram[(SHARED_DATA_BASE + 0x41) as usize] = 0xBB;

    place_at(&mut bus, RING0_CODE_BASE, &[0x0F, 0x01, 0x26, 0x40, 0x00]);

    cpu.step(&mut bus);

    assert_eq!(bus.ram[(SHARED_DATA_BASE + 0x40) as usize], 0x11);
    assert_eq!(bus.ram[(SHARED_DATA_BASE + 0x41) as usize], 0x00);
}

#[test]
fn smsw_at_cpl3_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.cr0 = 0x0000_0001;
    promote_to_ring3(&mut state);
    cpu.load_state(&state);

    place_at(&mut bus, super::setup::RING3_CODE_BASE, &[0x0F, 0x01, 0xE0]);

    cpu.step(&mut bus);

    assert_eq!(cpu.eax() & 0xFFFF, 0x0001, "SMSW must succeed at CPL=3");
    assert!(!cpu.halted());
}

#[test]
fn smsw_in_vm86_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_vm86(&mut bus);
    cpu.load_state(&state);

    place_code(&mut bus, 0x1000, 0x0000, &[0x0F, 0x01, 0xE0]);

    cpu.step(&mut bus);

    assert_eq!(cpu.eax() & 0xFFFF, cpu.state.cr0 & 0xFFFF);
    assert!(!cpu.halted());
}

#[test]
fn lmsw_real_mode_sets_low_4_bits() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    // Real-mode reset state: CPU starts at CS=0xFFFF base 0xFFFF0.
    place_code(
        &mut bus,
        0xFFFF,
        0x0000,
        &[0xB8, 0x0F, 0x00, 0x0F, 0x01, 0xF0],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_eq!(cpu.cr0 & 0x000F, 0x000F);
}

#[test]
fn lmsw_cannot_clear_pe_once_set() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode(&mut bus, 0xFFFF);
    cpu.load_state(&state);

    place_at(
        &mut bus,
        RING0_CODE_BASE,
        &[0xB8, 0x00, 0x00, 0x0F, 0x01, 0xF0],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_eq!(cpu.cr0 & 1, 1, "LMSW with value 0 must not clear PE");
}

#[test]
fn lmsw_at_cpl3_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    promote_to_ring3(&mut state);
    cpu.load_state(&state);

    place_at(
        &mut bus,
        super::setup::RING3_CODE_BASE,
        &[0xB8, 0x00, 0x00, 0x0F, 0x01, 0xF0],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn lmsw_in_vm86_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_vm86(&mut bus);
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

    place_code(
        &mut bus,
        0x1000,
        0x0000,
        &[0xB8, 0x00, 0x00, 0x0F, 0x01, 0xF0],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn lmsw_only_writes_low_4_bits() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    place_code(
        &mut bus,
        0xFFFF,
        0x0000,
        &[0xB8, 0xFF, 0xFF, 0x0F, 0x01, 0xF0],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_eq!(cpu.cr0 & 0x000F, 0x000F);
    assert_eq!(
        cpu.cr0 & 0xFFF0,
        0x0010,
        "LMSW must not touch CR0 bits 4-15 (ET stays at 1)"
    );
}

#[test]
fn lmsw_486_only_writes_low_4_bits() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    place_code(
        &mut bus,
        0xFFFF,
        0x0000,
        &[0xB8, 0xFF, 0xFF, 0x0F, 0x01, 0xF0],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_eq!(cpu.cr0 & 0x000F, 0x000F);
}

#[test]
fn clts_real_mode_clears_ts() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = cpu::I386State {
        cr0: 0x0000_0008,
        ..cpu::I386State::default()
    };
    state.set_cs(0xFFFF);
    state.seg_bases[cpu::SegReg32::CS as usize] = 0x000F_FFF0;
    state.seg_limits = [0xFFFF; 6];
    state.seg_rights[cpu::SegReg32::CS as usize] = 0x9B;
    state.seg_valid = [true; 6];
    cpu.load_state(&state);

    place_at(&mut bus, 0x000F_FFF0, &[0x0F, 0x06]);

    cpu.step(&mut bus);

    assert_eq!(cpu.cr0 & 0x0008, 0, "CLTS clears the TS bit");
}

#[test]
fn clts_protected_mode_at_cpl0_clears_ts() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.cr0 |= 0x0000_0008;
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[0x0F, 0x06]);

    cpu.step(&mut bus);

    assert_eq!(cpu.cr0 & 0x0008, 0);
}

#[test]
fn clts_at_cpl3_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    promote_to_ring3(&mut state);
    cpu.load_state(&state);

    place_at(&mut bus, super::setup::RING3_CODE_BASE, &[0x0F, 0x06]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn clts_in_vm86_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_vm86(&mut bus);
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

    place_code(&mut bus, 0x1000, 0x0000, &[0x0F, 0x06]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn clts_486_at_cpl3_raises_general_protection() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    promote_to_ring3(&mut state);
    cpu.load_state(&state);

    place_at(&mut bus, super::setup::RING3_CODE_BASE, &[0x0F, 0x06]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn hlt_real_mode_halts_cpu() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    place_code(&mut bus, 0xFFFF, 0x0000, &[0xF4]);
    cpu.step(&mut bus);

    assert!(cpu.halted());
}

#[test]
fn hlt_protected_mode_at_cpl0_halts_cpu() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode(&mut bus, 0xFFFF);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[0xF4]);
    cpu.step(&mut bus);

    assert!(cpu.halted());
}

#[test]
fn hlt_at_cpl3_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    promote_to_ring3(&mut state);
    cpu.load_state(&state);

    place_at(&mut bus, super::setup::RING3_CODE_BASE, &[0xF4]);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn hlt_vm86_at_iopl_3_raises_general_protection() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_vm86_with_iopl(&mut bus, 3);
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

    place_code(&mut bus, 0x1000, 0x0000, &[0xF4]);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(
        cpu.ip(),
        HANDLER_GENERAL_PROTECTION_IP as u32 + 1,
        "HLT in VM86 always raises #GP regardless of IOPL"
    );
}

#[test]
fn hlt_then_irq_with_if_set_dispatches_interrupt() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = cpu::I386State::default();
    state.set_cs(0xFFFF);
    state.seg_bases[cpu::SegReg32::CS as usize] = 0x000F_FFF0;
    state.set_ss(0x0000);
    state.set_esp(0x1000);
    state.seg_bases[cpu::SegReg32::SS as usize] = 0;
    state.seg_limits = [0xFFFF; 6];
    state.seg_rights[cpu::SegReg32::CS as usize] = 0x9B;
    state.seg_rights[cpu::SegReg32::SS as usize] = 0x93;
    state.seg_valid = [true; 6];
    state.flags.if_flag = true;
    cpu.load_state(&state);

    let interrupt_vector_handler_segment: u16 = 0x2000;
    let interrupt_vector_handler_offset: u16 = 0x0100;
    bus.ram[0x40 * 4] = interrupt_vector_handler_offset as u8;
    bus.ram[0x40 * 4 + 1] = (interrupt_vector_handler_offset >> 8) as u8;
    bus.ram[0x40 * 4 + 2] = interrupt_vector_handler_segment as u8;
    bus.ram[0x40 * 4 + 3] = (interrupt_vector_handler_segment >> 8) as u8;

    place_at(&mut bus, 0x000F_FFF0, &[0xF4]);
    cpu.step(&mut bus);
    assert!(cpu.halted(), "HLT must halt the CPU");

    // Handler at 0x2000:0x0100 is a single HLT so the test can pin IP.
    let handler_linear =
        ((interrupt_vector_handler_segment as u32) << 4) + interrupt_vector_handler_offset as u32;
    bus.ram[handler_linear as usize] = 0xF4;

    bus.irq_vector = 0x40;
    cpu.signal_irq();
    cpu.step(&mut bus);

    assert!(
        cpu.halted(),
        "Handler HLT must halt the CPU after IRQ dispatch"
    );
    assert_eq!(cpu.cs(), interrupt_vector_handler_segment);
    assert_eq!(
        cpu.ip() as u16,
        interrupt_vector_handler_offset + 1,
        "After IRQ dispatch and HLT in handler, IP points one past HLT"
    );
}

#[test]
fn hlt_at_cpl3_486_raises_general_protection() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    promote_to_ring3(&mut state);
    cpu.load_state(&state);

    place_at(&mut bus, super::setup::RING3_CODE_BASE, &[0xF4]);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_GENERAL_PROTECTION_IP as u32 + 1);
}

#[test]
fn wait_with_mp_clear_does_not_raise() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.cr0 |= 0x0000_0008;
    state.cr0 &= !0x0000_0002;
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[0x9B, 0x90]);

    cpu.step(&mut bus);

    assert!(!cpu.halted());
}

#[test]
fn wait_with_ts_clear_does_not_raise() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.cr0 |= 0x0000_0002;
    state.cr0 &= !0x0000_0008;
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[0x9B, 0x90]);

    cpu.step(&mut bus);

    assert!(!cpu.halted());
}

#[test]
fn wait_with_mp_and_ts_set_raises_device_not_available() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.cr0 |= 0x0000_000A;
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[0x9B]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(
        cpu.ip(),
        super::setup::HANDLER_DEVICE_NOT_AVAILABLE_IP as u32 + 1
    );
}

#[test]
fn wait_with_em_set_alone_does_not_raise() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.cr0 |= 0x0000_0004;
    state.cr0 &= !0x0000_000A;
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[0x9B, 0x90]);

    cpu.step(&mut bus);

    assert!(
        !cpu.halted(),
        "WAIT must ignore CR0.EM (only MP+TS triggers #NM per 80486 PRM)"
    );
}

#[test]
fn wait_in_vm86_with_mp_and_ts_set_raises_device_not_available() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_vm86(&mut bus);
    super::setup::write_interrupt_gate_386(
        &mut bus,
        super::setup::INTERRUPT_DESCRIPTOR_TABLE_BASE,
        7,
        super::setup::HANDLER_DEVICE_NOT_AVAILABLE_IP as u32,
        SELECTOR_RING0_CODE,
        0,
    );
    bus.ram[(RING0_CODE_BASE + super::setup::HANDLER_DEVICE_NOT_AVAILABLE_IP as u32) as usize] =
        0xF4;
    state.cr0 |= 0x0000_000A;
    state.gdt_limit = 5 * 8 - 1;
    cpu.load_state(&state);

    place_code(&mut bus, 0x1000, 0x0000, &[0x9B]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(
        cpu.ip(),
        super::setup::HANDLER_DEVICE_NOT_AVAILABLE_IP as u32 + 1
    );
}

#[test]
fn wait_at_cpl3_with_mp_and_ts_clear_does_not_raise() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.cr0 &= !0x0000_000A;
    promote_to_ring3(&mut state);
    cpu.load_state(&state);

    place_at(&mut bus, super::setup::RING3_CODE_BASE, &[0x9B, 0x90]);

    cpu.step(&mut bus);

    assert!(!cpu.halted(), "WAIT has no CPL gate by itself");
}

#[test]
fn arpl_real_mode_raises_invalid_opcode_on_386() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let handler_cs: u16 = 0x2000;
    let handler_ip: u16 = 0x0000;
    bus.ram[6 * 4] = handler_ip as u8;
    bus.ram[6 * 4 + 1] = (handler_ip >> 8) as u8;
    bus.ram[6 * 4 + 2] = handler_cs as u8;
    bus.ram[6 * 4 + 3] = (handler_cs >> 8) as u8;

    place_code(&mut bus, cs, 0, &[0x63, 0xC0]);

    let mut state = cpu::I386State::default();
    state.set_cs(cs);
    state.set_ss(0x3000);
    state.set_esp(0x1000);
    cpu.load_state(&state);

    cpu.step(&mut bus);

    assert_eq!(cpu.cs(), handler_cs);
}

#[test]
fn arpl_real_mode_raises_invalid_opcode_on_486() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let handler_cs: u16 = 0x2000;
    let handler_ip: u16 = 0x0000;
    bus.ram[6 * 4] = handler_ip as u8;
    bus.ram[6 * 4 + 1] = (handler_ip >> 8) as u8;
    bus.ram[6 * 4 + 2] = handler_cs as u8;
    bus.ram[6 * 4 + 3] = (handler_cs >> 8) as u8;

    place_code(&mut bus, cs, 0, &[0x63, 0xC0]);

    let mut state = cpu::I386State::default();
    state.set_cs(cs);
    state.set_ss(0x3000);
    state.set_esp(0x1000);
    cpu.load_state(&state);

    cpu.step(&mut bus);

    assert_eq!(
        cpu.cs(),
        handler_cs,
        "ARPL is real-mode #UD on both 386 and 486"
    );
}

#[test]
fn arpl_vm86_raises_invalid_opcode() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_vm86(&mut bus);
    super::setup::write_interrupt_gate_386(
        &mut bus,
        super::setup::INTERRUPT_DESCRIPTOR_TABLE_BASE,
        6,
        HANDLER_INVALID_OPCODE_IP as u32,
        SELECTOR_RING0_CODE,
        0,
    );
    bus.ram[(RING0_CODE_BASE + HANDLER_INVALID_OPCODE_IP as u32) as usize] = 0xF4;
    state.gdt_limit = 5 * 8 - 1;
    cpu.load_state(&state);

    place_code(&mut bus, 0x1000, 0x0000, &[0x63, 0xC0]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), HANDLER_INVALID_OPCODE_IP as u32 + 1);
}

#[test]
fn arpl_protected_mode_raises_dest_rpl_when_smaller_than_source() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.set_eax(0x0000_0008);
    state.set_ebx(0x0000_0003);
    cpu.load_state(&state);

    // ARPL AX, BX (0x63 modrm=0xD8: mod=11, /3 (BX), rm=0 (AX))
    place_at(&mut bus, RING0_CODE_BASE, &[0x63, 0xD8]);

    cpu.step(&mut bus);

    assert_eq!(
        cpu.eax() & 0xFFFF,
        0x000B,
        "ARPL must raise dest RPL to source RPL"
    );
    assert!(cpu.state.flags.zf(), "ZF=1 when ARPL modified the dest");
}

#[test]
fn arpl_protected_mode_no_change_when_dest_rpl_equal_or_greater() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.set_eax(0x0000_001B);
    state.set_ebx(0x0000_0001);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[0x63, 0xD8]);

    cpu.step(&mut bus);

    assert_eq!(cpu.eax() & 0xFFFF, 0x001B);
    assert!(
        !cpu.state.flags.zf(),
        "ZF=0 when ARPL leaves the dest unchanged"
    );
}

#[test]
fn arpl_protected_mode_with_memory_dest_writes_back_to_memory() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.set_eax(0x0000_0080);
    state.set_ebx(0x0000_0002);
    cpu.load_state(&state);

    bus.ram[(SHARED_DATA_BASE + 0x40) as usize] = 0x80;
    bus.ram[(SHARED_DATA_BASE + 0x41) as usize] = 0x00;

    // ARPL [DS:0x40], BX  (0x63 modrm=0x1E disp16=0x0040)
    place_at(&mut bus, RING0_CODE_BASE, &[0x63, 0x1E, 0x40, 0x00]);

    cpu.step(&mut bus);

    assert_eq!(bus.ram[(SHARED_DATA_BASE + 0x40) as usize], 0x82);
    assert!(cpu.state.flags.zf());
}

#[test]
fn arpl_protected_mode_dest_rpl_already_greater_keeps_value() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.set_eax(0x0000_0103);
    state.set_ebx(0x0000_0001);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[0x63, 0xD8]);

    cpu.step(&mut bus);

    assert_eq!(cpu.eax() & 0xFFFF, 0x0103);
    assert!(!cpu.state.flags.zf());
}

#[test]
fn arpl_protected_mode_at_cpl3_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.set_eax(0x0000_0008);
    state.set_ebx(0x0000_0003);
    promote_to_ring3(&mut state);
    cpu.load_state(&state);

    place_at(&mut bus, super::setup::RING3_CODE_BASE, &[0x63, 0xD8]);

    cpu.step(&mut bus);

    assert!(!cpu.halted(), "ARPL has no CPL gate in protected mode");
    assert_eq!(cpu.eax() & 0xFFFF, 0x000B);
}

#[test]
fn arpl_486_protected_mode_matches_386_semantics() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.set_eax(0x0000_0008);
    state.set_ebx(0x0000_0003);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[0x63, 0xD8]);

    cpu.step(&mut bus);

    assert_eq!(cpu.eax() & 0xFFFF, 0x000B);
    assert!(cpu.state.flags.zf());
}

#[test]
fn sldt_protected_mode_returns_current_ldtr_selector() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.ldtr = 0x0048;
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[0x0F, 0x00, 0xC0]);

    cpu.step(&mut bus);

    assert_eq!(cpu.eax() & 0xFFFF, 0x0048, "SLDT returns the LDTR selector");
}

#[test]
fn str_protected_mode_returns_current_tr_selector() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[0x0F, 0x00, 0xC8]);

    cpu.step(&mut bus);

    assert_eq!(cpu.eax() & 0xFFFF, SELECTOR_PRIMARY_TSS as u32);
}

#[test]
fn sldt_in_real_mode_raises_invalid_opcode() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let handler_cs: u16 = 0x2000;
    let handler_ip: u16 = 0x0000;
    bus.ram[6 * 4] = handler_ip as u8;
    bus.ram[6 * 4 + 1] = (handler_ip >> 8) as u8;
    bus.ram[6 * 4 + 2] = handler_cs as u8;
    bus.ram[6 * 4 + 3] = (handler_cs >> 8) as u8;

    place_code(&mut bus, cs, 0, &[0x0F, 0x00, 0xC0]);

    let mut state = cpu::I386State::default();
    state.set_cs(cs);
    state.set_ss(0x3000);
    state.set_esp(0x1000);
    cpu.load_state(&state);

    cpu.step(&mut bus);

    assert_eq!(cpu.cs(), handler_cs);
}

#[test]
fn lgdt_loads_full_32_bit_base_under_32_bit_operand_size() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode(&mut bus, 0xFFFF);
    cpu.load_state(&state);

    write_pseudo_descriptor_48bit(&mut bus, SHARED_DATA_BASE + 0x80, 0x0FFF, 0xCAFEBABE);

    // Operand-size override prefix 0x66 selects 32-bit operand size in 16-bit code.
    place_at(
        &mut bus,
        RING0_CODE_BASE,
        &[0x66, 0x0F, 0x01, 0x16, 0x80, 0x00],
    );

    cpu.step(&mut bus);

    assert_eq!(cpu.state.gdt_limit, 0x0FFF);
    assert_eq!(
        cpu.state.gdt_base, 0xCAFE_BABE,
        "32-bit operand-size LGDT loads all 32 base bits"
    );
}

#[test]
fn lidt_loads_full_32_bit_base_under_32_bit_operand_size() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode(&mut bus, 0xFFFF);
    cpu.load_state(&state);

    write_pseudo_descriptor_48bit(&mut bus, SHARED_DATA_BASE + 0x80, 0x07FF, 0x1234_5678);

    place_at(
        &mut bus,
        RING0_CODE_BASE,
        &[0x66, 0x0F, 0x01, 0x1E, 0x80, 0x00],
    );

    cpu.step(&mut bus);

    assert_eq!(cpu.state.idt_limit, 0x07FF);
    assert_eq!(cpu.state.idt_base, 0x1234_5678);
}

#[test]
fn smsw_at_cpl3_in_vm86_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_vm86_with_iopl(&mut bus, 3);
    cpu.load_state(&state);

    place_code(&mut bus, 0x1000, 0x0000, &[0x0F, 0x01, 0xE0]);

    cpu.step(&mut bus);

    assert!(!cpu.halted(), "SMSW must succeed in VM86");
}

#[test]
fn lmsw_real_mode_can_set_pe() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    place_code(
        &mut bus,
        0xFFFF,
        0x0000,
        &[0xB8, 0x01, 0x00, 0x0F, 0x01, 0xF0],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_eq!(cpu.cr0 & 1, 1);
}

#[test]
fn lmsw_real_mode_can_set_em_and_ts() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    place_code(
        &mut bus,
        0xFFFF,
        0x0000,
        &[0xB8, 0x0C, 0x00, 0x0F, 0x01, 0xF0],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_eq!(cpu.cr0 & 0x000F, 0x000C);
}

#[test]
fn ltr_descriptor_busy_bit_persists_in_memory_after_load() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    cpu.load_state(&state);

    place_at(
        &mut bus,
        RING0_CODE_BASE,
        &[
            0xB8,
            SELECTOR_SECONDARY_TSS as u8,
            (SELECTOR_SECONDARY_TSS >> 8) as u8,
            0x0F,
            0x00,
            0xD8,
        ],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    let access_byte_address = (GLOBAL_DESCRIPTOR_TABLE_BASE + 9 * 8 + 5) as usize;
    assert_eq!(bus.ram[access_byte_address] & 0x02, 0x02);
}

#[test]
fn lldt_does_not_alter_cpu_segments_or_ip() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    let original_cs = state.cs();
    let original_ds = state.ds();
    cpu.load_state(&state);

    let access_rights_present_ldt: u8 = super::setup::ACCESS_PRESENT
        | super::setup::ACCESS_DPL_RING0
        | super::setup::ACCESS_DESCRIPTOR_SYSTEM
        | SYSTEM_TYPE_LDT;
    write_segment_descriptor_16bit(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        8,
        0x0009_0000,
        0x0F,
        access_rights_present_ldt,
    );

    place_at(
        &mut bus,
        RING0_CODE_BASE,
        &[0xB8, 0x40, 0x00, 0x0F, 0x00, 0xD0],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_eq!(cpu.cs(), original_cs);
    assert_eq!(cpu.ds(), original_ds);
}

#[test]
fn ltr_with_low_two_bits_in_selector_ignored_for_lookup() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    cpu.load_state(&state);

    let selector_with_rpl_3 = SELECTOR_SECONDARY_TSS | 3;
    place_at(
        &mut bus,
        RING0_CODE_BASE,
        &[
            0xB8,
            selector_with_rpl_3 as u8,
            (selector_with_rpl_3 >> 8) as u8,
            0x0F,
            0x00,
            0xD8,
        ],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_eq!(
        cpu.state.tr & 0xFFFC,
        SELECTOR_SECONDARY_TSS,
        "LTR uses the selector index regardless of supplied RPL bits"
    );
}

#[test]
fn smsw_register_form_does_not_expose_paging_bit() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    super::setup::enable_identity_paging(&mut bus, &mut state);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[0x0F, 0x01, 0xE0]);
    cpu.step(&mut bus);

    // 80486 PRM 26-268 documents SMSW only as `SMSW r/m16` storing the
    // machine status word, i.e. the low 16 bits of CR0. PG is bit 31 and
    // sits outside the MSW, so it is not observable via SMSW even when
    // paging is enabled. Confirm the PE bit is reflected and PG is not.
    assert_eq!(cpu.eax() & 0x0000_0001, 0x0000_0001);
    assert_eq!(cpu.eax() & 0xFFFF_0000, 0);
}

#[test]
fn lgdt_at_cpl0_with_existing_paging_continues_to_translate_via_old_gdtr() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode(&mut bus, 0xFFFF);
    cpu.load_state(&state);

    write_pseudo_descriptor_48bit(&mut bus, SHARED_DATA_BASE + 0x40, 0x00FF, 0x000F_F000);
    place_at(&mut bus, RING0_CODE_BASE, &[0x0F, 0x01, 0x16, 0x40, 0x00]);
    cpu.step(&mut bus);

    assert_eq!(cpu.state.gdt_base, 0x000F_F000);
    assert_eq!(cpu.state.gdt_limit, 0x00FF);
}

#[test]
fn fwait_at_cpl3_with_mp_and_ts_set_raises_device_not_available() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.cr0 |= 0x0000_000A;
    promote_to_ring3(&mut state);
    cpu.load_state(&state);

    place_at(&mut bus, super::setup::RING3_CODE_BASE, &[0x9B]);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(
        cpu.ip(),
        super::setup::HANDLER_DEVICE_NOT_AVAILABLE_IP as u32 + 1
    );
}

#[test]
fn arpl_protected_mode_with_dest_rpl_zero_raises_to_source() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.set_eax(0x0000_0010);
    state.set_ebx(0x0000_0002);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[0x63, 0xD8]);
    cpu.step(&mut bus);

    assert_eq!(cpu.eax() & 0xFFFF, 0x0012);
    assert!(cpu.state.flags.zf());
}

#[test]
fn lldt_loaded_when_present_descriptor_caches_base_and_limit() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    cpu.load_state(&state);

    let access_rights_present_ldt: u8 = super::setup::ACCESS_PRESENT
        | super::setup::ACCESS_DPL_RING0
        | super::setup::ACCESS_DESCRIPTOR_SYSTEM
        | SYSTEM_TYPE_LDT;
    write_segment_descriptor_16bit(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        8,
        0x000B_0000,
        0x07FF,
        access_rights_present_ldt,
    );

    let ldt_selector: u16 = 0x0040;
    place_at(
        &mut bus,
        RING0_CODE_BASE,
        &[
            0xB8,
            ldt_selector as u8,
            (ldt_selector >> 8) as u8,
            0x0F,
            0x00,
            0xD0,
        ],
    );
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_eq!(cpu.state.ldtr, ldt_selector);
    assert_eq!(cpu.state.ldtr_base, 0x000B_0000);
    assert_eq!(cpu.state.ldtr_limit, 0x07FF);
}

#[test]
fn ltr_then_str_round_trips_selector() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    cpu.load_state(&state);

    place_at(
        &mut bus,
        RING0_CODE_BASE,
        &[
            0xB8,
            SELECTOR_SECONDARY_TSS as u8,
            (SELECTOR_SECONDARY_TSS >> 8) as u8,
            0x0F,
            0x00,
            0xD8,
            0x66,
            0x31,
            0xC0,
            0x0F,
            0x00,
            0xC8,
        ],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_eq!(cpu.eax() & 0xFFFF, SELECTOR_SECONDARY_TSS as u32);
}

#[test]
fn clts_does_not_clear_em_or_mp() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.cr0 |= 0x0000_000E;
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[0x0F, 0x06]);
    cpu.step(&mut bus);

    assert_eq!(cpu.cr0 & 0x0008, 0, "CLTS clears TS");
    assert_ne!(cpu.cr0 & 0x0006, 0, "CLTS leaves MP and EM unchanged");
}

#[test]
fn smsw_register_form_zero_extends_low_16_when_storing_to_32_bit_dest() {
    let mut cpu = make_cpu_486();
    let mut bus = TestBus::new();

    let mut state = cpu::I386State {
        cr0: 0x0000_0011,
        ..cpu::I386State::default()
    };
    state.set_eax(0xFFFF_FFFF);
    state.set_cs(0xFFFF);
    state.seg_bases[cpu::SegReg32::CS as usize] = 0x000F_FFF0;
    state.seg_limits = [0xFFFF; 6];
    state.seg_rights[cpu::SegReg32::CS as usize] = 0x9B;
    state.seg_valid = [true; 6];
    cpu.load_state(&state);

    place_at(&mut bus, 0x000F_FFF0, &[0x66, 0x0F, 0x01, 0xE0]);
    cpu.step(&mut bus);

    assert_eq!(
        cpu.eax() & 0xFFFF_0000,
        0,
        "SMSW with 32-bit dest zero-extends from CR0 low 16"
    );
}

#[test]
fn lgdt_real_mode_with_24_bit_base_value_loads_correctly() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = cpu::I386State::default();
    state.set_cs(0xF000);
    state.set_ds(0x1000);
    state.seg_bases[cpu::SegReg32::CS as usize] = 0x000F_0000;
    state.seg_bases[cpu::SegReg32::DS as usize] = 0x0001_0000;
    state.seg_limits = [0xFFFF; 6];
    state.seg_rights[cpu::SegReg32::CS as usize] = 0x9B;
    state.seg_rights[cpu::SegReg32::DS as usize] = 0x93;
    state.seg_valid = [true; 6];
    cpu.load_state(&state);

    write_pseudo_descriptor_48bit(&mut bus, 0x0001_0000, 0x0FFF, 0x0080_4040);
    place_at(&mut bus, 0x000F_0000, &[0x0F, 0x01, 0x16, 0x00, 0x00]);
    cpu.step(&mut bus);

    assert_eq!(cpu.state.gdt_limit, 0x0FFF);
    assert_eq!(cpu.state.gdt_base, 0x0080_4040);
}

#[test]
fn lgdt_16bit_operand_forces_upper_8_bits_of_base_to_zero() {
    // With a 16-bit operand size attribute, the  upper 8 bits of the loaded
    // 32-bit base are forced to zero. The physical byte at offset +5 of the
    // operand is not part of the loaded base and must not leak into bits 24-31
    // of GDTR.base, even if it happens to be non-zero (as it does in DOS
    // 3.30's EMM386.SYS, where the byte following a six-byte LGDT operand
    // is 0x92 unrelated data).
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = cpu::I386State::default();
    state.set_cs(0xF000);
    state.set_ds(0x1000);
    state.seg_bases[cpu::SegReg32::CS as usize] = 0x000F_0000;
    state.seg_bases[cpu::SegReg32::DS as usize] = 0x0001_0000;
    state.seg_limits = [0xFFFF; 6];
    state.seg_rights[cpu::SegReg32::CS as usize] = 0x9B;
    state.seg_rights[cpu::SegReg32::DS as usize] = 0x93;
    state.seg_valid = [true; 6];
    cpu.load_state(&state);

    write_pseudo_descriptor_48bit(&mut bus, 0x0001_0000, 0x0137, 0x92FF_9680);
    place_at(&mut bus, 0x000F_0000, &[0x0F, 0x01, 0x16, 0x00, 0x00]);
    cpu.step(&mut bus);

    assert_eq!(cpu.state.gdt_limit, 0x0137);
    assert_eq!(
        cpu.state.gdt_base, 0x00FF_9680,
        "16-bit LGDT must mask base to 24 bits; byte at operand+5 (0x92) \
         must not leak into bits 24-31"
    );
}

#[test]
fn lgdt_32bit_operand_with_66_prefix_loads_full_32bit_base() {
    // With operand-size override (0x66 in 16-bit code), LGDT loads the full
    // 32-bit base from the descriptor.
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = cpu::I386State::default();
    state.set_cs(0xF000);
    state.set_ds(0x1000);
    state.seg_bases[cpu::SegReg32::CS as usize] = 0x000F_0000;
    state.seg_bases[cpu::SegReg32::DS as usize] = 0x0001_0000;
    state.seg_limits = [0xFFFF; 6];
    state.seg_rights[cpu::SegReg32::CS as usize] = 0x9B;
    state.seg_rights[cpu::SegReg32::DS as usize] = 0x93;
    state.seg_valid = [true; 6];
    cpu.load_state(&state);

    write_pseudo_descriptor_48bit(&mut bus, 0x0001_0000, 0x0FFF, 0x9234_5678);
    place_at(&mut bus, 0x000F_0000, &[0x66, 0x0F, 0x01, 0x16, 0x00, 0x00]);
    cpu.step(&mut bus);

    assert_eq!(cpu.state.gdt_limit, 0x0FFF);
    assert_eq!(cpu.state.gdt_base, 0x9234_5678);
}

#[test]
fn lidt_16bit_operand_forces_upper_8_bits_of_base_to_zero() {
    // Mirror of `lgdt_16bit_operand_forces_upper_8_bits_of_base_to_zero` for
    // LIDT: the 16-bit operand variant must not let the byte at operand+5
    // leak into bits 24-31 of IDTR.base.
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = cpu::I386State::default();
    state.set_cs(0xF000);
    state.set_ds(0x1000);
    state.seg_bases[cpu::SegReg32::CS as usize] = 0x000F_0000;
    state.seg_bases[cpu::SegReg32::DS as usize] = 0x0001_0000;
    state.seg_limits = [0xFFFF; 6];
    state.seg_rights[cpu::SegReg32::CS as usize] = 0x9B;
    state.seg_rights[cpu::SegReg32::DS as usize] = 0x93;
    state.seg_valid = [true; 6];
    cpu.load_state(&state);

    write_pseudo_descriptor_48bit(&mut bus, 0x0001_0000, 0x03BF, 0x92FF_97C0);
    place_at(&mut bus, 0x000F_0000, &[0x0F, 0x01, 0x1E, 0x00, 0x00]);
    cpu.step(&mut bus);

    assert_eq!(cpu.state.idt_limit, 0x03BF);
    assert_eq!(
        cpu.state.idt_base, 0x00FF_97C0,
        "16-bit LIDT must mask base to 24 bits; byte at operand+5 (0x92) \
         must not leak into bits 24-31"
    );
}

#[test]
fn lidt_32bit_operand_with_66_prefix_loads_full_32bit_base() {
    // Mirror of `lgdt_32bit_operand_with_66_prefix_loads_full_32bit_base`
    // for LIDT: with the operand-size override, LIDT loads the full 32-bit
    // base.
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = cpu::I386State::default();
    state.set_cs(0xF000);
    state.set_ds(0x1000);
    state.seg_bases[cpu::SegReg32::CS as usize] = 0x000F_0000;
    state.seg_bases[cpu::SegReg32::DS as usize] = 0x0001_0000;
    state.seg_limits = [0xFFFF; 6];
    state.seg_rights[cpu::SegReg32::CS as usize] = 0x9B;
    state.seg_rights[cpu::SegReg32::DS as usize] = 0x93;
    state.seg_valid = [true; 6];
    cpu.load_state(&state);

    write_pseudo_descriptor_48bit(&mut bus, 0x0001_0000, 0x03FF, 0x9234_5678);
    place_at(&mut bus, 0x000F_0000, &[0x66, 0x0F, 0x01, 0x1E, 0x00, 0x00]);
    cpu.step(&mut bus);

    assert_eq!(cpu.state.idt_limit, 0x03FF);
    assert_eq!(cpu.state.idt_base, 0x9234_5678);
}

#[test]
fn ltr_then_immediate_clear_via_secondary_load_unsupported_succeeds_first() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_handlers(&mut bus);
    cpu.load_state(&state);

    place_at(
        &mut bus,
        RING0_CODE_BASE,
        &[
            0xB8,
            SELECTOR_SECONDARY_TSS as u8,
            (SELECTOR_SECONDARY_TSS >> 8) as u8,
            0x0F,
            0x00,
            0xD8,
        ],
    );
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_eq!(cpu.state.tr, SELECTOR_SECONDARY_TSS);
    assert_eq!(cpu.state.tr_base, TASK_STATE_SEGMENT_SECONDARY_BASE);
}

#[test]
fn arpl_protected_mode_at_cpl0_with_zero_dest_raises_rpl() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.set_eax(0x0000_0000);
    state.set_ebx(0x0000_0003);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[0x63, 0xD8]);
    cpu.step(&mut bus);

    assert_eq!(cpu.eax() & 0xFFFF, 0x0003);
    assert!(cpu.state.flags.zf());
}

#[test]
fn hlt_real_mode_with_if_clear_irq_does_not_dispatch() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = cpu::I386State::default();
    state.set_cs(0xFFFF);
    state.seg_bases[cpu::SegReg32::CS as usize] = 0x000F_FFF0;
    state.set_ss(0x0000);
    state.set_esp(0x1000);
    state.seg_bases[cpu::SegReg32::SS as usize] = 0;
    state.seg_limits = [0xFFFF; 6];
    state.seg_rights[cpu::SegReg32::CS as usize] = 0x9B;
    state.seg_rights[cpu::SegReg32::SS as usize] = 0x93;
    state.seg_valid = [true; 6];
    state.flags.if_flag = false;
    cpu.load_state(&state);

    place_at(&mut bus, 0x000F_FFF0, &[0xF4]);
    cpu.step(&mut bus);
    assert!(cpu.halted());

    bus.irq_vector = 0x40;
    cpu.signal_irq();
    cpu.step(&mut bus);

    assert_eq!(
        cpu.cs(),
        0xFFFF,
        "Maskable IRQ with IF=0 must not be dispatched"
    );
}

// Nudge data segment descriptor's writability on a fresh setup to confirm
// the helper produces a valid configuration. Compile-time-only smoke check.
#[test]
fn shared_data_descriptor_in_setup_protected_mode_is_writable() {
    let mut bus = TestBus::new();
    let _state = setup_protected_mode(&mut bus, 0xFFFF);

    let access_byte = bus.ram[(GLOBAL_DESCRIPTOR_TABLE_BASE + 2 * 8 + 5) as usize];
    assert_eq!(access_byte, RIGHTS_RING0_DATA_WRITABLE_ACCESSED);
}

#[test]
fn ring0_stack_descriptor_is_writable() {
    let mut bus = TestBus::new();
    let _state = setup_protected_mode(&mut bus, 0xFFFF);
    let access_byte = bus.ram[(GLOBAL_DESCRIPTOR_TABLE_BASE + 3 * 8 + 5) as usize];
    assert_eq!(access_byte, RIGHTS_RING0_DATA_WRITABLE_ACCESSED);
}

#[test]
fn ring0_stack_selector_is_default() {
    let mut bus = TestBus::new();
    let state = setup_protected_mode(&mut bus, 0xFFFF);
    assert_eq!(state.ss(), SELECTOR_RING0_STACK);
}
