use common::{Bus as _, Cpu as _};
use cpu::I386;

const RAM_SIZE: usize = 1024 * 1024;
const ADDRESS_MASK: u32 = 0x000F_FFFF;

struct TestBus {
    ram: Vec<u8>,
    irq_pending: bool,
    irq_vector: u8,
    hle_port_bypass: bool,
}

impl TestBus {
    fn new() -> Self {
        Self {
            ram: vec![0u8; RAM_SIZE],
            irq_pending: false,
            irq_vector: 0,
            hle_port_bypass: false,
        }
    }
}

impl common::Bus for TestBus {
    fn read_byte(&mut self, address: u32) -> u8 {
        self.ram[(address & ADDRESS_MASK) as usize]
    }

    fn write_byte(&mut self, address: u32, value: u8) {
        self.ram[(address & ADDRESS_MASK) as usize] = value;
    }

    fn io_read_byte(&mut self, _port: u16) -> u8 {
        0xFF
    }

    fn io_write_byte(&mut self, _port: u16, _value: u8) {}

    fn is_io_port_unrestricted(&self, port: u16) -> bool {
        self.hle_port_bypass && matches!(port, 0x07EE..=0x07F0)
    }

    fn has_irq(&self) -> bool {
        self.irq_pending
    }

    fn acknowledge_irq(&mut self) -> u8 {
        self.irq_pending = false;
        self.irq_vector
    }

    fn has_nmi(&self) -> bool {
        false
    }

    fn acknowledge_nmi(&mut self) {}

    fn current_cycle(&self) -> u64 {
        0
    }

    fn set_current_cycle(&mut self, _cycle: u64) {}
}

fn place_code(bus: &mut TestBus, cs: u16, ip: u16, code: &[u8]) {
    let base = (cs as u32) << 4;
    for (i, &byte) in code.iter().enumerate() {
        bus.write_byte(base + ip as u32 + i as u32, byte);
    }
}

/// Real mode: CR0 should have PE=0 after reset.
#[test]
fn i386_cr0_after_reset() {
    let cpu: I386 = I386::new();

    assert_eq!(cpu.cr0 & 1, 0, "PE bit should be clear after reset");
    assert_eq!(
        cpu.cr0, 0x0000_0010,
        "CR0 reset value: ET=1, reserved bits 5-30 clear"
    );
}

/// Real mode: SMSW reads back the low 16 bits of CR0 (no mode switch).
#[test]
fn i386_smsw_returns_cr0_low() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    // SMSW AX (0x0F 0x01 modrm=0xE0: mod=11, /4, rm=AX)
    place_code(&mut bus, 0xFFFF, 0x0000, &[0x0F, 0x01, 0xE0]);

    cpu.step(&mut bus);

    assert_eq!(
        cpu.eax() & 0xFFFF,
        0x0010,
        "SMSW should store low 16 bits of CR0 (0x0010 = ET) into AX"
    );
}

/// Real mode -> Protected mode: LMSW sets PE bit in CR0.
#[test]
fn i386_lmsw_sets_pe() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    // MOV AX, 0x0001
    // LMSW AX
    place_code(
        &mut bus,
        0xFFFF,
        0x0000,
        &[
            0xB8, 0x01, 0x00, // MOV AX, 1
            0x0F, 0x01, 0xF0, // LMSW AX
        ],
    );

    cpu.step(&mut bus);
    assert_eq!(cpu.cr0 & 1, 0, "PE should be clear before LMSW");

    cpu.step(&mut bus);
    assert_eq!(cpu.cr0 & 1, 1, "PE should be set after LMSW with value 1");
}

/// Protected mode: LMSW cannot clear PE once set (no switch back to real mode).
#[test]
fn i386_lmsw_cannot_clear_pe() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    // MOV AX, 1 -> LMSW AX (set PE) -> MOV AX, 0 -> LMSW AX (try to clear PE)
    place_code(
        &mut bus,
        0xFFFF,
        0x0000,
        &[
            0xB8, 0x01, 0x00, // MOV AX, 1
            0x0F, 0x01, 0xF0, // LMSW AX (set PE)
            0xB8, 0x00, 0x00, // MOV AX, 0
            0x0F, 0x01, 0xF0, // LMSW AX (attempt clear PE)
        ],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    assert_eq!(cpu.cr0 & 1, 1, "PE should be set");

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    assert_eq!(cpu.cr0 & 1, 1, "LMSW must not be able to clear PE once set");
}

/// Real mode -> Protected mode: MOV CR0 sets PE bit.
#[test]
fn i386_mov_cr0_sets_pe() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    // MOV EAX, 1 (66h prefix)
    // MOV CR0, EAX (0x0F 0x22 0xC0)
    place_code(
        &mut bus,
        0xFFFF,
        0x0000,
        &[
            0x66, 0xB8, 0x01, 0x00, 0x00, 0x00, // MOV EAX, 1
            0x0F, 0x22, 0xC0, // MOV CR0, EAX
        ],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_eq!(cpu.cr0, 0x0000_0001, "CR0 should be 1 (PE set)");
}

/// Protected mode -> Unreal mode: MOV CR0 clears PE to return to real mode.
#[test]
fn i386_mov_cr0_can_clear_pe() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    // Set up in protected mode at CPL=0 (as on real hardware, MOV CR0 requires CPL=0).
    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.set_eax(0x0000_0000);
    cpu.load_state(&state);

    assert_eq!(cpu.cr0 & 1, 1, "PE should be set in protected mode");

    // MOV CR0, EAX (EAX=0 clears PE)
    place_at(&mut bus, PM_CODE_BASE, &[0x0F, 0x22, 0xC0]);

    cpu.step(&mut bus);
    assert_eq!(
        cpu.cr0 & 1,
        0,
        "MOV CR0 should be able to clear PE (unreal mode)"
    );
}

/// MOV CR0 masks reserved bits: only PG(31), ET(4), TS(3), EM(2), MP(1), PE(0) are writable.
#[test]
fn i386_mov_cr0_masks_reserved_bits() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    // MOV EAX, 0xFFFFFFFF (66h prefix + B8 imm32)
    // MOV CR0, EAX (0F 22 C0)
    place_code(
        &mut bus,
        0xFFFF,
        0x0000,
        &[
            0x66, 0xB8, 0xFF, 0xFF, 0xFF, 0xFF, // MOV EAX, 0xFFFFFFFF
            0x0F, 0x22, 0xC0, // MOV CR0, EAX
        ],
    );

    cpu.step(&mut bus); // MOV EAX
    cpu.step(&mut bus); // MOV CR0, EAX

    assert_eq!(
        cpu.cr0, 0x8000_001F,
        "Only PG(31), ET(4), TS(3), EM(2), MP(1), PE(0) should be writable on 386"
    );
}

/// Real mode: MOV r32, CR0 reads the current CR0 value (no mode switch).
#[test]
fn i386_mov_r32_cr0_reads_cr0() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    // MOV EAX, CR0 (0x0F 0x20 0xC0: mod=11, reg=0 (CR0), rm=0 (EAX))
    place_code(&mut bus, 0xFFFF, 0x0000, &[0x0F, 0x20, 0xC0]);

    cpu.step(&mut bus);

    assert_eq!(
        cpu.eax(),
        0x0000_0010,
        "MOV EAX, CR0 should read reset value of CR0 (ET=1)"
    );
}

/// i386: opcode 0x63 (ARPL placeholder) triggers #UD (INT 6).
#[test]
fn i386_invalid_opcode_0x63_triggers_ud() {
    use cpu::I386State;
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let ip: u16 = 0x0050;
    place_code(&mut bus, cs, ip, &[0x63, 0x90]);

    let handler_cs: u16 = 0x2000;
    let handler_ip: u16 = 0x0000;
    let ivt_addr = 6 * 4;
    bus.ram[ivt_addr] = handler_ip as u8;
    bus.ram[ivt_addr + 1] = (handler_ip >> 8) as u8;
    bus.ram[ivt_addr + 2] = handler_cs as u8;
    bus.ram[ivt_addr + 3] = (handler_cs >> 8) as u8;

    let mut state = I386State::default();
    state.set_cs(cs);
    state.set_eip(ip as u32);
    state.set_ss(0x3000);
    state.set_esp(0x1000);
    cpu.load_state(&state);

    cpu.step(&mut bus);

    assert_eq!(
        cpu.cs(),
        handler_cs,
        "i386: #UD should jump to INT 6 handler CS"
    );
    assert_eq!(
        cpu.ip() as u16,
        handler_ip,
        "i386: #UD should jump to INT 6 handler IP"
    );

    let esp = cpu.esp();
    let ss_base = (cpu.ss() as u32) << 4;
    let pushed_ip = bus.ram[(ss_base + esp) as usize] as u16
        | (bus.ram[(ss_base + esp + 1) as usize] as u16) << 8;
    assert_eq!(
        pushed_ip, ip,
        "i386: #UD should push faulting IP={ip:#06X} on stack, got {pushed_ip:#06X}"
    );
}

const PM_GDT_BASE: u32 = 0x80000;
const PM_IDT_BASE: u32 = 0x90000;
const PM_CODE_BASE: u32 = 0x50000;
const PM_DATA_BASE: u32 = 0x10000;
const PM_STACK_BASE: u32 = 0x00000;

const PM_CS_SEL: u16 = 0x0008;
const PM_DS_SEL: u16 = 0x0010;
const PM_SS_SEL: u16 = 0x0018;

const PM_GP_HANDLER_IP: u16 = 0x8000;

fn place_at(bus: &mut TestBus, addr: u32, code: &[u8]) {
    for (i, &byte) in code.iter().enumerate() {
        bus.ram[addr as usize + i] = byte;
    }
}

fn write_gdt_entry(
    bus: &mut TestBus,
    gdt_base: u32,
    entry_index: u16,
    base: u32,
    limit: u32,
    rights: u8,
    granularity: u8,
) {
    let addr = (gdt_base + (entry_index as u32) * 8) as usize;
    bus.ram[addr] = limit as u8;
    bus.ram[addr + 1] = (limit >> 8) as u8;
    bus.ram[addr + 2] = base as u8;
    bus.ram[addr + 3] = (base >> 8) as u8;
    bus.ram[addr + 4] = (base >> 16) as u8;
    bus.ram[addr + 5] = rights;
    bus.ram[addr + 6] = granularity | ((limit >> 16) as u8 & 0x0F);
    bus.ram[addr + 7] = (base >> 24) as u8;
}

fn write_gdt_entry16(
    bus: &mut TestBus,
    gdt_base: u32,
    entry_index: u16,
    base: u32,
    limit: u16,
    rights: u8,
) {
    write_gdt_entry(bus, gdt_base, entry_index, base, limit as u32, rights, 0);
}

fn write_idt_gate(
    bus: &mut TestBus,
    idt_base: u32,
    vector: u8,
    offset: u32,
    selector: u16,
    gate_type: u8,
    dpl: u8,
) {
    let addr = (idt_base + (vector as u32) * 8) as usize;
    bus.ram[addr] = offset as u8;
    bus.ram[addr + 1] = (offset >> 8) as u8;
    bus.ram[addr + 2] = selector as u8;
    bus.ram[addr + 3] = (selector >> 8) as u8;
    bus.ram[addr + 4] = 0;
    bus.ram[addr + 5] = 0x80 | (dpl << 5) | gate_type;
    bus.ram[addr + 6] = (offset >> 16) as u8;
    bus.ram[addr + 7] = (offset >> 24) as u8;
}

fn setup_protected_mode(bus: &mut TestBus, ds_limit: u16) -> cpu::I386State {
    write_gdt_entry16(bus, PM_GDT_BASE, 0, 0, 0, 0);
    write_gdt_entry16(bus, PM_GDT_BASE, 1, PM_CODE_BASE, 0xFFFF, 0x9B);
    write_gdt_entry16(bus, PM_GDT_BASE, 2, PM_DATA_BASE, ds_limit, 0x93);
    write_gdt_entry16(bus, PM_GDT_BASE, 3, PM_STACK_BASE, 0xFFFF, 0x93);

    write_idt_gate(
        bus,
        PM_IDT_BASE,
        13,
        PM_GP_HANDLER_IP as u32,
        PM_CS_SEL,
        14,
        0,
    );

    bus.ram[(PM_CODE_BASE + PM_GP_HANDLER_IP as u32) as usize] = 0xF4;

    let mut state = cpu::I386State {
        cr0: 0x0001,
        ip: 0x0000,
        ..Default::default()
    };
    state.set_esp(0xFFF0);

    state.set_cs(PM_CS_SEL);
    state.set_ds(PM_DS_SEL);
    state.set_ss(PM_SS_SEL);
    state.set_es(PM_DS_SEL);

    state.seg_bases[cpu::SegReg32::ES as usize] = PM_DATA_BASE;
    state.seg_bases[cpu::SegReg32::CS as usize] = PM_CODE_BASE;
    state.seg_bases[cpu::SegReg32::SS as usize] = PM_STACK_BASE;
    state.seg_bases[cpu::SegReg32::DS as usize] = PM_DATA_BASE;

    state.seg_limits[cpu::SegReg32::ES as usize] = ds_limit as u32;
    state.seg_limits[cpu::SegReg32::CS as usize] = 0xFFFF;
    state.seg_limits[cpu::SegReg32::SS as usize] = 0xFFFF;
    state.seg_limits[cpu::SegReg32::DS as usize] = ds_limit as u32;

    state.seg_rights[cpu::SegReg32::ES as usize] = 0x93;
    state.seg_rights[cpu::SegReg32::CS as usize] = 0x9B;
    state.seg_rights[cpu::SegReg32::SS as usize] = 0x93;
    state.seg_rights[cpu::SegReg32::DS as usize] = 0x93;

    state.seg_valid = [true, true, true, true, false, false];

    state.gdt_base = PM_GDT_BASE;
    state.gdt_limit = 4 * 8 - 1;

    state.idt_base = PM_IDT_BASE;
    state.idt_limit = 256 * 8 - 1;

    state
}

fn read_word_at(bus: &TestBus, addr: u32) -> u16 {
    bus.ram[addr as usize] as u16 | ((bus.ram[addr as usize + 1] as u16) << 8)
}

fn read_dword_at(bus: &TestBus, addr: u32) -> u32 {
    bus.ram[addr as usize] as u32
        | ((bus.ram[addr as usize + 1] as u32) << 8)
        | ((bus.ram[addr as usize + 2] as u32) << 16)
        | ((bus.ram[addr as usize + 3] as u32) << 24)
}

fn write_word_at(bus: &mut TestBus, addr: u32, value: u16) {
    bus.ram[addr as usize] = value as u8;
    bus.ram[addr as usize + 1] = (value >> 8) as u8;
}

fn write_dword_at(bus: &mut TestBus, addr: u32, value: u32) {
    bus.ram[addr as usize] = value as u8;
    bus.ram[addr as usize + 1] = (value >> 8) as u8;
    bus.ram[addr as usize + 2] = (value >> 16) as u8;
    bus.ram[addr as usize + 3] = (value >> 24) as u8;
}

const TEST_PAGE_DIRECTORY_BASE: u32 = 0xA0000;
const TEST_PAGE_TABLE_BASE: u32 = 0xA1000;
const TEST_PAGE_PRESENT_WRITABLE: u32 = 0x003;
const TEST_PAGE_PRESENT_WRITABLE_USER: u32 = 0x007;

fn enable_identity_paging(bus: &mut TestBus, state: &mut cpu::I386State) {
    write_dword_at(
        bus,
        TEST_PAGE_DIRECTORY_BASE,
        TEST_PAGE_TABLE_BASE | TEST_PAGE_PRESENT_WRITABLE,
    );

    for page_index in 0..1024u32 {
        write_dword_at(
            bus,
            TEST_PAGE_TABLE_BASE + page_index * 4,
            (page_index << 12) | TEST_PAGE_PRESENT_WRITABLE,
        );
    }

    state.cr0 |= 0x8000_0000;
    state.cr3 = TEST_PAGE_DIRECTORY_BASE;
}

fn unmap_identity_page(bus: &mut TestBus, linear_address: u32) {
    let page_index = (linear_address >> 12) & 0x3FF;
    write_dword_at(bus, TEST_PAGE_TABLE_BASE + page_index * 4, page_index << 12);
}

fn set_identity_page_flags(bus: &mut TestBus, linear_address: u32, flags: u32) {
    let page_index = (linear_address >> 12) & 0x3FF;
    write_dword_at(
        bus,
        TEST_PAGE_TABLE_BASE + page_index * 4,
        (page_index << 12) | flags,
    );
}

#[test]
fn i386_lmsw_only_writes_low_4_bits() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    place_code(
        &mut bus,
        0xFFFF,
        0x0000,
        &[
            0xB8, 0xFF, 0xFF, // MOV AX, 0xFFFF
            0x0F, 0x01, 0xF0, // LMSW AX
        ],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_eq!(cpu.cr0 & 0xF, 0xF, "LMSW should set low 4 bits");
    assert_eq!(
        cpu.cr0 & 0xFFF0,
        0x0010,
        "LMSW on 386 should preserve bits 4-15 from original CR0 (ET=1)"
    );
}

#[test]
fn i386_mov_al_moffs_protected_mode_reads_correctly() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_protected_mode(&mut bus, 0xFFFF);
    cpu.load_state(&state);

    bus.ram[(PM_DATA_BASE + 0x05) as usize] = 0xAB;

    place_at(&mut bus, PM_CODE_BASE, &[0xA0, 0x05, 0x00]);

    cpu.step(&mut bus);

    assert_eq!(cpu.al(), 0xAB);
}

#[test]
fn i386_mov_al_moffs_protected_mode_gp_on_limit_violation() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_protected_mode(&mut bus, 0x000F);
    cpu.load_state(&state);

    place_at(&mut bus, PM_CODE_BASE, &[0xA0, 0x20, 0x00]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_GP_HANDLER_IP as u32 + 1);
}

#[test]
fn i386_mov_moffs_al_protected_mode_writes_correctly() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.set_eax(0x0042);
    cpu.load_state(&state);

    place_at(&mut bus, PM_CODE_BASE, &[0xA2, 0x10, 0x00]);

    cpu.step(&mut bus);

    assert_eq!(bus.ram[(PM_DATA_BASE + 0x10) as usize], 0x42);
}

#[test]
fn i386_xlat_protected_mode_reads_correctly() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.set_ebx(0x0100);
    state.set_eax(0x0005);
    cpu.load_state(&state);

    bus.ram[(PM_DATA_BASE + 0x105) as usize] = 0x7E;

    place_at(&mut bus, PM_CODE_BASE, &[0xD7]);

    cpu.step(&mut bus);

    assert_eq!(cpu.al(), 0x7E);
}

#[test]
fn i386_xlat_protected_mode_gp_on_limit_violation() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_protected_mode(&mut bus, 0x000F);
    cpu.load_state(&state);
    cpu.state.set_ebx(0x0020);
    cpu.state.set_eax(0x0005);

    place_at(&mut bus, PM_CODE_BASE, &[0xD7]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
}

#[test]
fn i386_far_jmp_conforming_code_adjusts_cs_rpl_to_cpl() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_protected_mode(&mut bus, 0xFFFF);
    cpu.load_state(&state);

    write_gdt_entry16(&mut bus, PM_GDT_BASE, 4, PM_CODE_BASE, 0xFFFF, 0x9E);
    cpu.state.gdt_limit = 5 * 8 - 1;

    place_at(&mut bus, PM_CODE_BASE, &[0xEA, 0x00, 0x20, 0x21, 0x00]);
    bus.ram[(PM_CODE_BASE + 0x2000) as usize] = 0xF4;

    cpu.step(&mut bus);

    assert_eq!(cpu.cs(), 0x0020);
    assert_eq!(cpu.cs() & 3, 0);
    assert_eq!(cpu.ip(), 0x2000);
}

#[test]
fn i386_protected_mode_int_dispatches_via_intgate() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_protected_mode(&mut bus, 0xFFFF);
    cpu.load_state(&state);

    let handler_ip: u16 = 0x3000;
    write_idt_gate(
        &mut bus,
        PM_IDT_BASE,
        0x20,
        handler_ip as u32,
        PM_CS_SEL,
        14,
        3,
    );
    bus.ram[(PM_CODE_BASE + handler_ip as u32) as usize] = 0xF4;

    cpu.state.flags.if_flag = true;

    place_at(&mut bus, PM_CODE_BASE, &[0xCD, 0x20]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), handler_ip as u32 + 1);
    assert!(!cpu.state.flags.if_flag, "Interrupt gate should clear IF");

    // 386 gate pushes dwords
    let sp = cpu.esp();
    let pushed_eip = read_dword_at(&bus, PM_STACK_BASE + sp);
    let pushed_cs = read_dword_at(&bus, PM_STACK_BASE + sp + 4);
    let pushed_eflags = read_dword_at(&bus, PM_STACK_BASE + sp + 8);

    assert_eq!(pushed_eip, 0x0002);
    assert_eq!(pushed_cs as u16, PM_CS_SEL);
    assert!(
        pushed_eflags & 0x0200 != 0,
        "Pushed flags should have IF set"
    );
}

#[test]
fn i386_protected_mode_int_trapgate_preserves_if() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_protected_mode(&mut bus, 0xFFFF);
    cpu.load_state(&state);

    let handler_ip: u16 = 0x4000;
    write_idt_gate(
        &mut bus,
        PM_IDT_BASE,
        0x21,
        handler_ip as u32,
        PM_CS_SEL,
        15,
        3,
    );
    bus.ram[(PM_CODE_BASE + handler_ip as u32) as usize] = 0xF4;

    cpu.state.flags.if_flag = true;

    place_at(&mut bus, PM_CODE_BASE, &[0xCD, 0x21]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert!(cpu.state.flags.if_flag, "Trap gate should preserve IF");
}

#[test]
fn i386_protected_mode_int_sets_cs_rpl_correctly() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_protected_mode(&mut bus, 0xFFFF);
    cpu.load_state(&state);

    let handler_ip: u16 = 0x5000;
    write_idt_gate(
        &mut bus,
        PM_IDT_BASE,
        0x30,
        handler_ip as u32,
        PM_CS_SEL,
        14,
        3,
    );
    bus.ram[(PM_CODE_BASE + handler_ip as u32) as usize] = 0xF4;

    place_at(&mut bus, PM_CODE_BASE, &[0xCD, 0x30]);

    cpu.step(&mut bus);

    assert_eq!(cpu.cs() & !3, PM_CS_SEL & !3);
    assert_eq!(cpu.cs() & 3, 0);
    assert_eq!(cpu.ip(), handler_ip as u32);
}

#[test]
fn i386_popf_pushf_iopl_nt_roundtrip() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_protected_mode(&mut bus, 0xFFFF);
    cpu.load_state(&state);

    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[
            0x68, 0x02, 0x70, // PUSH 0x7002
            0x9D, // POPF
            0x9C, // PUSHF
            0x58, // POP AX
        ],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    let flags = cpu.eax() as u16;
    assert_eq!((flags >> 12) & 3, 3, "IOPL should be 3");
    assert_ne!(flags & 0x4000, 0, "NT should be set");
}

/// 32-bit PUSH of a 16-bit segment register allocates a 4-byte stack slot
/// but writes only the low 2 bytes; the upper 2 bytes of the slot are left
/// untouched. Documented i386/i486 behavior (Pentium and later zero-extend).
#[test]
fn i386_push_sreg_o32_leaves_upper_stack_slot_untouched() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_protected_mode(&mut bus, 0xFFFF);
    cpu.load_state(&state);

    let sentinel_hi: u16 = 0xDEAD;
    let sentinel_lo: u16 = 0xBEEF;
    let initial_sp = cpu.esp();
    let slot_addr = PM_STACK_BASE + (initial_sp - 4);
    write_dword_at(
        &mut bus,
        slot_addr,
        ((sentinel_hi as u32) << 16) | sentinel_lo as u32,
    );

    place_at(&mut bus, PM_CODE_BASE, &[0x66, 0x1E]);

    cpu.step(&mut bus);

    assert_eq!(
        cpu.esp(),
        initial_sp - 4,
        "o32 PUSH DS must allocate a 4-byte stack slot",
    );
    assert_eq!(
        read_word_at(&bus, slot_addr),
        PM_DS_SEL,
        "low 2 bytes of slot must hold the DS selector",
    );
    assert_eq!(
        read_word_at(&bus, slot_addr + 2),
        sentinel_hi,
        "upper 2 bytes of slot must be left untouched (i386/i486 semantics)",
    );
}

#[test]
fn i386_gp_escalates_to_double_fault() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.gdt_limit = 8 * 8 - 1;
    cpu.load_state(&state);

    let df_handler_ip: u16 = 0x6000;
    write_idt_gate(&mut bus, PM_IDT_BASE, 13, 0x0000, 0x0028, 14, 0);
    write_idt_gate(
        &mut bus,
        PM_IDT_BASE,
        8,
        df_handler_ip as u32,
        PM_CS_SEL,
        14,
        0,
    );
    bus.ram[(PM_CODE_BASE + df_handler_ip as u32) as usize] = 0xF4;

    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[
            0xB8, 0x28, 0x00, // MOV AX, 0x0028
            0x8E, 0xD8, // MOV DS, AX
        ],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
}

#[test]
fn i386_lgdt_at_cpl3_triggers_gp() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    let ring3_cs_sel: u16 = 0x0020;
    write_gdt_entry16(&mut bus, PM_GDT_BASE, 4, PM_CODE_BASE, 0xFFFF, 0xFB);
    state.gdt_limit = 6 * 8 - 1;
    state.set_cs(ring3_cs_sel | 3);
    state.seg_bases[cpu::SegReg32::CS as usize] = PM_CODE_BASE;
    state.seg_limits[cpu::SegReg32::CS as usize] = 0xFFFF;
    state.seg_rights[cpu::SegReg32::CS as usize] = 0xFB;

    let ring3_ss_sel: u16 = 0x0028;
    write_gdt_entry16(&mut bus, PM_GDT_BASE, 5, PM_STACK_BASE, 0xFFFF, 0xF3);
    state.set_ss(ring3_ss_sel | 3);
    state.seg_bases[cpu::SegReg32::SS as usize] = PM_STACK_BASE;
    state.seg_limits[cpu::SegReg32::SS as usize] = 0xFFFF;
    state.seg_rights[cpu::SegReg32::SS as usize] = 0xF3;

    cpu.load_state(&state);

    place_at(&mut bus, PM_CODE_BASE, &[0x0F, 0x01, 0x16, 0x00, 0x10]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
}

#[test]
fn i386_ltr_marks_386_tss_busy() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.gdt_limit = 6 * 8 - 1;
    cpu.load_state(&state);

    let tss_base: u32 = 0x60000;
    write_gdt_entry16(&mut bus, PM_GDT_BASE, 4, tss_base, 103, 0x89);

    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[0xB8, 0x20, 0x00, 0x0F, 0x00, 0xD8],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    let desc_addr = PM_GDT_BASE + 4 * 8;
    let rights = bus.ram[(desc_addr + 5) as usize];
    let desc_type = rights & 0x0F;
    assert_eq!(desc_type, 0x0B, "LTR should mark 386 TSS as busy (type 11)");
}

#[test]
fn i386_ltr_busy_tss_triggers_gp() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.gdt_limit = 6 * 8 - 1;
    cpu.load_state(&state);

    write_gdt_entry16(&mut bus, PM_GDT_BASE, 4, 0x60000, 103, 0x8B);

    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[0xB8, 0x20, 0x00, 0x0F, 0x00, 0xD8],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
}

#[test]
fn i386_lar_accepts_386_tss_descriptor() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.gdt_limit = 6 * 8 - 1;
    cpu.load_state(&state);

    write_gdt_entry16(&mut bus, PM_GDT_BASE, 4, 0x60000, 103, 0x89);

    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[0xBB, 0x20, 0x00, 0x0F, 0x02, 0xC3],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf(), "LAR on 386 TSS should set ZF=1");
    assert_eq!(cpu.eax() as u16, 0x8900);
}

#[test]
fn i386_lsl_rejects_call_gate() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.gdt_limit = 6 * 8 - 1;
    cpu.load_state(&state);

    write_gdt_entry16(&mut bus, PM_GDT_BASE, 4, 0x60000, 0x002B, 0x84);

    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[0xBB, 0x20, 0x00, 0x0F, 0x03, 0xC3],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(!cpu.state.flags.zf(), "LSL on call gate should set ZF=0");
}

#[test]
fn i386_interrupt_conforming_code_dispatches() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.gdt_limit = 6 * 8 - 1;
    cpu.load_state(&state);

    write_gdt_entry16(&mut bus, PM_GDT_BASE, 4, PM_CODE_BASE, 0xFFFF, 0x9E);

    let handler_ip: u16 = 0x7000;
    write_idt_gate(
        &mut bus,
        PM_IDT_BASE,
        0x40,
        handler_ip as u32,
        0x0020,
        14,
        3,
    );
    bus.ram[(PM_CODE_BASE + handler_ip as u32) as usize] = 0xF4;

    place_at(&mut bus, PM_CODE_BASE, &[0xCD, 0x40]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), handler_ip as u32 + 1);
}

#[test]
fn i386_load_segment_sets_accessed_bit() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.gdt_limit = 6 * 8 - 1;
    cpu.load_state(&state);

    write_gdt_entry16(&mut bus, PM_GDT_BASE, 4, PM_DATA_BASE, 0xFFFF, 0x92);

    let desc_addr = PM_GDT_BASE + 4 * 8;
    assert_eq!(bus.ram[(desc_addr + 5) as usize] & 0x01, 0);

    place_at(&mut bus, PM_CODE_BASE, &[0xB8, 0x20, 0x00, 0x8E, 0xD8]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_ne!(bus.ram[(desc_addr + 5) as usize] & 0x01, 0);
}

#[test]
fn i386_verr_conforming_code_dpl_exemption() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.gdt_limit = 7 * 8 - 1;

    let ring3_cs_sel: u16 = 0x0028;
    write_gdt_entry16(&mut bus, PM_GDT_BASE, 5, PM_CODE_BASE, 0xFFFF, 0xFB);
    state.set_cs(ring3_cs_sel | 3);
    state.seg_bases[cpu::SegReg32::CS as usize] = PM_CODE_BASE;
    state.seg_limits[cpu::SegReg32::CS as usize] = 0xFFFF;
    state.seg_rights[cpu::SegReg32::CS as usize] = 0xFB;

    let ring3_ss_sel: u16 = 0x0030;
    write_gdt_entry16(&mut bus, PM_GDT_BASE, 6, PM_STACK_BASE, 0xFFFF, 0xF3);
    state.set_ss(ring3_ss_sel | 3);
    state.seg_bases[cpu::SegReg32::SS as usize] = PM_STACK_BASE;
    state.seg_limits[cpu::SegReg32::SS as usize] = 0xFFFF;
    state.seg_rights[cpu::SegReg32::SS as usize] = 0xF3;

    cpu.load_state(&state);

    write_gdt_entry16(&mut bus, PM_GDT_BASE, 4, PM_CODE_BASE, 0xFFFF, 0x9E);

    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[0xB8, 0x20, 0x00, 0x0F, 0x00, 0xE0],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf());
}

#[test]
fn i386_gate_offset_exceeds_limit_gp_zero() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.gdt_limit = 6 * 8 - 1;
    cpu.load_state(&state);

    write_gdt_entry16(&mut bus, PM_GDT_BASE, 4, PM_CODE_BASE, 0x0010, 0x9B);

    write_idt_gate(&mut bus, PM_IDT_BASE, 0x50, 0x0100, 0x0020, 14, 3);

    let gp_handler_ip: u16 = 0x9000;
    write_idt_gate(
        &mut bus,
        PM_IDT_BASE,
        13,
        gp_handler_ip as u32,
        PM_CS_SEL,
        14,
        0,
    );
    bus.ram[(PM_CODE_BASE + gp_handler_ip as u32) as usize] = 0xF4;

    place_at(&mut bus, PM_CODE_BASE, &[0xCD, 0x50]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());

    let sp = cpu.esp();
    let error_code = read_dword_at(&bus, PM_STACK_BASE + sp);
    assert_eq!(error_code, 0);
}

#[test]
fn i386_lldt_selector_0x0004_is_nonnull() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.gdt_limit = 6 * 8 - 1;
    cpu.load_state(&state);

    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[0xB8, 0x04, 0x00, 0x0F, 0x00, 0xD0],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
}

#[test]
fn i386_wrong_type_not_present_gives_gp_not_np() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.gdt_limit = 6 * 8 - 1;
    cpu.load_state(&state);

    write_gdt_entry16(&mut bus, PM_GDT_BASE, 4, PM_DATA_BASE, 0xFFFF, 0x18);

    let np_handler_ip: u16 = 0xA000;
    write_idt_gate(
        &mut bus,
        PM_IDT_BASE,
        11,
        np_handler_ip as u32,
        PM_CS_SEL,
        14,
        0,
    );
    bus.ram[(PM_CODE_BASE + np_handler_ip as u32) as usize] = 0x90;
    bus.ram[(PM_CODE_BASE + np_handler_ip as u32 + 1) as usize] = 0xF4;

    place_at(&mut bus, PM_CODE_BASE, &[0xB8, 0x20, 0x00, 0x8E, 0xD8]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_GP_HANDLER_IP as u32 + 1);
}

#[test]
fn i386_indirect_call_far_invalid_segment_sp_unchanged() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_protected_mode(&mut bus, 0xFFFF);
    cpu.load_state(&state);

    let sp_before = cpu.esp();

    let ptr_addr = PM_DATA_BASE + 0x100;
    bus.ram[ptr_addr as usize] = 0x34;
    bus.ram[ptr_addr as usize + 1] = 0x12;
    bus.ram[ptr_addr as usize + 2] = 0x28;
    bus.ram[ptr_addr as usize + 3] = 0x00;

    place_at(&mut bus, PM_CODE_BASE, &[0xFF, 0x1E, 0x00, 0x01]);

    cpu.step(&mut bus);

    let sp_after_fault = cpu.esp();
    // 386 gate pushes dwords: EFLAGS(4) + CS(4) + EIP(4) + error_code(4) = 16 bytes
    assert_eq!(sp_before - sp_after_fault, 16);
}

const PM_RING3_CODE_BASE: u32 = 0x60000;
const PM_RING3_STACK_BASE: u32 = 0x20000;
const PM_TSS_BASE: u32 = 0x70000;

const PM_RING3_CS_SEL: u16 = 0x0023;
const PM_RING3_DS_SEL: u16 = 0x002B;
const PM_RING3_SS_SEL: u16 = 0x0033;
const PM_TSS_SEL: u16 = 0x0038;

const PM_TS_HANDLER_IP: u16 = 0x9000;
const PM_SS_HANDLER_IP: u16 = 0xB000;

fn setup_protected_mode_with_ring3(bus: &mut TestBus) -> cpu::I386State {
    write_gdt_entry16(bus, PM_GDT_BASE, 0, 0, 0, 0);
    write_gdt_entry16(bus, PM_GDT_BASE, 1, PM_CODE_BASE, 0xFFFF, 0x9B);
    write_gdt_entry16(bus, PM_GDT_BASE, 2, PM_DATA_BASE, 0xFFFF, 0x93);
    write_gdt_entry16(bus, PM_GDT_BASE, 3, PM_STACK_BASE, 0xFFFF, 0x93);
    write_gdt_entry16(bus, PM_GDT_BASE, 4, PM_RING3_CODE_BASE, 0xFFFF, 0xFB);
    write_gdt_entry16(bus, PM_GDT_BASE, 5, PM_DATA_BASE, 0xFFFF, 0xF3);
    write_gdt_entry16(bus, PM_GDT_BASE, 6, PM_RING3_STACK_BASE, 0xFFFF, 0xF3);
    write_gdt_entry16(bus, PM_GDT_BASE, 7, PM_TSS_BASE, 103, 0x89);

    write_idt_gate(
        bus,
        PM_IDT_BASE,
        10,
        PM_TS_HANDLER_IP as u32,
        PM_CS_SEL,
        14,
        0,
    );
    write_idt_gate(
        bus,
        PM_IDT_BASE,
        12,
        PM_SS_HANDLER_IP as u32,
        PM_CS_SEL,
        14,
        0,
    );
    write_idt_gate(
        bus,
        PM_IDT_BASE,
        13,
        PM_GP_HANDLER_IP as u32,
        PM_CS_SEL,
        14,
        0,
    );

    bus.ram[(PM_CODE_BASE + PM_TS_HANDLER_IP as u32) as usize] = 0xF4;
    bus.ram[(PM_CODE_BASE + PM_SS_HANDLER_IP as u32) as usize] = 0xF4;
    bus.ram[(PM_CODE_BASE + PM_GP_HANDLER_IP as u32) as usize] = 0xF4;

    // 386 TSS: ESP0 at offset 4, SS0 at offset 8
    write_dword_at(bus, PM_TSS_BASE + 4, 0xFFF0);
    write_word_at(bus, PM_TSS_BASE + 8, PM_SS_SEL);

    let mut state = cpu::I386State {
        cr0: 0x0001,
        ip: 0x0000,
        ..Default::default()
    };

    state.set_esp(0xFFF0);
    state.set_cs(PM_CS_SEL);
    state.set_ds(PM_DS_SEL);
    state.set_ss(PM_SS_SEL);
    state.set_es(PM_DS_SEL);

    state.seg_bases[cpu::SegReg32::ES as usize] = PM_DATA_BASE;
    state.seg_bases[cpu::SegReg32::CS as usize] = PM_CODE_BASE;
    state.seg_bases[cpu::SegReg32::SS as usize] = PM_STACK_BASE;
    state.seg_bases[cpu::SegReg32::DS as usize] = PM_DATA_BASE;

    state.seg_limits = [0xFFFF; 6];
    state.seg_rights[cpu::SegReg32::ES as usize] = 0x93;
    state.seg_rights[cpu::SegReg32::CS as usize] = 0x9B;
    state.seg_rights[cpu::SegReg32::SS as usize] = 0x93;
    state.seg_rights[cpu::SegReg32::DS as usize] = 0x93;
    state.seg_valid = [true, true, true, true, false, false];

    state.gdt_base = PM_GDT_BASE;
    state.gdt_limit = 8 * 8 - 1;
    state.idt_base = PM_IDT_BASE;
    state.idt_limit = 256 * 8 - 1;

    state.tr = PM_TSS_SEL;
    state.tr_base = PM_TSS_BASE;
    state.tr_limit = 103;
    state.tr_rights = 0x8B;

    state
}

fn make_ring3_state(state: &mut cpu::I386State) {
    state.set_cs(PM_RING3_CS_SEL);
    state.seg_bases[cpu::SegReg32::CS as usize] = PM_RING3_CODE_BASE;
    state.seg_rights[cpu::SegReg32::CS as usize] = 0xFB;

    state.set_ss(PM_RING3_SS_SEL);
    state.seg_bases[cpu::SegReg32::SS as usize] = PM_RING3_STACK_BASE;
    state.seg_rights[cpu::SegReg32::SS as usize] = 0xF3;

    state.set_ds(PM_RING3_DS_SEL);
    state.seg_bases[cpu::SegReg32::DS as usize] = PM_DATA_BASE;
    state.seg_rights[cpu::SegReg32::DS as usize] = 0xF3;

    state.set_es(PM_RING3_DS_SEL);
    state.seg_bases[cpu::SegReg32::ES as usize] = PM_DATA_BASE;
    state.seg_rights[cpu::SegReg32::ES as usize] = 0xF3;
}

#[test]
fn i386_ring3_segment_load_reads_supervisor_descriptor_table_pages() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let mut state = setup_protected_mode_with_ring3(&mut bus);
    make_ring3_state(&mut state);
    enable_identity_paging(&mut bus, &mut state);

    write_dword_at(
        &mut bus,
        TEST_PAGE_DIRECTORY_BASE,
        TEST_PAGE_TABLE_BASE | TEST_PAGE_PRESENT_WRITABLE_USER,
    );
    set_identity_page_flags(
        &mut bus,
        PM_RING3_CODE_BASE,
        TEST_PAGE_PRESENT_WRITABLE_USER,
    );
    set_identity_page_flags(
        &mut bus,
        PM_RING3_STACK_BASE,
        TEST_PAGE_PRESENT_WRITABLE_USER,
    );
    set_identity_page_flags(&mut bus, PM_DATA_BASE, TEST_PAGE_PRESENT_WRITABLE_USER);
    set_identity_page_flags(&mut bus, PM_GDT_BASE, TEST_PAGE_PRESENT_WRITABLE);

    cpu.load_state(&state);
    place_at(
        &mut bus,
        PM_RING3_CODE_BASE,
        &[0xB8, PM_RING3_DS_SEL as u8, 0x00, 0x8E, 0xD8, 0x90],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_eq!(cpu.ds(), PM_RING3_DS_SEL);
    assert_eq!(cpu.ip(), 5);
}

#[test]
fn i386_ring3_call_gate_reads_supervisor_descriptor_table_pages() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let mut state = setup_protected_mode_with_ring3(&mut bus);
    make_ring3_state(&mut state);

    let gate_target_ip: u16 = 0x0100;
    let target_selector: u16 = 0x1000;

    write_gdt_gate(
        &mut bus,
        PM_GDT_BASE,
        8,
        gate_target_ip as u32,
        target_selector,
        0,
        12,
        3,
    );

    write_gdt_entry16(
        &mut bus,
        PM_GDT_BASE,
        0x200,
        PM_RING3_CODE_BASE,
        0xFFFF,
        0xFB,
    );
    state.gdt_limit = target_selector + 7;

    bus.ram[(PM_RING3_CODE_BASE + gate_target_ip as u32) as usize] = 0xF4;

    enable_identity_paging(&mut bus, &mut state);

    write_dword_at(
        &mut bus,
        TEST_PAGE_DIRECTORY_BASE,
        TEST_PAGE_TABLE_BASE | TEST_PAGE_PRESENT_WRITABLE_USER,
    );
    set_identity_page_flags(
        &mut bus,
        PM_RING3_CODE_BASE,
        TEST_PAGE_PRESENT_WRITABLE_USER,
    );
    set_identity_page_flags(
        &mut bus,
        PM_RING3_STACK_BASE,
        TEST_PAGE_PRESENT_WRITABLE_USER,
    );
    set_identity_page_flags(&mut bus, PM_DATA_BASE, TEST_PAGE_PRESENT_WRITABLE_USER);
    set_identity_page_flags(&mut bus, PM_GDT_BASE, TEST_PAGE_PRESENT_WRITABLE_USER);
    set_identity_page_flags(&mut bus, PM_GDT_BASE + 0x1000, TEST_PAGE_PRESENT_WRITABLE);
    // The default ring-3 ESP is 0xFFF0; with SS base 0x20000 the CALL's
    // return-frame push lands on the segment's last page (0x2F000). The
    // gate's atomicity pre-checks resolve that page before any commit, so
    // it must be mapped user-accessible.
    set_identity_page_flags(
        &mut bus,
        PM_RING3_STACK_BASE + 0xF000,
        TEST_PAGE_PRESENT_WRITABLE_USER,
    );

    cpu.load_state(&state);
    place_at(
        &mut bus,
        PM_RING3_CODE_BASE,
        &[0x9A, 0x00, 0x00, 0x43, 0x00],
    );

    cpu.step(&mut bus);

    assert_eq!(cpu.ip(), gate_target_ip as u32);
    assert_eq!(cpu.cs() & !3, target_selector);
    assert_eq!(cpu.cs() & 3, 3);
}

#[test]
fn i386_ring3_task_switch_reads_supervisor_tss_descriptor() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let mut state = setup_protected_mode_with_ring3(&mut bus);
    make_ring3_state(&mut state);

    let task2_sel: u16 = 0x0048;
    write_gdt_entry16(&mut bus, PM_GDT_BASE, 9, PM_TSS2_BASE, 103, 0xE9);
    state.gdt_limit = 10 * 8 - 1;

    let task2_ip: u32 = 0x0500;
    write_tss386(
        &mut bus,
        PM_TSS2_BASE,
        0,
        0xFFF0,
        PM_SS_SEL,
        task2_ip,
        0x0002,
        0,
        0,
        0,
        0,
        0xFFF0,
        0,
        0,
        0,
        PM_DS_SEL,
        PM_CS_SEL,
        PM_SS_SEL,
        PM_DS_SEL,
        0,
        0,
        0,
    );

    bus.ram[(PM_CODE_BASE + task2_ip) as usize] = 0xF4;

    enable_identity_paging(&mut bus, &mut state);

    // The new task must run with the same identity-mapped page directory so
    // GDT and TSS reads resolve correctly after CR3 is reloaded from the TSS.
    write_dword_at(&mut bus, PM_TSS2_BASE + 28, TEST_PAGE_DIRECTORY_BASE);

    write_dword_at(
        &mut bus,
        TEST_PAGE_DIRECTORY_BASE,
        TEST_PAGE_TABLE_BASE | TEST_PAGE_PRESENT_WRITABLE_USER,
    );
    set_identity_page_flags(
        &mut bus,
        PM_RING3_CODE_BASE,
        TEST_PAGE_PRESENT_WRITABLE_USER,
    );
    set_identity_page_flags(
        &mut bus,
        PM_RING3_STACK_BASE,
        TEST_PAGE_PRESENT_WRITABLE_USER,
    );
    set_identity_page_flags(&mut bus, PM_DATA_BASE, TEST_PAGE_PRESENT_WRITABLE_USER);
    set_identity_page_flags(&mut bus, PM_TSS_BASE, TEST_PAGE_PRESENT_WRITABLE_USER);
    set_identity_page_flags(&mut bus, PM_TSS2_BASE, TEST_PAGE_PRESENT_WRITABLE_USER);
    set_identity_page_flags(&mut bus, PM_CODE_BASE, TEST_PAGE_PRESENT_WRITABLE_USER);
    set_identity_page_flags(&mut bus, PM_STACK_BASE, TEST_PAGE_PRESENT_WRITABLE_USER);
    set_identity_page_flags(&mut bus, PM_GDT_BASE, TEST_PAGE_PRESENT_WRITABLE);

    cpu.load_state(&state);
    place_at(
        &mut bus,
        PM_RING3_CODE_BASE,
        &[0xEA, 0x00, 0x00, task2_sel as u8, (task2_sel >> 8) as u8],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.cs(), PM_CS_SEL);
    assert_eq!(cpu.ip(), task2_ip + 1);
    assert_eq!(cpu.cs() & 3, 0);
}

#[test]
fn i386_ring3_task_switch_accesses_supervisor_tss_body() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let mut state = setup_protected_mode_with_ring3(&mut bus);
    make_ring3_state(&mut state);

    let task2_sel: u16 = 0x0048;
    write_gdt_entry16(&mut bus, PM_GDT_BASE, 9, PM_TSS2_BASE, 103, 0xE9);
    state.gdt_limit = 10 * 8 - 1;

    let task2_ip: u32 = 0x0500;
    write_tss386(
        &mut bus,
        PM_TSS2_BASE,
        0,
        0xFFF0,
        PM_SS_SEL,
        task2_ip,
        0x0002,
        0,
        0,
        0,
        0,
        0xFFF0,
        0,
        0,
        0,
        PM_DS_SEL,
        PM_CS_SEL,
        PM_SS_SEL,
        PM_DS_SEL,
        0,
        0,
        0,
    );

    bus.ram[(PM_CODE_BASE + task2_ip) as usize] = 0xF4;

    enable_identity_paging(&mut bus, &mut state);

    write_dword_at(&mut bus, PM_TSS2_BASE + 28, TEST_PAGE_DIRECTORY_BASE);

    write_dword_at(
        &mut bus,
        TEST_PAGE_DIRECTORY_BASE,
        TEST_PAGE_TABLE_BASE | TEST_PAGE_PRESENT_WRITABLE_USER,
    );
    set_identity_page_flags(
        &mut bus,
        PM_RING3_CODE_BASE,
        TEST_PAGE_PRESENT_WRITABLE_USER,
    );
    set_identity_page_flags(
        &mut bus,
        PM_RING3_STACK_BASE,
        TEST_PAGE_PRESENT_WRITABLE_USER,
    );
    set_identity_page_flags(&mut bus, PM_DATA_BASE, TEST_PAGE_PRESENT_WRITABLE_USER);
    set_identity_page_flags(&mut bus, PM_CODE_BASE, TEST_PAGE_PRESENT_WRITABLE_USER);
    set_identity_page_flags(&mut bus, PM_STACK_BASE, TEST_PAGE_PRESENT_WRITABLE_USER);
    set_identity_page_flags(&mut bus, PM_GDT_BASE, TEST_PAGE_PRESENT_WRITABLE);
    set_identity_page_flags(&mut bus, PM_TSS_BASE, TEST_PAGE_PRESENT_WRITABLE);
    set_identity_page_flags(&mut bus, PM_TSS2_BASE, TEST_PAGE_PRESENT_WRITABLE);

    cpu.load_state(&state);
    place_at(
        &mut bus,
        PM_RING3_CODE_BASE,
        &[0xEA, 0x00, 0x00, task2_sel as u8, (task2_sel >> 8) as u8],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.cs(), PM_CS_SEL);
    assert_eq!(cpu.ip(), task2_ip + 1);
    assert_eq!(cpu.cs() & 3, 0);
}

#[test]
fn i386_ret_far_inter_privilege_ring0_to_ring3() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_ring3(&mut bus);
    cpu.load_state(&state);

    place_at(&mut bus, PM_CODE_BASE, &[0xCB]);

    let ring3_ip: u16 = 0x0100;

    let sp = cpu.esp();
    write_word_at(&mut bus, PM_STACK_BASE + sp, ring3_ip);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 2, PM_RING3_CS_SEL);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 4, 0xFF00);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 6, PM_RING3_SS_SEL);

    cpu.step(&mut bus);

    assert_eq!(cpu.ip() as u16, ring3_ip);
    assert_eq!(cpu.cs() & 3, 3);
    assert_eq!(cpu.ss(), PM_RING3_SS_SEL);
    assert_eq!(cpu.esp() & 0xFFFF, 0xFF00);
}

#[test]
fn i386_ret_far_same_privilege() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_ring3(&mut bus);
    cpu.load_state(&state);

    place_at(&mut bus, PM_CODE_BASE, &[0xCB]);

    let target_ip: u16 = 0x0200;
    bus.ram[(PM_CODE_BASE + target_ip as u32) as usize] = 0xF4;

    let sp = cpu.esp();
    write_word_at(&mut bus, PM_STACK_BASE + sp, target_ip);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 2, PM_CS_SEL);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.cs() & 3, 0);
    assert_eq!(cpu.esp(), 0xFFF4);
}

#[test]
fn i386_iret_inter_privilege_ring0_to_ring3() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_ring3(&mut bus);
    cpu.load_state(&state);

    place_at(&mut bus, PM_CODE_BASE, &[0xCF]);

    let ring3_ip: u16 = 0x0100;

    let sp = cpu.esp();
    write_word_at(&mut bus, PM_STACK_BASE + sp, ring3_ip);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 2, PM_RING3_CS_SEL);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 4, 0x0202);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 6, 0xFF00);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 8, PM_RING3_SS_SEL);

    cpu.step(&mut bus);

    assert_eq!(cpu.ip() as u16, ring3_ip);
    assert_eq!(cpu.cs() & 3, 3);
    assert_eq!(cpu.ss(), PM_RING3_SS_SEL);
    assert_eq!(cpu.esp() & 0xFFFF, 0xFF00);
}

#[test]
fn i386_ret_far_rpl_less_than_cpl_faults() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let mut state = setup_protected_mode_with_ring3(&mut bus);
    make_ring3_state(&mut state);
    state.set_esp(0xFFF0);
    cpu.load_state(&state);

    place_at(&mut bus, PM_RING3_CODE_BASE, &[0xCB]);

    write_word_at(&mut bus, PM_RING3_STACK_BASE + 0xFFF0, 0x0000);
    write_word_at(&mut bus, PM_RING3_STACK_BASE + 0xFFF2, PM_CS_SEL);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_GP_HANDLER_IP as u32 + 1);
}

#[test]
fn i386_ret_far_nonconforming_dpl_ne_rpl_faults() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_ring3(&mut bus);
    cpu.load_state(&state);

    write_gdt_entry16(&mut bus, PM_GDT_BASE, 8, PM_RING3_CODE_BASE, 0xFFFF, 0x9B);
    cpu.state.gdt_limit = 9 * 8 - 1;

    place_at(&mut bus, PM_CODE_BASE, &[0xCB]);

    let sp = cpu.esp();
    write_word_at(&mut bus, PM_STACK_BASE + sp, 0x0100);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 2, 0x0043);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 4, 0xFF00);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 6, PM_RING3_SS_SEL);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_GP_HANDLER_IP as u32 + 1);
}

#[test]
fn i386_interrupt_inter_privilege_ring3_to_ring0() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let mut state = setup_protected_mode_with_ring3(&mut bus);
    make_ring3_state(&mut state);
    state.set_esp(0xFFF0);
    state.flags.expand(0x0202);
    cpu.load_state(&state);

    place_at(&mut bus, PM_RING3_CODE_BASE, &[0x90]);

    write_idt_gate(&mut bus, PM_IDT_BASE, 0x20, 0x0300, PM_CS_SEL, 14, 0);
    bus.ram[(PM_CODE_BASE + 0x0300) as usize] = 0xF4;

    bus.irq_vector = 0x20;
    cpu.signal_irq();

    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.cs() & 3, 0);
    assert_eq!(cpu.ss(), PM_SS_SEL);
}

#[test]
fn i386_interrupt_inter_privilege_ss_fault_uses_ts_vector() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let mut state = setup_protected_mode_with_ring3(&mut bus);
    make_ring3_state(&mut state);
    state.set_esp(0xFFF0);
    state.flags.expand(0x0202);
    cpu.load_state(&state);

    place_at(&mut bus, PM_RING3_CODE_BASE, &[0x90]);

    write_idt_gate(&mut bus, PM_IDT_BASE, 0x20, 0x0300, PM_CS_SEL, 14, 0);

    let ts_handler_ip: u16 = 0x0500;
    write_idt_gate(
        &mut bus,
        PM_IDT_BASE,
        10,
        ts_handler_ip as u32,
        0x0020,
        14,
        0,
    );
    bus.ram[(PM_RING3_CODE_BASE + ts_handler_ip as u32) as usize] = 0xF4;

    write_word_at(&mut bus, PM_TSS_BASE + 8, 0x0000);

    bus.irq_vector = 0x20;
    cpu.signal_irq();

    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), ts_handler_ip as u32 + 1);
}

#[test]
fn i386_ret_far_imm_inter_privilege_reads_correct_offset() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_ring3(&mut bus);
    cpu.load_state(&state);

    place_at(&mut bus, PM_CODE_BASE, &[0xCA, 0x04, 0x00]);

    let ring3_ip: u16 = 0x0100;

    let sp = cpu.esp();
    write_word_at(&mut bus, PM_STACK_BASE + sp, ring3_ip);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 2, PM_RING3_CS_SEL);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 8, 0xFE00);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 10, PM_RING3_SS_SEL);

    cpu.step(&mut bus);

    assert_eq!(cpu.ip() as u16, ring3_ip);
    assert_eq!(cpu.cs() & 3, 3);
    assert_eq!(cpu.ss(), PM_RING3_SS_SEL);
    assert_eq!(cpu.esp() & 0xFFFF, 0xFE04);
}

#[test]
fn i386_ret_far_ip_exceeds_limit_faults() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_ring3(&mut bus);
    cpu.load_state(&state);

    write_gdt_entry16(&mut bus, PM_GDT_BASE, 8, PM_CODE_BASE, 0x0100, 0x9B);
    cpu.state.gdt_limit = 9 * 8 - 1;

    place_at(&mut bus, PM_CODE_BASE, &[0xCB]);

    let sp = cpu.esp();
    write_word_at(&mut bus, PM_STACK_BASE + sp, 0x0200);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 2, 0x0040);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_GP_HANDLER_IP as u32 + 1);

    let handler_sp = cpu.esp();
    let error_code = read_dword_at(&bus, PM_STACK_BASE + handler_sp);
    assert_eq!(error_code, 0);
}

#[test]
fn i386_ret_far_same_privilege_invalid_cs_preserves_source_frame() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.seg_granularity[cpu::SegReg32::CS as usize] = 0x40;
    state.seg_limits[cpu::SegReg32::CS as usize] = 0xFFFF_FFFF;
    state.seg_granularity[cpu::SegReg32::SS as usize] = 0x40;
    state.seg_limits[cpu::SegReg32::SS as usize] = 0xFFFF_FFFF;
    state.set_esp(0x0001_0000);
    cpu.load_state(&state);

    let original_esp = cpu.esp();
    write_dword_at(&mut bus, PM_STACK_BASE + original_esp, 0x2000);
    write_dword_at(&mut bus, PM_STACK_BASE + original_esp + 4, 0x0040);
    place_at(&mut bus, PM_CODE_BASE, &[0xCB]);

    cpu.step(&mut bus);

    assert_eq!(cpu.ip(), u32::from(PM_GP_HANDLER_IP));
    assert_eq!(cpu.esp(), original_esp.wrapping_sub(16));
    assert_eq!(
        read_dword_at(&bus, PM_STACK_BASE + cpu.esp()),
        0x0040,
        "error code should contain the invalid return selector"
    );
    assert_eq!(
        read_dword_at(&bus, PM_STACK_BASE + cpu.esp() + 4),
        0x0000,
        "fault frame should point back to the RETF instruction"
    );
    assert_eq!(
        read_dword_at(&bus, PM_STACK_BASE + original_esp),
        0x2000,
        "the original RETF frame must remain in place"
    );
}

#[test]
fn i386_pop_segment_invalid_selector_preserves_source_frame() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.seg_granularity[cpu::SegReg32::CS as usize] = 0x40;
    state.seg_limits[cpu::SegReg32::CS as usize] = 0xFFFF_FFFF;
    state.seg_granularity[cpu::SegReg32::SS as usize] = 0x40;
    state.seg_limits[cpu::SegReg32::SS as usize] = 0xFFFF_FFFF;
    state.set_esp(0x0001_0000);
    cpu.load_state(&state);

    let original_esp = cpu.esp();
    write_dword_at(&mut bus, PM_STACK_BASE + original_esp, 0x0040);
    place_at(&mut bus, PM_CODE_BASE, &[0x07]);

    cpu.step(&mut bus);

    assert_eq!(cpu.ip(), u32::from(PM_GP_HANDLER_IP));
    assert_eq!(cpu.esp(), original_esp.wrapping_sub(16));
    assert_eq!(
        read_dword_at(&bus, PM_STACK_BASE + cpu.esp()),
        0x0040,
        "error code should contain the invalid popped selector"
    );
    assert_eq!(
        read_dword_at(&bus, PM_STACK_BASE + cpu.esp() + 4),
        0x0000,
        "fault frame should point back to the POP ES instruction"
    );
    assert_eq!(
        read_dword_at(&bus, PM_STACK_BASE + original_esp),
        0x0040,
        "the original POP source slot must remain in place"
    );
}

#[test]
fn i386_invalidate_nonconforming_code_in_ds_on_return() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_ring3(&mut bus);
    cpu.load_state(&state);

    cpu.state.seg_rights[cpu::SegReg32::DS as usize] = 0x9B;

    place_at(&mut bus, PM_CODE_BASE, &[0xCB]);

    let ring3_ip: u16 = 0x0100;
    bus.ram[(PM_RING3_CODE_BASE + ring3_ip as u32) as usize] = 0xF4;

    let sp = cpu.esp();
    write_word_at(&mut bus, PM_STACK_BASE + sp, ring3_ip);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 2, PM_RING3_CS_SEL);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 4, 0xFF00);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 6, PM_RING3_SS_SEL);

    cpu.step(&mut bus);

    assert_eq!(cpu.cs() & 3, 3);
    assert!(!cpu.state.seg_valid[cpu::SegReg32::DS as usize]);
}

#[test]
fn i386_keep_nonconforming_code_in_ds_when_dpl_sufficient() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_ring3(&mut bus);
    cpu.load_state(&state);

    // Replace GDT entry 2 (PM_DS_SEL) with a readable non-conforming code
    // segment at DPL 3 so the cached and stored rights agree.
    write_gdt_entry16(&mut bus, PM_GDT_BASE, 2, PM_DATA_BASE, 0xFFFF, 0xFB);
    cpu.state.seg_rights[cpu::SegReg32::DS as usize] = 0xFB;

    place_at(&mut bus, PM_CODE_BASE, &[0xCB]);

    let ring3_ip: u16 = 0x0100;
    bus.ram[(PM_RING3_CODE_BASE + ring3_ip as u32) as usize] = 0xF4;

    let sp = cpu.esp();
    write_word_at(&mut bus, PM_STACK_BASE + sp, ring3_ip);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 2, PM_RING3_CS_SEL);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 4, 0xFF00);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 6, PM_RING3_SS_SEL);

    cpu.step(&mut bus);

    assert_eq!(cpu.cs() & 3, 3);
    assert!(cpu.state.seg_valid[cpu::SegReg32::DS as usize]);
}

#[test]
fn i386_keep_conforming_code_in_ds_on_return() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_ring3(&mut bus);
    cpu.load_state(&state);

    // Replace GDT entry 2 (PM_DS_SEL) with a readable conforming code segment
    // at DPL 0 so the cached and stored rights agree.
    write_gdt_entry16(&mut bus, PM_GDT_BASE, 2, PM_DATA_BASE, 0xFFFF, 0x9F);
    cpu.state.seg_rights[cpu::SegReg32::DS as usize] = 0x9F;

    place_at(&mut bus, PM_CODE_BASE, &[0xCB]);

    let ring3_ip: u16 = 0x0100;
    bus.ram[(PM_RING3_CODE_BASE + ring3_ip as u32) as usize] = 0xF4;

    let sp = cpu.esp();
    write_word_at(&mut bus, PM_STACK_BASE + sp, ring3_ip);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 2, PM_RING3_CS_SEL);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 4, 0xFF00);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 6, PM_RING3_SS_SEL);

    cpu.step(&mut bus);

    assert_eq!(cpu.cs() & 3, 3);
    assert!(cpu.state.seg_valid[cpu::SegReg32::DS as usize]);
}

#[test]
fn i386_ss_limit_violation_uses_ss_vector() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let mut state = setup_protected_mode_with_ring3(&mut bus);
    make_ring3_state(&mut state);
    state.seg_limits[cpu::SegReg32::SS as usize] = 0x0004;
    state.set_esp(0xFFF0);
    state.flags.expand(0x0002);
    cpu.load_state(&state);

    place_at(&mut bus, PM_RING3_CODE_BASE, &[0x50]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_SS_HANDLER_IP as u32 + 1);

    let handler_sp = cpu.esp();
    let error_code = read_dword_at(&bus, PM_STACK_BASE + handler_sp);
    assert_eq!(error_code, 0);
}

#[test]
fn i386_ds_limit_violation_uses_gp_vector() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_ring3(&mut bus);
    cpu.load_state(&state);

    cpu.state.seg_limits[cpu::SegReg32::DS as usize] = 0x0010;

    place_at(&mut bus, PM_CODE_BASE, &[0xA3, 0x20, 0x00]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_GP_HANDLER_IP as u32 + 1);

    let handler_sp = cpu.esp();
    let error_code = read_dword_at(&bus, PM_STACK_BASE + handler_sp);
    assert_eq!(error_code, 0);
}

#[test]
fn i386_hardware_interrupt_error_code_has_ext_bit() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_ring3(&mut bus);
    cpu.load_state(&state);

    cpu.state.idt_limit = 14 * 8 - 1;
    cpu.state.flags.expand(0x0202);

    place_at(&mut bus, PM_CODE_BASE, &[0x90]);

    bus.irq_vector = 14;
    cpu.signal_irq();

    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_GP_HANDLER_IP as u32 + 1);

    let handler_sp = cpu.esp();
    let error_code = read_dword_at(&bus, PM_STACK_BASE + handler_sp);
    assert_eq!(error_code, (14 * 8 + 2 + 1) as u32);
}

#[test]
fn i386_software_interrupt_error_code_no_ext_bit() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_ring3(&mut bus);
    cpu.load_state(&state);

    cpu.state.idt_limit = 14 * 8 - 1;

    place_at(&mut bus, PM_CODE_BASE, &[0xCD, 0x0E]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_GP_HANDLER_IP as u32 + 1);

    let handler_sp = cpu.esp();
    let error_code = read_dword_at(&bus, PM_STACK_BASE + handler_sp);
    assert_eq!(error_code, (14 * 8 + 2) as u32);
}

const PM_TSS2_BASE: u32 = 0x71000;
const PM_NP_HANDLER_IP: u16 = 0xA000;
const PM_TSS2_SEL: u16 = 0x0048;
const PM_CONFORMING_CS_SEL: u16 = 0x0058;

#[allow(clippy::too_many_arguments)]
fn write_gdt_gate(
    bus: &mut TestBus,
    gdt_base: u32,
    entry_index: u16,
    offset: u32,
    selector: u16,
    param_count: u8,
    gate_type: u8,
    dpl: u8,
) {
    let addr = (gdt_base + (entry_index as u32) * 8) as usize;
    bus.ram[addr] = offset as u8;
    bus.ram[addr + 1] = (offset >> 8) as u8;
    bus.ram[addr + 2] = selector as u8;
    bus.ram[addr + 3] = (selector >> 8) as u8;
    bus.ram[addr + 4] = param_count & 0x1F;
    bus.ram[addr + 5] = 0x80 | (dpl << 5) | gate_type;
    bus.ram[addr + 6] = (offset >> 16) as u8;
    bus.ram[addr + 7] = (offset >> 24) as u8;
}

#[allow(clippy::too_many_arguments)]
fn write_tss386(
    bus: &mut TestBus,
    tss_base: u32,
    backlink: u16,
    esp0: u32,
    ss0: u16,
    eip: u32,
    eflags: u32,
    eax: u32,
    ecx: u32,
    edx: u32,
    ebx: u32,
    esp: u32,
    ebp: u32,
    esi: u32,
    edi: u32,
    es: u16,
    cs: u16,
    ss: u16,
    ds: u16,
    fs: u16,
    gs: u16,
    ldt: u16,
) {
    write_word_at(bus, tss_base, backlink);
    write_dword_at(bus, tss_base + 4, esp0);
    write_word_at(bus, tss_base + 8, ss0);
    write_dword_at(bus, tss_base + 12, 0);
    write_word_at(bus, tss_base + 16, 0);
    write_dword_at(bus, tss_base + 20, 0);
    write_word_at(bus, tss_base + 24, 0);
    write_dword_at(bus, tss_base + 28, 0); // CR3
    write_dword_at(bus, tss_base + 32, eip);
    write_dword_at(bus, tss_base + 36, eflags);
    write_dword_at(bus, tss_base + 40, eax);
    write_dword_at(bus, tss_base + 44, ecx);
    write_dword_at(bus, tss_base + 48, edx);
    write_dword_at(bus, tss_base + 52, ebx);
    write_dword_at(bus, tss_base + 56, esp);
    write_dword_at(bus, tss_base + 60, ebp);
    write_dword_at(bus, tss_base + 64, esi);
    write_dword_at(bus, tss_base + 68, edi);
    write_word_at(bus, tss_base + 72, es);
    write_word_at(bus, tss_base + 76, cs);
    write_word_at(bus, tss_base + 80, ss);
    write_word_at(bus, tss_base + 84, ds);
    write_word_at(bus, tss_base + 88, fs);
    write_word_at(bus, tss_base + 92, gs);
    write_word_at(bus, tss_base + 96, ldt);
}

fn setup_protected_mode_extended(bus: &mut TestBus) -> cpu::I386State {
    write_gdt_entry16(bus, PM_GDT_BASE, 0, 0, 0, 0);
    write_gdt_entry16(bus, PM_GDT_BASE, 1, PM_CODE_BASE, 0xFFFF, 0x9B);
    write_gdt_entry16(bus, PM_GDT_BASE, 2, PM_DATA_BASE, 0xFFFF, 0x93);
    write_gdt_entry16(bus, PM_GDT_BASE, 3, PM_STACK_BASE, 0xFFFF, 0x93);
    write_gdt_entry16(bus, PM_GDT_BASE, 4, PM_RING3_CODE_BASE, 0xFFFF, 0xFB);
    write_gdt_entry16(bus, PM_GDT_BASE, 5, PM_DATA_BASE, 0xFFFF, 0xF3);
    write_gdt_entry16(bus, PM_GDT_BASE, 6, PM_RING3_STACK_BASE, 0xFFFF, 0xF3);
    write_gdt_entry16(bus, PM_GDT_BASE, 7, PM_TSS_BASE, 103, 0x8B);
    write_gdt_entry16(bus, PM_GDT_BASE, 8, 0, 0, 0);
    write_gdt_entry16(bus, PM_GDT_BASE, 9, PM_TSS2_BASE, 103, 0x89);
    write_gdt_entry16(bus, PM_GDT_BASE, 10, 0, 0, 0);
    write_gdt_entry16(bus, PM_GDT_BASE, 11, PM_CODE_BASE, 0xFFFF, 0x9F);

    write_idt_gate(
        bus,
        PM_IDT_BASE,
        10,
        PM_TS_HANDLER_IP as u32,
        PM_CS_SEL,
        14,
        0,
    );
    write_idt_gate(
        bus,
        PM_IDT_BASE,
        11,
        PM_NP_HANDLER_IP as u32,
        PM_CS_SEL,
        14,
        0,
    );
    write_idt_gate(
        bus,
        PM_IDT_BASE,
        12,
        PM_SS_HANDLER_IP as u32,
        PM_CS_SEL,
        14,
        0,
    );
    write_idt_gate(
        bus,
        PM_IDT_BASE,
        13,
        PM_GP_HANDLER_IP as u32,
        PM_CS_SEL,
        14,
        0,
    );

    bus.ram[(PM_CODE_BASE + PM_TS_HANDLER_IP as u32) as usize] = 0xF4;
    bus.ram[(PM_CODE_BASE + PM_NP_HANDLER_IP as u32) as usize] = 0xF4;
    bus.ram[(PM_CODE_BASE + PM_SS_HANDLER_IP as u32) as usize] = 0xF4;
    bus.ram[(PM_CODE_BASE + PM_GP_HANDLER_IP as u32) as usize] = 0xF4;

    write_dword_at(bus, PM_TSS_BASE + 4, 0xFFF0);
    write_word_at(bus, PM_TSS_BASE + 8, PM_SS_SEL);

    let mut state = cpu::I386State {
        cr0: 0x0001,
        ip: 0x0000,
        ..Default::default()
    };

    state.set_esp(0xFFF0);
    state.set_cs(PM_CS_SEL);
    state.set_ds(PM_DS_SEL);
    state.set_ss(PM_SS_SEL);
    state.set_es(PM_DS_SEL);

    state.seg_bases[cpu::SegReg32::ES as usize] = PM_DATA_BASE;
    state.seg_bases[cpu::SegReg32::CS as usize] = PM_CODE_BASE;
    state.seg_bases[cpu::SegReg32::SS as usize] = PM_STACK_BASE;
    state.seg_bases[cpu::SegReg32::DS as usize] = PM_DATA_BASE;

    state.seg_limits = [0xFFFF; 6];
    state.seg_rights[cpu::SegReg32::ES as usize] = 0x93;
    state.seg_rights[cpu::SegReg32::CS as usize] = 0x9B;
    state.seg_rights[cpu::SegReg32::SS as usize] = 0x93;
    state.seg_rights[cpu::SegReg32::DS as usize] = 0x93;
    state.seg_valid = [true, true, true, true, false, false];

    state.gdt_base = PM_GDT_BASE;
    state.gdt_limit = 12 * 8 - 1;
    state.idt_base = PM_IDT_BASE;
    state.idt_limit = 256 * 8 - 1;

    state.tr = PM_TSS_SEL;
    state.tr_base = PM_TSS_BASE;
    state.tr_limit = 103;
    state.tr_rights = 0x8B;

    state
}

#[test]
fn i386_ss_type_violation_raises_gp_not_ss() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_extended(&mut bus);
    cpu.load_state(&state);

    write_gdt_entry16(&mut bus, PM_GDT_BASE, 3, PM_STACK_BASE, 0xFFFF, 0x9B);

    place_at(&mut bus, PM_CODE_BASE, &[0xB8, 0x18, 0x00, 0x8E, 0xD0]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_GP_HANDLER_IP as u32 + 1);
}

#[test]
fn i386_ss_privilege_violation_raises_gp_not_ss() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_extended(&mut bus);
    cpu.load_state(&state);

    write_gdt_entry16(&mut bus, PM_GDT_BASE, 3, PM_STACK_BASE, 0xFFFF, 0xF3);

    place_at(&mut bus, PM_CODE_BASE, &[0xB8, 0x18, 0x00, 0x8E, 0xD0]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_GP_HANDLER_IP as u32 + 1);
}

#[test]
fn i386_ss_not_present_raises_ss() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_extended(&mut bus);
    cpu.load_state(&state);

    write_gdt_entry16(&mut bus, PM_GDT_BASE, 3, PM_STACK_BASE, 0xFFFF, 0x13);

    place_at(&mut bus, PM_CODE_BASE, &[0xB8, 0x18, 0x00, 0x8E, 0xD0]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_SS_HANDLER_IP as u32 + 1);
}

#[test]
fn i386_ret_far_to_conforming_code_adopts_rpl() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_extended(&mut bus);
    cpu.load_state(&state);

    let conforming_sel_rpl3: u16 = PM_CONFORMING_CS_SEL | 3;
    let target_ip: u16 = 0x0200;

    let sp = cpu.esp();
    write_word_at(&mut bus, PM_STACK_BASE + sp, target_ip);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 2, conforming_sel_rpl3);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 4, 0xF000);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 6, PM_RING3_SS_SEL);

    place_at(&mut bus, PM_CODE_BASE, &[0xCB]);

    cpu.step(&mut bus);

    assert_eq!(cpu.ip() as u16, target_ip);
    assert_eq!(cpu.cs() & 3, 3);
}

#[test]
fn i386_call_far_through_call_gate_same_privilege() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_extended(&mut bus);
    cpu.load_state(&state);

    let gate_target_ip: u16 = 0x0100;
    write_gdt_gate(
        &mut bus,
        PM_GDT_BASE,
        8,
        gate_target_ip as u32,
        PM_CS_SEL,
        0,
        4,
        0,
    );

    bus.ram[(PM_CODE_BASE + gate_target_ip as u32) as usize] = 0xF4;

    place_at(&mut bus, PM_CODE_BASE, &[0x9A, 0x99, 0x99, 0x40, 0x00]);

    let old_cs = cpu.cs();

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), gate_target_ip as u32 + 1);

    let sp = cpu.esp();
    let pushed_ip = read_word_at(&bus, PM_STACK_BASE + sp);
    let pushed_cs = read_word_at(&bus, PM_STACK_BASE + sp + 2);
    assert_eq!(pushed_ip, 5);
    assert_eq!(pushed_cs, old_cs);
}

#[test]
fn i386_call_far_through_call_gate_inter_privilege() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let mut state = setup_protected_mode_extended(&mut bus);
    make_ring3_state(&mut state);
    cpu.load_state(&state);

    let gate_target_ip: u16 = 0x0100;
    write_gdt_gate(
        &mut bus,
        PM_GDT_BASE,
        8,
        gate_target_ip as u32,
        PM_CS_SEL,
        0,
        4,
        3,
    );

    bus.ram[(PM_CODE_BASE + gate_target_ip as u32) as usize] = 0xF4;

    place_at(
        &mut bus,
        PM_RING3_CODE_BASE,
        &[0x9A, 0x00, 0x00, 0x40, 0x00],
    );

    let old_ss = cpu.ss();
    let old_sp = cpu.esp();
    let old_cs = cpu.cs();

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), gate_target_ip as u32 + 1);
    assert_eq!(cpu.cs() & 3, 0);

    let sp = cpu.esp();
    let ss_base = cpu.state.seg_bases[cpu::SegReg32::SS as usize];
    let pushed_ip = read_word_at(&bus, ss_base + sp);
    let pushed_cs = read_word_at(&bus, ss_base + sp + 2);
    let pushed_sp = read_word_at(&bus, ss_base + sp + 4);
    let pushed_ss = read_word_at(&bus, ss_base + sp + 6);
    assert_eq!(pushed_ss, old_ss);
    assert_eq!(pushed_sp, old_sp as u16);
    assert_eq!(pushed_cs, old_cs);
    assert_eq!(pushed_ip, 5);
}

#[test]
fn i386_call_far_through_call_gate_with_parameters() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let mut state = setup_protected_mode_extended(&mut bus);
    make_ring3_state(&mut state);
    cpu.load_state(&state);

    let gate_target_ip: u16 = 0x0100;
    write_gdt_gate(
        &mut bus,
        PM_GDT_BASE,
        8,
        gate_target_ip as u32,
        PM_CS_SEL,
        2,
        4,
        3,
    );

    bus.ram[(PM_CODE_BASE + gate_target_ip as u32) as usize] = 0xF4;

    let r3_sp = cpu.esp();
    write_word_at(&mut bus, PM_RING3_STACK_BASE + r3_sp - 2, 0xBBBB);
    write_word_at(&mut bus, PM_RING3_STACK_BASE + r3_sp - 4, 0xAAAA);
    cpu.state.set_esp(r3_sp - 4);

    place_at(
        &mut bus,
        PM_RING3_CODE_BASE,
        &[0x9A, 0x00, 0x00, 0x40, 0x00],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());

    let sp = cpu.esp();
    let ss_base = cpu.state.seg_bases[cpu::SegReg32::SS as usize];
    let pushed_ip = read_word_at(&bus, ss_base + sp);
    let param1 = read_word_at(&bus, ss_base + sp + 4);
    let param2 = read_word_at(&bus, ss_base + sp + 6);
    assert_eq!(pushed_ip, 5);
    assert_eq!(param1, 0xAAAA);
    assert_eq!(param2, 0xBBBB);
    let pushed_sp = read_word_at(&bus, ss_base + sp + 8);
    let pushed_ss = read_word_at(&bus, ss_base + sp + 10);
    assert_eq!(pushed_ss & 3, 3);
    assert_eq!(pushed_sp, (r3_sp - 4) as u16);
}

#[test]
fn i386_call_gate_inter_priv_too_small_stack_raises_ss_zero() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let mut state = setup_protected_mode_extended(&mut bus);
    make_ring3_state(&mut state);
    cpu.load_state(&state);

    // Shrink the kernel stack pointer (TSS ESP0) to a value that cannot fit
    // even the minimum return frame for a 32-bit gate (4 dwords = 16 bytes).
    write_dword_at(&mut bus, PM_TSS_BASE + 4, 8);

    // 32-bit call gate, no parameters; required stack space = 16 bytes,
    // available = 8 bytes -> #SS(0).
    let gate_target_ip: u32 = 0x0000_0100;
    write_gdt_gate(
        &mut bus,
        PM_GDT_BASE,
        8,
        gate_target_ip,
        PM_CS_SEL,
        0,
        12,
        3,
    );
    bus.ram[(PM_CODE_BASE + gate_target_ip) as usize] = 0xF4;

    place_at(
        &mut bus,
        PM_RING3_CODE_BASE,
        &[0x9A, 0x00, 0x00, 0x40, 0x00], // CALL FAR 0x0040:0
    );

    cpu.step(&mut bus); // CALL FAR -> #SS(0)
    cpu.step(&mut bus); // HLT in #SS handler

    assert!(cpu.halted(), "expected to land at the #SS handler HLT");
    assert_eq!(
        cpu.ip(),
        PM_SS_HANDLER_IP as u32 + 1,
        "should be at #SS handler"
    );

    let sp = cpu.esp();
    let ss_base = cpu.state.seg_bases[cpu::SegReg32::SS as usize];
    let error_code = read_dword_at(&bus, ss_base + sp);
    assert_eq!(error_code, 0, "stack-space #SS error code must be 0");
}

#[test]
fn i386_jmp_far_through_call_gate_same_privilege() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_extended(&mut bus);
    cpu.load_state(&state);

    let gate_target_ip: u16 = 0x0100;
    write_gdt_gate(
        &mut bus,
        PM_GDT_BASE,
        8,
        gate_target_ip as u32,
        PM_CS_SEL,
        0,
        4,
        0,
    );

    bus.ram[(PM_CODE_BASE + gate_target_ip as u32) as usize] = 0xF4;

    place_at(&mut bus, PM_CODE_BASE, &[0xEA, 0x00, 0x00, 0x40, 0x00]);

    let sp_before = cpu.esp();

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), gate_target_ip as u32 + 1);
    assert_eq!(cpu.esp(), sp_before);
}

#[test]
fn i386_jmp_far_call_gate_inner_privilege_faults() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let mut state = setup_protected_mode_extended(&mut bus);
    make_ring3_state(&mut state);
    cpu.load_state(&state);

    write_gdt_gate(&mut bus, PM_GDT_BASE, 8, 0x0100, PM_CS_SEL, 0, 4, 3);

    place_at(
        &mut bus,
        PM_RING3_CODE_BASE,
        &[0xEA, 0x00, 0x00, 0x40, 0x00],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_GP_HANDLER_IP as u32 + 1);
}

#[test]
fn i386_call_gate_dpl_insufficient_faults() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let mut state = setup_protected_mode_extended(&mut bus);
    make_ring3_state(&mut state);
    cpu.load_state(&state);

    write_gdt_gate(&mut bus, PM_GDT_BASE, 8, 0x0100, PM_CS_SEL, 0, 4, 0);

    place_at(
        &mut bus,
        PM_RING3_CODE_BASE,
        &[0x9A, 0x00, 0x00, 0x43, 0x00],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_GP_HANDLER_IP as u32 + 1);
}

#[test]
fn i386_jmp_far_to_tss_switches_task() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_extended(&mut bus);
    cpu.load_state(&state);

    cpu.state.set_eax(0x1111);
    cpu.state.set_ebx(0x2222);

    let target_ip: u16 = 0x0300;
    write_tss386(
        &mut bus,
        PM_TSS2_BASE,
        0,
        0xFFF0,
        PM_SS_SEL,
        target_ip as u32,
        0x0002,
        0xAAAA,
        0xBBBB,
        0xCCCC,
        0xDDDD,
        0xEE00,
        0xFF00,
        0x1100,
        0x2200,
        PM_DS_SEL,
        PM_CS_SEL,
        PM_SS_SEL,
        PM_DS_SEL,
        0,
        0,
        0,
    );

    bus.ram[(PM_CODE_BASE + target_ip as u32) as usize] = 0xF4;

    place_at(&mut bus, PM_CODE_BASE, &[0xEA, 0x00, 0x00, 0x48, 0x00]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), target_ip as u32 + 1);
    assert_eq!(cpu.eax(), 0xAAAA);
    assert_eq!(cpu.state.ebx(), 0xDDDD);
    assert_eq!(cpu.state.ecx(), 0xBBBB);
    assert_eq!(cpu.state.tr, PM_TSS2_SEL);

    let saved_eax = read_dword_at(&bus, PM_TSS_BASE + 40);
    let saved_ebx = read_dword_at(&bus, PM_TSS_BASE + 52);
    assert_eq!(saved_eax, 0x1111);
    assert_eq!(saved_ebx, 0x2222);

    let old_tss_rights = bus.ram[(PM_GDT_BASE + 7 * 8 + 5) as usize];
    let new_tss_rights = bus.ram[(PM_GDT_BASE + 9 * 8 + 5) as usize];
    assert_eq!(old_tss_rights & 0x02, 0, "Old TSS should be idle");
    assert_ne!(new_tss_rights & 0x02, 0, "New TSS should be busy");
}

#[test]
fn i386_call_far_to_tss_sets_nt_and_backlink() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_extended(&mut bus);
    cpu.load_state(&state);

    let target_ip: u16 = 0x0300;
    write_tss386(
        &mut bus,
        PM_TSS2_BASE,
        0,
        0xFFF0,
        PM_SS_SEL,
        target_ip as u32,
        0x0002,
        0,
        0,
        0,
        0,
        0xEE00,
        0,
        0,
        0,
        PM_DS_SEL,
        PM_CS_SEL,
        PM_SS_SEL,
        PM_DS_SEL,
        0,
        0,
        0,
    );

    bus.ram[(PM_CODE_BASE + target_ip as u32) as usize] = 0xF4;

    place_at(&mut bus, PM_CODE_BASE, &[0x9A, 0x00, 0x00, 0x48, 0x00]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert!(
        cpu.state.flags.nt,
        "NT should be set after CALL task switch"
    );

    let backlink = read_word_at(&bus, PM_TSS2_BASE);
    assert_eq!(backlink, PM_TSS_SEL);

    let old_tss_rights = bus.ram[(PM_GDT_BASE + 7 * 8 + 5) as usize];
    assert_ne!(old_tss_rights & 0x02, 0, "Old TSS should remain busy");
}

#[test]
fn i386_iret_with_nt_returns_to_previous_task() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_extended(&mut bus);
    cpu.load_state(&state);

    let task2_ip: u16 = 0x0300;
    write_tss386(
        &mut bus,
        PM_TSS2_BASE,
        0,
        0xFFF0,
        PM_SS_SEL,
        task2_ip as u32,
        0x0002,
        0,
        0,
        0,
        0,
        0xEE00,
        0,
        0,
        0,
        PM_DS_SEL,
        PM_CS_SEL,
        PM_SS_SEL,
        PM_DS_SEL,
        0,
        0,
        0,
    );

    bus.ram[(PM_CODE_BASE + task2_ip as u32) as usize] = 0xCF; // IRET
    bus.ram[(PM_CODE_BASE + 5) as usize] = 0xF4; // HLT

    place_at(&mut bus, PM_CODE_BASE, &[0x9A, 0x00, 0x00, 0x48, 0x00]);

    bus.ram[(PM_GDT_BASE + 7 * 8 + 5) as usize] |= 0x02;

    cpu.step(&mut bus); // CALL FAR -> task 2
    cpu.step(&mut bus); // IRET -> task 1
    cpu.step(&mut bus); // HLT

    assert!(cpu.halted());
    assert_eq!(cpu.state.tr, PM_TSS_SEL);
    assert_eq!(cpu.ip(), 6);
}

#[test]
fn i386_task_switch_saves_and_restores_registers() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_extended(&mut bus);
    cpu.load_state(&state);

    cpu.state.set_eax(0x1234);
    cpu.state.set_ecx(0x5678);
    cpu.state.set_edx(0x9ABC);
    cpu.state.set_ebx(0xDEF0);
    cpu.state.set_ebp(0x1357);
    cpu.state.set_esi(0x2468);
    cpu.state.set_edi(0x3579);

    let task2_ip: u16 = 0x0300;
    write_tss386(
        &mut bus,
        PM_TSS2_BASE,
        0,
        0xFFF0,
        PM_SS_SEL,
        task2_ip as u32,
        0x0002,
        0xFF00,
        0xEE00,
        0xDD00,
        0xCC00,
        0xBB00,
        0xAA00,
        0x9900,
        0x8800,
        PM_DS_SEL,
        PM_CS_SEL,
        PM_SS_SEL,
        PM_DS_SEL,
        0,
        0,
        0,
    );

    bus.ram[(PM_CODE_BASE + task2_ip as u32) as usize] = 0xCF;
    bus.ram[(PM_CODE_BASE + 5) as usize] = 0xF4;

    bus.ram[(PM_GDT_BASE + 7 * 8 + 5) as usize] |= 0x02;

    place_at(&mut bus, PM_CODE_BASE, &[0x9A, 0x00, 0x00, 0x48, 0x00]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.eax(), 0x1234);
    assert_eq!(cpu.state.ecx(), 0x5678);
    assert_eq!(cpu.state.edx(), 0x9ABC);
    assert_eq!(cpu.state.ebx(), 0xDEF0);
    assert_eq!(cpu.state.ebp(), 0x1357);
    assert_eq!(cpu.state.esi(), 0x2468);
    assert_eq!(cpu.state.edi(), 0x3579);
}

#[test]
fn i386_task_switch_cs_with_dpl_ne_rpl_raises_ts() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_extended(&mut bus);
    cpu.load_state(&state);

    let task2_ip: u16 = 0x0300;
    // CS selector 0x0020 = GDT index 4 (DPL=3 ring3 code) with RPL=0.
    // For non-conforming code, DPL must equal RPL, so this must raise #TS.
    let bad_cs: u16 = 0x0020;
    write_tss386(
        &mut bus,
        PM_TSS2_BASE,
        0,
        0xFFF0,
        PM_SS_SEL,
        task2_ip as u32,
        0x0002,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        PM_DS_SEL,
        bad_cs,
        PM_SS_SEL,
        PM_DS_SEL,
        0,
        0,
        0,
    );

    bus.ram[(PM_CODE_BASE + task2_ip as u32) as usize] = 0xF4;

    place_at(&mut bus, PM_CODE_BASE, &[0x9A, 0x00, 0x00, 0x48, 0x00]);

    cpu.step(&mut bus); // CALL FAR -> task switch -> #TS
    cpu.step(&mut bus); // HLT in #TS handler

    assert!(cpu.halted(), "expected to land at the #TS handler HLT");
    assert_eq!(
        cpu.ip(),
        PM_TS_HANDLER_IP as u32 + 1,
        "should be at #TS handler"
    );
}

#[test]
fn i386_task_switch_loads_cr3_when_paging_enabled() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let mut state = setup_protected_mode_extended(&mut bus);

    // Two identity-mapped page directories pointing at the same page table,
    // so the GDT and TSSes are reachable through either CR3.
    enable_identity_paging(&mut bus, &mut state);
    const NEW_TASK_CR3: u32 = 0xB_0000;
    write_dword_at(
        &mut bus,
        NEW_TASK_CR3,
        TEST_PAGE_TABLE_BASE | TEST_PAGE_PRESENT_WRITABLE,
    );

    let task2_ip: u16 = 0x0300;
    write_tss386(
        &mut bus,
        PM_TSS2_BASE,
        0,
        0xFFF0,
        PM_SS_SEL,
        task2_ip as u32,
        0x0002,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        PM_DS_SEL,
        PM_CS_SEL,
        PM_SS_SEL,
        PM_DS_SEL,
        0,
        0,
        0,
    );
    write_dword_at(&mut bus, PM_TSS2_BASE + 28, NEW_TASK_CR3);

    bus.ram[(PM_CODE_BASE + task2_ip as u32) as usize] = 0xF4;

    cpu.load_state(&state);

    place_at(&mut bus, PM_CODE_BASE, &[0x9A, 0x00, 0x00, 0x48, 0x00]);

    cpu.step(&mut bus); // CALL FAR -> task switch
    cpu.step(&mut bus); // HLT in new task

    assert!(cpu.halted());
    assert_eq!(
        cpu.cr3, NEW_TASK_CR3,
        "task switch must load CR3 from the new TSS when paging is enabled"
    );
}

#[test]
fn i386_fs_gs_segment_loading_in_protected_mode() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.gdt_limit = 6 * 8 - 1;
    cpu.load_state(&state);

    write_gdt_entry16(&mut bus, PM_GDT_BASE, 4, 0x40000, 0x1FFF, 0x93);
    write_gdt_entry16(&mut bus, PM_GDT_BASE, 5, 0x42000, 0x0FFF, 0x93);

    // MOV AX, 0x0020; MOV FS, AX; MOV AX, 0x0028; MOV GS, AX
    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[
            0xB8, 0x20, 0x00, 0x8E, 0xE0, // MOV FS, AX
            0xB8, 0x28, 0x00, 0x8E, 0xE8, // MOV GS, AX
        ],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_eq!(cpu.fs(), 0x0020);
    assert_eq!(cpu.state.seg_bases[cpu::SegReg32::FS as usize], 0x40000);
    assert_eq!(cpu.state.seg_limits[cpu::SegReg32::FS as usize], 0x1FFF);
    assert!(cpu.state.seg_valid[cpu::SegReg32::FS as usize]);

    assert_eq!(cpu.gs(), 0x0028);
    assert_eq!(cpu.state.seg_bases[cpu::SegReg32::GS as usize], 0x42000);
    assert_eq!(cpu.state.seg_limits[cpu::SegReg32::GS as usize], 0x0FFF);
    assert!(cpu.state.seg_valid[cpu::SegReg32::GS as usize]);
}

#[test]
fn i386_fs_gs_invalidated_on_privilege_return() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_ring3(&mut bus);
    cpu.load_state(&state);

    cpu.state.seg_rights[cpu::SegReg32::FS as usize] = 0x9B;
    cpu.state.seg_valid[cpu::SegReg32::FS as usize] = true;
    cpu.state.seg_rights[cpu::SegReg32::GS as usize] = 0x9B;
    cpu.state.seg_valid[cpu::SegReg32::GS as usize] = true;

    place_at(&mut bus, PM_CODE_BASE, &[0xCB]);

    let ring3_ip: u16 = 0x0100;
    bus.ram[(PM_RING3_CODE_BASE + ring3_ip as u32) as usize] = 0xF4;

    let sp = cpu.esp();
    write_word_at(&mut bus, PM_STACK_BASE + sp, ring3_ip);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 2, PM_RING3_CS_SEL);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 4, 0xFF00);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 6, PM_RING3_SS_SEL);

    cpu.step(&mut bus);

    assert_eq!(cpu.cs() & 3, 3);
    assert!(!cpu.state.seg_valid[cpu::SegReg32::FS as usize]);
    assert!(!cpu.state.seg_valid[cpu::SegReg32::GS as usize]);
}

#[test]
fn i386_granularity_bit_scales_limit() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.gdt_limit = 6 * 8 - 1;
    cpu.load_state(&state);

    // G=1, limit[19:16]=0xF, raw limit 0x0FFFF -> scaled = (0x0FFFF << 12) | 0xFFF = 0x0FFFFFFF
    write_gdt_entry(&mut bus, PM_GDT_BASE, 4, PM_DATA_BASE, 0x0FFFF, 0x93, 0x80);

    // MOV BX, 0x0020; 66 0F 03 C3 = LSL EAX, EBX
    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[0xBB, 0x20, 0x00, 0x66, 0x0F, 0x03, 0xC3],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf());
    assert_eq!(cpu.eax(), 0x0FFF_FFFF);
}

/// SS B-bit: push/pop use full ESP when SS descriptor has B-bit (bit 6 of granularity) set.
#[test]
fn i386_push_pop_use_esp_with_b_bit() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    // Set SS granularity byte with B-bit (0x40) to enable 32-bit stack ops.
    // Also need large limit for ESP > 0xFFFF.
    state.seg_granularity[cpu::SegReg32::SS as usize] = 0x40;
    state.seg_limits[cpu::SegReg32::SS as usize] = 0xFFFF_FFFF;
    // ESP within the 1MB test RAM, but > 0xFFFF to verify 32-bit operation.
    state.set_esp(0x0002_0000);
    cpu.load_state(&state);

    // PUSH AX (0x50), POP BX (0x5B)
    place_at(&mut bus, PM_CODE_BASE, &[0x50, 0x5B]);

    let esp_before = cpu.esp();
    cpu.step(&mut bus); // PUSH AX
    let esp_after_push = cpu.esp();
    assert_eq!(
        esp_before.wrapping_sub(2),
        esp_after_push,
        "PUSH should decrement full ESP by 2"
    );
    assert!(
        esp_after_push > 0xFFFF,
        "ESP should be > 0xFFFF (using 32-bit stack pointer)"
    );

    cpu.step(&mut bus); // POP BX
    assert_eq!(cpu.esp(), esp_before, "POP should restore ESP");
}

/// SS B-bit: push_dword/pop_dword use full ESP.
#[test]
fn i386_push_pop_dword_use_esp_with_b_bit() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.seg_granularity[cpu::SegReg32::SS as usize] = 0x40;
    state.seg_limits[cpu::SegReg32::SS as usize] = 0xFFFF_FFFF;
    state.set_esp(0x0001_0000);
    cpu.load_state(&state);

    // 66 50 = PUSH EAX, 66 5B = POP EBX
    place_at(&mut bus, PM_CODE_BASE, &[0x66, 0x50, 0x66, 0x5B]);

    let esp_before = cpu.esp();
    cpu.step(&mut bus); // PUSH EAX
    let esp_after_push = cpu.esp();
    assert_eq!(
        esp_before.wrapping_sub(4),
        esp_after_push,
        "PUSH EAX should decrement full ESP by 4"
    );

    cpu.step(&mut bus); // POP EBX
    assert_eq!(cpu.esp(), esp_before, "POP EBX should restore ESP");
}

/// IRET in protected mode with operand size override pops 32-bit EIP/CS/EFLAGS.
#[test]
fn i386_iret_32bit_protected_mode() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.seg_granularity[cpu::SegReg32::SS as usize] = 0x40;
    state.seg_limits[cpu::SegReg32::SS as usize] = 0xFFFF_FFFF;
    cpu.load_state(&state);

    let target_eip: u32 = 0x2000;
    let target_cs: u32 = PM_CS_SEL as u32;
    let target_eflags: u32 = 0x0000_0202; // IF set

    // Set up stack with 32-bit IRET frame.
    let sp = cpu.esp();
    write_dword_at(&mut bus, PM_STACK_BASE + sp - 12, target_eip);
    write_dword_at(&mut bus, PM_STACK_BASE + sp - 8, target_cs);
    write_dword_at(&mut bus, PM_STACK_BASE + sp - 4, target_eflags);
    cpu.state.set_esp(sp - 12);

    // 66 CF = 32-bit IRET
    place_at(&mut bus, PM_CODE_BASE, &[0x66, 0xCF]);

    bus.ram[(PM_CODE_BASE + target_eip) as usize] = 0xF4; // HLT at target

    cpu.step(&mut bus); // IRET
    cpu.step(&mut bus); // HLT

    assert!(cpu.halted());
    assert_eq!(
        cpu.ip(),
        target_eip + 1,
        "EIP should be at target + 1 after HLT"
    );
    assert_eq!(cpu.cs(), PM_CS_SEL);
}

#[test]
fn i386_iret_same_privilege_invalid_cs_preserves_source_frame() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.seg_granularity[cpu::SegReg32::SS as usize] = 0x40;
    state.seg_limits[cpu::SegReg32::SS as usize] = 0xFFFF_FFFF;
    cpu.load_state(&state);

    let original_esp = cpu.esp().wrapping_sub(12);
    write_dword_at(&mut bus, PM_STACK_BASE + original_esp, 0x2000);
    write_dword_at(&mut bus, PM_STACK_BASE + original_esp + 4, 0x0040);
    write_dword_at(&mut bus, PM_STACK_BASE + original_esp + 8, 0x0000_0202);
    cpu.state.set_esp(original_esp);

    place_at(&mut bus, PM_CODE_BASE, &[0x66, 0xCF]);

    cpu.step(&mut bus);

    assert_eq!(cpu.ip(), u32::from(PM_GP_HANDLER_IP));
    assert_eq!(cpu.esp(), original_esp.wrapping_sub(16));
    assert_eq!(
        read_dword_at(&bus, PM_STACK_BASE + cpu.esp()),
        0x0040,
        "error code should contain the invalid return selector"
    );
    assert_eq!(
        read_dword_at(&bus, PM_STACK_BASE + cpu.esp() + 4),
        0x0000,
        "fault frame should point back to the IRET instruction"
    );
    assert_eq!(
        read_dword_at(&bus, PM_STACK_BASE + original_esp),
        0x2000,
        "the original IRET frame must remain in place"
    );
}

/// RETF in protected mode with operand size override pops 32-bit EIP/CS.
#[test]
fn i386_retf_32bit_protected_mode() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.seg_granularity[cpu::SegReg32::SS as usize] = 0x40;
    state.seg_limits[cpu::SegReg32::SS as usize] = 0xFFFF_FFFF;
    cpu.load_state(&state);

    let target_eip: u32 = 0x3000;
    let target_cs: u32 = PM_CS_SEL as u32;

    let sp = cpu.esp();
    write_dword_at(&mut bus, PM_STACK_BASE + sp - 8, target_eip);
    write_dword_at(&mut bus, PM_STACK_BASE + sp - 4, target_cs);
    cpu.state.set_esp(sp - 8);

    // 66 CB = 32-bit RETF
    place_at(&mut bus, PM_CODE_BASE, &[0x66, 0xCB]);

    bus.ram[(PM_CODE_BASE + target_eip) as usize] = 0xF4; // HLT

    cpu.step(&mut bus); // RETF
    cpu.step(&mut bus); // HLT

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), target_eip + 1);
    assert_eq!(cpu.cs(), PM_CS_SEL);
}

#[test]
fn i386_ret_near_imm_uses_full_esp_on_32bit_stack() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.seg_granularity[cpu::SegReg32::CS as usize] = 0x40;
    state.seg_limits[cpu::SegReg32::CS as usize] = 0xFFFF_FFFF;
    state.seg_granularity[cpu::SegReg32::SS as usize] = 0x40;
    state.seg_limits[cpu::SegReg32::SS as usize] = 0xFFFF_FFFF;
    state.set_esp(0x0003_FFF8);
    cpu.load_state(&state);

    let stack_pointer = cpu.esp();
    let target_eip = 0x0200;
    write_dword_at(&mut bus, PM_STACK_BASE + stack_pointer, target_eip);

    place_at(&mut bus, PM_CODE_BASE, &[0xC2, 0x04, 0x00]);

    cpu.step(&mut bus);

    assert_eq!(
        cpu.ip(),
        target_eip,
        "RET imm16 should pop a 32-bit return EIP"
    );
    assert_eq!(
        cpu.esp(),
        stack_pointer.wrapping_add(8),
        "RET imm16 on a 32-bit stack must advance the full ESP"
    );
}

#[test]
fn i386_popa_ignores_discarded_esp_slot() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.seg_granularity[cpu::SegReg32::CS as usize] = 0x40;
    state.seg_limits[cpu::SegReg32::CS as usize] = 0xFFFF_FFFF;
    state.seg_granularity[cpu::SegReg32::SS as usize] = 0x40;
    state.seg_limits[cpu::SegReg32::SS as usize] = 0xFFFF_FFFF;
    state.set_esp(0x0001_0000);
    cpu.load_state(&state);

    let stack_pointer = cpu.esp();
    write_dword_at(&mut bus, PM_STACK_BASE + stack_pointer, 0x1111_1111);
    write_dword_at(&mut bus, PM_STACK_BASE + stack_pointer + 4, 0x2222_2222);
    write_dword_at(&mut bus, PM_STACK_BASE + stack_pointer + 8, 0x3333_3333);
    write_dword_at(&mut bus, PM_STACK_BASE + stack_pointer + 12, 0xC036_1234);
    write_dword_at(&mut bus, PM_STACK_BASE + stack_pointer + 16, 0x4444_4444);
    write_dword_at(&mut bus, PM_STACK_BASE + stack_pointer + 20, 0x5555_5555);
    write_dword_at(&mut bus, PM_STACK_BASE + stack_pointer + 24, 0x6666_6666);
    write_dword_at(&mut bus, PM_STACK_BASE + stack_pointer + 28, 0x7777_7777);

    place_at(&mut bus, PM_CODE_BASE, &[0x61]);

    cpu.step(&mut bus);

    assert_eq!(cpu.edi(), 0x1111_1111);
    assert_eq!(cpu.esi(), 0x2222_2222);
    assert_eq!(cpu.ebp(), 0x3333_3333);
    assert_eq!(cpu.ebx(), 0x4444_4444);
    assert_eq!(cpu.edx(), 0x5555_5555);
    assert_eq!(cpu.ecx(), 0x6666_6666);
    assert_eq!(cpu.eax(), 0x7777_7777);
    assert_eq!(
        cpu.esp(),
        stack_pointer.wrapping_add(32),
        "POPAD must ignore the discarded ESP slot and keep the post-pop ESP"
    );
}

/// PUSHFD/POPFD preserves upper EFLAGS bits.
#[test]
fn i386_pushfd_popfd_upper_eflags() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.seg_granularity[cpu::SegReg32::SS as usize] = 0x40;
    state.seg_limits[cpu::SegReg32::SS as usize] = 0xFFFF_FFFF;
    state.set_esp(0x0001_0000);
    // Set bits 18-31 and RF in upper EFLAGS (simulating hardware state).
    state.eflags_upper = 0xFFFD_0000;
    cpu.load_state(&state);

    // 66 9C = PUSHFD, 66 9D = POPFD
    place_at(&mut bus, PM_CODE_BASE, &[0x66, 0x9C, 0x66, 0x9D]);

    cpu.step(&mut bus); // PUSHFD
    let sp = cpu.esp();
    let pushed_flags = read_dword_at(&bus, PM_STACK_BASE + sp);
    assert_eq!(
        pushed_flags & 0xFFFC_0000,
        0,
        "PUSHFD should push bits 18-31 as 0"
    );
    assert_eq!(
        pushed_flags & 0x0001_0000,
        0,
        "PUSHFD should clear RF (bit 16)"
    );

    cpu.step(&mut bus); // POPFD
    assert_eq!(
        cpu.state.eflags_upper & 0xFFFC_0000,
        0xFFFC_0000,
        "POPFD should preserve bits 18-31 in eflags_upper"
    );
    assert_eq!(
        cpu.state.eflags_upper & 0x0001_0000,
        0,
        "POPFD should not restore RF from the stack image"
    );
}

/// Real-mode POPF can set IOPL and NT bits.
#[test]
fn i386_real_mode_popf_sets_iopl_nt() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    // Real mode: push flags with IOPL=3 and NT set, then popf.
    // 0x7002 = NT(bit14) | IOPL=3(bits12-13) | reserved(bit1)
    place_code(
        &mut bus,
        0xFFFF,
        0x0000,
        &[
            0x68, 0x02, 0x70, // PUSH 0x7002
            0x9D, // POPF
            0x9C, // PUSHF
            0x58, // POP AX
        ],
    );

    cpu.step(&mut bus); // PUSH
    cpu.step(&mut bus); // POPF
    cpu.step(&mut bus); // PUSHF
    cpu.step(&mut bus); // POP AX

    let flags = cpu.eax() as u16;
    assert_eq!(
        (flags >> 12) & 3,
        3,
        "Real-mode POPF should be able to set IOPL=3"
    );
    assert_ne!(flags & 0x4000, 0, "Real-mode POPF should be able to set NT");
}

/// CLTS at CPL != 0 triggers #GP.
#[test]
fn i386_clts_gp_at_cpl3() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_ring3(&mut bus);
    make_ring3_state(&mut state);
    cpu.load_state(&state);

    // 0F 06 = CLTS
    place_at(&mut bus, PM_RING3_CODE_BASE, &[0x0F, 0x06]);

    cpu.step(&mut bus); // CLTS -> #GP
    cpu.step(&mut bus); // HLT in GP handler

    assert!(cpu.halted());
    assert_eq!(
        cpu.ip(),
        PM_GP_HANDLER_IP as u32 + 1,
        "CLTS at CPL 3 should trigger #GP"
    );
}

/// MOV CR0, EAX at CPL != 0 triggers #GP.
#[test]
fn i386_mov_cr_gp_at_cpl3() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_ring3(&mut bus);
    make_ring3_state(&mut state);
    cpu.load_state(&state);

    // 0F 22 C0 = MOV CR0, EAX
    place_at(&mut bus, PM_RING3_CODE_BASE, &[0x0F, 0x22, 0xC0]);

    cpu.step(&mut bus); // MOV CR0 -> #GP
    cpu.step(&mut bus); // HLT in GP handler

    assert!(cpu.halted());
    assert_eq!(
        cpu.ip(),
        PM_GP_HANDLER_IP as u32 + 1,
        "MOV CR0 at CPL 3 should trigger #GP"
    );
}

/// MOV to CS (seg_index=1) triggers #UD.
#[test]
fn i386_mov_to_cs_triggers_ud() {
    use cpu::I386State;
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let ip: u16 = 0x0050;
    // 8E C8 = MOV CS, AX (modrm 0xC8: mod=11, reg=001 (CS), rm=000 (AX))
    place_code(&mut bus, cs, ip, &[0x8E, 0xC8]);

    let handler_cs: u16 = 0x2000;
    let handler_ip: u16 = 0x0000;
    let ivt_addr = 6 * 4; // INT 6 = #UD
    bus.ram[ivt_addr] = handler_ip as u8;
    bus.ram[ivt_addr + 1] = (handler_ip >> 8) as u8;
    bus.ram[ivt_addr + 2] = handler_cs as u8;
    bus.ram[ivt_addr + 3] = (handler_cs >> 8) as u8;

    let mut state = I386State::default();
    state.set_cs(cs);
    state.set_eip(ip as u32);
    state.set_ss(0x3000);
    state.set_esp(0x1000);
    cpu.load_state(&state);

    cpu.step(&mut bus);

    assert_eq!(cpu.cs(), handler_cs, "MOV to CS should trigger #UD (INT 6)");
}

/// SMSW to register writes full 32-bit CR0.
#[test]
fn i386_smsw_register_32bit() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    // SMSW EAX: 66 0F 01 E0 (modrm=0xE0: mod=11, /4, rm=EAX, with 66 prefix)
    place_code(&mut bus, 0xFFFF, 0x0000, &[0x66, 0x0F, 0x01, 0xE0]);

    cpu.step(&mut bus);

    assert_eq!(
        cpu.eax(),
        0x0000_0010,
        "SMSW to register should write full 32-bit CR0 (ET=1 after reset)"
    );
}

/// LAR rejects descriptor types 6 and 7 (286 interrupt/trap gates).
#[test]
fn i386_lar_rejects_int_trap_gates() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.gdt_limit = 6 * 8 - 1;
    cpu.load_state(&state);

    // Type 6 = 286 interrupt gate (DPL=0, present)
    let addr = (PM_GDT_BASE + 4 * 8) as usize;
    bus.ram[addr] = 0x00;
    bus.ram[addr + 1] = 0x00;
    bus.ram[addr + 2] = PM_CS_SEL as u8;
    bus.ram[addr + 3] = (PM_CS_SEL >> 8) as u8;
    bus.ram[addr + 4] = 0x00;
    bus.ram[addr + 5] = 0x86; // Present, DPL=0, type=6
    bus.ram[addr + 6] = 0x00;
    bus.ram[addr + 7] = 0x00;

    // MOV BX, 0x0020; LAR AX, BX
    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[0xBB, 0x20, 0x00, 0x0F, 0x02, 0xC3],
    );

    cpu.step(&mut bus); // MOV BX
    cpu.step(&mut bus); // LAR

    assert!(
        !cpu.state.flags.zf(),
        "LAR should reject type 6 (286 interrupt gate): ZF=0"
    );
}

/// IDT task gate (type 5) triggers task switch on interrupt.
#[test]
fn i386_idt_task_gate_triggers_task_switch() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_extended(&mut bus);
    cpu.load_state(&state);

    let target_ip: u16 = 0x0400;
    write_tss386(
        &mut bus,
        PM_TSS2_BASE,
        0,
        0xFFF0,
        PM_SS_SEL,
        target_ip as u32,
        0x0002,
        0xBEEF,
        0,
        0,
        0,
        0x2200,
        0,
        0,
        0,
        PM_DS_SEL,
        PM_CS_SEL,
        PM_SS_SEL,
        PM_DS_SEL,
        0,
        0,
        0,
    );

    bus.ram[(PM_CODE_BASE + target_ip as u32) as usize] = 0xF4;

    // Write IDT task gate at vector 0x40: type=5, DPL=3
    write_idt_gate(&mut bus, PM_IDT_BASE, 0x40, 0, PM_TSS2_SEL, 5, 3);

    // INT 0x40
    place_at(&mut bus, PM_CODE_BASE, &[0xCD, 0x40]);

    cpu.step(&mut bus); // INT 0x40 -> task switch
    cpu.step(&mut bus); // HLT in new task

    assert!(cpu.halted());
    assert_eq!(
        cpu.state.tr, PM_TSS2_SEL,
        "Task gate should switch to new TSS"
    );
    assert_eq!(cpu.eax(), 0xBEEF, "Should load EAX from new TSS");
}

/// 32-bit segment offsets work correctly (offsets > 0xFFFF).
#[test]
fn i386_32bit_segment_offset() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    // Widen DS limit to 4GB and set G-bit.
    state.seg_limits[cpu::SegReg32::DS as usize] = 0xFFFF_FFFF;
    state.seg_granularity[cpu::SegReg32::DS as usize] = 0xC0; // G=1, D/B=1
    // Widen CS limit to 4GB and set D-bit.
    state.seg_limits[cpu::SegReg32::CS as usize] = 0xFFFF_FFFF;
    state.seg_granularity[cpu::SegReg32::CS as usize] = 0xC0;
    cpu.load_state(&state);

    // Place a known value at DS:0x10001 (PM_DATA_BASE + 0x10001)
    // This is within the 1MB address space since PM_DATA_BASE=0x10000
    // 0x10000 + 0x10001 = 0x20001 - within our 1MB RAM
    let target_addr = PM_DATA_BASE + 0x10001;
    bus.ram[(target_addr & ADDRESS_MASK) as usize] = 0x42;

    // A0 01 00 01 00 = MOV AL, [0x00010001] (default 32-bit addressing from CS D-bit)
    place_at(&mut bus, PM_CODE_BASE, &[0xA0, 0x01, 0x00, 0x01, 0x00]);

    cpu.step(&mut bus);

    assert_eq!(
        cpu.al(),
        0x42,
        "Should read from 32-bit offset 0x10001 in DS segment"
    );
}

#[test]
fn i386_dbit_32bit_cs_uses_32bit_operand_size() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.seg_limits[cpu::SegReg32::CS as usize] = 0xFFFF_FFFF;
    state.seg_granularity[cpu::SegReg32::CS as usize] = 0x40; // D-bit set
    state.seg_limits[cpu::SegReg32::DS as usize] = 0xFFFF_FFFF;
    cpu.load_state(&state);

    // B8 78 56 34 12 = MOV EAX, 0x12345678 (32-bit immediate due to D-bit)
    place_at(&mut bus, PM_CODE_BASE, &[0xB8, 0x78, 0x56, 0x34, 0x12]);

    cpu.step(&mut bus);

    assert_eq!(
        cpu.eax(),
        0x12345678,
        "32-bit CS D-bit should make MOV EAX use 32-bit immediate"
    );
}

#[test]
fn i386_dbit_16bit_cs_uses_16bit_operand_size() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_protected_mode(&mut bus, 0xFFFF);
    cpu.load_state(&state);

    // B8 34 12 = MOV AX, 0x1234 (16-bit immediate, D-bit clear)
    place_at(&mut bus, PM_CODE_BASE, &[0xB8, 0x34, 0x12]);

    cpu.state.set_eax(0xFFFF_0000);
    cpu.step(&mut bus);

    assert_eq!(
        cpu.eax() & 0xFFFF,
        0x1234,
        "16-bit CS should make MOV AX use 16-bit immediate"
    );
    assert_eq!(
        cpu.eax() & 0xFFFF_0000,
        0xFFFF_0000,
        "Upper 16 bits of EAX should be preserved"
    );
}

#[test]
fn i386_far_call_32bit_pushes_dword_cs_eip() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.seg_limits[cpu::SegReg32::CS as usize] = 0xFFFF_FFFF;
    state.seg_granularity[cpu::SegReg32::CS as usize] = 0x40; // D-bit set
    state.seg_limits[cpu::SegReg32::SS as usize] = 0xFFFF_FFFF;
    state.gdt_limit = 6 * 8 - 1;
    cpu.load_state(&state);

    // Target: same code segment, different offset
    let target_cs_sel: u16 = PM_CS_SEL;
    write_gdt_entry(
        &mut bus,
        PM_GDT_BASE,
        1,
        PM_CODE_BASE,
        0xFFFF_FFFF,
        0x9B,
        0x40,
    );

    // 9A 00 10 00 00 08 00 = CALL FAR 0008:00001000 (32-bit operand)
    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[0x9A, 0x00, 0x10, 0x00, 0x00, 0x08, 0x00],
    );
    bus.ram[(PM_CODE_BASE + 0x1000) as usize] = 0xF4; // HLT at target

    let old_esp = cpu.esp();
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    let new_esp = cpu.esp();
    // 32-bit far call pushes 4 bytes CS + 4 bytes EIP = 8 bytes
    assert_eq!(old_esp - new_esp, 8, "32-bit far CALL should push 8 bytes");
    let pushed_cs = read_dword_at(&bus, PM_STACK_BASE + new_esp + 4);
    let pushed_eip = read_dword_at(&bus, PM_STACK_BASE + new_esp);
    assert_eq!(
        pushed_cs as u16, target_cs_sel,
        "Pushed CS should match old CS"
    );
    assert_eq!(pushed_eip, 7, "Pushed EIP should be return address");
}

#[test]
fn i386_invalidate_segment_checks_rpl() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_with_ring3(&mut bus);
    cpu.load_state(&state);

    // Set DS with RPL=3, DPL=2 data segment
    write_gdt_entry16(&mut bus, PM_GDT_BASE, 8, PM_DATA_BASE, 0xFFFF, 0xD3);
    cpu.state.gdt_limit = 9 * 8 - 1;

    // Load DS with selector RPL=3 pointing to DPL=2 segment
    // (This would normally fail privilege checks, so set up directly)
    let ds_sel: u16 = 0x0043; // index=8, RPL=3
    cpu.state.set_ds(ds_sel);
    cpu.state.seg_bases[cpu::SegReg32::DS as usize] = PM_DATA_BASE;
    cpu.state.seg_limits[cpu::SegReg32::DS as usize] = 0xFFFF;
    cpu.state.seg_rights[cpu::SegReg32::DS as usize] = 0xD3; // DPL=2
    cpu.state.seg_valid[cpu::SegReg32::DS as usize] = true;

    // Return to ring 3 via IRET
    let ring3_ip: u16 = 0x0100;
    bus.ram[(PM_RING3_CODE_BASE + ring3_ip as u32) as usize] = 0xF4;

    let sp = cpu.esp();
    write_word_at(&mut bus, PM_STACK_BASE + sp, ring3_ip);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 2, PM_RING3_CS_SEL);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 4, 0xFF00);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 6, 0xFFF0);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 8, PM_RING3_SS_SEL);

    // IRET
    place_at(&mut bus, PM_CODE_BASE, &[0xCF]);

    cpu.step(&mut bus);

    // DS should be invalidated because DPL(2) < RPL(3)
    assert!(
        !cpu.state.seg_valid[cpu::SegReg32::DS as usize],
        "DS with DPL < RPL should be invalidated on privilege change"
    );
}

#[test]
fn i386_conforming_interrupt_cs_rpl_equals_cpl() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_ring3(&mut bus);
    make_ring3_state(&mut state);
    cpu.load_state(&state);

    // Add conforming code segment at GDT index 8 (DPL=0, conforming)
    write_gdt_entry16(&mut bus, PM_GDT_BASE, 8, PM_CODE_BASE, 0xFFFF, 0x9E);
    cpu.state.gdt_limit = 9 * 8 - 1;

    let handler_ip: u16 = 0x5500;
    let conforming_cs_sel: u16 = 0x0040;
    write_idt_gate(
        &mut bus,
        PM_IDT_BASE,
        0x30,
        handler_ip as u32,
        conforming_cs_sel,
        14,
        3,
    );

    // INT 0x30 from ring 3
    place_at(&mut bus, PM_RING3_CODE_BASE, &[0xCD, 0x30]);

    cpu.step(&mut bus);

    // CS RPL should be CPL (3), not target DPL (0)
    assert_eq!(
        cpu.cs() & 3,
        3,
        "Conforming code interrupt CS RPL should equal CPL, not target DPL"
    );
    assert_eq!(cpu.ip() as u16, handler_ip);
}

#[test]
fn i386_task_gate_pushes_error_code() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_protected_mode_extended(&mut bus);
    cpu.load_state(&state);

    let target_ip: u16 = 0x0400;
    let task_esp: u32 = 0xFFF0;
    write_tss386(
        &mut bus,
        PM_TSS2_BASE,
        0,
        0xFFF0, // esp0
        PM_SS_SEL,
        target_ip as u32,
        0x0002,
        0,
        0,
        0,
        0,
        task_esp, // esp
        0,
        0,
        0,
        PM_DS_SEL,
        PM_CS_SEL,
        PM_SS_SEL,
        PM_DS_SEL,
        0,
        0,
        0,
    );

    bus.ram[(PM_CODE_BASE + target_ip as u32) as usize] = 0xF4;

    // Write IDT task gate at vector 13 (#GP): type=5, DPL=0
    write_idt_gate(&mut bus, PM_IDT_BASE, 13, 0, PM_TSS2_SEL, 5, 0);

    // Trigger #GP by loading an invalid segment
    place_at(&mut bus, PM_CODE_BASE, &[0xB8, 0xFF, 0xFF, 0x8E, 0xD8]);

    cpu.step(&mut bus); // MOV AX, 0xFFFF
    cpu.step(&mut bus); // MOV DS, AX -> #GP -> task gate
    cpu.step(&mut bus); // HLT in new task

    assert!(cpu.halted());
    assert_eq!(cpu.state.tr, PM_TSS2_SEL);

    // Error code should be pushed on new task's stack
    let new_esp = cpu.esp();
    assert_eq!(
        new_esp,
        task_esp.wrapping_sub(4) & 0xFFFF,
        "ESP should be decremented by 4 (error code push)"
    );
    let error_code = read_dword_at(&bus, PM_STACK_BASE + new_esp);
    assert_eq!(
        error_code & 0xFFFC,
        0xFFFC,
        "Error code should contain faulting selector"
    );
}

#[test]
fn i386_sldt_32bit_register_zero_extends() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_protected_mode(&mut bus, 0xFFFF);
    cpu.load_state(&state);

    // Set LDTR to a known value
    cpu.state.ldtr = 0x0018;
    // Set EAX to have garbage in upper 16 bits
    cpu.state.set_eax(0xDEAD_0000);

    // 66 0F 00 C0 = SLDT EAX (with 32-bit operand size prefix)
    place_at(&mut bus, PM_CODE_BASE, &[0x66, 0x0F, 0x00, 0xC0]);

    cpu.step(&mut bus);

    assert_eq!(
        cpu.eax(),
        0x0000_0018,
        "SLDT with 32-bit operand to register should zero-extend"
    );
}

#[test]
fn i386_str_32bit_register_zero_extends() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_protected_mode(&mut bus, 0xFFFF);
    cpu.load_state(&state);

    cpu.state.tr = 0x0020;
    cpu.state.set_eax(0xBEEF_0000);

    // 66 0F 00 C8 = STR EAX (with 32-bit operand size prefix)
    place_at(&mut bus, PM_CODE_BASE, &[0x66, 0x0F, 0x00, 0xC8]);

    cpu.step(&mut bus);

    assert_eq!(
        cpu.eax(),
        0x0000_0020,
        "STR with 32-bit operand to register should zero-extend"
    );
}

#[test]
fn i386_push_seg_32bit_uses_esp_and_writes_4_bytes() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    // Set SS B-bit (use 32-bit ESP)
    state.seg_granularity[cpu::SegReg32::SS as usize] = 0x40;
    state.seg_limits[cpu::SegReg32::SS as usize] = 0xFFFF_FFFF;
    // Set CS D-bit (32-bit operand default)
    state.seg_granularity[cpu::SegReg32::CS as usize] = 0x40;
    state.seg_limits[cpu::SegReg32::CS as usize] = 0xFFFF_FFFF;
    state.set_esp(0x10000);
    cpu.load_state(&state);

    // 1E = PUSH DS (with 32-bit operand size from D-bit)
    place_at(&mut bus, PM_CODE_BASE, &[0x1E]);

    cpu.step(&mut bus);

    let new_esp = cpu.esp();
    assert_eq!(
        new_esp, 0x0FFFC,
        "PUSH DS should decrement ESP by 4 when 32-bit"
    );
    let pushed = read_dword_at(&bus, PM_STACK_BASE + new_esp);
    assert_eq!(
        pushed as u16, PM_DS_SEL,
        "Low 16 bits should be DS selector"
    );
    assert_eq!(pushed >> 16, 0, "High 16 bits should be zero");
}

#[test]
fn i386_pop_seg_32bit_uses_esp() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.seg_granularity[cpu::SegReg32::SS as usize] = 0x40;
    state.seg_limits[cpu::SegReg32::SS as usize] = 0xFFFF_FFFF;
    state.seg_granularity[cpu::SegReg32::CS as usize] = 0x40;
    state.seg_limits[cpu::SegReg32::CS as usize] = 0xFFFF_FFFF;
    state.gdt_limit = 6 * 8 - 1;
    state.set_esp(0x10000);
    cpu.load_state(&state);

    // Write DS selector at ESP
    write_dword_at(&mut bus, PM_STACK_BASE + 0x10000, PM_DS_SEL as u32);
    // Add a second data segment to pop into ES
    write_gdt_entry16(&mut bus, PM_GDT_BASE, 4, PM_DATA_BASE, 0xFFFF, 0x93);

    // 07 = POP ES (with 32-bit operand size from D-bit)
    place_at(&mut bus, PM_CODE_BASE, &[0x07]);

    cpu.step(&mut bus);

    assert_eq!(
        cpu.esp(),
        0x10004,
        "POP ES should increment ESP by 4 when 32-bit"
    );
    assert_eq!(cpu.es(), PM_DS_SEL, "ES should be loaded with DS selector");
}

#[test]
fn i386_iret_cpl3_cannot_set_vm_flag() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_ring3(&mut bus);
    make_ring3_state(&mut state);
    cpu.load_state(&state);

    // Set up stack for 32-bit IRET at ring 3 (same privilege)
    // Push: EIP, CS, EFLAGS (with VM bit 17 set)
    let iret_eip: u32 = 0x0200;
    let iret_eflags: u32 = 0x0002_0202; // VM=1, IF=1, reserved bit 1 set
    let sp = cpu.esp();
    write_dword_at(&mut bus, PM_RING3_STACK_BASE + sp, iret_eip);
    write_dword_at(
        &mut bus,
        PM_RING3_STACK_BASE + sp + 4,
        PM_RING3_CS_SEL as u32,
    );
    write_dword_at(&mut bus, PM_RING3_STACK_BASE + sp + 8, iret_eflags);

    // 66 CF = IRETD (32-bit IRET with operand size prefix)
    place_at(&mut bus, PM_RING3_CODE_BASE, &[0x66, 0xCF]);

    cpu.step(&mut bus);

    assert_eq!(cpu.ip(), iret_eip, "IP should land at iret_eip after IRETD");
    assert_eq!(
        cpu.state.eflags_upper & 0x0002_0000,
        0,
        "IRET at CPL 3 should not be able to set VM flag"
    );
}

#[test]
fn i386_retf_32bit_inter_privilege_correct_state() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_ring3(&mut bus);
    state.seg_granularity[cpu::SegReg32::CS as usize] = 0x40; // D-bit set
    state.seg_limits[cpu::SegReg32::CS as usize] = 0xFFFF_FFFF;
    state.seg_granularity[cpu::SegReg32::SS as usize] = 0x40;
    state.seg_limits[cpu::SegReg32::SS as usize] = 0xFFFF_FFFF;
    cpu.load_state(&state);

    let ring3_ip: u32 = 0x0100;

    // Set up 32-bit stack frame for inter-privilege far return
    let sp = cpu.esp();
    write_dword_at(&mut bus, PM_STACK_BASE + sp, ring3_ip);
    write_dword_at(&mut bus, PM_STACK_BASE + sp + 4, PM_RING3_CS_SEL as u32);
    write_dword_at(&mut bus, PM_STACK_BASE + sp + 8, 0x0000_FF00);
    write_dword_at(&mut bus, PM_STACK_BASE + sp + 12, PM_RING3_SS_SEL as u32);

    // CB = RETF (32-bit due to D-bit)
    place_at(&mut bus, PM_CODE_BASE, &[0xCB]);

    cpu.step(&mut bus);

    assert_eq!(cpu.cs(), PM_RING3_CS_SEL);
    assert_eq!(cpu.ss(), PM_RING3_SS_SEL);
    assert_eq!(cpu.ip(), ring3_ip);
}

#[test]
fn i386_ltr_rejects_undersized_386_tss() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.gdt_limit = 6 * 8 - 1;
    cpu.load_state(&state);

    let tss_base: u32 = 0x60000;
    // 386 TSS (type 9) with limit 50 < 103
    write_gdt_entry16(&mut bus, PM_GDT_BASE, 4, tss_base, 50, 0x89);

    // MOV AX, 0x0020 -> LTR AX
    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[0xB8, 0x20, 0x00, 0x0F, 0x00, 0xD8],
    );

    cpu.step(&mut bus); // MOV AX
    cpu.step(&mut bus); // LTR -> #TS
    cpu.step(&mut bus); // HLT in GP handler

    assert!(cpu.halted());
}

#[test]
fn i386_ltr_rejects_undersized_286_tss() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.gdt_limit = 6 * 8 - 1;
    cpu.load_state(&state);

    let tss_base: u32 = 0x60000;
    // 286 TSS (type 1) with limit 20 < 43
    write_gdt_entry16(&mut bus, PM_GDT_BASE, 4, tss_base, 20, 0x81);

    // MOV AX, 0x0020 -> LTR AX
    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[0xB8, 0x20, 0x00, 0x0F, 0x00, 0xD8],
    );

    cpu.step(&mut bus); // MOV AX
    cpu.step(&mut bus); // LTR -> #TS
    cpu.step(&mut bus); // HLT in GP handler

    assert!(cpu.halted());
}

#[test]
fn i386_ltr_accepts_minimum_size_386_tss() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.gdt_limit = 6 * 8 - 1;
    cpu.load_state(&state);

    let tss_base: u32 = 0x60000;
    // 386 TSS (type 9) with limit exactly 103
    write_gdt_entry16(&mut bus, PM_GDT_BASE, 4, tss_base, 103, 0x89);

    // MOV AX, 0x0020 -> LTR AX
    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[0xB8, 0x20, 0x00, 0x0F, 0x00, 0xD8],
    );

    cpu.step(&mut bus); // MOV AX
    cpu.step(&mut bus); // LTR

    assert_eq!(cpu.state.tr, 0x0020, "LTR should succeed with limit=103");
    let desc_addr = PM_GDT_BASE + 4 * 8;
    let rights = bus.ram[(desc_addr + 5) as usize];
    assert_eq!(
        rights & 0x0F,
        0x0B,
        "TSS should be marked busy after successful LTR"
    );
}

/// Fix 1: SS not-present during inter-privilege interrupt raises #SS (vector 12), not #TS (vector 10).
#[test]
fn i386_interrupt_inter_privilege_ss_not_present_raises_ss() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let mut state = setup_protected_mode_with_ring3(&mut bus);

    // Make GDT entry 3 (ring 0 SS) not-present: rights=0x12 (S=1, writable data, P=0, DPL=0).
    write_gdt_entry16(&mut bus, PM_GDT_BASE, 3, PM_STACK_BASE, 0xFFFF, 0x12);

    // Override #TS and #SS IDT entries to dispatch into ring-3 code (selector 0x0020 = GDT
    // entry 4, DPL=3 code) so no stack switch is needed for the fault handler itself.
    let ts_handler_ip: u16 = 0x0500;
    let ss_handler_ip: u16 = 0x0600;
    write_idt_gate(
        &mut bus,
        PM_IDT_BASE,
        10,
        ts_handler_ip as u32,
        0x0020,
        14,
        0,
    );
    write_idt_gate(
        &mut bus,
        PM_IDT_BASE,
        12,
        ss_handler_ip as u32,
        0x0020,
        14,
        0,
    );
    bus.ram[(PM_RING3_CODE_BASE + ts_handler_ip as u32) as usize] = 0xF4;
    bus.ram[(PM_RING3_CODE_BASE + ss_handler_ip as u32) as usize] = 0xF4;

    make_ring3_state(&mut state);
    state.set_esp(0xFFF0);
    state.flags.expand(0x0202);
    cpu.load_state(&state);

    place_at(&mut bus, PM_RING3_CODE_BASE, &[0x90]);

    write_idt_gate(&mut bus, PM_IDT_BASE, 0x20, 0x0300, PM_CS_SEL, 14, 0);

    bus.irq_vector = 0x20;
    cpu.signal_irq();

    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(
        cpu.ip(),
        ss_handler_ip as u32 + 1,
        "SS not-present during inter-privilege interrupt should dispatch to #SS (vector 12)"
    );
}

/// Fix 2: MOV CR0 masks WP bit (bit 16) on 386.
#[test]
fn i386_mov_cr0_strips_wp_bit() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    // MOV EAX, 0x0001_0001 (PE + WP)
    // MOV CR0, EAX
    place_code(
        &mut bus,
        0xFFFF,
        0x0000,
        &[
            0x66, 0xB8, 0x01, 0x00, 0x01, 0x00, // MOV EAX, 0x00010001
            0x0F, 0x22, 0xC0, // MOV CR0, EAX
        ],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_eq!(
        cpu.cr0, 0x0000_0001,
        "MOV CR0 on 386 should mask WP bit (bit 16)"
    );
}

/// Fix 3: Task switch reads full 20-bit LDT limit with granularity.
#[test]
fn i386_task_switch_reads_full_ldt_limit() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_extended(&mut bus);
    cpu.load_state(&state);

    // Use GDT entry 8 (currently empty) as the LDT descriptor for the new task.
    // Write a 32-bit GDT entry with 20-bit limit: base=0x30000, limit=0x1_FFFF, rights=0x82 (LDT).
    let ldt_sel: u16 = 0x0040; // GDT entry 8
    write_gdt_entry(&mut bus, PM_GDT_BASE, 8, 0x30000, 0x1_FFFF, 0x82, 0x00);

    let task2_ip: u16 = 0x0300;
    write_tss386(
        &mut bus,
        PM_TSS2_BASE,
        0,
        0xFFF0,
        PM_SS_SEL,
        task2_ip as u32,
        0x0002,
        0,
        0,
        0,
        0,
        0xFF00,
        0,
        0,
        0,
        PM_DS_SEL,
        PM_CS_SEL,
        PM_SS_SEL,
        PM_DS_SEL,
        0,
        0,
        ldt_sel,
    );

    bus.ram[(PM_CODE_BASE + task2_ip as u32) as usize] = 0xF4;

    // JMP TSS2 (far JMP to TSS2 selector 0x0048)
    place_at(&mut bus, PM_CODE_BASE, &[0xEA, 0x00, 0x00, 0x48, 0x00]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(
        cpu.state.ldtr_limit, 0x1_FFFF,
        "Task switch should read full 20-bit LDT limit"
    );
}

/// Fix 3b: Task switch reads LDT limit with granularity bit set.
#[test]
fn i386_task_switch_reads_ldt_limit_with_granularity() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_extended(&mut bus);
    cpu.load_state(&state);

    // GDT entry 8 as LDT descriptor: base=0x30000, limit=0x0_0003, rights=0x82, G=1.
    // With G=1: effective limit = (0x0003 << 12) | 0xFFF = 0x3FFF.
    let ldt_sel: u16 = 0x0040;
    write_gdt_entry(&mut bus, PM_GDT_BASE, 8, 0x30000, 0x0003, 0x82, 0x80);

    let task2_ip: u16 = 0x0300;
    write_tss386(
        &mut bus,
        PM_TSS2_BASE,
        0,
        0xFFF0,
        PM_SS_SEL,
        task2_ip as u32,
        0x0002,
        0,
        0,
        0,
        0,
        0xFF00,
        0,
        0,
        0,
        PM_DS_SEL,
        PM_CS_SEL,
        PM_SS_SEL,
        PM_DS_SEL,
        0,
        0,
        ldt_sel,
    );

    bus.ram[(PM_CODE_BASE + task2_ip as u32) as usize] = 0xF4;

    place_at(&mut bus, PM_CODE_BASE, &[0xEA, 0x00, 0x00, 0x48, 0x00]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(
        cpu.state.ldtr_limit, 0x3FFF,
        "Task switch should apply granularity to LDT limit"
    );
}

/// Helper: create a real-mode I386State with stale ip_upper = 0x0001_0000.
/// CS = 0x0000 (base 0x00000), SS = 0x0000 (base 0x00000), SP = 0xFFF0.
/// Code must be placed at offset 0x10000 in the bus (cs_base + ip_upper + ip).
fn real_mode_stale_ip_upper() -> cpu::I386State {
    let mut state = cpu::I386State {
        cr0: 0,
        ip: 0x0000,
        ip_upper: 0x0001_0000,
        ..Default::default()
    };

    state.set_cs(0x0000);
    state.seg_bases[cpu::SegReg32::CS as usize] = 0x00000;
    // "Big real mode": the test exercises a stale ip_upper that puts the
    // effective EIP at 0x10000, which a strict real-mode CS limit (0xFFFF)
    // would reject during fetch. Model the CS descriptor cache having
    // retained a larger limit from a prior protected-mode load so the
    // fetch succeeds and the JMP/CALL/RET ip_upper clearing can be tested.
    state.seg_limits[cpu::SegReg32::CS as usize] = 0xFFFF_FFFF;
    state.seg_rights[cpu::SegReg32::CS as usize] = 0x9B;

    state.set_ss(0x0000);
    state.seg_bases[cpu::SegReg32::SS as usize] = 0x00000;
    state.seg_limits[cpu::SegReg32::SS as usize] = 0xFFFF;
    state.seg_rights[cpu::SegReg32::SS as usize] = 0x93;

    state.set_ds(0x0000);
    state.seg_bases[cpu::SegReg32::DS as usize] = 0x00000;
    state.seg_limits[cpu::SegReg32::DS as usize] = 0xFFFF;
    state.seg_rights[cpu::SegReg32::DS as usize] = 0x93;

    state.set_es(0x0000);
    state.seg_bases[cpu::SegReg32::ES as usize] = 0x00000;
    state.seg_limits[cpu::SegReg32::ES as usize] = 0xFFFF;
    state.seg_rights[cpu::SegReg32::ES as usize] = 0x93;

    state.set_esp(0xFFF0);
    state.seg_valid = [true, true, true, true, false, false];
    state
}

/// JMP SHORT (EB xx): 16-bit short jump must clear ip_upper.
#[test]
fn i386_jmp_short_clears_ip_upper() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    cpu.load_state(&real_mode_stale_ip_upper());

    // EB 10 = JMP SHORT +16. After consuming 2 bytes, IP=0x0002, then IP += 0x10 = 0x0012.
    place_at(&mut bus, 0x10000, &[0xEB, 0x10]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.ip_upper, 0, "JMP SHORT must clear ip_upper");
    assert_eq!(cpu.state.ip, 0x0012);
}

/// JMP NEAR rel16 (E9 xx xx): 16-bit near jump must clear ip_upper.
#[test]
fn i386_jmp_near_rel16_clears_ip_upper() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    cpu.load_state(&real_mode_stale_ip_upper());

    // E9 FE 00 = JMP NEAR +0x00FE. After 3 bytes, IP=0x0003, then IP += 0xFE = 0x0101.
    place_at(&mut bus, 0x10000, &[0xE9, 0xFE, 0x00]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.ip_upper, 0, "JMP NEAR rel16 must clear ip_upper");
    assert_eq!(cpu.state.ip, 0x0101);
}

/// JMP FAR ptr16:16 (EA xx xx xx xx): 16-bit far jump must clear ip_upper.
#[test]
fn i386_jmp_far_16bit_clears_ip_upper() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    cpu.load_state(&real_mode_stale_ip_upper());

    // EA 00 02 00 10 = JMP FAR 0x1000:0x0200
    place_at(&mut bus, 0x10000, &[0xEA, 0x00, 0x02, 0x00, 0x10]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.ip_upper, 0, "JMP FAR 16-bit must clear ip_upper");
    assert_eq!(cpu.state.ip, 0x0200);
    assert_eq!(cpu.cs(), 0x1000);
}

/// CALL NEAR rel16 (E8 xx xx): 16-bit near call must clear ip_upper.
#[test]
fn i386_call_near_rel16_clears_ip_upper() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    cpu.load_state(&real_mode_stale_ip_upper());

    // E8 FD 00 = CALL NEAR +0x00FD. After 3 bytes, IP=0x0003, then IP += 0xFD = 0x0100.
    place_at(&mut bus, 0x10000, &[0xE8, 0xFD, 0x00]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.ip_upper, 0, "CALL NEAR rel16 must clear ip_upper");
    assert_eq!(cpu.state.ip, 0x0100);
}

/// CALL FAR ptr16:16 (9A xx xx xx xx): 16-bit far call must clear ip_upper.
#[test]
fn i386_call_far_16bit_clears_ip_upper() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    cpu.load_state(&real_mode_stale_ip_upper());

    // 9A 00 05 00 20 = CALL FAR 0x2000:0x0500
    place_at(&mut bus, 0x10000, &[0x9A, 0x00, 0x05, 0x00, 0x20]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.ip_upper, 0, "CALL FAR 16-bit must clear ip_upper");
    assert_eq!(cpu.state.ip, 0x0500);
    assert_eq!(cpu.cs(), 0x2000);
}

/// RET NEAR (C3): 16-bit near return must clear ip_upper.
#[test]
fn i386_ret_near_clears_ip_upper() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    cpu.load_state(&real_mode_stale_ip_upper());

    // Push return address 0x1234 onto stack.
    let sp = cpu.esp();
    write_word_at(&mut bus, sp - 2, 0x1234);
    cpu.state.set_esp(sp - 2);

    // C3 = RET NEAR
    place_at(&mut bus, 0x10000, &[0xC3]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.ip_upper, 0, "RET NEAR must clear ip_upper");
    assert_eq!(cpu.state.ip, 0x1234);
}

/// RET NEAR imm16 (C2 xx xx): 16-bit near return with immediate must clear ip_upper.
#[test]
fn i386_ret_near_imm_clears_ip_upper() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    cpu.load_state(&real_mode_stale_ip_upper());

    // Push return address 0xABCD onto stack.
    let sp = cpu.esp();
    write_word_at(&mut bus, sp - 2, 0xABCD);
    cpu.state.set_esp(sp - 2);

    // C2 04 00 = RET NEAR 4
    place_at(&mut bus, 0x10000, &[0xC2, 0x04, 0x00]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.ip_upper, 0, "RET NEAR imm16 must clear ip_upper");
    assert_eq!(cpu.state.ip, 0xABCD);
}

/// RET FAR (CB): 16-bit far return must clear ip_upper.
#[test]
fn i386_ret_far_16bit_clears_ip_upper() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    cpu.load_state(&real_mode_stale_ip_upper());

    // Push CS:IP (0x3000:0x0400) onto stack.
    let sp = cpu.esp();
    write_word_at(&mut bus, sp - 4, 0x0400); // IP
    write_word_at(&mut bus, sp - 2, 0x3000); // CS
    cpu.state.set_esp(sp - 4);

    // CB = RET FAR
    place_at(&mut bus, 0x10000, &[0xCB]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.ip_upper, 0, "RET FAR 16-bit must clear ip_upper");
    assert_eq!(cpu.state.ip, 0x0400);
    assert_eq!(cpu.cs(), 0x3000);
}

/// RET FAR imm16 (CA xx xx): 16-bit far return with immediate must clear ip_upper.
#[test]
fn i386_ret_far_imm_16bit_clears_ip_upper() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    cpu.load_state(&real_mode_stale_ip_upper());

    // Push CS:IP (0x4000:0x0600) onto stack.
    let sp = cpu.esp();
    write_word_at(&mut bus, sp - 4, 0x0600); // IP
    write_word_at(&mut bus, sp - 2, 0x4000); // CS
    cpu.state.set_esp(sp - 4);

    // CA 06 00 = RET FAR 6
    place_at(&mut bus, 0x10000, &[0xCA, 0x06, 0x00]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.ip_upper, 0, "RET FAR imm16 must clear ip_upper");
    assert_eq!(cpu.state.ip, 0x0600);
    assert_eq!(cpu.cs(), 0x4000);
}

/// IRET (CF): 16-bit IRET in real mode must clear ip_upper.
#[test]
fn i386_iret_16bit_real_mode_clears_ip_upper() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    cpu.load_state(&real_mode_stale_ip_upper());

    // Push FLAGS, CS, IP onto stack (IRET pops IP, CS, FLAGS).
    let sp = cpu.esp();
    write_word_at(&mut bus, sp - 6, 0x0800); // IP
    write_word_at(&mut bus, sp - 4, 0x5000); // CS
    write_word_at(&mut bus, sp - 2, 0x0202); // FLAGS (IF=1)
    cpu.state.set_esp(sp - 6);

    // CF = IRET
    place_at(&mut bus, 0x10000, &[0xCF]);

    cpu.step(&mut bus);

    assert_eq!(
        cpu.state.ip_upper, 0,
        "IRET 16-bit real mode must clear ip_upper"
    );
    assert_eq!(cpu.state.ip, 0x0800);
    assert_eq!(cpu.cs(), 0x5000);
}

/// Jcc short (e.g. JE, 74 xx): conditional short jump must clear ip_upper when taken.
#[test]
fn i386_jcc_short_clears_ip_upper() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    cpu.load_state(&real_mode_stale_ip_upper());
    cpu.state.flags.zero_val = 0; // ZF=1 so JE is taken.

    // 74 20 = JE SHORT +32. After consuming 2 bytes, IP=0x0002, then IP += 0x20 = 0x0022.
    place_at(&mut bus, 0x10000, &[0x74, 0x20]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.ip_upper, 0, "Jcc short must clear ip_upper");
    assert_eq!(cpu.state.ip, 0x0022);
}

/// Jcc near (0F 84 xx xx): conditional near jump must clear ip_upper when taken.
#[test]
fn i386_jcc_near_16bit_clears_ip_upper() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    cpu.load_state(&real_mode_stale_ip_upper());
    cpu.state.flags.zero_val = 0; // ZF=1 so JE is taken.

    // 0F 84 FC 00 = JE NEAR +0x00FC. After 4 bytes, IP=0x0004, then IP += 0xFC = 0x0100.
    place_at(&mut bus, 0x10000, &[0x0F, 0x84, 0xFC, 0x00]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.ip_upper, 0, "Jcc near 16-bit must clear ip_upper");
    assert_eq!(cpu.state.ip, 0x0100);
}

/// LOOP (E2 xx): loop instruction must clear ip_upper when taken.
#[test]
fn i386_loop_clears_ip_upper() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    cpu.load_state(&real_mode_stale_ip_upper());
    cpu.state.set_ecx(5); // CX=5, will decrement to 4 (nonzero -> loop taken).

    // E2 10 = LOOP +16. After 2 bytes, IP=0x0002, then IP += 0x10 = 0x0012.
    place_at(&mut bus, 0x10000, &[0xE2, 0x10]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.ip_upper, 0, "LOOP must clear ip_upper");
    assert_eq!(cpu.state.ip, 0x0012);
}

/// JCXZ (E3 xx): jump-if-CX-zero must clear ip_upper when taken.
#[test]
fn i386_jcxz_clears_ip_upper() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    cpu.load_state(&real_mode_stale_ip_upper());
    cpu.state.set_ecx(0); // CX=0 -> jump taken.

    // E3 10 = JCXZ +16. After 2 bytes, IP=0x0002, then IP += 0x10 = 0x0012.
    place_at(&mut bus, 0x10000, &[0xE3, 0x10]);

    cpu.step(&mut bus);

    assert_eq!(cpu.state.ip_upper, 0, "JCXZ must clear ip_upper");
    assert_eq!(cpu.state.ip, 0x0012);
}

/// End-to-end: enter PM, execute in PM (ip_upper set), exit to real mode via
/// MOV CR0 + JMP FAR, verify the game can continue executing at correct addresses.
#[test]
fn i386_pm_to_real_mode_jmp_far_clears_ip_upper() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    // Set up a minimal GDT for PM entry.
    let gdt_base: u32 = 0x80000;
    let code_base: u32 = 0x50000;
    write_gdt_entry16(&mut bus, gdt_base, 0, 0, 0, 0); // Null.
    // CS uses limit 0xFFFFF with G=1 so the test's 32-bit JMP to EIP
    // = 0x10006 stays within the segment limit; otherwise the fetch-side
    // CS-limit check (added per 80486 PRM Table 22-2) would #GP before
    // we get to test the CR0.PE clear and subsequent JMP FAR.
    write_gdt_entry(&mut bus, gdt_base, 1, code_base, 0x000F_FFFF, 0x9B, 0x80);
    write_gdt_entry16(&mut bus, gdt_base, 2, 0x00000, 0xFFFF, 0x93); // DS/SS.

    // Start in PM at code_base:0x0000.
    let mut state = cpu::I386State {
        cr0: 0x0001,
        ip: 0x0000,
        ip_upper: 0,
        ..Default::default()
    };
    state.set_cs(0x0008);
    state.seg_bases[cpu::SegReg32::CS as usize] = code_base;
    state.seg_limits[cpu::SegReg32::CS as usize] = 0xFFFF_FFFF;
    state.seg_rights[cpu::SegReg32::CS as usize] = 0x9B;
    state.seg_granularity[cpu::SegReg32::CS as usize] = 0x80;
    state.set_ss(0x0010);
    state.seg_bases[cpu::SegReg32::SS as usize] = 0x00000;
    state.seg_limits[cpu::SegReg32::SS as usize] = 0xFFFF;
    state.seg_rights[cpu::SegReg32::SS as usize] = 0x93;
    state.set_ds(0x0010);
    state.seg_bases[cpu::SegReg32::DS as usize] = 0x00000;
    state.seg_limits[cpu::SegReg32::DS as usize] = 0xFFFF;
    state.seg_rights[cpu::SegReg32::DS as usize] = 0x93;
    state.set_esp(0xFFF0);
    state.gdt_base = gdt_base;
    state.gdt_limit = 3 * 8 - 1;
    state.seg_valid = [true, true, true, true, false, false];
    cpu.load_state(&state);

    // In PM, jump to a high EIP to set ip_upper.
    // 66 E9 xx xx xx xx = JMP NEAR rel32 (32-bit operand override in 16-bit code).
    // We jump forward by 0x10000 bytes to set ip_upper to 0x0001.
    // After 6 instruction bytes: EIP = 0x0006 + 0x10000 = 0x10006.
    place_at(&mut bus, code_base, &[0x66, 0xE9, 0x00, 0x00, 0x01, 0x00]);

    cpu.step(&mut bus);
    assert_eq!(
        cpu.state.ip_upper, 0x0001_0000,
        "After 32-bit near jump, ip_upper should be 0x10000"
    );
    assert_eq!(cpu.state.ip, 0x0006);

    // Now at code_base + 0x10006. Clear CR0.PE to return to real mode, then JMP FAR.
    // 66 B8 00 00 00 00 = MOV EAX, 0  (use 32-bit immediate to clear all bits)
    // 0F 22 C0          = MOV CR0, EAX
    // EA 00 01 00 10    = JMP FAR 0x1000:0x0100
    // Place all instructions at once so the prefetch cache picks up the correct bytes.
    place_at(
        &mut bus,
        code_base + 0x10006,
        &[
            0x66, 0xB8, 0x00, 0x00, 0x00, 0x00, // MOV EAX, 0
            0x0F, 0x22, 0xC0, // MOV CR0, EAX
            0xEA, 0x00, 0x01, 0x00, 0x10, // JMP FAR 0x1000:0x0100
        ],
    );

    cpu.step(&mut bus); // MOV EAX, 0
    cpu.step(&mut bus); // MOV CR0, EAX
    assert_eq!(cpu.cr0 & 1, 0, "PE should be cleared");
    assert_eq!(
        cpu.state.ip_upper, 0x0001_0000,
        "ip_upper still stale after MOV CR0"
    );

    // Place HLT at the correct destination (0x1000*16 + 0x0100 = 0x10100).
    bus.ram[0x10100] = 0xF4;

    cpu.step(&mut bus); // JMP FAR
    assert_eq!(
        cpu.state.ip_upper, 0,
        "JMP FAR in real mode must clear ip_upper"
    );
    assert_eq!(cpu.cs(), 0x1000);
    assert_eq!(cpu.state.ip, 0x0100);

    cpu.step(&mut bus); // Should execute HLT at 0x10100.
    assert!(
        cpu.halted(),
        "CPU should halt at correct address after ip_upper cleared"
    );
}

const VM86_GDT_BASE: u32 = 0x80000;
const VM86_IDT_BASE: u32 = 0x90000;
const VM86_TSS_BASE: u32 = 0xC0000;
const VM86_CODE_BASE: u32 = 0x50000;
const VM86_HANDLER_IP: u16 = 0x0100;

// GDT entry indices for VM86 tests.
const VM86_CS_SEL: u16 = 0x0008; // CPL0 code
const VM86_SS0_SEL: u16 = 0x0018; // CPL0 32-bit stack (B=1)

/// Sets up the minimal VM86 environment used by Tests 1–3.
/// Returns a state with CR0.PE=1, EFLAGS.VM=1, IOPL=3, VM86 segments.
fn setup_vm86(bus: &mut TestBus) -> cpu::I386State {
    // GDT: null | CPL0 code | data | CPL0 32-bit stack | TSS
    write_gdt_entry16(bus, VM86_GDT_BASE, 0, 0, 0, 0);
    write_gdt_entry16(bus, VM86_GDT_BASE, 1, VM86_CODE_BASE, 0xFFFF, 0x9B); // code, DPL=0
    write_gdt_entry16(bus, VM86_GDT_BASE, 2, 0x00000, 0xFFFF, 0x93); // data, DPL=0
    // 32-bit stack: granularity byte = 0x40 sets D/B=1 -> use_esp() = true
    write_gdt_entry(bus, VM86_GDT_BASE, 3, 0x00000, 0xFFFF, 0x93, 0x40); // 32-bit stack
    // 386 TSS (type 9, present, DPL=0): rights=0x89
    write_gdt_entry16(bus, VM86_GDT_BASE, 4, VM86_TSS_BASE, 0x0A, 0x89); // TSS

    // IDT: vector 0x42 -> 386 interrupt gate, DPL=3, targets CPL0 handler
    write_idt_gate(
        bus,
        VM86_IDT_BASE,
        0x42,
        VM86_HANDLER_IP as u32,
        VM86_CS_SEL,
        14,
        3,
    );
    // Handler: HLT
    bus.ram[(VM86_CODE_BASE + VM86_HANDLER_IP as u32) as usize] = 0xF4;

    // TSS: ESP0=0x1000, SS0=VM86_SS0_SEL
    write_dword_at(bus, VM86_TSS_BASE + 0x04, 0x1000);
    write_word_at(bus, VM86_TSS_BASE + 0x08, VM86_SS0_SEL);

    let mut state = cpu::I386State {
        cr0: 0x0001,               // PE=1
        eflags_upper: 0x0002_0000, // VM=1 (bit 17)
        seg_valid: [true; 6],
        gdt_base: VM86_GDT_BASE,
        gdt_limit: 5 * 8 - 1,
        idt_base: VM86_IDT_BASE,
        idt_limit: 256 * 8 - 1,
        tr: 0x0020,
        tr_base: VM86_TSS_BASE,
        tr_limit: 0x0A, // covers ESP0 (offset 4-7) and SS0 (offset 8-9)
        tr_rights: 0x89,
        ..cpu::I386State::default()
    };
    state.flags.iopl = 3; // IOPL=3: INT goes through IDT without #GP

    // VM86 code: CS=0x1000, IP=0x0000 (physical 0x10000)
    state.set_cs(0x1000);
    state.seg_bases[cpu::SegReg32::CS as usize] = 0x10000;
    state.seg_limits[cpu::SegReg32::CS as usize] = 0xFFFF;
    state.seg_rights[cpu::SegReg32::CS as usize] = 0x9B;

    // VM86 stack: SS=0x2000, SP=0xF000 (physical 0x2F000)
    state.set_ss(0x2000);
    state.set_esp(0xF000);
    state.seg_bases[cpu::SegReg32::SS as usize] = 0x20000;
    state.seg_limits[cpu::SegReg32::SS as usize] = 0xFFFF;
    state.seg_rights[cpu::SegReg32::SS as usize] = 0x93;

    // VM86 data segments
    state.set_es(0x3000);
    state.seg_bases[cpu::SegReg32::ES as usize] = 0x30000;
    state.seg_limits[cpu::SegReg32::ES as usize] = 0xFFFF;
    state.seg_rights[cpu::SegReg32::ES as usize] = 0x93;

    state.set_ds(0x4000);
    state.seg_bases[cpu::SegReg32::DS as usize] = 0x40000;
    state.seg_limits[cpu::SegReg32::DS as usize] = 0xFFFF;
    state.seg_rights[cpu::SegReg32::DS as usize] = 0x93;

    state.set_fs(0x5000);
    state.seg_bases[cpu::SegReg32::FS as usize] = 0x50000;
    state.seg_limits[cpu::SegReg32::FS as usize] = 0xFFFF;
    state.seg_rights[cpu::SegReg32::FS as usize] = 0x93;

    state.set_gs(0x6000);
    state.seg_bases[cpu::SegReg32::GS as usize] = 0x60000;
    state.seg_limits[cpu::SegReg32::GS as usize] = 0xFFFF;
    state.seg_rights[cpu::SegReg32::GS as usize] = 0x93;

    state
}

/// VM flag must be cleared before dword pushes in interrupt dispatch so
/// that push_dword uses 32-bit ESP (B=1 in the PL0 SS descriptor) rather than
/// the 16-bit SP.
#[test]
fn vm86_interrupt_dispatch_uses_pl0_stack_correctly() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_vm86(&mut bus);
    cpu.load_state(&state);

    // INT 0x42 at VM86 code base (CS=0x1000 -> physical 0x10000)
    bus.ram[0x10000] = 0xCD; // INT imm8
    bus.ram[0x10001] = 0x42; // vector 0x42

    cpu.step(&mut bus);

    // After the dispatch, CS should be the CPL0 handler's selector and IP the handler offset.
    assert_eq!(
        cpu.cs(),
        VM86_CS_SEL,
        "CS should switch to CPL0 code selector"
    );
    assert_eq!(
        cpu.ip() as u16,
        VM86_HANDLER_IP,
        "IP should jump to IDT handler offset"
    );

    // PL0 SS has base=0 and B=1 (32-bit). ESP0=0x1000; after 9 dword pushes: ESP=0x1000-36=0xFDC.
    let esp = cpu.esp();
    assert_eq!(
        esp, 0x0000_0FDC,
        "ESP should be decremented by 36 (9 dwords) using 32-bit ESP"
    );

    // Verify the 9-dword VM86 frame on the PL0 stack (SS0 base = 0).
    // Pushes (highest to lowest): GS, FS, DS, ES, SS, SP, EFLAGS, CS, EIP
    assert_eq!(
        read_dword_at(&bus, 0x0FFC),
        0x6000,
        "GS pushed first (highest)"
    );
    assert_eq!(read_dword_at(&bus, 0x0FF8), 0x5000, "FS");
    assert_eq!(read_dword_at(&bus, 0x0FF4), 0x4000, "DS");
    assert_eq!(read_dword_at(&bus, 0x0FF0), 0x3000, "ES");
    assert_eq!(read_dword_at(&bus, 0x0FEC), 0x2000, "old SS");
    assert_eq!(
        read_dword_at(&bus, 0x0FE8),
        0xF000,
        "old SP (VM86 stack pointer)"
    );
    // EFLAGS: VM=1, IOPL=3 (bits 12-13 = 0x3000)
    let pushed_eflags = read_dword_at(&bus, 0x0FE4);
    assert_ne!(
        pushed_eflags & 0x0002_0000,
        0,
        "VM bit must be set in pushed EFLAGS"
    );
    assert_eq!(
        (pushed_eflags >> 12) & 3,
        3,
        "IOPL=3 must be preserved in pushed EFLAGS"
    );
    assert_eq!(read_dword_at(&bus, 0x0FE0), 0x1000, "old CS");
    assert_eq!(
        read_dword_at(&bus, 0x0FDC),
        0x0002,
        "return EIP = IP after INT instruction (2 bytes)"
    );
}

/// When TSS ESP0 exceeds 0xFFFF, the interrupt dispatch from VM86 must use
/// full 32-bit ESP (not truncated 16-bit SP). The VM flag must be cleared
/// before the ESP assignment so that `use_esp()` consults the B bit of the
/// new SS descriptor rather than unconditionally returning false for VM86.
#[test]
fn vm86_interrupt_dispatch_esp0_above_64k() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_vm86(&mut bus);

    // Override the ring 0 stack descriptor (GDT entry 3) to be a 4GB flat
    // segment (G=1, B=1, limit=0xFFFFF) so ESP0 values above 0xFFFF are valid.
    write_gdt_entry(
        &mut bus,
        VM86_GDT_BASE,
        3,
        0x00000,
        0xFFFFF,
        0x93,
        0xC0, // G=1, B=1
    );

    // Set TSS ESP0 to a value above 0xFFFF.
    let esp0: u32 = 0x1_E000;
    write_dword_at(&mut bus, VM86_TSS_BASE + 0x04, esp0);

    cpu.load_state(&state);

    // INT 0x42 at VM86 code base (CS=0x1000 -> physical 0x10000)
    bus.ram[0x10000] = 0xCD; // INT imm8
    bus.ram[0x10001] = 0x42; // vector 0x42

    cpu.step(&mut bus);

    // After dispatch the handler should be running at CPL0.
    assert_eq!(cpu.cs(), VM86_CS_SEL);
    assert_eq!(cpu.ip() as u16, VM86_HANDLER_IP);

    // ESP must be ESP0 - 36 (9 dword pushes) using FULL 32-bit arithmetic.
    let expected_esp = esp0 - 36;
    let actual_esp = cpu.esp();
    assert_eq!(
        actual_esp,
        expected_esp,
        "ESP should be {expected_esp:#X} (ESP0={esp0:#X} - 36), got {actual_esp:#X}. \
         If ESP is {:#X}, the VM flag was not cleared before the ESP assignment, \
         causing 16-bit SP truncation.",
        (esp0 as u16).wrapping_sub(36) as u32
    );

    // Verify EFLAGS on the PL0 stack has VM=1 at the correct (32-bit) address.
    // SS0 base = 0 (GDT entry 3), so physical address = ESP0 - 28.
    let eflags_addr = esp0 - 28;
    let pushed_eflags = read_dword_at(&bus, eflags_addr);
    assert_ne!(
        pushed_eflags & 0x0002_0000,
        0,
        "VM bit must be set in pushed EFLAGS at {eflags_addr:#X}"
    );
    assert_eq!(
        (pushed_eflags >> 12) & 3,
        3,
        "IOPL=3 must be preserved in pushed EFLAGS"
    );

    // Verify the rest of the V86 frame is at the correct addresses.
    assert_eq!(read_dword_at(&bus, esp0 - 4), 0x6000, "GS");
    assert_eq!(read_dword_at(&bus, esp0 - 8), 0x5000, "FS");
    assert_eq!(read_dword_at(&bus, esp0 - 12), 0x4000, "DS");
    assert_eq!(read_dword_at(&bus, esp0 - 16), 0x3000, "ES");
    assert_eq!(read_dword_at(&bus, esp0 - 20), 0x2000, "old SS");
    assert_eq!(read_dword_at(&bus, esp0 - 24), 0xF000, "old SP");
    assert_eq!(read_dword_at(&bus, esp0 - 32), 0x1000, "old CS");
    assert_eq!(read_dword_at(&bus, esp0 - 36), 0x0002, "return EIP");
}

/// IRET in VM86 must not allow the VM86 task to raise its own IOPL.
/// expand() sets IOPL unconditionally; load_flags(..., 3, true) preserves it.
#[test]
fn vm86_iret_cannot_change_iopl() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_vm86(&mut bus);
    state.flags.iopl = 0; // Start with IOPL=0

    // VM86 stack: push 16-bit IRET frame with FLAGS that has IOPL=3 (bits 12-13 set).
    // Frame (16-bit IRET in VM86, IOPL=3 path): IP, CS, FLAGS
    // The IRET short path (IOPL=3 check) will pop: IP, CS, FLAGS
    let vm86_stack_base: u32 = 0x20000; // SS=0x2000 -> base 0x20000
    let sp: u16 = 0xF000;
    // Push FLAGS with IOPL=3 (bits 12-13 = 0x3000) at stack top
    let new_flags: u16 = 0x3202; // IOPL=3, IF=1, bit1=1
    write_word_at(&mut bus, vm86_stack_base + sp as u32 - 6, 0x0010); // new IP
    write_word_at(&mut bus, vm86_stack_base + sp as u32 - 4, 0x1000); // new CS
    write_word_at(&mut bus, vm86_stack_base + sp as u32 - 2, new_flags); // FLAGS with IOPL=3
    state.set_esp(sp as u32 - 6);

    cpu.load_state(&state);

    // IRET (0xCF) in VM86 with IOPL=3: uses the short path, pops IP/CS/FLAGS
    place_code(&mut bus, 0x1000, 0x0000, &[0xCF]);

    cpu.step(&mut bus);

    assert_eq!(
        cpu.flags.iopl, 0,
        "IOPL must not change: VM86 task cannot escalate IOPL via IRET"
    );
}

/// 32-bit IRETD in VM86 must clear ip_upper. EIP is a 16-bit value in V86
/// mode and the popped upper 16 bits are discarded so subsequent fetches do
/// not exceed the implicit 64KB CS limit.
#[test]
fn vm86_iretd_clears_ip_upper() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_vm86(&mut bus);
    cpu.load_state(&state);

    let new_eip: u32 = 0xDEAD_0010;
    let new_cs: u32 = 0x1000;
    let new_eflags: u32 = 0x0002_3202; // VM=1, IOPL=3, IF=1, bit1

    let vm86_stack_base: u32 = 0x20000;
    let sp: u32 = 0xF000;
    write_dword_at(&mut bus, vm86_stack_base + sp - 12, new_eip);
    write_dword_at(&mut bus, vm86_stack_base + sp - 8, new_cs);
    write_dword_at(&mut bus, vm86_stack_base + sp - 4, new_eflags);
    let mut state = setup_vm86(&mut bus);
    state.set_esp(sp - 12);
    cpu.load_state(&state);

    place_code(&mut bus, 0x1000, 0x0000, &[0x66, 0xCF]);

    cpu.step(&mut bus);

    assert_eq!(
        cpu.ip() as u16,
        0x0010,
        "IP should be low 16 bits of new_eip"
    );
    assert_eq!(
        cpu.ip_upper, 0,
        "ip_upper must be zero in VM86 regardless of the popped EIP upper bits"
    );
}

/// IOPB boundary check must use >= not > so that a port whose bitmap byte
/// falls exactly at tr_limit raises #GP rather than reading a stale/sentinel byte.
#[test]
fn iopb_boundary_raises_gp_at_limit() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    // Use setup_protected_mode as a base (CPL=0 currently; change to CPL=3).
    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    // Force CPL=3 by setting RPL bits in CS selector.
    state.set_cs(PM_CS_SEL | 3);
    state.flags.iopl = 0; // IOPL=0 < CPL=3 -> IOPB will be consulted

    // Add TSS to GDT (entry 5, selector 0x28), extend GDT limit.
    const TSS_BASE: u32 = 0xC0000;
    const TSS_SEL: u16 = 0x0028;
    // 386 TSS (type=9, present, DPL=0): limit=0x69 covers IOPB byte at offset 0x68.
    write_gdt_entry16(&mut bus, PM_GDT_BASE, 5, TSS_BASE, 0x69, 0x89);
    state.gdt_limit = 6 * 8 - 1;

    // TSS: provide ESP0/SS0 for the CPL3->CPL0 #GP dispatch (offset 4=ESP0, 8=SS0).
    write_dword_at(&mut bus, TSS_BASE + 0x04, 0xFF00); // ESP0: PL0 stack at top of PM_STACK
    write_word_at(&mut bus, TSS_BASE + 0x08, PM_SS_SEL); // SS0: existing CPL0 stack segment
    // IOPB pointer at offset 0x66 = 0x0068 (IOPB starts at TSS+0x68).
    write_word_at(&mut bus, TSS_BASE + 0x66, 0x0068);
    // IOPB byte for ports 0-7: all bits 0 -> ports 0-7 allowed.
    bus.ram[(TSS_BASE + 0x68) as usize] = 0x00;
    // No sentinel byte (TSS+0x69 is exactly at tr_limit=0x69 -> boundary check triggers).

    state.tr = TSS_SEL;
    state.tr_base = TSS_BASE;
    state.tr_limit = 0x69; // byte_idx for port 8 = 0x68+1 = 0x69 ≥ 0x69 -> #GP
    state.tr_rights = 0x89;
    cpu.load_state(&state);

    // IN AL, 0 (port 0): byte_idx = 0x68 < 0x69 -> reads IOPB, bit 0 = 0 -> allowed.
    // IN AL, 8 (port 8): byte_idx = 0x69 ≥ 0x69 -> must raise #GP.
    place_at(&mut bus, PM_CODE_BASE, &[0xE4, 0x00, 0xE4, 0x08]);

    cpu.step(&mut bus); // IN AL, 0 -> should succeed
    assert_eq!(
        cpu.al(),
        0xFF,
        "IN AL, 0 should succeed: port 0 is in bitmap and allowed"
    );
    assert_eq!(cpu.ip() as u16, 2, "IP should advance past IN AL, 0");

    cpu.step(&mut bus); // IN AL, 8 -> byte_idx=0x69 >= tr_limit=0x69 -> #GP dispatched
    cpu.step(&mut bus); // HLT at #GP handler
    assert!(
        cpu.halted(),
        "IN AL, 8 must raise #GP (IOPB byte outside TSS limit)"
    );
    assert_eq!(
        cpu.ip(),
        PM_GP_HANDLER_IP as u32 + 1,
        "should be at HLT+1 after #GP from IN AL, 8"
    );
}

/// IRET from CPL0 returning to VM86 must consume 60 clocks, not 22.
#[test]
fn iret_cpl0_to_vm86_clocks_60() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    // Set up CPL0 protected mode.
    let mut state = setup_protected_mode(&mut bus, 0xFFFF);

    // Build a 9-dword CPL0->VM86 IRET stack frame on the PM stack.
    // Frame order on stack (high to low, ESP points to EIP):
    //   EIP, CS, EFLAGS (VM=1), ESP, SS, ES, DS, FS, GS
    let stack_base: u32 = PM_STACK_BASE;
    let initial_esp: u32 = 0xFFC0;
    let frame_esp = initial_esp - 9 * 4; // 0xFF9C
    write_dword_at(&mut bus, stack_base + frame_esp, 0x0010); // new EIP
    write_dword_at(&mut bus, stack_base + frame_esp + 4, 0x1000); // new CS (VM86)
    write_dword_at(&mut bus, stack_base + frame_esp + 8, 0x0002_3202); // EFLAGS: VM=1, IOPL=3
    write_dword_at(&mut bus, stack_base + frame_esp + 12, 0xF000); // new ESP (VM86 stack)
    write_dword_at(&mut bus, stack_base + frame_esp + 16, 0x2000); // new SS (VM86)
    write_dword_at(&mut bus, stack_base + frame_esp + 20, 0x3000); // new ES (VM86)
    write_dword_at(&mut bus, stack_base + frame_esp + 24, 0x4000); // new DS (VM86)
    write_dword_at(&mut bus, stack_base + frame_esp + 28, 0x5000); // new FS (VM86)
    write_dword_at(&mut bus, stack_base + frame_esp + 32, 0x6000); // new GS (VM86)
    state.set_esp(frame_esp);

    cpu.load_state(&state);

    // 66h CF = 32-bit IRETD
    place_at(&mut bus, PM_CODE_BASE, &[0x66, 0xCF]);

    cpu.step(&mut bus);

    assert_eq!(
        cpu.cycles_consumed(),
        60,
        "IRET from CPL0 returning to VM86 must take 60 clock cycles per Intel 80386 manual §17"
    );
    // Sanity check: we did enter VM86 mode.
    assert_ne!(
        cpu.eflags_upper & 0x0002_0000,
        0,
        "VM bit should be set after IRET to VM86"
    );
}

/// 32-bit IRETD from CPL0 returning to VM86 must clear the upper 16 bits of
/// EIP. EIP is a 16-bit value in V86 mode and the popped upper 16 bits are
/// discarded so subsequent fetches do not exceed the implicit 64KB CS limit.
#[test]
fn iret_cpl0_to_vm86_clears_ip_upper() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);

    // Build a 9-dword CPL0->VM86 IRET stack frame with a non-zero upper EIP.
    let stack_base: u32 = PM_STACK_BASE;
    let initial_esp: u32 = 0xFFC0;
    let frame_esp = initial_esp - 9 * 4;
    let new_eip: u32 = 0x0001_5678;
    write_dword_at(&mut bus, stack_base + frame_esp, new_eip);
    write_dword_at(&mut bus, stack_base + frame_esp + 4, 0x1000);
    write_dword_at(&mut bus, stack_base + frame_esp + 8, 0x0002_3202);
    write_dword_at(&mut bus, stack_base + frame_esp + 12, 0xF000);
    write_dword_at(&mut bus, stack_base + frame_esp + 16, 0x2000);
    write_dword_at(&mut bus, stack_base + frame_esp + 20, 0x3000);
    write_dword_at(&mut bus, stack_base + frame_esp + 24, 0x4000);
    write_dword_at(&mut bus, stack_base + frame_esp + 28, 0x5000);
    write_dword_at(&mut bus, stack_base + frame_esp + 32, 0x6000);
    state.set_esp(frame_esp);

    cpu.load_state(&state);

    place_at(&mut bus, PM_CODE_BASE, &[0x66, 0xCF]);

    cpu.step(&mut bus);

    assert_ne!(
        cpu.eflags_upper & 0x0002_0000,
        0,
        "VM bit should be set after IRETD to VM86"
    );
    assert_eq!(
        cpu.ip() as u16,
        new_eip as u16,
        "IP low 16 bits should match the popped EIP"
    );
    assert_eq!(
        cpu.ip_upper, 0,
        "ip_upper must be zero in VM86 regardless of the popped EIP upper bits"
    );
}

/// 32-bit EAX IN (operand-size override) must consult the IOPB for both `port`
/// and `port+2`. If `port+2` is denied, a #GP must be raised even though `port`
/// is allowed.
#[test]
fn iopb_32bit_eax_denied_at_port_plus_two() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.set_cs(PM_CS_SEL | 3); // Force CPL=3
    state.flags.iopl = 0; // IOPL=0 < CPL=3 -> IOPB consulted

    const TSS_BASE: u32 = 0xC0000;
    const TSS_SEL: u16 = 0x0028;
    // 386 TSS, limit=0x6A covers the IOPB byte (offset 0x68) plus sentinel (0x69).
    write_gdt_entry16(&mut bus, PM_GDT_BASE, 5, TSS_BASE, 0x6A, 0x89);
    state.gdt_limit = 6 * 8 - 1;

    write_dword_at(&mut bus, TSS_BASE + 0x04, 0xFF00); // ESP0 for #GP stack switch
    write_word_at(&mut bus, TSS_BASE + 0x08, PM_SS_SEL); // SS0
    write_word_at(&mut bus, TSS_BASE + 0x66, 0x0068); // IOPB pointer
    // bit 0 = 0 (port 0 allowed), bit 2 = 1 (port 2 denied)
    bus.ram[(TSS_BASE + 0x68) as usize] = 0x04;

    state.tr = TSS_SEL;
    state.tr_base = TSS_BASE;
    state.tr_limit = 0x6A;
    state.tr_rights = 0x89;
    cpu.load_state(&state);

    // 66 E5 00 = operand-size override + IN EAX, imm=0 (accesses port 0 and port 2)
    place_at(&mut bus, PM_CODE_BASE, &[0x66, 0xE5, 0x00]);

    cpu.step(&mut bus); // IN EAX,0 -> port 2 denied -> #GP dispatched
    cpu.step(&mut bus); // HLT at #GP handler
    assert!(
        cpu.halted(),
        "IN EAX,0 must raise #GP when port+2 is denied in IOPB"
    );
    assert_eq!(
        cpu.ip(),
        PM_GP_HANDLER_IP as u32 + 1,
        "should halt at GP handler HLT+1"
    );
}

/// 32-bit EAX IN succeeds when both `port` and `port+2` are clear in the IOPB.
#[test]
fn iopb_32bit_eax_allowed_when_both_ports_clear() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.set_cs(PM_CS_SEL | 3); // Force CPL=3
    state.flags.iopl = 0;

    const TSS_BASE: u32 = 0xC0000;
    const TSS_SEL: u16 = 0x0028;
    write_gdt_entry16(&mut bus, PM_GDT_BASE, 5, TSS_BASE, 0x6A, 0x89);
    state.gdt_limit = 6 * 8 - 1;

    write_dword_at(&mut bus, TSS_BASE + 0x04, 0xFF00);
    write_word_at(&mut bus, TSS_BASE + 0x08, PM_SS_SEL);
    write_word_at(&mut bus, TSS_BASE + 0x66, 0x0068);
    // All bits 0: all ports allowed
    bus.ram[(TSS_BASE + 0x68) as usize] = 0x00;

    state.tr = TSS_SEL;
    state.tr_base = TSS_BASE;
    state.tr_limit = 0x6A;
    state.tr_rights = 0x89;
    cpu.load_state(&state);

    place_at(&mut bus, PM_CODE_BASE, &[0x66, 0xE5, 0x00]); // IN EAX, 0

    cpu.step(&mut bus);

    assert!(
        !cpu.halted(),
        "IN EAX,0 must succeed when both port 0 and port 2 are allowed in IOPB"
    );
    assert_eq!(
        cpu.ip() as u16,
        3,
        "IP must advance past the 3-byte IN EAX instruction"
    );
}

/// 16-bit IRET in VM86 mode (IOPL=3 short path) must clear the RF flag.
/// Previously the implementation preserved the old RF bit instead of clearing it.
#[test]
fn vm86_iret_16bit_clears_rf() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_vm86(&mut bus);
    // Push a 16-bit IRET frame: [SP+0]=new IP, [SP+2]=new CS, [SP+4]=new FLAGS
    let vm86_stack_base: u32 = 0x20000; // SS=0x2000 -> base 0x20000
    let sp: u16 = 0xF000;
    write_word_at(&mut bus, vm86_stack_base + sp as u32 - 6, 0x0010); // new IP
    write_word_at(&mut bus, vm86_stack_base + sp as u32 - 4, 0x1000); // new CS
    write_word_at(&mut bus, vm86_stack_base + sp as u32 - 2, 0x0202); // new FLAGS: IF=1, bit1
    state.set_esp(sp as u32 - 6);
    cpu.load_state(&state);

    // Set RF bit in eflags_upper before IRET to confirm it gets cleared.
    cpu.eflags_upper |= 0x0001_0000;

    place_code(&mut bus, 0x1000, 0x0000, &[0xCF]); // IRET

    cpu.step(&mut bus);

    assert_eq!(
        cpu.eflags_upper & 0x0001_0000,
        0,
        "RF must be cleared by 16-bit IRET in VM86 mode"
    );
    assert_ne!(
        cpu.eflags_upper & 0x0002_0000,
        0,
        "VM flag must remain set after 16-bit IRET in VM86 mode"
    );
    assert_eq!(
        cpu.ip() as u16,
        0x0010,
        "IP must be set to the return address from the IRET frame"
    );
}

/// Stored CPL must be 0 after load_state with real-mode segment caches,
/// remain 0 after entering protected mode (PE transition with real-mode CS cache),
/// and become 3 after a far return to ring 3.
#[test]
fn i386_stored_cpl_tracks_privilege_transitions() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    assert_eq!(cpu.stored_cpl, 0, "stored_cpl should be 0 after reset");

    let state = setup_protected_mode_with_ring3(&mut bus);
    cpu.load_state(&state);

    assert_eq!(
        cpu.stored_cpl, 0,
        "stored_cpl should be 0 for ring-0 protected mode"
    );

    // RETF to ring 3.
    place_at(&mut bus, PM_CODE_BASE, &[0xCB]);
    let ring3_ip: u16 = 0x0100;
    bus.ram[(PM_RING3_CODE_BASE + ring3_ip as u32) as usize] = 0xF4; // HLT at target

    let sp = cpu.esp();
    write_word_at(&mut bus, PM_STACK_BASE + sp, ring3_ip);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 2, PM_RING3_CS_SEL);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 4, 0xFF00);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 6, PM_RING3_SS_SEL);

    cpu.step(&mut bus);

    assert_eq!(cpu.cs() & 3, 3, "CS RPL should be 3 after ring transition");
    assert_eq!(
        cpu.stored_cpl, 3,
        "stored_cpl should be 3 after far return to ring 3"
    );
}

/// Protected mode: triple fault (no valid IDT entries) causes CPU shutdown+halt.
///
/// Per Intel 386 Programmer's Reference Manual §9.8.8:
/// "If any other exception occurs while attempting to invoke the double-fault
/// handler, the processor shuts down."
///
/// Fault cascade with IDT limit = 0:
///  1. INT 3 -> interrupt_protected -> IDT too small -> #GP (trap_level 1)
///  2. #GP dispatch -> IDT too small -> #DF (trap_level 2)
///  3. #DF dispatch -> IDT too small -> shutdown (trap_level 3, CPU halts)
#[test]
fn i386_triple_fault_halts_cpu() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    write_gdt_entry16(&mut bus, PM_GDT_BASE, 0, 0, 0, 0);
    write_gdt_entry16(&mut bus, PM_GDT_BASE, 1, PM_CODE_BASE, 0xFFFF, 0x9B);
    write_gdt_entry16(&mut bus, PM_GDT_BASE, 3, PM_STACK_BASE, 0xFFFF, 0x93);

    let mut state = cpu::I386State {
        cr0: 0x0001,
        ip: 0x0000,
        ..Default::default()
    };
    state.set_esp(0xFFF0);
    state.set_cs(PM_CS_SEL);
    state.set_ss(PM_SS_SEL);

    state.seg_bases[cpu::SegReg32::CS as usize] = PM_CODE_BASE;
    state.seg_bases[cpu::SegReg32::SS as usize] = PM_STACK_BASE;
    state.seg_limits[cpu::SegReg32::CS as usize] = 0xFFFF;
    state.seg_limits[cpu::SegReg32::SS as usize] = 0xFFFF;
    state.seg_rights[cpu::SegReg32::CS as usize] = 0x9B;
    state.seg_rights[cpu::SegReg32::SS as usize] = 0x93;
    state.seg_valid = [false, true, true, false, false, false];

    state.gdt_base = PM_GDT_BASE;
    state.gdt_limit = 4 * 8 - 1;
    state.idt_base = PM_IDT_BASE;
    state.idt_limit = 0;

    cpu.load_state(&state);

    // INT 3 with no valid IDT entries triggers the triple-fault cascade.
    place_at(&mut bus, PM_CODE_BASE, &[0xCC]);

    cpu.step(&mut bus);

    assert!(cpu.halted(), "CPU must be halted after triple fault");
}

const PM_DF_HANDLER_IP: u16 = 0xC000;

/// Protected mode: contributory + contributory -> double fault.
///
/// Per Intel 386 manual Table 9-4: when a contributory exception occurs
/// while dispatching another contributory exception, the CPU escalates
/// to a double fault (#DF, vector 8).
///
/// Setup: IDT entry for #DE (vector 0, contributory) points to a selector
/// beyond the GDT limit. Dispatching #DE fails with #GP (vector 13,
/// contributory). Contributory + contributory escalates to #DF.
#[test]
fn i386_double_fault_contributory_plus_contributory() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    write_gdt_entry16(&mut bus, PM_GDT_BASE, 0, 0, 0, 0);
    write_gdt_entry16(&mut bus, PM_GDT_BASE, 1, PM_CODE_BASE, 0xFFFF, 0x9B);
    write_gdt_entry16(&mut bus, PM_GDT_BASE, 2, PM_DATA_BASE, 0xFFFF, 0x93);
    write_gdt_entry16(&mut bus, PM_GDT_BASE, 3, PM_STACK_BASE, 0xFFFF, 0x93);

    // IDT entry 0 (#DE): selector 0x28 (index 5) beyond GDT limit -> #GP on dispatch.
    write_idt_gate(&mut bus, PM_IDT_BASE, 0, 0x0000, 0x28, 14, 0);
    // IDT entry 8 (#DF): valid handler.
    write_idt_gate(
        &mut bus,
        PM_IDT_BASE,
        8,
        PM_DF_HANDLER_IP as u32,
        PM_CS_SEL,
        14,
        0,
    );
    // IDT entry 13 (#GP): valid handler (should NOT be reached).
    write_idt_gate(
        &mut bus,
        PM_IDT_BASE,
        13,
        PM_GP_HANDLER_IP as u32,
        PM_CS_SEL,
        14,
        0,
    );

    bus.ram[(PM_CODE_BASE + PM_DF_HANDLER_IP as u32) as usize] = 0xF4; // HLT
    bus.ram[(PM_CODE_BASE + PM_GP_HANDLER_IP as u32) as usize] = 0xF4; // HLT

    let mut state = cpu::I386State {
        cr0: 0x0001,
        ip: 0x0000,
        ..Default::default()
    };
    state.set_esp(0xFFF0);
    state.set_cs(PM_CS_SEL);
    state.set_ds(PM_DS_SEL);
    state.set_ss(PM_SS_SEL);
    state.set_es(PM_DS_SEL);

    state.seg_bases[cpu::SegReg32::ES as usize] = PM_DATA_BASE;
    state.seg_bases[cpu::SegReg32::CS as usize] = PM_CODE_BASE;
    state.seg_bases[cpu::SegReg32::SS as usize] = PM_STACK_BASE;
    state.seg_bases[cpu::SegReg32::DS as usize] = PM_DATA_BASE;

    state.seg_limits = [0xFFFF; 6];
    state.seg_rights[cpu::SegReg32::ES as usize] = 0x93;
    state.seg_rights[cpu::SegReg32::CS as usize] = 0x9B;
    state.seg_rights[cpu::SegReg32::SS as usize] = 0x93;
    state.seg_rights[cpu::SegReg32::DS as usize] = 0x93;
    state.seg_valid = [true, true, true, true, false, false];

    state.gdt_base = PM_GDT_BASE;
    state.gdt_limit = 4 * 8 - 1;
    state.idt_base = PM_IDT_BASE;
    state.idt_limit = 256 * 8 - 1;

    cpu.load_state(&state);

    // DIV CL with AX=1, CL=0 -> #DE (vector 0, contributory).
    // F6 F1 = DIV CL
    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[
            0xB0, 0x01, // MOV AL, 1
            0xB1, 0x00, // MOV CL, 0
            0xF6, 0xF1, // DIV CL
        ],
    );

    cpu.step(&mut bus); // MOV AL, 1
    cpu.step(&mut bus); // MOV CL, 0
    cpu.step(&mut bus); // DIV CL -> #DE -> #GP -> #DF
    cpu.step(&mut bus); // HLT in #DF handler

    assert!(cpu.halted(), "CPU must be halted in #DF handler");
    assert_eq!(
        cpu.ip(),
        PM_DF_HANDLER_IP as u32 + 1,
        "IP should be in #DF handler (after HLT)"
    );
}

/// Protected mode: benign + contributory -> normal (no escalation).
///
/// Per Intel 386 manual Table 9-4: when a contributory exception occurs
/// while dispatching a benign exception, the CPU handles it normally
/// (no escalation to double fault).
///
/// Setup: IDT entry for #UD (vector 6, benign) points to a selector
/// beyond the GDT limit. Dispatching #UD fails with #GP (vector 13,
/// contributory). Benign + contributory = OK, so #GP is dispatched
/// normally via the #GP handler.
#[test]
fn i386_no_double_fault_benign_plus_contributory() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    write_gdt_entry16(&mut bus, PM_GDT_BASE, 0, 0, 0, 0);
    write_gdt_entry16(&mut bus, PM_GDT_BASE, 1, PM_CODE_BASE, 0xFFFF, 0x9B);
    write_gdt_entry16(&mut bus, PM_GDT_BASE, 2, PM_DATA_BASE, 0xFFFF, 0x93);
    write_gdt_entry16(&mut bus, PM_GDT_BASE, 3, PM_STACK_BASE, 0xFFFF, 0x93);

    // IDT entry 6 (#UD): selector 0x28 (index 5) beyond GDT limit -> #GP on dispatch.
    write_idt_gate(&mut bus, PM_IDT_BASE, 6, 0x0000, 0x28, 14, 0);
    // IDT entry 8 (#DF): valid handler (should NOT be reached).
    write_idt_gate(
        &mut bus,
        PM_IDT_BASE,
        8,
        PM_DF_HANDLER_IP as u32,
        PM_CS_SEL,
        14,
        0,
    );
    // IDT entry 13 (#GP): valid handler (SHOULD be reached).
    write_idt_gate(
        &mut bus,
        PM_IDT_BASE,
        13,
        PM_GP_HANDLER_IP as u32,
        PM_CS_SEL,
        14,
        0,
    );

    bus.ram[(PM_CODE_BASE + PM_DF_HANDLER_IP as u32) as usize] = 0xF4; // HLT
    bus.ram[(PM_CODE_BASE + PM_GP_HANDLER_IP as u32) as usize] = 0xF4; // HLT

    let mut state = cpu::I386State {
        cr0: 0x0001,
        ip: 0x0000,
        ..Default::default()
    };
    state.set_esp(0xFFF0);
    state.set_cs(PM_CS_SEL);
    state.set_ds(PM_DS_SEL);
    state.set_ss(PM_SS_SEL);
    state.set_es(PM_DS_SEL);

    state.seg_bases[cpu::SegReg32::ES as usize] = PM_DATA_BASE;
    state.seg_bases[cpu::SegReg32::CS as usize] = PM_CODE_BASE;
    state.seg_bases[cpu::SegReg32::SS as usize] = PM_STACK_BASE;
    state.seg_bases[cpu::SegReg32::DS as usize] = PM_DATA_BASE;

    state.seg_limits = [0xFFFF; 6];
    state.seg_rights[cpu::SegReg32::ES as usize] = 0x93;
    state.seg_rights[cpu::SegReg32::CS as usize] = 0x9B;
    state.seg_rights[cpu::SegReg32::SS as usize] = 0x93;
    state.seg_rights[cpu::SegReg32::DS as usize] = 0x93;
    state.seg_valid = [true, true, true, true, false, false];

    state.gdt_base = PM_GDT_BASE;
    state.gdt_limit = 4 * 8 - 1;
    state.idt_base = PM_IDT_BASE;
    state.idt_limit = 256 * 8 - 1;

    cpu.load_state(&state);

    // 0F FF is an undefined opcode -> #UD (vector 6, benign).
    place_at(&mut bus, PM_CODE_BASE, &[0x0F, 0xFF]);

    cpu.step(&mut bus); // #UD -> dispatch fails -> #GP (benign + contrib = OK)
    cpu.step(&mut bus); // HLT in #GP handler

    assert!(cpu.halted(), "CPU must be halted in #GP handler");
    assert_eq!(
        cpu.ip(),
        PM_GP_HANDLER_IP as u32 + 1,
        "IP should be in #GP handler (not #DF)"
    );
}

/// 32-bit code segment: JMP short (EB rel8) at EIP > 0xFFFF preserves upper EIP bits.
///
/// Per Intel 386 Programmer's Reference Manual §3.5.1.1:
/// "The processor forms an effective address by adding this relative
/// displacement to the address contained in EIP."
///
/// A short branch must add the signed 8-bit displacement to the full 32-bit
/// EIP, not just the lower 16-bit IP.
#[test]
fn i386_jmp_short_preserves_eip_upper_in_32bit_mode() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    // 32-bit code segment: base=0, limit=0xFFFFF (page granularity = 4GB),
    // D-bit set (granularity byte 0xC0 = page granularity + 32-bit default).
    write_gdt_entry(&mut bus, PM_GDT_BASE, 0, 0, 0, 0, 0);
    write_gdt_entry(&mut bus, PM_GDT_BASE, 1, 0, 0xFFFFF, 0x9B, 0xC0);
    write_gdt_entry(&mut bus, PM_GDT_BASE, 3, PM_STACK_BASE, 0xFFFF, 0x93, 0);
    write_idt_gate(
        &mut bus,
        PM_IDT_BASE,
        13,
        PM_GP_HANDLER_IP as u32,
        PM_CS_SEL,
        14,
        0,
    );

    let start_eip: u32 = 0x1_0004;

    let mut state = cpu::I386State {
        cr0: 0x0001,
        ..Default::default()
    };
    state.set_eip(start_eip);
    state.set_esp(0xFFF0);
    state.set_cs(PM_CS_SEL);
    state.set_ss(PM_SS_SEL);

    state.seg_bases[cpu::SegReg32::CS as usize] = 0;
    state.seg_bases[cpu::SegReg32::SS as usize] = PM_STACK_BASE;
    state.seg_limits[cpu::SegReg32::CS as usize] = 0xFFFF_FFFF;
    state.seg_limits[cpu::SegReg32::SS as usize] = 0xFFFF;
    state.seg_rights[cpu::SegReg32::CS as usize] = 0x9B;
    state.seg_rights[cpu::SegReg32::SS as usize] = 0x93;
    state.seg_granularity[cpu::SegReg32::CS as usize] = 0xC0;
    state.seg_valid = [false, true, true, false, false, false];

    state.gdt_base = PM_GDT_BASE;
    state.gdt_limit = 4 * 8 - 1;
    state.idt_base = PM_IDT_BASE;
    state.idt_limit = 256 * 8 - 1;

    cpu.load_state(&state);

    // JMP short -4 (EB FC) at EIP=0x10004.
    bus.ram[start_eip as usize] = 0xEB; // JMP short
    bus.ram[start_eip as usize + 1] = 0xFC; // rel8 = -4

    // Place HLT at target (0x10002).
    bus.ram[0x1_0002] = 0xF4; // HLT

    cpu.step(&mut bus); // JMP short -> EIP = 0x10002
    cpu.step(&mut bus); // HLT

    assert!(cpu.halted(), "CPU must halt after JMP short -> HLT");
    assert_eq!(
        cpu.ip(),
        0x1_0003,
        "EIP should be 0x10003 (0x10002 + 1 for HLT), upper bits preserved"
    );
}

/// 32-bit code segment: Jcc short (e.g., JZ rel8) at EIP > 0xFFFF preserves upper EIP bits.
#[test]
fn i386_jcc_short_preserves_eip_upper_in_32bit_mode() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    write_gdt_entry(&mut bus, PM_GDT_BASE, 0, 0, 0, 0, 0);
    write_gdt_entry(&mut bus, PM_GDT_BASE, 1, 0, 0xFFFFF, 0x9B, 0xC0);
    write_gdt_entry(&mut bus, PM_GDT_BASE, 3, PM_STACK_BASE, 0xFFFF, 0x93, 0);

    let start_eip: u32 = 0x1_0000;

    let mut state = cpu::I386State {
        cr0: 0x0001,
        ..Default::default()
    };
    state.set_eip(start_eip);
    state.set_esp(0xFFF0);
    state.set_cs(PM_CS_SEL);
    state.set_ss(PM_SS_SEL);

    state.seg_bases[cpu::SegReg32::CS as usize] = 0;
    state.seg_bases[cpu::SegReg32::SS as usize] = PM_STACK_BASE;
    state.seg_limits[cpu::SegReg32::CS as usize] = 0xFFFF_FFFF;
    state.seg_limits[cpu::SegReg32::SS as usize] = 0xFFFF;
    state.seg_rights[cpu::SegReg32::CS as usize] = 0x9B;
    state.seg_rights[cpu::SegReg32::SS as usize] = 0x93;
    state.seg_granularity[cpu::SegReg32::CS as usize] = 0xC0;
    state.seg_valid = [false, true, true, false, false, false];

    state.gdt_base = PM_GDT_BASE;
    state.gdt_limit = 4 * 8 - 1;
    state.idt_base = PM_IDT_BASE;
    state.idt_limit = 256 * 8 - 1;

    cpu.load_state(&state);

    // XOR EAX, EAX (sets ZF=1) then JZ +4 at EIP=0x10002.
    // After JZ instruction (2 bytes), IP = 0x10004. Disp +4 -> EIP = 0x10008.
    bus.ram[start_eip as usize] = 0x31; // XOR AX, AX (2 bytes: 31 C0)
    bus.ram[start_eip as usize + 1] = 0xC0;
    bus.ram[start_eip as usize + 2] = 0x74; // JZ rel8
    bus.ram[start_eip as usize + 3] = 0x04; // +4
    bus.ram[0x1_0008] = 0xF4; // HLT at target

    cpu.step(&mut bus); // XOR AX, AX (ZF=1)
    cpu.step(&mut bus); // JZ +4 -> EIP = 0x10008
    cpu.step(&mut bus); // HLT

    assert!(cpu.halted());
    assert_eq!(
        cpu.ip(),
        0x1_0009,
        "EIP should be 0x10009 (after HLT at 0x10008), upper bits preserved"
    );
}

/// 32-bit code segment: LOOP at EIP > 0xFFFF preserves upper EIP bits.
#[test]
fn i386_loop_preserves_eip_upper_in_32bit_mode() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    write_gdt_entry(&mut bus, PM_GDT_BASE, 0, 0, 0, 0, 0);
    write_gdt_entry(&mut bus, PM_GDT_BASE, 1, 0, 0xFFFFF, 0x9B, 0xC0);
    write_gdt_entry(&mut bus, PM_GDT_BASE, 3, PM_STACK_BASE, 0xFFFF, 0x93, 0);

    let start_eip: u32 = 0x1_0000;

    let mut state = cpu::I386State {
        cr0: 0x0001,
        ..Default::default()
    };
    state.set_eip(start_eip);
    state.set_esp(0xFFF0);
    state.set_ecx(1); // LOOP will decrement to 0 and NOT branch
    state.set_cs(PM_CS_SEL);
    state.set_ss(PM_SS_SEL);

    state.seg_bases[cpu::SegReg32::CS as usize] = 0;
    state.seg_bases[cpu::SegReg32::SS as usize] = PM_STACK_BASE;
    state.seg_limits[cpu::SegReg32::CS as usize] = 0xFFFF_FFFF;
    state.seg_limits[cpu::SegReg32::SS as usize] = 0xFFFF;
    state.seg_rights[cpu::SegReg32::CS as usize] = 0x9B;
    state.seg_rights[cpu::SegReg32::SS as usize] = 0x93;
    state.seg_granularity[cpu::SegReg32::CS as usize] = 0xC0;
    state.seg_valid = [false, true, true, false, false, false];

    state.gdt_base = PM_GDT_BASE;
    state.gdt_limit = 4 * 8 - 1;
    state.idt_base = PM_IDT_BASE;
    state.idt_limit = 256 * 8 - 1;

    cpu.load_state(&state);

    // ECX=2: LOOP -2 at EIP=0x10000. First iteration branches back to 0x10000.
    // Second iteration CX=0, falls through to HLT at 0x10002.
    state.set_ecx(2);
    cpu.load_state(&state);

    bus.ram[start_eip as usize] = 0xE2; // LOOP rel8
    bus.ram[start_eip as usize + 1] = 0xFE; // -2 (back to self)
    bus.ram[0x1_0002] = 0xF4; // HLT

    cpu.step(&mut bus); // LOOP: ECX 2->1, branch to 0x10000
    assert_eq!(cpu.ip(), 0x1_0000, "LOOP should branch back to 0x10000");

    cpu.step(&mut bus); // LOOP: ECX 1->0, fall through to 0x10002
    assert_eq!(cpu.ip(), 0x1_0002, "LOOP should fall through to 0x10002");

    cpu.step(&mut bus); // HLT
    assert!(cpu.halted());
}

const PM_DE_HANDLER_IP: u16 = 0xD000;
const PM_UD_HANDLER_IP: u16 = 0xD100;
const PM_BP_HANDLER_IP: u16 = 0xD200;
const PM_OF_HANDLER_IP: u16 = 0xD300;
const PM_BR_HANDLER_IP: u16 = 0xD400;
const PM_DB_HANDLER_IP: u16 = 0xD500;
const PM_PAGE_FAULT_HANDLER_IP: u16 = 0xD600;

fn setup_protected_mode_with_exception_handlers(bus: &mut TestBus) -> cpu::I386State {
    write_gdt_entry16(bus, PM_GDT_BASE, 0, 0, 0, 0);
    write_gdt_entry16(bus, PM_GDT_BASE, 1, PM_CODE_BASE, 0xFFFF, 0x9B);
    write_gdt_entry16(bus, PM_GDT_BASE, 2, PM_DATA_BASE, 0xFFFF, 0x93);
    write_gdt_entry16(bus, PM_GDT_BASE, 3, PM_STACK_BASE, 0xFFFF, 0x93);

    // Vector 0: #DE
    write_idt_gate(
        bus,
        PM_IDT_BASE,
        0,
        PM_DE_HANDLER_IP as u32,
        PM_CS_SEL,
        14,
        0,
    );
    // Vector 1: #DB
    write_idt_gate(
        bus,
        PM_IDT_BASE,
        1,
        PM_DB_HANDLER_IP as u32,
        PM_CS_SEL,
        14,
        0,
    );
    // Vector 3: #BP (trap gate so EIP points after INT3)
    write_idt_gate(
        bus,
        PM_IDT_BASE,
        3,
        PM_BP_HANDLER_IP as u32,
        PM_CS_SEL,
        15,
        0,
    );
    // Vector 4: #OF (trap gate)
    write_idt_gate(
        bus,
        PM_IDT_BASE,
        4,
        PM_OF_HANDLER_IP as u32,
        PM_CS_SEL,
        15,
        0,
    );
    // Vector 5: #BR
    write_idt_gate(
        bus,
        PM_IDT_BASE,
        5,
        PM_BR_HANDLER_IP as u32,
        PM_CS_SEL,
        14,
        0,
    );
    // Vector 6: #UD
    write_idt_gate(
        bus,
        PM_IDT_BASE,
        6,
        PM_UD_HANDLER_IP as u32,
        PM_CS_SEL,
        14,
        0,
    );
    // Vector 13: #GP
    write_idt_gate(
        bus,
        PM_IDT_BASE,
        13,
        PM_GP_HANDLER_IP as u32,
        PM_CS_SEL,
        14,
        0,
    );
    // Vector 14: #PF
    write_idt_gate(
        bus,
        PM_IDT_BASE,
        14,
        PM_PAGE_FAULT_HANDLER_IP as u32,
        PM_CS_SEL,
        14,
        0,
    );

    bus.ram[(PM_CODE_BASE + PM_DE_HANDLER_IP as u32) as usize] = 0xF4;
    bus.ram[(PM_CODE_BASE + PM_UD_HANDLER_IP as u32) as usize] = 0xF4;
    bus.ram[(PM_CODE_BASE + PM_BP_HANDLER_IP as u32) as usize] = 0xF4;
    bus.ram[(PM_CODE_BASE + PM_OF_HANDLER_IP as u32) as usize] = 0xF4;
    bus.ram[(PM_CODE_BASE + PM_BR_HANDLER_IP as u32) as usize] = 0xF4;
    bus.ram[(PM_CODE_BASE + PM_DB_HANDLER_IP as u32) as usize] = 0xF4;
    bus.ram[(PM_CODE_BASE + PM_GP_HANDLER_IP as u32) as usize] = 0xF4;
    bus.ram[(PM_CODE_BASE + PM_PAGE_FAULT_HANDLER_IP as u32) as usize] = 0xF4;

    let mut state = cpu::I386State {
        cr0: 0x0001,
        ip: 0x0000,
        ..Default::default()
    };
    state.set_esp(0xFFF0);
    state.set_cs(PM_CS_SEL);
    state.set_ds(PM_DS_SEL);
    state.set_ss(PM_SS_SEL);
    state.set_es(PM_DS_SEL);

    state.seg_bases[cpu::SegReg32::ES as usize] = PM_DATA_BASE;
    state.seg_bases[cpu::SegReg32::CS as usize] = PM_CODE_BASE;
    state.seg_bases[cpu::SegReg32::SS as usize] = PM_STACK_BASE;
    state.seg_bases[cpu::SegReg32::DS as usize] = PM_DATA_BASE;

    state.seg_limits = [0xFFFF; 6];
    state.seg_rights[cpu::SegReg32::ES as usize] = 0x93;
    state.seg_rights[cpu::SegReg32::CS as usize] = 0x9B;
    state.seg_rights[cpu::SegReg32::SS as usize] = 0x93;
    state.seg_rights[cpu::SegReg32::DS as usize] = 0x93;
    state.seg_valid = [true, true, true, true, false, false];

    state.gdt_base = PM_GDT_BASE;
    state.gdt_limit = 4 * 8 - 1;
    state.idt_base = PM_IDT_BASE;
    state.idt_limit = 256 * 8 - 1;

    state
}

#[test]
fn i386_div_by_zero_byte_raises_de() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    // DIV CL with CL=0, AX=1
    cpu.state.set_eax(0x0001);
    cpu.state.set_ecx(0x0000);
    // F6 F1 = DIV CL
    place_at(&mut bus, PM_CODE_BASE, &[0xF6, 0xF1]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_DE_HANDLER_IP as u32 + 1);
}

#[test]
fn i386_div_by_zero_word_raises_de() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    // DIV CX with CX=0, DX:AX=1
    cpu.state.set_eax(0x0001);
    cpu.state.set_edx(0x0000);
    cpu.state.set_ecx(0x0000);
    // F7 F1 = DIV CX
    place_at(&mut bus, PM_CODE_BASE, &[0xF7, 0xF1]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_DE_HANDLER_IP as u32 + 1);
}

#[test]
fn i386_div_by_zero_dword_raises_de() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    // 66 F7 F1 = DIV ECX (with operand size prefix for 32-bit)
    cpu.state.set_eax(0x0000_0001);
    cpu.state.set_edx(0x0000_0000);
    cpu.state.set_ecx(0x0000_0000);
    place_at(&mut bus, PM_CODE_BASE, &[0x66, 0xF7, 0xF1]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_DE_HANDLER_IP as u32 + 1);
}

#[test]
fn i386_fault_entry_pushes_eflags_with_rf_set() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    cpu.state.set_eax(0x0001);
    cpu.state.set_ecx(0x0000);
    place_at(&mut bus, PM_CODE_BASE, &[0xF6, 0xF1]); // DIV CL

    cpu.step(&mut bus); // DIV CL -> #DE
    cpu.step(&mut bus); // HLT in #DE handler

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_DE_HANDLER_IP as u32 + 1);

    // #DE pushes no error code, frame is EIP, CS, EFLAGS at SP+0, +4, +8.
    let sp = cpu.esp();
    let pushed_eflags = read_dword_at(&bus, PM_STACK_BASE + sp + 8);
    assert!(
        pushed_eflags & 0x0001_0000 != 0,
        "fault entry must push EFLAGS with RF=1, got {:#010x}",
        pushed_eflags,
    );
}

#[test]
fn i386_software_int_does_not_push_rf() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    place_at(&mut bus, PM_CODE_BASE, &[0xCD, 0x00]); // INT 0 (software int, NOT a fault)

    cpu.step(&mut bus); // INT 0 -> handler entry
    cpu.step(&mut bus); // HLT in #DE handler

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_DE_HANDLER_IP as u32 + 1);

    let sp = cpu.esp();
    let pushed_eflags = read_dword_at(&bus, PM_STACK_BASE + sp + 8);
    assert!(
        pushed_eflags & 0x0001_0000 == 0,
        "software INT must push EFLAGS with RF=0, got {:#010x}",
        pushed_eflags,
    );
}

#[test]
fn i386_idiv_overflow_raises_de() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    // IDIV CL with AX=0x8000 (-32768), CL=0xFF (-1) -> quotient 32768 overflows signed byte
    cpu.state.set_eax(0x8000);
    cpu.state.set_ecx(0x00FF);
    // F6 F9 = IDIV CL
    place_at(&mut bus, PM_CODE_BASE, &[0xF6, 0xF9]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_DE_HANDLER_IP as u32 + 1);
}

#[test]
fn i386_div_by_zero_no_error_code_pushed() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    let sp_before = cpu.esp();

    cpu.state.set_eax(0x0001);
    cpu.state.set_ecx(0x0000);
    place_at(&mut bus, PM_CODE_BASE, &[0xF6, 0xF1]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    // #DE pushes no error code: only EFLAGS + CS + EIP = 12 bytes (3 dwords)
    let sp_after = cpu.esp();
    assert_eq!(
        sp_before - sp_after,
        12,
        "#DE should not push an error code"
    );
}

#[test]
fn i386_lock_nop_raises_ud() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    // F0 90 = LOCK NOP
    place_at(&mut bus, PM_CODE_BASE, &[0xF0, 0x90]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_UD_HANDLER_IP as u32 + 1);
}

#[test]
fn i386_lock_mov_raises_ud() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    // F0 89 C0 = LOCK MOV EAX, EAX
    place_at(&mut bus, PM_CODE_BASE, &[0xF0, 0x89, 0xC0]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_UD_HANDLER_IP as u32 + 1);
}

fn setup_protected_mode_ring3_with_exception_handlers(bus: &mut TestBus) -> cpu::I386State {
    write_gdt_entry16(bus, PM_GDT_BASE, 0, 0, 0, 0);
    write_gdt_entry16(bus, PM_GDT_BASE, 1, PM_CODE_BASE, 0xFFFF, 0x9B);
    write_gdt_entry16(bus, PM_GDT_BASE, 2, PM_DATA_BASE, 0xFFFF, 0x93);
    write_gdt_entry16(bus, PM_GDT_BASE, 3, PM_STACK_BASE, 0xFFFF, 0x93);
    write_gdt_entry16(bus, PM_GDT_BASE, 4, PM_RING3_CODE_BASE, 0xFFFF, 0xFB);
    write_gdt_entry16(bus, PM_GDT_BASE, 5, PM_DATA_BASE, 0xFFFF, 0xF3);
    write_gdt_entry16(bus, PM_GDT_BASE, 6, PM_RING3_STACK_BASE, 0xFFFF, 0xF3);
    write_gdt_entry16(bus, PM_GDT_BASE, 7, PM_TSS_BASE, 103, 0x89);

    // Vector 0: #DE
    write_idt_gate(
        bus,
        PM_IDT_BASE,
        0,
        PM_DE_HANDLER_IP as u32,
        PM_CS_SEL,
        14,
        0,
    );
    // Vector 6: #UD
    write_idt_gate(
        bus,
        PM_IDT_BASE,
        6,
        PM_UD_HANDLER_IP as u32,
        PM_CS_SEL,
        14,
        0,
    );
    // Vector 13: #GP
    write_idt_gate(
        bus,
        PM_IDT_BASE,
        13,
        PM_GP_HANDLER_IP as u32,
        PM_CS_SEL,
        14,
        0,
    );

    bus.ram[(PM_CODE_BASE + PM_DE_HANDLER_IP as u32) as usize] = 0xF4;
    bus.ram[(PM_CODE_BASE + PM_UD_HANDLER_IP as u32) as usize] = 0xF4;
    bus.ram[(PM_CODE_BASE + PM_GP_HANDLER_IP as u32) as usize] = 0xF4;

    write_dword_at(bus, PM_TSS_BASE + 4, 0xFFF0);
    write_word_at(bus, PM_TSS_BASE + 8, PM_SS_SEL);
    // IOPB offset at TSS+0x66: point past TSS limit so no port is permitted
    write_word_at(bus, PM_TSS_BASE + 0x66, 0x0068);

    let mut state = cpu::I386State {
        cr0: 0x0001,
        ip: 0x0000,
        ..Default::default()
    };

    state.set_esp(0xFFF0);
    state.set_cs(PM_RING3_CS_SEL);
    state.set_ds(PM_RING3_DS_SEL);
    state.set_ss(PM_RING3_SS_SEL);
    state.set_es(PM_RING3_DS_SEL);

    state.seg_bases[cpu::SegReg32::ES as usize] = PM_DATA_BASE;
    state.seg_bases[cpu::SegReg32::CS as usize] = PM_RING3_CODE_BASE;
    state.seg_bases[cpu::SegReg32::SS as usize] = PM_RING3_STACK_BASE;
    state.seg_bases[cpu::SegReg32::DS as usize] = PM_DATA_BASE;

    state.seg_limits = [0xFFFF; 6];
    state.seg_rights[cpu::SegReg32::ES as usize] = 0xF3;
    state.seg_rights[cpu::SegReg32::CS as usize] = 0xFB;
    state.seg_rights[cpu::SegReg32::SS as usize] = 0xF3;
    state.seg_rights[cpu::SegReg32::DS as usize] = 0xF3;
    state.seg_valid = [true, true, true, true, false, false];

    state.gdt_base = PM_GDT_BASE;
    state.gdt_limit = 8 * 8 - 1;
    state.idt_base = PM_IDT_BASE;
    state.idt_limit = 256 * 8 - 1;

    state.tr = PM_TSS_SEL;
    state.tr_base = PM_TSS_BASE;
    state.tr_limit = 103;
    state.tr_rights = 0x8B;

    state.flags.iopl = 0;

    state
}

#[test]
fn i386_in_at_cpl3_raises_gp() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_ring3_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    // E4 42 = IN AL, 0x42
    place_at(&mut bus, PM_RING3_CODE_BASE, &[0xE4, 0x42]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_GP_HANDLER_IP as u32 + 1);
}

#[test]
fn i386_out_at_cpl3_raises_gp() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_ring3_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    // E6 42 = OUT 0x42, AL
    place_at(&mut bus, PM_RING3_CODE_BASE, &[0xE6, 0x42]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_GP_HANDLER_IP as u32 + 1);
}

#[test]
fn i386_rep_insb_at_cpl3_raises_gp() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_ring3_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    cpu.state.set_ecx(1);
    cpu.state.set_edx(0x0042);
    // F3 6C = REP INSB
    place_at(&mut bus, PM_RING3_CODE_BASE, &[0xF3, 0x6C]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_GP_HANDLER_IP as u32 + 1);
}

#[test]
fn i386_rep_outsb_at_cpl3_raises_gp() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_ring3_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    cpu.state.set_ecx(1);
    cpu.state.set_edx(0x0042);
    // F3 6E = REP OUTSB
    place_at(&mut bus, PM_RING3_CODE_BASE, &[0xF3, 0x6E]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_GP_HANDLER_IP as u32 + 1);
}

#[test]
fn i386_hlt_at_cpl3_raises_gp() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_ring3_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    // F4 = HLT
    place_at(&mut bus, PM_RING3_CODE_BASE, &[0xF4]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_GP_HANDLER_IP as u32 + 1);
}

#[test]
fn i386_int3_raises_bp() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    // CC = INT3
    place_at(&mut bus, PM_CODE_BASE, &[0xCC]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_BP_HANDLER_IP as u32 + 1);
}

#[test]
fn i386_int3_return_ip_after_instruction() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    // CC = INT3 (1 byte)
    place_at(&mut bus, PM_CODE_BASE, &[0xCC]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());

    // Pushed EIP should point after the 1-byte INT3 (i.e., 0x0001)
    let sp = cpu.esp();
    let pushed_eip = read_dword_at(&bus, PM_STACK_BASE + sp);
    assert_eq!(
        pushed_eip, 0x0001,
        "INT3 is a trap; pushed EIP should point after the instruction"
    );
}

#[test]
fn i386_into_with_overflow_raises_of() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    // Set OF=1: ADD 0x7FFF + 1 overflows a signed 16-bit
    // B8 FF 7F = MOV AX, 0x7FFF
    // 05 01 00 = ADD AX, 1
    // CE = INTO
    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[
            0xB8, 0xFF, 0x7F, // MOV AX, 0x7FFF
            0x05, 0x01, 0x00, // ADD AX, 1
            0xCE, // INTO
        ],
    );

    cpu.step(&mut bus); // MOV AX, 0x7FFF
    cpu.step(&mut bus); // ADD AX, 1 -> OF=1
    cpu.step(&mut bus); // INTO -> vector 4
    cpu.step(&mut bus); // HLT

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_OF_HANDLER_IP as u32 + 1);
}

#[test]
fn i386_into_without_overflow_is_nop() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    // OF=0 (default), INTO should be a no-op, then HLT
    // CE = INTO
    // F4 = HLT
    place_at(&mut bus, PM_CODE_BASE, &[0xCE, 0xF4]);

    cpu.step(&mut bus); // INTO - OF=0, no trap
    cpu.step(&mut bus); // HLT

    assert!(cpu.halted());
    assert_eq!(
        cpu.ip(),
        0x0002,
        "INTO with OF=0 should fall through to next instruction"
    );
}

#[test]
fn i386_bound_in_range_succeeds() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    // Set up bounds in memory at DS:0x0000 - lower=0x0000, upper=0x00FF
    write_word_at(&mut bus, PM_DATA_BASE, 0x0000);
    write_word_at(&mut bus, PM_DATA_BASE + 2, 0x00FF);

    // AX = 0x0050 (within [0, 0xFF])
    cpu.state.set_eax(0x0050);

    // 62 06 00 00 = BOUND AX, [0x0000]
    // F4 = HLT
    place_at(&mut bus, PM_CODE_BASE, &[0x62, 0x06, 0x00, 0x00, 0xF4]);

    cpu.step(&mut bus); // BOUND - in range, no exception
    cpu.step(&mut bus); // HLT

    assert!(cpu.halted());
    assert_eq!(
        cpu.ip(),
        0x0005,
        "BOUND in range should continue to next instruction"
    );
}

#[test]
fn i386_bound_below_raises_br() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    // Bounds: lower=0x0010, upper=0x00FF
    write_word_at(&mut bus, PM_DATA_BASE, 0x0010);
    write_word_at(&mut bus, PM_DATA_BASE + 2, 0x00FF);

    // AX = 0x0005 (below lower bound)
    cpu.state.set_eax(0x0005);

    // 62 06 00 00 = BOUND AX, [0x0000]
    place_at(&mut bus, PM_CODE_BASE, &[0x62, 0x06, 0x00, 0x00]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_BR_HANDLER_IP as u32 + 1);
}

#[test]
fn i386_bound_above_raises_br() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    // Bounds: lower=0x0000, upper=0x00FF
    write_word_at(&mut bus, PM_DATA_BASE, 0x0000);
    write_word_at(&mut bus, PM_DATA_BASE + 2, 0x00FF);

    // AX = 0x0100 (above upper bound)
    cpu.state.set_eax(0x0100);

    // 62 06 00 00 = BOUND AX, [0x0000]
    place_at(&mut bus, PM_CODE_BASE, &[0x62, 0x06, 0x00, 0x00]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_BR_HANDLER_IP as u32 + 1);
}

#[test]
fn i386_tf_single_step_raises_db() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    // Set TF=1
    cpu.state.flags.tf = true;

    // 90 = NOP
    place_at(&mut bus, PM_CODE_BASE, &[0x90]);

    cpu.step(&mut bus); // NOP - after execution, TF fires #DB
    cpu.step(&mut bus); // HLT in handler

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_DB_HANDLER_IP as u32 + 1);
}

#[test]
fn i386_tf_cleared_in_handler() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    cpu.state.flags.tf = true;

    // 90 = NOP
    place_at(&mut bus, PM_CODE_BASE, &[0x90]);

    cpu.step(&mut bus); // NOP
    cpu.step(&mut bus); // HLT in handler

    assert!(cpu.halted());
    assert!(
        !cpu.state.flags.tf,
        "TF should be cleared in the #DB handler"
    );
}

#[test]
fn i386_tf_set_in_pushed_eflags() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    cpu.state.flags.tf = true;

    // 90 = NOP
    place_at(&mut bus, PM_CODE_BASE, &[0x90]);

    cpu.step(&mut bus); // NOP
    cpu.step(&mut bus); // HLT in handler

    assert!(cpu.halted());

    // Stack: EIP, CS, EFLAGS (pushed by interrupt gate, 32-bit pushes)
    let sp = cpu.esp();
    let pushed_eflags = read_dword_at(&bus, PM_STACK_BASE + sp + 8);
    assert_ne!(pushed_eflags & 0x0100, 0, "Pushed EFLAGS should have TF=1");
}

#[test]
fn i386_movsb_forward() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    // Source byte at DS:0x100
    bus.ram[(PM_DATA_BASE + 0x100) as usize] = 0xAB;
    cpu.state.set_esi(0x0100);
    cpu.state.set_edi(0x0200);
    cpu.state.flags.df = false; // DF=0 -> forward

    // A4 = MOVSB, F4 = HLT
    place_at(&mut bus, PM_CODE_BASE, &[0xA4, 0xF4]);

    cpu.step(&mut bus); // MOVSB
    cpu.step(&mut bus); // HLT

    assert!(cpu.halted());
    assert_eq!(bus.ram[(PM_DATA_BASE + 0x200) as usize], 0xAB);
    assert_eq!(cpu.state.esi(), 0x0101);
    assert_eq!(cpu.state.edi(), 0x0201);
}

#[test]
fn i386_movsb_backward() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    bus.ram[(PM_DATA_BASE + 0x100) as usize] = 0xCD;
    cpu.state.set_esi(0x0100);
    cpu.state.set_edi(0x0200);
    cpu.state.flags.df = true; // DF=1 -> backward

    place_at(&mut bus, PM_CODE_BASE, &[0xA4, 0xF4]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(bus.ram[(PM_DATA_BASE + 0x200) as usize], 0xCD);
    assert_eq!(cpu.state.esi(), 0x00FF);
    assert_eq!(cpu.state.edi(), 0x01FF);
}

#[test]
fn i386_rep_movsb() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    let src = 0x0100u32;
    let dst = 0x0200u32;
    let count = 5u32;

    for i in 0..count {
        bus.ram[(PM_DATA_BASE + src + i) as usize] = (0x10 + i) as u8;
    }

    cpu.state.set_esi(src);
    cpu.state.set_edi(dst);
    cpu.state.set_ecx(count);
    cpu.state.flags.df = false;

    // F3 A4 = REP MOVSB, F4 = HLT
    place_at(&mut bus, PM_CODE_BASE, &[0xF3, 0xA4, 0xF4]);

    for _ in 0..20 {
        cpu.step(&mut bus);
        if cpu.halted() {
            break;
        }
    }

    assert!(cpu.halted());
    for i in 0..count {
        assert_eq!(
            bus.ram[(PM_DATA_BASE + dst + i) as usize],
            (0x10 + i) as u8,
            "REP MOVSB: byte {i} should match"
        );
    }
    assert_eq!(cpu.state.ecx(), 0);
    assert_eq!(cpu.state.esi(), src + count);
    assert_eq!(cpu.state.edi(), dst + count);
}

#[test]
fn i386_rep_movsw() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    let src = 0x0100u32;
    let dst = 0x0200u32;
    let count = 3u32;

    for i in 0..count {
        write_word_at(&mut bus, PM_DATA_BASE + src + i * 2, (0xAA00 + i) as u16);
    }

    cpu.state.set_esi(src);
    cpu.state.set_edi(dst);
    cpu.state.set_ecx(count);
    cpu.state.flags.df = false;

    // F3 A5 = REP MOVSW, F4 = HLT
    place_at(&mut bus, PM_CODE_BASE, &[0xF3, 0xA5, 0xF4]);

    for _ in 0..20 {
        cpu.step(&mut bus);
        if cpu.halted() {
            break;
        }
    }

    assert!(cpu.halted());
    for i in 0..count {
        assert_eq!(
            read_word_at(&bus, PM_DATA_BASE + dst + i * 2),
            (0xAA00 + i) as u16,
            "REP MOVSW: word {i} should match"
        );
    }
    assert_eq!(cpu.state.ecx(), 0);
}

#[test]
fn i386_rep_movsd() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    let src = 0x0100u32;
    let dst = 0x0200u32;
    let count = 2u32;

    write_dword_at(&mut bus, PM_DATA_BASE + src, 0xDEAD_BEEF);
    write_dword_at(&mut bus, PM_DATA_BASE + src + 4, 0xCAFE_BABE);

    cpu.state.set_esi(src);
    cpu.state.set_edi(dst);
    cpu.state.set_ecx(count);
    cpu.state.flags.df = false;

    // 66 prefix for 32-bit operand: F3 66 A5 = REP MOVSD, F4 = HLT
    place_at(&mut bus, PM_CODE_BASE, &[0xF3, 0x66, 0xA5, 0xF4]);

    for _ in 0..20 {
        cpu.step(&mut bus);
        if cpu.halted() {
            break;
        }
    }

    assert!(cpu.halted());
    assert_eq!(read_dword_at(&bus, PM_DATA_BASE + dst), 0xDEAD_BEEF);
    assert_eq!(read_dword_at(&bus, PM_DATA_BASE + dst + 4), 0xCAFE_BABE);
    assert_eq!(cpu.state.ecx(), 0);
}

#[test]
fn i386_page_faulting_moffs_load_preserves_destination_register() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let mut state = setup_protected_mode_with_exception_handlers(&mut bus);
    enable_identity_paging(&mut bus, &mut state);

    let source_offset = 0x3000u32;
    let source_linear = PM_DATA_BASE + source_offset;
    unmap_identity_page(&mut bus, source_linear);

    cpu.load_state(&state);
    cpu.state.set_eax(0x89AB_CDEF);

    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[
            0x67,
            0x66,
            0xA1,
            source_offset as u8,
            (source_offset >> 8) as u8,
            (source_offset >> 16) as u8,
            (source_offset >> 24) as u8,
        ],
    );

    cpu.step(&mut bus);

    assert_eq!(cpu.state.eax(), 0x89AB_CDEF);
    assert_eq!(cpu.ip(), PM_PAGE_FAULT_HANDLER_IP as u32);
    assert_eq!(cpu.cr2, source_linear);

    let stack_pointer = cpu.esp();
    assert_eq!(read_dword_at(&bus, PM_STACK_BASE + stack_pointer), 0);
    assert_eq!(read_dword_at(&bus, PM_STACK_BASE + stack_pointer + 4), 0);
    assert_eq!(
        read_dword_at(&bus, PM_STACK_BASE + stack_pointer + 8) as u16,
        PM_CS_SEL
    );
}

#[test]
fn i386_rep_movsd_source_page_fault_preserves_restart_state() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let mut state = setup_protected_mode_with_exception_handlers(&mut bus);
    enable_identity_paging(&mut bus, &mut state);

    let source_offset = 0x0FFCu32;
    let source_fault_linear = PM_DATA_BASE + 0x1000;
    let destination_offset = 0x2000u32;
    let destination_second_value = 0xA5A5_A5A5;

    write_dword_at(&mut bus, PM_DATA_BASE + source_offset, 0xDEAD_BEEF);
    write_dword_at(
        &mut bus,
        PM_DATA_BASE + destination_offset + 4,
        destination_second_value,
    );
    unmap_identity_page(&mut bus, source_fault_linear);

    cpu.load_state(&state);
    cpu.state.set_esi(source_offset);
    cpu.state.set_edi(destination_offset);
    cpu.state.set_ecx(2);
    cpu.state.flags.df = false;

    // REP MOVSD with 32-bit address and operand size in a 16-bit code segment.
    place_at(&mut bus, PM_CODE_BASE, &[0xF3, 0x67, 0x66, 0xA5, 0xF4]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_PAGE_FAULT_HANDLER_IP as u32 + 1);
    assert_eq!(cpu.cr2, source_fault_linear);
    assert_eq!(
        read_dword_at(&bus, PM_DATA_BASE + destination_offset),
        0xDEAD_BEEF
    );
    assert_eq!(
        read_dword_at(&bus, PM_DATA_BASE + destination_offset + 4),
        destination_second_value
    );
    assert_eq!(cpu.state.ecx(), 1);
    assert_eq!(cpu.state.esi(), 0x1000);
    assert_eq!(cpu.state.edi(), destination_offset + 4);
}

#[test]
fn i386_resumed_rep_movsd_source_page_fault_restarts_at_prefix() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let mut state = setup_protected_mode_with_exception_handlers(&mut bus);
    enable_identity_paging(&mut bus, &mut state);

    let source_offset = 0x0FFCu32;
    let source_fault_linear = PM_DATA_BASE + 0x1000;
    let destination_offset = 0x2000u32;
    let destination_second_value = 0x5A5A_5A5A;

    write_dword_at(&mut bus, PM_DATA_BASE + source_offset, 0xDEAD_BEEF);
    write_dword_at(
        &mut bus,
        PM_DATA_BASE + destination_offset + 4,
        destination_second_value,
    );
    unmap_identity_page(&mut bus, source_fault_linear);

    cpu.load_state(&state);
    cpu.state.set_esi(source_offset);
    cpu.state.set_edi(destination_offset);
    cpu.state.set_ecx(2);
    cpu.state.flags.df = false;

    // REP MOVSD with 32-bit address and operand size in a 16-bit code segment.
    place_at(&mut bus, PM_CODE_BASE, &[0xF3, 0x67, 0x66, 0xA5, 0xF4]);

    let _ = cpu.run_for(1, &mut bus);

    assert_eq!(cpu.ip(), 0x0000);
    assert_eq!(cpu.state.ecx(), 1);
    assert_eq!(
        read_dword_at(&bus, PM_DATA_BASE + destination_offset),
        0xDEAD_BEEF
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_PAGE_FAULT_HANDLER_IP as u32 + 1);
    assert_eq!(cpu.cr2, source_fault_linear);
    assert_eq!(
        read_dword_at(&bus, PM_DATA_BASE + destination_offset + 4),
        destination_second_value
    );
    assert_eq!(cpu.state.ecx(), 1);
    assert_eq!(cpu.state.esi(), 0x1000);
    assert_eq!(cpu.state.edi(), destination_offset + 4);

    let stack_pointer = cpu.esp();
    assert_eq!(read_dword_at(&bus, PM_STACK_BASE + stack_pointer), 0);
    assert_eq!(
        read_dword_at(&bus, PM_STACK_BASE + stack_pointer + 4),
        0,
        "page fault should restart at the REP prefix"
    );
    assert_eq!(
        read_dword_at(&bus, PM_STACK_BASE + stack_pointer + 8) as u16,
        PM_CS_SEL
    );
}

#[test]
fn i386_resumed_rep_movsd_fault_preserves_high_eip() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let mut state = setup_protected_mode_with_exception_handlers(&mut bus);
    enable_identity_paging(&mut bus, &mut state);

    state.seg_granularity[cpu::SegReg32::CS as usize] = 0xC0;
    state.seg_limits[cpu::SegReg32::CS as usize] = 0xFFFF_FFFF;

    let code_offset: u32 = 0x0001_0000;
    let source_offset: u32 = 0x0FFC;
    let source_fault_linear = PM_DATA_BASE + 0x1000;
    let destination_offset: u32 = 0x2000;

    write_dword_at(&mut bus, PM_DATA_BASE + source_offset, 0xDEAD_BEEF);
    unmap_identity_page(&mut bus, source_fault_linear);

    place_at(&mut bus, PM_CODE_BASE + code_offset, &[0xF3, 0xA5, 0xF4]);

    state.set_eip(code_offset);

    cpu.load_state(&state);
    cpu.state.set_esi(source_offset);
    cpu.state.set_edi(destination_offset);
    cpu.state.set_ecx(2);
    cpu.state.flags.df = false;

    let _ = cpu.run_for(1, &mut bus);
    assert_eq!(cpu.state.ecx(), 1);
    // Per i486 PRM section 3.6: a paused REP leaves EIP at the prefix.
    // F3 A5 (prefix + MOVSD opcode) starts at code_offset, so EIP must
    // point at code_offset (and preserve the upper 16 bits), not past it.
    assert_eq!(cpu.ip(), code_offset);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_PAGE_FAULT_HANDLER_IP as u32 + 1);
    assert_eq!(cpu.cr2, source_fault_linear);

    let stack_pointer = cpu.esp();
    assert_eq!(read_dword_at(&bus, PM_STACK_BASE + stack_pointer), 0);
    assert_eq!(
        read_dword_at(&bus, PM_STACK_BASE + stack_pointer + 4),
        code_offset,
        "resumed REP fault must restart at the REP prefix with upper EIP bits preserved"
    );
    assert_eq!(
        read_dword_at(&bus, PM_STACK_BASE + stack_pointer + 8) as u16,
        PM_CS_SEL
    );
}

#[test]
fn i386_stosb() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    cpu.state.set_eax(0x42);
    cpu.state.set_edi(0x0300);
    cpu.state.flags.df = false;

    // AA = STOSB, F4 = HLT
    place_at(&mut bus, PM_CODE_BASE, &[0xAA, 0xF4]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(bus.ram[(PM_DATA_BASE + 0x300) as usize], 0x42);
    assert_eq!(cpu.state.edi(), 0x0301);
}

#[test]
fn i386_rep_stosb() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    cpu.state.set_eax(0xFF);
    cpu.state.set_edi(0x0300);
    cpu.state.set_ecx(4);
    cpu.state.flags.df = false;

    // F3 AA = REP STOSB, F4 = HLT
    place_at(&mut bus, PM_CODE_BASE, &[0xF3, 0xAA, 0xF4]);

    for _ in 0..20 {
        cpu.step(&mut bus);
        if cpu.halted() {
            break;
        }
    }

    assert!(cpu.halted());
    for i in 0..4u32 {
        assert_eq!(bus.ram[(PM_DATA_BASE + 0x300 + i) as usize], 0xFF);
    }
    assert_eq!(cpu.state.ecx(), 0);
    assert_eq!(cpu.state.edi(), 0x0304);
}

#[test]
fn i386_rep_stosd_destination_page_fault_preserves_restart_state() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let mut state = setup_protected_mode_with_exception_handlers(&mut bus);
    enable_identity_paging(&mut bus, &mut state);

    let destination_offset = 0x0FFCu32;
    let destination_fault_linear = PM_DATA_BASE + 0x1000;
    let destination_second_value = 0x5A5A_5A5A;

    write_dword_at(
        &mut bus,
        PM_DATA_BASE + destination_offset + 4,
        destination_second_value,
    );
    unmap_identity_page(&mut bus, destination_fault_linear);

    cpu.load_state(&state);
    cpu.state.set_eax(0xCAFE_BABE);
    cpu.state.set_edi(destination_offset);
    cpu.state.set_ecx(2);
    cpu.state.flags.df = false;

    // REP STOSD with 32-bit address and operand size in a 16-bit code segment.
    place_at(&mut bus, PM_CODE_BASE, &[0xF3, 0x67, 0x66, 0xAB, 0xF4]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_PAGE_FAULT_HANDLER_IP as u32 + 1);
    assert_eq!(cpu.cr2, destination_fault_linear);
    assert_eq!(
        read_dword_at(&bus, PM_DATA_BASE + destination_offset),
        0xCAFE_BABE
    );
    assert_eq!(
        read_dword_at(&bus, PM_DATA_BASE + destination_offset + 4),
        destination_second_value
    );
    assert_eq!(cpu.state.ecx(), 1);
    assert_eq!(cpu.state.edi(), 0x1000);
}

#[test]
fn i386_lodsb() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    bus.ram[(PM_DATA_BASE + 0x100) as usize] = 0x77;
    cpu.state.set_esi(0x0100);
    cpu.state.set_eax(0x0000);
    cpu.state.flags.df = false;

    // AC = LODSB, F4 = HLT
    place_at(&mut bus, PM_CODE_BASE, &[0xAC, 0xF4]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.al(), 0x77);
    assert_eq!(cpu.state.esi(), 0x0101);
}

#[test]
fn i386_scasb_equal() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    bus.ram[(PM_DATA_BASE + 0x200) as usize] = 0x42;
    cpu.state.set_eax(0x42);
    cpu.state.set_edi(0x0200);
    cpu.state.flags.df = false;

    // AE = SCASB, F4 = HLT
    place_at(&mut bus, PM_CODE_BASE, &[0xAE, 0xF4]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert!(
        cpu.state.flags.zf(),
        "SCASB should set ZF when AL matches ES:EDI byte"
    );
    assert_eq!(cpu.state.edi(), 0x0201);
}

#[test]
fn i386_repne_scasb() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    // Place string "ABCDE" at ES:0x200
    let data = b"ABCDE";
    for (i, &b) in data.iter().enumerate() {
        bus.ram[(PM_DATA_BASE + 0x200 + i as u32) as usize] = b;
    }

    cpu.state.set_eax(b'C' as u32); // Search for 'C'
    cpu.state.set_edi(0x0200);
    cpu.state.set_ecx(5);
    cpu.state.flags.df = false;

    // F2 AE = REPNE SCASB, F4 = HLT
    place_at(&mut bus, PM_CODE_BASE, &[0xF2, 0xAE, 0xF4]);

    for _ in 0..20 {
        cpu.step(&mut bus);
        if cpu.halted() {
            break;
        }
    }

    assert!(cpu.halted());
    assert!(
        cpu.state.flags.zf(),
        "REPNE SCASB should find 'C' and set ZF"
    );
    assert_eq!(
        cpu.state.edi(),
        0x0203,
        "EDI should point past the found byte"
    );
    assert_eq!(cpu.state.ecx(), 2, "ECX should reflect remaining count");
}

#[test]
fn i386_repe_cmpsb() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    // Source string at DS:0x100 = "ABXDE"
    // Compare string at ES:0x200 = "ABCDE"
    let src = b"ABXDE";
    let cmp = b"ABCDE";
    for (i, &b) in src.iter().enumerate() {
        bus.ram[(PM_DATA_BASE + 0x100 + i as u32) as usize] = b;
    }
    for (i, &b) in cmp.iter().enumerate() {
        bus.ram[(PM_DATA_BASE + 0x200 + i as u32) as usize] = b;
    }

    cpu.state.set_esi(0x0100);
    cpu.state.set_edi(0x0200);
    cpu.state.set_ecx(5);
    cpu.state.flags.df = false;

    // F3 A6 = REPE CMPSB, F4 = HLT
    place_at(&mut bus, PM_CODE_BASE, &[0xF3, 0xA6, 0xF4]);

    for _ in 0..20 {
        cpu.step(&mut bus);
        if cpu.halted() {
            break;
        }
    }

    assert!(cpu.halted());
    assert!(
        !cpu.state.flags.zf(),
        "REPE CMPSB should clear ZF on mismatch"
    );
    assert_eq!(
        cpu.state.ecx(),
        2,
        "ECX should reflect remaining count after mismatch at index 2"
    );
    assert_eq!(cpu.state.esi(), 0x0103);
    assert_eq!(cpu.state.edi(), 0x0203);
}

#[test]
fn i386_load_null_ds_succeeds() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    // B8 00 00 = MOV AX, 0x0000
    // 8E D8 = MOV DS, AX
    // F4 = HLT
    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[0xB8, 0x00, 0x00, 0x8E, 0xD8, 0xF4],
    );

    cpu.step(&mut bus); // MOV AX, 0
    cpu.step(&mut bus); // MOV DS, AX
    cpu.step(&mut bus); // HLT

    assert!(cpu.halted());
    assert_eq!(
        cpu.ip(),
        0x0006,
        "Loading null selector into DS at CPL=0 should succeed"
    );
}

#[test]
fn i386_load_null_fs_succeeds() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    // B8 00 00 = MOV AX, 0x0000
    // 8E E0 = MOV FS, AX
    // F4 = HLT
    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[0xB8, 0x00, 0x00, 0x8E, 0xE0, 0xF4],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(
        cpu.ip(),
        0x0006,
        "Loading null selector into FS should succeed"
    );
}

#[test]
fn i386_load_null_gs_succeeds() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    // B8 00 00 = MOV AX, 0x0000
    // 8E E8 = MOV GS, AX
    // F4 = HLT
    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[0xB8, 0x00, 0x00, 0x8E, 0xE8, 0xF4],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(
        cpu.ip(),
        0x0006,
        "Loading null selector into GS should succeed"
    );
}

#[test]
fn i386_load_null_ss_at_cpl0_raises_gp() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    // B8 00 00 = MOV AX, 0x0000
    // 8E D0 = MOV SS, AX
    place_at(&mut bus, PM_CODE_BASE, &[0xB8, 0x00, 0x00, 0x8E, 0xD0]);

    cpu.step(&mut bus); // MOV AX, 0
    cpu.step(&mut bus); // MOV SS, AX -> #GP
    cpu.step(&mut bus); // HLT in GP handler

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_GP_HANDLER_IP as u32 + 1);
}

#[test]
fn i386_access_via_null_ds_raises_gp() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    // Load null into DS, then try to read through it
    // B8 00 00 = MOV AX, 0x0000
    // 8E D8 = MOV DS, AX
    // A0 00 00 = MOV AL, [0x0000] (uses DS)
    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[0xB8, 0x00, 0x00, 0x8E, 0xD8, 0xA0, 0x00, 0x00],
    );

    cpu.step(&mut bus); // MOV AX, 0
    cpu.step(&mut bus); // MOV DS, AX
    cpu.step(&mut bus); // MOV AL, [0x0000] -> #GP (null DS)
    cpu.step(&mut bus); // HLT in GP handler

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_GP_HANDLER_IP as u32 + 1);
}

#[test]
fn i386_lea_reg_indirect() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    cpu.state.set_ebx(0x1234);

    // 8D 03 = LEA AX, [BP+DI] in 16-bit - but we need LEA AX, [BX]
    // In 16-bit mode: LEA AX, [BX] = 8D 07
    // F4 = HLT
    place_at(&mut bus, PM_CODE_BASE, &[0x8D, 0x07, 0xF4]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    // 16-bit addressing: LEA AX, [BX] uses BX (low 16 bits of EBX)
    assert_eq!(cpu.eax() & 0xFFFF, 0x1234);
}

#[test]
fn i386_lea_reg_plus_disp8() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    cpu.state.set_ebx(0x1000);

    // 16-bit mode: LEA AX, [BX+0x10] = 8D 47 10
    // F4 = HLT
    place_at(&mut bus, PM_CODE_BASE, &[0x8D, 0x47, 0x10, 0xF4]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.eax() & 0xFFFF, 0x1010);
}

#[test]
fn i386_lea_reg_plus_disp16() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    cpu.state.set_ebx(0x1000);

    // 16-bit mode: LEA AX, [BX+0x2000] = 8D 87 00 20
    // F4 = HLT
    place_at(&mut bus, PM_CODE_BASE, &[0x8D, 0x87, 0x00, 0x20, 0xF4]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.eax() & 0xFFFF, 0x3000);
}

#[test]
fn i386_lea_sib_base_index() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    cpu.state.set_ebx(0x1000);
    cpu.state.set_ecx(0x0200);

    // 67 prefix for 32-bit addressing in 16-bit code segment:
    // 67 8D 04 0B = LEA AX, [EBX+ECX] (SIB: scale=0, index=ECX, base=EBX)
    // F4 = HLT
    place_at(&mut bus, PM_CODE_BASE, &[0x67, 0x8D, 0x04, 0x0B, 0xF4]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.eax() & 0xFFFF, 0x1200);
}

#[test]
fn i386_lea_sib_scale_2() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    cpu.state.set_ebx(0x1000);
    cpu.state.set_ecx(0x0100);

    // 67 8D 04 4B = LEA AX, [EBX+ECX*2] (SIB: scale=1, index=ECX, base=EBX)
    // F4 = HLT
    place_at(&mut bus, PM_CODE_BASE, &[0x67, 0x8D, 0x04, 0x4B, 0xF4]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.eax() & 0xFFFF, 0x1200);
}

#[test]
fn i386_lea_sib_scale_4() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    cpu.state.set_ebx(0x1000);
    cpu.state.set_ecx(0x0040);

    // 67 8D 04 8B = LEA AX, [EBX+ECX*4] (SIB: scale=2, index=ECX, base=EBX)
    // F4 = HLT
    place_at(&mut bus, PM_CODE_BASE, &[0x67, 0x8D, 0x04, 0x8B, 0xF4]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.eax() & 0xFFFF, 0x1100);
}

#[test]
fn i386_lea_sib_scale_8() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    cpu.state.set_ebx(0x1000);
    cpu.state.set_ecx(0x0020);

    // 67 8D 04 CB = LEA AX, [EBX+ECX*8] (SIB: scale=3, index=ECX, base=EBX)
    // F4 = HLT
    place_at(&mut bus, PM_CODE_BASE, &[0x67, 0x8D, 0x04, 0xCB, 0xF4]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.eax() & 0xFFFF, 0x1100);
}

#[test]
fn i386_lea_disp32_only() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    // 67 8D 05 78 56 34 12 = LEA AX, [0x12345678] (ModRM: mod=00, rm=101 -> disp32)
    // F4 = HLT
    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[0x67, 0x8D, 0x05, 0x78, 0x56, 0x34, 0x12, 0xF4],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.eax() & 0xFFFF, 0x5678);
}

#[test]
fn i386_xchg_reg_reg() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    cpu.state.set_eax(0x1111);
    cpu.state.set_ebx(0x2222);

    // 93 = XCHG AX, BX (short form: 90+reg)
    // F4 = HLT
    place_at(&mut bus, PM_CODE_BASE, &[0x93, 0xF4]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.eax() & 0xFFFF, 0x2222);
    assert_eq!(cpu.state.ebx() & 0xFFFF, 0x1111);
}

#[test]
fn i386_xchg_reg_mem() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    cpu.state.set_eax(0x00AA);
    write_word_at(&mut bus, PM_DATA_BASE + 0x50, 0x00BB);

    // 87 06 50 00 = XCHG AX, [0x0050]
    // F4 = HLT
    place_at(&mut bus, PM_CODE_BASE, &[0x87, 0x06, 0x50, 0x00, 0xF4]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.eax() & 0xFFFF, 0x00BB);
    assert_eq!(read_word_at(&bus, PM_DATA_BASE + 0x50), 0x00AA);
}

#[test]
fn i386_lock_jcc_near_raises_ud() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    // F0 0F 80 00 00 = LOCK JO near (rel16=0)
    place_at(&mut bus, PM_CODE_BASE, &[0xF0, 0x0F, 0x80, 0x00, 0x00]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_UD_HANDLER_IP as u32 + 1);
}

#[test]
fn i386_lock_movzx_raises_ud() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    // F0 0F B6 06 50 00 = LOCK MOVZX AX, byte [0x0050]
    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[0xF0, 0x0F, 0xB6, 0x06, 0x50, 0x00],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_UD_HANDLER_IP as u32 + 1);
}

#[test]
fn i386_lock_bt_reg_raises_ud() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    // F0 0F A3 06 50 00 = LOCK BT [0x0050], AX
    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[0xF0, 0x0F, 0xA3, 0x06, 0x50, 0x00],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_UD_HANDLER_IP as u32 + 1);
}

#[test]
fn i386_lock_bt_imm_raises_ud() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    // F0 0F BA 26 50 00 00 = LOCK BT [0x0050], 0 (group BA, reg=4)
    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[0xF0, 0x0F, 0xBA, 0x26, 0x50, 0x00, 0x00],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_UD_HANDLER_IP as u32 + 1);
}

#[test]
fn i386_lock_cmp_imm_raises_ud() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    // F0 83 3E 50 00 00 = LOCK CMP word [0x0050], 0 (group 83, reg=7)
    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[0xF0, 0x83, 0x3E, 0x50, 0x00, 0x00],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_UD_HANDLER_IP as u32 + 1);
}

#[test]
fn i386_lock_test_raises_ud() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    // F0 F6 06 50 00 01 = LOCK TEST byte [0x0050], 0x01 (group F6, reg=0)
    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[0xF0, 0xF6, 0x06, 0x50, 0x00, 0x01],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_UD_HANDLER_IP as u32 + 1);
}

#[test]
fn i386_lock_mul_raises_ud() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    // F0 F6 26 50 00 = LOCK MUL byte [0x0050] (group F6, reg=4)
    place_at(&mut bus, PM_CODE_BASE, &[0xF0, 0xF6, 0x26, 0x50, 0x00]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_UD_HANDLER_IP as u32 + 1);
}

#[test]
fn i386_lock_call_indirect_raises_ud() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    // F0 FF 16 50 00 = LOCK CALL [0x0050] (group FF, reg=2)
    place_at(&mut bus, PM_CODE_BASE, &[0xF0, 0xFF, 0x16, 0x50, 0x00]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_UD_HANDLER_IP as u32 + 1);
}

#[test]
fn i386_lock_add_mem_executes() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    // F0 01 06 50 00 = LOCK ADD [0x0050], AX
    // F4 = HLT
    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[0xF0, 0x01, 0x06, 0x50, 0x00, 0xF4],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_ne!(cpu.ip(), PM_UD_HANDLER_IP as u32 + 1);
}

#[test]
fn i386_lock_bts_mem_executes() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    // F0 0F AB 06 50 00 = LOCK BTS [0x0050], AX
    // F4 = HLT
    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[0xF0, 0x0F, 0xAB, 0x06, 0x50, 0x00, 0xF4],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_ne!(cpu.ip(), PM_UD_HANDLER_IP as u32 + 1);
}

#[test]
fn i386_lock_not_mem_executes() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_exception_handlers(&mut bus);
    cpu.load_state(&state);

    // F0 F6 16 50 00 = LOCK NOT byte [0x0050] (group F6, reg=2)
    // F4 = HLT
    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[0xF0, 0xF6, 0x16, 0x50, 0x00, 0xF4],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_ne!(cpu.ip(), PM_UD_HANDLER_IP as u32 + 1);
}

#[test]
fn iopb_word_io_checks_two_consecutive_bits() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.set_cs(PM_CS_SEL | 3);
    state.flags.iopl = 0;

    const TSS_BASE: u32 = 0xC0000;
    const TSS_SEL: u16 = 0x0028;
    write_gdt_entry16(&mut bus, PM_GDT_BASE, 5, TSS_BASE, 0x6A, 0x89);
    state.gdt_limit = 6 * 8 - 1;

    write_dword_at(&mut bus, TSS_BASE + 0x04, 0xFF00);
    write_word_at(&mut bus, TSS_BASE + 0x08, PM_SS_SEL);
    write_word_at(&mut bus, TSS_BASE + 0x66, 0x0068);
    // bit 1 set -> port 1 denied; word IN from port 0 checks ports 0 and 1
    bus.ram[(TSS_BASE + 0x68) as usize] = 0x02;

    state.tr = TSS_SEL;
    state.tr_base = TSS_BASE;
    state.tr_limit = 0x6A;
    state.tr_rights = 0x89;
    cpu.load_state(&state);

    // E5 00 = IN AX, 0 (word I/O, checks ports 0-1)
    place_at(&mut bus, PM_CODE_BASE, &[0xE5, 0x00]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    assert!(
        cpu.halted(),
        "IN AX,0 must raise #GP when port 1 is denied in IOPB"
    );
    assert_eq!(cpu.ip(), PM_GP_HANDLER_IP as u32 + 1);
}

#[test]
fn iopb_dword_io_checks_four_consecutive_bits() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.set_cs(PM_CS_SEL | 3);
    state.flags.iopl = 0;

    const TSS_BASE: u32 = 0xC0000;
    const TSS_SEL: u16 = 0x0028;
    write_gdt_entry16(&mut bus, PM_GDT_BASE, 5, TSS_BASE, 0x6A, 0x89);
    state.gdt_limit = 6 * 8 - 1;

    write_dword_at(&mut bus, TSS_BASE + 0x04, 0xFF00);
    write_word_at(&mut bus, TSS_BASE + 0x08, PM_SS_SEL);
    write_word_at(&mut bus, TSS_BASE + 0x66, 0x0068);
    // bit 3 set -> port 3 denied; dword IN from port 0 checks ports 0-3
    bus.ram[(TSS_BASE + 0x68) as usize] = 0x08;

    state.tr = TSS_SEL;
    state.tr_base = TSS_BASE;
    state.tr_limit = 0x6A;
    state.tr_rights = 0x89;
    cpu.load_state(&state);

    // 66 E5 00 = IN EAX, 0 (dword I/O, checks ports 0-3)
    place_at(&mut bus, PM_CODE_BASE, &[0x66, 0xE5, 0x00]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    assert!(
        cpu.halted(),
        "IN EAX,0 must raise #GP when port 3 is denied in IOPB"
    );
    assert_eq!(cpu.ip(), PM_GP_HANDLER_IP as u32 + 1);
}

#[test]
fn iopb_dword_io_checks_port_plus_one() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.set_cs(PM_CS_SEL | 3);
    state.flags.iopl = 0;

    const TSS_BASE: u32 = 0xC0000;
    const TSS_SEL: u16 = 0x0028;
    write_gdt_entry16(&mut bus, PM_GDT_BASE, 5, TSS_BASE, 0x6A, 0x89);
    state.gdt_limit = 6 * 8 - 1;

    write_dword_at(&mut bus, TSS_BASE + 0x04, 0xFF00);
    write_word_at(&mut bus, TSS_BASE + 0x08, PM_SS_SEL);
    write_word_at(&mut bus, TSS_BASE + 0x66, 0x0068);
    // only bit 1 set -> port 1 denied; dword IN from port 0 must check port 1 too
    bus.ram[(TSS_BASE + 0x68) as usize] = 0x02;

    state.tr = TSS_SEL;
    state.tr_base = TSS_BASE;
    state.tr_limit = 0x6A;
    state.tr_rights = 0x89;
    cpu.load_state(&state);

    // 66 E5 00 = IN EAX, 0 (dword I/O, checks ports 0-3)
    place_at(&mut bus, PM_CODE_BASE, &[0x66, 0xE5, 0x00]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    assert!(
        cpu.halted(),
        "IN EAX,0 must raise #GP when port 1 is denied (old code only checked port and port+2)"
    );
    assert_eq!(cpu.ip(), PM_GP_HANDLER_IP as u32 + 1);
}

#[test]
fn iopb_cross_byte_boundary() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.set_cs(PM_CS_SEL | 3);
    state.flags.iopl = 0;

    const TSS_BASE: u32 = 0xC0000;
    const TSS_SEL: u16 = 0x0028;
    write_gdt_entry16(&mut bus, PM_GDT_BASE, 5, TSS_BASE, 0x6B, 0x89);
    state.gdt_limit = 6 * 8 - 1;

    write_dword_at(&mut bus, TSS_BASE + 0x04, 0xFF00);
    write_word_at(&mut bus, TSS_BASE + 0x08, PM_SS_SEL);
    write_word_at(&mut bus, TSS_BASE + 0x66, 0x0068);
    // byte 0 (ports 0-7): all clear
    bus.ram[(TSS_BASE + 0x68) as usize] = 0x00;
    // byte 1 (ports 8-15): bit 0 set -> port 8 denied
    // Word IN from port 7 checks bit 7 of byte 0 and bit 0 of byte 1
    bus.ram[(TSS_BASE + 0x69) as usize] = 0x01;

    state.tr = TSS_SEL;
    state.tr_base = TSS_BASE;
    state.tr_limit = 0x6B;
    state.tr_rights = 0x89;
    cpu.load_state(&state);

    // E5 07 = IN AX, 7 (word I/O from port 7, crosses byte boundary)
    place_at(&mut bus, PM_CODE_BASE, &[0xE5, 0x07]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    assert!(
        cpu.halted(),
        "IN AX,7 must raise #GP when port 8 (bit 0 of second IOPB byte) is denied"
    );
    assert_eq!(cpu.ip(), PM_GP_HANDLER_IP as u32 + 1);
}

#[test]
fn iopb_word_io_allowed_when_bits_clear() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.set_cs(PM_CS_SEL | 3);
    state.flags.iopl = 0;

    const TSS_BASE: u32 = 0xC0000;
    const TSS_SEL: u16 = 0x0028;
    write_gdt_entry16(&mut bus, PM_GDT_BASE, 5, TSS_BASE, 0x6A, 0x89);
    state.gdt_limit = 6 * 8 - 1;

    write_dword_at(&mut bus, TSS_BASE + 0x04, 0xFF00);
    write_word_at(&mut bus, TSS_BASE + 0x08, PM_SS_SEL);
    write_word_at(&mut bus, TSS_BASE + 0x66, 0x0068);
    // all bits clear -> all ports allowed
    bus.ram[(TSS_BASE + 0x68) as usize] = 0x00;

    state.tr = TSS_SEL;
    state.tr_base = TSS_BASE;
    state.tr_limit = 0x6A;
    state.tr_rights = 0x89;
    cpu.load_state(&state);

    // E5 00 = IN AX, 0 (word I/O, ports 0-1 both allowed)
    place_at(&mut bus, PM_CODE_BASE, &[0xE5, 0x00]);

    cpu.step(&mut bus);

    assert!(
        !cpu.halted(),
        "IN AX,0 must succeed when both port bits are clear in IOPB"
    );
    assert_eq!(cpu.ip() as u16, 2, "IP must advance past 2-byte IN AX");
}

const VM86_UD_HANDLER_IP: u16 = 0x0200;

fn setup_vm86_with_ud_handler(bus: &mut TestBus) -> cpu::I386State {
    let mut state = setup_vm86(bus);

    write_idt_gate(
        bus,
        VM86_IDT_BASE,
        6,
        VM86_UD_HANDLER_IP as u32,
        VM86_CS_SEL,
        14,
        0,
    );
    bus.ram[(VM86_CODE_BASE + VM86_UD_HANDLER_IP as u32) as usize] = 0xF4;

    // Extend TSS limit to cover ESP0/SS0 properly for the stack switch
    state.tr_limit = 103;
    write_gdt_entry16(bus, VM86_GDT_BASE, 4, VM86_TSS_BASE, 103, 0x89);

    state
}

#[test]
fn vm86_arpl_raises_ud() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_vm86_with_ud_handler(&mut bus);
    cpu.load_state(&state);

    // 63 C0 = ARPL AX, AX
    place_code(&mut bus, 0x1000, 0x0000, &[0x63, 0xC0]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    assert!(cpu.halted());
    assert_eq!(cpu.ip(), VM86_UD_HANDLER_IP as u32 + 1);
}

#[test]
fn vm86_sldt_raises_ud() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_vm86_with_ud_handler(&mut bus);
    cpu.load_state(&state);

    // 0F 00 C0 = SLDT AX (modrm=C0: mod=11, reg=0, rm=0)
    place_code(&mut bus, 0x1000, 0x0000, &[0x0F, 0x00, 0xC0]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    assert!(cpu.halted());
    assert_eq!(cpu.ip(), VM86_UD_HANDLER_IP as u32 + 1);
}

#[test]
fn vm86_str_raises_ud() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_vm86_with_ud_handler(&mut bus);
    cpu.load_state(&state);

    // 0F 00 C8 = STR AX (modrm=C8: mod=11, reg=1, rm=0)
    place_code(&mut bus, 0x1000, 0x0000, &[0x0F, 0x00, 0xC8]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    assert!(cpu.halted());
    assert_eq!(cpu.ip(), VM86_UD_HANDLER_IP as u32 + 1);
}

#[test]
fn vm86_lldt_raises_ud() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_vm86_with_ud_handler(&mut bus);
    cpu.load_state(&state);

    // 0F 00 D0 = LLDT AX (modrm=D0: mod=11, reg=2, rm=0)
    place_code(&mut bus, 0x1000, 0x0000, &[0x0F, 0x00, 0xD0]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    assert!(cpu.halted());
    assert_eq!(cpu.ip(), VM86_UD_HANDLER_IP as u32 + 1);
}

#[test]
fn vm86_ltr_raises_ud() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_vm86_with_ud_handler(&mut bus);
    cpu.load_state(&state);

    // 0F 00 D8 = LTR AX (modrm=D8: mod=11, reg=3, rm=0)
    place_code(&mut bus, 0x1000, 0x0000, &[0x0F, 0x00, 0xD8]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    assert!(cpu.halted());
    assert_eq!(cpu.ip(), VM86_UD_HANDLER_IP as u32 + 1);
}

#[test]
fn vm86_verr_raises_ud() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_vm86_with_ud_handler(&mut bus);
    cpu.load_state(&state);

    // 0F 00 E0 = VERR AX (modrm=E0: mod=11, reg=4, rm=0)
    place_code(&mut bus, 0x1000, 0x0000, &[0x0F, 0x00, 0xE0]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    assert!(cpu.halted());
    assert_eq!(cpu.ip(), VM86_UD_HANDLER_IP as u32 + 1);
}

#[test]
fn vm86_verw_raises_ud() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_vm86_with_ud_handler(&mut bus);
    cpu.load_state(&state);

    // 0F 00 E8 = VERW AX (modrm=E8: mod=11, reg=5, rm=0)
    place_code(&mut bus, 0x1000, 0x0000, &[0x0F, 0x00, 0xE8]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    assert!(cpu.halted());
    assert_eq!(cpu.ip(), VM86_UD_HANDLER_IP as u32 + 1);
}

#[test]
fn vm86_lar_raises_ud() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_vm86_with_ud_handler(&mut bus);
    cpu.load_state(&state);

    // 0F 02 C0 = LAR AX, AX
    place_code(&mut bus, 0x1000, 0x0000, &[0x0F, 0x02, 0xC0]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    assert!(cpu.halted());
    assert_eq!(cpu.ip(), VM86_UD_HANDLER_IP as u32 + 1);
}

#[test]
fn vm86_lsl_raises_ud() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_vm86_with_ud_handler(&mut bus);
    cpu.load_state(&state);

    // 0F 03 C0 = LSL AX, AX
    place_code(&mut bus, 0x1000, 0x0000, &[0x0F, 0x03, 0xC0]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    assert!(cpu.halted());
    assert_eq!(cpu.ip(), VM86_UD_HANDLER_IP as u32 + 1);
}

#[test]
fn task_switch_tss_t_bit_raises_debug_trap() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_extended(&mut bus);
    cpu.load_state(&state);

    // Add #DB handler (vector 1) to IDT.
    write_idt_gate(
        &mut bus,
        PM_IDT_BASE,
        1,
        PM_DB_HANDLER_IP as u32,
        PM_CS_SEL,
        14,
        0,
    );
    bus.ram[(PM_CODE_BASE + PM_DB_HANDLER_IP as u32) as usize] = 0xF4;

    let target_ip: u16 = 0x0300;
    write_tss386(
        &mut bus,
        PM_TSS2_BASE,
        0,
        0xFFF0,
        PM_SS_SEL,
        target_ip as u32,
        0x0002,
        0,
        0,
        0,
        0,
        0xEE00,
        0,
        0,
        0,
        PM_DS_SEL,
        PM_CS_SEL,
        PM_SS_SEL,
        PM_DS_SEL,
        0,
        0,
        0,
    );
    // Set T-bit at TSS2 offset 100 (0x64), bit 0.
    write_word_at(&mut bus, PM_TSS2_BASE + 100, 0x0001);

    bus.ram[(PM_CODE_BASE + target_ip as u32) as usize] = 0xF4;

    // EA 00 00 48 00 = JMP FAR 0x0048:0x0000
    place_at(&mut bus, PM_CODE_BASE, &[0xEA, 0x00, 0x00, 0x48, 0x00]);

    cpu.step(&mut bus); // JMP FAR -> task switch, debug trap fires
    cpu.step(&mut bus); // HLT at #DB handler

    assert!(cpu.halted());
    assert_eq!(
        cpu.ip(),
        PM_DB_HANDLER_IP as u32 + 1,
        "should halt at #DB handler"
    );
    assert_ne!(
        cpu.dr6 & 0x8000,
        0,
        "DR6.BT (bit 15) must be set after T-bit debug trap"
    );
}

#[test]
fn task_switch_tss_t_bit_clear_no_debug_trap() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_extended(&mut bus);
    cpu.load_state(&state);

    // Add #DB handler just in case (should NOT be reached).
    write_idt_gate(
        &mut bus,
        PM_IDT_BASE,
        1,
        PM_DB_HANDLER_IP as u32,
        PM_CS_SEL,
        14,
        0,
    );
    bus.ram[(PM_CODE_BASE + PM_DB_HANDLER_IP as u32) as usize] = 0xF4;

    let target_ip: u16 = 0x0300;
    write_tss386(
        &mut bus,
        PM_TSS2_BASE,
        0,
        0xFFF0,
        PM_SS_SEL,
        target_ip as u32,
        0x0002,
        0,
        0,
        0,
        0,
        0xEE00,
        0,
        0,
        0,
        PM_DS_SEL,
        PM_CS_SEL,
        PM_SS_SEL,
        PM_DS_SEL,
        0,
        0,
        0,
    );
    // T-bit clear (default 0).

    bus.ram[(PM_CODE_BASE + target_ip as u32) as usize] = 0xF4;

    place_at(&mut bus, PM_CODE_BASE, &[0xEA, 0x00, 0x00, 0x48, 0x00]);

    cpu.step(&mut bus); // JMP FAR -> task switch, no debug trap
    cpu.step(&mut bus); // HLT at new task's first instruction

    assert!(cpu.halted());
    assert_eq!(
        cpu.ip(),
        target_ip as u32 + 1,
        "should halt at new task's first instruction, not #DB handler"
    );
    assert_eq!(
        cpu.dr6 & 0x8000,
        0,
        "DR6.BT must NOT be set when T-bit is clear"
    );
}

#[test]
fn task_switch_tss_t_bit_sets_dr6_bt() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_extended(&mut bus);
    cpu.load_state(&state);

    write_idt_gate(
        &mut bus,
        PM_IDT_BASE,
        1,
        PM_DB_HANDLER_IP as u32,
        PM_CS_SEL,
        14,
        0,
    );
    bus.ram[(PM_CODE_BASE + PM_DB_HANDLER_IP as u32) as usize] = 0xF4;

    let target_ip: u16 = 0x0300;
    write_tss386(
        &mut bus,
        PM_TSS2_BASE,
        0,
        0xFFF0,
        PM_SS_SEL,
        target_ip as u32,
        0x0002,
        0,
        0,
        0,
        0,
        0xEE00,
        0,
        0,
        0,
        PM_DS_SEL,
        PM_CS_SEL,
        PM_SS_SEL,
        PM_DS_SEL,
        0,
        0,
        0,
    );
    write_word_at(&mut bus, PM_TSS2_BASE + 100, 0x0001);
    bus.ram[(PM_CODE_BASE + target_ip as u32) as usize] = 0xF4;

    place_at(&mut bus, PM_CODE_BASE, &[0xEA, 0x00, 0x00, 0x48, 0x00]);

    // Clear DR6 lower bits to verify only BT gets set.
    cpu.dr6 = 0xFFFF_0FF0;

    cpu.step(&mut bus); // JMP FAR -> task switch + debug trap
    cpu.step(&mut bus); // HLT at #DB handler

    assert!(cpu.halted());
    assert_eq!(cpu.dr6 & 0x8000, 0x8000, "DR6.BT (bit 15) must be set");
    // B0-B3 (bits 0-3) should remain 0 since no breakpoint register matched.
    assert_eq!(
        cpu.dr6 & 0x000F,
        0,
        "DR6 B0-B3 bits should not be set for T-bit trap"
    );
}

/// Sets up VM86 mode with an IOPB that denies all ports and a #GP handler.
/// Returns the VM86 code physical address where test instructions should be placed.
fn setup_vm86_with_iopb(bus: &mut TestBus) -> (cpu::I386State, u32) {
    const GP_HANDLER_IP: u16 = 0x0200;

    write_gdt_entry16(bus, VM86_GDT_BASE, 0, 0, 0, 0);
    write_gdt_entry16(bus, VM86_GDT_BASE, 1, VM86_CODE_BASE, 0xFFFF, 0x9B);
    write_gdt_entry16(bus, VM86_GDT_BASE, 2, 0x00000, 0xFFFF, 0x93);
    write_gdt_entry(bus, VM86_GDT_BASE, 3, 0x00000, 0xFFFF, 0x93, 0x40);
    // 386 TSS (type=9): limit covers IOPB for ports up to 0x07FF.
    // IOPB at offset 0x68, port 0x07F0 is in byte 0xFE, need +1 for sentinel.
    write_gdt_entry16(bus, VM86_GDT_BASE, 4, VM86_TSS_BASE, 0x168, 0x89);

    // #GP handler (vector 13): 386 interrupt gate, DPL=0, targets CPL0 code.
    write_idt_gate(
        bus,
        VM86_IDT_BASE,
        13,
        GP_HANDLER_IP as u32,
        VM86_CS_SEL,
        14,
        0,
    );
    bus.ram[(VM86_CODE_BASE + GP_HANDLER_IP as u32) as usize] = 0xF4; // HLT

    // TSS: ESP0/SS0 for the VM86->CPL0 transition on #GP.
    write_dword_at(bus, VM86_TSS_BASE + 0x04, 0x1000);
    write_word_at(bus, VM86_TSS_BASE + 0x08, VM86_SS0_SEL);
    // IOPB pointer at TSS offset 0x66.
    write_word_at(bus, VM86_TSS_BASE + 0x66, 0x0068);
    // Fill IOPB with 0xFF (all ports denied) for bytes covering ports 0-0x07FF.
    for i in 0..0x100u32 {
        bus.ram[(VM86_TSS_BASE + 0x68 + i) as usize] = 0xFF;
    }

    let mut state = cpu::I386State {
        cr0: 0x0001,
        eflags_upper: 0x0002_0000,
        seg_valid: [true; 6],
        gdt_base: VM86_GDT_BASE,
        gdt_limit: 5 * 8 - 1,
        idt_base: VM86_IDT_BASE,
        idt_limit: 256 * 8 - 1,
        tr: 0x0020,
        tr_base: VM86_TSS_BASE,
        tr_limit: 0x168,
        tr_rights: 0x89,
        ..cpu::I386State::default()
    };
    state.flags.iopl = 3;

    state.set_cs(0x1000);
    state.seg_bases[cpu::SegReg32::CS as usize] = 0x10000;
    state.seg_limits[cpu::SegReg32::CS as usize] = 0xFFFF;
    state.seg_rights[cpu::SegReg32::CS as usize] = 0x9B;

    state.set_ss(0x2000);
    state.set_esp(0xF000);
    state.seg_bases[cpu::SegReg32::SS as usize] = 0x20000;
    state.seg_limits[cpu::SegReg32::SS as usize] = 0xFFFF;
    state.seg_rights[cpu::SegReg32::SS as usize] = 0x93;

    state.set_ds(0x4000);
    state.seg_bases[cpu::SegReg32::DS as usize] = 0x40000;
    state.seg_limits[cpu::SegReg32::DS as usize] = 0xFFFF;
    state.seg_rights[cpu::SegReg32::DS as usize] = 0x93;

    state.set_es(0x3000);
    state.seg_bases[cpu::SegReg32::ES as usize] = 0x30000;
    state.seg_limits[cpu::SegReg32::ES as usize] = 0xFFFF;
    state.seg_rights[cpu::SegReg32::ES as usize] = 0x93;

    // VM86 code at CS:0000 = physical 0x10000.
    (state, 0x10000)
}

#[test]
fn vm86_hle_port_bypass_allows_bios_trap_port() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    bus.hle_port_bypass = true;

    let (state, code_base) = setup_vm86_with_iopb(&mut bus);
    cpu.load_state(&state);

    // OUT DX, AL to BIOS HLE port 0x07F0 (IOPB denies it, but bypass allows).
    #[rustfmt::skip]
    place_at(&mut bus, code_base, &[
        0xBA, 0xF0, 0x07, // MOV DX, 0x07F0
        0xEE,             // OUT DX, AL
    ]);

    cpu.step(&mut bus); // MOV DX, 0x07F0
    cpu.step(&mut bus); // OUT DX, AL
    assert!(
        !cpu.halted(),
        "OUT to BIOS HLE port 0x07F0 must not raise #GP when bypass is enabled"
    );
    assert_eq!(cpu.ip(), 4, "IP should advance past both instructions");
}

#[test]
fn vm86_hle_port_bypass_allows_sasi_trap_port() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    bus.hle_port_bypass = true;

    let (state, code_base) = setup_vm86_with_iopb(&mut bus);
    cpu.load_state(&state);

    // OUT DX, AL to SASI HLE port 0x07EF.
    #[rustfmt::skip]
    place_at(&mut bus, code_base, &[
        0xBA, 0xEF, 0x07, // MOV DX, 0x07EF
        0xEE,             // OUT DX, AL
    ]);

    cpu.step(&mut bus); // MOV DX, 0x07EF
    cpu.step(&mut bus); // OUT DX, AL
    assert!(
        !cpu.halted(),
        "OUT to SASI HLE port 0x07EF must not raise #GP when bypass is enabled"
    );
    assert_eq!(cpu.ip(), 4, "IP should advance past both instructions");
}

#[test]
fn vm86_hle_port_bypass_allows_ide_trap_port() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    bus.hle_port_bypass = true;

    let (state, code_base) = setup_vm86_with_iopb(&mut bus);
    cpu.load_state(&state);

    // OUT DX, AL to IDE HLE port 0x07EE.
    #[rustfmt::skip]
    place_at(&mut bus, code_base, &[
        0xBA, 0xEE, 0x07, // MOV DX, 0x07EE
        0xEE,             // OUT DX, AL
    ]);

    cpu.step(&mut bus); // MOV DX, 0x07EE
    cpu.step(&mut bus); // OUT DX, AL
    assert!(
        !cpu.halted(),
        "OUT to IDE HLE port 0x07EE must not raise #GP when bypass is enabled"
    );
    assert_eq!(cpu.ip(), 4, "IP should advance past both instructions");
}

#[test]
fn vm86_non_hle_port_still_denied_by_iopb() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    bus.hle_port_bypass = true;

    let (state, code_base) = setup_vm86_with_iopb(&mut bus);
    cpu.load_state(&state);

    // OUT DX, AL to a non-HLE port (0x0060). IOPB denies it, bypass does not help.
    #[rustfmt::skip]
    place_at(&mut bus, code_base, &[
        0xBA, 0x60, 0x00, // MOV DX, 0x0060
        0xEE,             // OUT DX, AL
    ]);

    cpu.step(&mut bus); // MOV DX, 0x0060
    cpu.step(&mut bus); // OUT DX, AL -> #GP dispatched
    cpu.step(&mut bus); // HLT at #GP handler
    assert!(
        cpu.halted(),
        "OUT to non-HLE port 0x0060 must raise #GP even with HLE bypass enabled"
    );
}

#[test]
fn i386_call_gate_param_copy_wraps_when_old_ss_is_16bit() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let mut state = setup_protected_mode_extended(&mut bus);
    make_ring3_state(&mut state);
    // SS is GDT entry 6 with default granularity (D=0, 16-bit stack).
    state.set_esp(0xFFFC);
    cpu.load_state(&state);

    let gate_target_ip: u32 = 0x0000_0100;
    write_gdt_gate(
        &mut bus,
        PM_GDT_BASE,
        8,
        gate_target_ip,
        PM_CS_SEL,
        2,
        12, // 386/32-bit call gate
        3,
    );

    bus.ram[(PM_CODE_BASE + gate_target_ip) as usize] = 0xF4;

    // First parameter: starts at the very top of the 16-bit SS, last 4 bytes.
    write_dword_at(&mut bus, PM_RING3_STACK_BASE + 0xFFFC, 0xAAAA_AAAA);
    // Second parameter: must come from offset 0x0000 of SS after wrap.
    write_dword_at(&mut bus, PM_RING3_STACK_BASE, 0xBBBB_BBBB);
    // Trap value at the non-wrapped linear address. Reading this means we did
    // not wrap mod 0x10000 the way a 16-bit SS requires.
    write_dword_at(&mut bus, PM_RING3_STACK_BASE + 0x1_0000, 0xCCCC_CCCC);

    place_at(
        &mut bus,
        PM_RING3_CODE_BASE,
        &[0x9A, 0x00, 0x00, 0x40, 0x00], // CALL FAR 0x0040:0x0000 (gate selector 0x0040 = entry 8 RPL 0)
    );

    cpu.step(&mut bus); // CALL FAR through gate
    cpu.step(&mut bus); // HLT at gate target

    assert!(cpu.halted(), "expected to land at the HLT in the target");
    assert_eq!(cpu.cs() & 3, 0, "should now be at ring 0");

    // On the new ring 0 stack: params pushed in reverse order followed by
    // the return frame (CS:EIP for a CALL through a 32-bit gate).
    // Stack grows down, so top has the return EIP, then return CS, then
    // the parameters in original (forward) order.
    let sp = cpu.esp();
    let ss_base = cpu.state.seg_bases[cpu::SegReg32::SS as usize];
    let return_eip = read_dword_at(&bus, ss_base + sp);
    let return_cs = read_dword_at(&bus, ss_base + sp + 4) & 0xFFFF;
    let param0 = read_dword_at(&bus, ss_base + sp + 8);
    let param1 = read_dword_at(&bus, ss_base + sp + 12);
    let pushed_esp = read_dword_at(&bus, ss_base + sp + 16);
    let pushed_ss = read_dword_at(&bus, ss_base + sp + 20) & 0xFFFF;

    assert_eq!(return_eip, 5, "return EIP should follow the 5-byte CALL");
    assert_eq!(return_cs, PM_RING3_CS_SEL as u32);
    assert_eq!(param0, 0xAAAA_AAAA, "param 0 from SS:0xFFFC");
    assert_eq!(
        param1, 0xBBBB_BBBB,
        "param 1 must be read from SS:0x0000 after a 16-bit wrap"
    );
    assert_ne!(
        param1, 0xCCCC_CCCC,
        "param 1 must not be read from the non-wrapped linear address"
    );
    assert_eq!(pushed_esp, 0xFFFC);
    assert_eq!(pushed_ss, PM_RING3_SS_SEL as u32);
}

#[test]
fn i386_iret_inter_privilege_reloads_data_segment_descriptor() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let mut state = setup_protected_mode_with_ring3(&mut bus);

    // Kernel (CPL 0) holds DS = PM_RING3_DS_SEL. Legal: data DPL=3, max(CPL,RPL)=3.
    state.set_ds(PM_RING3_DS_SEL);
    state.seg_bases[cpu::SegReg32::DS as usize] = PM_DATA_BASE;
    state.seg_rights[cpu::SegReg32::DS as usize] = 0xF3;
    state.seg_limits[cpu::SegReg32::DS as usize] = 0xFFFF;

    cpu.load_state(&state);

    // Rewrite GDT entry 5 with a different base while it is cached at CPL 0.
    const NEW_DS_BASE: u32 = PM_DATA_BASE + 0x1000;
    write_gdt_entry16(&mut bus, PM_GDT_BASE, 5, NEW_DS_BASE, 0xFFFF, 0xF3);

    place_at(&mut bus, PM_CODE_BASE, &[0xCF]); // IRET

    let ring3_ip: u16 = 0x0100;
    let sp = cpu.esp();
    write_word_at(&mut bus, PM_STACK_BASE + sp, ring3_ip);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 2, PM_RING3_CS_SEL);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 4, 0x0202);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 6, 0xFF00);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 8, PM_RING3_SS_SEL);

    cpu.step(&mut bus);

    assert_eq!(cpu.cs() & 3, 3, "IRET should land at ring 3");
    assert_eq!(cpu.ds(), PM_RING3_DS_SEL, "DS selector preserved");
    assert_eq!(
        cpu.state.seg_bases[cpu::SegReg32::DS as usize],
        NEW_DS_BASE,
        "DS cached base must reflect the freshly read GDT descriptor"
    );
}

#[test]
fn i386_iret_inter_privilege_zeros_execute_only_code_in_data_register() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let mut state = setup_protected_mode_with_ring3(&mut bus);

    // GDT entry 5 starts as a writable user data segment. Replace it with an
    // execute-only conforming code segment (rights 0xFC: P=1, DPL=3, S=1,
    // type=1100b -> code, conforming, not readable, not accessed).
    write_gdt_entry16(&mut bus, PM_GDT_BASE, 5, PM_DATA_BASE, 0xFFFF, 0xFC);

    // Pre-load DS at CPL 0 with the matching cached rights, simulating having
    // loaded it before the descriptor was clobbered.
    state.set_ds(PM_RING3_DS_SEL);
    state.seg_bases[cpu::SegReg32::DS as usize] = PM_DATA_BASE;
    state.seg_rights[cpu::SegReg32::DS as usize] = 0xFC;
    state.seg_limits[cpu::SegReg32::DS as usize] = 0xFFFF;

    cpu.load_state(&state);

    place_at(&mut bus, PM_CODE_BASE, &[0xCF]); // IRET

    let ring3_ip: u16 = 0x0100;
    let sp = cpu.esp();
    write_word_at(&mut bus, PM_STACK_BASE + sp, ring3_ip);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 2, PM_RING3_CS_SEL);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 4, 0x0202);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 6, 0xFF00);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 8, PM_RING3_SS_SEL);

    cpu.step(&mut bus);

    assert_eq!(cpu.cs() & 3, 3, "IRET should land at ring 3");
    assert_eq!(
        cpu.ds(),
        0,
        "Execute-only code in DS must be zeroed on privilege change"
    );
}

/// Validate-then-commit IRET path: an inter-privilege return whose new SS
/// fails validation (DPL!=RPL) must raise #GP without touching any of CS,
/// EIP, EFLAGS, ESP, DS or ES. The new pipeline validates SS first, so this
/// fault must fire before any state mutation.
#[test]
fn i386_iret_inter_priv_invalid_ss_preserves_source_frame() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_ring3(&mut bus);
    cpu.load_state(&state);

    // Replace GDT entry 6 (RING3_SS) with a DPL=2 data descriptor. The RPL
    // of the new SS selector (0x33 -> RPL 3) no longer matches its DPL,
    // making the descriptor unacceptable.
    write_gdt_entry16(&mut bus, PM_GDT_BASE, 6, PM_RING3_STACK_BASE, 0xFFFF, 0xD3);

    place_at(&mut bus, PM_CODE_BASE, &[0xCF]);

    let pre_cs = cpu.cs();
    let pre_ss = cpu.ss();
    let pre_ds = cpu.ds();
    let pre_es = cpu.es();
    let original_esp = cpu.esp();

    let ring3_ip: u16 = 0x0100;
    write_word_at(&mut bus, PM_STACK_BASE + original_esp, ring3_ip);
    write_word_at(&mut bus, PM_STACK_BASE + original_esp + 2, PM_RING3_CS_SEL);
    write_word_at(&mut bus, PM_STACK_BASE + original_esp + 4, 0x0202);
    write_word_at(&mut bus, PM_STACK_BASE + original_esp + 6, 0xFF00);
    write_word_at(&mut bus, PM_STACK_BASE + original_esp + 8, PM_RING3_SS_SEL);

    cpu.step(&mut bus);

    assert_eq!(
        cpu.ip(),
        u32::from(PM_GP_HANDLER_IP),
        "invalid new SS must raise #GP"
    );
    assert_eq!(cpu.cs(), pre_cs, "CS must not have been committed");
    assert_eq!(cpu.ss(), pre_ss, "SS must not have been committed");
    assert_eq!(cpu.ds(), pre_ds, "DS must not have been committed");
    assert_eq!(cpu.es(), pre_es, "ES must not have been committed");

    let handler_sp = cpu.esp();
    assert_eq!(
        handler_sp,
        original_esp.wrapping_sub(16),
        "fault frame must consume exactly 4 dwords on the source stack"
    );
    let error_code = read_dword_at(&bus, PM_STACK_BASE + handler_sp);
    assert_eq!(
        error_code as u16,
        PM_RING3_SS_SEL & 0xFFFC,
        "error code should encode the invalid SS selector"
    );
    assert_eq!(
        read_dword_at(&bus, PM_STACK_BASE + handler_sp + 4),
        0,
        "fault frame EIP must point back at the IRET instruction"
    );
    assert_eq!(
        read_word_at(&bus, PM_STACK_BASE + original_esp + 8),
        PM_RING3_SS_SEL,
        "original IRET frame must survive the failed inter-privilege return"
    );
}

/// Validate-then-commit IRET path: an inter-privilege return where SS is
/// acceptable but the new CS fails validation must raise #GP with no SS
/// commit having occurred. This is the race the rewrite specifically closes:
/// the old commit-as-you-go path loaded SS before validating CS, so an
/// invalid CS would leave SS mutated.
#[test]
fn i386_iret_inter_priv_invalid_cs_no_partial_ss_commit() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_ring3(&mut bus);
    cpu.load_state(&state);

    // Replace GDT entry 4 (RING3_CS) with a data descriptor: still present,
    // legal-looking, but no longer a code segment - rejected by
    // validate_cs_for_return.
    write_gdt_entry16(&mut bus, PM_GDT_BASE, 4, PM_RING3_CODE_BASE, 0xFFFF, 0xF3);

    place_at(&mut bus, PM_CODE_BASE, &[0xCF]);

    let pre_cs = cpu.cs();
    let pre_ss = cpu.ss();
    let pre_ds = cpu.ds();
    let pre_es = cpu.es();
    let original_esp = cpu.esp();

    let ring3_ip: u16 = 0x0100;
    write_word_at(&mut bus, PM_STACK_BASE + original_esp, ring3_ip);
    write_word_at(&mut bus, PM_STACK_BASE + original_esp + 2, PM_RING3_CS_SEL);
    write_word_at(&mut bus, PM_STACK_BASE + original_esp + 4, 0x0202);
    write_word_at(&mut bus, PM_STACK_BASE + original_esp + 6, 0xFF00);
    write_word_at(&mut bus, PM_STACK_BASE + original_esp + 8, PM_RING3_SS_SEL);

    cpu.step(&mut bus);

    assert_eq!(
        cpu.ip(),
        u32::from(PM_GP_HANDLER_IP),
        "non-code new CS must raise #GP"
    );
    assert_eq!(cpu.cs(), pre_cs, "CS must not have been committed");
    assert_eq!(
        cpu.ss(),
        pre_ss,
        "SS must not have been committed despite passing its own validation"
    );
    assert_eq!(cpu.ds(), pre_ds, "DS must not have been committed");
    assert_eq!(cpu.es(), pre_es, "ES must not have been committed");

    let handler_sp = cpu.esp();
    let error_code = read_dword_at(&bus, PM_STACK_BASE + handler_sp);
    assert_eq!(
        error_code as u16,
        PM_RING3_CS_SEL & 0xFFFC,
        "error code should encode the invalid CS selector"
    );
    assert_eq!(
        read_dword_at(&bus, PM_STACK_BASE + handler_sp + 4),
        0,
        "fault frame EIP must point back at the IRET instruction"
    );
}

/// Validate-then-commit IRET path: when the inter-privilege return drops the
/// CPL below the cached DS DPL, the new pipeline pre-decides "Null" and the
/// commit block writes the null selector. Exercises check_data_segment_at_cpl
/// + apply_data_segment_decision on a happy path with no descriptor fault.
#[test]
fn i386_iret_inter_priv_dpl2_data_segment_nulled_on_ring3_return() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();
    let mut state = setup_protected_mode_with_ring3(&mut bus);

    // Add a DPL=2 data descriptor at GDT index 8 (selector 0x40, RPL 2 -> 0x42).
    const DPL2_DS_SEL: u16 = 0x0042;
    write_gdt_entry16(&mut bus, PM_GDT_BASE, 8, PM_DATA_BASE, 0xFFFF, 0xD3);
    state.gdt_limit = 9 * 8 - 1;

    // Pre-load DS at CPL 0 with the DPL=2 selector. Legal at CPL 0 because
    // DPL=2 >= max(CPL,RPL)=2.
    state.set_ds(DPL2_DS_SEL);
    state.seg_bases[cpu::SegReg32::DS as usize] = PM_DATA_BASE;
    state.seg_rights[cpu::SegReg32::DS as usize] = 0xD3;
    state.seg_limits[cpu::SegReg32::DS as usize] = 0xFFFF;

    cpu.load_state(&state);

    place_at(&mut bus, PM_CODE_BASE, &[0xCF]);

    let ring3_ip: u16 = 0x0100;
    let sp = cpu.esp();
    write_word_at(&mut bus, PM_STACK_BASE + sp, ring3_ip);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 2, PM_RING3_CS_SEL);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 4, 0x0202);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 6, 0xFF00);
    write_word_at(&mut bus, PM_STACK_BASE + sp + 8, PM_RING3_SS_SEL);

    cpu.step(&mut bus);

    assert_eq!(cpu.cs() & 3, 3, "IRET should land at ring 3");
    assert_eq!(
        cpu.ds(),
        0,
        "DS with DPL=2 must be nulled when returning to CPL=3"
    );
}
