use crate::{
    DiskIo,
    filesystem::{
        fat::FatVolume,
        fat_dir::{self, DirEntry},
    },
};

#[derive(Clone)]
/// Authoritative FAT cluster-chain and byte-offset cursor.
pub(crate) struct FatFileCursor {
    start_cluster: u16,
    file_size: u32,
    position: u32,
    cached_cluster_index: u32,
    cached_cluster: u16,
}

state_struct_codec!(FatFileCursor {
    start_cluster,
    file_size,
    position,
    cached_cluster_index,
    cached_cluster,
});

impl FatFileCursor {
    pub(crate) fn new(entry: &DirEntry) -> Self {
        Self::with_position(entry.start_cluster, entry.file_size, 0)
    }

    pub(crate) fn with_position(start_cluster: u16, file_size: u32, position: u32) -> Self {
        Self {
            start_cluster,
            file_size,
            position,
            cached_cluster_index: 0,
            cached_cluster: 0,
        }
    }

    pub(crate) fn position(&self) -> u32 {
        self.position
    }

    pub(crate) fn remaining(&self) -> u32 {
        self.file_size.saturating_sub(self.position)
    }

    pub(crate) fn read_chunk(
        &mut self,
        vol: &FatVolume,
        disk: &mut dyn DiskIo,
        max_bytes: usize,
    ) -> Result<Vec<u8>, u16> {
        if self.position >= self.file_size || max_bytes == 0 {
            return Ok(Vec::new());
        }
        if self.start_cluster < 2 {
            return Err(0x001F);
        }

        let cluster_size = vol.bpb.cluster_size();
        let bytes_to_read = (max_bytes as u32).min(self.remaining());
        let mut result = Vec::with_capacity(bytes_to_read as usize);

        while result.len() < bytes_to_read as usize {
            let cluster_index = self.position / cluster_size;
            let offset_in_cluster = self.position % cluster_size;
            let cluster = self.cluster_for_index(vol, cluster_index)?;
            let cluster_data = vol.read_cluster(cluster, disk)?;

            let available = (cluster_size - offset_in_cluster)
                .min(bytes_to_read - result.len() as u32) as usize;
            let start = offset_in_cluster as usize;
            let end = start + available;
            result.extend_from_slice(&cluster_data[start..end]);
            self.position += available as u32;
        }

        Ok(result)
    }

    fn cluster_for_index(&mut self, vol: &FatVolume, target_index: u32) -> Result<u16, u16> {
        if self.start_cluster < 2 {
            return Err(0x001F);
        }
        if target_index == 0 {
            self.cached_cluster_index = 0;
            self.cached_cluster = self.start_cluster;
            return Ok(self.start_cluster);
        }

        let (mut current_index, mut current_cluster) =
            if self.cached_cluster >= 2 && target_index >= self.cached_cluster_index {
                (self.cached_cluster_index, self.cached_cluster)
            } else {
                (0, self.start_cluster)
            };

        while current_index < target_index {
            current_cluster = vol.next_cluster(current_cluster).ok_or(0x001Fu16)?;
            current_index += 1;
        }

        self.cached_cluster_index = current_index;
        self.cached_cluster = current_cluster;
        Ok(current_cluster)
    }
}

pub(crate) struct FatFileWriter {
    start_cluster: u16,
    position: u32,
    cached_cluster_index: u32,
    cached_cluster: u16,
}

impl FatFileWriter {
    pub(crate) fn new(start_cluster: u16, position: u32) -> Self {
        Self {
            start_cluster,
            position,
            cached_cluster_index: 0,
            cached_cluster: 0,
        }
    }

    pub(crate) fn start_cluster(&self) -> u16 {
        self.start_cluster
    }

    pub(crate) fn position(&self) -> u32 {
        self.position
    }

    pub(crate) fn current_cluster(&self) -> u16 {
        if self.cached_cluster >= 2 {
            self.cached_cluster
        } else {
            self.start_cluster
        }
    }

    pub(crate) fn write_chunk(
        &mut self,
        vol: &mut FatVolume,
        disk: &mut dyn DiskIo,
        data: &[u8],
    ) -> Result<(), u16> {
        if data.is_empty() {
            return Ok(());
        }

        let cluster_size = vol.bpb.cluster_size() as usize;
        let mut offset = 0usize;
        while offset < data.len() {
            let cluster_index = self.position / cluster_size as u32;
            let offset_in_cluster = (self.position % cluster_size as u32) as usize;
            let cluster = self.cluster_for_index(vol, disk, cluster_index)?;
            let bytes_to_write = (cluster_size - offset_in_cluster).min(data.len() - offset);

            if offset_in_cluster == 0 && bytes_to_write == cluster_size {
                vol.write_cluster(cluster, &data[offset..offset + bytes_to_write], disk)?;
            } else {
                let mut cluster_data = vol.read_cluster(cluster, disk)?;
                cluster_data[offset_in_cluster..offset_in_cluster + bytes_to_write]
                    .copy_from_slice(&data[offset..offset + bytes_to_write]);
                vol.write_cluster(cluster, &cluster_data, disk)?;
            }

            self.position += bytes_to_write as u32;
            offset += bytes_to_write;
        }

        Ok(())
    }

    fn cluster_for_index(
        &mut self,
        vol: &mut FatVolume,
        disk: &mut dyn DiskIo,
        target_index: u32,
    ) -> Result<u16, u16> {
        if self.start_cluster < 2 {
            let first_cluster = vol.allocate_cluster(0).ok_or(0x001Fu16)?;
            zero_cluster(vol, disk, first_cluster)?;
            self.start_cluster = first_cluster;
            self.cached_cluster_index = 0;
            self.cached_cluster = first_cluster;
            if target_index == 0 {
                return Ok(first_cluster);
            }
        }

        let (mut current_index, mut current_cluster) =
            if self.cached_cluster >= 2 && target_index >= self.cached_cluster_index {
                (self.cached_cluster_index, self.cached_cluster)
            } else {
                (0, self.start_cluster)
            };

        while current_index < target_index {
            if let Some(next_cluster) = vol.next_cluster(current_cluster) {
                current_cluster = next_cluster;
            } else {
                let new_cluster = vol.allocate_cluster(current_cluster).ok_or(0x001Fu16)?;
                zero_cluster(vol, disk, new_cluster)?;
                current_cluster = new_cluster;
            }
            current_index += 1;
        }

        self.cached_cluster_index = current_index;
        self.cached_cluster = current_cluster;
        Ok(current_cluster)
    }
}

fn zero_cluster(vol: &FatVolume, disk: &mut dyn DiskIo, cluster: u16) -> Result<(), u16> {
    let zeros = vec![0u8; vol.bpb.cluster_size() as usize];
    vol.write_cluster(cluster, &zeros, disk)
}

pub(crate) fn read_all(
    vol: &FatVolume,
    entry: &DirEntry,
    disk: &mut dyn DiskIo,
) -> Result<Vec<u8>, u16> {
    read_chain(vol, entry.start_cluster, entry.file_size, disk)
}

/// Reads `file_size` bytes of the cluster chain that starts at
/// `start_cluster`.
pub(crate) fn read_chain(
    vol: &FatVolume,
    start_cluster: u16,
    file_size: u32,
    disk: &mut dyn DiskIo,
) -> Result<Vec<u8>, u16> {
    if file_size == 0 {
        return Ok(Vec::new());
    }

    let mut cursor = FatFileCursor::with_position(start_cluster, file_size, 0);
    cursor.read_chunk(vol, disk, file_size as usize)
}

#[derive(Clone, Copy)]
pub(crate) struct FileCreateOptions {
    pub(crate) attributes: u8,
    pub(crate) time: u16,
    pub(crate) date: u16,
}

/// Writes a file into a directory. An existing file keeps its directory
/// slot and its reserved entry bytes: its old chain is freed and its entry
/// is updated in place. A failed write deletes the existing entry. A new
/// file gets a new entry.
pub(crate) fn create_or_replace_file(
    vol: &mut FatVolume,
    dir_cluster: u16,
    fcb_name: &[u8; 11],
    data: &[u8],
    options: FileCreateOptions,
    disk: &mut dyn DiskIo,
) -> Result<(), u16> {
    let existing = fat_dir::find_entry(vol, dir_cluster, fcb_name, disk)?;
    if let Some(existing) = &existing {
        if existing.attribute & fat_dir::ATTR_DIRECTORY != 0 {
            return Err(0x0005);
        }
        if existing.start_cluster >= 2 {
            vol.free_chain(existing.start_cluster);
        }
    }

    let mut writer = FatFileWriter::new(0, 0);
    if let Err(error) = writer.write_chunk(vol, disk, data) {
        if writer.start_cluster() >= 2 {
            vol.free_chain(writer.start_cluster());
        }
        if let Some(existing) = &existing {
            let _ = fat_dir::delete_entry(vol, existing, disk);
        }
        return Err(error);
    }

    if let Some(existing) = existing {
        return update_file_entry_in_place(
            vol,
            existing,
            options,
            writer.start_cluster(),
            writer.position(),
            disk,
        );
    }

    let new_entry = DirEntry {
        name: *fcb_name,
        attribute: options.attributes,
        reserved: [0; fat_dir::RESERVED_LENGTH],
        time: options.time,
        date: options.date,
        start_cluster: writer.start_cluster(),
        file_size: writer.position(),
        dir_sector: 0,
        dir_offset: 0,
    };

    if let Err(error) = fat_dir::create_entry(vol, dir_cluster, &new_entry, disk) {
        if writer.start_cluster() >= 2 {
            vol.free_chain(writer.start_cluster());
        }
        return Err(error);
    }

    Ok(())
}

/// Updates an existing file entry in its slot with a new chain: the name
/// and the reserved bytes stay. A failed update frees the new chain and
/// deletes the entry.
pub(crate) fn update_file_entry_in_place(
    vol: &mut FatVolume,
    existing: DirEntry,
    options: FileCreateOptions,
    start_cluster: u16,
    file_size: u32,
    disk: &mut dyn DiskIo,
) -> Result<(), u16> {
    let updated = DirEntry {
        attribute: options.attributes,
        time: options.time,
        date: options.date,
        start_cluster,
        file_size,
        ..existing
    };
    if let Err(error) = fat_dir::update_entry(vol, &updated, disk) {
        if start_cluster >= 2 {
            vol.free_chain(start_cluster);
        }
        let _ = fat_dir::delete_entry(vol, &updated, disk);
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        filesystem::fat::FatVolume,
        test_support::{
            FLOPPY_2HD_DATA_CLUSTER_COUNT, FLOPPY_2HD_DEVICE_ID, FLOPPY_2HD_ROOT_SECTOR, MockDisk,
            directory_entry_bytes, formatted_2hd_floppy, human68k_2hd_boot_sector,
            set_2hd_fat_entries, set_fat12_entry, store_2hd_single_cluster, store_directory_entry,
        },
    };

    /// Mounts a 2HD floppy with the PC-98 2HD drive rule.
    fn mount_2hd(disk: &mut MockDisk) -> FatVolume {
        FatVolume::mount(
            FLOPPY_2HD_DEVICE_ID,
            0,
            &crate::drive_map::pc98::floppy_2hd_geometry(),
            disk,
        )
        .unwrap()
    }

    /// Options of a replaced file.
    const REPLACE_OPTIONS: FileCreateOptions = FileCreateOptions {
        attributes: fat_dir::ATTR_ARCHIVE | fat_dir::ATTR_READ_ONLY,
        time: 0x1234,
        date: 0x5678,
    };

    /// Payload of three clusters of a 2HD floppy.
    fn three_cluster_payload() -> Vec<u8> {
        (0..3000).map(|index| (index % 251) as u8).collect()
    }

    fn mount_fat12_disk(
        start_cluster: u16,
        file_size: u32,
    ) -> (MockDisk, FatVolume, DirEntry, Vec<u8>) {
        let device_id = 0x90;
        let sector_size = 1024u16;
        let reserved = 1u16;
        let sectors_per_fat = 2u16;
        let root_entries = 16u16;
        let root_dir_sectors = 1u32;
        let data_clusters = 1221u32;
        let total_sectors =
            reserved as u32 + sectors_per_fat as u32 + root_dir_sectors + data_clusters;
        let mut disk = MockDisk::new(device_id, sector_size, 8, 2, total_sectors);

        let mut boot = vec![0u8; sector_size as usize];
        boot[11..13].copy_from_slice(&sector_size.to_le_bytes());
        boot[13] = 1;
        boot[14..16].copy_from_slice(&reserved.to_le_bytes());
        boot[16] = 1;
        boot[17..19].copy_from_slice(&root_entries.to_le_bytes());
        boot[19..21].copy_from_slice(&(total_sectors as u16).to_le_bytes());
        boot[22..24].copy_from_slice(&sectors_per_fat.to_le_bytes());
        disk.write_sector(0, &boot);

        let mut fat = vec![0u8; sectors_per_fat as usize * sector_size as usize];
        fat[0] = 0xFE;
        fat[1] = 0xFF;
        fat[2] = 0xFF;
        set_fat12_entry(&mut fat, start_cluster, start_cluster + 1);
        set_fat12_entry(&mut fat, start_cluster + 1, start_cluster + 2);
        set_fat12_entry(&mut fat, start_cluster + 2, 0x0FFF);
        for sector_index in 0..sectors_per_fat as u32 {
            let offset = sector_index as usize * sector_size as usize;
            disk.write_sector(
                1 + sector_index,
                &fat[offset..offset + sector_size as usize],
            );
        }

        let mut root = vec![0u8; sector_size as usize];
        root[0..11].copy_from_slice(b"BIGFILE BIN");
        root[26..28].copy_from_slice(&start_cluster.to_le_bytes());
        root[28..32].copy_from_slice(&file_size.to_le_bytes());
        disk.write_sector(3, &root);

        let payload: Vec<u8> = (0..file_size).map(|index| (index & 0xFF) as u8).collect();
        for cluster_offset in 0..3u32 {
            let lba = 4 + (start_cluster as u32 - 2) + cluster_offset;
            let start = cluster_offset as usize * sector_size as usize;
            let end = (start + sector_size as usize).min(payload.len());
            let mut cluster = vec![0u8; sector_size as usize];
            cluster[..end - start].copy_from_slice(&payload[start..end]);
            disk.write_sector(lba, &cluster);
        }

        let mut mount_disk = MockDisk {
            device_id: disk.device_id,
            sector_size: disk.sector_size,
            sectors_per_track: disk.sectors_per_track,
            heads: disk.heads,
            data: disk.data.clone(),
        };
        let volume = FatVolume::mount(
            device_id,
            0,
            &crate::drive_map::pc98::fixed_disk_geometry(),
            &mut mount_disk,
        )
        .expect("mount FAT12");
        let entry = DirEntry {
            name: *b"BIGFILE BIN",
            attribute: 0x20,
            reserved: [0; fat_dir::RESERVED_LENGTH],
            time: 0,
            date: 0,
            start_cluster,
            file_size,
            dir_sector: 3,
            dir_offset: 0,
        };
        (disk, volume, entry, payload)
    }

    fn mount_fat16_disk(
        start_cluster: u16,
        file_size: u32,
    ) -> (MockDisk, FatVolume, DirEntry, Vec<u8>) {
        let device_id = 0x80;
        let sector_size = 512u16;
        let reserved = 1u16;
        let sectors_per_fat = 32u16;
        let root_entries = 16u16;
        let root_dir_sectors = 1u32;
        let data_clusters = 5000u32;
        let total_sectors =
            reserved as u32 + sectors_per_fat as u32 + root_dir_sectors + data_clusters;
        let mut disk = MockDisk::new(device_id, sector_size, 17, 4, total_sectors);

        let mut boot = vec![0u8; sector_size as usize];
        boot[11..13].copy_from_slice(&sector_size.to_le_bytes());
        boot[13] = 1;
        boot[14..16].copy_from_slice(&reserved.to_le_bytes());
        boot[16] = 1;
        boot[17..19].copy_from_slice(&root_entries.to_le_bytes());
        boot[19..21].copy_from_slice(&(total_sectors as u16).to_le_bytes());
        boot[22..24].copy_from_slice(&sectors_per_fat.to_le_bytes());
        disk.write_sector(0, &boot);

        let mut fat = vec![0u8; sectors_per_fat as usize * sector_size as usize];
        fat[0] = 0xF8;
        fat[1] = 0xFF;
        fat[2] = 0xFF;
        fat[3] = 0xFF;
        let first_offset = start_cluster as usize * 2;
        fat[first_offset..first_offset + 2].copy_from_slice(&(start_cluster + 1).to_le_bytes());
        fat[first_offset + 2..first_offset + 4].copy_from_slice(&(start_cluster + 2).to_le_bytes());
        fat[first_offset + 4..first_offset + 6].copy_from_slice(&0xFFFFu16.to_le_bytes());
        for sector_index in 0..sectors_per_fat as u32 {
            let offset = sector_index as usize * sector_size as usize;
            disk.write_sector(
                1 + sector_index,
                &fat[offset..offset + sector_size as usize],
            );
        }

        let mut root = vec![0u8; sector_size as usize];
        root[0..11].copy_from_slice(b"BIGFILE BIN");
        root[26..28].copy_from_slice(&start_cluster.to_le_bytes());
        root[28..32].copy_from_slice(&file_size.to_le_bytes());
        disk.write_sector(33, &root);

        let payload: Vec<u8> = (0..file_size)
            .map(|index| 255 - ((index & 0xFF) as u8))
            .collect();
        for cluster_offset in 0..3u32 {
            let lba = 34 + (start_cluster as u32 - 2) + cluster_offset;
            let start = cluster_offset as usize * sector_size as usize;
            let end = (start + sector_size as usize).min(payload.len());
            let mut cluster = vec![0u8; sector_size as usize];
            cluster[..end - start].copy_from_slice(&payload[start..end]);
            disk.write_sector(lba, &cluster);
        }

        let mut mount_disk = MockDisk {
            device_id: disk.device_id,
            sector_size: disk.sector_size,
            sectors_per_track: disk.sectors_per_track,
            heads: disk.heads,
            data: disk.data.clone(),
        };
        let volume = FatVolume::mount(
            device_id,
            0,
            &crate::drive_map::pc98::fixed_disk_geometry(),
            &mut mount_disk,
        )
        .expect("mount FAT16");
        let entry = DirEntry {
            name: *b"BIGFILE BIN",
            attribute: 0x20,
            reserved: [0; fat_dir::RESERVED_LENGTH],
            time: 0,
            date: 0,
            start_cluster,
            file_size,
            dir_sector: 33,
            dir_offset: 0,
        };
        (disk, volume, entry, payload)
    }

    #[test]
    fn fat12_cursor_reads_across_cluster_boundary_at_high_cluster_numbers() {
        let (mut disk, volume, entry, payload) = mount_fat12_disk(682, 2500);
        let mut cursor = FatFileCursor::new(&entry);
        let mut combined = Vec::new();
        combined.extend_from_slice(&cursor.read_chunk(&volume, &mut disk, 700).unwrap());
        combined.extend_from_slice(&cursor.read_chunk(&volume, &mut disk, 900).unwrap());
        combined.extend_from_slice(&cursor.read_chunk(&volume, &mut disk, 2000).unwrap());
        assert_eq!(combined, payload);
        assert!(
            cursor
                .read_chunk(&volume, &mut disk, 16)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn fat16_cursor_reads_across_cluster_boundary_at_high_cluster_numbers() {
        let (mut disk, volume, entry, payload) = mount_fat16_disk(4094, 1400);
        let mut cursor = FatFileCursor::new(&entry);
        let mut combined = Vec::new();
        combined.extend_from_slice(&cursor.read_chunk(&volume, &mut disk, 300).unwrap());
        combined.extend_from_slice(&cursor.read_chunk(&volume, &mut disk, 700).unwrap());
        combined.extend_from_slice(&cursor.read_chunk(&volume, &mut disk, 700).unwrap());
        assert_eq!(combined, payload);
    }

    #[test]
    fn read_all_returns_complete_file_contents() {
        let (mut disk, volume, entry, payload) = mount_fat12_disk(682, 2500);
        assert_eq!(read_all(&volume, &entry, &mut disk).unwrap(), payload);
    }

    #[test]
    fn fat12_writer_overwrites_across_cluster_boundary_at_high_cluster_numbers() {
        let (mut disk, mut volume, entry, _) = mount_fat12_disk(682, 2500);
        let payload = (0..2500)
            .map(|index| 255 - (index % 251) as u8)
            .collect::<Vec<_>>();

        let mut writer = FatFileWriter::new(entry.start_cluster, 0);
        writer
            .write_chunk(&mut volume, &mut disk, &payload)
            .unwrap();

        let updated_entry = DirEntry {
            file_size: payload.len() as u32,
            ..entry
        };
        assert_eq!(
            read_all(&volume, &updated_entry, &mut disk).unwrap(),
            payload
        );
    }

    #[test]
    fn fat16_writer_overwrites_across_cluster_boundary_at_high_cluster_numbers() {
        let (mut disk, mut volume, entry, _) = mount_fat16_disk(4094, 1400);
        let payload = (0..1400)
            .map(|index| ((index * 3) % 253) as u8)
            .collect::<Vec<_>>();

        let mut writer = FatFileWriter::new(entry.start_cluster, 0);
        writer
            .write_chunk(&mut volume, &mut disk, &payload)
            .unwrap();

        let updated_entry = DirEntry {
            file_size: payload.len() as u32,
            ..entry
        };
        assert_eq!(
            read_all(&volume, &updated_entry, &mut disk).unwrap(),
            payload
        );
    }

    #[test]
    fn fat16_writer_appends_past_end_of_chain() {
        let cluster_size = 512usize;
        let original_size = cluster_size * 2;
        let (mut disk, mut volume, entry, original_payload) =
            mount_fat16_disk(4094, original_size as u32);
        let suffix = b"tail-data".to_vec();

        let mut writer = FatFileWriter::new(entry.start_cluster, original_payload.len() as u32);
        writer.write_chunk(&mut volume, &mut disk, &suffix).unwrap();

        let mut expected = original_payload;
        expected.extend_from_slice(&suffix);

        let updated_entry = DirEntry {
            start_cluster: writer.start_cluster(),
            file_size: writer.position(),
            ..entry
        };
        assert_eq!(
            read_all(&volume, &updated_entry, &mut disk).unwrap(),
            expected
        );
    }

    #[test]
    fn replacing_a_file_keeps_its_slot_and_its_reserved_bytes() {
        let mut disk = formatted_2hd_floppy(&human68k_2hd_boot_sector());
        let deleted = directory_entry_bytes(b"\xE5LD     X  ", 0x20, &[], 0, 0);
        store_directory_entry(&mut disk, FLOPPY_2HD_ROOT_SECTOR, 0, &deleted);
        let existing = directory_entry_bytes(b"LONGNAMEX  ", 0x20, b"1A", 10, 5);
        store_directory_entry(&mut disk, FLOPPY_2HD_ROOT_SECTOR, 1, &existing);
        store_2hd_single_cluster(&mut disk, 10, b"short");

        let mut volume = mount_2hd(&mut disk);
        let payload = three_cluster_payload();
        create_or_replace_file(
            &mut volume,
            0,
            b"LONGNAMEX  ",
            &payload,
            REPLACE_OPTIONS,
            &mut disk,
        )
        .unwrap();
        volume.flush_fat(&mut disk).unwrap();
        assert_eq!(volume.read_fat_entry(10), 0);

        let root = disk.sector(FLOPPY_2HD_ROOT_SECTOR).to_vec();
        assert_eq!(root[..32], deleted);
        let rewritten = &root[32..64];
        assert_eq!(&rewritten[..11], b"LONGNAMEX  ");
        assert_eq!(
            rewritten[11],
            fat_dir::ATTR_ARCHIVE | fat_dir::ATTR_READ_ONLY
        );
        assert_eq!(&rewritten[12..22], b"1A\0\0\0\0\0\0\0\0");
        assert_eq!(&rewritten[22..26], &[0x34, 0x12, 0x78, 0x56]);
        assert_eq!(&rewritten[28..32], &3000u32.to_le_bytes());

        let entry = fat_dir::find_entry(&volume, 0, b"LONGNAMEX  ", &mut disk)
            .unwrap()
            .unwrap();
        assert_eq!(entry.dir_offset, 32);
        assert_eq!(entry.start_cluster, 2);
        assert_eq!(read_all(&volume, &entry, &mut disk).unwrap(), payload);
    }

    #[test]
    fn failed_replace_deletes_the_entry_and_frees_both_chains() {
        let mut disk = formatted_2hd_floppy(&human68k_2hd_boot_sector());
        let existing = directory_entry_bytes(b"LONGNAMEX  ", 0x20, b"1A", 2, 5);
        store_directory_entry(&mut disk, FLOPPY_2HD_ROOT_SECTOR, 0, &existing);
        store_2hd_single_cluster(&mut disk, 2, b"short");
        let last_cluster = FLOPPY_2HD_DATA_CLUSTER_COUNT + 1;
        set_2hd_fat_entries(&mut disk, 4..=last_cluster, 0x0FFF);

        let mut volume = mount_2hd(&mut disk);
        assert_eq!(volume.free_cluster_count(), 1);
        assert!(
            create_or_replace_file(
                &mut volume,
                0,
                b"LONGNAMEX  ",
                &three_cluster_payload(),
                REPLACE_OPTIONS,
                &mut disk,
            )
            .is_err()
        );

        assert_eq!(volume.read_fat_entry(2), 0);
        assert_eq!(volume.read_fat_entry(3), 0);
        assert_eq!(volume.free_cluster_count(), 2);
        assert_eq!(disk.sector(FLOPPY_2HD_ROOT_SECTOR)[0], 0xE5);
        assert!(
            fat_dir::find_entry(&volume, 0, b"LONGNAMEX  ", &mut disk)
                .unwrap()
                .is_none()
        );
    }
}
