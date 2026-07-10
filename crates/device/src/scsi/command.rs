//! Machine-agnostic SCSI command definitions and CDB parsing.
//!
//! This layer is independent of any host bus adapter. It defines the opcode,
//! status, and sense constants plus small helpers for decoding a Command
//! Descriptor Block (CDB). A concrete target (see [`crate::scsi::disk`])
//! interprets these against its backing store.

/// SCSI command opcodes for a direct-access (disk) device.
pub mod opcode {
    /// TEST UNIT READY.
    pub const TEST_UNIT_READY: u8 = 0x00;
    /// REZERO UNIT.
    pub const REZERO_UNIT: u8 = 0x01;
    /// REQUEST SENSE.
    pub const REQUEST_SENSE: u8 = 0x03;
    /// FORMAT UNIT.
    pub const FORMAT_UNIT: u8 = 0x04;
    /// READ(6).
    pub const READ6: u8 = 0x08;
    /// WRITE(6).
    pub const WRITE6: u8 = 0x0A;
    /// SEEK(6).
    pub const SEEK6: u8 = 0x0B;
    /// INQUIRY.
    pub const INQUIRY: u8 = 0x12;
    /// MODE SELECT(6).
    pub const MODE_SELECT6: u8 = 0x15;
    /// MODE SENSE(6).
    pub const MODE_SENSE6: u8 = 0x1A;
    /// START STOP UNIT.
    pub const START_STOP: u8 = 0x1B;
    /// PREVENT ALLOW MEDIUM REMOVAL.
    pub const PREVENT_ALLOW: u8 = 0x1E;
    /// READ CAPACITY(10).
    pub const READ_CAPACITY: u8 = 0x25;
    /// READ(10).
    pub const READ10: u8 = 0x28;
    /// WRITE(10).
    pub const WRITE10: u8 = 0x2A;
    /// SEEK(10).
    pub const SEEK10: u8 = 0x2B;
    /// VERIFY(10).
    pub const VERIFY10: u8 = 0x2F;
    /// READ SUB-CHANNEL.
    pub const READ_SUB_CHANNEL: u8 = 0x42;
    /// READ TOC.
    pub const READ_TOC: u8 = 0x43;
    /// PLAY AUDIO(10).
    pub const PLAY_AUDIO10: u8 = 0x45;
    /// PLAY AUDIO MSF.
    pub const PLAY_AUDIO_MSF: u8 = 0x47;
    /// PAUSE/RESUME audio playback.
    pub const PAUSE_RESUME: u8 = 0x4B;
}

/// SCSI status byte values (byte returned during the STATUS phase).
pub mod status {
    /// Command completed without error.
    pub const GOOD: u8 = 0x00;
    /// Command failed; sense data is available via REQUEST SENSE.
    pub const CHECK_CONDITION: u8 = 0x02;
}

/// SCSI sense keys (REQUEST SENSE byte 2, low nibble).
pub mod sense_key {
    /// No sense information.
    pub const NO_SENSE: u8 = 0x00;
    /// Logical unit not ready.
    pub const NOT_READY: u8 = 0x02;
    /// Unrecoverable medium error.
    pub const MEDIUM_ERROR: u8 = 0x03;
    /// Illegal request (invalid command or parameter).
    pub const ILLEGAL_REQUEST: u8 = 0x05;
    /// Unit attention (medium change, reset).
    pub const UNIT_ATTENTION: u8 = 0x06;
}

/// SCSI Additional Sense Codes (REQUEST SENSE byte 12).
pub mod asc {
    /// No additional sense information.
    pub const NO_ADDITIONAL: u8 = 0x00;
    /// Logical block address out of range.
    pub const LBA_OUT_OF_RANGE: u8 = 0x21;
    /// Invalid command operation code.
    pub const INVALID_COMMAND: u8 = 0x20;
    /// Invalid field in CDB.
    pub const INVALID_FIELD_IN_CDB: u8 = 0x24;
    /// Logical unit not supported.
    pub const LOGICAL_UNIT_NOT_SUPPORTED: u8 = 0x25;
    /// Not-ready-to-ready transition (medium may have changed).
    pub const NOT_READY_TO_READY_TRANSITION: u8 = 0x28;
    /// Medium not present.
    pub const MEDIUM_NOT_PRESENT: u8 = 0x3A;
}

/// The direction of the data phase a command requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// No data phase (STATUS follows COMMAND directly).
    None,
    /// The target sends data to the host (DATA IN).
    In,
    /// The host sends data to the target (DATA OUT).
    Out,
}

/// SCSI sense data (fixed-format, the fields we report).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SenseData {
    /// Sense key.
    pub key: u8,
    /// Additional sense code.
    pub asc: u8,
    /// Additional sense code qualifier.
    pub ascq: u8,
}

impl SenseData {
    /// No-sense (all clear) sense data.
    pub const CLEAR: SenseData = SenseData {
        key: sense_key::NO_SENSE,
        asc: asc::NO_ADDITIONAL,
        ascq: 0,
    };

    /// Builds sense data with a zero qualifier.
    pub fn new(key: u8, asc: u8) -> Self {
        Self { key, asc, ascq: 0 }
    }
}

impl Default for SenseData {
    fn default() -> Self {
        SenseData::CLEAR
    }
}

/// Returns the CDB length in bytes for a command opcode, from its group code
/// (top 3 bits): group 0 = 6, groups 1/2 = 10, group 5 = 12.
pub fn cdb_length(opcode: u8) -> usize {
    match opcode >> 5 {
        0 => 6,
        1 | 2 => 10,
        5 => 12,
        _ => 6,
    }
}

/// Extracts the LUN from CDB byte 1 (bits 5-7).
pub fn cdb_lun(cdb: &[u8]) -> u8 {
    cdb.get(1).map_or(0, |b| b >> 5)
}

/// Decodes the (LBA, transfer length in blocks) of a READ/WRITE CDB. A 6-byte
/// command carries a 21-bit LBA and an 8-bit count where 0 means 256 blocks; a
/// 10-byte command carries a 32-bit LBA and a 16-bit count.
pub fn read_write_lba_length(cdb: &[u8]) -> Option<(u32, u32)> {
    match cdb.first()? >> 5 {
        0 => {
            let lba = (((cdb.get(1)? & 0x1F) as u32) << 16)
                | ((*cdb.get(2)? as u32) << 8)
                | *cdb.get(3)? as u32;
            let raw = *cdb.get(4)? as u32;
            let length = if raw == 0 { 256 } else { raw };
            Some((lba, length))
        }
        1 | 2 => {
            let lba = ((*cdb.get(2)? as u32) << 24)
                | ((*cdb.get(3)? as u32) << 16)
                | ((*cdb.get(4)? as u32) << 8)
                | *cdb.get(5)? as u32;
            let length = ((*cdb.get(7)? as u32) << 8) | *cdb.get(8)? as u32;
            Some((lba, length))
        }
        _ => None,
    }
}
