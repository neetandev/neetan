//! CD-ROM drives of guest CD-ROM device drivers.
//!
//! The ISO 9660 code of NeetanDos reads sectors synchronously through
//! `CdromIo`. A guest CD-ROM driver answers only through guest calls.
//! NeetanDos therefore loads the whole volume of a guest CD-ROM drive into a
//! host image: device open (`0Dh`), read long (`80h`) of the primary volume
//! descriptor (ISO 9660 or High Sierra) for the volume size, read long of
//! every sector of the volume through the transfer buffer of the layout and
//! device close (`0Eh`). The FM Towns `MENU_DRV` volume holds 128 KiB.
//! `DriveCdrom` serves the image to the ISO 9660 code.
//!
//! The load sends IOCTL input `09h` (media changed) in front of the reads,
//! like MSCDEX. Every later entry checks the medium of a loaded image once
//! with the same request: an answer other than "not changed" loads the
//! image again. A driver error of a read long asks the `INT 24h` handler
//! (`critical_error.rs`).
//!
//! The first access of an entry to an unloaded drive fails with "not ready"
//! and asks for the guest steps and a rerun of the entry.

use common::{AudioChannelInfo, CdAudioState, CdAudioStatus, CdromTrackInfo, warn};

use crate::{
    CdromIo, DosState, MemoryAccess, NeetanDos,
    critical_error::{CriticalErrorStep, DiskArea},
    device_request::{REQUEST_OFF_TRANSFER, request_status, status_is_error, write_request_header},
    drive_map::{self, DriveDevice},
    filesystem::iso9660::VolumeFormat,
    guest_call::{FarPointer, GuestCall, GuestCallArea, GuestStep, StepStart},
    guest_volume::ERROR_NOT_READY,
};

/// Bytes per cooked CD sector.
pub(crate) const CDROM_SECTOR_SIZE: usize = 2048;

/// Sector of the primary volume descriptor.
const VOLUME_DESCRIPTOR_SECTOR: u32 = 16;

/// Largest volume image in sectors (4 MiB).
const IMAGE_SECTOR_LIMIT: u32 = 2048;

/// Command code of read long.
const COMMAND_READ_LONG: u8 = 0x80;

/// Command code of device open.
const COMMAND_DEVICE_OPEN: u8 = 0x0D;

/// Command code of device close.
const COMMAND_DEVICE_CLOSE: u8 = 0x0E;

/// Length of a read long request.
const READ_LONG_REQUEST_LENGTH: u8 = 0x1B;

/// Command code of IOCTL input.
const COMMAND_IOCTL_INPUT: u8 = 0x03;
/// Length of an IOCTL input request.
const IOCTL_INPUT_REQUEST_LENGTH: u8 = 0x1A;
/// Offset of the transfer count word of IOCTL input.
const IOCTL_OFF_COUNT: u32 = 0x12;
/// Control block code of the media changed query.
const MEDIA_CHANGED_CODE: u8 = 0x09;
/// Length of the media changed control block.
const MEDIA_CHANGED_BLOCK_LENGTH: u16 = 2;
/// Media changed answer: the medium did not change. `00h` (unknown) and
/// `FFh` (changed) load the image again.
const MEDIA_NOT_CHANGED: u8 = 0x01;
/// Media changed answer: the medium changed.
const MEDIA_CHANGED: u8 = 0xFF;
/// Length of a device open or close request.
const OPEN_CLOSE_REQUEST_LENGTH: u8 = 0x0D;

/// Offset of the addressing mode byte of read long (`0` = HSG).
const READ_LONG_OFF_ADDRESSING_MODE: u32 = 0x0D;

/// Offset of the sector count word of read long.
const READ_LONG_OFF_COUNT: u32 = 0x12;

/// Offset of the start sector dword of read long.
const READ_LONG_OFF_START_SECTOR: u32 = 0x14;

/// Offset of the read mode byte of read long (`0` = cooked).
const READ_LONG_OFF_READ_MODE: u32 = 0x18;

/// Returns the bit of a drive in a drive mask.
const fn drive_bit(drive_index: u8) -> u32 {
    1 << drive_index
}

save_state::runtime_state! {
    /// Images and per entry state of the guest CD-ROM drives.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) struct GuestCdroms {
        /// Volume image per drive letter. A loading image holds the sectors
        /// read so far.
        pub(crate) images: Vec<Option<Vec<u8>>>,
        /// Drives with a complete image.
        pub(crate) loaded: u32,
        /// Drives whose load failed in the current entry.
        pub(crate) load_failed: u32,
        /// Drives whose medium the current entry checked.
        pub(crate) media_checked: u32,
    }
}

impl Default for GuestCdroms {
    fn default() -> Self {
        Self {
            images: vec![None; drive_map::DRIVE_LETTER_COUNT],
            loaded: 0,
            load_failed: 0,
            media_checked: 0,
        }
    }
}

impl GuestCdroms {
    /// Forgets the failures and the media checks of the previous entry.
    pub(crate) fn begin_entry(&mut self) {
        self.load_failed = 0;
        self.media_checked = 0;
    }

    /// Drops the image of a drive.
    pub(crate) fn drop_image(&mut self, drive_index: u8) {
        if let Some(slot) = self.images.get_mut(usize::from(drive_index)) {
            *slot = None;
        }
        self.loaded &= !drive_bit(drive_index);
    }

    /// Returns the complete image of a drive.
    fn image(&self, drive_index: u8) -> Option<&[u8]> {
        if self.loaded & drive_bit(drive_index) == 0 {
            return None;
        }
        self.images
            .get(usize::from(drive_index))
            .and_then(Option::as_deref)
    }
}

/// A request to a guest CD-ROM device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GuestCdromStep {
    /// Device open in front of the load.
    Open {
        /// Drive of the device.
        drive_index: u8,
    },
    /// IOCTL input `09h` (media changed). During a load the answer only
    /// reaches the driver. For a loaded image a changed medium or an unknown
    /// state loads the image again.
    MediaCheck {
        /// Drive of the device.
        drive_index: u8,
    },
    /// Read long of the primary volume descriptor.
    ReadVolumeSize {
        /// Drive of the device.
        drive_index: u8,
    },
    /// Read long of one sector into the image.
    ReadSector {
        /// Drive of the device.
        drive_index: u8,
        /// Sector of the volume.
        sector: u32,
    },
    /// Device close after the load.
    Close {
        /// Drive of the device.
        drive_index: u8,
    },
}

/// Save state tag of `GuestCdromStep::Open`.
const CDROM_TAG_OPEN: u8 = 0;
/// Save state tag of `GuestCdromStep::ReadVolumeSize`.
const CDROM_TAG_READ_VOLUME_SIZE: u8 = 1;
/// Save state tag of `GuestCdromStep::ReadSector`.
const CDROM_TAG_READ_SECTOR: u8 = 2;
/// Save state tag of `GuestCdromStep::Close`.
const CDROM_TAG_CLOSE: u8 = 3;
/// Save state tag of `GuestCdromStep::MediaCheck`.
const CDROM_TAG_MEDIA_CHECK: u8 = 4;

impl GuestCdromStep {
    /// Returns the drive of the step.
    fn drive_index(&self) -> u8 {
        match self {
            Self::Open { drive_index }
            | Self::MediaCheck { drive_index }
            | Self::ReadVolumeSize { drive_index }
            | Self::ReadSector { drive_index, .. }
            | Self::Close { drive_index } => *drive_index,
        }
    }
}

impl save_state::StateEncode for GuestCdromStep {
    fn encode_state(&self, output: &mut Vec<u8>) {
        let tag = match self {
            Self::Open { .. } => CDROM_TAG_OPEN,
            Self::MediaCheck { .. } => CDROM_TAG_MEDIA_CHECK,
            Self::ReadVolumeSize { .. } => CDROM_TAG_READ_VOLUME_SIZE,
            Self::ReadSector { .. } => CDROM_TAG_READ_SECTOR,
            Self::Close { .. } => CDROM_TAG_CLOSE,
        };
        save_state::StateEncode::encode_state(&tag, output);
        save_state::StateEncode::encode_state(&self.drive_index(), output);
        if let Self::ReadSector { sector, .. } = self {
            save_state::StateEncode::encode_state(sector, output);
        }
    }
}

impl save_state::StateDecode for GuestCdromStep {
    fn decode_state(
        decoder: &mut save_state::StateDecoder<'_>,
    ) -> Result<Self, save_state::StateDecodeError> {
        let tag = <u8 as save_state::StateDecode>::decode_state(decoder)?;
        let drive_index = save_state::StateDecode::decode_state(decoder)?;
        match tag {
            CDROM_TAG_OPEN => Ok(Self::Open { drive_index }),
            CDROM_TAG_READ_VOLUME_SIZE => Ok(Self::ReadVolumeSize { drive_index }),
            CDROM_TAG_READ_SECTOR => Ok(Self::ReadSector {
                drive_index,
                sector: save_state::StateDecode::decode_state(decoder)?,
            }),
            CDROM_TAG_CLOSE => Ok(Self::Close { drive_index }),
            CDROM_TAG_MEDIA_CHECK => Ok(Self::MediaCheck { drive_index }),
            _ => Err(save_state::StateDecodeError::InvalidTag),
        }
    }
}

/// `CdromIo` of one CD-ROM drive for the ISO 9660 code: the machine drive or
/// the image of a guest CD-ROM drive.
pub(crate) struct DriveCdrom<'a> {
    /// CD-ROM drive of the machine.
    machine: &'a mut dyn CdromIo,
    /// Image of a guest CD-ROM drive. `None` reads the machine drive.
    image: Option<&'a [u8]>,
}

impl CdromIo for DriveCdrom<'_> {
    fn cdrom_present(&self) -> bool {
        self.image.is_some() || self.machine.cdrom_present()
    }

    fn cdrom_media_loaded(&self) -> bool {
        self.image.is_some() || self.machine.cdrom_media_loaded()
    }

    fn read_sector_cooked(&self, lba: u32, buffer: &mut [u8]) -> Option<usize> {
        let Some(image) = self.image else {
            return self.machine.read_sector_cooked(lba, buffer);
        };
        let start = usize::try_from(lba).ok()? * CDROM_SECTOR_SIZE;
        let sector = image.get(start..start + CDROM_SECTOR_SIZE)?;
        let count = buffer.len().min(CDROM_SECTOR_SIZE);
        buffer[..count].copy_from_slice(&sector[..count]);
        Some(count)
    }

    fn read_sector_raw(&self, lba: u32, buffer: &mut [u8]) -> Option<usize> {
        match self.image {
            Some(_) => None,
            None => self.machine.read_sector_raw(lba, buffer),
        }
    }

    fn track_count(&self) -> u8 {
        match self.image {
            Some(_) => 1,
            None => self.machine.track_count(),
        }
    }

    fn track_info(&self, track_number: u8) -> Option<CdromTrackInfo> {
        match self.image {
            Some(_) => None,
            None => self.machine.track_info(track_number),
        }
    }

    fn leadout_lba(&self) -> u32 {
        match self.image {
            Some(image) => (image.len() / CDROM_SECTOR_SIZE) as u32,
            None => self.machine.leadout_lba(),
        }
    }

    fn total_sectors(&self) -> u32 {
        match self.image {
            Some(image) => (image.len() / CDROM_SECTOR_SIZE) as u32,
            None => self.machine.total_sectors(),
        }
    }

    fn audio_play(&mut self, start_lba: u32, sector_count: u32) {
        if self.image.is_none() {
            self.machine.audio_play(start_lba, sector_count);
        }
    }

    fn audio_stop(&mut self) {
        if self.image.is_none() {
            self.machine.audio_stop();
        }
    }

    fn audio_resume(&mut self) {
        if self.image.is_none() {
            self.machine.audio_resume();
        }
    }

    fn audio_state(&self) -> CdAudioStatus {
        match self.image {
            Some(_) => CdAudioStatus {
                state: CdAudioState::Stopped,
                current_lba: 0,
                start_lba: 0,
                end_lba: 0,
            },
            None => self.machine.audio_state(),
        }
    }

    fn audio_channel_info(&self) -> AudioChannelInfo {
        self.machine.audio_channel_info()
    }

    fn set_audio_channel_info(&mut self, info: &AudioChannelInfo) {
        if self.image.is_none() {
            self.machine.set_audio_channel_info(info);
        }
    }
}

impl DosState {
    /// Returns the `CdromIo` of the CD-ROM drive `drive_index`. A guest
    /// CD-ROM drive without a loaded image reads like an empty drive.
    pub(crate) fn drive_cdrom<'a>(
        &'a self,
        drive_index: u8,
        machine: &'a mut dyn CdromIo,
    ) -> DriveCdrom<'a> {
        let image = match self.drive_map.entry(drive_index).map(|entry| entry.device) {
            Some(DriveDevice::GuestCdrom(_)) => {
                Some(self.guest_cdroms.image(drive_index).unwrap_or(&[]))
            }
            Some(
                DriveDevice::Cdrom
                | DriveDevice::Floppy { .. }
                | DriveDevice::FixedDisk { .. }
                | DriveDevice::MemoryImage { .. }
                | DriveDevice::GuestDriver(_)
                | DriveDevice::Virtual,
            )
            | None => None,
        };
        DriveCdrom { machine, image }
    }

    /// Checks that the image of a guest CD-ROM drive is loaded and that the
    /// current entry checked its medium. Otherwise asks for the load or the
    /// media check and a rerun of the entry and fails with "not ready".
    /// Other drives pass.
    pub(crate) fn require_guest_cdrom(&mut self, drive_index: u8) -> Result<(), u16> {
        if self.guest_cdrom_device(drive_index).is_none() {
            return Ok(());
        }
        let bit = drive_bit(drive_index);
        if self.guest_cdroms.load_failed & bit != 0 {
            return Err(ERROR_NOT_READY);
        }
        if self.guest_cdroms.loaded & bit == 0 {
            self.guest_requests
                .steps
                .push(GuestStep::Cdrom(GuestCdromStep::Open { drive_index }));
            self.guest_requests.redispatch = true;
            return Err(ERROR_NOT_READY);
        }
        if self.guest_cdroms.media_checked & bit == 0 {
            self.guest_requests
                .steps
                .push(GuestStep::Cdrom(GuestCdromStep::MediaCheck { drive_index }));
            self.guest_requests.redispatch = true;
            return Err(ERROR_NOT_READY);
        }
        Ok(())
    }

    /// Returns the device header of a guest CD-ROM drive.
    pub(crate) fn guest_cdrom_device(&self, drive_index: u8) -> Option<FarPointer> {
        match self.drive_map.entry(drive_index)?.device {
            DriveDevice::GuestCdrom(device) => {
                Some(FarPointer::new(device.header_segment, device.header_offset))
            }
            DriveDevice::Cdrom
            | DriveDevice::Floppy { .. }
            | DriveDevice::FixedDisk { .. }
            | DriveDevice::MemoryImage { .. }
            | DriveDevice::GuestDriver(_)
            | DriveDevice::Virtual => None,
        }
    }

    /// Returns the CD transfer buffer of the layout.
    fn cdrom_transfer_buffer(&self) -> Option<FarPointer> {
        self.structures()
            .private_area
            .transfer_buffer
            .map(|offset| FarPointer::new(self.dos_data_segment(), offset))
    }

    /// Marks a failed load of a drive and drops its image.
    fn fail_guest_cdrom(&mut self, drive_index: u8) {
        self.guest_cdroms.load_failed |= drive_bit(drive_index);
        self.guest_cdroms.drop_image(drive_index);
    }
}

impl NeetanDos {
    /// Writes the request of a CD-ROM step and returns its guest call. A step
    /// without its device or transfer buffer fails the drive.
    pub(crate) fn prepare_cdrom_step(
        &mut self,
        step: &GuestCdromStep,
        area: &GuestCallArea,
        memory: &mut dyn MemoryAccess,
    ) -> StepStart {
        let drive_index = step.drive_index();
        let (Some(device), Some(transfer)) = (
            self.state.guest_cdrom_device(drive_index),
            self.state.cdrom_transfer_buffer(),
        ) else {
            self.state.fail_guest_cdrom(drive_index);
            return StepStart::Finished(Vec::new());
        };
        let request = area.request_address();
        match *step {
            GuestCdromStep::Open { .. } => {
                write_request_header(
                    memory,
                    request,
                    OPEN_CLOSE_REQUEST_LENGTH,
                    0,
                    COMMAND_DEVICE_OPEN,
                );
            }
            GuestCdromStep::Close { .. } => {
                write_request_header(
                    memory,
                    request,
                    OPEN_CLOSE_REQUEST_LENGTH,
                    0,
                    COMMAND_DEVICE_CLOSE,
                );
            }
            GuestCdromStep::MediaCheck { .. } => {
                write_media_changed_request(memory, request, transfer);
            }
            GuestCdromStep::ReadVolumeSize { .. } => {
                write_read_long(memory, request, transfer, VOLUME_DESCRIPTOR_SECTOR);
            }
            GuestCdromStep::ReadSector { sector, .. } => {
                write_read_long(memory, request, transfer, sector);
            }
        }
        StepStart::Call(GuestCall::DeviceRequest {
            device,
            data_segment: self.state.dos_data_segment(),
        })
    }

    /// Reads the result of a CD-ROM step and returns the steps that follow.
    pub(crate) fn finish_cdrom_step(
        &mut self,
        step: GuestCdromStep,
        area: &GuestCallArea,
        memory: &mut dyn MemoryAccess,
    ) -> Vec<GuestStep> {
        let drive_index = step.drive_index();
        let status = request_status(memory, area.request_address());
        let follow = |step: GuestCdromStep| vec![GuestStep::Cdrom(step)];
        match step {
            GuestCdromStep::Open { .. } => follow(GuestCdromStep::MediaCheck { drive_index }),
            GuestCdromStep::Close { .. } => Vec::new(),
            GuestCdromStep::MediaCheck { .. } => {
                if self.state.guest_cdroms.loaded & drive_bit(drive_index) == 0 {
                    return follow(GuestCdromStep::ReadVolumeSize { drive_index });
                }
                let answer = self
                    .state
                    .cdrom_transfer_buffer()
                    .map_or(MEDIA_CHANGED, |transfer| {
                        memory.read_byte(transfer.linear() + 1)
                    });
                if status_is_error(status) {
                    self.state.fail_guest_cdrom(drive_index);
                    return Vec::new();
                }
                if answer == MEDIA_NOT_CHANGED {
                    self.state.guest_cdroms.media_checked |= drive_bit(drive_index);
                    return Vec::new();
                }
                self.state.guest_cdroms.drop_image(drive_index);
                follow(GuestCdromStep::Open { drive_index })
            }
            GuestCdromStep::ReadVolumeSize { .. } if status_is_error(status) => {
                self.critical_error(step, status)
            }
            GuestCdromStep::ReadSector { .. } if status_is_error(status) => {
                self.critical_error(step, status)
            }
            GuestCdromStep::ReadVolumeSize { .. } => {
                let Some(sector) = self.read_transferred_sector(status, memory) else {
                    warn!(
                        "NeetanDos: the guest CD-ROM drive {drive_index} has no volume descriptor ({status:04X})"
                    );
                    self.state.fail_guest_cdrom(drive_index);
                    return follow(GuestCdromStep::Close { drive_index });
                };
                let Some(format) = VolumeFormat::of_primary_descriptor(&sector) else {
                    warn!(
                        "NeetanDos: the guest CD-ROM drive {drive_index} has no ISO 9660 or High Sierra volume"
                    );
                    self.state.fail_guest_cdrom(drive_index);
                    return follow(GuestCdromStep::Close { drive_index });
                };
                let volume_size = format.volume_space_size(&sector);
                if volume_size <= VOLUME_DESCRIPTOR_SECTOR || volume_size > IMAGE_SECTOR_LIMIT {
                    warn!(
                        "NeetanDos: the guest CD-ROM volume of {volume_size} sectors is not loaded"
                    );
                    self.state.fail_guest_cdrom(drive_index);
                    return follow(GuestCdromStep::Close { drive_index });
                }
                self.state.guest_cdroms.images[usize::from(drive_index)] =
                    Some(vec![0; volume_size as usize * CDROM_SECTOR_SIZE]);
                follow(GuestCdromStep::ReadSector {
                    drive_index,
                    sector: 0,
                })
            }
            GuestCdromStep::ReadSector { sector, .. } => {
                let data = self.read_transferred_sector(status, memory);
                let Some(image) = self.state.guest_cdroms.images[usize::from(drive_index)].as_mut()
                else {
                    return follow(GuestCdromStep::Close { drive_index });
                };
                let Some(data) = data else {
                    warn!("NeetanDos: read long of sector {sector} failed ({status:04X})");
                    self.state.fail_guest_cdrom(drive_index);
                    return follow(GuestCdromStep::Close { drive_index });
                };
                let start = sector as usize * CDROM_SECTOR_SIZE;
                image[start..start + CDROM_SECTOR_SIZE].copy_from_slice(&data);
                if start + CDROM_SECTOR_SIZE < image.len() {
                    follow(GuestCdromStep::ReadSector {
                        drive_index,
                        sector: sector + 1,
                    })
                } else {
                    self.state.guest_cdroms.loaded |= drive_bit(drive_index);
                    self.state.guest_cdroms.media_checked |= drive_bit(drive_index);
                    follow(GuestCdromStep::Close { drive_index })
                }
            }
        }
    }

    /// Marks the load of the drive of `step` as failed and asks the
    /// `INT 24h` handler for the driver error of `status`. Retry runs `step`
    /// again, fail closes the device.
    fn critical_error(&mut self, step: GuestCdromStep, status: u16) -> Vec<GuestStep> {
        let drive_index = step.drive_index();
        self.state.guest_cdroms.load_failed |= drive_bit(drive_index);
        let Some(device) = self.state.guest_cdrom_device(drive_index) else {
            return Vec::new();
        };
        vec![GuestStep::CriticalError(CriticalErrorStep {
            drive_index,
            error: status as u8,
            flags: DiskArea::Data.handler_flags(false, false),
            device,
            retry: vec![GuestStep::Cdrom(step)],
            ignore: Vec::new(),
            fail: vec![GuestStep::Cdrom(GuestCdromStep::Close { drive_index })],
        })]
    }

    /// Returns the sector in the transfer buffer after a read long with
    /// `status`, `None` after an error.
    fn read_transferred_sector(&self, status: u16, memory: &dyn MemoryAccess) -> Option<Vec<u8>> {
        if status_is_error(status) {
            return None;
        }
        let transfer = self.state.cdrom_transfer_buffer()?;
        let mut sector = vec![0u8; CDROM_SECTOR_SIZE];
        memory.read_block(transfer.linear(), &mut sector);
        Some(sector)
    }
}

/// Writes an IOCTL input request of the media changed control block
/// (`09h`) at `transfer`.
fn write_media_changed_request(memory: &mut dyn MemoryAccess, request: u32, transfer: FarPointer) {
    write_request_header(
        memory,
        request,
        IOCTL_INPUT_REQUEST_LENGTH,
        0,
        COMMAND_IOCTL_INPUT,
    );
    memory.write_word(request + REQUEST_OFF_TRANSFER, transfer.offset);
    memory.write_word(request + REQUEST_OFF_TRANSFER + 2, transfer.segment);
    memory.write_word(request + IOCTL_OFF_COUNT, MEDIA_CHANGED_BLOCK_LENGTH);
    memory.write_byte(transfer.linear(), MEDIA_CHANGED_CODE);
    memory.write_byte(transfer.linear() + 1, 0);
}

/// Writes a read long request of one cooked sector into `transfer`.
fn write_read_long(memory: &mut dyn MemoryAccess, request: u32, transfer: FarPointer, sector: u32) {
    write_request_header(
        memory,
        request,
        READ_LONG_REQUEST_LENGTH,
        0,
        COMMAND_READ_LONG,
    );
    memory.write_byte(request + READ_LONG_OFF_ADDRESSING_MODE, 0);
    memory.write_word(request + REQUEST_OFF_TRANSFER, transfer.offset);
    memory.write_word(request + REQUEST_OFF_TRANSFER + 2, transfer.segment);
    memory.write_word(request + READ_LONG_OFF_COUNT, 1);
    memory.write_word(request + READ_LONG_OFF_START_SECTOR, sector as u16);
    memory.write_word(
        request + READ_LONG_OFF_START_SECTOR + 2,
        (sector >> 16) as u16,
    );
    memory.write_byte(request + READ_LONG_OFF_READ_MODE, 0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_round_trip_through_the_save_state() {
        for step in [
            GuestCdromStep::Open { drive_index: 17 },
            GuestCdromStep::MediaCheck { drive_index: 17 },
            GuestCdromStep::ReadVolumeSize { drive_index: 17 },
            GuestCdromStep::ReadSector {
                drive_index: 17,
                sector: 0x12345,
            },
            GuestCdromStep::Close { drive_index: 17 },
        ] {
            let mut encoded = Vec::new();
            save_state::StateEncode::encode_state(&step, &mut encoded);
            let mut decoder = save_state::StateDecoder::new(&encoded, 1 << 10);
            let decoded: GuestCdromStep =
                save_state::StateDecode::decode_state(&mut decoder).expect("step decodes");
            assert_eq!(decoded, step);
        }
    }

    #[test]
    fn read_long_requests_one_cooked_sector() {
        let mut memory = crate::test_support::MockMemory::with_extended_memory(0x10_0000, 0);
        write_read_long(
            &mut memory,
            0x1000,
            FarPointer::new(0x2000, 0x4300),
            0x0001_0010,
        );
        assert_eq!(memory.read_byte(0x1000), READ_LONG_REQUEST_LENGTH);
        assert_eq!(memory.read_byte(0x1002), COMMAND_READ_LONG);
        assert_eq!(memory.read_word(0x1000 + REQUEST_OFF_TRANSFER), 0x4300);
        assert_eq!(memory.read_word(0x1000 + REQUEST_OFF_TRANSFER + 2), 0x2000);
        assert_eq!(memory.read_word(0x1000 + READ_LONG_OFF_COUNT), 1);
        assert_eq!(
            memory.read_word(0x1000 + READ_LONG_OFF_START_SECTOR),
            0x0010
        );
        assert_eq!(
            memory.read_word(0x1000 + READ_LONG_OFF_START_SECTOR + 2),
            0x0001
        );
    }
}
