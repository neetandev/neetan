//! IDE High-Level Emulation (HLE) - controller-specific functions.
//!
//! Shared HLE functions (read, write, init, format, etc.) live in
//! `crate::disk_hle`. This module contains IDE-specific functions:
//! sense (returns 0x0F for IDE) and motor control extensions (D0h/E0h/F0h).

use crate::{
    disk::MountedHdd,
    disk_hle::{STATUS_NOT_READY, mounted_drive},
};

/// Status code AH=40h (Equipment Check, CF=1) returned by the motor control functions for a
/// unit without a mounted drive. The code comes from the HDD BIOS error status table of the
/// PC-9800 Technical Data Book BIOS (section 8.6, "エラーステータス(HDD BIOS)").
const STATUS_EQUIPMENT_CHECK: u8 = 0x40;

/// Executes a BIOS sense operation: returns the media type.
/// For SASI-compatible images (256-byte sectors with standard SASI geometry),
/// returns the SASI media type code. Otherwise returns 0x0F (IDE).
/// A unit without a drive returns 60h (Not Ready).
pub(super) fn execute_sense(drive_index: usize, drives: &[Option<MountedHdd>]) -> u8 {
    let Some(drive) = mounted_drive(drives, drive_index) else {
        return STATUS_NOT_READY;
    };
    if let Some(sense_code) = drive.geometry().sasi_new_sense_type() {
        return sense_code;
    }
    0x0F
}

/// Executes an IDE-specific Check Power Mode (function D0h).
/// Returns 0x00 on success, 0x40 if drive absent.
pub(super) fn execute_check_power_mode(drive_index: usize, drives: &[Option<MountedHdd>]) -> u8 {
    motor_control_status(drive_index, drives)
}

/// Executes an IDE-specific Motor ON (function E0h).
/// Returns 0x00 on success, 0x40 if drive absent.
/// No-op in emulation since virtual drives are always ready.
pub(super) fn execute_motor_on(drive_index: usize, drives: &[Option<MountedHdd>]) -> u8 {
    motor_control_status(drive_index, drives)
}

/// Executes an IDE-specific Motor OFF (function F0h).
/// Returns 0x00 on success, 0x40 if drive absent.
/// No-op in emulation since virtual drives are always ready.
pub(super) fn execute_motor_off(drive_index: usize, drives: &[Option<MountedHdd>]) -> u8 {
    motor_control_status(drive_index, drives)
}

/// Returns the status of a motor control function: 00h for a present unit, 40h otherwise.
fn motor_control_status(drive_index: usize, drives: &[Option<MountedHdd>]) -> u8 {
    if mounted_drive(drives, drive_index).is_some() {
        0x00
    } else {
        STATUS_EQUIPMENT_CHECK
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk::{HddFormat, HddGeometry, HddImage};

    fn make_test_drive() -> MountedHdd {
        let geometry = HddGeometry {
            cylinders: 20,
            heads: 4,
            sectors_per_track: 17,
            sector_size: 512,
        };
        let data = vec![0u8; geometry.total_bytes() as usize];
        MountedHdd::new(HddImage::from_raw(geometry, HddFormat::Hdi, data), None)
    }

    fn make_sasi_compat_drive() -> MountedHdd {
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
    fn sense_returns_ide_type() {
        let drives: [Option<MountedHdd>; 2] = [Some(make_test_drive()), None];
        assert_eq!(execute_sense(0, &drives), 0x0F);
    }

    #[test]
    fn sense_returns_sasi_type_for_sasi_compat_image() {
        let drives: [Option<MountedHdd>; 2] = [Some(make_sasi_compat_drive()), None];
        assert_eq!(execute_sense(0, &drives), 0x00);
    }

    #[test]
    fn check_power_mode_with_drive() {
        let drives: [Option<MountedHdd>; 2] = [Some(make_test_drive()), None];
        assert_eq!(execute_check_power_mode(0, &drives), 0x00);
    }

    #[test]
    fn check_power_mode_without_drive() {
        let drives: [Option<MountedHdd>; 2] = [None, None];
        assert_eq!(execute_check_power_mode(0, &drives), 0x40);
    }

    #[test]
    fn motor_on_with_drive() {
        let drives: [Option<MountedHdd>; 2] = [Some(make_test_drive()), None];
        assert_eq!(execute_motor_on(0, &drives), 0x00);
    }

    #[test]
    fn motor_on_without_drive() {
        let drives: [Option<MountedHdd>; 2] = [None, None];
        assert_eq!(execute_motor_on(0, &drives), 0x40);
    }

    #[test]
    fn motor_off_with_drive() {
        let drives: [Option<MountedHdd>; 2] = [Some(make_test_drive()), None];
        assert_eq!(execute_motor_off(0, &drives), 0x00);
    }

    #[test]
    fn motor_off_without_drive() {
        let drives: [Option<MountedHdd>; 2] = [None, None];
        assert_eq!(execute_motor_off(0, &drives), 0x40);
    }

    #[test]
    fn absent_units_two_to_fifteen_report_not_ready_and_equipment_check() {
        let drives: [Option<MountedHdd>; 2] = [Some(make_test_drive()), Some(make_test_drive())];
        for unit in 2..16 {
            assert_eq!(execute_sense(unit, &drives), 0x60);
            assert_eq!(execute_check_power_mode(unit, &drives), 0x40);
            assert_eq!(execute_motor_on(unit, &drives), 0x40);
            assert_eq!(execute_motor_off(unit, &drives), 0x40);
        }
    }

    #[test]
    fn sense_of_empty_unit_one_reports_not_ready() {
        let drives: [Option<MountedHdd>; 2] = [Some(make_test_drive()), None];
        assert_eq!(execute_sense(1, &drives), 0x60);
    }
}
