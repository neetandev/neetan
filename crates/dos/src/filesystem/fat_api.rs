//! Public façade for FAT12/FAT16 read/write access.
//!
//! Mount the volume with `mount_hdd` (which finds the first active DOS
//! partition automatically) or `mount_fdd` (which mounts at offset 0).
//! Call `flush` explicitly after any write.

use std::fmt;

use crate::{
    DiskIo,
    filesystem::{
        fat::FatVolume,
        fat_dir::{
            self, ATTR_DIRECTORY, ATTR_HIDDEN, ATTR_SYSTEM, ATTR_VOLUME_ID, DirEntry,
            fcb_to_display_name, name_to_fcb,
        },
        fat_file::{self, FileCreateOptions},
        fat_partition::find_partition_offset,
    },
};

/// Errors surfaced by the FAT façade. Wraps the internal `u16` DOS error
/// codes the lower layers use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FatError {
    Io,
    NotFound,
    AlreadyExists,
    NotADirectory,
    IsADirectory,
    DiskFull,
    InvalidName,
    InvalidPath,
    Other(u16),
}

impl fmt::Display for FatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FatError::Io => write!(f, "disk I/O error"),
            FatError::NotFound => write!(f, "not found"),
            FatError::AlreadyExists => write!(f, "already exists"),
            FatError::NotADirectory => write!(f, "not a directory"),
            FatError::IsADirectory => write!(f, "is a directory"),
            FatError::DiskFull => write!(f, "disk full"),
            FatError::InvalidName => write!(f, "invalid filename"),
            FatError::InvalidPath => write!(f, "invalid path"),
            FatError::Other(code) => write!(f, "DOS error {code:#06x}"),
        }
    }
}

impl std::error::Error for FatError {}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MissingComponent {
    Error,
    ReturnNone,
}

fn map_err(code: u16) -> FatError {
    match code {
        0x0002 | 0x0003 => FatError::NotFound,
        0x0005 => FatError::AlreadyExists,
        0x0008 => FatError::DiskFull,
        0x000F => FatError::Io,
        0x0012 => FatError::IsADirectory,
        0x001F => FatError::Io,
        other => FatError::Other(other),
    }
}

/// Metadata describing a single FAT directory entry.
#[derive(Debug, Clone)]
pub struct Metadata {
    /// Display-form name (e.g. `b"FILE.TXT"`).
    pub name: Vec<u8>,
    /// Raw 8.3 FCB form (11 bytes, space-padded).
    pub fcb_name: [u8; 11],
    pub size: u32,
    pub attributes: u8,
    pub time: u16,
    pub date: u16,
}

impl Metadata {
    pub fn is_dir(&self) -> bool {
        self.attributes & ATTR_DIRECTORY != 0
    }

    pub fn is_volume_label(&self) -> bool {
        self.attributes & ATTR_VOLUME_ID != 0
    }
}

/// A mounted FAT12/FAT16 volume.
pub struct FatFs<'d> {
    volume: FatVolume,
    disk: &'d mut dyn DiskIo,
}

impl<'d> FatFs<'d> {
    /// Mounts an HDD volume. Reads the PC-98 partition table to find the
    /// first active DOS partition; falls back to offset 0 if none is found.
    pub fn mount_hdd(disk: &'d mut dyn DiskIo, drive_da: u8) -> Result<Self, FatError> {
        let offset = find_partition_offset(drive_da, disk).map_err(map_err)?;
        let volume = FatVolume::mount(drive_da, offset, disk).map_err(map_err)?;
        Ok(Self { volume, disk })
    }

    /// Mounts an FDD volume (no partition table; partition offset is 0).
    pub fn mount_fdd(disk: &'d mut dyn DiskIo, drive_da: u8) -> Result<Self, FatError> {
        let volume = FatVolume::mount(drive_da, 0, disk).map_err(map_err)?;
        Ok(Self { volume, disk })
    }

    /// Returns metadata for the entry at `dos_path`, or `None` if it does not
    /// exist (including when any intermediate directory is missing). Returns
    /// an error only on malformed paths, on traversing through a
    /// non-directory (`NotADirectory`), or on I/O failures.
    pub fn stat(&mut self, dos_path: &[u8]) -> Result<Option<Metadata>, FatError> {
        let parts = split_dos_path(dos_path)?;
        if parts.is_empty() {
            // Root directory.
            return Ok(Some(Metadata {
                name: Vec::new(),
                fcb_name: [b' '; 11],
                size: 0,
                attributes: ATTR_DIRECTORY,
                time: 0,
                date: 0,
            }));
        }
        match self.try_walk_to_parent(&parts)? {
            Some((parent_cluster, leaf)) => {
                let entry = fat_dir::find_entry(&self.volume, parent_cluster, &leaf, self.disk)
                    .map_err(map_err)?;
                Ok(entry.map(metadata_from_entry))
            }
            None => Ok(None),
        }
    }

    /// Lists the entries of the directory at `dos_path` (or the root, if the
    /// path is empty / "\"). Skips `.` and `..` pseudo-entries.
    pub fn list_dir(&mut self, dos_path: &[u8]) -> Result<Vec<Metadata>, FatError> {
        let dir_cluster = self.resolve_dir_cluster(dos_path)?;
        let pattern = [b'?'; 11];
        let attr_mask = ATTR_HIDDEN | ATTR_SYSTEM | ATTR_DIRECTORY | ATTR_VOLUME_ID;
        let mut out = Vec::new();
        let mut index = 0u16;
        loop {
            let next = fat_dir::find_matching(
                &self.volume,
                dir_cluster,
                &pattern,
                attr_mask,
                index,
                self.disk,
            )
            .map_err(map_err)?;
            match next {
                Some((entry, next_index)) => {
                    if entry.name != *b".          " && entry.name != *b"..         " {
                        out.push(metadata_from_entry(entry));
                    }
                    index = next_index;
                }
                None => return Ok(out),
            }
        }
    }

    /// Reads the entire contents of the file at `dos_path`.
    pub fn read_file(&mut self, dos_path: &[u8]) -> Result<Vec<u8>, FatError> {
        let parts = split_dos_path(dos_path)?;
        if parts.is_empty() {
            return Err(FatError::IsADirectory);
        }
        let (parent_cluster, leaf) = self.walk_to_parent(&parts)?;
        let entry = fat_dir::find_entry(&self.volume, parent_cluster, &leaf, self.disk)
            .map_err(map_err)?
            .ok_or(FatError::NotFound)?;
        if entry.attribute & ATTR_DIRECTORY != 0 {
            return Err(FatError::IsADirectory);
        }
        fat_file::read_all(&self.volume, &entry, self.disk).map_err(map_err)
    }

    /// Creates or replaces the file at `dos_path`. The parent directory must
    /// already exist; call `mkdir_p` on the parent first if necessary.
    pub fn write_file(
        &mut self,
        dos_path: &[u8],
        data: &[u8],
        attributes: u8,
        time: u16,
        date: u16,
    ) -> Result<(), FatError> {
        let parts = split_dos_path(dos_path)?;
        if parts.is_empty() {
            return Err(FatError::InvalidPath);
        }
        let (parent_cluster, leaf) = self.walk_to_parent(&parts)?;
        fat_file::create_or_replace_file(
            &mut self.volume,
            parent_cluster,
            &leaf,
            data,
            FileCreateOptions {
                attributes,
                time,
                date,
            },
            self.disk,
        )
        .map_err(map_err)
    }

    /// Creates the directory at `dos_path` and any missing parent
    /// directories. Idempotent.
    pub fn mkdir_p(&mut self, dos_path: &[u8], time: u16, date: u16) -> Result<(), FatError> {
        let parts = split_dos_path(dos_path)?;
        let mut parent_cluster = 0u16;
        for part in &parts {
            let fcb = name_to_fcb(part);
            match fat_dir::find_entry(&self.volume, parent_cluster, &fcb, self.disk)
                .map_err(map_err)?
            {
                Some(entry) => {
                    if entry.attribute & ATTR_DIRECTORY == 0 {
                        return Err(FatError::NotADirectory);
                    }
                    parent_cluster = entry.start_cluster;
                }
                None => {
                    let created = fat_dir::create_subdirectory(
                        &mut self.volume,
                        parent_cluster,
                        fcb,
                        time,
                        date,
                        self.disk,
                    )
                    .map_err(map_err)?;
                    parent_cluster = created.start_cluster;
                }
            }
        }
        Ok(())
    }

    /// Flushes the cached FAT to disk. Must be called explicitly after writes
    /// to ensure on-disk consistency.
    pub fn flush(&mut self) -> Result<(), FatError> {
        self.volume.flush_fat(self.disk).map_err(map_err)
    }

    fn resolve_dir_cluster(&mut self, dos_path: &[u8]) -> Result<u16, FatError> {
        let parts = split_dos_path(dos_path)?;
        self.walk_dir_parts(&parts, MissingComponent::Error)?
            .ok_or(FatError::NotFound)
    }

    /// Like `walk_to_parent`, but returns `Ok(None)` if any intermediate
    /// component is missing instead of `Err(NotFound)`. Still returns
    /// `Err(NotADirectory)` when an intermediate is a file.
    fn try_walk_to_parent(
        &mut self,
        parts: &[Vec<u8>],
    ) -> Result<Option<(u16, [u8; 11])>, FatError> {
        Ok(self
            .walk_dir_parts(&parts[..parts.len() - 1], MissingComponent::ReturnNone)?
            .map(|cluster| (cluster, name_to_fcb(parts.last().unwrap()))))
    }

    /// Walks all but the last component, returning `(parent_cluster, leaf_fcb)`.
    /// Requires `parts` to be non-empty.
    fn walk_to_parent(&mut self, parts: &[Vec<u8>]) -> Result<(u16, [u8; 11]), FatError> {
        let cluster = self
            .walk_dir_parts(&parts[..parts.len() - 1], MissingComponent::Error)?
            .ok_or(FatError::NotFound)?;
        Ok((cluster, name_to_fcb(parts.last().unwrap())))
    }

    fn walk_dir_parts(
        &mut self,
        parts: &[Vec<u8>],
        missing: MissingComponent,
    ) -> Result<Option<u16>, FatError> {
        let mut cluster = 0u16;
        for part in parts {
            let fcb = name_to_fcb(part);
            let entry =
                fat_dir::find_entry(&self.volume, cluster, &fcb, self.disk).map_err(map_err)?;
            let Some(entry) = entry else {
                return match missing {
                    MissingComponent::Error => Err(FatError::NotFound),
                    MissingComponent::ReturnNone => Ok(None),
                };
            };
            if entry.attribute & ATTR_DIRECTORY != 0 {
                cluster = entry.start_cluster;
            } else {
                return Err(FatError::NotADirectory);
            }
        }
        Ok(Some(cluster))
    }
}

fn metadata_from_entry(entry: DirEntry) -> Metadata {
    Metadata {
        name: fcb_to_display_name(&entry.name),
        fcb_name: entry.name,
        size: entry.file_size,
        attributes: entry.attribute,
        time: entry.time,
        date: entry.date,
    }
}

/// Splits a DOS path into components. Accepts an optional leading drive
/// letter `[A-Z]:`, and both `\` and `/` as separators. The drive letter is
/// discarded (the caller pre-selected the volume). A leading separator is
/// allowed but not required. All paths are treated as absolute from the
/// volume root.
fn split_dos_path(path: &[u8]) -> Result<Vec<Vec<u8>>, FatError> {
    let mut pos = 0;
    if path.len() >= 2 && path[1] == b':' {
        let letter = path[0].to_ascii_uppercase();
        if !letter.is_ascii_uppercase() {
            return Err(FatError::InvalidPath);
        }
        pos = 2;
    }
    let mut components = Vec::new();
    let mut current = Vec::new();
    while pos < path.len() {
        let byte = path[pos];
        if byte == 0 {
            break;
        }
        if byte == b'\\' || byte == b'/' {
            if !current.is_empty() {
                components.push(std::mem::take(&mut current));
            }
            pos += 1;
            continue;
        }
        current.push(byte);
        pos += 1;
    }
    if !current.is_empty() {
        components.push(current);
    }
    Ok(components)
}
