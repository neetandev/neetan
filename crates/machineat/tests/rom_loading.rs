//! ROM-loading tests.
//!
//! Content matching is by BLAKE3 digest, so a synthetic file cannot satisfy a
//! slot. These tests cover the scan and error paths of `load_rom_set` without
//! any copyrighted image.

use std::{fs, path::PathBuf};

use machineat::{RomError, load_rom_set};

/// System BIOS image size the loader accepts (64 KiB).
const SYSTEM_BIOS_SIZE: usize = 0x1_0000;
/// VGA BIOS image size the loader accepts (32 KiB).
const VGA_BIOS_SIZE: usize = 0x8000;

fn temp_dir(tag: &str) -> PathBuf {
    let unique = format!(
        "neetan_at_rom_{}_{}_{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let path = std::env::temp_dir().join(unique);
    fs::create_dir_all(&path).unwrap();
    path
}

fn write_file(dir: &std::path::Path, name: &str, size: usize, fill: u8) {
    fs::write(dir.join(name), vec![fill; size]).unwrap();
}

#[test]
fn wrong_content_of_correct_size_reports_missing() {
    let dir = temp_dir("wrong");
    // Right-sized but bogus images: neither digest matches, so the system BIOS
    // slot is reported missing.
    write_file(&dir, "chips_1.ami", SYSTEM_BIOS_SIZE, 0xFF);
    write_file(&dir, "et4000.bin", VGA_BIOS_SIZE, 0xAA);

    match load_rom_set(&dir) {
        Err(RomError::Missing { label, .. }) => assert_eq!(label, "system-bios"),
        Err(other) => panic!("expected Missing system-bios, got {other:?}"),
        Ok(_) => panic!("expected Missing system-bios, got Ok"),
    }

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn stray_unknown_size_files_are_ignored() {
    let dir = temp_dir("stray");
    // Files whose sizes are not a known ROM size are skipped by the scan, so
    // the first required slot is reported missing rather than a read error.
    write_file(&dir, "readme.txt", 42, 0x00);
    write_file(&dir, "junk.dat", 1234, 0x00);

    assert!(matches!(load_rom_set(&dir), Err(RomError::Missing { .. })));

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn missing_directory_reports_read_error() {
    let result = load_rom_set(std::path::Path::new("/nonexistent/at/roms"));
    assert!(matches!(result, Err(RomError::Read { .. })));
}
