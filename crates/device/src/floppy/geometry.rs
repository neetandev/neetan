//! Physical floppy geometry and the logical order of the sectors of a track.
//!
//! A geometry describes a disk whose tracks hold the same number of sectors of
//! the same size. A loaded image derives its geometry from track 0 and its
//! track table. The logical sector lookup maps a position in a track to the ID
//! of the sector that the position holds.

use super::{
    FddType, FloppyFormat, FloppyImage,
    d88::{D88MediaType, D88Sector},
};

/// Byte count of a sector with size code 0.
pub(crate) const SMALLEST_SECTOR_SIZE: u16 = 128;

/// Byte count of a sector with the largest size code.
pub(crate) const LARGEST_SECTOR_SIZE: u16 = 16384;

/// Largest size code of a sector that a geometry describes.
pub(crate) const LARGEST_SIZE_CODE: u8 = 7;

/// The physical geometry of a floppy disk whose tracks hold the same sectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FloppyGeometry {
    /// Cylinder count.
    pub cylinders: u16,
    /// Head count.
    pub heads: u8,
    /// Sectors per track.
    pub sectors_per_track: u8,
    /// Bytes per sector, a power of two from 128 to 16384.
    pub sector_size: u16,
}

impl FloppyGeometry {
    /// Returns the uPD765 size code N of the sector size (sector size = 128 << N).
    pub const fn size_code(&self) -> u8 {
        (self.sector_size / SMALLEST_SECTOR_SIZE).trailing_zeros() as u8
    }
}

impl FddType {
    /// Returns the physical geometry of a blank disk of the type.
    pub const fn geometry(self) -> FloppyGeometry {
        match self {
            FddType::Hd2 => FloppyGeometry {
                cylinders: 77,
                heads: 2,
                sectors_per_track: 8,
                sector_size: 1024,
            },
            FddType::Hd2Fmt144 => FloppyGeometry {
                cylinders: 80,
                heads: 2,
                sectors_per_track: 18,
                sector_size: 512,
            },
            FddType::Dd2 => FloppyGeometry {
                cylinders: 80,
                heads: 2,
                sectors_per_track: 16,
                sector_size: 256,
            },
            FddType::Dd2Fmt720 => FloppyGeometry {
                cylinders: 80,
                heads: 2,
                sectors_per_track: 9,
                sector_size: 512,
            },
            FddType::D2 => FloppyGeometry {
                cylinders: 40,
                heads: 2,
                sectors_per_track: 16,
                sector_size: 256,
            },
        }
    }

    /// Returns the D88 media type of a blank disk of the type.
    pub const fn media_type(self) -> D88MediaType {
        match self {
            FddType::Hd2 | FddType::Hd2Fmt144 => D88MediaType::Disk2HD,
            FddType::Dd2 | FddType::Dd2Fmt720 => D88MediaType::Disk2DD,
            FddType::D2 => D88MediaType::Disk2D,
        }
    }
}

/// The media types that a nominal format applies to.
#[derive(Debug, Clone, Copy)]
enum MediaSelector {
    /// Every media type.
    Any,
    /// One media type.
    Only(D88MediaType),
}

impl MediaSelector {
    /// Returns whether the selector applies to `media_type`.
    fn matches(self, media_type: D88MediaType) -> bool {
        match self {
            MediaSelector::Any => true,
            MediaSelector::Only(selected) => selected == media_type,
        }
    }
}

/// A nominal floppy format: a track layout and the cylinder count of its disks.
#[derive(Debug, Clone, Copy)]
struct NominalFormat {
    /// Bytes per sector.
    sector_size: u16,
    /// Sectors per track.
    sectors_per_track: u8,
    /// The media types of the format.
    media: MediaSelector,
    /// Cylinder count of the format.
    cylinders: u16,
}

/// PC-98 2HD 1.25 MB: 8 sectors of 1024 bytes on 77 cylinders.
const NOMINAL_2HD_8X1024: NominalFormat = NominalFormat {
    sector_size: 1024,
    sectors_per_track: 8,
    media: MediaSelector::Any,
    cylinders: 77,
};

/// X68000 2HS and 2HDE: 9 sectors of 1024 bytes on 80 cylinders.
const NOMINAL_2HS_9X1024: NominalFormat = NominalFormat {
    sector_size: 1024,
    sectors_per_track: 9,
    media: MediaSelector::Any,
    cylinders: 80,
};

/// 2HC 1.2 MB: 15 sectors of 512 bytes on 80 cylinders.
const NOMINAL_2HC_15X512: NominalFormat = NominalFormat {
    sector_size: 512,
    sectors_per_track: 15,
    media: MediaSelector::Any,
    cylinders: 80,
};

/// 2HQ 1.44 MB: 18 sectors of 512 bytes on 80 cylinders.
const NOMINAL_2HQ_18X512: NominalFormat = NominalFormat {
    sector_size: 512,
    sectors_per_track: 18,
    media: MediaSelector::Any,
    cylinders: 80,
};

/// 2DD 640 KB: 8 sectors of 512 bytes on 80 cylinders.
const NOMINAL_2DD_8X512: NominalFormat = NominalFormat {
    sector_size: 512,
    sectors_per_track: 8,
    media: MediaSelector::Only(D88MediaType::Disk2DD),
    cylinders: 80,
};

/// 2DD 720 KB: 9 sectors of 512 bytes on 80 cylinders.
const NOMINAL_2DD_9X512: NominalFormat = NominalFormat {
    sector_size: 512,
    sectors_per_track: 9,
    media: MediaSelector::Only(D88MediaType::Disk2DD),
    cylinders: 80,
};

/// 2D 320 KB: 8 sectors of 512 bytes on 40 cylinders.
const NOMINAL_2D_8X512: NominalFormat = NominalFormat {
    sector_size: 512,
    sectors_per_track: 8,
    media: MediaSelector::Only(D88MediaType::Disk2D),
    cylinders: 40,
};

/// 2D 360 KB: 9 sectors of 512 bytes on 40 cylinders.
const NOMINAL_2D_9X512: NominalFormat = NominalFormat {
    sector_size: 512,
    sectors_per_track: 9,
    media: MediaSelector::Only(D88MediaType::Disk2D),
    cylinders: 40,
};

/// 2DD 640 KB: 16 sectors of 256 bytes on 80 cylinders.
const NOMINAL_2DD_16X256: NominalFormat = NominalFormat {
    sector_size: 256,
    sectors_per_track: 16,
    media: MediaSelector::Only(D88MediaType::Disk2DD),
    cylinders: 80,
};

/// 2D 320 KB: 16 sectors of 256 bytes on 40 cylinders.
const NOMINAL_2D_16X256: NominalFormat = NominalFormat {
    sector_size: 256,
    sectors_per_track: 16,
    media: MediaSelector::Only(D88MediaType::Disk2D),
    cylinders: 40,
};

/// Every nominal format of a container image.
const NOMINAL_FORMATS: [NominalFormat; 10] = [
    NOMINAL_2HD_8X1024,
    NOMINAL_2HS_9X1024,
    NOMINAL_2HC_15X512,
    NOMINAL_2HQ_18X512,
    NOMINAL_2DD_8X512,
    NOMINAL_2DD_9X512,
    NOMINAL_2D_8X512,
    NOMINAL_2D_9X512,
    NOMINAL_2DD_16X256,
    NOMINAL_2D_16X256,
];

/// Returns the cylinder count of the nominal format with `sector_size`,
/// `sectors_per_track` and `media_type`, or `None` outside the known formats.
fn nominal_cylinders(
    sector_size: u16,
    sectors_per_track: u8,
    media_type: D88MediaType,
) -> Option<u16> {
    NOMINAL_FORMATS
        .iter()
        .find(|format| {
            format.sector_size == sector_size
                && format.sectors_per_track == sectors_per_track
                && format.media.matches(media_type)
        })
        .map(|format| format.cylinders)
}

impl FloppyImage {
    /// Returns the physical geometry of the disk.
    ///
    /// The result is `None` for an IBM XDF disk, for a track 0 without
    /// sectors, for a track 0 with mixed size codes, for a size code above 7
    /// and for a sector whose data length does not match its size code.
    ///
    /// The sector size and the sectors per track come from track 0. A
    /// fixed-layout format takes the cylinders from its track table. A
    /// container format takes them from the nominal format of its track 0
    /// layout and media type, and from its track table outside the known
    /// formats. The disk has 2 heads when any odd track index holds sectors.
    pub fn physical_geometry(&self) -> Option<FloppyGeometry> {
        if self.format == FloppyFormat::IbmXdf {
            return None;
        }
        let sectors_per_track = u8::try_from(self.disk.sector_count(0))
            .ok()
            .filter(|&count| count > 0)?;
        let size_code = self.disk.sector_at_index(0, 0)?.size_code;
        if size_code > LARGEST_SIZE_CODE {
            return None;
        }
        let sector_size = SMALLEST_SECTOR_SIZE << size_code;
        let uniform = (0..usize::from(sectors_per_track)).all(|index| {
            self.disk.sector_at_index(0, index).is_some_and(|sector| {
                sector.size_code == size_code && sector.data.len() == usize::from(sector_size)
            })
        });
        if !uniform {
            return None;
        }

        let track_slot_count = self.disk.track_slot_count();
        let double_sided = (1..track_slot_count)
            .step_by(2)
            .any(|track_index| self.disk.sector_count(track_index) > 0);
        let heads = if double_sided { 2 } else { 1 };
        let table_cylinders = u16::try_from(track_slot_count.div_ceil(2)).ok()?;
        let cylinders = if self.format.has_fixed_layout() {
            table_cylinders
        } else {
            nominal_cylinders(sector_size, sectors_per_track, self.disk.media_type)
                .unwrap_or(table_cylinders)
        };

        Some(FloppyGeometry {
            cylinders,
            heads,
            sectors_per_track,
            sector_size,
        })
    }

    /// Returns the C/H/R/N ID of the sector at position `rank` (0-based) of the
    /// track at `cylinder` and `head` in logical order, or `None` when `rank`
    /// is at or above the sectors per track of `geometry`.
    ///
    /// A track that holds exactly the sectors per track of `geometry`, all with
    /// its size code, the cylinder ID `cylinder` and distinct records, gives the
    /// ID of its sector with the `rank`-th smallest record. Any other track
    /// gives the ID with the record `rank + 1`.
    pub fn logical_sector_id(
        &self,
        geometry: &FloppyGeometry,
        cylinder: u8,
        head: u8,
        rank: usize,
    ) -> Option<(u8, u8, u8, u8)> {
        if rank >= usize::from(geometry.sectors_per_track) {
            return None;
        }
        let size_code = geometry.size_code();
        let track_index = usize::from(cylinder) * 2 + usize::from(head);
        let sector_count = self.disk.sector_count(track_index);
        if sector_count == usize::from(geometry.sectors_per_track) {
            let mut sectors: Vec<&D88Sector> = (0..sector_count)
                .filter_map(|index| self.disk.sector_at_index(track_index, index))
                .collect();
            sectors.sort_by_key(|sector| sector.record);
            let distinct_records = sectors
                .windows(2)
                .all(|pair| pair[0].record != pair[1].record);
            let matching_ids = sectors
                .iter()
                .all(|sector| sector.size_code == size_code && sector.cylinder == cylinder);
            if distinct_records && matching_ids {
                let sector = sectors[rank];
                return Some((
                    sector.cylinder,
                    sector.head,
                    sector.record,
                    sector.size_code,
                ));
            }
        }
        let record = u8::try_from(rank + 1).ok()?;
        Some((cylinder, head, record, size_code))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::floppy::{D88Disk, MsxDskGeometry, dim, fdi, hdm, ibm_xdf};

    /// Verifies the geometry and the media type of every floppy type.
    #[test]
    fn fdd_type_geometry_and_media_type() {
        for (fdd_type, cylinders, sectors_per_track, sector_size, size_code, media_type) in [
            (FddType::Hd2, 77, 8, 1024, 3, D88MediaType::Disk2HD),
            (FddType::Hd2Fmt144, 80, 18, 512, 2, D88MediaType::Disk2HD),
            (FddType::Dd2, 80, 16, 256, 1, D88MediaType::Disk2DD),
            (FddType::Dd2Fmt720, 80, 9, 512, 2, D88MediaType::Disk2DD),
            (FddType::D2, 40, 16, 256, 1, D88MediaType::Disk2D),
        ] {
            let geometry = fdd_type.geometry();
            assert_eq!(
                geometry,
                FloppyGeometry {
                    cylinders,
                    heads: 2,
                    sectors_per_track,
                    sector_size,
                },
                "{fdd_type:?}"
            );
            assert_eq!(geometry.size_code(), size_code, "{fdd_type:?}");
            assert_eq!(fdd_type.media_type(), media_type, "{fdd_type:?}");
        }
    }

    /// Verifies the size code of every sector size from 128 to 16384 bytes.
    #[test]
    fn geometry_size_code_matches_the_sector_size() {
        for size_code in 0..=LARGEST_SIZE_CODE {
            let geometry = FloppyGeometry {
                cylinders: 1,
                heads: 2,
                sectors_per_track: 1,
                sector_size: SMALLEST_SECTOR_SIZE << size_code,
            };
            assert_eq!(geometry.size_code(), size_code);
        }
        let largest = FloppyGeometry {
            cylinders: 1,
            heads: 2,
            sectors_per_track: 1,
            sector_size: LARGEST_SECTOR_SIZE,
        };
        assert_eq!(largest.size_code(), LARGEST_SIZE_CODE);
    }

    /// Builds one sector with the ID C/H/R/N and zeroed data of its size.
    fn build_sector(cylinder: u8, head: u8, record: u8, size_code: u8) -> D88Sector {
        D88Sector {
            cylinder,
            head,
            record,
            size_code,
            sector_count: 0,
            mfm_flag: 0x00,
            deleted: 0x00,
            status: 0x00,
            reserved: [0u8; 5],
            data: vec![0u8; usize::from(SMALLEST_SECTOR_SIZE) << size_code],
            source_offset: None,
        }
    }

    /// Builds a track table of `cylinders` x `heads` tracks with records 1 to
    /// `sectors_per_track` of size code `size_code`.
    fn build_uniform_tracks(
        cylinders: u8,
        heads: u8,
        sectors_per_track: u8,
        size_code: u8,
    ) -> Vec<Option<Vec<D88Sector>>> {
        let mut tracks = vec![None; usize::from(cylinders) * 2];
        for cylinder in 0..cylinders {
            for head in 0..heads {
                tracks[usize::from(cylinder) * 2 + usize::from(head)] = Some(
                    (1..=sectors_per_track)
                        .map(|record| build_sector(cylinder, head, record, size_code))
                        .collect(),
                );
            }
        }
        tracks
    }

    /// Builds one track of sectors with the cylinder, head and size code IDs
    /// and the given records in their stored order.
    fn build_track(cylinder: u8, head: u8, records: &[u8], size_code: u8) -> Vec<D88Sector> {
        records
            .iter()
            .map(|&record| build_sector(cylinder, head, record, size_code))
            .collect()
    }

    /// Builds a D88 image from a track table.
    fn build_d88_image(
        media_type: D88MediaType,
        tracks: Vec<Option<Vec<D88Sector>>>,
    ) -> FloppyImage {
        FloppyImage::from_d88(D88Disk::from_tracks(
            String::new(),
            false,
            media_type,
            tracks,
        ))
    }

    /// Builds a DIM image of `media_type` without saved tracks.
    fn build_dim_image(media_type: u8) -> FloppyImage {
        let mut header = dim::blank_header();
        header[0] = media_type;
        FloppyImage::from_dim_bytes(&header).unwrap()
    }

    /// Returns the geometry with `cylinders`, `heads`, `sectors_per_track` and `sector_size`.
    const fn geometry(
        cylinders: u16,
        heads: u8,
        sectors_per_track: u8,
        sector_size: u16,
    ) -> FloppyGeometry {
        FloppyGeometry {
            cylinders,
            heads,
            sectors_per_track,
            sector_size,
        }
    }

    /// Verifies the physical geometry of the fixed-layout formats.
    #[test]
    fn physical_geometry_of_fixed_layout_formats() {
        let hdm = FloppyImage::from_hdm_bytes(&vec![0; hdm::HDM_FILE_SIZE]).unwrap();
        assert_eq!(hdm.physical_geometry(), Some(geometry(77, 2, 8, 1024)));

        let xdf = FloppyImage::from_xdf_bytes(&vec![0; hdm::HDM_FILE_SIZE]).unwrap();
        assert_eq!(xdf.physical_geometry(), Some(geometry(77, 2, 8, 1024)));

        let dim_2hd = build_dim_image(dim::MEDIA_2HD);
        assert_eq!(dim_2hd.physical_geometry(), Some(geometry(77, 2, 8, 1024)));

        let dim_2hq = build_dim_image(dim::MEDIA_2HQ);
        assert_eq!(dim_2hq.physical_geometry(), Some(geometry(80, 2, 18, 512)));

        let dim_2hs = build_dim_image(dim::MEDIA_2HS);
        assert_eq!(dim_2hs.physical_geometry(), Some(geometry(80, 2, 9, 1024)));

        let fdd_type = FddType::Hd2Fmt144;
        let mut fdi_bytes = fdi::header(fdi::media_code(fdd_type), &fdd_type.geometry());
        fdi_bytes.resize(fdi_bytes.len() + 80 * 2 * 18 * 512, 0);
        let fdi_1440 = FloppyImage::from_fdi_bytes(&fdi_bytes).unwrap();
        assert_eq!(fdi_1440.physical_geometry(), Some(geometry(80, 2, 18, 512)));

        let img_1200 = FloppyImage::from_img_bytes(&vec![0; 80 * 2 * 15 * 512]).unwrap();
        assert_eq!(img_1200.physical_geometry(), Some(geometry(80, 2, 15, 512)));

        let img_360 = FloppyImage::from_img_bytes(&vec![0; 40 * 2 * 9 * 512]).unwrap();
        assert_eq!(img_360.physical_geometry(), Some(geometry(40, 2, 9, 512)));
    }

    /// Verifies that a single-sided MSX disk keeps its 80 cylinders and 1 head.
    #[test]
    fn physical_geometry_of_single_sided_msx_disk() {
        let image = FloppyImage::from_msx_dsk_bytes(&vec![0; 360 * 1024]).unwrap();
        assert_eq!(
            image.format,
            FloppyFormat::MsxDsk(MsxDskGeometry::Tracks80Sides1)
        );
        assert_eq!(image.physical_geometry(), Some(geometry(80, 1, 9, 512)));
    }

    /// Verifies that a container format takes the cylinders of its nominal format.
    #[test]
    fn physical_geometry_of_container_formats() {
        let d88_2hd = build_d88_image(D88MediaType::Disk2HD, build_uniform_tracks(81, 2, 8, 3));
        assert_eq!(d88_2hd.physical_geometry(), Some(geometry(77, 2, 8, 1024)));

        let d88_1440 = build_d88_image(D88MediaType::Disk2HD, build_uniform_tracks(81, 2, 18, 2));
        assert_eq!(d88_1440.physical_geometry(), Some(geometry(80, 2, 18, 512)));

        let d88_2d = build_d88_image(D88MediaType::Disk2D, build_uniform_tracks(42, 2, 9, 2));
        assert_eq!(d88_2d.physical_geometry(), Some(geometry(40, 2, 9, 512)));

        let d77_disk = D88Disk::from_tracks(
            String::new(),
            false,
            D88MediaType::Disk2D,
            build_uniform_tracks(41, 2, 16, 1),
        );
        let d77_2d = FloppyImage::from_d77_bytes(&d77_disk.to_bytes()).unwrap();
        assert_eq!(d77_2d.format, FloppyFormat::D77);
        assert_eq!(d77_2d.physical_geometry(), Some(geometry(40, 2, 16, 256)));
    }

    /// Verifies that a container format outside the nominal formats takes the cylinders of its track table.
    #[test]
    fn physical_geometry_of_unknown_container_layout() {
        let image = build_d88_image(D88MediaType::Disk2HD, build_uniform_tracks(35, 1, 10, 1));
        assert_eq!(image.physical_geometry(), Some(geometry(35, 1, 10, 256)));
    }

    /// Verifies every case without a geometry: IBM XDF, mixed size codes, no
    /// sectors, a size code above 7 and a data length that does not match the size code.
    #[test]
    fn physical_geometry_is_none_for_mixed_layouts() {
        let ibm_xdf =
            FloppyImage::from_ibm_xdf_bytes(&vec![0; ibm_xdf::IBM_XDF_FILE_SIZE]).unwrap();
        assert_eq!(ibm_xdf.physical_geometry(), None);

        let mut tracks = build_uniform_tracks(77, 2, 8, 3);
        tracks[0] = Some(vec![build_sector(0, 0, 1, 3), build_sector(0, 0, 2, 2)]);
        let mixed = build_d88_image(D88MediaType::Disk2HD, tracks);
        assert_eq!(mixed.physical_geometry(), None);

        let empty = build_d88_image(D88MediaType::Disk2HD, vec![None, Some(Vec::new())]);
        assert_eq!(empty.physical_geometry(), None);

        let mut tracks = build_uniform_tracks(2, 2, 2, 3);
        tracks[0] = Some(build_track(0, 0, &[1, 2], LARGEST_SIZE_CODE + 1));
        let oversized = build_d88_image(D88MediaType::Disk2HD, tracks);
        assert_eq!(oversized.physical_geometry(), None);

        let mut tracks = build_uniform_tracks(77, 2, 8, 3);
        if let Some(track) = tracks[0].as_mut() {
            track[3].data.truncate(512);
        }
        let short_data = build_d88_image(D88MediaType::Disk2HD, tracks);
        assert_eq!(short_data.physical_geometry(), None);
    }

    /// Verifies that the logical IDs of a DIM 2HS disk number the records 10 to 18 except the first sector.
    #[test]
    fn logical_sector_id_of_dim_2hs() {
        let image = build_dim_image(dim::MEDIA_2HS);
        let geometry = image.physical_geometry().unwrap();
        assert_eq!(
            image.logical_sector_id(&geometry, 0, 0, 0),
            Some((0, 0, 1, 3))
        );
        assert_eq!(
            image.logical_sector_id(&geometry, 0, 0, 1),
            Some((0, 0, 11, 3))
        );
        assert_eq!(
            image.logical_sector_id(&geometry, 0, 1, 0),
            Some((0, 1, 10, 3))
        );
        assert_eq!(
            image.logical_sector_id(&geometry, 79, 1, 8),
            Some((79, 1, 18, 3))
        );
    }

    /// Verifies that the logical IDs of a DIM 2HDE disk set bit 7 of the head.
    #[test]
    fn logical_sector_id_of_dim_2hde() {
        let image = build_dim_image(dim::MEDIA_2HDE);
        let geometry = image.physical_geometry().unwrap();
        assert_eq!(
            image.logical_sector_id(&geometry, 0, 0, 0),
            Some((0, 0, 1, 3))
        );
        assert_eq!(
            image.logical_sector_id(&geometry, 0, 1, 0),
            Some((0, 0x81, 1, 3))
        );
    }

    /// Verifies that a D88 dump of a 2HS disk gives the record 10 for rank 0.
    #[test]
    fn logical_sector_id_of_d88_2hs_dump() {
        let records: Vec<u8> = (10..=18).collect();
        let tracks = (0..4u8)
            .map(|track_index| Some(build_track(track_index / 2, track_index % 2, &records, 3)))
            .collect();
        let image = build_d88_image(D88MediaType::Disk2HD, tracks);
        let geometry = image.physical_geometry().unwrap();
        assert_eq!(geometry.sectors_per_track, 9);
        assert_eq!(
            image.logical_sector_id(&geometry, 0, 0, 0),
            Some((0, 0, 10, 3))
        );
        assert_eq!(
            image.logical_sector_id(&geometry, 1, 1, 8),
            Some((1, 1, 18, 3))
        );
    }

    /// Verifies that an interleaved container track gives its records in ascending order.
    #[test]
    fn logical_sector_id_of_interleaved_track() {
        let tracks = vec![Some(build_track(
            0,
            0,
            &[10, 15, 11, 16, 12, 17, 13, 18, 14],
            3,
        ))];
        let image = build_d88_image(D88MediaType::Disk2HD, tracks);
        let geometry = geometry(80, 2, 9, 1024);
        for rank in 0..9u8 {
            assert_eq!(
                image.logical_sector_id(&geometry, 0, 0, usize::from(rank)),
                Some((0, 0, 10 + rank, 3))
            );
        }
    }

    /// Verifies that a track with a duplicate record gives the ID with the record `rank + 1`.
    #[test]
    fn logical_sector_id_of_track_with_duplicate_record() {
        let mut tracks = build_uniform_tracks(77, 2, 8, 3);
        tracks[2] = Some(build_track(1, 0, &[1, 1, 2, 3, 4, 5, 6, 7], 3));
        let image = build_d88_image(D88MediaType::Disk2HD, tracks);
        let geometry = FddType::Hd2.geometry();
        for rank in 0..8u8 {
            assert_eq!(
                image.logical_sector_id(&geometry, 1, 0, usize::from(rank)),
                Some((1, 0, rank + 1, 3))
            );
        }
        assert_eq!(image.logical_sector_id(&geometry, 1, 0, 8), None);
    }

    /// Verifies that a track that holds the sectors of another cylinder gives the ID with the record `rank + 1`.
    #[test]
    fn logical_sector_id_of_swapped_tracks() {
        let mut tracks = build_uniform_tracks(77, 2, 8, 3);
        tracks.swap(4, 6);
        let image = build_d88_image(D88MediaType::Disk2HD, tracks);
        let geometry = FddType::Hd2.geometry();
        for rank in 0..8u8 {
            assert_eq!(
                image.logical_sector_id(&geometry, 2, 0, usize::from(rank)),
                Some((2, 0, rank + 1, 3))
            );
        }
        assert!(image.find_sector_near_track_index(4, 2, 0, 1, 3).is_some());
    }
}
