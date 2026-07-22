//! HDI (Anex86) hard disk image format parser.
//!
//! PC-98 HDD image with a compact 32-byte geometry header (header size, total
//! sectors, sector size, sectors-per-track, heads, cylinders).

use crate::disk::{HddError, HddFormat, HddGeometry, HddImage, validate_geometry};

/// HDI header size (fixed at 32 bytes).
pub(crate) const HDI_HEADER_SIZE: usize = 32;

impl HddImage {
    /// Parses an HDI image from raw bytes.
    pub fn from_hdi(data: &[u8]) -> Result<Self, HddError> {
        if data.len() < HDI_HEADER_SIZE {
            return Err(HddError::TooSmall {
                format: "HDI",
                minimum: HDI_HEADER_SIZE,
                actual: data.len(),
            });
        }

        let header_size = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
        let sector_size = u32::from_le_bytes([data[16], data[17], data[18], data[19]]);
        let sectors_per_track = u32::from_le_bytes([data[20], data[21], data[22], data[23]]);
        let heads = u32::from_le_bytes([data[24], data[25], data[26], data[27]]);
        let cylinders = u32::from_le_bytes([data[28], data[29], data[30], data[31]]);

        validate_geometry(cylinders, heads, sectors_per_track, sector_size as u16)?;

        let geometry = HddGeometry {
            cylinders: cylinders as u16,
            heads: heads as u8,
            sectors_per_track: sectors_per_track as u8,
            sector_size: sector_size as u16,
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
            format: HddFormat::Hdi,
            data: data[data_start..data_start + expected_data_size].to_vec(),
            header_bytes: data[..data_start].to_vec(),
        })
    }
}

/// Synthesizes a default HDI header for the given geometry.
pub(crate) fn synth_header(geometry: HddGeometry) -> Vec<u8> {
    let mut header = vec![0u8; HDI_HEADER_SIZE];
    let total_sectors = geometry.total_sectors();
    header[8..12].copy_from_slice(&(HDI_HEADER_SIZE as u32).to_le_bytes());
    header[12..16].copy_from_slice(&total_sectors.to_le_bytes());
    header[16..20].copy_from_slice(&(geometry.sector_size as u32).to_le_bytes());
    header[20..24].copy_from_slice(&(geometry.sectors_per_track as u32).to_le_bytes());
    header[24..28].copy_from_slice(&(geometry.heads as u32).to_le_bytes());
    header[28..32].copy_from_slice(&(geometry.cylinders as u32).to_le_bytes());
    header
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk::test_support::build_hdi_image;

    #[test]
    fn parse_hdi_10mb() {
        let image = build_hdi_image(310, 4, 33, 256);
        let hdd = HddImage::from_hdi(&image).unwrap();

        assert_eq!(hdd.geometry.cylinders, 310);
        assert_eq!(hdd.geometry.heads, 4);
        assert_eq!(hdd.geometry.sectors_per_track, 33);
        assert_eq!(hdd.geometry.sector_size, 256);
        assert_eq!(hdd.format, HddFormat::Hdi);
    }

    #[test]
    fn hdi_roundtrip() {
        let image = build_hdi_image(310, 4, 33, 256);
        let hdd = HddImage::from_hdi(&image).unwrap();
        let serialized = hdd.to_bytes();

        assert_eq!(serialized.len(), image.len());
        assert_eq!(&serialized[HDI_HEADER_SIZE..], &image[HDI_HEADER_SIZE..]);
    }

    #[test]
    fn hdi_too_small_rejected() {
        let data = vec![0u8; 16];
        assert!(matches!(
            HddImage::from_hdi(&data),
            Err(HddError::TooSmall { format: "HDI", .. })
        ));
    }

    #[test]
    fn hdi_with_larger_header() {
        let mut image = build_hdi_image(153, 4, 33, 256);
        // Simulate a larger header by setting header_size and inserting padding.
        let new_header_size = 4096u32;
        image[8..12].copy_from_slice(&new_header_size.to_le_bytes());
        let padding = vec![0u8; (new_header_size as usize) - HDI_HEADER_SIZE];
        let data_portion = image[HDI_HEADER_SIZE..].to_vec();
        image.truncate(HDI_HEADER_SIZE);
        image.extend_from_slice(&padding);
        image.extend_from_slice(&data_portion);

        let hdd = HddImage::from_hdi(&image).unwrap();
        assert_eq!(hdd.geometry.cylinders, 153);
        assert_eq!(hdd.header_bytes.len(), 4096);

        // Roundtrip preserves the larger header byte-for-byte.
        let serialized = hdd.to_bytes();
        assert_eq!(serialized.len(), image.len());
        assert_eq!(&serialized[..4096], &image[..4096]);
    }
}
