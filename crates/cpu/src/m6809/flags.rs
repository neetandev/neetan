/// Motorola 6809 condition code register state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct M6809Flags {
    /// Entire state flag.
    pub entire: bool,
    /// FIRQ mask flag.
    pub firq_mask: bool,
    /// Half-carry flag.
    pub half_carry: bool,
    /// IRQ mask flag.
    pub irq_mask: bool,
    /// Negative flag.
    pub negative: bool,
    /// Zero flag.
    pub zero: bool,
    /// Overflow flag.
    pub overflow: bool,
    /// Carry flag.
    pub carry: bool,
}

impl M6809Flags {
    /// Entire-state bit.
    pub const ENTIRE: u8 = 0x80;
    /// FIRQ mask bit.
    pub const FIRQ_MASK: u8 = 0x40;
    /// Half-carry bit.
    pub const HALF_CARRY: u8 = 0x20;
    /// IRQ mask bit.
    pub const IRQ_MASK: u8 = 0x10;
    /// Negative bit.
    pub const NEGATIVE: u8 = 0x08;
    /// Zero bit.
    pub const ZERO: u8 = 0x04;
    /// Overflow bit.
    pub const OVERFLOW: u8 = 0x02;
    /// Carry bit.
    pub const CARRY: u8 = 0x01;

    pub(crate) const NZ: u8 = Self::NEGATIVE | Self::ZERO;
    pub(crate) const NZV: u8 = Self::NEGATIVE | Self::ZERO | Self::OVERFLOW;
    pub(crate) const NZVC: u8 = Self::NEGATIVE | Self::ZERO | Self::OVERFLOW | Self::CARRY;
    pub(crate) const HNZVC: u8 =
        Self::HALF_CARRY | Self::NEGATIVE | Self::ZERO | Self::OVERFLOW | Self::CARRY;

    /// Creates flags from a packed condition code byte.
    pub fn new(value: u8) -> Self {
        let mut flags = Self::default();
        flags.expand(value);
        flags
    }

    /// Returns the packed condition code byte.
    pub const fn compress(self) -> u8 {
        (if self.entire { Self::ENTIRE } else { 0 })
            | (if self.firq_mask { Self::FIRQ_MASK } else { 0 })
            | (if self.half_carry { Self::HALF_CARRY } else { 0 })
            | (if self.irq_mask { Self::IRQ_MASK } else { 0 })
            | (if self.negative { Self::NEGATIVE } else { 0 })
            | (if self.zero { Self::ZERO } else { 0 })
            | (if self.overflow { Self::OVERFLOW } else { 0 })
            | (if self.carry { Self::CARRY } else { 0 })
    }

    /// Replaces all flags from a packed condition code byte.
    pub fn expand(&mut self, value: u8) {
        self.entire = value & Self::ENTIRE != 0;
        self.firq_mask = value & Self::FIRQ_MASK != 0;
        self.half_carry = value & Self::HALF_CARRY != 0;
        self.irq_mask = value & Self::IRQ_MASK != 0;
        self.negative = value & Self::NEGATIVE != 0;
        self.zero = value & Self::ZERO != 0;
        self.overflow = value & Self::OVERFLOW != 0;
        self.carry = value & Self::CARRY != 0;
    }

    pub(crate) fn set_bits(&mut self, mask: u8, value: u8) {
        let next = (self.compress() & !mask) | (value & mask);
        self.expand(next);
    }
}
