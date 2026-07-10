//! Integration tests for the DIM floppy container.

use std::path::PathBuf;

use device::floppy::{FloppyFormat, MountedFloppy, dim, load_floppy_image};

/// Geometry row used by the tests: (media type, cylinders, sectors, size, N).
const MEDIA_ROWS: [(u8, usize, usize, usize, u8); 5] = [
    (0x00, 77, 8, 1024, 3),
    (0x01, 80, 9, 1024, 3),
    (0x02, 80, 15, 512, 2),
    (0x03, 80, 9, 1024, 3),
    (0x09, 80, 18, 512, 2),
];

/// Builds a DIM image with the given media type and saved-track set.
fn build_dim(media_type: u8, saved_tracks: &[usize], fill: u8) -> Vec<u8> {
    let (_, cylinders, sectors, size, _) = MEDIA_ROWS
        .iter()
        .copied()
        .find(|row| row.0 == media_type)
        .expect("known media type");
    let tracks_per_disk = cylinders * 2;
    let bytes_per_track = sectors * size;

    let mut image = vec![0u8; 256];
    image[0] = media_type;
    image[0xAB..0xB6].copy_from_slice(b"DIFC HEADER");
    image[0xB6] = b' ';
    image[0xB7] = b' ';
    for &track in saved_tracks {
        assert!(track < tracks_per_disk);
        image[1 + track] = 0x01;
    }
    for &track in saved_tracks {
        let base = image.len();
        image.resize(base + bytes_per_track, fill);
        image[base] = track as u8;
    }
    image
}

fn tempfile_with(bytes: &[u8], suffix: &str) -> PathBuf {
    use std::{
        fs::OpenOptions,
        io::Write,
        sync::atomic::{AtomicU64, Ordering},
    };

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let dir = std::env::temp_dir();
    let unique = format!(
        "neetan_dim_test_{}_{}_{}{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        COUNTER.fetch_add(1, Ordering::Relaxed),
        suffix
    );
    let path = dir.join(unique);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .expect("create temp file");
    file.write_all(bytes).expect("write temp file");
    path
}

#[test]
fn all_media_types_parse_with_expected_geometry() {
    for (media_type, cylinders, sectors, size, size_code) in MEDIA_ROWS {
        let tracks_per_disk = cylinders * 2;
        let data = build_dim(media_type, &(0..tracks_per_disk).collect::<Vec<_>>(), 0x00);
        let (disk, header) = dim::from_bytes(&data)
            .unwrap_or_else(|error| panic!("media 0x{media_type:02X}: {error}"));
        assert_eq!(header[0], media_type);
        for track in 0..tracks_per_disk {
            assert_eq!(
                disk.sector_count(track),
                sectors,
                "media 0x{media_type:02X} track {track}"
            );
            let sector = disk.sector_at_index(track, 0).unwrap();
            assert_eq!(sector.data.len(), size);
            assert_eq!(sector.size_code, size_code);
        }
        assert_eq!(disk.sector_count(tracks_per_disk), 0);
    }
}

#[test]
fn header_validation_rejects_bad_images() {
    assert!(matches!(
        dim::from_bytes(&[0u8; 100]),
        Err(dim::DimError::TooShort { actual: 100 })
    ));

    let mut missing_magic = build_dim(0x00, &[0], 0x00);
    missing_magic[0xAB] = b'X';
    assert!(matches!(
        dim::from_bytes(&missing_magic),
        Err(dim::DimError::MissingMagic)
    ));

    let mut bad_media = build_dim(0x00, &[0], 0x00);
    bad_media[0] = 0x04;
    assert!(matches!(
        dim::from_bytes(&bad_media),
        Err(dim::DimError::UnsupportedMediaType(0x04))
    ));

    let mut bad_flag = build_dim(0x00, &[0], 0x00);
    bad_flag[1] = 0x02;
    assert!(matches!(
        dim::from_bytes(&bad_flag),
        Err(dim::DimError::InvalidTrackFlag {
            track: 0,
            value: 0x02
        })
    ));

    // A set flag beyond the 2HD track count (154) is invalid.
    let mut over_flag = build_dim(0x00, &[0], 0x00);
    over_flag[1 + 154] = 0x01;
    assert!(matches!(
        dim::from_bytes(&over_flag),
        Err(dim::DimError::InvalidTrackFlag {
            track: 154,
            value: 0x01
        })
    ));

    let mut bad_over_track = build_dim(0x00, &[0], 0x00);
    bad_over_track[0xFF] = 100;
    assert!(matches!(
        dim::from_bytes(&bad_over_track),
        Err(dim::DimError::InvalidOverTrack(100))
    ));

    let mut good_over_track = build_dim(0x00, &[0], 0x00);
    good_over_track[0xFF] = 154;
    assert!(dim::from_bytes(&good_over_track).is_ok());

    let mut wrong_size = build_dim(0x00, &[0], 0x00);
    wrong_size.push(0);
    assert!(matches!(
        dim::from_bytes(&wrong_size),
        Err(dim::DimError::InvalidSize { .. })
    ));
}

#[test]
fn absent_tracks_read_as_filler_and_have_no_source_offset() {
    let data = build_dim(0x00, &[0], 0xAA);
    let (disk, _) = dim::from_bytes(&data).unwrap();

    let saved = disk.find_sector(0, 0, 1, 3).unwrap();
    assert_eq!(saved.data[0], 0x00);
    assert_eq!(saved.source_offset, Some(256));

    let absent = disk.find_sector(0, 1, 1, 3).unwrap();
    assert!(absent.data.iter().all(|&byte| byte == 0xE5));
    assert_eq!(absent.source_offset, None);
}

#[test]
fn saved_track_source_offsets_skip_absent_tracks() {
    // Tracks 0 and 5 saved: track 5 data starts right after track 0 data.
    let data = build_dim(0x00, &[0, 5], 0x11);
    let (disk, _) = dim::from_bytes(&data).unwrap();
    let bytes_per_track = 8 * 1024;

    let sector = disk.find_sector(2, 1, 1, 3).unwrap();
    assert_eq!(sector.source_offset, Some(256 + bytes_per_track as u64));
}

#[test]
fn two_hs_records_use_offset_numbering() {
    let tracks: Vec<usize> = (0..160).collect();
    let data = build_dim(0x01, &tracks, 0x00);
    let (disk, _) = dim::from_bytes(&data).unwrap();

    // First sector of the disk keeps record 1; the rest of track 0 is 11..=18.
    assert!(disk.find_sector_on_track_index(0, 0, 0, 1, 3).is_some());
    for record in 11..=18 {
        assert!(
            disk.find_sector_on_track_index(0, 0, 0, record, 3)
                .is_some()
        );
    }
    // Other tracks number 10..=18.
    for record in 10..=18 {
        assert!(
            disk.find_sector_on_track_index(1, 0, 1, record, 3)
                .is_some()
        );
    }
    assert!(disk.find_sector_on_track_index(1, 0, 1, 1, 3).is_none());
}

#[test]
fn two_hde_heads_use_bit_seven_numbering() {
    let tracks: Vec<usize> = (0..160).collect();
    let data = build_dim(0x03, &tracks, 0x00);
    let (disk, _) = dim::from_bytes(&data).unwrap();

    // First sector of the disk keeps head 0; the rest use 0x80 | head.
    assert!(disk.find_sector_on_track_index(0, 0, 0, 1, 3).is_some());
    for record in 2..=9 {
        assert!(
            disk.find_sector_on_track_index(0, 0, 0x80, record, 3)
                .is_some()
        );
    }
    for record in 1..=9 {
        assert!(
            disk.find_sector_on_track_index(1, 0, 0x81, record, 3)
                .is_some()
        );
    }
}

#[test]
fn roundtrip_unchanged_preserves_header_and_flags() {
    let mut data = build_dim(0x00, &[0, 3, 7], 0x42);
    // Metadata that must survive: comment area and version byte.
    data[0xC0] = b'N';
    data[0xFE] = 0x19;
    let (disk, header) = dim::from_bytes(&data).unwrap();
    assert_eq!(dim::to_bytes(&disk, &header), data);
}

#[test]
fn writing_an_absent_track_adds_its_flag_on_reemit() {
    let data = build_dim(0x00, &[0], 0x00);
    let (mut disk, header) = dim::from_bytes(&data).unwrap();

    // Write into absent track 3 (C=1, H=1).
    let sector = disk.find_sector_on_track_index_mut(3, 1, 1, 2, 3).unwrap();
    sector.data.fill(0x77);

    let emitted = dim::to_bytes(&disk, &header);
    assert_eq!(emitted[1], 0x01, "track 0 stays saved");
    assert_eq!(emitted[1 + 3], 0x01, "track 3 becomes saved");
    assert_eq!(emitted.len(), 256 + 2 * 8 * 1024);

    let (reparsed, _) = dim::from_bytes(&emitted).unwrap();
    let sector = reparsed.find_sector_on_track_index(3, 1, 1, 2, 3).unwrap();
    assert!(sector.data.iter().all(|&byte| byte == 0x77));
    assert!(sector.source_offset.is_some());
}

#[test]
fn mounted_dim_write_through_hits_saved_tracks() {
    let data = build_dim(0x00, &[0], 0x00);
    let path = tempfile_with(&data, ".dim");

    let image = load_floppy_image(&path, &data).unwrap();
    assert_eq!(image.format, FloppyFormat::Dim);
    let mut mounted = MountedFloppy::new(image, Some(path.clone()));

    let pattern = [0x5Au8; 1024];
    assert!(mounted.write_sector_data(0, 0, 0, 2, 3, &pattern));
    mounted.flush();
    assert!(!mounted.is_dirty());

    let raw = std::fs::read(&path).unwrap();
    assert!(raw[256 + 1024..256 + 2048].iter().all(|&byte| byte == 0x5A));

    drop(mounted);
    std::fs::remove_file(&path).ok();
}

#[test]
fn mounted_dim_flush_extends_file_for_new_tracks_and_refreshes_offsets() {
    let data = build_dim(0x00, &[0, 5], 0x00);
    let path = tempfile_with(&data, ".dim");

    let image = load_floppy_image(&path, &data).unwrap();
    let mut mounted = MountedFloppy::new(image, Some(path.clone()));

    // Write into absent track 2; this cannot write through and marks dirty.
    let pattern = [0x99u8; 1024];
    assert!(mounted.write_sector_data(2, 1, 0, 1, 3, &pattern));
    assert!(mounted.is_dirty());
    mounted.flush();
    assert!(!mounted.is_dirty());

    // The file now stores tracks 0, 2, and 5.
    let raw = std::fs::read(&path).unwrap();
    assert_eq!(raw.len(), 256 + 3 * 8 * 1024);
    assert_eq!(raw[1 + 2], 0x01);

    // After the reparse the shifted track 5 must still write through in place.
    let pattern = [0x33u8; 1024];
    assert!(mounted.write_sector_data(5, 2, 1, 1, 3, &pattern));
    mounted.flush();
    let raw = std::fs::read(&path).unwrap();
    let track5_offset = 256 + 2 * 8 * 1024;
    assert!(
        raw[track5_offset..track5_offset + 1024]
            .iter()
            .all(|&byte| byte == 0x33)
    );

    drop(mounted);
    std::fs::remove_file(&path).ok();
}

#[test]
fn mounted_dim_format_track_updates_flags() {
    let data = build_dim(0x00, &[0], 0x00);
    let path = tempfile_with(&data, ".dim");

    let image = load_floppy_image(&path, &data).unwrap();
    let mut mounted = MountedFloppy::new(image, Some(path.clone()));

    // Format absent track 1 with the standard 2HD layout and non-blank fill.
    let chrn: Vec<(u8, u8, u8, u8)> = (1..=8).map(|record| (0, 1, record, 3)).collect();
    mounted.format_track(1, &chrn, 3, 0x00);
    assert!(!mounted.is_dirty());

    let raw = std::fs::read(&path).unwrap();
    assert_eq!(raw[1 + 1], 0x01, "track 1 flagged after format");
    assert_eq!(raw.len(), 256 + 2 * 8 * 1024);

    drop(mounted);
    std::fs::remove_file(&path).ok();
}

#[test]
fn incompatible_format_stays_dirty_and_leaves_file_unchanged() {
    let data = build_dim(0x00, &[0], 0x00);
    let path = tempfile_with(&data, ".dim");

    let image = load_floppy_image(&path, &data).unwrap();
    let mut mounted = MountedFloppy::new(image, Some(path.clone()));

    // 256-byte sectors cannot be represented by a 2HD DIM.
    mounted.format_track(0, &[(0, 0, 1, 1)], 1, 0x00);
    assert!(mounted.is_dirty());
    assert_eq!(std::fs::read(&path).unwrap(), data);

    drop(mounted);
    std::fs::remove_file(&path).ok();
}

#[test]
fn xdf_and_2hd_extensions_load_as_xdf() {
    let data = vec![0u8; 1_261_568];
    for extension in [".xdf", ".2hd"] {
        let path = tempfile_with(&data, extension);
        let image = load_floppy_image(&path, &data).unwrap();
        assert_eq!(image.format, FloppyFormat::Xdf);
        assert_eq!(image.format_name(), "XDF");
        assert_eq!(image.to_bytes(), data);
        std::fs::remove_file(&path).ok();
    }
}

#[test]
fn xdf_rejects_wrong_size() {
    let data = vec![0u8; 1_261_569];
    let path = tempfile_with(&data, ".xdf");
    assert!(load_floppy_image(&path, &data).is_err());
    std::fs::remove_file(&path).ok();
}

#[test]
fn mounted_xdf_write_through() {
    let mut data = vec![0u8; 1_261_568];
    for (index, byte) in data.iter_mut().enumerate() {
        *byte = (index & 0xFF) as u8;
    }
    let path = tempfile_with(&data, ".xdf");

    let image = load_floppy_image(&path, &data).unwrap();
    let mut mounted = MountedFloppy::new(image, Some(path.clone()));

    let pattern = [0xCCu8; 1024];
    assert!(mounted.write_sector_data(0, 0, 0, 1, 3, &pattern));
    mounted.flush();

    let raw = std::fs::read(&path).unwrap();
    assert!(raw[..1024].iter().all(|&byte| byte == 0xCC));
    assert_eq!(raw[1024], (1024 & 0xFF) as u8);

    drop(mounted);
    std::fs::remove_file(&path).ok();
}
