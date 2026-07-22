//! THD (T98) hard disk image format parser.
//!
//! PC-98 HDD image with a minimal 256-byte header holding only the cylinder
//! count; heads/SPT/sector-size are fixed SASI geometry (8 heads, 33 SPT,
//! 256-byte sectors).

use crate::disk::{HddError, HddFormat, HddGeometry, HddImage};

/// THD header size (fixed at 256 bytes).
pub(crate) const THD_HEADER_SIZE: usize = 256;

/// THD fixed geometry: 33 sectors per track.
pub(crate) const THD_SECTORS_PER_TRACK: u8 = 33;

/// THD fixed geometry: 8 heads.
pub(crate) const THD_HEADS: u8 = 8;

/// THD fixed sector size: 256 bytes.
pub(crate) const THD_SECTOR_SIZE: u16 = 256;

impl HddImage {
    /// Parses a THD image from raw bytes.
    pub fn from_thd(data: &[u8]) -> Result<Self, HddError> {
        if data.len() < THD_HEADER_SIZE {
            return Err(HddError::TooSmall {
                format: "THD",
                minimum: THD_HEADER_SIZE,
                actual: data.len(),
            });
        }

        let cylinders = u16::from_le_bytes([data[0], data[1]]);
        if cylinders == 0 {
            return Err(HddError::InvalidGeometry {
                field: "cylinders",
                value: cylinders as u32,
            });
        }

        let geometry = HddGeometry {
            cylinders,
            heads: THD_HEADS,
            sectors_per_track: THD_SECTORS_PER_TRACK,
            sector_size: THD_SECTOR_SIZE,
        };

        let data_start = THD_HEADER_SIZE;
        let expected_data_size = geometry.total_bytes() as usize;
        if data.len() < data_start + expected_data_size {
            return Err(HddError::DataTruncated {
                expected: data_start + expected_data_size,
                actual: data.len(),
            });
        }

        Ok(HddImage {
            geometry,
            format: HddFormat::Thd,
            data: data[data_start..data_start + expected_data_size].to_vec(),
            header_bytes: data[..THD_HEADER_SIZE].to_vec(),
        })
    }
}

/// Synthesizes a default THD header for the given geometry.
pub(crate) fn synth_header(geometry: HddGeometry) -> Vec<u8> {
    let mut header = vec![0u8; THD_HEADER_SIZE];
    header[0..2].copy_from_slice(&geometry.cylinders.to_le_bytes());
    header
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk::test_support::build_thd_image;

    #[test]
    fn parse_thd_20mb() {
        let image = build_thd_image(310);
        let hdd = HddImage::from_thd(&image).unwrap();

        assert_eq!(hdd.geometry.cylinders, 310);
        assert_eq!(hdd.geometry.heads, THD_HEADS);
        assert_eq!(hdd.geometry.sectors_per_track, THD_SECTORS_PER_TRACK);
        assert_eq!(hdd.geometry.sector_size, THD_SECTOR_SIZE);
        assert_eq!(hdd.format, HddFormat::Thd);
    }

    #[test]
    fn thd_roundtrip() {
        let image = build_thd_image(153);
        let hdd = HddImage::from_thd(&image).unwrap();
        let serialized = hdd.to_bytes();

        assert_eq!(serialized.len(), image.len());
        assert_eq!(&serialized[..2], &image[..2]);
        assert_eq!(&serialized[THD_HEADER_SIZE..], &image[THD_HEADER_SIZE..]);
    }

    #[test]
    fn thd_too_small_rejected() {
        let data = vec![0u8; 100];
        assert!(matches!(
            HddImage::from_thd(&data),
            Err(HddError::TooSmall { format: "THD", .. })
        ));
    }

    #[test]
    fn thd_zero_cylinders_rejected() {
        let mut image = build_thd_image(153);
        image[0] = 0;
        image[1] = 0;
        assert!(matches!(
            HddImage::from_thd(&image),
            Err(HddError::InvalidGeometry {
                field: "cylinders",
                ..
            })
        ));
    }
}
