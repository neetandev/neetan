//! The atomic replacement of a file.

use std::{
    io::Write,
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
pub(crate) fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
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
    let result = write_synced(&temporary, bytes)
        .and_then(|()| match permissions {
            Some(permissions) => std::fs::set_permissions(&temporary, permissions),
            None => Ok(()),
        })
        .and_then(|()| std::fs::rename(&temporary, &target));
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result?;
    if let Err(error) = sync_folder(&target) {
        warn!(
            "Failed to flush the folder of {}: {error}",
            target.display()
        );
    }
    Ok(())
}

/// Returns the file that a save of `path` writes: the end of a chain of symbolic links.
fn link_target(path: &Path) -> std::io::Result<PathBuf> {
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
    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        format!("{} has too many levels of symbolic links", path.display()),
    ))
}

/// Flushes the folder of `path` to the disk.
#[cfg(unix)]
fn sync_folder(path: &Path) -> std::io::Result<()> {
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
fn sync_folder(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Returns a temporary path in the folder of `path`.
fn temporary_path(path: &Path) -> std::io::Result<PathBuf> {
    let Some(file_name) = path.file_name() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
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

/// Writes `bytes` into a new file at `path` and flushes it to the disk.
fn write_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = std::fs::File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Returns an empty and unique temporary folder.
    fn temporary_folder(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!(
            "neetan_atomic_file_test_{}_{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("folder is created");
        folder
    }

    /// Verifies that a write follows a symbolic link, keeps the permissions and creates the folder.
    #[cfg(unix)]
    #[test]
    fn writes_follow_links_and_keep_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let folder = temporary_folder("link");
        let real = folder.join("real.conf");
        std::fs::write(&real, "machine=X1\n").expect("file is written");
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o640))
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
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Verifies that a write replaces the target and leaves no temporary file.
    #[test]
    fn writes_replace_the_target() {
        let folder = temporary_folder("replace");
        let path = folder.join("game.conf");
        std::fs::write(&path, "machine=X1\n").expect("file is written");
        write_atomically(&path, b"machine = X1TURBO\n").expect("file writes");
        write_atomically(&path, b"machine = X1\n").expect("file writes again");
        assert_eq!(
            std::fs::read_to_string(&path).expect("file reads"),
            "machine = X1\n"
        );
        let names: Vec<_> = std::fs::read_dir(&folder)
            .expect("folder reads")
            .map(|entry| entry.expect("entry reads").file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("game.conf")]);
        let _ = std::fs::remove_dir_all(&folder);
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
