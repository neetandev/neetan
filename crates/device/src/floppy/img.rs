//! Raw IMG floppy disk image format parser for IBM PC media.
//!
//! IMG is a headerless raw sector format; the geometry is selected by the
//! exact file size. Supported geometries cover the IBM PC formats of the
//! DOS/V era: 360 KB and 1.2 MB 5.25-inch, 720 KB and 1.44 MB 3.5-inch,
//! and the Japanese 3-mode 1.23 MB format.

use std::fmt;

use common::warn;

use super::d88::{D88Disk, D88MediaType, D88Sector};

/// Fixed geometry of a raw IMG image, selected by exact file size.
#[derive(Debug, Clone, Copy)]
pub struct ImgGeometry {
    /// Cylinder count.
    pub cylinders: u8,
    /// Head count.
    pub heads: u8,
    /// Sectors per track.
    pub sectors: u8,
    /// Bytes per sector.
    pub sector_size: usize,
    /// uPD765 N size code (sector size = 128 << N).
    pub size_code: u8,
    /// D88 media classification of the geometry.
    pub media_type: D88MediaType,
}

impl ImgGeometry {
    /// Total byte size of an image with this geometry.
    pub const fn file_size(&self) -> usize {
        self.cylinders as usize * self.heads as usize * self.sectors as usize * self.sector_size
    }
}

/// 5.25-inch 2D 360 KB: 40 cylinders x 2 heads x 9 sectors x 512 bytes.
const GEOMETRY_360K: ImgGeometry = ImgGeometry {
    cylinders: 40,
    heads: 2,
    sectors: 9,
    sector_size: 512,
    size_code: 2,
    media_type: D88MediaType::Disk2D,
};

/// 3.5-inch 2DD 720 KB: 80 cylinders x 2 heads x 9 sectors x 512 bytes.
const GEOMETRY_720K: ImgGeometry = ImgGeometry {
    cylinders: 80,
    heads: 2,
    sectors: 9,
    sector_size: 512,
    size_code: 2,
    media_type: D88MediaType::Disk2DD,
};

/// 5.25-inch 2HD 1.2 MB: 80 cylinders x 2 heads x 15 sectors x 512 bytes.
const GEOMETRY_1200K: ImgGeometry = ImgGeometry {
    cylinders: 80,
    heads: 2,
    sectors: 15,
    sector_size: 512,
    size_code: 2,
    media_type: D88MediaType::Disk2HD,
};

/// 3.5-inch 2HD 3-mode 1.23 MB: 77 cylinders x 2 heads x 8 sectors x 1024 bytes.
const GEOMETRY_1232K: ImgGeometry = ImgGeometry {
    cylinders: 77,
    heads: 2,
    sectors: 8,
    sector_size: 1024,
    size_code: 3,
    media_type: D88MediaType::Disk2HD,
};

/// 3.5-inch 2HD 1.44 MB: 80 cylinders x 2 heads x 18 sectors x 512 bytes.
const GEOMETRY_1440K: ImgGeometry = ImgGeometry {
    cylinders: 80,
    heads: 2,
    sectors: 18,
    sector_size: 512,
    size_code: 2,
    media_type: D88MediaType::Disk2HD,
};

/// All recognized IMG geometries, matched against the exact file size.
const IMG_GEOMETRIES: [ImgGeometry; 5] = [
    GEOMETRY_360K,
    GEOMETRY_720K,
    GEOMETRY_1200K,
    GEOMETRY_1232K,
    GEOMETRY_1440K,
];

/// Error type for IMG parsing.
#[derive(Debug, Clone)]
pub enum ImgError {
    /// Image data does not match any recognized geometry.
    UnknownSize {
        /// Actual byte count of the image data.
        actual: usize,
    },
}

impl fmt::Display for ImgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImgError::UnknownSize { actual } => {
                write!(
                    f,
                    "IMG image size is {actual} bytes, which matches no known floppy geometry"
                )
            }
        }
    }
}

/// Returns the geometry for an exact image byte size, if recognized.
pub fn detect_geometry(len: usize) -> Option<&'static ImgGeometry> {
    IMG_GEOMETRIES
        .iter()
        .find(|geometry| geometry.file_size() == len)
}

/// Parses a raw IMG disk image from raw bytes, detecting the geometry
/// from the exact file size.
pub fn from_bytes(data: &[u8]) -> Result<D88Disk, ImgError> {
    let Some(geometry) = detect_geometry(data.len()) else {
        return Err(ImgError::UnknownSize { actual: data.len() });
    };

    let total_tracks = geometry.cylinders as usize * geometry.heads as usize;
    let mut track_sectors = Vec::with_capacity(total_tracks);
    let mut offset = 0;

    for cylinder in 0..geometry.cylinders {
        for head in 0..geometry.heads {
            let mut sectors = Vec::with_capacity(geometry.sectors as usize);

            for record in 1..=geometry.sectors {
                let sector_data = data[offset..offset + geometry.sector_size].to_vec();
                let data_offset = offset;
                offset += geometry.sector_size;

                sectors.push(D88Sector {
                    cylinder,
                    head,
                    record,
                    size_code: geometry.size_code,
                    sector_count: geometry.sectors as u16,
                    mfm_flag: 0x00,
                    deleted: 0x00,
                    status: 0x00,
                    reserved: [0u8; 5],
                    data: sector_data,
                    source_offset: Some(data_offset as u64),
                });
            }

            track_sectors.push(Some(sectors));
        }
    }

    Ok(D88Disk::from_tracks(
        String::new(),
        false,
        geometry.media_type,
        track_sectors,
    ))
}

/// Finds the table geometry that represents `disk` without loss.
fn geometry_for_disk(disk: &D88Disk) -> Option<&'static ImgGeometry> {
    IMG_GEOMETRIES
        .iter()
        .find(|geometry| matches_geometry(disk, geometry))
}

fn matches_geometry(disk: &D88Disk, geometry: &ImgGeometry) -> bool {
    let total_tracks = geometry.cylinders as usize * geometry.heads as usize;
    if disk.track_slot_count() > total_tracks
        && (total_tracks..disk.track_slot_count()).any(|track| disk.sector_count(track) != 0)
    {
        return false;
    }
    for cylinder in 0..geometry.cylinders {
        for head in 0..geometry.heads {
            let track = cylinder as usize * geometry.heads as usize + head as usize;
            if disk.sector_count(track) != geometry.sectors as usize {
                return false;
            }
            for record in 1..=geometry.sectors {
                let Some(sector) = disk.find_sector(cylinder, head, record, geometry.size_code)
                else {
                    return false;
                };
                if sector.data.len() != geometry.sector_size {
                    return false;
                }
            }
        }
    }
    true
}

/// Serializes a `D88Disk` back into the raw IMG layout of the geometry
/// that represents it. Falls back to the 1.44 MB layout with zero fill
/// for unrepresentable disks (only reachable after a guest FORMAT TRACK
/// produced an incompatible layout).
pub fn to_bytes(disk: &D88Disk) -> Vec<u8> {
    let geometry = match geometry_for_disk(disk) {
        Some(geometry) => geometry,
        None => {
            warn!(
                "IMG serializer: disk layout matches no known IMG geometry; \
                 emitting a 1.44 MB image with zero fill for missing sectors."
            );
            &GEOMETRY_1440K
        }
    };

    let mut out = vec![0u8; geometry.file_size()];
    for cylinder in 0..geometry.cylinders {
        for head in 0..geometry.heads {
            for record in 1..=geometry.sectors {
                let slot_index = (cylinder as usize * geometry.heads as usize + head as usize)
                    * geometry.sectors as usize
                    + (record - 1) as usize;
                let offset = slot_index * geometry.sector_size;

                if let Some(sector) = disk.find_sector(cylinder, head, record, geometry.size_code)
                    && sector.data.len() == geometry.sector_size
                {
                    out[offset..offset + geometry.sector_size].copy_from_slice(&sector.data);
                }
            }
        }
    }

    out
}

/// Returns whether `disk` can be represented without data loss as raw IMG.
pub(crate) fn is_representable(disk: &D88Disk) -> bool {
    geometry_for_disk(disk).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reject_unknown_size() {
        assert!(matches!(
            from_bytes(&[0; 100]),
            Err(ImgError::UnknownSize { actual: 100 })
        ));
        assert!(matches!(
            from_bytes(&vec![0; 1_474_560 + 512]),
            Err(ImgError::UnknownSize { .. })
        ));
    }

    #[test]
    fn detect_all_geometries() {
        assert_eq!(detect_geometry(368_640).unwrap().sectors, 9);
        assert_eq!(detect_geometry(737_280).unwrap().cylinders, 80);
        assert_eq!(detect_geometry(1_228_800).unwrap().sectors, 15);
        assert_eq!(detect_geometry(1_261_568).unwrap().sector_size, 1024);
        assert_eq!(detect_geometry(1_474_560).unwrap().sectors, 18);
        assert!(detect_geometry(1_884_160).is_none());
        assert!(detect_geometry(0).is_none());
    }

    #[test]
    fn parse_1440k_image() {
        let disk = from_bytes(&vec![0u8; 1_474_560]).unwrap();
        assert_eq!(disk.media_type, D88MediaType::Disk2HD);
        assert_eq!(disk.sector_count(0), 18);
        assert_eq!(disk.sector_count(159), 18);
        assert_eq!(disk.sector_count(160), 0);
    }

    #[test]
    fn parse_360k_image() {
        let disk = from_bytes(&vec![0u8; 368_640]).unwrap();
        assert_eq!(disk.media_type, D88MediaType::Disk2D);
        assert_eq!(disk.sector_count(0), 9);
        assert_eq!(disk.sector_count(79), 9);
        assert_eq!(disk.sector_count(80), 0);
    }

    #[test]
    fn roundtrip_unchanged() {
        for size in [368_640usize, 737_280, 1_228_800, 1_261_568, 1_474_560] {
            let mut data = vec![0u8; size];
            for (i, byte) in data.iter_mut().enumerate() {
                *byte = (i & 0xFF) as u8;
            }
            let disk = from_bytes(&data).unwrap();
            assert!(is_representable(&disk));
            assert_eq!(to_bytes(&disk), data, "round trip failed for size {size}");
        }
    }

    #[test]
    fn source_offsets_recorded() {
        let disk = from_bytes(&vec![0u8; 1_474_560]).unwrap();
        let sector = disk.find_sector(0, 0, 1, 2).unwrap();
        assert_eq!(sector.source_offset, Some(0));
        let sector = disk.find_sector(1, 0, 1, 2).unwrap();
        assert_eq!(sector.source_offset, Some(2 * 18 * 512));
        let sector = disk.find_sector(0, 1, 3, 2).unwrap();
        assert_eq!(sector.source_offset, Some((18 + 2) * 512));
    }
}
