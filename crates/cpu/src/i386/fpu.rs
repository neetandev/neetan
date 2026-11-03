use softfloat::{ExceptionFlags, Fp80, FpClass, Precision, RoundingMode};

use super::{I386, Step};

const TAG_VALID: u16 = 0b00;
const TAG_ZERO: u16 = 0b01;
const TAG_SPECIAL: u16 = 0b10;
const TAG_EMPTY: u16 = 0b11;

/// The IE flag of the status word and the IM bit of the control word.
const INVALID_OPERATION: u16 = 0x0001;
/// The DE flag of the status word and the DM bit of the control word.
const DENORMAL_OPERAND: u16 = 0x0002;
/// The ZE flag of the status word and the ZM bit of the control word.
const ZERO_DIVIDE: u16 = 0x0004;
/// The OE flag of the status word and the OM bit of the control word.
const OVERFLOW: u16 = 0x0008;
/// The UE flag of the status word and the UM bit of the control word.
const UNDERFLOW: u16 = 0x0010;
/// The C1 condition code bit of the status word.
const CONDITION_C1: u16 = 0x0200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct X87State {
    pub registers: [Fp80; 8],
    pub control_word: u16,
    pub status_word: u16,
    pub tag_word: u16,
    pub fip_offset: u32,
    pub fip_selector: u16,
    pub fdp_offset: u32,
    pub fdp_selector: u16,
    pub fpu_opcode: u16,
}

impl save_state::StateEncode for X87State {
    fn encode_state(&self, output: &mut alloc::vec::Vec<u8>) {
        for register in self.registers {
            save_state::StateEncode::encode_state(&register.to_le_bytes(), output);
        }
        save_state::StateEncode::encode_state(&self.control_word, output);
        save_state::StateEncode::encode_state(&self.status_word, output);
        save_state::StateEncode::encode_state(&self.tag_word, output);
        save_state::StateEncode::encode_state(&self.fip_offset, output);
        save_state::StateEncode::encode_state(&self.fip_selector, output);
        save_state::StateEncode::encode_state(&self.fdp_offset, output);
        save_state::StateEncode::encode_state(&self.fdp_selector, output);
        save_state::StateEncode::encode_state(&self.fpu_opcode, output);
    }
}

impl save_state::StateDecode for X87State {
    fn decode_state(
        decoder: &mut save_state::StateDecoder<'_>,
    ) -> Result<Self, save_state::StateDecodeError> {
        let mut registers = [Fp80::ZERO; 8];
        for register in &mut registers {
            let bytes = <[u8; 10] as save_state::StateDecode>::decode_state(decoder)?;
            *register = Fp80::from_le_bytes(bytes);
        }
        Ok(Self {
            registers,
            control_word: save_state::StateDecode::decode_state(decoder)?,
            status_word: save_state::StateDecode::decode_state(decoder)?,
            tag_word: save_state::StateDecode::decode_state(decoder)?,
            fip_offset: save_state::StateDecode::decode_state(decoder)?,
            fip_selector: save_state::StateDecode::decode_state(decoder)?,
            fdp_offset: save_state::StateDecode::decode_state(decoder)?,
            fdp_selector: save_state::StateDecode::decode_state(decoder)?,
            fpu_opcode: save_state::StateDecode::decode_state(decoder)?,
        })
    }
}

impl Default for X87State {
    fn default() -> Self {
        Self {
            registers: [Fp80::ZERO; 8],
            control_word: 0x037F,
            status_word: 0x0000,
            tag_word: 0xFFFF,
            fip_offset: 0,
            fip_selector: 0,
            fdp_offset: 0,
            fdp_selector: 0,
            fpu_opcode: 0,
        }
    }
}

impl<const CPU_MODEL: u8, const ADDRESS_WIDTH: u8> I386<CPU_MODEL, ADDRESS_WIDTH> {
    pub(super) fn fpu_st_phys(&self, i: u8) -> usize {
        let top = ((self.state.fpu.status_word >> 11) & 7) as u8;
        ((top.wrapping_add(i)) & 7) as usize
    }

    pub(super) fn fpu_st(&self, i: u8) -> Fp80 {
        self.state.fpu.registers[self.fpu_st_phys(i)]
    }

    pub(super) fn fpu_is_empty(&self, i: u8) -> bool {
        let phys = self.fpu_st_phys(i);
        let tag = (self.state.fpu.tag_word >> (phys * 2)) & 3;
        tag == TAG_EMPTY
    }

    pub(super) fn fpu_push(&mut self, value: Fp80) -> bool {
        if !self.fpu_check_unmasked_overflow() {
            return false;
        }
        let top = ((self.state.fpu.status_word >> 11) & 7) as u8;
        let new_top = top.wrapping_sub(1) & 7;
        self.state.fpu.status_word =
            (self.state.fpu.status_word & !0x3800) | ((new_top as u16) << 11);

        let phys = new_top as usize;
        let old_tag = (self.state.fpu.tag_word >> (phys * 2)) & 3;
        if old_tag != TAG_EMPTY {
            // Stack overflow: IE + SF, C1=1
            self.state.fpu.status_word |= 0x0041 | 0x0200; // IE | SF | C1
            self.fpu_update_es();
            self.state.fpu.registers[phys] = Fp80::INDEFINITE;
            self.fpu_set_tag(phys, TAG_SPECIAL);
            return false;
        }

        self.state.fpu.registers[phys] = value;
        self.fpu_set_tag_from_value(phys, &value);
        true
    }

    /// Checks that a push finds ST(7) empty. A stack overflow with the invalid operation
    /// exception unmasked records IE, SF and C1 and returns false: TOP and the registers
    /// keep their values (80486 PRM 16.2.2.1).
    pub(super) fn fpu_check_unmasked_overflow(&mut self) -> bool {
        if self.fpu_is_empty(7) || self.fpu_invalid_masked() {
            return true;
        }
        self.state.fpu.status_word |= 0x0041 | CONDITION_C1;
        self.fpu_update_es();
        false
    }

    pub(super) fn fpu_pop(&mut self) {
        let top = ((self.state.fpu.status_word >> 11) & 7) as u8;
        let phys = top as usize;
        self.fpu_set_tag(phys, TAG_EMPTY);
        let new_top = top.wrapping_add(1) & 7;
        self.state.fpu.status_word =
            (self.state.fpu.status_word & !0x3800) | ((new_top as u16) << 11);
    }

    pub(super) fn fpu_write_st(&mut self, i: u8, value: Fp80) {
        let phys = self.fpu_st_phys(i);
        self.state.fpu.registers[phys] = value;
        self.fpu_set_tag_from_value(phys, &value);
    }

    pub(super) fn fpu_set_tag(&mut self, phys: usize, tag: u16) {
        let shift = phys * 2;
        self.state.fpu.tag_word &= !(3 << shift);
        self.state.fpu.tag_word |= (tag & 3) << shift;
    }

    pub(super) fn fpu_set_tag_from_value(&mut self, phys: usize, value: &Fp80) {
        let tag = match value.classify() {
            FpClass::Zero => TAG_ZERO,
            FpClass::Normal => TAG_VALID,
            _ => TAG_SPECIAL,
        };
        self.fpu_set_tag(phys, tag);
    }

    pub(super) fn fpu_check_underflow(&mut self, i: u8) -> bool {
        if self.fpu_is_empty(i) {
            // Stack underflow: IE + SF, C1=0
            self.state.fpu.status_word |= 0x0041; // IE | SF
            self.state.fpu.status_word &= !0x0200; // C1=0
            self.fpu_update_es();
            return true;
        }
        false
    }

    pub(super) fn fpu_rounding_mode(&self) -> RoundingMode {
        match (self.state.fpu.control_word >> 10) & 3 {
            0 => RoundingMode::NearestEven,
            1 => RoundingMode::Down,
            2 => RoundingMode::Up,
            3 => RoundingMode::Zero,
            _ => unreachable!(),
        }
    }

    pub(super) fn fpu_precision(&self) -> Precision {
        match (self.state.fpu.control_word >> 8) & 3 {
            0 => Precision::Single,
            1 => Precision::Double, // reserved, treat as double
            2 => Precision::Double,
            3 => Precision::Extended,
            _ => unreachable!(),
        }
    }

    pub(super) fn fpu_es_pending(&self) -> bool {
        self.state.fpu.status_word & 0x0080 != 0
    }

    pub(super) fn fpu_update_es(&mut self) {
        let unmasked = (self.state.fpu.status_word & !self.state.fpu.control_word & 0x3F) != 0;
        if unmasked {
            self.state.fpu.status_word |= 0x8080; // ES | B
        } else {
            self.state.fpu.status_word &= !0x8080;
        }
    }

    /// Accumulates the exception flags of an operation into the status word and sets C1 to
    /// the round-up indication of the result (80486 PRM 15.1.2, table 15-1). UE reports a
    /// tiny and inexact result while underflow is masked and every tiny result while it is
    /// unmasked (80486 PRM 16.2.5).
    pub(super) fn fpu_check_result(&mut self, ef: &ExceptionFlags) {
        let mut sw = self.state.fpu.status_word & !0x0200;
        if ef.rounded_up {
            sw |= 0x0200;
        }
        if ef.invalid {
            sw |= 0x01;
        }
        if ef.denormal {
            sw |= 0x02;
        }
        if ef.zero_divide {
            sw |= 0x04;
        }
        if ef.overflow {
            sw |= 0x08;
        }
        let underflow_masked = self.state.fpu.control_word & 0x10 != 0;
        if ef.underflow || (ef.tiny && !underflow_masked) {
            sw |= 0x10;
        }
        if ef.precision {
            sw |= 0x20;
        }
        self.state.fpu.status_word = sw;
        self.fpu_update_es();
    }

    /// Returns whether the invalid operation exception is masked.
    pub(super) fn fpu_invalid_masked(&self) -> bool {
        self.state.fpu.control_word & INVALID_OPERATION != 0
    }

    /// Returns whether the exception with the mask bit `mask` is unmasked.
    fn fpu_unmasked(&self, mask: u16) -> bool {
        self.state.fpu.control_word & mask == 0
    }

    /// ORs the exception flags `flags` into the status word with C1 cleared.
    fn fpu_record_flags(&mut self, flags: u16) {
        self.state.fpu.status_word = (self.state.fpu.status_word & !CONDITION_C1) | flags;
        self.fpu_update_es();
    }

    /// Returns whether `ef` holds an unmasked invalid operation, zero divide or denormal
    /// operand exception. Such an exception leaves the operands and TOP unchanged and the
    /// instruction delivers no result (80486 PRM 16.2.2.1, 16.2.2.2, 16.2.3 and 16.2.4).
    pub(super) fn fpu_operation_suppressed(&self, ef: &ExceptionFlags) -> bool {
        (ef.invalid && self.fpu_unmasked(INVALID_OPERATION))
            || (ef.zero_divide && self.fpu_unmasked(ZERO_DIVIDE))
            || (ef.denormal && self.fpu_unmasked(DENORMAL_OPERAND))
    }

    /// Records the exceptions of an operation that `fpu_operation_suppressed` stopped: IE,
    /// ZE and DE. The lower priority exceptions are not reported (80486 PRM 16.2.7).
    pub(super) fn fpu_record_suppressed(&mut self, ef: &ExceptionFlags) {
        let mut flags = 0;
        if ef.invalid {
            flags |= INVALID_OPERATION;
        }
        if ef.denormal {
            flags |= DENORMAL_OPERAND;
        }
        if ef.zero_divide {
            flags |= ZERO_DIVIDE;
        }
        self.fpu_record_flags(flags);
    }

    /// Records the exceptions of an operation with a register destination and returns the
    /// value to write, or `None` when an unmasked exception suppresses the result. An
    /// unmasked overflow or underflow delivers the biased result with the PE and C1 of its
    /// rounding (80486 PRM 16.2.5.1 and 16.2.5.2).
    pub(super) fn fpu_register_result(
        &mut self,
        ef: &ExceptionFlags,
        result: Fp80,
    ) -> Option<Fp80> {
        if self.fpu_operation_suppressed(ef) {
            self.fpu_record_suppressed(ef);
            return None;
        }
        let unmasked_range = (ef.overflow && self.fpu_unmasked(OVERFLOW))
            || (ef.tiny && self.fpu_unmasked(UNDERFLOW));
        if unmasked_range && let Some(biased) = ef.biased {
            let mut flags = *ef;
            flags.precision = biased.precision;
            flags.rounded_up = biased.rounded_up;
            self.fpu_check_result(&flags);
            return Some(biased.value);
        }
        self.fpu_check_result(ef);
        Some(result)
    }

    /// Returns whether an unmasked exception of `ef` suppresses a store to memory: an
    /// invalid operation, a denormal source, an overflow or a tiny result. No result is
    /// stored and the operand stays on the stack (80486 PRM 16.2.2.2, 16.2.4, 16.2.5.1 and
    /// 16.2.5.2).
    pub(super) fn fpu_store_suppressed(&self, ef: &ExceptionFlags) -> bool {
        (ef.invalid && self.fpu_unmasked(INVALID_OPERATION))
            || (ef.denormal && self.fpu_unmasked(DENORMAL_OPERAND))
            || (ef.overflow && self.fpu_unmasked(OVERFLOW))
            || (ef.tiny && self.fpu_unmasked(UNDERFLOW))
    }

    /// Records the exceptions of a store that `fpu_store_suppressed` stopped. An unmasked
    /// denormal source reports DE alone, as the lower priority exceptions are not reported
    /// (80486 PRM 16.2.7). Otherwise IE, OE and UE are reported together with a masked DE
    /// (80486 PRM 16.2.7, item 4): no result is stored and the operand stays on the stack
    /// (80486 PRM 16.2.5.1 and 16.2.5.2). Observed behaviour: PE and C1 stay clear.
    pub(super) fn fpu_record_store_suppressed(&mut self, ef: &ExceptionFlags) {
        if ef.denormal && self.fpu_unmasked(DENORMAL_OPERAND) {
            self.fpu_record_flags(DENORMAL_OPERAND);
            return;
        }
        let mut flags = 0;
        if ef.invalid {
            flags |= INVALID_OPERATION;
        }
        if ef.denormal {
            flags |= DENORMAL_OPERAND;
        }
        if ef.overflow {
            flags |= OVERFLOW;
        }
        if ef.tiny && self.fpu_unmasked(UNDERFLOW) {
            flags |= UNDERFLOW;
        }
        self.fpu_record_flags(flags);
    }

    pub(super) fn fpu_raise_exception(&mut self, bus: &mut impl common::Bus) -> Step {
        if self.state.cr0 & 0x20 != 0 {
            // NE=1: native #MF (vector 16)
            self.raise_fault(16, bus)?;
        } else {
            // NE=0: DOS-compatible, signal via FERR#
            bus.signal_fpu_error();
        }
        Ok(())
    }

    /// Records the instruction pointer and, for a memory operand, the data pointer of the
    /// current escape instruction as offsets with their selectors (80486 PRM 15.1.5).
    pub(super) fn fpu_update_pointers(&mut self, esc_bits: u8, modrm: u8, has_memory: bool) {
        self.state.fpu.fip_offset = self.prev_ip_upper | self.prev_ip as u32;
        self.state.fpu.fip_selector = self.sregs[crate::SegReg32::CS as usize];
        self.state.fpu.fpu_opcode = ((esc_bits as u16 & 7) << 8) | modrm as u16;

        if has_memory {
            self.state.fpu.fdp_offset = self.eo32;
            self.state.fpu.fdp_selector = self.sregs[self.ea_seg as usize];
        }
    }

    /// Checks the x87 memory operand of `size` bytes at the effective address against the
    /// segment limit and, in protected mode, the access rights.
    pub(super) fn fpu_check_operand(
        &mut self,
        size: u32,
        write: bool,
        bus: &mut impl common::Bus,
    ) -> Step {
        self.check_segment_access(self.ea_seg, self.eo32, size, write, bus)
    }

    pub(super) fn fpu_read_u16(&mut self, bus: &mut impl common::Bus) -> Step<u16> {
        self.fpu_check_operand(2, false, bus)?;
        self.read_word_linear(bus, self.ea)
    }

    pub(super) fn fpu_read_u32(&mut self, bus: &mut impl common::Bus) -> Step<u32> {
        self.fpu_check_operand(4, false, bus)?;
        let ea = self.ea;
        let lo = self.read_word_linear(bus, ea)? as u32;
        let hi = self.read_word_linear(bus, ea.wrapping_add(2))? as u32;
        Ok(lo | (hi << 16))
    }

    pub(super) fn fpu_read_u64(&mut self, bus: &mut impl common::Bus) -> Step<u64> {
        self.fpu_check_operand(8, false, bus)?;
        let ea = self.ea;
        let w0 = self.read_word_linear(bus, ea)? as u64;
        let w1 = self.read_word_linear(bus, ea.wrapping_add(2))? as u64;
        let w2 = self.read_word_linear(bus, ea.wrapping_add(4))? as u64;
        let w3 = self.read_word_linear(bus, ea.wrapping_add(6))? as u64;
        Ok(w0 | (w1 << 16) | (w2 << 32) | (w3 << 48))
    }

    pub(super) fn fpu_read_tbyte(&mut self, bus: &mut impl common::Bus) -> Step<[u8; 10]> {
        self.fpu_check_operand(10, false, bus)?;
        let ea = self.ea;
        // Translate every byte first so a partial fault doesn't mutate the
        // FPU stack via fpu_push later.
        let mut phys = [0u32; 10];
        for (i, slot) in phys.iter_mut().enumerate() {
            *slot = self.translate_linear(ea.wrapping_add(i as u32), false, bus)?;
        }
        let mut bytes = [0u8; 10];
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = bus.read_byte(phys[i]);
        }
        Ok(bytes)
    }

    pub(super) fn fpu_write_u16(&mut self, bus: &mut impl common::Bus, value: u16) -> Step {
        self.fpu_check_operand(2, true, bus)?;
        let ea = self.ea;
        self.write_word_linear(bus, ea, value)
    }

    pub(super) fn fpu_write_u32(&mut self, bus: &mut impl common::Bus, value: u32) -> Step {
        self.fpu_check_operand(4, true, bus)?;
        let ea = self.ea;
        self.write_word_linear(bus, ea, value as u16)?;
        self.write_word_linear(bus, ea.wrapping_add(2), (value >> 16) as u16)
    }

    pub(super) fn fpu_write_u64(&mut self, bus: &mut impl common::Bus, value: u64) -> Step {
        self.fpu_check_operand(8, true, bus)?;
        let ea = self.ea;
        self.write_word_linear(bus, ea, value as u16)?;
        self.write_word_linear(bus, ea.wrapping_add(2), (value >> 16) as u16)?;
        self.write_word_linear(bus, ea.wrapping_add(4), (value >> 32) as u16)?;
        self.write_word_linear(bus, ea.wrapping_add(6), (value >> 48) as u16)
    }

    pub(super) fn fpu_write_tbyte(&mut self, bus: &mut impl common::Bus, bytes: &[u8; 10]) -> Step {
        self.fpu_check_operand(10, true, bus)?;
        let ea = self.ea;
        // Translate every byte first; only commit writes once all
        // translations succeed so a fault leaves the destination
        // memory unchanged.
        let mut phys = [0u32; 10];
        for (i, slot) in phys.iter_mut().enumerate() {
            *slot = self.translate_linear(ea.wrapping_add(i as u32), true, bus)?;
        }
        for (i, &byte) in bytes.iter().enumerate() {
            bus.write_byte(phys[i], byte);
        }
        Ok(())
    }

    pub(super) fn fpu_init(&mut self) {
        self.state.fpu.control_word = 0x037F;
        self.state.fpu.status_word = 0x0000;
        self.state.fpu.tag_word = 0xFFFF;
        self.state.fpu.fip_offset = 0;
        self.state.fpu.fip_selector = 0;
        self.state.fpu.fdp_offset = 0;
        self.state.fpu.fdp_selector = 0;
        self.state.fpu.fpu_opcode = 0;
    }

    /// Clears C1 for an instruction whose C1 reports only a stack overflow (80486 PRM
    /// 15.1.2, table 15-1, "Zero or O/U#").
    pub(super) fn fpu_clear_c1(&mut self) {
        self.state.fpu.status_word &= !0x0200;
    }

    /// Clears the C2 condition code bit.
    pub(super) fn fpu_clear_c2(&mut self) {
        self.state.fpu.status_word &= !0x0400;
    }

    pub(super) fn fpu_set_cc(&mut self, c3: bool, c2: bool, c1: bool, c0: bool) {
        let mut sw = self.state.fpu.status_word & !0x4700; // clear C3, C2, C1, C0
        if c0 {
            sw |= 0x0100;
        } // C0 = bit 8
        if c1 {
            sw |= 0x0200;
        } // C1 = bit 9
        if c2 {
            sw |= 0x0400;
        } // C2 = bit 10
        if c3 {
            sw |= 0x4000;
        } // C3 = bit 14
        self.state.fpu.status_word = sw;
    }
}
