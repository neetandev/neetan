//! X68000 SASI (.hdf) headerless hard disk image parser.
//!
//! The base X68000 uses a SASI hard-disk controller with 256-byte sectors. Its
//! `.hdf` image is headerless, so the geometry is recovered from the exact byte
//! length, which must match one of the three fixed SASI drive capacities
//! (10/20/40 MB). The SUPER/XVI internal-SCSI `.hdf` is a plain 512-byte raw
//! flat image and is loaded with [`HddImage::from_raw_flat`] instead.

use crate::disk::{HddError, HddFormat, HddGeometry, HddImage};

/// Sectors per track of an X68000 SASI hard disk.
const X68K_SASI_SECTORS_PER_TRACK: u8 = 33;

/// Sector size of an X68000 SASI hard disk.
const X68K_SASI_SECTOR_SIZE: u16 = 256;

/// Exact byte size of a 10 MB X68000 SASI .hdf image (309 cylinders, 4 heads).
pub const X68K_SASI_HDF_10MB_BYTES: usize = 10_441_728;

/// Exact byte size of a 20 MB X68000 SASI .hdf image (614 cylinders, 4 heads).
pub const X68K_SASI_HDF_20MB_BYTES: usize = 20_748_288;

/// Exact byte size of a 40 MB X68000 SASI .hdf image (614 cylinders, 8 heads).
pub const X68K_SASI_HDF_40MB_BYTES: usize = 41_496_576;

impl HddImage {
    /// Parses a 256-byte-sector X68000 SASI .hdf image. Its size must match one
    /// of the three fixed SASI drive capacities (10, 20, or 40 MB).
    pub fn from_x68k_sasi(data: Vec<u8>) -> Result<Self, HddError> {
        let (cylinders, heads) = match data.len() {
            X68K_SASI_HDF_10MB_BYTES => (309, 4),
            X68K_SASI_HDF_20MB_BYTES => (614, 4),
            X68K_SASI_HDF_40MB_BYTES => (614, 8),
            _ => {
                return Err(HddError::InvalidGeometry {
                    field: "SASI .hdf size (must be exactly a 10, 20, or 40 MB image)",
                    value: data.len() as u32,
                });
            }
        };
        let geometry = HddGeometry {
            cylinders,
            heads,
            sectors_per_track: X68K_SASI_SECTORS_PER_TRACK,
            sector_size: X68K_SASI_SECTOR_SIZE,
        };
        Ok(HddImage::from_raw(geometry, HddFormat::Raw, data))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk::{MountedHdd, test_support::tempfile_with};

    #[test]
    fn sasi_sizes_map_to_drive_geometries() {
        let cases = [
            (X68K_SASI_HDF_10MB_BYTES, 309u16, 4u8),
            (X68K_SASI_HDF_20MB_BYTES, 614, 4),
            (X68K_SASI_HDF_40MB_BYTES, 614, 8),
        ];
        for (bytes, cylinders, heads) in cases {
            let image = HddImage::from_x68k_sasi(vec![0u8; bytes]).unwrap();
            assert_eq!(image.geometry.cylinders, cylinders);
            assert_eq!(image.geometry.heads, heads);
            assert_eq!(image.geometry.sectors_per_track, 33);
            assert_eq!(image.geometry.sector_size, 256);
            assert_eq!(image.geometry.total_bytes(), bytes as u64);
            assert_eq!(image.format, HddFormat::Raw);
            assert!(image.header_bytes.is_empty());
        }
    }

    #[test]
    fn sasi_rejects_other_sizes() {
        assert!(HddImage::from_x68k_sasi(vec![0u8; X68K_SASI_HDF_10MB_BYTES - 256]).is_err());
        assert!(HddImage::from_x68k_sasi(vec![0u8; X68K_SASI_HDF_10MB_BYTES + 256]).is_err());
        assert!(HddImage::from_x68k_sasi(Vec::new()).is_err());
    }

    #[test]
    fn sasi_flushes_headerless_round_trip() {
        let mut data = vec![0u8; X68K_SASI_HDF_10MB_BYTES];
        data[0] = 0x60;
        let path = tempfile_with(&data, ".hdf");

        let image = HddImage::from_x68k_sasi(data).unwrap();
        let mut mounted = MountedHdd::new(image, Some(path.clone()));
        assert!(mounted.write_sector(1, &[0xA5u8; 256]));
        mounted.flush();

        let written = std::fs::read(&path).unwrap();
        assert_eq!(written.len(), X68K_SASI_HDF_10MB_BYTES);
        assert_eq!(written[0], 0x60);
        assert_eq!(written[256], 0xA5);

        drop(mounted);
        std::fs::remove_file(&path).ok();
    }
}
