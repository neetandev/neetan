//! Tests for the strict isolated configuration loader.

use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

use automation::CommonConfig;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn unique_dir() -> PathBuf {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("neetan-auto-config-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
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
    assert_eq!(loaded.pc98_roms, Some(config_dir.join("local")));
}
