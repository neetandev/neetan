use save_state::{StateValidationError, ValidateState};

use super::{
    I286,
    flags::I286Flags,
    modrm::EaClass,
    timing::{I286FinishState, I286Timing},
};
use crate::{ByteReg, RegisterFile16, SegReg16, WordReg};

save_state::runtime_state! {
    /// Complete authoritative 80286 state at a resumable boundary.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct I286State {
        /// General-purpose register file.
        pub regs: RegisterFile16,
        /// Segment registers: ES, CS, SS, DS.
        pub sregs: [u16; 4],
        /// Instruction pointer.
        pub ip: u16,
        /// CPU flags.
        pub flags: I286Flags,
        /// Machine Status Word.
        pub msw: u16,
        /// Global Descriptor Table Register base (24-bit).
        pub gdt_base: u32,
        /// Global Descriptor Table Register limit.
        pub gdt_limit: u16,
        /// Interrupt Descriptor Table Register base (24-bit).
        pub idt_base: u32,
        /// Interrupt Descriptor Table Register limit.
        pub idt_limit: u16,
        /// Cached 24-bit physical base per segment (ES/CS/SS/DS).
        pub seg_bases: [u32; 4],
        /// Cached limit per segment (ES/CS/SS/DS).
        pub seg_limits: [u16; 4],
        /// Cached access-rights byte per segment (ES/CS/SS/DS).
        pub seg_rights: [u8; 4],
        /// Whether the segment register currently holds a valid loaded descriptor.
        pub seg_valid: [bool; 4],
        /// LDT selector.
        pub ldtr: u16,
        /// LDT cached base.
        pub ldtr_base: u32,
        /// LDT cached limit.
        pub ldtr_limit: u16,
        /// Task Register selector.
        pub tr: u16,
        /// TR cached base.
        pub tr_base: u32,
        /// TR cached limit.
        pub tr_limit: u16,
        /// TR cached access rights.
        pub tr_rights: u8,
        pub(super) prev_ip: u16,
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
        pub(super) rep_opcode: u8,
        pub(super) rep_type: u8,
        pub(super) rep_active: bool,
        pub(super) ea: u32,
        pub(super) eo: u16,
        pub(super) ea_seg: SegReg16,
        pub(crate) ea_class: EaClass,
        pub(crate) finish_state: I286FinishState,
        pub(super) trap_level: u8,
        pub(super) shutdown: bool,
        pub(super) timing: I286Timing,
    }
}

impl I286State {
    /// Initializes real-mode descriptor caches and a cold frontend.
    pub fn initialize_real_mode_caches(&mut self) {
        for &segment in &[SegReg16::ES, SegReg16::CS, SegReg16::SS, SegReg16::DS] {
            let selector = self.sregs[segment as usize];
            self.seg_bases[segment as usize] = u32::from(selector) << 4;
            self.seg_limits[segment as usize] = 0xFFFF;
            self.seg_rights[segment as usize] = if segment == SegReg16::CS { 0x9B } else { 0x93 };
            self.seg_valid[segment as usize] = true;
        }
        self.timing
            .reset(self.sregs[SegReg16::CS as usize], self.ip);
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

impl ValidateState for I286State {
    fn validate_state(&self, _context: &()) -> Result<(), StateValidationError> {
        if self.flags.iopl > 3 {
            return Err(StateValidationError::new("80286 IOPL is invalid"));
        }
        if self.pending_irq & !0x03 != 0 || self.no_interrupt > 1 || self.inhibit_all > 1 {
            return Err(StateValidationError::new(
                "80286 interrupt latch is invalid",
            ));
        }
        if self.rep_active && self.rep_type > 1 {
            return Err(StateValidationError::new(
                "80286 REP continuation is invalid",
            ));
        }
        if self.gdt_base > 0x00FF_FFFF
            || self.idt_base > 0x00FF_FFFF
            || self.ldtr_base > 0x00FF_FFFF
            || self.tr_base > 0x00FF_FFFF
            || self.seg_bases.iter().any(|base| *base > 0x00FF_FFFF)
        {
            return Err(StateValidationError::new(
                "80286 physical base is outside the address space",
            ));
        }
        self.timing.validate_state(&())
    }
}

impl I286 {
    /// Loads complete CPU state without resetting execution or timing latches.
    pub fn load_state(&mut self, state: &I286State) {
        self.state = state.clone();
    }

    /// Clones the authoritative state at a resumable execution boundary.
    pub fn capture_state(&self) -> I286State {
        self.state.clone()
    }

    /// Validates and replaces the authoritative state transactionally.
    pub fn restore_state(
        &mut self,
        state: I286State,
    ) -> Result<(), save_state::StateValidationError> {
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
