//! Software-visible CPU model identity: reset EDX component/revision values and
//! the EFLAGS.AC (bit 18) writability split that programs use to tell a 386DX
//! from a 486DX.

use common::{Bus as _, Cpu as _};
use cpu::{CPU_MODEL_386, CPU_MODEL_486, I386, I386State};

const RAM_SIZE: usize = 1 << 20;
const ADDRESS_MASK: u32 = 0x000F_FFFF;

const EFLAGS_ALIGNMENT_CHECK: u32 = 0x0004_0000;

struct TestBus {
    ram: Vec<u8>,
}

impl TestBus {
    fn new() -> Self {
        Self {
            ram: vec![0u8; RAM_SIZE],
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

fn place_code(bus: &mut TestBus, cs: u16, ip: u16, code: &[u8]) {
    let base = (cs as u32) << 4;
    for (i, &byte) in code.iter().enumerate() {
        bus.write_byte(base + ip as u32 + i as u32, byte);
    }
}

fn setup_real_mode_state(cs: u16, ip: u16) -> I386State {
    let mut state = I386State::default();
    state.set_cs(cs);
    state.set_eip(ip as u32);
    state.set_ss(0x3000);
    state.set_esp(0x1000);
    state
}

#[test]
fn reset_edx_reports_386dx_revision() {
    let cpu = I386::<{ CPU_MODEL_386 }>::new();
    assert_eq!(
        cpu.edx(),
        0x0000_0308,
        "386DX reset: DH=3 component identifier, DL=8 D-step revision"
    );
}

#[test]
fn reset_edx_reports_486dx2_revision() {
    let cpu = I386::<{ CPU_MODEL_486 }>::new();
    assert_eq!(
        cpu.edx(),
        0x0000_0435,
        "486 reset: DH=4 component identifier, i486DX2-66 revision"
    );
}

/// The canonical 386-vs-486 detection: toggle EFLAGS.AC through PUSHFD/POPFD
/// and observe whether the bit sticks. Returns `original ^ readback` restricted
/// to the AC bit: nonzero means the toggle was swallowed (386), zero means the
/// toggle stuck (486).
fn run_ac_toggle_detection<const CPU_MODEL: u8>() -> u32 {
    let mut cpu = I386::<CPU_MODEL>::new();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let ip: u16 = 0x0000;
    place_code(
        &mut bus,
        cs,
        ip,
        &[
            0x66, 0x9C, // PUSHFD
            0x66, 0x58, // POP EAX
            0x66, 0x89, 0xC3, // MOV EBX, EAX
            0x66, 0x35, 0x00, 0x00, 0x04, 0x00, // XOR EAX, 0x00040000
            0x66, 0x50, // PUSH EAX
            0x66, 0x9D, // POPFD
            0x66, 0x9C, // PUSHFD
            0x66, 0x58, // POP EAX
        ],
    );

    let state = setup_real_mode_state(cs, ip);
    cpu.load_state(&state);
    for _ in 0..8 {
        cpu.step(&mut bus);
    }

    (cpu.eax() ^ cpu.ebx()) & EFLAGS_ALIGNMENT_CHECK
}

#[test]
fn pushfd_popfd_ac_detection_reports_386() {
    assert_eq!(
        run_ac_toggle_detection::<{ CPU_MODEL_386 }>(),
        0,
        "the AC toggle must be swallowed on the 386"
    );
}

#[test]
fn pushfd_popfd_ac_detection_reports_486() {
    assert_eq!(
        run_ac_toggle_detection::<{ CPU_MODEL_486 }>(),
        EFLAGS_ALIGNMENT_CHECK,
        "the AC toggle must stick on the 486"
    );
}

/// Runs a real-mode IRETD whose stacked EFLAGS image has AC set, then reads
/// EFLAGS back through PUSHFD. Returns the AC bit of the readback.
fn run_real_mode_iretd_ac<const CPU_MODEL: u8>() -> u32 {
    let mut cpu = I386::<CPU_MODEL>::new();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let ip: u16 = 0x0000;
    place_code(
        &mut bus,
        cs,
        ip,
        &[
            0x66, 0x68, 0x02, 0x00, 0x04, 0x00, // PUSH 0x00040002 (EFLAGS, AC set)
            0x66, 0x68, 0x00, 0x10, 0x00, 0x00, // PUSH 0x00001000 (CS)
            0x66, 0x68, 0x00, 0x02, 0x00, 0x00, // PUSH 0x00000200 (EIP)
            0x66, 0xCF, // IRETD
        ],
    );
    place_code(
        &mut bus,
        cs,
        0x0200,
        &[
            0x66, 0x9C, // PUSHFD
            0x66, 0x58, // POP EAX
        ],
    );

    let state = setup_real_mode_state(cs, ip);
    cpu.load_state(&state);
    for _ in 0..6 {
        cpu.step(&mut bus);
    }

    cpu.eax() & EFLAGS_ALIGNMENT_CHECK
}

#[test]
fn real_mode_iretd_ac_stays_clear_on_386() {
    assert_eq!(run_real_mode_iretd_ac::<{ CPU_MODEL_386 }>(), 0);
}

#[test]
fn real_mode_iretd_ac_loads_on_486() {
    assert_eq!(
        run_real_mode_iretd_ac::<{ CPU_MODEL_486 }>(),
        EFLAGS_ALIGNMENT_CHECK
    );
}

const PM_GDT_BASE: u32 = 0x80000;
const PM_IDT_BASE: u32 = 0x90000;
const PM_CODE_BASE: u32 = 0x50000;
const PM_DATA_BASE: u32 = 0x10000;
const PM_STACK_BASE: u32 = 0x00000;
const PM_TSS_OUT_BASE: u32 = 0x70000;
const PM_TSS_IN_BASE: u32 = 0x71000;

const PM_CS_SEL: u16 = 0x0008;
const PM_DS_SEL: u16 = 0x0010;
const PM_SS_SEL: u16 = 0x0018;
const PM_TSS_OUT_SEL: u16 = 0x0020;
const PM_TSS_IN_SEL: u16 = 0x0028;

const TSS_386_LIMIT: u16 = 103;

fn write_gdt_entry16(bus: &mut TestBus, entry_index: u16, base: u32, limit: u16, rights: u8) {
    let addr = (PM_GDT_BASE + (entry_index as u32) * 8) as usize;
    bus.ram[addr] = limit as u8;
    bus.ram[addr + 1] = (limit >> 8) as u8;
    bus.ram[addr + 2] = base as u8;
    bus.ram[addr + 3] = (base >> 8) as u8;
    bus.ram[addr + 4] = (base >> 16) as u8;
    bus.ram[addr + 5] = rights;
    bus.ram[addr + 6] = 0;
    bus.ram[addr + 7] = (base >> 24) as u8;
}

fn write_dword_at(bus: &mut TestBus, addr: u32, value: u32) {
    bus.ram[addr as usize] = value as u8;
    bus.ram[addr as usize + 1] = (value >> 8) as u8;
    bus.ram[addr as usize + 2] = (value >> 16) as u8;
    bus.ram[addr as usize + 3] = (value >> 24) as u8;
}

fn setup_protected_mode(bus: &mut TestBus) -> I386State {
    write_gdt_entry16(bus, 0, 0, 0, 0);
    write_gdt_entry16(bus, 1, PM_CODE_BASE, 0xFFFF, 0x9B);
    write_gdt_entry16(bus, 2, PM_DATA_BASE, 0xFFFF, 0x93);
    write_gdt_entry16(bus, 3, PM_STACK_BASE, 0xFFFF, 0x93);

    let mut state = I386State {
        cr0: 0x0001,
        ip: 0x0000,
        ..Default::default()
    };
    state.set_esp(0xFFF0);

    state.set_cs(PM_CS_SEL);
    state.set_ds(PM_DS_SEL);
    state.set_ss(PM_SS_SEL);
    state.set_es(PM_DS_SEL);

    state.seg_bases[cpu::SegReg32::CS as usize] = PM_CODE_BASE;
    state.seg_bases[cpu::SegReg32::DS as usize] = PM_DATA_BASE;
    state.seg_bases[cpu::SegReg32::SS as usize] = PM_STACK_BASE;
    state.seg_bases[cpu::SegReg32::ES as usize] = PM_DATA_BASE;

    state.seg_limits = [0xFFFF; 6];

    state.seg_rights[cpu::SegReg32::CS as usize] = 0x9B;
    state.seg_rights[cpu::SegReg32::DS as usize] = 0x93;
    state.seg_rights[cpu::SegReg32::SS as usize] = 0x93;
    state.seg_rights[cpu::SegReg32::ES as usize] = 0x93;

    state.seg_valid = [true, true, true, true, false, false];

    state.gdt_base = PM_GDT_BASE;
    state.gdt_limit = 6 * 8 - 1;
    state.idt_base = PM_IDT_BASE;
    state.idt_limit = 256 * 8 - 1;

    state
}

/// Runs a same-CPL protected-mode IRETD at CPL 0 whose stacked EFLAGS image has
/// AC set, then reads EFLAGS back through PUSHFD. Returns the AC bit.
fn run_protected_mode_iretd_ac<const CPU_MODEL: u8>() -> u32 {
    let mut cpu = I386::<CPU_MODEL>::new();
    let mut bus = TestBus::new();
    let state = setup_protected_mode(&mut bus);

    place_code(
        &mut bus,
        (PM_CODE_BASE >> 4) as u16,
        0x0000,
        &[
            0x66, 0x68, 0x02, 0x00, 0x04, 0x00, // PUSH 0x00040002 (EFLAGS, AC set)
            0x66, 0x68, 0x08, 0x00, 0x00, 0x00, // PUSH 0x00000008 (CS, RPL 0)
            0x66, 0x68, 0x00, 0x02, 0x00, 0x00, // PUSH 0x00000200 (EIP)
            0x66, 0xCF, // IRETD
        ],
    );
    place_code(
        &mut bus,
        (PM_CODE_BASE >> 4) as u16,
        0x0200,
        &[
            0x66, 0x9C, // PUSHFD
            0x66, 0x58, // POP EAX
        ],
    );

    cpu.load_state(&state);
    for _ in 0..6 {
        cpu.step(&mut bus);
    }

    cpu.eax() & EFLAGS_ALIGNMENT_CHECK
}

#[test]
fn protected_mode_iretd_ac_stays_clear_on_386() {
    assert_eq!(run_protected_mode_iretd_ac::<{ CPU_MODEL_386 }>(), 0);
}

#[test]
fn protected_mode_iretd_ac_loads_on_486() {
    assert_eq!(
        run_protected_mode_iretd_ac::<{ CPU_MODEL_486 }>(),
        EFLAGS_ALIGNMENT_CHECK
    );
}

/// Switches to a new task via a far JMP to a 386 TSS whose saved EFLAGS image
/// has AC set, then reads EFLAGS back through PUSHFD in the incoming task.
/// Returns the AC bit.
fn run_task_switch_ac<const CPU_MODEL: u8>() -> u32 {
    let mut cpu = I386::<CPU_MODEL>::new();
    let mut bus = TestBus::new();
    let mut state = setup_protected_mode(&mut bus);

    write_gdt_entry16(&mut bus, 4, PM_TSS_OUT_BASE, TSS_386_LIMIT, 0x8B);
    write_gdt_entry16(&mut bus, 5, PM_TSS_IN_BASE, TSS_386_LIMIT, 0x89);

    // Incoming 386 TSS: EFLAGS with AC set, entry at CS:0x0200.
    write_dword_at(&mut bus, PM_TSS_IN_BASE + 0x20, 0x0000_0200); // EIP
    write_dword_at(&mut bus, PM_TSS_IN_BASE + 0x24, 0x0004_0002); // EFLAGS
    write_dword_at(&mut bus, PM_TSS_IN_BASE + 0x38, 0x0000_FF00); // ESP
    write_dword_at(&mut bus, PM_TSS_IN_BASE + 0x48, PM_DS_SEL as u32); // ES
    write_dword_at(&mut bus, PM_TSS_IN_BASE + 0x4C, PM_CS_SEL as u32); // CS
    write_dword_at(&mut bus, PM_TSS_IN_BASE + 0x50, PM_SS_SEL as u32); // SS
    write_dword_at(&mut bus, PM_TSS_IN_BASE + 0x54, PM_DS_SEL as u32); // DS
    write_dword_at(&mut bus, PM_TSS_IN_BASE + 0x60, 0); // LDT

    place_code(
        &mut bus,
        (PM_CODE_BASE >> 4) as u16,
        0x0000,
        &[
            // JMP FAR PM_TSS_IN_SEL:0x0000 (task switch; the offset is ignored)
            0xEA,
            0x00,
            0x00,
            PM_TSS_IN_SEL as u8,
            (PM_TSS_IN_SEL >> 8) as u8,
        ],
    );
    place_code(
        &mut bus,
        (PM_CODE_BASE >> 4) as u16,
        0x0200,
        &[
            0x66, 0x9C, // PUSHFD
            0x66, 0x58, // POP EAX
        ],
    );

    state.tr = PM_TSS_OUT_SEL;
    state.tr_base = PM_TSS_OUT_BASE;
    state.tr_limit = TSS_386_LIMIT as u32;
    state.tr_rights = 0x8B;

    cpu.load_state(&state);
    for _ in 0..3 {
        cpu.step(&mut bus);
    }

    cpu.eax() & EFLAGS_ALIGNMENT_CHECK
}

#[test]
fn task_switch_ac_stays_clear_on_386() {
    assert_eq!(run_task_switch_ac::<{ CPU_MODEL_386 }>(), 0);
}

#[test]
fn task_switch_ac_loads_on_486() {
    assert_eq!(
        run_task_switch_ac::<{ CPU_MODEL_486 }>(),
        EFLAGS_ALIGNMENT_CHECK
    );
}
