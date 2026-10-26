//! The atomic replacement of a file.

use std::{
    fs::{Metadata, Permissions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

use common::warn;

/// The extension of the temporary file of a write.
const TEMPORARY_EXTENSION: &str = "tmp";
/// The most symbolic links that a write follows to its real target.
const MAXIMUM_LINK_DEPTH: usize = 40;
/// Counter that keeps the temporary files of concurrent writes apart.
static TEMPORARY_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Replaces the file at `path` with `bytes` in one step.
///
/// A symbolic link at `path` is followed to its real target. The bytes go
/// into a temporary file in the folder of the target, with the permissions of
/// an existing target. A rename then replaces the target. A rename on one
/// file system is atomic, and it replaces an existing target on Windows as
/// well. The folder of the target is created when it is absent. On Unix the
/// folder is flushed after the rename as a best effort. A failed flush ends
/// with a warning. A failed write removes the temporary file.
///
/// Returns the metadata of the written file. The open temporary file gives
/// it after the sync, before the rename puts the file in place.
pub(crate) fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<Metadata> {
    let target = link_target(path)?;
    if let Some(folder) = target
        .parent()
        .filter(|folder| !folder.as_os_str().is_empty())
    {
        std::fs::create_dir_all(folder)?;
    }
    let temporary = temporary_path(&target)?;
    let permissions = std::fs::metadata(&target)
        .ok()
        .map(|metadata| metadata.permissions());
    let result = write_synced(&temporary, bytes, permissions).and_then(|metadata| {
        std::fs::rename(&temporary, &target)?;
        Ok(metadata)
    });
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    let metadata = result?;
    if let Err(error) = sync_folder(&target) {
        warn!(
            "Failed to flush the folder of {}: {error}",
            target.display()
        );
    }
    Ok(metadata)
}

/// Returns the file that a save of `path` writes: the end of a chain of symbolic links.
fn link_target(path: &Path) -> io::Result<PathBuf> {
    let mut target = path.to_path_buf();
    for _ in 0..MAXIMUM_LINK_DEPTH {
        let is_link = std::fs::symlink_metadata(&target)
            .is_ok_and(|metadata| metadata.file_type().is_symlink());
        if !is_link {
            return Ok(target);
        }
        let link = std::fs::read_link(&target)?;
        target = match target.parent() {
            Some(folder) if link.is_relative() => folder.join(link),
            Some(_) | None => link,
        };
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("{} has too many levels of symbolic links", path.display()),
    ))
}

/// Flushes the folder of `path` to the disk.
#[cfg(unix)]
fn sync_folder(path: &Path) -> io::Result<()> {
    match path
        .parent()
        .filter(|folder| !folder.as_os_str().is_empty())
    {
        Some(folder) => std::fs::File::open(folder)?.sync_all(),
        None => std::fs::File::open(".")?.sync_all(),
    }
}

/// Flushes the folder of `path` to the disk. Other systems commit the rename on their own.
#[cfg(not(unix))]
fn sync_folder(_path: &Path) -> io::Result<()> {
    Ok(())
}

/// Returns a temporary path in the folder of `path`.
fn temporary_path(path: &Path) -> io::Result<PathBuf> {
    let Some(file_name) = path.file_name() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} has no file name", path.display()),
        ));
    };
    let mut temporary_name = std::ffi::OsString::from(".");
    temporary_name.push(file_name);
    temporary_name.push(format!(
        ".{}.{}.{TEMPORARY_EXTENSION}",
        std::process::id(),
        TEMPORARY_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    Ok(path.with_file_name(temporary_name))
}

/// Writes `bytes` into a new file at `path`, gives it `permissions` when
/// there are some and flushes it to the disk. Returns the metadata of the
/// open file after the flush.
fn write_synced(
    path: &Path,
    bytes: &[u8],
    permissions: Option<Permissions>,
) -> io::Result<Metadata> {
    let mut file = std::fs::File::create(path)?;
    file.write_all(bytes)?;
    if let Some(permissions) = permissions {
        file.set_permissions(permissions)?;
    }
    file.sync_all()?;
    file.metadata()
}

#[cfg(test)]
mod tests {
    use common::temporary::TemporaryFolder;

    use super::*;

    /// Verifies that a write follows a symbolic link, keeps the permissions and creates the folder.
    #[cfg(unix)]
    #[test]
    fn writes_follow_links_and_keep_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let temporary = TemporaryFolder::new("atomic_file_test_link");
        let folder = temporary.path();
        let real = folder.join("real.conf");
        std::fs::write(&real, "machine=X1\n").expect("file is written");
        std::fs::set_permissions(&real, Permissions::from_mode(0o640))
            .expect("permissions are set");
        let link = folder.join("link.conf");
        std::os::unix::fs::symlink("real.conf", &link).expect("link is created");

        write_atomically(&link, b"machine = X1TURBO\n").expect("link writes");
        assert!(
            std::fs::symlink_metadata(&link)
                .expect("link exists")
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            std::fs::read_to_string(&real).expect("file reads"),
            "machine = X1TURBO\n"
        );
        let mode = std::fs::metadata(&real)
            .expect("file exists")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o640);

        let nested = folder.join("new").join("folder").join("game.conf");
        write_atomically(&nested, b"bios = on\n").expect("nested file writes");
        assert_eq!(
            std::fs::read_to_string(&nested).expect("file reads"),
            "bios = on\n"
        );
    }

    /// Verifies that a write replaces the target, returns the metadata of the placed file and leaves no temporary file.
    #[test]
    fn writes_replace_the_target() {
        let temporary = TemporaryFolder::new("atomic_file_test_replace");
        let folder = temporary.path();
        let path = folder.join("game.conf");
        std::fs::write(&path, "machine=X1\n").expect("file is written");
        write_atomically(&path, b"machine = X1TURBO\n").expect("file writes");
        let written = write_atomically(&path, b"machine = X1\n").expect("file writes again");
        let placed = std::fs::metadata(&path).expect("file exists");
        assert_eq!(written.len(), b"machine = X1\n".len() as u64);
        assert_eq!(written.len(), placed.len());
        assert_eq!(
            written.modified().expect("the file system keeps the time"),
            placed.modified().expect("the file system keeps the time")
        );
        assert_eq!(
            std::fs::read_to_string(&path).expect("file reads"),
            "machine = X1\n"
        );
        let names: Vec<_> = std::fs::read_dir(folder)
            .expect("folder reads")
            .map(|entry| entry.expect("entry reads").file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("game.conf")]);
    }

    /// Verifies that the temporary names of two writes differ.
    #[test]
    fn temporary_names_differ_for_each_write() {
        let path = Path::new("/games/edge.conf");
        let first = temporary_path(path).expect("path has a file name");
        let second = temporary_path(path).expect("path has a file name");
        assert_ne!(first, second);
        assert_eq!(first.parent(), path.parent());
        assert!(
            first
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(".edge.conf.") && name.ends_with(".tmp"))
        );
    }
}
