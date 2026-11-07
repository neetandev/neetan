//! End-to-end tests for the `neetan copy` subcommand.

use std::path::{Path, PathBuf};

use common::temporary::TemporaryFolder;
use device::{
    disk::{
        HddFormat, HddGeometry, HddImage, HddSizeType, blank_hdd_image,
        format::{
            PartitionTableType, fat_region, format_hdd_image, hdd_boot_sector, hdd_bpb_params,
        },
    },
    floppy::{
        FloppyFormat, FloppyGeometry, FloppyImage,
        d88::{D88Disk, D88MediaType, D88Sector},
        fdi, load_floppy_image,
    },
};
use user_interface::{config::CopyArg, copy::copy};

const FLOPPY_CYLINDERS: usize = 77;
const FLOPPY_HEADS: usize = 2;
const FLOPPY_SECTORS_PER_TRACK: usize = 8;
const FLOPPY_SECTOR_SIZE: usize = 1024;
const FLOPPY_RESERVED_SECTORS: usize = 1;
const FLOPPY_FAT_COUNT: usize = 2;
const FLOPPY_SECTORS_PER_FAT: usize = 2;
const FLOPPY_ROOT_ENTRY_COUNT: usize = 192;
const FLOPPY_ROOT_DIRECTORY_SECTORS: usize =
    (FLOPPY_ROOT_ENTRY_COUNT * 32).div_ceil(FLOPPY_SECTOR_SIZE);
const FLOPPY_TOTAL_SECTORS: usize = FLOPPY_CYLINDERS * FLOPPY_HEADS * FLOPPY_SECTORS_PER_TRACK;

const HDD_CYLINDERS: u16 = 80;
const HDD_HEADS: u8 = 8;
const HDD_SECTORS_PER_TRACK: u8 = 17;
const HDD_SECTOR_SIZE: u16 = 512;

/// Builds a blank FAT12-formatted 2HD floppy image (1232 KiB, 77x2x8x1024).
/// Returns the raw bytes suitable for writing as a `.hdm` file.
fn build_blank_fat12_floppy() -> Vec<u8> {
    let sectors_per_cluster: u8 = 1;

    let mut image = vec![0u8; FLOPPY_TOTAL_SECTORS * FLOPPY_SECTOR_SIZE];

    image[11..13].copy_from_slice(&(FLOPPY_SECTOR_SIZE as u16).to_le_bytes());
    image[13] = sectors_per_cluster;
    image[14..16].copy_from_slice(&(FLOPPY_RESERVED_SECTORS as u16).to_le_bytes());
    image[16] = FLOPPY_FAT_COUNT as u8;
    image[17..19].copy_from_slice(&(FLOPPY_ROOT_ENTRY_COUNT as u16).to_le_bytes());
    image[19..21].copy_from_slice(&(FLOPPY_TOTAL_SECTORS as u16).to_le_bytes());
    image[21] = 0xF0;
    image[22..24].copy_from_slice(&(FLOPPY_SECTORS_PER_FAT as u16).to_le_bytes());
    image[24..26].copy_from_slice(&(FLOPPY_SECTORS_PER_TRACK as u16).to_le_bytes());
    image[26..28].copy_from_slice(&(FLOPPY_HEADS as u16).to_le_bytes());

    let fat1_offset = FLOPPY_RESERVED_SECTORS * FLOPPY_SECTOR_SIZE;
    image[fat1_offset] = 0xF0;
    image[fat1_offset + 1] = 0xFF;
    image[fat1_offset + 2] = 0xFF;
    let fat2_offset = fat1_offset + FLOPPY_SECTORS_PER_FAT * FLOPPY_SECTOR_SIZE;
    image[fat2_offset] = 0xF0;
    image[fat2_offset + 1] = 0xFF;
    image[fat2_offset + 2] = 0xFF;

    image
}

fn set_fat12_entry(fat: &mut [u8], cluster: u16, value: u16) {
    let value = value & 0x0FFF;
    let offset = (cluster as usize * 3) / 2;
    if cluster & 1 == 0 {
        fat[offset] = (value & 0x00FF) as u8;
        fat[offset + 1] = (fat[offset + 1] & 0xF0) | ((value >> 8) as u8 & 0x0F);
    } else {
        fat[offset] = (fat[offset] & 0x0F) | (((value << 4) as u8) & 0xF0);
        fat[offset + 1] = (value >> 4) as u8;
    }
}

fn mark_floppy_data_clusters_used(image: &mut [u8]) {
    let data_cluster_count = FLOPPY_TOTAL_SECTORS
        - FLOPPY_RESERVED_SECTORS
        - FLOPPY_FAT_COUNT * FLOPPY_SECTORS_PER_FAT
        - FLOPPY_ROOT_DIRECTORY_SECTORS;
    for fat_index in 0..FLOPPY_FAT_COUNT {
        let fat_offset =
            (FLOPPY_RESERVED_SECTORS + fat_index * FLOPPY_SECTORS_PER_FAT) * FLOPPY_SECTOR_SIZE;
        let fat = &mut image[fat_offset..fat_offset + FLOPPY_SECTORS_PER_FAT * FLOPPY_SECTOR_SIZE];
        for cluster in 2..data_cluster_count as u16 + 2 {
            set_fat12_entry(fat, cluster, 0x0FFF);
        }
    }
}

fn build_d88_tracks(disk_data: &[u8]) -> Vec<Option<Vec<D88Sector>>> {
    let mut tracks = Vec::with_capacity(FLOPPY_CYLINDERS * FLOPPY_HEADS);
    for track_index in 0..FLOPPY_CYLINDERS * FLOPPY_HEADS {
        let cylinder = (track_index / FLOPPY_HEADS) as u8;
        let head = (track_index % FLOPPY_HEADS) as u8;
        let mut sectors = Vec::with_capacity(FLOPPY_SECTORS_PER_TRACK);
        for sector_index in 0..FLOPPY_SECTORS_PER_TRACK {
            let linear_sector = track_index * FLOPPY_SECTORS_PER_TRACK + sector_index;
            let data_offset = linear_sector * FLOPPY_SECTOR_SIZE;
            sectors.push(D88Sector {
                cylinder,
                head,
                record: (sector_index + 1) as u8,
                size_code: 3,
                sector_count: FLOPPY_SECTORS_PER_TRACK as u16,
                mfm_flag: 0,
                deleted: 0,
                status: 0,
                reserved: [0; 5],
                data: disk_data[data_offset..data_offset + FLOPPY_SECTOR_SIZE].to_vec(),
                source_offset: None,
            });
        }
        tracks.push(Some(sectors));
    }
    tracks
}

fn build_blank_d88_floppy() -> Vec<u8> {
    let disk_data = build_blank_fat12_floppy();
    let disk = D88Disk::from_tracks(
        "COPYTEST".to_string(),
        false,
        D88MediaType::Disk2HD,
        build_d88_tracks(&disk_data),
    );
    FloppyImage::from_d88(disk).to_bytes()
}

fn write_pc98_partition_table(image_data: &mut [u8]) -> u32 {
    image_data[4..8].copy_from_slice(b"IPL1");
    let partition_table_offset = HDD_SECTOR_SIZE as usize;
    let partition = &mut image_data[partition_table_offset..partition_table_offset + 32];
    partition[0] = 0xA0;
    partition[1] = 0x91;
    partition[8] = 0;
    partition[9] = 0;
    partition[10] = 1;
    partition[11] = 0;
    partition[12] = HDD_SECTORS_PER_TRACK - 1;
    partition[13] = HDD_HEADS - 1;
    partition[14] = (HDD_CYLINDERS - 1) as u8;
    partition[15] = ((HDD_CYLINDERS - 1) >> 8) as u8;
    partition[16..32].copy_from_slice(b"MS-DOS 6.20\x00\x00\x00\x00\x00");

    HDD_HEADS as u32 * HDD_SECTORS_PER_TRACK as u32
}

fn build_blank_partitioned_nhd() -> Vec<u8> {
    let geometry = HddGeometry {
        cylinders: HDD_CYLINDERS,
        heads: HDD_HEADS,
        sectors_per_track: HDD_SECTORS_PER_TRACK,
        sector_size: HDD_SECTOR_SIZE,
    };
    let sector_size = HDD_SECTOR_SIZE as usize;
    let mut image_data = vec![0u8; geometry.total_sectors() as usize * sector_size];
    let partition_lba = write_pc98_partition_table(&mut image_data);
    let partition_byte_offset = partition_lba as usize * sector_size;
    let reserved_sectors = 1u16;
    let fat_count = 2u8;
    let sectors_per_fat = 64u16;
    let root_entry_count = 512u16;
    let partition_sectors = geometry.total_sectors() - partition_lba;

    let boot_sector = &mut image_data[partition_byte_offset..partition_byte_offset + sector_size];
    boot_sector[0] = 0xEB;
    boot_sector[1] = 0x3C;
    boot_sector[2] = 0x90;
    boot_sector[3..11].copy_from_slice(b"NEETAN  ");
    boot_sector[11..13].copy_from_slice(&HDD_SECTOR_SIZE.to_le_bytes());
    boot_sector[13] = 1;
    boot_sector[14..16].copy_from_slice(&reserved_sectors.to_le_bytes());
    boot_sector[16] = fat_count;
    boot_sector[17..19].copy_from_slice(&root_entry_count.to_le_bytes());
    boot_sector[19..21].copy_from_slice(&(partition_sectors as u16).to_le_bytes());
    boot_sector[21] = 0xF8;
    boot_sector[22..24].copy_from_slice(&sectors_per_fat.to_le_bytes());
    boot_sector[24..26].copy_from_slice(&(HDD_SECTORS_PER_TRACK as u16).to_le_bytes());
    boot_sector[26..28].copy_from_slice(&(HDD_HEADS as u16).to_le_bytes());

    let first_fat_offset = partition_byte_offset + reserved_sectors as usize * sector_size;
    {
        let first_fat = &mut image_data
            [first_fat_offset..first_fat_offset + sectors_per_fat as usize * sector_size];
        first_fat[0] = 0xF8;
        first_fat[1] = 0xFF;
        first_fat[2] = 0xFF;
        first_fat[3] = 0xFF;
    }
    let fat_copy = image_data
        [first_fat_offset..first_fat_offset + sectors_per_fat as usize * sector_size]
        .to_vec();
    let second_fat_offset = first_fat_offset + sectors_per_fat as usize * sector_size;
    image_data[second_fat_offset..second_fat_offset + fat_copy.len()].copy_from_slice(&fat_copy);

    HddImage::from_raw(geometry, HddFormat::Nhd, image_data).to_bytes()
}

/// Cylinder count of the AT flat test image (about 4 MB).
const AT_HDD_CYLINDERS: u32 = 8;
/// Heads of the AT flat geometry.
const AT_HDD_HEADS: u32 = 16;
/// Sectors per track of the AT flat geometry.
const AT_HDD_SECTORS_PER_TRACK: u32 = 63;
/// Partition start LBA (one track reserved for the MBR).
const AT_PARTITION_START_LBA: u32 = 63;

/// Builds a flat AT `.hdd` image with an MBR whose single active FAT16
/// partition starts at a nonzero LBA and holds a blank FAT16 filesystem.
fn build_blank_partitioned_at_hdd() -> Vec<u8> {
    let total_sectors = AT_HDD_CYLINDERS * AT_HDD_HEADS * AT_HDD_SECTORS_PER_TRACK;
    let sector_size = 512usize;
    let mut image_data = vec![0u8; total_sectors as usize * sector_size];

    // Master boot record: signature plus one active FAT16B entry.
    let partition_sectors = total_sectors - AT_PARTITION_START_LBA;
    image_data[510] = 0x55;
    image_data[511] = 0xAA;
    let entry = &mut image_data[0x1BE..0x1BE + 16];
    entry[0] = 0x80;
    entry[4] = 0x06;
    entry[8..12].copy_from_slice(&AT_PARTITION_START_LBA.to_le_bytes());
    entry[12..16].copy_from_slice(&partition_sectors.to_le_bytes());

    // FAT16 boot sector at the partition start.
    let reserved_sectors = 1u16;
    let fat_count = 2u8;
    let sectors_per_fat = 32u16;
    let root_entry_count = 512u16;
    let partition_byte_offset = AT_PARTITION_START_LBA as usize * sector_size;
    let boot_sector = &mut image_data[partition_byte_offset..partition_byte_offset + sector_size];
    boot_sector[0] = 0xEB;
    boot_sector[1] = 0x3C;
    boot_sector[2] = 0x90;
    boot_sector[3..11].copy_from_slice(b"NEETAN  ");
    boot_sector[11..13].copy_from_slice(&(sector_size as u16).to_le_bytes());
    boot_sector[13] = 1;
    boot_sector[14..16].copy_from_slice(&reserved_sectors.to_le_bytes());
    boot_sector[16] = fat_count;
    boot_sector[17..19].copy_from_slice(&root_entry_count.to_le_bytes());
    boot_sector[19..21].copy_from_slice(&(partition_sectors as u16).to_le_bytes());
    boot_sector[21] = 0xF8;
    boot_sector[22..24].copy_from_slice(&sectors_per_fat.to_le_bytes());
    boot_sector[24..26].copy_from_slice(&(AT_HDD_SECTORS_PER_TRACK as u16).to_le_bytes());
    boot_sector[26..28].copy_from_slice(&(AT_HDD_HEADS as u16).to_le_bytes());
    boot_sector[510] = 0x55;
    boot_sector[511] = 0xAA;

    // Both FAT copies with the FAT16 media/EOC head entries.
    let first_fat_offset = partition_byte_offset + reserved_sectors as usize * sector_size;
    for fat_index in 0..fat_count as usize {
        let fat_offset = first_fat_offset + fat_index * sectors_per_fat as usize * sector_size;
        image_data[fat_offset] = 0xF8;
        image_data[fat_offset + 1] = 0xFF;
        image_data[fat_offset + 2] = 0xFF;
        image_data[fat_offset + 3] = 0xFF;
    }

    image_data
}

fn make_blank_at_hdd_image(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, build_blank_partitioned_at_hdd()).unwrap();
    path
}

fn make_blank_image(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, build_blank_fat12_floppy()).unwrap();
    path
}

fn make_blank_d88_image(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, build_blank_d88_floppy()).unwrap();
    path
}

fn make_blank_nhd_image(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, build_blank_partitioned_nhd()).unwrap();
    path
}

fn host(path: impl Into<PathBuf>) -> CopyArg {
    CopyArg::Host(path.into())
}

fn image(image_path: impl Into<PathBuf>, dos_path: impl Into<Vec<u8>>) -> CopyArg {
    CopyArg::Image {
        image_path: image_path.into(),
        dos_path: dos_path.into(),
    }
}

#[test]
fn host_to_image_roundtrip_preserves_a_single_file() {
    let folder = TemporaryFolder::new("copy_roundtrip");
    let dir = folder.path();
    let image_path = make_blank_image(dir, "disk.hdm");
    let src = dir.join("input.txt");
    std::fs::write(&src, b"hello round trip\n").unwrap();

    copy(host(&src), image(&image_path, b"A:\\OUT.TXT".to_vec())).expect("write into image");

    let out = dir.join("out.txt");
    copy(image(&image_path, b"A:\\OUT.TXT".to_vec()), host(&out)).expect("read from image");

    assert_eq!(std::fs::read(&out).unwrap(), b"hello round trip\n",);
}

#[test]
fn host_directory_is_copied_recursively_with_subdirs() {
    let folder = TemporaryFolder::new("copy_recursive");
    let dir = folder.path();
    let image_path = make_blank_image(dir, "disk.hdm");
    let src = dir.join("srcdir");
    std::fs::create_dir_all(src.join("SUB")).unwrap();
    std::fs::write(src.join("A.TXT"), b"alpha\n").unwrap();
    std::fs::write(src.join("SUB/B.TXT"), b"bravo\n").unwrap();

    copy(host(&src), image(&image_path, b"A:\\TEST".to_vec())).expect("recursive copy in");

    // After the host->image copy, TEST already exists as a directory. Copy
    // semantics therefore place SRCDIR underneath it on the second run.
    let out = dir.join("outdir");
    std::fs::create_dir_all(&out).unwrap();
    copy(image(&image_path, b"A:\\TEST".to_vec()), host(&out)).expect("recursive copy out");

    let extracted = out.join("TEST");
    assert!(
        extracted.is_dir(),
        "expected {} to exist",
        extracted.display()
    );
    assert_eq!(std::fs::read(extracted.join("A.TXT")).unwrap(), b"alpha\n");
    assert_eq!(
        std::fs::read(extracted.join("SUB").join("B.TXT")).unwrap(),
        b"bravo\n",
    );
}

#[test]
fn intermediate_directories_are_created_for_nested_targets() {
    let folder = TemporaryFolder::new("copy_mkdir_p");
    let dir = folder.path();
    let image_path = make_blank_image(dir, "disk.hdm");
    let src = dir.join("payload.txt");
    std::fs::write(&src, b"contents\n").unwrap();

    copy(
        host(&src),
        image(&image_path, b"A:\\DEEP\\NESTED\\PATH\\X.TXT".to_vec()),
    )
    .expect("write with nested target");

    let out = dir.join("readback.txt");
    copy(
        image(&image_path, b"A:\\DEEP\\NESTED\\PATH\\X.TXT".to_vec()),
        host(&out),
    )
    .expect("readback");
    assert_eq!(std::fs::read(&out).unwrap(), b"contents\n");
}

#[test]
fn image_to_image_copies_a_subtree_across_images() {
    let folder = TemporaryFolder::new("copy_img2img");
    let dir = folder.path();
    let source_image = make_blank_image(dir, "src.hdm");
    let dest_image = make_blank_image(dir, "dst.hdm");

    // Stage a small tree on the source image first.
    let stage = dir.join("stage");
    std::fs::create_dir_all(stage.join("SUB")).unwrap();
    std::fs::write(stage.join("A.TXT"), b"AAA").unwrap();
    std::fs::write(stage.join("SUB/B.TXT"), b"BBB").unwrap();
    copy(host(&stage), image(&source_image, b"A:\\TREE".to_vec())).unwrap();

    // Now copy from the source image into the destination image.
    copy(
        image(&source_image, b"A:\\TREE".to_vec()),
        image(&dest_image, b"A:\\COPIED".to_vec()),
    )
    .expect("image-to-image copy");

    let out = dir.join("verify");
    std::fs::create_dir_all(&out).unwrap();
    copy(image(&dest_image, b"A:\\COPIED".to_vec()), host(&out)).unwrap();
    assert_eq!(
        std::fs::read(out.join("COPIED").join("A.TXT")).unwrap(),
        b"AAA"
    );
    assert_eq!(
        std::fs::read(out.join("COPIED").join("SUB").join("B.TXT")).unwrap(),
        b"BBB",
    );
}

#[test]
fn host_to_host_is_rejected() {
    let folder = TemporaryFolder::new("copy_hh_reject");
    let dir = folder.path();
    let a = dir.join("a");
    let b = dir.join("b");
    std::fs::write(&a, b"x").unwrap();
    let result = copy(host(&a), host(&b));
    assert!(result.is_err());
    // No file should have been created at the destination.
    assert!(!b.exists());
}

#[test]
fn pre_flight_rejects_long_filenames_before_writing() {
    let folder = TemporaryFolder::new("copy_preflight");
    let dir = folder.path();
    let image_path = make_blank_image(dir, "disk.hdm");
    let tree = dir.join("tree");
    std::fs::create_dir_all(&tree).unwrap();
    std::fs::write(tree.join("short.txt"), b"ok").unwrap();
    std::fs::write(tree.join("this_is_too_long_for_dos.txt"), b"bad").unwrap();

    let before = std::fs::read(&image_path).unwrap();
    let result = copy(host(&tree), image(&image_path, b"A:\\TARGET".to_vec()));
    assert!(result.is_err(), "expected pre-flight rejection");
    let after = std::fs::read(&image_path).unwrap();
    assert_eq!(
        before, after,
        "image must be untouched when pre-flight validation fails",
    );
}

#[test]
fn missing_source_in_image_returns_error() {
    let folder = TemporaryFolder::new("copy_missing");
    let dir = folder.path();
    let image_path = make_blank_image(dir, "disk.hdm");
    let out = dir.join("out.txt");
    let result = copy(image(&image_path, b"A:\\NOSUCH.TXT".to_vec()), host(&out));
    assert!(result.is_err());
    assert!(!out.exists());
}

#[test]
fn unknown_image_extension_is_rejected() {
    let folder = TemporaryFolder::new("copy_ext");
    let dir = folder.path();
    let bogus = dir.join("nope.xyz");
    std::fs::write(&bogus, b"not an image").unwrap();
    let out = dir.join("out.txt");
    let result = copy(image(&bogus, b"A:\\X.TXT".to_vec()), host(&out));
    assert!(result.is_err());
}

#[test]
fn overwriting_an_existing_file_replaces_its_contents() {
    let folder = TemporaryFolder::new("copy_overwrite");
    let dir = folder.path();
    let image_path = make_blank_image(dir, "disk.hdm");
    let v1 = dir.join("v1.txt");
    let v2 = dir.join("v2.txt");
    std::fs::write(&v1, b"first").unwrap();
    std::fs::write(&v2, b"second_overwrite").unwrap();

    copy(host(&v1), image(&image_path, b"A:\\FILE.TXT".to_vec())).unwrap();
    copy(host(&v2), image(&image_path, b"A:\\FILE.TXT".to_vec())).unwrap();

    let out = dir.join("out.txt");
    copy(image(&image_path, b"A:\\FILE.TXT".to_vec()), host(&out)).unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), b"second_overwrite");
}

#[test]
fn lowercase_host_filename_is_normalized_to_uppercase() {
    let folder = TemporaryFolder::new("copy_lower");
    let dir = folder.path();
    let image_path = make_blank_image(dir, "disk.hdm");
    let src = dir.join("mixed.txt");
    std::fs::write(&src, b"mixed case").unwrap();

    copy(host(&src), image(&image_path, b"A:\\".to_vec())).unwrap();

    let out = dir.join("readback.txt");
    copy(image(&image_path, b"A:\\MIXED.TXT".to_vec()), host(&out))
        .expect("file is stored under upper-cased basename");
    assert_eq!(std::fs::read(&out).unwrap(), b"mixed case");
}

#[test]
fn file_sizes_around_cluster_boundaries_roundtrip() {
    let folder = TemporaryFolder::new("copy_sizes");
    let dir = folder.path();
    let image_path = make_blank_image(dir, "disk.hdm");
    let sizes = [0usize, 1, 1023, 1024, 1025, 4097];

    for (index, size) in sizes.into_iter().enumerate() {
        let src = dir.join(format!("src{index}.bin"));
        let data = (0..size)
            .map(|byte_index| (byte_index % 251) as u8)
            .collect::<Vec<_>>();
        std::fs::write(&src, &data).unwrap();

        let dos_path = format!("A:\\S{index}.BIN").into_bytes();
        copy(host(&src), image(&image_path, dos_path.clone())).expect("write test size");

        let out = dir.join(format!("out{index}.bin"));
        copy(image(&image_path, dos_path), host(&out)).expect("read test size");
        assert_eq!(std::fs::read(&out).unwrap(), data, "size {size}");
    }
}

#[test]
fn empty_directories_survive_host_and_image_copies() {
    let folder = TemporaryFolder::new("copy_empty_dirs");
    let dir = folder.path();
    let source_image = make_blank_image(dir, "src.hdm");
    let dest_image = make_blank_image(dir, "dst.hdm");
    let src = dir.join("tree");
    std::fs::create_dir_all(src.join("EMPTY")).unwrap();
    std::fs::create_dir_all(src.join("NEST/EMPTY2")).unwrap();

    copy(host(&src), image(&source_image, b"A:\\TREE".to_vec())).unwrap();

    let host_out = dir.join("host_out");
    std::fs::create_dir_all(&host_out).unwrap();
    copy(image(&source_image, b"A:\\TREE".to_vec()), host(&host_out)).unwrap();
    assert!(host_out.join("TREE/EMPTY").is_dir());
    assert_eq!(
        std::fs::read_dir(host_out.join("TREE/EMPTY"))
            .unwrap()
            .count(),
        0
    );
    assert!(host_out.join("TREE/NEST/EMPTY2").is_dir());

    copy(
        image(&source_image, b"A:\\TREE".to_vec()),
        image(&dest_image, b"A:\\COPIED".to_vec()),
    )
    .unwrap();

    let image_out = dir.join("image_out");
    std::fs::create_dir_all(&image_out).unwrap();
    copy(image(&dest_image, b"A:\\COPIED".to_vec()), host(&image_out)).unwrap();
    assert!(image_out.join("COPIED/EMPTY").is_dir());
    assert_eq!(
        std::fs::read_dir(image_out.join("COPIED/EMPTY"))
            .unwrap()
            .count(),
        0
    );
    assert!(image_out.join("COPIED/NEST/EMPTY2").is_dir());
}

#[test]
fn files_are_placed_inside_existing_directories() {
    let folder = TemporaryFolder::new("copy_existing_dirs");
    let dir = folder.path();
    let image_path = make_blank_image(dir, "disk.hdm");
    let seed = dir.join("seed.txt");
    let payload = dir.join("payload.bin");
    std::fs::write(&seed, b"seed").unwrap();
    std::fs::write(&payload, b"payload").unwrap();

    copy(
        host(&seed),
        image(&image_path, b"A:\\DIR\\SEED.TXT".to_vec()),
    )
    .unwrap();
    copy(host(&payload), image(&image_path, b"A:\\DIR".to_vec())).unwrap();

    let readback = dir.join("readback.bin");
    copy(
        image(&image_path, b"A:\\DIR\\PAYLOAD.BIN".to_vec()),
        host(&readback),
    )
    .unwrap();
    assert_eq!(std::fs::read(&readback).unwrap(), b"payload");

    let host_dir = dir.join("host_dir");
    std::fs::create_dir_all(&host_dir).unwrap();
    copy(
        image(&image_path, b"A:\\DIR\\PAYLOAD.BIN".to_vec()),
        host(&host_dir),
    )
    .unwrap();
    assert_eq!(
        std::fs::read(host_dir.join("PAYLOAD.BIN")).unwrap(),
        b"payload"
    );
}

#[test]
fn trailing_slash_missing_image_destination_leaves_image_unchanged() {
    let folder = TemporaryFolder::new("copy_slash_missing");
    let dir = folder.path();
    let image_path = make_blank_image(dir, "disk.hdm");
    let src = dir.join("input.txt");
    std::fs::write(&src, b"input").unwrap();

    let before = std::fs::read(&image_path).unwrap();
    let result = copy(host(&src), image(&image_path, b"A:\\MISSING\\".to_vec()));
    assert!(result.is_err());
    assert_eq!(std::fs::read(&image_path).unwrap(), before);
}

#[test]
fn file_parent_component_rejects_copy_without_image_mutation() {
    let folder = TemporaryFolder::new("copy_file_parent");
    let dir = folder.path();
    let image_path = make_blank_image(dir, "disk.hdm");
    let blocker = dir.join("blocker.txt");
    let src = dir.join("input.txt");
    std::fs::write(&blocker, b"blocker").unwrap();
    std::fs::write(&src, b"input").unwrap();

    copy(host(&blocker), image(&image_path, b"A:\\BLOCK".to_vec())).unwrap();
    let before = std::fs::read(&image_path).unwrap();
    let result = copy(
        host(&src),
        image(&image_path, b"A:\\BLOCK\\CHILD.TXT".to_vec()),
    );
    assert!(result.is_err());
    assert_eq!(std::fs::read(&image_path).unwrap(), before);
}

#[test]
fn host_directory_to_existing_image_file_is_rejected_without_mutation() {
    let folder = TemporaryFolder::new("copy_dir_to_file");
    let dir = folder.path();
    let image_path = make_blank_image(dir, "disk.hdm");
    let existing = dir.join("existing.txt");
    let src = dir.join("tree");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("A.TXT"), b"alpha").unwrap();
    std::fs::write(&existing, b"existing").unwrap();

    copy(host(&existing), image(&image_path, b"A:\\DEST".to_vec())).unwrap();
    let before = std::fs::read(&image_path).unwrap();
    let result = copy(host(&src), image(&image_path, b"A:\\DEST".to_vec()));
    assert!(result.is_err());
    assert_eq!(std::fs::read(&image_path).unwrap(), before);
}

#[test]
fn image_directory_to_existing_host_file_is_rejected_without_overwrite() {
    let folder = TemporaryFolder::new("copy_image_dir_to_file");
    let dir = folder.path();
    let image_path = make_blank_image(dir, "disk.hdm");
    let src = dir.join("tree");
    let dest = dir.join("dest");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("A.TXT"), b"alpha").unwrap();
    std::fs::write(&dest, b"host file").unwrap();

    copy(host(&src), image(&image_path, b"A:\\TREE".to_vec())).unwrap();
    let result = copy(image(&image_path, b"A:\\TREE".to_vec()), host(&dest));
    assert!(result.is_err());
    assert_eq!(std::fs::read(&dest).unwrap(), b"host file");
}

#[test]
fn disk_full_error_leaves_image_bytes_unchanged() {
    let folder = TemporaryFolder::new("copy_disk_full");
    let dir = folder.path();
    let image_path = dir.join("full.hdm");
    let mut image_bytes = build_blank_fat12_floppy();
    mark_floppy_data_clusters_used(&mut image_bytes);
    std::fs::write(&image_path, &image_bytes).unwrap();
    let src = dir.join("input.txt");
    std::fs::write(&src, b"x").unwrap();

    let result = copy(host(&src), image(&image_path, b"A:\\X.TXT".to_vec()));
    assert!(result.is_err());
    assert_eq!(std::fs::read(&image_path).unwrap(), image_bytes);
}

#[test]
fn invalid_image_source_path_does_not_truncate_for_image_to_host() {
    let folder = TemporaryFolder::new("copy_bad_source_host");
    let dir = folder.path();
    let image_path = make_blank_image(dir, "disk.hdm");
    let src = dir.join("source.txt");
    let out = dir.join("out.txt");
    std::fs::write(&src, b"must not copy").unwrap();

    copy(host(&src), image(&image_path, b"A:\\TOOLONGN.TXT".to_vec())).unwrap();
    let result = copy(
        image(&image_path, b"A:\\TOOLONGNAME.TXT".to_vec()),
        host(&out),
    );
    assert!(result.is_err());
    assert!(!out.exists());
}

#[test]
fn invalid_image_source_path_does_not_truncate_for_image_to_image() {
    let folder = TemporaryFolder::new("copy_bad_source_image");
    let dir = folder.path();
    let source_image = make_blank_image(dir, "src.hdm");
    let dest_image = make_blank_image(dir, "dst.hdm");
    let src = dir.join("source.txt");
    std::fs::write(&src, b"must not copy").unwrap();

    copy(
        host(&src),
        image(&source_image, b"A:\\TOOLONGN.TXT".to_vec()),
    )
    .unwrap();
    let before = std::fs::read(&dest_image).unwrap();
    let result = copy(
        image(&source_image, b"A:\\TOOLONGNAME.TXT".to_vec()),
        image(&dest_image, b"A:\\OUT.TXT".to_vec()),
    );
    assert!(result.is_err());
    assert_eq!(std::fs::read(&dest_image).unwrap(), before);
}

// Windows strips trailing dots from file names at the OS level, so `FILE` and
// `FILE.` cannot coexist in the host directory; the collision scenario can
// only be constructed on case-sensitive Unix-like filesystems.
#[cfg(unix)]
#[test]
fn host_name_collisions_are_rejected_before_writing() {
    let folder = TemporaryFolder::new("copy_collisions");
    let dir = folder.path();
    let image_path = make_blank_image(dir, "disk.hdm");
    let tree = dir.join("tree");
    std::fs::create_dir_all(&tree).unwrap();
    std::fs::write(tree.join("FILE"), b"first").unwrap();
    std::fs::write(tree.join("FILE."), b"second").unwrap();

    let before = std::fs::read(&image_path).unwrap();
    let result = copy(host(&tree), image(&image_path, b"A:\\TARGET".to_vec()));
    assert!(result.is_err());
    assert_eq!(std::fs::read(&image_path).unwrap(), before);
}

#[test]
fn d88_floppy_roundtrip_preserves_file_contents() {
    let folder = TemporaryFolder::new("copy_d88");
    let dir = folder.path();
    let image_path = make_blank_d88_image(dir, "disk.d88");
    let src = dir.join("source.txt");
    let out = dir.join("out.txt");
    std::fs::write(&src, b"d88 data").unwrap();

    copy(host(&src), image(&image_path, b"A:\\D88.TXT".to_vec())).unwrap();
    copy(image(&image_path, b"A:\\D88.TXT".to_vec()), host(&out)).unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), b"d88 data");
}

#[test]
fn partitioned_nhd_roundtrip_preserves_file_contents() {
    let folder = TemporaryFolder::new("copy_nhd");
    let dir = folder.path();
    let image_path = make_blank_nhd_image(dir, "disk.nhd");
    let src = dir.join("source.txt");
    let out = dir.join("out.txt");
    std::fs::write(&src, b"nhd data").unwrap();

    copy(host(&src), image(&image_path, b"A:\\NHD.TXT".to_vec())).unwrap();
    copy(image(&image_path, b"A:\\NHD.TXT".to_vec()), host(&out)).unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), b"nhd data");
}

#[test]
fn mbr_partitioned_at_hdd_roundtrip_preserves_file_contents() {
    let folder = TemporaryFolder::new("copy_athdd");
    let dir = folder.path();
    let image_path = make_blank_at_hdd_image(dir, "disk.hdd");
    let src = dir.join("source.txt");
    let out = dir.join("out.txt");
    std::fs::write(&src, b"mbr partition data").unwrap();

    copy(host(&src), image(&image_path, b"A:\\MBR.TXT".to_vec())).unwrap();
    copy(image(&image_path, b"A:\\MBR.TXT".to_vec()), host(&out)).unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), b"mbr partition data");

    // The write landed inside the partition: the MBR sector is untouched
    // and the file data sits past the partition start.
    let bytes = std::fs::read(&image_path).unwrap();
    assert_eq!(bytes[510], 0x55);
    assert_eq!(bytes[511], 0xAA);
    let needle = b"mbr partition data";
    let position = bytes
        .windows(needle.len())
        .position(|window| window == needle)
        .expect("file data present in the image");
    assert!(position >= AT_PARTITION_START_LBA as usize * 512);
}

#[test]
fn dosv_144mb_img_floppy_roundtrip() {
    let folder = TemporaryFolder::new("copy_imgfdd");
    let dir = folder.path();
    // A blank 1.44 MB DOS floppy: FAT12, 18 sectors per track, 2 heads.
    let mut data = vec![0u8; 1_474_560];
    data[11..13].copy_from_slice(&512u16.to_le_bytes());
    data[13] = 1;
    data[14..16].copy_from_slice(&1u16.to_le_bytes());
    data[16] = 2;
    data[17..19].copy_from_slice(&224u16.to_le_bytes());
    data[19..21].copy_from_slice(&2880u16.to_le_bytes());
    data[21] = 0xF0;
    data[22..24].copy_from_slice(&9u16.to_le_bytes());
    data[24..26].copy_from_slice(&18u16.to_le_bytes());
    data[26..28].copy_from_slice(&2u16.to_le_bytes());
    for fat_offset in [512usize, 512 + 9 * 512] {
        data[fat_offset] = 0xF0;
        data[fat_offset + 1] = 0xFF;
        data[fat_offset + 2] = 0xFF;
    }
    let image_path = dir.join("disk.img");
    std::fs::write(&image_path, &data).unwrap();

    let src = dir.join("source.txt");
    let out = dir.join("out.txt");
    std::fs::write(&src, b"dosv floppy data").unwrap();

    copy(host(&src), image(&image_path, b"A:\\DOSV.TXT".to_vec())).unwrap();
    copy(image(&image_path, b"A:\\DOSV.TXT".to_vec()), host(&out)).unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), b"dosv floppy data");
}

#[test]
fn msx_720kb_dsk_floppy_roundtrip() {
    let folder = TemporaryFolder::new("copy_msxdsk");
    let dir = folder.path();
    let mut data = vec![0u8; 737_280];
    data[11..13].copy_from_slice(&512u16.to_le_bytes());
    data[13] = 2;
    data[14..16].copy_from_slice(&1u16.to_le_bytes());
    data[16] = 2;
    data[17..19].copy_from_slice(&112u16.to_le_bytes());
    data[19..21].copy_from_slice(&1440u16.to_le_bytes());
    data[21] = 0xF9;
    data[22..24].copy_from_slice(&3u16.to_le_bytes());
    data[24..26].copy_from_slice(&9u16.to_le_bytes());
    data[26..28].copy_from_slice(&2u16.to_le_bytes());
    for fat_offset in [512usize, 512 + 3 * 512] {
        data[fat_offset] = 0xF9;
        data[fat_offset + 1] = 0xFF;
        data[fat_offset + 2] = 0xFF;
    }
    let image_path = dir.join("disk.dsk");
    std::fs::write(&image_path, &data).unwrap();

    let source = dir.join("source.txt");
    let output = dir.join("output.txt");
    std::fs::write(&source, b"MSX DSK data").unwrap();

    copy(host(&source), image(&image_path, b"A:\\MSX.TXT".to_vec())).unwrap();
    copy(image(&image_path, b"A:\\MSX.TXT".to_vec()), host(&output)).unwrap();
    assert_eq!(std::fs::read(&output).unwrap(), b"MSX DSK data");
    assert_eq!(std::fs::metadata(&image_path).unwrap().len(), 737_280);
}

/// Builds a PC-98 `.hdi` image partitioned and formatted in its image
/// geometry with the partition on cylinder 1, the layout of an image written
/// before the PC-98 IDE BIOS geometry.
fn build_image_geometry_layout_hdi(cylinders: u16, heads: u8, sectors_per_track: u8) -> Vec<u8> {
    let geometry = HddGeometry {
        cylinders,
        heads,
        sectors_per_track,
        sector_size: HDD_SECTOR_SIZE,
    };
    let mut image = HddImage::from_raw(
        geometry,
        HddFormat::Hdi,
        vec![0u8; geometry.total_bytes() as usize],
    );
    let start = u32::from(heads) * u32::from(sectors_per_track);
    let partition_sectors = u32::from(cylinders - 2) * start;

    let mut table = vec![0u8; usize::from(HDD_SECTOR_SIZE)];
    table[0] = 0xA1;
    table[1] = 0x91;
    table[6..8].copy_from_slice(&1u16.to_le_bytes());
    table[10..12].copy_from_slice(&1u16.to_le_bytes());
    table[14..16].copy_from_slice(&(cylinders - 1).to_le_bytes());
    image.write_sector(1, &table);

    let bpb = hdd_bpb_params(HDD_SECTOR_SIZE, partition_sectors);
    let boot = hdd_boot_sector(
        &bpb,
        HDD_SECTOR_SIZE,
        partition_sectors,
        sectors_per_track,
        heads,
        start,
    );
    image.write_sector(start, &boot);
    let fat = fat_region(&bpb, HDD_SECTOR_SIZE);
    for copy in 0..u32::from(bpb.num_fats) {
        let base = start + u32::from(bpb.reserved_sectors) + copy * u32::from(bpb.sectors_per_fat);
        for (offset, sector) in fat.chunks_exact(usize::from(HDD_SECTOR_SIZE)).enumerate() {
            image.write_sector(base + offset as u32, sector);
        }
    }
    image.to_bytes()
}

#[test]
fn ide_image_formatted_in_the_bios_geometry_roundtrips_a_file() {
    let folder = TemporaryFolder::new("copy_ide_bios_geometry");
    let dir = folder.path();
    let mut disk = blank_hdd_image(HddSizeType::IdeMb500);
    format_hdd_image(&mut disk, PartitionTableType::Pc98).unwrap();
    let image_path = dir.join("disk.hdi");
    std::fs::write(&image_path, disk.to_bytes()).unwrap();
    let src = dir.join("input.txt");
    std::fs::write(&src, b"bios geometry\n").unwrap();

    copy(host(&src), image(&image_path, b"C:\\OUT.TXT".to_vec())).expect("write into image");
    let out = dir.join("out.txt");
    copy(image(&image_path, b"C:\\OUT.TXT".to_vec()), host(&out)).expect("read from image");

    assert_eq!(std::fs::read(&out).unwrap(), b"bios geometry\n");
}

#[test]
fn ide_image_in_the_image_geometry_is_migrated_and_roundtrips_a_file() {
    let folder = TemporaryFolder::new("copy_ide_image_geometry");
    let dir = folder.path();
    let image_path = dir.join("disk.hdi");
    std::fs::write(&image_path, build_image_geometry_layout_hdi(977, 10, 17)).unwrap();
    let src = dir.join("input.txt");
    std::fs::write(&src, b"image geometry\n").unwrap();

    copy(host(&src), image(&image_path, b"C:\\OUT.TXT".to_vec())).expect("write into image");
    let out = dir.join("out.txt");
    copy(image(&image_path, b"C:\\OUT.TXT".to_vec()), host(&out)).expect("read from image");

    assert_eq!(std::fs::read(&out).unwrap(), b"image geometry\n");
    let written = std::fs::read(&image_path).unwrap();
    let boot_record = 32 + 136 * usize::from(HDD_SECTOR_SIZE);
    assert_eq!(&written[boot_record + 24..boot_record + 28], &[17, 0, 8, 0]);
}

/// The FAT12 layout of a floppy volume in a test image.
#[derive(Clone, Copy)]
struct FloppyLayout {
    cylinders: usize,
    heads: usize,
    sectors_per_track: usize,
    sector_size: usize,
    sectors_per_cluster: usize,
    sectors_per_fat: usize,
    root_entry_count: usize,
    media_byte: u8,
}

/// Reserved sectors of every test volume: the boot sector.
const LAYOUT_RESERVED_SECTORS: usize = 1;
/// FAT copies of every test volume.
const LAYOUT_FAT_COUNT: usize = 2;
/// Byte count of a directory entry.
const DIRECTORY_ENTRY_SIZE: usize = 32;

/// PC-98 and X68000 2HD: 77 x 2 x 8 x 1024.
const LAYOUT_2HD: FloppyLayout = FloppyLayout {
    cylinders: 77,
    heads: 2,
    sectors_per_track: 8,
    sector_size: 1024,
    sectors_per_cluster: 1,
    sectors_per_fat: 2,
    root_entry_count: 192,
    media_byte: 0xFE,
};

/// 2HQ 1.44 MB: 80 x 2 x 18 x 512.
const LAYOUT_2HQ: FloppyLayout = FloppyLayout {
    cylinders: 80,
    heads: 2,
    sectors_per_track: 18,
    sector_size: 512,
    sectors_per_cluster: 1,
    sectors_per_fat: 9,
    root_entry_count: 224,
    media_byte: 0xF0,
};

/// X68000 2HS and 2HDE: 80 x 2 x 9 x 1024.
const LAYOUT_2HS: FloppyLayout = FloppyLayout {
    cylinders: 80,
    heads: 2,
    sectors_per_track: 9,
    sector_size: 1024,
    sectors_per_cluster: 1,
    sectors_per_fat: 3,
    root_entry_count: 192,
    media_byte: 0xFA,
};

/// 2D 320 KB with 256-byte sectors: 40 x 2 x 16 x 256.
const LAYOUT_2D_256: FloppyLayout = FloppyLayout {
    cylinders: 40,
    heads: 2,
    sectors_per_track: 16,
    sector_size: 256,
    sectors_per_cluster: 2,
    sectors_per_fat: 4,
    root_entry_count: 64,
    media_byte: 0xFE,
};

impl FloppyLayout {
    /// Returns the sector count of the volume.
    const fn total_sectors(&self) -> usize {
        self.cylinders * self.heads * self.sectors_per_track
    }

    /// Returns the uPD765 size code N of the sector size.
    const fn size_code(&self) -> u8 {
        (self.sector_size / 128).trailing_zeros() as u8
    }

    /// Returns the physical geometry of the layout.
    const fn geometry(&self) -> FloppyGeometry {
        FloppyGeometry {
            cylinders: self.cylinders as u16,
            heads: self.heads as u8,
            sectors_per_track: self.sectors_per_track as u8,
            sector_size: self.sector_size as u16,
        }
    }

    /// Returns the byte offset of the root directory in the volume.
    const fn root_directory_offset(&self) -> usize {
        (LAYOUT_RESERVED_SECTORS + LAYOUT_FAT_COUNT * self.sectors_per_fat) * self.sector_size
    }

    /// Returns the byte offset of `cluster` in the volume.
    const fn cluster_offset(&self, cluster: u16) -> usize {
        let root_directory_bytes = self.root_entry_count * DIRECTORY_ENTRY_SIZE;
        let data_offset = self.root_directory_offset()
            + root_directory_bytes.div_ceil(self.sector_size) * self.sector_size;
        data_offset + (cluster as usize - 2) * self.sectors_per_cluster * self.sector_size
    }

    /// Returns the byte count of a cluster.
    const fn cluster_size(&self) -> usize {
        self.sectors_per_cluster * self.sector_size
    }
}

/// The boot sector of a test volume.
#[derive(Clone, Copy)]
enum BootSector {
    /// A DOS boot sector with a little-endian BPB at `0x0B`.
    Dos,
    /// A Human68k 3.x boot sector: `X68IPL30` at offset 3 and a little-endian BPB at `0x0B`.
    Human68kIpl,
    /// A Human68k 2.x boot sector: `Hudson soft 2.00` and a big-endian BPB at `0x12`.
    Human68kHudson,
    /// A zeroed boot sector without a BPB.
    Blank,
}

/// Writes the little-endian BPB of `layout` at `0x0B` of `boot_sector`.
fn write_little_endian_bpb(boot_sector: &mut [u8], layout: &FloppyLayout) {
    boot_sector[11..13].copy_from_slice(&(layout.sector_size as u16).to_le_bytes());
    boot_sector[13] = layout.sectors_per_cluster as u8;
    boot_sector[14..16].copy_from_slice(&(LAYOUT_RESERVED_SECTORS as u16).to_le_bytes());
    boot_sector[16] = LAYOUT_FAT_COUNT as u8;
    boot_sector[17..19].copy_from_slice(&(layout.root_entry_count as u16).to_le_bytes());
    boot_sector[19..21].copy_from_slice(&(layout.total_sectors() as u16).to_le_bytes());
    boot_sector[21] = layout.media_byte;
    boot_sector[22..24].copy_from_slice(&(layout.sectors_per_fat as u16).to_le_bytes());
    boot_sector[24..26].copy_from_slice(&(layout.sectors_per_track as u16).to_le_bytes());
    boot_sector[26..28].copy_from_slice(&(layout.heads as u16).to_le_bytes());
}

/// Builds the linear sectors of an empty FAT12 volume of `layout` with `boot`.
fn build_fat12_volume(layout: &FloppyLayout, boot: BootSector) -> Vec<u8> {
    let mut volume = vec![0u8; layout.total_sectors() * layout.sector_size];
    let boot_sector = &mut volume[..layout.sector_size];
    match boot {
        BootSector::Dos => {
            boot_sector[..3].copy_from_slice(&[0xEB, 0x3C, 0x90]);
            boot_sector[3..11].copy_from_slice(b"NEETAN  ");
            write_little_endian_bpb(boot_sector, layout);
        }
        BootSector::Human68kIpl => {
            boot_sector[0] = 0x60;
            boot_sector[1] = 0x3C;
            boot_sector[3..11].copy_from_slice(b"X68IPL30");
            write_little_endian_bpb(boot_sector, layout);
        }
        BootSector::Human68kHudson => {
            boot_sector[0] = 0x60;
            boot_sector[1] = 0x1C;
            boot_sector[2..18].copy_from_slice(b"Hudson soft 2.00");
            boot_sector[18..20].copy_from_slice(&(layout.sector_size as u16).to_be_bytes());
            boot_sector[20] = layout.sectors_per_cluster as u8;
            boot_sector[21] = LAYOUT_FAT_COUNT as u8;
            boot_sector[22..24].copy_from_slice(&(LAYOUT_RESERVED_SECTORS as u16).to_be_bytes());
            boot_sector[24..26].copy_from_slice(&(layout.root_entry_count as u16).to_be_bytes());
            boot_sector[26..28].copy_from_slice(&(layout.total_sectors() as u16).to_be_bytes());
            boot_sector[28] = layout.media_byte;
            boot_sector[29] = layout.sectors_per_fat as u8;
        }
        BootSector::Blank => {}
    }
    for fat_index in 0..LAYOUT_FAT_COUNT {
        let fat_offset =
            (LAYOUT_RESERVED_SECTORS + fat_index * layout.sectors_per_fat) * layout.sector_size;
        volume[fat_offset..fat_offset + 3].copy_from_slice(&[layout.media_byte, 0xFF, 0xFF]);
    }
    volume
}

/// Stores a file in the root directory slot `slot` of `volume`: the 11-byte FCB name, the
/// Human68k name continuation of bytes 12 to 21 and `data` in consecutive clusters from
/// `first_cluster`.
fn add_root_file(
    volume: &mut [u8],
    layout: &FloppyLayout,
    slot: usize,
    fcb_name: &[u8; 11],
    continuation: &[u8],
    first_cluster: u16,
    data: &[u8],
) {
    let entry_offset = layout.root_directory_offset() + slot * DIRECTORY_ENTRY_SIZE;
    add_directory_entry(
        volume,
        entry_offset,
        fcb_name,
        0x20,
        continuation,
        first_cluster,
        data.len() as u32,
    );
    let cluster_count = data.len().div_ceil(layout.cluster_size()).max(1) as u16;
    for index in 0..cluster_count {
        let cluster = first_cluster + index;
        let next = if index + 1 == cluster_count {
            0x0FFF
        } else {
            cluster + 1
        };
        set_layout_fat_entry(volume, layout, cluster, next);
    }
    let data_offset = layout.cluster_offset(first_cluster);
    volume[data_offset..data_offset + data.len()].copy_from_slice(data);
}

/// Writes a 32-byte directory entry at `entry_offset` of `volume`.
fn add_directory_entry(
    volume: &mut [u8],
    entry_offset: usize,
    fcb_name: &[u8; 11],
    attribute: u8,
    continuation: &[u8],
    start_cluster: u16,
    size: u32,
) {
    let entry = &mut volume[entry_offset..entry_offset + DIRECTORY_ENTRY_SIZE];
    entry[..11].copy_from_slice(fcb_name);
    entry[11] = attribute;
    entry[12..12 + continuation.len()].copy_from_slice(continuation);
    entry[26..28].copy_from_slice(&start_cluster.to_le_bytes());
    entry[28..32].copy_from_slice(&size.to_le_bytes());
}

/// Sets the FAT12 entry of `cluster` to `value` in every FAT of `volume`.
fn set_layout_fat_entry(volume: &mut [u8], layout: &FloppyLayout, cluster: u16, value: u16) {
    for fat_index in 0..LAYOUT_FAT_COUNT {
        let fat_offset =
            (LAYOUT_RESERVED_SECTORS + fat_index * layout.sectors_per_fat) * layout.sector_size;
        let fat_size = layout.sectors_per_fat * layout.sector_size;
        set_fat12_entry(
            &mut volume[fat_offset..fat_offset + fat_size],
            cluster,
            value,
        );
    }
}

/// Returns the sector ID head and record of the sector at `slot` of a track. The function
/// takes the cylinder, the physical head and the slot.
type SectorIdRule = fn(u8, u8, u8) -> (u8, u8);

/// Records 1 to n with the physical head.
fn standard_sector_id(_cylinder: u8, head: u8, slot: u8) -> (u8, u8) {
    (head, slot + 1)
}

/// X68000 2HS: records 10 to 18 except the first sector of the disk.
fn sector_id_2hs(cylinder: u8, head: u8, slot: u8) -> (u8, u8) {
    if cylinder == 0 && head == 0 && slot == 0 {
        (head, 1)
    } else {
        (head, slot + 10)
    }
}

/// Builds the D88 track table of `volume` with `extra_cylinders` formatted E5 cylinders past
/// the layout.
fn build_layout_d88_tracks(
    layout: &FloppyLayout,
    volume: &[u8],
    sector_id: SectorIdRule,
    extra_cylinders: usize,
) -> Vec<Option<Vec<D88Sector>>> {
    let track_count = (layout.cylinders + extra_cylinders) * layout.heads;
    (0..track_count)
        .map(|track_index| {
            let cylinder = (track_index / layout.heads) as u8;
            let head = (track_index % layout.heads) as u8;
            let sectors = (0..layout.sectors_per_track)
                .map(|slot| {
                    let linear = track_index * layout.sectors_per_track + slot;
                    let data = if cylinder as usize >= layout.cylinders {
                        vec![0xE5; layout.sector_size]
                    } else {
                        let offset = linear * layout.sector_size;
                        volume[offset..offset + layout.sector_size].to_vec()
                    };
                    let (id_head, record) = sector_id(cylinder, head, slot as u8);
                    D88Sector {
                        cylinder,
                        head: id_head,
                        record,
                        size_code: layout.size_code(),
                        sector_count: layout.sectors_per_track as u16,
                        mfm_flag: 0,
                        deleted: 0,
                        status: 0,
                        reserved: [0; 5],
                        data,
                        source_offset: None,
                    }
                })
                .collect();
            Some(sectors)
        })
        .collect()
}

/// Builds the bytes of a D88 or D77 container of `volume`.
fn build_layout_d88(
    layout: &FloppyLayout,
    volume: &[u8],
    media_type: D88MediaType,
    sector_id: SectorIdRule,
    extra_cylinders: usize,
) -> Vec<u8> {
    let tracks = build_layout_d88_tracks(layout, volume, sector_id, extra_cylinders);
    D88Disk::from_tracks("COPYTEST".to_string(), false, media_type, tracks).to_bytes()
}

/// Byte count of the DIM header.
const DIM_HEADER_SIZE: usize = 256;
/// Byte offset of the `DIFC HEADER` magic in the DIM header.
const DIM_MAGIC_OFFSET: usize = 0xAB;
/// DIM media type of 2HD.
const DIM_MEDIA_2HD: u8 = 0x00;
/// DIM media type of 2HS.
const DIM_MEDIA_2HS: u8 = 0x01;
/// DIM media type of 2HDE.
const DIM_MEDIA_2HDE: u8 = 0x03;
/// DIM media type of 2HQ.
const DIM_MEDIA_2HQ: u8 = 0x09;
/// Byte offset of the comment in the DIM header.
const DIM_COMMENT_OFFSET: usize = 0xC2;
/// Comment of the DIM test images. The parser ignores it.
const DIM_COMMENT_MARKER: &[u8] = b"NEETAN COPY TEST";

/// Builds a DIM image of `volume` with every track marked as saved.
fn build_layout_dim(layout: &FloppyLayout, volume: &[u8], media_type: u8) -> Vec<u8> {
    let mut image = vec![0u8; DIM_HEADER_SIZE];
    image[0] = media_type;
    for track in 0..layout.cylinders * layout.heads {
        image[1 + track] = 0x01;
    }
    image[DIM_MAGIC_OFFSET..DIM_MAGIC_OFFSET + 11].copy_from_slice(b"DIFC HEADER");
    image[DIM_COMMENT_OFFSET..DIM_COMMENT_OFFSET + DIM_COMMENT_MARKER.len()]
        .copy_from_slice(DIM_COMMENT_MARKER);
    image.extend_from_slice(volume);
    image
}

/// Builds an FDI image of `volume` with `media_code`. The header padding holds the marker
/// bytes of `fdi_padding_marker`.
fn build_layout_fdi(layout: &FloppyLayout, volume: &[u8], media_code: u32) -> Vec<u8> {
    let mut image = fdi::header(media_code, &layout.geometry());
    image[fdi::FDI_GEOMETRY_FIELDS_SIZE..].copy_from_slice(&fdi_padding_marker());
    image.extend_from_slice(volume);
    image
}

/// Returns the marker bytes of the FDI header padding from byte 32 to 4095. The parser ignores
/// them.
fn fdi_padding_marker() -> Vec<u8> {
    (fdi::FDI_GEOMETRY_FIELDS_SIZE..fdi::FDI_STANDARD_HEADER_SIZE)
        .map(|offset| (offset % 255) as u8 + 1)
        .collect()
}

/// Asserts that the FDI header padding of `image` holds the marker bytes.
fn assert_fdi_padding_marker(image: &[u8]) {
    assert_eq!(
        image[fdi::FDI_GEOMETRY_FIELDS_SIZE..fdi::FDI_STANDARD_HEADER_SIZE],
        fdi_padding_marker()
    );
}

/// Byte count of the common NFD header.
const NFD_COMMON_HEADER_SIZE: usize = 0x120;
/// Byte count of the NFD R1 track table: 164 track offsets.
const NFD_R1_TRACK_TABLE_SIZE: usize = 164 * 4;
/// Byte count of an NFD R1 track header.
const NFD_R1_TRACK_HEADER_SIZE: usize = 16;
/// Byte count of an NFD sector entry.
const NFD_SECTOR_ENTRY_SIZE: usize = 16;
/// NFD PDA byte of a 2HD sector.
const NFD_PDA_2HD: u8 = 0x90;

/// Builds an NFD R1 image of `volume` in which every sector carries one retry copy. Returns the
/// image and the file offset of the first sector.
fn build_layout_nfd_r1(layout: &FloppyLayout, volume: &[u8]) -> (Vec<u8>, usize) {
    let track_count = layout.cylinders * layout.heads;
    let track_metadata_size =
        NFD_R1_TRACK_HEADER_SIZE + layout.sectors_per_track * NFD_SECTOR_ENTRY_SIZE;
    let fixed_header_end = NFD_COMMON_HEADER_SIZE + NFD_R1_TRACK_TABLE_SIZE;
    let head_size = fixed_header_end + track_count * track_metadata_size;
    let mut image = vec![0u8; head_size];
    image[..15].copy_from_slice(b"T98FDDIMAGE.R1\0");
    image[0x110..0x114].copy_from_slice(&(head_size as u32).to_le_bytes());
    for track_index in 0..track_count {
        let metadata_offset = fixed_header_end + track_index * track_metadata_size;
        let table_offset = NFD_COMMON_HEADER_SIZE + track_index * 4;
        image[table_offset..table_offset + 4]
            .copy_from_slice(&(metadata_offset as u32).to_le_bytes());
        image[metadata_offset..metadata_offset + 2]
            .copy_from_slice(&(layout.sectors_per_track as u16).to_le_bytes());
        for slot in 0..layout.sectors_per_track {
            let entry = metadata_offset + NFD_R1_TRACK_HEADER_SIZE + slot * NFD_SECTOR_ENTRY_SIZE;
            image[entry] = (track_index / layout.heads) as u8;
            image[entry + 1] = (track_index % layout.heads) as u8;
            image[entry + 2] = slot as u8 + 1;
            image[entry + 3] = layout.size_code();
            image[entry + 10] = 1;
            image[entry + 11] = NFD_PDA_2HD;
        }
    }
    for sector in volume.chunks_exact(layout.sector_size) {
        image.extend_from_slice(sector);
        image.extend_from_slice(sector);
    }
    (image, head_size)
}

/// Writes `data` as `A:\FILE.BIN` into the image at `image_path`, reads it back and asserts
/// that the content and the byte count of the image file stay.
fn assert_file_roundtrip(dir: &Path, image_path: &Path, data: &[u8]) {
    let size_before = std::fs::metadata(image_path).unwrap().len();
    let source = dir.join("source.bin");
    let output = dir.join("output.bin");
    std::fs::write(&source, data).unwrap();
    copy(host(&source), image(image_path, b"A:\\FILE.BIN".to_vec())).expect("write into image");
    copy(image(image_path, b"A:\\FILE.BIN".to_vec()), host(&output)).expect("read from image");
    assert_eq!(std::fs::read(&output).unwrap(), data);
    assert_eq!(std::fs::metadata(image_path).unwrap().len(), size_before);
}

/// Returns a test payload of `length` bytes that spans several clusters.
fn payload(length: usize) -> Vec<u8> {
    (0..length).map(|index| (index % 251) as u8).collect()
}

/// Byte count of the payload of the round trip tests: more than four 1024-byte clusters.
const PAYLOAD_LENGTH: usize = 4500;

#[test]
fn fdi_2hd_roundtrip_keeps_the_header() {
    let folder = TemporaryFolder::new("copy_fdi_2hd");
    let dir = folder.path();
    let mut bytes = fdi::standard_2hd_header().to_vec();
    bytes[fdi::FDI_GEOMETRY_FIELDS_SIZE..].copy_from_slice(&fdi_padding_marker());
    bytes.extend_from_slice(&build_blank_fat12_floppy());
    let image_path = dir.join("disk.fdi");
    std::fs::write(&image_path, &bytes).unwrap();

    assert_file_roundtrip(dir, &image_path, &payload(PAYLOAD_LENGTH));

    let listing = dir.join("listing");
    std::fs::create_dir_all(&listing).unwrap();
    copy(image(&image_path, b"A:\\".to_vec()), host(&listing)).expect("list the root");
    assert_eq!(
        std::fs::read(listing.join("FILE.BIN")).unwrap(),
        payload(PAYLOAD_LENGTH)
    );
    let written = std::fs::read(&image_path).unwrap();
    assert_eq!(
        written[..fdi::FDI_STANDARD_HEADER_SIZE],
        bytes[..fdi::FDI_STANDARD_HEADER_SIZE]
    );
    assert_fdi_padding_marker(&written);
}

#[test]
fn fdi_144mb_roundtrip_keeps_the_header() {
    let folder = TemporaryFolder::new("copy_fdi_144");
    let dir = folder.path();
    let volume = build_fat12_volume(&LAYOUT_2HQ, BootSector::Dos);
    let bytes = build_layout_fdi(&LAYOUT_2HQ, &volume, fdi::MEDIA_CODE_2HD_1440);
    let image_path = dir.join("disk.fdi");
    std::fs::write(&image_path, &bytes).unwrap();

    assert_file_roundtrip(dir, &image_path, &payload(PAYLOAD_LENGTH));
    let written = std::fs::read(&image_path).unwrap();
    assert_eq!(
        written[..fdi::FDI_STANDARD_HEADER_SIZE],
        bytes[..fdi::FDI_STANDARD_HEADER_SIZE]
    );
    assert_fdi_padding_marker(&written);
}

#[test]
fn xdf_and_2hd_roundtrip_keeps_the_file_size() {
    for extension in ["xdf", "2hd"] {
        let folder = TemporaryFolder::new("copy_xdf");
        let dir = folder.path();
        let image_path = dir.join(format!("disk.{extension}"));
        std::fs::write(&image_path, build_blank_fat12_floppy()).unwrap();

        assert_file_roundtrip(dir, &image_path, &payload(PAYLOAD_LENGTH));
        assert_eq!(std::fs::metadata(&image_path).unwrap().len(), 1_261_568);
    }
}

#[test]
fn dim_2hd_and_2hq_roundtrip_keeps_the_header() {
    for (layout, media_type) in [(LAYOUT_2HD, DIM_MEDIA_2HD), (LAYOUT_2HQ, DIM_MEDIA_2HQ)] {
        let folder = TemporaryFolder::new("copy_dim");
        let dir = folder.path();
        let volume = build_fat12_volume(&layout, BootSector::Dos);
        let bytes = build_layout_dim(&layout, &volume, media_type);
        let image_path = dir.join("disk.dim");
        std::fs::write(&image_path, &bytes).unwrap();

        assert_file_roundtrip(dir, &image_path, &payload(PAYLOAD_LENGTH));
        let written = std::fs::read(&image_path).unwrap();
        assert_eq!(written[..DIM_HEADER_SIZE], bytes[..DIM_HEADER_SIZE]);
        assert_eq!(
            written[DIM_COMMENT_OFFSET..DIM_COMMENT_OFFSET + DIM_COMMENT_MARKER.len()],
            *DIM_COMMENT_MARKER
        );
    }
}

#[test]
fn dim_2hs_and_2hde_with_human68k_boot_sector_roundtrip() {
    for media_type in [DIM_MEDIA_2HS, DIM_MEDIA_2HDE] {
        let folder = TemporaryFolder::new("copy_dim_2hs");
        let dir = folder.path();
        let volume = build_fat12_volume(&LAYOUT_2HS, BootSector::Human68kHudson);
        let bytes = build_layout_dim(&LAYOUT_2HS, &volume, media_type);
        let image_path = dir.join("disk.dim");
        std::fs::write(&image_path, &bytes).unwrap();

        assert_file_roundtrip(dir, &image_path, &payload(PAYLOAD_LENGTH));
        let written = FloppyImage::from_dim_bytes(&std::fs::read(&image_path).unwrap()).unwrap();
        let file_cluster_offset = LAYOUT_2HS.cluster_offset(2);
        let track = file_cluster_offset / (LAYOUT_2HS.sectors_per_track * LAYOUT_2HS.sector_size);
        let slot = file_cluster_offset / LAYOUT_2HS.sector_size % LAYOUT_2HS.sectors_per_track;
        let sector = written.sector_at_index(track, slot).unwrap();
        assert_eq!(sector.data, payload(PAYLOAD_LENGTH)[..1024]);
    }
}

#[test]
fn d77_2d_roundtrip_keeps_the_format() {
    let folder = TemporaryFolder::new("copy_d77");
    let dir = folder.path();
    let volume = build_fat12_volume(&LAYOUT_2D_256, BootSector::Dos);
    let bytes = build_layout_d88(
        &LAYOUT_2D_256,
        &volume,
        D88MediaType::Disk2D,
        standard_sector_id,
        0,
    );
    let image_path = dir.join("disk.d77");
    std::fs::write(&image_path, &bytes).unwrap();

    assert_file_roundtrip(dir, &image_path, &payload(PAYLOAD_LENGTH));
    let written = std::fs::read(&image_path).unwrap();
    let reloaded = load_floppy_image(&image_path, &written).unwrap();
    assert_eq!(reloaded.format, FloppyFormat::D77);
}

#[test]
fn d88_2hd_with_extra_tracks_mounts_through_the_default_bpb() {
    let folder = TemporaryFolder::new("copy_d88_extra_tracks");
    let dir = folder.path();
    let mut volume = build_fat12_volume(&LAYOUT_2HD, BootSector::Blank);
    add_root_file(
        &mut volume,
        &LAYOUT_2HD,
        0,
        b"DEFAULT TXT",
        &[],
        2,
        b"default bpb",
    );
    let bytes = build_layout_d88(
        &LAYOUT_2HD,
        &volume,
        D88MediaType::Disk2HD,
        standard_sector_id,
        4,
    );
    let image_path = dir.join("disk.d88");
    std::fs::write(&image_path, &bytes).unwrap();

    let output = dir.join("output.txt");
    copy(
        image(&image_path, b"A:\\DEFAULT.TXT".to_vec()),
        host(&output),
    )
    .expect("the volume mounts through the default 2HD BPB");
    assert_eq!(std::fs::read(&output).unwrap(), b"default bpb");
    assert_file_roundtrip(dir, &image_path, &payload(PAYLOAD_LENGTH));
}

#[test]
fn d88_dump_of_a_2hs_disk_reads_a_file() {
    let folder = TemporaryFolder::new("copy_d88_2hs");
    let dir = folder.path();
    let mut volume = build_fat12_volume(&LAYOUT_2HS, BootSector::Human68kHudson);
    let data = payload(PAYLOAD_LENGTH);
    add_root_file(&mut volume, &LAYOUT_2HS, 0, b"DUMP    BIN", &[], 2, &data);
    let bytes = build_layout_d88(
        &LAYOUT_2HS,
        &volume,
        D88MediaType::Disk2HD,
        sector_id_2hs,
        0,
    );
    let image_path = dir.join("disk.d88");
    std::fs::write(&image_path, &bytes).unwrap();

    let output = dir.join("output.bin");
    copy(image(&image_path, b"A:\\DUMP.BIN".to_vec()), host(&output)).expect("read the file");
    assert_eq!(std::fs::read(&output).unwrap(), data);
}

#[test]
fn nfd_r1_write_updates_every_retry_copy() {
    let folder = TemporaryFolder::new("copy_nfd_r1");
    let dir = folder.path();
    let volume = build_fat12_volume(&LAYOUT_2HD, BootSector::Dos);
    let (bytes, data_offset) = build_layout_nfd_r1(&LAYOUT_2HD, &volume);
    let image_path = dir.join("disk.nfd");
    std::fs::write(&image_path, &bytes).unwrap();

    assert_file_roundtrip(dir, &image_path, &payload(PAYLOAD_LENGTH));

    let written = std::fs::read(&image_path).unwrap();
    let sector_size = LAYOUT_2HD.sector_size;
    let file_sector = LAYOUT_2HD.cluster_offset(2) / sector_size;
    let primary = data_offset + file_sector * 2 * sector_size;
    assert_eq!(
        written[primary..primary + sector_size],
        payload(PAYLOAD_LENGTH)[..sector_size]
    );
    for pair in written[data_offset..].chunks_exact(2 * sector_size) {
        assert_eq!(pair[..sector_size], pair[sector_size..]);
    }
}

#[test]
fn d77_without_fat_volume_fails_with_a_clear_message() {
    let folder = TemporaryFolder::new("copy_d77_no_fat");
    let dir = folder.path();
    let volume = vec![0u8; LAYOUT_2D_256.total_sectors() * LAYOUT_2D_256.sector_size];
    let bytes = build_layout_d88(
        &LAYOUT_2D_256,
        &volume,
        D88MediaType::Disk2D,
        standard_sector_id,
        0,
    );
    let image_path = dir.join("basic.d77");
    std::fs::write(&image_path, &bytes).unwrap();

    let output = dir.join("output");
    let error = copy(image(&image_path, b"A:\\".to_vec()), host(&output))
        .expect_err("the disk holds no FAT volume");
    let message = error.to_string();
    assert!(
        message.contains("basic.d77 holds no FAT file system"),
        "{message}"
    );
    assert!(!output.exists());
}

/// Builds a Human68k 2HD volume with the 18.3 names `LONGNAME1A.X` and `LONGNAME2B.X` after a
/// deleted slot.
fn build_human68k_long_name_volume() -> Vec<u8> {
    let mut volume = build_fat12_volume(&LAYOUT_2HD, BootSector::Human68kIpl);
    volume[LAYOUT_2HD.root_directory_offset()] = 0xE5;
    add_root_file(
        &mut volume,
        &LAYOUT_2HD,
        1,
        b"LONGNAMEX  ",
        b"1A",
        2,
        b"first long name",
    );
    add_root_file(
        &mut volume,
        &LAYOUT_2HD,
        2,
        b"LONGNAMEX  ",
        b"2B",
        3,
        b"second long name",
    );
    volume
}

#[test]
fn human68k_long_names_extract_with_their_own_contents() {
    let folder = TemporaryFolder::new("copy_human68k_names");
    let dir = folder.path();
    let image_path = dir.join("disk.xdf");
    std::fs::write(&image_path, build_human68k_long_name_volume()).unwrap();

    let output = dir.join("output");
    std::fs::create_dir_all(&output).unwrap();
    copy(image(&image_path, b"A:\\".to_vec()), host(&output)).expect("extract the root");
    assert_eq!(
        std::fs::read(output.join("LONGNAME1A.X")).unwrap(),
        b"first long name"
    );
    assert_eq!(
        std::fs::read(output.join("LONGNAME2B.X")).unwrap(),
        b"second long name"
    );
}

#[test]
fn human68k_overwrite_keeps_the_slot_and_the_name_continuation() {
    let folder = TemporaryFolder::new("copy_human68k_overwrite");
    let dir = folder.path();
    let image_path = dir.join("disk.xdf");
    let original = build_human68k_long_name_volume();
    std::fs::write(&image_path, &original).unwrap();
    let source = dir.join("source.bin");
    std::fs::write(&source, b"replacement").unwrap();

    copy(
        host(&source),
        image(&image_path, b"A:\\LONGNAME.X".to_vec()),
    )
    .expect("overwrite");

    let written = std::fs::read(&image_path).unwrap();
    let root = LAYOUT_2HD.root_directory_offset();
    assert_eq!(written[root], 0xE5);
    let entry = &written[root + DIRECTORY_ENTRY_SIZE..root + 2 * DIRECTORY_ENTRY_SIZE];
    let original_entry = &original[root + DIRECTORY_ENTRY_SIZE..root + 2 * DIRECTORY_ENTRY_SIZE];
    assert_eq!(entry[..11], *b"LONGNAMEX  ");
    assert_eq!(entry[12..22], original_entry[12..22]);
    assert_eq!(entry[28..32], 11u32.to_le_bytes());

    let output = dir.join("output");
    std::fs::create_dir_all(&output).unwrap();
    copy(image(&image_path, b"A:\\".to_vec()), host(&output)).expect("extract the root");
    assert_eq!(
        std::fs::read(output.join("LONGNAME1A.X")).unwrap(),
        b"replacement"
    );
    assert_eq!(
        std::fs::read(output.join("LONGNAME2B.X")).unwrap(),
        b"second long name"
    );
}

#[test]
fn shift_jis_names_extract_with_their_decoded_names() {
    let folder = TemporaryFolder::new("copy_shift_jis");
    let dir = folder.path();
    let mut volume = build_fat12_volume(&LAYOUT_2HD, BootSector::Dos);
    add_root_file(
        &mut volume,
        &LAYOUT_2HD,
        0,
        b"\x83\x47\x83\x66\x83\x42\x83\x5EX  ",
        &[],
        2,
        b"editor",
    );
    add_root_file(
        &mut volume,
        &LAYOUT_2HD,
        1,
        b"\x95\x5C      X  ",
        &[],
        3,
        b"table",
    );
    let image_path = dir.join("disk.hdm");
    std::fs::write(&image_path, &volume).unwrap();

    let output = dir.join("output");
    std::fs::create_dir_all(&output).unwrap();
    copy(image(&image_path, b"A:\\".to_vec()), host(&output)).expect("extract the root");
    assert_eq!(
        std::fs::read(output.join("\u{30A8}\u{30C7}\u{30A3}\u{30BF}.X")).unwrap(),
        b"editor"
    );
    assert_eq!(std::fs::read(output.join("\u{8868}.X")).unwrap(), b"table");
}

#[test]
fn human68k_character_across_base_and_continuation_extracts() {
    let folder = TemporaryFolder::new("copy_human68k_spanning");
    let dir = folder.path();
    let mut volume = build_fat12_volume(&LAYOUT_2HD, BootSector::Human68kIpl);
    add_root_file(
        &mut volume,
        &LAYOUT_2HD,
        0,
        b"\xCA\xDF\xC0\xB0\xDD\x88\xEA\x97X  ",
        b"\x97",
        2,
        b"pattern list",
    );
    let image_path = dir.join("disk.xdf");
    std::fs::write(&image_path, &volume).unwrap();

    let output = dir.join("output");
    std::fs::create_dir_all(&output).unwrap();
    copy(image(&image_path, b"A:\\".to_vec()), host(&output)).expect("extract the root");
    assert_eq!(
        std::fs::read(output.join("\u{FF8A}\u{FF9F}\u{FF80}\u{FF70}\u{FF9D}\u{4E00}\u{89A7}.X"))
            .unwrap(),
        b"pattern list"
    );
}

#[test]
fn directory_cycle_fails_before_writing_host_files() {
    let folder = TemporaryFolder::new("copy_directory_cycle");
    let dir = folder.path();
    let mut volume = build_fat12_volume(&LAYOUT_2HD, BootSector::Dos);
    add_root_file(&mut volume, &LAYOUT_2HD, 0, b"FILE    TXT", &[], 2, b"file");
    let subdirectory_cluster = 3;
    add_directory_entry(
        &mut volume,
        LAYOUT_2HD.root_directory_offset() + DIRECTORY_ENTRY_SIZE,
        b"SUB        ",
        0x10,
        &[],
        subdirectory_cluster,
        0,
    );
    set_layout_fat_entry(&mut volume, &LAYOUT_2HD, subdirectory_cluster, 0x0FFF);
    add_directory_entry(
        &mut volume,
        LAYOUT_2HD.cluster_offset(subdirectory_cluster),
        b"LOOP       ",
        0x10,
        &[],
        subdirectory_cluster,
        0,
    );
    let image_path = dir.join("disk.hdm");
    std::fs::write(&image_path, &volume).unwrap();

    let output = dir.join("output");
    std::fs::create_dir_all(&output).unwrap();
    let error = copy(image(&image_path, b"A:\\".to_vec()), host(&output))
        .expect_err("the directory tree holds a cycle");
    let message = error.to_string();
    assert!(message.contains("directory cycle"), "{message}");
    assert!(message.contains("A:\\SUB\\LOOP"), "{message}");
    assert_eq!(std::fs::read_dir(&output).unwrap().count(), 0);
}

#[test]
fn image_name_that_is_no_host_file_name_fails_before_writing() {
    let folder = TemporaryFolder::new("copy_invalid_host_name");
    let dir = folder.path();
    let mut volume = build_fat12_volume(&LAYOUT_2HD, BootSector::Dos);
    add_root_file(&mut volume, &LAYOUT_2HD, 0, b"GOOD    TXT", &[], 2, b"good");
    add_root_file(
        &mut volume,
        &LAYOUT_2HD,
        1,
        b"BAD\x7F    TXT",
        &[],
        3,
        b"bad",
    );
    let image_path = dir.join("disk.hdm");
    std::fs::write(&image_path, &volume).unwrap();

    let output = dir.join("output");
    std::fs::create_dir_all(&output).unwrap();
    let error = copy(image(&image_path, b"A:\\".to_vec()), host(&output))
        .expect_err("the name holds a control character");
    let message = error.to_string();
    assert!(message.contains("A:\\BAD\\x7F.TXT"), "{message}");
    assert_eq!(std::fs::read_dir(&output).unwrap().count(), 0);
}

/// Cluster of the `TREE` directory of the tree volumes.
const TREE_DIRECTORY_CLUSTER: u16 = 2;

/// Builds a 2HD volume with `boot` and the directory `A:\TREE`. The entries
/// `(fcb_name, continuation, attribute, start_cluster)` of `entries` fill its
/// slots, each with a size of 0.
fn build_tree_volume(boot: BootSector, entries: &[(&[u8; 11], &[u8], u8, u16)]) -> Vec<u8> {
    let mut volume = build_fat12_volume(&LAYOUT_2HD, boot);
    add_directory_entry(
        &mut volume,
        LAYOUT_2HD.root_directory_offset(),
        b"TREE       ",
        0x10,
        &[],
        TREE_DIRECTORY_CLUSTER,
        0,
    );
    set_layout_fat_entry(&mut volume, &LAYOUT_2HD, TREE_DIRECTORY_CLUSTER, 0x0FFF);
    let tree_offset = LAYOUT_2HD.cluster_offset(TREE_DIRECTORY_CLUSTER);
    for (slot, (fcb_name, continuation, attribute, start_cluster)) in entries.iter().enumerate() {
        add_directory_entry(
            &mut volume,
            tree_offset + slot * DIRECTORY_ENTRY_SIZE,
            fcb_name,
            *attribute,
            continuation,
            *start_cluster,
            0,
        );
    }
    volume
}

#[test]
fn image_to_image_copy_of_names_with_one_8_3_name_fails_without_writing() {
    let folder = TemporaryFolder::new("copy_image_8_3_collision");
    let dir = folder.path();
    let source_image = dir.join("source.xdf");
    let volume = build_tree_volume(
        BootSector::Human68kIpl,
        &[
            (b"LONGNAMEX  ", b"1A", 0x20, 0),
            (b"LONGNAMEX  ", b"2B", 0x20, 0),
        ],
    );
    std::fs::write(&source_image, volume).unwrap();
    let dest_image = make_blank_image(dir, "dest.hdm");
    let before = std::fs::read(&dest_image).unwrap();

    let error = copy(
        image(&source_image, b"A:\\TREE".to_vec()),
        image(&dest_image, b"A:\\COPIED".to_vec()),
    )
    .expect_err("both names write the 8.3 name LONGNAME.X");
    let message = error.to_string();
    assert!(message.contains("A:\\TREE\\LONGNAME1A.X"), "{message}");
    assert!(message.contains("A:\\TREE\\LONGNAME2B.X"), "{message}");
    assert!(message.contains("no file was written"), "{message}");
    assert_eq!(std::fs::read(&dest_image).unwrap(), before);
}

#[test]
fn host_names_equal_ignoring_the_case_fail_before_writing() {
    let folder = TemporaryFolder::new("copy_host_case_collision");
    let dir = folder.path();
    let image_path = dir.join("disk.xdf");
    let volume = build_tree_volume(
        BootSector::Human68kIpl,
        &[
            (b"LONGNAMEX  ", b"ab", 0x20, 0),
            (b"LONGNAMEX  ", b"AB", 0x20, 0),
        ],
    );
    std::fs::write(&image_path, volume).unwrap();

    let output = dir.join("output");
    std::fs::create_dir_all(&output).unwrap();
    let error = copy(image(&image_path, b"A:\\".to_vec()), host(&output))
        .expect_err("the host names are equal ignoring the case");
    let message = error.to_string();
    assert!(message.contains("equal ignoring the case"), "{message}");
    assert_eq!(std::fs::read_dir(&output).unwrap().count(), 0);
}

#[test]
fn subdirectory_with_an_invalid_start_cluster_names_its_path() {
    let folder = TemporaryFolder::new("copy_invalid_start_cluster");
    let dir = folder.path();
    let image_path = dir.join("disk.hdm");
    let volume = build_tree_volume(BootSector::Dos, &[(b"BROKEN     ", b"", 0x10, 0x0FF0)]);
    std::fs::write(&image_path, volume).unwrap();

    let output = dir.join("output");
    std::fs::create_dir_all(&output).unwrap();
    let error = copy(image(&image_path, b"A:\\TREE".to_vec()), host(&output))
        .expect_err("the subdirectory starts outside the data area");
    let message = error.to_string();
    assert!(message.contains("A:\\TREE\\BROKEN"), "{message}");
    assert_eq!(std::fs::read_dir(&output).unwrap().count(), 0);
}
