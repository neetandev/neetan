//! Blank floppy disk images: the bytes of an empty disk of one type in one container.

use super::{
    FddType,
    d88::{D88Disk, D88Sector},
    fdi,
};

/// The container of a blank floppy disk image.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum FloppyContainer {
    /// The D88 container with a track table.
    D88,
    /// The Anex86 FDI geometry header followed by the sectors.
    Fdi,
    /// The headerless sectors.
    Raw,
}

/// Returns the bytes of a zero-filled floppy disk of `fdd_type` in `container`.
pub fn blank_floppy_image_bytes(fdd_type: FddType, container: FloppyContainer) -> Vec<u8> {
    let geometry = fdd_type.geometry();
    let sector_size = usize::from(geometry.sector_size);
    let sectors_per_track = usize::from(geometry.sectors_per_track);
    let track_count = usize::from(geometry.cylinders) * usize::from(geometry.heads);
    let data_size = track_count * sectors_per_track * sector_size;

    match container {
        FloppyContainer::Raw => vec![0u8; data_size],
        FloppyContainer::Fdi => {
            let mut bytes = fdi::header(fdi::media_code(fdd_type), &geometry);
            bytes.resize(bytes.len() + data_size, 0);
            bytes
        }
        FloppyContainer::D88 => {
            let heads = usize::from(geometry.heads);
            let track_sectors = (0..track_count)
                .map(|track_index| {
                    let cylinder = (track_index / heads) as u8;
                    let head = (track_index % heads) as u8;
                    let sectors = (1..=geometry.sectors_per_track)
                        .map(|record| D88Sector {
                            cylinder,
                            head,
                            record,
                            size_code: geometry.size_code(),
                            sector_count: u16::from(geometry.sectors_per_track),
                            mfm_flag: 0x00,
                            deleted: 0x00,
                            status: 0x00,
                            reserved: [0u8; 5],
                            data: vec![0u8; sector_size],
                            source_offset: None,
                        })
                        .collect();
                    Some(sectors)
                })
                .collect();
            D88Disk::from_tracks(String::new(), false, fdd_type.media_type(), track_sectors)
                .to_bytes()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::floppy::load_floppy_image;

    /// Every floppy type with the file names that parse its blank image.
    const CASES: [(FddType, &[(&str, FloppyContainer)]); 5] = [
        (
            FddType::Hd2,
            &[
                ("blank.d88", FloppyContainer::D88),
                ("blank.fdi", FloppyContainer::Fdi),
                ("blank.xdf", FloppyContainer::Raw),
            ],
        ),
        (
            FddType::Hd2Fmt144,
            &[
                ("blank.d88", FloppyContainer::D88),
                ("blank.fdi", FloppyContainer::Fdi),
                ("blank.img", FloppyContainer::Raw),
            ],
        ),
        (
            FddType::Dd2,
            &[
                ("blank.d88", FloppyContainer::D88),
                ("blank.fdi", FloppyContainer::Fdi),
            ],
        ),
        (
            FddType::Dd2Fmt720,
            &[
                ("blank.d88", FloppyContainer::D88),
                ("blank.fdi", FloppyContainer::Fdi),
                ("blank.dsk", FloppyContainer::Raw),
                ("blank.img", FloppyContainer::Raw),
            ],
        ),
        (
            FddType::D2,
            &[
                ("blank.d88", FloppyContainer::D88),
                ("blank.fdi", FloppyContainer::Fdi),
            ],
        ),
    ];

    /// Verifies that every blank image parses back with the geometry of its type.
    #[test]
    fn every_blank_image_parses_with_its_geometry() {
        for (fdd_type, files) in CASES {
            for &(name, container) in files {
                let bytes = blank_floppy_image_bytes(fdd_type, container);
                let image = load_floppy_image(Path::new(name), &bytes)
                    .unwrap_or_else(|error| panic!("{fdd_type:?} {name}: {error}"));
                assert_eq!(
                    image.physical_geometry(),
                    Some(fdd_type.geometry()),
                    "{fdd_type:?} {name}"
                );
            }
        }
    }
}
