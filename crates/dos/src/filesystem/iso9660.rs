use crate::{CdromIo, filesystem::fat_dir};

const ISO_SECTOR_SIZE: usize = 2048;
const PVD_LBA: u32 = 16;
const PVD_ROOT_RECORD_OFFSET: usize = 156;

#[derive(Debug, Clone)]
pub(crate) struct IsoVolume {
    pub volume_label: Vec<u8>,
    pub root_directory: IsoDirectory,
}

#[derive(Debug, Clone)]
pub(crate) struct IsoDirectory {
    pub start_lba: u32,
    pub data_length: u32,
}

#[derive(Debug, Clone)]
pub(crate) struct IsoDirEntry {
    pub name: [u8; 11],
    pub attribute: u8,
    pub time: u16,
    pub date: u16,
    pub start_lba: u32,
    pub file_size: u32,
    pub directory: Option<IsoDirectory>,
}

impl IsoVolume {
    pub(crate) fn mount(cdrom: &dyn CdromIo) -> Result<Self, u16> {
        if !cdrom.cdrom_media_loaded() {
            return Err(0x0015);
        }

        let mut sector = [0u8; ISO_SECTOR_SIZE];
        let count = cdrom
            .read_sector_cooked(PVD_LBA, &mut sector)
            .ok_or(0x001Fu16)?;
        if count < ISO_SECTOR_SIZE || sector[0] != 1 || &sector[1..6] != b"CD001" || sector[6] != 1
        {
            return Err(0x001F);
        }

        let volume_label = trim_trailing_spaces(&sector[40..72]).to_vec();
        let root_directory = parse_directory_record(&sector[PVD_ROOT_RECORD_OFFSET..])
            .ok_or(0x001Fu16)?
            .directory
            .ok_or(0x001Fu16)?;

        Ok(Self {
            volume_label,
            root_directory,
        })
    }
}

pub(crate) fn find_entry(
    volume: &IsoVolume,
    directory: &IsoDirectory,
    name: &[u8; 11],
    cdrom: &dyn CdromIo,
) -> Result<Option<IsoDirEntry>, u16> {
    for_each_entry(volume, directory, cdrom, |entry| {
        if entry.name == *name {
            return IterAction::Return(entry);
        }
        IterAction::Continue
    })
}

pub(crate) fn find_matching(
    volume: &IsoVolume,
    directory: &IsoDirectory,
    pattern: &[u8; 11],
    attr_mask: u8,
    start_index: u16,
    cdrom: &dyn CdromIo,
) -> Result<Option<(IsoDirEntry, u16)>, u16> {
    let mut current_index = 0u16;
    let result = for_each_entry(volume, directory, cdrom, |entry| {
        if current_index < start_index {
            current_index += 1;
            return IterAction::Continue;
        }
        current_index += 1;

        if entry.attribute & fat_dir::ATTR_HIDDEN != 0 && attr_mask & fat_dir::ATTR_HIDDEN == 0 {
            return IterAction::Continue;
        }
        if entry.attribute & fat_dir::ATTR_SYSTEM != 0 && attr_mask & fat_dir::ATTR_SYSTEM == 0 {
            return IterAction::Continue;
        }
        if entry.attribute & fat_dir::ATTR_DIRECTORY != 0
            && attr_mask & fat_dir::ATTR_DIRECTORY == 0
        {
            return IterAction::Continue;
        }

        if fat_dir::matches_pattern(&entry.name, pattern) {
            return IterAction::Return(entry);
        }
        IterAction::Continue
    })?;

    Ok(result.map(|entry| (entry, current_index)))
}

pub(crate) fn read_file_chunk(
    entry: &IsoDirEntry,
    position: u32,
    max_bytes: usize,
    cdrom: &dyn CdromIo,
) -> Result<Vec<u8>, u16> {
    if position >= entry.file_size || max_bytes == 0 {
        return Ok(Vec::new());
    }

    let bytes_to_read = (max_bytes as u32).min(entry.file_size - position) as usize;
    let start_lba = entry.start_lba + position / ISO_SECTOR_SIZE as u32;
    let end_position = position as usize + bytes_to_read;
    let end_lba = entry.start_lba + end_position.div_ceil(ISO_SECTOR_SIZE) as u32;
    let mut result = Vec::with_capacity((end_lba - start_lba) as usize * ISO_SECTOR_SIZE);

    for lba in start_lba..end_lba {
        let mut sector = [0u8; ISO_SECTOR_SIZE];
        let count = cdrom
            .read_sector_cooked(lba, &mut sector)
            .ok_or(0x001Fu16)?;
        result.extend_from_slice(&sector[..count.min(ISO_SECTOR_SIZE)]);
    }

    let offset = position as usize % ISO_SECTOR_SIZE;
    let end = offset + bytes_to_read;
    Ok(result[offset..end].to_vec())
}

pub(crate) fn read_all(entry: &IsoDirEntry, cdrom: &dyn CdromIo) -> Result<Vec<u8>, u16> {
    read_file_chunk(entry, 0, entry.file_size as usize, cdrom)
}

enum IterAction {
    Continue,
    Return(IsoDirEntry),
}

fn for_each_entry(
    _volume: &IsoVolume,
    directory: &IsoDirectory,
    cdrom: &dyn CdromIo,
    mut callback: impl FnMut(IsoDirEntry) -> IterAction,
) -> Result<Option<IsoDirEntry>, u16> {
    if !cdrom.cdrom_media_loaded() {
        return Err(0x0015);
    }

    let sector_count = directory.data_length.div_ceil(ISO_SECTOR_SIZE as u32);
    for sector_index in 0..sector_count {
        let mut sector = [0u8; ISO_SECTOR_SIZE];
        let count = cdrom
            .read_sector_cooked(directory.start_lba + sector_index, &mut sector)
            .ok_or(0x001Fu16)?;
        let limit = count.min(ISO_SECTOR_SIZE);
        let mut offset = 0usize;

        while offset < limit {
            let record_len = sector[offset] as usize;
            if record_len == 0 {
                break;
            }

            if offset + record_len > limit {
                return Err(0x001F);
            }

            if let Some(entry) = parse_directory_record(&sector[offset..offset + record_len])
                && !is_self_or_parent(&entry)
            {
                match callback(entry) {
                    IterAction::Continue => {}
                    IterAction::Return(entry) => return Ok(Some(entry)),
                }
            }

            offset += record_len;
        }
    }

    Ok(None)
}

fn parse_directory_record(record: &[u8]) -> Option<IsoDirEntry> {
    if record.len() < 34 {
        return None;
    }

    let name_length = record[32] as usize;
    if record.len() < 33 + name_length {
        return None;
    }

    let start_lba = u32::from_le_bytes(record[2..6].try_into().ok()?);
    let file_size = u32::from_le_bytes(record[10..14].try_into().ok()?);
    let flags = record[25];
    let is_directory = flags & 0x02 != 0;
    let identifier = &record[33..33 + name_length];
    let name = match identifier {
        [0] => *b".          ",
        [1] => *b"..         ",
        _ => {
            let display_name = iso_identifier_to_name(identifier, is_directory)?;
            fat_dir::name_to_fcb(&display_name)
        }
    };
    let attribute = if is_directory {
        fat_dir::ATTR_DIRECTORY
    } else {
        fat_dir::ATTR_READ_ONLY
    };
    let (time, date) = iso_datetime_to_dos(&record[18..25]);

    Some(IsoDirEntry {
        name,
        attribute,
        time,
        date,
        start_lba,
        file_size,
        directory: is_directory.then_some(IsoDirectory {
            start_lba,
            data_length: file_size,
        }),
    })
}

fn iso_identifier_to_name(identifier: &[u8], is_directory: bool) -> Option<Vec<u8>> {
    match identifier {
        [0] => Some(b".".to_vec()),
        [1] => Some(b"..".to_vec()),
        _ => {
            let mut name = identifier.to_vec();
            if !is_directory
                && let Some(version_separator) = name.iter().position(|&byte| byte == b';')
            {
                name.truncate(version_separator);
            }
            while name.last() == Some(&b'.') {
                name.pop();
            }
            Some(name)
        }
    }
}

fn iso_datetime_to_dos(recording_time: &[u8]) -> (u16, u16) {
    if recording_time.len() < 7 {
        return (0, 0);
    }

    let year = 1900u16.saturating_add(recording_time[0] as u16);
    let month = recording_time[1].clamp(1, 12) as u16;
    let day = recording_time[2].clamp(1, 31) as u16;
    let hour = recording_time[3].min(23) as u16;
    let minute = recording_time[4].min(59) as u16;
    let second = recording_time[5].min(59) as u16;

    let dos_date = year.saturating_sub(1980).saturating_mul(512) | (month << 5) | day;
    let dos_time = (hour << 11) | (minute << 5) | (second / 2);
    (dos_time, dos_date)
}

fn trim_trailing_spaces(bytes: &[u8]) -> &[u8] {
    let mut end = bytes.len();
    while end > 0 && bytes[end - 1] == b' ' {
        end -= 1;
    }
    &bytes[..end]
}

fn is_self_or_parent(entry: &IsoDirEntry) -> bool {
    entry.name == *b".          " || entry.name == *b"..         "
}
