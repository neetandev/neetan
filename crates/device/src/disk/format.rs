//! Hard-disk formatting: partition table, BPB/boot sector, FAT, and root
//! directory layout for FAT12/FAT16 volumes.

use super::{HddGeometry, HddImage};

/// The partition-table style written to the first sectors of a hard disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartitionTableType {
    /// PC-98 partition table (MID/SID entry at sector 1, IPL at sector 0).
    Pc98,
    /// PC/AT master boot record (partition entry plus 0x55AA at sector 0).
    At,
}

/// FAT BPB fields resolved for a given volume geometry.
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
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FormatError::ImageTooSmall => f.write_str("hard disk image is too small to format"),
            FormatError::SectorOutOfRange => f.write_str("format wrote a sector out of range"),
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

/// Resolves the BPB for a hard-disk partition of `partition_sectors` sectors.
pub fn hdd_bpb_params(sector_size: u16, partition_sectors: u32) -> BpbParams {
    let bytes_per_sector = sector_size;
    let reserved_sectors: u16 = 1;
    let num_fats: u8 = 2;
    let root_entry_count: u16 = 512;

    let volume_bytes = partition_sectors as u64 * sector_size as u64;
    let sectors_per_cluster: u8 = if sector_size == 256 {
        // SASI 256-byte sectors: target 2 KB clusters
        if volume_bytes <= (64 << 20) {
            8
        } else if volume_bytes <= (128 << 20) {
            16
        } else if volume_bytes <= (256 << 20) {
            32
        } else {
            64
        }
    } else {
        // IDE 512-byte sectors: the MS-DOS FORMAT FAT16 cluster size
        // steps. DOS SETUP and FORMAT compute the same layout from the
        // partition size, so matching them keeps every component of the
        // guest OS on one filesystem geometry.
        if volume_bytes <= (128 << 20) {
            4
        } else if volume_bytes <= (256 << 20) {
            8
        } else if volume_bytes <= (512 << 20) {
            16
        } else {
            32
        }
    };

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

/// Builds the PC-98 partition-table sector (disk sector 1). One FAT16 partition
/// spanning the disk, active, data starting on the second track.
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
    sector[16..32].copy_from_slice(b"NEETAN          ");

    sector
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
    boot[0] = 0xEB;
    boot[1] = 0x3C;
    boot[2] = 0x90;
    boot[3..11].copy_from_slice(b"NEETAN  ");
    boot[11..13].copy_from_slice(&bpb.bytes_per_sector.to_le_bytes());
    boot[13] = bpb.sectors_per_cluster;
    boot[14..16].copy_from_slice(&bpb.reserved_sectors.to_le_bytes());
    boot[16] = bpb.num_fats;
    boot[17..19].copy_from_slice(&bpb.root_entry_count.to_le_bytes());
    if partition_sectors <= 0xFFFF {
        boot[19..21].copy_from_slice(&(partition_sectors as u16).to_le_bytes());
    } else {
        boot[19..21].copy_from_slice(&0u16.to_le_bytes());
        boot[32..36].copy_from_slice(&partition_sectors.to_le_bytes());
    }
    boot[21] = bpb.media_descriptor;
    boot[22..24].copy_from_slice(&bpb.sectors_per_fat.to_le_bytes());
    boot[24..26].copy_from_slice(&(sectors_per_track as u16).to_le_bytes());
    boot[26..28].copy_from_slice(&(heads as u16).to_le_bytes());
    boot[28..32].copy_from_slice(&partition_offset.to_le_bytes());

    // The extended BPB and the boot signature make DOS accept the BPB when
    // it mounts the volume. Without them the IBM DOS kernel builds a default
    // layout from the partition size instead, which breaks the filesystem
    // as soon as SYS trusts the on-disk BPB.
    boot[36] = HDD_PHYSICAL_DRIVE_NUMBER;
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

    let signature_offset = sector_size as usize - 2;
    boot[signature_offset] = 0x55;
    boot[signature_offset + 1] = 0xAA;
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

/// Formats an in-memory hard-disk image: writes the partition table, boot
/// sector, both FAT copies, and an empty root directory. The data area is left
/// as-is (a freshly built blank image is already zero-filled).
pub fn format_hdd_image(
    image: &mut HddImage,
    table: PartitionTableType,
) -> Result<(), FormatError> {
    let geometry: HddGeometry = image.geometry;
    let sector_size = geometry.sector_size;
    let total_sectors = geometry.total_sectors();
    let sectors_per_track = geometry.sectors_per_track;
    let partition_offset = sectors_per_track as u32;

    if total_sectors <= partition_offset {
        return Err(FormatError::ImageTooSmall);
    }
    let partition_sectors = total_sectors - partition_offset;
    let bpb = hdd_bpb_params(sector_size, partition_sectors);

    let write = |image: &mut HddImage, lba: u32, data: &[u8]| -> Result<(), FormatError> {
        if image.write_sector(lba, data) {
            Ok(())
        } else {
            Err(FormatError::SectorOutOfRange)
        }
    };

    match table {
        PartitionTableType::Pc98 => {
            let ipl = vec![0u8; sector_size as usize];
            write(image, 0, &ipl)?;
            let part = pc98_partition_table_sector(
                sector_size,
                total_sectors,
                geometry.heads,
                sectors_per_track,
            );
            write(image, 1, &part)?;
        }
        PartitionTableType::At => {
            let mbr = at_master_boot_record(
                sector_size,
                partition_offset,
                partition_sectors,
                geometry.heads,
                sectors_per_track,
            );
            write(image, 0, &mbr)?;
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
    write(image, partition_offset, &boot)?;

    let fat = fat_region(&bpb, sector_size);
    let fat_base = partition_offset + bpb.reserved_sectors as u32;
    for fat_index in 0..bpb.num_fats as u32 {
        let base = fat_base + fat_index * bpb.sectors_per_fat as u32;
        for (offset, chunk) in fat.chunks_exact(sector_size as usize).enumerate() {
            write(image, base + offset as u32, chunk)?;
        }
    }

    let root_base = fat_base + bpb.num_fats as u32 * bpb.sectors_per_fat as u32;
    let root_dir_sectors = (bpb.root_entry_count as u32 * 32).div_ceil(sector_size as u32);
    let empty = vec![0u8; sector_size as usize];
    for offset in 0..root_dir_sectors {
        write(image, root_base + offset, &empty)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk::{HddFormat, HddSizeType, blank_hdd_image};

    #[test]
    fn pc98_format_is_mountable_fat() {
        let mut image = blank_hdd_image(HddSizeType::Mb40);
        format_hdd_image(&mut image, PartitionTableType::Pc98).unwrap();

        // Sector 1 carries the PC-98 partition table.
        let part = image.read_sector(1).unwrap();
        assert_eq!(part[0], 0x21);
        assert_eq!(part[1], 0x81);
        assert_eq!(&part[16..22], b"NEETAN");

        // The boot sector sits at the start of the second track.
        let partition_offset = image.geometry.sectors_per_track as u32;
        let boot = image.read_sector(partition_offset).unwrap();
        assert_eq!(&boot[0..3], &[0xEB, 0x3C, 0x90]);
        assert_eq!(&boot[3..11], b"NEETAN  ");
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
