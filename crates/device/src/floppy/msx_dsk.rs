//! Raw sector-based MSX DSK floppy images.

use std::fmt;

use common::warn;

use super::d88::{D88Disk, D88MediaType, D88Sector};

/// Bytes stored in one MSX DSK sector.
const SECTOR_SIZE: usize = 512;
/// D88 size code for a 512-byte sector.
const SIZE_CODE: u8 = 2;
/// Sectors stored on each supported MSX DSK track.
const SECTORS_PER_TRACK: u8 = 9;
/// Byte offset of the FAT media descriptor.
const FAT_MEDIA_DESCRIPTOR_OFFSET: usize = SECTOR_SIZE;
/// First byte of the BIOS parameter block.
const BPB_OFFSET: usize = 0x0B;

/// Supported raw MSX DSK geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsxDskGeometry {
    /// 80 cylinders, one head and nine sectors per track.
    Tracks80Sides1,
    /// 40 cylinders, two heads and nine sectors per track.
    Tracks40Sides2,
    /// 80 cylinders, two heads and nine sectors per track.
    Tracks80Sides2,
}

impl MsxDskGeometry {
    /// Cylinder count.
    pub const fn cylinders(self) -> u8 {
        match self {
            Self::Tracks80Sides1 | Self::Tracks80Sides2 => 80,
            Self::Tracks40Sides2 => 40,
        }
    }

    /// Head count.
    pub const fn heads(self) -> u8 {
        match self {
            Self::Tracks80Sides1 => 1,
            Self::Tracks40Sides2 | Self::Tracks80Sides2 => 2,
        }
    }

    /// Total byte size.
    pub const fn file_size(self) -> usize {
        self.cylinders() as usize * self.heads() as usize * SECTORS_PER_TRACK as usize * SECTOR_SIZE
    }
}

/// Error while parsing a raw MSX DSK image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MsxDskError {
    /// The byte size is not a supported raw MSX layout.
    UnsupportedSize {
        /// Actual byte count.
        actual: usize,
    },
    /// The explicit geometry does not match the byte size.
    GeometrySizeMismatch {
        /// Selected geometry.
        geometry: MsxDskGeometry,
        /// Expected byte count.
        expected: usize,
        /// Actual byte count.
        actual: usize,
    },
}

impl fmt::Display for MsxDskError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSize { actual } => write!(
                formatter,
                "MSX DSK image size is {actual} bytes, expected 368640 or 737280 bytes"
            ),
            Self::GeometrySizeMismatch {
                geometry,
                expected,
                actual,
            } => write!(
                formatter,
                "MSX DSK geometry {geometry:?} requires {expected} bytes, image has {actual} bytes"
            ),
        }
    }
}

impl std::error::Error for MsxDskError {}

/// Detects geometry from the boot sector, FAT descriptor and byte size.
pub fn detect_geometry(data: &[u8]) -> Result<MsxDskGeometry, MsxDskError> {
    match data.len() {
        368_640 | 737_280 => {}
        actual => return Err(MsxDskError::UnsupportedSize { actual }),
    }

    if let Some(geometry) = geometry_from_bpb(data) {
        return Ok(geometry);
    }
    if let Some(geometry) = geometry_from_media_descriptor(data) {
        return Ok(geometry);
    }
    Ok(if data.len() == 368_640 {
        MsxDskGeometry::Tracks80Sides1
    } else {
        MsxDskGeometry::Tracks80Sides2
    })
}

/// Parses a raw MSX DSK image using detected geometry.
pub fn from_bytes(data: &[u8]) -> Result<(D88Disk, MsxDskGeometry), MsxDskError> {
    let geometry = detect_geometry(data)?;
    from_bytes_with_geometry(data, geometry).map(|disk| (disk, geometry))
}

/// Parses a raw MSX DSK image with an explicit geometry.
pub fn from_bytes_with_geometry(
    data: &[u8],
    geometry: MsxDskGeometry,
) -> Result<D88Disk, MsxDskError> {
    let expected = geometry.file_size();
    if data.len() != expected {
        return Err(MsxDskError::GeometrySizeMismatch {
            geometry,
            expected,
            actual: data.len(),
        });
    }

    let mut tracks = vec![None; usize::from(geometry.cylinders()) * 2];
    let mut offset = 0;
    for cylinder in 0..geometry.cylinders() {
        for head in 0..geometry.heads() {
            let mut sectors = Vec::with_capacity(usize::from(SECTORS_PER_TRACK));
            for record in 1..=SECTORS_PER_TRACK {
                let end = offset + SECTOR_SIZE;
                sectors.push(D88Sector {
                    cylinder,
                    head,
                    record,
                    size_code: SIZE_CODE,
                    sector_count: u16::from(SECTORS_PER_TRACK),
                    mfm_flag: 0x00,
                    deleted: 0x00,
                    status: 0x00,
                    reserved: [0; 5],
                    data: data[offset..end].to_vec(),
                    source_offset: Some(offset as u64),
                });
                offset = end;
            }
            tracks[usize::from(cylinder) * 2 + usize::from(head)] = Some(sectors);
        }
    }

    Ok(D88Disk::from_tracks(
        String::new(),
        false,
        if geometry.cylinders() == 80 {
            D88MediaType::Disk2DD
        } else {
            D88MediaType::Disk2D
        },
        tracks,
    ))
}

/// Serializes a disk into its selected raw MSX DSK geometry.
pub fn to_bytes(disk: &D88Disk, geometry: MsxDskGeometry) -> Vec<u8> {
    let mut output = vec![0; geometry.file_size()];
    let mut offset = 0;
    let mut warned = false;
    for cylinder in 0..geometry.cylinders() {
        for head in 0..geometry.heads() {
            for record in 1..=SECTORS_PER_TRACK {
                if let Some(sector) = disk.find_sector(cylinder, head, record, SIZE_CODE)
                    && sector.data.len() == SECTOR_SIZE
                {
                    output[offset..offset + SECTOR_SIZE].copy_from_slice(&sector.data);
                } else if !warned {
                    warn!(
                        "MSX DSK serializer: missing or invalid sector at C={cylinder} H={head} R={record}; emitting zeros"
                    );
                    warned = true;
                }
                offset += SECTOR_SIZE;
            }
        }
    }
    output
}

/// Returns whether the disk fits the selected raw geometry without loss.
pub(crate) fn is_representable(disk: &D88Disk, geometry: MsxDskGeometry) -> bool {
    for track_index in 0..disk.track_slot_count() {
        let cylinder = track_index / 2;
        let head = track_index % 2;
        let expected =
            cylinder < usize::from(geometry.cylinders()) && head < usize::from(geometry.heads());
        if !expected && disk.sector_count(track_index) != 0 {
            return false;
        }
    }
    for cylinder in 0..geometry.cylinders() {
        for head in 0..geometry.heads() {
            for record in 1..=SECTORS_PER_TRACK {
                let Some(sector) = disk.find_sector(cylinder, head, record, SIZE_CODE) else {
                    return false;
                };
                if sector.data.len() != SECTOR_SIZE {
                    return false;
                }
            }
        }
    }
    true
}

/// Reads a supported geometry from an MSX-DOS BIOS parameter block.
fn geometry_from_bpb(data: &[u8]) -> Option<MsxDskGeometry> {
    if !matches!(data.first(), Some(0xE9 | 0xEB)) || data.len() < BPB_OFFSET + 17 {
        return None;
    }
    let bytes_per_sector = u16::from_le_bytes([data[0x0B], data[0x0C]]);
    let sectors_per_track = u16::from_le_bytes([data[0x18], data[0x19]]);
    let heads = u16::from_le_bytes([data[0x1A], data[0x1B]]);
    if bytes_per_sector != SECTOR_SIZE as u16 || sectors_per_track != 9 {
        return None;
    }
    geometry_for_size_and_heads(data.len(), heads as u8)
}

/// Reads the head count encoded by a FAT media descriptor.
fn geometry_from_media_descriptor(data: &[u8]) -> Option<MsxDskGeometry> {
    let descriptor = *data.get(FAT_MEDIA_DESCRIPTOR_OFFSET)?;
    let heads = match descriptor {
        0xF8 | 0xFC => 1,
        0xF9 | 0xFD => 2,
        _ => return None,
    };
    geometry_for_size_and_heads(data.len(), heads)
}

/// Resolves an image size and head count to a supported geometry.
fn geometry_for_size_and_heads(size: usize, heads: u8) -> Option<MsxDskGeometry> {
    match (size, heads) {
        (368_640, 1) => Some(MsxDskGeometry::Tracks80Sides1),
        (368_640, 2) => Some(MsxDskGeometry::Tracks40Sides2),
        (737_280, 2) => Some(MsxDskGeometry::Tracks80Sides2),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds deterministic bytes for one geometry.
    fn image(geometry: MsxDskGeometry) -> Vec<u8> {
        let mut data = vec![0; geometry.file_size()];
        for (index, byte) in data.iter_mut().enumerate() {
            *byte = index as u8;
        }
        data
    }

    #[test]
    /// Image sizes have stable fallback geometries.
    fn detects_supported_sizes_without_metadata() {
        assert_eq!(
            detect_geometry(&vec![0; 368_640]).unwrap(),
            MsxDskGeometry::Tracks80Sides1
        );
        assert_eq!(
            detect_geometry(&vec![0; 737_280]).unwrap(),
            MsxDskGeometry::Tracks80Sides2
        );
    }

    #[test]
    /// The BIOS parameter block resolves an ambiguous 360 KB image.
    fn boot_sector_selects_double_sided_360k() {
        let mut data = vec![0; 368_640];
        data[0] = 0xEB;
        data[0x0B..0x0D].copy_from_slice(&512u16.to_le_bytes());
        data[0x18..0x1A].copy_from_slice(&9u16.to_le_bytes());
        data[0x1A..0x1C].copy_from_slice(&2u16.to_le_bytes());
        assert_eq!(
            detect_geometry(&data).unwrap(),
            MsxDskGeometry::Tracks40Sides2
        );
    }

    #[test]
    /// The FAT descriptor resolves geometry without a boot signature.
    fn fat_descriptor_is_used_when_the_boot_sector_is_absent() {
        let mut data = vec![0; 368_640];
        data[FAT_MEDIA_DESCRIPTOR_OFFSET] = 0xFD;
        assert_eq!(
            detect_geometry(&data).unwrap(),
            MsxDskGeometry::Tracks40Sides2
        );
    }

    #[test]
    /// Every supported geometry preserves its bytes.
    fn all_geometries_round_trip() {
        for geometry in [
            MsxDskGeometry::Tracks80Sides1,
            MsxDskGeometry::Tracks40Sides2,
            MsxDskGeometry::Tracks80Sides2,
        ] {
            let data = image(geometry);
            let disk = from_bytes_with_geometry(&data, geometry).unwrap();
            assert!(is_representable(&disk, geometry));
            assert_eq!(to_bytes(&disk, geometry), data);
        }
    }

    #[test]
    /// Changed sector bytes survive raw serialization.
    fn changed_sector_round_trips_through_raw_serialization() {
        let geometry = MsxDskGeometry::Tracks80Sides2;
        let mut disk = from_bytes_with_geometry(&image(geometry), geometry).unwrap();
        disk.find_sector_on_track_index_mut(0, 0, 0, 1, SIZE_CODE)
            .expect("first sector exists")
            .data
            .fill(0xA5);

        let serialized = to_bytes(&disk, geometry);
        let reparsed = from_bytes_with_geometry(&serialized, geometry).unwrap();
        assert!(
            reparsed
                .find_sector(0, 0, 1, SIZE_CODE)
                .unwrap()
                .data
                .iter()
                .all(|byte| *byte == 0xA5)
        );
    }

    #[test]
    /// Single-sided cylinders occupy canonical side-zero slots.
    fn single_sided_tracks_use_even_canonical_slots() {
        let data = image(MsxDskGeometry::Tracks80Sides1);
        let disk = from_bytes_with_geometry(&data, MsxDskGeometry::Tracks80Sides1).unwrap();
        assert_eq!(disk.sector_count(0), 9);
        assert_eq!(disk.sector_count(1), 0);
        assert_eq!(disk.sector_count(2), 9);
        assert_eq!(disk.sector_count(158), 9);
    }

    #[test]
    /// A selected geometry must match the image size.
    fn explicit_geometry_rejects_the_wrong_size() {
        assert!(matches!(
            from_bytes_with_geometry(&vec![0; 368_640], MsxDskGeometry::Tracks80Sides2),
            Err(MsxDskError::GeometrySizeMismatch { .. })
        ));
    }
}
