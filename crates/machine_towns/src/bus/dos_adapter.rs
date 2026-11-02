//! Adapters between NeetanDos and the FM Towns devices.
//!
//! NeetanDos reaches the guest memory through [`DosMemoryAccess`] and the
//! block devices through [`DosDiskIo`]. The device ids follow the disk BIOS:
//! floppy drives `20h`-`23h`, the OS ROM drive `42h` and the SCSI hard disks
//! `B0h`-`B6h`. The CD-ROM drive is the `CdromIo` side of [`DosDiskIo`].

use core::cell::RefCell;

use common::{
    AudioChannelInfo, CdAudioState, CdAudioStatus, CdromIo, CdromTrackInfo, CdromTrackType,
    CursorAccess, DiskIo, HardwareCursorState, MemoryAccess,
};
use device::{
    cdrom::{CdImage, TrackType},
    cdrom_towns::TownsCdController,
    scsi::TownsScsiController,
    wd17xx_fdc::{WD17XX_PLATFORM_FM_TOWNS, Wd17xxFdc},
};
use dos::drive_map::fm_towns::{
    FLOPPY_DEVICE_BASE, FLOPPY_DRIVE_COUNT, OS_ROM_DEVICE_ID, OS_ROM_SECTOR_SIZE, SCSI_DEVICE_BASE,
    SCSI_TARGET_COUNT,
};

use super::scsi_command::{SCSI_GOOD, SCSI_READ10, SCSI_WRITE10, scsi_transfer_command};
use crate::memory::TownsMemory;

/// Status of a request to a device without a medium or an unknown device.
const STATUS_NOT_READY: u8 = 0x02;

/// Status of a request whose sector is missing or unreadable.
const STATUS_SECTOR_ERROR: u8 = 0x10;

/// Status of a write to a read-only medium.
const STATUS_WRITE_PROTECTED: u8 = 0x03;

/// Number of heads of a floppy disk.
const FLOPPY_HEADS: u8 = 2;

/// First sector number of a floppy track.
const FLOPPY_FIRST_RECORD: u8 = 1;

/// Size of a SCSI hard disk block in bytes.
const SCSI_BLOCK_SIZE: usize = device::scsi::disk::SCSI_BLOCK_SIZE;

/// Control byte of a data track in the Q subchannel.
const DATA_TRACK_CONTROL: u8 = 0x14;

/// Control byte of an audio track in the Q subchannel.
const AUDIO_TRACK_CONTROL: u8 = 0x10;

/// Guest memory as the CPU sees it, through the FM Towns memory map. A read
/// has the side effects of a CPU read.
pub(super) struct DosMemoryAccess<'a> {
    /// Physical memory of the machine.
    memory: RefCell<&'a mut TownsMemory>,
}

impl<'a> DosMemoryAccess<'a> {
    /// Wraps the physical memory of the machine.
    pub(super) fn new(memory: &'a mut TownsMemory) -> Self {
        Self {
            memory: RefCell::new(memory),
        }
    }
}

impl MemoryAccess for DosMemoryAccess<'_> {
    fn read_byte(&self, address: u32) -> u8 {
        self.memory.borrow_mut().read_byte(address)
    }

    fn write_byte(&mut self, address: u32, value: u8) {
        self.memory.get_mut().write_byte(address, value);
    }

    fn read_word(&self, address: u32) -> u16 {
        u16::from_le_bytes([self.read_byte(address), self.read_byte(address + 1)])
    }

    fn write_word(&mut self, address: u32, value: u16) {
        let [low, high] = value.to_le_bytes();
        self.write_byte(address, low);
        self.write_byte(address + 1, high);
    }

    fn read_block(&self, address: u32, buffer: &mut [u8]) {
        let mut memory = self.memory.borrow_mut();
        for (offset, byte) in buffer.iter_mut().enumerate() {
            *byte = memory.read_byte(address + offset as u32);
        }
    }

    fn write_block(&mut self, address: u32, data: &[u8]) {
        let memory = self.memory.get_mut();
        for (offset, &byte) in data.iter().enumerate() {
            memory.write_byte(address + offset as u32, byte);
        }
    }
}

/// The hardware cursor of a machine without a text VRAM console.
pub(super) struct NoCursor;

impl CursorAccess for NoCursor {
    fn read(&self) -> HardwareCursorState {
        HardwareCursorState {
            visible: false,
            row: 0,
            col: 0,
        }
    }

    fn write(&mut self, _state: HardwareCursorState) {}
}

/// Block devices and the CD-ROM drive of the machine.
pub(super) struct DosDiskIo<'a> {
    /// Floppy disk controller with the drives `20h`-`23h`.
    pub(super) floppy: &'a mut Wd17xxFdc<WD17XX_PLATFORM_FM_TOWNS>,
    /// SCSI controller with the hard disks `B0h`-`B6h`.
    pub(super) scsi: &'a mut TownsScsiController,
    /// CD-ROM controller.
    pub(super) cdrom: &'a mut TownsCdController,
    /// OS ROM image of the drive `42h`.
    pub(super) os_rom: &'a [u8],
}

/// Block device behind a device id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BlockDevice {
    /// Floppy drive with its drive number.
    Floppy(usize),
    /// The OS ROM drive.
    OsRom,
    /// SCSI hard disk with its target id.
    Scsi(usize),
}

impl BlockDevice {
    /// Returns the block device of `device_id`.
    fn from_device_id(device_id: u8) -> Option<Self> {
        if device_id == OS_ROM_DEVICE_ID {
            Some(Self::OsRom)
        } else if (FLOPPY_DEVICE_BASE..FLOPPY_DEVICE_BASE + FLOPPY_DRIVE_COUNT).contains(&device_id)
        {
            Some(Self::Floppy(usize::from(device_id - FLOPPY_DEVICE_BASE)))
        } else if (SCSI_DEVICE_BASE..SCSI_DEVICE_BASE + SCSI_TARGET_COUNT).contains(&device_id) {
            Some(Self::Scsi(usize::from(device_id - SCSI_DEVICE_BASE)))
        } else {
            None
        }
    }
}

/// Geometry of a floppy disk, read from its first track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FloppyGeometry {
    /// Bytes per sector.
    sector_size: usize,
    /// Sectors per track.
    sectors_per_track: u8,
    /// Number of cylinders.
    cylinders: u16,
}

impl FloppyGeometry {
    /// Returns the cylinder, the head and the sector number of `lba`.
    fn address(&self, lba: u32) -> Option<(u8, u8, u8)> {
        let sectors_per_track = u32::from(self.sectors_per_track);
        let track = lba / sectors_per_track;
        let cylinder = track / u32::from(FLOPPY_HEADS);
        if cylinder >= u32::from(self.cylinders) {
            return None;
        }
        let head = track % u32::from(FLOPPY_HEADS);
        let record = lba % sectors_per_track + u32::from(FLOPPY_FIRST_RECORD);
        Some((cylinder as u8, head as u8, record as u8))
    }

    /// Returns the number of sectors of the disk.
    fn total_sectors(&self) -> u32 {
        u32::from(self.cylinders) * u32::from(FLOPPY_HEADS) * u32::from(self.sectors_per_track)
    }
}

impl DosDiskIo<'_> {
    /// Returns the geometry of the floppy in `drive`.
    fn floppy_geometry(&self, drive: usize) -> Option<FloppyGeometry> {
        if !self.floppy.drive_has_media(drive) {
            return None;
        }
        let size_code = self
            .floppy
            .sector_size_code_direct(drive, 0, 0, FLOPPY_FIRST_RECORD)?;
        let sectors_per_track = u8::try_from(self.floppy.drive_sector_count(drive, 0, 0)).ok()?;
        let track_slots = self.floppy.drive_track_slot_count(drive);
        let cylinders = u16::try_from(track_slots.div_ceil(usize::from(FLOPPY_HEADS))).ok()?;
        if sectors_per_track == 0 || cylinders == 0 {
            return None;
        }
        Some(FloppyGeometry {
            sector_size: 128 << (size_code & 0x03),
            sectors_per_track,
            cylinders,
        })
    }

    /// Reads `count` floppy sectors from `lba` on.
    fn read_floppy(&self, drive: usize, lba: u32, count: u32) -> Result<Vec<u8>, u8> {
        let geometry = self.floppy_geometry(drive).ok_or(STATUS_NOT_READY)?;
        let mut data = Vec::with_capacity(geometry.sector_size * count as usize);
        for sector in lba..lba + count {
            let (cylinder, head, record) = geometry.address(sector).ok_or(STATUS_SECTOR_ERROR)?;
            let (bytes, crc_error) = self
                .floppy
                .read_sector_direct(drive, cylinder, head, record)
                .ok_or(STATUS_SECTOR_ERROR)?;
            if crc_error || bytes.len() != geometry.sector_size {
                return Err(STATUS_SECTOR_ERROR);
            }
            data.extend_from_slice(&bytes);
        }
        Ok(data)
    }

    /// Writes floppy sectors from `lba` on.
    fn write_floppy(&mut self, drive: usize, lba: u32, data: &[u8]) -> Result<(), u8> {
        let geometry = self.floppy_geometry(drive).ok_or(STATUS_NOT_READY)?;
        if self.floppy.drive_write_protected(drive) {
            return Err(STATUS_WRITE_PROTECTED);
        }
        for (sector, bytes) in (lba..).zip(data.chunks_exact(geometry.sector_size)) {
            let (cylinder, head, record) = geometry.address(sector).ok_or(STATUS_SECTOR_ERROR)?;
            if !self
                .floppy
                .write_sector_direct(drive, cylinder, head, record, bytes)
            {
                return Err(STATUS_SECTOR_ERROR);
            }
        }
        Ok(())
    }

    /// Reads `count` sectors of the OS ROM drive from `lba` on.
    fn read_os_rom(&self, lba: u32, count: u32) -> Result<Vec<u8>, u8> {
        let sector_size = usize::from(OS_ROM_SECTOR_SIZE);
        let start = lba as usize * sector_size;
        let end = start + count as usize * sector_size;
        self.os_rom
            .get(start..end)
            .map(<[u8]>::to_vec)
            .ok_or(STATUS_SECTOR_ERROR)
    }

    /// Reads `count` blocks of the SCSI hard disk `target` from `lba` on.
    fn read_scsi(&mut self, target: usize, lba: u32, count: u32) -> Result<Vec<u8>, u8> {
        let blocks = u16::try_from(count).map_err(|_| STATUS_SECTOR_ERROR)?;
        let command = scsi_transfer_command(SCSI_READ10, lba, blocks);
        match self.scsi.execute_data_in_direct(target, &command) {
            Some((data, SCSI_GOOD)) if data.len() == count as usize * SCSI_BLOCK_SIZE => Ok(data),
            Some(_) => Err(STATUS_SECTOR_ERROR),
            None => Err(STATUS_NOT_READY),
        }
    }

    /// Writes blocks of the SCSI hard disk `target` from `lba` on.
    fn write_scsi(&mut self, target: usize, lba: u32, data: &[u8]) -> Result<(), u8> {
        let blocks =
            u16::try_from(data.len() / SCSI_BLOCK_SIZE).map_err(|_| STATUS_SECTOR_ERROR)?;
        let command = scsi_transfer_command(SCSI_WRITE10, lba, blocks);
        match self.scsi.execute_data_out_direct(target, &command, data) {
            Some(SCSI_GOOD) => Ok(()),
            Some(_) => Err(STATUS_SECTOR_ERROR),
            None => Err(STATUS_NOT_READY),
        }
    }

    /// Returns the loaded disc image.
    fn disc(&self) -> Option<&CdImage> {
        self.cdrom.image()
    }
}

impl DiskIo for DosDiskIo<'_> {
    fn read_sectors(&mut self, device_id: u8, lba: u32, count: u32) -> Result<Vec<u8>, u8> {
        match BlockDevice::from_device_id(device_id).ok_or(STATUS_NOT_READY)? {
            BlockDevice::Floppy(drive) => self.read_floppy(drive, lba, count),
            BlockDevice::OsRom => self.read_os_rom(lba, count),
            BlockDevice::Scsi(target) => self.read_scsi(target, lba, count),
        }
    }

    fn write_sectors(&mut self, device_id: u8, lba: u32, data: &[u8]) -> Result<(), u8> {
        match BlockDevice::from_device_id(device_id).ok_or(STATUS_NOT_READY)? {
            BlockDevice::Floppy(drive) => self.write_floppy(drive, lba, data),
            BlockDevice::OsRom => Err(STATUS_WRITE_PROTECTED),
            BlockDevice::Scsi(target) => self.write_scsi(target, lba, data),
        }
    }

    fn sector_size(&self, device_id: u8) -> Option<u16> {
        match BlockDevice::from_device_id(device_id)? {
            BlockDevice::Floppy(drive) => {
                u16::try_from(self.floppy_geometry(drive)?.sector_size).ok()
            }
            BlockDevice::OsRom => Some(OS_ROM_SECTOR_SIZE),
            BlockDevice::Scsi(target) => {
                self.scsi.disk_block_count(target)?;
                Some(SCSI_BLOCK_SIZE as u16)
            }
        }
    }

    fn total_sectors(&self, device_id: u8) -> Option<u32> {
        match BlockDevice::from_device_id(device_id)? {
            BlockDevice::Floppy(drive) => Some(self.floppy_geometry(drive)?.total_sectors()),
            BlockDevice::OsRom => {
                Some((self.os_rom.len() / usize::from(OS_ROM_SECTOR_SIZE)) as u32)
            }
            BlockDevice::Scsi(target) => self.scsi.disk_block_count(target),
        }
    }

    fn drive_geometry(&self, device_id: u8) -> Option<(u16, u8, u8)> {
        match BlockDevice::from_device_id(device_id)? {
            BlockDevice::Floppy(drive) => {
                let geometry = self.floppy_geometry(drive)?;
                Some((geometry.cylinders, FLOPPY_HEADS, geometry.sectors_per_track))
            }
            BlockDevice::OsRom | BlockDevice::Scsi(_) => None,
        }
    }
}

impl CdromIo for DosDiskIo<'_> {
    fn cdrom_present(&self) -> bool {
        true
    }

    fn cdrom_media_loaded(&self) -> bool {
        self.cdrom.has_disc()
    }

    fn read_sector_cooked(&self, lba: u32, buffer: &mut [u8]) -> Option<usize> {
        self.disc()?.read_sector(lba, buffer)
    }

    fn read_sector_raw(&self, lba: u32, buffer: &mut [u8]) -> Option<usize> {
        self.disc()?.read_sector_raw(lba, buffer)
    }

    fn track_count(&self) -> u8 {
        self.disc().map_or(0, CdImage::track_count)
    }

    fn track_info(&self, track_number: u8) -> Option<CdromTrackInfo> {
        let track = self.disc()?.track(track_number)?;
        let (track_type, control) = match track.track_type {
            TrackType::Data => (CdromTrackType::Data, DATA_TRACK_CONTROL),
            TrackType::Audio => (CdromTrackType::Audio, AUDIO_TRACK_CONTROL),
        };
        Some(CdromTrackInfo {
            start_lba: track.start_lba,
            track_type,
            control,
        })
    }

    fn leadout_lba(&self) -> u32 {
        self.cdrom.total_sectors()
    }

    fn total_sectors(&self) -> u32 {
        self.cdrom.total_sectors()
    }

    fn audio_play(&mut self, _start_lba: u32, _sector_count: u32) {}

    fn audio_stop(&mut self) {}

    fn audio_resume(&mut self) {}

    fn audio_state(&self) -> CdAudioStatus {
        CdAudioStatus {
            state: CdAudioState::Stopped,
            current_lba: 0,
            start_lba: 0,
            end_lba: 0,
        }
    }

    fn audio_channel_info(&self) -> AudioChannelInfo {
        AudioChannelInfo {
            input_channel: [0, 1, 2, 3],
            volume: [0xFF, 0xFF, 0, 0],
        }
    }

    fn set_audio_channel_info(&mut self, _info: &AudioChannelInfo) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_ids_select_the_block_devices() {
        assert_eq!(
            BlockDevice::from_device_id(0x20),
            Some(BlockDevice::Floppy(0))
        );
        assert_eq!(
            BlockDevice::from_device_id(0x23),
            Some(BlockDevice::Floppy(3))
        );
        assert_eq!(BlockDevice::from_device_id(0x24), None);
        assert_eq!(BlockDevice::from_device_id(0x42), Some(BlockDevice::OsRom));
        assert_eq!(
            BlockDevice::from_device_id(0xB0),
            Some(BlockDevice::Scsi(0))
        );
        assert_eq!(
            BlockDevice::from_device_id(0xB6),
            Some(BlockDevice::Scsi(6))
        );
        assert_eq!(BlockDevice::from_device_id(0xB7), None);
    }

    #[test]
    fn floppy_lba_follows_cylinder_head_sector_order() {
        let geometry = FloppyGeometry {
            sector_size: 1024,
            sectors_per_track: 8,
            cylinders: 77,
        };
        assert_eq!(geometry.address(0), Some((0, 0, 1)));
        assert_eq!(geometry.address(7), Some((0, 0, 8)));
        assert_eq!(geometry.address(8), Some((0, 1, 1)));
        assert_eq!(geometry.address(16), Some((1, 0, 1)));
        assert_eq!(geometry.total_sectors(), 1232);
        assert_eq!(geometry.address(1231), Some((76, 1, 8)));
        assert_eq!(geometry.address(1232), None);
    }
}
