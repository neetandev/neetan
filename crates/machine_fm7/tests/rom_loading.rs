//! ROM-loading tests that run without any real ROM present.
//!
//! Content matching is by BLAKE3 digest, so a synthetic file cannot satisfy a
//! slot; these tests cover the scan and error paths. The positive path (real
//! dumps resolving every slot, and the optional-kanji distinction) lives in
//! `temp_rom_hashes.rs`.

use std::fs;

use common::temporary::TemporaryFolder;
use machine_fm7::{Fm7Model, RomError, load_rom_set};

fn write_file(dir: &std::path::Path, name: &str, size: usize, fill: u8) {
    fs::write(dir.join(name), vec![fill; size]).unwrap();
}

#[test]
fn wrong_content_of_correct_size_reports_missing() {
    let folder = TemporaryFolder::new("fm7_rom_wrong");
    let dir = folder.path();
    // Files with the right sizes but bogus content: nothing matches any digest.
    write_file(dir, "fbasic.bin", 31744, 0xFF);
    write_file(dir, "boot_bas.bin", 512, 0xFF);
    write_file(dir, "boot_dos.bin", 512, 0xAA);
    write_file(dir, "subsys_c.bin", 10240, 0xFF);
    write_file(dir, "kanji.bin", 131072, 0xFF);

    let result = load_rom_set(Fm7Model::Fm7, dir);
    match result {
        Err(RomError::Missing { label, .. }) => assert_eq!(label, "f-basic 3.0"),
        Err(other) => panic!("expected Missing f-basic, got {other:?}"),
        Ok(_) => panic!("expected Missing f-basic, got Ok"),
    }
}

#[test]
fn stray_unknown_size_files_are_ignored() {
    let folder = TemporaryFolder::new("fm7_rom_stray");
    let dir = folder.path();
    // Only stray files of unknown sizes: the scan skips them, and the first
    // required slot is reported missing rather than raising a read error.
    write_file(dir, "readme.txt", 42, 0x00);
    write_file(dir, "junk.dat", 1234, 0x00);

    let result = load_rom_set(Fm7Model::Fm7, dir);
    assert!(matches!(result, Err(RomError::Missing { .. })));
}

#[test]
fn av_reports_missing_on_bogus_set() {
    let folder = TemporaryFolder::new("fm7_rom_av");
    let dir = folder.path();
    write_file(dir, "fbasic.bin", 31744, 0xFF);
    write_file(dir, "initiate.bin", 8192, 0xFF);
    write_file(dir, "kanji.bin", 131072, 0xFF);

    let result = load_rom_set(Fm7Model::Fm77Av, dir);
    assert!(matches!(result, Err(RomError::Missing { .. })));
}

#[test]
fn missing_directory_reports_read_error() {
    let result = load_rom_set(Fm7Model::Fm7, std::path::Path::new("/nonexistent/fm7/roms"));
    assert!(matches!(result, Err(RomError::Read { .. })));
}
