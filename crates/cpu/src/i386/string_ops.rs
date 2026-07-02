use super::{Fault, I386, Step};
use crate::{ByteReg, DwordReg, SegReg32, WordReg};

type ProbedDwordWrite = (u32, Option<(u32, u32, u32)>);

impl<const CPU_MODEL: u8, const ADDRESS_WIDTH: u8> I386<CPU_MODEL, ADDRESS_WIDTH> {
    #[inline(always)]
    fn string_index_si(&self) -> u32 {
        if self.address_size_override {
            self.regs.dword(DwordReg::ESI)
        } else {
            self.regs.word(WordReg::SI) as u32
        }
    }

    #[inline(always)]
    fn string_index_di(&self) -> u32 {
        if self.address_size_override {
            self.regs.dword(DwordReg::EDI)
        } else {
            self.regs.word(WordReg::DI) as u32
        }
    }

    #[inline(always)]
    fn string_set_si(&mut self, value: u32) {
        if self.address_size_override {
            self.regs.set_dword(DwordReg::ESI, value);
        } else {
            self.regs.set_word(WordReg::SI, value as u16);
        }
    }

    #[inline(always)]
    fn string_set_di(&mut self, value: u32) {
        if self.address_size_override {
            self.regs.set_dword(DwordReg::EDI, value);
        } else {
            self.regs.set_word(WordReg::DI, value as u16);
        }
    }

    #[inline(always)]
    fn string_advance_si(&mut self, bytes: u32) {
        let si = self.string_index_si();
        let next = if self.flags.df {
            si.wrapping_sub(bytes)
        } else {
            si.wrapping_add(bytes)
        };
        self.string_set_si(next);
    }

    #[inline(always)]
    fn string_advance_di(&mut self, bytes: u32) {
        let di = self.string_index_di();
        let next = if self.flags.df {
            di.wrapping_sub(bytes)
        } else {
            di.wrapping_add(bytes)
        };
        self.string_set_di(next);
    }

    #[inline(always)]
    fn string_addr(&self, base: u32, offset: u32) -> u32 {
        self.string_addr_delta(base, offset, 0)
    }

    #[inline(always)]
    fn string_addr_delta(&self, base: u32, offset: u32, delta: u8) -> u32 {
        let effective_offset = if self.address_size_override {
            offset.wrapping_add(delta as u32)
        } else {
            (offset as u16).wrapping_add(delta as u16) as u32
        };
        base.wrapping_add(effective_offset)
    }

    #[inline(always)]
    fn string_read_word(
        &mut self,
        bus: &mut impl common::Bus,
        base: u32,
        offset: u32,
    ) -> Step<u16> {
        let l0 = self.string_addr_delta(base, offset, 0);
        let same_page = if self.address_size_override {
            l0 & 0xFFF <= 0xFFE
        } else {
            l0 & 0xFFF <= 0xFFE && (offset as u16) <= 0xFFFE
        };
        if same_page {
            let a0 = self.translate_linear(l0, false, bus)?;
            return Ok(bus.read_word(a0));
        }
        let l1 = self.string_addr_delta(base, offset, 1);
        let a0 = self.translate_linear(l0, false, bus)?;
        let a1 = self.translate_linear(l1, false, bus)?;
        Ok(bus.read_byte(a0) as u16 | ((bus.read_byte(a1) as u16) << 8))
    }

    #[inline(always)]
    fn string_write_word(
        &mut self,
        bus: &mut impl common::Bus,
        base: u32,
        offset: u32,
        value: u16,
    ) -> bool {
        let Ok((a0, cross)) = self.string_probe_write_word(bus, base, offset) else {
            return false;
        };
        match cross {
            None => bus.write_word(a0, value),
            Some(a1) => {
                bus.write_byte(a0, value as u8);
                bus.write_byte(a1, (value >> 8) as u8);
            }
        }
        true
    }

    /// Translates a word-sized destination for a string write without
    /// performing the write. On the contiguous fast path returns
    /// `Some((a0, None))`; on a cross-page split returns
    /// `Some((a0, Some(a1)))`. Returns `None` if either translation
    /// faulted (the fault is already raised).
    #[inline(always)]
    fn string_probe_write_word(
        &mut self,
        bus: &mut impl common::Bus,
        base: u32,
        offset: u32,
    ) -> Step<(u32, Option<u32>)> {
        let l0 = self.string_addr_delta(base, offset, 0);
        let same_page = if self.address_size_override {
            l0 & 0xFFF <= 0xFFE
        } else {
            l0 & 0xFFF <= 0xFFE && (offset as u16) <= 0xFFFE
        };
        if same_page {
            let a0 = self.translate_linear(l0, true, bus)?;
            return Ok((a0, None));
        }
        let l1 = self.string_addr_delta(base, offset, 1);
        let a0 = self.translate_linear(l0, true, bus)?;
        let a1 = self.translate_linear(l1, true, bus)?;
        Ok((a0, Some(a1)))
    }

    #[inline(always)]
    fn string_read_dword(
        &mut self,
        bus: &mut impl common::Bus,
        base: u32,
        offset: u32,
    ) -> Step<u32> {
        let l0 = self.string_addr_delta(base, offset, 0);
        let same_page = if self.address_size_override {
            l0 & 0xFFF <= 0xFFC
        } else {
            l0 & 0xFFF <= 0xFFC && (offset as u16) <= 0xFFFC
        };
        if same_page {
            let a0 = self.translate_linear(l0, false, bus)?;
            return Ok(bus.read_dword(a0));
        }
        let l1 = self.string_addr_delta(base, offset, 1);
        let l2 = self.string_addr_delta(base, offset, 2);
        let l3 = self.string_addr_delta(base, offset, 3);
        let a0 = self.translate_linear(l0, false, bus)?;
        let a1 = self.translate_linear(l1, false, bus)?;
        let a2 = self.translate_linear(l2, false, bus)?;
        let a3 = self.translate_linear(l3, false, bus)?;
        Ok(bus.read_byte(a0) as u32
            | ((bus.read_byte(a1) as u32) << 8)
            | ((bus.read_byte(a2) as u32) << 16)
            | ((bus.read_byte(a3) as u32) << 24))
    }

    #[inline(always)]
    fn string_write_dword(
        &mut self,
        bus: &mut impl common::Bus,
        base: u32,
        offset: u32,
        value: u32,
    ) -> bool {
        let Ok((a0, cross)) = self.string_probe_write_dword(bus, base, offset) else {
            return false;
        };
        match cross {
            None => bus.write_dword(a0, value),
            Some((a1, a2, a3)) => {
                bus.write_byte(a0, value as u8);
                bus.write_byte(a1, (value >> 8) as u8);
                bus.write_byte(a2, (value >> 16) as u8);
                bus.write_byte(a3, (value >> 24) as u8);
            }
        }
        true
    }

    /// Translates a dword-sized destination for a string write without
    /// performing the write. See [`string_probe_write_word`].
    #[inline(always)]
    fn string_probe_write_dword(
        &mut self,
        bus: &mut impl common::Bus,
        base: u32,
        offset: u32,
    ) -> Step<ProbedDwordWrite> {
        let l0 = self.string_addr_delta(base, offset, 0);
        let same_page = if self.address_size_override {
            l0 & 0xFFF <= 0xFFC
        } else {
            l0 & 0xFFF <= 0xFFC && (offset as u16) <= 0xFFFC
        };
        if same_page {
            let a0 = self.translate_linear(l0, true, bus)?;
            return Ok((a0, None));
        }
        let l1 = self.string_addr_delta(base, offset, 1);
        let l2 = self.string_addr_delta(base, offset, 2);
        let l3 = self.string_addr_delta(base, offset, 3);
        let a0 = self.translate_linear(l0, true, bus)?;
        let a1 = self.translate_linear(l1, true, bus)?;
        let a2 = self.translate_linear(l2, true, bus)?;
        let a3 = self.translate_linear(l3, true, bus)?;
        Ok((a0, Some((a1, a2, a3))))
    }

    pub(super) fn movsb(&mut self, bus: &mut impl common::Bus) -> Step {
        let si = self.string_index_si();
        let di = self.string_index_di();
        let src_seg = self.default_seg(SegReg32::DS);
        self.check_segment_access(src_seg, si, 1, false, bus)?;
        self.check_segment_access(SegReg32::ES, di, 1, true, bus)?;
        let src_linear = self.string_addr(self.seg_base(src_seg), si);
        let dst_linear = self.string_addr(self.seg_base(SegReg32::ES), di);
        let src_phys = self.translate_linear(src_linear, false, bus)?;
        let val = bus.read_byte(src_phys);
        let dst_phys = self.translate_linear(dst_linear, true, bus)?;
        bus.write_byte(dst_phys, val);
        self.string_advance_si(1);
        self.string_advance_di(1);
        self.clk(Self::timing(7, 7));
        Ok(())
    }

    pub(super) fn movsw(&mut self, bus: &mut impl common::Bus) -> Step {
        let si = self.string_index_si();
        let di = self.string_index_di();
        let src_seg = self.default_seg(SegReg32::DS);
        let access_size = if self.operand_size_override { 4 } else { 2 };
        self.check_segment_access(src_seg, si, access_size, false, bus)?;
        self.check_segment_access(SegReg32::ES, di, access_size, true, bus)?;
        let src_base = self.seg_base(src_seg);
        let dst_base = self.seg_base(SegReg32::ES);
        if self.operand_size_override {
            let val = self.string_read_dword(bus, src_base, si)?;
            if !self.string_write_dword(bus, dst_base, di, val) {
                return Ok(());
            }
            self.string_advance_si(4);
            self.string_advance_di(4);
        } else {
            let val = self.string_read_word(bus, src_base, si)?;
            if !self.string_write_word(bus, dst_base, di, val) {
                return Ok(());
            }
            self.string_advance_si(2);
            self.string_advance_di(2);
        }
        let misaligned = if self.operand_size_override {
            si & 3 != 0 || di & 3 != 0
        } else {
            si & 1 != 0 || di & 1 != 0
        };
        let penalty = if misaligned { Self::timing(4, 3) } else { 0 };
        self.clk(Self::timing(7, 7) + penalty);
        Ok(())
    }

    pub(super) fn cmpsb(&mut self, bus: &mut impl common::Bus) -> Step {
        let si = self.string_index_si();
        let di = self.string_index_di();
        let src_seg = self.default_seg(SegReg32::DS);
        self.check_segment_access(src_seg, si, 1, false, bus)?;
        self.check_segment_access(SegReg32::ES, di, 1, false, bus)?;
        let src_linear = self.string_addr(self.seg_base(src_seg), si);
        let dst_linear = self.string_addr(self.seg_base(SegReg32::ES), di);
        let src_phys = self.translate_linear(src_linear, false, bus)?;
        let dst_phys = self.translate_linear(dst_linear, false, bus)?;
        let src = bus.read_byte(src_phys);
        let dst = bus.read_byte(dst_phys);
        self.alu_sub_byte(src, dst);
        self.string_advance_si(1);
        self.string_advance_di(1);
        self.clk(Self::timing(10, 8));
        Ok(())
    }

    pub(super) fn cmpsw(&mut self, bus: &mut impl common::Bus) -> Step {
        let si = self.string_index_si();
        let di = self.string_index_di();
        let src_seg = self.default_seg(SegReg32::DS);
        let access_size = if self.operand_size_override { 4 } else { 2 };
        self.check_segment_access(src_seg, si, access_size, false, bus)?;
        self.check_segment_access(SegReg32::ES, di, access_size, false, bus)?;
        let src_base = self.seg_base(src_seg);
        let dst_base = self.seg_base(SegReg32::ES);
        if self.operand_size_override {
            let src = self.string_read_dword(bus, src_base, si)?;
            let dst = self.string_read_dword(bus, dst_base, di)?;
            self.alu_sub_dword(src, dst);
            self.string_advance_si(4);
            self.string_advance_di(4);
        } else {
            let src = self.string_read_word(bus, src_base, si)?;
            let dst = self.string_read_word(bus, dst_base, di)?;
            self.alu_sub_word(src, dst);
            self.string_advance_si(2);
            self.string_advance_di(2);
        }
        let misaligned = if self.operand_size_override {
            si & 3 != 0 || di & 3 != 0
        } else {
            si & 1 != 0 || di & 1 != 0
        };
        let penalty = if misaligned { Self::timing(4, 3) } else { 0 };
        self.clk(Self::timing(10, 8) + penalty);
        Ok(())
    }

    pub(super) fn stosb(&mut self, bus: &mut impl common::Bus) -> Step {
        let di = self.string_index_di();
        self.check_segment_access(SegReg32::ES, di, 1, true, bus)?;
        let linear = self.string_addr(self.seg_base(SegReg32::ES), di);
        let al = self.regs.byte(ByteReg::AL);
        let addr = self.translate_linear(linear, true, bus)?;
        bus.write_byte(addr, al);
        self.string_advance_di(1);
        self.clk(Self::timing(4, 5));
        Ok(())
    }

    pub(super) fn stosw(&mut self, bus: &mut impl common::Bus) -> Step {
        let di = self.string_index_di();
        let access_size = if self.operand_size_override { 4 } else { 2 };
        self.check_segment_access(SegReg32::ES, di, access_size, true, bus)?;
        let base = self.seg_base(SegReg32::ES);
        if self.operand_size_override {
            if !self.string_write_dword(bus, base, di, self.regs.dword(DwordReg::EAX)) {
                return Ok(());
            }
            self.string_advance_di(4);
        } else {
            if !self.string_write_word(bus, base, di, self.regs.word(WordReg::AX)) {
                return Ok(());
            }
            self.string_advance_di(2);
        }
        let misaligned = if self.operand_size_override {
            di & 3 != 0
        } else {
            di & 1 != 0
        };
        let penalty = if misaligned { Self::timing(4, 3) } else { 0 };
        self.clk(Self::timing(4, 5) + penalty);
        Ok(())
    }

    pub(super) fn lodsb(&mut self, bus: &mut impl common::Bus) -> Step {
        let si = self.string_index_si();
        let src_seg = self.default_seg(SegReg32::DS);
        self.check_segment_access(src_seg, si, 1, false, bus)?;
        let linear = self.string_addr(self.seg_base(src_seg), si);
        let addr = self.translate_linear(linear, false, bus)?;
        let val = bus.read_byte(addr);
        self.regs.set_byte(ByteReg::AL, val);
        self.string_advance_si(1);
        self.clk(Self::timing(5, 5));
        Ok(())
    }

    pub(super) fn lodsw(&mut self, bus: &mut impl common::Bus) -> Step {
        let si = self.string_index_si();
        let src_seg = self.default_seg(SegReg32::DS);
        let access_size = if self.operand_size_override { 4 } else { 2 };
        self.check_segment_access(src_seg, si, access_size, false, bus)?;
        let base = self.seg_base(src_seg);
        if self.operand_size_override {
            let val = self.string_read_dword(bus, base, si)?;
            self.regs.set_dword(DwordReg::EAX, val);
            self.string_advance_si(4);
        } else {
            let val = self.string_read_word(bus, base, si)?;
            self.regs.set_word(WordReg::AX, val);
            self.string_advance_si(2);
        }
        let misaligned = if self.operand_size_override {
            si & 3 != 0
        } else {
            si & 1 != 0
        };
        let penalty = if misaligned { Self::timing(4, 3) } else { 0 };
        self.clk(Self::timing(5, 5) + penalty);
        Ok(())
    }

    pub(super) fn scasb(&mut self, bus: &mut impl common::Bus) -> Step {
        let di = self.string_index_di();
        self.check_segment_access(SegReg32::ES, di, 1, false, bus)?;
        let linear = self.string_addr(self.seg_base(SegReg32::ES), di);
        let addr = self.translate_linear(linear, false, bus)?;
        let dst = bus.read_byte(addr);
        let al = self.regs.byte(ByteReg::AL);
        self.alu_sub_byte(al, dst);
        self.string_advance_di(1);
        self.clk(Self::timing(7, 6));
        Ok(())
    }

    pub(super) fn scasw(&mut self, bus: &mut impl common::Bus) -> Step {
        let di = self.string_index_di();
        let access_size = if self.operand_size_override { 4 } else { 2 };
        self.check_segment_access(SegReg32::ES, di, access_size, false, bus)?;
        let base = self.seg_base(SegReg32::ES);
        if self.operand_size_override {
            let dst = self.string_read_dword(bus, base, di)?;
            let eax = self.regs.dword(DwordReg::EAX);
            self.alu_sub_dword(eax, dst);
            self.string_advance_di(4);
        } else {
            let dst = self.string_read_word(bus, base, di)?;
            let aw = self.regs.word(WordReg::AX);
            self.alu_sub_word(aw, dst);
            self.string_advance_di(2);
        }
        let misaligned = if self.operand_size_override {
            di & 3 != 0
        } else {
            di & 1 != 0
        };
        let penalty = if misaligned { Self::timing(4, 3) } else { 0 };
        self.clk(Self::timing(7, 6) + penalty);
        Ok(())
    }

    pub(super) fn insb(&mut self, bus: &mut impl common::Bus) -> Step {
        let port = self.regs.word(WordReg::DX);
        if self.check_io_privilege(port, 1, bus).is_err() {
            return Err(Fault);
        }
        let di = self.string_index_di();
        self.check_segment_access(SegReg32::ES, di, 1, true, bus)?;
        let linear = self.string_addr(self.seg_base(SegReg32::ES), di);
        let addr = self.translate_linear(linear, true, bus)?;
        let val = bus.io_read_byte(port);
        bus.write_byte(addr, val);
        self.string_advance_di(1);
        self.clk(Self::timing(15, 17));
        Ok(())
    }

    pub(super) fn insw(&mut self, bus: &mut impl common::Bus) -> Step {
        let port = self.regs.word(WordReg::DX);
        let size = if self.operand_size_override { 4 } else { 2 };
        if self.check_io_privilege(port, size, bus).is_err() {
            return Err(Fault);
        }
        let di = self.string_index_di();
        self.check_segment_access(SegReg32::ES, di, size as u32, true, bus)?;
        let base = self.seg_base(SegReg32::ES);
        if self.operand_size_override {
            let Ok((a0, cross)) = self.string_probe_write_dword(bus, base, di) else {
                return Ok(());
            };
            let low = bus.io_read_word(port) as u32;
            let high = bus.io_read_word(port.wrapping_add(2)) as u32;
            let val = low | (high << 16);
            match cross {
                None => bus.write_dword(a0, val),
                Some((a1, a2, a3)) => {
                    bus.write_byte(a0, val as u8);
                    bus.write_byte(a1, (val >> 8) as u8);
                    bus.write_byte(a2, (val >> 16) as u8);
                    bus.write_byte(a3, (val >> 24) as u8);
                }
            }
            self.string_advance_di(4);
        } else {
            let Ok((a0, cross)) = self.string_probe_write_word(bus, base, di) else {
                return Ok(());
            };
            let val = bus.io_read_word(port);
            match cross {
                None => bus.write_word(a0, val),
                Some(a1) => {
                    bus.write_byte(a0, val as u8);
                    bus.write_byte(a1, (val >> 8) as u8);
                }
            }
            self.string_advance_di(2);
        }
        let misaligned = if self.operand_size_override {
            di & 3 != 0
        } else {
            di & 1 != 0
        };
        let penalty = if misaligned { Self::timing(4, 3) } else { 0 };
        self.clk(Self::timing(15, 17) + penalty);
        Ok(())
    }

    pub(super) fn outsb(&mut self, bus: &mut impl common::Bus) -> Step {
        let port = self.regs.word(WordReg::DX);
        if self.check_io_privilege(port, 1, bus).is_err() {
            return Err(Fault);
        }
        let si = self.string_index_si();
        let src_seg = self.default_seg(SegReg32::DS);
        self.check_segment_access(src_seg, si, 1, false, bus)?;
        let linear = self.string_addr(self.seg_base(src_seg), si);
        let addr = self.translate_linear(linear, false, bus)?;
        let val = bus.read_byte(addr);
        bus.io_write_byte(port, val);
        self.string_advance_si(1);
        self.clk(Self::timing(14, 17));
        Ok(())
    }

    pub(super) fn outsw(&mut self, bus: &mut impl common::Bus) -> Step {
        let port = self.regs.word(WordReg::DX);
        let size = if self.operand_size_override { 4 } else { 2 };
        if self.check_io_privilege(port, size, bus).is_err() {
            return Err(Fault);
        }
        let si = self.string_index_si();
        let src_seg = self.default_seg(SegReg32::DS);
        self.check_segment_access(src_seg, si, size as u32, false, bus)?;
        let base = self.seg_base(src_seg);
        if self.operand_size_override {
            let val = self.string_read_dword(bus, base, si)?;
            bus.io_write_word(port, val as u16);
            bus.io_write_word(port.wrapping_add(2), (val >> 16) as u16);
            self.string_advance_si(4);
        } else {
            let val = self.string_read_word(bus, base, si)?;
            bus.io_write_word(port, val);
            self.string_advance_si(2);
        }
        let misaligned = if self.operand_size_override {
            si & 3 != 0
        } else {
            si & 1 != 0
        };
        let penalty = if misaligned { Self::timing(4, 3) } else { 0 };
        self.clk(Self::timing(14, 17) + penalty);
        Ok(())
    }
}
