//! T-state cycle Bus Interface Unit for the NEC V20/V30.
//!
//! This module implements the T-state and Ta-state machine shared by both
//! NEC V20 (8-bit data bus, 4-byte prefetch queue, byte fetches) and NEC V30
//! (16-bit data bus, 6-byte prefetch queue, word fetches with single-byte
//! first fetch on odd-target branch).
//!
//! The bus mode is selected by the `MODEL` const-generic on [`VX0`]:
//! - [`V30_V20_COMPAT`](super::V20_BUS) - V20-style 8-bit bus, used by
//!   the cycle-accurate verification suite against V20 SingleStepTests.
//! - [`V30_NATIVE`](super::V30_BUS) - V30 16-bit bus, used in production
//!   for PC-98 emulation.

use common::Bus;

use super::{V20_BUS, V30_BUS, VX0};
use crate::SegReg16;

/// Maximum prefetch queue capacity across all bus modes (V30 has the widest at
/// 6 bytes). The instruction queue array is sized to this constant; the active
/// mode's queue capacity is queried via [`queue_size_for`].
pub const MAX_QUEUE_SIZE: usize = 6;

/// Returns the prefetch queue capacity for the given bus mode.
///
/// V20 mode uses a 4-byte queue. V30 mode is widened to 6 bytes per the
/// NEC V30 datasheet.
#[inline(always)]
pub const fn queue_size_for(model: u8) -> usize {
    match model {
        V20_BUS => 4,
        V30_BUS => 6,
        _ => 4,
    }
}

/// Returns the prefetch fetch width (bytes per m-cycle) for the given bus mode.
///
/// V20 mode fetches one byte per m-cycle (8-bit bus). V30 mode fetches one
/// word per m-cycle when the prefetch IP is even-aligned, and one byte after
/// a branch to an odd target (handled by the BIU's `transfer_size` selection
/// on each fetch, not by this constant).
#[inline(always)]
pub const fn fetch_size_for(model: u8) -> usize {
    match model {
        V20_BUS => 1,
        V30_BUS => 2,
        _ => 1,
    }
}

/// Queue length at which the V30 BIU applies the first policy throttle.
/// V20 has no policy throttle so this constant is meaningful only for V30 mode.
pub const fn policy_len0_for(model: u8) -> usize {
    queue_size_for(model).saturating_sub(2)
}

/// Queue length at which the V30 BIU applies the second policy throttle.
pub const fn policy_len1_for(model: u8) -> usize {
    queue_size_for(model).saturating_sub(3)
}

/// Number of T-states the V30 BIU waits after hitting a policy throttle.
const POLICY_THROTTLE_DELAY: u8 = 3;

/// 20-bit physical address wrap mask (V20/V30 have 1 MiB of address space).
pub const ADDRESS_MASK: u32 = 0xFFFFF;
/// Safety bound on the inner T-state poll loops used by the BIU helpers.
const BIU_LOOP_GUARD: u32 = 64;

/// Main bus cycle T-state.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum TCycle {
    Tinit,
    #[default]
    Ti,
    T1,
    T2,
    T3,
    Tw,
    T4,
}

/// Pipelined address cycle Ta-state.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum TaCycle {
    Tr,
    Ts,
    T0,
    #[default]
    Td,
    Ta,
}

/// Bus cycle status (mapped to S0-S2 pins).
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum BusStatus {
    IoRead,
    IoWrite,
    CodeFetch,
    MemRead,
    MemWrite,
    #[default]
    Passive,
}

/// Prefetch state machine.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum FetchState {
    #[default]
    Normal,
    PausedFull,
    Delayed(u8),
    Suspended,
}

/// EU bus request pending type.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum BusPendingType {
    #[default]
    None,
    EuEarly,
    EuLate,
}

/// Bus transfer size for a single m-cycle.
///
/// V20 mode always issues byte transfers on the 8-bit external bus; word EU
/// operations decompose into two byte m-cycles. V30 mode issues word transfers
/// on aligned addresses (1 m-cycle) and falls back to two byte transfers on
/// odd addresses (2 m-cycles), per the NEC V30 16-bit-bus datasheet.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum TransferSize {
    #[default]
    Byte,
    Word,
}

/// Operand size spanning one or two m-cycles.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum OperandSize {
    #[default]
    Operand8,
    Operand16,
}

/// Whether a queue read is the first byte of an instruction or a subsequent byte.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum QueueType {
    First,
    Subsequent,
}

/// V20/V30 prefetch-queue operation in the active bus cycle.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum QueueOp {
    #[default]
    Idle,
    First,
    Flush,
    Subsequent,
}

state_enum_codec!(TCycle {
    TCycle::Tinit = 0,
    TCycle::Ti = 1,
    TCycle::T1 = 2,
    TCycle::T2 = 3,
    TCycle::T3 = 4,
    TCycle::Tw = 5,
    TCycle::T4 = 6,
});

state_enum_codec!(TaCycle {
    TaCycle::Tr = 0,
    TaCycle::Ts = 1,
    TaCycle::T0 = 2,
    TaCycle::Td = 3,
    TaCycle::Ta = 4,
});

state_enum_codec!(BusStatus {
    BusStatus::IoRead = 0,
    BusStatus::IoWrite = 1,
    BusStatus::CodeFetch = 2,
    BusStatus::MemRead = 3,
    BusStatus::MemWrite = 4,
    BusStatus::Passive = 5,
});

impl save_state::StateEncode for FetchState {
    fn encode_state(&self, output: &mut ::alloc::vec::Vec<u8>) {
        match self {
            Self::Normal => save_state::StateEncode::encode_state(&0u8, output),
            Self::PausedFull => save_state::StateEncode::encode_state(&1u8, output),
            Self::Delayed(cycles) => {
                save_state::StateEncode::encode_state(&2u8, output);
                save_state::StateEncode::encode_state(cycles, output);
            }
            Self::Suspended => save_state::StateEncode::encode_state(&3u8, output),
        }
    }
}

impl save_state::StateDecode for FetchState {
    fn decode_state(
        decoder: &mut save_state::StateDecoder<'_>,
    ) -> Result<Self, save_state::StateDecodeError> {
        match <u8 as save_state::StateDecode>::decode_state(decoder)? {
            0 => Ok(Self::Normal),
            1 => Ok(Self::PausedFull),
            2 => Ok(Self::Delayed(
                <u8 as save_state::StateDecode>::decode_state(decoder)?,
            )),
            3 => Ok(Self::Suspended),
            _ => Err(save_state::StateDecodeError::InvalidTag),
        }
    }
}

state_enum_codec!(BusPendingType {
    BusPendingType::None = 0,
    BusPendingType::EuEarly = 1,
    BusPendingType::EuLate = 2,
});

state_enum_codec!(TransferSize {
    TransferSize::Byte = 0,
    TransferSize::Word = 1,
});

state_enum_codec!(OperandSize {
    OperandSize::Operand8 = 0,
    OperandSize::Operand16 = 1,
});

state_enum_codec!(QueueOp {
    QueueOp::Idle = 0,
    QueueOp::First = 1,
    QueueOp::Flush = 2,
    QueueOp::Subsequent = 3,
});

/// Coarse T-state phase exposed in the cycle trace.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
#[allow(missing_docs)]
pub enum V30BusPhase {
    #[default]
    Ti,
    T1,
    T2,
    T3,
    Tw,
    T4,
}

/// Queue operation reported per cycle.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
#[allow(missing_docs)]
pub enum V30QueueOpTrace {
    #[default]
    Idle,
    First,
    Subsequent,
    Flush,
}

/// Internal Ta-state captured for diagnostics.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
#[allow(missing_docs)]
pub enum V30TaCycle {
    Tr,
    Ts,
    T0,
    #[default]
    Td,
    Ta,
}

#[allow(dead_code)]
impl<const MODEL: u8> VX0<MODEL> {
    #[inline(always)]
    pub(super) fn queue_len(&self) -> usize {
        self.instruction_queue_len
    }

    #[inline(always)]
    pub(super) fn queue_has_room_for_fetch(&self) -> bool {
        self.instruction_queue_len + fetch_size_for(MODEL) <= queue_size_for(MODEL)
    }

    #[inline(always)]
    pub(super) fn queue_pop(&mut self) -> u8 {
        debug_assert!(self.instruction_queue_len > 0, "queue underrun");
        let value = self.instruction_queue[0];
        let queue_length = self.instruction_queue_len;
        if queue_length > 1 {
            self.instruction_queue.copy_within(1..queue_length, 0);
        }
        self.instruction_queue_len -= 1;
        value
    }

    #[inline(always)]
    pub(super) fn queue_push8(&mut self, byte: u8) -> u16 {
        debug_assert!(
            self.instruction_queue_len < queue_size_for(MODEL),
            "queue overrun"
        );
        let queue_length = self.instruction_queue_len;
        self.instruction_queue[queue_length] = byte;
        self.instruction_queue_len += 1;
        1
    }

    /// Push a 16-bit word into the queue at an even address. If `a0` is true,
    /// only the high byte of the word is pushed (odd address fetch). Used by
    /// V30 mode's word fetches; V20 mode pushes one byte at a time via
    /// [`queue_push8`].
    pub(super) fn queue_push16(&mut self, word: u16, a0: bool) -> u16 {
        if a0 {
            self.queue_push8((word >> 8) as u8);
            1
        } else {
            self.queue_push8((word & 0xFF) as u8);
            self.queue_push8((word >> 8) as u8);
            2
        }
    }

    /// Returns true when the queue is at one of the V30 throttle thresholds.
    /// Always false in V20 mode (no throttle).
    #[inline(always)]
    pub(super) fn queue_at_policy_len(&self) -> bool {
        if MODEL != V30_BUS {
            return false;
        }
        self.instruction_queue_len == policy_len0_for(MODEL)
            || self.instruction_queue_len == policy_len1_for(MODEL)
    }

    /// Returns true when the queue is at the deeper V30 throttle threshold.
    /// Always false in V20 mode.
    #[inline(always)]
    pub(super) fn queue_at_policy_threshold(&self) -> bool {
        if MODEL != V30_BUS {
            return false;
        }
        self.instruction_queue_len == policy_len1_for(MODEL)
    }

    /// Extract the byte currently on the active data-bus lane. When BHE is
    /// asserted (V30 16-bit bus, odd-address byte transfer) the byte lives in
    /// the high half of `data_bus`; otherwise in the low half.
    #[inline(always)]
    fn biu_byte_on_lane(&self) -> u8 {
        if self.bhe {
            (self.data_bus >> 8) as u8
        } else {
            self.data_bus as u8
        }
    }

    /// Zero-extend the byte on the active data-bus lane into the low byte of
    /// a u16 (used when combining two byte transfers into a word result).
    #[inline(always)]
    fn biu_byte_on_lane_as_low_half(&self) -> u16 {
        if self.bhe {
            self.data_bus >> 8
        } else {
            self.data_bus & 0x00FF
        }
    }

    /// Shift the byte on the active data-bus lane into the high byte of a
    /// u16 (used when combining two byte transfers into a word result).
    #[inline(always)]
    fn biu_byte_on_lane_as_high_half(&self) -> u16 {
        if self.bhe {
            self.data_bus & 0xFF00
        } else {
            self.data_bus << 8
        }
    }

    /// If BHE is asserted, shift the byte held in the low half of `data_bus`
    /// into the high half so it sits on the active bus lane for an odd-address
    /// byte transfer (V30 16-bit bus only).
    #[inline(always)]
    fn biu_align_byte_to_lane(&mut self) {
        if self.bhe {
            self.data_bus <<= 8;
        }
    }

    pub(super) fn queue_flush(&mut self) {
        self.instruction_queue_len = 0;
        self.instruction_preload = None;
        self.fetch_state = FetchState::Normal;
        self.queue_op = QueueOp::Flush;
    }

    fn bus_phase_from(t_cycle: TCycle) -> V30BusPhase {
        match t_cycle {
            TCycle::Tinit | TCycle::T1 => V30BusPhase::T1,
            TCycle::T2 => V30BusPhase::T2,
            TCycle::T3 => V30BusPhase::T3,
            TCycle::Tw => V30BusPhase::Tw,
            TCycle::T4 => V30BusPhase::T4,
            TCycle::Ti => V30BusPhase::Ti,
        }
    }

    fn current_bus_phase(&self) -> V30BusPhase {
        Self::bus_phase_from(self.t_cycle)
    }

    fn current_trace_queue_op(&self) -> V30QueueOpTrace {
        match self.last_queue_op {
            QueueOp::First => V30QueueOpTrace::First,
            QueueOp::Subsequent => V30QueueOpTrace::Subsequent,
            QueueOp::Flush => V30QueueOpTrace::Flush,
            QueueOp::Idle => V30QueueOpTrace::Idle,
        }
    }

    fn current_trace_data(&self) -> u8 {
        if self.bus_status_latch != BusStatus::Passive
            && matches!(self.current_bus_phase(), V30BusPhase::T3 | V30BusPhase::Tw)
        {
            self.data_bus as u8
        } else {
            0
        }
    }

    fn current_trace_ta_cycle(&self) -> V30TaCycle {
        match self.ta_cycle {
            TaCycle::Tr => V30TaCycle::Tr,
            TaCycle::Ts => V30TaCycle::Ts,
            TaCycle::T0 => V30TaCycle::T0,
            TaCycle::Td => V30TaCycle::Td,
            TaCycle::Ta => V30TaCycle::Ta,
        }
    }

    /// Advance the CPU by one T-state. This is the unit of time used by the
    /// T-state BIU. Every EU or BIU operation ultimately reduces to a sequence
    /// of `cycle` calls.
    pub(super) fn cycle(&mut self, bus: &mut impl Bus) {
        // `entered_from_tinit` controls real state-transition logic below
        // (forces t_cycle to T1 for this cycle and overrides the late
        // dispatch). It is also reused for the trace snapshot when capture
        // is enabled.
        let entered_from_tinit = self.t_cycle == TCycle::Tinit;
        if entered_from_tinit {
            self.t_cycle = TCycle::T1;
        }

        self.cycles_remaining -= 1;

        match self.bus_status_latch {
            BusStatus::Passive => {
                self.transfer_n = 0;
                match self.fetch_state {
                    FetchState::Delayed(0) => {
                        self.fetch_state = FetchState::Normal;
                        self.biu_make_fetch_decision();
                    }
                    FetchState::PausedFull if self.queue_has_room_for_fetch() => {
                        self.biu_make_fetch_decision();
                    }
                    _ => {}
                }
            }
            BusStatus::MemRead
            | BusStatus::MemWrite
            | BusStatus::IoRead
            | BusStatus::IoWrite
            | BusStatus::CodeFetch => match self.t_cycle {
                TCycle::Tinit => unreachable!("Tinit was translated to T1 above"),
                TCycle::Ti => {
                    self.biu_make_fetch_decision();
                }
                TCycle::T1 => {}
                TCycle::T2 => {
                    if self.final_transfer {
                        self.biu_make_fetch_decision();
                    }
                }
                TCycle::T3 | TCycle::Tw => {
                    self.biu_do_bus_transfer(bus);
                }
                TCycle::T4 => {
                    if self.bus_status_latch == BusStatus::CodeFetch {
                        let inc = match self.transfer_size {
                            TransferSize::Byte => self.queue_push8(self.data_bus as u8),
                            TransferSize::Word => {
                                self.queue_push16(self.data_bus, self.prefetch_ip & 1 == 1)
                            }
                        };
                        self.prefetch_ip = self.prefetch_ip.wrapping_add(inc);
                    }
                    if self.final_transfer {
                        self.biu_make_fetch_decision();
                    }
                }
            },
        }

        if let FetchState::Delayed(count) = self.fetch_state
            && self.t_cycle != TCycle::Tw
            && count > 0
        {
            self.fetch_state = FetchState::Delayed(count - 1);
        }

        self.ta_cycle = match self.ta_cycle {
            TaCycle::Tr => TaCycle::Ts,
            TaCycle::Ts => TaCycle::T0,
            TaCycle::T0 => match (self.pl_status, self.bus_pending) {
                (BusStatus::CodeFetch, BusPendingType::None) => {
                    if matches!(self.t_cycle, TCycle::Ti | TCycle::T4)
                        && self.fetch_state != FetchState::Suspended
                    {
                        self.biu_bus_begin_fetch();
                        TaCycle::Td
                    } else {
                        TaCycle::T0
                    }
                }
                (BusStatus::CodeFetch, BusPendingType::EuLate) => {
                    if matches!(self.t_cycle, TCycle::Ti | TCycle::T4) {
                        self.biu_fetch_abort();
                        self.ta_cycle
                    } else {
                        TaCycle::T0
                    }
                }
                _ => {
                    if matches!(self.t_cycle, TCycle::Ti | TCycle::T4) {
                        TaCycle::Td
                    } else {
                        TaCycle::T0
                    }
                }
            },
            TaCycle::Td => TaCycle::Td,
            TaCycle::Ta => TaCycle::Ta,
        };

        self.t_cycle = if entered_from_tinit {
            TCycle::T1
        } else {
            match self.t_cycle {
                TCycle::Tinit => TCycle::T1,
                TCycle::Ti => match self.bus_status_latch {
                    BusStatus::Passive => TCycle::Ti,
                    _ => TCycle::T1,
                },
                TCycle::T1 => match self.bus_status_latch {
                    BusStatus::Passive => TCycle::T1,
                    _ => TCycle::T2,
                },
                TCycle::T2 => TCycle::T3,
                TCycle::Tw | TCycle::T3 => {
                    self.biu_bus_end();
                    TCycle::T4
                }
                TCycle::T4 => {
                    self.bus_status_latch = BusStatus::Passive;
                    TCycle::Ti
                }
            }
        };

        self.last_queue_op = self.queue_op;
        self.queue_op = QueueOp::Idle;
        self.last_queue_byte = 0;
    }

    #[inline(always)]
    pub(super) fn cycles(&mut self, bus: &mut impl Bus, count: u32) {
        for _ in 0..count {
            self.cycle(bus);
        }
    }

    fn biu_bus_end(&mut self) {}

    fn biu_do_bus_transfer(&mut self, bus: &mut impl Bus) {
        match (self.bus_status_latch, self.transfer_size) {
            (BusStatus::CodeFetch, TransferSize::Byte) => {
                let byte = bus.fetch_opcode_byte(self.address_latch & ADDRESS_MASK);
                self.data_bus = byte as u16;
            }
            (BusStatus::CodeFetch, TransferSize::Word) => {
                let base = self.address_latch & !1;
                self.data_bus = bus.fetch_opcode_word(base & ADDRESS_MASK);
            }
            (BusStatus::MemRead, TransferSize::Byte) => {
                let byte = bus.read_byte(self.address_latch & ADDRESS_MASK);
                self.data_bus = byte as u16;
                self.biu_align_byte_to_lane();
            }
            (BusStatus::MemRead, TransferSize::Word) => {
                let base = self.address_latch & !1;
                let lo = bus.read_byte(base & ADDRESS_MASK) as u16;
                let hi = bus.read_byte(base.wrapping_add(1) & ADDRESS_MASK) as u16;
                self.data_bus = lo | (hi << 8);
            }
            (BusStatus::MemWrite, TransferSize::Byte) => {
                let byte = self.biu_byte_on_lane();
                bus.write_byte(self.address_latch & ADDRESS_MASK, byte);
            }
            (BusStatus::MemWrite, TransferSize::Word) => {
                let base = self.address_latch & !1;
                bus.write_byte(base & ADDRESS_MASK, self.data_bus as u8);
                bus.write_byte(
                    base.wrapping_add(1) & ADDRESS_MASK,
                    (self.data_bus >> 8) as u8,
                );
            }
            (BusStatus::IoRead, TransferSize::Byte) => {
                let port = (self.address_latch & 0xFFFF) as u16;
                let byte = bus.io_read_byte(port);
                self.data_bus = byte as u16;
                self.biu_align_byte_to_lane();
            }
            (BusStatus::IoRead, TransferSize::Word) => {
                let port = (self.address_latch & 0xFFFF) as u16;
                self.data_bus = bus.io_read_word(port);
            }
            (BusStatus::IoWrite, TransferSize::Byte) => {
                let port = (self.address_latch & 0xFFFF) as u16;
                let byte = self.biu_byte_on_lane();
                bus.io_write_byte(port, byte);
            }
            (BusStatus::IoWrite, TransferSize::Word) => {
                let port = (self.address_latch & 0xFFFF) as u16;
                bus.io_write_word(port, self.data_bus);
            }
            (BusStatus::Passive, _) => {}
        }
        self.bus_status = BusStatus::Passive;
    }

    fn biu_address_start(&mut self, new_bus_status: BusStatus) {
        self.ta_cycle = if self.ta_cycle == TaCycle::Ta {
            TaCycle::Ts
        } else {
            TaCycle::Tr
        };
        self.pl_status = new_bus_status;
    }

    /// Shared body of the two code-fetch entry points. Programs the BIU bus
    /// state for a CS:IP fetch m-cycle. No precondition asserts: callers that
    /// need them (`biu_start_code_fetch_for_eu`) check before delegating.
    fn biu_start_code_fetch_inner(&mut self) {
        if self.queue_has_room_for_fetch() {
            self.operand_size = match fetch_size_for(MODEL) {
                1 => OperandSize::Operand8,
                _ => OperandSize::Operand16,
            };
            self.fetch_state = FetchState::Normal;
            self.pl_status = BusStatus::Passive;
            self.bus_status = BusStatus::CodeFetch;
            self.bus_status_latch = BusStatus::CodeFetch;
            self.t_cycle = TCycle::Tinit;
            let cs_base = self.seg_base(SegReg16::CS);
            self.address_latch = cs_base.wrapping_add(self.prefetch_ip as u32) & ADDRESS_MASK;
            self.address_bus = self.address_latch;
            self.transfer_size = if MODEL == V30_BUS && self.prefetch_ip & 1 == 0 {
                TransferSize::Word
            } else {
                TransferSize::Byte
            };
            self.bhe = false;
            self.transfer_n = 1;
            self.final_transfer = true;
        }
    }

    fn biu_bus_begin_fetch(&mut self) {
        self.biu_start_code_fetch_inner();
    }

    fn biu_fetch_abort(&mut self) {
        self.ta_cycle = TaCycle::Ta;
    }

    pub(super) fn biu_fetch_suspend(&mut self, bus: &mut impl Bus) {
        self.fetch_state = FetchState::Suspended;
        if self.bus_status_latch == BusStatus::CodeFetch {
            self.biu_bus_wait_finish(bus);
        }
        self.ta_cycle = TaCycle::Td;
        self.pl_status = BusStatus::Passive;
    }

    pub(super) fn biu_fetch_suspend_with_ready_memory_read(&mut self, bus: &mut impl Bus) {
        self.biu_fetch_suspend(bus);
        self.biu_ready_memory_read();
    }

    pub(super) fn biu_fetch_suspend_after_pending_fetch(&mut self, bus: &mut impl Bus) {
        if self.t_cycle == TCycle::Ti
            && self.bus_status_latch == BusStatus::Passive
            && self.pl_status == BusStatus::CodeFetch
        {
            let mut guard = 0u32;
            while self.bus_status_latch == BusStatus::Passive
                && self.pl_status == BusStatus::CodeFetch
                && guard < BIU_LOOP_GUARD
            {
                self.cycle(bus);
                guard += 1;
            }
        }
        self.fetch_state = FetchState::Suspended;
        if self.bus_status_latch == BusStatus::CodeFetch {
            self.biu_bus_wait_finish(bus);
        }
        self.ta_cycle = TaCycle::Td;
        self.pl_status = BusStatus::Passive;
    }

    pub(super) fn biu_fetch_resume_immediate_for_eu(&mut self) {
        if self.fetch_state == FetchState::Suspended {
            self.fetch_state = FetchState::Normal;
            self.biu_complete_current_bus_for_eu();
            self.biu_start_code_fetch_for_eu();
        }
    }

    pub(super) fn biu_prefetch_before_pointer_segment_read(&mut self, bus: &mut impl Bus) {
        if self.fetch_state == FetchState::Suspended {
            self.fetch_state = FetchState::Normal;
        }
        if self.t_cycle == TCycle::T4
            && self.bus_status_latch != BusStatus::CodeFetch
            && self.queue_has_room_for_fetch()
        {
            self.biu_complete_current_bus_for_eu();
            self.biu_start_code_fetch_for_eu();
            self.biu_bus_wait_finish(bus);
            self.biu_complete_code_fetch_for_eu();
            self.biu_ready_memory_read();
        }
    }

    pub(super) fn biu_queue_flush(&mut self) {
        self.queue_flush();
        self.fetch_state = FetchState::Normal;
        self.biu_fetch_start();
    }

    pub(super) fn biu_queue_flush_early(&mut self) {
        self.queue_flush();
        self.fetch_state = FetchState::Normal;
        self.biu_fetch_start();
        if self.pl_status == BusStatus::CodeFetch && self.ta_cycle == TaCycle::Tr {
            self.ta_cycle = TaCycle::Ts;
        }
    }

    fn biu_fetch_start(&mut self) {
        if self.bus_pending == BusPendingType::EuEarly || self.pl_status == BusStatus::CodeFetch {
            return;
        }
        match self.fetch_state {
            FetchState::Delayed(_) => {}
            _ => {
                self.fetch_state = FetchState::Normal;
                self.biu_address_start(BusStatus::CodeFetch);
            }
        }
    }

    fn biu_make_fetch_decision(&mut self) {
        if !self.queue_has_room_for_fetch() {
            self.fetch_state = FetchState::PausedFull;
            return;
        }
        if self.bus_pending == BusPendingType::EuEarly {
            return;
        }
        if self.fetch_state == FetchState::Suspended {
            return;
        }
        if MODEL == V30_BUS
            && self.queue_at_policy_len()
            && self.bus_status_latch == BusStatus::CodeFetch
        {
            if self.ta_cycle == TaCycle::Td && !matches!(self.fetch_state, FetchState::Delayed(_)) {
                self.fetch_state = FetchState::Delayed(POLICY_THROTTLE_DELAY);
            }
        } else if self.ta_cycle == TaCycle::Td {
            self.biu_fetch_start();
        }
    }

    pub(super) fn biu_instruction_entry_queue_len_for_timing(&self) -> usize {
        self.instruction_entry_queue_bytes as usize
    }

    pub(super) fn biu_latch_is_code_fetch(&self) -> bool {
        self.bus_status_latch == BusStatus::CodeFetch
    }

    pub(super) fn biu_prepare_memory_read(&mut self) {
        self.biu_address_start(BusStatus::MemRead);
    }

    pub(super) fn biu_prepare_memory_write(&mut self) {
        self.biu_address_start(BusStatus::MemWrite);
    }

    pub(super) fn biu_prepare_memory_write_from_ts(&mut self) {
        self.ta_cycle = TaCycle::Ts;
        self.pl_status = BusStatus::MemWrite;
    }

    pub(super) fn biu_ready_memory_read(&mut self) {
        self.pl_status = BusStatus::MemRead;
        self.ta_cycle = TaCycle::Td;
    }

    pub(super) fn biu_ready_io_read(&mut self) {
        self.pl_status = BusStatus::IoRead;
        self.ta_cycle = TaCycle::Td;
    }

    pub(super) fn biu_ready_memory_write(&mut self) {
        self.pl_status = BusStatus::MemWrite;
        self.ta_cycle = TaCycle::Td;
    }

    pub(super) fn biu_prefetch_before_rmw_write(&mut self, bus: &mut impl Bus) {
        if self.bus_status_latch == BusStatus::CodeFetch {
            self.biu_bus_wait_finish(bus);
            self.biu_complete_code_fetch_for_eu();
            self.biu_ready_memory_write();
            return;
        }
        if self.t_cycle == TCycle::Ti
            && self.bus_status_latch == BusStatus::Passive
            && self.pl_status == BusStatus::CodeFetch
        {
            self.biu_ready_memory_write();
            return;
        }

        if self.t_cycle != TCycle::T4
            || self.bus_status_latch == BusStatus::CodeFetch
            || self.pl_status != BusStatus::CodeFetch
            || !self.queue_has_room_for_fetch()
        {
            return;
        }

        self.cycle(bus);
        if self.bus_status_latch == BusStatus::CodeFetch {
            self.biu_bus_wait_finish(bus);
            self.biu_complete_code_fetch_for_eu();
            self.biu_ready_memory_write();
        }
    }

    pub(super) fn biu_prefetch_after_immediate_before_rmw_write(&mut self, bus: &mut impl Bus) {
        if self.bus_status_latch == BusStatus::CodeFetch {
            self.biu_bus_wait_finish(bus);
            self.biu_complete_code_fetch_for_eu();
        } else if self.t_cycle == TCycle::Ti
            && self.bus_status_latch == BusStatus::Passive
            && self.pl_status == BusStatus::CodeFetch
        {
            let mut guard = 0u32;
            while self.bus_status_latch == BusStatus::Passive
                && self.pl_status == BusStatus::CodeFetch
                && guard < BIU_LOOP_GUARD
            {
                self.cycle(bus);
                guard += 1;
            }
            if self.bus_status_latch == BusStatus::CodeFetch {
                self.biu_bus_wait_finish(bus);
                self.biu_complete_code_fetch_for_eu();
            }
        } else if self.t_cycle == TCycle::T4
            && self.bus_status_latch == BusStatus::Passive
            && self.pl_status == BusStatus::CodeFetch
            && self.queue_has_room_for_fetch()
        {
            self.cycle(bus);
            if self.bus_status_latch == BusStatus::CodeFetch {
                self.biu_bus_wait_finish(bus);
                self.biu_complete_code_fetch_for_eu();
            }
        }

        if self.t_cycle == TCycle::Ti
            && self.bus_status_latch == BusStatus::Passive
            && self.queue_has_room_for_fetch()
        {
            self.biu_start_code_fetch_for_eu();
            self.biu_bus_wait_finish(bus);
            self.biu_complete_code_fetch_for_eu();
        }
        self.biu_ready_memory_write();
    }

    /// Shared body of the two `biu_complete_code_fetch_*` entry points: pushes
    /// the just-fetched bytes into the queue, advances the prefetch IP, and
    /// clears the bus state. The two callers diverge afterwards: one resets
    /// `t_cycle` to `Ti` (no immediate EU bus follows), the other sets
    /// `bus_pending = EuEarly` so an EU bus cycle can take over.
    fn biu_complete_code_fetch_inner(&mut self) -> bool {
        if self.t_cycle != TCycle::T4 || self.bus_status_latch != BusStatus::CodeFetch {
            return false;
        }
        let inc = match self.transfer_size {
            TransferSize::Byte => self.queue_push8(self.data_bus as u8),
            TransferSize::Word => self.queue_push16(self.data_bus, self.prefetch_ip & 1 == 1),
        };
        self.prefetch_ip = self.prefetch_ip.wrapping_add(inc);
        self.bus_status = BusStatus::Passive;
        self.bus_status_latch = BusStatus::Passive;
        self.ta_cycle = TaCycle::Td;
        self.pl_status = BusStatus::Passive;
        if !self.queue_has_room_for_fetch() {
            self.fetch_state = FetchState::PausedFull;
        }
        true
    }

    pub(super) fn biu_complete_code_fetch_for_eu(&mut self) {
        if self.biu_complete_code_fetch_inner() {
            self.t_cycle = TCycle::Ti;
        }
    }

    pub(super) fn biu_complete_current_bus_for_eu(&mut self) {
        if self.t_cycle == TCycle::T4 && self.bus_status_latch != BusStatus::CodeFetch {
            self.bus_status = BusStatus::Passive;
            self.bus_status_latch = BusStatus::Passive;
            self.t_cycle = TCycle::Ti;
            self.ta_cycle = TaCycle::Td;
            self.pl_status = BusStatus::Passive;
        }
    }

    pub(super) fn biu_delay_next_eu_address_from_t4(&mut self) {
        debug_assert_eq!(self.t_cycle, TCycle::T4);
        self.pl_status = BusStatus::Passive;
        self.bus_pending = BusPendingType::None;
        self.ta_cycle = TaCycle::Ts;
    }

    pub(super) fn biu_complete_code_fetch_for_immediate_eu_bus(&mut self) {
        if self.biu_complete_code_fetch_inner() {
            self.bus_pending = BusPendingType::EuEarly;
        }
    }

    pub(super) fn biu_start_code_fetch_for_eu(&mut self) {
        debug_assert_eq!(self.bus_status_latch, BusStatus::Passive);
        debug_assert_eq!(self.t_cycle, TCycle::Ti);
        self.biu_start_code_fetch_inner();
    }

    pub(super) fn biu_complete_code_fetch_and_start_for_eu(&mut self, bus: &mut impl Bus) {
        if self.bus_status_latch == BusStatus::CodeFetch {
            self.biu_bus_wait_finish(bus);
            self.biu_complete_code_fetch_for_eu();
        }
        self.biu_start_code_fetch_for_eu();
    }

    pub(super) fn biu_queue_flush_and_start_code_fetch_for_eu(&mut self) {
        self.queue_flush();
        self.fetch_state = FetchState::Normal;
        self.biu_start_code_fetch_for_eu();
    }

    #[inline]
    pub(super) fn biu_bus_wait_finish(&mut self, bus: &mut impl Bus) {
        if self.bus_status_latch != BusStatus::Passive {
            let mut guard = 0u32;
            while self.t_cycle != TCycle::T4 && guard < BIU_LOOP_GUARD {
                self.cycle(bus);
                guard += 1;
            }
        }
    }

    #[inline]
    fn biu_bus_wait_until_tx(&mut self, bus: &mut impl Bus) {
        if matches!(
            self.bus_status_latch,
            BusStatus::MemRead
                | BusStatus::MemWrite
                | BusStatus::IoRead
                | BusStatus::IoWrite
                | BusStatus::CodeFetch
        ) {
            let mut guard = 0u32;
            while self.t_cycle != TCycle::T3 && guard < BIU_LOOP_GUARD {
                self.cycle(bus);
                guard += 1;
            }
        }
    }

    #[inline]
    fn biu_bus_wait_delay(&mut self, bus: &mut impl Bus) -> bool {
        match self.fetch_state {
            FetchState::Delayed(0) => {
                self.ta_cycle = TaCycle::Td;
                true
            }
            FetchState::Delayed(delay) => {
                self.cycles(bus, delay as u32);
                self.ta_cycle = TaCycle::Ta;
                true
            }
            _ => false,
        }
    }

    #[inline]
    fn biu_bus_wait_address(&mut self, bus: &mut impl Bus) {
        let mut guard = 0u32;
        while !matches!(self.ta_cycle, TaCycle::Td | TaCycle::Ta) && guard < BIU_LOOP_GUARD {
            self.cycle(bus);
            guard += 1;
        }
    }

    fn biu_complete_pending_fetch_before_eu(
        &mut self,
        bus: &mut impl Bus,
        new_bus_status: BusStatus,
    ) -> bool {
        if self.t_cycle != TCycle::Ti
            || self.pl_status != BusStatus::CodeFetch
            || self.bus_status_latch != BusStatus::Passive
        {
            return false;
        }

        let mut guard = 0u32;
        while self.bus_status_latch == BusStatus::Passive
            && self.pl_status == BusStatus::CodeFetch
            && guard < BIU_LOOP_GUARD
        {
            self.cycle(bus);
            guard += 1;
        }

        if self.bus_status_latch != BusStatus::CodeFetch {
            return false;
        }

        self.bus_pending = BusPendingType::EuEarly;
        self.biu_address_start(new_bus_status);
        self.biu_bus_wait_finish(bus);
        self.biu_complete_code_fetch_for_eu();
        true
    }

    fn biu_complete_first_byte_fetch_handoff_before_eu(
        &mut self,
        bus: &mut impl Bus,
        new_bus_status: BusStatus,
    ) -> bool {
        if new_bus_status != BusStatus::MemWrite
            || self.bus_status_latch != BusStatus::CodeFetch
            || !matches!(self.t_cycle, TCycle::T1 | TCycle::T2)
            || self.last_queue_op != QueueOp::First
        {
            return false;
        }

        self.biu_bus_wait_finish(bus);
        if self.t_cycle == TCycle::T4 && self.pl_status == BusStatus::CodeFetch {
            self.cycle(bus);
        }
        if self.bus_status_latch != BusStatus::CodeFetch {
            return false;
        }

        self.bus_pending = BusPendingType::EuEarly;
        self.biu_address_start(new_bus_status);
        self.biu_bus_wait_finish(bus);
        self.biu_complete_code_fetch_for_eu();
        true
    }

    /// Begin an EU bus m-cycle. Handles bus-cycle synchronisation, fetch
    /// aborts, and address cycle setup before starting T1.
    #[allow(clippy::too_many_arguments)]
    fn biu_bus_begin(
        &mut self,
        bus: &mut impl Bus,
        new_bus_status: BusStatus,
        address: u32,
        data: u16,
        size: TransferSize,
        op_size: OperandSize,
        first: bool,
    ) {
        debug_assert_ne!(
            new_bus_status,
            BusStatus::CodeFetch,
            "biu_bus_begin cannot start a code fetch"
        );

        let mut fetch_abort = false;
        let cold_first_byte_read_handoff = matches!(
            new_bus_status,
            BusStatus::MemRead | BusStatus::IoRead | BusStatus::IoWrite
        ) && self.bus_status_latch == BusStatus::CodeFetch
            && matches!(self.t_cycle, TCycle::T1 | TCycle::T2)
            && self.last_queue_op == QueueOp::First;

        let eu_address_pipelined = self
            .biu_complete_first_byte_fetch_handoff_before_eu(bus, new_bus_status)
            || self.biu_complete_pending_fetch_before_eu(bus, new_bus_status);

        match self.t_cycle {
            TCycle::Ti if eu_address_pipelined => {}
            TCycle::Ti if self.pl_status == new_bus_status => {}
            TCycle::Ti => {
                self.biu_address_start(new_bus_status);
            }
            TCycle::T1 | TCycle::T2 => {
                self.bus_pending = BusPendingType::EuEarly;
                self.biu_address_start(new_bus_status);
            }
            _ => {
                if self.pl_status == BusStatus::CodeFetch {
                    self.bus_pending = BusPendingType::EuLate;
                    fetch_abort = true;
                } else if !self.final_transfer {
                    self.bus_pending = BusPendingType::EuEarly;
                }
            }
        }

        self.biu_bus_wait_finish(bus);
        if self.bus_pending == BusPendingType::EuEarly
            && self.t_cycle == TCycle::T4
            && self.bus_status_latch == BusStatus::CodeFetch
            && !cold_first_byte_read_handoff
        {
            self.biu_complete_code_fetch_for_eu();
        }
        let was_delay = self.biu_bus_wait_delay(bus);
        self.biu_bus_wait_address(bus);
        if cold_first_byte_read_handoff
            && self.t_cycle == TCycle::Ti
            && self.bus_status_latch == BusStatus::Passive
        {
            self.cycle(bus);
        }

        if was_delay || fetch_abort {
            self.biu_address_start(new_bus_status);
            self.biu_bus_wait_address(bus);
        }

        if self.t_cycle == TCycle::T4
            && self.bus_status_latch != BusStatus::CodeFetch
            && self.bus_pending != BusPendingType::EuEarly
        {
            self.cycle(bus);
        }

        match size {
            TransferSize::Word => {
                self.transfer_n = 1;
                self.final_transfer = true;
            }
            TransferSize::Byte => {
                if first {
                    match op_size {
                        OperandSize::Operand8 => {
                            self.transfer_n = 1;
                            self.final_transfer = true;
                        }
                        OperandSize::Operand16 => {
                            self.transfer_n = 1;
                            self.final_transfer = false;
                        }
                    }
                } else {
                    self.transfer_n = 2;
                    self.final_transfer = true;
                }
            }
        }

        // BHE asserted on V30 16-bit bus when accessing the high lane: word
        // transfers always assert it, byte transfers assert it for odd
        // addresses. V20 8-bit bus has no BHE so this stays false.
        self.bhe = MODEL == V30_BUS && (matches!(size, TransferSize::Word) || (address & 1 != 0));
        self.bus_pending = BusPendingType::None;
        self.pl_status = BusStatus::Passive;
        self.bus_status = new_bus_status;
        self.bus_status_latch = new_bus_status;
        self.t_cycle = TCycle::Tinit;
        self.address_bus = address;
        self.address_latch = address;
        self.data_bus = data;
        self.transfer_size = size;
        self.operand_size = op_size;

        if matches!(size, TransferSize::Byte) {
            self.biu_align_byte_to_lane();
        }
    }

    pub(super) fn biu_read_u8_physical(&mut self, bus: &mut impl Bus, address: u32) -> u8 {
        let wrapped_address = address & ADDRESS_MASK;
        self.biu_bus_begin(
            bus,
            BusStatus::MemRead,
            wrapped_address,
            0,
            TransferSize::Byte,
            OperandSize::Operand8,
            true,
        );
        self.biu_bus_wait_finish(bus);
        self.biu_byte_on_lane()
    }

    pub(super) fn biu_write_u8_physical(&mut self, bus: &mut impl Bus, address: u32, byte: u8) {
        let wrapped_address = address & ADDRESS_MASK;
        self.biu_bus_begin(
            bus,
            BusStatus::MemWrite,
            wrapped_address,
            byte as u16,
            TransferSize::Byte,
            OperandSize::Operand8,
            true,
        );
        self.biu_bus_wait_until_tx(bus);
    }

    pub(super) fn biu_chain_eu_transfer(&mut self) {
        if matches!(self.t_cycle, TCycle::T3 | TCycle::T4)
            && self.bus_status_latch != BusStatus::CodeFetch
        {
            self.pl_status = BusStatus::Passive;
            self.ta_cycle = TaCycle::Td;
            self.final_transfer = false;
        }
    }

    /// Read a word as two byte m-cycles. Used by V20 mode unconditionally
    /// (8-bit external bus) and by V30 mode when the operand straddles an
    /// odd address (16-bit bus falls back to two byte cycles).
    fn biu_read_word_pair(
        &mut self,
        bus: &mut impl Bus,
        status: BusStatus,
        lo_address: u32,
        hi_address: u32,
    ) -> u16 {
        self.biu_bus_begin(
            bus,
            status,
            lo_address,
            0,
            TransferSize::Byte,
            OperandSize::Operand16,
            true,
        );
        self.biu_bus_wait_finish(bus);
        let lo = self.biu_byte_on_lane_as_low_half();

        self.biu_bus_begin(
            bus,
            status,
            hi_address,
            0,
            TransferSize::Byte,
            OperandSize::Operand16,
            false,
        );
        self.biu_bus_wait_finish(bus);
        let hi = self.biu_byte_on_lane_as_high_half();
        lo | hi
    }

    /// Write a word as two byte m-cycles. Used by V20 mode unconditionally
    /// and by V30 mode when the operand straddles an odd address.
    fn biu_write_word_pair(
        &mut self,
        bus: &mut impl Bus,
        status: BusStatus,
        lo_address: u32,
        hi_address: u32,
        word: u16,
    ) {
        self.biu_bus_begin(
            bus,
            status,
            lo_address,
            word & 0x00FF,
            TransferSize::Byte,
            OperandSize::Operand16,
            true,
        );
        self.biu_bus_wait_until_tx(bus);

        self.biu_bus_begin(
            bus,
            status,
            hi_address,
            (word >> 8) & 0x00FF,
            TransferSize::Byte,
            OperandSize::Operand16,
            false,
        );
        self.biu_bus_wait_until_tx(bus);
    }

    /// V30 mode: read a word in a single aligned m-cycle (16-bit bus).
    fn biu_read_word_aligned(
        &mut self,
        bus: &mut impl Bus,
        status: BusStatus,
        address: u32,
    ) -> u16 {
        self.biu_bus_begin(
            bus,
            status,
            address,
            0,
            TransferSize::Word,
            OperandSize::Operand16,
            true,
        );
        self.biu_bus_wait_finish(bus);
        self.data_bus
    }

    /// V30 mode: write a word in a single aligned m-cycle (16-bit bus).
    fn biu_write_word_aligned(
        &mut self,
        bus: &mut impl Bus,
        status: BusStatus,
        address: u32,
        word: u16,
    ) {
        self.biu_bus_begin(
            bus,
            status,
            address,
            word,
            TransferSize::Word,
            OperandSize::Operand16,
            true,
        );
        self.biu_bus_wait_until_tx(bus);
    }

    /// Read a 16-bit word at `lo_address`/`hi_address`, dispatching on bus mode.
    /// V20 always uses two byte m-cycles; V30 uses one word m-cycle when the
    /// addresses form an aligned pair (lo even, hi = lo + 1) and falls back to
    /// two byte m-cycles otherwise.
    fn biu_read_word(
        &mut self,
        bus: &mut impl Bus,
        status: BusStatus,
        lo_address: u32,
        hi_address: u32,
    ) -> u16 {
        if MODEL == V30_BUS && lo_address & 1 == 0 && hi_address == lo_address.wrapping_add(1) {
            self.biu_read_word_aligned(bus, status, lo_address)
        } else {
            self.biu_read_word_pair(bus, status, lo_address, hi_address)
        }
    }

    fn biu_write_word(
        &mut self,
        bus: &mut impl Bus,
        status: BusStatus,
        lo_address: u32,
        hi_address: u32,
        word: u16,
    ) {
        if MODEL == V30_BUS && lo_address & 1 == 0 && hi_address == lo_address.wrapping_add(1) {
            self.biu_write_word_aligned(bus, status, lo_address, word);
        } else {
            self.biu_write_word_pair(bus, status, lo_address, hi_address, word);
        }
    }

    pub(super) fn biu_read_u16(&mut self, bus: &mut impl Bus, seg: SegReg16, offset: u16) -> u16 {
        let lo_address = self.seg_base(seg).wrapping_add(offset as u32) & ADDRESS_MASK;
        let hi_address = self
            .seg_base(seg)
            .wrapping_add(offset.wrapping_add(1) as u32)
            & ADDRESS_MASK;
        self.biu_read_word(bus, BusStatus::MemRead, lo_address, hi_address)
    }

    pub(super) fn biu_read_word_physical_pair(
        &mut self,
        bus: &mut impl Bus,
        lo_address: u32,
        hi_address: u32,
    ) -> u16 {
        self.biu_read_word(
            bus,
            BusStatus::MemRead,
            lo_address & ADDRESS_MASK,
            hi_address & ADDRESS_MASK,
        )
    }

    pub(super) fn biu_write_word_physical_pair(
        &mut self,
        bus: &mut impl Bus,
        lo_address: u32,
        hi_address: u32,
        word: u16,
    ) {
        self.biu_write_word(
            bus,
            BusStatus::MemWrite,
            lo_address & ADDRESS_MASK,
            hi_address & ADDRESS_MASK,
            word,
        );
    }

    pub(super) fn biu_read_u16_physical(&mut self, bus: &mut impl Bus, address: u32) -> u16 {
        let wrapped_address = address & ADDRESS_MASK;
        let next_wrapped_address = wrapped_address.wrapping_add(1) & ADDRESS_MASK;
        self.biu_read_word(
            bus,
            BusStatus::MemRead,
            wrapped_address,
            next_wrapped_address,
        )
    }

    pub(super) fn biu_write_u16(
        &mut self,
        bus: &mut impl Bus,
        seg: SegReg16,
        offset: u16,
        word: u16,
    ) {
        let lo_address = self.seg_base(seg).wrapping_add(offset as u32) & ADDRESS_MASK;
        let hi_address = self
            .seg_base(seg)
            .wrapping_add(offset.wrapping_add(1) as u32)
            & ADDRESS_MASK;
        self.biu_write_word(bus, BusStatus::MemWrite, lo_address, hi_address, word);
    }

    pub(super) fn biu_write_u16_physical(&mut self, bus: &mut impl Bus, address: u32, word: u16) {
        let wrapped_address = address & ADDRESS_MASK;
        let next_wrapped_address = wrapped_address.wrapping_add(1) & ADDRESS_MASK;
        self.biu_write_word(
            bus,
            BusStatus::MemWrite,
            wrapped_address,
            next_wrapped_address,
            word,
        );
    }

    pub(super) fn biu_io_read_u8(&mut self, bus: &mut impl Bus, port: u16) -> u8 {
        self.biu_bus_begin(
            bus,
            BusStatus::IoRead,
            port as u32,
            0,
            TransferSize::Byte,
            OperandSize::Operand8,
            true,
        );
        self.biu_bus_wait_finish(bus);
        self.biu_byte_on_lane()
    }

    pub(super) fn biu_io_write_u8(&mut self, bus: &mut impl Bus, port: u16, byte: u8) {
        self.biu_bus_begin(
            bus,
            BusStatus::IoWrite,
            port as u32,
            byte as u16,
            TransferSize::Byte,
            OperandSize::Operand8,
            true,
        );
        self.biu_bus_wait_until_tx(bus);
    }

    pub(super) fn biu_io_read_u16(&mut self, bus: &mut impl Bus, port: u16) -> u16 {
        let lo_address = port as u32;
        let hi_address = port.wrapping_add(1) as u32;
        self.biu_read_word(bus, BusStatus::IoRead, lo_address, hi_address)
    }

    pub(super) fn biu_io_write_u16(&mut self, bus: &mut impl Bus, port: u16, word: u16) {
        let lo_address = port as u32;
        let hi_address = port.wrapping_add(1) as u32;
        self.biu_write_word(bus, BusStatus::IoWrite, lo_address, hi_address, word);
    }

    /// Read one instruction byte from the queue, cycling if empty.
    pub(super) fn biu_queue_read(&mut self, bus: &mut impl Bus, dtype: QueueType) -> u8 {
        if MODEL == V30_BUS
            && matches!(self.fetch_state, FetchState::Delayed(_))
            && self.queue_at_policy_threshold()
        {
            self.fetch_state = FetchState::Delayed(0);
        }

        if let Some(preload) = self.instruction_preload.take() {
            self.last_queue_op = QueueOp::First;
            self.last_queue_byte = preload;
            self.biu_fetch_on_queue_read();
            return preload;
        }

        let byte = if self.queue_len() > 0 {
            let b = self.queue_pop();
            self.biu_fetch_on_queue_read();
            b
        } else {
            self.ensure_fetch_in_flight();
            let mut guard = 0u32;
            while self.queue_len() == 0 && guard < BIU_LOOP_GUARD {
                self.cycle(bus);
                guard += 1;
            }
            if self.queue_len() > 0 {
                self.queue_pop()
            } else {
                0
            }
        };

        self.queue_op = match dtype {
            QueueType::First => QueueOp::First,
            QueueType::Subsequent => QueueOp::Subsequent,
        };
        self.last_queue_byte = byte;
        self.cycle(bus);
        byte
    }

    pub(super) fn biu_next_queue_byte_for_timing(&self) -> Option<u8> {
        self.instruction_preload
            .or_else(|| (self.queue_len() > 0).then_some(self.instruction_queue[0]))
    }

    pub(super) fn biu_queue_read_no_cycle(&mut self) -> u8 {
        let byte = if let Some(preload) = self.instruction_preload.take() {
            preload
        } else if self.queue_len() > 0 {
            self.queue_pop()
        } else {
            debug_assert!(self.queue_len() > 0, "queue underrun");
            0
        };
        self.biu_fetch_on_queue_read();
        byte
    }

    fn ensure_fetch_in_flight(&mut self) {
        if self.bus_status_latch == BusStatus::Passive
            && self.pl_status != BusStatus::CodeFetch
            && !matches!(self.fetch_state, FetchState::Delayed(_))
        {
            self.biu_fetch_start();
        }
    }

    pub(super) fn biu_fetch_on_queue_read(&mut self) {
        if self.bus_status == BusStatus::Passive && self.queue_has_room_for_fetch() {
            match self.fetch_state {
                FetchState::Suspended => {
                    self.ta_cycle = TaCycle::Td;
                }
                FetchState::PausedFull => {
                    if self.t_cycle == TCycle::Ti {
                        self.ta_cycle = TaCycle::Td;
                    } else {
                        return;
                    }
                }
                _ => {}
            }
            self.biu_fetch_start();
        }
    }

    pub(super) fn biu_fetch_next(&mut self, bus: &mut impl Bus) {
        self.ensure_fetch_in_flight();

        let mut guard = 0u32;
        while self.queue_len() == 0 && self.instruction_preload.is_none() && guard < BIU_LOOP_GUARD
        {
            self.cycle(bus);
            guard += 1;
        }

        if self.instruction_preload.is_none() && self.queue_len() > 0 {
            let byte = self.queue_pop();
            self.instruction_preload = Some(byte);
            self.queue_op = QueueOp::First;
            self.last_queue_byte = byte;
            self.biu_fetch_on_queue_read();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FetchProbeBus {
        byte_fetches: Vec<u32>,
        word_fetches: Vec<u32>,
        current_cycle: u64,
    }

    impl Bus for FetchProbeBus {
        fn read_byte(&mut self, _address: u32) -> u8 {
            panic!("word prefetch must not use data reads")
        }

        fn write_byte(&mut self, _address: u32, _value: u8) {}

        fn io_read_byte(&mut self, _port: u16) -> u8 {
            0
        }

        fn io_write_byte(&mut self, _port: u16, _value: u8) {}

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
            self.current_cycle
        }

        fn set_current_cycle(&mut self, cycle: u64) {
            self.current_cycle = cycle;
        }

        fn fetch_opcode_byte(&mut self, address: u32) -> u8 {
            self.byte_fetches.push(address);
            address as u8
        }

        fn fetch_opcode_word(&mut self, address: u32) -> u16 {
            self.word_fetches.push(address);
            0xDEAD
        }
    }

    #[test]
    fn v30_word_code_fetch_uses_wide_bus_dispatch() {
        let mut cpu = VX0::<V30_BUS>::new();
        cpu.bus_status_latch = BusStatus::CodeFetch;
        cpu.transfer_size = TransferSize::Word;
        cpu.address_latch = 0x12345;
        let mut bus = FetchProbeBus::default();

        cpu.biu_do_bus_transfer(&mut bus);

        assert!(bus.byte_fetches.is_empty());
        assert_eq!(bus.word_fetches, [0x12344]);
        assert_eq!(cpu.data_bus, 0xDEAD);
    }
}
