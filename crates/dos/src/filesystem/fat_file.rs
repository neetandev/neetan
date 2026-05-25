use crate::{
    DiskIo,
    filesystem::{
        fat::FatVolume,
        fat_dir::{self, DirEntry},
    },
};

pub(crate) struct FatFileCursor {
    start_cluster: u16,
    file_size: u32,
    position: u32,
    cached_cluster_index: u32,
    cached_cluster: u16,
}

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
    if entry.file_size == 0 {
        return Ok(Vec::new());
    }

    let mut cursor = FatFileCursor::new(entry);
    cursor.read_chunk(vol, disk, entry.file_size as usize)
}

#[derive(Clone, Copy)]
pub(crate) struct FileCreateOptions {
    pub(crate) attributes: u8,
    pub(crate) time: u16,
    pub(crate) date: u16,
}

pub(crate) fn create_or_replace_file(
    vol: &mut FatVolume,
    dir_cluster: u16,
    fcb_name: &[u8; 11],
    data: &[u8],
    options: FileCreateOptions,
    disk: &mut dyn DiskIo,
) -> Result<(), u16> {
    if let Some(existing) = fat_dir::find_entry(vol, dir_cluster, fcb_name, disk)? {
        if existing.attribute & fat_dir::ATTR_DIRECTORY != 0 {
            return Err(0x0005);
        }
        if existing.start_cluster >= 2 {
            vol.free_chain(existing.start_cluster);
        }
        fat_dir::delete_entry(vol, &existing, disk)?;
    }

    let mut writer = FatFileWriter::new(0, 0);
    if let Err(error) = writer.write_chunk(vol, disk, data) {
        if writer.start_cluster() >= 2 {
            vol.free_chain(writer.start_cluster());
        }
        return Err(error);
    }

    let new_entry = DirEntry {
        name: *fcb_name,
        attribute: options.attributes,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filesystem::fat::FatVolume;

    struct MockDisk {
        drive_da: u8,
        sector_size: u16,
        sectors_per_track: u8,
        heads: u8,
        data: Vec<u8>,
    }

    impl MockDisk {
        fn new(
            drive_da: u8,
            sector_size: u16,
            sectors_per_track: u8,
            heads: u8,
            total_sectors: u32,
        ) -> Self {
            Self {
                drive_da,
                sector_size,
                sectors_per_track,
                heads,
                data: vec![0u8; total_sectors as usize * sector_size as usize],
            }
        }

        fn write_sector(&mut self, lba: u32, sector: &[u8]) {
            let offset = lba as usize * self.sector_size as usize;
            self.data[offset..offset + self.sector_size as usize].copy_from_slice(sector);
        }
    }

    impl DiskIo for MockDisk {
        fn read_sectors(&mut self, drive_da: u8, lba: u32, count: u32) -> Result<Vec<u8>, u8> {
            if drive_da != self.drive_da {
                return Err(0x02);
            }
            let start = lba as usize * self.sector_size as usize;
            let end = start + count as usize * self.sector_size as usize;
            self.data.get(start..end).map(|s| s.to_vec()).ok_or(0x10)
        }

        fn write_sectors(&mut self, drive_da: u8, lba: u32, data: &[u8]) -> Result<(), u8> {
            if drive_da != self.drive_da {
                return Err(0x02);
            }
            let start = lba as usize * self.sector_size as usize;
            let end = start + data.len();
            let Some(slice) = self.data.get_mut(start..end) else {
                return Err(0x10);
            };
            slice.copy_from_slice(data);
            Ok(())
        }

        fn sector_size(&self, drive_da: u8) -> Option<u16> {
            (drive_da == self.drive_da).then_some(self.sector_size)
        }

        fn total_sectors(&self, drive_da: u8) -> Option<u32> {
            (drive_da == self.drive_da).then_some(self.data.len() as u32 / self.sector_size as u32)
        }

        fn drive_geometry(&self, drive_da: u8) -> Option<(u16, u8, u8)> {
            (drive_da == self.drive_da).then_some((
                self.total_sectors(drive_da)? as u16
                    / self.heads as u16
                    / self.sectors_per_track as u16,
                self.heads,
                self.sectors_per_track,
            ))
        }
    }

    fn set_fat12_entry(fat: &mut [u8], cluster: u16, value: u16) {
        let offset = (cluster as usize * 3) / 2;
        if cluster & 1 == 0 {
            fat[offset] = (value & 0x00FF) as u8;
            fat[offset + 1] = (fat[offset + 1] & 0xF0) | ((value >> 8) as u8 & 0x0F);
        } else {
            fat[offset] = (fat[offset] & 0x0F) | (((value << 4) as u8) & 0xF0);
            fat[offset + 1] = (value >> 4) as u8;
        }
    }

    fn mount_fat12_disk(
        start_cluster: u16,
        file_size: u32,
    ) -> (MockDisk, FatVolume, DirEntry, Vec<u8>) {
        let drive_da = 0x90;
        let sector_size = 1024u16;
        let reserved = 1u16;
        let sectors_per_fat = 2u16;
        let root_entries = 16u16;
        let root_dir_sectors = 1u32;
        let data_clusters = 1221u32;
        let total_sectors =
            reserved as u32 + sectors_per_fat as u32 + root_dir_sectors + data_clusters;
        let mut disk = MockDisk::new(drive_da, sector_size, 8, 2, total_sectors);

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
            drive_da: disk.drive_da,
            sector_size: disk.sector_size,
            sectors_per_track: disk.sectors_per_track,
            heads: disk.heads,
            data: disk.data.clone(),
        };
        let volume = FatVolume::mount(drive_da, 0, &mut mount_disk).expect("mount FAT12");
        let entry = DirEntry {
            name: *b"BIGFILE BIN",
            attribute: 0x20,
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
        let drive_da = 0x80;
        let sector_size = 512u16;
        let reserved = 1u16;
        let sectors_per_fat = 32u16;
        let root_entries = 16u16;
        let root_dir_sectors = 1u32;
        let data_clusters = 5000u32;
        let total_sectors =
            reserved as u32 + sectors_per_fat as u32 + root_dir_sectors + data_clusters;
        let mut disk = MockDisk::new(drive_da, sector_size, 17, 4, total_sectors);

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
            drive_da: disk.drive_da,
            sector_size: disk.sector_size,
            sectors_per_track: disk.sectors_per_track,
            heads: disk.heads,
            data: disk.data.clone(),
        };
        let volume = FatVolume::mount(drive_da, 0, &mut mount_disk).expect("mount FAT16");
        let entry = DirEntry {
            name: *b"BIGFILE BIN",
            attribute: 0x20,
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
}
