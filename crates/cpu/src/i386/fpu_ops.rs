use softfloat::{ExceptionFlags, Fp80, FpOrdering, Precision, RoundingMode};

use super::{I386, Step};

impl<const CPU_MODEL: u8, const ADDRESS_WIDTH: u8> I386<CPU_MODEL, ADDRESS_WIDTH> {
    pub(super) fn fpu_fninit(&mut self) {
        self.fpu_init();
        self.clk(Self::timing(33, 17));
    }

    pub(super) fn fpu_fnclex(&mut self) {
        self.state.fpu.status_word &= !0x80FF; // clear bits 0-7 and bit 15
        self.clk(Self::timing(11, 7));
    }

    pub(super) fn fpu_fldcw(&mut self, bus: &mut impl common::Bus) -> Step {
        let cw = self.fpu_read_u16(bus)?;
        self.state.fpu.control_word = cw;
        self.fpu_update_es();
        self.clk(Self::timing(19, 4));
        Ok(())
    }

    pub(super) fn fpu_fnstcw(&mut self, bus: &mut impl common::Bus) {
        let cw = self.state.fpu.control_word;
        if self.fpu_write_u16(bus, cw).is_err() {
            return;
        }
        self.clk(Self::timing(15, 3));
    }

    pub(super) fn fpu_fnstsw_m16(&mut self, bus: &mut impl common::Bus) {
        let sw = self.state.fpu.status_word;
        if self.fpu_write_u16(bus, sw).is_err() {
            return;
        }
        self.clk(Self::timing(15, 3));
    }

    pub(super) fn fpu_fnstsw_ax(&mut self) {
        let sw = self.state.fpu.status_word;
        self.regs.set_word(crate::WordReg::AX, sw);
        self.clk(Self::timing(13, 3));
    }

    pub(super) fn fpu_fnop(&mut self) {
        self.clk(Self::timing(12, 3));
    }

    pub(super) fn fpu_fnop_legacy(&mut self) {
        self.clk(Self::timing(12, 3));
    }

    pub(super) fn fpu_fincstp(&mut self) {
        let top = ((self.state.fpu.status_word >> 11) & 7) as u8;
        let new_top = (top.wrapping_add(1)) & 7;
        self.state.fpu.status_word =
            (self.state.fpu.status_word & !0x3800) | ((new_top as u16) << 11);
        self.state.fpu.status_word &= !0x0200; // C1=0
        self.clk(Self::timing(21, 3));
    }

    pub(super) fn fpu_fdecstp(&mut self) {
        let top = ((self.state.fpu.status_word >> 11) & 7) as u8;
        let new_top = (top.wrapping_sub(1)) & 7;
        self.state.fpu.status_word =
            (self.state.fpu.status_word & !0x3800) | ((new_top as u16) << 11);
        self.state.fpu.status_word &= !0x0200; // C1=0
        self.clk(Self::timing(22, 3));
    }

    pub(super) fn fpu_ffree(&mut self, i: u8) {
        let phys = self.fpu_st_phys(i);
        self.fpu_set_tag(phys, 0b11); // Empty
        self.clk(Self::timing(18, 3));
    }

    pub(super) fn fpu_fld_m32(&mut self, bus: &mut impl common::Bus) -> Step {
        let raw = self.fpu_read_u32(bus)?;
        let val = f32::from_bits(raw);
        let mut ef = ExceptionFlags::default();
        let fp = Fp80::from_f32(val, &mut ef);
        self.fpu_check_result(&ef);
        self.fpu_push(fp);
        self.clk(Self::timing(20, 3));
        Ok(())
    }

    pub(super) fn fpu_fld_m64(&mut self, bus: &mut impl common::Bus) -> Step {
        let raw = self.fpu_read_u64(bus)?;
        let val = f64::from_bits(raw);
        let mut ef = ExceptionFlags::default();
        let fp = Fp80::from_f64(val, &mut ef);
        self.fpu_check_result(&ef);
        self.fpu_push(fp);
        self.clk(Self::timing(25, 3));
        Ok(())
    }

    pub(super) fn fpu_fld_m80(&mut self, bus: &mut impl common::Bus) -> Step {
        let bytes = self.fpu_read_tbyte(bus)?;
        let fp = Fp80::from_le_bytes(bytes);
        self.fpu_push(fp);
        self.clk(Self::timing(44, 6));
        Ok(())
    }

    pub(super) fn fpu_fld_sti(&mut self, i: u8) {
        if self.fpu_check_underflow(i) {
            let masked = self.state.fpu.control_word & 1 != 0;
            if masked {
                self.fpu_push(Fp80::INDEFINITE);
            }
            self.clk(Self::timing(14, 4));
            return;
        }
        let val = self.fpu_st(i);
        self.fpu_push(val);
        self.clk(Self::timing(14, 4));
    }

    pub(super) fn fpu_fst_m32(&mut self, bus: &mut impl common::Bus) -> Step {
        if self.fpu_check_underflow(0) {
            let masked = self.state.fpu.control_word & 1 != 0;
            if masked {
                let bits = Fp80::INDEFINITE
                    .to_f32(self.fpu_rounding_mode(), &mut ExceptionFlags::default());
                self.fpu_write_u32(bus, bits.to_bits())?;
            }
            self.clk(Self::timing(44, 7));
            return Ok(());
        }
        let val = self.fpu_st(0);
        let mut ef = ExceptionFlags::default();
        let f = val.to_f32(self.fpu_rounding_mode(), &mut ef);
        self.fpu_check_result(&ef);
        self.fpu_write_u32(bus, f.to_bits())?;
        self.clk(Self::timing(44, 7));
        Ok(())
    }

    pub(super) fn fpu_fst_m64(&mut self, bus: &mut impl common::Bus) -> Step {
        if self.fpu_check_underflow(0) {
            let masked = self.state.fpu.control_word & 1 != 0;
            if masked {
                let bits = Fp80::INDEFINITE
                    .to_f64(self.fpu_rounding_mode(), &mut ExceptionFlags::default());
                self.fpu_write_u64(bus, bits.to_bits())?;
            }
            self.clk(Self::timing(45, 8));
            return Ok(());
        }
        let val = self.fpu_st(0);
        let mut ef = ExceptionFlags::default();
        let f = val.to_f64(self.fpu_rounding_mode(), &mut ef);
        self.fpu_check_result(&ef);
        self.fpu_write_u64(bus, f.to_bits())?;
        self.clk(Self::timing(45, 8));
        Ok(())
    }

    pub(super) fn fpu_fst_sti(&mut self, i: u8) {
        if self.fpu_check_underflow(0) {
            let masked = self.state.fpu.control_word & 1 != 0;
            if masked {
                self.fpu_write_st(i, Fp80::INDEFINITE);
            }
            self.clk(Self::timing(11, 3));
            return;
        }
        let val = self.fpu_st(0);
        self.fpu_write_st(i, val);
        self.clk(Self::timing(11, 3));
    }

    pub(super) fn fpu_fstp_m32(&mut self, bus: &mut impl common::Bus) -> Step {
        self.fpu_fst_m32(bus)?;
        self.fpu_pop();
        Ok(())
    }

    pub(super) fn fpu_fstp_m64(&mut self, bus: &mut impl common::Bus) -> Step {
        self.fpu_fst_m64(bus)?;
        self.fpu_pop();
        Ok(())
    }

    pub(super) fn fpu_fstp_m80(&mut self, bus: &mut impl common::Bus) {
        if self.fpu_check_underflow(0) {
            let masked = self.state.fpu.control_word & 1 != 0;
            if masked
                && self
                    .fpu_write_tbyte(bus, &Fp80::INDEFINITE.to_le_bytes())
                    .is_err()
            {
                return;
            }
            self.fpu_pop();
            self.clk(Self::timing(53, 6));
            return;
        }
        let val = self.fpu_st(0);
        if self.fpu_write_tbyte(bus, &val.to_le_bytes()).is_err() {
            return;
        }
        self.fpu_pop();
        self.clk(Self::timing(53, 6));
    }

    pub(super) fn fpu_fstp_sti(&mut self, i: u8) {
        self.fpu_fst_sti(i);
        self.fpu_pop();
    }

    pub(super) fn fpu_fild_m16(&mut self, bus: &mut impl common::Bus) -> Step {
        let raw = self.fpu_read_u16(bus)?;
        let fp = Fp80::from_i16(raw as i16);
        self.fpu_push(fp);
        self.clk(Self::timing(61, 13));
        Ok(())
    }

    pub(super) fn fpu_fild_m32(&mut self, bus: &mut impl common::Bus) -> Step {
        let raw = self.fpu_read_u32(bus)?;
        let fp = Fp80::from_i32(raw as i32);
        self.fpu_push(fp);
        self.clk(Self::timing(45, 9));
        Ok(())
    }

    pub(super) fn fpu_fild_m64(&mut self, bus: &mut impl common::Bus) -> Step {
        let raw = self.fpu_read_u64(bus)?;
        let fp = Fp80::from_i64(raw as i64);
        self.fpu_push(fp);
        self.clk(Self::timing(56, 10));
        Ok(())
    }

    pub(super) fn fpu_fist_m16(&mut self, bus: &mut impl common::Bus) -> Step {
        if self.fpu_check_underflow(0) {
            let masked = self.state.fpu.control_word & 1 != 0;
            if masked {
                self.fpu_write_u16(bus, 0x8000u16)?;
            }
            self.clk(Self::timing(82, 29));
            return Ok(());
        }
        let val = self.fpu_st(0);
        let mut ef = ExceptionFlags::default();
        let i = val.to_i16(self.fpu_rounding_mode(), &mut ef);
        self.fpu_check_result(&ef);
        self.fpu_write_u16(bus, i as u16)?;
        self.clk(Self::timing(82, 29));
        Ok(())
    }

    pub(super) fn fpu_fist_m32(&mut self, bus: &mut impl common::Bus) -> Step {
        if self.fpu_check_underflow(0) {
            let masked = self.state.fpu.control_word & 1 != 0;
            if masked {
                self.fpu_write_u32(bus, 0x8000_0000u32)?;
            }
            self.clk(Self::timing(79, 28));
            return Ok(());
        }
        let val = self.fpu_st(0);
        let mut ef = ExceptionFlags::default();
        let i = val.to_i32(self.fpu_rounding_mode(), &mut ef);
        self.fpu_check_result(&ef);
        self.fpu_write_u32(bus, i as u32)?;
        self.clk(Self::timing(79, 28));
        Ok(())
    }

    pub(super) fn fpu_fistp_m16(&mut self, bus: &mut impl common::Bus) -> Step {
        self.fpu_fist_m16(bus)?;
        self.fpu_pop();
        Ok(())
    }

    pub(super) fn fpu_fistp_m32(&mut self, bus: &mut impl common::Bus) -> Step {
        self.fpu_fist_m32(bus)?;
        self.fpu_pop();
        Ok(())
    }

    pub(super) fn fpu_fistp_m64(&mut self, bus: &mut impl common::Bus) {
        if self.fpu_check_underflow(0) {
            let masked = self.state.fpu.control_word & 1 != 0;
            if masked && self.fpu_write_u64(bus, 0x8000_0000_0000_0000u64).is_err() {
                return;
            }
            self.fpu_pop();
            self.clk(Self::timing(80, 28));
            return;
        }
        let val = self.fpu_st(0);
        let mut ef = ExceptionFlags::default();
        let i = val.to_i64(self.fpu_rounding_mode(), &mut ef);
        self.fpu_check_result(&ef);
        if self.fpu_write_u64(bus, i as u64).is_err() {
            return;
        }
        self.fpu_pop();
        self.clk(Self::timing(80, 28));
    }

    pub(super) fn fpu_fbld(&mut self, bus: &mut impl common::Bus) -> Step {
        let bytes = self.fpu_read_tbyte(bus)?;
        let mut ef = ExceptionFlags::default();
        let fp = Fp80::from_bcd(bytes, &mut ef);
        self.fpu_check_result(&ef);
        self.fpu_push(fp);
        self.clk(Self::timing(266, 70));
        Ok(())
    }

    pub(super) fn fpu_fbstp(&mut self, bus: &mut impl common::Bus) {
        if self.fpu_check_underflow(0) {
            let masked = self.state.fpu.control_word & 1 != 0;
            if masked
                && self
                    .fpu_write_tbyte(
                        bus,
                        &[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0xFF, 0xFF],
                    )
                    .is_err()
            {
                return;
            }
            self.fpu_pop();
            self.clk(Self::timing(512, 172));
            return;
        }
        let val = self.fpu_st(0);
        let mut ef = ExceptionFlags::default();
        let bcd = val.to_bcd(self.fpu_rounding_mode(), &mut ef);
        self.fpu_check_result(&ef);
        if self.fpu_write_tbyte(bus, &bcd).is_err() {
            return;
        }
        self.fpu_pop();
        self.clk(Self::timing(512, 172));
    }

    pub(super) fn fpu_fxch(&mut self, i: u8) {
        let underflow_0 = self.fpu_check_underflow(0);
        let underflow_i = self.fpu_check_underflow(i);
        if !underflow_0 && !underflow_i {
            let a = self.fpu_st(0);
            let b = self.fpu_st(i);
            self.fpu_write_st(0, b);
            self.fpu_write_st(i, a);
        } else {
            let masked = self.state.fpu.control_word & 1 != 0;
            if masked {
                if underflow_0 {
                    self.fpu_write_st(0, Fp80::INDEFINITE);
                }
                if underflow_i {
                    self.fpu_write_st(i, Fp80::INDEFINITE);
                }
                let a = self.fpu_st(0);
                let b = self.fpu_st(i);
                self.fpu_write_st(0, b);
                self.fpu_write_st(i, a);
            }
        }
        self.state.fpu.status_word &= !0x0200; // C1=0
        self.clk(Self::timing(18, 4));
    }

    pub(super) fn fpu_fld1(&mut self) {
        self.fpu_push(Fp80::ONE);
        self.clk(Self::timing(24, 4));
    }

    pub(super) fn fpu_fldz(&mut self) {
        self.fpu_push(Fp80::ZERO);
        self.clk(Self::timing(20, 4));
    }

    pub(super) fn fpu_fldl2t(&mut self) {
        let rc = self.fpu_rounding_mode();
        let val = match rc {
            RoundingMode::Up => Fp80::LOG2_10_UP,
            _ => Fp80::LOG2_10_DOWN,
        };
        self.fpu_push(val);
        self.clk(Self::timing(40, 8));
    }

    pub(super) fn fpu_fldl2e(&mut self) {
        let rc = self.fpu_rounding_mode();
        let val = match rc {
            RoundingMode::Up | RoundingMode::NearestEven => Fp80::LOG2_E_UP,
            _ => Fp80::LOG2_E_DOWN,
        };
        self.fpu_push(val);
        self.clk(Self::timing(40, 8));
    }

    pub(super) fn fpu_fldpi(&mut self) {
        let rc = self.fpu_rounding_mode();
        let val = match rc {
            RoundingMode::Up | RoundingMode::NearestEven => Fp80::PI_UP,
            _ => Fp80::PI_DOWN,
        };
        self.fpu_push(val);
        self.clk(Self::timing(40, 8));
    }

    pub(super) fn fpu_fldlg2(&mut self) {
        let rc = self.fpu_rounding_mode();
        let val = match rc {
            RoundingMode::Up | RoundingMode::NearestEven => Fp80::LOG10_2_UP,
            _ => Fp80::LOG10_2_DOWN,
        };
        self.fpu_push(val);
        self.clk(Self::timing(41, 8));
    }

    pub(super) fn fpu_fldln2(&mut self) {
        let rc = self.fpu_rounding_mode();
        let val = match rc {
            RoundingMode::Up | RoundingMode::NearestEven => Fp80::LN_2_UP,
            _ => Fp80::LN_2_DOWN,
        };
        self.fpu_push(val);
        self.clk(Self::timing(41, 8));
    }

    fn fpu_arith_reg_reg(
        &mut self,
        dest: u8,
        src: u8,
        op: fn(Fp80, Fp80, RoundingMode, Precision, &mut ExceptionFlags) -> Fp80,
        t387: i32,
        t486: i32,
    ) {
        if self.fpu_check_underflow(dest) || self.fpu_check_underflow(src) {
            let masked = self.state.fpu.control_word & 1 != 0;
            if masked {
                self.fpu_write_st(dest, Fp80::INDEFINITE);
            }
            self.clk(Self::timing(t387, t486));
            return;
        }
        let a = self.fpu_st(dest);
        let b = self.fpu_st(src);
        let rc = self.fpu_rounding_mode();
        let pc = self.fpu_precision();
        let mut ef = ExceptionFlags::default();
        let result = op(a, b, rc, pc, &mut ef);
        self.fpu_check_result(&ef);
        self.fpu_write_st(dest, result);
        self.clk(Self::timing(t387, t486));
    }

    fn fpu_arith_m32(
        &mut self,
        op: fn(Fp80, Fp80, RoundingMode, Precision, &mut ExceptionFlags) -> Fp80,
        t387: i32,
        t486: i32,
        bus: &mut impl common::Bus,
    ) -> Step {
        if self.fpu_check_underflow(0) {
            let masked = self.state.fpu.control_word & 1 != 0;
            if masked {
                self.fpu_write_st(0, Fp80::INDEFINITE);
            }
            self.clk(Self::timing(t387, t486));
            return Ok(());
        }
        let raw = self.fpu_read_u32(bus)?;
        let val = f32::from_bits(raw);
        let mut ef = ExceptionFlags::default();
        let b = Fp80::from_f32(val, &mut ef);
        let a = self.fpu_st(0);
        let rc = self.fpu_rounding_mode();
        let pc = self.fpu_precision();
        let result = op(a, b, rc, pc, &mut ef);
        self.fpu_check_result(&ef);
        self.fpu_write_st(0, result);
        self.clk(Self::timing(t387, t486));
        Ok(())
    }

    fn fpu_arith_m64(
        &mut self,
        op: fn(Fp80, Fp80, RoundingMode, Precision, &mut ExceptionFlags) -> Fp80,
        t387: i32,
        t486: i32,
        bus: &mut impl common::Bus,
    ) -> Step {
        if self.fpu_check_underflow(0) {
            let masked = self.state.fpu.control_word & 1 != 0;
            if masked {
                self.fpu_write_st(0, Fp80::INDEFINITE);
            }
            self.clk(Self::timing(t387, t486));
            return Ok(());
        }
        let raw = self.fpu_read_u64(bus)?;
        let val = f64::from_bits(raw);
        let mut ef = ExceptionFlags::default();
        let b = Fp80::from_f64(val, &mut ef);
        let a = self.fpu_st(0);
        let rc = self.fpu_rounding_mode();
        let pc = self.fpu_precision();
        let result = op(a, b, rc, pc, &mut ef);
        self.fpu_check_result(&ef);
        self.fpu_write_st(0, result);
        self.clk(Self::timing(t387, t486));
        Ok(())
    }

    fn fpu_arith_mi32(
        &mut self,
        op: fn(Fp80, Fp80, RoundingMode, Precision, &mut ExceptionFlags) -> Fp80,
        t387: i32,
        t486: i32,
        bus: &mut impl common::Bus,
    ) -> Step {
        if self.fpu_check_underflow(0) {
            let masked = self.state.fpu.control_word & 1 != 0;
            if masked {
                self.fpu_write_st(0, Fp80::INDEFINITE);
            }
            self.clk(Self::timing(t387, t486));
            return Ok(());
        }
        let raw = self.fpu_read_u32(bus)?;
        let b = Fp80::from_i32(raw as i32);
        let a = self.fpu_st(0);
        let rc = self.fpu_rounding_mode();
        let pc = self.fpu_precision();
        let mut ef = ExceptionFlags::default();
        let result = op(a, b, rc, pc, &mut ef);
        self.fpu_check_result(&ef);
        self.fpu_write_st(0, result);
        self.clk(Self::timing(t387, t486));
        Ok(())
    }

    fn fpu_arith_mi16(
        &mut self,
        op: fn(Fp80, Fp80, RoundingMode, Precision, &mut ExceptionFlags) -> Fp80,
        t387: i32,
        t486: i32,
        bus: &mut impl common::Bus,
    ) -> Step {
        if self.fpu_check_underflow(0) {
            let masked = self.state.fpu.control_word & 1 != 0;
            if masked {
                self.fpu_write_st(0, Fp80::INDEFINITE);
            }
            self.clk(Self::timing(t387, t486));
            return Ok(());
        }
        let raw = self.fpu_read_u16(bus)?;
        let b = Fp80::from_i16(raw as i16);
        let a = self.fpu_st(0);
        let rc = self.fpu_rounding_mode();
        let pc = self.fpu_precision();
        let mut ef = ExceptionFlags::default();
        let result = op(a, b, rc, pc, &mut ef);
        self.fpu_check_result(&ef);
        self.fpu_write_st(0, result);
        self.clk(Self::timing(t387, t486));
        Ok(())
    }

    pub(super) fn fpu_fadd_m32(&mut self, bus: &mut impl common::Bus) -> Step {
        self.fpu_arith_m32(Fp80::add, 24, 8, bus)?;
        Ok(())
    }

    pub(super) fn fpu_fadd_m64(&mut self, bus: &mut impl common::Bus) -> Step {
        self.fpu_arith_m64(Fp80::add, 29, 8, bus)?;
        Ok(())
    }

    pub(super) fn fpu_fadd_st0_sti(&mut self, i: u8) {
        self.fpu_arith_reg_reg(0, i, Fp80::add, 23, 8);
    }

    pub(super) fn fpu_fadd_sti_st0(&mut self, i: u8) {
        self.fpu_arith_reg_reg(i, 0, Fp80::add, 23, 8);
    }

    pub(super) fn fpu_faddp_sti_st0(&mut self, i: u8) {
        self.fpu_arith_reg_reg(i, 0, Fp80::add, 23, 8);
        self.fpu_pop();
    }

    pub(super) fn fpu_fiadd_m32(&mut self, bus: &mut impl common::Bus) -> Step {
        self.fpu_arith_mi32(Fp80::add, 57, 19, bus)?;
        Ok(())
    }

    pub(super) fn fpu_fiadd_m16(&mut self, bus: &mut impl common::Bus) -> Step {
        self.fpu_arith_mi16(Fp80::add, 71, 20, bus)?;
        Ok(())
    }

    pub(super) fn fpu_fsub_m32(&mut self, bus: &mut impl common::Bus) -> Step {
        self.fpu_arith_m32(Fp80::sub, 24, 8, bus)?;
        Ok(())
    }

    pub(super) fn fpu_fsub_m64(&mut self, bus: &mut impl common::Bus) -> Step {
        self.fpu_arith_m64(Fp80::sub, 28, 8, bus)?;
        Ok(())
    }

    pub(super) fn fpu_fsub_st0_sti(&mut self, i: u8) {
        self.fpu_arith_reg_reg(0, i, Fp80::sub, 26, 8);
    }

    pub(super) fn fpu_fsub_sti_st0(&mut self, i: u8) {
        self.fpu_arith_reg_reg(i, 0, Fp80::sub, 26, 8);
    }

    pub(super) fn fpu_fsubp_sti_st0(&mut self, i: u8) {
        self.fpu_arith_reg_reg(i, 0, Fp80::sub, 26, 8);
        self.fpu_pop();
    }

    pub(super) fn fpu_fisub_m32(&mut self, bus: &mut impl common::Bus) -> Step {
        self.fpu_arith_mi32(Fp80::sub, 57, 19, bus)?;
        Ok(())
    }

    pub(super) fn fpu_fisub_m16(&mut self, bus: &mut impl common::Bus) -> Step {
        self.fpu_arith_mi16(Fp80::sub, 71, 20, bus)?;
        Ok(())
    }

    fn fpu_subr(
        a: Fp80,
        b: Fp80,
        rc: RoundingMode,
        pc: Precision,
        ef: &mut ExceptionFlags,
    ) -> Fp80 {
        Fp80::sub(b, a, rc, pc, ef)
    }

    pub(super) fn fpu_fsubr_m32(&mut self, bus: &mut impl common::Bus) -> Step {
        self.fpu_arith_m32(Self::fpu_subr, 24, 8, bus)?;
        Ok(())
    }

    pub(super) fn fpu_fsubr_m64(&mut self, bus: &mut impl common::Bus) -> Step {
        self.fpu_arith_m64(Self::fpu_subr, 28, 8, bus)?;
        Ok(())
    }

    pub(super) fn fpu_fsubr_st0_sti(&mut self, i: u8) {
        // ST(0) = ST(i) - ST(0)
        self.fpu_arith_reg_reg(0, i, Self::fpu_subr, 26, 8);
    }

    pub(super) fn fpu_fsubr_sti_st0(&mut self, i: u8) {
        // ST(i) = ST(0) - ST(i)
        self.fpu_arith_reg_reg(i, 0, Self::fpu_subr, 26, 8);
    }

    pub(super) fn fpu_fsubrp_sti_st0(&mut self, i: u8) {
        self.fpu_arith_reg_reg(i, 0, Self::fpu_subr, 26, 8);
        self.fpu_pop();
    }

    pub(super) fn fpu_fisubr_m32(&mut self, bus: &mut impl common::Bus) -> Step {
        self.fpu_arith_mi32(Self::fpu_subr, 57, 19, bus)?;
        Ok(())
    }

    pub(super) fn fpu_fisubr_m16(&mut self, bus: &mut impl common::Bus) -> Step {
        self.fpu_arith_mi16(Self::fpu_subr, 71, 20, bus)?;
        Ok(())
    }

    pub(super) fn fpu_fmul_m32(&mut self, bus: &mut impl common::Bus) -> Step {
        self.fpu_arith_m32(Fp80::mul, 27, 11, bus)?;
        Ok(())
    }

    pub(super) fn fpu_fmul_m64(&mut self, bus: &mut impl common::Bus) -> Step {
        self.fpu_arith_m64(Fp80::mul, 32, 14, bus)?;
        Ok(())
    }

    pub(super) fn fpu_fmul_st0_sti(&mut self, i: u8) {
        self.fpu_arith_reg_reg(0, i, Fp80::mul, 46, 16);
    }

    pub(super) fn fpu_fmul_sti_st0(&mut self, i: u8) {
        self.fpu_arith_reg_reg(i, 0, Fp80::mul, 46, 16);
    }

    pub(super) fn fpu_fmulp_sti_st0(&mut self, i: u8) {
        self.fpu_arith_reg_reg(i, 0, Fp80::mul, 29, 16);
        self.fpu_pop();
    }

    pub(super) fn fpu_fimul_m32(&mut self, bus: &mut impl common::Bus) -> Step {
        self.fpu_arith_mi32(Fp80::mul, 61, 22, bus)?;
        Ok(())
    }

    pub(super) fn fpu_fimul_m16(&mut self, bus: &mut impl common::Bus) -> Step {
        self.fpu_arith_mi16(Fp80::mul, 76, 23, bus)?;
        Ok(())
    }

    pub(super) fn fpu_fdiv_m32(&mut self, bus: &mut impl common::Bus) -> Step {
        self.fpu_arith_m32(Fp80::div, 89, 73, bus)?;
        Ok(())
    }

    pub(super) fn fpu_fdiv_m64(&mut self, bus: &mut impl common::Bus) -> Step {
        self.fpu_arith_m64(Fp80::div, 94, 73, bus)?;
        Ok(())
    }

    pub(super) fn fpu_fdiv_st0_sti(&mut self, i: u8) {
        self.fpu_arith_reg_reg(0, i, Fp80::div, 88, 73);
    }

    pub(super) fn fpu_fdiv_sti_st0(&mut self, i: u8) {
        self.fpu_arith_reg_reg(i, 0, Fp80::div, 88, 73);
    }

    pub(super) fn fpu_fdivp_sti_st0(&mut self, i: u8) {
        self.fpu_arith_reg_reg(i, 0, Fp80::div, 91, 73);
        self.fpu_pop();
    }

    pub(super) fn fpu_fidiv_m32(&mut self, bus: &mut impl common::Bus) -> Step {
        self.fpu_arith_mi32(Fp80::div, 120, 84, bus)?;
        Ok(())
    }

    pub(super) fn fpu_fidiv_m16(&mut self, bus: &mut impl common::Bus) -> Step {
        self.fpu_arith_mi16(Fp80::div, 136, 85, bus)?;
        Ok(())
    }

    fn fpu_divr(
        a: Fp80,
        b: Fp80,
        rc: RoundingMode,
        pc: Precision,
        ef: &mut ExceptionFlags,
    ) -> Fp80 {
        Fp80::div(b, a, rc, pc, ef)
    }

    pub(super) fn fpu_fdivr_m32(&mut self, bus: &mut impl common::Bus) -> Step {
        self.fpu_arith_m32(Self::fpu_divr, 89, 73, bus)?;
        Ok(())
    }

    pub(super) fn fpu_fdivr_m64(&mut self, bus: &mut impl common::Bus) -> Step {
        self.fpu_arith_m64(Self::fpu_divr, 94, 73, bus)?;
        Ok(())
    }

    pub(super) fn fpu_fdivr_st0_sti(&mut self, i: u8) {
        self.fpu_arith_reg_reg(0, i, Self::fpu_divr, 88, 73);
    }

    pub(super) fn fpu_fdivr_sti_st0(&mut self, i: u8) {
        self.fpu_arith_reg_reg(i, 0, Self::fpu_divr, 88, 73);
    }

    pub(super) fn fpu_fdivrp_sti_st0(&mut self, i: u8) {
        self.fpu_arith_reg_reg(i, 0, Self::fpu_divr, 91, 73);
        self.fpu_pop();
    }

    pub(super) fn fpu_fidivr_m32(&mut self, bus: &mut impl common::Bus) -> Step {
        self.fpu_arith_mi32(Self::fpu_divr, 121, 84, bus)?;
        Ok(())
    }

    pub(super) fn fpu_fidivr_m16(&mut self, bus: &mut impl common::Bus) -> Step {
        self.fpu_arith_mi16(Self::fpu_divr, 135, 85, bus)?;
        Ok(())
    }

    // Unary arithmetic
    pub(super) fn fpu_fsqrt(&mut self) {
        if self.fpu_check_underflow(0) {
            let masked = self.state.fpu.control_word & 1 != 0;
            if masked {
                self.fpu_write_st(0, Fp80::INDEFINITE);
            }
            self.clk(Self::timing(122, 83));
            return;
        }
        let a = self.fpu_st(0);
        let rc = self.fpu_rounding_mode();
        let pc = self.fpu_precision();
        let mut ef = ExceptionFlags::default();
        let result = a.sqrt(rc, pc, &mut ef);
        self.fpu_check_result(&ef);
        self.fpu_write_st(0, result);
        self.clk(Self::timing(122, 83));
    }

    pub(super) fn fpu_fabs(&mut self) {
        if self.fpu_check_underflow(0) {
            let masked = self.state.fpu.control_word & 1 != 0;
            if masked {
                self.fpu_write_st(0, Fp80::INDEFINITE);
            }
            self.clk(Self::timing(22, 3));
            return;
        }
        let val = self.fpu_st(0);
        self.fpu_write_st(0, val.abs());
        self.state.fpu.status_word &= !0x0200; // C1=0
        self.clk(Self::timing(22, 3));
    }

    pub(super) fn fpu_fchs(&mut self) {
        if self.fpu_check_underflow(0) {
            let masked = self.state.fpu.control_word & 1 != 0;
            if masked {
                self.fpu_write_st(0, Fp80::INDEFINITE);
            }
            self.clk(Self::timing(24, 6));
            return;
        }
        let val = self.fpu_st(0);
        self.fpu_write_st(0, val.negate());
        self.state.fpu.status_word &= !0x0200; // C1=0
        self.clk(Self::timing(24, 6));
    }

    pub(super) fn fpu_frndint(&mut self) {
        if self.fpu_check_underflow(0) {
            let masked = self.state.fpu.control_word & 1 != 0;
            if masked {
                self.fpu_write_st(0, Fp80::INDEFINITE);
            }
            self.clk(Self::timing(66, 21));
            return;
        }
        let a = self.fpu_st(0);
        let rc = self.fpu_rounding_mode();
        let mut ef = ExceptionFlags::default();
        let result = a.round_to_int(rc, &mut ef);
        self.fpu_check_result(&ef);
        self.fpu_write_st(0, result);
        self.clk(Self::timing(66, 21));
    }

    pub(super) fn fpu_fscale(&mut self) {
        if self.fpu_check_underflow(0) || self.fpu_check_underflow(1) {
            let masked = self.state.fpu.control_word & 1 != 0;
            if masked {
                self.fpu_write_st(0, Fp80::INDEFINITE);
            }
            self.clk(Self::timing(67, 30));
            return;
        }
        let a = self.fpu_st(0);
        let b = self.fpu_st(1);
        let mut ef = ExceptionFlags::default();
        let result = a.scale(b, &mut ef);
        self.fpu_check_result(&ef);
        self.fpu_write_st(0, result);
        self.clk(Self::timing(67, 30));
    }

    pub(super) fn fpu_fxtract(&mut self) {
        if self.fpu_check_underflow(0) {
            let masked = self.state.fpu.control_word & 1 != 0;
            if masked {
                self.fpu_write_st(0, Fp80::INDEFINITE);
                self.fpu_push(Fp80::INDEFINITE);
            }
            self.clk(Self::timing(70, 16));
            return;
        }
        let a = self.fpu_st(0);
        let mut ef = ExceptionFlags::default();
        let (significand, exponent) = a.extract(&mut ef);
        self.fpu_check_result(&ef);
        self.fpu_write_st(0, exponent);
        self.fpu_push(significand);
        self.clk(Self::timing(70, 16));
    }

    pub(super) fn fpu_fprem(&mut self) {
        if self.fpu_check_underflow(0) || self.fpu_check_underflow(1) {
            let masked = self.state.fpu.control_word & 1 != 0;
            if masked {
                self.fpu_write_st(0, Fp80::INDEFINITE);
            }
            self.clk(Self::timing(74, 70));
            return;
        }
        let a = self.fpu_st(0);
        let b = self.fpu_st(1);
        let mut ef = ExceptionFlags::default();
        let (result, quotient_bits, complete) = a.partial_remainder(b, &mut ef);
        self.fpu_check_result(&ef);
        self.fpu_write_st(0, result);
        // Set condition codes: C2=!complete, C0=Q2, C3=Q1, C1=Q0
        let c2 = !complete;
        let c0 = quotient_bits & 4 != 0;
        let c3 = quotient_bits & 2 != 0;
        let c1 = quotient_bits & 1 != 0;
        self.fpu_set_cc(c3, c2, c1, c0);
        self.clk(Self::timing(74, 70));
    }

    pub(super) fn fpu_fprem1(&mut self) {
        if self.fpu_check_underflow(0) || self.fpu_check_underflow(1) {
            let masked = self.state.fpu.control_word & 1 != 0;
            if masked {
                self.fpu_write_st(0, Fp80::INDEFINITE);
            }
            self.clk(Self::timing(95, 72));
            return;
        }
        let a = self.fpu_st(0);
        let b = self.fpu_st(1);
        let mut ef = ExceptionFlags::default();
        let (result, quotient_bits, complete) = a.ieee_remainder(b, &mut ef);
        self.fpu_check_result(&ef);
        self.fpu_write_st(0, result);
        let c2 = !complete;
        let c0 = quotient_bits & 4 != 0;
        let c3 = quotient_bits & 2 != 0;
        let c1 = quotient_bits & 1 != 0;
        self.fpu_set_cc(c3, c2, c1, c0);
        self.clk(Self::timing(95, 72));
    }

    fn fpu_compare_and_set_cc(&mut self, ordering: FpOrdering) {
        let (c3, c2, c0) = match ordering {
            FpOrdering::Greater => (false, false, false),
            FpOrdering::Less => (false, false, true),
            FpOrdering::Equal => (true, false, false),
            FpOrdering::Unordered => (true, true, true),
        };
        self.fpu_set_cc(c3, c2, false, c0);
    }

    pub(super) fn fpu_fcom_m32(&mut self, bus: &mut impl common::Bus) -> Step {
        if self.fpu_check_underflow(0) {
            self.fpu_compare_and_set_cc(FpOrdering::Unordered);
            self.clk(Self::timing(26, 4));
            return Ok(());
        }
        let raw = self.fpu_read_u32(bus)?;
        let val = f32::from_bits(raw);
        let mut ef = ExceptionFlags::default();
        let b = Fp80::from_f32(val, &mut ef);
        let a = self.fpu_st(0);
        let ordering = a.compare(b, &mut ef);
        self.fpu_check_result(&ef);
        self.fpu_compare_and_set_cc(ordering);
        self.clk(Self::timing(26, 4));
        Ok(())
    }

    pub(super) fn fpu_fcom_m64(&mut self, bus: &mut impl common::Bus) -> Step {
        if self.fpu_check_underflow(0) {
            self.fpu_compare_and_set_cc(FpOrdering::Unordered);
            self.clk(Self::timing(31, 4));
            return Ok(());
        }
        let raw = self.fpu_read_u64(bus)?;
        let val = f64::from_bits(raw);
        let mut ef = ExceptionFlags::default();
        let b = Fp80::from_f64(val, &mut ef);
        let a = self.fpu_st(0);
        let ordering = a.compare(b, &mut ef);
        self.fpu_check_result(&ef);
        self.fpu_compare_and_set_cc(ordering);
        self.clk(Self::timing(31, 4));
        Ok(())
    }

    pub(super) fn fpu_fcom_sti(&mut self, i: u8) {
        if self.fpu_check_underflow(0) || self.fpu_check_underflow(i) {
            self.fpu_compare_and_set_cc(FpOrdering::Unordered);
            self.clk(Self::timing(24, 4));
            return;
        }
        let a = self.fpu_st(0);
        let b = self.fpu_st(i);
        let mut ef = ExceptionFlags::default();
        let ordering = a.compare(b, &mut ef);
        self.fpu_check_result(&ef);
        self.fpu_compare_and_set_cc(ordering);
        self.clk(Self::timing(24, 4));
    }

    pub(super) fn fpu_fcomp_m32(&mut self, bus: &mut impl common::Bus) -> Step {
        self.fpu_fcom_m32(bus)?;
        self.fpu_pop();
        Ok(())
    }

    pub(super) fn fpu_fcomp_m64(&mut self, bus: &mut impl common::Bus) -> Step {
        self.fpu_fcom_m64(bus)?;
        self.fpu_pop();
        Ok(())
    }

    pub(super) fn fpu_fcomp_sti(&mut self, i: u8) {
        self.fpu_fcom_sti(i);
        self.fpu_pop();
    }

    pub(super) fn fpu_fcompp(&mut self) {
        if self.fpu_check_underflow(0) || self.fpu_check_underflow(1) {
            self.fpu_compare_and_set_cc(FpOrdering::Unordered);
            self.fpu_pop();
            self.fpu_pop();
            self.clk(Self::timing(26, 5));
            return;
        }
        let a = self.fpu_st(0);
        let b = self.fpu_st(1);
        let mut ef = ExceptionFlags::default();
        let ordering = a.compare(b, &mut ef);
        self.fpu_check_result(&ef);
        self.fpu_compare_and_set_cc(ordering);
        self.fpu_pop();
        self.fpu_pop();
        self.clk(Self::timing(26, 5));
    }

    pub(super) fn fpu_fucom_sti(&mut self, i: u8) {
        if self.fpu_check_underflow(0) || self.fpu_check_underflow(i) {
            self.fpu_compare_and_set_cc(FpOrdering::Unordered);
            self.clk(Self::timing(24, 4));
            return;
        }
        let a = self.fpu_st(0);
        let b = self.fpu_st(i);
        let mut ef = ExceptionFlags::default();
        let ordering = a.compare_quiet(b, &mut ef);
        self.fpu_check_result(&ef);
        self.fpu_compare_and_set_cc(ordering);
        self.clk(Self::timing(24, 4));
    }

    pub(super) fn fpu_fucomp_sti(&mut self, i: u8) {
        self.fpu_fucom_sti(i);
        self.fpu_pop();
    }

    pub(super) fn fpu_fucompp(&mut self) {
        if self.fpu_check_underflow(0) || self.fpu_check_underflow(1) {
            self.fpu_compare_and_set_cc(FpOrdering::Unordered);
            self.fpu_pop();
            self.fpu_pop();
            self.clk(Self::timing(26, 5));
            return;
        }
        let a = self.fpu_st(0);
        let b = self.fpu_st(1);
        let mut ef = ExceptionFlags::default();
        let ordering = a.compare_quiet(b, &mut ef);
        self.fpu_check_result(&ef);
        self.fpu_compare_and_set_cc(ordering);
        self.fpu_pop();
        self.fpu_pop();
        self.clk(Self::timing(26, 5));
    }

    pub(super) fn fpu_ficom_m32(&mut self, bus: &mut impl common::Bus) -> Step {
        if self.fpu_check_underflow(0) {
            self.fpu_compare_and_set_cc(FpOrdering::Unordered);
            self.clk(Self::timing(56, 15));
            return Ok(());
        }
        let raw = self.fpu_read_u32(bus)?;
        let b = Fp80::from_i32(raw as i32);
        let a = self.fpu_st(0);
        let mut ef = ExceptionFlags::default();
        let ordering = a.compare(b, &mut ef);
        self.fpu_check_result(&ef);
        self.fpu_compare_and_set_cc(ordering);
        self.clk(Self::timing(56, 15));
        Ok(())
    }

    pub(super) fn fpu_ficom_m16(&mut self, bus: &mut impl common::Bus) -> Step {
        if self.fpu_check_underflow(0) {
            self.fpu_compare_and_set_cc(FpOrdering::Unordered);
            self.clk(Self::timing(71, 16));
            return Ok(());
        }
        let raw = self.fpu_read_u16(bus)?;
        let b = Fp80::from_i16(raw as i16);
        let a = self.fpu_st(0);
        let mut ef = ExceptionFlags::default();
        let ordering = a.compare(b, &mut ef);
        self.fpu_check_result(&ef);
        self.fpu_compare_and_set_cc(ordering);
        self.clk(Self::timing(71, 16));
        Ok(())
    }

    pub(super) fn fpu_ficomp_m32(&mut self, bus: &mut impl common::Bus) -> Step {
        self.fpu_ficom_m32(bus)?;
        self.fpu_pop();
        Ok(())
    }

    pub(super) fn fpu_ficomp_m16(&mut self, bus: &mut impl common::Bus) -> Step {
        self.fpu_ficom_m16(bus)?;
        self.fpu_pop();
        Ok(())
    }

    pub(super) fn fpu_ftst(&mut self) {
        if self.fpu_check_underflow(0) {
            self.fpu_compare_and_set_cc(FpOrdering::Unordered);
            self.clk(Self::timing(28, 4));
            return;
        }
        let a = self.fpu_st(0);
        let mut ef = ExceptionFlags::default();
        let ordering = a.compare(Fp80::ZERO, &mut ef);
        self.fpu_check_result(&ef);
        self.fpu_compare_and_set_cc(ordering);
        self.clk(Self::timing(28, 4));
    }

    pub(super) fn fpu_fxam(&mut self) {
        let phys = self.fpu_st_phys(0);
        let val = self.state.fpu.registers[phys];
        let tag = (self.state.fpu.tag_word >> (phys * 2)) & 3;

        let c1 = val.sign();

        let (c3, c2, c0) = if tag == 0b11 {
            // Empty
            (true, false, true)
        } else {
            use softfloat::FpClass;
            match val.classify() {
                FpClass::Unsupported => (false, false, false),
                FpClass::Nan => (false, false, true),
                FpClass::Normal => (false, true, false),
                FpClass::Infinity => (false, true, true),
                FpClass::Zero => (true, false, false),
                FpClass::Empty => (true, false, true),
                FpClass::Denormal => (true, true, false),
            }
        };

        self.fpu_set_cc(c3, c2, c1, c0);
        self.clk(Self::timing(30, 8));
    }

    pub(super) fn fpu_f2xm1(&mut self) {
        if self.fpu_check_underflow(0) {
            let masked = self.state.fpu.control_word & 1 != 0;
            if masked {
                self.fpu_write_st(0, Fp80::INDEFINITE);
            }
            self.clk(Self::timing(211, 140));
            return;
        }
        let a = self.fpu_st(0);
        let mut ef = ExceptionFlags::default();
        let result = a.f2xm1(&mut ef);
        self.fpu_check_result(&ef);
        self.fpu_write_st(0, result);
        self.clk(Self::timing(211, 140));
    }

    pub(super) fn fpu_fyl2x(&mut self) {
        if self.fpu_check_underflow(0) || self.fpu_check_underflow(1) {
            let masked = self.state.fpu.control_word & 1 != 0;
            if masked {
                self.fpu_write_st(1, Fp80::INDEFINITE);
            }
            self.fpu_pop();
            self.clk(Self::timing(120, 196));
            return;
        }
        let x = self.fpu_st(0);
        let y = self.fpu_st(1);
        let mut ef = ExceptionFlags::default();
        let result = x.fyl2x(y, &mut ef);
        self.fpu_check_result(&ef);
        self.fpu_write_st(1, result);
        self.fpu_pop();
        self.clk(Self::timing(120, 196));
    }

    pub(super) fn fpu_fyl2xp1(&mut self) {
        if self.fpu_check_underflow(0) || self.fpu_check_underflow(1) {
            let masked = self.state.fpu.control_word & 1 != 0;
            if masked {
                self.fpu_write_st(1, Fp80::INDEFINITE);
            }
            self.fpu_pop();
            self.clk(Self::timing(257, 171));
            return;
        }
        let x = self.fpu_st(0);
        let y = self.fpu_st(1);
        let mut ef = ExceptionFlags::default();
        let result = x.fyl2xp1(y, &mut ef);
        self.fpu_check_result(&ef);
        self.fpu_write_st(1, result);
        self.fpu_pop();
        self.clk(Self::timing(257, 171));
    }

    pub(super) fn fpu_fptan(&mut self) {
        if self.fpu_check_underflow(0) {
            let masked = self.state.fpu.control_word & 1 != 0;
            if masked {
                self.fpu_write_st(0, Fp80::INDEFINITE);
                self.fpu_push(Fp80::ONE);
            }
            self.clk(Self::timing(191, 200));
            return;
        }
        let x = self.fpu_st(0);
        let mut ef = ExceptionFlags::default();
        let (tan_val, out_of_range) = x.fptan(&mut ef);
        if out_of_range {
            self.state.fpu.status_word |= 0x0400; // C2=1
            self.clk(Self::timing(191, 200));
            return;
        }
        self.fpu_check_result(&ef);
        self.fpu_write_st(0, tan_val);
        self.fpu_push(Fp80::ONE);
        self.state.fpu.status_word &= !0x0400; // C2=0
        self.clk(Self::timing(191, 200));
    }

    pub(super) fn fpu_fpatan(&mut self) {
        if self.fpu_check_underflow(0) || self.fpu_check_underflow(1) {
            let masked = self.state.fpu.control_word & 1 != 0;
            if masked {
                self.fpu_write_st(1, Fp80::INDEFINITE);
            }
            self.fpu_pop();
            self.clk(Self::timing(314, 218));
            return;
        }
        let x = self.fpu_st(0); // ST(0)
        let y = self.fpu_st(1); // ST(1)
        let mut ef = ExceptionFlags::default();
        let result = y.fpatan(x, &mut ef);
        self.fpu_check_result(&ef);
        self.fpu_write_st(1, result);
        self.fpu_pop();
        self.clk(Self::timing(314, 218));
    }

    pub(super) fn fpu_fsin(&mut self) {
        if self.fpu_check_underflow(0) {
            let masked = self.state.fpu.control_word & 1 != 0;
            if masked {
                self.fpu_write_st(0, Fp80::INDEFINITE);
            }
            self.clk(Self::timing(122, 257));
            return;
        }
        let x = self.fpu_st(0);
        let mut ef = ExceptionFlags::default();
        let (result, out_of_range) = x.fsin(&mut ef);
        if out_of_range {
            self.state.fpu.status_word |= 0x0400; // C2=1
            self.clk(Self::timing(122, 257));
            return;
        }
        self.fpu_check_result(&ef);
        self.fpu_write_st(0, result);
        self.state.fpu.status_word &= !0x0400; // C2=0
        self.clk(Self::timing(122, 257));
    }

    pub(super) fn fpu_fcos(&mut self) {
        if self.fpu_check_underflow(0) {
            let masked = self.state.fpu.control_word & 1 != 0;
            if masked {
                self.fpu_write_st(0, Fp80::INDEFINITE);
            }
            self.clk(Self::timing(123, 257));
            return;
        }
        let x = self.fpu_st(0);
        let mut ef = ExceptionFlags::default();
        let (result, out_of_range) = x.fcos(&mut ef);
        if out_of_range {
            self.state.fpu.status_word |= 0x0400; // C2=1
            self.clk(Self::timing(123, 257));
            return;
        }
        self.fpu_check_result(&ef);
        self.fpu_write_st(0, result);
        self.state.fpu.status_word &= !0x0400; // C2=0
        self.clk(Self::timing(123, 257));
    }

    pub(super) fn fpu_fsincos(&mut self) {
        if self.fpu_check_underflow(0) {
            let masked = self.state.fpu.control_word & 1 != 0;
            if masked {
                self.fpu_write_st(0, Fp80::INDEFINITE);
                self.fpu_push(Fp80::INDEFINITE);
            }
            self.clk(Self::timing(194, 292));
            return;
        }
        let x = self.fpu_st(0);
        let mut ef = ExceptionFlags::default();
        let (sin_val, cos_val, out_of_range) = x.fsincos(&mut ef);
        if out_of_range {
            self.state.fpu.status_word |= 0x0400; // C2=1
            self.clk(Self::timing(194, 292));
            return;
        }
        self.fpu_check_result(&ef);
        self.fpu_write_st(0, sin_val);
        self.fpu_push(cos_val);
        self.state.fpu.status_word &= !0x0400; // C2=0
        self.clk(Self::timing(194, 292));
    }

    pub(super) fn fpu_fnstenv(&mut self, bus: &mut impl common::Bus) {
        let use_32bit = self.code_segment_32bit() ^ self.operand_size_override;
        let protected = self.is_protected_mode();

        let saved = if use_32bit {
            self.fpu_fnstenv_32bit(protected, bus)
        } else {
            self.fpu_fnstenv_16bit(protected, bus)
        };
        if saved.is_err() {
            return;
        }

        // After FSTENV, mask all exceptions. Only runs on full success
        // so a partial-fault FNSTENV leaves the control word untouched.
        self.state.fpu.control_word |= 0x3F;
        self.fpu_update_es();
        self.clk(Self::timing(103, 67));
    }

    fn fpu_fnstenv_16bit(&mut self, protected: bool, bus: &mut impl common::Bus) -> Step {
        let base = self.ea;
        self.write_word_linear(bus, base, self.state.fpu.control_word)?;
        self.write_word_linear(bus, base.wrapping_add(2), self.state.fpu.status_word)?;
        self.write_word_linear(bus, base.wrapping_add(4), self.state.fpu.tag_word)?;

        if protected {
            self.write_word_linear(bus, base.wrapping_add(6), self.state.fpu.fip_offset as u16)?;
            self.write_word_linear(bus, base.wrapping_add(8), self.state.fpu.fip_selector)?;
            self.write_word_linear(bus, base.wrapping_add(10), self.state.fpu.fdp_offset as u16)?;
            self.write_word_linear(bus, base.wrapping_add(12), self.state.fpu.fdp_selector)
        } else {
            self.write_word_linear(bus, base.wrapping_add(6), self.state.fpu.fip_offset as u16)?;
            let ip_hi_opcode = (self.state.fpu.fpu_opcode & 0x7FF)
                | (((self.state.fpu.fip_offset >> 16) as u16 & 0xF) << 12);
            self.write_word_linear(bus, base.wrapping_add(8), ip_hi_opcode)?;
            self.write_word_linear(bus, base.wrapping_add(10), self.state.fpu.fdp_offset as u16)?;
            self.write_word_linear(
                bus,
                base.wrapping_add(12),
                ((self.state.fpu.fdp_offset >> 16) as u16 & 0xF) << 12,
            )
        }
    }

    fn fpu_write_env_u32(&mut self, bus: &mut impl common::Bus, addr: u32, val: u32) -> Step {
        self.write_word_linear(bus, addr, val as u16)?;
        self.write_word_linear(bus, addr.wrapping_add(2), (val >> 16) as u16)
    }

    fn fpu_read_env_u32(&mut self, bus: &mut impl common::Bus, addr: u32) -> Step<u32> {
        let lo = self.read_word_linear(bus, addr)? as u32;
        let hi = self.read_word_linear(bus, addr.wrapping_add(2))? as u32;
        Ok(lo | (hi << 16))
    }

    fn fpu_fnstenv_32bit(&mut self, protected: bool, bus: &mut impl common::Bus) -> Step {
        let base = self.ea;
        self.fpu_write_env_u32(bus, base, 0xFFFF_0000 | self.state.fpu.control_word as u32)?;
        self.fpu_write_env_u32(
            bus,
            base.wrapping_add(4),
            0xFFFF_0000 | self.state.fpu.status_word as u32,
        )?;
        self.fpu_write_env_u32(
            bus,
            base.wrapping_add(8),
            0xFFFF_0000 | self.state.fpu.tag_word as u32,
        )?;

        if protected {
            self.fpu_write_env_u32(bus, base.wrapping_add(12), self.state.fpu.fip_offset)?;
            let opcode_cs = (self.state.fpu.fpu_opcode as u32 & 0x7FF) << 16
                | self.state.fpu.fip_selector as u32;
            self.fpu_write_env_u32(bus, base.wrapping_add(16), opcode_cs)?;
            self.fpu_write_env_u32(bus, base.wrapping_add(20), self.state.fpu.fdp_offset)?;
            self.fpu_write_env_u32(
                bus,
                base.wrapping_add(24),
                0xFFFF_0000 | self.state.fpu.fdp_selector as u32,
            )
        } else {
            self.fpu_write_env_u32(
                bus,
                base.wrapping_add(12),
                0xFFFF_0000 | (self.state.fpu.fip_offset & 0xFFFF),
            )?;
            let ip_hi_opcode = (self.state.fpu.fpu_opcode as u32 & 0x7FF)
                | (((self.state.fpu.fip_offset >> 16) & 0xF) << 12);
            self.fpu_write_env_u32(bus, base.wrapping_add(16), ip_hi_opcode)?;
            self.fpu_write_env_u32(
                bus,
                base.wrapping_add(20),
                0xFFFF_0000 | (self.state.fpu.fdp_offset & 0xFFFF),
            )?;
            let dp_hi = ((self.state.fpu.fdp_offset >> 16) & 0xF) << 12;
            self.fpu_write_env_u32(bus, base.wrapping_add(24), dp_hi)
        }
    }

    pub(super) fn fpu_fldenv(&mut self, bus: &mut impl common::Bus) {
        let use_32bit = self.code_segment_32bit() ^ self.operand_size_override;
        let protected = self.is_protected_mode();

        let loaded = if use_32bit {
            self.fpu_fldenv_32bit(protected, bus)
        } else {
            self.fpu_fldenv_16bit(protected, bus)
        };
        if loaded.is_err() {
            return;
        }

        self.fpu_update_es();
        self.clk(Self::timing(71, 44));
    }

    fn fpu_fldenv_16bit(&mut self, protected: bool, bus: &mut impl common::Bus) -> Step {
        // Gather all words first so a faulting read leaves the FPU
        // state untouched.
        let base = self.ea;
        let cw = self.read_word_linear(bus, base)?;
        let sw = self.read_word_linear(bus, base.wrapping_add(2))?;
        let tw = self.read_word_linear(bus, base.wrapping_add(4))?;
        let w6 = self.read_word_linear(bus, base.wrapping_add(6))?;
        let w8 = self.read_word_linear(bus, base.wrapping_add(8))?;
        let w10 = self.read_word_linear(bus, base.wrapping_add(10))?;
        let w12 = self.read_word_linear(bus, base.wrapping_add(12))?;

        self.state.fpu.control_word = cw;
        self.state.fpu.status_word = sw;
        self.state.fpu.tag_word = tw;

        if protected {
            self.state.fpu.fip_offset = w6 as u32;
            self.state.fpu.fip_selector = w8;
            self.state.fpu.fdp_offset = w10 as u32;
            self.state.fpu.fdp_selector = w12;
        } else {
            self.state.fpu.fip_offset = w6 as u32 | (((w8 >> 12) as u32 & 0xF) << 16);
            self.state.fpu.fpu_opcode = w8 & 0x7FF;
            self.state.fpu.fdp_offset = w10 as u32 | (((w12 >> 12) as u32 & 0xF) << 16);
        }
        Ok(())
    }

    fn fpu_fldenv_32bit(&mut self, protected: bool, bus: &mut impl common::Bus) -> Step {
        let base = self.ea;
        let cw_dw = self.fpu_read_env_u32(bus, base)?;
        let sw_dw = self.fpu_read_env_u32(bus, base.wrapping_add(4))?;
        let tw_dw = self.fpu_read_env_u32(bus, base.wrapping_add(8))?;
        let dw12 = self.fpu_read_env_u32(bus, base.wrapping_add(12))?;
        let dw16 = self.fpu_read_env_u32(bus, base.wrapping_add(16))?;
        let dw20 = self.fpu_read_env_u32(bus, base.wrapping_add(20))?;
        let dw24 = self.fpu_read_env_u32(bus, base.wrapping_add(24))?;

        self.state.fpu.control_word = cw_dw as u16;
        self.state.fpu.status_word = sw_dw as u16;
        self.state.fpu.tag_word = tw_dw as u16;

        if protected {
            self.state.fpu.fip_offset = dw12;
            self.state.fpu.fip_selector = dw16 as u16;
            self.state.fpu.fpu_opcode = ((dw16 >> 16) & 0x7FF) as u16;
            self.state.fpu.fdp_offset = dw20;
            self.state.fpu.fdp_selector = dw24 as u16;
        } else {
            self.state.fpu.fip_offset = (dw12 & 0xFFFF) | (((dw16 >> 12) & 0xF) << 16);
            self.state.fpu.fpu_opcode = (dw16 & 0x7FF) as u16;
            self.state.fpu.fdp_offset = (dw20 & 0xFFFF) | (((dw24 >> 12) & 0xF) << 16);
        }
        Ok(())
    }

    pub(super) fn fpu_fnsave(&mut self, bus: &mut impl common::Bus) -> Step {
        let use_32bit = self.code_segment_32bit() ^ self.operand_size_override;
        let protected = self.is_protected_mode();

        let env_saved = if use_32bit {
            self.fpu_fnstenv_32bit(protected, bus)
        } else {
            self.fpu_fnstenv_16bit(protected, bus)
        };
        if env_saved.is_err() {
            return Ok(());
        }

        let env_size: u32 = if use_32bit { 28 } else { 14 };
        let reg_base = self.ea.wrapping_add(env_size);

        // Save all 8 registers in physical order R0-R7. Translate every
        // byte first so a fault in the middle leaves the FPU stack
        // unmodified (FNSAVE re-inits the FPU only on full success).
        let mut phys = [0u32; 80];
        for i in 0..80u32 {
            let p = self.translate_linear(reg_base.wrapping_add(i), true, bus)?;
            phys[i as usize] = p;
        }
        for reg_idx in 0..8u32 {
            let val = self.state.fpu.registers[reg_idx as usize];
            let bytes = val.to_le_bytes();
            for (j, &byte) in bytes.iter().enumerate() {
                bus.write_byte(phys[(reg_idx * 10 + j as u32) as usize], byte);
            }
        }

        // FSAVE reinitializes the FPU - only on full success.
        self.fpu_init();
        self.clk(Self::timing(375, 154));
        Ok(())
    }

    pub(super) fn fpu_frstor(&mut self, bus: &mut impl common::Bus) -> Step {
        let use_32bit = self.code_segment_32bit() ^ self.operand_size_override;
        let protected = self.is_protected_mode();

        // Gather all 80 register bytes via translate_linear before any
        // commit, so a faulting load leaves both the env and register
        // stack at their previous values.
        let env_size: u32 = if use_32bit { 28 } else { 14 };
        let reg_base = self.ea.wrapping_add(env_size);
        let mut phys = [0u32; 80];
        for i in 0..80u32 {
            let p = self.translate_linear(reg_base.wrapping_add(i), false, bus)?;
            phys[i as usize] = p;
        }
        let mut new_regs = [Fp80::ZERO; 8];
        for reg_idx in 0..8u32 {
            let mut bytes = [0u8; 10];
            for (j, byte) in bytes.iter_mut().enumerate() {
                *byte = bus.read_byte(phys[(reg_idx * 10 + j as u32) as usize]);
            }
            new_regs[reg_idx as usize] = Fp80::from_le_bytes(bytes);
        }

        // Now load the env. fpu_fldenv_* gather their reads internally
        // before committing, so a fault here leaves FPU state untouched.
        let env_loaded = if use_32bit {
            self.fpu_fldenv_32bit(protected, bus)
        } else {
            self.fpu_fldenv_16bit(protected, bus)
        };
        if env_loaded.is_err() {
            return Ok(());
        }

        // All gathers succeeded; commit the FPU register stack.
        self.state.fpu.registers = new_regs;

        self.fpu_update_es();
        self.clk(Self::timing(308, 131));
        Ok(())
    }
}
