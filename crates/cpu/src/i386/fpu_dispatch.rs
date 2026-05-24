use super::{I386, Step};

impl<const CPU_MODEL: u8> I386<CPU_MODEL> {
    pub(super) fn fpu_escape(&mut self, opcode: u8, bus: &mut impl common::Bus) -> Step {
        // CR0.EM=1 -> #NM
        if self.cr0 & 0x04 != 0 {
            return self.raise_fault(7, bus);
        }

        // CR0.TS=1 -> #NM
        if self.cr0 & 0x08 != 0 {
            return self.raise_fault(7, bus);
        }

        let modrm = self.fetch(bus);
        let has_memory = modrm < 0xC0;

        // ES=1 -> deliver pending exception before executing
        if self.fpu_es_pending() {
            return self.fpu_raise_exception(bus);
        }

        if has_memory {
            self.calc_ea(modrm, bus);
        }

        let esc_bits = opcode & 7;
        match esc_bits {
            0 => self.fpu_esc_d8(modrm, has_memory, bus)?,
            1 => self.fpu_esc_d9(modrm, has_memory, bus)?,
            2 => self.fpu_esc_da(modrm, has_memory, bus)?,
            3 => self.fpu_esc_db(modrm, has_memory, bus)?,
            4 => self.fpu_esc_dc(modrm, has_memory, bus)?,
            5 => self.fpu_esc_dd(modrm, has_memory, bus)?,
            6 => self.fpu_esc_de(modrm, has_memory, bus)?,
            7 => self.fpu_esc_df(modrm, has_memory, bus)?,
            _ => unreachable!(),
        }
        Ok(())
    }

    pub(super) fn fpu_wait(&mut self, bus: &mut impl common::Bus) -> Step {
        // CR0.MP=1 AND CR0.TS=1 -> #NM
        if self.cr0 & 0x02 != 0 && self.cr0 & 0x08 != 0 {
            return self.raise_fault(7, bus);
        }

        // ES=1 -> deliver pending exception
        if self.fpu_es_pending() {
            return self.fpu_raise_exception(bus);
        }

        self.clk(Self::timing(6, 1));
        Ok(())
    }

    // D8: Single-precision arithmetic
    fn fpu_esc_d8(&mut self, modrm: u8, has_memory: bool, bus: &mut impl common::Bus) -> Step {
        let reg = (modrm >> 3) & 7;
        self.fpu_update_pointers(0, modrm, has_memory);

        if has_memory {
            match reg {
                0 => self.fpu_fadd_m32(bus)?,
                1 => self.fpu_fmul_m32(bus)?,
                2 => self.fpu_fcom_m32(bus)?,
                3 => self.fpu_fcomp_m32(bus)?,
                4 => self.fpu_fsub_m32(bus)?,
                5 => self.fpu_fsubr_m32(bus)?,
                6 => self.fpu_fdiv_m32(bus)?,
                7 => self.fpu_fdivr_m32(bus)?,
                _ => unreachable!(),
            }
        } else {
            let i = modrm & 7;
            match reg {
                0 => self.fpu_fadd_st0_sti(i),
                1 => self.fpu_fmul_st0_sti(i),
                2 => self.fpu_fcom_sti(i),
                3 => self.fpu_fcomp_sti(i),
                4 => self.fpu_fsub_st0_sti(i),
                5 => self.fpu_fsubr_st0_sti(i),
                6 => self.fpu_fdiv_st0_sti(i),
                7 => self.fpu_fdivr_st0_sti(i),
                _ => unreachable!(),
            }
        }
        Ok(())
    }

    // D9: Load/Store, Transcendentals, Control
    fn fpu_esc_d9(&mut self, modrm: u8, has_memory: bool, bus: &mut impl common::Bus) -> Step {
        if has_memory {
            let reg = (modrm >> 3) & 7;
            // Control instructions don't update FIP/FDP: FLDENV(4), FLDCW(5), FSTENV(6), FSTCW(7)
            if reg < 4 {
                self.fpu_update_pointers(1, modrm, true);
            }
            match reg {
                0 => self.fpu_fld_m32(bus)?,
                1 => self.clk(Self::timing(2, 2)), // reserved
                2 => self.fpu_fst_m32(bus)?,
                3 => self.fpu_fstp_m32(bus)?,
                4 => self.fpu_fldenv(bus),
                5 => self.fpu_fldcw(bus)?,
                6 => self.fpu_fnstenv(bus),
                7 => self.fpu_fnstcw(bus),
                _ => unreachable!(),
            }
        } else {
            match modrm {
                0xC0..=0xC7 => {
                    self.fpu_update_pointers(1, modrm, false);
                    self.fpu_fld_sti(modrm & 7);
                }
                0xC8..=0xCF => {
                    self.fpu_update_pointers(1, modrm, false);
                    self.fpu_fxch(modrm & 7);
                }
                0xD0 => {
                    // FNOP - control instruction, no pointer update
                    self.fpu_fnop();
                }
                0xD8..=0xDF => {
                    self.fpu_update_pointers(1, modrm, false);
                    self.fpu_fstp_sti(modrm & 7);
                }
                0xE0 => {
                    self.fpu_update_pointers(1, modrm, false);
                    self.fpu_fchs();
                }
                0xE1 => {
                    self.fpu_update_pointers(1, modrm, false);
                    self.fpu_fabs();
                }
                0xE4 => {
                    self.fpu_update_pointers(1, modrm, false);
                    self.fpu_ftst();
                }
                0xE5 => {
                    self.fpu_update_pointers(1, modrm, false);
                    self.fpu_fxam();
                }
                0xE8 => {
                    self.fpu_update_pointers(1, modrm, false);
                    self.fpu_fld1();
                }
                0xE9 => {
                    self.fpu_update_pointers(1, modrm, false);
                    self.fpu_fldl2t();
                }
                0xEA => {
                    self.fpu_update_pointers(1, modrm, false);
                    self.fpu_fldl2e();
                }
                0xEB => {
                    self.fpu_update_pointers(1, modrm, false);
                    self.fpu_fldpi();
                }
                0xEC => {
                    self.fpu_update_pointers(1, modrm, false);
                    self.fpu_fldlg2();
                }
                0xED => {
                    self.fpu_update_pointers(1, modrm, false);
                    self.fpu_fldln2();
                }
                0xEE => {
                    self.fpu_update_pointers(1, modrm, false);
                    self.fpu_fldz();
                }
                0xF0 => {
                    self.fpu_update_pointers(1, modrm, false);
                    self.fpu_f2xm1();
                }
                0xF1 => {
                    self.fpu_update_pointers(1, modrm, false);
                    self.fpu_fyl2x();
                }
                0xF2 => {
                    self.fpu_update_pointers(1, modrm, false);
                    self.fpu_fptan();
                }
                0xF3 => {
                    self.fpu_update_pointers(1, modrm, false);
                    self.fpu_fpatan();
                }
                0xF4 => {
                    self.fpu_update_pointers(1, modrm, false);
                    self.fpu_fxtract();
                }
                0xF5 => {
                    self.fpu_update_pointers(1, modrm, false);
                    self.fpu_fprem1();
                }
                0xF6 => {
                    self.fpu_update_pointers(1, modrm, false);
                    self.fpu_fdecstp();
                }
                0xF7 => {
                    self.fpu_update_pointers(1, modrm, false);
                    self.fpu_fincstp();
                }
                0xF8 => {
                    self.fpu_update_pointers(1, modrm, false);
                    self.fpu_fprem();
                }
                0xF9 => {
                    self.fpu_update_pointers(1, modrm, false);
                    self.fpu_fyl2xp1();
                }
                0xFA => {
                    self.fpu_update_pointers(1, modrm, false);
                    self.fpu_fsqrt();
                }
                0xFB => {
                    self.fpu_update_pointers(1, modrm, false);
                    self.fpu_fsincos();
                }
                0xFC => {
                    self.fpu_update_pointers(1, modrm, false);
                    self.fpu_frndint();
                }
                0xFD => {
                    self.fpu_update_pointers(1, modrm, false);
                    self.fpu_fscale();
                }
                0xFE => {
                    self.fpu_update_pointers(1, modrm, false);
                    self.fpu_fsin();
                }
                0xFF => {
                    self.fpu_update_pointers(1, modrm, false);
                    self.fpu_fcos();
                }
                _ => {
                    // Undefined register encodings: treat as NOP
                    self.clk(Self::timing(2, 2));
                }
            }
        }
        Ok(())
    }

    // DA: 32-bit integer arithmetic
    fn fpu_esc_da(&mut self, modrm: u8, has_memory: bool, bus: &mut impl common::Bus) -> Step {
        let reg = (modrm >> 3) & 7;
        self.fpu_update_pointers(2, modrm, has_memory);

        if has_memory {
            match reg {
                0 => self.fpu_fiadd_m32(bus)?,
                1 => self.fpu_fimul_m32(bus)?,
                2 => self.fpu_ficom_m32(bus)?,
                3 => self.fpu_ficomp_m32(bus)?,
                4 => self.fpu_fisub_m32(bus)?,
                5 => self.fpu_fisubr_m32(bus)?,
                6 => self.fpu_fidiv_m32(bus)?,
                7 => self.fpu_fidivr_m32(bus)?,
                _ => unreachable!(),
            }
        } else {
            match modrm {
                0xE9 => self.fpu_fucompp(),
                _ => self.clk(Self::timing(2, 2)), // reserved
            }
        }
        Ok(())
    }

    // DB: 32-bit integer load/store, extended load/store
    fn fpu_esc_db(&mut self, modrm: u8, has_memory: bool, bus: &mut impl common::Bus) -> Step {
        if has_memory {
            let reg = (modrm >> 3) & 7;
            self.fpu_update_pointers(3, modrm, true);
            match reg {
                0 => self.fpu_fild_m32(bus)?,
                1 => self.clk(Self::timing(2, 2)), // reserved
                2 => self.fpu_fist_m32(bus)?,
                3 => self.fpu_fistp_m32(bus)?,
                4 => self.clk(Self::timing(2, 2)), // reserved
                5 => self.fpu_fld_m80(bus)?,
                6 => self.clk(Self::timing(2, 2)), // reserved
                7 => self.fpu_fstp_m80(bus),
                _ => unreachable!(),
            }
        } else {
            match modrm {
                // Control instructions - no pointer update
                0xE0 => self.fpu_fnop_legacy(), // FENI (8087 compat)
                0xE1 => self.fpu_fnop_legacy(), // FDISI (8087 compat)
                0xE2 => self.fpu_fnclex(),
                0xE3 => self.fpu_fninit(),
                0xE4 => self.fpu_fnop_legacy(), // FSETPM (80287 compat)
                _ => self.clk(Self::timing(2, 2)), // reserved
            }
        }
        Ok(())
    }

    // DC: Double-precision arithmetic
    fn fpu_esc_dc(&mut self, modrm: u8, has_memory: bool, bus: &mut impl common::Bus) -> Step {
        let reg = (modrm >> 3) & 7;
        self.fpu_update_pointers(4, modrm, has_memory);

        if has_memory {
            match reg {
                0 => self.fpu_fadd_m64(bus)?,
                1 => self.fpu_fmul_m64(bus)?,
                2 => self.fpu_fcom_m64(bus)?,
                3 => self.fpu_fcomp_m64(bus)?,
                4 => self.fpu_fsub_m64(bus)?,
                5 => self.fpu_fsubr_m64(bus)?,
                6 => self.fpu_fdiv_m64(bus)?,
                7 => self.fpu_fdivr_m64(bus)?,
                _ => unreachable!(),
            }
        } else {
            let i = modrm & 7;
            // Note: DC register form has swapped FSUB/FSUBR and FDIV/FDIVR encodings
            match reg {
                0 => self.fpu_fadd_sti_st0(i),
                1 => self.fpu_fmul_sti_st0(i),
                4 => self.fpu_fsubr_sti_st0(i), // DC E0+i = FSUBR ST(i), ST(0)
                5 => self.fpu_fsub_sti_st0(i),  // DC E8+i = FSUB ST(i), ST(0)
                6 => self.fpu_fdivr_sti_st0(i), // DC F0+i = FDIVR ST(i), ST(0)
                7 => self.fpu_fdiv_sti_st0(i),  // DC F8+i = FDIV ST(i), ST(0)
                _ => self.clk(Self::timing(2, 2)), // reserved (2,3 in register form)
            }
        }
        Ok(())
    }

    // DD: Double-precision memory, stack management
    fn fpu_esc_dd(&mut self, modrm: u8, has_memory: bool, bus: &mut impl common::Bus) -> Step {
        if has_memory {
            let reg = (modrm >> 3) & 7;
            // FRSTOR(4), FSAVE(6), FSTSW(7) are control-ish, but FRSTOR/FSAVE do update
            // We update pointers for data transfer ones (0-3), not for control (4-7)
            if reg < 4 {
                self.fpu_update_pointers(5, modrm, true);
            }
            match reg {
                0 => self.fpu_fld_m64(bus)?,
                1 => self.clk(Self::timing(2, 2)), // reserved
                2 => self.fpu_fst_m64(bus)?,
                3 => self.fpu_fstp_m64(bus)?,
                4 => self.fpu_frstor(bus)?,
                5 => self.clk(Self::timing(2, 2)), // reserved
                6 => self.fpu_fnsave(bus)?,
                7 => self.fpu_fnstsw_m16(bus),
                _ => unreachable!(),
            }
        } else {
            let i = modrm & 7;
            match (modrm >> 3) & 7 {
                0 => {
                    // FFREE - control, no pointer update
                    self.fpu_ffree(i);
                }
                1 => {
                    self.fpu_update_pointers(5, modrm, false);
                    self.fpu_fxch(i); // DD C8+i alias
                }
                2 => {
                    self.fpu_update_pointers(5, modrm, false);
                    self.fpu_fst_sti(i);
                }
                3 => {
                    self.fpu_update_pointers(5, modrm, false);
                    self.fpu_fstp_sti(i);
                }
                4 => {
                    self.fpu_update_pointers(5, modrm, false);
                    self.fpu_fucom_sti(i);
                }
                5 => {
                    self.fpu_update_pointers(5, modrm, false);
                    self.fpu_fucomp_sti(i);
                }
                _ => self.clk(Self::timing(2, 2)), // reserved
            }
        }
        Ok(())
    }

    // DE: 16-bit integer arithmetic, pop variants
    fn fpu_esc_de(&mut self, modrm: u8, has_memory: bool, bus: &mut impl common::Bus) -> Step {
        let reg = (modrm >> 3) & 7;
        self.fpu_update_pointers(6, modrm, has_memory);

        if has_memory {
            match reg {
                0 => self.fpu_fiadd_m16(bus)?,
                1 => self.fpu_fimul_m16(bus)?,
                2 => self.fpu_ficom_m16(bus)?,
                3 => self.fpu_ficomp_m16(bus)?,
                4 => self.fpu_fisub_m16(bus)?,
                5 => self.fpu_fisubr_m16(bus)?,
                6 => self.fpu_fidiv_m16(bus)?,
                7 => self.fpu_fidivr_m16(bus)?,
                _ => unreachable!(),
            }
        } else {
            let i = modrm & 7;
            // Note: DE register form has swapped FSUB/FSUBR and FDIV/FDIVR
            match reg {
                0 => self.fpu_faddp_sti_st0(i),
                1 => self.fpu_fmulp_sti_st0(i),
                2 => self.clk(Self::timing(2, 2)), // reserved
                3 => {
                    if modrm == 0xD9 {
                        self.fpu_fcompp();
                    } else {
                        self.clk(Self::timing(2, 2)); // reserved
                    }
                }
                4 => self.fpu_fsubrp_sti_st0(i), // DE E0+i = FSUBRP
                5 => self.fpu_fsubp_sti_st0(i),  // DE E8+i = FSUBP
                6 => self.fpu_fdivrp_sti_st0(i), // DE F0+i = FDIVRP
                7 => self.fpu_fdivp_sti_st0(i),  // DE F8+i = FDIVP
                _ => self.clk(Self::timing(2, 2)), // reserved
            }
        }
        Ok(())
    }

    // DF: 16/64-bit integer, BCD, status word
    fn fpu_esc_df(&mut self, modrm: u8, has_memory: bool, bus: &mut impl common::Bus) -> Step {
        if has_memory {
            let reg = (modrm >> 3) & 7;
            self.fpu_update_pointers(7, modrm, true);
            match reg {
                0 => self.fpu_fild_m16(bus)?,
                1 => self.clk(Self::timing(2, 2)), // reserved
                2 => self.fpu_fist_m16(bus)?,
                3 => self.fpu_fistp_m16(bus)?,
                4 => self.fpu_fbld(bus)?,
                5 => self.fpu_fild_m64(bus)?,
                6 => self.fpu_fbstp(bus),
                7 => self.fpu_fistp_m64(bus),
                _ => unreachable!(),
            }
        } else {
            match modrm {
                // FSTSW AX - control instruction, no pointer update
                0xE0 => self.fpu_fnstsw_ax(),
                _ => self.clk(Self::timing(2, 2)), // reserved
            }
        }
        Ok(())
    }
}
