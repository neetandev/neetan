use super::{I386, Step};
use crate::{ByteReg, DwordReg, SegReg32, WordReg, build_x86_reg_word_table, build_x86_rm_table};

static MODRM_REG: [u8; 256] = build_x86_reg_word_table();
static MODRM_RM: [u8; 256] = build_x86_rm_table();

impl<const CPU_MODEL: u8, const ADDRESS_WIDTH: u8> I386<CPU_MODEL, ADDRESS_WIDTH> {
    #[inline(always)]
    pub(super) fn reg_dword(&self, modrm: u8) -> DwordReg {
        DwordReg::from_index(MODRM_REG[modrm as usize])
    }

    #[inline(always)]
    pub(super) fn reg_word(&self, modrm: u8) -> WordReg {
        WordReg::from_index(MODRM_REG[modrm as usize])
    }

    #[inline(always)]
    pub(super) fn reg_byte(&self, modrm: u8) -> ByteReg {
        ByteReg::from_index(MODRM_REG[modrm as usize])
    }

    #[inline(always)]
    pub(super) fn rm_word(&self, modrm: u8) -> WordReg {
        WordReg::from_index(MODRM_RM[modrm as usize])
    }

    #[inline(always)]
    pub(super) fn rm_dword(&self, modrm: u8) -> DwordReg {
        DwordReg::from_index(MODRM_RM[modrm as usize])
    }

    #[inline(always)]
    pub(super) fn rm_byte(&self, modrm: u8) -> ByteReg {
        ByteReg::from_index(MODRM_RM[modrm as usize])
    }

    pub(super) fn calc_ea(&mut self, modrm: u8, bus: &mut impl common::Bus) {
        if self.address_size_override {
            self.calc_ea32(modrm, bus);
        } else {
            self.calc_ea16(modrm, bus);
        }
    }

    fn calc_ea16(&mut self, modrm: u8, bus: &mut impl common::Bus) {
        let mode = modrm >> 6;
        let rm = modrm & 7;

        match mode {
            0 => match rm {
                0 => {
                    self.eo = self
                        .regs
                        .word(WordReg::BX)
                        .wrapping_add(self.regs.word(WordReg::SI));
                    self.eo32 = self.eo as u32;
                    self.ea_seg = self.default_seg(SegReg32::DS);
                    self.ea = self.seg_base(self.ea_seg).wrapping_add(self.eo32);
                }
                1 => {
                    self.eo = self
                        .regs
                        .word(WordReg::BX)
                        .wrapping_add(self.regs.word(WordReg::DI));
                    self.eo32 = self.eo as u32;
                    self.ea_seg = self.default_seg(SegReg32::DS);
                    self.ea = self.seg_base(self.ea_seg).wrapping_add(self.eo32);
                }
                2 => {
                    self.eo = self
                        .regs
                        .word(WordReg::BP)
                        .wrapping_add(self.regs.word(WordReg::SI));
                    self.eo32 = self.eo as u32;
                    self.ea_seg = self.default_seg(SegReg32::SS);
                    self.ea = self.seg_base(self.ea_seg).wrapping_add(self.eo32);
                }
                3 => {
                    self.eo = self
                        .regs
                        .word(WordReg::BP)
                        .wrapping_add(self.regs.word(WordReg::DI));
                    self.eo32 = self.eo as u32;
                    self.ea_seg = self.default_seg(SegReg32::SS);
                    self.ea = self.seg_base(self.ea_seg).wrapping_add(self.eo32);
                }
                4 => {
                    self.eo = self.regs.word(WordReg::SI);
                    self.eo32 = self.eo as u32;
                    self.ea_seg = self.default_seg(SegReg32::DS);
                    self.ea = self.seg_base(self.ea_seg).wrapping_add(self.eo32);
                }
                5 => {
                    self.eo = self.regs.word(WordReg::DI);
                    self.eo32 = self.eo as u32;
                    self.ea_seg = self.default_seg(SegReg32::DS);
                    self.ea = self.seg_base(self.ea_seg).wrapping_add(self.eo32);
                }
                6 => {
                    self.eo = self.fetchword(bus);
                    self.eo32 = self.eo as u32;
                    self.ea_seg = self.default_seg(SegReg32::DS);
                    self.ea = self.seg_base(self.ea_seg).wrapping_add(self.eo32);
                }
                7 => {
                    self.eo = self.regs.word(WordReg::BX);
                    self.eo32 = self.eo as u32;
                    self.ea_seg = self.default_seg(SegReg32::DS);
                    self.ea = self.seg_base(self.ea_seg).wrapping_add(self.eo32);
                }
                _ => unreachable!(),
            },
            1 => {
                let disp = self.fetch(bus) as i8 as u16;
                let seg = if rm == 2 || rm == 3 || rm == 6 {
                    SegReg32::SS
                } else {
                    SegReg32::DS
                };
                self.eo = self.ea_base(rm).wrapping_add(disp);
                self.eo32 = self.eo as u32;
                self.ea_seg = self.default_seg(seg);
                self.ea = self.seg_base(self.ea_seg).wrapping_add(self.eo32);
            }
            2 => {
                let disp = self.fetchword(bus);
                let seg = if rm == 2 || rm == 3 || rm == 6 {
                    SegReg32::SS
                } else {
                    SegReg32::DS
                };
                self.eo = self.ea_base(rm).wrapping_add(disp);
                self.eo32 = self.eo as u32;
                self.ea_seg = self.default_seg(seg);
                self.ea = self.seg_base(self.ea_seg).wrapping_add(self.eo32);
            }
            3 => {}
            _ => unreachable!(),
        }
    }

    fn calc_ea32(&mut self, modrm: u8, bus: &mut impl common::Bus) {
        let mode = modrm >> 6;
        let rm = modrm & 7;

        let mut offset = 0u32;
        let default_seg;

        if rm == 4 {
            let sib = self.fetch(bus);
            let scale = (sib >> 6) & 0x3;
            let index = (sib >> 3) & 0x7;
            let base = sib & 0x7;
            let scaled_base_no_index = index == 4 && scale != 0 && !(mode == 0 && base == 5);

            if index != 4 {
                offset = offset.wrapping_add(self.addr32_reg(index) << scale);
            } else if scaled_base_no_index {
                // Undefined 80386 SIB rows: with no index and scale > 1,
                // hardware scales the base register.
                offset = offset.wrapping_add(self.addr32_reg(base) << scale);
            }

            if mode == 0 && base == 5 {
                offset = offset.wrapping_add(self.fetchdword(bus));
                default_seg = SegReg32::DS;
            } else if scaled_base_no_index {
                default_seg = if base == 4 || base == 5 {
                    SegReg32::SS
                } else {
                    SegReg32::DS
                };
            } else {
                offset = offset.wrapping_add(self.addr32_reg(base));
                default_seg = if base == 4 || base == 5 {
                    SegReg32::SS
                } else {
                    SegReg32::DS
                };
            }
        } else if mode == 0 && rm == 5 {
            offset = self.fetchdword(bus);
            default_seg = SegReg32::DS;
        } else {
            offset = self.addr32_reg(rm);
            default_seg = if rm == 4 || rm == 5 {
                SegReg32::SS
            } else {
                SegReg32::DS
            };
        }

        match mode {
            0 => {}
            1 => {
                let disp = self.fetch(bus) as i8 as i32 as u32;
                offset = offset.wrapping_add(disp);
            }
            2 => {
                let disp = self.fetchdword(bus);
                offset = offset.wrapping_add(disp);
            }
            3 => return,
            _ => unreachable!(),
        }

        self.eo = offset as u16;
        self.eo32 = offset;
        self.ea_seg = self.default_seg(default_seg);
        self.ea = self.seg_base(self.ea_seg).wrapping_add(self.eo32);
    }

    #[inline(always)]
    fn ea_base(&self, rm: u8) -> u16 {
        match rm {
            0 => self
                .regs
                .word(WordReg::BX)
                .wrapping_add(self.regs.word(WordReg::SI)),
            1 => self
                .regs
                .word(WordReg::BX)
                .wrapping_add(self.regs.word(WordReg::DI)),
            2 => self
                .regs
                .word(WordReg::BP)
                .wrapping_add(self.regs.word(WordReg::SI)),
            3 => self
                .regs
                .word(WordReg::BP)
                .wrapping_add(self.regs.word(WordReg::DI)),
            4 => self.regs.word(WordReg::SI),
            5 => self.regs.word(WordReg::DI),
            6 => self.regs.word(WordReg::BP),
            7 => self.regs.word(WordReg::BX),
            _ => unreachable!(),
        }
    }

    #[inline(always)]
    fn addr32_reg(&self, index: u8) -> u32 {
        match index & 7 {
            0 => self.regs.dword(DwordReg::EAX),
            1 => self.regs.dword(DwordReg::ECX),
            2 => self.regs.dword(DwordReg::EDX),
            3 => self.regs.dword(DwordReg::EBX),
            4 => self.regs.dword(DwordReg::ESP),
            5 => self.regs.dword(DwordReg::EBP),
            6 => self.regs.dword(DwordReg::ESI),
            7 => self.regs.dword(DwordReg::EDI),
            _ => unreachable!(),
        }
    }

    pub(super) fn get_rm_byte(&mut self, modrm: u8, bus: &mut impl common::Bus) -> Step<u8> {
        if modrm >= 0xC0 {
            Ok(self.regs.byte(self.rm_byte(modrm)))
        } else {
            self.calc_ea(modrm, bus);
            self.check_segment_access(self.ea_seg, self.eo32, 1, false, bus)?;
            let addr = self.translate_linear(self.ea, false, bus)?;
            Ok(bus.read_byte(addr))
        }
    }

    pub(super) fn get_rm_byte_for_update(
        &mut self,
        modrm: u8,
        bus: &mut impl common::Bus,
    ) -> Step<u8> {
        if modrm >= 0xC0 {
            Ok(self.regs.byte(self.rm_byte(modrm)))
        } else {
            self.calc_ea(modrm, bus);
            self.check_segment_access(self.ea_seg, self.eo32, 1, true, bus)?;
            let addr = self.translate_linear(self.ea, true, bus)?;
            Ok(bus.read_byte(addr))
        }
    }

    pub(super) fn get_rm_word(&mut self, modrm: u8, bus: &mut impl common::Bus) -> Step<u16> {
        if modrm >= 0xC0 {
            Ok(self.regs.word(self.rm_word(modrm)))
        } else {
            self.calc_ea(modrm, bus);
            self.seg_read_word(bus)
        }
    }

    pub(super) fn get_rm_word_for_update(
        &mut self,
        modrm: u8,
        bus: &mut impl common::Bus,
    ) -> Step<u16> {
        if modrm >= 0xC0 {
            Ok(self.regs.word(self.rm_word(modrm)))
        } else {
            self.calc_ea(modrm, bus);
            self.seg_read_word_for_update(bus)
        }
    }

    pub(super) fn get_rm_dword(&mut self, modrm: u8, bus: &mut impl common::Bus) -> Step<u32> {
        if modrm >= 0xC0 {
            Ok(self.regs.dword(self.rm_dword(modrm)))
        } else {
            self.calc_ea(modrm, bus);
            self.seg_read_dword(bus)
        }
    }

    pub(super) fn get_rm_dword_for_update(
        &mut self,
        modrm: u8,
        bus: &mut impl common::Bus,
    ) -> Step<u32> {
        if modrm >= 0xC0 {
            Ok(self.regs.dword(self.rm_dword(modrm)))
        } else {
            self.calc_ea(modrm, bus);
            self.seg_read_dword_for_update(bus)
        }
    }

    pub(super) fn putback_rm_byte(
        &mut self,
        modrm: u8,
        value: u8,
        bus: &mut impl common::Bus,
    ) -> Step {
        if modrm >= 0xC0 {
            let reg = self.rm_byte(modrm);
            self.regs.set_byte(reg, value);
        } else {
            let addr = self.translate_linear(self.ea, true, bus)?;
            bus.write_byte(addr, value);
        }
        Ok(())
    }

    pub(super) fn putback_rm_word(
        &mut self,
        modrm: u8,
        value: u16,
        bus: &mut impl common::Bus,
    ) -> Step {
        if modrm >= 0xC0 {
            let reg = self.rm_word(modrm);
            self.regs.set_word(reg, value);
            Ok(())
        } else {
            self.seg_write_word(bus, value)
        }
    }

    pub(super) fn putback_rm_dword(
        &mut self,
        modrm: u8,
        value: u32,
        bus: &mut impl common::Bus,
    ) -> Step {
        if modrm >= 0xC0 {
            let reg = self.rm_dword(modrm);
            self.regs.set_dword(reg, value);
            Ok(())
        } else {
            self.seg_write_dword(bus, value)
        }
    }

    pub(super) fn put_rm_byte(&mut self, modrm: u8, value: u8, bus: &mut impl common::Bus) -> Step {
        if modrm >= 0xC0 {
            let reg = self.rm_byte(modrm);
            self.regs.set_byte(reg, value);
        } else {
            self.calc_ea(modrm, bus);
            let addr = self.translate_linear(self.ea, true, bus)?;
            bus.write_byte(addr, value);
        }
        Ok(())
    }

    pub(super) fn put_rm_word(
        &mut self,
        modrm: u8,
        value: u16,
        bus: &mut impl common::Bus,
    ) -> Step {
        if modrm >= 0xC0 {
            let reg = self.rm_word(modrm);
            self.regs.set_word(reg, value);
            Ok(())
        } else {
            self.calc_ea(modrm, bus);
            self.seg_write_word(bus, value)
        }
    }

    pub(super) fn put_rm_dword(
        &mut self,
        modrm: u8,
        value: u32,
        bus: &mut impl common::Bus,
    ) -> Step {
        if modrm >= 0xC0 {
            let reg = self.rm_dword(modrm);
            self.regs.set_dword(reg, value);
            Ok(())
        } else {
            self.calc_ea(modrm, bus);
            self.seg_write_dword(bus, value)
        }
    }
}
