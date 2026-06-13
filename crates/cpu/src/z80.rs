//! Implements the Zilog Z80 emulation.
//!
//! Following references were used to write the emulator:
//!
//! - Zilog, "Z80 CPU User Manual".
//! - MAME Z80 emulator (`devices/cpu/z80/`).

mod alu;
mod execute;
mod execute_cb;
mod execute_ed;
mod execute_xy;
mod flags;
mod interrupt;
mod state;

use core::ops::{Deref, DerefMut};

use common::CpuZ80;
pub use flags::Z80Flags;
pub use state::Z80State;

/// Default Z80 clock frequency used by verification tests.
pub const DEFAULT_CLOCK_HZ: u32 = 4_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IndexMode {
    HL,
    IX,
    IY,
}

/// Zilog Z80 CPU emulator.
pub struct Z80 {
    /// Embedded state for save/restore.
    pub state: Z80State,

    clock_hz: u32,
    halted: bool,
    pending_irq: u8,
    cycles_remaining: i64,
    run_start_cycle: u64,
    run_budget: u64,
    q_latch: bool,
}

impl Deref for Z80 {
    type Target = Z80State;

    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl DerefMut for Z80 {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

impl Default for Z80 {
    fn default() -> Self {
        Self::new(DEFAULT_CLOCK_HZ)
    }
}

impl Z80 {
    /// Creates a new Z80 CPU in its reset state.
    pub fn new(clock_hz: u32) -> Self {
        let mut cpu = Self {
            state: Z80State::default(),
            clock_hz,
            halted: false,
            pending_irq: 0,
            cycles_remaining: 0,
            run_start_cycle: 0,
            run_budget: 0,
            q_latch: false,
        };
        cpu.reset();
        cpu
    }

    #[inline(always)]
    pub(crate) fn clk(&mut self, cycles: i32) {
        self.cycles_remaining -= i64::from(cycles);
    }

    #[inline(always)]
    pub(crate) fn read_byte(&mut self, bus: &mut impl common::Bus, address: u16) -> u8 {
        let value = bus.read_byte(u32::from(address));
        self.clk(3);
        value
    }

    #[inline(always)]
    pub(crate) fn write_byte(&mut self, bus: &mut impl common::Bus, address: u16, value: u8) {
        bus.write_byte(u32::from(address), value);
        self.clk(3);
    }

    #[inline(always)]
    pub(crate) fn fetch_u8(&mut self, bus: &mut impl common::Bus) -> u8 {
        let value = bus.read_byte(u32::from(self.pc));
        self.pc = self.pc.wrapping_add(1);
        self.clk(3);
        value
    }

    #[inline(always)]
    pub(crate) fn fetch_u16(&mut self, bus: &mut impl common::Bus) -> u16 {
        let low = u16::from(self.fetch_u8(bus));
        let high = u16::from(self.fetch_u8(bus));
        low | (high << 8)
    }

    #[inline(always)]
    pub(crate) fn fetch_m1(&mut self, bus: &mut impl common::Bus) -> u8 {
        let value = bus.fetch_opcode_byte(u32::from(self.pc));
        self.pc = self.pc.wrapping_add(1);
        self.increment_r();
        self.clk(4);
        value
    }

    #[inline(always)]
    pub(crate) fn read_port(&mut self, bus: &mut impl common::Bus, port: u16) -> u8 {
        let value = bus.io_read_byte(port);
        self.clk(4);
        value
    }

    #[inline(always)]
    pub(crate) fn write_port(&mut self, bus: &mut impl common::Bus, port: u16, value: u8) {
        bus.io_write_byte(port, value);
        self.clk(4);
    }

    pub(crate) fn push(&mut self, bus: &mut impl common::Bus, value: u16) {
        self.sp = self.sp.wrapping_sub(1);
        self.write_byte(bus, self.sp, (value >> 8) as u8);
        self.sp = self.sp.wrapping_sub(1);
        self.write_byte(bus, self.sp, value as u8);
    }

    pub(crate) fn pop(&mut self, bus: &mut impl common::Bus) -> u16 {
        let low = u16::from(self.read_byte(bus, self.sp));
        self.sp = self.sp.wrapping_add(1);
        let high = u16::from(self.read_byte(bus, self.sp));
        self.sp = self.sp.wrapping_add(1);
        low | (high << 8)
    }

    #[inline(always)]
    pub(crate) fn set_q_latch(&mut self, value: bool) {
        self.q_latch = value;
    }

    pub(crate) fn finish_q(&mut self) {
        self.q = if self.q_latch {
            self.flags.compress()
        } else {
            0
        };
    }

    pub(crate) fn execute_one(&mut self, bus: &mut impl common::Bus) {
        if self.pending_irq != 0 {
            self.check_interrupts(bus);
        }
        self.ei = 0;
        self.p = 0;
        self.set_q_latch(false);

        let mut mode = IndexMode::HL;
        let mut xy_prefix = false;
        let mut opcode = self.fetch_m1(bus);
        loop {
            match opcode {
                0xDD => {
                    mode = IndexMode::IX;
                    xy_prefix = true;
                    opcode = self.fetch_m1(bus);
                }
                0xFD => {
                    mode = IndexMode::IY;
                    xy_prefix = true;
                    opcode = self.fetch_m1(bus);
                }
                _ => break,
            }
        }

        match opcode {
            0xCB if mode != IndexMode::HL => {
                let displacement = self.fetch_u8(bus) as i8;
                let address = self
                    .current_hl(mode)
                    .wrapping_add_signed(i16::from(displacement));
                self.wz = address;
                let cb_opcode = self.fetch_u8(bus);
                self.clk(2);
                self.execute_cb_indexed(cb_opcode, address, bus);
            }
            0xCB => {
                let cb_opcode = self.fetch_m1(bus);
                self.execute_cb(cb_opcode, bus);
            }
            0xED => {
                let ed_opcode = self.fetch_m1(bus);
                self.execute_ed(ed_opcode, bus);
            }
            _ => self.execute_base(opcode, mode, xy_prefix, bus),
        }

        self.finish_q();
    }

    /// Loads CPU state from a snapshot, resetting runtime flags.
    pub fn load_state(&mut self, state: &Z80State) {
        self.state = state.clone();
        self.halted = false;
        self.pending_irq = 0;
        self.q_latch = false;
    }

    /// Executes exactly one logical instruction (should only be used in tests).
    pub fn step(&mut self, bus: &mut impl common::Bus) {
        let start_cycle = bus.current_cycle();
        self.cycles_remaining = i64::MAX;
        self.execute_one(bus);
        self.cycles_remaining -= bus.drain_wait_cycles();
        bus.set_current_cycle(start_cycle + self.cycles_consumed());
    }

    /// Returns the number of T-states consumed by the last `step()` call.
    pub fn cycles_consumed(&self) -> u64 {
        (i64::MAX - self.cycles_remaining) as u64
    }
}

impl CpuZ80 for Z80 {
    fn run_for(&mut self, cycles_to_run: u64, bus: &mut impl common::Bus) -> u64 {
        let start_cycle = bus.current_cycle();
        self.run_start_cycle = start_cycle;
        self.run_budget = cycles_to_run;
        self.cycles_remaining = cycles_to_run as i64;

        while self.cycles_remaining > 0 {
            if self.halted {
                if bus.has_nmi() {
                    self.pending_irq |= crate::PENDING_NMI;
                }
                if self.iff1 && bus.has_irq() {
                    self.pending_irq |= crate::PENDING_IRQ;
                } else {
                    self.pending_irq &= !crate::PENDING_IRQ;
                }
                if self.pending_irq != 0 {
                    self.halted = false;
                } else {
                    let consumed = (cycles_to_run as i64 - self.cycles_remaining) as u64;
                    bus.set_current_cycle(start_cycle + consumed);
                    return consumed;
                }
            }

            self.execute_one(bus);
            self.cycles_remaining -= bus.drain_wait_cycles();

            let consumed = cycles_to_run as i64 - self.cycles_remaining;
            bus.set_current_cycle(start_cycle + consumed as u64);

            if bus.has_nmi() {
                self.pending_irq |= crate::PENDING_NMI;
            }
            if bus.has_irq() {
                self.pending_irq |= crate::PENDING_IRQ;
            } else {
                self.pending_irq &= !crate::PENDING_IRQ;
            }

            if bus.reset_pending() || bus.cpu_should_yield() {
                break;
            }
        }

        let actual = (cycles_to_run as i64 - self.cycles_remaining) as u64;
        bus.set_current_cycle(start_cycle + actual);
        actual
    }

    fn reset(&mut self) {
        self.state = Z80State::default();
        self.clock_hz = self.clock_hz.max(1);
        self.halted = false;
        self.pending_irq = 0;
        self.q_latch = false;
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
        self.state.pc
    }

    fn set_pc(&mut self, value: u16) {
        self.state.pc = value;
    }

    fn sp(&self) -> u16 {
        self.state.sp
    }

    fn set_sp(&mut self, value: u16) {
        self.state.sp = value;
    }

    fn af(&self) -> u16 {
        self.state.af()
    }

    fn set_af(&mut self, value: u16) {
        self.state.set_af(value);
    }

    fn bc(&self) -> u16 {
        self.state.bc()
    }

    fn set_bc(&mut self, value: u16) {
        self.state.set_bc(value);
    }

    fn de(&self) -> u16 {
        self.state.de()
    }

    fn set_de(&mut self, value: u16) {
        self.state.set_de(value);
    }

    fn hl(&self) -> u16 {
        self.state.hl()
    }

    fn set_hl(&mut self, value: u16) {
        self.state.set_hl(value);
    }

    fn ix(&self) -> u16 {
        self.state.ix
    }

    fn set_ix(&mut self, value: u16) {
        self.state.ix = value;
    }

    fn iy(&self) -> u16 {
        self.state.iy
    }

    fn set_iy(&mut self, value: u16) {
        self.state.iy = value;
    }

    fn i(&self) -> u8 {
        self.state.i
    }

    fn set_i(&mut self, value: u8) {
        self.state.i = value;
    }

    fn r(&self) -> u8 {
        self.state.r()
    }

    fn set_r(&mut self, value: u8) {
        self.state.set_r(value);
    }

    fn iff1(&self) -> bool {
        self.state.iff1
    }

    fn set_iff1(&mut self, value: bool) {
        self.state.iff1 = value;
    }

    fn iff2(&self) -> bool {
        self.state.iff2
    }

    fn set_iff2(&mut self, value: bool) {
        self.state.iff2 = value;
    }

    fn im(&self) -> u8 {
        self.state.im
    }

    fn set_im(&mut self, value: u8) {
        self.state.im = value & 0x03;
    }
}
