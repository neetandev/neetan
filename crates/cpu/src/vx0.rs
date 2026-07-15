//! Implements the NEC V20 (µPD70108) and V30 (µPD70116) emulation.
//!
//! The CPU model is selected via the const-generic parameter `MODEL`:
//! - [`V20_BUS`] - V20 bus mode (8-bit data bus, 4-byte
//!   prefetch queue with byte fetches). Used by the cycle-accurate
//!   verification suite against V20 SingleStepTests.
//! - [`V30_BUS`] - V30 bus mode (16-bit data bus, 6-byte prefetch
//!   queue with word fetches; single-byte first fetch on odd-target branch).
//!   We modelered the different BIU behavior, so the V30 core should also now
//!   be cycle count accurate. But we will validate this claim, once V30
//!   trace data is available from the SingleStepTests project.
//!
//! The instruction set, register layout, segmentation, address space, and IVT
//! layout are identical between V20 and V30; only the bus interface differs.

mod alu;
mod biu;
mod execute;
mod execute_0f;
mod execute_group;
mod flags;
mod interrupt;
mod modrm;
mod rep;
mod state;
mod string_ops;

use core::ops::{Deref, DerefMut};

use biu::{
    ADDRESS_MASK, BusPendingType, BusStatus, FetchState, MAX_QUEUE_SIZE, OperandSize, QueueOp,
    QueueType, TCycle, TaCycle, TransferSize, queue_size_for,
};
pub use biu::{V30BusPhase, V30QueueOpTrace, V30TaCycle};
use common::Cpu as _;
pub use flags::V30Flags;
pub use state::V30State;

use crate::{SegReg16, WordReg};

/// V20 bus mode constant. Selects an 8-bit data bus, 4-byte prefetch
/// queue with byte fetches. Used for cycle-accurate verification against V20
/// SingleStepTests.
pub const V20_BUS: u8 = 1;
/// V30 bus mode constant. Selects a 16-bit data bus, 6-byte prefetch
/// queue with word fetches (and single-byte first fetch on odd-target branch).
/// Used in production for PC-98 emulation.
pub const V30_BUS: u8 = 2;

/// Type alias for the V20 of the VX0 core.
pub type V20 = VX0<V20_BUS>;
/// Type alias for the V30 of the VX0 core.
pub type V30 = VX0<V30_BUS>;

/// Terminal fetch behavior for the current V20/V30 instruction.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum FinishState {
    #[default]
    Normal,
    NoTerminalFetch,
}

state_enum_codec!(FinishState {
    FinishState::Normal = 0,
    FinishState::NoTerminalFetch = 1,
});

/// NEC V20/V30 CPU emulator. The bus mode is selected by the const-generic
/// `MODEL`: see [`V20_BUS`] and [`V30_BUS`].
///
/// Use the type aliases [`V20`] and [`V30`] in code that wants to
/// pin a specific mode without specifying the const-generic argument.
pub struct VX0<const BUS: u8 = V30_BUS> {
    /// Embedded state for save/restore.
    pub state: V30State,

    cycles_remaining: i64,
    run_start_cycle: u64,
    run_budget: u64,
}

impl<const MODEL: u8> Deref for VX0<MODEL> {
    type Target = V30State;
    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl<const MODEL: u8> DerefMut for VX0<MODEL> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

impl<const MODEL: u8> Default for VX0<MODEL> {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(dead_code)]
impl<const MODEL: u8> VX0<MODEL> {
    /// Creates a new V30 CPU in its reset state.
    pub fn new() -> Self {
        let mut cpu = Self {
            state: V30State::default(),
            cycles_remaining: 0,
            run_start_cycle: 0,
            run_budget: 0,
        };
        cpu.reset();
        cpu
    }

    /// Spend `cycles` internal EU cycles. Each cycle advances the BIU
    /// T-state machine by one tick.
    #[inline(always)]
    pub(crate) fn clk(&mut self, bus: &mut impl common::Bus, cycles: i32) {
        if cycles > 0 {
            self.cycles(bus, cycles as u32);
        }
    }

    #[inline(always)]
    pub(crate) fn clk_modrm(
        &mut self,
        bus: &mut impl common::Bus,
        modrm: u8,
        reg_cycles: i32,
        mem_cycles: i32,
    ) {
        if modrm >= 0xC0 {
            self.clk(bus, reg_cycles);
        } else {
            self.clk(bus, mem_cycles);
        }
    }

    #[inline(always)]
    pub(crate) fn clk_modrm_word(
        &mut self,
        bus: &mut impl common::Bus,
        modrm: u8,
        reg_cycles: i32,
        mem_cycles: i32,
    ) {
        if modrm >= 0xC0 {
            self.clk(bus, reg_cycles);
        } else {
            self.clk(bus, mem_cycles);
        }
    }

    #[inline(always)]
    pub(crate) fn clk_modrm_alu(
        &mut self,
        bus: &mut impl common::Bus,
        modrm: u8,
        reg_cycles: i32,
        mem_cycles: i32,
    ) {
        if modrm >= 0xC0
            && self.seg_prefix
            && self.instruction_entry_queue_had_current_instruction()
        {
            return;
        }
        self.clk_modrm(bus, modrm, reg_cycles, mem_cycles);
    }

    #[inline(always)]
    pub(crate) fn clk_modrm_alu_word(
        &mut self,
        bus: &mut impl common::Bus,
        modrm: u8,
        reg_cycles: i32,
        mem_cycles: i32,
    ) {
        if modrm >= 0xC0
            && self.seg_prefix
            && self.instruction_entry_queue_had_current_instruction()
        {
            return;
        }
        self.clk_modrm_word(bus, modrm, reg_cycles, mem_cycles);
    }

    #[inline(always)]
    pub(crate) fn clk_ea_done(&mut self, bus: &mut impl common::Bus) {
        let cycles = match self.t_cycle {
            TCycle::T2 => 3,
            TCycle::T3 => 0,
            TCycle::Tinit | TCycle::Ti | TCycle::T1 | TCycle::Tw | TCycle::T4 => 1,
        };
        self.clk(bus, cycles);
    }

    #[inline(always)]
    pub(crate) fn clk_accumulator_immediate_byte_tail(&mut self, bus: &mut impl common::Bus) {
        let cycles = if self.seg_prefix && self.instruction_entry_queue_had_current_instruction() {
            1
        } else {
            2
        };
        self.clk(bus, cycles);
    }

    #[inline(always)]
    pub(crate) fn instruction_entry_queue_had_current_instruction(&self) -> bool {
        let instruction_len = self.ip.wrapping_sub(self.prev_ip) as usize;
        usize::from(self.instruction_entry_queue_bytes) >= instruction_len
    }

    #[inline(always)]
    pub(crate) fn fetch(&mut self, bus: &mut impl common::Bus) -> u8 {
        let value = self.biu_queue_read(bus, QueueType::Subsequent);
        self.ip = self.ip.wrapping_add(1);
        value
    }

    #[inline(always)]
    pub(crate) fn fetch_no_cycle(&mut self) -> u8 {
        let value = self.biu_queue_read_no_cycle();
        self.ip = self.ip.wrapping_add(1);
        value
    }

    #[inline(always)]
    pub(crate) fn fetch_first(&mut self, bus: &mut impl common::Bus) -> u8 {
        let value = self.biu_queue_read(bus, QueueType::First);
        self.ip = self.ip.wrapping_add(1);
        value
    }

    #[inline(always)]
    pub(crate) fn fetchword(&mut self, bus: &mut impl common::Bus) -> u16 {
        let low = self.fetch(bus) as u16;
        let high = self.fetch(bus) as u16;
        low | (high << 8)
    }

    #[inline(always)]
    pub(crate) fn default_segment(&self, seg: SegReg16) -> SegReg16 {
        if self.seg_prefix && matches!(seg, SegReg16::DS | SegReg16::SS) {
            self.prefix_seg
        } else {
            seg
        }
    }

    #[inline(always)]
    pub(crate) fn default_base(&self, seg: SegReg16) -> u32 {
        self.seg_base(self.default_segment(seg))
    }

    #[inline(always)]
    pub(crate) fn seg_base(&self, seg: SegReg16) -> u32 {
        (self.sregs[seg as usize] as u32) << 4
    }

    #[inline(always)]
    pub(crate) fn set_effective_address(&mut self, seg: SegReg16, offset: u16) {
        self.effective_address_segment = self.default_segment(seg);
        self.eo = offset;
        self.ea = self
            .seg_base(self.effective_address_segment)
            .wrapping_add(offset as u32)
            & ADDRESS_MASK;
    }

    /// Computes the physical address for a byte at `eo + delta`, wrapping
    /// the offset within the 16-bit segment boundary.
    #[inline(always)]
    pub(crate) fn seg_addr(&self, delta: u16) -> u32 {
        let seg_base = self.ea.wrapping_sub(self.eo as u32);
        seg_base.wrapping_add(self.eo.wrapping_add(delta) as u32) & ADDRESS_MASK
    }

    /// Reads a word from memory at `ea + delta`, wrapping the offset
    /// within the segment boundary (offset 0xFFFF wraps to 0x0000).
    #[inline(always)]
    pub(crate) fn seg_read_word_at(&mut self, bus: &mut impl common::Bus, delta: u16) -> u16 {
        let lo_address = self.seg_addr(delta);
        let hi_address = self.seg_addr(delta.wrapping_add(1));
        self.biu_read_word_physical_pair(bus, lo_address, hi_address)
    }

    /// Reads a word from memory at the current EA.
    #[inline(always)]
    pub(crate) fn seg_read_word(&mut self, bus: &mut impl common::Bus) -> u16 {
        self.seg_read_word_at(bus, 0)
    }

    /// Writes a word to memory at the current EA.
    #[inline(always)]
    pub(crate) fn seg_write_word(&mut self, bus: &mut impl common::Bus, value: u16) {
        let lo_address = self.seg_addr(0);
        let hi_address = self.seg_addr(1);
        self.biu_write_word_physical_pair(bus, lo_address, hi_address, value);
    }

    /// Reads a word from `base:offset`, where `base` is a pre-shifted 20-bit
    /// segment base. Wraps the offset within 16 bits.
    #[inline(always)]
    pub(crate) fn read_word_seg(
        &mut self,
        bus: &mut impl common::Bus,
        base: u32,
        offset: u16,
    ) -> u16 {
        let lo_address = base.wrapping_add(offset as u32) & ADDRESS_MASK;
        let hi_address = base.wrapping_add(offset.wrapping_add(1) as u32) & ADDRESS_MASK;
        self.biu_read_word_physical_pair(bus, lo_address, hi_address)
    }

    /// Writes a word to `base:offset`, where `base` is a pre-shifted 20-bit
    /// segment base. Wraps the offset within 16 bits.
    #[inline(always)]
    pub(crate) fn write_word_seg(
        &mut self,
        bus: &mut impl common::Bus,
        base: u32,
        offset: u16,
        value: u16,
    ) {
        let lo_address = base.wrapping_add(offset as u32) & ADDRESS_MASK;
        let hi_address = base.wrapping_add(offset.wrapping_add(1) as u32) & ADDRESS_MASK;
        self.biu_write_word_physical_pair(bus, lo_address, hi_address, value);
    }

    #[inline(always)]
    pub(crate) fn read_memory_byte(&mut self, bus: &mut impl common::Bus, address: u32) -> u8 {
        self.biu_read_u8_physical(bus, address)
    }

    #[inline(always)]
    pub(crate) fn write_memory_byte(
        &mut self,
        bus: &mut impl common::Bus,
        address: u32,
        value: u8,
    ) {
        self.biu_write_u8_physical(bus, address, value);
    }

    #[inline(always)]
    pub(crate) fn read_memory_word(&mut self, bus: &mut impl common::Bus, address: u32) -> u16 {
        self.biu_read_u16_physical(bus, address)
    }

    #[inline(always)]
    pub(crate) fn write_memory_word(
        &mut self,
        bus: &mut impl common::Bus,
        address: u32,
        value: u16,
    ) {
        self.biu_write_u16_physical(bus, address, value);
    }

    #[inline(always)]
    pub(crate) fn read_io_byte(&mut self, bus: &mut impl common::Bus, port: u16) -> u8 {
        self.biu_io_read_u8(bus, port)
    }

    #[inline(always)]
    pub(crate) fn write_io_byte(&mut self, bus: &mut impl common::Bus, port: u16, value: u8) {
        self.biu_io_write_u8(bus, port, value);
    }

    #[inline(always)]
    pub(crate) fn read_io_word(&mut self, bus: &mut impl common::Bus, port: u16) -> u16 {
        self.biu_io_read_u16(bus, port)
    }

    #[inline(always)]
    pub(crate) fn write_io_word(&mut self, bus: &mut impl common::Bus, port: u16, value: u16) {
        self.biu_io_write_u16(bus, port, value);
    }

    /// Reset queue and pipeline state without advancing the bus. Called by
    /// the CPU trait's `set_cs`/`set_ip` which have no bus access.
    fn flush_prefetch_queue(&mut self) {
        self.prefetch_ip = self.ip;
        self.queue_flush();
        self.queue_op = QueueOp::Idle;
        self.last_queue_op = QueueOp::Idle;
        self.last_queue_byte = 0;
        self.t_cycle = TCycle::Ti;
        self.ta_cycle = TaCycle::Td;
        self.bus_status = BusStatus::Passive;
        self.bus_status_latch = BusStatus::Passive;
        self.pl_status = BusStatus::Passive;
        self.bus_pending = BusPendingType::None;
        self.fetch_state = FetchState::Normal;
    }

    pub(crate) fn flush_and_fetch(&mut self) {
        self.biu_queue_flush();
    }

    pub(crate) fn flush_and_fetch_early(&mut self) {
        self.prefetch_ip = self.ip;
        self.biu_queue_flush_early();
    }

    pub(crate) fn restart_fetch_after_delay(&mut self, bus: &mut impl common::Bus, delay: i32) {
        self.biu_fetch_suspend(bus);
        self.biu_complete_current_bus_for_eu();
        self.clk(bus, delay);
        self.prefetch_ip = self.ip;
        self.biu_queue_flush_and_start_code_fetch_for_eu();
    }

    fn finish_step_timing(&mut self, bus: &mut impl common::Bus) {
        if self.halted || self.rep_state.active {
            return;
        }
        match self.finish_state {
            FinishState::Normal => self.biu_fetch_next(bus),
            FinishState::NoTerminalFetch => {}
        }
    }

    /// Installs prefetched instruction bytes and advances the BIU fetch pointer.
    pub fn install_prefetch_queue(&mut self, bytes: &[u8]) {
        assert!(
            bytes.len() <= queue_size_for(MODEL),
            "V30 prefetch queue overflow"
        );
        self.instruction_queue = [0; MAX_QUEUE_SIZE];
        self.instruction_queue[..bytes.len()].copy_from_slice(bytes);
        self.instruction_queue_len = bytes.len();
        self.instruction_preload = None;
        self.prefetch_ip = self.ip.wrapping_add(bytes.len() as u16);
        self.queue_op = QueueOp::Idle;
        self.last_queue_op = QueueOp::Idle;
        self.last_queue_byte = 0;
        self.fetch_state = FetchState::Normal;
        self.t_cycle = TCycle::Ti;
        self.ta_cycle = TaCycle::Td;
        self.bus_status = BusStatus::Passive;
        self.bus_status_latch = BusStatus::Passive;
        self.pl_status = BusStatus::Passive;
        self.bus_pending = BusPendingType::None;
    }

    /// Loads complete CPU state without resetting execution or BIU latches.
    pub fn load_state(&mut self, state: &V30State) {
        self.state = state.clone();
    }

    /// Clones the authoritative state at a resumable execution boundary.
    pub fn capture_state(&self) -> V30State {
        self.state.clone()
    }

    /// Validates and replaces the authoritative state transactionally.
    pub fn restore_state(
        &mut self,
        state: V30State,
    ) -> Result<(), save_state::StateValidationError> {
        save_state::restore_root(self, state, &MODEL)
    }

    pub(crate) fn push(&mut self, bus: &mut impl common::Bus, value: u16) {
        let sp = self.regs.word(WordReg::SP).wrapping_sub(2);
        self.regs.set_word(WordReg::SP, sp);
        let base = self.seg_base(SegReg16::SS);
        self.write_word_seg(bus, base, sp, value);
    }

    pub(crate) fn pop(&mut self, bus: &mut impl common::Bus) -> u16 {
        let sp = self.regs.word(WordReg::SP);
        let base = self.seg_base(SegReg16::SS);
        let value = self.read_word_seg(bus, base, sp);
        self.regs.set_word(WordReg::SP, sp.wrapping_add(2));
        value
    }

    fn execute_one(&mut self, bus: &mut impl common::Bus) {
        self.prev_ip = self.ip;
        self.instruction_entry_queue_bytes =
            (self.queue_len() + usize::from(self.instruction_preload.is_some())) as u8;

        if self.pending_irq != 0 {
            self.check_interrupts(bus);
        }
        if self.no_interrupt > 0 {
            self.no_interrupt -= 1;
        }
        if self.inhibit_all > 0 {
            self.inhibit_all -= 1;
        }

        self.seg_prefix = false;
        self.rep_state.prefix = false;
        self.finish_state = FinishState::Normal;

        if self.rep_state.active {
            self.continue_rep(bus);
        } else {
            let mut opcode = self.fetch_first(bus);
            loop {
                match opcode {
                    0x26 => {
                        self.seg_prefix = true;
                        self.prefix_seg = SegReg16::ES;
                        self.clk(bus, 2);
                        opcode = self.fetch(bus);
                    }
                    0x2E => {
                        self.seg_prefix = true;
                        self.prefix_seg = SegReg16::CS;
                        self.clk(bus, 2);
                        opcode = self.fetch(bus);
                    }
                    0x36 => {
                        self.seg_prefix = true;
                        self.prefix_seg = SegReg16::SS;
                        self.clk(bus, 2);
                        opcode = self.fetch(bus);
                    }
                    0x3E => {
                        self.seg_prefix = true;
                        self.prefix_seg = SegReg16::DS;
                        self.clk(bus, 2);
                        opcode = self.fetch(bus);
                    }
                    0xF0 => {
                        self.inhibit_all = 1;
                        self.clk(bus, 2);
                        opcode = self.fetch(bus);
                    }
                    _ => {
                        self.opcode_start_ip = self.ip.wrapping_sub(1);
                        self.dispatch(opcode, bus);
                        break;
                    }
                }
            }
        }
    }

    /// Executes exactly one logical instruction (should only be used in tests).
    pub fn step(&mut self, bus: &mut impl common::Bus) {
        self.cycles_remaining = i64::MAX;
        self.execute_one(bus);
        self.finish_step_timing(bus);
    }

    /// Returns the number of cycles consumed by the last `step()` call.
    pub fn cycles_consumed(&self) -> u64 {
        (i64::MAX - self.cycles_remaining) as u64
    }

    /// Returns the last computed effective address (for alignment checks).
    pub fn last_ea(&self) -> u32 {
        self.ea
    }

    /// Signals a maskable interrupt request (IRQ).
    pub fn signal_irq(&mut self) {
        self.pending_irq |= crate::PENDING_IRQ;
    }

    /// Signals a non-maskable interrupt (NMI).
    pub fn signal_nmi(&mut self) {
        self.pending_irq |= crate::PENDING_NMI;
    }
}

impl<const MODEL: u8> common::Cpu for VX0<MODEL> {
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
                if self.flags.if_flag {
                    if bus.has_irq() {
                        self.pending_irq |= crate::PENDING_IRQ;
                    }
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
            if !self.rep_state.active {
                self.finish_step_timing(bus);
            }
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

            if bus.reset_pending() {
                break;
            }
            if bus.cpu_should_yield() {
                break;
            }
        }

        let actual = (cycles_to_run as i64 - self.cycles_remaining) as u64;
        bus.set_current_cycle(start_cycle + actual);
        actual
    }

    fn reset(&mut self) {
        self.state = V30State::default();
        self.sregs[SegReg16::CS as usize] = 0xFFFF;
        self.prev_ip = 0;
        self.opcode_start_ip = 0;
        self.halted = false;
        self.pending_irq = 0;
        self.no_interrupt = 0;
        self.inhibit_all = 0;
        self.rep_state.active = false;
        self.rep_state.restart_ip = 0;
        self.rep_state.prefix = false;
        self.seg_prefix = false;
        self.ea = 0;
        self.eo = 0;
        self.effective_address_segment = SegReg16::DS;
        self.instruction_queue = [0; MAX_QUEUE_SIZE];
        self.instruction_queue_len = 0;
        self.instruction_preload = None;
        self.instruction_entry_queue_bytes = 0;
        self.prefetch_ip = self.ip;
        self.queue_op = QueueOp::Idle;
        self.last_queue_op = QueueOp::Idle;
        self.last_queue_byte = 0;
        self.t_cycle = TCycle::Ti;
        self.ta_cycle = TaCycle::Td;
        self.bus_status = BusStatus::Passive;
        self.bus_status_latch = BusStatus::Passive;
        self.pl_status = BusStatus::Passive;
        self.bus_pending = BusPendingType::None;
        self.fetch_state = FetchState::Normal;
        self.transfer_size = TransferSize::Byte;
        self.operand_size = OperandSize::Operand8;
        self.transfer_n = 1;
        self.final_transfer = false;
        self.address_bus = 0;
        self.address_latch = 0;
        self.data_bus = 0;
        self.bhe = false;
    }

    fn halted(&self) -> bool {
        self.halted
    }

    fn ax(&self) -> u16 {
        self.state.ax()
    }

    fn set_ax(&mut self, v: u16) {
        self.state.set_ax(v);
    }

    fn bx(&self) -> u16 {
        self.state.bx()
    }

    fn set_bx(&mut self, v: u16) {
        self.state.set_bx(v);
    }

    fn cx(&self) -> u16 {
        self.state.cx()
    }

    fn set_cx(&mut self, v: u16) {
        self.state.set_cx(v);
    }

    fn dx(&self) -> u16 {
        self.state.dx()
    }

    fn set_dx(&mut self, v: u16) {
        self.state.set_dx(v);
    }

    fn sp(&self) -> u16 {
        self.state.sp()
    }

    fn set_sp(&mut self, v: u16) {
        self.state.set_sp(v);
    }

    fn bp(&self) -> u16 {
        self.state.bp()
    }

    fn set_bp(&mut self, v: u16) {
        self.state.set_bp(v);
    }

    fn si(&self) -> u16 {
        self.state.si()
    }

    fn set_si(&mut self, v: u16) {
        self.state.set_si(v);
    }

    fn di(&self) -> u16 {
        self.state.di()
    }

    fn set_di(&mut self, v: u16) {
        self.state.set_di(v);
    }

    fn es(&self) -> u16 {
        self.state.es()
    }

    fn set_es(&mut self, v: u16) {
        self.state.set_es(v);
    }

    fn cs(&self) -> u16 {
        self.state.cs()
    }

    fn set_cs(&mut self, v: u16) {
        self.state.set_cs(v);
        self.flush_prefetch_queue();
    }

    fn ss(&self) -> u16 {
        self.state.ss()
    }

    fn set_ss(&mut self, v: u16) {
        self.state.set_ss(v);
    }

    fn ds(&self) -> u16 {
        self.state.ds()
    }

    fn set_ds(&mut self, v: u16) {
        self.state.set_ds(v);
    }

    fn ip(&self) -> u16 {
        self.state.ip
    }

    fn set_ip(&mut self, v: u16) {
        self.state.ip = v;
        self.flush_prefetch_queue();
    }

    fn flags(&self) -> u16 {
        self.state.compressed_flags()
    }

    fn set_flags(&mut self, v: u16) {
        self.state.set_compressed_flags(v);
    }

    fn cpu_type(&self) -> common::CpuType {
        common::CpuType::V30
    }

    fn load_segment_real_mode(&mut self, seg: common::SegmentRegister, selector: u16) {
        match seg {
            common::SegmentRegister::ES => self.state.set_es(selector),
            common::SegmentRegister::CS => self.state.set_cs(selector),
            common::SegmentRegister::SS => self.state.set_ss(selector),
            common::SegmentRegister::DS => self.state.set_ds(selector),
        }
    }

    fn segment_base(&self, seg: common::SegmentRegister) -> u32 {
        match seg {
            common::SegmentRegister::ES => u32::from(self.state.es()) << 4,
            common::SegmentRegister::CS => u32::from(self.state.cs()) << 4,
            common::SegmentRegister::SS => u32::from(self.state.ss()) << 4,
            common::SegmentRegister::DS => u32::from(self.state.ds()) << 4,
        }
    }
}

impl<const MODEL: u8> save_state::AfterRestore for VX0<MODEL> {
    fn after_restore(&mut self) {
        self.cycles_remaining = 0;
        self.run_start_cycle = 0;
        self.run_budget = 0;
    }
}

impl<const MODEL: u8> save_state::RestoreTarget for VX0<MODEL> {
    type State = V30State;
    type ValidationContext = u8;

    fn replace_state(&mut self, state: Self::State) {
        self.state = state;
    }
}
