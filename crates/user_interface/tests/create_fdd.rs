//! Tests for the `neetan create-fdd` subcommand geometries.

use std::sync::atomic::{AtomicU64, Ordering};

use device::floppy::{d88::D88MediaType, load_floppy_image};
use user_interface::{config::FddType, create::create_fdd_image};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_path(label: &str) -> std::path::PathBuf {
    let pid = std::process::id();
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("neetan_create_{label}_{pid}_{counter}.d88"))
}

#[test]
fn creates_144mb_2hd_floppy() {
    let path = temp_path("2hd144");
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

    std::fs::remove_file(&path).ok();
}

#[test]
fn creates_raw_xdf_floppy_for_both_extensions() {
    for extension in ["xdf", "2hd"] {
        let pid = std::process::id();
        let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("neetan_create_xdf_{pid}_{counter}.{extension}"));
        create_fdd_image(&path, FddType::Hd2).expect("create raw XDF image");

        let data = std::fs::read(&path).expect("read created image");
        assert_eq!(data.len(), 1_261_568);
        assert!(data.iter().all(|&byte| byte == 0));

        let disk = load_floppy_image(&path, &data).expect("parse created image");
        assert_eq!(disk.format_name(), "XDF");
        for track in 0..(77 * 2) {
            assert_eq!(disk.sector_count(track), 8);
        }

        std::fs::remove_file(&path).ok();
    }
}

#[test]
fn rejects_raw_xdf_output_for_other_types() {
    let pid = std::process::id();
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("neetan_create_xdf_bad_{pid}_{counter}.xdf"));
    for fdd_type in [FddType::Hd2Fmt144, FddType::Dd2, FddType::D2] {
        assert!(create_fdd_image(&path, fdd_type).is_err());
    }
    assert!(!path.exists());
}
