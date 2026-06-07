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
            0x10 => self.int2fh_10h_share(cpu),
            0x11 => self.int2fh_11h_network_redirector(cpu, memory),
            0x12 => self.int2fh_12h_dos_internal(cpu, memory),
            0x15 => self.int2fh_15h_mscdex(cpu, memory, cdrom),
            0x16 => self.int2fh_16h_windows_check(cpu, memory),
            0x43 => self.int2fh_43h_xms_check(cpu),
            0x46 => self.int2fh_46h_windows_mcb(cpu),
            0x48 => self.int2fh_48h_doskey_check(cpu),
            0x4A => self.int2fh_4ah_hma_query(cpu),
            0x4D => self.int2fh_4dh_kkcfunc(),
            0x4F => set_iret_carry(cpu, memory, true), // Keyboard intercept: no translation.
            0xD2 => self.int2fh_d2h_quarterdeck_rpci(cpu),
            0xFE => self.int2fh_feh_user_multiplex(),
            _ => warn!("INT 2Fh AH={ah:#04X} is unimplemented"),
        }
    }

    /// AH=10h: SHARE.EXE interface. Installation check reports not present.
    fn int2fh_10h_share(&self, cpu: &mut dyn CpuAccess) {
        let al = cpu.ax() as u8;
        if al == 0x00 {
            cpu.set_ax(cpu.ax() & 0xFF00);
        }
    }

    /// AH=11h: DOS network redirector interface.
    fn int2fh_11h_network_redirector(
        &self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
    ) {
        let al = cpu.ax() as u8;
        match al {
            0x00 => cpu.set_ax(cpu.ax() & 0xFF00),
            _ => {
                cpu.set_ax(0x0001);
                set_iret_carry(cpu, memory, true);
            }
        }
    }

    /// AH=12h: DOS-internal services.
    fn int2fh_12h_dos_internal(&self, cpu: &mut dyn CpuAccess, memory: &dyn MemoryAccess) {
        let al = cpu.ax() as u8;
        match al {
            0x03 => cpu.set_ds(tables::DOS_DATA_SEGMENT),
            0x13 => {
                // Real DOS 6.20 reads the caller's pushed stack word into AX
                // (so AH = high byte of the original PUSH), then uppercases
                // the low byte. AX from the INT 2Fh call (1213h) is clobbered.
                let caller_stack_addr = cpu
                    .linear_address(SegmentRegister::SS, cpu.sp())
                    .wrapping_add(6);
                let stack_word = memory.read_word(caller_stack_addr);
                let character = stack_word as u8;
                let uppercase = if character.is_ascii_lowercase() {
                    character.to_ascii_uppercase()
                } else {
                    character
                };
                cpu.set_ax((stack_word & 0xFF00) | uppercase as u16);
            }
            0x2E => self.int2fh_122eh_error_tables(cpu),
            _ => {
                warn!("INT 2Fh AX=12{al:02X}h is unimplemented");
            }
        }
    }

    /// AX=122Eh: Get DOS 5+ error table addresses or the message retriever.
    fn int2fh_122eh_error_tables(&self, cpu: &mut dyn CpuAccess) {
        let dl = cpu.dx() as u8;
        match dl {
            0x00 => self.set_error_table_result(cpu, tables::ERROR_TABLE_STANDARD_TOKEN),
            0x02 => self.set_error_table_result(cpu, tables::ERROR_TABLE_PARAMETER_TOKEN),
            0x04 => self.set_error_table_result(cpu, tables::ERROR_TABLE_CRITICAL_TOKEN),
            0x06 => {
                // Real DOS 6.20 has no parse error table here; return NULL.
                cpu.set_es(0);
                cpu.set_di(0);
            }
            0x08 => {
                cpu.set_es(tables::DOS_DATA_SEGMENT);
                cpu.set_di(tables::ERROR_RETRIEVER_STUB_OFFSET);
            }
            // DL=01h/03h/05h/07h/09h set the tables; DOS 5+ ignores them.
            0x01 | 0x03 | 0x05 | 0x07 | 0x09 => {}
            _ => {
                warn!("INT 2Fh AX=122Eh DL={dl:#04X} is unimplemented");
            }
        }
    }

    fn set_error_table_result(&self, cpu: &mut dyn CpuAccess, table_token: u16) {
        cpu.set_es(tables::ERROR_TABLE_SEGMENT);
        cpu.set_di(table_token);
    }

    /// Retriever callback fired through INT FDh from the stub at
    /// `tables::ERROR_RETRIEVER_STUB_ADDR`. The caller passes the table token
    /// in DI and the error number in AX; we write a counted message to a
    /// scratch buffer and return ES:DI pointing at the count byte.
    pub(crate) fn int2fh_122eh_retrieve_error_message(
        &self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
    ) {
        let message = error_table_message(cpu.di(), cpu.ax());
        let len = message.len().min(tables::ERROR_MESSAGE_BUFFER_SIZE - 1);
        memory.write_byte(tables::ERROR_MESSAGE_BUFFER_ADDR, len as u8);
        memory.write_block(tables::ERROR_MESSAGE_BUFFER_ADDR + 1, &message[..len]);
        if len + 1 < tables::ERROR_MESSAGE_BUFFER_SIZE {
            memory.write_byte(tables::ERROR_MESSAGE_BUFFER_ADDR + 1 + len as u32, 0);
        }
        cpu.set_es(tables::DOS_DATA_SEGMENT);
        cpu.set_di(tables::ERROR_MESSAGE_BUFFER_OFFSET);
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
                    memory.write_word(buffer_addr + 1, 0);
                    memory.write_word(buffer_addr + 3, tables::CDROM_MIRROR_HEADER_SEGMENT);
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

    /// AH=16h: Windows enhanced mode broadcasts and DOSMGR API.
    ///
    /// AL=00h ("Get Enhanced Mode version") returns AL=0 to signal Windows is
    /// not running. AL=07h with BX=15h is the DOSMGR virtual device API used
    /// during Windows enhanced-mode init. All other broadcasts (160Ah/0Bh/0Ch
    /// and friends) must leave the caller's registers and CF untouched, just
    /// like real DOS 6.20 with no Windows resident.
    fn int2fh_16h_windows_check(&mut self, cpu: &mut dyn CpuAccess, memory: &mut dyn MemoryAccess) {
        let al = cpu.ax() as u8;
        match al {
            0x00 => cpu.set_ax(cpu.ax() & 0xFF00),
            0x07 if cpu.bx() == 0x0015 => self.int2fh_1607h_dosmgr_api(cpu, memory),
            _ => {}
        }
    }

    /// AX=1607h BX=15h: DOSMGR virtual device API.
    ///
    /// Each subfunction is selected by CX. Responses are modeled on real DOS
    /// 5+/6.20 kernels: CX=0 returns the patch table, CX=1 acknowledges the
    /// device-specific flags, CX=2/3/4/5 implement the optional services
    /// DOSMGR uses to probe for HMA, instance data, and kernel device
    /// drivers.
    fn int2fh_1607h_dosmgr_api(&self, cpu: &mut dyn CpuAccess, memory: &mut dyn MemoryAccess) {
        match cpu.cx() {
            0x0000 if cpu.dx() == 0x0000 => {
                let (segment, offset) =
                    tables::dos_data_far(tables::WINDOWS_DOSMGR_PATCH_TABLE_OFFSET);
                cpu.set_cx(0x0001);
                cpu.set_dx(0x0000);
                cpu.set_es(segment);
                cpu.set_bx(offset);
                set_iret_carry(cpu, memory, false);
            }
            0x0001 => {
                let value = cpu.dx();
                cpu.set_ax(0xB97C);
                cpu.set_bx(value);
                cpu.set_cx(0x0000);
                cpu.set_dx(0xA2AB);
                set_iret_carry(cpu, memory, false);
            }
            0x0002 => {
                cpu.set_cx(0x0000);
                set_iret_carry(cpu, memory, false);
            }
            0x0003 => {
                if cpu.dx() == 0x0001 {
                    cpu.set_ax(0xB97C);
                    cpu.set_cx(0x0058);
                    cpu.set_dx(0xA2AB);
                }
                set_iret_carry(cpu, memory, false);
            }
            0x0004 => {
                cpu.set_cx(0x0000);
                cpu.set_dx(0x0000);
                set_iret_carry(cpu, memory, false);
            }
            0x0005 => {
                // Device driver size probe. Inputs: ES:DI points at a device
                // header inside the DOS data segment. We report the HLE
                // device-driver region by responding only when ES:DI is in
                // our reserved area below FIRST_MCB_OFFSET.
                if cpu.es() == tables::DOS_DATA_SEGMENT && cpu.di() < tables::FIRST_MCB_OFFSET {
                    cpu.set_ax(0xB97C);
                    cpu.set_bx(0x0000);
                    cpu.set_cx(tables::FIRST_MCB_OFFSET);
                    cpu.set_dx(0xA2AB);
                } else {
                    cpu.set_ax(0x0000);
                    cpu.set_bx(0x0000);
                    cpu.set_cx(0x0000);
                    cpu.set_dx(0x0000);
                }
                set_iret_carry(cpu, memory, false);
            }
            _ => {}
        }
    }

    /// AH=46h: DOS 5+/Windows MCB save/restore compatibility hooks. Real DOS
    /// 6.20 itself does NOT mutate any MCB state for AL=01h/02h or the other
    /// subfunctions; oracle reads confirm the trashed bytes survive a 4602h
    /// "restore". Windows installs its own MCB management when it runs, so on
    /// stock DOS these calls only need to preserve the caller's registers and
    /// CF.
    fn int2fh_46h_windows_mcb(&self, cpu: &dyn CpuAccess) {
        let al = cpu.ax() as u8;
        if !matches!(al, 0x01 | 0x02 | 0x03 | 0x04 | 0x80) {
            warn!("INT 2Fh AX={:#06X}h is unimplemented", cpu.ax());
        }
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

    /// AH=4Dh: KKCFUNC.SYS API.
    fn int2fh_4dh_kkcfunc(&self) {}

    /// AH=FEh: unclaimed user multiplex slot.
    fn int2fh_feh_user_multiplex(&self) {}

    /// AH=D2h: Quarterdeck RPCI/PCL-838 multiplex slot.
    fn int2fh_d2h_quarterdeck_rpci(&self, cpu: &mut dyn CpuAccess) {
        if cpu.ax() as u8 == 0x00 {
            cpu.set_ax(cpu.ax() & 0xFF00);
        }
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

fn error_table_message(table_token: u16, error_number: u16) -> &'static [u8] {
    match table_token {
        tables::ERROR_TABLE_PARAMETER_TOKEN => parameter_error_message(error_number),
        tables::ERROR_TABLE_CRITICAL_TOKEN => critical_error_message(error_number),
        _ => standard_error_message(error_number),
    }
}

fn standard_error_message(error_number: u16) -> &'static [u8] {
    match error_number {
        0x00 => b"No error",
        0x01 => b"Invalid function number",
        0x02 => b"File not found",
        0x03 => b"Path not found",
        0x04 => b"Too many open files",
        0x05 => b"Access denied",
        0x06 => b"Invalid handle",
        0x07 => b"Memory control blocks destroyed",
        0x08 => b"Insufficient memory",
        0x09 => b"Invalid memory block address",
        0x0A => b"Invalid environment",
        0x0B => b"Invalid format",
        0x0C => b"Invalid access code",
        0x0D => b"Invalid data",
        0x0F => b"Invalid drive",
        0x10 => b"Cannot remove current directory",
        0x11 => b"Not same device",
        0x12 => b"No more files",
        0x13 => b"Write protect error",
        0x14 => b"Unknown unit",
        0x15 => b"Drive not ready",
        0x16 => b"Unknown command",
        0x17 => b"Data error",
        0x18 => b"Bad request structure length",
        0x19 => b"Seek error",
        0x1A => b"Unknown media type",
        0x1B => b"Sector not found",
        0x1C => b"Printer out of paper",
        0x1D => b"Write fault",
        0x1E => b"Read fault",
        0x1F => b"General failure",
        0x20 => b"Sharing violation",
        0x21 => b"Lock violation",
        0x22 => b"Invalid disk change",
        0x23 => b"FCB unavailable",
        0x24 => b"Sharing buffer overflow",
        0x25 => b"Code page mismatch",
        0x26 => b"Cannot complete file operation",
        0x4F => b"Reserved error",
        0x51 => b"Duplicate FCB",
        0x52 => b"Cannot make directory",
        0x53 => b"Fail on INT 24",
        0x54 => b"Too many redirections",
        0x55 => b"Duplicate redirection",
        0x56 => b"Invalid password",
        0x57 => b"Invalid parameter",
        0x58 => b"Network write fault",
        0x59 => b"Function not supported on network",
        _ => b"Unknown error",
    }
}

fn parameter_error_message(error_number: u16) -> &'static [u8] {
    match error_number {
        0x01 => b"Too many parameters",
        0x02 => b"Required parameter missing",
        0x03 => b"Invalid switch",
        0x04 => b"Invalid keyword",
        0x06 => b"Parameter value not in allowed range",
        0x07 | 0x08 => b"Parameter value not allowed",
        0x09 => b"Parameter format not correct",
        0x0A => b"Invalid parameter",
        0x0B => b"Invalid parameter combination",
        _ => b"Parameter error",
    }
}

fn critical_error_message(error_number: u16) -> &'static [u8] {
    match error_number {
        0x13 => b"Write protect error",
        0x14 => b"Unknown unit",
        0x15 => b"Drive not ready",
        0x16 => b"Unknown command",
        0x17 => b"Data error",
        0x18 => b"Bad request structure length",
        0x19 => b"Seek error",
        0x1A => b"Unknown media type",
        0x1B => b"Sector not found",
        0x1C => b"Printer out of paper",
        0x1D => b"Write fault",
        0x1E => b"Read fault",
        0x1F => b"General failure",
        0x20 => b"Sharing violation",
        0x21 => b"Lock violation",
        0x22 => b"Invalid disk change",
        0x23 => b"FCB unavailable",
        0x24 => b"Sharing buffer overflow",
        0x25 => b"Code page mismatch",
        0x26 => b"Cannot complete file operation",
        _ => b"Critical error",
    }
}
