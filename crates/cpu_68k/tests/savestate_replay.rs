use std::collections::HashMap;

use common::{Bus, CpuM68000 as _};
use cpu_68k::{M68000, M68000State};
use save_state::{decode_runtime_state, encode_runtime_state};

const PROGRAM_BASE: u32 = 0x0004_0000;

#[derive(Debug, Clone, PartialEq, Eq)]
enum BusEvent {
    Read(u32, u8),
    Write(u32, u8),
    Interrupt(u8),
}

#[derive(Clone)]
struct TraceBus {
    memory: HashMap<u32, u8>,
    events: Vec<BusEvent>,
    current_cycle: u64,
    interrupt_level: u8,
}

impl TraceBus {
    fn new() -> Self {
        Self {
            memory: HashMap::new(),
            events: Vec::new(),
            current_cycle: 0,
            interrupt_level: 0,
        }
    }

    fn set_word(&mut self, address: u32, value: u16) {
        self.memory.insert(address, (value >> 8) as u8);
        self.memory.insert(address + 1, value as u8);
    }

    fn set_long(&mut self, address: u32, value: u32) {
        self.set_word(address, (value >> 16) as u16);
        self.set_word(address + 2, value as u16);
    }

    fn set_program(&mut self, words: &[u16]) {
        for (index, word) in words.iter().copied().enumerate() {
            self.set_word(PROGRAM_BASE + 2 * index as u32, word);
        }
    }

    fn program_state(&self, status_register: u16) -> M68000State {
        M68000State {
            pc: PROGRAM_BASE,
            sr: status_register,
            ir: self.word(PROGRAM_BASE),
            irc: self.word(PROGRAM_BASE + 2),
            usp: 0x0007_0000,
            ssp: 0x0008_0000,
            ..M68000State::default()
        }
    }

    fn byte(&self, address: u32) -> u8 {
        self.memory.get(&address).copied().unwrap_or(0)
    }

    fn word(&self, address: u32) -> u16 {
        (u16::from(self.byte(address)) << 8) | u16::from(self.byte(address + 1))
    }
}

impl Bus for TraceBus {
    fn read_byte(&mut self, address: u32) -> u8 {
        let value = self.byte(address);
        self.events.push(BusEvent::Read(address, value));
        value
    }

    fn write_byte(&mut self, address: u32, value: u8) {
        self.memory.insert(address, value);
        self.events.push(BusEvent::Write(address, value));
    }

    fn io_read_byte(&mut self, _port: u16) -> u8 {
        unreachable!()
    }

    fn io_write_byte(&mut self, _port: u16, _value: u8) {
        unreachable!()
    }

    fn has_irq(&self) -> bool {
        false
    }

    fn acknowledge_irq(&mut self) -> u8 {
        unreachable!()
    }

    fn has_nmi(&self) -> bool {
        false
    }

    fn acknowledge_nmi(&mut self) {}

    fn m68000_interrupt_level(&self) -> u8 {
        self.interrupt_level
    }

    fn m68000_acknowledge_interrupt(&mut self, level: u8) -> u8 {
        self.events.push(BusEvent::Interrupt(level));
        0x18 + level
    }

    fn current_cycle(&self) -> u64 {
        self.current_cycle
    }

    fn set_current_cycle(&mut self, cycle: u64) {
        self.current_cycle = cycle;
    }
}

fn replay(source: &M68000, source_bus: &TraceBus, operation: impl Fn(&mut M68000, &mut TraceBus)) {
    let encoded = encode_runtime_state(&source.capture_state());
    let state = decode_runtime_state(&encoded, 4096).unwrap();
    let mut restored = M68000::new(source.clock_hz());
    restored.restore_state(state).unwrap();

    let mut uninterrupted = M68000::new(source.clock_hz());
    uninterrupted.restore_state(source.capture_state()).unwrap();
    let mut uninterrupted_bus = source_bus.clone();
    let mut restored_bus = source_bus.clone();
    uninterrupted_bus.events.clear();
    restored_bus.events.clear();

    operation(&mut uninterrupted, &mut uninterrupted_bus);
    operation(&mut restored, &mut restored_bus);

    assert_eq!(restored.capture_state(), uninterrupted.capture_state());
    assert_eq!(restored_bus.events, uninterrupted_bus.events);
    assert_eq!(restored_bus.memory, uninterrupted_bus.memory);
    assert_eq!(restored_bus.current_cycle, uninterrupted_bus.current_cycle);
}

#[test]
fn m68000_replays_prefetched_next_instruction() {
    let mut bus = TraceBus::new();
    bus.set_program(&[0x4E71, 0x7001, 0x4E71]);
    let mut cpu = M68000::new(10_000_000);
    cpu.load_state(bus.program_state(0x2000));
    cpu.step(&mut bus);

    replay(&cpu, &bus, |replayed, replayed_bus| {
        replayed.step(replayed_bus);
        assert_eq!(replayed.save_state().data[0], 1);
    });
}

#[test]
fn m68000_replays_stop_wakeup_and_interrupt_dispatch() {
    let mut bus = TraceBus::new();
    bus.set_program(&[0x4E72, 0x2000, 0x4E71]);
    bus.set_long((0x18 + 3) * 4, 0x0006_0000);
    bus.set_word(0x0006_0000, 0x4E71);
    bus.set_word(0x0006_0002, 0x4E71);
    let mut cpu = M68000::new(10_000_000);
    cpu.load_state(bus.program_state(0x2000));
    cpu.step(&mut bus);
    assert!(cpu.halted());
    bus.interrupt_level = 3;

    replay(&cpu, &bus, |replayed, replayed_bus| {
        replayed.step(replayed_bus);
        assert!(!replayed.halted());
        assert!(replayed_bus.events.contains(&BusEvent::Interrupt(3)));
    });
}
