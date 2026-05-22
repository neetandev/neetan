//! FAT12/FAT16 read/write implementation.

use crate::{DiskIo, filesystem::fat_bpb::Bpb};

/// A mounted FAT12 or FAT16 volume.
pub(crate) struct FatVolume {
    pub drive_da: u8,
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
}

impl FatVolume {
    /// Mounts a FAT volume by reading the boot sector and loading the FAT.
    ///
    /// `partition_offset` is in physical sectors.  The BPB may declare a
    /// larger logical sector size (e.g. 1024 bytes on a 256-byte SASI HDD).
    /// All internal LBA calculations account for this ratio.
    pub fn mount(drive_da: u8, partition_offset: u32, disk: &mut dyn DiskIo) -> Result<Self, u16> {
        let physical_sector_size = disk.sector_size(drive_da).ok_or(0x001Fu16)?;

        // Read enough physical sectors to cover at least 512 bytes (minimum BPB).
        let boot_phys_count = (512u32).div_ceil(physical_sector_size as u32);
        let boot_sector = disk
            .read_sectors(drive_da, partition_offset, boot_phys_count)
            .map_err(|_| 0x001Fu16)?;
        let bpb = match Bpb::parse(&boot_sector)
            .filter(|bpb| bpb.is_supported_with_physical_sector_size(physical_sector_size))
        {
            Some(bpb) => bpb,
            None => Self::fallback_bpb(drive_da, partition_offset, physical_sector_size, disk)?,
        };

        let sector_ratio = bpb.bytes_per_sector as u32 / physical_sector_size as u32;

        let first_root_sector = bpb.first_root_sector();
        let first_data_sector = bpb.first_data_sector();
        let data_cluster_count = bpb.data_cluster_count();
        let is_fat16 = bpb.is_fat16();

        let fat_start = partition_offset + bpb.reserved_sectors as u32 * sector_ratio;
        let fat_sectors = bpb.sectors_per_fat as u32 * sector_ratio;
        let fat_data = disk
            .read_sectors(drive_da, fat_start, fat_sectors)
            .map_err(|_| 0x001Fu16)?;

        Ok(Self {
            drive_da,
            partition_offset,
            bpb,
            first_root_sector,
            first_data_sector,
            data_cluster_count,
            is_fat16,
            fat_cache: fat_data,
            fat_dirty: false,
            sector_ratio,
        })
    }

    fn fallback_bpb(
        drive_da: u8,
        partition_offset: u32,
        physical_sector_size: u16,
        disk: &mut dyn DiskIo,
    ) -> Result<Bpb, u16> {
        if partition_offset != 0 || drive_da & 0xF0 != 0x90 || physical_sector_size != 1024 {
            return Err(0x001F);
        }

        let (cylinders, heads, sectors_per_track) =
            disk.drive_geometry(drive_da).ok_or(0x001Fu16)?;
        if cylinders != 77 || heads != 2 || sectors_per_track != 8 {
            return Err(0x001F);
        }

        let bpb = Bpb::pc98_2hd_floppy();
        let fat_data = disk
            .read_sectors(drive_da, bpb.reserved_sectors as u32, 1)
            .map_err(|_| 0x001Fu16)?;
        if !bpb.has_valid_initial_fat_entries(&fat_data) {
            return Err(0x001F);
        }

        Ok(bpb)
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

    /// Converts a cluster number to an absolute physical LBA.
    pub fn cluster_to_lba(&self, cluster: u16) -> u32 {
        self.partition_offset
            + self.first_data_sector * self.sector_ratio
            + (cluster as u32 - 2) * self.bpb.sectors_per_cluster as u32 * self.sector_ratio
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
        disk.read_sectors(self.drive_da, lba, count)
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
        disk.write_sectors(self.drive_da, lba, data)
            .map_err(|_| 0x001F)
    }

    /// Reads one BPB logical sector at a physical LBA.
    pub fn read_sector_abs(&self, abs_lba: u32, disk: &mut dyn DiskIo) -> Result<Vec<u8>, u16> {
        disk.read_sectors(self.drive_da, abs_lba, self.sector_ratio)
            .map_err(|_| 0x001F)
    }

    /// Writes one BPB logical sector at a physical LBA.
    pub fn write_sector_abs(
        &self,
        abs_lba: u32,
        data: &[u8],
        disk: &mut dyn DiskIo,
    ) -> Result<(), u16> {
        disk.write_sectors(self.drive_da, abs_lba, data)
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
            disk.write_sectors(self.drive_da, fat_start, &self.fat_cache)
                .map_err(|_| 0x001Fu16)?;
        }
        self.fat_dirty = false;
        Ok(())
    }
}
