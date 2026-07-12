//! PC/AT master boot record partition table parsing.

use crate::DiskIo;

/// Byte offset of the first partition entry in the MBR.
const PARTITION_TABLE_OFFSET: usize = 0x1BE;
/// Size of one partition entry.
const PARTITION_ENTRY_SIZE: usize = 16;
/// Number of primary partition entries.
const PARTITION_COUNT: usize = 4;
/// Byte offset of the boot signature.
const SIGNATURE_OFFSET: usize = 510;

/// Partition type: FAT12.
const TYPE_FAT12: u8 = 0x01;
/// Partition type: FAT16 with fewer than 65,536 sectors.
const TYPE_FAT16_SMALL: u8 = 0x04;
/// Partition type: FAT16B.
const TYPE_FAT16B: u8 = 0x06;
/// Partition type: FAT16B with LBA addressing.
const TYPE_FAT16B_LBA: u8 = 0x0E;

/// Status byte flag: the partition is active (bootable).
const STATUS_ACTIVE: u8 = 0x80;

/// A parsed MBR partition entry.
struct MbrPartitionEntry {
    status: u8,
    partition_type: u8,
    start_lba: u32,
}

/// Returns whether the partition holds a FAT12/FAT16 filesystem the FAT
/// layer can mount (FAT32 types are not supported).
fn is_fat_partition(entry: &MbrPartitionEntry) -> bool {
    matches!(
        entry.partition_type,
        TYPE_FAT12 | TYPE_FAT16_SMALL | TYPE_FAT16B | TYPE_FAT16B_LBA
    )
}

/// Parses the four primary partition entries from MBR sector data.
fn parse_partition_table(sector_data: &[u8]) -> Vec<MbrPartitionEntry> {
    let mut entries = Vec::new();
    for index in 0..PARTITION_COUNT {
        let offset = PARTITION_TABLE_OFFSET + index * PARTITION_ENTRY_SIZE;
        if offset + PARTITION_ENTRY_SIZE > sector_data.len() {
            break;
        }
        let entry = &sector_data[offset..];
        if entry[4] == 0 {
            continue;
        }
        entries.push(MbrPartitionEntry {
            status: entry[0],
            partition_type: entry[4],
            start_lba: u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]]),
        });
    }
    entries
}

/// Finds the partition offset (LBA) of the first FAT partition in the MBR,
/// preferring an active one. A disk without the 0x55AA boot signature
/// mounts at offset 0 (an unpartitioned volume); a partitioned disk
/// without any FAT12/FAT16 partition is an error.
pub(crate) fn find_mbr_partition_offset(drive_da: u8, disk: &mut dyn DiskIo) -> Result<u32, u16> {
    let sector_data = disk.read_sectors(drive_da, 0, 1).map_err(|_| 0x001Fu16)?;

    if sector_data.len() < SIGNATURE_OFFSET + 2
        || sector_data[SIGNATURE_OFFSET] != 0x55
        || sector_data[SIGNATURE_OFFSET + 1] != 0xAA
    {
        return Ok(0);
    }

    let entries = parse_partition_table(&sector_data);
    if entries.is_empty() {
        return Ok(0);
    }

    let fat_entries: Vec<&MbrPartitionEntry> = entries
        .iter()
        .filter(|entry| is_fat_partition(entry))
        .collect();
    let selected = fat_entries
        .iter()
        .find(|entry| entry.status & STATUS_ACTIVE != 0)
        .or_else(|| fat_entries.first());

    match selected {
        Some(entry) => Ok(entry.start_lba),
        None => Err(0x001F),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-sector in-memory disk serving only the MBR.
    struct MbrDisk {
        sector: Vec<u8>,
    }

    impl DiskIo for MbrDisk {
        fn read_sectors(&mut self, _drive_da: u8, lba: u32, count: u32) -> Result<Vec<u8>, u8> {
            if lba == 0 && count == 1 {
                Ok(self.sector.clone())
            } else {
                Err(0xFF)
            }
        }

        fn write_sectors(&mut self, _drive_da: u8, _lba: u32, _data: &[u8]) -> Result<(), u8> {
            Err(0xFF)
        }

        fn sector_size(&self, _drive_da: u8) -> Option<u16> {
            Some(512)
        }

        fn total_sectors(&self, _drive_da: u8) -> Option<u32> {
            Some(1)
        }

        fn drive_geometry(&self, _drive_da: u8) -> Option<(u16, u8, u8)> {
            Some((1, 16, 63))
        }
    }

    fn blank_mbr() -> Vec<u8> {
        let mut sector = vec![0u8; 512];
        sector[SIGNATURE_OFFSET] = 0x55;
        sector[SIGNATURE_OFFSET + 1] = 0xAA;
        sector
    }

    fn set_entry(sector: &mut [u8], index: usize, status: u8, partition_type: u8, start_lba: u32) {
        let offset = PARTITION_TABLE_OFFSET + index * PARTITION_ENTRY_SIZE;
        sector[offset] = status;
        sector[offset + 4] = partition_type;
        sector[offset + 8..offset + 12].copy_from_slice(&start_lba.to_le_bytes());
    }

    #[test]
    fn missing_signature_mounts_at_offset_zero() {
        let mut disk = MbrDisk {
            sector: vec![0u8; 512],
        };
        assert_eq!(find_mbr_partition_offset(0x80, &mut disk), Ok(0));
    }

    #[test]
    fn empty_table_mounts_at_offset_zero() {
        let mut disk = MbrDisk {
            sector: blank_mbr(),
        };
        assert_eq!(find_mbr_partition_offset(0x80, &mut disk), Ok(0));
    }

    #[test]
    fn first_fat_partition_offset_is_returned() {
        let mut sector = blank_mbr();
        set_entry(&mut sector, 0, 0x00, TYPE_FAT16B, 63);
        let mut disk = MbrDisk { sector };
        assert_eq!(find_mbr_partition_offset(0x80, &mut disk), Ok(63));
    }

    #[test]
    fn active_fat_partition_is_preferred() {
        let mut sector = blank_mbr();
        set_entry(&mut sector, 0, 0x00, TYPE_FAT12, 63);
        set_entry(&mut sector, 1, STATUS_ACTIVE, TYPE_FAT16B_LBA, 2048);
        let mut disk = MbrDisk { sector };
        assert_eq!(find_mbr_partition_offset(0x80, &mut disk), Ok(2048));
    }

    #[test]
    fn non_fat_only_table_is_an_error() {
        let mut sector = blank_mbr();
        // FAT32 (0x0B) and Linux (0x83) partitions cannot be mounted.
        set_entry(&mut sector, 0, STATUS_ACTIVE, 0x0B, 63);
        set_entry(&mut sector, 1, 0x00, 0x83, 4096);
        let mut disk = MbrDisk { sector };
        assert!(find_mbr_partition_offset(0x80, &mut disk).is_err());
    }

    #[test]
    fn all_fat_types_are_accepted() {
        for partition_type in [TYPE_FAT12, TYPE_FAT16_SMALL, TYPE_FAT16B, TYPE_FAT16B_LBA] {
            let mut sector = blank_mbr();
            set_entry(&mut sector, 3, 0x00, partition_type, 129);
            let mut disk = MbrDisk { sector };
            assert_eq!(
                find_mbr_partition_offset(0x80, &mut disk),
                Ok(129),
                "type {partition_type:#04X}"
            );
        }
    }
}
