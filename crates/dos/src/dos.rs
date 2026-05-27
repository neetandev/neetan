//! INT 21h function dispatcher (AH routing).

use common::warn;

use crate::{
    BufferedInputState, CpuAccess, DriveIo, MemoryAccess, NeetanDos, SegmentRegister, Tracing,
    adjust_iret_ip, country, filesystem, memory, set_iret_carry, set_iret_zf, tables,
};

impl NeetanDos {
    /// Dispatches an INT 21h call based on the AH register.
    pub(crate) fn int21h(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        disk: &mut dyn DriveIo,
        tracer: &mut impl Tracing,
    ) {
        let indos_addr = self.state.indos_addr;
        let indos = memory.read_byte(indos_addr);
        memory.write_byte(indos_addr, indos.wrapping_add(1));

        let ah = (cpu.ax() >> 8) as u8;
        match ah {
            0x00 => {
                tracer.trace_int21h_terminate(cpu, memory);
                self.terminate_process(cpu, memory, 0, 0);
            }
            0x01 => self.int21h_01h_keyboard_input_with_echo(cpu, memory),
            0x02 => self.int21h_02h_display_character(cpu, memory),
            0x06 => self.int21h_06h_direct_console_io(cpu, memory),
            0x07 => self.int21h_07h_direct_char_input(cpu, memory),
            0x08 => self.int21h_08h_char_input_no_echo(cpu, memory),
            0x09 => self.int21h_09h_display_string(cpu, memory),
            0x0A => self.int21h_0ah_buffered_input(cpu, memory),
            0x0B => self.int21h_0bh_check_keyboard_status(cpu, memory),
            0x0C => self.int21h_0ch_flush_and_invoke(cpu, memory),
            0x0D => self.int21h_0dh_disk_reset(disk),
            0x0E => self.int21h_0eh_select_drive(cpu, memory),
            0x19 => self.int21h_19h_get_current_drive(cpu),
            0x1A => self.int21h_1ah_set_dta(cpu),
            0x1C => self.int21h_1ch_get_alloc_info(cpu, memory),
            0x1F => self.int21h_1fh_get_dpb_default(cpu, memory),
            0x25 => self.int21h_25h_set_interrupt_vector(cpu, memory),
            0x29 => self.int21h_29h_parse_filename(cpu, memory),
            0x2A => self.int21h_2ah_get_date(cpu),
            0x2B => self.int21h_2bh_set_date(cpu),
            0x2C => self.int21h_2ch_get_time(cpu),
            0x2D => self.int21h_2dh_set_time(cpu),
            0x2F => self.int21h_2fh_get_dta(cpu),
            0x30 => self.int21h_30h_get_version(cpu),
            0x32 => self.int21h_32h_get_dpb(cpu, memory),
            0x33 => self.int21h_33h_extended(cpu),
            0x34 => self.int21h_34h_get_indos(cpu),
            0x35 => self.int21h_35h_get_interrupt_vector(cpu, memory),
            0x36 => self.int21h_36h_get_free_disk_space(cpu, memory, disk),
            0x37 => self.int21h_37h_switch_char(cpu),
            0x38 => self.int21h_38h_get_country_info(cpu, memory),
            0x39 => self.int21h_39h_mkdir(cpu, memory, disk),
            0x3B => {
                tracer.trace_int21h_chdir(cpu, memory);
                tracer.trace_int21h_set_current_directory(cpu, memory);
                self.int21h_3bh_chdir(cpu, memory, disk);
            }
            0x3C => {
                tracer.trace_int21h_create(cpu, memory);
                self.int21h_3ch_create_file(cpu, memory, disk);
            }
            0x3D => {
                tracer.trace_int21h_open(cpu, memory);
                self.int21h_3dh_open_file(cpu, memory, disk);
            }
            0x3E => {
                tracer.trace_int21h_close(cpu, memory);
                self.int21h_3eh_close_handle(cpu, memory, disk);
            }
            0x3F => {
                tracer.trace_int21h_read(cpu, memory);
                self.int21h_3fh_read(cpu, memory, disk);
            }
            0x40 => {
                tracer.trace_int21h_write(cpu, memory);
                self.int21h_40h_write(cpu, memory, disk);
            }
            0x41 => {
                tracer.trace_int21h_delete(cpu, memory);
                self.int21h_41h_delete_file(cpu, memory, disk);
            }
            0x42 => {
                tracer.trace_int21h_lseek(cpu, memory);
                self.int21h_42h_lseek(cpu, memory);
            }
            0x43 => {
                tracer.trace_int21h_get_set_attributes(cpu, memory);
                self.int21h_43h_get_set_attributes(cpu, memory, disk);
            }
            0x44 => {
                tracer.trace_int21h_ioctl(cpu, memory);
                self.int21h_44h_ioctl(cpu, memory, disk);
            }
            0x45 => self.int21h_45h_dup_handle(cpu, memory),
            0x47 => {
                tracer.trace_int21h_get_current_directory(cpu, memory);
                self.int21h_47h_get_current_directory(cpu, memory);
            }
            0x48 => self.int21h_48h_allocate(cpu, memory),
            0x49 => self.int21h_49h_free(cpu, memory),
            0x4A => self.int21h_4ah_resize(cpu, memory),
            0x31 => self.int21h_31h_tsr(cpu, memory),
            0x4B => {
                tracer.trace_int21h_exec(cpu, memory);
                self.int21h_4bh_exec(cpu, memory, disk);
            }
            0x4C => {
                tracer.trace_int21h_terminate(cpu, memory);
                self.int21h_4ch_terminate(cpu, memory);
            }
            0x4D => self.int21h_4dh_get_return_code(cpu),
            0x4E => {
                tracer.trace_int21h_find_first(cpu, memory);
                self.int21h_4eh_find_first(cpu, memory, disk);
            }
            0x4F => {
                tracer.trace_int21h_find_next(cpu, memory);
                self.int21h_4fh_find_next(cpu, memory, disk);
            }
            0x50 => self.int21h_50h_set_psp(cpu),
            0x51 => self.int21h_51h_get_psp(cpu),
            0x52 => self.int21h_52h_get_sysvars(cpu),
            0x53 => self.int21h_53h_create_dpb_from_bpb(cpu, memory),
            0x55 => self.int21h_55h_create_child_psp(cpu, memory),
            0x56 => {
                tracer.trace_int21h_rename(cpu, memory);
                self.int21h_56h_rename(cpu, memory, disk);
            }
            0x57 => self.int21h_57h_get_set_datetime(cpu, memory),
            0x58 => self.int21h_58h_allocation_strategy(cpu, memory),
            0x5D => self.int21h_5dh_server_call(cpu, memory),
            0x5E | 0x5F => {
                cpu.set_ax(0x0001);
                set_iret_carry(cpu, memory, true);
            }
            0x60 => self.int21h_60h_truename(cpu, memory),
            0x61 => cpu.set_ax(cpu.ax() & 0xFF00),
            0x62 => self.int21h_62h_get_psp(cpu),
            0x63 => self.int21h_63h_dbcs_support(cpu),
            0x64 => {}
            0x65 => self.int21h_65h_get_extended_country_info(cpu, memory),
            0x67 => self.int21h_67h_set_handle_count(cpu, memory),
            0x68 | 0x6A => self.int21h_68h_commit_file(cpu, memory),
            0x69 => self.int21h_69h_get_set_media_info(cpu, memory),
            0x6B => {
                cpu.set_ax(0x0001);
                set_iret_carry(cpu, memory, true);
            }
            0xDC => self.int21h_dch_netware_connection_number(cpu, memory),
            0xFF => self.int21h_ffh_shell_step(cpu, memory, disk),
            _ => warn!("INT 21h AH={ah:#04X} is unimplemented"),
        }

        let indos = memory.read_byte(indos_addr);
        memory.write_byte(indos_addr, indos.wrapping_sub(1));
    }

    /// AH=02h: Display character.
    /// DL = character to display.
    /// Returns AL = last character output.
    fn int21h_02h_display_character(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
    ) {
        let dl = (cpu.dx() & 0xFF) as u8;
        self.console.process_byte(memory, dl);
        cpu.set_ax((cpu.ax() & 0xFF00) | dl as u16);
    }

    /// Reads one key byte for INT 21h input functions.
    ///
    /// Extended keys (arrows, function keys) have ch=0x00 in the keyboard buffer.
    /// NEC DOS IO.SYS expands these into the programmed escape sequences from
    /// the function key map (INT DCh CL=0x0C/0x0D). This method queues the
    /// escape sequence bytes and returns them one at a time.
    ///
    /// Returns `Some(byte)` if a byte is available, `None` if the keyboard
    /// buffer is empty (and no pending bytes).
    fn read_input_byte(&mut self, memory: &mut dyn MemoryAccess) -> Option<u8> {
        if let Some(byte) = self.state.pending_key_bytes.pop_front() {
            return Some(byte);
        }
        if !tables::key_available(memory) {
            return None;
        }
        let (scan, ch) = tables::read_key(memory);
        if ch == 0x00 {
            // Extended key: look up the escape sequence in the function key map.
            if let Some(seq) = self.lookup_fnkey_sequence(scan) {
                if seq.is_empty() {
                    // No mapping: return raw 0x00 + scan code (legacy fallback).
                    self.state.pending_key_bytes.push_back(scan);
                    return Some(0x00);
                }
                // Queue remaining bytes after the first one.
                for &b in &seq[1..] {
                    self.state.pending_key_bytes.push_back(b);
                }
                return Some(seq[0]);
            }
            // Unknown scan code: return raw 0x00 + scan code.
            self.state.pending_key_bytes.push_back(scan);
            return Some(0x00);
        }
        Some(ch)
    }

    /// Returns true if an input byte is ready (pending bytes or key in buffer).
    fn input_byte_available(&self, memory: &dyn MemoryAccess) -> bool {
        !self.state.pending_key_bytes.is_empty() || tables::key_available(memory)
    }

    /// Looks up the escape sequence for a hardware scan code in the function key map.
    /// Returns the sequence bytes (up to the first NUL), or None if not a mapped key.
    fn lookup_fnkey_sequence(&self, scan: u8) -> Option<Vec<u8>> {
        // Map hardware scan code to fn_key_map offset and slot size.
        // fn_key_map layout (specifier 0x0000):
        //   0-159:   F1-F10 (10 x 16 bytes), scan codes 0x62-0x6B
        //   160-319: Shift+F1-F10 (10 x 16 bytes) -- shifted versions, not mapped by scan
        //   320+:    editing keys (11 x 6 bytes):
        //     0=ROLL UP(0x36), 1=ROLL DOWN(0x37), 2=INS(0x38), 3=DEL(0x39),
        //     4=UP(0x3A), 5=LEFT(0x3B), 6=RIGHT(0x3C), 7=DOWN(0x3D),
        //     8=HOME(0x3E), 9=HELP(0x3F), 10=SHIFT+HOME
        let (offset, max_len) = match scan {
            0x62..=0x6B => {
                let idx = (scan - 0x62) as usize;
                (idx * 16, 15)
            }
            0x36..=0x3F => {
                let idx = (scan - 0x36) as usize;
                (320 + idx * 6, 5)
            }
            _ => return None,
        };

        let map = &self.state.fn_key_map;
        let mut seq = Vec::new();
        for i in 0..max_len {
            let b = map.get(offset + i).copied().unwrap_or(0);
            if b == 0 {
                break;
            }
            seq.push(b);
        }
        Some(seq)
    }

    /// AH=01h: Keyboard input with echo (blocking).
    /// Waits for a key, echoes it, returns AL = character.
    fn int21h_01h_keyboard_input_with_echo(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
    ) {
        match self.read_input_byte(memory) {
            Some(ch) => {
                self.console.process_byte(memory, ch);
                cpu.set_ax((cpu.ax() & 0xFF00) | ch as u16);
            }
            None => adjust_iret_ip(cpu, memory, -2),
        }
    }

    /// AH=06h: Direct console I/O.
    /// DL = character to output (if DL != FFh).
    /// DL = FFh: input request (returns ZF=1 if no char, ZF=0 + AL=char if available).
    fn int21h_06h_direct_console_io(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
    ) {
        let dl = (cpu.dx() & 0xFF) as u8;
        if dl == 0xFF {
            match self.read_input_byte(memory) {
                Some(ch) => {
                    cpu.set_ax((cpu.ax() & 0xFF00) | ch as u16);
                    set_iret_zf(cpu, memory, false);
                }
                None => {
                    cpu.set_ax(cpu.ax() & 0xFF00);
                    set_iret_zf(cpu, memory, true);
                }
            }
            return;
        }
        self.console.process_byte(memory, dl);
        cpu.set_ax((cpu.ax() & 0xFF00) | dl as u16);
    }

    /// AH=07h: Direct character input without echo (blocking, no Ctrl+C check).
    /// Waits for a key, returns AL = character.
    fn int21h_07h_direct_char_input(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
    ) {
        match self.read_input_byte(memory) {
            Some(ch) => cpu.set_ax((cpu.ax() & 0xFF00) | ch as u16),
            None => adjust_iret_ip(cpu, memory, -2),
        }
    }

    /// AH=08h: Character input without echo (blocking, with Ctrl+C check).
    /// Waits for a key, returns AL = character.
    fn int21h_08h_char_input_no_echo(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
    ) {
        match self.read_input_byte(memory) {
            Some(ch) => cpu.set_ax((cpu.ax() & 0xFF00) | ch as u16),
            None => adjust_iret_ip(cpu, memory, -2),
        }
    }

    /// AH=0Ah: Buffered keyboard input (blocking, with echo).
    /// DS:DX -> buffer: byte[0]=max chars, byte[1]=actual count, byte[2+]=data.
    fn int21h_0ah_buffered_input(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
    ) {
        if self.state.buffered_input.is_none() {
            let buffer_addr = cpu.linear_address(SegmentRegister::DS, cpu.dx());
            let max_chars = memory.read_byte(buffer_addr);
            if max_chars == 0 {
                return;
            }
            self.state.buffered_input = Some(BufferedInputState {
                buffer_addr,
                max_chars,
                current_pos: 0,
            });
        }

        let ch = match self.read_input_byte(memory) {
            Some(ch) => ch,
            None => {
                adjust_iret_ip(cpu, memory, -2);
                return;
            }
        };
        let bi = self.state.buffered_input.as_mut().unwrap();

        match ch {
            0x0D => {
                let addr = bi.buffer_addr;
                let pos = bi.current_pos;
                memory.write_byte(addr + 1, pos);
                memory.write_byte(addr + 2 + pos as u32, 0x0D);
                self.console.process_byte(memory, b'\r');
                self.console.process_byte(memory, b'\n');
                self.state.buffered_input = None;
            }
            0x08 => {
                if let Some(bi) = self.state.buffered_input.as_mut()
                    && bi.current_pos > 0
                {
                    bi.current_pos -= 1;
                    self.console.process_byte(memory, 0x08);
                    self.console.process_byte(memory, b' ');
                    self.console.process_byte(memory, 0x08);
                }
                adjust_iret_ip(cpu, memory, -2);
            }
            _ => {
                let bi = self.state.buffered_input.as_mut().unwrap();
                if bi.current_pos < bi.max_chars.saturating_sub(1) {
                    let addr = bi.buffer_addr + 2 + bi.current_pos as u32;
                    memory.write_byte(addr, ch);
                    bi.current_pos += 1;
                    self.console.process_byte(memory, ch);
                }
                adjust_iret_ip(cpu, memory, -2);
            }
        }
    }

    /// AH=0Bh: Check keyboard status (non-blocking).
    /// Returns AL = FFh if key available, 00h if not.
    fn int21h_0bh_check_keyboard_status(&self, cpu: &mut dyn CpuAccess, memory: &dyn MemoryAccess) {
        let al: u8 = if self.input_byte_available(memory) {
            0xFF
        } else {
            0x00
        };
        cpu.set_ax((cpu.ax() & 0xFF00) | al as u16);
    }

    /// AH=0Ch: Flush input buffer and invoke input function.
    /// AL = function to invoke (01h, 06h, 07h, 08h, or 0Ah).
    fn int21h_0ch_flush_and_invoke(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
    ) {
        tables::flush_keyboard_buffer(memory);
        self.state.pending_key_bytes.clear();
        let al = (cpu.ax() & 0xFF) as u8;
        match al {
            0x01 => self.int21h_01h_keyboard_input_with_echo(cpu, memory),
            0x06 => self.int21h_06h_direct_console_io(cpu, memory),
            0x07 => self.int21h_07h_direct_char_input(cpu, memory),
            0x08 => self.int21h_08h_char_input_no_echo(cpu, memory),
            0x0A => self.int21h_0ah_buffered_input(cpu, memory),
            _ => {}
        }
    }

    /// AH=09h: Display string.
    /// DS:DX = pointer to '$'-terminated string.
    /// Returns AL = 0x24 ('$').
    fn int21h_09h_display_string(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
    ) {
        let start = cpu.linear_address(SegmentRegister::DS, cpu.dx());
        for addr in start..start + 0xFFFFu32 {
            let byte = memory.read_byte(addr);
            if byte == b'$' {
                break;
            }
            self.console.process_byte(memory, byte);
        }
        cpu.set_ax((cpu.ax() & 0xFF00) | 0x24);
    }

    /// AH=0Eh: Select default drive.
    /// DL = new default drive (0=A, 1=B, ...).
    /// Returns AL = number of logical drives (LASTDRIVE).
    fn int21h_0eh_select_drive(&mut self, cpu: &mut dyn CpuAccess, memory: &dyn MemoryAccess) {
        self.state.current_drive = cpu.dx() as u8;
        let lastdrive = memory.read_byte(self.state.sysvars_base + tables::SYSVARS_OFF_LASTDRIVE);
        cpu.set_ax((cpu.ax() & 0xFF00) | lastdrive as u16);
    }

    /// AH=19h: Get current default drive.
    /// Returns AL = current drive (0=A, 1=B, ...).
    fn int21h_19h_get_current_drive(&self, cpu: &mut dyn CpuAccess) {
        cpu.set_ax((cpu.ax() & 0xFF00) | self.state.current_drive as u16);
    }

    /// AH=1Ah: Set Disk Transfer Area address.
    /// DS:DX = new DTA address.
    fn int21h_1ah_set_dta(&mut self, cpu: &dyn CpuAccess) {
        self.state.dta_segment = cpu.ds();
        self.state.dta_offset = cpu.dx();
        self.state.dta_address = cpu.linear_address(SegmentRegister::DS, cpu.dx());
    }

    /// AH=25h: Set interrupt vector.
    /// AL = interrupt number, DS:DX = new handler address.
    fn int21h_25h_set_interrupt_vector(&self, cpu: &dyn CpuAccess, memory: &mut dyn MemoryAccess) {
        let vector = (cpu.ax() & 0xFF) as u32;
        let ivt_addr = vector * 4;
        memory.write_word(ivt_addr, cpu.dx());
        memory.write_word(ivt_addr + 2, cpu.ds());
    }

    /// AH=2Fh: Get DTA address.
    /// Returns ES:BX = current DTA address.
    fn int21h_2fh_get_dta(&self, cpu: &mut dyn CpuAccess) {
        cpu.set_es(self.state.dta_segment);
        cpu.set_bx(self.state.dta_offset);
    }

    /// AH=1Fh: Get DPB for the default drive (undocumented).
    /// Returns DS:BX = DPB pointer, AL=00h. AL=FFh if invalid drive.
    fn int21h_1fh_get_dpb_default(&self, cpu: &mut dyn CpuAccess, memory: &dyn MemoryAccess) {
        self.get_dpb_for_drive(cpu, memory, self.state.current_drive);
    }

    /// AH=32h: Get DPB for specified drive (undocumented).
    /// DL = drive (0=default, 1=A, 2=B, ...).
    /// Returns DS:BX = DPB pointer, AL=00h. AL=FFh if invalid drive.
    fn int21h_32h_get_dpb(&self, cpu: &mut dyn CpuAccess, memory: &dyn MemoryAccess) {
        let dl = (cpu.dx() & 0xFF) as u8;
        let drive_index = if dl == 0 {
            self.state.current_drive
        } else {
            dl - 1
        };
        self.get_dpb_for_drive(cpu, memory, drive_index);
    }

    fn get_dpb_for_drive(
        &self,
        cpu: &mut dyn CpuAccess,
        memory: &dyn MemoryAccess,
        drive_index: u8,
    ) {
        if drive_index >= 26 {
            cpu.set_ax((cpu.ax() & 0xFF00) | 0xFF);
            return;
        }

        let cds_addr = tables::CDS_BASE + (drive_index as u32) * tables::CDS_ENTRY_SIZE;
        let cds_flags = memory.read_word(cds_addr + tables::CDS_OFF_FLAGS);
        if cds_flags == 0 {
            cpu.set_ax((cpu.ax() & 0xFF00) | 0xFF);
            return;
        }

        let dpb_off = memory.read_word(cds_addr + tables::CDS_OFF_DPB_PTR);
        let dpb_seg = memory.read_word(cds_addr + tables::CDS_OFF_DPB_PTR + 2);
        cpu.set_ds(dpb_seg);
        cpu.set_bx(dpb_off);
        cpu.set_ax(cpu.ax() & 0xFF00);
    }

    /// AH=60h: Qualify/canonicalize filename (TRUENAME, undocumented).
    /// DS:SI = input ASCIIZ path, ES:DI = 128-byte output buffer.
    /// Returns CF=0 on success, CF=1 with AX=error on failure.
    fn int21h_60h_truename(&self, cpu: &mut dyn CpuAccess, memory: &mut dyn MemoryAccess) {
        let input_addr = cpu.linear_address(SegmentRegister::DS, cpu.si());
        let output_addr = cpu.linear_address(SegmentRegister::ES, cpu.di());

        let mut path = Vec::new();
        for i in 0..128u32 {
            let byte = memory.read_byte(input_addr + i);
            if byte == 0 {
                break;
            }
            path.push(byte);
        }

        if path.is_empty() {
            cpu.set_ax(0x0003);
            set_iret_carry(cpu, memory, true);
            return;
        }

        // Determine drive letter and whether the path is absolute.
        let (drive_letter, rest) = if path.len() >= 2 && path[1] == b':' {
            (path[0].to_ascii_uppercase(), &path[2..])
        } else {
            (b'A' + self.state.current_drive, &path[..])
        };

        if !drive_letter.is_ascii_uppercase() {
            cpu.set_ax(0x000F);
            set_iret_carry(cpu, memory, true);
            return;
        }

        // Build the full path: if relative, prepend the CWD from the CDS.
        let mut full = Vec::with_capacity(128);
        full.push(drive_letter);
        full.push(b':');

        if rest.first() == Some(&b'\\') || rest.first() == Some(&b'/') {
            full.extend_from_slice(rest);
        } else {
            // Read the CWD from CDS for this drive.
            let drive_index = (drive_letter - b'A') as u32;
            let cds_addr = tables::CDS_BASE + drive_index * tables::CDS_ENTRY_SIZE;

            let mut cwd = Vec::new();
            for i in 0..67u32 {
                let byte = memory.read_byte(cds_addr + tables::CDS_OFF_PATH + i);
                if byte == 0 {
                    break;
                }
                cwd.push(byte);
            }

            // CWD is like "A:\DIR" -- skip the "A:" prefix.
            let cwd_path = if cwd.len() >= 2 && cwd[1] == b':' {
                &cwd[2..]
            } else {
                &cwd[..]
            };

            full.extend_from_slice(cwd_path);
            if !full.ends_with(b"\\") {
                full.push(b'\\');
            }
            full.extend_from_slice(rest);
        }

        // Normalize slashes.
        for byte in &mut full {
            if *byte == b'/' {
                *byte = b'\\';
            }
        }

        let normalized = normalize_path(&full);

        // Uppercase and write to output buffer.
        for (i, &byte) in normalized.iter().enumerate() {
            memory.write_byte(output_addr + i as u32, country::uppercase_char(byte));
        }
        memory.write_byte(output_addr + normalized.len() as u32, 0x00);

        set_iret_carry(cpu, memory, false);
    }

    /// AH=30h: Get DOS version number.
    /// Returns AL=major (6), AH=minor (20), BH=OEM, BL=0.
    fn int21h_30h_get_version(&self, cpu: &mut dyn CpuAccess) {
        let (major, minor) = self.state.version;
        cpu.set_ax((minor as u16) << 8 | major as u16);
        // BH=OEM serial number (0x00 = IBM/NEC compatible), BL=0x00
        cpu.set_bx(0x0000);
    }

    /// AH=33h: Extended functions.
    /// AL=00h: Get Ctrl-Break check state -> DL.
    /// AL=01h: Set Ctrl-Break check state <- DL.
    /// AL=02h: Swap Ctrl-Break flag: get old into DL, set new from DL.
    /// AL=03h/04h: Code page switching (reserved, returns AL=FFh).
    /// AL=06h: Get true DOS version -> BL=major, BH=minor.
    fn int21h_33h_extended(&mut self, cpu: &mut dyn CpuAccess) {
        let al = (cpu.ax() & 0xFF) as u8;
        match al {
            0x00 => {
                cpu.set_dx((cpu.dx() & 0xFF00) | self.state.ctrl_break as u16);
            }
            0x01 => {
                self.state.ctrl_break = (cpu.dx() & 0x00FF) != 0;
            }
            0x02 => {
                let old = self.state.ctrl_break as u16;
                self.state.ctrl_break = (cpu.dx() & 0x00FF) != 0;
                cpu.set_dx((cpu.dx() & 0xFF00) | old);
            }
            0x03 | 0x04 => {
                cpu.set_ax((cpu.ax() & 0xFF00) | 0xFF);
            }
            0x06 => {
                let (major, minor) = self.state.version;
                cpu.set_bx((minor as u16) << 8 | major as u16);
                cpu.set_dx(0x0000);
            }
            _ => {}
        }
    }

    /// AH=34h: Get address of InDOS flag.
    /// Returns ES:BX pointing to the InDOS byte.
    fn int21h_34h_get_indos(&self, cpu: &mut dyn CpuAccess) {
        let segment = (self.state.indos_addr >> 4) as u16;
        let offset = (self.state.indos_addr & 0x0F) as u16;
        cpu.set_es(segment);
        cpu.set_bx(offset);
    }

    /// AH=35h: Get interrupt vector.
    /// AL = interrupt number.
    /// Returns ES:BX = handler address.
    fn int21h_35h_get_interrupt_vector(&self, cpu: &mut dyn CpuAccess, memory: &dyn MemoryAccess) {
        let vector = (cpu.ax() & 0xFF) as u32;
        let ivt_addr = vector * 4;
        let offset = memory.read_word(ivt_addr);
        let segment = memory.read_word(ivt_addr + 2);
        cpu.set_es(segment);
        cpu.set_bx(offset);
    }

    /// AH=37h: Get/set switch character and availdev flag (undocumented).
    /// AL=00h: Get switch char -> DL, AL=0.
    /// AL=01h: Set switch char <- DL, AL=0.
    /// AL=02h: Get availdev flag -> DL=FFh (always true in DOS 3.0+), AL=0.
    /// AL=03h: Set availdev flag (ignored in DOS 3.0+), AL=0.
    fn int21h_37h_switch_char(&mut self, cpu: &mut dyn CpuAccess) {
        let al = (cpu.ax() & 0xFF) as u8;
        match al {
            0x00 => {
                cpu.set_dx((cpu.dx() & 0xFF00) | self.state.switch_char as u16);
                cpu.set_ax(cpu.ax() & 0xFF00);
            }
            0x01 => {
                self.state.switch_char = (cpu.dx() & 0xFF) as u8;
                cpu.set_ax(cpu.ax() & 0xFF00);
            }
            0x02 => {
                cpu.set_dx((cpu.dx() & 0xFF00) | 0xFF);
                cpu.set_ax(cpu.ax() & 0xFF00);
            }
            0x03 => {
                cpu.set_ax(cpu.ax() & 0xFF00);
            }
            _ => {
                cpu.set_ax((cpu.ax() & 0xFF00) | 0xFF);
            }
        }
    }

    /// AH=38h: Get country-dependent information.
    /// AL=00h: Get current country info. DS:DX = 34-byte buffer. BX = country code on return.
    fn int21h_38h_get_country_info(&self, cpu: &mut dyn CpuAccess, memory: &mut dyn MemoryAccess) {
        let buffer_addr = cpu.linear_address(SegmentRegister::DS, cpu.dx());
        country::write_country_info(memory, buffer_addr);
        cpu.set_bx(country::COUNTRY_CODE);
        set_iret_carry(cpu, memory, false);
    }

    /// AH=3Bh: Change current directory (CHDIR).
    /// DS:DX = ASCIIZ pathname.
    fn int21h_3bh_chdir(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        disk: &mut dyn DriveIo,
    ) {
        let path_addr = cpu.linear_address(SegmentRegister::DS, cpu.dx());

        let mut path_bytes = Vec::new();
        for i in 0..80u32 {
            let byte = memory.read_byte(path_addr + i);
            if byte == 0 {
                break;
            }
            path_bytes.push(byte);
        }

        match filesystem::change_directory(&mut self.state, memory, disk, &path_bytes) {
            Ok(()) => {
                set_iret_carry(cpu, memory, false);
            }
            Err(error_code) => {
                cpu.set_ax(error_code);
                set_iret_carry(cpu, memory, true);
            }
        }
    }

    /// AH=47h: Get current directory.
    /// DL = drive (0=default, 1=A, 2=B, ...).
    /// DS:SI = 64-byte buffer for path (without leading backslash).
    fn int21h_47h_get_current_directory(
        &self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
    ) {
        let dl = (cpu.dx() & 0xFF) as u8;
        let drive_index = if dl == 0 {
            self.state.current_drive
        } else {
            dl - 1
        };

        if drive_index >= 26 {
            cpu.set_ax(0x000F); // invalid drive
            set_iret_carry(cpu, memory, true);
            return;
        }

        let cds_addr = tables::CDS_BASE + (drive_index as u32) * tables::CDS_ENTRY_SIZE;
        let cds_flags = memory.read_word(cds_addr + tables::CDS_OFF_FLAGS);
        if cds_flags == 0 {
            cpu.set_ax(0x000F);
            set_iret_carry(cpu, memory, true);
            return;
        }

        // Read CDS path
        let mut path = Vec::new();
        for i in 0..67u32 {
            let byte = memory.read_byte(cds_addr + tables::CDS_OFF_PATH + i);
            if byte == 0 {
                break;
            }
            path.push(byte);
        }

        // Copy everything after "X:\" to the buffer
        let buffer_addr = cpu.linear_address(SegmentRegister::DS, cpu.si());
        let skip = if path.len() >= 3 && path[1] == b':' && path[2] == b'\\' {
            3
        } else if path.len() >= 2 && path[1] == b':' {
            2
        } else {
            0
        };

        let remaining = &path[skip..];
        for (i, &byte) in remaining.iter().enumerate() {
            memory.write_byte(buffer_addr + i as u32, byte);
        }
        memory.write_byte(buffer_addr + remaining.len() as u32, 0x00);

        set_iret_carry(cpu, memory, false);
    }

    /// AH=48h: Allocate memory block.
    /// BX = number of paragraphs requested.
    /// Success: CF=0, AX = segment of allocated block.
    /// Failure: CF=1, AX = 8 (insufficient memory), BX = largest available.
    fn int21h_48h_allocate(&mut self, cpu: &mut dyn CpuAccess, memory: &mut dyn MemoryAccess) {
        let paragraphs = cpu.bx();
        let first_mcb = memory.read_word(self.state.sysvars_base - 2);
        // UMB is considered for allocation only when DOS has been told to
        // link UMB (INT 21h AX=5803h) and the memory manager has a UMB
        // region. Strategy flags +0x40/+0x80 drive the actual preference.
        let umb_first = self
            .state
            .umb_link
            .then(|| self.umb_first_mcb_segment())
            .flatten();
        match memory::allocate_dos(
            memory,
            first_mcb,
            umb_first,
            paragraphs,
            self.state.current_psp,
            self.state.allocation_strategy,
        ) {
            Ok(segment) => {
                cpu.set_ax(segment);
                set_iret_carry(cpu, memory, false);
            }
            Err((error_code, largest)) => {
                cpu.set_ax(error_code as u16);
                cpu.set_bx(largest);
                set_iret_carry(cpu, memory, true);
            }
        }
    }

    /// AH=49h: Free memory block.
    /// ES = segment of block to free.
    /// Success: CF=0.
    /// Failure: CF=1, AX = error code.
    fn int21h_49h_free(&self, cpu: &mut dyn CpuAccess, memory: &mut dyn MemoryAccess) {
        let data_segment = cpu.es();
        let first_mcb = memory.read_word(self.state.sysvars_base - 2);
        let umb_first = self
            .state
            .umb_link
            .then(|| self.umb_first_mcb_segment())
            .flatten();
        match memory::free_dos(memory, first_mcb, umb_first, data_segment) {
            Ok(()) => {
                set_iret_carry(cpu, memory, false);
            }
            Err(error_code) => {
                cpu.set_ax(error_code as u16);
                set_iret_carry(cpu, memory, true);
            }
        }
    }

    /// AH=4Ah: Resize memory block (SETBLOCK).
    /// ES = segment of block, BX = new size in paragraphs.
    /// Success: CF=0.
    /// Failure: CF=1, AX = error code, BX = max available paragraphs.
    fn int21h_4ah_resize(&self, cpu: &mut dyn CpuAccess, memory: &mut dyn MemoryAccess) {
        let data_segment = cpu.es();
        let new_paragraphs = cpu.bx();
        let first_mcb = memory.read_word(self.state.sysvars_base - 2);
        let umb_first = self
            .state
            .umb_link
            .then(|| self.umb_first_mcb_segment())
            .flatten();
        match memory::resize_dos(memory, first_mcb, umb_first, data_segment, new_paragraphs) {
            Ok(()) => {
                set_iret_carry(cpu, memory, false);
            }
            Err((error_code, max_available)) => {
                cpu.set_ax(error_code as u16);
                cpu.set_bx(max_available);
                set_iret_carry(cpu, memory, true);
            }
        }
    }

    /// AH=31h: Terminate and Stay Resident.
    /// AL = return code, DX = paragraphs to keep resident.
    fn int21h_31h_tsr(&mut self, cpu: &mut dyn CpuAccess, memory: &mut dyn MemoryAccess) {
        let return_code = (cpu.ax() & 0xFF) as u8;
        let keep_paragraphs = cpu.dx();
        self.terminate_process_tsr(cpu, memory, return_code, keep_paragraphs);
    }

    /// AH=4Ch: Terminate process with return code.
    /// AL = return code.
    fn int21h_4ch_terminate(&mut self, cpu: &mut dyn CpuAccess, memory: &mut dyn MemoryAccess) {
        let return_code = (cpu.ax() & 0xFF) as u8;
        self.terminate_process(cpu, memory, return_code, 0);
    }

    /// AH=4Dh: Get return code of child process.
    /// Returns AL = exit code, AH = termination type (0-3).
    fn int21h_4dh_get_return_code(&mut self, cpu: &mut dyn CpuAccess) {
        cpu.set_ax(
            (self.state.last_termination_type as u16) << 8 | self.state.last_return_code as u16,
        );
        // Clear after reading (one-shot)
        self.state.last_return_code = 0;
        self.state.last_termination_type = 0;
    }

    /// AH=50h: Set current PSP address (undocumented).
    /// BX = new PSP segment.
    fn int21h_50h_set_psp(&mut self, cpu: &dyn CpuAccess) {
        self.state.current_psp = cpu.bx();
    }

    /// AH=51h: Get current PSP address (undocumented).
    /// Returns BX = segment of current PSP.
    fn int21h_51h_get_psp(&self, cpu: &mut dyn CpuAccess) {
        cpu.set_bx(self.state.current_psp);
    }

    /// AH=52h: Get List of Lists (SYSVARS pointer).
    /// Returns ES:BX pointing to SYSVARS. The pointer carries a non-zero
    /// offset so that programs can read the negative-offset fields (e.g. the
    /// first-MCB pointer at SYSVARS-2) via ES:[BX-2] without underflowing BX.
    fn int21h_52h_get_sysvars(&self, cpu: &mut dyn CpuAccess) {
        cpu.set_es(tables::SYSVARS_LIST_SEGMENT);
        cpu.set_bx(tables::SYSVARS_LIST_OFFSET);
    }

    /// AH=53h: Translate a BIOS Parameter Block (DS:SI) into a DOS 4.x Drive
    /// Parameter Block (ES:BP).
    fn int21h_53h_create_dpb_from_bpb(
        &self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
    ) {
        let bpb_addr = cpu.linear_address(SegmentRegister::DS, cpu.si());
        let dpb_addr = cpu.linear_address(SegmentRegister::ES, cpu.bp());

        let bytes_per_sector = memory.read_word(bpb_addr);
        let sectors_per_cluster = memory.read_byte(bpb_addr + 0x02);
        let reserved_sectors = memory.read_word(bpb_addr + 0x03);
        let num_fats = memory.read_byte(bpb_addr + 0x05);
        let root_entries = memory.read_word(bpb_addr + 0x06);
        let total_sectors_16 = memory.read_word(bpb_addr + 0x08);
        let media_descriptor = memory.read_byte(bpb_addr + 0x0A);
        let sectors_per_fat = memory.read_word(bpb_addr + 0x0B);
        let total_sectors_32 = read_dword(memory, bpb_addr + 0x15);
        let total_sectors = if total_sectors_16 != 0 {
            total_sectors_16 as u32
        } else {
            total_sectors_32
        };

        if bytes_per_sector == 0
            || sectors_per_cluster == 0
            || !sectors_per_cluster.is_power_of_two()
            || num_fats == 0
            || sectors_per_fat == 0
            || total_sectors == 0
        {
            cpu.set_ax(0x0001);
            set_iret_carry(cpu, memory, true);
            return;
        }

        let first_root_sector = reserved_sectors as u32 + num_fats as u32 * sectors_per_fat as u32;
        let root_dir_sectors = (root_entries as u32 * 32).div_ceil(bytes_per_sector as u32);
        let first_data_sector = first_root_sector + root_dir_sectors;
        let max_cluster = total_sectors
            .saturating_sub(first_data_sector)
            .checked_div(sectors_per_cluster as u32)
            .unwrap_or(0)
            + 1;

        memory.write_word(
            dpb_addr + tables::DPB_OFF_BYTES_PER_SECTOR,
            bytes_per_sector,
        );
        memory.write_byte(
            dpb_addr + tables::DPB_OFF_CLUSTER_MASK,
            sectors_per_cluster - 1,
        );
        memory.write_byte(
            dpb_addr + tables::DPB_OFF_CLUSTER_SHIFT,
            sectors_per_cluster.trailing_zeros() as u8,
        );
        memory.write_word(
            dpb_addr + tables::DPB_OFF_RESERVED_SECTORS,
            reserved_sectors,
        );
        memory.write_byte(dpb_addr + tables::DPB_OFF_NUM_FATS, num_fats);
        memory.write_word(dpb_addr + tables::DPB_OFF_ROOT_ENTRIES, root_entries);
        memory.write_word(
            dpb_addr + tables::DPB_OFF_FIRST_DATA_SECTOR,
            first_data_sector.min(u16::MAX as u32) as u16,
        );
        memory.write_word(
            dpb_addr + tables::DPB_OFF_MAX_CLUSTER,
            max_cluster.min(u16::MAX as u32) as u16,
        );
        memory.write_word(dpb_addr + tables::DPB_OFF_SECTORS_PER_FAT, sectors_per_fat);
        memory.write_word(
            dpb_addr + tables::DPB_OFF_FIRST_ROOT_SECTOR,
            first_root_sector.min(u16::MAX as u32) as u16,
        );
        memory.write_byte(dpb_addr + tables::DPB_OFF_MEDIA_DESC, media_descriptor);
        // Real DOS 6.20 leaves the access flag (+0x18) untouched; AH=53h only
        // fills fields derivable from the BPB.
        memory.write_word(dpb_addr + 0x1D, 0x0000);
        memory.write_word(dpb_addr + 0x1F, 0xFFFF);
        set_iret_carry(cpu, memory, false);
    }

    /// AH=58h: Get/set memory allocation strategy / UMB link.
    /// AL=00h: Get -> AX = strategy (0=first fit, 1=best fit, 2=last fit,
    ///         +0x40 high-first-then-low, +0x80 high-only).
    /// AL=01h: Set <- BX = strategy.
    /// AL=02h: Get -> AX = UMB link state (0 = not linked, 1 = linked).
    /// AL=03h: Set <- BX = UMB link state.
    fn int21h_58h_allocation_strategy(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
    ) {
        let al = (cpu.ax() & 0xFF) as u8;
        match al {
            0x00 => {
                cpu.set_ax(self.state.allocation_strategy);
                set_iret_carry(cpu, memory, false);
            }
            0x01 => {
                // Valid strategies: low nibble 0..=2 (first/best/last fit),
                // high byte 0x00 (low-only), 0x40 (high-first-then-low), or
                // 0x80 (high-only). Anything else is rejected with
                // AX=01h/CF=1 per INT 21h convention.
                let strategy = cpu.bx();
                if matches!(
                    strategy,
                    0x00 | 0x01 | 0x02 | 0x40 | 0x41 | 0x42 | 0x80 | 0x81 | 0x82
                ) {
                    self.state.allocation_strategy = strategy;
                    set_iret_carry(cpu, memory, false);
                } else {
                    cpu.set_ax(0x0001);
                    set_iret_carry(cpu, memory, true);
                }
            }
            0x02 => {
                // Returns the link state in AL, preserving the AH=58h
                // function code in AH (real DOS leaves AH untouched).
                cpu.set_ax((cpu.ax() & 0xFF00) | u16::from(self.state.umb_link));
                set_iret_carry(cpu, memory, false);
            }
            0x03 => {
                let link_state = cpu.bx();
                if link_state > 1 {
                    cpu.set_ax(0x0001);
                    set_iret_carry(cpu, memory, true);
                    return;
                }

                // Only accept the UMB link request if UMB is actually
                // available (memory manager initialized and UMB enabled).
                if let Some(umb_first) = self.umb_first_mcb_segment() {
                    let first_mcb = memory.read_word(self.state.sysvars_base - 2);
                    let linked = link_state != 0;
                    match memory::set_dos_umb_link_state(memory, first_mcb, Some(umb_first), linked)
                    {
                        Ok(()) => {
                            self.state.umb_link = linked;
                            set_iret_carry(cpu, memory, false);
                        }
                        Err(error_code) => {
                            cpu.set_ax(error_code as u16);
                            set_iret_carry(cpu, memory, true);
                        }
                    }
                } else {
                    self.state.umb_link = false;
                    set_iret_carry(cpu, memory, false);
                }
            }
            _ => {
                cpu.set_ax(0x0001); // invalid function
                set_iret_carry(cpu, memory, true);
            }
        }
    }

    /// AH=62h: Get PSP address.
    /// Returns BX = segment of current PSP.
    fn int21h_62h_get_psp(&self, cpu: &mut dyn CpuAccess) {
        cpu.set_bx(self.state.current_psp);
    }

    /// AH=63h: DBCS support functions (undocumented).
    /// AL=00h: Get DBCS lead byte table -> DS:SI.
    /// AL=01h: Set interim console flag <- DL.
    /// AL=02h: Get interim console flag -> DL.
    fn int21h_63h_dbcs_support(&mut self, cpu: &mut dyn CpuAccess) {
        let al = (cpu.ax() & 0xFF) as u8;
        match al {
            0x00 => {
                let segment = (self.state.dbcs_table_addr >> 4) as u16;
                let offset = (self.state.dbcs_table_addr & 0x0F) as u16;
                cpu.set_ds(segment);
                cpu.set_si(offset);
            }
            0x01 => {
                self.state.interim_console_flag = (cpu.dx() & 0xFF) as u8;
            }
            0x02 => {
                cpu.set_dx((cpu.dx() & 0xFF00) | self.state.interim_console_flag as u16);
            }
            _ => {}
        }
    }

    /// AH=65h: Get extended country information and uppercase functions.
    /// AL=01h: Get extended country info.
    /// AL=03h: Get country lowercase table pointer.
    /// AL=05h: Get country filename character table pointer.
    /// AL=07h: Get DBCS table info.
    /// AL=20h/A0h: Uppercase character in DL.
    /// AL=21h/A1h: Uppercase counted string at DS:DX, length CX.
    /// AL=22h/A2h: Uppercase ASCIIZ string at DS:DX.
    /// AL=23h/A3h: Yes/no character check for DL.
    fn int21h_65h_get_extended_country_info(
        &self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
    ) {
        let al = (cpu.ax() & 0xFF) as u8;

        match al {
            0x20 | 0xA0 => {
                let ch = (cpu.dx() & 0xFF) as u8;
                cpu.set_dx((cpu.dx() & 0xFF00) | country::uppercase_char(ch) as u16);
                set_iret_carry(cpu, memory, false);
            }
            0x21 | 0xA1 => {
                let addr = cpu.linear_address(SegmentRegister::DS, cpu.dx());
                let len = cpu.cx() as u32;
                for i in 0..len {
                    let ch = memory.read_byte(addr + i);
                    memory.write_byte(addr + i, country::uppercase_char(ch));
                }
                set_iret_carry(cpu, memory, false);
            }
            0x22 | 0xA2 => {
                let addr = cpu.linear_address(SegmentRegister::DS, cpu.dx());
                for i in 0..256u32 {
                    let ch = memory.read_byte(addr + i);
                    if ch == 0 {
                        break;
                    }
                    memory.write_byte(addr + i, country::uppercase_char(ch));
                }
                set_iret_carry(cpu, memory, false);
            }
            0x23 | 0xA3 => {
                let ch = (cpu.dx() & 0xFF) as u8;
                if country::is_yesno_char(ch) {
                    cpu.set_ax(0x0000);
                } else {
                    cpu.set_ax(0x0002);
                }
                set_iret_carry(cpu, memory, false);
            }
            0x01 | 0x03 | 0x05 | 0x07 => {
                let buffer_addr = cpu.linear_address(SegmentRegister::ES, cpu.di());
                let max_bytes = cpu.cx();

                let written = match al {
                    0x01 => country::write_extended_country_info(memory, buffer_addr, max_bytes),
                    0x03 => country::write_extended_lowercase_info(
                        memory,
                        buffer_addr,
                        max_bytes,
                        self.state.dbcs_table_addr,
                    ),
                    0x05 => country::write_extended_filename_char_info(
                        memory,
                        buffer_addr,
                        max_bytes,
                        self.state.dbcs_table_addr,
                    ),
                    0x07 => country::write_extended_dbcs_info(memory, buffer_addr, max_bytes),
                    _ => unreachable!(),
                };

                if written > 0 {
                    cpu.set_cx(written);
                    set_iret_carry(cpu, memory, false);
                } else {
                    cpu.set_ax(0x0001);
                    set_iret_carry(cpu, memory, true);
                }
            }
            _ => {
                cpu.set_ax(0x0001);
                set_iret_carry(cpu, memory, true);
            }
        }
    }

    /// AH=55h: Create child PSP (undocumented).
    /// DX = segment for new PSP. SI = memory top for child.
    fn int21h_55h_create_child_psp(&self, cpu: &dyn CpuAccess, memory: &mut dyn MemoryAccess) {
        let child_seg = cpu.dx();
        let mem_top = cpu.si();
        let base = (child_seg as u32) << 4;

        // Write INT 20h at PSP:0000.
        memory.write_byte(base, 0xCD);
        memory.write_byte(base + 1, 0x20);

        // Memory top segment.
        memory.write_word(base + tables::PSP_OFF_MEM_TOP, mem_top);

        let parent_base = (self.state.current_psp as u32) << 4;
        let (parent_jft_addr, parent_jft_size) =
            crate::DosState::handle_table_info_for_psp(memory, self.state.current_psp);
        for i in 0..20u32 {
            let handle = if i < parent_jft_size as u32 {
                memory.read_byte(parent_jft_addr + i)
            } else {
                0xFF
            };
            memory.write_byte(base + tables::PSP_OFF_JFT + i, handle);
        }

        // Default handle table size and pointer.
        memory.write_word(base + tables::PSP_OFF_HANDLE_SIZE, 20);
        tables::write_far_ptr(
            memory,
            base + tables::PSP_OFF_HANDLE_PTR,
            child_seg,
            tables::PSP_OFF_JFT as u16,
        );

        // Parent PSP.
        memory.write_word(base + tables::PSP_OFF_PARENT_PSP, self.state.current_psp);

        // Environment segment (inherited from parent).
        let parent_env = memory.read_word(parent_base + tables::PSP_OFF_ENV_SEG);
        memory.write_word(base + tables::PSP_OFF_ENV_SEG, parent_env);

        // INT 21h / RETF stub at PSP:0050h.
        memory.write_block(base + tables::PSP_OFF_INT21_STUB, &[0xCD, 0x21, 0xCB]);
    }

    /// AH=67h: Set handle count.
    /// BX = requested number of handles for the current process.
    fn int21h_67h_set_handle_count(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
    ) {
        let requested = cpu.bx();
        if requested > u8::MAX as u16 + 1 {
            cpu.set_ax(0x0004);
            set_iret_carry(cpu, memory, true);
            return;
        }

        let psp_base = (self.state.current_psp as u32) << 4;
        let (old_table_addr, current_size) = self.state.handle_table_info(memory);
        if requested <= current_size {
            set_iret_carry(cpu, memory, false);
            return;
        }

        let paragraphs = requested.div_ceil(16);
        let first_mcb = memory.read_word(self.state.sysvars_base - 2);
        let umb_first = self
            .state
            .umb_link
            .then(|| self.umb_first_mcb_segment())
            .flatten();

        let new_segment = match memory::allocate_dos(
            memory,
            first_mcb,
            umb_first,
            paragraphs,
            self.state.current_psp,
            self.state.allocation_strategy,
        ) {
            Ok(segment) => segment,
            Err((error_code, largest)) => {
                cpu.set_ax(error_code as u16);
                cpu.set_bx(largest);
                set_iret_carry(cpu, memory, true);
                return;
            }
        };

        let new_table_addr = (new_segment as u32) << 4;
        for i in 0..requested as u32 {
            memory.write_byte(new_table_addr + i, 0xFF);
        }
        for i in 0..current_size as u32 {
            let handle = memory.read_byte(old_table_addr + i);
            memory.write_byte(new_table_addr + i, handle);
        }

        let old_offset = memory.read_word(psp_base + tables::PSP_OFF_HANDLE_PTR);
        let old_segment = memory.read_word(psp_base + tables::PSP_OFF_HANDLE_PTR + 2);
        memory.write_word(psp_base + tables::PSP_OFF_HANDLE_SIZE, requested);
        tables::write_far_ptr(
            memory,
            psp_base + tables::PSP_OFF_HANDLE_PTR,
            new_segment,
            0x0000,
        );

        if old_segment != self.state.current_psp || old_offset != tables::PSP_OFF_JFT as u16 {
            let _ = memory::free_dos(memory, first_mcb, umb_first, old_segment);
        }

        set_iret_carry(cpu, memory, false);
    }

    /// AH=68h / AH=6Ah: Commit (flush) file.
    /// BX = file handle. HLE does not buffer, so just validate and return CF=0.
    fn int21h_68h_commit_file(&self, cpu: &mut dyn CpuAccess, memory: &mut dyn MemoryAccess) {
        let handle = cpu.bx();
        match self.state.handle_to_sft_index(handle, memory) {
            Ok(_) => set_iret_carry(cpu, memory, false),
            Err(err) => {
                cpu.set_ax(err);
                set_iret_carry(cpu, memory, true);
            }
        }
    }

    /// AH=69h: Get/set media info (volume serial number).
    /// AL=00h: Get. BL = drive (0=default). DS:DX = 25-byte buffer.
    /// AL=01h: Set (ignored for our HLE implementation).
    fn int21h_69h_get_set_media_info(
        &self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
    ) {
        let al = (cpu.ax() & 0xFF) as u8;
        let bl = (cpu.bx() & 0xFF) as u8;
        let drive_index = if bl == 0 {
            self.state.current_drive
        } else {
            bl - 1
        };

        if drive_index >= 26 {
            cpu.set_ax(0x000F);
            set_iret_carry(cpu, memory, true);
            return;
        }

        match al {
            0x00 => {
                let buffer_addr = cpu.linear_address(SegmentRegister::DS, cpu.dx());
                // Info level: 0
                memory.write_word(buffer_addr, 0x0000);
                // Serial number (synthetic from drive index).
                memory.write_word(buffer_addr + 2, 0x1234);
                memory.write_word(buffer_addr + 4, 0x5678 + drive_index as u16);
                // Volume label (11 bytes, space-padded).
                memory.write_block(buffer_addr + 6, b"NO NAME    ");
                // File system type (8 bytes).
                memory.write_block(buffer_addr + 17, b"FAT12   ");
                set_iret_carry(cpu, memory, false);
            }
            0x01 => {
                set_iret_carry(cpu, memory, false);
            }
            _ => {
                cpu.set_ax(0x0001);
                set_iret_carry(cpu, memory, true);
            }
        }
    }

    /// AH=2Ah: Get system date.
    /// Returns CX=year, DH=month, DL=day, AL=day-of-week.
    fn int21h_2ah_get_date(&mut self, cpu: &mut dyn CpuAccess) {
        let (year, month, day, dow) = self.state.current_date_parts();
        cpu.set_cx(year);
        cpu.set_dx((month << 8) | day);
        cpu.set_ax((cpu.ax() & 0xFF00) | dow);
    }

    /// AH=2Bh: Set system date (no-op).
    /// Returns AL=0 (success).
    fn int21h_2bh_set_date(&mut self, cpu: &mut dyn CpuAccess) {
        cpu.set_ax(cpu.ax() & 0xFF00);
    }

    /// AH=2Ch: Get system time.
    /// Returns CH=hour, CL=minute, DH=second, DL=hundredths.
    fn int21h_2ch_get_time(&mut self, cpu: &mut dyn CpuAccess) {
        let (hour, minute, second) = self.state.current_time_parts();
        cpu.set_cx((hour as u16) << 8 | minute as u16);
        cpu.set_dx((second as u16) << 8);
    }

    /// AH=2Dh: Set system time (no-op).
    /// Returns AL=0 (success).
    fn int21h_2dh_set_time(&mut self, cpu: &mut dyn CpuAccess) {
        cpu.set_ax(cpu.ax() & 0xFF00);
    }

    /// AH=DCh: Get NetWare connection number. No network redirector is
    /// present, so report connection 0 (AL=0) with CF=0. Real DOS 6.20 leaves
    /// CX untouched.
    fn int21h_dch_netware_connection_number(
        &self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
    ) {
        cpu.set_ax(cpu.ax() & 0xFF00);
        set_iret_carry(cpu, memory, false);
    }

    /// First MCB segment of the UMB chain, or `None` when no UMB region is
    /// available. The segment depends on whether EMS is enabled (D000h with
    /// EMS, C000h without).
    fn umb_first_mcb_segment(&self) -> Option<u16> {
        self.state
            .memory_manager
            .as_ref()
            .filter(|memory_manager| memory_manager.is_umb_enabled())
            .map(|memory_manager| memory_manager.umb_first_mcb_segment())
    }
}

/// Normalizes a DOS path by resolving `.` and `..` components.
/// Input/output is a byte vector like `A:\FOO\BAR\..\BAZ`.
pub(crate) fn normalize_path(path: &[u8]) -> Vec<u8> {
    // Find the root prefix (e.g. "A:\")
    let root_len = if path.len() >= 3 && path[1] == b':' && path[2] == b'\\' {
        3
    } else if path.len() >= 2 && path[1] == b':' {
        2
    } else {
        0
    };

    let prefix = &path[..root_len];
    let rest = &path[root_len..];

    let mut components: Vec<&[u8]> = Vec::new();
    for part in rest.split(|&b| b == b'\\') {
        if part.is_empty() || part == b"." {
            continue;
        } else if part == b".." {
            components.pop();
        } else {
            components.push(part);
        }
    }

    let mut result = Vec::from(prefix);
    for (i, component) in components.iter().enumerate() {
        if i > 0 {
            result.push(b'\\');
        }
        result.extend_from_slice(component);
    }

    // Ensure at least "X:\"
    if result.len() == 2 && result[1] == b':' {
        result.push(b'\\');
    }

    result
}

fn read_dword(memory: &dyn MemoryAccess, address: u32) -> u32 {
    memory.read_word(address) as u32 | ((memory.read_word(address + 2) as u32) << 16)
}

#[cfg(test)]
mod tests {
    use crate::{
        CpuAccess, MemoryAccess, NeetanDos,
        memory::{self, memory_manager::MemoryManager},
        tables::{
            ENV_BLOCK_PARAGRAPHS, FIRST_MCB_SEGMENT, FREE_MCB_SEGMENT, MCB_OFF_NAME, MCB_OFF_OWNER,
            MCB_OFF_SIZE, MCB_OFF_TYPE, MCB_OWNER_DOS, MEMORY_TOP_SEGMENT, SYSVARS_BASE,
            UMB_FIRST_MCB_SEGMENT,
        },
        test_support::{MockCpu, MockMemory},
    };

    fn prepare_dos_with_umb() -> (NeetanDos, MockMemory) {
        let mut dos = NeetanDos::new();
        let mut memory = MockMemory::with_extended_memory(0x200000, 0x200000);
        let layout = memory::initial_mcb_layout(ENV_BLOCK_PARAGRAPHS);
        memory::write_initial_mcb_chain(&mut memory, layout);
        memory.write_word(SYSVARS_BASE - 2, FIRST_MCB_SEGMENT);
        dos.state.memory_manager = Some(MemoryManager::new(
            memory.extended_memory_size(),
            true,
            true,
            false,
            &mut memory,
        ));
        (dos, memory)
    }

    fn iret_carry(memory: &MockMemory, cpu: &MockCpu) -> bool {
        memory.read_word(cpu.iret_flags_addr()) & 0x0001 != 0
    }

    fn mcb_addr(segment: u16) -> u32 {
        (segment as u32) << 4
    }

    #[test]
    fn int21h_dch_reports_netware_absent() {
        let dos = NeetanDos::new();
        let mut memory = MockMemory::with_extended_memory(0x200000, 0);
        let mut cpu = MockCpu::default();

        cpu.set_ax(0xDC7F);
        cpu.set_cx(0x1234);
        dos.int21h_dch_netware_connection_number(&mut cpu, &mut memory);

        assert_eq!(cpu.ax(), 0xDC00);
        assert_eq!(
            cpu.cx(),
            0x1234,
            "AH=DCh leaves CX untouched on real DOS 6.20"
        );
        assert!(!iret_carry(&memory, &cpu));
    }

    #[test]
    fn int21h_5802_preserves_function_in_ah() {
        let (mut dos, mut memory) = prepare_dos_with_umb();
        let mut cpu = MockCpu::default();

        cpu.set_ax(0x5802);
        dos.int21h_58h_allocation_strategy(&mut cpu, &mut memory);

        assert_eq!(cpu.ax(), 0x5800);
        assert!(!iret_carry(&memory, &cpu));

        cpu.set_ax(0x5803);
        cpu.set_bx(1);
        dos.int21h_58h_allocation_strategy(&mut cpu, &mut memory);
        assert!(!iret_carry(&memory, &cpu));

        cpu.set_ax(0x5802);
        dos.int21h_58h_allocation_strategy(&mut cpu, &mut memory);

        assert_eq!(cpu.ax(), 0x5801);
        assert!(!iret_carry(&memory, &cpu));
    }

    #[test]
    fn int21h_5803_links_and_unlinks_umb_with_spanning_entry() {
        let (mut dos, mut memory) = prepare_dos_with_umb();
        let mut cpu = MockCpu::default();

        cpu.set_ax(0x0003);
        cpu.set_bx(1);
        dos.int21h_58h_allocation_strategy(&mut cpu, &mut memory);

        assert!(dos.state.umb_link);
        assert!(!iret_carry(&memory, &cpu));
        assert_eq!(
            memory.read_byte(mcb_addr(FREE_MCB_SEGMENT) + MCB_OFF_TYPE),
            0x4D
        );
        assert_eq!(
            memory.read_byte(mcb_addr(MEMORY_TOP_SEGMENT) + MCB_OFF_TYPE),
            0x4D
        );
        assert_eq!(
            memory.read_word(mcb_addr(MEMORY_TOP_SEGMENT) + MCB_OFF_OWNER),
            MCB_OWNER_DOS
        );
        assert_eq!(
            memory.read_word(mcb_addr(MEMORY_TOP_SEGMENT) + MCB_OFF_SIZE),
            UMB_FIRST_MCB_SEGMENT - MEMORY_TOP_SEGMENT - 1
        );
        assert_eq!(
            memory.read_byte(mcb_addr(MEMORY_TOP_SEGMENT) + MCB_OFF_NAME),
            b'C'
        );
        assert_eq!(
            memory.read_byte(mcb_addr(MEMORY_TOP_SEGMENT) + MCB_OFF_NAME + 1),
            b'S'
        );

        cpu.set_ax(0x0003);
        cpu.set_bx(0);
        dos.int21h_58h_allocation_strategy(&mut cpu, &mut memory);

        assert!(!dos.state.umb_link);
        assert!(!iret_carry(&memory, &cpu));
        assert_eq!(
            memory.read_byte(mcb_addr(FREE_MCB_SEGMENT) + MCB_OFF_TYPE),
            0x5A
        );
    }

    #[test]
    fn int21h_4ah_requires_linked_umb_and_sets_bx_to_max_on_invalid_block() {
        let (mut dos, mut memory) = prepare_dos_with_umb();
        let mut cpu = MockCpu::default();
        let segment = dos
            .state
            .memory_manager
            .as_ref()
            .expect("memory manager should exist")
            .umb_allocate(0x10, &mut memory)
            .expect("UMB allocation should succeed")
            .0;

        let first_mcb = memory.read_word(SYSVARS_BASE - 2);
        let expected_max = memory::largest_available_dos(&memory, first_mcb, None, 0);

        cpu.set_es(segment);
        cpu.set_bx(0x0008);
        dos.int21h_4ah_resize(&mut cpu, &mut memory);

        assert!(iret_carry(&memory, &cpu));
        assert_eq!(cpu.ax(), 0x0009);
        assert_eq!(cpu.bx(), expected_max);

        cpu.set_ax(0x0003);
        cpu.set_bx(1);
        dos.int21h_58h_allocation_strategy(&mut cpu, &mut memory);

        cpu.set_es(segment);
        cpu.set_bx(0x0008);
        dos.int21h_4ah_resize(&mut cpu, &mut memory);

        assert!(!iret_carry(&memory, &cpu));

        let first_mcb_after = memory.read_word(SYSVARS_BASE - 2);
        let umb_first = Some(UMB_FIRST_MCB_SEGMENT);
        let expected_max_linked =
            memory::largest_available_dos(&memory, first_mcb_after, umb_first, 0);

        cpu.set_es(0xEEEE);
        cpu.set_bx(0x1234);
        dos.int21h_4ah_resize(&mut cpu, &mut memory);

        assert!(iret_carry(&memory, &cpu));
        assert_eq!(cpu.ax(), 0x0009);
        assert_eq!(cpu.bx(), expected_max_linked);
        assert_ne!(cpu.bx(), 0x1234);
    }

    #[test]
    fn int21h_4ah_sets_bx_on_mcb_destroyed_error() {
        let (dos, mut memory) = prepare_dos_with_umb();
        let mut cpu = MockCpu::default();

        let first_mcb = memory.read_word(SYSVARS_BASE - 2);
        memory.write_byte((first_mcb as u32) << 4, 0x42);

        cpu.set_es(first_mcb + 1);
        cpu.set_bx(0xAAAA);
        dos.int21h_4ah_resize(&mut cpu, &mut memory);

        assert!(iret_carry(&memory, &cpu));
        assert_eq!(cpu.ax(), 0x0007);
        assert_ne!(cpu.bx(), 0xAAAA);
    }

    #[test]
    fn int21h_4ah_sets_bx_on_insufficient_memory_error() {
        let (mut dos, mut memory) = prepare_dos_with_umb();
        let mut cpu = MockCpu::default();

        let mm = dos
            .state
            .memory_manager
            .as_mut()
            .expect("memory manager should exist");
        let (alloc_segment, _) = mm
            .umb_allocate(0x10, &mut memory)
            .expect("UMB allocation should succeed");

        cpu.set_ax(0x0003);
        cpu.set_bx(1);
        dos.int21h_58h_allocation_strategy(&mut cpu, &mut memory);
        assert!(!iret_carry(&memory, &cpu));

        cpu.set_es(alloc_segment);
        cpu.set_bx(0xFFFF);
        dos.int21h_4ah_resize(&mut cpu, &mut memory);

        assert!(iret_carry(&memory, &cpu));
        assert_eq!(cpu.ax(), 0x0008);
        assert_ne!(cpu.bx(), 0xFFFF);
    }
}
