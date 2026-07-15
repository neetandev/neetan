use common::Cpu as _;
use cpu::{CPU_MODEL_486_DX, I386};
use softfloat::Fp80;

const RAM_SIZE: usize = 2 << 20;
const ADDRESS_MASK: u32 = 0x001F_FFFF;

struct TestBus {
    ram: Vec<u8>,
    irq_pending: bool,
    irq_vector: u8,
    io_log: Vec<(u16, u8)>,
    /// Captures the un-masked physical address of every write reaching the bus.
    /// Used by tests that assert the CPU passes 32-bit physical addresses
    /// through to the bus without internal clamping.
    write_address_log: Vec<u32>,
}

impl TestBus {
    fn new() -> Self {
        Self {
            ram: vec![0u8; RAM_SIZE],
            irq_pending: false,
            irq_vector: 0,
            io_log: Vec::new(),
            write_address_log: Vec::new(),
        }
    }
}

impl common::Bus for TestBus {
    fn read_byte(&mut self, address: u32) -> u8 {
        self.ram[(address & ADDRESS_MASK) as usize]
    }

    fn write_byte(&mut self, address: u32, value: u8) {
        self.write_address_log.push(address);
        self.ram[(address & ADDRESS_MASK) as usize] = value;
    }

    fn io_read_byte(&mut self, _port: u16) -> u8 {
        0xFF
    }

    fn io_write_byte(&mut self, port: u16, value: u8) {
        self.io_log.push((port, value));
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

// --- Memory layout ---
//
// Physical memory map (2MB):
//   0x00000 - 0x00FFF : page for stack (SS base = 0)
//   0x10000 - 0x10FFF : page for data (DS base)
//   0x50000 - 0x50FFF : page for code (CS base)
//   0x60000 - 0x60FFF : page for ring-3 code
//   0x70000 - 0x70FFF : TSS
//   0x80000 - 0x80FFF : GDT
//   0x90000 - 0x97FFF : IDT (256 * 8 = 2048 bytes)
//   0xA0000 - 0xA0FFF : page directory
//   0xA1000 - 0xA1FFF : page table 0 (maps linear 0x00000000..0x003FFFFF)
//   0xB0000 - 0xB0FFF : remapped physical page (for testing remap)
//   0xC0000 - 0xC0FFF : ring-3 data page
//   0xD0000 - 0xD0FFF : read-only page (no R/W bit)
//   0x20000 - 0x20FFF : ring-3 stack

const PG_GDT_BASE: u32 = 0x80000;
const PG_IDT_BASE: u32 = 0x90000;
const PG_CODE_BASE: u32 = 0x50000;
const PG_DATA_BASE: u32 = 0x10000;
const PG_STACK_BASE: u32 = 0x00000;
const PG_RING3_CODE_BASE: u32 = 0x60000;
const PG_RING3_STACK_BASE: u32 = 0x20000;
const PG_TSS_BASE: u32 = 0x70000;
const PG_PAGE_DIR: u32 = 0xA0000;
const PG_PAGE_TABLE_0: u32 = 0xA1000;
const PG_REMAP_PAGE: u32 = 0xB0000;
const _PG_RING3_DATA_PAGE: u32 = 0xC0000;
const _PG_READONLY_PAGE: u32 = 0xD0000;

const PG_CS_SEL: u16 = 0x0008;
const PG_DS_SEL: u16 = 0x0010;
const PG_SS_SEL: u16 = 0x0018;
const PG_RING3_CS_SEL: u16 = 0x0023;
const PG_RING3_DS_SEL: u16 = 0x002B;
const PG_RING3_SS_SEL: u16 = 0x0033;
const PG_TSS_SEL: u16 = 0x0038;

const PG_GP_HANDLER_IP: u16 = 0x8000;
const PG_PF_HANDLER_IP: u16 = 0x9000;

// PTE/PDE flags
const PTE_P: u32 = 0x01;
const PTE_RW: u32 = 0x02;
const PTE_US: u32 = 0x04;
const PTE_A: u32 = 0x20;
const PTE_D: u32 = 0x40;

fn place_at(bus: &mut TestBus, addr: u32, code: &[u8]) {
    for (i, &byte) in code.iter().enumerate() {
        bus.ram[addr as usize + i] = byte;
    }
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

fn write_gdt_entry16(
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
    bus.ram[addr + 7] = (base >> 24) as u8;
}

fn write_gdt_entry32(
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

/// Sets up an identity-mapped page table covering linear 0x000000..0x1FFFFF.
/// All pages are supervisor, read/write, present.
fn setup_identity_page_tables(bus: &mut TestBus) {
    // Page directory: entry 0 points to page table 0.
    write_dword_at(bus, PG_PAGE_DIR, PG_PAGE_TABLE_0 | PTE_P | PTE_RW);

    // Page table 0: identity-map 1024 pages (4MB), covering our 2MB RAM.
    for i in 0..512u32 {
        let phys = i * 0x1000;
        write_dword_at(bus, PG_PAGE_TABLE_0 + i * 4, phys | PTE_P | PTE_RW);
    }
}

/// Sets up protected mode with paging enabled (CR0 = PE+PG).
/// Identity-maps all memory. Returns state at CPL 0.
fn setup_paged_protected_mode(bus: &mut TestBus) -> cpu::I386State {
    // GDT: null, CS (ring 0), DS (ring 0), SS (ring 0),
    //       CS (ring 3), DS (ring 3), SS (ring 3), TSS
    write_gdt_entry16(bus, PG_GDT_BASE, 0, 0, 0, 0);
    write_gdt_entry16(bus, PG_GDT_BASE, 1, PG_CODE_BASE, 0xFFFF, 0x9B);
    write_gdt_entry16(bus, PG_GDT_BASE, 2, PG_DATA_BASE, 0xFFFF, 0x93);
    write_gdt_entry16(bus, PG_GDT_BASE, 3, PG_STACK_BASE, 0xFFFF, 0x93);
    write_gdt_entry16(bus, PG_GDT_BASE, 4, PG_RING3_CODE_BASE, 0xFFFF, 0xFB);
    write_gdt_entry16(bus, PG_GDT_BASE, 5, PG_DATA_BASE, 0xFFFF, 0xF3);
    write_gdt_entry16(bus, PG_GDT_BASE, 6, PG_RING3_STACK_BASE, 0xFFFF, 0xF3);
    write_gdt_entry16(bus, PG_GDT_BASE, 7, PG_TSS_BASE, 103, 0x89);

    // IDT: #GP (13) and #PF (14) handlers.
    write_idt_gate(
        bus,
        PG_IDT_BASE,
        13,
        PG_GP_HANDLER_IP as u32,
        PG_CS_SEL,
        14,
        0,
    );
    write_idt_gate(
        bus,
        PG_IDT_BASE,
        14,
        PG_PF_HANDLER_IP as u32,
        PG_CS_SEL,
        14,
        0,
    );

    // HLT at each handler entry.
    bus.ram[(PG_CODE_BASE + PG_GP_HANDLER_IP as u32) as usize] = 0xF4;
    bus.ram[(PG_CODE_BASE + PG_PF_HANDLER_IP as u32) as usize] = 0xF4;

    // TSS: ESP0 at offset 4, SS0 at offset 8.
    write_dword_at(bus, PG_TSS_BASE + 4, 0xFFF0);
    write_word_at(bus, PG_TSS_BASE + 8, PG_SS_SEL);

    // Page tables: identity-mapped.
    setup_identity_page_tables(bus);

    let mut state = cpu::I386State {
        cr0: 0x8000_0001, // PE + PG
        cr3: PG_PAGE_DIR,
        ip: 0x0000,
        ..Default::default()
    };

    state.set_esp(0xFFF0);
    state.set_cs(PG_CS_SEL);
    state.set_ds(PG_DS_SEL);
    state.set_ss(PG_SS_SEL);
    state.set_es(PG_DS_SEL);

    state.seg_bases[cpu::SegReg32::ES as usize] = PG_DATA_BASE;
    state.seg_bases[cpu::SegReg32::CS as usize] = PG_CODE_BASE;
    state.seg_bases[cpu::SegReg32::SS as usize] = PG_STACK_BASE;
    state.seg_bases[cpu::SegReg32::DS as usize] = PG_DATA_BASE;

    state.seg_limits = [0xFFFF; 6];
    state.seg_rights[cpu::SegReg32::ES as usize] = 0x93;
    state.seg_rights[cpu::SegReg32::CS as usize] = 0x9B;
    state.seg_rights[cpu::SegReg32::SS as usize] = 0x93;
    state.seg_rights[cpu::SegReg32::DS as usize] = 0x93;
    state.seg_valid = [true, true, true, true, false, false];

    state.gdt_base = PG_GDT_BASE;
    state.gdt_limit = 8 * 8 - 1;
    state.idt_base = PG_IDT_BASE;
    state.idt_limit = 256 * 8 - 1;

    state.tr = PG_TSS_SEL;
    state.tr_base = PG_TSS_BASE;
    state.tr_limit = 103;
    state.tr_rights = 0x8B;

    state
}

fn make_ring3(state: &mut cpu::I386State) {
    state.set_cs(PG_RING3_CS_SEL);
    state.seg_bases[cpu::SegReg32::CS as usize] = PG_RING3_CODE_BASE;
    state.seg_rights[cpu::SegReg32::CS as usize] = 0xFB;

    state.set_ss(PG_RING3_SS_SEL);
    state.seg_bases[cpu::SegReg32::SS as usize] = PG_RING3_STACK_BASE;
    state.seg_rights[cpu::SegReg32::SS as usize] = 0xF3;

    state.set_ds(PG_RING3_DS_SEL);
    state.seg_bases[cpu::SegReg32::DS as usize] = PG_DATA_BASE;
    state.seg_rights[cpu::SegReg32::DS as usize] = 0xF3;

    state.set_es(PG_RING3_DS_SEL);
    state.seg_bases[cpu::SegReg32::ES as usize] = PG_DATA_BASE;
    state.seg_rights[cpu::SegReg32::ES as usize] = 0xF3;
}

/// Transforms a ring-0 protected-mode + paging state into V86 mode.
/// CS / SS / DS / ES are reshaped to real-mode-style caches (base =
/// selector << 4, limit 0xFFFF, data-style rights). The VM flag (EFLAGS
/// bit 17) is set. The caller is responsible for placing the V86
/// instruction at `(cs_selector << 4) + ip` linear, and ensuring those
/// pages are present and identity-mapped.
fn make_v86(state: &mut cpu::I386State, cs_selector: u16, ss_selector: u16) {
    state.set_cs(cs_selector);
    state.seg_bases[cpu::SegReg32::CS as usize] = (cs_selector as u32) << 4;
    state.seg_limits[cpu::SegReg32::CS as usize] = 0xFFFF;
    state.seg_rights[cpu::SegReg32::CS as usize] = 0xF3;

    state.set_ss(ss_selector);
    state.seg_bases[cpu::SegReg32::SS as usize] = (ss_selector as u32) << 4;
    state.seg_limits[cpu::SegReg32::SS as usize] = 0xFFFF;
    state.seg_rights[cpu::SegReg32::SS as usize] = 0xF3;
    state.seg_granularity[cpu::SegReg32::SS as usize] = 0;

    state.set_ds(0);
    state.seg_bases[cpu::SegReg32::DS as usize] = 0;
    state.seg_limits[cpu::SegReg32::DS as usize] = 0xFFFF;
    state.seg_rights[cpu::SegReg32::DS as usize] = 0xF3;

    state.set_es(0);
    state.seg_bases[cpu::SegReg32::ES as usize] = 0;
    state.seg_limits[cpu::SegReg32::ES as usize] = 0xFFFF;
    state.seg_rights[cpu::SegReg32::ES as usize] = 0xF3;

    state.eflags_upper |= 0x0002_0000; // VM=1
    state.flags.iopl = 3;
}

/// Identity-mapped paging: MOV AL,[DS:offset] reads through page tables correctly.
#[test]
fn paging_identity_map_read_byte() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_paged_protected_mode(&mut bus);
    cpu.load_state(&state);

    // Write test value at DS:0x0042 = physical PG_DATA_BASE + 0x42.
    bus.ram[(PG_DATA_BASE + 0x42) as usize] = 0xBE;

    // MOV AL, [0x0042] ; A0 42 00
    place_at(&mut bus, PG_CODE_BASE, &[0xA0, 0x42, 0x00]);
    cpu.step(&mut bus);

    assert_eq!(
        cpu.al(),
        0xBE,
        "identity-mapped read should return correct value"
    );
}

/// Identity-mapped paging: MOV [DS:offset], AL writes through page tables correctly.
#[test]
fn paging_identity_map_write_byte() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_paged_protected_mode(&mut bus);
    cpu.load_state(&state);

    // Set AL = 0x5A, then MOV [0x0080], AL ; A2 80 00
    place_at(
        &mut bus,
        PG_CODE_BASE,
        &[
            0xB0, 0x5A, // MOV AL, 0x5A
            0xA2, 0x80, 0x00, // MOV [0x0080], AL
        ],
    );
    cpu.step(&mut bus); // MOV AL, imm8
    cpu.step(&mut bus); // MOV [moffs], AL

    assert_eq!(
        bus.ram[(PG_DATA_BASE + 0x80) as usize],
        0x5A,
        "identity-mapped write should store correct value"
    );
}

/// Identity-mapped paging: word read across page-aligned addresses.
#[test]
fn paging_identity_map_read_word() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_paged_protected_mode(&mut bus);
    cpu.load_state(&state);

    // Write 0xCAFE at DS:0x0100.
    write_word_at(&mut bus, PG_DATA_BASE + 0x100, 0xCAFE);

    // MOV AX, [0x0100] ; A1 00 01
    place_at(&mut bus, PG_CODE_BASE, &[0xA1, 0x00, 0x01]);
    cpu.step(&mut bus);

    assert_eq!(cpu.ax(), 0xCAFE);
}

/// Identity-mapped paging: dword read.
#[test]
fn paging_identity_map_read_dword() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_paged_protected_mode(&mut bus);
    cpu.load_state(&state);

    write_dword_at(&mut bus, PG_DATA_BASE + 0x200, 0xDEAD_BEEF);

    // MOV EAX, [0x0200] ; 66 A1 00 02
    place_at(&mut bus, PG_CODE_BASE, &[0x66, 0xA1, 0x00, 0x02]);
    cpu.step(&mut bus);

    assert_eq!(cpu.eax(), 0xDEAD_BEEF);
}

/// Remapped page: linear address maps to a different physical page.
#[test]
fn paging_remap_read() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_paged_protected_mode(&mut bus);
    cpu.load_state(&state);

    // Remap linear page at DS:0x0000 (linear = PG_DATA_BASE = 0x10000,
    // page index = 0x10) to physical PG_REMAP_PAGE instead of identity.
    let pte_index = PG_DATA_BASE >> 12; // = 0x10
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + pte_index * 4,
        PG_REMAP_PAGE | PTE_P | PTE_RW,
    );

    // Write test value at the REMAPPED physical page.
    bus.ram[(PG_REMAP_PAGE + 0x0010) as usize] = 0x77;

    // MOV AL, [0x0010] ; A0 10 00
    place_at(&mut bus, PG_CODE_BASE, &[0xA0, 0x10, 0x00]);
    cpu.step(&mut bus);

    assert_eq!(
        cpu.al(),
        0x77,
        "read through remapped PTE should hit the remapped physical page"
    );
}

/// Remapped page: write goes to remapped physical page.
#[test]
fn paging_remap_write() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_paged_protected_mode(&mut bus);
    cpu.load_state(&state);

    // Remap data page to PG_REMAP_PAGE.
    let pte_index = PG_DATA_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + pte_index * 4,
        PG_REMAP_PAGE | PTE_P | PTE_RW,
    );

    // MOV AL, 0xAA ; MOV [0x0020], AL
    place_at(&mut bus, PG_CODE_BASE, &[0xB0, 0xAA, 0xA2, 0x20, 0x00]);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_eq!(
        bus.ram[(PG_REMAP_PAGE + 0x20) as usize],
        0xAA,
        "write through remapped PTE should store in the remapped physical page"
    );
    assert_eq!(
        bus.ram[(PG_DATA_BASE + 0x20) as usize],
        0x00,
        "original identity-mapped physical page should be untouched"
    );
}

/// Reading a page sets Accessed bit on PDE and PTE.
#[test]
fn paging_accessed_bit_on_read() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_paged_protected_mode(&mut bus);
    cpu.load_state(&state);

    // Clear A bits on the PDE and the PTE for the data page.
    let pte_index = PG_DATA_BASE >> 12;
    let pde_before = read_dword_at(&bus, PG_PAGE_DIR);
    let pte_before = read_dword_at(&bus, PG_PAGE_TABLE_0 + pte_index * 4);
    write_dword_at(&mut bus, PG_PAGE_DIR, pde_before & !PTE_A);
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + pte_index * 4,
        pte_before & !PTE_A,
    );

    // MOV AL, [0x0000] ; A0 00 00
    place_at(&mut bus, PG_CODE_BASE, &[0xA0, 0x00, 0x00]);
    cpu.step(&mut bus);

    let pde_after = read_dword_at(&bus, PG_PAGE_DIR);
    let pte_after = read_dword_at(&bus, PG_PAGE_TABLE_0 + pte_index * 4);

    assert_ne!(
        pde_after & PTE_A,
        0,
        "PDE Accessed bit must be set after read"
    );
    assert_ne!(
        pte_after & PTE_A,
        0,
        "PTE Accessed bit must be set after read"
    );
    assert_eq!(
        pte_after & PTE_D,
        0,
        "PTE Dirty bit must NOT be set after read"
    );
}

/// Writing a page sets both Accessed and Dirty bits on PTE.
#[test]
fn paging_dirty_bit_on_write() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_paged_protected_mode(&mut bus);
    cpu.load_state(&state);

    // Clear A+D bits.
    let pte_index = PG_DATA_BASE >> 12;
    let pde_before = read_dword_at(&bus, PG_PAGE_DIR);
    let pte_before = read_dword_at(&bus, PG_PAGE_TABLE_0 + pte_index * 4);
    write_dword_at(&mut bus, PG_PAGE_DIR, pde_before & !(PTE_A | PTE_D));
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + pte_index * 4,
        pte_before & !(PTE_A | PTE_D),
    );

    // MOV BYTE [0x0000], 0x55 ; C6 06 00 00 55
    place_at(&mut bus, PG_CODE_BASE, &[0xC6, 0x06, 0x00, 0x00, 0x55]);
    cpu.step(&mut bus);

    let pde_after = read_dword_at(&bus, PG_PAGE_DIR);
    let pte_after = read_dword_at(&bus, PG_PAGE_TABLE_0 + pte_index * 4);

    assert_ne!(
        pde_after & PTE_A,
        0,
        "PDE Accessed bit must be set after write"
    );
    assert_ne!(
        pte_after & PTE_A,
        0,
        "PTE Accessed bit must be set after write"
    );
    assert_ne!(
        pte_after & PTE_D,
        0,
        "PTE Dirty bit must be set after write"
    );
}

/// BT reads the memory operand but must not dirty the page.
#[test]
fn paging_group_ba_bt_imm_sets_accessed_not_dirty() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_paged_protected_mode(&mut bus);
    cpu.load_state(&state);

    let pte_index = PG_DATA_BASE >> 12;
    let pde_before = read_dword_at(&bus, PG_PAGE_DIR);
    let pte_before = read_dword_at(&bus, PG_PAGE_TABLE_0 + pte_index * 4);
    write_dword_at(&mut bus, PG_PAGE_DIR, pde_before & !(PTE_A | PTE_D));
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + pte_index * 4,
        pte_before & !(PTE_A | PTE_D),
    );

    write_dword_at(&mut bus, PG_DATA_BASE + 0x80, 1 << 5);

    // Operand-size override; BT dword [0x0080], 5.
    place_at(
        &mut bus,
        PG_CODE_BASE,
        &[0x66, 0x0F, 0xBA, 0x26, 0x80, 0x00, 0x05],
    );
    cpu.step(&mut bus);

    let pde_after = read_dword_at(&bus, PG_PAGE_DIR);
    let pte_after = read_dword_at(&bus, PG_PAGE_TABLE_0 + pte_index * 4);

    assert_ne!(cpu.eflags() & 1, 0, "BT must load the selected bit into CF");
    assert_ne!(
        pde_after & PTE_A,
        0,
        "PDE Accessed bit must be set after BT memory read"
    );
    assert_ne!(
        pte_after & PTE_A,
        0,
        "PTE Accessed bit must be set after BT memory read"
    );
    assert_eq!(
        pte_after & PTE_D,
        0,
        "PTE Dirty bit must NOT be set after BT memory read"
    );
    assert_eq!(
        read_dword_at(&bus, PG_DATA_BASE + 0x80),
        1 << 5,
        "BT must not modify the memory operand"
    );
}

/// #PF on read from a not-present PDE. Error code = 0 (not present, read, supervisor).
/// Uses PDE 1 (linear 0x400000+) for data so clearing it doesn't affect code/stack.
#[test]
fn paging_fault_pde_not_present_read() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_paged_protected_mode(&mut bus);
    // Move DS/ES to linear 0x400000 (covered by PDE 1, not PDE 0).
    state.seg_bases[cpu::SegReg32::DS as usize] = 0x00400000;
    state.seg_bases[cpu::SegReg32::ES as usize] = 0x00400000;
    cpu.load_state(&state);

    // Set up PDE 1 with a page table, then clear it.
    let pt1_base = 0xA2000u32;
    write_dword_at(&mut bus, PG_PAGE_DIR + 4, pt1_base | PTE_P | PTE_RW);
    write_dword_at(&mut bus, pt1_base, PG_DATA_BASE | PTE_P | PTE_RW);
    // Now clear PDE 1 - code/stack remain under PDE 0.
    write_dword_at(&mut bus, PG_PAGE_DIR + 4, 0);

    // MOV AL, [0x0000] - DS:0 = linear 0x400000, PDE 1 not present -> #PF.
    place_at(&mut bus, PG_CODE_BASE, &[0xA0, 0x00, 0x00]);
    cpu.step(&mut bus); // faults
    cpu.step(&mut bus); // executes HLT in #PF handler

    assert!(cpu.halted(), "CPU should halt in #PF handler");
    assert_eq!(
        cpu.ip(),
        PG_PF_HANDLER_IP as u32 + 1,
        "should be in #PF handler"
    );

    // Error code pushed on stack: 0 (not present, read, supervisor).
    let sp = cpu.esp();
    let error_code = read_word_at(&bus, PG_STACK_BASE + sp);
    assert_eq!(
        error_code, 0x0000,
        "error code: not present, read, supervisor"
    );

    // CR2 = faulting linear address = DS base + 0 = 0x400000.
    assert_eq!(cpu.cr2, 0x00400000, "CR2 must hold faulting linear address");
}

/// #PF on read from a not-present PTE.
#[test]
fn paging_fault_pte_not_present_read() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_paged_protected_mode(&mut bus);
    cpu.load_state(&state);

    // Clear Present bit on the PTE for the data page.
    let pte_index = PG_DATA_BASE >> 12;
    write_dword_at(&mut bus, PG_PAGE_TABLE_0 + pte_index * 4, 0);

    // MOV AL, [0x0042]
    place_at(&mut bus, PG_CODE_BASE, &[0xA0, 0x42, 0x00]);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PG_PF_HANDLER_IP as u32 + 1);

    let sp = cpu.esp();
    let error_code = read_word_at(&bus, PG_STACK_BASE + sp);
    assert_eq!(
        error_code, 0x0000,
        "error code: not present, read, supervisor"
    );
    assert_eq!(cpu.cr2, PG_DATA_BASE + 0x42);
}

/// #PF on write to a not-present PTE. Error code bit 1 (W/R) set.
#[test]
fn paging_fault_pte_not_present_write() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_paged_protected_mode(&mut bus);
    cpu.load_state(&state);

    let pte_index = PG_DATA_BASE >> 12;
    write_dword_at(&mut bus, PG_PAGE_TABLE_0 + pte_index * 4, 0);

    // MOV BYTE [0x0010], 0xFF ; C6 06 10 00 FF
    place_at(&mut bus, PG_CODE_BASE, &[0xC6, 0x06, 0x10, 0x00, 0xFF]);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    let sp = cpu.esp();
    let error_code = read_word_at(&bus, PG_STACK_BASE + sp);
    assert_eq!(
        error_code, 0x0002,
        "error code: not present, write, supervisor"
    );
    assert_eq!(cpu.cr2, PG_DATA_BASE + 0x10);
}

/// Ring-3 read from a supervisor-only page (#PF with P=1, U/S=1).
#[test]
fn paging_fault_user_reads_supervisor_page() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_paged_protected_mode(&mut bus);
    make_ring3(&mut state);
    cpu.load_state(&state);

    // PDE: present + R/W + U/S (user can traverse directory).
    let pde = read_dword_at(&bus, PG_PAGE_DIR);
    write_dword_at(&mut bus, PG_PAGE_DIR, pde | PTE_US);

    // Make ring-3 code page user-accessible so code fetch succeeds.
    let code_pte_index = PG_RING3_CODE_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + code_pte_index * 4,
        PG_RING3_CODE_BASE | PTE_P | PTE_RW | PTE_US,
    );

    // Make ring-3 stack page user-accessible.
    let stack_pte_index = PG_RING3_STACK_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + stack_pte_index * 4,
        PG_RING3_STACK_BASE | PTE_P | PTE_RW | PTE_US,
    );

    // Make the data page supervisor-only: present + R/W but NO U/S.
    let pte_index = PG_DATA_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + pte_index * 4,
        PG_DATA_BASE | PTE_P | PTE_RW,
    );

    // MOV AL, [0x0000] from ring 3 code.
    place_at(&mut bus, PG_RING3_CODE_BASE, &[0xA0, 0x00, 0x00]);
    cpu.step(&mut bus); // faults on data read (data PTE lacks U/S)
    cpu.step(&mut bus); // HLT in handler

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PG_PF_HANDLER_IP as u32 + 1);

    // Error code: present=1, read=0, user=1 -> 0x05.
    let sp = cpu.esp();
    let error_code = read_word_at(&bus, PG_STACK_BASE + sp);
    assert_eq!(error_code, 0x0005, "error code: present, read, user");
}

/// Ring-3 write to a read-only user page (#PF with P=1, W/R=1, U/S=1).
#[test]
fn paging_fault_user_writes_readonly_page() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_paged_protected_mode(&mut bus);
    make_ring3(&mut state);
    cpu.load_state(&state);

    // PDE: present + R/W + U/S (user can traverse directory).
    let pde = read_dword_at(&bus, PG_PAGE_DIR);
    write_dword_at(&mut bus, PG_PAGE_DIR, pde | PTE_US);

    // Make ring-3 code page user-accessible.
    let code_pte_index = PG_RING3_CODE_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + code_pte_index * 4,
        PG_RING3_CODE_BASE | PTE_P | PTE_RW | PTE_US,
    );

    // Make ring-3 stack page user-accessible.
    let stack_pte_index = PG_RING3_STACK_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + stack_pte_index * 4,
        PG_RING3_STACK_BASE | PTE_P | PTE_RW | PTE_US,
    );

    // Make the data page user-accessible but read-only: present + U/S, no R/W.
    let pte_index = PG_DATA_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + pte_index * 4,
        PG_DATA_BASE | PTE_P | PTE_US,
    );

    // MOV BYTE [0x0000], 0x11 from ring 3.
    place_at(
        &mut bus,
        PG_RING3_CODE_BASE,
        &[0xC6, 0x06, 0x00, 0x00, 0x11],
    );
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    let sp = cpu.esp();
    let error_code = read_word_at(&bus, PG_STACK_BASE + sp);
    assert_eq!(error_code, 0x0007, "error code: present, write, user");
}

#[test]
fn paging_fault_user_read_modify_write_reports_write() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_paged_protected_mode(&mut bus);
    make_ring3(&mut state);
    state.set_ebx(0);
    cpu.load_state(&state);

    let pde = read_dword_at(&bus, PG_PAGE_DIR);
    write_dword_at(&mut bus, PG_PAGE_DIR, pde | PTE_US);

    let code_pte_index = PG_RING3_CODE_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + code_pte_index * 4,
        PG_RING3_CODE_BASE | PTE_P | PTE_RW | PTE_US,
    );

    let stack_pte_index = PG_RING3_STACK_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + stack_pte_index * 4,
        PG_RING3_STACK_BASE | PTE_P | PTE_RW | PTE_US,
    );

    let data_pte_index = PG_DATA_BASE >> 12;
    write_dword_at(&mut bus, PG_PAGE_TABLE_0 + data_pte_index * 4, 0);

    place_at(&mut bus, PG_RING3_CODE_BASE, &[0x26, 0x80, 0x0F, 0x00]);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PG_PF_HANDLER_IP as u32 + 1);
    assert_eq!(cpu.cr2, PG_DATA_BASE);

    let sp = cpu.esp();
    let error_code = read_word_at(&bus, PG_STACK_BASE + sp);
    assert_eq!(error_code, 0x0006, "error code: not present, write, user");

    let return_eip = read_dword_at(&bus, PG_STACK_BASE + sp + 4);
    assert_eq!(return_eip, 0, "fault should restart at the segment prefix");
}

/// Helper for RMW page-fault tests: ring-3 user, ES segment override, [BX]
/// addressing pointing at a not-present user data page; expect error 0x0006.
fn assert_rmw_user_fault_writes_with_cpu<const CPU_MODEL: u8>(
    cpu: &mut I386<{ CPU_MODEL }>,
    instruction: &[u8],
    expected_eip_after_fault: u32,
) {
    let mut bus = TestBus::new();

    let mut state = setup_paged_protected_mode(&mut bus);
    make_ring3(&mut state);
    state.set_ebx(0);
    state.set_edi(0);
    cpu.load_state(&state);

    let pde = read_dword_at(&bus, PG_PAGE_DIR);
    write_dword_at(&mut bus, PG_PAGE_DIR, pde | PTE_US);

    let code_pte_index = PG_RING3_CODE_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + code_pte_index * 4,
        PG_RING3_CODE_BASE | PTE_P | PTE_RW | PTE_US,
    );

    let stack_pte_index = PG_RING3_STACK_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + stack_pte_index * 4,
        PG_RING3_STACK_BASE | PTE_P | PTE_RW | PTE_US,
    );

    let data_pte_index = PG_DATA_BASE >> 12;
    write_dword_at(&mut bus, PG_PAGE_TABLE_0 + data_pte_index * 4, 0);

    place_at(&mut bus, PG_RING3_CODE_BASE, instruction);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PG_PF_HANDLER_IP as u32 + 1);
    assert_eq!(cpu.cr2, PG_DATA_BASE);

    let sp = cpu.esp();
    let error_code = read_word_at(&bus, PG_STACK_BASE + sp);
    assert_eq!(error_code, 0x0006, "error code: not present, write, user");

    let return_eip = read_dword_at(&bus, PG_STACK_BASE + sp + 4);
    assert_eq!(
        return_eip, expected_eip_after_fault,
        "fault should restart at the segment prefix"
    );
}

fn assert_rmw_user_fault_writes(instruction: &[u8], expected_eip_after_fault: u32) {
    let mut cpu: I386 = I386::new();
    assert_rmw_user_fault_writes_with_cpu(&mut cpu, instruction, expected_eip_after_fault);
}

fn assert_rmw_user_fault_writes_486(instruction: &[u8], expected_eip_after_fault: u32) {
    let mut cpu: I386<{ cpu::CPU_MODEL_486_DX }> = I386::new();
    assert_rmw_user_fault_writes_with_cpu(&mut cpu, instruction, expected_eip_after_fault);
}

#[test]
fn paging_fault_user_inc_r_m8_reports_write() {
    // ES: INC byte ptr [BX] -- 0x26 0xFE /0
    assert_rmw_user_fault_writes(&[0x26, 0xFE, 0x07], 0);
}

#[test]
fn paging_fault_user_inc_r_m16_reports_write() {
    // ES: INC word ptr [BX] -- 0x26 0xFF /0
    assert_rmw_user_fault_writes(&[0x26, 0xFF, 0x07], 0);
}

#[test]
fn paging_fault_user_inc_r_m32_reports_write() {
    // ES: INC dword ptr [BX] -- 0x26 0x66 0xFF /0
    assert_rmw_user_fault_writes(&[0x26, 0x66, 0xFF, 0x07], 0);
}

#[test]
fn paging_fault_user_dec_r_m8_reports_write() {
    // ES: DEC byte ptr [BX] -- 0x26 0xFE /1
    assert_rmw_user_fault_writes(&[0x26, 0xFE, 0x0F], 0);
}

#[test]
fn paging_fault_user_or_r_m16_imm_reports_write() {
    // ES: OR word ptr [BX], 0 -- 0x26 0x81 /1
    assert_rmw_user_fault_writes(&[0x26, 0x81, 0x0F, 0x00, 0x00], 0);
}

#[test]
fn paging_fault_user_or_r_m32_imm_reports_write() {
    // ES: OR dword ptr [BX], 0 -- 0x26 0x66 0x81 /1
    assert_rmw_user_fault_writes(&[0x26, 0x66, 0x81, 0x0F, 0x00, 0x00, 0x00, 0x00], 0);
}

#[test]
fn paging_fault_user_or_r_m16_imm8_reports_write() {
    // ES: OR word ptr [BX], 0 (sign-extended imm8) -- 0x26 0x83 /1
    assert_rmw_user_fault_writes(&[0x26, 0x83, 0x0F, 0x00], 0);
}

#[test]
fn paging_fault_user_add_mem_byte_reg_reports_write() {
    // ES: ADD byte ptr [BX], CL -- 0x26 0x00 /CL
    // ModRM 0x0F = mod=00 reg=001 (CL) rm=111 ([BX])
    assert_rmw_user_fault_writes(&[0x26, 0x00, 0x0F], 0);
}

#[test]
fn paging_fault_user_add_mem_word_reg_reports_write() {
    // ES: ADD word ptr [BX], CX -- 0x26 0x01 /CX
    assert_rmw_user_fault_writes(&[0x26, 0x01, 0x0F], 0);
}

#[test]
fn paging_fault_user_xchg_mem_byte_reg_reports_write() {
    // ES: XCHG byte ptr [BX], CL -- 0x26 0x86 /CL
    assert_rmw_user_fault_writes(&[0x26, 0x86, 0x0F], 0);
}

#[test]
fn paging_fault_user_xchg_mem_word_reg_reports_write() {
    // ES: XCHG word ptr [BX], CX -- 0x26 0x87 /CX
    assert_rmw_user_fault_writes(&[0x26, 0x87, 0x0F], 0);
}

#[test]
fn paging_fault_user_cmpxchg_byte_reports_write() {
    // ES: CMPXCHG byte ptr [BX], CL -- 0x26 0x0F 0xB0 /CL
    // CMPXCHG always treated as a write because it conditionally writes; the
    // architectural access is RMW so the W/R bit must be set on a fault.
    // CMPXCHG is a 486+ instruction.
    assert_rmw_user_fault_writes_486(&[0x26, 0x0F, 0xB0, 0x0F], 0);
}

#[test]
fn paging_fault_user_xadd_byte_reports_write() {
    // ES: XADD byte ptr [BX], CL -- 0x26 0x0F 0xC0 /CL
    // XADD is a 486+ instruction.
    assert_rmw_user_fault_writes_486(&[0x26, 0x0F, 0xC0, 0x0F], 0);
}

#[test]
fn paging_fault_user_bts_imm_reports_write() {
    // ES: BTS word ptr [BX], 0 -- 0x26 0x0F 0xBA /5 imm8
    // ModRM 0x2F = mod=00 reg=101 (/5) rm=111 ([BX])
    assert_rmw_user_fault_writes(&[0x26, 0x0F, 0xBA, 0x2F, 0x00], 0);
}

#[test]
fn paging_fault_user_btr_imm_reports_write() {
    // ES: BTR word ptr [BX], 0 -- 0x26 0x0F 0xBA /6 imm8
    assert_rmw_user_fault_writes(&[0x26, 0x0F, 0xBA, 0x37, 0x00], 0);
}

#[test]
fn paging_fault_user_btc_imm_reports_write() {
    // ES: BTC word ptr [BX], 0 -- 0x26 0x0F 0xBA /7 imm8
    assert_rmw_user_fault_writes(&[0x26, 0x0F, 0xBA, 0x3F, 0x00], 0);
}

/// On a 386, supervisor (CPL 0) can write to a page with R/W=0.
#[test]
fn paging_supervisor_writes_readonly_page() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_paged_protected_mode(&mut bus);
    cpu.load_state(&state);

    // Make the data page read-only (present, no R/W, no U/S).
    let pte_index = PG_DATA_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + pte_index * 4,
        PG_DATA_BASE | PTE_P,
    );

    // MOV BYTE [0x0050], 0xCC ; C6 06 50 00 CC
    place_at(&mut bus, PG_CODE_BASE, &[0xC6, 0x06, 0x50, 0x00, 0xCC]);
    cpu.step(&mut bus);

    assert_eq!(
        bus.ram[(PG_DATA_BASE + 0x50) as usize],
        0xCC,
        "386 supervisor must be able to write read-only pages (no WP bit)"
    );
    assert!(!cpu.halted(), "should NOT fault");
}

/// Writing CR3 flushes TLB: remapping a page after a CR3 write takes effect.
#[test]
fn paging_cr3_write_flushes_tlb() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_paged_protected_mode(&mut bus);
    cpu.load_state(&state);

    // Place ALL code upfront to avoid prefetch cache issues.
    place_at(
        &mut bus,
        PG_CODE_BASE,
        &[
            0xA0, 0x10, 0x00, // MOV AL, [0x10]
            0x0F, 0x20, 0xD8, // MOV EAX, CR3
            0x0F, 0x22, 0xD8, // MOV CR3, EAX
            0xA0, 0x10, 0x00, // MOV AL, [0x10]
        ],
    );

    // First read to prime the TLB.
    bus.ram[(PG_DATA_BASE + 0x10) as usize] = 0x11;
    cpu.step(&mut bus); // MOV AL, [0x10]
    assert_eq!(cpu.al(), 0x11);

    // Now remap the data page to PG_REMAP_PAGE.
    let pte_index = PG_DATA_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + pte_index * 4,
        PG_REMAP_PAGE | PTE_P | PTE_RW,
    );
    bus.ram[(PG_REMAP_PAGE + 0x10) as usize] = 0x22;

    cpu.step(&mut bus); // MOV EAX, CR3
    cpu.step(&mut bus); // MOV CR3, EAX (flushes TLB)
    cpu.step(&mut bus); // MOV AL, [0x10] - should use new mapping

    assert_eq!(
        cpu.al(),
        0x22,
        "after CR3 write (TLB flush), read should use remapped page"
    );
}

/// Code fetch goes through paging: remap the code page and verify execution.
#[test]
fn paging_code_fetch_through_page_tables() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_paged_protected_mode(&mut bus);
    cpu.load_state(&state);

    // Remap the code page to PG_REMAP_PAGE.
    let pte_index = PG_CODE_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + pte_index * 4,
        PG_REMAP_PAGE | PTE_P | PTE_RW,
    );

    // Place code in the REMAPPED physical page: MOV AL, 0xEE ; HLT
    bus.ram[PG_REMAP_PAGE as usize] = 0xB0; // MOV AL, imm8
    bus.ram[(PG_REMAP_PAGE + 1) as usize] = 0xEE;
    bus.ram[(PG_REMAP_PAGE + 2) as usize] = 0xF4; // HLT

    cpu.step(&mut bus); // MOV AL, 0xEE - fetched from remapped page
    assert_eq!(cpu.al(), 0xEE, "code fetch must go through paging");

    cpu.step(&mut bus); // HLT
    assert!(cpu.halted());
}

/// PUSH/POP go through paging with a remapped stack page.
#[test]
fn paging_push_pop_through_page_tables() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_paged_protected_mode(&mut bus);
    state.set_esp(0x0100); // Put ESP in page 0 so remapping page 0 affects it.
    cpu.load_state(&state);

    // Remap page 0 (linear 0x0000-0x0FFF) to PG_REMAP_PAGE.
    write_dword_at(&mut bus, PG_PAGE_TABLE_0, PG_REMAP_PAGE | PTE_P | PTE_RW);

    // PUSH 0x1234 ; POP BX
    place_at(
        &mut bus,
        PG_CODE_BASE,
        &[
            0x68, 0x34, 0x12, // PUSH 0x1234
            0x5B, // POP BX
        ],
    );

    let sp_before = cpu.esp();
    cpu.step(&mut bus); // PUSH
    cpu.step(&mut bus); // POP

    assert_eq!(
        cpu.bx(),
        0x1234,
        "POP should read value pushed through paged stack"
    );
    assert_eq!(cpu.esp(), sp_before, "SP should be restored after PUSH+POP");

    // Verify the push went to the remapped physical page.
    let push_addr = (sp_before - 2) as usize;
    assert_eq!(
        bus.ram[PG_REMAP_PAGE as usize + push_addr],
        0x34,
        "PUSH should write to remapped stack page (low byte)"
    );
    assert_eq!(
        bus.ram[PG_REMAP_PAGE as usize + push_addr + 1],
        0x12,
        "PUSH should write to remapped stack page (high byte)"
    );
}

/// MOVSB copies data between two differently-mapped pages.
#[test]
fn paging_movsb_between_remapped_pages() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_paged_protected_mode(&mut bus);
    cpu.load_state(&state);

    // Source: DS:SI -> data page (identity-mapped at PG_DATA_BASE).
    // Dest: ES:DI -> we'll remap ES's target page.
    //
    // DS base = PG_DATA_BASE = 0x10000
    // ES base = PG_DATA_BASE = 0x10000
    // We'll use SI=0x0000 (source) and DI=0x0100 (dest).
    // Remap the dest area: PTE for linear (PG_DATA_BASE + 0x0100) is the
    // same page (PG_DATA_BASE >> 12 = 0x10), so we need a different approach.
    //
    // Use a different segment setup: put source data at DS:0, dest at ES:0
    // with ES pointing to a remapped page.

    // Write source data.
    bus.ram[PG_DATA_BASE as usize] = 0xAA;
    bus.ram[(PG_DATA_BASE + 1) as usize] = 0xBB;
    bus.ram[(PG_DATA_BASE + 2) as usize] = 0xCC;

    // Set up: CLD; MOV CX, 3; REP MOVSB
    // SI=0x0000, DI=0x0100 (both within same page, identity-mapped)
    place_at(
        &mut bus,
        PG_CODE_BASE,
        &[
            0xFC, // CLD
            0xB9, 0x03, 0x00, // MOV CX, 3
            0xF3, 0xA4, // REP MOVSB
        ],
    );

    // Set SI=0, DI=0x100 via state reload.
    let mut s = state;
    s.set_esi(0x0000);
    s.set_edi(0x0100);
    cpu.load_state(&s);

    cpu.step(&mut bus); // CLD
    cpu.step(&mut bus); // MOV CX, 3
    cpu.step(&mut bus); // REP MOVSB (3 iterations)

    assert_eq!(bus.ram[(PG_DATA_BASE + 0x100) as usize], 0xAA);
    assert_eq!(bus.ram[(PG_DATA_BASE + 0x101) as usize], 0xBB);
    assert_eq!(bus.ram[(PG_DATA_BASE + 0x102) as usize], 0xCC);
    assert_eq!(cpu.cx(), 0, "CX should be 0 after REP MOVSB");
}

#[test]
fn paging_fault_on_cross_page_modrm_fetch_does_not_raise_secondary_fault() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_paged_protected_mode_32bit(&mut bus);
    state.set_eip(0x0FFF);
    state.set_eax(0x1F0000);
    cpu.load_state(&state);

    let original_esp = cpu.esp();
    place_at(&mut bus, PG_CODE_BASE + 0x0FFF, &[0x8B]);

    let next_code_page = (PG_CODE_BASE + 0x1000) >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + next_code_page * 4,
        (PG_CODE_BASE + 0x1000) & 0xFFFF_F000,
    );

    cpu.step(&mut bus);

    assert_eq!(cpu.cs(), PG_CS_SEL);
    assert_eq!(cpu.eip(), PG_PF_HANDLER_IP as u32);
    assert_eq!(
        cpu.cr2,
        PG_CODE_BASE + 0x1000,
        "CR2 must hold the faulting linear address"
    );

    let handler_sp = cpu.esp();
    assert_eq!(
        handler_sp,
        original_esp.wrapping_sub(16),
        "exactly one #PF frame (error_code + EIP + CS + EFLAGS) must be on the stack"
    );
    assert_eq!(
        read_dword_at(&bus, PG_STACK_BASE + handler_sp + 4),
        0x0FFF,
        "fault frame return EIP must point to the faulting instruction"
    );
    assert_eq!(
        read_dword_at(&bus, PG_STACK_BASE + handler_sp + 8) as u16,
        PG_CS32_SEL,
        "fault frame return CS must be the original 32-bit code selector"
    );
}

/// #PF on instruction fetch from a not-present code page.
#[test]
fn paging_fault_on_code_fetch() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_paged_protected_mode(&mut bus);
    cpu.load_state(&state);

    // Unmap the code page.
    let pte_index = PG_CODE_BASE >> 12;
    write_dword_at(&mut bus, PG_PAGE_TABLE_0 + pte_index * 4, 0);

    cpu.step(&mut bus); // fetch faults
    cpu.step(&mut bus); // HLT in #PF handler

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PG_PF_HANDLER_IP as u32 + 1);
    assert_eq!(
        cpu.cr2, PG_CODE_BASE,
        "CR2 should hold the faulting code fetch address"
    );
}

/// #PF when PUSH writes to a not-present stack page.
/// Uses ring 3 so the #PF handler gets a TSS stack switch to a valid stack.
#[test]
fn paging_fault_on_stack_push() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_paged_protected_mode(&mut bus);
    make_ring3(&mut state);
    state.set_esp(0x0100); // Put ring-3 ESP in page 0 of ring-3 stack.
    cpu.load_state(&state);

    // PDE: present + R/W + U/S (user can traverse directory).
    let pde = read_dword_at(&bus, PG_PAGE_DIR);
    write_dword_at(&mut bus, PG_PAGE_DIR, pde | PTE_US);

    // Make ring-3 code page user-accessible.
    let code_pte_index = PG_RING3_CODE_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + code_pte_index * 4,
        PG_RING3_CODE_BASE | PTE_P | PTE_RW | PTE_US,
    );

    // Unmap ring-3 stack page (PG_RING3_STACK_BASE >> 12 = page 0x20).
    // The #PF handler switches to TSS ESP0=0xFFF0 / SS0 (page 0xF), still mapped.
    let stack_pte_index = PG_RING3_STACK_BASE >> 12;
    write_dword_at(&mut bus, PG_PAGE_TABLE_0 + stack_pte_index * 4, 0);

    // PUSH AX (0x50)
    place_at(&mut bus, PG_RING3_CODE_BASE, &[0x50]);
    cpu.step(&mut bus); // faults on stack write
    cpu.step(&mut bus); // HLT in #PF handler

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PG_PF_HANDLER_IP as u32 + 1);

    // Error code should have W/R=1 (write) and U/S=1 (user).
    // Handler stack is at SS0:ESP0 from TSS (ESP0=0xFFF0, page 0xF, SS base=0).
    let sp = cpu.esp();
    let error_code = read_word_at(&bus, PG_STACK_BASE + sp);
    assert_eq!(
        error_code & 0x06,
        0x06,
        "stack push #PF error code should have W/R=1 and U/S=1"
    );
}

/// User-mode access denied when PDE lacks U/S, even if PTE has U/S.
/// Since all pages are under PDE 0, removing U/S from PDE blocks ALL user
/// access including code fetch. This test verifies the code fetch fault.
#[test]
fn paging_fault_user_denied_by_pde() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_paged_protected_mode(&mut bus);
    make_ring3(&mut state);
    cpu.load_state(&state);

    // PDE: present + R/W but NO U/S. This blocks all user-mode access.
    write_dword_at(&mut bus, PG_PAGE_DIR, PG_PAGE_TABLE_0 | PTE_P | PTE_RW);

    // PTE for ring-3 code page: present + R/W + U/S. The PDE still blocks it.
    let code_pte_index = PG_RING3_CODE_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + code_pte_index * 4,
        PG_RING3_CODE_BASE | PTE_P | PTE_RW | PTE_US,
    );

    place_at(&mut bus, PG_RING3_CODE_BASE, &[0xA0, 0x00, 0x00]);
    cpu.step(&mut bus); // code fetch faults (PDE lacks U/S)
    cpu.step(&mut bus); // HLT in #PF handler (runs at ring 0, supervisor access OK)

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PG_PF_HANDLER_IP as u32 + 1);
    let sp = cpu.esp();
    let error_code = read_word_at(&bus, PG_STACK_BASE + sp);
    assert_eq!(
        error_code, 0x0005,
        "user denied by PDE: present + read + user"
    );
    assert_eq!(
        cpu.cr2, PG_RING3_CODE_BASE,
        "CR2 = faulting code fetch address"
    );
}

/// Ring-3 read from a user-accessible page succeeds.
#[test]
fn paging_user_read_succeeds() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_paged_protected_mode(&mut bus);
    make_ring3(&mut state);
    cpu.load_state(&state);

    // PDE: P + RW + US.
    write_dword_at(
        &mut bus,
        PG_PAGE_DIR,
        PG_PAGE_TABLE_0 | PTE_P | PTE_RW | PTE_US,
    );

    // PTE for data page: P + RW + US.
    let pte_index = PG_DATA_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + pte_index * 4,
        PG_DATA_BASE | PTE_P | PTE_RW | PTE_US,
    );

    // Also make ring-3 code page user-accessible.
    let code_pte_index = PG_RING3_CODE_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + code_pte_index * 4,
        PG_RING3_CODE_BASE | PTE_P | PTE_RW | PTE_US,
    );

    // Also make ring-3 stack page user-accessible.
    let stack_pte_index = PG_RING3_STACK_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + stack_pte_index * 4,
        PG_RING3_STACK_BASE | PTE_P | PTE_RW | PTE_US,
    );

    bus.ram[(PG_DATA_BASE + 0x30) as usize] = 0x99;

    // MOV AL, [0x0030]
    place_at(&mut bus, PG_RING3_CODE_BASE, &[0xA0, 0x30, 0x00]);
    cpu.step(&mut bus);

    assert_eq!(cpu.al(), 0x99, "ring-3 read from user page should succeed");
    assert!(!cpu.halted(), "should not fault");
}

/// Ring-3 write to a user R/W page succeeds.
#[test]
fn paging_user_write_succeeds() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_paged_protected_mode(&mut bus);
    make_ring3(&mut state);
    cpu.load_state(&state);

    // PDE: P + RW + US.
    write_dword_at(
        &mut bus,
        PG_PAGE_DIR,
        PG_PAGE_TABLE_0 | PTE_P | PTE_RW | PTE_US,
    );

    // All user pages: P + RW + US.
    let pte_index = PG_DATA_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + pte_index * 4,
        PG_DATA_BASE | PTE_P | PTE_RW | PTE_US,
    );
    let code_pte = PG_RING3_CODE_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + code_pte * 4,
        PG_RING3_CODE_BASE | PTE_P | PTE_RW | PTE_US,
    );
    let stack_pte = PG_RING3_STACK_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + stack_pte * 4,
        PG_RING3_STACK_BASE | PTE_P | PTE_RW | PTE_US,
    );

    // MOV BYTE [0x0060], 0x77
    place_at(
        &mut bus,
        PG_RING3_CODE_BASE,
        &[0xC6, 0x06, 0x60, 0x00, 0x77],
    );
    cpu.step(&mut bus);

    assert_eq!(
        bus.ram[(PG_DATA_BASE + 0x60) as usize],
        0x77,
        "ring-3 write to user R/W page should succeed"
    );
    assert!(!cpu.halted());
}

/// With paging disabled (CR0.PG=0), addresses pass through with 24-bit mask.
#[test]
fn paging_disabled_identity_passthrough() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    // Set up protected mode WITHOUT paging (CR0 = PE only).
    let mut state = setup_paged_protected_mode(&mut bus);
    state.cr0 = 0x0000_0001; // PE only, PG=0
    cpu.load_state(&state);

    bus.ram[(PG_DATA_BASE + 0x10) as usize] = 0x42;

    // MOV AL, [0x0010]
    place_at(&mut bus, PG_CODE_BASE, &[0xA0, 0x10, 0x00]);
    cpu.step(&mut bus);

    assert_eq!(
        cpu.al(),
        0x42,
        "with PG=0, memory access should use linear address directly"
    );
}

/// MOV CR3, EAX stores the page directory base and flushes TLB.
#[test]
fn paging_mov_cr3_changes_page_directory() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_paged_protected_mode(&mut bus);
    cpu.load_state(&state);

    // Set up a second page directory at 0x100000.
    let pd2 = 0x100000u32;
    let pt2 = 0x101000u32;
    // PDE 0 -> pt2.
    write_dword_at(&mut bus, pd2, pt2 | PTE_P | PTE_RW);
    // Map the code page identity.
    let code_pte = PG_CODE_BASE >> 12;
    write_dword_at(&mut bus, pt2 + code_pte * 4, PG_CODE_BASE | PTE_P | PTE_RW);
    // Map the data page to PG_REMAP_PAGE.
    let data_pte = PG_DATA_BASE >> 12;
    write_dword_at(&mut bus, pt2 + data_pte * 4, PG_REMAP_PAGE | PTE_P | PTE_RW);
    // Map the stack page identity.
    write_dword_at(&mut bus, pt2, PG_STACK_BASE | PTE_P | PTE_RW);

    bus.ram[(PG_REMAP_PAGE + 0x20) as usize] = 0xDD;

    // MOV EAX, pd2 ; MOV CR3, EAX ; MOV AL, [0x20]
    place_at(
        &mut bus,
        PG_CODE_BASE,
        &[
            0x66,
            0xB8,
            pd2 as u8,
            (pd2 >> 8) as u8,
            (pd2 >> 16) as u8,
            (pd2 >> 24) as u8, // MOV EAX, pd2
            0x0F,
            0x22,
            0xD8, // MOV CR3, EAX
            0xA0,
            0x20,
            0x00, // MOV AL, [0x20]
        ],
    );

    cpu.step(&mut bus); // MOV EAX, pd2
    cpu.step(&mut bus); // MOV CR3, EAX
    assert_eq!(cpu.state.cr3, pd2);

    cpu.step(&mut bus); // MOV AL, [0x20]
    assert_eq!(
        cpu.al(),
        0xDD,
        "after MOV CR3, data read should use the new page directory"
    );
}

/// Two distinct linear pages (in the same page table) map to different physical pages.
#[test]
fn paging_two_pages_different_physical() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_paged_protected_mode(&mut bus);
    cpu.load_state(&state);

    // DS base is PG_DATA_BASE = 0x10000.
    // DS:0x0000 -> page 0x10 (identity -> phys 0x10000).
    // DS:0x1000 -> page 0x11 (we'll remap to PG_REMAP_PAGE).
    let pte_11 = (PG_DATA_BASE >> 12) + 1; // page index 0x11
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + pte_11 * 4,
        PG_REMAP_PAGE | PTE_P | PTE_RW,
    );

    bus.ram[PG_DATA_BASE as usize] = 0x11;
    bus.ram[PG_REMAP_PAGE as usize] = 0x22;

    // MOV AL, [0x0000] ; MOV AH, [0x1000]
    // Note: [0x1000] needs 67h prefix for 32-bit addressing in 16-bit mode,
    // or we can use a register-indirect approach.
    // Simpler: MOV AL, [0x0000] then change SI and do MOV AH, [SI].
    place_at(
        &mut bus,
        PG_CODE_BASE,
        &[
            0xA0, 0x00, 0x00, // MOV AL, [0x0000]
            0xBE, 0x00, 0x10, // MOV SI, 0x1000
            0x8A, 0x24, // MOV AH, [SI]
        ],
    );

    cpu.step(&mut bus); // MOV AL, [0x0000]
    assert_eq!(cpu.al(), 0x11);

    cpu.step(&mut bus); // MOV SI, 0x1000
    cpu.step(&mut bus); // MOV AH, [SI]
    assert_eq!(
        cpu.ah(),
        0x22,
        "second page should read from remapped physical page"
    );
}

/// Paging requires both CR0.PG=1 AND CR0.PE=1.
#[test]
fn paging_requires_pe_and_pg() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    // Set up protected mode with paging.
    let state = setup_paged_protected_mode(&mut bus);
    cpu.load_state(&state);

    // Verify paging is active.
    assert_eq!(cpu.state.cr0 & 0x8000_0001, 0x8000_0001);

    // Remap data page so we can distinguish paged vs unpaged.
    let pte_index = PG_DATA_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + pte_index * 4,
        PG_REMAP_PAGE | PTE_P | PTE_RW,
    );
    bus.ram[PG_REMAP_PAGE as usize] = 0xAA;
    bus.ram[PG_DATA_BASE as usize] = 0xBB;

    // Read should go through page tables -> get 0xAA from remapped page.
    place_at(&mut bus, PG_CODE_BASE, &[0xA0, 0x00, 0x00]);
    cpu.step(&mut bus);
    assert_eq!(
        cpu.al(),
        0xAA,
        "with paging enabled, should read remapped page"
    );
}

const PG_CS32_SEL: u16 = 0x0040;

fn setup_paged_protected_mode_32bit(bus: &mut TestBus) -> cpu::I386State {
    let mut state = setup_paged_protected_mode(bus);

    // Add a 32-bit code segment at GDT index 8 (selector 0x40).
    // D-bit (0x40 in granularity byte) = 1 means 32-bit default operand/address size.
    write_gdt_entry32(bus, PG_GDT_BASE, 8, PG_CODE_BASE, 0xFFFF, 0x9B, 0x40);

    state.set_cs(PG_CS32_SEL);
    state.seg_bases[cpu::SegReg32::CS as usize] = PG_CODE_BASE;
    state.seg_rights[cpu::SegReg32::CS as usize] = 0x9B;
    state.seg_granularity[cpu::SegReg32::CS as usize] = 0x40;

    state
}

#[test]
fn paging_tlb_hit_read_write() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_paged_protected_mode(&mut bus);
    cpu.load_state(&state);

    bus.ram[(PG_DATA_BASE + 0x20) as usize] = 0xAA;

    // Clear A/D bits on PDE and PTE for data page.
    let pte_index = PG_DATA_BASE >> 12;
    write_dword_at(&mut bus, PG_PAGE_DIR, PG_PAGE_TABLE_0 | PTE_P | PTE_RW);
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + pte_index * 4,
        PG_DATA_BASE | PTE_P | PTE_RW,
    );

    // MOV AL, [0x0020] (TLB miss); MOV AL, [0x0020] (TLB hit);
    // MOV [0x0021], AL (write, TLB hit); HLT
    place_at(
        &mut bus,
        PG_CODE_BASE,
        &[
            0xA0, 0x20, 0x00, // MOV AL, [0x0020]
            0xA0, 0x20, 0x00, // MOV AL, [0x0020]
            0xA2, 0x21, 0x00, // MOV [0x0021], AL
            0xF4, // HLT
        ],
    );
    cpu.step(&mut bus); // first read (TLB miss)
    cpu.step(&mut bus); // second read (TLB hit)
    cpu.step(&mut bus); // write (TLB hit)
    cpu.step(&mut bus); // HLT

    assert_eq!(cpu.al(), 0xAA);
    assert_eq!(bus.ram[(PG_DATA_BASE + 0x21) as usize], 0xAA);
    assert!(cpu.halted());

    let pte = read_dword_at(&bus, PG_PAGE_TABLE_0 + pte_index * 4);
    assert_ne!(pte & PTE_A, 0, "PTE Accessed bit must be set");
    assert_ne!(pte & PTE_D, 0, "PTE Dirty bit must be set after write");
}

#[test]
fn paging_tlb_hit_preserves_user_read_permission() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_paged_protected_mode(&mut bus);
    cpu.load_state(&state);

    write_dword_at(
        &mut bus,
        PG_PAGE_DIR,
        PG_PAGE_TABLE_0 | PTE_P | PTE_RW | PTE_US,
    );
    let code_pte_index = PG_RING3_CODE_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + code_pte_index * 4,
        PG_RING3_CODE_BASE | PTE_P | PTE_RW | PTE_US,
    );
    let stack_pte_index = PG_RING3_STACK_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + stack_pte_index * 4,
        PG_RING3_STACK_BASE | PTE_P | PTE_RW | PTE_US,
    );
    let data_pte_index = PG_DATA_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + data_pte_index * 4,
        PG_DATA_BASE | PTE_P | PTE_RW,
    );

    bus.ram[PG_DATA_BASE as usize] = 0x5A;
    place_at(&mut bus, PG_CODE_BASE, &[0xA0, 0x00, 0x00]);
    cpu.step(&mut bus);
    assert_eq!(cpu.al(), 0x5A);

    make_ring3(&mut cpu.state);
    cpu.state.stored_cpl = 3;
    cpu.state.set_eip(0);
    place_at(&mut bus, PG_RING3_CODE_BASE, &[0xA0, 0x00, 0x00]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PG_PF_HANDLER_IP as u32 + 1);
    let error_code = read_word_at(&bus, PG_STACK_BASE + cpu.esp());
    assert_eq!(error_code, 0x0005, "present + read + user");
}

#[test]
fn paging_fetch_cache_preserves_user_read_permission() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_paged_protected_mode(&mut bus);
    state.seg_bases[cpu::SegReg32::CS as usize] = PG_RING3_CODE_BASE;
    cpu.load_state(&state);

    write_dword_at(
        &mut bus,
        PG_PAGE_DIR,
        PG_PAGE_TABLE_0 | PTE_P | PTE_RW | PTE_US,
    );
    let code_pte_index = PG_RING3_CODE_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + code_pte_index * 4,
        PG_RING3_CODE_BASE | PTE_P | PTE_RW,
    );
    let stack_pte_index = PG_RING3_STACK_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + stack_pte_index * 4,
        PG_RING3_STACK_BASE | PTE_P | PTE_RW | PTE_US,
    );

    place_at(&mut bus, PG_RING3_CODE_BASE, &[0x90, 0xF4]);
    cpu.step(&mut bus);

    make_ring3(&mut cpu.state);
    cpu.state.stored_cpl = 3;
    cpu.state.set_eip(1);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PG_PF_HANDLER_IP as u32 + 1);
    let error_code = read_word_at(&bus, PG_STACK_BASE + cpu.esp());
    assert_eq!(error_code, 0x0005, "present + fetch + user");
}

#[test]
fn paging_accessed_dirty_bits_already_set() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_paged_protected_mode(&mut bus);
    cpu.load_state(&state);

    let pte_index = PG_DATA_BASE >> 12;
    // Pre-set A+D on PDE and PTE.
    write_dword_at(
        &mut bus,
        PG_PAGE_DIR,
        PG_PAGE_TABLE_0 | PTE_P | PTE_RW | PTE_A | PTE_D,
    );
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + pte_index * 4,
        PG_DATA_BASE | PTE_P | PTE_RW | PTE_A | PTE_D,
    );

    bus.ram[(PG_DATA_BASE + 0x30) as usize] = 0xBB;

    // MOV BYTE [0x0030], 0xCC ; HLT
    place_at(
        &mut bus,
        PG_CODE_BASE,
        &[0xC6, 0x06, 0x30, 0x00, 0xCC, 0xF4],
    );
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_eq!(bus.ram[(PG_DATA_BASE + 0x30) as usize], 0xCC);
    assert!(cpu.halted());

    let pde = read_dword_at(&bus, PG_PAGE_DIR);
    let pte = read_dword_at(&bus, PG_PAGE_TABLE_0 + pte_index * 4);
    assert_eq!(
        pde,
        PG_PAGE_TABLE_0 | PTE_P | PTE_RW | PTE_A | PTE_D,
        "PDE bits should be unchanged (skip-write optimization)"
    );
    assert_eq!(
        pte,
        PG_DATA_BASE | PTE_P | PTE_RW | PTE_A | PTE_D,
        "PTE bits should be unchanged (skip-write optimization)"
    );
}

#[test]
fn paging_pde_pte_permission_conflict() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    // Scenario (a): PDE has U/S+R/W, PTE has U/S but NO R/W. User write -> error 0x0007.
    let mut state = setup_paged_protected_mode(&mut bus);
    make_ring3(&mut state);
    cpu.load_state(&state);

    write_dword_at(
        &mut bus,
        PG_PAGE_DIR,
        PG_PAGE_TABLE_0 | PTE_P | PTE_RW | PTE_US,
    );

    let code_pte_index = PG_RING3_CODE_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + code_pte_index * 4,
        PG_RING3_CODE_BASE | PTE_P | PTE_RW | PTE_US,
    );
    let stack_pte_index = PG_RING3_STACK_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + stack_pte_index * 4,
        PG_RING3_STACK_BASE | PTE_P | PTE_RW | PTE_US,
    );

    // Data PTE: present + U/S but NO R/W.
    let data_pte_index = PG_DATA_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + data_pte_index * 4,
        PG_DATA_BASE | PTE_P | PTE_US,
    );

    // MOV BYTE [0x0000], 0x11 from ring 3.
    place_at(
        &mut bus,
        PG_RING3_CODE_BASE,
        &[0xC6, 0x06, 0x00, 0x00, 0x11],
    );
    cpu.step(&mut bus); // fault
    cpu.step(&mut bus); // HLT in handler

    assert!(cpu.halted());
    let sp = cpu.esp();
    let error_code = read_word_at(&bus, PG_STACK_BASE + sp);
    assert_eq!(error_code, 0x0007, "present + write + user");

    // Scenario (b): PDE has NO U/S, PTE has U/S+R/W. User read -> error 0x0005.
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_paged_protected_mode(&mut bus);
    make_ring3(&mut state);
    cpu.load_state(&state);

    // PDE: present + R/W but NO U/S.
    write_dword_at(&mut bus, PG_PAGE_DIR, PG_PAGE_TABLE_0 | PTE_P | PTE_RW);

    let code_pte_index = PG_RING3_CODE_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + code_pte_index * 4,
        PG_RING3_CODE_BASE | PTE_P | PTE_RW | PTE_US,
    );

    place_at(&mut bus, PG_RING3_CODE_BASE, &[0xA0, 0x00, 0x00]);
    cpu.step(&mut bus); // code fetch faults (PDE lacks U/S)
    cpu.step(&mut bus); // HLT

    assert!(cpu.halted());
    let sp = cpu.esp();
    let error_code = read_word_at(&bus, PG_STACK_BASE + sp);
    assert_eq!(error_code, 0x0005, "present + read + user");
}

#[test]
fn paging_stosb_cross_page_boundary() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_paged_protected_mode(&mut bus);
    state.seg_bases[cpu::SegReg32::ES as usize] = 0;
    state.set_edi(0x0FFC);
    state.set_ecx(8);
    state.flags.df = false;
    cpu.load_state(&state);

    // Set AL=0xDD, then CLD; REP STOSB; HLT
    place_at(
        &mut bus,
        PG_CODE_BASE,
        &[
            0xB0, 0xDD, // MOV AL, 0xDD
            0xFC, // CLD
            0xF3, 0xAA, // REP STOSB
            0xF4, // HLT
        ],
    );
    cpu.step(&mut bus); // MOV AL, 0xDD
    cpu.step(&mut bus); // CLD
    cpu.step(&mut bus); // REP STOSB
    cpu.step(&mut bus); // HLT

    assert!(cpu.halted());
    for i in 0..8u32 {
        assert_eq!(
            bus.ram[(0x0FFC + i) as usize],
            0xDD,
            "byte at 0x{:04X} should be 0xDD",
            0x0FFC + i
        );
    }

    // Check A+D bits on both pages.
    let pte_page0 = read_dword_at(&bus, PG_PAGE_TABLE_0);
    let pte_page1 = read_dword_at(&bus, PG_PAGE_TABLE_0 + 4);
    assert_ne!(pte_page0 & (PTE_A | PTE_D), 0, "page 0 should have A+D");
    assert_ne!(pte_page1 & (PTE_A | PTE_D), 0, "page 1 should have A+D");
}

#[test]
fn paging_cmpsb_cross_page_boundary() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_paged_protected_mode(&mut bus);
    // DS base=0, ES base=0 for direct linear addressing.
    state.seg_bases[cpu::SegReg32::DS as usize] = 0;
    state.seg_bases[cpu::SegReg32::ES as usize] = 0;
    state.set_esi(0x0FFE);
    state.set_edi(0x10FFE);
    state.set_ecx(4);
    state.flags.df = false;
    cpu.load_state(&state);

    // Identity-map page 0x10 and 0x11 (for ES:EDI region at 0x10000-0x11FFF).
    // These should already be identity-mapped by setup_identity_page_tables.

    // Write identical patterns at the page boundaries.
    let pattern = [0xAA, 0xBB, 0xCC, 0xDD];
    for (i, &byte) in pattern.iter().enumerate() {
        bus.ram[0x0FFE + i] = byte;
        bus.ram[0x10FFE + i] = byte;
    }

    // CLD; REPE CMPSB; HLT
    place_at(
        &mut bus,
        PG_CODE_BASE,
        &[
            0xFC, // CLD
            0xF3, 0xA6, // REPE CMPSB
            0xF4, // HLT
        ],
    );
    cpu.step(&mut bus); // CLD
    cpu.step(&mut bus); // REPE CMPSB
    cpu.step(&mut bus); // HLT

    assert!(cpu.halted());
    assert!(cpu.state.flags.zf(), "ZF should be set (strings equal)");
    assert_eq!(cpu.cx(), 0, "CX should be 0 (all compared)");
}

#[test]
fn paging_push_dword_cross_page() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_paged_protected_mode(&mut bus);
    // SS base=0, ESP=0x1002 so PUSH dword writes to 0x0FFE-0x1001 (crosses 0x1000).
    state.seg_bases[cpu::SegReg32::SS as usize] = 0;
    state.set_esp(0x1002);
    state.set_eax(0xDEADBEEF);
    cpu.load_state(&state);

    // 66 50 = PUSH EAX (in 16-bit code segment, 0x66 prefix promotes to dword)
    // F4 = HLT
    place_at(&mut bus, PG_CODE_BASE, &[0x66, 0x50, 0xF4]);
    cpu.step(&mut bus); // PUSH EAX
    cpu.step(&mut bus); // HLT

    assert!(cpu.halted());
    assert_eq!(cpu.esp(), 0x0FFE);
    assert_eq!(bus.ram[0x0FFE], 0xEF);
    assert_eq!(bus.ram[0x0FFF], 0xBE);
    assert_eq!(bus.ram[0x1000], 0xAD);
    assert_eq!(bus.ram[0x1001], 0xDE);

    let pte_page0 = read_dword_at(&bus, PG_PAGE_TABLE_0);
    let pte_page1 = read_dword_at(&bus, PG_PAGE_TABLE_0 + 4);
    assert_ne!(pte_page0 & (PTE_A | PTE_D), 0, "page 0 should have A+D");
    assert_ne!(pte_page1 & (PTE_A | PTE_D), 0, "page 1 should have A+D");
}

#[test]
fn paging_outsb_translates_source() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_paged_protected_mode(&mut bus);

    // Remap DS data page (linear 0x10000) -> physical PG_REMAP_PAGE (0xB0000).
    let pte_index = PG_DATA_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + pte_index * 4,
        PG_REMAP_PAGE | PTE_P | PTE_RW,
    );

    // Write 0x42 to the remapped physical location.
    bus.ram[(PG_REMAP_PAGE + 0x20) as usize] = 0x42;

    // Set SI=0x0020, DX=0x1234.
    state.set_esi(0x0020);
    state.set_edx(0x1234);
    state.flags.df = false;
    cpu.load_state(&state);

    // OUTSB; HLT (opcode 0x6E, F4)
    place_at(&mut bus, PG_CODE_BASE, &[0x6E, 0xF4]);
    cpu.step(&mut bus); // OUTSB
    cpu.step(&mut bus); // HLT

    assert!(cpu.halted());
    assert!(
        bus.io_log.contains(&(0x1234, 0x42)),
        "OUTSB should write 0x42 to port 0x1234, got {:?}",
        bus.io_log
    );
    assert_eq!(cpu.state.esi() & 0xFFFF, 0x0021, "ESI should advance by 1");
}

#[test]
fn paging_lgdt_from_paged_memory() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_paged_protected_mode(&mut bus);
    cpu.load_state(&state);

    // Remap DS data page (linear 0x10000) -> physical PG_REMAP_PAGE (0xB0000).
    let pte_index = PG_DATA_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + pte_index * 4,
        PG_REMAP_PAGE | PTE_P | PTE_RW,
    );

    // Write a 6-byte LGDT operand at physical 0xB0050: limit=0x00FF, base=0x00012345.
    bus.ram[(PG_REMAP_PAGE + 0x50) as usize] = 0xFF; // limit low
    bus.ram[(PG_REMAP_PAGE + 0x51) as usize] = 0x00; // limit high
    bus.ram[(PG_REMAP_PAGE + 0x52) as usize] = 0x45; // base byte 0
    bus.ram[(PG_REMAP_PAGE + 0x53) as usize] = 0x23; // base byte 1
    bus.ram[(PG_REMAP_PAGE + 0x54) as usize] = 0x01; // base byte 2
    bus.ram[(PG_REMAP_PAGE + 0x55) as usize] = 0x00; // base byte 3 (16-bit LGDT uses 3 bytes)

    // LGDT [0x0050] ; HLT
    // 16-bit mode: 0F 01 16 50 00 = LGDT [0x0050]
    place_at(
        &mut bus,
        PG_CODE_BASE,
        &[0x0F, 0x01, 0x16, 0x50, 0x00, 0xF4],
    );
    cpu.step(&mut bus); // LGDT
    cpu.step(&mut bus); // HLT

    assert!(cpu.halted());
    assert_eq!(cpu.state.gdt_limit, 0x00FF, "GDT limit should be 0x00FF");
    assert_eq!(
        cpu.state.gdt_base & 0x00FFFFFF,
        0x00012345,
        "GDT base should be 0x00012345 (24-bit in 16-bit mode)"
    );
}

#[test]
fn paging_prefix_66_idempotent() {
    // 16-bit code segment: double 0x66 should still result in 32-bit push (idempotent).
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_paged_protected_mode(&mut bus);
    state.set_esp(0xFFF0);
    state.set_eax(0x12345678);
    cpu.load_state(&state);

    // 66 66 50 F4 = double prefix + PUSH EAX + HLT
    place_at(&mut bus, PG_CODE_BASE, &[0x66, 0x66, 0x50, 0xF4]);
    cpu.step(&mut bus); // PUSH EAX (with double 0x66 prefix)
    cpu.step(&mut bus); // HLT

    assert!(cpu.halted());
    let esp = cpu.esp();
    assert_eq!(
        0xFFF0 - esp,
        4,
        "double 0x66 in 16-bit segment should be idempotent (32-bit push)"
    );
    let pushed = read_dword_at(&bus, PG_STACK_BASE + esp);
    assert_eq!(pushed, 0x12345678);

    // 32-bit code segment: double 0x66 should result in 16-bit push (idempotent override).
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_paged_protected_mode_32bit(&mut bus);
    state.set_esp(0xFFF0);
    state.set_eax(0xAABBCCDD);
    cpu.load_state(&state);

    // 66 66 50 F4 = double prefix + PUSH AX + HLT (in 32-bit segment, 0x66 = 16-bit)
    place_at(&mut bus, PG_CODE_BASE, &[0x66, 0x66, 0x50, 0xF4]);
    cpu.step(&mut bus); // PUSH AX (16-bit due to idempotent override)
    cpu.step(&mut bus); // HLT

    assert!(cpu.halted());
    let esp = cpu.esp();
    assert_eq!(
        0xFFF0 - esp,
        2,
        "double 0x66 in 32-bit segment should be idempotent (16-bit push)"
    );
    let pushed = read_word_at(&bus, PG_STACK_BASE + esp);
    assert_eq!(pushed, 0xCCDD);
}

#[test]
fn paging_prefix_67_idempotent() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_paged_protected_mode(&mut bus);
    cpu.load_state(&state);

    bus.ram[(PG_DATA_BASE + 0x100) as usize] = 0x99;

    // 67 67 8A 05 00 01 00 00 F4
    // Double 0x67 prefix + MOV AL, [disp32] with disp32=0x00000100 + HLT
    // In 16-bit segment, 0x67 switches to 32-bit addressing.
    // Idempotent: still 32-bit addressing. Toggle: back to 16-bit (wrong decode).
    place_at(
        &mut bus,
        PG_CODE_BASE,
        &[0x67, 0x67, 0x8A, 0x05, 0x00, 0x01, 0x00, 0x00, 0xF4],
    );
    cpu.step(&mut bus); // MOV AL, [0x00000100]
    cpu.step(&mut bus); // HLT

    assert!(cpu.halted());
    assert_eq!(
        cpu.al(),
        0x99,
        "double 0x67 in 16-bit segment should be idempotent (32-bit address)"
    );
}

#[test]
fn paging_supervisor_override_ring3_interrupt() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_paged_protected_mode(&mut bus);
    make_ring3(&mut state);
    state.flags.if_flag = false;
    state.flags.iopl = 3;
    cpu.load_state(&state);

    // PDE: P + RW + US.
    write_dword_at(
        &mut bus,
        PG_PAGE_DIR,
        PG_PAGE_TABLE_0 | PTE_P | PTE_RW | PTE_US,
    );

    // Ring-3 code page: user-accessible.
    let code_pte_index = PG_RING3_CODE_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + code_pte_index * 4,
        PG_RING3_CODE_BASE | PTE_P | PTE_RW | PTE_US,
    );
    // Ring-3 stack page: user-accessible.
    let stack_pte_index = PG_RING3_STACK_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + stack_pte_index * 4,
        PG_RING3_STACK_BASE | PTE_P | PTE_RW | PTE_US,
    );

    // Mark IDT pages as supervisor-only (clear U/S).
    let idt_pte_start = PG_IDT_BASE >> 12;
    for i in 0..2u32 {
        write_dword_at(
            &mut bus,
            PG_PAGE_TABLE_0 + (idt_pte_start + i) * 4,
            (PG_IDT_BASE + i * 0x1000) | PTE_P | PTE_RW,
        );
    }

    // Set up IRQ vector 0x40 -> ring-0 handler at a known offset in code segment.
    let irq_handler_offset: u32 = 0xA000;
    write_idt_gate(
        &mut bus,
        PG_IDT_BASE,
        0x40,
        irq_handler_offset,
        PG_CS_SEL,
        14,
        0,
    );
    // Place HLT at the handler.
    bus.ram[(PG_CODE_BASE + irq_handler_offset) as usize] = 0xF4;

    // Ensure the handler's page is mapped (supervisor-only is fine for ring-0 handler).
    let handler_page = (PG_CODE_BASE + irq_handler_offset) >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + handler_page * 4,
        (PG_CODE_BASE + irq_handler_offset) & 0xFFFFF000 | PTE_P | PTE_RW,
    );

    // Ring-3 code: STI; JMP $-2 (infinite loop, waiting for IRQ).
    // STI = 0xFB, JMP short -2 = EB FE
    place_at(&mut bus, PG_RING3_CODE_BASE, &[0xFB, 0xEB, 0xFE]);

    // Set up IRQ.
    bus.irq_pending = true;
    bus.irq_vector = 0x40;
    cpu.signal_irq();

    cpu.step(&mut bus); // STI (inhibits IRQ for one instruction)
    cpu.step(&mut bus); // JMP $-2 - IRQ fires after this instruction
    cpu.step(&mut bus); // HLT in IRQ handler

    assert!(
        cpu.halted(),
        "CPU should halt in IRQ handler (IDT on supervisor page should be accessible)"
    );
    assert_eq!(
        cpu.ip(),
        irq_handler_offset + 1,
        "should be in the IRQ handler"
    );
}

#[test]
fn paging_fault_pending_stops_dispatch() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_paged_protected_mode(&mut bus);
    cpu.load_state(&state);

    // Unmap the data page.
    let pte_index = PG_DATA_BASE >> 12;
    write_dword_at(&mut bus, PG_PAGE_TABLE_0 + pte_index * 4, 0);

    // Sentinel: ensure physical 0x10010 is 0x00.
    bus.ram[(PG_DATA_BASE + 0x10) as usize] = 0x00;

    // MOV [0x0000], AL (fault); MOV BYTE [0x0010], 0xFF (should NOT execute); HLT
    place_at(
        &mut bus,
        PG_CODE_BASE,
        &[
            0xA2, 0x00, 0x00, // MOV [0x0000], AL
            0xC6, 0x06, 0x10, 0x00, 0xFF, // MOV BYTE [0x0010], 0xFF
            0xF4, // HLT
        ],
    );
    cpu.step(&mut bus); // faults on first MOV
    cpu.step(&mut bus); // HLT in #PF handler

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PG_PF_HANDLER_IP as u32 + 1);
    assert_eq!(cpu.cr2, PG_DATA_BASE, "CR2 should be faulting address");
    assert_eq!(
        bus.ram[(PG_DATA_BASE + 0x10) as usize],
        0x00,
        "second MOV should not have executed"
    );
}

#[test]
fn paging_rep_movsb_fault_mid_operation() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_paged_protected_mode(&mut bus);
    // Source: DS base=0x10000, SI=0. Fill source with 0x11-0x18.
    for i in 0..8u32 {
        bus.ram[(PG_DATA_BASE + i) as usize] = (0x11 + i) as u8;
    }

    // Destination: ES base=0x30000, DI=0x0FFC. Linear addresses 0x30FFC-0x31003.
    // Page 0x30 is identity-mapped, page 0x31 is unmapped -> fault at 0x31000.
    state.seg_bases[cpu::SegReg32::ES as usize] = 0x30000;
    state.set_esi(0x0000);
    state.set_edi(0x0FFC);
    state.set_ecx(8);
    state.flags.df = false;

    // Ensure page 0x30 is mapped.
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + 0x30 * 4,
        0x30000 | PTE_P | PTE_RW,
    );
    // Unmap page 0x31.
    write_dword_at(&mut bus, PG_PAGE_TABLE_0 + 0x31 * 4, 0);

    cpu.load_state(&state);

    // CLD; REP MOVSB; HLT
    place_at(
        &mut bus,
        PG_CODE_BASE,
        &[
            0xFC, // CLD
            0xF3, 0xA4, // REP MOVSB
            0xF4, // HLT
        ],
    );
    cpu.step(&mut bus); // CLD
    cpu.step(&mut bus); // REP MOVSB - faults mid-operation at destination 0x31000
    cpu.step(&mut bus); // HLT in #PF handler

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PG_PF_HANDLER_IP as u32 + 1);
    assert_eq!(
        cpu.cr2, 0x31000,
        "CR2 should be 0x31000 (unmapped dest page)"
    );

    // First 4 bytes should have been written (0x30FFC-0x30FFF).
    assert_eq!(bus.ram[0x30FFC], 0x11);
    assert_eq!(bus.ram[0x30FFD], 0x12);
    assert_eq!(bus.ram[0x30FFE], 0x13);
    assert_eq!(bus.ram[0x30FFF], 0x14);

    // ECX should reflect remaining count (4 remaining).
    assert_eq!(cpu.cx(), 4, "CX should be 4 (4 iterations remaining)");
}

#[test]
fn cr0_wp_bit_masked_on_386() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_paged_protected_mode(&mut bus);
    cpu.load_state(&state);

    // MOV EAX, 0xFFFFFFFF ; 66 B8 FF FF FF FF
    // MOV CR0, EAX        ; 0F 22 C0
    place_at(
        &mut bus,
        PG_CODE_BASE,
        &[0x66, 0xB8, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 0x22, 0xC0],
    );
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_eq!(
        cpu.cr0 & 0x0001_0000,
        0,
        "WP bit (16) must be masked off on 386"
    );
}

#[test]
fn cr0_wp_bit_accepted_on_486() {
    let mut cpu: I386<{ CPU_MODEL_486_DX }> = I386::new();
    let mut bus = TestBus::new();

    let state = setup_paged_protected_mode(&mut bus);
    cpu.load_state(&state);

    // MOV EAX, 0xFFFFFFFF ; 66 B8 FF FF FF FF
    // MOV CR0, EAX        ; 0F 22 C0
    place_at(
        &mut bus,
        PG_CODE_BASE,
        &[0x66, 0xB8, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 0x22, 0xC0],
    );
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_ne!(
        cpu.cr0 & 0x0001_0000,
        0,
        "WP bit (16) must be accepted on 486"
    );
}

#[test]
fn supervisor_write_to_readonly_page_without_wp() {
    let mut cpu: I386<{ CPU_MODEL_486_DX }> = I386::new();
    let mut bus = TestBus::new();

    let state = setup_paged_protected_mode(&mut bus);
    cpu.load_state(&state);
    // WP=0 (default): supervisor writes to read-only pages should succeed.

    let pte_index = PG_DATA_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + pte_index * 4,
        PG_DATA_BASE | PTE_P,
    );

    // MOV BYTE [0x0050], 0xCC ; C6 06 50 00 CC
    place_at(&mut bus, PG_CODE_BASE, &[0xC6, 0x06, 0x50, 0x00, 0xCC]);
    cpu.step(&mut bus);

    assert_eq!(bus.ram[(PG_DATA_BASE + 0x50) as usize], 0xCC);
    assert!(
        !cpu.halted(),
        "486 supervisor with WP=0 must write read-only pages"
    );
}

#[test]
fn supervisor_write_to_readonly_page_with_wp() {
    let mut cpu: I386<{ CPU_MODEL_486_DX }> = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_paged_protected_mode(&mut bus);
    // Set WP bit (16) in CR0.
    state.cr0 |= 0x0001_0000;
    cpu.load_state(&state);

    let pte_index = PG_DATA_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + pte_index * 4,
        PG_DATA_BASE | PTE_P,
    );

    // MOV BYTE [0x0050], 0xCC ; C6 06 50 00 CC
    place_at(&mut bus, PG_CODE_BASE, &[0xC6, 0x06, 0x50, 0x00, 0xCC]);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(
        cpu.halted(),
        "486 supervisor with WP=1 must fault on write to read-only page"
    );
    assert_eq!(cpu.ip(), PG_PF_HANDLER_IP as u32 + 1);

    // Error code: P=1 (protection violation), W/R=1 (write), U/S=0 (supervisor).
    // That's bits 0 and 1 = 0x03.
    let handler_esp = cpu.esp();
    let error_code = read_dword_at(&bus, PG_STACK_BASE + handler_esp);
    assert_eq!(error_code, 0x03, "error code should be P=1, W/R=1, U/S=0");
}

#[test]
fn supervisor_read_from_readonly_page_with_wp() {
    let mut cpu: I386<{ CPU_MODEL_486_DX }> = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_paged_protected_mode(&mut bus);
    state.cr0 |= 0x0001_0000;
    cpu.load_state(&state);

    let pte_index = PG_DATA_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + pte_index * 4,
        PG_DATA_BASE | PTE_P,
    );

    bus.ram[(PG_DATA_BASE + 0x50) as usize] = 0xAB;

    // MOV AL, [0x0050] ; A0 50 00
    place_at(&mut bus, PG_CODE_BASE, &[0xA0, 0x50, 0x00]);
    cpu.step(&mut bus);

    assert!(
        !cpu.halted(),
        "WP only affects writes, reads should succeed"
    );
    assert_eq!(cpu.al(), 0xAB);
}

/// PC-9821AS/AP advertise a 32-bit address space at the machine layer
/// (`MachineModel::address_mask` returns `0xFFFF_FFFF` for them, and the bus
/// recognises the PEGC linear-frame-buffer high alias at `0xFFF0_0000-0xFFF7_FFFF`).
/// Paging must therefore deliver high physical addresses to the bus unmodified;
/// any internal clamp inside the CPU paging module would silently redirect
/// 32-bit-aliased writes (e.g. Win95's PEGC framebuffer mapping) to the wrong
/// destination. Mapping a linear page to physical `0xFFF0_5000` and writing
/// through it must reach the bus with the high bits intact.
#[test]
fn paging_passes_high_physical_address_to_bus() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_paged_protected_mode(&mut bus);
    cpu.load_state(&state);

    // Remap the linear page covering DS:0x0000 (linear = PG_DATA_BASE = 0x10000)
    // to physical 0xFFF0_5000 - a page-aligned address with bits 24..31 set.
    const HIGH_PHYS_PAGE: u32 = 0xFFF0_5000;
    let pte_index = PG_DATA_BASE >> 12;
    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + pte_index * 4,
        HIGH_PHYS_PAGE | PTE_P | PTE_RW,
    );

    // MOV BYTE [0x0050], 0x42 ; C6 06 50 00 42
    place_at(&mut bus, PG_CODE_BASE, &[0xC6, 0x06, 0x50, 0x00, 0x42]);
    cpu.step(&mut bus);

    let expected = HIGH_PHYS_PAGE | 0x50;
    assert!(
        bus.write_address_log.contains(&expected),
        "bus.write_byte must be invoked with the full 32-bit physical address \
         {:#010X}; observed writes: {:?}",
        expected,
        bus.write_address_log,
    );
}

/// PC-9801RA uses a 386DX whose chipset only decodes 24 physical address bits
/// while paging is disabled (real mode and unpaged protected mode). The RA BIOS
/// enumerates extended memory by writing test patterns at addresses above 16MB
/// and observing the chipset wrap to low memory; that probe fails if the CPU
/// forwards the full 32-bit linear address to the bus. With paging enabled,
/// addresses must still pass through unmodified.
#[test]
fn paging_disabled_masks_address_to_24_bit() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_paged_protected_mode(&mut bus);
    state.cr0 = 0x0000_0001; // PE only, PG=0
    state.seg_bases[cpu::SegReg32::DS as usize] = 0x0E00_0000;
    cpu.load_state(&state);

    // MOV BYTE [0x0050], 0x42 ; C6 06 50 00 42
    place_at(&mut bus, PG_CODE_BASE, &[0xC6, 0x06, 0x50, 0x00, 0x42]);
    cpu.step(&mut bus);

    let unmasked = 0x0E00_0050;
    let masked = unmasked & 0x00FF_FFFF;
    assert!(
        bus.write_address_log.contains(&masked),
        "bus.write_byte must receive the 24-bit-masked address {:#010X}; \
         observed writes: {:?}",
        masked,
        bus.write_address_log,
    );
    assert!(
        !bus.write_address_log.contains(&unmasked),
        "bus.write_byte must NOT receive the unmasked address {:#010X}; \
         observed writes: {:?}",
        unmasked,
        bus.write_address_log,
    );
}

/// Validate-then-commit IRET path: a #PF raised while reading the new SS/CS
/// descriptor must deliver the fault with the source IRET frame intact - the
/// pushed CS:EIP must still point at the IRET, EFLAGS/ESP must be unchanged,
/// and the original IRET stack frame must remain on the source stack.
///
/// The PTE that backs the new SS GDT entry is cleared by relocating the GDT
/// to straddle two pages: ring-0 entries (CS at index 1) stay on the mapped
/// page so the #PF handler can dispatch, while the ring-3 SS/CS entries used
/// by the IRET migration end up on the page we clear.
#[test]
fn paging_iret_inter_priv_pf_on_descriptor_read_preserves_source_frame() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_paged_protected_mode(&mut bus);

    // Relocate the GDT so that entries 0..1 (null, ring-0 CS used by the #PF
    // handler) live on page 0x80000 while entries 2..7 spill into page
    // 0x81000. We can then clear the PTE backing 0x81000 to fault the new SS
    // descriptor read while keeping the #PF dispatch path intact.
    const SPLIT_GDT_BASE: u32 = 0x80FF0;
    for entry_index in 0..8u32 {
        let src = (PG_GDT_BASE + entry_index * 8) as usize;
        let dst = (SPLIT_GDT_BASE + entry_index * 8) as usize;
        for byte_offset in 0..8usize {
            bus.ram[dst + byte_offset] = bus.ram[src + byte_offset];
        }
    }
    state.gdt_base = SPLIT_GDT_BASE;
    state.gdt_limit = 8 * 8 - 1;

    cpu.load_state(&state);

    place_at(&mut bus, PG_CODE_BASE, &[0xCF]);

    let ring3_ip: u16 = 0x0100;
    let ring3_esp_low: u16 = 0xFF00;
    let original_esp = cpu.esp();

    write_word_at(&mut bus, PG_STACK_BASE + original_esp, ring3_ip);
    write_word_at(&mut bus, PG_STACK_BASE + original_esp + 2, PG_RING3_CS_SEL);
    write_word_at(&mut bus, PG_STACK_BASE + original_esp + 4, 0x0202);
    write_word_at(&mut bus, PG_STACK_BASE + original_esp + 6, ring3_esp_low);
    write_word_at(&mut bus, PG_STACK_BASE + original_esp + 8, PG_RING3_SS_SEL);

    let pre_cs = cpu.cs();
    let pre_ss = cpu.ss();
    let pre_ds = cpu.ds();
    let pre_es = cpu.es();

    // Clear the PTE that covers the GDT entries used by the IRET migration.
    // The page that backs entry 1 (#PF handler's CS) remains mapped.
    const SPILL_PTE_INDEX: u32 = 0x81;
    let saved_spill_pte = read_dword_at(&bus, PG_PAGE_TABLE_0 + SPILL_PTE_INDEX * 4);
    write_dword_at(&mut bus, PG_PAGE_TABLE_0 + SPILL_PTE_INDEX * 4, 0);

    cpu.step(&mut bus); // IRET -> #PF during the ring-3 SS descriptor read
    cpu.step(&mut bus); // HLT in #PF handler

    assert!(cpu.halted(), "CPU should halt inside the #PF handler");
    assert_eq!(
        cpu.ip(),
        PG_PF_HANDLER_IP as u32 + 1,
        "control must transfer to the #PF handler"
    );

    let handler_sp = cpu.esp();
    assert_eq!(
        handler_sp,
        original_esp.wrapping_sub(16),
        "fault frame must consume exactly 4 dwords on the source stack"
    );

    let pushed_eip = read_dword_at(&bus, PG_STACK_BASE + handler_sp + 4);
    let pushed_cs = read_dword_at(&bus, PG_STACK_BASE + handler_sp + 8);
    assert_eq!(
        pushed_eip, 0,
        "fault frame EIP must point back at the IRET instruction"
    );
    assert_eq!(
        pushed_cs & 0xFFFF,
        pre_cs as u32,
        "fault frame CS must still be the source CS"
    );

    assert_eq!(cpu.cs(), pre_cs, "CS must not have been committed");
    assert_eq!(cpu.ss(), pre_ss, "SS must not have been committed");
    assert_eq!(cpu.ds(), pre_ds, "DS must not have been committed");
    assert_eq!(cpu.es(), pre_es, "ES must not have been committed");

    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + SPILL_PTE_INDEX * 4,
        saved_spill_pte,
    );
    assert_eq!(
        read_word_at(&bus, PG_STACK_BASE + original_esp),
        ring3_ip,
        "original IRET frame EIP word must survive the failed IRET"
    );
    assert_eq!(
        read_word_at(&bus, PG_STACK_BASE + original_esp + 2),
        PG_RING3_CS_SEL,
        "original IRET frame CS word must survive the failed IRET"
    );
    assert_eq!(
        read_word_at(&bus, PG_STACK_BASE + original_esp + 8),
        PG_RING3_SS_SEL,
        "original IRET frame SS word must survive the failed IRET"
    );
}

/// Discriminating test for the validate-then-commit IRET pipeline: place the
/// fault during the data-segment revalidation phase, where the previous
/// commit-as-you-go path would have already committed CS, EIP, EFLAGS, SS
/// and ESP before discovering the descriptor was unreadable. The new pipeline
/// must validate every operand first and deliver the #PF same-privilege from
/// CPL 0 with no architectural state changed.
///
/// This test would fail against the old pipeline because the fault would be
/// delivered inter-privilege (CPL 3 -> CPL 0) and the fault frame ESP would
/// land on the TSS-supplied stack with 6 dwords pushed, not 4.
#[test]
fn paging_iret_inter_priv_pf_on_data_segment_decode_does_not_commit() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_paged_protected_mode(&mut bus);

    // Build a CPL-0 data descriptor at GDT entry index 512 (offset 0x1000 in
    // the GDT page). The descriptor sits in physical page 0x81000 so we can
    // make its read fault while leaving the standard ring-0/ring-3 entries on
    // the mapped page 0x80000.
    const HIGH_DS_SEL: u16 = 0x1000;
    write_gdt_entry16(&mut bus, PG_GDT_BASE, 512, PG_DATA_BASE, 0xFFFF, 0x93);
    state.gdt_limit = 0x1007;

    // Pre-load DS at CPL 0 with the high-index selector. Cached rights match
    // the descriptor we placed; the cached fields are only consulted on
    // segment access, not during the IRET pre-decision.
    state.set_ds(HIGH_DS_SEL);
    state.seg_bases[cpu::SegReg32::DS as usize] = PG_DATA_BASE;
    state.seg_rights[cpu::SegReg32::DS as usize] = 0x93;
    state.seg_limits[cpu::SegReg32::DS as usize] = 0xFFFF;

    cpu.load_state(&state);

    place_at(&mut bus, PG_CODE_BASE, &[0xCF]);

    let ring3_ip: u16 = 0x0100;
    let ring3_esp_low: u16 = 0xFF00;
    let original_esp = cpu.esp();

    write_word_at(&mut bus, PG_STACK_BASE + original_esp, ring3_ip);
    write_word_at(&mut bus, PG_STACK_BASE + original_esp + 2, PG_RING3_CS_SEL);
    write_word_at(&mut bus, PG_STACK_BASE + original_esp + 4, 0x0202);
    write_word_at(&mut bus, PG_STACK_BASE + original_esp + 6, ring3_esp_low);
    write_word_at(&mut bus, PG_STACK_BASE + original_esp + 8, PG_RING3_SS_SEL);

    let pre_cs = cpu.cs();
    let pre_ss = cpu.ss();
    let pre_ds = cpu.ds();
    let pre_es = cpu.es();

    // Unmap the page that backs the high-index GDT entry. SS/CS validation
    // still reads from page 0x80000 (mapped); only the DS pre-decision read
    // will fault.
    const SPILL_PTE_INDEX: u32 = 0x81;
    let saved_spill_pte = read_dword_at(&bus, PG_PAGE_TABLE_0 + SPILL_PTE_INDEX * 4);
    write_dword_at(&mut bus, PG_PAGE_TABLE_0 + SPILL_PTE_INDEX * 4, 0);

    cpu.step(&mut bus); // IRET -> #PF during DS check_data_segment_at_cpl
    cpu.step(&mut bus); // HLT in #PF handler

    assert!(cpu.halted(), "CPU should halt inside the #PF handler");
    assert_eq!(
        cpu.ip(),
        PG_PF_HANDLER_IP as u32 + 1,
        "control must transfer to the #PF handler"
    );

    // The fault must be delivered same-privilege from CPL 0. The old pipeline
    // would have already loaded CS/SS to ring 3, making this an inter-priv
    // fault with a 6-dword frame on a TSS-supplied stack.
    let handler_sp = cpu.esp();
    assert_eq!(
        handler_sp,
        original_esp.wrapping_sub(16),
        "same-privilege #PF must consume 4 dwords on the unchanged source stack"
    );

    let pushed_eip = read_dword_at(&bus, PG_STACK_BASE + handler_sp + 4);
    let pushed_cs = read_dword_at(&bus, PG_STACK_BASE + handler_sp + 8);
    assert_eq!(
        pushed_eip, 0,
        "fault frame EIP must still point at the IRET instruction"
    );
    assert_eq!(
        pushed_cs & 0xFFFF,
        pre_cs as u32,
        "fault frame CS must still be the source ring-0 CS"
    );

    assert_eq!(cpu.cs(), pre_cs, "CS must not have been committed");
    assert_eq!(cpu.ss(), pre_ss, "SS must not have been committed");
    assert_eq!(
        cpu.ds(),
        pre_ds,
        "DS must not have been mutated by the IRET pre-decision"
    );
    assert_eq!(cpu.es(), pre_es, "ES must not have been committed");

    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + SPILL_PTE_INDEX * 4,
        saved_spill_pte,
    );
}

/// RET FAR inter-priv must validate every operand (CS, SS, DS/ES/FS/GS) before
/// committing any state. A #PF raised during the DS revalidation phase has to
/// be delivered same-privilege from CPL 0 with CS, SS, EIP and ESP unchanged.
#[test]
fn paging_retf_inter_priv_pf_on_data_segment_decode_does_not_commit() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_paged_protected_mode(&mut bus);

    const HIGH_DS_SEL: u16 = 0x1000;
    write_gdt_entry16(&mut bus, PG_GDT_BASE, 512, PG_DATA_BASE, 0xFFFF, 0x93);
    state.gdt_limit = 0x1007;

    state.set_ds(HIGH_DS_SEL);
    state.seg_bases[cpu::SegReg32::DS as usize] = PG_DATA_BASE;
    state.seg_rights[cpu::SegReg32::DS as usize] = 0x93;
    state.seg_limits[cpu::SegReg32::DS as usize] = 0xFFFF;

    cpu.load_state(&state);

    // 0xCB - RET FAR (16-bit operand size).
    place_at(&mut bus, PG_CODE_BASE, &[0xCB]);

    let ring3_ip: u16 = 0x0100;
    let ring3_sp_low: u16 = 0xFF00;
    let original_esp = cpu.esp();

    write_word_at(&mut bus, PG_STACK_BASE + original_esp, ring3_ip);
    write_word_at(&mut bus, PG_STACK_BASE + original_esp + 2, PG_RING3_CS_SEL);
    write_word_at(&mut bus, PG_STACK_BASE + original_esp + 4, ring3_sp_low);
    write_word_at(&mut bus, PG_STACK_BASE + original_esp + 6, PG_RING3_SS_SEL);

    let pre_cs = cpu.cs();
    let pre_ss = cpu.ss();
    let pre_ds = cpu.ds();
    let pre_es = cpu.es();

    const SPILL_PTE_INDEX: u32 = 0x81;
    let saved_spill_pte = read_dword_at(&bus, PG_PAGE_TABLE_0 + SPILL_PTE_INDEX * 4);
    write_dword_at(&mut bus, PG_PAGE_TABLE_0 + SPILL_PTE_INDEX * 4, 0);

    cpu.step(&mut bus); // RET FAR -> #PF during DS check_data_segment_at_cpl
    cpu.step(&mut bus); // HLT in #PF handler

    assert!(cpu.halted(), "CPU should halt inside the #PF handler");
    assert_eq!(
        cpu.ip(),
        PG_PF_HANDLER_IP as u32 + 1,
        "control must transfer to the #PF handler"
    );

    let handler_sp = cpu.esp();
    assert_eq!(
        handler_sp,
        original_esp.wrapping_sub(16),
        "same-privilege #PF must consume 4 dwords on the unchanged source stack"
    );

    let pushed_eip = read_dword_at(&bus, PG_STACK_BASE + handler_sp + 4);
    let pushed_cs = read_dword_at(&bus, PG_STACK_BASE + handler_sp + 8);
    assert_eq!(
        pushed_eip, 0,
        "fault frame EIP must still point at the RET FAR instruction"
    );
    assert_eq!(
        pushed_cs & 0xFFFF,
        pre_cs as u32,
        "fault frame CS must still be the source ring-0 CS"
    );

    assert_eq!(cpu.cs(), pre_cs, "CS must not have been committed");
    assert_eq!(cpu.ss(), pre_ss, "SS must not have been committed");
    assert_eq!(
        cpu.ds(),
        pre_ds,
        "DS must not have been mutated by the RET FAR pre-decision"
    );
    assert_eq!(cpu.es(), pre_es, "ES must not have been committed");

    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + SPILL_PTE_INDEX * 4,
        saved_spill_pte,
    );
}

/// RET FAR imm16 inter-priv: same atomicity contract as the bare RET FAR. The
/// immediate stack-cleanup operand must not change the validate-then-commit
/// shape; a #PF during DS revalidation has to land same-privilege with all
/// segment caches untouched.
#[test]
fn paging_retf_imm_inter_priv_pf_on_data_segment_decode_does_not_commit() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_paged_protected_mode(&mut bus);

    const HIGH_DS_SEL: u16 = 0x1000;
    write_gdt_entry16(&mut bus, PG_GDT_BASE, 512, PG_DATA_BASE, 0xFFFF, 0x93);
    state.gdt_limit = 0x1007;

    state.set_ds(HIGH_DS_SEL);
    state.seg_bases[cpu::SegReg32::DS as usize] = PG_DATA_BASE;
    state.seg_rights[cpu::SegReg32::DS as usize] = 0x93;
    state.seg_limits[cpu::SegReg32::DS as usize] = 0xFFFF;

    cpu.load_state(&state);

    // 0xCA imm16 - RET FAR imm16, immediate = 8. The inter-priv path reads
    // new_sp / new_ss from `sp + 4 + imm`, so the SS/SP slots sit 8 bytes
    // higher than for the bare RET FAR variant.
    place_at(&mut bus, PG_CODE_BASE, &[0xCA, 0x08, 0x00]);

    let ring3_ip: u16 = 0x0100;
    let ring3_sp_low: u16 = 0xFF00;
    let original_esp = cpu.esp();

    write_word_at(&mut bus, PG_STACK_BASE + original_esp, ring3_ip);
    write_word_at(&mut bus, PG_STACK_BASE + original_esp + 2, PG_RING3_CS_SEL);
    write_word_at(&mut bus, PG_STACK_BASE + original_esp + 12, ring3_sp_low);
    write_word_at(&mut bus, PG_STACK_BASE + original_esp + 14, PG_RING3_SS_SEL);

    let pre_cs = cpu.cs();
    let pre_ss = cpu.ss();
    let pre_ds = cpu.ds();
    let pre_es = cpu.es();

    const SPILL_PTE_INDEX: u32 = 0x81;
    let saved_spill_pte = read_dword_at(&bus, PG_PAGE_TABLE_0 + SPILL_PTE_INDEX * 4);
    write_dword_at(&mut bus, PG_PAGE_TABLE_0 + SPILL_PTE_INDEX * 4, 0);

    cpu.step(&mut bus); // RET FAR imm16 -> #PF during DS check
    cpu.step(&mut bus); // HLT in #PF handler

    assert!(cpu.halted(), "CPU should halt inside the #PF handler");
    let handler_sp = cpu.esp();
    assert_eq!(
        handler_sp,
        original_esp.wrapping_sub(16),
        "same-privilege #PF must consume 4 dwords on the unchanged source stack"
    );

    let pushed_eip = read_dword_at(&bus, PG_STACK_BASE + handler_sp + 4);
    let pushed_cs = read_dword_at(&bus, PG_STACK_BASE + handler_sp + 8);
    assert_eq!(
        pushed_eip, 0,
        "fault frame EIP must still point at the RET FAR imm16 instruction"
    );
    assert_eq!(
        pushed_cs & 0xFFFF,
        pre_cs as u32,
        "fault frame CS must still be the source ring-0 CS"
    );

    assert_eq!(cpu.cs(), pre_cs, "CS must not have been committed");
    assert_eq!(cpu.ss(), pre_ss, "SS must not have been committed");
    assert_eq!(cpu.ds(), pre_ds, "DS must not have been mutated");
    assert_eq!(cpu.es(), pre_es, "ES must not have been committed");

    write_dword_at(
        &mut bus,
        PG_PAGE_TABLE_0 + SPILL_PTE_INDEX * 4,
        saved_spill_pte,
    );
}

/// 386 call gate descriptor: target offset + selector + param count + rights.
/// Rights byte: P=1, DPL, S=0, type (4=286 call gate, 12=386 call gate).
#[allow(clippy::too_many_arguments)]
fn write_gdt_call_gate(
    bus: &mut TestBus,
    gdt_base: u32,
    entry_index: u32,
    offset: u32,
    selector: u16,
    param_count: u8,
    gate_type: u8,
    dpl: u8,
) {
    let addr = (gdt_base + entry_index * 8) as usize;
    bus.ram[addr] = offset as u8;
    bus.ram[addr + 1] = (offset >> 8) as u8;
    bus.ram[addr + 2] = selector as u8;
    bus.ram[addr + 3] = (selector >> 8) as u8;
    bus.ram[addr + 4] = param_count & 0x1F;
    bus.ram[addr + 5] = 0x80 | (dpl << 5) | gate_type;
    bus.ram[addr + 6] = (offset >> 16) as u8;
    bus.ram[addr + 7] = (offset >> 24) as u8;
}

/// Inter-priv CALL FAR through a 386 call gate must validate every page
/// touched by the parameter copy and the new-stack pushes before committing
/// SS:ESP to the target stack. If a parameter read from the OLD (ring-3) stack
/// triggers a #PF, the fault must be reported with SS:ESP still pointing at
/// the caller's stack so the #PF is delivered inter-privilege from CPL 3.
#[test]
fn paging_call_gate_pf_on_param_copy_does_not_commit_ss_esp() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_paged_protected_mode(&mut bus);

    // 386 call gate at GDT entry 8 -> selector 0x43 (entry 8 << 3 | RPL=3).
    // Gate target = PG_CS_SEL (DPL=0), gate_count = 2 dwords, target offset =
    // PG_GP_HANDLER_IP (HLT). Gate DPL = 3 so ring-3 CALL FAR can reach it.
    const CALL_GATE_INDEX: u32 = 8;
    write_gdt_call_gate(
        &mut bus,
        PG_GDT_BASE,
        CALL_GATE_INDEX,
        PG_GP_HANDLER_IP as u32,
        PG_CS_SEL,
        2,
        12,
        3,
    );
    state.gdt_limit = 9 * 8 - 1;

    // PTE_US on every page touched by the CPL=3 caller and by the still-CPL=3
    // code in code_descriptor (which reads gate fields and TSS fields without
    // the supervisor-override path used for descriptor decode).
    let pde = read_dword_at(&bus, PG_PAGE_DIR);
    write_dword_at(&mut bus, PG_PAGE_DIR, pde | PTE_US);
    for page in [
        PG_RING3_CODE_BASE,
        PG_RING3_STACK_BASE,
        PG_GDT_BASE,
        PG_TSS_BASE,
        PG_STACK_BASE,
    ] {
        write_dword_at(
            &mut bus,
            PG_PAGE_TABLE_0 + (page >> 12) * 4,
            page | PTE_P | PTE_RW | PTE_US,
        );
    }

    make_ring3(&mut state);
    state.set_esp(0x0FF0);
    cpu.load_state(&state);

    let r3_sp = cpu.esp();
    // Parameter placeholders at SS:r3_sp + 0 and SS:r3_sp + 4 on the ring-3
    // stack page. After the unmap below they will be unreachable.
    write_dword_at(&mut bus, PG_RING3_STACK_BASE + r3_sp, 0xAAAA_AAAA);
    write_dword_at(&mut bus, PG_RING3_STACK_BASE + r3_sp + 4, 0xBBBB_BBBB);

    // CALL FAR 0x0043:0x0000 (the offset bytes are ignored; gate provides
    // the actual target offset).
    let gate_selector: u16 = ((CALL_GATE_INDEX as u16) << 3) | 3;
    place_at(
        &mut bus,
        PG_RING3_CODE_BASE,
        &[
            0x9A,
            0x00,
            0x00,
            gate_selector as u8,
            (gate_selector >> 8) as u8,
        ],
    );

    // Unmap the ring-3 stack page so the parameter copy faults.
    let stack_pte_index: u32 = PG_RING3_STACK_BASE >> 12;
    let saved_pte = read_dword_at(&bus, PG_PAGE_TABLE_0 + stack_pte_index * 4);
    write_dword_at(&mut bus, PG_PAGE_TABLE_0 + stack_pte_index * 4, 0);

    cpu.step(&mut bus); // CALL FAR -> #PF during param copy
    cpu.step(&mut bus); // HLT in #PF handler

    assert!(cpu.halted(), "CPU should halt inside the #PF handler");
    assert_eq!(
        cpu.ip(),
        PG_PF_HANDLER_IP as u32 + 1,
        "control must transfer to the #PF handler"
    );

    // The #PF must be delivered inter-privilege (CPL 3 -> CPL 0) using
    // TSS.SS0:ESP0 = PG_SS_SEL:0xFFF0. The dispatch frame pushes 6 dwords:
    // saved_ss, saved_esp, EFLAGS, CS, EIP, error code (top to bottom).
    let pushed_ss = read_dword_at(&bus, PG_STACK_BASE + 0xFFEC);
    let pushed_sp = read_dword_at(&bus, PG_STACK_BASE + 0xFFE8);

    assert_eq!(
        pushed_ss & 0xFFFF,
        PG_RING3_SS_SEL as u32,
        "dispatch frame SS must be the caller's ring-3 SS"
    );
    assert_eq!(
        pushed_sp, r3_sp,
        "dispatch frame ESP must be the caller's ring-3 ESP"
    );

    write_dword_at(&mut bus, PG_PAGE_TABLE_0 + stack_pte_index * 4, saved_pte);
}

/// Task switch must mark the new TSS busy and save the old task's CPU
/// state atomically: a #PF on the new TSS busy-bit write must leave the
/// caller's TSS unmodified (still marked busy in the GDT) and the old
/// task's state save un-applied.
///
/// 486 with CR0.WP=1 lets a supervisor write to a read-only page fault, so
/// we can place the new TSS descriptor on a read-only GDT page while the
/// surrounding state save / decode reads still succeed.
#[test]
fn paging_task_switch_pf_on_new_tss_busy_bit_leaves_old_tss_busy() {
    let mut cpu: I386<{ CPU_MODEL_486_DX }> = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_paged_protected_mode(&mut bus);
    state.cr0 |= 0x0001_0000; // CR0.WP = 1.

    // Mark the current TSS (entry 7 = PG_TSS_SEL) busy in the GDT to match
    // the cached tr_rights = 0x8B from `setup_paged_protected_mode`.
    bus.ram[(PG_GDT_BASE + 7 * 8 + 5) as usize] = 0x8B;

    // Lay down a second TSS descriptor at GDT entry 512: the descriptor
    // straddles into page 0x81 so we can make that page read-only without
    // affecting the rest of the GDT. Available 386 TSS, base = TSS1_BASE,
    // limit = 103.
    const TSS1_BASE: u32 = 0x71000;
    const TSS1_GDT_ENTRY: u32 = 512;
    let tss1_sel: u16 = (TSS1_GDT_ENTRY as u16) << 3;
    write_gdt_entry16(
        &mut bus,
        PG_GDT_BASE,
        TSS1_GDT_ENTRY as u16,
        TSS1_BASE,
        103,
        0x89,
    );
    state.gdt_limit = (TSS1_GDT_ENTRY * 8 + 7) as u16;

    // Pre-populate TSS1 body with a plausible state so a successful switch
    // would resume cleanly. Both buggy and fixed paths fault before TSS1
    // takes effect, but we want any post-fault descriptor walks to see a
    // legal target.
    write_dword_at(&mut bus, TSS1_BASE + 32, 0x0000); // EIP
    write_dword_at(&mut bus, TSS1_BASE + 36, 0x0002); // EFLAGS
    write_word_at(&mut bus, TSS1_BASE + 76, PG_CS_SEL); // CS
    write_word_at(&mut bus, TSS1_BASE + 80, PG_SS_SEL); // SS
    write_word_at(&mut bus, TSS1_BASE + 84, PG_DS_SEL); // DS

    cpu.load_state(&state);

    // Sentinel in EAX so a successful state save into the old TSS body is
    // detectable afterwards.
    cpu.state.set_eax(0xCAFE_BABE);

    // JMP FAR 0x1000:0 - 0xEA imm16:imm16 (TSS selector dispatch).
    place_at(
        &mut bus,
        PG_CODE_BASE,
        &[0xEA, 0x00, 0x00, tss1_sel as u8, (tss1_sel >> 8) as u8],
    );

    // Make page 0x81 (which holds the TSS1 descriptor) read-only: present
    // but no PTE_RW. With CR0.WP=1 the busy-bit write at GDT[512]+5 = 0x81005
    // faults; reads of the descriptor itself still succeed.
    write_dword_at(&mut bus, PG_PAGE_TABLE_0 + 0x81 * 4, 0x81000 | PTE_P);

    cpu.step(&mut bus); // JMP FAR (task switch) -> #PF on new TSS busy bit
    cpu.step(&mut bus); // HLT in #PF handler

    assert!(cpu.halted(), "CPU should halt inside the #PF handler");
    assert_eq!(
        cpu.ip(),
        PG_PF_HANDLER_IP as u32 + 1,
        "control must transfer to the #PF handler"
    );

    let tss0_rights = bus.ram[(PG_GDT_BASE + 7 * 8 + 5) as usize];
    assert_eq!(
        tss0_rights, 0x8B,
        "old TSS descriptor must still be marked busy after the failed switch"
    );

    let saved_eax = read_dword_at(&bus, PG_TSS_BASE + 40);
    assert_eq!(
        saved_eax, 0,
        "old TSS body must not have been written (the state save must be \
         pre-translated and aborted before any commit)"
    );
}

/// Pseudo-precise faulting on the segment-descriptor accessed-bit toggle.
/// IRET inter-privilege must not commit CS to its new selector before the
/// A-bit write on that descriptor has completed; otherwise a #PF on the
/// A-bit write (here forced via 486 + CR0.WP=1 + a read-only descriptor
/// table page) is observed inter-privilege from CPL 3 even though the
/// IRET should have aborted with CPL still 0.
///
/// The GDT is relocated so the ring-0 entries (used by the #PF handler
/// dispatch) stay on a writable page while the ring-3 entries (the IRET
/// migration target) land on a read-only page.
#[test]
fn paging_iret_inter_priv_pf_on_cs_a_bit_does_not_commit_cs() {
    let mut cpu: I386<{ CPU_MODEL_486_DX }> = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_paged_protected_mode(&mut bus);
    state.cr0 |= 0x0001_0000; // CR0.WP = 1.

    // Relocate GDT so entries 0-3 (null + ring-0 CS/DS/SS) stay on page
    // 0x80 and entries 4-7 (ring-3 CS/DS/SS + TSS) spill onto page 0x81.
    const SPLIT_GDT_BASE: u32 = 0x80FE0;
    for entry_index in 0..8u32 {
        let src = (PG_GDT_BASE + entry_index * 8) as usize;
        let dst = (SPLIT_GDT_BASE + entry_index * 8) as usize;
        for byte_offset in 0..8usize {
            bus.ram[dst + byte_offset] = bus.ram[src + byte_offset];
        }
    }
    state.gdt_base = SPLIT_GDT_BASE;
    state.gdt_limit = 8 * 8 - 1;

    // Clear the A-bit on the new (ring-3) CS so the IRET path will attempt
    // the descriptor write that we want to fault.
    let ring3_cs_rights_addr = (SPLIT_GDT_BASE + 4 * 8 + 5) as usize;
    bus.ram[ring3_cs_rights_addr] = 0xFB & !0x01;

    cpu.load_state(&state);

    // 16-bit IRET (0xCF). The 5-word frame returns to ring 3.
    place_at(&mut bus, PG_CODE_BASE, &[0xCF]);

    let ring3_ip: u16 = 0x0100;
    let ring3_sp_low: u16 = 0xFF00;
    let original_esp = cpu.esp();

    write_word_at(&mut bus, PG_STACK_BASE + original_esp, ring3_ip);
    write_word_at(&mut bus, PG_STACK_BASE + original_esp + 2, PG_RING3_CS_SEL);
    write_word_at(&mut bus, PG_STACK_BASE + original_esp + 4, 0x0202);
    write_word_at(&mut bus, PG_STACK_BASE + original_esp + 6, ring3_sp_low);
    write_word_at(&mut bus, PG_STACK_BASE + original_esp + 8, PG_RING3_SS_SEL);

    let pre_cs = cpu.cs();

    // Mark page 0x81 (which now holds the ring-3 descriptors) read-only.
    // With CR0.WP=1, the supervisor A-bit write on the ring-3 CS descriptor
    // at 0x81005 faults; reads of the descriptor itself still succeed.
    let saved_pte = read_dword_at(&bus, PG_PAGE_TABLE_0 + 0x81 * 4);
    write_dword_at(&mut bus, PG_PAGE_TABLE_0 + 0x81 * 4, 0x81000 | PTE_P);

    cpu.step(&mut bus); // IRET -> #PF on the new CS A-bit write
    cpu.step(&mut bus); // HLT in the #PF handler

    assert!(cpu.halted(), "CPU should halt inside the #PF handler");
    assert_eq!(
        cpu.ip(),
        PG_PF_HANDLER_IP as u32 + 1,
        "control must transfer to the #PF handler"
    );

    // The A-bit write runs first and the fault is delivered same-privilege
    // from CPL 0 on the unchanged source stack with a 4-dword frame.
    let handler_sp = cpu.esp();
    assert_eq!(
        handler_sp,
        original_esp.wrapping_sub(16),
        "same-privilege #PF must consume 4 dwords"
    );

    let pushed_cs = read_dword_at(&bus, PG_STACK_BASE + handler_sp + 8);
    assert_eq!(
        pushed_cs & 0xFFFF,
        pre_cs as u32,
        "fault frame CS must still be the source ring-0 CS"
    );

    write_dword_at(&mut bus, PG_PAGE_TABLE_0 + 0x81 * 4, saved_pte);
}

/// LEAVE atomicity: a #PF on the BP pop must leave ESP/EBP at their
/// pre-instruction values. The 386 LEAVE conceptually does
/// `SP := BP; pop BP`. A non-atomic implementation commits the SP update
/// before the pop and leaves SP at BP after the fault, breaking restart.
#[test]
fn paging_fault_leave_16bit_preserves_sp_bp_on_fault() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state_template = setup_paged_protected_mode(&mut bus);
    let mut state = state_template;
    state.set_esp(0x0FF0);
    state.set_ebp(0x4000);
    cpu.load_state(&state);

    // Mark page 4 (linear 0x4000..0x4FFF) as not present. SS:BP points there.
    let bp_pte_index: u32 = 0x4;
    write_dword_at(&mut bus, PG_PAGE_TABLE_0 + bp_pte_index * 4, 0);

    // LEAVE (0xC9)
    place_at(&mut bus, PG_CODE_BASE, &[0xC9]);

    cpu.step(&mut bus); // LEAVE -> #PF
    cpu.step(&mut bus); // HLT in handler

    assert!(cpu.halted(), "handler must HLT");
    assert_eq!(cpu.ip(), PG_PF_HANDLER_IP as u32 + 1);
    assert_eq!(
        cpu.cr2, 0x4000,
        "CR2 must point at the linear address of the faulting BP pop"
    );

    // Same-privilege #PF pushes 4 dwords on the source stack: err, EIP, CS, EFLAGS.
    // So handler_sp == ESP_at_fault - 16. If LEAVE is atomic, ESP_at_fault is
    // the original ESP (0x0FF0). If LEAVE prematurely committed SP := BP,
    // ESP_at_fault would be BP (0x4000).
    let handler_sp = cpu.esp();
    assert_eq!(
        handler_sp,
        0x0FF0_u32.wrapping_sub(16),
        "ESP at fault must be the pre-instruction ESP, not BP"
    );

    assert_eq!(
        cpu.ebp(),
        0x4000,
        "EBP must not have been popped on faulting LEAVE"
    );
}

/// LEAVE r/m32 (with operand-size override 0x66) - same atomicity property
/// as the 16-bit form, but pops a dword instead of a word.
#[test]
fn paging_fault_leave_32bit_preserves_sp_bp_on_fault() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state_template = setup_paged_protected_mode(&mut bus);
    let mut state = state_template;
    state.set_esp(0x0FF0);
    state.set_ebp(0x4000);
    cpu.load_state(&state);

    let bp_pte_index: u32 = 0x4;
    write_dword_at(&mut bus, PG_PAGE_TABLE_0 + bp_pte_index * 4, 0);

    // 0x66 0xC9 - LEAVE with operand-size override (pops 4 bytes into EBP)
    place_at(&mut bus, PG_CODE_BASE, &[0x66, 0xC9]);

    cpu.step(&mut bus); // LEAVE -> #PF
    cpu.step(&mut bus); // HLT in handler

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PG_PF_HANDLER_IP as u32 + 1);
    assert_eq!(cpu.cr2, 0x4000);

    let handler_sp = cpu.esp();
    assert_eq!(
        handler_sp,
        0x0FF0_u32.wrapping_sub(16),
        "ESP at fault must be the pre-instruction ESP, not BP"
    );

    assert_eq!(cpu.ebp(), 0x4000, "EBP must not have been popped");
}

/// Sets up a paged ring-0 state with TOP=0 and ST(0)=Fp80::ONE (a value that
/// converts losslessly to f32/f64, so fpu_check_result leaves the precision
/// flag clear). Marks PTE for linear page 0x14 as not present and sets BX
/// to 0x4000, so DS:[BX] = 0x14000 will #PF on any memory FPU access. The
/// FPU memory stores below all target this address.
fn setup_fpu_fault_state(bus: &mut TestBus) -> cpu::I386State {
    let mut state = setup_paged_protected_mode(bus);
    state.set_ebx(0x4000);
    state.fpu.registers[0] = Fp80::ONE;
    state.fpu.status_word = 0; // TOP=0, no exception bits
    state.fpu.tag_word = 0xFFFC; // reg 0 valid (00), regs 1..7 empty (11)
    state.fpu.control_word = 0x037F; // default; precision = extended
    state
}

/// FSTP DWORD PTR [BX] atomicity: a #PF on the memory write must leave
/// the FPU stack untouched. The buggy implementation calls fpu_pop()
/// unconditionally after fpu_fst_m32 (whose internal write returns Step
/// but whose void signature swallows the failure), so on fault TOP would
/// advance and ST(0)'s tag would flip to empty.
#[test]
fn paging_fault_fstp_m32_preserves_st0_on_fault() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_fpu_fault_state(&mut bus);
    cpu.load_state(&state);

    // PTE 0x14 = not present. DS:BX = 0x10000 + 0x4000 = 0x14000.
    write_dword_at(&mut bus, PG_PAGE_TABLE_0 + 0x14 * 4, 0);

    // FSTP DWORD PTR [BX] - 0xD9 /3 mod=00 rm=111 -> 0xD9 0x1F
    place_at(&mut bus, PG_CODE_BASE, &[0xD9, 0x1F]);

    cpu.step(&mut bus); // FSTP -> #PF
    cpu.step(&mut bus); // HLT in handler

    assert!(cpu.halted());
    assert_eq!(cpu.ip(), PG_PF_HANDLER_IP as u32 + 1);
    assert_eq!(cpu.cr2, 0x14000);

    let top = (cpu.state.fpu.status_word >> 11) & 7;
    assert_eq!(top, 0, "TOP must not have advanced on faulting FSTP");

    let tag_st0 = cpu.state.fpu.tag_word & 0x3;
    assert_eq!(
        tag_st0, 0b00,
        "ST(0) tag must remain valid on faulting FSTP"
    );

    assert_eq!(
        cpu.state.fpu.registers[0],
        Fp80::ONE,
        "ST(0) register slot must be unchanged on faulting FSTP"
    );
}

/// FSTP QWORD PTR [BX] - same property for the 64-bit form (DD /3).
#[test]
fn paging_fault_fstp_m64_preserves_st0_on_fault() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_fpu_fault_state(&mut bus);
    cpu.load_state(&state);

    write_dword_at(&mut bus, PG_PAGE_TABLE_0 + 0x14 * 4, 0);

    // FSTP QWORD PTR [BX] - 0xDD /3 mod=00 rm=111 -> 0xDD 0x1F
    place_at(&mut bus, PG_CODE_BASE, &[0xDD, 0x1F]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.cr2, 0x14000);

    let top = (cpu.state.fpu.status_word >> 11) & 7;
    assert_eq!(top, 0, "TOP must not advance on faulting FSTP m64");
    assert_eq!(cpu.state.fpu.tag_word & 0x3, 0b00);
    assert_eq!(cpu.state.fpu.registers[0], Fp80::ONE);
}

/// FISTP WORD PTR [BX] - DF /3 mod=00 rm=111 -> 0xDF 0x1F.
#[test]
fn paging_fault_fistp_m16_preserves_st0_on_fault() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_fpu_fault_state(&mut bus);
    cpu.load_state(&state);

    write_dword_at(&mut bus, PG_PAGE_TABLE_0 + 0x14 * 4, 0);

    place_at(&mut bus, PG_CODE_BASE, &[0xDF, 0x1F]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.cr2, 0x14000);

    let top = (cpu.state.fpu.status_word >> 11) & 7;
    assert_eq!(top, 0, "TOP must not advance on faulting FISTP m16");
    assert_eq!(cpu.state.fpu.tag_word & 0x3, 0b00);
    assert_eq!(cpu.state.fpu.registers[0], Fp80::ONE);
}

/// FISTP DWORD PTR [BX] - DB /3 mod=00 rm=111 -> 0xDB 0x1F.
#[test]
fn paging_fault_fistp_m32_preserves_st0_on_fault() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_fpu_fault_state(&mut bus);
    cpu.load_state(&state);

    write_dword_at(&mut bus, PG_PAGE_TABLE_0 + 0x14 * 4, 0);

    place_at(&mut bus, PG_CODE_BASE, &[0xDB, 0x1F]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted());
    assert_eq!(cpu.cr2, 0x14000);

    let top = (cpu.state.fpu.status_word >> 11) & 7;
    assert_eq!(top, 0, "TOP must not advance on faulting FISTP m32");
    assert_eq!(cpu.state.fpu.tag_word & 0x3, 0b00);
    assert_eq!(cpu.state.fpu.registers[0], Fp80::ONE);
}

/// Layout for V86 RET FAR atomicity tests:
/// - V86 CS = 0x0500 -> linear 0x5000 (page 0x5, present, contains RET FAR)
/// - V86 SS = 0x0000 -> linear 0..0xFFFF
/// - Page 0 (linear 0..0xFFF) is present; the V86 stack IP slot lives here.
/// - Page 1 (linear 0x1000..0x1FFF) is marked NOT PRESENT; the V86 stack
///   CS slot lives here. The second pop on RET FAR is what should #PF.
/// - On the V86 -> ring 0 #PF transition, the CPU loads SS:ESP from TSS
///   (ESP0=0xFFF0, SS0=PG_SS_SEL, base 0) and pushes a 10-dword frame
///   (GS, FS, DS, ES, SS, ESP, EFLAGS, CS, EIP, errcode = 40 bytes).
const V86_CS_SELECTOR: u16 = 0x0500;
const V86_SS_SELECTOR: u16 = 0x0000;
const V86_CS_BASE: u32 = (V86_CS_SELECTOR as u32) << 4; // 0x5000
const V86_PF_FRAME_BYTES: u32 = 40;

fn setup_v86_ret_far_fault(bus: &mut TestBus, sp: u16) -> cpu::I386State {
    let mut state = setup_paged_protected_mode(bus);
    make_v86(&mut state, V86_CS_SELECTOR, V86_SS_SELECTOR);
    state.set_esp(sp as u32);
    state.ip = 0;
    state.ip_upper = 0;

    // V86 runs at CPL=3, so PDE and the pages directly touched by V86
    // need US=1. We mark the whole identity map US-accessible; the
    // kernel-side IDT/GDT/TSS/handler pages are accessed in supervisor
    // mode (CPL drops to 0 on #PF entry) and don't depend on US.
    let pde = read_dword_at(bus, PG_PAGE_DIR);
    write_dword_at(bus, PG_PAGE_DIR, pde | PTE_US);
    for i in 0..512u32 {
        let pte = read_dword_at(bus, PG_PAGE_TABLE_0 + i * 4);
        if pte & PTE_P != 0 {
            write_dword_at(bus, PG_PAGE_TABLE_0 + i * 4, pte | PTE_US);
        }
    }

    // Mark linear page 0x1000 (PTE index 1) as not present.
    write_dword_at(bus, PG_PAGE_TABLE_0 + 4, 0);
    state
}

fn assert_v86_pf_frame_preserves_sp(
    cpu: &I386,
    bus: &TestBus,
    expected_user_esp: u32,
    expected_user_eip: u32,
) {
    assert_v86_pf_frame_preserves_sp_with_cr2(
        cpu,
        bus,
        expected_user_esp,
        expected_user_eip,
        0x1000,
    );
}

fn assert_v86_pf_frame_preserves_sp_with_cr2(
    cpu: &I386,
    bus: &TestBus,
    expected_user_esp: u32,
    expected_user_eip: u32,
    expected_cr2: u32,
) {
    assert!(cpu.halted(), "ring-0 #PF handler must HLT");
    assert_eq!(cpu.ip(), PG_PF_HANDLER_IP as u32 + 1);
    assert_eq!(cpu.cr2, expected_cr2, "CR2 must point at the faulting slot");

    // Ring-0 ESP after the 10-dword V86 fault frame push.
    let handler_sp = cpu.esp();
    assert_eq!(
        handler_sp,
        0xFFF0u32.wrapping_sub(V86_PF_FRAME_BYTES),
        "V86 #PF must consume 40 bytes on the TSS-supplied stack"
    );

    // V86 frame layout (low to high): errcode, EIP, CS, EFLAGS, ESP, SS, ...
    let saved_eip = read_dword_at(bus, PG_STACK_BASE + handler_sp + 4);
    let saved_esp = read_dword_at(bus, PG_STACK_BASE + handler_sp + 16);

    assert_eq!(
        saved_eip, expected_user_eip,
        "saved EIP must point at the faulting instruction for restart"
    );
    assert_eq!(
        saved_esp, expected_user_esp,
        "saved V86 ESP must be the pre-instruction SP, not the partially-modified SP"
    );
}

/// RET FAR in V86 - second pop (CS slot) faults; SP must remain at its
/// pre-instruction value. Buggy code commits SP += 2 from the first pop
/// before the second pop runs, so the saved-ESP frame slot would read 0x1000.
#[test]
fn paging_fault_ret_far_v86_preserves_sp_on_cs_fault() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_v86_ret_far_fault(&mut bus, 0x0FFE);
    cpu.load_state(&state);

    // RET FAR at V86 CS:IP = 0x0500:0x0000 = linear 0x5000.
    place_at(&mut bus, V86_CS_BASE, &[0xCB]);

    cpu.step(&mut bus); // RET FAR -> #PF on the CS pop
    cpu.step(&mut bus); // HLT in handler

    assert_v86_pf_frame_preserves_sp(&cpu, &bus, 0x0FFE, 0);
}

/// RET FAR imm16 in V86 - same property; SP must not have absorbed the
/// imm operand either.
#[test]
fn paging_fault_ret_far_imm_v86_preserves_sp_on_cs_fault() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_v86_ret_far_fault(&mut bus, 0x0FFE);
    cpu.load_state(&state);

    // RET FAR 0x0004 -- 0xCA imm16
    place_at(&mut bus, V86_CS_BASE, &[0xCA, 0x04, 0x00]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_v86_pf_frame_preserves_sp(&cpu, &bus, 0x0FFE, 0);
}

/// RET FAR with operand-size override 0x66 in V86 - 32-bit IP / CS pops.
/// SP = 0x0FFC means the dword IP slot is at 0x0FFC..0x0FFF (page 0,
/// present) and the dword CS slot is at 0x1000..0x1003 (page 1, not
/// present). The first read succeeds; the second must fault without
/// committing SP.
#[test]
fn paging_fault_ret_far_o32_v86_preserves_esp_on_cs_fault() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_v86_ret_far_fault(&mut bus, 0x0FFC);
    cpu.load_state(&state);

    // 0x66 0xCB -- operand-size override + RET FAR
    place_at(&mut bus, V86_CS_BASE, &[0x66, 0xCB]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_v86_pf_frame_preserves_sp(&cpu, &bus, 0x0FFC, 0);
}

/// IRET in V86 - 16-bit form pops IP, CS, FLAGS. SP=0x0FFE puts the IP
/// slot in page 0 and the CS slot at the start of page 1 (not present).
/// Buggy code commits SP from the first pop before the second runs.
#[test]
fn paging_fault_iret_v86_preserves_sp_on_cs_fault() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_v86_ret_far_fault(&mut bus, 0x0FFE);
    cpu.load_state(&state);

    place_at(&mut bus, V86_CS_BASE, &[0xCF]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_v86_pf_frame_preserves_sp(&cpu, &bus, 0x0FFE, 0);
}

/// IRETD in V86 - 32-bit form pops 3 dwords.
#[test]
fn paging_fault_iretd_v86_preserves_sp_on_cs_fault() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_v86_ret_far_fault(&mut bus, 0x0FFC);
    cpu.load_state(&state);

    place_at(&mut bus, V86_CS_BASE, &[0x66, 0xCF]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_v86_pf_frame_preserves_sp(&cpu, &bus, 0x0FFC, 0);
}

/// Variant of the V86 fault harness with page 2 (linear 0x2000) present
/// and page 1 (linear 0x1000) not present. Stack starts in page 2 and
/// pushes descend into page 1 so push #2+ faults.
fn setup_v86_descending_push_fault(bus: &mut TestBus, sp: u16) -> cpu::I386State {
    let mut state = setup_paged_protected_mode(bus);
    make_v86(&mut state, V86_CS_SELECTOR, V86_SS_SELECTOR);
    state.set_esp(sp as u32);
    state.ip = 0;
    state.ip_upper = 0;

    let pde = read_dword_at(bus, PG_PAGE_DIR);
    write_dword_at(bus, PG_PAGE_DIR, pde | PTE_US);
    for i in 0..512u32 {
        let pte = read_dword_at(bus, PG_PAGE_TABLE_0 + i * 4);
        if pte & PTE_P != 0 {
            write_dword_at(bus, PG_PAGE_TABLE_0 + i * 4, pte | PTE_US);
        }
    }
    // Page 1 (linear 0x1000) is NOT PRESENT.
    write_dword_at(bus, PG_PAGE_TABLE_0 + 4, 0);
    state
}

/// PUSHA in V86 - 8 sequential pushes. SP=0x2002: push #1 (AX) at 0x2000
/// (page 2, OK), push #2 (CX) at 0x1FFE (page 1, FAULT).
#[test]
fn paging_fault_pusha_v86_preserves_sp_on_fault() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_v86_descending_push_fault(&mut bus, 0x2002);
    state.set_eax(0xAAAA);
    state.set_ecx(0xCCCC);
    cpu.load_state(&state);

    place_at(&mut bus, V86_CS_BASE, &[0x60]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_v86_pf_frame_preserves_sp_with_cr2(&cpu, &bus, 0x2002, 0, 0x1FFE);
}

/// PUSHAD in V86 - 8 sequential dword pushes. SP=0x2004: push #1 (EAX)
/// at 0x2000 (page 2, OK), push #2 (ECX) at 0x1FFC (page 1, FAULT).
#[test]
fn paging_fault_pushad_v86_preserves_sp_on_fault() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_v86_descending_push_fault(&mut bus, 0x2004);
    state.set_eax(0xAAAAAAAA);
    state.set_ecx(0xCCCCCCCC);
    cpu.load_state(&state);

    place_at(&mut bus, V86_CS_BASE, &[0x66, 0x60]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_v86_pf_frame_preserves_sp_with_cr2(&cpu, &bus, 0x2004, 0, 0x1FFC);
}

/// V86 -> ring0 INT dispatch must be restartable: if a push onto the
/// TSS-supplied kernel stack faults, the #PF handler must observe the
/// original V86 state (VM=1, V86 SS:ESP, V86 segment registers, EIP
/// pointing at the INT instruction) so that IRET back re-executes the
/// INT cleanly. Layout:
/// - TSS ESP0 = 0x4020 (kernel stack just inside page 4, present).
/// - Page 3 (linear 0x3000..0x3FFF) NOT PRESENT.
/// - V86 INT pushes 9 dwords descending from 0x401C to 0x3FFC. The 9th
///   push at 0x3FFC is in page 3 (FAULT).
/// - The subsequent #PF dispatch is itself inter-priv (CPL was still 3
///   since the original INT had not yet committed CS). #PF pushes 10
///   dwords (from_vm86=true if VM was preserved) totaling 40 bytes,
///   descending from 0x401C to 0x3FF8 - all in page 4.
#[test]
fn paging_fault_v86_int_dispatch_preserves_state_on_kernel_stack_fault() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_paged_protected_mode(&mut bus);
    write_dword_at(&mut bus, PG_TSS_BASE + 4, 0x4020);

    make_v86(&mut state, V86_CS_SELECTOR, V86_SS_SELECTOR);
    let v86_esp_orig: u32 = 0x0F00;
    state.set_esp(v86_esp_orig);
    state.ip = 0;
    state.ip_upper = 0;

    let pde = read_dword_at(&bus, PG_PAGE_DIR);
    write_dword_at(&mut bus, PG_PAGE_DIR, pde | PTE_US);
    for i in 0..512u32 {
        let pte = read_dword_at(&bus, PG_PAGE_TABLE_0 + i * 4);
        if pte & PTE_P != 0 {
            write_dword_at(&mut bus, PG_PAGE_TABLE_0 + i * 4, pte | PTE_US);
        }
    }
    write_dword_at(&mut bus, PG_PAGE_TABLE_0 + 3 * 4, 0);

    // INT 0x80 -> 386 gate so the V86 dispatch pushes 9 dwords (36 bytes,
    // crossing into page 3 and faulting).
    write_idt_gate(
        &mut bus,
        PG_IDT_BASE,
        0x80,
        PG_PF_HANDLER_IP as u32,
        PG_CS_SEL,
        14,
        3,
    );
    // #PF gate -> 286 gate so its V86 dispatch pushes 10 words (20 bytes),
    // which fits entirely in page 4. This lets the #PF actually deliver
    // rather than triple-faulting on the same absent page, so the test
    // can observe the preserved V86 state in the saved frame.
    write_idt_gate(
        &mut bus,
        PG_IDT_BASE,
        14,
        PG_PF_HANDLER_IP as u32,
        PG_CS_SEL,
        6,
        0,
    );

    cpu.load_state(&state);
    place_at(&mut bus, V86_CS_BASE, &[0xCD, 0x80]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(cpu.halted(), "#PF handler must HLT");
    assert_eq!(
        cpu.cr2, 0x3FFC,
        "CR2 must point at the faulting kernel stack push"
    );

    // #PF gate is a 286 gate -> V86 inter-priv pushes 10 words (20 bytes).
    let handler_sp = cpu.esp();
    assert_eq!(
        handler_sp,
        0x4020u32.wrapping_sub(20),
        "V86 #PF (286 gate) must consume 20 bytes on the TSS stack"
    );

    // 286-gate V86 inter-priv frame (low to high): errcode, IP, CS,
    // FLAGS, SP, SS, ES, DS, FS, GS -- each 2 bytes.
    let frame_base = PG_STACK_BASE + handler_sp;
    let saved_ip = read_word_at(&bus, frame_base + 2);
    let saved_cs = read_word_at(&bus, frame_base + 4);
    let saved_flags = read_word_at(&bus, frame_base + 6);
    let saved_sp = read_word_at(&bus, frame_base + 8);
    let saved_ss = read_word_at(&bus, frame_base + 10);

    assert_eq!(saved_ip, 0, "saved IP must point at INT 0x80 for restart");
    assert_eq!(
        saved_cs, V86_CS_SELECTOR,
        "saved CS must be V86 CS, not kernel CS"
    );
    // VM is necessarily 0 inside the ring-0 handler; instead, verify
    // that the saved frame structure proves VM was 1 at fault time -
    // a from_vm86 inter-priv push from the secondary #PF dispatch
    // includes the V86 segment registers (GS, FS, DS, ES). If the
    // primary INT dispatch had pre-cleared VM, the #PF dispatch would
    // have taken the non-vm86 path with a shorter frame, and we'd have
    // failed the handler_sp = ESP0 - 20 assertion above.
    let _ = saved_flags;
    let saved_es = read_word_at(&bus, frame_base + 12);
    let saved_ds = read_word_at(&bus, frame_base + 14);
    let saved_fs = read_word_at(&bus, frame_base + 16);
    let saved_gs = read_word_at(&bus, frame_base + 18);
    // V86 ES/DS/FS/GS were left at 0 by make_v86, so all four should be 0.
    assert_eq!(saved_es, 0, "saved V86 ES must be preserved");
    assert_eq!(saved_ds, 0, "saved V86 DS must be preserved");
    assert_eq!(saved_fs, 0, "saved V86 FS must be preserved");
    assert_eq!(saved_gs, 0, "saved V86 GS must be preserved");
    assert_eq!(
        saved_sp as u32, v86_esp_orig,
        "saved SP must be the V86 SP, not the kernel TSS ESP"
    );
    assert_eq!(
        saved_ss, V86_SS_SELECTOR,
        "saved SS must be V86 SS, not the kernel SS"
    );
}

/// CALL FAR ptr16:16 in V86 - sequential push(CS); push(IP). SP=0x2002:
/// push CS at 0x2000 (page 2, OK), push IP at 0x1FFE (page 1, FAULT).
/// Buggy code commits SP after the first push, so the saved-ESP frame
/// slot would read 0x2000 instead of 0x2002.
#[test]
fn paging_fault_call_far_v86_preserves_sp_on_ip_fault() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_v86_descending_push_fault(&mut bus, 0x2002);
    cpu.load_state(&state);

    // CALL FAR 0x1234:0x5678 - 0x9A imm16 imm16
    place_at(&mut bus, V86_CS_BASE, &[0x9A, 0x78, 0x56, 0x34, 0x12]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_v86_pf_frame_preserves_sp_with_cr2(&cpu, &bus, 0x2002, 0, 0x1FFE);
}

/// CALL FAR ptr16:32 in V86 (0x66 prefix) - dword pushes. SP=0x2004:
/// push CS dword at 0x2000-0x2003 (page 2, OK), push EIP dword at
/// 0x1FFC-0x1FFF (page 1, FAULT).
#[test]
fn paging_fault_call_far_o32_v86_preserves_esp_on_eip_fault() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_v86_descending_push_fault(&mut bus, 0x2004);
    cpu.load_state(&state);

    // 0x66 0x9A imm32 imm16 - CALL FAR ptr16:32
    place_at(
        &mut bus,
        V86_CS_BASE,
        &[0x66, 0x9A, 0x78, 0x56, 0x34, 0x12, 0x00, 0x10],
    );

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_v86_pf_frame_preserves_sp_with_cr2(&cpu, &bus, 0x2004, 0, 0x1FFC);
}

/// CALL m16:16 far indirect (group FF /3) in V86 - sequential push(CS);
/// push(IP). Same fault property as CALL FAR ptr16:16. The far pointer
/// itself is placed in a mapped data page.
#[test]
fn paging_fault_call_m_far_v86_preserves_sp_on_ip_fault() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_v86_descending_push_fault(&mut bus, 0x2002);
    cpu.load_state(&state);

    // Place a m16:16 far pointer at DS:[0x6000] (page 6, mapped).
    // Layout: IP word at +0, CS word at +2.
    write_word_at(&mut bus, 0x6000, 0x5678);
    write_word_at(&mut bus, 0x6002, 0x1234);

    // CALL FAR WORD PTR [0x6000] - 0xFF /3 with modrm 0x1E disp16.
    place_at(&mut bus, V86_CS_BASE, &[0xFF, 0x1E, 0x00, 0x60]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_v86_pf_frame_preserves_sp_with_cr2(&cpu, &bus, 0x2002, 0, 0x1FFE);
}

/// ENTER 0, 2 in V86 with the source frame pointer landing in an absent
/// page. The instruction pushes BP, then walks SS:[BP-2] for the chain.
/// Buggy code commits the first push (SP -= 2, write BP at SP-2) before
/// the chain read fires. Atomic code reads the chain into a buffer
/// before any commit, so a #PF leaves SP and the stack word at the BP
/// slot untouched. SP=0x4002 (page 4, mapped), BP=0x1004 (page 1, absent).
/// Chain read at SS:[0x1002] faults; we observe CR2=0x1002, saved-ESP=
/// 0x4002, the BP push slot at linear 0x4000 still zeroed, and the V86
/// BP register unchanged.
#[test]
fn paging_fault_enter_level_2_chain_read_preserves_state() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let mut state = setup_v86_descending_push_fault(&mut bus, 0x4002);
    state.set_ebp(0x1004);
    cpu.load_state(&state);

    // ENTER 0x0000, 0x02 - 0xC8 imm16 imm8.
    place_at(&mut bus, V86_CS_BASE, &[0xC8, 0x00, 0x00, 0x02]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_v86_pf_frame_preserves_sp_with_cr2(&cpu, &bus, 0x4002, 0, 0x1002);

    // Push slot for BP at SS:[0x4000] must remain at its initial zero
    // value - the chain read fault must abort before the BP push commits.
    let bp_slot = read_word_at(&bus, 0x4000);
    assert_eq!(
        bp_slot, 0,
        "BP push slot must not be written when ENTER faults on chain read"
    );
    assert_eq!(
        cpu.ebp(),
        0x1004,
        "EBP must remain at its V86 value when ENTER faults"
    );
}

/// POP word ptr [absent_page] in V86 - pop reads from stack (mapped),
/// then put writes to memory (absent). Buggy code commits SP from the
/// pop before attempting the write, so the saved-ESP frame slot would
/// read 0x2002. Atomic code rolls SP back when the put faults.
#[test]
fn paging_fault_pop_rm_mem_v86_preserves_sp() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_v86_descending_push_fault(&mut bus, 0x2000);
    cpu.load_state(&state);

    // Seed the top-of-stack with a known word so pop reads valid data.
    write_word_at(&mut bus, 0x2000, 0xBEEF);

    // POP WORD PTR [0x1000] - 0x8F /0 with modrm 0x06 disp16. Memory
    // target lands in absent page 1.
    place_at(&mut bus, V86_CS_BASE, &[0x8F, 0x06, 0x00, 0x10]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_v86_pf_frame_preserves_sp_with_cr2(&cpu, &bus, 0x2000, 0, 0x1000);
}

/// POP DWORD PTR [absent_page] in V86 (0x66 prefix) - same property
/// with dword pop. SP=0x2000 so the stack read fits in page 2 and the
/// memory write hits page 1.
#[test]
fn paging_fault_pop_rm_o32_mem_v86_preserves_esp() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let state = setup_v86_descending_push_fault(&mut bus, 0x2000);
    cpu.load_state(&state);

    write_dword_at(&mut bus, 0x2000, 0xDEAD_BEEF);

    // 0x66 0x8F 0x06 disp16 - POP DWORD PTR [disp16] (16-bit address mode).
    place_at(&mut bus, V86_CS_BASE, &[0x66, 0x8F, 0x06, 0x00, 0x10]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_v86_pf_frame_preserves_sp_with_cr2(&cpu, &bus, 0x2000, 0, 0x1000);
}
