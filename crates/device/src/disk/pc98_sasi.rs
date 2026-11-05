//! PC-98 SASI disks and the migration of their images to the NEC layout.
//!
//! A PC-98 disk with 256-byte sectors is a SASI disk. Its NEC layout uses
//! the image geometry. Cylinder 0 holds the IPL and the partition table. The
//! partition starts on cylinder 1 and ends on the last cylinder.
//!
//! The migration relocates the FAT12 or FAT16 volume of an image that neetan
//! formatted with its partition in cylinder 0 to cylinder 1. The partition
//! name of such an image is the neetan partition name. A volume larger than its new span shrinks: every used
//! cluster past the new end takes a free cluster of the volume, and the FAT
//! chains and the directory entries follow it. The FAT keeps its size and
//! its type. An image the migration cannot relocate safely stays unchanged.

use std::ops::Range;

use super::{
    HddImage,
    format::{PC98_NEC_PARTITION_START_CYLINDER, PC98_PARTITION_NAME},
    pc98_ide::{
        BOOT_RECORD_BYTES_PER_SECTOR_OFFSET, BOOT_RECORD_SECTORS_PER_FAT_16_OFFSET,
        BOOT_RECORD_TOTAL_SECTORS_16_OFFSET, BOOT_RECORD_TOTAL_SECTORS_32_OFFSET, ENTRY_END_OFFSET,
        ENTRY_IPL_OFFSET, ENTRY_START_OFFSET, PARTITION_ENTRY_SIZE, PARTITION_TABLE_LBA,
        boot_record_in_geometry, cylinder_address, is_empty_entry, read_double_word,
        read_partition, read_word,
    },
};

/// Sector size of a PC-98 SASI disk.
const SASI_SECTOR_SIZE: u16 = 256;

/// Partition table entry offset of the partition name.
const ENTRY_NAME_OFFSET: usize = 16;

/// Boot record offset of the sectors per cluster byte.
const BOOT_RECORD_SECTORS_PER_CLUSTER_OFFSET: usize = 13;

/// Boot record offset of the reserved sectors word.
const BOOT_RECORD_RESERVED_SECTORS_OFFSET: usize = 14;

/// Boot record offset of the FAT count byte.
const BOOT_RECORD_FAT_COUNT_OFFSET: usize = 16;

/// Boot record offset of the root directory entry count word.
const BOOT_RECORD_ROOT_ENTRIES_OFFSET: usize = 17;

/// Lowest cluster count of a FAT16 volume.
const FAT16_MINIMUM_CLUSTERS: u32 = 4085;

/// Lowest cluster count past a FAT16 volume.
const FAT16_CLUSTER_LIMIT: u32 = 65525;

/// Number of the first data cluster.
const FIRST_CLUSTER: u32 = 2;

/// Size of a directory entry.
const DIRECTORY_ENTRY_SIZE: usize = 32;

/// Directory entry offset of the attribute byte.
const DIRECTORY_ENTRY_ATTRIBUTE_OFFSET: usize = 11;

/// Directory entry offset of the start cluster word.
const DIRECTORY_ENTRY_CLUSTER_OFFSET: usize = 26;

/// First name byte of the entry that ends a directory.
const END_OF_DIRECTORY_MARKER: u8 = 0x00;

/// First name byte of a deleted entry.
const DELETED_ENTRY_MARKER: u8 = 0xE5;

/// First name byte of the "." and ".." entries.
const DOT_ENTRY_MARKER: u8 = b'.';

/// Attribute bit of a volume label.
const ATTRIBUTE_VOLUME_LABEL: u8 = 0x08;

/// Attribute bit of a directory.
const ATTRIBUTE_DIRECTORY: u8 = 0x10;

/// Attribute of a long file name entry.
const ATTRIBUTE_LONG_NAME: u8 = 0x0F;

/// The relocated volume and its place in the NEC layout.
struct Migration {
    entry_index: usize,
    new_start: u32,
    volume: Vec<u8>,
    has_nec_partition_start: bool,
}

/// The layout of a FAT12 or FAT16 volume, in bytes from the volume start.
struct FatLayout {
    bytes_per_sector: usize,
    sectors_per_cluster: u32,
    fat_count: usize,
    fat_size: usize,
    first_fat_offset: usize,
    root_offset: usize,
    root_size: usize,
    data_sector: u32,
    data_offset: usize,
    cluster_count: u32,
    is_fat16: bool,
}

/// Returns whether a PC-98 disk with this sector size is a SASI disk, which
/// takes the NEC layout in its image geometry.
pub fn is_sasi_sector_size(sector_size: u16) -> bool {
    sector_size == SASI_SECTOR_SIZE
}

/// Relocates the FAT volume of a SASI image that neetan formatted with its
/// only partition in cylinder 0 into the NEC layout. Returns whether the
/// image changed. An image in the NEC layout, from another tool, with
/// another partition table, with a volume that is not readable, or with too
/// few free clusters to shrink the volume stays unchanged.
pub fn migrate_to_nec_layout(image: &mut HddImage) -> bool {
    let Some(migration) = plan_migration(image) else {
        return false;
    };
    let Some(table) = image.read_sector(PARTITION_TABLE_LBA) else {
        return false;
    };
    let mut table = table.to_vec();

    let sector_size = usize::from(image.geometry.sector_size);
    let new_start_offset = migration.new_start as usize * sector_size;
    image.data[new_start_offset..new_start_offset + migration.volume.len()]
        .copy_from_slice(&migration.volume);

    let boot_record = boot_record_in_geometry(
        &migration.volume[..sector_size],
        image.geometry,
        migration.new_start,
        migration.has_nec_partition_start,
    );
    image.write_sector(migration.new_start, &boot_record);

    let entry = &mut table[migration.entry_index * PARTITION_ENTRY_SIZE..][..PARTITION_ENTRY_SIZE];
    let start = cylinder_address(PC98_NEC_PARTITION_START_CYLINDER);
    entry[ENTRY_IPL_OFFSET..ENTRY_IPL_OFFSET + 4].copy_from_slice(&start);
    entry[ENTRY_START_OFFSET..ENTRY_START_OFFSET + 4].copy_from_slice(&start);
    entry[ENTRY_END_OFFSET..ENTRY_END_OFFSET + 4]
        .copy_from_slice(&cylinder_address(image.geometry.cylinders - 1));
    image.write_sector(PARTITION_TABLE_LBA, &table);
    true
}

/// Plans the migration and builds the relocated volume. Returns `None` when
/// the image needs no migration or cannot be migrated safely.
fn plan_migration(image: &HddImage) -> Option<Migration> {
    if !is_sasi_sector_size(image.geometry.sector_size) {
        return None;
    }
    let geometry = image.geometry;
    let sectors_per_cylinder = u32::from(geometry.heads) * u32::from(geometry.sectors_per_track);
    let table = image.read_sector(PARTITION_TABLE_LBA)?;
    let mut entries = table
        .as_chunks::<PARTITION_ENTRY_SIZE>()
        .0
        .iter()
        .enumerate()
        .filter(|(_, entry)| !is_empty_entry(entry.as_slice()));
    let (entry_index, entry) = entries.next()?;
    if entries.next().is_some()
        || entry[ENTRY_NAME_OFFSET..ENTRY_NAME_OFFSET + PC98_PARTITION_NAME.len()]
            != PC98_PARTITION_NAME[..]
    {
        return None;
    }
    let partition = read_partition(image, entry_index, entry)?;
    if partition.start <= PARTITION_TABLE_LBA
        || partition.start >= sectors_per_cylinder
        || partition.ipl != partition.start
    {
        return None;
    }

    let new_start = u32::from(PC98_NEC_PARTITION_START_CYLINDER) * sectors_per_cylinder;
    let new_span = geometry.total_sectors().checked_sub(new_start)?;
    let sector_size = usize::from(geometry.sector_size);
    let start_offset = partition.start as usize * sector_size;
    let volume_size = partition.volume.volume_sectors as usize * sector_size;
    let mut volume = image.data[start_offset..start_offset + volume_size].to_vec();
    if partition.volume.volume_sectors > new_span {
        shrink_volume(&mut volume, new_span as usize * sector_size)?;
    }
    Some(Migration {
        entry_index: partition.entry_index,
        new_start,
        volume,
        has_nec_partition_start: partition.volume.has_nec_partition_start,
    })
}

/// Shrinks a FAT12 or FAT16 volume to at most `size_limit` bytes. Moves the
/// used clusters past the new end into free clusters, rewrites the FAT
/// copies and the start clusters of the directory entries, lowers the total
/// sectors of the BPB and truncates the volume. Returns `None` before any
/// change when the volume is not readable, has too few free clusters, or
/// would change its FAT type. Returns `None` after a partial change when a
/// directory entry points past the new end or a directory chain is broken.
fn shrink_volume(volume: &mut Vec<u8>, size_limit: usize) -> Option<()> {
    let layout = FatLayout::read(volume)?;
    let new_total_sectors = u32::try_from(size_limit / layout.bytes_per_sector).ok()?;
    let new_cluster_count =
        new_total_sectors.checked_sub(layout.data_sector)? / layout.sectors_per_cluster;
    if new_cluster_count == 0 || (new_cluster_count >= FAT16_MINIMUM_CLUSTERS) != layout.is_fat16 {
        return None;
    }

    let fat = volume[layout.fat_range(0)].to_vec();
    if (1..layout.fat_count).any(|index| volume[layout.fat_range(index)] != fat[..]) {
        return None;
    }
    let old_end = FIRST_CLUSTER + layout.cluster_count;
    let new_end = FIRST_CLUSTER + new_cluster_count;
    let is_fat16 = layout.is_fat16;
    let moved: Vec<u32> = (new_end..old_end)
        .filter(|&cluster| is_used(fat_entry(&fat, is_fat16, cluster), is_fat16))
        .collect();
    let free: Vec<u32> = (FIRST_CLUSTER..new_end)
        .filter(|&cluster| fat_entry(&fat, is_fat16, cluster) == 0)
        .take(moved.len())
        .collect();
    if free.len() < moved.len() {
        return None;
    }

    let mut remap: Vec<u32> = (0..old_end).collect();
    for (&old_cluster, &new_cluster) in moved.iter().zip(&free) {
        remap[old_cluster as usize] = new_cluster;
    }
    let remap_link = |value: u32| {
        if (FIRST_CLUSTER..old_end).contains(&value) {
            remap[value as usize]
        } else {
            value
        }
    };
    let mut new_fat = fat.clone();
    for cluster in FIRST_CLUSTER..new_end {
        let value = remap_link(fat_entry(&fat, is_fat16, cluster));
        set_fat_entry(&mut new_fat, is_fat16, cluster, value);
    }
    for (&old_cluster, &new_cluster) in moved.iter().zip(&free) {
        let value = remap_link(fat_entry(&fat, is_fat16, old_cluster));
        set_fat_entry(&mut new_fat, is_fat16, new_cluster, value);
    }
    for cluster in new_end..old_end {
        set_fat_entry(&mut new_fat, is_fat16, cluster, 0);
    }
    let links_stay_inside = (FIRST_CLUSTER..new_end).all(|cluster| {
        let value = fat_entry(&new_fat, is_fat16, cluster);
        value == 0
            || is_end_of_chain(value, is_fat16)
            || is_bad_cluster(value, is_fat16)
            || (FIRST_CLUSTER..new_end).contains(&value)
    });
    if !links_stay_inside {
        return None;
    }

    for (&old_cluster, &new_cluster) in moved.iter().zip(&free) {
        volume.copy_within(
            layout.cluster_range(old_cluster),
            layout.cluster_range(new_cluster).start,
        );
    }
    for index in 0..layout.fat_count {
        volume[layout.fat_range(index)].copy_from_slice(&new_fat);
    }
    remap_directories(volume, &layout, &new_fat, new_end, &remap)?;

    if read_word(volume, BOOT_RECORD_TOTAL_SECTORS_16_OFFSET) != 0 {
        volume[BOOT_RECORD_TOTAL_SECTORS_16_OFFSET..][..2]
            .copy_from_slice(&u16::try_from(new_total_sectors).ok()?.to_le_bytes());
    } else {
        volume[BOOT_RECORD_TOTAL_SECTORS_32_OFFSET..][..4]
            .copy_from_slice(&new_total_sectors.to_le_bytes());
    }
    volume.truncate(new_total_sectors as usize * layout.bytes_per_sector);
    Some(())
}

/// Rewrites the start cluster of every entry in the root directory and all
/// subdirectories through `remap`. Follows the subdirectories through the
/// rewritten FAT. Returns `None` when an entry points past the new end
/// of the volume or a directory chain is broken.
fn remap_directories(
    volume: &mut [u8],
    layout: &FatLayout,
    fat: &[u8],
    new_end: u32,
    remap: &[u32],
) -> Option<()> {
    let mut visited = vec![false; new_end as usize];
    let mut pending = vec![vec![
        layout.root_offset..layout.root_offset + layout.root_size,
    ]];
    while let Some(regions) = pending.pop() {
        'directory: for region in regions {
            for offset in region.step_by(DIRECTORY_ENTRY_SIZE) {
                let entry = &mut volume[offset..offset + DIRECTORY_ENTRY_SIZE];
                match entry[0] {
                    END_OF_DIRECTORY_MARKER => break 'directory,
                    DELETED_ENTRY_MARKER => continue,
                    _ => {}
                }
                let attribute = entry[DIRECTORY_ENTRY_ATTRIBUTE_OFFSET];
                if attribute == ATTRIBUTE_LONG_NAME || attribute & ATTRIBUTE_VOLUME_LABEL != 0 {
                    continue;
                }
                let cluster = read_word(entry, DIRECTORY_ENTRY_CLUSTER_OFFSET);
                let new_cluster = *remap.get(usize::from(cluster))?;
                if new_cluster != 0 && !(FIRST_CLUSTER..new_end).contains(&new_cluster) {
                    return None;
                }
                entry[DIRECTORY_ENTRY_CLUSTER_OFFSET..][..2]
                    .copy_from_slice(&(new_cluster as u16).to_le_bytes());
                if attribute & ATTRIBUTE_DIRECTORY == 0
                    || entry[0] == DOT_ENTRY_MARKER
                    || new_cluster == 0
                {
                    continue;
                }
                if std::mem::replace(&mut visited[new_cluster as usize], true) {
                    return None;
                }
                let chain = cluster_chain(fat, layout.is_fat16, new_cluster, new_end)?;
                pending.push(
                    chain
                        .into_iter()
                        .map(|cluster| layout.cluster_range(cluster))
                        .collect(),
                );
            }
        }
    }
    Some(())
}

/// Returns the clusters of the chain from `start`. Returns `None` when the
/// chain leaves the clusters below `end` or loops.
fn cluster_chain(fat: &[u8], is_fat16: bool, start: u32, end: u32) -> Option<Vec<u32>> {
    let mut chain = Vec::new();
    let mut cluster = start;
    loop {
        if !(FIRST_CLUSTER..end).contains(&cluster) || chain.len() >= end as usize {
            return None;
        }
        chain.push(cluster);
        let value = fat_entry(fat, is_fat16, cluster);
        if is_end_of_chain(value, is_fat16) {
            return Some(chain);
        }
        cluster = value;
    }
}

impl FatLayout {
    /// Reads the layout from the BPB at the start of a volume. Returns `None`
    /// when the BPB does not describe a FAT12 or FAT16 volume inside the
    /// volume bytes.
    fn read(volume: &[u8]) -> Option<Self> {
        let bytes_per_sector = usize::from(read_word(volume, BOOT_RECORD_BYTES_PER_SECTOR_OFFSET));
        let sectors_per_cluster = u32::from(volume[BOOT_RECORD_SECTORS_PER_CLUSTER_OFFSET]);
        let reserved_sectors = u32::from(read_word(volume, BOOT_RECORD_RESERVED_SECTORS_OFFSET));
        let fat_count = usize::from(volume[BOOT_RECORD_FAT_COUNT_OFFSET]);
        let root_entries = usize::from(read_word(volume, BOOT_RECORD_ROOT_ENTRIES_OFFSET));
        let sectors_per_fat = u32::from(read_word(volume, BOOT_RECORD_SECTORS_PER_FAT_16_OFFSET));
        let total_sectors = match read_word(volume, BOOT_RECORD_TOTAL_SECTORS_16_OFFSET) {
            0 => read_double_word(volume, BOOT_RECORD_TOTAL_SECTORS_32_OFFSET),
            total => u32::from(total),
        };
        if !bytes_per_sector.is_power_of_two()
            || !sectors_per_cluster.is_power_of_two()
            || reserved_sectors == 0
            || fat_count == 0
            || sectors_per_fat == 0
        {
            return None;
        }

        let root_sectors = (root_entries * DIRECTORY_ENTRY_SIZE).div_ceil(bytes_per_sector);
        let data_sector = reserved_sectors
            + u32::try_from(fat_count).ok()? * sectors_per_fat
            + u32::try_from(root_sectors).ok()?;
        let cluster_count = total_sectors.checked_sub(data_sector)? / sectors_per_cluster;
        if cluster_count == 0
            || cluster_count >= FAT16_CLUSTER_LIMIT
            || total_sectors as usize * bytes_per_sector > volume.len()
        {
            return None;
        }
        let is_fat16 = cluster_count >= FAT16_MINIMUM_CLUSTERS;
        let fat_size = sectors_per_fat as usize * bytes_per_sector;
        let entry_bits = if is_fat16 { 16 } else { 12 };
        if fat_size * 8 / entry_bits < (FIRST_CLUSTER + cluster_count) as usize {
            return None;
        }
        let first_fat_offset = reserved_sectors as usize * bytes_per_sector;
        let root_offset = first_fat_offset + fat_count * fat_size;
        Some(Self {
            bytes_per_sector,
            sectors_per_cluster,
            fat_count,
            fat_size,
            first_fat_offset,
            root_offset,
            root_size: root_sectors * bytes_per_sector,
            data_sector,
            data_offset: data_sector as usize * bytes_per_sector,
            cluster_count,
            is_fat16,
        })
    }

    /// Returns the byte range of a FAT copy.
    fn fat_range(&self, index: usize) -> Range<usize> {
        let start = self.first_fat_offset + index * self.fat_size;
        start..start + self.fat_size
    }

    /// Returns the byte range of a data cluster.
    fn cluster_range(&self, cluster: u32) -> Range<usize> {
        let size = self.sectors_per_cluster as usize * self.bytes_per_sector;
        let start = self.data_offset + (cluster - FIRST_CLUSTER) as usize * size;
        start..start + size
    }
}

/// Reads the FAT entry of a cluster.
fn fat_entry(fat: &[u8], is_fat16: bool, cluster: u32) -> u32 {
    let cluster = cluster as usize;
    if is_fat16 {
        return u32::from(read_word(fat, cluster * 2));
    }
    let word = read_word(fat, cluster * 3 / 2);
    u32::from(if cluster % 2 == 1 {
        word >> 4
    } else {
        word & 0x0FFF
    })
}

/// Writes the FAT entry of a cluster.
fn set_fat_entry(fat: &mut [u8], is_fat16: bool, cluster: u32, value: u32) {
    let cluster = cluster as usize;
    if is_fat16 {
        fat[cluster * 2..cluster * 2 + 2].copy_from_slice(&(value as u16).to_le_bytes());
        return;
    }
    let offset = cluster * 3 / 2;
    let word = read_word(fat, offset);
    let value = value as u16 & 0x0FFF;
    let word = if cluster % 2 == 1 {
        (word & 0x000F) | (value << 4)
    } else {
        (word & 0xF000) | value
    };
    fat[offset..offset + 2].copy_from_slice(&word.to_le_bytes());
}

/// Returns whether a FAT entry ends a chain.
fn is_end_of_chain(value: u32, is_fat16: bool) -> bool {
    value >= if is_fat16 { 0xFFF8 } else { 0x0FF8 }
}

/// Returns whether a FAT entry marks a bad cluster.
fn is_bad_cluster(value: u32, is_fat16: bool) -> bool {
    value == if is_fat16 { 0xFFF7 } else { 0x0FF7 }
}

/// Returns whether a FAT entry belongs to a file or a directory.
fn is_used(value: u32, is_fat16: bool) -> bool {
    value != 0 && !is_bad_cluster(value, is_fat16)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk::{
        HddGeometry, HddSizeType,
        format::{PartitionTableType, format_hdd_image},
        test_support::neetan_formatted_sasi_in_cylinder_0,
    };

    /// LBA of the volume start on head 1 of cylinder 0.
    const OLD_START: u32 = 33;

    /// Returns the geometry of a disk type.
    fn geometry(size: HddSizeType) -> HddGeometry {
        size.geometry().0
    }

    /// Returns the LBA of cylinder 1 of an image.
    fn new_start(image: &HddImage) -> u32 {
        u32::from(image.geometry.heads) * u32::from(image.geometry.sectors_per_track)
    }

    /// Returns the volume bytes of an image from `start`.
    fn volume(image: &mut HddImage, start: u32) -> &mut [u8] {
        &mut image.data[start as usize * 256..]
    }

    /// Returns the cluster end after the shrink and the cluster end before
    /// it of an image with its volume in cylinder 0.
    fn cluster_ends(image: &mut HddImage) -> (u32, u32) {
        let new_span = image.geometry.total_sectors() - new_start(image);
        let layout = FatLayout::read(volume(image, OLD_START)).unwrap();
        let new_clusters = (new_span * 256 / layout.bytes_per_sector as u32 - layout.data_sector)
            / layout.sectors_per_cluster;
        (
            FIRST_CLUSTER + new_clusters,
            FIRST_CLUSTER + layout.cluster_count,
        )
    }

    /// Writes a chain over `clusters` into every FAT copy and `content` into
    /// the clusters.
    fn write_chain(image: &mut HddImage, clusters: &[u32], content: &[u8]) {
        let volume = volume(image, OLD_START);
        let layout = FatLayout::read(volume).unwrap();
        for (index, &cluster) in clusters.iter().enumerate() {
            let value = clusters.get(index + 1).copied().unwrap_or(0xFFFF);
            set_fat_entries(volume, &layout, cluster, value);
            let range = layout.cluster_range(cluster);
            let size = range.len();
            let chunk = content.get(index * size..).unwrap_or(&[]);
            let chunk = &chunk[..chunk.len().min(size)];
            volume[range.start..range.start + chunk.len()].copy_from_slice(chunk);
        }
    }

    /// Writes the FAT entry of a cluster into every FAT copy.
    fn set_fat_entries(volume: &mut [u8], layout: &FatLayout, cluster: u32, value: u32) {
        for copy in 0..layout.fat_count {
            let range = layout.fat_range(copy);
            set_fat_entry(&mut volume[range], layout.is_fat16, cluster, value);
        }
    }

    /// Writes directory entries to the start of the root directory.
    fn write_root(image: &mut HddImage, entries: &[u8]) {
        let volume = volume(image, OLD_START);
        let root_offset = FatLayout::read(volume).unwrap().root_offset;
        volume[root_offset..root_offset + entries.len()].copy_from_slice(entries);
    }

    /// Builds a directory entry.
    fn directory_entry(name: &[u8; 11], attribute: u8, cluster: u32, size: u32) -> Vec<u8> {
        let mut entry = vec![0u8; DIRECTORY_ENTRY_SIZE];
        entry[..11].copy_from_slice(name);
        entry[DIRECTORY_ENTRY_ATTRIBUTE_OFFSET] = attribute;
        entry[DIRECTORY_ENTRY_CLUSTER_OFFSET..][..2]
            .copy_from_slice(&(cluster as u16).to_le_bytes());
        entry[28..32].copy_from_slice(&size.to_le_bytes());
        entry
    }

    /// Returns `size` bytes of a pattern seeded by `seed`.
    fn pattern(seed: u8, size: usize) -> Vec<u8> {
        (0..size)
            .map(|index| (index as u32 * 7 + u32::from(seed) * 13) as u8)
            .collect()
    }

    /// Returns the directory entries of the root directory or of the
    /// directory chain at `cluster` of the volume at `start`.
    fn directory_entries(image: &HddImage, start: u32, cluster: u32) -> Vec<Vec<u8>> {
        let volume = &image.data[start as usize * 256..];
        let layout = FatLayout::read(volume).unwrap();
        let fat = &volume[layout.fat_range(0)];
        let regions: Vec<Range<usize>> = if cluster == 0 {
            std::iter::once(layout.root_offset..layout.root_offset + layout.root_size).collect()
        } else {
            let end = FIRST_CLUSTER + layout.cluster_count;
            cluster_chain(fat, layout.is_fat16, cluster, end)
                .unwrap()
                .into_iter()
                .map(|cluster| layout.cluster_range(cluster))
                .collect()
        };
        regions
            .into_iter()
            .flat_map(|region| region.step_by(DIRECTORY_ENTRY_SIZE))
            .map(|offset| volume[offset..offset + DIRECTORY_ENTRY_SIZE].to_vec())
            .take_while(|entry| entry[0] != END_OF_DIRECTORY_MARKER)
            .filter(|entry| entry[0] != DELETED_ENTRY_MARKER)
            .collect()
    }

    /// Returns the directory entry at `path` of the volume at `start`.
    fn find_entry(image: &HddImage, start: u32, path: &[&[u8; 11]]) -> Vec<u8> {
        let mut cluster = 0;
        let mut found = Vec::new();
        for name in path {
            found = directory_entries(image, start, cluster)
                .into_iter()
                .find(|entry| &entry[..11] == name.as_slice())
                .unwrap();
            cluster = u32::from(read_word(&found, DIRECTORY_ENTRY_CLUSTER_OFFSET));
        }
        found
    }

    /// Reads the file at `path` of the volume at `start`.
    fn read_file(image: &HddImage, start: u32, path: &[&[u8; 11]]) -> Vec<u8> {
        let entry = find_entry(image, start, path);
        let size = read_double_word(&entry, 28) as usize;
        let volume = &image.data[start as usize * 256..];
        let layout = FatLayout::read(volume).unwrap();
        let fat = &volume[layout.fat_range(0)];
        let end = FIRST_CLUSTER + layout.cluster_count;
        let cluster = u32::from(read_word(&entry, DIRECTORY_ENTRY_CLUSTER_OFFSET));
        let mut content: Vec<u8> = cluster_chain(fat, layout.is_fat16, cluster, end)
            .unwrap()
            .into_iter()
            .flat_map(|cluster| volume[layout.cluster_range(cluster)].to_vec())
            .collect();
        content.truncate(size);
        content
    }

    /// Asserts that a second migration leaves the image unchanged.
    fn assert_idempotent(image: &mut HddImage) {
        let migrated = image.clone();
        assert!(!migrate_to_nec_layout(image));
        assert_eq!(image.data(), migrated.data());
    }

    /// Asserts that the migration leaves the image unchanged.
    fn assert_unchanged(mut image: HddImage) {
        let original = image.clone();
        assert!(!migrate_to_nec_layout(&mut image));
        assert_eq!(image.data(), original.data());
    }

    /// Asserts that an empty volume of the geometry migrates to the NEC
    /// layout and keeps its FAT type.
    fn assert_migrates_an_empty_volume(geometry: HddGeometry) {
        let original = neetan_formatted_sasi_in_cylinder_0(geometry);
        let new_start = new_start(&original);
        let mut image = original.clone();

        assert!(migrate_to_nec_layout(&mut image), "{geometry:?}");
        let table = image.read_sector(PARTITION_TABLE_LBA).unwrap();
        assert_eq!(&table[..2], &[0x21, 0x81]);
        assert_eq!(&table[4..8], &[0, 0, 1, 0]);
        assert_eq!(&table[8..12], &[0, 0, 1, 0]);
        let [cylinder_low, cylinder_high] = (geometry.cylinders - 1).to_le_bytes();
        assert_eq!(&table[12..16], &[0, 0, cylinder_low, cylinder_high]);
        assert_eq!(&table[16..32], PC98_PARTITION_NAME);

        let boot_record = image.read_sector(new_start).unwrap();
        let original_boot_record = original.read_sector(OLD_START).unwrap();
        assert_eq!(&boot_record[..19], &original_boot_record[..19]);
        assert_eq!(&boot_record[21..28], &original_boot_record[21..28]);
        assert_eq!(read_double_word(boot_record, 28), new_start);
        let total_sectors = match read_word(boot_record, 19) {
            0 => read_double_word(boot_record, 32),
            total => u32::from(total),
        };
        assert_eq!(total_sectors, geometry.total_sectors() - new_start);
        assert_eq!(&boot_record[36..], &original_boot_record[36..]);

        let layout = FatLayout::read(volume(&mut image, new_start)).unwrap();
        let original_layout = FatLayout::read(volume(&mut original.clone(), OLD_START)).unwrap();
        assert_eq!(layout.is_fat16, original_layout.is_fat16);
        assert!(layout.cluster_count < original_layout.cluster_count);
        assert!(directory_entries(&image, new_start, 0).is_empty());

        assert_idempotent(&mut image);
    }

    #[test]
    fn migrates_an_empty_volume_of_every_sasi_type() {
        let sizes = [
            HddSizeType::Mb5,
            HddSizeType::Mb10,
            HddSizeType::Mb15,
            HddSizeType::Mb20,
            HddSizeType::Mb30,
            HddSizeType::Mb40,
        ];
        for size in sizes {
            assert_migrates_an_empty_volume(geometry(size));
        }
    }

    #[test]
    fn migrates_an_empty_volume_of_a_non_standard_geometry() {
        assert_migrates_an_empty_volume(HddGeometry {
            cylinders: 320,
            heads: 4,
            sectors_per_track: 33,
            sector_size: 256,
        });
    }

    /// Asserts that files and a subdirectory chain in the cut-off tail of
    /// the volume move into free clusters and keep their content.
    fn assert_moves_files_out_of_the_tail(size: HddSizeType, is_fat16: bool) {
        let mut image = neetan_formatted_sasi_in_cylinder_0(geometry(size));
        let (new_end, old_end) = cluster_ends(&mut image);
        assert!(old_end - new_end >= 9);
        let tail_cluster = |index: u32| new_end + index;
        let keep = pattern(1, 2 * 2048 - 100);
        let tail = pattern(2, 4 * 2048);
        let inner = pattern(3, 3000);
        let last = pattern(4, 1500);
        write_chain(&mut image, &[2, 3], &keep);
        write_chain(
            &mut image,
            &[tail_cluster(0), 4, tail_cluster(1), tail_cluster(2)],
            &tail,
        );

        let mut sub = Vec::new();
        sub.extend(directory_entry(b".          ", 0x10, tail_cluster(3), 0));
        sub.extend(directory_entry(b"..         ", 0x10, 0, 0));
        sub.extend(directory_entry(b"INNER   TXT", 0x20, tail_cluster(4), 3000));
        sub.extend(directory_entry(b"DEEP       ", 0x10, tail_cluster(6), 0));
        write_chain(&mut image, &[tail_cluster(3)], &sub);
        write_chain(&mut image, &[tail_cluster(4), tail_cluster(5)], &inner);

        let mut deep = Vec::new();
        deep.extend(directory_entry(b".          ", 0x10, tail_cluster(6), 0));
        deep.extend(directory_entry(b"..         ", 0x10, tail_cluster(3), 0));
        for _ in 2..64 {
            deep.extend(directory_entry(b"\xE5ELETED BIN", 0x20, old_end - 1, 10));
        }
        deep.extend(directory_entry(b"LAST    BIN", 0x20, tail_cluster(8), 1500));
        write_chain(&mut image, &[tail_cluster(6), tail_cluster(7)], &deep);
        write_chain(&mut image, &[tail_cluster(8)], &last);

        let mut root = Vec::new();
        root.extend(directory_entry(b"NEETAN     ", 0x08, 0, 0));
        root.extend(directory_entry(b"KEEP    BIN", 0x20, 2, keep.len() as u32));
        root.extend(directory_entry(
            b"TAIL    BIN",
            0x20,
            tail_cluster(0),
            tail.len() as u32,
        ));
        root.extend(directory_entry(b"SUB        ", 0x10, tail_cluster(3), 0));
        write_root(&mut image, &root);

        assert!(migrate_to_nec_layout(&mut image));
        let start = new_start(&image);

        assert_eq!(read_file(&image, start, &[b"KEEP    BIN"]), keep);
        assert_eq!(read_file(&image, start, &[b"TAIL    BIN"]), tail);
        assert_eq!(
            read_file(&image, start, &[b"SUB        ", b"INNER   TXT"]),
            inner
        );
        assert_eq!(
            read_file(
                &image,
                start,
                &[b"SUB        ", b"DEEP       ", b"LAST    BIN"]
            ),
            last
        );

        let cluster_of = |path: &[&[u8; 11]]| {
            u32::from(read_word(
                &find_entry(&image, start, path),
                DIRECTORY_ENTRY_CLUSTER_OFFSET,
            ))
        };
        let sub_cluster = cluster_of(&[b"SUB        "]);
        let deep_cluster = cluster_of(&[b"SUB        ", b"DEEP       "]);
        assert!(sub_cluster < new_end && deep_cluster < new_end);
        assert_eq!(cluster_of(&[b"SUB        ", b".          "]), sub_cluster);
        assert_eq!(cluster_of(&[b"SUB        ", b"..         "]), 0);
        assert_eq!(
            cluster_of(&[b"SUB        ", b"DEEP       ", b".          "]),
            deep_cluster
        );
        assert_eq!(
            cluster_of(&[b"SUB        ", b"DEEP       ", b"..         "]),
            sub_cluster
        );

        let volume = &image.data[start as usize * 256..];
        let layout = FatLayout::read(volume).unwrap();
        assert_eq!(layout.is_fat16, is_fat16);
        assert_eq!(FIRST_CLUSTER + layout.cluster_count, new_end);
        assert_eq!(volume[layout.fat_range(0)], volume[layout.fat_range(1)]);

        assert_idempotent(&mut image);
    }

    #[test]
    fn moves_files_and_directories_out_of_the_tail_of_a_fat16_volume() {
        assert_moves_files_out_of_the_tail(HddSizeType::Mb40, true);
    }

    #[test]
    fn moves_files_and_directories_out_of_the_tail_of_a_fat12_volume() {
        assert_moves_files_out_of_the_tail(HddSizeType::Mb5, false);
    }

    #[test]
    fn leaves_a_volume_too_full_to_shrink_unchanged() {
        let mut image = neetan_formatted_sasi_in_cylinder_0(geometry(HddSizeType::Mb40));
        let (new_end, old_end) = cluster_ends(&mut image);
        let used: Vec<u32> = (FIRST_CLUSTER..new_end).chain([old_end - 1]).collect();
        write_chain(&mut image, &used, &[]);
        assert_unchanged(image);
    }

    #[test]
    fn leaves_a_directory_entry_into_a_free_tail_cluster_unchanged() {
        let mut image = neetan_formatted_sasi_in_cylinder_0(geometry(HddSizeType::Mb40));
        let (new_end, _) = cluster_ends(&mut image);
        write_root(
            &mut image,
            &directory_entry(b"BROKEN  BIN", 0x20, new_end, 10),
        );
        write_chain(&mut image, &[new_end + 1], &[]);
        assert_unchanged(image);
    }

    #[test]
    fn leaves_a_looping_directory_chain_unchanged() {
        let mut image = neetan_formatted_sasi_in_cylinder_0(geometry(HddSizeType::Mb40));
        let (new_end, _) = cluster_ends(&mut image);
        let mut sub = Vec::new();
        sub.extend(directory_entry(b".          ", 0x10, new_end, 0));
        sub.extend(directory_entry(b"..         ", 0x10, 0, 0));
        write_chain(&mut image, &[new_end, new_end + 1], &sub);
        let volume = volume(&mut image, OLD_START);
        let layout = FatLayout::read(volume).unwrap();
        set_fat_entries(volume, &layout, new_end + 1, new_end);
        write_root(
            &mut image,
            &directory_entry(b"SUB        ", 0x10, new_end, 0),
        );
        assert_unchanged(image);
    }

    #[test]
    fn leaves_a_cross_linked_directory_unchanged() {
        let mut image = neetan_formatted_sasi_in_cylinder_0(geometry(HddSizeType::Mb40));
        let (new_end, _) = cluster_ends(&mut image);
        let mut sub = Vec::new();
        sub.extend(directory_entry(b".          ", 0x10, new_end, 0));
        sub.extend(directory_entry(b"..         ", 0x10, 0, 0));
        write_chain(&mut image, &[new_end], &sub);
        let mut root = directory_entry(b"FIRST      ", 0x10, new_end, 0);
        root.extend(directory_entry(b"SECOND     ", 0x10, new_end, 0));
        write_root(&mut image, &root);
        assert_unchanged(image);
    }

    #[test]
    fn migrates_the_partition_start_of_a_nec_boot_record() {
        let mut image = neetan_formatted_sasi_in_cylinder_0(geometry(HddSizeType::Mb40));
        let mut boot_record = image.read_sector(OLD_START).unwrap().to_vec();
        boot_record[1] = 0x44;
        boot_record[0x3E..0x42].copy_from_slice(&OLD_START.to_le_bytes());
        image.write_sector(OLD_START, &boot_record);

        assert!(migrate_to_nec_layout(&mut image));
        let boot_record = image.read_sector(264).unwrap();
        assert_eq!(read_double_word(boot_record, 0x3E), 264);
    }

    #[test]
    fn leaves_an_image_of_another_tool_unchanged() {
        let mut image = neetan_formatted_sasi_in_cylinder_0(geometry(HddSizeType::Mb40));
        let mut table = image.read_sector(PARTITION_TABLE_LBA).unwrap().to_vec();
        table[16..32].copy_from_slice(b"MS-DOS 6.20     ");
        image.write_sector(PARTITION_TABLE_LBA, &table);
        assert_unchanged(image);
    }

    #[test]
    fn leaves_an_image_formatted_in_the_nec_layout_unchanged() {
        let mut image = crate::disk::blank_hdd_image(HddSizeType::Mb40);
        format_hdd_image(&mut image, PartitionTableType::Pc98).unwrap();
        assert_unchanged(image);
    }

    #[test]
    fn leaves_a_blank_image_unchanged() {
        assert_unchanged(crate::disk::blank_hdd_image(HddSizeType::Mb40));
    }

    #[test]
    fn leaves_a_volume_with_unequal_fat_copies_unchanged() {
        let mut image = neetan_formatted_sasi_in_cylinder_0(geometry(HddSizeType::Mb40));
        let (new_end, _) = cluster_ends(&mut image);
        write_chain(&mut image, &[new_end], &[]);
        let volume = volume(&mut image, OLD_START);
        let layout = FatLayout::read(volume).unwrap();
        let range = layout.fat_range(1);
        set_fat_entry(&mut volume[range], layout.is_fat16, new_end, 0);
        assert_unchanged(image);
    }
}
