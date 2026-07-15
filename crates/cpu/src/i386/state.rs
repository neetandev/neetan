use core::ops::{Deref, DerefMut};

use save_state::{StateValidationError, ValidateState};

use super::{
    ADDRESS_WIDTH_24, ADDRESS_WIDTH_32, CPU_MODEL_386_DX, CPU_MODEL_386_SX, CPU_MODEL_486_DX, I386,
    flags::I386Flags, fpu::X87State,
};
use crate::{ByteReg, DwordReg, RegisterFile32, SegReg32};

save_state::runtime_state! {
    /// Complete authoritative 80386 and 80486 state at a resumable boundary.
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
        /// Cached granularity byte per segment.
        pub seg_granularity: [u8; 6],
        /// Whether each segment holds a valid loaded descriptor.
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
        /// Stored current privilege level.
        pub stored_cpl: u16,
        /// x87 FPU state.
        pub fpu: X87State,
        /// Internal execution, translation, and prefetch state.
        #[doc(hidden)]
        pub internal: I386InternalState,
    }
}

save_state::runtime_state! {
    /// Internal 80386 and 80486 execution state.
    #[doc(hidden)]
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct I386InternalState {
        pub(super) prev_ip: u16,
        pub(super) prev_ip_upper: u32,
        pub(super) seg_prefix: bool,
        pub(super) prefix_seg: SegReg32,
        pub(super) operand_size_override: bool,
        pub(super) address_size_override: bool,
        pub(super) lock_prefix: bool,
        pub(super) halted: bool,
        pub(super) fault_pending: bool,
        pub(super) supervisor_override: bool,
        pub(super) pending_irq: u8,
        pub(super) no_interrupt: u8,
        pub(super) inhibit_all: u8,
        pub(super) preserve_resume_flag: bool,
        pub(super) rep_ip: u16,
        pub(super) rep_ip_upper: u32,
        pub(super) rep_restart_ip: u16,
        pub(super) rep_restart_ip_upper: u32,
        pub(super) rep_seg_prefix: bool,
        pub(super) rep_prefix_seg: SegReg32,
        pub(super) rep_opcode: u8,
        pub(super) rep_type: u8,
        pub(super) rep_operand_size_override: bool,
        pub(super) rep_address_size_override: bool,
        pub(super) rep_active: bool,
        pub(super) rep_completed: bool,
        pub(super) ea: u32,
        pub(super) eo: u16,
        pub(super) eo32: u32,
        pub(super) ea_seg: SegReg32,
        pub(super) fetch_page_valid: bool,
        pub(super) fetch_page_tag: u32,
        pub(super) fetch_page_phys: u32,
        pub(super) fetch_page_user: bool,
        pub(super) prefetch_valid: bool,
        pub(super) prefetch_addr: u32,
        pub(super) prefetch_byte: u8,
        pub(super) tlb_valid: [bool; 64],
        pub(super) tlb_tag: [u32; 64],
        pub(super) tlb_phys: [u32; 64],
        pub(super) tlb_writable: [bool; 64],
        pub(super) tlb_user: [bool; 64],
        pub(super) tlb_dirty: [bool; 64],
        pub(super) debug_trap_pending: bool,
        pub(super) trap_level: u8,
        pub(super) prev_exception_class: u8,
        pub(super) shutdown: bool,
        pub(super) sx_code_fetch_bytes: u32,
    }
}

impl Default for I386InternalState {
    fn default() -> Self {
        Self {
            prev_ip: 0,
            prev_ip_upper: 0,
            seg_prefix: false,
            prefix_seg: SegReg32::DS,
            operand_size_override: false,
            address_size_override: false,
            lock_prefix: false,
            halted: false,
            fault_pending: false,
            supervisor_override: false,
            pending_irq: 0,
            no_interrupt: 0,
            inhibit_all: 0,
            preserve_resume_flag: false,
            rep_ip: 0,
            rep_ip_upper: 0,
            rep_restart_ip: 0,
            rep_restart_ip_upper: 0,
            rep_seg_prefix: false,
            rep_prefix_seg: SegReg32::DS,
            rep_opcode: 0,
            rep_type: 0,
            rep_operand_size_override: false,
            rep_address_size_override: false,
            rep_active: false,
            rep_completed: false,
            ea: 0,
            eo: 0,
            eo32: 0,
            ea_seg: SegReg32::DS,
            fetch_page_valid: false,
            fetch_page_tag: 0,
            fetch_page_phys: 0,
            fetch_page_user: false,
            prefetch_valid: false,
            prefetch_addr: 0,
            prefetch_byte: 0,
            tlb_valid: [false; 64],
            tlb_tag: [0; 64],
            tlb_phys: [0; 64],
            tlb_writable: [false; 64],
            tlb_user: [false; 64],
            tlb_dirty: [false; 64],
            debug_trap_pending: false,
            trap_level: 0,
            prev_exception_class: 0,
            shutdown: false,
            sx_code_fetch_bytes: 0,
        }
    }
}

impl Deref for I386State {
    type Target = I386InternalState;

    fn deref(&self) -> &Self::Target {
        &self.internal
    }
}

impl DerefMut for I386State {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.internal
    }
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
            internal: I386InternalState::default(),
        }
    }
}

impl I386State {
    fn set_segment_for_constructed_state(&mut self, segment: SegReg32, selector: u16) {
        let index = segment as usize;
        self.sregs[index] = selector;
        if self.cr0 & 1 == 0 && !self.seg_valid[index] {
            self.seg_bases[index] = u32::from(selector) << 4;
            self.seg_limits[index] = 0xFFFF;
            self.seg_rights[index] = if segment == SegReg32::CS { 0x9B } else { 0x93 };
            self.seg_granularity[index] = 0;
            self.seg_valid[index] = true;
        }
    }

    /// Initializes real-mode descriptor caches and cold execution internals.
    pub fn initialize_real_mode_caches(&mut self) {
        for segment_index in 0..6 {
            let segment = SegReg32::from_index(segment_index);
            let selector = self.sregs[segment as usize];
            self.seg_bases[segment as usize] = u32::from(selector) << 4;
            self.seg_limits[segment as usize] = 0xFFFF;
            self.seg_rights[segment as usize] = if segment == SegReg32::CS { 0x9B } else { 0x93 };
            self.seg_granularity[segment as usize] = 0;
            self.seg_valid[segment as usize] = true;
        }
        self.stored_cpl = 0;
        self.internal = I386InternalState::default();
    }

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
        self.set_segment_for_constructed_state(SegReg32::CS, v);
        self.stored_cpl = if self.cr0 & 1 != 0 { v & 3 } else { 0 };
    }

    /// Returns the DS segment register.
    pub fn ds(&self) -> u16 {
        self.sregs[SegReg32::DS as usize]
    }

    /// Sets the DS segment register.
    pub fn set_ds(&mut self, v: u16) {
        self.set_segment_for_constructed_state(SegReg32::DS, v);
    }

    /// Returns the ES segment register.
    pub fn es(&self) -> u16 {
        self.sregs[SegReg32::ES as usize]
    }

    /// Sets the ES segment register.
    pub fn set_es(&mut self, v: u16) {
        self.set_segment_for_constructed_state(SegReg32::ES, v);
    }

    /// Returns the FS segment register.
    pub fn fs(&self) -> u16 {
        self.sregs[SegReg32::FS as usize]
    }

    /// Sets the FS segment register.
    pub fn set_fs(&mut self, v: u16) {
        self.set_segment_for_constructed_state(SegReg32::FS, v);
    }

    /// Returns the GS segment register.
    pub fn gs(&self) -> u16 {
        self.sregs[SegReg32::GS as usize]
    }

    /// Sets the GS segment register.
    pub fn set_gs(&mut self, v: u16) {
        self.set_segment_for_constructed_state(SegReg32::GS, v);
    }

    /// Returns the SS segment register.
    pub fn ss(&self) -> u16 {
        self.sregs[SegReg32::SS as usize]
    }

    /// Sets the SS segment register.
    pub fn set_ss(&mut self, v: u16) {
        self.set_segment_for_constructed_state(SegReg32::SS, v);
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

impl ValidateState<(u8, u8)> for I386State {
    fn validate_state(&self, context: &(u8, u8)) -> Result<(), StateValidationError> {
        let (cpu_model, address_width) = *context;
        if !matches!(
            cpu_model,
            CPU_MODEL_386_DX | CPU_MODEL_386_SX | CPU_MODEL_486_DX
        ) || !matches!(address_width, ADDRESS_WIDTH_24 | ADDRESS_WIDTH_32)
        {
            return Err(StateValidationError::new(
                "386 CPU configuration is invalid",
            ));
        }
        if self.flags.iopl > 3 || self.stored_cpl > 3 {
            return Err(StateValidationError::new("386 privilege state is invalid"));
        }
        if self.pending_irq & !0x03 != 0 || self.no_interrupt > 1 || self.inhibit_all > 1 {
            return Err(StateValidationError::new("386 interrupt latch is invalid"));
        }
        if self.rep_active && self.rep_type > 1 {
            return Err(StateValidationError::new("386 REP continuation is invalid"));
        }
        if self.trap_level > 3 || self.prev_exception_class > 3 {
            return Err(StateValidationError::new(
                "386 exception nesting state is invalid",
            ));
        }
        Ok(())
    }
}

impl<const CPU_MODEL: u8, const ADDRESS_WIDTH: u8> I386<CPU_MODEL, ADDRESS_WIDTH> {
    /// Loads complete CPU state without resetting execution or translation state.
    pub fn load_state(&mut self, state: &I386State) {
        self.state = state.clone();
    }

    /// Clones the authoritative state at a resumable execution boundary.
    pub fn capture_state(&self) -> I386State {
        self.state.clone()
    }

    /// Validates and replaces the authoritative state transactionally.
    pub fn restore_state(
        &mut self,
        state: I386State,
    ) -> Result<(), save_state::StateValidationError> {
        save_state::restore_root(self, state, &(CPU_MODEL, ADDRESS_WIDTH))
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
