//! Machine-neutral SASI target command engine.
//!
//! Implements the drive-side SASI command set shared by the PC-9801-27
//! interface board and the X68000 internal hard disk controller. Host
//! adapter front ends own bus phases, signaling, and DMA; the engine owns
//! command decoding, sector transfer state, and sense reporting.

use crate::disk::MountedHdd;

/// SASI sector size in bytes.
pub(super) const SASI_BLOCK_SIZE: usize = 256;

/// Test Drive Ready command opcode.
const COMMAND_TEST_DRIVE_READY: u8 = 0x00;
/// Recalibrate command opcode.
const COMMAND_RECALIBRATE: u8 = 0x01;
/// Request Sense command opcode.
const COMMAND_REQUEST_SENSE: u8 = 0x03;
/// Format Drive command opcode.
const COMMAND_FORMAT_DRIVE: u8 = 0x04;
/// Format Track command opcode.
const COMMAND_FORMAT_TRACK: u8 = 0x06;
/// Read command opcode.
const COMMAND_READ: u8 = 0x08;
/// Write command opcode.
const COMMAND_WRITE: u8 = 0x0A;
/// Seek command opcode.
const COMMAND_SEEK: u8 = 0x0B;
/// Vendor-specific Assign Drive command opcode.
const COMMAND_ASSIGN_DRIVE: u8 = 0xC2;

/// Number of parameter bytes following an Assign Drive command.
const ASSIGN_DRIVE_PARAMETER_COUNT: u8 = 10;

/// Host-adapter-specific SASI target behavior parameters.
#[derive(Debug, Clone, Copy)]
pub(super) struct SasiTargetProfile {
    /// Mask applied to the logical unit number from command byte 1.
    pub lun_mask: u8,
    /// Sense error code for a ready check against an absent drive.
    pub drive_not_ready: u8,
    /// Sense error code for a transfer or format against an absent drive.
    pub missing_drive_failure: u8,
    /// Sense error code for an unreadable, unwritable, or out-of-range sector.
    pub sector_failure: u8,
    /// Completion code stored for commands the target does not implement.
    pub unknown_command: u8,
    /// Completion code stored after a Format Drive command.
    pub format_drive_completion: u8,
    /// Number of sectors erased by one Format Block command.
    pub format_track_span: u32,
    /// Whether a zero block count in Read/Write means 256 blocks.
    pub zero_count_means_256: bool,
    /// Whether transfers and seeks are range-checked before they start.
    pub validate_transfer_range: bool,
}

/// Target behavior matching the PC-9801-27 controller.
pub(super) const PC98_TARGET_PROFILE: SasiTargetProfile = SasiTargetProfile {
    lun_mask: 0x01,
    drive_not_ready: 0x7F,
    missing_drive_failure: 0x0F,
    sector_failure: 0x0F,
    unknown_command: 0x00,
    format_drive_completion: 0x0F,
    format_track_span: 1,
    zero_count_means_256: false,
    validate_transfer_range: false,
};

/// Target behavior matching the X68000 internal hard disk controller.
pub(super) const X68K_TARGET_PROFILE: SasiTargetProfile = SasiTargetProfile {
    lun_mask: 0x07,
    drive_not_ready: 0x20,
    missing_drive_failure: 0x20,
    sector_failure: 0x21,
    unknown_command: 0x20,
    format_drive_completion: 0x00,
    format_track_span: 33,
    zero_count_means_256: true,
    validate_transfer_range: true,
};

/// Outcome of a complete six-byte command delivered to the target engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SasiCommandStart {
    /// The command finished; the front end enters its completion flow.
    Complete,
    /// The target streams sector data to the initiator from the block buffer.
    DataIn,
    /// The target expects sector data from the initiator into the block buffer.
    DataOut,
    /// The target streams the four sense bytes.
    Sense,
    /// The target expects vendor-command parameter bytes.
    VendorParameters {
        /// Number of parameter bytes the target expects.
        count: u8,
    },
    /// The front end must erase the addressed drive before completing.
    FormatDrive,
    /// The front end must fill the addressed track with the format filler.
    FormatTrack,
}

/// Progress of a byte-granular SASI data transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SasiTransferStep {
    /// The transfer continues with more data bytes.
    Continue,
    /// The transfer finished successfully.
    Complete,
    /// The transfer aborted and recorded an error code.
    Failed,
}

/// Drive-side SASI command and transfer state.
#[derive(Debug)]
pub(super) struct SasiTargetEngine {
    profile: SasiTargetProfile,
    unit: u8,
    sector: u32,
    blocks_remaining: u16,
    data_buffer: [u8; SASI_BLOCK_SIZE],
    data_position: usize,
    data_size: usize,
    sense_data: [u8; 4],
    status: u8,
    error_code: u8,
}

save_state::runtime_state! {
/// Mutable SASI target command and transfer state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SasiTargetEngineState {
    unit: u8,
    sector: u32,
    blocks_remaining: u16,
    data_buffer: [u8; SASI_BLOCK_SIZE],
    data_position: usize,
    data_size: usize,
    sense_data: [u8; 4],
    status: u8,
    error_code: u8,
}}

impl SasiTargetEngine {
    /// Creates an idle engine with the given host adapter behavior.
    pub(super) fn new(profile: SasiTargetProfile) -> Self {
        Self {
            profile,
            unit: 0,
            sector: 0,
            blocks_remaining: 0,
            data_buffer: [0; SASI_BLOCK_SIZE],
            data_position: 0,
            data_size: 0,
            sense_data: [0; 4],
            status: 0,
            error_code: 0,
        }
    }

    pub(super) fn capture_state(&self) -> SasiTargetEngineState {
        SasiTargetEngineState {
            unit: self.unit,
            sector: self.sector,
            blocks_remaining: self.blocks_remaining,
            data_buffer: self.data_buffer,
            data_position: self.data_position,
            data_size: self.data_size,
            sense_data: self.sense_data,
            status: self.status,
            error_code: self.error_code,
        }
    }

    pub(super) fn restore_state(&mut self, state: SasiTargetEngineState) {
        self.unit = state.unit;
        self.sector = state.sector;
        self.blocks_remaining = state.blocks_remaining;
        self.data_buffer = state.data_buffer;
        self.data_position = state.data_position;
        self.data_size = state.data_size;
        self.sense_data = state.sense_data;
        self.status = state.status;
        self.error_code = state.error_code;
    }

    pub(super) fn validate_state(
        state: &SasiTargetEngineState,
    ) -> Result<(), save_state::StateValidationError> {
        if state.data_position > state.data_size || state.data_size > SASI_BLOCK_SIZE {
            return Err(save_state::StateValidationError::new(
                "SASI target buffer position is invalid",
            ));
        }
        Ok(())
    }

    /// Returns the currently selected unit (drive) number.
    pub(super) fn current_unit(&self) -> u8 {
        self.unit
    }

    /// Returns the current sector address.
    pub(super) fn current_sector(&self) -> u32 {
        self.sector
    }

    /// Returns the four sense bytes prepared by Request Sense.
    pub(super) fn sense_bytes(&self) -> [u8; 4] {
        self.sense_data
    }

    /// Returns the status byte for the status phase.
    pub(super) fn status_byte(&self) -> u8 {
        if self.error_code == 0 {
            self.status
        } else {
            0x02
        }
    }

    /// Returns the filled portion of the block buffer.
    pub(super) fn buffer(&self) -> &[u8] {
        &self.data_buffer[..self.data_size]
    }

    /// Decodes and starts a six-byte command against the attached drives.
    pub(super) fn begin_command(
        &mut self,
        command: &[u8; 6],
        drives: &[Option<MountedHdd>],
    ) -> SasiCommandStart {
        self.unit = (command[1] >> 5) & self.profile.lun_mask;
        let drive_present = self.drive(drives).is_some();

        match command[0] {
            COMMAND_TEST_DRIVE_READY => {
                self.finish_ready_check(drive_present);
                SasiCommandStart::Complete
            }
            COMMAND_RECALIBRATE => {
                if drive_present {
                    self.sector = 0;
                }
                self.finish_ready_check(drive_present);
                SasiCommandStart::Complete
            }
            COMMAND_REQUEST_SENSE => {
                self.sense_data[0] = self.error_code;
                self.sense_data[1] = (self.unit << 5) | ((self.sector >> 16) as u8 & 0x1F);
                self.sense_data[2] = (self.sector >> 8) as u8;
                self.sense_data[3] = self.sector as u8;
                self.error_code = 0x00;
                self.status = 0x00;
                SasiCommandStart::Sense
            }
            COMMAND_FORMAT_DRIVE => {
                self.sector = 0;
                self.status = 0;
                if self.drive(drives).is_none() {
                    self.error_code = self.profile.missing_drive_failure;
                    return SasiCommandStart::Complete;
                }
                self.error_code = self.profile.format_drive_completion;
                SasiCommandStart::FormatDrive
            }
            COMMAND_FORMAT_TRACK => {
                self.sector = Self::sector_address(command);
                self.status = 0;
                let Some(drive) = self.drive(drives) else {
                    self.error_code = self.profile.missing_drive_failure;
                    return SasiCommandStart::Complete;
                };
                let span = u64::from(self.profile.format_track_span);
                if u64::from(self.sector) + span > u64::from(drive.geometry().total_sectors()) {
                    self.error_code = self.profile.sector_failure;
                    return SasiCommandStart::Complete;
                }
                self.error_code = 0x00;
                SasiCommandStart::FormatTrack
            }
            COMMAND_READ => match self.begin_transfer(command, drives) {
                Ok(()) => SasiCommandStart::DataIn,
                Err(()) => SasiCommandStart::Complete,
            },
            COMMAND_WRITE => match self.begin_transfer(command, drives) {
                Ok(()) => SasiCommandStart::DataOut,
                Err(()) => SasiCommandStart::Complete,
            },
            COMMAND_SEEK => {
                self.sector = Self::sector_address(command);
                self.blocks_remaining = u16::from(command[4]);
                self.status = 0x00;
                if self.profile.validate_transfer_range {
                    let Some(drive) = self.drive(drives) else {
                        self.error_code = self.profile.missing_drive_failure;
                        return SasiCommandStart::Complete;
                    };
                    if self.sector >= drive.geometry().total_sectors() {
                        self.error_code = self.profile.sector_failure;
                        return SasiCommandStart::Complete;
                    }
                }
                self.error_code = 0x00;
                SasiCommandStart::Complete
            }
            COMMAND_ASSIGN_DRIVE => {
                self.status = 0x00;
                SasiCommandStart::VendorParameters {
                    count: ASSIGN_DRIVE_PARAMETER_COUNT,
                }
            }
            _ => {
                self.error_code = self.profile.unknown_command;
                SasiCommandStart::Complete
            }
        }
    }

    /// Marks the vendor-command parameter phase as successfully finished.
    pub(super) fn complete_vendor_parameters(&mut self) {
        self.error_code = 0x00;
    }

    /// Reads the next data byte of an active read transfer.
    pub(super) fn read_byte(&mut self, drives: &[Option<MountedHdd>]) -> (u8, SasiTransferStep) {
        let value = self.data_buffer[self.data_position];
        self.data_position += 1;
        if self.data_position < self.data_size {
            return (value, SasiTransferStep::Continue);
        }
        self.blocks_remaining -= 1;
        if self.blocks_remaining == 0 {
            self.error_code = 0x00;
            return (value, SasiTransferStep::Complete);
        }
        self.sector += 1;
        if self.load_block(drives) {
            (value, SasiTransferStep::Continue)
        } else {
            self.error_code = self.profile.sector_failure;
            (value, SasiTransferStep::Failed)
        }
    }

    /// Buffers one initiator byte; returns true when the block buffer is full.
    pub(super) fn push_write_byte(&mut self, value: u8) -> bool {
        self.data_buffer[self.data_position] = value;
        self.data_position += 1;
        self.data_position >= self.data_size
    }

    /// Commits the filled block buffer to disk and advances the transfer.
    pub(super) fn commit_write_block(
        &mut self,
        drives: &mut [Option<MountedHdd>],
    ) -> SasiTransferStep {
        let unit = self.unit as usize;
        let Some(drive) = drives.get_mut(unit).and_then(Option::as_mut) else {
            self.error_code = self.profile.sector_failure;
            return SasiTransferStep::Failed;
        };
        if !drive.write_sector(self.sector, &self.data_buffer[..self.data_size]) {
            self.error_code = self.profile.sector_failure;
            return SasiTransferStep::Failed;
        }
        self.advance_write_block()
    }

    /// Advances a buffered write without committing it to disk. Returns the
    /// (unit, sector) whose buffer the front end must persist.
    pub(super) fn finish_buffered_write_block(
        &mut self,
        drives: &[Option<MountedHdd>],
    ) -> (Option<(u8, u32)>, SasiTransferStep) {
        if self.drive(drives).is_none() {
            self.error_code = self.profile.sector_failure;
            return (None, SasiTransferStep::Failed);
        }
        let block = (self.unit, self.sector);
        let step = self.advance_write_block();
        (Some(block), step)
    }

    fn begin_transfer(
        &mut self,
        command: &[u8; 6],
        drives: &[Option<MountedHdd>],
    ) -> Result<(), ()> {
        self.sector = Self::sector_address(command);
        let raw_count = u16::from(command[4]);
        self.blocks_remaining = if raw_count == 0 && self.profile.zero_count_means_256 {
            256
        } else {
            raw_count
        };
        self.status = 0;
        let Some(drive) = self.drive(drives) else {
            self.error_code = self.profile.missing_drive_failure;
            return Err(());
        };
        if self.blocks_remaining == 0 {
            self.error_code = self.profile.sector_failure;
            return Err(());
        }
        if self.profile.validate_transfer_range
            && u64::from(self.sector) + u64::from(self.blocks_remaining)
                > u64::from(drive.geometry().total_sectors())
        {
            self.error_code = self.profile.sector_failure;
            return Err(());
        }
        if !self.load_block(drives) {
            self.error_code = self.profile.sector_failure;
            return Err(());
        }
        Ok(())
    }

    fn advance_write_block(&mut self) -> SasiTransferStep {
        self.blocks_remaining -= 1;
        if self.blocks_remaining == 0 {
            self.error_code = 0x00;
            SasiTransferStep::Complete
        } else {
            self.sector += 1;
            self.data_position = 0;
            SasiTransferStep::Continue
        }
    }

    fn finish_ready_check(&mut self, drive_present: bool) {
        if drive_present {
            self.status = 0x00;
            self.error_code = 0x00;
        } else {
            self.status = 0x02;
            self.error_code = self.profile.drive_not_ready;
        }
    }

    fn sector_address(command: &[u8; 6]) -> u32 {
        ((command[1] & 0x1F) as u32) << 16 | (command[2] as u32) << 8 | command[3] as u32
    }

    fn load_block(&mut self, drives: &[Option<MountedHdd>]) -> bool {
        self.data_position = 0;
        self.data_size = 0;

        let unit = self.unit as usize;
        let Some(drive) = drives.get(unit).and_then(Option::as_ref) else {
            return false;
        };
        if drive.geometry().sector_size != SASI_BLOCK_SIZE as u16 {
            return false;
        }
        let Some(sector_data) = drive.read_sector(self.sector) else {
            return false;
        };
        self.data_buffer.copy_from_slice(sector_data);
        self.data_size = SASI_BLOCK_SIZE;
        true
    }

    fn drive<'drives>(&self, drives: &'drives [Option<MountedHdd>]) -> Option<&'drives MountedHdd> {
        drives.get(self.unit as usize).and_then(Option::as_ref)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk::{HddFormat, HddGeometry, HddImage};

    fn make_test_drive() -> MountedHdd {
        let geometry = HddGeometry {
            cylinders: 153,
            heads: 4,
            sectors_per_track: 33,
            sector_size: 256,
        };
        let total = geometry.total_bytes() as usize;
        let mut data = vec![0u8; total];
        for lba in 0..geometry.total_sectors() {
            let offset = lba as usize * 256;
            data[offset] = (lba >> 8) as u8;
            data[offset + 1] = lba as u8;
        }
        MountedHdd::new(HddImage::from_raw(geometry, HddFormat::Thd, data), None)
    }

    fn command(bytes: [u8; 6]) -> [u8; 6] {
        bytes
    }

    #[test]
    fn test_drive_ready_reports_presence() {
        let mut engine = SasiTargetEngine::new(PC98_TARGET_PROFILE);
        let drives = [Some(make_test_drive()), None];

        let start = engine.begin_command(&command([0x00, 0, 0, 0, 0, 0]), &drives);
        assert_eq!(start, SasiCommandStart::Complete);
        assert_eq!(engine.status_byte(), 0x00);

        let start = engine.begin_command(&command([0x00, 0x20, 0, 0, 0, 0]), &drives);
        assert_eq!(start, SasiCommandStart::Complete);
        assert_eq!(engine.status_byte(), 0x02);
    }

    #[test]
    fn recalibrate_resets_sector() {
        let mut engine = SasiTargetEngine::new(PC98_TARGET_PROFILE);
        let drives = [Some(make_test_drive()), None];

        engine.begin_command(&command([0x08, 0, 0, 5, 1, 0]), &drives);
        assert_eq!(engine.current_sector(), 5);

        engine.begin_command(&command([0x01, 0, 0, 0, 0, 0]), &drives);
        assert_eq!(engine.current_sector(), 0);
    }

    #[test]
    fn request_sense_reports_last_error_and_address() {
        let mut engine = SasiTargetEngine::new(PC98_TARGET_PROFILE);
        let drives = [Some(make_test_drive()), None];

        // Read past the end of the drive to record a sector failure.
        let start = engine.begin_command(&command([0x08, 0x1F, 0xFF, 0xFF, 1, 0]), &drives);
        assert_eq!(start, SasiCommandStart::Complete);
        assert_eq!(engine.status_byte(), 0x02);

        let start = engine.begin_command(&command([0x03, 0, 0, 0, 0, 0]), &drives);
        assert_eq!(start, SasiCommandStart::Sense);
        let sense = engine.sense_bytes();
        assert_eq!(sense[0], PC98_TARGET_PROFILE.sector_failure);
        assert_eq!(sense[1], 0x1F);
        assert_eq!(sense[2], 0xFF);
        assert_eq!(sense[3], 0xFF);
        assert_eq!(engine.status_byte(), 0x00);
    }

    #[test]
    fn read_streams_blocks_and_completes() {
        let mut engine = SasiTargetEngine::new(PC98_TARGET_PROFILE);
        let drives = [Some(make_test_drive()), None];

        let start = engine.begin_command(&command([0x08, 0, 0, 0, 2, 0]), &drives);
        assert_eq!(start, SasiCommandStart::DataIn);

        let mut last_step = SasiTransferStep::Continue;
        let mut bytes = Vec::new();
        for _ in 0..512 {
            let (value, step) = engine.read_byte(&drives);
            bytes.push(value);
            last_step = step;
        }
        assert_eq!(last_step, SasiTransferStep::Complete);
        assert_eq!(bytes[0], 0x00);
        assert_eq!(bytes[1], 0x00);
        assert_eq!(bytes[256], 0x00);
        assert_eq!(bytes[257], 0x01);
    }

    #[test]
    fn read_with_zero_block_count_fails() {
        let mut engine = SasiTargetEngine::new(PC98_TARGET_PROFILE);
        let drives = [Some(make_test_drive()), None];

        let start = engine.begin_command(&command([0x08, 0, 0, 0, 0, 0]), &drives);
        assert_eq!(start, SasiCommandStart::Complete);
        assert_eq!(engine.status_byte(), 0x02);
    }

    #[test]
    fn committed_write_persists_blocks() {
        let mut engine = SasiTargetEngine::new(PC98_TARGET_PROFILE);
        let mut drives = [Some(make_test_drive()), None];

        let start = engine.begin_command(&command([0x0A, 0, 0, 5, 1, 0]), &drives);
        assert_eq!(start, SasiCommandStart::DataOut);

        for _ in 0..255 {
            assert!(!engine.push_write_byte(0xAA));
        }
        assert!(engine.push_write_byte(0xAA));
        assert_eq!(
            engine.commit_write_block(&mut drives),
            SasiTransferStep::Complete
        );

        let sector = drives[0].as_ref().unwrap().read_sector(5).unwrap();
        assert!(sector.iter().all(|&byte| byte == 0xAA));
    }

    #[test]
    fn buffered_write_reports_block_for_external_commit() {
        let mut engine = SasiTargetEngine::new(PC98_TARGET_PROFILE);
        let drives = [Some(make_test_drive()), None];

        let start = engine.begin_command(&command([0x0A, 0, 0, 7, 2, 0]), &drives);
        assert_eq!(start, SasiCommandStart::DataOut);

        for _ in 0..256 {
            engine.push_write_byte(0x5A);
        }
        let (block, step) = engine.finish_buffered_write_block(&drives);
        assert_eq!(block, Some((0, 7)));
        assert_eq!(step, SasiTransferStep::Continue);
        assert_eq!(engine.buffer().len(), 256);

        for _ in 0..256 {
            engine.push_write_byte(0x5A);
        }
        let (block, step) = engine.finish_buffered_write_block(&drives);
        assert_eq!(block, Some((0, 8)));
        assert_eq!(step, SasiTransferStep::Complete);
    }

    #[test]
    fn vendor_command_expects_ten_parameter_bytes() {
        let mut engine = SasiTargetEngine::new(PC98_TARGET_PROFILE);
        let drives = [Some(make_test_drive()), None];

        let start = engine.begin_command(&command([0xC2, 0, 0, 0, 0, 0]), &drives);
        assert_eq!(start, SasiCommandStart::VendorParameters { count: 10 });

        engine.complete_vendor_parameters();
        assert_eq!(engine.status_byte(), 0x00);
    }

    #[test]
    fn format_track_validates_sector_range() {
        let mut engine = SasiTargetEngine::new(PC98_TARGET_PROFILE);
        let drives = [Some(make_test_drive()), None];

        let start = engine.begin_command(&command([0x06, 0, 0, 0, 0, 0]), &drives);
        assert_eq!(start, SasiCommandStart::FormatTrack);

        let start = engine.begin_command(&command([0x06, 0x1F, 0xFF, 0xFF, 0, 0]), &drives);
        assert_eq!(start, SasiCommandStart::Complete);
        assert_eq!(engine.status_byte(), 0x02);
    }

    #[test]
    fn format_drive_uses_profile_completion_code() {
        let mut engine = SasiTargetEngine::new(PC98_TARGET_PROFILE);
        let drives = [Some(make_test_drive()), None];

        let start = engine.begin_command(&command([0x04, 0, 0, 0, 0, 0]), &drives);
        assert_eq!(start, SasiCommandStart::FormatDrive);
        assert_eq!(engine.status_byte(), 0x02);
        assert_eq!(engine.sense_bytes()[0], 0x00);

        engine.begin_command(&command([0x03, 0, 0, 0, 0, 0]), &drives);
        assert_eq!(
            engine.sense_bytes()[0],
            PC98_TARGET_PROFILE.format_drive_completion
        );
    }

    #[test]
    fn unknown_command_uses_profile_code() {
        let quiet = SasiTargetProfile {
            unknown_command: 0x00,
            ..PC98_TARGET_PROFILE
        };
        let strict = SasiTargetProfile {
            unknown_command: 0x20,
            ..PC98_TARGET_PROFILE
        };
        let drives = [Some(make_test_drive()), None];

        let mut engine = SasiTargetEngine::new(quiet);
        let start = engine.begin_command(&command([0xFF, 0, 0, 0, 0, 0]), &drives);
        assert_eq!(start, SasiCommandStart::Complete);
        assert_eq!(engine.status_byte(), 0x00);

        let mut engine = SasiTargetEngine::new(strict);
        let start = engine.begin_command(&command([0xFF, 0, 0, 0, 0, 0]), &drives);
        assert_eq!(start, SasiCommandStart::Complete);
        assert_eq!(engine.status_byte(), 0x02);
    }

    #[test]
    fn lun_mask_selects_unit() {
        let mut engine = SasiTargetEngine::new(PC98_TARGET_PROFILE);
        let drives = [None, Some(make_test_drive())];

        engine.begin_command(&command([0x00, 0x20, 0, 0, 0, 0]), &drives);
        assert_eq!(engine.current_unit(), 1);
        assert_eq!(engine.status_byte(), 0x00);
    }
}
