use super::{M6809, M6809Flags};

impl M6809 {
    #[inline(always)]
    fn set_flags8(&mut self, mask: u8, left: u8, right: u8, result: u32) -> u8 {
        let mut bits = self.flags.compress() & !mask;
        if mask & M6809Flags::HALF_CARRY != 0 && ((left as u32 ^ right as u32 ^ result) & 0x10) != 0
        {
            bits |= M6809Flags::HALF_CARRY;
        }
        if mask & M6809Flags::NEGATIVE != 0 && result & 0x80 != 0 {
            bits |= M6809Flags::NEGATIVE;
        }
        if mask & M6809Flags::ZERO != 0 && result as u8 == 0 {
            bits |= M6809Flags::ZERO;
        }
        if mask & M6809Flags::OVERFLOW != 0
            && ((left as u32 ^ right as u32 ^ result ^ (result >> 1)) & 0x80) != 0
        {
            bits |= M6809Flags::OVERFLOW;
        }
        if mask & M6809Flags::CARRY != 0 && result & 0x100 != 0 {
            bits |= M6809Flags::CARRY;
        }
        self.flags.expand(bits);
        result as u8
    }

    #[inline(always)]
    fn set_flags16(&mut self, mask: u8, left: u16, right: u16, result: u32) -> u16 {
        let mut bits = self.flags.compress() & !mask;
        if mask & M6809Flags::HALF_CARRY != 0 && ((left as u32 ^ right as u32 ^ result) & 0x10) != 0
        {
            bits |= M6809Flags::HALF_CARRY;
        }
        if mask & M6809Flags::NEGATIVE != 0 && result & 0x8000 != 0 {
            bits |= M6809Flags::NEGATIVE;
        }
        if mask & M6809Flags::ZERO != 0 && result as u16 == 0 {
            bits |= M6809Flags::ZERO;
        }
        if mask & M6809Flags::OVERFLOW != 0
            && ((left as u32 ^ right as u32 ^ result ^ (result >> 1)) & 0x8000) != 0
        {
            bits |= M6809Flags::OVERFLOW;
        }
        if mask & M6809Flags::CARRY != 0 && result & 0x1_0000 != 0 {
            bits |= M6809Flags::CARRY;
        }
        self.flags.expand(bits);
        result as u16
    }

    #[inline(always)]
    pub(crate) fn set_nz8(&mut self, value: u8) -> u8 {
        self.set_flags8(M6809Flags::NZ, 0, value, u32::from(value))
    }

    #[inline(always)]
    pub(crate) fn set_nzv8(&mut self, value: u8) -> u8 {
        self.set_flags8(M6809Flags::NZV, 0, value, u32::from(value))
    }

    #[inline(always)]
    pub(crate) fn set_nzv16(&mut self, value: u16) -> u16 {
        self.set_flags16(M6809Flags::NZV, 0, value, u32::from(value))
    }

    #[inline(always)]
    pub(crate) fn set_z16(&mut self, value: u16) -> u16 {
        self.set_flags16(M6809Flags::ZERO, 0, value, u32::from(value))
    }

    #[inline(always)]
    pub(crate) fn neg8(&mut self, value: u8) -> u8 {
        self.set_flags8(
            M6809Flags::NZVC,
            0,
            value,
            0u32.wrapping_sub(u32::from(value)),
        )
    }

    #[inline(always)]
    pub(crate) fn com8(&mut self, value: u8) -> u8 {
        self.flags.overflow = false;
        self.flags.carry = true;
        self.set_nz8(!value)
    }

    #[inline(always)]
    pub(crate) fn lsr8(&mut self, value: u8) -> u8 {
        self.flags.carry = value & 0x01 != 0;
        self.set_nz8(value >> 1)
    }

    #[inline(always)]
    pub(crate) fn ror8(&mut self, value: u8) -> u8 {
        let old_carry = self.flags.carry;
        self.flags.carry = value & 0x01 != 0;
        let result = (value >> 1) | if old_carry { 0x80 } else { 0 };
        self.set_nz8(result)
    }

    #[inline(always)]
    pub(crate) fn asr8(&mut self, value: u8) -> u8 {
        self.flags.carry = value & 0x01 != 0;
        self.set_nz8((value >> 1) | (value & 0x80))
    }

    #[inline(always)]
    pub(crate) fn asl8(&mut self, value: u8) -> u8 {
        self.set_flags8(M6809Flags::NZVC, value, value, u32::from(value) << 1)
    }

    #[inline(always)]
    pub(crate) fn rol8(&mut self, value: u8) -> u8 {
        let old_carry = self.flags.carry;
        self.flags.carry = value & 0x80 != 0;
        let result = (u32::from(value) << 1) | u32::from(old_carry);
        self.set_flags8(M6809Flags::NZV, value, value, result)
    }

    #[inline(always)]
    pub(crate) fn dec8(&mut self, value: u8) -> u8 {
        self.set_flags8(M6809Flags::NZV, value, 1, u32::from(value).wrapping_sub(1))
    }

    #[inline(always)]
    pub(crate) fn xdec8(&mut self, value: u8) -> u8 {
        self.flags.carry = value != 0;
        self.dec8(value)
    }

    #[inline(always)]
    pub(crate) fn inc8(&mut self, value: u8) -> u8 {
        self.set_flags8(M6809Flags::NZV, value, 1, u32::from(value) + 1)
    }

    #[inline(always)]
    pub(crate) fn tst8(&mut self, value: u8) {
        self.set_nzv8(value);
    }

    #[inline(always)]
    pub(crate) fn clr8(&mut self) -> u8 {
        self.flags.set_bits(M6809Flags::NZVC, M6809Flags::ZERO);
        0
    }

    #[inline(always)]
    pub(crate) fn xclr8(&mut self) -> u8 {
        self.flags.set_bits(M6809Flags::NZV, M6809Flags::ZERO);
        0
    }

    #[inline(always)]
    pub(crate) fn sub8(&mut self, left: u8, right: u8) -> u8 {
        self.set_flags8(
            M6809Flags::NZVC,
            left,
            right,
            u32::from(left).wrapping_sub(u32::from(right)),
        )
    }

    #[inline(always)]
    pub(crate) fn sbc8(&mut self, left: u8, right: u8) -> u8 {
        let carry = u32::from(self.flags.carry);
        self.set_flags8(
            M6809Flags::NZVC,
            left,
            right,
            u32::from(left)
                .wrapping_sub(u32::from(right))
                .wrapping_sub(carry),
        )
    }

    #[inline(always)]
    pub(crate) fn cmp8(&mut self, left: u8, right: u8) {
        self.sub8(left, right);
    }

    #[inline(always)]
    pub(crate) fn and8(&mut self, left: u8, right: u8) -> u8 {
        self.flags.overflow = false;
        self.set_nz8(left & right)
    }

    #[inline(always)]
    pub(crate) fn bit8(&mut self, left: u8, right: u8) {
        self.flags.overflow = false;
        self.set_nz8(left & right);
    }

    #[inline(always)]
    pub(crate) fn eor8(&mut self, left: u8, right: u8) -> u8 {
        self.flags.overflow = false;
        self.set_nz8(left ^ right)
    }

    #[inline(always)]
    pub(crate) fn adc8(&mut self, left: u8, right: u8) -> u8 {
        let carry = u32::from(self.flags.carry);
        self.set_flags8(
            M6809Flags::HNZVC,
            left,
            right,
            u32::from(left) + u32::from(right) + carry,
        )
    }

    #[inline(always)]
    pub(crate) fn or8(&mut self, left: u8, right: u8) -> u8 {
        self.flags.overflow = false;
        self.set_nz8(left | right)
    }

    #[inline(always)]
    pub(crate) fn add8(&mut self, left: u8, right: u8) -> u8 {
        self.set_flags8(
            M6809Flags::HNZVC,
            left,
            right,
            u32::from(left) + u32::from(right),
        )
    }

    #[inline(always)]
    pub(crate) fn add16(&mut self, left: u16, right: u16) -> u16 {
        self.set_flags16(
            M6809Flags::NZVC,
            left,
            right,
            u32::from(left) + u32::from(right),
        )
    }

    #[inline(always)]
    pub(crate) fn sub16(&mut self, left: u16, right: u16) -> u16 {
        self.set_flags16(
            M6809Flags::NZVC,
            left,
            right,
            u32::from(left).wrapping_sub(u32::from(right)),
        )
    }

    #[inline(always)]
    pub(crate) fn cmp16(&mut self, left: u16, right: u16) {
        self.sub16(left, right);
    }

    #[inline(always)]
    pub(crate) fn daa(&mut self) {
        let mut correction = 0u16;
        let most_significant_nibble = self.a & 0xF0;
        let least_significant_nibble = self.a & 0x0F;

        if least_significant_nibble > 0x09 || self.flags.half_carry {
            correction |= 0x06;
        }
        if most_significant_nibble > 0x80 && least_significant_nibble > 0x09 {
            correction |= 0x60;
        }
        if most_significant_nibble > 0x90 || self.flags.carry {
            correction |= 0x60;
        }

        let result = u16::from(self.a) + correction;
        self.flags.overflow = false;
        if result & 0x0100 != 0 {
            self.flags.carry = true;
        }
        self.a = self.set_nz8(result as u8);
    }

    #[inline(always)]
    pub(crate) fn mul(&mut self) {
        let result = u16::from(self.a) * u16::from(self.b);
        let value = self.set_flags16(M6809Flags::ZERO, 0, result, u32::from(result));
        self.set_d(value);
        self.flags.carry = self.d() & 0x0080 != 0;
    }

    #[inline(always)]
    pub(crate) fn sex(&mut self) {
        let value = self.b as i8 as i16 as u16;
        let result = self.set_flags16(M6809Flags::NZ, 0, value, u32::from(value));
        self.set_d(result);
    }
}
