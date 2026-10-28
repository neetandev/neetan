//! Tests for the `neetan create-fdd` subcommand geometries.

use common::temporary::TemporaryFolder;
use device::floppy::{FddType, d88::D88MediaType, load_floppy_image};
use user_interface::create::create_fdd_image;

#[test]
fn creates_144mb_2hd_floppy() {
    let folder = TemporaryFolder::new("create_2hd144");
    let path = folder.path().join("image.d88");
    create_fdd_image(&path, FddType::Hd2Fmt144).expect("create 1.44MB image");

    let data = std::fs::read(&path).expect("read created image");
    // 80 cyl x 2 heads x 18 spt x 512 B = 1,474,560 bytes of sector data.
    let disk = load_floppy_image(&path, &data).expect("parse created image");
    assert_eq!(disk.media_type, D88MediaType::Disk2HD);

    for track in 0..(80 * 2) {
        assert_eq!(
            disk.sector_count(track),
            18,
            "track {track} should have 18 sectors"
        );
    }
    let sector = disk.sector_at_index(0, 0).expect("first sector");
    assert_eq!(sector.size_code, 2, "512-byte sectors -> size code 2");
    assert_eq!(sector.data.len(), 512);
}

#[test]
fn creates_raw_xdf_floppy_for_both_extensions() {
    for extension in ["xdf", "2hd"] {
        let folder = TemporaryFolder::new("create_xdf");
        let path = folder.path().join(format!("image.{extension}"));
        create_fdd_image(&path, FddType::Hd2).expect("create raw XDF image");

        let data = std::fs::read(&path).expect("read created image");
        assert_eq!(data.len(), 1_261_568);
        assert!(data.iter().all(|&byte| byte == 0));

        let disk = load_floppy_image(&path, &data).expect("parse created image");
        assert_eq!(disk.format_name(), "XDF");
        for track in 0..(77 * 2) {
            assert_eq!(disk.sector_count(track), 8);
        }
    }
}

#[test]
fn rejects_raw_xdf_output_for_other_types() {
    let folder = TemporaryFolder::new("create_xdf_bad");
    let path = folder.path().join("image.xdf");
    for fdd_type in [
        FddType::Hd2Fmt144,
        FddType::Dd2,
        FddType::Dd2Fmt720,
        FddType::D2,
    ] {
        assert!(create_fdd_image(&path, fdd_type).is_err());
    }
    assert!(!path.exists());
}

/// Verifies that the raw 1.44 MB and 720 KB images load back by their size with their media type
/// and sector count.
#[test]
fn creates_raw_floppies_that_load_by_size() {
    let cases = [
        (
            "img",
            FddType::Hd2Fmt144,
            D88MediaType::Disk2HD,
            1_474_560,
            18,
            "IMG",
        ),
        (
            "ima",
            FddType::Hd2Fmt144,
            D88MediaType::Disk2HD,
            1_474_560,
            18,
            "IMG",
        ),
        (
            "img",
            FddType::Dd2Fmt720,
            D88MediaType::Disk2DD,
            737_280,
            9,
            "IMG",
        ),
        (
            "dsk",
            FddType::Dd2Fmt720,
            D88MediaType::Disk2DD,
            737_280,
            9,
            "MSX DSK",
        ),
    ];
    for (extension, fdd_type, media_type, size, sectors, format) in cases {
        let folder = TemporaryFolder::new("create_raw_floppy");
        let path = folder.path().join(format!("image.{extension}"));
        create_fdd_image(&path, fdd_type).expect("create raw image");

        let data = std::fs::read(&path).expect("read created image");
        assert_eq!(data.len(), size, "{extension}");
        let disk = load_floppy_image(&path, &data).expect("parse created image");
        assert_eq!(disk.format_name(), format, "{extension}");
        assert_eq!(disk.media_type, media_type, "{extension}");
        for track in 0..(80 * 2) {
            assert_eq!(disk.sector_count(track), sectors, "{extension} {track}");
        }
    }
}

/// Verifies that a 720 KB D88 image loads back with 9 sectors of 512 bytes on every track.
#[test]
fn creates_720kb_d88_floppy() {
    let folder = TemporaryFolder::new("create_2dd720");
    let path = folder.path().join("image.d88");
    create_fdd_image(&path, FddType::Dd2Fmt720).expect("create 720 KB image");

    let data = std::fs::read(&path).expect("read created image");
    let disk = load_floppy_image(&path, &data).expect("parse created image");
    assert_eq!(disk.media_type, D88MediaType::Disk2DD);
    for track in 0..(80 * 2) {
        assert_eq!(disk.sector_count(track), 9);
    }
    let sector = disk.sector_at_index(0, 0).expect("first sector");
    assert_eq!(sector.data.len(), 512);
}

/// Verifies that a second create on the path of an existing image fails and keeps the image.
#[test]
fn a_second_create_on_the_same_path_fails_and_keeps_the_first_file() {
    let folder = TemporaryFolder::new("create_fdd_twice");
    let path = folder.path().join("image.d88");
    create_fdd_image(&path, FddType::D2).expect("create first image");
    let first = std::fs::read(&path).expect("read first image");

    let error = create_fdd_image(&path, FddType::Hd2).expect_err("the file exists");
    assert!(error.to_string().contains("failed to create"), "{error}");
    assert_eq!(std::fs::read(&path).expect("read kept image"), first);
}
