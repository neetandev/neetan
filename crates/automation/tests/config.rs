//! Tests for the strict isolated configuration loader.

use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

use automation::CommonConfig;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A unique temporary folder. Dropping the value removes the folder.
struct TemporaryFolder(PathBuf);

impl std::ops::Deref for TemporaryFolder {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TemporaryFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Creates an empty and unique temporary folder.
fn unique_dir() -> TemporaryFolder {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("neetan-auto-config-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    TemporaryFolder(dir)
}

fn write(dir: &Path, name: &str, contents: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, contents).unwrap();
    path
}

#[test]
fn accepts_comments_and_blank_lines() {
    let dir = unique_dir();
    let config = write(
        &dir,
        "common.conf",
        "# a comment\n\n; another comment\ntimeout = 42\n",
    );
    let loaded = CommonConfig::load(None, Some(&config)).unwrap();
    assert_eq!(loaded.timeout_seconds, 42);
}

#[test]
fn rejects_malformed_line() {
    let dir = unique_dir();
    let config = write(&dir, "common.conf", "this line has no equals\n");
    let error = CommonConfig::load(None, Some(&config)).unwrap_err();
    assert!(error.contains("malformed line"), "{error}");
}

#[test]
fn rejects_unknown_key() {
    let dir = unique_dir();
    let config = write(&dir, "common.conf", "nonsense-key = value\n");
    let error = CommonConfig::load(None, Some(&config)).unwrap_err();
    assert!(error.contains("unknown key"), "{error}");
}

#[test]
fn rejects_duplicate_key_within_one_file() {
    let dir = unique_dir();
    let config = write(&dir, "common.conf", "pc98-roms = a\npc98-roms = b\n");
    let error = CommonConfig::load(None, Some(&config)).unwrap_err();
    assert!(error.contains("duplicate key"), "{error}");
}

#[test]
fn rejects_invalid_timeout() {
    let dir = unique_dir();
    let config = write(&dir, "common.conf", "timeout = soon\n");
    let error = CommonConfig::load(None, Some(&config)).unwrap_err();
    assert!(error.contains("invalid timeout"), "{error}");
}

#[test]
fn rejects_invalid_guest_time() {
    let dir = unique_dir();
    let config = write(&dir, "common.conf", "guest-time = 2000-13-01T00:00:00\n");
    let error = CommonConfig::load(None, Some(&config)).unwrap_err();
    assert!(error.contains("out of range"), "{error}");
}

#[test]
fn config_layer_overrides_global_without_conflict() {
    let global_dir = unique_dir();
    let config_dir = unique_dir();
    let global = write(&global_dir, "global.conf", "pc98-roms = global\n");
    let config = write(&config_dir, "common.conf", "pc98-roms = local\n");
    let loaded = CommonConfig::load(Some(&global), Some(&config)).unwrap();
    assert_eq!(
        loaded.rom_folders.get("pc98-roms"),
        Some(&config_dir.join("local"))
    );
}

/// Verifies that `roms-folder` fills the unset ROM directories and that an explicit key wins.
#[test]
fn roms_folder_derives_the_unset_rom_directories() {
    let dir = unique_dir();
    std::fs::create_dir_all(dir.join("roms").join("pc98")).unwrap();
    let config = write(
        &dir,
        "common.conf",
        "roms-folder = roms
x1-roms = elsewhere/x1
",
    );
    let loaded = CommonConfig::load(None, Some(&config)).unwrap();
    assert_eq!(loaded.roms_folder, Some(dir.join("roms")));
    assert_eq!(
        loaded.rom_folders.get("pc98-roms"),
        Some(&dir.join("roms").join("pc98"))
    );
    assert_eq!(
        loaded.rom_folders.get("x1-roms"),
        Some(&dir.join("elsewhere/x1"))
    );
    assert_eq!(
        loaded.rom_folders.get("pc88-roms"),
        Some(&dir.join("roms").join("pc88"))
    );
    assert_eq!(loaded.rom_folders.get("at-roms"), None);
    assert_eq!(loaded.rom_folders.get("mt32-roms"), None);
}

/// Verifies that the ROM directories stay unset without `roms-folder`.
#[test]
fn rom_directories_stay_unset_without_roms_folder() {
    let dir = unique_dir();
    std::fs::create_dir_all(dir.join("roms").join("pc98")).unwrap();
    let config = write(&dir, "common.conf", "timeout = 5\n");
    let loaded = CommonConfig::load(None, Some(&config)).unwrap();
    assert!(loaded.rom_folders.is_empty());
}

/// Verifies that every host key outside the ROM folders ends with an error.
#[test]
fn rejects_host_keys_outside_the_rom_folders() {
    for line in [
        "config-folder = config\n",
        "backend = legacy\n",
        "audio-volume = 0.5\n",
    ] {
        let dir = unique_dir();
        let config = write(&dir, "common.conf", line);
        let error = CommonConfig::load(None, Some(&config)).unwrap_err();
        assert!(error.contains("no effect in a headless session"), "{error}");
    }
}

/// Verifies that an empty ROM key is unset and an empty `roms-folder` is an error.
#[test]
fn empty_rom_values_unset_or_fail() {
    let dir = unique_dir();
    std::fs::create_dir_all(dir.join("roms").join("pc98")).unwrap();
    let global = write(&dir, "global.conf", "pc98-roms = explicit\n");
    let config = write(&dir, "common.conf", "roms-folder = roms\npc98-roms =\n");
    let loaded = CommonConfig::load(Some(&global), Some(&config)).unwrap();
    assert_eq!(
        loaded.rom_folders.get("pc98-roms"),
        Some(&dir.join("roms").join("pc98"))
    );

    let config = write(&dir, "empty.conf", "roms-folder =\n");
    let error = CommonConfig::load(None, Some(&config)).unwrap_err();
    assert!(error.contains("'roms-folder' is empty"), "{error}");
}
