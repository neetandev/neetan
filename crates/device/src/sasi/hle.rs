//! SASI High-Level Emulation (HLE) - controller-specific functions.
//!
//! Shared HLE functions (read, write, init, format, etc.) live in
//! `crate::disk_hle`. This module contains only the SASI-specific
//! sense implementation.

use crate::{
    disk::MountedHdd,
    disk_hle::{STATUS_NOT_READY, mounted_drive},
};

/// Executes a BIOS sense operation: returns the SASI media type.
/// A unit without a drive returns 60h (Not Ready).
pub(super) fn execute_sense(drive_index: usize, drives: &[Option<MountedHdd>]) -> u8 {
    let Some(drive) = mounted_drive(drives, drive_index) else {
        return STATUS_NOT_READY;
    };
    drive.geometry().sasi_media_type().unwrap_or(0x0F)
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
        let data = vec![0u8; geometry.total_bytes() as usize];
        MountedHdd::new(HddImage::from_raw(geometry, HddFormat::Thd, data), None)
    }

    #[test]
    fn sense_returns_media_type() {
        let drives: [Option<MountedHdd>; 2] = [Some(make_test_drive()), None];
        let media_type = execute_sense(0, &drives);
        assert_eq!(media_type, 0); // 5 MB SASI = type 0
    }

    #[test]
    fn sense_of_absent_unit_reports_not_ready() {
        let drives: [Option<MountedHdd>; 2] = [Some(make_test_drive()), None];
        assert_eq!(execute_sense(1, &drives), 0x60);
        assert_eq!(execute_sense(2, &drives), 0x60);
        assert_eq!(execute_sense(3, &drives), 0x60);
        assert_eq!(execute_sense(4, &drives), 0x60);
        assert_eq!(execute_sense(15, &drives), 0x60);
    }
}
