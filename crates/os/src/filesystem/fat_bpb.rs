//! BIOS Parameter Block parsing and validation.

/// BIOS Parameter Block parsed from a FAT boot sector.
#[derive(Debug, Clone)]
pub(crate) struct Bpb {
    pub bytes_per_sector: u16,
    pub sectors_per_cluster: u8,
    pub reserved_sectors: u16,
    pub num_fats: u8,
    pub root_entry_count: u16,
    pub total_sectors_16: u16,
    pub media_descriptor: u8,
    pub sectors_per_fat: u16,
    pub total_sectors_32: u32,
}

impl Bpb {
    /// Parses a BPB from a boot sector (at least 62 bytes required).
    pub fn parse(sector: &[u8]) -> Option<Self> {
        if sector.len() < 62 {
            return None;
        }

        let bytes_per_sector = u16::from_le_bytes([sector[11], sector[12]]);
        if bytes_per_sector == 0 || (bytes_per_sector & (bytes_per_sector - 1)) != 0 {
            return None;
        }

        let sectors_per_cluster = sector[13];
        if sectors_per_cluster == 0 || (sectors_per_cluster & (sectors_per_cluster - 1)) != 0 {
            return None;
        }

        let reserved_sectors = u16::from_le_bytes([sector[14], sector[15]]);
        let num_fats = sector[16];
        let root_entry_count = u16::from_le_bytes([sector[17], sector[18]]);
        let total_sectors_16 = u16::from_le_bytes([sector[19], sector[20]]);
        let media_descriptor = sector[21];
        let sectors_per_fat = u16::from_le_bytes([sector[22], sector[23]]);
        let total_sectors_32 = u32::from_le_bytes([sector[32], sector[33], sector[34], sector[35]]);

        Some(Self {
            bytes_per_sector,
            sectors_per_cluster,
            reserved_sectors,
            num_fats,
            root_entry_count,
            total_sectors_16,
            media_descriptor,
            sectors_per_fat,
            total_sectors_32,
        })
    }

    pub fn pc98_2hd_floppy() -> Self {
        Self {
            bytes_per_sector: 1024,
            sectors_per_cluster: 1,
            reserved_sectors: 1,
            num_fats: 2,
            root_entry_count: 192,
            total_sectors_16: 1232,
            media_descriptor: 0xFE,
            sectors_per_fat: 2,
            total_sectors_32: 0,
        }
    }

    pub fn is_supported_with_physical_sector_size(&self, physical_sector_size: u16) -> bool {
        if physical_sector_size == 0
            || self.bytes_per_sector < physical_sector_size
            || !self.bytes_per_sector.is_multiple_of(physical_sector_size)
        {
            return false;
        }

        self.reserved_sectors != 0
            && self.num_fats != 0
            && self.root_entry_count != 0
            && self.sectors_per_fat != 0
            && self.total_sectors() != 0
            && self.first_data_sector() < self.total_sectors()
    }

    pub fn has_valid_initial_fat_entries(&self, fat_data: &[u8]) -> bool {
        if self.is_fat16() {
            fat_data.len() >= 4
                && fat_data[0] == self.media_descriptor
                && fat_data[1] == 0xFF
                && fat_data[2] == 0xFF
                && fat_data[3] == 0xFF
        } else {
            fat_data.len() >= 3
                && fat_data[0] == self.media_descriptor
                && fat_data[1] == 0xFF
                && fat_data[2] == 0xFF
        }
    }

    /// Total sectors on the volume.
    pub fn total_sectors(&self) -> u32 {
        if self.total_sectors_16 != 0 {
            self.total_sectors_16 as u32
        } else {
            self.total_sectors_32
        }
    }

    /// Number of sectors occupied by the root directory.
    pub fn root_dir_sectors(&self) -> u32 {
        (self.root_entry_count as u32 * 32).div_ceil(self.bytes_per_sector as u32)
    }

    /// First sector of the root directory (relative to partition start).
    pub fn first_root_sector(&self) -> u32 {
        self.reserved_sectors as u32 + (self.num_fats as u32 * self.sectors_per_fat as u32)
    }

    /// First data sector (relative to partition start).
    pub fn first_data_sector(&self) -> u32 {
        self.first_root_sector() + self.root_dir_sectors()
    }

    /// Number of data clusters on the volume.
    pub fn data_cluster_count(&self) -> u32 {
        let total = self.total_sectors();
        let data_sectors = total.saturating_sub(self.first_data_sector());
        data_sectors / self.sectors_per_cluster as u32
    }

    /// Returns true if the volume is FAT16 (>= 4085 data clusters).
    pub fn is_fat16(&self) -> bool {
        self.data_cluster_count() >= 4085
    }

    /// Bytes per cluster.
    pub fn cluster_size(&self) -> u32 {
        self.bytes_per_sector as u32 * self.sectors_per_cluster as u32
    }
}
