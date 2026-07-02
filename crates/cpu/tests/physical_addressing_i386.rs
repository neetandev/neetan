//! Tests for the model-dependent physical address width of the i386/i486
//! core. The 32-bit configuration (FM TOWNS wiring) must fetch code and
//! access data above 16 MB and at the FFFC0000 reset region, while the
//! 24-bit configuration (PC-98 wiring) must keep truncating physical
//! addresses exactly as before.

use cpu::{ADDRESS_WIDTH_24, ADDRESS_WIDTH_32, CPU_MODEL_386, CPU_MODEL_486, I386, SegReg32};

const LOW_RAM_BASE: u32 = 0x0000_0000;
const LOW_RAM_SIZE: usize = 0x0020_0000;
const EXTENDED_RAM_BASE: u32 = 0x0100_0000;
const EXTENDED_RAM_SIZE: usize = 0x0001_0000;
const SYSROM_BASE: u32 = 0xFFFC_0000;
const SYSROM_SIZE: usize = 0x0004_0000;

/// Sparse bus with three memory regions mirroring the FM TOWNS layout:
/// low RAM, extended RAM above 16 MB, and a writable region at the
/// FFFC0000 SYSROM window that covers the FFFFFFF0 reset vector.
struct SparseBus {
    low_ram: Vec<u8>,
    extended_ram: Vec<u8>,
    sysrom: Vec<u8>,
    /// Un-masked physical address of every write reaching the bus.
    write_address_log: Vec<u32>,
}

impl SparseBus {
    fn new() -> Self {
        Self {
            low_ram: vec![0u8; LOW_RAM_SIZE],
            extended_ram: vec![0u8; EXTENDED_RAM_SIZE],
            sysrom: vec![0u8; SYSROM_SIZE],
            write_address_log: Vec::new(),
        }
    }

    fn region_mut(&mut self, address: u32) -> Option<&mut u8> {
        let address = address as usize;
        if address < LOW_RAM_BASE as usize + LOW_RAM_SIZE {
            return Some(&mut self.low_ram[address]);
        }
        if (EXTENDED_RAM_BASE as usize..EXTENDED_RAM_BASE as usize + EXTENDED_RAM_SIZE)
            .contains(&address)
        {
            return Some(&mut self.extended_ram[address - EXTENDED_RAM_BASE as usize]);
        }
        if address >= SYSROM_BASE as usize {
            return Some(&mut self.sysrom[address - SYSROM_BASE as usize]);
        }
        None
    }

    fn place_at(&mut self, address: u32, bytes: &[u8]) {
        for (index, &byte) in bytes.iter().enumerate() {
            *self
                .region_mut(address + index as u32)
                .expect("placement outside mapped test regions") = byte;
        }
    }

    fn byte_at(&mut self, address: u32) -> u8 {
        *self
            .region_mut(address)
            .expect("read outside mapped test regions")
    }

    fn write_dword_at(&mut self, address: u32, value: u32) {
        self.place_at(address, &value.to_le_bytes());
    }
}

impl common::Bus for SparseBus {
    fn read_byte(&mut self, address: u32) -> u8 {
        match self.region_mut(address) {
            Some(byte) => *byte,
            None => 0xFF,
        }
    }

    fn write_byte(&mut self, address: u32, value: u8) {
        self.write_address_log.push(address);
        if let Some(byte) = self.region_mut(address) {
            *byte = value;
        }
    }

    fn io_read_byte(&mut self, _port: u16) -> u8 {
        0xFF
    }

    fn io_write_byte(&mut self, _port: u16, _value: u8) {}

    fn has_irq(&self) -> bool {
        false
    }

    fn acknowledge_irq(&mut self) -> u8 {
        0
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

fn run_until_halt(cpu: &mut impl common::Cpu, bus: &mut SparseBus) {
    for _ in 0..1_000 {
        cpu.run_for(16, bus);
        if cpu.halted() {
            return;
        }
    }
    panic!("CPU did not halt");
}

/// Builds a protected-mode state (no paging) with 16-bit segments whose
/// descriptor caches are preloaded to the given bases. No far transfers
/// happen in the tests, so no GDT is required.
fn protected_mode_state(code_base: u32, data_base: u32, stack_base: u32) -> cpu::I386State {
    let mut state = cpu::I386State {
        cr0: 0x0000_0001,
        ip: 0,
        ..Default::default()
    };

    state.set_esp(0xFFF0);
    state.set_cs(0x0008);
    state.set_ds(0x0010);
    state.set_es(0x0010);
    state.set_ss(0x0018);

    state.seg_bases[SegReg32::CS as usize] = code_base;
    state.seg_bases[SegReg32::DS as usize] = data_base;
    state.seg_bases[SegReg32::ES as usize] = data_base;
    state.seg_bases[SegReg32::SS as usize] = stack_base;

    state.seg_limits = [0xFFFF; 6];
    state.seg_rights[SegReg32::CS as usize] = 0x9B;
    state.seg_rights[SegReg32::DS as usize] = 0x93;
    state.seg_rights[SegReg32::ES as usize] = 0x93;
    state.seg_rights[SegReg32::SS as usize] = 0x93;
    state.seg_valid = [true, true, true, true, false, false];

    state
}

fn reset_vector_lands_at_fffffff0<const CPU_MODEL: u8>() {
    let mut cpu: I386<CPU_MODEL, ADDRESS_WIDTH_32> = I386::new();
    let mut bus = SparseBus::new();

    assert_eq!(cpu.state.cs(), 0xF000);
    assert_eq!(cpu.state.seg_bases[SegReg32::CS as usize], 0xFFFF_0000);
    assert_eq!(cpu.state.eip(), 0xFFF0);

    // MOV AL, 0x5A; HLT at the architectural reset vector.
    bus.place_at(0xFFFF_FFF0, &[0xB0, 0x5A, 0xF4]);

    run_until_halt(&mut cpu, &mut bus);
    assert_eq!(cpu.al(), 0x5A);
}

#[test]
fn reset_vector_lands_at_fffffff0_386() {
    reset_vector_lands_at_fffffff0::<CPU_MODEL_386>();
}

#[test]
fn reset_vector_lands_at_fffffff0_486() {
    reset_vector_lands_at_fffffff0::<CPU_MODEL_486>();
}

/// Models the real boot flow: the first far jump out of the reset region
/// drops the CS base to selector << 4, landing in the low F8000-FFFFF
/// SYSROM shadow.
fn reset_far_jump_drops_cs_base<const CPU_MODEL: u8>() {
    let mut cpu: I386<CPU_MODEL, ADDRESS_WIDTH_32> = I386::new();
    let mut bus = SparseBus::new();

    // JMP FAR F000:8000 at the reset vector.
    bus.place_at(0xFFFF_FFF0, &[0xEA, 0x00, 0x80, 0x00, 0xF0]);
    // MOV AL, 0x42; HLT at linear 000F8000.
    bus.place_at(0x000F_8000, &[0xB0, 0x42, 0xF4]);

    run_until_halt(&mut cpu, &mut bus);
    assert_eq!(cpu.al(), 0x42);
    assert_eq!(cpu.state.cs(), 0xF000);
    assert_eq!(cpu.state.seg_bases[SegReg32::CS as usize], 0x000F_0000);
}

#[test]
fn reset_far_jump_drops_cs_base_386() {
    reset_far_jump_drops_cs_base::<CPU_MODEL_386>();
}

#[test]
fn reset_far_jump_drops_cs_base_486() {
    reset_far_jump_drops_cs_base::<CPU_MODEL_486>();
}

fn protected_mode_fetch_above_16mb<const CPU_MODEL: u8>() {
    let mut cpu: I386<CPU_MODEL, ADDRESS_WIDTH_32> = I386::new();
    let mut bus = SparseBus::new();

    // Code in extended RAM above 16 MB, data in the SYSROM region.
    let state = protected_mode_state(EXTENDED_RAM_BASE, SYSROM_BASE, 0);
    cpu.load_state(&state);

    // MOV AL, 0x99; MOV [0x0010], AL; MOV AH, [0x0020]; HLT
    bus.place_at(SYSROM_BASE + 0x20, &[0x77]);
    bus.place_at(
        EXTENDED_RAM_BASE,
        &[0xB0, 0x99, 0xA2, 0x10, 0x00, 0x8A, 0x26, 0x20, 0x00, 0xF4],
    );

    run_until_halt(&mut cpu, &mut bus);
    assert_eq!(cpu.al(), 0x99);
    assert_eq!(cpu.ah(), 0x77);
    assert_eq!(bus.byte_at(SYSROM_BASE + 0x10), 0x99);
    assert!(bus.write_address_log.contains(&(SYSROM_BASE + 0x10)));
}

#[test]
fn protected_mode_fetch_above_16mb_386() {
    protected_mode_fetch_above_16mb::<CPU_MODEL_386>();
}

#[test]
fn protected_mode_fetch_above_16mb_486() {
    protected_mode_fetch_above_16mb::<CPU_MODEL_486>();
}

const PAGE_DIRECTORY: u32 = 0x000A_0000;
const PAGE_TABLE_0: u32 = 0x000A_1000;
const PAGE_TABLE_1: u32 = 0x000A_2000;
const PAGE_TABLE_2: u32 = 0x000A_3000;
const PTE_PRESENT_WRITABLE: u32 = 0x03;

/// Identity-maps the low 2 MB, maps linear 00400000 to the SYSROM region
/// and linear 00800000 to extended RAM above 16 MB.
fn setup_high_page_tables(bus: &mut SparseBus) {
    bus.write_dword_at(PAGE_DIRECTORY, PAGE_TABLE_0 | PTE_PRESENT_WRITABLE);
    bus.write_dword_at(PAGE_DIRECTORY + 4, PAGE_TABLE_1 | PTE_PRESENT_WRITABLE);
    bus.write_dword_at(PAGE_DIRECTORY + 8, PAGE_TABLE_2 | PTE_PRESENT_WRITABLE);

    for index in 0..512u32 {
        bus.write_dword_at(
            PAGE_TABLE_0 + index * 4,
            (index * 0x1000) | PTE_PRESENT_WRITABLE,
        );
    }
    for index in 0..16u32 {
        bus.write_dword_at(
            PAGE_TABLE_1 + index * 4,
            (SYSROM_BASE + index * 0x1000) | PTE_PRESENT_WRITABLE,
        );
        bus.write_dword_at(
            PAGE_TABLE_2 + index * 4,
            (EXTENDED_RAM_BASE + index * 0x1000) | PTE_PRESENT_WRITABLE,
        );
    }
}

fn paged_fetch_above_16mb<const CPU_MODEL: u8>() {
    let mut cpu: I386<CPU_MODEL, ADDRESS_WIDTH_32> = I386::new();
    let mut bus = SparseBus::new();

    setup_high_page_tables(&mut bus);

    // Code segment based at linear 00400000 (physical SYSROM region),
    // data segment at linear 00800000 (physical extended RAM).
    let mut state = protected_mode_state(0x0040_0000, 0x0080_0000, 0);
    state.cr0 = 0x8000_0001;
    state.cr3 = PAGE_DIRECTORY;
    cpu.load_state(&state);

    // MOV AL, 0x33; MOV [0x0010], AL; MOV AH, [0x0020]; HLT
    bus.place_at(EXTENDED_RAM_BASE + 0x20, &[0x44]);
    bus.place_at(
        SYSROM_BASE,
        &[0xB0, 0x33, 0xA2, 0x10, 0x00, 0x8A, 0x26, 0x20, 0x00, 0xF4],
    );

    run_until_halt(&mut cpu, &mut bus);
    assert_eq!(cpu.al(), 0x33);
    assert_eq!(cpu.ah(), 0x44);
    assert_eq!(bus.byte_at(EXTENDED_RAM_BASE + 0x10), 0x33);
    assert!(bus.write_address_log.contains(&(EXTENDED_RAM_BASE + 0x10)));
}

#[test]
fn paged_fetch_above_16mb_386() {
    paged_fetch_above_16mb::<CPU_MODEL_386>();
}

#[test]
fn paged_fetch_above_16mb_486() {
    paged_fetch_above_16mb::<CPU_MODEL_486>();
}

#[test]
fn legacy_reset_vector_stays_at_ffff0() {
    let cpu: I386<CPU_MODEL_386, ADDRESS_WIDTH_24> = I386::new();
    assert_eq!(cpu.state.cs(), 0xFFFF);
    assert_eq!(cpu.state.seg_bases[SegReg32::CS as usize], 0x000F_FFF0);
    assert_eq!(cpu.state.eip(), 0x0000);

    let cpu: I386<CPU_MODEL_486, ADDRESS_WIDTH_24> = I386::new();
    assert_eq!(cpu.state.cs(), 0xFFFF);
    assert_eq!(cpu.state.seg_bases[SegReg32::CS as usize], 0x000F_FFF0);
    assert_eq!(cpu.state.eip(), 0x0000);
}

#[test]
fn legacy_default_width_is_24_bit() {
    let cpu: I386 = I386::new();
    assert_eq!(cpu.state.cs(), 0xFFFF);
    assert_eq!(cpu.state.seg_bases[SegReg32::CS as usize], 0x000F_FFF0);
}

/// With the 24-bit wiring a non-paged access above 16 MB is truncated to
/// the low 16 MB, exactly like the PC-98 targets relied on so far.
fn legacy_width_masks_to_24_bit<const CPU_MODEL: u8>() {
    let mut cpu: I386<CPU_MODEL, ADDRESS_WIDTH_24> = I386::new();
    let mut bus = SparseBus::new();

    // Code segment base 01050000 truncates to 00050000, data segment
    // base 01060000 truncates to 00060000: both land in low RAM.
    let state = protected_mode_state(0x0105_0000, 0x0106_0000, 0);
    cpu.load_state(&state);

    // MOV AL, 0xAB; MOV [0x0010], AL; HLT
    bus.place_at(0x0005_0000, &[0xB0, 0xAB, 0xA2, 0x10, 0x00, 0xF4]);

    run_until_halt(&mut cpu, &mut bus);
    assert_eq!(cpu.al(), 0xAB);
    assert_eq!(bus.byte_at(0x0006_0010), 0xAB);
    assert!(bus.write_address_log.contains(&0x0006_0010));
}

#[test]
fn legacy_width_masks_to_24_bit_386() {
    legacy_width_masks_to_24_bit::<CPU_MODEL_386>();
}

#[test]
fn legacy_width_masks_to_24_bit_486() {
    legacy_width_masks_to_24_bit::<CPU_MODEL_486>();
}
