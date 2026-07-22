//! RAW (.h0-.h4) headerless flat hard disk image parser.
//!
//! FM Towns / X68000 headerless flat 512-byte-sector image; the extension digit
//! is the SCSI drive index. The CHS geometry is synthesized only to fill the
//! `HddGeometry` container; the access path is purely LBA-based.

use crate::disk::{HddError, HddFormat, HddGeometry, HddImage};

/// Raw (.h0-.h4) images use a fixed 512-byte sector.
const RAW_SECTOR_SIZE: u16 = 512;

/// Synthesized head count for the raw-image geometry container.
const RAW_HEADS: u8 = 8;

/// Synthesized sectors-per-track for the raw-image geometry container.
const RAW_SECTORS_PER_TRACK: u8 = 32;

impl HddImage {
    /// Loads a headerless flat 512-byte-sector image (.h0-.h4). The SCSI path
    /// is purely LBA-based; the CHS geometry is synthesized only to fill the
    /// `HddGeometry` container. The image size must be a nonzero multiple of
    /// 128 KiB (one synthesized cylinder), which every whole-megabyte image
    /// satisfies.
    pub fn from_raw_flat(data: Vec<u8>) -> Result<Self, HddError> {
        let sectors_per_cylinder = RAW_HEADS as usize * RAW_SECTORS_PER_TRACK as usize;
        let cylinder_bytes = sectors_per_cylinder * RAW_SECTOR_SIZE as usize;
        if data.is_empty() || !data.len().is_multiple_of(cylinder_bytes) {
            return Err(HddError::InvalidGeometry {
                field: "raw image size (must be a nonzero multiple of 128 KiB)",
                value: data.len() as u32,
            });
        }
        let cylinders = data.len() / cylinder_bytes;
        if cylinders > u16::MAX as usize {
            return Err(HddError::InvalidGeometry {
                field: "raw image cylinders",
                value: cylinders as u32,
            });
        }
        let geometry = HddGeometry {
            cylinders: cylinders as u16,
            heads: RAW_HEADS,
            sectors_per_track: RAW_SECTORS_PER_TRACK,
            sector_size: RAW_SECTOR_SIZE,
        };
        Ok(Self::from_raw(geometry, HddFormat::Raw, data))
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::disk::load_hdd_image;

    #[test]
    fn parse_raw_h0_round_trips() {
        // 1 MiB image: 2048 sectors of 512 bytes = 8 cyls x 8 heads x 32 spt.
        let mut data = vec![0u8; 1024 * 1024];
        for lba in 0..(data.len() / 512) {
            data[lba * 512] = lba as u8;
        }
        let hdd = load_hdd_image(Path::new("disk.h0"), &data).unwrap();

        assert_eq!(hdd.format, HddFormat::Raw);
        assert_eq!(hdd.geometry.sector_size, 512);
        assert_eq!(hdd.geometry.heads, RAW_HEADS);
        assert_eq!(hdd.geometry.sectors_per_track, RAW_SECTORS_PER_TRACK);
        assert_eq!(hdd.geometry.cylinders, 8);
        assert_eq!(hdd.geometry.total_sectors(), 2048);
        assert_eq!(hdd.read_sector(5).unwrap()[0], 5);
        // Headerless: serialization is byte-identical to the source file.
        assert_eq!(hdd.to_bytes(), data);
    }

    #[test]
    fn raw_h1_extension_also_parses() {
        let data = vec![0u8; 128 * 1024];
        let hdd = load_hdd_image(Path::new("disk.h1"), &data).unwrap();
        assert_eq!(hdd.format, HddFormat::Raw);
        assert_eq!(hdd.geometry.cylinders, 1);
    }

    #[test]
    fn raw_rejects_unaligned_size() {
        // 300 KiB is a multiple of 512 but not of the 128 KiB cylinder size.
        let data = vec![0u8; 300 * 1024];
        assert!(matches!(
            HddImage::from_raw_flat(data),
            Err(HddError::InvalidGeometry { .. })
        ));
    }
}
