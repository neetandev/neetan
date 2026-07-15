use save_state::{StateValidationError, ValidateState};

use super::flags::M6809Flags;

save_state::runtime_state! {
    /// Complete authoritative MC6809 state at a resumable boundary.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct M6809State {
        /// Accumulator A.
        pub a: u8,
        /// Accumulator B.
        pub b: u8,
        /// X index register.
        pub x: u16,
        /// Y index register.
        pub y: u16,
        /// Hardware stack pointer.
        pub s: u16,
        /// User stack pointer.
        pub u: u16,
        /// Program counter.
        pub pc: u16,
        /// Direct page register.
        pub dp: u8,
        /// Condition code flags.
        pub flags: M6809Flags,
        /// Whether SYNC or CWAI has halted instruction fetch.
        pub halted: bool,
        /// Latched IRQ, NMI, and FIRQ inputs.
        pub pending_irq: u8,
        /// Whether loading the hardware stack has armed NMI.
        pub nmi_armed: bool,
        /// Whether CWAI already stacked the complete register state.
        pub cwai_waiting: bool,
        /// Deferred write address for the split extended CLR operation.
        pub pending_extended_clear: Option<u16>,
    }
}

impl M6809State {
    /// Returns the combined D accumulator.
    pub fn d(&self) -> u16 {
        (u16::from(self.a) << 8) | u16::from(self.b)
    }

    /// Sets the combined D accumulator.
    pub fn set_d(&mut self, value: u16) {
        self.a = (value >> 8) as u8;
        self.b = value as u8;
    }
}

impl ValidateState for M6809State {
    fn validate_state(&self, _context: &()) -> Result<(), StateValidationError> {
        if self.pending_irq & !0x07 != 0 {
            return Err(StateValidationError::new(
                "MC6809 pending interrupt mask is invalid",
            ));
        }
        if self.cwai_waiting && !self.halted {
            return Err(StateValidationError::new(
                "MC6809 CWAI state must remain halted",
            ));
        }
        Ok(())
    }
}
