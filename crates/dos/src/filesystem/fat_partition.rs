//! PC-98 HDD partition table parsing.

use crate::DiskIo;

const PARTITION_ENTRY_SIZE: usize = 32;
const MAX_PARTITIONS: usize = 16;

/// A parsed PC-98 partition table entry.
pub(crate) struct Pc98PartitionEntry {
    pub mid: u8,
    pub sid: u8,
    pub data_start_sector: u8,
    pub data_start_head: u8,
    pub data_start_cylinder: u16,
}

/// Returns true if this is an active DOS partition.
fn is_dos_partition(entry: &Pc98PartitionEntry) -> bool {
    // sid bit 7 must be set (active) and mid type nibble in 0x20-0x2F range (DOS/Windows).
    // mid bit 7 is the bootable flag, so mask it out before checking the type.
    (entry.sid & 0x80 != 0) && (entry.mid & 0x70 == 0x20)
}

/// Parses partition entries from sector 1 data.
fn parse_partition_table(sector_data: &[u8]) -> Vec<Pc98PartitionEntry> {
    let mut entries = Vec::new();
    for i in 0..MAX_PARTITIONS {
        let offset = i * PARTITION_ENTRY_SIZE;
        if offset + PARTITION_ENTRY_SIZE > sector_data.len() {
            break;
        }
        let d = &sector_data[offset..];
        if d[0] == 0 && d[1] == 0 {
            break; // empty entry
        }
        let mut name = [0u8; 16];
        name.copy_from_slice(&d[16..32]);
        entries.push(Pc98PartitionEntry {
            mid: d[0],
            sid: d[1],
            data_start_sector: d[8],
            data_start_head: d[9],
            data_start_cylinder: u16::from_le_bytes([d[10], d[11]]),
        });
    }
    entries
}

/// Converts CHS to LBA given geometry.
fn chs_to_lba(cylinder: u16, head: u8, sector: u8, heads: u8, sectors_per_track: u8) -> u32 {
    (cylinder as u32 * heads as u32 + head as u32) * sectors_per_track as u32 + sector as u32
}

/// Finds the partition offset (LBA) for the first active DOS partition on an HDD.
pub(crate) fn find_partition_offset(drive_da: u8, disk: &mut dyn DiskIo) -> Result<u32, u16> {
    // Read sector 1 (partition table)
    let sector_data = disk.read_sectors(drive_da, 1, 1).map_err(|_| 0x001Fu16)?;

    let entries = parse_partition_table(&sector_data);

    // Get drive geometry for CHS conversion
    let (_, heads, spt) = disk.drive_geometry(drive_da).ok_or(0x001Fu16)?;

    for entry in &entries {
        if is_dos_partition(entry) {
            return Ok(chs_to_lba(
                entry.data_start_cylinder,
                entry.data_start_head,
                entry.data_start_sector,
                heads,
                spt,
            ));
        }
    }

    // No active DOS partition found; try offset 0 (maybe the entire disk is formatted)
    Ok(0)
}
