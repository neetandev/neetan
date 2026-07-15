use save_state::{StateValidationError, ValidateState};

use super::flags::Z80Flags;

save_state::runtime_state! {
    /// Complete authoritative Z80 state at an instruction boundary.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct Z80State {
        /// Accumulator.
        pub a: u8,
        /// Flags.
        pub flags: Z80Flags,
        /// General register B.
        pub b: u8,
        /// General register C.
        pub c: u8,
        /// General register D.
        pub d: u8,
        /// General register E.
        pub e: u8,
        /// General register H.
        pub h: u8,
        /// General register L.
        pub l: u8,
        /// Interrupt vector register.
        pub i: u8,
        /// Refresh register low 7 bits.
        pub r: u8,
        /// Refresh register bit 7.
        pub r_high: u8,
        /// Shadow AF register pair.
        pub af_alt: u16,
        /// Shadow BC register pair.
        pub bc_alt: u16,
        /// Shadow DE register pair.
        pub de_alt: u16,
        /// Shadow HL register pair.
        pub hl_alt: u16,
        /// Index register IX.
        pub ix: u16,
        /// Index register IY.
        pub iy: u16,
        /// Stack pointer.
        pub sp: u16,
        /// Program counter.
        pub pc: u16,
        /// Internal WZ register.
        pub wz: u16,
        /// Interrupt flip-flop 1.
        pub iff1: bool,
        /// Interrupt flip-flop 2.
        pub iff2: bool,
        /// Interrupt mode.
        pub im: u8,
        /// EI deferral latch exposed by the verification corpus.
        pub ei: u8,
        /// LD A,I / LD A,R latch exposed by the verification corpus.
        pub p: u8,
        /// Q latch exposed by the verification corpus.
        pub q: u8,
        /// Whether the core is stopped by HALT.
        pub halted: bool,
        /// Latched maskable and non-maskable interrupt inputs.
        pub pending_irq: u8,
        /// Internal Q update latch for the current instruction boundary.
        pub q_latch: bool,
    }
}

impl Z80State {
    /// Returns AF.
    pub fn af(&self) -> u16 {
        (u16::from(self.a) << 8) | u16::from(self.flags.compress())
    }

    /// Sets AF.
    pub fn set_af(&mut self, value: u16) {
        self.a = (value >> 8) as u8;
        self.flags.expand(value as u8);
    }

    /// Returns BC.
    pub fn bc(&self) -> u16 {
        (u16::from(self.b) << 8) | u16::from(self.c)
    }

    /// Sets BC.
    pub fn set_bc(&mut self, value: u16) {
        self.b = (value >> 8) as u8;
        self.c = value as u8;
    }

    /// Returns DE.
    pub fn de(&self) -> u16 {
        (u16::from(self.d) << 8) | u16::from(self.e)
    }

    /// Sets DE.
    pub fn set_de(&mut self, value: u16) {
        self.d = (value >> 8) as u8;
        self.e = value as u8;
    }

    /// Returns HL.
    pub fn hl(&self) -> u16 {
        (u16::from(self.h) << 8) | u16::from(self.l)
    }

    /// Sets HL.
    pub fn set_hl(&mut self, value: u16) {
        self.h = (value >> 8) as u8;
        self.l = value as u8;
    }

    /// Returns the software-visible refresh register value.
    pub fn r(&self) -> u8 {
        self.r_high | (self.r & 0x7F)
    }

    /// Sets the software-visible refresh register value.
    pub fn set_r(&mut self, value: u8) {
        self.r = value & 0x7F;
        self.r_high = value & 0x80;
    }
}

impl ValidateState for Z80State {
    fn validate_state(&self, _context: &()) -> Result<(), StateValidationError> {
        if self.r > 0x7F || self.r_high & 0x7F != 0 {
            return Err(StateValidationError::new(
                "Z80 refresh register representation is invalid",
            ));
        }
        if self.im > 2 || self.ei > 1 || self.p > 1 {
            return Err(StateValidationError::new(
                "Z80 interrupt or verification latch is invalid",
            ));
        }
        if self.pending_irq & !0x03 != 0 {
            return Err(StateValidationError::new(
                "Z80 pending interrupt mask is invalid",
            ));
        }
        Ok(())
    }
}
