//! Directory entry parsing, creation, 8.3 name handling.

use crate::{DiskIo, filesystem::fat::FatVolume};

pub(crate) const DIR_ENTRY_SIZE: usize = 32;

pub(crate) const ATTR_READ_ONLY: u8 = 0x01;
pub(crate) const ATTR_HIDDEN: u8 = 0x02;
pub(crate) const ATTR_SYSTEM: u8 = 0x04;
pub(crate) const ATTR_VOLUME_ID: u8 = 0x08;
pub(crate) const ATTR_DIRECTORY: u8 = 0x10;
pub(crate) const ATTR_ARCHIVE: u8 = 0x20;

/// A parsed directory entry with its on-disk location.
#[derive(Debug, Clone)]
pub(crate) struct DirEntry {
    pub name: [u8; 11],
    pub attribute: u8,
    pub time: u16,
    pub date: u16,
    pub start_cluster: u16,
    pub file_size: u32,
    /// Absolute sector containing this entry.
    pub dir_sector: u32,
    /// Byte offset of this entry within the sector.
    pub dir_offset: u16,
}

impl DirEntry {
    /// Serializes the entry back to a 32-byte buffer.
    pub fn to_bytes(&self) -> [u8; 32] {
        let mut buf = [0u8; 32];
        buf[0..11].copy_from_slice(&self.name);
        buf[11] = self.attribute;
        // bytes 12-21: reserved/NT, creation time, etc. - leave as zero
        let time_bytes = self.time.to_le_bytes();
        buf[22] = time_bytes[0];
        buf[23] = time_bytes[1];
        let date_bytes = self.date.to_le_bytes();
        buf[24] = date_bytes[0];
        buf[25] = date_bytes[1];
        let cluster_bytes = self.start_cluster.to_le_bytes();
        buf[26] = cluster_bytes[0];
        buf[27] = cluster_bytes[1];
        let size_bytes = self.file_size.to_le_bytes();
        buf[28] = size_bytes[0];
        buf[29] = size_bytes[1];
        buf[30] = size_bytes[2];
        buf[31] = size_bytes[3];
        buf
    }
}

/// Parses a 32-byte directory entry. Returns None for deleted (0xE5) or
/// end-of-directory (0x00) markers.
pub(crate) fn parse_entry(data: &[u8], sector: u32, offset: u16) -> Option<DirEntry> {
    if data.len() < 32 {
        return None;
    }
    let first_byte = data[0];
    if first_byte == 0x00 || first_byte == 0xE5 {
        return None;
    }
    Some(DirEntry {
        name: data[0..11].try_into().unwrap(),
        attribute: data[11],
        time: u16::from_le_bytes([data[22], data[23]]),
        date: u16::from_le_bytes([data[24], data[25]]),
        start_cluster: u16::from_le_bytes([data[26], data[27]]),
        file_size: u32::from_le_bytes([data[28], data[29], data[30], data[31]]),
        dir_sector: sector,
        dir_offset: offset,
    })
}

/// Converts a display filename (e.g. "FILE.TXT") to 8.3 FCB format (11 bytes, space-padded).
pub(crate) fn name_to_fcb(name: &[u8]) -> [u8; 11] {
    let mut fcb = [b' '; 11];

    // Handle special wildcard names
    if name == b"*.*" || name == b"*" {
        return [b'?'; 11];
    }

    let mut src = 0;
    let mut dst = 0;
    let name_upper: Vec<u8> = name.iter().map(|b| b.to_ascii_uppercase()).collect();

    // Copy base name (up to 8 chars, stop at '.')
    while src < name_upper.len() && dst < 8 {
        if name_upper[src] == b'.' {
            break;
        }
        if name_upper[src] == b'*' {
            while dst < 8 {
                fcb[dst] = b'?';
                dst += 1;
            }
            break;
        }
        fcb[dst] = name_upper[src];
        src += 1;
        dst += 1;
    }

    // Skip to extension
    while src < name_upper.len() && name_upper[src] != b'.' {
        src += 1;
    }
    if src < name_upper.len() && name_upper[src] == b'.' {
        src += 1;
    }

    // Copy extension (up to 3 chars)
    dst = 8;
    while src < name_upper.len() && dst < 11 {
        if name_upper[src] == b'*' {
            while dst < 11 {
                fcb[dst] = b'?';
                dst += 1;
            }
            break;
        }
        fcb[dst] = name_upper[src];
        src += 1;
        dst += 1;
    }

    fcb
}

/// Converts an FCB-format filename to a display name (e.g. "FILE.TXT").
pub(crate) fn fcb_to_display_name(fcb: &[u8; 11]) -> Vec<u8> {
    let mut result = Vec::with_capacity(13);

    // Base name (trim trailing spaces)
    let base_end = fcb[..8]
        .iter()
        .rposition(|&b| b != b' ')
        .map_or(0, |p| p + 1);
    result.extend_from_slice(&fcb[..base_end]);

    // Extension (trim trailing spaces)
    let ext_end = fcb[8..11]
        .iter()
        .rposition(|&b| b != b' ')
        .map_or(0, |p| p + 1);
    if ext_end > 0 {
        result.push(b'.');
        result.extend_from_slice(&fcb[8..8 + ext_end]);
    }

    result
}

/// Returns true if the FCB name matches the pattern (with '?' wildcards).
pub(crate) fn matches_pattern(name: &[u8; 11], pattern: &[u8; 11]) -> bool {
    for i in 0..11 {
        if pattern[i] != b'?' && pattern[i] != name[i] {
            return false;
        }
    }
    true
}

/// Searches a directory for an entry with the exact FCB name.
/// `dir_cluster` == 0 means root directory.
pub(crate) fn find_entry(
    vol: &FatVolume,
    dir_cluster: u16,
    name: &[u8; 11],
    disk: &mut dyn DiskIo,
) -> Result<Option<DirEntry>, u16> {
    for_each_entry(vol, dir_cluster, disk, |entry| {
        if entry.name == *name {
            return IterAction::Return(entry);
        }
        IterAction::Continue
    })
}

/// Searches a directory for the next entry matching the pattern (with wildcards)
/// and attribute mask, starting at `start_index`.
/// Returns the entry and the next index to resume from.
pub(crate) fn find_matching(
    vol: &FatVolume,
    dir_cluster: u16,
    pattern: &[u8; 11],
    attr_mask: u8,
    start_index: u16,
    disk: &mut dyn DiskIo,
) -> Result<Option<(DirEntry, u16)>, u16> {
    let mut current_index = 0u16;
    let result = for_each_entry(vol, dir_cluster, disk, |entry| {
        if current_index < start_index {
            current_index += 1;
            return IterAction::Continue;
        }
        current_index += 1;

        // Skip volume labels unless specifically requested
        if entry.attribute & ATTR_VOLUME_ID != 0 && attr_mask & ATTR_VOLUME_ID == 0 {
            return IterAction::Continue;
        }

        // Hidden/system/directory entries only shown if attr_mask includes them
        if entry.attribute & ATTR_HIDDEN != 0 && attr_mask & ATTR_HIDDEN == 0 {
            return IterAction::Continue;
        }
        if entry.attribute & ATTR_SYSTEM != 0 && attr_mask & ATTR_SYSTEM == 0 {
            return IterAction::Continue;
        }
        if entry.attribute & ATTR_DIRECTORY != 0 && attr_mask & ATTR_DIRECTORY == 0 {
            return IterAction::Continue;
        }

        if matches_pattern(&entry.name, pattern) {
            return IterAction::Return(entry);
        }
        IterAction::Continue
    })?;

    Ok(result.map(|entry| (entry, current_index)))
}

/// Creates a new directory entry in the given directory.
/// Finds the first free slot (0x00 or 0xE5) and writes the entry.
pub(crate) fn create_entry(
    vol: &mut FatVolume,
    dir_cluster: u16,
    entry: &DirEntry,
    disk: &mut dyn DiskIo,
) -> Result<DirEntry, u16> {
    let sector_size = vol.bpb.bytes_per_sector as usize;
    let entries_per_sector = sector_size / DIR_ENTRY_SIZE;

    let sectors = dir_sectors(vol, dir_cluster, disk)?;
    for abs_sector in &sectors {
        let sector_data = vol.read_sector_abs(*abs_sector, disk)?;
        for i in 0..entries_per_sector {
            let offset = i * DIR_ENTRY_SIZE;
            let first_byte = sector_data[offset];
            if first_byte == 0x00 || first_byte == 0xE5 {
                let mut new_sector = sector_data;
                let entry_bytes = entry.to_bytes();
                new_sector[offset..offset + DIR_ENTRY_SIZE].copy_from_slice(&entry_bytes);
                vol.write_sector_abs(*abs_sector, &new_sector, disk)?;
                let mut created = entry.clone();
                created.dir_sector = *abs_sector;
                created.dir_offset = offset as u16;
                return Ok(created);
            }
        }
    }

    if dir_cluster == 0 {
        return Err(0x0005);
    }

    let mut last_cluster = dir_cluster;
    while let Some(next_cluster) = vol.next_cluster(last_cluster) {
        last_cluster = next_cluster;
    }

    let new_cluster = vol.allocate_cluster(last_cluster).ok_or(0x0005u16)?;
    let cluster_size = vol.bpb.sectors_per_cluster as usize * vol.bpb.bytes_per_sector as usize;
    let zero_data = vec![0u8; cluster_size];
    if let Err(error) = vol.write_cluster(new_cluster, &zero_data, disk) {
        vol.write_fat_entry(last_cluster, if vol.is_fat16 { 0xFFFF } else { 0x0FFF });
        vol.write_fat_entry(new_cluster, 0);
        return Err(error);
    }

    let abs_sector = vol.cluster_to_lba(new_cluster);
    let mut sector_data = vec![0u8; sector_size];
    let entry_bytes = entry.to_bytes();
    sector_data[..DIR_ENTRY_SIZE].copy_from_slice(&entry_bytes);
    vol.write_sector_abs(abs_sector, &sector_data, disk)?;

    let mut created = entry.clone();
    created.dir_sector = abs_sector;
    created.dir_offset = 0;
    Ok(created)
}

/// Updates an existing directory entry on disk at its stored location.
pub(crate) fn update_entry(
    vol: &FatVolume,
    entry: &DirEntry,
    disk: &mut dyn DiskIo,
) -> Result<(), u16> {
    let mut sector_data = vol.read_sector_abs(entry.dir_sector, disk)?;
    let offset = entry.dir_offset as usize;
    let entry_bytes = entry.to_bytes();
    sector_data[offset..offset + DIR_ENTRY_SIZE].copy_from_slice(&entry_bytes);
    vol.write_sector_abs(entry.dir_sector, &sector_data, disk)
}

/// Marks a directory entry as deleted (sets first byte to 0xE5).
pub(crate) fn delete_entry(
    vol: &FatVolume,
    entry: &DirEntry,
    disk: &mut dyn DiskIo,
) -> Result<(), u16> {
    let mut sector_data = vol.read_sector_abs(entry.dir_sector, disk)?;
    let offset = entry.dir_offset as usize;
    sector_data[offset] = 0xE5;
    vol.write_sector_abs(entry.dir_sector, &sector_data, disk)
}

enum IterAction {
    Continue,
    Return(DirEntry),
}

/// Iterates over all valid entries in a directory, calling the callback for each.
fn for_each_entry(
    vol: &FatVolume,
    dir_cluster: u16,
    disk: &mut dyn DiskIo,
    mut callback: impl FnMut(DirEntry) -> IterAction,
) -> Result<Option<DirEntry>, u16> {
    let sector_size = vol.bpb.bytes_per_sector as usize;
    let entries_per_sector = sector_size / DIR_ENTRY_SIZE;

    let sectors = dir_sectors(vol, dir_cluster, disk)?;
    for abs_sector in &sectors {
        let sector_data = vol.read_sector_abs(*abs_sector, disk)?;
        for i in 0..entries_per_sector {
            let offset = i * DIR_ENTRY_SIZE;
            let first_byte = sector_data[offset];
            if first_byte == 0x00 {
                return Ok(None);
            }
            if first_byte == 0xE5 {
                continue;
            }
            if let Some(entry) = parse_entry(
                &sector_data[offset..offset + DIR_ENTRY_SIZE],
                *abs_sector,
                offset as u16,
            ) {
                match callback(entry) {
                    IterAction::Continue => {}
                    IterAction::Return(e) => return Ok(Some(e)),
                }
            }
        }
    }
    Ok(None)
}

/// Collects the physical LBAs for each BPB logical sector in a directory.
/// Root directory (dir_cluster == 0) uses fixed sectors; subdirectories follow cluster chain.
/// Each returned LBA points to the start of one BPB logical sector
/// (which may span multiple physical sectors when sector_ratio > 1).
fn dir_sectors(vol: &FatVolume, dir_cluster: u16, disk: &mut dyn DiskIo) -> Result<Vec<u32>, u16> {
    let _ = disk;
    let ratio = vol.sector_ratio();
    if dir_cluster == 0 {
        let start = vol.root_dir_lba();
        let count = vol.root_dir_sectors();
        Ok((0..count).map(|i| start + i * ratio).collect())
    } else {
        let mut sectors = Vec::new();
        let spc = vol.bpb.sectors_per_cluster as u32;
        let mut cluster = dir_cluster;
        loop {
            let base = vol.cluster_to_lba(cluster);
            for i in 0..spc {
                sectors.push(base + i * ratio);
            }
            match vol.next_cluster(cluster) {
                Some(next) => cluster = next,
                None => break,
            }
        }
        Ok(sectors)
    }
}

/// Creates a new subdirectory under `parent_cluster`. Allocates a fresh cluster,
/// zeroes it, writes `.` and `..` entries, and links the directory entry into
/// the parent. Flushes the FAT on success; rolls back the cluster allocation
/// on failure. `parent_cluster == 0` means the root directory.
pub(crate) fn create_subdirectory(
    vol: &mut FatVolume,
    parent_cluster: u16,
    fcb_name: [u8; 11],
    time: u16,
    date: u16,
    disk: &mut dyn DiskIo,
) -> Result<DirEntry, u16> {
    if find_entry(vol, parent_cluster, &fcb_name, disk)?.is_some() {
        return Err(0x0005);
    }

    let new_cluster = vol.allocate_cluster(0).ok_or(0x0005u16)?;
    let cluster_size = vol.bpb.sectors_per_cluster as usize * vol.bpb.bytes_per_sector as usize;
    let zeros = vec![0u8; cluster_size];
    vol.write_cluster(new_cluster, &zeros, disk).map_err(|_| {
        vol.free_chain(new_cluster);
        0x001Fu16
    })?;

    let dot_entry = DirEntry {
        name: *b".          ",
        attribute: ATTR_DIRECTORY,
        time,
        date,
        start_cluster: new_cluster,
        file_size: 0,
        dir_sector: 0,
        dir_offset: 0,
    };
    if let Err(error) = create_entry(vol, new_cluster, &dot_entry, disk) {
        vol.free_chain(new_cluster);
        let _ = vol.flush_fat(disk);
        return Err(error);
    }

    let dotdot_entry = DirEntry {
        name: *b"..         ",
        attribute: ATTR_DIRECTORY,
        time,
        date,
        start_cluster: parent_cluster,
        file_size: 0,
        dir_sector: 0,
        dir_offset: 0,
    };
    if let Err(error) = create_entry(vol, new_cluster, &dotdot_entry, disk) {
        vol.free_chain(new_cluster);
        let _ = vol.flush_fat(disk);
        return Err(error);
    }

    let dir_entry = DirEntry {
        name: fcb_name,
        attribute: ATTR_DIRECTORY,
        time,
        date,
        start_cluster: new_cluster,
        file_size: 0,
        dir_sector: 0,
        dir_offset: 0,
    };
    let created = match create_entry(vol, parent_cluster, &dir_entry, disk) {
        Ok(entry) => entry,
        Err(error) => {
            vol.free_chain(new_cluster);
            let _ = vol.flush_fat(disk);
            return Err(error);
        }
    };

    vol.flush_fat(disk).map_err(|_| 0x001Fu16)?;
    Ok(created)
}
