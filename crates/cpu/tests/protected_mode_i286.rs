use common::{Bus as _, Cpu as _};
use cpu::I286;

const RAM_SIZE: usize = 1024 * 1024;
const ADDRESS_MASK: u32 = 0x000F_FFFF;

struct TestBus {
    ram: Vec<u8>,
    irq_pending: bool,
    irq_vector: u8,
}

impl TestBus {
    fn new() -> Self {
        Self {
            ram: vec![0u8; RAM_SIZE],
            irq_pending: false,
            irq_vector: 0,
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
    limit: u16,
    rights: u8,
) {
    let addr = (gdt_base + (entry_index as u32) * 8) as usize;
    bus.ram[addr] = limit as u8;
    bus.ram[addr + 1] = (limit >> 8) as u8;
    bus.ram[addr + 2] = base as u8;
    bus.ram[addr + 3] = (base >> 8) as u8;
    bus.ram[addr + 4] = (base >> 16) as u8;
    bus.ram[addr + 5] = rights;
    bus.ram[addr + 6] = 0;
    bus.ram[addr + 7] = 0;
}

fn write_idt_gate(
    bus: &mut TestBus,
    idt_base: u32,
    vector: u8,
    offset: u16,
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
    bus.ram[addr + 6] = 0;
    bus.ram[addr + 7] = 0;
}

fn setup_protected_mode(bus: &mut TestBus, ds_limit: u16) -> cpu::I286State {
    write_gdt_entry(bus, PM_GDT_BASE, 0, 0, 0, 0);
    write_gdt_entry(bus, PM_GDT_BASE, 1, PM_CODE_BASE, 0xFFFF, 0x9B);
    write_gdt_entry(bus, PM_GDT_BASE, 2, PM_DATA_BASE, ds_limit, 0x93);
    write_gdt_entry(bus, PM_GDT_BASE, 3, PM_STACK_BASE, 0xFFFF, 0x93);

    write_idt_gate(bus, PM_IDT_BASE, 13, PM_GP_HANDLER_IP, PM_CS_SEL, 6, 0);

    bus.ram[(PM_CODE_BASE + PM_GP_HANDLER_IP as u32) as usize] = 0xF4; // HLT

    let mut state = cpu::I286State::default();
    state.msw = 0x0001;

    state.set_sp(0xFFF0);

    state.set_cs(PM_CS_SEL);
    state.set_ds(PM_DS_SEL);
    state.set_ss(PM_SS_SEL);
    state.set_es(PM_DS_SEL);

    state.seg_bases[cpu::SegReg16::ES as usize] = PM_DATA_BASE;
    state.seg_bases[cpu::SegReg16::CS as usize] = PM_CODE_BASE;
    state.seg_bases[cpu::SegReg16::SS as usize] = PM_STACK_BASE;
    state.seg_bases[cpu::SegReg16::DS as usize] = PM_DATA_BASE;

    state.seg_limits[cpu::SegReg16::ES as usize] = ds_limit;
    state.seg_limits[cpu::SegReg16::CS as usize] = 0xFFFF;
    state.seg_limits[cpu::SegReg16::SS as usize] = 0xFFFF;
    state.seg_limits[cpu::SegReg16::DS as usize] = ds_limit;

    state.seg_rights[cpu::SegReg16::ES as usize] = 0x93;
    state.seg_rights[cpu::SegReg16::CS as usize] = 0x9B;
    state.seg_rights[cpu::SegReg16::SS as usize] = 0x93;
    state.seg_rights[cpu::SegReg16::DS as usize] = 0x93;

    state.seg_valid = [true, true, true, true];

    state.gdt_base = PM_GDT_BASE;
    state.gdt_limit = 4 * 8 - 1;

    state.idt_base = PM_IDT_BASE;
    state.idt_limit = 256 * 8 - 1;

    state
}

fn read_word_at(bus: &TestBus, addr: u32) -> u16 {
    bus.ram[addr as usize] as u16 | ((bus.ram[addr as usize + 1] as u16) << 8)
}

/// Real mode: MSW should have PE=0 after reset.
#[test]
fn i286_msw_pe_clear_after_reset() {
    let cpu = I286::new();
    assert_eq!(cpu.msw & 1, 0, "PE bit should be clear after reset");
    assert_eq!(cpu.msw, 0xFFF0, "MSW reset value should be 0xFFF0");
}

/// Real mode: SMSW reads back the current MSW (no mode switch).
#[test]
fn i286_smsw_returns_msw() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();

    // SMSW AX (0x0F 0x01 modrm=0xE0: mod=11, /4, rm=AX)
    place_code(&mut bus, 0xFFFF, 0x0000, &[0x0F, 0x01, 0xE0]);

    cpu.step(&mut bus);

    assert_eq!(cpu.ax(), 0xFFF0, "SMSW should store MSW (0xFFF0) into AX");
}

/// Real mode -> Protected mode: LMSW sets PE bit in MSW.
#[test]
fn i286_lmsw_sets_pe() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();

    // MOV AX, 0x0001
    // LMSW AX (0x0F 0x01 modrm=0xF0: mod=11, /6, rm=AX)
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
    assert_eq!(cpu.msw & 1, 0, "PE should be clear before LMSW");

    cpu.step(&mut bus);
    assert_eq!(cpu.msw & 1, 1, "PE should be set after LMSW with value 1");
}

/// Real mode -> Protected mode: LMSW writes all 16 bits of MSW.
#[test]
fn i286_lmsw_writes_all_bits() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();

    // MOV AX, 0xFFFF
    // LMSW AX
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

    assert_eq!(cpu.msw, 0xFFFF, "LMSW should write all 16 bits of MSW");
}

/// i286: undefined opcode 0x66 triggers #UD (INT 6).
///
/// The 80286 introduced the Invalid Opcode exception. The fault pushes CS:IP
/// pointing to the faulting opcode, then vectors through IVT entry 6.
#[test]
fn i286_invalid_opcode_0x66_triggers_ud() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let ip: u16 = 0x0050;
    place_code(&mut bus, cs, ip, &[0x66, 0x90]);

    // Set up INT 6 handler at 0x2000:0x0000
    let handler_cs: u16 = 0x2000;
    let handler_ip: u16 = 0x0000;
    let ivt_addr = 6 * 4; // INT 6 = offset 0x18 in IVT
    bus.ram[ivt_addr] = handler_ip as u8;
    bus.ram[ivt_addr + 1] = (handler_ip >> 8) as u8;
    bus.ram[ivt_addr + 2] = handler_cs as u8;
    bus.ram[ivt_addr + 3] = (handler_cs >> 8) as u8;

    let mut state = cpu::I286State::default();
    state.set_cs(cs);
    state.ip = ip;
    state.set_ss(0x3000);
    state.set_sp(0x1000);
    state.initialize_real_mode_caches();
    cpu.load_state(&state);

    cpu.step(&mut bus);

    assert_eq!(
        cpu.cs(),
        handler_cs,
        "i286: #UD should jump to INT 6 handler CS"
    );
    assert_eq!(
        cpu.ip, handler_ip,
        "i286: #UD should jump to INT 6 handler IP"
    );

    // Verify the return IP pushed on stack points to the faulting opcode
    let sp = cpu.sp();
    let ss_base = (cpu.ss() as u32) << 4;
    let pushed_ip = bus.ram[(ss_base + sp as u32) as usize] as u16
        | (bus.ram[(ss_base + sp as u32 + 1) as usize] as u16) << 8;
    assert_eq!(
        pushed_ip, ip,
        "i286: #UD should push faulting IP={ip:#06X} on stack, got {pushed_ip:#06X}"
    );
}

/// i286: undefined opcode 0x67 triggers #UD (INT 6).
#[test]
fn i286_invalid_opcode_0x67_triggers_ud() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let ip: u16 = 0x0050;
    place_code(&mut bus, cs, ip, &[0x67, 0x90]);

    let handler_cs: u16 = 0x2000;
    let handler_ip: u16 = 0x0000;
    let ivt_addr = 6 * 4;
    bus.ram[ivt_addr] = handler_ip as u8;
    bus.ram[ivt_addr + 1] = (handler_ip >> 8) as u8;
    bus.ram[ivt_addr + 2] = handler_cs as u8;
    bus.ram[ivt_addr + 3] = (handler_cs >> 8) as u8;

    let mut state = cpu::I286State::default();
    state.set_cs(cs);
    state.ip = ip;
    state.set_ss(0x3000);
    state.set_sp(0x1000);
    state.initialize_real_mode_caches();
    cpu.load_state(&state);

    cpu.step(&mut bus);

    assert_eq!(cpu.cs(), handler_cs);
    assert_eq!(cpu.ip, handler_ip);

    let sp = cpu.sp();
    let ss_base = (cpu.ss() as u32) << 4;
    let pushed_ip = bus.ram[(ss_base + sp as u32) as usize] as u16
        | (bus.ram[(ss_base + sp as u32 + 1) as usize] as u16) << 8;
    assert_eq!(
        pushed_ip, ip,
        "i286: #UD should push faulting IP={ip:#06X} on stack, got {pushed_ip:#06X}"
    );
}

/// LMSW loads all 16 bits of the source operand into MSW, not just the low nibble.
/// Old behavior masked to low 4 bits: (0xFFF0 & 0xFFF0) | (0x0035 & 0x000F) = 0xFFF5.
/// Correct behavior: MSW = 0x0035 (PE set, other bits from operand).
#[test]
fn i286_lmsw_loads_all_bits() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();

    place_code(
        &mut bus,
        0xFFFF,
        0x0000,
        &[
            0xB8, 0x35, 0x00, // MOV AX, 0x0035
            0x0F, 0x01, 0xF0, // LMSW AX
        ],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_eq!(
        cpu.msw, 0x0035,
        "LMSW should load all 16 bits, not just the low nibble"
    );
}

/// LMSW cannot clear PE once set, even though it loads all 16 bits.
#[test]
fn i286_lmsw_cannot_clear_pe() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();

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
    assert_eq!(cpu.msw & 1, 1, "PE should be set after LMSW with value 1");

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    assert_eq!(cpu.msw & 1, 1, "LMSW must not be able to clear PE once set");
}

/// MOV AL, [moffs] in protected mode reads data through segment base correctly.
#[test]
fn i286_mov_al_moffs_protected_mode_reads_correctly() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();

    let state = setup_protected_mode(&mut bus, 0xFFFF);
    cpu.load_state(&state);

    bus.ram[(PM_DATA_BASE + 0x05) as usize] = 0xAB;

    // MOV AL, [0x0005] (opcode 0xA0, offset_lo, offset_hi)
    place_at(&mut bus, PM_CODE_BASE, &[0xA0, 0x05, 0x00]);

    cpu.step(&mut bus);

    assert_eq!(
        cpu.al(),
        0xAB,
        "MOV AL,[moffs] should read DS:0x05 via segment base in protected mode"
    );
}

/// MOV AL, [moffs] in protected mode raises #GP when offset exceeds DS limit.
#[test]
fn i286_mov_al_moffs_protected_mode_gp_on_limit_violation() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();

    let state = setup_protected_mode(&mut bus, 0x000F);
    cpu.load_state(&state);

    // MOV AL, [0x0020] - offset 0x20 exceeds DS limit 0x0F
    place_at(&mut bus, PM_CODE_BASE, &[0xA0, 0x20, 0x00]);

    cpu.step(&mut bus); // MOV faults, dispatch goes to #GP handler
    cpu.step(&mut bus); // Execute HLT at #GP handler

    assert!(
        cpu.halted(),
        "CPU should be halted at #GP handler after DS limit violation"
    );
    assert_eq!(
        cpu.ip(),
        PM_GP_HANDLER_IP + 1,
        "IP should be past HLT inside #GP handler"
    );
}

/// MOV [moffs], AL in protected mode writes data through segment base correctly.
#[test]
fn i286_mov_moffs_al_protected_mode_writes_correctly() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.set_ax(0x0042);
    cpu.load_state(&state);

    // MOV [0x0010], AL (opcode 0xA2, offset_lo, offset_hi)
    place_at(&mut bus, PM_CODE_BASE, &[0xA2, 0x10, 0x00]);

    cpu.step(&mut bus);

    assert_eq!(
        bus.ram[(PM_DATA_BASE + 0x10) as usize],
        0x42,
        "MOV [moffs],AL should write to DS:0x10 via segment base in protected mode"
    );
}

/// XLAT in protected mode reads DS:[BX+AL] through segment-checked path.
#[test]
fn i286_xlat_protected_mode_reads_correctly() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.set_bx(0x0100);
    state.set_ax(0x0005); // AL = 0x05
    cpu.load_state(&state);

    bus.ram[(PM_DATA_BASE + 0x105) as usize] = 0x7E;

    // XLAT (opcode 0xD7) reads DS:[BX + AL]
    place_at(&mut bus, PM_CODE_BASE, &[0xD7]);

    cpu.step(&mut bus);

    assert_eq!(
        cpu.al(),
        0x7E,
        "XLAT should read DS:[BX+AL] via segment base in protected mode"
    );
}

/// XLAT in protected mode raises #GP when BX+AL exceeds DS limit.
#[test]
fn i286_xlat_protected_mode_gp_on_limit_violation() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();

    let state = setup_protected_mode(&mut bus, 0x000F);
    cpu.load_state(&state);
    cpu.set_bx(0x0020);
    cpu.set_ax(0x0005); // BX+AL = 0x25, exceeds limit 0x0F

    // XLAT (opcode 0xD7)
    place_at(&mut bus, PM_CODE_BASE, &[0xD7]);

    cpu.step(&mut bus); // XLAT faults, dispatch to #GP handler
    cpu.step(&mut bus); // Execute HLT at handler

    assert!(
        cpu.halted(),
        "CPU should be halted at #GP handler after DS limit violation in XLAT"
    );
}

/// Far JMP to conforming code segment adjusts CS RPL bits to current CPL.
/// Without Fix 3, CS would retain the raw selector RPL (1), making cpl() return 1.
/// With Fix 3, CS RPL is set to current CPL (0).
#[test]
fn i286_far_jmp_conforming_code_adjusts_cs_rpl_to_cpl() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();

    let state = setup_protected_mode(&mut bus, 0xFFFF);
    cpu.load_state(&state);

    // Add conforming code segment at GDT index 4 (selector 0x20).
    // Rights 0x9E = P(0x80) | DPL=0(0x00) | S(0x10) | code(0x08) | conforming(0x04) | readable(0x02)
    write_gdt_entry(&mut bus, PM_GDT_BASE, 4, PM_CODE_BASE, 0xFFFF, 0x9E);
    cpu.gdt_limit = 5 * 8 - 1;

    // JMP FAR 0x0021:0x2000 - selector 0x0021 has index 4, RPL=1.
    // For conforming code, RPL is not checked against CPL, so this should succeed.
    place_at(&mut bus, PM_CODE_BASE, &[0xEA, 0x00, 0x20, 0x21, 0x00]);

    // Place HLT at the target (CS:0x2000 = CODE_BASE + 0x2000)
    bus.ram[(PM_CODE_BASE + 0x2000) as usize] = 0xF4;

    cpu.step(&mut bus);

    assert_eq!(
        cpu.cs(),
        0x0020,
        "CS RPL should be adjusted to CPL (0), not retain selector RPL (1)"
    );
    assert_eq!(
        cpu.cs() & 3,
        0,
        "CPL should remain 0 after JMP to conforming code"
    );
    assert_eq!(cpu.ip(), 0x2000, "IP should be at JMP target offset");
}

/// INT in protected mode dispatches through IDT interrupt gate, clearing IF.
#[test]
fn i286_protected_mode_int_dispatches_via_intgate() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();

    let state = setup_protected_mode(&mut bus, 0xFFFF);
    cpu.load_state(&state);

    // Set up IDT gate for INT 0x20: interrupt gate (type 6), DPL=3 (allow software INT)
    let handler_ip: u16 = 0x3000;
    write_idt_gate(&mut bus, PM_IDT_BASE, 0x20, handler_ip, PM_CS_SEL, 6, 3);

    // Place HLT at handler
    bus.ram[(PM_CODE_BASE + handler_ip as u32) as usize] = 0xF4;

    // Enable IF before INT
    cpu.flags.if_flag = true;

    // INT 0x20 (opcode 0xCD 0x20)
    place_at(&mut bus, PM_CODE_BASE, &[0xCD, 0x20]);

    cpu.step(&mut bus); // Execute INT 0x20 (dispatches to handler)
    cpu.step(&mut bus); // Execute HLT at handler

    assert!(cpu.halted(), "CPU should be halted at INT handler");
    assert_eq!(cpu.ip(), handler_ip + 1, "IP should be past HLT in handler");
    assert!(!cpu.flags.if_flag, "Interrupt gate should clear IF");

    // Verify stack frame: FLAGS, CS, IP were pushed (topmost is IP)
    let sp = cpu.sp();
    let pushed_ip = read_word_at(&bus, PM_STACK_BASE + sp as u32);
    let pushed_cs = read_word_at(&bus, PM_STACK_BASE + sp as u32 + 2);
    let pushed_flags = read_word_at(&bus, PM_STACK_BASE + sp as u32 + 4);

    assert_eq!(
        pushed_ip, 0x0002,
        "Pushed IP should be return address (after 2-byte INT instruction)"
    );
    assert_eq!(
        pushed_cs, PM_CS_SEL,
        "Pushed CS should be original CS selector"
    );
    assert_ne!(
        pushed_flags & 0x0200,
        0,
        "Pushed flags should have IF set (was enabled before INT)"
    );
}

/// INT in protected mode dispatches through IDT trap gate, preserving IF.
#[test]
fn i286_protected_mode_int_trapgate_preserves_if() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();

    let state = setup_protected_mode(&mut bus, 0xFFFF);
    cpu.load_state(&state);

    // Set up IDT gate for INT 0x21: trap gate (type 7), DPL=3
    let handler_ip: u16 = 0x4000;
    write_idt_gate(&mut bus, PM_IDT_BASE, 0x21, handler_ip, PM_CS_SEL, 7, 3);

    // Place HLT at handler
    bus.ram[(PM_CODE_BASE + handler_ip as u32) as usize] = 0xF4;

    cpu.flags.if_flag = true;

    // INT 0x21 (opcode 0xCD 0x21)
    place_at(&mut bus, PM_CODE_BASE, &[0xCD, 0x21]);

    cpu.step(&mut bus); // Execute INT 0x21
    cpu.step(&mut bus); // Execute HLT at handler

    assert!(cpu.halted(), "CPU should be halted at trap handler");
    assert!(
        cpu.flags.if_flag,
        "Trap gate should preserve IF (not clear it)"
    );
}

/// Protected-mode INT sets CS selector with RPL matching target DPL.
#[test]
fn i286_protected_mode_int_sets_cs_rpl_correctly() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();

    let state = setup_protected_mode(&mut bus, 0xFFFF);
    cpu.load_state(&state);

    let handler_ip: u16 = 0x5000;
    write_idt_gate(&mut bus, PM_IDT_BASE, 0x30, handler_ip, PM_CS_SEL, 6, 3);

    bus.ram[(PM_CODE_BASE + handler_ip as u32) as usize] = 0xF4; // HLT

    // INT 0x30 (opcode 0xCD 0x30)
    place_at(&mut bus, PM_CODE_BASE, &[0xCD, 0x30]);

    cpu.step(&mut bus); // Execute INT

    // CS selector should have RPL = target DPL (0)
    assert_eq!(
        cpu.cs() & !3,
        PM_CS_SEL & !3,
        "CS index should match gate target"
    );
    assert_eq!(cpu.cs() & 3, 0, "CS RPL should reflect target DPL (0)");
    assert_eq!(cpu.ip(), handler_ip, "IP should be at gate offset");
}

#[test]
fn i286_real_mode_fault_no_error_code() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();

    let handler_cs: u16 = 0x2000;
    let handler_ip: u16 = 0x0000;
    let ivt_addr = 6 * 4;
    bus.ram[ivt_addr] = handler_ip as u8;
    bus.ram[ivt_addr + 1] = (handler_ip >> 8) as u8;
    bus.ram[ivt_addr + 2] = handler_cs as u8;
    bus.ram[ivt_addr + 3] = (handler_cs >> 8) as u8;

    let mut state = cpu::I286State::default();
    state.set_cs(0x1000);
    state.ip = 0x0050;
    state.set_ss(0x3000);
    state.set_sp(0x1000);
    state.initialize_real_mode_caches();
    cpu.load_state(&state);

    // Place invalid opcode to trigger #UD.
    place_code(&mut bus, 0x1000, 0x0050, &[0x66]);

    let sp_before = cpu.sp();
    cpu.step(&mut bus);

    // Real mode: push FLAGS, CS, IP = 6 bytes, SP decremented by 6.
    assert_eq!(
        sp_before - cpu.sp(),
        6,
        "Real-mode fault should push only FLAGS/CS/IP (6 bytes, not 8)"
    );
}

#[test]
fn i286_popf_pushf_iopl_nt_roundtrip() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();

    let state = setup_protected_mode(&mut bus, 0xFFFF);
    cpu.load_state(&state);

    // At CPL=0, POPF can set IOPL and NT.
    // PUSH 0x7002 (IOPL=3, NT=1, bit1 always set); POPF; PUSHF; POP AX
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

    cpu.step(&mut bus); // PUSH
    cpu.step(&mut bus); // POPF
    cpu.step(&mut bus); // PUSHF
    cpu.step(&mut bus); // POP AX

    let flags = cpu.ax();
    assert_eq!((flags >> 12) & 3, 3, "IOPL should be 3 after POPF at CPL=0");
    assert_ne!(flags & 0x4000, 0, "NT should be set after POPF at CPL=0");
}

#[test]
fn i286_gp_escalates_to_double_fault() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    // Extend GDT for extra entries.
    state.gdt_limit = 8 * 8 - 1;
    cpu.load_state(&state);

    // Set up #GP handler (vector 13) to point to an invalid segment, causing another #GP.
    // This should escalate to #DF (vector 8).
    let df_handler_ip: u16 = 0x6000;
    write_idt_gate(&mut bus, PM_IDT_BASE, 13, 0x0000, 0x0028, 6, 0); // invalid selector 0x28
    write_idt_gate(&mut bus, PM_IDT_BASE, 8, df_handler_ip, PM_CS_SEL, 6, 0);

    bus.ram[(PM_CODE_BASE + df_handler_ip as u32) as usize] = 0xF4; // HLT

    // Trigger #GP by loading an invalid segment.
    // MOV AX, 0x0028; MOV DS, AX
    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[
            0xB8, 0x28, 0x00, // MOV AX, 0x0028
            0x8E, 0xD8, // MOV DS, AX
        ],
    );

    cpu.step(&mut bus); // MOV AX
    cpu.step(&mut bus); // MOV DS - triggers #GP, #GP handler faults too -> #DF

    // The CPU should now be at the #DF handler.
    // It may take one more step to execute the HLT.
    cpu.step(&mut bus);

    assert!(
        cpu.halted(),
        "CPU should be halted at #DF handler after double fault"
    );
}

#[test]
fn i286_lgdt_at_cpl3_triggers_gp() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    // Set CPL=3 by changing CS selector RPL.
    let ring3_cs_sel: u16 = 0x0020;
    // Create ring 3 code segment at GDT index 4.
    // Rights: P=1, DPL=3, S=1, code=1, readable=1 = 0xFB
    write_gdt_entry(&mut bus, PM_GDT_BASE, 4, PM_CODE_BASE, 0xFFFF, 0xFB);
    state.gdt_limit = 5 * 8 - 1;
    state.set_cs(ring3_cs_sel | 3);
    state.seg_bases[cpu::SegReg16::CS as usize] = PM_CODE_BASE;
    state.seg_limits[cpu::SegReg16::CS as usize] = 0xFFFF;
    state.seg_rights[cpu::SegReg16::CS as usize] = 0xFB;

    // Also need a ring 3 SS.
    let ring3_ss_sel: u16 = 0x0028;
    write_gdt_entry(&mut bus, PM_GDT_BASE, 5, PM_STACK_BASE, 0xFFFF, 0xF3);
    state.gdt_limit = 6 * 8 - 1;
    state.set_ss(ring3_ss_sel | 3);
    state.seg_bases[cpu::SegReg16::SS as usize] = PM_STACK_BASE;
    state.seg_limits[cpu::SegReg16::SS as usize] = 0xFFFF;
    state.seg_rights[cpu::SegReg16::SS as usize] = 0xF3;

    cpu.load_state(&state);

    // LGDT [0x1000] (0x0F 0x01 0x16 0x00 0x10)
    place_at(&mut bus, PM_CODE_BASE, &[0x0F, 0x01, 0x16, 0x00, 0x10]);

    cpu.step(&mut bus); // LGDT at CPL=3 should trigger #GP
    cpu.step(&mut bus); // HLT at #GP handler

    assert!(
        cpu.halted(),
        "LGDT at CPL=3 should trigger #GP -> halt at handler"
    );
}

#[test]
fn i286_ltr_marks_tss_busy() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.gdt_limit = 6 * 8 - 1;
    cpu.load_state(&state);

    // Create an available TSS (type 1) at GDT index 4 (selector 0x20).
    // Rights: P=1, DPL=0, type=1 (available TSS) = 0x81
    let tss_base: u32 = 0x60000;
    write_gdt_entry(&mut bus, PM_GDT_BASE, 4, tss_base, 0x002B, 0x81);

    // LTR AX with AX = 0x0020 (selector for GDT index 4)
    // MOV AX, 0x0020; 0F 00 /3 = LTR AX (modrm=0xD8)
    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[
            0xB8, 0x20, 0x00, // MOV AX, 0x0020
            0x0F, 0x00, 0xD8, // LTR AX
        ],
    );

    cpu.step(&mut bus); // MOV AX
    cpu.step(&mut bus); // LTR

    // Read back descriptor rights byte from GDT.
    let desc_addr = PM_GDT_BASE + 4 * 8;
    let rights = bus.ram[(desc_addr + 5) as usize];
    let desc_type = rights & 0x0F;
    assert_eq!(
        desc_type, 0x03,
        "LTR should mark TSS type as busy (3), got type {}",
        desc_type
    );
}

#[test]
fn i286_ltr_busy_tss_triggers_gp() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.gdt_limit = 6 * 8 - 1;
    cpu.load_state(&state);

    // Create a busy TSS (type 3) at GDT index 4 (selector 0x20).
    // Rights: P=1, DPL=0, type=3 (busy TSS) = 0x83
    write_gdt_entry(&mut bus, PM_GDT_BASE, 4, 0x60000, 0x002B, 0x83);

    // MOV AX, 0x0020; LTR AX
    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[
            0xB8, 0x20, 0x00, // MOV AX, 0x0020
            0x0F, 0x00, 0xD8, // LTR AX
        ],
    );

    cpu.step(&mut bus); // MOV AX
    cpu.step(&mut bus); // LTR on busy TSS -> #GP
    cpu.step(&mut bus); // HLT at #GP handler

    assert!(
        cpu.halted(),
        "LTR on busy TSS should trigger #GP -> halt at handler"
    );
}

#[test]
fn i286_lar_accepts_tss_descriptor() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.gdt_limit = 6 * 8 - 1;
    cpu.load_state(&state);

    // Create an available TSS (type 1) at GDT index 4 (selector 0x20).
    // Rights: P=1, DPL=0, type=1 = 0x81
    write_gdt_entry(&mut bus, PM_GDT_BASE, 4, 0x60000, 0x002B, 0x81);

    // MOV BX, 0x0020; LAR AX, BX
    // LAR = 0F 02 modrm. modrm for LAR AX,BX: reg=AX(0), rm=BX(3) -> 0xC3
    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[
            0xBB, 0x20, 0x00, // MOV BX, 0x0020
            0x0F, 0x02, 0xC3, // LAR AX, BX
        ],
    );

    cpu.step(&mut bus); // MOV BX
    cpu.step(&mut bus); // LAR

    assert!(
        cpu.flags.zf(),
        "LAR on TSS descriptor should set ZF=1 (valid)"
    );
    assert_eq!(
        cpu.ax(),
        0x8100,
        "LAR should return rights byte shifted left by 8"
    );
}

#[test]
fn i286_lsl_rejects_call_gate() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.gdt_limit = 6 * 8 - 1;
    cpu.load_state(&state);

    // Create a call gate (type 4) at GDT index 4 (selector 0x20).
    // Rights: P=1, DPL=0, type=4 = 0x84
    write_gdt_entry(&mut bus, PM_GDT_BASE, 4, 0x60000, 0x002B, 0x84);

    // MOV BX, 0x0020; LSL AX, BX
    // LSL = 0F 03 modrm. modrm for LSL AX,BX: reg=AX(0), rm=BX(3) -> 0xC3
    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[
            0xBB, 0x20, 0x00, // MOV BX, 0x0020
            0x0F, 0x03, 0xC3, // LSL AX, BX
        ],
    );

    cpu.step(&mut bus); // MOV BX
    cpu.step(&mut bus); // LSL

    assert!(
        !cpu.flags.zf(),
        "LSL on call gate (type 4) should set ZF=0 (invalid)"
    );
}

#[test]
fn i286_interrupt_conforming_code_dispatches() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.gdt_limit = 6 * 8 - 1;
    cpu.load_state(&state);

    // Create conforming code segment at GDT index 4 (selector 0x20).
    // Rights: P=1, DPL=0, S=1, code=1, conforming=1, readable=1 = 0x9E
    write_gdt_entry(&mut bus, PM_GDT_BASE, 4, PM_CODE_BASE, 0xFFFF, 0x9E);

    // Set up IDT gate for INT 0x40 -> conforming code segment.
    let handler_ip: u16 = 0x7000;
    write_idt_gate(&mut bus, PM_IDT_BASE, 0x40, handler_ip, 0x0020, 6, 3);

    bus.ram[(PM_CODE_BASE + handler_ip as u32) as usize] = 0xF4; // HLT

    // INT 0x40
    place_at(&mut bus, PM_CODE_BASE, &[0xCD, 0x40]);

    cpu.step(&mut bus); // INT 0x40
    cpu.step(&mut bus); // HLT

    assert!(cpu.halted(), "INT to conforming code should dispatch");
    assert_eq!(cpu.ip(), handler_ip + 1);
}

#[test]
fn i286_load_segment_sets_accessed_bit() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.gdt_limit = 6 * 8 - 1;
    cpu.load_state(&state);

    // Create data segment at GDT index 4 (selector 0x20).
    // Rights: P=1, DPL=0, S=1, data, writable, NOT accessed = 0x92
    write_gdt_entry(&mut bus, PM_GDT_BASE, 4, PM_DATA_BASE, 0xFFFF, 0x92);

    // Verify accessed bit is clear before.
    let desc_addr = PM_GDT_BASE + 4 * 8;
    assert_eq!(
        bus.ram[(desc_addr + 5) as usize] & 0x01,
        0,
        "Accessed bit should be clear before loading segment"
    );

    // MOV AX, 0x0020; MOV DS, AX
    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[
            0xB8, 0x20, 0x00, // MOV AX, 0x0020
            0x8E, 0xD8, // MOV DS, AX
        ],
    );

    cpu.step(&mut bus); // MOV AX
    cpu.step(&mut bus); // MOV DS

    let rights_after = bus.ram[(desc_addr + 5) as usize];
    assert_ne!(
        rights_after & 0x01,
        0,
        "Accessed bit should be set after loading segment"
    );
}

#[test]
fn i286_verr_conforming_code_dpl_exemption() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.gdt_limit = 6 * 8 - 1;
    // Use CPL=3 selector.
    let ring3_cs_sel: u16 = 0x0028;
    write_gdt_entry(&mut bus, PM_GDT_BASE, 5, PM_CODE_BASE, 0xFFFF, 0xFB);
    state.gdt_limit = 6 * 8 - 1;
    state.set_cs(ring3_cs_sel | 3);
    state.seg_bases[cpu::SegReg16::CS as usize] = PM_CODE_BASE;
    state.seg_limits[cpu::SegReg16::CS as usize] = 0xFFFF;
    state.seg_rights[cpu::SegReg16::CS as usize] = 0xFB;

    let ring3_ss_sel: u16 = 0x0030;
    write_gdt_entry(&mut bus, PM_GDT_BASE, 6, PM_STACK_BASE, 0xFFFF, 0xF3);
    state.gdt_limit = 7 * 8 - 1;
    state.set_ss(ring3_ss_sel | 3);
    state.seg_bases[cpu::SegReg16::SS as usize] = PM_STACK_BASE;
    state.seg_limits[cpu::SegReg16::SS as usize] = 0xFFFF;
    state.seg_rights[cpu::SegReg16::SS as usize] = 0xF3;

    cpu.load_state(&state);

    // Create conforming code segment at GDT index 4 (selector 0x20).
    // DPL=0, conforming, readable = 0x9E (P=1, DPL=0, S=1, C=1, E=1, R=1)
    write_gdt_entry(&mut bus, PM_GDT_BASE, 4, PM_CODE_BASE, 0xFFFF, 0x9E);

    // VERR with selector 0x0020 (conforming code, DPL=0)
    // MOV AX, 0x0020; 0F 00 /4 = VERR AX (modrm 0xE0)
    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[
            0xB8, 0x20, 0x00, // MOV AX, 0x0020
            0x0F, 0x00, 0xE0, // VERR AX
        ],
    );

    cpu.step(&mut bus); // MOV AX
    cpu.step(&mut bus); // VERR

    assert!(
        cpu.flags.zf(),
        "VERR on conforming code with DPL=0 at CPL=3 should return ZF=1 (readable)"
    );
}

#[test]
fn i286_gate_offset_exceeds_limit_gp_zero() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.gdt_limit = 6 * 8 - 1;
    cpu.load_state(&state);

    // Create a code segment with small limit at GDT index 4.
    write_gdt_entry(&mut bus, PM_GDT_BASE, 4, PM_CODE_BASE, 0x0010, 0x9B);

    // IDT gate for INT 0x50 -> offset 0x0100 in selector 0x0020 (limit 0x10 < 0x100).
    write_idt_gate(&mut bus, PM_IDT_BASE, 0x50, 0x0100, 0x0020, 6, 3);

    // Set up a #GP handler that stores the error code to check it.
    let gp_handler_ip: u16 = 0x9000;
    write_idt_gate(&mut bus, PM_IDT_BASE, 13, gp_handler_ip, PM_CS_SEL, 6, 0);
    bus.ram[(PM_CODE_BASE + gp_handler_ip as u32) as usize] = 0xF4; // HLT

    // INT 0x50
    place_at(&mut bus, PM_CODE_BASE, &[0xCD, 0x50]);

    cpu.step(&mut bus); // INT 0x50 -> gate offset > limit -> #GP(0)
    cpu.step(&mut bus); // HLT

    assert!(cpu.halted());

    // Check error code pushed on stack.
    let sp = cpu.sp();
    let error_code = read_word_at(&bus, PM_STACK_BASE + sp as u32);
    assert_eq!(
        error_code, 0,
        "Gate offset > segment limit should push error code 0, got {error_code:#06X}"
    );
}

#[test]
fn i286_lldt_selector_0x0004_is_nonnull() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.gdt_limit = 6 * 8 - 1;
    cpu.load_state(&state);

    // Selector 0x0004 has TI=1 (LDT reference), index=0.
    // With old `& !7` mask: 0x0004 & !7 == 0 -> treated as null.
    // With correct `& 0xFFFC` mask: 0x0004 & 0xFFFC == 4 -> non-null -> validate.
    // Since no LDT is set up, this should fault (#GP).

    // MOV AX, 0x0004; LLDT AX (0F 00 /2 = modrm 0xD0)
    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[
            0xB8, 0x04, 0x00, // MOV AX, 0x0004
            0x0F, 0x00, 0xD0, // LLDT AX
        ],
    );

    cpu.step(&mut bus); // MOV AX
    cpu.step(&mut bus); // LLDT 0x0004 -> #GP (TI=1, must be from GDT)
    cpu.step(&mut bus); // HLT at handler

    assert!(
        cpu.halted(),
        "LLDT with selector 0x0004 should be treated as non-null and fault"
    );
}

#[test]
fn i286_wrong_type_not_present_gives_gp_not_np() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode(&mut bus, 0xFFFF);
    state.gdt_limit = 6 * 8 - 1;
    cpu.load_state(&state);

    // Create an execute-only code segment at GDT index 4 (selector 0x20).
    // NOT present, code, NOT readable: 0x18 (P=0, DPL=0, S=1, code, execute-only)
    write_gdt_entry(&mut bus, PM_GDT_BASE, 4, PM_DATA_BASE, 0xFFFF, 0x18);

    // Set up separate #NP handler (vector 11) at different address.
    let np_handler_ip: u16 = 0xA000;
    write_idt_gate(&mut bus, PM_IDT_BASE, 11, np_handler_ip, PM_CS_SEL, 6, 0);
    bus.ram[(PM_CODE_BASE + np_handler_ip as u32) as usize] = 0x90; // NOP (distinguishable)
    bus.ram[(PM_CODE_BASE + np_handler_ip as u32 + 1) as usize] = 0xF4; // HLT

    // MOV AX, 0x0020; MOV DS, AX
    // Loading code segment into DS should #GP (wrong type), NOT #NP.
    place_at(
        &mut bus,
        PM_CODE_BASE,
        &[
            0xB8, 0x20, 0x00, // MOV AX, 0x0020
            0x8E, 0xD8, // MOV DS, AX
        ],
    );

    cpu.step(&mut bus); // MOV AX
    cpu.step(&mut bus); // MOV DS -> #GP (code segment not readable as data)
    cpu.step(&mut bus); // HLT at #GP handler

    assert!(cpu.halted());
    // If we ended up at #GP handler (PM_GP_HANDLER_IP + 1), it was #GP.
    // If we ended up at #NP handler (np_handler_ip + 2), it was #NP.
    assert_eq!(
        cpu.ip(),
        PM_GP_HANDLER_IP + 1,
        "Wrong-type descriptor should trigger #GP, not #NP"
    );
}

#[test]
fn i286_indirect_call_far_invalid_segment_sp_unchanged() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();

    let state = setup_protected_mode(&mut bus, 0xFFFF);
    cpu.load_state(&state);

    let sp_before = cpu.sp();

    // Set up memory with invalid far pointer: offset=0x1234, segment=0x0028 (no GDT entry).
    let ptr_addr = PM_DATA_BASE + 0x100;
    bus.ram[ptr_addr as usize] = 0x34;
    bus.ram[ptr_addr as usize + 1] = 0x12;
    bus.ram[ptr_addr as usize + 2] = 0x28; // invalid selector
    bus.ram[ptr_addr as usize + 3] = 0x00;

    // CALL FAR [0x0100] (FF /3, modrm for [disp16])
    // FF 1E 00 01 = CALL FAR [0x0100]
    place_at(&mut bus, PM_CODE_BASE, &[0xFF, 0x1E, 0x00, 0x01]);

    cpu.step(&mut bus); // CALL FAR -> fault before pushing

    // SP should be unchanged since segment validation happens before push.
    // Note: after the fault, we're at the #GP handler, so SP includes the
    // exception frame (FLAGS, CS, IP, error code = 8 bytes).
    // But the old CS:IP should NOT have been pushed before the fault.
    // The key insight: with the fix, load_segment happens first, and if it faults,
    // the push never happens. The SP change is ONLY from the exception frame.
    let sp_after_fault = cpu.sp();
    // Exception frame in PM: FLAGS + CS + IP + error_code = 8 bytes.
    assert_eq!(
        sp_before - sp_after_fault,
        8,
        "SP should only reflect the exception frame (8 bytes), not an extra 4 from CALL FAR push"
    );
}

#[test]
fn i286_reset_preserves_gp_registers() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();

    // Enter protected mode.
    let state = setup_protected_mode(&mut bus, 0xFFFF);
    cpu.load_state(&state);
    assert_ne!(cpu.msw & 1, 0, "should be in protected mode");

    // Set GP registers to known non-zero values.
    cpu.state.set_ax(0x1234);
    cpu.state.set_bx(0x5678);
    cpu.state.set_cx(0x9ABC);
    cpu.state.set_dx(0xDEF0);
    cpu.state.set_sp(0x1000);
    cpu.state.set_bp(0x2000);
    cpu.state.set_si(0x3000);
    cpu.state.set_di(0x4000);

    // Trigger reset (simulates port 0xF0 warm reset path).
    cpu.reset();

    // Control registers must be back to real-mode defaults.
    assert_eq!(cpu.msw & 1, 0, "PE bit should be clear after reset");
    assert_eq!(cpu.msw, 0xFFF0, "MSW should be reset value");

    // GP registers must be preserved (undocumented but relied upon).
    assert_eq!(cpu.state.ax(), 0x1234, "AX should survive reset");
    assert_eq!(cpu.state.bx(), 0x5678, "BX should survive reset");
    assert_eq!(cpu.state.cx(), 0x9ABC, "CX should survive reset");
    assert_eq!(cpu.state.dx(), 0xDEF0, "DX should survive reset");
    assert_eq!(cpu.state.sp(), 0x1000, "SP should survive reset");
    assert_eq!(cpu.state.bp(), 0x2000, "BP should survive reset");
    assert_eq!(cpu.state.si(), 0x3000, "SI should survive reset");
    assert_eq!(cpu.state.di(), 0x4000, "DI should survive reset");
}

const PM_RING3_CODE_BASE: u32 = 0x60000;
const PM_RING3_STACK_BASE: u32 = 0x20000;
const PM_TSS_BASE: u32 = 0x70000;

const PM_RING3_CS_SEL: u16 = 0x0023; // GDT index 4, RPL 3
const PM_RING3_DS_SEL: u16 = 0x002B; // GDT index 5, RPL 3
const PM_RING3_SS_SEL: u16 = 0x0033; // GDT index 6, RPL 3
const PM_TSS_SEL: u16 = 0x0038; // GDT index 7

const PM_TS_HANDLER_IP: u16 = 0x9000;
const PM_SS_HANDLER_IP: u16 = 0xB000;

fn write_word_at(bus: &mut TestBus, addr: u32, value: u16) {
    bus.ram[addr as usize] = value as u8;
    bus.ram[addr as usize + 1] = (value >> 8) as u8;
}

fn setup_protected_mode_with_ring3(bus: &mut TestBus) -> cpu::I286State {
    write_gdt_entry(bus, PM_GDT_BASE, 0, 0, 0, 0);
    // Ring 0: code (0x08), data (0x10), stack (0x18)
    write_gdt_entry(bus, PM_GDT_BASE, 1, PM_CODE_BASE, 0xFFFF, 0x9B);
    write_gdt_entry(bus, PM_GDT_BASE, 2, PM_DATA_BASE, 0xFFFF, 0x93);
    write_gdt_entry(bus, PM_GDT_BASE, 3, PM_STACK_BASE, 0xFFFF, 0x93);
    // Ring 3: code (0x20/0x23), data (0x28/0x2B), stack (0x30/0x33)
    write_gdt_entry(bus, PM_GDT_BASE, 4, PM_RING3_CODE_BASE, 0xFFFF, 0xFB);
    write_gdt_entry(bus, PM_GDT_BASE, 5, PM_DATA_BASE, 0xFFFF, 0xF3);
    write_gdt_entry(bus, PM_GDT_BASE, 6, PM_RING3_STACK_BASE, 0xFFFF, 0xF3);
    // TSS (0x38)
    write_gdt_entry(bus, PM_GDT_BASE, 7, PM_TSS_BASE, 43, 0x81);

    // IDT handlers for exception vectors
    write_idt_gate(bus, PM_IDT_BASE, 10, PM_TS_HANDLER_IP, PM_CS_SEL, 6, 0);
    write_idt_gate(bus, PM_IDT_BASE, 12, PM_SS_HANDLER_IP, PM_CS_SEL, 6, 0);
    write_idt_gate(bus, PM_IDT_BASE, 13, PM_GP_HANDLER_IP, PM_CS_SEL, 6, 0);

    // HLT at each handler
    bus.ram[(PM_CODE_BASE + PM_TS_HANDLER_IP as u32) as usize] = 0xF4;
    bus.ram[(PM_CODE_BASE + PM_SS_HANDLER_IP as u32) as usize] = 0xF4;
    bus.ram[(PM_CODE_BASE + PM_GP_HANDLER_IP as u32) as usize] = 0xF4;

    // TSS: ring 0 SS:SP
    write_word_at(bus, PM_TSS_BASE + 2, 0xFFF0); // SP0
    write_word_at(bus, PM_TSS_BASE + 4, PM_SS_SEL); // SS0

    let mut state = cpu::I286State::default();
    state.msw = 0x0001;

    state.set_sp(0xFFF0);

    state.set_cs(PM_CS_SEL);
    state.set_ds(PM_DS_SEL);
    state.set_ss(PM_SS_SEL);
    state.set_es(PM_DS_SEL);

    state.seg_bases[cpu::SegReg16::ES as usize] = PM_DATA_BASE;
    state.seg_bases[cpu::SegReg16::CS as usize] = PM_CODE_BASE;
    state.seg_bases[cpu::SegReg16::SS as usize] = PM_STACK_BASE;
    state.seg_bases[cpu::SegReg16::DS as usize] = PM_DATA_BASE;

    state.seg_limits = [0xFFFF; 4];

    state.seg_rights[cpu::SegReg16::ES as usize] = 0x93;
    state.seg_rights[cpu::SegReg16::CS as usize] = 0x9B;
    state.seg_rights[cpu::SegReg16::SS as usize] = 0x93;
    state.seg_rights[cpu::SegReg16::DS as usize] = 0x93;

    state.seg_valid = [true; 4];

    state.gdt_base = PM_GDT_BASE;
    state.gdt_limit = 8 * 8 - 1;
    state.idt_base = PM_IDT_BASE;
    state.idt_limit = 256 * 8 - 1;

    state.tr = PM_TSS_SEL;
    state.tr_base = PM_TSS_BASE;
    state.tr_limit = 43;

    state
}

fn make_ring3_state(state: &mut cpu::I286State) {
    state.set_cs(PM_RING3_CS_SEL);
    state.seg_bases[cpu::SegReg16::CS as usize] = PM_RING3_CODE_BASE;
    state.seg_rights[cpu::SegReg16::CS as usize] = 0xFB;

    state.set_ss(PM_RING3_SS_SEL);
    state.seg_bases[cpu::SegReg16::SS as usize] = PM_RING3_STACK_BASE;
    state.seg_rights[cpu::SegReg16::SS as usize] = 0xF3;

    state.set_ds(PM_RING3_DS_SEL);
    state.seg_bases[cpu::SegReg16::DS as usize] = PM_DATA_BASE;
    state.seg_rights[cpu::SegReg16::DS as usize] = 0xF3;

    state.set_es(PM_RING3_DS_SEL);
    state.seg_bases[cpu::SegReg16::ES as usize] = PM_DATA_BASE;
    state.seg_rights[cpu::SegReg16::ES as usize] = 0xF3;
}

#[test]
fn i286_ret_far_inter_privilege_ring0_to_ring3() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_ring3(&mut bus);
    cpu.load_state(&state);

    // RETF at CS:0x0000
    place_at(&mut bus, PM_CODE_BASE, &[0xCB]);

    // Ring 3 target: HLT
    let ring3_ip: u16 = 0x0100;
    bus.ram[(PM_RING3_CODE_BASE + ring3_ip as u32) as usize] = 0xF4;

    // Stack frame: IP, CS(ring3), SP(ring3), SS(ring3)
    let sp = cpu.sp();
    write_word_at(&mut bus, PM_STACK_BASE + sp as u32, ring3_ip);
    write_word_at(&mut bus, PM_STACK_BASE + sp as u32 + 2, PM_RING3_CS_SEL);
    write_word_at(&mut bus, PM_STACK_BASE + sp as u32 + 4, 0xFF00);
    write_word_at(&mut bus, PM_STACK_BASE + sp as u32 + 6, PM_RING3_SS_SEL);

    cpu.step(&mut bus); // RETF -> ring 3
    cpu.step(&mut bus); // HLT

    assert!(cpu.halted());
    assert_eq!(cpu.cs() & 3, 3, "CPL should be 3");
    assert_eq!(cpu.ss(), PM_RING3_SS_SEL);
    assert_eq!(cpu.sp(), 0xFF00);
}

#[test]
fn i286_ret_far_same_privilege() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_ring3(&mut bus);
    cpu.load_state(&state);

    // RETF at CS:0x0000
    place_at(&mut bus, PM_CODE_BASE, &[0xCB]);

    // Return target: HLT at CS:0x0200
    let target_ip: u16 = 0x0200;
    bus.ram[(PM_CODE_BASE + target_ip as u32) as usize] = 0xF4;

    // Stack: IP, CS (same ring 0 selector)
    let sp = cpu.sp();
    write_word_at(&mut bus, PM_STACK_BASE + sp as u32, target_ip);
    write_word_at(&mut bus, PM_STACK_BASE + sp as u32 + 2, PM_CS_SEL);

    cpu.step(&mut bus); // RETF
    cpu.step(&mut bus); // HLT

    assert!(cpu.halted());
    assert_eq!(cpu.cs() & 3, 0, "CPL should remain 0");
    assert_eq!(cpu.sp(), 0xFFF4, "SP should be old SP + 4");
}

#[test]
fn i286_iret_inter_privilege_ring0_to_ring3() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_ring3(&mut bus);
    cpu.load_state(&state);

    // IRET at CS:0x0000
    place_at(&mut bus, PM_CODE_BASE, &[0xCF]);

    let ring3_ip: u16 = 0x0100;
    bus.ram[(PM_RING3_CODE_BASE + ring3_ip as u32) as usize] = 0xF4;

    // IRET stack: IP, CS, FLAGS, SP, SS
    let sp = cpu.sp();
    write_word_at(&mut bus, PM_STACK_BASE + sp as u32, ring3_ip);
    write_word_at(&mut bus, PM_STACK_BASE + sp as u32 + 2, PM_RING3_CS_SEL);
    write_word_at(&mut bus, PM_STACK_BASE + sp as u32 + 4, 0x0202); // FLAGS (IF=1)
    write_word_at(&mut bus, PM_STACK_BASE + sp as u32 + 6, 0xFF00);
    write_word_at(&mut bus, PM_STACK_BASE + sp as u32 + 8, PM_RING3_SS_SEL);

    cpu.step(&mut bus); // IRET -> ring 3
    cpu.step(&mut bus); // HLT

    assert!(cpu.halted());
    assert_eq!(cpu.cs() & 3, 3, "CPL should be 3 after IRET");
    assert_eq!(cpu.ss(), PM_RING3_SS_SEL);
    assert_eq!(cpu.sp(), 0xFF00);
}

#[test]
fn i286_ret_far_rpl_less_than_cpl_faults() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();
    let mut state = setup_protected_mode_with_ring3(&mut bus);
    make_ring3_state(&mut state);
    state.set_sp(0xFFF0);
    cpu.load_state(&state);

    // RETF at ring 3 code
    place_at(&mut bus, PM_RING3_CODE_BASE, &[0xCB]);

    // Stack: return IP, CS with RPL=0 (trying to return to ring 0 from ring 3)
    write_word_at(&mut bus, PM_RING3_STACK_BASE + 0xFFF0, 0x0000);
    write_word_at(&mut bus, PM_RING3_STACK_BASE + 0xFFF2, PM_CS_SEL); // RPL=0

    cpu.step(&mut bus); // RETF -> #GP (RPL < CPL)
    cpu.step(&mut bus); // HLT at #GP handler

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_GP_HANDLER_IP + 1, "Should be at #GP handler");
}

#[test]
fn i286_ret_far_nonconforming_dpl_ne_rpl_faults() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_ring3(&mut bus);
    cpu.load_state(&state);

    // Create a code segment at GDT index 8 with DPL=0 but we'll use RPL=3.
    // Non-conforming code: DPL(0) != RPL(3) -> fault.
    write_gdt_entry(&mut bus, PM_GDT_BASE, 8, PM_RING3_CODE_BASE, 0xFFFF, 0x9B);
    // Extend GDT limit
    cpu.state.gdt_limit = 9 * 8 - 1;

    // RETF at CS:0x0000
    place_at(&mut bus, PM_CODE_BASE, &[0xCB]);

    // Stack: IP, CS = selector 0x0043 (GDT index 8, RPL=3)
    // DPL=0 in descriptor, RPL=3 in selector -> DPL != RPL -> #GP
    let sp = cpu.sp();
    write_word_at(&mut bus, PM_STACK_BASE + sp as u32, 0x0100);
    write_word_at(&mut bus, PM_STACK_BASE + sp as u32 + 2, 0x0043);
    // Inter-privilege: also provide SP/SS
    write_word_at(&mut bus, PM_STACK_BASE + sp as u32 + 4, 0xFF00);
    write_word_at(&mut bus, PM_STACK_BASE + sp as u32 + 6, PM_RING3_SS_SEL);

    cpu.step(&mut bus); // RETF -> #GP (DPL != RPL)
    cpu.step(&mut bus); // HLT at #GP handler

    assert!(cpu.halted());
    assert_eq!(
        cpu.ip(),
        PM_GP_HANDLER_IP + 1,
        "Should fault: non-conforming DPL != RPL"
    );
}

#[test]
fn i286_interrupt_inter_privilege_ring3_to_ring0() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();
    let mut state = setup_protected_mode_with_ring3(&mut bus);
    make_ring3_state(&mut state);
    state.set_sp(0xFFF0);
    state.flags.expand(0x0202); // IF=1
    cpu.load_state(&state);

    // Ring 3 code: NOP (something to execute after interrupt)
    place_at(&mut bus, PM_RING3_CODE_BASE, &[0x90]);

    // Set up IDT gate for vector 0x20 (DPL=0, INT gate -> ring 0)
    // Note: hardware interrupt ignores gate DPL check.
    write_idt_gate(&mut bus, PM_IDT_BASE, 0x20, 0x0300, PM_CS_SEL, 6, 0);
    bus.ram[(PM_CODE_BASE + 0x0300) as usize] = 0xF4; // HLT at handler

    // Trigger hardware IRQ
    bus.irq_vector = 0x20;
    cpu.signal_irq();

    cpu.step(&mut bus); // Processes IRQ -> switches to ring 0 -> HLT

    assert!(cpu.halted());
    assert_eq!(cpu.cs() & 3, 0, "CPL should be 0 after interrupt");
    assert_eq!(cpu.ss(), PM_SS_SEL, "SS should be ring 0 SS from TSS");
}

#[test]
fn i286_interrupt_inter_privilege_ss_fault_uses_ts_vector() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();
    let mut state = setup_protected_mode_with_ring3(&mut bus);
    make_ring3_state(&mut state);
    state.set_sp(0xFFF0);
    state.flags.expand(0x0202);
    cpu.load_state(&state);

    place_at(&mut bus, PM_RING3_CODE_BASE, &[0x90]);

    // Set up IDT gate for vector 0x20 targeting ring 0 code
    write_idt_gate(&mut bus, PM_IDT_BASE, 0x20, 0x0300, PM_CS_SEL, 6, 0);

    // #TS handler (vector 10) in ring 3 code to avoid cascade.
    // Using ring 3 code segment (selector 0x0020, DPL=3) so the dispatch
    // is same-privilege (ring 3 -> ring 3) and doesn't need TSS stack switch.
    let ts_handler_ip: u16 = 0x0500;
    write_idt_gate(&mut bus, PM_IDT_BASE, 10, ts_handler_ip, 0x0020, 6, 0);
    bus.ram[(PM_RING3_CODE_BASE + ts_handler_ip as u32) as usize] = 0xF4; // HLT

    // TSS: null SS for ring 0 -> will fail SS validation
    write_word_at(&mut bus, PM_TSS_BASE + 4, 0x0000);

    bus.irq_vector = 0x20;
    cpu.signal_irq();

    // One step: IRQ dispatch -> null SS -> #TS -> same-privilege dispatch -> HLT
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(
        cpu.ip(),
        ts_handler_ip + 1,
        "Invalid SS should trigger #TS (vector 10), not #SS (vector 12)"
    );
}

#[test]
fn i286_ret_far_imm_inter_privilege_reads_correct_offset() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_ring3(&mut bus);
    cpu.load_state(&state);

    // RETF 4 at CS:0x0000 (opcode 0xCA, imm16=4)
    place_at(&mut bus, PM_CODE_BASE, &[0xCA, 0x04, 0x00]);

    let ring3_ip: u16 = 0x0100;
    bus.ram[(PM_RING3_CODE_BASE + ring3_ip as u32) as usize] = 0xF4;

    // Stack layout for RETF 4 inter-privilege:
    // SP+0: IP
    // SP+2: CS
    // SP+4..SP+7: 4 bytes of parameters (imm=4)
    // SP+8: new SP  (SP+4+imm)
    // SP+10: new SS (SP+6+imm)
    let sp = cpu.sp();
    write_word_at(&mut bus, PM_STACK_BASE + sp as u32, ring3_ip);
    write_word_at(&mut bus, PM_STACK_BASE + sp as u32 + 2, PM_RING3_CS_SEL);
    // 4 bytes of parameters at SP+4 (don't care about content)
    write_word_at(&mut bus, PM_STACK_BASE + sp as u32 + 8, 0xFE00); // new SP
    write_word_at(&mut bus, PM_STACK_BASE + sp as u32 + 10, PM_RING3_SS_SEL); // new SS

    cpu.step(&mut bus); // RETF 4 -> ring 3
    cpu.step(&mut bus); // HLT

    assert!(cpu.halted());
    assert_eq!(cpu.cs() & 3, 3, "CPL should be 3");
    assert_eq!(cpu.ss(), PM_RING3_SS_SEL, "SS should be ring 3");
    // SP = new_sp + imm = 0xFE00 + 4 = 0xFE04
    assert_eq!(cpu.sp(), 0xFE04, "SP should be new_sp + imm");
}

#[test]
fn i286_ret_far_ip_exceeds_limit_faults() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_ring3(&mut bus);
    cpu.load_state(&state);

    // Create a code segment with a small limit at GDT index 8
    write_gdt_entry(&mut bus, PM_GDT_BASE, 8, PM_CODE_BASE, 0x0100, 0x9B);
    cpu.state.gdt_limit = 9 * 8 - 1;

    place_at(&mut bus, PM_CODE_BASE, &[0xCB]); // RETF

    // Stack: return IP=0x0200 (> limit 0x0100), same-ring CS
    let sp = cpu.sp();
    write_word_at(&mut bus, PM_STACK_BASE + sp as u32, 0x0200); // IP > limit
    write_word_at(&mut bus, PM_STACK_BASE + sp as u32 + 2, 0x0040); // sel 0x40 = GDT index 8

    cpu.step(&mut bus); // RETF -> #GP (IP > limit)
    cpu.step(&mut bus); // HLT at #GP handler

    assert!(cpu.halted());
    assert_eq!(
        cpu.ip(),
        PM_GP_HANDLER_IP + 1,
        "IP > limit should trigger #GP"
    );

    // Verify error code is 0 (per spec)
    let handler_sp = cpu.sp();
    let error_code = read_word_at(&bus, PM_STACK_BASE + handler_sp as u32);
    assert_eq!(error_code, 0, "Error code should be 0 for IP > limit");
}

#[test]
fn i286_invalidate_nonconforming_code_in_ds_on_return() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_ring3(&mut bus);
    cpu.load_state(&state);

    // DS is loaded with non-conforming code segment DPL=0 (rights 0x9B)
    // (readable code, DPL=0, non-conforming)
    cpu.state.seg_rights[cpu::SegReg16::DS as usize] = 0x9B;

    // RETF
    place_at(&mut bus, PM_CODE_BASE, &[0xCB]);

    let ring3_ip: u16 = 0x0100;
    bus.ram[(PM_RING3_CODE_BASE + ring3_ip as u32) as usize] = 0xF4;

    let sp = cpu.sp();
    write_word_at(&mut bus, PM_STACK_BASE + sp as u32, ring3_ip);
    write_word_at(&mut bus, PM_STACK_BASE + sp as u32 + 2, PM_RING3_CS_SEL);
    write_word_at(&mut bus, PM_STACK_BASE + sp as u32 + 4, 0xFF00);
    write_word_at(&mut bus, PM_STACK_BASE + sp as u32 + 6, PM_RING3_SS_SEL);

    cpu.step(&mut bus); // RETF -> ring 3

    assert_eq!(cpu.cs() & 3, 3);
    assert!(
        !cpu.state.seg_valid[cpu::SegReg16::DS as usize],
        "DS with non-conforming code DPL=0 should be invalidated at ring 3"
    );
}

#[test]
fn i286_keep_nonconforming_code_in_ds_when_dpl_sufficient() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_ring3(&mut bus);
    cpu.load_state(&state);

    // DS with non-conforming code DPL=3 (rights 0xFB)
    cpu.state.seg_rights[cpu::SegReg16::DS as usize] = 0xFB;

    place_at(&mut bus, PM_CODE_BASE, &[0xCB]);

    let ring3_ip: u16 = 0x0100;
    bus.ram[(PM_RING3_CODE_BASE + ring3_ip as u32) as usize] = 0xF4;

    let sp = cpu.sp();
    write_word_at(&mut bus, PM_STACK_BASE + sp as u32, ring3_ip);
    write_word_at(&mut bus, PM_STACK_BASE + sp as u32 + 2, PM_RING3_CS_SEL);
    write_word_at(&mut bus, PM_STACK_BASE + sp as u32 + 4, 0xFF00);
    write_word_at(&mut bus, PM_STACK_BASE + sp as u32 + 6, PM_RING3_SS_SEL);

    cpu.step(&mut bus); // RETF -> ring 3

    assert_eq!(cpu.cs() & 3, 3);
    assert!(
        cpu.state.seg_valid[cpu::SegReg16::DS as usize],
        "DS with non-conforming code DPL=3 should NOT be invalidated at ring 3"
    );
}

#[test]
fn i286_keep_conforming_code_in_ds_on_return() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_ring3(&mut bus);
    cpu.load_state(&state);

    // DS with conforming code DPL=0 (rights 0x9F: P=1, DPL=0, S=1, code, conforming, readable, accessed)
    cpu.state.seg_rights[cpu::SegReg16::DS as usize] = 0x9F;

    place_at(&mut bus, PM_CODE_BASE, &[0xCB]);

    let ring3_ip: u16 = 0x0100;
    bus.ram[(PM_RING3_CODE_BASE + ring3_ip as u32) as usize] = 0xF4;

    let sp = cpu.sp();
    write_word_at(&mut bus, PM_STACK_BASE + sp as u32, ring3_ip);
    write_word_at(&mut bus, PM_STACK_BASE + sp as u32 + 2, PM_RING3_CS_SEL);
    write_word_at(&mut bus, PM_STACK_BASE + sp as u32 + 4, 0xFF00);
    write_word_at(&mut bus, PM_STACK_BASE + sp as u32 + 6, PM_RING3_SS_SEL);

    cpu.step(&mut bus); // RETF -> ring 3

    assert_eq!(cpu.cs() & 3, 3);
    assert!(
        cpu.state.seg_valid[cpu::SegReg16::DS as usize],
        "DS with conforming code should never be invalidated"
    );
}

#[test]
fn i286_ss_limit_violation_uses_ss_vector() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();
    let mut state = setup_protected_mode_with_ring3(&mut bus);
    make_ring3_state(&mut state);
    // Ring 3 SS with small limit
    state.seg_limits[cpu::SegReg16::SS as usize] = 0x0004;
    state.set_sp(0xFFF0); // Way beyond limit
    state.flags.expand(0x0002);
    cpu.load_state(&state);

    // PUSH AX - writes to SS:SP-2, which exceeds limit
    place_at(&mut bus, PM_RING3_CODE_BASE, &[0x50]);

    cpu.step(&mut bus); // PUSH AX -> SS limit fault -> #SS
    cpu.step(&mut bus); // HLT at #SS handler

    assert!(cpu.halted());
    assert_eq!(
        cpu.ip(),
        PM_SS_HANDLER_IP + 1,
        "SS limit violation should trigger #SS (vector 12), not #GP (vector 13)"
    );

    // Verify error code is 0 on the ring 0 handler stack
    let handler_sp = cpu.sp();
    let error_code = read_word_at(&bus, PM_STACK_BASE + handler_sp as u32);
    assert_eq!(
        error_code, 0,
        "Error code for runtime SS violation should be 0"
    );
}

#[test]
fn i286_ds_limit_violation_uses_gp_vector() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_ring3(&mut bus);
    cpu.load_state(&state);

    // Set DS limit to 0x0010
    cpu.state.seg_limits[cpu::SegReg16::DS as usize] = 0x0010;

    // MOV [0x0020], AX (opcode 0xA3 + imm16) - writes to DS:0x0020 > limit
    place_at(&mut bus, PM_CODE_BASE, &[0xA3, 0x20, 0x00]);

    cpu.step(&mut bus); // MOV -> DS limit fault -> #GP
    cpu.step(&mut bus); // HLT at #GP handler

    assert!(cpu.halted());
    assert_eq!(
        cpu.ip(),
        PM_GP_HANDLER_IP + 1,
        "DS limit violation should trigger #GP (vector 13)"
    );

    // Verify error code is 0
    let handler_sp = cpu.sp();
    let error_code = read_word_at(&bus, PM_STACK_BASE + handler_sp as u32);
    assert_eq!(
        error_code, 0,
        "Error code for runtime DS violation should be 0"
    );
}

#[test]
fn i286_hardware_interrupt_error_code_has_ext_bit() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_ring3(&mut bus);
    cpu.load_state(&state);

    // Shrink IDT to cover only vectors 0-13 (14*8 = 112 bytes, limit = 111)
    cpu.state.idt_limit = 14 * 8 - 1;
    cpu.state.flags.expand(0x0202); // IF=1

    // NOP at code start
    place_at(&mut bus, PM_CODE_BASE, &[0x90]);

    // Trigger hardware IRQ for vector 14 (beyond IDT limit)
    bus.irq_vector = 14;
    cpu.signal_irq();

    // One step: IRQ dispatch -> IDT bounds fault -> #GP dispatch -> HLT
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_GP_HANDLER_IP + 1);

    // Error code should be: vector*8 + 2 + ext = 14*8 + 2 + 1 = 115
    let handler_sp = cpu.sp();
    let error_code = read_word_at(&bus, PM_STACK_BASE + handler_sp as u32);
    assert_eq!(
        error_code,
        14 * 8 + 2 + 1,
        "Hardware interrupt IDT fault error code should include EXT bit (bit 0 = 1)"
    );
}

#[test]
fn i286_software_interrupt_error_code_no_ext_bit() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_with_ring3(&mut bus);
    cpu.load_state(&state);

    // Shrink IDT to cover only vectors 0-13
    cpu.state.idt_limit = 14 * 8 - 1;

    // INT 14 (CD 0E) - software interrupt for vector beyond IDT limit
    place_at(&mut bus, PM_CODE_BASE, &[0xCD, 0x0E]);

    // Step 1: INT 14 -> IDT bounds fault -> #GP dispatch
    cpu.step(&mut bus);
    // Step 2: HLT at #GP handler
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PM_GP_HANDLER_IP + 1);

    // Error code should be: vector*8 + 2 + 0 = 14*8 + 2 = 114 (no EXT bit)
    let handler_sp = cpu.sp();
    let error_code = read_word_at(&bus, PM_STACK_BASE + handler_sp as u32);
    assert_eq!(
        error_code,
        14 * 8 + 2,
        "Software interrupt IDT fault error code should NOT include EXT bit"
    );
}

const PM_TSS2_BASE: u32 = 0x71000;
const PM_NP_HANDLER_IP: u16 = 0xA000;
const PM_TSS2_SEL: u16 = 0x0048; // GDT index 9
const PM_CONFORMING_CS_SEL: u16 = 0x0058; // GDT index 11

#[allow(clippy::too_many_arguments)]
fn write_gdt_gate(
    bus: &mut TestBus,
    gdt_base: u32,
    entry_index: u16,
    offset: u16,
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
    bus.ram[addr + 6] = 0;
    bus.ram[addr + 7] = 0;
}

#[allow(clippy::too_many_arguments)]
fn write_tss(
    bus: &mut TestBus,
    tss_base: u32,
    backlink: u16,
    sp0: u16,
    ss0: u16,
    ip: u16,
    flags: u16,
    ax: u16,
    cx: u16,
    dx: u16,
    bx: u16,
    sp: u16,
    bp: u16,
    si: u16,
    di: u16,
    es: u16,
    cs: u16,
    ss: u16,
    ds: u16,
    ldt: u16,
) {
    write_word_at(bus, tss_base, backlink);
    write_word_at(bus, tss_base + 2, sp0);
    write_word_at(bus, tss_base + 4, ss0);
    write_word_at(bus, tss_base + 6, 0); // SP1
    write_word_at(bus, tss_base + 8, 0); // SS1
    write_word_at(bus, tss_base + 10, 0); // SP2
    write_word_at(bus, tss_base + 12, 0); // SS2
    write_word_at(bus, tss_base + 14, ip);
    write_word_at(bus, tss_base + 16, flags);
    write_word_at(bus, tss_base + 18, ax);
    write_word_at(bus, tss_base + 20, cx);
    write_word_at(bus, tss_base + 22, dx);
    write_word_at(bus, tss_base + 24, bx);
    write_word_at(bus, tss_base + 26, sp);
    write_word_at(bus, tss_base + 28, bp);
    write_word_at(bus, tss_base + 30, si);
    write_word_at(bus, tss_base + 32, di);
    write_word_at(bus, tss_base + 34, es);
    write_word_at(bus, tss_base + 36, cs);
    write_word_at(bus, tss_base + 38, ss);
    write_word_at(bus, tss_base + 40, ds);
    write_word_at(bus, tss_base + 42, ldt);
}

fn setup_protected_mode_extended(bus: &mut TestBus) -> cpu::I286State {
    // Null descriptor
    write_gdt_entry(bus, PM_GDT_BASE, 0, 0, 0, 0);
    // Ring 0: code (0x08), data (0x10), stack (0x18)
    write_gdt_entry(bus, PM_GDT_BASE, 1, PM_CODE_BASE, 0xFFFF, 0x9B);
    write_gdt_entry(bus, PM_GDT_BASE, 2, PM_DATA_BASE, 0xFFFF, 0x93);
    write_gdt_entry(bus, PM_GDT_BASE, 3, PM_STACK_BASE, 0xFFFF, 0x93);
    // Ring 3: code (0x20/0x23), data (0x28/0x2B), stack (0x30/0x33)
    write_gdt_entry(bus, PM_GDT_BASE, 4, PM_RING3_CODE_BASE, 0xFFFF, 0xFB);
    write_gdt_entry(bus, PM_GDT_BASE, 5, PM_DATA_BASE, 0xFFFF, 0xF3);
    write_gdt_entry(bus, PM_GDT_BASE, 6, PM_RING3_STACK_BASE, 0xFFFF, 0xF3);
    // TSS1 (0x38) - current task, idle type 1 (will be marked busy by state)
    write_gdt_entry(bus, PM_GDT_BASE, 7, PM_TSS_BASE, 43, 0x83);
    // Reserved entries 8-10 for gates/TSS2 (filled by tests)
    // Entry 8: call gate slot (0x40)
    write_gdt_entry(bus, PM_GDT_BASE, 8, 0, 0, 0);
    // Entry 9: TSS2 (0x48) - second task, idle
    write_gdt_entry(bus, PM_GDT_BASE, 9, PM_TSS2_BASE, 43, 0x81);
    // Entry 10: task gate slot (0x50)
    write_gdt_entry(bus, PM_GDT_BASE, 10, 0, 0, 0);
    // Entry 11: conforming code segment (0x58)
    write_gdt_entry(bus, PM_GDT_BASE, 11, PM_CODE_BASE, 0xFFFF, 0x9F);

    // IDT
    write_idt_gate(bus, PM_IDT_BASE, 10, PM_TS_HANDLER_IP, PM_CS_SEL, 6, 0);
    write_idt_gate(bus, PM_IDT_BASE, 11, PM_NP_HANDLER_IP, PM_CS_SEL, 6, 0);
    write_idt_gate(bus, PM_IDT_BASE, 12, PM_SS_HANDLER_IP, PM_CS_SEL, 6, 0);
    write_idt_gate(bus, PM_IDT_BASE, 13, PM_GP_HANDLER_IP, PM_CS_SEL, 6, 0);

    // HLT at each handler
    bus.ram[(PM_CODE_BASE + PM_TS_HANDLER_IP as u32) as usize] = 0xF4;
    bus.ram[(PM_CODE_BASE + PM_NP_HANDLER_IP as u32) as usize] = 0xF4;
    bus.ram[(PM_CODE_BASE + PM_SS_HANDLER_IP as u32) as usize] = 0xF4;
    bus.ram[(PM_CODE_BASE + PM_GP_HANDLER_IP as u32) as usize] = 0xF4;

    // TSS1: ring 0 stack
    write_word_at(bus, PM_TSS_BASE + 2, 0xFFF0); // SP0
    write_word_at(bus, PM_TSS_BASE + 4, PM_SS_SEL); // SS0

    let mut state = cpu::I286State::default();
    state.msw = 0x0001;

    state.set_sp(0xFFF0);
    state.set_cs(PM_CS_SEL);
    state.set_ds(PM_DS_SEL);
    state.set_ss(PM_SS_SEL);
    state.set_es(PM_DS_SEL);

    state.seg_bases[cpu::SegReg16::ES as usize] = PM_DATA_BASE;
    state.seg_bases[cpu::SegReg16::CS as usize] = PM_CODE_BASE;
    state.seg_bases[cpu::SegReg16::SS as usize] = PM_STACK_BASE;
    state.seg_bases[cpu::SegReg16::DS as usize] = PM_DATA_BASE;

    state.seg_limits = [0xFFFF; 4];
    state.seg_rights[cpu::SegReg16::ES as usize] = 0x93;
    state.seg_rights[cpu::SegReg16::CS as usize] = 0x9B;
    state.seg_rights[cpu::SegReg16::SS as usize] = 0x93;
    state.seg_rights[cpu::SegReg16::DS as usize] = 0x93;
    state.seg_valid = [true; 4];

    state.gdt_base = PM_GDT_BASE;
    state.gdt_limit = 12 * 8 - 1;
    state.idt_base = PM_IDT_BASE;
    state.idt_limit = 256 * 8 - 1;

    state.tr = PM_TSS_SEL;
    state.tr_base = PM_TSS_BASE;
    state.tr_limit = 43;
    state.tr_rights = 0x83;

    state
}

#[test]
fn i286_ss_type_violation_raises_gp_not_ss() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_extended(&mut bus);
    cpu.load_state(&state);

    // Put a code segment descriptor at GDT entry 3 (SS slot) - type violation.
    write_gdt_entry(&mut bus, PM_GDT_BASE, 3, PM_STACK_BASE, 0xFFFF, 0x9B);

    // MOV AX, 0x0018; MOV SS, AX
    place_at(&mut bus, PM_CODE_BASE, &[0xB8, 0x18, 0x00, 0x8E, 0xD0]);

    cpu.step(&mut bus); // MOV AX, 0x0018
    cpu.step(&mut bus); // MOV SS, AX -> should fault
    cpu.step(&mut bus); // HLT in handler

    assert!(cpu.halted());
    assert_eq!(
        cpu.ip(),
        PM_GP_HANDLER_IP + 1,
        "SS type violation should go to #GP handler (vector 13), not #SS"
    );
}

#[test]
fn i286_ss_privilege_violation_raises_gp_not_ss() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_extended(&mut bus);
    cpu.load_state(&state);

    // Put a ring-3 writable data segment at GDT entry 3 (SS slot).
    // CPL=0 but DPL=3 -> privilege mismatch -> should be #GP.
    write_gdt_entry(&mut bus, PM_GDT_BASE, 3, PM_STACK_BASE, 0xFFFF, 0xF3);

    // MOV AX, 0x0018; MOV SS, AX
    place_at(&mut bus, PM_CODE_BASE, &[0xB8, 0x18, 0x00, 0x8E, 0xD0]);

    cpu.step(&mut bus); // MOV AX, 0x0018
    cpu.step(&mut bus); // MOV SS, AX -> should fault
    cpu.step(&mut bus); // HLT in handler

    assert!(cpu.halted());
    assert_eq!(
        cpu.ip(),
        PM_GP_HANDLER_IP + 1,
        "SS privilege violation should go to #GP handler (vector 13), not #SS"
    );
}

#[test]
fn i286_ss_not_present_raises_ss() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_extended(&mut bus);
    cpu.load_state(&state);

    // Put a not-present writable data segment at GDT entry 3.
    // Type and privilege are correct, but not present -> should be #SS (vector 12).
    write_gdt_entry(&mut bus, PM_GDT_BASE, 3, PM_STACK_BASE, 0xFFFF, 0x13);

    // MOV AX, 0x0018; MOV SS, AX
    place_at(&mut bus, PM_CODE_BASE, &[0xB8, 0x18, 0x00, 0x8E, 0xD0]);

    cpu.step(&mut bus); // MOV AX, 0x0018
    cpu.step(&mut bus); // MOV SS, AX -> should fault
    cpu.step(&mut bus); // HLT in handler

    assert!(cpu.halted());
    assert_eq!(
        cpu.ip(),
        PM_SS_HANDLER_IP + 1,
        "SS not-present should go to #SS handler (vector 12)"
    );
}

#[test]
fn i286_ret_far_to_conforming_code_adopts_rpl() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_extended(&mut bus);
    cpu.load_state(&state);

    // Conforming code segment is at GDT index 11 (selector 0x58), DPL=0.
    // We'll do a far return to selector 0x5B (RPL=3 in conforming segment).
    // After return, CPL should be 3 (from RPL), not 0 (old CPL).
    let conforming_sel_rpl3: u16 = PM_CONFORMING_CS_SEL | 3; // 0x005B
    let target_ip: u16 = 0x0200;

    // HLT at target
    bus.ram[(PM_CODE_BASE + target_ip as u32) as usize] = 0xF4;

    // Build stack frame for RETF: [IP, CS(conforming RPL3), SP_new, SS_new]
    // This is an inter-privilege return (RPL=3 > CPL=0).
    let sp = cpu.sp();
    let ss_base = PM_STACK_BASE;
    write_word_at(&mut bus, ss_base + sp as u32, target_ip);
    write_word_at(&mut bus, ss_base + sp as u32 + 2, conforming_sel_rpl3);
    // New stack for ring 3.
    write_word_at(&mut bus, ss_base + sp as u32 + 4, 0xF000); // new SP
    write_word_at(&mut bus, ss_base + sp as u32 + 6, PM_RING3_SS_SEL); // new SS

    // RETF
    place_at(&mut bus, PM_CODE_BASE, &[0xCB]);

    cpu.step(&mut bus); // RETF
    cpu.step(&mut bus); // HLT

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), target_ip + 1);
    assert_eq!(
        cpu.cs() & 3,
        3,
        "After far return to conforming code with RPL=3, CPL should be 3"
    );
}

#[test]
fn i286_call_far_through_call_gate_same_privilege() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_extended(&mut bus);
    cpu.load_state(&state);

    // Call gate at GDT entry 8 (selector 0x40): target is CS_SEL offset 0x100, 0 params.
    let gate_target_ip: u16 = 0x0100;
    write_gdt_gate(&mut bus, PM_GDT_BASE, 8, gate_target_ip, PM_CS_SEL, 0, 4, 0);

    // HLT at target
    bus.ram[(PM_CODE_BASE + gate_target_ip as u32) as usize] = 0xF4;

    // CALL FAR 0x0040:0x9999 - the offset in the instruction is ignored; gate offset is used.
    place_at(&mut bus, PM_CODE_BASE, &[0x9A, 0x99, 0x99, 0x40, 0x00]);

    let old_cs = cpu.cs();
    let old_ip_after_call = 5u16; // IP after fetching the 5-byte CALL FAR instruction

    cpu.step(&mut bus); // CALL FAR
    cpu.step(&mut bus); // HLT

    assert!(cpu.halted());
    assert_eq!(
        cpu.ip(),
        gate_target_ip + 1,
        "Should execute at gate target offset, not instruction offset"
    );

    // Old CS:IP should be on stack.
    let sp = cpu.sp();
    let pushed_ip = read_word_at(&bus, PM_STACK_BASE + sp as u32);
    let pushed_cs = read_word_at(&bus, PM_STACK_BASE + sp as u32 + 2);
    assert_eq!(pushed_ip, old_ip_after_call);
    assert_eq!(pushed_cs, old_cs);
}

#[test]
fn i286_call_far_through_call_gate_inter_privilege() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();
    let mut state = setup_protected_mode_extended(&mut bus);

    // Start at ring 3.
    make_ring3_state(&mut state);
    cpu.load_state(&state);

    // Call gate at GDT entry 8: ring 0 code, DPL=3 so ring 3 can use it.
    let gate_target_ip: u16 = 0x0100;
    write_gdt_gate(&mut bus, PM_GDT_BASE, 8, gate_target_ip, PM_CS_SEL, 0, 4, 3);

    // HLT at ring 0 target
    bus.ram[(PM_CODE_BASE + gate_target_ip as u32) as usize] = 0xF4;

    // CALL FAR 0x0040:0x0000 (gate selector, offset ignored)
    place_at(
        &mut bus,
        PM_RING3_CODE_BASE,
        &[0x9A, 0x00, 0x00, 0x40, 0x00],
    );

    let old_ss = cpu.ss();
    let old_sp = cpu.sp();
    let old_cs = cpu.cs();

    cpu.step(&mut bus); // CALL FAR through gate
    cpu.step(&mut bus); // HLT

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), gate_target_ip + 1);
    assert_eq!(
        cpu.cs() & 3,
        0,
        "Should be at ring 0 after inter-privilege call"
    );

    // New stack should contain: old_SS, old_SP, old_CS, old_IP.
    let sp = cpu.sp();
    let ss_base = cpu.state.seg_bases[cpu::SegReg16::SS as usize];
    let pushed_ip = read_word_at(&bus, ss_base + sp as u32);
    let pushed_cs = read_word_at(&bus, ss_base + sp as u32 + 2);
    let pushed_sp = read_word_at(&bus, ss_base + sp as u32 + 4);
    let pushed_ss = read_word_at(&bus, ss_base + sp as u32 + 6);
    assert_eq!(pushed_ss, old_ss, "Old SS should be on new stack");
    assert_eq!(pushed_sp, old_sp, "Old SP should be on new stack");
    assert_eq!(pushed_cs, old_cs, "Old CS should be on new stack");
    assert_eq!(
        pushed_ip, 5,
        "Old IP (after CALL FAR) should be on new stack"
    );
}

#[test]
fn i286_call_far_through_call_gate_with_parameters() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();
    let mut state = setup_protected_mode_extended(&mut bus);

    // Start at ring 3.
    make_ring3_state(&mut state);
    cpu.load_state(&state);

    // Call gate: ring 0 code, DPL=3, 2 parameters.
    let gate_target_ip: u16 = 0x0100;
    write_gdt_gate(&mut bus, PM_GDT_BASE, 8, gate_target_ip, PM_CS_SEL, 2, 4, 3);

    // HLT at target
    bus.ram[(PM_CODE_BASE + gate_target_ip as u32) as usize] = 0xF4;

    // Push two parameter words onto ring 3 stack before CALL.
    let r3_sp = cpu.sp();
    let r3_ss_base = PM_RING3_STACK_BASE;
    write_word_at(&mut bus, r3_ss_base + r3_sp as u32 - 2, 0xBBBB); // param 2 (pushed first)
    write_word_at(&mut bus, r3_ss_base + r3_sp as u32 - 4, 0xAAAA); // param 1
    cpu.state.set_sp(r3_sp - 4);

    // CALL FAR 0x0040:0x0000
    place_at(
        &mut bus,
        PM_RING3_CODE_BASE,
        &[0x9A, 0x00, 0x00, 0x40, 0x00],
    );

    cpu.step(&mut bus); // CALL FAR
    cpu.step(&mut bus); // HLT

    assert!(cpu.halted());

    // New stack should contain: old_SS, old_SP, param1, param2, old_CS, old_IP.
    let sp = cpu.sp();
    let ss_base = cpu.state.seg_bases[cpu::SegReg16::SS as usize];
    let pushed_ip = read_word_at(&bus, ss_base + sp as u32);
    let param1 = read_word_at(&bus, ss_base + sp as u32 + 4);
    let param2 = read_word_at(&bus, ss_base + sp as u32 + 6);
    assert_eq!(pushed_ip, 5);
    assert_eq!(
        param1, 0xAAAA,
        "First parameter should be copied to new stack"
    );
    assert_eq!(
        param2, 0xBBBB,
        "Second parameter should be copied to new stack"
    );
    // old_SP and old_SS are further up the stack.
    let pushed_sp = read_word_at(&bus, ss_base + sp as u32 + 8);
    let pushed_ss = read_word_at(&bus, ss_base + sp as u32 + 10);
    assert_eq!(pushed_ss & 3, 3, "Pushed SS should be ring 3 selector");
    assert_eq!(
        pushed_sp,
        r3_sp - 4,
        "Pushed SP should be ring 3 SP before CALL"
    );
}

#[test]
fn i286_jmp_far_through_call_gate_same_privilege() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_extended(&mut bus);
    cpu.load_state(&state);

    // Call gate at GDT entry 8: same-privilege, target offset 0x0100.
    let gate_target_ip: u16 = 0x0100;
    write_gdt_gate(&mut bus, PM_GDT_BASE, 8, gate_target_ip, PM_CS_SEL, 0, 4, 0);

    // HLT at target
    bus.ram[(PM_CODE_BASE + gate_target_ip as u32) as usize] = 0xF4;

    // JMP FAR 0x0040:0x0000
    place_at(&mut bus, PM_CODE_BASE, &[0xEA, 0x00, 0x00, 0x40, 0x00]);

    let sp_before = cpu.sp();

    cpu.step(&mut bus); // JMP FAR
    cpu.step(&mut bus); // HLT

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), gate_target_ip + 1);
    assert_eq!(cpu.sp(), sp_before, "JMP should not push anything on stack");
}

#[test]
fn i286_jmp_far_call_gate_inner_privilege_faults() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();
    let mut state = setup_protected_mode_extended(&mut bus);
    make_ring3_state(&mut state);
    cpu.load_state(&state);

    // Call gate: target is ring 0 (non-conforming, DPL 0 < CPL 3), gate DPL=3.
    write_gdt_gate(&mut bus, PM_GDT_BASE, 8, 0x0100, PM_CS_SEL, 0, 4, 3);

    // JMP FAR 0x0040:0x0000 - JMP cannot do inter-privilege via call gate.
    place_at(
        &mut bus,
        PM_RING3_CODE_BASE,
        &[0xEA, 0x00, 0x00, 0x40, 0x00],
    );

    cpu.step(&mut bus); // JMP FAR -> #GP
    cpu.step(&mut bus); // HLT in handler

    assert!(cpu.halted());
    assert_eq!(
        cpu.ip(),
        PM_GP_HANDLER_IP + 1,
        "JMP through inner-privilege call gate should fault with #GP"
    );
}

#[test]
fn i286_call_gate_dpl_insufficient_faults() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();
    let mut state = setup_protected_mode_extended(&mut bus);
    make_ring3_state(&mut state);
    cpu.load_state(&state);

    // Call gate with DPL=0: ring 3 caller cannot use it.
    write_gdt_gate(&mut bus, PM_GDT_BASE, 8, 0x0100, PM_CS_SEL, 0, 4, 0);

    // CALL FAR 0x0043:0x0000 (RPL=3 to match ring 3, but gate DPL=0)
    place_at(
        &mut bus,
        PM_RING3_CODE_BASE,
        &[0x9A, 0x00, 0x00, 0x43, 0x00],
    );

    cpu.step(&mut bus); // CALL FAR -> #GP
    cpu.step(&mut bus); // HLT

    assert!(cpu.halted());
    assert_eq!(
        cpu.ip(),
        PM_GP_HANDLER_IP + 1,
        "Call gate DPL < max(CPL, RPL) should raise #GP"
    );
}

#[test]
fn i286_jmp_far_to_tss_switches_task() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_extended(&mut bus);
    cpu.load_state(&state);

    // Set up initial register values to verify they get saved.
    cpu.state.set_ax(0x1111);
    cpu.state.set_bx(0x2222);

    // TSS2 at GDT entry 9 (selector 0x48), idle.
    let target_ip: u16 = 0x0300;
    write_tss(
        &mut bus,
        PM_TSS2_BASE,
        0,         // backlink
        0xFFF0,    // SP0
        PM_SS_SEL, // SS0
        target_ip, // IP
        0x0002,    // FLAGS (IF=0)
        0xAAAA,    // AX
        0xBBBB,    // CX
        0xCCCC,    // DX
        0xDDDD,    // BX
        0xEE00,    // SP
        0xFF00,    // BP
        0x1100,    // SI
        0x2200,    // DI
        PM_DS_SEL, // ES
        PM_CS_SEL, // CS
        PM_SS_SEL, // SS
        PM_DS_SEL, // DS
        0,         // LDT
    );

    // HLT at the target IP in the new task.
    bus.ram[(PM_CODE_BASE + target_ip as u32) as usize] = 0xF4;

    // JMP FAR 0x0048:0x0000 (TSS selector).
    place_at(&mut bus, PM_CODE_BASE, &[0xEA, 0x00, 0x00, 0x48, 0x00]);

    cpu.step(&mut bus); // JMP FAR -> task switch
    cpu.step(&mut bus); // HLT in new task

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), target_ip + 1);
    assert_eq!(cpu.ax(), 0xAAAA, "AX should be loaded from new TSS");
    assert_eq!(cpu.bx(), 0xDDDD, "BX should be loaded from new TSS");
    assert_eq!(cpu.cx(), 0xBBBB, "CX should be loaded from new TSS");
    assert_eq!(cpu.state.tr, PM_TSS2_SEL, "TR should point to new TSS");

    // Verify old TSS got the saved registers.
    let saved_ax = read_word_at(&bus, PM_TSS_BASE + 18);
    let saved_bx = read_word_at(&bus, PM_TSS_BASE + 24);
    assert_eq!(saved_ax, 0x1111, "Old AX should be saved in old TSS");
    assert_eq!(saved_bx, 0x2222, "Old BX should be saved in old TSS");

    // Verify busy bits: old TSS should be idle, new TSS should be busy.
    let old_tss_rights = bus.ram[(PM_GDT_BASE + 7 * 8 + 5) as usize];
    let new_tss_rights = bus.ram[(PM_GDT_BASE + 9 * 8 + 5) as usize];
    assert_eq!(
        old_tss_rights & 0x02,
        0,
        "Old TSS should be marked idle after JMP"
    );
    assert_ne!(
        new_tss_rights & 0x02,
        0,
        "New TSS should be marked busy after JMP"
    );
}

#[test]
fn i286_call_far_to_tss_sets_nt_and_backlink() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_extended(&mut bus);
    cpu.load_state(&state);

    let target_ip: u16 = 0x0300;
    write_tss(
        &mut bus,
        PM_TSS2_BASE,
        0,
        0xFFF0,
        PM_SS_SEL,
        target_ip,
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
    );

    bus.ram[(PM_CODE_BASE + target_ip as u32) as usize] = 0xF4;

    // CALL FAR 0x0048:0x0000
    place_at(&mut bus, PM_CODE_BASE, &[0x9A, 0x00, 0x00, 0x48, 0x00]);

    cpu.step(&mut bus); // CALL FAR -> task switch
    cpu.step(&mut bus); // HLT

    assert!(cpu.halted());
    assert!(
        cpu.state.flags.nt,
        "NT flag should be set after CALL task switch"
    );

    // Backlink in new TSS should be old TR.
    let backlink = read_word_at(&bus, PM_TSS2_BASE);
    assert_eq!(
        backlink, PM_TSS_SEL,
        "Backlink in new TSS should point to old TSS selector"
    );

    // Old TSS should still be busy (CALL doesn't mark old as idle).
    let old_tss_rights = bus.ram[(PM_GDT_BASE + 7 * 8 + 5) as usize];
    assert_ne!(
        old_tss_rights & 0x02,
        0,
        "Old TSS should remain busy after CALL"
    );
}

#[test]
fn i286_iret_with_nt_returns_to_previous_task() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_extended(&mut bus);
    cpu.load_state(&state);

    // Step 1: CALL FAR to TSS2 (sets NT, writes backlink).
    let task2_ip: u16 = 0x0300;
    write_tss(
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
        0xEE00,
        0,
        0,
        0,
        PM_DS_SEL,
        PM_CS_SEL,
        PM_SS_SEL,
        PM_DS_SEL,
        0,
    );

    // Task 2 code: IRET at task2_ip.
    bus.ram[(PM_CODE_BASE + task2_ip as u32) as usize] = 0xCF; // IRET

    // HLT right after the CALL FAR instruction (IP after = 5).
    bus.ram[(PM_CODE_BASE + 5) as usize] = 0xF4;

    // CALL FAR 0x0048:0x0000
    place_at(&mut bus, PM_CODE_BASE, &[0x9A, 0x00, 0x00, 0x48, 0x00]);

    // Mark TSS1 as busy (it already is via tr_rights but the GDT entry matters for IRET).
    let tss1_rights_addr = (PM_GDT_BASE + 7 * 8 + 5) as usize;
    bus.ram[tss1_rights_addr] |= 0x02;

    cpu.step(&mut bus); // CALL FAR -> switch to task 2
    cpu.step(&mut bus); // IRET in task 2 (NT set) -> switch back to task 1
    cpu.step(&mut bus); // HLT in task 1

    assert!(cpu.halted());
    assert_eq!(
        cpu.state.tr, PM_TSS_SEL,
        "Should have switched back to original task"
    );
    assert_eq!(cpu.ip(), 6, "Should be at IP after CALL FAR (5) + HLT (1)");
}

#[test]
fn i286_task_switch_saves_and_restores_registers() {
    let mut cpu = I286::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode_extended(&mut bus);
    cpu.load_state(&state);

    // Set distinctive register values.
    cpu.state.set_ax(0x1234);
    cpu.state.set_cx(0x5678);
    cpu.state.set_dx(0x9ABC);
    cpu.state.set_bx(0xDEF0);
    cpu.state.set_bp(0x1357);
    cpu.state.set_si(0x2468);
    cpu.state.set_di(0x3579);

    // TSS2: IRET at target_ip.
    let task2_ip: u16 = 0x0300;
    write_tss(
        &mut bus,
        PM_TSS2_BASE,
        0,
        0xFFF0,
        PM_SS_SEL,
        task2_ip,
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
    );

    bus.ram[(PM_CODE_BASE + task2_ip as u32) as usize] = 0xCF; // IRET
    bus.ram[(PM_CODE_BASE + 5) as usize] = 0xF4; // HLT after return

    // Mark TSS1 as busy in GDT.
    bus.ram[(PM_GDT_BASE + 7 * 8 + 5) as usize] |= 0x02;

    // CALL FAR 0x0048:0x0000
    place_at(&mut bus, PM_CODE_BASE, &[0x9A, 0x00, 0x00, 0x48, 0x00]);

    cpu.step(&mut bus); // CALL FAR -> task 2
    cpu.step(&mut bus); // IRET -> back to task 1
    cpu.step(&mut bus); // HLT

    assert!(cpu.halted());
    assert_eq!(cpu.ax(), 0x1234, "AX should be restored");
    assert_eq!(cpu.cx(), 0x5678, "CX should be restored");
    assert_eq!(cpu.dx(), 0x9ABC, "DX should be restored");
    assert_eq!(cpu.bx(), 0xDEF0, "BX should be restored");
    assert_eq!(cpu.state.bp(), 0x1357, "BP should be restored");
    assert_eq!(cpu.state.si(), 0x2468, "SI should be restored");
    assert_eq!(cpu.state.di(), 0x3579, "DI should be restored");
}
