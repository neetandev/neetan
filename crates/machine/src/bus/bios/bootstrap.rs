//! Bootstrap entry point invoked from the master HLE dispatcher.

use common::Cpu;
use device::floppy::D88MediaType;

use super::{
    super::{
        BootDevice, Pc9801Bus,
        os_adapter::{OsCpuAccess, OsDiskIo, OsMemoryAccess},
    },
    boot_sector_has_signature, iret_stack_base,
};
use crate::Tracing;

impl<T: Tracing> Pc9801Bus<T> {
    fn try_boot_fdd(&mut self, cpu: &mut impl Cpu, drive: usize) -> bool {
        if !self.floppy.has_drive(drive) {
            return false;
        }
        let Some(n) = self.floppy.boot_sector_size_code(drive) else {
            return false;
        };

        let is_2hd = self
            .floppy
            .drive(drive)
            .is_some_and(|d| d.media_type == D88MediaType::Disk2HD);

        // Load the entire IPL block, not a single sector.
        let total_bytes: usize = if is_2hd { 0x400 } else { 0x200 };
        let sector_size: usize = 128usize << usize::from(n);
        let mut boot_data: Vec<u8> = Vec::with_capacity(total_bytes);
        let mut sector_record: u8 = 1;
        while boot_data.len() < total_bytes {
            let Some(sector) = self
                .floppy
                .read_sector_data(drive, 0, 0, 0, sector_record, n)
            else {
                break;
            };
            let take = (total_bytes - boot_data.len()).min(sector.len());
            boot_data.extend_from_slice(&sector[..take]);
            if take < sector.len() || sector_record == u8::MAX {
                break;
            }
            sector_record += 1;
        }

        if boot_data.len() < sector_size {
            return false;
        }

        let da_base: usize = if is_2hd { 0x90 } else { 0x70 };
        if !is_2hd {
            let equipped = self.floppy.fdc_1mb().state.drive_equipped & 0x0F;
            self.memory.state.ram[0x055C] &= !equipped;
            self.memory.state.ram[0x055D] |= equipped << 4;
            self.memory.state.ram[0x0494] = (equipped & 0x03) << 6;
        }
        self.try_boot_from_data(cpu, &boot_data, (da_base | drive) as u8)
    }

    fn try_boot_from_data(
        &mut self,
        cpu: &mut impl Cpu,
        boot_data: &[u8],
        boot_device: u8,
    ) -> bool {
        if !boot_data.iter().any(|&b| b != 0) {
            return false;
        }
        for (i, &byte) in boot_data.iter().enumerate() {
            self.memory.write_byte(0x1FC00 + i as u32, byte);
        }
        self.memory.state.ram[0x0584] = boot_device;
        let iret_base = iret_stack_base(cpu);
        self.write_mem_word(iret_base, 0x0000); // IP
        self.write_mem_word(iret_base + 2, 0x1FC0); // CS
        self.write_mem_word(iret_base + 4, 0x0002); // FLAGS (reserved bit 1 set)
        true
    }

    pub(super) fn hle_bootstrap(&mut self, cpu: &mut impl Cpu) {
        // The HLE dispatch uses IRET to transfer control to the boot sector.
        // We rewrite the IRET frame (6 bytes: IP, CS, FLAGS) at the current
        // SP to redirect execution. SP is left at its current value (set to
        // 0x7C00 by the ITF entry stub). After IRET, SP becomes SP+6.
        //
        // The stack must NOT be placed near 0x0600 because boot sectors
        // commonly load program data to segment 0060:0000 (linear 0x0600),
        // which would overwrite return addresses on the stack.

        // Initialize SASI drives before boot attempt (equivalent of INT 1Bh AH=03).
        for hdd in 0..2usize {
            if self.sasi.drive_geometry(hdd).is_some() {
                let current_lo = self.memory.state.ram[0x055C];
                let current_hi = self.memory.state.ram[0x055D];
                let current_equip = u16::from(current_lo) | (u16::from(current_hi) << 8);
                let disk_equip = self.sasi.execute_init(current_equip);
                self.memory.state.ram[0x055C] = disk_equip as u8;
                self.memory.state.ram[0x055D] = (disk_equip >> 8) as u8;
                break;
            }
        }

        // Initialize IDE drives before boot attempt (equivalent of INT 1Bh AH=03).
        for hdd in 0..2usize {
            if self.ide.drive_geometry(hdd).is_some() {
                let current_lo = self.memory.state.ram[0x055C];
                let current_hi = self.memory.state.ram[0x055D];
                let current_equip = u16::from(current_lo) | (u16::from(current_hi) << 8);
                let disk_equip = self.ide.execute_init(current_equip);
                self.memory.state.ram[0x055C] = disk_equip as u8;
                self.memory.state.ram[0x055D] = (disk_equip >> 8) as u8;
                break;
            }
        }

        // Set IDE device connection flags and BIOS work area.
        if self.machine_model.has_ide() {
            let hdd_flags = self.ide.compute_connection_flags();
            // 0x05BA: HDD-only connection flags (compatibility mode - no CD-ROM bit).
            self.memory.state.ram[0x05BA] = hdd_flags;
            // 0x05BB: same HDD-only backup used by the WinNT4.0 workaround.
            self.memory.state.ram[0x05BB] = hdd_flags;
            // 0x0457: IDE drive capacity (1).
            // Ref: undoc98 `memsys.txt`
            self.memory.state.ram[0x0457] = self.ide.bios_capacity_byte();
            // 0x05B0: IDE HDD capacity type ("IDE drive capacity (2)").
            // Ref: undoc98 `memsys.txt`
            self.memory.state.ram[0x05B0] = 0xFF;
            // ROM 0xF8E80+0x10 (offset 0x10E90): all connected device bits including CD-ROM.
            // Bit 2 set when CD-ROM present on channel 1 master - read by NECCD.SYS to detect the drive.
            let all_device_flags = hdd_flags | if self.ide.has_cdrom() { 0x04 } else { 0x00 };
            self.memory.set_rom_byte(0x10E90, all_device_flags);
            // ROM 0xF8E80+0x11 (offset 0x10E91): clear bit 7 for 17KB NECCDD.SYS compatibility.
            let flags_byte = self.memory.rom_byte(0x10E91);
            self.memory.set_rom_byte(0x10E91, flags_byte & !0x80);
        }

        // Install IDE expansion ROM's IRQ 9 handler and presence markers.
        // The real BIOS scans expansion ROMs and calls their init code.
        // Our ide.rom's init entry installs the IRQ handler at D800:008D
        // and sets ROM presence markers at 0x04B0/0x04B8.
        if self.ide.rom_installed() {
            // IVT[0x11] = D800:008D (IDE IRQ 9 handler in expansion ROM)
            self.memory.state.ram[0x0044] = 0x8D; // offset low
            self.memory.state.ram[0x0045] = 0x00; // offset high
            self.memory.state.ram[0x0046] = 0x00; // segment low (D800)
            self.memory.state.ram[0x0047] = 0xD8; // segment high
            // ROM presence markers (segment >> 8)
            self.memory.state.ram[0x04B0] = 0xD8;
            self.memory.state.ram[0x04B8] = 0xD8;
            // Unmask IRQ 9 (slave PIC IR1) so IDE/ATAPI interrupts can fire.
            self.pic.state.chips[1].imr &= !0x02;
            self.pic.invalidate_irq_cache();
        }

        // Boot device selection.
        match self.boot_device {
            BootDevice::Auto => {
                // FDD 0-1 -> CD-ROM (IDE only) -> SASI HDD 0-1 -> IDE HDD 0-1.
                for drive in 0..4usize {
                    if self.try_boot_fdd(cpu, drive) {
                        return;
                    }
                }
                if self.machine_model.has_ide()
                    && let Some(boot_data) = self.ide.read_cdrom_boot_sector()
                    && boot_sector_has_signature(&boot_data)
                    && self.try_boot_from_data(cpu, &boot_data, 0x82)
                {
                    let lo = self.memory.state.ram[0x055C];
                    let hi = self.memory.state.ram[0x055D];
                    let disk_equip = u16::from(lo) | (u16::from(hi) << 8) | 0x0400;
                    self.memory.state.ram[0x055C] = disk_equip as u8;
                    self.memory.state.ram[0x055D] = (disk_equip >> 8) as u8;
                    return;
                }
                for hdd in 0..2usize {
                    if let Some(data) = self.sasi.read_boot_sector(hdd)
                        && self.try_boot_from_data(cpu, &data, 0x80 | hdd as u8)
                    {
                        return;
                    }
                }
                for hdd in 0..2usize {
                    if let Some(data) = self.ide.read_boot_sector(hdd)
                        && self.try_boot_from_data(cpu, &data, 0x80 | hdd as u8)
                    {
                        return;
                    }
                }
            }
            BootDevice::Fdd1 => {
                if self.try_boot_fdd(cpu, 0) {
                    return;
                }
            }
            BootDevice::Fdd2 => {
                if self.try_boot_fdd(cpu, 1) {
                    return;
                }
            }
            BootDevice::Hdd1 => {
                if let Some(data) = self.sasi.read_boot_sector(0)
                    && self.try_boot_from_data(cpu, &data, 0x80)
                {
                    return;
                }
                if let Some(data) = self.ide.read_boot_sector(0)
                    && self.try_boot_from_data(cpu, &data, 0x80)
                {
                    return;
                }
            }
            BootDevice::Hdd2 => {
                if let Some(data) = self.sasi.read_boot_sector(1)
                    && self.try_boot_from_data(cpu, &data, 0x81)
                {
                    return;
                }
                if let Some(data) = self.ide.read_boot_sector(1)
                    && self.try_boot_from_data(cpu, &data, 0x81)
                {
                    return;
                }
            }
            BootDevice::Os => {}
        }

        // No bootable device found (or Os selected): activate NEETAN OS HLE DOS.
        let mut neetan_os = os::NeetanOs::new();
        neetan_os.set_host_local_time_fn(self.host_local_time_fn);
        neetan_os.set_ems_enabled(self.ems_enabled);
        neetan_os.set_xms_enabled(self.xms_enabled);
        neetan_os.set_xms_32_enabled(self.xms_32_enabled);
        neetan_os.set_xms_hmamin_kb(self.xms_hmamin_kb);

        {
            let mut cpu_access = OsCpuAccess(cpu);
            let access_page = self.access_page_index();
            let mut mem_access = OsMemoryAccess::new(
                &mut self.memory,
                access_page,
                self.b_bank_ems,
                self.vram_ems_bank,
            );
            let mut disk_io = OsDiskIo {
                floppy: &mut self.floppy,
                sasi: &mut self.sasi,
                ide: &mut self.ide,
            };
            neetan_os.boot(
                &mut cpu_access,
                &mut mem_access,
                &mut disk_io,
                &mut self.tracer,
            );
        }

        // Enable GDC hardware cursor for HLE OS.
        self.gdc_master.state.cursor_display = true;

        // Redirect IRET to COMMAND.COM's entry point (PSP:0100h).
        // The stub at PSP:0100h sets up its own stack inside COMMAND.COM's
        // MCB allocation before entering the shell loop, so child program
        // allocations cannot overwrite the parent's IRET frame.
        let psp_segment = neetan_os.command_com_psp();
        self.os = Some(neetan_os);
        let iret_base = iret_stack_base(cpu);
        self.write_mem_word(iret_base, 0x0100); // IP = entry point
        self.write_mem_word(iret_base + 2, psp_segment); // CS = PSP segment
        self.write_mem_word(iret_base + 4, 0x0202); // FLAGS (IF set)
    }
}
