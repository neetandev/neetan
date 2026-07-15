use save_state::{StateValidationError, ValidateState};

use super::{
    FinishState, VX0,
    biu::{
        BusPendingType, BusStatus, FetchState, MAX_QUEUE_SIZE, OperandSize, QueueOp, TCycle,
        TaCycle, TransferSize, queue_size_for,
    },
    flags::V30Flags,
    rep::RepState,
};
use crate::{ByteReg, RegisterFile16, SegReg16, WordReg};

save_state::runtime_state! {
    /// Complete authoritative V20 and V30 state at a resumable boundary.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct V30State {
        /// General-purpose register file.
        pub regs: RegisterFile16,
        /// Segment registers: ES, CS, SS, DS.
        pub sregs: [u16; 4],
        /// Instruction pointer.
        pub ip: u16,
        /// CPU flags.
        pub flags: V30Flags,
        pub(super) prev_ip: u16,
        pub(super) opcode_start_ip: u16,
        pub(super) seg_prefix: bool,
        pub(super) prefix_seg: SegReg16,
        pub(super) halted: bool,
        pub(super) pending_irq: u8,
        pub(super) no_interrupt: u8,
        pub(super) inhibit_all: u8,
        pub(super) rep_state: RepState,
        pub(super) finish_state: FinishState,
        pub(super) ea: u32,
        pub(super) eo: u16,
        pub(super) effective_address_segment: SegReg16,
        pub(super) instruction_queue: [u8; MAX_QUEUE_SIZE],
        pub(super) instruction_queue_len: usize,
        pub(super) instruction_preload: Option<u8>,
        pub(super) instruction_entry_queue_bytes: u8,
        pub(super) prefetch_ip: u16,
        pub(super) queue_op: QueueOp,
        pub(super) last_queue_op: QueueOp,
        pub(super) last_queue_byte: u8,
        pub(super) t_cycle: TCycle,
        pub(super) ta_cycle: TaCycle,
        pub(super) bus_status: BusStatus,
        pub(super) bus_status_latch: BusStatus,
        pub(super) pl_status: BusStatus,
        pub(super) bus_pending: BusPendingType,
        pub(super) fetch_state: FetchState,
        pub(super) transfer_size: TransferSize,
        pub(super) operand_size: OperandSize,
        pub(super) transfer_n: u32,
        pub(super) final_transfer: bool,
        pub(super) address_bus: u32,
        pub(super) address_latch: u32,
        pub(super) data_bus: u16,
        pub(super) bhe: bool,
    }
}

impl V30State {
    /// Initializes a cold BIU frontend at the current instruction pointer.
    pub fn initialize_cold_frontend(&mut self) {
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

    /// Returns the AX register.
    pub fn ax(&self) -> u16 {
        self.regs.word(WordReg::AX)
    }

    /// Sets the AX register.
    pub fn set_ax(&mut self, v: u16) {
        self.regs.set_word(WordReg::AX, v);
    }

    /// Returns the CX register.
    pub fn cx(&self) -> u16 {
        self.regs.word(WordReg::CX)
    }

    /// Sets the CX register.
    pub fn set_cx(&mut self, v: u16) {
        self.regs.set_word(WordReg::CX, v);
    }

    /// Returns the DX register.
    pub fn dx(&self) -> u16 {
        self.regs.word(WordReg::DX)
    }

    /// Sets the DX register.
    pub fn set_dx(&mut self, v: u16) {
        self.regs.set_word(WordReg::DX, v);
    }

    /// Returns the BX register.
    pub fn bx(&self) -> u16 {
        self.regs.word(WordReg::BX)
    }

    /// Sets the BX register.
    pub fn set_bx(&mut self, v: u16) {
        self.regs.set_word(WordReg::BX, v);
    }

    /// Returns the SP register.
    pub fn sp(&self) -> u16 {
        self.regs.word(WordReg::SP)
    }

    /// Sets the SP register.
    pub fn set_sp(&mut self, v: u16) {
        self.regs.set_word(WordReg::SP, v);
    }

    /// Returns the BP register.
    pub fn bp(&self) -> u16 {
        self.regs.word(WordReg::BP)
    }

    /// Sets the BP register.
    pub fn set_bp(&mut self, v: u16) {
        self.regs.set_word(WordReg::BP, v);
    }

    /// Returns the SI register.
    pub fn si(&self) -> u16 {
        self.regs.word(WordReg::SI)
    }

    /// Sets the SI register.
    pub fn set_si(&mut self, v: u16) {
        self.regs.set_word(WordReg::SI, v);
    }

    /// Returns the DI register.
    pub fn di(&self) -> u16 {
        self.regs.word(WordReg::DI)
    }

    /// Sets the DI register.
    pub fn set_di(&mut self, v: u16) {
        self.regs.set_word(WordReg::DI, v);
    }

    /// Returns the ES segment register.
    pub fn es(&self) -> u16 {
        self.sregs[SegReg16::ES as usize]
    }

    /// Sets the ES segment register.
    pub fn set_es(&mut self, v: u16) {
        self.sregs[SegReg16::ES as usize] = v;
    }

    /// Returns the CS segment register.
    pub fn cs(&self) -> u16 {
        self.sregs[SegReg16::CS as usize]
    }

    /// Sets the CS segment register.
    pub fn set_cs(&mut self, v: u16) {
        self.sregs[SegReg16::CS as usize] = v;
    }

    /// Returns the SS segment register.
    pub fn ss(&self) -> u16 {
        self.sregs[SegReg16::SS as usize]
    }

    /// Sets the SS segment register.
    pub fn set_ss(&mut self, v: u16) {
        self.sregs[SegReg16::SS as usize] = v;
    }

    /// Returns the DS segment register.
    pub fn ds(&self) -> u16 {
        self.sregs[SegReg16::DS as usize]
    }

    /// Sets the DS segment register.
    pub fn set_ds(&mut self, v: u16) {
        self.sregs[SegReg16::DS as usize] = v;
    }

    /// Returns the compressed flags register value.
    pub fn compressed_flags(&self) -> u16 {
        self.flags.compress()
    }

    /// Sets all flags from a compressed flags value.
    pub fn set_compressed_flags(&mut self, v: u16) {
        self.flags.expand(v);
    }
}

impl ValidateState<u8> for V30State {
    fn validate_state(&self, model: &u8) -> Result<(), StateValidationError> {
        if !matches!(*model, super::V20_BUS | super::V30_BUS) {
            return Err(StateValidationError::new("V20/V30 bus model is invalid"));
        }
        if self.instruction_queue_len > queue_size_for(*model)
            || usize::from(self.instruction_entry_queue_bytes) > queue_size_for(*model)
        {
            return Err(StateValidationError::new(
                "V20/V30 prefetch queue length is invalid",
            ));
        }
        if self.pending_irq & !0x03 != 0 || self.no_interrupt > 1 || self.inhibit_all > 1 {
            return Err(StateValidationError::new(
                "V20/V30 interrupt latch is invalid",
            ));
        }
        if self.rep_state.active && self.rep_state.type_ > 3 {
            return Err(StateValidationError::new(
                "V20/V30 REP continuation is invalid",
            ));
        }
        if self.transfer_n > 2 || self.address_bus > 0x000F_FFFF || self.address_latch > 0x000F_FFFF
        {
            return Err(StateValidationError::new(
                "V20/V30 bus interface state is invalid",
            ));
        }
        Ok(())
    }
}

impl<const MODEL: u8> VX0<MODEL> {
    /// Returns the AL register value.
    pub fn al(&self) -> u8 {
        self.regs.byte(ByteReg::AL)
    }

    /// Returns the AH register value.
    pub fn ah(&self) -> u8 {
        self.regs.byte(ByteReg::AH)
    }

    /// Returns the CL register value.
    pub fn cl(&self) -> u8 {
        self.regs.byte(ByteReg::CL)
    }

    /// Returns the instruction pointer.
    pub fn ip(&self) -> u16 {
        self.ip
    }

    /// Returns the compressed flags register value.
    pub fn flags_register(&self) -> u16 {
        self.flags.compress()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_allows_consumed_instruction_entry_queue_bytes() {
        let state = V30State {
            instruction_queue_len: 1,
            instruction_entry_queue_bytes: queue_size_for(super::super::V30_BUS) as u8,
            ..Default::default()
        };

        assert!(state.validate_state(&super::super::V30_BUS).is_ok());
    }

    #[test]
    fn validation_rejects_entry_queue_bytes_above_capacity() {
        let state = V30State {
            instruction_entry_queue_bytes: queue_size_for(super::super::V30_BUS) as u8 + 1,
            ..Default::default()
        };

        assert!(state.validate_state(&super::super::V30_BUS).is_err());
    }
}
