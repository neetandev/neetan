//! ISO CD-ROM images: one data track of 2048-byte or raw 2352-byte sectors
//! without a control file.

use super::{
    CdError, CueSheet, CueTrack, RAW_MODE1_DATA_OFFSET, RAW_MODE2_DATA_OFFSET, SECTOR_SIZE_COOKED,
    SECTOR_SIZE_RAW, SectorLayout, TrackType,
};

/// The 12-byte sync pattern at the start of a raw sector (ECMA-130).
pub(super) const RAW_SECTOR_SYNC: [u8; 12] = [
    0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00,
];

/// Offset of the mode byte within a raw sector.
const RAW_MODE_BYTE_OFFSET: usize = 15;

/// The sector that holds the first volume descriptor.
const VOLUME_DESCRIPTOR_SECTOR: usize = 16;

/// The standard identifier of an ISO 9660 volume descriptor.
const ISO9660_IDENTIFIER: &[u8; 5] = b"CD001";

/// Offset of the ISO 9660 identifier within the volume descriptor.
const ISO9660_IDENTIFIER_OFFSET: usize = 1;

/// The standard identifier of a High Sierra volume descriptor.
const HIGH_SIERRA_IDENTIFIER: &[u8; 5] = b"CDROM";

/// Offset of the High Sierra identifier within the volume descriptor.
const HIGH_SIERRA_IDENTIFIER_OFFSET: usize = 9;

/// The version byte that follows the identifier of a volume descriptor.
const VOLUME_DESCRIPTOR_VERSION: u8 = 1;

/// Returns the raw layout of a sector that starts with the sync pattern and holds mode 1 or 2.
fn raw_sector_layout(sector: &[u8]) -> Option<SectorLayout> {
    if !sector.starts_with(&RAW_SECTOR_SYNC) {
        return None;
    }
    match sector.get(RAW_MODE_BYTE_OFFSET)? {
        1 => Some(SectorLayout::RawMode1),
        2 => Some(SectorLayout::RawMode2),
        _ => None,
    }
}

/// Returns whether the user data holds an ISO 9660 or High Sierra volume descriptor.
fn holds_volume_descriptor(user_data: &[u8]) -> bool {
    [
        (ISO9660_IDENTIFIER, ISO9660_IDENTIFIER_OFFSET),
        (HIGH_SIERRA_IDENTIFIER, HIGH_SIERRA_IDENTIFIER_OFFSET),
    ]
    .into_iter()
    .any(|(identifier, offset)| {
        let version_offset = offset + identifier.len();
        user_data.get(offset..version_offset) == Some(identifier.as_slice())
            && user_data.get(version_offset) == Some(&VOLUME_DESCRIPTOR_VERSION)
    })
}

/// Detects the sector layout of an ISO image from its volume descriptor or its length.
pub(super) fn detect_iso_layout(data: &[u8]) -> Result<SectorLayout, CdError> {
    let raw_size = usize::from(SECTOR_SIZE_RAW);
    let cooked_size = usize::from(SECTOR_SIZE_COOKED);

    let raw_descriptor_start = VOLUME_DESCRIPTOR_SECTOR * raw_size;
    if let Some(sector) = data.get(raw_descriptor_start..raw_descriptor_start + raw_size)
        && let Some(layout) = raw_sector_layout(sector)
    {
        let user_data_offset = if layout == SectorLayout::RawMode2 {
            RAW_MODE2_DATA_OFFSET
        } else {
            RAW_MODE1_DATA_OFFSET
        };
        if holds_volume_descriptor(&sector[user_data_offset..]) {
            return Ok(layout);
        }
    }

    let cooked_descriptor_start = VOLUME_DESCRIPTOR_SECTOR * cooked_size;
    if let Some(sector) = data.get(cooked_descriptor_start..cooked_descriptor_start + cooked_size)
        && holds_volume_descriptor(sector)
    {
        return Ok(SectorLayout::Cooked);
    }

    if !data.is_empty()
        && data.len().is_multiple_of(raw_size)
        && let Some(layout) = raw_sector_layout(&data[..raw_size])
    {
        return Ok(layout);
    }

    if !data.is_empty() && data.len().is_multiple_of(cooked_size) {
        return Ok(SectorLayout::Cooked);
    }

    Err(CdError::UnsupportedFormat(format!(
        "ISO image of {} bytes holds neither 2048-byte nor 2352-byte sectors",
        data.len()
    )))
}

/// Builds the sheet of the single data track of an ISO image.
pub(super) fn iso_sheet(layout: SectorLayout) -> CueSheet {
    let sector_size = match layout {
        SectorLayout::Cooked => SECTOR_SIZE_COOKED,
        SectorLayout::RawMode1 | SectorLayout::RawMode2 | SectorLayout::Audio => SECTOR_SIZE_RAW,
    };
    CueSheet {
        file_names: vec!["image.iso".to_string()],
        tracks: vec![CueTrack {
            number: 1,
            track_type: TrackType::Data,
            sector_size,
            sector_layout: layout,
            file_index: 0,
            index00_lba: None,
            index01_lba: Some(0),
            pregap_sectors: 0,
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::{
        super::{
            CdImage,
            tests::{make_raw_data_sector, make_raw_mode2_sector},
        },
        *,
    };

    /// Size of a cooked sector in bytes.
    const COOKED: usize = SECTOR_SIZE_COOKED as usize;

    /// Size of a raw sector in bytes.
    const RAW: usize = SECTOR_SIZE_RAW as usize;

    /// Builds a cooked image whose sectors hold their index in the first two bytes and whose
    /// sector 16 holds the given descriptor identifier at its offset with the version byte 1.
    fn make_cooked_iso(sector_count: usize, descriptor: Option<(&[u8; 5], usize)>) -> Vec<u8> {
        let mut data = vec![0u8; sector_count * COOKED];
        for (index, sector) in data.as_chunks_mut::<COOKED>().0.iter_mut().enumerate() {
            sector[..2].copy_from_slice(&(index as u16).to_le_bytes());
        }
        if let Some((identifier, offset)) = descriptor {
            let start = VOLUME_DESCRIPTOR_SECTOR * COOKED + offset;
            data[start..start + identifier.len()].copy_from_slice(identifier);
            data[start + identifier.len()] = VOLUME_DESCRIPTOR_VERSION;
        }
        data
    }

    /// Builds a raw image of Mode 1 sectors whose user data holds the sector index and whose
    /// sector 16 holds an ISO 9660 descriptor.
    fn make_raw_mode1_iso(sector_count: usize) -> Vec<u8> {
        make_raw_iso(sector_count, make_raw_data_sector, RAW_MODE1_DATA_OFFSET)
    }

    /// Builds a raw image of Mode 2 sectors whose user data holds the sector index and whose
    /// sector 16 holds an ISO 9660 descriptor.
    fn make_raw_mode2_iso(sector_count: usize) -> Vec<u8> {
        make_raw_iso(sector_count, make_raw_mode2_sector, RAW_MODE2_DATA_OFFSET)
    }

    /// Builds a raw image from a sector builder and writes an ISO 9660 descriptor into the user
    /// data of sector 16.
    fn make_raw_iso(
        sector_count: usize,
        make_sector: fn(u8) -> Vec<u8>,
        user_data_offset: usize,
    ) -> Vec<u8> {
        let mut data: Vec<u8> = (0..sector_count)
            .flat_map(|index| make_sector(index as u8))
            .collect();
        let start = VOLUME_DESCRIPTOR_SECTOR * RAW + user_data_offset;
        data[start] = 0x01;
        data[start + 1..start + 6].copy_from_slice(ISO9660_IDENTIFIER);
        data[start + 6] = VOLUME_DESCRIPTOR_VERSION;
        data
    }

    /// Verifies that a cooked image with an ISO 9660 descriptor loads as one cooked data track.
    #[test]
    fn iso_cooked_with_iso9660_descriptor() {
        let data = make_cooked_iso(20, Some((ISO9660_IDENTIFIER, ISO9660_IDENTIFIER_OFFSET)));
        let image = CdImage::from_iso(data.clone()).unwrap();

        assert_eq!(image.track_count(), 1);
        assert_eq!(image.total_sectors(), 20);
        let track = image.track(1).unwrap();
        assert_eq!(track.track_type, TrackType::Data);
        assert_eq!(track.sector_count, 20);
        assert_eq!(track.sector_size, 2048);
        assert_eq!(track.start_lba, 0);

        let mut buffer = [0u8; 2048];
        assert_eq!(image.read_sector(5, &mut buffer), Some(2048));
        assert_eq!(buffer.as_slice(), &data[5 * COOKED..6 * COOKED]);

        let mut raw_buffer = [0u8; 2352];
        assert_eq!(image.read_sector_raw(5, &mut raw_buffer), Some(2048));

        let mut padded = data;
        padded.extend_from_slice(&[0u8; 100]);
        assert_eq!(detect_iso_layout(&padded).unwrap(), SectorLayout::Cooked);
    }

    /// Verifies that a High Sierra descriptor gives the cooked layout.
    #[test]
    fn iso_cooked_with_high_sierra_descriptor() {
        let mut data = make_cooked_iso(
            20,
            Some((HIGH_SIERRA_IDENTIFIER, HIGH_SIERRA_IDENTIFIER_OFFSET)),
        );
        data.extend_from_slice(&[0u8; 100]);
        assert_eq!(detect_iso_layout(&data).unwrap(), SectorLayout::Cooked);
    }

    /// Verifies that zero sectors without a descriptor give the cooked layout by their length.
    #[test]
    fn iso_cooked_without_descriptor() {
        let data = vec![0u8; 4 * COOKED];
        assert_eq!(detect_iso_layout(&data).unwrap(), SectorLayout::Cooked);
        let image = CdImage::from_iso(data).unwrap();
        assert_eq!(image.total_sectors(), 4);
    }

    /// Verifies that a volume descriptor needs the version byte 1.
    #[test]
    fn iso_descriptor_needs_the_version_byte() {
        for (identifier, offset) in [
            (ISO9660_IDENTIFIER, ISO9660_IDENTIFIER_OFFSET),
            (HIGH_SIERRA_IDENTIFIER, HIGH_SIERRA_IDENTIFIER_OFFSET),
        ] {
            let mut data = make_cooked_iso(20, Some((identifier, offset)));
            data[VOLUME_DESCRIPTOR_SECTOR * COOKED + offset + identifier.len()] = 0;
            data.extend_from_slice(&[0u8; 100]);
            assert!(matches!(
                detect_iso_layout(&data),
                Err(CdError::UnsupportedFormat(_))
            ));
        }
    }

    /// Verifies that a descriptor allows a partial last sector, which the track drops.
    #[test]
    fn iso_cooked_with_trailing_partial_sector() {
        let mut data = make_cooked_iso(20, Some((ISO9660_IDENTIFIER, ISO9660_IDENTIFIER_OFFSET)));
        data.extend_from_slice(&[0u8; 100]);
        let image = CdImage::from_iso(data).unwrap();
        assert_eq!(image.total_sectors(), 20);
        assert_eq!(image.track(1).unwrap().sector_count, 20);
    }

    /// Verifies that raw Mode 1 sectors with a descriptor load as a raw Mode 1 track.
    #[test]
    fn iso_raw_mode1_with_descriptor() {
        let mut data = make_raw_mode1_iso(20);
        data.extend_from_slice(&[0u8; 100]);
        assert_eq!(detect_iso_layout(&data).unwrap(), SectorLayout::RawMode1);
        let image = CdImage::from_iso(data.clone()).unwrap();
        let track = image.track(1).unwrap();
        assert_eq!(track.sector_size, 2352);
        assert_eq!(track.sector_count, 20);

        let mut buffer = [0u8; 2048];
        assert_eq!(image.read_sector(5, &mut buffer), Some(2048));
        assert_eq!(
            buffer.as_slice(),
            &data[5 * RAW + RAW_MODE1_DATA_OFFSET..5 * RAW + RAW_MODE1_DATA_OFFSET + COOKED]
        );
        assert_eq!(image.read_sector(16, &mut buffer), Some(2048));
        assert_eq!(&buffer[1..6], ISO9660_IDENTIFIER);

        let mut raw_buffer = [0u8; 2352];
        assert_eq!(image.read_sector_raw(5, &mut raw_buffer), Some(2352));
        assert_eq!(raw_buffer.as_slice(), &data[5 * RAW..6 * RAW]);
    }

    /// Verifies that raw Mode 2 sectors with a descriptor load as a raw Mode 2 track.
    #[test]
    fn iso_raw_mode2_with_descriptor() {
        let mut data = make_raw_mode2_iso(20);
        data.extend_from_slice(&[0u8; 100]);
        assert_eq!(detect_iso_layout(&data).unwrap(), SectorLayout::RawMode2);
        let image = CdImage::from_iso(data.clone()).unwrap();
        let track = image.track(1).unwrap();
        assert_eq!(track.sector_size, 2352);
        assert_eq!(track.sector_count, 20);

        let mut buffer = [0u8; 2048];
        assert_eq!(image.read_sector(16, &mut buffer), Some(2048));
        assert_eq!(
            buffer.as_slice(),
            &data[16 * RAW + RAW_MODE2_DATA_OFFSET..16 * RAW + RAW_MODE2_DATA_OFFSET + COOKED]
        );
        assert_eq!(&buffer[1..6], ISO9660_IDENTIFIER);

        let mut raw_buffer = [0u8; 2352];
        assert_eq!(image.read_sector_raw(5, &mut raw_buffer), Some(2352));
        assert_eq!(raw_buffer.as_slice(), &data[5 * RAW..6 * RAW]);
    }

    /// Verifies that raw sectors without a descriptor give the raw layout by the sync pattern.
    #[test]
    fn iso_raw_without_descriptor() {
        let data: Vec<u8> = (0..3).flat_map(|_| make_raw_data_sector(0x5A)).collect();
        assert_eq!(detect_iso_layout(&data).unwrap(), SectorLayout::RawMode1);
        let image = CdImage::from_iso(data).unwrap();
        assert_eq!(image.total_sectors(), 3);
        let mut buffer = [0u8; 2048];
        assert_eq!(image.read_sector(2, &mut buffer), Some(2048));
        assert!(buffer.iter().all(|&byte| byte == 0x5A));
    }

    /// Verifies that a cooked descriptor decides a length that fits both sector sizes.
    #[test]
    fn iso_cooked_length_of_both_sizes() {
        let mut data = make_cooked_iso(147, Some((ISO9660_IDENTIFIER, ISO9660_IDENTIFIER_OFFSET)));
        data[..RAW_SECTOR_SYNC.len()].copy_from_slice(&RAW_SECTOR_SYNC);
        data[RAW_MODE_BYTE_OFFSET] = 1;
        assert_eq!(data.len(), 128 * RAW);
        assert_eq!(detect_iso_layout(&data).unwrap(), SectorLayout::Cooked);
    }

    /// Verifies that a raw descriptor decides a length that fits both sector sizes.
    #[test]
    fn iso_raw_length_of_both_sizes() {
        let mut data = make_raw_mode1_iso(128);
        data[..RAW_SECTOR_SYNC.len()].fill(0);
        assert_eq!(data.len(), 147 * COOKED);
        assert_eq!(detect_iso_layout(&data).unwrap(), SectorLayout::RawMode1);
    }

    /// Verifies that the ISO track equals the track of a single-track CUE sheet of each mode.
    #[test]
    fn iso_equals_single_track_cue() {
        let cases = [
            (
                "MODE1/2048",
                make_cooked_iso(20, Some((ISO9660_IDENTIFIER, ISO9660_IDENTIFIER_OFFSET))),
            ),
            ("MODE1/2352", make_raw_mode1_iso(20)),
            ("MODE2/2352", make_raw_mode2_iso(20)),
        ];
        for (mode, data) in cases {
            let cue = format!("FILE \"x.bin\" BINARY\n  TRACK 01 {mode}\n    INDEX 01 00:00:00\n");
            let from_cue = CdImage::from_cue(&cue, data.clone()).unwrap();
            let from_iso = CdImage::from_iso(data).unwrap();

            assert_eq!(from_iso.total_sectors(), from_cue.total_sectors(), "{mode}");
            assert_eq!(from_iso.tracks().len(), 1, "{mode}");
            assert_eq!(from_cue.tracks().len(), 1, "{mode}");
            let iso_track = &from_iso.tracks()[0];
            let cue_track = &from_cue.tracks()[0];
            assert_eq!(iso_track.number, cue_track.number, "{mode}");
            assert_eq!(iso_track.track_type, cue_track.track_type, "{mode}");
            assert_eq!(iso_track.sector_size, cue_track.sector_size, "{mode}");
            assert_eq!(iso_track.start_lba, cue_track.start_lba, "{mode}");
            assert_eq!(iso_track.pregap_lba, cue_track.pregap_lba, "{mode}");
            assert_eq!(iso_track.sector_count, cue_track.sector_count, "{mode}");
            assert_eq!(iso_track.file_offset, cue_track.file_offset, "{mode}");
            assert_eq!(iso_track.sector_layout, cue_track.sector_layout, "{mode}");
        }
    }

    /// Verifies that an empty image fails.
    #[test]
    fn iso_rejects_empty_file() {
        assert!(matches!(
            CdImage::from_iso(Vec::new()),
            Err(CdError::UnsupportedFormat(_))
        ));
    }

    /// Verifies that a length of neither sector size without a descriptor fails.
    #[test]
    fn iso_rejects_odd_length() {
        assert!(matches!(
            CdImage::from_iso(vec![0u8; 2048 * 3 + 100]),
            Err(CdError::UnsupportedFormat(_))
        ));
    }

    /// Verifies that raw sectors with an unknown mode byte fail.
    #[test]
    fn iso_rejects_raw_sync_with_unknown_mode() {
        let data: Vec<u8> = (0..3)
            .flat_map(|_| {
                let mut sector = make_raw_data_sector(0x00);
                sector[RAW_MODE_BYTE_OFFSET] = 0;
                sector
            })
            .collect();
        assert_eq!(data.len(), 7056);
        assert!(matches!(
            CdImage::from_iso(data),
            Err(CdError::UnsupportedFormat(_))
        ));
    }
}
