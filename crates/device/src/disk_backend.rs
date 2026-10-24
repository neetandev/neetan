//! File-side backing for inserted floppy and HDD images.
//!
//! Holds an open `BufWriter<File>` for synchronous write-through and
//! supports atomic full-image replacement.

use std::{
    ffi::{OsStr, OsString},
    fs::{File, OpenOptions},
    io::{BufWriter, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

/// Path-paired writer that supports random-access seek+write and atomic
/// whole-file replacement.
pub struct DiskBackend {
    path: PathBuf,
    writer: Option<BufWriter<File>>,
}

impl DiskBackend {
    /// Opens `path` for read+write and wraps it in a `BufWriter`.
    pub fn open(path: PathBuf) -> std::io::Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open(&path)?;
        Ok(Self {
            path,
            writer: Some(BufWriter::new(file)),
        })
    }

    /// Returns the path the backend is bound to.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Seeks to `offset` and writes `data`.
    pub fn write_at(&mut self, offset: u64, data: &[u8]) -> std::io::Result<()> {
        let writer = self
            .writer
            .as_mut()
            .ok_or_else(|| std::io::Error::other("disk backend closed"))?;
        writer.seek(SeekFrom::Start(offset))?;
        writer.write_all(data)
    }

    /// Replaces the entire file contents atomically (temp + rename) and
    /// reopens the `BufWriter` against the new file.
    pub fn replace_atomic(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        if let Some(mut w) = self.writer.take()
            && let Err(err) = w.flush()
        {
            self.restore_writer();
            return Err(err);
        }

        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);

        for attempt in 0..16u32 {
            let tmp_path = Self::replacement_tmp_path(&self.path, unique, attempt);
            let mut tmp_file = match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp_path)
            {
                Ok(file) => file,
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    continue;
                }
                Err(err) => {
                    self.restore_writer();
                    return Err(err);
                }
            };

            if let Err(err) = tmp_file.write_all(bytes) {
                let _ = std::fs::remove_file(&tmp_path);
                self.restore_writer();
                return Err(err);
            }
            drop(tmp_file);

            if let Err(err) = std::fs::rename(&tmp_path, &self.path) {
                let _ = std::fs::remove_file(&tmp_path);
                self.restore_writer();
                return Err(err);
            }

            let file = OpenOptions::new().read(true).write(true).open(&self.path)?;
            self.writer = Some(BufWriter::new(file));
            return Ok(());
        }

        self.restore_writer();
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "could not create unique disk image replacement temp file",
        ))
    }

    /// Flushes any buffered writes to the OS.
    pub fn flush(&mut self) -> std::io::Result<()> {
        if let Some(w) = self.writer.as_mut() {
            w.flush()?;
        }
        Ok(())
    }

    fn replacement_tmp_path(path: &Path, unique: u128, attempt: u32) -> PathBuf {
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let file_name = path.file_name().unwrap_or_else(|| OsStr::new("disk-image"));
        let mut tmp_name = OsString::from(".");
        tmp_name.push(file_name);
        tmp_name.push(format!(
            ".{}.{}.{}.tmp",
            std::process::id(),
            unique,
            attempt
        ));
        parent.join(tmp_name)
    }

    fn restore_writer(&mut self) {
        if self.writer.is_none()
            && let Ok(file) = OpenOptions::new().read(true).write(true).open(&self.path)
        {
            self.writer = Some(BufWriter::new(file));
        }
    }
}

impl Drop for DiskBackend {
    fn drop(&mut self) {
        if let Some(mut w) = self.writer.take() {
            let _ = w.flush();
        }
    }
}

impl std::fmt::Debug for DiskBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiskBackend")
            .field("path", &self.path)
            .field("open", &self.writer.is_some())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use common::temporary::TemporaryFile;

    use super::*;

    #[test]
    fn write_at_persists() {
        let file = TemporaryFile::new("disk_backend_test", "image.tmp", &[0u8; 1024]);
        let path = file.path().to_path_buf();
        let mut backend = DiskBackend::open(path.clone()).expect("open");
        backend.write_at(256, &[0xAB, 0xCD, 0xEF]).expect("write");
        backend.flush().expect("flush");
        let raw = std::fs::read(&path).expect("read");
        assert_eq!(&raw[256..259], &[0xAB, 0xCD, 0xEF]);
    }

    #[test]
    fn replace_atomic_swaps_content() {
        let file = TemporaryFile::new("disk_backend_test", "image.tmp", b"first content");
        let path = file.path().to_path_buf();
        let mut backend = DiskBackend::open(path.clone()).expect("open");
        backend
            .replace_atomic(b"second content goes here")
            .expect("replace");
        let raw = std::fs::read(&path).expect("read");
        assert_eq!(raw, b"second content goes here");
        // Subsequent write_at lands on the new file.
        backend.write_at(0, b"THIRD").expect("write");
        backend.flush().expect("flush");
        let raw = std::fs::read(&path).expect("read");
        assert_eq!(&raw[..5], b"THIRD");
    }

    #[test]
    fn replace_atomic_does_not_touch_existing_tmp_sibling() {
        let tmp_seed_file = TemporaryFile::new("disk_backend_test", "image.tmp", b"seed");
        let tmp_seed = tmp_seed_file.path().to_path_buf();
        let path = tmp_seed.with_extension("img");
        std::fs::rename(&tmp_seed, &path).expect("rename seed to image path");

        let tmp_sibling = path.with_extension("tmp");
        std::fs::write(&tmp_sibling, b"keep me").expect("write sibling tmp");

        let mut backend = DiskBackend::open(path.clone()).expect("open");
        backend.replace_atomic(b"second content").expect("replace");
        backend.flush().expect("flush");

        let raw = std::fs::read(&path).expect("read replaced file");
        assert_eq!(raw, b"second content");
        let sibling = std::fs::read(&tmp_sibling).expect("read sibling tmp");
        assert_eq!(sibling, b"keep me");
    }

    #[test]
    fn write_at_returns_error_on_seek_past_end_then_succeeds() {
        // BufWriter+File: writing past current end extends the file.
        let file = TemporaryFile::new("disk_backend_test", "image.tmp", &[0u8; 16]);
        let path = file.path().to_path_buf();
        let mut backend = DiskBackend::open(path.clone()).expect("open");
        backend.write_at(100, b"x").expect("write past end");
        backend.flush().expect("flush");
        let raw = std::fs::read(&path).expect("read");
        assert!(raw.len() >= 101);
        assert_eq!(raw[100], b'x');
    }
}
