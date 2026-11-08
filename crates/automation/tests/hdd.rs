//! In-memory hard disk synthesis and formatting.
//!
//! `create-hdd!` builds a zeroed image in memory and mounts it RAM-backed, and
//! `format-hdd!` lays down the partition table of the machine family and an
//! empty FAT volume in place. The committed script drives the full Scheme
//! wrapper, native, and session path, and confirms the formatted disk survives
//! a hard reset. The session tests compare every formatted disk type with the
//! layout builders and with the FORMAT command of the HLE DOS, and check the
//! volume with the FAT code of the HLE DOS.

#[path = "common/harness.rs"]
mod harness;

use std::path::Path;

use automation::{AutomationSession, MediaKind, OpError};
use common::{BootDevice, MachineModel, RunTarget};
use device::disk::{
    HddGeometry, HddImage, HddSizeType, blank_hdd_image,
    format::{
        PartitionTableType, at_master_boot_record, fat_region, format_hdd_image, hdd_boot_sector,
        hdd_bpb_params, pc98_nec_partition_table_sector,
    },
    load_hdd_image, pc98_ide,
};
use dos::{
    DiskIo,
    drive_map::{PartitionScheme, pc98},
    filesystem::fat_api::FatFs,
};
use harness::{assert_completed_ok, make_session, run_committed_script};
use machine_factory::config::{EmulatorConfig, MachineSelection};

/// Tick budget of every frame run.
const MAX_TICKS: u64 = u64::MAX / 4;

/// Tick interval between two input drains.
const DRAIN_INTERVAL_TICKS: u64 = 1_000_000;

/// Frames between two checks of the screen while the HLE DOS works.
const POLL_FRAMES: u64 = 10;

/// Screen checks before a wait for the HLE DOS gives up.
const POLL_LIMIT: usize = 300;

/// Prompt of the HLE DOS on the hard disk drive.
const DRIVE_A_PROMPT: &str = "A:\u{a5}>";

/// Question of the FORMAT command before it writes the disk.
const FORMAT_QUESTION: &str = "Proceed with Format (Y/N)?";

/// Frames between two typed characters.
const TYPING_SPACING_FRAMES: u64 = 2;

/// Device id of the first fixed disk in the HLE DOS.
const FIRST_FIXED_DISK: u8 = 0x80;

/// Disk status of a failed sector access.
const SECTOR_ACCESS_ERROR: u8 = 0x40;

/// File the tests write into a formatted volume.
const TEST_FILE: &[u8] = b"TEST.TXT";

/// Contents of the file the tests write into a formatted volume.
const TEST_CONTENTS: &[u8] = b"NEETAN\r\n";

/// Directory the HLE DOS creates in a formatted volume.
const TEST_DIRECTORY: &[u8] = b"NEETAN";

/// PC-98 SASI disk types with the model that mounts them.
const PC98_SASI_DISKS: [(MachineModel, HddSizeType); 7] = [
    (MachineModel::PC9801VM, HddSizeType::Mb5),
    (MachineModel::PC9801VM, HddSizeType::Mb10),
    (MachineModel::PC9801VM, HddSizeType::Mb15),
    (MachineModel::PC9801VM, HddSizeType::Mb20),
    (MachineModel::PC9801VM, HddSizeType::Mb30),
    (MachineModel::PC9801VM, HddSizeType::Mb40),
    (MachineModel::PC9821AP, HddSizeType::Mb20),
];

/// PC-98 IDE disk types, mounted on the PC-9821AP.
const PC98_IDE_DISKS: [HddSizeType; 5] = [
    HddSizeType::IdeMb40,
    HddSizeType::IdeMb80,
    HddSizeType::IdeMb120,
    HddSizeType::IdeMb200,
    HddSizeType::IdeMb500,
];

/// PC/AT disk types.
const AT_DISKS: [HddSizeType; 4] = [
    HddSizeType::AtMb40,
    HddSizeType::AtMb100,
    HddSizeType::AtMb250,
    HddSizeType::AtMb504,
];

/// Runs the committed script that creates, formats and hard-resets a disk.
#[test]
fn create_format_and_reset_in_memory_hdd() {
    let run = run_committed_script("hdd-create.scm", 60);
    assert_completed_ok(&run.termination);
}

/// A formatted disk image read through the `DiskIo` of the HLE DOS, addressed
/// in the CHS geometry its BIOS reports.
struct ImageDisk {
    image: HddImage,
    bios_geometry: HddGeometry,
}

impl DiskIo for ImageDisk {
    fn read_sectors(&mut self, device_id: u8, lba: u32, count: u32) -> Result<Vec<u8>, u8> {
        if device_id != FIRST_FIXED_DISK {
            return Err(SECTOR_ACCESS_ERROR);
        }
        let mut data = Vec::new();
        for offset in 0..count {
            data.extend_from_slice(
                self.image
                    .read_sector(lba + offset)
                    .ok_or(SECTOR_ACCESS_ERROR)?,
            );
        }
        Ok(data)
    }

    fn write_sectors(&mut self, device_id: u8, lba: u32, data: &[u8]) -> Result<(), u8> {
        if device_id != FIRST_FIXED_DISK {
            return Err(SECTOR_ACCESS_ERROR);
        }
        let sector_size = usize::from(self.image.geometry.sector_size);
        for (offset, sector) in data.chunks(sector_size).enumerate() {
            if !self.image.write_sector(lba + offset as u32, sector) {
                return Err(SECTOR_ACCESS_ERROR);
            }
        }
        Ok(())
    }

    fn sector_size(&self, device_id: u8) -> Option<u16> {
        (device_id == FIRST_FIXED_DISK).then_some(self.image.geometry.sector_size)
    }

    fn total_sectors(&self, device_id: u8) -> Option<u32> {
        (device_id == FIRST_FIXED_DISK).then_some(self.image.geometry.total_sectors())
    }

    fn drive_geometry(&self, device_id: u8) -> Option<(u16, u8, u8)> {
        (device_id == FIRST_FIXED_DISK).then_some((
            self.bios_geometry.cylinders,
            self.bios_geometry.heads,
            self.bios_geometry.sectors_per_track,
        ))
    }
}

/// Opens a session with a machine of `selection` that boots `boot_device`.
fn open_session(selection: MachineSelection, boot_device: BootDevice) -> AutomationSession {
    let (mut session, _receiver) = make_session(Path::new("."), Path::new("."));
    let mut config = EmulatorConfig::for_model(selection);
    config.boot_device = boot_device;
    session
        .open_machine(config, Vec::new())
        .expect("open machine");
    session
}

/// Returns the PC/AT model selection.
fn at_selection() -> MachineSelection {
    "AT486DX66".parse().expect("PC/AT model")
}

/// Runs the machine of `session` for `frames` frames.
fn run_frames(session: &mut AutomationSession, frames: u64) {
    session
        .run(RunTarget::Frames(frames), MAX_TICKS, DRAIN_INTERVAL_TICKS)
        .expect("frame run");
}

/// Runs the machine of `session` until the last line on the screen of the
/// HLE DOS is `line`.
fn wait_for_last_line(session: &mut AutomationSession, line: &str) {
    for _ in 0..POLL_LIMIT {
        run_frames(session, POLL_FRAMES);
        let lines = session.text_screen_lines("display.main").expect("screen");
        let last = lines
            .iter()
            .map(|text| text.trim())
            .rfind(|text| !text.is_empty());
        if last == Some(line) {
            return;
        }
    }
    panic!("the HLE DOS never showed {line:?}");
}

/// Types `text` into the HLE DOS and waits until the last line on the screen
/// is `line`.
fn type_command(session: &mut AutomationSession, text: &str, line: &str) {
    session
        .type_text(text, TYPING_SPACING_FRAMES, MAX_TICKS)
        .expect("type text");
    wait_for_last_line(session, line);
}

/// Parses the bytes of a mounted disk of `size`.
fn parse_image(size: HddSizeType, bytes: &[u8]) -> HddImage {
    let name = format!("disk.{}", size.geometry().1.file_extension());
    load_hdd_image(Path::new(&name), bytes).expect("parse disk image")
}

/// Asserts that `image` holds the layout the disk image tooling builds for
/// `table` in the BIOS geometry `geometry`: the partition table, the boot
/// record with its BPB, both FAT copies and the empty root directory.
fn assert_layout(image: &HddImage, table: PartitionTableType, geometry: HddGeometry, tag: &str) {
    let sector_size = geometry.sector_size;
    let sectors_per_track = geometry.sectors_per_track;
    let partition_start = match table {
        PartitionTableType::Pc98 => u32::from(geometry.heads) * u32::from(sectors_per_track),
        PartitionTableType::At => u32::from(sectors_per_track),
    };
    let partition_sectors = geometry.total_sectors() - partition_start;
    let sector = |lba: u32| image.read_sector(lba).expect("sector in range").to_vec();

    match table {
        PartitionTableType::Pc98 => {
            assert!(sector(0).iter().all(|&byte| byte == 0), "{tag}: empty IPL");
            assert_eq!(
                sector(1),
                pc98_nec_partition_table_sector(sector_size, geometry.cylinders - 1),
                "{tag}: partition table"
            );
            assert_eq!(
                &sector(1)[8..12],
                &[0, 0, 1, 0],
                "{tag}: start on cylinder 1"
            );
        }
        PartitionTableType::At => {
            assert_eq!(
                sector(0),
                at_master_boot_record(
                    sector_size,
                    partition_start,
                    partition_sectors,
                    geometry.heads,
                    sectors_per_track
                ),
                "{tag}: master boot record"
            );
        }
    }

    let bpb = hdd_bpb_params(sector_size, partition_sectors);
    let boot_record = sector(partition_start);
    assert_eq!(
        boot_record,
        hdd_boot_sector(
            &bpb,
            sector_size,
            partition_sectors,
            sectors_per_track,
            geometry.heads,
            partition_start
        ),
        "{tag}: boot record"
    );
    assert_eq!(
        u16::from_le_bytes([boot_record[24], boot_record[25]]),
        u16::from(sectors_per_track),
        "{tag}: BPB sectors per track"
    );
    assert_eq!(
        u16::from_le_bytes([boot_record[26], boot_record[27]]),
        u16::from(geometry.heads),
        "{tag}: BPB heads"
    );
    assert_eq!(
        u32::from_le_bytes(boot_record[28..32].try_into().unwrap()),
        partition_start,
        "{tag}: BPB hidden sectors"
    );

    let fat = fat_region(&bpb, sector_size);
    let fat_start = partition_start + u32::from(bpb.reserved_sectors);
    for copy in 0..u32::from(bpb.num_fats) {
        let copy_start = fat_start + copy * u32::from(bpb.sectors_per_fat);
        for (index, expected) in fat.chunks_exact(usize::from(sector_size)).enumerate() {
            assert_eq!(
                sector(copy_start + index as u32),
                expected,
                "{tag}: FAT copy {copy} sector {index}"
            );
        }
    }

    let root_start = fat_start + u32::from(bpb.num_fats) * u32::from(bpb.sectors_per_fat);
    let root_sectors = (u32::from(bpb.root_entry_count) * 32).div_ceil(u32::from(sector_size));
    for index in 0..root_sectors {
        assert!(
            sector(root_start + index).iter().all(|&byte| byte == 0),
            "{tag}: root directory sector {index}"
        );
    }
}

/// Mounts the volume of `disk` with the FAT code of the HLE DOS and returns
/// the volume start the partition scheme finds.
fn mount_volume(disk: &mut ImageDisk, scheme: PartitionScheme) -> (u32, FatFs<'_>) {
    let mut entry = pc98::drive_entry(FIRST_FIXED_DISK).expect("fixed disk entry");
    entry.partition_scheme = scheme;
    let volume_start = scheme
        .volume_start(FIRST_FIXED_DISK, disk)
        .expect("volume start");
    let volume = FatFs::mount(disk, &entry).expect("mount FAT volume");
    (volume_start, volume)
}

/// Formats a blank disk of `size` with `format-hdd!` on a PC-98 `model`,
/// checks the layout, creates a directory through the HLE DOS, and compares the
/// formatted disk with the disk the FORMAT command of the HLE DOS writes.
fn check_pc98_disk(model: MachineModel, size: HddSizeType, bios_geometry: HddGeometry) {
    let tag = format!("{model} {}", size.as_token());

    let mut session = open_session(MachineSelection::Pc98(model), BootDevice::Dos);
    session.create_hdd(0, size).expect("create disk");
    session.format_hdd(0, None).expect("format disk");
    let formatted = session
        .media_image_bytes(MediaKind::Hdd, 0)
        .expect("formatted disk");
    assert_layout(
        &parse_image(size, &formatted),
        PartitionTableType::Pc98,
        bios_geometry,
        &tag,
    );

    wait_for_last_line(&mut session, DRIVE_A_PROMPT);
    type_command(&mut session, "MD NEETAN\n", DRIVE_A_PROMPT);
    let written = session
        .media_image_bytes(MediaKind::Hdd, 0)
        .expect("written disk");
    drop(session);
    let mut disk = ImageDisk {
        image: parse_image(size, &written),
        bios_geometry,
    };
    let (volume_start, mut volume) = mount_volume(&mut disk, PartitionScheme::Pc98);
    assert_eq!(
        volume_start,
        u32::from(bios_geometry.heads) * u32::from(bios_geometry.sectors_per_track),
        "{tag}: volume on cylinder 1"
    );
    let directory = volume
        .stat(TEST_DIRECTORY)
        .expect("stat directory")
        .expect("directory created by the HLE DOS");
    assert!(
        directory.is_dir(),
        "{tag}: directory created by the HLE DOS"
    );
    drop(disk);

    let mut session = open_session(MachineSelection::Pc98(model), BootDevice::Dos);
    session.create_hdd(0, size).expect("create disk");
    wait_for_last_line(&mut session, DRIVE_A_PROMPT);
    type_command(&mut session, "FORMAT A: /Q\n", FORMAT_QUESTION);
    type_command(&mut session, "Y\n", DRIVE_A_PROMPT);
    let dos_formatted = session
        .media_image_bytes(MediaKind::Hdd, 0)
        .expect("disk formatted by DOS");
    assert!(
        formatted == dos_formatted,
        "{tag}: format-hdd! and the FORMAT command write different disks"
    );
}

/// Checks every PC-98 SASI disk type on the SASI and the IDE machines.
#[test]
fn format_hdd_writes_the_nec_layout_of_every_pc98_sasi_disk() {
    for (model, size) in PC98_SASI_DISKS {
        check_pc98_disk(model, size, size.geometry().0);
    }
}

/// Checks every PC-98 IDE disk type in the PC-98 IDE BIOS geometry.
#[test]
fn format_hdd_writes_the_nec_layout_of_every_pc98_ide_disk() {
    for size in PC98_IDE_DISKS {
        let bios_geometry = pc98_ide::bios_geometry(size.geometry().0);
        assert_eq!(bios_geometry.heads, 8, "{}", size.as_token());
        assert_eq!(bios_geometry.sectors_per_track, 17, "{}", size.as_token());
        check_pc98_disk(MachineModel::PC9821AP, size, bios_geometry);
    }
}

/// Checks every PC/AT disk type against the image tooling and the FAT code.
#[test]
fn format_hdd_writes_a_master_boot_record_on_every_at_disk() {
    for size in AT_DISKS {
        let tag = size.as_token();
        let geometry = size.geometry().0;
        let mut session = open_session(at_selection(), BootDevice::Auto);
        session.create_hdd(0, size).expect("create disk");
        session.format_hdd(0, None).expect("format disk");
        let formatted = session
            .media_image_bytes(MediaKind::Hdd, 0)
            .expect("formatted disk");
        drop(session);

        let mut tooling = blank_hdd_image(size);
        format_hdd_image(&mut tooling, PartitionTableType::At).expect("tooling format");
        assert!(
            formatted == tooling.to_bytes(),
            "{tag}: format-hdd! and the image tooling write different disks"
        );
        drop(tooling);

        let image = parse_image(size, &formatted);
        assert_layout(&image, PartitionTableType::At, geometry, tag);
        let mut disk = ImageDisk {
            image,
            bios_geometry: geometry,
        };
        let (volume_start, mut volume) = mount_volume(&mut disk, PartitionScheme::MasterBootRecord);
        assert_eq!(volume_start, u32::from(geometry.sectors_per_track), "{tag}");
        volume
            .write_file(TEST_FILE, TEST_CONTENTS, 0, 0, 0)
            .expect("write file");
        assert_eq!(
            volume.read_file(TEST_FILE).expect("read file"),
            TEST_CONTENTS,
            "{tag}"
        );
        let names: Vec<Vec<u8>> = volume
            .list_dir(b"")
            .expect("list root")
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        assert_eq!(names, [TEST_FILE.to_vec()], "{tag}");
    }
}

/// Checks that an explicit table must match the machine family.
#[test]
fn format_hdd_rejects_the_partition_table_of_another_machine_family() {
    let mut session = open_session(
        MachineSelection::Pc98(MachineModel::PC9801VM),
        BootDevice::Auto,
    );
    session
        .create_hdd(0, HddSizeType::Mb40)
        .expect("create disk");
    let blank = session
        .media_image_bytes(MediaKind::Hdd, 0)
        .expect("blank disk");
    assert!(matches!(
        session.format_hdd(0, Some(PartitionTableType::At)),
        Err(OpError::Argument(_))
    ));
    assert!(
        session
            .media_image_bytes(MediaKind::Hdd, 0)
            .expect("unchanged disk")
            == blank
    );
    session
        .format_hdd(0, Some(PartitionTableType::Pc98))
        .expect("format with the PC-98 table");
    drop(session);

    let mut session = open_session(at_selection(), BootDevice::Auto);
    session
        .create_hdd(0, HddSizeType::AtMb40)
        .expect("create disk");
    assert!(matches!(
        session.format_hdd(0, Some(PartitionTableType::Pc98)),
        Err(OpError::Argument(_))
    ));
    session
        .format_hdd(0, Some(PartitionTableType::At))
        .expect("format with the PC/AT table");
}

/// Checks that the FM Towns reports formatting as unsupported.
#[test]
fn format_hdd_is_unsupported_on_a_machine_family_of_another_partition_table() {
    let mut session = open_session("fmtowns".parse().expect("FM Towns model"), BootDevice::Auto);
    session
        .create_hdd(0, HddSizeType::ScsiMb20)
        .expect("create disk");
    assert!(matches!(
        session.format_hdd(0, None),
        Err(OpError::Unsupported(_))
    ));
}

/// Checks that a blank X68000 hard disk mounts as an `.hdf` image and that a
/// saved copy of it mounts again.
#[cfg(feature = "x68k")]
#[test]
fn x68k_blank_hdd_mounts_and_saves_as_hdf() {
    let folder = common::temporary::TemporaryFolder::new("auto_hdd_x68k_hdf");
    let (mut session, _receiver) = make_session(folder.path(), folder.path());
    session.install_machine(harness::build_x68k());
    let mount = session
        .create_hdd(0, HddSizeType::X68kSasiMb20)
        .expect("create disk");
    assert_eq!(mount.format, "HDF");

    session
        .media_save(MediaKind::Hdd, 0, "saved.hdf")
        .expect("save disk");
    let saved = std::fs::read(folder.path().join("saved.hdf")).expect("read saved disk");
    assert_eq!(
        saved,
        session
            .media_image_bytes(MediaKind::Hdd, 0)
            .expect("blank disk")
    );

    session
        .media_insert(MediaKind::Hdd, 1, "saved.hdf".to_owned())
        .expect("mount saved disk");
    assert_eq!(
        session
            .media_image_bytes(MediaKind::Hdd, 1)
            .expect("mounted copy"),
        saved
    );
}
