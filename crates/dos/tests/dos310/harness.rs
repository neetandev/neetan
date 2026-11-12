use common::{Bus, CpuMode, HleDosVersion, MachineModel};
use device::{
    disk::{HddFormat, HddGeometry, HddImage},
    floppy::d88::{D88Disk, D88MediaType, D88Sector},
};

/// Machine of the suite: a PC-9801RS with a 386SX.
pub type Machine = machine_98::Pc9801Rs;

/// PC-9821AP of the CD-ROM tests: a 486DX2 with an IDE CD-ROM drive.
pub type CdromMachine = machine_98::Pc9821Ap;

/// Segment of the injected test code.
pub const INJECT_CODE_SEGMENT: u16 = 0x2000;
/// Linear address of the injected test code.
pub const INJECT_CODE_BASE: u32 = (INJECT_CODE_SEGMENT as u32) << 4;
/// Offset of the result area in the inject segment.
pub const INJECT_RESULT_OFFSET: u16 = 0x0100;
/// Linear address of the result area.
pub const INJECT_RESULT_BASE: u32 = INJECT_CODE_BASE + INJECT_RESULT_OFFSET as u32;
/// Offset of the data area (paths and buffers) in the inject segment.
pub const INJECT_DATA_OFFSET: u16 = 0x0200;
/// Linear address of the data area.
pub const INJECT_DATA_BASE: u32 = INJECT_CODE_BASE + INJECT_DATA_OFFSET as u32;
/// Cycle budget of injected code without disk access.
pub const INJECT_BUDGET: u64 = 50_000_000;
/// Cycle budget of injected code with disk access.
pub const INJECT_BUDGET_DISK_IO: u64 = 500_000_000;

/// DOS data segment of NeetanDos on the PC-98.
pub const DOS_DATA_SEGMENT: u16 = 0x0200;

/// Tiny .COM program for EXEC: terminates with exit code 42h.
/// MOV AH, 4Ch ; MOV AL, 42h ; INT 21h
pub const TEST_COM_PROGRAM: &[u8] = &[0xB4, 0x4C, 0xB0, 0x42, 0xCD, 0x21];

/// Content of TESTFILE.TXT on the test media.
pub const TEST_FILE_CONTENT: &[u8] = b"HELLO WORLD\r\n";

/// Cycle limit of a boot or a command until the prompt shows.
const BOOT_MAX_CYCLES: u64 = 500_000_000;
/// Cycles between two checks for the prompt.
const BOOT_CHECK_INTERVAL: u64 = 1_000_000;
/// Audio sample rate of the bus.
const SAMPLE_RATE: u32 = 48_000;
/// Character code of the shell prompt `>`.
const PROMPT_CHARACTER: u16 = 0x003E;
/// Columns of the text screen.
const TEXT_VRAM_COLUMNS: usize = 80;
/// Rows of the text screen.
const TEXT_VRAM_ROWS: usize = 25;
/// Cells of the text screen.
const TEXT_VRAM_CELL_COUNT: usize = TEXT_VRAM_COLUMNS * TEXT_VRAM_ROWS;
/// BDA byte of the connected floppy drives.
const FLOPPY_EQUIPMENT_ADDRESS: u32 = 0x055C;
/// BDA byte of the connected hard disks.
const HARD_DISK_EQUIPMENT_ADDRESS: u32 = 0x055D;
/// Equipment bit of the first floppy drive.
const FLOPPY_DRIVE_ONE: u8 = 0x01;
/// Equipment bit of the first hard disk.
const HARD_DISK_ONE: u8 = 0x01;
/// End of the BDA keyboard buffer.
const KEYBOARD_BUFFER_HEAD_LIMIT: u32 = 0x0522;
/// Start of the BDA keyboard buffer.
const KEYBOARD_BUFFER_START: u32 = 0x0502;
/// BDA word of the keyboard buffer tail.
const KEYBOARD_BUFFER_TAIL: u32 = 0x0526;
/// BDA byte of the keyboard buffer count.
const KEYBOARD_BUFFER_COUNT: u32 = 0x0528;
/// Entries of the keyboard buffer.
const KEYBOARD_BUFFER_CAPACITY: u8 = 0x10;
/// Size of a FAT directory entry.
const DIRECTORY_ENTRY_SIZE: usize = 32;
/// Archive attribute of the test files.
const FILE_ATTRIBUTE_ARCHIVE: u8 = 0x20;
/// DOS time of the test files: 12:00:00.
const FILE_TIME: u16 = 0x6000;
/// DOS date of the test files: 1995-01-01.
const FILE_DATE: u16 = 0x1E21;
/// Cylinders of a 2HD floppy.
const FLOPPY_CYLINDERS: usize = 77;
/// Heads of a 2HD floppy.
const FLOPPY_HEADS: usize = 2;
/// Sectors per track of a 2HD floppy.
const FLOPPY_SECTORS_PER_TRACK: usize = 8;
/// Sector size of a 2HD floppy.
const FLOPPY_SECTOR_SIZE: usize = 1024;
/// D88 size code of a 1024-byte sector.
const FLOPPY_SIZE_CODE: u8 = 3;
/// Root directory entries of a 2HD floppy.
const FLOPPY_ROOT_ENTRY_COUNT: usize = 192;
/// Sectors per FAT of a 2HD floppy.
const FLOPPY_SECTORS_PER_FAT: usize = 2;
/// Sectors of a 2HD floppy.
const FLOPPY_TOTAL_SECTORS: usize = FLOPPY_CYLINDERS * FLOPPY_HEADS * FLOPPY_SECTORS_PER_TRACK;
/// Media descriptor of a 2HD floppy.
const FLOPPY_MEDIA_DESCRIPTOR: u8 = 0xFE;
/// D88 density flag of an MFM sector.
const FLOPPY_MFM_FLAG: u8 = 0x40;
/// Cylinders of the test hard disk.
const HDD_CYLINDERS: u16 = 20;
/// Heads of the test hard disk.
const HDD_HEADS: u8 = 8;
/// Sectors per track of the test hard disk.
const HDD_SECTORS_PER_TRACK: u8 = 17;
/// Sector size of the test hard disk.
const HDD_SECTOR_SIZE: usize = 512;
/// Sectors per cluster of the test hard disk volume.
const HDD_SECTORS_PER_CLUSTER: u8 = 4;
/// Root directory entries of the test hard disk volume.
const HDD_ROOT_ENTRY_COUNT: usize = 512;
/// Sectors per FAT of the test hard disk volume.
const HDD_SECTORS_PER_FAT: usize = 16;
/// Media descriptor of a hard disk.
const HDD_MEDIA_DESCRIPTOR: u8 = 0xF8;
/// Reserved sectors of every test volume.
const RESERVED_SECTORS: usize = 1;
/// FAT copies of every test volume.
const FAT_COUNT: usize = 2;
/// End of chain marker of FAT12.
const FAT12_END_OF_CHAIN: u16 = 0x0FFF;
/// End of chain marker of FAT16.
const FAT16_END_OF_CHAIN: u16 = 0xFFFF;
/// Size of a PC-98 partition table entry.
const PARTITION_ENTRY_SIZE: usize = 32;
/// Partition entry byte of a bootable DOS partition.
const PARTITION_BOOTABLE_DOS: u8 = 0xA0;
/// Partition entry byte of an active FAT partition.
const PARTITION_ACTIVE_FAT: u8 = 0x91;

/// One file of a test volume.
#[derive(Clone, Copy)]
pub struct TestFile<'a> {
    /// 11-byte FCB form of the name.
    pub name: [u8; 11],
    /// File content.
    pub data: &'a [u8],
}

/// FAT variant of a test volume.
#[derive(Clone, Copy)]
enum FatKind {
    /// 12-bit FAT entries.
    Fat12,
    /// 16-bit FAT entries.
    Fat16,
}

/// Geometry of a FAT volume of a test image.
#[derive(Clone, Copy)]
struct VolumeSpec {
    /// Bytes per sector.
    sector_size: usize,
    /// Sectors per cluster.
    sectors_per_cluster: u8,
    /// Root directory entries.
    root_entry_count: usize,
    /// Sectors per FAT.
    sectors_per_fat: usize,
    /// Sectors of the volume.
    total_sectors: usize,
    /// Media descriptor byte.
    media_descriptor: u8,
    /// Sectors per track of the BPB.
    sectors_per_track: u16,
    /// Heads of the BPB.
    heads: u16,
    /// FAT variant.
    fat_kind: FatKind,
}

impl VolumeSpec {
    /// Returns the byte offset of the root directory in the volume.
    fn root_directory_offset(&self) -> usize {
        (RESERVED_SECTORS + FAT_COUNT * self.sectors_per_fat) * self.sector_size
    }

    /// Returns the byte offset of cluster 2 in the volume.
    fn data_offset(&self) -> usize {
        self.root_directory_offset()
            + (self.root_entry_count * DIRECTORY_ENTRY_SIZE).div_ceil(self.sector_size)
                * self.sector_size
    }
}

/// Returns the standard files of the test media.
pub fn standard_files() -> [TestFile<'static>; 2] {
    [
        TestFile {
            name: *b"TESTFILETXT",
            data: TEST_FILE_CONTENT,
        },
        TestFile {
            name: *b"TEST    COM",
            data: TEST_COM_PROGRAM,
        },
    ]
}

/// Writes `value` into the FAT entry of `cluster`.
fn set_fat_entry(fat: &mut [u8], fat_kind: FatKind, cluster: u16, value: u16) {
    match fat_kind {
        FatKind::Fat12 => {
            let value = value & 0x0FFF;
            let offset = usize::from(cluster) * 3 / 2;
            if cluster & 1 == 0 {
                fat[offset] = value as u8;
                fat[offset + 1] = (fat[offset + 1] & 0xF0) | (value >> 8) as u8;
            } else {
                fat[offset] = (fat[offset] & 0x0F) | ((value << 4) as u8);
                fat[offset + 1] = (value >> 4) as u8;
            }
        }
        FatKind::Fat16 => {
            let offset = usize::from(cluster) * 2;
            fat[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
        }
    }
}

/// Writes a FAT volume with `files` into `volume`.
fn write_volume(volume: &mut [u8], spec: VolumeSpec, files: &[TestFile<'_>]) {
    let boot_sector = &mut volume[..spec.sector_size];
    boot_sector[0..3].copy_from_slice(&[0xEB, 0x3C, 0x90]);
    boot_sector[3..11].copy_from_slice(b"NEETAN  ");
    boot_sector[11..13].copy_from_slice(&(spec.sector_size as u16).to_le_bytes());
    boot_sector[13] = spec.sectors_per_cluster;
    boot_sector[14..16].copy_from_slice(&(RESERVED_SECTORS as u16).to_le_bytes());
    boot_sector[16] = FAT_COUNT as u8;
    boot_sector[17..19].copy_from_slice(&(spec.root_entry_count as u16).to_le_bytes());
    boot_sector[19..21].copy_from_slice(&(spec.total_sectors as u16).to_le_bytes());
    boot_sector[21] = spec.media_descriptor;
    boot_sector[22..24].copy_from_slice(&(spec.sectors_per_fat as u16).to_le_bytes());
    boot_sector[24..26].copy_from_slice(&spec.sectors_per_track.to_le_bytes());
    boot_sector[26..28].copy_from_slice(&spec.heads.to_le_bytes());

    let fat_size = spec.sectors_per_fat * spec.sector_size;
    let mut fat = vec![0u8; fat_size];
    fat[0] = spec.media_descriptor;
    fat[1] = 0xFF;
    fat[2] = 0xFF;
    if let FatKind::Fat16 = spec.fat_kind {
        fat[3] = 0xFF;
    }
    let end_of_chain = match spec.fat_kind {
        FatKind::Fat12 => FAT12_END_OF_CHAIN,
        FatKind::Fat16 => FAT16_END_OF_CHAIN,
    };

    let cluster_size = spec.sector_size * usize::from(spec.sectors_per_cluster);
    let mut next_cluster = 2u16;
    for (index, file) in files.iter().enumerate() {
        let cluster_count = file.data.len().div_ceil(cluster_size).max(1);
        let start_cluster = next_cluster;
        for cluster_index in 0..cluster_count {
            let cluster = start_cluster + cluster_index as u16;
            let value = if cluster_index + 1 == cluster_count {
                end_of_chain
            } else {
                cluster + 1
            };
            set_fat_entry(&mut fat, spec.fat_kind, cluster, value);
            let file_offset = cluster_index * cluster_size;
            let length = cluster_size.min(file.data.len().saturating_sub(file_offset));
            let disk_offset = spec.data_offset() + usize::from(cluster - 2) * cluster_size;
            volume[disk_offset..disk_offset + length]
                .copy_from_slice(&file.data[file_offset..file_offset + length]);
        }
        let entry_offset = spec.root_directory_offset() + index * DIRECTORY_ENTRY_SIZE;
        let entry = &mut volume[entry_offset..entry_offset + DIRECTORY_ENTRY_SIZE];
        entry[0..11].copy_from_slice(&file.name);
        entry[11] = FILE_ATTRIBUTE_ARCHIVE;
        entry[22..24].copy_from_slice(&FILE_TIME.to_le_bytes());
        entry[24..26].copy_from_slice(&FILE_DATE.to_le_bytes());
        entry[26..28].copy_from_slice(&start_cluster.to_le_bytes());
        entry[28..32].copy_from_slice(&(file.data.len() as u32).to_le_bytes());
        next_cluster += cluster_count as u16;
    }

    for copy in 0..FAT_COUNT {
        let offset = (RESERVED_SECTORS + copy * spec.sectors_per_fat) * spec.sector_size;
        volume[offset..offset + fat_size].copy_from_slice(&fat);
    }
}

/// Builds a PC-98 2HD floppy (FAT12, 1024-byte sectors) with `files`.
pub fn create_floppy(files: &[TestFile<'_>]) -> device::floppy::FloppyImage {
    let spec = VolumeSpec {
        sector_size: FLOPPY_SECTOR_SIZE,
        sectors_per_cluster: 1,
        root_entry_count: FLOPPY_ROOT_ENTRY_COUNT,
        sectors_per_fat: FLOPPY_SECTORS_PER_FAT,
        total_sectors: FLOPPY_TOTAL_SECTORS,
        media_descriptor: FLOPPY_MEDIA_DESCRIPTOR,
        sectors_per_track: FLOPPY_SECTORS_PER_TRACK as u16,
        heads: FLOPPY_HEADS as u16,
        fat_kind: FatKind::Fat12,
    };
    let mut disk_data = vec![0u8; FLOPPY_TOTAL_SECTORS * FLOPPY_SECTOR_SIZE];
    write_volume(&mut disk_data, spec, files);

    let track_count = FLOPPY_CYLINDERS * FLOPPY_HEADS;
    let tracks = (0..track_count)
        .map(|track_index| {
            let sectors = (0..FLOPPY_SECTORS_PER_TRACK)
                .map(|sector_index| {
                    let offset = (track_index * FLOPPY_SECTORS_PER_TRACK + sector_index)
                        * FLOPPY_SECTOR_SIZE;
                    D88Sector {
                        cylinder: (track_index / FLOPPY_HEADS) as u8,
                        head: (track_index % FLOPPY_HEADS) as u8,
                        record: (sector_index + 1) as u8,
                        size_code: FLOPPY_SIZE_CODE,
                        sector_count: FLOPPY_SECTORS_PER_TRACK as u16,
                        mfm_flag: FLOPPY_MFM_FLAG,
                        deleted: 0,
                        status: 0,
                        reserved: [0; 5],
                        data: disk_data[offset..offset + FLOPPY_SECTOR_SIZE].to_vec(),
                        source_offset: None,
                    }
                })
                .collect();
            Some(sectors)
        })
        .collect();
    let disk = D88Disk::from_tracks("DOS310".to_string(), false, D88MediaType::Disk2HD, tracks);
    device::floppy::FloppyImage::from_d88(disk)
}

/// Builds a SASI hard disk with a PC-98 partition table and one FAT16
/// partition (512-byte sectors) that holds `files`.
pub fn create_hdd(files: &[TestFile<'_>]) -> HddImage {
    let total_sectors =
        usize::from(HDD_CYLINDERS) * usize::from(HDD_HEADS) * usize::from(HDD_SECTORS_PER_TRACK);
    let mut image = vec![0u8; total_sectors * HDD_SECTOR_SIZE];
    image[0..2].copy_from_slice(&[0xEB, 0x1E]);
    image[4..8].copy_from_slice(b"IPL1");

    let partition = &mut image[HDD_SECTOR_SIZE..HDD_SECTOR_SIZE + PARTITION_ENTRY_SIZE];
    partition[0] = PARTITION_BOOTABLE_DOS;
    partition[1] = PARTITION_ACTIVE_FAT;
    partition[6] = 1;
    partition[10] = 1;
    partition[12] = HDD_SECTORS_PER_TRACK - 1;
    partition[13] = HDD_HEADS - 1;
    partition[14..16].copy_from_slice(&(HDD_CYLINDERS - 1).to_le_bytes());
    partition[16..27].copy_from_slice(b"MS-DOS 3.10");

    let partition_sector = usize::from(HDD_HEADS) * usize::from(HDD_SECTORS_PER_TRACK);
    let spec = VolumeSpec {
        sector_size: HDD_SECTOR_SIZE,
        sectors_per_cluster: HDD_SECTORS_PER_CLUSTER,
        root_entry_count: HDD_ROOT_ENTRY_COUNT,
        sectors_per_fat: HDD_SECTORS_PER_FAT,
        total_sectors: total_sectors - partition_sector,
        media_descriptor: HDD_MEDIA_DESCRIPTOR,
        sectors_per_track: u16::from(HDD_SECTORS_PER_TRACK),
        heads: u16::from(HDD_HEADS),
        fat_kind: FatKind::Fat16,
    };
    write_volume(
        &mut image[partition_sector * HDD_SECTOR_SIZE..],
        spec,
        files,
    );

    let geometry = HddGeometry {
        cylinders: HDD_CYLINDERS,
        heads: HDD_HEADS,
        sectors_per_track: HDD_SECTORS_PER_TRACK,
        sector_size: HDD_SECTOR_SIZE as u16,
    };
    HddImage::from_raw(geometry, HddFormat::Nhd, image)
}

/// Creates a PC-9801RS whose HLE DOS emulates `version`.
pub fn create_machine_with_version(version: HleDosVersion) -> Machine {
    let mut bus = machine_98::Pc9801Bus::new(MachineModel::PC9801RS, CpuMode::High, SAMPLE_RATE);
    bus.load_font_rom(machine_98::BUILTIN_FONT_ROM);
    bus.set_dos_version(version);
    Machine::new(cpu::I386::new(), bus)
}

/// Creates a PC-9801RS whose HLE DOS emulates MS-DOS 3.10.
pub fn create_machine() -> Machine {
    create_machine_with_version(HleDosVersion::Dos310)
}

/// Runs `machine` until the shell prompt is visible.
pub fn wait_for_prompt<const CPU_MODEL: u8>(
    machine: &mut machine_98::Pc98Machine<cpu::I386<CPU_MODEL>>,
) {
    let mut total_cycles = 0;
    loop {
        total_cycles += machine.run_for(BOOT_CHECK_INTERVAL);
        if prompt_visible(&machine.bus) {
            return;
        }
        assert!(
            total_cycles < BOOT_MAX_CYCLES,
            "HLE DOS did not show the prompt within {BOOT_MAX_CYCLES} cycles"
        );
    }
}

/// Boots the HLE DOS 3.10 without media.
pub fn boot() -> Machine {
    let mut machine = create_machine();
    wait_for_prompt(&mut machine);
    machine
}

/// Boots the HLE DOS of `version` without media.
pub fn boot_with_version(version: HleDosVersion) -> Machine {
    let mut machine = create_machine_with_version(version);
    wait_for_prompt(&mut machine);
    machine
}

/// Boots the HLE DOS 3.10 directly with `floppy` in drive A:.
pub fn boot_with_floppy(floppy: device::floppy::FloppyImage) -> Machine {
    let mut machine = create_machine();
    machine.bus.set_boot_device(machine_98::BootDevice::Dos);
    machine.bus.insert_floppy(0, floppy, None);
    machine
        .bus
        .write_byte(FLOPPY_EQUIPMENT_ADDRESS, FLOPPY_DRIVE_ONE);
    wait_for_prompt(&mut machine);
    machine
}

/// Boots the HLE DOS 3.10 directly with `hdd` as the first SASI hard disk.
/// The hard disk is drive A:.
pub fn boot_with_hdd(hdd: HddImage) -> Machine {
    let mut machine = create_machine();
    machine.bus.set_boot_device(machine_98::BootDevice::Dos);
    machine.bus.insert_hdd(0, hdd, None);
    machine
        .bus
        .write_byte(HARD_DISK_EQUIPMENT_ADDRESS, HARD_DISK_ONE);
    wait_for_prompt(&mut machine);
    machine
}

/// Boots the HLE DOS 3.10 directly with `floppy` in the first floppy drive
/// and `hdd` as the first SASI hard disk.
pub fn boot_with_floppy_and_hdd(floppy: device::floppy::FloppyImage, hdd: HddImage) -> Machine {
    let mut machine = create_machine();
    machine.bus.set_boot_device(machine_98::BootDevice::Dos);
    machine.bus.insert_floppy(0, floppy, None);
    machine.bus.insert_hdd(0, hdd, None);
    machine
        .bus
        .write_byte(FLOPPY_EQUIPMENT_ADDRESS, FLOPPY_DRIVE_ONE);
    machine
        .bus
        .write_byte(HARD_DISK_EQUIPMENT_ADDRESS, HARD_DISK_ONE);
    wait_for_prompt(&mut machine);
    machine
}

/// Boots the HLE DOS 3.10 on a PC-9821AP with its CD-ROM drive and no disc.
pub fn boot_cdrom_machine() -> CdromMachine {
    let mut bus = machine_98::Pc9801Bus::new(MachineModel::PC9821AP, CpuMode::High, SAMPLE_RATE);
    bus.load_font_rom(machine_98::BUILTIN_FONT_ROM);
    bus.set_dos_version(HleDosVersion::Dos310);
    let mut machine = CdromMachine::new(cpu::I386::new(), bus);
    wait_for_prompt(&mut machine);
    machine
}

/// Returns true while the shell prompt is visible in text VRAM.
pub fn prompt_visible(bus: &machine_98::Pc9801Bus) -> bool {
    text_vram_codes(bus).contains(&PROMPT_CHARACTER)
}

/// Returns the character codes of the text screen.
fn text_vram_codes(bus: &machine_98::Pc9801Bus) -> Vec<u16> {
    bus.text_vram()
        .as_chunks::<2>()
        .0
        .iter()
        .take(TEXT_VRAM_CELL_COUNT)
        .map(|cell| u16::from_le_bytes([cell[0], cell[1]]))
        .collect()
}

/// Returns true when text VRAM holds `text`. A full-width character takes
/// two cells: the character and an empty right half.
pub fn screen_contains(bus: &machine_98::Pc9801Bus, text: &str) -> bool {
    let cells: Vec<common::JisChar> = bus
        .text_vram()
        .as_chunks::<2>()
        .0
        .iter()
        .take(TEXT_VRAM_CELL_COUNT)
        .map(|cell| common::JisChar::from_vram_bytes(cell[0], cell[1]))
        .collect();
    let mut expected = Vec::new();
    for character in common::str_to_jis(text) {
        expected.push(character);
        if !character.is_ank() {
            expected.push(common::JisChar::from_u16(0x0000));
        }
    }
    cells
        .windows(expected.len())
        .any(|window| window == expected)
}

/// Returns the ASCII text of every row of text VRAM.
pub fn screen_rows(bus: &machine_98::Pc9801Bus) -> Vec<String> {
    text_vram_codes(bus)
        .chunks(TEXT_VRAM_COLUMNS)
        .take(TEXT_VRAM_ROWS)
        .map(|row| {
            row.iter()
                .map(|&code| match u8::try_from(code) {
                    Ok(byte) if byte.is_ascii_graphic() || byte == b' ' => char::from(byte),
                    Ok(_) | Err(_) => ' ',
                })
                .collect()
        })
        .collect()
}

/// Injects ASCII characters into the PC-98 keyboard buffer.
pub fn type_string(bus: &mut machine_98::Pc9801Bus, text: &[u8]) {
    for &character in text {
        let count = bus.read_byte_direct(KEYBOARD_BUFFER_COUNT);
        assert!(count < KEYBOARD_BUFFER_CAPACITY, "keyboard buffer full");
        let tail = u32::from(read_word(bus, KEYBOARD_BUFFER_TAIL));
        bus.write_byte(tail, character);
        bus.write_byte(tail + 1, 0x00);
        let mut next_tail = tail + 2;
        if next_tail >= KEYBOARD_BUFFER_HEAD_LIMIT {
            next_tail = KEYBOARD_BUFFER_START;
        }
        bus.write_byte(KEYBOARD_BUFFER_TAIL, next_tail as u8);
        bus.write_byte(KEYBOARD_BUFFER_TAIL + 1, (next_tail >> 8) as u8);
        bus.write_byte(KEYBOARD_BUFFER_COUNT, count + 1);
    }
}

/// Writes `data` at the linear address `address`.
pub fn write_bytes(bus: &mut machine_98::Pc9801Bus, address: u32, data: &[u8]) {
    for (index, &byte) in data.iter().enumerate() {
        bus.write_byte(address + index as u32, byte);
    }
}

/// Runs `code` at `INJECT_CODE_SEGMENT:0000` for `budget` cycles with CS,
/// DS, ES and SS set to the inject segment.
pub fn inject_and_run_with_budget<const CPU_MODEL: u8>(
    machine: &mut machine_98::Pc98Machine<cpu::I386<CPU_MODEL>>,
    code: &[u8],
    budget: u64,
) {
    write_bytes(&mut machine.bus, INJECT_CODE_BASE, code);
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
    machine.run_for(budget);
}

/// Runs `code` with the default budget.
pub fn inject_and_run<const CPU_MODEL: u8>(
    machine: &mut machine_98::Pc98Machine<cpu::I386<CPU_MODEL>>,
    code: &[u8],
) {
    inject_and_run_with_budget(machine, code, INJECT_BUDGET);
}

/// Reads the byte at the linear address `address`.
pub fn read_byte(bus: &machine_98::Pc9801Bus, address: u32) -> u8 {
    bus.read_byte_direct(address)
}

/// Reads the word at the linear address `address`.
pub fn read_word(bus: &machine_98::Pc9801Bus, address: u32) -> u16 {
    u16::from_le_bytes([
        bus.read_byte_direct(address),
        bus.read_byte_direct(address + 1),
    ])
}

/// Reads a far pointer and returns `(segment, offset)`.
pub fn read_far_pointer(bus: &machine_98::Pc9801Bus, address: u32) -> (u16, u16) {
    (read_word(bus, address + 2), read_word(bus, address))
}

/// Reads `length` bytes at the linear address `address`.
pub fn read_bytes(bus: &machine_98::Pc9801Bus, address: u32, length: usize) -> Vec<u8> {
    (0..length)
        .map(|index| bus.read_byte_direct(address + index as u32))
        .collect()
}

/// Returns the linear address of a far pointer `(segment, offset)`.
pub fn far_to_linear((segment, offset): (u16, u16)) -> u32 {
    (u32::from(segment) << 4) + u32::from(offset)
}

/// Reads the word at `offset` of the result area.
pub fn result_word(bus: &machine_98::Pc9801Bus, offset: u32) -> u16 {
    read_word(bus, INJECT_RESULT_BASE + offset)
}

/// Returns the far pointer `(segment, offset)` of `INT 21h AH=52h`.
pub fn list_of_lists_pointer<const CPU_MODEL: u8>(
    machine: &mut machine_98::Pc98Machine<cpu::I386<CPU_MODEL>>,
) -> (u16, u16) {
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x52,             // MOV AH, 52h
        0xCD, 0x21,             // INT 21h
        0x89, 0x1E, 0x00, 0x01, // MOV [0100h], BX
        0x8C, 0x06, 0x02, 0x01, // MOV [0102h], ES
        0xFA,                   // CLI
        0xF4,                   // HLT
    ];
    inject_and_run(machine, code);
    (result_word(&machine.bus, 2), result_word(&machine.bus, 0))
}
