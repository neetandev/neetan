//! IN / OUT / INS / OUTS privilege gating tests.
//!
//! 80486 PRM Chapter 8 (I/O) defines two distinct gates for port access:
//!
//! - Protected mode (non-VM86): if CPL <= IOPL, the access is unconditionally
//!   allowed. Otherwise the I/O Permission Bitmap inside the active TSS is
//!   consulted: a 0 bit allows the port, a 1 bit denies (-> #GP(0)). If the
//!   bitmap offset stored at TSS+0x66 places the bitmap byte beyond the TSS
//!   segment limit, all I/O is denied.
//! - Virtual-8086 mode: IOPL is NOT consulted. The bitmap is always consulted.
//!   IOPL gates CLI/STI/PUSHF/POPF/INTn but never I/O. (CR4.VME is post-486.)
//!
//! Word and dword accesses examine multiple consecutive bits in the bitmap;
//! the access is denied if any bit in the [port..port+size) window is 1.
//! Two consecutive bitmap bytes are read so that accesses that span a byte
//! boundary are evaluated against the full window. The byte at
//! `tr_base + iopb_offset + bitmap_size` is the mandatory sentinel (0xFF):
//! this is what closes off accesses near the end of the bitmap.

use common::Cpu as _;
use cpu::{CPU_MODEL_386_DX, I386, I386State};

use super::setup::{
    ACCESS_DESCRIPTOR_SYSTEM, ACCESS_DPL_RING0, ACCESS_PRESENT, DEFAULT_IO_MAP_BASE_OFFSET,
    GLOBAL_DESCRIPTOR_TABLE_BASE, HANDLER_GENERAL_PROTECTION_IP, INTERRUPT_DESCRIPTOR_TABLE_BASE,
    RING0_CODE_BASE, RING3_CODE_BASE, SELECTOR_RING0_CODE, SELECTOR_RING0_STACK, SHARED_DATA_BASE,
    SYSTEM_TYPE_TSS_286_BUSY, TSS_OFFSET_ESP0, TSS_OFFSET_IO_MAP_BASE_FIELD, TSS_OFFSET_SS0,
    TestBus, install_io_permission_bitmap, install_protected_mode_general_protection_handler,
    make_cpu_386, place_at, promote_to_ring3, setup_protected_mode_with_handlers,
    setup_protected_mode_with_iopb, setup_vm86_with_iopl_and_iopb, write_dword_at,
    write_interrupt_gate_386, write_segment_descriptor_16bit, write_word_at,
};

const IN_AL_IMM: u8 = 0xE4;
const OUT_IMM_AL: u8 = 0xE6;
const IN_AL_DX: u8 = 0xEC;
const IN_AX_DX: u8 = 0xED;
const OUT_DX_AL: u8 = 0xEE;
const OUT_DX_AX: u8 = 0xEF;
const INSB: u8 = 0x6C;
const INSW: u8 = 0x6D;
const OUTSB: u8 = 0x6E;
const OUTSW: u8 = 0x6F;
const REP_PREFIX: u8 = 0xF3;
const OPERAND_SIZE_PREFIX: u8 = 0x66;
const HLT_OPCODE: u8 = 0xF4;

fn install_vm86_general_protection_handler(bus: &mut TestBus) {
    write_interrupt_gate_386(
        bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        13,
        HANDLER_GENERAL_PROTECTION_IP as u32,
        SELECTOR_RING0_CODE,
        0,
    );
    bus.ram[(RING0_CODE_BASE + HANDLER_GENERAL_PROTECTION_IP as u32) as usize] = HLT_OPCODE;
}

fn promote_to_cpl3_iopl(state: &mut I386State, iopl: u8) {
    promote_to_ring3(state);
    state.set_esp(0xFFE0);
    state.flags.iopl = iopl & 3;
}

fn assert_general_protection_taken(cpu: &I386<{ CPU_MODEL_386_DX }>) {
    assert!(cpu.halted(), "expected #GP handler to halt");
    assert_eq!(
        cpu.ip(),
        HANDLER_GENERAL_PROTECTION_IP as u32 + 1,
        "expected EIP to be parked at HLT inside the #GP handler"
    );
}

fn assert_no_fault(cpu: &I386<{ CPU_MODEL_386_DX }>) {
    assert!(
        !cpu.halted(),
        "expected the I/O instruction to complete without halting"
    );
}

// Group A: CPL <= IOPL. The bitmap must NOT be consulted; even a deny-all
// bitmap (or a missing bitmap with iopb_offset > tr_limit) is irrelevant.

#[test]
fn in_al_imm_pm_cpl0_iopl0_allows() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();
    bus.io_read_default = 0xA5;

    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[]);
    state.flags.iopl = 0;
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[IN_AL_IMM, 0x42]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(cpu.eax() & 0xFF, 0xA5);
}

#[test]
fn in_al_dx_pm_cpl0_iopl3_allows() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();
    bus.io_read_default = 0x5A;

    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[]);
    state.flags.iopl = 3;
    state.set_edx(0x0123);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[IN_AL_DX]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(cpu.eax() & 0xFF, 0x5A);
}

#[test]
fn out_imm_al_pm_cpl0_iopl0_allows() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[]);
    state.flags.iopl = 0;
    state.set_eax(0x99);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[OUT_IMM_AL, 0x80]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(bus.io_write_log, vec![(0x80u16, 0x99u8)]);
}

#[test]
fn out_dx_al_pm_cpl0_iopl3_allows() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[]);
    state.flags.iopl = 3;
    state.set_edx(0x0070);
    state.set_eax(0xCD);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[OUT_DX_AL]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(bus.io_write_log, vec![(0x0070u16, 0xCDu8)]);
}

#[test]
fn in_ax_dx_pm_cpl0_iopl0_word_allows() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();
    bus.io_read_default = 0x77;

    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[]);
    state.flags.iopl = 0;
    state.set_edx(0x0040);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[IN_AX_DX]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(cpu.eax() & 0xFFFF, 0x7777);
}

#[test]
fn out_dx_ax_pm_cpl0_iopl0_word_allows() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[]);
    state.flags.iopl = 0;
    state.set_edx(0x0050);
    state.set_eax(0xBEEF);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[OUT_DX_AX]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(
        bus.io_write_log,
        vec![(0x0050u16, 0xEFu8), (0x0051u16, 0xBEu8)]
    );
}

#[test]
fn in_eax_dx_pm_cpl0_iopl0_dword_allows() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();
    bus.io_read_default = 0x11;

    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[]);
    state.flags.iopl = 0;
    state.set_edx(0x0060);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[OPERAND_SIZE_PREFIX, IN_AX_DX]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(cpu.eax(), 0x1111_1111);
}

#[test]
fn out_dx_eax_pm_cpl0_iopl3_dword_allows() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[]);
    state.flags.iopl = 3;
    state.set_edx(0x0080);
    state.set_eax(0xDEAD_BEEF);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[OPERAND_SIZE_PREFIX, OUT_DX_AX]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(
        bus.io_write_log,
        vec![
            (0x0080u16, 0xEFu8),
            (0x0081u16, 0xBEu8),
            (0x0082u16, 0xADu8),
            (0x0083u16, 0xDEu8),
        ]
    );
}

#[test]
fn insb_pm_cpl0_iopl0_allows() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();
    bus.io_read_default = 0xA1;

    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[]);
    state.flags.iopl = 0;
    state.set_edx(0x0090);
    state.set_edi(0x0100);
    state.flags.df = false;
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[INSB]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(bus.ram[(SHARED_DATA_BASE + 0x0100) as usize], 0xA1);
    assert_eq!(cpu.edi() & 0xFFFF, 0x0101);
}

#[test]
fn outsb_pm_cpl0_iopl0_allows() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[]);
    state.flags.iopl = 0;
    state.set_edx(0x00A0);
    state.set_esi(0x0200);
    state.flags.df = false;
    cpu.load_state(&state);
    bus.ram[(SHARED_DATA_BASE + 0x0200) as usize] = 0xC3;

    place_at(&mut bus, RING0_CODE_BASE, &[OUTSB]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(bus.io_write_log, vec![(0x00A0u16, 0xC3u8)]);
    assert_eq!(cpu.esi() & 0xFFFF, 0x0201);
}

// Group B: CPL > IOPL with bitmap allowing the port (bit = 0) -> success.

#[test]
fn in_al_dx_pm_cpl3_iopl0_port_allowed_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();
    bus.io_read_default = 0x42;

    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[0x10]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0010);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[IN_AL_DX]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(cpu.eax() & 0xFF, 0x42);
}

#[test]
fn in_al_imm_pm_cpl3_iopl0_port_allowed_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();
    bus.io_read_default = 0x77;

    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[0x20]);
    promote_to_cpl3_iopl(&mut state, 0);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[IN_AL_IMM, 0x20]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(cpu.eax() & 0xFF, 0x77);
}

#[test]
fn out_dx_al_pm_cpl3_iopl0_port_allowed_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[0x30]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0030);
    state.set_eax(0xCC);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[OUT_DX_AL]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(bus.io_write_log, vec![(0x0030u16, 0xCCu8)]);
}

#[test]
fn out_imm_al_pm_cpl3_iopl0_port_allowed_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[0x18]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_eax(0x99);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[OUT_IMM_AL, 0x18]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(bus.io_write_log, vec![(0x0018u16, 0x99u8)]);
}

#[test]
fn in_ax_dx_pm_cpl3_iopl0_two_ports_allowed_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();
    bus.io_read_default = 0xAA;

    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[0x20, 0x21]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0020);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[IN_AX_DX]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(cpu.eax() & 0xFFFF, 0xAAAA);
}

#[test]
fn out_dx_ax_pm_cpl3_iopl0_two_ports_allowed_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_iopb(&mut bus, 16, &[0x40, 0x41]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0040);
    state.set_eax(0xCAFE);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[OUT_DX_AX]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(
        bus.io_write_log,
        vec![(0x0040u16, 0xFEu8), (0x0041u16, 0xCAu8)]
    );
}

#[test]
fn in_eax_dx_pm_cpl3_iopl0_four_ports_allowed_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();
    bus.io_read_default = 0x55;

    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[0x10, 0x11, 0x12, 0x13]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0010);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[OPERAND_SIZE_PREFIX, IN_AX_DX]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(cpu.eax(), 0x5555_5555);
}

#[test]
fn out_eax_dx_pm_cpl3_iopl0_four_ports_allowed_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_iopb(&mut bus, 16, &[0x70, 0x71, 0x72, 0x73]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0070);
    state.set_eax(0x1234_5678);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[OPERAND_SIZE_PREFIX, OUT_DX_AX]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(
        bus.io_write_log,
        vec![
            (0x0070u16, 0x78u8),
            (0x0071u16, 0x56u8),
            (0x0072u16, 0x34u8),
            (0x0073u16, 0x12u8),
        ]
    );
}

#[test]
fn insb_pm_cpl3_iopl0_port_allowed_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();
    bus.io_read_default = 0xB0;

    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[0x18]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0018);
    state.set_edi(0x0300);
    state.flags.df = false;
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[INSB]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(bus.ram[(SHARED_DATA_BASE + 0x0300) as usize], 0xB0);
    assert_eq!(cpu.edi() & 0xFFFF, 0x0301);
}

#[test]
fn outsb_pm_cpl3_iopl0_port_allowed_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[0x28]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0028);
    state.set_esi(0x0400);
    state.flags.df = false;
    cpu.load_state(&state);
    bus.ram[(SHARED_DATA_BASE + 0x0400) as usize] = 0x55;

    place_at(&mut bus, RING3_CODE_BASE, &[OUTSB]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(bus.io_write_log, vec![(0x0028u16, 0x55u8)]);
    assert_eq!(cpu.esi() & 0xFFFF, 0x0401);
}

#[test]
fn insw_pm_cpl3_iopl0_two_ports_allowed_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();
    bus.io_read_default = 0xC0;

    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[0x38, 0x39]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0038);
    state.set_edi(0x0500);
    state.flags.df = false;
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[INSW]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(bus.ram[(SHARED_DATA_BASE + 0x0500) as usize], 0xC0);
    assert_eq!(bus.ram[(SHARED_DATA_BASE + 0x0501) as usize], 0xC0);
    assert_eq!(cpu.edi() & 0xFFFF, 0x0502);
}

#[test]
fn outsw_pm_cpl3_iopl0_two_ports_allowed_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_iopb(&mut bus, 16, &[0x48, 0x49]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0048);
    state.set_esi(0x0600);
    state.flags.df = false;
    cpu.load_state(&state);
    write_word_at(&mut bus, SHARED_DATA_BASE + 0x0600, 0xABCD);

    place_at(&mut bus, RING3_CODE_BASE, &[OUTSW]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(
        bus.io_write_log,
        vec![(0x0048u16, 0xCDu8), (0x0049u16, 0xABu8)]
    );
    assert_eq!(cpu.esi() & 0xFFFF, 0x0602);
}

#[test]
fn insd_pm_cpl3_iopl0_four_ports_allowed_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();
    bus.io_read_default = 0x33;

    let mut state = setup_protected_mode_with_iopb(&mut bus, 16, &[0x60, 0x61, 0x62, 0x63]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0060);
    state.set_edi(0x0700);
    state.flags.df = false;
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[OPERAND_SIZE_PREFIX, INSW]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(bus.ram[(SHARED_DATA_BASE + 0x0700) as usize], 0x33);
    assert_eq!(bus.ram[(SHARED_DATA_BASE + 0x0701) as usize], 0x33);
    assert_eq!(bus.ram[(SHARED_DATA_BASE + 0x0702) as usize], 0x33);
    assert_eq!(bus.ram[(SHARED_DATA_BASE + 0x0703) as usize], 0x33);
    assert_eq!(cpu.edi() & 0xFFFF, 0x0704);
}

#[test]
fn outsd_pm_cpl3_iopl0_four_ports_allowed_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_iopb(&mut bus, 32, &[0x80, 0x81, 0x82, 0x83]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0080);
    state.set_esi(0x0800);
    state.flags.df = false;
    cpu.load_state(&state);
    write_dword_at(&mut bus, SHARED_DATA_BASE + 0x0800, 0xDEAD_BEEF);

    place_at(&mut bus, RING3_CODE_BASE, &[OPERAND_SIZE_PREFIX, OUTSW]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(
        bus.io_write_log,
        vec![
            (0x0080u16, 0xEFu8),
            (0x0081u16, 0xBEu8),
            (0x0082u16, 0xADu8),
            (0x0083u16, 0xDEu8),
        ]
    );
    assert_eq!(cpu.esi() & 0xFFFF, 0x0804);
}

// Group C: CPL > IOPL with bitmap denying the port (bit = 1) -> #GP(0).

#[test]
fn in_al_dx_pm_cpl3_iopl0_port_denied_raises_gp() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0010);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[IN_AL_DX]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_taken(&cpu);
    assert!(bus.io_write_log.is_empty());
}

#[test]
fn in_al_imm_pm_cpl3_iopl0_port_denied_raises_gp() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[]);
    promote_to_cpl3_iopl(&mut state, 0);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[IN_AL_IMM, 0x20]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_taken(&cpu);
}

#[test]
fn out_dx_al_pm_cpl3_iopl0_port_denied_raises_gp() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0030);
    state.set_eax(0xCC);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[OUT_DX_AL]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_taken(&cpu);
    assert!(bus.io_write_log.is_empty());
}

#[test]
fn out_imm_al_pm_cpl3_iopl0_port_denied_raises_gp() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_eax(0x66);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[OUT_IMM_AL, 0x40]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_taken(&cpu);
    assert!(bus.io_write_log.is_empty());
}

#[test]
fn in_ax_dx_pm_cpl3_iopl0_one_port_denied_raises_gp() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    // Allow only the low half of the word access. The high port (0x0021)
    // is denied, so the word access must fault.
    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[0x20]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0020);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[IN_AX_DX]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_taken(&cpu);
}

#[test]
fn out_dx_ax_pm_cpl3_iopl0_high_port_denied_raises_gp() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[0x40]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0040);
    state.set_eax(0x1234);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[OUT_DX_AX]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_taken(&cpu);
    assert!(bus.io_write_log.is_empty());
}

#[test]
fn in_eax_dx_pm_cpl3_iopl0_high_byte_denied_raises_gp() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    // Allow only ports 0x10..=0x12; port 0x13 (the high byte of the dword)
    // is still denied.
    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[0x10, 0x11, 0x12]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0010);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[OPERAND_SIZE_PREFIX, IN_AX_DX]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_taken(&cpu);
}

#[test]
fn out_eax_dx_pm_cpl3_iopl0_low_byte_denied_raises_gp() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[0x21, 0x22, 0x23]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0020);
    state.set_eax(0xDEAD_BEEF);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[OPERAND_SIZE_PREFIX, OUT_DX_AX]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_taken(&cpu);
    assert!(bus.io_write_log.is_empty());
}

#[test]
fn insb_pm_cpl3_iopl0_port_denied_raises_gp() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0018);
    state.set_edi(0x0300);
    state.flags.df = false;
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[INSB]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_taken(&cpu);
    // EDI must be unchanged because the access faulted before the store.
    assert_eq!(cpu.edi() & 0xFFFF, 0x0300);
    assert_eq!(bus.ram[(SHARED_DATA_BASE + 0x0300) as usize], 0);
}

#[test]
fn outsb_pm_cpl3_iopl0_port_denied_raises_gp() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0028);
    state.set_esi(0x0400);
    state.flags.df = false;
    cpu.load_state(&state);
    bus.ram[(SHARED_DATA_BASE + 0x0400) as usize] = 0x55;

    place_at(&mut bus, RING3_CODE_BASE, &[OUTSB]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_taken(&cpu);
    assert!(bus.io_write_log.is_empty());
    assert_eq!(cpu.esi() & 0xFFFF, 0x0400);
}

#[test]
fn insw_pm_cpl3_iopl0_one_port_denied_raises_gp() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[0x38]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0038);
    state.set_edi(0x0500);
    state.flags.df = false;
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[INSW]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_taken(&cpu);
    assert_eq!(cpu.edi() & 0xFFFF, 0x0500);
}

#[test]
fn outsw_pm_cpl3_iopl0_one_port_denied_raises_gp() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[0x49]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0048);
    state.set_esi(0x0600);
    state.flags.df = false;
    cpu.load_state(&state);
    write_word_at(&mut bus, SHARED_DATA_BASE + 0x0600, 0xABCD);

    place_at(&mut bus, RING3_CODE_BASE, &[OUTSW]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_taken(&cpu);
    assert!(bus.io_write_log.is_empty());
    assert_eq!(cpu.esi() & 0xFFFF, 0x0600);
}

// Group D: word/dword accesses spanning a bitmap byte boundary. The CPU
// must read both bytes; the result is denied iff any covered bit is 1.

#[test]
fn in_ax_dx_pm_cpl3_iopl0_spans_byte_boundary_allowed_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();
    bus.io_read_default = 0x12;

    // Word access at port 7 reads bit 7 of bitmap byte 0 plus bit 0 of byte 1.
    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[0x07, 0x08]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0007);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[IN_AX_DX]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(cpu.eax() & 0xFFFF, 0x1212);
}

#[test]
fn in_ax_dx_pm_cpl3_iopl0_spans_byte_boundary_high_byte_denied_raises_gp() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    // Allow port 7 in byte 0, leave byte 1's bit 0 set (port 8 denied).
    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[0x07]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0007);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[IN_AX_DX]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_taken(&cpu);
}

#[test]
fn in_ax_dx_pm_cpl3_iopl0_spans_byte_boundary_low_byte_denied_raises_gp() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    // Allow port 8 in byte 1, leave byte 0's bit 7 set (port 7 denied).
    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[0x08]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0007);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[IN_AX_DX]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_taken(&cpu);
}

#[test]
fn in_eax_dx_pm_cpl3_iopl0_spans_byte_boundary_allowed_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();
    bus.io_read_default = 0x99;

    // Dword access at port 6 covers bits 6,7 of byte 0 and bits 0,1 of byte 1.
    let mut state = setup_protected_mode_with_iopb(&mut bus, 16, &[0x06, 0x07, 0x08, 0x09]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0006);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[OPERAND_SIZE_PREFIX, IN_AX_DX]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(cpu.eax(), 0x9999_9999);
}

#[test]
fn in_eax_dx_pm_cpl3_iopl0_spans_byte_boundary_one_bit_denied_raises_gp() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    // Bit 0 of byte 1 (port 8) intentionally NOT cleared.
    let mut state = setup_protected_mode_with_iopb(&mut bus, 16, &[0x06, 0x07, 0x09]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0006);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[OPERAND_SIZE_PREFIX, IN_AX_DX]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_taken(&cpu);
}

#[test]
fn out_eax_dx_pm_cpl3_iopl0_spans_byte_boundary_one_bit_denied_raises_gp() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    // Bit 6 of byte 0 (port 6) intentionally NOT cleared.
    let mut state = setup_protected_mode_with_iopb(&mut bus, 16, &[0x07, 0x08, 0x09]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0006);
    state.set_eax(0xCAFE_BABE);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[OPERAND_SIZE_PREFIX, OUT_DX_AX]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_taken(&cpu);
    assert!(bus.io_write_log.is_empty());
}

// Group E: IOPB offset > TSS limit. The CPU treats this as deny-all.

#[test]
fn in_al_dx_pm_cpl3_iopl0_iopb_outside_tss_denies() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    write_word_at(
        &mut bus,
        state.tr_base + TSS_OFFSET_IO_MAP_BASE_FIELD,
        0x0100,
    );
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0040);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[IN_AL_DX]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_taken(&cpu);
}

#[test]
fn in_al_imm_pm_cpl3_iopl0_iopb_outside_tss_denies() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    write_word_at(
        &mut bus,
        state.tr_base + TSS_OFFSET_IO_MAP_BASE_FIELD,
        0x0100,
    );
    promote_to_cpl3_iopl(&mut state, 0);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[IN_AL_IMM, 0x40]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_taken(&cpu);
}

#[test]
fn out_dx_ax_pm_cpl3_iopl0_iopb_outside_tss_denies() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    write_word_at(
        &mut bus,
        state.tr_base + TSS_OFFSET_IO_MAP_BASE_FIELD,
        0x0100,
    );
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0050);
    state.set_eax(0xBEEF);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[OUT_DX_AX]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_taken(&cpu);
    assert!(bus.io_write_log.is_empty());
}

#[test]
fn out_eax_dx_pm_cpl3_iopl0_iopb_outside_tss_denies() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    write_word_at(
        &mut bus,
        state.tr_base + TSS_OFFSET_IO_MAP_BASE_FIELD,
        0x0100,
    );
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0080);
    state.set_eax(0x1234_5678);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[OPERAND_SIZE_PREFIX, OUT_DX_AX]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_taken(&cpu);
    assert!(bus.io_write_log.is_empty());
}

#[test]
fn insb_pm_cpl3_iopl0_iopb_outside_tss_denies() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    write_word_at(
        &mut bus,
        state.tr_base + TSS_OFFSET_IO_MAP_BASE_FIELD,
        0x0100,
    );
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0030);
    state.set_edi(0x0900);
    state.flags.df = false;
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[INSB]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_taken(&cpu);
    assert_eq!(cpu.edi() & 0xFFFF, 0x0900);
}

#[test]
fn outsb_pm_cpl3_iopl0_iopb_outside_tss_denies() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    write_word_at(
        &mut bus,
        state.tr_base + TSS_OFFSET_IO_MAP_BASE_FIELD,
        0x0100,
    );
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0030);
    state.set_esi(0x0A00);
    state.flags.df = false;
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[OUTSB]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_taken(&cpu);
    assert!(bus.io_write_log.is_empty());
    assert_eq!(cpu.esi() & 0xFFFF, 0x0A00);
}

// Group F: CPL <= IOPL with a deliberately broken IOPB. The bitmap must
// not be consulted, so I/O still succeeds.

#[test]
fn in_al_dx_pm_cpl0_iopl0_iopb_outside_tss_still_allows() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();
    bus.io_read_default = 0x77;

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    write_word_at(
        &mut bus,
        state.tr_base + TSS_OFFSET_IO_MAP_BASE_FIELD,
        0x0100,
    );
    state.flags.iopl = 0;
    state.set_edx(0x0040);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[IN_AL_DX]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(cpu.eax() & 0xFF, 0x77);
}

#[test]
fn out_dx_ax_pm_cpl3_iopl3_iopb_outside_tss_still_allows() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    write_word_at(
        &mut bus,
        state.tr_base + TSS_OFFSET_IO_MAP_BASE_FIELD,
        0x0100,
    );
    promote_to_cpl3_iopl(&mut state, 3);
    state.set_edx(0x0050);
    state.set_eax(0xBEEF);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[OUT_DX_AX]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(
        bus.io_write_log,
        vec![(0x0050u16, 0xEFu8), (0x0051u16, 0xBEu8)]
    );
}

#[test]
fn insb_pm_cpl0_iopl0_iopb_outside_tss_still_allows() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();
    bus.io_read_default = 0x88;

    let mut state = setup_protected_mode_with_handlers(&mut bus);
    write_word_at(
        &mut bus,
        state.tr_base + TSS_OFFSET_IO_MAP_BASE_FIELD,
        0x0100,
    );
    state.flags.iopl = 0;
    state.set_edx(0x0030);
    state.set_edi(0x0900);
    state.flags.df = false;
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[INSB]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(bus.ram[(SHARED_DATA_BASE + 0x0900) as usize], 0x88);
}

// Group G: VM86 IOPL=3. The IOPL gate is satisfied (VM86 always CPL=3 = IOPL=3
// here) yet the bitmap is still consulted because VM86 ignores IOPL for I/O.

#[test]
fn in_al_dx_vm86_iopl3_port_allowed_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();
    bus.io_read_default = 0xAB;

    let mut state = setup_vm86_with_iopl_and_iopb(&mut bus, 3, 8, &[0x10]);
    state.set_edx(0x0010);
    cpu.load_state(&state);

    place_at(&mut bus, 0x0001_0000, &[IN_AL_DX]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(cpu.eax() & 0xFF, 0xAB);
}

#[test]
fn in_al_dx_vm86_iopl3_port_denied_raises_gp() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_vm86_with_iopl_and_iopb(&mut bus, 3, 8, &[]);
    install_vm86_general_protection_handler(&mut bus);
    state.set_edx(0x0010);
    cpu.load_state(&state);

    place_at(&mut bus, 0x0001_0000, &[IN_AL_DX]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_taken(&cpu);
}

#[test]
fn out_dx_al_vm86_iopl3_port_allowed_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_vm86_with_iopl_and_iopb(&mut bus, 3, 8, &[0x20]);
    state.set_edx(0x0020);
    state.set_eax(0xCD);
    cpu.load_state(&state);

    place_at(&mut bus, 0x0001_0000, &[OUT_DX_AL]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(bus.io_write_log, vec![(0x0020u16, 0xCDu8)]);
}

#[test]
fn out_dx_al_vm86_iopl3_port_denied_raises_gp() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_vm86_with_iopl_and_iopb(&mut bus, 3, 8, &[]);
    install_vm86_general_protection_handler(&mut bus);
    state.set_edx(0x0020);
    state.set_eax(0xCD);
    cpu.load_state(&state);

    place_at(&mut bus, 0x0001_0000, &[OUT_DX_AL]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_taken(&cpu);
    assert!(bus.io_write_log.is_empty());
}

#[test]
fn insb_vm86_iopl3_port_allowed_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();
    bus.io_read_default = 0x42;

    let mut state = setup_vm86_with_iopl_and_iopb(&mut bus, 3, 8, &[0x18]);
    state.set_edx(0x0018);
    state.set_edi(0x0100);
    state.flags.df = false;
    cpu.load_state(&state);

    place_at(&mut bus, 0x0001_0000, &[INSB]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    // ES base in setup_vm86_with_iopl is 0x30000.
    assert_eq!(bus.ram[(0x0003_0000 + 0x0100) as usize], 0x42);
}

#[test]
fn outsb_vm86_iopl3_port_denied_raises_gp() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_vm86_with_iopl_and_iopb(&mut bus, 3, 8, &[]);
    install_vm86_general_protection_handler(&mut bus);
    state.set_edx(0x0028);
    state.set_esi(0x0100);
    state.flags.df = false;
    cpu.load_state(&state);

    place_at(&mut bus, 0x0001_0000, &[OUTSB]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_taken(&cpu);
    assert!(bus.io_write_log.is_empty());
}

// Group H: VM86 IOPL < 3. IOPL is irrelevant for I/O; only the bitmap
// gates the access. Allow ports succeed even at IOPL=0.

#[test]
fn in_al_dx_vm86_iopl0_port_allowed_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();
    bus.io_read_default = 0xCC;

    let mut state = setup_vm86_with_iopl_and_iopb(&mut bus, 0, 8, &[0x10]);
    state.set_edx(0x0010);
    cpu.load_state(&state);

    place_at(&mut bus, 0x0001_0000, &[IN_AL_DX]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(cpu.eax() & 0xFF, 0xCC);
}

#[test]
fn in_al_dx_vm86_iopl0_port_denied_raises_gp() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_vm86_with_iopl_and_iopb(&mut bus, 0, 8, &[]);
    install_vm86_general_protection_handler(&mut bus);
    state.set_edx(0x0010);
    cpu.load_state(&state);

    place_at(&mut bus, 0x0001_0000, &[IN_AL_DX]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_taken(&cpu);
}

#[test]
fn out_dx_al_vm86_iopl1_port_allowed_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_vm86_with_iopl_and_iopb(&mut bus, 1, 8, &[0x20]);
    state.set_edx(0x0020);
    state.set_eax(0xAA);
    cpu.load_state(&state);

    place_at(&mut bus, 0x0001_0000, &[OUT_DX_AL]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(bus.io_write_log, vec![(0x0020u16, 0xAAu8)]);
}

#[test]
fn out_dx_al_vm86_iopl2_port_denied_raises_gp() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_vm86_with_iopl_and_iopb(&mut bus, 2, 8, &[]);
    install_vm86_general_protection_handler(&mut bus);
    state.set_edx(0x0020);
    state.set_eax(0xAA);
    cpu.load_state(&state);

    place_at(&mut bus, 0x0001_0000, &[OUT_DX_AL]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_taken(&cpu);
    assert!(bus.io_write_log.is_empty());
}

#[test]
fn in_ax_dx_vm86_iopl0_two_ports_allowed_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();
    bus.io_read_default = 0xBB;

    let mut state = setup_vm86_with_iopl_and_iopb(&mut bus, 0, 8, &[0x30, 0x31]);
    state.set_edx(0x0030);
    cpu.load_state(&state);

    place_at(&mut bus, 0x0001_0000, &[IN_AX_DX]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(cpu.eax() & 0xFFFF, 0xBBBB);
}

#[test]
fn in_eax_dx_vm86_iopl0_dword_spans_byte_boundary_allowed_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();
    bus.io_read_default = 0xDD;

    let mut state = setup_vm86_with_iopl_and_iopb(&mut bus, 0, 16, &[0x06, 0x07, 0x08, 0x09]);
    state.set_edx(0x0006);
    cpu.load_state(&state);

    place_at(&mut bus, 0x0001_0000, &[OPERAND_SIZE_PREFIX, IN_AX_DX]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(cpu.eax(), 0xDDDD_DDDD);
}

#[test]
fn in_al_imm_vm86_iopl0_port_denied_raises_gp() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let state = setup_vm86_with_iopl_and_iopb(&mut bus, 0, 8, &[]);
    install_vm86_general_protection_handler(&mut bus);
    cpu.load_state(&state);

    place_at(&mut bus, 0x0001_0000, &[IN_AL_IMM, 0x42]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_taken(&cpu);
}

#[test]
fn out_imm_al_vm86_iopl0_port_allowed_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_vm86_with_iopl_and_iopb(&mut bus, 0, 8, &[0x18]);
    state.set_eax(0x66);
    cpu.load_state(&state);

    place_at(&mut bus, 0x0001_0000, &[OUT_IMM_AL, 0x18]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(bus.io_write_log, vec![(0x0018u16, 0x66u8)]);
}

#[test]
fn insb_vm86_iopl0_port_denied_raises_gp() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_vm86_with_iopl_and_iopb(&mut bus, 0, 8, &[]);
    install_vm86_general_protection_handler(&mut bus);
    state.set_edx(0x0010);
    state.set_edi(0x0100);
    state.flags.df = false;
    cpu.load_state(&state);

    place_at(&mut bus, 0x0001_0000, &[INSB]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_taken(&cpu);
}

// Group I: REP-prefixed string I/O. The IOPB check runs once per iteration,
// but because DX (= port) is constant across iterations, either every
// iteration is allowed (no fault, ECX consumed to 0) or the first iteration
// faults (ECX/ESI/EDI rolled back to the start).

#[test]
fn rep_outsb_pm_cpl3_iopl0_port_allowed_completes_all() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[0x28]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0028);
    state.set_esi(0x0400);
    state.set_ecx(4);
    state.flags.df = false;
    cpu.load_state(&state);
    bus.ram[(SHARED_DATA_BASE + 0x0400) as usize] = 0x10;
    bus.ram[(SHARED_DATA_BASE + 0x0401) as usize] = 0x20;
    bus.ram[(SHARED_DATA_BASE + 0x0402) as usize] = 0x30;
    bus.ram[(SHARED_DATA_BASE + 0x0403) as usize] = 0x40;

    place_at(&mut bus, RING3_CODE_BASE, &[REP_PREFIX, OUTSB]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(
        bus.io_write_log,
        vec![
            (0x0028u16, 0x10u8),
            (0x0028u16, 0x20u8),
            (0x0028u16, 0x30u8),
            (0x0028u16, 0x40u8),
        ]
    );
    assert_eq!(cpu.ecx(), 0);
    assert_eq!(cpu.esi() & 0xFFFF, 0x0404);
}

#[test]
fn rep_outsb_pm_cpl3_iopl0_port_denied_first_iteration_faults() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0028);
    state.set_esi(0x0400);
    state.set_ecx(4);
    state.flags.df = false;
    cpu.load_state(&state);
    bus.ram[(SHARED_DATA_BASE + 0x0400) as usize] = 0x55;

    place_at(&mut bus, RING3_CODE_BASE, &[REP_PREFIX, OUTSB]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_taken(&cpu);
    assert!(bus.io_write_log.is_empty());
    // No iteration completed, so ECX/ESI are unchanged.
    assert_eq!(cpu.ecx(), 4);
    assert_eq!(cpu.esi() & 0xFFFF, 0x0400);
}

#[test]
fn rep_insb_pm_cpl3_iopl0_port_allowed_completes_all() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();
    bus.io_read_default = 0x77;

    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[0x18]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0018);
    state.set_edi(0x0500);
    state.set_ecx(3);
    state.flags.df = false;
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[REP_PREFIX, INSB]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(bus.ram[(SHARED_DATA_BASE + 0x0500) as usize], 0x77);
    assert_eq!(bus.ram[(SHARED_DATA_BASE + 0x0501) as usize], 0x77);
    assert_eq!(bus.ram[(SHARED_DATA_BASE + 0x0502) as usize], 0x77);
    assert_eq!(cpu.ecx(), 0);
    assert_eq!(cpu.edi() & 0xFFFF, 0x0503);
}

#[test]
fn rep_insb_pm_cpl3_iopl0_port_denied_first_iteration_faults() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0018);
    state.set_edi(0x0500);
    state.set_ecx(3);
    state.flags.df = false;
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[REP_PREFIX, INSB]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_taken(&cpu);
    assert_eq!(cpu.ecx(), 3);
    assert_eq!(cpu.edi() & 0xFFFF, 0x0500);
    assert_eq!(bus.ram[(SHARED_DATA_BASE + 0x0500) as usize], 0);
}

#[test]
fn rep_outsb_pm_cpl3_iopl0_zero_count_skips_io_check() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    // Even with a deny-all IOPB the I/O check is bypassed for ECX=0
    // because do_rep returns immediately without touching outsb.
    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0028);
    state.set_esi(0x0400);
    state.set_ecx(0);
    state.flags.df = false;
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[REP_PREFIX, OUTSB]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert!(bus.io_write_log.is_empty());
    assert_eq!(cpu.ecx(), 0);
    assert_eq!(cpu.esi() & 0xFFFF, 0x0400);
}

#[test]
fn rep_outsw_pm_cpl3_iopl0_two_ports_allowed_completes_all() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_iopb(&mut bus, 16, &[0x40, 0x41]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0040);
    state.set_esi(0x0600);
    state.set_ecx(2);
    state.flags.df = false;
    cpu.load_state(&state);
    write_word_at(&mut bus, SHARED_DATA_BASE + 0x0600, 0x1234);
    write_word_at(&mut bus, SHARED_DATA_BASE + 0x0602, 0x5678);

    place_at(&mut bus, RING3_CODE_BASE, &[REP_PREFIX, OUTSW]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(
        bus.io_write_log,
        vec![
            (0x0040u16, 0x34u8),
            (0x0041u16, 0x12u8),
            (0x0040u16, 0x78u8),
            (0x0041u16, 0x56u8),
        ]
    );
    assert_eq!(cpu.ecx(), 0);
    assert_eq!(cpu.esi() & 0xFFFF, 0x0604);
}

#[test]
fn rep_outsb_pm_cpl3_iopl0_port_denied_eip_parked_at_prefix() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0028);
    state.set_esi(0x0400);
    state.set_ecx(4);
    state.flags.df = false;
    let original_eip = state.ip;
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[REP_PREFIX, OUTSB]);

    // Step once to take the fault. The IDT push records the saved EIP from
    // prev_ip, which start_rep set to the REP prefix byte.
    cpu.step(&mut bus);

    // The CPL=3 -> CPL=0 #GP fault frame is six dwords pushed on the kernel
    // stack at TSS.SS0:ESP0 (= linear 0xFFF0 with SS base 0 here):
    //   ESP0 - 4  : SS  (old)
    //   ESP0 - 8  : ESP (old)
    //   ESP0 - 12 : EFLAGS
    //   ESP0 - 16 : CS  (old)
    //   ESP0 - 20 : EIP
    //   ESP0 - 24 : error_code  <- new ESP after fault entry
    let saved_eip_address = 0xFFF0u32 - 20;
    let saved_eip = u32::from_le_bytes([
        bus.ram[saved_eip_address as usize],
        bus.ram[saved_eip_address as usize + 1],
        bus.ram[saved_eip_address as usize + 2],
        bus.ram[saved_eip_address as usize + 3],
    ]);
    assert_eq!(saved_eip, original_eip as u32);
}

// Group J: sentinel byte. With a one-byte bitmap, the sentinel sits at
// io_map_base + 1 and is read for word/dword accesses near port 7.

#[test]
fn in_ax_dx_pm_cpl3_iopl0_word_access_at_port_7_reads_sentinel_and_denies() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    // Bitmap is one byte with port 7 cleared. The sentinel byte at offset+1
    // is 0xFF, so its bit 0 (port 8) is set, and a word access at port 7
    // covers bit 7 of byte 0 (allowed) plus bit 0 of the sentinel (denied).
    let mut state = setup_protected_mode_with_iopb(&mut bus, 1, &[0x07]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0007);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[IN_AX_DX]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_taken(&cpu);
}

#[test]
fn in_al_dx_pm_cpl3_iopl0_byte_access_under_sentinel_uses_bitmap_only() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();
    bus.io_read_default = 0x33;

    // A byte access at port 7 only reads bitmap byte 0 even though the
    // sentinel sits in the second IOPB byte read by check_io_privilege.
    let mut state = setup_protected_mode_with_iopb(&mut bus, 1, &[0x07]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0007);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[IN_AL_DX]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(cpu.eax() & 0xFF, 0x33);
}

#[test]
fn in_ax_dx_pm_cpl3_iopl0_word_at_byte_15_pulls_in_sentinel() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    // Bitmap is two bytes covering ports 0..15. Port 15 is allowed, but
    // a word access at port 15 reads bit 7 of byte 1 (allowed) plus bit 0
    // of the sentinel (denied).
    let mut state = setup_protected_mode_with_iopb(&mut bus, 2, &[0x0F]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x000F);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[IN_AX_DX]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_taken(&cpu);
}

// Group K: is_io_port_unrestricted bypass. Tests that a bus-side allow
// override works regardless of bitmap state.

#[test]
fn in_al_dx_pm_cpl3_iopl0_unrestricted_port_bypasses_bitmap() {
    struct UnrestrictedBus {
        inner: TestBus,
        unrestricted_port: u16,
    }

    impl common::Bus for UnrestrictedBus {
        fn read_byte(&mut self, address: u32) -> u8 {
            self.inner.read_byte(address)
        }
        fn write_byte(&mut self, address: u32, value: u8) {
            self.inner.write_byte(address, value);
        }
        fn io_read_byte(&mut self, port: u16) -> u8 {
            self.inner.io_read_byte(port)
        }
        fn io_write_byte(&mut self, port: u16, value: u8) {
            self.inner.io_write_byte(port, value);
        }
        fn is_io_port_unrestricted(&self, port: u16) -> bool {
            port == self.unrestricted_port
        }
        fn has_irq(&self) -> bool {
            self.inner.has_irq()
        }
        fn acknowledge_irq(&mut self) -> u8 {
            self.inner.acknowledge_irq()
        }
        fn has_nmi(&self) -> bool {
            self.inner.has_nmi()
        }
        fn acknowledge_nmi(&mut self) {
            self.inner.acknowledge_nmi();
        }
        fn current_cycle(&self) -> u64 {
            0
        }
        fn set_current_cycle(&mut self, _cycle: u64) {}
    }

    let mut cpu = make_cpu_386();
    let mut bus = UnrestrictedBus {
        inner: TestBus::new(),
        unrestricted_port: 0x0070,
    };
    bus.inner.io_read_default = 0xEE;

    let mut state = setup_protected_mode_with_iopb(&mut bus.inner, 8, &[]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0070);
    cpu.load_state(&state);

    place_at(&mut bus.inner, RING3_CODE_BASE, &[IN_AL_DX]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(cpu.eax() & 0xFF, 0xEE);
}

// Group L: confirm DEFAULT_IO_MAP_BASE_OFFSET / install_io_permission_bitmap
// helpers wire the cached TR limit through to check_io_privilege. A test
// that intentionally narrows tr_limit but leaves a valid IOPB must still
// deny based on the limit, not the bitmap content.

#[test]
fn in_al_dx_pm_cpl3_iopl0_iopb_within_tss_but_byte_index_past_limit_denies() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    // Build a normal one-byte bitmap, then truncate state.tr_limit so the
    // bitmap's byte_index+1 falls outside the limit.
    let mut state = setup_protected_mode_with_iopb(&mut bus, 1, &[0x07]);
    state.tr_limit = (DEFAULT_IO_MAP_BASE_OFFSET as u32) - 1;
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0007);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[IN_AL_DX]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_taken(&cpu);
}

#[test]
fn in_al_dx_pm_cpl3_iopl0_undersized_tss_denies_io() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    // A 386 TSS with limit < 0x67 cannot hold the IOPB pointer at offset
    // 0x66, so the CPU rejects all I/O when CPL > IOPL.
    let mut state = setup_protected_mode_with_handlers(&mut bus);
    state.tr_limit = 0x40;
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0010);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[IN_AL_DX]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_taken(&cpu);
}

// Group M: ensure each IN form correctly reads from the io_read_byte path
// and produces the expected register width when allowed.

#[test]
fn in_ax_dx_pm_cpl0_iopl0_reads_word_value_from_two_byte_reads() {
    struct PortValueBus {
        inner: TestBus,
        port_low: u8,
        port_high: u8,
    }

    impl common::Bus for PortValueBus {
        fn read_byte(&mut self, address: u32) -> u8 {
            self.inner.read_byte(address)
        }
        fn write_byte(&mut self, address: u32, value: u8) {
            self.inner.write_byte(address, value);
        }
        fn io_read_byte(&mut self, port: u16) -> u8 {
            if port == 0x0050 {
                self.port_low
            } else if port == 0x0051 {
                self.port_high
            } else {
                0xFF
            }
        }
        fn io_write_byte(&mut self, port: u16, value: u8) {
            self.inner.io_write_byte(port, value);
        }
        fn is_io_port_unrestricted(&self, _port: u16) -> bool {
            false
        }
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

    let mut cpu = make_cpu_386();
    let mut bus = PortValueBus {
        inner: TestBus::new(),
        port_low: 0x12,
        port_high: 0x34,
    };

    let mut state = setup_protected_mode_with_iopb(&mut bus.inner, 8, &[]);
    state.flags.iopl = 0;
    state.set_edx(0x0050);
    cpu.load_state(&state);

    place_at(&mut bus.inner, RING0_CODE_BASE, &[IN_AX_DX]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(cpu.eax() & 0xFFFF, 0x3412);
}

// Group N: VM86 with #GP handler invocation - verify the saved-frame EIP
// points at the faulting I/O instruction (not advanced).

#[test]
fn in_al_dx_vm86_iopl0_port_denied_handler_sees_faulting_eip() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_vm86_with_iopl_and_iopb(&mut bus, 0, 8, &[]);
    install_vm86_general_protection_handler(&mut bus);
    state.set_edx(0x0010);
    let starting_eip = state.ip as u32;
    cpu.load_state(&state);

    place_at(&mut bus, 0x0001_0000, &[IN_AL_DX]);

    cpu.step(&mut bus);

    // VM86 dispatch pushes a 9-dword frame onto TSS.SS0:ESP0. ESP0 = 0x1000,
    // so EIP sits at 0x1000 - 9*4 + 4*4 = 0x1000 - 20 (5th from the bottom).
    let frame_base_esp = 0x1000u32;
    let stack_base = 0u32; // SELECTOR_RING0_STACK base.
    let saved_eip_address = stack_base + frame_base_esp - 9 * 4;
    let saved_eip = u32::from_le_bytes([
        bus.ram[saved_eip_address as usize],
        bus.ram[saved_eip_address as usize + 1],
        bus.ram[saved_eip_address as usize + 2],
        bus.ram[saved_eip_address as usize + 3],
    ]);
    assert_eq!(saved_eip, starting_eip);
}

// Group O: PM dispatch saved-frame EIP for the #GP at CPL3 - the fault
// frame on the ring-0 stack must hold the original CPL=3 EIP.

#[test]
fn out_dx_al_pm_cpl3_iopl0_port_denied_handler_sees_faulting_eip() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[]);
    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0050);
    state.set_eax(0xCD);
    let starting_eip = state.ip as u32;
    write_dword_at(&mut bus, state.tr_base + TSS_OFFSET_ESP0, 0xFFF0);
    write_word_at(
        &mut bus,
        state.tr_base + TSS_OFFSET_SS0,
        SELECTOR_RING0_STACK,
    );
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[OUT_DX_AL]);

    cpu.step(&mut bus);

    // CPL=3 -> CPL=0 #GP fault frame layout on the kernel stack (TSS.SS0:ESP0,
    // with SS base 0 and ESP0=0xFFF0 in this test):
    //   ESP0 - 4  : SS  (old)
    //   ESP0 - 8  : ESP (old)
    //   ESP0 - 12 : EFLAGS
    //   ESP0 - 16 : CS  (old)
    //   ESP0 - 20 : EIP
    //   ESP0 - 24 : error_code  <- new ESP after fault entry
    let saved_eip_address = 0xFFF0u32 - 20;
    let saved_eip = u32::from_le_bytes([
        bus.ram[saved_eip_address as usize],
        bus.ram[saved_eip_address as usize + 1],
        bus.ram[saved_eip_address as usize + 2],
        bus.ram[saved_eip_address as usize + 3],
    ]);
    assert_eq!(saved_eip, starting_eip);
}

// Group P: a 286 TSS has no I/O permission bitmap; check_io_privilege
// rejects every CPL > IOPL access because (tr_rights & 0x0F) < 9. The fault
// dispatch then needs SS0/SP0 from the 286 layout (offsets 4 and 2), so the
// test rewrites the existing TSS image as a 286-style TSS and points the
// GDT TSS slot at it.

#[test]
fn in_al_dx_pm_cpl3_iopl0_286_tss_denies_io() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_handlers(&mut bus);

    // Replace the standard 386 TSS image with a 286-style TSS layout.
    // 286 TSS: SP0 at offset 2 (word), SS0 at offset 4 (word). Limit = 43.
    for offset in 0..0x68u32 {
        bus.ram[(state.tr_base + offset) as usize] = 0;
    }
    write_word_at(&mut bus, state.tr_base + 2, 0xFFF0);
    write_word_at(&mut bus, state.tr_base + 4, SELECTOR_RING0_STACK);

    let tss_286_busy =
        ACCESS_PRESENT | ACCESS_DPL_RING0 | ACCESS_DESCRIPTOR_SYSTEM | SYSTEM_TYPE_TSS_286_BUSY;
    write_segment_descriptor_16bit(
        &mut bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        state.tr >> 3,
        state.tr_base,
        43,
        tss_286_busy,
    );
    state.tr_rights = tss_286_busy;
    state.tr_limit = 43;

    promote_to_cpl3_iopl(&mut state, 0);
    state.set_edx(0x0010);
    cpu.load_state(&state);

    place_at(&mut bus, RING3_CODE_BASE, &[IN_AL_DX]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_taken(&cpu);
}

// Group Q: IOPL gradient PM cases - cross-check that exactly the cases
// CPL > IOPL trigger bitmap consultation, and that the comparison is `>`
// (not `>=`).

#[test]
fn in_al_dx_pm_cpl_equals_iopl_2_does_not_consult_bitmap() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();
    bus.io_read_default = 0xC1;

    // CPL=2, IOPL=2 -> CPL <= IOPL, allowed regardless of bitmap.
    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[]);
    state.set_cs(SELECTOR_RING0_CODE | 2);
    state.stored_cpl = 2;
    state.flags.iopl = 2;
    state.set_edx(0x0040);
    cpu.load_state(&state);

    place_at(&mut bus, RING0_CODE_BASE, &[IN_AL_DX]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(cpu.eax() & 0xFF, 0xC1);
}

#[test]
fn in_al_dx_pm_cpl_one_above_iopl_consults_bitmap() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_protected_mode_with_iopb(&mut bus, 8, &[]);
    state.set_cs(SELECTOR_RING0_CODE | 2);
    state.stored_cpl = 2;
    state.flags.iopl = 1;
    state.set_edx(0x0040);
    cpu.load_state(&state);
    install_protected_mode_general_protection_handler(&mut bus);

    place_at(&mut bus, RING0_CODE_BASE, &[IN_AL_DX]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_taken(&cpu);
}

// Group R: IOPB layout helper roundtrip. After install_io_permission_bitmap
// runs, the cached tr_limit must precisely cover bitmap+sentinel.

#[test]
fn install_io_permission_bitmap_extends_tr_limit_to_cover_sentinel() {
    let mut bus = TestBus::new();
    let mut state = setup_protected_mode_with_handlers(&mut bus);
    install_io_permission_bitmap(&mut bus, &mut state, DEFAULT_IO_MAP_BASE_OFFSET, 8, &[0x00]);

    // Bitmap is 8 bytes + 1 sentinel; tr_limit = io_map_base + 8 = 0x70.
    assert_eq!(state.tr_limit, DEFAULT_IO_MAP_BASE_OFFSET as u32 + 8);
    assert_eq!(
        bus.ram[(state.tr_base + DEFAULT_IO_MAP_BASE_OFFSET as u32 + 8) as usize],
        0xFF
    );
}

// Group S: VM86 word/dword and IOPL=2 with and without IOPB.

#[test]
fn in_ax_dx_vm86_iopl2_one_port_denied_raises_gp() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_vm86_with_iopl_and_iopb(&mut bus, 2, 8, &[0x30]);
    install_vm86_general_protection_handler(&mut bus);
    state.set_edx(0x0030);
    cpu.load_state(&state);

    place_at(&mut bus, 0x0001_0000, &[IN_AX_DX]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_taken(&cpu);
}

#[test]
fn out_dx_ax_vm86_iopl2_two_ports_allowed_succeeds() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_vm86_with_iopl_and_iopb(&mut bus, 2, 16, &[0x40, 0x41]);
    state.set_edx(0x0040);
    state.set_eax(0xBEEF);
    cpu.load_state(&state);

    place_at(&mut bus, 0x0001_0000, &[OUT_DX_AX]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(
        bus.io_write_log,
        vec![(0x0040u16, 0xEFu8), (0x0041u16, 0xBEu8)]
    );
}

#[test]
fn in_eax_dx_vm86_iopl1_dword_one_byte_denied_raises_gp() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = setup_vm86_with_iopl_and_iopb(&mut bus, 1, 16, &[0x06, 0x07, 0x09]);
    install_vm86_general_protection_handler(&mut bus);
    state.set_edx(0x0006);
    cpu.load_state(&state);

    place_at(&mut bus, 0x0001_0000, &[OPERAND_SIZE_PREFIX, IN_AX_DX]);

    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_general_protection_taken(&cpu);
}

// Group T: real-mode (no PE) - the I/O check is bypassed entirely.

#[test]
fn in_al_dx_real_mode_always_allowed() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();
    bus.io_read_default = 0x21;

    let mut state = super::setup::make_real_mode_state();
    state.set_edx(0x0010);
    cpu.load_state(&state);

    place_at(&mut bus, 0x000F_0000, &[IN_AL_DX]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(cpu.eax() & 0xFF, 0x21);
}

#[test]
fn out_dx_al_real_mode_always_allowed() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();

    let mut state = super::setup::make_real_mode_state();
    state.set_edx(0x0070);
    state.set_eax(0xAA);
    cpu.load_state(&state);

    place_at(&mut bus, 0x000F_0000, &[OUT_DX_AL]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(bus.io_write_log, vec![(0x0070u16, 0xAAu8)]);
}

#[test]
fn insb_real_mode_always_allowed() {
    let mut cpu = make_cpu_386();
    let mut bus = TestBus::new();
    bus.io_read_default = 0x99;

    let mut state = super::setup::make_real_mode_state();
    state.set_edx(0x0030);
    state.set_edi(0x0100);
    state.flags.df = false;
    cpu.load_state(&state);

    place_at(&mut bus, 0x000F_0000, &[INSB]);

    cpu.step(&mut bus);

    assert_no_fault(&cpu);
    assert_eq!(bus.ram[0x0100], 0x99);
}
