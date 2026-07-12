//! IBM XDF (Extended Density Format) floppy disk image parser.
//!
//! XDF stores 1.84 MB on standard 3.5-inch 2HD media and is the
//! distribution format of PC DOS 7.0 and OS/2 Warp. Cylinder 0 carries 19
//! standard 512-byte sectors per side in a remapped logical order so the
//! disk boots normally. Cylinders 1-79 carry one 8 KiB, one 2 KiB, one
//! 1 KiB and one 512-byte sector per side.

use std::fmt;

use common::warn;

use super::d88::{D88Disk, D88MediaType, D88Sector};

/// Exact byte size of an IBM XDF raw image.
pub(crate) const IBM_XDF_FILE_SIZE: usize = 1_884_160;
/// Cylinder count.
const CYLINDERS: u8 = 80;
/// Head count.
const HEADS: u8 = 2;
/// Byte size of one per-cylinder blob in the flat image (both heads).
const CYLINDER_BLOB_BYTES: usize = 23_552;
/// Sector size on cylinder 0.
const CYL0_SECTOR_SIZE: usize = 512;
/// uPD765 N size code for the 512-byte cylinder-0 sectors.
const CYL0_SIZE_CODE: u8 = 2;
/// Sectors per side on cylinder 0.
const CYL0_SECTORS_PER_TRACK: usize = 19;
/// Sectors per side on cylinders 1-79.
const TRACK_SECTORS_PER_TRACK: usize = 4;

/// Physical sector ID sequence of cylinder 0, head 0.
const CYL0_IDS_HEAD0: [u8; CYL0_SECTORS_PER_TRACK] = [
    1, 138, 129, 139, 130, 2, 131, 3, 132, 4, 133, 5, 134, 6, 135, 7, 136, 8, 137,
];
/// Physical sector ID sequence of cylinder 0, head 1.
const CYL0_IDS_HEAD1: [u8; CYL0_SECTORS_PER_TRACK] = [
    144, 135, 145, 136, 146, 137, 147, 138, 129, 139, 130, 140, 131, 141, 132, 142, 133, 143, 134,
];

/// Sector sizes on cylinders 1-79, per head, in physical ID-table order.
const TRACK_SECTOR_SIZES: [[usize; TRACK_SECTORS_PER_TRACK]; 2] =
    [[1024, 512, 2048, 8192], [2048, 512, 1024, 8192]];
/// Sector IDs on cylinders 1-79, per head, in the same order.
const TRACK_SECTOR_IDS: [[u8; TRACK_SECTORS_PER_TRACK]; 2] =
    [[131, 130, 132, 134], [132, 130, 131, 134]];
/// Data offsets into the per-cylinder blob on cylinders 1-79, per head.
const TRACK_DATA_OFFSETS: [[usize; TRACK_SECTORS_PER_TRACK]; 2] =
    [[0, 11_264, 1_024, 12_288], [20_480, 11_776, 22_528, 3_072]];

/// uPD765 N size code for a sector byte size (sizes are powers of two).
const fn size_code_for(size: usize) -> u8 {
    (size.trailing_zeros() - 7) as u8
}

/// Error type for IBM XDF parsing.
#[derive(Debug, Clone)]
pub enum IbmXdfError {
    /// Image data is not the expected size.
    InvalidSize {
        /// Actual byte count of the image data.
        actual: usize,
    },
}

impl fmt::Display for IbmXdfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IbmXdfError::InvalidSize { actual } => {
                write!(
                    f,
                    "IBM XDF image size is {actual} bytes, expected exactly {IBM_XDF_FILE_SIZE}"
                )
            }
        }
    }
}

/// Cylinder-0 logical 512-byte block index for a physical sector ID.
///
/// Blocks are numbered across the whole cylinder blob; blocks 20-22 and
/// 37-41 are padding that no physical sector maps to.
fn cyl0_logical_block(head: u8, id: u8) -> usize {
    if head == 0 {
        if id >= 129 {
            (id - 129) as usize
        } else {
            (id + 11) as usize
        }
    } else if id == 129 {
        11
    } else if id < 144 {
        (id - 130) as usize + 23
    } else {
        (id - 130) as usize + 28
    }
}

/// Byte offset of a sector's data in the flat image.
fn sector_source_offset(cylinder: u8, head: u8, physical_index: usize, id: u8) -> usize {
    let blob_base = cylinder as usize * CYLINDER_BLOB_BYTES;
    if cylinder == 0 {
        blob_base + cyl0_logical_block(head, id) * CYL0_SECTOR_SIZE
    } else {
        blob_base + TRACK_DATA_OFFSETS[head as usize][physical_index]
    }
}

/// Parses an IBM XDF disk image from raw bytes.
pub fn from_bytes(data: &[u8]) -> Result<D88Disk, IbmXdfError> {
    if data.len() != IBM_XDF_FILE_SIZE {
        return Err(IbmXdfError::InvalidSize { actual: data.len() });
    }

    let total_tracks = CYLINDERS as usize * HEADS as usize;
    let mut track_sectors = Vec::with_capacity(total_tracks);

    for cylinder in 0..CYLINDERS {
        for head in 0..HEADS {
            let mut sectors = Vec::new();

            if cylinder == 0 {
                let ids = if head == 0 {
                    &CYL0_IDS_HEAD0
                } else {
                    &CYL0_IDS_HEAD1
                };
                for (physical_index, &id) in ids.iter().enumerate() {
                    let offset = sector_source_offset(cylinder, head, physical_index, id);
                    sectors.push(D88Sector {
                        cylinder,
                        head,
                        record: id,
                        size_code: CYL0_SIZE_CODE,
                        sector_count: CYL0_SECTORS_PER_TRACK as u16,
                        mfm_flag: 0x00,
                        deleted: 0x00,
                        status: 0x00,
                        reserved: [0u8; 5],
                        data: data[offset..offset + CYL0_SECTOR_SIZE].to_vec(),
                        source_offset: Some(offset as u64),
                    });
                }
            } else {
                for physical_index in 0..TRACK_SECTORS_PER_TRACK {
                    let id = TRACK_SECTOR_IDS[head as usize][physical_index];
                    let size = TRACK_SECTOR_SIZES[head as usize][physical_index];
                    let offset = sector_source_offset(cylinder, head, physical_index, id);
                    sectors.push(D88Sector {
                        cylinder,
                        head,
                        record: id,
                        size_code: size_code_for(size),
                        sector_count: TRACK_SECTORS_PER_TRACK as u16,
                        mfm_flag: 0x00,
                        deleted: 0x00,
                        status: 0x00,
                        reserved: [0u8; 5],
                        data: data[offset..offset + size].to_vec(),
                        source_offset: Some(offset as u64),
                    });
                }
            }

            track_sectors.push(Some(sectors));
        }
    }

    Ok(D88Disk::from_tracks(
        String::new(),
        false,
        D88MediaType::Disk2HD,
        track_sectors,
    ))
}

/// Serializes a `D88Disk` back into the flat IBM XDF layout. Missing or
/// wrong-sized sectors are emitted as zeros with a warning; the cylinder-0
/// padding blocks are always zero.
pub fn to_bytes(disk: &D88Disk) -> Vec<u8> {
    let mut out = vec![0u8; IBM_XDF_FILE_SIZE];
    let mut warned = false;

    for_each_expected_sector(|cylinder, head, physical_index, id, size| {
        let offset = sector_source_offset(cylinder, head, physical_index, id);
        match disk.find_sector(cylinder, head, id, size_code_for(size)) {
            Some(sector) if sector.data.len() == size => {
                out[offset..offset + size].copy_from_slice(&sector.data);
            }
            _ => {
                if !warned {
                    warn!(
                        "IBM XDF serializer: missing or wrong-sized sector at \
                         C={cylinder} H={head} R={id}; emitting zeros. \
                         (XDF cannot represent non-standard geometry.)"
                    );
                    warned = true;
                }
            }
        }
    });

    out
}

/// Returns whether `disk` can be represented without data loss as IBM XDF.
pub(crate) fn is_representable(disk: &D88Disk) -> bool {
    let mut representable = true;
    for_each_expected_sector(|cylinder, head, _physical_index, id, size| {
        match disk.find_sector(cylinder, head, id, size_code_for(size)) {
            Some(sector) if sector.data.len() == size => {}
            _ => representable = false,
        }
    });
    representable
}

/// Invokes `visit(cylinder, head, physical_index, id, size)` for every
/// sector of the fixed XDF layout.
fn for_each_expected_sector(mut visit: impl FnMut(u8, u8, usize, u8, usize)) {
    for cylinder in 0..CYLINDERS {
        for head in 0..HEADS {
            if cylinder == 0 {
                let ids = if head == 0 {
                    &CYL0_IDS_HEAD0
                } else {
                    &CYL0_IDS_HEAD1
                };
                for (physical_index, &id) in ids.iter().enumerate() {
                    visit(cylinder, head, physical_index, id, CYL0_SECTOR_SIZE);
                }
            } else {
                for physical_index in 0..TRACK_SECTORS_PER_TRACK {
                    visit(
                        cylinder,
                        head,
                        physical_index,
                        TRACK_SECTOR_IDS[head as usize][physical_index],
                        TRACK_SECTOR_SIZES[head as usize][physical_index],
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reject_wrong_size() {
        assert!(matches!(
            from_bytes(&[0; 100]),
            Err(IbmXdfError::InvalidSize { actual: 100 })
        ));
        assert!(matches!(
            from_bytes(&vec![0; 1_261_568]),
            Err(IbmXdfError::InvalidSize { .. })
        ));
    }

    #[test]
    fn size_codes() {
        assert_eq!(size_code_for(512), 2);
        assert_eq!(size_code_for(1024), 3);
        assert_eq!(size_code_for(2048), 4);
        assert_eq!(size_code_for(8192), 6);
    }

    #[test]
    fn cylinder0_block_mapping() {
        // Boot sector: lbn 0 = head 0, ID 129.
        assert_eq!(cyl0_logical_block(0, 129), 0);
        // FAT: lbn 1-11 = head 0 IDs 130-139, then head 1 ID 129.
        assert_eq!(cyl0_logical_block(0, 130), 1);
        assert_eq!(cyl0_logical_block(0, 139), 10);
        assert_eq!(cyl0_logical_block(1, 129), 11);
        // Aux FS: lbn 12-19 = head 0 IDs 1-8.
        assert_eq!(cyl0_logical_block(0, 1), 12);
        assert_eq!(cyl0_logical_block(0, 8), 19);
        // Root directory: lbn 23-36 = head 1 IDs 130-143.
        assert_eq!(cyl0_logical_block(1, 130), 23);
        assert_eq!(cyl0_logical_block(1, 143), 36);
        // Data area start: lbn 42-45 = head 1 IDs 144-147.
        assert_eq!(cyl0_logical_block(1, 144), 42);
        assert_eq!(cyl0_logical_block(1, 147), 45);
    }

    #[test]
    fn cylinder0_track_structure() {
        let disk = from_bytes(&vec![0u8; IBM_XDF_FILE_SIZE]).unwrap();
        assert_eq!(disk.sector_count(0), 19);
        assert_eq!(disk.sector_count(1), 19);
        for head in 0..2u8 {
            for id in 129..=139u8 {
                assert!(
                    disk.find_sector(0, head, id, CYL0_SIZE_CODE).is_some()
                        || disk.find_sector(0, 1 - head, id, CYL0_SIZE_CODE).is_some(),
                    "cylinder 0 ID {id} not found on either head"
                );
            }
        }
    }

    #[test]
    fn mixed_size_track_structure() {
        let disk = from_bytes(&vec![0u8; IBM_XDF_FILE_SIZE]).unwrap();
        for track in [2usize, 3, 158, 159] {
            assert_eq!(disk.sector_count(track), 4, "track {track}");
        }
        let sector = disk.find_sector(1, 0, 134, 6).unwrap();
        assert_eq!(sector.data.len(), 8192);
        let sector = disk.find_sector(1, 1, 132, 4).unwrap();
        assert_eq!(sector.data.len(), 2048);
        let sector = disk.find_sector(79, 0, 130, 2).unwrap();
        assert_eq!(sector.data.len(), 512);
        let sector = disk.find_sector(79, 1, 131, 3).unwrap();
        assert_eq!(sector.data.len(), 1024);
    }

    fn build_pattern_image() -> Vec<u8> {
        let mut data = vec![0u8; IBM_XDF_FILE_SIZE];
        for (i, byte) in data.iter_mut().enumerate() {
            *byte = ((i / 512) & 0xFF) as u8 ^ (i & 0xFF) as u8;
        }
        data
    }

    #[test]
    fn boot_sector_is_block_zero() {
        let mut data = vec![0u8; IBM_XDF_FILE_SIZE];
        data[0] = 0xEB;
        data[510] = 0x55;
        data[511] = 0xAA;
        let disk = from_bytes(&data).unwrap();
        let boot = disk.find_sector(0, 0, 129, CYL0_SIZE_CODE).unwrap();
        assert_eq!(boot.data[0], 0xEB);
        assert_eq!(boot.data[510], 0x55);
        assert_eq!(boot.data[511], 0xAA);
        assert_eq!(boot.source_offset, Some(0));
    }

    #[test]
    fn source_offsets_match_layout() {
        let disk = from_bytes(&build_pattern_image()).unwrap();
        // Head 1 root directory start: block 23.
        let sector = disk.find_sector(0, 1, 130, CYL0_SIZE_CODE).unwrap();
        assert_eq!(sector.source_offset, Some(23 * 512));
        // Cylinder 5, head 0, 8 KiB sector at blob offset 12,288.
        let sector = disk.find_sector(5, 0, 134, 6).unwrap();
        assert_eq!(
            sector.source_offset,
            Some((5 * CYLINDER_BLOB_BYTES + 12_288) as u64)
        );
        // Cylinder 5, head 1, 8 KiB sector at blob offset 3,072.
        let sector = disk.find_sector(5, 1, 134, 6).unwrap();
        assert_eq!(
            sector.source_offset,
            Some((5 * CYLINDER_BLOB_BYTES + 3_072) as u64)
        );
    }

    #[test]
    fn roundtrip_unchanged() {
        let mut original = build_pattern_image();
        // Zero the cylinder-0 padding blocks (20-22 and 37-41), which no
        // physical sector maps to and the serializer never writes.
        for block in (20..=22).chain(37..=41) {
            original[block * 512..(block + 1) * 512].fill(0);
        }
        let disk = from_bytes(&original).unwrap();
        assert!(is_representable(&disk));
        assert_eq!(to_bytes(&disk), original);
    }
}
