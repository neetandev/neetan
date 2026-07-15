use super::{
    ADDRESS_MASK, EaClass, I286, address_is_odd, modrm,
    timing::{
        I286ControlTransferTimingTemplate, I286DemandPrefetchPolicy, I286FinishState,
        LOCK_PREFIX_OVERLAP_CREDIT,
    },
};
use crate::{ByteReg, SegReg16, WordReg};

const FAR_INDIRECT_JMP_RESTART_TIMING: I286ControlTransferTimingTemplate =
    I286ControlTransferTimingTemplate {
        initial_internal_cycles: 4,
        restart_prefetch_fetches: 3,
        final_internal_cycles: 2,
    };

const LOCK_FAR_INDIRECT_JMP_RESTART_TIMING: I286ControlTransferTimingTemplate =
    I286ControlTransferTimingTemplate {
        initial_internal_cycles: 5,
        restart_prefetch_fetches: 3,
        final_internal_cycles: 2,
    };

const FAR_INDIRECT_CALL_RESTART_TIMING: I286ControlTransferTimingTemplate =
    I286ControlTransferTimingTemplate {
        initial_internal_cycles: 2,
        restart_prefetch_fetches: 1,
        final_internal_cycles: 2,
    };

const NEAR_INDIRECT_CALL_MEMORY_RESTART_TIMING: I286ControlTransferTimingTemplate =
    I286ControlTransferTimingTemplate {
        initial_internal_cycles: 1,
        restart_prefetch_fetches: 3,
        final_internal_cycles: 2,
    };

const SPLIT_STACK_NEAR_INDIRECT_CALL_MEMORY_RESTART_TIMING: I286ControlTransferTimingTemplate =
    I286ControlTransferTimingTemplate {
        initial_internal_cycles: 0,
        restart_prefetch_fetches: 3,
        final_internal_cycles: 2,
    };

const LOCK_NEAR_INDIRECT_CALL_MEMORY_RESTART_TIMING: I286ControlTransferTimingTemplate =
    I286ControlTransferTimingTemplate {
        initial_internal_cycles: 2,
        restart_prefetch_fetches: 3,
        final_internal_cycles: 2,
    };

const LOCK_SPLIT_STACK_NEAR_INDIRECT_CALL_MEMORY_RESTART_TIMING: I286ControlTransferTimingTemplate =
    I286ControlTransferTimingTemplate {
        initial_internal_cycles: 1,
        restart_prefetch_fetches: 3,
        final_internal_cycles: 2,
    };

const NEAR_INDIRECT_JMP_RESTART_TIMING: I286ControlTransferTimingTemplate =
    I286ControlTransferTimingTemplate {
        initial_internal_cycles: 2,
        restart_prefetch_fetches: 3,
        final_internal_cycles: 2,
    };

const LOCK_NEAR_INDIRECT_JMP_MEMORY_RESTART_TIMING: I286ControlTransferTimingTemplate =
    I286ControlTransferTimingTemplate {
        initial_internal_cycles: 3,
        restart_prefetch_fetches: 3,
        final_internal_cycles: 2,
    };

const NEAR_INDIRECT_JMP_REGISTER_RESTART_TIMING: I286ControlTransferTimingTemplate =
    I286ControlTransferTimingTemplate {
        initial_internal_cycles: 1,
        restart_prefetch_fetches: 3,
        final_internal_cycles: 2,
    };

const PREFIXED_NEAR_INDIRECT_JMP_REGISTER_RESTART_TIMING: I286ControlTransferTimingTemplate =
    I286ControlTransferTimingTemplate {
        initial_internal_cycles: 0,
        restart_prefetch_fetches: 3,
        final_internal_cycles: 2,
    };

impl I286 {
    /// True when the operand is a register form, the LOCK prefix is active,
    /// and the prefix count is odd. The 286 charges an extra AU demand cycle
    /// in this configuration during indirect-pointer/jmp dispatch.
    fn lock_register_form_with_odd_prefix(&self, ea_class: EaClass) -> bool {
        self.state.timing.lock_active()
            && self.state.timing.prefix_count_is_odd()
            && ea_class.is_no_displacement_memory()
    }

    fn shift_cl_memory_cycles(&self, count_cycles: i32) -> i32 {
        let memory_operand_credit = match self.ea_class {
            EaClass::SingleRegister => 2,
            EaClass::DoubleRegister => 3,
            EaClass::Disp8Double | EaClass::Disp16Double => 1,
            _ => 0,
        };
        8 + count_cycles - memory_operand_credit
    }

    fn divide_word_memory_cycles(&self, memory_cycles: i32) -> i32 {
        let memory_operand_credit = match self.ea_class {
            EaClass::SingleRegister | EaClass::DoubleRegister => 3,
            EaClass::Disp8Double if self.state.timing.lock_active() => 3,
            _ => 0,
        };
        memory_cycles - memory_operand_credit
    }

    fn divide_byte_memory_cycles(&self, memory_cycles: i32) -> i32 {
        let memory_operand_credit = match self.ea_class {
            EaClass::SingleRegister => 2,
            EaClass::DoubleRegister => 3,
            EaClass::Disp8Double if self.state.timing.lock_active() => 3,
            EaClass::Disp8Double | EaClass::Disp16Double => 1,
            _ => 0,
        };
        memory_cycles - memory_operand_credit
    }

    fn prepare_byte_immediate_memory_operand(&mut self, modrm: u8) {
        if self.state.timing.prefix_count_is_nonzero() {
            if modrm::modrm_is_no_displacement_memory(modrm) {
                self.state
                    .timing
                    .note_demand_prefetch_policy(I286DemandPrefetchPolicy::BeforeNoTurnaround);
                if self.state.timing.prefix_count_is_odd() {
                    self.state.timing.suppress_next_demand_prefetch();
                    self.state
                        .timing
                        .borrow_internal_cycles(LOCK_PREFIX_OVERLAP_CREDIT);
                    self.state.timing.note_au_demand_cycles(1);
                } else {
                    self.state.timing.note_demand_prefetch_limit(1);
                }
                return;
            }
            if modrm::modrm_is_direct_memory(modrm) {
                if self.state.timing.prefix_count_is_odd() {
                    self.state.timing.note_demand_prefetch_limit(1);
                    self.state.timing.note_au_demand_cycles(1);
                } else {
                    self.state
                        .timing
                        .note_demand_prefetch_policy(I286DemandPrefetchPolicy::BeforeNoTurnaround);
                    self.state.timing.note_demand_prefetch_limit(2);
                }
                return;
            }
            if modrm::modrm_is_disp8_memory(modrm) {
                self.state.timing.note_demand_prefetch_policy(
                    I286DemandPrefetchPolicy::AfterTurnaroundThenPrefetch,
                );
                self.state.timing.clear_au_demand_cycles();
                let au_cycles = i32::from(modrm::modrm_uses_double_register_base(modrm))
                    + i32::from(self.state.timing.prefix_count_is_even());
                if au_cycles != 0 {
                    self.state.timing.note_au_demand_cycles(au_cycles as u8);
                }
                self.state.timing.note_demand_prefetch_limit(1);
                return;
            }
            if modrm::modrm_is_disp16_memory(modrm) {
                if self.state.timing.prefix_count_is_odd() {
                    let au_cycles = if modrm::modrm_uses_double_register_base(modrm) {
                        2
                    } else {
                        1
                    };
                    self.state.timing.note_au_demand_cycles(au_cycles);
                    self.state.timing.note_demand_prefetch_limit(1);
                } else {
                    self.state
                        .timing
                        .note_demand_prefetch_policy(I286DemandPrefetchPolicy::BeforeNoTurnaround);
                    if modrm::modrm_uses_double_register_base(modrm) {
                        self.state.timing.note_au_demand_cycles(1);
                    }
                    self.state.timing.note_demand_prefetch_limit(2);
                }
                return;
            }
            self.prepare_immediate_memory_operand(modrm);
            return;
        }

        match modrm {
            _ if modrm::modrm_is_direct_memory(modrm) => {
                self.state
                    .timing
                    .note_demand_prefetch_policy(I286DemandPrefetchPolicy::BeforeNoTurnaround);
                self.state.timing.note_demand_prefetch_limit(2);
            }
            _ if modrm::modrm_is_no_displacement_memory(modrm) => {
                self.state
                    .timing
                    .note_demand_prefetch_policy(I286DemandPrefetchPolicy::BeforeNoTurnaround);
                self.state.timing.note_demand_prefetch_limit(1);
            }
            _ if modrm::modrm_is_disp8_memory(modrm) => {
                let au_cycles = if modrm::modrm_uses_double_register_base(modrm) {
                    2
                } else {
                    1
                };
                self.state.timing.note_au_demand_cycles(au_cycles);
                self.state.timing.note_demand_prefetch_limit(1);
            }
            _ if modrm::modrm_is_disp16_memory(modrm) => {
                self.state
                    .timing
                    .note_demand_prefetch_policy(I286DemandPrefetchPolicy::BeforeNoTurnaround);
                self.state.timing.note_demand_prefetch_limit(2);
            }
            _ => {}
        }
    }

    pub(super) fn prepare_immediate_memory_operand(&mut self, modrm: u8) {
        match modrm {
            _ if modrm::modrm_is_direct_memory(modrm) => {
                self.state.timing.note_demand_prefetch_limit(2);
            }
            _ if modrm::modrm_is_no_displacement_memory(modrm) => {
                self.state
                    .timing
                    .note_demand_prefetch_policy(I286DemandPrefetchPolicy::BeforeTurnaround);
                self.state.timing.note_demand_prefetch_limit(1);
            }
            _ if modrm::modrm_is_disp8_memory(modrm) && self.state.timing.prefix_count_is_odd() => {
                self.state.timing.note_demand_prefetch_policy(
                    I286DemandPrefetchPolicy::AfterTurnaroundPrefetchThenGap,
                );
                self.state.timing.note_demand_prefetch_limit(1);
            }
            _ if modrm::modrm_is_disp8_memory(modrm) || modrm::modrm_is_disp16_memory(modrm) => {
                self.state.timing.note_demand_prefetch_limit(2);
            }
            _ => {}
        }
    }

    pub(super) fn prepare_sign_extended_immediate_memory_operand(&mut self, modrm: u8) {
        match modrm {
            _ if modrm::modrm_is_direct_memory(modrm) => {
                self.state.timing.note_demand_prefetch_policy(
                    I286DemandPrefetchPolicy::BeforeAndAfterTurnaround,
                );
                self.state.timing.note_demand_prefetch_limit(1);
            }
            _ if modrm::modrm_is_no_displacement_memory(modrm) => {
                self.state
                    .timing
                    .note_demand_prefetch_policy(I286DemandPrefetchPolicy::AfterTurnaround);
                self.state.timing.note_demand_prefetch_limit(1);
            }
            _ if modrm::modrm_is_disp8_memory(modrm) && self.state.timing.prefix_count_is_odd() => {
                self.state.timing.note_demand_prefetch_policy(
                    I286DemandPrefetchPolicy::AfterTurnaroundAuThenPrefetchThenGap,
                );
                let au_cycles = if modrm::modrm_uses_double_register_base(modrm) {
                    1
                } else {
                    0
                };
                self.state.timing.note_au_demand_cycles(au_cycles);
                self.state.timing.note_demand_prefetch_limit(1);
            }
            _ if modrm::modrm_is_disp8_memory(modrm) => {
                let au_cycles = if modrm::modrm_uses_double_register_base(modrm) {
                    3
                } else {
                    2
                };
                self.state.timing.note_au_demand_cycles(au_cycles);
                self.state.timing.note_demand_prefetch_limit(1);
            }
            _ if modrm::modrm_is_disp16_memory(modrm)
                && self.state.timing.prefix_count_is_nonzero() =>
            {
                self.state.timing.note_demand_prefetch_policy(
                    I286DemandPrefetchPolicy::BeforeAndAfterTurnaround,
                );
                self.state.timing.note_au_demand_cycles(1);
                self.state.timing.note_demand_prefetch_limit(1);
            }
            _ if modrm::modrm_is_disp16_memory(modrm) => {
                self.state.timing.note_demand_prefetch_policy(
                    I286DemandPrefetchPolicy::BeforeAndAfterTurnaround,
                );
                self.state.timing.note_demand_prefetch_limit(1);
            }
            _ if modrm::modrm_is_memory(modrm) => {
                self.state.timing.note_demand_prefetch_limit(1);
            }
            _ => {}
        }
    }

    fn prepare_immediate_shift_memory_operand(&mut self, modrm: u8, count: u8) {
        if self.state.timing.prefix_count_is_zero() {
            if modrm::modrm_is_disp16_memory(modrm)
                && self
                    .state
                    .timing
                    .prefetch_wrapped_before_instruction_start()
            {
                self.state
                    .timing
                    .note_demand_prefetch_policy(I286DemandPrefetchPolicy::BeforeNoTurnaround);
                self.state.timing.note_au_demand_cycles(2);
                self.state.timing.note_demand_prefetch_limit(2);
                return;
            }
            self.prepare_byte_immediate_memory_operand(modrm);
            return;
        }

        let prefix_count = self.state.timing.prefix_count();
        let no_displacement = self.ea_class.is_no_displacement_memory();
        if count & 0x1F == 0
            && self.state.timing.lock_active()
            && prefix_count == 2
            && !no_displacement
        {
            self.state
                .timing
                .note_demand_prefetch_policy(I286DemandPrefetchPolicy::BeforeNoTurnaround);
            self.state.timing.note_demand_prefetch_limit(1);
            return;
        }

        if !no_displacement {
            if (prefix_count & 1 == 0 && self.ea_class.is_disp8())
                || (self.state.timing.lock_active() && prefix_count == 2)
            {
                self.state.timing.note_demand_prefetch_policy(
                    I286DemandPrefetchPolicy::AfterTurnaroundThenPrefetch,
                );
                let au_demand_cycles = match self.ea_class {
                    EaClass::Disp8Double => 2,
                    _ => 1,
                };
                self.state.timing.note_au_demand_cycles(au_demand_cycles);
                self.state.timing.note_demand_prefetch_limit(1);
            } else if prefix_count & 1 == 0 {
                self.state
                    .timing
                    .note_demand_prefetch_policy(I286DemandPrefetchPolicy::BeforeNoTurnaround);
                self.state.timing.note_demand_prefetch_limit(2);
            } else {
                if self.ea_class.is_disp8() {
                    self.state.timing.note_demand_prefetch_policy(
                        I286DemandPrefetchPolicy::AfterTurnaroundThenPrefetch,
                    );
                } else {
                    self.state
                        .timing
                        .note_demand_prefetch_policy(I286DemandPrefetchPolicy::BeforeNoTurnaround);
                    let au_demand_cycles = match self.ea_class {
                        EaClass::Disp16Double => 2,
                        _ => 1,
                    };
                    self.state.timing.note_au_demand_cycles(au_demand_cycles);
                }
                self.state.timing.note_demand_prefetch_limit(1);
            }
            return;
        }

        self.state
            .timing
            .note_demand_prefetch_policy(I286DemandPrefetchPolicy::BeforeNoTurnaround);
        self.state.timing.note_demand_prefetch_limit(1);
        if self.state.timing.prefix_count_is_odd() {
            self.state.timing.clear_au_demand_cycles();
            self.state.timing.suppress_next_demand_prefetch();
            self.state.timing.note_au_demand_cycles(1);
        }
    }

    fn prepare_far_indirect_pointer_read(&mut self) {
        if self.ea_class.is_disp8() || self.state.timing.prefix_count_is_odd() {
            self.state.timing.passivize_next_demand_prefetch();
        }

        if self.lock_register_form_with_odd_prefix(self.ea_class) {
            self.state.timing.note_au_demand_cycles(1);
        }
    }

    fn prepare_far_indirect_modrm_fetch(&mut self, bus: &mut impl common::Bus) {
        if self.state.timing.prefix_count_at_most(1) {
            return;
        }

        let code_segment_base = self.seg_bases[SegReg16::CS as usize];
        let modrm =
            bus.read_byte(code_segment_base.wrapping_add(u32::from(self.ip)) & ADDRESS_MASK);
        let reg = modrm::modrm_register(modrm);
        if !matches!(reg, 3 | 5) || modrm::modrm_is_register(modrm) {
            return;
        }

        if modrm::modrm_is_no_displacement_memory(modrm) {
            self.state.timing.passivize_next_code_fetch();
        }
    }

    fn prepare_near_indirect_jmp_read(&mut self, modrm: u8) {
        if modrm::modrm_is_register(modrm) {
            return;
        }

        let ea_class = EaClass::from_modrm(modrm);
        if ea_class.is_disp8() || self.state.timing.prefix_count_is_odd() {
            self.state.timing.passivize_next_demand_prefetch();
        }

        if self.lock_register_form_with_odd_prefix(ea_class) {
            self.state.timing.note_au_demand_cycles(1);
        }
    }

    /// Group 0x80: ALU r/m8, imm8
    pub(super) fn group_80(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let operation = modrm::modrm_register(modrm);
        let src;
        let dst;
        if modrm::modrm_is_register(modrm) {
            src = self.fetch(bus);
            dst = self.regs.byte(self.rm_byte(modrm));
        } else {
            self.calc_ea(modrm, bus);
            self.prepare_byte_immediate_memory_operand(modrm);
            src = self.fetch(bus);
            dst = self.seg_read_byte_at(bus, 0);
            self.state.timing.note_au_idle();
        }

        let result = match operation {
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
                self.clk_modrm_prefetch(bus, modrm, 3, 6);
                return;
            }
            _ => unreachable!(),
        };
        if operation != 7 {
            self.putback_rm_byte(modrm, result, bus);
        }
        self.clk_modrm_prefetch(bus, modrm, 3, 7);
    }

    /// Group 0x81: ALU r/m16, imm16
    pub(super) fn group_81(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let operation = modrm::modrm_register(modrm);
        let src;
        let dst;
        if modrm::modrm_is_register(modrm) {
            src = self.fetchword(bus);
            dst = self.regs.word(self.rm_word(modrm));
        } else {
            self.calc_ea(modrm, bus);
            self.prepare_immediate_memory_operand(modrm);
            src = self.fetchword(bus);
            dst = self.seg_read_word(bus);
            self.state.timing.note_au_idle();
        }

        let result = match operation {
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
                self.clk_modrm_word_prefetch(bus, modrm, 6, 6, 0);
                return;
            }
            _ => unreachable!(),
        };
        if operation != 7 {
            self.putback_rm_word(modrm, result, bus);
        }
        self.clk_modrm_word_prefetch(bus, modrm, 6, 6, 0);
    }

    /// Group 0x82: ALU r/m8, imm8 (same as 0x80)
    pub(super) fn group_82(&mut self, bus: &mut impl common::Bus) {
        self.group_80(bus);
    }

    /// Group 0x83: ALU r/m16, sign-extended imm8
    pub(super) fn group_83(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let operation = modrm::modrm_register(modrm);
        let src;
        let dst;
        if modrm::modrm_is_register(modrm) {
            src = self.fetch(bus) as i8 as u16;
            dst = self.regs.word(self.rm_word(modrm));
        } else {
            self.calc_ea(modrm, bus);
            self.prepare_sign_extended_immediate_memory_operand(modrm);
            src = self.fetch(bus) as i8 as u16;
            dst = self.seg_read_word(bus);
            self.state.timing.note_au_idle();
        }

        let result = match operation {
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
                if modrm::modrm_is_register(modrm) {
                    self.clk_visible(1);
                    self.clk_prefetch(bus, 5);
                } else {
                    self.clk_modrm_word_prefetch(bus, modrm, 6, 6, 0);
                }
                return;
            }
            _ => unreachable!(),
        };
        if operation != 7 {
            self.putback_rm_word(modrm, result, bus);
        }
        if modrm::modrm_is_register(modrm) {
            self.clk_visible(1);
            self.clk_prefetch(bus, 5);
        } else {
            self.clk_modrm_word_prefetch(bus, modrm, 6, 6, 0);
        }
    }

    /// Group 0xC0: shift/rotate r/m8, imm8
    pub(super) fn group_c0(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let (dst, count) = if modrm::modrm_is_register(modrm) {
            (self.regs.byte(self.rm_byte(modrm)), self.fetch(bus))
        } else {
            self.calc_ea(modrm, bus);
            let count = self.fetch(bus);
            self.prepare_immediate_shift_memory_operand(modrm, count);
            let dst = self.seg_read_byte_at(bus, 0);
            self.state.timing.note_au_idle();
            (dst, count)
        };
        let result = match modrm::modrm_register(modrm) {
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
        let n = (count & 0x1F) as i32;
        if modrm::modrm_is_register(modrm) {
            self.putback_rm_byte(modrm, result, bus);
            self.clk_modrm_prefetch(bus, modrm, 5 + n, 8 + n);
        } else if n == 0 {
            self.clk_visible(4);
        } else {
            let short_ea_path = matches!(
                self.ea_class,
                EaClass::Direct
                    | EaClass::SingleRegister
                    | EaClass::Disp8Single
                    | EaClass::Disp16Single
            );
            self.state
                .timing
                .overlap_immediate_shift_writeback_compute(short_ea_path);
            self.clk_modrm_prefetch(bus, modrm, 5 + n, 8 + n);
            self.putback_rm_byte(modrm, result, bus);
            self.clk_visible(1);
        }
    }

    /// Group 0xC1: shift/rotate r/m16, imm8
    pub(super) fn group_c1(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let (dst, count) = if modrm::modrm_is_register(modrm) {
            (self.regs.word(self.rm_word(modrm)), self.fetch(bus))
        } else {
            self.calc_ea(modrm, bus);
            let count = self.fetch(bus);
            self.prepare_immediate_shift_memory_operand(modrm, count);
            let dst = self.seg_read_word(bus);
            self.state.timing.note_au_idle();
            (dst, count)
        };
        let result = match modrm::modrm_register(modrm) {
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
        let n = (count & 0x1F) as i32;
        if modrm::modrm_is_register(modrm) {
            self.putback_rm_word(modrm, result, bus);
            self.clk_modrm_word_prefetch(bus, modrm, 5 + n, 8 + n, 0);
        } else if n == 0 {
            self.clk_visible(4);
        } else {
            let write_splits = self.word_access_is_split(self.ea_seg, self.eo);
            let short_ea_path = matches!(
                self.ea_class,
                EaClass::Direct
                    | EaClass::SingleRegister
                    | EaClass::Disp8Single
                    | EaClass::Disp16Single
            );
            self.state
                .timing
                .overlap_immediate_shift_writeback_compute(short_ea_path);
            self.clk_modrm_word_prefetch(bus, modrm, 5 + n, 8 + n, 0);
            self.putback_rm_word(modrm, result, bus);
            if !write_splits {
                self.clk_visible(1);
            }
        }
    }

    /// Group 0xD0: shift/rotate r/m8, 1
    pub(super) fn group_d0(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let destination = self.get_rm_byte(modrm, bus);
        let result = match modrm::modrm_register(modrm) {
            0 => self.alu_rol_byte(destination, 1),
            1 => self.alu_ror_byte(destination, 1),
            2 => self.alu_rcl_byte(destination, 1),
            3 => self.alu_rcr_byte(destination, 1),
            4 => self.alu_shl_byte(destination, 1),
            5 => self.alu_shr_byte(destination, 1),
            6 => self.alu_shl_byte(destination, 1),
            7 => self.alu_sar_byte(destination, 1),
            _ => unreachable!(),
        };
        self.putback_rm_byte(modrm, result, bus);
        self.clk_modrm_prefetch(bus, modrm, 2, 6);
    }

    /// Group 0xD1: shift/rotate r/m16, 1
    pub(super) fn group_d1(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let destination = self.get_rm_word(modrm, bus);
        let result = match modrm::modrm_register(modrm) {
            0 => self.alu_rol_word(destination, 1),
            1 => self.alu_ror_word(destination, 1),
            2 => self.alu_rcl_word(destination, 1),
            3 => self.alu_rcr_word(destination, 1),
            4 => self.alu_shl_word(destination, 1),
            5 => self.alu_shr_word(destination, 1),
            6 => self.alu_shl_word(destination, 1),
            7 => self.alu_sar_word(destination, 1),
            _ => unreachable!(),
        };
        self.putback_rm_word(modrm, result, bus);
        self.clk_modrm_word_prefetch(bus, modrm, 2, 6, 0);
    }

    /// Group 0xD2: shift/rotate r/m8, CL
    pub(super) fn group_d2(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let dst = self.get_rm_byte(modrm, bus);
        let count = self.regs.byte(ByteReg::CL);
        let result = match modrm::modrm_register(modrm) {
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
        let n = (count & 0x1F) as i32;
        if modrm::modrm_is_register(modrm) {
            self.putback_rm_byte(modrm, result, bus);
            self.clk_modrm_prefetch(bus, modrm, 5 + n, 8 + n);
        } else if n == 0 {
            self.clk_visible(4);
        } else {
            let prefixed_displacement_path = !matches!(
                self.ea_class,
                EaClass::SingleRegister | EaClass::DoubleRegister
            );
            self.state
                .timing
                .overlap_read_modify_write_compute(prefixed_displacement_path);
            self.clk_modrm_prefetch(bus, modrm, 5 + n, self.shift_cl_memory_cycles(n));
            self.putback_rm_byte(modrm, result, bus);
            self.clk_visible(1);
        }
    }

    /// Group 0xD3: shift/rotate r/m16, CL
    pub(super) fn group_d3(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let dst = self.get_rm_word(modrm, bus);
        let count = self.regs.byte(ByteReg::CL);
        let result = match modrm::modrm_register(modrm) {
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
        let n = (count & 0x1F) as i32;
        if modrm::modrm_is_register(modrm) {
            self.putback_rm_word(modrm, result, bus);
            self.clk_modrm_word_prefetch(bus, modrm, 5 + n, 8 + n, 0);
        } else if n == 0 {
            self.clk_visible(4);
        } else {
            let write_splits = self.word_access_is_split(self.ea_seg, self.eo);
            let prefixed_displacement_path = !matches!(
                self.ea_class,
                EaClass::SingleRegister | EaClass::DoubleRegister
            );
            self.state
                .timing
                .overlap_read_modify_write_compute(prefixed_displacement_path);
            self.clk_modrm_word_prefetch(bus, modrm, 5 + n, self.shift_cl_memory_cycles(n), 0);
            self.putback_rm_word(modrm, result, bus);
            if !write_splits {
                self.clk_visible(1);
            }
        }
    }

    /// Group 0xF6: various byte operations
    pub(super) fn group_f6(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let op = modrm::modrm_register(modrm);
        match op {
            0 | 1 => {
                // TEST r/m8, imm8
                let src;
                let dst;
                if modrm::modrm_is_register(modrm) {
                    src = self.fetch(bus);
                    dst = self.regs.byte(self.rm_byte(modrm));
                } else {
                    self.calc_ea(modrm, bus);
                    self.prepare_byte_immediate_memory_operand(modrm);
                    src = self.fetch(bus);
                    dst = self.seg_read_byte_at(bus, 0);
                    self.state.timing.note_au_idle();
                }
                self.alu_and_byte(dst, src);
                self.clk_modrm_prefetch(bus, modrm, 3, 6);
            }
            2 => {
                // NOT r/m8
                let dst = self.get_rm_byte(modrm, bus);
                self.putback_rm_byte(modrm, !dst, bus);
                self.clk_modrm_prefetch(bus, modrm, 2, 6);
            }
            3 => {
                // NEG r/m8
                let dst = self.get_rm_byte(modrm, bus);
                let result = self.alu_neg_byte(dst);
                self.putback_rm_byte(modrm, result, bus);
                self.clk_modrm_prefetch(bus, modrm, 2, 6);
            }
            4 => {
                // MUL r/m8 (unsigned)
                let src = self.get_rm_byte(modrm, bus);
                let al = self.regs.byte(ByteReg::AL);
                let result = al as u16 * src as u16;
                self.regs.set_word(WordReg::AX, result);
                self.flags.carry_val = if result & 0xFF00 != 0 { 1 } else { 0 };
                self.flags.overflow_val = self.flags.carry_val;
                self.clk_modrm_prefetch(bus, modrm, 13, 16);
            }
            5 => {
                // IMUL r/m8 (signed)
                let src = self.get_rm_byte(modrm, bus) as i8 as i16;
                let al = self.regs.byte(ByteReg::AL) as i8 as i16;
                let result = al * src;
                self.regs.set_word(WordReg::AX, result as u16);
                let ah = (result >> 8) as i8;
                let al_sign = result as i8;
                self.flags.carry_val = if ah != (al_sign >> 7) { 1 } else { 0 };
                self.flags.overflow_val = self.flags.carry_val;
                self.clk_modrm_prefetch(bus, modrm, 13, 16);
            }
            6 => {
                // DIV r/m8 (unsigned)
                let src = self.get_rm_byte(modrm, bus) as u16;
                if src == 0 {
                    self.raise_fault(0, bus);
                    return;
                }
                let aw = self.regs.word(WordReg::AX);
                let quotient = aw / src;
                if quotient > 0xFF {
                    self.raise_fault(0, bus);
                    return;
                }
                let remainder = aw % src;
                self.regs.set_byte(ByteReg::AL, quotient as u8);
                self.regs.set_byte(ByteReg::AH, remainder as u8);
                self.clk_modrm_prefetch(bus, modrm, 14, self.divide_byte_memory_cycles(17));
            }
            7 => {
                // IDIV r/m8 (signed)
                let src = self.get_rm_byte(modrm, bus) as i8 as i16;
                if src == 0 {
                    self.raise_fault(0, bus);
                    return;
                }
                let aw = self.regs.word(WordReg::AX) as i16;
                let Some(quotient) = aw.checked_div(src) else {
                    self.raise_fault(0, bus);
                    return;
                };
                if !(-128..=127).contains(&quotient) {
                    self.raise_fault(0, bus);
                    return;
                }
                let remainder = aw.checked_rem(src).unwrap_or(0);
                self.regs.set_byte(ByteReg::AL, quotient as u8);
                self.regs.set_byte(ByteReg::AH, remainder as u8);
                self.clk_modrm_prefetch(bus, modrm, 17, self.divide_byte_memory_cycles(20));
            }
            _ => unreachable!(),
        }
    }

    /// Group 0xF7: various word operations
    pub(super) fn group_f7(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        let op = modrm::modrm_register(modrm);
        match op {
            0 | 1 => {
                // TEST r/m16, imm16
                let src;
                let dst;
                if modrm::modrm_is_register(modrm) {
                    src = self.fetchword(bus);
                    dst = self.regs.word(self.rm_word(modrm));
                } else {
                    self.calc_ea(modrm, bus);
                    self.prepare_immediate_memory_operand(modrm);
                    src = self.fetchword(bus);
                    dst = self.seg_read_word(bus);
                    self.state.timing.note_au_idle();
                }
                self.alu_and_word(dst, src);
                self.clk_modrm_word_prefetch(bus, modrm, 6, 7, 0);
            }
            2 => {
                // NOT r/m16
                let dst = self.get_rm_word(modrm, bus);
                self.putback_rm_word(modrm, !dst, bus);
                self.clk_modrm_word_prefetch(bus, modrm, 2, 6, 0);
            }
            3 => {
                // NEG r/m16
                let dst = self.get_rm_word(modrm, bus);
                let result = self.alu_neg_word(dst);
                self.putback_rm_word(modrm, result, bus);
                self.clk_modrm_word_prefetch(bus, modrm, 2, 6, 0);
            }
            4 => {
                // MUL r/m16 (unsigned)
                let src = self.get_rm_word(modrm, bus);
                let aw = self.regs.word(WordReg::AX);
                let result = aw as u32 * src as u32;
                self.regs.set_word(WordReg::AX, result as u16);
                self.regs.set_word(WordReg::DX, (result >> 16) as u16);
                self.flags.carry_val = if result & 0xFFFF0000 != 0 { 1 } else { 0 };
                self.flags.overflow_val = self.flags.carry_val;
                self.clk_modrm_word_prefetch(bus, modrm, 21, 24, 0);
            }
            5 => {
                // IMUL r/m16 (signed)
                let src = self.get_rm_word(modrm, bus) as i16 as i32;
                let aw = self.regs.word(WordReg::AX) as i16 as i32;
                let result = aw * src;
                self.regs.set_word(WordReg::AX, result as u16);
                self.regs.set_word(WordReg::DX, (result >> 16) as u16);
                let upper = (result >> 16) as i16;
                let lower_sign = result as i16;
                self.flags.carry_val = if upper != (lower_sign >> 15) { 1 } else { 0 };
                self.flags.overflow_val = self.flags.carry_val;
                self.clk_modrm_word_prefetch(bus, modrm, 21, 24, 0);
            }
            6 => {
                // DIV r/m16 (unsigned)
                let src = self.get_rm_word(modrm, bus) as u32;
                if src == 0 {
                    self.raise_fault(0, bus);
                    return;
                }
                let dw = self.regs.word(WordReg::DX) as u32;
                let aw = self.regs.word(WordReg::AX) as u32;
                let dividend = (dw << 16) | aw;
                let quotient = dividend / src;
                if quotient > 0xFFFF {
                    self.raise_fault(0, bus);
                    return;
                }
                let remainder = dividend % src;
                self.regs.set_word(WordReg::AX, quotient as u16);
                self.regs.set_word(WordReg::DX, remainder as u16);
                self.clk_modrm_word_prefetch(bus, modrm, 22, self.divide_word_memory_cycles(25), 0);
            }
            7 => {
                // IDIV r/m16 (signed)
                let src = self.get_rm_word(modrm, bus) as i16 as i32;
                if src == 0 {
                    self.raise_fault(0, bus);
                    return;
                }
                let dw = self.regs.word(WordReg::DX) as u32;
                let aw = self.regs.word(WordReg::AX) as u32;
                let dividend = ((dw << 16) | aw) as i32;
                let Some(quotient) = dividend.checked_div(src) else {
                    self.raise_fault(0, bus);
                    return;
                };
                if !(-32768..=32767).contains(&quotient) {
                    self.raise_fault(0, bus);
                    return;
                }
                let remainder = dividend.checked_rem(src).unwrap_or(0);
                self.regs.set_word(WordReg::AX, quotient as u16);
                self.regs.set_word(WordReg::DX, remainder as u16);
                self.clk_modrm_word_prefetch(bus, modrm, 25, self.divide_word_memory_cycles(28), 0);
            }
            _ => unreachable!(),
        }
    }

    /// Group 0xFE: INC/DEC r/m8
    pub(super) fn group_fe(&mut self, bus: &mut impl common::Bus) {
        let modrm = self.fetch(bus);
        match modrm::modrm_register(modrm) {
            0 => {
                // INC r/m8
                let dst = self.get_rm_byte(modrm, bus);
                let result = self.alu_inc_byte(dst);
                self.putback_rm_byte(modrm, result, bus);
                self.clk_modrm_prefetch(bus, modrm, 2, 6);
            }
            1 => {
                // DEC r/m8
                let dst = self.get_rm_byte(modrm, bus);
                let result = self.alu_dec_byte(dst);
                self.putback_rm_byte(modrm, result, bus);
                self.clk_modrm_prefetch(bus, modrm, 2, 6);
            }
            _ => {
                self.clk(2);
            }
        }
    }

    /// Group 0xFF: various word operations
    pub(super) fn group_ff(&mut self, bus: &mut impl common::Bus) {
        self.prepare_far_indirect_modrm_fetch(bus);
        let modrm = self.fetch(bus);
        match modrm::modrm_register(modrm) {
            0 => {
                // INC r/m16
                let dst = self.get_rm_word(modrm, bus);
                let result = self.alu_inc_word(dst);
                self.putback_rm_word(modrm, result, bus);
                self.clk_modrm_word_prefetch(bus, modrm, 2, 6, 0);
            }
            1 => {
                // DEC r/m16
                let dst = self.get_rm_word(modrm, bus);
                let result = self.alu_dec_word(dst);
                self.putback_rm_word(modrm, result, bus);
                self.clk_modrm_word_prefetch(bus, modrm, 2, 6, 0);
            }
            2 => {
                // CALL r/m16 (near indirect)
                self.finish_state = I286FinishState::ControlTransferRestart;
                if modrm::modrm_is_memory(modrm) {
                    self.prepare_near_indirect_jmp_read(modrm);
                }
                let dst = self.get_rm_word(modrm, bus);
                let return_ip = self.ip;
                self.ip = dst;
                if modrm::modrm_is_register(modrm) {
                    let stack_write_split = self.word_access_is_split(
                        SegReg16::SS,
                        self.regs.word(WordReg::SP).wrapping_sub(2),
                    );
                    self.state.timing.arm_control_transfer_restart(self.ip);
                    let code_segment_base = self.seg_bases[SegReg16::CS as usize];
                    let initial_cycles = if !self.state.timing.lock_active()
                        && self.state.timing.prefix_count_is_odd()
                    {
                        0
                    } else {
                        1
                    };
                    self.state
                        .timing
                        .advance_control_transfer_internal_cycles(initial_cycles);
                    self.state
                        .timing
                        .advance_control_transfer_fetches(bus, code_segment_base, 1);
                    self.sync_timing_cycles();
                    self.push(bus, return_ip);
                    if !stack_write_split {
                        self.state.timing.advance_control_transfer_fetches(
                            bus,
                            code_segment_base,
                            1,
                        );
                    }
                    self.state.timing.complete_control_transfer_restart(2);
                    self.sync_timing_cycles();
                } else {
                    let stack_write_split = self.word_access_is_split(
                        SegReg16::SS,
                        self.regs.word(WordReg::SP).wrapping_sub(2),
                    );
                    self.state.timing.suppress_next_read_writeback_gap();
                    self.push(bus, return_ip);
                    let timing = match (self.state.timing.lock_active(), stack_write_split) {
                        (true, true) => LOCK_SPLIT_STACK_NEAR_INDIRECT_CALL_MEMORY_RESTART_TIMING,
                        (true, false) => LOCK_NEAR_INDIRECT_CALL_MEMORY_RESTART_TIMING,
                        (false, true) => SPLIT_STACK_NEAR_INDIRECT_CALL_MEMORY_RESTART_TIMING,
                        (false, false) => NEAR_INDIRECT_CALL_MEMORY_RESTART_TIMING,
                    };
                    self.clk_control_transfer_restart(bus, self.ip, timing);
                }
            }
            3 => {
                // CALL m16:16 (far indirect)
                if modrm::modrm_is_register(modrm) {
                    return;
                }
                self.finish_state = I286FinishState::ControlTransferRestart;
                let sp_pen = self.sp_penalty(2);
                self.calc_ea(modrm, bus);
                self.prepare_far_indirect_pointer_read();
                let offset = self.seg_read_word(bus);
                self.state.timing.suppress_next_memory_read_window();
                let segment = self.seg_read_word_at(bus, 2);
                let cs = self.sregs[SegReg16::CS as usize];
                let return_ip = self.ip;
                if self.is_protected_mode() {
                    self.code_descriptor(
                        segment,
                        offset,
                        super::TaskType::Call,
                        cs,
                        return_ip,
                        bus,
                    );
                } else {
                    if !self.load_segment(SegReg16::CS, segment, bus) {
                        return;
                    }
                    self.ip = offset;
                    self.state
                        .timing
                        .arm_control_transfer_restart_without_gap_credit(self.ip);
                    let code_segment_base = self.seg_bases[SegReg16::CS as usize];
                    let (terminal_fetch_count, restart_tail_cycles) =
                        self.state.timing.control_transfer_restart_fetches_and_tail(
                            FAR_INDIRECT_CALL_RESTART_TIMING,
                        );
                    self.state.timing.suppress_next_read_writeback_gap();
                    if !address_is_odd(self.ea) {
                        self.clk_visible(1);
                    }
                    self.state.timing.note_au_idle();
                    self.state
                        .timing
                        .note_demand_prefetch_policy(I286DemandPrefetchPolicy::None);
                    self.push(bus, cs);
                    self.state.timing.advance_control_transfer_internal_cycles(
                        FAR_INDIRECT_CALL_RESTART_TIMING.initial_internal_cycles,
                    );
                    self.state
                        .timing
                        .advance_control_transfer_fetches(bus, code_segment_base, 1);
                    self.sync_timing_cycles();
                    self.push(bus, return_ip);
                    self.state.timing.advance_control_transfer_fetches(
                        bus,
                        code_segment_base,
                        terminal_fetch_count,
                    );
                    self.state
                        .timing
                        .complete_control_transfer_restart(restart_tail_cycles);
                    self.sync_timing_cycles();
                    return;
                }
                let ea_pen = if address_is_odd(self.ea) { 8 } else { 0 };
                self.clk(16 + sp_pen + ea_pen);
            }
            4 => {
                // JMP r/m16 (near indirect)
                self.finish_state = I286FinishState::ControlTransferRestart;
                let dst = if modrm::modrm_is_register(modrm) {
                    self.regs.word(self.rm_word(modrm))
                } else {
                    self.calc_ea(modrm, bus);
                    self.prepare_near_indirect_jmp_read(modrm);
                    let value = self.seg_read_word(bus);
                    self.state.timing.note_au_idle();
                    value
                };
                self.ip = dst;
                let timing = if modrm::modrm_is_register(modrm) {
                    if self.state.timing.lock_active()
                        && (self.state.timing.lock_prefix_after_prefix()
                            || (self.state.timing.lock_prefix_followed_by_prefix()
                                && self.state.timing.prefix_count_is_odd()))
                    {
                        NEAR_INDIRECT_JMP_REGISTER_RESTART_TIMING
                    } else if self.state.timing.lock_active() {
                        NEAR_INDIRECT_JMP_RESTART_TIMING
                    } else if self.state.timing.prefix_count_is_odd() {
                        PREFIXED_NEAR_INDIRECT_JMP_REGISTER_RESTART_TIMING
                    } else {
                        NEAR_INDIRECT_JMP_REGISTER_RESTART_TIMING
                    }
                } else if self.state.timing.lock_active() {
                    LOCK_NEAR_INDIRECT_JMP_MEMORY_RESTART_TIMING
                } else {
                    NEAR_INDIRECT_JMP_RESTART_TIMING
                };
                self.clk_control_transfer_restart(bus, self.ip, timing);
            }
            5 => {
                // JMP m16:16 (far indirect)
                if modrm::modrm_is_register(modrm) {
                    return;
                }
                self.finish_state = I286FinishState::ControlTransferRestart;
                self.calc_ea(modrm, bus);
                self.prepare_far_indirect_pointer_read();
                let offset = self.seg_read_word(bus);
                self.state.timing.suppress_next_memory_read_window();
                let segment = self.seg_read_word_at(bus, 2);
                if self.is_protected_mode() {
                    self.code_descriptor(segment, offset, super::TaskType::Jmp, 0, 0, bus);
                } else {
                    self.ip = offset;
                    if !self.load_segment(SegReg16::CS, segment, bus) {
                        return;
                    }
                    let timing = if self.state.timing.lock_active() {
                        LOCK_FAR_INDIRECT_JMP_RESTART_TIMING
                    } else {
                        FAR_INDIRECT_JMP_RESTART_TIMING
                    };
                    self.clk_control_transfer_restart_without_gap_credit(bus, self.ip, timing);
                    return;
                }
                let penalty = if address_is_odd(self.ea) { 8 } else { 0 };
                self.clk(15 + penalty);
            }
            6 | 7 => {
                // PUSH r/m16 (7 is undocumented alias)
                let needs_register_setup_cycle = self.state.timing.prefix_count_is_even()
                    && (!self.state.timing.lock_active()
                        || !self.state.timing.lock_prefix_after_prefix());
                if modrm::modrm_is_register(modrm) && modrm::modrm_rm(modrm) == 4 {
                    if needs_register_setup_cycle {
                        self.clk_visible(1);
                    }
                    self.push_sp(bus);
                } else if modrm::modrm_is_register(modrm) {
                    let tail_cycles = self.stack_push_tail_cycles(3);
                    let val = self.regs.word(self.rm_word(modrm));
                    if needs_register_setup_cycle {
                        self.clk_visible(1);
                    }
                    self.push(bus, val);
                    self.clk(tail_cycles);
                } else {
                    let val = self.get_rm_word(modrm, bus);
                    self.push(bus, val);
                    let ea_pen = if address_is_odd(self.ea) { 4 } else { 0 };
                    self.clk(5 + ea_pen);
                }
            }
            _ => {
                self.clk(2);
            }
        }
    }
}
