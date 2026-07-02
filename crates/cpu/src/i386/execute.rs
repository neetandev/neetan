use super::{
    CPU_MODEL_386, CPU_MODEL_486, EFLAGS_ALIGNMENT_CHECK_FLAG, EFLAGS_RESUME_FLAG,
    EFLAGS_VIRTUAL_8086_FLAG, Fault, I386, Step,
};
use crate::{ByteReg, DwordReg, SegReg32, WordReg};

impl<const CPU_MODEL: u8, const ADDRESS_WIDTH: u8> I386<CPU_MODEL, ADDRESS_WIDTH> {
    pub(super) fn dispatch(&mut self, opcode: u8, bus: &mut impl common::Bus) -> Step {
        match opcode {
            // ADD
            0x00 => self.add_br8(bus)?,
            0x01 => self.add_wr16(bus)?,
            0x02 => self.add_r8b(bus)?,
            0x03 => self.add_r16w(bus)?,
            0x04 => self.add_ald8(bus),
            0x05 => self.add_axd16(bus),
            0x06 => self.push_seg(SegReg32::ES, bus)?,
            0x07 => self.pop_seg(SegReg32::ES, bus)?,

            // OR
            0x08 => self.or_br8(bus)?,
            0x09 => self.or_wr16(bus)?,
            0x0A => self.or_r8b(bus)?,
            0x0B => self.or_r16w(bus)?,
            0x0C => self.or_ald8(bus),
            0x0D => self.or_axd16(bus),
            0x0E => self.push_seg(SegReg32::CS, bus)?,
            0x0F => self.extended_0f(bus)?,

            // ADC
            0x10 => self.adc_br8(bus)?,
            0x11 => self.adc_wr16(bus)?,
            0x12 => self.adc_r8b(bus)?,
            0x13 => self.adc_r16w(bus)?,
            0x14 => self.adc_ald8(bus),
            0x15 => self.adc_axd16(bus),
            0x16 => self.push_seg(SegReg32::SS, bus)?,
            0x17 => {
                self.pop_seg(SegReg32::SS, bus)?;
                self.inhibit_all = 1;
            }

            // SBB
            0x18 => self.sbb_br8(bus)?,
            0x19 => self.sbb_wr16(bus)?,
            0x1A => self.sbb_r8b(bus)?,
            0x1B => self.sbb_r16w(bus)?,
            0x1C => self.sbb_ald8(bus),
            0x1D => self.sbb_axd16(bus),
            0x1E => self.push_seg(SegReg32::DS, bus)?,
            0x1F => self.pop_seg(SegReg32::DS, bus)?,

            // AND
            0x20 => self.and_br8(bus)?,
            0x21 => self.and_wr16(bus)?,
            0x22 => self.and_r8b(bus)?,
            0x23 => self.and_r16w(bus)?,
            0x24 => self.and_ald8(bus),
            0x25 => self.and_axd16(bus),
            0x26 => self.invalid(bus)?,
            0x27 => self.daa(bus),

            // SUB
            0x28 => self.sub_br8(bus)?,
            0x29 => self.sub_wr16(bus)?,
            0x2A => self.sub_r8b(bus)?,
            0x2B => self.sub_r16w(bus)?,
            0x2C => self.sub_ald8(bus),
            0x2D => self.sub_axd16(bus),
            0x2E => self.invalid(bus)?,
            0x2F => self.das(bus),

            // XOR
            0x30 => self.xor_br8(bus)?,
            0x31 => self.xor_wr16(bus)?,
            0x32 => self.xor_r8b(bus)?,
            0x33 => self.xor_r16w(bus)?,
            0x34 => self.xor_ald8(bus),
            0x35 => self.xor_axd16(bus),
            0x36 => self.invalid(bus)?,
            0x37 => self.aaa(bus),

            // CMP
            0x38 => self.cmp_br8(bus)?,
            0x39 => self.cmp_wr16(bus)?,
            0x3A => self.cmp_r8b(bus)?,
            0x3B => self.cmp_r16w(bus)?,
            0x3C => self.cmp_ald8(bus),
            0x3D => self.cmp_axd16(bus),
            0x3E => self.invalid(bus)?,
            0x3F => self.aas(bus),

            // INC word registers
            0x40 => self.inc_word_reg(WordReg::AX),
            0x41 => self.inc_word_reg(WordReg::CX),
            0x42 => self.inc_word_reg(WordReg::DX),
            0x43 => self.inc_word_reg(WordReg::BX),
            0x44 => self.inc_word_reg(WordReg::SP),
            0x45 => self.inc_word_reg(WordReg::BP),
            0x46 => self.inc_word_reg(WordReg::SI),
            0x47 => self.inc_word_reg(WordReg::DI),

            // DEC word registers
            0x48 => self.dec_word_reg(WordReg::AX),
            0x49 => self.dec_word_reg(WordReg::CX),
            0x4A => self.dec_word_reg(WordReg::DX),
            0x4B => self.dec_word_reg(WordReg::BX),
            0x4C => self.dec_word_reg(WordReg::SP),
            0x4D => self.dec_word_reg(WordReg::BP),
            0x4E => self.dec_word_reg(WordReg::SI),
            0x4F => self.dec_word_reg(WordReg::DI),

            // PUSH word registers
            0x50 => self.push_word_reg(WordReg::AX, bus)?,
            0x51 => self.push_word_reg(WordReg::CX, bus)?,
            0x52 => self.push_word_reg(WordReg::DX, bus)?,
            0x53 => self.push_word_reg(WordReg::BX, bus)?,
            0x54 => self.push_sp(bus)?,
            0x55 => self.push_word_reg(WordReg::BP, bus)?,
            0x56 => self.push_word_reg(WordReg::SI, bus)?,
            0x57 => self.push_word_reg(WordReg::DI, bus)?,

            // POP word registers
            0x58 => self.pop_word_reg(WordReg::AX, bus)?,
            0x59 => self.pop_word_reg(WordReg::CX, bus)?,
            0x5A => self.pop_word_reg(WordReg::DX, bus)?,
            0x5B => self.pop_word_reg(WordReg::BX, bus)?,
            0x5C => self.pop_word_reg(WordReg::SP, bus)?,
            0x5D => self.pop_word_reg(WordReg::BP, bus)?,
            0x5E => self.pop_word_reg(WordReg::SI, bus)?,
            0x5F => self.pop_word_reg(WordReg::DI, bus)?,

            // 80186 instructions
            0x60 => self.pusha(bus)?,
            0x61 => self.popa(bus)?,
            0x62 => self.bound(bus)?,
            0x63 => self.arpl(bus)?,
            0x64 => self.invalid(bus)?,
            0x65 => self.invalid(bus)?,
            0x66 => self.invalid(bus)?,
            0x67 => self.invalid(bus)?,
            0x68 => self.push_imm16(bus)?,
            0x69 => self.imul_r16w_imm16(bus)?,
            0x6A => self.push_imm8(bus)?,
            0x6B => self.imul_r16w_imm8(bus)?,
            0x6C => self.insb(bus)?,
            0x6D => self.insw(bus)?,
            0x6E => self.outsb(bus)?,
            0x6F => self.outsw(bus)?,

            // Jcc (short jumps)
            0x70 => self.jcc(bus, self.flags.of()),
            0x71 => self.jcc(bus, !self.flags.of()),
            0x72 => self.jcc(bus, self.flags.cf()),
            0x73 => self.jcc(bus, !self.flags.cf()),
            0x74 => self.jcc(bus, self.flags.zf()),
            0x75 => self.jcc(bus, !self.flags.zf()),
            0x76 => self.jcc(bus, self.flags.cf() || self.flags.zf()),
            0x77 => self.jcc_swapped(bus, !self.flags.cf() && !self.flags.zf()),
            0x78 => self.jcc(bus, self.flags.sf()),
            0x79 => self.jcc(bus, !self.flags.sf()),
            0x7A => self.jcc(bus, self.flags.pf()),
            0x7B => self.jcc(bus, !self.flags.pf()),
            0x7C => self.jcc(bus, self.flags.sf() != self.flags.of()),
            0x7D => self.jcc_swapped(bus, self.flags.sf() == self.flags.of()),
            0x7E => self.jcc(bus, self.flags.zf() || (self.flags.sf() != self.flags.of())),
            0x7F => self.jcc_swapped(
                bus,
                !self.flags.zf() && (self.flags.sf() == self.flags.of()),
            ),

            // Group 1
            0x80 => self.group_80(bus)?,
            0x81 => self.group_81(bus)?,
            0x82 => self.group_82(bus)?,
            0x83 => self.group_83(bus)?,

            // TEST
            0x84 => self.test_br8(bus)?,
            0x85 => self.test_wr16(bus)?,

            // XCHG
            0x86 => self.xchg_br8(bus)?,
            0x87 => self.xchg_wr16(bus)?,

            // MOV r/m, reg
            0x88 => self.mov_br8(bus)?,
            0x89 => self.mov_wr16(bus)?,
            0x8A => self.mov_r8b(bus)?,
            0x8B => self.mov_r16w(bus)?,

            // MOV r/m, sreg / LEA / MOV sreg, r/m
            0x8C => self.mov_rm_sreg(bus)?,
            0x8D => self.lea(bus)?,
            0x8E => self.mov_sreg_rm(bus)?,
            0x8F => self.pop_rm(bus)?,

            // XCHG AX, reg / NOP
            0x90 => self.clk(Self::timing(3, 1)),
            0x91 => self.xchg_aw(WordReg::CX),
            0x92 => self.xchg_aw(WordReg::DX),
            0x93 => self.xchg_aw(WordReg::BX),
            0x94 => self.xchg_aw(WordReg::SP),
            0x95 => self.xchg_aw(WordReg::BP),
            0x96 => self.xchg_aw(WordReg::SI),
            0x97 => self.xchg_aw(WordReg::DI),

            // CBW, CWD
            0x98 => self.cbw(),
            0x99 => self.cwd(),

            // CALL far, WAIT
            0x9A => self.call_far(bus)?,
            0x9B => self.fpu_wait(bus)?,
            0x9C => self.pushf(bus)?,
            0x9D => self.popf(bus)?,
            0x9E => self.sahf(),
            0x9F => self.lahf(),

            // MOV AL/AX, [addr] and [addr], AL/AX
            0xA0 => self.mov_al_moffs(bus)?,
            0xA1 => self.mov_aw_moffs(bus)?,
            0xA2 => self.mov_moffs_al(bus)?,
            0xA3 => self.mov_moffs_aw(bus)?,

            // String ops
            0xA4 => self.movsb(bus)?,
            0xA5 => self.movsw(bus)?,
            0xA6 => self.cmpsb(bus)?,
            0xA7 => self.cmpsw(bus)?,

            // TEST AL/AX, imm
            0xA8 => self.test_al_imm8(bus),
            0xA9 => self.test_aw_imm16(bus),

            // STOS, LODS, SCAS
            0xAA => self.stosb(bus)?,
            0xAB => self.stosw(bus)?,
            0xAC => self.lodsb(bus)?,
            0xAD => self.lodsw(bus)?,
            0xAE => self.scasb(bus)?,
            0xAF => self.scasw(bus)?,

            // MOV byte reg, imm8
            0xB0 => self.mov_byte_reg_imm(ByteReg::AL, bus),
            0xB1 => self.mov_byte_reg_imm(ByteReg::CL, bus),
            0xB2 => self.mov_byte_reg_imm(ByteReg::DL, bus),
            0xB3 => self.mov_byte_reg_imm(ByteReg::BL, bus),
            0xB4 => self.mov_byte_reg_imm(ByteReg::AH, bus),
            0xB5 => self.mov_byte_reg_imm(ByteReg::CH, bus),
            0xB6 => self.mov_byte_reg_imm(ByteReg::DH, bus),
            0xB7 => self.mov_byte_reg_imm(ByteReg::BH, bus),

            // MOV word reg, imm16
            0xB8 => self.mov_word_reg_imm(WordReg::AX, bus),
            0xB9 => self.mov_word_reg_imm(WordReg::CX, bus),
            0xBA => self.mov_word_reg_imm(WordReg::DX, bus),
            0xBB => self.mov_word_reg_imm(WordReg::BX, bus),
            0xBC => self.mov_word_reg_imm(WordReg::SP, bus),
            0xBD => self.mov_word_reg_imm(WordReg::BP, bus),
            0xBE => self.mov_word_reg_imm(WordReg::SI, bus),
            0xBF => self.mov_word_reg_imm(WordReg::DI, bus),

            // Shift/rotate groups
            0xC0 => self.group_c0(bus)?,
            0xC1 => self.group_c1(bus)?,

            // RET near imm16, RET near
            0xC2 => self.ret_near_imm(bus)?,
            0xC3 => self.ret_near(bus)?,

            // LES, LDS
            0xC4 => self.les(bus)?,
            0xC5 => self.lds(bus)?,

            // MOV r/m, imm
            0xC6 => self.mov_rm_imm8(bus)?,
            0xC7 => self.mov_rm_imm16(bus)?,

            // ENTER, LEAVE
            0xC8 => self.enter(bus)?,
            0xC9 => self.leave(bus)?,

            // RET far imm16, RET far
            0xCA => self.ret_far_imm(bus)?,
            0xCB => self.ret_far(bus)?,

            // INT 3, INT imm8, INTO, IRET
            0xCC => self.int3(bus)?,
            0xCD => self.int_imm(bus)?,
            0xCE => self.into(bus)?,
            0xCF => self.iret(bus)?,

            // Shift/rotate groups
            0xD0 => self.group_d0(bus)?,
            0xD1 => self.group_d1(bus)?,
            0xD2 => self.group_d2(bus)?,
            0xD3 => self.group_d3(bus)?,

            // AAM, AAD
            0xD4 => self.aam(bus),
            0xD5 => self.aad(bus),

            // undocumented SALC
            0xD6 => self.salc(),

            // XLAT
            0xD7 => self.xlat(bus)?,

            // FPU escape
            0xD8..=0xDF => self.fpu_escape(opcode, bus)?,

            // LOOPNE, LOOPE, LOOP, JCXZ
            0xE0 => self.loopne(bus),
            0xE1 => self.loope(bus),
            0xE2 => self.loop_(bus),
            0xE3 => self.jcxz(bus),

            // IN, OUT
            0xE4 => self.in_al_imm(bus)?,
            0xE5 => self.in_aw_imm(bus)?,
            0xE6 => self.out_imm_al(bus)?,
            0xE7 => self.out_imm_aw(bus)?,

            // CALL near, JMP near, JMP far, JMP short
            0xE8 => self.call_near(bus)?,
            0xE9 => self.jmp_near(bus),
            0xEA => self.jmp_far(bus)?,
            0xEB => self.jmp_short(bus),

            // IN, OUT (DX port)
            0xEC => self.in_al_dw(bus)?,
            0xED => self.in_aw_dw(bus)?,
            0xEE => self.out_dw_al(bus)?,
            0xEF => self.out_dw_aw(bus)?,

            0xF0 => self.invalid(bus)?,
            0xF1 => self.invalid(bus)?,

            // REPNE, REPE
            0xF2 => self.repne(bus)?,
            0xF3 => self.repe(bus)?,

            // HLT
            0xF4 => self.hlt(bus)?,

            // CMC
            0xF5 => self.cmc(),

            // Group 3 byte/word
            0xF6 => self.group_f6(bus)?,
            0xF7 => self.group_f7(bus)?,

            // CLC, STC, CLI, STI, CLD, STD
            0xF8 => self.clc(),
            0xF9 => self.stc(),
            0xFA => self.cli(bus)?,
            0xFB => self.sti(bus)?,
            0xFC => self.cld(),
            0xFD => self.std(),

            // Group 4/5
            0xFE => self.group_fe(bus)?,
            0xFF => self.group_ff(bus)?,
        }
        Ok(())
    }

    fn add_br8(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        let src = self.regs.byte(self.reg_byte(modrm));
        let dst = self.get_rm_byte_for_update(modrm, bus)?;
        let result = self.alu_add_byte(dst, src);
        self.putback_rm_byte(modrm, result, bus)?;
        self.clk_modrm(modrm, Self::timing(2, 1), Self::timing(7, 3));
        Ok(())
    }

    fn add_wr16(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        if self.operand_size_override {
            let src = self.regs.dword(self.reg_dword(modrm));
            let dst = self.get_rm_dword_for_update(modrm, bus)?;
            let result = self.alu_add_dword(dst, src);
            self.putback_rm_dword(modrm, result, bus)?;
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(7, 3), 4);
        } else {
            let src = self.regs.word(self.reg_word(modrm));
            let dst = self.get_rm_word_for_update(modrm, bus)?;
            let result = self.alu_add_word(dst, src);
            self.putback_rm_word(modrm, result, bus)?;
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(7, 3), 2);
        }
        Ok(())
    }

    fn add_r8b(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        let dst = self.regs.byte(self.reg_byte(modrm));
        let src = self.get_rm_byte(modrm, bus)?;
        let result = self.alu_add_byte(dst, src);
        let reg = self.reg_byte(modrm);
        self.regs.set_byte(reg, result);
        self.clk_modrm(modrm, Self::timing(2, 1), Self::timing(6, 2));
        Ok(())
    }

    fn add_r16w(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        if self.operand_size_override {
            let dst = self.regs.dword(self.reg_dword(modrm));
            let src = self.get_rm_dword(modrm, bus)?;
            let result = self.alu_add_dword(dst, src);
            let reg = self.reg_dword(modrm);
            self.regs.set_dword(reg, result);
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(6, 2), 2);
        } else {
            let dst = self.regs.word(self.reg_word(modrm));
            let src = self.get_rm_word(modrm, bus)?;
            let result = self.alu_add_word(dst, src);
            let reg = self.reg_word(modrm);
            self.regs.set_word(reg, result);
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(6, 2), 1);
        }
        Ok(())
    }

    fn add_ald8(&mut self, bus: &mut impl common::Bus) {
        let src = self.fetch(bus);
        let dst = self.regs.byte(ByteReg::AL);
        let result = self.alu_add_byte(dst, src);
        self.regs.set_byte(ByteReg::AL, result);
        self.clk(Self::timing(2, 1));
    }

    fn add_axd16(&mut self, bus: &mut impl common::Bus) {
        if self.operand_size_override {
            let src = self.fetchdword(bus);
            let dst = self.regs.dword(DwordReg::EAX);
            let result = self.alu_add_dword(dst, src);
            self.regs.set_dword(DwordReg::EAX, result);
        } else {
            let src = self.fetchword(bus);
            let dst = self.regs.word(WordReg::AX);
            let result = self.alu_add_word(dst, src);
            self.regs.set_word(WordReg::AX, result);
        }
        self.clk(Self::timing(2, 1));
    }

    fn or_br8(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        let src = self.regs.byte(self.reg_byte(modrm));
        let dst = self.get_rm_byte_for_update(modrm, bus)?;
        let result = self.alu_or_byte(dst, src);
        self.putback_rm_byte(modrm, result, bus)?;
        self.clk_modrm(modrm, Self::timing(2, 1), Self::timing(7, 3));
        Ok(())
    }

    fn or_wr16(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        if self.operand_size_override {
            let src = self.regs.dword(self.reg_dword(modrm));
            let dst = self.get_rm_dword_for_update(modrm, bus)?;
            let result = self.alu_or_dword(dst, src);
            self.putback_rm_dword(modrm, result, bus)?;
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(7, 3), 4);
        } else {
            let src = self.regs.word(self.reg_word(modrm));
            let dst = self.get_rm_word_for_update(modrm, bus)?;
            let result = self.alu_or_word(dst, src);
            self.putback_rm_word(modrm, result, bus)?;
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(7, 3), 2);
        }
        Ok(())
    }

    fn or_r8b(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        let dst = self.regs.byte(self.reg_byte(modrm));
        let src = self.get_rm_byte(modrm, bus)?;
        let result = self.alu_or_byte(dst, src);
        let reg = self.reg_byte(modrm);
        self.regs.set_byte(reg, result);
        self.clk_modrm(modrm, Self::timing(2, 1), Self::timing(6, 2));
        Ok(())
    }

    fn or_r16w(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        if self.operand_size_override {
            let dst = self.regs.dword(self.reg_dword(modrm));
            let src = self.get_rm_dword(modrm, bus)?;
            let result = self.alu_or_dword(dst, src);
            let reg = self.reg_dword(modrm);
            self.regs.set_dword(reg, result);
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(6, 2), 2);
        } else {
            let dst = self.regs.word(self.reg_word(modrm));
            let src = self.get_rm_word(modrm, bus)?;
            let result = self.alu_or_word(dst, src);
            let reg = self.reg_word(modrm);
            self.regs.set_word(reg, result);
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(6, 2), 1);
        }
        Ok(())
    }

    fn or_ald8(&mut self, bus: &mut impl common::Bus) {
        let src = self.fetch(bus);
        let dst = self.regs.byte(ByteReg::AL);
        let result = self.alu_or_byte(dst, src);
        self.regs.set_byte(ByteReg::AL, result);
        self.clk(Self::timing(2, 1));
    }

    fn or_axd16(&mut self, bus: &mut impl common::Bus) {
        if self.operand_size_override {
            let src = self.fetchdword(bus);
            let dst = self.regs.dword(DwordReg::EAX);
            let result = self.alu_or_dword(dst, src);
            self.regs.set_dword(DwordReg::EAX, result);
        } else {
            let src = self.fetchword(bus);
            let dst = self.regs.word(WordReg::AX);
            let result = self.alu_or_word(dst, src);
            self.regs.set_word(WordReg::AX, result);
        }
        self.clk(Self::timing(2, 1));
    }

    fn adc_br8(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        let src = self.regs.byte(self.reg_byte(modrm));
        let dst = self.get_rm_byte_for_update(modrm, bus)?;
        let cf = self.flags.cf_val();
        let result = self.alu_adc_byte(dst, src, cf);
        self.putback_rm_byte(modrm, result, bus)?;
        self.clk_modrm(modrm, Self::timing(2, 1), Self::timing(7, 3));
        Ok(())
    }

    fn adc_wr16(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        let cf = self.flags.cf_val();
        if self.operand_size_override {
            let src = self.regs.dword(self.reg_dword(modrm));
            let dst = self.get_rm_dword_for_update(modrm, bus)?;
            let result = self.alu_adc_dword(dst, src, cf);
            self.putback_rm_dword(modrm, result, bus)?;
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(7, 3), 4);
        } else {
            let src = self.regs.word(self.reg_word(modrm));
            let dst = self.get_rm_word_for_update(modrm, bus)?;
            let result = self.alu_adc_word(dst, src, cf);
            self.putback_rm_word(modrm, result, bus)?;
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(7, 3), 2);
        }
        Ok(())
    }

    fn adc_r8b(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        let dst = self.regs.byte(self.reg_byte(modrm));
        let src = self.get_rm_byte(modrm, bus)?;
        let cf = self.flags.cf_val();
        let result = self.alu_adc_byte(dst, src, cf);
        let reg = self.reg_byte(modrm);
        self.regs.set_byte(reg, result);
        self.clk_modrm(modrm, Self::timing(2, 1), Self::timing(6, 2));
        Ok(())
    }

    fn adc_r16w(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        let cf = self.flags.cf_val();
        if self.operand_size_override {
            let dst = self.regs.dword(self.reg_dword(modrm));
            let src = self.get_rm_dword(modrm, bus)?;
            let result = self.alu_adc_dword(dst, src, cf);
            let reg = self.reg_dword(modrm);
            self.regs.set_dword(reg, result);
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(6, 2), 2);
        } else {
            let dst = self.regs.word(self.reg_word(modrm));
            let src = self.get_rm_word(modrm, bus)?;
            let result = self.alu_adc_word(dst, src, cf);
            let reg = self.reg_word(modrm);
            self.regs.set_word(reg, result);
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(6, 2), 1);
        }
        Ok(())
    }

    fn adc_ald8(&mut self, bus: &mut impl common::Bus) {
        let src = self.fetch(bus);
        let dst = self.regs.byte(ByteReg::AL);
        let cf = self.flags.cf_val();
        let result = self.alu_adc_byte(dst, src, cf);
        self.regs.set_byte(ByteReg::AL, result);
        self.clk(Self::timing(2, 1));
    }

    fn adc_axd16(&mut self, bus: &mut impl common::Bus) {
        let cf = self.flags.cf_val();
        if self.operand_size_override {
            let src = self.fetchdword(bus);
            let dst = self.regs.dword(DwordReg::EAX);
            let result = self.alu_adc_dword(dst, src, cf);
            self.regs.set_dword(DwordReg::EAX, result);
        } else {
            let src = self.fetchword(bus);
            let dst = self.regs.word(WordReg::AX);
            let result = self.alu_adc_word(dst, src, cf);
            self.regs.set_word(WordReg::AX, result);
        }
        self.clk(Self::timing(2, 1));
    }

    fn sbb_br8(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        let src = self.regs.byte(self.reg_byte(modrm));
        let dst = self.get_rm_byte_for_update(modrm, bus)?;
        let cf = self.flags.cf_val();
        let result = self.alu_sbb_byte(dst, src, cf);
        self.putback_rm_byte(modrm, result, bus)?;
        self.clk_modrm(modrm, Self::timing(2, 1), Self::timing(7, 3));
        Ok(())
    }

    fn sbb_wr16(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        let cf = self.flags.cf_val();
        if self.operand_size_override {
            let src = self.regs.dword(self.reg_dword(modrm));
            let dst = self.get_rm_dword_for_update(modrm, bus)?;
            let result = self.alu_sbb_dword(dst, src, cf);
            self.putback_rm_dword(modrm, result, bus)?;
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(7, 3), 4);
        } else {
            let src = self.regs.word(self.reg_word(modrm));
            let dst = self.get_rm_word_for_update(modrm, bus)?;
            let result = self.alu_sbb_word(dst, src, cf);
            self.putback_rm_word(modrm, result, bus)?;
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(7, 3), 2);
        }
        Ok(())
    }

    fn sbb_r8b(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        let dst = self.regs.byte(self.reg_byte(modrm));
        let src = self.get_rm_byte(modrm, bus)?;
        let cf = self.flags.cf_val();
        let result = self.alu_sbb_byte(dst, src, cf);
        let reg = self.reg_byte(modrm);
        self.regs.set_byte(reg, result);
        self.clk_modrm(modrm, Self::timing(2, 1), Self::timing(6, 2));
        Ok(())
    }

    fn sbb_r16w(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        let cf = self.flags.cf_val();
        if self.operand_size_override {
            let dst = self.regs.dword(self.reg_dword(modrm));
            let src = self.get_rm_dword(modrm, bus)?;
            let result = self.alu_sbb_dword(dst, src, cf);
            let reg = self.reg_dword(modrm);
            self.regs.set_dword(reg, result);
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(6, 2), 2);
        } else {
            let dst = self.regs.word(self.reg_word(modrm));
            let src = self.get_rm_word(modrm, bus)?;
            let result = self.alu_sbb_word(dst, src, cf);
            let reg = self.reg_word(modrm);
            self.regs.set_word(reg, result);
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(6, 2), 1);
        }
        Ok(())
    }

    fn sbb_ald8(&mut self, bus: &mut impl common::Bus) {
        let src = self.fetch(bus);
        let dst = self.regs.byte(ByteReg::AL);
        let cf = self.flags.cf_val();
        let result = self.alu_sbb_byte(dst, src, cf);
        self.regs.set_byte(ByteReg::AL, result);
        self.clk(Self::timing(2, 1));
    }

    fn sbb_axd16(&mut self, bus: &mut impl common::Bus) {
        let cf = self.flags.cf_val();
        if self.operand_size_override {
            let src = self.fetchdword(bus);
            let dst = self.regs.dword(DwordReg::EAX);
            let result = self.alu_sbb_dword(dst, src, cf);
            self.regs.set_dword(DwordReg::EAX, result);
        } else {
            let src = self.fetchword(bus);
            let dst = self.regs.word(WordReg::AX);
            let result = self.alu_sbb_word(dst, src, cf);
            self.regs.set_word(WordReg::AX, result);
        }
        self.clk(Self::timing(2, 1));
    }

    fn and_br8(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        let src = self.regs.byte(self.reg_byte(modrm));
        let dst = self.get_rm_byte_for_update(modrm, bus)?;
        let result = self.alu_and_byte(dst, src);
        self.putback_rm_byte(modrm, result, bus)?;
        self.clk_modrm(modrm, Self::timing(2, 1), Self::timing(7, 3));
        Ok(())
    }

    fn and_wr16(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        if self.operand_size_override {
            let src = self.regs.dword(self.reg_dword(modrm));
            let dst = self.get_rm_dword_for_update(modrm, bus)?;
            let result = self.alu_and_dword(dst, src);
            self.putback_rm_dword(modrm, result, bus)?;
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(7, 3), 4);
        } else {
            let src = self.regs.word(self.reg_word(modrm));
            let dst = self.get_rm_word_for_update(modrm, bus)?;
            let result = self.alu_and_word(dst, src);
            self.putback_rm_word(modrm, result, bus)?;
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(7, 3), 2);
        }
        Ok(())
    }

    fn and_r8b(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        let dst = self.regs.byte(self.reg_byte(modrm));
        let src = self.get_rm_byte(modrm, bus)?;
        let result = self.alu_and_byte(dst, src);
        let reg = self.reg_byte(modrm);
        self.regs.set_byte(reg, result);
        self.clk_modrm(modrm, Self::timing(2, 1), Self::timing(6, 2));
        Ok(())
    }

    fn and_r16w(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        if self.operand_size_override {
            let dst = self.regs.dword(self.reg_dword(modrm));
            let src = self.get_rm_dword(modrm, bus)?;
            let result = self.alu_and_dword(dst, src);
            let reg = self.reg_dword(modrm);
            self.regs.set_dword(reg, result);
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(6, 2), 2);
        } else {
            let dst = self.regs.word(self.reg_word(modrm));
            let src = self.get_rm_word(modrm, bus)?;
            let result = self.alu_and_word(dst, src);
            let reg = self.reg_word(modrm);
            self.regs.set_word(reg, result);
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(6, 2), 1);
        }
        Ok(())
    }

    fn and_ald8(&mut self, bus: &mut impl common::Bus) {
        let src = self.fetch(bus);
        let dst = self.regs.byte(ByteReg::AL);
        let result = self.alu_and_byte(dst, src);
        self.regs.set_byte(ByteReg::AL, result);
        self.clk(Self::timing(2, 1));
    }

    fn and_axd16(&mut self, bus: &mut impl common::Bus) {
        if self.operand_size_override {
            let src = self.fetchdword(bus);
            let dst = self.regs.dword(DwordReg::EAX);
            let result = self.alu_and_dword(dst, src);
            self.regs.set_dword(DwordReg::EAX, result);
        } else {
            let src = self.fetchword(bus);
            let dst = self.regs.word(WordReg::AX);
            let result = self.alu_and_word(dst, src);
            self.regs.set_word(WordReg::AX, result);
        }
        self.clk(Self::timing(2, 1));
    }

    fn sub_br8(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        let src = self.regs.byte(self.reg_byte(modrm));
        let dst = self.get_rm_byte_for_update(modrm, bus)?;
        let result = self.alu_sub_byte(dst, src);
        self.putback_rm_byte(modrm, result, bus)?;
        self.clk_modrm(modrm, Self::timing(2, 1), Self::timing(7, 3));
        Ok(())
    }

    fn sub_wr16(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        if self.operand_size_override {
            let src = self.regs.dword(self.reg_dword(modrm));
            let dst = self.get_rm_dword_for_update(modrm, bus)?;
            let result = self.alu_sub_dword(dst, src);
            self.putback_rm_dword(modrm, result, bus)?;
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(7, 3), 4);
        } else {
            let src = self.regs.word(self.reg_word(modrm));
            let dst = self.get_rm_word_for_update(modrm, bus)?;
            let result = self.alu_sub_word(dst, src);
            self.putback_rm_word(modrm, result, bus)?;
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(7, 3), 2);
        }
        Ok(())
    }

    fn sub_r8b(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        let dst = self.regs.byte(self.reg_byte(modrm));
        let src = self.get_rm_byte(modrm, bus)?;
        let result = self.alu_sub_byte(dst, src);
        let reg = self.reg_byte(modrm);
        self.regs.set_byte(reg, result);
        self.clk_modrm(modrm, Self::timing(2, 1), Self::timing(6, 2));
        Ok(())
    }

    fn sub_r16w(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        if self.operand_size_override {
            let dst = self.regs.dword(self.reg_dword(modrm));
            let src = self.get_rm_dword(modrm, bus)?;
            let result = self.alu_sub_dword(dst, src);
            let reg = self.reg_dword(modrm);
            self.regs.set_dword(reg, result);
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(6, 2), 2);
        } else {
            let dst = self.regs.word(self.reg_word(modrm));
            let src = self.get_rm_word(modrm, bus)?;
            let result = self.alu_sub_word(dst, src);
            let reg = self.reg_word(modrm);
            self.regs.set_word(reg, result);
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(6, 2), 1);
        }
        Ok(())
    }

    fn sub_ald8(&mut self, bus: &mut impl common::Bus) {
        let src = self.fetch(bus);
        let dst = self.regs.byte(ByteReg::AL);
        let result = self.alu_sub_byte(dst, src);
        self.regs.set_byte(ByteReg::AL, result);
        self.clk(Self::timing(2, 1));
    }

    fn sub_axd16(&mut self, bus: &mut impl common::Bus) {
        if self.operand_size_override {
            let src = self.fetchdword(bus);
            let dst = self.regs.dword(DwordReg::EAX);
            let result = self.alu_sub_dword(dst, src);
            self.regs.set_dword(DwordReg::EAX, result);
        } else {
            let src = self.fetchword(bus);
            let dst = self.regs.word(WordReg::AX);
            let result = self.alu_sub_word(dst, src);
            self.regs.set_word(WordReg::AX, result);
        }
        self.clk(Self::timing(2, 1));
    }

    fn xor_br8(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        let src = self.regs.byte(self.reg_byte(modrm));
        let dst = self.get_rm_byte_for_update(modrm, bus)?;
        let result = self.alu_xor_byte(dst, src);
        self.putback_rm_byte(modrm, result, bus)?;
        self.clk_modrm(modrm, Self::timing(2, 1), Self::timing(7, 3));
        Ok(())
    }

    fn xor_wr16(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        if self.operand_size_override {
            let src = self.regs.dword(self.reg_dword(modrm));
            let dst = self.get_rm_dword_for_update(modrm, bus)?;
            let result = self.alu_xor_dword(dst, src);
            self.putback_rm_dword(modrm, result, bus)?;
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(7, 3), 4);
        } else {
            let src = self.regs.word(self.reg_word(modrm));
            let dst = self.get_rm_word_for_update(modrm, bus)?;
            let result = self.alu_xor_word(dst, src);
            self.putback_rm_word(modrm, result, bus)?;
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(7, 3), 2);
        }
        Ok(())
    }

    fn xor_r8b(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        let dst = self.regs.byte(self.reg_byte(modrm));
        let src = self.get_rm_byte(modrm, bus)?;
        let result = self.alu_xor_byte(dst, src);
        let reg = self.reg_byte(modrm);
        self.regs.set_byte(reg, result);
        self.clk_modrm(modrm, Self::timing(2, 1), Self::timing(6, 2));
        Ok(())
    }

    fn xor_r16w(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        if self.operand_size_override {
            let dst = self.regs.dword(self.reg_dword(modrm));
            let src = self.get_rm_dword(modrm, bus)?;
            let result = self.alu_xor_dword(dst, src);
            let reg = self.reg_dword(modrm);
            self.regs.set_dword(reg, result);
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(6, 2), 2);
        } else {
            let dst = self.regs.word(self.reg_word(modrm));
            let src = self.get_rm_word(modrm, bus)?;
            let result = self.alu_xor_word(dst, src);
            let reg = self.reg_word(modrm);
            self.regs.set_word(reg, result);
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(6, 2), 1);
        }
        Ok(())
    }

    fn xor_ald8(&mut self, bus: &mut impl common::Bus) {
        let src = self.fetch(bus);
        let dst = self.regs.byte(ByteReg::AL);
        let result = self.alu_xor_byte(dst, src);
        self.regs.set_byte(ByteReg::AL, result);
        self.clk(Self::timing(2, 1));
    }

    fn xor_axd16(&mut self, bus: &mut impl common::Bus) {
        if self.operand_size_override {
            let src = self.fetchdword(bus);
            let dst = self.regs.dword(DwordReg::EAX);
            let result = self.alu_xor_dword(dst, src);
            self.regs.set_dword(DwordReg::EAX, result);
        } else {
            let src = self.fetchword(bus);
            let dst = self.regs.word(WordReg::AX);
            let result = self.alu_xor_word(dst, src);
            self.regs.set_word(WordReg::AX, result);
        }
        self.clk(Self::timing(2, 1));
    }

    fn cmp_br8(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        let src = self.regs.byte(self.reg_byte(modrm));
        let dst = self.get_rm_byte(modrm, bus)?;
        self.alu_sub_byte(dst, src);
        self.clk_modrm(modrm, Self::timing(2, 1), Self::timing(5, 2));
        Ok(())
    }

    fn cmp_wr16(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        if self.operand_size_override {
            let src = self.regs.dword(self.reg_dword(modrm));
            let dst = self.get_rm_dword(modrm, bus)?;
            self.alu_sub_dword(dst, src);
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(5, 2), 2);
        } else {
            let src = self.regs.word(self.reg_word(modrm));
            let dst = self.get_rm_word(modrm, bus)?;
            self.alu_sub_word(dst, src);
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(5, 2), 1);
        }
        Ok(())
    }

    fn cmp_r8b(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        let dst = self.regs.byte(self.reg_byte(modrm));
        let src = self.get_rm_byte(modrm, bus)?;
        self.alu_sub_byte(dst, src);
        self.clk_modrm(modrm, Self::timing(2, 1), Self::timing(6, 2));
        Ok(())
    }

    fn cmp_r16w(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        if self.operand_size_override {
            let dst = self.regs.dword(self.reg_dword(modrm));
            let src = self.get_rm_dword(modrm, bus)?;
            self.alu_sub_dword(dst, src);
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(6, 2), 2);
        } else {
            let dst = self.regs.word(self.reg_word(modrm));
            let src = self.get_rm_word(modrm, bus)?;
            self.alu_sub_word(dst, src);
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(6, 2), 1);
        }
        Ok(())
    }

    fn cmp_ald8(&mut self, bus: &mut impl common::Bus) {
        let src = self.fetch(bus);
        let dst = self.regs.byte(ByteReg::AL);
        self.alu_sub_byte(dst, src);
        self.clk(Self::timing(2, 1));
    }

    fn cmp_axd16(&mut self, bus: &mut impl common::Bus) {
        if self.operand_size_override {
            let src = self.fetchdword(bus);
            let dst = self.regs.dword(DwordReg::EAX);
            self.alu_sub_dword(dst, src);
        } else {
            let src = self.fetchword(bus);
            let dst = self.regs.word(WordReg::AX);
            self.alu_sub_word(dst, src);
        }
        self.clk(Self::timing(2, 1));
    }

    fn inc_word_reg(&mut self, reg: WordReg) {
        if self.operand_size_override {
            let dreg = DwordReg::from_index(reg as u8);
            let val = self.regs.dword(dreg);
            let result = self.alu_inc_dword(val);
            self.regs.set_dword(dreg, result);
        } else {
            let val = self.regs.word(reg);
            let result = self.alu_inc_word(val);
            self.regs.set_word(reg, result);
        }
        self.clk(Self::timing(2, 1));
    }

    fn dec_word_reg(&mut self, reg: WordReg) {
        if self.operand_size_override {
            let dreg = DwordReg::from_index(reg as u8);
            let val = self.regs.dword(dreg);
            let result = self.alu_dec_dword(val);
            self.regs.set_dword(dreg, result);
        } else {
            let val = self.regs.word(reg);
            let result = self.alu_dec_word(val);
            self.regs.set_word(reg, result);
        }
        self.clk(Self::timing(2, 1));
    }

    fn push_word_reg(&mut self, reg: WordReg, bus: &mut impl common::Bus) -> Step {
        let penalty = self.sp_penalty();
        if self.operand_size_override {
            let dreg = DwordReg::from_index(reg as u8);
            let val = self.regs.dword(dreg);
            self.push_dword(bus, val)?;
        } else {
            let val = self.regs.word(reg);
            self.push(bus, val)?;
        }
        self.clk(Self::timing(2, 1) + penalty);
        Ok(())
    }

    pub(super) fn push_sp(&mut self, bus: &mut impl common::Bus) -> Step {
        let penalty = self.sp_penalty();
        if self.operand_size_override {
            let esp = self.regs.dword(DwordReg::ESP);
            self.push_dword(bus, esp)?;
        } else {
            let sp = self.regs.word(WordReg::SP);
            self.push(bus, sp)?;
        }
        self.clk(Self::timing(2, 1) + penalty);
        Ok(())
    }

    fn pop_word_reg(&mut self, reg: WordReg, bus: &mut impl common::Bus) -> Step {
        let penalty = self.sp_penalty();
        if self.operand_size_override {
            let dreg = DwordReg::from_index(reg as u8);
            let val = self.pop_dword(bus)?;
            self.regs.set_dword(dreg, val);
        } else {
            let val = self.pop(bus)?;
            self.regs.set_word(reg, val);
        }
        self.clk(Self::timing(4, 4) + penalty);
        Ok(())
    }

    pub(super) fn push_seg(&mut self, seg: SegReg32, bus: &mut impl common::Bus) -> Step {
        let val = self.sregs[seg as usize];
        let penalty = self.sp_penalty();
        if self.operand_size_override {
            // i386/i486 quirk: a 32-bit PUSH of a 16-bit segment register
            // allocates a 4-byte stack slot but only writes the low 2 bytes;
            // the upper 2 bytes of the stack slot are left untouched. Pentium
            // and later CPUs zero-extend. We match the 386/486 manuals here.
            let sp_new = if self.use_esp() {
                let esp = self.regs.dword(DwordReg::ESP).wrapping_sub(4);
                self.regs.set_dword(DwordReg::ESP, esp);
                esp
            } else {
                let sp = self.regs.word(WordReg::SP).wrapping_sub(4);
                self.regs.set_word(WordReg::SP, sp);
                sp as u32
            };
            let base = self.seg_base(SegReg32::SS);
            let l0 = base.wrapping_add(sp_new);
            if l0 & 0xFFF <= 0xFFE {
                let a0 = self.translate_linear(l0, true, bus)?;
                bus.write_word(a0, val);
            } else {
                let a0 = self.translate_linear(l0, true, bus)?;
                let a1 = self.translate_linear(l0.wrapping_add(1), true, bus)?;
                bus.write_byte(a0, val as u8);
                bus.write_byte(a1, (val >> 8) as u8);
            }
        } else {
            self.push(bus, val)?;
        }
        self.clk(Self::timing(2, 3) + penalty);
        Ok(())
    }

    pub(super) fn pop_seg(&mut self, seg: SegReg32, bus: &mut impl common::Bus) -> Step {
        let penalty = self.sp_penalty();
        let slot_size = if self.operand_size_override { 4 } else { 2 };
        let was_use_esp = self.use_esp();
        let sp = if was_use_esp {
            self.regs.dword(DwordReg::ESP)
        } else {
            self.regs.word(WordReg::SP) as u32
        };
        self.check_segment_access(SegReg32::SS, sp, slot_size, false, bus)?;

        let base = self.seg_base(SegReg32::SS);
        let l0 = base.wrapping_add(sp);
        let val = if l0 & 0xFFF <= 0xFFE {
            let a0 = self.translate_linear(l0, false, bus)?;
            bus.read_word(a0)
        } else {
            let a0 = self.translate_linear(l0, false, bus)?;
            let a1 = self.translate_linear(l0.wrapping_add(1), false, bus)?;
            bus.read_byte(a0) as u16 | ((bus.read_byte(a1) as u16) << 8)
        };
        self.load_segment(seg, val, bus)?;
        let new_sp = sp.wrapping_add(slot_size);
        if was_use_esp {
            self.regs.set_dword(DwordReg::ESP, new_sp);
        } else {
            self.regs.set_word(WordReg::SP, new_sp as u16);
        }
        self.clk(Self::timing(7, 3) + penalty);
        Ok(())
    }

    fn pusha(&mut self, bus: &mut impl common::Bus) -> Step {
        // Atomic PUSHA: probe writability of all 8 stack slots before
        // committing SP, so a #PF mid-sequence leaves SP and memory
        // unchanged from the OS handler's perspective. (Sequential pushes
        // would commit SP between operations, leaving the saved-V86-ESP
        // slot of an inter-priv fault frame pointing at a partial state
        // that the IRET-back would observe.)
        let penalty = self.sp_penalty();
        let use_esp = self.use_esp();
        let sp_orig = if use_esp {
            self.regs.dword(DwordReg::ESP)
        } else {
            self.regs.word(WordReg::SP) as u32
        };
        let stack_offset = |delta: u32| -> u32 {
            if use_esp {
                sp_orig.wrapping_sub(delta)
            } else {
                (sp_orig as u16).wrapping_sub(delta as u16) as u32
            }
        };
        let ss_base = self.seg_base(SegReg32::SS);

        if self.operand_size_override {
            let values = [
                self.regs.dword(DwordReg::EAX),
                self.regs.dword(DwordReg::ECX),
                self.regs.dword(DwordReg::EDX),
                self.regs.dword(DwordReg::EBX),
                self.regs.dword(DwordReg::ESP),
                self.regs.dword(DwordReg::EBP),
                self.regs.dword(DwordReg::ESI),
                self.regs.dword(DwordReg::EDI),
            ];
            // Probe every byte of every slot before any commit.
            for i in 1..=8u32 {
                let offset = stack_offset(4 * i);
                self.check_segment_access(SegReg32::SS, offset, 4, true, bus)?;
                let l0 = ss_base.wrapping_add(offset);
                for b in 0..4u32 {
                    self.translate_linear(l0.wrapping_add(b), true, bus)?;
                }
            }
            // All slots accessible -- commit SP and do all writes. The
            // writes go through write_dword_seg which retranslates from
            // the TLB primed by the probe loop above.
            self.commit_sp(stack_offset(32));
            for (i, &val) in values.iter().enumerate() {
                let offset = stack_offset(4 * (i as u32 + 1));
                self.write_dword_seg(bus, SegReg32::SS, offset, val)?;
            }
        } else {
            let values = [
                self.regs.word(WordReg::AX),
                self.regs.word(WordReg::CX),
                self.regs.word(WordReg::DX),
                self.regs.word(WordReg::BX),
                self.regs.word(WordReg::SP),
                self.regs.word(WordReg::BP),
                self.regs.word(WordReg::SI),
                self.regs.word(WordReg::DI),
            ];
            for i in 1..=8u32 {
                let offset = stack_offset(2 * i);
                self.check_segment_access(SegReg32::SS, offset, 2, true, bus)?;
                let l0 = ss_base.wrapping_add(offset);
                for b in 0..2u32 {
                    self.translate_linear(l0.wrapping_add(b), true, bus)?;
                }
            }
            self.commit_sp(stack_offset(16));
            for (i, &val) in values.iter().enumerate() {
                let offset = stack_offset(2 * (i as u32 + 1));
                self.write_word_seg(bus, SegReg32::SS, offset, val)?;
            }
        }
        self.clk(Self::timing(18, 11) + penalty);
        Ok(())
    }

    fn popa(&mut self, bus: &mut impl common::Bus) -> Step {
        let penalty = self.sp_penalty();
        if self.operand_size_override {
            // Probe the entire stack window before any commit: snapshot
            // SP, peek 8 dwords, restore SP, then commit registers in the
            // final block. If any peek faults, no register has changed.
            let saved_sp = if self.use_esp() {
                self.regs.dword(DwordReg::ESP)
            } else {
                self.regs.word(WordReg::SP) as u32
            };
            let edi = match self.pop_dword(bus) {
                Ok(v) => v,
                Err(e) => {
                    self.commit_sp(saved_sp);
                    return Err(e);
                }
            };
            let esi = match self.pop_dword(bus) {
                Ok(v) => v,
                Err(e) => {
                    self.commit_sp(saved_sp);
                    return Err(e);
                }
            };
            let ebp = match self.pop_dword(bus) {
                Ok(v) => v,
                Err(e) => {
                    self.commit_sp(saved_sp);
                    return Err(e);
                }
            };
            let popped_esp = match self.pop_dword(bus) {
                Ok(v) => v,
                Err(e) => {
                    self.commit_sp(saved_sp);
                    return Err(e);
                }
            };
            let ebx = match self.pop_dword(bus) {
                Ok(v) => v,
                Err(e) => {
                    self.commit_sp(saved_sp);
                    return Err(e);
                }
            };
            let edx = match self.pop_dword(bus) {
                Ok(v) => v,
                Err(e) => {
                    self.commit_sp(saved_sp);
                    return Err(e);
                }
            };
            let ecx = match self.pop_dword(bus) {
                Ok(v) => v,
                Err(e) => {
                    self.commit_sp(saved_sp);
                    return Err(e);
                }
            };
            let eax = match self.pop_dword(bus) {
                Ok(v) => v,
                Err(e) => {
                    self.commit_sp(saved_sp);
                    return Err(e);
                }
            };
            self.regs.set_dword(DwordReg::EDI, edi);
            self.regs.set_dword(DwordReg::ESI, esi);
            self.regs.set_dword(DwordReg::EBP, ebp);
            if !self.use_esp() {
                // i386-specific POPAD quirk: with a 16-bit stack address size
                // the ESP slot is popped into ESP in full, and only then is SP
                // overwritten with its actual post-pop value. The popped
                // dword's upper half survives in ESP's upper 16 bits.
                // The Pentium dropped this and just does SP += 4.
                let sp_after = self.regs.word(WordReg::SP);
                let new_esp = (popped_esp & 0xFFFF_0000) | sp_after as u32;
                self.regs.set_dword(DwordReg::ESP, new_esp);
            }
            self.regs.set_dword(DwordReg::EBX, ebx);
            self.regs.set_dword(DwordReg::EDX, edx);
            self.regs.set_dword(DwordReg::ECX, ecx);
            self.regs.set_dword(DwordReg::EAX, eax);
        } else {
            let saved_sp = if self.use_esp() {
                self.regs.dword(DwordReg::ESP)
            } else {
                self.regs.word(WordReg::SP) as u32
            };
            let iy = match self.pop(bus) {
                Ok(v) => v,
                Err(e) => {
                    self.commit_sp(saved_sp);
                    return Err(e);
                }
            };
            let ix = match self.pop(bus) {
                Ok(v) => v,
                Err(e) => {
                    self.commit_sp(saved_sp);
                    return Err(e);
                }
            };
            let bp = match self.pop(bus) {
                Ok(v) => v,
                Err(e) => {
                    self.commit_sp(saved_sp);
                    return Err(e);
                }
            };
            let _discard = match self.pop(bus) {
                Ok(v) => v,
                Err(e) => {
                    self.commit_sp(saved_sp);
                    return Err(e);
                }
            };
            let bw = match self.pop(bus) {
                Ok(v) => v,
                Err(e) => {
                    self.commit_sp(saved_sp);
                    return Err(e);
                }
            };
            let dw = match self.pop(bus) {
                Ok(v) => v,
                Err(e) => {
                    self.commit_sp(saved_sp);
                    return Err(e);
                }
            };
            let cw = match self.pop(bus) {
                Ok(v) => v,
                Err(e) => {
                    self.commit_sp(saved_sp);
                    return Err(e);
                }
            };
            let aw = match self.pop(bus) {
                Ok(v) => v,
                Err(e) => {
                    self.commit_sp(saved_sp);
                    return Err(e);
                }
            };
            self.regs.set_word(WordReg::DI, iy);
            self.regs.set_word(WordReg::SI, ix);
            self.regs.set_word(WordReg::BP, bp);
            self.regs.set_word(WordReg::BX, bw);
            self.regs.set_word(WordReg::DX, dw);
            self.regs.set_word(WordReg::CX, cw);
            self.regs.set_word(WordReg::AX, aw);
        }
        self.clk(Self::timing(24, 9) + penalty);
        Ok(())
    }

    fn bound(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        if modrm >= 0xC0 {
            return Ok(());
        }
        self.calc_ea(modrm, bus);
        if self.operand_size_override {
            let val = self.regs.dword(self.reg_dword(modrm)) as i32;
            let ea_pen = if self.ea & 3 != 0 { 8 } else { 0 };
            let low = self.seg_read_dword(bus)?;
            let high = self.seg_read_dword_at(bus, 4)?;
            let low = low as i32;
            let high = high as i32;
            if val < low || val > high {
                let sp_pen = self.sp_penalty();
                self.raise_interrupt(5, bus)?;
                self.clk(Self::timing(56, 7) + ea_pen + sp_pen);
            } else {
                self.clk(Self::timing(10, 7) + ea_pen);
            }
        } else {
            let val = self.regs.word(self.reg_word(modrm)) as i16;
            let ea_pen = if self.ea & 1 == 1 { 8 } else { 0 };
            let low = self.seg_read_word(bus)?;
            let high = self.seg_read_word_at(bus, 2)?;
            let low = low as i16;
            let high = high as i16;
            if val < low || val > high {
                let sp_pen = self.sp_penalty();
                self.raise_interrupt(5, bus)?;
                self.clk(Self::timing(56, 7) + ea_pen + sp_pen);
            } else {
                self.clk(Self::timing(10, 7) + ea_pen);
            }
        }
        Ok(())
    }

    fn arpl(&mut self, bus: &mut impl common::Bus) -> Step {
        if !self.is_protected_mode() || self.is_virtual_mode() {
            self.raise_fault(6, bus)?;
            return Ok(());
        }

        let modrm = self.fetch(bus);
        let dst = self.get_rm_word(modrm, bus)?;
        let src_rpl = self.regs.word(self.reg_word(modrm)) & 3;
        let dst_rpl = dst & 3;
        if dst_rpl < src_rpl {
            let result = (dst & !3) | src_rpl;
            self.putback_rm_word(modrm, result, bus)?;
            self.flags.zero_val = 0; // ZF=1
        } else {
            self.flags.zero_val = 1; // ZF=0
        }
        self.clk_modrm(modrm, Self::timing(10, 9), Self::timing(11, 9));
        Ok(())
    }

    fn push_imm16(&mut self, bus: &mut impl common::Bus) -> Step {
        let penalty = self.sp_penalty();
        if self.operand_size_override {
            let val = self.fetchdword(bus);
            self.push_dword(bus, val)?;
        } else {
            let val = self.fetchword(bus);
            self.push(bus, val)?;
        }
        self.clk(Self::timing(2, 1) + penalty);
        Ok(())
    }

    fn push_imm8(&mut self, bus: &mut impl common::Bus) -> Step {
        let penalty = self.sp_penalty();
        if self.operand_size_override {
            let val = self.fetch(bus) as i8 as i32 as u32;
            self.push_dword(bus, val)?;
        } else {
            let val = self.fetch(bus) as i8 as u16;
            self.push(bus, val)?;
        }
        self.clk(Self::timing(2, 1) + penalty);
        Ok(())
    }

    fn imul_r16w_imm16(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        if self.operand_size_override {
            let src = self.get_rm_dword(modrm, bus)?;
            let src = src as i32 as i64;
            let imm = self.fetchdword(bus) as i32 as i64;
            let result = src * imm;
            let reg = self.reg_dword(modrm);
            self.regs.set_dword(reg, result as u32);
            self.flags.carry_val = if result < i32::MIN as i64 || result > i32::MAX as i64 {
                1
            } else {
                0
            };
        } else {
            let src = self.get_rm_word(modrm, bus)?;
            let src = src as i16 as i32;
            let imm = self.fetchword(bus) as i16 as i32;
            let result = src * imm;
            let reg = self.reg_word(modrm);
            self.regs.set_word(reg, result as u16);
            self.flags.carry_val = if !(-0x8000..=0x7FFF).contains(&result) {
                1
            } else {
                0
            };
        };
        self.flags.overflow_val = self.flags.carry_val;
        if self.operand_size_override {
            self.clk_modrm_word(modrm, Self::timing(38, 13), Self::timing(41, 13), 2);
        } else {
            self.clk_modrm_word(modrm, Self::timing(22, 13), Self::timing(25, 13), 1);
        }
        Ok(())
    }

    fn imul_r16w_imm8(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        if self.operand_size_override {
            let src = self.get_rm_dword(modrm, bus)?;
            let src = src as i32 as i64;
            let imm = self.fetch(bus) as i8 as i64;
            let result = src * imm;
            let reg = self.reg_dword(modrm);
            self.regs.set_dword(reg, result as u32);
            self.flags.carry_val = if result < i32::MIN as i64 || result > i32::MAX as i64 {
                1
            } else {
                0
            };
        } else {
            let src = self.get_rm_word(modrm, bus)?;
            let src = src as i16 as i32;
            let imm = self.fetch(bus) as i8 as i32;
            let result = src * imm;
            let reg = self.reg_word(modrm);
            self.regs.set_word(reg, result as u16);
            self.flags.carry_val = if !(-0x8000..=0x7FFF).contains(&result) {
                1
            } else {
                0
            };
        };
        self.flags.overflow_val = self.flags.carry_val;
        if self.operand_size_override {
            self.clk_modrm_word(modrm, Self::timing(14, 13), Self::timing(17, 13), 2);
        } else {
            self.clk_modrm_word(modrm, Self::timing(14, 13), Self::timing(17, 13), 1);
        }
        Ok(())
    }

    fn jcc(&mut self, bus: &mut impl common::Bus, condition: bool) {
        let disp = self.fetch(bus) as i8;
        if condition {
            self.apply_branch_disp8(disp);
            match CPU_MODEL {
                CPU_MODEL_386 => {
                    let m = self.next_instruction_length_approx(bus);
                    self.clk(7 + m);
                }
                CPU_MODEL_486 => self.clk(3),
                _ => {
                    unreachable!("Unhandled CPU_MODEL")
                }
            }
        } else {
            self.clk(Self::timing(3, 1));
        }
    }

    fn jcc_swapped(&mut self, bus: &mut impl common::Bus, condition: bool) {
        let disp = self.fetch(bus) as i8;
        if condition {
            self.apply_branch_disp8(disp);
            match CPU_MODEL {
                CPU_MODEL_386 => {
                    let m = self.next_instruction_length_approx(bus);
                    self.clk(7 + m);
                }
                CPU_MODEL_486 => self.clk(3),
                _ => {
                    unreachable!("Unhandled CPU_MODEL")
                }
            }
        } else {
            self.clk(Self::timing(3, 1));
        }
    }

    fn test_br8(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        let src = self.regs.byte(self.reg_byte(modrm));
        let dst = self.get_rm_byte(modrm, bus)?;
        self.alu_and_byte(dst, src);
        self.clk_modrm(modrm, Self::timing(2, 1), Self::timing(5, 2));
        Ok(())
    }

    fn test_wr16(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        if self.operand_size_override {
            let src = self.regs.dword(self.reg_dword(modrm));
            let dst = self.get_rm_dword(modrm, bus)?;
            self.alu_and_dword(dst, src);
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(5, 2), 2);
        } else {
            let src = self.regs.word(self.reg_word(modrm));
            let dst = self.get_rm_word(modrm, bus)?;
            self.alu_and_word(dst, src);
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(5, 2), 1);
        }
        Ok(())
    }

    fn test_al_imm8(&mut self, bus: &mut impl common::Bus) {
        let src = self.fetch(bus);
        let dst = self.regs.byte(ByteReg::AL);
        self.alu_and_byte(dst, src);
        self.clk(Self::timing(2, 1));
    }

    fn test_aw_imm16(&mut self, bus: &mut impl common::Bus) {
        if self.operand_size_override {
            let src = self.fetchdword(bus);
            let dst = self.regs.dword(DwordReg::EAX);
            self.alu_and_dword(dst, src);
        } else {
            let src = self.fetchword(bus);
            let dst = self.regs.word(WordReg::AX);
            self.alu_and_word(dst, src);
        }
        self.clk(Self::timing(2, 1));
    }

    fn xchg_br8(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        let reg = self.reg_byte(modrm);
        let reg_val = self.regs.byte(reg);
        let rm_val = self.get_rm_byte_for_update(modrm, bus)?;
        self.regs.set_byte(reg, rm_val);
        self.putback_rm_byte(modrm, reg_val, bus)?;
        self.clk_modrm(modrm, Self::timing(3, 3), Self::timing(5, 5));
        Ok(())
    }

    fn xchg_wr16(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        if self.operand_size_override {
            let reg = self.reg_dword(modrm);
            let reg_val = self.regs.dword(reg);
            let rm_val = self.get_rm_dword_for_update(modrm, bus)?;
            self.regs.set_dword(reg, rm_val);
            self.putback_rm_dword(modrm, reg_val, bus)?;
            self.clk_modrm_word(modrm, Self::timing(3, 3), Self::timing(5, 5), 4);
        } else {
            let reg = self.reg_word(modrm);
            let reg_val = self.regs.word(reg);
            let rm_val = self.get_rm_word_for_update(modrm, bus)?;
            self.regs.set_word(reg, rm_val);
            self.putback_rm_word(modrm, reg_val, bus)?;
            self.clk_modrm_word(modrm, Self::timing(3, 3), Self::timing(5, 5), 2);
        }
        Ok(())
    }

    fn xchg_aw(&mut self, reg: WordReg) {
        if self.operand_size_override {
            let dreg = DwordReg::from_index(reg as u8);
            let eax = self.regs.dword(DwordReg::EAX);
            let val = self.regs.dword(dreg);
            self.regs.set_dword(DwordReg::EAX, val);
            self.regs.set_dword(dreg, eax);
        } else {
            let aw = self.regs.word(WordReg::AX);
            let val = self.regs.word(reg);
            self.regs.set_word(WordReg::AX, val);
            self.regs.set_word(reg, aw);
        }
        self.clk(Self::timing(3, 3));
    }

    fn mov_br8(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        let val = self.regs.byte(self.reg_byte(modrm));
        self.put_rm_byte(modrm, val, bus)?;
        self.clk_modrm(modrm, Self::timing(2, 1), Self::timing(2, 1));
        Ok(())
    }

    fn mov_wr16(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        if self.operand_size_override {
            let val = self.regs.dword(self.reg_dword(modrm));
            self.put_rm_dword(modrm, val, bus)?;
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(2, 1), 2);
        } else {
            let val = self.regs.word(self.reg_word(modrm));
            self.put_rm_word(modrm, val, bus)?;
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(2, 1), 1);
        }
        Ok(())
    }

    fn mov_r8b(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        let val = self.get_rm_byte(modrm, bus)?;
        let reg = self.reg_byte(modrm);
        self.regs.set_byte(reg, val);
        self.clk_modrm(modrm, Self::timing(2, 1), Self::timing(4, 1));
        Ok(())
    }

    fn mov_r16w(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        if self.operand_size_override {
            let val = self.get_rm_dword(modrm, bus)?;
            let reg = self.reg_dword(modrm);
            self.regs.set_dword(reg, val);
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(4, 1), 2);
        } else {
            let val = self.get_rm_word(modrm, bus)?;
            let reg = self.reg_word(modrm);
            self.regs.set_word(reg, val);
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(4, 1), 1);
        }
        Ok(())
    }

    fn mov_rm_sreg(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        let seg_index = (modrm >> 3) & 7;
        if seg_index > 5 {
            return self.raise_fault(6, bus)?;
        }
        let seg = SegReg32::from_index(seg_index);
        let val = self.sregs[seg as usize];
        if self.operand_size_override && modrm >= 0xC0 {
            let reg = self.rm_dword(modrm);
            self.regs.set_dword(reg, val as u32);
            self.clk_modrm_word(modrm, Self::timing(2, 3), Self::timing(2, 3), 2);
        } else {
            self.put_rm_word(modrm, val, bus)?;
            self.clk_modrm_word(modrm, Self::timing(2, 3), Self::timing(2, 3), 1);
        }
        Ok(())
    }

    fn mov_sreg_rm(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        let seg_index = (modrm >> 3) & 7;
        if seg_index > 5 || seg_index == 1 {
            self.raise_fault(6, bus)?;
            return Ok(());
        }
        let val = if self.operand_size_override && modrm >= 0xC0 {
            let v = self.get_rm_dword(modrm, bus)?;
            v as u16
        } else {
            self.get_rm_word(modrm, bus)?
        };
        let seg = SegReg32::from_index(seg_index);
        self.load_segment(seg, val, bus)?;
        if seg == SegReg32::SS {
            self.inhibit_all = 1;
        }
        self.clk_modrm_word(modrm, Self::timing(2, 3), Self::timing(5, 9), 1);
        Ok(())
    }

    fn lea(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        if modrm >= 0xC0 {
            self.invalid(bus)?;
            return Ok(());
        }
        self.calc_ea(modrm, bus);
        if self.operand_size_override {
            let reg = self.reg_dword(modrm);
            let val = self.eo32;
            self.regs.set_dword(reg, val);
        } else {
            let reg = self.reg_word(modrm);
            let val = self.eo;
            self.regs.set_word(reg, val);
        }
        self.clk(Self::timing(2, 1));
        Ok(())
    }

    fn pop_rm(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        let sp_pen = self.sp_penalty();

        if modrm >= 0xC0 {
            if self.operand_size_override {
                let val = self.pop_dword(bus)?;
                self.put_rm_dword(modrm, val, bus)?;
            } else {
                let val = self.pop(bus)?;
                self.put_rm_word(modrm, val, bus)?;
            }
        } else {
            let use_esp = self.use_esp();
            let old_sp = if use_esp {
                self.regs.dword(DwordReg::ESP)
            } else {
                self.regs.word(WordReg::SP) as u32
            };
            let pop_bytes: u32 = if self.operand_size_override { 4 } else { 2 };
            let new_sp = if use_esp {
                old_sp.wrapping_add(pop_bytes)
            } else {
                (old_sp as u16).wrapping_add(pop_bytes as u16) as u32
            };
            let val = if self.operand_size_override {
                self.read_dword_seg(bus, SegReg32::SS, old_sp)?
            } else {
                self.read_word_seg(bus, SegReg32::SS, old_sp)? as u32
            };

            self.commit_sp(new_sp);
            self.calc_ea(modrm, bus);
            let ea_seg = self.ea_seg;
            let eo32 = self.eo32;
            self.commit_sp(old_sp);

            self.check_segment_access(ea_seg, eo32, pop_bytes, true, bus)?;
            let base = self.seg_base(ea_seg);
            let l0 = base.wrapping_add(eo32);
            for b in 0..pop_bytes {
                self.translate_linear(l0.wrapping_add(b), true, bus)?;
            }

            self.commit_sp(new_sp);
            if self.operand_size_override {
                self.write_dword_seg(bus, ea_seg, eo32, val)?;
            } else {
                self.write_word_seg(bus, ea_seg, eo32, val as u16)?;
            }
        }
        if modrm >= 0xC0 {
            self.clk(Self::timing(4, 4) + sp_pen);
        } else {
            let ea_pen = if self.operand_size_override {
                if self.ea & 3 != 0 {
                    Self::timing(4, 3)
                } else {
                    0
                }
            } else if self.ea & 1 != 0 {
                Self::timing(4, 3)
            } else {
                0
            };
            self.clk(Self::timing(5, 5) + sp_pen + ea_pen);
        }
        Ok(())
    }

    fn cbw(&mut self) {
        if self.operand_size_override {
            let ax = self.regs.word(WordReg::AX) as i16 as i32 as u32;
            self.regs.set_dword(DwordReg::EAX, ax);
        } else {
            let al = self.regs.byte(ByteReg::AL) as i8 as i16 as u16;
            self.regs.set_word(WordReg::AX, al);
        }
        self.clk(Self::timing(3, 3));
    }

    fn cwd(&mut self) {
        if self.operand_size_override {
            let eax = self.regs.dword(DwordReg::EAX) as i32;
            self.regs
                .set_dword(DwordReg::EDX, if eax < 0 { 0xFFFF_FFFF } else { 0 });
        } else {
            let aw = self.regs.word(WordReg::AX) as i16;
            self.regs
                .set_word(WordReg::DX, if aw < 0 { 0xFFFF } else { 0 });
        }
        self.clk(Self::timing(2, 3));
    }

    fn call_far(&mut self, bus: &mut impl common::Bus) -> Step {
        if self.operand_size_override {
            let penalty = self.sp_penalty();
            let offset = self.fetchdword(bus);
            let segment = self.fetchword(bus);
            let cs = self.sregs[SegReg32::CS as usize];
            let eip = self.ip_upper | self.ip as u32;
            if self.is_protected_mode() && !self.is_virtual_mode() {
                self.code_descriptor(segment, offset, super::TaskType::Call, cs, eip, bus)?;
            } else {
                self.far_push_real_v86_dword(bus, cs as u32, eip)?;
                self.load_segment(SegReg32::CS, segment, bus)?;
                self.ip = offset as u16;
                self.ip_upper = offset & 0xFFFF_0000;
            }
            match CPU_MODEL {
                CPU_MODEL_386 => {
                    let m = self.next_instruction_length_approx(bus);
                    self.clk(17 + m + penalty);
                }
                CPU_MODEL_486 => self.clk(18 + penalty),
                _ => {
                    unreachable!("Unhandled CPU_MODEL")
                }
            }
        } else {
            let penalty = self.sp_penalty();
            let offset = self.fetchword(bus);
            let segment = self.fetchword(bus);
            let cs = self.sregs[SegReg32::CS as usize];
            let ip = self.ip;
            if self.is_protected_mode() && !self.is_virtual_mode() {
                self.code_descriptor(
                    segment,
                    offset as u32,
                    super::TaskType::Call,
                    cs,
                    ip as u32,
                    bus,
                )?;
            } else {
                self.far_push_real_v86_word(bus, cs, ip)?;
                self.load_segment(SegReg32::CS, segment, bus)?;
                self.ip = offset;
                self.ip_upper = 0;
            }
            match CPU_MODEL {
                CPU_MODEL_386 => {
                    let m = self.next_instruction_length_approx(bus);
                    self.clk(17 + m + penalty);
                }
                CPU_MODEL_486 => self.clk(18 + penalty),
                _ => {
                    unreachable!("Unhandled CPU_MODEL")
                }
            }
        }
        Ok(())
    }

    /// Probe-then-commit real/V86 far push pair (CS first, then IP).
    pub(super) fn far_push_real_v86_word(
        &mut self,
        bus: &mut impl common::Bus,
        cs: u16,
        ip: u16,
    ) -> Step {
        let use_esp = self.use_esp();
        let sp_orig = if use_esp {
            self.regs.dword(DwordReg::ESP)
        } else {
            self.regs.word(WordReg::SP) as u32
        };
        let stack_offset = |delta: u32| -> u32 {
            if use_esp {
                sp_orig.wrapping_sub(delta)
            } else {
                (sp_orig as u16).wrapping_sub(delta as u16) as u32
            }
        };
        let ss_base = self.seg_base(SegReg32::SS);
        for i in 1..=2u32 {
            let off = stack_offset(2 * i);
            self.check_segment_access(SegReg32::SS, off, 2, true, bus)?;
            let l0 = ss_base.wrapping_add(off);
            self.translate_linear(l0, true, bus)?;
            self.translate_linear(l0.wrapping_add(1), true, bus)?;
        }
        self.commit_sp(stack_offset(4));
        self.write_word_seg(bus, SegReg32::SS, stack_offset(2), cs)?;
        self.write_word_seg(bus, SegReg32::SS, stack_offset(4), ip)?;
        Ok(())
    }

    /// 32-bit variant of `far_push_real_v86_word`: pushes CS (zero-extended)
    /// and EIP as dwords.
    pub(super) fn far_push_real_v86_dword(
        &mut self,
        bus: &mut impl common::Bus,
        cs: u32,
        eip: u32,
    ) -> Step {
        let use_esp = self.use_esp();
        let sp_orig = if use_esp {
            self.regs.dword(DwordReg::ESP)
        } else {
            self.regs.word(WordReg::SP) as u32
        };
        let stack_offset = |delta: u32| -> u32 {
            if use_esp {
                sp_orig.wrapping_sub(delta)
            } else {
                (sp_orig as u16).wrapping_sub(delta as u16) as u32
            }
        };
        let ss_base = self.seg_base(SegReg32::SS);
        for i in 1..=2u32 {
            let off = stack_offset(4 * i);
            self.check_segment_access(SegReg32::SS, off, 4, true, bus)?;
            let l0 = ss_base.wrapping_add(off);
            for b in 0..4u32 {
                self.translate_linear(l0.wrapping_add(b), true, bus)?;
            }
        }
        self.commit_sp(stack_offset(8));
        self.write_dword_seg(bus, SegReg32::SS, stack_offset(4), cs)?;
        self.write_dword_seg(bus, SegReg32::SS, stack_offset(8), eip)?;
        Ok(())
    }

    fn call_near(&mut self, bus: &mut impl common::Bus) -> Step {
        if self.operand_size_override {
            let penalty = self.sp_penalty();
            let disp = self.fetchdword(bus) as i32;
            let return_eip = self.ip_upper | self.ip as u32;
            self.push_dword(bus, return_eip)?;
            let target = return_eip.wrapping_add(disp as u32);
            self.ip = target as u16;
            self.ip_upper = target & 0xFFFF_0000;
            match CPU_MODEL {
                CPU_MODEL_386 => {
                    let m = self.next_instruction_length_approx(bus);
                    self.clk(7 + m + penalty);
                }
                CPU_MODEL_486 => self.clk(3 + penalty),
                _ => {
                    unreachable!("Unhandled CPU_MODEL")
                }
            }
        } else {
            let penalty = self.sp_penalty();
            let disp = self.fetchword(bus) as i16;
            self.push(bus, self.ip)?;
            self.ip = self.ip.wrapping_add(disp as u16);
            self.ip_upper = 0;
            match CPU_MODEL {
                CPU_MODEL_386 => {
                    let m = self.next_instruction_length_approx(bus);
                    self.clk(7 + m + penalty);
                }
                CPU_MODEL_486 => self.clk(3 + penalty),
                _ => {
                    unreachable!("Unhandled CPU_MODEL")
                }
            }
        }
        Ok(())
    }

    fn jmp_near(&mut self, bus: &mut impl common::Bus) {
        if self.operand_size_override {
            let disp = self.fetchdword(bus) as i32;
            let eip = self.ip_upper | self.ip as u32;
            let target = eip.wrapping_add(disp as u32);
            self.ip = target as u16;
            self.ip_upper = target & 0xFFFF_0000;
            match CPU_MODEL {
                CPU_MODEL_386 => {
                    let m = self.next_instruction_length_approx(bus);
                    self.clk(7 + m);
                }
                CPU_MODEL_486 => self.clk(3),
                _ => {
                    unreachable!("Unhandled CPU_MODEL")
                }
            }
        } else {
            let disp = self.fetchword(bus) as i16;
            self.ip = self.ip.wrapping_add(disp as u16);
            self.ip_upper = 0;
            match CPU_MODEL {
                CPU_MODEL_386 => {
                    let m = self.next_instruction_length_approx(bus);
                    self.clk(7 + m);
                }
                CPU_MODEL_486 => self.clk(3),
                _ => {
                    unreachable!("Unhandled CPU_MODEL")
                }
            }
        }
    }

    fn jmp_far(&mut self, bus: &mut impl common::Bus) -> Step {
        if self.operand_size_override {
            let offset = self.fetchdword(bus);
            let segment = self.fetchword(bus);
            if self.is_protected_mode() && !self.is_virtual_mode() {
                self.code_descriptor(segment, offset, super::TaskType::Jmp, 0, 0, bus)?;
            } else {
                self.load_segment(SegReg32::CS, segment, bus)?;
                self.ip = offset as u16;
                self.ip_upper = offset & 0xFFFF_0000;
            }
            match CPU_MODEL {
                CPU_MODEL_386 => {
                    let m = self.next_instruction_length_approx(bus);
                    self.clk(12 + m);
                }
                CPU_MODEL_486 => self.clk(17),
                _ => {
                    unreachable!("Unhandled CPU_MODEL")
                }
            }
        } else {
            let offset = self.fetchword(bus);
            let segment = self.fetchword(bus);
            if self.is_protected_mode() && !self.is_virtual_mode() {
                self.code_descriptor(segment, offset as u32, super::TaskType::Jmp, 0, 0, bus)?;
            } else {
                self.load_segment(SegReg32::CS, segment, bus)?;
                self.ip = offset;
                self.ip_upper = 0;
            }
            match CPU_MODEL {
                CPU_MODEL_386 => {
                    let m = self.next_instruction_length_approx(bus);
                    self.clk(12 + m);
                }
                CPU_MODEL_486 => self.clk(17),
                _ => {
                    unreachable!("Unhandled CPU_MODEL")
                }
            }
        }
        Ok(())
    }

    fn jmp_short(&mut self, bus: &mut impl common::Bus) {
        let disp = self.fetch(bus) as i8;
        self.apply_branch_disp8(disp);
        match CPU_MODEL {
            CPU_MODEL_386 => {
                let m = self.next_instruction_length_approx(bus);
                self.clk(7 + m);
            }
            CPU_MODEL_486 => self.clk(3),
            _ => {
                unreachable!("Unhandled CPU_MODEL")
            }
        }
    }

    fn ret_near(&mut self, bus: &mut impl common::Bus) -> Step {
        let penalty = self.sp_penalty();
        if self.operand_size_override {
            let eip = self.pop_dword(bus)?;
            self.ip = eip as u16;
            self.ip_upper = eip & 0xFFFF_0000;
        } else {
            let ip = self.pop(bus)?;
            self.ip = ip;
            self.ip_upper = 0;
        }
        match CPU_MODEL {
            CPU_MODEL_386 => {
                let m = self.next_instruction_length_approx(bus);
                self.clk(10 + m + penalty);
            }
            CPU_MODEL_486 => self.clk(5 + penalty),
            _ => {
                unreachable!("Unhandled CPU_MODEL")
            }
        }
        Ok(())
    }

    fn ret_near_imm(&mut self, bus: &mut impl common::Bus) -> Step {
        let penalty = self.sp_penalty();
        let imm = self.fetchword(bus);
        if self.operand_size_override {
            let eip = self.pop_dword(bus)?;
            self.ip = eip as u16;
            self.ip_upper = eip & 0xFFFF_0000;
        } else {
            let ip = self.pop(bus)?;
            self.ip = ip;
            self.ip_upper = 0;
        }
        if self.use_esp() {
            let stack_pointer = self.regs.dword(DwordReg::ESP).wrapping_add(imm as u32);
            self.regs.set_dword(DwordReg::ESP, stack_pointer);
        } else {
            let stack_pointer = self.regs.word(WordReg::SP).wrapping_add(imm);
            self.regs.set_word(WordReg::SP, stack_pointer);
        }
        match CPU_MODEL {
            CPU_MODEL_386 => {
                let m = self.next_instruction_length_approx(bus);
                self.clk(10 + m + penalty);
            }
            CPU_MODEL_486 => self.clk(5 + penalty),
            _ => {
                unreachable!("Unhandled CPU_MODEL")
            }
        }
        Ok(())
    }

    fn ret_far(&mut self, bus: &mut impl common::Bus) -> Step {
        let penalty = self.sp_penalty();

        if !self.is_protected_mode() || self.is_virtual_mode() {
            let use_esp = self.use_esp();
            let sp = if use_esp {
                self.regs.dword(DwordReg::ESP)
            } else {
                self.regs.word(WordReg::SP) as u32
            };
            let stack_offset = |delta: u32| -> u32 {
                if use_esp {
                    sp.wrapping_add(delta)
                } else {
                    (sp as u16).wrapping_add(delta as u16) as u32
                }
            };
            let ss_base = self.seg_base(SegReg32::SS);
            if self.operand_size_override {
                let new_eip = self.read_dword_linear(bus, ss_base.wrapping_add(stack_offset(0)))?;
                let new_cs_dword =
                    self.read_dword_linear(bus, ss_base.wrapping_add(stack_offset(4)))?;
                self.load_segment(SegReg32::CS, new_cs_dword as u16, bus)?;
                self.commit_sp(stack_offset(8));
                self.ip = new_eip as u16;
                self.ip_upper = new_eip & 0xFFFF_0000;
            } else {
                let new_ip = self.read_word_linear(bus, ss_base.wrapping_add(stack_offset(0)))?;
                let new_cs = self.read_word_linear(bus, ss_base.wrapping_add(stack_offset(2)))?;
                self.load_segment(SegReg32::CS, new_cs, bus)?;
                self.commit_sp(stack_offset(4));
                self.ip = new_ip;
                self.ip_upper = 0;
            }
            match CPU_MODEL {
                CPU_MODEL_386 => {
                    let m = self.next_instruction_length_approx(bus);
                    self.clk(18 + m + penalty);
                }
                CPU_MODEL_486 => self.clk(13 + penalty),
                _ => {
                    unreachable!("Unhandled CPU_MODEL")
                }
            }
            return Ok(());
        }

        // Protected mode far return.
        let sp = if self.use_esp() {
            self.regs.dword(DwordReg::ESP)
        } else {
            self.regs.word(WordReg::SP) as u32
        };
        let ss_base = self.seg_base(SegReg32::SS);

        if self.operand_size_override {
            let new_eip = self.read_dword_linear(bus, ss_base.wrapping_add(sp))?;
            let new_cs_dword =
                self.read_dword_linear(bus, ss_base.wrapping_add(sp.wrapping_add(4)))?;
            let new_cs = new_cs_dword as u16;

            let new_rpl = new_cs & 3;
            let old_cpl = self.cpl();

            if new_rpl > old_cpl {
                let new_esp =
                    self.read_dword_linear(bus, ss_base.wrapping_add(sp.wrapping_add(8)))?;
                let new_ss_dword =
                    self.read_dword_linear(bus, ss_base.wrapping_add(sp.wrapping_add(12)))?;
                let new_ss = new_ss_dword as u16;

                self.commit_retf_inter_priv(new_cs, new_eip, new_ss, new_esp, new_rpl, bus)?;
            } else {
                self.commit_retf_intra_priv(new_cs, new_eip, sp.wrapping_add(8), bus)?;
            }
        } else {
            let new_ip = self.read_word_linear(bus, ss_base.wrapping_add(sp))?;
            let new_cs = self.read_word_linear(bus, ss_base.wrapping_add(sp.wrapping_add(2)))?;

            let new_rpl = new_cs & 3;
            let old_cpl = self.cpl();

            if new_rpl > old_cpl {
                let new_sp =
                    self.read_word_linear(bus, ss_base.wrapping_add(sp.wrapping_add(4)))?;
                let new_ss =
                    self.read_word_linear(bus, ss_base.wrapping_add(sp.wrapping_add(6)))?;

                self.commit_retf_inter_priv(
                    new_cs,
                    new_ip as u32,
                    new_ss,
                    new_sp as u32,
                    new_rpl,
                    bus,
                )?;
            } else {
                self.commit_retf_intra_priv(new_cs, new_ip as u32, sp.wrapping_add(4), bus)?;
            }
        }

        match CPU_MODEL {
            CPU_MODEL_386 => {
                let m = self.next_instruction_length_approx(bus);
                self.clk(18 + m + penalty);
            }
            CPU_MODEL_486 => self.clk(13 + penalty),
            _ => {
                unreachable!("Unhandled CPU_MODEL")
            }
        }
        Ok(())
    }

    fn ret_far_imm(&mut self, bus: &mut impl common::Bus) -> Step {
        let penalty = self.sp_penalty();
        let imm = self.fetchword(bus);

        if !self.is_protected_mode() || self.is_virtual_mode() {
            let use_esp = self.use_esp();
            let sp = if use_esp {
                self.regs.dword(DwordReg::ESP)
            } else {
                self.regs.word(WordReg::SP) as u32
            };
            let stack_offset = |delta: u32| -> u32 {
                if use_esp {
                    sp.wrapping_add(delta)
                } else {
                    (sp as u16).wrapping_add(delta as u16) as u32
                }
            };
            let ss_base = self.seg_base(SegReg32::SS);
            if self.operand_size_override {
                let new_eip = self.read_dword_linear(bus, ss_base.wrapping_add(stack_offset(0)))?;
                let new_cs_dword =
                    self.read_dword_linear(bus, ss_base.wrapping_add(stack_offset(4)))?;
                self.load_segment(SegReg32::CS, new_cs_dword as u16, bus)?;
                self.commit_sp(stack_offset(8u32.wrapping_add(imm as u32)));
                self.ip = new_eip as u16;
                self.ip_upper = new_eip & 0xFFFF_0000;
            } else {
                let new_ip = self.read_word_linear(bus, ss_base.wrapping_add(stack_offset(0)))?;
                let new_cs = self.read_word_linear(bus, ss_base.wrapping_add(stack_offset(2)))?;
                self.load_segment(SegReg32::CS, new_cs, bus)?;
                self.commit_sp(stack_offset(4u32.wrapping_add(imm as u32)));
                self.ip = new_ip;
                self.ip_upper = 0;
            }
            match CPU_MODEL {
                CPU_MODEL_386 => {
                    let m = self.next_instruction_length_approx(bus);
                    self.clk(18 + m + penalty);
                }
                CPU_MODEL_486 => self.clk(14 + penalty),
                _ => {
                    unreachable!("Unhandled CPU_MODEL")
                }
            }
            return Ok(());
        }

        // Protected mode far return with immediate.
        let sp = if self.use_esp() {
            self.regs.dword(DwordReg::ESP)
        } else {
            self.regs.word(WordReg::SP) as u32
        };
        let ss_base = self.seg_base(SegReg32::SS);
        let imm32 = imm as u32;

        if self.operand_size_override {
            let new_eip = self.read_dword_linear(bus, ss_base.wrapping_add(sp))?;
            let new_cs_dword =
                self.read_dword_linear(bus, ss_base.wrapping_add(sp.wrapping_add(4)))?;
            let new_cs = new_cs_dword as u16;

            let new_rpl = new_cs & 3;
            let old_cpl = self.cpl();

            if new_rpl > old_cpl {
                let sp_ss_base = sp.wrapping_add(8).wrapping_add(imm32);
                let new_esp = self.read_dword_linear(bus, ss_base.wrapping_add(sp_ss_base))?;
                let new_ss_dword =
                    self.read_dword_linear(bus, ss_base.wrapping_add(sp_ss_base.wrapping_add(4)))?;
                let new_ss = new_ss_dword as u16;

                let adj_esp = new_esp.wrapping_add(imm32);
                self.commit_retf_inter_priv(new_cs, new_eip, new_ss, adj_esp, new_rpl, bus)?;
            } else {
                let new_sp_val = sp.wrapping_add(8).wrapping_add(imm32);
                self.commit_retf_intra_priv(new_cs, new_eip, new_sp_val, bus)?;
            }
        } else {
            let new_ip = self.read_word_linear(bus, ss_base.wrapping_add(sp))?;
            let new_cs = self.read_word_linear(bus, ss_base.wrapping_add(sp.wrapping_add(2)))?;

            let new_rpl = new_cs & 3;
            let old_cpl = self.cpl();

            if new_rpl > old_cpl {
                let sp_ss_base = sp.wrapping_add(4).wrapping_add(imm32);
                let new_sp = self.read_word_linear(bus, ss_base.wrapping_add(sp_ss_base))?;
                let new_ss =
                    self.read_word_linear(bus, ss_base.wrapping_add(sp_ss_base.wrapping_add(2)))?;

                let adj_sp = new_sp.wrapping_add(imm);
                self.commit_retf_inter_priv(
                    new_cs,
                    new_ip as u32,
                    new_ss,
                    adj_sp as u32,
                    new_rpl,
                    bus,
                )?;
            } else {
                let new_sp_val = sp.wrapping_add(4).wrapping_add(imm32);
                self.commit_retf_intra_priv(new_cs, new_ip as u32, new_sp_val, bus)?;
            }
        }

        match CPU_MODEL {
            CPU_MODEL_386 => {
                let m = self.next_instruction_length_approx(bus);
                self.clk(18 + m + penalty);
            }
            CPU_MODEL_486 => self.clk(14 + penalty),
            _ => {
                unreachable!("Unhandled CPU_MODEL")
            }
        }
        Ok(())
    }

    /// Inter-privilege RET FAR commit: validates the new CS and SS plus all
    /// four data segments at the new CPL, then commits in a fault-atomic
    /// order.
    fn commit_retf_inter_priv(
        &mut self,
        new_cs: u16,
        new_eip: u32,
        new_ss: u16,
        new_esp: u32,
        new_rpl: u16,
        bus: &mut impl common::Bus,
    ) -> Step {
        let ss_validation = match self.validate_ss_for_iret_return(new_ss, new_rpl, bus)? {
            Err((vector, error_code)) => {
                return self.raise_fault_with_code(vector, error_code, bus);
            }
            Ok(validation) => validation,
        };

        let cs_validation = match self.validate_cs_for_return(new_cs, new_eip, bus)? {
            Err((vector, error_code)) => {
                return self.raise_fault_with_code(vector, error_code, bus);
            }
            Ok(validation) => validation,
        };

        let new_cpl = new_rpl;
        let ds_decision = self.check_data_segment_at_cpl(SegReg32::DS, new_cpl, bus)?;
        let es_decision = self.check_data_segment_at_cpl(SegReg32::ES, new_cpl, bus)?;
        let fs_decision = self.check_data_segment_at_cpl(SegReg32::FS, new_cpl, bus)?;
        let gs_decision = self.check_data_segment_at_cpl(SegReg32::GS, new_cpl, bus)?;

        self.set_accessed_bit(cs_validation.adjusted_selector, bus)?;
        self.set_loaded_segment_cache(
            SegReg32::CS,
            cs_validation.adjusted_selector,
            cs_validation.descriptor,
        );
        self.ip = new_eip as u16;
        self.ip_upper = new_eip & 0xFFFF_0000;

        self.set_accessed_bit(new_ss, bus)?;
        self.set_loaded_segment_cache(SegReg32::SS, new_ss, ss_validation.descriptor);
        if self.use_esp() {
            self.regs.set_dword(DwordReg::ESP, new_esp);
        } else {
            self.regs.set_word(WordReg::SP, new_esp as u16);
        }

        self.apply_data_segment_decision(SegReg32::DS, ds_decision);
        self.apply_data_segment_decision(SegReg32::ES, es_decision);
        self.apply_data_segment_decision(SegReg32::FS, fs_decision);
        self.apply_data_segment_decision(SegReg32::GS, gs_decision);
        Ok(())
    }

    /// Intra-privilege RET FAR commit: validates the new CS without touching
    /// any state, then commits CS A-bit, cache, ESP, and IP. No data-segment
    /// revalidation (CPL is unchanged).
    fn commit_retf_intra_priv(
        &mut self,
        new_cs: u16,
        new_eip: u32,
        new_esp: u32,
        bus: &mut impl common::Bus,
    ) -> Step {
        let cs_validation = match self.validate_cs_for_return(new_cs, new_eip, bus)? {
            Err((vector, error_code)) => {
                return self.raise_fault_with_code(vector, error_code, bus);
            }
            Ok(validation) => validation,
        };

        self.set_accessed_bit(cs_validation.adjusted_selector, bus)?;
        self.set_loaded_segment_cache(
            SegReg32::CS,
            cs_validation.adjusted_selector,
            cs_validation.descriptor,
        );
        if self.use_esp() {
            self.regs.set_dword(DwordReg::ESP, new_esp);
        } else {
            self.regs.set_word(WordReg::SP, new_esp as u16);
        }
        self.ip = new_eip as u16;
        self.ip_upper = new_eip & 0xFFFF_0000;
        Ok(())
    }

    fn pushf(&mut self, bus: &mut impl common::Bus) -> Step {
        if self.is_virtual_mode() && self.flags.iopl < 3 {
            self.raise_fault_with_code(13, 0, bus)?;
            return Ok(());
        }
        let penalty = self.sp_penalty();
        if self.operand_size_override {
            // PUSHFD: RF (bit 16) is masked. VM (bit 17) is always included.
            // AC (bit 18, 486+) is included; remaining bits 19-31 push as 0.
            let upper_mask = Self::eflags_upper_writable() & !EFLAGS_RESUME_FLAG;
            let flags_val = (self.eflags_upper & upper_mask) | self.flags.compress() as u32;
            self.push_dword(bus, flags_val)?;
        } else {
            let flags_val = self.flags.compress();
            self.push(bus, flags_val)?;
        }
        let base = match CPU_MODEL {
            CPU_MODEL_386 => 4,
            CPU_MODEL_486 => {
                if self.is_protected_mode() && !self.is_virtual_mode() {
                    3
                } else {
                    4
                }
            }
            _ => unreachable!("Unhandled CPU_MODEL"),
        };
        self.clk(base + penalty);
        Ok(())
    }

    fn popf(&mut self, bus: &mut impl common::Bus) -> Step {
        if self.is_virtual_mode() && self.flags.iopl < 3 {
            self.raise_fault_with_code(13, 0, bus)?;
            return Ok(());
        }
        self.preserve_resume_flag = true;
        let penalty = self.sp_penalty();
        let cpl = self.cpl();
        let pm = self.is_protected_mode();
        if self.operand_size_override {
            let val = self.pop_dword(bus)?;
            self.flags.load_flags(val as u16, cpl, pm);
            // VM (bit 17) is not modifiable via POPFD (only IRET at CPL=0).
            // RF (bit 16) is not modified by POPFD.
            // AC (bit 18, 486+) follows the popped value.
            let ac_mask = Self::eflags_upper_writable() & EFLAGS_ALIGNMENT_CHECK_FLAG;
            self.eflags_upper = (self.eflags_upper & !ac_mask) | (val & ac_mask);
        } else {
            let val = self.pop(bus)?;
            self.flags.load_flags(val, cpl, pm);
        }
        let base = match CPU_MODEL {
            CPU_MODEL_386 => 5,
            CPU_MODEL_486 => {
                if self.is_protected_mode() && !self.is_virtual_mode() {
                    6
                } else {
                    9
                }
            }
            _ => unreachable!("Unhandled CPU_MODEL"),
        };
        self.clk(base + penalty);
        Ok(())
    }

    fn sahf(&mut self) {
        let ah = self.regs.byte(ByteReg::AH);
        self.flags.carry_val = (ah & 0x01) as u32;
        self.flags.parity_val = if ah & 0x04 != 0 { 0 } else { 1 };
        self.flags.aux_val = (ah & 0x10) as u32;
        self.flags.zero_val = if ah & 0x40 != 0 { 0 } else { 1 };
        self.flags.sign_val = if ah & 0x80 != 0 { -1 } else { 0 };
        self.clk(Self::timing(3, 2));
    }

    fn lahf(&mut self) {
        let flags_val = self.flags.compress() as u8;
        self.regs.set_byte(ByteReg::AH, flags_val);
        self.clk(Self::timing(2, 3));
    }

    fn mov_al_moffs(&mut self, bus: &mut impl common::Bus) -> Step {
        let seg = self.default_seg(SegReg32::DS);
        let offset = if self.address_size_override {
            self.fetchdword(bus)
        } else {
            self.fetchword(bus) as u32
        };
        let val = self.read_byte_seg(bus, seg, offset)?;
        self.regs.set_byte(ByteReg::AL, val);
        self.clk(Self::timing(4, 1));
        Ok(())
    }

    fn mov_aw_moffs(&mut self, bus: &mut impl common::Bus) -> Step {
        let seg = self.default_seg(SegReg32::DS);
        let offset = if self.address_size_override {
            self.fetchdword(bus)
        } else {
            self.fetchword(bus) as u32
        };
        self.ea_seg = seg;
        self.eo = offset as u16;
        self.eo32 = offset;
        self.ea = self.seg_base(seg).wrapping_add(offset);
        if self.operand_size_override {
            let val = self.seg_read_dword(bus)?;
            self.regs.set_dword(DwordReg::EAX, val);
            let penalty = if self.ea & 3 != 0 {
                Self::timing(4, 3)
            } else {
                0
            };
            self.clk(Self::timing(4, 1) + penalty);
        } else {
            let val = self.seg_read_word(bus)?;
            self.regs.set_word(WordReg::AX, val);
            let penalty = if self.ea & 1 != 0 {
                Self::timing(4, 3)
            } else {
                0
            };
            self.clk(Self::timing(4, 1) + penalty);
        }
        Ok(())
    }

    fn mov_moffs_al(&mut self, bus: &mut impl common::Bus) -> Step {
        let seg = self.default_seg(SegReg32::DS);
        let al = self.regs.byte(ByteReg::AL);
        let offset = if self.address_size_override {
            self.fetchdword(bus)
        } else {
            self.fetchword(bus) as u32
        };
        self.write_byte_seg(bus, seg, offset, al)?;
        self.clk(Self::timing(2, 1));
        Ok(())
    }

    fn mov_moffs_aw(&mut self, bus: &mut impl common::Bus) -> Step {
        let seg = self.default_seg(SegReg32::DS);
        let offset = if self.address_size_override {
            self.fetchdword(bus)
        } else {
            self.fetchword(bus) as u32
        };
        self.ea_seg = seg;
        self.eo = offset as u16;
        self.eo32 = offset;
        self.ea = self.seg_base(seg).wrapping_add(offset);
        if self.operand_size_override {
            self.seg_write_dword(bus, self.regs.dword(DwordReg::EAX))?;
            let penalty = if self.ea & 3 != 0 {
                Self::timing(4, 3)
            } else {
                0
            };
            self.clk(Self::timing(2, 1) + penalty);
        } else {
            self.seg_write_word(bus, self.regs.word(WordReg::AX))?;
            let penalty = if self.ea & 1 != 0 {
                Self::timing(4, 3)
            } else {
                0
            };
            self.clk(Self::timing(2, 1) + penalty);
        }
        Ok(())
    }

    fn mov_byte_reg_imm(&mut self, reg: ByteReg, bus: &mut impl common::Bus) {
        let val = self.fetch(bus);
        self.regs.set_byte(reg, val);
        self.clk(Self::timing(2, 1));
    }

    fn mov_word_reg_imm(&mut self, reg: WordReg, bus: &mut impl common::Bus) {
        if self.operand_size_override {
            let val = self.fetchdword(bus);
            self.regs.set_dword(DwordReg::from_index(reg as u8), val);
        } else {
            let val = self.fetchword(bus);
            self.regs.set_word(reg, val);
        }
        self.clk(Self::timing(2, 1));
    }

    fn mov_rm_imm8(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        if modrm >= 0xC0 {
            let val = self.fetch(bus);
            let reg = self.rm_byte(modrm);
            self.regs.set_byte(reg, val);
        } else {
            self.calc_ea(modrm, bus);
            let val = self.fetch(bus);
            let addr = self.translate_linear(self.ea, true, bus)?;
            bus.write_byte(addr, val);
        }
        self.clk_modrm(modrm, Self::timing(2, 1), Self::timing(2, 1));
        Ok(())
    }

    fn mov_rm_imm16(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        if self.operand_size_override {
            if modrm >= 0xC0 {
                let val = self.fetchdword(bus);
                let reg = self.rm_dword(modrm);
                self.regs.set_dword(reg, val);
            } else {
                self.calc_ea(modrm, bus);
                let val = self.fetchdword(bus);
                self.seg_write_dword(bus, val)?;
            }
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(2, 1), 2);
        } else {
            if modrm >= 0xC0 {
                let val = self.fetchword(bus);
                let reg = self.rm_word(modrm);
                self.regs.set_word(reg, val);
            } else {
                self.calc_ea(modrm, bus);
                let val = self.fetchword(bus);
                self.seg_write_word(bus, val)?;
            }
            self.clk_modrm_word(modrm, Self::timing(2, 1), Self::timing(2, 1), 1);
        }
        Ok(())
    }

    fn les(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        if modrm >= 0xC0 {
            self.invalid(bus)?;
            return Ok(());
        }
        self.calc_ea(modrm, bus);
        // Snapshot CS:EIP so a fault inside the pointer fetch aborts the
        // instruction without committing the segment-register or
        // destination-register update. raise_fault restores fault_pending
        // to its pre-instruction value after a clean dispatch, so it does
        // not survive as a signal here.
        let initial_cs = self.sregs[SegReg32::CS as usize];
        let initial_ip = self.ip;
        let initial_ip_upper = self.ip_upper;
        if self.operand_size_override {
            let offset = self.seg_read_dword(bus)?;
            let segment = self.seg_read_word_at(bus, 4)?;
            if self.sregs[SegReg32::CS as usize] != initial_cs
                || self.ip != initial_ip
                || self.ip_upper != initial_ip_upper
            {
                return Ok(());
            }
            self.load_segment(SegReg32::ES, segment, bus)?;
            let reg = self.reg_dword(modrm);
            self.regs.set_dword(reg, offset);
            self.clk(Self::timing(7, 6));
            return Ok(());
        }
        let offset = self.seg_read_word(bus)?;
        let segment = self.seg_read_word_at(bus, 2)?;
        if self.sregs[SegReg32::CS as usize] != initial_cs
            || self.ip != initial_ip
            || self.ip_upper != initial_ip_upper
        {
            return Ok(());
        }
        self.load_segment(SegReg32::ES, segment, bus)?;
        let reg = self.reg_word(modrm);
        self.regs.set_word(reg, offset);
        self.clk(Self::timing(7, 6));
        Ok(())
    }

    fn lds(&mut self, bus: &mut impl common::Bus) -> Step {
        let modrm = self.fetch(bus);
        if modrm >= 0xC0 {
            self.invalid(bus)?;
            return Ok(());
        }
        self.calc_ea(modrm, bus);
        let initial_cs = self.sregs[SegReg32::CS as usize];
        let initial_ip = self.ip;
        let initial_ip_upper = self.ip_upper;
        if self.operand_size_override {
            let offset = self.seg_read_dword(bus)?;
            let segment = self.seg_read_word_at(bus, 4)?;
            if self.sregs[SegReg32::CS as usize] != initial_cs
                || self.ip != initial_ip
                || self.ip_upper != initial_ip_upper
            {
                return Ok(());
            }
            self.load_segment(SegReg32::DS, segment, bus)?;
            let reg = self.reg_dword(modrm);
            self.regs.set_dword(reg, offset);
            self.clk(Self::timing(7, 6));
            return Ok(());
        }
        let offset = self.seg_read_word(bus)?;
        let segment = self.seg_read_word_at(bus, 2)?;
        if self.sregs[SegReg32::CS as usize] != initial_cs
            || self.ip != initial_ip
            || self.ip_upper != initial_ip_upper
        {
            return Ok(());
        }
        self.load_segment(SegReg32::DS, segment, bus)?;
        let reg = self.reg_word(modrm);
        self.regs.set_word(reg, offset);
        self.clk(Self::timing(7, 6));
        Ok(())
    }

    fn enter(&mut self, bus: &mut impl common::Bus) -> Step {
        let alloc = self.fetchword(bus);
        let level = self.fetch(bus) & 0x1F;
        let sp_pen = self.sp_penalty();
        let operand_size: u32 = if self.operand_size_override { 4 } else { 2 };
        let push_count: u32 = if level == 0 { 1 } else { level as u32 + 1 };
        let use_esp = self.use_esp();
        let esp_full = self.regs.dword(DwordReg::ESP);
        let sp_in = if use_esp {
            esp_full
        } else {
            esp_full as u16 as u32
        };
        let bp_in: u32 = if self.operand_size_override {
            self.regs.dword(DwordReg::EBP)
        } else {
            self.regs.word(WordReg::BP) as u32
        };
        let stack_offset = |delta: u32| -> u32 {
            if use_esp {
                sp_in.wrapping_sub(delta)
            } else {
                (sp_in as u16).wrapping_sub(delta as u16) as u32
            }
        };

        // FrameTemp follows the SP-commit wrap rules but also preserves
        // ESP's upper 16 bits when the stack is 16-bit (B=0).
        let frame_ptr_value = if use_esp {
            sp_in.wrapping_sub(operand_size)
        } else {
            (esp_full & 0xFFFF_0000) | ((sp_in as u16).wrapping_sub(operand_size as u16) as u32)
        };

        // Pre-compute every offset the instruction will touch. `level` is
        // masked to [0, 31] so the buffers are bounded.
        let mut walk_offsets: [u32; 31] = [0; 31];
        let chain_count: u32 = if level >= 2 { level as u32 - 1 } else { 0 };
        if chain_count > 0 {
            let wrap_walk_16 = !self.operand_size_override || !use_esp;
            let mut walk = bp_in;
            for entry in walk_offsets.iter_mut().take(chain_count as usize) {
                walk = walk.wrapping_sub(operand_size);
                *entry = if wrap_walk_16 {
                    walk as u16 as u32
                } else {
                    walk
                };
            }
        }
        let mut push_offsets: [u32; 32] = [0; 32];
        for k in 0..push_count {
            push_offsets[k as usize] = stack_offset(operand_size * (k + 1));
        }
        let final_sp = stack_offset(operand_size * push_count + alloc as u32);

        // Probe phase. Chain reads (read access); push writes (write); and
        // the final-SP byte (write, for the alloc-area #PF pre-check).
        let ss_base = self.seg_base(SegReg32::SS);
        for k in 0..chain_count {
            let off = walk_offsets[k as usize];
            self.check_segment_access(SegReg32::SS, off, operand_size, false, bus)?;
            let l0 = ss_base.wrapping_add(off);
            for b in 0..operand_size {
                self.translate_linear(l0.wrapping_add(b), false, bus)?;
            }
        }
        for k in 0..push_count {
            let off = push_offsets[k as usize];
            self.check_segment_access(SegReg32::SS, off, operand_size, true, bus)?;
            let l0 = ss_base.wrapping_add(off);
            for b in 0..operand_size {
                self.translate_linear(l0.wrapping_add(b), true, bus)?;
            }
        }
        self.check_segment_access(SegReg32::SS, final_sp, operand_size, true, bus)?;
        self.translate_linear(ss_base.wrapping_add(final_sp), true, bus)?;

        // Sequential operation against memory (TLB-hot, cannot fault).
        if self.operand_size_override {
            self.write_dword_seg(bus, SegReg32::SS, push_offsets[0], bp_in)?;
        } else {
            self.write_word_seg(bus, SegReg32::SS, push_offsets[0], bp_in as u16)?;
        }
        for k in 0..chain_count {
            let read_off = walk_offsets[k as usize];
            let push_off = push_offsets[(k + 1) as usize];
            if self.operand_size_override {
                let val = self.read_dword_seg(bus, SegReg32::SS, read_off)?;
                self.write_dword_seg(bus, SegReg32::SS, push_off, val)?;
            } else {
                let val = self.read_word_seg(bus, SegReg32::SS, read_off)?;
                self.write_word_seg(bus, SegReg32::SS, push_off, val)?;
            }
        }
        if level > 0 {
            let frame_off = push_offsets[(push_count - 1) as usize];
            if self.operand_size_override {
                self.write_dword_seg(bus, SegReg32::SS, frame_off, frame_ptr_value)?;
            } else {
                self.write_word_seg(bus, SegReg32::SS, frame_off, frame_ptr_value as u16)?;
            }
        }

        self.commit_sp(final_sp);
        if self.operand_size_override {
            self.regs.set_dword(DwordReg::EBP, frame_ptr_value);
        } else {
            self.regs.set_word(WordReg::BP, frame_ptr_value as u16);
        }

        if level == 0 {
            self.clk(Self::timing(10, 14) + sp_pen);
        } else if level == 1 {
            self.clk(Self::timing(12, 17) + sp_pen);
        } else {
            let l = level as i32;
            self.clk(Self::timing(15 + 4 * (l - 1), 17 + 3 * l) + sp_pen);
        }
        Ok(())
    }

    fn leave(&mut self, bus: &mut impl common::Bus) -> Step {
        let bp_offset = if self.use_esp() {
            self.regs.dword(DwordReg::EBP)
        } else {
            self.regs.word(WordReg::BP) as u32
        };
        let penalty = self.sp_penalty();
        if self.operand_size_override {
            let val = self.read_dword_seg(bus, SegReg32::SS, bp_offset)?;
            self.commit_sp(bp_offset.wrapping_add(4));
            self.regs.set_dword(DwordReg::EBP, val);
        } else {
            let val = self.read_word_seg(bus, SegReg32::SS, bp_offset)?;
            self.commit_sp(bp_offset.wrapping_add(2));
            self.regs.set_word(WordReg::BP, val);
        }
        self.clk(Self::timing(4, 5) + penalty);
        Ok(())
    }

    fn int3(&mut self, bus: &mut impl common::Bus) -> Step {
        let penalty = self.sp_penalty();
        self.raise_software_interrupt(3, false, bus)?;
        self.clk(Self::timing(33, 26) + penalty);
        Ok(())
    }

    fn int_imm(&mut self, bus: &mut impl common::Bus) -> Step {
        let penalty = self.sp_penalty();
        let vector = self.fetch(bus);
        self.raise_software_interrupt(vector, true, bus)?;
        self.clk(Self::timing(37, 30) + penalty);
        Ok(())
    }

    fn into(&mut self, bus: &mut impl common::Bus) -> Step {
        if self.flags.of() {
            let penalty = self.sp_penalty();
            self.raise_software_interrupt(4, false, bus)?;
            self.clk(Self::timing(35, 28) + penalty);
        } else {
            self.clk(Self::timing(3, 3));
        }
        Ok(())
    }

    fn iret(&mut self, bus: &mut impl common::Bus) -> Step {
        self.preserve_resume_flag = true;
        let penalty = self.sp_penalty();

        if !self.is_protected_mode() {
            let use_esp = self.use_esp();
            let sp = if use_esp {
                self.regs.dword(DwordReg::ESP)
            } else {
                self.regs.word(WordReg::SP) as u32
            };
            let stack_offset = |delta: u32| -> u32 {
                if use_esp {
                    sp.wrapping_add(delta)
                } else {
                    (sp as u16).wrapping_add(delta as u16) as u32
                }
            };
            let ss_base = self.seg_base(SegReg32::SS);
            if self.operand_size_override {
                let eip = self.read_dword_linear(bus, ss_base.wrapping_add(stack_offset(0)))?;
                let cs_dword =
                    self.read_dword_linear(bus, ss_base.wrapping_add(stack_offset(4)))?;
                let eflags = self.read_dword_linear(bus, ss_base.wrapping_add(stack_offset(8)))?;
                self.load_segment(SegReg32::CS, cs_dword as u16, bus)?;
                self.commit_sp(stack_offset(12));
                self.ip = eip as u16;
                self.ip_upper = eip & 0xFFFF_0000;
                self.flags.load_flags(eflags as u16, 0, false);
                // Real-mode IRETD loads RF (and AC on the 486); VM is unchanged.
                self.eflags_upper =
                    eflags & (Self::eflags_upper_writable() & !EFLAGS_VIRTUAL_8086_FLAG);
            } else {
                let ip = self.read_word_linear(bus, ss_base.wrapping_add(stack_offset(0)))?;
                let cs = self.read_word_linear(bus, ss_base.wrapping_add(stack_offset(2)))?;
                let flags_val =
                    self.read_word_linear(bus, ss_base.wrapping_add(stack_offset(4)))?;
                self.load_segment(SegReg32::CS, cs, bus)?;
                self.commit_sp(stack_offset(6));
                self.ip = ip;
                self.ip_upper = 0;
                self.flags.load_flags(flags_val, 0, false);
            }
            self.clk(Self::timing(22, 15) + penalty);
            return Ok(());
        }

        if self.is_virtual_mode() {
            if self.flags.iopl < 3 {
                self.raise_fault_with_code(13, 0, bus)?;
                return Ok(());
            }

            let use_esp = self.use_esp();
            let sp = if use_esp {
                self.regs.dword(DwordReg::ESP)
            } else {
                self.regs.word(WordReg::SP) as u32
            };
            let stack_offset = |delta: u32| -> u32 {
                if use_esp {
                    sp.wrapping_add(delta)
                } else {
                    (sp as u16).wrapping_add(delta as u16) as u32
                }
            };
            let ss_base = self.seg_base(SegReg32::SS);
            if self.operand_size_override {
                let new_eip = self.read_dword_linear(bus, ss_base.wrapping_add(stack_offset(0)))?;
                let new_cs_dword =
                    self.read_dword_linear(bus, ss_base.wrapping_add(stack_offset(4)))?;
                let new_eflags =
                    self.read_dword_linear(bus, ss_base.wrapping_add(stack_offset(8)))?;
                let new_cs = new_cs_dword as u16;

                self.sregs[SegReg32::CS as usize] = new_cs;
                self.set_real_segment_cache(SegReg32::CS, new_cs);
                self.commit_sp(stack_offset(12));
                // EIP in V86 mode is a 16-bit value; the upper 16 bits of the
                // popped EIP are discarded so subsequent fetches do not exceed
                // the implicit 64KB CS limit.
                self.ip = new_eip as u16;
                self.ip_upper = 0;
                self.flags.load_flags(new_eflags as u16, 3, true);
                self.eflags_upper =
                    (new_eflags & Self::eflags_upper_writable()) | EFLAGS_VIRTUAL_8086_FLAG;
            } else {
                let new_ip = self.read_word_linear(bus, ss_base.wrapping_add(stack_offset(0)))?;
                let new_cs = self.read_word_linear(bus, ss_base.wrapping_add(stack_offset(2)))?;
                let new_flags =
                    self.read_word_linear(bus, ss_base.wrapping_add(stack_offset(4)))?;

                self.sregs[SegReg32::CS as usize] = new_cs;
                self.set_real_segment_cache(SegReg32::CS, new_cs);
                self.commit_sp(stack_offset(6));
                self.ip = new_ip;
                self.ip_upper = 0;
                self.flags.load_flags(new_flags, 3, true);
                self.eflags_upper = EFLAGS_VIRTUAL_8086_FLAG;
            }

            self.clk(Self::timing(22, 15) + penalty);
            return Ok(());
        }

        // Protected mode IRET.
        if self.flags.nt {
            // Task return via back-link in current TSS.
            let backlink = self.read_word_linear(bus, self.tr_base)?;
            self.switch_task(backlink, super::TaskType::Iret, bus)?;
            let flags_val = self.flags.compress();
            let cpl = self.cpl();
            self.flags.load_flags(flags_val, cpl, true);
            self.clk(Self::timing(22, 15) + penalty);
            return Ok(());
        }

        let old_cpl = self.cpl();

        let sp = if self.use_esp() {
            self.regs.dword(DwordReg::ESP)
        } else {
            self.regs.word(WordReg::SP) as u32
        };
        let ss_base = self.seg_base(SegReg32::SS);

        if self.operand_size_override {
            let new_eip = self.read_dword_linear(bus, ss_base.wrapping_add(sp))?;
            let new_cs_dword =
                self.read_dword_linear(bus, ss_base.wrapping_add(sp.wrapping_add(4)))?;
            let new_cs = new_cs_dword as u16;
            let new_eflags =
                self.read_dword_linear(bus, ss_base.wrapping_add(sp.wrapping_add(8)))?;

            // IRET from CPL0 to virtual-8086 mode.
            // Stack frame: EIP, CS, EFLAGS, ESP, SS, ES, DS, FS, GS.
            if old_cpl == 0 && (new_eflags & 0x0002_0000) != 0 {
                let new_esp =
                    self.read_dword_linear(bus, ss_base.wrapping_add(sp.wrapping_add(12)))?;
                let new_ss_dword =
                    self.read_dword_linear(bus, ss_base.wrapping_add(sp.wrapping_add(16)))?;
                let new_ss = new_ss_dword as u16;
                let new_es_dword =
                    self.read_dword_linear(bus, ss_base.wrapping_add(sp.wrapping_add(20)))?;
                let new_es = new_es_dword as u16;
                let new_ds_dword =
                    self.read_dword_linear(bus, ss_base.wrapping_add(sp.wrapping_add(24)))?;
                let new_ds = new_ds_dword as u16;
                let new_fs_dword =
                    self.read_dword_linear(bus, ss_base.wrapping_add(sp.wrapping_add(28)))?;
                let new_fs = new_fs_dword as u16;
                let new_gs_dword =
                    self.read_dword_linear(bus, ss_base.wrapping_add(sp.wrapping_add(32)))?;
                let new_gs = new_gs_dword as u16;

                self.flags.load_flags(new_eflags as u16, old_cpl, true);
                self.eflags_upper =
                    (new_eflags & Self::eflags_upper_writable()) | EFLAGS_VIRTUAL_8086_FLAG;

                self.sregs[SegReg32::CS as usize] = new_cs;
                self.set_real_segment_cache(SegReg32::CS, new_cs);
                // In V86 mode, EIP is a 16-bit value; the upper 16 bits of
                // the popped 32-bit EIP are discarded so subsequent fetches
                // do not exceed the implicit 64KB segment limit.
                self.ip = new_eip as u16;
                self.ip_upper = 0;

                self.sregs[SegReg32::SS as usize] = new_ss;
                self.set_real_segment_cache(SegReg32::SS, new_ss);
                self.regs.set_dword(DwordReg::ESP, new_esp);

                self.sregs[SegReg32::ES as usize] = new_es;
                self.set_real_segment_cache(SegReg32::ES, new_es);
                self.sregs[SegReg32::DS as usize] = new_ds;
                self.set_real_segment_cache(SegReg32::DS, new_ds);
                self.sregs[SegReg32::FS as usize] = new_fs;
                self.set_real_segment_cache(SegReg32::FS, new_fs);
                self.sregs[SegReg32::GS as usize] = new_gs;
                self.set_real_segment_cache(SegReg32::GS, new_gs);

                self.clk(Self::timing(60, 15) + penalty);
                return Ok(());
            }

            let new_rpl = new_cs & 3;

            if new_rpl > old_cpl {
                let new_esp =
                    self.read_dword_linear(bus, ss_base.wrapping_add(sp.wrapping_add(12)))?;
                let new_ss_dword =
                    self.read_dword_linear(bus, ss_base.wrapping_add(sp.wrapping_add(16)))?;
                let new_ss = new_ss_dword as u16;

                let ss_validation = match self.validate_ss_for_iret_return(new_ss, new_rpl, bus)? {
                    Err((vector, error_code)) => {
                        return self.raise_fault_with_code(vector, error_code, bus);
                    }
                    Ok(validation) => validation,
                };

                let cs_validation = match self.validate_cs_for_return(new_cs, new_eip, bus)? {
                    Err((vector, error_code)) => {
                        return self.raise_fault_with_code(vector, error_code, bus);
                    }
                    Ok(validation) => validation,
                };

                let new_cpl = new_rpl;
                let ds_decision = self.check_data_segment_at_cpl(SegReg32::DS, new_cpl, bus)?;
                let es_decision = self.check_data_segment_at_cpl(SegReg32::ES, new_cpl, bus)?;
                let fs_decision = self.check_data_segment_at_cpl(SegReg32::FS, new_cpl, bus)?;
                let gs_decision = self.check_data_segment_at_cpl(SegReg32::GS, new_cpl, bus)?;

                self.flags.load_flags(new_eflags as u16, old_cpl, true);
                if old_cpl == 0 {
                    self.eflags_upper = new_eflags & Self::eflags_upper_writable();
                } else {
                    self.eflags_upper =
                        new_eflags & (Self::eflags_upper_writable() & !EFLAGS_VIRTUAL_8086_FLAG);
                }

                self.set_accessed_bit(cs_validation.adjusted_selector, bus)?;
                self.set_loaded_segment_cache(
                    SegReg32::CS,
                    cs_validation.adjusted_selector,
                    cs_validation.descriptor,
                );
                self.ip = new_eip as u16;
                self.ip_upper = new_eip & 0xFFFF_0000;

                self.set_accessed_bit(new_ss, bus)?;
                self.set_loaded_segment_cache(SegReg32::SS, new_ss, ss_validation.descriptor);
                if self.use_esp() {
                    self.regs.set_dword(DwordReg::ESP, new_esp);
                } else {
                    self.regs.set_word(WordReg::SP, new_esp as u16);
                }

                self.apply_data_segment_decision(SegReg32::DS, ds_decision);
                self.apply_data_segment_decision(SegReg32::ES, es_decision);
                self.apply_data_segment_decision(SegReg32::FS, fs_decision);
                self.apply_data_segment_decision(SegReg32::GS, gs_decision);
            } else {
                let cs_validation = match self.validate_cs_for_return(new_cs, new_eip, bus)? {
                    Err((vector, error_code)) => {
                        return self.raise_fault_with_code(vector, error_code, bus);
                    }
                    Ok(validation) => validation,
                };

                self.set_accessed_bit(cs_validation.adjusted_selector, bus)?;
                self.set_loaded_segment_cache(
                    SegReg32::CS,
                    cs_validation.adjusted_selector,
                    cs_validation.descriptor,
                );
                if self.use_esp() {
                    self.regs.set_dword(DwordReg::ESP, sp.wrapping_add(12));
                } else {
                    self.regs.set_word(WordReg::SP, sp.wrapping_add(12) as u16);
                }
                self.ip = new_eip as u16;
                self.ip_upper = new_eip & 0xFFFF_0000;
                self.flags.load_flags(new_eflags as u16, old_cpl, true);
                if old_cpl == 0 {
                    self.eflags_upper = new_eflags & Self::eflags_upper_writable();
                } else {
                    self.eflags_upper =
                        new_eflags & (Self::eflags_upper_writable() & !EFLAGS_VIRTUAL_8086_FLAG);
                }
            }
        } else {
            let new_ip = self.read_word_linear(bus, ss_base.wrapping_add(sp))?;
            let new_cs = self.read_word_linear(bus, ss_base.wrapping_add(sp.wrapping_add(2)))?;
            let new_flags = self.read_word_linear(bus, ss_base.wrapping_add(sp.wrapping_add(4)))?;

            let new_rpl = new_cs & 3;

            if new_rpl > old_cpl {
                let new_sp =
                    self.read_word_linear(bus, ss_base.wrapping_add(sp.wrapping_add(6)))?;
                let new_ss =
                    self.read_word_linear(bus, ss_base.wrapping_add(sp.wrapping_add(8)))?;

                let ss_validation = match self.validate_ss_for_iret_return(new_ss, new_rpl, bus)? {
                    Err((vector, error_code)) => {
                        return self.raise_fault_with_code(vector, error_code, bus);
                    }
                    Ok(validation) => validation,
                };

                let cs_validation = match self.validate_cs_for_return(new_cs, new_ip as u32, bus)? {
                    Err((vector, error_code)) => {
                        return self.raise_fault_with_code(vector, error_code, bus);
                    }
                    Ok(validation) => validation,
                };

                let new_cpl = new_rpl;
                let ds_decision = self.check_data_segment_at_cpl(SegReg32::DS, new_cpl, bus)?;
                let es_decision = self.check_data_segment_at_cpl(SegReg32::ES, new_cpl, bus)?;
                let fs_decision = self.check_data_segment_at_cpl(SegReg32::FS, new_cpl, bus)?;
                let gs_decision = self.check_data_segment_at_cpl(SegReg32::GS, new_cpl, bus)?;

                self.flags.load_flags(new_flags, old_cpl, true);

                self.set_accessed_bit(cs_validation.adjusted_selector, bus)?;
                self.set_loaded_segment_cache(
                    SegReg32::CS,
                    cs_validation.adjusted_selector,
                    cs_validation.descriptor,
                );
                self.ip = new_ip;
                self.ip_upper = 0;

                self.set_accessed_bit(new_ss, bus)?;
                self.set_loaded_segment_cache(SegReg32::SS, new_ss, ss_validation.descriptor);
                if self.use_esp() {
                    self.regs.set_dword(DwordReg::ESP, new_sp as u32);
                } else {
                    self.regs.set_word(WordReg::SP, new_sp);
                }

                self.apply_data_segment_decision(SegReg32::DS, ds_decision);
                self.apply_data_segment_decision(SegReg32::ES, es_decision);
                self.apply_data_segment_decision(SegReg32::FS, fs_decision);
                self.apply_data_segment_decision(SegReg32::GS, gs_decision);
            } else {
                let cs_validation = match self.validate_cs_for_return(new_cs, new_ip as u32, bus)? {
                    Err((vector, error_code)) => {
                        return self.raise_fault_with_code(vector, error_code, bus);
                    }
                    Ok(validation) => validation,
                };

                self.set_accessed_bit(cs_validation.adjusted_selector, bus)?;
                self.set_loaded_segment_cache(
                    SegReg32::CS,
                    cs_validation.adjusted_selector,
                    cs_validation.descriptor,
                );
                if self.use_esp() {
                    self.regs.set_dword(DwordReg::ESP, sp.wrapping_add(6));
                } else {
                    self.regs.set_word(WordReg::SP, sp.wrapping_add(6) as u16);
                }
                self.ip = new_ip;
                self.ip_upper = 0;
                self.flags.load_flags(new_flags, old_cpl, true);
            }
        }

        self.clk(Self::timing(22, 15) + penalty);
        Ok(())
    }

    fn loopne(&mut self, bus: &mut impl common::Bus) {
        let disp = self.fetch(bus) as i8;
        let count = if self.address_size_override {
            let value = self.regs.dword(DwordReg::ECX).wrapping_sub(1);
            self.regs.set_dword(DwordReg::ECX, value);
            value
        } else {
            let value = self.regs.word(WordReg::CX).wrapping_sub(1);
            self.regs.set_word(WordReg::CX, value);
            value as u32
        };
        if count != 0 && !self.flags.zf() {
            self.apply_branch_disp8(disp);
            match CPU_MODEL {
                CPU_MODEL_386 => {
                    let m = self.next_instruction_length_approx(bus);
                    self.clk(11 + m);
                }
                CPU_MODEL_486 => self.clk(9),
                _ => {
                    unreachable!("Unhandled CPU_MODEL")
                }
            }
        } else {
            self.clk(Self::timing(5, 6));
        }
    }

    fn loope(&mut self, bus: &mut impl common::Bus) {
        let disp = self.fetch(bus) as i8;
        let count = if self.address_size_override {
            let value = self.regs.dword(DwordReg::ECX).wrapping_sub(1);
            self.regs.set_dword(DwordReg::ECX, value);
            value
        } else {
            let value = self.regs.word(WordReg::CX).wrapping_sub(1);
            self.regs.set_word(WordReg::CX, value);
            value as u32
        };
        if count != 0 && self.flags.zf() {
            self.apply_branch_disp8(disp);
            match CPU_MODEL {
                CPU_MODEL_386 => {
                    let m = self.next_instruction_length_approx(bus);
                    self.clk(11 + m);
                }
                CPU_MODEL_486 => self.clk(9),
                _ => {
                    unreachable!("Unhandled CPU_MODEL")
                }
            }
        } else {
            self.clk(Self::timing(5, 6));
        }
    }

    fn loop_(&mut self, bus: &mut impl common::Bus) {
        let disp = self.fetch(bus) as i8;
        let count = if self.address_size_override {
            let value = self.regs.dword(DwordReg::ECX).wrapping_sub(1);
            self.regs.set_dword(DwordReg::ECX, value);
            value
        } else {
            let value = self.regs.word(WordReg::CX).wrapping_sub(1);
            self.regs.set_word(WordReg::CX, value);
            value as u32
        };
        if count != 0 {
            self.apply_branch_disp8(disp);
            match CPU_MODEL {
                CPU_MODEL_386 => {
                    let m = self.next_instruction_length_approx(bus);
                    self.clk(11 + m);
                }
                CPU_MODEL_486 => self.clk(7),
                _ => {
                    unreachable!("Unhandled CPU_MODEL")
                }
            }
        } else {
            self.clk(Self::timing(5, 6));
        }
    }

    fn jcxz(&mut self, bus: &mut impl common::Bus) {
        let disp = self.fetch(bus) as i8;
        let is_zero = if self.address_size_override {
            self.regs.dword(DwordReg::ECX) == 0
        } else {
            self.regs.word(WordReg::CX) == 0
        };
        if is_zero {
            self.apply_branch_disp8(disp);
            match CPU_MODEL {
                CPU_MODEL_386 => {
                    let m = self.next_instruction_length_approx(bus);
                    self.clk(9 + m);
                }
                CPU_MODEL_486 => self.clk(8),
                _ => {
                    unreachable!("Unhandled CPU_MODEL")
                }
            }
        } else {
            self.clk(Self::timing(5, 5));
        }
    }

    fn in_al_imm(&mut self, bus: &mut impl common::Bus) -> Step {
        let port = self.fetch(bus) as u16;
        if self.check_io_privilege(port, 1, bus).is_err() {
            return Err(Fault);
        }
        let val = bus.io_read_byte(port);
        self.regs.set_byte(ByteReg::AL, val);
        self.clk(Self::timing(12, 14));
        Ok(())
    }

    fn in_aw_imm(&mut self, bus: &mut impl common::Bus) -> Step {
        let port = self.fetch(bus) as u16;
        let size = if self.operand_size_override { 4 } else { 2 };
        if self.check_io_privilege(port, size, bus).is_err() {
            return Err(Fault);
        }
        if self.operand_size_override {
            let low = bus.io_read_word(port) as u32;
            let high = bus.io_read_word(port.wrapping_add(2)) as u32;
            self.regs.set_dword(DwordReg::EAX, low | (high << 16));
        } else {
            let val = bus.io_read_word(port);
            self.regs.set_word(WordReg::AX, val);
        }
        self.clk(Self::timing(12, 14));
        Ok(())
    }

    fn out_imm_al(&mut self, bus: &mut impl common::Bus) -> Step {
        let port = self.fetch(bus) as u16;
        if self.check_io_privilege(port, 1, bus).is_err() {
            return Err(Fault);
        }
        let val = self.regs.byte(ByteReg::AL);
        bus.io_write_byte(port, val);
        self.clk(Self::timing(10, 16));
        Ok(())
    }

    fn out_imm_aw(&mut self, bus: &mut impl common::Bus) -> Step {
        let port = self.fetch(bus) as u16;
        let size = if self.operand_size_override { 4 } else { 2 };
        if self.check_io_privilege(port, size, bus).is_err() {
            return Err(Fault);
        }
        if self.operand_size_override {
            let val = self.regs.dword(DwordReg::EAX);
            bus.io_write_word(port, val as u16);
            bus.io_write_word(port.wrapping_add(2), (val >> 16) as u16);
        } else {
            let val = self.regs.word(WordReg::AX);
            bus.io_write_word(port, val);
        }
        self.clk(Self::timing(10, 16));
        Ok(())
    }

    fn in_al_dw(&mut self, bus: &mut impl common::Bus) -> Step {
        let port = self.regs.word(WordReg::DX);
        if self.check_io_privilege(port, 1, bus).is_err() {
            return Err(Fault);
        }
        let val = bus.io_read_byte(port);
        self.regs.set_byte(ByteReg::AL, val);
        self.clk(Self::timing(13, 14));
        Ok(())
    }

    fn in_aw_dw(&mut self, bus: &mut impl common::Bus) -> Step {
        let port = self.regs.word(WordReg::DX);
        let size = if self.operand_size_override { 4 } else { 2 };
        if self.check_io_privilege(port, size, bus).is_err() {
            return Err(Fault);
        }
        if self.operand_size_override {
            let low = bus.io_read_word(port) as u32;
            let high = bus.io_read_word(port.wrapping_add(2)) as u32;
            self.regs.set_dword(DwordReg::EAX, low | (high << 16));
        } else {
            let val = bus.io_read_word(port);
            self.regs.set_word(WordReg::AX, val);
        }
        self.clk(Self::timing(13, 14));
        Ok(())
    }

    fn out_dw_al(&mut self, bus: &mut impl common::Bus) -> Step {
        let port = self.regs.word(WordReg::DX);
        if self.check_io_privilege(port, 1, bus).is_err() {
            return Err(Fault);
        }
        let val = self.regs.byte(ByteReg::AL);
        bus.io_write_byte(port, val);
        self.clk(Self::timing(11, 16));
        Ok(())
    }

    fn out_dw_aw(&mut self, bus: &mut impl common::Bus) -> Step {
        let port = self.regs.word(WordReg::DX);
        let size = if self.operand_size_override { 4 } else { 2 };
        if self.check_io_privilege(port, size, bus).is_err() {
            return Err(Fault);
        }
        if self.operand_size_override {
            let val = self.regs.dword(DwordReg::EAX);
            bus.io_write_word(port, val as u16);
            bus.io_write_word(port.wrapping_add(2), (val >> 16) as u16);
        } else {
            let val = self.regs.word(WordReg::AX);
            bus.io_write_word(port, val);
        }
        self.clk(Self::timing(11, 16));
        Ok(())
    }

    fn xlat(&mut self, bus: &mut impl common::Bus) -> Step {
        let seg = self.default_seg(SegReg32::DS);
        let offset = if self.address_size_override {
            self.regs
                .dword(DwordReg::EBX)
                .wrapping_add(self.regs.byte(ByteReg::AL) as u32)
        } else {
            self.regs
                .word(WordReg::BX)
                .wrapping_add(self.regs.byte(ByteReg::AL) as u16) as u32
        };
        let val = self.read_byte_seg(bus, seg, offset)?;
        self.regs.set_byte(ByteReg::AL, val);
        self.clk(Self::timing(5, 4));
        Ok(())
    }

    fn daa(&mut self, _bus: &mut impl common::Bus) {
        let old_al = self.regs.byte(ByteReg::AL);
        let old_cf = self.flags.cf();
        let old_af = self.flags.af();
        let mut al = old_al;
        let mut carry = old_cf;

        if (old_al & 0x0F) > 9 || old_af {
            al = al.wrapping_add(6);
            carry = old_cf || al < old_al;
            self.flags.aux_val = 1;
        } else {
            self.flags.aux_val = 0;
        }
        if old_al > 0x99 || old_cf {
            al = al.wrapping_add(0x60);
            carry = true;
        }
        self.flags.carry_val = u32::from(carry);
        self.regs.set_byte(ByteReg::AL, al);
        self.flags.set_szpf_byte(al as u32);
        self.clk(Self::timing(4, 2));
    }

    fn das(&mut self, _bus: &mut impl common::Bus) {
        let old_al = self.regs.byte(ByteReg::AL);
        let old_cf = self.flags.cf();
        let old_af = self.flags.af();
        let mut al = old_al;
        let mut carry = old_cf;

        if (old_al & 0x0F) > 9 || old_af {
            al = al.wrapping_sub(6);
            carry = old_cf || old_al < 6;
            self.flags.aux_val = 1;
        } else {
            self.flags.aux_val = 0;
        }
        if old_al > 0x99 || old_cf {
            al = al.wrapping_sub(0x60);
            carry = true;
        }
        self.flags.carry_val = u32::from(carry);
        self.regs.set_byte(ByteReg::AL, al);
        self.flags.set_szpf_byte(al as u32);
        self.clk(Self::timing(4, 2));
    }

    fn aaa(&mut self, _bus: &mut impl common::Bus) {
        if (self.regs.byte(ByteReg::AL) & 0x0F) > 9 || self.flags.af() {
            let ax = self.regs.word(WordReg::AX).wrapping_add(0x0106);
            self.regs.set_word(WordReg::AX, ax);
            let val = self.regs.byte(ByteReg::AL) & 0x0F;
            self.regs.set_byte(ByteReg::AL, val);
            self.flags.aux_val = 1;
            self.flags.carry_val = 1;
        } else {
            let al = self.regs.byte(ByteReg::AL) & 0x0F;
            self.regs.set_byte(ByteReg::AL, al);
            self.flags.aux_val = 0;
            self.flags.carry_val = 0;
        }
        self.clk(Self::timing(4, 3));
    }

    fn aas(&mut self, _bus: &mut impl common::Bus) {
        if (self.regs.byte(ByteReg::AL) & 0x0F) > 9 || self.flags.af() {
            let ax = self.regs.word(WordReg::AX).wrapping_sub(0x0106);
            self.regs.set_word(WordReg::AX, ax);
            let val = self.regs.byte(ByteReg::AL) & 0x0F;
            self.regs.set_byte(ByteReg::AL, val);
            self.flags.aux_val = 1;
            self.flags.carry_val = 1;
        } else {
            let al = self.regs.byte(ByteReg::AL) & 0x0F;
            self.regs.set_byte(ByteReg::AL, al);
            self.flags.aux_val = 0;
            self.flags.carry_val = 0;
        }
        self.clk(Self::timing(4, 3));
    }

    fn aam(&mut self, bus: &mut impl common::Bus) {
        let base = self.fetch(bus);
        if base == 0 {
            self.regs.set_byte(ByteReg::AH, 0xFF);
            let val = self.regs.byte(ByteReg::AL) as u32;
            self.flags.set_szpf_byte(val);
            self.clk(Self::timing(17, 15));
            return;
        }
        let al = self.regs.byte(ByteReg::AL);
        self.regs.set_byte(ByteReg::AH, al / base);
        self.regs.set_byte(ByteReg::AL, al % base);
        let val = self.regs.byte(ByteReg::AL) as u32;
        self.flags.set_szpf_byte(val);
        self.clk(Self::timing(17, 15));
    }

    fn aad(&mut self, bus: &mut impl common::Bus) {
        let base = self.fetch(bus);
        let al = self.regs.byte(ByteReg::AL);
        let ah = self.regs.byte(ByteReg::AH);
        let result = al.wrapping_add(ah.wrapping_mul(base));
        self.regs.set_byte(ByteReg::AL, result);
        self.regs.set_byte(ByteReg::AH, 0);
        self.flags.set_szpf_byte(result as u32);
        self.clk(Self::timing(19, 14));
    }

    fn salc(&mut self) {
        let val = if self.flags.cf() { 0xFF } else { 0x00 };
        self.regs.set_byte(ByteReg::AL, val);
        self.clk(Self::timing(2, 2));
    }

    fn clc(&mut self) {
        self.flags.carry_val = 0;
        self.clk(Self::timing(2, 2));
    }

    fn stc(&mut self) {
        self.flags.carry_val = 1;
        self.clk(Self::timing(2, 2));
    }

    fn cli(&mut self, bus: &mut impl common::Bus) -> Step {
        if self.is_protected_mode() && self.cpl() > u16::from(self.flags.iopl) {
            self.raise_fault_with_code(13, 0, bus)?;
            return Ok(());
        }
        self.flags.if_flag = false;
        self.clk(Self::timing(3, 5));
        Ok(())
    }

    fn sti(&mut self, bus: &mut impl common::Bus) -> Step {
        if self.is_protected_mode() && self.cpl() > u16::from(self.flags.iopl) {
            self.raise_fault_with_code(13, 0, bus)?;
            return Ok(());
        }
        self.flags.if_flag = true;
        self.no_interrupt = 1;
        self.clk(Self::timing(3, 5));
        Ok(())
    }

    fn cld(&mut self) {
        self.flags.df = false;
        self.clk(Self::timing(2, 2));
    }

    fn std(&mut self) {
        self.flags.df = true;
        self.clk(Self::timing(2, 2));
    }

    fn cmc(&mut self) {
        self.flags.carry_val = if self.flags.cf() { 0 } else { 1 };
        self.clk(Self::timing(2, 2));
    }

    fn hlt(&mut self, bus: &mut impl common::Bus) -> Step {
        if self.is_protected_mode() && self.cpl() != 0 {
            self.raise_fault_with_code(13, 0, bus)?;
            return Ok(());
        }
        self.halted = true;
        self.clk(Self::timing(5, 4));
        Ok(())
    }

    fn invalid(&mut self, bus: &mut impl common::Bus) -> Step {
        // #UD (Invalid Opcode, INT 6): introduced with the 80286.
        // The fault pushes CS:IP pointing to the faulting opcode.
        self.raise_fault(6, bus)?;
        Ok(())
    }
}
