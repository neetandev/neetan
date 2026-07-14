//! T-state cycle-accurate Bus Interface Unit for the 8086.
//!
//! This module implements the 8086 BIU's T-state and Ta-state machine.
//!
//! The implementation was heavily influenced by the 8088 core in MartyPC,
//! which is licensed under the permissive MIT license.
//!
//! The model tracks every bus cycle at T-state granularity so that EU/BIU
//! interactions (prefetch aborts, bus wait-for-T4, queue policy throttling)
//! produce cycle-exact counts matching real hardware.

use common::Bus;

use super::I8086;
use crate::SegReg16;

/// 8086 instruction queue capacity.
pub const QUEUE_SIZE: usize = 6;
/// 8086 prefetch fetch width (16-bit bus fetches one word per m-cycle).
pub const FETCH_SIZE: usize = 2;
/// Queue length at which the BIU applies the first policy throttle.
pub const POLICY_LEN0: usize = QUEUE_SIZE - 2;
/// Queue length at which the BIU applies the second policy throttle.
pub const POLICY_LEN1: usize = QUEUE_SIZE - 3;
/// 20-bit physical address wrap mask (8086 has 1 MiB of address space).
pub const ADDRESS_MASK: u32 = 0xFFFFF;
/// Number of T-states the BIU waits after hitting a policy throttle.
const POLICY_THROTTLE_DELAY: u8 = 3;
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
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum TransferSize {
    #[default]
    Byte,
    Word,
}

/// Operand size spanning one or two m-cycles (8086 word-aligned = 1 m-cycle).
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

#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum QueueOp {
    #[default]
    Idle,
    First,
    Flush,
    Subsequent,
}

impl I8086 {
    #[inline(always)]
    pub(super) fn queue_len(&self) -> usize {
        self.instruction_queue_len
    }

    #[inline(always)]
    pub(super) fn queue_has_room_for_fetch(&self) -> bool {
        self.instruction_queue_len + FETCH_SIZE <= QUEUE_SIZE
    }

    #[inline(always)]
    pub(super) fn queue_at_policy_len(&self) -> bool {
        self.instruction_queue_len == POLICY_LEN0 || self.instruction_queue_len == POLICY_LEN1
    }

    #[inline(always)]
    pub(super) fn queue_at_policy_threshold(&self) -> bool {
        self.instruction_queue_len == POLICY_LEN1
    }

    #[inline(always)]
    pub(super) fn queue_pop(&mut self) -> u8 {
        debug_assert!(self.instruction_queue_len > 0, "queue underrun");
        let value = self.instruction_queue[0];
        if self.instruction_queue_len > 1 {
            self.instruction_queue
                .copy_within(1..self.instruction_queue_len, 0);
        }
        self.instruction_queue_len -= 1;
        value
    }

    #[inline(always)]
    pub(super) fn queue_push8(&mut self, byte: u8) -> u16 {
        debug_assert!(self.instruction_queue_len < QUEUE_SIZE, "queue overrun");
        self.instruction_queue[self.instruction_queue_len] = byte;
        self.instruction_queue_len += 1;
        1
    }

    /// Push a 16-bit word into the queue at an even address. If `a0` is true,
    /// only the high byte of the word is pushed (odd address fetch).
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

    pub(super) fn queue_flush(&mut self) {
        self.instruction_queue_len = 0;
        self.instruction_preload = None;
        self.fetch_state = FetchState::Normal;
        self.queue_op = QueueOp::Flush;
    }

    /// Advance the CPU by one T-state. This is the unit of time used by the
    /// T-state BIU. Every EU or BIU operation ultimately reduces to a sequence
    /// of `cycle` calls.
    pub(super) fn cycle(&mut self, bus: &mut impl Bus) {
        // Tinit is a synthetic state used to indicate "bus cycle just started".
        if self.t_cycle == TCycle::Tinit {
            self.t_cycle = TCycle::T1;
        }

        self.cycles_remaining -= 1;

        // Phase 1: Operate on the current (latched) T-state.
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
                    // With zero wait states, the transfer always completes on T3.
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

        // Phase 2: Advance the fetch delay counter (except during wait states).
        if let FetchState::Delayed(count) = self.fetch_state
            && self.t_cycle != TCycle::Tw
            && count > 0
        {
            self.fetch_state = FetchState::Delayed(count - 1);
        }

        // Phase 3: Advance the Ta-cycle (address cycle pipeline).
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
                        self.t_cycle = TCycle::Tinit;
                        TaCycle::Td
                    } else {
                        TaCycle::T0
                    }
                }
            },
            TaCycle::Td => TaCycle::Td,
            TaCycle::Ta => TaCycle::Ta,
        };

        // Phase 4: Advance the main T-cycle.
        self.t_cycle = match self.t_cycle {
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
                // No wait states are simulated; always proceed to T4.
                self.biu_bus_end();
                TCycle::T4
            }
            TCycle::T4 => {
                self.bus_status_latch = BusStatus::Passive;
                TCycle::Ti
            }
        };

        self.last_queue_op = self.queue_op;
        self.queue_op = QueueOp::Idle;
    }

    #[inline]
    pub(super) fn cycles(&mut self, bus: &mut impl Bus, count: u32) {
        for _ in 0..count {
            self.cycle(bus);
        }
    }

    fn biu_bus_end(&mut self) {
        // Reset command signals (placeholder; we don't model the 8288).
    }

    /// Extract the byte currently on the active data-bus lane. When BHE is
    /// asserted the byte lives in the high half of `data_bus`, otherwise in
    /// the low half.
    #[inline(always)]
    fn biu_byte_on_lane(&self) -> u8 {
        if self.bhe {
            (self.data_bus >> 8) as u8
        } else {
            self.data_bus as u8
        }
    }

    /// Zero-extend the byte on the active data-bus lane into the low byte of
    /// a u16. Used when combining two byte transfers into a word result.
    #[inline(always)]
    fn biu_byte_on_lane_as_low_half(&self) -> u16 {
        if self.bhe {
            self.data_bus >> 8
        } else {
            self.data_bus & 0x00FF
        }
    }

    /// Shift the byte on the active data-bus lane into the high byte of a
    /// u16. Used when combining two byte transfers into a word result.
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
    /// byte transfer.
    #[inline(always)]
    fn biu_align_byte_to_lane(&mut self) {
        if self.bhe {
            self.data_bus <<= 8;
        }
    }

    fn biu_do_bus_transfer(&mut self, bus: &mut impl Bus) {
        match (self.bus_status_latch, self.transfer_size) {
            (BusStatus::CodeFetch, TransferSize::Byte) => {
                self.data_bus = bus.fetch_opcode_byte(self.address_latch & ADDRESS_MASK) as u16;
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
            _ => {}
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

    fn biu_bus_begin_fetch(&mut self) {
        if self.queue_has_room_for_fetch() {
            self.operand_size = match FETCH_SIZE {
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
            self.transfer_size = if self.prefetch_ip & 1 == 0 {
                TransferSize::Word
            } else {
                TransferSize::Byte
            };
            self.transfer_n = 1;
            self.final_transfer = true;
        }
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

    pub(super) fn biu_queue_flush(&mut self, bus: &mut impl Bus) {
        self.biu_commit_pending_write_inline(bus);
        self.queue_flush();
        self.bus_status = BusStatus::Passive;
        self.bus_status_latch = BusStatus::Passive;
        self.t_cycle = TCycle::Ti;
        self.ta_cycle = TaCycle::Td;
        self.pl_status = BusStatus::Passive;
        self.bus_pending = BusPendingType::None;
        self.fetch_state = FetchState::Normal;
        self.biu_fetch_start();
    }

    pub(super) fn biu_queue_flush_early(&mut self, bus: &mut impl Bus) {
        self.biu_commit_pending_write_inline(bus);
        self.queue_flush();
        self.bus_status = BusStatus::Passive;
        self.bus_status_latch = BusStatus::Passive;
        self.t_cycle = TCycle::Ti;
        self.ta_cycle = TaCycle::Td;
        self.pl_status = BusStatus::Passive;
        self.bus_pending = BusPendingType::None;
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
        if self.queue_at_policy_len() && self.bus_status_latch == BusStatus::CodeFetch {
            if self.ta_cycle == TaCycle::Td && !matches!(self.fetch_state, FetchState::Delayed(_)) {
                self.fetch_state = FetchState::Delayed(POLICY_THROTTLE_DELAY);
            }
        } else if self.ta_cycle == TaCycle::Td {
            self.biu_fetch_start();
        }
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

    pub(super) fn biu_commit_pending_write_inline(&mut self, bus: &mut impl Bus) {
        if matches!(
            self.bus_status_latch,
            BusStatus::MemWrite | BusStatus::IoWrite
        ) && matches!(self.t_cycle, TCycle::T3 | TCycle::Tw)
        {
            self.biu_do_bus_transfer(bus);
            self.biu_bus_end();
            self.bus_status = BusStatus::Passive;
            self.bus_status_latch = BusStatus::Passive;
            self.t_cycle = TCycle::Ti;
            self.transfer_n = 0;
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

        match self.t_cycle {
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

        let was_delay = self.biu_bus_wait_delay(bus);

        self.biu_bus_wait_address(bus);

        if was_delay || fetch_abort {
            self.biu_address_start(new_bus_status);
            self.biu_bus_wait_address(bus);
        }

        if self.t_cycle == TCycle::T4 && self.bus_status_latch != BusStatus::CodeFetch {
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

        self.bhe = matches!(size, TransferSize::Word) || (address & 1 != 0);
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

    /// Read a word in a single aligned m-cycle.
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

    /// Read a word that spans two byte m-cycles because the operand is at an
    /// odd address. The caller provides the two byte addresses.
    fn biu_read_word_unaligned(
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

    /// Write a word in a single aligned m-cycle.
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

    /// Write a word that spans two byte m-cycles because the operand is at an
    /// odd address. The caller provides both byte addresses.
    fn biu_write_word_unaligned(
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

    /// Read a 16-bit word from `seg:offset`. Odd-aligned accesses split into
    /// two byte transfers; aligned accesses complete in one m-cycle.
    pub(super) fn biu_read_u16(&mut self, bus: &mut impl Bus, seg: SegReg16, offset: u16) -> u16 {
        let lo_address = self.seg_base(seg).wrapping_add(offset as u32) & ADDRESS_MASK;
        if offset & 1 == 0 {
            self.biu_read_word_aligned(bus, BusStatus::MemRead, lo_address)
        } else {
            let hi_address = self
                .seg_base(seg)
                .wrapping_add(offset.wrapping_add(1) as u32)
                & ADDRESS_MASK;
            self.biu_read_word_unaligned(bus, BusStatus::MemRead, lo_address, hi_address)
        }
    }

    pub(super) fn biu_read_u16_physical(&mut self, bus: &mut impl Bus, address: u32) -> u16 {
        let wrapped_address = address & ADDRESS_MASK;
        if wrapped_address & 1 == 0 {
            self.biu_read_word_aligned(bus, BusStatus::MemRead, wrapped_address)
        } else {
            let next_wrapped_address = wrapped_address.wrapping_add(1) & ADDRESS_MASK;
            self.biu_read_word_unaligned(
                bus,
                BusStatus::MemRead,
                wrapped_address,
                next_wrapped_address,
            )
        }
    }

    /// Write a 16-bit word to `seg:offset`.
    pub(super) fn biu_write_u16(
        &mut self,
        bus: &mut impl Bus,
        seg: SegReg16,
        offset: u16,
        word: u16,
    ) {
        let lo_address = self.seg_base(seg).wrapping_add(offset as u32) & ADDRESS_MASK;
        if offset & 1 == 0 {
            self.biu_write_word_aligned(bus, BusStatus::MemWrite, lo_address, word);
        } else {
            let hi_address = self
                .seg_base(seg)
                .wrapping_add(offset.wrapping_add(1) as u32)
                & ADDRESS_MASK;
            self.biu_write_word_unaligned(bus, BusStatus::MemWrite, lo_address, hi_address, word);
        }
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
        if port & 1 == 0 {
            self.biu_read_word_aligned(bus, BusStatus::IoRead, lo_address)
        } else {
            let hi_address = port.wrapping_add(1) as u32;
            self.biu_read_word_unaligned(bus, BusStatus::IoRead, lo_address, hi_address)
        }
    }

    pub(super) fn biu_io_write_u16(&mut self, bus: &mut impl Bus, port: u16, word: u16) {
        let lo_address = port as u32;
        if port & 1 == 0 {
            self.biu_write_word_aligned(bus, BusStatus::IoWrite, lo_address, word);
        } else {
            let hi_address = port.wrapping_add(1) as u32;
            self.biu_write_word_unaligned(bus, BusStatus::IoWrite, lo_address, hi_address, word);
        }
    }

    /// Read one instruction byte from the queue, cycling if empty.
    pub(super) fn biu_queue_read(&mut self, bus: &mut impl Bus, dtype: QueueType) -> u8 {
        // Cancel fetch delay if queue is at threshold.
        if matches!(self.fetch_state, FetchState::Delayed(_)) && self.queue_at_policy_threshold() {
            self.fetch_state = FetchState::Delayed(0);
        }

        if let Some(preload) = self.instruction_preload.take() {
            self.last_queue_op = QueueOp::First;
            self.nx = false;
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
        self.cycle(bus);
        byte
    }

    /// Schedule a code fetch if no fetch/address cycle is currently in
    /// flight. Used as a safety valve in wait-for-queue loops.
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
            if self.nx {
                self.nx = false;
                self.rni = false;
            }
            self.cycle(bus);
            guard += 1;
        }

        if self.instruction_preload.is_none() && self.queue_len() > 0 {
            let byte = self.queue_pop();
            self.instruction_preload = Some(byte);
            self.queue_op = QueueOp::First;
            self.biu_fetch_on_queue_read();
        }

        if self.nx {
            self.nx = false;
        }
        if self.rni {
            self.rni = false;
        }

        self.cycle(bus);
    }

    /// Preload the next instruction's first byte without spending the trailing
    /// finish cycle.
    pub(super) fn biu_preload_next(&mut self, bus: &mut impl Bus) {
        self.ensure_fetch_in_flight();
        let mut guard = 0u32;
        while self.queue_len() == 0 && self.instruction_preload.is_none() && guard < BIU_LOOP_GUARD
        {
            if self.nx {
                self.nx = false;
                self.rni = false;
            }
            self.cycle(bus);
            guard += 1;
        }
        if self.instruction_preload.is_none() && self.queue_len() > 0 {
            let byte = self.queue_pop();
            self.instruction_preload = Some(byte);
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
    fn word_code_fetch_uses_wide_bus_dispatch() {
        let mut cpu = I8086::new();
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
