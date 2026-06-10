//! Implements the Intel 80286 emulation.
//!
//! The protected mode is not fully developed in this core.
//! Only OS/2 1.x should need it, which is currently lost.
//!
//! Following references were used to write the emulator:
//!
//! - Intel Corporation, "80286 Programmer's Reference Manual".
//! - MAME Intel i286 emulator (`devices/cpu/i86/i286.cpp`).

mod alu;
mod execute;
mod execute_0f;
mod execute_group;
mod flags;
mod interrupt;
mod modrm;
mod rep;
mod state;
mod string_ops;
mod timing;

use core::ops::{Deref, DerefMut};

use common::Cpu as _;
pub use flags::I286Flags;
pub use modrm::EaClass;
pub use state::I286State;
pub use timing::{
    I286AuStage, I286BusPhase, I286CycleState, I286CycleTraceEntry, I286EuStage, I286FinishState,
    I286FlushState, I286PendingBusRequest, I286RepState, I286TimingMilestones, I286TraceBusStatus,
    I286WarmStartConfig,
};
use timing::{I286ColdStartPrefetchPolicy, I286Timing};

use crate::{SegReg16, WordReg};

/// 24-bit physical address mask.
pub(super) const ADDRESS_MASK: u32 = 0x00FF_FFFF;

/// Returns true when the low bit of the supplied bus address is set.
/// The 286 has a 16-bit data bus, so an odd address forces an extra
/// bus access for word transfers; many timing decisions branch on this.
#[inline(always)]
pub(super) const fn address_is_odd(address: u32) -> bool {
    address & 1 == 1
}

#[derive(Clone, Copy)]
struct SegmentDescriptor {
    base: u32,
    limit: u16,
    rights: u8,
}

#[derive(Clone, Copy, PartialEq)]
enum TaskType {
    Iret,
    Jmp,
    Call,
}

#[derive(Clone, Copy)]
struct I286OpcodeLookahead {
    opcode: u8,
    operand_offset: u16,
}

impl I286OpcodeLookahead {
    #[inline(always)]
    const fn is_segment_override(self) -> bool {
        matches!(self.opcode, 0x26 | 0x2E | 0x36 | 0x3E)
    }

    #[inline(always)]
    const fn is_string(self) -> bool {
        matches!(self.opcode, 0x6C..=0x6F | 0xA4..=0xA7 | 0xAA..=0xAF)
    }

    #[inline(always)]
    const fn is_xlat(self) -> bool {
        self.opcode == 0xD7
    }

    #[inline(always)]
    const fn is_leave(self) -> bool {
        self.opcode == 0xC9
    }

    #[inline(always)]
    const fn is_short_jump(self) -> bool {
        self.opcode == 0xEB
    }

    #[inline(always)]
    const fn is_fpu_escape(self) -> bool {
        matches!(self.opcode, 0xD8..=0xDF)
    }

    #[inline(always)]
    const fn is_group_ff(self) -> bool {
        self.opcode == 0xFF
    }

    #[inline(always)]
    const fn is_les_or_lds(self) -> bool {
        matches!(self.opcode, 0xC4 | 0xC5)
    }

    #[inline(always)]
    const fn is_lock_prefetch_candidate(self, after_segment_prefix: bool) -> bool {
        if Self::binary_alu_opcode(self.opcode)
            || matches!(
                self.opcode,
                0x62 | 0x68 | 0x69 | 0x6B | 0x80..=0x8D | 0x9A | 0xA0..=0xA3 | 0xC0..=0xC2 | 0xC4
                    | 0xC5 | 0xD0..=0xD3 | 0xD8..=0xDF | 0xE4..=0xE7 | 0xEA | 0xF6 | 0xF7
                    | 0xFE
                    | 0xCA
            )
        {
            return true;
        }

        !after_segment_prefix && matches!(self.opcode, 0xD4 | 0xD5 | 0xE8 | 0xE9)
    }

    #[inline(always)]
    const fn binary_alu_opcode(opcode: u8) -> bool {
        matches!(
            opcode,
            0x00..=0x05
                | 0x08..=0x0D
                | 0x10..=0x15
                | 0x18..=0x1D
                | 0x20..=0x25
                | 0x28..=0x2D
                | 0x30..=0x35
                | 0x38..=0x3D
        )
    }
}

#[derive(Clone, Copy)]
struct I286GroupFfLookahead {
    modrm: u8,
}

impl I286GroupFfLookahead {
    #[inline(always)]
    const fn register_field(self) -> u8 {
        (self.modrm >> 3) & 7
    }

    #[inline(always)]
    const fn is_register_form(self) -> bool {
        self.modrm >= 0xC0
    }

    #[inline(always)]
    const fn is_memory_form(self) -> bool {
        self.modrm < 0xC0
    }

    #[inline(always)]
    const fn is_mode0_non_direct_memory_form(self) -> bool {
        self.modrm < 0x40 && (self.modrm & 7) != 6
    }

    #[inline(always)]
    const fn lock_prefetches_indirect_control_transfer(self) -> bool {
        if !matches!(self.register_field(), 2..=5) || self.is_register_form() {
            return false;
        }

        let mode = self.modrm >> 6;
        mode != 0 || (self.modrm & 7) == 6
    }

    #[inline(always)]
    const fn lock_prefetches_push(self, lock_prefix_after_prefix: bool) -> bool {
        matches!(self.register_field(), 6 | 7)
            && (!lock_prefix_after_prefix || self.is_memory_form())
    }

    #[inline(always)]
    const fn segment_prefix_passivizes_indirect_control_transfer(self) -> bool {
        matches!(self.register_field(), 2 | 4) && self.is_register_form()
            || matches!(self.register_field(), 2..=5) && self.is_mode0_non_direct_memory_form()
    }

    #[inline(always)]
    const fn segment_prefix_single_passivizes_indirect_control_transfer(self) -> bool {
        self.register_field() == 4 && self.is_register_form()
    }
}

/// Intel 80286 CPU emulator.
pub struct I286 {
    /// Embedded state for save/restore.
    pub state: I286State,

    prev_ip: u16,
    seg_prefix: bool,
    prefix_seg: SegReg16,

    halted: bool,
    pending_irq: u8,
    no_interrupt: u8,
    inhibit_all: u8,

    rep_ip: u16,
    rep_restart_ip: u16,
    rep_seg_prefix: bool,
    rep_prefix_seg: SegReg16,
    rep_opcode: u8,
    rep_type: u8,
    rep_active: bool,

    cycles_remaining: i64,
    run_start_cycle: u64,
    run_budget: u64,

    ea: u32,
    eo: u16,
    ea_seg: SegReg16,
    pub(crate) ea_class: EaClass,
    pub(crate) finish_state: I286FinishState,

    trap_level: u8,
    shutdown: bool,
    timing: I286Timing,
}

impl Deref for I286 {
    type Target = I286State;
    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl DerefMut for I286 {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

impl Default for I286 {
    fn default() -> Self {
        Self::new()
    }
}

impl I286 {
    /// Creates a new I286 CPU in its reset state.
    pub fn new() -> Self {
        let mut cpu = Self {
            state: I286State::default(),
            prev_ip: 0,
            seg_prefix: false,
            prefix_seg: SegReg16::DS,
            halted: false,
            pending_irq: 0,
            no_interrupt: 0,
            inhibit_all: 0,
            rep_ip: 0,
            rep_restart_ip: 0,
            rep_seg_prefix: false,
            rep_prefix_seg: SegReg16::DS,
            rep_opcode: 0,
            rep_type: 0,
            rep_active: false,
            cycles_remaining: 0,
            run_start_cycle: 0,
            run_budget: 0,
            ea: 0,
            eo: 0,
            ea_seg: SegReg16::DS,
            ea_class: EaClass::Register,
            finish_state: I286FinishState::Linear,
            trap_level: 0,
            shutdown: false,
            timing: I286Timing::new(),
        };
        cpu.reset();
        cpu
    }

    #[inline(always)]
    fn sync_timing_cycles(&mut self) {
        self.cycles_remaining -= i64::from(self.timing.take_cycle_debt());
    }

    #[inline(always)]
    fn clk(&mut self, cycles: i32) {
        self.timing.advance_internal_cycles(cycles);
        self.sync_timing_cycles();
    }

    #[inline(always)]
    fn clk_visible(&mut self, cycles: u8) {
        self.timing.advance_visible_internal_cycles(cycles);
        self.sync_timing_cycles();
    }

    #[inline(always)]
    fn clk_prefix(&mut self, bus: &mut impl common::Bus) {
        let code_segment_base = self.seg_bases[SegReg16::CS as usize];
        self.timing
            .advance_prefix_overlap_prefetch(bus, code_segment_base);
        self.sync_timing_cycles();
    }

    #[inline(always)]
    fn clk_prefix_passive(&mut self) {
        self.timing.advance_prefix_overlap_passive();
        self.sync_timing_cycles();
    }

    #[inline(always)]
    fn clk_prefix_single_passive(&mut self) {
        self.timing.advance_prefix_overlap_single_passive();
        self.sync_timing_cycles();
    }

    #[inline(always)]
    fn clk_lock_prefix(
        &mut self,
        bus: &mut impl common::Bus,
        next_opcode: u8,
        prefix_count_before_lock: u8,
        prefetches_during_lock_prefix: bool,
    ) {
        let code_segment_base = self.seg_bases[SegReg16::CS as usize];
        let suppress_lock_prefix_cycle = (next_opcode == 0xC9 && prefix_count_before_lock == 1)
            || (prefix_count_before_lock & 1 == 1 && Self::string_opcode(next_opcode))
            || (prefix_count_before_lock != 0
                && (Self::segment_override_prefix(next_opcode)
                    || prefix_count_before_lock & 1 == 1)
                && self.lock_prefix_followed_by_xlat(bus, next_opcode));

        if prefetches_during_lock_prefix {
            self.timing
                .advance_lock_prefix_prefetch(bus, code_segment_base)
        } else if suppress_lock_prefix_cycle {
            self.timing.clear_lock_prefix_pending_cycle();
        } else {
            self.timing.advance_lock_prefix_passive_cycle()
        };

        if self.consumed_opcode_lookahead(next_opcode).is_les_or_lds()
            && prefix_count_before_lock & 1 == 1
        {
            self.timing.suppress_next_demand_prefetch();
        }
        self.sync_timing_cycles();
    }

    #[inline(always)]
    fn segment_override_prefix(opcode: u8) -> bool {
        matches!(opcode, 0x26 | 0x2E | 0x36 | 0x3E)
    }

    #[inline(always)]
    const fn string_opcode(opcode: u8) -> bool {
        I286OpcodeLookahead {
            opcode,
            operand_offset: 0,
        }
        .is_string()
    }

    fn code_byte_at(&self, bus: &mut impl common::Bus, offset: u16) -> u8 {
        let code_segment_base = self.seg_bases[SegReg16::CS as usize];
        bus.read_byte(code_segment_base.wrapping_add(u32::from(offset)) & ADDRESS_MASK)
    }

    fn consumed_opcode_lookahead(&self, opcode: u8) -> I286OpcodeLookahead {
        I286OpcodeLookahead {
            opcode,
            operand_offset: self.ip,
        }
    }

    fn next_opcode_lookahead(&self, bus: &mut impl common::Bus) -> I286OpcodeLookahead {
        I286OpcodeLookahead {
            opcode: self.code_byte_at(bus, self.ip),
            operand_offset: self.ip.wrapping_add(1),
        }
    }

    fn next_non_segment_opcode_lookahead(&self, bus: &mut impl common::Bus) -> I286OpcodeLookahead {
        let mut lookahead = self.next_opcode_lookahead(bus);
        while lookahead.is_segment_override() {
            lookahead = I286OpcodeLookahead {
                opcode: self.code_byte_at(bus, lookahead.operand_offset),
                operand_offset: lookahead.operand_offset.wrapping_add(1),
            };
        }
        lookahead
    }

    fn group_ff_lookahead(
        &self,
        bus: &mut impl common::Bus,
        operand_offset: u16,
    ) -> I286GroupFfLookahead {
        I286GroupFfLookahead {
            modrm: self.code_byte_at(bus, operand_offset),
        }
    }

    fn lock_prefix_prefetches_for_next_opcode(
        &self,
        bus: &mut impl common::Bus,
        opcode: u8,
    ) -> bool {
        let lookahead = self.consumed_opcode_lookahead(opcode);
        if !lookahead.is_segment_override() {
            if lookahead.is_fpu_escape() && self.timing.lock_prefix_after_prefix() {
                let modrm = self.code_byte_at(bus, lookahead.operand_offset);
                if modrm >= 0xC0 {
                    return false;
                }
            }
            if lookahead.is_lock_prefetch_candidate(false) {
                return true;
            }

            if !lookahead.is_group_ff() {
                return false;
            }

            let group_ff = self.group_ff_lookahead(bus, lookahead.operand_offset);
            return matches!(group_ff.register_field(), 0 | 1)
                || group_ff.lock_prefetches_indirect_control_transfer()
                || (!self.timing.lock_prefix_after_prefix()
                    && group_ff.lock_prefetches_push(self.timing.lock_prefix_after_prefix()));
        }

        let final_lookahead = self.next_non_segment_opcode_lookahead(bus);

        if final_lookahead.is_xlat() {
            return !self.timing.lock_prefix_after_prefix();
        }

        if final_lookahead.is_string() {
            return !self.timing.lock_prefix_after_prefix();
        }

        if final_lookahead.is_short_jump() {
            return true;
        }

        if final_lookahead.is_lock_prefetch_candidate(true) {
            return true;
        }

        if final_lookahead.is_leave() {
            return self.timing.prefix_count_at_most(3);
        }

        if !final_lookahead.is_group_ff() {
            return false;
        }

        let group_ff = self.group_ff_lookahead(bus, final_lookahead.operand_offset);
        let register_field = group_ff.register_field();
        if matches!(register_field, 0 | 1) {
            return true;
        }

        if register_field == 4 && group_ff.is_register_form() {
            return !self.timing.lock_prefix_after_prefix();
        }

        if matches!(register_field, 6 | 7) {
            return true;
        }

        matches!(register_field, 3..=5) && group_ff.is_memory_form()
    }

    fn lock_prefix_followed_by_xlat(&self, bus: &mut impl common::Bus, opcode: u8) -> bool {
        let lookahead = self.consumed_opcode_lookahead(opcode);
        if !lookahead.is_segment_override() {
            return lookahead.is_xlat();
        }

        self.next_non_segment_opcode_lookahead(bus).is_xlat()
    }

    fn segment_prefix_passivizes_ff_indirect_control_transfer_prefetch(
        &self,
        lookahead: I286OpcodeLookahead,
        bus: &mut impl common::Bus,
    ) -> bool {
        if !lookahead.is_group_ff() {
            return false;
        }

        self.group_ff_lookahead(bus, lookahead.operand_offset)
            .segment_prefix_passivizes_indirect_control_transfer()
    }

    fn segment_prefix_single_passivizes_ff_indirect_control_transfer_prefetch(
        &self,
        lookahead: I286OpcodeLookahead,
        bus: &mut impl common::Bus,
    ) -> bool {
        if !self.timing.lock_active() || !self.timing.lock_prefix_after_prefix() {
            return false;
        }

        if !lookahead.is_group_ff() {
            return false;
        }

        self.group_ff_lookahead(bus, lookahead.operand_offset)
            .segment_prefix_single_passivizes_indirect_control_transfer()
    }

    fn segment_prefix_skips_prefetch_after_lock(&self, lookahead: I286OpcodeLookahead) -> bool {
        if !self.timing.lock_active()
            || !self.timing.lock_prefix_after_prefix()
            || !self.timing.lock_prefix_followed_by_prefix()
        {
            return false;
        }

        lookahead.is_leave() || lookahead.is_string()
    }

    fn clk_segment_override_prefix(&mut self, bus: &mut impl common::Bus) {
        let lookahead = self.next_opcode_lookahead(bus);
        if self.segment_prefix_skips_prefetch_after_lock(lookahead) {
            return;
        }
        if lookahead.is_string() {
            self.clk_prefix_single_passive();
            return;
        }
        if lookahead.is_leave()
            || lookahead.is_xlat()
            || (lookahead.is_short_jump()
                && (!self.timing.lock_active() || self.timing.lock_prefix_after_prefix()))
            || self.segment_prefix_single_passivizes_ff_indirect_control_transfer_prefetch(
                lookahead, bus,
            )
        {
            self.clk_prefix_single_passive();
        } else if self
            .segment_prefix_passivizes_ff_indirect_control_transfer_prefetch(lookahead, bus)
        {
            self.clk_prefix_passive();
        } else {
            self.clk_prefix(bus);
        }
    }

    #[inline(always)]
    fn clk_prefetch(&mut self, bus: &mut impl common::Bus, cycles: i32) {
        let code_segment_base = self.seg_bases[SegReg16::CS as usize];
        self.timing.note_execution_cycles();
        self.timing
            .advance_internal_cycles_with_prefetch(bus, code_segment_base, cycles);
        self.sync_timing_cycles();
    }

    #[inline(always)]
    fn clk_forced_prefetch(&mut self, bus: &mut impl common::Bus) {
        let code_segment_base = self.seg_bases[SegReg16::CS as usize];
        self.timing.note_execution_cycles();
        self.timing
            .advance_forced_prefetch_fetch(bus, code_segment_base);
        self.sync_timing_cycles();
    }

    #[inline(always)]
    fn clk_control_transfer_restart(
        &mut self,
        bus: &mut impl common::Bus,
        instruction_pointer: u16,
        timing: timing::I286ControlTransferTimingTemplate,
    ) {
        self.finish_state = I286FinishState::ControlTransferRestart;
        let code_segment_base = self.seg_bases[SegReg16::CS as usize];
        self.timing
            .arm_control_transfer_restart(instruction_pointer);
        self.timing
            .advance_control_transfer_restart(bus, code_segment_base, timing);
        self.sync_timing_cycles();
    }

    #[inline(always)]
    fn clk_control_transfer_restart_without_gap_credit(
        &mut self,
        bus: &mut impl common::Bus,
        instruction_pointer: u16,
        timing: timing::I286ControlTransferTimingTemplate,
    ) {
        self.finish_state = I286FinishState::ControlTransferRestart;
        let code_segment_base = self.seg_bases[SegReg16::CS as usize];
        self.timing
            .arm_control_transfer_restart_without_gap_credit(instruction_pointer);
        self.timing
            .advance_control_transfer_restart(bus, code_segment_base, timing);
        self.sync_timing_cycles();
    }

    #[inline(always)]
    fn clk_modrm_prefetch(
        &mut self,
        bus: &mut impl common::Bus,
        modrm: u8,
        reg_cycles: i32,
        mem_eu_base_cycles: i32,
    ) {
        if modrm::modrm_is_register(modrm) {
            self.clk_prefetch(bus, reg_cycles);
        } else {
            let au_cycles = modrm::ea_class_au_cycles(self.ea_class);
            self.clk_prefetch(bus, mem_eu_base_cycles + au_cycles);
        }
    }

    #[inline(always)]
    fn clk_modrm_word_prefetch(
        &mut self,
        bus: &mut impl common::Bus,
        modrm: u8,
        reg_cycles: i32,
        mem_eu_base_cycles: i32,
        word_accesses: i32,
    ) {
        if modrm::modrm_is_register(modrm) {
            self.clk_prefetch(bus, reg_cycles);
        } else {
            let au_cycles = modrm::ea_class_au_cycles(self.ea_class);
            let odd_penalty = if address_is_odd(self.ea) {
                4 * word_accesses
            } else {
                0
            };
            self.clk_prefetch(bus, mem_eu_base_cycles + au_cycles + odd_penalty);
        }
    }

    #[inline(always)]
    fn sp_penalty(&self, word_accesses: i32) -> i32 {
        if self.regs.word(WordReg::SP) & 1 == 1 {
            4 * word_accesses
        } else {
            0
        }
    }

    #[inline(always)]
    fn stack_push_tail_cycles(&self, even_tail_cycles: i32) -> i32 {
        even_tail_cycles - i32::from(self.regs.word(WordReg::SP) & 1)
    }

    #[inline(always)]
    fn fetch(&mut self, bus: &mut impl common::Bus) -> u8 {
        let addr =
            self.seg_bases[SegReg16::CS as usize].wrapping_add(self.ip as u32) & ADDRESS_MASK;
        let value = bus.read_byte(addr);
        let code_segment_base = self.seg_bases[SegReg16::CS as usize];
        self.timing.note_code_byte_consumed(
            bus,
            code_segment_base,
            self.ip,
            Self::opcode_cold_start_prefetch_policy(value),
        );
        self.sync_timing_cycles();
        self.ip = self.ip.wrapping_add(1);
        value
    }

    #[inline(always)]
    fn opcode_cold_start_prefetch_policy(opcode: u8) -> I286ColdStartPrefetchPolicy {
        match opcode {
            0xC3 => I286ColdStartPrefetchPolicy::PassiveLastFetchWindow,
            0xCB | 0xCF => I286ColdStartPrefetchPolicy::StopBeforeLastFetch,
            _ => I286ColdStartPrefetchPolicy::Complete,
        }
    }

    #[inline(always)]
    fn fetchword(&mut self, bus: &mut impl common::Bus) -> u16 {
        let low = self.fetch(bus) as u16;
        let high = self.fetch(bus) as u16;
        low | (high << 8)
    }

    #[inline(always)]
    fn default_seg(&self, seg: SegReg16) -> SegReg16 {
        if self.seg_prefix && matches!(seg, SegReg16::DS | SegReg16::SS) {
            self.prefix_seg
        } else {
            seg
        }
    }

    #[inline(always)]
    fn default_base(&self, seg: SegReg16) -> u32 {
        self.seg_bases[self.default_seg(seg) as usize]
    }

    #[inline(always)]
    fn seg_base(&self, seg: SegReg16) -> u32 {
        self.seg_bases[seg as usize]
    }

    #[inline(always)]
    fn read_io_byte(&mut self, bus: &mut impl common::Bus, port: u16) -> u8 {
        let value = bus.io_read_byte(port);
        self.timing
            .note_io_read_byte(bus, self.seg_bases[SegReg16::CS as usize], port, value);
        self.sync_timing_cycles();
        value
    }

    #[inline(always)]
    fn read_io_word(&mut self, bus: &mut impl common::Bus, port: u16) -> u16 {
        let value = bus.io_read_word(port);
        self.timing
            .note_io_read_word(bus, self.seg_bases[SegReg16::CS as usize], port, value);
        self.sync_timing_cycles();
        value
    }

    #[inline(always)]
    fn write_io_byte(&mut self, bus: &mut impl common::Bus, port: u16, value: u8) {
        bus.io_write_byte(port, value);
        self.timing
            .note_io_write_byte(bus, self.seg_bases[SegReg16::CS as usize], port, value);
        self.sync_timing_cycles();
    }

    #[inline(always)]
    fn write_io_word(&mut self, bus: &mut impl common::Bus, port: u16, value: u16) {
        bus.io_write_word(port, value);
        self.timing
            .note_io_write_word(bus, self.seg_bases[SegReg16::CS as usize], port, value);
        self.sync_timing_cycles();
    }

    /// Computes the physical address for a byte at `eo + delta`, wrapping
    /// the offset within the 16-bit segment boundary.
    #[inline(always)]
    fn seg_addr(&self, delta: u16) -> u32 {
        self.seg_base(self.ea_seg)
            .wrapping_add(self.eo.wrapping_add(delta) as u32)
            & ADDRESS_MASK
    }

    #[inline(always)]
    fn cpl(&self) -> u16 {
        self.sregs[SegReg16::CS as usize] & 3
    }

    #[inline(always)]
    fn is_protected_mode(&self) -> bool {
        self.msw & 1 != 0
    }

    #[inline(always)]
    fn segment_error_code(selector: u16) -> u16 {
        selector & 0xFFFC
    }

    fn set_real_segment_cache(&mut self, seg: SegReg16, selector: u16) {
        self.seg_bases[seg as usize] = (selector as u32) << 4;
        self.seg_limits[seg as usize] = 0xFFFF;
        self.seg_rights[seg as usize] = if seg == SegReg16::CS { 0x9B } else { 0x93 };
        self.seg_valid[seg as usize] = true;
    }

    fn set_loaded_segment_cache(
        &mut self,
        seg: SegReg16,
        selector: u16,
        descriptor: SegmentDescriptor,
    ) {
        self.sregs[seg as usize] = selector;
        self.seg_bases[seg as usize] = descriptor.base;
        self.seg_limits[seg as usize] = descriptor.limit;
        self.seg_rights[seg as usize] = descriptor.rights;
        self.seg_valid[seg as usize] = true;
    }

    fn set_null_segment(&mut self, seg: SegReg16, selector: u16) {
        self.sregs[seg as usize] = selector;
        self.seg_bases[seg as usize] = 0;
        self.seg_limits[seg as usize] = 0;
        self.seg_rights[seg as usize] = 0;
        self.seg_valid[seg as usize] = false;
    }

    fn decode_descriptor(
        &self,
        selector: u16,
        bus: &mut impl common::Bus,
    ) -> Option<SegmentDescriptor> {
        let addr = self.descriptor_addr_checked(selector)?;
        let limit = bus.read_byte(addr & ADDRESS_MASK) as u16
            | ((bus.read_byte(addr.wrapping_add(1) & ADDRESS_MASK) as u16) << 8);
        let base = bus.read_byte(addr.wrapping_add(2) & ADDRESS_MASK) as u32
            | ((bus.read_byte(addr.wrapping_add(3) & ADDRESS_MASK) as u32) << 8)
            | ((bus.read_byte(addr.wrapping_add(4) & ADDRESS_MASK) as u32) << 16);
        let rights = bus.read_byte(addr.wrapping_add(5) & ADDRESS_MASK);
        Some(SegmentDescriptor {
            base,
            limit,
            rights,
        })
    }

    fn descriptor_dpl(rights: u8) -> u16 {
        ((rights >> 5) & 0x03) as u16
    }

    fn descriptor_is_segment(rights: u8) -> bool {
        rights & 0x10 != 0
    }

    fn descriptor_is_code(rights: u8) -> bool {
        rights & 0x08 != 0
    }

    fn descriptor_is_conforming_code(rights: u8) -> bool {
        Self::descriptor_is_code(rights) && rights & 0x04 != 0
    }

    fn descriptor_is_readable(rights: u8) -> bool {
        !Self::descriptor_is_code(rights) || rights & 0x02 != 0
    }

    fn descriptor_is_writable(rights: u8) -> bool {
        !Self::descriptor_is_code(rights) && rights & 0x02 != 0
    }

    fn descriptor_is_expand_down(rights: u8) -> bool {
        !Self::descriptor_is_code(rights) && rights & 0x04 != 0
    }

    fn descriptor_present(rights: u8) -> bool {
        rights & 0x80 != 0
    }

    fn raise_segment_not_present(
        &mut self,
        seg: SegReg16,
        selector: u16,
        bus: &mut impl common::Bus,
    ) {
        let vector = if seg == SegReg16::SS { 12 } else { 11 };
        self.raise_fault_with_code(vector, Self::segment_error_code(selector), bus);
    }

    fn raise_segment_protection(
        &mut self,
        seg: SegReg16,
        selector: u16,
        bus: &mut impl common::Bus,
    ) {
        let vector = if seg == SegReg16::SS { 12 } else { 13 };
        self.raise_fault_with_code(vector, Self::segment_error_code(selector), bus);
    }

    fn load_protected_segment(
        &mut self,
        seg: SegReg16,
        selector: u16,
        bus: &mut impl common::Bus,
    ) -> bool {
        if matches!(seg, SegReg16::DS | SegReg16::ES) && selector & 0xFFFC == 0 {
            self.set_null_segment(seg, selector);
            return true;
        }
        if selector & 0xFFFC == 0 {
            self.raise_segment_protection(seg, selector, bus);
            return false;
        }

        let Some(descriptor) = self.decode_descriptor(selector, bus) else {
            self.raise_segment_protection(seg, selector, bus);
            return false;
        };
        let rights = descriptor.rights;
        let cpl = self.cpl();
        let rpl = selector & 0x0003;
        let dpl = Self::descriptor_dpl(rights);

        // Bug #14: Check type and privilege BEFORE present.
        match seg {
            SegReg16::CS => {
                if !Self::descriptor_is_segment(rights) || !Self::descriptor_is_code(rights) {
                    self.raise_segment_protection(seg, selector, bus);
                    return false;
                }
                if Self::descriptor_is_conforming_code(rights) {
                    if dpl > cpl {
                        self.raise_segment_protection(seg, selector, bus);
                        return false;
                    }
                } else if dpl != cpl || rpl > cpl {
                    self.raise_segment_protection(seg, selector, bus);
                    return false;
                }
                // Present check for CS -> #NP.
                if !Self::descriptor_present(rights) {
                    self.raise_segment_not_present(seg, selector, bus);
                    return false;
                }
                // Bug #10: Set accessed bit.
                self.set_accessed_bit(selector, bus);
                if Self::descriptor_is_conforming_code(rights) {
                    let adjusted = (selector & !3) | cpl;
                    self.set_loaded_segment_cache(seg, adjusted, descriptor);
                } else {
                    let adjusted = (selector & !3) | dpl;
                    self.set_loaded_segment_cache(seg, adjusted, descriptor);
                }
                return true;
            }
            SegReg16::SS => {
                if !Self::descriptor_is_segment(rights) || !Self::descriptor_is_writable(rights) {
                    self.raise_fault_with_code(13, Self::segment_error_code(selector), bus);
                    return false;
                }
                if dpl != cpl || rpl != cpl {
                    self.raise_fault_with_code(13, Self::segment_error_code(selector), bus);
                    return false;
                }
                // Present check for SS -> #SS (vector 12).
                if !Self::descriptor_present(rights) {
                    self.raise_fault_with_code(12, Self::segment_error_code(selector), bus);
                    return false;
                }
            }
            SegReg16::DS | SegReg16::ES => {
                if !Self::descriptor_is_segment(rights) {
                    self.raise_segment_protection(seg, selector, bus);
                    return false;
                }
                if !Self::descriptor_is_readable(rights) {
                    self.raise_segment_protection(seg, selector, bus);
                    return false;
                }
                if !Self::descriptor_is_conforming_code(rights) && dpl < cpl.max(rpl) {
                    self.raise_segment_protection(seg, selector, bus);
                    return false;
                }
                // Present check for DS/ES -> #NP.
                if !Self::descriptor_present(rights) {
                    self.raise_segment_not_present(seg, selector, bus);
                    return false;
                }
            }
        }

        // Bug #10: Set accessed bit.
        self.set_accessed_bit(selector, bus);
        self.set_loaded_segment_cache(seg, selector, descriptor);
        true
    }

    fn load_cs_for_return(
        &mut self,
        selector: u16,
        new_ip: u16,
        bus: &mut impl common::Bus,
    ) -> bool {
        if selector & 0xFFFC == 0 {
            self.raise_fault_with_code(13, 0, bus);
            return false;
        }
        let Some(descriptor) = self.decode_descriptor(selector, bus) else {
            self.raise_fault_with_code(13, Self::segment_error_code(selector), bus);
            return false;
        };
        let rights = descriptor.rights;
        let cpl = self.cpl();
        let rpl = selector & 0x0003;
        let dpl = Self::descriptor_dpl(rights);

        if rpl < cpl {
            self.raise_fault_with_code(13, Self::segment_error_code(selector), bus);
            return false;
        }
        if !Self::descriptor_is_segment(rights) || !Self::descriptor_is_code(rights) {
            self.raise_fault_with_code(13, Self::segment_error_code(selector), bus);
            return false;
        }
        if Self::descriptor_is_conforming_code(rights) {
            if dpl > rpl {
                self.raise_fault_with_code(13, Self::segment_error_code(selector), bus);
                return false;
            }
        } else if dpl != rpl {
            self.raise_fault_with_code(13, Self::segment_error_code(selector), bus);
            return false;
        }
        if !Self::descriptor_present(rights) {
            self.raise_fault_with_code(11, Self::segment_error_code(selector), bus);
            return false;
        }
        if new_ip > descriptor.limit {
            self.raise_fault_with_code(13, 0, bus);
            return false;
        }
        self.set_accessed_bit(selector, bus);
        let adjusted = (selector & !3) | rpl;
        self.set_loaded_segment_cache(SegReg16::CS, adjusted, descriptor);
        true
    }

    fn check_segment_access(
        &mut self,
        seg: SegReg16,
        offset: u16,
        size: u16,
        write: bool,
        bus: &mut impl common::Bus,
    ) -> bool {
        if !self.is_protected_mode() {
            return true;
        }

        if !self.seg_valid[seg as usize] {
            let vector = if seg == SegReg16::SS { 12 } else { 13 };
            self.raise_fault_with_code(vector, 0, bus);
            return false;
        }

        let rights = self.seg_rights[seg as usize];
        let end = offset as u32 + size.saturating_sub(1) as u32;
        let limit = self.seg_limits[seg as usize] as u32;
        if Self::descriptor_is_expand_down(rights) {
            if offset as u32 <= limit || end > 0xFFFF {
                self.raise_fault_with_code(if seg == SegReg16::SS { 12 } else { 13 }, 0, bus);
                return false;
            }
        } else if end > limit {
            self.raise_fault_with_code(if seg == SegReg16::SS { 12 } else { 13 }, 0, bus);
            return false;
        }

        if write {
            if !Self::descriptor_is_writable(rights) {
                let vector = if seg == SegReg16::SS { 12 } else { 13 };
                self.raise_fault_with_code(vector, 0, bus);
                return false;
            }
        } else if !Self::descriptor_is_readable(rights) {
            let vector = if seg == SegReg16::SS { 12 } else { 13 };
            self.raise_fault_with_code(vector, 0, bus);
            return false;
        }

        true
    }

    #[inline(always)]
    fn seg_read_byte_at(&mut self, bus: &mut impl common::Bus, delta: u16) -> u8 {
        let offset = self.eo.wrapping_add(delta);
        if !self.check_segment_access(self.ea_seg, offset, 1, false, bus) {
            return 0;
        }
        let address = self.seg_addr(delta);
        let value = bus.read_byte(address);
        self.timing.note_memory_read_byte(
            bus,
            self.seg_bases[SegReg16::CS as usize],
            address,
            value,
        );
        self.sync_timing_cycles();
        value
    }

    #[inline(always)]
    fn seg_write_byte_at(&mut self, bus: &mut impl common::Bus, delta: u16, value: u8) {
        let offset = self.eo.wrapping_add(delta);
        if !self.check_segment_access(self.ea_seg, offset, 1, true, bus) {
            return;
        }
        let address = self.seg_addr(delta);
        bus.write_byte(address, value);
        self.timing.note_memory_write_byte(
            bus,
            self.seg_bases[SegReg16::CS as usize],
            address,
            value,
        );
        self.sync_timing_cycles();
    }

    /// Reads a word from memory at `ea + delta`, wrapping the offset
    /// within the segment boundary (offset 0xFFFF wraps to 0x0000).
    #[inline(always)]
    fn seg_read_word_at(&mut self, bus: &mut impl common::Bus, delta: u16) -> u16 {
        let offset = self.eo.wrapping_add(delta);
        if !self.check_segment_access(self.ea_seg, offset, 2, false, bus) {
            return 0;
        }
        let low_address = self.seg_addr(delta);
        let high_address = self.seg_addr(delta.wrapping_add(1));
        let low = bus.read_byte(low_address) as u16;
        let high = bus.read_byte(high_address) as u16;
        let value = low | (high << 8);
        self.timing.note_memory_read_word(
            bus,
            self.seg_bases[SegReg16::CS as usize],
            low_address,
            high_address,
            value,
        );
        self.sync_timing_cycles();
        value
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
        if !self.check_segment_access(self.ea_seg, self.eo, 2, true, bus) {
            return;
        }
        let low_address = self.ea;
        let high_address = self.seg_addr(1);
        bus.write_byte(low_address, value as u8);
        bus.write_byte(high_address, (value >> 8) as u8);
        self.timing.note_memory_write_word(
            bus,
            self.seg_bases[SegReg16::CS as usize],
            low_address,
            high_address,
            value,
        );
        self.sync_timing_cycles();
    }

    /// Reads a byte from `seg:offset`, wrapping the offset within 16 bits.
    #[inline(always)]
    fn read_byte_seg(&mut self, bus: &mut impl common::Bus, seg: SegReg16, offset: u16) -> u8 {
        if !self.check_segment_access(seg, offset, 1, false, bus) {
            return 0;
        }
        let base = self.seg_base(seg);
        let address = base.wrapping_add(offset as u32) & ADDRESS_MASK;
        let value = bus.read_byte(address);
        self.timing.note_memory_read_byte(
            bus,
            self.seg_bases[SegReg16::CS as usize],
            address,
            value,
        );
        self.sync_timing_cycles();
        value
    }

    /// Writes a byte to `seg:offset`, wrapping the offset within 16 bits.
    #[inline(always)]
    fn write_byte_seg(
        &mut self,
        bus: &mut impl common::Bus,
        seg: SegReg16,
        offset: u16,
        value: u8,
    ) {
        if !self.check_segment_access(seg, offset, 1, true, bus) {
            return;
        }
        let base = self.seg_base(seg);
        let address = base.wrapping_add(offset as u32) & ADDRESS_MASK;
        bus.write_byte(address, value);
        self.timing.note_memory_write_byte(
            bus,
            self.seg_bases[SegReg16::CS as usize],
            address,
            value,
        );
        self.sync_timing_cycles();
    }

    /// Reads a word from `seg:offset`, wrapping the offset within 16 bits.
    #[inline(always)]
    fn read_word_seg(&mut self, bus: &mut impl common::Bus, seg: SegReg16, offset: u16) -> u16 {
        if !self.check_segment_access(seg, offset, 2, false, bus) {
            return 0;
        }
        let base = self.seg_base(seg);
        let low_address = base.wrapping_add(offset as u32) & ADDRESS_MASK;
        let high_address = base.wrapping_add(offset.wrapping_add(1) as u32) & ADDRESS_MASK;
        let low = bus.read_byte(low_address) as u16;
        let high = bus.read_byte(high_address) as u16;
        let value = low | (high << 8);
        self.timing.note_memory_read_word(
            bus,
            self.seg_bases[SegReg16::CS as usize],
            low_address,
            high_address,
            value,
        );
        self.sync_timing_cycles();
        value
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
        if !self.check_segment_access(seg, offset, 2, true, bus) {
            return;
        }
        let base = self.seg_base(seg);
        let low_address = base.wrapping_add(offset as u32) & ADDRESS_MASK;
        let high_address = base.wrapping_add(offset.wrapping_add(1) as u32) & ADDRESS_MASK;
        bus.write_byte(low_address, value as u8);
        bus.write_byte(high_address, (value >> 8) as u8);
        self.timing.note_memory_write_word(
            bus,
            self.seg_bases[SegReg16::CS as usize],
            low_address,
            high_address,
            value,
        );
        self.sync_timing_cycles();
    }

    fn push(&mut self, bus: &mut impl common::Bus, value: u16) {
        let sp = self.regs.word(WordReg::SP).wrapping_sub(2);
        if !self.check_segment_access(SegReg16::SS, sp, 2, true, bus) {
            return;
        }
        self.regs.set_word(WordReg::SP, sp);
        let base = self.seg_base(SegReg16::SS);
        let low_address = base.wrapping_add(sp as u32) & ADDRESS_MASK;
        let high_address = base.wrapping_add(sp.wrapping_add(1) as u32) & ADDRESS_MASK;
        bus.write_byte(low_address, value as u8);
        bus.write_byte(high_address, (value >> 8) as u8);
        self.timing.note_memory_write_word(
            bus,
            self.seg_bases[SegReg16::CS as usize],
            low_address,
            high_address,
            value,
        );
        self.timing.borrow_internal_cycles(2);
        self.sync_timing_cycles();
    }

    fn pop(&mut self, bus: &mut impl common::Bus) -> u16 {
        let sp = self.regs.word(WordReg::SP);
        if !self.check_segment_access(SegReg16::SS, sp, 2, false, bus) {
            return 0;
        }
        let base = self.seg_base(SegReg16::SS);
        let low_address = base.wrapping_add(sp as u32) & ADDRESS_MASK;
        let high_address = base.wrapping_add(sp.wrapping_add(1) as u32) & ADDRESS_MASK;
        let low = bus.read_byte(low_address) as u16;
        let high = bus.read_byte(high_address) as u16;
        let value = low | (high << 8);
        self.timing.note_memory_read_word(
            bus,
            self.seg_bases[SegReg16::CS as usize],
            low_address,
            high_address,
            value,
        );
        self.timing.borrow_internal_cycles(2);
        self.sync_timing_cycles();
        self.regs.set_word(WordReg::SP, sp.wrapping_add(2));
        value
    }

    fn word_access_is_split(&self, seg: SegReg16, offset: u16) -> bool {
        let base = self.seg_base(seg);
        let low_address = base.wrapping_add(offset as u32) & ADDRESS_MASK;
        let high_address = base.wrapping_add(offset.wrapping_add(1) as u32) & ADDRESS_MASK;
        (low_address.wrapping_add(1) & ADDRESS_MASK) != high_address || low_address & 1 != 0
    }

    fn load_segment(&mut self, seg: SegReg16, selector: u16, bus: &mut impl common::Bus) -> bool {
        if !self.is_protected_mode() {
            self.sregs[seg as usize] = selector;
            self.set_real_segment_cache(seg, selector);
            return true;
        }
        self.load_protected_segment(seg, selector, bus)
    }

    /// Returns the physical address of the descriptor for `selector`, provided
    /// the selector is non-null and falls within the table limit.
    fn descriptor_addr_checked(&self, selector: u16) -> Option<u32> {
        if selector & 0xFFFC == 0 {
            return None;
        }
        let (table_base, table_limit) = if selector & 4 != 0 {
            (self.ldtr_base, self.ldtr_limit)
        } else {
            (self.gdt_base, self.gdt_limit)
        };
        let index = (selector & !7) as u32;
        if index.wrapping_add(7) > table_limit as u32 {
            return None;
        }
        Some(table_base.wrapping_add(index))
    }

    fn set_accessed_bit(&self, selector: u16, bus: &mut impl common::Bus) {
        if let Some(addr) = self.descriptor_addr_checked(selector) {
            let rights = bus.read_byte(addr.wrapping_add(5) & ADDRESS_MASK);
            if rights & 0x01 == 0 {
                bus.write_byte(addr.wrapping_add(5) & ADDRESS_MASK, rights | 0x01);
            }
        }
    }

    fn invalidate_segment_if_needed(&mut self, seg: SegReg16, new_cpl: u16) {
        if !self.seg_valid[seg as usize] {
            return;
        }
        let rights = self.seg_rights[seg as usize];
        if !Self::descriptor_is_segment(rights) {
            self.set_null_segment(seg, 0);
            return;
        }
        if Self::descriptor_is_conforming_code(rights) {
            return;
        }
        let dpl = Self::descriptor_dpl(rights);
        if dpl < new_cpl {
            self.set_null_segment(seg, 0);
        }
    }

    fn read_word_phys(&self, bus: &mut impl common::Bus, addr: u32) -> u16 {
        bus.read_byte(addr & ADDRESS_MASK) as u16
            | ((bus.read_byte(addr.wrapping_add(1) & ADDRESS_MASK) as u16) << 8)
    }

    fn write_word_phys(&self, bus: &mut impl common::Bus, addr: u32, value: u16) {
        bus.write_byte(addr & ADDRESS_MASK, value as u8);
        bus.write_byte(addr.wrapping_add(1) & ADDRESS_MASK, (value >> 8) as u8);
    }

    fn switch_task(&mut self, ntask: u16, task_type: TaskType, bus: &mut impl common::Bus) {
        if ntask & 0x0004 != 0 {
            self.raise_fault_with_code(10, Self::segment_error_code(ntask), bus);
            return;
        }

        let Some(naddr) = self.descriptor_addr_checked(ntask) else {
            self.raise_fault_with_code(10, Self::segment_error_code(ntask), bus);
            return;
        };

        let ndesc = SegmentDescriptor {
            limit: self.read_word_phys(bus, naddr),
            base: bus.read_byte(naddr.wrapping_add(2) & ADDRESS_MASK) as u32
                | ((bus.read_byte(naddr.wrapping_add(3) & ADDRESS_MASK) as u32) << 8)
                | ((bus.read_byte(naddr.wrapping_add(4) & ADDRESS_MASK) as u32) << 16),
            rights: bus.read_byte(naddr.wrapping_add(5) & ADDRESS_MASK),
        };

        let r = ndesc.rights;
        if Self::descriptor_is_segment(r) || (r & 0x0D) != 0x01 {
            self.raise_fault_with_code(13, Self::segment_error_code(ntask), bus);
            return;
        }
        if task_type == TaskType::Iret {
            if r & 0x02 == 0 {
                self.raise_fault_with_code(13, Self::segment_error_code(ntask), bus);
                return;
            }
        } else if r & 0x02 != 0 {
            self.raise_fault_with_code(13, Self::segment_error_code(ntask), bus);
            return;
        }

        if !Self::descriptor_present(r) {
            self.raise_fault_with_code(11, Self::segment_error_code(ntask), bus);
            return;
        }

        if ndesc.limit < 43 {
            self.raise_fault_with_code(10, Self::segment_error_code(ntask), bus);
            return;
        }

        let mut flags = self.flags.compress();

        if task_type == TaskType::Call {
            self.write_word_phys(bus, ndesc.base, self.tr);
        }

        if task_type == TaskType::Iret {
            flags &= !0x4000;
        }

        // Save current state to old TSS.
        let old_base = self.tr_base;
        self.write_word_phys(bus, old_base.wrapping_add(14), self.ip);
        self.write_word_phys(bus, old_base.wrapping_add(16), flags);
        self.write_word_phys(bus, old_base.wrapping_add(18), self.regs.word(WordReg::AX));
        self.write_word_phys(bus, old_base.wrapping_add(20), self.regs.word(WordReg::CX));
        self.write_word_phys(bus, old_base.wrapping_add(22), self.regs.word(WordReg::DX));
        self.write_word_phys(bus, old_base.wrapping_add(24), self.regs.word(WordReg::BX));
        self.write_word_phys(bus, old_base.wrapping_add(26), self.regs.word(WordReg::SP));
        self.write_word_phys(bus, old_base.wrapping_add(28), self.regs.word(WordReg::BP));
        self.write_word_phys(bus, old_base.wrapping_add(30), self.regs.word(WordReg::SI));
        self.write_word_phys(bus, old_base.wrapping_add(32), self.regs.word(WordReg::DI));
        self.write_word_phys(
            bus,
            old_base.wrapping_add(34),
            self.sregs[SegReg16::ES as usize],
        );
        self.write_word_phys(
            bus,
            old_base.wrapping_add(36),
            self.sregs[SegReg16::CS as usize],
        );
        self.write_word_phys(
            bus,
            old_base.wrapping_add(38),
            self.sregs[SegReg16::SS as usize],
        );
        self.write_word_phys(
            bus,
            old_base.wrapping_add(40),
            self.sregs[SegReg16::DS as usize],
        );

        // Read all fields from new TSS.
        let new_base = ndesc.base;
        let ntss_ip = self.read_word_phys(bus, new_base.wrapping_add(14));
        let ntss_flags = self.read_word_phys(bus, new_base.wrapping_add(16));
        let ntss_ax = self.read_word_phys(bus, new_base.wrapping_add(18));
        let ntss_cx = self.read_word_phys(bus, new_base.wrapping_add(20));
        let ntss_dx = self.read_word_phys(bus, new_base.wrapping_add(22));
        let ntss_bx = self.read_word_phys(bus, new_base.wrapping_add(24));
        let ntss_sp = self.read_word_phys(bus, new_base.wrapping_add(26));
        let ntss_bp = self.read_word_phys(bus, new_base.wrapping_add(28));
        let ntss_si = self.read_word_phys(bus, new_base.wrapping_add(30));
        let ntss_di = self.read_word_phys(bus, new_base.wrapping_add(32));
        let ntss_es = self.read_word_phys(bus, new_base.wrapping_add(34));
        let ntss_cs = self.read_word_phys(bus, new_base.wrapping_add(36));
        let ntss_ss = self.read_word_phys(bus, new_base.wrapping_add(38));
        let ntss_ds = self.read_word_phys(bus, new_base.wrapping_add(40));
        let ntss_ldt = self.read_word_phys(bus, new_base.wrapping_add(42));

        // Mark old TSS idle (JMP/IRET).
        if task_type != TaskType::Call
            && let Some(oaddr) = self.descriptor_addr_checked(self.tr)
        {
            let old_rights = bus.read_byte(oaddr.wrapping_add(5) & ADDRESS_MASK);
            bus.write_byte(oaddr.wrapping_add(5) & ADDRESS_MASK, old_rights & !0x02);
        }

        // Mark new TSS busy (CALL/JMP).
        if task_type != TaskType::Iret {
            let new_rights = bus.read_byte(naddr.wrapping_add(5) & ADDRESS_MASK);
            bus.write_byte(naddr.wrapping_add(5) & ADDRESS_MASK, new_rights | 0x02);
        }

        // Update TR.
        self.tr = ntask;
        self.tr_limit = ndesc.limit;
        self.tr_base = ndesc.base;
        self.tr_rights = bus.read_byte(naddr.wrapping_add(5) & ADDRESS_MASK);

        // Load registers from new TSS.
        self.flags.expand(ntss_flags);
        self.regs.set_word(WordReg::AX, ntss_ax);
        self.regs.set_word(WordReg::CX, ntss_cx);
        self.regs.set_word(WordReg::DX, ntss_dx);
        self.regs.set_word(WordReg::BX, ntss_bx);
        self.regs.set_word(WordReg::SP, ntss_sp);
        self.regs.set_word(WordReg::BP, ntss_bp);
        self.regs.set_word(WordReg::SI, ntss_si);
        self.regs.set_word(WordReg::DI, ntss_di);

        // Load LDT from new TSS.
        if ntss_ldt & 0x0004 != 0 {
            self.raise_fault_with_code(10, Self::segment_error_code(ntss_ldt), bus);
            return;
        }
        if ntss_ldt & 0xFFFC != 0 {
            let Some(ldtaddr) = self.descriptor_addr_checked(ntss_ldt) else {
                self.raise_fault_with_code(10, Self::segment_error_code(ntss_ldt), bus);
                return;
            };
            let ldt_desc = SegmentDescriptor {
                limit: self.read_word_phys(bus, ldtaddr),
                base: bus.read_byte(ldtaddr.wrapping_add(2) & ADDRESS_MASK) as u32
                    | ((bus.read_byte(ldtaddr.wrapping_add(3) & ADDRESS_MASK) as u32) << 8)
                    | ((bus.read_byte(ldtaddr.wrapping_add(4) & ADDRESS_MASK) as u32) << 16),
                rights: bus.read_byte(ldtaddr.wrapping_add(5) & ADDRESS_MASK),
            };
            let lr = ldt_desc.rights;
            if Self::descriptor_is_segment(lr) || (lr & 0x0F) != 0x02 {
                self.raise_fault_with_code(10, Self::segment_error_code(ntss_ldt), bus);
                return;
            }
            if !Self::descriptor_present(lr) {
                self.raise_fault_with_code(10, Self::segment_error_code(ntss_ldt), bus);
                return;
            }
            self.ldtr = ntss_ldt;
            self.ldtr_base = ldt_desc.base;
            self.ldtr_limit = ldt_desc.limit;
        } else {
            self.ldtr = 0;
            self.ldtr_base = 0;
            self.ldtr_limit = 0;
        }

        if task_type == TaskType::Call {
            self.flags.nt = true;
        }

        self.msw |= 8;

        // Load segment registers from new TSS. SS first (uses new CS RPL as CPL).
        let new_cpl = ntss_cs & 3;
        self.load_task_data_segment(SegReg16::SS, ntss_ss, new_cpl, bus);
        self.load_task_code_segment(ntss_cs, ntss_ip, bus);
        let cpl = self.cpl();
        self.load_task_data_segment(SegReg16::ES, ntss_es, cpl, bus);
        self.load_task_data_segment(SegReg16::DS, ntss_ds, cpl, bus);
    }

    fn load_task_data_segment(
        &mut self,
        seg: SegReg16,
        selector: u16,
        required_cpl: u16,
        bus: &mut impl common::Bus,
    ) {
        if selector & 0xFFFC == 0 {
            self.set_null_segment(seg, selector);
            return;
        }
        let Some(descriptor) = self.decode_descriptor(selector, bus) else {
            self.raise_fault_with_code(10, Self::segment_error_code(selector), bus);
            return;
        };
        let rights = descriptor.rights;
        if seg == SegReg16::SS {
            if !Self::descriptor_is_segment(rights) || !Self::descriptor_is_writable(rights) {
                self.raise_fault_with_code(10, Self::segment_error_code(selector), bus);
                return;
            }
            let dpl = Self::descriptor_dpl(rights);
            let rpl = selector & 3;
            if dpl != required_cpl || rpl != required_cpl {
                self.raise_fault_with_code(10, Self::segment_error_code(selector), bus);
                return;
            }
        } else {
            if !Self::descriptor_is_segment(rights) || !Self::descriptor_is_readable(rights) {
                self.raise_fault_with_code(10, Self::segment_error_code(selector), bus);
                return;
            }
            if !Self::descriptor_is_conforming_code(rights) {
                let dpl = Self::descriptor_dpl(rights);
                let rpl = selector & 3;
                if dpl < required_cpl.max(rpl) {
                    self.raise_fault_with_code(10, Self::segment_error_code(selector), bus);
                    return;
                }
            }
        }
        if !Self::descriptor_present(rights) {
            self.raise_fault_with_code(10, Self::segment_error_code(selector), bus);
            return;
        }
        self.set_accessed_bit(selector, bus);
        self.set_loaded_segment_cache(seg, selector, descriptor);
    }

    fn load_task_code_segment(&mut self, selector: u16, offset: u16, bus: &mut impl common::Bus) {
        if selector & 0xFFFC == 0 {
            self.raise_fault_with_code(10, Self::segment_error_code(selector), bus);
            return;
        }
        let Some(descriptor) = self.decode_descriptor(selector, bus) else {
            self.raise_fault_with_code(10, Self::segment_error_code(selector), bus);
            return;
        };
        let rights = descriptor.rights;
        if !Self::descriptor_is_segment(rights) || !Self::descriptor_is_code(rights) {
            self.raise_fault_with_code(10, Self::segment_error_code(selector), bus);
            return;
        }
        if !Self::descriptor_present(rights) {
            self.raise_fault_with_code(11, Self::segment_error_code(selector), bus);
            return;
        }
        if offset > descriptor.limit {
            self.raise_fault_with_code(10, 0, bus);
            return;
        }
        self.set_accessed_bit(selector, bus);
        let cpl = selector & 3;
        let adjusted = (selector & !3) | cpl;
        self.set_loaded_segment_cache(SegReg16::CS, adjusted, descriptor);
        self.ip = offset;
    }

    fn code_descriptor(
        &mut self,
        selector: u16,
        offset: u16,
        gate: TaskType,
        old_cs: u16,
        old_ip: u16,
        bus: &mut impl common::Bus,
    ) -> bool {
        let Some(addr) = self.descriptor_addr_checked(selector) else {
            self.raise_fault_with_code(13, Self::segment_error_code(selector), bus);
            return false;
        };

        let desc = SegmentDescriptor {
            limit: self.read_word_phys(bus, addr),
            base: bus.read_byte(addr.wrapping_add(2) & ADDRESS_MASK) as u32
                | ((bus.read_byte(addr.wrapping_add(3) & ADDRESS_MASK) as u32) << 8)
                | ((bus.read_byte(addr.wrapping_add(4) & ADDRESS_MASK) as u32) << 16),
            rights: bus.read_byte(addr.wrapping_add(5) & ADDRESS_MASK),
        };
        let r = desc.rights;
        let cpl = self.cpl();
        let rpl = selector & 3;

        if Self::descriptor_is_segment(r) {
            if !Self::descriptor_is_code(r) {
                self.raise_fault_with_code(13, Self::segment_error_code(selector), bus);
                return false;
            }
            if Self::descriptor_is_conforming_code(r) {
                if Self::descriptor_dpl(r) > cpl {
                    self.raise_fault_with_code(13, Self::segment_error_code(selector), bus);
                    return false;
                }
            } else if rpl > cpl || Self::descriptor_dpl(r) != cpl {
                self.raise_fault_with_code(13, Self::segment_error_code(selector), bus);
                return false;
            }
            if !Self::descriptor_present(r) {
                self.raise_fault_with_code(11, Self::segment_error_code(selector), bus);
                return false;
            }
            if offset > desc.limit {
                self.raise_fault_with_code(13, 0, bus);
                return false;
            }
            self.set_accessed_bit(selector, bus);
            let adjusted = (selector & !3) | cpl;
            self.set_loaded_segment_cache(SegReg16::CS, adjusted, desc);
            self.ip = offset;
            if gate == TaskType::Call {
                self.push(bus, old_cs);
                self.push(bus, old_ip);
            }
            return true;
        }

        // System descriptor: gate DPL must be >= max(CPL, RPL).
        let dpl = Self::descriptor_dpl(r);
        if dpl < cpl.max(rpl) {
            self.raise_fault_with_code(13, Self::segment_error_code(selector), bus);
            return false;
        }
        if !Self::descriptor_present(r) {
            self.raise_fault_with_code(11, Self::segment_error_code(selector), bus);
            return false;
        }

        let gate_type = r & 0x0F;
        match gate_type {
            4 => {
                // Call gate.
                let gate_offset = self.read_word_phys(bus, addr);
                let gate_selector = self.read_word_phys(bus, addr.wrapping_add(2));
                let gate_count = self.read_word_phys(bus, addr.wrapping_add(4)) & 0x1F;

                let Some(target_addr) = self.descriptor_addr_checked(gate_selector) else {
                    self.raise_fault_with_code(13, Self::segment_error_code(gate_selector), bus);
                    return false;
                };
                let target_desc = SegmentDescriptor {
                    limit: self.read_word_phys(bus, target_addr),
                    base: bus.read_byte(target_addr.wrapping_add(2) & ADDRESS_MASK) as u32
                        | ((bus.read_byte(target_addr.wrapping_add(3) & ADDRESS_MASK) as u32) << 8)
                        | ((bus.read_byte(target_addr.wrapping_add(4) & ADDRESS_MASK) as u32)
                            << 16),
                    rights: bus.read_byte(target_addr.wrapping_add(5) & ADDRESS_MASK),
                };
                let tr = target_desc.rights;
                if !Self::descriptor_is_code(tr) || !Self::descriptor_is_segment(tr) {
                    self.raise_fault_with_code(13, Self::segment_error_code(gate_selector), bus);
                    return false;
                }
                let target_dpl = Self::descriptor_dpl(tr);
                if target_dpl > cpl {
                    self.raise_fault_with_code(13, Self::segment_error_code(gate_selector), bus);
                    return false;
                }
                if !Self::descriptor_present(tr) {
                    self.raise_fault_with_code(11, Self::segment_error_code(gate_selector), bus);
                    return false;
                }
                if gate_offset > target_desc.limit {
                    self.raise_fault_with_code(13, 0, bus);
                    return false;
                }

                if !Self::descriptor_is_conforming_code(tr) && target_dpl < cpl {
                    // Inter-privilege call via call gate.
                    if gate == TaskType::Jmp {
                        self.raise_fault_with_code(
                            13,
                            Self::segment_error_code(gate_selector),
                            bus,
                        );
                        return false;
                    }

                    let tss_sp_offset = 2 + target_dpl * 4;
                    let tss_ss_offset = 4 + target_dpl * 4;
                    let tss_sp =
                        self.read_word_phys(bus, self.tr_base.wrapping_add(tss_sp_offset as u32));
                    let tss_ss =
                        self.read_word_phys(bus, self.tr_base.wrapping_add(tss_ss_offset as u32));

                    let saved_ss = self.sregs[SegReg16::SS as usize];
                    let saved_sp = self.regs.word(WordReg::SP);
                    let old_ss_base = self.seg_base(SegReg16::SS);

                    // Load new SS with target DPL as required privilege.
                    self.load_task_data_segment(SegReg16::SS, tss_ss, target_dpl, bus);
                    self.regs.set_word(WordReg::SP, tss_sp);

                    self.push(bus, saved_ss);
                    self.push(bus, saved_sp);
                    for i in (0..gate_count).rev() {
                        let param = self.read_word_phys(
                            bus,
                            old_ss_base.wrapping_add(saved_sp.wrapping_add(i * 2) as u32),
                        );
                        self.push(bus, param);
                    }
                }

                self.set_accessed_bit(gate_selector, bus);
                let adjusted = (gate_selector & !3) | target_dpl;
                self.set_loaded_segment_cache(SegReg16::CS, adjusted, target_desc);
                self.ip = gate_offset;
                if gate == TaskType::Call {
                    self.push(bus, old_cs);
                    self.push(bus, old_ip);
                }
                true
            }
            5 => {
                // Task gate: extract TSS selector and switch.
                let task_selector = self.read_word_phys(bus, addr.wrapping_add(2));
                self.switch_task(task_selector, gate, bus);
                let flags_val = self.flags.compress();
                let cpl = self.cpl();
                self.flags.load_flags(flags_val, cpl, true);
                true
            }
            1 => {
                // Idle TSS descriptor: direct task switch.
                self.switch_task(selector, gate, bus);
                let flags_val = self.flags.compress();
                let cpl = self.cpl();
                self.flags.load_flags(flags_val, cpl, true);
                true
            }
            3 => {
                // Busy TSS: #GP.
                self.raise_fault_with_code(13, Self::segment_error_code(selector), bus);
                false
            }
            _ => {
                self.raise_fault_with_code(13, Self::segment_error_code(selector), bus);
                false
            }
        }
    }

    fn execute_one(&mut self, bus: &mut impl common::Bus) {
        self.prev_ip = self.ip;
        self.timing
            .begin_instruction(self.sregs[SegReg16::CS as usize], self.ip, self.rep_active);

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
        self.ea_class = EaClass::Register;
        self.finish_state = I286FinishState::Linear;

        if self.rep_active {
            self.continue_rep(bus);
        } else {
            let mut opcode = self.fetch(bus);
            let mut prefix_count = 0u8;
            loop {
                match opcode {
                    0x26 => {
                        self.seg_prefix = true;
                        self.prefix_seg = SegReg16::ES;
                        self.timing.note_prefix();
                        prefix_count = prefix_count.saturating_add(1);
                        self.clk_segment_override_prefix(bus);
                        opcode = self.fetch(bus);
                    }
                    0x2E => {
                        self.seg_prefix = true;
                        self.prefix_seg = SegReg16::CS;
                        self.timing.note_prefix();
                        prefix_count = prefix_count.saturating_add(1);
                        self.clk_segment_override_prefix(bus);
                        opcode = self.fetch(bus);
                    }
                    0x36 => {
                        self.seg_prefix = true;
                        self.prefix_seg = SegReg16::SS;
                        self.timing.note_prefix();
                        prefix_count = prefix_count.saturating_add(1);
                        self.clk_segment_override_prefix(bus);
                        opcode = self.fetch(bus);
                    }
                    0x3E => {
                        self.seg_prefix = true;
                        self.prefix_seg = SegReg16::DS;
                        self.timing.note_prefix();
                        prefix_count = prefix_count.saturating_add(1);
                        self.clk_segment_override_prefix(bus);
                        opcode = self.fetch(bus);
                    }
                    0xF0 => {
                        let prefix_count_before_lock = prefix_count;
                        self.timing.note_lock_prefix(prefix_count_before_lock);
                        prefix_count = prefix_count.saturating_add(1);
                        opcode = self.fetch(bus);
                        self.timing.note_lock_prefix_followed_by_prefix(
                            Self::segment_override_prefix(opcode),
                        );
                        let prefetches_during_lock_prefix =
                            self.lock_prefix_prefetches_for_next_opcode(bus, opcode);
                        self.clk_lock_prefix(
                            bus,
                            opcode,
                            prefix_count_before_lock,
                            prefetches_during_lock_prefix,
                        );
                    }
                    _ => {
                        self.dispatch(opcode, bus);
                        break;
                    }
                }
            }
        }

        self.timing
            .finish_instruction(self.ip, self.halted, self.shutdown, self.finish_state);
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
    }

    /// Signals a maskable interrupt request (IRQ).
    pub fn signal_irq(&mut self) {
        self.pending_irq |= crate::PENDING_IRQ;
    }

    /// Seeds the timing model with a warm front-end state so the next
    /// instruction starts from an already-filled queue.
    /// Calling this with empty bytes, zero decoded entries, and a pending
    /// `ControlTransfer` flush preserves the post-reset defaults.
    #[cfg(feature = "verification")]
    pub fn install_front_end_state(
        &mut self,
        prefetch_bytes: &[u8],
        decoded_entries: u8,
        pending_flush: I286FlushState,
    ) {
        self.timing.install_front_end_state(
            self.ip,
            prefetch_bytes,
            decoded_entries,
            pending_flush,
        );
    }
}

impl common::Cpu for I286 {
    crate::impl_cpu_run_for!();

    fn reset(&mut self) {
        // GP registers (AX, BX, CX, DX, SP, BP, SI, DI) are preserved across
        // reset on real 286 hardware. Intel documents them as "undefined", but
        // the register file is not cleared by the RESET signal. The PC-98 VX
        // BIOS relies on SP surviving the warm reset triggered via port 0xF0
        // after testing extended memory in protected mode.

        // Reset segment registers and their caches.
        self.sregs = [0; 4];
        self.set_real_segment_cache(SegReg16::ES, 0);
        self.set_real_segment_cache(SegReg16::CS, 0);
        self.set_real_segment_cache(SegReg16::SS, 0);
        self.set_real_segment_cache(SegReg16::DS, 0);
        self.sregs[SegReg16::CS as usize] = 0xFFFF;
        self.seg_bases[SegReg16::CS as usize] = 0xFFFF0;

        // Reset control registers.
        self.msw = 0xFFF0;
        self.ip = 0;
        self.flags = I286Flags::default();

        // Reset descriptor table registers.
        self.idt_base = 0;
        self.idt_limit = 0x03FF;
        self.gdt_base = 0;
        self.gdt_limit = 0;
        self.ldtr = 0;
        self.ldtr_base = 0;
        self.ldtr_limit = 0;
        self.tr = 0;
        self.tr_base = 0;
        self.tr_limit = 0;
        self.tr_rights = 0;

        // Reset runtime state.
        self.prev_ip = 0;
        self.halted = false;
        self.pending_irq = 0;
        self.no_interrupt = 0;
        self.inhibit_all = 0;
        self.rep_active = false;
        self.rep_restart_ip = 0;
        self.rep_type = 0;
        self.seg_prefix = false;
        self.ea = 0;
        self.eo = 0;
        self.ea_seg = SegReg16::DS;
        self.ea_class = EaClass::Register;
        self.finish_state = I286FinishState::Linear;
        self.trap_level = 0;
        self.shutdown = false;
        self.timing
            .reset(self.sregs[SegReg16::CS as usize], self.ip);
    }

    fn halted(&self) -> bool {
        self.halted || self.shutdown
    }

    fn warm_reset(&mut self, ss: u16, sp: u16, cs: u16, ip: u16) {
        self.reset();
        self.sregs[SegReg16::SS as usize] = ss;
        self.set_real_segment_cache(SegReg16::SS, ss);
        self.state.set_sp(sp);
        self.sregs[SegReg16::CS as usize] = cs;
        self.set_real_segment_cache(SegReg16::CS, cs);
        self.ip = ip;
        self.timing
            .reset(self.sregs[SegReg16::CS as usize], self.ip);
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
    }

    fn flags(&self) -> u16 {
        self.state.compressed_flags()
    }

    fn set_flags(&mut self, v: u16) {
        self.state.set_compressed_flags(v);
    }

    fn cpu_type(&self) -> common::CpuType {
        common::CpuType::I286
    }

    fn load_segment_real_mode(&mut self, seg: common::SegmentRegister, selector: u16) {
        let seg16 = match seg {
            common::SegmentRegister::ES => SegReg16::ES,
            common::SegmentRegister::CS => SegReg16::CS,
            common::SegmentRegister::SS => SegReg16::SS,
            common::SegmentRegister::DS => SegReg16::DS,
        };
        self.state.sregs[seg16 as usize] = selector;
        self.set_real_segment_cache(seg16, selector);
    }

    fn segment_base(&self, seg: common::SegmentRegister) -> u32 {
        let seg16 = match seg {
            common::SegmentRegister::ES => SegReg16::ES,
            common::SegmentRegister::CS => SegReg16::CS,
            common::SegmentRegister::SS => SegReg16::SS,
            common::SegmentRegister::DS => SegReg16::DS,
        };
        self.state.seg_bases[seg16 as usize]
    }
}
