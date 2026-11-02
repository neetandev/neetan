//! The FM Towns HLE OS ROM image.
//!
//! The image is a 512 KiB FAT12 volume with the layout of the real OS ROM:
//!
//! * `00000h` the boot record: `IPL4` and `RETF`, no BPB.
//! * `00200h` and `00600h` the two FAT copies (two sectors of 512 bytes each).
//! * `00A00h` the root directory with `70h` entries. The volume label
//!   `OS-ROM` comes first. An unused entry holds `00h` and `F6h` bytes.
//! * Cluster `n` starts at `1000h + 400h * n`. Cluster 2 starts at `1800h`.
//!   `IO.SYS` reads the kernel by address from cluster `4Eh` (`14800h`). The
//!   kernel file starts there whatever its directory position.
//! * The clusters in front of the kernel hold zeros. The clusters behind the
//!   last file hold `E5h` up to `3FFFFh`. `40000h`-`7FFFFh` holds `FFh`.

use std::fmt;

/// Size of the image in bytes.
pub const IMAGE_SIZE: usize = 0x80000;

/// Boot record at offset 0: `IPL4` followed by `RETF`.
pub const BOOT_RECORD: [u8; 5] = [b'I', b'P', b'L', b'4', 0xCB];

/// Offsets of the two FAT copies.
pub const FAT_OFFSETS: [usize; 2] = [0x0200, 0x0600];

/// Size of one FAT copy in bytes.
pub const FAT_SIZE: usize = 0x0400;

/// Offset of the root directory.
pub const ROOT_DIRECTORY_OFFSET: usize = 0x0A00;

/// Number of root directory entries.
pub const ROOT_ENTRY_COUNT: usize = 0x70;

/// Size of a directory entry.
pub const DIRECTORY_ENTRY_SIZE: usize = 32;

/// Offset of the virtual cluster 0. Cluster `n` starts at this offset plus
/// `n` clusters.
pub const CLUSTER_ORIGIN_OFFSET: usize = 0x1000;

/// Size of a cluster: two sectors of 512 bytes.
pub const CLUSTER_SIZE: usize = 0x0400;

/// First cluster of the data area.
pub const FIRST_DATA_CLUSTER: u16 = 2;

/// Cluster of the kernel file. `IO.SYS` reads the kernel from its offset.
pub const KERNEL_CLUSTER: u16 = 0x4E;

/// Number of kernel bytes that `IO.SYS` reads from the OS ROM.
pub const KERNEL_IMAGE_SIZE: usize = 0x7800;

/// Offset of the kernel bytes that `IO.SYS` reads.
pub const KERNEL_OFFSET: usize = cluster_offset(KERNEL_CLUSTER);

/// End of the FAT data area of the image. The real ROM keeps voice data
/// behind it.
pub const DATA_AREA_END: usize = 0x40000;

/// Last cluster that fits in front of `DATA_AREA_END`.
pub const LAST_CLUSTER: u16 = ((DATA_AREA_END - CLUSTER_ORIGIN_OFFSET) / CLUSTER_SIZE - 1) as u16;

/// Media descriptor of the OS ROM drive.
pub const MEDIA_DESCRIPTOR: u8 = 0xFB;

/// FAT12 entry of the last cluster of a chain.
pub const END_OF_CHAIN: u16 = 0x0FFF;

/// Fill byte of the free clusters behind the last file.
pub const FREE_CLUSTER_FILL: u8 = 0xE5;

/// Fill byte of the area behind `DATA_AREA_END`.
pub const UNUSED_AREA_FILL: u8 = 0xFF;

/// Fill byte of an unused directory entry behind its first byte.
pub const UNUSED_ENTRY_FILL: u8 = 0xF6;

/// Volume label of the image.
pub const VOLUME_LABEL: [u8; 11] = *b"OS-ROM     ";

/// Attribute of the volume label entry: volume label and archive.
pub const ATTRIBUTE_VOLUME_LABEL: u8 = 0x28;

/// Attribute of a file entry: archive.
pub const ATTRIBUTE_ARCHIVE: u8 = 0x20;

/// Modification date of every entry: 2026-01-01.
pub const ENTRY_DATE: u16 = ((2026 - 1980) << 9) | (1 << 5) | 1;

/// Modification time of every entry: 00:00:00.
pub const ENTRY_TIME: u16 = 0;

/// Directory name of the kernel file.
pub const MSDOS_SYS_NAME: [u8; 11] = *b"MSDOS   SYS";

/// Directory name of the shell.
pub const COMMAND_COM_NAME: [u8; 11] = *b"COMMAND COM";

/// Directory name of the MSCDEX program.
pub const MSCDEX_EXE_NAME: [u8; 11] = *b"MSCDEX  EXE";

/// Offset of the attribute byte in a directory entry.
const ENTRY_ATTRIBUTE: usize = 0x0B;

/// Offset of the time word in a directory entry.
const ENTRY_TIME_FIELD: usize = 0x16;

/// Offset of the date word in a directory entry.
const ENTRY_DATE_FIELD: usize = 0x18;

/// Offset of the first cluster word in a directory entry.
const ENTRY_FIRST_CLUSTER: usize = 0x1A;

/// Offset of the size dword in a directory entry.
const ENTRY_SIZE: usize = 0x1C;

/// Returns the image offset of cluster `cluster`.
pub const fn cluster_offset(cluster: u16) -> usize {
    CLUSTER_ORIGIN_OFFSET + cluster as usize * CLUSTER_SIZE
}

/// A file of the image.
#[derive(Clone, Copy, Debug)]
pub struct OsRomFile<'a> {
    /// Name and extension in directory form, blank padded.
    pub name: [u8; 11],
    /// File content.
    pub data: &'a [u8],
    /// Fixed first cluster. `None` places the file behind the clusters in use.
    pub first_cluster: Option<u16>,
}

/// Error of the image builder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildError {
    /// The kernel file exceeds the bytes that `IO.SYS` reads.
    KernelTooLarge(usize),
    /// More files than root directory entries.
    TooManyFiles(usize),
    /// A fixed file overlaps clusters of another file.
    ClusterInUse(u16),
    /// The files exceed the data area.
    DataAreaFull,
}

impl fmt::Display for BuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::KernelTooLarge(size) => write!(
                formatter,
                "MSDOS.SYS has {size} bytes, the limit is {KERNEL_IMAGE_SIZE}"
            ),
            Self::TooManyFiles(count) => write!(
                formatter,
                "{count} files do not fit the {ROOT_ENTRY_COUNT} root directory entries"
            ),
            Self::ClusterInUse(cluster) => {
                write!(formatter, "cluster {cluster:#X} is used by two files")
            }
            Self::DataAreaFull => write!(formatter, "the files exceed the data area"),
        }
    }
}

impl std::error::Error for BuildError {}

/// Returns the three files of the OS ROM in directory order: `MSDOS.SYS` at
/// the kernel cluster, `COMMAND.COM` and `MSCDEX.EXE`.
pub fn standard_files<'a>(
    msdos_sys: &'a [u8],
    command_com: &'a [u8],
    mscdex_exe: &'a [u8],
) -> [OsRomFile<'a>; 3] {
    [
        OsRomFile {
            name: MSDOS_SYS_NAME,
            data: msdos_sys,
            first_cluster: Some(KERNEL_CLUSTER),
        },
        OsRomFile {
            name: COMMAND_COM_NAME,
            data: command_com,
            first_cluster: None,
        },
        OsRomFile {
            name: MSCDEX_EXE_NAME,
            data: mscdex_exe,
            first_cluster: None,
        },
    ]
}

/// Returns the number of clusters of `size` bytes. An empty file has none.
fn cluster_count(size: usize) -> usize {
    size.div_ceil(CLUSTER_SIZE)
}

/// Writes the FAT12 entry of `cluster`.
fn write_fat_entry(fat: &mut [u8], cluster: u16, value: u16) {
    let offset = usize::from(cluster) * 3 / 2;
    if cluster.is_multiple_of(2) {
        fat[offset] = value as u8;
        fat[offset + 1] = (fat[offset + 1] & 0xF0) | ((value >> 8) as u8 & 0x0F);
    } else {
        fat[offset] = (fat[offset] & 0x0F) | ((value << 4) as u8);
        fat[offset + 1] = (value >> 4) as u8;
    }
}

/// Assigns the first cluster of every file: the fixed clusters first, then
/// the other files in directory order behind the highest cluster in use.
fn allocate_clusters(files: &[OsRomFile]) -> Result<Vec<Option<u16>>, BuildError> {
    let mut used = vec![false; usize::from(LAST_CLUSTER) + 1];
    let mut first_clusters = vec![None; files.len()];
    let mut claim = |first: u16, count: usize| -> Result<(), BuildError> {
        let end = usize::from(first) + count;
        if first < FIRST_DATA_CLUSTER || end > used.len() {
            return Err(BuildError::DataAreaFull);
        }
        let range = &mut used[usize::from(first)..end];
        if let Some(position) = range.iter().position(|&in_use| in_use) {
            return Err(BuildError::ClusterInUse(first + position as u16));
        }
        range.fill(true);
        Ok(())
    };

    let mut next_free = FIRST_DATA_CLUSTER;
    for (index, file) in files.iter().enumerate() {
        let count = cluster_count(file.data.len());
        if let Some(first) = file.first_cluster
            && count > 0
        {
            claim(first, count)?;
            first_clusters[index] = Some(first);
            next_free = next_free.max(first + count as u16);
        }
    }
    for (index, file) in files.iter().enumerate() {
        let count = cluster_count(file.data.len());
        if file.first_cluster.is_some() || count == 0 {
            continue;
        }
        claim(next_free, count)?;
        first_clusters[index] = Some(next_free);
        next_free += count as u16;
    }
    Ok(first_clusters)
}

/// Returns the image range of root directory entry `index`.
fn directory_entry_range(index: usize) -> std::ops::Range<usize> {
    let start = ROOT_DIRECTORY_OFFSET + index * DIRECTORY_ENTRY_SIZE;
    start..start + DIRECTORY_ENTRY_SIZE
}

/// Writes a directory entry.
fn write_directory_entry(
    entry: &mut [u8],
    name: &[u8; 11],
    attribute: u8,
    first_cluster: u16,
    size: u32,
) {
    entry.fill(0);
    entry[..11].copy_from_slice(name);
    entry[ENTRY_ATTRIBUTE] = attribute;
    entry[ENTRY_TIME_FIELD..ENTRY_TIME_FIELD + 2].copy_from_slice(&ENTRY_TIME.to_le_bytes());
    entry[ENTRY_DATE_FIELD..ENTRY_DATE_FIELD + 2].copy_from_slice(&ENTRY_DATE.to_le_bytes());
    entry[ENTRY_FIRST_CLUSTER..ENTRY_FIRST_CLUSTER + 2]
        .copy_from_slice(&first_cluster.to_le_bytes());
    entry[ENTRY_SIZE..ENTRY_SIZE + 4].copy_from_slice(&size.to_le_bytes());
}

/// Builds the OS ROM image with `files` in directory order behind the volume
/// label. The kernel file (`MSDOS.SYS`) must fit the bytes that `IO.SYS`
/// reads.
pub fn build_os_rom(files: &[OsRomFile]) -> Result<Vec<u8>, BuildError> {
    if files.len() + 1 > ROOT_ENTRY_COUNT {
        return Err(BuildError::TooManyFiles(files.len()));
    }
    if let Some(kernel) = files.iter().find(|file| file.name == MSDOS_SYS_NAME)
        && kernel.data.len() > KERNEL_IMAGE_SIZE
    {
        return Err(BuildError::KernelTooLarge(kernel.data.len()));
    }
    let first_clusters = allocate_clusters(files)?;

    let mut image = vec![UNUSED_AREA_FILL; IMAGE_SIZE];
    image[..DATA_AREA_END].fill(0);
    image[..BOOT_RECORD.len()].copy_from_slice(&BOOT_RECORD);

    let mut fat = vec![0u8; FAT_SIZE];
    write_fat_entry(&mut fat, 0, 0x0F00 | u16::from(MEDIA_DESCRIPTOR));
    write_fat_entry(&mut fat, 1, END_OF_CHAIN);

    let directory_end = ROOT_DIRECTORY_OFFSET + ROOT_ENTRY_COUNT * DIRECTORY_ENTRY_SIZE;
    for entry in image[ROOT_DIRECTORY_OFFSET..directory_end]
        .as_chunks_mut::<DIRECTORY_ENTRY_SIZE>()
        .0
    {
        entry[0] = 0;
        entry[1..].fill(UNUSED_ENTRY_FILL);
    }
    write_directory_entry(
        &mut image[directory_entry_range(0)],
        &VOLUME_LABEL,
        ATTRIBUTE_VOLUME_LABEL,
        0,
        0,
    );
    let mut free_area_start = cluster_offset(FIRST_DATA_CLUSTER);
    for (index, (file, first_cluster)) in files.iter().zip(&first_clusters).enumerate() {
        write_directory_entry(
            &mut image[directory_entry_range(index + 1)],
            &file.name,
            ATTRIBUTE_ARCHIVE,
            first_cluster.unwrap_or(0),
            file.data.len() as u32,
        );
        let Some(first) = first_cluster else {
            continue;
        };
        let count = cluster_count(file.data.len()) as u16;
        for cluster in *first..*first + count {
            let next = if cluster + 1 == *first + count {
                END_OF_CHAIN
            } else {
                cluster + 1
            };
            write_fat_entry(&mut fat, cluster, next);
        }
        let start = cluster_offset(*first);
        image[start..start + file.data.len()].copy_from_slice(file.data);
        free_area_start = free_area_start.max(cluster_offset(*first + count));
    }
    image[free_area_start..DATA_AREA_END].fill(FREE_CLUSTER_FILL);
    for offset in FAT_OFFSETS {
        image[offset..offset + FAT_SIZE].copy_from_slice(&fat);
    }
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The committed image of `make`.
    const COMMITTED_IMAGE: &[u8] = include_bytes!("../osrom.rom");

    /// Returns the FAT12 entry of `cluster` from the first FAT.
    fn fat_entry(image: &[u8], cluster: u16) -> u16 {
        let fat = &image[FAT_OFFSETS[0]..FAT_OFFSETS[0] + FAT_SIZE];
        let offset = usize::from(cluster) * 3 / 2;
        let pair = u16::from_le_bytes([fat[offset], fat[offset + 1]]);
        if cluster.is_multiple_of(2) {
            pair & 0x0FFF
        } else {
            pair >> 4
        }
    }

    /// Returns the root directory entry of `name`.
    fn directory_entry<'a>(image: &'a [u8], name: &[u8; 11]) -> Option<&'a [u8]> {
        image
            [ROOT_DIRECTORY_OFFSET..ROOT_DIRECTORY_OFFSET + ROOT_ENTRY_COUNT * DIRECTORY_ENTRY_SIZE]
            .as_chunks::<DIRECTORY_ENTRY_SIZE>()
            .0
            .iter()
            .find(|entry| &entry[..11] == name)
            .map(|entry| entry.as_slice())
    }

    /// Returns the first cluster and the size of the file `name`.
    fn file_location(image: &[u8], name: &[u8; 11]) -> (u16, usize) {
        let entry = directory_entry(image, name).expect("file in the root directory");
        let first =
            u16::from_le_bytes([entry[ENTRY_FIRST_CLUSTER], entry[ENTRY_FIRST_CLUSTER + 1]]);
        let size = u32::from_le_bytes(entry[ENTRY_SIZE..ENTRY_SIZE + 4].try_into().unwrap());
        (first, size as usize)
    }

    /// Returns the cluster chain of `first` from the FAT.
    fn cluster_chain(image: &[u8], first: u16) -> Vec<u16> {
        let mut chain = vec![first];
        let mut cluster = first;
        loop {
            let next = fat_entry(image, cluster);
            if next >= 0x0FF8 {
                return chain;
            }
            assert!(chain.len() < 0x200, "the chain loops");
            chain.push(next);
            cluster = next;
        }
    }

    /// Reads the file `name` by its FAT chain.
    fn read_file(image: &[u8], name: &[u8; 11]) -> Vec<u8> {
        let (first, size) = file_location(image, name);
        let mut data = Vec::new();
        if size > 0 {
            for cluster in cluster_chain(image, first) {
                let start = cluster_offset(cluster);
                data.extend_from_slice(&image[start..start + CLUSTER_SIZE]);
            }
        }
        data.truncate(size);
        data
    }

    /// Returns a test image with a kernel of `kernel_size` bytes.
    fn test_image(kernel_size: usize) -> Vec<u8> {
        let kernel = vec![0x4B; kernel_size];
        build_os_rom(&standard_files(&kernel, &[0xC3; 25], &[0xEE; 0x0500])).unwrap()
    }

    #[test]
    fn kernel_offset_matches_the_load_address_of_io_sys() {
        assert_eq!(KERNEL_OFFSET, 0x14800);
        assert_eq!(cluster_offset(FIRST_DATA_CLUSTER), 0x1800);
        assert_eq!(LAST_CLUSTER, 0xFB);
    }

    #[test]
    fn boot_record_holds_the_ipl_signature_and_no_bpb() {
        let image = test_image(0x100);
        assert_eq!(image.len(), IMAGE_SIZE);
        assert_eq!(&image[..5], b"IPL4\xCB");
        assert!(image[5..FAT_OFFSETS[0]].iter().all(|&byte| byte == 0));
    }

    #[test]
    fn both_fats_start_with_the_media_descriptor_and_agree() {
        let image = test_image(0x100);
        assert_eq!(&image[0x200..0x203], &[0xFB, 0xFF, 0xFF]);
        assert_eq!(image[0x200..0x600], image[0x600..0xA00]);
    }

    #[test]
    fn volume_label_comes_first() {
        let image = test_image(0x100);
        let entry = &image[ROOT_DIRECTORY_OFFSET..ROOT_DIRECTORY_OFFSET + DIRECTORY_ENTRY_SIZE];
        assert_eq!(&entry[..11], b"OS-ROM     ");
        assert_eq!(entry[ENTRY_ATTRIBUTE], 0x28);
        assert_eq!(&entry[ENTRY_FIRST_CLUSTER..], &[0; 6]);
    }

    #[test]
    fn files_follow_the_label_in_directory_order() {
        let image = test_image(0x100);
        let names: Vec<&[u8]> = image[ROOT_DIRECTORY_OFFSET + DIRECTORY_ENTRY_SIZE..]
            .as_chunks::<DIRECTORY_ENTRY_SIZE>()
            .0
            .iter()
            .take(3)
            .map(|entry| &entry[..11])
            .collect();
        assert_eq!(names, [b"MSDOS   SYS", b"COMMAND COM", b"MSCDEX  EXE"]);
        let entry = directory_entry(&image, &COMMAND_COM_NAME).unwrap();
        assert_eq!(entry[ENTRY_ATTRIBUTE], 0x20);
        assert_eq!(
            &entry[ENTRY_DATE_FIELD..ENTRY_DATE_FIELD + 2],
            &[0x21, 0x5C]
        );
    }

    #[test]
    fn unused_directory_entries_hold_zero_and_f6h() {
        let image = test_image(0x100);
        let entry_offset = ROOT_DIRECTORY_OFFSET + 4 * DIRECTORY_ENTRY_SIZE;
        let entry = &image[entry_offset..entry_offset + DIRECTORY_ENTRY_SIZE];
        assert_eq!(entry[0], 0);
        assert!(entry[1..].iter().all(|&byte| byte == 0xF6));
        let last = ROOT_DIRECTORY_OFFSET + ROOT_ENTRY_COUNT * DIRECTORY_ENTRY_SIZE;
        assert_eq!(last, cluster_offset(FIRST_DATA_CLUSTER));
        assert_eq!(image[last - 1], 0xF6);
    }

    #[test]
    fn kernel_starts_at_cluster_4eh_and_the_others_follow() {
        let image = test_image(0x7000);
        assert_eq!(file_location(&image, &MSDOS_SYS_NAME), (0x4E, 0x7000));
        assert_eq!(file_location(&image, &COMMAND_COM_NAME), (0x6A, 25));
        assert_eq!(file_location(&image, &MSCDEX_EXE_NAME), (0x6B, 0x0500));
        assert_eq!(image[KERNEL_OFFSET], 0x4B);
        assert_eq!(image[cluster_offset(0x6A)], 0xC3);
    }

    #[test]
    fn kernel_keeps_cluster_4eh_behind_other_files_in_the_directory() {
        let kernel = [0x4B; 0x0800];
        let files = [
            OsRomFile {
                name: COMMAND_COM_NAME,
                data: &[0xC3; 0x0900],
                first_cluster: None,
            },
            OsRomFile {
                name: MSDOS_SYS_NAME,
                data: &kernel,
                first_cluster: Some(KERNEL_CLUSTER),
            },
        ];
        let image = build_os_rom(&files).unwrap();
        assert_eq!(file_location(&image, &MSDOS_SYS_NAME).0, 0x4E);
        assert_eq!(file_location(&image, &COMMAND_COM_NAME).0, 0x50);
        assert_eq!(&image[KERNEL_OFFSET..KERNEL_OFFSET + 0x0800], &kernel);
    }

    #[test]
    fn fat_chains_link_the_clusters_of_each_file() {
        let image = test_image(0x7800);
        let kernel_chain = cluster_chain(&image, 0x4E);
        assert_eq!(kernel_chain, (0x4E..=0x6B).collect::<Vec<_>>());
        assert_eq!(cluster_chain(&image, 0x6C), [0x6C]);
        assert_eq!(cluster_chain(&image, 0x6D), [0x6D, 0x6E]);
        assert_eq!(fat_entry(&image, 0x4D), 0);
        assert_eq!(fat_entry(&image, 0x6F), 0);
        assert_eq!(read_file(&image, &MSCDEX_EXE_NAME), vec![0xEE; 0x0500]);
    }

    #[test]
    fn fill_bytes_follow_the_real_layout() {
        let image = test_image(0x7800);
        assert!(
            image[cluster_offset(2)..KERNEL_OFFSET]
                .iter()
                .all(|&byte| byte == 0)
        );
        let mscdex_end = cluster_offset(0x6D) + 0x0500;
        assert!(
            image[mscdex_end..cluster_offset(0x6F)]
                .iter()
                .all(|&byte| byte == 0)
        );
        assert!(
            image[cluster_offset(0x6F)..DATA_AREA_END]
                .iter()
                .all(|&byte| byte == 0xE5)
        );
        assert!(image[DATA_AREA_END..].iter().all(|&byte| byte == 0xFF));
    }

    #[test]
    fn oversized_kernel_is_rejected() {
        let kernel = vec![0; KERNEL_IMAGE_SIZE + 1];
        assert_eq!(
            build_os_rom(&standard_files(&kernel, &[], &[])).unwrap_err(),
            BuildError::KernelTooLarge(KERNEL_IMAGE_SIZE + 1)
        );
    }

    #[test]
    fn overlapping_fixed_files_are_rejected() {
        let files = [
            OsRomFile {
                name: MSDOS_SYS_NAME,
                data: &[0; 0x0800],
                first_cluster: Some(KERNEL_CLUSTER),
            },
            OsRomFile {
                name: COMMAND_COM_NAME,
                data: &[0; 0x10],
                first_cluster: Some(KERNEL_CLUSTER + 1),
            },
        ];
        assert_eq!(
            build_os_rom(&files).unwrap_err(),
            BuildError::ClusterInUse(KERNEL_CLUSTER + 1)
        );
    }

    #[test]
    fn files_beyond_the_data_area_are_rejected() {
        let large = vec![0; DATA_AREA_END];
        assert_eq!(
            build_os_rom(&standard_files(&[], &large, &[])).unwrap_err(),
            BuildError::DataAreaFull
        );
    }

    #[test]
    fn committed_image_equals_a_build_of_its_files() {
        let msdos_sys = read_file(COMMITTED_IMAGE, &MSDOS_SYS_NAME);
        let command_com = read_file(COMMITTED_IMAGE, &COMMAND_COM_NAME);
        let mscdex_exe = read_file(COMMITTED_IMAGE, &MSCDEX_EXE_NAME);
        assert_eq!(msdos_sys.len(), KERNEL_IMAGE_SIZE);
        assert_eq!(
            command_com,
            include_bytes!("../../dos/dos.rom"),
            "COMMAND.COM is the shell stub of utils/dos"
        );
        let rebuilt = build_os_rom(&standard_files(&msdos_sys, &command_com, &mscdex_exe));
        assert!(rebuilt.unwrap() == COMMITTED_IMAGE);
    }

    #[test]
    fn committed_kernel_carries_the_entry_and_the_ctl_sys_pattern() {
        let kernel = &COMMITTED_IMAGE[KERNEL_OFFSET..KERNEL_OFFSET + KERNEL_IMAGE_SIZE];
        assert_eq!(kernel[0], 0xE9);
        let pattern = 0x11D2;
        assert_eq!(
            &kernel[pattern..pattern + 8],
            &[0x8C, 0xCB, 0x8E, 0xDB, 0xFE, 0x06, 0xCF, 0x02]
        );
        let operand = |distance: usize| {
            u16::from_le_bytes([kernel[pattern + distance], kernel[pattern + distance + 1]])
        };
        assert_eq!(
            [operand(0x14), operand(0x17), operand(0x1A), operand(0x1D)],
            [0x051E, 0x0584, 0x0520, 0x0582]
        );
        let reserved = [
            0x0A10..0x0B80,
            0x4250..0x4270,
            0x4300..0x4B00,
            0x6AA0..0x7800,
        ];
        for range in reserved {
            assert!(kernel[range].iter().all(|&byte| byte == 0));
        }
    }
}
