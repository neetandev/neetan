//! NHD (T98-Next) hard disk image format parser.
//!
//! PC-98 HDD image with a 512-byte header carrying a "T98HDDIMAGE.R0" signature
//! and full CHS + sector-size geometry.

use crate::disk::{HddError, HddFormat, HddGeometry, HddImage, validate_geometry};

/// NHD file signature: "T98HDDIMAGE.R0\0" (15 bytes).
pub(crate) const NHD_SIGNATURE: &[u8; 15] = b"T98HDDIMAGE.R0\0";

/// NHD header size (fixed at 512 bytes).
pub(crate) const NHD_HEADER_SIZE: usize = 512;

impl HddImage {
    /// Parses an NHD image from raw bytes.
    pub fn from_nhd(data: &[u8]) -> Result<Self, HddError> {
        if data.len() < NHD_HEADER_SIZE {
            return Err(HddError::TooSmall {
                format: "NHD",
                minimum: NHD_HEADER_SIZE,
                actual: data.len(),
            });
        }
        if &data[..15] != NHD_SIGNATURE {
            return Err(HddError::InvalidSignature {
                format: "NHD",
                expected: "T98HDDIMAGE.R0",
            });
        }

        let header_size = u32::from_le_bytes([data[0x110], data[0x111], data[0x112], data[0x113]]);
        let cylinders = u32::from_le_bytes([data[0x114], data[0x115], data[0x116], data[0x117]]);
        let heads = u16::from_le_bytes([data[0x118], data[0x119]]);
        let sectors_per_track = u16::from_le_bytes([data[0x11A], data[0x11B]]);
        let sector_size = u16::from_le_bytes([data[0x11C], data[0x11D]]);

        validate_geometry(
            cylinders,
            heads as u32,
            sectors_per_track as u32,
            sector_size,
        )?;

        let geometry = HddGeometry {
            cylinders: cylinders as u16,
            heads: heads as u8,
            sectors_per_track: sectors_per_track as u8,
            sector_size,
        };

        let data_start = header_size as usize;
        let expected_data_size = geometry.total_bytes() as usize;
        if data.len() < data_start + expected_data_size {
            return Err(HddError::DataTruncated {
                expected: data_start + expected_data_size,
                actual: data.len(),
            });
        }

        Ok(HddImage {
            geometry,
            format: HddFormat::Nhd,
            data: data[data_start..data_start + expected_data_size].to_vec(),
            header_bytes: data[..data_start].to_vec(),
        })
    }
}

/// Synthesizes a default NHD header for the given geometry.
pub(crate) fn synth_header(geometry: HddGeometry) -> Vec<u8> {
    let mut header = vec![0u8; NHD_HEADER_SIZE];
    header[..15].copy_from_slice(NHD_SIGNATURE);
    header[0x110..0x114].copy_from_slice(&(NHD_HEADER_SIZE as u32).to_le_bytes());
    header[0x114..0x118].copy_from_slice(&(geometry.cylinders as u32).to_le_bytes());
    header[0x118..0x11A].copy_from_slice(&(geometry.heads as u16).to_le_bytes());
    header[0x11A..0x11C].copy_from_slice(&(geometry.sectors_per_track as u16).to_le_bytes());
    header[0x11C..0x11E].copy_from_slice(&geometry.sector_size.to_le_bytes());
    header
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk::test_support::build_nhd_image;

    #[test]
    fn parse_nhd_5mb() {
        let image = build_nhd_image(153, 4, 33, 256);
        let hdd = HddImage::from_nhd(&image).unwrap();

        assert_eq!(hdd.geometry.cylinders, 153);
        assert_eq!(hdd.geometry.heads, 4);
        assert_eq!(hdd.geometry.sectors_per_track, 33);
        assert_eq!(hdd.geometry.sector_size, 256);
        assert_eq!(hdd.geometry.total_sectors(), 153 * 4 * 33);
        assert_eq!(hdd.format, HddFormat::Nhd);
    }

    #[test]
    fn nhd_roundtrip() {
        let image = build_nhd_image(153, 4, 33, 256);
        let hdd = HddImage::from_nhd(&image).unwrap();
        let serialized = hdd.to_bytes();

        assert_eq!(serialized.len(), image.len());
        // Header should match.
        assert_eq!(&serialized[..15], NHD_SIGNATURE);
        // Data should match.
        let data_start = NHD_HEADER_SIZE;
        assert_eq!(&serialized[data_start..], &image[data_start..]);
    }

    #[test]
    fn nhd_too_small_rejected() {
        let data = vec![0u8; 100];
        assert!(matches!(
            HddImage::from_nhd(&data),
            Err(HddError::TooSmall { format: "NHD", .. })
        ));
    }

    #[test]
    fn nhd_bad_signature_rejected() {
        let mut image = build_nhd_image(153, 4, 33, 256);
        image[0] = b'X';
        assert!(matches!(
            HddImage::from_nhd(&image),
            Err(HddError::InvalidSignature { format: "NHD", .. })
        ));
    }

    #[test]
    fn nhd_truncated_data_rejected() {
        let mut image = build_nhd_image(153, 4, 33, 256);
        image.truncate(NHD_HEADER_SIZE + 100);
        assert!(matches!(
            HddImage::from_nhd(&image),
            Err(HddError::DataTruncated { .. })
        ));
    }

    #[test]
    fn nhd_with_512_byte_sectors() {
        let image = build_nhd_image(100, 4, 17, 512);
        let hdd = HddImage::from_nhd(&image).unwrap();

        assert_eq!(hdd.geometry.sector_size, 512);
        assert_eq!(hdd.geometry.total_sectors(), 100 * 4 * 17);

        let sector = hdd.read_sector(0).unwrap();
        assert_eq!(sector.len(), 512);
    }
}
