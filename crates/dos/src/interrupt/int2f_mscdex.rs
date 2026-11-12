//! INT 2Fh AH=15h: the MSCDEX interface of the HLE MSCDEX.
//!
//! The drives come from `DosState::mscdex_drives`. A guest CD-ROM drive
//! loads its volume image through guest calls on its first access. The
//! entry then runs again.

use common::warn;

use crate::{
    CdromIo, CpuAccess, MemoryAccess, NeetanDos, SegmentRegister,
    cdrom::STATUS_UNKNOWN_COMMAND,
    filesystem::{fat_dir, iso9660},
};

/// DOS error code of an invalid drive.
const ERROR_INVALID_DRIVE: u16 = 0x000F;

/// DOS error code of a drive that is not ready.
const ERROR_NOT_READY: u16 = 0x0015;

/// DOS error code of a missing file.
const ERROR_FILE_NOT_FOUND: u16 = 0x0002;

/// DOS error code of a missing path.
const ERROR_PATH_NOT_FOUND: u16 = 0x0003;

/// Sector of the primary volume descriptor.
const VOLUME_DESCRIPTOR_SECTOR: u32 = 16;

/// Bytes per cooked CD sector.
const SECTOR_SIZE: usize = 2048;

/// Length of an identifier field of the primary volume descriptor.
const IDENTIFIER_LENGTH: usize = 37;

/// Offset of the copyright file identifier in the primary volume descriptor.
const COPYRIGHT_FILE_OFFSET: usize = 702;

/// Offset of the abstract file identifier in the primary volume descriptor.
const ABSTRACT_FILE_OFFSET: usize = 739;

/// Offset of the bibliographic file identifier in the primary volume descriptor.
const BIBLIOGRAPHIC_FILE_OFFSET: usize = 776;

/// MSCDEX version 2.10 in `BX` of `AX=150Ch`.
const MSCDEX_VERSION: u16 = 0x020A;

/// `BX` of `AX=150Bh` while MSCDEX serves a drive.
const DRIVE_CHECK_SIGNATURE: u16 = 0xADAD;

/// Disk format of `AX=150Fh`: ISO 9660.
const DISK_FORMAT_ISO9660: u16 = 0x0001;

/// Size of one entry of the device list of `AX=1501h`.
const DEVICE_LIST_ENTRY_SIZE: u32 = 5;

impl NeetanDos {
    /// AH=15h: MSCDEX CD-ROM interface.
    pub(crate) fn int2fh_15h_mscdex(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        cdrom: &mut dyn CdromIo,
    ) {
        let drives = self.state.mscdex_drives(cdrom);
        let al = cpu.ax() as u8;
        match al {
            0x00 => {
                cpu.set_bx(drives.len() as u16);
                if let Some(first) = drives.first() {
                    cpu.set_cx(u16::from(first.drive_index));
                }
            }
            0x01 => {
                let buffer = cpu.linear_address(SegmentRegister::ES, cpu.bx());
                for (index, drive) in drives.iter().enumerate() {
                    let entry = buffer + index as u32 * DEVICE_LIST_ENTRY_SIZE;
                    memory.write_byte(entry, 0);
                    memory.write_word(entry + 1, drive.device.offset);
                    memory.write_word(entry + 3, drive.device.segment);
                }
            }
            0x02 => self.mscdex_volume_identifier(cpu, memory, cdrom, COPYRIGHT_FILE_OFFSET),
            0x03 => self.mscdex_volume_identifier(cpu, memory, cdrom, ABSTRACT_FILE_OFFSET),
            0x04 => self.mscdex_volume_identifier(cpu, memory, cdrom, BIBLIOGRAPHIC_FILE_OFFSET),
            0x05 => self.mscdex_read_volume_descriptor(cpu, memory, cdrom),
            0x06 | 0x07 | 0x09 | 0x0A => {}
            0x08 => self.mscdex_absolute_read(cpu, memory, cdrom),
            0x0B => {
                let drive_index = cpu.cx() as u8;
                if !drives.is_empty() {
                    cpu.set_bx(DRIVE_CHECK_SIGNATURE);
                }
                if drives.iter().any(|drive| drive.drive_index == drive_index) {
                    cpu.set_ax(cpu.ax() | 0x00FF);
                } else {
                    cpu.set_ax(0);
                }
            }
            0x0C => cpu.set_bx(MSCDEX_VERSION),
            0x0D => {
                let buffer = cpu.linear_address(SegmentRegister::ES, cpu.bx());
                for (index, drive) in drives.iter().enumerate() {
                    memory.write_byte(buffer + index as u32, drive.drive_index);
                }
            }
            0x0F => self.mscdex_directory_entry(cpu, memory, cdrom),
            0x10 => {
                let request = cpu.linear_address(SegmentRegister::ES, cpu.bx());
                let drive_index = cpu.cx() as u8;
                match drives.iter().find(|drive| drive.drive_index == drive_index) {
                    Some(drive) if drive.native => {
                        self.handle_device_request(memory, cdrom, request, drive.device);
                    }
                    Some(_) => {
                        warn!("MSCDEX: device requests to a guest CD-ROM drive are unsupported");
                        memory.write_word(request + 3, STATUS_UNKNOWN_COMMAND);
                    }
                    None => {
                        cpu.set_ax(ERROR_INVALID_DRIVE);
                        self.state.set_iret_carry(cpu, memory, true);
                    }
                }
            }
            _ => {
                warn!("INT 2Fh AX=15{al:02X}h is unimplemented");
            }
        }
    }

    /// Checks the MSCDEX drive in `CX` and loads the image of a guest drive.
    /// Returns `None` after it set the error result or asked for a rerun.
    fn ready_mscdex_drive(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        cdrom: &dyn CdromIo,
        drive_index: u8,
    ) -> Option<()> {
        let result = match self.state.mscdex_drive(drive_index, cdrom) {
            None => Err(ERROR_INVALID_DRIVE),
            Some(_) => self.state.require_guest_cdrom(drive_index),
        };
        match result {
            Ok(()) => Some(()),
            Err(_) if self.state.guest_requests.redispatch => None,
            Err(error) => {
                cpu.set_ax(error);
                self.state.set_iret_carry(cpu, memory, true);
                None
            }
        }
    }

    /// Fails the call with "not ready".
    fn mscdex_not_ready(&self, cpu: &mut dyn CpuAccess, memory: &mut dyn MemoryAccess) {
        cpu.set_ax(ERROR_NOT_READY);
        self.state.set_iret_carry(cpu, memory, true);
    }

    /// AX=1502h-1504h: copies an identifier field of the primary volume
    /// descriptor of drive `CX` to `ES:BX`.
    fn mscdex_volume_identifier(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        cdrom: &mut dyn CdromIo,
        field_offset: usize,
    ) {
        let drive_index = cpu.cx() as u8;
        if self
            .ready_mscdex_drive(cpu, memory, cdrom, drive_index)
            .is_none()
        {
            return;
        }
        let view = self.state.drive_cdrom(drive_index, cdrom);
        let mut sector = [0u8; SECTOR_SIZE];
        if !view.cdrom_media_loaded()
            || view
                .read_sector_cooked(VOLUME_DESCRIPTOR_SECTOR, &mut sector)
                .is_none()
        {
            self.mscdex_not_ready(cpu, memory);
            return;
        }
        let buffer = cpu.linear_address(SegmentRegister::ES, cpu.bx());
        memory.write_block(
            buffer,
            &sector[field_offset..field_offset + IDENTIFIER_LENGTH],
        );
        memory.write_byte(buffer + IDENTIFIER_LENGTH as u32, 0);
        self.state.set_iret_carry(cpu, memory, false);
    }

    /// AX=1505h: reads volume descriptor `DX` of drive `CX` to `ES:BX`.
    fn mscdex_read_volume_descriptor(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        cdrom: &mut dyn CdromIo,
    ) {
        let drive_index = cpu.cx() as u8;
        if self
            .ready_mscdex_drive(cpu, memory, cdrom, drive_index)
            .is_none()
        {
            return;
        }
        let view = self.state.drive_cdrom(drive_index, cdrom);
        if !view.cdrom_media_loaded() {
            self.mscdex_not_ready(cpu, memory);
            return;
        }
        let mut sector = [0u8; SECTOR_SIZE];
        let Some(count) =
            view.read_sector_cooked(VOLUME_DESCRIPTOR_SECTOR + u32::from(cpu.dx()), &mut sector)
        else {
            self.mscdex_not_ready(cpu, memory);
            return;
        };
        memory.write_block(
            cpu.linear_address(SegmentRegister::ES, cpu.bx()),
            &sector[..count],
        );
        let descriptor_type = match sector[0] {
            1 => 1,
            0xFF => 0xFF,
            _ => 0,
        };
        cpu.set_ax(descriptor_type);
        self.state.set_iret_carry(cpu, memory, false);
    }

    /// AX=1508h: reads `DX` sectors from sector `SI:DI` of drive `CX` to
    /// `ES:BX`.
    fn mscdex_absolute_read(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        cdrom: &mut dyn CdromIo,
    ) {
        let drive_index = cpu.cx() as u8;
        if self
            .ready_mscdex_drive(cpu, memory, cdrom, drive_index)
            .is_none()
        {
            return;
        }
        let view = self.state.drive_cdrom(drive_index, cdrom);
        if !view.cdrom_media_loaded() {
            self.mscdex_not_ready(cpu, memory);
            return;
        }
        let sector_count = u32::from(cpu.dx());
        let first_sector = (u32::from(cpu.si()) << 16) | u32::from(cpu.di());
        let buffer = cpu.linear_address(SegmentRegister::ES, cpu.bx());
        let mut sector = [0u8; SECTOR_SIZE];
        for index in 0..sector_count {
            let Some(count) = view.read_sector_cooked(first_sector + index, &mut sector) else {
                self.mscdex_not_ready(cpu, memory);
                return;
            };
            memory.write_block(buffer + index * SECTOR_SIZE as u32, &sector[..count]);
        }
        self.state.set_iret_carry(cpu, memory, false);
    }

    /// AX=150Fh: copies the ISO 9660 directory record of the path at `ES:BX`
    /// on drive `CL` to `SI:DI` and returns the disk format in `AX`.
    fn mscdex_directory_entry(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        cdrom: &mut dyn CdromIo,
    ) {
        let drive_index = cpu.cx() as u8;
        if self
            .ready_mscdex_drive(cpu, memory, cdrom, drive_index)
            .is_none()
        {
            return;
        }
        let path = crate::DosState::read_asciiz(
            memory,
            cpu.linear_address(SegmentRegister::ES, cpu.bx()),
            128,
        );
        let record = {
            let view = self.state.drive_cdrom(drive_index, cdrom);
            directory_record(&view, &path)
        };
        match record {
            Ok(record) => {
                let buffer = (u32::from(cpu.si()) << 4) + u32::from(cpu.di());
                memory.write_block(buffer, &record);
                cpu.set_ax(DISK_FORMAT_ISO9660);
                self.state.set_iret_carry(cpu, memory, false);
            }
            Err(error) => {
                cpu.set_ax(error);
                self.state.set_iret_carry(cpu, memory, true);
            }
        }
    }
}

/// Returns the directory record of `path` on the ISO 9660 volume of
/// `cdrom`. The path may start with a drive and a backslash.
fn directory_record(cdrom: &dyn CdromIo, path: &[u8]) -> Result<Vec<u8>, u16> {
    let volume = iso9660::IsoVolume::mount(cdrom)?;
    let path = if path.len() >= 2 && path[1] == b':' {
        &path[2..]
    } else {
        path
    };
    let components: Vec<&[u8]> = path
        .split(|&byte| byte == b'\\' || byte == b'/')
        .filter(|component| !component.is_empty())
        .collect();
    let Some((last, parents)) = components.split_last() else {
        return Err(ERROR_FILE_NOT_FOUND);
    };
    let mut directory = volume.root_directory.clone();
    for component in parents {
        let entry =
            iso9660::find_entry(&volume, &directory, &fat_dir::name_to_fcb(component), cdrom)?
                .ok_or(ERROR_PATH_NOT_FOUND)?;
        directory = entry.directory.ok_or(ERROR_PATH_NOT_FOUND)?;
    }
    iso9660::find_record(&volume, &directory, &fat_dir::name_to_fcb(last), cdrom)?
        .map(|(_, record)| record)
        .ok_or(ERROR_FILE_NOT_FOUND)
}
