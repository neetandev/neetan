use common::{Bus as _, Cpu as _};
use cpu::{I8086, I8086State};

const RAM_SIZE: usize = 1024 * 1024;
const ADDRESS_MASK: u32 = 0x000F_FFFF;

struct TestBus {
    ram: Vec<u8>,
    irq_pending: bool,
    irq_vector: u8,
    current_cycle: u64,
}

impl TestBus {
    fn new() -> Self {
        Self {
            ram: vec![0u8; RAM_SIZE],
            irq_pending: false,
            irq_vector: 0,
            current_cycle: 0,
        }
    }

    fn raise_irq(&mut self, vector: u8) {
        self.irq_pending = true;
        self.irq_vector = vector;
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
        self.current_cycle
    }

    fn set_current_cycle(&mut self, cycle: u64) {
        self.current_cycle = cycle;
    }
}

fn place_code(bus: &mut TestBus, cs: u16, ip: u16, code: &[u8]) {
    let base = (cs as u32) << 4;
    for (index, &byte) in code.iter().enumerate() {
        bus.write_byte(base + ip as u32 + index as u32, byte);
    }
}

fn set_interrupt_vector(bus: &mut TestBus, vector: u8, target_segment: u16, target_offset: u16) {
    let base = u32::from(vector) * 4;
    bus.write_byte(base, (target_offset & 0x00FF) as u8);
    bus.write_byte(base + 1, (target_offset >> 8) as u8);
    bus.write_byte(base + 2, (target_segment & 0x00FF) as u8);
    bus.write_byte(base + 3, (target_segment >> 8) as u8);
}

fn setup_state(cs: u16, ip: u16) -> I8086State {
    let mut state = I8086State::default();
    state.set_cs(cs);
    state.set_ss(0x2000);
    state.set_ds(0x3000);
    state.set_es(0x4000);
    state.set_sp(0x0100);
    state.ip = ip;
    state.initialize_cold_frontend();
    state
}

#[test]
fn i8086_f1_is_a_lock_prefix_alias() {
    let mut cpu = I8086::new();
    let mut bus = TestBus::new();

    place_code(&mut bus, 0x1000, 0x0000, &[0xF1, 0x90, 0xF4]);

    let state = setup_state(0x1000, 0x0000);
    cpu.load_state(&state);

    cpu.step(&mut bus);

    assert_eq!(cpu.ip(), 0x0002, "F1 should prefix the following opcode");
    assert!(!cpu.halted(), "F1 NOP should not halt the CPU");
}

#[test]
fn i8086_rep_handles_multiple_segment_prefixes() {
    let mut cpu = I8086::new();
    let mut bus = TestBus::new();

    place_code(&mut bus, 0x1000, 0x0000, &[0xF3, 0x26, 0x2E, 0xA4]);
    bus.write_byte((0x1000u32 << 4) + 0x0010, 0xAA);
    bus.write_byte((0x4000u32 << 4) + 0x0010, 0x55);

    let mut state = setup_state(0x1000, 0x0000);
    state.set_cx(1);
    state.set_si(0x0010);
    state.set_di(0x0020);
    cpu.load_state(&state);

    cpu.step(&mut bus);

    assert_eq!(
        bus.ram[((0x4000u32 << 4) + 0x0020) as usize],
        0xAA,
        "the last segment override must win after REP"
    );
    assert_eq!(cpu.cx(), 0, "REP MOVSB should consume CX");
}

#[test]
fn i8086_fe_call_register_uses_widened_byte_operand() {
    let mut cpu = I8086::new();
    let mut bus = TestBus::new();

    place_code(&mut bus, 0x1000, 0x0000, &[0xFE, 0xD7]);
    bus.write_byte((0x2000u32 << 4) + 0x00FF, 0x7A);

    let mut state = setup_state(0x1000, 0x0000);
    state.set_bx(0xE2D8);
    cpu.load_state(&state);

    cpu.step(&mut bus);

    assert_eq!(cpu.ip(), 0xD8E2, "CALL BH should use BX with swapped bytes");
    assert_eq!(cpu.sp(), 0x00FE, "CALL should push a return address");
    let stack_address = ((0x2000u32 << 4) + 0x00FE) as usize;
    assert_eq!(bus.ram[stack_address], 0x02);
    assert_eq!(bus.ram[stack_address + 1], 0x7A);
}

#[test]
fn i8086_fe_push_byte_writes_one_stack_byte() {
    let mut cpu = I8086::new();
    let mut bus = TestBus::new();

    place_code(&mut bus, 0x1000, 0x0000, &[0xFE, 0x36, 0x00, 0x10]);
    bus.write_byte((0x3000u32 << 4) + 0x1000, 0x23);
    bus.write_byte((0x2000u32 << 4) + 0x00FF, 0x7A);

    let state = setup_state(0x1000, 0x0000);
    cpu.load_state(&state);

    cpu.step(&mut bus);

    assert_eq!(cpu.sp(), 0x00FE, "PUSH byte should still decrement SP by 2");
    let stack_address = ((0x2000u32 << 4) + 0x00FE) as usize;
    assert_eq!(bus.ram[stack_address], 0x23, "low byte should be written");
    assert_eq!(
        bus.ram[stack_address + 1],
        0x7A,
        "high byte should remain untouched"
    );
}

#[test]
fn i8086_fe_callf_register_uses_prefetch_dependent_ip_handoff() {
    let mut cpu = I8086::new();
    let mut bus = TestBus::new();

    place_code(&mut bus, 0x4177, 0x1234, &[0xFE, 0xD9]);
    bus.write_byte((0x3000u32 << 4) + 0x0004, 0xB5);
    bus.write_byte((0x2000u32 << 4) + 0x00FD, 0x7A);
    bus.write_byte((0x2000u32 << 4) + 0x00FF, 0x6C);

    let state = setup_state(0x4177, 0x1234);
    cpu.load_state(&state);

    cpu.step(&mut bus);

    assert_eq!(cpu.cs(), 0xFFB5);
    assert_eq!(cpu.ip(), 0x1234);
    assert_eq!(cpu.sp(), 0x00FC);
    let stack_address = ((0x2000u32 << 4) + 0x00FC) as usize;
    assert_eq!(bus.ram[stack_address], 0x36);
    assert_eq!(bus.ram[stack_address + 1], 0x7A);
    assert_eq!(bus.ram[stack_address + 2], 0x77);
    assert_eq!(bus.ram[stack_address + 3], 0x6C);
}

#[test]
fn i8086_ff_jmpf_register_uses_prefetch_dependent_ip_handoff() {
    let mut cpu = I8086::new();
    let mut bus = TestBus::new();

    place_code(&mut bus, 0x1000, 0x1234, &[0xFF, 0xE8]);
    bus.write_byte((0x3000u32 << 4) + 0x0004, 0x34);
    bus.write_byte((0x3000u32 << 4) + 0x0005, 0x12);

    let state = setup_state(0x1000, 0x1234);
    cpu.load_state(&state);
    cpu.install_prefetch_queue(&[0xFF, 0xE8, 0x90, 0x90]);

    cpu.step(&mut bus);

    assert_eq!(cpu.cs(), 0x1234);
    assert_eq!(cpu.ip(), 0x1230);
    assert_eq!(cpu.sp(), 0x0100);
}

#[test]
fn i8086_interrupt_entry_preserves_return_ip_when_flushing_queue() {
    const IRQ_VECTOR: u8 = 0x20;

    let mut cpu = I8086::new();
    let mut bus = TestBus::new();

    place_code(&mut bus, 0x1000, 0x0000, &[0xFB, 0x90, 0xF4]);
    place_code(&mut bus, 0x2000, 0x0000, &[0xCF]);
    set_interrupt_vector(&mut bus, IRQ_VECTOR, 0x2000, 0x0000);
    bus.raise_irq(IRQ_VECTOR);

    let state = setup_state(0x1000, 0x0000);
    cpu.load_state(&state);

    let _ = cpu.run_for(1_000, &mut bus);

    assert!(cpu.halted(), "CPU should return from IRQ and execute HLT");
    assert_eq!(cpu.cs(), 0x1000);
    assert_eq!(cpu.ip(), 0x0003);
    assert_eq!(cpu.sp(), 0x0100);
}

#[test]
fn i8086_pop_rm_overlap_matches_undefined_corpus_final_state() {
    let mut cpu = I8086::new();
    let mut bus = TestBus::new();

    place_code(&mut bus, 0x09FC, 0xF8C0, &[0x36, 0x8F, 0x79, 0xB2]);

    let mut state = I8086State::default();
    state.set_ax(0xF382);
    state.set_bx(0x0A25);
    state.set_cx(0x3DB2);
    state.set_dx(0x76CB);
    state.set_cs(0x09FC);
    state.set_ss(0x300B);
    state.set_ds(0x62EF);
    state.set_es(0x432E);
    state.set_sp(0x59DF);
    state.set_bp(0x6C9F);
    state.set_si(0xF282);
    state.set_di(0x5009);
    state.ip = 0xF8C0;
    state.set_compressed_flags(0xF082);
    state.initialize_cold_frontend();

    bus.write_byte(0x35A8F, 0x56);
    bus.write_byte(0x35A90, 0x56);

    cpu.load_state(&state);

    cpu.step(&mut bus);

    assert_eq!(cpu.sp(), 0x59E1);
    assert_eq!(cpu.ip(), 0xF8C4);
    assert_eq!(bus.ram[0x35A90], 0x56);
    assert_eq!(bus.ram[0x35A91], 0x56);
}
