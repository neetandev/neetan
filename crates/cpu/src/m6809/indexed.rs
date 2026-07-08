use super::{EffectiveAddress, M6809};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IndexRegister {
    X,
    Y,
    U,
    S,
}

impl M6809 {
    pub(crate) fn indexed_address(&mut self, bus: &mut impl common::Bus) -> EffectiveAddress {
        let postbyte = self.fetch_u8(bus);
        let index_register = match postbyte & 0x60 {
            0x00 => IndexRegister::X,
            0x20 => IndexRegister::Y,
            0x40 => IndexRegister::U,
            0x60 => IndexRegister::S,
            _ => IndexRegister::X,
        };
        let extra_cycles = indexed_extra_cycles(postbyte);

        let mut address = if postbyte & 0x80 == 0 {
            let offset = ((postbyte & 0x0F) | if postbyte & 0x10 != 0 { 0xF0 } else { 0 }) as i8;
            self.index_register(index_register)
                .wrapping_add_signed(i16::from(offset))
        } else {
            match postbyte & 0x0F {
                0x00 => {
                    let value = self.index_register(index_register);
                    self.set_index_register(index_register, value.wrapping_add(1));
                    value
                }
                0x01 => {
                    let value = self.index_register(index_register);
                    self.set_index_register(index_register, value.wrapping_add(2));
                    value
                }
                0x02 => {
                    let value = self.index_register(index_register).wrapping_sub(1);
                    self.set_index_register(index_register, value);
                    value
                }
                0x03 => {
                    let value = self.index_register(index_register).wrapping_sub(2);
                    self.set_index_register(index_register, value);
                    value
                }
                0x04 => self.index_register(index_register),
                0x05 => self
                    .index_register(index_register)
                    .wrapping_add_signed(i16::from(self.b as i8)),
                0x06 => self
                    .index_register(index_register)
                    .wrapping_add_signed(i16::from(self.a as i8)),
                0x08 => {
                    let offset = self.fetch_u8(bus) as i8;
                    self.index_register(index_register)
                        .wrapping_add_signed(i16::from(offset))
                }
                0x09 => {
                    let offset = self.fetch_u16(bus);
                    self.index_register(index_register).wrapping_add(offset)
                }
                0x0B => self.index_register(index_register).wrapping_add(self.d()),
                0x0C => {
                    let offset = self.fetch_u8(bus) as i8;
                    self.pc.wrapping_add_signed(i16::from(offset))
                }
                0x0D => {
                    let offset = self.fetch_u16(bus);
                    self.pc.wrapping_add(offset)
                }
                0x0F => self.fetch_u16(bus),
                0x07 | 0x0A | 0x0E => 0,
                _ => 0,
            }
        };

        if postbyte & 0x90 == 0x90 {
            address = self.read_word(bus, address);
        }

        EffectiveAddress {
            address,
            extra_cycles,
        }
    }

    #[inline(always)]
    fn index_register(&self, register: IndexRegister) -> u16 {
        match register {
            IndexRegister::X => self.x,
            IndexRegister::Y => self.y,
            IndexRegister::U => self.u,
            IndexRegister::S => self.s,
        }
    }

    #[inline(always)]
    fn set_index_register(&mut self, register: IndexRegister, value: u16) {
        match register {
            IndexRegister::X => self.x = value,
            IndexRegister::Y => self.y = value,
            IndexRegister::U => self.u = value,
            IndexRegister::S => self.s = value,
        }
    }
}

#[inline(always)]
fn indexed_extra_cycles(postbyte: u8) -> i32 {
    if postbyte & 0x80 == 0 {
        return 1;
    }

    let indirect = postbyte & 0x10 != 0;
    match postbyte & 0x0F {
        0x00 | 0x02 => {
            if indirect {
                5
            } else {
                2
            }
        }
        0x01 | 0x03 => {
            if indirect {
                6
            } else {
                3
            }
        }
        0x04 => {
            if indirect {
                3
            } else {
                0
            }
        }
        0x05 | 0x06 | 0x08 | 0x0C => {
            if indirect {
                4
            } else {
                1
            }
        }
        0x09 | 0x0B => {
            if indirect {
                7
            } else {
                4
            }
        }
        0x0D => {
            if indirect {
                8
            } else {
                5
            }
        }
        0x0F => {
            if indirect {
                5
            } else {
                2
            }
        }
        0x07 | 0x0A | 0x0E => {
            if indirect {
                2
            } else {
                -1
            }
        }
        _ => 0,
    }
}
