//! INT DCh: NEC DOS Extension (IO.SYS replacement).
//!
//! Dispatched by the CL register with subfunctions in AX. This is a NEC-only
//! interrupt with no IBM PC equivalent. NEETAN DOS provides this handler since
//! it replaces IO.SYS.

use common::warn;

use crate::{
    CpuAccess, DiskIo, MemoryAccess, NeetanDos, SegmentRegister,
    drive_map::{self, pc98 as pc98_drives},
    filesystem::fat_partition,
    tables, version_gate,
};

/// Machine type of CL=12h for a PC-98 in normal mode.
const MACHINE_TYPE_NORMAL_MODE: u16 = 0x0003;

/// CL=80h subfunction that writes the disk information buffer.
const DISK_INFORMATION_SUBFUNCTION: u16 = 0x0000;

/// CL=80h subfunction that copies the console state of the work area.
const CONSOLE_STATE_SUBFUNCTION: u16 = 0x0001;

/// CL=80h subfunction that reports the BRANCH 4670 virtual drives.
const BRANCH_4670_SUBFUNCTION: u16 = 0x8000;

/// CL=80h subfunction that answers with the work area byte at `0060:27D2h`
/// of a NEC MS-DOS boot. The meaning of the byte is unknown.
const WORK_AREA_27D2_SUBFUNCTION: u16 = 0x8001;

/// CL=80h subfunction that reports the Japanese input FEP of MS-DOS.
const FEP_SUBFUNCTION: u16 = 0x8002;

/// CL=80h AX=8000h answer: no BRANCH 4670 virtual drive.
const BRANCH_4670_ABSENT: u16 = 0x0000;

/// CL=80h AX=8001h answer: the byte at `0060:27D2h` after a NEC MS-DOS boot.
const WORK_AREA_27D2_VALUE: u16 = 0x0000;

/// CL=80h AX=8002h answer: the Japanese input FEP of MS-DOS is absent.
const FEP_ABSENT: u16 = 0x0000;

/// Size of the buffer of CL=80h AX=0000h and AX=0001h.
const DISK_INFORMATION_BUFFER_SIZE: u32 = 0x100;

/// Fixed disk units with a partition count and partition start slots in the
/// CL=80h buffer (SASI and IDE #0 to #3).
const DISK_INFORMATION_FIXED_DISK_UNITS: usize = 4;

/// Buffer offset of the active partition count of fixed disk unit 0. The
/// counts of the units 1 to 3 follow.
const DISK_INFORMATION_OFF_PARTITION_COUNTS: u32 = 0x00;

/// Buffer offset of the number of 1 MB floppy drives.
const DISK_INFORMATION_OFF_FLOPPY_2HD_DRIVES: u32 = 0x0C;

/// Buffer offset of the number of 640 KB floppy drives.
const DISK_INFORMATION_OFF_FLOPPY_2DD_DRIVES: u32 = 0x0D;

/// Buffer offset of the number of fixed disk drives.
const DISK_INFORMATION_OFF_FIXED_DISK_DRIVES: u32 = 0x0E;

/// Buffer offset of the first physical sector of partition 1 of fixed disk
/// unit 0.
const DISK_INFORMATION_OFF_PARTITION_STARTS: u32 = 0x10;

/// Size of the partition start slots of one fixed disk unit (4 partitions).
const DISK_INFORMATION_UNIT_STARTS_SIZE: u32 = 0x10;

/// Bytes of the work area from `0060:0106h` that CL=80h AX=0001h copies.
const CONSOLE_STATE_SIZE: u32 = 0x1A;

/// CL=0Eh channel in the high nibble of `DL`: the built-in RS-232C port.
const RS232C_CHANNEL_BUILT_IN: u8 = 0x0;

/// CL=0Eh channel in the high nibble of `DL`: channel 1 of the RS-232C
/// extension board.
const RS232C_CHANNEL_EXTENSION_1: u8 = 0x1;

/// CL=0Eh channel in the high nibble of `DL`: channel 2 of the RS-232C
/// extension board.
const RS232C_CHANNEL_EXTENSION_2: u8 = 0x2;

/// CL=0Eh function in the low nibble of `DL`: length of the received data.
const RS232C_RECEIVE_LENGTH: u8 = 0x0;

/// CL=0Eh function in the low nibble of `DL`: initialize the port with the
/// protocol in `BX`.
const RS232C_INITIALIZE: u8 = 0x1;

/// CL=0Eh function in the low nibble of `DL`: read the protocol of the port.
const RS232C_READ_PROTOCOL: u8 = 0x6;

/// CL=0Eh function in the low nibble of `DL`: check for the RS-232C
/// extension board.
const RS232C_EXTENSION_BOARD_CHECK: u8 = 0x7;

/// CL=0Eh `AX` answer: the function succeeded, or the receive buffer is
/// empty.
const RS232C_SUCCESS: u16 = 0x0000;

/// CL=0Eh `AX` answer: no RS-232C extension board is installed.
const RS232C_NO_EXTENSION_BOARD: u16 = 0xFFFF;

/// Bits of the AUX protocol word that hold the speed.
const AUX_PROTOCOL_SPEED_MASK: u16 = 0x0F00;

/// Bits of the AUX protocol word that CL=0Eh DL=01h keeps.
const AUX_PROTOCOL_KEPT_MASK: u16 = 0xF000;

impl NeetanDos {
    /// Dispatches an INT DCh call based on the CL register.
    pub(crate) fn intdch(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        disk: &mut dyn DiskIo,
    ) {
        let cl = (cpu.cx() & 0xFF) as u8;
        if let Some(answer) = version_gate::intdch_answer(&self.state.platform, cl, cpu.ax()) {
            self.state.answer_predated_function(cpu, memory, answer);
            return;
        }
        match cl {
            // CL=0Ah initializes the RS-232C port through RSDRV.SYS. Without
            // the driver the call changes nothing.
            0x00..=0x08 | 0x0A => {}
            0x0C => self.intdch_0ch_read_fnkey_map(cpu, memory),
            0x0D => self.intdch_0dh_write_fnkey_map(cpu, memory),
            0x0E => self.intdch_0eh_rs232c(cpu, memory),
            0x0F => self.intdch_0fh_softkey_control(cpu, memory),
            0x10 => self.intdch_10h_console(cpu, memory),
            0x12 => self.intdch_12h_system_identification(cpu),
            0x13 => self.intdch_13h_daua_mapping_buffer(cpu, memory),
            0x15 => self.intdch_15h_internal_revision(cpu, memory),
            0x80 => self.intdch_80h_disk_information(cpu, memory, disk),
            0x81 => self.intdch_81h_extended_memory_query(cpu, memory),
            _ => warn!("INT DCh CL={cl:#04X} is unimplemented"),
        }
    }

    /// CL=10h: Console display subfunctions (dispatched by AH).
    fn intdch_10h_console(&mut self, cpu: &mut dyn CpuAccess, memory: &mut dyn MemoryAccess) {
        let ah = (cpu.ax() >> 8) as u8;
        match ah {
            0x00 => {
                // Single character output.
                let dl = (cpu.dx() & 0xFF) as u8;
                self.console.process_byte(memory, dl);
            }
            0x01 => {
                // String display. DS:DX = string, BX = length.
                let addr = cpu.linear_address(SegmentRegister::DS, cpu.dx());
                let length = cpu.bx() as u32;
                for i in 0..length {
                    let byte = memory.read_byte(addr + i);
                    self.console.process_byte(memory, byte);
                }
            }
            0x02 => {
                // Set attribute.
                let dl = (cpu.dx() & 0xFF) as u8;
                self.console.set_attribute(memory, dl);
            }
            0x03 => {
                // Cursor positioning. DH = row, DL = column.
                let dh = (cpu.dx() >> 8) as u8;
                let dl = (cpu.dx() & 0xFF) as u8;
                self.console.set_cursor_position(memory, dh, dl);
            }
            0x04 => {
                // Cursor down 1 line (with scroll at bottom).
                self.console.linefeed(memory);
            }
            0x05 => {
                // Cursor up 1 line (with scroll at top).
                self.console.reverse_linefeed(memory);
            }
            0x06 => {
                // Cursor up N lines (clamp, no scroll).
                let dl = (cpu.dx() & 0xFF) as u8;
                self.console.cursor_up(memory, dl.max(1));
            }
            0x07 => {
                // Cursor down N lines (clamp, no scroll).
                let dl = (cpu.dx() & 0xFF) as u8;
                self.console.cursor_down(memory, dl.max(1));
            }
            0x08 => {
                // Cursor right N columns (clamp).
                let dl = (cpu.dx() & 0xFF) as u8;
                self.console.cursor_right(memory, dl.max(1));
            }
            0x09 => {
                // Cursor left N columns (clamp).
                let dl = (cpu.dx() & 0xFF) as u8;
                self.console.cursor_left(memory, dl.max(1));
            }
            0x0A => {
                // Erase in display.
                let dl = (cpu.dx() & 0xFF) as u8;
                if dl == 2 {
                    self.console.clear_screen(memory);
                }
            }
            0x0B => {
                // Erase in line.
                let dl = (cpu.dx() & 0xFF) as u8;
                match dl {
                    0 => self.console.clear_line_from_cursor(memory),
                    1 => self.console.clear_line_to_cursor(memory),
                    2 => self.console.clear_line(memory),
                    _ => {}
                }
            }
            0x0C => {
                // Insert lines at the cursor row.
                let dl = (cpu.dx() & 0xFF) as u8;
                self.console.insert_lines(memory, dl.max(1));
            }
            0x0D => {
                // Delete lines at the cursor row.
                let dl = (cpu.dx() & 0xFF) as u8;
                self.console.delete_lines(memory, dl.max(1));
            }
            0x0E => {
                // Kanji/graph mode switching.
                let dl = (cpu.dx() & 0xFF) as u8;
                match dl {
                    0 => {
                        memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_KANJI_MODE, 0x01);
                        memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_GRAPH_CHAR, 0x20);
                    }
                    3 => {
                        memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_KANJI_MODE, 0x00);
                        memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_GRAPH_CHAR, 0x67);
                    }
                    _ => {}
                }
            }
            _ => warn!("INT DCh CL=10h AH={ah:#04X} is unimplemented"),
        }
    }

    /// CL=12h: System identification.
    /// Returns AX = product number of the DOS version, DX = machine type.
    fn intdch_12h_system_identification(&self, cpu: &mut dyn CpuAccess) {
        cpu.set_ax(self.state.iosys_identity().intdch_product_number);
        cpu.set_dx(MACHINE_TYPE_NORMAL_MODE);
    }

    /// CL=13h: Fill 96-byte DA/UA mapping buffer at DS:DX.
    fn intdch_13h_daua_mapping_buffer(&self, cpu: &dyn CpuAccess, memory: &mut dyn MemoryAccess) {
        let buffer_addr = cpu.linear_address(SegmentRegister::DS, cpu.dx());
        let base = tables::IOSYS_BASE;

        // +00h-0Fh: the DA/UA table of the drives A: to P:.
        memory.write_block(
            buffer_addr,
            &drive_map::pc98::daua_table(&self.state.drive_map),
        );

        // +10h-19h: 10 bytes reserved (zero).
        for i in 0x10..0x1Au32 {
            memory.write_byte(buffer_addr + i, 0x00);
        }

        // +1Ah-4Dh: the extended DA/UA table of all drive letters.
        memory.write_block(
            buffer_addr + 0x1A,
            &drive_map::pc98::extended_daua_table(&self.state.drive_map),
        );

        // +4Eh: FD logical drive duplicate flag (0060:0038h).
        let fd_dup = memory.read_byte(base + tables::IOSYS_OFF_FD_DUPLICATE);
        memory.write_byte(buffer_addr + 0x4E, fd_dup);

        // +4Fh: FD logical drive duplicate flag (0060:013Bh).
        let fd_dup2 = memory.read_byte(base + tables::IOSYS_OFF_FD_DUPLICATE2);
        memory.write_byte(buffer_addr + 0x4F, fd_dup2);

        // +50h: Last accessed drive number (0060:0136h).
        let last_drive = memory.read_byte(base + tables::IOSYS_OFF_LAST_DRIVE_UNIT);
        memory.write_byte(buffer_addr + 0x50, last_drive);

        // +51h-5Fh: 15 bytes reserved (zero).
        for i in 0x51..0x60u32 {
            memory.write_byte(buffer_addr + i, 0x00);
        }
    }

    /// CL=15h: Internal IO.SYS revision.
    /// Returns AL = revision byte from 0060:0022h.
    fn intdch_15h_internal_revision(&self, cpu: &mut dyn CpuAccess, memory: &dyn MemoryAccess) {
        let revision = memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_INTERNAL_REVISION);
        cpu.set_ax((cpu.ax() & 0xFF00) | revision as u16);
    }

    /// CL=80h: Disk information of the `IO.SYS`. AX=0000h writes the drive
    /// counts and the partition starts into the 100h byte buffer at DS:DX.
    /// AX=0001h writes the console state of the work area into the buffer.
    /// AX=8000h-8002h return the state of the BRANCH 4670 drives, the byte at
    /// `0060:27D2h` and the state of the FEP in AX. Every other register keeps
    /// its value.
    fn intdch_80h_disk_information(
        &self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        disk: &mut dyn DiskIo,
    ) {
        match cpu.ax() {
            DISK_INFORMATION_SUBFUNCTION => {
                let buffer = cpu.linear_address(SegmentRegister::DS, cpu.dx());
                self.write_disk_information(memory, buffer, disk);
            }
            CONSOLE_STATE_SUBFUNCTION => {
                let buffer = cpu.linear_address(SegmentRegister::DS, cpu.dx());
                clear_disk_information_buffer(memory, buffer);
                let state = tables::IOSYS_BASE + tables::IOSYS_OFF_SPECIAL_INPUT;
                for offset in 0..CONSOLE_STATE_SIZE {
                    memory.write_byte(buffer + offset, memory.read_byte(state + offset));
                }
            }
            BRANCH_4670_SUBFUNCTION => cpu.set_ax(BRANCH_4670_ABSENT),
            WORK_AREA_27D2_SUBFUNCTION => cpu.set_ax(WORK_AREA_27D2_VALUE),
            FEP_SUBFUNCTION => cpu.set_ax(FEP_ABSENT),
            _ => {}
        }
    }

    /// Writes the CL=80h AX=0000h buffer at `buffer`: the partition count of
    /// each fixed disk unit, the drive count of each drive class and the
    /// partition start of each fixed disk unit. The drive map holds one
    /// volume per fixed disk unit: the first active DOS partition. A unit
    /// without an active DOS partition counts no partition and no drive.
    fn write_disk_information(
        &self,
        memory: &mut dyn MemoryAccess,
        buffer: u32,
        disk: &mut dyn DiskIo,
    ) {
        let mut floppy_2hd_drives = 0u8;
        let mut floppy_2dd_drives = 0u8;
        let mut partition_starts = [None; DISK_INFORMATION_FIXED_DISK_UNITS];
        for drive_index in 0..drive_map::DRIVE_LETTER_COUNT as u8 {
            let Some(entry) = self.state.drive_map.entry(drive_index) else {
                continue;
            };
            match entry.device {
                drive_map::DriveDevice::Floppy { device_id } => {
                    match pc98_drives::device_class(device_id) {
                        pc98_drives::FLOPPY_2HD_CLASS => floppy_2hd_drives += 1,
                        pc98_drives::FLOPPY_2DD_CLASS => floppy_2dd_drives += 1,
                        _ => {}
                    }
                }
                drive_map::DriveDevice::FixedDisk { device_id } => {
                    let unit = usize::from(device_id & pc98_drives::UNIT_MASK);
                    if let Some(slot) = partition_starts.get_mut(unit) {
                        *slot = active_partition_start(entry.partition_scheme, device_id, disk);
                    }
                }
                drive_map::DriveDevice::MemoryImage { .. }
                | drive_map::DriveDevice::Cdrom
                | drive_map::DriveDevice::GuestDriver(_)
                | drive_map::DriveDevice::GuestCdrom(_)
                | drive_map::DriveDevice::Virtual => {}
            }
        }

        clear_disk_information_buffer(memory, buffer);
        let mut fixed_disk_drives = 0u8;
        for (unit, start) in partition_starts.iter().enumerate() {
            let Some(start) = start else {
                continue;
            };
            fixed_disk_drives += 1;
            memory.write_byte(
                buffer + DISK_INFORMATION_OFF_PARTITION_COUNTS + unit as u32,
                1,
            );
            let slot = buffer
                + DISK_INFORMATION_OFF_PARTITION_STARTS
                + unit as u32 * DISK_INFORMATION_UNIT_STARTS_SIZE;
            memory.write_word(slot, *start as u16);
            memory.write_word(slot + 2, (*start >> 16) as u16);
        }
        memory.write_byte(
            buffer + DISK_INFORMATION_OFF_FLOPPY_2HD_DRIVES,
            floppy_2hd_drives,
        );
        memory.write_byte(
            buffer + DISK_INFORMATION_OFF_FLOPPY_2DD_DRIVES,
            floppy_2dd_drives,
        );
        memory.write_byte(
            buffer + DISK_INFORMATION_OFF_FIXED_DISK_DRIVES,
            fixed_disk_drives,
        );
    }

    /// CL=0Eh: RS-232C functions. The high nibble of `DL` selects the
    /// channel and the low nibble the function. On the built-in port DL=00h
    /// answers an empty receive buffer and DL=01h stores the parameters in
    /// `BH` and the speed in the low nibble of `BL` into the AUX protocol
    /// word at `0060:0068h`. Both return AX=0000h. DL=06h returns the AUX
    /// protocol in `BH` and its speed in `BL` with `AX` unchanged. DL=07h
    /// reports no extension board with AX=FFFFh. Every function of the
    /// channels 1 and 2 of the absent extension board returns AX=FFFFh. Other
    /// values of `DL` change no register. The RS-232C hardware keeps its
    /// state.
    fn intdch_0eh_rs232c(&self, cpu: &mut dyn CpuAccess, memory: &mut dyn MemoryAccess) {
        let dl = (cpu.dx() & 0xFF) as u8;
        let protocol_address = tables::IOSYS_BASE + tables::IOSYS_OFF_AUX_PROTOCOL;
        match dl >> 4 {
            RS232C_CHANNEL_BUILT_IN => match dl & 0x0F {
                RS232C_RECEIVE_LENGTH => cpu.set_ax(RS232C_SUCCESS),
                RS232C_INITIALIZE => {
                    let protocol = memory.read_word(protocol_address);
                    let parameters = cpu.bx() >> 8;
                    let speed = (cpu.bx() << 8) & AUX_PROTOCOL_SPEED_MASK;
                    memory.write_word(
                        protocol_address,
                        (protocol & AUX_PROTOCOL_KEPT_MASK) | speed | parameters,
                    );
                    cpu.set_ax(RS232C_SUCCESS);
                }
                RS232C_READ_PROTOCOL => {
                    let protocol = memory.read_word(protocol_address);
                    let speed = (protocol & AUX_PROTOCOL_SPEED_MASK) >> 8;
                    cpu.set_bx((protocol << 8) | speed);
                }
                RS232C_EXTENSION_BOARD_CHECK => cpu.set_ax(RS232C_NO_EXTENSION_BOARD),
                _ => {}
            },
            RS232C_CHANNEL_EXTENSION_1 | RS232C_CHANNEL_EXTENSION_2 => {
                cpu.set_ax(RS232C_NO_EXTENSION_BOARD);
            }
            _ => {}
        }
    }

    /// CL=81h: Extended memory query.
    /// Returns AL = extended memory size in 128KB units from 0060:0031h.
    fn intdch_81h_extended_memory_query(&self, cpu: &mut dyn CpuAccess, memory: &dyn MemoryAccess) {
        let ext_mem = memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_EXT_MEM_128K);
        cpu.set_ax((cpu.ax() & 0xFF00) | ext_mem as u16);
    }

    /// CL=0Ch: Read programmable function key mapping.
    /// AX = key specifier, DS:DX = destination buffer.
    ///
    /// Internal fn_key_map uses the basic layout (386 bytes):
    ///   0-159: F1-F10 (10x16), 160-319: Shift+F1-F10 (10x16), 320-385: editing (11x6).
    /// AX=0xFF uses the extended layout (786 bytes) which inserts VF and Ctrl+F
    /// groups between the basic groups.
    fn intdch_0ch_read_fnkey_map(&self, cpu: &dyn CpuAccess, memory: &mut dyn MemoryAccess) {
        let key_specifier = cpu.ax();
        let buffer_addr = cpu.linear_address(SegmentRegister::DS, cpu.dx());

        if key_specifier == 0x00FF {
            // Extended format: repack basic layout into extended wire format.
            // F1-F10 (160 bytes) at ext offset 0
            self.copy_fnkey_to_buffer(memory, buffer_addr, 0, 0, 160);
            // VF1-VF5 (80 bytes, zeros) at ext offset 160
            self.zero_buffer(memory, buffer_addr, 160, 80);
            // Shift+F1-F10 (160 bytes) at ext offset 240
            self.copy_fnkey_to_buffer(memory, buffer_addr, 240, 160, 160);
            // Shift+VF1-VF5 (80 bytes, zeros) at ext offset 400
            self.zero_buffer(memory, buffer_addr, 400, 80);
            // Editing keys (66 bytes) at ext offset 480
            self.copy_fnkey_to_buffer(memory, buffer_addr, 480, 320, 66);
            // Ctrl+F1-F10 (160 bytes, zeros) at ext offset 546
            self.zero_buffer(memory, buffer_addr, 546, 160);
            // Ctrl+VF1-VF5 (80 bytes, zeros) at ext offset 706
            self.zero_buffer(memory, buffer_addr, 706, 80);
            return;
        }

        let (src_offset, length) = match key_specifier {
            0x0000 => (0, 386),
            0x0001..=0x000A => {
                let idx = (key_specifier - 1) as usize;
                (idx * 16, 16)
            }
            0x000B..=0x0014 => {
                let idx = (key_specifier - 0x000B) as usize;
                (160 + idx * 16, 16)
            }
            0x0015..=0x001F => {
                let idx = (key_specifier - 0x0015) as usize;
                (320 + idx * 6, 6)
            }
            _ => return,
        };

        self.copy_fnkey_to_buffer(memory, buffer_addr, 0, src_offset, length);
    }

    fn copy_fnkey_to_buffer(
        &self,
        memory: &mut dyn MemoryAccess,
        buffer_addr: u32,
        buf_offset: usize,
        map_offset: usize,
        length: usize,
    ) {
        for i in 0..length {
            let byte = self
                .state
                .fn_key_map
                .get(map_offset + i)
                .copied()
                .unwrap_or(0);
            memory.write_byte(buffer_addr + (buf_offset + i) as u32, byte);
        }
    }

    fn zero_buffer(
        &self,
        memory: &mut dyn MemoryAccess,
        buffer_addr: u32,
        buf_offset: usize,
        length: usize,
    ) {
        for i in 0..length {
            memory.write_byte(buffer_addr + (buf_offset + i) as u32, 0);
        }
    }

    /// CL=0Fh: CTRL+function key soft-key control.
    /// AX = subfunction: 0000h/0001h set/clear CTRL+Fn,
    /// 0002h/0003h set/clear CTRL+XFER/NFER,
    /// 8000h/8002h read state.
    fn intdch_0fh_softkey_control(&self, cpu: &mut dyn CpuAccess, memory: &mut dyn MemoryAccess) {
        let ax = cpu.ax();
        let addr = tables::IOSYS_BASE + tables::IOSYS_OFF_SOFTKEY_FLAGS;
        let flags = memory.read_byte(addr);
        match ax {
            0x0000 => memory.write_byte(addr, flags | 0x01),
            0x0001 => memory.write_byte(addr, flags & !0x01),
            0x0002 => memory.write_byte(addr, flags | 0x02),
            0x0003 => memory.write_byte(addr, flags & !0x02),
            0x8000 => cpu.set_ax(if flags & 0x01 != 0 { 0x0000 } else { 0x0001 }),
            0x8002 => cpu.set_ax(if flags & 0x02 != 0 { 0x0000 } else { 0x0001 }),
            _ => {}
        }
    }

    /// CL=0Dh: Write programmable function key mapping.
    /// AX = key specifier, DS:DX = source buffer.
    fn intdch_0dh_write_fnkey_map(&mut self, cpu: &dyn CpuAccess, memory: &dyn MemoryAccess) {
        let key_specifier = cpu.ax();
        let buffer_addr = cpu.linear_address(SegmentRegister::DS, cpu.dx());

        if key_specifier == 0x00FF {
            // Extended format: unpack extended wire format into basic layout.
            // F1-F10 (160 bytes) from ext offset 0
            self.copy_buffer_to_fnkey(memory, buffer_addr, 0, 0, 160);
            // Skip VF1-VF5 at ext offset 160 (80 bytes)
            // Shift+F1-F10 (160 bytes) from ext offset 240
            self.copy_buffer_to_fnkey(memory, buffer_addr, 240, 160, 160);
            // Skip Shift+VF1-VF5 at ext offset 400 (80 bytes)
            // Editing keys (66 bytes) from ext offset 480
            self.copy_buffer_to_fnkey(memory, buffer_addr, 480, 320, 66);
            // Skip Ctrl+F1-F10 at ext offset 546 and Ctrl+VF1-VF5 at ext offset 706
            return;
        }

        let (dst_offset, length) = match key_specifier {
            0x0000 => (0, 386),
            0x0001..=0x000A => {
                let idx = (key_specifier - 1) as usize;
                (idx * 16, 16)
            }
            0x000B..=0x0014 => {
                let idx = (key_specifier - 0x000B) as usize;
                (160 + idx * 16, 16)
            }
            0x0015..=0x001F => {
                let idx = (key_specifier - 0x0015) as usize;
                (320 + idx * 6, 6)
            }
            _ => return,
        };

        for i in 0..length {
            let byte = memory.read_byte(buffer_addr + i as u32);
            if let Some(dest) = self.state.fn_key_map.get_mut(dst_offset + i) {
                *dest = byte;
            }
        }
    }

    fn copy_buffer_to_fnkey(
        &mut self,
        memory: &dyn MemoryAccess,
        buffer_addr: u32,
        buf_offset: usize,
        map_offset: usize,
        length: usize,
    ) {
        for i in 0..length {
            let byte = memory.read_byte(buffer_addr + (buf_offset + i) as u32);
            if let Some(dest) = self.state.fn_key_map.get_mut(map_offset + i) {
                *dest = byte;
            }
        }
    }
}

/// Writes zeros into the 100h byte buffer of CL=80h at `buffer`.
fn clear_disk_information_buffer(memory: &mut dyn MemoryAccess, buffer: u32) {
    for offset in 0..DISK_INFORMATION_BUFFER_SIZE {
        memory.write_byte(buffer + offset, 0);
    }
}

/// Returns the first physical sector of the first active DOS partition of
/// the fixed disk `device_id`, or `None` when the disk has none.
fn active_partition_start(
    partition_scheme: drive_map::PartitionScheme,
    device_id: u8,
    disk: &mut dyn DiskIo,
) -> Option<u32> {
    match partition_scheme {
        drive_map::PartitionScheme::Pc98 => {
            fat_partition::find_active_partition_start(device_id, disk)
                .ok()
                .flatten()
        }
        drive_map::PartitionScheme::Unpartitioned
        | drive_map::PartitionScheme::MasterBootRecord
        | drive_map::PartitionScheme::FmTowns { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use common::HleDosVersion;

    use super::*;
    use crate::{
        DosPlatform,
        test_support::{MockCpu, MockMemory},
    };

    /// Returns a PC-98 NeetanDos with the IO.SYS work area in 1 MiB of memory.
    fn pc98_dos_with_work_area() -> (NeetanDos, MockMemory) {
        pc98_dos_of_version(HleDosVersion::Dos620)
    }

    /// Returns a PC-98 NeetanDos of `version` with the IO.SYS work area in
    /// 1 MiB of memory and the CTRL+function key flags set.
    fn pc98_dos_of_version(version: HleDosVersion) -> (NeetanDos, MockMemory) {
        let dos = NeetanDos::new(DosPlatform::pc98(version));
        let mut memory = MockMemory::with_extended_memory(0x10_0000, 0);
        dos.write_iosys_work_area(&mut memory);
        memory.write_byte(
            tables::IOSYS_BASE + tables::IOSYS_OFF_SOFTKEY_FLAGS,
            SOFTKEY_FLAGS_SET,
        );
        (dos, memory)
    }

    /// CTRL+function key and CTRL+XFER/NFER flags both set.
    const SOFTKEY_FLAGS_SET: u8 = 0x03;

    /// `AX`, `BX` and `DX` in front of a gated call.
    const GATE_PRESETS: [u16; 3] = [0xAB00, 0x1234, 0x5678];

    /// Calls INT DCh with `cl` and `ax` and the presets in `BX` and `DX`.
    /// Returns `[AX, BX, CX, DX]` after the call.
    fn call_intdch(version: HleDosVersion, cl: u8, ax: u16) -> [u16; 4] {
        let (mut dos, mut memory) = pc98_dos_of_version(version);
        let mut cpu = MockCpu::default();
        cpu.set_ax(ax);
        cpu.set_bx(GATE_PRESETS[1]);
        cpu.set_cx(u16::from(cl));
        cpu.set_dx(GATE_PRESETS[2]);
        dos.intdch(&mut cpu, &mut memory, &mut crate::test_support::NoDrives);
        [cpu.ax(), cpu.bx(), cpu.cx(), cpu.dx()]
    }

    #[test]
    fn dos310_cl_12h_returns_the_ps98_011_product_number() {
        assert_eq!(
            call_intdch(HleDosVersion::Dos310, 0x12, 0x0000),
            [0x0001, GATE_PRESETS[1], 0x0012, 0x0003]
        );
    }

    #[test]
    fn dos310_leaves_the_younger_functions_unanswered() {
        for (cl, ax) in [
            (0x0F, 0x8000),
            (0x0F, 0x8002),
            (0x15, GATE_PRESETS[0]),
            (0x81, GATE_PRESETS[0]),
        ] {
            assert_eq!(
                call_intdch(HleDosVersion::Dos310, cl, ax),
                [ax, GATE_PRESETS[1], u16::from(cl), GATE_PRESETS[2]],
                "CL={cl:02X}h AX={ax:04X}h"
            );
        }
    }

    #[test]
    fn dos620_answers_the_functions_that_dos310_leaves_unanswered() {
        assert_eq!(call_intdch(HleDosVersion::Dos620, 0x0F, 0x8000)[0], 0x0000);
        assert_eq!(call_intdch(HleDosVersion::Dos620, 0x0F, 0x8002)[0], 0x0000);
        assert_eq!(
            call_intdch(HleDosVersion::Dos620, 0x15, GATE_PRESETS[0])[0],
            0xAB01
        );
        assert_eq!(
            call_intdch(HleDosVersion::Dos620, 0x81, GATE_PRESETS[0])[0],
            0xAB00
        );
    }

    #[test]
    fn dos310_keeps_the_ctrl_function_key_settings() {
        let (mut dos, mut memory) = pc98_dos_of_version(HleDosVersion::Dos310);
        let mut cpu = MockCpu::default();
        cpu.set_ax(0x0001);
        cpu.set_cx(0x000F);
        dos.intdch(&mut cpu, &mut memory, &mut crate::test_support::NoDrives);
        assert_eq!(
            memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_SOFTKEY_FLAGS),
            SOFTKEY_FLAGS_SET & !0x01
        );
    }

    #[test]
    fn work_area_holds_the_product_number_of_the_version() {
        let (_, memory) = pc98_dos_of_version(HleDosVersion::Dos310);
        let product = tables::IOSYS_BASE + tables::IOSYS_OFF_PRODUCT_NUMBER;
        let revision = tables::IOSYS_BASE + tables::IOSYS_OFF_INTERNAL_REVISION;
        assert_eq!(memory.read_word(product), 0x0000);
        assert_eq!(memory.read_byte(revision), 0x00);
        let (_, memory) = pc98_dos_of_version(HleDosVersion::Dos620);
        assert_eq!(memory.read_word(product), 0x0200);
        assert_eq!(memory.read_byte(revision), 0x01);
    }

    #[test]
    fn drive_table_pointer_points_to_the_table_of_all_drives() {
        for version in [HleDosVersion::Dos310, HleDosVersion::Dos620] {
            let (_, memory) = pc98_dos_of_version(version);
            let pointer = tables::IOSYS_BASE + tables::IOSYS_OFF_DAUA_PTR;
            assert_eq!(memory.read_word(pointer), 0x2C86, "{version:?}");
            assert_eq!(memory.read_word(pointer + 2), 0x0060, "{version:?}");
        }
    }

    #[test]
    fn cl_12h_returns_the_product_number_and_the_machine_type() {
        let (mut dos, mut memory) = pc98_dos_with_work_area();
        let mut cpu = MockCpu::default();
        cpu.set_ax(0x0000);
        cpu.set_bx(0x0000);
        cpu.set_cx(0x0012);
        dos.intdch(&mut cpu, &mut memory, &mut crate::test_support::NoDrives);
        assert_eq!(cpu.ax(), 0x0200);
        assert_eq!(cpu.bx(), 0x0000);
        assert_eq!(cpu.cx(), 0x0012);
        assert_eq!(cpu.dx(), 0x0003);
    }

    /// AUX protocol word at `0060:0068h` in front of a CL=0Eh call.
    const AUX_PROTOCOL_PRESET: u16 = 0xF74D;

    /// Calls INT DCh CL=0Eh with `bx` and `dx`, `AX` from the presets and the
    /// AUX protocol preset. Returns `[AX, BX, CX, DX]` and the AUX protocol
    /// after the call.
    fn call_rs232c(version: HleDosVersion, bx: u16, dx: u16) -> ([u16; 4], u16) {
        let (mut dos, mut memory) = pc98_dos_of_version(version);
        let protocol = tables::IOSYS_BASE + tables::IOSYS_OFF_AUX_PROTOCOL;
        memory.write_word(protocol, AUX_PROTOCOL_PRESET);
        let mut cpu = MockCpu::default();
        cpu.set_ax(GATE_PRESETS[0]);
        cpu.set_bx(bx);
        cpu.set_cx(0x000E);
        cpu.set_dx(dx);
        dos.intdch(&mut cpu, &mut memory, &mut crate::test_support::NoDrives);
        (
            [cpu.ax(), cpu.bx(), cpu.cx(), cpu.dx()],
            memory.read_word(protocol),
        )
    }

    #[test]
    fn cl_0eh_initialize_keeps_the_high_nibble_of_the_aux_protocol() {
        assert_eq!(
            call_rs232c(HleDosVersion::Dos620, 0x4EFA, 0x0001),
            ([0x0000, 0x4EFA, 0x000E, 0x0001], 0xFA4E)
        );
    }

    #[test]
    fn cl_0eh_read_protocol_masks_the_speed() {
        assert_eq!(
            call_rs232c(HleDosVersion::Dos620, GATE_PRESETS[1], 0x0006),
            (
                [GATE_PRESETS[0], 0x4D07, 0x000E, 0x0006],
                AUX_PROTOCOL_PRESET
            )
        );
    }

    #[test]
    fn dos310_cl_0eh_changes_no_register() {
        for dx in [0x0000, 0x0001, 0x0006, 0x0007, 0x0011] {
            assert_eq!(
                call_rs232c(HleDosVersion::Dos310, GATE_PRESETS[1], dx),
                (
                    [GATE_PRESETS[0], GATE_PRESETS[1], 0x000E, dx],
                    AUX_PROTOCOL_PRESET
                ),
                "DX={dx:04X}h"
            );
        }
    }

    #[test]
    fn cl_15h_returns_the_internal_revision_in_al() {
        let (mut dos, mut memory) = pc98_dos_with_work_area();
        let mut cpu = MockCpu::default();
        cpu.set_ax(0xAB00);
        cpu.set_cx(0x0015);
        dos.intdch(&mut cpu, &mut memory, &mut crate::test_support::NoDrives);
        assert_eq!(cpu.ax(), 0xAB01);
    }
}
