//! Motorola 68000 bus transaction tests.
//!
//! Drives the CPU core through a synthetic machine bus that can pass, delay,
//! or fault every access, covering byte lanes, endianness, function codes,
//! address-error precedence, bus errors, double faults, reset vectors,
//! interrupt acknowledge, and wait cycles.

use std::collections::{HashMap, HashSet};

use common::{
    Bus, CpuM68000, M68000AccessSize, M68000BusAccess, M68000BusError, M68000CycleKind,
    M68000FunctionCode,
};
use cpu_68k::{M68000, M68000State};

/// Base address of the test program.
const PROGRAM_BASE: u32 = 0x000400;
/// Supervisor stack pointer used by most tests.
const SUPERVISOR_STACK: u32 = 0x008000;
/// User stack pointer used by most tests.
const USER_STACK: u32 = 0x007000;
/// Bus-error handler planted at vector 2.
const BUS_ERROR_HANDLER: u32 = 0x003000;
/// Address-error handler planted at vector 3.
const ADDRESS_ERROR_HANDLER: u32 = 0x003100;
/// Spurious-interrupt handler planted at vector 24.
const SPURIOUS_HANDLER: u32 = 0x004000;
/// Level 3 autovector handler planted at vector 27.
const AUTOVECTOR_HANDLER: u32 = 0x005000;
/// Handler planted at the scripted interrupt vector 0x40.
const SCRIPTED_IRQ_HANDLER: u32 = 0x002000;
/// Size of a group 0 exception stack frame in bytes.
const GROUP0_FRAME_SIZE: u32 = 14;
/// Size of a group 1/2 exception stack frame in bytes.
const GROUP12_FRAME_SIZE: u32 = 6;
/// NOP opcode.
const NOP: u16 = 0x4E71;

/// One access observed by the scripted bus, including faulted attempts.
#[derive(Debug, Clone, Copy)]
struct RecordedAccess {
    access: M68000BusAccess,
    write: bool,
}

/// A 68000 bus with scriptable faults, wait cycles, and interrupt vectors.
struct ScriptedBus {
    ram: HashMap<u32, u8>,
    access_log: Vec<RecordedAccess>,
    fault_reads: HashSet<u32>,
    fault_writes: HashSet<u32>,
    wait_cycles_per_access: i64,
    pending_wait_cycles: i64,
    interrupt_level: u8,
    interrupt_vector: u8,
    current_cycle: u64,
}

impl ScriptedBus {
    fn new() -> Self {
        Self {
            ram: HashMap::new(),
            access_log: Vec::new(),
            fault_reads: HashSet::new(),
            fault_writes: HashSet::new(),
            wait_cycles_per_access: 0,
            pending_wait_cycles: 0,
            interrupt_level: 0,
            interrupt_vector: 0,
            current_cycle: 0,
        }
    }

    fn ram_byte(&self, address: u32) -> u8 {
        self.ram.get(&address).copied().unwrap_or(0)
    }

    fn ram_word(&self, address: u32) -> u16 {
        (u16::from(self.ram_byte(address)) << 8) | u16::from(self.ram_byte(address + 1))
    }

    fn ram_long(&self, address: u32) -> u32 {
        (u32::from(self.ram_word(address)) << 16) | u32::from(self.ram_word(address + 2))
    }

    fn set_word(&mut self, address: u32, value: u16) {
        self.ram.insert(address, (value >> 8) as u8);
        self.ram.insert(address + 1, value as u8);
    }

    fn set_long(&mut self, address: u32, value: u32) {
        self.set_word(address, (value >> 16) as u16);
        self.set_word(address + 2, value as u16);
    }

    /// Plants program words at the given address.
    fn set_program(&mut self, address: u32, words: &[u16]) {
        for (index, &word) in words.iter().enumerate() {
            self.set_word(address + 2 * index as u32, word);
        }
    }

    /// Accesses made to the data space, in order.
    fn data_accesses(&self) -> Vec<RecordedAccess> {
        self.access_log
            .iter()
            .filter(|entry| {
                matches!(
                    entry.access.function_code,
                    M68000FunctionCode::UserData | M68000FunctionCode::SupervisorData
                )
            })
            .copied()
            .collect()
    }

    /// Accesses touching the given address, in order.
    fn accesses_at(&self, address: u32) -> Vec<RecordedAccess> {
        self.access_log
            .iter()
            .filter(|entry| entry.access.address == address)
            .copied()
            .collect()
    }
}

impl Bus for ScriptedBus {
    fn read_byte(&mut self, address: u32) -> u8 {
        self.ram_byte(address)
    }

    fn write_byte(&mut self, address: u32, value: u8) {
        self.ram.insert(address, value);
    }

    fn io_read_byte(&mut self, _port: u16) -> u8 {
        panic!("the 68000 has no I/O port space");
    }

    fn io_write_byte(&mut self, _port: u16, _value: u8) {
        panic!("the 68000 has no I/O port space");
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

    fn m68000_interrupt_level(&self) -> u8 {
        self.interrupt_level
    }

    fn m68000_read(&mut self, access: M68000BusAccess) -> Result<u16, M68000BusError> {
        self.access_log.push(RecordedAccess {
            access,
            write: false,
        });
        self.pending_wait_cycles += self.wait_cycles_per_access;
        if self.fault_reads.contains(&access.address) {
            return Err(M68000BusError);
        }
        if matches!(access.function_code, M68000FunctionCode::CpuSpace) {
            return Ok(u16::from(self.interrupt_vector));
        }
        match access.size {
            M68000AccessSize::Byte => Ok(u16::from(self.ram_byte(access.address))),
            M68000AccessSize::Word => Ok(self.ram_word(access.address)),
        }
    }

    fn m68000_write(&mut self, access: M68000BusAccess, value: u16) -> Result<(), M68000BusError> {
        self.access_log.push(RecordedAccess {
            access,
            write: true,
        });
        self.pending_wait_cycles += self.wait_cycles_per_access;
        if self.fault_writes.contains(&access.address) {
            return Err(M68000BusError);
        }
        match access.size {
            M68000AccessSize::Byte => {
                self.ram.insert(access.address, value as u8);
            }
            M68000AccessSize::Word => self.set_word(access.address, value),
        }
        Ok(())
    }

    fn current_cycle(&self) -> u64 {
        self.current_cycle
    }

    fn set_current_cycle(&mut self, cycle: u64) {
        self.current_cycle = cycle;
    }

    fn drain_wait_cycles(&mut self) -> i64 {
        core::mem::take(&mut self.pending_wait_cycles)
    }
}

/// A bus that keeps the default `m68000_read` bridge, so interrupt
/// acknowledge autovectors through `m68000_acknowledge_interrupt`.
struct AutovectorBus {
    ram: HashMap<u32, u8>,
    interrupt_level: u8,
    current_cycle: u64,
}

impl Bus for AutovectorBus {
    fn read_byte(&mut self, address: u32) -> u8 {
        self.ram.get(&address).copied().unwrap_or(0)
    }

    fn write_byte(&mut self, address: u32, value: u8) {
        self.ram.insert(address, value);
    }

    fn io_read_byte(&mut self, _port: u16) -> u8 {
        panic!("the 68000 has no I/O port space");
    }

    fn io_write_byte(&mut self, _port: u16, _value: u8) {
        panic!("the 68000 has no I/O port space");
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

    fn m68000_interrupt_level(&self) -> u8 {
        self.interrupt_level
    }

    fn current_cycle(&self) -> u64 {
        self.current_cycle
    }

    fn set_current_cycle(&mut self, cycle: u64) {
        self.current_cycle = cycle;
    }
}

/// Builds a bus with exception handlers planted behind the standard vectors.
fn scripted_bus() -> ScriptedBus {
    let mut bus = ScriptedBus::new();
    bus.set_long(0x008, BUS_ERROR_HANDLER);
    bus.set_long(0x00C, ADDRESS_ERROR_HANDLER);
    bus.set_long(0x060, SPURIOUS_HANDLER);
    bus.set_long(0x06C, AUTOVECTOR_HANDLER);
    bus.set_long(0x100, SCRIPTED_IRQ_HANDLER);
    for handler in [
        BUS_ERROR_HANDLER,
        ADDRESS_ERROR_HANDLER,
        SPURIOUS_HANDLER,
        AUTOVECTOR_HANDLER,
        SCRIPTED_IRQ_HANDLER,
    ] {
        bus.set_program(handler, &[NOP, NOP, NOP]);
    }
    bus
}

/// Builds CPU state at [`PROGRAM_BASE`] with the prefetch queue primed from
/// the program already planted in the bus RAM.
fn program_state(bus: &ScriptedBus, sr: u16) -> M68000State {
    M68000State {
        pc: PROGRAM_BASE,
        sr,
        ir: bus.ram_word(PROGRAM_BASE),
        irc: bus.ram_word(PROGRAM_BASE + 2),
        usp: USER_STACK,
        ssp: SUPERVISOR_STACK,
        ..M68000State::default()
    }
}

/// Loads the program words, primes the CPU, and executes one instruction.
fn run_program(bus: &mut ScriptedBus, words: &[u16], sr: u16) -> (M68000, u64) {
    bus.set_program(PROGRAM_BASE, words);
    let mut cpu = M68000::new(10_000_000);
    cpu.load_state(program_state(bus, sr));
    bus.access_log.clear();
    let cycles = cpu.step(bus);
    (cpu, cycles)
}

#[test]
fn byte_read_even_address_uses_upper_lane() {
    let mut bus = scripted_bus();
    bus.ram.insert(0x1000, 0xAB);
    bus.ram.insert(0x1001, 0xCD);
    let (cpu, _) = run_program(&mut bus, &[0x1038, 0x1000, NOP], 0x2700);
    let data = bus.data_accesses();
    assert_eq!(data.len(), 1);
    assert_eq!(data[0].access.address, 0x1000);
    assert_eq!(data[0].access.size, M68000AccessSize::Byte);
    assert!(!data[0].write);
    assert_eq!(cpu.save_state().data[0] & 0xFF, 0xAB);
    assert!(bus.accesses_at(0x1001).is_empty());
}

#[test]
fn byte_read_odd_address_uses_lower_lane() {
    let mut bus = scripted_bus();
    bus.ram.insert(0x1000, 0xAB);
    bus.ram.insert(0x1001, 0xCD);
    let (cpu, _) = run_program(&mut bus, &[0x1038, 0x1001, NOP], 0x2700);
    let data = bus.data_accesses();
    assert_eq!(data.len(), 1);
    assert_eq!(data[0].access.address, 0x1001);
    assert_eq!(data[0].access.size, M68000AccessSize::Byte);
    assert_eq!(cpu.save_state().data[0] & 0xFF, 0xCD);
    assert!(bus.accesses_at(0x1000).is_empty());
}

#[test]
fn byte_write_uses_addressed_lane_only() {
    for (target, untouched) in [(0x1000u32, 0x1001u32), (0x1001, 0x1000)] {
        let mut bus = scripted_bus();
        bus.ram.insert(0x1000, 0x11);
        bus.ram.insert(0x1001, 0x22);
        bus.set_program(PROGRAM_BASE, &[0x11C0, target as u16, NOP]);
        let mut cpu = M68000::new(10_000_000);
        let mut state = program_state(&bus, 0x2700);
        state.data[0] = 0xAB;
        cpu.load_state(state);
        bus.access_log.clear();
        cpu.step(&mut bus);
        let before = if untouched == 0x1000 { 0x11 } else { 0x22 };
        assert_eq!(bus.ram_byte(target), 0xAB);
        assert_eq!(bus.ram_byte(untouched), before);
        let data = bus.data_accesses();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].access.address, target);
        assert_eq!(data[0].access.size, M68000AccessSize::Byte);
        assert!(data[0].write);
    }
}

#[test]
fn word_read_is_big_endian() {
    let mut bus = scripted_bus();
    bus.ram.insert(0x1000, 0x12);
    bus.ram.insert(0x1001, 0x34);
    let (cpu, _) = run_program(&mut bus, &[0x3038, 0x1000, NOP], 0x2700);
    let data = bus.data_accesses();
    assert_eq!(data.len(), 1);
    assert_eq!(data[0].access.address, 0x1000);
    assert_eq!(data[0].access.size, M68000AccessSize::Word);
    assert_eq!(cpu.save_state().data[0] & 0xFFFF, 0x1234);
}

#[test]
fn word_write_is_big_endian() {
    let mut bus = scripted_bus();
    bus.set_program(PROGRAM_BASE, &[0x31C0, 0x1000, NOP]);
    let mut cpu = M68000::new(10_000_000);
    let mut state = program_state(&bus, 0x2700);
    state.data[0] = 0x0000_ABCD;
    cpu.load_state(state);
    cpu.step(&mut bus);
    assert_eq!(bus.ram_byte(0x1000), 0xAB);
    assert_eq!(bus.ram_byte(0x1001), 0xCD);
}

#[test]
fn long_access_is_two_word_cycles() {
    let mut bus = scripted_bus();
    bus.set_long(0x1000, 0x1234_5678);
    let (cpu, _) = run_program(&mut bus, &[0x2038, 0x1000, NOP], 0x2700);
    let data = bus.data_accesses();
    assert_eq!(data.len(), 2);
    assert_eq!(data[0].access.address, 0x1000);
    assert_eq!(data[1].access.address, 0x1002);
    assert!(
        data.iter()
            .all(|entry| entry.access.size == M68000AccessSize::Word)
    );
    assert_eq!(cpu.save_state().data[0], 0x1234_5678);
}

#[test]
fn function_codes_follow_supervisor_bit() {
    let mut bus = scripted_bus();
    let (_, _) = run_program(&mut bus, &[0x3038, 0x1000, NOP], 0x2700);
    assert!(bus.access_log.iter().all(|entry| {
        matches!(
            entry.access.function_code,
            M68000FunctionCode::SupervisorProgram | M68000FunctionCode::SupervisorData
        )
    }));
    assert_eq!(bus.data_accesses().len(), 1);

    let mut bus = scripted_bus();
    let (_, _) = run_program(&mut bus, &[0x3038, 0x1000, NOP], 0x0700);
    assert!(bus.access_log.iter().all(|entry| {
        matches!(
            entry.access.function_code,
            M68000FunctionCode::UserProgram | M68000FunctionCode::UserData
        )
    }));
    assert_eq!(bus.data_accesses().len(), 1);
}

#[test]
fn word_access_to_odd_address_takes_address_error() {
    let mut bus = scripted_bus();
    let (cpu, _) = run_program(&mut bus, &[0x3038, 0x1001, NOP], 0x2700);
    assert!(!cpu.halted());
    let state = cpu.save_state();
    assert_eq!(state.pc, ADDRESS_ERROR_HANDLER);
    assert_eq!(state.ssp, SUPERVISOR_STACK - GROUP0_FRAME_SIZE);
    let frame = state.ssp;
    let status_word = bus.ram_word(frame);
    assert_eq!(status_word & 0x10, 0x10, "read fault");
    assert_eq!(status_word & 0x07, 5, "supervisor data function code");
    assert_eq!(bus.ram_long(frame + 2), 0x1001, "odd fault address");
    assert_eq!(bus.ram_word(frame + 8), 0x2700, "pre-exception SR");
    let doomed = bus.accesses_at(0x1000);
    assert_eq!(doomed.len(), 1, "one even-ized word cycle reaches the bus");
    assert_eq!(doomed[0].access.size, M68000AccessSize::Word);
}

#[test]
fn bus_fault_on_doomed_cycle_is_discarded() {
    let mut bus = scripted_bus();
    bus.fault_reads.insert(0x1000);
    let (cpu, _) = run_program(&mut bus, &[0x3038, 0x1001, NOP], 0x2700);
    assert!(!cpu.halted());
    let state = cpu.save_state();
    assert_eq!(state.pc, ADDRESS_ERROR_HANDLER, "address error wins");
    assert_eq!(bus.ram_long(state.ssp + 2), 0x1001);
}

#[test]
fn read_bus_error_stacks_group_zero_frame() {
    let mut bus = scripted_bus();
    bus.fault_reads.insert(0x1000);
    let (cpu, _) = run_program(&mut bus, &[0x3038, 0x1000, NOP], 0x2700);
    assert!(!cpu.halted());
    let state = cpu.save_state();
    assert_eq!(state.pc, BUS_ERROR_HANDLER);
    assert_eq!(state.ssp, SUPERVISOR_STACK - GROUP0_FRAME_SIZE);
    assert_ne!(state.sr & 0x2000, 0, "supervisor bit set");
    let frame = state.ssp;
    let status_word = bus.ram_word(frame);
    assert_eq!(status_word & 0x10, 0x10, "read fault");
    assert_eq!(status_word & 0x07, 5, "supervisor data function code");
    assert_eq!(bus.ram_long(frame + 2), 0x1000, "fault address");
    assert_eq!(bus.ram_word(frame + 6), 0x3038, "instruction register");
    assert_eq!(bus.ram_word(frame + 8), 0x2700, "pre-exception SR");
}

#[test]
fn write_bus_error_reports_write_direction() {
    let mut bus = scripted_bus();
    bus.fault_writes.insert(0x1001);
    bus.set_program(PROGRAM_BASE, &[0x11C0, 0x1001, NOP]);
    let mut cpu = M68000::new(10_000_000);
    let mut state = program_state(&bus, 0x2700);
    state.data[0] = 0xAB;
    cpu.load_state(state);
    cpu.step(&mut bus);
    assert!(!cpu.halted());
    let state = cpu.save_state();
    assert_eq!(state.pc, BUS_ERROR_HANDLER);
    let frame = state.ssp;
    assert_eq!(bus.ram_word(frame) & 0x10, 0, "write fault");
    assert_eq!(bus.ram_long(frame + 2), 0x1001, "odd byte fault address");
}

#[test]
fn user_mode_fault_reports_user_function_code() {
    let mut bus = scripted_bus();
    bus.fault_reads.insert(0x1000);
    let (cpu, _) = run_program(&mut bus, &[0x1038, 0x1000, NOP], 0x0700);
    assert!(!cpu.halted());
    let state = cpu.save_state();
    assert_eq!(state.pc, BUS_ERROR_HANDLER);
    assert_ne!(state.sr & 0x2000, 0, "exception enters supervisor mode");
    assert_eq!(state.usp, USER_STACK, "user stack untouched");
    let frame = state.ssp;
    let status_word = bus.ram_word(frame);
    assert_eq!(status_word & 0x07, 1, "user data function code");
    assert_eq!(status_word & 0x10, 0x10, "read fault");
}

#[test]
fn faulted_long_read_issues_single_cycle() {
    let mut bus = scripted_bus();
    bus.fault_reads.insert(0x1000);
    let (cpu, _) = run_program(&mut bus, &[0x2038, 0x1000, NOP], 0x2700);
    assert!(!cpu.halted());
    assert_eq!(bus.accesses_at(0x1000).len(), 1);
    assert!(
        bus.accesses_at(0x1002).is_empty(),
        "second word cycle is never issued"
    );
    assert_eq!(bus.ram_long(cpu.save_state().ssp + 2), 0x1000);
}

/// Marks every word in the supervisor stack frame region as write-faulting.
fn fault_stack_region(bus: &mut ScriptedBus) {
    for address in (SUPERVISOR_STACK - 2 * GROUP0_FRAME_SIZE)..SUPERVISOR_STACK {
        bus.fault_writes.insert(address);
    }
}

#[test]
fn bus_fault_during_bus_error_stacking_double_faults() {
    let mut bus = scripted_bus();
    bus.fault_reads.insert(0x1000);
    fault_stack_region(&mut bus);
    let (mut cpu, _) = run_program(&mut bus, &[0x3038, 0x1000, NOP], 0x2700);
    assert!(cpu.halted());
    let last = bus.access_log.last().unwrap();
    assert!(last.write, "halt right at the faulting stack write");
    assert_eq!(cpu.step(&mut bus), 0, "halted CPU consumes no cycles");
}

#[test]
fn bus_fault_during_address_error_stacking_double_faults() {
    let mut bus = scripted_bus();
    fault_stack_region(&mut bus);
    let (mut cpu, _) = run_program(&mut bus, &[0x3038, 0x1001, NOP], 0x2700);
    assert!(cpu.halted());
    assert_eq!(cpu.step(&mut bus), 0);
}

#[test]
fn address_error_during_bus_error_stacking_double_faults() {
    let mut bus = scripted_bus();
    bus.fault_reads.insert(0x1000);
    bus.set_program(PROGRAM_BASE, &[0x3038, 0x1000, NOP]);
    let mut cpu = M68000::new(10_000_000);
    let mut state = program_state(&bus, 0x2700);
    state.ssp = SUPERVISOR_STACK + 1;
    cpu.load_state(state);
    cpu.step(&mut bus);
    assert!(cpu.halted(), "odd SSP faults the group 0 stacking");
    assert_eq!(cpu.step(&mut bus), 0);
}

#[test]
fn address_error_during_address_error_stacking_double_faults() {
    let mut bus = scripted_bus();
    bus.set_program(PROGRAM_BASE, &[0x3038, 0x1001, NOP]);
    let mut cpu = M68000::new(10_000_000);
    let mut state = program_state(&bus, 0x2700);
    state.ssp = SUPERVISOR_STACK + 1;
    cpu.load_state(state);
    cpu.step(&mut bus);
    assert!(cpu.halted());
    assert_eq!(cpu.step(&mut bus), 0);
}

#[test]
fn odd_group_zero_vector_double_faults() {
    let mut bus = scripted_bus();
    bus.fault_reads.insert(0x1000);
    bus.set_long(0x008, 0x0000_2001);
    let (mut cpu, _) = run_program(&mut bus, &[0x3038, 0x1000, NOP], 0x2700);
    assert!(
        cpu.halted(),
        "address error on the handler prefetch during group 0 processing halts"
    );
    assert_eq!(cpu.step(&mut bus), 0);
}

#[test]
fn odd_interrupt_vector_takes_normal_address_error() {
    let mut bus = scripted_bus();
    bus.set_long(0x100, 0x0000_2001);
    bus.interrupt_level = 3;
    bus.interrupt_vector = 0x40;
    let (cpu, _) = run_program(&mut bus, &[NOP, NOP], 0x2000);
    assert!(
        !cpu.halted(),
        "group 1/2 exception processing is not a double fault window"
    );
    let state = cpu.save_state();
    assert_eq!(state.pc, ADDRESS_ERROR_HANDLER);
    assert_eq!(
        state.ssp,
        SUPERVISOR_STACK - GROUP12_FRAME_SIZE - GROUP0_FRAME_SIZE,
        "interrupt frame plus address error frame"
    );
}

#[test]
fn reset_fetches_vectors_with_reset_kind() {
    let mut bus = scripted_bus();
    bus.set_long(0x000, 0x0010_4000);
    bus.set_long(0x004, 0x0010_0200);
    bus.set_program(0x10_0200, &[NOP, NOP, NOP]);
    let mut cpu = M68000::new(10_000_000);
    cpu.step(&mut bus);
    assert!(!cpu.halted());
    assert_eq!(cpu.ssp(), 0x0010_4000);
    let expected = [0x000, 0x002, 0x004, 0x006];
    for (index, &address) in expected.iter().enumerate() {
        let entry = bus.access_log[index];
        assert_eq!(entry.access.address, address);
        assert_eq!(entry.access.size, M68000AccessSize::Word);
        assert_eq!(
            entry.access.function_code,
            M68000FunctionCode::SupervisorProgram
        );
        assert_eq!(entry.access.cycle_kind, M68000CycleKind::ResetVector);
    }
    for entry in &bus.access_log[4..] {
        assert_eq!(entry.access.cycle_kind, M68000CycleKind::Normal);
    }
    assert_eq!(bus.access_log[4].access.address, 0x10_0200);
}

#[test]
fn reset_after_run_retags_vectors() {
    let mut bus = scripted_bus();
    bus.set_long(0x000, u32::from(SUPERVISOR_STACK as u16));
    bus.set_long(0x004, PROGRAM_BASE);
    let (mut cpu, _) = run_program(&mut bus, &[NOP, NOP, NOP], 0x2700);
    assert!(
        bus.access_log
            .iter()
            .all(|entry| entry.access.cycle_kind == M68000CycleKind::Normal)
    );
    cpu.reset();
    bus.access_log.clear();
    cpu.step(&mut bus);
    let reset_reads = bus
        .access_log
        .iter()
        .filter(|entry| entry.access.cycle_kind == M68000CycleKind::ResetVector)
        .count();
    assert_eq!(reset_reads, 4);
}

#[test]
fn faulted_reset_vector_read_halts() {
    let mut bus = scripted_bus();
    bus.fault_reads.insert(0x000);
    let mut cpu = M68000::new(10_000_000);
    cpu.step(&mut bus);
    assert!(cpu.halted());
    assert_eq!(bus.access_log.len(), 1);
    assert_eq!(cpu.step(&mut bus), 0);
}

#[test]
fn interrupt_acknowledge_is_cpu_space_access() {
    let mut bus = scripted_bus();
    bus.interrupt_level = 3;
    bus.interrupt_vector = 0x40;
    let (cpu, _) = run_program(&mut bus, &[NOP, NOP], 0x2000);
    let acknowledge: Vec<RecordedAccess> = bus
        .access_log
        .iter()
        .filter(|entry| entry.access.function_code == M68000FunctionCode::CpuSpace)
        .copied()
        .collect();
    assert_eq!(acknowledge.len(), 1);
    assert_eq!(acknowledge[0].access.address, 0xFF_FFF6);
    assert_eq!(acknowledge[0].access.size, M68000AccessSize::Byte);
    assert_eq!(acknowledge[0].access.cycle_kind, M68000CycleKind::Normal);
    let state = cpu.save_state();
    assert_eq!(state.pc, SCRIPTED_IRQ_HANDLER);
    assert_eq!(state.sr & 0x0700, 0x0300, "interrupt mask raised to 3");
    assert_eq!(state.ssp, SUPERVISOR_STACK - GROUP12_FRAME_SIZE);
    assert_eq!(bus.ram_word(state.ssp), 0x2000, "stacked SR");
    let frame_writes = bus
        .access_log
        .iter()
        .filter(|entry| entry.write)
        .collect::<Vec<_>>();
    assert!(
        frame_writes
            .iter()
            .all(|entry| { entry.access.function_code == M68000FunctionCode::SupervisorData })
    );
    assert_eq!(frame_writes.len(), 3, "three word frame");
}

#[test]
fn interrupt_acknowledge_default_bridge_autovectors() {
    let mut bus = AutovectorBus {
        ram: HashMap::new(),
        interrupt_level: 3,
        current_cycle: 0,
    };
    let mut scratch = scripted_bus();
    scratch.set_program(PROGRAM_BASE, &[NOP, NOP]);
    scratch.set_program(AUTOVECTOR_HANDLER, &[NOP, NOP]);
    scratch.set_long(0x06C, AUTOVECTOR_HANDLER);
    bus.ram = scratch.ram.clone();
    let mut cpu = M68000::new(10_000_000);
    cpu.load_state(program_state(&scratch, 0x2000));
    cpu.step(&mut bus);
    assert_eq!(
        cpu.save_state().pc,
        AUTOVECTOR_HANDLER,
        "default bridge yields vector 0x18 + level"
    );
}

#[test]
fn faulted_interrupt_acknowledge_takes_spurious_vector() {
    let mut bus = scripted_bus();
    bus.interrupt_level = 3;
    bus.interrupt_vector = 0x40;
    bus.fault_reads.insert(0xFF_FFF6);
    let (cpu, _) = run_program(&mut bus, &[NOP, NOP], 0x2000);
    assert!(!cpu.halted());
    assert_eq!(cpu.save_state().pc, SPURIOUS_HANDLER);
}

#[test]
fn step_drains_wait_cycles() {
    let mut plain_bus = scripted_bus();
    let (_, plain_cycles) = run_program(&mut plain_bus, &[NOP, NOP], 0x2700);

    let mut waiting_bus = scripted_bus();
    waiting_bus.wait_cycles_per_access = 2;
    let (_, waiting_cycles) = run_program(&mut waiting_bus, &[NOP, NOP], 0x2700);

    let access_count = waiting_bus.access_log.len() as u64;
    assert!(access_count > 0);
    assert_eq!(waiting_cycles, plain_cycles + 2 * access_count);
    assert_eq!(waiting_bus.current_cycle(), waiting_cycles);
    assert_eq!(waiting_bus.pending_wait_cycles, 0);
}

#[test]
fn step_and_run_for_agree_on_wait_cycles() {
    let mut step_bus = scripted_bus();
    step_bus.wait_cycles_per_access = 3;
    let (_, step_cycles) = run_program(&mut step_bus, &[0x3038, 0x1000, NOP], 0x2700);

    let mut run_for_bus = scripted_bus();
    run_for_bus.wait_cycles_per_access = 3;
    run_for_bus.set_program(PROGRAM_BASE, &[0x3038, 0x1000, NOP]);
    let mut cpu = M68000::new(10_000_000);
    cpu.load_state(program_state(&run_for_bus, 0x2700));
    let run_for_cycles = cpu.run_for(1, &mut run_for_bus);

    assert_eq!(step_cycles, run_for_cycles);
    assert_eq!(step_bus.current_cycle(), run_for_bus.current_cycle());
}
