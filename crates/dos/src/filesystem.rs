//! Drive trait, DiskIo trait, drive mapping, error types.

use common::is_shift_jis_lead_byte;

use crate::{DiskIo, DosState, DriveIo, MemoryAccess, dos, process::COMMAND_COM_STUB, tables};

pub mod fat;
pub mod fat_api;
pub(crate) mod fat_bpb;
pub(crate) mod fat_dir;
pub(crate) mod fat_file;
pub(crate) mod fat_partition;
pub(crate) mod iso9660;
pub(crate) mod mbr_partition;
pub(crate) mod virtual_drive;

#[derive(Debug, Clone, Copy)]
/// FAT file identity retained by an open DOS handle.
pub(crate) struct FatHandleMetadata {
    pub drive_index: u8,
    pub name: [u8; 11],
    pub attribute: u8,
    pub time: u16,
    pub date: u16,
    pub start_cluster: u16,
    pub file_size: u32,
    pub position: u32,
    pub dir_sector: u32,
    pub dir_offset: u16,
}

state_struct_codec!(FatHandleMetadata {
    drive_index,
    name,
    attribute,
    time,
    date,
    start_cluster,
    file_size,
    position,
    dir_sector,
    dir_offset,
});

#[derive(Debug, Clone, Copy)]
/// Deferred FAT write data owned by an open DOS handle.
pub(crate) struct PendingFatFile {
    pub drive_index: u8,
    pub dir_cluster: u16,
    pub name: [u8; 11],
    pub attribute: u8,
    pub time: u16,
    pub date: u16,
    pub start_cluster: u16,
    pub file_size: u32,
    pub position: u32,
}

state_struct_codec!(PendingFatFile {
    drive_index,
    dir_cluster,
    name,
    attribute,
    time,
    date,
    start_cluster,
    file_size,
    position,
});

#[derive(Debug, Clone)]
pub(crate) enum ReadDirectory {
    Fat(u16),
    Iso(iso9660::IsoDirectory),
}

impl save_state::StateEncode for ReadDirectory {
    fn encode_state(&self, output: &mut Vec<u8>) {
        match self {
            Self::Fat(cluster) => {
                save_state::StateEncode::encode_state(&0u8, output);
                save_state::StateEncode::encode_state(cluster, output);
            }
            Self::Iso(directory) => {
                save_state::StateEncode::encode_state(&1u8, output);
                save_state::StateEncode::encode_state(directory, output);
            }
        }
    }
}

impl save_state::StateDecode for ReadDirectory {
    fn decode_state(
        decoder: &mut save_state::StateDecoder<'_>,
    ) -> Result<Self, save_state::StateDecodeError> {
        match <u8 as save_state::StateDecode>::decode_state(decoder)? {
            0 => Ok(Self::Fat(save_state::StateDecode::decode_state(decoder)?)),
            1 => Ok(Self::Iso(save_state::StateDecode::decode_state(decoder)?)),
            _ => Err(save_state::StateDecodeError::InvalidTag),
        }
    }
}

#[derive(Debug, Clone)]
/// Parsed path for a pending file read.
pub(crate) struct ReadFilePath {
    pub drive_index: u8,
    pub directory: ReadDirectory,
    pub name: [u8; 11],
}

state_struct_codec!(ReadFilePath {
    drive_index,
    directory,
    name,
});

#[derive(Debug, Clone)]
/// Parsed path for a pending directory read.
pub(crate) struct ReadDirPath {
    pub drive_index: u8,
    pub directory: ReadDirectory,
}

state_struct_codec!(ReadDirPath {
    drive_index,
    directory,
});

#[derive(Debug, Clone)]
/// Filesystem-neutral directory entry returned to DOS services.
pub(crate) struct ReadDirEntry {
    pub name: [u8; 11],
    pub attribute: u8,
    pub time: u16,
    pub date: u16,
    pub file_size: u32,
    pub source: ReadDirEntrySource,
}

state_struct_codec!(ReadDirEntry {
    name,
    attribute,
    time,
    date,
    file_size,
    source,
});

#[derive(Debug, Clone)]
pub(crate) enum ReadDirEntrySource {
    Fat(fat_dir::DirEntry),
    Iso(iso9660::IsoDirEntry),
}

impl save_state::StateEncode for ReadDirEntrySource {
    fn encode_state(&self, output: &mut Vec<u8>) {
        match self {
            Self::Fat(entry) => {
                save_state::StateEncode::encode_state(&0u8, output);
                save_state::StateEncode::encode_state(entry, output);
            }
            Self::Iso(entry) => {
                save_state::StateEncode::encode_state(&1u8, output);
                save_state::StateEncode::encode_state(entry, output);
            }
        }
    }
}

impl save_state::StateDecode for ReadDirEntrySource {
    fn decode_state(
        decoder: &mut save_state::StateDecoder<'_>,
    ) -> Result<Self, save_state::StateDecodeError> {
        match <u8 as save_state::StateDecode>::decode_state(decoder)? {
            0 => Ok(Self::Fat(save_state::StateDecode::decode_state(decoder)?)),
            1 => Ok(Self::Iso(save_state::StateDecode::decode_state(decoder)?)),
            _ => Err(save_state::StateDecodeError::InvalidTag),
        }
    }
}

impl ReadDirEntry {
    pub(crate) fn from_fat(entry: fat_dir::DirEntry) -> Self {
        Self {
            name: entry.name,
            attribute: entry.attribute,
            time: entry.time,
            date: entry.date,
            file_size: entry.file_size,
            source: ReadDirEntrySource::Fat(entry),
        }
    }

    pub(crate) fn from_iso(entry: iso9660::IsoDirEntry) -> Self {
        Self {
            name: entry.name,
            attribute: entry.attribute,
            time: entry.time,
            date: entry.date,
            file_size: entry.file_size,
            source: ReadDirEntrySource::Iso(entry),
        }
    }
}

pub(crate) fn find_read_entry(
    state: &DosState,
    path: &ReadFilePath,
    device: &mut dyn DriveIo,
) -> Result<Option<ReadDirEntry>, u16> {
    match &path.directory {
        ReadDirectory::Fat(dir_cluster) => {
            let volume = state.fat_volumes[path.drive_index as usize]
                .as_ref()
                .ok_or(0x000Fu16)?;
            let entry = fat_dir::find_entry(volume, *dir_cluster, &path.name, device)?;
            Ok(entry.map(ReadDirEntry::from_fat))
        }
        ReadDirectory::Iso(directory) => {
            let volume = iso9660::IsoVolume::mount(device)?;
            let entry = iso9660::find_entry(&volume, directory, &path.name, device)?;
            Ok(entry.map(ReadDirEntry::from_iso))
        }
    }
}

pub(crate) fn find_matching_read_entry(
    state: &DosState,
    drive_index: u8,
    directory: &ReadDirectory,
    pattern: &[u8; 11],
    attr_mask: u8,
    start_index: u16,
    device: &mut dyn DriveIo,
) -> Result<Option<(ReadDirEntry, u16)>, u16> {
    match directory {
        ReadDirectory::Fat(dir_cluster) => {
            let volume = state.fat_volumes[drive_index as usize]
                .as_ref()
                .ok_or(0x000Fu16)?;
            let entry = fat_dir::find_matching(
                volume,
                *dir_cluster,
                pattern,
                attr_mask,
                start_index,
                device,
            )?;
            Ok(entry.map(|(entry, next_index)| (ReadDirEntry::from_fat(entry), next_index)))
        }
        ReadDirectory::Iso(directory) => {
            let volume = iso9660::IsoVolume::mount(device)?;
            let entry = iso9660::find_matching(
                &volume,
                directory,
                pattern,
                attr_mask,
                start_index,
                device,
            )?;
            Ok(entry.map(|(entry, next_index)| (ReadDirEntry::from_iso(entry), next_index)))
        }
    }
}

pub(crate) fn read_entry_all(
    state: &DosState,
    drive_index: u8,
    entry: &ReadDirEntry,
    device: &mut dyn DriveIo,
) -> Result<Vec<u8>, u16> {
    match &entry.source {
        ReadDirEntrySource::Fat(entry) => {
            let volume = state.fat_volumes[drive_index as usize]
                .as_ref()
                .ok_or(0x000Fu16)?;
            fat_file::read_all(volume, entry, device)
        }
        ReadDirEntrySource::Iso(entry) => iso9660::read_all(entry, device),
    }
}

/// Returns true if the byte is a DBCS (Shift-JIS) lead byte.
pub(crate) fn is_dbcs_lead_byte(b: u8) -> bool {
    is_shift_jis_lead_byte(b)
}

/// Splits a DOS path into components, handling SJIS double-byte characters
/// where 0x5C can appear as a trail byte and must not be treated as backslash.
/// Returns (drive_index, components, is_absolute).
/// `drive_index` is None if no drive letter prefix.
pub(crate) fn split_path(path: &[u8]) -> (Option<u8>, Vec<&[u8]>, bool) {
    if path.is_empty() {
        return (None, Vec::new(), false);
    }

    let mut pos = 0;
    let mut drive_index = None;

    // Check for drive letter prefix "X:"
    if path.len() >= 2 && path[1] == b':' {
        let letter = path[0].to_ascii_uppercase();
        if letter.is_ascii_uppercase() {
            drive_index = Some(letter - b'A');
            pos = 2;
        }
    }

    // Check if path is absolute (starts with backslash after optional drive)
    let is_absolute = pos < path.len() && path[pos] == b'\\';

    // Split remaining path on backslash, respecting DBCS
    let mut components = Vec::new();
    let mut comp_start = pos;
    let mut i = pos;

    while i < path.len() {
        if path[i] == 0 {
            break;
        }
        if is_dbcs_lead_byte(path[i]) && i + 1 < path.len() {
            i += 2;
            continue;
        }
        if path[i] == b'\\' {
            if i > comp_start {
                components.push(&path[comp_start..i]);
            }
            i += 1;
            comp_start = i;
            continue;
        }
        i += 1;
    }
    // Last component
    if i > comp_start {
        components.push(&path[comp_start..i]);
    }

    (drive_index, components, is_absolute)
}

/// Resolves a path to `(drive_index, directory_cluster, fcb_name)`.
/// If the path has no explicit drive, uses `current_drive`.
pub(crate) fn resolve_file_path(
    state: &mut DosState,
    path: &[u8],
    mem: &dyn MemoryAccess,
    disk: &mut dyn DiskIo,
) -> Result<(u8, u16, [u8; 11]), u16> {
    let normalized_path = dos::normalize_path(path);
    let (drive_opt, components, is_absolute) = split_path(&normalized_path);
    let drive_index = drive_opt.unwrap_or(state.current_drive);

    if components.is_empty() {
        return Err(0x0002);
    }

    if drive_index != 25 {
        state.ensure_volume_mounted(drive_index, mem, disk)?;
    }

    let mut dir_cluster = if is_absolute || drive_index == 25 {
        0
    } else {
        current_dir_cluster(state, drive_index, mem, disk)?
    };

    if components.len() > 1 {
        let volume = state.fat_volumes[drive_index as usize]
            .as_ref()
            .ok_or(0x000Fu16)?;
        for component in &components[..components.len() - 1] {
            let fcb = fat_dir::name_to_fcb(component);
            let entry = fat_dir::find_entry(volume, dir_cluster, &fcb, disk)?.ok_or(0x0003u16)?;
            if entry.attribute & fat_dir::ATTR_DIRECTORY == 0 {
                return Err(0x0003);
            }
            dir_cluster = entry.start_cluster;
        }
    }

    let fcb_name = fat_dir::name_to_fcb(components.last().unwrap());
    Ok((drive_index, dir_cluster, fcb_name))
}

pub(crate) fn resolve_read_file_path(
    state: &mut DosState,
    path: &[u8],
    mem: &dyn MemoryAccess,
    device: &mut dyn DriveIo,
) -> Result<ReadFilePath, u16> {
    let normalized_path = dos::normalize_path(path);
    let (drive_opt, components, is_absolute) = split_path(&normalized_path);
    let drive_index = drive_opt.unwrap_or(state.current_drive);

    if components.is_empty() {
        return Err(0x0002);
    }

    if drive_index == 25 {
        let (drive_index, dir_cluster, name) = resolve_file_path(state, path, mem, device)?;
        return Ok(ReadFilePath {
            drive_index,
            directory: ReadDirectory::Fat(dir_cluster),
            name,
        });
    }

    state.ensure_readable_drive_ready(drive_index, mem, device)?;

    let mut directory = if is_absolute {
        if state.drive_has_cdrom_filesystem(drive_index, mem) {
            let volume = iso9660::IsoVolume::mount(device)?;
            ReadDirectory::Iso(volume.root_directory)
        } else {
            ReadDirectory::Fat(0)
        }
    } else {
        current_read_directory(state, drive_index, mem, device)?
    };

    for component in &components[..components.len() - 1] {
        let fcb = fat_dir::name_to_fcb(component);
        directory = match &directory {
            ReadDirectory::Fat(dir_cluster) => {
                let volume = state.fat_volumes[drive_index as usize]
                    .as_ref()
                    .ok_or(0x000Fu16)?;
                let entry =
                    fat_dir::find_entry(volume, *dir_cluster, &fcb, device)?.ok_or(0x0003u16)?;
                if entry.attribute & fat_dir::ATTR_DIRECTORY == 0 {
                    return Err(0x0003);
                }
                ReadDirectory::Fat(entry.start_cluster)
            }
            ReadDirectory::Iso(directory) => {
                let volume = iso9660::IsoVolume::mount(device)?;
                let entry =
                    iso9660::find_entry(&volume, directory, &fcb, device)?.ok_or(0x0003u16)?;
                let next_directory = entry.directory.ok_or(0x0003u16)?;
                ReadDirectory::Iso(next_directory)
            }
        };
    }

    Ok(ReadFilePath {
        drive_index,
        directory,
        name: fat_dir::name_to_fcb(components.last().unwrap()),
    })
}

/// Resolves a path to a directory, returning `(drive_index, dir_cluster)`.
pub(crate) fn resolve_dir_path(
    state: &mut DosState,
    path: &[u8],
    mem: &dyn MemoryAccess,
    disk: &mut dyn DiskIo,
) -> Result<(u8, u16), u16> {
    let normalized_path = dos::normalize_path(path);
    let (drive_opt, components, is_absolute) = split_path(&normalized_path);
    let drive_index = drive_opt.unwrap_or(state.current_drive);

    if drive_index == 25 {
        if components.is_empty() {
            return Ok((25, 0));
        }
        return Err(0x0003);
    }
    state.ensure_volume_mounted(drive_index, mem, disk)?;

    let volume = state.fat_volumes[drive_index as usize]
        .as_ref()
        .ok_or(0x000Fu16)?;

    let start_cluster = if is_absolute || components.is_empty() {
        if components.is_empty() && !is_absolute {
            current_dir_cluster(state, drive_index, mem, disk)?
        } else {
            0
        }
    } else {
        current_dir_cluster(state, drive_index, mem, disk)?
    };

    let mut dir_cluster = start_cluster;
    for component in &components {
        let fcb = fat_dir::name_to_fcb(component);
        let entry = fat_dir::find_entry(volume, dir_cluster, &fcb, disk)?.ok_or(0x0003u16)?;
        if entry.attribute & fat_dir::ATTR_DIRECTORY == 0 {
            return Err(0x0003);
        }
        dir_cluster = entry.start_cluster;
    }

    Ok((drive_index, dir_cluster))
}

pub(crate) fn resolve_read_dir_path(
    state: &mut DosState,
    path: &[u8],
    mem: &dyn MemoryAccess,
    device: &mut dyn DriveIo,
) -> Result<ReadDirPath, u16> {
    let normalized_path = dos::normalize_path(path);
    let (drive_opt, components, is_absolute) = split_path(&normalized_path);
    let drive_index = drive_opt.unwrap_or(state.current_drive);

    if drive_index == 25 {
        let (drive_index, dir_cluster) = resolve_dir_path(state, path, mem, device)?;
        return Ok(ReadDirPath {
            drive_index,
            directory: ReadDirectory::Fat(dir_cluster),
        });
    }

    state.ensure_readable_drive_ready(drive_index, mem, device)?;

    let mut directory = if is_absolute || components.is_empty() {
        if state.drive_has_cdrom_filesystem(drive_index, mem) {
            let volume = iso9660::IsoVolume::mount(device)?;
            ReadDirectory::Iso(volume.root_directory)
        } else if components.is_empty() && !is_absolute {
            current_read_directory(state, drive_index, mem, device)?
        } else {
            ReadDirectory::Fat(0)
        }
    } else {
        current_read_directory(state, drive_index, mem, device)?
    };

    for component in &components {
        let fcb = fat_dir::name_to_fcb(component);
        directory = match &directory {
            ReadDirectory::Fat(dir_cluster) => {
                let volume = state.fat_volumes[drive_index as usize]
                    .as_ref()
                    .ok_or(0x000Fu16)?;
                let entry =
                    fat_dir::find_entry(volume, *dir_cluster, &fcb, device)?.ok_or(0x0003u16)?;
                if entry.attribute & fat_dir::ATTR_DIRECTORY == 0 {
                    return Err(0x0003);
                }
                ReadDirectory::Fat(entry.start_cluster)
            }
            ReadDirectory::Iso(directory) => {
                let volume = iso9660::IsoVolume::mount(device)?;
                let entry =
                    iso9660::find_entry(&volume, directory, &fcb, device)?.ok_or(0x0003u16)?;
                let next_directory = entry.directory.ok_or(0x0003u16)?;
                ReadDirectory::Iso(next_directory)
            }
        };
    }

    Ok(ReadDirPath {
        drive_index,
        directory,
    })
}

pub(crate) fn change_directory(
    state: &mut DosState,
    memory: &mut dyn MemoryAccess,
    device: &mut dyn DriveIo,
    path_bytes: &[u8],
) -> Result<(), u16> {
    if path_bytes.is_empty() {
        return Err(0x0003);
    }

    let (drive_index, path_start) = if path_bytes.len() >= 2 && path_bytes[1] == b':' {
        let letter = path_bytes[0].to_ascii_uppercase();
        if !letter.is_ascii_uppercase() {
            return Err(0x0003);
        }
        (letter - b'A', 2)
    } else {
        (state.current_drive, 0)
    };

    let cds_addr = tables::CDS_BASE + (drive_index as u32) * tables::CDS_ENTRY_SIZE;
    let cds_flags = memory.read_word(cds_addr + tables::CDS_OFF_FLAGS);
    if cds_flags == 0 {
        return Err(0x000F);
    }

    let remaining = &path_bytes[path_start..];
    let mut new_path = Vec::with_capacity(67);
    new_path.push(b'A' + drive_index);
    new_path.push(b':');

    if remaining.is_empty() || remaining[0] == b'\\' {
        if remaining.is_empty() {
            new_path.push(b'\\');
        } else {
            new_path.extend_from_slice(remaining);
        }
    } else {
        let current = read_cds_path(memory, drive_index);
        if current.len() > 2 {
            new_path.extend_from_slice(&current[2..]);
        } else {
            new_path.push(b'\\');
        }
        if new_path.last() != Some(&b'\\') {
            new_path.push(b'\\');
        }
        new_path.extend_from_slice(remaining);
    }

    let normalized = dos::normalize_path(&new_path);
    let final_path = if normalized.len() > 3 && normalized.last() == Some(&b'\\') {
        &normalized[..normalized.len() - 1]
    } else {
        &normalized
    };

    if final_path.len() > 67 {
        return Err(0x0003);
    }

    if final_path.len() > 3 {
        if drive_index == 25 {
            return Err(0x0003);
        }
        resolve_read_dir_path(state, final_path, memory, device)?;
    }

    for i in 0..67u32 {
        if (i as usize) < final_path.len() {
            memory.write_byte(cds_addr + tables::CDS_OFF_PATH + i, final_path[i as usize]);
        } else {
            memory.write_byte(cds_addr + tables::CDS_OFF_PATH + i, 0x00);
        }
    }

    Ok(())
}

pub(crate) fn create_directory(
    state: &mut DosState,
    memory: &dyn MemoryAccess,
    disk: &mut dyn DiskIo,
    path: &[u8],
) -> Result<(), u16> {
    let (drive_index, parent_cluster, fcb_name) = resolve_file_path(state, path, memory, disk)?;
    create_directory_in_parent(state, disk, drive_index, parent_cluster, fcb_name, None)
}

pub(crate) fn remove_directory(
    state: &mut DosState,
    memory: &dyn MemoryAccess,
    disk: &mut dyn DiskIo,
    path: &[u8],
) -> Result<(), u16> {
    let (drive_index, parent_cluster, fcb_name) = resolve_file_path(state, path, memory, disk)?;

    if drive_index == 25 {
        return Err(0x0005);
    }

    let volume = state.fat_volumes[drive_index as usize]
        .as_mut()
        .ok_or(0x000Fu16)?;
    let entry = fat_dir::find_entry(volume, parent_cluster, &fcb_name, disk)?.ok_or(0x0003u16)?;

    if entry.attribute & fat_dir::ATTR_DIRECTORY == 0 {
        return Err(0x0003);
    }

    if !directory_is_empty(volume, entry.start_cluster, disk)? {
        return Err(0x0012);
    }

    fat_dir::delete_entry(volume, &entry, disk)?;
    if entry.start_cluster >= 2 {
        volume.free_chain(entry.start_cluster);
    }
    volume.flush_fat(disk)?;
    Ok(())
}

pub(crate) fn create_or_truncate_file(
    state: &mut DosState,
    memory: &dyn MemoryAccess,
    disk: &mut dyn DiskIo,
    path: &[u8],
    attributes: u8,
) -> Result<fat_dir::DirEntry, u16> {
    let (drive_index, dir_cluster, fcb_name) = resolve_file_path(state, path, memory, disk)?;

    if drive_index == 25 {
        return Err(0x0005);
    }

    let (time, date) = state.dos_timestamp_now();
    let volume = state.fat_volumes[drive_index as usize]
        .as_mut()
        .ok_or(0x000Fu16)?;

    if let Some(existing) = fat_dir::find_entry(volume, dir_cluster, &fcb_name, disk)? {
        if existing.start_cluster >= 2 {
            volume.free_chain(existing.start_cluster);
        }
        let mut updated = existing;
        updated.file_size = 0;
        updated.start_cluster = 0;
        updated.attribute = attributes & 0x27;
        updated.time = time;
        updated.date = date;
        fat_dir::update_entry(volume, &updated, disk)?;
        volume.flush_fat(disk)?;
        return Ok(updated);
    }

    let new_entry = fat_dir::DirEntry {
        name: fcb_name,
        attribute: attributes & 0x27,
        time,
        date,
        start_cluster: 0,
        file_size: 0,
        dir_sector: 0,
        dir_offset: 0,
    };
    let created = fat_dir::create_entry(volume, dir_cluster, &new_entry, disk)?;
    volume.flush_fat(disk)?;
    Ok(created)
}

pub(crate) fn delete_file(
    state: &mut DosState,
    memory: &dyn MemoryAccess,
    disk: &mut dyn DiskIo,
    path: &[u8],
) -> Result<(), u16> {
    let (drive_index, dir_cluster, fcb_name) = resolve_file_path(state, path, memory, disk)?;

    delete_file_by_components(state, disk, drive_index, dir_cluster, fcb_name)
}

pub(crate) fn delete_file_by_components(
    state: &mut DosState,
    disk: &mut dyn DiskIo,
    drive_index: u8,
    dir_cluster: u16,
    fcb_name: [u8; 11],
) -> Result<(), u16> {
    if drive_index == 25 {
        return Err(0x0005);
    }

    let volume = state.fat_volumes[drive_index as usize]
        .as_mut()
        .ok_or(0x000Fu16)?;
    let entry = fat_dir::find_entry(volume, dir_cluster, &fcb_name, disk)?.ok_or(0x0002u16)?;

    if entry.attribute & (fat_dir::ATTR_DIRECTORY | fat_dir::ATTR_VOLUME_ID) != 0 {
        return Err(0x0005);
    }

    if entry.start_cluster >= 2 {
        volume.free_chain(entry.start_cluster);
    }
    fat_dir::delete_entry(volume, &entry, disk)?;
    volume.flush_fat(disk)?;
    Ok(())
}

pub(crate) fn rename_entry(
    state: &mut DosState,
    memory: &dyn MemoryAccess,
    disk: &mut dyn DiskIo,
    old_path: &[u8],
    new_path: &[u8],
) -> Result<(), u16> {
    let (drive_old, dir_old, fcb_old) = resolve_file_path(state, old_path, memory, disk)?;
    let (drive_new, dir_new, fcb_new) = resolve_file_path(state, new_path, memory, disk)?;

    rename_entry_by_components(
        state,
        disk,
        (drive_old, dir_old, fcb_old),
        (drive_new, dir_new, fcb_new),
    )
}

pub(crate) fn rename_entry_by_components(
    state: &mut DosState,
    disk: &mut dyn DiskIo,
    old_entry: (u8, u16, [u8; 11]),
    new_entry: (u8, u16, [u8; 11]),
) -> Result<(), u16> {
    let (drive_old, dir_old, fcb_old) = old_entry;
    let (drive_new, dir_new, fcb_new) = new_entry;

    if drive_old != drive_new {
        return Err(0x0011);
    }
    if drive_old == 25 {
        return Err(0x0005);
    }

    let volume = state.fat_volumes[drive_old as usize]
        .as_mut()
        .ok_or(0x000Fu16)?;
    let mut entry = fat_dir::find_entry(volume, dir_old, &fcb_old, disk)?.ok_or(0x0002u16)?;

    if fat_dir::find_entry(volume, dir_new, &fcb_new, disk)?.is_some() {
        return Err(0x0005);
    }

    entry.name = fcb_new;
    fat_dir::update_entry(volume, &entry, disk)?;
    Ok(())
}

pub(crate) fn get_attributes(
    state: &mut DosState,
    memory: &dyn MemoryAccess,
    disk: &mut dyn DriveIo,
    path: &[u8],
) -> Result<u8, u16> {
    let read_path = resolve_read_file_path(state, path, memory, disk)?;
    let drive_index = read_path.drive_index;

    if drive_index == 25 {
        let (_, _, fcb_name) = resolve_file_path(state, path, memory, disk)?;
        if let Some(virtual_entry) = state.virtual_drive.find_entry(&fcb_name) {
            return Ok(virtual_entry.attribute);
        }
        return Err(0x0002);
    }

    let entry = find_read_entry(state, &read_path, disk)?.ok_or(0x0002u16)?;
    Ok(entry.attribute)
}

pub(crate) fn set_attributes(
    state: &mut DosState,
    memory: &dyn MemoryAccess,
    disk: &mut dyn DriveIo,
    path: &[u8],
    attributes: u8,
) -> Result<u8, u16> {
    let read_path = resolve_read_file_path(state, path, memory, disk)?;
    let ReadDirEntrySource::Fat(entry) = find_read_entry(state, &read_path, disk)?
        .ok_or(0x0002u16)?
        .source
    else {
        return Err(0x0005);
    };

    let volume = state.fat_volumes[read_path.drive_index as usize]
        .as_mut()
        .ok_or(0x000Fu16)?;
    let mut updated = entry;
    updated.attribute = attributes & 0x27;
    fat_dir::update_entry(volume, &updated, disk)?;
    Ok(updated.attribute)
}

pub(crate) fn write_fat_handle(
    state: &mut DosState,
    disk: &mut dyn DiskIo,
    mut metadata: FatHandleMetadata,
    data: &[u8],
) -> Result<(u16, FatHandleMetadata), u16> {
    let volume = state.fat_volumes[metadata.drive_index as usize]
        .as_mut()
        .ok_or(0x000Fu16)?;
    let mut writer = fat_file::FatFileWriter::new(metadata.start_cluster, metadata.position);
    writer.write_chunk(volume, disk, data)?;
    metadata.start_cluster = writer.start_cluster();
    metadata.position = writer.position();
    metadata.file_size = metadata.file_size.max(metadata.position);
    volume.flush_fat(disk)?;
    Ok((data.len() as u16, metadata))
}

pub(crate) fn flush_fat_handle(
    state: &mut DosState,
    disk: &mut dyn DiskIo,
    metadata: &FatHandleMetadata,
) -> Result<(), u16> {
    let volume = state.fat_volumes[metadata.drive_index as usize]
        .as_mut()
        .ok_or(0x000Fu16)?;
    let entry = fat_dir::DirEntry {
        name: metadata.name,
        attribute: metadata.attribute,
        time: metadata.time,
        date: metadata.date,
        start_cluster: metadata.start_cluster,
        file_size: metadata.file_size,
        dir_sector: metadata.dir_sector,
        dir_offset: metadata.dir_offset,
    };
    fat_dir::update_entry(volume, &entry, disk)?;
    volume.flush_fat(disk)?;
    Ok(())
}

pub(crate) fn write_pending_file_chunk(
    state: &mut DosState,
    disk: &mut dyn DiskIo,
    mut pending_file: PendingFatFile,
    data: &[u8],
) -> Result<(PendingFatFile, u16), u16> {
    let volume = state.fat_volumes[pending_file.drive_index as usize]
        .as_mut()
        .ok_or(0x000Fu16)?;
    let mut writer =
        fat_file::FatFileWriter::new(pending_file.start_cluster, pending_file.position);
    writer.write_chunk(volume, disk, data)?;
    pending_file.start_cluster = writer.start_cluster();
    pending_file.position = writer.position();
    pending_file.file_size = pending_file.file_size.max(pending_file.position);
    Ok((pending_file, writer.current_cluster()))
}

pub(crate) fn finish_pending_file(
    state: &mut DosState,
    disk: &mut dyn DiskIo,
    pending_file: PendingFatFile,
) -> Result<(), u16> {
    let volume = state.fat_volumes[pending_file.drive_index as usize]
        .as_mut()
        .ok_or(0x000Fu16)?;

    if let Some(existing) =
        fat_dir::find_entry(volume, pending_file.dir_cluster, &pending_file.name, disk)?
    {
        if existing.start_cluster >= 2 {
            volume.free_chain(existing.start_cluster);
        }
        fat_dir::delete_entry(volume, &existing, disk)?;
    }

    let new_entry = fat_dir::DirEntry {
        name: pending_file.name,
        attribute: pending_file.attribute,
        time: pending_file.time,
        date: pending_file.date,
        start_cluster: pending_file.start_cluster,
        file_size: pending_file.file_size,
        dir_sector: 0,
        dir_offset: 0,
    };
    fat_dir::create_entry(volume, pending_file.dir_cluster, &new_entry, disk)?;
    volume.flush_fat(disk)?;
    Ok(())
}

pub(crate) fn ensure_directory(
    state: &mut DosState,
    disk: &mut dyn DiskIo,
    drive_index: u8,
    parent_cluster: u16,
    name: [u8; 11],
    timestamp: Option<(u16, u16)>,
) -> Result<u16, u16> {
    if drive_index == 25 {
        return Err(0x0005);
    }

    if let Some(existing) = state.fat_volumes[drive_index as usize]
        .as_mut()
        .ok_or(0x000Fu16)
        .and_then(|volume| fat_dir::find_entry(volume, parent_cluster, &name, disk))?
    {
        if existing.attribute & fat_dir::ATTR_DIRECTORY == 0 {
            return Err(0x0005);
        }
        return Ok(existing.start_cluster);
    }

    let (time, date) = timestamp.unwrap_or_else(|| state.dos_timestamp_now());
    create_directory_in_parent(
        state,
        disk,
        drive_index,
        parent_cluster,
        name,
        Some((time, date)),
    )?;

    let volume = state.fat_volumes[drive_index as usize]
        .as_mut()
        .ok_or(0x000Fu16)?;
    let entry = fat_dir::find_entry(volume, parent_cluster, &name, disk)?.ok_or(0x0003u16)?;
    Ok(entry.start_cluster)
}

pub(crate) fn create_dos_mock_files(
    state: &mut DosState,
    memory: &dyn MemoryAccess,
    disk: &mut dyn DiskIo,
    drive_index: u8,
) -> Result<(), u16> {
    if drive_index == 25 {
        return Err(0x0005);
    }

    let cds_addr = tables::CDS_BASE + (drive_index as u32) * tables::CDS_ENTRY_SIZE;
    let cds_flags = memory.read_word(cds_addr + tables::CDS_OFF_FLAGS);
    if cds_flags == 0 {
        return Err(0x000Fu16);
    }
    if state.mscdex.drive_letter == drive_index && cds_flags & tables::CDS_FLAG_PHYSICAL == 0 {
        return Err(0x0005);
    }

    state.ensure_volume_mounted(drive_index, memory, disk)?;

    let (time, date) = state.dos_timestamp_now();
    let volume = state.fat_volumes[drive_index as usize]
        .as_mut()
        .ok_or(0x000Fu16)?;

    let io_sys = fat_dir::name_to_fcb(b"IO.SYS");
    let msdos_sys = fat_dir::name_to_fcb(b"MSDOS.SYS");
    let command_com = fat_dir::name_to_fcb(b"COMMAND.COM");

    for name in [io_sys, msdos_sys, command_com] {
        if fat_dir::find_entry(volume, 0, &name, disk)?.is_some() {
            return Err(0x0050);
        }
    }

    let mut created = Vec::new();
    let files = [
        (
            io_sys,
            &[][..],
            fat_dir::ATTR_READ_ONLY | fat_dir::ATTR_HIDDEN | fat_dir::ATTR_SYSTEM,
        ),
        (
            msdos_sys,
            &[][..],
            fat_dir::ATTR_READ_ONLY | fat_dir::ATTR_HIDDEN | fat_dir::ATTR_SYSTEM,
        ),
        (
            command_com,
            COMMAND_COM_STUB,
            fat_dir::ATTR_READ_ONLY | fat_dir::ATTR_ARCHIVE,
        ),
    ];

    for (name, content, attributes) in files {
        if fat_file::create_or_replace_file(
            volume,
            0,
            &name,
            content,
            fat_file::FileCreateOptions {
                attributes,
                time,
                date,
            },
            disk,
        )
        .is_err()
        {
            rollback_created_files(volume, disk, &created);
            let _ = volume.flush_fat(disk);
            return Err(0x001Fu16);
        }
        created.push(name);
    }

    if volume.flush_fat(disk).is_err() {
        rollback_created_files(volume, disk, &created);
        let _ = volume.flush_fat(disk);
        return Err(0x001Fu16);
    }

    Ok(())
}

fn create_directory_in_parent(
    state: &mut DosState,
    disk: &mut dyn DiskIo,
    drive_index: u8,
    parent_cluster: u16,
    fcb_name: [u8; 11],
    timestamp: Option<(u16, u16)>,
) -> Result<(), u16> {
    if drive_index == 25 {
        return Err(0x0005);
    }

    let (time, date) = timestamp.unwrap_or_else(|| state.dos_timestamp_now());
    let volume = state.fat_volumes[drive_index as usize]
        .as_mut()
        .ok_or(0x000Fu16)?;

    fat_dir::create_subdirectory(volume, parent_cluster, fcb_name, time, date, disk)?;

    Ok(())
}

fn directory_is_empty(
    volume: &fat::FatVolume,
    dir_cluster: u16,
    disk: &mut dyn DiskIo,
) -> Result<bool, u16> {
    let all_pattern = [b'?'; 11];
    let mut start_index = 0u16;

    loop {
        let result = fat_dir::find_matching(
            volume,
            dir_cluster,
            &all_pattern,
            fat_dir::ATTR_HIDDEN | fat_dir::ATTR_SYSTEM | fat_dir::ATTR_DIRECTORY,
            start_index,
            disk,
        )?;

        match result {
            Some((entry, next_index)) => {
                if entry.name == *b".          " || entry.name == *b"..         " {
                    start_index = next_index;
                    continue;
                }
                return Ok(false);
            }
            None => return Ok(true),
        }
    }
}

fn current_dir_cluster(
    state: &DosState,
    drive_index: u8,
    mem: &dyn MemoryAccess,
    disk: &mut dyn DiskIo,
) -> Result<u16, u16> {
    let cds_path = read_cds_path(mem, drive_index);
    let (_, components, _) = split_path(&cds_path);
    let volume = state.fat_volumes[drive_index as usize]
        .as_ref()
        .ok_or(0x000Fu16)?;

    let mut dir_cluster = 0u16;
    for component in &components {
        let fcb = fat_dir::name_to_fcb(component);
        if let Some(entry) = fat_dir::find_entry(volume, dir_cluster, &fcb, disk)?
            && entry.attribute & fat_dir::ATTR_DIRECTORY != 0
        {
            dir_cluster = entry.start_cluster;
        }
    }

    Ok(dir_cluster)
}

fn current_read_directory(
    state: &mut DosState,
    drive_index: u8,
    mem: &dyn MemoryAccess,
    device: &mut dyn DriveIo,
) -> Result<ReadDirectory, u16> {
    if state.drive_has_cdrom_filesystem(drive_index, mem) {
        let volume = iso9660::IsoVolume::mount(device)?;
        let cds_path = read_cds_path(mem, drive_index);
        let (_, components, _) = split_path(&cds_path);
        let mut directory = volume.root_directory.clone();

        for component in &components {
            let fcb = fat_dir::name_to_fcb(component);
            let entry = iso9660::find_entry(&volume, &directory, &fcb, device)?.ok_or(0x0003u16)?;
            directory = entry.directory.ok_or(0x0003u16)?;
        }

        Ok(ReadDirectory::Iso(directory))
    } else {
        Ok(ReadDirectory::Fat(current_dir_cluster(
            state,
            drive_index,
            mem,
            device,
        )?))
    }
}

fn read_cds_path(mem: &dyn MemoryAccess, drive_index: u8) -> Vec<u8> {
    let cds_addr = tables::CDS_BASE + (drive_index as u32) * tables::CDS_ENTRY_SIZE;
    let mut path = Vec::new();
    for i in 0..67u32 {
        let byte = mem.read_byte(cds_addr + tables::CDS_OFF_PATH + i);
        if byte == 0 {
            break;
        }
        path.push(byte);
    }
    path
}

fn rollback_created_files(
    volume: &mut fat::FatVolume,
    disk: &mut dyn DiskIo,
    created: &[[u8; 11]],
) {
    for name in created {
        if let Ok(Some(entry)) = fat_dir::find_entry(volume, 0, name, disk) {
            if entry.start_cluster >= 2 {
                volume.free_chain(entry.start_cluster);
            }
            let _ = fat_dir::delete_entry(volume, &entry, disk);
        }
    }
}
