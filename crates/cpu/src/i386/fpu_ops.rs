use softfloat::{ExceptionFlags, Fp80, FpClass, FpOrdering, Precision, RoundingMode};

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
        let mut ef = ExceptionFlags::default();
        let fp = Fp80::from_f32(f32::from_bits(raw), &mut ef);
        self.fpu_push_converted(fp, &ef);
        self.clk(Self::timing(20, 3));
        Ok(())
    }

    pub(super) fn fpu_fld_m64(&mut self, bus: &mut impl common::Bus) -> Step {
        let raw = self.fpu_read_u64(bus)?;
        let mut ef = ExceptionFlags::default();
        let fp = Fp80::from_f64(f64::from_bits(raw), &mut ef);
        self.fpu_push_converted(fp, &ef);
        self.clk(Self::timing(25, 3));
        Ok(())
    }

    /// Pushes `value`, converted from a memory operand with the exceptions `ef`. An
    /// unmasked invalid operation leaves the stack unchanged (80486 PRM 16.2.2.2).
    /// Observed behaviour: an unmasked denormal operand exception still pushes the
    /// normalized value.
    fn fpu_push_converted(&mut self, value: Fp80, ef: &ExceptionFlags) {
        if self.fpu_is_empty(7) && ef.invalid && !self.fpu_invalid_masked() {
            self.fpu_record_suppressed(ef);
            return;
        }
        self.fpu_check_result(ef);
        self.fpu_push(value);
    }

    pub(super) fn fpu_fld_m80(&mut self, bus: &mut impl common::Bus) -> Step {
        let bytes = self.fpu_read_tbyte(bus)?;
        self.fpu_clear_c1();
        let fp = Fp80::from_le_bytes(bytes);
        self.fpu_push(fp);
        self.clk(Self::timing(44, 6));
        Ok(())
    }

    pub(super) fn fpu_fld_sti(&mut self, i: u8) {
        self.fpu_clear_c1();
        if self.fpu_check_underflow(i) {
            if self.fpu_invalid_masked() {
                self.fpu_push(Fp80::INDEFINITE);
            }
            self.clk(Self::timing(14, 4));
            return;
        }
        let val = self.fpu_st(i);
        self.fpu_push(val);
        self.clk(Self::timing(14, 4));
    }

    /// Stores ST(0) as a single real. Returns whether the store completed: an empty ST(0)
    /// or an unmasked exception with no stored result returns false.
    pub(super) fn fpu_fst_m32(&mut self, bus: &mut impl common::Bus) -> Step<bool> {
        if self.fpu_is_empty(0) {
            let masked = self.fpu_invalid_masked();
            if masked {
                let bits = Fp80::INDEFINITE
                    .to_f32(self.fpu_rounding_mode(), &mut ExceptionFlags::default());
                self.fpu_write_u32(bus, bits.to_bits())?;
            }
            self.fpu_check_underflow(0);
            self.clk(Self::timing(44, 7));
            return Ok(masked);
        }
        let val = self.fpu_st(0);
        let mut ef = ExceptionFlags::default();
        let f = val.to_f32(self.fpu_rounding_mode(), &mut ef);
        if self.fpu_store_suppressed(&ef) {
            self.fpu_record_store_suppressed(&ef);
            self.clk(Self::timing(44, 7));
            return Ok(false);
        }
        self.fpu_write_u32(bus, f.to_bits())?;
        self.fpu_check_result(&ef);
        self.clk(Self::timing(44, 7));
        Ok(true)
    }

    /// Stores ST(0) as a double real. Returns whether the store completed: an empty ST(0)
    /// or an unmasked exception with no stored result returns false.
    pub(super) fn fpu_fst_m64(&mut self, bus: &mut impl common::Bus) -> Step<bool> {
        if self.fpu_is_empty(0) {
            let masked = self.fpu_invalid_masked();
            if masked {
                let bits = Fp80::INDEFINITE
                    .to_f64(self.fpu_rounding_mode(), &mut ExceptionFlags::default());
                self.fpu_write_u64(bus, bits.to_bits())?;
            }
            self.fpu_check_underflow(0);
            self.clk(Self::timing(45, 8));
            return Ok(masked);
        }
        let val = self.fpu_st(0);
        let mut ef = ExceptionFlags::default();
        let f = val.to_f64(self.fpu_rounding_mode(), &mut ef);
        if self.fpu_store_suppressed(&ef) {
            self.fpu_record_store_suppressed(&ef);
            self.clk(Self::timing(45, 8));
            return Ok(false);
        }
        self.fpu_write_u64(bus, f.to_bits())?;
        self.fpu_check_result(&ef);
        self.clk(Self::timing(45, 8));
        Ok(true)
    }

    /// Copies ST(0) to ST(i). Returns false for an empty ST(0) with the invalid operation
    /// exception unmasked.
    pub(super) fn fpu_fst_sti(&mut self, i: u8) -> bool {
        self.fpu_clear_c1();
        self.clk(Self::timing(11, 3));
        if self.fpu_check_underflow(0) {
            let masked = self.fpu_invalid_masked();
            if masked {
                self.fpu_write_st(i, Fp80::INDEFINITE);
            }
            return masked;
        }
        let val = self.fpu_st(0);
        self.fpu_write_st(i, val);
        true
    }

    pub(super) fn fpu_fstp_m32(&mut self, bus: &mut impl common::Bus) -> Step {
        if self.fpu_fst_m32(bus)? {
            self.fpu_pop();
        }
        Ok(())
    }

    pub(super) fn fpu_fstp_m64(&mut self, bus: &mut impl common::Bus) -> Step {
        if self.fpu_fst_m64(bus)? {
            self.fpu_pop();
        }
        Ok(())
    }

    pub(super) fn fpu_fstp_m80(&mut self, bus: &mut impl common::Bus) {
        if self.fpu_is_empty(0) {
            let masked = self.fpu_invalid_masked();
            if masked
                && self
                    .fpu_write_tbyte(bus, &Fp80::INDEFINITE.to_le_bytes())
                    .is_err()
            {
                return;
            }
            self.fpu_check_underflow(0);
            if masked {
                self.fpu_pop();
            }
            self.clk(Self::timing(53, 6));
            return;
        }
        let val = self.fpu_st(0);
        if self.fpu_write_tbyte(bus, &val.to_le_bytes()).is_err() {
            return;
        }
        self.fpu_clear_c1();
        self.fpu_pop();
        self.clk(Self::timing(53, 6));
    }

    pub(super) fn fpu_fstp_sti(&mut self, i: u8) {
        if self.fpu_fst_sti(i) {
            self.fpu_pop();
        }
    }

    pub(super) fn fpu_fild_m16(&mut self, bus: &mut impl common::Bus) -> Step {
        let raw = self.fpu_read_u16(bus)?;
        self.fpu_clear_c1();
        let fp = Fp80::from_i16(raw as i16);
        self.fpu_push(fp);
        self.clk(Self::timing(61, 13));
        Ok(())
    }

    pub(super) fn fpu_fild_m32(&mut self, bus: &mut impl common::Bus) -> Step {
        let raw = self.fpu_read_u32(bus)?;
        self.fpu_clear_c1();
        let fp = Fp80::from_i32(raw as i32);
        self.fpu_push(fp);
        self.clk(Self::timing(45, 9));
        Ok(())
    }

    pub(super) fn fpu_fild_m64(&mut self, bus: &mut impl common::Bus) -> Step {
        let raw = self.fpu_read_u64(bus)?;
        self.fpu_clear_c1();
        let fp = Fp80::from_i64(raw as i64);
        self.fpu_push(fp);
        self.clk(Self::timing(56, 10));
        Ok(())
    }

    /// Stores ST(0) as a word integer. Returns whether the store completed: an empty ST(0)
    /// or an unmasked invalid operation with no stored result returns false.
    pub(super) fn fpu_fist_m16(&mut self, bus: &mut impl common::Bus) -> Step<bool> {
        if self.fpu_is_empty(0) {
            let masked = self.fpu_invalid_masked();
            if masked {
                self.fpu_write_u16(bus, 0x8000u16)?;
            }
            self.fpu_check_underflow(0);
            self.clk(Self::timing(82, 29));
            return Ok(masked);
        }
        let val = self.fpu_st(0);
        let mut ef = ExceptionFlags::default();
        let i = val.to_i16(self.fpu_rounding_mode(), &mut ef);
        if self.fpu_store_suppressed(&ef) {
            self.fpu_record_store_suppressed(&ef);
            self.clk(Self::timing(82, 29));
            return Ok(false);
        }
        self.fpu_write_u16(bus, i as u16)?;
        self.fpu_check_result(&ef);
        self.clk(Self::timing(82, 29));
        Ok(true)
    }

    /// Stores ST(0) as a short integer. Returns whether the store completed: an empty
    /// ST(0) or an unmasked invalid operation with no stored result returns false.
    pub(super) fn fpu_fist_m32(&mut self, bus: &mut impl common::Bus) -> Step<bool> {
        if self.fpu_is_empty(0) {
            let masked = self.fpu_invalid_masked();
            if masked {
                self.fpu_write_u32(bus, 0x8000_0000u32)?;
            }
            self.fpu_check_underflow(0);
            self.clk(Self::timing(79, 28));
            return Ok(masked);
        }
        let val = self.fpu_st(0);
        let mut ef = ExceptionFlags::default();
        let i = val.to_i32(self.fpu_rounding_mode(), &mut ef);
        if self.fpu_store_suppressed(&ef) {
            self.fpu_record_store_suppressed(&ef);
            self.clk(Self::timing(79, 28));
            return Ok(false);
        }
        self.fpu_write_u32(bus, i as u32)?;
        self.fpu_check_result(&ef);
        self.clk(Self::timing(79, 28));
        Ok(true)
    }

    pub(super) fn fpu_fistp_m16(&mut self, bus: &mut impl common::Bus) -> Step {
        if self.fpu_fist_m16(bus)? {
            self.fpu_pop();
        }
        Ok(())
    }

    pub(super) fn fpu_fistp_m32(&mut self, bus: &mut impl common::Bus) -> Step {
        if self.fpu_fist_m32(bus)? {
            self.fpu_pop();
        }
        Ok(())
    }

    pub(super) fn fpu_fistp_m64(&mut self, bus: &mut impl common::Bus) {
        if self.fpu_is_empty(0) {
            let masked = self.fpu_invalid_masked();
            if masked && self.fpu_write_u64(bus, 0x8000_0000_0000_0000u64).is_err() {
                return;
            }
            self.fpu_check_underflow(0);
            if masked {
                self.fpu_pop();
            }
            self.clk(Self::timing(80, 28));
            return;
        }
        let val = self.fpu_st(0);
        let mut ef = ExceptionFlags::default();
        let i = val.to_i64(self.fpu_rounding_mode(), &mut ef);
        if self.fpu_store_suppressed(&ef) {
            self.fpu_record_store_suppressed(&ef);
            self.clk(Self::timing(80, 28));
            return;
        }
        if self.fpu_write_u64(bus, i as u64).is_err() {
            return;
        }
        self.fpu_check_result(&ef);
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
        if self.fpu_is_empty(0) {
            let masked = self.fpu_invalid_masked();
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
            self.fpu_check_underflow(0);
            if masked {
                self.fpu_pop();
            }
            self.clk(Self::timing(512, 172));
            return;
        }
        let val = self.fpu_st(0);
        let mut ef = ExceptionFlags::default();
        let bcd = val.to_bcd(self.fpu_rounding_mode(), &mut ef);
        if self.fpu_store_suppressed(&ef) {
            self.fpu_record_store_suppressed(&ef);
            self.clk(Self::timing(512, 172));
            return;
        }
        if self.fpu_write_tbyte(bus, &bcd).is_err() {
            return;
        }
        self.fpu_check_result(&ef);
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
        self.fpu_clear_c1();
        self.fpu_push(Fp80::ONE);
        self.clk(Self::timing(24, 4));
    }

    pub(super) fn fpu_fldz(&mut self) {
        self.fpu_clear_c1();
        self.fpu_push(Fp80::ZERO);
        self.clk(Self::timing(20, 4));
    }

    pub(super) fn fpu_fldl2t(&mut self) {
        self.fpu_clear_c1();
        let rc = self.fpu_rounding_mode();
        let val = match rc {
            RoundingMode::Up => Fp80::LOG2_10_UP,
            _ => Fp80::LOG2_10_DOWN,
        };
        self.fpu_push(val);
        self.clk(Self::timing(40, 8));
    }

    pub(super) fn fpu_fldl2e(&mut self) {
        self.fpu_clear_c1();
        let rc = self.fpu_rounding_mode();
        let val = match rc {
            RoundingMode::Up | RoundingMode::NearestEven => Fp80::LOG2_E_UP,
            _ => Fp80::LOG2_E_DOWN,
        };
        self.fpu_push(val);
        self.clk(Self::timing(40, 8));
    }

    pub(super) fn fpu_fldpi(&mut self) {
        self.fpu_clear_c1();
        let rc = self.fpu_rounding_mode();
        let val = match rc {
            RoundingMode::Up | RoundingMode::NearestEven => Fp80::PI_UP,
            _ => Fp80::PI_DOWN,
        };
        self.fpu_push(val);
        self.clk(Self::timing(40, 8));
    }

    pub(super) fn fpu_fldlg2(&mut self) {
        self.fpu_clear_c1();
        let rc = self.fpu_rounding_mode();
        let val = match rc {
            RoundingMode::Up | RoundingMode::NearestEven => Fp80::LOG10_2_UP,
            _ => Fp80::LOG10_2_DOWN,
        };
        self.fpu_push(val);
        self.clk(Self::timing(41, 8));
    }

    pub(super) fn fpu_fldln2(&mut self) {
        self.fpu_clear_c1();
        let rc = self.fpu_rounding_mode();
        let val = match rc {
            RoundingMode::Up | RoundingMode::NearestEven => Fp80::LN_2_UP,
            _ => Fp80::LN_2_DOWN,
        };
        self.fpu_push(val);
        self.clk(Self::timing(41, 8));
    }

    /// Computes ST(dest) = op(ST(dest), ST(src)). Returns whether a result was delivered:
    /// a stack underflow or an unmasked exception without a result returns false.
    fn fpu_arith_reg_reg(
        &mut self,
        dest: u8,
        src: u8,
        op: fn(Fp80, Fp80, RoundingMode, Precision, &mut ExceptionFlags) -> Fp80,
        t387: i32,
        t486: i32,
    ) -> bool {
        self.clk(Self::timing(t387, t486));
        if self.fpu_check_underflow(dest) || self.fpu_check_underflow(src) {
            let masked = self.fpu_invalid_masked();
            if masked {
                self.fpu_write_st(dest, Fp80::INDEFINITE);
            }
            return masked;
        }
        let a = self.fpu_st(dest);
        let b = self.fpu_st(src);
        let rc = self.fpu_rounding_mode();
        let pc = self.fpu_precision();
        let mut ef = ExceptionFlags::default();
        let result = op(a, b, rc, pc, &mut ef);
        self.fpu_deliver(dest, &ef, result)
    }

    /// Writes `result` with the exceptions `ef` to ST(dest) through `fpu_register_result`.
    /// Returns whether the result was delivered.
    fn fpu_deliver(&mut self, dest: u8, ef: &ExceptionFlags, result: Fp80) -> bool {
        match self.fpu_register_result(ef, result) {
            Some(value) => {
                self.fpu_write_st(dest, value);
                true
            }
            None => false,
        }
    }

    /// Computes ST(0) = op(ST(0), operand) for the memory operand `operand`, converted with
    /// the exceptions `conversion`. Observed behaviour: the operand read comes before the
    /// stack underflow check, so a faulting operand raises its fault with an empty ST(0).
    fn fpu_arith_memory(
        &mut self,
        operand: Fp80,
        conversion: ExceptionFlags,
        op: fn(Fp80, Fp80, RoundingMode, Precision, &mut ExceptionFlags) -> Fp80,
        t387: i32,
        t486: i32,
    ) {
        self.clk(Self::timing(t387, t486));
        if self.fpu_check_underflow(0) {
            if self.fpu_invalid_masked() {
                self.fpu_write_st(0, Fp80::INDEFINITE);
            }
            return;
        }
        let a = self.fpu_st(0);
        let rc = self.fpu_rounding_mode();
        let pc = self.fpu_precision();
        let mut ef = ExceptionFlags::default();
        let result = op(a, operand, rc, pc, &mut ef);
        Self::fpu_merge_conversion(&mut ef, &conversion, a, operand);
        self.fpu_deliver(0, &ef, result);
    }

    /// Adds the exceptions `conversion` of a memory operand conversion to the exceptions
    /// `ef` of the operation on `a` and the converted `b`. The denormal operand exception
    /// of the conversion is not reported when a NaN operand, an invalid operation or a zero
    /// divide takes priority (80486 PRM 16.2.7).
    fn fpu_merge_conversion(
        ef: &mut ExceptionFlags,
        conversion: &ExceptionFlags,
        a: Fp80,
        b: Fp80,
    ) {
        ef.invalid |= conversion.invalid;
        let higher_priority = a.is_nan() || b.is_nan() || ef.invalid || ef.zero_divide;
        if conversion.denormal && !higher_priority {
            ef.denormal = true;
        }
    }

    fn fpu_arith_m32(
        &mut self,
        op: fn(Fp80, Fp80, RoundingMode, Precision, &mut ExceptionFlags) -> Fp80,
        t387: i32,
        t486: i32,
        bus: &mut impl common::Bus,
    ) -> Step {
        let raw = self.fpu_read_u32(bus)?;
        let mut ef = ExceptionFlags::default();
        let operand = Fp80::from_f32(f32::from_bits(raw), &mut ef);
        self.fpu_arith_memory(operand, ef, op, t387, t486);
        Ok(())
    }

    fn fpu_arith_m64(
        &mut self,
        op: fn(Fp80, Fp80, RoundingMode, Precision, &mut ExceptionFlags) -> Fp80,
        t387: i32,
        t486: i32,
        bus: &mut impl common::Bus,
    ) -> Step {
        let raw = self.fpu_read_u64(bus)?;
        let mut ef = ExceptionFlags::default();
        let operand = Fp80::from_f64(f64::from_bits(raw), &mut ef);
        self.fpu_arith_memory(operand, ef, op, t387, t486);
        Ok(())
    }

    fn fpu_arith_mi32(
        &mut self,
        op: fn(Fp80, Fp80, RoundingMode, Precision, &mut ExceptionFlags) -> Fp80,
        t387: i32,
        t486: i32,
        bus: &mut impl common::Bus,
    ) -> Step {
        let raw = self.fpu_read_u32(bus)?;
        let operand = Fp80::from_i32(raw as i32);
        self.fpu_arith_memory(operand, ExceptionFlags::default(), op, t387, t486);
        Ok(())
    }

    fn fpu_arith_mi16(
        &mut self,
        op: fn(Fp80, Fp80, RoundingMode, Precision, &mut ExceptionFlags) -> Fp80,
        t387: i32,
        t486: i32,
        bus: &mut impl common::Bus,
    ) -> Step {
        let raw = self.fpu_read_u16(bus)?;
        let operand = Fp80::from_i16(raw as i16);
        self.fpu_arith_memory(operand, ExceptionFlags::default(), op, t387, t486);
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
        if self.fpu_arith_reg_reg(i, 0, Fp80::add, 23, 8) {
            self.fpu_pop();
        }
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
        if self.fpu_arith_reg_reg(i, 0, Fp80::sub, 26, 8) {
            self.fpu_pop();
        }
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
        if self.fpu_arith_reg_reg(i, 0, Self::fpu_subr, 26, 8) {
            self.fpu_pop();
        }
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
        if self.fpu_arith_reg_reg(i, 0, Fp80::mul, 29, 16) {
            self.fpu_pop();
        }
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
        if self.fpu_arith_reg_reg(i, 0, Fp80::div, 91, 73) {
            self.fpu_pop();
        }
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
        if self.fpu_arith_reg_reg(i, 0, Self::fpu_divr, 91, 73) {
            self.fpu_pop();
        }
    }

    pub(super) fn fpu_fidivr_m32(&mut self, bus: &mut impl common::Bus) -> Step {
        self.fpu_arith_mi32(Self::fpu_divr, 121, 84, bus)?;
        Ok(())
    }

    pub(super) fn fpu_fidivr_m16(&mut self, bus: &mut impl common::Bus) -> Step {
        self.fpu_arith_mi16(Self::fpu_divr, 135, 85, bus)?;
        Ok(())
    }

    /// Applies the unary operation `op` to ST(0) with the stack underflow check.
    fn fpu_unary(
        &mut self,
        op: impl FnOnce(Fp80, &mut ExceptionFlags) -> Fp80,
        t387: i32,
        t486: i32,
    ) {
        self.clk(Self::timing(t387, t486));
        if self.fpu_check_underflow(0) {
            if self.fpu_invalid_masked() {
                self.fpu_write_st(0, Fp80::INDEFINITE);
            }
            return;
        }
        let mut ef = ExceptionFlags::default();
        let result = op(self.fpu_st(0), &mut ef);
        self.fpu_deliver(0, &ef, result);
    }

    pub(super) fn fpu_fsqrt(&mut self) {
        let rc = self.fpu_rounding_mode();
        let pc = self.fpu_precision();
        self.fpu_unary(|value, ef| value.sqrt(rc, pc, ef), 122, 83);
    }

    pub(super) fn fpu_fabs(&mut self) {
        if self.fpu_check_underflow(0) {
            if self.fpu_invalid_masked() {
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
            if self.fpu_invalid_masked() {
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
        let rc = self.fpu_rounding_mode();
        self.fpu_unary(|value, ef| value.round_to_int(rc, ef), 66, 21);
    }

    /// Checks ST(0) and ST(1) for a stack underflow. Returns true when both hold a value.
    /// An underflow with the invalid operation exception masked writes the QNaN indefinite
    /// to ST(`dest`).
    fn fpu_check_two_operands(&mut self, dest: u8) -> bool {
        if self.fpu_check_underflow(0) || self.fpu_check_underflow(1) {
            if self.fpu_invalid_masked() {
                self.fpu_write_st(dest, Fp80::INDEFINITE);
            }
            return false;
        }
        true
    }

    pub(super) fn fpu_fscale(&mut self) {
        self.clk(Self::timing(67, 30));
        if !self.fpu_check_two_operands(0) {
            return;
        }
        let a = self.fpu_st(0);
        let b = self.fpu_st(1);
        let mut ef = ExceptionFlags::default();
        let result = a.scale(b, self.fpu_rounding_mode(), &mut ef);
        self.fpu_deliver(0, &ef, result);
    }

    pub(super) fn fpu_fxtract(&mut self) {
        self.clk(Self::timing(70, 16));
        if self.fpu_check_underflow(0) {
            if self.fpu_invalid_masked() {
                self.fpu_write_st(0, Fp80::INDEFINITE);
                self.fpu_push(Fp80::INDEFINITE);
            }
            return;
        }
        if !self.fpu_check_unmasked_overflow() || self.fpu_masked_push_overflow() {
            return;
        }
        let a = self.fpu_st(0);
        let mut ef = ExceptionFlags::default();
        let (significand, exponent) = a.extract(&mut ef);
        if self.fpu_operation_suppressed(&ef) {
            self.fpu_record_suppressed(&ef);
            return;
        }
        self.fpu_check_result(&ef);
        self.fpu_write_st(0, exponent);
        self.fpu_push(significand);
    }

    pub(super) fn fpu_fprem(&mut self) {
        self.clk(Self::timing(74, 70));
        if !self.fpu_check_two_operands(0) {
            return;
        }
        let a = self.fpu_st(0);
        let b = self.fpu_st(1);
        let mut ef = ExceptionFlags::default();
        let (result, quotient_bits, complete) = a.partial_remainder(b, &mut ef);
        self.fpu_deliver_remainder(&ef, result, quotient_bits, complete);
    }

    pub(super) fn fpu_fprem1(&mut self) {
        self.clk(Self::timing(95, 72));
        if !self.fpu_check_two_operands(0) {
            return;
        }
        let a = self.fpu_st(0);
        let b = self.fpu_st(1);
        let mut ef = ExceptionFlags::default();
        let (result, quotient_bits, complete) = a.ieee_remainder(b, &mut ef);
        self.fpu_deliver_remainder(&ef, result, quotient_bits, complete);
    }

    /// Writes the partial remainder `result` to ST(0) and sets the condition codes: C2 for
    /// an incomplete reduction and C0, C3 and C1 to the quotient bits Q2, Q1 and Q0 (80486
    /// PRM, FPREM). Observed behaviour: an unmasked exception without a result clears C2
    /// and keeps C0 and C3.
    fn fpu_deliver_remainder(
        &mut self,
        ef: &ExceptionFlags,
        result: Fp80,
        quotient_bits: u8,
        complete: bool,
    ) {
        if !self.fpu_deliver(0, ef, result) {
            self.fpu_clear_c2();
            return;
        }
        let c2 = !complete;
        let c0 = quotient_bits & 4 != 0;
        let c3 = quotient_bits & 2 != 0;
        let c1 = quotient_bits & 1 != 0;
        self.fpu_set_cc(c3, c2, c1, c0);
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

    /// Compares ST(0) with `operand`, converted with the exceptions `conversion`, and sets
    /// the condition codes. `quiet` selects the unordered compare. Returns whether the
    /// compare completed: a stack underflow or an unmasked exception returns false, and the
    /// popping forms then leave TOP unchanged (80486 PRM 16.2.2). Observed behaviour: the
    /// condition codes are set in every case, also when an unmasked invalid operation or
    /// denormal operand exception stops the compare.
    fn fpu_compare_st0(&mut self, operand: Option<(Fp80, ExceptionFlags)>, quiet: bool) -> bool {
        if self.fpu_check_underflow(0) {
            self.fpu_compare_and_set_cc(FpOrdering::Unordered);
            return self.fpu_invalid_masked();
        }
        let Some((b, conversion)) = operand else {
            self.fpu_compare_and_set_cc(FpOrdering::Unordered);
            return self.fpu_invalid_masked();
        };
        let a = self.fpu_st(0);
        let mut ef = ExceptionFlags::default();
        let ordering = if quiet {
            a.compare_quiet(b, &mut ef)
        } else {
            a.compare(b, &mut ef)
        };
        Self::fpu_merge_conversion(&mut ef, &conversion, a, b);
        let completed = if self.fpu_operation_suppressed(&ef) {
            self.fpu_record_suppressed(&ef);
            false
        } else {
            self.fpu_check_result(&ef);
            true
        };
        self.fpu_compare_and_set_cc(ordering);
        completed
    }

    /// Returns ST(i) for a register compare, or `None` after a stack underflow of ST(0) or
    /// ST(i).
    fn fpu_compare_register(&mut self, i: u8) -> Option<(Fp80, ExceptionFlags)> {
        if self.fpu_check_underflow(0) || self.fpu_check_underflow(i) {
            return None;
        }
        Some((self.fpu_st(i), ExceptionFlags::default()))
    }

    pub(super) fn fpu_fcom_m32(&mut self, bus: &mut impl common::Bus) -> Step<bool> {
        let raw = self.fpu_read_u32(bus)?;
        let mut ef = ExceptionFlags::default();
        let b = Fp80::from_f32(f32::from_bits(raw), &mut ef);
        self.clk(Self::timing(26, 4));
        Ok(self.fpu_compare_st0(Some((b, ef)), false))
    }

    pub(super) fn fpu_fcom_m64(&mut self, bus: &mut impl common::Bus) -> Step<bool> {
        let raw = self.fpu_read_u64(bus)?;
        let mut ef = ExceptionFlags::default();
        let b = Fp80::from_f64(f64::from_bits(raw), &mut ef);
        self.clk(Self::timing(31, 4));
        Ok(self.fpu_compare_st0(Some((b, ef)), false))
    }

    pub(super) fn fpu_fcom_sti(&mut self, i: u8) -> bool {
        self.clk(Self::timing(24, 4));
        let operand = self.fpu_compare_register(i);
        self.fpu_compare_st0(operand, false)
    }

    pub(super) fn fpu_fcomp_m32(&mut self, bus: &mut impl common::Bus) -> Step {
        if self.fpu_fcom_m32(bus)? {
            self.fpu_pop();
        }
        Ok(())
    }

    pub(super) fn fpu_fcomp_m64(&mut self, bus: &mut impl common::Bus) -> Step {
        if self.fpu_fcom_m64(bus)? {
            self.fpu_pop();
        }
        Ok(())
    }

    pub(super) fn fpu_fcomp_sti(&mut self, i: u8) {
        if self.fpu_fcom_sti(i) {
            self.fpu_pop();
        }
    }

    pub(super) fn fpu_fcompp(&mut self) {
        self.clk(Self::timing(26, 5));
        let operand = self.fpu_compare_register(1);
        if self.fpu_compare_st0(operand, false) {
            self.fpu_pop();
            self.fpu_pop();
        }
    }

    pub(super) fn fpu_fucom_sti(&mut self, i: u8) -> bool {
        self.clk(Self::timing(24, 4));
        let operand = self.fpu_compare_register(i);
        self.fpu_compare_st0(operand, true)
    }

    pub(super) fn fpu_fucomp_sti(&mut self, i: u8) {
        if self.fpu_fucom_sti(i) {
            self.fpu_pop();
        }
    }

    pub(super) fn fpu_fucompp(&mut self) {
        self.clk(Self::timing(26, 5));
        let operand = self.fpu_compare_register(1);
        if self.fpu_compare_st0(operand, true) {
            self.fpu_pop();
            self.fpu_pop();
        }
    }

    pub(super) fn fpu_ficom_m32(&mut self, bus: &mut impl common::Bus) -> Step<bool> {
        let raw = self.fpu_read_u32(bus)?;
        let b = Fp80::from_i32(raw as i32);
        self.clk(Self::timing(56, 15));
        Ok(self.fpu_compare_st0(Some((b, ExceptionFlags::default())), false))
    }

    pub(super) fn fpu_ficom_m16(&mut self, bus: &mut impl common::Bus) -> Step<bool> {
        let raw = self.fpu_read_u16(bus)?;
        let b = Fp80::from_i16(raw as i16);
        self.clk(Self::timing(71, 16));
        Ok(self.fpu_compare_st0(Some((b, ExceptionFlags::default())), false))
    }

    pub(super) fn fpu_ficomp_m32(&mut self, bus: &mut impl common::Bus) -> Step {
        if self.fpu_ficom_m32(bus)? {
            self.fpu_pop();
        }
        Ok(())
    }

    pub(super) fn fpu_ficomp_m16(&mut self, bus: &mut impl common::Bus) -> Step {
        if self.fpu_ficom_m16(bus)? {
            self.fpu_pop();
        }
        Ok(())
    }

    pub(super) fn fpu_ftst(&mut self) {
        self.clk(Self::timing(28, 4));
        self.fpu_compare_st0(Some((Fp80::ZERO, ExceptionFlags::default())), false);
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
        self.fpu_unary(|value, ef| value.f2xm1(ef), 211, 140);
    }

    /// Computes ST(1) = op(ST(0), ST(1)) and pops. A stack underflow or an unmasked
    /// exception without a result leaves TOP unchanged.
    fn fpu_binary_pop(
        &mut self,
        op: impl FnOnce(Fp80, Fp80, &mut ExceptionFlags) -> Fp80,
        t387: i32,
        t486: i32,
    ) {
        self.clk(Self::timing(t387, t486));
        if !self.fpu_check_two_operands(1) {
            if self.fpu_invalid_masked() {
                self.fpu_pop();
            }
            return;
        }
        let x = self.fpu_st(0);
        let y = self.fpu_st(1);
        let mut ef = ExceptionFlags::default();
        let result = op(x, y, &mut ef);
        if self.fpu_deliver(1, &ef, result) {
            self.fpu_pop();
        }
    }

    pub(super) fn fpu_fyl2x(&mut self) {
        self.fpu_binary_pop(|x, y, ef| x.fyl2x(y, ef), 120, 196);
    }

    pub(super) fn fpu_fyl2xp1(&mut self) {
        self.fpu_binary_pop(|x, y, ef| x.fyl2xp1(y, ef), 257, 171);
    }

    pub(super) fn fpu_fpatan(&mut self) {
        self.fpu_binary_pop(|x, y, ef| y.fpatan(x, ef), 314, 218);
    }

    /// Runs the one-result trigonometric operation `op` on ST(0). An operand out of range
    /// sets C2 and leaves ST(0) unchanged. Otherwise C2 is cleared, also by an unmasked
    /// exception without a result (observed behaviour).
    fn fpu_trigonometric(
        &mut self,
        op: impl FnOnce(Fp80, &mut ExceptionFlags) -> (Fp80, bool),
        t387: i32,
        t486: i32,
    ) {
        self.clk(Self::timing(t387, t486));
        if self.fpu_check_underflow(0) {
            if self.fpu_invalid_masked() {
                self.fpu_write_st(0, Fp80::INDEFINITE);
            }
            return;
        }
        let mut ef = ExceptionFlags::default();
        let (result, out_of_range) = op(self.fpu_st(0), &mut ef);
        if out_of_range {
            self.state.fpu.status_word |= 0x0400; // C2=1
            return;
        }
        self.fpu_deliver(0, &ef, result);
        self.fpu_clear_c2();
    }

    pub(super) fn fpu_fsin(&mut self) {
        self.fpu_trigonometric(|value, ef| value.fsin(ef), 122, 257);
    }

    pub(super) fn fpu_fcos(&mut self) {
        self.fpu_trigonometric(|value, ef| value.fcos(ef), 123, 257);
    }

    /// Handles a stack overflow of an instruction that replaces ST(0) and pushes a second
    /// result, with the invalid operation exception masked. ST(0) and the pushed register
    /// both receive the QNaN indefinite before any computation (80486 PRM 16.2.2.1 and
    /// 25.2.3, FPTAN). Returns true when ST(7) is in use.
    fn fpu_masked_push_overflow(&mut self) -> bool {
        if self.fpu_is_empty(7) {
            return false;
        }
        self.fpu_write_st(0, Fp80::INDEFINITE);
        self.fpu_push(Fp80::INDEFINITE);
        true
    }

    /// Runs the two-result operation `op` on ST(0): ST(0) receives the first result and
    /// the second result is pushed. An unmasked stack overflow or an unmasked exception
    /// leaves the stack unchanged. A masked stack overflow leaves the QNaN indefinite in
    /// ST(1) and ST(0) and clears C2. An operand out of range sets C2.
    fn fpu_two_results(
        &mut self,
        op: impl FnOnce(Fp80, &mut ExceptionFlags) -> (Fp80, Fp80, bool),
        masked_underflow_push: Fp80,
        t387: i32,
        t486: i32,
    ) {
        self.clk(Self::timing(t387, t486));
        if self.fpu_check_underflow(0) {
            if self.fpu_invalid_masked() {
                self.fpu_write_st(0, Fp80::INDEFINITE);
                self.fpu_push(masked_underflow_push);
            }
            return;
        }
        if !self.fpu_check_unmasked_overflow() {
            return;
        }
        if self.fpu_masked_push_overflow() {
            self.fpu_clear_c2();
            return;
        }
        let mut ef = ExceptionFlags::default();
        let (first, second, out_of_range) = op(self.fpu_st(0), &mut ef);
        if out_of_range {
            self.state.fpu.status_word |= 0x0400; // C2=1
            return;
        }
        self.fpu_clear_c2();
        let Some(first) = self.fpu_register_result(&ef, first) else {
            return;
        };
        self.fpu_write_st(0, first);
        self.fpu_push(second);
    }

    pub(super) fn fpu_fptan(&mut self) {
        self.fpu_two_results(
            |value, ef| {
                let (tangent, out_of_range) = value.fptan(ef);
                (tangent, Fp80::ONE, out_of_range)
            },
            Fp80::ONE,
            191,
            200,
        );
    }

    pub(super) fn fpu_fsincos(&mut self) {
        self.fpu_two_results(|value, ef| value.fsincos(ef), Fp80::INDEFINITE, 194, 292);
    }

    /// Returns whether FSTENV, FLDENV, FSAVE and FRSTOR use the protected mode image. Real
    /// mode and virtual-8086 mode use the real mode image (80486 PRM 15.1.5).
    fn fpu_environment_protected(&self) -> bool {
        self.is_protected_mode() && !self.is_virtual_mode()
    }

    /// Returns the size in bytes of the environment image for the operand size.
    fn fpu_environment_size(&self) -> u32 {
        if self.operand_size_override { 28 } else { 14 }
    }

    /// Returns the tag word with the exact tag of every nonempty register derived from its
    /// contents, as FSTENV and FSAVE store it (80486 PRM 15.1.4).
    fn fpu_tag_word_image(&self) -> u16 {
        let mut tag_word = self.state.fpu.tag_word;
        for physical in 0..8 {
            let shift = physical * 2;
            if (tag_word >> shift) & 3 == 0b11 {
                continue;
            }
            let tag = match self.state.fpu.registers[physical].classify() {
                FpClass::Zero => 0b01,
                FpClass::Normal => 0b00,
                _ => 0b10,
            };
            tag_word = (tag_word & !(3 << shift)) | (tag << shift);
        }
        tag_word
    }

    pub(super) fn fpu_fnstenv(&mut self, bus: &mut impl common::Bus) {
        let env_size = self.fpu_environment_size();
        if self.fpu_check_operand(env_size, true, bus).is_err() {
            return;
        }
        if self.fpu_store_environment(bus).is_err() {
            return;
        }

        // After FSTENV, mask all exceptions. Only runs on full success
        // so a partial-fault FNSTENV leaves the control word untouched.
        self.state.fpu.control_word |= 0x3F;
        self.fpu_update_es();
        self.clk(Self::timing(103, 67));
    }

    /// Writes the environment image in the format of the operand size and the mode
    /// (80486 PRM figures 15-5 to 15-8).
    fn fpu_store_environment(&mut self, bus: &mut impl common::Bus) -> Step {
        let base = self.ea;
        let fields = self.fpu_environment_fields();
        if self.operand_size_override {
            for (index, value) in fields.into_iter().enumerate() {
                self.fpu_write_env_u32(bus, base.wrapping_add(index as u32 * 4), value)?;
            }
        } else {
            for (index, value) in fields.into_iter().enumerate() {
                self.write_word_linear(bus, base.wrapping_add(index as u32 * 2), value as u16)?;
            }
        }
        Ok(())
    }

    /// Returns the seven fields of the environment image for the operand size and the mode.
    /// A 16-bit image stores the low word of each field.
    fn fpu_environment_fields(&self) -> [u32; 7] {
        let fpu = &self.state.fpu;
        let wide = self.operand_size_override;
        let reserved = if wide {
            ENVIRONMENT_RESERVED_HIGH_WORD
        } else {
            0
        };
        let opcode = fpu.fpu_opcode as u32 & ENVIRONMENT_OPCODE_MASK;
        let control = reserved | fpu.control_word as u32;
        let status = reserved | fpu.status_word as u32;
        let tag = reserved | self.fpu_tag_word_image() as u32;

        if self.fpu_environment_protected() {
            // Figures 15-5 and 15-7: offsets and selectors. The opcode in bits 26..16 of the
            // CS selector dword of the 32-bit image is observed behaviour.
            let selector_dword_opcode = if wide {
                opcode << PROTECTED_OPCODE_SHIFT
            } else {
                0
            };
            return [
                control,
                status,
                tag,
                fpu.fip_offset,
                selector_dword_opcode | fpu.fip_selector as u32,
                fpu.fdp_offset,
                reserved | fpu.fdp_selector as u32,
            ];
        }

        // Figures 15-6 and 15-8: the pointers as linear addresses. The linear address
        // selector * 16 + offset is observed behaviour.
        let instruction = real_mode_linear_pointer(fpu.fip_selector, fpu.fip_offset);
        let data = real_mode_linear_pointer(fpu.fdp_selector, fpu.fdp_offset);
        let high_bits = if wide {
            REAL_POINTER_HIGH_BITS_32
        } else {
            REAL_POINTER_HIGH_BITS_16
        };
        [
            control,
            status,
            tag,
            reserved | (instruction & REAL_POINTER_LOW_MASK),
            real_pointer_high_field(instruction, high_bits) | opcode,
            reserved | (data & REAL_POINTER_LOW_MASK),
            real_pointer_high_field(data, high_bits),
        ]
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

    pub(super) fn fpu_fldenv(&mut self, bus: &mut impl common::Bus) {
        let env_size = self.fpu_environment_size();
        if self.fpu_check_operand(env_size, false, bus).is_err() {
            return;
        }
        let Ok(words) = self.fpu_read_environment(bus) else {
            return;
        };
        self.fpu_apply_environment(words);

        self.fpu_update_es();
        self.clk(Self::timing(71, 44));
    }

    /// Reads the seven fields of the environment image in the format of the operand size.
    fn fpu_read_environment(&mut self, bus: &mut impl common::Bus) -> Step<[u32; 7]> {
        let base = self.ea;
        let mut words = [0u32; 7];
        if self.operand_size_override {
            for (index, word) in words.iter_mut().enumerate() {
                *word = self.fpu_read_env_u32(bus, base.wrapping_add(index as u32 * 4))?;
            }
        } else {
            for (index, word) in words.iter_mut().enumerate() {
                *word = self.read_word_linear(bus, base.wrapping_add(index as u32 * 2))? as u32;
            }
        }
        Ok(words)
    }

    /// Loads the environment fields read from an image in the format of the operand size
    /// and the mode (80486 PRM figures 15-5 to 15-8).
    fn fpu_apply_environment(&mut self, fields: [u32; 7]) {
        let wide = self.operand_size_override;
        let protected = self.fpu_environment_protected();
        let fpu = &mut self.state.fpu;
        fpu.control_word = fields[0] as u16;
        fpu.status_word = fields[1] as u16;
        fpu.tag_word = fields[2] as u16;

        if protected {
            let offset_mask = if wide { u32::MAX } else { 0xFFFF };
            fpu.fip_offset = fields[3] & offset_mask;
            fpu.fip_selector = fields[4] as u16;
            fpu.fdp_offset = fields[5] & offset_mask;
            fpu.fdp_selector = fields[6] as u16;
            // Observed behaviour: the 16-bit image, which has no opcode field, loads the
            // opcode as zero.
            fpu.fpu_opcode = if wide {
                ((fields[4] >> PROTECTED_OPCODE_SHIFT) & ENVIRONMENT_OPCODE_MASK) as u16
            } else {
                0
            };
            return;
        }

        // Observed behaviour: the loaded linear pointers become the offsets, and the
        // selectors read as zero.
        let high_bits = if wide {
            REAL_POINTER_HIGH_BITS_32
        } else {
            REAL_POINTER_HIGH_BITS_16
        };
        fpu.fip_offset = (fields[3] & REAL_POINTER_LOW_MASK)
            | real_pointer_from_high_field(fields[4], high_bits);
        fpu.fdp_offset = (fields[5] & REAL_POINTER_LOW_MASK)
            | real_pointer_from_high_field(fields[6], high_bits);
        fpu.fpu_opcode = (fields[4] & ENVIRONMENT_OPCODE_MASK) as u16;
        fpu.fip_selector = 0;
        fpu.fdp_selector = 0;
    }

    pub(super) fn fpu_fnsave(&mut self, bus: &mut impl common::Bus) -> Step {
        let env_size = self.fpu_environment_size();
        if self.fpu_check_operand(env_size + 80, true, bus).is_err() {
            return Ok(());
        }

        if self.fpu_store_environment(bus).is_err() {
            return Ok(());
        }

        // Translate every byte of the register area before the first register write.
        // FNSAVE re-initializes the FPU only on full success.
        let reg_base = self.ea.wrapping_add(env_size);
        let mut phys = [0u32; 80];
        for i in 0..80u32 {
            phys[i as usize] = self.translate_linear(reg_base.wrapping_add(i), true, bus)?;
        }

        // The registers follow the environment in stack order ST(0) to ST(7).
        for stack_index in 0..8u8 {
            let bytes = self.fpu_st(stack_index).to_le_bytes();
            for (j, &byte) in bytes.iter().enumerate() {
                bus.write_byte(phys[stack_index as usize * 10 + j], byte);
            }
        }

        self.fpu_init();
        self.clk(Self::timing(375, 154));
        Ok(())
    }

    pub(super) fn fpu_frstor(&mut self, bus: &mut impl common::Bus) -> Step {
        let env_size = self.fpu_environment_size();
        if self.fpu_check_operand(env_size + 80, false, bus).is_err() {
            return Ok(());
        }

        // Read the environment and all 80 register bytes before any commit, so a faulting
        // load leaves both the environment and the register stack at their previous values.
        let Ok(words) = self.fpu_read_environment(bus) else {
            return Ok(());
        };
        let reg_base = self.ea.wrapping_add(env_size);
        let mut phys = [0u32; 80];
        for i in 0..80u32 {
            phys[i as usize] = self.translate_linear(reg_base.wrapping_add(i), false, bus)?;
        }
        let mut stack_values = [Fp80::ZERO; 8];
        for (stack_index, value) in stack_values.iter_mut().enumerate() {
            let mut bytes = [0u8; 10];
            for (j, byte) in bytes.iter_mut().enumerate() {
                *byte = bus.read_byte(phys[stack_index * 10 + j]);
            }
            *value = Fp80::from_le_bytes(bytes);
        }

        self.fpu_apply_environment(words);

        // The image holds the registers in stack order relative to the loaded TOP.
        for (stack_index, value) in stack_values.into_iter().enumerate() {
            let physical = self.fpu_st_phys(stack_index as u8);
            self.state.fpu.registers[physical] = value;
        }

        self.fpu_update_es();
        self.clk(Self::timing(308, 131));
        Ok(())
    }
}

/// The reserved high word of the control, status, tag and pointer fields of a 32-bit
/// environment image. Observed behaviour: it is stored as ones.
const ENVIRONMENT_RESERVED_HIGH_WORD: u32 = 0xFFFF_0000;
/// The opcode field: the low 11 bits of the last escape instruction (80486 PRM 15.1.5).
const ENVIRONMENT_OPCODE_MASK: u32 = 0x07FF;
/// The bit position of the opcode in the CS selector dword of the 32-bit protected mode
/// image.
const PROTECTED_OPCODE_SHIFT: u32 = 16;
/// Pointer bits 15..0, stored in the low word of the pointer offset field (figures 15-6
/// and 15-8).
const REAL_POINTER_LOW_MASK: u32 = 0xFFFF;
/// The first pointer bit of the high field: bits 16 and up.
const REAL_POINTER_HIGH_FIRST_BIT: u32 = 16;
/// The bit position of the high pointer bits in their field: bits 31..16 sit at bits
/// 27..12 of the 32-bit field (figure 15-6), bits 19..16 at bits 15..12 of the 16-bit
/// field (figure 15-8).
const REAL_POINTER_HIGH_FIELD_SHIFT: u32 = 12;
/// The count of high pointer bits in the 32-bit image: bits 31..16.
const REAL_POINTER_HIGH_BITS_32: u32 = 16;
/// The count of high pointer bits in the 16-bit image: bits 19..16.
const REAL_POINTER_HIGH_BITS_16: u32 = 4;

/// Returns the linear address `selector * 16 + offset` of an x87 pointer.
fn real_mode_linear_pointer(selector: u16, offset: u32) -> u32 {
    ((selector as u32) << 4).wrapping_add(offset)
}

/// Returns the field that holds the `count` pointer bits from bit 16 up at bit 12.
fn real_pointer_high_field(pointer: u32, count: u32) -> u32 {
    let high_bits = (pointer >> REAL_POINTER_HIGH_FIRST_BIT) & ((1 << count) - 1);
    high_bits << REAL_POINTER_HIGH_FIELD_SHIFT
}

/// Returns the pointer bits from bit 16 up held in the high field with `count` bits.
fn real_pointer_from_high_field(field: u32, count: u32) -> u32 {
    let high_bits = (field >> REAL_POINTER_HIGH_FIELD_SHIFT) & ((1 << count) - 1);
    high_bits << REAL_POINTER_HIGH_FIRST_BIT
}
