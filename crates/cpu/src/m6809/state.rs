use super::flags::M6809Flags;

/// Snapshot of all Motorola 6809 registers.
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
