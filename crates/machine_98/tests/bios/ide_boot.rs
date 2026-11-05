use common::{Bus, Cpu, DiskIo};
use device::disk::{
    HddFormat, HddGeometry, HddImage, HddSizeType, blank_hdd_image,
    format::{fat_region, hdd_boot_sector, hdd_bpb_params, pc98_partition_table_sector},
};
use dos::{drive_map::pc98, filesystem::fat_api::FatFs};

use super::{
    TEST_CODE, create_machine_pc9821as, create_machine_pc9821as_empty_hdd,
    create_machine_pc9821as_hdd, read_ram_u16, write_bytes,
};

const BDA_DISK_EQUIP: usize = 0x055C;
const BDA_F2HD_MODE: usize = 0x0493;
const BDA_BOOT_DEVICE: usize = 0x0584;
const BDA_DISK_PRM0: usize = 0x05E8;
const BDA_DISK_PRM1: usize = 0x05EC;

#[test]
fn ide_initialization_pc9821() {
    let mut machine = create_machine_pc9821as_hdd();
    let _cycles = boot_to_halt_hdd!(machine);
    let state = machine.inspection_state();

    let disk_equip = read_ram_u16(&state.memory.ram, BDA_DISK_EQUIP);
    assert_eq!(
        disk_equip, 0x0103,
        "DISK_EQUIP should have IDE drive 0 + 2 FDD drives (got {disk_equip:#06X})"
    );
    assert_eq!(
        state.memory.ram[BDA_BOOT_DEVICE], 0x80,
        "BOOT_DEVICE should be 0x80 for IDE-0"
    );
    assert_eq!(state.memory.ram[BDA_F2HD_MODE], 0xFF, "F2HD_MODE");
    assert_eq!(state.memory.ram[0x1FC00], 0xFA, "Boot sector byte 0 (CLI)");
    assert_eq!(state.memory.ram[0x1FC01], 0xF4, "Boot sector byte 1 (HLT)");
}

#[test]
fn ide_disk_parameter_pointers_reach_drive_tables() {
    use common::Bus as _;

    let mut machine = create_machine_pc9821as_hdd();
    let _cycles = boot_to_halt_hdd!(machine);
    let state = machine.inspection_state();

    let table_address = |pointer: usize| {
        let offset = u32::from(read_ram_u16(&state.memory.ram, pointer));
        let segment = u32::from(read_ram_u16(&state.memory.ram, pointer + 2));
        (segment << 4) + offset
    };
    let drive0 = table_address(BDA_DISK_PRM0);
    let drive1 = table_address(BDA_DISK_PRM1);
    assert_ne!(drive0, 0, "DISK_PRM0 must point at a parameter table");
    assert_ne!(drive1, 0, "DISK_PRM1 must point at a parameter table");

    let table: Vec<u8> = (0..6).map(|i| machine.bus.read_byte(drive0 + i)).collect();
    assert_eq!(
        table,
        [9, 0, 8, 8, 0x0F, 0x0F],
        "10 cylinders of 8 heads x 17 sectors"
    );
    assert_eq!(machine.bus.read_byte(drive1 + 2), 0x00, "no IDE HD #1");
}

#[test]
fn ide_empty_boot_sector_falls_through_to_fdd() {
    let mut machine = create_machine_pc9821as_empty_hdd();
    let _cycles = boot_to_halt!(machine);
    let state = machine.inspection_state();
    assert_eq!(
        state.memory.ram[BDA_BOOT_DEVICE], 0x90,
        "BOOT_DEVICE should be 0x90 for FDD-0 (got {:#04X})",
        state.memory.ram[BDA_BOOT_DEVICE]
    );
    assert_eq!(state.memory.ram[0x1FC00], 0xFA, "Boot sector byte 0 (CLI)");
    assert_eq!(state.memory.ram[0x1FC01], 0xF4, "Boot sector byte 1 (HLT)");
}

/// Builds an IDE image whose sectors hold their LBA, with a halt boot sector.
fn make_lba_marked_ide_hdd() -> HddImage {
    let geometry = HddGeometry {
        cylinders: 20,
        heads: 4,
        sectors_per_track: 17,
        sector_size: 512,
    };
    let mut data = vec![0u8; geometry.total_bytes() as usize];
    for (lba, sector) in data.as_chunks_mut::<512>().0.iter_mut().enumerate() {
        sector[..4].copy_from_slice(&(lba as u32).to_le_bytes());
    }
    data[0] = 0xFA; // CLI
    data[1] = 0xF4; // HLT
    HddImage::from_raw(geometry, HddFormat::Hdi, data)
}

#[test]
fn ide_boot_sets_the_drive_translation_to_8_heads_and_17_sectors() {
    let mut machine = create_machine_pc9821as();
    machine.bus.insert_hdd(0, make_lba_marked_ide_hdd(), None);
    let _cycles = boot_to_halt_hdd!(machine);

    machine.bus.io_write_byte(0x064C, 0xA2);
    machine.bus.io_write_byte(0x0644, 0x01);
    machine.bus.io_write_byte(0x0646, 0x01);
    machine.bus.io_write_byte(0x0648, 0x01);
    machine.bus.io_write_byte(0x064A, 0x00);
    machine.bus.io_write_byte(0x064E, 0x20);
    machine.run_for(100_000);

    let low = machine.bus.io_read_word(0x0640);
    let high = machine.bus.io_read_word(0x0640);
    let lba = u32::from(low) | (u32::from(high) << 16);
    assert_eq!(lba, 170, "CHS 1/2/1 is LBA 170 with 8 heads x 17 sectors");
}

/// Sector size of the IDE test images.
const SECTOR_SIZE: u16 = 512;

/// A test image as a disk addressed by LBA in the image geometry.
struct ImageDisk<'a> {
    image: &'a mut HddImage,
}

impl DiskIo for ImageDisk<'_> {
    fn read_sectors(&mut self, _device_id: u8, lba: u32, count: u32) -> Result<Vec<u8>, u8> {
        let mut data = Vec::new();
        for offset in 0..count {
            data.extend_from_slice(self.image.read_sector(lba + offset).ok_or(0x40u8)?);
        }
        Ok(data)
    }

    fn write_sectors(&mut self, _device_id: u8, lba: u32, data: &[u8]) -> Result<(), u8> {
        for (offset, sector) in data.chunks_exact(usize::from(SECTOR_SIZE)).enumerate() {
            if !self.image.write_sector(lba + offset as u32, sector) {
                return Err(0x40);
            }
        }
        Ok(())
    }

    fn sector_size(&self, _device_id: u8) -> Option<u16> {
        Some(SECTOR_SIZE)
    }

    fn total_sectors(&self, _device_id: u8) -> Option<u32> {
        Some(self.image.geometry.total_sectors())
    }

    fn drive_geometry(&self, _device_id: u8) -> Option<(u16, u8, u8)> {
        let geometry = self.image.geometry;
        Some((
            geometry.cylinders,
            geometry.heads,
            geometry.sectors_per_track,
        ))
    }
}

/// The IDE drive 0 of a halted PC-9821 read through INT 1Bh with CHS
/// addresses in the geometry that function 84h reports.
struct Int1bhDisk<'a> {
    machine: &'a mut machine_98::Pc9821As,
    geometry: (u16, u8, u8),
}

/// Linear address of the INT 1Bh transfer buffer.
const INT1BH_BUFFER: u32 = 0x20000;

/// Sectors one INT 1Bh read transfers at most.
const INT1BH_SECTORS_PER_CALL: u32 = 64;

impl Int1bhDisk<'_> {
    /// Runs INT 1Bh with ES:BP at the transfer buffer and returns AX, BX, CX
    /// and DX.
    fn call(&mut self, ax: u16, bx: u16, cx: u16, dx: u16) -> [u16; 4] {
        #[rustfmt::skip]
        let code = [
            0xB8, 0x00, 0x20,                   // MOV AX, 0x2000
            0x8E, 0xC0,                         // MOV ES, AX
            0x31, 0xED,                         // XOR BP, BP
            0xBB, bx as u8, (bx >> 8) as u8,    // MOV BX, bx
            0xB9, cx as u8, (cx >> 8) as u8,    // MOV CX, cx
            0xBA, dx as u8, (dx >> 8) as u8,    // MOV DX, dx
            0xB8, ax as u8, (ax >> 8) as u8,    // MOV AX, ax
            0xCD, 0x1B,                         // INT 0x1B
            0xA3, 0x00, 0x06,                   // MOV [0x0600], AX
            0x89, 0x1E, 0x02, 0x06,             // MOV [0x0602], BX
            0x89, 0x0E, 0x04, 0x06,             // MOV [0x0604], CX
            0x89, 0x16, 0x06, 0x06,             // MOV [0x0606], DX
            0xF4,                               // HLT
        ];
        write_bytes(&mut self.machine.bus, TEST_CODE, &code);
        self.machine.cpu.load_state(&{
            let mut state = cpu::I386State {
                ip: TEST_CODE as u16,
                ..Default::default()
            };
            state.set_esp(0x4000);
            state
        });
        while {
            self.machine.run_for(20_000);
            !self.machine.cpu.halted()
        } {}
        let ram = &self.machine.inspection_state().memory.ram;
        [0x0600, 0x0602, 0x0604, 0x0606].map(|address| read_ram_u16(ram, address))
    }
}

impl DiskIo for Int1bhDisk<'_> {
    fn read_sectors(&mut self, _device_id: u8, lba: u32, count: u32) -> Result<Vec<u8>, u8> {
        let (_, heads, sectors_per_track) = self.geometry;
        let mut data = Vec::new();
        let mut done = 0;
        while done < count {
            let chunk = (count - done).min(INT1BH_SECTORS_PER_CALL);
            let current = lba + done;
            let track = current / u32::from(sectors_per_track);
            let cylinder = (track / u32::from(heads)) as u16;
            let head = (track % u32::from(heads)) as u16;
            let sector = (current % u32::from(sectors_per_track)) as u16;
            let [ax, ..] = self.call(
                0x0680,
                (chunk * u32::from(SECTOR_SIZE)) as u16,
                cylinder,
                (head << 8) | sector,
            );
            if ax >> 8 >= 0x20 {
                return Err((ax >> 8) as u8);
            }
            let ram = &self.machine.inspection_state().memory.ram;
            let start = INT1BH_BUFFER as usize;
            data.extend_from_slice(&ram[start..start + (chunk * u32::from(SECTOR_SIZE)) as usize]);
            done += chunk;
        }
        Ok(data)
    }

    fn write_sectors(&mut self, _device_id: u8, _lba: u32, _data: &[u8]) -> Result<(), u8> {
        Err(0x40)
    }

    fn sector_size(&self, _device_id: u8) -> Option<u16> {
        Some(SECTOR_SIZE)
    }

    fn total_sectors(&self, _device_id: u8) -> Option<u32> {
        let (cylinders, heads, sectors_per_track) = self.geometry;
        Some(u32::from(cylinders) * u32::from(heads) * u32::from(sectors_per_track))
    }

    fn drive_geometry(&self, _device_id: u8) -> Option<(u16, u8, u8)> {
        Some(self.geometry)
    }
}

/// Builds an IDE image partitioned and formatted in its image geometry, the
/// layout of an image written before the BIOS geometry. The partition starts
/// on cylinder 1 and holds the file `\DATA\FILE.BIN`.
fn make_image_geometry_layout(
    cylinders: u16,
    heads: u8,
    sectors_per_track: u8,
    content: &[u8],
) -> HddImage {
    let geometry = HddGeometry {
        cylinders,
        heads,
        sectors_per_track,
        sector_size: SECTOR_SIZE,
    };
    let mut image = HddImage::from_raw(
        geometry,
        HddFormat::Hdi,
        vec![0u8; geometry.total_bytes() as usize],
    );
    let start = u32::from(heads) * u32::from(sectors_per_track);
    let end_cylinder = cylinders - 1;
    let partition_sectors = u32::from(end_cylinder - 1) * start;

    let mut table = vec![0u8; usize::from(SECTOR_SIZE)];
    table[0] = 0xA1;
    table[1] = 0x91;
    table[6..8].copy_from_slice(&1u16.to_le_bytes());
    table[10..12].copy_from_slice(&1u16.to_le_bytes());
    table[14..16].copy_from_slice(&end_cylinder.to_le_bytes());
    table[16..32].copy_from_slice(b"MS-DOS 6.20     ");
    image.write_sector(1, &table);

    format_volume_with_file(&mut image, start, partition_sectors, content);
    image
}

/// Builds an IDE image of a disk type formatted by `format_hdd_image` before
/// the BIOS geometry: the volume starts on head 1 of cylinder 0 of the image
/// geometry, fills the rest of the disk and holds the file
/// `\DATA\FILE.BIN`.
fn make_neetan_formatted_layout(size: HddSizeType, content: &[u8]) -> HddImage {
    let mut image = blank_hdd_image(size);
    let geometry = image.geometry;
    let total_sectors = geometry.total_sectors();
    let start = u32::from(geometry.sectors_per_track);
    image.write_sector(
        1,
        &pc98_partition_table_sector(
            SECTOR_SIZE,
            total_sectors,
            geometry.heads,
            geometry.sectors_per_track,
        ),
    );
    format_volume_with_file(&mut image, start, total_sectors - start, content);
    image
}

/// Formats a FAT volume of `partition_sectors` sectors at `start` in the
/// image geometry and writes `content` to `\DATA\FILE.BIN`.
fn format_volume_with_file(
    image: &mut HddImage,
    start: u32,
    partition_sectors: u32,
    content: &[u8],
) {
    let geometry = image.geometry;
    let bpb = hdd_bpb_params(SECTOR_SIZE, partition_sectors);
    let boot = hdd_boot_sector(
        &bpb,
        SECTOR_SIZE,
        partition_sectors,
        geometry.sectors_per_track,
        geometry.heads,
        start,
    );
    image.write_sector(start, &boot);
    let fat = fat_region(&bpb, SECTOR_SIZE);
    for copy in 0..u32::from(bpb.num_fats) {
        let base = start + u32::from(bpb.reserved_sectors) + copy * u32::from(bpb.sectors_per_fat);
        for (offset, sector) in fat.chunks_exact(usize::from(SECTOR_SIZE)).enumerate() {
            image.write_sector(base + offset as u32, sector);
        }
    }

    let entry = pc98::drive_entry(0x80).unwrap();
    let mut disk = ImageDisk { image };
    let mut volume = FatFs::mount(&mut disk, &entry).unwrap();
    volume.mkdir_p(b"\\DATA", 0, 0x21).unwrap();
    volume
        .write_file(b"\\DATA\\FILE.BIN", content, 0x20, 0, 0x21)
        .unwrap();
    volume.flush().unwrap();
}

/// Returns the content of the test file.
fn test_file_content() -> Vec<u8> {
    (0..300_000u32)
        .map(|index| (index * 7 + index / 251) as u8)
        .collect()
}

/// Mounts an image laid out in the image geometry on IDE drive 0 and reads
/// its file back through INT 1Bh in the geometry function 84h reports.
fn assert_file_survives_migration(image: HddImage, bios_cylinders: u16) {
    let content = test_file_content();

    let mut machine = create_machine_pc9821as();
    machine.bus.insert_hdd(0, image, None);
    let _cycles = boot_to_halt!(machine);

    let mut disk = Int1bhDisk {
        machine: &mut machine,
        geometry: (0, 0, 0),
    };
    let [ax, bx, cx, dx] = disk.call(0x8480, 0, 0, 0);
    assert_eq!(
        (ax >> 8, bx, cx, dx),
        (0x0F, 0x0200, bios_cylinders - 1, 0x0811)
    );
    disk.geometry = (cx + 1, (dx >> 8) as u8, dx as u8);

    let entry = pc98::drive_entry(0x80).unwrap();
    let mut volume = FatFs::mount(&mut disk, &entry).unwrap();
    let file = volume.read_file(b"\\DATA\\FILE.BIN").unwrap();
    assert!(file == content, "the file reads back unchanged");
}

#[test]
fn ide_mount_migrates_a_10_head_17_sector_layout() {
    let image = make_image_geometry_layout(977, 10, 17, &test_file_content());
    assert_file_survives_migration(image, 1221);
}

#[test]
fn ide_mount_migrates_a_15_head_28_sector_layout() {
    let image = make_image_geometry_layout(977, 15, 28, &test_file_content());
    assert_file_survives_migration(image, 3017);
}

#[test]
fn ide_mount_migrates_a_16_head_63_sector_layout() {
    let image = make_image_geometry_layout(1015, 16, 63, &test_file_content());
    assert_file_survives_migration(image, 7522);
}

#[test]
fn ide_mount_migrates_a_neetan_formatted_ide500_image_by_enlarging() {
    let image = make_neetan_formatted_layout(HddSizeType::IdeMb500, &test_file_content());
    assert_file_survives_migration(image, 7530);
}

#[test]
fn ide_mount_migrates_a_neetan_formatted_ide40_image_by_enlarging() {
    let image = make_neetan_formatted_layout(HddSizeType::IdeMb40, &test_file_content());
    assert_file_survives_migration(image, 612);
}
