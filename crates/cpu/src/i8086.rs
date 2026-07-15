//! Implements the Intel 8086 CPU emulator.

mod alu;
mod biu;
mod execute;
mod execute_group;
mod flags;
mod interrupt;
mod modrm;
mod muldiv_timing;
mod rep;
mod state;
mod string_ops;

use core::ops::{Deref, DerefMut};

use biu::{
    ADDRESS_MASK, BusPendingType, BusStatus, FetchState, OperandSize, QUEUE_SIZE, QueueOp,
    QueueType, TCycle, TaCycle, TransferSize,
};
use common::Cpu as _;
pub use flags::I8086Flags;
use rep::RepType;
pub use state::I8086State;

use crate::{SegReg16, WordReg};

/// PC-9801F 8086 CPU clock at 5 MHz.
pub const PC9801F_CPU_CLOCK_5MHZ: u32 = 5_000_000;

/// PC-9801F 8086 CPU clock at 8 MHz.
pub const PC9801F_CPU_CLOCK_8MHZ: u32 = 8_000_000;

/// Intel 8086 CPU emulator.
pub struct I8086 {
    /// Embedded state for save/restore.
    pub state: I8086State,
    cycles_remaining: i64,
    run_start_cycle: u64,
    run_budget: u64,
}

/// Terminal fetch behavior for the current 8086 instruction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum StepFinishCycle {
    #[default]
    WithFetchCycle,
    PreloadedOnly,
    TerminalWritebackFetchCycle,
    TerminalWritebackRni,
    TerminalWritebackInlineCommit,
}

state_enum_codec!(StepFinishCycle {
    StepFinishCycle::WithFetchCycle = 0,
    StepFinishCycle::PreloadedOnly = 1,
    StepFinishCycle::TerminalWritebackFetchCycle = 2,
    StepFinishCycle::TerminalWritebackRni = 3,
    StepFinishCycle::TerminalWritebackInlineCommit = 4,
});

impl Deref for I8086 {
    type Target = I8086State;
    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl DerefMut for I8086 {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

impl Default for I8086 {
    fn default() -> Self {
        Self::new()
    }
}

impl I8086 {
    /// Creates a new I8086 CPU in its reset state.
    pub fn new() -> Self {
        let mut cpu = Self {
            state: I8086State::default(),
            cycles_remaining: 0,
            run_start_cycle: 0,
            run_budget: 0,
        };
        cpu.reset();
        cpu
    }

    /// Spend `cycles` internal EU cycles (not bus transfers). Each cycle
    /// advances the BIU T-state machine by one tick.
    #[inline(always)]
    fn clk(&mut self, bus: &mut impl common::Bus, cycles: i32) {
        if cycles > 0 {
            self.cycles(bus, cycles as u32);
        }
    }

    #[inline(always)]
    fn clk_modrm(
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
    fn clk_modrm_word(
        &mut self,
        bus: &mut impl common::Bus,
        modrm: u8,
        reg_cycles: i32,
        mem_cycles: i32,
    ) {
        if modrm >= 0xC0 {
            self.clk(bus, reg_cycles);
        } else {
            // 8086 handles word alignment at the T-state level via
            // `biu_read_u16`/`biu_write_u16`, so no additional penalty is
            // added here.
            self.clk(bus, mem_cycles);
        }
    }

    #[inline(always)]
    fn clk_muldiv_modrm(&mut self, bus: &mut impl common::Bus, modrm: u8, base_cycles: i32) {
        if modrm >= 0xC0 {
            self.clk(bus, base_cycles + 1);
        } else {
            self.clk(bus, base_cycles);
        }
    }

    #[inline(always)]
    fn clk_eaload(&mut self, bus: &mut impl common::Bus) {
        self.clk(bus, 2);
    }

    #[inline(always)]
    fn clk_eadone(&mut self, bus: &mut impl common::Bus) {
        self.clk(bus, 2);
    }

    #[inline(always)]
    fn fetch(&mut self, bus: &mut impl common::Bus) -> u8 {
        let value = self.biu_queue_read(bus, QueueType::Subsequent);
        self.ip = self.ip.wrapping_add(1);
        value
    }

    #[inline(always)]
    fn fetch_interrupt_vector(&mut self, bus: &mut impl common::Bus) -> u8 {
        if !self.instruction_entry_queue_full() {
            return self.fetch(bus);
        }

        let value = self.queue_pop();
        self.queue_op = QueueOp::Subsequent;
        self.cycle(bus);
        self.biu_fetch_on_queue_read();
        self.cycle(bus);
        self.ip = self.ip.wrapping_add(1);
        value
    }

    #[inline(always)]
    fn fetch_first(&mut self, bus: &mut impl common::Bus) -> u8 {
        let value = self.biu_queue_read(bus, QueueType::First);
        self.ip = self.ip.wrapping_add(1);
        value
    }

    #[inline(always)]
    fn fetchword(&mut self, bus: &mut impl common::Bus) -> u16 {
        let low = self.fetch(bus) as u16;
        let high = self.fetch(bus) as u16;
        low | (high << 8)
    }

    #[inline(always)]
    fn default_segment(&self, seg: SegReg16) -> SegReg16 {
        if self.seg_prefix && matches!(seg, SegReg16::DS | SegReg16::SS) {
            self.prefix_seg
        } else {
            seg
        }
    }

    #[inline(always)]
    fn default_base(&self, seg: SegReg16) -> u32 {
        self.seg_base(self.default_segment(seg))
    }

    #[inline(always)]
    fn seg_base(&self, seg: SegReg16) -> u32 {
        (self.sregs[seg as usize] as u32) << 4
    }

    #[inline(always)]
    fn set_effective_address(&mut self, seg: SegReg16, offset: u16) {
        self.effective_address_segment = self.default_segment(seg);
        self.eo = offset;
        self.ea = self
            .seg_base(self.effective_address_segment)
            .wrapping_add(offset as u32)
            & ADDRESS_MASK;
    }

    /// Reads a word from memory at `ea + delta`, wrapping the offset
    /// within the segment boundary (offset 0xFFFF wraps to 0x0000).
    #[inline(always)]
    fn seg_read_word_at(&mut self, bus: &mut impl common::Bus, delta: u16) -> u16 {
        let offset = self.eo.wrapping_add(delta);
        self.biu_read_u16(bus, self.effective_address_segment, offset)
    }

    /// Reads a word from memory at the current EA, wrapping the offset
    /// within the segment boundary (offset 0xFFFF wraps to 0x0000).
    #[inline(always)]
    fn seg_read_word(&mut self, bus: &mut impl common::Bus) -> u16 {
        self.seg_read_word_at(bus, 0)
    }

    /// Writes a word to memory at the current EA, wrapping the offset
    /// within the segment boundary (offset 0xFFFF wraps to 0x0000).
    #[inline(always)]
    fn seg_write_word(&mut self, bus: &mut impl common::Bus, value: u16) {
        let offset = self.eo;
        self.biu_write_u16(bus, self.effective_address_segment, offset, value);
    }

    /// Reads a word from `seg:offset`, wrapping the offset within 16 bits.
    #[inline(always)]
    fn read_word_seg(&mut self, bus: &mut impl common::Bus, seg: SegReg16, offset: u16) -> u16 {
        self.biu_read_u16(bus, seg, offset)
    }

    /// Writes a word to `seg:offset`, wrapping the offset within 16 bits.
    #[inline(always)]
    fn write_word_seg(
        &mut self,
        bus: &mut impl common::Bus,
        seg: SegReg16,
        offset: u16,
        value: u16,
    ) {
        self.biu_write_u16(bus, seg, offset, value);
    }

    #[inline(always)]
    fn read_memory_byte(&mut self, bus: &mut impl common::Bus, address: u32) -> u8 {
        self.biu_read_u8_physical(bus, address)
    }

    #[inline(always)]
    fn write_memory_byte(&mut self, bus: &mut impl common::Bus, address: u32, value: u8) {
        self.biu_write_u8_physical(bus, address, value);
    }

    #[inline(always)]
    fn read_memory_word(&mut self, bus: &mut impl common::Bus, address: u32) -> u16 {
        self.biu_read_u16_physical(bus, address)
    }

    #[inline(always)]
    fn read_io_byte(&mut self, bus: &mut impl common::Bus, port: u16) -> u8 {
        self.biu_io_read_u8(bus, port)
    }

    #[inline(always)]
    fn write_io_byte(&mut self, bus: &mut impl common::Bus, port: u16, value: u8) {
        self.biu_io_write_u8(bus, port, value);
    }

    #[inline(always)]
    fn read_io_word(&mut self, bus: &mut impl common::Bus, port: u16) -> u16 {
        self.biu_io_read_u16(bus, port)
    }

    #[inline(always)]
    fn write_io_word(&mut self, bus: &mut impl common::Bus, port: u16, value: u16) {
        self.biu_io_write_u16(bus, port, value);
    }

    /// Reset queue and pipeline state without advancing the bus. Called by
    /// the CPU trait's `set_cs`/`set_ip` which have no bus access.
    fn flush_prefetch_queue(&mut self) {
        self.prefetch_ip = self.ip;
        self.queue_flush();
        self.step_finish_cycle = StepFinishCycle::WithFetchCycle;
        self.nx = false;
        self.rni = false;
        self.queue_op = QueueOp::Idle;
        self.last_queue_op = QueueOp::Idle;
        self.t_cycle = TCycle::Ti;
        self.ta_cycle = TaCycle::Td;
        self.bus_status = BusStatus::Passive;
        self.bus_status_latch = BusStatus::Passive;
        self.pl_status = BusStatus::Passive;
        self.bus_pending = BusPendingType::None;
        self.fetch_state = FetchState::Normal;
    }

    /// Flush the queue and start a fetch cycle via the BIU. Used by the EU
    /// after jumps, interrupts, and RET-class instructions.
    fn flush_and_fetch(&mut self, bus: &mut impl common::Bus) {
        self.prefetch_ip = self.ip;
        self.biu_queue_flush(bus);
    }

    fn flush_and_fetch_early(&mut self, bus: &mut impl common::Bus) {
        self.prefetch_ip = self.ip;
        self.biu_queue_flush_early(bus);
    }

    fn set_ip_and_flush(&mut self, bus: &mut impl common::Bus, ip: u16) {
        self.ip = ip;
        self.flush_and_fetch(bus);
    }

    fn set_cs_ip_and_flush(&mut self, bus: &mut impl common::Bus, cs: u16, ip: u16) {
        self.sregs[SegReg16::CS as usize] = cs;
        self.ip = ip;
        self.flush_and_fetch(bus);
    }

    fn finish_step_timing(&mut self, bus: &mut impl common::Bus) {
        if self.halted || self.rep_active {
            self.step_finish_cycle = StepFinishCycle::WithFetchCycle;
            return;
        }

        match self.step_finish_cycle {
            StepFinishCycle::WithFetchCycle => self.biu_fetch_next(bus),
            StepFinishCycle::PreloadedOnly => self.biu_preload_next(bus),
            StepFinishCycle::TerminalWritebackFetchCycle => {
                self.finish_terminal_writeback_with_fetch(bus);
            }
            StepFinishCycle::TerminalWritebackRni => self.finish_terminal_writeback_rni(bus),
            StepFinishCycle::TerminalWritebackInlineCommit => {
                self.biu_commit_pending_write_inline(bus);
            }
        }
        self.step_finish_cycle = StepFinishCycle::WithFetchCycle;
    }

    fn reset_instruction_timing(&mut self) {
        self.modrm_displacement = 0;
        self.modrm_has_displacement = false;
        self.step_finish_cycle = StepFinishCycle::WithFetchCycle;
    }

    #[inline(always)]
    fn finish_on_terminal_writeback(&mut self, modrm: u8) {
        if modrm < 0xC0 {
            self.step_finish_cycle = StepFinishCycle::TerminalWritebackRni;
        }
    }

    #[inline(always)]
    fn finish_on_terminal_writeback_inline_commit(&mut self, modrm: u8) {
        if modrm < 0xC0 {
            self.step_finish_cycle = StepFinishCycle::TerminalWritebackInlineCommit;
        }
    }

    fn finish_on_terminal_writeback_with_fetch(&mut self, modrm: u8) {
        if modrm < 0xC0 {
            self.step_finish_cycle = StepFinishCycle::TerminalWritebackFetchCycle;
        }
    }

    fn finish_terminal_writeback_with_fetch(&mut self, bus: &mut impl common::Bus) {
        self.biu_bus_wait_finish(bus);
        self.biu_fetch_next(bus);
    }

    fn finish_terminal_writeback_rni(&mut self, bus: &mut impl common::Bus) {
        self.biu_bus_wait_finish(bus);
        self.biu_preload_next(bus);
    }

    #[inline(always)]
    fn instruction_started_at_odd_address(&self) -> bool {
        self.prev_ip & 1 == 1
    }

    #[inline(always)]
    fn opcode_started_at_odd_address(&self) -> bool {
        self.opcode_start_ip & 1 == 1
    }

    #[inline(always)]
    fn odd_queue_start_penalty(&self) -> i32 {
        i32::from(self.instruction_started_at_odd_address())
    }

    #[inline(always)]
    fn queue_resident_eu_cycles(&self, total_cycles: i32, trailing_bytes: i32) -> i32 {
        total_cycles - trailing_bytes - 2
    }

    #[inline(always)]
    fn instruction_entry_queue_full(&self) -> bool {
        debug_assert!(usize::from(self.instruction_entry_queue_bytes) <= QUEUE_SIZE);
        usize::from(self.instruction_entry_queue_bytes) == QUEUE_SIZE
    }

    /// If IP is odd after the last queue read, the next opcode byte is already
    /// present as the high byte of the prefetched 16-bit word.
    #[inline(always)]
    fn next_instruction_uses_prefetched_high_byte(&self) -> bool {
        self.ip & 1 == 1 && self.queue_len() > 0
    }

    #[inline(always)]
    fn terminal_writeback_sees_full_queue(&self) -> bool {
        self.queue_len() == QUEUE_SIZE
            && self.instruction_preload.is_none()
            && self.fetch_state == FetchState::PausedFull
            && self.pl_status == BusStatus::Passive
    }

    #[inline(always)]
    fn mov_rm_reg_store_uses_terminal_writeback_rni(&self, modrm: u8) -> bool {
        if !self.terminal_writeback_sees_full_queue() {
            return false;
        }

        if !self.seg_prefix {
            return self.has_disp16_double_register_base(modrm);
        }

        self.has_disp16_single_register_base(modrm)
            || self.has_mod0_single_register_base(modrm)
            || (self.has_disp16_double_register_base(modrm)
                && !self.has_disp16_cycle4_double_register_base(modrm))
    }

    #[inline(always)]
    fn mov_rm_sreg_uses_inline_commit(&self, modrm: u8) -> bool {
        if !self.terminal_writeback_sees_full_queue() {
            return false;
        }

        match modrm & 0xC7 {
            0x06 => true,
            0x80..=0x87 => !self.seg_prefix || !matches!(modrm & 0xC7, 0x80 | 0x83),
            0x04..=0x07 => self.seg_prefix,
            _ => false,
        }
    }

    #[inline(always)]
    fn mov_rm_sreg_uses_terminal_writeback_with_fetch(&self, modrm: u8) -> bool {
        match modrm & 0xC7 {
            0x06 | 0x84..=0x87 => !self.terminal_writeback_sees_full_queue(),
            0x44..=0x47 => self.terminal_writeback_sees_full_queue(),
            0x80 | 0x83 => self.terminal_writeback_sees_full_queue() && self.seg_prefix,
            _ => false,
        }
    }

    #[inline(always)]
    fn far_indirect_transfer_uses_preloaded_handoff(&self, modrm: u8) -> bool {
        if self.queue_len() == 5 && self.ip & 1 == 1 {
            return self.has_simple_disp16_base(modrm);
        }

        if self.queue_len() != QUEUE_SIZE || self.ip & 1 != 0 {
            return false;
        }

        self.has_disp8_single_register_base(modrm)
            || (modrm & 0xC7) == 0x06
            || (!self.seg_prefix && self.has_disp16_single_register_base(modrm))
            || (self.seg_prefix && self.has_disp16_cycle4_double_register_base(modrm))
    }

    #[inline(always)]
    fn far_immediate_jump_uses_preloaded_finish(&self) -> bool {
        self.seg_prefix && self.next_instruction_uses_prefetched_high_byte()
    }

    #[inline(always)]
    fn far_immediate_call_uses_drained_queue_handoff(&self) -> bool {
        self.queue_len() == 0
            && self.instruction_preload.is_none()
            && self.fetch_state == FetchState::Normal
            && self.pl_status == BusStatus::CodeFetch
            && self.bus_status_latch == BusStatus::CodeFetch
            && self.t_cycle == TCycle::T3
            && self.ta_cycle == TaCycle::Ts
    }

    #[inline(always)]
    fn charge_seg_prefix_cycle(&mut self, _bus: &mut impl common::Bus) {}

    #[inline(always)]
    fn corr(&mut self, bus: &mut impl common::Bus) {
        self.clk(bus, 1);
    }

    #[inline(always)]
    fn nearcall_routine(&mut self, bus: &mut impl common::Bus, new_ip: u16, early_fetch: bool) {
        let return_ip = self.ip;
        self.clk(bus, 1);
        self.ip = new_ip;
        if early_fetch {
            self.flush_and_fetch_early(bus);
        } else {
            self.flush_and_fetch(bus);
        }
        self.clk(bus, 3);
        self.push(bus, return_ip);
    }

    #[inline(always)]
    fn farcall_routine(
        &mut self,
        bus: &mut impl common::Bus,
        new_cs: u16,
        new_ip: u16,
        jump: bool,
        early_fetch: bool,
    ) {
        if jump {
            self.clk(bus, 1);
        }
        self.biu_fetch_suspend(bus);
        self.clk(bus, 2);
        self.corr(bus);
        self.clk(bus, 1);
        let return_cs = self.sregs[SegReg16::CS as usize];
        self.push(bus, return_cs);
        self.sregs[SegReg16::CS as usize] = new_cs;
        self.clk(bus, 2);
        self.nearcall_routine(bus, new_ip, early_fetch);
    }

    fn farret_routine(&mut self, bus: &mut impl common::Bus, far: bool) {
        self.clk(bus, 1);
        let ip = self.pop(bus);
        self.ip = ip;
        self.biu_fetch_suspend(bus);
        self.clk(bus, 3);

        if far {
            self.clk(bus, 1);
            let cs = self.pop(bus);
            self.sregs[SegReg16::CS as usize] = cs;
        }

        self.flush_and_fetch(bus);
        self.clk(bus, 2);
    }

    #[inline(always)]
    fn odd_opcode_start_penalty(&self) -> i32 {
        i32::from(self.opcode_started_at_odd_address())
    }

    #[inline(always)]
    fn opcode_start_penalty_2(&self) -> i32 {
        2 * self.odd_opcode_start_penalty()
    }

    #[inline(always)]
    fn opcode_even_penalty_2(&self) -> i32 {
        if self.opcode_started_at_odd_address() {
            0
        } else {
            2
        }
    }

    /// Installs prefetched instruction bytes and advances the BIU fetch pointer.
    pub fn install_prefetch_queue(&mut self, bytes: &[u8]) {
        assert!(
            bytes.len() <= self.instruction_queue.len(),
            "8086 prefetch queue overflow"
        );
        self.instruction_queue = [0; 6];
        self.instruction_queue[..bytes.len()].copy_from_slice(bytes);
        self.instruction_queue_len = bytes.len();
        self.instruction_preload = None;
        self.prefetch_ip = self.ip.wrapping_add(bytes.len() as u16);
        self.step_finish_cycle = StepFinishCycle::WithFetchCycle;
        self.nx = false;
        self.rni = false;
        self.queue_op = QueueOp::Idle;
        self.last_queue_op = QueueOp::Idle;
        self.fetch_state = FetchState::Normal;
        self.t_cycle = TCycle::Ti;
        self.ta_cycle = TaCycle::Td;
        self.bus_status = BusStatus::Passive;
        self.bus_status_latch = BusStatus::Passive;
        self.pl_status = BusStatus::Passive;
        self.bus_pending = BusPendingType::None;
    }

    fn push(&mut self, bus: &mut impl common::Bus, value: u16) {
        let sp = self.regs.word(WordReg::SP).wrapping_sub(2);
        self.regs.set_word(WordReg::SP, sp);
        self.write_word_seg(bus, SegReg16::SS, sp, value);
    }

    fn pop(&mut self, bus: &mut impl common::Bus) -> u16 {
        let sp = self.regs.word(WordReg::SP);
        let value = self.read_word_seg(bus, SegReg16::SS, sp);
        self.regs.set_word(WordReg::SP, sp.wrapping_add(2));
        value
    }

    fn execute_one(&mut self, bus: &mut impl common::Bus) {
        self.prev_ip = self.ip;
        self.reset_instruction_timing();
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

        if self.rep_active {
            self.continue_rep(bus);
        } else {
            let mut pending_rep: Option<RepType> = None;
            let mut opcode = self.fetch_first(bus);
            loop {
                match opcode {
                    0x26 => {
                        self.seg_prefix = true;
                        self.prefix_seg = SegReg16::ES;
                        self.clk(bus, 1);
                        opcode = self.fetch(bus);
                    }
                    0x2E => {
                        self.seg_prefix = true;
                        self.prefix_seg = SegReg16::CS;
                        self.clk(bus, 1);
                        opcode = self.fetch(bus);
                    }
                    0x36 => {
                        self.seg_prefix = true;
                        self.prefix_seg = SegReg16::SS;
                        self.clk(bus, 1);
                        opcode = self.fetch(bus);
                    }
                    0x3E => {
                        self.seg_prefix = true;
                        self.prefix_seg = SegReg16::DS;
                        self.clk(bus, 1);
                        opcode = self.fetch(bus);
                    }
                    0xF0 | 0xF1 => {
                        self.inhibit_all = 1;
                        self.clk(bus, 1);
                        opcode = self.fetch(bus);
                    }
                    0xF2 => {
                        pending_rep = Some(RepType::RepNe);
                        self.clk(bus, 1);
                        opcode = self.fetch(bus);
                    }
                    0xF3 => {
                        pending_rep = Some(RepType::RepE);
                        self.clk(bus, 1);
                        opcode = self.fetch(bus);
                    }
                    _ => {
                        self.opcode_start_ip = self.ip.wrapping_sub(1);
                        if let Some(rep_type) = pending_rep {
                            self.start_rep_with_opcode(rep_type, opcode, bus);
                        } else {
                            self.dispatch(opcode, bus);
                        }
                        break;
                    }
                }
            }
        }
    }

    /// Executes exactly one logical instruction (should only be used in tests).
    ///
    /// Sets `cycles_remaining` to `i64::MAX` so that REP-prefixed string
    /// instructions run to completion in a single call instead of pausing
    /// mid-loop when the cycle budget runs out. This is necessary because
    /// `execute_one` resumes a paused REP via `continue_rep`, which would
    /// otherwise split a single logical instruction across multiple calls.
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

impl common::Cpu for I8086 {
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
            if !self.rep_active {
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
        self.state = I8086State::default();
        self.sregs[SegReg16::CS as usize] = 0xFFFF;
        self.prev_ip = 0;
        self.opcode_start_ip = 0;
        self.halted = false;
        self.pending_irq = 0;
        self.no_interrupt = 0;
        self.inhibit_all = 0;
        self.rep_active = false;
        self.rep_restart_ip = 0;
        self.rep_prefix = false;
        self.seg_prefix = false;
        self.ea = 0;
        self.eo = 0;
        self.effective_address_segment = SegReg16::DS;
        self.modrm_displacement = 0;
        self.modrm_has_displacement = false;
        self.instruction_queue_len = 0;
        self.instruction_preload = None;
        self.instruction_entry_queue_bytes = 0;
        self.prefetch_ip = self.ip;
        self.step_finish_cycle = StepFinishCycle::WithFetchCycle;
        self.nx = false;
        self.rni = false;
        self.queue_op = QueueOp::Idle;
        self.last_queue_op = QueueOp::Idle;
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
        self.bhe = false;
        self.address_bus = 0;
        self.address_latch = 0;
        self.data_bus = 0;
        self.reset_instruction_timing();
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
        common::CpuType::I8086
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

impl save_state::AfterRestore for I8086 {
    fn after_restore(&mut self) {
        self.cycles_remaining = 0;
        self.run_start_cycle = 0;
        self.run_budget = 0;
    }
}

impl save_state::RestoreTarget for I8086 {
    type State = I8086State;
    type ValidationContext = ();

    fn replace_state(&mut self, state: Self::State) {
        self.state = state;
    }
}
