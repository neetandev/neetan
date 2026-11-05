//! PC-98 IDE BIOS geometry and the migration of images to it.
//!
//! The PC-98 IDE BIOS addresses a disk with 512-byte sectors as 8 heads x 17
//! sectors per track. The cylinder count is the disk capacity divided by
//! 8 x 17, capped at 65535. A disk with 256-byte sectors keeps the geometry
//! of its image.
//!
//! The migration relocates the FAT partitions of an image laid out in its
//! image geometry into the NEC layout of the BIOS geometry: the first
//! partition starts on cylinder 1, every partition starts on a cylinder
//! boundary and ends on a whole cylinder. The volumes keep their size. When
//! the relocated partitions need more room, the migration appends cylinders
//! to the image and updates its header. It rewrites the partition table and
//! the geometry and partition start of each boot record. An image with a
//! partition it cannot relocate safely stays unchanged.

use super::{HddFormat, HddGeometry, HddImage};

/// Heads of the PC-98 IDE BIOS geometry.
pub const BIOS_HEADS: u8 = 8;

/// Sectors per track of the PC-98 IDE BIOS geometry.
pub const BIOS_SECTORS_PER_TRACK: u8 = 17;

/// Sectors of one cylinder of the PC-98 IDE BIOS geometry.
const BIOS_SECTORS_PER_CYLINDER: u32 = BIOS_HEADS as u32 * BIOS_SECTORS_PER_TRACK as u32;

/// Highest cylinder count of the PC-98 IDE BIOS geometry.
const MAXIMUM_CYLINDERS: u32 = 65535;

/// Sector size the PC-98 IDE BIOS geometry applies to.
const BIOS_SECTOR_SIZE: u16 = 512;

/// LBA of the PC-98 partition table.
const PARTITION_TABLE_LBA: u32 = 1;

/// Size of one PC-98 partition table entry.
const PARTITION_ENTRY_SIZE: usize = 32;

/// Entry offset of the IPL address.
const ENTRY_IPL_OFFSET: usize = 4;

/// Entry offset of the data start address.
const ENTRY_START_OFFSET: usize = 8;

/// Entry offset of the data end address.
const ENTRY_END_OFFSET: usize = 12;

/// Boot record offset of the bytes per sector word.
const BOOT_RECORD_BYTES_PER_SECTOR_OFFSET: usize = 11;

/// Boot record offset of the 16-bit total sectors word.
const BOOT_RECORD_TOTAL_SECTORS_16_OFFSET: usize = 19;

/// Boot record offset of the 16-bit sectors per FAT word.
const BOOT_RECORD_SECTORS_PER_FAT_16_OFFSET: usize = 22;

/// Boot record offset of the sectors per track word.
const BOOT_RECORD_SECTORS_PER_TRACK_OFFSET: usize = 24;

/// Boot record offset of the heads word.
const BOOT_RECORD_HEADS_OFFSET: usize = 26;

/// Boot record offset of the hidden sectors double word.
const BOOT_RECORD_HIDDEN_SECTORS_OFFSET: usize = 28;

/// Boot record offset of the 32-bit total sectors double word.
const BOOT_RECORD_TOTAL_SECTORS_32_OFFSET: usize = 32;

/// FAT32 boot record offset of the backup boot sector word.
const FAT32_BACKUP_BOOT_SECTOR_OFFSET: usize = 50;

/// NEC boot record offset of the double word that holds the LBA of the
/// partition start. The NEC boot code reads the volume through it.
const NEC_BOOT_RECORD_PARTITION_START_OFFSET: usize = 0x3E;

/// First offset of the boot code of a NEC boot record.
const NEC_BOOT_RECORD_CODE_OFFSET: usize = 0x46;

/// HDI header offset of the data size, in sectors or in bytes.
const HDI_DATA_SIZE_OFFSET: usize = 12;

/// HDI header offset of the cylinders double word.
const HDI_CYLINDERS_OFFSET: usize = 28;

/// NHD header offset of the cylinders double word.
const NHD_CYLINDERS_OFFSET: usize = 0x114;

/// Returns the geometry the PC-98 IDE BIOS uses for a disk with the given
/// image geometry.
pub fn bios_geometry(image_geometry: HddGeometry) -> HddGeometry {
    if !is_ide_sector_size(image_geometry.sector_size) {
        return image_geometry;
    }
    let cylinders =
        (image_geometry.total_sectors() / BIOS_SECTORS_PER_CYLINDER).min(MAXIMUM_CYLINDERS);
    HddGeometry {
        cylinders: cylinders as u16,
        heads: BIOS_HEADS,
        sectors_per_track: BIOS_SECTORS_PER_TRACK,
        sector_size: BIOS_SECTOR_SIZE,
    }
}

/// Returns whether a disk with this sector size is a PC-98 IDE disk, which
/// the PC-98 IDE BIOS addresses in the BIOS geometry.
pub fn is_ide_sector_size(sector_size: u16) -> bool {
    sector_size == BIOS_SECTOR_SIZE
}

/// Returns whether an image format holds a PC-98 disk that the PC-98 IDE
/// BIOS addresses in the BIOS geometry.
pub fn is_ide_image_format(format: HddFormat) -> bool {
    match format {
        HddFormat::Hdi | HddFormat::Nhd => true,
        HddFormat::Thd | HddFormat::Raw | HddFormat::AtFlat => false,
    }
}

/// The FAT volume of a partition.
struct FatVolume {
    volume_sectors: u32,
    backup_boot_sector: Option<u32>,
    has_nec_partition_start: bool,
}

/// A partition of the image geometry.
struct Partition {
    entry_index: usize,
    ipl: u32,
    start: u32,
    end: u32,
    volume: FatVolume,
}

/// The cylinders appended to an image and the new HDI data size.
struct Enlargement {
    cylinders: u16,
    hdi_data_size: Option<u32>,
}

/// A partition and its place in the BIOS geometry.
struct Placement {
    partition: Partition,
    new_start: u32,
    copy_length: u32,
    end_cylinder: u16,
}

/// Relocates the partitions of an image laid out in its image geometry into
/// the PC-98 IDE BIOS geometry. Appends cylinders to the image when the
/// relocated partitions need them. Returns whether the image changed. An
/// image that already uses the BIOS geometry, holds no partition, or has a
/// partition without a readable FAT volume or that cannot be placed stays
/// unchanged.
pub fn migrate_to_bios_geometry(image: &mut HddImage) -> bool {
    let Some((placements, enlargement)) = plan_relocation(image) else {
        return false;
    };
    let Some(table) = image.read_sector(PARTITION_TABLE_LBA) else {
        return false;
    };
    let mut table = table.to_vec();
    if let Some(enlargement) = enlargement {
        enlarge(image, &enlargement);
    }

    let sector_size = usize::from(BIOS_SECTOR_SIZE);
    let (backward, forward): (Vec<&Placement>, Vec<&Placement>) = placements
        .iter()
        .partition(|placement| placement.new_start <= placement.partition.start);
    for placement in backward.into_iter().chain(forward.into_iter().rev()) {
        let source = placement.partition.start as usize * sector_size;
        image.data.copy_within(
            source..source + placement.copy_length as usize * sector_size,
            placement.new_start as usize * sector_size,
        );
    }

    for placement in &placements {
        let partition = &placement.partition;
        let volume = &partition.volume;
        let boot_records = std::iter::once((0, volume.has_nec_partition_start))
            .chain(volume.backup_boot_sector.map(|offset| (offset, false)));
        for (offset, has_nec_partition_start) in boot_records {
            let lba = placement.new_start + offset;
            if let Some(boot_record) = image.read_sector(lba) {
                let boot_record = boot_record_in_bios_geometry(
                    boot_record,
                    placement.new_start,
                    has_nec_partition_start,
                );
                image.write_sector(lba, &boot_record);
            }
        }
        let entry =
            &mut table[partition.entry_index * PARTITION_ENTRY_SIZE..][..PARTITION_ENTRY_SIZE];
        let ipl = lba_to_chs(placement.new_start + partition.ipl - partition.start);
        entry[ENTRY_IPL_OFFSET..ENTRY_IPL_OFFSET + 4].copy_from_slice(&ipl);
        let start_cylinder = (placement.new_start / BIOS_SECTORS_PER_CYLINDER) as u16;
        entry[ENTRY_START_OFFSET..ENTRY_START_OFFSET + 4]
            .copy_from_slice(&cylinder_address(start_cylinder));
        entry[ENTRY_END_OFFSET..ENTRY_END_OFFSET + 4]
            .copy_from_slice(&cylinder_address(placement.end_cylinder));
    }
    image.write_sector(PARTITION_TABLE_LBA, &table);
    true
}

/// Plans the relocation of every partition. Returns the placements and the
/// enlargement the relocated layout needs, or `None` when the image needs no
/// relocation or a partition cannot be read or placed.
fn plan_relocation(image: &HddImage) -> Option<(Vec<Placement>, Option<Enlargement>)> {
    let old_geometry = image.geometry;
    let new_geometry = bios_geometry(old_geometry);
    if old_geometry.heads == new_geometry.heads
        && old_geometry.sectors_per_track == new_geometry.sectors_per_track
    {
        return None;
    }
    let table = image.read_sector(PARTITION_TABLE_LBA)?;
    if has_boot_record_in_geometry(image, table, new_geometry) {
        return None;
    }

    let mut partitions = Vec::new();
    for (entry_index, entry) in table
        .as_chunks::<PARTITION_ENTRY_SIZE>()
        .0
        .iter()
        .enumerate()
    {
        if is_empty_entry(entry) {
            continue;
        }
        partitions.push(read_partition(image, entry_index, entry)?);
    }
    if partitions.is_empty() {
        return None;
    }
    partitions.sort_by_key(|partition| partition.start);
    if partitions[0].start <= PARTITION_TABLE_LBA
        || partitions
            .windows(2)
            .any(|pair| pair[1].start < pair[0].end)
    {
        return None;
    }

    let mut new_start = BIOS_SECTORS_PER_CYLINDER;
    let mut placements = Vec::with_capacity(partitions.len());
    for partition in partitions {
        let span = partition.end - partition.start;
        let ipl_offset = partition.ipl - partition.start;
        let volume = &partition.volume;
        let end = ((new_start + span) / BIOS_SECTORS_PER_CYLINDER * BIOS_SECTORS_PER_CYLINDER).max(
            (new_start + volume.volume_sectors).div_ceil(BIOS_SECTORS_PER_CYLINDER)
                * BIOS_SECTORS_PER_CYLINDER,
        );
        let copy_length = if span <= end - new_start {
            span
        } else if ipl_offset < volume.volume_sectors {
            volume.volume_sectors
        } else {
            return None;
        };
        if ipl_offset >= end - new_start {
            return None;
        }
        placements.push(Placement {
            partition,
            new_start,
            copy_length,
            end_cylinder: u16::try_from(end / BIOS_SECTORS_PER_CYLINDER - 1).ok()?,
        });
        new_start = end;
    }

    let enlargement = if new_start <= u32::from(new_geometry.cylinders) * BIOS_SECTORS_PER_CYLINDER
    {
        None
    } else {
        Some(plan_enlargement(image, new_start)?)
    };
    Some((placements, enlargement))
}

/// Plans the cylinders an image needs to hold `sectors` sectors. Returns
/// `None` when the image is no HDI or NHD image, its HDI data size holds
/// neither the sector count nor the byte count, the new data size overflows,
/// or the BIOS geometry would pass its cylinder limit.
fn plan_enlargement(image: &HddImage, sectors: u32) -> Option<Enlargement> {
    let old_geometry = image.geometry;
    let sectors_per_cylinder =
        u32::from(old_geometry.heads) * u32::from(old_geometry.sectors_per_track);
    let cylinders = u16::try_from(sectors.div_ceil(sectors_per_cylinder)).ok()?;
    let enlarged = HddGeometry {
        cylinders,
        ..old_geometry
    };
    if enlarged.total_sectors() / BIOS_SECTORS_PER_CYLINDER > MAXIMUM_CYLINDERS {
        return None;
    }
    let hdi_data_size = match image.format {
        HddFormat::Hdi => {
            let size = read_double_word(&image.header_bytes, HDI_DATA_SIZE_OFFSET);
            if size == old_geometry.total_sectors() {
                Some(enlarged.total_sectors())
            } else if u64::from(size) == old_geometry.total_bytes() {
                Some(u32::try_from(enlarged.total_bytes()).ok()?)
            } else {
                return None;
            }
        }
        HddFormat::Nhd => None,
        HddFormat::Thd | HddFormat::Raw | HddFormat::AtFlat => return None,
    };
    Some(Enlargement {
        cylinders,
        hdi_data_size,
    })
}

/// Appends zeroed cylinders to an HDI or NHD image and writes the new
/// geometry to its header.
fn enlarge(image: &mut HddImage, enlargement: &Enlargement) {
    image.geometry.cylinders = enlargement.cylinders;
    image.data.resize(image.geometry.total_bytes() as usize, 0);
    let cylinders = u32::from(enlargement.cylinders).to_le_bytes();
    let header = &mut image.header_bytes;
    match image.format {
        HddFormat::Hdi => {
            if let Some(size) = enlargement.hdi_data_size {
                header[HDI_DATA_SIZE_OFFSET..HDI_DATA_SIZE_OFFSET + 4]
                    .copy_from_slice(&size.to_le_bytes());
            }
            header[HDI_CYLINDERS_OFFSET..HDI_CYLINDERS_OFFSET + 4].copy_from_slice(&cylinders);
        }
        HddFormat::Nhd => {
            header[NHD_CYLINDERS_OFFSET..NHD_CYLINDERS_OFFSET + 4].copy_from_slice(&cylinders);
        }
        HddFormat::Thd | HddFormat::Raw | HddFormat::AtFlat => {}
    }
}

/// Returns whether a partition table entry is empty: its MID and SID are
/// zero.
fn is_empty_entry(entry: &[u8]) -> bool {
    entry[0] == 0 && entry[1] == 0
}

/// Reads a partition table entry and the boot record of its partition in the
/// image geometry. An empty IPL address means the partition start. Returns
/// `None` when the entry does not describe a partition with a readable FAT
/// volume in the image geometry.
fn read_partition(image: &HddImage, entry_index: usize, entry: &[u8]) -> Option<Partition> {
    let geometry = image.geometry;
    let start = chs_to_lba(&entry[ENTRY_START_OFFSET..ENTRY_START_OFFSET + 4], geometry)?;
    let ipl_address = &entry[ENTRY_IPL_OFFSET..ENTRY_IPL_OFFSET + 4];
    let ipl = if ipl_address.iter().all(|&byte| byte == 0) {
        start
    } else {
        chs_to_lba(ipl_address, geometry)?
    };
    let end_address = &entry[ENTRY_END_OFFSET..ENTRY_END_OFFSET + 4];
    let end = if end_address[0] == 0 && end_address[1] == 0 {
        let cylinder = u32::from(u16::from_le_bytes([end_address[2], end_address[3]]));
        (cylinder + 1) * u32::from(geometry.heads) * u32::from(geometry.sectors_per_track)
    } else {
        chs_to_lba(end_address, geometry)? + 1
    }
    .min(geometry.total_sectors());
    if start >= end || ipl < start || ipl >= end {
        return None;
    }
    let boot_record = image.read_sector(start)?;
    if !bpb_holds_geometry(boot_record, geometry) {
        return None;
    }
    let volume = read_fat_volume(image, boot_record, start, end)?;
    Some(Partition {
        entry_index,
        ipl,
        start,
        end,
        volume,
    })
}

/// Reads the FAT volume of a partition from `start` to `end` whose boot
/// record holds a BPB of the image geometry. Returns `None` when the BPB does
/// not describe a volume inside the partition.
fn read_fat_volume(
    image: &HddImage,
    boot_record: &[u8],
    start: u32,
    end: u32,
) -> Option<FatVolume> {
    if read_double_word(boot_record, BOOT_RECORD_HIDDEN_SECTORS_OFFSET) != start {
        return None;
    }
    let bytes_per_sector = read_word(boot_record, BOOT_RECORD_BYTES_PER_SECTOR_OFFSET);
    if !bytes_per_sector.is_power_of_two() || bytes_per_sector < BIOS_SECTOR_SIZE {
        return None;
    }
    let sectors_per_logical_sector = u32::from(bytes_per_sector / BIOS_SECTOR_SIZE);
    let total_sectors = match read_word(boot_record, BOOT_RECORD_TOTAL_SECTORS_16_OFFSET) {
        0 => read_double_word(boot_record, BOOT_RECORD_TOTAL_SECTORS_32_OFFSET),
        total => u32::from(total),
    };
    let volume_sectors = total_sectors.checked_mul(sectors_per_logical_sector)?;
    if volume_sectors == 0 || volume_sectors > end - start {
        return None;
    }

    let is_fat32 = read_word(boot_record, BOOT_RECORD_SECTORS_PER_FAT_16_OFFSET) == 0;
    let has_nec_partition_start = !is_fat32
        && boot_record[0] == 0xEB
        && usize::from(boot_record[1]) + 2 >= NEC_BOOT_RECORD_CODE_OFFSET
        && read_double_word(boot_record, NEC_BOOT_RECORD_PARTITION_START_OFFSET) == start;
    let backup_boot_sector = if is_fat32 {
        match read_word(boot_record, FAT32_BACKUP_BOOT_SECTOR_OFFSET) {
            0 | 0xFFFF => None,
            backup => {
                let offset = u32::from(backup) * sectors_per_logical_sector;
                let backup_record = image.read_sector(start + offset)?;
                if offset >= volume_sectors || !bpb_holds_geometry(backup_record, image.geometry) {
                    return None;
                }
                Some(offset)
            }
        }
    } else {
        None
    };
    Some(FatVolume {
        volume_sectors,
        backup_boot_sector,
        has_nec_partition_start,
    })
}

/// Returns whether a partition of the table starts with a boot record whose
/// BPB holds the given geometry.
fn has_boot_record_in_geometry(image: &HddImage, table: &[u8], geometry: HddGeometry) -> bool {
    table
        .as_chunks::<PARTITION_ENTRY_SIZE>()
        .0
        .iter()
        .filter(|entry| !is_empty_entry(entry.as_slice()))
        .filter_map(|entry| {
            chs_to_lba(&entry[ENTRY_START_OFFSET..ENTRY_START_OFFSET + 4], geometry)
        })
        .filter_map(|start| image.read_sector(start))
        .any(|boot_record| bpb_holds_geometry(boot_record, geometry))
}

/// Returns whether a boot record carries a BPB with the sectors per track and
/// heads of the geometry.
fn bpb_holds_geometry(boot_record: &[u8], geometry: HddGeometry) -> bool {
    matches!(boot_record[0], 0xEB | 0xE9)
        && read_word(boot_record, BOOT_RECORD_SECTORS_PER_TRACK_OFFSET)
            == u16::from(geometry.sectors_per_track)
        && read_word(boot_record, BOOT_RECORD_HEADS_OFFSET) == u16::from(geometry.heads)
}

/// Returns the boot record with the BIOS geometry and the hidden sectors of
/// a partition at `start` in its BPB. A NEC boot record also takes `start`
/// as its partition start.
fn boot_record_in_bios_geometry(
    boot_record: &[u8],
    start: u32,
    has_nec_partition_start: bool,
) -> Vec<u8> {
    let mut new_boot_record = boot_record.to_vec();
    new_boot_record[BOOT_RECORD_SECTORS_PER_TRACK_OFFSET..][..2]
        .copy_from_slice(&u16::from(BIOS_SECTORS_PER_TRACK).to_le_bytes());
    new_boot_record[BOOT_RECORD_HEADS_OFFSET..][..2]
        .copy_from_slice(&u16::from(BIOS_HEADS).to_le_bytes());
    new_boot_record[BOOT_RECORD_HIDDEN_SECTORS_OFFSET..][..4].copy_from_slice(&start.to_le_bytes());
    if has_nec_partition_start {
        new_boot_record[NEC_BOOT_RECORD_PARTITION_START_OFFSET..][..4]
            .copy_from_slice(&start.to_le_bytes());
    }
    new_boot_record
}

/// Reads a little-endian word.
fn read_word(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

/// Reads a little-endian double word.
fn read_double_word(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

/// Decodes a partition table address. Returns `None` when its head or
/// sector lies outside the geometry.
fn chs_to_lba(address: &[u8], geometry: HddGeometry) -> Option<u32> {
    let sector = address[0];
    let head = address[1];
    let cylinder = u16::from_le_bytes([address[2], address[3]]);
    if head >= geometry.heads || sector >= geometry.sectors_per_track {
        return None;
    }
    Some(
        (u32::from(cylinder) * u32::from(geometry.heads) + u32::from(head))
            * u32::from(geometry.sectors_per_track)
            + u32::from(sector),
    )
}

/// Encodes an LBA of a disk in the BIOS geometry as a partition table
/// address.
fn lba_to_chs(lba: u32) -> [u8; 4] {
    let sectors_per_track = u32::from(BIOS_SECTORS_PER_TRACK);
    let track = lba / sectors_per_track;
    let [cylinder_low, cylinder_high] = ((track / u32::from(BIOS_HEADS)) as u16).to_le_bytes();
    [
        (lba % sectors_per_track) as u8,
        (track % u32::from(BIOS_HEADS)) as u8,
        cylinder_low,
        cylinder_high,
    ]
}

/// Encodes sector 0 of head 0 of a cylinder as a partition table address.
fn cylinder_address(cylinder: u16) -> [u8; 4] {
    let [cylinder_low, cylinder_high] = cylinder.to_le_bytes();
    [0, 0, cylinder_low, cylinder_high]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk::{
        HddFormat, HddSizeType, blank_hdd_image,
        format::{PartitionTableType, format_hdd_image},
        test_support::{
            image_in_image_geometry, neetan_formatted_in_image_geometry,
            pc98_image_in_image_geometry,
        },
    };

    /// Returns the IPL, start and end addresses of a partition table entry.
    fn entry_addresses(image: &HddImage, entry_index: usize) -> [[u8; 4]; 3] {
        let entry = &image.read_sector(PARTITION_TABLE_LBA).unwrap()[entry_index * 32..];
        [ENTRY_IPL_OFFSET, ENTRY_START_OFFSET, ENTRY_END_OFFSET]
            .map(|offset| entry[offset..offset + 4].try_into().unwrap())
    }

    /// Asserts that the partition of `entry_index` starts at `new_start`,
    /// that its end cylinder holds its volume within the BIOS cylinders, that
    /// its boot record carries the BIOS geometry and that its volume holds
    /// the data of the old partition at `old_start`.
    fn assert_relocated(
        image: &HddImage,
        original: &HddImage,
        entry_index: usize,
        old_start: u32,
        new_start: u32,
    ) {
        let [ipl, start, end] = entry_addresses(image, entry_index);
        assert_eq!(ipl, lba_to_chs(new_start));
        assert_eq!(start, lba_to_chs(new_start));
        assert_eq!(&end[..2], &[0, 0]);
        let end_cylinder = u32::from(u16::from_le_bytes([end[2], end[3]]));

        let boot_record = image.read_sector(new_start).unwrap();
        let original_boot_record = original.read_sector(old_start).unwrap();
        assert_eq!(&boot_record[24..28], &[17, 0, 8, 0]);
        assert_eq!(read_double_word(boot_record, 28), new_start);
        assert_eq!(&boot_record[..24], &original_boot_record[..24]);
        assert_eq!(&boot_record[32..], &original_boot_record[32..]);

        let volume = match read_word(boot_record, 19) {
            0 => read_double_word(boot_record, 32),
            total => u32::from(total),
        } * u32::from(read_word(boot_record, 11) / 512);
        assert!((end_cylinder + 1) * BIOS_SECTORS_PER_CYLINDER >= new_start + volume);
        assert!(end_cylinder < u32::from(bios_geometry(image.geometry).cylinders));
        for offset in 1..volume {
            assert_eq!(
                image.read_sector(new_start + offset),
                original.read_sector(old_start + offset),
                "volume sector {offset}"
            );
        }
    }

    fn assert_migrates(cylinders: u16, heads: u8, sectors_per_track: u8) {
        let original = image_in_image_geometry(cylinders, heads, sectors_per_track);
        let old_start = u32::from(heads) * u32::from(sectors_per_track);
        let mut image = original.clone();

        assert!(migrate_to_bios_geometry(&mut image));
        assert_eq!(image.geometry, original.geometry);
        assert_eq!(image.read_sector(0), original.read_sector(0));
        assert_relocated(&image, &original, 0, old_start, BIOS_SECTORS_PER_CYLINDER);

        let migrated = image.clone();
        assert!(!migrate_to_bios_geometry(&mut image));
        assert_eq!(image.data(), migrated.data());
    }

    fn assert_unchanged(mut image: HddImage) {
        let original = image.clone();
        assert!(!migrate_to_bios_geometry(&mut image));
        assert_eq!(image.data(), original.data());
    }

    #[test]
    fn bios_geometry_of_the_ide_disk_types() {
        let expected_cylinders = [
            (HddSizeType::IdeMb40, 610),
            (HddSizeType::IdeMb80, 1221),
            (HddSizeType::IdeMb120, 1831),
            (HddSizeType::IdeMb200, 3017),
            (HddSizeType::IdeMb500, 7522),
        ];
        for (size, cylinders) in expected_cylinders {
            let (geometry, _) = size.geometry();
            assert_eq!(
                bios_geometry(geometry),
                HddGeometry {
                    cylinders,
                    heads: 8,
                    sectors_per_track: 17,
                    sector_size: 512,
                },
                "{}",
                size.as_token()
            );
        }
    }

    #[test]
    fn bios_geometry_caps_the_cylinders() {
        let geometry = HddGeometry {
            cylinders: 65535,
            heads: 16,
            sectors_per_track: 63,
            sector_size: 512,
        };
        assert_eq!(bios_geometry(geometry).cylinders, 65535);
    }

    #[test]
    fn bios_geometry_keeps_256_byte_sector_disks() {
        let (geometry, _) = HddSizeType::Mb40.geometry();
        assert_eq!(bios_geometry(geometry), geometry);
    }

    #[test]
    fn migrates_a_10_head_17_sector_layout() {
        assert_migrates(977, 10, 17);
    }

    #[test]
    fn migrates_a_15_head_28_sector_layout() {
        assert_migrates(977, 15, 28);
    }

    #[test]
    fn migrates_a_16_head_63_sector_layout() {
        assert_migrates(1015, 16, 63);
    }

    #[test]
    fn migrates_two_partitions_back_to_back() {
        let original =
            pc98_image_in_image_geometry(977, 15, 17, &[(1, 400, 101_000), (401, 970, 145_000)]);
        let mut image = original.clone();

        assert!(migrate_to_bios_geometry(&mut image));
        assert_relocated(&image, &original, 0, 255, 136);
        let [_, _, first_end] = entry_addresses(&image, 0);
        let second_start = (u32::from(u16::from_le_bytes([first_end[2], first_end[3]])) + 1) * 136;
        assert_relocated(&image, &original, 1, 401 * 255, second_start);

        let migrated = image.clone();
        assert!(!migrate_to_bios_geometry(&mut image));
        assert_eq!(image.data(), migrated.data());
    }

    #[test]
    fn migrates_the_fat32_backup_boot_sector() {
        let mut image = image_in_image_geometry(977, 10, 17);
        let mut boot_record = image.read_sector(170).unwrap().to_vec();
        boot_record[22..24].copy_from_slice(&0u16.to_le_bytes());
        boot_record[50..52].copy_from_slice(&6u16.to_le_bytes());
        image.write_sector(170, &boot_record);
        image.write_sector(170 + 24, &boot_record);

        assert!(migrate_to_bios_geometry(&mut image));
        let backup = image.read_sector(136 + 24).unwrap();
        assert_eq!(&backup[24..28], &[17, 0, 8, 0]);
        assert_eq!(read_double_word(backup, 28), 136);
    }

    #[test]
    fn migrates_the_partition_start_of_a_nec_boot_record() {
        let mut image = image_in_image_geometry(977, 10, 17);
        let mut boot_record = image.read_sector(170).unwrap().to_vec();
        boot_record[0x3E..0x42].copy_from_slice(&170u32.to_le_bytes());
        image.write_sector(170, &boot_record);

        assert!(migrate_to_bios_geometry(&mut image));
        let boot_record = image.read_sector(136).unwrap();
        assert_eq!(read_double_word(boot_record, 0x3E), 136);
    }

    #[test]
    fn leaves_a_blank_image_unchanged() {
        assert_unchanged(blank_hdd_image(HddSizeType::IdeMb500));
    }

    #[test]
    fn leaves_an_image_formatted_in_the_bios_geometry_unchanged() {
        let mut image = blank_hdd_image(HddSizeType::IdeMb500);
        format_hdd_image(&mut image, PartitionTableType::Pc98).unwrap();
        assert_unchanged(image);
    }

    #[test]
    fn leaves_a_migrated_image_with_an_old_boot_record_copy_unchanged() {
        let original = image_in_image_geometry(977, 10, 17);
        let mut image = original.clone();
        assert!(migrate_to_bios_geometry(&mut image));
        image.write_sector(170, original.read_sector(170).unwrap());
        assert_unchanged(image);
    }

    #[test]
    fn leaves_a_boot_record_in_another_geometry_unchanged() {
        let mut image = image_in_image_geometry(977, 10, 17);
        let mut boot_record = image.read_sector(170).unwrap().to_vec();
        boot_record[26] = 15;
        image.write_sector(170, &boot_record);
        assert_unchanged(image);
    }

    #[test]
    fn leaves_an_address_outside_the_image_geometry_unchanged() {
        let mut image = image_in_image_geometry(977, 10, 17);
        let mut table = image.read_sector(PARTITION_TABLE_LBA).unwrap().to_vec();
        table[9] = 10;
        image.write_sector(PARTITION_TABLE_LBA, &table);
        assert_unchanged(image);
    }

    /// Asserts that a migration that appends cylinders to the image moves
    /// the partition of entry 0 from `old_start` to cylinder 1 with its whole
    /// volume, and that the header geometry covers the enlarged image.
    fn assert_migrates_by_enlarging(original: HddImage, old_start: u32, cylinders: u16) {
        let mut image = original.clone();

        assert!(migrate_to_bios_geometry(&mut image));
        assert_eq!(
            image.geometry,
            HddGeometry {
                cylinders,
                ..original.geometry
            }
        );
        let reparsed = HddImage::from_hdi(&image.to_bytes()).unwrap();
        assert_eq!(reparsed.geometry, image.geometry);
        assert_eq!(
            read_double_word(&reparsed.header_bytes, HDI_DATA_SIZE_OFFSET),
            image.geometry.total_sectors()
        );
        assert_relocated(&image, &original, 0, old_start, BIOS_SECTORS_PER_CYLINDER);

        let migrated = image.clone();
        assert!(!migrate_to_bios_geometry(&mut image));
        assert_eq!(image.data(), migrated.data());
    }

    #[test]
    fn migrates_a_neetan_formatted_ide200_image_by_enlarging() {
        assert_migrates_by_enlarging(
            neetan_formatted_in_image_geometry(HddSizeType::IdeMb200),
            28,
            978,
        );
    }

    #[test]
    fn migrates_a_neetan_formatted_ide500_image_by_enlarging() {
        assert_migrates_by_enlarging(
            neetan_formatted_in_image_geometry(HddSizeType::IdeMb500),
            63,
            1016,
        );
    }

    #[test]
    fn migrates_the_nec_fdisk_layout_of_a_5_head_17_sector_disk_by_enlarging() {
        assert_migrates_by_enlarging(
            pc98_image_in_image_geometry(977, 5, 17, &[(1, 976, 82_956)]),
            85,
            978,
        );
    }

    #[test]
    fn leaves_an_image_past_the_bios_cylinder_limit_unchanged() {
        let geometry = HddGeometry {
            cylinders: 65535,
            heads: 16,
            sectors_per_track: 63,
            sector_size: 512,
        };
        let template = pc98_image_in_image_geometry(4, 16, 63, &[(1, 2, 2000)]);
        let mut data = template.data().to_vec();
        let mut table = data[512..1024].to_vec();
        table[14..16].copy_from_slice(&65534u16.to_le_bytes());
        data[512..1024].copy_from_slice(&table);
        let boot_record = 1008 * 512;
        data[boot_record + 19..boot_record + 21].copy_from_slice(&0u16.to_le_bytes());
        data[boot_record + 32..boot_record + 36]
            .copy_from_slice(&((65533 * 1008 - 4) / 4u32).to_le_bytes());
        assert_unchanged(HddImage::from_raw(geometry, HddFormat::Hdi, data));
    }

    #[test]
    fn leaves_an_image_with_a_partition_without_a_fat_volume_unchanged() {
        let mut image =
            pc98_image_in_image_geometry(977, 15, 17, &[(1, 400, 101_000), (401, 970, 145_000)]);
        let second_start = 401 * 255;
        let mut boot_record = image.read_sector(second_start).unwrap().to_vec();
        boot_record[0] = 0x00;
        image.write_sector(second_start, &boot_record);
        assert_unchanged(image);
    }

    #[test]
    fn migrates_a_partition_backward_and_a_later_one_forward() {
        let original =
            pc98_image_in_image_geometry(977, 9, 17, &[(1, 10, 1528), (11, 970, 146_724)]);
        let mut image = original.clone();

        assert!(migrate_to_bios_geometry(&mut image));
        assert_relocated(&image, &original, 0, 153, 136);
        assert_relocated(&image, &original, 1, 11 * 153, 13 * 136);

        let migrated = image.clone();
        assert!(!migrate_to_bios_geometry(&mut image));
        assert_eq!(image.data(), migrated.data());
    }

    #[test]
    fn migrates_an_nhd_image_by_enlarging() {
        let hdi = neetan_formatted_in_image_geometry(HddSizeType::IdeMb500);
        let original = HddImage::from_raw(hdi.geometry, HddFormat::Nhd, hdi.data().to_vec());
        let mut image = original.clone();

        assert!(migrate_to_bios_geometry(&mut image));
        let reparsed = HddImage::from_nhd(&image.to_bytes()).unwrap();
        assert_eq!(reparsed.geometry.cylinders, 1016);
        assert_eq!(reparsed.data(), image.data());
        assert_relocated(&image, &original, 0, 63, 136);
    }

    #[test]
    fn treats_an_entry_with_zero_mid_and_sid_as_empty() {
        let mut image = image_in_image_geometry(977, 10, 17);
        let mut table = image.read_sector(PARTITION_TABLE_LBA).unwrap().to_vec();
        table[32 + 8..32 + 16].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        image.write_sector(PARTITION_TABLE_LBA, &table);
        let original = image.clone();

        assert!(migrate_to_bios_geometry(&mut image));
        assert_relocated(&image, &original, 0, 170, 136);
    }

    #[test]
    fn leaves_an_hdi_image_with_an_unknown_data_size_unchanged() {
        let mut image = neetan_formatted_in_image_geometry(HddSizeType::IdeMb500);
        image.header_bytes[HDI_DATA_SIZE_OFFSET..HDI_DATA_SIZE_OFFSET + 4]
            .copy_from_slice(&12345u32.to_le_bytes());
        assert_unchanged(image);
    }

    #[test]
    fn leaves_an_ipl_outside_the_new_partition_unchanged() {
        let mut image = image_in_image_geometry(977, 15, 17);
        let ipl = 255 + 247_850u32;
        let mut table = image.read_sector(PARTITION_TABLE_LBA).unwrap().to_vec();
        table[4] = (ipl % 17) as u8;
        table[5] = (ipl / 17 % 15) as u8;
        table[6..8].copy_from_slice(&((ipl / 255) as u16).to_le_bytes());
        image.write_sector(PARTITION_TABLE_LBA, &table);
        assert_unchanged(image);
    }

    #[test]
    fn leaves_an_image_without_a_fat_partition_unchanged() {
        let mut image = image_in_image_geometry(977, 10, 17);
        let mut boot_record = image.read_sector(170).unwrap().to_vec();
        boot_record[0] = 0x00;
        image.write_sector(170, &boot_record);
        assert_unchanged(image);
    }
}
