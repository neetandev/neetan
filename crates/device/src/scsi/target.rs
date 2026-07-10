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
