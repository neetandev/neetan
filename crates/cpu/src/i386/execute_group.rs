use super::{CPU_MODEL_386_DX, CPU_MODEL_386_SX, CPU_MODEL_486_DX, I386, Step};
use crate::{ByteReg, DwordReg, SegReg32, WordReg};

impl<const CPU_MODEL: u8, const ADDRESS_WIDTH: u8> I386<CPU_MODEL, ADDRESS_WIDTH> {
    /// Returns (register_cycles, memory_cycles) for shift/rotate group instructions.
    /// `count_source`: 0 = imm8 (C0/C1), 1 = literal 1 (D0/D1), 2 = CL (D2/D3).
    #[inline(always)]
    fn shift_group_timing(extension: u8, count_source: u8) -> (i32, i32) {
        match CPU_MODEL {
            CPU_MODEL_386_DX | CPU_MODEL_386_SX => match extension & 7 {
                2 | 3 => (9, 10),
                _ => (3, 7),
            },
            CPU_MODEL_486_DX => match count_source {
                0 => match extension & 7 {
                    2 | 3 => (8, 9),
                    _ => (2, 4),
                },
                1 => (3, 4),
                _ => match extension & 7 {
                    2 | 3 => (8, 9),
                    _ => (3, 4),
                },
            },
            _ => {
                unreachable!("Unhandled CPU_MODEL")
            }
        }
    }

    /// Base clocks for a far indirect JMP (opcode FF /5). Only protected mode
    /// pays the descriptor-load cost; real and V86 mode load CS directly and are
    /// much cheaper, consistent with the far CALL (FF /3) timing. The datasheet's
    /// flat 43 is the protected-mode figure; the 386EX real-mode traces show the
    /// cheaper cost. Applies to the 386DX and 386SX alike (shared execution
    /// timing); the SX bus penalty for the pointer fetch is charged separately.
    #[inline(always)]
    fn far_jmp_indirect_base(&self) -> i32 {
        if self.is_protected_mode() && !self.is_virtual_mode() {
            43
        } else {
            18
        }
    }

    /// Group 0x80: ALU r/m8, imm8
    pub(super) fn group_80(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        let extension = (modrm >> 3) & 7;
        if self.lock_prefix && extension == 7 {
            self.raise_fault(6, bus)?;
            return Ok(());
        }
        let dst = (if extension == 7 {
            self.get_rm_byte(modrm, bus)
        } else {
            self.get_rm_byte_for_update(modrm, bus)
        })?;
        let src = self.fetch(bus);
        let result = match extension {
            0 => self.alu_add_byte(dst, src),
            1 => self.alu_or_byte(dst, src),
            2 => {
                let cf = self.flags.cf_val();
                self.alu_adc_byte(dst, src, cf)
            }
            3 => {
                let cf = self.flags.cf_val();
                self.alu_sbb_byte(dst, src, cf)
            }
            4 => self.alu_and_byte(dst, src),
            5 => self.alu_sub_byte(dst, src),
            6 => self.alu_xor_byte(dst, src),
            7 => {
                self.alu_sub_byte(dst, src);
                self.clk_modrm(modrm, Self::timing(2, 1), Self::timing(5, 2));
                return Ok(());
            }
            _ => unreachable!(),
        };
        if extension != 7 {
            self.putback_rm_byte(modrm, result, bus)?;
        }
        self.clk_modrm(modrm, Self::timing(2, 1), Self::timing(7, 3));
        Ok(())
    }

    /// Group 0x81: ALU r/m16, imm16
    pub(super) fn group_81(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        let extension = (modrm >> 3) & 7;
        if self.lock_prefix && extension == 7 {
            self.raise_fault(6, bus)?;
            return Ok(());
        }
        if self.operand_size_override {
            let dst = (if extension == 7 {
                self.get_rm_dword(modrm, bus)
            } else {
                self.get_rm_dword_for_update(modrm, bus)
            })?;
            let src = self.fetchdword(bus);
            let result = match extension {
                0 => self.alu_add_dword(dst, src),
                1 => self.alu_or_dword(dst, src),
                2 => {
                    let cf = self.flags.cf_val();
                    self.alu_adc_dword(dst, src, cf)
                }
                3 => {
                    let cf = self.flags.cf_val();
                    self.alu_sbb_dword(dst, src, cf)
                }
                4 => self.alu_and_dword(dst, src),
                5 => self.alu_sub_dword(dst, src),
                6 => self.alu_xor_dword(dst, src),
                7 => {
                    self.alu_sub_dword(dst, src);
                    self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(5, 2), 2);
                    return Ok(());
                }
                _ => unreachable!(),
            };
            if extension != 7 {
                self.putback_rm_dword(modrm, result, bus)?;
            }
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(7, 3), 4);
        } else {
            let dst = (if extension == 7 {
                self.get_rm_word(modrm, bus)
            } else {
                self.get_rm_word_for_update(modrm, bus)
            })?;
            let src = self.fetchword(bus);
            let result = match extension {
                0 => self.alu_add_word(dst, src),
                1 => self.alu_or_word(dst, src),
                2 => {
                    let cf = self.flags.cf_val();
                    self.alu_adc_word(dst, src, cf)
                }
                3 => {
                    let cf = self.flags.cf_val();
                    self.alu_sbb_word(dst, src, cf)
                }
                4 => self.alu_and_word(dst, src),
                5 => self.alu_sub_word(dst, src),
                6 => self.alu_xor_word(dst, src),
                7 => {
                    self.alu_sub_word(dst, src);
                    self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(5, 2), 1);
                    return Ok(());
                }
                _ => unreachable!(),
            };
            if extension != 7 {
                self.putback_rm_word(modrm, result, bus)?;
            }
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(7, 3), 2);
        }
        Ok(())
    }

    /// Group 0x82: ALU r/m8, imm8 (same as 0x80)
    pub(super) fn group_82(&mut self, bus: &mut impl common::Bus) -> Step {
        self.group_80(bus)?;
        Ok(())
    }

    /// Group 0x83: ALU r/m16, sign-extended imm8
    pub(super) fn group_83(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        let extension = (modrm >> 3) & 7;
        if self.lock_prefix && extension == 7 {
            self.raise_fault(6, bus)?;
            return Ok(());
        }
        if self.operand_size_override {
            let dst = (if extension == 7 {
                self.get_rm_dword(modrm, bus)
            } else {
                self.get_rm_dword_for_update(modrm, bus)
            })?;
            let src = self.fetch(bus) as i8 as i32 as u32;
            let result = match extension {
                0 => self.alu_add_dword(dst, src),
                1 => self.alu_or_dword(dst, src),
                2 => {
                    let cf = self.flags.cf_val();
                    self.alu_adc_dword(dst, src, cf)
                }
                3 => {
                    let cf = self.flags.cf_val();
                    self.alu_sbb_dword(dst, src, cf)
                }
                4 => self.alu_and_dword(dst, src),
                5 => self.alu_sub_dword(dst, src),
                6 => self.alu_xor_dword(dst, src),
                7 => {
                    self.alu_sub_dword(dst, src);
                    self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(5, 2), 2);
                    return Ok(());
                }
                _ => unreachable!(),
            };
            if extension != 7 {
                self.putback_rm_dword(modrm, result, bus)?;
            }
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(7, 3), 4);
        } else {
            let dst = (if extension == 7 {
                self.get_rm_word(modrm, bus)
            } else {
                self.get_rm_word_for_update(modrm, bus)
            })?;
            let src = self.fetch(bus) as i8 as u16;
            let result = match extension {
                0 => self.alu_add_word(dst, src),
                1 => self.alu_or_word(dst, src),
                2 => {
                    let cf = self.flags.cf_val();
                    self.alu_adc_word(dst, src, cf)
                }
                3 => {
                    let cf = self.flags.cf_val();
                    self.alu_sbb_word(dst, src, cf)
                }
                4 => self.alu_and_word(dst, src),
                5 => self.alu_sub_word(dst, src),
                6 => self.alu_xor_word(dst, src),
                7 => {
                    self.alu_sub_word(dst, src);
                    self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(5, 2), 1);
                    return Ok(());
                }
                _ => unreachable!(),
            };
            if extension != 7 {
                self.putback_rm_word(modrm, result, bus)?;
            }
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(7, 3), 2);
        }
        Ok(())
    }

    /// Group 0xC0: shift/rotate r/m8, imm8
    pub(super) fn group_c0(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        let dst = self.get_rm_byte(modrm, bus)?;
        let count = self.fetch(bus);
        let extension = (modrm >> 3) & 7;
        let result = match extension {
            0 => self.alu_rol_byte(dst, count),
            1 => self.alu_ror_byte(dst, count),
            2 => self.alu_rcl_byte(dst, count),
            3 => self.alu_rcr_byte(dst, count),
            4 => self.alu_shl_byte(dst, count),
            5 => self.alu_shr_byte(dst, count),
            6 => self.alu_shl_byte(dst, count), // undocumented: same as SHL
            7 => self.alu_sar_byte(dst, count),
            _ => unreachable!(),
        };
        self.putback_rm_byte(modrm, result, bus)?;
        let (register_cycles, memory_cycles) = Self::shift_group_timing(extension, 0);
        self.clk_modrm(modrm, register_cycles, memory_cycles);
        Ok(())
    }

    /// Group 0xC1: shift/rotate r/m16, imm8
    pub(super) fn group_c1(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        let extension = (modrm >> 3) & 7;
        let (register_cycles, memory_cycles) = Self::shift_group_timing(extension, 0);
        if self.operand_size_override {
            let dst = self.get_rm_dword(modrm, bus)?;
            let count = self.fetch(bus);
            let result = match extension {
                0 => self.alu_rol_dword(dst, count),
                1 => self.alu_ror_dword(dst, count),
                2 => self.alu_rcl_dword(dst, count),
                3 => self.alu_rcr_dword(dst, count),
                4 => self.alu_shl_dword(dst, count),
                5 => self.alu_shr_dword(dst, count),
                6 => self.alu_shl_dword(dst, count),
                7 => self.alu_sar_dword(dst, count),
                _ => unreachable!(),
            };
            self.putback_rm_dword(modrm, result, bus)?;
            self.clk_modrm_word(modrm, register_cycles, memory_cycles, 4);
        } else {
            let dst = self.get_rm_word(modrm, bus)?;
            let count = self.fetch(bus);
            let result = match extension {
                0 => self.alu_rol_word(dst, count),
                1 => self.alu_ror_word(dst, count),
                2 => self.alu_rcl_word(dst, count),
                3 => self.alu_rcr_word(dst, count),
                4 => self.alu_shl_word(dst, count),
                5 => self.alu_shr_word(dst, count),
                6 => self.alu_shl_word(dst, count),
                7 => self.alu_sar_word(dst, count),
                _ => unreachable!(),
            };
            self.putback_rm_word(modrm, result, bus)?;
            self.clk_modrm_word(modrm, register_cycles, memory_cycles, 2);
        }
        Ok(())
    }

    /// Group 0xD0: shift/rotate r/m8, 1
    pub(super) fn group_d0(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        let dst = self.get_rm_byte(modrm, bus)?;
        let extension = (modrm >> 3) & 7;
        let result = match extension {
            0 => self.alu_rol_byte(dst, 1),
            1 => self.alu_ror_byte(dst, 1),
            2 => self.alu_rcl_byte(dst, 1),
            3 => self.alu_rcr_byte(dst, 1),
            4 => self.alu_shl_byte(dst, 1),
            5 => self.alu_shr_byte(dst, 1),
            6 => self.alu_shl_byte(dst, 1),
            7 => self.alu_sar_byte(dst, 1),
            _ => unreachable!(),
        };
        self.putback_rm_byte(modrm, result, bus)?;
        let (register_cycles, memory_cycles) = Self::shift_group_timing(extension, 1);
        self.clk_modrm(modrm, register_cycles, memory_cycles);
        Ok(())
    }

    /// Group 0xD1: shift/rotate r/m16, 1
    pub(super) fn group_d1(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        let extension = (modrm >> 3) & 7;
        let (register_cycles, memory_cycles) = Self::shift_group_timing(extension, 1);
        if self.operand_size_override {
            let dst = self.get_rm_dword(modrm, bus)?;
            let result = match extension {
                0 => self.alu_rol_dword(dst, 1),
                1 => self.alu_ror_dword(dst, 1),
                2 => self.alu_rcl_dword(dst, 1),
                3 => self.alu_rcr_dword(dst, 1),
                4 => self.alu_shl_dword(dst, 1),
                5 => self.alu_shr_dword(dst, 1),
                6 => self.alu_shl_dword(dst, 1),
                7 => self.alu_sar_dword(dst, 1),
                _ => unreachable!(),
            };
            self.putback_rm_dword(modrm, result, bus)?;
            self.clk_modrm_word(modrm, register_cycles, memory_cycles, 4);
        } else {
            let dst = self.get_rm_word(modrm, bus)?;
            let result = match extension {
                0 => self.alu_rol_word(dst, 1),
                1 => self.alu_ror_word(dst, 1),
                2 => self.alu_rcl_word(dst, 1),
                3 => self.alu_rcr_word(dst, 1),
                4 => self.alu_shl_word(dst, 1),
                5 => self.alu_shr_word(dst, 1),
                6 => self.alu_shl_word(dst, 1),
                7 => self.alu_sar_word(dst, 1),
                _ => unreachable!(),
            };
            self.putback_rm_word(modrm, result, bus)?;
            self.clk_modrm_word(modrm, register_cycles, memory_cycles, 2);
        }
        Ok(())
    }

    /// Group 0xD2: shift/rotate r/m8, CL
    pub(super) fn group_d2(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        let dst = self.get_rm_byte(modrm, bus)?;
        let count = self.regs.byte(ByteReg::CL);
        let extension = (modrm >> 3) & 7;
        let result = match extension {
            0 => self.alu_rol_byte(dst, count),
            1 => self.alu_ror_byte(dst, count),
            2 => self.alu_rcl_byte(dst, count),
            3 => self.alu_rcr_byte(dst, count),
            4 => self.alu_shl_byte(dst, count),
            5 => self.alu_shr_byte(dst, count),
            6 => self.alu_shl_byte(dst, count),
            7 => self.alu_sar_byte(dst, count),
            _ => unreachable!(),
        };
        self.putback_rm_byte(modrm, result, bus)?;
        let (register_cycles, memory_cycles) = Self::shift_group_timing(extension, 2);
        self.clk_modrm(modrm, register_cycles, memory_cycles);
        Ok(())
    }

    /// Group 0xD3: shift/rotate r/m16, CL
    pub(super) fn group_d3(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        let count = self.regs.byte(ByteReg::CL);
        let extension = (modrm >> 3) & 7;
        let (register_cycles, memory_cycles) = Self::shift_group_timing(extension, 2);
        if self.operand_size_override {
            let dst = self.get_rm_dword(modrm, bus)?;
            let result = match extension {
                0 => self.alu_rol_dword(dst, count),
                1 => self.alu_ror_dword(dst, count),
                2 => self.alu_rcl_dword(dst, count),
                3 => self.alu_rcr_dword(dst, count),
                4 => self.alu_shl_dword(dst, count),
                5 => self.alu_shr_dword(dst, count),
                6 => self.alu_shl_dword(dst, count),
                7 => self.alu_sar_dword(dst, count),
                _ => unreachable!(),
            };
            self.putback_rm_dword(modrm, result, bus)?;
            self.clk_modrm_word(modrm, register_cycles, memory_cycles, 4);
        } else {
            let dst = self.get_rm_word(modrm, bus)?;
            let result = match extension {
                0 => self.alu_rol_word(dst, count),
                1 => self.alu_ror_word(dst, count),
                2 => self.alu_rcl_word(dst, count),
                3 => self.alu_rcr_word(dst, count),
                4 => self.alu_shl_word(dst, count),
                5 => self.alu_shr_word(dst, count),
                6 => self.alu_shl_word(dst, count),
                7 => self.alu_sar_word(dst, count),
                _ => unreachable!(),
            };
            self.putback_rm_word(modrm, result, bus)?;
            self.clk_modrm_word(modrm, register_cycles, memory_cycles, 2);
        }
        Ok(())
    }

    /// Group 0xF6: various byte operations
    pub(super) fn group_f6(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        let op = (modrm >> 3) & 7;
        if self.lock_prefix && !matches!(op, 2 | 3) {
            self.raise_fault(6, bus)?;
            return Ok(());
        }
        match op {
            0 | 1 => {
                // TEST r/m8, imm8
                let dst = self.get_rm_byte(modrm, bus)?;
                let src = self.fetch(bus);
                self.alu_and_byte(dst, src);
                self.clk_modrm(modrm, Self::timing(2, 1), Self::timing(5, 2));
            }
            2 => {
                // NOT r/m8
                let dst = self.get_rm_byte(modrm, bus)?;
                self.putback_rm_byte(modrm, !dst, bus)?;
                self.clk_modrm(modrm, Self::timing(2, 1), Self::timing(6, 3));
            }
            3 => {
                // NEG r/m8
                let dst = self.get_rm_byte(modrm, bus)?;
                let result = self.alu_neg_byte(dst);
                self.putback_rm_byte(modrm, result, bus)?;
                self.clk_modrm(modrm, Self::timing(2, 1), Self::timing(6, 3));
            }
            4 => {
                // MUL r/m8 (unsigned)
                let src = self.get_rm_byte(modrm, bus)?;
                let al = self.regs.byte(ByteReg::AL);
                let result = al as u16 * src as u16;
                self.regs.set_word(WordReg::AX, result);
                self.flags.carry_val = if result & 0xFF00 != 0 { 1 } else { 0 };
                self.flags.overflow_val = self.flags.carry_val;
                self.clk_modrm(modrm, Self::timing(14, 13), Self::timing(17, 13));
            }
            5 => {
                // IMUL r/m8 (signed)
                let src = self.get_rm_byte(modrm, bus)?;
                let src = src as i8 as i16;
                let al = self.regs.byte(ByteReg::AL) as i8 as i16;
                let result = al * src;
                self.regs.set_word(WordReg::AX, result as u16);
                let ah = (result >> 8) as i8;
                let al_sign = result as i8;
                self.flags.carry_val = if ah != (al_sign >> 7) { 1 } else { 0 };
                self.flags.overflow_val = self.flags.carry_val;
                self.clk_modrm(modrm, Self::timing(14, 13), Self::timing(17, 13));
            }
            6 => {
                // DIV r/m8 (unsigned)
                let src = self.get_rm_byte(modrm, bus)?;
                let src = src as u16;
                if src == 0 {
                    self.raise_fault(0, bus)?;
                    return Ok(());
                }
                let aw = self.regs.word(WordReg::AX);
                let quotient = aw / src;
                if quotient > 0xFF {
                    self.raise_fault(0, bus)?;
                    return Ok(());
                }
                let remainder = aw % src;
                self.regs.set_byte(ByteReg::AL, quotient as u8);
                self.regs.set_byte(ByteReg::AH, remainder as u8);
                // DIV leaves condition flags undefined; do not preserve prior CF state.
                self.flags.carry_val = u32::from(!self.flags.cf());
                // Toggle AF after division (hardware quirk found by NP21W).
                self.flags.aux_val ^= 0x10;
                self.clk_modrm(modrm, Self::timing(14, 16), Self::timing(17, 16));
            }
            7 => {
                // IDIV r/m8 (signed)
                let src = self.get_rm_byte(modrm, bus)?;
                let src = src as i8 as i16;
                if src == 0 {
                    self.raise_fault(0, bus)?;
                    return Ok(());
                }
                let aw = self.regs.word(WordReg::AX) as i16;
                let Some(quotient) = aw.checked_div(src) else {
                    self.raise_fault(0, bus)?;
                    return Ok(());
                };
                if !(-128..=127).contains(&quotient) {
                    self.raise_fault(0, bus)?;
                    return Ok(());
                }
                let remainder = aw.checked_rem(src).unwrap_or(0);
                self.regs.set_byte(ByteReg::AL, quotient as u8);
                self.regs.set_byte(ByteReg::AH, remainder as u8);
                // IDIV leaves condition flags undefined; do not preserve prior CF state.
                self.flags.carry_val = u32::from(!self.flags.cf());
                // Toggle AF after division (hardware quirk found by NP21W).
                self.flags.aux_val ^= 0x10;
                self.clk_modrm(modrm, Self::timing(19, 19), Self::timing(19, 20));
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    /// Group 0xF7: various word operations
    pub(super) fn group_f7(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        let op = (modrm >> 3) & 7;
        if self.lock_prefix && !matches!(op, 2 | 3) {
            self.raise_fault(6, bus)?;
            return Ok(());
        }
        if self.operand_size_override {
            match op {
                0 | 1 => {
                    // TEST r/m32, imm32
                    let dst = self.get_rm_dword(modrm, bus)?;
                    let src = self.fetchdword(bus);
                    self.alu_and_dword(dst, src);
                    self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(5, 2), 2);
                }
                2 => {
                    // NOT r/m32
                    let dst = self.get_rm_dword(modrm, bus)?;
                    self.putback_rm_dword(modrm, !dst, bus)?;
                    self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(6, 3), 4);
                }
                3 => {
                    // NEG r/m32
                    let dst = self.get_rm_dword(modrm, bus)?;
                    let result = self.alu_neg_dword(dst);
                    self.putback_rm_dword(modrm, result, bus)?;
                    self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(6, 3), 4);
                }
                4 => {
                    // MUL r/m32 (unsigned)
                    let src = self.get_rm_dword(modrm, bus)?;
                    let eax = self.regs.dword(DwordReg::EAX);
                    let result = eax as u64 * src as u64;
                    self.regs.set_dword(DwordReg::EAX, result as u32);
                    self.regs.set_dword(DwordReg::EDX, (result >> 32) as u32);
                    self.flags.carry_val = u32::from((result >> 32) != 0);
                    self.flags.overflow_val = self.flags.carry_val;
                    self.clk_modrm_word(modrm, Self::timing(38, 13), Self::timing(41, 13), 2);
                }
                5 => {
                    // IMUL r/m32 (signed)
                    let src = self.get_rm_dword(modrm, bus)?;
                    let src = src as i32 as i64;
                    let eax = self.regs.dword(DwordReg::EAX) as i32 as i64;
                    let result = eax * src;
                    self.regs.set_dword(DwordReg::EAX, result as u32);
                    self.regs.set_dword(DwordReg::EDX, (result >> 32) as u32);
                    let lower_sign_extended = (result as i32) as i64;
                    self.flags.carry_val = u32::from(result != lower_sign_extended);
                    self.flags.overflow_val = self.flags.carry_val;
                    self.clk_modrm_word(modrm, Self::timing(38, 13), Self::timing(41, 13), 2);
                }
                6 => {
                    // DIV r/m32 (unsigned)
                    let src = self.get_rm_dword(modrm, bus)?;
                    let src = src as u64;
                    if src == 0 {
                        self.raise_fault(0, bus)?;
                        return Ok(());
                    }
                    let edx = self.regs.dword(DwordReg::EDX) as u64;
                    let eax = self.regs.dword(DwordReg::EAX) as u64;
                    let dividend = (edx << 32) | eax;
                    let quotient = dividend / src;
                    if quotient > u32::MAX as u64 {
                        self.raise_fault(0, bus)?;
                        return Ok(());
                    }
                    let remainder = dividend % src;
                    self.regs.set_dword(DwordReg::EAX, quotient as u32);
                    self.regs.set_dword(DwordReg::EDX, remainder as u32);
                    // DIV leaves condition flags undefined; do not preserve prior CF state.
                    self.flags.carry_val = u32::from(!self.flags.cf());
                    // Toggle AF after division (hardware quirk found by NP21W).
                    self.flags.aux_val ^= 0x10;
                    self.clk_modrm_word(modrm, Self::timing(38, 40), Self::timing(41, 40), 2);
                }
                7 => {
                    // IDIV r/m32 (signed)
                    let src = self.get_rm_dword(modrm, bus)?;
                    let src = src as i32 as i64;
                    if src == 0 {
                        self.raise_fault(0, bus)?;
                        return Ok(());
                    }
                    let edx = self.regs.dword(DwordReg::EDX) as u64;
                    let eax = self.regs.dword(DwordReg::EAX) as u64;
                    let dividend = ((edx << 32) | eax) as i64;
                    let Some(quotient) = dividend.checked_div(src) else {
                        self.raise_fault(0, bus)?;
                        return Ok(());
                    };
                    if !((i32::MIN as i64)..=(i32::MAX as i64)).contains(&quotient) {
                        self.raise_fault(0, bus)?;
                        return Ok(());
                    }
                    let remainder = dividend.checked_rem(src).unwrap_or(0);
                    self.regs.set_dword(DwordReg::EAX, quotient as u32);
                    self.regs.set_dword(DwordReg::EDX, remainder as u32);
                    // IDIV leaves condition flags undefined; do not preserve prior CF state.
                    self.flags.carry_val = u32::from(!self.flags.cf());
                    // Toggle AF after division (hardware quirk found by NP21W).
                    self.flags.aux_val ^= 0x10;
                    self.clk_modrm_word(modrm, Self::timing(43, 43), Self::timing(43, 44), 2);
                }
                _ => unreachable!(),
            }
        } else {
            match op {
                0 | 1 => {
                    // TEST r/m16, imm16
                    let dst = self.get_rm_word(modrm, bus)?;
                    let src = self.fetchword(bus);
                    self.alu_and_word(dst, src);
                    self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(5, 2), 1);
                }
                2 => {
                    // NOT r/m16
                    let dst = self.get_rm_word(modrm, bus)?;
                    self.putback_rm_word(modrm, !dst, bus)?;
                    self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(6, 3), 2);
                }
                3 => {
                    // NEG r/m16
                    let dst = self.get_rm_word(modrm, bus)?;
                    let result = self.alu_neg_word(dst);
                    self.putback_rm_word(modrm, result, bus)?;
                    self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(6, 3), 2);
                }
                4 => {
                    // MUL r/m16 (unsigned)
                    let src = self.get_rm_word(modrm, bus)?;
                    let aw = self.regs.word(WordReg::AX);
                    let result = aw as u32 * src as u32;
                    self.regs.set_word(WordReg::AX, result as u16);
                    self.regs.set_word(WordReg::DX, (result >> 16) as u16);
                    self.flags.carry_val = if result & 0xFFFF0000 != 0 { 1 } else { 0 };
                    self.flags.overflow_val = self.flags.carry_val;
                    self.clk_modrm_word(modrm, Self::timing(22, 13), Self::timing(25, 13), 1);
                }
                5 => {
                    // IMUL r/m16 (signed)
                    let src = self.get_rm_word(modrm, bus)?;
                    let src = src as i16 as i32;
                    let aw = self.regs.word(WordReg::AX) as i16 as i32;
                    let result = aw * src;
                    self.regs.set_word(WordReg::AX, result as u16);
                    self.regs.set_word(WordReg::DX, (result >> 16) as u16);
                    let upper = (result >> 16) as i16;
                    let lower_sign = result as i16;
                    self.flags.carry_val = if upper != (lower_sign >> 15) { 1 } else { 0 };
                    self.flags.overflow_val = self.flags.carry_val;
                    self.clk_modrm_word(modrm, Self::timing(22, 13), Self::timing(25, 13), 1);
                }
                6 => {
                    // DIV r/m16 (unsigned)
                    let src = self.get_rm_word(modrm, bus)?;
                    let src = src as u32;
                    if src == 0 {
                        self.raise_fault(0, bus)?;
                        return Ok(());
                    }
                    let dw = self.regs.word(WordReg::DX) as u32;
                    let aw = self.regs.word(WordReg::AX) as u32;
                    let dividend = (dw << 16) | aw;
                    let quotient = dividend / src;
                    if quotient > 0xFFFF {
                        self.raise_fault(0, bus)?;
                        return Ok(());
                    }
                    let remainder = dividend % src;
                    self.regs.set_word(WordReg::AX, quotient as u16);
                    self.regs.set_word(WordReg::DX, remainder as u16);
                    // DIV leaves condition flags undefined; do not preserve prior CF state.
                    self.flags.carry_val = u32::from(!self.flags.cf());
                    // Toggle AF after division (hardware quirk found by NP21W).
                    self.flags.aux_val ^= 0x10;
                    self.clk_modrm_word(modrm, Self::timing(22, 24), Self::timing(25, 24), 1);
                }
                7 => {
                    // IDIV r/m16 (signed)
                    let src = self.get_rm_word(modrm, bus)?;
                    let src = src as i16 as i32;
                    if src == 0 {
                        self.raise_fault(0, bus)?;
                        return Ok(());
                    }
                    let dw = self.regs.word(WordReg::DX) as u32;
                    let aw = self.regs.word(WordReg::AX) as u32;
                    let dividend = ((dw << 16) | aw) as i32;
                    let Some(quotient) = dividend.checked_div(src) else {
                        self.raise_fault(0, bus)?;
                        return Ok(());
                    };
                    if !(-32768..=32767).contains(&quotient) {
                        self.raise_fault(0, bus)?;
                        return Ok(());
                    }
                    let remainder = dividend.checked_rem(src).unwrap_or(0);
                    self.regs.set_word(WordReg::AX, quotient as u16);
                    self.regs.set_word(WordReg::DX, remainder as u16);
                    // IDIV leaves condition flags undefined; do not preserve prior CF state.
                    self.flags.carry_val = u32::from(!self.flags.cf());
                    // Toggle AF after division (hardware quirk found by NP21W).
                    self.flags.aux_val ^= 0x10;
                    self.clk_modrm_word(modrm, Self::timing(27, 27), Self::timing(27, 28), 1);
                }
                _ => unreachable!(),
            }
        }
        Ok(())
    }

    /// Group 0xFE: INC/DEC r/m8
    pub(super) fn group_fe(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        if self.lock_prefix && (modrm >> 3) & 7 >= 2 {
            self.raise_fault(6, bus)?;
            return Ok(());
        }
        match (modrm >> 3) & 7 {
            0 => {
                // INC r/m8
                let dst = self.get_rm_byte_for_update(modrm, bus)?;
                let result = self.alu_inc_byte(dst);
                self.putback_rm_byte(modrm, result, bus)?;
                self.clk_modrm(modrm, Self::timing(2, 1), Self::timing(6, 3));
            }
            1 => {
                // DEC r/m8
                let dst = self.get_rm_byte_for_update(modrm, bus)?;
                let result = self.alu_dec_byte(dst);
                self.putback_rm_byte(modrm, result, bus)?;
                self.clk_modrm(modrm, Self::timing(2, 1), Self::timing(6, 3));
            }
            _ => {
                self.clk(Self::timing(2, 1));
            }
        }
        Ok(())
    }

    /// Group 0xFF: various word operations
    pub(super) fn group_ff(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        if self.lock_prefix && (modrm >> 3) & 7 >= 2 {
            self.raise_fault(6, bus)?;
            return Ok(());
        }
        match (modrm >> 3) & 7 {
            0 => {
                if self.operand_size_override {
                    // INC r/m32
                    let dst = self.get_rm_dword_for_update(modrm, bus)?;
                    let result = self.alu_inc_dword(dst);
                    self.putback_rm_dword(modrm, result, bus)?;
                    self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(6, 3), 4);
                } else {
                    // INC r/m16
                    let dst = self.get_rm_word_for_update(modrm, bus)?;
                    let result = self.alu_inc_word(dst);
                    self.putback_rm_word(modrm, result, bus)?;
                    self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(6, 3), 2);
                }
            }
            1 => {
                if self.operand_size_override {
                    // DEC r/m32
                    let dst = self.get_rm_dword_for_update(modrm, bus)?;
                    let result = self.alu_dec_dword(dst);
                    self.putback_rm_dword(modrm, result, bus)?;
                    self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(6, 3), 4);
                } else {
                    // DEC r/m16
                    let dst = self.get_rm_word_for_update(modrm, bus)?;
                    let result = self.alu_dec_word(dst);
                    self.putback_rm_word(modrm, result, bus)?;
                    self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(6, 3), 2);
                }
            }
            2 => {
                if self.operand_size_override {
                    // CALL r/m32 (near indirect)
                    let sp_pen = self.sp_penalty();
                    let dst = self.get_rm_dword(modrm, bus)?;
                    let return_eip = self.ip_upper | self.ip as u32;
                    self.push_dword(bus, return_eip)?;
                    self.ip = dst as u16;
                    self.ip_upper = dst & 0xFFFF_0000;
                    match CPU_MODEL {
                        CPU_MODEL_386_DX | CPU_MODEL_386_SX => {
                            let m = self.next_instruction_length_approx(bus);
                            if modrm >= 0xC0 {
                                self.clk(7 + m + sp_pen);
                            } else {
                                let ea_pen = if self.ea & 3 != 0 { 4 } else { 0 };
                                self.clk(10 + m + sp_pen + ea_pen);
                            }
                        }
                        CPU_MODEL_486_DX => self.clk(5 + sp_pen),
                        _ => {
                            unreachable!("Unhandled CPU_MODEL")
                        }
                    }
                } else {
                    // CALL r/m16 (near indirect)
                    let sp_pen = self.sp_penalty();
                    let dst = self.get_rm_word(modrm, bus)?;
                    self.push(bus, self.ip)?;
                    self.ip = dst;
                    self.ip_upper = 0;
                    match CPU_MODEL {
                        CPU_MODEL_386_DX | CPU_MODEL_386_SX => {
                            let m = self.next_instruction_length_approx(bus);
                            if modrm >= 0xC0 {
                                self.clk(7 + m + sp_pen);
                            } else {
                                let ea_pen = if self.ea & 1 != 0 { 4 } else { 0 };
                                self.clk(10 + m + sp_pen + ea_pen);
                            }
                        }
                        CPU_MODEL_486_DX => self.clk(5 + sp_pen),
                        _ => {
                            unreachable!("Unhandled CPU_MODEL")
                        }
                    }
                }
            }
            3 => {
                if modrm >= 0xC0 {
                    return Ok(());
                }
                if self.operand_size_override {
                    // CALL m16:32 (far indirect)
                    let sp_pen = self.sp_penalty();
                    self.calc_ea(modrm, bus);
                    let offset = self.seg_read_dword(bus)?;
                    let segment = self.seg_read_word_at(bus, 4)?;
                    let cs = self.sregs[SegReg32::CS as usize];
                    let old_eip = self.ip_upper | self.ip as u32;
                    if !self.is_protected_mode() || self.is_virtual_mode() {
                        self.far_push_real_v86_dword(bus, cs as u32, old_eip)?;
                        self.load_segment(SegReg32::CS, segment, bus)?;
                        self.ip = offset as u16;
                        self.ip_upper = offset & 0xFFFF_0000;
                    } else {
                        self.code_descriptor(
                            segment,
                            offset,
                            super::TaskType::Call,
                            cs,
                            old_eip,
                            bus,
                        )?;
                    }
                    match CPU_MODEL {
                        CPU_MODEL_386_DX | CPU_MODEL_386_SX => {
                            let m = self.next_instruction_length_approx(bus);
                            let ea_pen = if self.ea & 3 != 0 { 4 } else { 0 };
                            self.clk(22 + m + sp_pen + ea_pen);
                        }
                        CPU_MODEL_486_DX => self.clk(17 + sp_pen),
                        _ => {
                            unreachable!("Unhandled CPU_MODEL")
                        }
                    }
                } else {
                    // CALL m16:16 (far indirect)
                    let sp_pen = self.sp_penalty();
                    self.calc_ea(modrm, bus);
                    let offset = self.seg_read_word(bus)?;
                    let segment = self.seg_read_word_at(bus, 2)?;
                    let cs = self.sregs[SegReg32::CS as usize];
                    let old_eip = self.ip_upper | self.ip as u32;
                    if !self.is_protected_mode() || self.is_virtual_mode() {
                        self.far_push_real_v86_word(bus, cs, old_eip as u16)?;
                        self.load_segment(SegReg32::CS, segment, bus)?;
                        self.ip = offset;
                        self.ip_upper = 0;
                    } else {
                        self.code_descriptor(
                            segment,
                            offset as u32,
                            super::TaskType::Call,
                            cs,
                            old_eip,
                            bus,
                        )?;
                    }
                    match CPU_MODEL {
                        CPU_MODEL_386_DX | CPU_MODEL_386_SX => {
                            let m = self.next_instruction_length_approx(bus);
                            let ea_pen = if self.ea & 1 != 0 { 4 } else { 0 };
                            self.clk(22 + m + sp_pen + ea_pen);
                        }
                        CPU_MODEL_486_DX => self.clk(17 + sp_pen),
                        _ => {
                            unreachable!("Unhandled CPU_MODEL")
                        }
                    }
                }
            }
            4 => {
                if self.operand_size_override {
                    // JMP r/m32 (near indirect)
                    let dst = self.get_rm_dword(modrm, bus)?;
                    self.ip = dst as u16;
                    self.ip_upper = dst & 0xFFFF_0000;
                    match CPU_MODEL {
                        CPU_MODEL_386_DX | CPU_MODEL_386_SX => {
                            let m = self.next_instruction_length_approx(bus);
                            if modrm >= 0xC0 {
                                self.clk(7 + m);
                            } else {
                                let ea_pen = if self.ea & 3 != 0 { 4 } else { 0 };
                                self.clk(10 + m + ea_pen);
                            }
                        }
                        CPU_MODEL_486_DX => self.clk(5),
                        _ => {
                            unreachable!("Unhandled CPU_MODEL")
                        }
                    }
                } else {
                    // JMP r/m16 (near indirect)
                    let dst = self.get_rm_word(modrm, bus)?;
                    self.ip = dst;
                    self.ip_upper = 0;
                    match CPU_MODEL {
                        CPU_MODEL_386_DX | CPU_MODEL_386_SX => {
                            let m = self.next_instruction_length_approx(bus);
                            if modrm >= 0xC0 {
                                self.clk(7 + m);
                            } else {
                                let ea_pen = if self.ea & 1 != 0 { 4 } else { 0 };
                                self.clk(10 + m + ea_pen);
                            }
                        }
                        CPU_MODEL_486_DX => self.clk(5),
                        _ => {
                            unreachable!("Unhandled CPU_MODEL")
                        }
                    }
                }
            }
            5 => {
                if modrm >= 0xC0 {
                    return Ok(());
                }
                if self.operand_size_override {
                    // JMP m16:32 (far indirect)
                    self.calc_ea(modrm, bus);
                    let offset = self.seg_read_dword(bus)?;
                    let segment = self.seg_read_word_at(bus, 4)?;
                    if !self.is_protected_mode() || self.is_virtual_mode() {
                        self.load_segment(SegReg32::CS, segment, bus)?;
                        self.ip = offset as u16;
                        self.ip_upper = offset & 0xFFFF_0000;
                    } else {
                        self.code_descriptor(segment, offset, super::TaskType::Jmp, 0, 0, bus)?;
                    }
                    match CPU_MODEL {
                        CPU_MODEL_386_DX | CPU_MODEL_386_SX => {
                            let m = self.next_instruction_length_approx(bus);
                            let penalty = if self.ea & 3 != 0 { 4 } else { 0 };
                            self.clk(self.far_jmp_indirect_base() + m + penalty);
                        }
                        CPU_MODEL_486_DX => self.clk(13),
                        _ => {
                            unreachable!("Unhandled CPU_MODEL")
                        }
                    }
                } else {
                    // JMP m16:16 (far indirect)
                    self.calc_ea(modrm, bus);
                    let offset = self.seg_read_word(bus)?;
                    let segment = self.seg_read_word_at(bus, 2)?;
                    if !self.is_protected_mode() || self.is_virtual_mode() {
                        self.load_segment(SegReg32::CS, segment, bus)?;
                        self.ip = offset;
                        self.ip_upper = 0;
                    } else {
                        self.code_descriptor(
                            segment,
                            offset as u32,
                            super::TaskType::Jmp,
                            0,
                            0,
                            bus,
                        )?;
                    }
                    match CPU_MODEL {
                        CPU_MODEL_386_DX | CPU_MODEL_386_SX => {
                            let m = self.next_instruction_length_approx(bus);
                            let penalty = if self.ea & 1 != 0 { 4 } else { 0 };
                            self.clk(self.far_jmp_indirect_base() + m + penalty);
                        }
                        CPU_MODEL_486_DX => self.clk(13),
                        _ => {
                            unreachable!("Unhandled CPU_MODEL")
                        }
                    }
                }
            }
            6 | 7 => {
                if self.operand_size_override {
                    // PUSH r/m32 (7 is undocumented alias)
                    let sp_pen = self.sp_penalty();
                    let val = self.get_rm_dword(modrm, bus)?;
                    self.push_dword(bus, val)?;
                    if modrm >= 0xC0 {
                        self.clk(Self::timing(5, 1) + sp_pen);
                    } else {
                        let ea_pen = if self.ea & 3 != 0 {
                            Self::misaligned_operand_penalty()
                        } else {
                            0
                        };
                        self.clk(Self::timing(5, 4) + sp_pen + ea_pen);
                    }
                } else {
                    // PUSH r/m16 (7 is undocumented alias)
                    let sp_pen = self.sp_penalty();
                    let val = self.get_rm_word(modrm, bus)?;
                    self.push(bus, val)?;
                    if modrm >= 0xC0 {
                        self.clk(Self::timing(5, 1) + sp_pen);
                    } else {
                        let ea_pen = if self.ea & 1 != 0 {
                            Self::misaligned_operand_penalty()
                        } else {
                            0
                        };
                        self.clk(Self::timing(5, 4) + sp_pen + ea_pen);
                    }
                }
            }
            _ => {
                self.clk(Self::timing(2, 1));
            }
        }
        Ok(())
    }
}
