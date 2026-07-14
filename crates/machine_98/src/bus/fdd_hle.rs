use device::floppy::{D88MediaType, FloppyImage};

use crate::{Pc9801Bus, Tracing, bus::bios};

impl<T: Tracing> Pc9801Bus<T> {
    /// Returns `true` if a 640KB FDD HLE trap is pending.
    pub fn fdd640k_hle_pending(&self) -> bool {
        self.fdd640k_hle.hle_pending()
    }

    /// Executes the pending 640KB FDD HLE operation using the extension ROM stack frame.
    ///
    /// The PC-9801-09 ROM pushes DS, SI, DI, ES, BP, DX, CX, BX, AX (9 words)
    /// before dispatching to the entry-8 INT 1Bh handler. The ROM triggers
    /// the trap via `OUT TRAP_PORT, AL`, then pops all registers and IRETs.
    /// The stack frame at SS:SP has:
    /// SP+0x00: AX, SP+0x02: BX, SP+0x04: CX, SP+0x06: DX, SP+0x08: BP,
    /// SP+0x0A: ES, SP+0x0C: DI, SP+0x0E: SI, SP+0x10: DS,
    /// SP+0x12: IP, SP+0x14: CS, SP+0x16: FLAGS.
    pub fn execute_fdd640k_hle(&mut self, ss_base: u32, sp: u16) {
        let stack_base = ss_base.wrapping_add(u32::from(sp));

        let ax = self.read_word_direct(stack_base);
        let bx = self.read_word_direct(stack_base + 0x02);
        let cx = self.read_word_direct(stack_base + 0x04);
        let dx = self.read_word_direct(stack_base + 0x06);
        let bp = self.read_word_direct(stack_base + 0x08);
        let es = self.read_word_direct(stack_base + 0x0A);

        let function_code = (ax >> 8) as u8;
        let device_select = ax as u8;
        let function = function_code & 0x0F;
        let drive = (device_select & 0x03) as usize;
        let device_type = device_select & 0xF0;

        let result_ah = if !matches!(device_type, 0x50 | 0x70 | 0x90) {
            0x40
        } else {
            self.tracer.trace_int1bh_fdd_params(
                function_code,
                device_select,
                cx as u8,
                (dx >> 8) as u8,
                dx as u8,
                (cx >> 8) as u8,
            );

            match function {
                0x00 => {
                    if function_code & 0x10 != 0 {
                        self.fdd_seek_cylinder[drive] = cx as u8;
                    }
                    0x00
                }
                0x01 => {
                    if self.floppy.has_drive(drive) {
                        0x00
                    } else {
                        0x60
                    }
                }
                0x02 | 0x06 => self.execute_fdd640k_read(
                    function_code,
                    device_select,
                    bx,
                    cx,
                    dx,
                    bp,
                    es,
                    function == 0x02,
                ),
                0x03 => self.execute_fdd640k_init(),
                0x04 => self.execute_fdd640k_sense(ax, drive),
                0x05 => {
                    self.execute_fdd640k_write(function_code, device_select, bx, cx, dx, bp, es)
                }
                0x07 => 0x00,
                0x0A => self.execute_fdd640k_read_id(stack_base, cx, dx, drive),
                0x0D => {
                    self.execute_fdd640k_format(function_code, device_select, bx, cx, dx, bp, es)
                }
                0x0E => self.execute_fdd640k_set_operation_mode(function_code, device_select),
                _ => 0x40,
            }
        };

        self.tracer
            .trace_fdd640k_hle(function_code, device_select, result_ah, bx, cx, dx, es, bp);

        // Write result AH back to stack (high byte of AX word at stack_base).
        self.memory.write_byte(stack_base + 1, result_ah);

        // Update FLAGS on the stack: set CF on error, clear on success.
        let flags_addr = stack_base + 0x16;
        let mut flags = self.read_word_direct(flags_addr);
        if result_ah >= 0x20 {
            flags |= 0x0001;
        } else {
            flags &= !0x0001;
        }
        self.memory.write_byte(flags_addr, flags as u8);
        self.memory.write_byte(flags_addr + 1, (flags >> 8) as u8);

        self.fdd640k_hle.clear_hle_pending();
    }

    fn execute_fdd640k_init(&mut self) -> u8 {
        let equipped = self.floppy.fdc_640k().state.drive_equipped & 0x0F;
        let lo = self.read_byte_with_access_page(0x055C);
        let hi = self.read_byte_with_access_page(0x055D);
        let mut disk_equip = u16::from(lo) | (u16::from(hi) << 8);
        disk_equip = (disk_equip & 0x0FFF) | (u16::from(equipped) << 12);
        self.write_byte_with_access_page(0x055C, disk_equip as u8);
        self.write_byte_with_access_page(0x055D, (disk_equip >> 8) as u8);
        self.write_byte_with_access_page(0x0494, (equipped & 0x03) << 6);
        self.write_byte_with_access_page(0x05CA, 0xFF);

        // Match SASI/IDE init: unmask master IRQ 0 (system timer).
        self.pic.state.chips[0].imr &= !0x01;
        self.pic.invalidate_irq_cache();

        0x00
    }

    fn execute_fdd640k_sense(&mut self, ax: u16, drive: usize) -> u8 {
        if !self.floppy.has_drive(drive) {
            return 0x60;
        }

        let device_type = (ax as u8) & 0xF0;
        let drive_mask = 1u8 << drive;
        let mode_address = if device_type == 0x90 { 0x0493 } else { 0x05CA };
        let mode = self.read_byte_with_access_page(mode_address);
        let side_mode = u8::from(mode & drive_mask != 0);
        let cylinder_mode = if mode & (drive_mask << 4) != 0 {
            0x04
        } else {
            0x00
        };
        let mut result = if device_type == 0x50 {
            0x02 | side_mode
        } else if device_type == 0x70 {
            side_mode | cylinder_mode
        } else {
            0x01
        };
        if self.floppy.is_write_protected(drive) {
            result |= 0x10;
        }
        if (ax & 0x8F40) == 0x8400 {
            result |= 0x08;
        }
        result
    }

    fn execute_fdd640k_set_operation_mode(&mut self, function_code: u8, device_select: u8) -> u8 {
        let device_type = device_select & 0xF0;
        let mode_address = match device_type {
            0x10 | 0x90 => 0x0493,
            0x50 | 0x70 => 0x05CA,
            _ => return 0x40,
        };

        let mut mode = self.read_byte_with_access_page(mode_address);
        if function_code & 0x80 != 0 {
            mode = (mode & 0x0F) | ((device_select & 0x0F) << 4);
        } else {
            mode = (mode & 0xF0) | (device_select & 0x0F);
        }
        self.write_byte_with_access_page(mode_address, mode);
        0x00
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_fdd640k_read(
        &mut self,
        function_code: u8,
        device_select: u8,
        bx: u16,
        cx: u16,
        dx: u16,
        bp: u16,
        es: u16,
        diagnostic: bool,
    ) -> u8 {
        let drive = (device_select & 0x03) as usize;
        let device_type = device_select & 0xF0;
        if !self.floppy.has_drive(drive) {
            return 0x60;
        }
        if !Self::fdd640k_device_matches_media(device_type, self.floppy.drive(drive)) {
            return if diagnostic { 0x00 } else { 0xE0 };
        }

        let c = cx as u8;
        let mut h = (dx >> 8) as u8;
        let mut hd = (h ^ (device_select >> 2)) & 1;
        let r = dx as u8;
        let n = (cx >> 8) as u8;
        let requested_sector_size = 128usize << n;
        let buffer_address = self.fdd640k_buffer_address(es, bp, 0, false);
        let transfer_bytes = if bx == 0 {
            requested_sector_size
        } else {
            bx as usize
        };

        if Self::fdd640k_segment_wraps(bp, transfer_bytes) {
            return if diagnostic { 0x00 } else { 0x20 };
        }
        if function_code & 0x10 != 0 {
            self.fdd_seek_cylinder[drive] = c;
        }

        let multi_track = function_code & 0x80 != 0;
        let mut track_index = (self.fdd_seek_cylinder[drive] as usize) * 2 + hd as usize;
        let mut offset = 0usize;
        let mut current_record = r;
        let mut sectors_read = 0usize;

        while offset < transfer_bytes {
            if let Some(data) =
                self.read_fdd_hle_sector_data(drive, track_index, c, h, current_record, n)
            {
                if !diagnostic {
                    let copy_len = data.len().min(transfer_bytes - offset);
                    for (index, &byte) in data.iter().take(copy_len).enumerate() {
                        let address =
                            self.fdd640k_buffer_address(es, bp, (offset + index) as u32, true);
                        self.memory.write_byte(address, byte);
                    }
                }
                offset += data.len().min(transfer_bytes - offset);
                sectors_read += 1;
            } else if multi_track && hd == 0 {
                hd = 1;
                h = 1;
                track_index = (self.fdd_seek_cylinder[drive] as usize) * 2 + 1;
                current_record = 1;
                if let Some(data) =
                    self.read_fdd_hle_sector_data(drive, track_index, c, h, current_record, n)
                {
                    if !diagnostic {
                        let copy_len = data.len().min(transfer_bytes - offset);
                        for (index, &byte) in data.iter().take(copy_len).enumerate() {
                            let address =
                                self.fdd640k_buffer_address(es, bp, (offset + index) as u32, true);
                            self.memory.write_byte(address, byte);
                        }
                    }
                    offset += data.len().min(transfer_bytes - offset);
                    sectors_read += 1;
                } else {
                    self.tracer.trace_int1bh_fdd_read(
                        drive,
                        c,
                        h,
                        current_record,
                        n,
                        sectors_read,
                        buffer_address,
                        0xE0,
                    );
                    return if diagnostic { 0x00 } else { 0xE0 };
                }
            } else {
                self.tracer.trace_int1bh_fdd_read(
                    drive,
                    c,
                    h,
                    current_record,
                    n,
                    sectors_read,
                    buffer_address,
                    0xE0,
                );
                return if diagnostic { 0x00 } else { 0xE0 };
            }
            current_record += 1;
        }

        self.tracer
            .trace_int1bh_fdd_read(drive, c, h, r, n, sectors_read, buffer_address, 0x00);
        0x00
    }

    fn read_fdd_hle_sector_data(
        &self,
        drive: usize,
        track_index: usize,
        c: u8,
        h: u8,
        r: u8,
        n: u8,
    ) -> Option<Vec<u8>> {
        if let Some(data) = self.floppy.read_sector_data(drive, track_index, c, h, r, n) {
            return Some(data.to_vec());
        }

        for size_code in 0..=7 {
            if size_code == n {
                continue;
            }
            if let Some(data) = self
                .floppy
                .read_sector_data(drive, track_index, c, h, r, size_code)
            {
                return Some(data.to_vec());
            }
        }

        None
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_fdd640k_write(
        &mut self,
        function_code: u8,
        device_select: u8,
        bx: u16,
        cx: u16,
        dx: u16,
        bp: u16,
        es: u16,
    ) -> u8 {
        let drive = (device_select & 0x03) as usize;
        let c = cx as u8;
        let mut h = (dx >> 8) as u8;
        let r = dx as u8;
        let n = (cx >> 8) as u8;
        let sector_size = 128usize << n;
        let buffer_address = self.fdd640k_buffer_address(es, bp, 0, false);
        let transfer_bytes = if bx == 0 { sector_size } else { bx as usize };
        let sector_count = transfer_bytes / sector_size;

        if !self.floppy.has_drive(drive) {
            self.tracer.trace_int1bh_fdd_write(
                drive,
                c,
                h,
                r,
                n,
                sector_count,
                buffer_address,
                0x60,
            );
            return 0x60;
        }
        if self.floppy.is_write_protected(drive) {
            self.tracer.trace_int1bh_fdd_write(
                drive,
                c,
                h,
                r,
                n,
                sector_count,
                buffer_address,
                0x70,
            );
            return 0x70;
        }
        if Self::fdd640k_segment_wraps(bp, transfer_bytes) {
            self.tracer.trace_int1bh_fdd_write(
                drive,
                c,
                h,
                r,
                n,
                sector_count,
                buffer_address,
                0x20,
            );
            return 0x20;
        }
        if function_code & 0x10 != 0 {
            self.fdd_seek_cylinder[drive] = c;
        }

        let mut hd = (h ^ (device_select >> 2)) & 1;
        let multi_track = function_code & 0x80 != 0;
        let mut track_index = (self.fdd_seek_cylinder[drive] as usize) * 2 + hd as usize;
        let mut offset = 0u32;
        let mut current_record = r;

        for _ in 0..sector_count {
            let mut data = vec![0u8; sector_size];
            for (index, byte) in data.iter_mut().enumerate() {
                let address = self.fdd640k_buffer_address(es, bp, offset + index as u32, false);
                *byte = self.memory.read_byte(address);
            }

            if !self
                .floppy
                .write_sector_data(drive, track_index, c, h, current_record, n, &data)
            {
                if multi_track && hd == 0 {
                    hd = 1;
                    h = 1;
                    track_index = (self.fdd_seek_cylinder[drive] as usize) * 2 + 1;
                    current_record = 1;
                    if !self.floppy.write_sector_data(
                        drive,
                        track_index,
                        c,
                        h,
                        current_record,
                        n,
                        &data,
                    ) {
                        break;
                    }
                } else {
                    break;
                }
            }
            offset += sector_size as u32;
            current_record += 1;
        }

        let result = if offset == 0 { 0xE0 } else { 0x00 };
        self.tracer
            .trace_int1bh_fdd_write(drive, c, h, r, n, sector_count, buffer_address, result);
        result
    }

    fn execute_fdd640k_read_id(&mut self, stack_base: u32, cx: u16, dx: u16, drive: usize) -> u8 {
        if !self.floppy.has_drive(drive) {
            return 0x60;
        }

        let c = cx as u8;
        let h = (dx >> 8) as u8;
        let track_index = c as usize * 2 + h as usize;
        if let Some(disk) = self.floppy.drive(drive)
            && let Some(sector) = disk.sector_at_index(track_index, 0)
        {
            let new_cx = u16::from(sector.cylinder) | (u16::from(sector.size_code) << 8);
            let new_dx = (u16::from(sector.head) << 8) | u16::from(sector.record);
            self.memory.write_byte(stack_base + 0x04, new_cx as u8);
            self.memory
                .write_byte(stack_base + 0x05, (new_cx >> 8) as u8);
            self.memory.write_byte(stack_base + 0x06, new_dx as u8);
            self.memory
                .write_byte(stack_base + 0x07, (new_dx >> 8) as u8);
            0x00
        } else {
            0xE0
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_fdd640k_format(
        &mut self,
        function_code: u8,
        device_select: u8,
        bx: u16,
        cx: u16,
        dx: u16,
        bp: u16,
        es: u16,
    ) -> u8 {
        let drive = (device_select & 0x03) as usize;
        if !self.floppy.has_drive(drive) {
            return 0x60;
        }
        if self.floppy.is_write_protected(drive) {
            return 0x70;
        }
        if function_code & 0x10 != 0 {
            self.fdd_seek_cylinder[drive] = cx as u8;
        }

        let h = (dx >> 8) as u8;
        let hd = (h ^ (device_select >> 2)) & 1;
        let track_index = (self.fdd_seek_cylinder[drive] as usize) * 2 + hd as usize;
        let sector_count = bx as usize / 4;
        let mut chrn = Vec::with_capacity(sector_count);

        for index in 0..sector_count {
            let base = (index as u32) * 4;
            let c_addr = self.fdd640k_buffer_address(es, bp, base, false);
            let h_addr = self.fdd640k_buffer_address(es, bp, base + 1, false);
            let r_addr = self.fdd640k_buffer_address(es, bp, base + 2, false);
            let n_addr = self.fdd640k_buffer_address(es, bp, base + 3, false);
            let c = self.memory.read_byte(c_addr);
            let h = self.memory.read_byte(h_addr);
            let r = self.memory.read_byte(r_addr);
            let n = self.memory.read_byte(n_addr);
            chrn.push((c, h, r, n));
        }

        self.floppy
            .format_track(drive, track_index, &chrn, (cx >> 8) as u8, dx as u8);
        0x00
    }

    fn fdd640k_buffer_address(&mut self, es: u16, bp: u16, offset: u32, write: bool) -> u32 {
        let linear = (u32::from(es) << 4)
            .wrapping_add(u32::from(bp))
            .wrapping_add(offset);
        if write {
            bios::hle_page_translate_write(self.hle_cr0, self.hle_cr3, linear, &mut self.memory)
        } else {
            bios::hle_page_translate_read(self.hle_cr0, self.hle_cr3, linear, &mut self.memory)
        }
    }

    fn fdd640k_segment_wraps(bp: u16, length: usize) -> bool {
        if length == 0 {
            return false;
        }
        let start = u32::from(bp);
        let end = start.wrapping_add(length as u32 - 1);
        (start & 0xFFFF) > (end & 0xFFFF)
    }

    fn fdd640k_device_matches_media(device_type: u8, image: Option<&FloppyImage>) -> bool {
        let Some(image) = image else {
            return true;
        };

        match device_type {
            0x90 => image.media_type == D88MediaType::Disk2HD,
            0x50 | 0x70 => image.media_type != D88MediaType::Disk2HD,
            _ => true,
        }
    }
}
