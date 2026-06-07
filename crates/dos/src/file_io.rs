//! INT 21h file I/O function implementations.

use common::warn;

use crate::{
    CpuAccess, DiskIo, DosState, DriveIo, MemoryAccess, NeetanDos, SegmentRegister,
    filesystem::{
        self, FatHandleMetadata, ReadDirEntrySource, ReadDirectory, fat_dir,
        fat_file::FatFileCursor, find_matching_read_entry, find_read_entry, iso9660, split_path,
        virtual_drive::VirtualEntry,
    },
    set_iret_carry, tables,
};

/// Writes a 32-bit value to emulated memory.
fn write_dword(mem: &mut dyn MemoryAccess, addr: u32, value: u32) {
    mem.write_word(addr, value as u16);
    mem.write_word(addr + 2, (value >> 16) as u16);
}

/// Reads a 32-bit value from emulated memory.
fn read_dword(mem: &dyn MemoryAccess, addr: u32) -> u32 {
    mem.read_word(addr) as u32 | ((mem.read_word(addr + 2) as u32) << 16)
}

fn path_is_named_device(path: &[u8], device_name: &[u8; 8]) -> bool {
    let mut component = path;
    if component.len() >= 2 && component[1] == b':' {
        component = &component[2..];
    }
    if let Some(last_separator) = component
        .iter()
        .rposition(|byte| *byte == b'\\' || *byte == b'/')
    {
        component = &component[last_separator + 1..];
    }
    if let Some(extension) = component.iter().position(|byte| *byte == b'.') {
        component = &component[..extension];
    }
    if component.last() == Some(&b':') {
        component = &component[..component.len() - 1];
    }
    if component.is_empty() || component.len() > 8 {
        return false;
    }
    let mut padded = [b' '; 8];
    padded[..component.len()].copy_from_slice(component);
    padded.make_ascii_uppercase();
    &padded == device_name
}

struct CharacterDeviceOpen {
    sft_name: &'static [u8; 11],
    device_offset: u16,
    device_info: u16,
}

fn path_character_device(path: &[u8]) -> Option<CharacterDeviceOpen> {
    // Real DOS 6.20 sets bit 15 (mirror of DEVATTR_CHAR) and bit 6 (no-EOF
    // / binary mode) on every character-device SFT entry alongside bit 7
    // (CHAR) and the per-device low-byte flags.
    const CHAR_DEVICE_COMMON: u16 =
        tables::SFT_DEVINFO_DRIVER_CHAR | tables::SFT_DEVINFO_CHAR | tables::SFT_DEVINFO_EOF;
    if path_is_named_device(path, b"CON     ") {
        return Some(CharacterDeviceOpen {
            sft_name: b"CON        ",
            device_offset: tables::DEV_CON_OFFSET,
            device_info: CHAR_DEVICE_COMMON
                | tables::SFT_DEVINFO_SPECIAL
                | tables::SFT_DEVINFO_STDIN
                | tables::SFT_DEVINFO_STDOUT,
        });
    }
    if path_is_named_device(path, b"NUL     ") {
        return Some(CharacterDeviceOpen {
            sft_name: b"NUL        ",
            device_offset: tables::DEV_NUL_OFFSET,
            device_info: CHAR_DEVICE_COMMON | tables::SFT_DEVINFO_NUL,
        });
    }
    if path_is_named_device(path, b"AUX     ") {
        return Some(CharacterDeviceOpen {
            sft_name: b"AUX        ",
            device_offset: tables::DEV_NUL_OFFSET,
            device_info: CHAR_DEVICE_COMMON,
        });
    }
    if path_is_named_device(path, b"PRN     ") {
        return Some(CharacterDeviceOpen {
            sft_name: b"PRN        ",
            device_offset: tables::DEV_NUL_OFFSET,
            device_info: CHAR_DEVICE_COMMON,
        });
    }
    None
}

fn path_is_ems_device(path: &[u8]) -> bool {
    path_is_named_device(path, b"EMMXXXX0")
}

fn path_is_xms_device(path: &[u8]) -> bool {
    path_is_named_device(path, b"XMSXXXX0")
}

fn path_is_cdrom_device(path: &[u8], device_name: &[u8]) -> bool {
    let mut padded = [b' '; 8];
    let len = device_name.len().min(8);
    padded[..len].copy_from_slice(&device_name[..len]);
    path_is_named_device(path, &padded)
}

fn cdrom_sft_name(device_name: &[u8]) -> [u8; 11] {
    let mut name = [b' '; 11];
    let len = device_name.len().min(8);
    name[..len].copy_from_slice(&device_name[..len]);
    name
}

fn sft_is_cdrom_device(memory: &dyn MemoryAccess, sft_addr: u32) -> bool {
    memory.read_word(sft_addr + tables::SFT_ENT_DEV_PTR) == tables::DEV_CDROM_OFFSET
        && memory.read_word(sft_addr + tables::SFT_ENT_DEV_PTR + 2) == tables::DOS_DATA_SEGMENT
}

fn read_fat_handle_metadata(
    memory: &mut dyn MemoryAccess,
    sft_addr: u32,
    drive_index: u8,
) -> FatHandleMetadata {
    let dir_sector = memory.read_word(sft_addr + tables::SFT_ENT_DIR_SECTOR) as u32;
    let dir_index = memory.read_byte(sft_addr + tables::SFT_ENT_DIR_INDEX);
    let mut name = [0u8; 11];
    memory.read_block(sft_addr + tables::SFT_ENT_NAME, &mut name);

    FatHandleMetadata {
        drive_index,
        name,
        attribute: memory.read_byte(sft_addr + tables::SFT_ENT_FILE_ATTR),
        time: memory.read_word(sft_addr + tables::SFT_ENT_FILE_TIME),
        date: memory.read_word(sft_addr + tables::SFT_ENT_FILE_DATE),
        start_cluster: memory.read_word(sft_addr + tables::SFT_ENT_START_CLUSTER),
        file_size: read_dword(memory, sft_addr + tables::SFT_ENT_FILE_SIZE),
        position: read_dword(memory, sft_addr + tables::SFT_ENT_FILE_POS),
        dir_sector,
        dir_offset: dir_index as u16 * fat_dir::DIR_ENTRY_SIZE as u16,
    }
}

fn write_fat_handle_metadata(
    memory: &mut dyn MemoryAccess,
    sft_addr: u32,
    metadata: &FatHandleMetadata,
) {
    memory.write_byte(sft_addr + tables::SFT_ENT_FILE_ATTR, metadata.attribute);
    memory.write_word(
        sft_addr + tables::SFT_ENT_START_CLUSTER,
        metadata.start_cluster,
    );
    memory.write_word(sft_addr + tables::SFT_ENT_FILE_TIME, metadata.time);
    memory.write_word(sft_addr + tables::SFT_ENT_FILE_DATE, metadata.date);
    write_dword(
        memory,
        sft_addr + tables::SFT_ENT_FILE_SIZE,
        metadata.file_size,
    );
    write_dword(
        memory,
        sft_addr + tables::SFT_ENT_FILE_POS,
        metadata.position,
    );
    memory.write_word(
        sft_addr + tables::SFT_ENT_CUR_CLUSTER,
        metadata.start_cluster,
    );
    memory.write_word(
        sft_addr + tables::SFT_ENT_DIR_SECTOR,
        metadata.dir_sector as u16,
    );
    memory.write_byte(
        sft_addr + tables::SFT_ENT_DIR_INDEX,
        (metadata.dir_offset / fat_dir::DIR_ENTRY_SIZE as u16) as u8,
    );
    memory.write_block(sft_addr + tables::SFT_ENT_NAME, &metadata.name);
}

impl NeetanDos {
    /// AH=0Dh: Disk reset - flush all dirty FAT caches.
    pub(crate) fn int21h_0dh_disk_reset(&mut self, disk: &mut dyn DiskIo) {
        for vol in self.state.fat_volumes.iter_mut().flatten() {
            let _ = vol.flush_fat(disk);
        }
    }

    /// AH=1Ch: Get allocation information for specific drive.
    pub(crate) fn int21h_1ch_get_alloc_info(
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

        // Read DPB for the drive
        let dpb_ptr_addr = self.state.sysvars_base + tables::SYSVARS_OFF_DPB_PTR;
        let mut dpb_off = memory.read_word(dpb_ptr_addr);
        let mut dpb_seg = memory.read_word(dpb_ptr_addr + 2);
        let mut found = false;

        for _ in 0..26 {
            if dpb_seg == 0xFFFF && dpb_off == 0xFFFF {
                break;
            }
            let dpb_addr = ((dpb_seg as u32) << 4) + dpb_off as u32;
            let dpb_drive = memory.read_byte(dpb_addr + tables::DPB_OFF_DRIVE_NUM);
            if dpb_drive == drive_index {
                let spc = memory.read_byte(dpb_addr + tables::DPB_OFF_CLUSTER_MASK) as u16 + 1;
                let bps = memory.read_word(dpb_addr + tables::DPB_OFF_BYTES_PER_SECTOR);
                let max_cluster = memory.read_word(dpb_addr + tables::DPB_OFF_MAX_CLUSTER);
                let media = memory.read_byte(dpb_addr + tables::DPB_OFF_MEDIA_DESC);

                cpu.set_ax((cpu.ax() & 0xFF00) | spc);
                cpu.set_cx(bps);
                cpu.set_dx(max_cluster);
                cpu.set_bx(dpb_addr as u16); // DS:BX -> media byte (approximate)
                // Write media byte at DS:BX for callers that read it
                let ds_bx = cpu.linear_address(SegmentRegister::DS, cpu.bx());
                memory.write_byte(ds_bx, media);
                found = true;
                break;
            }
            // Follow chain
            let next_off = memory.read_word(dpb_addr + tables::DPB_OFF_NEXT_DPB);
            let next_seg = memory.read_word(dpb_addr + tables::DPB_OFF_NEXT_DPB + 2);
            dpb_off = next_off;
            dpb_seg = next_seg;
        }

        if !found {
            cpu.set_ax(0x00FF); // invalid drive
        }
    }

    /// AH=36h: Get free disk space.
    pub(crate) fn int21h_36h_get_free_disk_space(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &dyn MemoryAccess,
        disk: &mut dyn DriveIo,
    ) {
        let dl = (cpu.dx() & 0xFF) as u8;
        let drive_index = if dl == 0 {
            self.state.current_drive
        } else {
            dl - 1
        };

        if drive_index >= 26 {
            cpu.set_ax(0xFFFF);
            return;
        }

        if drive_index == 25 {
            cpu.set_ax(1);
            cpu.set_bx(0);
            cpu.set_cx(512);
            cpu.set_dx(0);
            return;
        }

        let cds_addr = tables::CDS_BASE + (drive_index as u32) * tables::CDS_ENTRY_SIZE;
        let cds_flags = memory.read_word(cds_addr + tables::CDS_OFF_FLAGS);
        if cds_flags == 0 {
            cpu.set_ax(0xFFFF);
            return;
        }

        if self.state.mscdex.drive_letter == drive_index
            && cds_flags & tables::CDS_FLAG_PHYSICAL == 0
        {
            cpu.set_ax(1);
            cpu.set_bx(0);
            cpu.set_cx(2048);
            cpu.set_dx(0);
            return;
        }

        if self
            .state
            .ensure_volume_mounted(drive_index, memory, disk)
            .is_err()
        {
            cpu.set_ax(0xFFFF);
            return;
        }

        let Some(volume) = self.state.fat_volumes[drive_index as usize].as_ref() else {
            cpu.set_ax(0xFFFF);
            return;
        };

        cpu.set_ax(volume.sectors_per_cluster());
        cpu.set_bx(volume.free_cluster_count());
        cpu.set_cx(volume.bytes_per_sector());
        cpu.set_dx(volume.total_cluster_count());
    }

    /// AH=29h: Parse filename into FCB.
    pub(crate) fn int21h_29h_parse_filename(
        &self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
    ) {
        let original_si_addr = cpu.linear_address(SegmentRegister::DS, cpu.si());
        let mut si_addr = original_si_addr;
        let di_addr = cpu.linear_address(SegmentRegister::ES, cpu.di());
        let al_flags = cpu.ax() as u8;

        // AL bit 0: skip leading separators (space, tab, comma, semicolon, etc.)
        if al_flags & 0x01 != 0 {
            loop {
                let ch = memory.read_byte(si_addr);
                if ch == b' ' || ch == b'\t' || ch == b';' || ch == b',' {
                    si_addr += 1;
                } else {
                    break;
                }
            }
        }
        let skipped = (si_addr - original_si_addr) as usize;

        // Read filename from DS:SI
        let path = DosState::read_asciiz(memory, si_addr, 128);

        let (drive_opt, components, _) = split_path(&path);

        // Drive byte at FCB+0
        if let Some(drive) = drive_opt {
            memory.write_byte(di_addr, drive + 1);
        } else if al_flags & 0x02 == 0 {
            memory.write_byte(di_addr, 0); // default drive
        }

        // Filename at FCB+1 (8 bytes) and extension at FCB+9 (3 bytes)
        let filename: &[u8] = components.last().copied().unwrap_or(b"");
        let fcb = fat_dir::name_to_fcb(filename);
        memory.write_block(di_addr + 1, &fcb);

        // Advance SI past skipped separators + parsed portion
        let mut advance = skipped;
        if drive_opt.is_some() {
            advance += 2; // "X:"
        }
        if let Some(last) = components.last()
            && let Some(pos) = path.windows(last.len()).position(|w| w == *last)
        {
            advance = skipped + pos + last.len();
        }
        cpu.set_si(cpu.si().wrapping_add(advance as u16));

        // Return AL: 0=no wildcards, 1=wildcards present, FF=invalid drive
        let has_wildcards = fcb.contains(&b'?');
        cpu.set_ax((cpu.ax() & 0xFF00) | if has_wildcards { 0x01 } else { 0x00 });
    }

    /// AH=39h: Create subdirectory.
    pub(crate) fn int21h_39h_mkdir(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        disk: &mut dyn DiskIo,
    ) {
        let path_addr = cpu.linear_address(SegmentRegister::DS, cpu.dx());
        let path = DosState::read_asciiz(memory, path_addr, 128);

        match filesystem::create_directory(&mut self.state, memory, disk, &path) {
            Ok(()) => set_iret_carry(cpu, memory, false),
            Err(error) => {
                cpu.set_ax(error);
                set_iret_carry(cpu, memory, true);
            }
        }
    }

    /// AH=3Ch: Create file.
    pub(crate) fn int21h_3ch_create_file(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        disk: &mut dyn DiskIo,
    ) {
        let path_addr = cpu.linear_address(SegmentRegister::DS, cpu.dx());
        let attr = cpu.cx() as u8;
        let path = DosState::read_asciiz(memory, path_addr, 128);

        let result = (|| -> Result<u16, u16> {
            let created =
                filesystem::create_or_truncate_file(&mut self.state, memory, disk, &path, attr)?;
            let drive_index = path
                .first()
                .filter(|_| path.len() >= 2 && path[1] == b':')
                .map(|letter| letter.to_ascii_uppercase() - b'A')
                .unwrap_or(self.state.current_drive);
            let (handle, sft_index) = self.state.allocate_handle(memory)?;
            self.write_sft_for_file(memory, sft_index, &created, drive_index, 0x0002);
            Ok(handle)
        })();

        match result {
            Ok(handle) => {
                cpu.set_ax(handle);
                set_iret_carry(cpu, memory, false);
            }
            Err(error) => {
                cpu.set_ax(error);
                set_iret_carry(cpu, memory, true);
            }
        }
    }

    /// AH=3Dh: Open file.
    pub(crate) fn int21h_3dh_open_file(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        disk: &mut dyn DriveIo,
    ) {
        let path_addr = cpu.linear_address(SegmentRegister::DS, cpu.dx());
        let open_mode = cpu.ax() as u8 & 0x03;
        let path = DosState::read_asciiz(memory, path_addr, 128);
        let is_ems_probe = path_is_ems_device(&path)
            && self.state.ems_enabled
            && self
                .state
                .memory_manager
                .as_ref()
                .is_some_and(|mm| mm.is_ems_enabled());
        let is_xms_probe = self.state.xms_enabled && path_is_xms_device(&path);

        let result = (|| -> Result<u16, u16> {
            if is_ems_probe {
                let (handle, sft_index) = self.state.allocate_handle(memory)?;
                self.write_sft_for_character_device(
                    memory,
                    sft_index,
                    b"EMMXXXX0   ",
                    tables::DEV_EMS_OFFSET,
                    tables::SFT_DEVINFO_DRIVER_CHAR
                        | tables::SFT_DEVINFO_CHAR
                        | tables::SFT_DEVINFO_EOF
                        | tables::SFT_DEVINFO_IOCTL,
                    open_mode as u16,
                );
                return Ok(handle);
            }
            if is_xms_probe {
                let (handle, sft_index) = self.state.allocate_handle(memory)?;
                self.write_sft_for_character_device(
                    memory,
                    sft_index,
                    b"XMSXXXX0   ",
                    tables::DEV_XMS_OFFSET,
                    tables::SFT_DEVINFO_DRIVER_CHAR
                        | tables::SFT_DEVINFO_CHAR
                        | tables::SFT_DEVINFO_EOF
                        | tables::SFT_DEVINFO_IOCTL,
                    open_mode as u16,
                );
                return Ok(handle);
            }
            if path_is_cdrom_device(&path, &self.state.mscdex.device_name) {
                let (handle, sft_index) = self.state.allocate_handle(memory)?;
                self.write_sft_for_character_device(
                    memory,
                    sft_index,
                    &cdrom_sft_name(&self.state.mscdex.device_name),
                    tables::DEV_CDROM_OFFSET,
                    tables::SFT_DEVINFO_DRIVER_CHAR
                        | tables::SFT_DEVINFO_CHAR
                        | tables::SFT_DEVINFO_EOF
                        | tables::SFT_DEVINFO_IOCTL,
                    open_mode as u16,
                );
                return Ok(handle);
            }
            if let Some(device) = path_character_device(&path) {
                let (handle, sft_index) = self.state.allocate_handle(memory)?;
                self.write_sft_for_character_device(
                    memory,
                    sft_index,
                    device.sft_name,
                    device.device_offset,
                    device.device_info,
                    open_mode as u16,
                );
                return Ok(handle);
            }

            let read_path =
                filesystem::resolve_read_file_path(&mut self.state, &path, memory, disk)?;
            let drive_index = read_path.drive_index;

            if drive_index == 25 {
                if open_mode != 0x00 {
                    return Err(0x0005); // Z: is read-only
                }
                let (_, _, fcb_name) =
                    filesystem::resolve_file_path(&mut self.state, &path, memory, disk)?;
                let ventry = self
                    .state
                    .virtual_drive
                    .find_entry(&fcb_name)
                    .ok_or(0x0002u16)?;
                let (handle, sft_index) = self.state.allocate_handle(memory)?;
                self.write_sft_for_virtual_file(memory, sft_index, ventry, open_mode as u16);
                return Ok(handle);
            }

            let entry = find_read_entry(&self.state, &read_path, disk)?.ok_or(0x0002u16)?;
            if entry.attribute & fat_dir::ATTR_DIRECTORY != 0 {
                return Err(0x0005);
            }
            let (handle, sft_index) = self.state.allocate_handle(memory)?;
            match &entry.source {
                ReadDirEntrySource::Fat(entry) => {
                    self.write_sft_for_file(
                        memory,
                        sft_index,
                        entry,
                        drive_index,
                        open_mode as u16,
                    );
                    self.state.open_iso_files[sft_index as usize] = None;
                }
                ReadDirEntrySource::Iso(entry) => {
                    if open_mode != 0x00 {
                        return Err(0x0005);
                    }
                    self.write_sft_for_iso_file(
                        memory,
                        sft_index,
                        entry,
                        drive_index,
                        open_mode as u16,
                    );
                    self.state.open_iso_files[sft_index as usize] = Some(entry.clone());
                }
            }
            Ok(handle)
        })();

        match result {
            Ok(handle) => {
                cpu.set_ax(handle);
                set_iret_carry(cpu, memory, false);
            }
            Err(error) => {
                cpu.set_ax(error);
                set_iret_carry(cpu, memory, true);
            }
        }
    }

    /// AH=3Eh: Close file handle.
    pub(crate) fn int21h_3eh_close_handle(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        disk: &mut dyn DiskIo,
    ) {
        let handle = cpu.bx();

        let result = (|| -> Result<(), u16> {
            let sft_index = self.state.handle_to_sft_index(handle, memory)?;
            let sft_addr = self.state.sft_entry_addr(sft_index).ok_or(0x0006u16)?;
            let ref_count = memory.read_word(sft_addr + tables::SFT_ENT_REF_COUNT);

            let dev_info = memory.read_word(sft_addr + tables::SFT_ENT_DEV_INFO);
            let is_device = dev_info & tables::SFT_DEVINFO_CHAR != 0;

            if !is_device {
                let drive_index = (dev_info & 0x003F) as u8;
                let metadata = read_fat_handle_metadata(memory, sft_addr, drive_index);
                let _ = filesystem::flush_fat_handle(&mut self.state, disk, &metadata);
            }

            self.state.free_handle(handle, memory);
            if ref_count <= 1 {
                self.state.open_iso_files[sft_index as usize] = None;
            }
            Ok(())
        })();

        match result {
            Ok(()) => set_iret_carry(cpu, memory, false),
            Err(error) => {
                cpu.set_ax(error);
                set_iret_carry(cpu, memory, true);
            }
        }
    }

    /// AH=3Fh: Read from file or device.
    pub(crate) fn int21h_3fh_read(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        disk: &mut dyn DriveIo,
    ) {
        let handle = cpu.bx();
        let count = cpu.cx() as u32;
        let buf_addr = cpu.linear_address(SegmentRegister::DS, cpu.dx());

        let result = (|| -> Result<u16, u16> {
            let sft_index = self.state.handle_to_sft_index(handle, memory)?;
            let sft_addr = self.state.sft_entry_addr(sft_index).ok_or(0x0006u16)?;

            let dev_info = memory.read_word(sft_addr + tables::SFT_ENT_DEV_INFO);
            if dev_info & tables::SFT_DEVINFO_CHAR != 0 {
                // Device read: return 0 bytes for now
                return Ok(0);
            }

            // File read
            let drive_index = (dev_info & 0x003F) as u8;
            let file_size = read_dword(memory, sft_addr + tables::SFT_ENT_FILE_SIZE);
            let mut position = read_dword(memory, sft_addr + tables::SFT_ENT_FILE_POS);

            if position >= file_size || count == 0 {
                return Ok(0);
            }

            if drive_index == 25 {
                let mut name = [0u8; 11];
                memory.read_block(sft_addr + tables::SFT_ENT_NAME, &mut name);
                let content = self
                    .state
                    .virtual_drive
                    .file_content(&name)
                    .ok_or(0x0005u16)?;
                let bytes_to_read = count.min(file_size - position) as usize;
                let start = position as usize;
                let end = (start + bytes_to_read).min(content.len());
                let actual = end.saturating_sub(start);
                if actual > 0 {
                    memory.write_block(buf_addr, &content[start..end]);
                }
                position += actual as u32;
                write_dword(memory, sft_addr + tables::SFT_ENT_FILE_POS, position);
                return Ok(actual as u16);
            }

            let bytes_to_read = count.min(file_size - position) as usize;
            if let Some(entry) = self.state.open_iso_files[sft_index as usize].as_ref() {
                let read_data = iso9660::read_file_chunk(entry, position, bytes_to_read, disk)?;
                memory.write_block(buf_addr, &read_data);
                position = position.saturating_add(read_data.len() as u32);
                write_dword(memory, sft_addr + tables::SFT_ENT_FILE_POS, position);
                return Ok(read_data.len() as u16);
            }

            let start_cluster = memory.read_word(sft_addr + tables::SFT_ENT_START_CLUSTER);
            if start_cluster < 2 {
                return Ok(0);
            }
            let vol = self.state.fat_volumes[drive_index as usize]
                .as_ref()
                .ok_or(0x000Fu16)?;
            let mut cursor = FatFileCursor::with_position(start_cluster, file_size, position);
            let read_data = cursor.read_chunk(vol, disk, bytes_to_read)?;
            memory.write_block(buf_addr, &read_data);
            position = cursor.position();

            // Update SFT position
            write_dword(memory, sft_addr + tables::SFT_ENT_FILE_POS, position);

            Ok(read_data.len() as u16)
        })();

        match result {
            Ok(count) => {
                cpu.set_ax(count);
                set_iret_carry(cpu, memory, false);
            }
            Err(error) => {
                cpu.set_ax(error);
                set_iret_carry(cpu, memory, true);
            }
        }
    }

    /// AH=40h: Write to file or device.
    pub(crate) fn int21h_40h_write(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        disk: &mut dyn DiskIo,
    ) {
        let handle = cpu.bx();
        let count = cpu.cx() as u32;
        let buf_addr = cpu.linear_address(SegmentRegister::DS, cpu.dx());

        let result = (|| -> Result<u16, u16> {
            let sft_index = self.state.handle_to_sft_index(handle, memory)?;
            let sft_addr = self.state.sft_entry_addr(sft_index).ok_or(0x0006u16)?;

            let dev_info = memory.read_word(sft_addr + tables::SFT_ENT_DEV_INFO);
            if dev_info & tables::SFT_DEVINFO_CHAR != 0 {
                if dev_info & tables::SFT_DEVINFO_NUL != 0 {
                    return Ok(count as u16);
                }
                // Device write: send to console
                for i in 0..count {
                    let byte = memory.read_byte(buf_addr + i);
                    self.console.process_byte(memory, byte);
                }
                return Ok(count as u16);
            }

            // File write
            let drive_index = (dev_info & 0x003F) as u8;
            let mut metadata = read_fat_handle_metadata(memory, sft_addr, drive_index);

            if count == 0 {
                if metadata.position < metadata.file_size {
                    metadata.file_size = metadata.position;
                    write_fat_handle_metadata(memory, sft_addr, &metadata);
                }
                memory.write_word(
                    sft_addr + tables::SFT_ENT_DEV_INFO,
                    dev_info & !tables::SFT_DEVINFO_EOF,
                );
                return Ok(0);
            }

            let (time, date) = self.state.dos_timestamp_now();
            let mut write_data = vec![0u8; count as usize];
            memory.read_block(buf_addr, &mut write_data);
            metadata.time = time;
            metadata.date = date;
            let (written, metadata) =
                filesystem::write_fat_handle(&mut self.state, disk, metadata, &write_data)?;
            write_fat_handle_metadata(memory, sft_addr, &metadata);
            memory.write_word(
                sft_addr + tables::SFT_ENT_DEV_INFO,
                dev_info & !tables::SFT_DEVINFO_EOF,
            );
            Ok(written)
        })();

        match result {
            Ok(count) => {
                cpu.set_ax(count);
                set_iret_carry(cpu, memory, false);
            }
            Err(error) => {
                cpu.set_ax(error);
                set_iret_carry(cpu, memory, true);
            }
        }
    }

    /// AH=41h: Delete file.
    pub(crate) fn int21h_41h_delete_file(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        disk: &mut dyn DiskIo,
    ) {
        let path_addr = cpu.linear_address(SegmentRegister::DS, cpu.dx());
        let path = DosState::read_asciiz(memory, path_addr, 128);

        let result = filesystem::delete_file(&mut self.state, memory, disk, &path);

        match result {
            Ok(()) => set_iret_carry(cpu, memory, false),
            Err(error) => {
                cpu.set_ax(error);
                set_iret_carry(cpu, memory, true);
            }
        }
    }

    /// AH=42h: Move file pointer (LSEEK).
    pub(crate) fn int21h_42h_lseek(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
    ) {
        let handle = cpu.bx();
        let origin = cpu.ax() as u8;
        let offset = ((cpu.cx() as u32) << 16) | cpu.dx() as u32;

        let result = (|| -> Result<u32, u16> {
            let sft_index = self.state.handle_to_sft_index(handle, memory)?;
            let sft_addr = self.state.sft_entry_addr(sft_index).ok_or(0x0006u16)?;

            let file_size = read_dword(memory, sft_addr + tables::SFT_ENT_FILE_SIZE);
            let current_pos = read_dword(memory, sft_addr + tables::SFT_ENT_FILE_POS);

            let new_pos = match origin {
                0x00 => offset,                                             // from start
                0x01 => (current_pos as i64 + offset as i32 as i64) as u32, // from current
                0x02 => (file_size as i64 + offset as i32 as i64) as u32,   // from end
                _ => return Err(0x0001),                                    // invalid function
            };

            write_dword(memory, sft_addr + tables::SFT_ENT_FILE_POS, new_pos);
            Ok(new_pos)
        })();

        match result {
            Ok(pos) => {
                cpu.set_ax(pos as u16);
                cpu.set_dx((pos >> 16) as u16);
                set_iret_carry(cpu, memory, false);
            }
            Err(error) => {
                cpu.set_ax(error);
                set_iret_carry(cpu, memory, true);
            }
        }
    }

    /// AH=43h: Get/set file attributes.
    pub(crate) fn int21h_43h_get_set_attributes(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        disk: &mut dyn DriveIo,
    ) {
        let path_addr = cpu.linear_address(SegmentRegister::DS, cpu.dx());
        let al = cpu.ax() as u8;
        let path = DosState::read_asciiz(memory, path_addr, 128);

        let result = match al {
            0x00 => filesystem::get_attributes(&mut self.state, memory, disk, &path).map(u16::from),
            0x01 => {
                filesystem::set_attributes(&mut self.state, memory, disk, &path, cpu.cx() as u8)
                    .map(u16::from)
            }
            _ => Err(0x0001),
        };

        match result {
            Ok(attr) => {
                cpu.set_cx(attr);
                set_iret_carry(cpu, memory, false);
            }
            Err(error) => {
                cpu.set_ax(error);
                set_iret_carry(cpu, memory, true);
            }
        }
    }

    /// AH=44h: IOCTL dispatch.
    pub(crate) fn int21h_44h_ioctl(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        drive: &mut dyn DriveIo,
    ) {
        let al = cpu.ax() as u8;
        let handle = cpu.bx();

        match al {
            0x00 => {
                // Get device information
                let result = (|| -> Result<u16, u16> {
                    let sft_index = self.state.handle_to_sft_index(handle, memory)?;
                    let sft_addr = self.state.sft_entry_addr(sft_index).ok_or(0x0006u16)?;
                    let dev_info = memory.read_word(sft_addr + tables::SFT_ENT_DEV_INFO);
                    Ok(dev_info)
                })();
                match result {
                    Ok(info) => {
                        cpu.set_ax(info);
                        cpu.set_dx(info);
                        set_iret_carry(cpu, memory, false);
                    }
                    Err(e) => {
                        cpu.set_ax(e);
                        set_iret_carry(cpu, memory, true);
                    }
                }
            }
            0x01 => {
                // Set device information
                let result = (|| -> Result<(), u16> {
                    let sft_index = self.state.handle_to_sft_index(handle, memory)?;
                    let sft_addr = self.state.sft_entry_addr(sft_index).ok_or(0x0006u16)?;
                    let mut dev_info = memory.read_word(sft_addr + tables::SFT_ENT_DEV_INFO);
                    dev_info = (dev_info & 0xFF00) | (cpu.dx() & 0x00FF);
                    memory.write_word(sft_addr + tables::SFT_ENT_DEV_INFO, dev_info);
                    Ok(())
                })();
                match result {
                    Ok(()) => set_iret_carry(cpu, memory, false),
                    Err(e) => {
                        cpu.set_ax(e);
                        set_iret_carry(cpu, memory, true);
                    }
                }
            }
            0x02 => {
                // Read from character-device control channel.
                let result = (|| -> Result<u16, u16> {
                    let sft_index = self.state.handle_to_sft_index(handle, memory)?;
                    let sft_addr = self.state.sft_entry_addr(sft_index).ok_or(0x0006u16)?;
                    let dev_info = memory.read_word(sft_addr + tables::SFT_ENT_DEV_INFO);
                    if dev_info & tables::SFT_DEVINFO_CHAR == 0
                        || dev_info & tables::SFT_DEVINFO_IOCTL == 0
                    {
                        return Err(0x0001);
                    }
                    if !sft_is_cdrom_device(memory, sft_addr) {
                        return Err(0x0001);
                    }

                    let transfer_addr = cpu.linear_address(SegmentRegister::DS, cpu.dx());
                    self.read_cdrom_control_channel(memory, drive, transfer_addr)?;
                    Ok(cpu.cx())
                })();
                match result {
                    Ok(bytes_read) => {
                        cpu.set_ax(bytes_read);
                        set_iret_carry(cpu, memory, false);
                    }
                    Err(e) => {
                        cpu.set_ax(e);
                        set_iret_carry(cpu, memory, true);
                    }
                }
            }
            0x03 => {
                // Write to character-device control channel.
                let result = (|| -> Result<u16, u16> {
                    let sft_index = self.state.handle_to_sft_index(handle, memory)?;
                    let sft_addr = self.state.sft_entry_addr(sft_index).ok_or(0x0006u16)?;
                    let dev_info = memory.read_word(sft_addr + tables::SFT_ENT_DEV_INFO);
                    if dev_info & tables::SFT_DEVINFO_CHAR == 0
                        || dev_info & tables::SFT_DEVINFO_IOCTL == 0
                    {
                        return Err(0x0001);
                    }
                    if !sft_is_cdrom_device(memory, sft_addr) {
                        return Err(0x0001);
                    }

                    let transfer_addr = cpu.linear_address(SegmentRegister::DS, cpu.dx());
                    self.write_cdrom_control_channel(memory, drive, transfer_addr)?;
                    Ok(cpu.cx())
                })();
                match result {
                    Ok(bytes_written) => {
                        cpu.set_ax(bytes_written);
                        set_iret_carry(cpu, memory, false);
                    }
                    Err(e) => {
                        cpu.set_ax(e);
                        set_iret_carry(cpu, memory, true);
                    }
                }
            }
            0x06 => {
                // Get input status
                cpu.set_ax((cpu.ax() & 0xFF00) | 0xFF);
                set_iret_carry(cpu, memory, false);
            }
            0x07 => {
                // Get output status
                cpu.set_ax((cpu.ax() & 0xFF00) | 0xFF);
                set_iret_carry(cpu, memory, false);
            }
            0x08 => {
                // Is removable?
                let drive = cpu.bx() as u8;
                let da_ua = memory
                    .read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_DAUA_TABLE + drive as u32);
                let removable = matches!(da_ua & 0xF0, 0x90 | 0x70);
                cpu.set_ax((cpu.ax() & 0xFF00) | if removable { 0x00 } else { 0x01 });
                set_iret_carry(cpu, memory, false);
            }
            _ => {
                warn!("INT 21h AH=44h AL={al:#04X}: IOCTL subfunction is unimplemented");
            }
        }
    }

    /// AH=45h: Duplicate file handle (DUP).
    pub(crate) fn int21h_45h_dup_handle(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
    ) {
        let handle = cpu.bx();

        let result = (|| -> Result<u16, u16> {
            let sft_index = self.state.handle_to_sft_index(handle, memory)?;
            let sft_addr = self.state.sft_entry_addr(sft_index).ok_or(0x0006u16)?;

            let new_handle = self.state.find_free_jft_handle(memory)?;

            // Point new handle to same SFT entry
            self.state.write_jft_entry(new_handle, sft_index, memory)?;
            let ref_count = memory.read_word(sft_addr + tables::SFT_ENT_REF_COUNT);
            memory.write_word(sft_addr + tables::SFT_ENT_REF_COUNT, ref_count + 1);

            Ok(new_handle)
        })();

        match result {
            Ok(h) => {
                cpu.set_ax(h);
                set_iret_carry(cpu, memory, false);
            }
            Err(error) => {
                cpu.set_ax(error);
                set_iret_carry(cpu, memory, true);
            }
        }
    }

    /// AH=4Eh: Find first matching file (FINDFIRST).
    pub(crate) fn int21h_4eh_find_first(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        disk: &mut dyn DriveIo,
    ) {
        let path_addr = cpu.linear_address(SegmentRegister::DS, cpu.dx());
        let attr_mask = cpu.cx() as u8;
        let path = DosState::read_asciiz(memory, path_addr, 128);

        let result = (|| -> Result<(), u16> {
            let (drive_index, pattern, dir_cluster, read_directory) = if let Ok(read_path) =
                filesystem::resolve_read_file_path(&mut self.state, &path, memory, disk)
            {
                let dir_cluster = match &read_path.directory {
                    ReadDirectory::Fat(dir_cluster) => *dir_cluster,
                    ReadDirectory::Iso(_) => 0,
                };
                (
                    read_path.drive_index,
                    read_path.name,
                    dir_cluster,
                    Some(read_path.directory),
                )
            } else {
                let (drive_index, dir_cluster, pattern) =
                    filesystem::resolve_file_path(&mut self.state, &path, memory, disk)?;
                (drive_index, pattern, dir_cluster, None)
            };

            let dta_addr = self.state.dta_address;

            // Write search state to DTA
            memory.write_byte(dta_addr, 0x80 | drive_index);
            memory.write_block(dta_addr + 1, &pattern);
            memory.write_byte(dta_addr + 0x0C, attr_mask);
            memory.write_word(dta_addr + 0x0D, 0); // start index
            memory.write_word(dta_addr + 0x0F, dir_cluster);
            self.state.read_find_directory = read_directory;

            // Perform the search
            self.do_find_next(memory, disk)
        })();

        match result {
            Ok(()) => set_iret_carry(cpu, memory, false),
            Err(error) => {
                cpu.set_ax(error);
                set_iret_carry(cpu, memory, true);
            }
        }
    }

    /// AH=4Fh: Find next matching file (FINDNEXT).
    pub(crate) fn int21h_4fh_find_next(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        disk: &mut dyn DriveIo,
    ) {
        match self.do_find_next(memory, disk) {
            Ok(()) => set_iret_carry(cpu, memory, false),
            Err(error) => {
                cpu.set_ax(error);
                set_iret_carry(cpu, memory, true);
            }
        }
    }

    /// Shared implementation for FINDFIRST/FINDNEXT.
    fn do_find_next(
        &mut self,
        memory: &mut dyn MemoryAccess,
        disk: &mut dyn DriveIo,
    ) -> Result<(), u16> {
        let dta_addr = self.state.dta_address;

        let drive_byte = memory.read_byte(dta_addr);
        let drive_index = drive_byte & 0x7F;
        let mut pattern = [0u8; 11];
        memory.read_block(dta_addr + 1, &mut pattern);
        let attr_mask = memory.read_byte(dta_addr + 0x0C);
        let start_index = memory.read_word(dta_addr + 0x0D);
        let dir_cluster = memory.read_word(dta_addr + 0x0F);

        if drive_index == 25 {
            // Virtual Z: drive
            if let Some((ventry, next_index)) =
                self.state
                    .virtual_drive
                    .find_matching(&pattern, attr_mask, start_index)
            {
                memory.write_word(dta_addr + 0x0D, next_index);
                write_find_result(
                    memory,
                    dta_addr,
                    ventry.attribute,
                    ventry.time,
                    ventry.date,
                    ventry.file_size,
                    &ventry.name,
                );
                return Ok(());
            }
            return Err(0x0012); // no more files
        }

        let directory = self
            .state
            .read_find_directory
            .clone()
            .unwrap_or(ReadDirectory::Fat(dir_cluster));

        let result = find_matching_read_entry(
            &self.state,
            drive_index,
            &directory,
            &pattern,
            attr_mask,
            start_index,
            disk,
        )?;

        if let Some((entry, next_index)) = result {
            memory.write_word(dta_addr + 0x0D, next_index);
            write_find_result(
                memory,
                dta_addr,
                entry.attribute,
                entry.time,
                entry.date,
                entry.file_size,
                &entry.name,
            );
            Ok(())
        } else {
            Err(0x0012) // no more files
        }
    }

    /// AH=56h: Rename file.
    pub(crate) fn int21h_56h_rename(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        disk: &mut dyn DiskIo,
    ) {
        let old_addr = cpu.linear_address(SegmentRegister::DS, cpu.dx());
        let new_addr = cpu.linear_address(SegmentRegister::ES, cpu.di());
        let old_path = DosState::read_asciiz(memory, old_addr, 128);
        let new_path = DosState::read_asciiz(memory, new_addr, 128);

        let result = filesystem::rename_entry(&mut self.state, memory, disk, &old_path, &new_path);

        match result {
            Ok(()) => set_iret_carry(cpu, memory, false),
            Err(error) => {
                cpu.set_ax(error);
                set_iret_carry(cpu, memory, true);
            }
        }
    }

    /// AH=57h: Get/set file date and time.
    /// AL=00h: Get file date/time. AL=01h: Set file date/time.
    /// AL=02h-04h: Extended attribute stubs (no-op, return CF=0).
    pub(crate) fn int21h_57h_get_set_datetime(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
    ) {
        let al = cpu.ax() as u8;
        let handle = cpu.bx();

        if matches!(al, 0x02..=0x04) {
            set_iret_carry(cpu, memory, false);
            return;
        }

        let result = (|| -> Result<(u16, u16), u16> {
            let sft_index = self.state.handle_to_sft_index(handle, memory)?;
            let sft_addr = self.state.sft_entry_addr(sft_index).ok_or(0x0006u16)?;

            match al {
                0x00 => {
                    let time = memory.read_word(sft_addr + tables::SFT_ENT_FILE_TIME);
                    let date = memory.read_word(sft_addr + tables::SFT_ENT_FILE_DATE);
                    Ok((time, date))
                }
                0x01 => {
                    memory.write_word(sft_addr + tables::SFT_ENT_FILE_TIME, cpu.cx());
                    memory.write_word(sft_addr + tables::SFT_ENT_FILE_DATE, cpu.dx());
                    Ok((cpu.cx(), cpu.dx()))
                }
                _ => Err(0x0001),
            }
        })();

        match result {
            Ok((time, date)) => {
                cpu.set_cx(time);
                cpu.set_dx(date);
                set_iret_carry(cpu, memory, false);
            }
            Err(error) => {
                cpu.set_ax(error);
                set_iret_carry(cpu, memory, true);
            }
        }
    }

    /// AH=5Dh: Server function call (undocumented).
    /// AL=06h: Get swappable data area pointer.
    /// AL=0Ah: Set extended error information (no-op).
    /// Other subfunctions: not supported.
    pub(crate) fn int21h_5dh_server_call(
        &self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
    ) {
        let al = (cpu.ax() & 0xFF) as u8;
        match al {
            0x06 => {
                self.write_swappable_data_area(memory);
                let (segment, offset) = tables::dos_data_far(tables::SDA_OFFSET);
                cpu.set_ds(segment);
                cpu.set_si(offset);
                cpu.set_cx(tables::SDA_SIZE);
                cpu.set_dx(tables::SDA_SWAP_ALWAYS_SIZE);
                set_iret_carry(cpu, memory, false);
            }
            0x0A => {
                set_iret_carry(cpu, memory, false);
            }
            _ => {
                cpu.set_ax(0x0001);
                set_iret_carry(cpu, memory, true);
            }
        }
    }

    /// Refreshes the SDA "current state" fields consumed by AX=5D06h. The
    /// critical-error flag (SDA+0), InDOS flag (SDA+1) and last-error fields
    /// are owned by the INT 21h dispatcher and INT 24h critical-error path
    /// and must not be touched here, or the dispatcher's pre/post InDOS
    /// counter rebalance would observe a stale value.
    fn write_swappable_data_area(&self, memory: &mut dyn MemoryAccess) {
        let base = tables::SDA_ADDR;
        tables::write_far_ptr(
            memory,
            base + 0x0C,
            self.state.dta_segment,
            self.state.dta_offset,
        );
        memory.write_word(base + 0x10, self.state.current_psp);
        memory.write_word(base + 0x12, 0x0000);
        memory.write_word(base + 0x14, self.state.last_return_code as u16);
        memory.write_byte(base + 0x16, self.state.current_drive);
        memory.write_byte(base + 0x17, self.state.ctrl_break as u8);
    }

    /// Helper: populates an SFT entry for a newly opened/created file.
    fn write_sft_for_file(
        &self,
        mem: &mut dyn MemoryAccess,
        sft_index: u8,
        entry: &fat_dir::DirEntry,
        drive_index: u8,
        open_mode: u16,
    ) {
        let sft_addr = match self.state.sft_entry_addr(sft_index) {
            Some(a) => a,
            None => return,
        };

        mem.write_word(sft_addr + tables::SFT_ENT_REF_COUNT, 1);
        mem.write_word(sft_addr + tables::SFT_ENT_OPEN_MODE, open_mode);
        mem.write_byte(sft_addr + tables::SFT_ENT_FILE_ATTR, entry.attribute);
        // Device info: drive number in low bits, not a char device
        mem.write_word(
            sft_addr + tables::SFT_ENT_DEV_INFO,
            tables::SFT_DEVINFO_EOF | drive_index as u16,
        );
        // DPB pointer (approximate: point to NUL device as placeholder)
        tables::write_far_ptr(
            mem,
            sft_addr + tables::SFT_ENT_DEV_PTR,
            tables::DOS_DATA_SEGMENT,
            tables::DEV_NUL_OFFSET,
        );
        mem.write_word(
            sft_addr + tables::SFT_ENT_START_CLUSTER,
            entry.start_cluster,
        );
        mem.write_word(sft_addr + tables::SFT_ENT_FILE_TIME, entry.time);
        mem.write_word(sft_addr + tables::SFT_ENT_FILE_DATE, entry.date);
        write_dword(mem, sft_addr + tables::SFT_ENT_FILE_SIZE, entry.file_size);
        write_dword(mem, sft_addr + tables::SFT_ENT_FILE_POS, 0);
        mem.write_word(sft_addr + tables::SFT_ENT_REL_CLUSTER, 0);
        mem.write_word(sft_addr + tables::SFT_ENT_CUR_CLUSTER, entry.start_cluster);
        mem.write_word(
            sft_addr + tables::SFT_ENT_DIR_SECTOR,
            entry.dir_sector as u16,
        );
        mem.write_byte(
            sft_addr + tables::SFT_ENT_DIR_INDEX,
            (entry.dir_offset / fat_dir::DIR_ENTRY_SIZE as u16) as u8,
        );
        mem.write_block(sft_addr + tables::SFT_ENT_NAME, &entry.name);
        mem.write_word(sft_addr + tables::SFT_ENT_PSP_OWNER, self.state.current_psp);
    }

    fn write_sft_for_character_device(
        &self,
        mem: &mut dyn MemoryAccess,
        sft_index: u8,
        name: &[u8; 11],
        device_offset: u16,
        device_info: u16,
        open_mode: u16,
    ) {
        let sft_addr = match self.state.sft_entry_addr(sft_index) {
            Some(address) => address,
            None => return,
        };

        mem.write_word(sft_addr + tables::SFT_ENT_REF_COUNT, 1);
        mem.write_word(sft_addr + tables::SFT_ENT_OPEN_MODE, open_mode);
        mem.write_byte(sft_addr + tables::SFT_ENT_FILE_ATTR, 0x00);
        mem.write_word(sft_addr + tables::SFT_ENT_DEV_INFO, device_info);
        tables::write_far_ptr(
            mem,
            sft_addr + tables::SFT_ENT_DEV_PTR,
            tables::DOS_DATA_SEGMENT,
            device_offset,
        );
        mem.write_word(sft_addr + tables::SFT_ENT_START_CLUSTER, 0);
        mem.write_word(sft_addr + tables::SFT_ENT_FILE_TIME, 0);
        mem.write_word(sft_addr + tables::SFT_ENT_FILE_DATE, 0);
        write_dword(mem, sft_addr + tables::SFT_ENT_FILE_SIZE, 0);
        write_dword(mem, sft_addr + tables::SFT_ENT_FILE_POS, 0);
        mem.write_word(sft_addr + tables::SFT_ENT_REL_CLUSTER, 0);
        mem.write_word(sft_addr + tables::SFT_ENT_CUR_CLUSTER, 0);
        mem.write_word(sft_addr + tables::SFT_ENT_DIR_SECTOR, 0);
        mem.write_byte(sft_addr + tables::SFT_ENT_DIR_INDEX, 0);
        mem.write_block(sft_addr + tables::SFT_ENT_NAME, name);
        mem.write_word(sft_addr + tables::SFT_ENT_PSP_OWNER, self.state.current_psp);
    }

    fn write_sft_for_iso_file(
        &self,
        mem: &mut dyn MemoryAccess,
        sft_index: u8,
        entry: &iso9660::IsoDirEntry,
        drive_index: u8,
        open_mode: u16,
    ) {
        let sft_addr = match self.state.sft_entry_addr(sft_index) {
            Some(address) => address,
            None => return,
        };

        mem.write_word(sft_addr + tables::SFT_ENT_REF_COUNT, 1);
        mem.write_word(sft_addr + tables::SFT_ENT_OPEN_MODE, open_mode);
        mem.write_byte(sft_addr + tables::SFT_ENT_FILE_ATTR, entry.attribute);
        mem.write_word(
            sft_addr + tables::SFT_ENT_DEV_INFO,
            tables::SFT_DEVINFO_EOF | drive_index as u16,
        );
        tables::write_far_ptr(
            mem,
            sft_addr + tables::SFT_ENT_DEV_PTR,
            tables::DOS_DATA_SEGMENT,
            tables::DEV_NUL_OFFSET,
        );
        mem.write_word(sft_addr + tables::SFT_ENT_START_CLUSTER, 0);
        mem.write_word(sft_addr + tables::SFT_ENT_FILE_TIME, entry.time);
        mem.write_word(sft_addr + tables::SFT_ENT_FILE_DATE, entry.date);
        write_dword(mem, sft_addr + tables::SFT_ENT_FILE_SIZE, entry.file_size);
        write_dword(mem, sft_addr + tables::SFT_ENT_FILE_POS, 0);
        mem.write_word(sft_addr + tables::SFT_ENT_REL_CLUSTER, 0);
        mem.write_word(sft_addr + tables::SFT_ENT_CUR_CLUSTER, 0);
        mem.write_word(sft_addr + tables::SFT_ENT_DIR_SECTOR, 0);
        mem.write_byte(sft_addr + tables::SFT_ENT_DIR_INDEX, 0);
        mem.write_block(sft_addr + tables::SFT_ENT_NAME, &entry.name);
        mem.write_word(sft_addr + tables::SFT_ENT_PSP_OWNER, self.state.current_psp);
    }

    /// Helper: populates an SFT entry for a virtual Z: drive file.
    fn write_sft_for_virtual_file(
        &self,
        mem: &mut dyn MemoryAccess,
        sft_index: u8,
        entry: &VirtualEntry,
        open_mode: u16,
    ) {
        let sft_addr = match self.state.sft_entry_addr(sft_index) {
            Some(a) => a,
            None => return,
        };

        mem.write_word(sft_addr + tables::SFT_ENT_REF_COUNT, 1);
        mem.write_word(sft_addr + tables::SFT_ENT_OPEN_MODE, open_mode);
        mem.write_byte(sft_addr + tables::SFT_ENT_FILE_ATTR, entry.attribute);
        mem.write_word(
            sft_addr + tables::SFT_ENT_DEV_INFO,
            tables::SFT_DEVINFO_EOF | 25,
        );
        tables::write_far_ptr(
            mem,
            sft_addr + tables::SFT_ENT_DEV_PTR,
            tables::DOS_DATA_SEGMENT,
            tables::DEV_NUL_OFFSET,
        );
        mem.write_word(sft_addr + tables::SFT_ENT_START_CLUSTER, 0);
        mem.write_word(sft_addr + tables::SFT_ENT_FILE_TIME, entry.time);
        mem.write_word(sft_addr + tables::SFT_ENT_FILE_DATE, entry.date);
        write_dword(mem, sft_addr + tables::SFT_ENT_FILE_SIZE, entry.file_size);
        write_dword(mem, sft_addr + tables::SFT_ENT_FILE_POS, 0);
        mem.write_word(sft_addr + tables::SFT_ENT_REL_CLUSTER, 0);
        mem.write_word(sft_addr + tables::SFT_ENT_CUR_CLUSTER, 0);
        mem.write_word(sft_addr + tables::SFT_ENT_DIR_SECTOR, 0);
        mem.write_byte(sft_addr + tables::SFT_ENT_DIR_INDEX, 0);
        mem.write_block(sft_addr + tables::SFT_ENT_NAME, &entry.name);
        mem.write_word(sft_addr + tables::SFT_ENT_PSP_OWNER, self.state.current_psp);
    }
}

/// Writes the FINDFIRST/FINDNEXT result to the DTA at the standard offsets.
fn write_find_result(
    mem: &mut dyn MemoryAccess,
    dta_addr: u32,
    attribute: u8,
    time: u16,
    date: u16,
    file_size: u32,
    fcb_name: &[u8; 11],
) {
    mem.write_byte(dta_addr + 0x15, attribute);
    mem.write_word(dta_addr + 0x16, time);
    mem.write_word(dta_addr + 0x18, date);
    write_dword(mem, dta_addr + 0x1A, file_size);
    // Write display filename at +0x1E (up to 13 bytes, ASCIIZ)
    let display = fat_dir::fcb_to_display_name(fcb_name);
    let len = display.len().min(12);
    mem.write_block(dta_addr + 0x1E, &display[..len]);
    mem.write_byte(dta_addr + 0x1E + len as u32, 0x00);
}
