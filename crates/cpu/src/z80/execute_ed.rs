use super::{IndexMode, Z80, Z80Flags};

impl Z80 {
    fn ld_a_i_or_r(&mut self, value: u8) {
        self.set_q_latch(true);
        let iff2 = self.iff2;
        self.a = value;
        self.flags.set_subtract(false);
        self.flags.set_half_carry(false);
        self.flags.set_parity_overflow(iff2);
        self.set_xysz(value);
        self.p = 1;
    }

    fn adc_hl_rr(&mut self, pair: u8) {
        self.set_q_latch(true);
        let right = self.read_pair(pair, IndexMode::HL);
        let hl = self.hl();
        self.wz = hl.wrapping_add(1);
        let carry_in = self.flags.carry();
        let low = self.add8(self.l, right as u8, carry_in);
        let high = self.add8(self.h, (right >> 8) as u8, self.flags.carry());
        let result = (u16::from(high) << 8) | u16::from(low);
        self.set_hl(result);
        self.flags.set_zero(result == 0);
        self.clk(7);
    }

    fn sbc_hl_rr(&mut self, pair: u8) {
        self.set_q_latch(true);
        let right = self.read_pair(pair, IndexMode::HL);
        let hl = self.hl();
        self.wz = hl.wrapping_add(1);
        let carry_in = self.flags.carry();
        let low = self.sub8(self.l, right as u8, carry_in);
        let high = self.sub8(self.h, (right >> 8) as u8, self.flags.carry());
        let result = (u16::from(high) << 8) | u16::from(low);
        self.set_hl(result);
        self.flags.set_zero(result == 0);
        self.clk(7);
    }

    fn in_flags(&mut self, value: u8) -> u8 {
        self.flags.set_subtract(false);
        self.flags.set_half_carry(false);
        self.set_parity(value);
        self.set_xysz(value);
        value
    }

    fn block_compare(&mut self, bus: &mut impl common::Bus, decrement: bool) {
        self.set_q_latch(true);
        self.wz = if decrement {
            self.wz.wrapping_sub(1)
        } else {
            self.wz.wrapping_add(1)
        };
        let address = self.hl();
        let data = self.read_byte(bus, address);
        self.set_hl(if decrement {
            address.wrapping_sub(1)
        } else {
            address.wrapping_add(1)
        });
        self.clk(5);
        let result = self.a.wrapping_sub(data);
        let carry = self.flags.carry();
        let count = self.bc().wrapping_sub(1);
        self.set_bc(count);
        self.flags.set_subtract(true);
        self.flags.set_parity_overflow(count != 0);
        let half_carry = ((self.a ^ data ^ result) & 0x10) != 0;
        self.flags.set_half_carry(half_carry);
        self.flags.set_sign(result & 0x80 != 0);
        self.flags.set_zero(result == 0);
        let adjusted = result.wrapping_sub(u8::from(half_carry));
        let flags = self.flags.compress();
        self.flags.expand(
            (flags & !(Z80Flags::X | Z80Flags::Y))
                | (adjusted & Z80Flags::X)
                | ((adjusted & 0x02) << 4),
        );
        self.flags.set_carry(carry);
    }

    fn block_load(&mut self, bus: &mut impl common::Bus, decrement: bool) {
        self.set_q_latch(true);
        let source = self.hl();
        let data = self.read_byte(bus, source);
        self.set_hl(if decrement {
            source.wrapping_sub(1)
        } else {
            source.wrapping_add(1)
        });
        let destination = self.de();
        self.write_byte(bus, destination, data);
        self.set_de(if decrement {
            destination.wrapping_sub(1)
        } else {
            destination.wrapping_add(1)
        });
        self.clk(2);
        let preserve = self.flags.compress() & (Z80Flags::SIGN | Z80Flags::ZERO | Z80Flags::CARRY);
        let count = self.bc().wrapping_sub(1);
        self.set_bc(count);
        self.flags.expand(preserve);
        self.flags.set_subtract(false);
        self.flags.set_half_carry(false);
        self.flags.set_parity_overflow(count != 0);
        let xy = self.a.wrapping_add(data);
        let flags = self.flags.compress();
        self.flags.expand(
            (flags & !(Z80Flags::X | Z80Flags::Y)) | (xy & Z80Flags::X) | ((xy & 0x02) << 4),
        );
    }

    fn block_in(&mut self, bus: &mut impl common::Bus, decrement: bool) -> (u8, u16) {
        self.set_q_latch(true);
        let port = self.bc();
        self.clk(1);
        let data = self.read_port(bus, port);
        self.b = self.b.wrapping_sub(1);
        let address = self.hl();
        self.write_byte(bus, address, data);
        self.set_hl(if decrement {
            address.wrapping_sub(1)
        } else {
            address.wrapping_add(1)
        });
        let carry_base = if decrement {
            self.c.wrapping_sub(1)
        } else {
            self.c.wrapping_add(1)
        };
        let carry = u16::from(carry_base) + u16::from(data) > 0xFF;
        let pv_value = (carry_base.wrapping_add(data) & 7) ^ self.b;
        self.flags.set_carry(carry);
        self.flags.set_subtract(data & 0x80 != 0);
        self.flags.set_parity_overflow(Self::parity_even(pv_value));
        self.set_xysz(self.b);
        self.flags.set_half_carry(carry);
        (data, port)
    }

    fn block_out(&mut self, bus: &mut impl common::Bus, decrement: bool) -> u8 {
        self.set_q_latch(true);
        self.clk(1);
        let address = self.hl();
        let data = self.read_byte(bus, address);
        self.set_hl(if decrement {
            address.wrapping_sub(1)
        } else {
            address.wrapping_add(1)
        });
        self.b = self.b.wrapping_sub(1);
        let port = self.bc();
        self.write_port(bus, port, data);
        self.wz = if decrement {
            port.wrapping_sub(1)
        } else {
            port.wrapping_add(1)
        };
        let l_value = self.l;
        let carry = u16::from(l_value) + u16::from(data) > 0xFF;
        let pv_value = (l_value.wrapping_add(data) & 7) ^ self.b;
        self.flags.set_carry(carry);
        self.flags.set_subtract(data & 0x80 != 0);
        self.flags.set_parity_overflow(Self::parity_even(pv_value));
        self.set_xysz(self.b);
        self.flags.set_half_carry(carry);
        data
    }

    fn repeat_xy_from_pc(&mut self) {
        self.set_xy((self.pc >> 8) as u8);
    }

    fn repeat_io_adjust_flags(&mut self, data: u8) {
        self.repeat_xy_from_pc();
        let mut parity = self.flags.parity_overflow();
        if self.flags.carry() {
            if data & 0x80 != 0 {
                parity ^= !Self::parity_even(self.b.wrapping_sub(1) & 7);
                let half_carry = self.b & 0x0F == 0;
                self.flags.set_half_carry(half_carry);
            } else {
                parity ^= !Self::parity_even(self.b.wrapping_add(1) & 7);
                let half_carry = self.b & 0x0F == 0x0F;
                self.flags.set_half_carry(half_carry);
            }
        } else {
            parity ^= !Self::parity_even(self.b & 7);
        }
        self.flags.set_parity_overflow(parity);
    }

    pub(crate) fn execute_ed(&mut self, opcode: u8, bus: &mut impl common::Bus) {
        let x = opcode >> 6;
        let y = (opcode >> 3) & 7;
        let z = opcode & 7;
        let p = y >> 1;
        let q = y & 1;

        match (x, z) {
            (1, 0) => {
                self.set_q_latch(true);
                self.wz = self.bc().wrapping_add(1);
                let port = self.bc();
                let raw = self.read_port(bus, port);
                let value = self.in_flags(raw);
                if y != 6 {
                    self.write_reg8_plain(y, IndexMode::HL, value);
                }
            }
            (1, 1) => {
                self.set_q_latch(false);
                let value = if y == 6 {
                    0
                } else {
                    self.read_reg8_plain(y, IndexMode::HL)
                };
                let port = self.bc();
                self.write_port(bus, port, value);
                self.wz = port.wrapping_add(1);
            }
            (1, 2) if q == 0 => self.sbc_hl_rr(p),
            (1, 2) => self.adc_hl_rr(p),
            (1, 3) if q == 0 => {
                self.set_q_latch(false);
                let address = self.fetch_u16(bus);
                let value = self.read_pair(p, IndexMode::HL);
                self.write_byte(bus, address, value as u8);
                self.write_byte(bus, address.wrapping_add(1), (value >> 8) as u8);
                self.wz = address.wrapping_add(1);
            }
            (1, 3) => {
                self.set_q_latch(false);
                let address = self.fetch_u16(bus);
                let low = self.read_byte(bus, address);
                let high = self.read_byte(bus, address.wrapping_add(1));
                self.write_pair(p, IndexMode::HL, u16::from(low) | (u16::from(high) << 8));
                self.wz = address.wrapping_add(1);
            }
            (1, 4) => {
                self.set_q_latch(true);
                self.a = self.sub8(0, self.a, false);
            }
            (1, 5) => {
                self.set_q_latch(false);
                self.wz = self.pop(bus);
                self.pc = self.wz;
                self.iff1 = self.iff2;
                // Opcode 0x4D (y == 1) is RETI; the rest of the group are RETN.
                // RETI signals the interrupt daisy chain to dismiss the handler.
                if y == 1 {
                    bus.notify_reti();
                }
            }
            (1, 6) => {
                self.set_q_latch(false);
                self.im = match y {
                    0 | 1 | 4 | 5 => 0,
                    2 | 6 => 1,
                    3 | 7 => 2,
                    _ => 0,
                };
            }
            (1, 7) => match y {
                0 => {
                    self.set_q_latch(false);
                    self.clk(1);
                    self.i = self.a;
                }
                1 => {
                    self.set_q_latch(false);
                    self.clk(1);
                    let a = self.a;
                    self.set_r(a);
                }
                2 => {
                    self.clk(1);
                    self.ld_a_i_or_r(self.i);
                }
                3 => {
                    self.clk(1);
                    self.ld_a_i_or_r(self.r());
                }
                4 => {
                    self.set_q_latch(true);
                    let address = self.hl();
                    self.wz = address.wrapping_add(1);
                    let data = self.read_byte(bus, address);
                    self.clk(4);
                    self.write_byte(bus, address, (data >> 4) | (self.a << 4));
                    self.a = (self.a & 0xF0) | (data & 0x0F);
                    self.flags.set_subtract(false);
                    self.flags.set_half_carry(false);
                    self.set_parity(self.a);
                    self.set_xysz(self.a);
                }
                5 => {
                    self.set_q_latch(true);
                    let address = self.hl();
                    self.wz = address.wrapping_add(1);
                    let data = self.read_byte(bus, address);
                    self.clk(4);
                    self.write_byte(bus, address, (data << 4) | (self.a & 0x0F));
                    self.a = (self.a & 0xF0) | (data >> 4);
                    self.flags.set_subtract(false);
                    self.flags.set_half_carry(false);
                    self.set_parity(self.a);
                    self.set_xysz(self.a);
                }
                _ => self.set_q_latch(false),
            },
            (2, 0) => match y {
                4 => self.block_load(bus, false),
                5 => self.block_load(bus, true),
                6 => {
                    self.block_load(bus, false);
                    if self.bc() != 0 {
                        self.clk(5);
                        self.pc = self.pc.wrapping_sub(2);
                        self.wz = self.pc.wrapping_add(1);
                        self.repeat_xy_from_pc();
                    }
                }
                7 => {
                    self.block_load(bus, true);
                    if self.bc() != 0 {
                        self.clk(5);
                        self.pc = self.pc.wrapping_sub(2);
                        self.wz = self.pc.wrapping_add(1);
                        self.repeat_xy_from_pc();
                    }
                }
                _ => self.set_q_latch(false),
            },
            (2, 1) => match y {
                4 => self.block_compare(bus, false),
                5 => self.block_compare(bus, true),
                6 => {
                    self.block_compare(bus, false);
                    if self.bc() != 0 && !self.flags.zero() {
                        self.clk(5);
                        self.pc = self.pc.wrapping_sub(2);
                        self.wz = self.pc.wrapping_add(1);
                        self.repeat_xy_from_pc();
                    }
                }
                7 => {
                    self.block_compare(bus, true);
                    if self.bc() != 0 && !self.flags.zero() {
                        self.clk(5);
                        self.pc = self.pc.wrapping_sub(2);
                        self.wz = self.pc.wrapping_add(1);
                        self.repeat_xy_from_pc();
                    }
                }
                _ => self.set_q_latch(false),
            },
            (2, 2) => match y {
                4 => {
                    let (_data, port) = self.block_in(bus, false);
                    self.wz = port.wrapping_add(1);
                }
                5 => {
                    let (_data, port) = self.block_in(bus, true);
                    self.wz = port.wrapping_sub(1);
                }
                6 => {
                    let (data, port) = self.block_in(bus, false);
                    self.wz = port.wrapping_add(1);
                    if self.b != 0 {
                        self.clk(5);
                        self.pc = self.pc.wrapping_sub(2);
                        self.wz = self.pc.wrapping_add(1);
                        self.repeat_io_adjust_flags(data);
                    }
                }
                7 => {
                    let (data, port) = self.block_in(bus, true);
                    self.wz = port.wrapping_sub(1);
                    if self.b != 0 {
                        self.clk(5);
                        self.pc = self.pc.wrapping_sub(2);
                        self.wz = self.pc.wrapping_add(1);
                        self.repeat_io_adjust_flags(data);
                    }
                }
                _ => self.set_q_latch(false),
            },
            (2, 3) => match y {
                4 => {
                    self.block_out(bus, false);
                }
                5 => {
                    self.block_out(bus, true);
                }
                6 => {
                    let data = self.block_out(bus, false);
                    if self.b != 0 {
                        self.clk(5);
                        self.pc = self.pc.wrapping_sub(2);
                        self.wz = self.pc.wrapping_add(1);
                        self.repeat_io_adjust_flags(data);
                    }
                }
                7 => {
                    let data = self.block_out(bus, true);
                    if self.b != 0 {
                        self.clk(5);
                        self.pc = self.pc.wrapping_sub(2);
                        self.wz = self.pc.wrapping_add(1);
                        self.repeat_io_adjust_flags(data);
                    }
                }
                _ => self.set_q_latch(false),
            },
            _ => self.set_q_latch(false),
        }
    }
}
