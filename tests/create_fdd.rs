//! Tests for the `neetan create-fdd` subcommand geometries.

use std::sync::atomic::{AtomicU64, Ordering};

use device::floppy::{d88::D88MediaType, load_floppy_image};
use neetan::{config::FddType, create::create_fdd_image};

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
