//! FAT12/FAT16 read/write implementation.

use crate::{
    DiskIo,
    drive_map::{BpbSource, DriveGeometry, fm_towns},
    filesystem::fat_bpb::{Bpb, is_human68k_boot_sector},
};

/// Bytes of the boot record that hold the BPB.
const BOOT_RECORD_MINIMUM_LENGTH: u32 = 512;

/// Number of the first data cluster.
const FIRST_DATA_CLUSTER: u16 = 2;

/// A mounted FAT12 or FAT16 volume.
#[derive(Clone)]
pub(crate) struct FatVolume {
    pub device_id: u8,
    pub partition_offset: u32,
    pub bpb: Bpb,
    pub first_root_sector: u32,
    pub first_data_sector: u32,
    pub data_cluster_count: u32,
    pub is_fat16: bool,
    fat_cache: Vec<u8>,
    fat_dirty: bool,
    /// Ratio of BPB logical sector size to physical sector size.
    /// On PC-98 SASI HDDs this can be 4 (1024-byte BPB / 256-byte physical).
    sector_ratio: u32,
    /// The boot sector of the volume is a Human68k boot sector.
    human68k: bool,
}

state_struct_codec!(FatVolume {
    device_id,
    partition_offset,
    bpb,
    first_root_sector,
    first_data_sector,
    data_cluster_count,
    is_fat16,
    fat_cache,
    fat_dirty,
    sector_ratio,
    human68k,
});

/// Mount error of a medium whose format the driver rule does not know.
pub(crate) const ERROR_UNKNOWN_MEDIA: u16 = 0x001A;

impl FatVolume {
    pub(crate) fn validate_state(&self) -> Result<(), save_state::StateValidationError> {
        let expected_fat_length = usize::from(self.bpb.sectors_per_fat)
            .checked_mul(usize::from(self.bpb.bytes_per_sector))
            .ok_or_else(|| save_state::StateValidationError::new("HLE DOS FAT size overflow"))?;
        if self.sector_ratio == 0
            || self.bpb.bytes_per_sector == 0
            || self.bpb.sectors_per_cluster == 0
            || self.fat_cache.len() != expected_fat_length
            || self.first_root_sector != self.bpb.first_root_sector()
            || self.first_data_sector != self.bpb.first_data_sector()
            || self.data_cluster_count != self.bpb.data_cluster_count()
            || self.is_fat16 != self.bpb.is_fat16()
        {
            return Err(save_state::StateValidationError::new(
                "HLE DOS FAT volume state is invalid",
            ));
        }
        Ok(())
    }

    /// Mounts a FAT volume: reads the boot sector to detect a Human68k
    /// volume, takes the BPB from the source of the drive geometry and loads
    /// the FAT.
    ///
    /// `partition_offset` is in physical sectors. The BPB may declare a
    /// larger logical sector size (e.g. 1024 bytes on a 256-byte SASI HDD).
    /// All internal LBA calculations account for this ratio.
    pub fn mount(
        device_id: u8,
        partition_offset: u32,
        geometry: &DriveGeometry,
        disk: &mut dyn DiskIo,
    ) -> Result<Self, u16> {
        let physical_sector_size = disk.sector_size(device_id).ok_or(0x001Fu16)?;
        let boot_record =
            Self::read_boot_record(device_id, partition_offset, physical_sector_size, disk);
        let human68k = boot_record.as_deref().is_some_and(is_human68k_boot_sector);
        let bpb = match geometry.bpb_source {
            BpbSource::BootRecord => Self::boot_record_bpb(
                boot_record.as_deref().ok_or(0x001Fu16)?,
                physical_sector_size,
            )
            .ok_or(0x001Fu16)?,
            BpbSource::BootRecordOrDefault => {
                match Self::boot_record_bpb(
                    boot_record.as_deref().ok_or(0x001Fu16)?,
                    physical_sector_size,
                ) {
                    Some(bpb) => bpb,
                    None => Self::validated_default_bpb(
                        device_id,
                        partition_offset,
                        physical_sector_size,
                        &geometry.default_bpb,
                        disk,
                    )?,
                }
            }
            BpbSource::Default => {
                if !geometry
                    .default_bpb
                    .is_supported_with_physical_sector_size(physical_sector_size)
                {
                    return Err(0x001F);
                }
                geometry.default_bpb
            }
            BpbSource::FmTownsFloppy => {
                Self::fm_towns_floppy_bpb(device_id, partition_offset, physical_sector_size, disk)?
            }
            BpbSource::Driver => {
                if !geometry
                    .bpb
                    .is_supported_with_physical_sector_size(physical_sector_size)
                {
                    return Err(0x001F);
                }
                geometry.bpb
            }
        };

        let sector_ratio = bpb.bytes_per_sector as u32 / physical_sector_size as u32;

        let first_root_sector = bpb.first_root_sector();
        let first_data_sector = bpb.first_data_sector();
        let data_cluster_count = bpb.data_cluster_count();
        let is_fat16 = bpb.is_fat16();

        let fat_start = partition_offset + bpb.reserved_sectors as u32 * sector_ratio;
        let fat_sectors = bpb.sectors_per_fat as u32 * sector_ratio;
        let fat_data = disk
            .read_sectors(device_id, fat_start, fat_sectors)
            .map_err(|_| 0x001Fu16)?;

        Ok(Self {
            device_id,
            partition_offset,
            bpb,
            first_root_sector,
            first_data_sector,
            data_cluster_count,
            is_fat16,
            fat_cache: fat_data,
            fat_dirty: false,
            sector_ratio,
            human68k,
        })
    }

    /// Reads the physical sectors that hold the first 512 bytes of the
    /// volume. Returns `None` when the read fails.
    pub(crate) fn read_boot_record(
        device_id: u8,
        partition_offset: u32,
        physical_sector_size: u16,
        disk: &mut dyn DiskIo,
    ) -> Option<Vec<u8>> {
        let boot_physical_count = BOOT_RECORD_MINIMUM_LENGTH.div_ceil(physical_sector_size as u32);
        disk.read_sectors(device_id, partition_offset, boot_physical_count)
            .ok()
    }

    /// Returns the BPB of a boot record when it describes a usable volume.
    /// A DOS BPB at `0Bh` comes first, then the BPB of a Human68k 2.x boot
    /// sector.
    pub(crate) fn boot_record_bpb(boot_record: &[u8], physical_sector_size: u16) -> Option<Bpb> {
        let is_usable =
            |bpb: &Bpb| bpb.is_supported_with_physical_sector_size(physical_sector_size);
        Bpb::parse(boot_record)
            .filter(is_usable)
            .or_else(|| Bpb::parse_human68k(boot_record).filter(is_usable))
    }

    /// Returns the BPB of an FM Towns floppy from the FAT ID in its sector 1
    /// and its physical format. A FAT ID that the format does not know fails
    /// with `ERROR_UNKNOWN_MEDIA`.
    fn fm_towns_floppy_bpb(
        device_id: u8,
        partition_offset: u32,
        physical_sector_size: u16,
        disk: &mut dyn DiskIo,
    ) -> Result<Bpb, u16> {
        let (_, _, sectors_per_track) = disk.drive_geometry(device_id).ok_or(0x001Fu16)?;
        let fat_sector = disk
            .read_sectors(device_id, partition_offset + 1, 1)
            .map_err(|_| 0x001Fu16)?;
        let fat_id = *fat_sector.first().ok_or(0x001Fu16)?;
        let bpb = fm_towns::floppy_media_bpb(fat_id, physical_sector_size, sectors_per_track)
            .ok_or(ERROR_UNKNOWN_MEDIA)?;
        bpb.is_supported_with_physical_sector_size(physical_sector_size)
            .then_some(bpb)
            .ok_or(0x001F)
    }

    /// Accepts the default BPB for a medium without a BPB in its boot record.
    /// The physical sectors, the device size and the first FAT entries must
    /// match the default BPB.
    fn validated_default_bpb(
        device_id: u8,
        partition_offset: u32,
        physical_sector_size: u16,
        default_bpb: &Bpb,
        disk: &mut dyn DiskIo,
    ) -> Result<Bpb, u16> {
        if physical_sector_size != default_bpb.bytes_per_sector {
            return Err(0x001F);
        }

        let (cylinders, heads, sectors_per_track) =
            disk.drive_geometry(device_id).ok_or(0x001Fu16)?;
        let device_sectors = cylinders as u32 * heads as u32 * sectors_per_track as u32;
        if device_sectors != default_bpb.total_sectors() {
            return Err(0x001F);
        }

        let fat_data = disk
            .read_sectors(
                device_id,
                partition_offset + default_bpb.reserved_sectors as u32,
                1,
            )
            .map_err(|_| 0x001Fu16)?;
        if !default_bpb.has_valid_initial_fat_entries(&fat_data) {
            return Err(0x001F);
        }

        Ok(*default_bpb)
    }

    /// Converts a logical sector of the volume to a physical LBA of the device.
    pub fn logical_sector_lba(&self, logical_sector: u32) -> u32 {
        self.partition_offset + logical_sector * self.sector_ratio
    }

    /// Reads a FAT entry for the given cluster number.
    pub fn read_fat_entry(&self, cluster: u16) -> u16 {
        if self.is_fat16 {
            let offset = cluster as usize * 2;
            if offset + 1 >= self.fat_cache.len() {
                return 0xFFFF;
            }
            u16::from_le_bytes([self.fat_cache[offset], self.fat_cache[offset + 1]])
        } else {
            // FAT12: 12-bit entries packed into 1.5 bytes each
            let offset = (cluster as usize * 3) / 2;
            if offset + 1 >= self.fat_cache.len() {
                return 0xFFF;
            }
            let pair = u16::from_le_bytes([self.fat_cache[offset], self.fat_cache[offset + 1]]);
            if cluster & 1 != 0 {
                pair >> 4
            } else {
                pair & 0x0FFF
            }
        }
    }

    /// Writes a FAT entry for the given cluster number.
    pub fn write_fat_entry(&mut self, cluster: u16, value: u16) {
        if self.is_fat16 {
            let offset = cluster as usize * 2;
            if offset + 1 >= self.fat_cache.len() {
                return;
            }
            let bytes = value.to_le_bytes();
            self.fat_cache[offset] = bytes[0];
            self.fat_cache[offset + 1] = bytes[1];
        } else {
            let offset = (cluster as usize * 3) / 2;
            if offset + 1 >= self.fat_cache.len() {
                return;
            }
            let existing = u16::from_le_bytes([self.fat_cache[offset], self.fat_cache[offset + 1]]);
            let new_pair = if cluster & 1 != 0 {
                (existing & 0x000F) | ((value & 0x0FFF) << 4)
            } else {
                (existing & 0xF000) | (value & 0x0FFF)
            };
            let bytes = new_pair.to_le_bytes();
            self.fat_cache[offset] = bytes[0];
            self.fat_cache[offset + 1] = bytes[1];
        }
        self.fat_dirty = true;
    }

    /// Returns the next cluster in the chain, or None if end-of-chain.
    pub fn next_cluster(&self, cluster: u16) -> Option<u16> {
        let entry = self.read_fat_entry(cluster);
        let eoc = if self.is_fat16 { 0xFFF8 } else { 0x0FF8 };
        if entry >= eoc || entry < 2 {
            None
        } else {
            Some(entry)
        }
    }

    /// Allocates a free cluster and chains it after `after_cluster`.
    /// If `after_cluster` is 0, the new cluster is not chained (used for first cluster).
    /// Returns the newly allocated cluster number.
    pub fn allocate_cluster(&mut self, after_cluster: u16) -> Option<u16> {
        let max = self.data_cluster_count as u16 + 2;
        let eoc = if self.is_fat16 { 0xFFFF } else { 0x0FFF };
        for candidate in 2..max {
            if self.read_fat_entry(candidate) == 0 {
                self.write_fat_entry(candidate, eoc);
                if after_cluster >= 2 {
                    self.write_fat_entry(after_cluster, candidate);
                }
                return Some(candidate);
            }
        }
        None
    }

    /// Frees an entire cluster chain starting at `start`.
    pub fn free_chain(&mut self, start: u16) {
        let mut cluster = start;
        while cluster >= 2 {
            let next = self.next_cluster(cluster);
            self.write_fat_entry(cluster, 0x0000);
            match next {
                Some(n) => cluster = n,
                None => break,
            }
        }
    }

    /// Returns true when the boot sector of the volume is a Human68k boot
    /// sector. Its directory entries then hold 18.3 names.
    pub fn is_human68k(&self) -> bool {
        self.human68k
    }

    /// Ratio of BPB logical sector size to physical sector size.
    pub fn sector_ratio(&self) -> u32 {
        self.sector_ratio
    }

    pub fn bytes_per_sector(&self) -> u16 {
        self.bpb.bytes_per_sector
    }

    pub fn sectors_per_cluster(&self) -> u16 {
        self.bpb.sectors_per_cluster as u16
    }

    pub fn total_cluster_count(&self) -> u16 {
        self.data_cluster_count.min(u16::MAX as u32) as u16
    }

    pub fn free_cluster_count(&self) -> u16 {
        let mut free_clusters = 0u16;
        let max_cluster = self.total_cluster_count().saturating_add(2);
        for cluster in 2..max_cluster {
            if self.read_fat_entry(cluster) == 0 {
                free_clusters = free_clusters.saturating_add(1);
            }
        }
        free_clusters
    }

    /// Returns true when `cluster` is a data cluster of the volume.
    pub fn is_data_cluster(&self, cluster: u16) -> bool {
        cluster >= FIRST_DATA_CLUSTER
            && u32::from(cluster) < self.data_cluster_count + u32::from(FIRST_DATA_CLUSTER)
    }

    /// Converts a cluster number to an absolute physical LBA. A cluster
    /// below the first data cluster maps to the first data cluster.
    pub fn cluster_to_lba(&self, cluster: u16) -> u32 {
        let cluster_index = u32::from(cluster.saturating_sub(FIRST_DATA_CLUSTER));
        self.partition_offset
            + self.first_data_sector * self.sector_ratio
            + cluster_index * self.bpb.sectors_per_cluster as u32 * self.sector_ratio
    }

    /// Returns the absolute physical LBA of the first root directory sector.
    pub fn root_dir_lba(&self) -> u32 {
        self.partition_offset + self.first_root_sector * self.sector_ratio
    }

    /// Number of BPB logical sectors in the root directory.
    pub fn root_dir_sectors(&self) -> u32 {
        self.bpb.root_dir_sectors()
    }

    /// Reads a single cluster (all sectors) into a Vec.
    pub fn read_cluster(&self, cluster: u16, disk: &mut dyn DiskIo) -> Result<Vec<u8>, u16> {
        let lba = self.cluster_to_lba(cluster);
        let count = self.bpb.sectors_per_cluster as u32 * self.sector_ratio;
        disk.read_sectors(self.device_id, lba, count)
            .map_err(|_| 0x001F)
    }

    /// Writes a full cluster to disk.
    pub fn write_cluster(
        &self,
        cluster: u16,
        data: &[u8],
        disk: &mut dyn DiskIo,
    ) -> Result<(), u16> {
        let lba = self.cluster_to_lba(cluster);
        disk.write_sectors(self.device_id, lba, data)
            .map_err(|_| 0x001F)
    }

    /// Reads one BPB logical sector at a physical LBA.
    pub fn read_sector_abs(&self, abs_lba: u32, disk: &mut dyn DiskIo) -> Result<Vec<u8>, u16> {
        disk.read_sectors(self.device_id, abs_lba, self.sector_ratio)
            .map_err(|_| 0x001F)
    }

    /// Writes one BPB logical sector at a physical LBA.
    pub fn write_sector_abs(
        &self,
        abs_lba: u32,
        data: &[u8],
        disk: &mut dyn DiskIo,
    ) -> Result<(), u16> {
        disk.write_sectors(self.device_id, abs_lba, data)
            .map_err(|_| 0x001F)
    }

    /// Flushes the cached FAT to all FAT copies on disk.
    pub fn flush_fat(&mut self, disk: &mut dyn DiskIo) -> Result<(), u16> {
        if !self.fat_dirty {
            return Ok(());
        }
        for fat_idx in 0..self.bpb.num_fats as u32 {
            let fat_start = self.partition_offset
                + self.bpb.reserved_sectors as u32 * self.sector_ratio
                + fat_idx * self.bpb.sectors_per_fat as u32 * self.sector_ratio;
            disk.write_sectors(self.device_id, fat_start, &self.fat_cache)
                .map_err(|_| 0x001Fu16)?;
        }
        self.fat_dirty = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drive_map::{fm_towns, pc98};

    /// Disk image in host memory with a fixed geometry.
    struct ImageDisk {
        /// Bytes per physical sector.
        sector_size: u16,
        /// Cylinders, heads and sectors per track.
        geometry: (u16, u8, u8),
        /// Image bytes.
        data: Vec<u8>,
    }

    impl DiskIo for ImageDisk {
        fn read_sectors(&mut self, _device_id: u8, lba: u32, count: u32) -> Result<Vec<u8>, u8> {
            let size = usize::from(self.sector_size);
            let start = lba as usize * size;
            let end = start + count as usize * size;
            self.data.get(start..end).map(<[u8]>::to_vec).ok_or(0x10)
        }

        fn write_sectors(&mut self, _device_id: u8, _lba: u32, _data: &[u8]) -> Result<(), u8> {
            Err(0x10)
        }

        fn sector_size(&self, _device_id: u8) -> Option<u16> {
            Some(self.sector_size)
        }

        fn total_sectors(&self, _device_id: u8) -> Option<u32> {
            Some((self.data.len() / usize::from(self.sector_size)) as u32)
        }

        fn drive_geometry(&self, _device_id: u8) -> Option<(u16, u8, u8)> {
            Some(self.geometry)
        }
    }

    /// OS ROM image: `IPL4` boot record without a BPB and the FAT at `200h`.
    fn os_rom_image() -> ImageDisk {
        let mut data = vec![0u8; 1280 * 512];
        data[..5].copy_from_slice(b"IPL4\xCB");
        data[0x200..0x203].copy_from_slice(&[0xFB, 0xFF, 0xFF]);
        ImageDisk {
            sector_size: 512,
            geometry: (80, 2, 8),
            data,
        }
    }

    /// PC-98 2HD floppy without a BPB in the boot sector.
    fn floppy_2hd_without_bpb(media_descriptor: u8) -> ImageDisk {
        let mut data = vec![0u8; 1232 * 1024];
        data[0x400..0x403].copy_from_slice(&[media_descriptor, 0xFF, 0xFF]);
        ImageDisk {
            sector_size: 1024,
            geometry: (77, 2, 8),
            data,
        }
    }

    #[test]
    fn os_rom_volume_mounts_with_the_platform_bpb() {
        let mut disk = os_rom_image();
        let geometry = fm_towns::os_rom_entry(2).geometry.unwrap();
        let volume = FatVolume::mount(fm_towns::OS_ROM_DEVICE_ID, 0, &geometry, &mut disk).unwrap();
        assert_eq!(volume.bpb, fm_towns::OS_ROM_BPB);
        assert_eq!(volume.root_dir_lba(), 5);
        assert_eq!(volume.cluster_to_lba(0x4E), 0x14800 / 512);
        assert_eq!(volume.read_fat_entry(0), 0xFFB);
    }

    #[test]
    fn boot_record_source_rejects_a_volume_without_bpb() {
        let mut disk = os_rom_image();
        let geometry = DriveGeometry::new(BpbSource::BootRecord, fm_towns::OS_ROM_BPB);
        assert_eq!(
            FatVolume::mount(fm_towns::OS_ROM_DEVICE_ID, 0, &geometry, &mut disk).err(),
            Some(0x001F)
        );
    }

    #[test]
    fn default_bpb_needs_a_matching_fat_id() {
        let geometry = pc98::floppy_2hd_geometry();
        let mut disk = floppy_2hd_without_bpb(0xFE);
        let volume = FatVolume::mount(0x90, 0, &geometry, &mut disk).unwrap();
        assert_eq!(volume.bpb, pc98::FLOPPY_2HD_BPB);

        let mut disk = floppy_2hd_without_bpb(0xF9);
        assert!(FatVolume::mount(0x90, 0, &geometry, &mut disk).is_err());
    }

    #[test]
    fn fm_towns_floppy_without_bpb_mounts_by_its_fat_id() {
        let geometry = fm_towns::floppy_entry(0, 0).geometry.unwrap();
        let mut disk = floppy_2hd_without_bpb(0xFE);
        let volume = FatVolume::mount(0x20, 0, &geometry, &mut disk).unwrap();
        assert_eq!(volume.bpb, fm_towns::FLOPPY_BPB);
        assert_eq!(volume.root_dir_lba(), 5);

        let mut disk = floppy_2hd_without_bpb(0xF0);
        assert_eq!(
            FatVolume::mount(0x20, 0, &geometry, &mut disk).err(),
            Some(ERROR_UNKNOWN_MEDIA)
        );
    }

    #[test]
    fn fm_towns_floppy_mounts_the_rare_formats_of_io_sys() {
        let geometry = fm_towns::floppy_entry(0, 0).geometry.unwrap();
        let mut data = vec![0u8; 77 * 26 * 128];
        data[0x80..0x83].copy_from_slice(&[0xFE, 0xFF, 0xFF]);
        let mut disk = ImageDisk {
            sector_size: 128,
            geometry: (77, 1, 26),
            data,
        };
        let volume = FatVolume::mount(0x20, 0, &geometry, &mut disk).unwrap();
        assert_eq!(volume.bpb, fm_towns::FLOPPY_128_BPB);
        assert_eq!(volume.root_dir_lba(), 13);

        let mut data = vec![0u8; 80 * 2 * 18 * 512];
        data[0x200..0x203].copy_from_slice(&[0xFE, 0xFF, 0xFF]);
        let mut disk = ImageDisk {
            sector_size: 512,
            geometry: (80, 2, 18),
            data,
        };
        let volume = FatVolume::mount(0x20, 0, &geometry, &mut disk).unwrap();
        assert_eq!(volume.bpb, fm_towns::FLOPPY_1440K_MODE_1024_BPB);
    }

    #[test]
    fn fm_towns_floppy_ignores_the_bpb_of_the_boot_record() {
        let geometry = fm_towns::floppy_entry(0, 0).geometry.unwrap();
        let mut disk = floppy_2hd_without_bpb(0xFE);
        disk.data[0x0B..0x18].copy_from_slice(&[
            0x00, 0x04, 0x01, 0x01, 0x00, 0x02, 0xC0, 0x00, 0xD0, 0x04, 0xFE, 0x02, 0x00,
        ]);
        let volume = FatVolume::mount(0x20, 0, &geometry, &mut disk).unwrap();
        assert_eq!(volume.bpb.media_descriptor, 0x01);
    }

    #[test]
    fn default_bpb_needs_a_matching_device_size() {
        let geometry = pc98::floppy_2hd_geometry();
        let mut disk = floppy_2hd_without_bpb(0xFE);
        disk.geometry = (80, 2, 8);
        assert!(FatVolume::mount(0x90, 0, &geometry, &mut disk).is_err());
    }

    #[test]
    fn driver_source_mounts_with_the_reported_bpb() {
        let mut disk = os_rom_image();
        let geometry = DriveGeometry::new(BpbSource::Driver, fm_towns::OS_ROM_BPB);
        let volume = FatVolume::mount(0x42, 0, &geometry, &mut disk).unwrap();
        assert_eq!(volume.bpb, fm_towns::OS_ROM_BPB);
    }

    #[test]
    fn logical_sectors_map_to_physical_sectors() {
        let mut data = vec![0u8; 0x8000 * 512];
        let boot = 3 * 512;
        data[boot + 0x0B..boot + 0x18].copy_from_slice(&[
            0x00, 0x08, 0x04, 0x01, 0x00, 0x02, 0x00, 0x02, 0x00, 0x1E, 0xF9, 0x03, 0x00,
        ]);
        let fat = boot + 2048;
        data[fat..fat + 3].copy_from_slice(&[0xF9, 0xFF, 0xFF]);
        let mut disk = ImageDisk {
            sector_size: 512,
            geometry: (0, 0, 0),
            data,
        };
        let partition_bpb = Bpb {
            bytes_per_sector: 0x0800,
            sectors_per_cluster: 4,
            reserved_sectors: 1,
            num_fats: 2,
            root_entry_count: 0x0200,
            total_sectors_16: 0x1E00,
            media_descriptor: 0xF9,
            sectors_per_fat: 3,
            total_sectors_32: 0,
        };
        let geometry = fm_towns::scsi_partition_entry(0, 0, 3, partition_bpb)
            .geometry
            .unwrap();
        let volume = FatVolume::mount(0xB0, 3, &geometry, &mut disk).unwrap();
        assert_eq!(volume.sector_ratio(), 4);
        assert_eq!(volume.logical_sector_lba(0), 3);
        assert_eq!(volume.logical_sector_lba(1), 7);
        assert_eq!(volume.root_dir_lba(), 0x1F);
        assert_eq!(volume.cluster_to_lba(2), 0x3F);
    }

    /// Human68k 2.x floppy in the 2HQ format (80 x 2 x 18 x 512) with its
    /// big-endian BPB and the FAT ID `F0`.
    fn human68k_2hq_image() -> ImageDisk {
        let mut data = vec![0u8; 80 * 2 * 18 * 512];
        data[..0x12].copy_from_slice(b"\x60\x1CHudson soft 2.00");
        data[0x12..0x1E].copy_from_slice(&[
            0x02, 0x00, 0x01, 0x02, 0x00, 0x01, 0x00, 0xE0, 0x0B, 0x40, 0xF0, 0x09,
        ]);
        data[0x200..0x203].copy_from_slice(&[0xF0, 0xFF, 0xFF]);
        ImageDisk {
            sector_size: 512,
            geometry: (80, 2, 18),
            data,
        }
    }

    #[test]
    fn human68k_2hq_volume_mounts_with_the_bpb_of_its_boot_sector() {
        let mut disk = human68k_2hq_image();
        let volume = FatVolume::mount(0x90, 0, &pc98::floppy_2hd_geometry(), &mut disk).unwrap();
        assert_eq!(
            volume.bpb,
            Bpb {
                bytes_per_sector: 512,
                sectors_per_cluster: 1,
                reserved_sectors: 1,
                num_fats: 2,
                root_entry_count: 224,
                total_sectors_16: 2880,
                media_descriptor: 0xF0,
                sectors_per_fat: 9,
                total_sectors_32: 0,
            }
        );
        assert_eq!(volume.root_dir_lba(), 19);
        assert!(volume.is_human68k());
    }

    #[test]
    fn human68k_3x_volume_reports_human68k() {
        let mut disk = floppy_2hd_without_bpb(0xFE);
        disk.data[..0x18].copy_from_slice(&[
            0x60, 0x3C, 0x90, b'X', b'6', b'8', b'I', b'P', b'L', b'3', b'0', 0x00, 0x04, 0x01,
            0x01, 0x00, 0x02, 0xC0, 0x00, 0xD0, 0x04, 0xFE, 0x02, 0x00,
        ]);
        let volume = FatVolume::mount(0x90, 0, &pc98::floppy_2dd_geometry(), &mut disk).unwrap();
        assert_eq!(volume.bpb, pc98::FLOPPY_2HD_BPB);
        assert!(volume.is_human68k());
    }

    #[test]
    fn pc98_volume_is_no_human68k_volume() {
        let mut disk = floppy_2hd_without_bpb(0xFE);
        disk.data[..3].copy_from_slice(&[0xEB, 0x1C, 0x90]);
        disk.data[0x0B..0x18].copy_from_slice(&[
            0x00, 0x04, 0x01, 0x01, 0x00, 0x02, 0xC0, 0x00, 0xD0, 0x04, 0xFE, 0x02, 0x00,
        ]);
        let volume = FatVolume::mount(0x90, 0, &pc98::floppy_2dd_geometry(), &mut disk).unwrap();
        assert!(!volume.is_human68k());

        let mut disk = floppy_2hd_without_bpb(0xFE);
        let volume = FatVolume::mount(0x90, 0, &pc98::floppy_2hd_geometry(), &mut disk).unwrap();
        assert!(!volume.is_human68k());
    }

    #[test]
    fn clusters_outside_the_data_area_are_no_data_clusters() {
        let mut disk = floppy_2hd_without_bpb(0xFE);
        let volume = FatVolume::mount(0x90, 0, &pc98::floppy_2hd_geometry(), &mut disk).unwrap();
        assert!(!volume.is_data_cluster(0));
        assert!(!volume.is_data_cluster(1));
        assert!(volume.is_data_cluster(2));
        assert!(volume.is_data_cluster(1222));
        assert!(!volume.is_data_cluster(1223));
        assert_eq!(volume.cluster_to_lba(0), volume.cluster_to_lba(2));
        assert_eq!(volume.cluster_to_lba(1), 11);
    }
}
