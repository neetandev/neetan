//! BIOS Parameter Block parsing and validation.

/// Offset of the BIOS parameter block in a FAT boot sector.
pub const BOOT_SECTOR_BPB_OFFSET: usize = 11;

/// Length of the DOS 3.31 BIOS parameter block (up to the 32-bit sector count).
pub const PARAMETER_BLOCK_LENGTH: usize = 25;

/// Size of one FAT directory entry in bytes.
const DIRECTORY_ENTRY_SIZE: u32 = 32;

/// Data cluster count from which a volume uses FAT16.
const FAT16_MINIMUM_CLUSTER_COUNT: u32 = 4085;

/// BIOS Parameter Block of a FAT volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bpb {
    /// Bytes per logical sector.
    pub bytes_per_sector: u16,
    /// Logical sectors per cluster.
    pub sectors_per_cluster: u8,
    /// Logical sectors in front of the first FAT.
    pub reserved_sectors: u16,
    /// Number of FAT copies.
    pub num_fats: u8,
    /// Number of root directory entries.
    pub root_entry_count: u16,
    /// Total logical sectors, zero when `total_sectors_32` holds the count.
    pub total_sectors_16: u16,
    /// Media descriptor byte.
    pub media_descriptor: u8,
    /// Logical sectors per FAT.
    pub sectors_per_fat: u16,
    /// Total logical sectors of a large volume.
    pub total_sectors_32: u32,
}

state_struct_codec!(Bpb {
    bytes_per_sector,
    sectors_per_cluster,
    reserved_sectors,
    num_fats,
    root_entry_count,
    total_sectors_16,
    media_descriptor,
    sectors_per_fat,
    total_sectors_32,
});

impl Bpb {
    /// Parses the BPB of a FAT boot sector (at least 62 bytes).
    pub fn parse(sector: &[u8]) -> Option<Self> {
        if sector.len() < 62 {
            return None;
        }
        Self::from_parameter_block(&sector[BOOT_SECTOR_BPB_OFFSET..])
    }

    /// Parses a BIOS parameter block that starts with the bytes per sector
    /// field. Returns `None` for sector and cluster sizes that are no powers
    /// of two.
    pub fn from_parameter_block(block: &[u8]) -> Option<Self> {
        if block.len() < PARAMETER_BLOCK_LENGTH {
            return None;
        }

        let bytes_per_sector = u16::from_le_bytes([block[0], block[1]]);
        if !bytes_per_sector.is_power_of_two() {
            return None;
        }

        let sectors_per_cluster = block[2];
        if !sectors_per_cluster.is_power_of_two() {
            return None;
        }

        Some(Self {
            bytes_per_sector,
            sectors_per_cluster,
            reserved_sectors: u16::from_le_bytes([block[3], block[4]]),
            num_fats: block[5],
            root_entry_count: u16::from_le_bytes([block[6], block[7]]),
            total_sectors_16: u16::from_le_bytes([block[8], block[9]]),
            media_descriptor: block[10],
            sectors_per_fat: u16::from_le_bytes([block[11], block[12]]),
            total_sectors_32: u32::from_le_bytes([block[21], block[22], block[23], block[24]]),
        })
    }

    /// Returns true when the BPB describes a usable volume on a device with
    /// the given physical sector size.
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

    /// Returns true when the first FAT entries hold the media descriptor and
    /// the end of chain marker.
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
        (self.root_entry_count as u32 * DIRECTORY_ENTRY_SIZE).div_ceil(self.bytes_per_sector as u32)
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
        self.data_cluster_count() >= FAT16_MINIMUM_CLUSTER_COUNT
    }

    /// Bytes per cluster.
    pub fn cluster_size(&self) -> u32 {
        self.bytes_per_sector as u32 * self.sectors_per_cluster as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Boot sector of a PC-98 2HD floppy with the BPB at offset 0Bh.
    fn floppy_boot_sector() -> Vec<u8> {
        let mut sector = vec![0u8; 1024];
        sector[BOOT_SECTOR_BPB_OFFSET..BOOT_SECTOR_BPB_OFFSET + 13].copy_from_slice(&[
            0x00, 0x04, 0x01, 0x01, 0x00, 0x02, 0xC0, 0x00, 0xD0, 0x04, 0xFE, 0x02, 0x00,
        ]);
        sector
    }

    #[test]
    fn parse_reads_the_boot_sector_fields() {
        let bpb = Bpb::parse(&floppy_boot_sector()).unwrap();
        assert_eq!(bpb.bytes_per_sector, 1024);
        assert_eq!(bpb.sectors_per_cluster, 1);
        assert_eq!(bpb.reserved_sectors, 1);
        assert_eq!(bpb.num_fats, 2);
        assert_eq!(bpb.root_entry_count, 192);
        assert_eq!(bpb.total_sectors(), 1232);
        assert_eq!(bpb.media_descriptor, 0xFE);
        assert_eq!(bpb.sectors_per_fat, 2);
        assert_eq!(bpb.first_root_sector(), 5);
        assert_eq!(bpb.first_data_sector(), 11);
        assert!(!bpb.is_fat16());
    }

    #[test]
    fn parse_rejects_sizes_that_are_no_powers_of_two() {
        let mut sector = floppy_boot_sector();
        sector[BOOT_SECTOR_BPB_OFFSET] = 0x01;
        assert!(Bpb::parse(&sector).is_none());
        let mut sector = floppy_boot_sector();
        sector[BOOT_SECTOR_BPB_OFFSET + 2] = 0x03;
        assert!(Bpb::parse(&sector).is_none());
        let mut sector = floppy_boot_sector();
        sector[BOOT_SECTOR_BPB_OFFSET + 2] = 0x00;
        assert!(Bpb::parse(&sector).is_none());
    }

    #[test]
    fn parameter_block_reads_the_32_bit_sector_count() {
        let mut block = [0u8; PARAMETER_BLOCK_LENGTH];
        block[0..2].copy_from_slice(&512u16.to_le_bytes());
        block[2] = 4;
        block[21..25].copy_from_slice(&0x0001_2345u32.to_le_bytes());
        let bpb = Bpb::from_parameter_block(&block).unwrap();
        assert_eq!(bpb.total_sectors(), 0x0001_2345);
        assert!(Bpb::from_parameter_block(&block[..PARAMETER_BLOCK_LENGTH - 1]).is_none());
    }

    #[test]
    fn supported_needs_a_multiple_of_the_physical_sector_size() {
        let bpb = Bpb::parse(&floppy_boot_sector()).unwrap();
        assert!(bpb.is_supported_with_physical_sector_size(1024));
        assert!(bpb.is_supported_with_physical_sector_size(256));
        assert!(!bpb.is_supported_with_physical_sector_size(2048));
        assert!(!bpb.is_supported_with_physical_sector_size(0));
    }
}
