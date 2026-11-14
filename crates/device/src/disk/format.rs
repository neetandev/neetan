//! FAT formatting: partition table, BPB/boot sector, FAT, and root
//! directory layout for FAT12/FAT16 volumes on hard disks, and the shared
//! boot sector writers of the floppy formatter.

use super::{
    HddGeometry, HddImage,
    pc98_ide::{self, ENTRY_END_OFFSET, ENTRY_IPL_OFFSET, ENTRY_START_OFFSET, cylinder_address},
    pc98_sasi,
};

/// The partition-table style written to the first sectors of a hard disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartitionTableType {
    /// PC-98 partition table (MID/SID entry at sector 1, IPL at sector 0).
    Pc98,
    /// PC/AT master boot record (partition entry plus 0x55AA at sector 0).
    At,
}

/// FAT BPB fields resolved for a given volume geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BpbParams {
    /// Bytes per sector.
    pub bytes_per_sector: u16,
    /// Sectors per allocation cluster.
    pub sectors_per_cluster: u8,
    /// Reserved sectors before the first FAT.
    pub reserved_sectors: u16,
    /// Number of FAT copies.
    pub num_fats: u8,
    /// Root directory entry count.
    pub root_entry_count: u16,
    /// FAT media descriptor byte.
    pub media_descriptor: u8,
    /// Sectors occupied by one FAT.
    pub sectors_per_fat: u16,
    /// Whether the volume is FAT16 (otherwise FAT12).
    pub is_fat16: bool,
}

/// Failure while applying a format to an in-memory image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatError {
    /// The geometry has too few sectors to hold a partition and a FAT volume.
    ImageTooSmall,
    /// A computed sector fell outside the image.
    SectorOutOfRange,
    /// The disk geometry has no FAT format on the machine family.
    UnsupportedGeometry,
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FormatError::ImageTooSmall => f.write_str("hard disk image is too small to format"),
            FormatError::SectorOutOfRange => f.write_str("format wrote a sector out of range"),
            FormatError::UnsupportedGeometry => {
                f.write_str("the disk geometry has no format on this machine family")
            }
        }
    }
}

impl std::error::Error for FormatError {}

/// Number of FAT sectors needed to describe `cluster_count` clusters.
fn fat_sectors_for(cluster_count: u32, bytes_per_sector: u16, is_fat16: bool) -> u16 {
    let fat_bytes = if is_fat16 {
        (cluster_count as u64 + 2) * 2
    } else {
        ((cluster_count as u64 + 2) * 3).div_ceil(2)
    };
    fat_bytes.div_ceil(bytes_per_sector as u64) as u16
}

/// Iteratively solves the FAT size and cluster count for a partition.
fn solve_hdd_fat_layout(
    partition_sectors: u32,
    bytes_per_sector: u16,
    sectors_per_cluster: u8,
    reserved_sectors: u16,
    num_fats: u8,
    root_entry_count: u16,
    is_fat16: bool,
) -> (u32, u16) {
    let root_dir_sectors = (root_entry_count as u32 * 32).div_ceil(bytes_per_sector as u32);
    let mut sectors_per_fat = 1u16;

    loop {
        let system_sectors =
            reserved_sectors as u32 + num_fats as u32 * sectors_per_fat as u32 + root_dir_sectors;
        let data_sectors = partition_sectors.saturating_sub(system_sectors);
        let cluster_count = data_sectors / sectors_per_cluster as u32;
        let needed = fat_sectors_for(cluster_count, bytes_per_sector, is_fat16);
        if needed == sectors_per_fat {
            return (cluster_count, sectors_per_fat);
        }
        sectors_per_fat = needed;
    }
}

/// Resolves the BPB of a FAT volume of `partition_sectors` sectors that
/// spans a hard disk from its second track. The volume takes the physical
/// sector size and the FAT16 cluster size steps of the MS-DOS FORMAT of the
/// PC/AT: 4 sectors per cluster up to 128 MiB, 8 up to 256 MiB, 16 up to
/// 512 MiB and 32 above.
pub fn hdd_bpb_params(sector_size: u16, partition_sectors: u32) -> BpbParams {
    let volume_bytes = partition_sectors as u64 * sector_size as u64;
    let sectors_per_cluster: u8 = if volume_bytes <= (128 << 20) {
        4
    } else if volume_bytes <= (256 << 20) {
        8
    } else if volume_bytes <= (512 << 20) {
        16
    } else {
        32
    };
    whole_partition_bpb(sector_size, partition_sectors, sectors_per_cluster)
}

/// Resolves the BPB of a FAT volume of `partition_sectors` sectors of
/// `sector_size` bytes with `sectors_per_cluster`: one reserved sector, two
/// FAT copies, 512 root entries and the media byte `F8h`. The volume is
/// FAT16 from 4085 clusters.
pub(super) fn whole_partition_bpb(
    sector_size: u16,
    partition_sectors: u32,
    sectors_per_cluster: u8,
) -> BpbParams {
    let bytes_per_sector = sector_size;
    let reserved_sectors: u16 = 1;
    let num_fats: u8 = 2;
    let root_entry_count: u16 = 512;

    let (fat12_clusters, fat12_sectors_per_fat) = solve_hdd_fat_layout(
        partition_sectors,
        bytes_per_sector,
        sectors_per_cluster,
        reserved_sectors,
        num_fats,
        root_entry_count,
        false,
    );
    let (is_fat16, sectors_per_fat) = if fat12_clusters >= 4085 {
        let (_, fat16_sectors_per_fat) = solve_hdd_fat_layout(
            partition_sectors,
            bytes_per_sector,
            sectors_per_cluster,
            reserved_sectors,
            num_fats,
            root_entry_count,
            true,
        );
        (true, fat16_sectors_per_fat)
    } else {
        (false, fat12_sectors_per_fat)
    };

    BpbParams {
        bytes_per_sector,
        sectors_per_cluster,
        reserved_sectors,
        num_fats,
        root_entry_count,
        media_descriptor: 0xF8,
        sectors_per_fat,
        is_fat16,
    }
}

/// Partition name of the partition neetan writes into a PC-98 partition
/// table.
pub const PC98_PARTITION_NAME: &[u8; 16] = b"NEETAN          ";

/// Builds the PC-98 partition-table sector (disk sector 1). One DOS
/// partition spanning the disk, active, data starting on the second track.
pub fn pc98_partition_table_sector(
    sector_size: u16,
    total_sectors: u32,
    heads: u8,
    sectors_per_track: u8,
) -> Vec<u8> {
    let mut sector = vec![0u8; sector_size as usize];

    // MID: 0x21 = DOS type (0x20) | subtype 0x01 (FAT16), not bootable (bit 7 clear).
    sector[0] = 0x21;
    // SID: 0x81 = active (bit 7) | system ID 0x01.
    sector[1] = 0x81;

    // Data start CHS: cylinder 0, head 1, sector 0 (first track after IPL/table).
    sector[8] = 0;
    sector[9] = 1;
    sector[10] = 0;
    sector[11] = 0;

    let last_sector = total_sectors - 1;
    let sectors_per_cylinder = heads as u32 * sectors_per_track as u32;
    let end_cylinder = last_sector / sectors_per_cylinder;
    let remainder = last_sector % sectors_per_cylinder;
    let end_head = remainder / sectors_per_track as u32;
    let end_sector = remainder % sectors_per_track as u32;
    sector[12] = end_sector as u8;
    sector[13] = end_head as u8;
    sector[14] = end_cylinder as u8;
    sector[15] = (end_cylinder >> 8) as u8;

    // Partition name (16 bytes, space-padded).
    sector[16..32].copy_from_slice(PC98_PARTITION_NAME);

    sector
}

/// First cylinder of the partition in the NEC layout of a PC-98 fixed disk.
pub const PC98_NEC_PARTITION_START_CYLINDER: u16 = 1;

/// MID byte of the partition neetan writes into a PC-98 partition table.
const PC98_PARTITION_MID: u8 = 0x21;

/// SID byte of an active partition with a FAT12 volume.
const PC98_SID_FAT12: u8 = 0x81;

/// SID byte of an active partition with a FAT16 volume of at most 65535
/// sectors.
const PC98_SID_FAT16: u8 = 0x91;

/// SID byte of an active partition with a FAT16 volume that stores its size
/// in the 32-bit total sector field.
const PC98_SID_FAT16_LARGE: u8 = 0xA1;

/// Partition table entry offset of the MID byte.
const PC98_ENTRY_MID_OFFSET: usize = 0;

/// Partition table entry offset of the SID byte.
const PC98_ENTRY_SID_OFFSET: usize = 1;

/// Partition table entry offset of the partition name.
const PC98_ENTRY_NAME_OFFSET: usize = 16;

/// Bytes of one size unit of a PC-98 fixed disk sized in units of 990 KiB.
const NEC_SMALL_UNIT_BYTES: u64 = 1_013_760;

/// Bytes of one size unit of a PC-98 fixed disk sized in units of 1 MiB.
const NEC_LARGE_UNIT_BYTES: u64 = 1_048_576;

/// Cylinders the NEC FORMAT never allocates: cylinder 0 with the IPL and the
/// partition table, and the last cylinder.
const NEC_UNUSED_CYLINDERS: u16 = 2;

/// Highest cylinder count of a disk whose volumes are FAT12 at every size.
const NEC_FAT12_ONLY_MAXIMUM_CYLINDERS: u16 = 615;

/// Lowest count of 990 KiB units on the usable cylinders of a disk sized in
/// units of 1 MiB.
const NEC_LARGE_DISK_MINIMUM_SMALL_UNITS: u64 = 151;

/// Largest FAT12 volume in size units on a disk with FAT16 volumes.
const NEC_FAT12_MAXIMUM_UNITS: u64 = 10;

/// Largest FAT16 volume in size units with 1024-byte logical sectors.
const NEC_FAT16_KIBIBYTE_SECTOR_MAXIMUM_UNITS: u64 = 64;

/// Largest FAT16 volume in size units with 2048-byte logical sectors.
const NEC_FAT16_TWO_KIBIBYTE_SECTOR_MAXIMUM_UNITS: u64 = 128;

/// Largest volume in size units. A FAT16 volume of 2047 MiB fills 65504
/// clusters of 32 KiB.
const NEC_MAXIMUM_UNITS: u64 = 2047;

/// Logical sector size of a FAT12 volume and of a FAT16 volume up to 64 units.
const NEC_KIBIBYTE_SECTOR_SIZE: u16 = 1024;

/// Logical sector size of a FAT16 volume of 65 to 128 units.
const NEC_TWO_KIBIBYTE_SECTOR_SIZE: u16 = 2048;

/// Logical sector size of a FAT16 volume of more than 128 units.
const NEC_LARGE_VOLUME_SECTOR_SIZE: u16 = 512;

/// Smallest logical sector size of the NEC layout. The physical sector size
/// of the disk divides it, so every logical sector holds whole physical
/// sectors.
const NEC_SMALLEST_SECTOR_SIZE: u16 = 512;

/// Bytes of the reserved area in front of the first FAT. The area holds at
/// least one logical sector.
const NEC_RESERVED_AREA_BYTES: u32 = 1024;

/// Bytes at the start of the reserved area that hold the BPB, the boot code
/// and the `55 AA` signature at their end.
const NEC_BOOT_BLOCK_BYTES: usize = 512;

/// Number of FAT copies.
const NEC_FAT_COUNT: u8 = 2;

/// Media descriptor byte of the BPB of a fixed disk.
const NEC_MEDIA_DESCRIPTOR: u8 = 0xF8;

/// FAT ID byte in entry 0 of both FAT copies.
const NEC_FAT_ID: u8 = 0xFE;

/// Largest cluster in bytes.
const NEC_MAXIMUM_CLUSTER_BYTES: u32 = 32768;

/// Highest data cluster count of a FAT12 volume.
const FAT12_MAXIMUM_CLUSTERS: u32 = 4084;

/// Highest data cluster count of a FAT16 volume.
const FAT16_MAXIMUM_CLUSTERS: u32 = 65524;

/// Upper unit bound and root directory entries of each volume size step.
const NEC_ROOT_ENTRY_STEPS: [(u64, u16); 8] = [
    (5, 512),
    (10, 768),
    (15, 1024),
    (20, 1280),
    (25, 1536),
    (30, 1792),
    (35, 2560),
    (u64::MAX, 3072),
];

/// Upper unit bound and sectors per cluster of each FAT12 volume size step on
/// a disk whose volumes are FAT12 at every size.
const NEC_FAT12_ONLY_CLUSTER_STEPS: [(u64, u8); 4] = [(5, 2), (15, 4), (30, 8), (u64::MAX, 16)];

/// Upper unit bound and sectors per cluster of each FAT12 volume size step on
/// a disk with FAT16 volumes.
const NEC_FAT12_CLUSTER_STEPS: [(u64, u8); 2] = [(5, 2), (u64::MAX, 4)];

/// Sectors per cluster of a FAT16 volume with 1024 or 2048-byte sectors.
const NEC_FAT16_CLUSTER_STEPS: [(u64, u8); 1] = [(u64::MAX, 2)];

/// Upper unit bound and sectors per cluster of each FAT16 volume size step
/// with 512-byte sectors.
const NEC_FAT16_LARGE_VOLUME_CLUSTER_STEPS: [(u64, u8); 2] = [(255, 8), (u64::MAX, 16)];

/// The size class of a PC-98 fixed disk in the NEC FORMAT.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NecDiskClass {
    /// At most 615 cylinders: FAT12 at every size, sized in 990 KiB units.
    Fat12Only,
    /// FAT12 up to 10 units and FAT16 above, sized in 990 KiB units.
    Small,
    /// FAT12 up to 10 units and FAT16 above, sized in 1 MiB units.
    Large,
}

/// Returns the value of the first step whose upper unit bound holds `units`.
fn step_value<T: Copy>(steps: &[(u64, T)], units: u64) -> T {
    steps
        .iter()
        .find(|(upper_bound, _)| units <= *upper_bound)
        .map_or(steps[steps.len() - 1].1, |(_, value)| *value)
}

/// The layout the NEC MS-DOS 6.20 FORMAT writes on a PC-98 fixed disk: the
/// partition extent, the SID of the partition entry and the BPB of the FAT
/// volume. The partition starts on cylinder 1. The volume can be smaller
/// than the partition. Its logical sector size can differ from the
/// physical sector size of the disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pc98NecLayout {
    /// Bytes per physical sector of the disk.
    pub physical_sector_size: u16,
    /// Heads of the disk geometry.
    pub heads: u8,
    /// Sectors per track of the disk geometry.
    pub sectors_per_track: u8,
    /// Last cylinder of the partition, inclusive.
    pub end_cylinder: u16,
    /// SID byte of the partition table entry.
    pub system_id: u8,
    /// BPB of the volume in logical sectors.
    pub bpb: BpbParams,
    /// Total logical sectors of the volume.
    pub volume_sectors: u32,
}

/// Resolves the layout of the NEC MS-DOS 6.20 FORMAT for a PC-98 fixed disk
/// of `geometry`, the geometry the BIOS addresses the disk in. The partition
/// takes the largest size the NEC FORMAT offers. The rules were measured
/// against the NEC MS-DOS 6.20 FORMAT on SASI and IDE disks.
pub fn pc98_nec_layout(geometry: HddGeometry) -> Result<Pc98NecLayout, FormatError> {
    let sector_size = u32::from(geometry.sector_size);
    if sector_size == 0 || !u32::from(NEC_SMALLEST_SECTOR_SIZE).is_multiple_of(sector_size) {
        return Err(FormatError::UnsupportedGeometry);
    }
    let cylinder_bytes = u64::from(geometry.heads)
        * u64::from(geometry.sectors_per_track)
        * u64::from(geometry.sector_size);
    if cylinder_bytes == 0 {
        return Err(FormatError::ImageTooSmall);
    }
    let usable_cylinders = u64::from(geometry.cylinders.saturating_sub(NEC_UNUSED_CYLINDERS));
    let cylinders_per_small_unit = NEC_SMALL_UNIT_BYTES.div_ceil(cylinder_bytes);
    let small_units = usable_cylinders / cylinders_per_small_unit;
    let class = if geometry.cylinders <= NEC_FAT12_ONLY_MAXIMUM_CYLINDERS {
        NecDiskClass::Fat12Only
    } else if small_units >= NEC_LARGE_DISK_MINIMUM_SMALL_UNITS {
        NecDiskClass::Large
    } else {
        NecDiskClass::Small
    };
    let (unit_bytes, offered_units) = match class {
        NecDiskClass::Fat12Only | NecDiskClass::Small => (NEC_SMALL_UNIT_BYTES, small_units),
        NecDiskClass::Large => (
            NEC_LARGE_UNIT_BYTES,
            usable_cylinders * cylinder_bytes / NEC_LARGE_UNIT_BYTES,
        ),
    };
    let units = offered_units.min(NEC_MAXIMUM_UNITS);
    if units == 0 {
        return Err(FormatError::ImageTooSmall);
    }

    let partition_cylinders = match class {
        NecDiskClass::Fat12Only | NecDiskClass::Small => units * cylinders_per_small_unit,
        NecDiskClass::Large => (units * unit_bytes).div_ceil(cylinder_bytes),
    };
    let is_fat16 = class != NecDiskClass::Fat12Only && units > NEC_FAT12_MAXIMUM_UNITS;
    let bytes_per_sector = if !is_fat16 || units <= NEC_FAT16_KIBIBYTE_SECTOR_MAXIMUM_UNITS {
        NEC_KIBIBYTE_SECTOR_SIZE
    } else if units <= NEC_FAT16_TWO_KIBIBYTE_SECTOR_MAXIMUM_UNITS {
        NEC_TWO_KIBIBYTE_SECTOR_SIZE
    } else {
        NEC_LARGE_VOLUME_SECTOR_SIZE
    };
    let volume_logical_sectors = units * unit_bytes / u64::from(bytes_per_sector);
    let volume_sectors = if bytes_per_sector == NEC_LARGE_VOLUME_SECTOR_SIZE {
        volume_logical_sectors
    } else {
        volume_logical_sectors.min(u64::from(u16::MAX))
    } as u32;

    let cluster_steps: &[(u64, u8)] = match (class, is_fat16, bytes_per_sector) {
        (NecDiskClass::Fat12Only, _, _) => &NEC_FAT12_ONLY_CLUSTER_STEPS,
        (_, false, _) => &NEC_FAT12_CLUSTER_STEPS,
        (_, true, NEC_LARGE_VOLUME_SECTOR_SIZE) => &NEC_FAT16_LARGE_VOLUME_CLUSTER_STEPS,
        (_, true, _) => &NEC_FAT16_CLUSTER_STEPS,
    };
    let maximum_clusters = if is_fat16 {
        FAT16_MAXIMUM_CLUSTERS
    } else {
        FAT12_MAXIMUM_CLUSTERS
    };
    let maximum_sectors_per_cluster = NEC_MAXIMUM_CLUSTER_BYTES / u32::from(bytes_per_sector);
    let mut sectors_per_cluster = step_value(cluster_steps, units);
    while volume_sectors / u32::from(sectors_per_cluster) > maximum_clusters
        && u32::from(sectors_per_cluster) < maximum_sectors_per_cluster
    {
        sectors_per_cluster *= 2;
    }

    let reserved_sectors = NEC_RESERVED_AREA_BYTES.div_ceil(u32::from(bytes_per_sector)) as u16;
    let root_entry_count = step_value(&NEC_ROOT_ENTRY_STEPS, units);
    let sectors_per_fat = match class {
        NecDiskClass::Fat12Only | NecDiskClass::Small => fat_sectors_for(
            volume_sectors / u32::from(sectors_per_cluster),
            bytes_per_sector,
            is_fat16,
        ),
        NecDiskClass::Large => {
            solve_hdd_fat_layout(
                volume_sectors,
                bytes_per_sector,
                sectors_per_cluster,
                reserved_sectors,
                NEC_FAT_COUNT,
                root_entry_count,
                is_fat16,
            )
            .1
        }
    };
    let system_id = if !is_fat16 {
        PC98_SID_FAT12
    } else if volume_sectors <= u32::from(u16::MAX) {
        PC98_SID_FAT16
    } else {
        PC98_SID_FAT16_LARGE
    };

    Ok(Pc98NecLayout {
        physical_sector_size: geometry.sector_size,
        heads: geometry.heads,
        sectors_per_track: geometry.sectors_per_track,
        end_cylinder: u16::try_from(
            u64::from(PC98_NEC_PARTITION_START_CYLINDER) + partition_cylinders - 1,
        )
        .map_err(|_| FormatError::ImageTooSmall)?,
        system_id,
        bpb: BpbParams {
            bytes_per_sector,
            sectors_per_cluster,
            reserved_sectors,
            num_fats: NEC_FAT_COUNT,
            root_entry_count,
            media_descriptor: NEC_MEDIA_DESCRIPTOR,
            sectors_per_fat,
            is_fat16,
        },
        volume_sectors,
    })
}

impl Pc98NecLayout {
    /// Physical sectors of one cylinder.
    fn sectors_per_cylinder(&self) -> u32 {
        u32::from(self.heads) * u32::from(self.sectors_per_track)
    }

    /// Physical sectors in one logical sector of the volume.
    pub fn physical_sectors_per_logical_sector(&self) -> u32 {
        u32::from(self.bpb.bytes_per_sector) / u32::from(self.physical_sector_size)
    }

    /// First physical sector of the partition, the hidden sectors of the
    /// BPB.
    pub fn partition_start(&self) -> u32 {
        u32::from(PC98_NEC_PARTITION_START_CYLINDER) * self.sectors_per_cylinder()
    }

    /// Physical sectors of the partition.
    pub fn partition_sectors(&self) -> u32 {
        u32::from(self.end_cylinder + 1 - PC98_NEC_PARTITION_START_CYLINDER)
            * self.sectors_per_cylinder()
    }

    /// Logical sectors of the root directory.
    pub fn root_directory_sectors(&self) -> u32 {
        (u32::from(self.bpb.root_entry_count) * 32).div_ceil(u32::from(self.bpb.bytes_per_sector))
    }

    /// Returns the physical sector of the logical sector `logical_sector` of
    /// the volume.
    fn physical_sector(&self, logical_sector: u32) -> u32 {
        self.partition_start() + logical_sector * self.physical_sectors_per_logical_sector()
    }

    /// Physical sector of the first sector of FAT copy `copy`.
    pub fn fat_lba(&self, copy: u8) -> u32 {
        self.physical_sector(
            u32::from(self.bpb.reserved_sectors)
                + u32::from(copy) * u32::from(self.bpb.sectors_per_fat),
        )
    }

    /// Physical sector of the first root directory sector.
    pub fn root_directory_lba(&self) -> u32 {
        self.fat_lba(self.bpb.num_fats)
    }

    /// Physical sector of the first sector of the data area.
    pub fn data_area_lba(&self) -> u32 {
        self.root_directory_lba()
            + self.root_directory_sectors() * self.physical_sectors_per_logical_sector()
    }

    /// Logical sectors of the data area.
    pub fn data_sectors(&self) -> u32 {
        let system_sectors = u32::from(self.bpb.reserved_sectors)
            + u32::from(self.bpb.num_fats) * u32::from(self.bpb.sectors_per_fat)
            + self.root_directory_sectors();
        self.volume_sectors.saturating_sub(system_sectors)
    }

    /// Builds the partition table sector (disk sector 1): one active DOS
    /// partition with its IPL and data on cylinder 1 and its data end on the
    /// whole cylinder `end_cylinder`.
    pub fn partition_table_sector(&self) -> Vec<u8> {
        let mut sector = vec![0u8; usize::from(self.physical_sector_size)];
        sector[PC98_ENTRY_MID_OFFSET] = PC98_PARTITION_MID;
        sector[PC98_ENTRY_SID_OFFSET] = self.system_id;
        let start = cylinder_address(PC98_NEC_PARTITION_START_CYLINDER);
        sector[ENTRY_IPL_OFFSET..ENTRY_IPL_OFFSET + 4].copy_from_slice(&start);
        sector[ENTRY_START_OFFSET..ENTRY_START_OFFSET + 4].copy_from_slice(&start);
        sector[ENTRY_END_OFFSET..ENTRY_END_OFFSET + 4]
            .copy_from_slice(&cylinder_address(self.end_cylinder));
        sector[PC98_ENTRY_NAME_OFFSET..PC98_ENTRY_NAME_OFFSET + PC98_PARTITION_NAME.len()]
            .copy_from_slice(PC98_PARTITION_NAME);
        sector
    }

    /// Builds the reserved area of the volume. Its first 512 bytes hold the
    /// BPB, the extended BPB, the parking loop and the `55 AA` signature at
    /// offset 510, the place of the signature in a boot sector of 512
    /// bytes. The rest of the area is zero.
    pub fn reserved_area(&self) -> Vec<u8> {
        let mut area = vec![
            0u8;
            usize::from(self.bpb.reserved_sectors)
                * usize::from(self.bpb.bytes_per_sector)
        ];
        let boot_block = &mut area[..NEC_BOOT_BLOCK_BYTES];
        write_bpb_fields(
            boot_block,
            DOS_BOOT_JUMP,
            &self.bpb,
            self.volume_sectors,
            self.sectors_per_track,
            self.heads,
            self.partition_start(),
        );
        write_extended_bpb_and_signature(boot_block, &self.bpb, HDD_PHYSICAL_DRIVE_NUMBER);
        area
    }

    /// Builds one FAT copy with the FAT ID `FEh` in entry 0, the end of chain
    /// marker in entry 1 and every other entry free.
    pub fn fat(&self) -> Vec<u8> {
        let mut fat = fat_region(&self.bpb, self.bpb.bytes_per_sector);
        fat[0] = NEC_FAT_ID;
        fat
    }

    /// Builds the empty root directory.
    pub fn root_directory(&self) -> Vec<u8> {
        vec![0u8; self.root_directory_sectors() as usize * usize::from(self.bpb.bytes_per_sector)]
    }
}

/// BIOS drive number of the first hard disk, stored in the extended BPB.
const HDD_PHYSICAL_DRIVE_NUMBER: u8 = 0x80;

/// Extended BPB marker: the serial number, label, and type fields are valid.
const EXTENDED_BOOT_SIGNATURE: u8 = 0x29;

/// Fixed volume serial number, keeping formatted images deterministic.
const VOLUME_SERIAL_NUMBER: u32 = 0x4E45_5441;

/// Master boot record bootstrap: relocates itself from 0000:7C00 to
/// 0000:0600, finds the active partition entry, reads its first sector to
/// 0000:7C00 through INT 13h using the entry's CHS fields, verifies the
/// 0x55AA signature, and jumps to it with DS:SI at the entry. Errors park in
/// a self-loop.
#[rustfmt::skip]
const AT_MBR_BOOTSTRAP: [u8; 77] = [
    0xFA,                               // CLI
    0x33, 0xC0,                         // XOR AX, AX
    0x8E, 0xD0,                         // MOV SS, AX
    0xBC, 0x00, 0x7C,                   // MOV SP, 0x7C00
    0xFB,                               // STI
    0x8E, 0xD8,                         // MOV DS, AX
    0x8E, 0xC0,                         // MOV ES, AX
    0xBE, 0x00, 0x7C,                   // MOV SI, 0x7C00
    0xBF, 0x00, 0x06,                   // MOV DI, 0x0600
    0xB9, 0x00, 0x01,                   // MOV CX, 0x0100
    0xFC,                               // CLD
    0xF3, 0xA5,                         // REP MOVSW
    0xEA, 0x1E, 0x06, 0x00, 0x00,       // JMP 0000:061E (relocated copy)
    0xBE, 0xBE, 0x07,                   // MOV SI, 0x07BE (first relocated entry)
    0xB9, 0x04, 0x00,                   // MOV CX, 4
    0x80, 0x3C, 0x80,                   // CMP BYTE [SI], 0x80
    0x74, 0x08,                         // JE found
    0x83, 0xC6, 0x10,                   // ADD SI, 16
    0xE2, 0xF6,                         // LOOP the entry scan
    0xEB, 0xFE,                         // JMP $ (park on failure)
    0x90,                               // NOP
    0x8B, 0x14,                         // MOV DX, [SI] (DL drive, DH head)
    0x8B, 0x4C, 0x02,                   // MOV CX, [SI+2] (cylinder/sector)
    0xBB, 0x00, 0x7C,                   // MOV BX, 0x7C00
    0xB8, 0x01, 0x02,                   // MOV AX, 0x0201 (read one sector)
    0xCD, 0x13,                         // INT 0x13
    0x72, 0xEE,                         // JC park
    0x81, 0x3E, 0xFE, 0x7D, 0x55, 0xAA, // CMP WORD [0x7DFE], 0xAA55
    0x75, 0xE6,                         // JNE park
    0xEA, 0x00, 0x7C, 0x00, 0x00,       // JMP 0000:7C00
];

/// Packs an LBA into the three INT 13h CHS bytes of a partition table entry
/// (head, sector with cylinder high bits, cylinder low byte). Cylinders past
/// the 10-bit limit clamp like FDISK.
fn partition_entry_chs(lba: u32, heads: u8, sectors_per_track: u8) -> [u8; 3] {
    let sectors_per_cylinder = u32::from(heads) * u32::from(sectors_per_track);
    let cylinder = (lba / sectors_per_cylinder).min(1023);
    let head = (lba / u32::from(sectors_per_track)) % u32::from(heads);
    let sector = lba % u32::from(sectors_per_track) + 1;
    [
        head as u8,
        sector as u8 | ((cylinder >> 2) & 0xC0) as u8,
        (cylinder & 0xFF) as u8,
    ]
}

/// Builds the PC/AT master boot record (disk sector 0). One active FAT16B
/// partition starting at `partition_offset`, the standard bootstrap that
/// chain-loads the active partition's boot sector, and the 0x55AA signature.
pub fn at_master_boot_record(
    sector_size: u16,
    partition_offset: u32,
    partition_sectors: u32,
    heads: u8,
    sectors_per_track: u8,
) -> Vec<u8> {
    /// Byte offset of the first partition entry in the MBR.
    const PARTITION_TABLE_OFFSET: usize = 0x1BE;
    /// Partition status: active/bootable.
    const STATUS_ACTIVE: u8 = 0x80;
    /// Partition type: FAT16B.
    const TYPE_FAT16B: u8 = 0x06;

    let mut sector = vec![0u8; sector_size as usize];
    sector[..AT_MBR_BOOTSTRAP.len()].copy_from_slice(&AT_MBR_BOOTSTRAP);

    let entry = PARTITION_TABLE_OFFSET;
    let start_chs = partition_entry_chs(partition_offset, heads, sectors_per_track);
    let end_chs = partition_entry_chs(
        partition_offset + partition_sectors - 1,
        heads,
        sectors_per_track,
    );
    sector[entry] = STATUS_ACTIVE;
    sector[entry + 1..entry + 4].copy_from_slice(&start_chs);
    sector[entry + 4] = TYPE_FAT16B;
    sector[entry + 5..entry + 8].copy_from_slice(&end_chs);
    sector[entry + 8..entry + 12].copy_from_slice(&partition_offset.to_le_bytes());
    sector[entry + 12..entry + 16].copy_from_slice(&partition_sectors.to_le_bytes());

    sector[510] = 0x55;
    sector[511] = 0xAA;

    sector
}

/// Writes the jump, the OEM name and the DOS 3.31 BPB of a FAT boot sector
/// into `boot`. A volume of more than 65535 sectors stores its size in the
/// 32-bit total sector field.
pub(crate) fn write_bpb_fields(
    boot: &mut [u8],
    jump: [u8; 3],
    bpb: &BpbParams,
    total_sectors: u32,
    sectors_per_track: u8,
    heads: u8,
    hidden_sectors: u32,
) {
    boot[0..3].copy_from_slice(&jump);
    boot[3..11].copy_from_slice(b"NEETAN  ");
    boot[11..13].copy_from_slice(&bpb.bytes_per_sector.to_le_bytes());
    boot[13] = bpb.sectors_per_cluster;
    boot[14..16].copy_from_slice(&bpb.reserved_sectors.to_le_bytes());
    boot[16] = bpb.num_fats;
    boot[17..19].copy_from_slice(&bpb.root_entry_count.to_le_bytes());
    if total_sectors <= 0xFFFF {
        boot[19..21].copy_from_slice(&(total_sectors as u16).to_le_bytes());
    } else {
        boot[19..21].copy_from_slice(&0u16.to_le_bytes());
        boot[32..36].copy_from_slice(&total_sectors.to_le_bytes());
    }
    boot[21] = bpb.media_descriptor;
    boot[22..24].copy_from_slice(&bpb.sectors_per_fat.to_le_bytes());
    boot[24..26].copy_from_slice(&(sectors_per_track as u16).to_le_bytes());
    boot[26..28].copy_from_slice(&(heads as u16).to_le_bytes());
    boot[28..32].copy_from_slice(&hidden_sectors.to_le_bytes());
}

/// Writes the extended BPB with the BIOS drive number `drive_number`, the
/// parking loop at the target of the `EB 3C` jump and the `55 AA` signature
/// at the end of `boot`.
pub(crate) fn write_extended_bpb_and_signature(boot: &mut [u8], bpb: &BpbParams, drive_number: u8) {
    // The extended BPB and the boot signature make DOS accept the BPB when
    // it mounts the volume. Without them the IBM DOS kernel builds a default
    // layout from the partition size instead, which breaks the filesystem
    // as soon as SYS trusts the on-disk BPB.
    boot[36] = drive_number;
    boot[38] = EXTENDED_BOOT_SIGNATURE;
    boot[39..43].copy_from_slice(&VOLUME_SERIAL_NUMBER.to_le_bytes());
    boot[43..54].copy_from_slice(b"NO NAME    ");
    boot[54..62].copy_from_slice(if bpb.is_fat16 {
        b"FAT16   "
    } else {
        b"FAT12   "
    });

    // The EB 3C jump at offset 0 lands here. A freshly formatted volume has
    // no operating system, so park instead of running into zero bytes.
    boot[62] = 0xEB;
    boot[63] = 0xFE;

    let signature_offset = boot.len() - 2;
    boot[signature_offset] = 0x55;
    boot[signature_offset + 1] = 0xAA;
}

/// The `JMP SHORT 3Eh` and `NOP` at the start of a DOS boot sector.
pub(crate) const DOS_BOOT_JUMP: [u8; 3] = [0xEB, 0x3C, 0x90];

/// Builds the hard-disk FAT boot sector (BPB) for a partition at
/// `partition_offset`.
pub fn hdd_boot_sector(
    bpb: &BpbParams,
    sector_size: u16,
    partition_sectors: u32,
    sectors_per_track: u8,
    heads: u8,
    partition_offset: u32,
) -> Vec<u8> {
    let mut boot = vec![0u8; sector_size as usize];
    write_bpb_fields(
        &mut boot,
        DOS_BOOT_JUMP,
        bpb,
        partition_sectors,
        sectors_per_track,
        heads,
        partition_offset,
    );
    write_extended_bpb_and_signature(&mut boot, bpb, HDD_PHYSICAL_DRIVE_NUMBER);
    boot
}

/// Builds one FAT copy (`sectors_per_fat` sectors) with its reserved cluster
/// entries initialized and every other entry free.
pub fn fat_region(bpb: &BpbParams, sector_size: u16) -> Vec<u8> {
    let fat_size = bpb.sectors_per_fat as usize * sector_size as usize;
    let mut fat_data = vec![0u8; fat_size];
    if bpb.is_fat16 {
        fat_data[0] = bpb.media_descriptor;
        fat_data[1] = 0xFF;
        fat_data[2] = 0xFF;
        fat_data[3] = 0xFF;
    } else {
        fat_data[0] = bpb.media_descriptor;
        fat_data[1] = 0xFF;
        fat_data[2] = 0xFF;
    }
    fat_data
}

/// Returns the geometry of the NEC layout a PC-98 partition table takes on
/// an image: the PC-98 IDE BIOS geometry on a PC-98 IDE image, the image
/// geometry on a SASI image. Returns `None` for any other image.
fn pc98_nec_layout_geometry(image: &HddImage) -> Option<HddGeometry> {
    if pc98_ide::is_ide_image_format(image.format)
        && pc98_ide::is_ide_sector_size(image.geometry.sector_size)
    {
        Some(pc98_ide::bios_geometry(image.geometry))
    } else if pc98_sasi::is_sasi_sector_size(image.geometry.sector_size) {
        Some(image.geometry)
    } else {
        None
    }
}

/// Writes `data` to the physical sectors of `image` from `lba` on.
fn write_sectors(image: &mut HddImage, lba: u32, data: &[u8]) -> Result<(), FormatError> {
    let sector_size = usize::from(image.geometry.sector_size);
    for (offset, sector) in data.chunks_exact(sector_size).enumerate() {
        if !image.write_sector(lba + offset as u32, sector) {
            return Err(FormatError::SectorOutOfRange);
        }
    }
    Ok(())
}

/// Formats an in-memory hard-disk image: writes the partition table, boot
/// sector, both FAT copies, and an empty root directory. The data area is left
/// as-is (a freshly built blank image is already zero-filled). A PC-98
/// partition table on a PC-98 IDE image or a SASI image lays the disk out in
/// the layout of the NEC MS-DOS 6.20 FORMAT (`pc98_nec_layout`). The IDE
/// image takes the PC-98 IDE BIOS geometry, the SASI image its own geometry.
pub fn format_hdd_image(
    image: &mut HddImage,
    table: PartitionTableType,
) -> Result<(), FormatError> {
    match table {
        PartitionTableType::Pc98 => match pc98_nec_layout_geometry(image) {
            Some(geometry) => format_pc98_nec_layout(image, &pc98_nec_layout(geometry)?),
            None => format_whole_disk_layout(image, table),
        },
        PartitionTableType::At => format_whole_disk_layout(image, table),
    }
}

/// Writes the IPL sector, the partition table, the reserved area, both FAT
/// copies and the empty root directory of `layout` to `image`.
fn format_pc98_nec_layout(image: &mut HddImage, layout: &Pc98NecLayout) -> Result<(), FormatError> {
    let ipl = vec![0u8; usize::from(layout.physical_sector_size)];
    write_sectors(image, 0, &ipl)?;
    write_sectors(image, 1, &layout.partition_table_sector())?;
    write_sectors(image, layout.partition_start(), &layout.reserved_area())?;
    let fat = layout.fat();
    for copy in 0..layout.bpb.num_fats {
        write_sectors(image, layout.fat_lba(copy), &fat)?;
    }
    write_sectors(image, layout.root_directory_lba(), &layout.root_directory())
}

/// Formats `image` with one partition from its second track to its end in the
/// image geometry and a volume of the physical sector size.
fn format_whole_disk_layout(
    image: &mut HddImage,
    table: PartitionTableType,
) -> Result<(), FormatError> {
    let geometry = image.geometry;
    let sector_size = geometry.sector_size;
    let total_sectors = geometry.total_sectors();
    let sectors_per_track = geometry.sectors_per_track;
    let partition_offset = u32::from(sectors_per_track);

    if total_sectors <= partition_offset {
        return Err(FormatError::ImageTooSmall);
    }
    let partition_sectors = total_sectors - partition_offset;
    let bpb = hdd_bpb_params(sector_size, partition_sectors);

    match table {
        PartitionTableType::Pc98 => {
            let ipl = vec![0u8; sector_size as usize];
            write_sectors(image, 0, &ipl)?;
            let part = pc98_partition_table_sector(
                sector_size,
                total_sectors,
                geometry.heads,
                sectors_per_track,
            );
            write_sectors(image, 1, &part)?;
        }
        PartitionTableType::At => {
            let mbr = at_master_boot_record(
                sector_size,
                partition_offset,
                partition_sectors,
                geometry.heads,
                sectors_per_track,
            );
            write_sectors(image, 0, &mbr)?;
        }
    }

    let boot = hdd_boot_sector(
        &bpb,
        sector_size,
        partition_sectors,
        sectors_per_track,
        geometry.heads,
        partition_offset,
    );
    write_sectors(image, partition_offset, &boot)?;

    let fat = fat_region(&bpb, sector_size);
    let fat_base = partition_offset + bpb.reserved_sectors as u32;
    for fat_index in 0..bpb.num_fats as u32 {
        write_sectors(
            image,
            fat_base + fat_index * bpb.sectors_per_fat as u32,
            &fat,
        )?;
    }

    let root_base = fat_base + bpb.num_fats as u32 * bpb.sectors_per_fat as u32;
    let root_dir_sectors = (bpb.root_entry_count as u32 * 32).div_ceil(sector_size as u32);
    let root = vec![0u8; root_dir_sectors as usize * sector_size as usize];
    write_sectors(image, root_base, &root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk::{HddFormat, HddSizeType, blank_hdd_image};

    /// The layout the NEC MS-DOS 6.20 FORMAT wrote on a disk type at the
    /// largest size it offers. Sectors are physical, except the BPB fields.
    struct MeasuredLayout {
        size: HddSizeType,
        end_cylinder: u16,
        partition_sectors: u32,
        system_id: u8,
        bytes_per_sector: u16,
        sectors_per_cluster: u8,
        reserved_sectors: u16,
        root_entry_count: u16,
        total_sectors_16: u16,
        total_sectors_32: u32,
        sectors_per_fat: u16,
        sectors_per_track: u16,
        heads: u16,
        hidden_sectors: u32,
        is_fat16: bool,
        first_fat_lba: u32,
        root_directory_lba: u32,
        data_area_lba: u32,
    }

    /// Builds one row of `MEASURED_LAYOUTS`.
    #[allow(clippy::too_many_arguments)]
    const fn measured(
        size: HddSizeType,
        end_cylinder: u16,
        partition_sectors: u32,
        system_id: u8,
        bpb: (u16, u8, u16, u16, u16, u32, u16),
        sectors_per_track: u16,
        heads: u16,
        hidden_sectors: u32,
        is_fat16: bool,
        positions: (u32, u32, u32),
    ) -> MeasuredLayout {
        MeasuredLayout {
            size,
            end_cylinder,
            partition_sectors,
            system_id,
            bytes_per_sector: bpb.0,
            sectors_per_cluster: bpb.1,
            reserved_sectors: bpb.2,
            root_entry_count: bpb.3,
            total_sectors_16: bpb.4,
            total_sectors_32: bpb.5,
            sectors_per_fat: bpb.6,
            sectors_per_track,
            heads,
            hidden_sectors,
            is_fat16,
            first_fat_lba: positions.0,
            root_directory_lba: positions.1,
            data_area_lba: positions.2,
        }
    }

    /// The layouts the NEC MS-DOS 6.20 FORMAT wrote on every PC-98 disk type.
    /// The BPB tuple is bytes per sector, sectors per cluster, reserved
    /// sectors, root entries, 16-bit total, 32-bit total and sectors per FAT.
    /// The positions are the first FAT, the root directory and the data area.
    #[rustfmt::skip]
    const MEASURED_LAYOUTS: [MeasuredLayout; 11] = [
        measured(HddSizeType::Mb5, 150, 19800, 0x81, (1024, 2, 1, 512, 4950, 0, 4), 33, 4, 132, false, (136, 168, 232)),
        measured(HddSizeType::Mb10, 300, 39600, 0x81, (1024, 4, 1, 768, 9900, 0, 4), 33, 4, 132, false, (136, 168, 264)),
        measured(HddSizeType::Mb15, 300, 59400, 0x81, (1024, 4, 1, 1024, 14850, 0, 6), 33, 6, 198, false, (202, 250, 378)),
        measured(HddSizeType::Mb20, 300, 79200, 0x81, (1024, 8, 1, 1280, 19800, 0, 4), 33, 8, 264, false, (268, 300, 460)),
        measured(HddSizeType::Mb30, 600, 118800, 0x81, (1024, 8, 1, 1792, 29700, 0, 6), 33, 6, 198, false, (202, 250, 474)),
        measured(HddSizeType::Mb40, 600, 158400, 0x81, (1024, 16, 1, 3072, 39600, 0, 4), 33, 8, 264, false, (268, 300, 684)),
        measured(HddSizeType::IdeMb40, 600, 81600, 0x81, (1024, 16, 1, 3072, 39600, 0, 4), 17, 8, 136, false, (138, 154, 346)),
        measured(HddSizeType::IdeMb80, 1215, 165240, 0x91, (2048, 2, 1, 3072, 40095, 0, 20), 17, 8, 136, true, (140, 300, 492)),
        measured(HddSizeType::IdeMb120, 1815, 246840, 0x91, (2048, 2, 1, 3072, 59895, 0, 30), 17, 8, 136, true, (140, 380, 572)),
        measured(HddSizeType::IdeMb200, 3012, 409632, 0xA1, (512, 8, 2, 3072, 0, 409600, 200), 17, 8, 136, true, (138, 538, 730)),
        measured(HddSizeType::IdeMb500, 7515, 1022040, 0xA1, (512, 16, 2, 3072, 0, 1021952, 250), 17, 8, 136, true, (138, 638, 830)),
    ];

    /// Reads a little-endian word.
    fn word(bytes: &[u8], offset: usize) -> u16 {
        u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
    }

    /// Reads a little-endian double word.
    fn double_word(bytes: &[u8], offset: usize) -> u32 {
        u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
    }

    /// Returns the geometry the BIOS addresses a disk type in.
    fn bios_geometry(size: HddSizeType) -> HddGeometry {
        pc98_ide::bios_geometry(size.geometry().0)
    }

    #[test]
    fn pc98_format_writes_the_measured_nec_layout_of_every_disk_type() {
        for expected in &MEASURED_LAYOUTS {
            let tag = expected.size.as_token();
            let mut image = blank_hdd_image(expected.size);
            format_hdd_image(&mut image, PartitionTableType::Pc98).unwrap();
            let sector_size = u32::from(image.geometry.sector_size);
            let sectors_per_cylinder =
                u32::from(expected.heads) * u32::from(expected.sectors_per_track);

            let table = image.read_sector(1).unwrap();
            assert_eq!(table[0], PC98_PARTITION_MID, "{tag}: MID");
            assert_eq!(table[1], expected.system_id, "{tag}: SID");
            assert_eq!(&table[2..4], &[0, 0], "{tag}: entry bytes 2 and 3");
            assert_eq!(&table[4..8], &[0, 0, 1, 0], "{tag}: IPL on cylinder 1");
            assert_eq!(
                &table[8..12],
                &[0, 0, 1, 0],
                "{tag}: data start on cylinder 1"
            );
            let [end_low, end_high] = expected.end_cylinder.to_le_bytes();
            assert_eq!(
                &table[12..16],
                &[0, 0, end_low, end_high],
                "{tag}: data end"
            );
            assert_eq!(&table[16..32], PC98_PARTITION_NAME, "{tag}: name");
            assert!(
                table[32..].iter().all(|&byte| byte == 0),
                "{tag}: one entry"
            );
            assert_eq!(
                u32::from(expected.end_cylinder) * sectors_per_cylinder,
                expected.partition_sectors,
                "{tag}: partition sectors"
            );

            let boot = image.read_sector(expected.hidden_sectors).unwrap().to_vec();
            assert_eq!(&boot[0..3], &DOS_BOOT_JUMP, "{tag}: jump");
            assert_eq!(&boot[3..11], b"NEETAN  ", "{tag}: OEM name");
            assert_eq!(
                word(&boot, 11),
                expected.bytes_per_sector,
                "{tag}: bytes per sector"
            );
            assert_eq!(
                boot[13], expected.sectors_per_cluster,
                "{tag}: sectors per cluster"
            );
            assert_eq!(
                word(&boot, 14),
                expected.reserved_sectors,
                "{tag}: reserved sectors"
            );
            assert_eq!(boot[16], 2, "{tag}: FAT count");
            assert_eq!(
                word(&boot, 17),
                expected.root_entry_count,
                "{tag}: root entries"
            );
            assert_eq!(
                word(&boot, 19),
                expected.total_sectors_16,
                "{tag}: 16-bit total"
            );
            assert_eq!(boot[21], 0xF8, "{tag}: media");
            assert_eq!(
                word(&boot, 22),
                expected.sectors_per_fat,
                "{tag}: sectors per FAT"
            );
            assert_eq!(
                word(&boot, 24),
                expected.sectors_per_track,
                "{tag}: sectors per track"
            );
            assert_eq!(word(&boot, 26), expected.heads, "{tag}: heads");
            assert_eq!(
                double_word(&boot, 28),
                expected.hidden_sectors,
                "{tag}: hidden sectors"
            );
            assert_eq!(
                double_word(&boot, 32),
                expected.total_sectors_32,
                "{tag}: 32-bit total"
            );
            assert_eq!(
                &boot[38..39],
                &[EXTENDED_BOOT_SIGNATURE],
                "{tag}: extended BPB"
            );
            let fat_type: &[u8; 8] = if expected.is_fat16 {
                b"FAT16   "
            } else {
                b"FAT12   "
            };
            assert_eq!(&boot[54..62], fat_type, "{tag}: FAT type");

            let ratio = u32::from(expected.bytes_per_sector) / sector_size;
            let fat_physical_sectors = u32::from(expected.sectors_per_fat) * ratio;
            let first_fat = expected.hidden_sectors + u32::from(expected.reserved_sectors) * ratio;
            let root_directory = first_fat + 2 * fat_physical_sectors;
            let root_physical_sectors =
                (u32::from(expected.root_entry_count) * 32).div_ceil(sector_size);
            assert_eq!(
                first_fat, expected.first_fat_lba,
                "{tag}: FAT 1 from the BPB"
            );
            assert_eq!(
                root_directory, expected.root_directory_lba,
                "{tag}: root from the BPB"
            );
            assert_eq!(
                root_directory + root_physical_sectors,
                expected.data_area_lba,
                "{tag}: data area from the BPB"
            );

            let fat_start: &[u8] = if expected.is_fat16 {
                &[0xFE, 0xFF, 0xFF, 0xFF]
            } else {
                &[0xFE, 0xFF, 0xFF]
            };
            for copy_start in [first_fat, first_fat + fat_physical_sectors] {
                let fat = image.read_sector(copy_start).unwrap();
                assert_eq!(
                    &fat[..fat_start.len()],
                    fat_start,
                    "{tag}: FAT[0] at {copy_start}"
                );
                assert!(
                    fat[fat_start.len()..].iter().all(|&byte| byte == 0),
                    "{tag}: free FAT"
                );
            }
            for lba in root_directory..expected.data_area_lba {
                assert!(
                    image
                        .read_sector(lba)
                        .unwrap()
                        .iter()
                        .all(|&byte| byte == 0),
                    "{tag}: empty root directory sector {lba}"
                );
            }

            let layout = pc98_nec_layout(bios_geometry(expected.size)).unwrap();
            assert_eq!(layout.partition_start(), expected.hidden_sectors, "{tag}");
            assert_eq!(
                layout.partition_sectors(),
                expected.partition_sectors,
                "{tag}"
            );
            assert_eq!(layout.fat_lba(0), expected.first_fat_lba, "{tag}");
            assert_eq!(
                layout.root_directory_lba(),
                expected.root_directory_lba,
                "{tag}"
            );
            assert_eq!(layout.data_area_lba(), expected.data_area_lba, "{tag}");
        }
    }

    #[test]
    fn nec_layout_offers_the_measured_size_of_custom_ide_disks() {
        let custom_disks: [(u16, u64, bool); 17] = [
            (610, 40, false),
            (615, 40, false),
            (616, 40, true),
            (617, 41, true),
            (963, 64, true),
            (1000, 66, true),
            (1900, 126, true),
            (1927, 128, true),
            (1929, 128, true),
            (2000, 133, true),
            (2200, 146, true),
            (2259, 150, true),
            (2266, 150, true),
            (2267, 150, true),
            (2280, 151, true),
            (2300, 152, true),
            (2400, 159, true),
        ];
        for (cylinders, offered_units, is_fat16) in custom_disks {
            let geometry = HddGeometry {
                cylinders,
                heads: 8,
                sectors_per_track: 17,
                sector_size: 512,
            };
            let layout = pc98_nec_layout(geometry).unwrap();
            let unit_bytes = if cylinders >= 2267 {
                NEC_LARGE_UNIT_BYTES
            } else {
                NEC_SMALL_UNIT_BYTES
            };
            let volume_bytes =
                u64::from(layout.volume_sectors) * u64::from(layout.bpb.bytes_per_sector);
            assert_eq!(volume_bytes, offered_units * unit_bytes, "c{cylinders}");
            assert_eq!(layout.bpb.is_fat16, is_fat16, "c{cylinders}");
            assert!(layout.end_cylinder <= cylinders - 2, "c{cylinders}");
        }
    }

    #[test]
    fn nec_layout_of_a_non_standard_sasi_geometry() {
        let geometry = HddGeometry {
            cylinders: 320,
            heads: 4,
            sectors_per_track: 33,
            sector_size: 256,
        };
        let mut image = HddImage::from_raw(
            geometry,
            HddFormat::Hdi,
            vec![0u8; geometry.total_bytes() as usize],
        );
        format_hdd_image(&mut image, PartitionTableType::Pc98).unwrap();

        let table = image.read_sector(1).unwrap();
        assert_eq!(&table[8..12], &[0, 0, 1, 0]);
        let [end_low, end_high] = 300u16.to_le_bytes();
        assert_eq!(&table[12..16], &[0, 0, end_low, end_high]);
        let boot = image.read_sector(132).unwrap();
        assert_eq!(double_word(boot, 28), 132);
        assert_eq!(word(boot, 19), 9900);
    }

    #[test]
    fn nec_layout_keeps_a_fat12_volume_of_many_heads_below_the_cluster_limit() {
        let geometry = HddGeometry {
            cylinders: 615,
            heads: 16,
            sectors_per_track: 33,
            sector_size: 256,
        };
        let layout = pc98_nec_layout(geometry).unwrap();
        assert!(!layout.bpb.is_fat16);
        assert_eq!(layout.bpb.bytes_per_sector, 1024);
        let clusters = layout.data_sectors() / u32::from(layout.bpb.sectors_per_cluster);
        assert!(clusters <= FAT12_MAXIMUM_CLUSTERS);
    }

    #[test]
    fn nec_layout_needs_a_usable_cylinder() {
        let geometry = HddGeometry {
            cylinders: 2,
            heads: 8,
            sectors_per_track: 33,
            sector_size: 256,
        };
        assert_eq!(pc98_nec_layout(geometry), Err(FormatError::ImageTooSmall));
    }

    #[test]
    fn nec_layout_places_the_boot_signature_at_offset_510() {
        let layout = pc98_nec_layout(bios_geometry(HddSizeType::IdeMb120)).unwrap();
        let reserved_area = layout.reserved_area();
        assert_eq!(reserved_area.len(), 2048);
        assert_eq!(&reserved_area[510..512], &[0x55, 0xAA]);
        assert!(reserved_area[512..].iter().all(|&byte| byte == 0));
    }

    #[test]
    fn pc98_format_of_a_disk_outside_the_nec_layout_spans_the_disk() {
        let mut image = blank_hdd_image(HddSizeType::ScsiMb20);
        format_hdd_image(&mut image, PartitionTableType::Pc98).unwrap();
        let geometry = image.geometry;
        let start = u32::from(geometry.sectors_per_track);
        let partition_sectors = geometry.total_sectors() - start;
        assert_eq!(
            image.read_sector(1).unwrap(),
            pc98_partition_table_sector(
                512,
                geometry.total_sectors(),
                geometry.heads,
                geometry.sectors_per_track
            )
        );
        assert_eq!(
            image.read_sector(start).unwrap(),
            hdd_boot_sector(
                &hdd_bpb_params(512, partition_sectors),
                512,
                partition_sectors,
                geometry.sectors_per_track,
                geometry.heads,
                start
            )
        );
    }

    #[test]
    fn at_format_writes_valid_mbr() {
        let mut image = blank_hdd_image(HddSizeType::AtMb40);
        assert_eq!(image.format, HddFormat::AtFlat);
        format_hdd_image(&mut image, PartitionTableType::At).unwrap();

        let mbr = image.read_sector(0).unwrap();
        assert_eq!(mbr[510], 0x55);
        assert_eq!(mbr[511], 0xAA);
        // The chain-loading bootstrap sits at the start of the sector.
        assert_eq!(&mbr[..AT_MBR_BOOTSTRAP.len()], &AT_MBR_BOOTSTRAP);
        // Active FAT16B partition entry at 0x1BE.
        assert_eq!(mbr[0x1BE], 0x80);
        assert_eq!(mbr[0x1BE + 4], 0x06);
        // The partition starts at CHS 0/1/1, the second track.
        assert_eq!(&mbr[0x1BE + 1..0x1BE + 4], &[0x01, 0x01, 0x00]);
        let start_lba = u32::from_le_bytes([
            mbr[0x1BE + 8],
            mbr[0x1BE + 9],
            mbr[0x1BE + 10],
            mbr[0x1BE + 11],
        ]);
        assert_eq!(start_lba, image.geometry.sectors_per_track as u32);
    }

    #[test]
    fn hdd_boot_sector_passes_dos_validation() {
        let mut image = blank_hdd_image(HddSizeType::AtMb100);
        format_hdd_image(&mut image, PartitionTableType::At).unwrap();

        let partition_offset = image.geometry.sectors_per_track as u32;
        let boot = image.read_sector(partition_offset).unwrap();

        assert_eq!(&boot[0..3], &[0xEB, 0x3C, 0x90]);
        assert_eq!(boot[36], HDD_PHYSICAL_DRIVE_NUMBER);
        assert_eq!(boot[38], EXTENDED_BOOT_SIGNATURE);
        assert_eq!(&boot[43..54], b"NO NAME    ");
        assert_eq!(&boot[54..62], b"FAT16   ");
        assert_eq!(boot[510], 0x55);
        assert_eq!(boot[511], 0xAA);

        // The MS-DOS FORMAT layout for a 100 MB FAT16 partition: 2 KB
        // clusters and 200 FAT sectors. DOS SETUP computes the same values
        // from the partition size, so any other layout corrupts the install.
        assert_eq!(boot[13], 4);
        assert_eq!(u16::from_le_bytes([boot[22], boot[23]]), 200);
    }
}
