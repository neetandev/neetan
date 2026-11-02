use std::ops::Range;

use crate::{CdromIo, filesystem::fat_dir};

const ISO_SECTOR_SIZE: usize = 2048;
const PVD_LBA: u32 = 16;

/// Recording format of a CD-ROM volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VolumeFormat {
    /// ISO 9660 (`CD001` at offset 1 of the volume descriptor).
    Iso9660,
    /// High Sierra (`CDROM` at offset 9 of the volume descriptor).
    HighSierra,
}

/// Save state tag of `VolumeFormat::Iso9660`.
const FORMAT_TAG_ISO9660: u8 = 0;
/// Save state tag of `VolumeFormat::HighSierra`.
const FORMAT_TAG_HIGH_SIERRA: u8 = 1;

impl VolumeFormat {
    /// Returns the format of the primary volume descriptor `sector`.
    pub(crate) fn of_primary_descriptor(sector: &[u8]) -> Option<Self> {
        if sector.len() < ISO_SECTOR_SIZE {
            return None;
        }
        if sector[0] == 1 && &sector[1..6] == b"CD001" && sector[6] == 1 {
            return Some(Self::Iso9660);
        }
        if sector[8] == 1 && &sector[9..14] == b"CDROM" && sector[14] == 1 {
            return Some(Self::HighSierra);
        }
        None
    }

    /// Byte range of the volume identifier in the volume descriptor.
    fn volume_label_range(self) -> Range<usize> {
        match self {
            Self::Iso9660 => 40..72,
            Self::HighSierra => 48..80,
        }
    }

    /// Offset of the little endian volume space size in the volume
    /// descriptor.
    fn volume_space_size_offset(self) -> usize {
        match self {
            Self::Iso9660 => 80,
            Self::HighSierra => 88,
        }
    }

    /// Offset of the root directory record in the volume descriptor.
    fn root_record_offset(self) -> usize {
        match self {
            Self::Iso9660 => 156,
            Self::HighSierra => 180,
        }
    }

    /// Offset of the file flags in a directory record. The recording date
    /// fills the bytes from offset 18 up to the flags.
    fn flags_offset(self) -> usize {
        match self {
            Self::Iso9660 => 25,
            Self::HighSierra => 24,
        }
    }

    /// Returns the volume space size in sectors of the primary volume
    /// descriptor `sector`.
    pub(crate) fn volume_space_size(self, sector: &[u8]) -> u32 {
        let offset = self.volume_space_size_offset();
        u32::from_le_bytes(sector[offset..offset + 4].try_into().expect("four bytes"))
    }
}

impl save_state::StateEncode for VolumeFormat {
    fn encode_state(&self, output: &mut Vec<u8>) {
        let tag = match self {
            Self::Iso9660 => FORMAT_TAG_ISO9660,
            Self::HighSierra => FORMAT_TAG_HIGH_SIERRA,
        };
        save_state::StateEncode::encode_state(&tag, output);
    }
}

impl save_state::StateDecode for VolumeFormat {
    fn decode_state(
        decoder: &mut save_state::StateDecoder<'_>,
    ) -> Result<Self, save_state::StateDecodeError> {
        match <u8 as save_state::StateDecode>::decode_state(decoder)? {
            FORMAT_TAG_ISO9660 => Ok(Self::Iso9660),
            FORMAT_TAG_HIGH_SIERRA => Ok(Self::HighSierra),
            _ => Err(save_state::StateDecodeError::InvalidTag),
        }
    }
}

#[derive(Debug, Clone)]
/// Mounted ISO 9660 or High Sierra volume geometry used by HLE DOS.
pub(crate) struct IsoVolume {
    pub format: VolumeFormat,
    pub volume_label: Vec<u8>,
    pub root_directory: IsoDirectory,
}

state_struct_codec!(IsoVolume {
    format,
    volume_label,
    root_directory,
});

#[derive(Debug, Clone)]
/// Parsed ISO-9660 directory location and size.
pub(crate) struct IsoDirectory {
    pub start_lba: u32,
    pub data_length: u32,
}

state_struct_codec!(IsoDirectory {
    start_lba,
    data_length,
});

#[derive(Debug, Clone)]
/// Parsed ISO-9660 directory entry returned to HLE DOS.
pub(crate) struct IsoDirEntry {
    pub name: [u8; 11],
    pub attribute: u8,
    pub time: u16,
    pub date: u16,
    pub start_lba: u32,
    pub file_size: u32,
    pub directory: Option<IsoDirectory>,
}

state_struct_codec!(IsoDirEntry {
    name,
    attribute,
    time,
    date,
    start_lba,
    file_size,
    directory,
});

impl IsoVolume {
    pub(crate) fn mount(cdrom: &dyn CdromIo) -> Result<Self, u16> {
        if !cdrom.cdrom_media_loaded() {
            return Err(0x0015);
        }

        let mut sector = [0u8; ISO_SECTOR_SIZE];
        let count = cdrom
            .read_sector_cooked(PVD_LBA, &mut sector)
            .ok_or(0x001Fu16)?;
        if count < ISO_SECTOR_SIZE {
            return Err(0x001F);
        }
        let format = VolumeFormat::of_primary_descriptor(&sector).ok_or(0x001Fu16)?;

        let volume_label = trim_trailing_spaces(&sector[format.volume_label_range()]).to_vec();
        let root_directory = parse_directory_record(&sector[format.root_record_offset()..], format)
            .ok_or(0x001Fu16)?
            .directory
            .ok_or(0x001Fu16)?;

        Ok(Self {
            format,
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
    for_each_entry(volume, directory, cdrom, |entry, _| {
        if entry.name == *name {
            return IterAction::Return(entry);
        }
        IterAction::Continue
    })
    .map(|found| found.map(|(entry, _)| entry))
}

/// Returns the entry `name` of `directory` with the bytes of its directory
/// record.
pub(crate) fn find_record(
    volume: &IsoVolume,
    directory: &IsoDirectory,
    name: &[u8; 11],
    cdrom: &dyn CdromIo,
) -> Result<Option<(IsoDirEntry, Vec<u8>)>, u16> {
    for_each_entry(volume, directory, cdrom, |entry, _| {
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
    if attr_mask == fat_dir::ATTR_VOLUME_ID {
        return Ok(volume_label_entry(volume, directory, pattern, start_index));
    }
    let mut current_index = 0u16;
    let result = for_each_entry(volume, directory, cdrom, |entry, _| {
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

    Ok(result.map(|(entry, _)| (entry, current_index)))
}

/// Returns the volume label as the only entry of a search for volume labels
/// in the root directory. The label fills the 11 bytes of the name.
fn volume_label_entry(
    volume: &IsoVolume,
    directory: &IsoDirectory,
    pattern: &[u8; 11],
    start_index: u16,
) -> Option<(IsoDirEntry, u16)> {
    if start_index != 0
        || volume.volume_label.is_empty()
        || directory.start_lba != volume.root_directory.start_lba
    {
        return None;
    }
    let mut name = [b' '; 11];
    let length = volume.volume_label.len().min(name.len());
    name[..length].copy_from_slice(&volume.volume_label[..length]);
    if !fat_dir::matches_pattern(&name, pattern) {
        return None;
    }
    let entry = IsoDirEntry {
        name,
        attribute: fat_dir::ATTR_VOLUME_ID,
        time: 0,
        date: 0,
        start_lba: 0,
        file_size: 0,
        directory: None,
    };
    Some((entry, 1))
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

/// Calls `callback` for every entry of `directory` except `.` and `..` with
/// the parsed entry and its record bytes. Returns the entry that the
/// callback returns, with its record bytes.
fn for_each_entry(
    volume: &IsoVolume,
    directory: &IsoDirectory,
    cdrom: &dyn CdromIo,
    mut callback: impl FnMut(IsoDirEntry, &[u8]) -> IterAction,
) -> Result<Option<(IsoDirEntry, Vec<u8>)>, u16> {
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

            let record = &sector[offset..offset + record_len];
            if let Some(entry) = parse_directory_record(record, volume.format)
                && !is_self_or_parent(&entry)
            {
                match callback(entry, record) {
                    IterAction::Continue => {}
                    IterAction::Return(entry) => return Ok(Some((entry, record.to_vec()))),
                }
            }

            offset += record_len;
        }
    }

    Ok(None)
}

/// Parses the directory record `record` of a volume in `format`.
fn parse_directory_record(record: &[u8], format: VolumeFormat) -> Option<IsoDirEntry> {
    if record.len() < 34 {
        return None;
    }

    let name_length = record[32] as usize;
    if record.len() < 33 + name_length {
        return None;
    }

    let start_lba = u32::from_le_bytes(record[2..6].try_into().ok()?);
    let file_size = u32::from_le_bytes(record[10..14].try_into().ok()?);
    let flags = record[format.flags_offset()];
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
    let (time, date) = iso_datetime_to_dos(&record[18..24]);

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

/// Converts the first six bytes (year since 1900, month, day, hour, minute,
/// second) of a recording date to DOS time and date.
fn iso_datetime_to_dos(recording_time: &[u8]) -> (u16, u16) {
    if recording_time.len() < 6 {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ImageCdrom;

    /// Writes a directory record of `format` at `offset` and returns its
    /// length.
    fn write_record(
        sector: &mut [u8],
        offset: usize,
        format: VolumeFormat,
        identifier: &[u8],
        extent: u32,
        size: u32,
        flags: u8,
    ) -> usize {
        let length = (33 + identifier.len() + 1) & !1;
        let record = &mut sector[offset..offset + length];
        record[0] = length as u8;
        record[2..6].copy_from_slice(&extent.to_le_bytes());
        record[6..10].copy_from_slice(&extent.to_be_bytes());
        record[10..14].copy_from_slice(&size.to_le_bytes());
        record[14..18].copy_from_slice(&size.to_be_bytes());
        record[18..24].copy_from_slice(&[90, 10, 3, 14, 26, 48]);
        record[format.flags_offset()] = flags;
        record[32] = identifier.len() as u8;
        record[33..33 + identifier.len()].copy_from_slice(identifier);
        length
    }

    /// Returns a volume of `format` with the root directory in sector 20
    /// and the file `RUN386.EXE;1` in sector 21.
    fn volume_image(format: VolumeFormat) -> Vec<u8> {
        let mut image = vec![0u8; 22 * ISO_SECTOR_SIZE];
        let descriptor = &mut image[16 * ISO_SECTOR_SIZE..17 * ISO_SECTOR_SIZE];
        match format {
            VolumeFormat::Iso9660 => {
                descriptor[0] = 1;
                descriptor[1..6].copy_from_slice(b"CD001");
                descriptor[6] = 1;
            }
            VolumeFormat::HighSierra => {
                descriptor[8] = 1;
                descriptor[9..14].copy_from_slice(b"CDROM");
                descriptor[14] = 1;
            }
        }
        let label = format.volume_label_range();
        descriptor[label.clone()].fill(b' ');
        descriptor[label.start..label.start + 4].copy_from_slice(b"MENU");
        let size_offset = format.volume_space_size_offset();
        descriptor[size_offset..size_offset + 4].copy_from_slice(&22u32.to_le_bytes());
        write_record(
            descriptor,
            format.root_record_offset(),
            format,
            &[0],
            20,
            ISO_SECTOR_SIZE as u32,
            0x02,
        );
        let directory = &mut image[20 * ISO_SECTOR_SIZE..21 * ISO_SECTOR_SIZE];
        let mut offset = write_record(directory, 0, format, &[0], 20, 2048, 0x02);
        offset += write_record(directory, offset, format, &[1], 20, 2048, 0x02);
        write_record(directory, offset, format, b"RUN386.EXE;1", 21, 5, 0x00);
        image[21 * ISO_SECTOR_SIZE..21 * ISO_SECTOR_SIZE + 5].copy_from_slice(b"MZ386");
        image
    }

    #[test]
    fn mounts_and_reads_an_iso9660_and_a_high_sierra_volume() {
        for format in [VolumeFormat::Iso9660, VolumeFormat::HighSierra] {
            let cdrom = ImageCdrom(volume_image(format));
            let volume = IsoVolume::mount(&cdrom).expect("volume mounts");
            assert_eq!(volume.format, format);
            assert_eq!(volume.volume_label, b"MENU");
            assert_eq!(volume.root_directory.start_lba, 20);
            let entry = find_entry(
                &volume,
                &volume.root_directory,
                &fat_dir::name_to_fcb(b"RUN386.EXE"),
                &cdrom,
            )
            .expect("directory reads")
            .expect("file exists");
            assert_eq!(entry.attribute, fat_dir::ATTR_READ_ONLY);
            assert_eq!(entry.date, (10 << 9) | (10 << 5) | 3);
            assert_eq!(read_all(&entry, &cdrom).expect("file reads"), b"MZ386");
        }
    }

    #[test]
    fn a_volume_label_search_returns_only_the_label() {
        let cdrom = ImageCdrom(volume_image(VolumeFormat::HighSierra));
        let volume = IsoVolume::mount(&cdrom).expect("volume mounts");
        let (entry, next_index) = find_matching(
            &volume,
            &volume.root_directory,
            b"???????????",
            fat_dir::ATTR_VOLUME_ID,
            0,
            &cdrom,
        )
        .expect("search runs")
        .expect("label found");
        assert_eq!(&entry.name, b"MENU       ");
        assert_eq!(entry.attribute, fat_dir::ATTR_VOLUME_ID);
        let next = find_matching(
            &volume,
            &volume.root_directory,
            b"???????????",
            fat_dir::ATTR_VOLUME_ID,
            next_index,
            &cdrom,
        )
        .expect("search runs");
        assert!(next.is_none());
    }

    #[test]
    fn volume_space_size_follows_the_format() {
        for format in [VolumeFormat::Iso9660, VolumeFormat::HighSierra] {
            let image = volume_image(format);
            let descriptor = &image[16 * ISO_SECTOR_SIZE..17 * ISO_SECTOR_SIZE];
            assert_eq!(
                VolumeFormat::of_primary_descriptor(descriptor),
                Some(format)
            );
            assert_eq!(format.volume_space_size(descriptor), 22);
        }
        assert_eq!(
            VolumeFormat::of_primary_descriptor(&[0; ISO_SECTOR_SIZE]),
            None
        );
    }
}
