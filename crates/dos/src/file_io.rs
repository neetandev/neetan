//! INT 21h file I/O function implementations.

use common::warn;

use crate::{
    CpuAccess, DiskIo, DosState, DriveIo, FarPointer, MemoryAccess, NeetanDos, SegmentRegister,
    device_chain,
    device_request::{COMMAND_IOCTL_INPUT, COMMAND_IOCTL_OUTPUT},
    dos::{StdoutRoute, route_for_character_device},
    filesystem::{
        self, FatHandleMetadata, ReadDirEntrySource, ReadDirectory, fat_dir,
        fat_file::FatFileCursor, find_matching_read_entry, find_read_entry, iso9660, split_path,
        virtual_drive::VirtualEntry,
    },
    guest_character::DeviceTransfer,
    layout::SystemFileTableFormat,
    tables,
};

/// DOS error code of an invalid handle.
const ERROR_INVALID_HANDLE: u16 = 0x0006;
/// Access bits of an `INT 21h AH=3Dh` open mode: 0 read, 1 write, 2 both.
const OPEN_ACCESS_MASK: u8 = 0x03;
/// DOS error code of an invalid function.
const ERROR_INVALID_FUNCTION: u16 = 0x0001;
/// DOS error code of a denied access.
const ERROR_ACCESS_DENIED: u16 = 0x0005;
/// Function number of `INT 21h AH=3Eh`. The kernel puts it back into AH
/// after a successful close.
const CLOSE_FUNCTION: u16 = 0x3E;
/// AL of `INT 2Fh AX=1120h` (network flush) without a redirector. Disk
/// reset of the DOS 3 kernel returns it.
const NETWORK_FLUSH_AL: u8 = 0x01;
/// AX of the share check `INT 2Ah AX=0300h` without a network. IOCTL
/// `AL=09h` returns it.
const SHARE_CHECK_AX: u16 = 0x0300;
/// AL after the close of a CD-ROM file: the answer of the redirector close
/// (`INT 2Fh AX=1106h`).
const REDIRECTOR_CLOSE_CODE: u8 = 0x00;

/// Writes a 32-bit value to emulated memory.
fn write_dword(mem: &mut dyn MemoryAccess, addr: u32, value: u32) {
    mem.write_word(addr, value as u16);
    mem.write_word(addr + 2, (value >> 16) as u16);
}

/// Reads a 32-bit value from emulated memory.
fn read_dword(mem: &dyn MemoryAccess, addr: u32) -> u32 {
    mem.read_word(addr) as u32 | ((mem.read_word(addr + 2) as u32) << 16)
}

/// Character device that an open of a device name reaches.
struct CharacterDeviceOpen {
    /// Name field of the SFT entry.
    sft_name: [u8; 11],
    /// Device driver header.
    device: FarPointer,
    /// Device information word of the SFT entry.
    device_info: u16,
}

/// Attribute bits that the device information word of an SFT entry copies:
/// standard input, standard output, NUL, CLOCK, fast console and IOCTL
/// support.
const DEVICE_INFO_ATTRIBUTE_BITS: u16 = 0x401F;

/// Returns the device name of the last component of `path` blank padded,
/// `None` for a component of more than eight characters.
fn path_device_name(path: &[u8]) -> Option<[u8; 8]> {
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
        return None;
    }
    let mut padded = [b' '; 8];
    padded[..component.len()].copy_from_slice(component);
    padded.make_ascii_uppercase();
    Some(padded)
}

/// Returns the character device of the device chain whose name matches the
/// last component of `path`. `AUX` and `PRN` fall back to the NUL device
/// when the chain holds no such device.
fn path_character_device(
    memory: &dyn MemoryAccess,
    path: &[u8],
    nul: FarPointer,
) -> Option<CharacterDeviceOpen> {
    let name = path_device_name(path)?;
    let mut sft_name = [b' '; 11];
    sft_name[..8].copy_from_slice(&name);
    if let Some(device) = device_chain::find_character_device(memory, nul, &name) {
        let attribute = memory.read_word(device.linear() + tables::DEVHDR_OFF_ATTRIBUTE);
        return Some(CharacterDeviceOpen {
            sft_name,
            device,
            device_info: tables::SFT_DEVINFO_CHAR_DEVICE_COMMON
                | (attribute & DEVICE_INFO_ATTRIBUTE_BITS),
        });
    }
    if &name == b"AUX     " || &name == b"PRN     " {
        return Some(CharacterDeviceOpen {
            sft_name,
            device: nul,
            device_info: tables::SFT_DEVINFO_CHAR_DEVICE_COMMON,
        });
    }
    None
}

fn path_is_named_device(path: &[u8], device_name: &[u8; 8]) -> bool {
    path_device_name(path).is_some_and(|name| &name == device_name)
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

fn read_fat_handle_metadata(
    memory: &mut dyn MemoryAccess,
    sft: &SystemFileTableFormat,
    sft_addr: u32,
    drive_index: u8,
) -> FatHandleMetadata {
    let (dir_sector, dir_index) = sft.read_directory_location(memory, sft_addr);
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
    sft: &SystemFileTableFormat,
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
    memory.write_word(sft_addr + sft.last_cluster, metadata.start_cluster);
    sft.write_directory_location(
        memory,
        sft_addr,
        metadata.dir_sector,
        (metadata.dir_offset / fat_dir::DIR_ENTRY_SIZE as u16) as u8,
    );
    memory.write_block(sft_addr + tables::SFT_ENT_NAME, &metadata.name);
}

impl NeetanDos {
    /// AH=0Dh: Disk reset - flush all dirty FAT caches. On a DOS 3 kernel AL
    /// is `01h`, the AL of the network flush `INT 2Fh AX=1120h` without a
    /// redirector. Other kernels keep AX.
    pub(crate) fn int21h_0dh_disk_reset(&mut self, cpu: &mut dyn CpuAccess, disk: &mut dyn DiskIo) {
        for vol in self.state.fat_volumes.iter_mut().flatten() {
            let _ = vol.flush_fat(disk);
        }
        if self.state.is_dos3_kernel() {
            cpu.set_ax(cpu.ax() & 0xFF00 | u16::from(NETWORK_FLUSH_AL));
        }
    }

    /// AH=1Ch: Get allocation information for specific drive.
    pub(crate) fn int21h_1ch_get_alloc_info(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        disk: &mut dyn DiskIo,
    ) {
        let dl = (cpu.dx() & 0xFF) as u8;
        let drive_index = if dl == 0 {
            self.state.current_drive
        } else {
            dl - 1
        };
        self.state.refresh_drive_geometry(drive_index, memory, disk);

        // Read DPB for the drive
        let dpb_layout = &self.state.structures().drive_parameter_block;
        let dpb_ptr_addr = self.state.list_of_lists_address() + tables::SYSVARS_OFF_DPB_PTR;
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
                let media = memory.read_byte(dpb_addr + dpb_layout.media_descriptor);

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
            let next_off = memory.read_word(dpb_addr + dpb_layout.next);
            let next_seg = memory.read_word(dpb_addr + dpb_layout.next + 2);
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

        let cds_addr = self.state.cds_entry_addr(drive_index, memory);
        let cds_flags = memory.read_word(cds_addr + tables::CDS_OFF_FLAGS);
        if cds_flags == 0 {
            cpu.set_ax(0xFFFF);
            return;
        }

        if self
            .state
            .drive_map
            .entry(drive_index)
            .is_some_and(|entry| entry.device.has_cdrom_file_system())
        {
            let total_sectors =
                crate::CdromIo::total_sectors(&self.state.drive_cdrom(drive_index, disk));
            cpu.set_ax(1);
            cpu.set_bx(0);
            cpu.set_cx(2048);
            cpu.set_dx(u16::try_from(total_sectors).unwrap_or(u16::MAX));
            return;
        }

        if self.state.ensure_volume_mounted(drive_index, disk).is_err() {
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
            Ok(()) => self.state.set_iret_carry(cpu, memory, false),
            Err(error) => {
                cpu.set_ax(error);
                self.state.set_iret_carry(cpu, memory, true);
            }
        }
    }

    /// AH=3Ah: Remove subdirectory. MS-DOS reports a directory that still
    /// holds entries as access denied.
    pub(crate) fn int21h_3ah_rmdir(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        disk: &mut dyn DiskIo,
    ) {
        let path_addr = cpu.linear_address(SegmentRegister::DS, cpu.dx());
        let path = DosState::read_asciiz(memory, path_addr, 128);

        match filesystem::remove_directory(&mut self.state, memory, disk, &path) {
            Ok(()) => self.state.set_iret_carry(cpu, memory, false),
            Err(error) => {
                let code = if error == filesystem::ERROR_DIRECTORY_NOT_EMPTY {
                    ERROR_ACCESS_DENIED
                } else {
                    error
                };
                cpu.set_ax(code);
                self.state.set_iret_carry(cpu, memory, true);
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
                self.state.set_iret_carry(cpu, memory, false);
            }
            Err(error) => {
                cpu.set_ax(error);
                self.state.set_iret_carry(cpu, memory, true);
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
        let open_mode = cpu.ax() as u8;
        let access_mode = open_mode & OPEN_ACCESS_MASK;
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
                    FarPointer::new(
                        self.state.dos_data_segment(),
                        self.state.self_boot_layout().ems_device,
                    ),
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
                    FarPointer::new(
                        self.state.dos_data_segment(),
                        self.state.self_boot_layout().xms_device,
                    ),
                    tables::SFT_DEVINFO_DRIVER_CHAR
                        | tables::SFT_DEVINFO_CHAR
                        | tables::SFT_DEVINFO_EOF
                        | tables::SFT_DEVINFO_IOCTL,
                    open_mode as u16,
                );
                return Ok(handle);
            }
            if self.has_builtin_cdrom_device(&*disk)
                && path_is_cdrom_device(&path, &self.state.mscdex.device_name)
            {
                let (handle, sft_index) = self.state.allocate_handle(memory)?;
                self.write_sft_for_character_device(
                    memory,
                    sft_index,
                    &cdrom_sft_name(&self.state.mscdex.device_name),
                    FarPointer::new(
                        self.state.dos_data_segment(),
                        self.state.self_boot_layout().cdrom_device,
                    ),
                    tables::SFT_DEVINFO_DRIVER_CHAR
                        | tables::SFT_DEVINFO_CHAR
                        | tables::SFT_DEVINFO_EOF
                        | tables::SFT_DEVINFO_IOCTL,
                    open_mode as u16,
                );
                return Ok(handle);
            }
            let nul = FarPointer::new(
                self.state.dos_data_segment(),
                self.state.nul_device_offset(),
            );
            if let Some(device) = path_character_device(memory, &path, nul) {
                let (handle, sft_index) = self.state.allocate_handle(memory)?;
                self.write_sft_for_character_device(
                    memory,
                    sft_index,
                    &device.sft_name,
                    device.device,
                    device.device_info,
                    open_mode as u16,
                );
                return Ok(handle);
            }

            let read_path =
                filesystem::resolve_read_file_path(&mut self.state, &path, memory, disk)?;
            let drive_index = read_path.drive_index;

            if drive_index == 25 {
                if access_mode != 0x00 {
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
                    if access_mode != 0x00 {
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
                self.state.set_iret_carry(cpu, memory, false);
            }
            Err(error) => {
                cpu.set_ax(error);
                self.state.set_iret_carry(cpu, memory, true);
            }
        }
    }

    /// AH=3Eh: Close file handle.
    ///
    /// On success AH keeps `3Eh` and AL holds the value the kernel close
    /// leaves: the low byte of the reference count before the close for a
    /// device or an unwritten file, the drive number for a written file and
    /// `00h` for a CD-ROM file of the redirector.
    pub(crate) fn int21h_3eh_close_handle(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        disk: &mut dyn DiskIo,
    ) {
        let handle = cpu.bx();

        let result = (|| -> Result<u8, u16> {
            let sft_index = self.state.handle_to_sft_index(handle, memory)?;
            let sft_addr = self
                .state
                .sft_entry_addr(sft_index, memory)
                .ok_or(ERROR_INVALID_HANDLE)?;
            let ref_count = memory.read_word(sft_addr + tables::SFT_ENT_REF_COUNT);

            let dev_info = memory.read_word(sft_addr + tables::SFT_ENT_DEV_INFO);
            let is_device = dev_info & tables::SFT_DEVINFO_CHAR != 0;

            if !is_device {
                let drive_index = (dev_info & 0x003F) as u8;
                let metadata = read_fat_handle_metadata(
                    memory,
                    &self.state.structures().system_file_table,
                    sft_addr,
                    drive_index,
                );
                let _ = filesystem::flush_fat_handle(&mut self.state, disk, &metadata);
            }

            let close_code = if self.state.open_iso_files[sft_index as usize].is_some() {
                REDIRECTOR_CLOSE_CODE
            } else if !is_device && dev_info & tables::SFT_DEVINFO_EOF == 0 {
                (dev_info & tables::SFT_DEVINFO_DRIVE_MASK) as u8
            } else {
                ref_count as u8
            };
            self.state.free_handle(handle, memory);
            if ref_count <= 1 {
                self.state.open_iso_files[sft_index as usize] = None;
            }
            Ok(close_code)
        })();

        match result {
            Ok(close_code) => {
                cpu.set_ax(CLOSE_FUNCTION << 8 | u16::from(close_code));
                self.state.set_iret_carry(cpu, memory, false);
            }
            Err(error) => {
                cpu.set_ax(error);
                self.state.set_iret_carry(cpu, memory, true);
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

        let console_device_info = self
            .state
            .handle_to_sft_index(handle, memory)
            .ok()
            .and_then(|sft_index| self.state.sft_entry_addr(sft_index, memory))
            .map(|sft_addr| memory.read_word(sft_addr + tables::SFT_ENT_DEV_INFO))
            .filter(|&device_info| crate::console_line::is_console_input(device_info));
        if let Some(device_info) = console_device_info {
            if device_info & tables::SFT_DEVINFO_RAW != 0 {
                self.read_console_raw(cpu, memory);
            } else {
                self.read_console_line(cpu, memory);
            }
            return;
        }

        let result = (|| -> Result<u16, u16> {
            let sft_index = self.state.handle_to_sft_index(handle, memory)?;
            let sft_addr = self
                .state
                .sft_entry_addr(sft_index, memory)
                .ok_or(ERROR_INVALID_HANDLE)?;

            let dev_info = memory.read_word(sft_addr + tables::SFT_ENT_DEV_INFO);
            if dev_info & tables::SFT_DEVINFO_CHAR != 0 {
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
                let cdrom = self.state.drive_cdrom(drive_index, disk);
                let read_data = iso9660::read_file_chunk(entry, position, bytes_to_read, &cdrom)?;
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
            Ok(read_count) => {
                cpu.set_ax(read_count);
                self.state.set_iret_carry(cpu, memory, false);
            }
            Err(error) => {
                cpu.set_ax(error);
                self.state.set_iret_carry(cpu, memory, true);
            }
        }
    }

    /// AH=40h: Write to file or device. A character device in cooked mode
    /// takes the bytes in front of the first end of file mark and AX returns
    /// their count.
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
            let sft_addr = self
                .state
                .sft_entry_addr(sft_index, memory)
                .ok_or(ERROR_INVALID_HANDLE)?;

            let dev_info = memory.read_word(sft_addr + tables::SFT_ENT_DEV_INFO);
            if dev_info & tables::SFT_DEVINFO_CHAR != 0 {
                let is_nul = dev_info & tables::SFT_DEVINFO_NUL != 0;
                let cooked = dev_info & tables::SFT_DEVINFO_RAW == 0;
                let written = if cooked && !is_nul {
                    (0..count)
                        .position(|i| memory.read_byte(buf_addr + i) == tables::END_OF_FILE_MARK)
                        .map_or(count, |position| position as u32)
                } else {
                    count
                };
                if self.stdout_trace_enabled() {
                    let route = route_for_character_device(memory, is_nul);
                    self.record_stdout_write(memory, handle, buf_addr, count, written, route);
                }
                if is_nul {
                    return Ok(count as u16);
                }
                for i in 0..written {
                    let byte = memory.read_byte(buf_addr + i);
                    if cooked {
                        self.write_stdout_kernel_byte(memory, byte);
                    } else {
                        self.write_stdout_byte(memory, byte);
                    }
                }
                return Ok(written as u16);
            }

            // A standard output handle pointing at a regular file means the
            // guest redirected console output.
            if (handle == 1 || handle == 2) && self.stdout_trace_enabled() {
                self.record_stdout_write(
                    memory,
                    handle,
                    buf_addr,
                    count,
                    count,
                    StdoutRoute::Redirected,
                );
            }

            // File write
            let drive_index = (dev_info & 0x003F) as u8;
            let mut metadata = read_fat_handle_metadata(
                memory,
                &self.state.structures().system_file_table,
                sft_addr,
                drive_index,
            );

            if count == 0 {
                if metadata.position < metadata.file_size {
                    metadata.file_size = metadata.position;
                    write_fat_handle_metadata(
                        memory,
                        &self.state.structures().system_file_table,
                        sft_addr,
                        &metadata,
                    );
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
            write_fat_handle_metadata(
                memory,
                &self.state.structures().system_file_table,
                sft_addr,
                &metadata,
            );
            memory.write_word(
                sft_addr + tables::SFT_ENT_DEV_INFO,
                dev_info & !tables::SFT_DEVINFO_EOF,
            );
            Ok(written)
        })();

        match result {
            Ok(count) => {
                cpu.set_ax(count);
                self.state.set_iret_carry(cpu, memory, false);
            }
            Err(error) => {
                cpu.set_ax(error);
                self.state.set_iret_carry(cpu, memory, true);
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
            Ok(()) => self.state.set_iret_carry(cpu, memory, false),
            Err(error) => {
                cpu.set_ax(error);
                self.state.set_iret_carry(cpu, memory, true);
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
            let sft_addr = self
                .state
                .sft_entry_addr(sft_index, memory)
                .ok_or(ERROR_INVALID_HANDLE)?;

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
                self.state.set_iret_carry(cpu, memory, false);
            }
            Err(error) => {
                cpu.set_ax(error);
                self.state.set_iret_carry(cpu, memory, true);
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
                if al == 0x00 {
                    // MS-DOS returns the attributes in AX as well.
                    cpu.set_ax(attr);
                }
                self.state.set_iret_carry(cpu, memory, false);
            }
            Err(error) => {
                cpu.set_ax(error);
                self.state.set_iret_carry(cpu, memory, true);
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
                    let sft_addr = self
                        .state
                        .sft_entry_addr(sft_index, memory)
                        .ok_or(ERROR_INVALID_HANDLE)?;
                    let dev_info = memory.read_word(sft_addr + tables::SFT_ENT_DEV_INFO);
                    Ok(dev_info)
                })();
                match result {
                    Ok(info) => {
                        cpu.set_ax(info);
                        cpu.set_dx(info);
                        self.state.set_iret_carry(cpu, memory, false);
                    }
                    Err(e) => {
                        cpu.set_ax(e);
                        self.state.set_iret_carry(cpu, memory, true);
                    }
                }
            }
            0x01 => {
                // Set device information
                let result = (|| -> Result<(), u16> {
                    let sft_index = self.state.handle_to_sft_index(handle, memory)?;
                    let sft_addr = self
                        .state
                        .sft_entry_addr(sft_index, memory)
                        .ok_or(ERROR_INVALID_HANDLE)?;
                    let mut dev_info = memory.read_word(sft_addr + tables::SFT_ENT_DEV_INFO);
                    dev_info = (dev_info & 0xFF00) | (cpu.dx() & 0x00FF);
                    memory.write_word(sft_addr + tables::SFT_ENT_DEV_INFO, dev_info);
                    Ok(())
                })();
                match result {
                    Ok(()) => self.state.set_iret_carry(cpu, memory, false),
                    Err(e) => {
                        cpu.set_ax(e);
                        self.state.set_iret_carry(cpu, memory, true);
                    }
                }
            }
            0x02 | 0x03 => {
                // Read from or write to the control channel of a character device.
                let Some(result) = self.ioctl_control_channel(cpu, memory, drive, al) else {
                    return;
                };
                match result {
                    Ok(count) => {
                        cpu.set_ax(count);
                        self.state.set_iret_carry(cpu, memory, false);
                    }
                    Err(e) => {
                        cpu.set_ax(e);
                        self.state.set_iret_carry(cpu, memory, true);
                    }
                }
            }
            0x06 => {
                // Get input status
                cpu.set_ax((cpu.ax() & 0xFF00) | 0xFF);
                self.state.set_iret_carry(cpu, memory, false);
            }
            0x07 => {
                // Get output status
                cpu.set_ax((cpu.ax() & 0xFF00) | 0xFF);
                self.state.set_iret_carry(cpu, memory, false);
            }
            0x08 => {
                // Is removable? BL = drive (0 = default, 1 = A:).
                let drive_number = cpu.bx() as u8;
                let drive_index = if drive_number == 0 {
                    self.state.current_drive
                } else {
                    drive_number - 1
                };
                match self.state.drive_map.entry(drive_index) {
                    Some(entry) => {
                        let removable = entry.device.is_removable();
                        cpu.set_ax(if removable { 0x0000 } else { 0x0001 });
                        self.state.set_iret_carry(cpu, memory, false);
                    }
                    None => {
                        cpu.set_ax(crate::ERROR_INVALID_DRIVE);
                        self.state.set_iret_carry(cpu, memory, true);
                    }
                }
            }
            0x09 => {
                // Is the drive remote? BL = drive (0 = default, 1 = A:).
                let drive_number = cpu.bx() as u8;
                let drive_index = if drive_number == 0 {
                    self.state.current_drive
                } else {
                    drive_number - 1
                };
                match self.state.drive_device_attributes(drive_index, memory) {
                    Some(attributes) => {
                        cpu.set_dx(attributes);
                        cpu.set_ax(SHARE_CHECK_AX);
                        self.state.set_iret_carry(cpu, memory, false);
                    }
                    None => {
                        cpu.set_ax(crate::ERROR_INVALID_DRIVE);
                        self.state.set_iret_carry(cpu, memory, true);
                    }
                }
            }
            _ => {
                warn!("INT 21h AH=44h AL={al:#04X}: IOCTL subfunction is unimplemented");
            }
        }
    }

    /// IOCTL `AL=02h` (read) and `AL=03h` (write) of the control channel of
    /// the character device behind handle BX: the CD-ROM device of NeetanDos
    /// or a guest driver with IOCTL support. Returns `None` while the request
    /// to a guest driver is pending.
    fn ioctl_control_channel(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        drive: &mut dyn DriveIo,
        subfunction: u8,
    ) -> Option<Result<u16, u16>> {
        let handle = cpu.bx();
        let sft_addr = match self.state.handle_to_sft_index(handle, memory) {
            Ok(sft_index) => match self.state.sft_entry_addr(sft_index, memory) {
                Some(address) => address,
                None => return Some(Err(ERROR_INVALID_HANDLE)),
            },
            Err(error) => return Some(Err(error)),
        };
        let dev_info = memory.read_word(sft_addr + tables::SFT_ENT_DEV_INFO);
        if dev_info & tables::SFT_DEVINFO_CHAR == 0 || dev_info & tables::SFT_DEVINFO_IOCTL == 0 {
            return Some(Err(ERROR_INVALID_FUNCTION));
        }
        let dos_data_segment = self.state.dos_data_segment();
        let transfer_addr = cpu.linear_address(SegmentRegister::DS, cpu.dx());
        if self.state.sft_is_builtin_cdrom_device(memory, sft_addr)
            && let Some(mirror_header) = self.state.builtin_cdrom_mirror_header()
        {
            let result = if subfunction == 0x02 {
                self.read_cdrom_control_channel(memory, drive, transfer_addr, mirror_header)
            } else {
                self.write_cdrom_control_channel(memory, drive, transfer_addr)
            };
            return Some(result.map(|()| cpu.cx()));
        }
        let device = FarPointer::new(
            memory.read_word(sft_addr + tables::SFT_ENT_DEV_PTR + 2),
            memory.read_word(sft_addr + tables::SFT_ENT_DEV_PTR),
        );
        if device.segment == dos_data_segment {
            return Some(Err(ERROR_INVALID_FUNCTION));
        }
        let command = if subfunction == 0x02 {
            COMMAND_IOCTL_INPUT
        } else {
            COMMAND_IOCTL_OUTPUT
        };
        self.state.guest_device_transfer(DeviceTransfer {
            device,
            command,
            buffer: FarPointer::new(cpu.ds(), cpu.dx()),
            count: cpu.cx(),
        })
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
            let sft_addr = self
                .state
                .sft_entry_addr(sft_index, memory)
                .ok_or(ERROR_INVALID_HANDLE)?;

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
                self.state.set_iret_carry(cpu, memory, false);
            }
            Err(error) => {
                cpu.set_ax(error);
                self.state.set_iret_carry(cpu, memory, true);
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
            self.state.read_find_directory = match read_directory {
                Some(ReadDirectory::Iso(directory)) => Some(ReadDirectory::Iso(directory)),
                Some(ReadDirectory::Fat(_)) | None => None,
            };

            // Perform the search
            self.do_find_next(memory, disk)
        })();

        match result {
            Ok(()) => {
                cpu.set_ax(0);
                self.state.set_iret_carry(cpu, memory, false);
            }
            Err(error) => {
                cpu.set_ax(error);
                self.state.set_iret_carry(cpu, memory, true);
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
            Ok(()) => {
                cpu.set_ax(0);
                self.state.set_iret_carry(cpu, memory, false);
            }
            Err(error) => {
                cpu.set_ax(error);
                self.state.set_iret_carry(cpu, memory, true);
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

        let directory = if self
            .state
            .fat_volumes
            .get(drive_index as usize)
            .and_then(Option::as_ref)
            .is_some()
        {
            ReadDirectory::Fat(dir_cluster)
        } else {
            self.state
                .read_find_directory
                .clone()
                .unwrap_or(ReadDirectory::Fat(dir_cluster))
        };

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
            Ok(()) => self.state.set_iret_carry(cpu, memory, false),
            Err(error) => {
                cpu.set_ax(error);
                self.state.set_iret_carry(cpu, memory, true);
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
            self.state.set_iret_carry(cpu, memory, false);
            return;
        }

        let result = (|| -> Result<(u16, u16), u16> {
            let sft_index = self.state.handle_to_sft_index(handle, memory)?;
            let sft_addr = self
                .state
                .sft_entry_addr(sft_index, memory)
                .ok_or(ERROR_INVALID_HANDLE)?;

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
                self.state.set_iret_carry(cpu, memory, false);
            }
            Err(error) => {
                cpu.set_ax(error);
                self.state.set_iret_carry(cpu, memory, true);
            }
        }
    }

    /// AH=5Dh: Server function call (undocumented).
    /// AL=06h: Get swappable data area pointer.
    /// AL=07h-09h: Get and set the redirected printer mode, flush the
    /// redirected printer output. DOS hands these to a network redirector
    /// through `INT 2Fh AX=1125h`. NeetanDos has no redirector and answers
    /// like the default `INT 2Fh` handler of DOS: invalid function.
    /// AL=0Ah: Set extended error information.
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
                let sda = &self.state.data_area().swappable_data_area;
                cpu.set_ds(self.state.dos_data_segment());
                cpu.set_si(sda.data_offset);
                cpu.set_cx(sda.size);
                cpu.set_dx(sda.always_swapped_size);
                self.state.set_iret_carry(cpu, memory, false);
            }
            0x0A => self.int21h_5d0ah_set_extended_error(cpu, memory),
            _ => {
                cpu.set_ax(0x0001);
                self.state.set_iret_carry(cpu, memory, true);
            }
        }
    }

    /// Refreshes the SDA "current state" fields consumed by AX=5D06h. The
    /// critical-error flag (SDA+0), InDOS flag (SDA+1) and last-error fields
    /// are owned by the INT 21h dispatcher and INT 24h critical-error path
    /// and must not be touched here, or the dispatcher's pre/post InDOS
    /// counter rebalance would observe a stale value.
    pub(crate) fn write_swappable_data_area(&self, memory: &mut dyn MemoryAccess) {
        use tables::{
            SDA_OFF_BREAK_FLAG, SDA_OFF_CURRENT_DRIVE, SDA_OFF_CURRENT_PSP, SDA_OFF_DTA,
            SDA_OFF_INT23_STACK_POINTER, SDA_OFF_RETURN_CODE,
        };
        let base = self.state.sda_address();
        tables::write_far_ptr(
            memory,
            base + u32::from(SDA_OFF_DTA),
            self.state.dta_segment,
            self.state.dta_offset,
        );
        memory.write_word(
            base + u32::from(SDA_OFF_CURRENT_PSP),
            self.state.current_psp,
        );
        memory.write_word(base + u32::from(SDA_OFF_INT23_STACK_POINTER), 0x0000);
        memory.write_word(
            base + u32::from(SDA_OFF_RETURN_CODE),
            self.state.last_return_code as u16,
        );
        memory.write_byte(
            base + u32::from(SDA_OFF_CURRENT_DRIVE),
            self.state.current_drive,
        );
        memory.write_byte(
            base + u32::from(SDA_OFF_BREAK_FLAG),
            self.state.ctrl_break as u8,
        );
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
        let sft_addr = match self.state.sft_entry_addr(sft_index, mem) {
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
            self.state.dos_data_segment(),
            self.state.nul_device_offset(),
        );
        mem.write_word(
            sft_addr + tables::SFT_ENT_START_CLUSTER,
            entry.start_cluster,
        );
        mem.write_word(sft_addr + tables::SFT_ENT_FILE_TIME, entry.time);
        mem.write_word(sft_addr + tables::SFT_ENT_FILE_DATE, entry.date);
        write_dword(mem, sft_addr + tables::SFT_ENT_FILE_SIZE, entry.file_size);
        write_dword(mem, sft_addr + tables::SFT_ENT_FILE_POS, 0);
        let sft = &self.state.structures().system_file_table;
        sft.write_cluster_position(mem, sft_addr, 0, entry.start_cluster);
        sft.write_directory_location(
            mem,
            sft_addr,
            entry.dir_sector,
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
        device: FarPointer,
        device_info: u16,
        open_mode: u16,
    ) {
        let sft_addr = match self.state.sft_entry_addr(sft_index, mem) {
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
            device.segment,
            device.offset,
        );
        mem.write_word(sft_addr + tables::SFT_ENT_START_CLUSTER, 0);
        mem.write_word(sft_addr + tables::SFT_ENT_FILE_TIME, 0);
        mem.write_word(sft_addr + tables::SFT_ENT_FILE_DATE, 0);
        write_dword(mem, sft_addr + tables::SFT_ENT_FILE_SIZE, 0);
        write_dword(mem, sft_addr + tables::SFT_ENT_FILE_POS, 0);
        let sft = &self.state.structures().system_file_table;
        sft.write_cluster_position(mem, sft_addr, 0, 0);
        sft.write_directory_location(mem, sft_addr, 0, 0);
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
        let sft_addr = match self.state.sft_entry_addr(sft_index, mem) {
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
            self.state.dos_data_segment(),
            self.state.nul_device_offset(),
        );
        mem.write_word(sft_addr + tables::SFT_ENT_START_CLUSTER, 0);
        mem.write_word(sft_addr + tables::SFT_ENT_FILE_TIME, entry.time);
        mem.write_word(sft_addr + tables::SFT_ENT_FILE_DATE, entry.date);
        write_dword(mem, sft_addr + tables::SFT_ENT_FILE_SIZE, entry.file_size);
        write_dword(mem, sft_addr + tables::SFT_ENT_FILE_POS, 0);
        let sft = &self.state.structures().system_file_table;
        sft.write_cluster_position(mem, sft_addr, 0, 0);
        sft.write_directory_location(mem, sft_addr, 0, 0);
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
        let sft_addr = match self.state.sft_entry_addr(sft_index, mem) {
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
            self.state.dos_data_segment(),
            self.state.nul_device_offset(),
        );
        mem.write_word(sft_addr + tables::SFT_ENT_START_CLUSTER, 0);
        mem.write_word(sft_addr + tables::SFT_ENT_FILE_TIME, entry.time);
        mem.write_word(sft_addr + tables::SFT_ENT_FILE_DATE, entry.date);
        write_dword(mem, sft_addr + tables::SFT_ENT_FILE_SIZE, entry.file_size);
        write_dword(mem, sft_addr + tables::SFT_ENT_FILE_POS, 0);
        let sft = &self.state.structures().system_file_table;
        sft.write_cluster_position(mem, sft_addr, 0, 0);
        sft.write_directory_location(mem, sft_addr, 0, 0);
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
