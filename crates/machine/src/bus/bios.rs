//! BIOS HLE handler implementations.
//!
//! Each handler reads/writes CPU registers directly via the `Cpu` trait.
//! The ROM stubs save AX/DX on the stack (clobbered by the trap OUT),
//! write the vector number to the trap port, and IRET. The Rust side
//! restores AX/DX from the stack before dispatching to the handler.

mod bootstrap;
mod cmt_printer;
mod comed;
mod crt;
mod disk;
mod floppy_interrupt;
mod graphics;
mod keyboard;
mod pmode;
mod serial_rs232c;
mod timer;

use common::{Cpu, SegmentRegister, warn};

use super::{
    Pc9801Bus,
    dos_adapter::{DosCpuAccess, DosCursorAccess, DosDiskIo, DosMemoryAccess},
};
use crate::{Tracing, memory::Pc9801Memory};

const PIT_CLOCK_8MHZ_LINEAGE: u32 = 1_996_800;
const PAGE_PRESENT: u32 = 0x01;
const PAGE_ACCESSED: u32 = 0x20;
const PAGE_DIRTY: u32 = 0x40;

fn iret_stack_base(cpu: &impl Cpu) -> u32 {
    cpu.segment_base(SegmentRegister::SS)
        .wrapping_add(u32::from(cpu.sp()))
}

fn hle_read_dword(memory: &Pc9801Memory, address: u32) -> u32 {
    memory.read_byte(address) as u32
        | ((memory.read_byte(address + 1) as u32) << 8)
        | ((memory.read_byte(address + 2) as u32) << 16)
        | ((memory.read_byte(address + 3) as u32) << 24)
}

fn hle_write_dword(memory: &mut Pc9801Memory, address: u32, value: u32) {
    memory.write_byte(address, value as u8);
    memory.write_byte(address + 1, (value >> 8) as u8);
    memory.write_byte(address + 2, (value >> 16) as u8);
    memory.write_byte(address + 3, (value >> 24) as u8);
}

fn hle_page_translate_access(
    cr0: u32,
    cr3: u32,
    linear: u32,
    write: bool,
    memory: &mut Pc9801Memory,
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

pub(super) fn hle_page_translate_read(
    cr0: u32,
    cr3: u32,
    linear: u32,
    memory: &mut Pc9801Memory,
) -> u32 {
    hle_page_translate_access(cr0, cr3, linear, false, memory)
}

pub(super) fn hle_page_translate_write(
    cr0: u32,
    cr3: u32,
    linear: u32,
    memory: &mut Pc9801Memory,
) -> u32 {
    hle_page_translate_access(cr0, cr3, linear, true, memory)
}

/// Read-only linear-to-physical translation for HLE accessors that only hold
/// an immutable memory reference and therefore cannot update the accessed and
/// dirty bits. Returns the linear address unchanged when paging is disabled or
/// the mapping is not present.
pub(super) fn hle_page_translate_read_only(
    cr0: u32,
    cr3: u32,
    linear: u32,
    memory: &Pc9801Memory,
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
    (pte & 0xFFFFF000) | offset
}

fn boot_sector_has_signature(data: &[u8]) -> bool {
    data.len() >= 0x400 && data[0x3FE] == 0x55 && data[0x3FF] == 0xAA
}

/// Computes the CGROM byte offset for a Kanji character given JIS row/col.
///
/// The font ROM uses an interleaved layout: each JIS column occupies a 4096-byte block,
/// with rows packed at 16-byte intervals within. Left half at the computed offset,
/// right half at offset + 0x800.
fn cgrom_kanji_offset(jis_row: u8, jis_col: u8) -> u32 {
    let col = (jis_col & 0x7F) as u32;
    let row = (jis_row.wrapping_sub(0x20) & 0x7F) as u32;
    col * 0x1000 + row * 16
}

fn reverse_bits(b: u8) -> u8 {
    let mut v = b;
    v = (v & 0xF0) >> 4 | (v & 0x0F) << 4;
    v = (v & 0xCC) >> 2 | (v & 0x33) << 2;
    (v & 0xAA) >> 1 | (v & 0x55) << 1
}

impl<T: Tracing> Pc9801Bus<T> {
    pub(crate) fn handle_bios_interval_timer_tick(&mut self) {
        if !self.bios_interval_timer_active {
            return;
        }

        let count = self.ram_read_u16(0x058A);
        let new_count = count.wrapping_sub(1);
        self.ram_write_u16(0x058A, new_count);

        if count > 0 && new_count == 0 {
            self.bios_interval_timer_active = false;
        }
    }

    /// Configures paging state used by HLE BIOS routines (SASI, INT 1Fh, etc.).
    /// When paging is active (CR0.PG + CR0.PE), HLE memory accesses translate
    /// linear addresses through the page tables rooted at CR3.
    pub fn set_hle_paging(&mut self, cr0: u32, cr3: u32) {
        self.hle_cr0 = cr0;
        self.hle_cr3 = cr3;
    }

    /// Executes the pending BIOS HLE operation with direct CPU register access.
    pub(crate) fn execute_bios_hle(&mut self, cpu: &mut impl Cpu) {
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

        self.tracer.trace_bios_hle(vector, cpu.ah(), cpu.al());

        match vector {
            0x08 => self.hle_int08h(cpu),
            0x09 => self.hle_int09h(cpu),
            0x0A => {
                self.pic.write_port0(0, 0x20);
                self.display_control.state.vsync_irq_enabled = true;
            }
            0x0B | 0x0D | 0x0E => self.pic.write_port0(0, 0x20),
            0x0C => self.hle_int0ch(cpu),
            0x10 | 0x11 | 0x14..=0x17 => {
                self.pic.write_port0(1, 0x20);
                self.pic.write_port0(0, 0x20);
            }
            0x12 => self.hle_int12h(cpu),
            0x13 => self.hle_int13h(cpu),
            0x18 => self.hle_int18h(cpu),
            0x19 => self.hle_int19h(cpu),
            0x1A => self.hle_int1ah(cpu),
            0x1B => self.hle_int1bh(cpu),
            0x1C => self.hle_int1ch(cpu),
            0x1F => self.hle_int1fh(cpu),
            0x20..=0x2A | 0x2F | 0x33 | 0x67 | 0xDC | 0xE7 | 0xEF | 0xFD | 0xFE => {
                if let Some(mut neetan_dos) = self.dos.take() {
                    let mut cpu_access = DosCpuAccess(cpu);
                    let access_page = self.access_page_index();
                    let mut mem_access = DosMemoryAccess::with_paging(
                        &mut self.memory,
                        access_page,
                        self.b_bank_ems,
                        self.vram_ems_bank,
                        self.hle_cr0,
                        self.hle_cr3,
                    );
                    let mut disk_io = DosDiskIo {
                        floppy: &mut self.floppy,
                        sasi: &mut self.sasi,
                        ide: &mut self.ide,
                    };
                    let mut cursor_access = DosCursorAccess(&mut self.gdc_master.state);
                    neetan_dos.dispatch(
                        vector,
                        &mut cpu_access,
                        &mut mem_access,
                        &mut disk_io,
                        &mut cursor_access,
                        &mut self.tracer,
                    );
                    self.dos = Some(neetan_dos);
                }
            }
            0xD2 => {}
            0xF0 => {
                if std::mem::take(&mut self.needs_full_reinit) {
                    self.initialize_post_boot_state();
                }
                self.hle_bootstrap(cpu);
            }
            0xF1 | 0xF2 => self.hle_bootstrap(cpu),
            0xF3 => self.hle_n88_basic_entry(),
            _ => {}
        }
    }

    fn hle_n88_basic_entry(&mut self) {
        warn!("N88-BASIC ROM entry invoked; this software requires an original ROM to run");
    }

    fn hle_linear_address(&self, cpu: &impl Cpu, seg: SegmentRegister, off: u32) -> u32 {
        cpu.segment_base(seg).wrapping_add(off)
    }

    fn hle_physical_address(
        &mut self,
        cpu: &impl Cpu,
        seg: SegmentRegister,
        off: u32,
        write: bool,
    ) -> u32 {
        let linear = self.hle_linear_address(cpu, seg, off);
        hle_page_translate_access(self.hle_cr0, self.hle_cr3, linear, write, &mut self.memory)
    }

    fn hle_read_byte(&mut self, cpu: &impl Cpu, seg: SegmentRegister, off: u32) -> u8 {
        let phys = self.hle_physical_address(cpu, seg, off, false);
        self.read_byte_direct(phys)
    }

    fn hle_write_byte(&mut self, cpu: &impl Cpu, seg: SegmentRegister, off: u32, value: u8) {
        let phys = self.hle_physical_address(cpu, seg, off, true);
        self.memory.write_byte(phys, value);
    }

    fn set_iret_cf(&mut self, cpu: &impl Cpu, error: bool) {
        let base = iret_stack_base(cpu);
        let flags_addr = base + 0x04;
        let mut flags = self.read_mem_word(flags_addr);
        if error {
            flags |= 0x0001;
        } else {
            flags &= !0x0001;
        }
        self.write_mem_word(flags_addr, flags);
    }

    fn write_result_ah_cf(&mut self, cpu: &mut impl Cpu, result_ah: u8) {
        cpu.set_ah(result_ah);
        self.set_iret_cf(cpu, result_ah >= 0x20);
    }

    fn read_mem_byte(&mut self, addr: u32) -> u8 {
        let phys = hle_page_translate_read(self.hle_cr0, self.hle_cr3, addr, &mut self.memory);
        self.memory.read_byte(phys)
    }

    fn read_mem_word(&mut self, addr: u32) -> u16 {
        u16::from(self.read_mem_byte(addr)) | (u16::from(self.read_mem_byte(addr + 1)) << 8)
    }

    fn write_mem_byte(&mut self, addr: u32, value: u8) {
        let phys = hle_page_translate_write(self.hle_cr0, self.hle_cr3, addr, &mut self.memory);
        self.memory.write_byte(phys, value);
    }

    fn write_mem_word(&mut self, addr: u32, value: u16) {
        self.write_mem_byte(addr, value as u8);
        self.write_mem_byte(addr + 1, (value >> 8) as u8);
    }

    fn ram_read_u16(&self, addr: usize) -> u16 {
        u16::from_le_bytes([self.memory.state.ram[addr], self.memory.state.ram[addr + 1]])
    }

    fn ram_write_u16(&mut self, addr: usize, value: u16) {
        let bytes = value.to_le_bytes();
        self.memory.state.ram[addr] = bytes[0];
        self.memory.state.ram[addr + 1] = bytes[1];
    }

    fn fdc_drain_results(fdc: &mut device::upd765a_fdc::Upd765aFdc, dest: &mut [u8]) -> usize {
        let mut count = 0;
        while count < dest.len() {
            let status = fdc.read_status();
            if status & 0xD0 != 0xD0 {
                // Not (RQM | DIO | CB) - no more result bytes.
                break;
            }
            dest[count] = fdc.read_data();
            count += 1;
        }
        count
    }
}

#[cfg(test)]
mod tests {
    use common::{Bus, Cpu, CpuMode, CpuType, MachineModel, NoTracing, SegmentRegister};
    use device::floppy::FloppyImage;

    use super::{
        PAGE_ACCESSED, PAGE_DIRTY, hle_page_translate_read, hle_page_translate_write,
        hle_read_dword, hle_write_dword,
    };
    use crate::{Pc9801Bus, memory::Pc9801Memory};

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

    fn test_bus() -> Pc9801Bus<NoTracing> {
        Pc9801Bus::new(MachineModel::PC9801RA, CpuMode::Low, 48000)
    }

    fn test_memory() -> Pc9801Memory {
        Pc9801Memory::new(MachineModel::PC9801RA, 0x400000)
    }

    fn write_bus_dword(bus: &mut Pc9801Bus<NoTracing>, address: u32, value: u32) {
        bus.write_byte(address, value as u8);
        bus.write_byte(address + 1, (value >> 8) as u8);
        bus.write_byte(address + 2, (value >> 16) as u8);
        bus.write_byte(address + 3, (value >> 24) as u8);
    }

    fn setup_hle_page_tables(bus: &mut Pc9801Bus<NoTracing>) {
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

    fn write_bus_word(bus: &mut Pc9801Bus<NoTracing>, address: u32, value: u16) {
        bus.write_byte(address, value as u8);
        bus.write_byte(address + 1, (value >> 8) as u8);
    }

    fn read_bus_word(bus: &mut Pc9801Bus<NoTracing>, address: u32) -> u16 {
        u16::from(bus.read_byte(address)) | (u16::from(bus.read_byte(address + 1)) << 8)
    }

    fn test_hdm_floppy(first_sector_byte: u8) -> FloppyImage {
        let mut data = vec![0u8; 1_261_568];
        data[0] = first_sector_byte;
        FloppyImage::from_hdm_bytes(&data).unwrap()
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
        bus.bios.write_trap_port(0x18);

        bus.execute_bios_hle(&mut cpu);

        assert_eq!(cpu.dx(), 0x5678);
        assert_eq!(cpu.ax(), 0x0200);
        assert_eq!(cpu.sp(), 0x0004);
        assert_eq!(read_bus_word(&mut bus, 0x0002_0000), 0xAAAA);
        assert_eq!(read_bus_word(&mut bus, 0x0002_0002), 0xBBBB);
    }

    #[test]
    fn int1bh_fdd_read_uses_paged_protected_mode_buffer() {
        let mut bus = test_bus();
        setup_hle_page_tables(&mut bus);
        bus.insert_floppy(0, test_hdm_floppy(0xA5), None);

        let mut cpu = TestCpu::default();
        cpu.set_ax(0x0690);
        cpu.set_bx(0);
        cpu.set_cx(0x0300);
        cpu.set_dx(0x0001);
        cpu.set_bp(0);
        cpu.set_es(0x2222);
        cpu.set_segment_base_for_test(SegmentRegister::ES, 0x0002_0000);

        bus.hle_int1bh(&mut cpu);

        assert_eq!(cpu.ah(), 0x00);
        assert_eq!(bus.read_byte(0x0003_0000), 0xA5);
        assert_eq!(bus.read_byte(0x0002_0000), 0x00);
        assert_eq!(bus.read_byte(0x0002_2220), 0x00);
    }

    #[test]
    fn set_iret_cf_uses_paged_stack_frame() {
        let mut bus = test_bus();
        setup_hle_page_tables(&mut bus);
        let mut cpu = TestCpu::default();
        cpu.set_sp(0);
        cpu.set_segment_base_for_test(SegmentRegister::SS, 0x0002_0000);

        write_bus_word(&mut bus, 0x0002_0004, 0x5555);
        write_bus_word(&mut bus, 0x0003_0004, 0x0200);

        bus.set_iret_cf(&cpu, true);

        assert_eq!(read_bus_word(&mut bus, 0x0003_0004), 0x0201);
        assert_eq!(read_bus_word(&mut bus, 0x0002_0004), 0x5555);

        bus.set_iret_cf(&cpu, false);

        assert_eq!(read_bus_word(&mut bus, 0x0003_0004), 0x0200);
        assert_eq!(read_bus_word(&mut bus, 0x0002_0004), 0x5555);
    }

    #[test]
    fn int18h_key_read_rewinds_paged_iret_frame() {
        let mut bus = test_bus();
        setup_hle_page_tables(&mut bus);
        let mut cpu = TestCpu::default();
        cpu.set_sp(0x0100);
        cpu.set_segment_base_for_test(SegmentRegister::SS, 0x0002_0000);

        write_bus_word(&mut bus, 0x0002_0100, 0xAAAA);
        write_bus_word(&mut bus, 0x0002_0104, 0xBBBB);
        write_bus_word(&mut bus, 0x0003_0100, 0x1234);
        write_bus_word(&mut bus, 0x0003_0104, 0x0000);

        bus.int18h_key_read(&mut cpu);

        assert_eq!(read_bus_word(&mut bus, 0x0003_0100), 0x1232);
        assert_eq!(read_bus_word(&mut bus, 0x0003_0104), 0x0200);
        assert_eq!(read_bus_word(&mut bus, 0x0002_0100), 0xAAAA);
        assert_eq!(read_bus_word(&mut bus, 0x0002_0104), 0xBBBB);
    }

    #[test]
    fn hle_int08h_chains_callback_on_paged_stack() {
        let mut bus = test_bus();
        setup_hle_page_tables(&mut bus);
        let mut cpu = TestCpu::default();
        cpu.set_sp(0x0100);
        cpu.set_segment_base_for_test(SegmentRegister::SS, 0x0002_0000);

        bus.bios_interval_timer_active = true;
        bus.ram_write_u16(0x058A, 1);
        bus.ram_write_u16(0x001C, 0x2222);
        bus.ram_write_u16(0x001E, 0x3333);

        write_bus_word(&mut bus, 0x0002_00FA, 0xAAAA);
        write_bus_word(&mut bus, 0x0002_00FC, 0xBBBB);
        write_bus_word(&mut bus, 0x0002_00FE, 0xCCCC);
        write_bus_word(&mut bus, 0x0003_0104, 0x0202);

        bus.hle_int08h(&mut cpu);

        assert_eq!(cpu.sp(), 0x00FA);
        assert_eq!(read_bus_word(&mut bus, 0x0003_00FA), 0x2222);
        assert_eq!(read_bus_word(&mut bus, 0x0003_00FC), 0x3333);
        assert_eq!(read_bus_word(&mut bus, 0x0003_00FE), 0x0002);
        assert_eq!(read_bus_word(&mut bus, 0x0002_00FA), 0xAAAA);
        assert_eq!(read_bus_word(&mut bus, 0x0002_00FC), 0xBBBB);
        assert_eq!(read_bus_word(&mut bus, 0x0002_00FE), 0xCCCC);
    }

    #[test]
    fn hle_int09h_chains_copy_vector_on_paged_stack() {
        let mut bus = test_bus();
        setup_hle_page_tables(&mut bus);
        let mut cpu = TestCpu::default();
        cpu.set_sp(0);
        cpu.set_segment_base_for_test(SegmentRegister::SS, 0x0002_0000);

        bus.keyboard_chained_raw_code = Some(0x60);
        bus.ram_write_u16(0x0018, 0x4321);
        bus.ram_write_u16(0x001A, 0x8765);

        write_bus_word(&mut bus, 0x0002_0000, 0xAAAA);
        write_bus_word(&mut bus, 0x0002_0002, 0xBBBB);
        write_bus_word(&mut bus, 0x0002_0004, 0xCCCC);
        write_bus_word(&mut bus, 0x0003_0000, 0x1111);
        write_bus_word(&mut bus, 0x0003_0002, 0x2222);
        write_bus_word(&mut bus, 0x0003_0004, 0x0202);

        bus.hle_int09h(&mut cpu);

        assert_eq!(read_bus_word(&mut bus, 0x0003_0000), 0x4321);
        assert_eq!(read_bus_word(&mut bus, 0x0003_0002), 0x8765);
        assert_eq!(read_bus_word(&mut bus, 0x0003_0004), 0x0002);
        assert_eq!(read_bus_word(&mut bus, 0x0003_0006), 0x1111);
        assert_eq!(read_bus_word(&mut bus, 0x0003_0008), 0x2222);
        assert_eq!(read_bus_word(&mut bus, 0x0003_000A), 0x0202);
        assert_eq!(read_bus_word(&mut bus, 0x0002_0000), 0xAAAA);
        assert_eq!(read_bus_word(&mut bus, 0x0002_0002), 0xBBBB);
        assert_eq!(read_bus_word(&mut bus, 0x0002_0004), 0xCCCC);
    }

    #[test]
    fn int18h_font_pattern_read_writes_paged_output_buffer() {
        let mut bus = test_bus();
        setup_hle_page_tables(&mut bus);
        let mut cpu = TestCpu::default();
        cpu.set_ax(0x1400);
        cpu.set_bx(0x2000);
        cpu.set_cx(0x0000);
        cpu.set_dx(0x8000);
        bus.memory.font_write(0x80000, 0xA5);

        for i in 0..18u32 {
            bus.write_byte(0x0002_0000 + i, 0x55);
            bus.write_byte(0x0003_0000 + i, 0x77);
        }

        bus.hle_int18h(&mut cpu);

        assert_eq!(read_bus_word(&mut bus, 0x0003_0000), 0x0102);
        assert_eq!(bus.read_byte(0x0003_0002), 0xA5);
        assert_eq!(bus.read_byte(0x0002_0000), 0x55);
        assert_eq!(bus.read_byte(0x0002_0001), 0x55);
        assert_eq!(bus.read_byte(0x0002_0002), 0x55);
    }

    #[test]
    fn int18h_multi_display_area_reads_paged_entry_table() {
        let mut bus = test_bus();
        setup_hle_page_tables(&mut bus);
        let mut cpu = TestCpu::default();
        cpu.set_ax(0x0F00);
        cpu.set_bx(0x2000);
        cpu.set_cx(0x0000);
        cpu.set_dx(0x0001);

        // Stale data at the linear address so we know the read came from the
        // physical mapping.
        write_bus_word(&mut bus, 0x0002_0000, 0xAAAA);
        write_bus_word(&mut bus, 0x0002_0002, 0xBBBB);

        // Real table at the physical address.
        write_bus_word(&mut bus, 0x0003_0000, 0x0080);
        write_bus_word(&mut bus, 0x0003_0002, 0x0001);

        bus.hle_int18h(&mut cpu);

        assert_eq!(bus.gdc_master.state.scroll[0].start_address, 0x0040);
        assert_ne!(bus.gdc_master.state.scroll[0].line_count, 0);
    }

    #[test]
    fn int19h_rx_count_reads_paged_buf_base() {
        let mut bus = test_bus();
        setup_hle_page_tables(&mut bus);
        let mut cpu = TestCpu::default();

        // BDA pointer at 0x0556/0x0558 points at linear 0x0002_0000.
        bus.ram_write_u16(0x0556, 0x0000);
        bus.ram_write_u16(0x0558, 0x2000);

        // Mark the buffer as initialized at the physical address (R_FLAG bit 7).
        bus.write_byte(0x0003_0002, 0x80);

        // Stale R_CNT at linear, real R_CNT at physical.
        write_bus_word(&mut bus, 0x0002_000E, 0xAAAA);
        write_bus_word(&mut bus, 0x0003_000E, 0x1234);

        // Mark linear as supervisor flag too so the bug-only path would see 0.
        bus.write_byte(0x0002_0002, 0x00);

        cpu.set_ax(0x0200);
        bus.hle_int19h(&mut cpu);

        assert_eq!(cpu.cx(), 0x1234);
        assert_eq!(cpu.ah(), 0x00);
    }

    #[test]
    fn int0ch_serial_irq_writes_paged_buf_base() {
        let mut bus = test_bus();
        setup_hle_page_tables(&mut bus);
        let mut cpu = TestCpu::default();

        // BDA pointer at 0x0556/0x0558 points at linear 0x0002_0000.
        bus.ram_write_u16(0x0556, 0x0000);
        bus.ram_write_u16(0x0558, 0x2000);

        // Buffer ring config at the physical address. R_HEADP=R_PUTP=0x0014,
        // R_TAILP=0x0024, R_GETP=0x0014, R_CNT=0, R_FLAG=0 (BFULL clear).
        write_bus_word(&mut bus, 0x0003_000A, 0x0014); // R_HEADP
        write_bus_word(&mut bus, 0x0003_000C, 0x0024); // R_TAILP
        write_bus_word(&mut bus, 0x0003_000E, 0x0000); // R_CNT
        write_bus_word(&mut bus, 0x0003_0010, 0x0014); // R_PUTP
        write_bus_word(&mut bus, 0x0003_0012, 0x0014); // R_GETP
        bus.write_byte(0x0003_0002, 0x00); // R_FLAG (no overflow)

        // Stale conflicting data at linear: if the handler reads/writes
        // through the linear path the test will see different values.
        write_bus_word(&mut bus, 0x0002_000E, 0xAAAA);
        bus.write_byte(0x0002_0002, 0xFF);

        bus.hle_int0ch(&mut cpu);

        // R_INT (offset 0x00) should be set with bit 7.
        let r_int = bus.read_byte(0x0003_0000);
        assert_ne!(r_int & 0x80, 0);
        // R_CNT incremented to 1 at the physical address.
        assert_eq!(read_bus_word(&mut bus, 0x0003_000E), 0x0001);
        // Linear address must be untouched.
        assert_eq!(read_bus_word(&mut bus, 0x0002_000E), 0xAAAA);
        assert_eq!(bus.read_byte(0x0002_0002), 0xFF);
    }
}
