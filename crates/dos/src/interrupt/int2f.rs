//! INT 2Fh: DOS Multiplex Interrupt.
//!
//! Dispatched by AH register. Provides installation checks for resident
//! services (Windows, XMS, DOSKEY, HMA).

use common::warn;

use crate::{CdromIo, CpuAccess, MemoryAccess, NeetanDos, SegmentRegister, set_iret_carry, tables};

impl NeetanDos {
    /// Dispatches an INT 2Fh call based on the AH register.
    pub(crate) fn int2fh(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        cdrom: &mut dyn CdromIo,
    ) {
        let ah = (cpu.ax() >> 8) as u8;
        match ah {
            0x15 => self.int2fh_15h_mscdex(cpu, memory, cdrom),
            0x16 => self.int2fh_16h_windows_check(cpu),
            0x43 => self.int2fh_43h_xms_check(cpu),
            0x48 => self.int2fh_48h_doskey_check(cpu),
            0x4A => self.int2fh_4ah_hma_query(cpu),
            0x4F => set_iret_carry(cpu, memory, true), // Keyboard intercept: no translation.
            _ => warn!("INT 2Fh AH={ah:#04X} is unimplemented"),
        }
    }

    /// AH=15h: MSCDEX CD-ROM interface.
    fn int2fh_15h_mscdex(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        cdrom: &mut dyn CdromIo,
    ) {
        let al = cpu.ax() as u8;
        match al {
            0x00 => {
                // Installation check.
                if cdrom.cdrom_present() {
                    cpu.set_bx(1); // 1 CD-ROM drive.
                    cpu.set_cx(u16::from(self.state.mscdex.drive_letter));
                } else {
                    cpu.set_bx(0);
                }
            }
            0x01 => {
                // Get CD-ROM drive device list.
                if cdrom.cdrom_present() {
                    let buffer_addr = cpu.linear_address(SegmentRegister::ES, cpu.bx());
                    memory.write_byte(buffer_addr, 0); // Subunit 0.
                    memory.write_word(buffer_addr + 1, tables::DEV_CDROM_OFFSET);
                    memory.write_word(buffer_addr + 3, tables::DOS_DATA_SEGMENT);
                }
            }
            0x02 => {
                // Get copyright file name from PVD.
                self.mscdex_read_pvd_field(cpu, memory, cdrom, 702);
            }
            0x03 => {
                // Get abstract file name from PVD.
                self.mscdex_read_pvd_field(cpu, memory, cdrom, 739);
            }
            0x04 => {
                // Get bibliographic doc file name from PVD.
                self.mscdex_read_pvd_field(cpu, memory, cdrom, 776);
            }
            0x05 => {
                // Read VTOC (Volume Descriptor).
                let drive = cpu.cx() as u8;
                if drive != self.state.mscdex.drive_letter {
                    cpu.set_ax(15);
                    set_iret_carry(cpu, memory, true);
                    return;
                }
                if !cdrom.cdrom_media_loaded() {
                    cpu.set_ax(21);
                    set_iret_carry(cpu, memory, true);
                    return;
                }
                let sector_index = cpu.dx() as u32;
                let lba = 16 + sector_index;
                let buffer_addr = cpu.linear_address(SegmentRegister::ES, cpu.bx());
                let mut sector_buf = [0u8; 2048];
                match cdrom.read_sector_cooked(lba, &mut sector_buf) {
                    Some(n) => {
                        memory.write_block(buffer_addr, &sector_buf[..n]);
                        let vd_type = sector_buf[0];
                        let result = match vd_type {
                            1 => 1,
                            0xFF => 0xFF,
                            _ => 0,
                        };
                        cpu.set_ax(result);
                        set_iret_carry(cpu, memory, false);
                    }
                    None => {
                        cpu.set_ax(21);
                        set_iret_carry(cpu, memory, true);
                    }
                }
            }
            0x06 | 0x07 | 0x09 | 0x0A => {
                // Debugging on/off, absolute disk write, reserved: no-op.
            }
            0x08 => {
                // Absolute disk read.
                let drive = cpu.cx() as u8;
                if drive != self.state.mscdex.drive_letter {
                    cpu.set_ax(15);
                    set_iret_carry(cpu, memory, true);
                    return;
                }
                if !cdrom.cdrom_media_loaded() {
                    cpu.set_ax(21);
                    set_iret_carry(cpu, memory, true);
                    return;
                }
                let sector_count = cpu.dx() as u32;
                let start_lba = ((cpu.si() as u32) << 16) | cpu.di() as u32;
                let buffer_addr = cpu.linear_address(SegmentRegister::ES, cpu.bx());
                let mut sector_buf = [0u8; 2048];
                for i in 0..sector_count {
                    match cdrom.read_sector_cooked(start_lba + i, &mut sector_buf) {
                        Some(n) => {
                            memory.write_block(buffer_addr + i * 2048, &sector_buf[..n]);
                        }
                        None => {
                            cpu.set_ax(21);
                            set_iret_carry(cpu, memory, true);
                            return;
                        }
                    }
                }
                set_iret_carry(cpu, memory, false);
            }
            0x0B => {
                // CD-ROM drive check.
                let drive = cpu.cx() as u8;
                if cdrom.cdrom_present() && drive == self.state.mscdex.drive_letter {
                    cpu.set_ax(cpu.ax() | 0x00FF); // Non-zero AL = is CD-ROM.
                    cpu.set_bx(0xADAD);
                } else {
                    cpu.set_ax(cpu.ax() & 0xFF00); // AL=0 = not CD-ROM.
                }
            }
            0x0C => {
                // MSCDEX version: 2.10.
                cpu.set_bx(0x020A);
            }
            0x0D => {
                // Get CD-ROM drive letters.
                if cdrom.cdrom_present() {
                    let buffer_addr = cpu.linear_address(SegmentRegister::ES, cpu.bx());
                    memory.write_byte(buffer_addr, self.state.mscdex.drive_letter);
                }
            }
            0x10 => {
                // Send device driver request.
                let request_addr = cpu.linear_address(SegmentRegister::ES, cpu.bx());
                if cdrom.cdrom_present() {
                    self.handle_device_request(memory, cdrom, request_addr);
                }
            }
            _ => {
                warn!("INT 2Fh AX=15{al:02X}h is unimplemented");
            }
        }
    }

    /// AH=16h: Windows enhanced mode check.
    /// Returns AL=00h (no Windows running).
    fn int2fh_16h_windows_check(&self, cpu: &mut dyn CpuAccess) {
        cpu.set_ax(cpu.ax() & 0xFF00);
    }

    /// AH=43h: XMS driver installation check and entry point.
    fn int2fh_43h_xms_check(&self, cpu: &mut dyn CpuAccess) {
        let al = cpu.ax() as u8;
        let xms_active = self
            .state
            .memory_manager
            .as_ref()
            .is_some_and(|mm| mm.is_xms_enabled());
        match al {
            0x00 => {
                if xms_active {
                    cpu.set_ax((cpu.ax() & 0xFF00) | 0x0080);
                } else {
                    cpu.set_ax(cpu.ax() & 0xFF00);
                }
            }
            0x10 if xms_active => {
                cpu.set_es(tables::XMS_ENTRY_STUB_SEGMENT);
                cpu.set_bx(tables::XMS_ENTRY_STUB_OFFSET);
            }
            _ => {}
        }
    }

    /// AH=48h: DOSKEY installation check.
    /// Returns AL=00h (DOSKEY not installed).
    fn int2fh_48h_doskey_check(&self, cpu: &mut dyn CpuAccess) {
        cpu.set_ax(cpu.ax() & 0xFF00);
    }

    /// AH=4Ah, AL=01h: HMA (High Memory Area) query.
    fn int2fh_4ah_hma_query(&self, cpu: &mut dyn CpuAccess) {
        let hma_free = self
            .state
            .memory_manager
            .as_ref()
            .is_some_and(|mm| mm.hma_exists() && !mm.hma_is_allocated());
        if hma_free {
            cpu.set_bx(0xFFFF);
        } else {
            cpu.set_bx(0x0000);
        }
    }

    /// Reads a 37-byte identifier field from the ISO 9660 Primary Volume
    /// Descriptor and writes it (null-terminated) to the caller's buffer.
    /// Used by subfunctions 02h (copyright), 03h (abstract), 04h (bibliographic).
    fn mscdex_read_pvd_field(
        &self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        cdrom: &dyn CdromIo,
        pvd_offset: usize,
    ) {
        let drive = cpu.cx() as u8;
        if drive != self.state.mscdex.drive_letter {
            cpu.set_ax(15);
            set_iret_carry(cpu, memory, true);
            return;
        }
        if !cdrom.cdrom_media_loaded() {
            cpu.set_ax(21);
            set_iret_carry(cpu, memory, true);
            return;
        }
        let mut sector_buf = [0u8; 2048];
        if cdrom.read_sector_cooked(16, &mut sector_buf).is_none() {
            cpu.set_ax(21);
            set_iret_carry(cpu, memory, true);
            return;
        }
        let buffer_addr = cpu.linear_address(SegmentRegister::ES, cpu.bx());
        memory.write_block(buffer_addr, &sector_buf[pvd_offset..pvd_offset + 37]);
        memory.write_byte(buffer_addr + 37, 0);
        set_iret_carry(cpu, memory, false);
    }
}
