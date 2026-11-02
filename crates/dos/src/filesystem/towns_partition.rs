//! FM Towns SCSI hard disk partition table.
//!
//! Physical sector 1 of the disk holds the table. It starts with a 6-byte
//! signature, either "Fujitsu" or "Matsushita C" in Shift-JIS. The header word
//! at `0Eh` holds the bytes per physical sector. Ten entries of `30h` bytes
//! follow at `20h`. An entry holds a boot flag, a system type, the start sector
//! and the length in physical sectors and two blank padded names.
//!
//! The `IO.SYS` block driver takes the n-th entry of the MS-DOS type and
//! builds the BPB from the partition size. It ignores the boot record of the
//! partition.

use crate::DiskIo;

/// Physical sector of the partition table.
pub const PARTITION_TABLE_SECTOR: u32 = 1;

/// Signature "Fujitsu" (富士通) in Shift-JIS.
pub const FUJITSU_SIGNATURE: [u8; 6] = [0x95, 0x78, 0x8E, 0x6D, 0x92, 0xCA];

/// Signature "Matsushita C" (松下Ｃ) in Shift-JIS.
pub const MATSUSHITA_SIGNATURE: [u8; 6] = [0x8F, 0xBC, 0x89, 0xBA, 0x82, 0x62];

/// Offset of the first entry in the table sector.
const FIRST_ENTRY_OFFSET: usize = 0x20;

/// Size of one entry.
const ENTRY_SIZE: usize = 0x30;

/// Number of entries in the table.
pub const ENTRY_COUNT: usize = 10;

/// Minimum length of the table sector data.
const TABLE_LENGTH: usize = FIRST_ENTRY_OFFSET + ENTRY_COUNT * ENTRY_SIZE;

/// Entry offset of the system type.
const ENTRY_SYSTEM_TYPE: usize = 0x01;

/// Entry offset of the start sector dword.
const ENTRY_START_SECTOR: usize = 0x02;

/// Entry offset of the length dword.
const ENTRY_SECTOR_COUNT: usize = 0x06;

/// System type of an MS-DOS partition.
pub const SYSTEM_TYPE_MS_DOS: u8 = 0x01;

/// The start sector field uses 24 bits in the IPL and in `IO.SYS`.
const START_SECTOR_MASK: u32 = 0x00FF_FFFF;

/// One entry of the FM Towns partition table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TownsPartitionEntry {
    /// System type (`01h` for MS-DOS).
    pub system_type: u8,
    /// First physical sector of the partition.
    pub start_sector: u32,
    /// Length of the partition in physical sectors.
    pub sector_count: u32,
}

impl TownsPartitionEntry {
    /// Returns true for an MS-DOS partition.
    pub fn is_ms_dos(&self) -> bool {
        self.system_type == SYSTEM_TYPE_MS_DOS
    }
}

/// The parsed partition table sector.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TownsPartitionTable {
    /// The ten entries in table order.
    pub entries: Vec<TownsPartitionEntry>,
}

impl TownsPartitionTable {
    /// Parses the table sector. Returns `None` without a known signature.
    pub fn parse(sector: &[u8]) -> Option<Self> {
        if sector.len() < TABLE_LENGTH {
            return None;
        }
        let signature = &sector[..FUJITSU_SIGNATURE.len()];
        if signature != FUJITSU_SIGNATURE && signature != MATSUSHITA_SIGNATURE {
            return None;
        }

        let entries = (0..ENTRY_COUNT)
            .map(|index| {
                let entry = &sector[FIRST_ENTRY_OFFSET + index * ENTRY_SIZE..];
                TownsPartitionEntry {
                    system_type: entry[ENTRY_SYSTEM_TYPE],
                    start_sector: read_dword(entry, ENTRY_START_SECTOR) & START_SECTOR_MASK,
                    sector_count: read_dword(entry, ENTRY_SECTOR_COUNT),
                }
            })
            .collect();

        Some(Self { entries })
    }

    /// Returns the n-th MS-DOS partition (0-based) in table order.
    pub fn ms_dos_partition(&self, partition_index: u8) -> Option<&TownsPartitionEntry> {
        self.entries
            .iter()
            .filter(|entry| entry.is_ms_dos())
            .nth(usize::from(partition_index))
    }

    /// Returns the number of MS-DOS partitions.
    pub fn ms_dos_partition_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.is_ms_dos())
            .count()
    }
}

/// Reads the partition table of a disk.
pub fn read_partition_table(
    device_id: u8,
    disk: &mut dyn DiskIo,
) -> Result<TownsPartitionTable, u16> {
    let sector = disk
        .read_sectors(device_id, PARTITION_TABLE_SECTOR, 1)
        .map_err(|_| 0x001Fu16)?;
    TownsPartitionTable::parse(&sector).ok_or(0x001F)
}

/// Returns the first physical sector of the n-th MS-DOS partition of a disk.
pub fn find_partition_offset(
    device_id: u8,
    partition_index: u8,
    disk: &mut dyn DiskIo,
) -> Result<u32, u16> {
    let table = read_partition_table(device_id, disk)?;
    table
        .ms_dos_partition(partition_index)
        .map(|entry| entry.start_sector)
        .ok_or(0x000F)
}

/// Reads a little-endian dword.
fn read_dword(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Physical sector size of the test disks.
    const SECTOR_SIZE: usize = 512;

    /// Writes one entry into a table sector.
    fn write_entry(sector: &mut [u8], index: usize, system_type: u8, start: u32, length: u32) {
        let entry = &mut sector[FIRST_ENTRY_OFFSET + index * ENTRY_SIZE..];
        entry[ENTRY_SYSTEM_TYPE] = system_type;
        entry[ENTRY_START_SECTOR..ENTRY_START_SECTOR + 4].copy_from_slice(&start.to_le_bytes());
        entry[ENTRY_SECTOR_COUNT..ENTRY_SECTOR_COUNT + 4].copy_from_slice(&length.to_le_bytes());
    }

    /// Builds a table sector with the given signature and empty entries.
    fn table_sector(signature: [u8; 6]) -> Vec<u8> {
        let mut sector = vec![0u8; SECTOR_SIZE];
        sector[..6].copy_from_slice(&signature);
        sector
    }

    /// Table with the layout of the Towns OS V1.1 L20 disk: one MS-DOS
    /// partition at sector 3 with `7803h` sectors.
    fn single_partition_table() -> Vec<u8> {
        let mut sector = table_sector(FUJITSU_SIGNATURE);
        write_entry(&mut sector, 0, SYSTEM_TYPE_MS_DOS, 3, 0x7803);
        sector
    }

    /// Disk with 512-byte sectors that serves a fixed table sector.
    struct TableDisk {
        table: Vec<u8>,
    }

    impl DiskIo for TableDisk {
        fn read_sectors(&mut self, _device_id: u8, lba: u32, count: u32) -> Result<Vec<u8>, u8> {
            if lba == PARTITION_TABLE_SECTOR && count == 1 {
                Ok(self.table.clone())
            } else {
                Err(0x10)
            }
        }

        fn write_sectors(&mut self, _device_id: u8, _lba: u32, _data: &[u8]) -> Result<(), u8> {
            Err(0x10)
        }

        fn sector_size(&self, _device_id: u8) -> Option<u16> {
            Some(SECTOR_SIZE as u16)
        }

        fn total_sectors(&self, _device_id: u8) -> Option<u32> {
            Some(0x0019_0000)
        }

        fn drive_geometry(&self, _device_id: u8) -> Option<(u16, u8, u8)> {
            None
        }
    }

    #[test]
    fn parses_the_single_partition_table() {
        let table = TownsPartitionTable::parse(&single_partition_table()).unwrap();
        assert_eq!(table.entries.len(), ENTRY_COUNT);
        let entry = &table.entries[0];
        assert!(entry.is_ms_dos());
        assert_eq!(entry.start_sector, 3);
        assert_eq!(entry.sector_count, 0x7803);
        assert_eq!(table.ms_dos_partition_count(), 1);
        assert!(!table.entries[1].is_ms_dos());
    }

    #[test]
    fn accepts_the_matsushita_signature() {
        let mut sector = table_sector(MATSUSHITA_SIGNATURE);
        write_entry(&mut sector, 0, SYSTEM_TYPE_MS_DOS, 0x40, 0x1000);
        let table = TownsPartitionTable::parse(&sector).unwrap();
        assert_eq!(table.ms_dos_partition(0).unwrap().start_sector, 0x40);
    }

    #[test]
    fn rejects_an_unknown_signature_and_a_short_sector() {
        let mut sector = single_partition_table();
        sector[0] = 0x00;
        assert!(TownsPartitionTable::parse(&sector).is_none());
        assert!(
            TownsPartitionTable::parse(&single_partition_table()[..TABLE_LENGTH - 1]).is_none()
        );
    }

    #[test]
    fn partition_index_counts_only_ms_dos_entries() {
        let mut sector = table_sector(FUJITSU_SIGNATURE);
        write_entry(&mut sector, 0, 0x90, 0x10, 0x100);
        write_entry(&mut sector, 1, SYSTEM_TYPE_MS_DOS, 0x200, 0x800);
        write_entry(&mut sector, 2, SYSTEM_TYPE_MS_DOS, 0xA00, 0x800);
        let table = TownsPartitionTable::parse(&sector).unwrap();
        assert_eq!(table.ms_dos_partition_count(), 2);
        assert_eq!(table.ms_dos_partition(0).unwrap().start_sector, 0x200);
        assert_eq!(table.ms_dos_partition(1).unwrap().start_sector, 0xA00);
        assert!(table.ms_dos_partition(2).is_none());
    }

    #[test]
    fn start_sector_uses_24_bits() {
        let mut sector = table_sector(FUJITSU_SIGNATURE);
        write_entry(&mut sector, 0, SYSTEM_TYPE_MS_DOS, 0x1234_5678, 0x800);
        let table = TownsPartitionTable::parse(&sector).unwrap();
        assert_eq!(table.entries[0].start_sector, 0x0034_5678);
    }

    #[test]
    fn finds_the_partition_offset_on_a_disk() {
        let mut disk = TableDisk {
            table: single_partition_table(),
        };
        assert_eq!(find_partition_offset(0xB0, 0, &mut disk), Ok(3));
        assert_eq!(find_partition_offset(0xB0, 1, &mut disk), Err(0x000F));
        disk.table[0] = 0;
        assert_eq!(find_partition_offset(0xB0, 0, &mut disk), Err(0x001F));
    }
}
