use super::Z80;

impl Z80 {
    pub(crate) fn parity_even(value: u8) -> bool {
        value.count_ones() & 1 == 0
    }

    pub(crate) fn set_xy(&mut self, value: u8) {
        self.flags.set_xy(value);
    }

    pub(crate) fn set_sz(&mut self, value: u8) {
        self.flags.set_sign(value & 0x80 != 0);
        self.flags.set_zero(value == 0);
    }

    pub(crate) fn set_xysz(&mut self, value: u8) {
        self.set_xy(value);
        self.set_sz(value);
    }

    pub(crate) fn set_parity(&mut self, value: u8) {
        self.flags.set_parity_overflow(Self::parity_even(value));
    }

    pub(crate) fn add8(&mut self, left: u8, right: u8, carry: bool) -> u8 {
        let carry_value = u16::from(carry);
        let sum = u16::from(left) + u16::from(right) + carry_value;
        let result = sum as u8;
        self.flags.set_carry(sum > 0xFF);
        self.flags.set_subtract(false);
        self.flags
            .set_parity_overflow((!((left ^ right) as u16) & ((left ^ result) as u16) & 0x80) != 0);
        self.flags
            .set_half_carry(((left ^ right ^ result) & 0x10) != 0);
        self.set_xysz(result);
        result
    }

    pub(crate) fn sub8(&mut self, left: u8, right: u8, carry: bool) -> u8 {
        let carry_value = i16::from(carry);
        let diff = i16::from(left) - i16::from(right) - carry_value;
        let result = diff as u8;
        self.flags.set_carry(diff < 0);
        self.flags.set_subtract(true);
        self.flags
            .set_parity_overflow(((left ^ right) & (left ^ result) & 0x80) != 0);
        self.flags
            .set_half_carry(((left ^ right ^ result) & 0x10) != 0);
        self.set_xysz(result);
        result
    }

    pub(crate) fn and8(&mut self, left: u8, right: u8) -> u8 {
        let result = left & right;
        self.flags.set_carry(false);
        self.flags.set_subtract(false);
        self.flags.set_half_carry(true);
        self.set_parity(result);
        self.set_xysz(result);
        result
    }

    pub(crate) fn or8(&mut self, left: u8, right: u8) -> u8 {
        let result = left | right;
        self.flags.set_carry(false);
        self.flags.set_subtract(false);
        self.flags.set_half_carry(false);
        self.set_parity(result);
        self.set_xysz(result);
        result
    }

    pub(crate) fn xor8(&mut self, left: u8, right: u8) -> u8 {
        let result = left ^ right;
        self.flags.set_carry(false);
        self.flags.set_subtract(false);
        self.flags.set_half_carry(false);
        self.set_parity(result);
        self.set_xysz(result);
        result
    }

    /// Sets subtraction flags and leaves A unchanged.
    pub(crate) fn cp8(&mut self, left: u8, right: u8) {
        let diff = left.wrapping_sub(right);
        let borrow = u16::from(left) < u16::from(right);
        self.flags.set_carry(borrow);
        self.flags.set_subtract(true);
        self.set_xy(right);
        self.set_sz(diff);
        self.flags
            .set_parity_overflow(((left ^ right) & (left ^ diff) & 0x80) != 0);
        self.flags
            .set_half_carry(((left ^ right ^ diff) & 0x10) != 0);
    }

    pub(crate) fn inc8(&mut self, value: u8) -> u8 {
        let result = value.wrapping_add(1);
        let carry = self.flags.carry();
        self.flags.set_subtract(false);
        self.flags.set_parity_overflow(result == 0x80);
        self.set_xysz(result);
        self.flags.set_half_carry(result & 0x0F == 0);
        self.flags.set_carry(carry);
        result
    }

    pub(crate) fn dec8(&mut self, value: u8) -> u8 {
        let result = value.wrapping_sub(1);
        let carry = self.flags.carry();
        self.flags.set_subtract(true);
        self.flags.set_parity_overflow(result == 0x7F);
        self.set_xysz(result);
        self.flags.set_half_carry(result & 0x0F == 0x0F);
        self.flags.set_carry(carry);
        result
    }

    pub(crate) fn bit_test(&mut self, bit: u8, value: u8) {
        let mask = 1u8 << (bit & 7);
        let masked = value & mask;
        let carry = self.flags.carry();
        self.flags.set_subtract(false);
        self.flags.set_half_carry(true);
        self.set_parity(masked);
        self.set_xy(value);
        self.set_sz(masked);
        self.flags.set_carry(carry);
    }

    pub(crate) fn bit_test_with_xy(&mut self, bit: u8, value: u8, xy_source: u8) {
        self.bit_test(bit, value);
        self.set_xy(xy_source);
    }

    pub(crate) fn rlc(&mut self, value: u8) -> u8 {
        let result = value.rotate_left(1);
        self.flags.set_carry(result & 1 != 0);
        self.flags.set_subtract(false);
        self.flags.set_half_carry(false);
        self.set_parity(result);
        self.set_xysz(result);
        result
    }

    pub(crate) fn rrc(&mut self, value: u8) -> u8 {
        let result = value.rotate_right(1);
        self.flags.set_carry(result & 0x80 != 0);
        self.flags.set_subtract(false);
        self.flags.set_half_carry(false);
        self.set_parity(result);
        self.set_xysz(result);
        result
    }

    pub(crate) fn rl(&mut self, value: u8) -> u8 {
        let carry_in = u8::from(self.flags.carry());
        let carry_out = value & 0x80 != 0;
        let result = (value << 1) | carry_in;
        self.flags.set_carry(carry_out);
        self.flags.set_subtract(false);
        self.flags.set_half_carry(false);
        self.set_parity(result);
        self.set_xysz(result);
        result
    }

    pub(crate) fn rr(&mut self, value: u8) -> u8 {
        let carry_in = u8::from(self.flags.carry()) << 7;
        let carry_out = value & 1 != 0;
        let result = (value >> 1) | carry_in;
        self.flags.set_carry(carry_out);
        self.flags.set_subtract(false);
        self.flags.set_half_carry(false);
        self.set_parity(result);
        self.set_xysz(result);
        result
    }

    pub(crate) fn sla(&mut self, value: u8) -> u8 {
        let carry_out = value & 0x80 != 0;
        let result = value << 1;
        self.flags.set_carry(carry_out);
        self.flags.set_subtract(false);
        self.flags.set_half_carry(false);
        self.set_parity(result);
        self.set_xysz(result);
        result
    }

    pub(crate) fn sll(&mut self, value: u8) -> u8 {
        let carry_out = value & 0x80 != 0;
        let result = (value << 1) | 1;
        self.flags.set_carry(carry_out);
        self.flags.set_subtract(false);
        self.flags.set_half_carry(false);
        self.set_parity(result);
        self.set_xysz(result);
        result
    }

    pub(crate) fn sra(&mut self, value: u8) -> u8 {
        let carry_out = value & 1 != 0;
        let result = (value & 0x80) | (value >> 1);
        self.flags.set_carry(carry_out);
        self.flags.set_subtract(false);
        self.flags.set_half_carry(false);
        self.set_parity(result);
        self.set_xysz(result);
        result
    }

    pub(crate) fn srl(&mut self, value: u8) -> u8 {
        let carry_out = value & 1 != 0;
        let result = value >> 1;
        self.flags.set_carry(carry_out);
        self.flags.set_subtract(false);
        self.flags.set_half_carry(false);
        self.set_parity(result);
        self.set_xysz(result);
        result
    }
}
