//! A SCSI direct-access (hard disk) target backed by a [`MountedHdd`].
//!
//! This is machine-agnostic: it interprets the SCSI command set against a
//! 512-byte-sector image and tracks sense data across REQUEST SENSE. A host
//! adapter front-end feeds it CDBs and moves the data-phase bytes.

use crate::{
    disk::MountedHdd,
    scsi::command::{
        Direction, SenseData, asc, cdb_lun, opcode, read_write_lba_length, sense_key, status,
    },
};

/// SCSI disks use a fixed 512-byte block.
pub const SCSI_BLOCK_SIZE: usize = 512;

/// A direct-access SCSI target (LUN 0) over a mounted hard disk image.
#[derive(Debug)]
pub struct ScsiDisk {
    drive: MountedHdd,
    sense: SenseData,
}

impl ScsiDisk {
    /// Wraps a mounted image as a SCSI hard disk.
    pub fn new(drive: MountedHdd) -> Self {
        Self {
            drive,
            sense: SenseData::CLEAR,
        }
    }

    /// Captures the command sense latch.
    pub fn capture_state(&self) -> SenseData {
        self.sense
    }

    /// Restores the command sense latch.
    pub fn restore_state(&mut self, state: SenseData) {
        self.sense = state;
    }

    /// Returns a stable mounted-media binding for this target.
    pub fn media_binding(
        &self,
        identifier: impl Into<String>,
        drive_index: u32,
    ) -> Result<save_state::MediaBinding, save_state::StateValidationError> {
        let geometry = self.drive.geometry();
        Ok(save_state::MediaBinding {
            identifier: save_state::MediaBindingId::new(identifier)?,
            slot: save_state::MediaSlot::new(save_state::MediaKind::HardDisk, drive_index),
            source_path: self.drive.source_path().cloned(),
            media_type: self.drive.image().format_name().to_owned(),
            identity: self.drive.identity(),
            geometry: Some(save_state::MediaGeometry::new(
                u32::from(geometry.cylinders),
                u32::from(geometry.heads),
                u32::from(geometry.sectors_per_track),
                u32::from(geometry.sector_size),
            )?),
            write_protected: false,
            backend_generation: None,
        })
    }

    /// Flushes pending writes to the backing file.
    pub fn flush(&mut self) {
        self.drive.flush();
    }

    /// Total number of 512-byte blocks on the disk.
    fn block_count(&self) -> u32 {
        (self.drive.image().data().len() / SCSI_BLOCK_SIZE) as u32
    }

    /// Records sense data and returns CHECK CONDITION.
    fn fail(&mut self, key: u8, additional: u8) -> u8 {
        self.sense = SenseData::new(key, additional);
        status::CHECK_CONDITION
    }

    /// Clears sense data and returns GOOD.
    fn ok(&mut self) -> u8 {
        self.sense = SenseData::CLEAR;
        status::GOOD
    }

    /// The data-phase direction a command requires.
    pub fn direction(&self, cdb: &[u8]) -> Direction {
        match cdb.first().copied().unwrap_or(0xFF) {
            opcode::REQUEST_SENSE
            | opcode::INQUIRY
            | opcode::MODE_SENSE6
            | opcode::READ_CAPACITY
            | opcode::READ6
            | opcode::READ10 => Direction::In,
            opcode::WRITE6 | opcode::WRITE10 => Direction::Out,
            _ => Direction::None,
        }
    }

    /// Number of DATA OUT bytes a write command expects.
    pub fn data_out_length(&self, cdb: &[u8]) -> usize {
        match read_write_lba_length(cdb) {
            Some((_, blocks)) => blocks as usize * SCSI_BLOCK_SIZE,
            None => 0,
        }
    }

    /// Executes a command with no data phase (TEST UNIT READY, SEEK,
    /// START STOP, PREVENT/ALLOW, VERIFY). Returns the STATUS byte.
    pub fn execute_no_data(&mut self, cdb: &[u8]) -> u8 {
        if let Some(bad) = self.check_lun(cdb) {
            return bad;
        }
        match cdb.first().copied().unwrap_or(0xFF) {
            opcode::TEST_UNIT_READY
            | opcode::REZERO_UNIT
            | opcode::SEEK6
            | opcode::SEEK10
            | opcode::START_STOP
            | opcode::PREVENT_ALLOW
            | opcode::VERIFY10 => self.ok(),
            // A low-level format with no defect list clears the medium.
            opcode::FORMAT_UNIT => self.format_unit(),
            // A WRITE with a zero block count is a no-op (DOS6 installer quirk).
            opcode::WRITE6 | opcode::WRITE10 => self.ok(),
            _ => self.fail(sense_key::ILLEGAL_REQUEST, asc::INVALID_COMMAND),
        }
    }

    /// Clears every block of the medium for a FORMAT UNIT low-level format.
    fn format_unit(&mut self) -> u8 {
        let zero = [0u8; SCSI_BLOCK_SIZE];
        for lba in 0..self.block_count() {
            self.drive.write_sector(lba, &zero);
        }
        self.ok()
    }

    /// Produces the DATA IN bytes for a read-type command, plus its STATUS byte.
    pub fn data_in(&mut self, cdb: &[u8]) -> (Vec<u8>, u8) {
        if let Some(bad) = self.check_lun(cdb) {
            return (Vec::new(), bad);
        }
        match cdb.first().copied().unwrap_or(0xFF) {
            opcode::REQUEST_SENSE => {
                let data = self.request_sense_data(cdb);
                // REQUEST SENSE itself always succeeds and clears the sense.
                self.sense = SenseData::CLEAR;
                (data, status::GOOD)
            }
            opcode::INQUIRY => (self.inquiry_data(cdb), self.ok()),
            opcode::READ_CAPACITY => (self.read_capacity_data(), self.ok()),
            opcode::MODE_SENSE6 => (self.mode_sense_data(cdb), self.ok()),
            opcode::READ6 | opcode::READ10 => self.read_data(cdb),
            _ => (
                Vec::new(),
                self.fail(sense_key::ILLEGAL_REQUEST, asc::INVALID_COMMAND),
            ),
        }
    }

    /// Consumes the DATA OUT bytes for a write command and returns its STATUS.
    pub fn write_data_out(&mut self, cdb: &[u8], data: &[u8]) -> u8 {
        if let Some(bad) = self.check_lun(cdb) {
            return bad;
        }
        let Some((lba, blocks)) = read_write_lba_length(cdb) else {
            return self.fail(sense_key::ILLEGAL_REQUEST, asc::INVALID_FIELD_IN_CDB);
        };
        if blocks == 0 {
            return self.ok();
        }
        if lba as u64 + blocks as u64 > self.block_count() as u64 {
            return self.fail(sense_key::ILLEGAL_REQUEST, asc::LBA_OUT_OF_RANGE);
        }
        for index in 0..blocks {
            let offset = index as usize * SCSI_BLOCK_SIZE;
            let sector = &data[offset..offset + SCSI_BLOCK_SIZE];
            if !self.drive.write_sector(lba + index, sector) {
                return self.fail(sense_key::MEDIUM_ERROR, asc::NO_ADDITIONAL);
            }
        }
        self.ok()
    }

    /// Returns CHECK CONDITION status if the CDB targets a non-zero LUN.
    fn check_lun(&mut self, cdb: &[u8]) -> Option<u8> {
        if cdb_lun(cdb) != 0 {
            Some(self.fail(sense_key::ILLEGAL_REQUEST, asc::LOGICAL_UNIT_NOT_SUPPORTED))
        } else {
            None
        }
    }

    fn read_data(&mut self, cdb: &[u8]) -> (Vec<u8>, u8) {
        let Some((lba, blocks)) = read_write_lba_length(cdb) else {
            return (
                Vec::new(),
                self.fail(sense_key::ILLEGAL_REQUEST, asc::INVALID_FIELD_IN_CDB),
            );
        };
        if blocks == 0 {
            return (Vec::new(), self.ok());
        }
        if lba as u64 + blocks as u64 > self.block_count() as u64 {
            return (
                Vec::new(),
                self.fail(sense_key::ILLEGAL_REQUEST, asc::LBA_OUT_OF_RANGE),
            );
        }
        let mut out = Vec::with_capacity(blocks as usize * SCSI_BLOCK_SIZE);
        for index in 0..blocks {
            match self.drive.read_sector(lba + index) {
                Some(sector) => out.extend_from_slice(sector),
                None => {
                    return (
                        Vec::new(),
                        self.fail(sense_key::MEDIUM_ERROR, asc::NO_ADDITIONAL),
                    );
                }
            }
        }
        (out, self.ok())
    }

    /// Fixed-format REQUEST SENSE data (up to the allocation length in cdb[4]).
    fn request_sense_data(&self, cdb: &[u8]) -> Vec<u8> {
        let mut data = vec![0u8; 18];
        data[0] = 0x70; // current error, fixed format
        data[2] = self.sense.key;
        data[7] = 10; // additional sense length
        data[12] = self.sense.asc;
        data[13] = self.sense.ascq;
        let allocation = *cdb.get(4).unwrap_or(&18) as usize;
        if allocation != 0 && allocation < data.len() {
            data.truncate(allocation);
        }
        data
    }

    /// Standard INQUIRY data (36 bytes) for a direct-access device.
    fn inquiry_data(&self, cdb: &[u8]) -> Vec<u8> {
        let mut data = vec![0u8; 36];
        data[0] = 0x00; // peripheral qualifier 0, direct-access device
        data[1] = 0x00; // not removable
        data[2] = 0x02; // SCSI-2
        data[3] = 0x02; // response data format
        data[4] = 31; // additional length (36 - 5)
        data[8..16].copy_from_slice(b"NEETAN  ");
        data[16..32].copy_from_slice(b"SCSI HARDDISK   ");
        data[32..36].copy_from_slice(b"1.0 ");
        let allocation = *cdb.get(4).unwrap_or(&36) as usize;
        if allocation != 0 && allocation < data.len() {
            data.truncate(allocation);
        }
        data
    }

    /// READ CAPACITY(10) data: last LBA and block size, both big-endian.
    fn read_capacity_data(&self) -> Vec<u8> {
        let last_lba = self.block_count().saturating_sub(1);
        let mut data = vec![0u8; 8];
        data[0..4].copy_from_slice(&last_lba.to_be_bytes());
        data[4..8].copy_from_slice(&(SCSI_BLOCK_SIZE as u32).to_be_bytes());
        data
    }

    /// MODE SENSE(6) data: a 4-byte header plus an 8-byte block descriptor
    /// reporting the block count and 512-byte block length.
    fn mode_sense_data(&self, cdb: &[u8]) -> Vec<u8> {
        let blocks = self.block_count();
        let mut data = vec![0u8; 12];
        data[0] = 11; // mode data length (following bytes)
        data[1] = 0x00; // medium type
        data[2] = 0x00; // device-specific parameter
        data[3] = 8; // block descriptor length
        data[4..8].copy_from_slice(&blocks.to_be_bytes());
        let block_size = SCSI_BLOCK_SIZE as u32;
        data[9] = (block_size >> 16) as u8;
        data[10] = (block_size >> 8) as u8;
        data[11] = block_size as u8;
        let allocation = *cdb.get(4).unwrap_or(&12) as usize;
        if allocation != 0 && allocation < data.len() {
            data.truncate(allocation);
        }
        data
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk::HddImage;

    fn disk_with_blocks(blocks: usize) -> ScsiDisk {
        let data = vec![0u8; blocks * SCSI_BLOCK_SIZE];
        let image = HddImage::from_raw_flat(data).unwrap();
        ScsiDisk::new(MountedHdd::new(image, None))
    }

    #[test]
    fn inquiry_reports_direct_access_device() {
        let mut disk = disk_with_blocks(2048);
        let (data, st) = disk.data_in(&[opcode::INQUIRY, 0, 0, 0, 36, 0]);
        assert_eq!(st, status::GOOD);
        assert_eq!(data[0], 0x00);
        assert_eq!(&data[8..16], b"NEETAN  ");
    }

    #[test]
    fn read_capacity_reports_last_lba() {
        let mut disk = disk_with_blocks(2048);
        let (data, st) = disk.data_in(&[opcode::READ_CAPACITY, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(st, status::GOOD);
        assert_eq!(
            u32::from_be_bytes([data[0], data[1], data[2], data[3]]),
            2047
        );
        assert_eq!(
            u32::from_be_bytes([data[4], data[5], data[6], data[7]]),
            512
        );
    }

    #[test]
    fn write_then_read_round_trips() {
        let mut disk = disk_with_blocks(2048);
        let mut sector = vec![0u8; SCSI_BLOCK_SIZE];
        sector[0] = 0xAB;
        sector[511] = 0xCD;
        // WRITE(10) LBA 10, 1 block.
        let write = [opcode::WRITE10, 0, 0, 0, 0, 10, 0, 0, 1, 0];
        assert_eq!(disk.write_data_out(&write, &sector), status::GOOD);
        // READ(10) LBA 10, 1 block.
        let read = [opcode::READ10, 0, 0, 0, 0, 10, 0, 0, 1, 0];
        let (data, st) = disk.data_in(&read);
        assert_eq!(st, status::GOOD);
        assert_eq!(data, sector);
    }

    #[test]
    fn read_out_of_range_sets_sense() {
        let mut disk = disk_with_blocks(2048);
        let read = [opcode::READ10, 0, 0, 0, 0x10, 0, 0, 0, 1, 0]; // LBA 0x100000
        let (data, st) = disk.data_in(&read);
        assert_eq!(st, status::CHECK_CONDITION);
        assert!(data.is_empty());
        // REQUEST SENSE reports ILLEGAL REQUEST / LBA out of range.
        let (sense, _) = disk.data_in(&[opcode::REQUEST_SENSE, 0, 0, 0, 18, 0]);
        assert_eq!(sense[2], sense_key::ILLEGAL_REQUEST);
        assert_eq!(sense[12], asc::LBA_OUT_OF_RANGE);
    }

    #[test]
    fn nonzero_lun_check_conditions() {
        let mut disk = disk_with_blocks(2048);
        let cdb = [opcode::TEST_UNIT_READY, 0x20, 0, 0, 0, 0];
        assert_eq!(disk.execute_no_data(&cdb), status::CHECK_CONDITION);
    }

    #[test]
    fn test_unit_ready_is_good() {
        let mut disk = disk_with_blocks(2048);
        assert_eq!(
            disk.execute_no_data(&[opcode::TEST_UNIT_READY, 0, 0, 0, 0, 0]),
            status::GOOD
        );
    }
}
