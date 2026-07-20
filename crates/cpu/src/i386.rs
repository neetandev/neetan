//! Implements the Intel 80386+ family emulation.
//!
//! The CPU model is selected via the const generic parameter `CPU_MODEL`:
//! - [`CPU_MODEL_386_DX`] - Intel 80386DX cycle timings.
//! - [`CPU_MODEL_486_DX`] - Intel 80486DX cycle timings.
//! - [`CPU_MODEL_386_SX`] - Intel 80386SX cycle timings.
//!
//! The physical address bus width is selected via the const generic parameter
//! `ADDRESS_WIDTH`:
//! - [`ADDRESS_WIDTH_24`] - 24 external address lines (PC-98 wiring).
//! - [`ADDRESS_WIDTH_32`] - full 32-bit address bus (FM TOWNS wiring).
//!
//! Following references were used to write the emulator:
//!
//! - Intel Corporation, "80386 Programmer's Reference Manual".
//! - Intel Corporation, "i486 Processor Programmer's Reference Manual".

mod alu;
mod execute;
mod execute_0f;
mod execute_group;
mod flags;
mod fpu;
mod fpu_dispatch;
mod fpu_ops;
mod interrupt;
mod modrm;
mod paging;
mod rep;
mod state;
mod string_ops;

use core::ops::{Deref, DerefMut};

use common::{Cpu as _, likely, unlikely};
pub use flags::I386Flags;
pub use state::I386State;

use crate::{SegReg32, WordReg};

/// CPU model constant for Intel 80386DX.
pub const CPU_MODEL_386_DX: u8 = 0;
/// CPU model constant for Intel 80486DX.
pub const CPU_MODEL_486_DX: u8 = 1;
/// CPU model constant for Intel 80386SX.
pub const CPU_MODEL_386_SX: u8 = 2;

/// Physical address width constant for a 24-bit address bus (PC-98 wiring).
pub const ADDRESS_WIDTH_24: u8 = 24;
/// Physical address width constant for a full 32-bit address bus (FM TOWNS wiring).
pub const ADDRESS_WIDTH_32: u8 = 32;

const EFLAGS_RESUME_FLAG: u32 = 0x0001_0000;
const EFLAGS_VIRTUAL_8086_FLAG: u32 = 0x0002_0000;
const EFLAGS_ALIGNMENT_CHECK_FLAG: u32 = 0x0004_0000;

/// EDX after RESET holds the component identifier and revision (386 PRM 10.1):
/// DH = 3 for the 386DX, DL = revision. 0x0308 is the D-step.
const RESET_EDX_386DX: u32 = 0x0000_0308;
/// EDX after RESET on the 486 (i486 PRM 10.2): 0x0435 identifies an i486DX2-66.
const RESET_EDX_486DX2: u32 = 0x0000_0435;
/// EDX after RESET on the 386SX: DH = 0x23 is the 386SX component identifier.
const RESET_EDX_386SX: u32 = 0x0000_2308;

/// Extra clocks on the 386SX for an aligned dword transfer, which the 16-bit
/// data bus splits into two bus cycles. Calibration constant.
const SX_EXTRA_CLOCKS_DWORD_ACCESS: i32 = 2;
/// Extra clocks on the 386SX for a misaligned word transfer, which splits
/// into two bus cycles. Calibration constant.
const SX_EXTRA_CLOCKS_MISALIGNED_WORD: i32 = 2;
/// Extra clocks on the 386SX for a misaligned dword transfer, which takes
/// three bus cycles instead of one. Calibration constant.
const SX_EXTRA_CLOCKS_MISALIGNED_DWORD: i32 = 4;
/// Extra clocks on the 386SX per 16-bit code word fetched, approximating the
/// halved code fetch bandwidth without a prefetch queue model. Calibration
/// constant.
const SX_EXTRA_CLOCKS_PER_CODE_WORD: i32 = 1;

/// Marker returned from any 386 primitive that may have raised a CPU exception.
///
/// The exception itself is already delivered into CPU state by `raise_fault*`;
/// `Fault` is pure call-stack signalling so callers can propagate the abort
/// via the `?` operator instead of inspecting `fault_pending` after every call.
pub(super) struct Fault;

/// Result alias for any 386 operation that may abort due to a raised fault.
pub(super) type Step<T = ()> = Result<T, Fault>;

#[derive(Clone, Copy)]
struct SegmentDescriptor {
    base: u32,
    limit: u32,
    rights: u8,
    granularity: u8,
}

#[derive(Clone, Copy, PartialEq)]
enum TaskType {
    Iret,
    Jmp,
    Call,
}

#[derive(Clone, Copy)]
struct CsReturnValidation {
    descriptor: SegmentDescriptor,
    adjusted_selector: u16,
}

#[derive(Clone, Copy)]
struct SsReturnValidation {
    descriptor: SegmentDescriptor,
}

#[derive(Clone, Copy)]
enum DataSegmentDecision {
    NoChange,
    Null {
        selector: u16,
    },
    Keep {
        selector: u16,
        descriptor: SegmentDescriptor,
    },
}

/// Intel 80386+ CPU emulator.
///
/// The const generic `CPU_MODEL` selects the instruction timings and feature set.
/// Use [`CPU_MODEL_386_DX`] for an 80386DX, [`CPU_MODEL_486_DX`] for an 80486DX or
/// [`CPU_MODEL_386_SX`] for an 80386SX (386DX behavior with 16-bit bus timing
/// penalties).
///
/// The const generic `ADDRESS_WIDTH` selects the physical address bus width.
/// Use [`ADDRESS_WIDTH_24`] for the PC-98 wiring (physical addresses are
/// truncated to 24 bits and reset lands at 000FFFF0) or [`ADDRESS_WIDTH_32`]
/// for machines with a full 32-bit address bus (reset lands at FFFFFFF0).
pub struct I386<
    const CPU_MODEL: u8 = { CPU_MODEL_386_DX },
    const ADDRESS_WIDTH: u8 = { ADDRESS_WIDTH_24 },
> {
    /// Embedded state for save/restore.
    pub state: I386State,
    cycles_remaining: i64,
    run_start_cycle: u64,
    run_budget: u64,
}

impl<const CPU_MODEL: u8, const ADDRESS_WIDTH: u8> Deref for I386<CPU_MODEL, ADDRESS_WIDTH> {
    type Target = I386State;
    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl<const CPU_MODEL: u8, const ADDRESS_WIDTH: u8> DerefMut for I386<CPU_MODEL, ADDRESS_WIDTH> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

impl<const CPU_MODEL: u8, const ADDRESS_WIDTH: u8> Default for I386<CPU_MODEL, ADDRESS_WIDTH> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const CPU_MODEL: u8, const ADDRESS_WIDTH: u8> I386<CPU_MODEL, ADDRESS_WIDTH> {
    /// Creates a new I386 CPU in its reset state.
    pub fn new() -> Self {
        let mut cpu = Self {
            state: I386State::default(),
            cycles_remaining: 0,
            run_start_cycle: 0,
            run_budget: 0,
        };
        cpu.reset();
        cpu
    }

    /// Selects between 386 and 486 cycle counts at compile time. The 386SX
    /// shares the 386DX execution timings; its bus penalties are charged
    /// separately.
    #[inline(always)]
    const fn timing(t386: i32, t486: i32) -> i32 {
        match CPU_MODEL {
            CPU_MODEL_386_DX | CPU_MODEL_386_SX => t386,
            CPU_MODEL_486_DX => t486,
            _ => unreachable!(),
        }
    }

    /// Like [`timing`], but lets the 386SX use a distinct execute-cycle count
    /// `tsx`. Non-SX models delegate to `timing(t386, t486)`, so the 386DX and
    /// 486DX are unaffected by construction. Keep `t386`/`t486` identical to
    /// the site's original `timing` call so only the SX diverges.
    #[inline(always)]
    const fn timing_sx(t386: i32, t486: i32, tsx: i32) -> i32 {
        match CPU_MODEL {
            CPU_MODEL_386_SX => tsx,
            _ => Self::timing(t386, t486),
        }
    }

    /// Misaligned-operand penalty charged at decode time: 4 on the 386DX,
    /// 3 on the 486. Zero on the 386SX, whose alignment cost is charged per
    /// bus transfer in `clk_sx_bus` instead.
    #[inline(always)]
    const fn misaligned_operand_penalty() -> i32 {
        match CPU_MODEL {
            CPU_MODEL_386_DX => 4,
            CPU_MODEL_486_DX => 3,
            CPU_MODEL_386_SX => 0,
            _ => unreachable!(),
        }
    }

    /// Charges the 386SX penalty for a dword I/O transfer, which the 16-bit
    /// bus splits into two port accesses. No-op on other models.
    #[inline(always)]
    pub(super) fn clk_sx_io_dword(&mut self) {
        if CPU_MODEL == CPU_MODEL_386_SX {
            self.clk(SX_EXTRA_CLOCKS_DWORD_ACCESS);
        }
    }

    /// Charges the 386SX 16-bit-data-bus penalty for one data transfer of
    /// `size` bytes at linear address `linear`. No-op on other models.
    #[inline(always)]
    fn clk_sx_bus(&mut self, linear: u32, size: u32) {
        if CPU_MODEL != CPU_MODEL_386_SX {
            return;
        }
        let misaligned = linear & 1 != 0;
        match size {
            2 => {
                if misaligned {
                    self.clk(SX_EXTRA_CLOCKS_MISALIGNED_WORD);
                }
            }
            4 => {
                if misaligned {
                    self.clk(SX_EXTRA_CLOCKS_MISALIGNED_DWORD);
                } else {
                    self.clk(SX_EXTRA_CLOCKS_DWORD_ACCESS);
                }
            }
            _ => {}
        }
    }

    /// Upper EFLAGS bits (16-31) that are writable on this CPU model: RF and
    /// VM on the 386DX, plus AC on the 486DX. All other upper bits are
    /// reserved and always read 0 (neither model implements CPUID, so ID is
    /// never writable). Software relies on AC being unwritable to tell a 386
    /// from a 486.
    #[inline(always)]
    const fn eflags_upper_writable() -> u32 {
        match CPU_MODEL {
            CPU_MODEL_386_DX | CPU_MODEL_386_SX => EFLAGS_RESUME_FLAG | EFLAGS_VIRTUAL_8086_FLAG,
            CPU_MODEL_486_DX => {
                EFLAGS_RESUME_FLAG | EFLAGS_VIRTUAL_8086_FLAG | EFLAGS_ALIGNMENT_CHECK_FLAG
            }
            _ => unreachable!(),
        }
    }

    /// Mask applied to physical addresses before they reach the bus,
    /// selected at compile time from `ADDRESS_WIDTH`.
    #[inline(always)]
    pub(super) const fn physical_address_mask() -> u32 {
        match ADDRESS_WIDTH {
            ADDRESS_WIDTH_24 => 0x00FF_FFFF,
            ADDRESS_WIDTH_32 => 0xFFFF_FFFF,
            _ => unreachable!(),
        }
    }

    #[inline(always)]
    fn clk(&mut self, cycles: i32) {
        self.cycles_remaining -= cycles as i64;
    }

    #[inline(always)]
    fn clk_modrm(&mut self, modrm: u8, reg_cycles: i32, mem_cycles: i32) {
        if modrm >= 0xC0 {
            self.clk(reg_cycles);
        } else {
            self.clk(mem_cycles);
        }
    }

    #[inline(always)]
    fn clk_modrm_word(&mut self, modrm: u8, reg_cycles: i32, mem_cycles: i32, word_accesses: i32) {
        if modrm >= 0xC0 {
            self.clk(reg_cycles);
        } else {
            let alignment_mask = match word_accesses {
                1 => 0x1,
                2 => {
                    if self.operand_size_override {
                        0x3
                    } else {
                        0x1
                    }
                }
                4 => 0x3,
                _ => {
                    if self.operand_size_override {
                        0x3
                    } else {
                        0x1
                    }
                }
            };
            let misaligned = self.ea & alignment_mask != 0;
            let penalty = if misaligned {
                Self::misaligned_operand_penalty()
            } else {
                0
            };
            self.clk(mem_cycles + penalty);
        }
    }

    #[inline(always)]
    fn sp_penalty(&self) -> i32 {
        let sp = if self.use_esp() {
            self.regs.dword(crate::DwordReg::ESP)
        } else {
            self.regs.word(WordReg::SP) as u32
        };
        let misaligned = if self.operand_size_override {
            sp & 3 != 0
        } else {
            sp & 1 != 0
        };
        if misaligned {
            Self::misaligned_operand_penalty()
        } else {
            0
        }
    }

    #[inline(always)]
    fn code_segment_32bit(&self) -> bool {
        if self.is_virtual_mode() {
            return false;
        }
        self.is_protected_mode()
            && self.seg_valid[SegReg32::CS as usize]
            && (self.seg_granularity[SegReg32::CS as usize] & 0x40) != 0
    }

    #[inline(always)]
    fn effective_eip(&self) -> u32 {
        self.ip_upper | self.ip as u32
    }

    #[inline(always)]
    fn advance_ip_byte(&mut self) {
        let next = (self.ip_upper | self.ip as u32).wrapping_add(1);
        self.ip = next as u16;
        self.ip_upper = next & 0xFFFF_0000;
    }

    #[inline(always)]
    fn advance_ip_by(&mut self, n: u32) {
        let next = self.effective_eip().wrapping_add(n);
        self.ip = next as u16;
        self.ip_upper = next & 0xFFFF_0000;
    }

    /// Applies a signed 8-bit branch displacement to EIP, respecting the
    /// current operand size: 32-bit mode preserves the full EIP, while
    /// 16-bit mode truncates to 16 bits.
    #[inline(always)]
    fn apply_branch_disp8(&mut self, disp: i8) {
        if self.operand_size_override {
            let eip = self.effective_eip().wrapping_add(disp as i32 as u32);
            self.ip = eip as u16;
            self.ip_upper = eip & 0xFFFF_0000;
        } else {
            self.ip = self.ip.wrapping_add(disp as u16);
            self.ip_upper = 0;
        }
    }

    #[inline(always)]
    fn next_instruction_length_approx(&self, bus: &mut impl common::Bus) -> i32 {
        let code_32bit = self.code_segment_32bit();
        let mut eip = self.effective_eip();
        let cs_base = self.seg_bases[SegReg32::CS as usize];
        let mut length = 0i32;

        loop {
            let addr = cs_base.wrapping_add(eip) & 0xFFFFFF;
            let byte = bus.fetch_opcode_byte(addr);
            match byte {
                0x26 | 0x2E | 0x36 | 0x3E | 0x64 | 0x65 | 0x66 | 0x67 | 0xF0 | 0xF2 | 0xF3 => {
                    length += 1;
                    if code_32bit {
                        eip = eip.wrapping_add(1);
                    } else {
                        eip = (eip as u16).wrapping_add(1) as u32;
                    }
                }
                _ => break,
            }
        }

        let opcode_addr = cs_base.wrapping_add(eip) & 0xFFFFFF;
        let opcode = bus.fetch_opcode_byte(opcode_addr);
        length += 1;

        if opcode == 0x0F {
            let opcode2_addr = (opcode_addr.wrapping_add(1)) & 0xFFFFFF;
            let opcode2 = bus.fetch_opcode_byte(opcode2_addr);
            length += 1;
            if Self::opcode_0f_has_modrm(opcode2) {
                length += 1;
            }
        } else if Self::opcode_has_modrm(opcode) {
            length += 1;
        }
        length
    }

    #[inline(always)]
    const fn opcode_0f_has_modrm(opcode: u8) -> bool {
        !matches!(
            opcode,
            0x06 | 0x08 | 0x09 | 0x0B | 0x80..=0x8F | 0xA0 | 0xA1 | 0xA2 | 0xA8 | 0xA9
        )
    }

    #[inline(always)]
    const fn opcode_has_modrm(opcode: u8) -> bool {
        const HAS_MODRM: [u32; 8] = [
            0x0F0F_0F0F,
            0x0F0F_0F0F,
            0x0000_0000,
            0x0000_0A0C,
            0x0000_FFFF,
            0x0000_0000,
            0xFF0F_00F3,
            0xC0C0_0000,
        ];
        let idx = (opcode >> 5) as usize;
        let bit = opcode & 0x1F;
        (HAS_MODRM[idx] >> bit) & 1 != 0
    }

    #[inline(always)]
    fn fetch(&mut self, bus: &mut impl common::Bus) -> u8 {
        if unlikely(self.fault_pending) {
            return 0;
        }
        let eip = self.effective_eip();
        // 80486 PRM Table 22-2: instruction execution crossing the CS limit
        // (e.g. continuing past offset 0xFFFF in real mode or V86) raises
        // #GP. Protected-mode descriptors honor the cached CS limit and
        // typically have plenty of headroom, so this is essentially free
        // there. fault_pending is asserted here so the prefix-decode loop
        // terminates instead of dispatching a stray 0 opcode at the handler.
        if unlikely(eip > self.seg_limits[SegReg32::CS as usize]) {
            let _: Step<()> = self.raise_fault_with_code(13, 0, bus);
            self.fault_pending = true;
            return 0;
        }
        let linear = self.seg_bases[SegReg32::CS as usize].wrapping_add(eip);
        let Ok(addr) = self.fetch_physical_address(linear, bus) else {
            self.prefetch_valid = false;
            return 0;
        };
        let value = if self.prefetch_valid && self.prefetch_addr == addr {
            self.prefetch_valid = false;
            self.prefetch_byte
        } else {
            bus.fetch_opcode_byte(addr)
        };
        if CPU_MODEL == CPU_MODEL_386_SX {
            self.sx_code_fetch_bytes += 1;
        }
        self.advance_ip_byte();
        // Prefetch next byte to model the 386 prefetch queue.
        // This ensures bytes already fetched before a REP string op
        // overwrites them are still seen correctly.
        let next_addr = addr.wrapping_add(1);
        if likely(next_addr & 0xFFF != 0) {
            self.prefetch_addr = next_addr;
            self.prefetch_byte = bus.fetch_opcode_byte(next_addr);
            self.prefetch_valid = true;
        } else {
            self.prefetch_valid = false;
        }
        value
    }

    #[inline(always)]
    fn fetch_physical_address(&mut self, linear: u32, bus: &mut impl common::Bus) -> Step<u32> {
        let page = linear >> 12;
        if likely(
            self.fetch_page_valid
                && self.fetch_page_tag == page
                && (!self.is_user_page_access() || self.fetch_page_user),
        ) {
            return Ok(self.fetch_page_phys | (linear & 0xFFF));
        }

        let addr = self.translate_linear(linear, false, bus)?;
        self.fetch_page_valid = true;
        self.fetch_page_tag = page;
        self.fetch_page_phys = addr & !0xFFF;
        self.fetch_page_user = if self.is_paging_enabled() {
            let slot = (page & 63) as usize;
            self.tlb_valid[slot] && self.tlb_tag[slot] == page && self.tlb_user[slot]
        } else {
            true
        };
        Ok(addr)
    }

    #[inline(always)]
    fn fetchword(&mut self, bus: &mut impl common::Bus) -> u16 {
        if likely(!self.fault_pending) {
            let eip = self.effective_eip();
            let linear = self.seg_bases[SegReg32::CS as usize].wrapping_add(eip);
            if likely((linear & 0xFFF) <= 0xFFE) {
                let Ok(addr) = self.fetch_physical_address(linear, bus) else {
                    return 0;
                };
                let value = bus.fetch_opcode_word(addr);
                if CPU_MODEL == CPU_MODEL_386_SX {
                    self.sx_code_fetch_bytes += 2;
                }
                self.advance_ip_by(2);
                return value;
            }
        }
        let low = self.fetch(bus) as u16;
        let high = self.fetch(bus) as u16;
        low | (high << 8)
    }

    #[inline(always)]
    fn fetchdword(&mut self, bus: &mut impl common::Bus) -> u32 {
        if likely(!self.fault_pending) {
            let eip = self.effective_eip();
            let linear = self.seg_bases[SegReg32::CS as usize].wrapping_add(eip);
            if likely((linear & 0xFFF) <= 0xFFC) {
                let Ok(addr) = self.fetch_physical_address(linear, bus) else {
                    return 0;
                };
                let value = bus.fetch_opcode_dword(addr);
                if CPU_MODEL == CPU_MODEL_386_SX {
                    self.sx_code_fetch_bytes += 4;
                }
                self.advance_ip_by(4);
                return value;
            }
        }
        let b0 = self.fetch(bus) as u32;
        let b1 = self.fetch(bus) as u32;
        let b2 = self.fetch(bus) as u32;
        let b3 = self.fetch(bus) as u32;
        b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)
    }

    #[inline(always)]
    fn default_seg(&self, seg: SegReg32) -> SegReg32 {
        if self.seg_prefix && matches!(seg, SegReg32::DS | SegReg32::SS) {
            self.prefix_seg
        } else {
            seg
        }
    }

    #[inline(always)]
    fn seg_base(&self, seg: SegReg32) -> u32 {
        self.seg_bases[seg as usize]
    }

    /// Computes the linear address for a byte at `eo32 + delta`.
    /// In 16-bit address mode, the offset wraps at 16 bits.
    #[inline(always)]
    fn seg_addr(&self, delta: u32) -> u32 {
        let offset = if self.address_size_override {
            self.eo32.wrapping_add(delta)
        } else {
            (self.eo32 as u16).wrapping_add(delta as u16) as u32
        };
        self.seg_base(self.ea_seg).wrapping_add(offset)
    }

    #[inline(always)]
    fn cpl(&self) -> u16 {
        if self.is_virtual_mode() {
            return 3;
        }
        if !self.is_protected_mode() {
            return 0;
        }
        self.stored_cpl
    }

    #[inline(always)]
    fn is_user_page_access(&self) -> bool {
        self.cpl() == 3 && !self.supervisor_override
    }

    #[inline(always)]
    fn is_protected_mode(&self) -> bool {
        self.cr0 & 1 != 0
    }

    #[inline(always)]
    fn is_virtual_mode(&self) -> bool {
        self.is_protected_mode() && (self.eflags_upper & 0x0002_0000) != 0
    }

    /// Returns `Ok(())` if the current privilege level and IOPL allow I/O access
    /// to `port`. When access is denied and the IOPB does not grant it, raises
    /// `#GP(0)` and returns `Err(Fault)`. CLI/STI/HLT use separate IOPL/CPL
    /// checks - this function is only for I/O port instructions.
    fn check_io_privilege(&mut self, port: u16, size: u8, bus: &mut impl common::Bus) -> Step {
        if !self.is_protected_mode() {
            return Ok(());
        }
        if !self.is_virtual_mode() && self.cpl() <= u16::from(self.flags.iopl) {
            return Ok(());
        }
        if bus.is_io_port_unrestricted(port) {
            return Ok(());
        }
        // CPL > IOPL (or VM86 with IOPL < 3): consult I/O Permission Bitmap in TSS.
        // TSS must be a 386 TSS (type field >= 9) and large enough to hold the IOPB pointer.
        if self.tr_limit < 0x67 || (self.tr_rights & 0x0F) < 9 {
            return self.raise_fault_with_code(13, 0, bus);
        }
        let iopb = self.read_word_linear(bus, self.tr_base.wrapping_add(0x66))?;
        let byte_idx = u32::from(iopb).wrapping_add(u32::from(port) / 8);
        // Read two consecutive bytes from the IOPB to handle accesses that span a
        // byte boundary (e.g. a dword access starting at port 7 touches bits in two
        // bytes). The +1 also satisfies the Intel spec requirement for a 0xFF sentinel
        // byte immediately after the IOPB inside the TSS.
        if byte_idx + 1 > self.tr_limit {
            return self.raise_fault_with_code(13, 0, bus);
        }
        let linear = self.tr_base.wrapping_add(byte_idx);
        let map = self.read_word_linear(bus, linear)?;
        let mask = (1u16 << size) - 1;
        if (map >> (port & 7)) & mask != 0 {
            return self.raise_fault_with_code(13, 0, bus);
        }
        Ok(())
    }

    /// Reads the next instruction byte from CS:IP without advancing IP.
    /// Used to inspect a ModR/M byte before deciding how to dispatch.
    fn peek_byte(&mut self, bus: &mut impl common::Bus) -> u8 {
        let eip = self.effective_eip();
        let linear = self.seg_bases[SegReg32::CS as usize].wrapping_add(eip);
        let Ok(addr) = self.fetch_physical_address(linear, bus) else {
            return 0;
        };
        if self.prefetch_valid && self.prefetch_addr == addr {
            self.prefetch_byte
        } else {
            bus.read_byte(addr)
        }
    }

    /// Returns `true` when a ModR/M byte selects a register operand
    /// (mod == 11b). The lockable opcodes that take a ModR/M byte (one-byte
    /// forms only) require a memory destination.
    #[inline(always)]
    fn lockable_modrm_is_register(modrm: u8) -> bool {
        modrm & 0xC0 == 0xC0
    }

    fn is_lockable(opcode: u8) -> bool {
        matches!(
            opcode,
            // ADD r/m, r | ADD r/m, imm (via 80/81/83)
            0x00 | 0x01
            // OR r/m, r
            | 0x08 | 0x09
            // ADC r/m, r
            | 0x10 | 0x11
            // SBB r/m, r
            | 0x18 | 0x19
            // AND r/m, r
            | 0x20 | 0x21
            // SUB r/m, r
            | 0x28 | 0x29
            // XOR r/m, r
            | 0x30 | 0x31
            // ADD/OR/ADC/SBB/AND/SUB/XOR/CMP r/m, imm (0x82 is alias for 0x80)
            | 0x80 | 0x81 | 0x82 | 0x83
            // XCHG r/m, r
            | 0x86 | 0x87
            // NOT/NEG r/m (F6 /2, /3 and F7 /2, /3)
            | 0xF6 | 0xF7
            // INC/DEC r/m (FE /0, /1 and FF /0, /1)
            | 0xFE | 0xFF
        )
    }

    #[inline(always)]
    fn segment_error_code(selector: u16) -> u16 {
        selector & 0xFFFC
    }

    fn set_real_segment_cache(&mut self, seg: SegReg32, selector: u16) {
        self.seg_bases[seg as usize] = (selector as u32) << 4;
        self.seg_limits[seg as usize] = 0xFFFF;
        self.seg_rights[seg as usize] = if seg == SegReg32::CS { 0x9B } else { 0x93 };
        self.seg_granularity[seg as usize] = 0;
        self.seg_valid[seg as usize] = true;
        if seg == SegReg32::CS {
            self.stored_cpl = 0;
        }
    }

    fn set_loaded_segment_cache(
        &mut self,
        seg: SegReg32,
        selector: u16,
        descriptor: SegmentDescriptor,
    ) {
        self.sregs[seg as usize] = selector;
        self.seg_bases[seg as usize] = descriptor.base;
        self.seg_limits[seg as usize] = descriptor.limit;
        self.seg_rights[seg as usize] = descriptor.rights;
        self.seg_granularity[seg as usize] = descriptor.granularity;
        self.seg_valid[seg as usize] = true;
        if seg == SegReg32::CS {
            self.stored_cpl = selector & 3;
        }
    }

    fn set_null_segment(&mut self, seg: SegReg32, selector: u16) {
        self.sregs[seg as usize] = selector;
        self.seg_bases[seg as usize] = 0;
        self.seg_limits[seg as usize] = 0;
        self.seg_rights[seg as usize] = 0;
        self.seg_granularity[seg as usize] = 0;
        self.seg_valid[seg as usize] = false;
    }

    fn decode_descriptor(
        &mut self,
        selector: u16,
        bus: &mut impl common::Bus,
    ) -> Step<Option<SegmentDescriptor>> {
        let Some(addr) = self.descriptor_addr_checked(selector) else {
            return Ok(None);
        };
        Ok(Some(self.decode_descriptor_at(addr, bus)?))
    }

    fn descriptor_dpl(rights: u8) -> u16 {
        ((rights >> 5) & 0x03) as u16
    }

    fn descriptor_is_segment(rights: u8) -> bool {
        rights & 0x10 != 0
    }

    fn descriptor_is_code(rights: u8) -> bool {
        rights & 0x08 != 0
    }

    fn descriptor_is_conforming_code(rights: u8) -> bool {
        Self::descriptor_is_code(rights) && rights & 0x04 != 0
    }

    fn descriptor_is_readable(rights: u8) -> bool {
        !Self::descriptor_is_code(rights) || rights & 0x02 != 0
    }

    fn descriptor_is_writable(rights: u8) -> bool {
        !Self::descriptor_is_code(rights) && rights & 0x02 != 0
    }

    fn descriptor_is_expand_down(rights: u8) -> bool {
        !Self::descriptor_is_code(rights) && rights & 0x04 != 0
    }

    fn descriptor_present(rights: u8) -> bool {
        rights & 0x80 != 0
    }

    fn raise_segment_not_present<T>(
        &mut self,
        seg: SegReg32,
        selector: u16,
        bus: &mut impl common::Bus,
    ) -> Step<T> {
        let vector = if seg == SegReg32::SS { 12 } else { 11 };
        self.raise_fault_with_code(vector, Self::segment_error_code(selector), bus)
    }

    fn raise_segment_protection<T>(
        &mut self,
        seg: SegReg32,
        selector: u16,
        bus: &mut impl common::Bus,
    ) -> Step<T> {
        let vector = if seg == SegReg32::SS { 12 } else { 13 };
        self.raise_fault_with_code(vector, Self::segment_error_code(selector), bus)
    }

    fn load_protected_segment(
        &mut self,
        seg: SegReg32,
        selector: u16,
        bus: &mut impl common::Bus,
    ) -> Step {
        if matches!(
            seg,
            SegReg32::DS | SegReg32::ES | SegReg32::FS | SegReg32::GS
        ) && selector & 0xFFFC == 0
        {
            self.set_null_segment(seg, selector);
            return Ok(());
        }
        if selector & 0xFFFC == 0 {
            // Null selector for CS or SS: always #GP(0) per 386 manual.
            return self.raise_fault_with_code(13, 0, bus);
        }

        let Some(descriptor) = self.decode_descriptor(selector, bus)? else {
            return self.raise_fault_with_code(13, Self::segment_error_code(selector), bus);
        };
        let rights = descriptor.rights;
        let cpl = self.cpl();
        let rpl = selector & 0x0003;
        let dpl = Self::descriptor_dpl(rights);

        match seg {
            SegReg32::CS => {
                if !Self::descriptor_is_segment(rights) || !Self::descriptor_is_code(rights) {
                    return self.raise_segment_protection(seg, selector, bus);
                }
                if Self::descriptor_is_conforming_code(rights) {
                    if dpl > cpl {
                        return self.raise_segment_protection(seg, selector, bus);
                    }
                } else if dpl != cpl || rpl > cpl {
                    return self.raise_segment_protection(seg, selector, bus);
                }
                if !Self::descriptor_present(rights) {
                    return self.raise_segment_not_present(seg, selector, bus);
                }
                self.set_accessed_bit(selector, bus)?;
                if Self::descriptor_is_conforming_code(rights) {
                    let adjusted = (selector & !3) | cpl;
                    self.set_loaded_segment_cache(seg, adjusted, descriptor);
                } else {
                    let adjusted = (selector & !3) | dpl;
                    self.set_loaded_segment_cache(seg, adjusted, descriptor);
                }
                return Ok(());
            }
            SegReg32::SS => {
                if !Self::descriptor_is_segment(rights) || !Self::descriptor_is_writable(rights) {
                    return self.raise_fault_with_code(13, Self::segment_error_code(selector), bus);
                }
                if dpl != cpl || rpl != cpl {
                    return self.raise_fault_with_code(13, Self::segment_error_code(selector), bus);
                }
                if !Self::descriptor_present(rights) {
                    return self.raise_fault_with_code(12, Self::segment_error_code(selector), bus);
                }
            }
            SegReg32::DS | SegReg32::ES | SegReg32::FS | SegReg32::GS => {
                if !Self::descriptor_is_segment(rights) {
                    return self.raise_segment_protection(seg, selector, bus);
                }
                if !Self::descriptor_is_readable(rights) {
                    return self.raise_segment_protection(seg, selector, bus);
                }
                if !Self::descriptor_is_conforming_code(rights) && dpl < cpl.max(rpl) {
                    return self.raise_segment_protection(seg, selector, bus);
                }
                if !Self::descriptor_present(rights) {
                    return self.raise_segment_not_present(seg, selector, bus);
                }
            }
        }

        self.set_accessed_bit(selector, bus)?;
        self.set_loaded_segment_cache(seg, selector, descriptor);
        Ok(())
    }

    /// Validates a candidate CS for an inter- or same-privilege return
    /// (IRET, RETF) without committing any state. Mirrors the checks in
    /// `load_cs_for_return` but reports the outcome instead of raising the
    /// fault or writing the segment cache, allowing the caller to defer all
    /// observable mutations until every operand has been validated.
    ///
    /// Returns `None` if a #PF was raised while reading the descriptor (the
    /// fault is already pending; the caller must abort without raising a
    /// second one). `Some(Err((vector, error_code)))` indicates the selector
    /// fails the protected-mode checks and the caller should raise that
    /// fault before any commit. `Some(Ok(_))` means the selector is
    /// acceptable and the caller may commit using the returned descriptor
    /// and adjusted selector.
    fn validate_cs_for_return(
        &mut self,
        selector: u16,
        new_ip: u32,
        bus: &mut impl common::Bus,
    ) -> Step<Result<CsReturnValidation, (u8, u16)>> {
        if selector & 0xFFFC == 0 {
            return Ok(Err((13, 0)));
        }
        let Some(descriptor) = self.decode_descriptor(selector, bus)? else {
            return Ok(Err((13, Self::segment_error_code(selector))));
        };
        let rights = descriptor.rights;
        let cpl = self.cpl();
        let rpl = selector & 0x0003;
        let dpl = Self::descriptor_dpl(rights);

        if rpl < cpl {
            return Ok(Err((13, Self::segment_error_code(selector))));
        }
        if !Self::descriptor_is_segment(rights) || !Self::descriptor_is_code(rights) {
            return Ok(Err((13, Self::segment_error_code(selector))));
        }
        if Self::descriptor_is_conforming_code(rights) {
            if dpl > rpl {
                return Ok(Err((13, Self::segment_error_code(selector))));
            }
        } else if dpl != rpl {
            return Ok(Err((13, Self::segment_error_code(selector))));
        }
        if !Self::descriptor_present(rights) {
            return Ok(Err((11, Self::segment_error_code(selector))));
        }
        if new_ip > descriptor.limit {
            return Ok(Err((13, 0)));
        }
        let adjusted_selector = (selector & !3) | rpl;
        Ok(Ok(CsReturnValidation {
            descriptor,
            adjusted_selector,
        }))
    }

    /// Variant of `precheck_ss_for_inter_priv_iret` that also returns the
    /// cached descriptor so the IRET commit block can reuse it without
    /// re-decoding.
    fn validate_ss_for_iret_return(
        &mut self,
        selector: u16,
        target_cpl: u16,
        bus: &mut impl common::Bus,
    ) -> Step<Result<SsReturnValidation, (u8, u16)>> {
        if selector & 0xFFFC == 0 {
            return Ok(Err((13, 0)));
        }
        let Some(descriptor) = self.decode_descriptor(selector, bus)? else {
            return Ok(Err((13, Self::segment_error_code(selector))));
        };
        let rights = descriptor.rights;
        let rpl = selector & 0x0003;
        let dpl = Self::descriptor_dpl(rights);
        if rpl != target_cpl {
            return Ok(Err((13, Self::segment_error_code(selector))));
        }
        if !Self::descriptor_is_segment(rights) || !Self::descriptor_is_writable(rights) {
            return Ok(Err((13, Self::segment_error_code(selector))));
        }
        if dpl != target_cpl {
            return Ok(Err((13, Self::segment_error_code(selector))));
        }
        if !Self::descriptor_present(rights) {
            return Ok(Err((12, Self::segment_error_code(selector))));
        }
        Ok(Ok(SsReturnValidation { descriptor }))
    }

    /// Returns the post-IRET decision for a data segment without committing
    /// it. Used during the IRET inter-privilege validate phase so that the
    /// faulting descriptor read happens before any architectural state is
    /// mutated. Returns `None` if a #PF was raised on the descriptor read;
    /// the caller must abort without raising another fault.
    fn check_data_segment_at_cpl(
        &mut self,
        seg: SegReg32,
        new_cpl: u16,
        bus: &mut impl common::Bus,
    ) -> Step<DataSegmentDecision> {
        if !self.seg_valid[seg as usize] {
            return Ok(DataSegmentDecision::NoChange);
        }
        let selector = self.sregs[seg as usize];
        if selector & 0xFFFC == 0 {
            return Ok(DataSegmentDecision::Null { selector });
        }
        let saved_supervisor_override = self.supervisor_override;
        self.supervisor_override = true;
        let descriptor = self.decode_descriptor(selector, bus);
        self.supervisor_override = saved_supervisor_override;
        let descriptor = descriptor?;
        let Some(descriptor) = descriptor else {
            return Ok(DataSegmentDecision::Null { selector: 0 });
        };
        let rights = descriptor.rights;
        if !Self::descriptor_is_segment(rights) {
            return Ok(DataSegmentDecision::Null { selector: 0 });
        }
        if Self::descriptor_is_code(rights) && !Self::descriptor_is_readable(rights) {
            return Ok(DataSegmentDecision::Null { selector: 0 });
        }
        if !Self::descriptor_present(rights) {
            return Ok(DataSegmentDecision::Null { selector: 0 });
        }
        if !Self::descriptor_is_conforming_code(rights) {
            let dpl = Self::descriptor_dpl(rights);
            let rpl = selector & 3;
            if dpl < new_cpl || dpl < rpl {
                return Ok(DataSegmentDecision::Null { selector: 0 });
            }
        }
        Ok(DataSegmentDecision::Keep {
            selector,
            descriptor,
        })
    }

    fn apply_data_segment_decision(&mut self, seg: SegReg32, decision: DataSegmentDecision) {
        match decision {
            DataSegmentDecision::NoChange => {}
            DataSegmentDecision::Null { selector } => self.set_null_segment(seg, selector),
            DataSegmentDecision::Keep {
                selector,
                descriptor,
            } => {
                self.set_loaded_segment_cache(seg, selector, descriptor);
            }
        }
    }

    fn check_segment_access(
        &mut self,
        seg: SegReg32,
        offset: u32,
        size: u32,
        write: bool,
        bus: &mut impl common::Bus,
    ) -> Step {
        if self.fault_pending {
            return Err(Fault);
        }
        if !self.is_protected_mode() {
            return Ok(());
        }

        if !self.is_virtual_mode() {
            if !self.seg_valid[seg as usize] {
                let vector = if seg == SegReg32::SS { 12 } else { 13 };
                return self.raise_fault_with_code(vector, 0, bus);
            }

            let rights = self.seg_rights[seg as usize];
            let end = offset.saturating_add(size.saturating_sub(1));
            let wrapped = offset.checked_add(size.saturating_sub(1)).is_none();
            let limit = self.seg_limits[seg as usize];
            if Self::descriptor_is_expand_down(rights) {
                let upper = if self.seg_granularity[seg as usize] & 0x40 != 0 {
                    0xFFFF_FFFF
                } else {
                    0xFFFF
                };
                if offset <= limit || end > upper || wrapped {
                    return self.raise_fault_with_code(
                        if seg == SegReg32::SS { 12 } else { 13 },
                        0,
                        bus,
                    );
                }
            } else if end > limit || wrapped {
                return self.raise_fault_with_code(
                    if seg == SegReg32::SS { 12 } else { 13 },
                    0,
                    bus,
                );
            }

            if write {
                if !Self::descriptor_is_writable(rights) {
                    let vector = if seg == SegReg32::SS { 12 } else { 13 };
                    return self.raise_fault_with_code(vector, 0, bus);
                }
            } else if !Self::descriptor_is_readable(rights) {
                let vector = if seg == SegReg32::SS { 12 } else { 13 };
                return self.raise_fault_with_code(vector, 0, bus);
            }
        }

        // 80486 PRM 9.9.16: alignment check (#AC, vector 17) fires when
        // CR0.AM=1, EFLAGS.AC=1, CPL=3, and the linear address is not
        // aligned to the access size. Instruction fetch and descriptor
        // accesses go through different paths and are exempt.
        if CPU_MODEL == CPU_MODEL_486_DX
            && (self.cr0 & 0x0004_0000) != 0
            && (self.eflags_upper & 0x0004_0000) != 0
            && self.cpl() == 3
        {
            let alignment_mask = match size {
                2 => 1,
                4 => 3,
                8 => 7,
                _ => 0,
            };
            if alignment_mask != 0 {
                let linear = self.seg_base(seg).wrapping_add(offset);
                if linear & alignment_mask != 0 {
                    return self.raise_fault_with_code(17, 0, bus);
                }
            }
        }

        Ok(())
    }

    #[inline(always)]
    fn seg_read_word_at_with(
        &mut self,
        bus: &mut impl common::Bus,
        delta: u32,
        for_update: bool,
    ) -> Step<u16> {
        let offset = if self.address_size_override {
            self.eo32.wrapping_add(delta)
        } else {
            (self.eo32 as u16).wrapping_add(delta as u16) as u32
        };
        self.check_segment_access(self.ea_seg, offset, 2, for_update, bus)?;
        let l0 = self.seg_addr(delta);
        let same_page = if self.address_size_override {
            l0 & 0xFFF <= 0xFFE
        } else {
            l0 & 0xFFF <= 0xFFE && (offset as u16) <= 0xFFFE
        };
        if same_page {
            let a0 = self.translate_linear(l0, for_update, bus)?;
            return Ok(bus.read_word(a0));
        }
        let l1 = self.seg_addr(delta.wrapping_add(1));
        let a0 = self.translate_linear(l0, for_update, bus)?;
        let a1 = self.translate_linear(l1, for_update, bus)?;
        Ok(bus.read_byte(a0) as u16 | ((bus.read_byte(a1) as u16) << 8))
    }

    #[inline(always)]
    fn seg_read_word_at(&mut self, bus: &mut impl common::Bus, delta: u32) -> Step<u16> {
        self.seg_read_word_at_with(bus, delta, false)
    }

    #[inline(always)]
    fn seg_read_word(&mut self, bus: &mut impl common::Bus) -> Step<u16> {
        self.seg_read_word_at_with(bus, 0, false)
    }

    #[inline(always)]
    pub(super) fn seg_read_word_for_update(&mut self, bus: &mut impl common::Bus) -> Step<u16> {
        self.seg_read_word_at_with(bus, 0, true)
    }

    #[inline(always)]
    fn seg_read_dword_at_with(
        &mut self,
        bus: &mut impl common::Bus,
        delta: u32,
        for_update: bool,
    ) -> Step<u32> {
        let offset = if self.address_size_override {
            self.eo32.wrapping_add(delta)
        } else {
            (self.eo32 as u16).wrapping_add(delta as u16) as u32
        };
        self.check_segment_access(self.ea_seg, offset, 4, for_update, bus)?;
        let l0 = self.seg_addr(delta);
        let same_page = if self.address_size_override {
            l0 & 0xFFF <= 0xFFC
        } else {
            l0 & 0xFFF <= 0xFFC && (offset as u16) <= 0xFFFC
        };
        if same_page {
            let a0 = self.translate_linear(l0, for_update, bus)?;
            return Ok(bus.read_dword(a0));
        }
        let l1 = self.seg_addr(delta.wrapping_add(1));
        let l2 = self.seg_addr(delta.wrapping_add(2));
        let l3 = self.seg_addr(delta.wrapping_add(3));
        let a0 = self.translate_linear(l0, for_update, bus)?;
        let a1 = self.translate_linear(l1, for_update, bus)?;
        let a2 = self.translate_linear(l2, for_update, bus)?;
        let a3 = self.translate_linear(l3, for_update, bus)?;
        Ok(bus.read_byte(a0) as u32
            | ((bus.read_byte(a1) as u32) << 8)
            | ((bus.read_byte(a2) as u32) << 16)
            | ((bus.read_byte(a3) as u32) << 24))
    }

    #[inline(always)]
    fn seg_read_dword_at(&mut self, bus: &mut impl common::Bus, delta: u32) -> Step<u32> {
        self.seg_read_dword_at_with(bus, delta, false)
    }

    #[inline(always)]
    fn seg_read_dword(&mut self, bus: &mut impl common::Bus) -> Step<u32> {
        self.seg_read_dword_at_with(bus, 0, false)
    }

    #[inline(always)]
    pub(super) fn seg_read_dword_for_update(&mut self, bus: &mut impl common::Bus) -> Step<u32> {
        self.seg_read_dword_at_with(bus, 0, true)
    }

    #[inline(always)]
    fn seg_write_word(&mut self, bus: &mut impl common::Bus, value: u16) -> Step {
        self.check_segment_access(self.ea_seg, self.eo32, 2, true, bus)?;
        let l0 = self.seg_addr(0);
        let same_page = if self.address_size_override {
            l0 & 0xFFF <= 0xFFE
        } else {
            l0 & 0xFFF <= 0xFFE && (self.eo32 as u16) <= 0xFFFE
        };
        if same_page {
            let a0 = self.translate_linear(l0, true, bus)?;
            bus.write_word(a0, value);
            return Ok(());
        }
        let l1 = self.seg_addr(1);
        let a0 = self.translate_linear(l0, true, bus)?;
        let a1 = self.translate_linear(l1, true, bus)?;
        bus.write_byte(a0, value as u8);
        bus.write_byte(a1, (value >> 8) as u8);
        Ok(())
    }

    #[inline(always)]
    fn seg_write_dword(&mut self, bus: &mut impl common::Bus, value: u32) -> Step {
        self.check_segment_access(self.ea_seg, self.eo32, 4, true, bus)?;
        let l0 = self.seg_addr(0);
        let same_page = if self.address_size_override {
            l0 & 0xFFF <= 0xFFC
        } else {
            l0 & 0xFFF <= 0xFFC && (self.eo32 as u16) <= 0xFFFC
        };
        if same_page {
            let a0 = self.translate_linear(l0, true, bus)?;
            bus.write_dword(a0, value);
            return Ok(());
        }
        let l1 = self.seg_addr(1);
        let l2 = self.seg_addr(2);
        let l3 = self.seg_addr(3);
        let a0 = self.translate_linear(l0, true, bus)?;
        let a1 = self.translate_linear(l1, true, bus)?;
        let a2 = self.translate_linear(l2, true, bus)?;
        let a3 = self.translate_linear(l3, true, bus)?;
        bus.write_byte(a0, value as u8);
        bus.write_byte(a1, (value >> 8) as u8);
        bus.write_byte(a2, (value >> 16) as u8);
        bus.write_byte(a3, (value >> 24) as u8);
        Ok(())
    }

    #[inline(always)]
    fn read_byte_seg(
        &mut self,
        bus: &mut impl common::Bus,
        seg: SegReg32,
        offset: u32,
    ) -> Step<u8> {
        self.check_segment_access(seg, offset, 1, false, bus)?;
        let linear = self.seg_base(seg).wrapping_add(offset);
        let addr = self.translate_linear(linear, false, bus)?;
        Ok(bus.read_byte(addr))
    }

    #[inline(always)]
    fn write_byte_seg(
        &mut self,
        bus: &mut impl common::Bus,
        seg: SegReg32,
        offset: u32,
        value: u8,
    ) -> Step {
        self.check_segment_access(seg, offset, 1, true, bus)?;
        let linear = self.seg_base(seg).wrapping_add(offset);
        let addr = self.translate_linear(linear, true, bus)?;
        bus.write_byte(addr, value);
        Ok(())
    }

    #[inline(always)]
    fn read_word_seg_with(
        &mut self,
        bus: &mut impl common::Bus,
        seg: SegReg32,
        offset: u32,
        for_update: bool,
    ) -> Step<u16> {
        self.check_segment_access(seg, offset, 2, for_update, bus)?;
        let base = self.seg_base(seg);
        let l0 = base.wrapping_add(offset);
        self.clk_sx_bus(l0, 2);
        if l0 & 0xFFF <= 0xFFE {
            let a0 = self.translate_linear(l0, for_update, bus)?;
            return Ok(bus.read_word(a0));
        }
        let l1 = base.wrapping_add(offset.wrapping_add(1));
        let a0 = self.translate_linear(l0, for_update, bus)?;
        let a1 = self.translate_linear(l1, for_update, bus)?;
        Ok(bus.read_byte(a0) as u16 | ((bus.read_byte(a1) as u16) << 8))
    }

    #[inline(always)]
    fn read_word_seg(
        &mut self,
        bus: &mut impl common::Bus,
        seg: SegReg32,
        offset: u32,
    ) -> Step<u16> {
        self.read_word_seg_with(bus, seg, offset, false)
    }

    #[inline(always)]
    pub(super) fn read_word_seg_for_update(
        &mut self,
        bus: &mut impl common::Bus,
        seg: SegReg32,
        offset: u32,
    ) -> Step<u16> {
        self.read_word_seg_with(bus, seg, offset, true)
    }

    #[inline(always)]
    fn read_dword_seg_with(
        &mut self,
        bus: &mut impl common::Bus,
        seg: SegReg32,
        offset: u32,
        for_update: bool,
    ) -> Step<u32> {
        self.check_segment_access(seg, offset, 4, for_update, bus)?;
        let base = self.seg_base(seg);
        let l0 = base.wrapping_add(offset);
        self.clk_sx_bus(l0, 4);
        if l0 & 0xFFF <= 0xFFC {
            let a0 = self.translate_linear(l0, for_update, bus)?;
            return Ok(bus.read_dword(a0));
        }
        let l1 = base.wrapping_add(offset.wrapping_add(1));
        let l2 = base.wrapping_add(offset.wrapping_add(2));
        let l3 = base.wrapping_add(offset.wrapping_add(3));
        let a0 = self.translate_linear(l0, for_update, bus)?;
        let a1 = self.translate_linear(l1, for_update, bus)?;
        let a2 = self.translate_linear(l2, for_update, bus)?;
        let a3 = self.translate_linear(l3, for_update, bus)?;
        Ok(bus.read_byte(a0) as u32
            | ((bus.read_byte(a1) as u32) << 8)
            | ((bus.read_byte(a2) as u32) << 16)
            | ((bus.read_byte(a3) as u32) << 24))
    }

    #[inline(always)]
    fn read_dword_seg(
        &mut self,
        bus: &mut impl common::Bus,
        seg: SegReg32,
        offset: u32,
    ) -> Step<u32> {
        self.read_dword_seg_with(bus, seg, offset, false)
    }

    #[inline(always)]
    pub(super) fn read_dword_seg_for_update(
        &mut self,
        bus: &mut impl common::Bus,
        seg: SegReg32,
        offset: u32,
    ) -> Step<u32> {
        self.read_dword_seg_with(bus, seg, offset, true)
    }

    #[inline(always)]
    fn write_word_seg(
        &mut self,
        bus: &mut impl common::Bus,
        seg: SegReg32,
        offset: u32,
        value: u16,
    ) -> Step {
        self.check_segment_access(seg, offset, 2, true, bus)?;
        let base = self.seg_base(seg);
        let l0 = base.wrapping_add(offset);
        self.clk_sx_bus(l0, 2);
        if l0 & 0xFFF <= 0xFFE {
            let a0 = self.translate_linear(l0, true, bus)?;
            bus.write_word(a0, value);
            return Ok(());
        }
        let l1 = base.wrapping_add(offset.wrapping_add(1));
        let a0 = self.translate_linear(l0, true, bus)?;
        let a1 = self.translate_linear(l1, true, bus)?;
        bus.write_byte(a0, value as u8);
        bus.write_byte(a1, (value >> 8) as u8);
        Ok(())
    }

    #[inline(always)]
    fn write_dword_seg(
        &mut self,
        bus: &mut impl common::Bus,
        seg: SegReg32,
        offset: u32,
        value: u32,
    ) -> Step {
        self.check_segment_access(seg, offset, 4, true, bus)?;
        let base = self.seg_base(seg);
        let l0 = base.wrapping_add(offset);
        self.clk_sx_bus(l0, 4);
        if l0 & 0xFFF <= 0xFFC {
            let a0 = self.translate_linear(l0, true, bus)?;
            bus.write_dword(a0, value);
            return Ok(());
        }
        let l1 = base.wrapping_add(offset.wrapping_add(1));
        let l2 = base.wrapping_add(offset.wrapping_add(2));
        let l3 = base.wrapping_add(offset.wrapping_add(3));
        let a0 = self.translate_linear(l0, true, bus)?;
        let a1 = self.translate_linear(l1, true, bus)?;
        let a2 = self.translate_linear(l2, true, bus)?;
        let a3 = self.translate_linear(l3, true, bus)?;
        bus.write_byte(a0, value as u8);
        bus.write_byte(a1, (value >> 8) as u8);
        bus.write_byte(a2, (value >> 16) as u8);
        bus.write_byte(a3, (value >> 24) as u8);
        Ok(())
    }

    #[inline(always)]
    fn use_esp(&self) -> bool {
        if self.is_virtual_mode() {
            return false;
        }
        self.seg_granularity[SegReg32::SS as usize] & 0x40 != 0
    }

    fn push(&mut self, bus: &mut impl common::Bus, value: u16) -> Step {
        let sp = if self.use_esp() {
            self.regs.dword(crate::DwordReg::ESP).wrapping_sub(2)
        } else {
            self.regs.word(WordReg::SP).wrapping_sub(2) as u32
        };
        self.check_segment_access(SegReg32::SS, sp, 2, true, bus)?;
        let base = self.seg_base(SegReg32::SS);
        let l0 = base.wrapping_add(sp);
        self.clk_sx_bus(l0, 2);
        if l0 & 0xFFF <= 0xFFE {
            let a0 = self.translate_linear(l0, true, bus)?;
            self.commit_sp(sp);
            bus.write_word(a0, value);
            return Ok(());
        }
        let l1 = base.wrapping_add(sp.wrapping_add(1));
        let a0 = self.translate_linear(l0, true, bus)?;
        let a1 = self.translate_linear(l1, true, bus)?;
        self.commit_sp(sp);
        bus.write_byte(a0, value as u8);
        bus.write_byte(a1, (value >> 8) as u8);
        Ok(())
    }

    fn pop(&mut self, bus: &mut impl common::Bus) -> Step<u16> {
        let sp = if self.use_esp() {
            self.regs.dword(crate::DwordReg::ESP)
        } else {
            self.regs.word(WordReg::SP) as u32
        };
        self.check_segment_access(SegReg32::SS, sp, 2, false, bus)?;
        let base = self.seg_base(SegReg32::SS);
        let l0 = base.wrapping_add(sp);
        self.clk_sx_bus(l0, 2);
        let value = if l0 & 0xFFF <= 0xFFE {
            let a0 = self.translate_linear(l0, false, bus)?;
            bus.read_word(a0)
        } else {
            let l1 = base.wrapping_add(sp.wrapping_add(1));
            let a0 = self.translate_linear(l0, false, bus)?;
            let a1 = self.translate_linear(l1, false, bus)?;
            bus.read_byte(a0) as u16 | ((bus.read_byte(a1) as u16) << 8)
        };
        self.commit_sp(sp.wrapping_add(2));
        Ok(value)
    }

    fn push_dword(&mut self, bus: &mut impl common::Bus, value: u32) -> Step {
        let sp = if self.use_esp() {
            self.regs.dword(crate::DwordReg::ESP).wrapping_sub(4)
        } else {
            self.regs.word(WordReg::SP).wrapping_sub(4) as u32
        };
        self.check_segment_access(SegReg32::SS, sp, 4, true, bus)?;
        let base = self.seg_base(SegReg32::SS);
        let l0 = base.wrapping_add(sp);
        self.clk_sx_bus(l0, 4);
        if l0 & 0xFFF <= 0xFFC {
            let a0 = self.translate_linear(l0, true, bus)?;
            self.commit_sp(sp);
            bus.write_dword(a0, value);
            return Ok(());
        }
        let l1 = base.wrapping_add(sp.wrapping_add(1));
        let l2 = base.wrapping_add(sp.wrapping_add(2));
        let l3 = base.wrapping_add(sp.wrapping_add(3));
        let a0 = self.translate_linear(l0, true, bus)?;
        let a1 = self.translate_linear(l1, true, bus)?;
        let a2 = self.translate_linear(l2, true, bus)?;
        let a3 = self.translate_linear(l3, true, bus)?;
        self.commit_sp(sp);
        bus.write_byte(a0, value as u8);
        bus.write_byte(a1, (value >> 8) as u8);
        bus.write_byte(a2, (value >> 16) as u8);
        bus.write_byte(a3, (value >> 24) as u8);
        Ok(())
    }

    /// Returns the bytes written into a dword selector slot for this CPU model.
    const fn dword_selector_write_size() -> u32 {
        if CPU_MODEL == CPU_MODEL_386_SX { 4 } else { 2 }
    }

    /// Allocates a dword stack slot and writes the model-specific selector image.
    fn push_dword_selector(&mut self, bus: &mut impl common::Bus, value: u16) -> Step {
        if Self::dword_selector_write_size() == 4 {
            return self.push_dword(bus, value as u32);
        }
        let stack_pointer = if self.use_esp() {
            self.regs.dword(crate::DwordReg::ESP).wrapping_sub(4)
        } else {
            self.regs.word(WordReg::SP).wrapping_sub(4) as u32
        };
        self.check_segment_access(SegReg32::SS, stack_pointer, 2, true, bus)?;
        let linear = self.seg_base(SegReg32::SS).wrapping_add(stack_pointer);
        self.clk_sx_bus(linear, 2);
        if linear & 0xFFF <= 0xFFE {
            let physical = self.translate_linear(linear, true, bus)?;
            self.commit_sp(stack_pointer);
            bus.write_word(physical, value);
            return Ok(());
        }
        let next_linear = linear.wrapping_add(1);
        let low_physical = self.translate_linear(linear, true, bus)?;
        let high_physical = self.translate_linear(next_linear, true, bus)?;
        self.commit_sp(stack_pointer);
        bus.write_byte(low_physical, value as u8);
        bus.write_byte(high_physical, (value >> 8) as u8);
        Ok(())
    }

    #[inline(always)]
    fn commit_sp(&mut self, new_sp: u32) {
        if self.use_esp() {
            self.regs.set_dword(crate::DwordReg::ESP, new_sp);
        } else {
            self.regs.set_word(WordReg::SP, new_sp as u16);
        }
    }

    fn pop_dword(&mut self, bus: &mut impl common::Bus) -> Step<u32> {
        let sp = if self.use_esp() {
            self.regs.dword(crate::DwordReg::ESP)
        } else {
            self.regs.word(WordReg::SP) as u32
        };
        self.check_segment_access(SegReg32::SS, sp, 4, false, bus)?;
        let base = self.seg_base(SegReg32::SS);
        let l0 = base.wrapping_add(sp);
        self.clk_sx_bus(l0, 4);
        let value = if l0 & 0xFFF <= 0xFFC {
            let a0 = self.translate_linear(l0, false, bus)?;
            bus.read_dword(a0)
        } else {
            let l1 = base.wrapping_add(sp.wrapping_add(1));
            let l2 = base.wrapping_add(sp.wrapping_add(2));
            let l3 = base.wrapping_add(sp.wrapping_add(3));
            let a0 = self.translate_linear(l0, false, bus)?;
            let a1 = self.translate_linear(l1, false, bus)?;
            let a2 = self.translate_linear(l2, false, bus)?;
            let a3 = self.translate_linear(l3, false, bus)?;
            bus.read_byte(a0) as u32
                | ((bus.read_byte(a1) as u32) << 8)
                | ((bus.read_byte(a2) as u32) << 16)
                | ((bus.read_byte(a3) as u32) << 24)
        };
        self.commit_sp(sp.wrapping_add(4));
        Ok(value)
    }

    fn load_segment(&mut self, seg: SegReg32, selector: u16, bus: &mut impl common::Bus) -> Step {
        if !self.is_protected_mode() || self.is_virtual_mode() {
            self.sregs[seg as usize] = selector;
            self.set_real_segment_cache(seg, selector);
            return Ok(());
        }
        self.load_protected_segment(seg, selector, bus)
    }

    fn descriptor_addr_checked(&self, selector: u16) -> Option<u32> {
        if selector & 0xFFFC == 0 {
            return None;
        }
        let (table_base, table_limit) = if selector & 4 != 0 {
            (self.ldtr_base, self.ldtr_limit)
        } else {
            (self.gdt_base, self.gdt_limit as u32)
        };
        let index = (selector & !7) as u32;
        if index.wrapping_add(7) > table_limit {
            return None;
        }
        Some(table_base.wrapping_add(index))
    }

    /// Sets the accessed bit on the descriptor for `selector`. Returns
    /// `Err(Fault)` if the descriptor byte's translation faulted (the
    /// page fault is already pending); callers must abort the segment
    /// commit in that case so the load is restartable. Null/out-of-table
    /// selectors return `Ok(())` without touching memory - those
    /// cases are unreachable from a successful `decode_descriptor`.
    fn set_accessed_bit(&mut self, selector: u16, bus: &mut impl common::Bus) -> Step {
        let Some(addr) = self.descriptor_addr_checked(selector) else {
            return Ok(());
        };
        let saved_supervisor_override = self.supervisor_override;
        self.supervisor_override = true;
        let linear = addr.wrapping_add(5);
        let phys = self.translate_linear(linear, true, bus);
        self.supervisor_override = saved_supervisor_override;
        let phys = phys?;
        let rights = bus.read_byte(phys);
        if rights & 0x01 == 0 {
            bus.write_byte(phys, rights | 0x01);
        }
        Ok(())
    }

    pub(super) fn read_word_linear(&mut self, bus: &mut impl common::Bus, addr: u32) -> Step<u16> {
        self.clk_sx_bus(addr, 2);
        if addr & 0xFFF <= 0xFFE {
            let a0 = self.translate_linear(addr, false, bus)?;
            return Ok(bus.read_word(a0));
        }
        let a0 = self.translate_linear(addr, false, bus)?;
        let a1 = self.translate_linear(addr.wrapping_add(1), false, bus)?;
        Ok(bus.read_byte(a0) as u16 | ((bus.read_byte(a1) as u16) << 8))
    }

    pub(super) fn read_word_linear_for_update(
        &mut self,
        bus: &mut impl common::Bus,
        addr: u32,
    ) -> Step<u16> {
        self.clk_sx_bus(addr, 2);
        if addr & 0xFFF <= 0xFFE {
            let a0 = self.translate_linear(addr, true, bus)?;
            return Ok(bus.read_word(a0));
        }
        let a0 = self.translate_linear(addr, true, bus)?;
        let a1 = self.translate_linear(addr.wrapping_add(1), true, bus)?;
        Ok(bus.read_byte(a0) as u16 | ((bus.read_byte(a1) as u16) << 8))
    }

    pub(super) fn write_word_linear(
        &mut self,
        bus: &mut impl common::Bus,
        addr: u32,
        value: u16,
    ) -> Step {
        self.clk_sx_bus(addr, 2);
        if addr & 0xFFF <= 0xFFE {
            let a0 = self.translate_linear(addr, true, bus)?;
            bus.write_word(a0, value);
            return Ok(());
        }
        let a0 = self.translate_linear(addr, true, bus)?;
        let a1 = self.translate_linear(addr.wrapping_add(1), true, bus)?;
        bus.write_byte(a0, value as u8);
        bus.write_byte(a1, (value >> 8) as u8);
        Ok(())
    }

    fn read_dword_linear(&mut self, bus: &mut impl common::Bus, addr: u32) -> Step<u32> {
        self.clk_sx_bus(addr, 4);
        if addr & 0xFFF <= 0xFFC {
            let a0 = self.translate_linear(addr, false, bus)?;
            return Ok(bus.read_dword(a0));
        }
        let a0 = self.translate_linear(addr, false, bus)?;
        let a1 = self.translate_linear(addr.wrapping_add(1), false, bus)?;
        let a2 = self.translate_linear(addr.wrapping_add(2), false, bus)?;
        let a3 = self.translate_linear(addr.wrapping_add(3), false, bus)?;
        Ok(bus.read_byte(a0) as u32
            | ((bus.read_byte(a1) as u32) << 8)
            | ((bus.read_byte(a2) as u32) << 16)
            | ((bus.read_byte(a3) as u32) << 24))
    }

    pub(super) fn read_dword_linear_for_update(
        &mut self,
        bus: &mut impl common::Bus,
        addr: u32,
    ) -> Step<u32> {
        self.clk_sx_bus(addr, 4);
        if addr & 0xFFF <= 0xFFC {
            let a0 = self.translate_linear(addr, true, bus)?;
            return Ok(bus.read_dword(a0));
        }
        let a0 = self.translate_linear(addr, true, bus)?;
        let a1 = self.translate_linear(addr.wrapping_add(1), true, bus)?;
        let a2 = self.translate_linear(addr.wrapping_add(2), true, bus)?;
        let a3 = self.translate_linear(addr.wrapping_add(3), true, bus)?;
        Ok(bus.read_byte(a0) as u32
            | ((bus.read_byte(a1) as u32) << 8)
            | ((bus.read_byte(a2) as u32) << 16)
            | ((bus.read_byte(a3) as u32) << 24))
    }

    fn write_dword_linear(&mut self, bus: &mut impl common::Bus, addr: u32, value: u32) -> Step {
        self.clk_sx_bus(addr, 4);
        if addr & 0xFFF <= 0xFFC {
            let a0 = self.translate_linear(addr, true, bus)?;
            bus.write_dword(a0, value);
            return Ok(());
        }
        let a0 = self.translate_linear(addr, true, bus)?;
        let a1 = self.translate_linear(addr.wrapping_add(1), true, bus)?;
        let a2 = self.translate_linear(addr.wrapping_add(2), true, bus)?;
        let a3 = self.translate_linear(addr.wrapping_add(3), true, bus)?;
        bus.write_byte(a0, value as u8);
        bus.write_byte(a1, (value >> 8) as u8);
        bus.write_byte(a2, (value >> 16) as u8);
        bus.write_byte(a3, (value >> 24) as u8);
        Ok(())
    }

    fn switch_task(&mut self, ntask: u16, task_type: TaskType, bus: &mut impl common::Bus) -> Step {
        let saved_supervisor_override = self.supervisor_override;
        self.supervisor_override = true;
        let result = self.switch_task_inner(ntask, task_type, bus);
        self.supervisor_override = saved_supervisor_override;
        result
    }

    /// Pre-translate (with write permission) every byte covered by the
    /// linear range `[start, end_inclusive]`, walking page-by-page so any
    /// #PF on the underlying PTE surfaces before commit.
    fn pre_translate_range(
        &mut self,
        start: u32,
        end_inclusive: u32,
        write: bool,
        bus: &mut impl common::Bus,
    ) -> Step {
        let start_page = start & !0xFFF;
        let end_page = end_inclusive & !0xFFF;
        let mut page = start_page;
        loop {
            let probe = if page == start_page { start } else { page };
            self.translate_linear(probe, write, bus)?;
            if page == end_page {
                break;
            }
            page = page.wrapping_add(0x1000);
        }
        Ok(())
    }

    fn switch_task_inner(
        &mut self,
        ntask: u16,
        task_type: TaskType,
        bus: &mut impl common::Bus,
    ) -> Step {
        if ntask & 0x0004 != 0 {
            return self.raise_fault_with_code(10, Self::segment_error_code(ntask), bus);
        }

        let Some(naddr) = self.descriptor_addr_checked(ntask) else {
            return self.raise_fault_with_code(10, Self::segment_error_code(ntask), bus);
        };

        let ndesc = self.decode_descriptor_at(naddr, bus)?;
        let ndesc_limit = ndesc.limit;
        let ndesc_base = ndesc.base;
        let ndesc_rights = ndesc.rights;

        let r = ndesc_rights;
        let tss_type = r & 0x0F;
        if Self::descriptor_is_segment(r) || !matches!(tss_type, 1 | 3 | 9 | 11) {
            return self.raise_fault_with_code(13, Self::segment_error_code(ntask), bus);
        }
        if task_type == TaskType::Iret {
            if r & 0x02 == 0 {
                return self.raise_fault_with_code(13, Self::segment_error_code(ntask), bus);
            }
        } else if r & 0x02 != 0 {
            return self.raise_fault_with_code(13, Self::segment_error_code(ntask), bus);
        }

        if !Self::descriptor_present(r) {
            return self.raise_fault_with_code(11, Self::segment_error_code(ntask), bus);
        }

        let is_386_tss = matches!(tss_type, 9 | 11);
        let min_limit: u32 = if is_386_tss { 103 } else { 43 };
        if ndesc_limit < min_limit {
            return self.raise_fault_with_code(10, Self::segment_error_code(ntask), bus);
        }

        let old_base = self.tr_base;
        let old_is_386 = self.tr_rights & 0x0F >= 9;
        let oaddr_opt = self.descriptor_addr_checked(self.tr);

        // Pre-translate every page the dispatch will write to,
        // before any commits. A #PF here aborts cleanly with the old TSS
        // descriptor still marked busy and the old TSS body untouched.
        let (save_start_off, save_end_off): (u32, u32) = if old_is_386 {
            (32, 95) // EIP..GS, last dword occupies 92..95
        } else {
            (14, 41) // IP..DS, last word occupies 40..41
        };
        self.pre_translate_range(
            old_base.wrapping_add(save_start_off),
            old_base.wrapping_add(save_end_off),
            true,
            bus,
        )?;
        if task_type == TaskType::Call {
            // Back-link write at new TSS base, 2 bytes.
            self.pre_translate_range(ndesc_base, ndesc_base.wrapping_add(1), true, bus)?;
        }
        if task_type != TaskType::Call
            && let Some(oaddr) = oaddr_opt
        {
            // Old TSS busy-bit byte (will be cleared).
            self.translate_linear(oaddr.wrapping_add(5), true, bus)?;
        }
        if task_type != TaskType::Iret {
            // New TSS busy-bit byte (will be set).
            self.translate_linear(naddr.wrapping_add(5), true, bus)?;
        }
        if is_386_tss {
            // T-bit byte (read at the end of the switch).
            self.translate_linear(ndesc_base.wrapping_add(100), false, bus)?;
        }

        let mut flags = self.flags.compress();

        if task_type == TaskType::Iret {
            flags &= !0x4000;
        }

        // Commit. New TSS marked busy FIRST so a transient
        // observer never sees both TSSes idle (matters when a host fault
        // handler peeks at the GDT). The old-TSS busy-bit clear, old-TSS
        // state save, back-link write, and TR / CR3 / regs updates follow.

        // Mark new TSS busy (CALL/JMP) before any other commit.
        let nlinear = naddr.wrapping_add(5);
        let new_rights = if task_type != TaskType::Iret {
            let nphys = self.translate_linear(nlinear, true, bus)?;
            let r = bus.read_byte(nphys);
            bus.write_byte(nphys, r | 0x02);
            r | 0x02
        } else {
            let nphys = self.translate_linear(nlinear, false, bus)?;
            bus.read_byte(nphys)
        };

        // Mark old TSS idle (JMP/IRET).
        if task_type != TaskType::Call
            && let Some(oaddr) = oaddr_opt
        {
            let olinear = oaddr.wrapping_add(5);
            let ophys = self.translate_linear(olinear, true, bus)?;
            let old_rights = bus.read_byte(ophys);
            bus.write_byte(ophys, old_rights & !0x02);
        }

        // Save current state to old TSS.
        if old_is_386 {
            let eip = self.ip_upper | self.ip as u32;
            let eflags = self.eflags_upper | flags as u32;
            self.write_dword_linear(bus, old_base.wrapping_add(32), eip)?;
            self.write_dword_linear(bus, old_base.wrapping_add(36), eflags)?;
            self.write_dword_linear(
                bus,
                old_base.wrapping_add(40),
                self.regs.dword(crate::DwordReg::EAX),
            )?;
            self.write_dword_linear(
                bus,
                old_base.wrapping_add(44),
                self.regs.dword(crate::DwordReg::ECX),
            )?;
            self.write_dword_linear(
                bus,
                old_base.wrapping_add(48),
                self.regs.dword(crate::DwordReg::EDX),
            )?;
            self.write_dword_linear(
                bus,
                old_base.wrapping_add(52),
                self.regs.dword(crate::DwordReg::EBX),
            )?;
            self.write_dword_linear(
                bus,
                old_base.wrapping_add(56),
                self.regs.dword(crate::DwordReg::ESP),
            )?;
            self.write_dword_linear(
                bus,
                old_base.wrapping_add(60),
                self.regs.dword(crate::DwordReg::EBP),
            )?;
            self.write_dword_linear(
                bus,
                old_base.wrapping_add(64),
                self.regs.dword(crate::DwordReg::ESI),
            )?;
            self.write_dword_linear(
                bus,
                old_base.wrapping_add(68),
                self.regs.dword(crate::DwordReg::EDI),
            )?;
            self.write_dword_linear(
                bus,
                old_base.wrapping_add(72),
                self.sregs[SegReg32::ES as usize] as u32,
            )?;
            self.write_dword_linear(
                bus,
                old_base.wrapping_add(76),
                self.sregs[SegReg32::CS as usize] as u32,
            )?;
            self.write_dword_linear(
                bus,
                old_base.wrapping_add(80),
                self.sregs[SegReg32::SS as usize] as u32,
            )?;
            self.write_dword_linear(
                bus,
                old_base.wrapping_add(84),
                self.sregs[SegReg32::DS as usize] as u32,
            )?;
            self.write_dword_linear(
                bus,
                old_base.wrapping_add(88),
                self.sregs[SegReg32::FS as usize] as u32,
            )?;
            self.write_dword_linear(
                bus,
                old_base.wrapping_add(92),
                self.sregs[SegReg32::GS as usize] as u32,
            )?;
        } else {
            self.write_word_linear(bus, old_base.wrapping_add(14), self.ip)?;
            self.write_word_linear(bus, old_base.wrapping_add(16), flags)?;
            self.write_word_linear(bus, old_base.wrapping_add(18), self.regs.word(WordReg::AX))?;
            self.write_word_linear(bus, old_base.wrapping_add(20), self.regs.word(WordReg::CX))?;
            self.write_word_linear(bus, old_base.wrapping_add(22), self.regs.word(WordReg::DX))?;
            self.write_word_linear(bus, old_base.wrapping_add(24), self.regs.word(WordReg::BX))?;
            self.write_word_linear(bus, old_base.wrapping_add(26), self.regs.word(WordReg::SP))?;
            self.write_word_linear(bus, old_base.wrapping_add(28), self.regs.word(WordReg::BP))?;
            self.write_word_linear(bus, old_base.wrapping_add(30), self.regs.word(WordReg::SI))?;
            self.write_word_linear(bus, old_base.wrapping_add(32), self.regs.word(WordReg::DI))?;
            self.write_word_linear(
                bus,
                old_base.wrapping_add(34),
                self.sregs[SegReg32::ES as usize],
            )?;
            self.write_word_linear(
                bus,
                old_base.wrapping_add(36),
                self.sregs[SegReg32::CS as usize],
            )?;
            self.write_word_linear(
                bus,
                old_base.wrapping_add(38),
                self.sregs[SegReg32::SS as usize],
            )?;
            self.write_word_linear(
                bus,
                old_base.wrapping_add(40),
                self.sregs[SegReg32::DS as usize],
            )?;
        }

        // Write back-link to new TSS after old state is saved.
        if task_type == TaskType::Call {
            self.write_word_linear(bus, ndesc_base, self.tr)?;
        }

        // Read all fields from new TSS.
        let new_base = ndesc_base;

        let (
            ntss_cr3,
            ntss_eip,
            ntss_eflags,
            ntss_eax,
            ntss_ecx,
            ntss_edx,
            ntss_ebx,
            ntss_esp,
            ntss_ebp,
            ntss_esi,
            ntss_edi,
            ntss_es,
            ntss_cs,
            ntss_ss,
            ntss_ds,
            ntss_fs,
            ntss_gs,
            ntss_ldt,
        );

        if is_386_tss {
            ntss_cr3 = self.read_dword_linear(bus, new_base.wrapping_add(28))?;
            ntss_eip = self.read_dword_linear(bus, new_base.wrapping_add(32))?;
            ntss_eflags = self.read_dword_linear(bus, new_base.wrapping_add(36))?;
            ntss_eax = self.read_dword_linear(bus, new_base.wrapping_add(40))?;
            ntss_ecx = self.read_dword_linear(bus, new_base.wrapping_add(44))?;
            ntss_edx = self.read_dword_linear(bus, new_base.wrapping_add(48))?;
            ntss_ebx = self.read_dword_linear(bus, new_base.wrapping_add(52))?;
            ntss_esp = self.read_dword_linear(bus, new_base.wrapping_add(56))?;
            ntss_ebp = self.read_dword_linear(bus, new_base.wrapping_add(60))?;
            ntss_esi = self.read_dword_linear(bus, new_base.wrapping_add(64))?;
            ntss_edi = self.read_dword_linear(bus, new_base.wrapping_add(68))?;
            ntss_es = self.read_word_linear(bus, new_base.wrapping_add(72))?;
            ntss_cs = self.read_word_linear(bus, new_base.wrapping_add(76))?;
            ntss_ss = self.read_word_linear(bus, new_base.wrapping_add(80))?;
            ntss_ds = self.read_word_linear(bus, new_base.wrapping_add(84))?;
            ntss_fs = self.read_word_linear(bus, new_base.wrapping_add(88))?;
            ntss_gs = self.read_word_linear(bus, new_base.wrapping_add(92))?;
            ntss_ldt = self.read_word_linear(bus, new_base.wrapping_add(96))?;
        } else {
            ntss_cr3 = 0;
            ntss_eip = self.read_word_linear(bus, new_base.wrapping_add(14))? as u32;
            ntss_eflags = self.read_word_linear(bus, new_base.wrapping_add(16))? as u32;
            ntss_eax = self.read_word_linear(bus, new_base.wrapping_add(18))? as u32;
            ntss_ecx = self.read_word_linear(bus, new_base.wrapping_add(20))? as u32;
            ntss_edx = self.read_word_linear(bus, new_base.wrapping_add(22))? as u32;
            ntss_ebx = self.read_word_linear(bus, new_base.wrapping_add(24))? as u32;
            ntss_esp = self.read_word_linear(bus, new_base.wrapping_add(26))? as u32;
            ntss_ebp = self.read_word_linear(bus, new_base.wrapping_add(28))? as u32;
            ntss_esi = self.read_word_linear(bus, new_base.wrapping_add(30))? as u32;
            ntss_edi = self.read_word_linear(bus, new_base.wrapping_add(32))? as u32;
            ntss_es = self.read_word_linear(bus, new_base.wrapping_add(34))?;
            ntss_cs = self.read_word_linear(bus, new_base.wrapping_add(36))?;
            ntss_ss = self.read_word_linear(bus, new_base.wrapping_add(38))?;
            ntss_ds = self.read_word_linear(bus, new_base.wrapping_add(40))?;
            ntss_ldt = self.read_word_linear(bus, new_base.wrapping_add(42))?;
            ntss_fs = 0;
            ntss_gs = 0;
        }

        // Update TR.
        self.tr = ntask;
        self.tr_limit = ndesc_limit;
        self.tr_base = ndesc_base;
        self.tr_rights = new_rights;

        // Load CR3 from the new 386 TSS while paging is enabled. The new task's
        // page directory must be installed before any subsequent linear access
        // (LDT load, segment descriptor reads) so those resolve against the
        // new task's page tables.
        if is_386_tss && self.is_paging_enabled() {
            self.cr3 = ntss_cr3;
            self.flush_tlb();
        }

        // Load registers from new TSS.
        self.flags.expand(ntss_eflags as u16);
        self.eflags_upper = ntss_eflags & Self::eflags_upper_writable();
        self.preserve_resume_flag = true;
        self.regs.set_dword(crate::DwordReg::EAX, ntss_eax);
        self.regs.set_dword(crate::DwordReg::ECX, ntss_ecx);
        self.regs.set_dword(crate::DwordReg::EDX, ntss_edx);
        self.regs.set_dword(crate::DwordReg::EBX, ntss_ebx);
        self.regs.set_dword(crate::DwordReg::ESP, ntss_esp);
        self.regs.set_dword(crate::DwordReg::EBP, ntss_ebp);
        self.regs.set_dword(crate::DwordReg::ESI, ntss_esi);
        self.regs.set_dword(crate::DwordReg::EDI, ntss_edi);

        // Load LDT from new TSS.
        if ntss_ldt & 0x0004 != 0 {
            return self.raise_fault_with_code(10, Self::segment_error_code(ntss_ldt), bus);
        }
        if ntss_ldt & 0xFFFC != 0 {
            let Some(ldtaddr) = self.descriptor_addr_checked(ntss_ldt) else {
                return self.raise_fault_with_code(10, Self::segment_error_code(ntss_ldt), bus);
            };
            let ldt_desc = self.decode_descriptor_at(ldtaddr, bus)?;
            let lr = ldt_desc.rights;
            if Self::descriptor_is_segment(lr) || (lr & 0x0F) != 0x02 {
                return self.raise_fault_with_code(10, Self::segment_error_code(ntss_ldt), bus);
            }
            if !Self::descriptor_present(lr) {
                return self.raise_fault_with_code(10, Self::segment_error_code(ntss_ldt), bus);
            }
            let ldt_base = ldt_desc.base;
            let ldt_limit = ldt_desc.limit;
            self.ldtr = ntss_ldt;
            self.ldtr_base = ldt_base;
            self.ldtr_limit = ldt_limit;
        } else {
            self.ldtr = 0;
            self.ldtr_base = 0;
            self.ldtr_limit = 0;
        }

        if task_type == TaskType::Call {
            self.flags.nt = true;
        }

        self.cr0 |= 8;

        // TSS T-bit (offset 100, bit 0): schedule a debug trap (#DB) after the
        // first instruction of the new task. Sets DR6.BT (bit 15).
        if is_386_tss {
            let debug_trap_word = self.read_word_linear(bus, new_base.wrapping_add(100))?;
            if debug_trap_word & 1 != 0 {
                self.debug_trap_pending = true;
            }
        }

        if is_386_tss && (ntss_eflags & 0x0002_0000) != 0 {
            // VM86 task: segment selectors are interpreted with real-mode
            // bases/limits, not protected-mode descriptors.
            self.sregs[SegReg32::ES as usize] = ntss_es;
            self.set_real_segment_cache(SegReg32::ES, ntss_es);
            self.sregs[SegReg32::CS as usize] = ntss_cs;
            self.set_real_segment_cache(SegReg32::CS, ntss_cs);
            self.sregs[SegReg32::SS as usize] = ntss_ss;
            self.set_real_segment_cache(SegReg32::SS, ntss_ss);
            self.sregs[SegReg32::DS as usize] = ntss_ds;
            self.set_real_segment_cache(SegReg32::DS, ntss_ds);
            self.sregs[SegReg32::FS as usize] = ntss_fs;
            self.set_real_segment_cache(SegReg32::FS, ntss_fs);
            self.sregs[SegReg32::GS as usize] = ntss_gs;
            self.set_real_segment_cache(SegReg32::GS, ntss_gs);
            self.ip = ntss_eip as u16;
            self.ip_upper = 0;
            return Ok(());
        }

        // Load segment registers from new TSS. SS first (uses new CS RPL as CPL).
        let new_cpl = ntss_cs & 3;
        self.load_task_data_segment(SegReg32::SS, ntss_ss, new_cpl, bus)?;
        self.load_task_code_segment(ntss_cs, ntss_eip, bus)?;
        let cpl = self.cpl();
        self.load_task_data_segment(SegReg32::ES, ntss_es, cpl, bus)?;
        self.load_task_data_segment(SegReg32::DS, ntss_ds, cpl, bus)?;
        self.load_task_data_segment(SegReg32::FS, ntss_fs, cpl, bus)?;
        self.load_task_data_segment(SegReg32::GS, ntss_gs, cpl, bus)?;

        Ok(())
    }

    fn load_task_data_segment(
        &mut self,
        seg: SegReg32,
        selector: u16,
        required_cpl: u16,
        bus: &mut impl common::Bus,
    ) -> Step {
        if selector & 0xFFFC == 0 {
            self.set_null_segment(seg, selector);
            return Ok(());
        }
        let Some(descriptor) = self.decode_descriptor(selector, bus)? else {
            return self.raise_fault_with_code(10, Self::segment_error_code(selector), bus);
        };
        let rights = descriptor.rights;
        if seg == SegReg32::SS {
            if !Self::descriptor_is_segment(rights) || !Self::descriptor_is_writable(rights) {
                return self.raise_fault_with_code(10, Self::segment_error_code(selector), bus);
            }
            let dpl = Self::descriptor_dpl(rights);
            let rpl = selector & 3;
            if dpl != required_cpl || rpl != required_cpl {
                return self.raise_fault_with_code(10, Self::segment_error_code(selector), bus);
            }
        } else {
            if !Self::descriptor_is_segment(rights) || !Self::descriptor_is_readable(rights) {
                return self.raise_fault_with_code(10, Self::segment_error_code(selector), bus);
            }
            if !Self::descriptor_is_conforming_code(rights) {
                let dpl = Self::descriptor_dpl(rights);
                let rpl = selector & 3;
                if dpl < required_cpl.max(rpl) {
                    return self.raise_fault_with_code(10, Self::segment_error_code(selector), bus);
                }
            }
        }
        if !Self::descriptor_present(rights) {
            return self.raise_fault_with_code(10, Self::segment_error_code(selector), bus);
        }
        self.set_accessed_bit(selector, bus)?;
        self.set_loaded_segment_cache(seg, selector, descriptor);
        Ok(())
    }

    fn load_task_code_segment(
        &mut self,
        selector: u16,
        offset: u32,
        bus: &mut impl common::Bus,
    ) -> Step {
        if selector & 0xFFFC == 0 {
            return self.raise_fault_with_code(10, Self::segment_error_code(selector), bus);
        }
        let Some(descriptor) = self.decode_descriptor(selector, bus)? else {
            return self.raise_fault_with_code(10, Self::segment_error_code(selector), bus);
        };
        let rights = descriptor.rights;
        if !Self::descriptor_is_segment(rights) || !Self::descriptor_is_code(rights) {
            return self.raise_fault_with_code(10, Self::segment_error_code(selector), bus);
        }
        let dpl = Self::descriptor_dpl(rights);
        let rpl = selector & 3;
        if Self::descriptor_is_conforming_code(rights) {
            if dpl > rpl {
                return self.raise_fault_with_code(10, Self::segment_error_code(selector), bus);
            }
        } else if dpl != rpl {
            return self.raise_fault_with_code(10, Self::segment_error_code(selector), bus);
        }
        if !Self::descriptor_present(rights) {
            return self.raise_fault_with_code(11, Self::segment_error_code(selector), bus);
        }
        if offset > descriptor.limit {
            return self.raise_fault_with_code(10, 0, bus);
        }
        self.set_accessed_bit(selector, bus)?;
        let adjusted = (selector & !3) | rpl;
        self.set_loaded_segment_cache(SegReg32::CS, adjusted, descriptor);
        self.ip = offset as u16;
        self.ip_upper = offset & 0xFFFF_0000;
        Ok(())
    }

    fn code_descriptor(
        &mut self,
        selector: u16,
        offset: u32,
        gate: TaskType,
        old_cs: u16,
        old_eip: u32,
        bus: &mut impl common::Bus,
    ) -> Step {
        let Some(addr) = self.descriptor_addr_checked(selector) else {
            return self.raise_fault_with_code(13, Self::segment_error_code(selector), bus);
        };

        let desc = self.decode_descriptor_at(addr, bus)?;
        let r = desc.rights;
        let cpl = self.cpl();
        let rpl = selector & 3;

        if Self::descriptor_is_segment(r) {
            if !Self::descriptor_is_code(r) {
                return self.raise_fault_with_code(13, Self::segment_error_code(selector), bus);
            }
            if Self::descriptor_is_conforming_code(r) {
                if Self::descriptor_dpl(r) > cpl {
                    return self.raise_fault_with_code(13, Self::segment_error_code(selector), bus);
                }
            } else if rpl > cpl || Self::descriptor_dpl(r) != cpl {
                return self.raise_fault_with_code(13, Self::segment_error_code(selector), bus);
            }
            if !Self::descriptor_present(r) {
                return self.raise_fault_with_code(11, Self::segment_error_code(selector), bus);
            }
            if offset > desc.limit {
                return self.raise_fault_with_code(13, 0, bus);
            }
            self.set_accessed_bit(selector, bus)?;
            let adjusted = (selector & !3) | cpl;
            self.set_loaded_segment_cache(SegReg32::CS, adjusted, desc);
            self.ip = offset as u16;
            self.ip_upper = offset & 0xFFFF_0000;
            if gate == TaskType::Call {
                if self.operand_size_override {
                    self.push_dword(bus, old_cs as u32)?;
                    self.push_dword(bus, old_eip)?;
                } else {
                    self.push(bus, old_cs)?;
                    self.push(bus, old_eip as u16)?;
                }
            }
            return Ok(());
        }

        // System descriptor.
        let dpl = Self::descriptor_dpl(r);
        if dpl < cpl.max(rpl) {
            return self.raise_fault_with_code(13, Self::segment_error_code(selector), bus);
        }
        if !Self::descriptor_present(r) {
            return self.raise_fault_with_code(11, Self::segment_error_code(selector), bus);
        }

        let gate_type = r & 0x0F;
        match gate_type {
            4 | 12 => {
                // Call gate (4=286, 12=386).
                let is_386_gate = gate_type == 12;
                let gate_selector = self.read_word_linear(bus, addr.wrapping_add(2))?;
                let gc_linear = addr.wrapping_add(4);
                let gc_phys = self.translate_linear(gc_linear, false, bus)?;
                let gate_count = bus.read_byte(gc_phys) & 0x1F;
                let gate_offset = if is_386_gate {
                    let lo = self.read_word_linear(bus, addr)?;
                    let hi = self.read_word_linear(bus, addr.wrapping_add(6))?;
                    lo as u32 | ((hi as u32) << 16)
                } else {
                    let lo = self.read_word_linear(bus, addr)?;
                    lo as u32
                };

                let Some(target_addr) = self.descriptor_addr_checked(gate_selector) else {
                    return self.raise_fault_with_code(
                        13,
                        Self::segment_error_code(gate_selector),
                        bus,
                    );
                };
                let target_desc = self.decode_descriptor_at(target_addr, bus)?;
                let tr = target_desc.rights;
                if !Self::descriptor_is_code(tr) || !Self::descriptor_is_segment(tr) {
                    return self.raise_fault_with_code(
                        13,
                        Self::segment_error_code(gate_selector),
                        bus,
                    );
                }
                let target_dpl = Self::descriptor_dpl(tr);
                if target_dpl > cpl {
                    return self.raise_fault_with_code(
                        13,
                        Self::segment_error_code(gate_selector),
                        bus,
                    );
                }
                if !Self::descriptor_present(tr) {
                    return self.raise_fault_with_code(
                        11,
                        Self::segment_error_code(gate_selector),
                        bus,
                    );
                }
                if gate_offset > target_desc.limit {
                    return self.raise_fault_with_code(13, 0, bus);
                }

                if !Self::descriptor_is_conforming_code(tr) && target_dpl < cpl {
                    // Inter-privilege call via call gate.
                    if gate == TaskType::Jmp {
                        return self.raise_fault_with_code(
                            13,
                            Self::segment_error_code(gate_selector),
                            bus,
                        );
                    }
                    self.commit_call_gate_inter_priv(
                        target_desc,
                        target_dpl,
                        gate_selector,
                        gate_offset,
                        is_386_gate,
                        gate_count,
                        old_cs,
                        old_eip,
                        bus,
                    )?;
                } else {
                    self.commit_call_gate_intra_priv(
                        target_desc,
                        gate,
                        gate_selector,
                        gate_offset,
                        is_386_gate,
                        cpl,
                        old_cs,
                        old_eip,
                        bus,
                    )?;
                }
                Ok(())
            }
            5 => {
                // Task gate.
                let task_selector = self.read_word_linear(bus, addr.wrapping_add(2))?;
                self.switch_task(task_selector, gate, bus)?;
                let flags_val = self.flags.compress();
                let cpl = self.cpl();
                self.flags.load_flags(flags_val, cpl, true);
                Ok(())
            }
            1 | 9 => {
                // Available TSS descriptor.
                self.switch_task(selector, gate, bus)?;
                let flags_val = self.flags.compress();
                let cpl = self.cpl();
                self.flags.load_flags(flags_val, cpl, true);
                Ok(())
            }
            3 | 11 => {
                // Busy TSS.
                self.raise_fault_with_code(13, Self::segment_error_code(selector), bus)
            }
            _ => self.raise_fault_with_code(13, Self::segment_error_code(selector), bus),
        }
    }

    /// Inter-privilege CALL through a call gate. Validates every operand and
    /// pre-translates every page the dispatch will touch before committing
    /// any architectural state. A #PF on a parameter read, a return-frame
    /// push slot, or one of the descriptor accessed-bit writes aborts with
    /// SS:ESP and CS:EIP still pointing at the caller's frame, so the fault
    /// is reported inter-privilege from the calling CPL.
    #[allow(clippy::too_many_arguments)]
    fn commit_call_gate_inter_priv(
        &mut self,
        target_desc: SegmentDescriptor,
        target_dpl: u16,
        gate_selector: u16,
        gate_offset: u32,
        is_386_gate: bool,
        gate_count: u8,
        old_cs: u16,
        old_eip: u32,
        bus: &mut impl common::Bus,
    ) -> Step {
        let is_386_tss = self.tr_rights & 0x0F >= 9;
        let tss_sp_offset = if is_386_tss {
            4 + target_dpl as u32 * 8
        } else {
            2 + target_dpl as u32 * 4
        };
        let tss_ss_offset = tss_sp_offset + if is_386_tss { 4 } else { 2 };
        let tss_esp = if is_386_tss {
            self.read_dword_linear(bus, self.tr_base.wrapping_add(tss_sp_offset))?
        } else {
            self.read_word_linear(bus, self.tr_base.wrapping_add(tss_sp_offset))? as u32
        };
        let tss_ss = self.read_word_linear(bus, self.tr_base.wrapping_add(tss_ss_offset))?;

        let saved_ss = self.sregs[SegReg32::SS as usize];
        let saved_esp = if self.use_esp() {
            self.regs.dword(crate::DwordReg::ESP)
        } else {
            self.regs.word(WordReg::SP) as u32
        };
        let old_ss_base = self.seg_base(SegReg32::SS);
        let old_ss_is_32bit = self.seg_granularity[SegReg32::SS as usize] & 0x40 != 0;

        if tss_ss & 0xFFFC == 0 {
            return self.raise_fault_with_code(10, 0, bus);
        }
        let Some(new_ss_desc) = self.decode_descriptor(tss_ss, bus)? else {
            return self.raise_fault_with_code(10, Self::segment_error_code(tss_ss), bus);
        };
        let new_ss_rights = new_ss_desc.rights;
        if !Self::descriptor_is_segment(new_ss_rights)
            || !Self::descriptor_is_writable(new_ss_rights)
        {
            return self.raise_fault_with_code(10, Self::segment_error_code(tss_ss), bus);
        }
        let new_ss_dpl = Self::descriptor_dpl(new_ss_rights);
        let new_ss_rpl = tss_ss & 3;
        if new_ss_dpl != target_dpl || new_ss_rpl != target_dpl {
            return self.raise_fault_with_code(10, Self::segment_error_code(tss_ss), bus);
        }
        if !Self::descriptor_present(new_ss_rights) {
            return self.raise_fault_with_code(10, Self::segment_error_code(tss_ss), bus);
        }

        // Stack-space pre-check: verify the new stack has room for the
        // parameters plus the return frame (SS, ESP, CS, EIP) before any
        // state is mutated.
        let bytes_per_push: u32 = if is_386_gate { 4 } else { 2 };
        let total_push_bytes = bytes_per_push * (4 + gate_count as u32);
        let new_ss_d_bit = new_ss_desc.granularity & 0x40 != 0;
        let stack_top = if new_ss_d_bit {
            tss_esp
        } else {
            tss_esp & 0xFFFF
        };
        let stack_space_ok = if Self::descriptor_is_expand_down(new_ss_rights) {
            stack_top
                .checked_sub(total_push_bytes)
                .is_some_and(|new_esp| new_esp > new_ss_desc.limit)
        } else {
            stack_top >= total_push_bytes
        };
        if !stack_space_ok {
            return self.raise_fault_with_code(12, 0, bus);
        }

        // Pre-translate every new-stack push slot (saved_ss, saved_esp,
        // gate_count parameters, and the outer CS:EIP push) before any
        // commits.
        let new_ss_base = new_ss_desc.base;
        let total_pushes: u32 = 4 + gate_count as u32;
        for i in 1..=total_pushes {
            let raw_off = tss_esp.wrapping_sub(i * bytes_per_push);
            let off_lo = if new_ss_d_bit {
                raw_off
            } else {
                raw_off as u16 as u32
            };
            let linear_lo = new_ss_base.wrapping_add(off_lo);
            self.translate_linear(linear_lo, true, bus)?;
            if bytes_per_push > 1 {
                let raw_hi = raw_off.wrapping_add(bytes_per_push - 1);
                let off_hi = if new_ss_d_bit {
                    raw_hi
                } else {
                    raw_hi as u16 as u32
                };
                let linear_hi = new_ss_base.wrapping_add(off_hi);
                if (linear_hi & !0xFFF) != (linear_lo & !0xFFF) {
                    self.translate_linear(linear_hi, true, bus)?;
                }
            }
        }

        // Read all parameters from the caller's stack now, before any
        // commit. Buffer them so the subsequent push phase has no faulting
        // operations.
        let mut params: [u32; 31] = [0; 31];
        for i in 0..gate_count as u32 {
            let offset_raw = saved_esp.wrapping_add(i * bytes_per_push);
            let offset = if old_ss_is_32bit {
                offset_raw
            } else {
                offset_raw & 0xFFFF
            };
            let linear = old_ss_base.wrapping_add(offset);
            params[i as usize] = if is_386_gate {
                self.read_dword_linear(bus, linear)?
            } else {
                self.read_word_linear(bus, linear)? as u32
            };
        }

        // Set descriptor accessed bits before any commit. On 486 + CR0.WP=1
        // these writes can #PF on a read-only descriptor-table page, so they
        // must occur before SS/CS caches are updated.
        self.set_accessed_bit(tss_ss, bus)?;
        self.set_accessed_bit(gate_selector, bus)?;

        // Commit: SS cache + ESP, push the inter-priv prologue, CS cache +
        // EIP, push the return frame. Every push from here on hits a
        // pre-translated page so cannot #PF.
        self.set_loaded_segment_cache(SegReg32::SS, tss_ss, new_ss_desc);
        if self.use_esp() {
            self.regs.set_dword(crate::DwordReg::ESP, tss_esp);
        } else {
            self.regs.set_word(WordReg::SP, tss_esp as u16);
        }

        if is_386_gate {
            self.push_dword(bus, saved_ss as u32)?;
            self.push_dword(bus, saved_esp)?;
            for i in (0..gate_count as u32).rev() {
                self.push_dword(bus, params[i as usize])?;
            }
        } else {
            self.push(bus, saved_ss)?;
            self.push(bus, saved_esp as u16)?;
            for i in (0..gate_count as u32).rev() {
                self.push(bus, params[i as usize] as u16)?;
            }
        }

        let adjusted = (gate_selector & !3) | target_dpl;
        self.set_loaded_segment_cache(SegReg32::CS, adjusted, target_desc);
        self.ip = gate_offset as u16;
        self.ip_upper = gate_offset & 0xFFFF_0000;

        if is_386_gate {
            self.push_dword(bus, old_cs as u32)?;
            self.push_dword(bus, old_eip)?;
        } else {
            self.push(bus, old_cs)?;
            self.push(bus, old_eip as u16)?;
        }
        Ok(())
    }

    /// Intra-privilege CALL or JMP through a call gate. Pre-translates the
    /// outer CS:EIP push slots (CALL only) before A-bit and CS cache commit
    /// so a fault on the push can abort with CS:EIP unchanged.
    #[allow(clippy::too_many_arguments)]
    fn commit_call_gate_intra_priv(
        &mut self,
        target_desc: SegmentDescriptor,
        gate: TaskType,
        gate_selector: u16,
        gate_offset: u32,
        is_386_gate: bool,
        cpl: u16,
        old_cs: u16,
        old_eip: u32,
        bus: &mut impl common::Bus,
    ) -> Step {
        // Pre-translate the outer push slots on the current stack so the
        // CALL's return frame push cannot fault after the CS cache has
        // already been committed.
        if gate == TaskType::Call {
            let bytes_per_push: u32 = if is_386_gate { 4 } else { 2 };
            let ss_base = self.seg_base(SegReg32::SS);
            let ss_b_bit = self.seg_granularity[SegReg32::SS as usize] & 0x40 != 0;
            let current_esp = if self.use_esp() {
                self.regs.dword(crate::DwordReg::ESP)
            } else {
                self.regs.word(WordReg::SP) as u32
            };
            for i in 1..=2u32 {
                let raw_off = current_esp.wrapping_sub(i * bytes_per_push);
                let off_lo = if ss_b_bit {
                    raw_off
                } else {
                    raw_off as u16 as u32
                };
                let linear_lo = ss_base.wrapping_add(off_lo);
                self.translate_linear(linear_lo, true, bus)?;
                if bytes_per_push > 1 {
                    let raw_hi = raw_off.wrapping_add(bytes_per_push - 1);
                    let off_hi = if ss_b_bit {
                        raw_hi
                    } else {
                        raw_hi as u16 as u32
                    };
                    let linear_hi = ss_base.wrapping_add(off_hi);
                    if (linear_hi & !0xFFF) != (linear_lo & !0xFFF) {
                        self.translate_linear(linear_hi, true, bus)?;
                    }
                }
            }
        }

        self.set_accessed_bit(gate_selector, bus)?;
        let adjusted = (gate_selector & !3) | cpl;
        self.set_loaded_segment_cache(SegReg32::CS, adjusted, target_desc);
        self.ip = gate_offset as u16;
        self.ip_upper = gate_offset & 0xFFFF_0000;
        if gate == TaskType::Call {
            if is_386_gate {
                self.push_dword(bus, old_cs as u32)?;
                self.push_dword(bus, old_eip)?;
            } else {
                self.push(bus, old_cs)?;
                self.push(bus, old_eip as u16)?;
            }
        }
        Ok(())
    }

    /// Reads the 8-byte descriptor at `addr` (a linear address).
    /// Returns `Err(Fault)` if any byte's translation faulted (the page
    /// fault is already pending). Callers must propagate as an abort;
    /// never fall back to a sentinel descriptor.
    #[inline]
    fn read_descriptor_byte(&mut self, linear: u32, bus: &mut impl common::Bus) -> Step<u8> {
        let phys = self.translate_linear(linear, false, bus)?;
        Ok(bus.read_byte(phys))
    }

    fn decode_descriptor_at(
        &mut self,
        addr: u32,
        bus: &mut impl common::Bus,
    ) -> Step<SegmentDescriptor> {
        let saved_supervisor_override = self.supervisor_override;
        self.supervisor_override = true;
        // translate_linear short-circuits once fault_pending is set, so the
        // later reads after a fault on byte N are cheap no-ops returning
        // Err. Collect first, restore the override, then propagate.
        // Read all bytes first so that a fault on byte N still restores the
        // supervisor_override before we propagate. translate_linear short-
        // circuits once fault_pending is set, so reads after the first fault
        // are cheap no-ops that propagate Err.
        let b0 = self.read_descriptor_byte(addr, bus);
        let b1 = self.read_descriptor_byte(addr.wrapping_add(1), bus);
        let b2 = self.read_descriptor_byte(addr.wrapping_add(2), bus);
        let b3 = self.read_descriptor_byte(addr.wrapping_add(3), bus);
        let b4 = self.read_descriptor_byte(addr.wrapping_add(4), bus);
        let rights = self.read_descriptor_byte(addr.wrapping_add(5), bus);
        let b6 = self.read_descriptor_byte(addr.wrapping_add(6), bus);
        let b7 = self.read_descriptor_byte(addr.wrapping_add(7), bus);
        self.supervisor_override = saved_supervisor_override;
        let b0 = b0?;
        let b1 = b1?;
        let b2 = b2?;
        let b3 = b3?;
        let b4 = b4?;
        let rights = rights?;
        let b6 = b6?;
        let b7 = b7?;

        let raw_limit = b0 as u32 | ((b1 as u32) << 8) | (((b6 & 0x0F) as u32) << 16);
        let base = b2 as u32 | ((b3 as u32) << 8) | ((b4 as u32) << 16) | ((b7 as u32) << 24);
        let limit = if b6 & 0x80 != 0 {
            (raw_limit << 12) | 0xFFF
        } else {
            raw_limit
        };
        Ok(SegmentDescriptor {
            base,
            limit,
            rights,
            granularity: b6,
        })
    }

    fn execute_with_prefixes(&mut self, bus: &mut impl common::Bus) -> Step {
        // 80486 PRM 9.9.13: instructions longer than 15 bytes raise #GP.
        // Track prefix bytes consumed and bail if a 15th would be next:
        // a 15-byte all-prefix run still requires an opcode byte, so the
        // total length necessarily exceeds the limit.
        let mut prefix_count: u32 = 0;
        let mut opcode = self.fetch(bus);
        if self.fault_pending {
            return Err(Fault);
        }
        loop {
            match opcode {
                0x26 | 0x2E | 0x36 | 0x3E | 0x64 | 0x65 | 0x66 | 0x67 | 0xF0 => {
                    if unlikely(prefix_count >= 14) {
                        return self.raise_fault_with_code(13, 0, bus);
                    }
                    prefix_count += 1;
                }
                _ => {}
            }
            match opcode {
                0x26 => {
                    self.seg_prefix = true;
                    self.prefix_seg = SegReg32::ES;
                    self.clk(Self::timing(0, 1));
                    opcode = self.fetch(bus);
                }
                0x2E => {
                    self.seg_prefix = true;
                    self.prefix_seg = SegReg32::CS;
                    self.clk(Self::timing(0, 1));
                    opcode = self.fetch(bus);
                }
                0x36 => {
                    self.seg_prefix = true;
                    self.prefix_seg = SegReg32::SS;
                    self.clk(Self::timing(0, 1));
                    opcode = self.fetch(bus);
                }
                0x3E => {
                    self.seg_prefix = true;
                    self.prefix_seg = SegReg32::DS;
                    self.clk(Self::timing(0, 1));
                    opcode = self.fetch(bus);
                }
                0x64 => {
                    self.seg_prefix = true;
                    self.prefix_seg = SegReg32::FS;
                    self.clk(Self::timing(0, 1));
                    opcode = self.fetch(bus);
                }
                0x65 => {
                    self.seg_prefix = true;
                    self.prefix_seg = SegReg32::GS;
                    self.clk(Self::timing(0, 1));
                    opcode = self.fetch(bus);
                }
                0x66 => {
                    self.operand_size_override = !self.code_segment_32bit();
                    self.clk(Self::timing(0, 1));
                    opcode = self.fetch(bus);
                }
                0x67 => {
                    self.address_size_override = !self.code_segment_32bit();
                    self.clk(Self::timing(0, 1));
                    opcode = self.fetch(bus);
                }
                0xF0 => {
                    self.lock_prefix = true;
                    self.clk(Self::timing(0, 1));
                    opcode = self.fetch(bus);
                }
                _ => {
                    if unlikely(self.lock_prefix && opcode != 0x0F && !Self::is_lockable(opcode)) {
                        return self.raise_fault(6, bus);
                    }
                    if unlikely(
                        self.lock_prefix
                            && Self::is_lockable(opcode)
                            && Self::lockable_modrm_is_register(self.peek_byte(bus)),
                    ) {
                        // 80486 PRM 13.1.1: LOCK with a lockable opcode
                        // is still illegal when the destination operand
                        // is a register (ModR/M mod == 11b).
                        return self.raise_fault(6, bus);
                    }
                    return self.dispatch(opcode, bus);
                }
            }
            if self.fault_pending {
                return Err(Fault);
            }
        }
    }

    fn execute_one(&mut self, bus: &mut impl common::Bus) {
        self.prefetch_valid &= self.rep_active | self.rep_completed;
        self.rep_completed = false;
        self.sx_code_fetch_bytes = 0;
        self.prev_ip = self.ip;
        self.prev_ip_upper = self.ip_upper;
        self.fault_pending = false;
        self.preserve_resume_flag = false;

        if unlikely(self.pending_irq != 0) {
            self.check_interrupts(bus);
            self.preserve_resume_flag = false;
        }
        if unlikely(self.no_interrupt > 0) {
            self.no_interrupt -= 1;
        }
        let inhibit = self.inhibit_all > 0;
        if unlikely(inhibit) {
            self.inhibit_all -= 1;
        }

        let tf_was_set = self.flags.tf;

        self.seg_prefix = false;
        self.lock_prefix = false;
        let d_bit = self.code_segment_32bit();
        self.operand_size_override = d_bit;
        self.address_size_override = d_bit;

        if self.rep_active {
            let _ = self.continue_rep(bus);
        } else {
            let _ = self.execute_with_prefixes(bus);
        }

        if CPU_MODEL == CPU_MODEL_386_SX && self.sx_code_fetch_bytes > 0 {
            let code_words = self.sx_code_fetch_bytes.div_ceil(2) as i32;
            self.clk(code_words * SX_EXTRA_CLOCKS_PER_CODE_WORD);
        }

        if likely(!self.preserve_resume_flag && !self.rep_active) {
            self.eflags_upper &= !EFLAGS_RESUME_FLAG;
        }

        if unlikely(tf_was_set && !self.fault_pending && !inhibit) {
            let _ = self.raise_trap(1, bus);
        }

        if unlikely(self.debug_trap_pending && !self.fault_pending) {
            self.debug_trap_pending = false;
            self.dr6 |= 0x8000; // BT (bit 15) - task switch debug trap
            let _ = self.raise_trap(1, bus);
        }
    }

    /// Executes exactly one logical instruction (should only be used in tests).
    pub fn step(&mut self, bus: &mut impl common::Bus) {
        self.cycles_remaining = i64::MAX;
        self.execute_one(bus);
    }

    /// Returns the number of cycles consumed by the last `step()` call.
    pub fn cycles_consumed(&self) -> u64 {
        (i64::MAX - self.cycles_remaining) as u64
    }

    /// Returns the last computed effective address (for alignment checks).
    pub fn last_ea(&self) -> u32 {
        self.ea
    }

    /// Signals a maskable interrupt request (IRQ).
    pub fn signal_irq(&mut self) {
        self.pending_irq |= crate::PENDING_IRQ;
    }

    /// Signals a non-maskable interrupt (NMI).
    pub fn signal_nmi(&mut self) {
        self.pending_irq |= crate::PENDING_NMI;
    }
}

impl<const CPU_MODEL: u8, const ADDRESS_WIDTH: u8> common::Cpu for I386<CPU_MODEL, ADDRESS_WIDTH> {
    crate::impl_cpu_run_for!();

    fn reset(&mut self) {
        self.state = I386State::default();
        // With a 24-bit address bus the PC-98 wiring expects the legacy
        // FFFF:0000 reset (first fetch at 000FFFF0). With the full 32-bit
        // bus the architectural i386/i486 reset applies: CS selector F000
        // with descriptor base FFFF0000 and EIP 0000FFF0, so the first
        // fetch lands at FFFFFFF0. The high CS base persists until the
        // first far transfer reloads CS.
        match ADDRESS_WIDTH {
            ADDRESS_WIDTH_24 => {
                self.sregs[SegReg32::CS as usize] = 0xFFFF;
            }
            ADDRESS_WIDTH_32 => {
                self.sregs[SegReg32::CS as usize] = 0xF000;
                self.ip = 0xFFF0;
            }
            _ => {
                unreachable!("Unhandled ADDRESS_WIDTH")
            }
        }
        match CPU_MODEL {
            CPU_MODEL_386_DX => {
                // 386DX reset: ET=1 (80387 coprocessor present).
                self.cr0 = 0x0000_0010;
                self.regs.set_dword(crate::DwordReg::EDX, RESET_EDX_386DX);
            }
            CPU_MODEL_486_DX => {
                // 486DX reset: ET=1 (on-chip FPU present). ET is hardwired on 486.
                self.cr0 = 0x0000_0010;
                self.regs.set_dword(crate::DwordReg::EDX, RESET_EDX_486DX2);
            }
            CPU_MODEL_386_SX => {
                // 386SX reset: ET=1 (80387SX coprocessor present).
                self.cr0 = 0x0000_0010;
                self.regs.set_dword(crate::DwordReg::EDX, RESET_EDX_386SX);
            }
            _ => {
                unreachable!("Unhandled CPU_MODEL")
            }
        }
        self.dr6 = 0xFFFF_0FF0;

        self.idt_limit = 0x03FF;

        for seg_idx in 0..6 {
            let seg = SegReg32::from_index(seg_idx);
            self.set_real_segment_cache(seg, self.sregs[seg as usize]);
        }
        match ADDRESS_WIDTH {
            ADDRESS_WIDTH_24 => {
                self.seg_bases[SegReg32::CS as usize] = 0xFFFF0;
            }
            ADDRESS_WIDTH_32 => {
                self.seg_bases[SegReg32::CS as usize] = 0xFFFF_0000;
            }
            _ => {
                unreachable!("Unhandled ADDRESS_WIDTH")
            }
        }

        self.prev_ip = 0;
        self.prev_ip_upper = 0;
        self.halted = false;
        self.pending_irq = 0;
        self.no_interrupt = 0;
        self.inhibit_all = 0;
        self.preserve_resume_flag = false;
        self.rep_active = false;
        self.rep_completed = false;
        self.rep_ip_upper = 0;
        self.rep_restart_ip = 0;
        self.rep_restart_ip_upper = 0;
        self.rep_type = 0;
        self.rep_operand_size_override = false;
        self.rep_address_size_override = false;
        self.seg_prefix = false;
        self.operand_size_override = false;
        self.address_size_override = false;
        self.ea = 0;
        self.eo = 0;
        self.eo32 = 0;
        self.ea_seg = SegReg32::DS;
        self.fetch_page_valid = false;
        self.fetch_page_tag = 0;
        self.fetch_page_phys = 0;
        self.prefetch_valid = false;
        self.prefetch_addr = 0;
        self.prefetch_byte = 0;
        self.trap_level = 0;
        self.shutdown = false;
    }

    fn halted(&self) -> bool {
        self.halted || self.shutdown
    }

    fn warm_reset(&mut self, ss: u16, sp: u16, cs: u16, ip: u16) {
        self.reset();
        self.sregs[SegReg32::SS as usize] = ss;
        self.set_real_segment_cache(SegReg32::SS, ss);
        self.regs.set_word(WordReg::SP, sp);
        self.sregs[SegReg32::CS as usize] = cs;
        self.set_real_segment_cache(SegReg32::CS, cs);
        self.ip = ip;
    }

    fn ax(&self) -> u16 {
        self.state.eax() as u16
    }

    fn set_ax(&mut self, v: u16) {
        let upper = self.state.eax() & 0xFFFF_0000;
        self.state.set_eax(upper | v as u32);
    }

    fn bx(&self) -> u16 {
        self.state.ebx() as u16
    }

    fn set_bx(&mut self, v: u16) {
        let upper = self.state.ebx() & 0xFFFF_0000;
        self.state.set_ebx(upper | v as u32);
    }

    fn cx(&self) -> u16 {
        self.state.ecx() as u16
    }

    fn set_cx(&mut self, v: u16) {
        let upper = self.state.ecx() & 0xFFFF_0000;
        self.state.set_ecx(upper | v as u32);
    }

    fn dx(&self) -> u16 {
        self.state.edx() as u16
    }

    fn set_dx(&mut self, v: u16) {
        let upper = self.state.edx() & 0xFFFF_0000;
        self.state.set_edx(upper | v as u32);
    }

    fn eax(&self) -> u32 {
        self.state.eax()
    }

    fn set_eax(&mut self, v: u32) {
        self.state.set_eax(v);
    }

    fn ebx(&self) -> u32 {
        self.state.ebx()
    }

    fn set_ebx(&mut self, v: u32) {
        self.state.set_ebx(v);
    }

    fn ecx(&self) -> u32 {
        self.state.ecx()
    }

    fn set_ecx(&mut self, v: u32) {
        self.state.set_ecx(v);
    }

    fn edx(&self) -> u32 {
        self.state.edx()
    }

    fn set_edx(&mut self, v: u32) {
        self.state.set_edx(v);
    }

    fn sp(&self) -> u16 {
        self.state.esp() as u16
    }

    fn set_sp(&mut self, v: u16) {
        let upper = self.state.esp() & 0xFFFF_0000;
        self.state.set_esp(upper | v as u32);
    }

    fn bp(&self) -> u16 {
        self.state.ebp() as u16
    }

    fn set_bp(&mut self, v: u16) {
        let upper = self.state.ebp() & 0xFFFF_0000;
        self.state.set_ebp(upper | v as u32);
    }

    fn si(&self) -> u16 {
        self.state.esi() as u16
    }

    fn set_si(&mut self, v: u16) {
        let upper = self.state.esi() & 0xFFFF_0000;
        self.state.set_esi(upper | v as u32);
    }

    fn di(&self) -> u16 {
        self.state.edi() as u16
    }

    fn set_di(&mut self, v: u16) {
        let upper = self.state.edi() & 0xFFFF_0000;
        self.state.set_edi(upper | v as u32);
    }

    fn es(&self) -> u16 {
        self.state.es()
    }

    fn set_es(&mut self, v: u16) {
        self.state.set_es(v);
    }

    fn cs(&self) -> u16 {
        self.state.cs()
    }

    fn set_cs(&mut self, v: u16) {
        self.state.set_cs(v);
    }

    fn ss(&self) -> u16 {
        self.state.ss()
    }

    fn set_ss(&mut self, v: u16) {
        self.state.set_ss(v);
    }

    fn ds(&self) -> u16 {
        self.state.ds()
    }

    fn set_ds(&mut self, v: u16) {
        self.state.set_ds(v);
    }

    fn ip(&self) -> u16 {
        self.state.ip
    }

    fn set_ip(&mut self, v: u16) {
        self.state.ip = v;
        self.state.ip_upper = 0;
    }

    fn flags(&self) -> u16 {
        self.state.flags.compress()
    }

    fn set_flags(&mut self, v: u16) {
        self.state.flags.expand(v);
    }

    fn cpu_type(&self) -> common::CpuType {
        common::CpuType::I386
    }

    fn cr0(&self) -> u32 {
        self.state.cr0
    }

    fn cr3(&self) -> u32 {
        self.state.cr3
    }

    fn protected_mode_state(&self) -> Option<common::ProtectedModeState> {
        let state = &self.state;
        let reading = |name: &'static str, value: u32| common::RegisterReading {
            name,
            value: u128::from(value),
        };
        let segment = |name: &'static str, index: SegReg32| common::SegmentReading {
            name,
            selector: u32::from(state.sregs[index as usize]),
            base: state.seg_bases[index as usize],
            limit: state.seg_limits[index as usize],
            rights: u32::from(state.seg_rights[index as usize]),
        };
        Some(common::ProtectedModeState {
            general: [
                reading("eax", state.eax()),
                reading("ebx", state.ebx()),
                reading("ecx", state.ecx()),
                reading("edx", state.edx()),
                reading("esp", state.esp()),
                reading("ebp", state.ebp()),
                reading("esi", state.esi()),
                reading("edi", state.edi()),
            ],
            segments: [
                segment("es", SegReg32::ES),
                segment("cs", SegReg32::CS),
                segment("ss", SegReg32::SS),
                segment("ds", SegReg32::DS),
                segment("fs", SegReg32::FS),
                segment("gs", SegReg32::GS),
            ],
            control: [
                reading("cr0", state.cr0),
                reading("cr2", state.cr2),
                reading("cr3", state.cr3),
            ],
            debug: [
                reading("dr0", state.dr0),
                reading("dr1", state.dr1),
                reading("dr2", state.dr2),
                reading("dr3", state.dr3),
                reading("dr6", state.dr6),
                reading("dr7", state.dr7),
            ],
            descriptor_tables: [
                common::DescriptorTableReading {
                    name: "gdtr",
                    selector: None,
                    base: state.gdt_base,
                    limit: u32::from(state.gdt_limit),
                },
                common::DescriptorTableReading {
                    name: "idtr",
                    selector: None,
                    base: state.idt_base,
                    limit: u32::from(state.idt_limit),
                },
                common::DescriptorTableReading {
                    name: "ldtr",
                    selector: Some(u32::from(state.ldtr)),
                    base: state.ldtr_base,
                    limit: state.ldtr_limit,
                },
                common::DescriptorTableReading {
                    name: "tr",
                    selector: Some(u32::from(state.tr)),
                    base: state.tr_base,
                    limit: state.tr_limit,
                },
            ],
            eip: u64::from(state.eip()),
            eflags: u64::from(state.eflags()),
        })
    }

    fn load_segment_real_mode(&mut self, seg: common::SegmentRegister, selector: u16) {
        let seg32 = match seg {
            common::SegmentRegister::ES => SegReg32::ES,
            common::SegmentRegister::CS => SegReg32::CS,
            common::SegmentRegister::SS => SegReg32::SS,
            common::SegmentRegister::DS => SegReg32::DS,
        };
        self.state.sregs[seg32 as usize] = selector;
        self.set_real_segment_cache(seg32, selector);
    }

    fn segment_base(&self, seg: common::SegmentRegister) -> u32 {
        let seg32 = match seg {
            common::SegmentRegister::ES => SegReg32::ES,
            common::SegmentRegister::CS => SegReg32::CS,
            common::SegmentRegister::SS => SegReg32::SS,
            common::SegmentRegister::DS => SegReg32::DS,
        };
        self.state.seg_bases[seg32 as usize]
    }
}

impl<const CPU_MODEL: u8, const ADDRESS_WIDTH: u8> save_state::AfterRestore
    for I386<CPU_MODEL, ADDRESS_WIDTH>
{
    fn after_restore(&mut self) {
        self.cycles_remaining = 0;
        self.run_start_cycle = 0;
        self.run_budget = 0;
    }
}

impl<const CPU_MODEL: u8, const ADDRESS_WIDTH: u8> save_state::RestoreTarget
    for I386<CPU_MODEL, ADDRESS_WIDTH>
{
    type State = I386State;
    type ValidationContext = (u8, u8);

    fn replace_state(&mut self, state: Self::State) {
        self.state = state;
    }
}

#[cfg(test)]
mod timing_tests {
    use common::Bus;

    use super::*;

    #[derive(Default)]
    struct FetchBus {
        data_reads: usize,
        byte_fetches: usize,
        word_fetches: usize,
        dword_fetches: usize,
        current_cycle: u64,
    }

    impl Bus for FetchBus {
        fn read_byte(&mut self, _address: u32) -> u8 {
            self.data_reads += 1;
            0
        }

        fn read_word(&mut self, _address: u32) -> u16 {
            self.data_reads += 1;
            0
        }

        fn read_dword(&mut self, _address: u32) -> u32 {
            self.data_reads += 1;
            0
        }

        fn write_byte(&mut self, _address: u32, _value: u8) {}

        fn io_read_byte(&mut self, _port: u16) -> u8 {
            0
        }

        fn io_write_byte(&mut self, _port: u16, _value: u8) {}

        fn fetch_opcode_byte(&mut self, address: u32) -> u8 {
            self.byte_fetches += 1;
            address as u8
        }

        fn fetch_opcode_word(&mut self, _address: u32) -> u16 {
            self.word_fetches += 1;
            0xABCD
        }

        fn fetch_opcode_dword(&mut self, _address: u32) -> u32 {
            self.dword_fetches += 1;
            0x1234_5678
        }

        fn has_irq(&self) -> bool {
            false
        }

        fn acknowledge_irq(&mut self) -> u8 {
            0
        }

        fn has_nmi(&self) -> bool {
            false
        }

        fn acknowledge_nmi(&mut self) {}

        fn current_cycle(&self) -> u64 {
            self.current_cycle
        }

        fn set_current_cycle(&mut self, cycle: u64) {
            self.current_cycle = cycle;
        }
    }

    fn fetch_test_cpu() -> I386<{ CPU_MODEL_386_DX }, { ADDRESS_WIDTH_32 }> {
        let mut cpu = I386::new();
        cpu.state.ip = 0;
        cpu.state.ip_upper = 0;
        cpu.state.seg_bases[SegReg32::CS as usize] = 0;
        cpu.state.seg_limits[SegReg32::CS as usize] = u32::MAX;
        cpu.fetch_page_valid = false;
        cpu.prefetch_valid = false;
        cpu
    }

    /// The 386SX-only execute-cycle override must never change the 386DX or
    /// 486DX result: for non-SX models `timing_sx(a, b, tsx)` equals
    /// `timing(a, b)` regardless of `tsx`.
    #[test]
    fn timing_sx_leaves_dx_and_486_unchanged() {
        type Dx = I386<{ CPU_MODEL_386_DX }, { ADDRESS_WIDTH_24 }>;
        type M486 = I386<{ CPU_MODEL_486_DX }, { ADDRESS_WIDTH_24 }>;
        for (t386, t486, tsx) in [(2, 1, 99), (7, 3, 0), (14, 13, 41), (5, 2, -7)] {
            assert_eq!(Dx::timing_sx(t386, t486, tsx), Dx::timing(t386, t486));
            assert_eq!(M486::timing_sx(t386, t486, tsx), M486::timing(t386, t486));
        }
    }

    #[test]
    fn wide_instruction_operands_use_fetch_bus_methods() {
        let mut cpu = fetch_test_cpu();
        let mut bus = FetchBus::default();

        assert_eq!(cpu.fetchword(&mut bus), 0xABCD);
        assert_eq!(bus.word_fetches, 1);
        assert_eq!(bus.data_reads, 0);

        cpu.state.ip = 0;
        cpu.fetch_page_valid = false;
        assert_eq!(cpu.fetchdword(&mut bus), 0x1234_5678);
        assert_eq!(bus.dword_fetches, 1);
        assert_eq!(bus.data_reads, 0);
    }

    #[test]
    fn page_crossing_instruction_operand_uses_byte_fetches() {
        let mut cpu = fetch_test_cpu();
        cpu.state.ip = 0x0FFF;
        let mut bus = FetchBus::default();

        cpu.fetchword(&mut bus);

        assert!(bus.byte_fetches >= 2);
        assert_eq!(bus.word_fetches, 0);
        assert_eq!(bus.data_reads, 0);
    }
}
