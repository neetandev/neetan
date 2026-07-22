//! AT flat (.hdd) headerless IDE hard disk image parser.
//!
//! Headerless flat AT/IDE image using the classic 16 head x 63 SPT x 512-byte
//! translation geometry, up to the 1023-cylinder CHS ceiling (about 504 MB).

use crate::disk::{HddError, HddFormat, HddGeometry, HddImage};

/// Heads of the classic AT IDE translation geometry.
const AT_FLAT_HEADS: u8 = 16;

/// Sectors per track of the classic AT IDE translation geometry.
const AT_FLAT_SECTORS_PER_TRACK: u8 = 63;

/// Sector size of an AT IDE hard disk.
const AT_FLAT_SECTOR_SIZE: u16 = 512;

/// Cylinder ceiling of CHS addressing (10 cylinder bits).
const AT_FLAT_MAX_CYLINDERS: u16 = 1023;

impl HddImage {
    /// Loads a headerless flat AT IDE image (.hdd) with the classic
    /// 16 head x 63 sector x 512 byte translation geometry. The image size
    /// must be a nonzero whole number of cylinders (516,096 bytes each) up
    /// to the 1023-cylinder CHS ceiling (about 504 MB).
    pub fn from_at_flat(data: Vec<u8>) -> Result<Self, HddError> {
        let cylinder_bytes = AT_FLAT_HEADS as usize
            * AT_FLAT_SECTORS_PER_TRACK as usize
            * AT_FLAT_SECTOR_SIZE as usize;
        if data.is_empty() || !data.len().is_multiple_of(cylinder_bytes) {
            return Err(HddError::InvalidGeometry {
                field: "AT flat image size (must be a nonzero multiple of 504 KiB)",
                value: data.len() as u32,
            });
        }
        let cylinders = data.len() / cylinder_bytes;
        if cylinders > AT_FLAT_MAX_CYLINDERS as usize {
            return Err(HddError::InvalidGeometry {
                field: "AT flat image cylinders (the CHS ceiling is 1023)",
                value: cylinders as u32,
            });
        }
        let geometry = HddGeometry {
            cylinders: cylinders as u16,
            heads: AT_FLAT_HEADS,
            sectors_per_track: AT_FLAT_SECTORS_PER_TRACK,
            sector_size: AT_FLAT_SECTOR_SIZE,
        };
        Ok(Self::from_raw(geometry, HddFormat::AtFlat, data))
    }
}
