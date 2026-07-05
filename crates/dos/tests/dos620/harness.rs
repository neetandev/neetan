use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

use common::{BUILTIN_FONT_ROM, Bus, CpuMode, JisChar, Machine as _, MachineModel, Tracing};
use device::{
    disk::{HddFormat, HddGeometry, HddImage},
    floppy::d88::{D88Disk, D88MediaType, D88Sector},
};

pub const INJECT_CODE_SEGMENT: u16 = 0x2000;
pub const INJECT_CODE_BASE: u32 = (INJECT_CODE_SEGMENT as u32) << 4;
pub const INJECT_RESULT_OFFSET: u16 = 0x0100;
pub const INJECT_RESULT_BASE: u32 = INJECT_CODE_BASE + INJECT_RESULT_OFFSET as u32;
pub const INJECT_BUDGET: u64 = 50_000_000;
pub const INJECT_BUDGET_DISK_IO: u64 = 500_000_000;

const HLE_BOOT_MAX_CYCLES: u64 = 500_000_000;
const HLE_BOOT_CHECK_INTERVAL: u64 = 1_000_000;
const PROMPT_WAIT_MAX_CYCLES: u64 = 500_000_000;
const PROMPT_WAIT_CHECK_INTERVAL: u64 = 100_000;
const TEXT_VRAM_COLUMNS: usize = 80;
const TEXT_VRAM_ROWS: usize = 25;
const TEXT_VRAM_CELL_COUNT: usize = TEXT_VRAM_COLUMNS * TEXT_VRAM_ROWS;
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
const FLOPPY_ROOT_DIRECTORY_OFFSET: usize =
    (FLOPPY_RESERVED_SECTORS + FLOPPY_FAT_COUNT * FLOPPY_SECTORS_PER_FAT) * FLOPPY_SECTOR_SIZE;
const FLOPPY_DATA_START_SECTOR: usize = FLOPPY_RESERVED_SECTORS
    + FLOPPY_FAT_COUNT * FLOPPY_SECTORS_PER_FAT
    + FLOPPY_ROOT_DIRECTORY_SECTORS;
const FLOPPY_TOTAL_SECTORS: usize = FLOPPY_CYLINDERS * FLOPPY_HEADS * FLOPPY_SECTORS_PER_TRACK;
const HDD_CYLINDERS: u16 = 20;
const HDD_HEADS: u8 = 8;
const HDD_SECTORS_PER_TRACK: u8 = 17;
static TEMP_CDROM_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy)]
struct TestFileSpec<'a> {
    name: [u8; 11],
    data: &'a [u8],
    attributes: u8,
    time: u16,
    date: u16,
}

#[derive(Clone, Copy)]
struct HddVolumeSpec<'a> {
    physical_sector_size: u16,
    logical_sector_size: u16,
    sectors_per_cluster: u8,
    root_entry_count: u16,
    sectors_per_fat: u16,
    fat_kind: FatKind,
    files: &'a [TestFileSpec<'a>],
}

#[derive(Clone, Copy)]
enum FatKind {
    Fat12,
    Fat16,
}

pub struct TempCdromCueFiles {
    pub cue_path: PathBuf,
    bin_paths: Vec<PathBuf>,
}

impl Drop for TempCdromCueFiles {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.cue_path);
        for bin_path in &self.bin_paths {
            let _ = std::fs::remove_file(bin_path);
        }
    }
}

fn next_temp_cdrom_stem(name: &str) -> String {
    let sequence = TEMP_CDROM_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("neetan_test_{name}_{}_{}", std::process::id(), sequence)
}

fn set_fat12_entry(fat: &mut [u8], cluster: u16, value: u16) {
    let masked_value = value & 0x0FFF;
    let offset = (cluster as usize * 3) / 2;
    if cluster & 1 == 0 {
        fat[offset] = (masked_value & 0x00FF) as u8;
        fat[offset + 1] = (fat[offset + 1] & 0xF0) | ((masked_value >> 8) as u8 & 0x0F);
    } else {
        fat[offset] = (fat[offset] & 0x0F) | (((masked_value << 4) as u8) & 0xF0);
        fat[offset + 1] = (masked_value >> 4) as u8;
    }
}

fn set_fat16_entry(fat: &mut [u8], cluster: u16, value: u16) {
    let offset = cluster as usize * 2;
    fat[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn initialize_hle_bus(bus: &mut machine::Pc9801Bus, xms_32_enabled: bool) {
    bus.load_font_rom(BUILTIN_FONT_ROM);
    if xms_32_enabled {
        bus.set_xms_32_enabled(true);
    }
}

fn create_ra_machine(xms_32_enabled: bool) -> machine::Pc9801Ra {
    let mut machine = machine::Pc9801Ra::new(
        cpu::I386::new(),
        machine::Pc9801Bus::new(MachineModel::PC9801RA, CpuMode::High, 48000),
    );
    initialize_hle_bus(&mut machine.bus, xms_32_enabled);
    machine
}

fn create_hle_machine_ap() -> machine::Pc9821Ap {
    let mut machine = machine::Pc9821Ap::new(
        cpu::I386::<{ cpu::CPU_MODEL_486 }>::new(),
        machine::Pc9801Bus::new(MachineModel::PC9821AP, CpuMode::High, 48000),
    );
    initialize_hle_bus(&mut machine.bus, false);
    machine
}

fn write_disk_equipment(bus: &mut machine::Pc9801Bus, disk_equip_low: u8, disk_equip_high: u8) {
    bus.write_byte(0x055C, disk_equip_low);
    bus.write_byte(0x055D, disk_equip_high);
}

fn wait_for_prompt<const CPU_MODEL: u8>(
    machine: &mut machine::Machine<cpu::I386<CPU_MODEL>>,
    max_cycles: u64,
    check_interval: u64,
    timeout_message: &str,
) {
    let mut total_cycles = 0u64;
    loop {
        total_cycles += machine.run_for(check_interval);
        if hle_prompt_visible(&machine.bus) {
            break;
        }
        assert!(total_cycles < max_cycles, "{timeout_message}");
    }
}

fn write_standard_floppy_boot_sector(boot_sector: &mut [u8]) {
    boot_sector[0] = 0xEB;
    boot_sector[1] = 0x3C;
    boot_sector[2] = 0x90;
    boot_sector[3..11].copy_from_slice(b"NEETAN  ");
    boot_sector[11..13].copy_from_slice(&(FLOPPY_SECTOR_SIZE as u16).to_le_bytes());
    boot_sector[13] = 1;
    boot_sector[14..16].copy_from_slice(&(FLOPPY_RESERVED_SECTORS as u16).to_le_bytes());
    boot_sector[16] = FLOPPY_FAT_COUNT as u8;
    boot_sector[17..19].copy_from_slice(&(FLOPPY_ROOT_ENTRY_COUNT as u16).to_le_bytes());
    boot_sector[19..21].copy_from_slice(&(FLOPPY_TOTAL_SECTORS as u16).to_le_bytes());
    boot_sector[21] = 0xFE;
    boot_sector[22..24].copy_from_slice(&(FLOPPY_SECTORS_PER_FAT as u16).to_le_bytes());
    boot_sector[24..26].copy_from_slice(&(FLOPPY_SECTORS_PER_TRACK as u16).to_le_bytes());
    boot_sector[26..28].copy_from_slice(&(FLOPPY_HEADS as u16).to_le_bytes());
}

fn write_root_directory_entry(
    directory_entry: &mut [u8],
    file_spec: TestFileSpec<'_>,
    start_cluster: u16,
) {
    directory_entry.fill(0);
    directory_entry[0..11].copy_from_slice(&file_spec.name);
    directory_entry[11] = file_spec.attributes;
    directory_entry[22..24].copy_from_slice(&file_spec.time.to_le_bytes());
    directory_entry[24..26].copy_from_slice(&file_spec.date.to_le_bytes());
    directory_entry[26..28].copy_from_slice(&start_cluster.to_le_bytes());
    directory_entry[28..32].copy_from_slice(&(file_spec.data.len() as u32).to_le_bytes());
}

fn build_d88_tracks(disk_data: &[u8], mfm_flag: u8) -> Vec<Option<Vec<D88Sector>>> {
    let total_tracks = FLOPPY_CYLINDERS * FLOPPY_HEADS;
    let mut tracks = Vec::with_capacity(total_tracks);
    for track_index in 0..total_tracks {
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
                mfm_flag,
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

fn create_d88_floppy_image(
    name: &str,
    disk_data: &[u8],
    mfm_flag: u8,
) -> device::floppy::FloppyImage {
    let disk = D88Disk::from_tracks(
        name.to_string(),
        false,
        D88MediaType::Disk2HD,
        build_d88_tracks(disk_data, mfm_flag),
    );
    device::floppy::FloppyImage::from_d88(disk)
}

fn build_test_floppy_image(name: &str, files: &[TestFileSpec<'_>]) -> device::floppy::FloppyImage {
    let mut disk_data = vec![0u8; FLOPPY_TOTAL_SECTORS * FLOPPY_SECTOR_SIZE];
    write_standard_floppy_boot_sector(&mut disk_data[0..FLOPPY_SECTOR_SIZE]);

    let fat_offset = FLOPPY_SECTOR_SIZE;
    {
        let fat =
            &mut disk_data[fat_offset..fat_offset + FLOPPY_SECTORS_PER_FAT * FLOPPY_SECTOR_SIZE];
        fat[0] = 0xFE;
        fat[1] = 0xFF;
        fat[2] = 0xFF;
    }

    let bytes_per_cluster = FLOPPY_SECTOR_SIZE;
    let mut next_cluster = 2u16;
    for (file_index, file_spec) in files.iter().copied().enumerate() {
        let cluster_count = file_spec.data.len().div_ceil(bytes_per_cluster).max(1);
        let start_cluster = next_cluster;
        for cluster_offset in 0..cluster_count {
            let cluster = start_cluster + cluster_offset as u16;
            let fat_value = if cluster_offset + 1 == cluster_count {
                0x0FFF
            } else {
                cluster + 1
            };
            set_fat12_entry(
                &mut disk_data
                    [fat_offset..fat_offset + FLOPPY_SECTORS_PER_FAT * FLOPPY_SECTOR_SIZE],
                cluster,
                fat_value,
            );

            let file_offset = cluster_offset * bytes_per_cluster;
            let copy_length =
                bytes_per_cluster.min(file_spec.data.len().saturating_sub(file_offset));
            if copy_length > 0 {
                let sector_index = FLOPPY_DATA_START_SECTOR + cluster as usize - 2;
                let disk_offset = sector_index * FLOPPY_SECTOR_SIZE;
                disk_data[disk_offset..disk_offset + copy_length]
                    .copy_from_slice(&file_spec.data[file_offset..file_offset + copy_length]);
            }
        }

        let directory_offset = FLOPPY_ROOT_DIRECTORY_OFFSET + file_index * 32;
        write_root_directory_entry(
            &mut disk_data[directory_offset..directory_offset + 32],
            file_spec,
            start_cluster,
        );
        next_cluster += cluster_count as u16;
    }

    let fat_copy =
        disk_data[fat_offset..fat_offset + FLOPPY_SECTORS_PER_FAT * FLOPPY_SECTOR_SIZE].to_vec();
    let second_fat_offset = (FLOPPY_RESERVED_SECTORS + FLOPPY_SECTORS_PER_FAT) * FLOPPY_SECTOR_SIZE;
    disk_data[second_fat_offset..second_fat_offset + fat_copy.len()].copy_from_slice(&fat_copy);

    create_d88_floppy_image(name, &disk_data, 0x40)
}

fn build_empty_floppy_disk_data() -> Vec<u8> {
    vec![0u8; FLOPPY_TOTAL_SECTORS * FLOPPY_SECTOR_SIZE]
}

fn total_hdd_sectors() -> u32 {
    HDD_CYLINDERS as u32 * HDD_HEADS as u32 * HDD_SECTORS_PER_TRACK as u32
}

fn standard_test_files<'a>() -> [TestFileSpec<'a>; 3] {
    [
        TestFileSpec {
            name: *b"COMMAND COM",
            data: TEST_COMMAND_COM,
            attributes: 0x20,
            time: TEST_FILE_TIME,
            date: TEST_FILE_DATE,
        },
        TestFileSpec {
            name: *b"TESTFILETXT",
            data: TEST_FILE_CONTENT,
            attributes: 0x20,
            time: TEST_FILE_TIME,
            date: TEST_FILE_DATE,
        },
        TestFileSpec {
            name: *b"TEST    COM",
            data: TEST_COM_PROGRAM,
            attributes: 0x20,
            time: TEST_FILE_TIME,
            date: TEST_FILE_DATE,
        },
    ]
}

fn write_pc98_partition_table(image_data: &mut [u8], physical_sector_size: usize) -> usize {
    image_data[0] = 0xEB;
    image_data[1] = 0x1E;
    image_data[4..8].copy_from_slice(b"IPL1");

    let partition_table_offset = physical_sector_size;
    let partition = &mut image_data[partition_table_offset..partition_table_offset + 32];
    partition[0] = 0xA0;
    partition[1] = 0x91;
    partition[4] = 0;
    partition[5] = 0;
    partition[6] = 1;
    partition[7] = 0;
    partition[8] = 0;
    partition[9] = 0;
    partition[10] = 1;
    partition[11] = 0;
    partition[12] = HDD_SECTORS_PER_TRACK - 1;
    partition[13] = HDD_HEADS - 1;
    partition[14] = (HDD_CYLINDERS - 1) as u8;
    partition[15] = ((HDD_CYLINDERS - 1) >> 8) as u8;
    partition[16..32].copy_from_slice(b"MS-DOS 6.20\x00\x00\x00\x00\x00");

    HDD_HEADS as usize * HDD_SECTORS_PER_TRACK as usize
}

fn build_hdd_image(volume_spec: HddVolumeSpec<'_>) -> device::disk::HddImage {
    let physical_sector_size = volume_spec.physical_sector_size as usize;
    let logical_sector_size = volume_spec.logical_sector_size as usize;
    let total_sectors = total_hdd_sectors();
    let mut image_data = vec![0u8; total_sectors as usize * physical_sector_size];

    let partition_lba = write_pc98_partition_table(&mut image_data, physical_sector_size) as u32;
    let partition_byte_offset = partition_lba as usize * physical_sector_size;
    let reserved_sectors = 1u16;
    let fat_count = 2u8;
    let root_directory_sectors =
        (volume_spec.root_entry_count as usize * 32).div_ceil(logical_sector_size);
    let partition_sectors = if logical_sector_size == physical_sector_size {
        total_sectors - partition_lba
    } else {
        (total_sectors - partition_lba) / (logical_sector_size / physical_sector_size) as u32
    };
    let first_data_sector = reserved_sectors as usize
        + fat_count as usize * volume_spec.sectors_per_fat as usize
        + root_directory_sectors;

    let boot_sector =
        &mut image_data[partition_byte_offset..partition_byte_offset + logical_sector_size];
    boot_sector[0] = 0xEB;
    boot_sector[1] = 0x3C;
    boot_sector[2] = 0x90;
    boot_sector[3..11].copy_from_slice(b"NEETAN  ");
    boot_sector[11..13].copy_from_slice(&volume_spec.logical_sector_size.to_le_bytes());
    boot_sector[13] = volume_spec.sectors_per_cluster;
    boot_sector[14..16].copy_from_slice(&reserved_sectors.to_le_bytes());
    boot_sector[16] = fat_count;
    boot_sector[17..19].copy_from_slice(&volume_spec.root_entry_count.to_le_bytes());
    if partition_sectors <= 0xFFFF {
        boot_sector[19..21].copy_from_slice(&(partition_sectors as u16).to_le_bytes());
    }
    boot_sector[21] = 0xF8;
    boot_sector[22..24].copy_from_slice(&volume_spec.sectors_per_fat.to_le_bytes());
    boot_sector[24..26].copy_from_slice(&(HDD_SECTORS_PER_TRACK as u16).to_le_bytes());
    boot_sector[26..28].copy_from_slice(&(HDD_HEADS as u16).to_le_bytes());

    let first_fat_offset = partition_byte_offset + reserved_sectors as usize * logical_sector_size;
    {
        let first_fat = &mut image_data[first_fat_offset
            ..first_fat_offset + volume_spec.sectors_per_fat as usize * logical_sector_size];
        match volume_spec.fat_kind {
            FatKind::Fat12 => {
                first_fat[0] = 0xF8;
                first_fat[1] = 0xFF;
                first_fat[2] = 0xFF;
            }
            FatKind::Fat16 => {
                first_fat[0] = 0xF8;
                first_fat[1] = 0xFF;
                first_fat[2] = 0xFF;
                first_fat[3] = 0xFF;
            }
        }
    }

    let bytes_per_cluster = logical_sector_size * volume_spec.sectors_per_cluster as usize;
    let root_directory_offset = partition_byte_offset
        + (reserved_sectors as usize + fat_count as usize * volume_spec.sectors_per_fat as usize)
            * logical_sector_size;
    let data_area_offset = partition_byte_offset + first_data_sector * logical_sector_size;

    let mut next_cluster = 2u16;
    for (file_index, file_spec) in volume_spec.files.iter().copied().enumerate() {
        let cluster_count = file_spec.data.len().div_ceil(bytes_per_cluster).max(1);
        let start_cluster = next_cluster;
        for cluster_offset in 0..cluster_count {
            let cluster = start_cluster + cluster_offset as u16;
            let fat_value = if cluster_offset + 1 == cluster_count {
                match volume_spec.fat_kind {
                    FatKind::Fat12 => 0x0FFF,
                    FatKind::Fat16 => 0xFFFF,
                }
            } else {
                cluster + 1
            };
            match volume_spec.fat_kind {
                FatKind::Fat12 => set_fat12_entry(
                    &mut image_data[first_fat_offset
                        ..first_fat_offset
                            + volume_spec.sectors_per_fat as usize * logical_sector_size],
                    cluster,
                    fat_value,
                ),
                FatKind::Fat16 => set_fat16_entry(
                    &mut image_data[first_fat_offset
                        ..first_fat_offset
                            + volume_spec.sectors_per_fat as usize * logical_sector_size],
                    cluster,
                    fat_value,
                ),
            }

            let file_offset = cluster_offset * bytes_per_cluster;
            let copy_length =
                bytes_per_cluster.min(file_spec.data.len().saturating_sub(file_offset));
            if copy_length > 0 {
                let cluster_byte_offset =
                    data_area_offset + (cluster as usize - 2) * bytes_per_cluster;
                image_data[cluster_byte_offset..cluster_byte_offset + copy_length]
                    .copy_from_slice(&file_spec.data[file_offset..file_offset + copy_length]);
            }
        }

        let directory_offset = root_directory_offset + file_index * 32;
        write_root_directory_entry(
            &mut image_data[directory_offset..directory_offset + 32],
            file_spec,
            start_cluster,
        );
        next_cluster += cluster_count as u16;
    }

    let fat_copy = image_data[first_fat_offset
        ..first_fat_offset + volume_spec.sectors_per_fat as usize * logical_sector_size]
        .to_vec();
    let second_fat_offset =
        first_fat_offset + volume_spec.sectors_per_fat as usize * logical_sector_size;
    image_data[second_fat_offset..second_fat_offset + fat_copy.len()].copy_from_slice(&fat_copy);

    let geometry = HddGeometry {
        cylinders: HDD_CYLINDERS,
        heads: HDD_HEADS,
        sectors_per_track: HDD_SECTORS_PER_TRACK,
        sector_size: volume_spec.physical_sector_size,
    };
    HddImage::from_raw(geometry, HddFormat::Nhd, image_data)
}

fn text_vram_codes(bus: &machine::Pc9801Bus) -> Vec<u16> {
    bus.text_vram()
        .chunks_exact(2)
        .take(TEXT_VRAM_CELL_COUNT)
        .map(|cell| u16::from_le_bytes([cell[0], cell[1]]))
        .collect()
}

fn text_vram_jis_chars(bus: &machine::Pc9801Bus) -> Vec<JisChar> {
    bus.text_vram()
        .chunks_exact(2)
        .take(TEXT_VRAM_CELL_COUNT)
        .map(|cell| JisChar::from_vram_bytes(cell[0], cell[1]))
        .collect()
}

/// Creates a machine with no disk images. The bootstrap will find no bootable
/// media and activate NEETAN DOS HLE DOS automatically.
pub fn create_hle_machine() -> machine::Pc9801Ra {
    create_ra_machine(true)
}

/// Boots a machine with NEETAN DOS HLE DOS (no disk images).
/// Returns the machine after the shell prompt (`>`) is visible in text VRAM.
pub fn boot_hle() -> machine::Pc9801Ra {
    boot_hle_with_time(None)
}

/// Boots a machine with NEETAN DOS HLE DOS and an optional fixed time provider.
pub fn boot_hle_with_time(time_fn: Option<fn() -> [u8; 6]>) -> machine::Pc9801Ra {
    let mut machine = create_hle_machine();
    if let Some(f) = time_fn {
        machine.bus.set_host_local_time_fn(f);
    }
    wait_for_prompt(
        &mut machine,
        HLE_BOOT_MAX_CYCLES,
        HLE_BOOT_CHECK_INTERVAL,
        "HLE DOS did not show prompt within 500000000 cycles",
    );
    machine
}

/// Boots a machine with XMS disabled on the bus (no XMS driver, no XMSXXXX0 device).
pub fn boot_hle_without_xms() -> machine::Pc9801Ra {
    let mut machine = create_hle_machine();
    machine.bus.set_xms_enabled(false);
    wait_for_prompt(
        &mut machine,
        HLE_BOOT_MAX_CYCLES,
        HLE_BOOT_CHECK_INTERVAL,
        "HLE DOS did not show prompt within 500000000 cycles",
    );
    machine
}

/// Boots a machine with EMS disabled on the bus (no EMS driver, no EMMXXXX0 device).
pub fn boot_hle_without_ems() -> machine::Pc9801Ra {
    let mut machine = create_hle_machine();
    machine.bus.set_ems_enabled(false);
    wait_for_prompt(
        &mut machine,
        HLE_BOOT_MAX_CYCLES,
        HLE_BOOT_CHECK_INTERVAL,
        "HLE DOS did not show prompt within 500000000 cycles",
    );
    machine
}

/// Boots a machine with both XMS and EMS disabled on the bus.
pub fn boot_hle_without_xms_and_ems() -> machine::Pc9801Ra {
    let mut machine = create_hle_machine();
    machine.bus.set_xms_enabled(false);
    machine.bus.set_ems_enabled(false);
    wait_for_prompt(
        &mut machine,
        HLE_BOOT_MAX_CYCLES,
        HLE_BOOT_CHECK_INTERVAL,
        "HLE DOS did not show prompt within 500000000 cycles",
    );
    machine
}

/// Boots a machine with the XMS /HMAMIN= threshold set to the given KB.
pub fn boot_hle_with_hmamin_kb(hmamin_kb: u16) -> machine::Pc9801Ra {
    let mut machine = create_hle_machine();
    machine.bus.set_xms_hmamin_kb(hmamin_kb);
    wait_for_prompt(
        &mut machine,
        HLE_BOOT_MAX_CYCLES,
        HLE_BOOT_CHECK_INTERVAL,
        "HLE DOS did not show prompt within 500000000 cycles",
    );
    machine
}

pub fn boot_hle_with_forced_os(
    floppy: Option<device::floppy::FloppyImage>,
    hdd: Option<device::disk::HddImage>,
) -> machine::Pc9801Ra {
    let mut machine = create_hle_machine();
    machine.bus.set_boot_device(machine::BootDevice::Dos);

    let mut disk_equip_low = 0u8;
    let mut disk_equip_high = 0u8;

    if let Some(floppy_image) = floppy {
        disk_equip_low |= 0x01;
        machine.bus.insert_floppy(0, floppy_image, None);
    }

    if let Some(hdd_image) = hdd {
        disk_equip_high |= 0x01;
        machine.bus.insert_hdd(0, hdd_image, None);
    }

    write_disk_equipment(&mut machine.bus, disk_equip_low, disk_equip_high);
    wait_for_prompt(
        &mut machine,
        HLE_BOOT_MAX_CYCLES,
        HLE_BOOT_CHECK_INTERVAL,
        "HLE DOS did not show prompt within 500000000 cycles",
    );

    machine
}

pub fn hle_prompt_visible(bus: &machine::Pc9801Bus) -> bool {
    find_char_in_text_vram(bus, 0x003E)
}

/// Known content for COMMAND.COM on the test floppy (100 bytes).
pub const TEST_COMMAND_COM: &[u8] = b"This is a fake COMMAND.COM for testing purposes. \
It contains exactly one hundred bytes of known content!!";

/// Known content for TESTFILE.TXT on the test floppy.
pub const TEST_FILE_CONTENT: &[u8] = b"HELLO WORLD\r\n";

/// Known content for README.TXT on the synthetic test CD-ROM.
pub const TEST_CDROM_README: &[u8] = b"NEETAN CD README\r\n";

/// Tiny .COM program for EXEC testing: terminates with exit code 0x42.
/// MOV AH, 4Ch ; B4 4C
/// MOV AL, 42h ; B0 42
/// INT 21h     ; CD 21
pub const TEST_COM_PROGRAM: &[u8] = &[0xB4, 0x4C, 0xB0, 0x42, 0xCD, 0x21];

/// Known date for files on the test floppy: 1995-01-01.
/// DOS date: ((15)<<9) | (1<<5) | 1 = 0x1E21
pub const TEST_FILE_DATE: u16 = 0x1E21;

/// Known time for files on the test floppy: 12:00:00.
/// DOS time: (12<<11) | (0<<5) | 0 = 0x6000
pub const TEST_FILE_TIME: u16 = 0x6000;

/// Creates a PC-98 2HD floppy (FAT12) with known test files as a D88 FloppyImage.
pub fn create_test_floppy() -> device::floppy::FloppyImage {
    build_test_floppy_image("TEST", &standard_test_files())
}

pub fn create_test_floppy_without_bpb() -> device::floppy::FloppyImage {
    let mut floppy = create_test_floppy();
    let boot_sector = floppy
        .find_sector_on_track_index_mut(0, 0, 0, 1, 3)
        .expect("test floppy has boot sector");
    let boot_code = [
        0x33, 0xC0, 0x8E, 0xC0, 0x8E, 0xD8, 0x8E, 0xD0, 0xBC, 0x8E, 0x02, 0x26, 0xA0, 0x84, 0x05,
        0xBD, 0x00, 0x06, 0xBB, 0x00, 0x14, 0xB9, 0x00, 0x03, 0xBA, 0x04, 0x01, 0xB4, 0x76, 0xCD,
        0x1B, 0xBD,
    ];
    boot_sector.data[..boot_code.len()].copy_from_slice(&boot_code);
    floppy
}

/// Creates a test floppy with custom program data at cluster 4.
/// `fcb_name` is the 11-byte FCB name (e.g. `b"TEST    COM"` or `b"TEST    EXE"`).
pub fn create_test_floppy_with_program(
    fcb_name: &[u8; 11],
    program_data: &[u8],
) -> device::floppy::FloppyImage {
    let files = [
        TestFileSpec {
            name: *b"COMMAND COM",
            data: TEST_COMMAND_COM,
            attributes: 0x20,
            time: TEST_FILE_TIME,
            date: TEST_FILE_DATE,
        },
        TestFileSpec {
            name: *b"TESTFILETXT",
            data: TEST_FILE_CONTENT,
            attributes: 0x20,
            time: TEST_FILE_TIME,
            date: TEST_FILE_DATE,
        },
        TestFileSpec {
            name: *fcb_name,
            data: program_data,
            attributes: 0x20,
            time: TEST_FILE_TIME,
            date: TEST_FILE_DATE,
        },
    ];
    build_test_floppy_image("TEST", &files)
}

pub fn create_test_floppy_with_autoexec(autoexec_data: &[u8]) -> device::floppy::FloppyImage {
    let files = [
        TestFileSpec {
            name: *b"COMMAND COM",
            data: TEST_COMMAND_COM,
            attributes: 0x20,
            time: TEST_FILE_TIME,
            date: TEST_FILE_DATE,
        },
        TestFileSpec {
            name: *b"TESTFILETXT",
            data: TEST_FILE_CONTENT,
            attributes: 0x20,
            time: TEST_FILE_TIME,
            date: TEST_FILE_DATE,
        },
        TestFileSpec {
            name: *b"TEST    COM",
            data: TEST_COM_PROGRAM,
            attributes: 0x20,
            time: TEST_FILE_TIME,
            date: TEST_FILE_DATE,
        },
        TestFileSpec {
            name: *b"AUTOEXECBAT",
            data: autoexec_data,
            attributes: 0x20,
            time: TEST_FILE_TIME,
            date: TEST_FILE_DATE,
        },
    ];
    build_test_floppy_image("AUTOEXEC", &files)
}

pub fn create_test_floppy_with_config_and_autoexec(
    config_data: &[u8],
    autoexec_data: &[u8],
) -> device::floppy::FloppyImage {
    let files = [
        TestFileSpec {
            name: *b"COMMAND COM",
            data: TEST_COMMAND_COM,
            attributes: 0x20,
            time: TEST_FILE_TIME,
            date: TEST_FILE_DATE,
        },
        TestFileSpec {
            name: *b"TESTFILETXT",
            data: TEST_FILE_CONTENT,
            attributes: 0x20,
            time: TEST_FILE_TIME,
            date: TEST_FILE_DATE,
        },
        TestFileSpec {
            name: *b"TEST    COM",
            data: TEST_COM_PROGRAM,
            attributes: 0x20,
            time: TEST_FILE_TIME,
            date: TEST_FILE_DATE,
        },
        TestFileSpec {
            name: *b"CONFIG  SYS",
            data: config_data,
            attributes: 0x20,
            time: TEST_FILE_TIME,
            date: TEST_FILE_DATE,
        },
        TestFileSpec {
            name: *b"AUTOEXECBAT",
            data: autoexec_data,
            attributes: 0x20,
            time: TEST_FILE_TIME,
            date: TEST_FILE_DATE,
        },
    ];
    build_test_floppy_image("CONFIG", &files)
}

/// Boots an HLE machine, then inserts a test floppy as drive A:.
/// The floppy is inserted after boot so the BIOS doesn't try to boot from it.
/// BDA_DISK_EQUIP is set before boot so discover_drives() sees the FDD.
pub fn boot_hle_with_floppy() -> machine::Pc9801Ra {
    let mut machine = create_hle_machine();
    write_disk_equipment(&mut machine.bus, 0x01, 0x00);
    wait_for_prompt(
        &mut machine,
        HLE_BOOT_MAX_CYCLES,
        HLE_BOOT_CHECK_INTERVAL,
        "HLE DOS did not show prompt within 500000000 cycles",
    );
    let floppy = create_test_floppy();
    machine.bus.insert_floppy(0, floppy, None);
    machine
}

/// Boots an HLE machine with a custom floppy image as drive A:.
pub fn boot_hle_with_floppy_image(floppy: device::floppy::FloppyImage) -> machine::Pc9801Ra {
    let mut machine = create_hle_machine();
    write_disk_equipment(&mut machine.bus, 0x01, 0x00);
    wait_for_prompt(
        &mut machine,
        HLE_BOOT_MAX_CYCLES,
        HLE_BOOT_CHECK_INTERVAL,
        "HLE DOS did not show prompt within 500000000 cycles",
    );
    machine.bus.insert_floppy(0, floppy, None);
    machine
}

pub fn create_blank_floppy() -> device::floppy::FloppyImage {
    create_d88_floppy_image("BLANK", &build_empty_floppy_disk_data(), 0x40)
}

pub fn create_parsed_empty_d88_floppy() -> device::floppy::FloppyImage {
    let d88 = D88Disk::from_tracks(
        String::new(),
        false,
        D88MediaType::Disk2HD,
        build_d88_tracks(&build_empty_floppy_disk_data(), 0x00),
    );
    let bytes = d88.to_bytes();
    device::floppy::FloppyImage::from_d88_bytes(&bytes).expect("parse empty D88 floppy")
}

pub fn boot_hle_with_two_floppies() -> machine::Pc9801Ra {
    let mut machine = create_hle_machine();
    write_disk_equipment(&mut machine.bus, 0x03, 0x00);
    wait_for_prompt(
        &mut machine,
        HLE_BOOT_MAX_CYCLES,
        HLE_BOOT_CHECK_INTERVAL,
        "HLE DOS did not show prompt within 500000000 cycles",
    );

    let floppy_a = create_test_floppy();
    let floppy_b = create_blank_floppy();
    machine.bus.insert_floppy(0, floppy_a, None);
    machine.bus.insert_floppy(1, floppy_b, None);

    machine
}

pub fn boot_hle_with_two_floppy_images(
    floppy_a: device::floppy::FloppyImage,
    floppy_b: device::floppy::FloppyImage,
) -> machine::Pc9801Ra {
    let mut machine = create_hle_machine();
    write_disk_equipment(&mut machine.bus, 0x03, 0x00);
    wait_for_prompt(
        &mut machine,
        HLE_BOOT_MAX_CYCLES,
        HLE_BOOT_CHECK_INTERVAL,
        "HLE DOS did not show prompt within 500000000 cycles",
    );
    machine.bus.insert_floppy(0, floppy_a, None);
    machine.bus.insert_floppy(1, floppy_b, None);

    machine
}

pub fn write_bytes(bus: &mut impl Bus, addr: u32, data: &[u8]) {
    for (i, &byte) in data.iter().enumerate() {
        bus.write_byte(addr + i as u32, byte);
    }
}

/// Positions the cursor by writing the IOSYS cursor bytes directly, the way a
/// program using the IO.SYS convention does. The HLE DOS dispatch reconciliation
/// propagates this change to the GDC on the next syscall.
pub fn set_cursor_position<T: Tracing>(bus: &mut machine::Pc9801Bus<T>, row: u8, col: u8) {
    const IOSYS_CURSOR_Y: u32 = 0x0600 + 0x0110;
    const IOSYS_CURSOR_X: u32 = 0x0600 + 0x011C;
    bus.write_byte(IOSYS_CURSOR_Y, row);
    bus.write_byte(IOSYS_CURSOR_X, col);
}

pub fn inject_and_run(machine: &mut machine::Pc9801Ra, code: &[u8]) {
    inject_and_run_with_budget(machine, code, INJECT_BUDGET);
}

fn load_injected_i386_state<const CPU_MODEL: u8>(
    machine: &mut machine::Machine<cpu::I386<CPU_MODEL>>,
) {
    let mut state = cpu::I386State {
        ip: 0x0000,
        ..Default::default()
    };
    state.set_cs(INJECT_CODE_SEGMENT);
    state.set_ss(INJECT_CODE_SEGMENT);
    state.set_ds(INJECT_CODE_SEGMENT);
    state.set_es(INJECT_CODE_SEGMENT);
    state.set_esp(0xFFFE);
    state.set_eflags(state.eflags() | 0x0200);
    machine.cpu.load_state(&state);
}

fn type_string_long_generic<const CPU_MODEL: u8>(
    machine: &mut machine::Machine<cpu::I386<CPU_MODEL>>,
    text: &[u8],
) {
    let chunk_size = 12;
    for chunk in text.chunks(chunk_size) {
        type_string(&mut machine.bus, chunk);
        machine.run_for(5_000_000);
    }
}

fn run_until_prompt_generic<const CPU_MODEL: u8>(
    machine: &mut machine::Machine<cpu::I386<CPU_MODEL>>,
) {
    wait_for_prompt(
        machine,
        PROMPT_WAIT_MAX_CYCLES,
        PROMPT_WAIT_CHECK_INTERVAL,
        "shell did not return to prompt within 500000000 cycles",
    );
}

pub fn inject_and_run_with_budget(machine: &mut machine::Pc9801Ra, code: &[u8], budget: u64) {
    write_bytes(&mut machine.bus, INJECT_CODE_BASE, code);
    load_injected_i386_state(machine);
    machine.run_for(budget);
}

/// Runs code via the INT 28h DOS idle hook.
///
/// COMMAND.COM's loop calls INT 28h on each iteration. This function hooks
/// INT 28h to run the given test code once, then restores the original
/// vector and IRETs. INT 21h functions with AH >= 0Ch (all file I/O) are
/// safe to call from within an INT 28h handler.
///
/// Layout at INJECT_CODE_BASE:
///   +0x0000: INT 28h hook stub (saves old vector, runs user code, restores, IRET)
///   +0x0080: user code (the actual test code)
///   +0x0100: result area
///   +0x0200: data area (filenames, buffers)
pub fn inject_and_run_via_int28(machine: &mut machine::Pc9801Ra, code: &[u8], budget: u64) {
    let base = INJECT_CODE_BASE;
    let seg_lo = (INJECT_CODE_SEGMENT & 0xFF) as u8;
    let seg_hi = (INJECT_CODE_SEGMENT >> 8) as u8;

    // Save old INT 28h vector (at IVT 0x00A0).
    let old_int28_off = read_word(&machine.bus, 0x00A0);
    let old_int28_seg = read_word(&machine.bus, 0x00A2);

    // Write user code at +0x0080.
    write_bytes(&mut machine.bus, base + 0x0080, code);

    let old_off_lo = (old_int28_off & 0xFF) as u8;
    let old_off_hi = (old_int28_off >> 8) as u8;
    let old_seg_lo = (old_int28_seg & 0xFF) as u8;
    let old_seg_hi = (old_int28_seg >> 8) as u8;
    // CALL rel16: target=0x0080, CALL at +0x09 (3 bytes), IP after=0x0C, rel=0x0080-0x0C=0x0074.
    #[rustfmt::skip]
    let stub: Vec<u8> = vec![
        0x1E,                               // PUSH DS                  ; +0x00
        0x06,                               // PUSH ES                  ; +0x01
        0xB8, seg_lo, seg_hi,               // MOV AX, seg              ; +0x02
        0x8E, 0xD8,                         // MOV DS, AX               ; +0x05
        0x8E, 0xC0,                         // MOV ES, AX               ; +0x07
        0xE8, 0x74, 0x00,                   // CALL 0080h               ; +0x09
        // After user code returns:
        0x07,                               // POP ES                   ; +0x0C
        0x1F,                               // POP DS                   ; +0x0D
        // Restore old INT 28h vector (write to IVT at 0000:00A0)
        0x50,                               // PUSH AX                  ; +0x0E
        0x53,                               // PUSH BX                  ; +0x0F
        0x1E,                               // PUSH DS                  ; +0x10
        0x31, 0xDB,                         // XOR BX, BX               ; +0x11
        0x8E, 0xDB,                         // MOV DS, BX               ; +0x13
        0xBB, 0xA0, 0x00,                   // MOV BX, 00A0h            ; +0x15
        0xC7, 0x07, old_off_lo, old_off_hi, // MOV [BX], old_offset    ; +0x18
        0xC7, 0x47, 0x02, old_seg_lo, old_seg_hi, // MOV [BX+2], old_segment ; +0x1C
        0x1F,                               // POP DS                   ; +0x21
        0x5B,                               // POP BX                   ; +0x22
        0x58,                               // POP AX                   ; +0x23
        0xCF,                               // IRET                     ; +0x24
    ];
    write_bytes(&mut machine.bus, base, &stub);

    // Set INT 28h vector to point to our stub.
    machine.bus.write_byte(0x00A0, 0x00); // offset low
    machine.bus.write_byte(0x00A1, 0x00); // offset high
    machine.bus.write_byte(0x00A2, seg_lo);
    machine.bus.write_byte(0x00A3, seg_hi);

    // Resume the machine. COMMAND.COM's loop calls INT 28h on each iteration,
    // which runs our stub, which runs the user code, restores INT 28h, and IRETs.
    machine.run_for(budget);
}

pub fn far_to_linear(segment: u16, offset: u16) -> u32 {
    ((segment as u32) << 4) + offset as u32
}

pub fn read_byte(bus: &machine::Pc9801Bus, addr: u32) -> u8 {
    bus.read_byte_direct(addr)
}

pub fn read_word(bus: &machine::Pc9801Bus, addr: u32) -> u16 {
    let low = bus.read_byte_direct(addr) as u16;
    let high = bus.read_byte_direct(addr + 1) as u16;
    low | (high << 8)
}

pub fn read_far_ptr(bus: &machine::Pc9801Bus, addr: u32) -> (u16, u16) {
    let offset = read_word(bus, addr);
    let segment = read_word(bus, addr + 2);
    (segment, offset)
}

pub fn read_bytes(bus: &machine::Pc9801Bus, addr: u32, len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| bus.read_byte_direct(addr + i as u32))
        .collect()
}

pub fn read_string(bus: &machine::Pc9801Bus, addr: u32, max_len: usize) -> Vec<u8> {
    let mut result = Vec::new();
    for i in 0..max_len {
        let byte = bus.read_byte_direct(addr + i as u32);
        if byte == 0 {
            break;
        }
        result.push(byte);
    }
    result
}

pub fn read_device_name(bus: &machine::Pc9801Bus, header_addr: u32) -> String {
    // Device header name field is at offset +0x0A, 8 bytes.
    let name_bytes = read_bytes(bus, header_addr + 0x0A, 8);
    String::from_utf8_lossy(&name_bytes).to_string()
}

pub fn result_byte(bus: &machine::Pc9801Bus, offset: u32) -> u8 {
    bus.read_byte_direct(INJECT_RESULT_BASE + offset)
}

pub fn result_word(bus: &machine::Pc9801Bus, offset: u32) -> u16 {
    read_word(bus, INJECT_RESULT_BASE + offset)
}

pub fn result_dword(bus: &machine::Pc9801Bus, offset: u32) -> u32 {
    let lo = result_word(bus, offset) as u32;
    let hi = result_word(bus, offset + 2) as u32;
    lo | (hi << 16)
}

pub fn get_sysvars_address(machine: &mut machine::Pc9801Ra) -> u32 {
    const RES_LO: u8 = (INJECT_RESULT_OFFSET & 0xFF) as u8;
    const RES_HI: u8 = (INJECT_RESULT_OFFSET >> 8) as u8;
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x52,                         // MOV AH, 52h
        0xCD, 0x21,                         // INT 21h
        0x89, 0x1E, RES_LO, RES_HI,         // MOV [result+0], BX
        0x8C, 0x06, RES_LO + 2, RES_HI,     // MOV [result+2], ES
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    inject_and_run(machine, code);

    let offset = result_word(&machine.bus, 0);
    let segment = result_word(&machine.bus, 2);
    far_to_linear(segment, offset)
}

pub fn get_psp_segment(machine: &mut machine::Pc9801Ra) -> u16 {
    const RES_LO: u8 = (INJECT_RESULT_OFFSET & 0xFF) as u8;
    const RES_HI: u8 = (INJECT_RESULT_OFFSET >> 8) as u8;
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x62,                         // MOV AH, 62h
        0xCD, 0x21,                         // INT 21h
        0x89, 0x1E, RES_LO, RES_HI,        // MOV [result+0], BX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    inject_and_run(machine, code);

    result_word(&machine.bus, 0)
}

/// Creates free memory by splitting the last MCB (Z block) in the chain.
/// COMMAND.COM owns all remaining memory after boot, so allocation tests
pub fn find_char_in_text_vram(bus: &machine::Pc9801Bus, char_code: u16) -> bool {
    text_vram_codes(bus)
        .into_iter()
        .any(|code| code == char_code)
}

pub fn find_string_in_text_vram(bus: &machine::Pc9801Bus, chars: &[u16]) -> bool {
    if chars.is_empty() {
        return true;
    }
    let codes = text_vram_codes(bus);
    codes.windows(chars.len()).any(|window| window == chars)
}

pub fn find_jis_string_in_text_vram(bus: &machine::Pc9801Bus, chars: &[JisChar]) -> bool {
    if chars.is_empty() {
        return true;
    }

    let jis_chars = text_vram_jis_chars(bus);
    for start in 0..jis_chars.len() {
        let mut cell_index = start;
        let mut matched = true;

        for &expected in chars {
            if cell_index >= jis_chars.len() {
                matched = false;
                break;
            }

            let actual = jis_chars[cell_index];
            if actual != expected {
                matched = false;
                break;
            }

            cell_index += 1;
            if !expected.is_ank() {
                if cell_index >= jis_chars.len() {
                    matched = false;
                    break;
                }

                let placeholder = jis_chars[cell_index];
                if placeholder != JisChar::from_u16(0x0000) {
                    matched = false;
                    break;
                }
                cell_index += 1;
            }
        }

        if matched {
            return true;
        }
    }

    false
}

pub fn text_vram_row_to_string(bus: &machine::Pc9801Bus, row: usize) -> String {
    let start_index = row * TEXT_VRAM_COLUMNS;
    text_vram_codes(bus)
        .into_iter()
        .skip(start_index)
        .take(TEXT_VRAM_COLUMNS)
        .map(|code| {
            if (0x20..=0x7E).contains(&code) {
                code as u8 as char
            } else {
                ' '
            }
        })
        .collect()
}

pub fn find_row_containing(bus: &machine::Pc9801Bus, needle: &str) -> Option<usize> {
    for row in 0..TEXT_VRAM_ROWS {
        let line = text_vram_row_to_string(bus, row);
        if line.contains(needle) {
            return Some(row);
        }
    }
    None
}

/// Creates a minimal in-memory HDD image with a FAT16 partition.
/// The image has a PC-98 partition table at sector 1 and a FAT16 volume at the partition offset.
/// `sector_size`: 256 or 512 bytes.
/// `test_files`: if true, populates COMMAND.COM and TESTFILE.TXT.
pub fn create_test_hdd(sector_size: u16) -> device::disk::HddImage {
    let files = [
        TestFileSpec {
            name: *b"COMMAND COM",
            data: TEST_COMMAND_COM,
            attributes: 0x20,
            time: TEST_FILE_TIME,
            date: TEST_FILE_DATE,
        },
        TestFileSpec {
            name: *b"TESTFILETXT",
            data: TEST_FILE_CONTENT,
            attributes: 0x20,
            time: TEST_FILE_TIME,
            date: TEST_FILE_DATE,
        },
    ];
    build_hdd_image(HddVolumeSpec {
        physical_sector_size: sector_size,
        logical_sector_size: sector_size,
        sectors_per_cluster: if sector_size == 256 { 8 } else { 4 },
        root_entry_count: 512,
        sectors_per_fat: 16,
        fat_kind: FatKind::Fat16,
        files: &files,
    })
}

pub fn create_test_hdd_with_many_txt_files(
    sector_size: u16,
    file_count: usize,
) -> device::disk::HddImage {
    let mut files = Vec::with_capacity(file_count + 1);
    files.push(TestFileSpec {
        name: *b"COMMAND COM",
        data: TEST_COMMAND_COM,
        attributes: 0x20,
        time: TEST_FILE_TIME,
        date: TEST_FILE_DATE,
    });

    for index in 0..file_count {
        let mut name = [b' '; 11];
        let stem = format!("F{index:07}");
        name[..8].copy_from_slice(stem.as_bytes());
        name[8..11].copy_from_slice(b"TXT");
        files.push(TestFileSpec {
            name,
            data: TEST_FILE_CONTENT,
            attributes: 0x20,
            time: TEST_FILE_TIME,
            date: TEST_FILE_DATE,
        });
    }

    build_hdd_image(HddVolumeSpec {
        physical_sector_size: sector_size,
        logical_sector_size: sector_size,
        sectors_per_cluster: if sector_size == 256 { 8 } else { 4 },
        root_entry_count: 512,
        sectors_per_fat: 16,
        fat_kind: FatKind::Fat16,
        files: &files,
    })
}

pub fn create_test_hdd_with_autoexec(
    sector_size: u16,
    autoexec_data: &[u8],
) -> device::disk::HddImage {
    let files = [
        TestFileSpec {
            name: *b"COMMAND COM",
            data: TEST_COMMAND_COM,
            attributes: 0x20,
            time: TEST_FILE_TIME,
            date: TEST_FILE_DATE,
        },
        TestFileSpec {
            name: *b"TESTFILETXT",
            data: TEST_FILE_CONTENT,
            attributes: 0x20,
            time: TEST_FILE_TIME,
            date: TEST_FILE_DATE,
        },
        TestFileSpec {
            name: *b"AUTOEXECBAT",
            data: autoexec_data,
            attributes: 0x20,
            time: TEST_FILE_TIME,
            date: TEST_FILE_DATE,
        },
    ];
    build_hdd_image(HddVolumeSpec {
        physical_sector_size: sector_size,
        logical_sector_size: sector_size,
        sectors_per_cluster: if sector_size == 256 { 8 } else { 4 },
        root_entry_count: 512,
        sectors_per_fat: 16,
        fat_kind: FatKind::Fat16,
        files: &files,
    })
}

/// Creates an HDD image where the physical sector size (256) differs from the
/// BPB logical sector size (1024). This is common on real PC-98 SASI drives.
/// The FAT volume uses 1024-byte logical sectors laid out across 256-byte physical sectors.
pub fn create_test_hdd_mismatched_sectors() -> device::disk::HddImage {
    let files = [
        TestFileSpec {
            name: *b"COMMAND COM",
            data: TEST_COMMAND_COM,
            attributes: 0x20,
            time: TEST_FILE_TIME,
            date: TEST_FILE_DATE,
        },
        TestFileSpec {
            name: *b"TESTFILETXT",
            data: TEST_FILE_CONTENT,
            attributes: 0x20,
            time: TEST_FILE_TIME,
            date: TEST_FILE_DATE,
        },
    ];
    build_hdd_image(HddVolumeSpec {
        physical_sector_size: 256,
        logical_sector_size: 1024,
        sectors_per_cluster: 4,
        root_entry_count: 192,
        sectors_per_fat: 2,
        fat_kind: FatKind::Fat12,
        files: &files,
    })
}

/// Boots an HLE machine (PC-9801RA / SASI) with an HDD that has mismatched
/// physical (256) and BPB logical (1024) sector sizes.
pub fn boot_hle_with_sasi_hdd_mismatched_sectors() -> machine::Pc9801Ra {
    let mut machine = create_ra_machine(false);
    write_disk_equipment(&mut machine.bus, 0x00, 0x01);
    wait_for_prompt(
        &mut machine,
        HLE_BOOT_MAX_CYCLES,
        HLE_BOOT_CHECK_INTERVAL,
        "HLE DOS did not show prompt within 500000000 cycles (SASI mismatched)",
    );

    let hdd = create_test_hdd_mismatched_sectors();
    machine.bus.insert_hdd(0, hdd, None);

    machine
}

/// Boots an HLE machine (PC-9801RA / SASI) with a test HDD as the first drive.
pub fn boot_hle_with_sasi_hdd(sector_size: u16) -> machine::Pc9801Ra {
    let mut machine = create_ra_machine(false);
    write_disk_equipment(&mut machine.bus, 0x00, 0x01);
    wait_for_prompt(
        &mut machine,
        HLE_BOOT_MAX_CYCLES,
        HLE_BOOT_CHECK_INTERVAL,
        "HLE DOS did not show prompt within 500000000 cycles (SASI)",
    );
    let hdd = create_test_hdd(sector_size);
    machine.bus.insert_hdd(0, hdd, None);

    machine
}

/// Boots an HLE machine (PC-9821AP / IDE) with a test HDD as the first drive.
pub fn boot_hle_with_ide_hdd(sector_size: u16) -> machine::Pc9821Ap {
    let mut machine = create_hle_machine_ap();
    write_disk_equipment(&mut machine.bus, 0x00, 0x01);
    wait_for_prompt(
        &mut machine,
        HLE_BOOT_MAX_CYCLES,
        HLE_BOOT_CHECK_INTERVAL,
        "HLE DOS did not show prompt within 500000000 cycles (IDE)",
    );
    let hdd = create_test_hdd(sector_size);
    machine.bus.insert_hdd(0, hdd, None);

    machine
}

/// Creates an empty (all-zeros) HDD image suitable for testing FORMAT.
pub fn create_empty_hdd(sector_size: u16) -> device::disk::HddImage {
    let data = vec![0u8; total_hdd_sectors() as usize * sector_size as usize];
    let geometry = HddGeometry {
        cylinders: HDD_CYLINDERS,
        heads: HDD_HEADS,
        sectors_per_track: HDD_SECTORS_PER_TRACK,
        sector_size,
    };
    HddImage::from_raw(geometry, HddFormat::Nhd, data)
}

/// Boots an HLE machine (PC-9801RA / SASI) with an empty HDD for format testing.
pub fn boot_hle_with_empty_sasi_hdd() -> machine::Pc9801Ra {
    let mut machine = create_ra_machine(false);
    write_disk_equipment(&mut machine.bus, 0x00, 0x01);

    let hdd = create_empty_hdd(256);
    machine.bus.insert_hdd(0, hdd, None);
    wait_for_prompt(
        &mut machine,
        HLE_BOOT_MAX_CYCLES,
        HLE_BOOT_CHECK_INTERVAL,
        "HLE DOS did not show prompt within 500000000 cycles (empty SASI)",
    );

    machine
}

/// Boots an HLE machine (PC-9821AP / IDE) with an empty HDD for format testing.
pub fn boot_hle_with_empty_ide_hdd() -> machine::Pc9821Ap {
    let mut machine = create_hle_machine_ap();
    write_disk_equipment(&mut machine.bus, 0x00, 0x01);

    let hdd = create_empty_hdd(512);
    machine.bus.insert_hdd(0, hdd, None);
    wait_for_prompt(
        &mut machine,
        HLE_BOOT_MAX_CYCLES,
        HLE_BOOT_CHECK_INTERVAL,
        "HLE DOS did not show prompt within 500000000 cycles (empty IDE)",
    );

    machine
}

fn write_both_endian_u16(destination: &mut [u8], value: u16) {
    destination[..2].copy_from_slice(&value.to_le_bytes());
    destination[2..4].copy_from_slice(&value.to_be_bytes());
}

fn write_both_endian_u32(destination: &mut [u8], value: u32) {
    destination[..4].copy_from_slice(&value.to_le_bytes());
    destination[4..8].copy_from_slice(&value.to_be_bytes());
}

fn cd_recording_time() -> [u8; 7] {
    [95, 1, 1, 12, 0, 0, 0]
}

fn write_cd_directory_record(
    buffer: &mut [u8],
    offset: &mut usize,
    identifier: &[u8],
    extent_lba: u32,
    data_length: u32,
    is_directory: bool,
) {
    let padding = usize::from(identifier.len().is_multiple_of(2));
    let length = 33 + identifier.len() + padding;
    let record = &mut buffer[*offset..*offset + length];
    record.fill(0);
    record[0] = length as u8;
    write_both_endian_u32(&mut record[2..10], extent_lba);
    write_both_endian_u32(&mut record[10..18], data_length);
    record[18..25].copy_from_slice(&cd_recording_time());
    record[25] = if is_directory { 0x02 } else { 0x00 };
    write_both_endian_u16(&mut record[28..32], 1);
    record[32] = identifier.len() as u8;
    record[33..33 + identifier.len()].copy_from_slice(identifier);
    *offset += length;
}

fn make_mode1_raw_sector(user_data: &[u8; 2048]) -> Vec<u8> {
    let mut sector = vec![0u8; 2352];
    sector[0] = 0x00;
    for byte in &mut sector[1..11] {
        *byte = 0xFF;
    }
    sector[11] = 0x00;
    sector[15] = 0x01;
    sector[16..16 + 2048].copy_from_slice(user_data);
    sector
}

fn make_mode2_raw_sector(user_data: &[u8; 2048]) -> Vec<u8> {
    let mut sector = vec![0u8; 2352];
    sector[0] = 0x00;
    for byte in &mut sector[1..11] {
        *byte = 0xFF;
    }
    sector[11] = 0x00;
    sector[13] = 0x02;
    sector[15] = 0x02;
    sector[24..24 + 2048].copy_from_slice(user_data);
    sector
}

/// Creates a minimal CD-ROM disc image with one data track and one audio track.
/// Uses raw (2352-byte) sectors throughout, as is standard for single-file BIN images.
pub fn create_test_cdimage() -> device::cdrom::CdImage {
    let cue = r#"FILE "test.bin" BINARY
  TRACK 01 MODE1/2352
    INDEX 01 00:00:00
  TRACK 02 AUDIO
    INDEX 01 00:02:00
"#;
    const ROOT_DIR_LBA: u32 = 20;
    const README_LBA: u32 = 21;
    const INSTALL_LBA: u32 = 22;
    const RUNBAT_LBA: u32 = 23;
    const RUNBAT_DATA: &[u8] = b"ECHO CDBATCH\r\n";

    // Track 1: 150 raw data sectors (2352 bytes each, with sync+header+user data).
    let mut bin_data = Vec::with_capacity(2352 * 200);
    for sector_index in 0..150u32 {
        let mut user_data = [0u8; 2048];
        match sector_index {
            16 => {
                user_data[0] = 1;
                user_data[1..6].copy_from_slice(b"CD001");
                user_data[6] = 1;
                user_data[8..40].copy_from_slice(b"NEETAN TEST CD                  ");
                user_data[40..72].copy_from_slice(b"NEETAN_CD                       ");
                write_both_endian_u32(&mut user_data[80..88], 150);
                write_both_endian_u16(&mut user_data[120..124], 1);
                write_both_endian_u16(&mut user_data[124..128], 1);
                write_both_endian_u16(&mut user_data[128..132], 2048);
                write_both_endian_u32(&mut user_data[132..140], 10);
                let mut root_record_offset = 156usize;
                write_cd_directory_record(
                    &mut user_data,
                    &mut root_record_offset,
                    &[0],
                    ROOT_DIR_LBA,
                    2048,
                    true,
                );
                let copyright = b"COPYRIGHT.TXT;1";
                user_data[702..702 + copyright.len()].copy_from_slice(copyright);
                for byte in &mut user_data[702 + copyright.len()..702 + 37] {
                    *byte = b' ';
                }
                let abstract_id = b"ABSTRACT.TXT;1";
                user_data[739..739 + abstract_id.len()].copy_from_slice(abstract_id);
                for byte in &mut user_data[739 + abstract_id.len()..739 + 37] {
                    *byte = b' ';
                }
                let biblio = b"BIBLIO.TXT;1";
                user_data[776..776 + biblio.len()].copy_from_slice(biblio);
                for byte in &mut user_data[776 + biblio.len()..776 + 37] {
                    *byte = b' ';
                }
            }
            17 => {
                user_data[0] = 0xFF;
                user_data[1..6].copy_from_slice(b"CD001");
                user_data[6] = 1;
            }
            ROOT_DIR_LBA => {
                let mut offset = 0usize;
                write_cd_directory_record(
                    &mut user_data,
                    &mut offset,
                    &[0],
                    ROOT_DIR_LBA,
                    2048,
                    true,
                );
                write_cd_directory_record(
                    &mut user_data,
                    &mut offset,
                    &[1],
                    ROOT_DIR_LBA,
                    2048,
                    true,
                );
                write_cd_directory_record(
                    &mut user_data,
                    &mut offset,
                    b"README.TXT;1",
                    README_LBA,
                    TEST_CDROM_README.len() as u32,
                    false,
                );
                write_cd_directory_record(
                    &mut user_data,
                    &mut offset,
                    b"INSTALL.EXE;1",
                    INSTALL_LBA,
                    4,
                    false,
                );
                write_cd_directory_record(
                    &mut user_data,
                    &mut offset,
                    b"RUNBAT.BAT;1",
                    RUNBAT_LBA,
                    RUNBAT_DATA.len() as u32,
                    false,
                );
            }
            README_LBA => {
                user_data[..TEST_CDROM_README.len()].copy_from_slice(TEST_CDROM_README);
            }
            INSTALL_LBA => {
                user_data[..4].copy_from_slice(b"MZ\x90\x00");
            }
            RUNBAT_LBA => {
                user_data[..RUNBAT_DATA.len()].copy_from_slice(RUNBAT_DATA);
            }
            _ => {
                user_data.fill(0x11);
            }
        }
        bin_data.extend_from_slice(&make_mode1_raw_sector(&user_data));
    }
    // Track 2: 50 audio sectors (2352 bytes each).
    bin_data.extend_from_slice(&vec![0xAAu8; 2352 * 50]);
    device::cdrom::CdImage::from_cue(cue, bin_data).unwrap()
}

pub fn create_test_cdimage_with_xcopy_tree() -> device::cdrom::CdImage {
    let cue = r#"FILE "xcopy.bin" BINARY
  TRACK 01 MODE1/2352
    INDEX 01 00:00:00
  TRACK 02 AUDIO
    INDEX 01 00:02:00
"#;
    const ROOT_DIR_LBA: u32 = 20;
    const SOURCE_DIR_LBA: u32 = 21;
    const SUBONE_DIR_LBA: u32 = 22;
    const SUBTWO_DIR_LBA: u32 = 23;
    const EMPTY_DIR_LBA: u32 = 24;
    const ROOT1_LBA: u32 = 30;
    const ROOT2_LBA: u32 = 31;
    const CHILD1_LBA: u32 = 32;
    const CHILD2_LBA: u32 = 33;
    const ROOT1_DATA: &[u8] = b"ROOT1\r\n";
    const ROOT2_DATA: &[u8] = b"ROOT2\r\n";
    const CHILD1_DATA: &[u8] = b"CHILD1\r\n";
    const CHILD2_DATA: &[u8] = b"CHILD2\r\n";

    let mut bin_data = Vec::with_capacity(2352 * 200);
    for sector_index in 0..150u32 {
        let mut user_data = [0u8; 2048];
        match sector_index {
            16 => {
                user_data[0] = 1;
                user_data[1..6].copy_from_slice(b"CD001");
                user_data[6] = 1;
                user_data[8..40].fill(b' ');
                user_data[8..28].copy_from_slice(b"NEETAN XCOPY TREE CD");
                user_data[40..72].fill(b' ');
                user_data[40..53].copy_from_slice(b"XCOPY_TREE_CD");
                write_both_endian_u32(&mut user_data[80..88], 150);
                write_both_endian_u16(&mut user_data[120..124], 1);
                write_both_endian_u16(&mut user_data[124..128], 1);
                write_both_endian_u16(&mut user_data[128..132], 2048);
                write_both_endian_u32(&mut user_data[132..140], 10);
                let mut root_record_offset = 156usize;
                write_cd_directory_record(
                    &mut user_data,
                    &mut root_record_offset,
                    &[0],
                    ROOT_DIR_LBA,
                    2048,
                    true,
                );
            }
            17 => {
                user_data[0] = 0xFF;
                user_data[1..6].copy_from_slice(b"CD001");
                user_data[6] = 1;
            }
            ROOT_DIR_LBA => {
                let mut offset = 0usize;
                write_cd_directory_record(
                    &mut user_data,
                    &mut offset,
                    &[0],
                    ROOT_DIR_LBA,
                    2048,
                    true,
                );
                write_cd_directory_record(
                    &mut user_data,
                    &mut offset,
                    &[1],
                    ROOT_DIR_LBA,
                    2048,
                    true,
                );
                write_cd_directory_record(
                    &mut user_data,
                    &mut offset,
                    b"YOURFOLD",
                    SOURCE_DIR_LBA,
                    2048,
                    true,
                );
            }
            SOURCE_DIR_LBA => {
                let mut offset = 0usize;
                write_cd_directory_record(
                    &mut user_data,
                    &mut offset,
                    &[0],
                    SOURCE_DIR_LBA,
                    2048,
                    true,
                );
                write_cd_directory_record(
                    &mut user_data,
                    &mut offset,
                    &[1],
                    ROOT_DIR_LBA,
                    2048,
                    true,
                );
                write_cd_directory_record(
                    &mut user_data,
                    &mut offset,
                    b"ROOT1.TXT;1",
                    ROOT1_LBA,
                    ROOT1_DATA.len() as u32,
                    false,
                );
                write_cd_directory_record(
                    &mut user_data,
                    &mut offset,
                    b"ROOT2.TXT;1",
                    ROOT2_LBA,
                    ROOT2_DATA.len() as u32,
                    false,
                );
                write_cd_directory_record(
                    &mut user_data,
                    &mut offset,
                    b"SUBONE",
                    SUBONE_DIR_LBA,
                    2048,
                    true,
                );
                write_cd_directory_record(
                    &mut user_data,
                    &mut offset,
                    b"SUBTWO",
                    SUBTWO_DIR_LBA,
                    2048,
                    true,
                );
                write_cd_directory_record(
                    &mut user_data,
                    &mut offset,
                    b"EMPTY",
                    EMPTY_DIR_LBA,
                    2048,
                    true,
                );
            }
            SUBONE_DIR_LBA => {
                let mut offset = 0usize;
                write_cd_directory_record(
                    &mut user_data,
                    &mut offset,
                    &[0],
                    SUBONE_DIR_LBA,
                    2048,
                    true,
                );
                write_cd_directory_record(
                    &mut user_data,
                    &mut offset,
                    &[1],
                    SOURCE_DIR_LBA,
                    2048,
                    true,
                );
                write_cd_directory_record(
                    &mut user_data,
                    &mut offset,
                    b"CHILD1.TXT;1",
                    CHILD1_LBA,
                    CHILD1_DATA.len() as u32,
                    false,
                );
            }
            SUBTWO_DIR_LBA => {
                let mut offset = 0usize;
                write_cd_directory_record(
                    &mut user_data,
                    &mut offset,
                    &[0],
                    SUBTWO_DIR_LBA,
                    2048,
                    true,
                );
                write_cd_directory_record(
                    &mut user_data,
                    &mut offset,
                    &[1],
                    SOURCE_DIR_LBA,
                    2048,
                    true,
                );
                write_cd_directory_record(
                    &mut user_data,
                    &mut offset,
                    b"CHILD2.TXT;1",
                    CHILD2_LBA,
                    CHILD2_DATA.len() as u32,
                    false,
                );
            }
            EMPTY_DIR_LBA => {
                let mut offset = 0usize;
                write_cd_directory_record(
                    &mut user_data,
                    &mut offset,
                    &[0],
                    EMPTY_DIR_LBA,
                    2048,
                    true,
                );
                write_cd_directory_record(
                    &mut user_data,
                    &mut offset,
                    &[1],
                    SOURCE_DIR_LBA,
                    2048,
                    true,
                );
            }
            ROOT1_LBA => {
                user_data[..ROOT1_DATA.len()].copy_from_slice(ROOT1_DATA);
            }
            ROOT2_LBA => {
                user_data[..ROOT2_DATA.len()].copy_from_slice(ROOT2_DATA);
            }
            CHILD1_LBA => {
                user_data[..CHILD1_DATA.len()].copy_from_slice(CHILD1_DATA);
            }
            CHILD2_LBA => {
                user_data[..CHILD2_DATA.len()].copy_from_slice(CHILD2_DATA);
            }
            _ => {
                user_data.fill(0x11);
            }
        }
        bin_data.extend_from_slice(&make_mode1_raw_sector(&user_data));
    }
    bin_data.extend_from_slice(&vec![0xAAu8; 2352 * 50]);
    device::cdrom::CdImage::from_cue(cue, bin_data).unwrap()
}

pub fn write_temp_mode2_multi_file_cdrom(name: &str) -> TempCdromCueFiles {
    const ROOT_DIRECTORY_LBA: u32 = 20;
    const README_LBA: u32 = 21;
    const SETUP_LBA: u32 = 22;
    const DATA_TRACK_SECTOR_COUNT: u32 = 150;
    const AUDIO_TRACK_TOTAL_SECTOR_COUNT: u32 = 152;

    let temp_stem = next_temp_cdrom_stem(name);
    let cue_path = std::env::temp_dir().join(format!("{temp_stem}.cue"));
    let data_track_path = std::env::temp_dir().join(format!("{temp_stem}_track01.bin"));
    let audio_track_path = std::env::temp_dir().join(format!("{temp_stem}_track02.bin"));

    let cue_content = format!(
        "FILE \"{}\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\nFILE \"{}\" BINARY\n  TRACK 02 AUDIO\n    INDEX 00 00:00:00\n    INDEX 01 00:02:00\n",
        data_track_path
            .file_name()
            .expect("track 1 file name")
            .to_string_lossy(),
        audio_track_path
            .file_name()
            .expect("track 2 file name")
            .to_string_lossy(),
    );
    std::fs::write(&cue_path, cue_content).expect("failed to write temp multi-file CUE");

    let mut data_track = Vec::with_capacity(DATA_TRACK_SECTOR_COUNT as usize * 2352);
    for sector_index in 0..DATA_TRACK_SECTOR_COUNT {
        let mut user_data = [0u8; 2048];
        match sector_index {
            16 => {
                user_data[0] = 1;
                user_data[1..6].copy_from_slice(b"CD001");
                user_data[6] = 1;
                user_data[8..40].fill(b' ');
                user_data[8..28].copy_from_slice(b"NEETAN MODE2 TEST CD");
                user_data[40..72].fill(b' ');
                user_data[40..51].copy_from_slice(b"MODE2_MULTI");
                write_both_endian_u32(&mut user_data[80..88], DATA_TRACK_SECTOR_COUNT);
                write_both_endian_u16(&mut user_data[120..124], 1);
                write_both_endian_u16(&mut user_data[124..128], 1);
                write_both_endian_u16(&mut user_data[128..132], 2048);
                write_both_endian_u32(&mut user_data[132..140], 10);
                let mut root_record_offset = 156usize;
                write_cd_directory_record(
                    &mut user_data,
                    &mut root_record_offset,
                    &[0],
                    ROOT_DIRECTORY_LBA,
                    2048,
                    true,
                );
            }
            17 => {
                user_data[0] = 0xFF;
                user_data[1..6].copy_from_slice(b"CD001");
                user_data[6] = 1;
            }
            ROOT_DIRECTORY_LBA => {
                let mut offset = 0usize;
                write_cd_directory_record(
                    &mut user_data,
                    &mut offset,
                    &[0],
                    ROOT_DIRECTORY_LBA,
                    2048,
                    true,
                );
                write_cd_directory_record(
                    &mut user_data,
                    &mut offset,
                    &[1],
                    ROOT_DIRECTORY_LBA,
                    2048,
                    true,
                );
                write_cd_directory_record(
                    &mut user_data,
                    &mut offset,
                    b"README.TXT;1",
                    README_LBA,
                    5,
                    false,
                );
                write_cd_directory_record(
                    &mut user_data,
                    &mut offset,
                    b"SETUP.EXE;1",
                    SETUP_LBA,
                    4,
                    false,
                );
            }
            README_LBA => {
                user_data[..5].copy_from_slice(b"HELLO");
            }
            SETUP_LBA => {
                user_data[..4].copy_from_slice(b"MZ\x90\x00");
            }
            _ => {
                user_data.fill(0x11);
            }
        }
        data_track.extend_from_slice(&make_mode2_raw_sector(&user_data));
    }

    let audio_track = vec![0xAAu8; AUDIO_TRACK_TOTAL_SECTOR_COUNT as usize * 2352];

    std::fs::write(&data_track_path, data_track).expect("failed to write temp data track");
    std::fs::write(&audio_track_path, audio_track).expect("failed to write temp audio track");

    TempCdromCueFiles {
        cue_path,
        bin_paths: vec![data_track_path, audio_track_path],
    }
}

/// Boots an HLE machine (PC-9821AP / IDE) with a test CD-ROM inserted.
/// The CD-ROM is inserted before boot so MSCDEX activates the Q: drive.
pub fn boot_hle_with_cdrom() -> machine::Pc9821Ap {
    let mut machine = create_hle_machine_ap();
    let cdimage = create_test_cdimage();
    machine.bus.insert_cdrom(cdimage);
    wait_for_prompt(
        &mut machine,
        HLE_BOOT_MAX_CYCLES,
        HLE_BOOT_CHECK_INTERVAL,
        "HLE DOS did not show prompt within 500000000 cycles (CDROM)",
    );
    machine
}

pub fn boot_hle_with_cdrom_path(path: &std::path::Path) -> machine::Pc9821Ap {
    let mut machine = create_hle_machine_ap();
    machine
        .insert_cdrom(path)
        .unwrap_or_else(|error| panic!("failed to insert CD-ROM {}: {error}", path.display()));
    wait_for_prompt(
        &mut machine,
        HLE_BOOT_MAX_CYCLES,
        HLE_BOOT_CHECK_INTERVAL,
        "HLE DOS did not show prompt within 500000000 cycles (CDROM path)",
    );

    machine
}

pub fn boot_hle_with_cdrom_image(cdimage: device::cdrom::CdImage) -> machine::Pc9821Ap {
    let mut machine = create_hle_machine_ap();
    machine.bus.insert_cdrom(cdimage);
    wait_for_prompt(
        &mut machine,
        HLE_BOOT_MAX_CYCLES,
        HLE_BOOT_CHECK_INTERVAL,
        "HLE DOS did not show prompt within 500000000 cycles (custom CDROM)",
    );

    machine
}

pub fn inject_and_run_generic_with_budget<const M: u8>(
    machine: &mut machine::Machine<cpu::I386<M>>,
    code: &[u8],
    budget: u64,
) {
    write_bytes(&mut machine.bus, INJECT_CODE_BASE, code);
    load_injected_i386_state(machine);
    machine.run_for(budget);
}

/// Injects ASCII characters into the PC-98 keyboard buffer.
pub fn type_string(bus: &mut machine::Pc9801Bus, text: &[u8]) {
    for &ch in text {
        let count = bus.read_byte_direct(0x0528);
        if count >= 0x10 {
            panic!("keyboard buffer full while injecting text");
        }
        let tail = read_word(bus, 0x0526) as u32;
        bus.write_byte(tail, ch);
        bus.write_byte(tail + 1, 0x00);
        let mut new_tail = tail + 2;
        if new_tail >= 0x0522 {
            new_tail = 0x0502;
        }
        write_word_raw(bus, 0x0526, new_tail as u16);
        bus.write_byte(0x0528, count + 1);
    }
}

pub const SCAN_DELETE: u8 = 0x39;
pub const SCAN_UP: u8 = 0x3A;
pub const SCAN_LEFT: u8 = 0x3B;

/// Injects a special key through the PC-98 keyboard pipeline.
/// Pushes make and break scancodes via the keyboard controller, then runs
/// the machine so the BIOS INT 09h handler processes them into KB_BUF.
pub fn type_special_key(machine: &mut machine::Pc9801Ra, scan_code: u8) {
    machine.bus.push_keyboard_scancode(scan_code); // press down
    machine.bus.push_keyboard_scancode(scan_code | 0x80); // release
    machine.run_for(100_000);
}

fn write_word_raw(bus: &mut machine::Pc9801Bus, addr: u32, value: u16) {
    bus.write_byte(addr, value as u8);
    bus.write_byte(addr + 1, (value >> 8) as u8);
}

/// Types a long string into the keyboard buffer, running the machine between
/// chunks to drain the 16-entry buffer. Use this for command strings longer
/// than ~14 characters.
pub fn type_string_long(machine: &mut machine::Pc9801Ra, text: &[u8]) {
    type_string_long_generic(machine, text);
}

/// Runs the machine until the shell prompt (`>`) reappears in text VRAM.
pub fn run_until_prompt(machine: &mut machine::Pc9801Ra) {
    run_until_prompt_generic(machine);
}

/// Types a long string for PC-9821AP machines.
pub fn type_string_long_ap(machine: &mut machine::Pc9821Ap, text: &[u8]) {
    type_string_long_generic(machine, text);
}

/// Runs the PC-9821AP machine until the shell prompt reappears.
pub fn run_until_prompt_ap(machine: &mut machine::Pc9821Ap) {
    run_until_prompt_generic(machine);
}
