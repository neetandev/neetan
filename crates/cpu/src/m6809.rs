//! Implements the Motorola MC6809 emulation.
//!
//! Following references were used to write the emulator:
//!
//! - Motorola, "MC6809-MC6809E Microprocessor Programming Manual" (https://www.maddes.net/m6809pm/index.htm).
//! - Darren Atkinson, "Motorola 6809 and Hitachi 6309 Programmer's Reference".

mod alu;
mod execute;
mod execute_page2;
mod execute_page3;
mod flags;
mod indexed;
mod interrupt;
mod state;

use core::ops::{Deref, DerefMut};

use common::Cpu6809;
pub use flags::M6809Flags;
pub use state::M6809State;

pub(crate) const VECTOR_SWI3: u16 = 0xFFF2;
pub(crate) const VECTOR_SWI2: u16 = 0xFFF4;
pub(crate) const VECTOR_FIRQ: u16 = 0xFFF6;
pub(crate) const VECTOR_IRQ: u16 = 0xFFF8;
pub(crate) const VECTOR_SWI: u16 = 0xFFFA;
pub(crate) const VECTOR_NMI: u16 = 0xFFFC;
pub(crate) const VECTOR_RESET: u16 = 0xFFFE;

const PENDING_FIRQ: u8 = 0x04;

/// Default 6809 clock frequency used by verification tests.
pub const M6809_DEFAULT_CLOCK_HZ: u32 = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AddressMode {
    Immediate,
    Direct,
    Indexed,
    Extended,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EffectiveAddress {
    pub(crate) address: u16,
    pub(crate) extra_cycles: i32,
}

/// Motorola MC6809 CPU emulator.
pub struct M6809 {
    /// Embedded state for save/restore.
    pub state: M6809State,

    clock_hz: u32,
    halted: bool,
    pending_irq: u8,
    cycles_remaining: i64,
    run_start_cycle: u64,
    run_budget: u64,
    nmi_armed: bool,
    cwai_waiting: bool,
    pending_extended_clear: Option<u16>,
}

impl Deref for M6809 {
    type Target = M6809State;

    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl DerefMut for M6809 {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

impl Default for M6809 {
    fn default() -> Self {
        Self::new(M6809_DEFAULT_CLOCK_HZ)
    }
}

impl M6809 {
    /// Creates a new 6809 CPU in its reset state.
    pub fn new(clock_hz: u32) -> Self {
        let mut cpu = Self {
            state: M6809State::default(),
            clock_hz,
            halted: false,
            pending_irq: 0,
            cycles_remaining: 0,
            run_start_cycle: 0,
            run_budget: 0,
            nmi_armed: false,
            cwai_waiting: false,
            pending_extended_clear: None,
        };
        cpu.reset();
        cpu
    }

    /// Loads CPU state from a snapshot, resetting runtime latches.
    pub fn load_state(&mut self, state: &M6809State) {
        self.state = state.clone();
        self.halted = false;
        self.pending_irq = 0;
        self.nmi_armed = true;
        self.cwai_waiting = false;
        self.pending_extended_clear = None;
    }

    /// Resets the CPU and fetches the reset vector from the bus.
    pub fn reset_with_bus(&mut self, bus: &mut impl common::Bus) {
        self.reset();
        self.pc = self.read_word_untracked(bus, VECTOR_RESET);
    }

    /// Asserts the FIRQ input latch.
    pub fn request_firq(&mut self) {
        self.pending_irq |= PENDING_FIRQ;
    }

    /// Clears the FIRQ input latch.
    pub fn clear_firq(&mut self) {
        self.pending_irq &= !PENDING_FIRQ;
    }

    /// Executes exactly one logical instruction.
    pub fn step(&mut self, bus: &mut impl common::Bus) {
        let start_cycle = bus.current_cycle();
        self.cycles_remaining = i64::MAX;
        self.execute_one(bus);
        if self.pending_extended_clear.is_some() {
            self.execute_one(bus);
        }
        self.cycles_remaining -= bus.drain_wait_cycles();
        bus.set_current_cycle(start_cycle + self.cycles_consumed());
        bus.on_instruction_end();
        self.cycles_remaining -= bus.drain_wait_cycles();
        bus.set_current_cycle(start_cycle + self.cycles_consumed());
    }

    /// Returns the number of cycles consumed by the last `step()` call.
    pub fn cycles_consumed(&self) -> u64 {
        (i64::MAX - self.cycles_remaining) as u64
    }

    #[inline(always)]
    pub(crate) fn clk(&mut self, cycles: i32) {
        self.cycles_remaining -= i64::from(cycles);
    }

    #[inline(always)]
    pub(crate) fn read_byte(&mut self, bus: &mut impl common::Bus, address: u16) -> u8 {
        let value = bus.read_byte(u32::from(address));
        self.clk(1);
        value
    }

    #[inline(always)]
    pub(crate) fn write_byte(&mut self, bus: &mut impl common::Bus, address: u16, value: u8) {
        bus.write_byte(u32::from(address), value);
        self.clk(1);
    }

    #[inline(always)]
    pub(crate) fn fetch_u8(&mut self, bus: &mut impl common::Bus) -> u8 {
        let value = bus.fetch_opcode_byte(u32::from(self.pc));
        self.pc = self.pc.wrapping_add(1);
        self.clk(1);
        value
    }

    #[inline(always)]
    pub(crate) fn fetch_u16(&mut self, bus: &mut impl common::Bus) -> u16 {
        let high = u16::from(self.fetch_u8(bus));
        let low = u16::from(self.fetch_u8(bus));
        (high << 8) | low
    }

    #[inline(always)]
    pub(crate) fn read_word(&mut self, bus: &mut impl common::Bus, address: u16) -> u16 {
        let high = u16::from(self.read_byte(bus, address));
        let low = u16::from(self.read_byte(bus, address.wrapping_add(1)));
        (high << 8) | low
    }

    #[inline(always)]
    pub(crate) fn write_word(&mut self, bus: &mut impl common::Bus, address: u16, value: u16) {
        self.write_byte(bus, address, (value >> 8) as u8);
        self.write_byte(bus, address.wrapping_add(1), value as u8);
    }

    #[inline(always)]
    fn read_word_untracked(&mut self, bus: &mut impl common::Bus, address: u16) -> u16 {
        let high = u16::from(bus.read_byte(u32::from(address)));
        let low = u16::from(bus.read_byte(u32::from(address.wrapping_add(1))));
        (high << 8) | low
    }

    #[inline(always)]
    pub(crate) fn direct_address(&mut self, bus: &mut impl common::Bus) -> u16 {
        (u16::from(self.dp) << 8) | u16::from(self.fetch_u8(bus))
    }

    #[inline(always)]
    pub(crate) fn finish_instruction(&mut self, cycle_start: i64, target_cycles: i32) {
        let consumed = cycle_start - self.cycles_remaining;
        let remaining = i64::from(target_cycles) - consumed;
        if remaining > 0 {
            self.cycles_remaining -= remaining;
        }
    }

    #[inline(always)]
    pub(crate) fn mark_s_loaded(&mut self) {
        self.nmi_armed = true;
    }

    pub(crate) fn push_byte(&mut self, bus: &mut impl common::Bus, stack: &mut u16, value: u8) {
        *stack = stack.wrapping_sub(1);
        self.write_byte(bus, *stack, value);
    }

    pub(crate) fn push_word(&mut self, bus: &mut impl common::Bus, stack: &mut u16, value: u16) {
        self.push_byte(bus, stack, value as u8);
        self.push_byte(bus, stack, (value >> 8) as u8);
    }

    pub(crate) fn pull_byte(&mut self, bus: &mut impl common::Bus, stack: &mut u16) -> u8 {
        let value = self.read_byte(bus, *stack);
        *stack = stack.wrapping_add(1);
        value
    }

    pub(crate) fn pull_word(&mut self, bus: &mut impl common::Bus, stack: &mut u16) -> u16 {
        let high = u16::from(self.pull_byte(bus, stack));
        let low = u16::from(self.pull_byte(bus, stack));
        (high << 8) | low
    }

    pub(crate) fn push_s_word(&mut self, bus: &mut impl common::Bus, value: u16) {
        let mut stack = self.s;
        self.push_word(bus, &mut stack, value);
        self.s = stack;
    }

    pub(crate) fn pull_s_byte(&mut self, bus: &mut impl common::Bus) -> u8 {
        let mut stack = self.s;
        let value = self.pull_byte(bus, &mut stack);
        self.s = stack;
        value
    }

    pub(crate) fn pull_s_word(&mut self, bus: &mut impl common::Bus) -> u16 {
        let mut stack = self.s;
        let value = self.pull_word(bus, &mut stack);
        self.s = stack;
        value
    }

    pub(crate) fn execute_one(&mut self, bus: &mut impl common::Bus) {
        let cycle_start = self.cycles_remaining;
        if let Some(address) = self.pending_extended_clear.take() {
            self.write_byte(bus, address, 0);
            let _ = self.clr8();
            self.finish_instruction(cycle_start, 3);
            return;
        }
        if let Some(cycles) = self.check_interrupts(bus) {
            self.finish_instruction(cycle_start, cycles);
            return;
        }

        let opcode = self.fetch_u8(bus);
        let target_cycles = match opcode {
            0x10 => {
                let page_opcode = self.fetch_u8(bus);
                self.execute_page2(page_opcode, bus)
            }
            0x11 => {
                let page_opcode = self.fetch_u8(bus);
                self.execute_page3(page_opcode, bus)
            }
            _ => self.execute_base(opcode, bus),
        };
        self.finish_instruction(cycle_start, target_cycles);
    }
}

impl Cpu6809 for M6809 {
    fn run_for(&mut self, cycles_to_run: u64, bus: &mut impl common::Bus) -> u64 {
        let start_cycle = bus.current_cycle();
        self.run_start_cycle = start_cycle;
        self.run_budget = cycles_to_run;
        self.cycles_remaining = cycles_to_run as i64;

        while self.cycles_remaining > 0 {
            if bus.has_nmi() && self.nmi_armed {
                self.pending_irq |= crate::PENDING_NMI;
            }
            if bus.has_irq() {
                self.pending_irq |= crate::PENDING_IRQ;
            } else {
                self.pending_irq &= !crate::PENDING_IRQ;
            }

            if self.halted {
                let interrupt_serviceable = self.pending_irq & crate::PENDING_NMI != 0
                    || self.pending_irq & PENDING_FIRQ != 0 && !self.flags.firq_mask
                    || self.pending_irq & crate::PENDING_IRQ != 0 && !self.flags.irq_mask;
                if self.cwai_waiting && !interrupt_serviceable || self.pending_irq == 0 {
                    let consumed = (cycles_to_run as i64 - self.cycles_remaining) as u64;
                    bus.set_current_cycle(start_cycle + consumed);
                    return consumed;
                }
                if !self.cwai_waiting {
                    self.halted = false;
                }
            }

            self.execute_one(bus);
            self.cycles_remaining -= bus.drain_wait_cycles();

            let consumed = cycles_to_run as i64 - self.cycles_remaining;
            bus.set_current_cycle(start_cycle + consumed as u64);

            if self.pending_extended_clear.is_some() {
                break;
            }

            bus.on_instruction_end();
            self.cycles_remaining -= bus.drain_wait_cycles();

            let consumed = cycles_to_run as i64 - self.cycles_remaining;
            bus.set_current_cycle(start_cycle + consumed as u64);

            if bus.reset_pending() || bus.cpu_should_yield() {
                break;
            }
        }

        let actual = (cycles_to_run as i64 - self.cycles_remaining) as u64;
        bus.set_current_cycle(start_cycle + actual);
        actual
    }

    fn reset(&mut self) {
        self.state = M6809State::default();
        self.clock_hz = self.clock_hz.max(1);
        self.flags.firq_mask = true;
        self.flags.irq_mask = true;
        self.halted = false;
        self.pending_irq = 0;
        self.nmi_armed = false;
        self.cwai_waiting = false;
        self.pending_extended_clear = None;
    }

    fn halted(&self) -> bool {
        self.halted
    }

    fn clock_hz(&self) -> u32 {
        self.clock_hz
    }

    fn set_clock_hz(&mut self, clock_hz: u32) {
        self.clock_hz = clock_hz.max(1);
    }

    fn pc(&self) -> u16 {
        self.pc
    }

    fn set_pc(&mut self, value: u16) {
        self.pc = value;
    }

    fn s(&self) -> u16 {
        self.s
    }

    fn set_s(&mut self, value: u16) {
        self.s = value;
        self.mark_s_loaded();
    }

    fn u(&self) -> u16 {
        self.u
    }

    fn set_u(&mut self, value: u16) {
        self.u = value;
    }

    fn x(&self) -> u16 {
        self.x
    }

    fn set_x(&mut self, value: u16) {
        self.x = value;
    }

    fn y(&self) -> u16 {
        self.y
    }

    fn set_y(&mut self, value: u16) {
        self.y = value;
    }

    fn a(&self) -> u8 {
        self.a
    }

    fn set_a(&mut self, value: u8) {
        self.a = value;
    }

    fn b(&self) -> u8 {
        self.b
    }

    fn set_b(&mut self, value: u8) {
        self.b = value;
    }

    fn d(&self) -> u16 {
        self.state.d()
    }

    fn set_d(&mut self, value: u16) {
        self.state.set_d(value);
    }

    fn dp(&self) -> u8 {
        self.dp
    }

    fn set_dp(&mut self, value: u8) {
        self.dp = value;
    }

    fn cc(&self) -> u8 {
        self.flags.compress()
    }

    fn set_cc(&mut self, value: u8) {
        self.flags.expand(value);
    }
}
