//! Synthesized power-on CMOS RAM image for the PC/AT.
//!
//! The CMOS is volatile (no disk persistence): it is built from the selected
//! machine configuration each time the machine starts. A BIOS-generated CMOS
//! image supplies the AMI defaults, while machine-specific fields are
//! synthesized below.

use device::{disk::HddGeometry, mc146818_rtc::CMOS_SIZE};

/// Register A power-on value: divider 010, periodic rate 1024 Hz.
const REG_A_DEFAULT: u8 = 0x26;
/// Register B power-on value: 24-hour, BCD.
const REG_B_DEFAULT: u8 = 0x02;
/// Register D power-on value: valid RAM and time (battery good).
const REG_D_DEFAULT: u8 = 0x80;

/// Diagnostic status byte (0x0E): no errors.
const DIAGNOSTIC_STATUS: usize = 0x0E;
/// Shutdown status byte (0x0F): normal (cold boot).
const SHUTDOWN_STATUS: usize = 0x0F;
/// Floppy drive type byte (0x10): drive A high nibble, drive B low nibble.
const FLOPPY_TYPE: usize = 0x10;
/// Hard disk type byte (0x12): drive 0 high nibble, drive 1 low nibble.
const HARD_DISK_TYPE: usize = 0x12;
/// Equipment byte (0x14).
const EQUIPMENT: usize = 0x14;
/// Extended hard disk type of drive 0 (0x19).
const HARD_DISK_0_EXTENDED_TYPE: usize = 0x19;
/// Extended hard disk type of drive 1 (0x1A).
const HARD_DISK_1_EXTENDED_TYPE: usize = 0x1A;
/// First byte of the drive 0 user-defined parameter block (0x1B-0x23).
const HARD_DISK_0_PARAMETERS: usize = 0x1B;
/// First byte of the drive 1 user-defined parameter block (0x24-0x2C).
const HARD_DISK_1_PARAMETERS: usize = 0x24;
/// Hard disk type nibble selecting the extended type byte.
const HARD_DISK_TYPE_EXTENDED: u8 = 0xF;
/// Extended hard disk type for user-defined geometry.
const HARD_DISK_EXTENDED_TYPE_USER: u8 = 47;
/// AMI miscellaneous flags byte (0x2D).
const AMI_MISC_FLAGS: usize = 0x2D;
/// AMI miscellaneous flag bit 5: boot sequence A: then C: when set,
/// C: then A: when clear (captured from the setup's option toggle).
const AMI_BOOT_SEQUENCE_FLOPPY_FIRST: u8 = 0x20;
/// Base memory in kibibytes, little-endian (0x15/0x16).
const BASE_MEMORY_LOW: usize = 0x15;
/// Extended memory in kibibytes, little-endian (0x17/0x18).
const EXTENDED_MEMORY_LOW: usize = 0x17;
/// Extended memory in kibibytes, little-endian (AMI mirror at 0x30/0x31).
const EXTENDED_MEMORY_MIRROR_LOW: usize = 0x30;
/// Century byte (0x32), BCD.
const CENTURY: usize = 0x32;

/// First byte of the AMI checksum coverage range.
const CHECKSUM_START: usize = 0x10;
/// Last byte (inclusive) of the AMI checksum coverage range.
const CHECKSUM_END: usize = 0x2D;
/// High byte of the stored checksum (0x2E).
const CHECKSUM_HIGH: usize = 0x2E;
/// Low byte of the stored checksum (0x2F).
const CHECKSUM_LOW: usize = 0x2F;
/// First byte of the AMI extended configuration checksum range.
const AMI_EXTENDED_CHECKSUM_START: usize = 0x34;
/// High byte of the AMI extended configuration checksum (0x3E).
const AMI_EXTENDED_CHECKSUM_HIGH: usize = 0x3E;
/// Low byte of the AMI extended configuration checksum (0x3F).
const AMI_EXTENDED_CHECKSUM_LOW: usize = 0x3F;
/// Last byte of the AMI extended configuration checksum range.
const AMI_EXTENDED_CHECKSUM_END: usize = 0x5C;
/// First register populated from AMI's saved BIOS-default configuration.
const AMI_BIOS_DEFAULTS_START: usize = 0x10;
/// AMI vendor configuration saved by "Auto Configuration with BIOS Defaults".
const AMI_BIOS_DEFAULTS: [u8; 0x5D - AMI_BIOS_DEFAULTS_START] = [
    0x40, 0xAB, 0x00, 0xBC, 0x03, 0x80, 0x02, 0x00, 0x3C, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x17, 0x02, 0x7F,
    0x00, 0x3C, 0x20, 0xE2, 0x00, 0x0F, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x5E,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x05, 0x24, 0x0B, 0x00, 0x30, 0x09, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x20, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x50, 0x00, 0x00, 0x40, 0xB2,
];

/// Floppy type nibble: no drive installed.
pub const FLOPPY_TYPE_NONE: u8 = 0x0;
/// Floppy type nibble for a 5.25-inch 360 KB drive.
pub const FLOPPY_TYPE_360K: u8 = 0x1;
/// Floppy type nibble for a 5.25-inch 1.2 MB drive.
pub const FLOPPY_TYPE_1200K: u8 = 0x2;
/// Floppy type nibble for a 3.5-inch 720 KB drive.
pub const FLOPPY_TYPE_720K: u8 = 0x3;
/// Floppy type nibble for a 3.5-inch 1.44 MB drive.
pub const FLOPPY_TYPE_1440K: u8 = 0x4;
/// Equipment byte: a diskette drive is present.
const EQUIP_DISKETTE_PRESENT: u8 = 0x01;
/// Equipment byte: a math coprocessor is present.
const EQUIP_FPU_PRESENT: u8 = 0x02;
/// Equipment byte bits 7:6: number of diskette drives minus one.
const EQUIP_DRIVE_COUNT_MASK: u8 = 0xC0;
/// Equipment byte bits 7:6 value for two diskette drives.
const EQUIP_TWO_DRIVES: u8 = 0x40;

/// Base (conventional) memory reported to the BIOS, in kibibytes.
const BASE_MEMORY_KIB: u16 = 640;
/// Extended memory ceiling the 16-bit CMOS field can express, in kibibytes.
const EXTENDED_MEMORY_MAX_KIB: u32 = 0xFFFF;

/// Builds the power-on CMOS image for a machine with `ram_bytes` of RAM.
pub fn initial_cmos(ram_bytes: usize) -> [u8; CMOS_SIZE] {
    let mut cmos = [0u8; CMOS_SIZE];

    cmos[0x0A] = REG_A_DEFAULT;
    cmos[0x0B] = REG_B_DEFAULT;
    cmos[0x0D] = REG_D_DEFAULT;

    cmos[DIAGNOSTIC_STATUS] = 0x00;
    cmos[SHUTDOWN_STATUS] = 0x00;

    cmos[AMI_BIOS_DEFAULTS_START..=AMI_EXTENDED_CHECKSUM_END].copy_from_slice(&AMI_BIOS_DEFAULTS);

    cmos[EQUIPMENT] = EQUIP_FPU_PRESENT;
    // One 1.44 MB drive A, no drive B.
    set_floppy_drive_types(&mut cmos, FLOPPY_TYPE_1440K, FLOPPY_TYPE_NONE);

    let base = BASE_MEMORY_KIB.to_le_bytes();
    cmos[BASE_MEMORY_LOW] = base[0];
    cmos[BASE_MEMORY_LOW + 1] = base[1];

    let extended_kib =
        ((ram_bytes.saturating_sub(0x10_0000) / 1024) as u32).min(EXTENDED_MEMORY_MAX_KIB) as u16;
    let extended = extended_kib.to_le_bytes();
    cmos[EXTENDED_MEMORY_LOW] = extended[0];
    cmos[EXTENDED_MEMORY_LOW + 1] = extended[1];
    cmos[EXTENDED_MEMORY_MIRROR_LOW] = extended[0];
    cmos[EXTENDED_MEMORY_MIRROR_LOW + 1] = extended[1];

    cmos[CENTURY] = 0x20;

    // Boot from floppy first (the AMI default block selects C: then A:).
    set_boot_sequence(&mut cmos, true);

    recompute_standard_checksum(&mut cmos);

    let extended_checksum: u16 = cmos[AMI_EXTENDED_CHECKSUM_START..=AMI_EXTENDED_CHECKSUM_END]
        .iter()
        .enumerate()
        .filter(|(offset, _)| {
            let register = AMI_EXTENDED_CHECKSUM_START + offset;
            register != AMI_EXTENDED_CHECKSUM_HIGH && register != AMI_EXTENDED_CHECKSUM_LOW
        })
        .map(|(_, &byte)| byte as u16)
        .fold(0u16, u16::wrapping_add);
    cmos[AMI_EXTENDED_CHECKSUM_HIGH] = (extended_checksum >> 8) as u8;
    cmos[AMI_EXTENDED_CHECKSUM_LOW] = extended_checksum as u8;

    cmos
}

/// Recomputes the standard CMOS checksum over registers 0x10-0x2D.
pub fn recompute_standard_checksum(cmos: &mut [u8; CMOS_SIZE]) {
    let checksum: u16 = cmos[CHECKSUM_START..=CHECKSUM_END]
        .iter()
        .map(|&byte| byte as u16)
        .fold(0u16, u16::wrapping_add);
    cmos[CHECKSUM_HIGH] = (checksum >> 8) as u8;
    cmos[CHECKSUM_LOW] = checksum as u8;
}

/// Selects the BIOS boot sequence: A: then C: when `floppy_first`, C:
/// then A: otherwise. Recomputes the standard checksum.
pub fn set_boot_sequence(cmos: &mut [u8; CMOS_SIZE], floppy_first: bool) {
    if floppy_first {
        cmos[AMI_MISC_FLAGS] |= AMI_BOOT_SEQUENCE_FLOPPY_FIRST;
    } else {
        cmos[AMI_MISC_FLAGS] &= !AMI_BOOT_SEQUENCE_FLOPPY_FIRST;
    }
    recompute_standard_checksum(cmos);
}

/// Sets a hard disk to user type 47 with the given geometry: the type
/// nibble in 0x12, the extended type byte, and the user-defined parameter
/// block (cylinders, heads, write precompensation, control byte, landing
/// zone, sectors per track), then recomputes the standard checksum.
pub fn set_hard_disk_user_type(cmos: &mut [u8; CMOS_SIZE], drive: usize, geometry: &HddGeometry) {
    let (nibble_shift, extended_type, parameters) = if drive == 0 {
        (4, HARD_DISK_0_EXTENDED_TYPE, HARD_DISK_0_PARAMETERS)
    } else {
        (0, HARD_DISK_1_EXTENDED_TYPE, HARD_DISK_1_PARAMETERS)
    };

    let nibble_mask = 0x0Fu8 << (4 - nibble_shift);
    cmos[HARD_DISK_TYPE] =
        (cmos[HARD_DISK_TYPE] & nibble_mask) | (HARD_DISK_TYPE_EXTENDED << nibble_shift);
    cmos[extended_type] = HARD_DISK_EXTENDED_TYPE_USER;

    let cylinders = geometry.cylinders.to_le_bytes();
    // No write precompensation on IDE drives.
    let write_precompensation = 0xFFFFu16.to_le_bytes();
    // Landing zone: past the last cylinder.
    let landing_zone = geometry.cylinders.to_le_bytes();
    let control_byte = hard_disk_control_byte(geometry);

    cmos[parameters] = cylinders[0];
    cmos[parameters + 1] = cylinders[1];
    cmos[parameters + 2] = geometry.heads;
    cmos[parameters + 3] = write_precompensation[0];
    cmos[parameters + 4] = write_precompensation[1];
    cmos[parameters + 5] = control_byte;
    cmos[parameters + 6] = landing_zone[0];
    cmos[parameters + 7] = landing_zone[1];
    cmos[parameters + 8] = geometry.sectors_per_track;

    recompute_standard_checksum(cmos);
}

/// Returns the drive control byte shared by the CMOS user parameter block
/// and the fixed disk parameter table: disabled retries plus the
/// more-than-8-heads bit.
pub fn hard_disk_control_byte(geometry: &HddGeometry) -> u8 {
    0xC0 | if geometry.heads > 8 { 0x08 } else { 0x00 }
}

/// Sets the floppy drive type nibbles (0x10) and the equipment-byte
/// diskette bits, then recomputes the standard checksum.
pub fn set_floppy_drive_types(cmos: &mut [u8; CMOS_SIZE], drive_a: u8, drive_b: u8) {
    cmos[FLOPPY_TYPE] = (drive_a << 4) | (drive_b & 0x0F);

    let mut equipment = cmos[EQUIPMENT] & !(EQUIP_DRIVE_COUNT_MASK | EQUIP_DISKETTE_PRESENT);
    if drive_a != FLOPPY_TYPE_NONE || drive_b != FLOPPY_TYPE_NONE {
        equipment |= EQUIP_DISKETTE_PRESENT;
    }
    if drive_a != FLOPPY_TYPE_NONE && drive_b != FLOPPY_TYPE_NONE {
        equipment |= EQUIP_TWO_DRIVES;
    }
    cmos[EQUIPMENT] = equipment;

    recompute_standard_checksum(cmos);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored_checksum(cmos: &[u8; CMOS_SIZE]) -> u16 {
        ((cmos[CHECKSUM_HIGH] as u16) << 8) | cmos[CHECKSUM_LOW] as u16
    }

    fn computed_checksum(cmos: &[u8; CMOS_SIZE]) -> u16 {
        cmos[CHECKSUM_START..=CHECKSUM_END]
            .iter()
            .map(|&byte| byte as u16)
            .fold(0u16, u16::wrapping_add)
    }

    fn computed_extended_checksum(cmos: &[u8; CMOS_SIZE]) -> u16 {
        cmos[AMI_EXTENDED_CHECKSUM_START..=AMI_EXTENDED_CHECKSUM_END]
            .iter()
            .enumerate()
            .filter(|(offset, _)| {
                let register = AMI_EXTENDED_CHECKSUM_START + offset;
                register != AMI_EXTENDED_CHECKSUM_HIGH && register != AMI_EXTENDED_CHECKSUM_LOW
            })
            .map(|(_, &byte)| byte as u16)
            .fold(0u16, u16::wrapping_add)
    }

    #[test]
    fn checksum_is_valid() {
        let cmos = initial_cmos(8 << 20);
        assert_eq!(stored_checksum(&cmos), computed_checksum(&cmos));
    }

    #[test]
    fn ami_extended_checksum_is_valid_and_nonzero() {
        let cmos = initial_cmos(8 << 20);
        let stored = ((cmos[AMI_EXTENDED_CHECKSUM_HIGH] as u16) << 8)
            | cmos[AMI_EXTENDED_CHECKSUM_LOW] as u16;
        assert_ne!(stored, 0);
        assert_eq!(stored, 0x025E);
        assert_eq!(stored, computed_extended_checksum(&cmos));
    }

    #[test]
    fn ami_bios_defaults_are_present() {
        let cmos = initial_cmos(8 << 20);
        assert_eq!(cmos[0x11], 0xAB);
        assert_eq!(cmos[0x13], 0xBC);
        // The defaults block value 0x17 plus the floppy-first boot bit.
        assert_eq!(cmos[0x2D], 0x37);
        assert_eq!(cmos[0x33], 0xE2);
        assert_eq!(cmos[0x5C], 0xB2);
    }

    #[test]
    fn boot_sequence_toggles_bit_five_and_keeps_the_checksum() {
        let mut cmos = initial_cmos(8 << 20);
        set_boot_sequence(&mut cmos, false);
        assert_eq!(cmos[0x2D], 0x17, "C: then A:");
        assert_eq!(stored_checksum(&cmos), computed_checksum(&cmos));

        set_boot_sequence(&mut cmos, true);
        assert_eq!(cmos[0x2D], 0x37, "A: then C:");
        assert_eq!(stored_checksum(&cmos), computed_checksum(&cmos));
    }

    #[test]
    fn base_memory_is_640k() {
        let cmos = initial_cmos(8 * 1024 * 1024);
        let base = u16::from_le_bytes([cmos[BASE_MEMORY_LOW], cmos[BASE_MEMORY_LOW + 1]]);
        assert_eq!(base, 640);
    }

    #[test]

    fn extended_memory_reflects_ram_size() {
        for mib in [2u32, 8, 64] {
            let cmos = initial_cmos((mib as usize) << 20);
            let extended =
                u16::from_le_bytes([cmos[EXTENDED_MEMORY_LOW], cmos[EXTENDED_MEMORY_LOW + 1]]);
            let mirror = u16::from_le_bytes([
                cmos[EXTENDED_MEMORY_MIRROR_LOW],
                cmos[EXTENDED_MEMORY_MIRROR_LOW + 1],
            ]);
            let expected = ((mib - 1) * 1024).min(EXTENDED_MEMORY_MAX_KIB) as u16;
            assert_eq!(extended, expected);
            assert_eq!(mirror, expected);
        }
    }
}
