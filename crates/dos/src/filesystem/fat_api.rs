//! Public facade for FAT12/FAT16 read/write access.
//!
//! Mount the volume of a drive entry with `mount`. The partition scheme of the
//! entry finds the volume start and its geometry names the BPB source.
//! Call `flush` explicitly after any write.

use std::fmt;

use crate::{
    DiskIo,
    drive_map::DriveEntry,
    filesystem::{
        fat::FatVolume,
        fat_dir::{
            self, ATTR_DIRECTORY, ATTR_HIDDEN, ATTR_SYSTEM, ATTR_VOLUME_ID, DirEntry,
            fcb_to_display_name, name_to_fcb,
        },
        fat_file::{self, FileCreateOptions},
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
    /// The floppy holds no FAT file system: no BPB and no FAT ID.
    UnknownMedia,
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
            FatError::UnknownMedia => write!(f, "no FAT file system"),
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

/// Lowest FAT ID byte in the first FAT entry.
const LOWEST_FAT_ID: u8 = 0xF0;

/// Metadata describing a single FAT directory entry.
#[derive(Debug, Clone)]
pub struct Metadata {
    /// Display-form name (e.g. `b"FILE.TXT"`).
    pub name: Vec<u8>,
    /// Raw name bytes of the entry: the 18.3 name on a Human68k volume,
    /// otherwise equal to `name`.
    pub full_name: Vec<u8>,
    /// Raw 8.3 FCB form (11 bytes, space-padded).
    pub fcb_name: [u8; 11],
    pub size: u32,
    pub attributes: u8,
    pub time: u16,
    pub date: u16,
    /// First cluster of the entry. The root directory has cluster 0.
    pub(crate) start_cluster: u16,
    /// True for the metadata of the root directory.
    pub(crate) root: bool,
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
    /// Mounts the volume of a drive entry: finds the volume start with the
    /// partition scheme and takes the BPB from the geometry of the entry.
    pub fn mount(disk: &'d mut dyn DiskIo, entry: &DriveEntry) -> Result<Self, FatError> {
        let device_id = entry.device.disk_device_id().ok_or(FatError::Io)?;
        let geometry = entry.geometry.ok_or(FatError::Io)?;
        let offset = entry
            .partition_scheme
            .volume_start(device_id, disk)
            .map_err(map_err)?;
        let volume = match FatVolume::mount(device_id, offset, &geometry, disk) {
            Ok(volume) => volume,
            Err(code) => {
                if entry.device.is_floppy() && holds_no_fat(device_id, offset, disk) {
                    return Err(FatError::UnknownMedia);
                }
                return Err(map_err(code));
            }
        };
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
                full_name: Vec::new(),
                fcb_name: [b' '; 11],
                size: 0,
                attributes: ATTR_DIRECTORY,
                time: 0,
                date: 0,
                start_cluster: 0,
                root: true,
            }));
        }
        match self.try_walk_to_parent(&parts)? {
            Some((parent_cluster, leaf)) => {
                let entry = fat_dir::find_entry(&self.volume, parent_cluster, &leaf, self.disk)
                    .map_err(map_err)?;
                Ok(entry.map(|entry| self.metadata_from_entry(entry)))
            }
            None => Ok(None),
        }
    }

    /// Lists the entries of the directory at `dos_path` (or the root, if the
    /// path is empty / "\"). Skips `.` and `..` pseudo-entries.
    pub fn list_dir(&mut self, dos_path: &[u8]) -> Result<Vec<Metadata>, FatError> {
        let dir_cluster = self.resolve_dir_cluster(dos_path)?;
        self.list_cluster_dir(dir_cluster)
    }

    /// Lists the entries of the directory that `directory` describes. Follows
    /// its start cluster, resolves no name and skips `.` and `..`. A
    /// subdirectory whose start cluster is no data cluster fails with `Io`.
    pub fn list_entry_dir(&mut self, directory: &Metadata) -> Result<Vec<Metadata>, FatError> {
        if !directory.is_dir() {
            return Err(FatError::NotADirectory);
        }
        if !directory.root && !self.volume.is_data_cluster(directory.start_cluster) {
            return Err(FatError::Io);
        }
        self.list_cluster_dir(directory.start_cluster)
    }

    /// Reads the entire contents of the file that `file` describes. Follows
    /// its start cluster and resolves no name. A file with data whose start
    /// cluster is no data cluster fails with `Io`.
    pub fn read_entry(&mut self, file: &Metadata) -> Result<Vec<u8>, FatError> {
        if file.is_dir() {
            return Err(FatError::IsADirectory);
        }
        if file.size > 0 && !self.volume.is_data_cluster(file.start_cluster) {
            return Err(FatError::Io);
        }
        fat_file::read_chain(&self.volume, file.start_cluster, file.size, self.disk)
            .map_err(map_err)
    }

    /// Lists the entries of the directory that starts at `dir_cluster`.
    /// Cluster 0 is the root directory.
    fn list_cluster_dir(&mut self, dir_cluster: u16) -> Result<Vec<Metadata>, FatError> {
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
                        out.push(self.metadata_from_entry(entry));
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

    /// Builds the metadata of a directory entry of the volume.
    fn metadata_from_entry(&self, entry: DirEntry) -> Metadata {
        let name = fcb_to_display_name(&entry.name);
        let full_name = if self.volume.is_human68k() {
            entry.human68k_display_name()
        } else {
            name.clone()
        };
        Metadata {
            name,
            full_name,
            fcb_name: entry.name,
            size: entry.file_size,
            attributes: entry.attribute,
            time: entry.time,
            date: entry.date,
            start_cluster: entry.start_cluster,
            root: false,
        }
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

/// Returns true when a floppy holds no FAT file system: its boot record has
/// no usable BPB and the sector after it starts with no FAT ID. A failed
/// read returns false.
fn holds_no_fat(device_id: u8, volume_start: u32, disk: &mut dyn DiskIo) -> bool {
    let Some(physical_sector_size) = disk.sector_size(device_id) else {
        return false;
    };
    let Some(boot_record) =
        FatVolume::read_boot_record(device_id, volume_start, physical_sector_size, disk)
    else {
        return false;
    };
    if FatVolume::boot_record_bpb(&boot_record, physical_sector_size).is_some() {
        return false;
    }
    let Ok(fat_sector) = disk.read_sectors(device_id, volume_start + 1, 1) else {
        return false;
    };
    !matches!(
        fat_sector.as_slice(),
        [fat_id, 0xFF, 0xFF, ..] if *fat_id >= LOWEST_FAT_ID
    )
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        drive_map::pc98,
        filesystem::fat_dir::ATTR_ARCHIVE,
        test_support::{
            FLOPPY_2HD_DATA_SECTOR, FLOPPY_2HD_DEVICE_ID, FLOPPY_2HD_ROOT_SECTOR, MockDisk,
            directory_entry_bytes, formatted_2hd_floppy, human68k_2hd_boot_sector,
            pc98_2hd_boot_sector, store_2hd_single_cluster, store_directory_entry,
        },
    };

    /// Device id of the first PC-98 2DD floppy drive.
    const FLOPPY_2DD_DEVICE_ID: u8 = 0x70;

    /// `表.X`: the trail byte of `表` is `5Ch`.
    const BACKSLASH_TRAIL_NAME: &[u8; 11] = b"\x95\x5C      X  ";

    /// `エディタ.X`: the trail byte of `デ` is the lowercase ASCII `66h`.
    const LOWERCASE_TRAIL_NAME: &[u8; 11] = b"\x83\x47\x83\x66\x83\x42\x83\x5EX  ";

    fn mount_error(disk: &mut MockDisk, device_id: u8) -> Option<FatError> {
        let entry = pc98::drive_entry(device_id).unwrap();
        FatFs::mount(disk, &entry).err()
    }

    #[test]
    fn floppy_without_bpb_and_fat_id_is_unknown_media() {
        let mut disk = MockDisk::new(FLOPPY_2DD_DEVICE_ID, 512, 8, 2, 1280);
        assert_eq!(
            mount_error(&mut disk, FLOPPY_2DD_DEVICE_ID),
            Some(FatError::UnknownMedia)
        );
        assert_eq!(FatError::UnknownMedia.to_string(), "no FAT file system");
    }

    #[test]
    fn floppy_with_a_fat_id_keeps_the_io_error() {
        let mut disk = MockDisk::new(FLOPPY_2DD_DEVICE_ID, 512, 8, 2, 1280);
        let mut fat = vec![0u8; 512];
        fat[..3].copy_from_slice(&[0xFE, 0xFF, 0xFF]);
        disk.write_sector(1, &fat);
        assert_eq!(
            mount_error(&mut disk, FLOPPY_2DD_DEVICE_ID),
            Some(FatError::Io)
        );
    }

    #[test]
    fn unreadable_fat_sector_keeps_the_io_error() {
        let mut disk = MockDisk::new(FLOPPY_2DD_DEVICE_ID, 512, 8, 2, 1);
        assert_eq!(
            mount_error(&mut disk, FLOPPY_2DD_DEVICE_ID),
            Some(FatError::Io)
        );
    }

    #[test]
    fn unreadable_boot_record_keeps_the_io_error() {
        let mut disk = MockDisk::new(FLOPPY_2DD_DEVICE_ID, 512, 8, 2, 0);
        assert_eq!(
            mount_error(&mut disk, FLOPPY_2DD_DEVICE_ID),
            Some(FatError::Io)
        );
    }

    #[test]
    fn entry_access_reaches_shift_jis_names_that_paths_miss() {
        let mut disk = formatted_2hd_floppy(&pc98_2hd_boot_sector());
        let directory = directory_entry_bytes(b"SUB        ", ATTR_DIRECTORY, &[], 2, 0);
        store_directory_entry(&mut disk, FLOPPY_2HD_ROOT_SECTOR, 0, &directory);
        let subdirectory_sector = FLOPPY_2HD_DATA_SECTOR;
        let first = directory_entry_bytes(BACKSLASH_TRAIL_NAME, ATTR_ARCHIVE, &[], 3, 5);
        let second = directory_entry_bytes(LOWERCASE_TRAIL_NAME, ATTR_ARCHIVE, &[], 4, 6);
        store_2hd_single_cluster(&mut disk, 2, &[]);
        store_directory_entry(&mut disk, subdirectory_sector, 0, &first);
        store_directory_entry(&mut disk, subdirectory_sector, 1, &second);
        store_2hd_single_cluster(&mut disk, 3, b"first");
        store_2hd_single_cluster(&mut disk, 4, b"second");

        let entry = pc98::drive_entry(FLOPPY_2HD_DEVICE_ID).unwrap();
        let mut fs = FatFs::mount(&mut disk, &entry).unwrap();
        let root = fs.stat(b"\\").unwrap().unwrap();
        let root_entries = fs.list_entry_dir(&root).unwrap();
        assert_eq!(root_entries.len(), 1);
        let files = fs.list_entry_dir(&root_entries[0]).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].full_name, b"\x95\x5C.X");
        assert_eq!(files[0].full_name, files[0].name);
        assert_eq!(fs.read_entry(&files[0]).unwrap(), b"first");
        assert_eq!(files[1].full_name, b"\x83\x47\x83\x66\x83\x42\x83\x5E.X");
        assert_eq!(fs.read_entry(&files[1]).unwrap(), b"second");

        assert!(
            fs.read_file(b"\\SUB\\\x83\x47\x83\x66\x83\x42\x83\x5E.X")
                .is_err()
        );
        assert_eq!(fs.read_entry(&root_entries[0]), Err(FatError::IsADirectory));
        assert_eq!(
            fs.list_entry_dir(&files[0]).err(),
            Some(FatError::NotADirectory)
        );
    }

    #[test]
    fn human68k_entries_carry_their_18_3_names() {
        let mut disk = formatted_2hd_floppy(&human68k_2hd_boot_sector());
        let first = directory_entry_bytes(b"LONGNAMEX  ", ATTR_ARCHIVE, b"1A", 2, 3);
        let second = directory_entry_bytes(b"LONGNAMEX  ", ATTR_ARCHIVE, b"2b", 3, 4);
        store_directory_entry(&mut disk, FLOPPY_2HD_ROOT_SECTOR, 0, &first);
        store_directory_entry(&mut disk, FLOPPY_2HD_ROOT_SECTOR, 1, &second);
        store_2hd_single_cluster(&mut disk, 2, b"one");
        store_2hd_single_cluster(&mut disk, 3, b"two!");

        let entry = pc98::drive_entry(FLOPPY_2HD_DEVICE_ID).unwrap();
        let mut fs = FatFs::mount(&mut disk, &entry).unwrap();
        let files = fs.list_dir(b"\\").unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].name, b"LONGNAME.X");
        assert_eq!(files[0].full_name, b"LONGNAME1A.X");
        assert_eq!(files[1].full_name, b"LONGNAME2b.X");
        assert_eq!(fs.read_entry(&files[0]).unwrap(), b"one");
        assert_eq!(fs.read_entry(&files[1]).unwrap(), b"two!");
    }

    #[test]
    fn entries_outside_the_data_area_fail_with_io() {
        let mut disk = formatted_2hd_floppy(&pc98_2hd_boot_sector());
        let entries = [
            directory_entry_bytes(b"ZERODIR    ", ATTR_DIRECTORY, &[], 0, 0),
            directory_entry_bytes(b"HIGHDIR    ", ATTR_DIRECTORY, &[], 1223, 0),
            directory_entry_bytes(b"LOWFILE TXT", ATTR_ARCHIVE, &[], 1, 5),
            directory_entry_bytes(b"HIGHFILETXT", ATTR_ARCHIVE, &[], 0xFFF0, 5),
            directory_entry_bytes(b"EMPTY   TXT", ATTR_ARCHIVE, &[], 0, 0),
        ];
        for (slot, entry) in entries.iter().enumerate() {
            store_directory_entry(&mut disk, FLOPPY_2HD_ROOT_SECTOR, slot, entry);
        }

        let entry = pc98::drive_entry(FLOPPY_2HD_DEVICE_ID).unwrap();
        let mut fs = FatFs::mount(&mut disk, &entry).unwrap();
        let root = fs.stat(b"\\").unwrap().unwrap();
        let files = fs.list_entry_dir(&root).unwrap();
        assert_eq!(files.len(), entries.len());
        assert_eq!(fs.list_entry_dir(&files[0]).err(), Some(FatError::Io));
        assert_eq!(fs.list_entry_dir(&files[1]).err(), Some(FatError::Io));
        assert_eq!(fs.read_entry(&files[2]), Err(FatError::Io));
        assert_eq!(fs.read_entry(&files[3]), Err(FatError::Io));
        assert_eq!(fs.read_entry(&files[4]), Ok(Vec::new()));
    }

    #[test]
    fn full_name_maps_a_stored_first_byte_05_to_e5() {
        for boot_sector in [pc98_2hd_boot_sector(), human68k_2hd_boot_sector()] {
            let mut disk = formatted_2hd_floppy(&boot_sector);
            let raw = directory_entry_bytes(b"\x05\xA0      TXT", ATTR_ARCHIVE, &[], 0, 0);
            store_directory_entry(&mut disk, FLOPPY_2HD_ROOT_SECTOR, 0, &raw);
            let entry = pc98::drive_entry(FLOPPY_2HD_DEVICE_ID).unwrap();
            let mut fs = FatFs::mount(&mut disk, &entry).unwrap();
            let files = fs.list_dir(b"\\").unwrap();
            assert_eq!(files[0].name, b"\xE5\xA0.TXT");
            assert_eq!(files[0].full_name, b"\xE5\xA0.TXT");
            assert_eq!(files[0].fcb_name, *b"\xE5\xA0      TXT");
        }
    }
}
