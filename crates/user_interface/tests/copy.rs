//! End-to-end tests for the `neetan copy` subcommand.

use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use device::{
    disk::{HddFormat, HddGeometry, HddImage},
    floppy::{
        FloppyImage,
        d88::{D88Disk, D88MediaType, D88Sector},
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

/// Returns a unique tempdir for a test, creating it on disk.
fn unique_tempdir(label: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let pid = std::process::id();
    let counter = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("neetan_copy_{label}_{pid}_{counter}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
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
    let dir = unique_tempdir("roundtrip");
    let image_path = make_blank_image(&dir, "disk.hdm");
    let src = dir.join("input.txt");
    std::fs::write(&src, b"hello round trip\n").unwrap();

    copy(host(&src), image(&image_path, b"A:\\OUT.TXT".to_vec())).expect("write into image");

    let out = dir.join("out.txt");
    copy(image(&image_path, b"A:\\OUT.TXT".to_vec()), host(&out)).expect("read from image");

    assert_eq!(std::fs::read(&out).unwrap(), b"hello round trip\n",);
}

#[test]
fn host_directory_is_copied_recursively_with_subdirs() {
    let dir = unique_tempdir("recursive");
    let image_path = make_blank_image(&dir, "disk.hdm");
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
    let dir = unique_tempdir("mkdir_p");
    let image_path = make_blank_image(&dir, "disk.hdm");
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
    let dir = unique_tempdir("img2img");
    let source_image = make_blank_image(&dir, "src.hdm");
    let dest_image = make_blank_image(&dir, "dst.hdm");

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
    let dir = unique_tempdir("hh_reject");
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
    let dir = unique_tempdir("preflight");
    let image_path = make_blank_image(&dir, "disk.hdm");
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
    let dir = unique_tempdir("missing");
    let image_path = make_blank_image(&dir, "disk.hdm");
    let out = dir.join("out.txt");
    let result = copy(image(&image_path, b"A:\\NOSUCH.TXT".to_vec()), host(&out));
    assert!(result.is_err());
    assert!(!out.exists());
}

#[test]
fn unknown_image_extension_is_rejected() {
    let dir = unique_tempdir("ext");
    let bogus = dir.join("nope.xyz");
    std::fs::write(&bogus, b"not an image").unwrap();
    let out = dir.join("out.txt");
    let result = copy(image(&bogus, b"A:\\X.TXT".to_vec()), host(&out));
    assert!(result.is_err());
}

#[test]
fn overwriting_an_existing_file_replaces_its_contents() {
    let dir = unique_tempdir("overwrite");
    let image_path = make_blank_image(&dir, "disk.hdm");
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
    let dir = unique_tempdir("lower");
    let image_path = make_blank_image(&dir, "disk.hdm");
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
    let dir = unique_tempdir("sizes");
    let image_path = make_blank_image(&dir, "disk.hdm");
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
    let dir = unique_tempdir("empty_dirs");
    let source_image = make_blank_image(&dir, "src.hdm");
    let dest_image = make_blank_image(&dir, "dst.hdm");
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
    let dir = unique_tempdir("existing_dirs");
    let image_path = make_blank_image(&dir, "disk.hdm");
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
    let dir = unique_tempdir("slash_missing");
    let image_path = make_blank_image(&dir, "disk.hdm");
    let src = dir.join("input.txt");
    std::fs::write(&src, b"input").unwrap();

    let before = std::fs::read(&image_path).unwrap();
    let result = copy(host(&src), image(&image_path, b"A:\\MISSING\\".to_vec()));
    assert!(result.is_err());
    assert_eq!(std::fs::read(&image_path).unwrap(), before);
}

#[test]
fn file_parent_component_rejects_copy_without_image_mutation() {
    let dir = unique_tempdir("file_parent");
    let image_path = make_blank_image(&dir, "disk.hdm");
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
    let dir = unique_tempdir("dir_to_file");
    let image_path = make_blank_image(&dir, "disk.hdm");
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
    let dir = unique_tempdir("image_dir_to_file");
    let image_path = make_blank_image(&dir, "disk.hdm");
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
    let dir = unique_tempdir("disk_full");
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
    let dir = unique_tempdir("bad_source_host");
    let image_path = make_blank_image(&dir, "disk.hdm");
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
    let dir = unique_tempdir("bad_source_image");
    let source_image = make_blank_image(&dir, "src.hdm");
    let dest_image = make_blank_image(&dir, "dst.hdm");
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
    let dir = unique_tempdir("collisions");
    let image_path = make_blank_image(&dir, "disk.hdm");
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
    let dir = unique_tempdir("d88");
    let image_path = make_blank_d88_image(&dir, "disk.d88");
    let src = dir.join("source.txt");
    let out = dir.join("out.txt");
    std::fs::write(&src, b"d88 data").unwrap();

    copy(host(&src), image(&image_path, b"A:\\D88.TXT".to_vec())).unwrap();
    copy(image(&image_path, b"A:\\D88.TXT".to_vec()), host(&out)).unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), b"d88 data");
}

#[test]
fn partitioned_nhd_roundtrip_preserves_file_contents() {
    let dir = unique_tempdir("nhd");
    let image_path = make_blank_nhd_image(&dir, "disk.nhd");
    let src = dir.join("source.txt");
    let out = dir.join("out.txt");
    std::fs::write(&src, b"nhd data").unwrap();

    copy(host(&src), image(&image_path, b"A:\\NHD.TXT".to_vec())).unwrap();
    copy(image(&image_path, b"A:\\NHD.TXT".to_vec()), host(&out)).unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), b"nhd data");
}

#[test]
fn mbr_partitioned_at_hdd_roundtrip_preserves_file_contents() {
    let dir = unique_tempdir("athdd");
    let image_path = make_blank_at_hdd_image(&dir, "disk.hdd");
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
    let dir = unique_tempdir("imgfdd");
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
    let dir = unique_tempdir("msxdsk");
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
