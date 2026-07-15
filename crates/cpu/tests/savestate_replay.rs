use common::{Bus, Cpu as _, Cpu6809, CpuZ80};
use cpu::{
    ADDRESS_WIDTH_24, CPU_MODEL_386_DX, CPU_MODEL_386_SX, CPU_MODEL_486_DX, I286, I386, I8086,
    M6809, V30, Z80,
};
use save_state::{RuntimeState, decode_runtime_state, encode_runtime_state};

#[derive(Debug, Clone, PartialEq, Eq)]
enum BusEvent {
    Read(u32, u8),
    Write(u32, u8),
    Opcode(u32, u8),
    Irq,
    Firq,
    Nmi,
}

#[derive(Clone)]
struct TraceBus {
    memory: Vec<u8>,
    events: Vec<BusEvent>,
    current_cycle: u64,
    irq: bool,
    nmi: bool,
    irq_vector: u8,
}

impl TraceBus {
    fn new() -> Self {
        Self {
            memory: vec![0; 0x10_0000],
            events: Vec::new(),
            current_cycle: 0,
            irq: false,
            nmi: false,
            irq_vector: 0xFF,
        }
    }

    fn set_bytes(&mut self, address: u32, bytes: &[u8]) {
        let start = address as usize;
        self.memory[start..start + bytes.len()].copy_from_slice(bytes);
    }

    fn set_big_endian_word(&mut self, address: u32, value: u16) {
        self.set_bytes(address, &value.to_be_bytes());
    }

    fn clear_events(&mut self) {
        self.events.clear();
    }
}

impl Bus for TraceBus {
    fn read_byte(&mut self, address: u32) -> u8 {
        let address = address & 0x000F_FFFF;
        let value = self.memory[address as usize];
        self.events.push(BusEvent::Read(address, value));
        value
    }

    fn write_byte(&mut self, address: u32, value: u8) {
        let address = address & 0x000F_FFFF;
        self.memory[address as usize] = value;
        self.events.push(BusEvent::Write(address, value));
    }

    fn io_read_byte(&mut self, _port: u16) -> u8 {
        0xFF
    }

    fn io_write_byte(&mut self, _port: u16, _value: u8) {}

    fn has_irq(&self) -> bool {
        self.irq
    }

    fn acknowledge_irq(&mut self) -> u8 {
        self.events.push(BusEvent::Irq);
        self.irq = false;
        self.irq_vector
    }

    fn acknowledge_firq(&mut self) {
        self.events.push(BusEvent::Firq);
    }

    fn has_nmi(&self) -> bool {
        self.nmi
    }

    fn acknowledge_nmi(&mut self) {
        self.events.push(BusEvent::Nmi);
        self.nmi = false;
    }

    fn current_cycle(&self) -> u64 {
        self.current_cycle
    }

    fn set_current_cycle(&mut self, cycle: u64) {
        self.current_cycle = cycle;
    }

    fn fetch_opcode_byte(&mut self, address: u32) -> u8 {
        let address = address & 0x000F_FFFF;
        let value = self.memory[address as usize];
        self.events.push(BusEvent::Opcode(address, value));
        value
    }
}

fn codec_round_trip<State: RuntimeState>(state: &State) -> State {
    let encoded = encode_runtime_state(state);
    decode_runtime_state(&encoded, 4096).unwrap()
}

fn replay_z80(source: &Z80, source_bus: &TraceBus, operation: impl Fn(&mut Z80, &mut TraceBus)) {
    let state = codec_round_trip(&source.capture_state());
    let mut restored = Z80::new(source.clock_hz());
    restored.restore_state(state).unwrap();

    let mut uninterrupted = Z80::new(source.clock_hz());
    uninterrupted.load_state(&source.capture_state());
    let mut uninterrupted_bus = source_bus.clone();
    let mut restored_bus = source_bus.clone();
    uninterrupted_bus.clear_events();
    restored_bus.clear_events();

    operation(&mut uninterrupted, &mut uninterrupted_bus);
    operation(&mut restored, &mut restored_bus);

    assert_eq!(restored.capture_state(), uninterrupted.capture_state());
    assert_eq!(restored_bus.events, uninterrupted_bus.events);
    assert_eq!(restored_bus.memory, uninterrupted_bus.memory);
    assert_eq!(restored_bus.current_cycle, uninterrupted_bus.current_cycle);
    assert_eq!(restored_bus.irq, uninterrupted_bus.irq);
    assert_eq!(restored_bus.nmi, uninterrupted_bus.nmi);
}

fn replay_m6809(
    source: &M6809,
    source_bus: &TraceBus,
    operation: impl Fn(&mut M6809, &mut TraceBus),
) {
    let state = codec_round_trip(&source.capture_state());
    let mut restored = M6809::new(source.clock_hz());
    restored.restore_state(state).unwrap();

    let mut uninterrupted = M6809::new(source.clock_hz());
    uninterrupted.load_state(&source.capture_state());
    let mut uninterrupted_bus = source_bus.clone();
    let mut restored_bus = source_bus.clone();
    uninterrupted_bus.clear_events();
    restored_bus.clear_events();

    operation(&mut uninterrupted, &mut uninterrupted_bus);
    operation(&mut restored, &mut restored_bus);

    assert_eq!(restored.capture_state(), uninterrupted.capture_state());
    assert_eq!(restored_bus.events, uninterrupted_bus.events);
    assert_eq!(restored_bus.memory, uninterrupted_bus.memory);
    assert_eq!(restored_bus.current_cycle, uninterrupted_bus.current_cycle);
    assert_eq!(restored_bus.irq, uninterrupted_bus.irq);
    assert_eq!(restored_bus.nmi, uninterrupted_bus.nmi);
}

fn replay_v30(source: &V30, source_bus: &TraceBus, operation: impl Fn(&mut V30, &mut TraceBus)) {
    let state = codec_round_trip(&source.capture_state());
    let mut restored = V30::new();
    restored.restore_state(state).unwrap();

    let mut uninterrupted = V30::new();
    uninterrupted.load_state(&source.capture_state());
    let mut uninterrupted_bus = source_bus.clone();
    let mut restored_bus = source_bus.clone();
    uninterrupted_bus.clear_events();
    restored_bus.clear_events();

    operation(&mut uninterrupted, &mut uninterrupted_bus);
    operation(&mut restored, &mut restored_bus);

    assert_eq!(restored.capture_state(), uninterrupted.capture_state());
    assert_eq!(restored_bus.events, uninterrupted_bus.events);
    assert_eq!(restored_bus.memory, uninterrupted_bus.memory);
    assert_eq!(restored_bus.current_cycle, uninterrupted_bus.current_cycle);
    assert_eq!(restored_bus.irq, uninterrupted_bus.irq);
    assert_eq!(restored_bus.nmi, uninterrupted_bus.nmi);
}

fn replay_i286(source: &I286, source_bus: &TraceBus, operation: impl Fn(&mut I286, &mut TraceBus)) {
    let state = codec_round_trip(&source.capture_state());
    let mut restored = I286::new();
    restored.restore_state(state).unwrap();

    let mut uninterrupted = I286::new();
    uninterrupted.load_state(&source.capture_state());
    let mut uninterrupted_bus = source_bus.clone();
    let mut restored_bus = source_bus.clone();
    uninterrupted_bus.clear_events();
    restored_bus.clear_events();

    operation(&mut uninterrupted, &mut uninterrupted_bus);
    operation(&mut restored, &mut restored_bus);

    assert_eq!(restored.capture_state(), uninterrupted.capture_state());
    assert_eq!(restored_bus.events, uninterrupted_bus.events);
    assert_eq!(restored_bus.memory, uninterrupted_bus.memory);
    assert_eq!(restored_bus.current_cycle, uninterrupted_bus.current_cycle);
    assert_eq!(restored_bus.irq, uninterrupted_bus.irq);
    assert_eq!(restored_bus.nmi, uninterrupted_bus.nmi);
}

fn replay_i8086(
    source: &I8086,
    source_bus: &TraceBus,
    operation: impl Fn(&mut I8086, &mut TraceBus),
) {
    let state = codec_round_trip(&source.capture_state());
    let mut restored = I8086::new();
    restored.restore_state(state).unwrap();

    let mut uninterrupted = I8086::new();
    uninterrupted.load_state(&source.capture_state());
    let mut uninterrupted_bus = source_bus.clone();
    let mut restored_bus = source_bus.clone();
    uninterrupted_bus.clear_events();
    restored_bus.clear_events();

    operation(&mut uninterrupted, &mut uninterrupted_bus);
    operation(&mut restored, &mut restored_bus);

    assert_eq!(restored.capture_state(), uninterrupted.capture_state());
    assert_eq!(restored_bus.events, uninterrupted_bus.events);
    assert_eq!(restored_bus.memory, uninterrupted_bus.memory);
    assert_eq!(restored_bus.current_cycle, uninterrupted_bus.current_cycle);
    assert_eq!(restored_bus.irq, uninterrupted_bus.irq);
    assert_eq!(restored_bus.nmi, uninterrupted_bus.nmi);
}

fn replay_i386<const CPU_MODEL: u8, const ADDRESS_WIDTH: u8>(
    source: &I386<CPU_MODEL, ADDRESS_WIDTH>,
    source_bus: &TraceBus,
    operation: impl Fn(&mut I386<CPU_MODEL, ADDRESS_WIDTH>, &mut TraceBus),
) {
    let state = codec_round_trip(&source.capture_state());
    let mut restored = I386::<CPU_MODEL, ADDRESS_WIDTH>::new();
    restored.restore_state(state).unwrap();

    let mut uninterrupted = I386::<CPU_MODEL, ADDRESS_WIDTH>::new();
    uninterrupted.load_state(&source.capture_state());
    let mut uninterrupted_bus = source_bus.clone();
    let mut restored_bus = source_bus.clone();
    uninterrupted_bus.clear_events();
    restored_bus.clear_events();

    operation(&mut uninterrupted, &mut uninterrupted_bus);
    operation(&mut restored, &mut restored_bus);

    assert_eq!(restored.capture_state(), uninterrupted.capture_state());
    assert_eq!(restored_bus.events, uninterrupted_bus.events);
    assert_eq!(restored_bus.memory, uninterrupted_bus.memory);
    assert_eq!(restored_bus.current_cycle, uninterrupted_bus.current_cycle);
    assert_eq!(restored_bus.irq, uninterrupted_bus.irq);
    assert_eq!(restored_bus.nmi, uninterrupted_bus.nmi);
}

#[test]
fn z80_replays_halt_with_pending_nmi() {
    let mut cpu = Z80::default();
    let mut bus = TraceBus::new();
    cpu.state.pc = 0x1000;
    cpu.state.sp = 0x9000;
    bus.set_bytes(0x1000, &[0x76]);
    bus.set_bytes(0x0066, &[0x00]);

    assert_eq!(cpu.run_for(4, &mut bus), 4);
    assert!(cpu.state.halted);
    bus.nmi = true;

    replay_z80(&cpu, &bus, |replayed, replayed_bus| {
        replayed.run_for(1, replayed_bus);
        assert!(replayed_bus.events.contains(&BusEvent::Nmi));
    });
}

#[test]
fn z80_replays_ei_interrupt_deferral() {
    let mut cpu = Z80::default();
    let mut bus = TraceBus::new();
    cpu.state.pc = 0x1000;
    cpu.state.sp = 0x9000;
    cpu.state.im = 1;
    bus.set_bytes(0x1000, &[0xFB, 0x00, 0x00]);
    bus.set_bytes(0x0038, &[0x00]);
    bus.irq = true;

    cpu.run_for(1, &mut bus);
    assert_eq!(cpu.state.ei, 1);
    assert_eq!(cpu.state.pending_irq, 1);

    replay_z80(&cpu, &bus, |replayed, replayed_bus| {
        replayed.run_for(4, replayed_bus);
        assert_eq!(replayed.state.pc, 0x1002);
        assert!(!replayed_bus.events.contains(&BusEvent::Irq));
        replayed.run_for(1, replayed_bus);
        assert!(replayed_bus.events.contains(&BusEvent::Irq));
    });
}

#[test]
fn z80_replays_repeating_ldir() {
    let mut cpu = Z80::default();
    let mut bus = TraceBus::new();
    cpu.state.pc = 0x1000;
    cpu.state.set_bc(2);
    cpu.state.set_hl(0x2000);
    cpu.state.set_de(0x3000);
    bus.set_bytes(0x1000, &[0xED, 0xB0]);
    bus.set_bytes(0x2000, &[0x41, 0x42]);

    cpu.run_for(1, &mut bus);
    assert_eq!(cpu.state.bc(), 1);
    assert_eq!(cpu.state.pc, 0x1000);

    replay_z80(&cpu, &bus, |replayed, replayed_bus| {
        replayed.run_for(1, replayed_bus);
        assert_eq!(replayed.state.bc(), 0);
        assert_eq!(replayed_bus.memory[0x3001], 0x42);
    });
}

#[test]
fn m6809_replays_split_extended_clear() {
    let mut cpu = M6809::default();
    let mut bus = TraceBus::new();
    cpu.state.pc = 0x1000;
    bus.set_bytes(0x1000, &[0x7F, 0x20, 0x00]);
    bus.set_bytes(0x2000, &[0xA5]);

    assert_eq!(cpu.run_for(100, &mut bus), 4);
    assert_eq!(cpu.state.pending_extended_clear, Some(0x2000));

    replay_m6809(&cpu, &bus, |replayed, replayed_bus| {
        replayed.run_for(3, replayed_bus);
        assert_eq!(replayed.state.pending_extended_clear, None);
        assert_eq!(replayed_bus.memory[0x2000], 0);
    });
}

#[test]
fn m6809_replays_cwai_without_double_stacking() {
    let mut cpu = M6809::default();
    let mut bus = TraceBus::new();
    cpu.state.pc = 0x1000;
    cpu.set_s(0x3000);
    bus.set_bytes(0x1000, &[0x3C, 0xBF]);
    bus.set_big_endian_word(0xFFF6, 0x2345);

    cpu.step(&mut bus);
    assert!(cpu.state.cwai_waiting);
    let stacked_pointer = cpu.state.s;
    cpu.request_firq();

    replay_m6809(&cpu, &bus, |replayed, replayed_bus| {
        replayed.run_for(1, replayed_bus);
        assert_eq!(replayed.state.s, stacked_pointer);
        assert_eq!(replayed.state.pc, 0x2345);
        assert!(replayed_bus.events.contains(&BusEvent::Firq));
    });
}

#[test]
fn m6809_replays_armed_nmi() {
    let mut cpu = M6809::default();
    let mut bus = TraceBus::new();
    cpu.state.pc = 0x1000;
    cpu.set_s(0x3000);
    bus.set_big_endian_word(0xFFFC, 0x2345);
    bus.nmi = true;
    assert!(cpu.state.nmi_armed);

    replay_m6809(&cpu, &bus, |replayed, replayed_bus| {
        replayed.run_for(1, replayed_bus);
        assert_eq!(replayed.state.pc, 0x2345);
        assert!(replayed_bus.events.contains(&BusEvent::Nmi));
    });
}

#[test]
fn z80_rejects_invalid_state_transactionally() {
    let mut cpu = Z80::default();
    cpu.state.pc = 0x1234;
    let before = cpu.capture_state();
    let mut invalid = before.clone();
    invalid.im = 3;

    assert!(cpu.restore_state(invalid).is_err());
    assert_eq!(cpu.capture_state(), before);
}

#[test]
fn m6809_rejects_invalid_state_transactionally() {
    let mut cpu = M6809::default();
    cpu.state.pc = 0x1234;
    let before = cpu.capture_state();
    let mut invalid = before.clone();
    invalid.cwai_waiting = true;
    invalid.halted = false;

    assert!(cpu.restore_state(invalid).is_err());
    assert_eq!(cpu.capture_state(), before);
}

#[test]
fn v30_replays_repeating_movsb_with_biu_state() {
    let mut cpu = V30::new();
    let mut bus = TraceBus::new();
    cpu.state.set_cs(0);
    cpu.state.ip = 0x1000;
    cpu.state.set_sp(0x9000);
    cpu.state.set_cx(2);
    cpu.state.set_si(0x2000);
    cpu.state.set_di(0x3000);
    cpu.state.initialize_cold_frontend();
    bus.set_bytes(0x1000, &[0xF3, 0xA4, 0xF4]);
    bus.set_bytes(0x2000, &[0x41, 0x42]);

    for _ in 0..8 {
        cpu.run_for(1, &mut bus);
        if cpu.state.cx() == 1 {
            break;
        }
    }
    assert_eq!(cpu.state.cx(), 1);

    replay_v30(&cpu, &bus, |replayed, replayed_bus| {
        replayed.run_for(100, replayed_bus);
        assert!(replayed.halted());
        assert_eq!(replayed.state.cx(), 0);
        assert_eq!(replayed_bus.memory[0x3001], 0x42);
    });
}

#[test]
fn i8086_replays_repeating_movsb_with_biu_state() {
    let mut cpu = I8086::new();
    let mut bus = TraceBus::new();
    cpu.state.set_cs(0);
    cpu.state.ip = 0x1000;
    cpu.state.set_sp(0x9000);
    cpu.state.set_cx(2);
    cpu.state.set_si(0x2000);
    cpu.state.set_di(0x3000);
    cpu.state.initialize_cold_frontend();
    bus.set_bytes(0x1000, &[0xF3, 0xA4, 0xF4]);
    bus.set_bytes(0x2000, &[0x41, 0x42]);

    for _ in 0..8 {
        cpu.run_for(1, &mut bus);
        if cpu.state.cx() == 1 {
            break;
        }
    }
    assert_eq!(cpu.state.cx(), 1);

    replay_i8086(&cpu, &bus, |replayed, replayed_bus| {
        replayed.run_for(100, replayed_bus);
        assert!(replayed.halted());
        assert_eq!(replayed.state.cx(), 0);
        assert_eq!(replayed_bus.memory[0x3001], 0x42);
    });
}

#[test]
fn i8086_replays_sti_interrupt_deferral() {
    let mut cpu = I8086::new();
    let mut bus = TraceBus::new();
    cpu.state.set_cs(0);
    cpu.state.ip = 0x1000;
    cpu.state.set_sp(0x9000);
    cpu.state.initialize_cold_frontend();
    bus.set_bytes(0x1000, &[0xFB, 0x90, 0x90]);
    bus.set_bytes(0x03FC, &[0x00, 0x20, 0x00, 0x00]);
    bus.set_bytes(0x2000, &[0x90]);
    bus.irq = true;

    cpu.step(&mut bus);
    cpu.signal_irq();

    replay_i8086(&cpu, &bus, |replayed, replayed_bus| {
        replayed.step(replayed_bus);
        assert!(!replayed_bus.events.contains(&BusEvent::Irq));
        replayed.step(replayed_bus);
        assert!(replayed_bus.events.contains(&BusEvent::Irq));
    });
}

#[test]
fn v30_replays_sti_interrupt_deferral() {
    let mut cpu = V30::new();
    let mut bus = TraceBus::new();
    cpu.state.set_cs(0);
    cpu.state.ip = 0x1000;
    cpu.state.set_sp(0x9000);
    cpu.state.initialize_cold_frontend();
    bus.set_bytes(0x1000, &[0xFB, 0x90, 0x90]);
    bus.set_bytes(0x03FC, &[0x00, 0x20, 0x00, 0x00]);
    bus.set_bytes(0x2000, &[0x90]);
    bus.irq = true;

    cpu.step(&mut bus);
    cpu.signal_irq();

    replay_v30(&cpu, &bus, |replayed, replayed_bus| {
        replayed.step(replayed_bus);
        assert!(!replayed_bus.events.contains(&BusEvent::Irq));
        replayed.step(replayed_bus);
        assert!(replayed_bus.events.contains(&BusEvent::Irq));
    });
}

#[test]
fn i286_replays_repeating_movsb_with_timing_state() {
    let mut cpu = I286::new();
    let mut bus = TraceBus::new();
    cpu.state.set_cs(0);
    cpu.state.ip = 0x1000;
    cpu.state.set_sp(0x9000);
    cpu.state.set_cx(2);
    cpu.state.set_si(0x2000);
    cpu.state.set_di(0x3000);
    cpu.state.initialize_real_mode_caches();
    bus.set_bytes(0x1000, &[0xF3, 0xA4, 0xF4]);
    bus.set_bytes(0x2000, &[0x41, 0x42]);

    for _ in 0..8 {
        cpu.run_for(1, &mut bus);
        if cpu.state.cx() == 1 {
            break;
        }
    }
    assert_eq!(cpu.state.cx(), 1);

    replay_i286(&cpu, &bus, |replayed, replayed_bus| {
        replayed.run_for(100, replayed_bus);
        assert!(replayed.halted());
        assert_eq!(replayed.state.cx(), 0);
        assert_eq!(replayed_bus.memory[0x3001], 0x42);
    });
}

#[test]
fn i286_replays_sti_interrupt_deferral_and_frontend() {
    let mut cpu = I286::new();
    let mut bus = TraceBus::new();
    cpu.state.set_cs(0);
    cpu.state.ip = 0x1000;
    cpu.state.set_sp(0x9000);
    cpu.state.initialize_real_mode_caches();
    bus.set_bytes(0x1000, &[0xFB, 0x90, 0x90]);
    bus.set_bytes(0x03FC, &[0x00, 0x20, 0x00, 0x00]);
    bus.set_bytes(0x2000, &[0x90]);
    bus.irq = true;

    cpu.step(&mut bus);
    cpu.signal_irq();

    replay_i286(&cpu, &bus, |replayed, replayed_bus| {
        replayed.step(replayed_bus);
        assert!(!replayed_bus.events.contains(&BusEvent::Irq));
        replayed.step(replayed_bus);
        assert!(replayed_bus.events.contains(&BusEvent::Irq));
    });
}

#[test]
fn i286_rejects_invalid_state_transactionally() {
    let mut cpu = I286::new();
    let before = cpu.capture_state();
    let mut invalid = before.clone();
    invalid.gdt_base = 0x0100_0000;

    assert!(cpu.restore_state(invalid).is_err());
    assert_eq!(cpu.capture_state(), before);
}

#[test]
fn i386_replays_repeating_movsb_with_prefetch_state() {
    let mut cpu = I386::<CPU_MODEL_386_DX, ADDRESS_WIDTH_24>::new();
    let mut bus = TraceBus::new();
    cpu.state.set_cs(0);
    cpu.state.set_eip(0x1000);
    cpu.state.set_ss(0);
    cpu.state.set_esp(0x9000);
    cpu.state.set_ecx(2);
    cpu.state.set_esi(0x2000);
    cpu.state.set_edi(0x3000);
    cpu.state.initialize_real_mode_caches();
    bus.set_bytes(0x1000, &[0xF3, 0xA4, 0xF4]);
    bus.set_bytes(0x2000, &[0x41, 0x42]);

    for _ in 0..8 {
        cpu.run_for(1, &mut bus);
        if cpu.state.ecx() == 1 {
            break;
        }
    }
    assert_eq!(cpu.state.ecx(), 1);

    replay_i386(&cpu, &bus, |replayed, replayed_bus| {
        replayed.run_for(100, replayed_bus);
        assert!(replayed.halted());
        assert_eq!(replayed.state.ecx(), 0);
        assert_eq!(replayed_bus.memory[0x3001], 0x42);
    });
}

#[test]
fn i386sx_replays_sti_interrupt_deferral() {
    let mut cpu = I386::<CPU_MODEL_386_SX, ADDRESS_WIDTH_24>::new();
    let mut bus = TraceBus::new();
    cpu.state.set_cs(0);
    cpu.state.set_eip(0x1000);
    cpu.state.set_ss(0);
    cpu.state.set_esp(0x9000);
    cpu.state.initialize_real_mode_caches();
    bus.set_bytes(0x1000, &[0xFB, 0x90, 0x90]);
    bus.set_bytes(0x03FC, &[0x00, 0x20, 0x00, 0x00]);
    bus.set_bytes(0x2000, &[0x90]);
    bus.irq = true;

    cpu.step(&mut bus);
    cpu.signal_irq();

    replay_i386(&cpu, &bus, |replayed, replayed_bus| {
        replayed.step(replayed_bus);
        assert!(!replayed_bus.events.contains(&BusEvent::Irq));
        replayed.step(replayed_bus);
        assert!(replayed_bus.events.contains(&BusEvent::Irq));
    });
}

#[test]
fn i486_replays_x87_state_and_prefetched_instruction() {
    let mut cpu = I386::<CPU_MODEL_486_DX, ADDRESS_WIDTH_24>::new();
    let mut bus = TraceBus::new();
    cpu.state.set_cs(0);
    cpu.state.set_eip(0x1000);
    cpu.state.set_ss(0);
    cpu.state.set_esp(0x9000);
    cpu.state.initialize_real_mode_caches();
    bus.set_bytes(0x1000, &[0xD9, 0xE8, 0xD9, 0xE8, 0xDE, 0xC1, 0xF4]);

    cpu.step(&mut bus);
    assert_ne!(cpu.state.fpu.tag_word, 0xFFFF);

    replay_i386(&cpu, &bus, |replayed, replayed_bus| {
        replayed.run_for(100, replayed_bus);
        assert!(replayed.halted());
    });
}

#[test]
fn i386_rejects_invalid_state_transactionally() {
    let mut cpu = I386::<CPU_MODEL_386_DX, ADDRESS_WIDTH_24>::new();
    let before = cpu.capture_state();
    let mut invalid = before.clone();
    invalid.flags.iopl = 4;

    assert!(cpu.restore_state(invalid).is_err());
    assert_eq!(cpu.capture_state(), before);
}
