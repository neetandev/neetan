//! INT 1Bh disk service (FDD + SASI HDD + IDE HDD).

use common::{Cpu, MachineModel, SegmentRegister};

use super::super::Pc9801Bus;
use crate::TraceSink;

impl<T: TraceSink> Pc9801Bus<T> {
    pub(super) fn hle_int1bh(&mut self, cpu: &mut impl Cpu) {
        let function = cpu.ah() & 0x0F;
        let da = cpu.al();
        let devtype = da & 0xF0;

        match devtype {
            // SASI/IDE: DA high nibble 0x80 or 0x00.
            0x80 | 0x00 => self.int1bh_hdd(cpu, function),
            // FDD: all known DA high nibbles for 1MB, 640KB, and 320KB FDCs.
            0x90 | 0x10 | 0x30 | 0xB0 | 0x70 | 0xF0 | 0x50 => self.int1bh_fdd(cpu, function),
            _ => self.write_result_ah_cf(cpu, 0x40),
        }
    }

    fn int1bh_fdd(&mut self, cpu: &mut impl Cpu, function: u8) {
        let result_ah = self.int1bh_fdd_dispatch(cpu, function);
        self.write_result_ah_cf(cpu, result_ah);
    }

    fn int1bh_fdd_dispatch(&mut self, cpu: &mut impl Cpu, function: u8) -> u8 {
        let drive = (cpu.al() & 0x03) as usize;
        let ah = cpu.ah();

        let devtype = cpu.al() & 0xF0;
        if self.machine_model == MachineModel::PC9801F && !matches!(devtype, 0x50 | 0x70 | 0x90) {
            return 0x40;
        }

        match function {
            0x00 => {
                if ah & 0x10 != 0 {
                    self.fdd_seek_cylinder[drive] = cpu.cl();
                }
                0x00
            }
            0x03 => {
                // Initialize FDD: update DISK_EQUIP.
                let da = cpu.al();
                let devtype = da & 0xF0;
                let is_1mb = matches!(devtype, 0x90 | 0x30 | 0xB0 | 0x10);
                let equip = if is_1mb {
                    u16::from(self.floppy.fdc_1mb().state.drive_equipped & 0x0F)
                } else {
                    u16::from(self.floppy.fdc_640k().state.drive_equipped & 0x0F)
                };
                let mut disk_equip = self.ram_read_u16(0x055C);
                if is_1mb {
                    disk_equip = (disk_equip & 0xFFF0) | equip;
                } else {
                    disk_equip = (disk_equip & 0x0FFF) | (equip << 12);
                }
                self.ram_write_u16(0x055C, disk_equip);

                // Match SASI/IDE init: unmask master IRQ 0 (system timer).
                self.pic.state.chips[0].imr &= !0x01;
                self.pic.invalidate_irq_cache();

                0x00
            }
            0x04 => {
                // Sense: return drive status.
                if !self.floppy.has_drive(drive) {
                    0x60
                } else {
                    let mut result = 0x00u8;
                    if self.floppy.is_write_protected(drive) {
                        result |= 0x10;
                    }
                    // Bit 0 = disk present.
                    result |= 0x01;
                    // Report dual-mode drive (1MB/640KB) for extended sense.
                    if (cpu.ax() & 0x8F40) == 0x8400 {
                        result |= 0x08;
                    }
                    result
                }
            }
            0x05 => {
                // Write sectors.
                let c = cpu.cl();
                let h = cpu.dh();
                let r = cpu.dl();
                let n_val = cpu.ch();
                let sector_size = 128usize << n_val;
                let buf_off = cpu.bp();
                let buf_addr =
                    self.hle_linear_address(cpu, SegmentRegister::ES, u32::from(buf_off));
                let transfer_bytes = cpu.bx() as usize;
                let size = if transfer_bytes > 0 {
                    transfer_bytes
                } else {
                    sector_size
                };
                let sector_count = size / sector_size;
                if !self.floppy.has_drive(drive) {
                    return 0x60;
                }
                if self.floppy.is_write_protected(drive) {
                    return 0x70;
                }
                let mut h = h;
                let mut hd = (h ^ (cpu.al() >> 2)) & 1;
                // Segment wrap check.
                if Self::fdd_buffer_segment_wraps(buf_off, size) {
                    return 0x20;
                }
                if ah & 0x10 != 0 {
                    self.fdd_seek_cylinder[drive] = c;
                }
                let multi_track = ah & 0x80 != 0;
                let mut track_index = (self.fdd_seek_cylinder[drive] as usize) * 2 + hd as usize;

                let mut offset = 0u32;
                let mut current_r = r;
                for _ in 0..sector_count {
                    let mut data = vec![0u8; sector_size];
                    for (j, data_byte) in data.iter_mut().enumerate() {
                        *data_byte = self.read_mem_byte(buf_addr + offset + j as u32);
                    }
                    if !self.floppy.write_sector_data(
                        drive,
                        track_index,
                        c,
                        h,
                        current_r,
                        n_val,
                        &data,
                    ) {
                        if multi_track && hd == 0 {
                            hd = 1;
                            h = 1;
                            track_index = (self.fdd_seek_cylinder[drive] as usize) * 2 + 1;
                            current_r = 1;
                            if !self.floppy.write_sector_data(
                                drive,
                                track_index,
                                c,
                                h,
                                current_r,
                                n_val,
                                &data,
                            ) {
                                break;
                            }
                        } else {
                            break;
                        }
                    }
                    offset += sector_size as u32;
                    current_r += 1;
                }
                // If no sector was written at all, the starting sector
                // didn't exist - return 0xE0. Otherwise the FDC reached
                // EOT and reports success for whatever was transferred.
                if offset == 0 { 0xE0 } else { 0x00 }
            }
            0x02 | 0x06 => {
                // Read sectors (0x06 = normal read, 0x02 = diagnostic read).
                if !self.floppy.has_drive(drive) {
                    return 0x60;
                }
                let is_diagnostic = function == 0x02;
                let c = cpu.cl();
                let mut h = cpu.dh();
                let mut hd = (h ^ (cpu.al() >> 2)) & 1;
                let r = cpu.dl();
                let n_val = cpu.ch();
                let sector_size = 128usize << n_val;
                let buf_off = cpu.bp();
                let buf_addr =
                    self.hle_linear_address(cpu, SegmentRegister::ES, u32::from(buf_off));
                let transfer_bytes = cpu.bx() as usize;
                let size = if transfer_bytes > 0 {
                    transfer_bytes
                } else {
                    sector_size
                };
                // Segment wrap check.
                if Self::fdd_buffer_segment_wraps(buf_off, size) {
                    return if is_diagnostic { 0x00 } else { 0x20 };
                }
                if ah & 0x10 != 0 {
                    self.fdd_seek_cylinder[drive] = c;
                }
                let sector_count = size / sector_size;
                let multi_track = ah & 0x80 != 0;
                let mut track_index = (self.fdd_seek_cylinder[drive] as usize) * 2 + hd as usize;

                let mut offset = 0u32;
                let mut current_r = r;
                for _ in 0..sector_count {
                    if let Some(data) = self
                        .floppy
                        .read_sector_data(drive, track_index, c, h, current_r, n_val)
                        .map(<[u8]>::to_vec)
                    {
                        if !is_diagnostic {
                            for (j, &byte) in data.iter().enumerate() {
                                self.write_mem_byte(buf_addr + offset + j as u32, byte);
                            }
                        }
                        offset += data.len() as u32;
                    } else if multi_track && hd == 0 {
                        // Head switch: try head 1 on same cylinder.
                        hd = 1;
                        h = 1;
                        track_index = (self.fdd_seek_cylinder[drive] as usize) * 2 + 1;
                        current_r = 1;
                        if let Some(data) = self
                            .floppy
                            .read_sector_data(drive, track_index, c, h, current_r, n_val)
                            .map(<[u8]>::to_vec)
                        {
                            if !is_diagnostic {
                                for (j, &byte) in data.iter().enumerate() {
                                    self.write_mem_byte(buf_addr + offset + j as u32, byte);
                                }
                            }
                            offset += data.len() as u32;
                        } else {
                            // Diagnostic read returns 0x00 on error.
                            return if is_diagnostic { 0x00 } else { 0xE0 };
                        }
                    } else {
                        return if is_diagnostic { 0x00 } else { 0xE0 };
                    }
                    current_r += 1;
                }
                0x00
            }
            0x07 => {
                // Recalibrate: no-op (always succeeds).
                0x00
            }
            0x0A => {
                // Read ID: return the next sector ID from the current track.
                if !self.floppy.has_drive(drive) {
                    0x60
                } else {
                    if ah & 0x10 != 0 {
                        self.fdd_seek_cylinder[drive] = cpu.cl();
                    }
                    let h = (cpu.dh() ^ (cpu.al() >> 2)) & 1;
                    let track_index = self.fdd_seek_cylinder[drive] as usize * 2 + h as usize;
                    if let Some(disk) = self.floppy.drive(drive)
                        && disk.sector_count(track_index) > 0
                    {
                        let sector_count = disk.sector_count(track_index);
                        let sector_index = self.fdd_read_id_index[drive] % sector_count;
                        let sector = disk
                            .sector_at_index(track_index, sector_index)
                            .expect("sector index must be valid");
                        self.fdd_read_id_index[drive] =
                            (self.fdd_read_id_index[drive] + 1) % sector_count;
                        cpu.set_cl(sector.cylinder);
                        cpu.set_dh(sector.head);
                        cpu.set_dl(sector.record);
                        cpu.set_ch(sector.size_code);
                        0x00
                    } else {
                        0xE0
                    }
                }
            }
            0x01 => {
                // Verify: no-op.
                if !self.floppy.has_drive(drive) {
                    return 0x60;
                }
                0x00
            }
            0x0D => {
                // Format track.
                if !self.floppy.has_drive(drive) {
                    return 0x60;
                }
                if self.floppy.is_write_protected(drive) {
                    return 0x70;
                }
                if ah & 0x10 != 0 {
                    self.fdd_seek_cylinder[drive] = cpu.cl();
                }
                let h = cpu.dh();
                let hd = (h ^ (cpu.al() >> 2)) & 1;
                let n_val = cpu.ch();
                let fill_byte = cpu.dl();
                let buf_off = cpu.bp();
                let buf_addr =
                    self.hle_linear_address(cpu, SegmentRegister::ES, u32::from(buf_off));
                let buf_size = cpu.bx() as usize;
                let sector_count = buf_size / 4;
                let track_index = (self.fdd_seek_cylinder[drive] as usize) * 2 + hd as usize;

                let mut chrn = Vec::with_capacity(sector_count);
                for i in 0..sector_count {
                    let base = buf_addr + (i as u32) * 4;
                    let c = self.read_mem_byte(base);
                    let h = self.read_mem_byte(base + 1);
                    let r = self.read_mem_byte(base + 2);
                    let n = self.read_mem_byte(base + 3);
                    chrn.push((c, h, r, n));
                }
                self.floppy
                    .format_track(drive, track_index, &chrn, n_val, fill_byte);
                0x00
            }
            0x0E => {
                // Set density.
                0x00
            }
            _ => 0x40,
        }
    }

    fn fdd_buffer_segment_wraps(buffer_offset: u16, size: usize) -> bool {
        if size == 0 {
            return false;
        }
        let start = u32::from(buffer_offset);
        let end = start.wrapping_add(size as u32 - 1);
        (start & 0xFFFF) > (end & 0xFFFF)
    }

    fn int1bh_hdd(&mut self, cpu: &mut impl Cpu, function: u8) {
        if self.machine_model.has_ide() {
            self.int1bh_ide(cpu, function);
        } else {
            self.int1bh_sasi(cpu, function);
        }
    }

    fn int1bh_sasi(&mut self, cpu: &mut impl Cpu, function: u8) {
        let ax = cpu.ax();
        let bx = cpu.bx();
        let cx = cpu.cx();
        let dx = cpu.dx();
        let bp = cpu.bp();

        let function_code = (ax >> 8) as u8;
        let drive_select = ax as u8;
        let drive_idx = device::sasi::drive_index(drive_select);

        let result_ah = match function {
            0x03 => {
                let current_lo = self.read_byte_direct(0x055C);
                let current_hi = self.read_byte_direct(0x055D);
                let current_equip = u16::from(current_lo) | (u16::from(current_hi) << 8);
                let disk_equip = self.sasi.execute_init(current_equip);
                self.memory.write_byte(0x055C, disk_equip as u8);
                self.memory.write_byte(0x055D, (disk_equip >> 8) as u8);
                self.pic.state.chips[0].imr &= !0x01;
                self.pic.invalidate_irq_cache();
                0x00
            }
            0x04 => match function_code {
                0x84 => {
                    let sense_result = self.sasi.execute_sense(drive_idx);
                    if sense_result >= 0x20 {
                        sense_result
                    } else {
                        if let Some(geometry) = self.sasi.drive_geometry(drive_idx) {
                            cpu.set_bx(geometry.sector_size);
                            cpu.set_cx(geometry.cylinders.saturating_sub(1));
                            let dx_value = ((u16::from(geometry.heads)) << 8)
                                | u16::from(geometry.sectors_per_track);
                            cpu.set_dx(dx_value);
                        }
                        sense_result
                    }
                }
                _ => self.sasi.execute_sense(drive_idx),
            },
            0x05 => {
                let xfer = device::sasi::transfer_size(bx);
                let geometry = self.sasi.drive_geometry(drive_idx);
                let pos = geometry
                    .map(|g| device::sasi::sector_position(drive_select, cx, dx, &g))
                    .unwrap_or(0);
                let addr = self.hle_linear_address(cpu, SegmentRegister::ES, u32::from(bp));
                self.hle_sasi_write_from_memory(drive_idx, xfer, pos, addr)
            }
            0x06 => {
                let xfer = device::sasi::transfer_size(bx);
                let geometry = self.sasi.drive_geometry(drive_idx);
                let pos = geometry
                    .map(|g| device::sasi::sector_position(drive_select, cx, dx, &g))
                    .unwrap_or(0);
                let addr = self.hle_linear_address(cpu, SegmentRegister::ES, u32::from(bp));
                self.hle_sasi_read_to_memory(drive_idx, xfer, pos, addr)
            }
            0x07 | 0x0F => 0x00,
            0x0D => {
                let geometry = self.sasi.drive_geometry(drive_idx);
                let pos = geometry
                    .map(|g| device::sasi::sector_position(drive_select, cx, dx, &g))
                    .unwrap_or(0);
                self.sasi.execute_format(drive_idx, pos)
            }
            0x0E => {
                let mode_set_result = self.sasi.execute_mode_set(drive_idx);
                if mode_set_result == 0x00 {
                    self.apply_sasi_mode_set(drive_idx, function_code);
                }
                mode_set_result
            }
            0x01 => 0x00,
            _ => 0x40,
        };

        self.write_result_ah_cf(cpu, result_ah);
    }

    fn int1bh_ide(&mut self, cpu: &mut impl Cpu, function: u8) {
        let ax = cpu.ax();
        let bx = cpu.bx();
        let cx = cpu.cx();
        let dx = cpu.dx();
        let bp = cpu.bp();

        let function_code = (ax >> 8) as u8;
        let drive_select = ax as u8;
        let drive_idx = device::ide::drive_index(drive_select);
        // In compatibility mode the CD-ROM is always at unit 1 (DA/UA=0x81).
        // If unit 1 has no HDD but a CD-ROM is present, treat it as the CD-ROM unit.
        let is_cdrom_unit = drive_idx == 1 && self.ide.has_cdrom() && !self.ide.has_hdd(1);

        // IDE-specific extensions use the full unmasked function code.
        let result_ah = match function_code {
            0xD0 => self.ide.execute_check_power_mode(drive_idx),
            0xE0 => self.ide.execute_motor_on(drive_idx),
            0xF0 => self.ide.execute_motor_off(drive_idx),
            _ => match function {
                0x03 => {
                    let current_lo = self.read_byte_direct(0x055C);
                    let current_hi = self.read_byte_direct(0x055D);
                    let current_equip = u16::from(current_lo) | (u16::from(current_hi) << 8);
                    let disk_equip = self.ide.execute_init(current_equip);
                    self.memory.write_byte(0x055C, disk_equip as u8);
                    self.memory.write_byte(0x055D, (disk_equip >> 8) as u8);
                    self.pic.state.chips[0].imr &= !0x01;
                    self.pic.invalidate_irq_cache();
                    0x00
                }
                0x04 => match function_code {
                    0x84 => {
                        if is_cdrom_unit {
                            self.ide.execute_cdrom_sense()
                        } else {
                            let sense_result = self.ide.execute_sense(drive_idx);
                            if sense_result >= 0x20 {
                                sense_result
                            } else {
                                if let Some(geometry) = self.ide.drive_geometry(drive_idx) {
                                    cpu.set_bx(geometry.sector_size);
                                    cpu.set_cx(geometry.cylinders.saturating_sub(1));
                                    let dx_value = ((u16::from(geometry.heads)) << 8)
                                        | u16::from(geometry.sectors_per_track);
                                    cpu.set_dx(dx_value);
                                }
                                sense_result
                            }
                        }
                    }
                    _ => {
                        if is_cdrom_unit {
                            self.ide.execute_cdrom_sense()
                        } else {
                            self.ide.execute_sense(drive_idx)
                        }
                    }
                },
                0x05 => {
                    let xfer = device::ide::transfer_size(bx);
                    let geometry = self.ide.drive_geometry(drive_idx);
                    let pos = geometry
                        .map(|g| device::ide::sector_position(drive_select, cx, dx, &g))
                        .unwrap_or(0);
                    let addr = self.hle_linear_address(cpu, SegmentRegister::ES, u32::from(bp));
                    self.hle_ide_write_from_memory(drive_idx, xfer, pos, addr)
                }
                0x06 => {
                    let xfer = device::ide::transfer_size(bx);
                    let geometry = self.ide.drive_geometry(drive_idx);
                    let pos = geometry
                        .map(|g| device::ide::sector_position(drive_select, cx, dx, &g))
                        .unwrap_or(0);
                    let addr = self.hle_linear_address(cpu, SegmentRegister::ES, u32::from(bp));
                    self.hle_ide_read_to_memory(drive_idx, xfer, pos, addr)
                }
                0x07 | 0x0F => 0x00,
                0x0D => {
                    let geometry = self.ide.drive_geometry(drive_idx);
                    let pos = geometry
                        .map(|g| device::ide::sector_position(drive_select, cx, dx, &g))
                        .unwrap_or(0);
                    self.ide.execute_format(drive_idx, pos)
                }
                0x0E => self.ide.execute_mode_set(drive_idx),
                0x01 => 0x00,
                _ => 0x40,
            },
        };

        self.write_result_ah_cf(cpu, result_ah);
    }
}
