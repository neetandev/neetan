use std::path::Path;

use device::floppy::{
    d88::{D88Disk, D88MediaType},
    nfd,
};

fn fixture_path(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixture")
        .join(name)
}

fn load_nfd_fixture(name: &str) -> D88Disk {
    let path = fixture_path(name);
    let data = std::fs::read(&path)
        .unwrap_or_else(|error| panic!("Failed to read {}: {error}", path.display()));
    nfd::from_bytes(&data)
        .unwrap_or_else(|error| panic!("Failed to parse {name}: {error}"))
        .0
}

fn load_d88_fixture(name: &str) -> D88Disk {
    let path = fixture_path(name);
    let data = std::fs::read(&path)
        .unwrap_or_else(|error| panic!("Failed to read {}: {error}", path.display()));
    D88Disk::from_bytes(&data).unwrap_or_else(|error| panic!("Failed to parse {name}: {error}"))
}

#[test]
fn parse_blank_2hd_nfd() {
    let disk = load_nfd_fixture("blank_2HD.nfd");
    assert_eq!(disk.media_type, D88MediaType::Disk2HD);
    assert!(!disk.write_protected);
}

#[test]
fn blank_2hd_nfd_track_structure() {
    let disk = load_nfd_fixture("blank_2HD.nfd");

    // 77 cylinders × 2 heads = 154 tracks, 26 sectors each.
    for track in 0..154 {
        assert_eq!(
            disk.sector_count(track),
            26,
            "Track {track} should have 26 sectors"
        );
    }

    // Track 154 should be empty.
    assert_eq!(disk.sector_count(154), 0);
}

#[test]
fn blank_2hd_nfd_track0_geometry() {
    let disk = load_nfd_fixture("blank_2HD.nfd");

    // Track 0 (C=0, H=0): N=0 (128 bytes), FM encoding (mfm_flag=0x40).
    let s = disk.find_sector(0, 0, 1, 0).unwrap();
    assert_eq!(s.data.len(), 128);
    assert_eq!(s.mfm_flag, 0x40);
}

#[test]
fn blank_2hd_nfd_track1_geometry() {
    let disk = load_nfd_fixture("blank_2HD.nfd");

    // Track 1 (C=0, H=1): N=1 (256 bytes), MFM encoding (mfm_flag=0x00).
    let s = disk.find_sector(0, 1, 1, 1).unwrap();
    assert_eq!(s.data.len(), 256);
    assert_eq!(s.mfm_flag, 0x00);
}

#[test]
fn blank_2hd_nfd_sector_lookup() {
    let disk = load_nfd_fixture("blank_2HD.nfd");

    // All 26 sectors on track 0 (R=1..=26, N=0).
    for r in 1..=26u8 {
        let s = disk.find_sector(0, 0, r, 0);
        assert!(s.is_some(), "Sector R={r} not found on track 0");
    }

    // Nonexistent sector.
    assert!(disk.find_sector(0, 0, 27, 0).is_none());
}

#[test]
fn blank_2hd_nfd_inner_track_sectors() {
    let disk = load_nfd_fixture("blank_2HD.nfd");

    // Inner tracks use N=1 (256 bytes).
    let s = disk.find_sector(10, 1, 1, 1).unwrap();
    assert_eq!(s.data.len(), 256);
}

#[test]
fn blank_2hd_nfd_data_content() {
    let disk = load_nfd_fixture("blank_2HD.nfd");

    // Blank disk sectors are filled with 0x40.
    let s = disk.find_sector(0, 0, 1, 0).unwrap();
    assert!(
        s.data.iter().all(|&b| b == 0x40),
        "Blank NFD sectors should be filled with 0x40"
    );
}

#[test]
fn sector_at_index_wraps_nfd() {
    let disk = load_nfd_fixture("blank_2HD.nfd");
    let s0 = disk.sector_at_index(0, 0).unwrap();
    let s26 = disk.sector_at_index(0, 26).unwrap();
    assert_eq!(s0.record, s26.record);
    assert_eq!(s0.cylinder, s26.cylinder);
}

#[test]
fn blank_2hd_nfd_roundtrip_byte_identical() {
    // Parsing then re-serializing the real fixture must reproduce it exactly,
    // pinning the spec header (dwHeadSize = 68,112, byHead = 2) and the 16-byte
    // Reserve3 tail preserved via NfdExtra.
    let path = fixture_path("blank_2HD.nfd");
    let data = std::fs::read(&path).expect("read fixture");
    let (disk, extra) = nfd::from_bytes(&data).expect("parse fixture");
    let serialized = nfd::to_bytes_r0(&disk, Some(&extra));
    assert_eq!(serialized, data);
}

#[test]
fn blank_2hd_nfd_matches_d88() {
    let nfd_disk = load_nfd_fixture("blank_2HD.nfd");
    let d88_disk = load_d88_fixture("blank_2HD.d88");

    assert_eq!(nfd_disk.media_type, d88_disk.media_type);

    // Compare all 154 tracks.
    for track in 0..154 {
        assert_eq!(
            nfd_disk.sector_count(track),
            d88_disk.sector_count(track),
            "Track {track} sector count mismatch"
        );

        for idx in 0..nfd_disk.sector_count(track) {
            let nfd_s = nfd_disk.sector_at_index(track, idx).unwrap();
            let d88_s = d88_disk.sector_at_index(track, idx).unwrap();

            assert_eq!(
                nfd_s.cylinder, d88_s.cylinder,
                "Track {track} sector {idx}: cylinder mismatch"
            );
            assert_eq!(
                nfd_s.head, d88_s.head,
                "Track {track} sector {idx}: head mismatch"
            );
            assert_eq!(
                nfd_s.record, d88_s.record,
                "Track {track} sector {idx}: record mismatch"
            );
            assert_eq!(
                nfd_s.size_code, d88_s.size_code,
                "Track {track} sector {idx}: size_code mismatch"
            );
            assert_eq!(
                nfd_s.data, d88_s.data,
                "Track {track} sector {idx}: data mismatch"
            );
        }
    }
}
