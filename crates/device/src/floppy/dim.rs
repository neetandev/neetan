//! DIM floppy disk image format parser.
//!
//! DIM is the DIFC.X container for X68000 2HD floppies. A 256-byte header
//! holds the media type, a per-track saved flag table, and metadata; only
//! flagged tracks are stored after the header, in ascending track order.
//! Absent tracks read as 0xE5 filler.

use std::fmt;

use common::warn;

use super::d88::{D88Disk, D88MediaType, D88Sector};

/// Size of the DIM container header.
pub const DIM_HEADER_SIZE: usize = 256;

/// Byte offset of the `DIFC HEADER` magic inside the header.
const MAGIC_OFFSET: usize = 0xAB;

/// The DIM header magic string.
const MAGIC: &[u8] = b"DIFC HEADER";

/// Number of track flag entries in the header.
const TRACK_FLAG_COUNT: usize = 170;

/// Fill byte used for tracks that are not stored in the container.
const ABSENT_FILL_BYTE: u8 = 0xE5;

/// Media type byte for 2HD (8 x 1024 bytes, 77 cylinders).
const MEDIA_2HD: u8 = 0x00;

/// Media type byte for 2HS (9 x 1024 bytes, 80 cylinders).
const MEDIA_2HS: u8 = 0x01;

/// Media type byte for 2HC (15 x 512 bytes, 80 cylinders).
const MEDIA_2HC: u8 = 0x02;

/// Media type byte for 2HDE (9 x 1024 bytes, 80 cylinders).
const MEDIA_2HDE: u8 = 0x03;

/// Media type byte for 2HQ (18 x 512 bytes, 80 cylinders).
const MEDIA_2HQ: u8 = 0x09;

/// Fixed physical geometry selected by the DIM media type byte.
#[derive(Debug, Clone, Copy)]
struct DimGeometry {
    cylinders: u8,
    sectors_per_track: u8,
    sector_size: usize,
    size_code: u8,
}

impl DimGeometry {
    /// Returns the geometry for a media type byte, if supported.
    fn from_media_type(media_type: u8) -> Option<Self> {
        match media_type {
            MEDIA_2HD => Some(Self {
                cylinders: 77,
                sectors_per_track: 8,
                sector_size: 1024,
                size_code: 3,
            }),
            MEDIA_2HS | MEDIA_2HDE => Some(Self {
                cylinders: 80,
                sectors_per_track: 9,
                sector_size: 1024,
                size_code: 3,
            }),
            MEDIA_2HC => Some(Self {
                cylinders: 80,
                sectors_per_track: 15,
                sector_size: 512,
                size_code: 2,
            }),
            MEDIA_2HQ => Some(Self {
                cylinders: 80,
                sectors_per_track: 18,
                sector_size: 512,
                size_code: 2,
            }),
            _ => None,
        }
    }

    /// Returns the number of tracks on the disk.
    fn tracks_per_disk(self) -> usize {
        usize::from(self.cylinders) * 2
    }

    /// Returns the stored byte length of one track.
    fn bytes_per_track(self) -> usize {
        usize::from(self.sectors_per_track) * self.sector_size
    }
}

/// Returns the ID head and record values for a sector slot.
///
/// 2HS numbers records 10..=18 except the first sector of the disk, and
/// 2HDE sets bit 7 of the ID head except on the first sector of the disk.
fn sector_id(media_type: u8, cylinder: u8, physical_head: u8, slot: u8) -> (u8, u8) {
    let is_first_disk_sector = cylinder == 0 && physical_head == 0 && slot == 0;
    match media_type {
        MEDIA_2HS => {
            let record = if is_first_disk_sector { 1 } else { slot + 10 };
            (physical_head, record)
        }
        MEDIA_2HDE => {
            let head = if is_first_disk_sector {
                0
            } else {
                0x80 | physical_head
            };
            (head, slot + 1)
        }
        _ => (physical_head, slot + 1),
    }
}

/// Error type for DIM parsing.
#[derive(Debug, Clone)]
pub enum DimError {
    /// Image is shorter than the 256-byte header.
    TooShort {
        /// Actual byte count of the image data.
        actual: usize,
    },
    /// The `DIFC HEADER` magic is missing.
    MissingMagic,
    /// The media type byte is not a supported format.
    UnsupportedMediaType(u8),
    /// A track flag holds a value other than 0x00 or 0x01.
    InvalidTrackFlag {
        /// Track index of the offending flag.
        track: usize,
        /// The flag value found.
        value: u8,
    },
    /// The over-track byte does not match the media track count.
    InvalidOverTrack(u8),
    /// Image size does not match the flagged track count.
    InvalidSize {
        /// Actual byte count of the image data.
        actual: usize,
        /// Expected byte count derived from the header.
        expected: usize,
    },
}

impl fmt::Display for DimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DimError::TooShort { actual } => {
                write!(
                    f,
                    "DIM image is {actual} bytes, shorter than the {DIM_HEADER_SIZE}-byte header"
                )
            }
            DimError::MissingMagic => write!(f, "DIM image does not contain the DIFC HEADER magic"),
            DimError::UnsupportedMediaType(media_type) => {
                write!(f, "DIM media type 0x{media_type:02X} is not supported")
            }
            DimError::InvalidTrackFlag { track, value } => {
                write!(
                    f,
                    "DIM track flag {track} holds invalid value 0x{value:02X}"
                )
            }
            DimError::InvalidOverTrack(value) => {
                write!(
                    f,
                    "DIM over-track byte 0x{value:02X} does not match the media"
                )
            }
            DimError::InvalidSize { actual, expected } => {
                write!(
                    f,
                    "DIM image size is {actual} bytes, expected exactly {expected}"
                )
            }
        }
    }
}

/// Builds a blank 2HD DIM header carrying only the magic and media type.
pub(crate) const fn blank_header() -> [u8; DIM_HEADER_SIZE] {
    let mut header = [0u8; DIM_HEADER_SIZE];
    header[0] = MEDIA_2HD;
    let mut index = 0;
    while index < MAGIC.len() {
        header[MAGIC_OFFSET + index] = MAGIC[index];
        index += 1;
    }
    header
}

/// Returns whether `data` carries the DIM header magic.
pub fn has_magic(data: &[u8]) -> bool {
    data.len() >= MAGIC_OFFSET + MAGIC.len()
        && &data[MAGIC_OFFSET..MAGIC_OFFSET + MAGIC.len()] == MAGIC
}

/// Parses a DIM disk image, returning the disk and a copy of the header.
pub fn from_bytes(data: &[u8]) -> Result<(D88Disk, Box<[u8; DIM_HEADER_SIZE]>), DimError> {
    if data.len() < DIM_HEADER_SIZE {
        return Err(DimError::TooShort { actual: data.len() });
    }
    if !has_magic(data) {
        return Err(DimError::MissingMagic);
    }

    let media_type = data[0];
    let Some(geometry) = DimGeometry::from_media_type(media_type) else {
        return Err(DimError::UnsupportedMediaType(media_type));
    };
    let tracks_per_disk = geometry.tracks_per_disk();

    for (track, &value) in data[1..1 + TRACK_FLAG_COUNT].iter().enumerate() {
        let valid = if track < tracks_per_disk {
            value == 0x00 || value == 0x01
        } else {
            value == 0x00
        };
        if !valid {
            return Err(DimError::InvalidTrackFlag { track, value });
        }
    }

    let over_track = data[0xFF];
    if over_track != 0 && usize::from(over_track) != tracks_per_disk {
        return Err(DimError::InvalidOverTrack(over_track));
    }

    let saved_tracks = data[1..1 + tracks_per_disk]
        .iter()
        .filter(|&&flag| flag == 0x01)
        .count();
    let expected = DIM_HEADER_SIZE + geometry.bytes_per_track() * saved_tracks;
    if data.len() != expected {
        return Err(DimError::InvalidSize {
            actual: data.len(),
            expected,
        });
    }

    let mut header = Box::new([0u8; DIM_HEADER_SIZE]);
    header.copy_from_slice(&data[..DIM_HEADER_SIZE]);

    let mut track_sectors = Vec::with_capacity(tracks_per_disk);
    let mut saved_index = 0usize;

    for track in 0..tracks_per_disk {
        let cylinder = (track / 2) as u8;
        let physical_head = (track % 2) as u8;
        let saved = data[1 + track] == 0x01;
        let track_offset = DIM_HEADER_SIZE + geometry.bytes_per_track() * saved_index;
        if saved {
            saved_index += 1;
        }

        let mut sectors = Vec::with_capacity(usize::from(geometry.sectors_per_track));
        for slot in 0..geometry.sectors_per_track {
            let (head, record) = sector_id(media_type, cylinder, physical_head, slot);
            let (sector_data, source_offset) = if saved {
                let offset = track_offset + usize::from(slot) * geometry.sector_size;
                (
                    data[offset..offset + geometry.sector_size].to_vec(),
                    Some(offset as u64),
                )
            } else {
                (vec![ABSENT_FILL_BYTE; geometry.sector_size], None)
            };

            sectors.push(D88Sector {
                cylinder,
                head,
                record,
                size_code: geometry.size_code,
                sector_count: u16::from(geometry.sectors_per_track),
                mfm_flag: 0x00,
                deleted: 0x00,
                status: 0x00,
                reserved: [0u8; 5],
                data: sector_data,
                source_offset,
            });
        }
        track_sectors.push(Some(sectors));
    }

    Ok((
        D88Disk::from_tracks(String::new(), false, D88MediaType::Disk2HD, track_sectors),
        header,
    ))
}

/// Returns whether a track holds only the absent-track filler byte.
fn track_is_blank(disk: &D88Disk, geometry: DimGeometry, media_type: u8, track: usize) -> bool {
    let cylinder = (track / 2) as u8;
    let physical_head = (track % 2) as u8;
    for slot in 0..geometry.sectors_per_track {
        let (head, record) = sector_id(media_type, cylinder, physical_head, slot);
        match disk.find_sector_on_track_index(track, cylinder, head, record, geometry.size_code) {
            Some(sector) if sector.data.iter().all(|&byte| byte == ABSENT_FILL_BYTE) => {}
            _ => return false,
        }
    }
    true
}

/// Serializes a `D88Disk` back into the DIM container layout.
///
/// Header metadata is preserved from `header`. The track flag table is
/// recomputed: a track is stored when its original flag was set or when its
/// current content is no longer blank filler, so formatting or writing a
/// previously absent track extends the saved set. Missing or wrong-sized
/// sectors are emitted as filler bytes; DIM cannot represent other layouts.
pub fn to_bytes(disk: &D88Disk, header: &[u8; DIM_HEADER_SIZE]) -> Vec<u8> {
    let media_type = header[0];
    let Some(geometry) = DimGeometry::from_media_type(media_type) else {
        warn!("DIM serializer: unsupported media type 0x{media_type:02X}; emitting header only");
        return header.to_vec();
    };
    let tracks_per_disk = geometry.tracks_per_disk();

    let mut stored = vec![false; tracks_per_disk];
    for (track, slot) in stored.iter_mut().enumerate() {
        let originally_saved = header[1 + track] == 0x01;
        *slot = originally_saved || !track_is_blank(disk, geometry, media_type, track);
    }

    let stored_count = stored.iter().filter(|&&saved| saved).count();
    let mut out = Vec::with_capacity(DIM_HEADER_SIZE + geometry.bytes_per_track() * stored_count);
    out.extend_from_slice(header);
    for (track, &saved) in stored.iter().enumerate() {
        out[1 + track] = u8::from(saved);
    }

    let mut warned = false;
    for (track, &saved) in stored.iter().enumerate() {
        if !saved {
            continue;
        }
        let cylinder = (track / 2) as u8;
        let physical_head = (track % 2) as u8;
        for slot in 0..geometry.sectors_per_track {
            let (head, record) = sector_id(media_type, cylinder, physical_head, slot);
            match disk.find_sector_on_track_index(track, cylinder, head, record, geometry.size_code)
            {
                Some(sector) if sector.data.len() == geometry.sector_size => {
                    out.extend_from_slice(&sector.data);
                }
                _ => {
                    if !warned {
                        warn!(
                            "DIM serializer: missing or wrong-sized sector at track {track} \
                             slot {slot}; emitting filler. (DIM cannot represent \
                             non-standard geometry.)"
                        );
                        warned = true;
                    }
                    out.extend(std::iter::repeat_n(ABSENT_FILL_BYTE, geometry.sector_size));
                }
            }
        }
    }

    out
}

/// Returns whether `disk` can be represented without data loss as DIM.
pub(crate) fn is_representable(disk: &D88Disk, header: &[u8; DIM_HEADER_SIZE]) -> bool {
    let media_type = header[0];
    let Some(geometry) = DimGeometry::from_media_type(media_type) else {
        return false;
    };
    let tracks_per_disk = geometry.tracks_per_disk();

    for track in 0..tracks_per_disk {
        let cylinder = (track / 2) as u8;
        let physical_head = (track % 2) as u8;
        for slot in 0..geometry.sectors_per_track {
            let (head, record) = sector_id(media_type, cylinder, physical_head, slot);
            let Some(sector) =
                disk.find_sector_on_track_index(track, cylinder, head, record, geometry.size_code)
            else {
                return false;
            };
            if sector.data.len() != geometry.sector_size {
                return false;
            }
        }
    }
    for track in tracks_per_disk..disk.track_slot_count() {
        if disk.sector_count(track) != 0 {
            return false;
        }
    }
    true
}
