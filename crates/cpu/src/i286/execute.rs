use super::{
    ADDRESS_MASK, EaClass, I286, address_is_odd, modrm,
    timing::{
        I286ControlTransferTimingTemplate, I286DemandPrefetchPolicy, I286FinishState,
        I286FpuEscapeTiming, LOCK_PREFIX_OVERLAP_CREDIT, MOFFS_PREFIX_OVERLAP_CREDIT,
        STACK_WORD_OVERLAP_CREDIT,
    },
};
use crate::{ByteReg, SegReg16, WordReg};

#[derive(Clone, Copy)]
struct I286BinaryAluTimingTemplate {
    register_eu_cycles: i32,
    memory_eu_base_cycles: i32,
    odd_word_bus_accesses: i32,
}

#[derive(Clone, Copy)]
struct I286MoveTimingTemplate {
    register_eu_cycles: i32,
    memory_eu_base_cycles: i32,
    odd_word_bus_accesses: i32,
}

#[derive(Clone, Copy)]
struct I286ShortBranchTimingTemplate {
    not_taken_cycles: i32,
    taken_control_transfer: I286ControlTransferTimingTemplate,
}

const BINARY_ALU_WRITE_BYTE_TIMING: I286BinaryAluTimingTemplate = I286BinaryAluTimingTemplate {
    register_eu_cycles: 3,
    memory_eu_base_cycles: 6,
    odd_word_bus_accesses: 0,
};
const BINARY_ALU_RM_WRITE_WORD_TIMING: I286BinaryAluTimingTemplate = I286BinaryAluTimingTemplate {
    register_eu_cycles: 3,
    memory_eu_base_cycles: 6,
    odd_word_bus_accesses: 0,
};
const BINARY_ALU_REG_WRITE_WORD_TIMING: I286BinaryAluTimingTemplate = I286BinaryAluTimingTemplate {
    register_eu_cycles: 3,
    memory_eu_base_cycles: 6,
    odd_word_bus_accesses: 0,
};
const BINARY_ALU_COMPARE_RM_BYTE_TIMING: I286BinaryAluTimingTemplate =
    I286BinaryAluTimingTemplate {
        register_eu_cycles: 3,
        memory_eu_base_cycles: 6,
        odd_word_bus_accesses: 0,
    };
const BINARY_ALU_COMPARE_RM_WORD_TIMING: I286BinaryAluTimingTemplate =
    I286BinaryAluTimingTemplate {
        register_eu_cycles: 3,
        memory_eu_base_cycles: 6,
        odd_word_bus_accesses: 0,
    };
const BINARY_ALU_COMPARE_REG_BYTE_TIMING: I286BinaryAluTimingTemplate =
    I286BinaryAluTimingTemplate {
        register_eu_cycles: 3,
        memory_eu_base_cycles: 5,
        odd_word_bus_accesses: 0,
    };
const BINARY_ALU_COMPARE_REG_WORD_TIMING: I286BinaryAluTimingTemplate =
    I286BinaryAluTimingTemplate {
        register_eu_cycles: 3,
        memory_eu_base_cycles: 5,
        odd_word_bus_accesses: 0,
    };
const MOV_WRITE_BYTE_TIMING: I286MoveTimingTemplate = I286MoveTimingTemplate {
    register_eu_cycles: 3,
    memory_eu_base_cycles: 2,
    odd_word_bus_accesses: 0,
};
const MOV_WRITE_WORD_TIMING: I286MoveTimingTemplate = I286MoveTimingTemplate {
    register_eu_cycles: 3,
    memory_eu_base_cycles: 1,
    odd_word_bus_accesses: 0,
};
const MOV_READ_BYTE_TIMING: I286MoveTimingTemplate = I286MoveTimingTemplate {
    register_eu_cycles: 3,
    memory_eu_base_cycles: 4,
    odd_word_bus_accesses: 0,
};
const MOV_READ_WORD_TIMING: I286MoveTimingTemplate = I286MoveTimingTemplate {
    register_eu_cycles: 3,
    memory_eu_base_cycles: 4,
    odd_word_bus_accesses: 0,
};

const SHORT_JCC_TIMING: I286ShortBranchTimingTemplate = I286ShortBranchTimingTemplate {
    not_taken_cycles: 5,
    taken_control_transfer: I286ControlTransferTimingTemplate {
        initial_internal_cycles: 2,
        restart_prefetch_fetches: 3,
        final_internal_cycles: 2,
    },
};

pub(super) const NEAR_CONTROL_TRANSFER_TIMING: I286ControlTransferTimingTemplate =
    I286ControlTransferTimingTemplate {
        initial_internal_cycles: 2,
        restart_prefetch_fetches: 3,
        final_internal_cycles: 2,
    };

const LOCK_SHORT_JMP_CONTROL_TRANSFER_TIMING: I286ControlTransferTimingTemplate =
    I286ControlTransferTimingTemplate {
        initial_internal_cycles: 3,
        restart_prefetch_fetches: 3,
        final_internal_cycles: 2,
    };

const FAR_IMMEDIATE_CONTROL_TRANSFER_TIMING: I286ControlTransferTimingTemplate =
    I286ControlTransferTimingTemplate {
        initial_internal_cycles: 6,
        restart_prefetch_fetches: 3,
        final_internal_cycles: 2,
    };

const PREFIXED_FAR_IMMEDIATE_CONTROL_TRANSFER_TIMING: I286ControlTransferTimingTemplate =
    I286ControlTransferTimingTemplate {
        initial_internal_cycles: 5,
        restart_prefetch_fetches: 3,
        final_internal_cycles: 2,
    };

const LOCK_PREFIXED_FAR_IMMEDIATE_CONTROL_TRANSFER_TIMING: I286ControlTransferTimingTemplate =
    I286ControlTransferTimingTemplate {
        initial_internal_cycles: 7,
        restart_prefetch_fetches: 3,
        final_internal_cycles: 2,
    };

const LOOP_CONTROL_TRANSFER_TIMING: I286ControlTransferTimingTemplate =
    I286ControlTransferTimingTemplate {
        initial_internal_cycles: 3,
        restart_prefetch_fetches: 3,
        final_internal_cycles: 2,
    };

const IRET_CONTROL_TRANSFER_TIMING: I286ControlTransferTimingTemplate =
    I286ControlTransferTimingTemplate {
        initial_internal_cycles: 4,
        restart_prefetch_fetches: 3,
        final_internal_cycles: 2,
    };

const LOCK_IRET_CONTROL_TRANSFER_TIMING: I286ControlTransferTimingTemplate =
    I286ControlTransferTimingTemplate {
        initial_internal_cycles: 5,
        restart_prefetch_fetches: 3,
        final_internal_cycles: 2,
    };

const LOCK_LOOP_CONTROL_TRANSFER_TIMING: I286ControlTransferTimingTemplate =
    I286ControlTransferTimingTemplate {
        initial_internal_cycles: 4,
        restart_prefetch_fetches: 3,
        final_internal_cycles: 2,
    };

const LOCK_JCXZ_CONTROL_TRANSFER_TIMING: I286ControlTransferTimingTemplate =
    I286ControlTransferTimingTemplate {
        initial_internal_cycles: 1,
        restart_prefetch_fetches: 3,
        final_internal_cycles: 2,
    };

impl I286 {
    #[inline(always)]
    fn clk_accumulator_imm8_template(&mut self, bus: &mut impl common::Bus) {
        self.clk_prefetch(bus, 4);
    }

    #[inline(always)]
    fn clk_accumulator_imm16_template(&mut self, bus: &mut impl common::Bus) {
        self.clk_prefetch(bus, 5);
    }

    #[inline(always)]
    fn clk_binary_alu_byte_template(
        &mut self,
        bus: &mut impl common::Bus,
        modrm: u8,
        timing: I286BinaryAluTimingTemplate,
    ) {
        self.clk_modrm_prefetch(
            bus,
            modrm,
            timing.register_eu_cycles,
            timing.memory_eu_base_cycles,
        );
    }

    #[inline(always)]
    fn clk_binary_alu_word_template(
        &mut self,
        bus: &mut impl common::Bus,
        modrm: u8,
        timing: I286BinaryAluTimingTemplate,
    ) {
        self.clk_modrm_word_prefetch(
            bus,
            modrm,
            timing.register_eu_cycles,
            timing.memory_eu_base_cycles,
            timing.odd_word_bus_accesses,
        );
    }

    #[inline(always)]
    fn clk_move_byte_template(
        &mut self,
        bus: &mut impl common::Bus,
        modrm: u8,
        timing: I286MoveTimingTemplate,
    ) {
        self.clk_modrm_prefetch(
            bus,
            modrm,
            timing.register_eu_cycles,
            timing.memory_eu_base_cycles,
        );
    }

    #[inline(always)]
    fn clk_move_word_template(
        &mut self,
        bus: &mut impl common::Bus,
        modrm: u8,
        timing: I286MoveTimingTemplate,
    ) {
        self.clk_modrm_word_prefetch(
            bus,
            modrm,
            timing.register_eu_cycles,
            timing.memory_eu_base_cycles,
            timing.odd_word_bus_accesses,
        );
    }

    fn prepare_imul_imm8_memory_operand(&mut self, modrm: u8) {
        self.prepare_sign_extended_immediate_memory_operand(modrm);

        let prefix_count = self.state.timing.prefix_count();
        if prefix_count == 0 {
            return;
        }

        match self.ea_class {
            EaClass::Direct => {
                if prefix_count & 1 == 0 {
                    self.state.timing.note_demand_prefetch_policy(
                        I286DemandPrefetchPolicy::BeforePrefetchGapThenPrefetch,
                    );
                    self.state.timing.note_demand_prefetch_limit(2);
                } else {
                    self.state.timing.note_demand_prefetch_policy(
                        I286DemandPrefetchPolicy::BeforeAndAfterTurnaroundThenGap,
                    );
                }
            }
            EaClass::Disp16Double => {
                self.state.timing.note_demand_prefetch_policy(
                    I286DemandPrefetchPolicy::BeforeAndAfterTurnaroundThenGap,
                );
            }
            EaClass::Disp8Single if prefix_count & 1 == 1 => {
                self.state.timing.note_demand_prefetch_policy(
                    I286DemandPrefetchPolicy::AfterTurnaroundAuThenGapThenPrefetch,
                );
            }
            _ => {}
        }
    }

    fn imul_imm8_memory_base_cycles(&self) -> i32 {
        let prefix_count = self.state.timing.prefix_count();

        if self.state.timing.lock_active() && self.ea_class == EaClass::DoubleRegister {
            21
        } else if prefix_count != 0 && self.ea_class == EaClass::Direct {
            if prefix_count & 1 == 0 { 26 } else { 24 }
        } else if prefix_count != 0 && self.ea_class == EaClass::Disp16Double {
            if prefix_count & 1 == 1 { 23 } else { 25 }
        } else {
            24
        }
    }

    #[inline(always)]
    fn borrow_no_displacement_read_only_cycles(&mut self, modrm: u8) {
        if modrm::modrm_is_register(modrm) {
            return;
        }

        if self.ea_class.is_no_displacement_memory()
            || self.state.timing.lock_active()
                && !self.state.timing.lock_prefix_followed_by_prefix()
                && self.ea_class.is_disp8()
        {
            self.state
                .timing
                .borrow_internal_cycles(LOCK_PREFIX_OVERLAP_CREDIT);
        }
    }

    pub(super) fn dispatch(&mut self, opcode: u8, bus: &mut impl common::Bus) {
        match opcode {
            // ADD
            0x00 => self.add_br8(bus),
            0x01 => self.add_wr16(bus),
            0x02 => self.add_r8b(bus),
            0x03 => self.add_r16w(bus),
            0x04 => self.add_ald8(bus),
            0x05 => self.add_axd16(bus),
            0x06 => self.push_seg(SegReg16::ES, bus),
            0x07 => self.pop_seg(SegReg16::ES, bus),

            // OR
            0x08 => self.or_br8(bus),
            0x09 => self.or_wr16(bus),
            0x0A => self.or_r8b(bus),
            0x0B => self.or_r16w(bus),
            0x0C => self.or_ald8(bus),
            0x0D => self.or_axd16(bus),
            0x0E => self.push_seg(SegReg16::CS, bus),
            0x0F => self.extended_0f(bus),

            // ADC
            0x10 => self.adc_br8(bus),
            0x11 => self.adc_wr16(bus),
            0x12 => self.adc_r8b(bus),
            0x13 => self.adc_r16w(bus),
            0x14 => self.adc_ald8(bus),
            0x15 => self.adc_axd16(bus),
            0x16 => self.push_seg(SegReg16::SS, bus),
            0x17 => {
                self.pop_seg(SegReg16::SS, bus);
                self.inhibit_all = 1;
            }

            // SBB
            0x18 => self.sbb_br8(bus),
            0x19 => self.sbb_wr16(bus),
            0x1A => self.sbb_r8b(bus),
            0x1B => self.sbb_r16w(bus),
            0x1C => self.sbb_ald8(bus),
            0x1D => self.sbb_axd16(bus),
            0x1E => self.push_seg(SegReg16::DS, bus),
            0x1F => self.pop_seg(SegReg16::DS, bus),

            // AND
            0x20 => self.and_br8(bus),
            0x21 => self.and_wr16(bus),
            0x22 => self.and_r8b(bus),
            0x23 => self.and_r16w(bus),
            0x24 => self.and_ald8(bus),
            0x25 => self.and_axd16(bus),
            0x26 => self.invalid(bus),
            0x27 => self.daa(bus),

            // SUB
            0x28 => self.sub_br8(bus),
            0x29 => self.sub_wr16(bus),
            0x2A => self.sub_r8b(bus),
            0x2B => self.sub_r16w(bus),
            0x2C => self.sub_ald8(bus),
            0x2D => self.sub_axd16(bus),
            0x2E => self.invalid(bus),
            0x2F => self.das(bus),

            // XOR
            0x30 => self.xor_br8(bus),
            0x31 => self.xor_wr16(bus),
            0x32 => self.xor_r8b(bus),
            0x33 => self.xor_r16w(bus),
            0x34 => self.xor_ald8(bus),
            0x35 => self.xor_axd16(bus),
            0x36 => self.invalid(bus),
            0x37 => self.aaa(bus),

            // CMP
            0x38 => self.cmp_br8(bus),
            0x39 => self.cmp_wr16(bus),
            0x3A => self.cmp_r8b(bus),
            0x3B => self.cmp_r16w(bus),
            0x3C => self.cmp_ald8(bus),
            0x3D => self.cmp_axd16(bus),
            0x3E => self.invalid(bus),
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
            0x50 => self.push_word_reg(WordReg::AX, bus),
            0x51 => self.push_word_reg(WordReg::CX, bus),
            0x52 => self.push_word_reg(WordReg::DX, bus),
            0x53 => self.push_word_reg(WordReg::BX, bus),
            0x54 => self.push_sp(bus),
            0x55 => self.push_word_reg(WordReg::BP, bus),
            0x56 => self.push_word_reg(WordReg::SI, bus),
            0x57 => self.push_word_reg(WordReg::DI, bus),

            // POP word registers
            0x58 => self.pop_word_reg(WordReg::AX, bus),
            0x59 => self.pop_word_reg(WordReg::CX, bus),
            0x5A => self.pop_word_reg(WordReg::DX, bus),
            0x5B => self.pop_word_reg(WordReg::BX, bus),
            0x5C => self.pop_word_reg(WordReg::SP, bus),
            0x5D => self.pop_word_reg(WordReg::BP, bus),
            0x5E => self.pop_word_reg(WordReg::SI, bus),
            0x5F => self.pop_word_reg(WordReg::DI, bus),

            // 80186 instructions
            0x60 => self.pusha(bus),
            0x61 => self.popa(bus),
            0x62 => self.bound(bus),
            0x63 => self.arpl(bus),
            0x64 => self.invalid(bus),
            0x65 => self.invalid(bus),
            0x66 => self.invalid(bus),
            0x67 => self.invalid(bus),
            0x68 => self.push_imm16(bus),
            0x69 => self.imul_r16w_imm16(bus),
            0x6A => self.push_imm8(bus),
            0x6B => self.imul_r16w_imm8(bus),
            0x6C => self.insb(bus),
            0x6D => self.insw(bus),
            0x6E => self.outsb(bus),
            0x6F => self.outsw(bus),

            // Jcc (short jumps)
            0x70 => self.jcc(bus, self.flags.of(), SHORT_JCC_TIMING),
            0x71 => self.jcc(bus, !self.flags.of(), SHORT_JCC_TIMING),
            0x72 => self.jcc(bus, self.flags.cf(), SHORT_JCC_TIMING),
            0x73 => self.jcc(bus, !self.flags.cf(), SHORT_JCC_TIMING),
            0x74 => self.jcc(bus, self.flags.zf(), SHORT_JCC_TIMING),
            0x75 => self.jcc(bus, !self.flags.zf(), SHORT_JCC_TIMING),
            0x76 => self.jcc(bus, self.flags.cf() || self.flags.zf(), SHORT_JCC_TIMING),
            0x77 => self.jcc(bus, !self.flags.cf() && !self.flags.zf(), SHORT_JCC_TIMING),
            0x78 => self.jcc(bus, self.flags.sf(), SHORT_JCC_TIMING),
            0x79 => self.jcc(bus, !self.flags.sf(), SHORT_JCC_TIMING),
            0x7A => self.jcc(bus, self.flags.pf(), SHORT_JCC_TIMING),
            0x7B => self.jcc(bus, !self.flags.pf(), SHORT_JCC_TIMING),
            0x7C => self.jcc(bus, self.flags.sf() != self.flags.of(), SHORT_JCC_TIMING),
            0x7D => self.jcc(bus, self.flags.sf() == self.flags.of(), SHORT_JCC_TIMING),
            0x7E => self.jcc(
                bus,
                self.flags.zf() || (self.flags.sf() != self.flags.of()),
                SHORT_JCC_TIMING,
            ),
            0x7F => self.jcc(
                bus,
                !self.flags.zf() && (self.flags.sf() == self.flags.of()),
                SHORT_JCC_TIMING,
            ),

            // Group 1
            0x80 => self.group_80(bus),
            0x81 => self.group_81(bus),
            0x82 => self.group_82(bus),
            0x83 => self.group_83(bus),

            // TEST
            0x84 => self.test_br8(bus),
            0x85 => self.test_wr16(bus),

            // XCHG
            0x86 => self.xchg_br8(bus),
            0x87 => self.xchg_wr16(bus),

            // MOV r/m, reg
            0x88 => self.mov_br8(bus),
            0x89 => self.mov_wr16(bus),
            0x8A => self.mov_r8b(bus),
            0x8B => self.mov_r16w(bus),

            // MOV r/m, sreg / LEA / MOV sreg, r/m
            0x8C => self.mov_rm_sreg(bus),
            0x8D => self.lea(bus),
            0x8E => self.mov_sreg_rm(bus),
            0x8F => self.pop_rm(bus),

            // XCHG AX, reg / NOP
            0x90 => self.clk(3),
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
            0x9A => self.call_far(bus),
            0x9B => self.clk(7), // WAIT
            0x9C => self.pushf(bus),
            0x9D => self.popf(bus),
            0x9E => self.sahf(),
            0x9F => self.lahf(),

            // MOV AL/AX, [addr] and [addr], AL/AX
            0xA0 => self.mov_al_moffs(bus),
            0xA1 => self.mov_aw_moffs(bus),
            0xA2 => self.mov_moffs_al(bus),
            0xA3 => self.mov_moffs_aw(bus),

            // String ops
            0xA4 => self.movsb(bus),
            0xA5 => self.movsw(bus),
            0xA6 => self.cmpsb(bus),
            0xA7 => self.cmpsw(bus),

            // TEST AL/AX, imm
            0xA8 => self.test_al_imm8(bus),
            0xA9 => self.test_aw_imm16(bus),

            // STOS, LODS, SCAS
            0xAA => self.stosb(bus),
            0xAB => self.stosw(bus),
            0xAC => self.lodsb(bus),
            0xAD => self.lodsw(bus),
            0xAE => self.scasb(bus),
            0xAF => self.scasw(bus),

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
            0xC0 => self.group_c0(bus),
            0xC1 => self.group_c1(bus),

            // RET near imm16, RET near
            0xC2 => self.ret_near_imm(bus),
            0xC3 => self.ret_near(bus),

            // LES, LDS
            0xC4 => self.les(bus),
            0xC5 => self.lds(bus),

            // MOV r/m, imm
            0xC6 => self.mov_rm_imm8(bus),
            0xC7 => self.mov_rm_imm16(bus),

            // ENTER, LEAVE
            0xC8 => self.enter(bus),
            0xC9 => self.leave(bus),

            // RET far imm16, RET far
            0xCA => self.ret_far_imm(bus),
            0xCB => self.ret_far(bus),

            // INT 3, INT imm8, INTO, IRET
            0xCC => self.int3(bus),
            0xCD => self.int_imm(bus),
            0xCE => self.into(bus),
            0xCF => self.iret(bus),

            // Shift/rotate groups
            0xD0 => self.group_d0(bus),
            0xD1 => self.group_d1(bus),
            0xD2 => self.group_d2(bus),
            0xD3 => self.group_d3(bus),

            // AAM, AAD
            0xD4 => self.aam(bus),
            0xD5 => self.aad(bus),

            // undocumented SALC
            0xD6 => self.salc(),

            // XLAT
            0xD7 => self.xlat(bus),

            // FPU escape (NOP on I286)
            0xD8..=0xDF => self.fpu_escape(bus),

            // LOOPNE, LOOPE, LOOP, JCXZ
            0xE0 => self.loopne(bus),
            0xE1 => self.loope(bus),
            0xE2 => self.loop_(bus),
            0xE3 => self.jcxz(bus),

            // IN, OUT
            0xE4 => self.in_al_imm(bus),
            0xE5 => self.in_aw_imm(bus),
            0xE6 => self.out_imm_al(bus),
            0xE7 => self.out_imm_aw(bus),

            // CALL near, JMP near, JMP far, JMP short
            0xE8 => self.call_near(bus),
            0xE9 => self.jmp_near(bus),
            0xEA => self.jmp_far(bus),
            0xEB => self.jmp_short(bus),

            // IN, OUT (DX port)
            0xEC => self.in_al_dw(bus),
            0xED => self.in_aw_dw(bus),
            0xEE => self.out_dw_al(bus),
            0xEF => self.out_dw_aw(bus),

            0xF0 => self.invalid(bus),
            0xF1 => self.invalid(bus),

            // REPNE, REPE
            0xF2 => self.repne(bus),
            0xF3 => self.repe(bus),

            // HLT
            0xF4 => self.hlt(),

            // CMC
            0xF5 => self.cmc(),

            // Group 3 byte/word
            0xF6 => self.group_f6(bus),
            0xF7 => self.group_f7(bus),

            // CLC, STC, CLI, STI, CLD, STD
            0xF8 => self.clc(),
            0xF9 => self.stc(),
            0xFA => self.cli(),
            0xFB => self.sti(),
            0xFC => self.cld(),
            0xFD => self.std(),

            // Group 4/5
            0xFE => self.group_fe(bus),
            0xFF => self.group_ff(bus),
        }
    }

    fn add_br8(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let src = self.regs.byte(self.reg_byte(modrm));
        let dst = self.get_rm_byte(modrm, bus);
        let result = self.alu_add_byte(dst, src);
        self.putback_rm_byte(modrm, result, bus);
        self.clk_binary_alu_byte_template(bus, modrm, BINARY_ALU_WRITE_BYTE_TIMING);
    }

    fn add_wr16(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let src = self.regs.word(self.reg_word(modrm));
        let dst = self.get_rm_word(modrm, bus);
        let result = self.alu_add_word(dst, src);
        self.putback_rm_word(modrm, result, bus);
        self.clk_binary_alu_word_template(bus, modrm, BINARY_ALU_RM_WRITE_WORD_TIMING);
    }

    fn add_r8b(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let dst = self.regs.byte(self.reg_byte(modrm));
        let src = self.get_rm_byte(modrm, bus);
        let result = self.alu_add_byte(dst, src);
        let reg = self.reg_byte(modrm);
        self.regs.set_byte(reg, result);
        self.clk_binary_alu_byte_template(bus, modrm, BINARY_ALU_WRITE_BYTE_TIMING);
    }

    fn add_r16w(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let dst = self.regs.word(self.reg_word(modrm));
        let src = self.get_rm_word(modrm, bus);
        let result = self.alu_add_word(dst, src);
        let reg = self.reg_word(modrm);
        self.regs.set_word(reg, result);
        self.clk_binary_alu_word_template(bus, modrm, BINARY_ALU_REG_WRITE_WORD_TIMING);
    }

    fn add_ald8(&mut self, bus: &mut impl common::Bus) {
        let src = self.fetch(bus);
        let dst = self.regs.byte(ByteReg::AL);
        let result = self.alu_add_byte(dst, src);
        self.regs.set_byte(ByteReg::AL, result);
        self.clk_accumulator_imm8_template(bus);
    }

    fn add_axd16(&mut self, bus: &mut impl common::Bus) {
        let src = self.fetchword(bus);
        let dst = self.regs.word(WordReg::AX);
        let result = self.alu_add_word(dst, src);
        self.regs.set_word(WordReg::AX, result);
        self.clk_accumulator_imm16_template(bus);
    }

    fn or_br8(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let src = self.regs.byte(self.reg_byte(modrm));
        let dst = self.get_rm_byte(modrm, bus);
        let result = self.alu_or_byte(dst, src);
        self.putback_rm_byte(modrm, result, bus);
        self.clk_binary_alu_byte_template(bus, modrm, BINARY_ALU_WRITE_BYTE_TIMING);
    }

    fn or_wr16(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let src = self.regs.word(self.reg_word(modrm));
        let dst = self.get_rm_word(modrm, bus);
        let result = self.alu_or_word(dst, src);
        self.putback_rm_word(modrm, result, bus);
        self.clk_binary_alu_word_template(bus, modrm, BINARY_ALU_RM_WRITE_WORD_TIMING);
    }

    fn or_r8b(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let dst = self.regs.byte(self.reg_byte(modrm));
        let src = self.get_rm_byte(modrm, bus);
        let result = self.alu_or_byte(dst, src);
        let reg = self.reg_byte(modrm);
        self.regs.set_byte(reg, result);
        self.clk_binary_alu_byte_template(bus, modrm, BINARY_ALU_WRITE_BYTE_TIMING);
    }

    fn or_r16w(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let dst = self.regs.word(self.reg_word(modrm));
        let src = self.get_rm_word(modrm, bus);
        let result = self.alu_or_word(dst, src);
        let reg = self.reg_word(modrm);
        self.regs.set_word(reg, result);
        self.clk_binary_alu_word_template(bus, modrm, BINARY_ALU_REG_WRITE_WORD_TIMING);
    }

    fn or_ald8(&mut self, bus: &mut impl common::Bus) {
        let src = self.fetch(bus);
        let dst = self.regs.byte(ByteReg::AL);
        let result = self.alu_or_byte(dst, src);
        self.regs.set_byte(ByteReg::AL, result);
        self.clk_accumulator_imm8_template(bus);
    }

    fn or_axd16(&mut self, bus: &mut impl common::Bus) {
        let src = self.fetchword(bus);
        let dst = self.regs.word(WordReg::AX);
        let result = self.alu_or_word(dst, src);
        self.regs.set_word(WordReg::AX, result);
        self.clk_accumulator_imm16_template(bus);
    }

    fn adc_br8(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let src = self.regs.byte(self.reg_byte(modrm));
        let dst = self.get_rm_byte(modrm, bus);
        let cf = self.flags.cf_val();
        let result = self.alu_adc_byte(dst, src, cf);
        self.putback_rm_byte(modrm, result, bus);
        self.clk_binary_alu_byte_template(bus, modrm, BINARY_ALU_WRITE_BYTE_TIMING);
    }

    fn adc_wr16(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let src = self.regs.word(self.reg_word(modrm));
        let dst = self.get_rm_word(modrm, bus);
        let cf = self.flags.cf_val();
        let result = self.alu_adc_word(dst, src, cf);
        self.putback_rm_word(modrm, result, bus);
        self.clk_binary_alu_word_template(bus, modrm, BINARY_ALU_RM_WRITE_WORD_TIMING);
    }

    fn adc_r8b(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let dst = self.regs.byte(self.reg_byte(modrm));
        let src = self.get_rm_byte(modrm, bus);
        let cf = self.flags.cf_val();
        let result = self.alu_adc_byte(dst, src, cf);
        let reg = self.reg_byte(modrm);
        self.regs.set_byte(reg, result);
        self.clk_binary_alu_byte_template(bus, modrm, BINARY_ALU_WRITE_BYTE_TIMING);
    }

    fn adc_r16w(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let dst = self.regs.word(self.reg_word(modrm));
        let src = self.get_rm_word(modrm, bus);
        let cf = self.flags.cf_val();
        let result = self.alu_adc_word(dst, src, cf);
        let reg = self.reg_word(modrm);
        self.regs.set_word(reg, result);
        self.clk_binary_alu_word_template(bus, modrm, BINARY_ALU_REG_WRITE_WORD_TIMING);
    }

    fn adc_ald8(&mut self, bus: &mut impl common::Bus) {
        let src = self.fetch(bus);
        let dst = self.regs.byte(ByteReg::AL);
        let cf = self.flags.cf_val();
        let result = self.alu_adc_byte(dst, src, cf);
        self.regs.set_byte(ByteReg::AL, result);
        self.clk_accumulator_imm8_template(bus);
    }

    fn adc_axd16(&mut self, bus: &mut impl common::Bus) {
        let src = self.fetchword(bus);
        let dst = self.regs.word(WordReg::AX);
        let cf = self.flags.cf_val();
        let result = self.alu_adc_word(dst, src, cf);
        self.regs.set_word(WordReg::AX, result);
        self.clk_accumulator_imm16_template(bus);
    }

    fn sbb_br8(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let src = self.regs.byte(self.reg_byte(modrm));
        let dst = self.get_rm_byte(modrm, bus);
        let cf = self.flags.cf_val();
        let result = self.alu_sbb_byte(dst, src, cf);
        self.putback_rm_byte(modrm, result, bus);
        self.clk_binary_alu_byte_template(bus, modrm, BINARY_ALU_WRITE_BYTE_TIMING);
    }

    fn sbb_wr16(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let src = self.regs.word(self.reg_word(modrm));
        let dst = self.get_rm_word(modrm, bus);
        let cf = self.flags.cf_val();
        let result = self.alu_sbb_word(dst, src, cf);
        self.putback_rm_word(modrm, result, bus);
        self.clk_binary_alu_word_template(bus, modrm, BINARY_ALU_RM_WRITE_WORD_TIMING);
    }

    fn sbb_r8b(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let dst = self.regs.byte(self.reg_byte(modrm));
        let src = self.get_rm_byte(modrm, bus);
        let cf = self.flags.cf_val();
        let result = self.alu_sbb_byte(dst, src, cf);
        let reg = self.reg_byte(modrm);
        self.regs.set_byte(reg, result);
        self.clk_binary_alu_byte_template(bus, modrm, BINARY_ALU_WRITE_BYTE_TIMING);
    }

    fn sbb_r16w(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let dst = self.regs.word(self.reg_word(modrm));
        let src = self.get_rm_word(modrm, bus);
        let cf = self.flags.cf_val();
        let result = self.alu_sbb_word(dst, src, cf);
        let reg = self.reg_word(modrm);
        self.regs.set_word(reg, result);
        self.clk_binary_alu_word_template(bus, modrm, BINARY_ALU_REG_WRITE_WORD_TIMING);
    }

    fn sbb_ald8(&mut self, bus: &mut impl common::Bus) {
        let src = self.fetch(bus);
        let dst = self.regs.byte(ByteReg::AL);
        let cf = self.flags.cf_val();
        let result = self.alu_sbb_byte(dst, src, cf);
        self.regs.set_byte(ByteReg::AL, result);
        self.clk_accumulator_imm8_template(bus);
    }

    fn sbb_axd16(&mut self, bus: &mut impl common::Bus) {
        let src = self.fetchword(bus);
        let dst = self.regs.word(WordReg::AX);
        let cf = self.flags.cf_val();
        let result = self.alu_sbb_word(dst, src, cf);
        self.regs.set_word(WordReg::AX, result);
        self.clk_accumulator_imm16_template(bus);
    }

    fn and_br8(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let src = self.regs.byte(self.reg_byte(modrm));
        let dst = self.get_rm_byte(modrm, bus);
        let result = self.alu_and_byte(dst, src);
        self.putback_rm_byte(modrm, result, bus);
        self.clk_binary_alu_byte_template(bus, modrm, BINARY_ALU_WRITE_BYTE_TIMING);
    }

    fn and_wr16(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let src = self.regs.word(self.reg_word(modrm));
        let dst = self.get_rm_word(modrm, bus);
        let result = self.alu_and_word(dst, src);
        self.putback_rm_word(modrm, result, bus);
        self.clk_binary_alu_word_template(bus, modrm, BINARY_ALU_RM_WRITE_WORD_TIMING);
    }

    fn and_r8b(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let dst = self.regs.byte(self.reg_byte(modrm));
        let src = self.get_rm_byte(modrm, bus);
        let result = self.alu_and_byte(dst, src);
        let reg = self.reg_byte(modrm);
        self.regs.set_byte(reg, result);
        self.clk_binary_alu_byte_template(bus, modrm, BINARY_ALU_WRITE_BYTE_TIMING);
    }

    fn and_r16w(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let dst = self.regs.word(self.reg_word(modrm));
        let src = self.get_rm_word(modrm, bus);
        let result = self.alu_and_word(dst, src);
        let reg = self.reg_word(modrm);
        self.regs.set_word(reg, result);
        self.clk_binary_alu_word_template(bus, modrm, BINARY_ALU_REG_WRITE_WORD_TIMING);
    }

    fn and_ald8(&mut self, bus: &mut impl common::Bus) {
        let src = self.fetch(bus);
        let dst = self.regs.byte(ByteReg::AL);
        let result = self.alu_and_byte(dst, src);
        self.regs.set_byte(ByteReg::AL, result);
        self.clk_accumulator_imm8_template(bus);
    }

    fn and_axd16(&mut self, bus: &mut impl common::Bus) {
        let src = self.fetchword(bus);
        let dst = self.regs.word(WordReg::AX);
        let result = self.alu_and_word(dst, src);
        self.regs.set_word(WordReg::AX, result);
        self.clk_accumulator_imm16_template(bus);
    }

    fn sub_br8(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let src = self.regs.byte(self.reg_byte(modrm));
        let dst = self.get_rm_byte(modrm, bus);
        let result = self.alu_sub_byte(dst, src);
        self.putback_rm_byte(modrm, result, bus);
        self.clk_binary_alu_byte_template(bus, modrm, BINARY_ALU_WRITE_BYTE_TIMING);
    }

    fn sub_wr16(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let src = self.regs.word(self.reg_word(modrm));
        let dst = self.get_rm_word(modrm, bus);
        let result = self.alu_sub_word(dst, src);
        self.putback_rm_word(modrm, result, bus);
        self.clk_binary_alu_word_template(bus, modrm, BINARY_ALU_RM_WRITE_WORD_TIMING);
    }

    fn sub_r8b(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let dst = self.regs.byte(self.reg_byte(modrm));
        let src = self.get_rm_byte(modrm, bus);
        let result = self.alu_sub_byte(dst, src);
        let reg = self.reg_byte(modrm);
        self.regs.set_byte(reg, result);
        self.clk_binary_alu_byte_template(bus, modrm, BINARY_ALU_WRITE_BYTE_TIMING);
    }

    fn sub_r16w(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let dst = self.regs.word(self.reg_word(modrm));
        let src = self.get_rm_word(modrm, bus);
        let result = self.alu_sub_word(dst, src);
        let reg = self.reg_word(modrm);
        self.regs.set_word(reg, result);
        self.clk_binary_alu_word_template(bus, modrm, BINARY_ALU_REG_WRITE_WORD_TIMING);
    }

    fn sub_ald8(&mut self, bus: &mut impl common::Bus) {
        let src = self.fetch(bus);
        let dst = self.regs.byte(ByteReg::AL);
        let result = self.alu_sub_byte(dst, src);
        self.regs.set_byte(ByteReg::AL, result);
        self.clk_accumulator_imm8_template(bus);
    }

    fn sub_axd16(&mut self, bus: &mut impl common::Bus) {
        let src = self.fetchword(bus);
        let dst = self.regs.word(WordReg::AX);
        let result = self.alu_sub_word(dst, src);
        self.regs.set_word(WordReg::AX, result);
        self.clk_accumulator_imm16_template(bus);
    }

    fn xor_br8(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let src = self.regs.byte(self.reg_byte(modrm));
        let dst = self.get_rm_byte(modrm, bus);
        let result = self.alu_xor_byte(dst, src);
        self.putback_rm_byte(modrm, result, bus);
        self.clk_binary_alu_byte_template(bus, modrm, BINARY_ALU_WRITE_BYTE_TIMING);
    }

    fn xor_wr16(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let src = self.regs.word(self.reg_word(modrm));
        let dst = self.get_rm_word(modrm, bus);
        let result = self.alu_xor_word(dst, src);
        self.putback_rm_word(modrm, result, bus);
        self.clk_binary_alu_word_template(bus, modrm, BINARY_ALU_RM_WRITE_WORD_TIMING);
    }

    fn xor_r8b(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let dst = self.regs.byte(self.reg_byte(modrm));
        let src = self.get_rm_byte(modrm, bus);
        let result = self.alu_xor_byte(dst, src);
        let reg = self.reg_byte(modrm);
        self.regs.set_byte(reg, result);
        self.clk_binary_alu_byte_template(bus, modrm, BINARY_ALU_WRITE_BYTE_TIMING);
    }

    fn xor_r16w(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let dst = self.regs.word(self.reg_word(modrm));
        let src = self.get_rm_word(modrm, bus);
        let result = self.alu_xor_word(dst, src);
        let reg = self.reg_word(modrm);
        self.regs.set_word(reg, result);
        self.clk_binary_alu_word_template(bus, modrm, BINARY_ALU_REG_WRITE_WORD_TIMING);
    }

    fn xor_ald8(&mut self, bus: &mut impl common::Bus) {
        let src = self.fetch(bus);
        let dst = self.regs.byte(ByteReg::AL);
        let result = self.alu_xor_byte(dst, src);
        self.regs.set_byte(ByteReg::AL, result);
        self.clk_accumulator_imm8_template(bus);
    }

    fn xor_axd16(&mut self, bus: &mut impl common::Bus) {
        let src = self.fetchword(bus);
        let dst = self.regs.word(WordReg::AX);
        let result = self.alu_xor_word(dst, src);
        self.regs.set_word(WordReg::AX, result);
        self.clk_accumulator_imm16_template(bus);
    }

    fn cmp_br8(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let src = self.regs.byte(self.reg_byte(modrm));
        let dst = self.get_rm_byte(modrm, bus);
        self.alu_sub_byte(dst, src);
        self.borrow_no_displacement_read_only_cycles(modrm);
        self.clk_binary_alu_byte_template(bus, modrm, BINARY_ALU_COMPARE_RM_BYTE_TIMING);
    }

    fn cmp_wr16(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let src = self.regs.word(self.reg_word(modrm));
        let dst = self.get_rm_word(modrm, bus);
        self.alu_sub_word(dst, src);
        self.borrow_no_displacement_read_only_cycles(modrm);
        self.clk_binary_alu_word_template(bus, modrm, BINARY_ALU_COMPARE_RM_WORD_TIMING);
    }

    fn cmp_r8b(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let dst = self.regs.byte(self.reg_byte(modrm));
        let src = self.get_rm_byte(modrm, bus);
        self.alu_sub_byte(dst, src);
        self.clk_binary_alu_byte_template(bus, modrm, BINARY_ALU_COMPARE_REG_BYTE_TIMING);
    }

    fn cmp_r16w(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let dst = self.regs.word(self.reg_word(modrm));
        let src = self.get_rm_word(modrm, bus);
        self.alu_sub_word(dst, src);
        self.clk_binary_alu_word_template(bus, modrm, BINARY_ALU_COMPARE_REG_WORD_TIMING);
    }

    fn cmp_ald8(&mut self, bus: &mut impl common::Bus) {
        let src = self.fetch(bus);
        let dst = self.regs.byte(ByteReg::AL);
        self.alu_sub_byte(dst, src);
        self.clk_accumulator_imm8_template(bus);
    }

    fn cmp_axd16(&mut self, bus: &mut impl common::Bus) {
        let src = self.fetchword(bus);
        let dst = self.regs.word(WordReg::AX);
        self.alu_sub_word(dst, src);
        self.clk_accumulator_imm16_template(bus);
    }

    fn inc_word_reg(&mut self, reg: WordReg) {
        let val = self.regs.word(reg);
        let result = self.alu_inc_word(val);
        self.regs.set_word(reg, result);
        self.clk(2);
    }

    fn dec_word_reg(&mut self, reg: WordReg) {
        let val = self.regs.word(reg);
        let result = self.alu_dec_word(val);
        self.regs.set_word(reg, result);
        self.clk(2);
    }

    fn push_word_reg(&mut self, reg: WordReg, bus: &mut impl common::Bus) {
        let tail_cycles = self.stack_push_tail_cycles(3);
        let val = self.regs.word(reg);
        self.push(bus, val);
        self.clk(tail_cycles);
    }

    pub(super) fn push_sp(&mut self, bus: &mut impl common::Bus) {
        let tail_cycles = self.stack_push_tail_cycles(3);
        let sp = self.regs.word(WordReg::SP);
        self.push(bus, sp);
        self.clk(tail_cycles);
    }

    fn pop_word_reg(&mut self, reg: WordReg, bus: &mut impl common::Bus) {
        let val = self.pop(bus);
        self.regs.set_word(reg, val);
        self.clk(5);
    }

    fn push_seg(&mut self, seg: SegReg16, bus: &mut impl common::Bus) {
        let tail_cycles = self.stack_push_tail_cycles(3);
        let val = self.sregs[seg as usize];
        self.push(bus, val);
        self.clk(tail_cycles);
    }

    fn pop_seg(&mut self, seg: SegReg16, bus: &mut impl common::Bus) {
        let val = self.pop(bus);
        if !self.load_segment(seg, val, bus) {
            return;
        }
        self.clk(5);
    }

    fn pusha(&mut self, bus: &mut impl common::Bus) {
        let sp = self.regs.word(WordReg::SP);
        let tail_cycles = 17 - i32::from(sp & 1);
        let final_sp = sp.wrapping_sub(16);
        let values = [
            self.regs.word(WordReg::DI),
            self.regs.word(WordReg::SI),
            self.regs.word(WordReg::BP),
            sp,
            self.regs.word(WordReg::BX),
            self.regs.word(WordReg::DX),
            self.regs.word(WordReg::CX),
            self.regs.word(WordReg::AX),
        ];
        self.regs.set_word(WordReg::SP, final_sp);
        for (index, value) in values.into_iter().enumerate() {
            self.write_word_seg(
                bus,
                SegReg16::SS,
                final_sp.wrapping_add((index * 2) as u16),
                value,
            );
            self.state
                .timing
                .borrow_internal_cycles(STACK_WORD_OVERLAP_CREDIT);
        }
        self.clk(tail_cycles);
    }

    fn popa(&mut self, bus: &mut impl common::Bus) {
        // Read order matches real hardware: AX is fetched first from sp+14,
        // then DI/SI/BP, the SP slot at sp+6 is discarded, and BX/DX/CX
        // follow. Each read overlaps the next by STACK_WORD_OVERLAP_CREDIT.
        const POPA_READS: [(u16, Option<WordReg>); 8] = [
            (14, Some(WordReg::AX)),
            (0, Some(WordReg::DI)),
            (2, Some(WordReg::SI)),
            (4, Some(WordReg::BP)),
            (6, None),
            (8, Some(WordReg::BX)),
            (10, Some(WordReg::DX)),
            (12, Some(WordReg::CX)),
        ];

        let sp = self.regs.word(WordReg::SP);
        let mut pending_writes: [Option<(WordReg, u16)>; 8] = [None; 8];
        for (index, (offset, destination)) in POPA_READS.iter().enumerate() {
            let value = self.read_word_seg(bus, SegReg16::SS, sp.wrapping_add(*offset));
            self.state
                .timing
                .borrow_internal_cycles(STACK_WORD_OVERLAP_CREDIT);
            if let Some(register) = destination {
                pending_writes[index] = Some((*register, value));
            }
        }
        for write in pending_writes.iter().flatten() {
            self.regs.set_word(write.0, write.1);
        }
        self.regs.set_word(WordReg::SP, sp.wrapping_add(16));
        self.clk(19);
    }

    fn bound(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let val = self.regs.word(self.reg_word(modrm)) as i16;
        if modrm::modrm_is_register(modrm) {
            return;
        }
        self.calc_ea(modrm, bus);
        let ea_pen = if address_is_odd(self.ea) { 8 } else { 0 };
        let low = self.seg_read_word(bus) as i16;
        self.state.timing.suppress_next_memory_read_window();
        let high = self.seg_read_word_at(bus, 2) as i16;
        if val < low || val > high {
            let sp_pen = self.sp_penalty(3);
            self.raise_fault(5, bus);
            self.clk(56 + ea_pen + sp_pen);
        } else {
            self.clk(11);
        }
    }

    fn arpl(&mut self, bus: &mut impl common::Bus) {
        if self.msw & 1 == 0 {
            self.raise_fault(6, bus);
            return;
        }
        let modrm = self.fetch(bus);
        let dst = self.get_rm_word(modrm, bus);
        let src_rpl = self.regs.word(self.reg_word(modrm)) & 3;
        let dst_rpl = dst & 3;
        if dst_rpl < src_rpl {
            let result = (dst & !3) | src_rpl;
            self.putback_rm_word(modrm, result, bus);
            self.flags.zero_val = 0; // ZF=1
        } else {
            self.flags.zero_val = 1; // ZF=0
        }
        self.clk_modrm_prefetch(bus, modrm, 10, 11);
    }

    fn push_imm16(&mut self, bus: &mut impl common::Bus) {
        let tail_cycles = self.stack_push_tail_cycles(3);
        let val = self.fetchword(bus);
        if self.state.timing.lock_active() {
            self.clk_visible(1);
        } else if self
            .state
            .timing
            .prefetch_wrapped_before_instruction_start()
        {
            self.clk_visible(2);
        } else {
            self.clk_forced_prefetch(bus);
        }
        self.push(bus, val);
        self.clk(tail_cycles);
    }

    fn push_imm8(&mut self, bus: &mut impl common::Bus) {
        let tail_cycles = self.stack_push_tail_cycles(3);
        let val = self.fetch(bus) as i8 as u16;
        if self.state.timing.lock_active() {
            self.clk_forced_prefetch(bus);
        } else {
            self.clk_visible(2);
        }
        self.push(bus, val);
        self.clk(tail_cycles);
    }

    fn imul_r16w_imm16(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let src;
        let imm;
        if modrm::modrm_is_register(modrm) {
            imm = self.fetchword(bus) as i16 as i32;
            src = self.regs.word(self.rm_word(modrm)) as i16 as i32;
        } else {
            self.calc_ea(modrm, bus);
            self.prepare_immediate_memory_operand(modrm);
            imm = self.fetchword(bus) as i16 as i32;
            src = self.seg_read_word(bus) as i16 as i32;
            self.state.timing.note_au_idle();
        }
        let result = src * imm;
        let reg = self.reg_word(modrm);
        self.regs.set_word(reg, result as u16);
        self.flags.carry_val = if !(-0x8000..=0x7FFF).contains(&result) {
            1
        } else {
            0
        };
        self.flags.overflow_val = self.flags.carry_val;
        self.clk_modrm_word_prefetch(bus, modrm, 24, 24, 0);
    }

    fn imul_r16w_imm8(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let src;
        let imm;
        if modrm::modrm_is_register(modrm) {
            imm = self.fetch(bus) as i8 as i32;
            src = self.regs.word(self.rm_word(modrm)) as i16 as i32;
        } else {
            self.calc_ea(modrm, bus);
            self.prepare_imul_imm8_memory_operand(modrm);
            imm = self.fetch(bus) as i8 as i32;
            src = self.seg_read_word(bus) as i16 as i32;
            self.state.timing.note_au_idle();
        }
        let result = src * imm;
        let reg = self.reg_word(modrm);
        self.regs.set_word(reg, result as u16);
        self.flags.carry_val = if !(-0x8000..=0x7FFF).contains(&result) {
            1
        } else {
            0
        };
        self.flags.overflow_val = self.flags.carry_val;
        if modrm::modrm_is_register(modrm) {
            self.clk_visible(1);
            self.clk_prefetch(bus, 23);
        } else {
            self.clk_modrm_word_prefetch(bus, modrm, 24, self.imul_imm8_memory_base_cycles(), 0);
        }
    }

    fn jcc(
        &mut self,
        bus: &mut impl common::Bus,
        condition: bool,
        timing: I286ShortBranchTimingTemplate,
    ) {
        let disp = self.fetch(bus) as i8;
        if condition {
            self.ip = self.ip.wrapping_add(disp as u16);
            self.clk_control_transfer_restart(bus, self.ip, timing.taken_control_transfer);
        } else {
            self.clk(timing.not_taken_cycles);
        }
    }

    fn test_br8(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let src = self.regs.byte(self.reg_byte(modrm));
        let dst = self.get_rm_byte(modrm, bus);
        self.alu_and_byte(dst, src);
        self.borrow_no_displacement_read_only_cycles(modrm);
        self.clk_modrm_prefetch(bus, modrm, 2, 6);
    }

    fn test_wr16(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let src = self.regs.word(self.reg_word(modrm));
        let dst = self.get_rm_word(modrm, bus);
        self.alu_and_word(dst, src);
        self.borrow_no_displacement_read_only_cycles(modrm);
        self.clk_modrm_word_prefetch(bus, modrm, 2, 6, 0);
    }

    fn test_al_imm8(&mut self, bus: &mut impl common::Bus) {
        let src = self.fetch(bus);
        let dst = self.regs.byte(ByteReg::AL);
        self.alu_and_byte(dst, src);
        self.clk_accumulator_imm8_template(bus);
    }

    fn test_aw_imm16(&mut self, bus: &mut impl common::Bus) {
        let src = self.fetchword(bus);
        let dst = self.regs.word(WordReg::AX);
        self.alu_and_word(dst, src);
        self.clk_accumulator_imm16_template(bus);
    }

    fn xchg_br8(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let reg = self.reg_byte(modrm);
        let reg_val = self.regs.byte(reg);
        let rm_val = self.get_rm_byte(modrm, bus);
        self.regs.set_byte(reg, rm_val);
        if modrm::modrm_is_register(modrm) {
            self.putback_rm_byte(modrm, reg_val, bus);
            self.clk_modrm_prefetch(bus, modrm, 3, 4);
        } else {
            self.state.timing.suppress_next_read_writeback_gap();
            self.putback_rm_byte(modrm, reg_val, bus);
            self.clk_visible(1);
        }
    }

    fn xchg_wr16(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let reg = self.reg_word(modrm);
        let reg_val = self.regs.word(reg);
        let rm_val = self.get_rm_word(modrm, bus);
        self.regs.set_word(reg, rm_val);
        if modrm::modrm_is_register(modrm) {
            self.putback_rm_word(modrm, reg_val, bus);
            self.clk_modrm_word_prefetch(bus, modrm, 3, 4, 0);
        } else {
            let write_splits = self.word_access_is_split(self.ea_seg, self.eo);
            self.state.timing.suppress_next_read_writeback_gap();
            self.putback_rm_word(modrm, reg_val, bus);
            if !write_splits {
                self.clk_visible(1);
            }
        }
    }

    fn xchg_aw(&mut self, reg: WordReg) {
        let aw = self.regs.word(WordReg::AX);
        let val = self.regs.word(reg);
        self.regs.set_word(WordReg::AX, val);
        self.regs.set_word(reg, aw);
        self.clk(3);
    }

    fn mov_br8(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let val = self.regs.byte(self.reg_byte(modrm));
        self.put_rm_byte(modrm, val, bus);
        self.clk_move_byte_template(bus, modrm, MOV_WRITE_BYTE_TIMING);
    }

    fn mov_wr16(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let val = self.regs.word(self.reg_word(modrm));
        self.put_rm_word(modrm, val, bus);
        self.clk_move_word_template(bus, modrm, MOV_WRITE_WORD_TIMING);
    }

    fn mov_r8b(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let val = self.get_rm_byte(modrm, bus);
        let reg = self.reg_byte(modrm);
        self.regs.set_byte(reg, val);
        self.clk_move_byte_template(bus, modrm, MOV_READ_BYTE_TIMING);
    }

    fn mov_r16w(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let val = self.get_rm_word(modrm, bus);
        let reg = self.reg_word(modrm);
        self.regs.set_word(reg, val);
        self.clk_move_word_template(bus, modrm, MOV_READ_WORD_TIMING);
    }

    fn mov_rm_sreg(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let seg = SegReg16::from_index((modrm >> 3) & 3);
        let val = self.sregs[seg as usize];
        self.put_rm_word(modrm, val, bus);
        self.clk_move_word_template(bus, modrm, MOV_WRITE_WORD_TIMING);
    }

    fn mov_sreg_rm(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let val = self.get_rm_word(modrm, bus);
        let seg = SegReg16::from_index((modrm >> 3) & 3);
        if !self.load_segment(seg, val, bus) {
            return;
        }
        if seg == SegReg16::SS {
            self.inhibit_all = 1;
        }
        self.clk_move_word_template(bus, modrm, MOV_READ_WORD_TIMING);
    }

    fn lea(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        self.calc_ea(modrm, bus);
        let reg = self.reg_word(modrm);
        let val = self.eo;
        self.regs.set_word(reg, val);
        if modrm::modrm_is_disp8_memory(modrm) {
            let au_cycles = modrm::ea_class_au_cycles(self.ea_class);
            self.clk_visible(1);
            self.clk_prefetch(bus, 5 + au_cycles);
        } else if modrm::modrm_is_disp16_memory(modrm) || modrm::modrm_is_direct_memory(modrm) {
            let au_cycles = modrm::ea_class_au_cycles(self.ea_class);
            self.clk_prefetch(bus, 6 + au_cycles);
        } else {
            self.clk(3);
        }
    }

    fn pop_rm(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let sp_pen = self.sp_penalty(1);
        let val = self.pop(bus);
        self.put_rm_word(modrm, val, bus);
        if modrm::modrm_is_register(modrm) {
            self.clk(5 + sp_pen);
        } else {
            let ea_pen = if address_is_odd(self.ea) { 4 } else { 0 };
            self.clk(5 + sp_pen + ea_pen);
        }
    }

    fn cbw(&mut self) {
        let al = self.regs.byte(ByteReg::AL) as i8 as i16 as u16;
        self.regs.set_word(WordReg::AX, al);
        self.clk(2);
    }

    fn cwd(&mut self) {
        let aw = self.regs.word(WordReg::AX) as i16;
        self.regs
            .set_word(WordReg::DX, if aw < 0 { 0xFFFF } else { 0 });
        self.clk(2);
    }

    fn call_far(&mut self, bus: &mut impl common::Bus) {
        self.finish_state = I286FinishState::ControlTransferRestart;
        let penalty = self.sp_penalty(2);
        let offset = self.fetchword(bus);
        let segment = self.fetchword(bus);
        let cs = self.sregs[SegReg16::CS as usize];
        let ip = self.ip;
        if self.is_protected_mode() {
            self.code_descriptor(segment, offset, super::TaskType::Call, cs, ip, bus);
        } else {
            let prefix_count = self.state.timing.prefix_count();
            let suppress_fallthrough_prefetch = self
                .state
                .timing
                .lock_prefix_suppresses_fallthrough_prefetch();
            if !suppress_fallthrough_prefetch {
                self.clk_forced_prefetch(bus);
            }
            if prefix_count & 1 == 1 {
                self.clk_visible(3);
            } else {
                self.clk_visible(4);
            }
            self.push(bus, cs);
            self.clk_visible(2);
            if !self.load_segment(SegReg16::CS, segment, bus) {
                return;
            }
            self.ip = offset;
            self.state.timing.arm_control_transfer_restart(self.ip);
            let code_segment_base = self.seg_bases[SegReg16::CS as usize];
            self.state
                .timing
                .advance_control_transfer_fetches(bus, code_segment_base, 1);
            self.sync_timing_cycles();
            self.push(bus, ip);
            self.state
                .timing
                .advance_control_transfer_fetches(bus, code_segment_base, 1);
            self.state.timing.complete_control_transfer_restart(2);
            self.sync_timing_cycles();
            return;
        }
        self.clk(13 + penalty);
    }

    fn call_near(&mut self, bus: &mut impl common::Bus) {
        self.finish_state = I286FinishState::ControlTransferRestart;
        let disp = self.fetchword(bus);
        let return_instruction_pointer = self.ip;
        self.ip = self.ip.wrapping_add(disp);

        self.state.timing.arm_control_transfer_restart(self.ip);
        let code_segment_base = self.seg_bases[SegReg16::CS as usize];
        let initial_cycles = if self.state.timing.lock_active() {
            1
        } else {
            2
        };
        self.state
            .timing
            .advance_control_transfer_internal_cycles(initial_cycles);
        self.state
            .timing
            .advance_control_transfer_fetches(bus, code_segment_base, 1);

        let stack_write_even = self.regs.word(WordReg::SP) & 1 == 0;
        self.push(bus, return_instruction_pointer);

        if stack_write_even {
            self.state
                .timing
                .advance_control_transfer_fetches(bus, code_segment_base, 1);
        }
        self.state.timing.complete_control_transfer_restart(2);
        self.sync_timing_cycles();
    }

    fn jmp_near(&mut self, bus: &mut impl common::Bus) {
        let disp = self.fetchword(bus);
        self.ip = self.ip.wrapping_add(disp);
        self.clk_control_transfer_restart(bus, self.ip, NEAR_CONTROL_TRANSFER_TIMING);
    }

    fn jmp_far(&mut self, bus: &mut impl common::Bus) {
        let offset = self.fetchword(bus);
        let segment = self.fetchword(bus);
        if self.is_protected_mode() {
            self.finish_state = I286FinishState::ControlTransferRestart;
            self.code_descriptor(segment, offset, super::TaskType::Jmp, 0, 0, bus);
        } else {
            let prefix_count = self.state.timing.prefix_count();
            let suppress_fallthrough_prefetch = self
                .state
                .timing
                .lock_prefix_suppresses_fallthrough_prefetch()
                && prefix_count & 1 == 0;
            if !suppress_fallthrough_prefetch {
                if self
                    .state
                    .timing
                    .prefetch_wrapped_before_instruction_start()
                {
                    self.clk_prefetch(bus, 2);
                } else {
                    self.clk_forced_prefetch(bus);
                }
            }
            if !self.load_segment(SegReg16::CS, segment, bus) {
                return;
            }
            self.ip = offset;
            let timing =
                if self.state.timing.lock_active() && prefix_count > 1 && prefix_count & 1 == 0 {
                    LOCK_PREFIXED_FAR_IMMEDIATE_CONTROL_TRANSFER_TIMING
                } else if !self.state.timing.lock_active() && prefix_count & 1 == 1 {
                    PREFIXED_FAR_IMMEDIATE_CONTROL_TRANSFER_TIMING
                } else {
                    FAR_IMMEDIATE_CONTROL_TRANSFER_TIMING
                };
            self.clk_control_transfer_restart(bus, self.ip, timing);
            return;
        }
        self.clk(11);
    }

    fn jmp_short(&mut self, bus: &mut impl common::Bus) {
        let disp = self.fetch(bus) as i8 as u16;
        self.ip = self.ip.wrapping_add(disp);
        let timing = if self.state.timing.leading_lock_prefix() {
            LOCK_SHORT_JMP_CONTROL_TRANSFER_TIMING
        } else {
            NEAR_CONTROL_TRANSFER_TIMING
        };
        self.clk_control_transfer_restart(bus, self.ip, timing);
    }

    fn ret_near(&mut self, bus: &mut impl common::Bus) {
        self.finish_state = I286FinishState::ControlTransferRestart;
        self.ip = self.pop(bus);

        self.state.timing.arm_control_transfer_restart(self.ip);
        let code_segment_base = self.seg_bases[SegReg16::CS as usize];
        self.state
            .timing
            .advance_control_transfer_internal_cycles(3);
        self.state
            .timing
            .advance_control_transfer_fetches(bus, code_segment_base, 3);
        self.state.timing.complete_control_transfer_restart(2);
        self.sync_timing_cycles();
    }

    fn ret_near_imm(&mut self, bus: &mut impl common::Bus) {
        self.finish_state = I286FinishState::ControlTransferRestart;
        let imm = self.fetchword(bus);
        if self.state.timing.lock_active() {
            self.clk_visible(1);
        } else {
            self.clk_visible(2);
        }
        self.ip = self.pop(bus);
        let sp = self.regs.word(WordReg::SP).wrapping_add(imm);
        self.regs.set_word(WordReg::SP, sp);
        self.state.timing.arm_control_transfer_restart(self.ip);
        let code_segment_base = self.seg_bases[SegReg16::CS as usize];
        self.state
            .timing
            .advance_control_transfer_internal_cycles(3);
        self.state
            .timing
            .advance_control_transfer_fetches(bus, code_segment_base, 3);
        self.state.timing.complete_control_transfer_restart(2);
        self.sync_timing_cycles();
    }

    fn ret_far(&mut self, bus: &mut impl common::Bus) {
        self.finish_state = I286FinishState::ControlTransferRestart;
        let penalty = self.sp_penalty(2);

        if !self.is_protected_mode() {
            if !self.state.timing.lock_active() {
                self.clk_visible(2);
            }
            self.ip = self.pop(bus);
            let cs = self.pop(bus);
            if !self.load_segment(SegReg16::CS, cs, bus) {
                return;
            }
            self.state.timing.arm_control_transfer_restart(self.ip);
            let code_segment_base = self.seg_bases[SegReg16::CS as usize];
            self.state
                .timing
                .advance_control_transfer_internal_cycles(4);
            self.state
                .timing
                .advance_control_transfer_fetches(bus, code_segment_base, 3);
            self.state.timing.complete_control_transfer_restart(2);
            self.sync_timing_cycles();
            return;
        }

        // Read new IP and CS from stack without modifying SP yet.
        let sp = self.regs.word(WordReg::SP);
        let ss_base = self.seg_base(SegReg16::SS);
        let new_ip = bus.read_byte(ss_base.wrapping_add(sp as u32) & ADDRESS_MASK) as u16
            | ((bus.read_byte(ss_base.wrapping_add(sp.wrapping_add(1) as u32) & ADDRESS_MASK)
                as u16)
                << 8);
        let new_cs = bus.read_byte(ss_base.wrapping_add(sp.wrapping_add(2) as u32) & ADDRESS_MASK)
            as u16
            | ((bus.read_byte(ss_base.wrapping_add(sp.wrapping_add(3) as u32) & ADDRESS_MASK)
                as u16)
                << 8);

        let new_rpl = new_cs & 3;
        let old_cpl = self.cpl();

        if new_rpl > old_cpl {
            // Inter-privilege return: also read new SP and SS.
            let new_sp = bus
                .read_byte(ss_base.wrapping_add(sp.wrapping_add(4) as u32) & ADDRESS_MASK)
                as u16
                | ((bus.read_byte(ss_base.wrapping_add(sp.wrapping_add(5) as u32) & ADDRESS_MASK)
                    as u16)
                    << 8);
            let new_ss = bus
                .read_byte(ss_base.wrapping_add(sp.wrapping_add(6) as u32) & ADDRESS_MASK)
                as u16
                | ((bus.read_byte(ss_base.wrapping_add(sp.wrapping_add(7) as u32) & ADDRESS_MASK)
                    as u16)
                    << 8);

            if !self.load_cs_for_return(new_cs, new_ip, bus) {
                return;
            }
            self.ip = new_ip;

            if !self.load_segment(SegReg16::SS, new_ss, bus) {
                return;
            }
            self.regs.set_word(WordReg::SP, new_sp);

            let new_cpl = self.cpl();
            self.invalidate_segment_if_needed(SegReg16::DS, new_cpl);
            self.invalidate_segment_if_needed(SegReg16::ES, new_cpl);
        } else {
            // Same-privilege return.
            self.regs.set_word(WordReg::SP, sp.wrapping_add(4));
            if !self.load_cs_for_return(new_cs, new_ip, bus) {
                return;
            }
            self.ip = new_ip;
        }

        self.clk(15 + penalty);
    }

    fn ret_far_imm(&mut self, bus: &mut impl common::Bus) {
        self.finish_state = I286FinishState::ControlTransferRestart;
        let penalty = self.sp_penalty(2);
        let imm = self.fetchword(bus);

        if !self.is_protected_mode() {
            if self.state.timing.lock_active() {
                self.clk_visible(1);
            } else {
                self.clk_visible(2);
            }
            self.ip = self.pop(bus);
            let cs = self.pop(bus);
            if !self.load_segment(SegReg16::CS, cs, bus) {
                return;
            }
            let sp = self.regs.word(WordReg::SP).wrapping_add(imm);
            self.regs.set_word(WordReg::SP, sp);
            self.state.timing.arm_control_transfer_restart(self.ip);
            let code_segment_base = self.seg_bases[SegReg16::CS as usize];
            self.state
                .timing
                .advance_control_transfer_internal_cycles(4);
            self.state
                .timing
                .advance_control_transfer_fetches(bus, code_segment_base, 3);
            self.state.timing.complete_control_transfer_restart(2);
            self.sync_timing_cycles();
            return;
        }

        let sp = self.regs.word(WordReg::SP);
        let ss_base = self.seg_base(SegReg16::SS);
        let new_ip = bus.read_byte(ss_base.wrapping_add(sp as u32) & ADDRESS_MASK) as u16
            | ((bus.read_byte(ss_base.wrapping_add(sp.wrapping_add(1) as u32) & ADDRESS_MASK)
                as u16)
                << 8);
        let new_cs = bus.read_byte(ss_base.wrapping_add(sp.wrapping_add(2) as u32) & ADDRESS_MASK)
            as u16
            | ((bus.read_byte(ss_base.wrapping_add(sp.wrapping_add(3) as u32) & ADDRESS_MASK)
                as u16)
                << 8);

        let new_rpl = new_cs & 3;
        let old_cpl = self.cpl();

        if new_rpl > old_cpl {
            let sp_ss_base = sp.wrapping_add(4).wrapping_add(imm);
            let new_sp = bus.read_byte(ss_base.wrapping_add(sp_ss_base as u32) & ADDRESS_MASK)
                as u16
                | ((bus.read_byte(
                    ss_base.wrapping_add(sp_ss_base.wrapping_add(1) as u32) & ADDRESS_MASK,
                ) as u16)
                    << 8);
            let new_ss = bus
                .read_byte(ss_base.wrapping_add(sp_ss_base.wrapping_add(2) as u32) & ADDRESS_MASK)
                as u16
                | ((bus.read_byte(
                    ss_base.wrapping_add(sp_ss_base.wrapping_add(3) as u32) & ADDRESS_MASK,
                ) as u16)
                    << 8);

            if !self.load_cs_for_return(new_cs, new_ip, bus) {
                return;
            }
            self.ip = new_ip;

            if !self.load_segment(SegReg16::SS, new_ss, bus) {
                return;
            }
            self.regs.set_word(WordReg::SP, new_sp.wrapping_add(imm));

            let new_cpl = self.cpl();
            self.invalidate_segment_if_needed(SegReg16::DS, new_cpl);
            self.invalidate_segment_if_needed(SegReg16::ES, new_cpl);
        } else {
            self.regs
                .set_word(WordReg::SP, sp.wrapping_add(4).wrapping_add(imm));
            if !self.load_cs_for_return(new_cs, new_ip, bus) {
                return;
            }
            self.ip = new_ip;
        }

        self.clk(15 + penalty);
    }

    fn pushf(&mut self, bus: &mut impl common::Bus) {
        let tail_cycles = self.stack_push_tail_cycles(3);
        let mut flags_val = self.flags.compress();
        if !self.is_protected_mode() {
            flags_val &= !0xF000;
        }
        self.push(bus, flags_val);
        self.clk(tail_cycles);
    }

    fn popf(&mut self, bus: &mut impl common::Bus) {
        let val = self.pop(bus);
        let cpl = self.cpl();
        let pm = self.is_protected_mode();
        self.flags.load_flags(val, cpl, pm);
        self.clk(5);
    }

    fn sahf(&mut self) {
        let ah = self.regs.byte(ByteReg::AH);
        self.flags.carry_val = (ah & 0x01) as u32;
        self.flags.parity_val = if ah & 0x04 != 0 { 0 } else { 1 };
        self.flags.aux_val = (ah & 0x10) as u32;
        self.flags.zero_val = if ah & 0x40 != 0 { 0 } else { 1 };
        self.flags.sign_val = if ah & 0x80 != 0 { -1 } else { 0 };
        self.clk(2);
    }

    fn lahf(&mut self) {
        let flags_val = self.flags.compress() as u8;
        self.regs.set_byte(ByteReg::AH, flags_val);
        self.clk(2);
    }

    fn mov_al_moffs(&mut self, bus: &mut impl common::Bus) {
        let offset = self.fetchword(bus);
        let seg = self.default_seg(SegReg16::DS);
        self.prepare_moffs_access(offset);
        let val = self.read_byte_seg(bus, seg, offset);
        self.regs.set_byte(ByteReg::AL, val);
        self.clk(5);
    }

    fn mov_aw_moffs(&mut self, bus: &mut impl common::Bus) {
        let offset = self.fetchword(bus);
        self.ea_seg = self.default_seg(SegReg16::DS);
        let base = self.seg_base(self.ea_seg);
        self.eo = offset;
        self.ea = base.wrapping_add(offset as u32) & ADDRESS_MASK;
        self.prepare_moffs_access(offset);
        let val = self.seg_read_word(bus);
        self.regs.set_word(WordReg::AX, val);
        self.clk(5);
    }

    fn mov_moffs_al(&mut self, bus: &mut impl common::Bus) {
        let offset = self.fetchword(bus);
        let seg = self.default_seg(SegReg16::DS);
        let val = self.regs.byte(ByteReg::AL);
        self.prepare_moffs_access(offset);
        self.write_byte_seg(bus, seg, offset, val);
        self.clk(3);
    }

    fn mov_moffs_aw(&mut self, bus: &mut impl common::Bus) {
        let offset = self.fetchword(bus);
        self.ea_seg = self.default_seg(SegReg16::DS);
        let base = self.seg_base(self.ea_seg);
        self.eo = offset;
        self.ea = base.wrapping_add(offset as u32) & ADDRESS_MASK;
        self.prepare_moffs_access(offset);
        self.seg_write_word(bus, self.regs.word(WordReg::AX));
        let tail_cycles = if address_is_odd(self.ea) { 2 } else { 3 };
        self.clk(tail_cycles);
    }

    fn prepare_moffs_access(&mut self, _offset: u16) {
        match self.state.timing.prefix_count() {
            0 => {
                self.state.timing.note_au_ready();
                self.state
                    .timing
                    .note_demand_prefetch_policy(I286DemandPrefetchPolicy::BeforeNoTurnaround);
            }
            count
                if self
                    .state
                    .timing
                    .lock_prefix_suppresses_fallthrough_prefetch()
                    && count & 1 == 0 =>
            {
                self.state
                    .timing
                    .borrow_internal_cycles(MOFFS_PREFIX_OVERLAP_CREDIT);
            }
            count if count & 1 == 1 => {
                self.clk_visible(1);
                self.state
                    .timing
                    .borrow_internal_cycles(MOFFS_PREFIX_OVERLAP_CREDIT);
            }
            _ => {
                self.state.timing.note_au_ready();
                self.state
                    .timing
                    .note_demand_prefetch_policy(I286DemandPrefetchPolicy::BeforeNoTurnaround);
            }
        }
    }

    fn mov_byte_reg_imm(&mut self, reg: ByteReg, bus: &mut impl common::Bus) {
        let val = self.fetch(bus);
        self.regs.set_byte(reg, val);
        self.clk_prefetch(bus, 3);
    }

    fn mov_word_reg_imm(&mut self, reg: WordReg, bus: &mut impl common::Bus) {
        let val = self.fetchword(bus);
        self.regs.set_word(reg, val);
        self.clk_prefetch(bus, 4);
    }

    fn mov_rm_imm8(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        if modrm::modrm_is_register(modrm) {
            let val = self.fetch(bus);
            let reg = self.rm_byte(modrm);
            self.regs.set_byte(reg, val);
        } else {
            self.calc_ea(modrm, bus);
            let val = self.fetch(bus);
            self.seg_write_byte_at(bus, 0, val);
        }
        self.clk_modrm_prefetch(bus, modrm, 2, 3);
    }

    fn mov_rm_imm16(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        if modrm::modrm_is_register(modrm) {
            let val = self.fetchword(bus);
            let reg = self.rm_word(modrm);
            self.regs.set_word(reg, val);
        } else {
            self.calc_ea(modrm, bus);
            let val = self.fetchword(bus);
            self.seg_write_word(bus, val);
        }
        self.clk_modrm_word_prefetch(bus, modrm, 2, 3, 1);
    }

    fn les(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        self.calc_ea(modrm, bus);
        let offset = self.seg_read_word(bus);
        self.state.timing.suppress_next_memory_read_window();
        let segment = self.seg_read_word_at(bus, 2);
        let reg = self.reg_word(modrm);
        self.regs.set_word(reg, offset);
        if !self.load_segment(SegReg16::ES, segment, bus) {
            return;
        }
        self.clk_visible(3);
    }

    fn lds(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        self.calc_ea(modrm, bus);
        let offset = self.seg_read_word(bus);
        self.state.timing.suppress_next_memory_read_window();
        let segment = self.seg_read_word_at(bus, 2);
        let reg = self.reg_word(modrm);
        self.regs.set_word(reg, offset);
        if !self.load_segment(SegReg16::DS, segment, bus) {
            return;
        }
        self.clk_visible(3);
    }

    fn enter(&mut self, bus: &mut impl common::Bus) {
        let alloc = self.fetchword(bus);
        let level = self.fetch(bus) & 0x1F;
        let bp = self.regs.word(WordReg::BP);
        let stack_write_split =
            self.word_access_is_split(SegReg16::SS, self.regs.word(WordReg::SP).wrapping_sub(2));
        let frame_read_split = level > 1
            && self.word_access_is_split(SegReg16::SS, self.regs.word(WordReg::BP).wrapping_sub(2));
        self.clk_prefetch(bus, 4);
        self.push(bus, bp);
        let frame_ptr = self.regs.word(WordReg::SP);
        if level > 0 {
            if stack_write_split {
                self.clk(2);
            } else {
                self.clk(4);
            }
            for _ in 1..level {
                let bp_val = self.regs.word(WordReg::BP).wrapping_sub(2);
                self.regs.set_word(WordReg::BP, bp_val);
                let val = self.read_word_seg(bus, SegReg16::SS, bp_val);
                self.state.timing.suppress_next_read_writeback_gap();
                self.push(bus, val);
            }
            if level > 1 {
                self.state.timing.advance_visible_internal_cycles(2);
                self.sync_timing_cycles();
            }
            self.push(bus, frame_ptr);
        }
        self.regs.set_word(WordReg::BP, frame_ptr);
        let sp = self.regs.word(WordReg::SP).wrapping_sub(alloc);
        self.regs.set_word(WordReg::SP, sp);
        if level == 0 {
            self.clk(9);
        } else if level == 1 {
            self.clk(8);
        } else {
            let tail_cycles = if frame_read_split { 6 } else { 4 };
            self.clk(2 * i32::from(level) + tail_cycles);
        }
    }

    fn leave(&mut self, bus: &mut impl common::Bus) {
        let bp = self.regs.word(WordReg::BP);
        self.regs.set_word(WordReg::SP, bp);
        let val = self.pop(bus);
        self.regs.set_word(WordReg::BP, val);
        self.clk(5);
    }

    fn int3(&mut self, bus: &mut impl common::Bus) {
        self.finish_state = I286FinishState::FaultRestart;
        let penalty = self.sp_penalty(3);
        self.raise_software_interrupt(3, bus);
        self.clk(23 + penalty);
    }

    fn int_imm(&mut self, bus: &mut impl common::Bus) {
        self.finish_state = I286FinishState::FaultRestart;
        let penalty = self.sp_penalty(3);
        let vector = self.fetch(bus);
        self.raise_software_interrupt(vector, bus);
        self.clk(23 + penalty);
    }

    fn into(&mut self, bus: &mut impl common::Bus) {
        if self.flags.of() {
            self.finish_state = I286FinishState::FaultRestart;
            let penalty = self.sp_penalty(3);
            self.raise_software_interrupt(4, bus);
            self.clk(24 + penalty);
        } else {
            self.clk(3);
        }
    }

    fn iret(&mut self, bus: &mut impl common::Bus) {
        self.finish_state = I286FinishState::ControlTransferRestart;
        let penalty = self.sp_penalty(3);

        if !self.is_protected_mode() {
            let sp = self.regs.word(WordReg::SP);
            let startup_cycles = if self.state.timing.lock_active() {
                1
            } else {
                3
            };
            self.clk_visible(startup_cycles);
            let flags_val = self.read_word_seg(bus, SegReg16::SS, sp.wrapping_add(4));
            let instruction_pointer = self.read_word_seg(bus, SegReg16::SS, sp);
            let cs = self.read_word_seg(bus, SegReg16::SS, sp.wrapping_add(2));
            self.regs.set_word(WordReg::SP, sp.wrapping_add(6));
            if !self.load_segment(SegReg16::CS, cs, bus) {
                return;
            }
            self.ip = instruction_pointer;
            self.flags.load_flags(flags_val, 0, false);
            let timing = if self.state.timing.lock_active() {
                LOCK_IRET_CONTROL_TRANSFER_TIMING
            } else {
                IRET_CONTROL_TRANSFER_TIMING
            };
            self.clk_control_transfer_restart(bus, self.ip, timing);
            return;
        }

        if self.flags.nt {
            let backlink = bus.read_byte(self.tr_base & ADDRESS_MASK) as u16
                | ((bus.read_byte(self.tr_base.wrapping_add(1) & ADDRESS_MASK) as u16) << 8);
            self.switch_task(backlink, super::TaskType::Iret, bus);
            let flags_val = self.flags.compress();
            let cpl = self.cpl();
            self.flags.load_flags(flags_val, cpl, true);
            self.clk(17 + penalty);
            return;
        }

        let old_cpl = self.cpl();

        // Read values from stack without modifying SP yet.
        let sp = self.regs.word(WordReg::SP);
        let ss_base = self.seg_base(SegReg16::SS);
        let new_ip = bus.read_byte(ss_base.wrapping_add(sp as u32) & ADDRESS_MASK) as u16
            | ((bus.read_byte(ss_base.wrapping_add(sp.wrapping_add(1) as u32) & ADDRESS_MASK)
                as u16)
                << 8);
        let new_cs = bus.read_byte(ss_base.wrapping_add(sp.wrapping_add(2) as u32) & ADDRESS_MASK)
            as u16
            | ((bus.read_byte(ss_base.wrapping_add(sp.wrapping_add(3) as u32) & ADDRESS_MASK)
                as u16)
                << 8);
        let new_flags =
            bus.read_byte(ss_base.wrapping_add(sp.wrapping_add(4) as u32) & ADDRESS_MASK) as u16
                | ((bus.read_byte(ss_base.wrapping_add(sp.wrapping_add(5) as u32) & ADDRESS_MASK)
                    as u16)
                    << 8);

        let new_rpl = new_cs & 3;

        if new_rpl > old_cpl {
            // Inter-privilege return: also read SP and SS from stack.
            let new_sp = bus
                .read_byte(ss_base.wrapping_add(sp.wrapping_add(6) as u32) & ADDRESS_MASK)
                as u16
                | ((bus.read_byte(ss_base.wrapping_add(sp.wrapping_add(7) as u32) & ADDRESS_MASK)
                    as u16)
                    << 8);
            let new_ss = bus
                .read_byte(ss_base.wrapping_add(sp.wrapping_add(8) as u32) & ADDRESS_MASK)
                as u16
                | ((bus.read_byte(ss_base.wrapping_add(sp.wrapping_add(9) as u32) & ADDRESS_MASK)
                    as u16)
                    << 8);

            if !self.load_cs_for_return(new_cs, new_ip, bus) {
                return;
            }
            self.ip = new_ip;
            self.flags.load_flags(new_flags, old_cpl, true);

            if !self.load_segment(SegReg16::SS, new_ss, bus) {
                return;
            }
            self.regs.set_word(WordReg::SP, new_sp);

            let new_cpl = self.cpl();
            self.invalidate_segment_if_needed(SegReg16::DS, new_cpl);
            self.invalidate_segment_if_needed(SegReg16::ES, new_cpl);
        } else {
            // Same-privilege return.
            self.regs.set_word(WordReg::SP, sp.wrapping_add(6));
            if !self.load_cs_for_return(new_cs, new_ip, bus) {
                return;
            }
            self.ip = new_ip;
            self.flags.load_flags(new_flags, old_cpl, true);
        }

        self.clk(17 + penalty);
    }

    fn loopne(&mut self, bus: &mut impl common::Bus) {
        let disp = self.fetch(bus) as i8 as u16;
        let cw = self.regs.word(WordReg::CX).wrapping_sub(1);
        self.regs.set_word(WordReg::CX, cw);
        if cw != 0 && !self.flags.zf() {
            self.ip = self.ip.wrapping_add(disp);
            let timing = if self.state.timing.cycle_state().lock_active {
                LOCK_LOOP_CONTROL_TRANSFER_TIMING
            } else {
                LOOP_CONTROL_TRANSFER_TIMING
            };
            self.clk_control_transfer_restart(bus, self.ip, timing);
        } else {
            self.clk(6);
        }
    }

    fn loope(&mut self, bus: &mut impl common::Bus) {
        let disp = self.fetch(bus) as i8 as u16;
        let cw = self.regs.word(WordReg::CX).wrapping_sub(1);
        self.regs.set_word(WordReg::CX, cw);
        if cw != 0 && self.flags.zf() {
            self.ip = self.ip.wrapping_add(disp);
            let timing = if self.state.timing.cycle_state().lock_active {
                LOCK_LOOP_CONTROL_TRANSFER_TIMING
            } else {
                LOOP_CONTROL_TRANSFER_TIMING
            };
            self.clk_control_transfer_restart(bus, self.ip, timing);
        } else {
            self.clk(6);
        }
    }

    fn loop_(&mut self, bus: &mut impl common::Bus) {
        let disp = self.fetch(bus) as i8 as u16;
        let cw = self.regs.word(WordReg::CX).wrapping_sub(1);
        self.regs.set_word(WordReg::CX, cw);
        if cw != 0 {
            self.ip = self.ip.wrapping_add(disp);
            let timing = if self.state.timing.cycle_state().lock_active {
                LOCK_LOOP_CONTROL_TRANSFER_TIMING
            } else {
                LOOP_CONTROL_TRANSFER_TIMING
            };
            self.clk_control_transfer_restart(bus, self.ip, timing);
        } else {
            self.clk(6);
        }
    }

    fn jcxz(&mut self, bus: &mut impl common::Bus) {
        let disp = self.fetch(bus) as i8 as u16;
        if self.regs.word(WordReg::CX) == 0 {
            let target = self.ip.wrapping_add(disp);
            let lock_active = self.state.timing.cycle_state().lock_active;
            if lock_active {
                self.clk_prefetch(bus, 3);
            }
            self.ip = target;
            let timing = if lock_active {
                LOCK_JCXZ_CONTROL_TRANSFER_TIMING
            } else {
                LOOP_CONTROL_TRANSFER_TIMING
            };
            self.clk_control_transfer_restart(bus, self.ip, timing);
        } else if self.state.timing.cycle_state().lock_active {
            self.clk_prefetch(bus, 6);
        } else {
            self.clk(6);
        }
    }

    fn in_al_imm(&mut self, bus: &mut impl common::Bus) {
        let port = self.fetch(bus) as u16;
        if port != 0 && !self.state.timing.lock_active() {
            self.clk_visible(1);
        }
        let val = self.read_io_byte(bus, port);
        self.regs.set_byte(ByteReg::AL, val);
        self.clk(3);
    }

    fn in_aw_imm(&mut self, bus: &mut impl common::Bus) {
        let port = self.fetch(bus) as u16;
        if port != 0 && !self.state.timing.lock_active() {
            self.clk_visible(1);
        }
        let val = self.read_io_word(bus, port);
        self.regs.set_word(WordReg::AX, val);
        self.clk(3);
    }

    fn out_imm_al(&mut self, bus: &mut impl common::Bus) {
        let port = self.fetch(bus) as u16;
        let val = self.regs.byte(ByteReg::AL);
        if port != 0 && !self.state.timing.lock_active() {
            self.clk_visible(1);
        }
        self.write_io_byte(bus, port, val);
        self.clk(1);
    }

    fn out_imm_aw(&mut self, bus: &mut impl common::Bus) {
        let port = self.fetch(bus) as u16;
        let val = self.regs.word(WordReg::AX);
        if !self.state.timing.lock_active() {
            self.clk_visible(1);
        }
        self.write_io_word(bus, port, val);
        let tail_cycles = if port & 1 == 0 { 1 } else { 0 };
        self.clk(tail_cycles);
    }

    fn in_al_dw(&mut self, bus: &mut impl common::Bus) {
        let port = self.regs.word(WordReg::DX);
        let val = self.read_io_byte(bus, port);
        self.regs.set_byte(ByteReg::AL, val);
        self.clk(5);
    }

    fn in_aw_dw(&mut self, bus: &mut impl common::Bus) {
        let port = self.regs.word(WordReg::DX);
        let val = self.read_io_word(bus, port);
        self.regs.set_word(WordReg::AX, val);
        self.clk(5);
    }

    fn out_dw_al(&mut self, bus: &mut impl common::Bus) {
        let port = self.regs.word(WordReg::DX);
        let val = self.regs.byte(ByteReg::AL);
        self.write_io_byte(bus, port, val);
        let tail_cycles = if port == 0 { 0 } else { 1 };
        self.clk(tail_cycles);
    }

    fn out_dw_aw(&mut self, bus: &mut impl common::Bus) {
        let port = self.regs.word(WordReg::DX);
        let val = self.regs.word(WordReg::AX);
        self.write_io_word(bus, port, val);
        let tail_cycles = if port & 1 == 0 { 1 } else { 0 };
        self.clk(tail_cycles);
    }

    fn xlat(&mut self, bus: &mut impl common::Bus) {
        let al = self.regs.byte(ByteReg::AL) as u16;
        let bw = self.regs.word(WordReg::BX);
        let offset = bw.wrapping_add(al);
        let seg = self.default_seg(SegReg16::DS);
        let val = self.read_byte_seg(bus, seg, offset);
        self.regs.set_byte(ByteReg::AL, val);
        self.clk(5);
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
        self.clk(3);
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
        self.clk(3);
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
        self.clk(3);
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
        self.clk(3);
    }

    fn aam(&mut self, bus: &mut impl common::Bus) {
        let base = self.fetch(bus);
        let cycles = if self.state.timing.cycle_state().lock_active {
            16
        } else {
            17
        };
        if base == 0 {
            self.regs.set_byte(ByteReg::AH, 0xFF);
            let val = self.regs.byte(ByteReg::AL) as u32;
            self.flags.set_szpf_byte(val);
            self.clk(cycles);
            return;
        }
        let al = self.regs.byte(ByteReg::AL);
        self.regs.set_byte(ByteReg::AH, al / base);
        self.regs.set_byte(ByteReg::AL, al % base);
        let val = self.regs.byte(ByteReg::AL) as u32;
        self.flags.set_szpf_byte(val);
        self.clk(cycles);
    }

    fn aad(&mut self, bus: &mut impl common::Bus) {
        let base = self.fetch(bus);
        let al = self.regs.byte(ByteReg::AL);
        let ah = self.regs.byte(ByteReg::AH);
        let result = al.wrapping_add(ah.wrapping_mul(base));
        self.regs.set_byte(ByteReg::AL, result);
        self.regs.set_byte(ByteReg::AH, 0);
        self.flags.set_szpf_byte(result as u32);
        let cycles = if self.state.timing.cycle_state().lock_active {
            14
        } else {
            15
        };
        self.clk(cycles);
    }

    fn salc(&mut self) {
        let carry = self.flags.cf();
        let val = if carry { 0xFF } else { 0x00 };
        self.regs.set_byte(ByteReg::AL, val);
        let cycles = if carry { 3 } else { 4 };
        self.clk(cycles);
    }

    fn fpu_escape(&mut self, bus: &mut impl common::Bus) {
        let instruction_pointer = self.prev_ip;
        let code_segment = self.sregs[SegReg16::CS as usize];
        let modrm = self.fetch(bus);
        let mut operand_pointer = None;
        if modrm::modrm_is_memory(modrm) {
            self.calc_ea(modrm, bus);
            operand_pointer = Some((self.eo, self.sregs[self.ea_seg as usize]));
        }

        let (mut pre_io_cycles, mut prefetch_lead_cycles): (u8, Option<u8>) = match self.ea_class {
            EaClass::Register => (6, None),
            EaClass::SingleRegister | EaClass::DoubleRegister => (14, None),
            EaClass::Direct | EaClass::Disp16Single => (16, Some(0)),
            EaClass::Disp16Double => (17, Some(0)),
            EaClass::Disp8Single => (16, Some(1)),
            EaClass::Disp8Double => (17, Some(1)),
        };
        if self.state.timing.prefix_count_is_odd() {
            pre_io_cycles = pre_io_cycles.saturating_sub(1);
            match self.ea_class {
                EaClass::Disp8Single | EaClass::Disp8Double => {
                    prefetch_lead_cycles = None;
                }
                EaClass::Register
                | EaClass::Direct
                | EaClass::SingleRegister
                | EaClass::DoubleRegister
                | EaClass::Disp16Single
                | EaClass::Disp16Double => {}
            }
        }
        if self.ea_class.is_register()
            && self
                .state
                .timing
                .lock_prefix_suppresses_fallthrough_prefetch()
        {
            pre_io_cycles = pre_io_cycles.saturating_sub(1);
        }
        if self.ea_class.is_memory()
            && self
                .state
                .timing
                .lock_prefix_suppresses_fallthrough_prefetch()
            && self.state.timing.prefix_count() == 2
        {
            pre_io_cycles = pre_io_cycles.saturating_sub(2);
            prefetch_lead_cycles = None;
        }
        let code_segment_base = self.seg_bases[SegReg16::CS as usize];
        self.state.timing.advance_fpu_escape(
            bus,
            code_segment_base,
            I286FpuEscapeTiming {
                pre_io_cycles,
                prefetch_lead_cycles,
                instruction_pointer,
                code_segment,
                operand_pointer,
            },
        );

        self.sync_timing_cycles();
    }

    fn clc(&mut self) {
        self.flags.carry_val = 0;
        self.clk(2);
    }

    fn stc(&mut self) {
        self.flags.carry_val = 1;
        self.clk(2);
    }

    fn cli(&mut self) {
        self.flags.if_flag = false;
        self.clk(3);
    }

    fn sti(&mut self) {
        self.flags.if_flag = true;
        self.no_interrupt = 1;
        self.clk(2);
    }

    fn cld(&mut self) {
        self.flags.df = false;
        self.clk(2);
    }

    fn std(&mut self) {
        self.flags.df = true;
        self.clk(2);
    }

    fn cmc(&mut self) {
        self.flags.carry_val = if self.flags.cf() { 0 } else { 1 };
        self.clk(2);
    }

    fn hlt(&mut self) {
        self.halted = true;
        self.state.timing.note_halt();
        self.sync_timing_cycles();
    }

    fn invalid(&mut self, bus: &mut impl common::Bus) {
        // #UD (Invalid Opcode, INT 6): introduced with the 80286.
        // The fault pushes CS:IP pointing to the faulting opcode.
        self.raise_fault(6, bus);
    }
}
