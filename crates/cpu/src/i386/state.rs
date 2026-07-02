use super::{I386, flags::I386Flags, fpu::X87State};
use crate::{ByteReg, DwordReg, RegisterFile32, SegReg32};

/// Snapshot of all I386 CPU registers and flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct I386State {
    /// General-purpose register file (32-bit).
    pub regs: RegisterFile32,
    /// Segment registers: ES, CS, SS, DS, FS, GS.
    pub sregs: [u16; 6],
    /// Instruction pointer (low 16 bits).
    pub ip: u16,
    /// Instruction pointer (upper 16 bits).
    pub ip_upper: u32,
    /// CPU flags (lower 16 bits via lazy evaluation).
    pub flags: I386Flags,
    /// Upper EFLAGS bits (bits 16-31).
    pub eflags_upper: u32,
    /// Control register 0.
    pub cr0: u32,
    /// Control register 2 (page fault linear address).
    pub cr2: u32,
    /// Control register 3.
    pub cr3: u32,
    /// Debug register 0.
    pub dr0: u32,
    /// Debug register 1.
    pub dr1: u32,
    /// Debug register 2.
    pub dr2: u32,
    /// Debug register 3.
    pub dr3: u32,
    /// Debug register 6.
    pub dr6: u32,
    /// Debug register 7.
    pub dr7: u32,
    /// Global Descriptor Table Register base.
    pub gdt_base: u32,
    /// Global Descriptor Table Register limit.
    pub gdt_limit: u16,
    /// Interrupt Descriptor Table Register base.
    pub idt_base: u32,
    /// Interrupt Descriptor Table Register limit.
    pub idt_limit: u16,
    /// Cached physical base per segment (ES/CS/SS/DS/FS/GS).
    pub seg_bases: [u32; 6],
    /// Cached effective limit per segment (after G-bit scaling).
    pub seg_limits: [u32; 6],
    /// Cached access-rights byte per segment.
    pub seg_rights: [u8; 6],
    /// Cached granularity byte (byte 6 of descriptor) per segment.
    pub seg_granularity: [u8; 6],
    /// Whether the segment register currently holds a valid loaded descriptor.
    pub seg_valid: [bool; 6],
    /// LDT selector.
    pub ldtr: u16,
    /// LDT cached base.
    pub ldtr_base: u32,
    /// LDT cached limit.
    pub ldtr_limit: u32,
    /// Task Register selector.
    pub tr: u16,
    /// TR cached base.
    pub tr_base: u32,
    /// TR cached limit.
    pub tr_limit: u32,
    /// TR cached access rights.
    pub tr_rights: u8,
    /// Stored current privilege level (updated on CS loads).
    pub stored_cpl: u16,
    /// x87 FPU state.
    pub fpu: X87State,
}

impl Default for I386State {
    /// Returns a state matching the i386/i486 post-reset defaults that
    /// matter for typical test scaffolding. Most fields are zero, but
    /// `idt_limit` is initialised to 0x3FF (256 entries x 4 bytes - 1)
    /// per 80486 PRM 22.5 so real-mode INT n through the default IVT
    /// dispatches normally.
    fn default() -> Self {
        Self {
            regs: RegisterFile32::default(),
            sregs: [0; 6],
            ip: 0,
            ip_upper: 0,
            flags: I386Flags::default(),
            eflags_upper: 0,
            cr0: 0,
            cr2: 0,
            cr3: 0,
            dr0: 0,
            dr1: 0,
            dr2: 0,
            dr3: 0,
            dr6: 0,
            dr7: 0,
            gdt_base: 0,
            gdt_limit: 0,
            idt_base: 0,
            idt_limit: 0x03FF,
            seg_bases: [0; 6],
            seg_limits: [0; 6],
            seg_rights: [0; 6],
            seg_granularity: [0; 6],
            seg_valid: [false; 6],
            ldtr: 0,
            ldtr_base: 0,
            ldtr_limit: 0,
            tr: 0,
            tr_base: 0,
            tr_limit: 0,
            tr_rights: 0,
            stored_cpl: 0,
            fpu: X87State::default(),
        }
    }
}

impl I386State {
    /// Returns the EAX register.
    pub fn eax(&self) -> u32 {
        self.regs.dword(DwordReg::EAX)
    }

    /// Sets the EAX register.
    pub fn set_eax(&mut self, v: u32) {
        self.regs.set_dword(DwordReg::EAX, v);
    }

    /// Returns the ECX register.
    pub fn ecx(&self) -> u32 {
        self.regs.dword(DwordReg::ECX)
    }

    /// Sets the ECX register.
    pub fn set_ecx(&mut self, v: u32) {
        self.regs.set_dword(DwordReg::ECX, v);
    }

    /// Returns the EDX register.
    pub fn edx(&self) -> u32 {
        self.regs.dword(DwordReg::EDX)
    }

    /// Sets the EDX register.
    pub fn set_edx(&mut self, v: u32) {
        self.regs.set_dword(DwordReg::EDX, v);
    }

    /// Returns the EBX register.
    pub fn ebx(&self) -> u32 {
        self.regs.dword(DwordReg::EBX)
    }

    /// Sets the EBX register.
    pub fn set_ebx(&mut self, v: u32) {
        self.regs.set_dword(DwordReg::EBX, v);
    }

    /// Returns the ESP register.
    pub fn esp(&self) -> u32 {
        self.regs.dword(DwordReg::ESP)
    }

    /// Sets the ESP register.
    pub fn set_esp(&mut self, v: u32) {
        self.regs.set_dword(DwordReg::ESP, v);
    }

    /// Returns the EBP register.
    pub fn ebp(&self) -> u32 {
        self.regs.dword(DwordReg::EBP)
    }

    /// Sets the EBP register.
    pub fn set_ebp(&mut self, v: u32) {
        self.regs.set_dword(DwordReg::EBP, v);
    }

    /// Returns the ESI register.
    pub fn esi(&self) -> u32 {
        self.regs.dword(DwordReg::ESI)
    }

    /// Sets the ESI register.
    pub fn set_esi(&mut self, v: u32) {
        self.regs.set_dword(DwordReg::ESI, v);
    }

    /// Returns the EDI register.
    pub fn edi(&self) -> u32 {
        self.regs.dword(DwordReg::EDI)
    }

    /// Sets the EDI register.
    pub fn set_edi(&mut self, v: u32) {
        self.regs.set_dword(DwordReg::EDI, v);
    }

    /// Returns the CS segment register.
    pub fn cs(&self) -> u16 {
        self.sregs[SegReg32::CS as usize]
    }

    /// Sets the CS segment register.
    pub fn set_cs(&mut self, v: u16) {
        self.sregs[SegReg32::CS as usize] = v;
    }

    /// Returns the DS segment register.
    pub fn ds(&self) -> u16 {
        self.sregs[SegReg32::DS as usize]
    }

    /// Sets the DS segment register.
    pub fn set_ds(&mut self, v: u16) {
        self.sregs[SegReg32::DS as usize] = v;
    }

    /// Returns the ES segment register.
    pub fn es(&self) -> u16 {
        self.sregs[SegReg32::ES as usize]
    }

    /// Sets the ES segment register.
    pub fn set_es(&mut self, v: u16) {
        self.sregs[SegReg32::ES as usize] = v;
    }

    /// Returns the FS segment register.
    pub fn fs(&self) -> u16 {
        self.sregs[SegReg32::FS as usize]
    }

    /// Sets the FS segment register.
    pub fn set_fs(&mut self, v: u16) {
        self.sregs[SegReg32::FS as usize] = v;
    }

    /// Returns the GS segment register.
    pub fn gs(&self) -> u16 {
        self.sregs[SegReg32::GS as usize]
    }

    /// Sets the GS segment register.
    pub fn set_gs(&mut self, v: u16) {
        self.sregs[SegReg32::GS as usize] = v;
    }

    /// Returns the SS segment register.
    pub fn ss(&self) -> u16 {
        self.sregs[SegReg32::SS as usize]
    }

    /// Sets the SS segment register.
    pub fn set_ss(&mut self, v: u16) {
        self.sregs[SegReg32::SS as usize] = v;
    }

    /// Returns the full 32-bit EIP.
    pub fn eip(&self) -> u32 {
        self.ip_upper | self.ip as u32
    }

    /// Sets the full 32-bit EIP.
    pub fn set_eip(&mut self, v: u32) {
        self.ip = v as u16;
        self.ip_upper = v & 0xFFFF_0000;
    }

    /// Returns the full 32-bit EFLAGS.
    pub fn eflags(&self) -> u32 {
        self.eflags_upper | self.flags.compress() as u32
    }

    /// Sets the full 32-bit EFLAGS.
    pub fn set_eflags(&mut self, v: u32) {
        self.eflags_upper = v & 0xFFFF_0000;
        self.flags.expand(v as u16);
    }
}

impl<const CPU_MODEL: u8, const ADDRESS_WIDTH: u8> I386<CPU_MODEL, ADDRESS_WIDTH> {
    /// Loads CPU state from a snapshot, resetting runtime flags.
    pub fn load_state(&mut self, state: &I386State) {
        self.state = state.clone();
        if self.state.seg_valid.iter().all(|&valid| !valid) && self.state.cr0 & 1 == 0 {
            for seg_idx in 0..6 {
                let seg = SegReg32::from_index(seg_idx);
                let selector = self.state.sregs[seg as usize];
                self.state.seg_bases[seg as usize] = (selector as u32) << 4;
                self.state.seg_limits[seg as usize] = 0xFFFF;
                self.state.seg_rights[seg as usize] = if seg == SegReg32::CS { 0x9B } else { 0x93 };
                self.state.seg_granularity[seg as usize] = 0;
                self.state.seg_valid[seg as usize] = true;
            }
        }
        if self.state.cr0 & 1 != 0 {
            self.state.stored_cpl = self.state.sregs[SegReg32::CS as usize] & 3;
        } else {
            self.state.stored_cpl = 0;
        }
        self.halted = false;
        self.shutdown = false;
        self.trap_level = 0;
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
        self.fetch_page_valid = false;
        self.prefetch_valid = false;
        self.flush_tlb();
    }

    /// Returns the AL register value.
    pub fn al(&self) -> u8 {
        self.regs.byte(ByteReg::AL)
    }

    /// Returns the AH register value.
    pub fn ah(&self) -> u8 {
        self.regs.byte(ByteReg::AH)
    }

    /// Returns the CL register value.
    pub fn cl(&self) -> u8 {
        self.regs.byte(ByteReg::CL)
    }

    /// Returns the instruction pointer.
    pub fn ip(&self) -> u32 {
        self.ip_upper | self.ip as u32
    }

    /// Returns the compressed flags register value.
    pub fn flags_register(&self) -> u32 {
        self.eflags_upper | self.flags.compress() as u32
    }
}
