//! FDI floppy disk image format parser.
//!
//! FDI is the floppy image format of the Anex86 PC-98 emulator. A header
//! starts with eight little-endian 32-bit geometry fields and is padded up to
//! its declared size. The raw sectors follow in cylinder, head and record
//! order. Every track holds the same number of sectors of the same size.

use std::fmt;

use common::warn;

use super::{
    FddType, FloppyGeometry,
    d88::{D88Disk, D88MediaType, D88Sector},
    geometry::{LARGEST_SECTOR_SIZE, SMALLEST_SECTOR_SIZE},
};

/// Byte count of the geometry fields at the start of every FDI header.
pub const FDI_GEOMETRY_FIELDS_SIZE: usize = 32;

/// Header size of the FDI images.
pub const FDI_STANDARD_HEADER_SIZE: usize = 4096;

/// Media code of a 2HD 1.2 MB disk (360 rpm).
pub const MEDIA_CODE_2HD: u32 = 0x90;

/// Media code of a 2HD 1.44 MB disk (300 rpm).
pub const MEDIA_CODE_2HD_1440: u32 = 0x30;

/// Media code of a 2DD 640 KB disk.
pub const MEDIA_CODE_2DD_640: u32 = 0x10;

/// Media code of a 2DD 720 KB disk.
pub const MEDIA_CODE_2DD_720: u32 = 0x70;

/// Media code of a 2D 320 KB disk.
pub const MEDIA_CODE_2D: u32 = 0x50;

/// Alternate media code of a 2HD 1.44 MB disk.
const MEDIA_CODE_2HD_1440_ALTERNATE: u32 = 0xB0;

/// Alternate media code of a 2DD disk.
const MEDIA_CODE_2DD_ALTERNATE: u32 = 0xF0;

/// Alternate media code of a 2D disk.
const MEDIA_CODE_2D_ALTERNATE: u32 = 0xD0;

/// Mask of the media code bits that select the media class.
const MEDIA_CODE_CLASS_MASK: u32 = 0xF0;

/// Byte offset of the media code field.
const MEDIA_CODE_OFFSET: usize = 0x04;

/// Byte offset of the header size field.
const HEADER_SIZE_OFFSET: usize = 0x08;

/// Byte offset of the data size field.
const DATA_SIZE_OFFSET: usize = 0x0C;

/// Byte offset of the sector size field.
const SECTOR_SIZE_OFFSET: usize = 0x10;

/// Byte offset of the sectors per track field.
const SECTORS_OFFSET: usize = 0x14;

/// Byte offset of the head count field.
const HEADS_OFFSET: usize = 0x18;

/// Byte offset of the cylinder count field.
const CYLINDERS_OFFSET: usize = 0x1C;

/// Smallest sectors per track count.
const MIN_SECTORS_PER_TRACK: u8 = 1;

/// Head count of every FDI image.
const FDI_HEADS: u8 = 2;

/// Smallest cylinder count.
const MIN_CYLINDERS: u16 = 1;

/// Largest cylinder count.
const MAX_CYLINDERS: u16 = 127;

/// The checked fields of an FDI header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FdiHeaderFields {
    /// D88 media type selected by the media code.
    media_type: D88MediaType,
    /// Byte offset of the sector data.
    header_size: usize,
    /// Geometry of the sector data.
    geometry: FloppyGeometry,
}

/// Error type for FDI parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FdiError {
    /// The image is shorter than the geometry fields.
    TooShort {
        /// Actual byte count of the image data.
        actual: usize,
    },
    /// The header size is below the geometry fields or past the end of the image.
    InvalidHeaderSize {
        /// The header size field.
        header_size: u32,
        /// Actual byte count of the image data.
        file_size: usize,
    },
    /// The media code is not in the media table.
    UnknownMediaCode(u32),
    /// The sector size is no power of two from 128 to 16384.
    InvalidSectorSize(u32),
    /// The sectors per track count is outside 1 to 255.
    InvalidSectorCount(u32),
    /// The head count is not 2.
    InvalidHeadCount(u32),
    /// The cylinder count is outside 1 to 127.
    InvalidCylinderCount(u32),
    /// The image size does not match the header size and the geometry.
    SizeMismatch {
        /// Actual byte count of the image data.
        actual: usize,
        /// Byte count given by the header size and the geometry.
        expected: u64,
    },
}

impl fmt::Display for FdiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FdiError::TooShort { actual } => write!(
                f,
                "FDI image is {actual} bytes, shorter than the {FDI_GEOMETRY_FIELDS_SIZE}-byte geometry header"
            ),
            FdiError::InvalidHeaderSize {
                header_size,
                file_size,
            } => write!(
                f,
                "FDI header size {header_size} is outside the range {FDI_GEOMETRY_FIELDS_SIZE} to {file_size}"
            ),
            FdiError::UnknownMediaCode(media_code) => {
                write!(f, "FDI media code 0x{media_code:02X} is not supported")
            }
            FdiError::InvalidSectorSize(sector_size) => write!(
                f,
                "FDI sector size {sector_size} is not a power of two from {SMALLEST_SECTOR_SIZE} to {LARGEST_SECTOR_SIZE}"
            ),
            FdiError::InvalidSectorCount(sectors) => write!(
                f,
                "FDI sectors per track count {sectors} is outside the range {MIN_SECTORS_PER_TRACK} to {}",
                u8::MAX
            ),
            FdiError::InvalidHeadCount(heads) => {
                write!(f, "FDI head count {heads} is not {FDI_HEADS}")
            }
            FdiError::InvalidCylinderCount(cylinders) => write!(
                f,
                "FDI cylinder count {cylinders} is outside the range {MIN_CYLINDERS} to {MAX_CYLINDERS}"
            ),
            FdiError::SizeMismatch { actual, expected } => {
                write!(
                    f,
                    "FDI image size is {actual} bytes, expected exactly {expected}"
                )
            }
        }
    }
}

impl std::error::Error for FdiError {}

/// Returns the media code that neetan writes for a blank disk of `fdd_type`.
pub const fn media_code(fdd_type: FddType) -> u32 {
    match fdd_type {
        FddType::Hd2 => MEDIA_CODE_2HD,
        FddType::Hd2Fmt144 => MEDIA_CODE_2HD_1440,
        FddType::Dd2 => MEDIA_CODE_2DD_640,
        FddType::Dd2Fmt720 => MEDIA_CODE_2DD_720,
        FddType::D2 => MEDIA_CODE_2D,
    }
}

/// Returns the D88 media type of a media code, or `None` for an unknown code.
fn media_type(media_code: u32) -> Option<D88MediaType> {
    match media_code & MEDIA_CODE_CLASS_MASK {
        MEDIA_CODE_2HD | MEDIA_CODE_2HD_1440 | MEDIA_CODE_2HD_1440_ALTERNATE => {
            Some(D88MediaType::Disk2HD)
        }
        MEDIA_CODE_2DD_640 | MEDIA_CODE_2DD_720 | MEDIA_CODE_2DD_ALTERNATE => {
            Some(D88MediaType::Disk2DD)
        }
        MEDIA_CODE_2D | MEDIA_CODE_2D_ALTERNATE => Some(D88MediaType::Disk2D),
        _ => None,
    }
}

/// Returns the byte count of the sector data of `geometry`.
const fn data_size(geometry: &FloppyGeometry) -> u64 {
    geometry.sector_size as u64
        * geometry.sectors_per_track as u64
        * geometry.heads as u64
        * geometry.cylinders as u64
}

/// Writes `value` little-endian at `offset` of `header`.
const fn write_field(header: &mut [u8; FDI_STANDARD_HEADER_SIZE], offset: usize, value: u32) {
    let bytes = value.to_le_bytes();
    let mut index = 0;
    while index < bytes.len() {
        header[offset + index] = bytes[index];
        index += 1;
    }
}

/// Returns whether `geometry` passes the sector size, sector count, head
/// count and cylinder count checks of an FDI header.
const fn passes_checks(geometry: &FloppyGeometry) -> bool {
    geometry.sector_size.is_power_of_two()
        && geometry.sector_size >= SMALLEST_SECTOR_SIZE
        && geometry.sector_size <= LARGEST_SECTOR_SIZE
        && geometry.sectors_per_track >= MIN_SECTORS_PER_TRACK
        && geometry.heads == FDI_HEADS
        && geometry.cylinders >= MIN_CYLINDERS
        && geometry.cylinders <= MAX_CYLINDERS
}

/// Builds a standard-size header for `media_code` and `geometry` with zero padding.
const fn encode_header(
    media_code: u32,
    geometry: &FloppyGeometry,
) -> [u8; FDI_STANDARD_HEADER_SIZE] {
    debug_assert!(passes_checks(geometry));
    let data_size_field = data_size(geometry) as u32;
    let mut header = [0u8; FDI_STANDARD_HEADER_SIZE];
    write_field(&mut header, MEDIA_CODE_OFFSET, media_code);
    write_field(
        &mut header,
        HEADER_SIZE_OFFSET,
        FDI_STANDARD_HEADER_SIZE as u32,
    );
    write_field(&mut header, DATA_SIZE_OFFSET, data_size_field);
    write_field(&mut header, SECTOR_SIZE_OFFSET, geometry.sector_size as u32);
    write_field(
        &mut header,
        SECTORS_OFFSET,
        geometry.sectors_per_track as u32,
    );
    write_field(&mut header, HEADS_OFFSET, geometry.heads as u32);
    write_field(&mut header, CYLINDERS_OFFSET, geometry.cylinders as u32);
    header
}

/// Builds a 4096-byte header for `media_code` and `geometry` with zero padding
/// and the data size field filled in. The geometry must pass the FDI checks.
pub fn header(media_code: u32, geometry: &FloppyGeometry) -> Vec<u8> {
    encode_header(media_code, geometry).to_vec()
}

/// Returns the header of a 2HD 1.2 MB image: 77 cylinders, 2 heads, 8 sectors of 1024 bytes.
pub const fn standard_2hd_header() -> [u8; FDI_STANDARD_HEADER_SIZE] {
    encode_header(MEDIA_CODE_2HD, &FddType::Hd2.geometry())
}

/// Reads the little-endian field at `offset` of `header`.
fn read_field(header: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        header[offset],
        header[offset + 1],
        header[offset + 2],
        header[offset + 3],
    ])
}

/// Reads the header at the start of `header` and runs the checks 1 to 7: the
/// length, the header size, the media code, the sector size, the sectors per
/// track, the heads and the cylinders. The caller checks the file size.
fn read_header(header: &[u8]) -> Result<FdiHeaderFields, FdiError> {
    if header.len() < FDI_GEOMETRY_FIELDS_SIZE {
        return Err(FdiError::TooShort {
            actual: header.len(),
        });
    }

    let header_size_field = read_field(header, HEADER_SIZE_OFFSET);
    let header_size = usize::try_from(header_size_field)
        .ok()
        .filter(|&size| (FDI_GEOMETRY_FIELDS_SIZE..=header.len()).contains(&size))
        .ok_or(FdiError::InvalidHeaderSize {
            header_size: header_size_field,
            file_size: header.len(),
        })?;

    let media_code = read_field(header, MEDIA_CODE_OFFSET);
    let media_type = media_type(media_code).ok_or(FdiError::UnknownMediaCode(media_code))?;

    let sector_size_field = read_field(header, SECTOR_SIZE_OFFSET);
    let sector_size = u16::try_from(sector_size_field)
        .ok()
        .filter(|size| {
            size.is_power_of_two() && (SMALLEST_SECTOR_SIZE..=LARGEST_SECTOR_SIZE).contains(size)
        })
        .ok_or(FdiError::InvalidSectorSize(sector_size_field))?;

    let sectors_field = read_field(header, SECTORS_OFFSET);
    let sectors_per_track = u8::try_from(sectors_field)
        .ok()
        .filter(|&sectors| sectors >= MIN_SECTORS_PER_TRACK)
        .ok_or(FdiError::InvalidSectorCount(sectors_field))?;

    let heads_field = read_field(header, HEADS_OFFSET);
    if heads_field != u32::from(FDI_HEADS) {
        return Err(FdiError::InvalidHeadCount(heads_field));
    }

    let cylinders_field = read_field(header, CYLINDERS_OFFSET);
    let cylinders = u16::try_from(cylinders_field)
        .ok()
        .filter(|cylinders| (MIN_CYLINDERS..=MAX_CYLINDERS).contains(cylinders))
        .ok_or(FdiError::InvalidCylinderCount(cylinders_field))?;

    Ok(FdiHeaderFields {
        media_type,
        header_size,
        geometry: FloppyGeometry {
            cylinders,
            heads: FDI_HEADS,
            sectors_per_track,
            sector_size,
        },
    })
}

/// Returns the track index, cylinder, head and record of every sector of
/// `geometry` in file order.
fn sector_slots(geometry: FloppyGeometry) -> impl Iterator<Item = (usize, u8, u8, u8)> {
    (0..geometry.cylinders).flat_map(move |cylinder| {
        (0..geometry.heads).flat_map(move |head| {
            let track_index = usize::from(cylinder) * 2 + usize::from(head);
            (1..=geometry.sectors_per_track)
                .map(move |record| (track_index, cylinder as u8, head, record))
        })
    })
}

/// Parses an FDI disk image, returning the disk and a copy of the header.
pub fn from_bytes(data: &[u8]) -> Result<(D88Disk, Box<[u8]>), FdiError> {
    let fields = read_header(data)?;
    let geometry = fields.geometry;
    let expected = fields.header_size as u64 + data_size(&geometry);
    if data.len() as u64 != expected {
        return Err(FdiError::SizeMismatch {
            actual: data.len(),
            expected,
        });
    }

    let sector_size = usize::from(geometry.sector_size);
    let size_code = geometry.size_code();
    let track_count = usize::from(geometry.cylinders) * usize::from(geometry.heads);
    let mut track_sectors: Vec<Option<Vec<D88Sector>>> = vec![None; track_count];
    let mut offset = fields.header_size;

    for (track_index, cylinder, head, record) in sector_slots(geometry) {
        let end = offset + sector_size;
        track_sectors[track_index]
            .get_or_insert_with(|| Vec::with_capacity(usize::from(geometry.sectors_per_track)))
            .push(D88Sector {
                cylinder,
                head,
                record,
                size_code,
                sector_count: u16::from(geometry.sectors_per_track),
                mfm_flag: 0x00,
                deleted: 0x00,
                status: 0x00,
                reserved: [0u8; 5],
                data: data[offset..end].to_vec(),
                source_offset: Some(offset as u64),
            });
        offset = end;
    }

    let header = data[..fields.header_size].to_vec().into_boxed_slice();
    Ok((
        D88Disk::from_tracks(String::new(), false, fields.media_type, track_sectors),
        header,
    ))
}

/// Serializes a `D88Disk` into the FDI layout of `header`.
///
/// The header is written verbatim. A missing sector or a sector of another
/// size is emitted as zero bytes. FDI cannot represent any other layout.
pub fn to_bytes(disk: &D88Disk, header: &[u8]) -> Vec<u8> {
    let fields = match read_header(header) {
        Ok(fields) => fields,
        Err(error) => {
            warn!("FDI serializer: invalid header ({error}); emitting the header only");
            return header.to_vec();
        }
    };
    let geometry = fields.geometry;
    let sector_size = usize::from(geometry.sector_size);
    let size_code = geometry.size_code();

    let mut out = Vec::with_capacity(fields.header_size + data_size(&geometry) as usize);
    out.extend_from_slice(&header[..fields.header_size]);

    let mut warned = false;
    for (track_index, cylinder, head, record) in sector_slots(geometry) {
        match disk.find_sector_on_track_index(track_index, cylinder, head, record, size_code) {
            Some(sector) if sector.data.len() == sector_size => {
                out.extend_from_slice(&sector.data);
            }
            _ => {
                if !warned {
                    warn!(
                        "FDI serializer: missing or wrong-sized sector at \
                         C={cylinder} H={head} R={record}; emitting zeros. \
                         (FDI cannot represent non-standard geometry.)"
                    );
                    warned = true;
                }
                out.resize(out.len() + sector_size, 0);
            }
        }
    }

    out
}

/// Returns whether `disk` can be represented without data loss in the FDI
/// layout of `header`.
pub(crate) fn is_representable(disk: &D88Disk, header: &[u8]) -> bool {
    let Ok(fields) = read_header(header) else {
        return false;
    };
    let geometry = fields.geometry;
    let sector_size = usize::from(geometry.sector_size);
    let size_code = geometry.size_code();
    let track_count = usize::from(geometry.cylinders) * usize::from(geometry.heads);

    let tracks_match = (0..track_count)
        .all(|track| disk.sector_count(track) == usize::from(geometry.sectors_per_track));
    let sectors_match = sector_slots(geometry).all(|(track_index, cylinder, head, record)| {
        disk.find_sector_on_track_index(track_index, cylinder, head, record, size_code)
            .is_some_and(|sector| sector.data.len() == sector_size)
    });
    let no_extra_tracks =
        (track_count..disk.track_slot_count()).all(|track| disk.sector_count(track) == 0);
    tracks_match && sectors_match && no_extra_tracks
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every blank floppy type.
    const FDD_TYPES: [FddType; 5] = [
        FddType::Hd2,
        FddType::Hd2Fmt144,
        FddType::Dd2,
        FddType::Dd2Fmt720,
        FddType::D2,
    ];

    /// Builds an image of `media_code` and `geometry` with a 4096-byte header
    /// and a byte pattern in the sector data.
    fn build_image(media_code: u32, geometry: &FloppyGeometry) -> Vec<u8> {
        let mut image = header(media_code, geometry);
        let size = data_size(geometry) as usize;
        image.extend((0..size).map(|index| (index % 251) as u8));
        image
    }

    /// Builds a minimal valid image: a 32-byte header, 1 cylinder, 2 heads
    /// and 1 sector of 128 bytes per track.
    fn build_small_image() -> Vec<u8> {
        let geometry = FloppyGeometry {
            cylinders: 1,
            heads: 2,
            sectors_per_track: 1,
            sector_size: 128,
        };
        let mut image = header(MEDIA_CODE_2HD, &geometry);
        image.truncate(FDI_GEOMETRY_FIELDS_SIZE);
        image[HEADER_SIZE_OFFSET..HEADER_SIZE_OFFSET + 4]
            .copy_from_slice(&(FDI_GEOMETRY_FIELDS_SIZE as u32).to_le_bytes());
        image.resize(FDI_GEOMETRY_FIELDS_SIZE + 256, 0xE5);
        image
    }

    /// Writes `value` into the field at `offset` of `image`.
    fn set_field(image: &mut [u8], offset: usize, value: u32) {
        image[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    /// Verifies that a 2HD image parses into 154 tracks of 8 sectors with their IDs and offsets.
    #[test]
    fn parses_2hd_image() {
        let geometry = FddType::Hd2.geometry();
        let image = build_image(MEDIA_CODE_2HD, &geometry);
        let (disk, header) = from_bytes(&image).unwrap();

        assert_eq!(disk.media_type, D88MediaType::Disk2HD);
        assert_eq!(header.len(), FDI_STANDARD_HEADER_SIZE);
        assert_eq!(disk.track_slot_count(), 154);
        for track in 0..154 {
            assert_eq!(disk.sector_count(track), 8, "track {track}");
        }
        for cylinder in [0u8, 40, 76] {
            for head in 0..2u8 {
                for record in 1..=8u8 {
                    let sector = disk.find_sector(cylinder, head, record, 3).unwrap();
                    assert_eq!(sector.cylinder, cylinder);
                    assert_eq!(sector.head, head);
                    assert_eq!(sector.record, record);
                    assert_eq!(sector.size_code, 3);
                    assert_eq!(sector.data.len(), 1024);
                }
            }
        }

        let first = disk.find_sector(0, 0, 1, 3).unwrap();
        assert_eq!(first.source_offset, Some(4096));
        assert_eq!(first.data[..], image[4096..4096 + 1024]);

        let last_offset = 4096 + (153 * 8 + 7) * 1024;
        let last = disk.find_sector(76, 1, 8, 3).unwrap();
        assert_eq!(last.source_offset, Some(last_offset as u64));
        assert_eq!(last.data[..], image[last_offset..]);
    }

    /// Verifies that the 1.44 MB, 640 KB, 720 KB and 320 KB images parse with their media types.
    #[test]
    fn parses_every_media_class() {
        for (media_code, fdd_type, media) in [
            (
                MEDIA_CODE_2HD_1440,
                FddType::Hd2Fmt144,
                D88MediaType::Disk2HD,
            ),
            (MEDIA_CODE_2DD_640, FddType::Dd2, D88MediaType::Disk2DD),
            (
                MEDIA_CODE_2DD_720,
                FddType::Dd2Fmt720,
                D88MediaType::Disk2DD,
            ),
            (MEDIA_CODE_2D, FddType::D2, D88MediaType::Disk2D),
        ] {
            let geometry = fdd_type.geometry();
            let image = build_image(media_code, &geometry);
            let (disk, _) = from_bytes(&image).unwrap();
            assert_eq!(disk.media_type, media, "{fdd_type:?}");
            let track_count = usize::from(geometry.cylinders) * 2;
            assert_eq!(disk.track_slot_count(), track_count, "{fdd_type:?}");
            for track in 0..track_count {
                assert_eq!(
                    disk.sector_count(track),
                    usize::from(geometry.sectors_per_track),
                    "{fdd_type:?} track {track}"
                );
            }
            let sector = disk.find_sector(0, 1, 1, geometry.size_code()).unwrap();
            assert_eq!(sector.data.len(), usize::from(geometry.sector_size));
        }
    }

    /// Verifies that the media table reads the high nibble of every media code.
    #[test]
    fn media_table_reads_the_high_nibble() {
        for (media_code, media) in [
            (0x90, Some(D88MediaType::Disk2HD)),
            (0x93, Some(D88MediaType::Disk2HD)),
            (0x30, Some(D88MediaType::Disk2HD)),
            (0xB0, Some(D88MediaType::Disk2HD)),
            (0x10, Some(D88MediaType::Disk2DD)),
            (0x70, Some(D88MediaType::Disk2DD)),
            (0xF0, Some(D88MediaType::Disk2DD)),
            (0x50, Some(D88MediaType::Disk2D)),
            (0xD0, Some(D88MediaType::Disk2D)),
            (0x00, None),
            (0x20, None),
            (0x80, None),
        ] {
            assert_eq!(media_type(media_code), media, "0x{media_code:02X}");
        }
    }

    /// Verifies that every image serializes back to its exact bytes, including non-zero header padding.
    #[test]
    fn round_trips_every_image() {
        for fdd_type in FDD_TYPES {
            let geometry = fdd_type.geometry();
            let mut image = build_image(media_code(fdd_type), &geometry);
            let (disk, header) = from_bytes(&image).unwrap();
            assert_eq!(to_bytes(&disk, &header), image, "{fdd_type:?}");

            image[FDI_GEOMETRY_FIELDS_SIZE..FDI_GEOMETRY_FIELDS_SIZE + 16]
                .copy_from_slice(b"Anex86 padding!!");
            image[FDI_STANDARD_HEADER_SIZE - 1] = 0xA5;
            let (disk, header) = from_bytes(&image).unwrap();
            assert_eq!(to_bytes(&disk, &header), image, "{fdd_type:?} with padding");
        }
    }

    /// Verifies that a header size field of 256 places the sector data at offset 256.
    #[test]
    fn header_size_field_places_the_sector_data() {
        const HEADER_SIZE: usize = 256;
        let geometry = FddType::Hd2.geometry();
        let mut image = build_image(MEDIA_CODE_2HD, &geometry);
        image.drain(HEADER_SIZE..FDI_STANDARD_HEADER_SIZE);
        set_field(&mut image, HEADER_SIZE_OFFSET, HEADER_SIZE as u32);

        let (disk, header) = from_bytes(&image).unwrap();
        assert_eq!(header.len(), HEADER_SIZE);
        let first = disk.find_sector(0, 0, 1, 3).unwrap();
        assert_eq!(first.source_offset, Some(HEADER_SIZE as u64));
        assert_eq!(first.data[..], image[HEADER_SIZE..HEADER_SIZE + 1024]);
        assert_eq!(to_bytes(&disk, &header), image);
    }

    /// Verifies that the minimal test image is valid.
    #[test]
    fn small_image_parses() {
        let (disk, _) = from_bytes(&build_small_image()).unwrap();
        assert_eq!(disk.sector_count(0), 1);
        assert_eq!(disk.sector_count(1), 1);
    }

    /// Verifies that every failed check rejects the image with its error.
    #[test]
    fn rejects_invalid_images() {
        let valid = build_small_image();
        let file_size = valid.len();

        assert_eq!(
            from_bytes(&valid[..31]).unwrap_err(),
            FdiError::TooShort { actual: 31 }
        );

        for header_size in [0u32, 31, file_size as u32 + 1] {
            let mut image = valid.clone();
            set_field(&mut image, HEADER_SIZE_OFFSET, header_size);
            assert_eq!(
                from_bytes(&image).unwrap_err(),
                FdiError::InvalidHeaderSize {
                    header_size,
                    file_size
                }
            );
        }

        let mut image = valid.clone();
        set_field(&mut image, MEDIA_CODE_OFFSET, 0x20);
        assert_eq!(
            from_bytes(&image).unwrap_err(),
            FdiError::UnknownMediaCode(0x20)
        );

        for sector_size in [0u32, 100, 32768] {
            let mut image = valid.clone();
            set_field(&mut image, SECTOR_SIZE_OFFSET, sector_size);
            assert_eq!(
                from_bytes(&image).unwrap_err(),
                FdiError::InvalidSectorSize(sector_size)
            );
        }

        for sectors in [0u32, 256] {
            let mut image = valid.clone();
            set_field(&mut image, SECTORS_OFFSET, sectors);
            assert_eq!(
                from_bytes(&image).unwrap_err(),
                FdiError::InvalidSectorCount(sectors)
            );
        }

        for heads in [1u32, 3] {
            let mut image = valid.clone();
            set_field(&mut image, HEADS_OFFSET, heads);
            assert_eq!(
                from_bytes(&image).unwrap_err(),
                FdiError::InvalidHeadCount(heads)
            );
        }

        for cylinders in [0u32, 128] {
            let mut image = valid.clone();
            set_field(&mut image, CYLINDERS_OFFSET, cylinders);
            assert_eq!(
                from_bytes(&image).unwrap_err(),
                FdiError::InvalidCylinderCount(cylinders)
            );
        }

        let mut image = valid.clone();
        image.push(0);
        assert_eq!(
            from_bytes(&image).unwrap_err(),
            FdiError::SizeMismatch {
                actual: file_size + 1,
                expected: file_size as u64
            }
        );
    }

    /// Verifies that the parser ignores the data size field.
    #[test]
    fn ignores_the_data_size_field() {
        let mut image = build_small_image();
        set_field(&mut image, DATA_SIZE_OFFSET, 0x1234_5678);
        assert!(from_bytes(&image).is_ok());
    }

    /// Verifies that a track formatted with 256-byte sectors makes the disk unrepresentable.
    #[test]
    fn formatted_track_with_other_sector_size_is_not_representable() {
        let image = build_image(MEDIA_CODE_2HD, &FddType::Hd2.geometry());
        let (mut disk, header) = from_bytes(&image).unwrap();
        assert!(is_representable(&disk, &header));

        disk.format_track(0, &[(0, 0, 1, 1)], 1, 0xE5);
        assert!(!is_representable(&disk, &header));
    }

    /// Verifies that a sector on a track past the last cylinder makes the disk unrepresentable.
    #[test]
    fn extra_track_is_not_representable() {
        let image = build_image(MEDIA_CODE_2HD, &FddType::Hd2.geometry());
        let (mut disk, header) = from_bytes(&image).unwrap();
        disk.format_track(154, &[(77, 0, 1, 3)], 3, 0xE5);
        assert!(!is_representable(&disk, &header));
    }

    /// Verifies that the header of each blank floppy type parses back to its geometry and media type.
    #[test]
    fn blank_header_of_every_type_parses_back() {
        for fdd_type in FDD_TYPES {
            let geometry = fdd_type.geometry();
            let mut image = header(media_code(fdd_type), &geometry);
            assert_eq!(image.len(), FDI_STANDARD_HEADER_SIZE);
            assert_eq!(
                u64::from(read_field(&image, DATA_SIZE_OFFSET)),
                data_size(&geometry)
            );
            image.resize(image.len() + data_size(&geometry) as usize, 0);

            let fields = read_header(&image).unwrap();
            assert_eq!(fields.geometry, geometry, "{fdd_type:?}");
            assert_eq!(fields.media_type, fdd_type.media_type(), "{fdd_type:?}");
            let (disk, _) = from_bytes(&image).unwrap();
            assert_eq!(disk.media_type, fdd_type.media_type(), "{fdd_type:?}");
        }
    }

    /// Verifies that the standard 2HD header equals the header of the Anex86 sample images.
    #[test]
    fn standard_2hd_header_matches_the_samples() {
        const SAMPLE_GEOMETRY_FIELDS: [u8; FDI_GEOMETRY_FIELDS_SIZE] = [
            0x00, 0x00, 0x00, 0x00, 0x90, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x40,
            0x13, 0x00, 0x00, 0x04, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00,
            0x4D, 0x00, 0x00, 0x00,
        ];
        let mut expected = [0u8; FDI_STANDARD_HEADER_SIZE];
        expected[..FDI_GEOMETRY_FIELDS_SIZE].copy_from_slice(&SAMPLE_GEOMETRY_FIELDS);
        assert_eq!(standard_2hd_header(), expected);
    }
}
