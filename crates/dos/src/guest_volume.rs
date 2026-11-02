//! Block units of guest device drivers.
//!
//! The FAT code of NeetanDos reads sectors synchronously through `DiskIo`.
//! A guest driver answers only through guest calls. NeetanDos therefore
//! loads the whole volume of a guest unit into a host image: build BPB
//! (`02h`), after a read of the first FAT sector for an IBM format unit, and
//! reads (`04h`) of every sector. The FAT code works on the image through
//! `GuestVolumeDisk`, a `DiskIo` that serves the image device ids and passes
//! every other id to the machine. A sector write updates the image and
//! queues a write request (`08h`, `09h` with verify) to the driver.
//!
//! Every entry checks the medium of a loaded unit once with a media check
//! (`01h`). A changed medium drops the image and loads it again. The answer
//! "don't know" does the same, like DOS for a drive without dirty buffers:
//! the writes of the entry run in front of the media check.
//!
//! A read of the load never crosses the end of the reserved sectors, the
//! FATs or the root directory. A driver error of a media check, a build BPB,
//! a read or a write asks the `INT 24h` handler (`critical_error.rs`) with
//! the area of the sector. A media check or build BPB error reports the FAT
//! area and its retry checks the medium again, like DOS. Ignore is allowed
//! for reads and writes of the data area. An ignored read leaves the failed
//! sector empty in the image and the load continues behind it. An ignored
//! write counts as written. A failed write reaches the program as the DOS
//! error of the call.
//!
//! The first access of an entry to an unloaded or unchecked unit fails with
//! "not ready" and asks for the guest steps and a rerun of the entry.

use std::collections::VecDeque;

use common::warn;

use crate::{
    CdromIo, DiskIo, DosState, MemoryAccess, NeetanDos,
    critical_error::{CriticalErrorStep, DiskArea},
    device_request::{
        BUILD_BPB_REQUEST_LENGTH, COMMAND_BUILD_BPB, COMMAND_MEDIA_CHECK, COMMAND_READ,
        COMMAND_WRITE, COMMAND_WRITE_VERIFY, MEDIA_CHANGED, MEDIA_CHECK_REQUEST_LENGTH,
        MEDIA_UNKNOWN, REQUEST_OFF_BPB_POINTER, REQUEST_OFF_COUNT, REQUEST_OFF_MEDIA,
        REQUEST_OFF_MEDIA_CHANGE, REQUEST_OFF_TRANSFER, TRANSFER_REQUEST_LENGTH, request_status,
        status_dos_error, status_is_error, write_request_header, write_transfer_fields,
    },
    drive_map::{self, DriveDevice, DriveEntry, GuestBlockUnit},
    filesystem::fat_bpb::{Bpb, PARAMETER_BLOCK_LENGTH},
    guest_call::{FarPointer, GuestCall, GuestCallArea, GuestRequests, GuestStep, StepStart},
    tables,
};

/// `DiskIo` device id of the image of drive A:. Drive n uses this id plus n.
pub(crate) const GUEST_VOLUME_DEVICE_ID_FIRST: u8 = 0xE0;

/// Largest volume image in bytes.
pub(crate) const GUEST_VOLUME_IMAGE_LIMIT: usize = 32 * 1024 * 1024;

/// Device attribute bit of a block driver that does not need the FAT sector
/// for build BPB.
const DEVATTR_NON_IBM_FORMAT: u16 = 0x2000;

/// Logical sector that holds the first FAT sector of an IBM format medium.
const FIRST_FAT_SECTOR: u16 = 1;

/// Largest sector number of a DOS 3 read or write request.
const LAST_REQUEST_SECTOR: u32 = 0xFFFF;

/// DOS error code of a drive that is not ready.
pub(crate) const ERROR_NOT_READY: u16 = 0x0015;

/// Returns the `DiskIo` device id of the image of a drive.
pub(crate) const fn image_device_id(drive_index: u8) -> u8 {
    GUEST_VOLUME_DEVICE_ID_FIRST + drive_index
}

/// Returns the drive of an image device id.
fn image_drive(device_id: u8) -> Option<u8> {
    device_id
        .checked_sub(GUEST_VOLUME_DEVICE_ID_FIRST)
        .filter(|&drive_index| usize::from(drive_index) < drive_map::DRIVE_LETTER_COUNT)
}

/// Returns the bit of a drive in a drive mask.
const fn drive_bit(drive_index: u8) -> u32 {
    1 << drive_index
}

save_state::runtime_state! {
    /// Host copy of the volume of a guest block unit.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) struct GuestVolumeImage {
        /// BPB that build BPB reported.
        pub(crate) bpb: Bpb,
        /// Every sector of the volume.
        pub(crate) sectors: Vec<u8>,
    }
}

impl GuestVolumeImage {
    /// Returns the byte range of `count` sectors from `first`, if it lies in the image.
    fn range(&self, first: u32, count: u32) -> Option<std::ops::Range<usize>> {
        let sector_size = usize::from(self.bpb.bytes_per_sector);
        let start = usize::try_from(first).ok()? * sector_size;
        let end = start + usize::try_from(count).ok()? * sector_size;
        (end <= self.sectors.len()).then_some(start..end)
    }
}

save_state::runtime_state! {
    /// Images and per entry state of the guest block units.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) struct GuestVolumes {
        /// Volume image per drive letter.
        pub(crate) images: Vec<Option<GuestVolumeImage>>,
        /// Drives with a complete image.
        pub(crate) loaded: u32,
        /// Drives whose medium the current entry checked.
        pub(crate) media_checked: u32,
        /// Drives whose check or load failed in the current entry.
        pub(crate) load_failed: u32,
    }
}

impl Default for GuestVolumes {
    fn default() -> Self {
        Self {
            images: vec![None; drive_map::DRIVE_LETTER_COUNT],
            loaded: 0,
            media_checked: 0,
            load_failed: 0,
        }
    }
}

impl GuestVolumes {
    /// Forgets the media checks and failures of the previous entry.
    pub(crate) fn begin_entry(&mut self) {
        self.media_checked = 0;
        self.load_failed = 0;
    }

    /// Drops the image of a drive.
    pub(crate) fn drop_image(&mut self, drive_index: u8) {
        if let Some(slot) = self.images.get_mut(usize::from(drive_index)) {
            *slot = None;
        }
        self.loaded &= !drive_bit(drive_index);
    }
}

save_state::runtime_state! {
    /// Sector write to a guest block unit that waits for its write request.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) struct SectorWrite {
        /// Drive of the unit.
        pub(crate) drive_index: u8,
        /// Logical sector.
        pub(crate) sector: u16,
        /// Sector data.
        pub(crate) data: Vec<u8>,
    }
}

/// A request to a guest block unit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GuestBlockStep {
    /// Media check (`01h`).
    MediaCheck {
        /// Drive of the unit.
        drive_index: u8,
    },
    /// Read (`04h`) of the first FAT sector in front of build BPB.
    ReadFatSector {
        /// Drive of the unit.
        drive_index: u8,
    },
    /// Build BPB (`02h`).
    BuildBpb {
        /// Drive of the unit.
        drive_index: u8,
    },
    /// Read (`04h`) of sectors into the image.
    ReadSectors {
        /// Drive of the unit.
        drive_index: u8,
        /// First logical sector.
        first_sector: u16,
        /// Number of sectors.
        count: u16,
    },
    /// Write (`08h`) or write with verify (`09h`) of one sector.
    WriteSector {
        /// Pending write.
        write: SectorWrite,
        /// True for write with verify.
        verify: bool,
    },
    /// Load of the sectors from `next_sector` on, after an ignored read.
    ContinueLoad {
        /// Drive of the unit.
        drive_index: u8,
        /// First sector that the load still reads.
        next_sector: u32,
    },
    /// Failed write: the DOS call of the operation fails with `error`.
    FailWrite {
        /// DOS error code of the write.
        error: u16,
    },
}

/// Save state tag of `GuestBlockStep::MediaCheck`.
const BLOCK_TAG_MEDIA_CHECK: u8 = 0;
/// Save state tag of `GuestBlockStep::ReadFatSector`.
const BLOCK_TAG_READ_FAT_SECTOR: u8 = 1;
/// Save state tag of `GuestBlockStep::BuildBpb`.
const BLOCK_TAG_BUILD_BPB: u8 = 2;
/// Save state tag of `GuestBlockStep::ReadSectors`.
const BLOCK_TAG_READ_SECTORS: u8 = 3;
/// Save state tag of `GuestBlockStep::WriteSector`.
const BLOCK_TAG_WRITE_SECTOR: u8 = 4;
/// Save state tag of `GuestBlockStep::ContinueLoad`.
const BLOCK_TAG_CONTINUE_LOAD: u8 = 5;
/// Save state tag of `GuestBlockStep::FailWrite`.
const BLOCK_TAG_FAIL_WRITE: u8 = 6;

impl save_state::StateEncode for GuestBlockStep {
    fn encode_state(&self, output: &mut Vec<u8>) {
        match self {
            Self::MediaCheck { drive_index } => {
                save_state::StateEncode::encode_state(&BLOCK_TAG_MEDIA_CHECK, output);
                save_state::StateEncode::encode_state(drive_index, output);
            }
            Self::ReadFatSector { drive_index } => {
                save_state::StateEncode::encode_state(&BLOCK_TAG_READ_FAT_SECTOR, output);
                save_state::StateEncode::encode_state(drive_index, output);
            }
            Self::BuildBpb { drive_index } => {
                save_state::StateEncode::encode_state(&BLOCK_TAG_BUILD_BPB, output);
                save_state::StateEncode::encode_state(drive_index, output);
            }
            Self::ReadSectors {
                drive_index,
                first_sector,
                count,
            } => {
                save_state::StateEncode::encode_state(&BLOCK_TAG_READ_SECTORS, output);
                save_state::StateEncode::encode_state(drive_index, output);
                save_state::StateEncode::encode_state(first_sector, output);
                save_state::StateEncode::encode_state(count, output);
            }
            Self::WriteSector { write, verify } => {
                save_state::StateEncode::encode_state(&BLOCK_TAG_WRITE_SECTOR, output);
                save_state::StateEncode::encode_state(write, output);
                save_state::StateEncode::encode_state(verify, output);
            }
            Self::ContinueLoad {
                drive_index,
                next_sector,
            } => {
                save_state::StateEncode::encode_state(&BLOCK_TAG_CONTINUE_LOAD, output);
                save_state::StateEncode::encode_state(drive_index, output);
                save_state::StateEncode::encode_state(next_sector, output);
            }
            Self::FailWrite { error } => {
                save_state::StateEncode::encode_state(&BLOCK_TAG_FAIL_WRITE, output);
                save_state::StateEncode::encode_state(error, output);
            }
        }
    }
}

impl save_state::StateDecode for GuestBlockStep {
    fn decode_state(
        decoder: &mut save_state::StateDecoder<'_>,
    ) -> Result<Self, save_state::StateDecodeError> {
        match <u8 as save_state::StateDecode>::decode_state(decoder)? {
            BLOCK_TAG_MEDIA_CHECK => Ok(Self::MediaCheck {
                drive_index: save_state::StateDecode::decode_state(decoder)?,
            }),
            BLOCK_TAG_READ_FAT_SECTOR => Ok(Self::ReadFatSector {
                drive_index: save_state::StateDecode::decode_state(decoder)?,
            }),
            BLOCK_TAG_BUILD_BPB => Ok(Self::BuildBpb {
                drive_index: save_state::StateDecode::decode_state(decoder)?,
            }),
            BLOCK_TAG_READ_SECTORS => Ok(Self::ReadSectors {
                drive_index: save_state::StateDecode::decode_state(decoder)?,
                first_sector: save_state::StateDecode::decode_state(decoder)?,
                count: save_state::StateDecode::decode_state(decoder)?,
            }),
            BLOCK_TAG_WRITE_SECTOR => Ok(Self::WriteSector {
                write: save_state::StateDecode::decode_state(decoder)?,
                verify: save_state::StateDecode::decode_state(decoder)?,
            }),
            BLOCK_TAG_CONTINUE_LOAD => Ok(Self::ContinueLoad {
                drive_index: save_state::StateDecode::decode_state(decoder)?,
                next_sector: save_state::StateDecode::decode_state(decoder)?,
            }),
            BLOCK_TAG_FAIL_WRITE => Ok(Self::FailWrite {
                error: save_state::StateDecode::decode_state(decoder)?,
            }),
            _ => Err(save_state::StateDecodeError::InvalidTag),
        }
    }
}

/// `DiskIo` that serves the images of guest block units and passes every
/// other device to the machine.
pub(crate) struct GuestVolumeDisk<'a, Device: DiskIo + CdromIo> {
    /// Disk and CD-ROM access of the machine.
    machine: &'a mut Device,
    /// Volume images per drive letter.
    images: Vec<Option<GuestVolumeImage>>,
    /// Sector writes of this handler run.
    writes: Vec<SectorWrite>,
}

impl<'a, Device: DiskIo + CdromIo> GuestVolumeDisk<'a, Device> {
    /// Wraps the machine access and the images.
    pub(crate) fn new(machine: &'a mut Device, images: Vec<Option<GuestVolumeImage>>) -> Self {
        Self {
            machine,
            images,
            writes: Vec::new(),
        }
    }

    /// Returns the images and the queued writes.
    pub(crate) fn into_parts(self) -> (Vec<Option<GuestVolumeImage>>, Vec<SectorWrite>) {
        (self.images, self.writes)
    }

    /// Returns the image behind a device id.
    fn image(&self, device_id: u8) -> Option<&GuestVolumeImage> {
        image_drive(device_id)
            .and_then(|drive_index| self.images.get(usize::from(drive_index)))
            .and_then(Option::as_ref)
    }
}

impl<Device: DiskIo + CdromIo> DiskIo for GuestVolumeDisk<'_, Device> {
    fn read_sectors(&mut self, device_id: u8, lba: u32, count: u32) -> Result<Vec<u8>, u8> {
        match self.image(device_id) {
            Some(image) => image
                .range(lba, count)
                .map(|range| image.sectors[range].to_vec())
                .ok_or(0xFF),
            None => self.machine.read_sectors(device_id, lba, count),
        }
    }

    fn write_sectors(&mut self, device_id: u8, lba: u32, data: &[u8]) -> Result<(), u8> {
        let Some(drive_index) = image_drive(device_id).filter(|_| self.image(device_id).is_some())
        else {
            return self.machine.write_sectors(device_id, lba, data);
        };
        let image = self.images[usize::from(drive_index)]
            .as_mut()
            .expect("image of the device id");
        let sector_size = usize::from(image.bpb.bytes_per_sector);
        if !data.len().is_multiple_of(sector_size) {
            return Err(0xFF);
        }
        let count = (data.len() / sector_size) as u32;
        let range = image.range(lba, count).ok_or(0xFFu8)?;
        image.sectors[range].copy_from_slice(data);
        for (index, chunk) in data.chunks(sector_size).enumerate() {
            self.writes.push(SectorWrite {
                drive_index,
                sector: (lba + index as u32) as u16,
                data: chunk.to_vec(),
            });
        }
        Ok(())
    }

    fn sector_size(&self, device_id: u8) -> Option<u16> {
        match self.image(device_id) {
            Some(image) => Some(image.bpb.bytes_per_sector),
            None => self.machine.sector_size(device_id),
        }
    }

    fn total_sectors(&self, device_id: u8) -> Option<u32> {
        match self.image(device_id) {
            Some(image) => Some(image.bpb.total_sectors()),
            None => DiskIo::total_sectors(self.machine, device_id),
        }
    }

    fn drive_geometry(&self, device_id: u8) -> Option<(u16, u8, u8)> {
        match self.image(device_id) {
            Some(_) => None,
            None => self.machine.drive_geometry(device_id),
        }
    }
}

impl<Device: DiskIo + CdromIo> CdromIo for GuestVolumeDisk<'_, Device> {
    fn cdrom_present(&self) -> bool {
        self.machine.cdrom_present()
    }

    fn cdrom_media_loaded(&self) -> bool {
        self.machine.cdrom_media_loaded()
    }

    fn read_sector_cooked(&self, lba: u32, buf: &mut [u8]) -> Option<usize> {
        self.machine.read_sector_cooked(lba, buf)
    }

    fn read_sector_raw(&self, lba: u32, buf: &mut [u8]) -> Option<usize> {
        self.machine.read_sector_raw(lba, buf)
    }

    fn track_count(&self) -> u8 {
        self.machine.track_count()
    }

    fn track_info(&self, track_number: u8) -> Option<common::CdromTrackInfo> {
        self.machine.track_info(track_number)
    }

    fn leadout_lba(&self) -> u32 {
        self.machine.leadout_lba()
    }

    fn total_sectors(&self) -> u32 {
        CdromIo::total_sectors(self.machine)
    }

    fn audio_play(&mut self, start_lba: u32, sector_count: u32) {
        self.machine.audio_play(start_lba, sector_count);
    }

    fn audio_stop(&mut self) {
        self.machine.audio_stop();
    }

    fn audio_resume(&mut self) {
        self.machine.audio_resume();
    }

    fn audio_state(&self) -> common::CdAudioStatus {
        self.machine.audio_state()
    }

    fn audio_channel_info(&self) -> common::AudioChannelInfo {
        self.machine.audio_channel_info()
    }

    fn set_audio_channel_info(&mut self, info: &common::AudioChannelInfo) {
        self.machine.set_audio_channel_info(info);
    }
}

impl DosState {
    /// Checks that the image of a guest unit is loaded and that the current
    /// entry checked its medium. Otherwise asks for the guest steps and a
    /// rerun of the entry and fails with "not ready".
    pub(crate) fn require_guest_volume(&mut self, drive_index: u8) -> Result<u8, u16> {
        let bit = drive_bit(drive_index);
        let volumes = &mut self.guest_volumes;
        if volumes.load_failed & bit != 0 {
            return Err(ERROR_NOT_READY);
        }
        if volumes.loaded & bit == 0 {
            self.guest_requests.volume_loads |= bit;
            self.guest_requests.redispatch = true;
            return Err(ERROR_NOT_READY);
        }
        if volumes.media_checked & bit == 0 {
            self.guest_requests.media_checks |= bit;
            self.guest_requests.redispatch = true;
            return Err(ERROR_NOT_READY);
        }
        Ok(image_device_id(drive_index))
    }

    /// Returns the guest block unit of a drive.
    fn guest_unit(&self, drive_index: u8) -> Option<GuestBlockUnit> {
        match self.drive_map.entry(drive_index)?.device {
            DriveDevice::GuestDriver(unit) => Some(unit),
            DriveDevice::Floppy { .. }
            | DriveDevice::FixedDisk { .. }
            | DriveDevice::MemoryImage { .. }
            | DriveDevice::Cdrom
            | DriveDevice::GuestCdrom(_)
            | DriveDevice::Virtual => None,
        }
    }

    /// Returns the area of logical sector `sector` of a drive: from the BPB
    /// of its image, else from the BPB of the drive map.
    fn drive_area(&self, drive_index: u8, sector: u32) -> DiskArea {
        let image_bpb = self.guest_volumes.images[usize::from(drive_index)]
            .as_ref()
            .map(|image| image.bpb);
        let bpb = image_bpb.or_else(|| {
            self.drive_map
                .entry(drive_index)
                .and_then(|entry| entry.geometry)
                .map(|geometry| geometry.bpb)
        });
        bpb.map_or(DiskArea::Data, |bpb| DiskArea::of_sector(&bpb, sector))
    }

    /// Returns the media descriptor of the current BPB of a drive.
    fn drive_media(&self, drive_index: u8) -> u8 {
        self.drive_map
            .entry(drive_index)
            .and_then(|entry| entry.geometry)
            .map_or(0, |geometry| geometry.bpb.media_descriptor)
    }

    /// Returns the sector data buffer of the first disk buffer and its size
    /// (the largest sector size of the list of lists).
    fn transfer_buffer(&self, memory: &dyn MemoryAccess) -> (FarPointer, u16) {
        let list_of_lists = self.list_of_lists_address();
        let header = FarPointer::read(memory, list_of_lists + tables::SYSVARS_OFF_BUFFER_PTR);
        let capacity = memory.read_word(list_of_lists + tables::SYSVARS_OFF_MAX_SECTOR);
        let data = FarPointer::new(
            header.segment,
            header
                .offset
                .wrapping_add(self.structures().disk_buffer.header_size as u16),
        );
        (data, capacity)
    }

    /// Marks a failed check or load of a drive and drops its image.
    pub(crate) fn fail_guest_volume(&mut self, drive_index: u8) {
        self.guest_volumes.load_failed |= drive_bit(drive_index);
        self.guest_volumes.drop_image(drive_index);
        if let Some(slot) = self.fat_volumes.get_mut(usize::from(drive_index)) {
            *slot = None;
        }
    }
}

impl NeetanDos {
    /// Appends the block steps of the requests of a handler run.
    pub(crate) fn queue_block_steps(
        &self,
        requests: &GuestRequests,
        steps: &mut VecDeque<GuestStep>,
    ) {
        for write in &requests.sector_writes {
            steps.push_back(GuestStep::Block(GuestBlockStep::WriteSector {
                write: write.clone(),
                verify: self.state.verify_writes,
            }));
        }
        for drive_index in 0..drive_map::DRIVE_LETTER_COUNT as u8 {
            let bit = drive_bit(drive_index);
            if requests.volume_loads & bit != 0 {
                steps.extend(Self::load_steps(drive_index));
            } else if requests.media_checks & bit != 0 {
                steps.push_back(GuestStep::Block(GuestBlockStep::MediaCheck { drive_index }));
            }
        }
    }

    /// Returns the first step of a volume load.
    fn load_steps(drive_index: u8) -> Vec<GuestStep> {
        vec![GuestStep::Block(GuestBlockStep::ReadFatSector {
            drive_index,
        })]
    }

    /// Writes the request of a block step and returns its guest call. A
    /// step without a call finishes at once.
    pub(crate) fn prepare_block_step(
        &mut self,
        step: &GuestBlockStep,
        area: &GuestCallArea,
        memory: &mut dyn MemoryAccess,
    ) -> StepStart {
        let drive_index = match step {
            GuestBlockStep::MediaCheck { drive_index }
            | GuestBlockStep::ReadFatSector { drive_index }
            | GuestBlockStep::BuildBpb { drive_index }
            | GuestBlockStep::ReadSectors { drive_index, .. } => *drive_index,
            GuestBlockStep::WriteSector { write, .. } => write.drive_index,
            GuestBlockStep::ContinueLoad {
                drive_index,
                next_sector,
            } => {
                return StepStart::Finished(self.continue_load(*drive_index, *next_sector, memory));
            }
            GuestBlockStep::FailWrite { error } => {
                if let Some(operation) = self.guest_calls.last_mut() {
                    operation.write_error.get_or_insert(*error);
                }
                return StepStart::Finished(Vec::new());
            }
        };
        let Some(unit) = self.state.guest_unit(drive_index) else {
            return self.skip_block_step(step);
        };
        let device = FarPointer::new(unit.header_segment, unit.header_offset);
        let request = area.request_address();
        let media = self.state.drive_media(drive_index);
        let (buffer, capacity) = self.state.transfer_buffer(memory);
        match step {
            GuestBlockStep::MediaCheck { .. } => {
                write_request_header(
                    memory,
                    request,
                    MEDIA_CHECK_REQUEST_LENGTH,
                    unit.unit,
                    COMMAND_MEDIA_CHECK,
                );
                memory.write_byte(request + REQUEST_OFF_MEDIA, media);
            }
            GuestBlockStep::ReadFatSector { .. } => {
                let sector_size = self
                    .state
                    .drive_map
                    .entry(drive_index)
                    .and_then(|entry| entry.geometry)
                    .map_or(0, |geometry| geometry.bpb.bytes_per_sector);
                let attribute = memory.read_word(device.linear() + tables::DEVHDR_OFF_ATTRIBUTE);
                if attribute & DEVATTR_NON_IBM_FORMAT != 0 {
                    return StepStart::Finished(vec![GuestStep::Block(GuestBlockStep::BuildBpb {
                        drive_index,
                    })]);
                }
                if sector_size == 0 || sector_size > capacity {
                    return self.skip_block_step(step);
                }
                write_request_header(
                    memory,
                    request,
                    TRANSFER_REQUEST_LENGTH,
                    unit.unit,
                    COMMAND_READ,
                );
                write_transfer_fields(
                    memory,
                    request,
                    media,
                    buffer.segment,
                    buffer.offset,
                    1,
                    FIRST_FAT_SECTOR,
                );
            }
            GuestBlockStep::BuildBpb { .. } => {
                write_request_header(
                    memory,
                    request,
                    BUILD_BPB_REQUEST_LENGTH,
                    unit.unit,
                    COMMAND_BUILD_BPB,
                );
                memory.write_byte(request + REQUEST_OFF_MEDIA, media);
                tables::write_far_ptr(
                    memory,
                    request + REQUEST_OFF_TRANSFER,
                    buffer.segment,
                    buffer.offset,
                );
            }
            GuestBlockStep::ReadSectors {
                first_sector,
                count,
                ..
            } => {
                if self.state.guest_volumes.load_failed & drive_bit(drive_index) != 0 {
                    return StepStart::Finished(Vec::new());
                }
                write_request_header(
                    memory,
                    request,
                    TRANSFER_REQUEST_LENGTH,
                    unit.unit,
                    COMMAND_READ,
                );
                write_transfer_fields(
                    memory,
                    request,
                    media,
                    buffer.segment,
                    buffer.offset,
                    *count,
                    *first_sector,
                );
            }
            GuestBlockStep::WriteSector { write, verify } => {
                if write.data.len() > usize::from(capacity) {
                    return self.skip_block_step(step);
                }
                memory.write_block(buffer.linear(), &write.data);
                let command = if *verify {
                    COMMAND_WRITE_VERIFY
                } else {
                    COMMAND_WRITE
                };
                write_request_header(memory, request, TRANSFER_REQUEST_LENGTH, unit.unit, command);
                write_transfer_fields(
                    memory,
                    request,
                    media,
                    buffer.segment,
                    buffer.offset,
                    1,
                    write.sector,
                );
            }
            GuestBlockStep::ContinueLoad { .. } | GuestBlockStep::FailWrite { .. } => {
                return StepStart::Finished(Vec::new());
            }
        }
        StepStart::Call(GuestCall::DeviceRequest {
            device,
            data_segment: area.segment(),
        })
    }

    /// Reads the result of a block step and returns the steps that follow
    /// from it.
    pub(crate) fn finish_block_step(
        &mut self,
        step: GuestBlockStep,
        area: &GuestCallArea,
        memory: &mut dyn MemoryAccess,
    ) -> Vec<GuestStep> {
        let request = area.request_address();
        let status = request_status(memory, request);
        match step {
            GuestBlockStep::MediaCheck { drive_index } => {
                if status_is_error(status) {
                    return self.block_critical_error(
                        drive_index,
                        status,
                        DiskArea::Fat.handler_flags(false, false),
                        vec![GuestStep::Block(step)],
                        Vec::new(),
                        Vec::new(),
                    );
                }
                let media_change = memory.read_byte(request + REQUEST_OFF_MEDIA_CHANGE);
                if media_change == MEDIA_CHANGED || media_change == MEDIA_UNKNOWN {
                    self.state.guest_volumes.drop_image(drive_index);
                    self.state.fat_volumes[usize::from(drive_index)] = None;
                    return Self::load_steps(drive_index);
                }
                self.state.guest_volumes.media_checked |= drive_bit(drive_index);
                Vec::new()
            }
            GuestBlockStep::ReadFatSector { drive_index } => {
                if status_is_error(status) {
                    let area = self
                        .state
                        .drive_area(drive_index, u32::from(FIRST_FAT_SECTOR));
                    return self.block_critical_error(
                        drive_index,
                        status,
                        area.handler_flags(false, false),
                        vec![GuestStep::Block(step)],
                        Vec::new(),
                        Vec::new(),
                    );
                }
                vec![GuestStep::Block(GuestBlockStep::BuildBpb { drive_index })]
            }
            GuestBlockStep::BuildBpb { drive_index } => {
                if status_is_error(status) {
                    return self.block_critical_error(
                        drive_index,
                        status,
                        DiskArea::Fat.handler_flags(false, false),
                        vec![GuestStep::Block(GuestBlockStep::MediaCheck { drive_index })],
                        Vec::new(),
                        Vec::new(),
                    );
                }
                self.finish_build_bpb(drive_index, request, memory)
            }
            GuestBlockStep::ReadSectors {
                drive_index,
                first_sector,
                count,
            } => self.finish_read_sectors(drive_index, first_sector, count, request, memory),
            GuestBlockStep::WriteSector { ref write, .. } => {
                if !status_is_error(status) {
                    return Vec::new();
                }
                warn!(
                    "Guest block driver of drive {} failed to write sector {} (status {status:#06X})",
                    char::from(b'A' + write.drive_index),
                    write.sector
                );
                let drive_index = write.drive_index;
                let area = self.state.drive_area(drive_index, u32::from(write.sector));
                self.block_critical_error(
                    drive_index,
                    status,
                    area.handler_flags(true, true),
                    vec![GuestStep::Block(step.clone())],
                    Vec::new(),
                    vec![GuestStep::Block(GuestBlockStep::FailWrite {
                        error: status_dos_error(status),
                    })],
                )
            }
            GuestBlockStep::ContinueLoad { .. } | GuestBlockStep::FailWrite { .. } => Vec::new(),
        }
    }

    /// Asks the `INT 24h` handler for the driver error of `status` of a
    /// request to the unit of `drive_index`. A drive without its unit fails
    /// at once.
    fn block_critical_error(
        &mut self,
        drive_index: u8,
        status: u16,
        flags: u8,
        retry: Vec<GuestStep>,
        ignore: Vec<GuestStep>,
        fail: Vec<GuestStep>,
    ) -> Vec<GuestStep> {
        let Some(unit) = self.state.guest_unit(drive_index) else {
            self.state.fail_guest_volume(drive_index);
            return fail;
        };
        vec![GuestStep::CriticalError(CriticalErrorStep {
            drive_index,
            error: status as u8,
            flags,
            device: FarPointer::new(unit.header_segment, unit.header_offset),
            retry,
            ignore,
            fail,
        })]
    }

    /// Takes the BPB of a build BPB request and starts the sector reads. A
    /// sector larger than the disk buffer or an image above the limit fails
    /// the unit.
    fn finish_build_bpb(
        &mut self,
        drive_index: u8,
        request: u32,
        memory: &dyn MemoryAccess,
    ) -> Vec<GuestStep> {
        let bpb_pointer = FarPointer::read(memory, request + REQUEST_OFF_BPB_POINTER);
        let mut block = [0u8; PARAMETER_BLOCK_LENGTH];
        memory.read_block(bpb_pointer.linear(), &mut block);
        let (_, capacity) = self.state.transfer_buffer(memory);
        let bpb = Bpb::from_parameter_block(&block).filter(|bpb| {
            let total_sectors = bpb.total_sectors();
            bpb.bytes_per_sector <= capacity
                && total_sectors > 0
                && total_sectors <= LAST_REQUEST_SECTOR + 1
                && total_sectors as usize * usize::from(bpb.bytes_per_sector)
                    <= GUEST_VOLUME_IMAGE_LIMIT
        });
        let Some(bpb) = bpb else {
            self.state.fail_guest_volume(drive_index);
            return Vec::new();
        };

        if let Some(DriveEntry {
            geometry: Some(geometry),
            ..
        }) = self.state.drive_map.entry_mut(drive_index)
        {
            geometry.bpb = bpb;
        }
        self.state.stale_parameter_blocks |= drive_bit(drive_index);
        let size = bpb.total_sectors() as usize * usize::from(bpb.bytes_per_sector);
        self.state.guest_volumes.images[usize::from(drive_index)] = Some(GuestVolumeImage {
            bpb,
            sectors: vec![0; size],
        });
        self.next_read_step(drive_index, 0, memory)
    }

    /// Returns the read step of the sectors from `first_sector` on. A
    /// request ends at the end of the area of its first sector.
    fn next_read_step(
        &self,
        drive_index: u8,
        first_sector: u32,
        memory: &dyn MemoryAccess,
    ) -> Vec<GuestStep> {
        let Some(image) = &self.state.guest_volumes.images[usize::from(drive_index)] else {
            return Vec::new();
        };
        let area_end = DiskArea::end(&image.bpb, first_sector);
        let (_, capacity) = self.state.transfer_buffer(memory);
        let per_request = (u32::from(capacity) / u32::from(image.bpb.bytes_per_sector)).max(1);
        let count = per_request.min(area_end - first_sector);
        vec![GuestStep::Block(GuestBlockStep::ReadSectors {
            drive_index,
            first_sector: first_sector as u16,
            count: count as u16,
        })]
    }

    /// Copies the sectors of a read request into the image. A failed request
    /// keeps the sectors in front of the failed sector, and the answer
    /// ignore continues the load behind the failed sector.
    fn finish_read_sectors(
        &mut self,
        drive_index: u8,
        first_sector: u16,
        count: u16,
        request: u32,
        memory: &dyn MemoryAccess,
    ) -> Vec<GuestStep> {
        let status = request_status(memory, request);
        let request_count = memory.read_word(request + REQUEST_OFF_COUNT);
        let next_sector = u32::from(first_sector) + u32::from(count);
        if status_is_error(status) {
            let transferred = request_count.min(count.saturating_sub(1));
            self.copy_loaded_sectors(drive_index, first_sector, transferred, memory);
            let failed_sector = u32::from(first_sector) + u32::from(transferred);
            let area = self.state.drive_area(drive_index, failed_sector);
            return self.block_critical_error(
                drive_index,
                status,
                area.handler_flags(false, true),
                vec![GuestStep::Block(GuestBlockStep::ReadSectors {
                    drive_index,
                    first_sector,
                    count,
                })],
                vec![GuestStep::Block(GuestBlockStep::ContinueLoad {
                    drive_index,
                    next_sector: failed_sector + 1,
                })],
                Vec::new(),
            );
        }
        if request_count != count {
            self.state.fail_guest_volume(drive_index);
            return Vec::new();
        }
        if !self.copy_loaded_sectors(drive_index, first_sector, count, memory) {
            self.state.fail_guest_volume(drive_index);
            return Vec::new();
        }
        self.continue_load(drive_index, next_sector, memory)
    }

    /// Copies `count` sectors from `first_sector` on out of the transfer
    /// buffer into the image of the drive. Returns false when the image
    /// does not hold them.
    fn copy_loaded_sectors(
        &mut self,
        drive_index: u8,
        first_sector: u16,
        count: u16,
        memory: &dyn MemoryAccess,
    ) -> bool {
        let (buffer, _) = self.state.transfer_buffer(memory);
        let Some(image) = self.state.guest_volumes.images[usize::from(drive_index)].as_mut() else {
            return true;
        };
        let Some(range) = image.range(u32::from(first_sector), u32::from(count)) else {
            return false;
        };
        memory.read_block(buffer.linear(), &mut image.sectors[range]);
        true
    }

    /// Returns the read step of the sectors from `next_sector` on, or marks
    /// the image of the drive as loaded and checked after its last sector.
    fn continue_load(
        &mut self,
        drive_index: u8,
        next_sector: u32,
        memory: &dyn MemoryAccess,
    ) -> Vec<GuestStep> {
        let Some(image) = &self.state.guest_volumes.images[usize::from(drive_index)] else {
            return Vec::new();
        };
        if next_sector < image.bpb.total_sectors() {
            return self.next_read_step(drive_index, next_sector, memory);
        }
        let bit = drive_bit(drive_index);
        self.state.guest_volumes.loaded |= bit;
        self.state.guest_volumes.media_checked |= bit;
        Vec::new()
    }

    /// Fails a block step that cannot issue its guest call.
    fn skip_block_step(&mut self, step: &GuestBlockStep) -> StepStart {
        match step {
            GuestBlockStep::MediaCheck { drive_index }
            | GuestBlockStep::ReadFatSector { drive_index }
            | GuestBlockStep::BuildBpb { drive_index }
            | GuestBlockStep::ReadSectors { drive_index, .. } => {
                self.state.fail_guest_volume(*drive_index);
            }
            GuestBlockStep::WriteSector { write, .. } => {
                warn!(
                    "Guest block write of drive {} sector {} skipped",
                    char::from(b'A' + write.drive_index),
                    write.sector
                );
            }
            GuestBlockStep::ContinueLoad { .. } | GuestBlockStep::FailWrite { .. } => {}
        }
        StepStart::Finished(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_device_ids_map_back_to_their_drive() {
        assert_eq!(image_device_id(0), 0xE0);
        assert_eq!(image_drive(0xE4), Some(4));
        assert_eq!(image_drive(0xF9), Some(25));
        assert_eq!(image_drive(0xFA), None);
        assert_eq!(image_drive(0x90), None);
    }

    #[test]
    fn block_steps_round_trip_through_the_save_state() {
        for step in [
            GuestBlockStep::MediaCheck { drive_index: 3 },
            GuestBlockStep::ReadFatSector { drive_index: 3 },
            GuestBlockStep::BuildBpb { drive_index: 3 },
            GuestBlockStep::ReadSectors {
                drive_index: 3,
                first_sector: 10,
                count: 2,
            },
            GuestBlockStep::WriteSector {
                write: SectorWrite {
                    drive_index: 3,
                    sector: 7,
                    data: vec![1, 2, 3],
                },
                verify: true,
            },
            GuestBlockStep::ContinueLoad {
                drive_index: 3,
                next_sector: 0x1_0000,
            },
            GuestBlockStep::FailWrite { error: 0x001D },
        ] {
            let mut encoded = Vec::new();
            save_state::StateEncode::encode_state(&step, &mut encoded);
            let mut decoder = save_state::StateDecoder::new(&encoded, usize::MAX);
            let decoded: GuestBlockStep =
                save_state::StateDecode::decode_state(&mut decoder).unwrap();
            assert_eq!(decoded, step);
        }
    }
}
