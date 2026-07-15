save_state::runtime_state! {
    /// Z80 flags register state.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub struct Z80Flags {
        bits: u8,
    }
}

impl Z80Flags {
    /// Carry flag.
    pub const CARRY: u8 = 0x01;
    /// Add/subtract flag.
    pub const SUBTRACT: u8 = 0x02;
    /// Parity/overflow flag.
    pub const PARITY_OVERFLOW: u8 = 0x04;
    /// Undocumented X flag.
    pub const X: u8 = 0x08;
    /// Half-carry flag.
    pub const HALF_CARRY: u8 = 0x10;
    /// Undocumented Y flag.
    pub const Y: u8 = 0x20;
    /// Zero flag.
    pub const ZERO: u8 = 0x40;
    /// Sign flag.
    pub const SIGN: u8 = 0x80;

    /// Creates flags from the raw flag byte.
    pub const fn new(bits: u8) -> Self {
        Self { bits }
    }

    /// Returns the raw flag byte.
    pub const fn compress(self) -> u8 {
        self.bits
    }

    /// Replaces the raw flag byte.
    pub fn expand(&mut self, bits: u8) {
        self.bits = bits;
    }

    /// Returns the carry flag.
    pub const fn carry(self) -> bool {
        self.bits & Self::CARRY != 0
    }

    /// Sets the carry flag.
    pub fn set_carry(&mut self, value: bool) {
        self.set(Self::CARRY, value);
    }

    /// Returns the subtract flag.
    pub const fn subtract(self) -> bool {
        self.bits & Self::SUBTRACT != 0
    }

    /// Sets the subtract flag.
    pub fn set_subtract(&mut self, value: bool) {
        self.set(Self::SUBTRACT, value);
    }

    /// Returns the parity/overflow flag.
    pub const fn parity_overflow(self) -> bool {
        self.bits & Self::PARITY_OVERFLOW != 0
    }

    /// Sets the parity/overflow flag.
    pub fn set_parity_overflow(&mut self, value: bool) {
        self.set(Self::PARITY_OVERFLOW, value);
    }

    /// Returns the half-carry flag.
    pub const fn half_carry(self) -> bool {
        self.bits & Self::HALF_CARRY != 0
    }

    /// Sets the half-carry flag.
    pub fn set_half_carry(&mut self, value: bool) {
        self.set(Self::HALF_CARRY, value);
    }

    /// Returns the zero flag.
    pub const fn zero(self) -> bool {
        self.bits & Self::ZERO != 0
    }

    /// Sets the zero flag.
    pub fn set_zero(&mut self, value: bool) {
        self.set(Self::ZERO, value);
    }

    /// Returns the sign flag.
    pub const fn sign(self) -> bool {
        self.bits & Self::SIGN != 0
    }

    /// Sets the sign flag.
    pub fn set_sign(&mut self, value: bool) {
        self.set(Self::SIGN, value);
    }

    /// Updates the X and Y bits from the provided byte.
    pub fn set_xy(&mut self, value: u8) {
        self.bits = (self.bits & !(Self::X | Self::Y)) | (value & (Self::X | Self::Y));
    }

    fn set(&mut self, bit: u8, value: bool) {
        if value {
            self.bits |= bit;
        } else {
            self.bits &= !bit;
        }
    }
}
