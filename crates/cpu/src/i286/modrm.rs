use super::{ADDRESS_MASK, I286, timing::I286DemandPrefetchPolicy};
use crate::{ByteReg, SegReg16, WordReg, build_x86_reg_word_table, build_x86_rm_table};

static MODRM_REG: [u8; 256] = build_x86_reg_word_table();
static MODRM_RM: [u8; 256] = build_x86_rm_table();

/// Addressing-mode shape of the current ModR/M byte, captured once per
/// instruction so the timing model can charge AU cycles without re-parsing
/// the byte in every opcode body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EaClass {
    /// Register operand (mode 3), no effective address.
    #[default]
    Register,
    /// Direct memory operand (mode 0, rm 6), single disp16.
    Direct,
    /// Memory operand with a single-register base (SI, DI, BX), mode 0.
    SingleRegister,
    /// Memory operand with a two-register base (BX+SI, BX+DI, BP+SI, BP+DI),
    /// mode 0.
    DoubleRegister,
    /// Memory operand with a single-register base plus disp8, mode 1.
    Disp8Single,
    /// Memory operand with a two-register base plus disp8, mode 1.
    Disp8Double,
    /// Memory operand with a single-register base plus disp16, mode 2.
    Disp16Single,
    /// Memory operand with a two-register base plus disp16, mode 2.
    Disp16Double,
}

state_enum_codec!(EaClass {
    EaClass::Register = 0,
    EaClass::Direct = 1,
    EaClass::SingleRegister = 2,
    EaClass::DoubleRegister = 3,
    EaClass::Disp8Single = 4,
    EaClass::Disp8Double = 5,
    EaClass::Disp16Single = 6,
    EaClass::Disp16Double = 7,
});

impl EaClass {
    /// Classifies the addressing mode encoded by the ModR/M byte.
    #[inline(always)]
    pub const fn from_modrm(modrm: u8) -> Self {
        if modrm_is_register(modrm) {
            return EaClass::Register;
        }
        let mode = modrm_mode(modrm);
        let rm = modrm_rm(modrm);
        match (mode, rm) {
            (0, 6) => EaClass::Direct,
            (0, 4) | (0, 5) | (0, 7) => EaClass::SingleRegister,
            (0, 0) | (0, 1) | (0, 2) | (0, 3) => EaClass::DoubleRegister,
            (1, 4) | (1, 5) | (1, 6) | (1, 7) => EaClass::Disp8Single,
            (1, 0) | (1, 1) | (1, 2) | (1, 3) => EaClass::Disp8Double,
            (2, 4) | (2, 5) | (2, 6) | (2, 7) => EaClass::Disp16Single,
            (2, 0) | (2, 1) | (2, 2) | (2, 3) => EaClass::Disp16Double,
            _ => unreachable!(),
        }
    }

    /// Returns true for register-register ModR/M forms.
    #[inline(always)]
    pub const fn is_register(self) -> bool {
        matches!(self, EaClass::Register)
    }

    /// Returns true for memory ModR/M forms.
    #[inline(always)]
    pub const fn is_memory(self) -> bool {
        !self.is_register()
    }

    /// Returns true for memory addressing without displacement bytes.
    #[inline(always)]
    pub const fn is_no_displacement_memory(self) -> bool {
        matches!(self, EaClass::SingleRegister | EaClass::DoubleRegister)
    }

    /// Returns true for memory addressing with an 8-bit displacement.
    #[inline(always)]
    pub const fn is_disp8(self) -> bool {
        matches!(self, EaClass::Disp8Single | EaClass::Disp8Double)
    }

    /// Returns true for memory addressing with a 16-bit displacement.
    #[inline(always)]
    pub const fn is_disp16(self) -> bool {
        matches!(self, EaClass::Disp16Single | EaClass::Disp16Double)
    }

    /// Returns true for memory addressing using one base register.
    #[inline(always)]
    pub const fn has_single_register_base(self) -> bool {
        matches!(
            self,
            EaClass::SingleRegister | EaClass::Disp8Single | EaClass::Disp16Single
        )
    }

    /// Returns true for memory addressing using a two-register base sum.
    #[inline(always)]
    pub const fn has_double_register_base(self) -> bool {
        matches!(
            self,
            EaClass::DoubleRegister | EaClass::Disp8Double | EaClass::Disp16Double
        )
    }
}

#[inline(always)]
pub(super) const fn modrm_mode(modrm: u8) -> u8 {
    modrm >> 6
}

#[inline(always)]
pub(super) const fn modrm_rm(modrm: u8) -> u8 {
    modrm & 7
}

#[inline(always)]
pub(super) const fn modrm_register(modrm: u8) -> u8 {
    (modrm >> 3) & 7
}

#[inline(always)]
pub(super) const fn modrm_is_register(modrm: u8) -> bool {
    modrm >= 0xC0
}

#[inline(always)]
pub(super) const fn modrm_is_memory(modrm: u8) -> bool {
    !modrm_is_register(modrm)
}

#[inline(always)]
pub(super) const fn modrm_is_direct_memory(modrm: u8) -> bool {
    modrm_mode(modrm) == 0 && modrm_rm(modrm) == 6
}

#[inline(always)]
pub(super) const fn modrm_is_no_displacement_memory(modrm: u8) -> bool {
    modrm_mode(modrm) == 0 && modrm_rm(modrm) != 6
}

#[inline(always)]
pub(super) const fn modrm_is_disp8_memory(modrm: u8) -> bool {
    modrm_mode(modrm) == 1
}

#[inline(always)]
pub(super) const fn modrm_is_disp16_memory(modrm: u8) -> bool {
    modrm_mode(modrm) == 2
}

#[inline(always)]
pub(super) const fn modrm_uses_double_register_base(modrm: u8) -> bool {
    modrm_rm(modrm) <= 3
}

/// Returns the incremental AU cycles contributed by the effective-address
/// shape alone, independent of the opcode's base EU cost. Register forms
/// and direct/single-register memory forms need no extra AU cycles;
/// two-register forms carry a one-cycle AU penalty because the 286 adds a
/// second register value to the base.
#[inline(always)]
pub(super) const fn ea_class_au_cycles(class: EaClass) -> i32 {
    match class {
        EaClass::Register => 0,
        EaClass::Direct => 0,
        EaClass::SingleRegister => 0,
        EaClass::DoubleRegister => 1,
        EaClass::Disp8Single => 0,
        EaClass::Disp8Double => 1,
        EaClass::Disp16Single => 0,
        EaClass::Disp16Double => 1,
    }
}

impl I286 {
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
    pub(super) fn rm_byte(&self, modrm: u8) -> ByteReg {
        ByteReg::from_index(MODRM_RM[modrm as usize])
    }

    pub(super) fn calc_ea(&mut self, modrm: u8, bus: &mut impl common::Bus) {
        self.ea_class = EaClass::from_modrm(modrm);
        let mode = modrm_mode(modrm);
        let rm = modrm_rm(modrm);
        self.state.timing.note_au_calculation();

        match mode {
            0 => match rm {
                0 => {
                    self.eo = self
                        .regs
                        .word(WordReg::BX)
                        .wrapping_add(self.regs.word(WordReg::SI));
                    self.ea =
                        self.default_base(SegReg16::DS).wrapping_add(self.eo as u32) & ADDRESS_MASK;
                    self.ea_seg = self.default_seg(SegReg16::DS);
                }
                1 => {
                    self.eo = self
                        .regs
                        .word(WordReg::BX)
                        .wrapping_add(self.regs.word(WordReg::DI));
                    self.ea =
                        self.default_base(SegReg16::DS).wrapping_add(self.eo as u32) & ADDRESS_MASK;
                    self.ea_seg = self.default_seg(SegReg16::DS);
                }
                2 => {
                    self.eo = self
                        .regs
                        .word(WordReg::BP)
                        .wrapping_add(self.regs.word(WordReg::SI));
                    self.ea =
                        self.default_base(SegReg16::SS).wrapping_add(self.eo as u32) & ADDRESS_MASK;
                    self.ea_seg = self.default_seg(SegReg16::SS);
                }
                3 => {
                    self.eo = self
                        .regs
                        .word(WordReg::BP)
                        .wrapping_add(self.regs.word(WordReg::DI));
                    self.ea =
                        self.default_base(SegReg16::SS).wrapping_add(self.eo as u32) & ADDRESS_MASK;
                    self.ea_seg = self.default_seg(SegReg16::SS);
                }
                4 => {
                    self.eo = self.regs.word(WordReg::SI);
                    self.ea =
                        self.default_base(SegReg16::DS).wrapping_add(self.eo as u32) & ADDRESS_MASK;
                    self.ea_seg = self.default_seg(SegReg16::DS);
                }
                5 => {
                    self.eo = self.regs.word(WordReg::DI);
                    self.ea =
                        self.default_base(SegReg16::DS).wrapping_add(self.eo as u32) & ADDRESS_MASK;
                    self.ea_seg = self.default_seg(SegReg16::DS);
                }
                6 => {
                    self.state.timing.note_au_displacement();
                    self.state
                        .timing
                        .note_demand_prefetch_policy(I286DemandPrefetchPolicy::BeforeTurnaround);
                    self.eo = self.fetchword(bus);
                    self.ea =
                        self.default_base(SegReg16::DS).wrapping_add(self.eo as u32) & ADDRESS_MASK;
                    self.ea_seg = self.default_seg(SegReg16::DS);
                }
                7 => {
                    self.eo = self.regs.word(WordReg::BX);
                    self.ea =
                        self.default_base(SegReg16::DS).wrapping_add(self.eo as u32) & ADDRESS_MASK;
                    self.ea_seg = self.default_seg(SegReg16::DS);
                }
                _ => unreachable!(),
            },
            1 => {
                self.state.timing.note_au_displacement();
                self.state
                    .timing
                    .note_demand_prefetch_policy(I286DemandPrefetchPolicy::AfterTurnaround);
                let disp = self.fetch(bus) as i8 as u16;
                let seg = if rm == 2 || rm == 3 || rm == 6 {
                    SegReg16::SS
                } else {
                    SegReg16::DS
                };
                self.eo = self.ea_base(rm).wrapping_add(disp);
                self.ea = self.default_base(seg).wrapping_add(self.eo as u32) & ADDRESS_MASK;
                self.ea_seg = self.default_seg(seg);
                if rm <= 3 {
                    self.state.timing.note_au_demand_cycles(1);
                }
            }
            2 => {
                self.state.timing.note_au_displacement();
                self.state
                    .timing
                    .note_demand_prefetch_policy(I286DemandPrefetchPolicy::BeforeTurnaround);
                let disp = self.fetchword(bus);
                let seg = if rm == 2 || rm == 3 || rm == 6 {
                    SegReg16::SS
                } else {
                    SegReg16::DS
                };
                self.eo = self.ea_base(rm).wrapping_add(disp);
                self.ea = self.default_base(seg).wrapping_add(self.eo as u32) & ADDRESS_MASK;
                self.ea_seg = self.default_seg(seg);
                if rm <= 3 {
                    self.state.timing.note_au_demand_cycles(1);
                }
            }
            _ => unreachable!(),
        }

        if mode == 0 && rm != 6 {
            self.state
                .timing
                .note_demand_prefetch_policy(I286DemandPrefetchPolicy::None);
        }

        self.state.timing.note_au_ready();
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

    pub(super) fn get_rm_byte(&mut self, modrm: u8, bus: &mut impl common::Bus) -> u8 {
        if modrm_is_register(modrm) {
            self.regs.byte(self.rm_byte(modrm))
        } else {
            self.calc_ea(modrm, bus);
            let value = self.seg_read_byte_at(bus, 0);
            self.state.timing.note_au_idle();
            value
        }
    }

    pub(super) fn get_rm_word(&mut self, modrm: u8, bus: &mut impl common::Bus) -> u16 {
        if modrm_is_register(modrm) {
            self.regs.word(self.rm_word(modrm))
        } else {
            self.calc_ea(modrm, bus);
            let value = self.seg_read_word(bus);
            self.state.timing.note_au_idle();
            value
        }
    }

    pub(super) fn putback_rm_byte(&mut self, modrm: u8, value: u8, bus: &mut impl common::Bus) {
        if modrm_is_register(modrm) {
            let reg = self.rm_byte(modrm);
            self.regs.set_byte(reg, value);
        } else {
            self.seg_write_byte_at(bus, 0, value);
            self.state.timing.note_au_idle();
        }
    }

    pub(super) fn putback_rm_word(&mut self, modrm: u8, value: u16, bus: &mut impl common::Bus) {
        if modrm_is_register(modrm) {
            let reg = self.rm_word(modrm);
            self.regs.set_word(reg, value);
        } else {
            self.seg_write_word(bus, value);
            self.state.timing.note_au_idle();
        }
    }

    pub(super) fn put_rm_byte(&mut self, modrm: u8, value: u8, bus: &mut impl common::Bus) {
        if modrm_is_register(modrm) {
            let reg = self.rm_byte(modrm);
            self.regs.set_byte(reg, value);
        } else {
            self.calc_ea(modrm, bus);
            self.seg_write_byte_at(bus, 0, value);
            self.state.timing.note_au_idle();
        }
    }

    pub(super) fn put_rm_word(&mut self, modrm: u8, value: u16, bus: &mut impl common::Bus) {
        if modrm_is_register(modrm) {
            let reg = self.rm_word(modrm);
            self.regs.set_word(reg, value);
        } else {
            self.calc_ea(modrm, bus);
            self.seg_write_word(bus, value);
            self.state.timing.note_au_idle();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ea_class_covers_all_modrm_forms() {
        let mut seen_register = false;
        let mut seen_direct = false;
        let mut seen_single_register = false;
        let mut seen_double_register = false;
        let mut seen_disp8_single = false;
        let mut seen_disp8_double = false;
        let mut seen_disp16_single = false;
        let mut seen_disp16_double = false;

        for byte in 0u16..=255u16 {
            let modrm = byte as u8;
            let class = EaClass::from_modrm(modrm);
            if modrm_is_register(modrm) {
                assert_eq!(class, EaClass::Register, "modrm=0x{modrm:02X}");
            } else {
                assert_ne!(class, EaClass::Register, "modrm=0x{modrm:02X}");
            }
            match class {
                EaClass::Register => seen_register = true,
                EaClass::Direct => seen_direct = true,
                EaClass::SingleRegister => seen_single_register = true,
                EaClass::DoubleRegister => seen_double_register = true,
                EaClass::Disp8Single => seen_disp8_single = true,
                EaClass::Disp8Double => seen_disp8_double = true,
                EaClass::Disp16Single => seen_disp16_single = true,
                EaClass::Disp16Double => seen_disp16_double = true,
            }
        }

        assert!(seen_register);
        assert!(seen_direct);
        assert!(seen_single_register);
        assert!(seen_double_register);
        assert!(seen_disp8_single);
        assert!(seen_disp8_double);
        assert!(seen_disp16_single);
        assert!(seen_disp16_double);
    }
}
