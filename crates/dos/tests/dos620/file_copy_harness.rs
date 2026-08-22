use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use common::Bus;
use device::{
    disk::{self, HddImage},
    floppy::{
        FloppyImage,
        d88::{D88Disk, D88MediaType, D88Sector},
    },
};

use crate::harness::*;

pub const RANDOM_FILE_FCB: [u8; 11] = *b"RAND    BIN";
pub const RANDOM_FILE_SIZE: usize = 0x4977;
const FAT12_EOC: u16 = 0x0FFF;

#[derive(Debug, Clone, Copy)]
struct BpbInfo {
    partition_offset: u32,
    bytes_per_sector: u16,
    sectors_per_cluster: u8,
    reserved_sectors: u16,
    num_fats: u8,
    root_entry_count: u16,
    sectors_per_fat: u16,
    is_fat16: bool,
}

#[derive(Debug, Clone, Copy)]
struct DirectoryEntryInfo {
    attribute: u8,
    start_cluster: u16,
    file_size: u32,
}

pub fn prng_bytes(len: usize) -> Vec<u8> {
    let mut state = 0x1234_5678u32;
    let mut out = Vec::with_capacity(len);
    for _ in 0..len {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        out.push((state >> 16) as u8);
    }
    out
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

pub fn create_random_file_floppy(file_data: &[u8]) -> FloppyImage {
    create_random_file_floppy_with_name(&RANDOM_FILE_FCB, file_data)
}

pub fn create_random_file_floppy_with_name(fcb_name: &[u8; 11], file_data: &[u8]) -> FloppyImage {
    let cylinders = 77usize;
    let heads = 2usize;
    let sectors_per_track = 8usize;
    let sector_size = 1024usize;
    let total_tracks = cylinders * heads;
    let total_sectors = cylinders * heads * sectors_per_track;
    let mut disk_data = vec![0u8; total_sectors * sector_size];

    {
        let bpb = &mut disk_data[0..sector_size];
        bpb[0] = 0xEB;
        bpb[1] = 0x3C;
        bpb[2] = 0x90;
        bpb[3..11].copy_from_slice(b"NEETAN  ");
        bpb[11..13].copy_from_slice(&1024u16.to_le_bytes());
        bpb[13] = 1;
        bpb[14..16].copy_from_slice(&1u16.to_le_bytes());
        bpb[16] = 2;
        bpb[17..19].copy_from_slice(&192u16.to_le_bytes());
        bpb[19..21].copy_from_slice(&1232u16.to_le_bytes());
        bpb[21] = 0xFE;
        bpb[22..24].copy_from_slice(&2u16.to_le_bytes());
        bpb[24..26].copy_from_slice(&8u16.to_le_bytes());
        bpb[26..28].copy_from_slice(&2u16.to_le_bytes());
    }

    let fat1_offset = sector_size;
    let fat = &mut disk_data[fat1_offset..fat1_offset + 2 * sector_size];
    fat[0] = 0xFE;
    fat[1] = 0xFF;
    fat[2] = 0xFF;
    set_fat12_entry(fat, 2, FAT12_EOC);
    set_fat12_entry(fat, 3, FAT12_EOC);

    let random_cluster_count = file_data.len().div_ceil(sector_size);
    for index in 0..random_cluster_count {
        let cluster = 4 + index as u16;
        let next = if index + 1 == random_cluster_count {
            FAT12_EOC
        } else {
            cluster + 1
        };
        set_fat12_entry(fat, cluster, next);
    }

    let fat2_offset = 3 * sector_size;
    let fat_copy = disk_data[fat1_offset..fat1_offset + 2 * sector_size].to_vec();
    disk_data[fat2_offset..fat2_offset + fat_copy.len()].copy_from_slice(&fat_copy);

    let root_offset = 5 * sector_size;
    {
        let entry = &mut disk_data[root_offset..root_offset + 32];
        entry[0..11].copy_from_slice(b"COMMAND COM");
        entry[11] = 0x20;
        entry[22..24].copy_from_slice(&TEST_FILE_TIME.to_le_bytes());
        entry[24..26].copy_from_slice(&TEST_FILE_DATE.to_le_bytes());
        entry[26..28].copy_from_slice(&2u16.to_le_bytes());
        entry[28..32].copy_from_slice(&(TEST_COMMAND_COM.len() as u32).to_le_bytes());
    }
    {
        let entry = &mut disk_data[root_offset + 32..root_offset + 64];
        entry[0..11].copy_from_slice(b"TESTFILETXT");
        entry[11] = 0x20;
        entry[22..24].copy_from_slice(&TEST_FILE_TIME.to_le_bytes());
        entry[24..26].copy_from_slice(&TEST_FILE_DATE.to_le_bytes());
        entry[26..28].copy_from_slice(&3u16.to_le_bytes());
        entry[28..32].copy_from_slice(&(TEST_FILE_CONTENT.len() as u32).to_le_bytes());
    }
    {
        let entry = &mut disk_data[root_offset + 64..root_offset + 96];
        entry[0..11].copy_from_slice(fcb_name);
        entry[11] = 0x20;
        entry[22..24].copy_from_slice(&TEST_FILE_TIME.to_le_bytes());
        entry[24..26].copy_from_slice(&TEST_FILE_DATE.to_le_bytes());
        entry[26..28].copy_from_slice(&4u16.to_le_bytes());
        entry[28..32].copy_from_slice(&(file_data.len() as u32).to_le_bytes());
    }

    let cluster2_offset = 11 * sector_size;
    disk_data[cluster2_offset..cluster2_offset + TEST_COMMAND_COM.len()]
        .copy_from_slice(TEST_COMMAND_COM);
    let cluster3_offset = 12 * sector_size;
    disk_data[cluster3_offset..cluster3_offset + TEST_FILE_CONTENT.len()]
        .copy_from_slice(TEST_FILE_CONTENT);

    for (index, chunk) in file_data.chunks(sector_size).enumerate() {
        let lba = 13 + index;
        let offset = lba * sector_size;
        disk_data[offset..offset + chunk.len()].copy_from_slice(chunk);
    }

    let mut tracks = Vec::with_capacity(total_tracks);
    for track_index in 0..total_tracks {
        let cylinder = (track_index / heads) as u8;
        let head = (track_index % heads) as u8;
        let mut sectors = Vec::with_capacity(sectors_per_track);
        for sector in 0..sectors_per_track {
            let lba = track_index * sectors_per_track + sector;
            let offset = lba * sector_size;
            sectors.push(D88Sector {
                cylinder,
                head,
                record: (sector + 1) as u8,
                size_code: 3,
                sector_count: sectors_per_track as u16,
                mfm_flag: 0x40,
                deleted: 0,
                status: 0,
                reserved: [0; 5],
                data: disk_data[offset..offset + sector_size].to_vec(),
                source_offset: None,
            });
        }
        tracks.push(Some(sectors));
    }

    let d88 = D88Disk::from_tracks("XRAND".to_string(), false, D88MediaType::Disk2HD, tracks);
    FloppyImage::from_d88(d88)
}

pub fn create_broken_chain_floppy_with_name(
    fcb_name: &[u8; 11],
    advertised_size: usize,
) -> FloppyImage {
    let first_cluster_data = prng_bytes(1024);
    let cylinders = 77usize;
    let heads = 2usize;
    let sectors_per_track = 8usize;
    let sector_size = 1024usize;
    let total_tracks = cylinders * heads;
    let total_sectors = cylinders * heads * sectors_per_track;
    let mut disk_data = vec![0u8; total_sectors * sector_size];

    {
        let bpb = &mut disk_data[0..sector_size];
        bpb[0] = 0xEB;
        bpb[1] = 0x3C;
        bpb[2] = 0x90;
        bpb[3..11].copy_from_slice(b"NEETAN  ");
        bpb[11..13].copy_from_slice(&1024u16.to_le_bytes());
        bpb[13] = 1;
        bpb[14..16].copy_from_slice(&1u16.to_le_bytes());
        bpb[16] = 2;
        bpb[17..19].copy_from_slice(&192u16.to_le_bytes());
        bpb[19..21].copy_from_slice(&1232u16.to_le_bytes());
        bpb[21] = 0xFE;
        bpb[22..24].copy_from_slice(&2u16.to_le_bytes());
        bpb[24..26].copy_from_slice(&8u16.to_le_bytes());
        bpb[26..28].copy_from_slice(&2u16.to_le_bytes());
    }

    let fat1_offset = sector_size;
    let fat = &mut disk_data[fat1_offset..fat1_offset + 2 * sector_size];
    fat[0] = 0xFE;
    fat[1] = 0xFF;
    fat[2] = 0xFF;
    set_fat12_entry(fat, 2, FAT12_EOC);
    set_fat12_entry(fat, 3, FAT12_EOC);
    set_fat12_entry(fat, 4, FAT12_EOC);

    let fat2_offset = 3 * sector_size;
    let fat_copy = disk_data[fat1_offset..fat1_offset + 2 * sector_size].to_vec();
    disk_data[fat2_offset..fat2_offset + fat_copy.len()].copy_from_slice(&fat_copy);

    let root_offset = 5 * sector_size;
    {
        let entry = &mut disk_data[root_offset..root_offset + 32];
        entry[0..11].copy_from_slice(b"COMMAND COM");
        entry[11] = 0x20;
        entry[22..24].copy_from_slice(&TEST_FILE_TIME.to_le_bytes());
        entry[24..26].copy_from_slice(&TEST_FILE_DATE.to_le_bytes());
        entry[26..28].copy_from_slice(&2u16.to_le_bytes());
        entry[28..32].copy_from_slice(&(TEST_COMMAND_COM.len() as u32).to_le_bytes());
    }
    {
        let entry = &mut disk_data[root_offset + 32..root_offset + 64];
        entry[0..11].copy_from_slice(b"TESTFILETXT");
        entry[11] = 0x20;
        entry[22..24].copy_from_slice(&TEST_FILE_TIME.to_le_bytes());
        entry[24..26].copy_from_slice(&TEST_FILE_DATE.to_le_bytes());
        entry[26..28].copy_from_slice(&3u16.to_le_bytes());
        entry[28..32].copy_from_slice(&(TEST_FILE_CONTENT.len() as u32).to_le_bytes());
    }
    {
        let entry = &mut disk_data[root_offset + 64..root_offset + 96];
        entry[0..11].copy_from_slice(fcb_name);
        entry[11] = 0x20;
        entry[22..24].copy_from_slice(&TEST_FILE_TIME.to_le_bytes());
        entry[24..26].copy_from_slice(&TEST_FILE_DATE.to_le_bytes());
        entry[26..28].copy_from_slice(&4u16.to_le_bytes());
        entry[28..32].copy_from_slice(&(advertised_size as u32).to_le_bytes());
    }

    let cluster2_offset = 11 * sector_size;
    disk_data[cluster2_offset..cluster2_offset + TEST_COMMAND_COM.len()]
        .copy_from_slice(TEST_COMMAND_COM);
    let cluster3_offset = 12 * sector_size;
    disk_data[cluster3_offset..cluster3_offset + TEST_FILE_CONTENT.len()]
        .copy_from_slice(TEST_FILE_CONTENT);
    let cluster4_offset = 13 * sector_size;
    disk_data[cluster4_offset..cluster4_offset + first_cluster_data.len()]
        .copy_from_slice(&first_cluster_data);

    let mut tracks = Vec::with_capacity(total_tracks);
    for track_index in 0..total_tracks {
        let cylinder = (track_index / heads) as u8;
        let head = (track_index % heads) as u8;
        let mut sectors = Vec::with_capacity(sectors_per_track);
        for sector in 0..sectors_per_track {
            let lba = track_index * sectors_per_track + sector;
            let offset = lba * sector_size;
            sectors.push(D88Sector {
                cylinder,
                head,
                record: (sector + 1) as u8,
                size_code: 3,
                sector_count: sectors_per_track as u16,
                mfm_flag: 0x40,
                deleted: 0,
                status: 0,
                reserved: [0; 5],
                data: disk_data[offset..offset + sector_size].to_vec(),
                source_offset: None,
            });
        }
        tracks.push(Some(sectors));
    }

    let d88 = D88Disk::from_tracks("BROKEN".to_string(), false, D88MediaType::Disk2HD, tracks);
    FloppyImage::from_d88(d88)
}

pub fn make_temp_hdd_path(prefix: &str) -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should be after unix epoch");
    std::env::temp_dir().join(format!(
        "neetan-{prefix}-{}-{}.nhd",
        std::process::id(),
        now.as_nanos()
    ))
}

pub fn load_hdd_from_path(path: &Path) -> HddImage {
    let bytes = fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    disk::load_hdd_image(path, &bytes)
        .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

pub fn boot_hle_with_temp_hdd_and_floppy(
    hdd_path: &Path,
    floppy: FloppyImage,
) -> machine_98::Pc9801Ra {
    let mut machine = create_hle_machine();
    machine.bus.write_byte(0x055C, 0x01);
    machine.bus.write_byte(0x055D, 0x01);
    machine.bus.insert_hdd(
        0,
        load_hdd_from_path(hdd_path),
        Some(hdd_path.to_path_buf()),
    );

    let mut total_cycles = 0u64;
    loop {
        total_cycles += machine.run_for(1_000_000);
        if hle_prompt_visible(&machine.bus) {
            break;
        }
        assert!(
            total_cycles < 500_000_000,
            "HLE DOS did not show prompt with HDD+FDD test setup"
        );
    }

    machine.bus.insert_floppy(0, floppy, None);
    machine
}

fn find_partition_offset(hdd: &HddImage) -> u32 {
    let sector = hdd.read_sector(1).expect("partition table sector");
    for entry in sector.as_chunks::<32>().0.iter().take(16) {
        if entry[0] == 0 && entry[1] == 0 {
            break;
        }
        let mid = entry[0];
        let sid = entry[1];
        if sid & 0x80 == 0 || mid & 0x70 != 0x20 {
            continue;
        }
        let start_sector = entry[8] as u32;
        let start_head = entry[9] as u32;
        let start_cylinder = u16::from_le_bytes([entry[10], entry[11]]) as u32;
        return (start_cylinder * hdd.geometry.heads as u32 + start_head)
            * hdd.geometry.sectors_per_track as u32
            + start_sector;
    }
    0
}

fn read_bpb(hdd: &HddImage) -> BpbInfo {
    let partition_offset = find_partition_offset(hdd);
    let boot = hdd
        .read_sector(partition_offset)
        .expect("partition boot sector");
    let total_sectors_16 = u16::from_le_bytes([boot[19], boot[20]]) as u32;
    let total_sectors_32 = u32::from_le_bytes([boot[32], boot[33], boot[34], boot[35]]);
    let total_sectors = if total_sectors_16 != 0 {
        total_sectors_16
    } else {
        total_sectors_32
    };
    let reserved_sectors = u16::from_le_bytes([boot[14], boot[15]]);
    let num_fats = boot[16];
    let root_entry_count = u16::from_le_bytes([boot[17], boot[18]]);
    let sectors_per_fat = u16::from_le_bytes([boot[22], boot[23]]);
    let bytes_per_sector = u16::from_le_bytes([boot[11], boot[12]]);
    let sectors_per_cluster = boot[13];
    let root_dir_sectors = (root_entry_count as u32 * 32).div_ceil(bytes_per_sector as u32);
    let first_data_sector =
        reserved_sectors as u32 + num_fats as u32 * sectors_per_fat as u32 + root_dir_sectors;
    let data_cluster_count =
        total_sectors.saturating_sub(first_data_sector) / sectors_per_cluster as u32;
    BpbInfo {
        partition_offset,
        bytes_per_sector,
        sectors_per_cluster,
        reserved_sectors,
        num_fats,
        root_entry_count,
        sectors_per_fat,
        is_fat16: data_cluster_count >= 4085,
    }
}

fn read_logical_sector(hdd: &HddImage, bpb: &BpbInfo, lba: u32) -> Option<Vec<u8>> {
    let physical = hdd.geometry.sector_size as usize;
    let logical = bpb.bytes_per_sector as usize;
    let ratio = logical / physical;
    let mut data = Vec::with_capacity(logical);
    for part in 0..ratio as u32 {
        data.extend_from_slice(hdd.read_sector(bpb.partition_offset + lba * ratio as u32 + part)?);
    }
    Some(data)
}

fn read_fat_entry(fat: &[u8], cluster: u16, is_fat16: bool) -> u16 {
    if is_fat16 {
        let offset = cluster as usize * 2;
        u16::from_le_bytes([fat[offset], fat[offset + 1]])
    } else {
        let offset = (cluster as usize * 3) / 2;
        let pair = u16::from_le_bytes([fat[offset], fat[offset + 1]]);
        if cluster & 1 == 0 {
            pair & 0x0FFF
        } else {
            pair >> 4
        }
    }
}

fn root_directory_sectors(bpb: &BpbInfo) -> u32 {
    (bpb.root_entry_count as u32 * 32).div_ceil(bpb.bytes_per_sector as u32)
}

fn fat_start_sector(bpb: &BpbInfo) -> u32 {
    bpb.reserved_sectors as u32
}

fn root_start_sector(bpb: &BpbInfo) -> u32 {
    fat_start_sector(bpb) + bpb.num_fats as u32 * bpb.sectors_per_fat as u32
}

fn data_start_sector(bpb: &BpbInfo) -> u32 {
    root_start_sector(bpb) + root_directory_sectors(bpb)
}

fn fat_end_of_chain(bpb: &BpbInfo) -> u16 {
    if bpb.is_fat16 { 0xFFF8 } else { 0x0FF8 }
}

fn read_file_allocation_table(hard_disk: &HddImage, bpb: &BpbInfo) -> Result<Vec<u8>, String> {
    let fat_start = fat_start_sector(bpb);
    let mut fat = Vec::with_capacity(bpb.sectors_per_fat as usize * bpb.bytes_per_sector as usize);
    for sector in 0..bpb.sectors_per_fat as u32 {
        let bytes = read_logical_sector(hard_disk, bpb, fat_start + sector)
            .ok_or_else(|| format!("failed to read FAT logical sector {}", fat_start + sector))?;
        fat.extend_from_slice(&bytes);
    }
    Ok(fat)
}

fn read_root_directory(hard_disk: &HddImage, bpb: &BpbInfo) -> Result<Vec<u8>, String> {
    let root_start = root_start_sector(bpb);
    let root_sectors = root_directory_sectors(bpb);
    let mut directory = Vec::with_capacity(root_sectors as usize * bpb.bytes_per_sector as usize);
    for sector in 0..root_sectors {
        let bytes = read_logical_sector(hard_disk, bpb, root_start + sector).ok_or_else(|| {
            format!(
                "failed to read root directory logical sector {}",
                root_start + sector
            )
        })?;
        directory.extend_from_slice(&bytes);
    }
    Ok(directory)
}

fn find_directory_entry(directory: &[u8], name: &[u8; 11]) -> Option<DirectoryEntryInfo> {
    for entry in directory.as_chunks::<32>().0 {
        if entry[0] == 0x00 {
            break;
        }
        if entry[0] == 0xE5 {
            continue;
        }
        if entry[0..11] == name[..] {
            return Some(DirectoryEntryInfo {
                attribute: entry[11],
                start_cluster: u16::from_le_bytes([entry[26], entry[27]]),
                file_size: u32::from_le_bytes([entry[28], entry[29], entry[30], entry[31]]),
            });
        }
    }
    None
}

fn fcb_name_from_component(component: &str) -> Result<[u8; 11], String> {
    let (stem, extension) = match component.split_once('.') {
        Some((stem, extension)) => (stem, extension),
        None => (component, ""),
    };
    if stem.is_empty() || stem.len() > 8 || extension.len() > 3 {
        return Err(format!("invalid 8.3 component {component}"));
    }

    let mut name = [b' '; 11];
    for (index, byte) in stem.bytes().enumerate() {
        if !byte.is_ascii() {
            return Err(format!("non-ASCII component {component}"));
        }
        name[index] = byte.to_ascii_uppercase();
    }
    for (index, byte) in extension.bytes().enumerate() {
        if !byte.is_ascii() {
            return Err(format!("non-ASCII component {component}"));
        }
        name[8 + index] = byte.to_ascii_uppercase();
    }
    Ok(name)
}

fn read_cluster_chain(
    hard_disk: &HddImage,
    bpb: &BpbInfo,
    fat: &[u8],
    start_cluster: u16,
    expected_size: Option<u32>,
) -> Result<Vec<u8>, String> {
    if start_cluster < 2 {
        if expected_size == Some(0) {
            return Ok(Vec::new());
        }
        return Err("cluster chain has no start cluster".to_string());
    }

    let data_start = data_start_sector(bpb);
    let expected_len = expected_size.map(|size| size as usize);
    let mut cluster = start_cluster;
    let mut data = Vec::with_capacity(expected_len.unwrap_or(0));
    let mut chain = Vec::new();
    let end_of_chain = fat_end_of_chain(bpb);

    while cluster >= 2 && cluster < end_of_chain {
        chain.push(cluster);
        let first_sector = data_start + (cluster as u32 - 2) * bpb.sectors_per_cluster as u32;
        for sector in 0..bpb.sectors_per_cluster as u32 {
            let logical_sector = first_sector + sector;
            let bytes = read_logical_sector(hard_disk, bpb, logical_sector).ok_or_else(|| {
                format!(
                    "cluster chain left disk: cluster={cluster:04X} logical_sector={logical_sector} chain={chain:?}"
                )
            })?;
            data.extend_from_slice(&bytes);
            if let Some(expected_len) = expected_len
                && data.len() >= expected_len
            {
                data.truncate(expected_len);
                return Ok(data);
            }
        }
        let next = read_fat_entry(fat, cluster, bpb.is_fat16);
        if next == 0 || next == cluster {
            break;
        }
        cluster = next;
    }

    if let Some(expected_len) = expected_len {
        if data.len() < expected_len {
            return Err(format!(
                "cluster chain ended early: expected {expected_len} bytes, got {} bytes, chain={chain:?}",
                data.len()
            ));
        }
        data.truncate(expected_len);
    }
    Ok(data)
}

fn read_subdirectory(
    hard_disk: &HddImage,
    bpb: &BpbInfo,
    fat: &[u8],
    entry: DirectoryEntryInfo,
) -> Result<Vec<u8>, String> {
    if entry.attribute & 0x10 == 0 {
        return Err("path component is not a directory".to_string());
    }
    read_cluster_chain(hard_disk, bpb, fat, entry.start_cluster, None)
}

fn find_hard_disk_entry(
    hard_disk_path: &Path,
    components: &[&str],
) -> Result<DirectoryEntryInfo, String> {
    if components.is_empty() {
        return Err("path must contain at least one component".to_string());
    }

    let hard_disk = load_hdd_from_path(hard_disk_path);
    let bpb = read_bpb(&hard_disk);
    let fat = read_file_allocation_table(&hard_disk, &bpb)?;
    let mut directory = read_root_directory(&hard_disk, &bpb)?;
    let mut found = None;

    for (index, component) in components.iter().enumerate() {
        let name = fcb_name_from_component(component)?;
        let entry = find_directory_entry(&directory, &name)
            .ok_or_else(|| format!("path component not found: {component}"))?;
        if index + 1 == components.len() {
            found = Some(entry);
            break;
        }
        directory = read_subdirectory(&hard_disk, &bpb, &fat, entry)?;
    }

    found.ok_or_else(|| "path not found".to_string())
}

pub fn extract_hard_disk_file(
    hard_disk_path: &Path,
    components: &[&str],
) -> Result<Vec<u8>, String> {
    let hard_disk = load_hdd_from_path(hard_disk_path);
    let bpb = read_bpb(&hard_disk);
    let fat = read_file_allocation_table(&hard_disk, &bpb)?;
    let mut directory = read_root_directory(&hard_disk, &bpb)?;

    for (index, component) in components.iter().enumerate() {
        let name = fcb_name_from_component(component)?;
        let entry = find_directory_entry(&directory, &name)
            .ok_or_else(|| format!("path component not found: {component}"))?;
        if index + 1 == components.len() {
            if entry.attribute & 0x10 != 0 {
                return Err(format!("path component is a directory: {component}"));
            }
            return read_cluster_chain(
                &hard_disk,
                &bpb,
                &fat,
                entry.start_cluster,
                Some(entry.file_size),
            );
        }
        directory = read_subdirectory(&hard_disk, &bpb, &fat, entry)?;
    }

    Err("path must contain at least one component".to_string())
}

pub fn hard_disk_directory_exists(
    hard_disk_path: &Path,
    components: &[&str],
) -> Result<bool, String> {
    match find_hard_disk_entry(hard_disk_path, components) {
        Ok(entry) => Ok(entry.attribute & 0x10 != 0),
        Err(error) if error.starts_with("path component not found:") => Ok(false),
        Err(error) => Err(error),
    }
}

pub fn extract_root_file(hdd_path: &Path, fcb_name: &[u8; 11]) -> Result<Vec<u8>, String> {
    let hard_disk = load_hdd_from_path(hdd_path);
    let bpb = read_bpb(&hard_disk);
    let fat = read_file_allocation_table(&hard_disk, &bpb)?;
    let root = read_root_directory(&hard_disk, &bpb)?;
    let entry = find_directory_entry(&root, fcb_name)
        .ok_or_else(|| "destination file not found".to_string())?;
    read_cluster_chain(
        &hard_disk,
        &bpb,
        &fat,
        entry.start_cluster,
        Some(entry.file_size),
    )
}

pub fn mismatch_offsets(lhs: &[u8], rhs: &[u8], limit: usize) -> Vec<usize> {
    lhs.iter()
        .zip(rhs.iter())
        .enumerate()
        .filter_map(|(index, (left, right))| (left != right).then_some(index))
        .take(limit)
        .collect()
}
