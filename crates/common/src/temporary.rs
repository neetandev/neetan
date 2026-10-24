//! Temporary folders and files below the system temp folder that remove themselves on drop.
//!
//! The removal runs in `Drop`. Drop also runs while a panic unwinds the stack. A failed test
//! therefore leaves nothing behind either.

use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

/// Prefix of every folder name below the system temp folder.
const FOLDER_PREFIX: &str = "neetan";

/// Counter that makes folder names unique inside one process.
static FOLDER_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A uniquely named folder below `std::env::temp_dir()`.
///
/// Dropping the value removes the folder with all its content.
#[derive(Debug)]
pub struct TemporaryFolder {
    /// Absolute path of the folder.
    path: PathBuf,
}

impl TemporaryFolder {
    /// Creates a new empty folder whose name holds `label`, the process id and a counter.
    ///
    /// Panics when the folder can not be created.
    pub fn new(label: &str) -> Self {
        let base = std::env::temp_dir();
        let process_id = std::process::id();
        loop {
            let counter = FOLDER_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = base.join(format!("{FOLDER_PREFIX}_{label}_{process_id}_{counter}"));
            match std::fs::create_dir(&path) {
                Ok(()) => return Self { path },
                Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                Err(error) => panic!(
                    "failed to create temporary folder {}: {error}",
                    path.display()
                ),
            }
        }
    }

    /// Returns the absolute path of the folder.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Writes `bytes` to the file at the relative path `file_name` inside the folder and returns
    /// its path.
    ///
    /// Creates the missing parent folders of the file. Panics when the file can not be written.
    pub fn write_file(&self, file_name: impl AsRef<Path>, bytes: &[u8]) -> PathBuf {
        let path = self.path.join(file_name);
        let written = match path.parent() {
            Some(parent) => {
                std::fs::create_dir_all(parent).and_then(|()| std::fs::write(&path, bytes))
            }
            None => std::fs::write(&path, bytes),
        };
        if let Err(error) = written {
            panic!("failed to write temporary file {}: {error}", path.display());
        }
        path
    }
}

impl AsRef<Path> for TemporaryFolder {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// A single file inside its own [`TemporaryFolder`].
///
/// Dropping the value removes the file and its folder.
#[derive(Debug)]
pub struct TemporaryFile {
    /// Folder that holds the file.
    _folder: TemporaryFolder,
    /// Absolute path of the file.
    path: PathBuf,
}

impl TemporaryFile {
    /// Creates a new folder from `label` and writes `bytes` to the file `file_name` inside it.
    ///
    /// Panics when the folder or the file can not be created.
    pub fn new(label: &str, file_name: &str, bytes: &[u8]) -> Self {
        let folder = TemporaryFolder::new(label);
        let path = folder.write_file(file_name, bytes);
        Self {
            _folder: folder,
            path,
        }
    }

    /// Returns the absolute path of the file.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl AsRef<Path> for TemporaryFile {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_exists_below_the_system_temp_folder() {
        let folder = TemporaryFolder::new("temporary_exists");
        assert!(folder.path().is_dir());
        assert!(folder.path().starts_with(std::env::temp_dir()));
    }

    #[test]
    fn folders_with_the_same_label_are_unique() {
        let first = TemporaryFolder::new("temporary_unique");
        let second = TemporaryFolder::new("temporary_unique");
        assert_ne!(first.path(), second.path());
        assert!(first.path().is_dir());
        assert!(second.path().is_dir());
    }

    #[test]
    fn folder_is_removed_on_drop() {
        let folder = TemporaryFolder::new("temporary_drop");
        let path = folder.path().to_path_buf();
        drop(folder);
        assert!(!path.exists());
    }

    #[test]
    fn folder_is_removed_after_a_panic() {
        let mut captured = PathBuf::new();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let folder = TemporaryFolder::new("temporary_panic");
            captured = folder.path().to_path_buf();
            panic!("test panic");
        }));
        assert!(result.is_err());
        assert!(!captured.as_os_str().is_empty());
        assert!(!captured.exists());
    }

    #[test]
    fn nested_content_is_removed_on_drop() {
        let folder = TemporaryFolder::new("temporary_nested");
        let nested_folder = folder.path().join("inner");
        std::fs::create_dir(&nested_folder).unwrap();
        let nested_file = nested_folder.join("data.bin");
        std::fs::write(&nested_file, b"content").unwrap();
        let path = folder.path().to_path_buf();
        drop(folder);
        assert!(!nested_file.exists());
        assert!(!path.exists());
    }

    #[test]
    fn written_file_holds_its_bytes() {
        let folder = TemporaryFolder::new("temporary_write");
        let path = folder.write_file("data.bin", b"bytes");
        assert_eq!(path.parent(), Some(folder.path()));
        assert_eq!(std::fs::read(&path).unwrap(), b"bytes");
    }

    #[test]
    fn written_file_creates_its_parent_folders() {
        let folder = TemporaryFolder::new("temporary_write_nested");
        let path = folder.write_file("first/second/data.bin", b"nested");
        assert_eq!(path, folder.path().join("first/second/data.bin"));
        assert_eq!(std::fs::read(&path).unwrap(), b"nested");
    }

    #[test]
    fn file_and_its_folder_are_removed_on_drop() {
        let file = TemporaryFile::new("temporary_file", "image.d88", b"image");
        let path = file.path().to_path_buf();
        let folder = path.parent().unwrap().to_path_buf();
        assert_eq!(std::fs::read(&path).unwrap(), b"image");
        drop(file);
        assert!(!path.exists());
        assert!(!folder.exists());
    }
}
