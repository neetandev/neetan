//! A SCSI target device attached to a host adapter front end.
//!
//! Wraps the concrete target implementations behind one dispatch type so a
//! host adapter can drive any attached device through the same CDB and
//! data-phase interface.

use crate::scsi::{cdrom::ScsiCdrom, command::Direction, disk::ScsiDisk};

/// A SCSI target device selectable on the bus.
#[derive(Debug)]
pub enum ScsiTarget {
    /// Direct-access hard disk (device type 0x00).
    Disk(ScsiDisk),
    /// Read-only CD-ROM with audio playback (device type 0x05).
    Cdrom(ScsiCdrom),
}

impl ScsiTarget {
    /// Returns the stable save-state kind tag for this target.
    pub fn state_kind(&self) -> u8 {
        match self {
            ScsiTarget::Disk(_) => 1,
            ScsiTarget::Cdrom(_) => 2,
        }
    }

    /// Returns the current in-memory bytes of a disk target's image.
    pub fn disk_image_bytes(&self) -> Option<Vec<u8>> {
        match self {
            ScsiTarget::Disk(disk) => Some(disk.image_bytes()),
            ScsiTarget::Cdrom(_) => None,
        }
    }

    /// Captures a disk target's command sense latch.
    pub fn capture_disk_state(&self) -> Option<crate::scsi::command::SenseData> {
        match self {
            ScsiTarget::Disk(disk) => Some(disk.capture_state()),
            ScsiTarget::Cdrom(_) => None,
        }
    }

    /// Restores a disk target's command sense latch.
    pub fn restore_disk_state(
        &mut self,
        state: crate::scsi::command::SenseData,
    ) -> Result<(), save_state::StateValidationError> {
        match self {
            ScsiTarget::Disk(disk) => {
                disk.restore_state(state);
                Ok(())
            }
            ScsiTarget::Cdrom(_) => Err(save_state::StateValidationError::new(
                "SCSI target type differs",
            )),
        }
    }

    /// Captures a CD-ROM target's electronics and playback state.
    pub fn capture_cdrom_state(&self) -> Option<crate::scsi::cdrom::ScsiCdromState> {
        match self {
            ScsiTarget::Disk(_) => None,
            ScsiTarget::Cdrom(cdrom) => Some(cdrom.capture_state()),
        }
    }

    /// Validates a CD-ROM target state against retained media.
    pub fn validate_cdrom_state(
        &self,
        state: &crate::scsi::cdrom::ScsiCdromState,
    ) -> Result<(), save_state::StateValidationError> {
        match self {
            ScsiTarget::Disk(_) => Err(save_state::StateValidationError::new(
                "SCSI target type differs",
            )),
            ScsiTarget::Cdrom(cdrom) => cdrom.validate_state(state),
        }
    }

    /// Restores a CD-ROM target's electronics and playback state.
    pub fn restore_cdrom_state(
        &mut self,
        state: crate::scsi::cdrom::ScsiCdromState,
    ) -> Result<(), save_state::StateValidationError> {
        match self {
            ScsiTarget::Disk(_) => Err(save_state::StateValidationError::new(
                "SCSI target type differs",
            )),
            ScsiTarget::Cdrom(cdrom) => cdrom.restore_state(state),
        }
    }

    /// Returns a mounted-media binding for a disk target.
    pub fn disk_media_binding(
        &self,
        identifier: impl Into<String>,
        drive_index: u32,
    ) -> Result<Option<save_state::MediaBinding>, save_state::StateValidationError> {
        match self {
            ScsiTarget::Disk(disk) => disk.media_binding(identifier, drive_index).map(Some),
            ScsiTarget::Cdrom(_) => Ok(None),
        }
    }

    /// The data-phase direction a command requires.
    pub fn direction(&self, cdb: &[u8]) -> Direction {
        match self {
            ScsiTarget::Disk(disk) => disk.direction(cdb),
            ScsiTarget::Cdrom(cdrom) => cdrom.direction(cdb),
        }
    }

    /// Number of DATA OUT bytes a command expects.
    pub fn data_out_length(&self, cdb: &[u8]) -> usize {
        match self {
            ScsiTarget::Disk(disk) => disk.data_out_length(cdb),
            ScsiTarget::Cdrom(cdrom) => cdrom.data_out_length(cdb),
        }
    }

    /// Executes a command with no data phase and returns the STATUS byte.
    pub fn execute_no_data(&mut self, cdb: &[u8]) -> u8 {
        match self {
            ScsiTarget::Disk(disk) => disk.execute_no_data(cdb),
            ScsiTarget::Cdrom(cdrom) => cdrom.execute_no_data(cdb),
        }
    }

    /// Produces the DATA IN bytes for a read-type command, plus its STATUS byte.
    pub fn data_in(&mut self, cdb: &[u8]) -> (Vec<u8>, u8) {
        match self {
            ScsiTarget::Disk(disk) => disk.data_in(cdb),
            ScsiTarget::Cdrom(cdrom) => cdrom.data_in(cdb),
        }
    }

    /// Consumes the DATA OUT bytes for a write command and returns its STATUS.
    pub fn write_data_out(&mut self, cdb: &[u8], data: &[u8]) -> u8 {
        match self {
            ScsiTarget::Disk(disk) => disk.write_data_out(cdb, data),
            ScsiTarget::Cdrom(cdrom) => cdrom.write_data_out(cdb, data),
        }
    }

    /// Flushes pending writes to the backing file.
    pub fn flush(&mut self) {
        match self {
            ScsiTarget::Disk(disk) => disk.flush(),
            ScsiTarget::Cdrom(cdrom) => cdrom.flush(),
        }
    }
}
