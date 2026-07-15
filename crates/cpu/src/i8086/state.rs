use core::ops::{Deref, DerefMut};

use save_state::{StateValidationError, ValidateState};

use super::{
    I8086, StepFinishCycle,
    biu::{
        ADDRESS_MASK, BusPendingType, BusStatus, FetchState, OperandSize, QUEUE_SIZE, QueueOp,
        TCycle, TaCycle, TransferSize,
    },
    flags::I8086Flags,
};
use crate::{ByteReg, RegisterFile16, SegReg16, WordReg};

save_state::runtime_state! {
    /// Complete authoritative Intel 8086 state at a resumable boundary.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct I8086State {
        /// General-purpose register file.
        pub regs: RegisterFile16,
        /// Segment registers: ES, CS, SS, DS.
        pub sregs: [u16; 4],
        /// Instruction pointer.
        pub ip: u16,
        /// CPU flags.
        pub flags: I8086Flags,
        /// Internal execution and bus-interface state.
        #[doc(hidden)]
        pub internal: I8086InternalState,
    }
}

save_state::runtime_state! {
    /// Internal Intel 8086 execution and bus-interface state.
    #[doc(hidden)]
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct I8086InternalState {
        pub(super) prev_ip: u16,
        pub(super) opcode_start_ip: u16,
        pub(super) seg_prefix: bool,
        pub(super) prefix_seg: SegReg16,
        pub(super) halted: bool,
        pub(super) pending_irq: u8,
        pub(super) no_interrupt: u8,
        pub(super) inhibit_all: u8,
        pub(super) rep_ip: u16,
        pub(super) rep_restart_ip: u16,
        pub(super) rep_seg_prefix: bool,
        pub(super) rep_prefix_seg: SegReg16,
        pub(super) rep_prefix: bool,
        pub(super) rep_opcode: u8,
        pub(super) rep_type: u8,
        pub(super) rep_active: bool,
        pub(super) ea: u32,
        pub(super) eo: u16,
        pub(super) effective_address_segment: SegReg16,
        pub(super) modrm_displacement: u16,
        pub(super) modrm_has_displacement: bool,
        pub(super) instruction_queue: [u8; QUEUE_SIZE],
        pub(super) instruction_queue_len: usize,
        pub(super) instruction_preload: Option<u8>,
        pub(super) instruction_entry_queue_bytes: u8,
        pub(super) prefetch_ip: u16,
        pub(super) step_finish_cycle: StepFinishCycle,
        pub(super) nx: bool,
        pub(super) rni: bool,
        pub(super) queue_op: QueueOp,
        pub(super) last_queue_op: QueueOp,
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
        pub(super) bhe: bool,
        pub(super) address_bus: u32,
        pub(super) address_latch: u32,
        pub(super) data_bus: u16,
    }
}

impl Default for I8086InternalState {
    fn default() -> Self {
        Self {
            prev_ip: 0,
            opcode_start_ip: 0,
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
            rep_prefix: false,
            rep_opcode: 0,
            rep_type: 0,
            rep_active: false,
            ea: 0,
            eo: 0,
            effective_address_segment: SegReg16::DS,
            modrm_displacement: 0,
            modrm_has_displacement: false,
            instruction_queue: [0; QUEUE_SIZE],
            instruction_queue_len: 0,
            instruction_preload: None,
            instruction_entry_queue_bytes: 0,
            prefetch_ip: 0,
            step_finish_cycle: StepFinishCycle::WithFetchCycle,
            nx: false,
            rni: false,
            queue_op: QueueOp::Idle,
            last_queue_op: QueueOp::Idle,
            t_cycle: TCycle::Ti,
            ta_cycle: TaCycle::Td,
            bus_status: BusStatus::Passive,
            bus_status_latch: BusStatus::Passive,
            pl_status: BusStatus::Passive,
            bus_pending: BusPendingType::None,
            fetch_state: FetchState::Normal,
            transfer_size: TransferSize::Byte,
            operand_size: OperandSize::Operand8,
            transfer_n: 1,
            final_transfer: false,
            bhe: false,
            address_bus: 0,
            address_latch: 0,
            data_bus: 0,
        }
    }
}

impl Deref for I8086State {
    type Target = I8086InternalState;

    fn deref(&self) -> &Self::Target {
        &self.internal
    }
}

impl DerefMut for I8086State {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.internal
    }
}

impl I8086State {
    /// Initializes a cold frontend at the current instruction pointer.
    pub fn initialize_cold_frontend(&mut self) {
        self.internal = I8086InternalState {
            prefetch_ip: self.ip,
            ..I8086InternalState::default()
        };
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

impl ValidateState<()> for I8086State {
    fn validate_state(&self, _context: &()) -> Result<(), StateValidationError> {
        if self.instruction_queue_len > QUEUE_SIZE
            || usize::from(self.instruction_entry_queue_bytes) > QUEUE_SIZE
        {
            return Err(StateValidationError::new(
                "8086 prefetch queue length is invalid",
            ));
        }
        if self.pending_irq & !0x03 != 0 || self.no_interrupt > 1 || self.inhibit_all > 1 {
            return Err(StateValidationError::new("8086 interrupt latch is invalid"));
        }
        if self.rep_active && self.rep_type > 1 {
            return Err(StateValidationError::new(
                "8086 REP continuation is invalid",
            ));
        }
        if self.transfer_n > 2
            || self.address_bus > ADDRESS_MASK
            || self.address_latch > ADDRESS_MASK
        {
            return Err(StateValidationError::new(
                "8086 bus interface state is invalid",
            ));
        }
        Ok(())
    }
}

impl I8086 {
    /// Loads complete CPU state without resetting execution or BIU latches.
    pub fn load_state(&mut self, state: &I8086State) {
        self.state = state.clone();
    }

    /// Clones the authoritative state at a resumable execution boundary.
    pub fn capture_state(&self) -> I8086State {
        self.state.clone()
    }

    /// Validates and replaces the authoritative state transactionally.
    pub fn restore_state(&mut self, state: I8086State) -> Result<(), StateValidationError> {
        save_state::restore_root(self, state, &())
    }

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
