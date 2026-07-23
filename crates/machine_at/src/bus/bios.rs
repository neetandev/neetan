//! BIOS HLE handler implementations.
//!
//! Each handler reads/writes CPU registers directly via the `Cpu` trait.
//! The ROM stubs save AX/DX on the stack (clobbered by the trap OUT),
//! write the vector number to the trap port, and IRET. The Rust side
//! restores AX/DX from the stack before dispatching to the handler.

mod bootstrap;
mod floppy;
mod floppy_interrupt;
mod hdd;
mod keyboard;
mod printer;
mod serial;
mod system;
mod timer;
pub(super) mod video;
mod video_graphics;
mod video_modes;
mod video_palette;
pub(crate) mod video_param;
mod video_text;

use common::{
    Cpu, SegmentRegister, StackVec, TraceCall, TraceCallInterface, TraceCallPhase, TraceContext,
    TraceEvent, TraceEventKey, TraceField, TraceSink, TraceValue, inspect::capture_x86_snapshot,
    trace_id,
};

use super::{AtBus, OPEN_BUS_BYTE};
use crate::memory::{AtMemory, UMA_BASE, VGA_WINDOW_BASE};

/// Real-mode segment of the system BIOS stub ROM.
pub(super) const BIOS_CODE_SEGMENT: u16 = 0xF000;
/// Stub ROM metadata word: vector table offset.
pub(super) const METADATA_VECTOR_TABLE: usize = 0;
/// Stub ROM metadata word: cold/POST entry point offset.
pub(super) const METADATA_COLD_ENTRY: usize = 2;
/// Stub ROM metadata word: boot-failure halt loop offset.
pub(super) const METADATA_HALT_LOOP: usize = 6;
/// Stub ROM metadata word: INT 15h AH=C0h configuration table offset.
pub(super) const METADATA_CONFIG_TABLE: usize = 8;
/// Stub ROM metadata word: Ctrl-Break int 1Bh helper offset.
pub(super) const METADATA_CONTROL_BREAK_HELPER: usize = 10;
/// Stub ROM metadata word: pause hold loop offset.
pub(super) const METADATA_PAUSE_WAIT_LOOP: usize = 12;
/// Stub ROM metadata word: INT 1Eh diskette parameter table offset.
pub(super) const METADATA_DISKETTE_PARAMETER_TABLE: usize = 14;
/// Stub ROM metadata word: INT 41h fixed disk parameter table offset.
pub(super) const METADATA_FDPT_DRIVE_0: usize = 16;
/// Stub ROM metadata word: INT 46h fixed disk parameter table offset.
pub(super) const METADATA_FDPT_DRIVE_1: usize = 18;
/// Stub ROM metadata word: INT 70h alarm int 4Ah helper offset.
pub(super) const METADATA_RTC_ALARM_HELPER: usize = 20;

/// Page table entry present bit.
const PAGE_PRESENT: u32 = 0x01;
/// Page table entry accessed bit.
const PAGE_ACCESSED: u32 = 0x20;
/// Page table entry dirty bit.
const PAGE_DIRTY: u32 = 0x40;

/// Returns the linear address of the IRET frame at the current SS:SP.
fn iret_stack_base(cpu: &impl Cpu) -> u32 {
    cpu.segment_base(SegmentRegister::SS)
        .wrapping_add(u32::from(cpu.sp()))
}

/// Reads a little-endian doubleword from the CPU's view of memory.
fn hle_read_dword(memory: &AtMemory, address: u32) -> u32 {
    let read = |offset: u32| u32::from(memory.read_physical(memory.apply_a20(address + offset)));
    read(0) | (read(1) << 8) | (read(2) << 16) | (read(3) << 24)
}

/// Writes a little-endian doubleword to the CPU's view of memory.
fn hle_write_dword(memory: &mut AtMemory, address: u32, value: u32) {
    for offset in 0..4u32 {
        let physical = memory.apply_a20(address + offset);
        memory.write_physical(physical, (value >> (offset * 8)) as u8);
    }
}

/// Translates a linear address through the guest page tables, updating the
/// accessed (and, for writes, dirty) bits like the CPU would. Returns the
/// linear address unchanged when paging is disabled or the mapping is not
/// present.
fn hle_page_translate_access(
    cr0: u32,
    cr3: u32,
    linear: u32,
    write: bool,
    memory: &mut AtMemory,
) -> u32 {
    if cr0 & 0x8000_0001 != 0x8000_0001 {
        return linear;
    }
    let dir_idx = (linear >> 22) & 0x3FF;
    let tbl_idx = (linear >> 12) & 0x3FF;
    let offset = linear & 0xFFF;
    let pde_addr = (cr3 & 0xFFFFF000) + dir_idx * 4;
    let pde = hle_read_dword(memory, pde_addr);
    if pde & PAGE_PRESENT == 0 {
        return linear;
    }
    let pte_addr = (pde & 0xFFFFF000) + tbl_idx * 4;
    let pte = hle_read_dword(memory, pte_addr);
    if pte & PAGE_PRESENT == 0 {
        return linear;
    }

    let accessed_pde = pde | PAGE_ACCESSED;
    if accessed_pde != pde {
        hle_write_dword(memory, pde_addr, accessed_pde);
    }

    let mut accessed_pte = pte | PAGE_ACCESSED;
    if write {
        accessed_pte |= PAGE_DIRTY;
    }
    if accessed_pte != pte {
        hle_write_dword(memory, pte_addr, accessed_pte);
    }

    (pte & 0xFFFFF000) | offset
}

/// Translates a linear address for an HLE read access.
fn hle_page_translate_read(cr0: u32, cr3: u32, linear: u32, memory: &mut AtMemory) -> u32 {
    hle_page_translate_access(cr0, cr3, linear, false, memory)
}

/// Translates a linear address for an HLE write access.
fn hle_page_translate_write(cr0: u32, cr3: u32, linear: u32, memory: &mut AtMemory) -> u32 {
    hle_page_translate_access(cr0, cr3, linear, true, memory)
}

impl<T: TraceSink> AtBus<T> {
    /// Configures paging state used by HLE BIOS routines. When paging is
    /// active (CR0.PG + CR0.PE), HLE memory accesses translate linear
    /// addresses through the page tables rooted at CR3.
    pub fn set_hle_paging(&mut self, cr0: u32, cr3: u32) {
        self.hle_cr0 = cr0;
        self.hle_cr3 = cr3;
    }

    /// Returns whether a BIOS HLE trap is pending.
    pub fn bios_hle_pending(&self) -> bool {
        self.bios.hle_pending()
    }

    /// Executes the pending BIOS HLE operation with direct CPU register access.
    pub fn execute_bios_hle(&mut self, cpu: &mut impl Cpu) {
        let vector = self.bios.pending_vector();
        self.bios.clear_hle_pending();

        // The assembly stub pushes AX and DX before clobbering them with the
        // trap port address and vector number. Restore the caller's original
        // values and adjust SP so the IRET frame sits at SS:SP+0.
        let sp = cpu.sp();
        let ss_base = cpu.segment_base(SegmentRegister::SS);
        let saved_dx = self.read_mem_word(ss_base.wrapping_add(u32::from(sp)));
        let saved_ax = self.read_mem_word(ss_base.wrapping_add(u32::from(sp.wrapping_add(2))));
        cpu.set_dx(saved_dx);
        cpu.set_ax(saved_ax);
        cpu.set_sp(sp.wrapping_add(4));

        let function = cpu.ah();
        let subfunction = cpu.al();

        // Capture the guest registers at dispatch entry so the boundary call
        // events carry an atomic snapshot. The handler runs to completion as
        // one CPU step, so a post-return snapshot would show clobbered
        // registers.
        if T::ENABLED
            && let Some(processor) = self.tracer.snapshot_request()
        {
            let snapshot = capture_x86_snapshot(processor, &*cpu);
            self.tracer.set_pending_snapshot(snapshot);
        }
        self.trace_call(
            trace_id::provider::AT_BIOS,
            TraceCallInterface::Interrupt(u32::from(vector)),
            Some(u64::from(function)),
            Some(u64::from(subfunction)),
            TraceCallPhase::Enter,
            None,
        );

        match vector {
            0x05 => self.hle_int05h(),
            0x08 => self.hle_int08h(),
            0x09 => self.hle_int09h(cpu),
            0x0E => self.hle_int0eh(),
            0x10 => self.hle_int10h(cpu),
            0x11 => self.hle_int11h(cpu),
            0x12 => self.hle_int12h(cpu),
            // Without a hard disk the real BIOS never revectors INT 13h, so
            // DL >= 80h calls land in the diskette handler like on INT 40h.
            0x13 => {
                let hard_disk_installed = self.ide.has_drive(0) || self.ide.has_drive(1);
                if cpu.dl() >= floppy::FIRST_HARD_DISK_DRIVE && hard_disk_installed {
                    self.hle_int13h_hdd(cpu);
                } else {
                    self.hle_int13h_floppy(cpu);
                }
            }
            0x40 => self.hle_int13h_floppy(cpu),
            0x14 => self.hle_int14h(cpu),
            0x15 => self.hle_int15h(cpu),
            0x16 => self.hle_int16h(cpu),
            0x17 => self.hle_int17h(),
            0x19 => self.hle_bootstrap(cpu),
            0x1A => self.hle_int1ah(cpu),
            0x70 => self.hle_int70h(cpu),
            0x76 => self.hle_int76h(),
            0xF0 => {
                if std::mem::take(&mut self.needs_full_reinit) {
                    self.initialize_post_boot_state();
                }
                self.hle_bootstrap(cpu);
            }
            0xF2 => self.hle_bootstrap(cpu),
            _ => {}
        }

        // The exit call event still carries the entry snapshot, so the call
        // arguments stay visible on either boundary.
        self.trace_call(
            trace_id::provider::AT_BIOS,
            TraceCallInterface::Interrupt(u32::from(vector)),
            Some(u64::from(function)),
            Some(u64::from(subfunction)),
            TraceCallPhase::Exit,
            Some(u64::from(cpu.ax())),
        );
        if T::ENABLED {
            self.tracer.clear_pending_snapshot();
        }
    }

    /// Emits a high-level call trace event for a BIOS HLE dispatch boundary.
    fn trace_call(
        &mut self,
        provider: &'static str,
        interface: TraceCallInterface,
        function: Option<u64>,
        subfunction: Option<u64>,
        phase: TraceCallPhase,
        result: Option<u64>,
    ) {
        if !T::ENABLED
            || !self
                .tracer
                .interested(TraceEventKey::Call { provider, phase })
        {
            return;
        }
        let mut fields = StackVec::<TraceField<'_>, 3>::new();
        if let Some(function) = function {
            fields.push(TraceField {
                name: trace_id::field::FUNCTION,
                value: TraceValue::Unsigned(function),
            });
        }
        if let Some(subfunction) = subfunction {
            fields.push(TraceField {
                name: trace_id::field::SUBFUNCTION,
                value: TraceValue::Unsigned(subfunction),
            });
        }
        if let Some(result) = result {
            fields.push(TraceField {
                name: trace_id::field::RESULT,
                value: TraceValue::Unsigned(result),
            });
        }
        self.tracer.trace(
            TraceContext::main_cpu(
                self.current_cycle,
                Some(u64::from(self.clocks.cpu_clock_hz)),
            ),
            TraceEvent::Call(TraceCall {
                provider,
                interface,
                phase,
                fields: &fields,
            }),
        );
    }

    /// Reads a byte at a physical address, routing the VGA display window to
    /// the adapter like a CPU access would.
    fn hle_physical_read_byte(&mut self, physical: u32) -> u8 {
        let physical = self.memory.apply_a20(physical);
        if (VGA_WINDOW_BASE..UMA_BASE).contains(&physical) && !self.memory.ab_internal(physical) {
            self.vga
                .mem_read(physical - VGA_WINDOW_BASE)
                .unwrap_or(OPEN_BUS_BYTE)
        } else {
            self.memory.read_physical(physical)
        }
    }

    /// Writes a byte at a physical address, routing the VGA display window to
    /// the adapter like a CPU access would.
    fn hle_physical_write_byte(&mut self, physical: u32, value: u8) {
        let physical = self.memory.apply_a20(physical);
        if (VGA_WINDOW_BASE..UMA_BASE).contains(&physical) && !self.memory.ab_internal(physical) {
            self.vga.mem_write(physical - VGA_WINDOW_BASE, value);
        } else {
            self.memory.write_physical(physical, value);
        }
    }

    /// Reads a byte from the guest's linear address space, honoring paging.
    fn read_mem_byte(&mut self, addr: u32) -> u8 {
        let phys = hle_page_translate_read(self.hle_cr0, self.hle_cr3, addr, &mut self.memory);
        self.hle_physical_read_byte(phys)
    }

    /// Reads a little-endian word from the guest's linear address space.
    fn read_mem_word(&mut self, addr: u32) -> u16 {
        u16::from(self.read_mem_byte(addr)) | (u16::from(self.read_mem_byte(addr + 1)) << 8)
    }

    /// Writes a byte to the guest's linear address space, honoring paging.
    fn write_mem_byte(&mut self, addr: u32, value: u8) {
        let phys = hle_page_translate_write(self.hle_cr0, self.hle_cr3, addr, &mut self.memory);
        self.hle_physical_write_byte(phys, value);
    }

    /// Writes a little-endian word to the guest's linear address space.
    fn write_mem_word(&mut self, addr: u32, value: u16) {
        self.write_mem_byte(addr, value as u8);
        self.write_mem_byte(addr + 1, (value >> 8) as u8);
    }

    /// Reads a little-endian doubleword from the guest's linear address space.
    fn read_mem_dword(&mut self, addr: u32) -> u32 {
        u32::from(self.read_mem_word(addr)) | (u32::from(self.read_mem_word(addr + 2)) << 16)
    }

    /// Writes a little-endian doubleword to the guest's linear address space.
    fn write_mem_dword(&mut self, addr: u32, value: u32) {
        self.write_mem_word(addr, value as u16);
        self.write_mem_word(addr + 2, (value >> 16) as u16);
    }

    /// Sets or clears the carry flag in the IRET frame FLAGS word so the
    /// stub's IRET returns the BIOS status to the caller.
    fn set_iret_cf(&mut self, cpu: &impl Cpu, error: bool) {
        let flags_address = iret_stack_base(cpu).wrapping_add(4);
        let flags = self.read_mem_word(flags_address);
        let flags = if error {
            flags | 0x0001
        } else {
            flags & !0x0001
        };
        self.write_mem_word(flags_address, flags);
    }

    /// Sets or clears the zero flag in the IRET frame FLAGS word so the
    /// stub's IRET returns the BIOS status to the caller.
    fn set_iret_zf(&mut self, cpu: &impl Cpu, zero: bool) {
        let flags_address = iret_stack_base(cpu).wrapping_add(4);
        let flags = self.read_mem_word(flags_address);
        let flags = if zero {
            flags | 0x0040
        } else {
            flags & !0x0040
        };
        self.write_mem_word(flags_address, flags);
    }

    /// Reads a little-endian word from the stub ROM metadata header.
    pub(crate) fn stub_rom_metadata_word(&self, offset: usize) -> u16 {
        u16::from(self.memory.bios_byte(offset))
            | (u16::from(self.memory.bios_byte(offset + 1)) << 8)
    }
}

#[cfg(test)]
mod tests {
    use common::{Bus, Cpu, CpuType, NoTrace, SegmentRegister};

    use super::{
        PAGE_ACCESSED, PAGE_DIRTY, hle_page_translate_read, hle_page_translate_write,
        hle_read_dword, hle_write_dword,
    };
    use crate::{AtBus, memory::AtMemory, rom::LoadedRoms};

    #[derive(Default)]
    struct TestCpu {
        ax: u16,
        bx: u16,
        cx: u16,
        dx: u16,
        sp: u16,
        bp: u16,
        si: u16,
        di: u16,
        es: u16,
        cs: u16,
        ss: u16,
        ds: u16,
        ip: u16,
        flags: u16,
        es_base: u32,
        cs_base: u32,
        ss_base: u32,
        ds_base: u32,
    }

    impl TestCpu {
        fn set_segment_base_for_test(&mut self, seg: SegmentRegister, base: u32) {
            match seg {
                SegmentRegister::ES => self.es_base = base,
                SegmentRegister::CS => self.cs_base = base,
                SegmentRegister::SS => self.ss_base = base,
                SegmentRegister::DS => self.ds_base = base,
            }
        }
    }

    impl Cpu for TestCpu {
        fn run_for(&mut self, _cycles_to_run: u64, _bus: &mut impl Bus) -> u64 {
            0
        }

        fn reset(&mut self) {}

        fn halted(&self) -> bool {
            false
        }

        fn ax(&self) -> u16 {
            self.ax
        }

        fn set_ax(&mut self, v: u16) {
            self.ax = v;
        }

        fn bx(&self) -> u16 {
            self.bx
        }

        fn set_bx(&mut self, v: u16) {
            self.bx = v;
        }

        fn cx(&self) -> u16 {
            self.cx
        }

        fn set_cx(&mut self, v: u16) {
            self.cx = v;
        }

        fn dx(&self) -> u16 {
            self.dx
        }

        fn set_dx(&mut self, v: u16) {
            self.dx = v;
        }

        fn sp(&self) -> u16 {
            self.sp
        }

        fn set_sp(&mut self, v: u16) {
            self.sp = v;
        }

        fn bp(&self) -> u16 {
            self.bp
        }

        fn set_bp(&mut self, v: u16) {
            self.bp = v;
        }

        fn si(&self) -> u16 {
            self.si
        }

        fn set_si(&mut self, v: u16) {
            self.si = v;
        }

        fn di(&self) -> u16 {
            self.di
        }

        fn set_di(&mut self, v: u16) {
            self.di = v;
        }

        fn es(&self) -> u16 {
            self.es
        }

        fn set_es(&mut self, v: u16) {
            self.es = v;
            self.es_base = u32::from(v) << 4;
        }

        fn cs(&self) -> u16 {
            self.cs
        }

        fn set_cs(&mut self, v: u16) {
            self.cs = v;
            self.cs_base = u32::from(v) << 4;
        }

        fn ss(&self) -> u16 {
            self.ss
        }

        fn set_ss(&mut self, v: u16) {
            self.ss = v;
            self.ss_base = u32::from(v) << 4;
        }

        fn ds(&self) -> u16 {
            self.ds
        }

        fn set_ds(&mut self, v: u16) {
            self.ds = v;
            self.ds_base = u32::from(v) << 4;
        }

        fn ip(&self) -> u16 {
            self.ip
        }

        fn set_ip(&mut self, v: u16) {
            self.ip = v;
        }

        fn flags(&self) -> u16 {
            self.flags
        }

        fn set_flags(&mut self, v: u16) {
            self.flags = v;
        }

        fn cpu_type(&self) -> CpuType {
            CpuType::I386
        }

        fn load_segment_real_mode(&mut self, seg: SegmentRegister, selector: u16) {
            match seg {
                SegmentRegister::ES => self.set_es(selector),
                SegmentRegister::CS => self.set_cs(selector),
                SegmentRegister::SS => self.set_ss(selector),
                SegmentRegister::DS => self.set_ds(selector),
            }
        }

        fn segment_base(&self, seg: SegmentRegister) -> u32 {
            match seg {
                SegmentRegister::ES => self.es_base,
                SegmentRegister::CS => self.cs_base,
                SegmentRegister::SS => self.ss_base,
                SegmentRegister::DS => self.ds_base,
            }
        }
    }

    fn test_bus() -> AtBus<NoTrace> {
        AtBus::new(
            50_000_000,
            8 * 1024 * 1024,
            LoadedRoms::hle_stub_set(),
            48000,
        )
    }

    fn test_memory() -> AtMemory {
        let roms = LoadedRoms::hle_stub_set();
        AtMemory::new(8 * 1024 * 1024, roms.system_bios, roms.vga_bios)
    }

    fn write_bus_dword(bus: &mut AtBus<NoTrace>, address: u32, value: u32) {
        bus.write_byte(address, value as u8);
        bus.write_byte(address + 1, (value >> 8) as u8);
        bus.write_byte(address + 2, (value >> 16) as u8);
        bus.write_byte(address + 3, (value >> 24) as u8);
    }

    fn setup_hle_page_tables(bus: &mut AtBus<NoTrace>) {
        const PAGE_PRESENT_WRITE: u32 = 0x03;
        let page_dir = 0x0008_0000;
        let page_table = 0x0008_1000;

        write_bus_dword(bus, page_dir, page_table | PAGE_PRESENT_WRITE);
        for page in 0..256u32 {
            write_bus_dword(
                bus,
                page_table + page * 4,
                (page * 0x1000) | PAGE_PRESENT_WRITE,
            );
        }

        write_bus_dword(bus, page_table + 0x20 * 4, 0x0003_0000 | PAGE_PRESENT_WRITE);
        bus.set_hle_paging(0x8000_0001, page_dir);
    }

    fn write_bus_word(bus: &mut AtBus<NoTrace>, address: u32, value: u16) {
        bus.write_byte(address, value as u8);
        bus.write_byte(address + 1, (value >> 8) as u8);
    }

    fn read_bus_word(bus: &mut AtBus<NoTrace>, address: u32) -> u16 {
        u16::from(bus.read_byte(address)) | (u16::from(bus.read_byte(address + 1)) << 8)
    }

    #[test]
    fn hle_translate_read_sets_accessed_bits() {
        let mut memory = test_memory();
        let cr0 = 0x8000_0001;
        let cr3 = 0x0000_1000;
        let linear = 0x0040_1234;
        let pde_addr = cr3 + 4;
        let pte_addr = 0x0000_2004;

        hle_write_dword(&mut memory, pde_addr, 0x0000_2003);
        hle_write_dword(&mut memory, pte_addr, 0x0000_3003);

        let physical = hle_page_translate_read(cr0, cr3, linear, &mut memory);

        assert_eq!(physical, 0x0000_3234);
        assert_eq!(
            hle_read_dword(&memory, pde_addr),
            0x0000_2003 | PAGE_ACCESSED
        );
        assert_eq!(
            hle_read_dword(&memory, pte_addr),
            0x0000_3003 | PAGE_ACCESSED
        );
    }

    #[test]
    fn hle_translate_write_sets_accessed_and_dirty_bits() {
        let mut memory = test_memory();
        let cr0 = 0x8000_0001;
        let cr3 = 0x0000_1000;
        let linear = 0x0040_1234;
        let pde_addr = cr3 + 4;
        let pte_addr = 0x0000_2004;

        hle_write_dword(&mut memory, pde_addr, 0x0000_2003);
        hle_write_dword(&mut memory, pte_addr, 0x0000_3003);

        let physical = hle_page_translate_write(cr0, cr3, linear, &mut memory);

        assert_eq!(physical, 0x0000_3234);
        assert_eq!(
            hle_read_dword(&memory, pde_addr),
            0x0000_2003 | PAGE_ACCESSED
        );
        assert_eq!(
            hle_read_dword(&memory, pte_addr),
            0x0000_3003 | PAGE_ACCESSED | PAGE_DIRTY
        );
    }

    #[test]
    fn hle_translate_non_present_returns_linear_without_side_effects() {
        let mut memory = test_memory();
        let cr0 = 0x8000_0001;
        let cr3 = 0x0000_1000;
        let linear = 0x0040_1234;
        let pde_addr = cr3 + 4;

        hle_write_dword(&mut memory, pde_addr, 0x0000_2002);

        let physical = hle_page_translate_write(cr0, cr3, linear, &mut memory);

        assert_eq!(physical, linear);
        assert_eq!(hle_read_dword(&memory, pde_addr), 0x0000_2002);
    }

    #[test]
    fn hle_translate_paging_disabled_returns_linear() {
        let mut memory = test_memory();
        let linear = 0x0040_1234;

        let physical = hle_page_translate_write(0, 0x0000_1000, linear, &mut memory);

        assert_eq!(physical, linear);
    }

    #[test]
    fn execute_bios_hle_restores_saved_registers_from_paged_stack() {
        let mut bus = test_bus();
        setup_hle_page_tables(&mut bus);
        let mut cpu = TestCpu::default();
        cpu.set_sp(0);
        cpu.set_segment_base_for_test(SegmentRegister::SS, 0x0002_0000);

        write_bus_word(&mut bus, 0x0002_0000, 0xAAAA);
        write_bus_word(&mut bus, 0x0002_0002, 0xBBBB);
        write_bus_word(&mut bus, 0x0003_0000, 0x5678);
        write_bus_word(&mut bus, 0x0003_0002, 0x0200);
        bus.bios.write_trap_port(0x09);

        bus.execute_bios_hle(&mut cpu);

        assert_eq!(cpu.dx(), 0x5678);
        assert_eq!(cpu.ax(), 0x0200);
        assert_eq!(cpu.sp(), 0x0004);
        assert_eq!(read_bus_word(&mut bus, 0x0002_0000), 0xAAAA);
        assert_eq!(read_bus_word(&mut bus, 0x0002_0002), 0xBBBB);
    }

    #[test]
    fn set_iret_cf_uses_paged_stack_frame() {
        let mut bus = test_bus();
        setup_hle_page_tables(&mut bus);
        let mut cpu = TestCpu::default();
        cpu.set_sp(0);
        cpu.set_segment_base_for_test(SegmentRegister::SS, 0x0002_0000);

        // Linear 0x20004 maps to physical 0x30004 through the page tables.
        write_bus_word(&mut bus, 0x0003_0004, 0x0202);

        bus.set_iret_cf(&cpu, true);
        assert_eq!(read_bus_word(&mut bus, 0x0003_0004), 0x0203);

        bus.set_iret_cf(&cpu, false);
        assert_eq!(read_bus_word(&mut bus, 0x0003_0004), 0x0202);
    }

    #[test]
    fn int15h_unsupported_function_reports_error() {
        let mut bus = test_bus();
        let mut cpu = TestCpu::default();
        // The stub pushed AX and DX, so SP sits 4 below the IRET frame slot.
        cpu.set_sp(0x7BF6);
        cpu.set_ss(0x0000);

        write_bus_word(&mut bus, 0x7BF6, 0x0000);
        write_bus_word(&mut bus, 0x7BF8, 0xE000);
        write_bus_word(&mut bus, 0x7BFE, 0x0202);
        bus.bios.write_trap_port(0x15);

        bus.execute_bios_hle(&mut cpu);

        assert_eq!(cpu.ah(), 0x86);
        assert_eq!(read_bus_word(&mut bus, 0x7BFE), 0x0203);
    }

    #[test]
    fn int13h_invalid_function_reports_bad_command() {
        let mut bus = test_bus();
        let mut cpu = TestCpu::default();
        // The stub pushed AX and DX, so SP sits 4 below the IRET frame slot.
        cpu.set_sp(0x7BF6);
        cpu.set_ss(0x0000);

        write_bus_word(&mut bus, 0x7BF6, 0x0000);
        write_bus_word(&mut bus, 0x7BF8, 0xFF00);
        write_bus_word(&mut bus, 0x7BFE, 0x0202);
        bus.bios.write_trap_port(0x13);

        bus.execute_bios_hle(&mut cpu);

        assert_eq!(cpu.ah(), 0x01);
        assert_eq!(read_bus_word(&mut bus, 0x7BFE), 0x0203);
        assert_eq!(bus.read_byte(0x441), 0x01);
    }

    #[test]
    fn int0eh_preserves_registers_and_frame() {
        let mut bus = test_bus();
        let mut cpu = TestCpu::default();
        cpu.set_sp(0x7BF6);
        cpu.set_ss(0x0000);

        write_bus_word(&mut bus, 0x7BF6, 0x1234);
        write_bus_word(&mut bus, 0x7BF8, 0x5678);
        write_bus_word(&mut bus, 0x7BFE, 0x0202);
        bus.bios.write_trap_port(0x0E);

        bus.execute_bios_hle(&mut cpu);

        assert_eq!(cpu.dx(), 0x1234);
        assert_eq!(cpu.ax(), 0x5678);
        assert_eq!(cpu.sp(), 0x7BFA);
        assert_eq!(read_bus_word(&mut bus, 0x7BFE), 0x0202);
        assert_eq!(bus.read_byte(0x43E) & 0x80, 0x80);
    }

    #[test]
    fn int76h_preserves_registers_and_frame() {
        let mut bus = test_bus();
        let mut cpu = TestCpu::default();
        cpu.set_sp(0x7BF6);
        cpu.set_ss(0x0000);

        write_bus_word(&mut bus, 0x7BF6, 0x1234);
        write_bus_word(&mut bus, 0x7BF8, 0x5678);
        write_bus_word(&mut bus, 0x7BFE, 0x0202);
        bus.bios.write_trap_port(0x76);

        bus.execute_bios_hle(&mut cpu);

        assert_eq!(cpu.dx(), 0x1234);
        assert_eq!(cpu.ax(), 0x5678);
        assert_eq!(cpu.sp(), 0x7BFA);
        assert_eq!(read_bus_word(&mut bus, 0x7BFE), 0x0202);
        assert_eq!(bus.read_byte(0x48E), 0xFF);
    }

    /// Builds a one-cylinder AT-flat hard disk image.
    fn one_cylinder_hdd() -> device::disk::HddImage {
        let data = vec![0u8; 16 * 63 * 512];
        device::disk::HddImage::from_at_flat(data).expect("valid AT flat image")
    }

    #[test]
    fn int13h_routes_hard_disk_calls_to_the_hdd_service() {
        let mut bus = test_bus();
        bus.insert_hdd(0, one_cylinder_hdd(), None)
            .expect("insert succeeds");
        let mut cpu = TestCpu::default();
        cpu.set_sp(0x7BF6);
        cpu.set_ss(0x0000);

        // Saved DX selects drive 80h, saved AX requests the invalid
        // function FFh.
        write_bus_word(&mut bus, 0x7BF6, 0x0080);
        write_bus_word(&mut bus, 0x7BF8, 0xFF00);
        write_bus_word(&mut bus, 0x7BFE, 0x0202);
        bus.bios.write_trap_port(0x13);

        bus.execute_bios_hle(&mut cpu);

        assert_eq!(cpu.ah(), 0x01);
        assert_eq!(read_bus_word(&mut bus, 0x7BFE), 0x0203);
        assert_eq!(bus.read_byte(0x474), 0x01, "hard disk status 40:74");
        assert_eq!(bus.read_byte(0x441), 0x00, "diskette status untouched");
    }

    #[test]
    fn int40h_keeps_hard_disk_calls_in_the_floppy_service() {
        let mut bus = test_bus();
        bus.insert_hdd(0, one_cylinder_hdd(), None)
            .expect("insert succeeds");
        let mut cpu = TestCpu::default();
        cpu.set_sp(0x7BF6);
        cpu.set_ss(0x0000);

        write_bus_word(&mut bus, 0x7BF6, 0x0080);
        write_bus_word(&mut bus, 0x7BF8, 0x0200);
        write_bus_word(&mut bus, 0x7BFE, 0x0202);
        bus.bios.write_trap_port(0x40);

        bus.execute_bios_hle(&mut cpu);

        assert_eq!(cpu.ah(), 0x01);
        assert_eq!(read_bus_word(&mut bus, 0x7BFE), 0x0203);
        assert_eq!(bus.read_byte(0x441), 0x01, "diskette status 40:41");
        assert_eq!(bus.read_byte(0x474), 0x00, "hard disk status untouched");
    }

    #[test]
    fn int13h_without_hard_disks_stays_in_the_floppy_service() {
        let mut bus = test_bus();
        let mut cpu = TestCpu::default();
        cpu.set_sp(0x7BF6);
        cpu.set_ss(0x0000);

        write_bus_word(&mut bus, 0x7BF6, 0x0080);
        write_bus_word(&mut bus, 0x7BF8, 0x0200);
        write_bus_word(&mut bus, 0x7BFE, 0x0202);
        bus.bios.write_trap_port(0x13);

        bus.execute_bios_hle(&mut cpu);

        assert_eq!(cpu.ah(), 0x01);
        assert_eq!(bus.read_byte(0x441), 0x01, "diskette status 40:41");
        assert_eq!(bus.read_byte(0x474), 0x00, "hard disk status untouched");
    }

    /// Seeds the BDA keyboard buffer pointers into their post-POST state.
    fn seed_keyboard_bda(bus: &mut AtBus<NoTrace>) {
        write_bus_word(bus, 0x480, 0x001E);
        write_bus_word(bus, 0x482, 0x003E);
        write_bus_word(bus, 0x41A, 0x001E);
        write_bus_word(bus, 0x41C, 0x001E);
    }

    #[test]
    fn int09h_buffers_translated_scancode() {
        let mut bus = test_bus();
        seed_keyboard_bda(&mut bus);
        let mut cpu = TestCpu::default();
        cpu.set_sp(0x7BF6);
        cpu.set_ss(0x0000);
        write_bus_word(&mut bus, 0x7BFE, 0x0202);

        bus.io_write(0x07F1, 0x1E);
        bus.execute_bios_hle(&mut cpu);

        assert_eq!(read_bus_word(&mut bus, 0x41E), 0x1E61);
        assert_eq!(read_bus_word(&mut bus, 0x41A), 0x001E);
        assert_eq!(read_bus_word(&mut bus, 0x41C), 0x0020);
    }

    #[test]
    fn keyboard_buffer_wraps_and_drops_when_full() {
        let mut bus = test_bus();
        seed_keyboard_bda(&mut bus);
        write_bus_word(&mut bus, 0x41A, 0x0020);
        write_bus_word(&mut bus, 0x41C, 0x003C);
        let mut cpu = TestCpu::default();
        cpu.set_ss(0x0000);

        cpu.set_sp(0x7BF6);
        write_bus_word(&mut bus, 0x7BFE, 0x0202);
        bus.io_write(0x07F1, 0x1E);
        bus.execute_bios_hle(&mut cpu);
        assert_eq!(read_bus_word(&mut bus, 0x43C), 0x1E61, "entry at the end");
        assert_eq!(read_bus_word(&mut bus, 0x41C), 0x001E, "tail wrapped");

        cpu.set_sp(0x7BF6);
        bus.io_write(0x07F1, 0x30);
        bus.execute_bios_hle(&mut cpu);
        assert_eq!(read_bus_word(&mut bus, 0x41C), 0x001E, "full buffer drops");
    }

    #[test]
    fn int09h_ctrl_alt_del_retargets_frame_to_cold_entry() {
        let mut bus = test_bus();
        seed_keyboard_bda(&mut bus);
        let mut cpu = TestCpu::default();
        cpu.set_sp(0x7BF6);
        cpu.set_ss(0x0000);
        // Control and alt held.
        bus.write_byte(0x417, 0x0C);
        write_bus_word(&mut bus, 0x7BFE, 0x0202);

        bus.io_write(0x07F1, 0x53);
        bus.execute_bios_hle(&mut cpu);

        let cold_entry =
            u16::from(bus.memory.bios_byte(2)) | (u16::from(bus.memory.bios_byte(3)) << 8);
        assert_eq!(read_bus_word(&mut bus, 0x472), 0x1234, "warm boot flag");
        assert!(bus.needs_full_reinit, "POST re-runs on the next 0xF0 trap");
        assert_eq!(cpu.sp(), 0x7BFA);
        assert_eq!(read_bus_word(&mut bus, 0x7BFA), cold_entry);
        assert_eq!(read_bus_word(&mut bus, 0x7BFC), 0xF000);
        assert_eq!(read_bus_word(&mut bus, 0x7BFE), 0x0002, "IF cleared");
    }

    #[test]
    fn int09h_ctrl_break_restacks_frame_for_the_rom_helper() {
        let mut bus = test_bus();
        seed_keyboard_bda(&mut bus);
        let mut cpu = TestCpu::default();
        cpu.set_sp(0x7BF6);
        cpu.set_ss(0x0000);
        // Control held.
        bus.write_byte(0x417, 0x04);
        write_bus_word(&mut bus, 0x7BFA, 0x1111);
        write_bus_word(&mut bus, 0x7BFC, 0x2222);
        write_bus_word(&mut bus, 0x7BFE, 0x0202);

        bus.io_write(0x07F1, 0x46);
        bus.execute_bios_hle(&mut cpu);

        let helper =
            u16::from(bus.memory.bios_byte(10)) | (u16::from(bus.memory.bios_byte(11)) << 8);
        assert_eq!(cpu.sp(), 0x7BF4, "helper frame pushed below the original");
        assert_eq!(read_bus_word(&mut bus, 0x7BF4), helper);
        assert_eq!(read_bus_word(&mut bus, 0x7BF6), 0xF000);
        assert_eq!(read_bus_word(&mut bus, 0x7BF8), 0x0002, "IF cleared");
        assert_eq!(read_bus_word(&mut bus, 0x7BFA), 0x1111, "original IP kept");
        assert_eq!(read_bus_word(&mut bus, 0x7BFC), 0x2222, "original CS kept");
        assert_eq!(
            read_bus_word(&mut bus, 0x7BFE),
            0x0202,
            "original FLAGS kept"
        );
        assert_eq!(bus.read_byte(0x471) & 0x80, 0x80, "break flag raised");
        assert_eq!(read_bus_word(&mut bus, 0x41E), 0x0000, "null keystroke");
        assert_eq!(read_bus_word(&mut bus, 0x41C), 0x0020);
    }

    #[test]
    fn int16h_blocking_read_rewinds_paged_iret_frame() {
        let mut bus = test_bus();
        setup_hle_page_tables(&mut bus);
        seed_keyboard_bda(&mut bus);
        let mut cpu = TestCpu::default();
        cpu.set_sp(0);
        cpu.set_segment_base_for_test(SegmentRegister::SS, 0x0002_0000);

        // Saved DX/AX below the frame; AH=00h requests the blocking read.
        write_bus_word(&mut bus, 0x0003_0000, 0x0000);
        write_bus_word(&mut bus, 0x0003_0002, 0x0000);
        write_bus_word(&mut bus, 0x0003_0004, 0x1234);
        write_bus_word(&mut bus, 0x0003_0008, 0x0002);

        bus.bios.write_trap_port(0x16);
        bus.execute_bios_hle(&mut cpu);

        assert_eq!(
            read_bus_word(&mut bus, 0x0003_0004),
            0x1232,
            "IP rewound over the INT 16h instruction"
        );
        assert_eq!(
            read_bus_word(&mut bus, 0x0003_0008),
            0x0202,
            "IF forced on for the wait"
        );
    }

    #[test]
    fn int16h_set_typematic_programs_the_keyboard() {
        let mut bus = test_bus();
        let mut cpu = TestCpu::default();
        cpu.set_sp(0x7BF6);
        cpu.set_ss(0x0000);
        cpu.set_bx(0x0103);
        // AH=03h AL=05h restored from the stacked AX save.
        write_bus_word(&mut bus, 0x7BF8, 0x0305);

        bus.bios.write_trap_port(0x16);
        bus.execute_bios_hle(&mut cpu);

        assert_eq!(
            bus.kbc.keyboard.typematic, 0x23,
            "delay 1 in bits 5-6, rate 3 in bits 0-4"
        );
    }

    #[test]
    fn set_iret_zf_edits_paged_stack_frame() {
        let mut bus = test_bus();
        setup_hle_page_tables(&mut bus);
        let mut cpu = TestCpu::default();
        cpu.set_sp(0);
        cpu.set_segment_base_for_test(SegmentRegister::SS, 0x0002_0000);

        // Linear 0x20004 maps to physical 0x30004 through the page tables.
        write_bus_word(&mut bus, 0x0003_0004, 0x0202);

        bus.set_iret_zf(&cpu, true);
        assert_eq!(read_bus_word(&mut bus, 0x0003_0004), 0x0242);

        bus.set_iret_zf(&cpu, false);
        assert_eq!(read_bus_word(&mut bus, 0x0003_0004), 0x0202);
    }

    #[test]
    fn bootstrap_failure_rewrites_iret_frame_to_halt_loop() {
        let mut bus = test_bus();
        let mut cpu = TestCpu::default();
        // The stub pushed AX and DX, so SP sits 4 below the IRET frame slot.
        cpu.set_sp(0x7BF6);
        cpu.set_ss(0x0000);

        bus.bios.write_trap_port(0xF2);
        bus.execute_bios_hle(&mut cpu);

        // No bootable media: the fabricated IRET frame must land in the
        // F-segment halt loop published by the stub ROM metadata.
        let halt_offset =
            u16::from(bus.memory.bios_byte(6)) | (u16::from(bus.memory.bios_byte(7)) << 8);
        assert_eq!(cpu.sp(), 0x7BFA);
        assert_eq!(read_bus_word(&mut bus, 0x7BFA), halt_offset);
        assert_eq!(read_bus_word(&mut bus, 0x7BFC), 0xF000);
    }
}
