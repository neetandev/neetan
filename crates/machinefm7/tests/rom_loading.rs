//! ROM-loading tests that run without any real ROM present.
//!
//! Content matching is by BLAKE3 digest, so a synthetic file cannot satisfy a
//! slot; these tests cover the scan and error paths. The positive path (real
//! dumps resolving every slot, and the optional-kanji distinction) lives in
//! `temp_rom_hashes.rs`.

use std::{fs, path::PathBuf};

use machinefm7::{Fm7Model, RomError, load_rom_set};

fn temp_dir(tag: &str) -> PathBuf {
    let unique = format!(
        "neetan_fm7_rom_{}_{}_{}",
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
    // Files with the right sizes but bogus content: nothing matches any digest.
    write_file(&dir, "fbasic.bin", 31744, 0xFF);
    write_file(&dir, "boot_bas.bin", 512, 0xFF);
    write_file(&dir, "boot_dos.bin", 512, 0xAA);
    write_file(&dir, "subsys_c.bin", 10240, 0xFF);
    write_file(&dir, "kanji.bin", 131072, 0xFF);

    let result = load_rom_set(Fm7Model::Fm7, &dir);
    match result {
        Err(RomError::Missing { label, .. }) => assert_eq!(label, "f-basic 3.0"),
        Err(other) => panic!("expected Missing f-basic, got {other:?}"),
        Ok(_) => panic!("expected Missing f-basic, got Ok"),
    }

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn stray_unknown_size_files_are_ignored() {
    let dir = temp_dir("stray");
    // Only stray files of unknown sizes: the scan skips them, and the first
    // required slot is reported missing rather than raising a read error.
    write_file(&dir, "readme.txt", 42, 0x00);
    write_file(&dir, "junk.dat", 1234, 0x00);

    let result = load_rom_set(Fm7Model::Fm7, &dir);
    assert!(matches!(result, Err(RomError::Missing { .. })));

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn av_reports_missing_on_bogus_set() {
    let dir = temp_dir("av");
    write_file(&dir, "fbasic.bin", 31744, 0xFF);
    write_file(&dir, "initiate.bin", 8192, 0xFF);
    write_file(&dir, "kanji.bin", 131072, 0xFF);

    let result = load_rom_set(Fm7Model::Fm77Av, &dir);
    assert!(matches!(result, Err(RomError::Missing { .. })));

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn missing_directory_reports_read_error() {
    let result = load_rom_set(Fm7Model::Fm7, std::path::Path::new("/nonexistent/fm7/roms"));
    assert!(matches!(result, Err(RomError::Read { .. })));
}
