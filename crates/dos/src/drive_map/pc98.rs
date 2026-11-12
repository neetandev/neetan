//! PC-98 drives: DA/UA device ids, the BDA disk equipment word and the
//! DA/UA tables of the `IO.SYS` work area.
//!
//! The `DiskIo` device id of a PC-98 drive is its DA/UA byte. The high nibble
//! names the device class, the low nibble the unit. PC-98 programs read the
//! drive assignment from the DA/UA tables at `0060:006Ch` and `0060:2C86h`.
//! The tables mirror the drive map.

use super::{BpbSource, DriveDevice, DriveEntry, DriveGeometry, DriveMap, PartitionScheme};
use crate::{
    DiskIo, MemoryAccess,
    filesystem::{fat::FatVolume, fat_bpb::Bpb, fat_dir},
    tables,
};

/// Mask of the device class nibble of a DA/UA byte.
pub const DEVICE_CLASS_MASK: u8 = 0xF0;

/// Mask of the unit nibble of a DA/UA byte.
pub const UNIT_MASK: u8 = 0x0F;

/// Device class of the 1 MB (2HD) floppy drives.
pub const FLOPPY_2HD_CLASS: u8 = 0x90;

/// Device class of the 640 KB (2DD) floppy drives.
pub const FLOPPY_2DD_CLASS: u8 = 0x70;

/// Device class of the fixed disks (SASI and IDE).
pub const FIXED_DISK_CLASS: u8 = 0x80;

/// Units per device class in the BDA disk equipment word.
const UNITS_PER_CLASS: u8 = 4;

/// Bit of unit 0 of the 2HD floppy drives in the disk equipment word.
const EQUIPMENT_FLOPPY_2HD_SHIFT: u8 = 0;

/// Bit of unit 0 of the fixed disks in the disk equipment word.
const EQUIPMENT_FIXED_DISK_SHIFT: u8 = 8;

/// Bit of unit 0 of the 2DD floppy drives in the disk equipment word.
const EQUIPMENT_FLOPPY_2DD_SHIFT: u8 = 12;

/// Lowest drive index of the second device class when the first class has
/// fewer drives (A: and B: stay with the first class).
const SECOND_CLASS_MINIMUM_START: usize = 2;

/// Number of drives in the DA/UA table at `0060:006Ch` (A: to P:).
pub const DAUA_TABLE_LENGTH: usize = 16;

/// Number of bytes in the extended DA/UA table at `0060:2C86h`.
pub const EXTENDED_DAUA_TABLE_LENGTH: usize = tables::IOSYS_EXT_DAUA_TABLE_SIZE as usize;

/// Bytes per drive in the extended DA/UA table (attribute and DA/UA).
const EXTENDED_DAUA_ENTRY_SIZE: usize = 2;

/// BPB of a 1 MB 2HD floppy: 77 cylinders, 2 heads, 8 sectors of 1024 bytes.
pub const FLOPPY_2HD_BPB: Bpb = Bpb {
    bytes_per_sector: 1024,
    sectors_per_cluster: 1,
    reserved_sectors: 1,
    num_fats: 2,
    root_entry_count: 192,
    total_sectors_16: 1232,
    media_descriptor: 0xFE,
    sectors_per_fat: 2,
    total_sectors_32: 0,
};

/// BPB of a 640 KB 2DD floppy: 80 cylinders, 2 heads, 8 sectors of 512 bytes.
pub const FLOPPY_2DD_BPB: Bpb = Bpb {
    bytes_per_sector: 512,
    sectors_per_cluster: 2,
    reserved_sectors: 1,
    num_fats: 2,
    root_entry_count: 112,
    total_sectors_16: 1280,
    media_descriptor: 0xFE,
    sectors_per_fat: 3,
    total_sectors_32: 0,
};

/// BPB of a fixed disk partition before its volume mounts: a 4080 cluster
/// FAT12 volume with 2 KB clusters.
pub const FIXED_DISK_BPB: Bpb = Bpb {
    bytes_per_sector: 512,
    sectors_per_cluster: 4,
    reserved_sectors: 1,
    num_fats: 2,
    root_entry_count: 512,
    total_sectors_16: 16_365,
    media_descriptor: 0xF8,
    sectors_per_fat: 8,
    total_sectors_32: 0,
};

/// Returns the device class nibble of a DA/UA byte.
pub const fn device_class(device_id: u8) -> u8 {
    device_id & DEVICE_CLASS_MASK
}

/// Returns the geometry of a 2HD floppy drive. Old 2HD media carry no BPB in
/// the boot sector.
pub const fn floppy_2hd_geometry() -> DriveGeometry {
    DriveGeometry::new(BpbSource::BootRecordOrDefault, FLOPPY_2HD_BPB)
}

/// Returns the geometry of a 2DD floppy drive.
pub const fn floppy_2dd_geometry() -> DriveGeometry {
    DriveGeometry::new(BpbSource::BootRecord, FLOPPY_2DD_BPB)
}

/// Returns the geometry of a fixed disk partition.
pub const fn fixed_disk_geometry() -> DriveGeometry {
    DriveGeometry::new(BpbSource::BootRecord, FIXED_DISK_BPB)
}

/// Returns the drive entry of a DA/UA, or `None` for an unknown class.
pub fn drive_entry(device_id: u8) -> Option<DriveEntry> {
    let unit = device_id & UNIT_MASK;
    let (device, partition_scheme, geometry) = match device_class(device_id) {
        FLOPPY_2HD_CLASS => (
            DriveDevice::Floppy { device_id },
            PartitionScheme::Unpartitioned,
            floppy_2hd_geometry(),
        ),
        FLOPPY_2DD_CLASS => (
            DriveDevice::Floppy { device_id },
            PartitionScheme::Unpartitioned,
            floppy_2dd_geometry(),
        ),
        FIXED_DISK_CLASS => (
            DriveDevice::FixedDisk { device_id },
            PartitionScheme::Pc98,
            fixed_disk_geometry(),
        ),
        _ => return None,
    };
    Some(DriveEntry {
        device,
        unit,
        partition_scheme,
        geometry: Some(geometry),
    })
}

/// Returns the DA/UA bytes of the units whose equipment bits are set.
fn equipped_units(disk_equipment: u16, class: u8, shift: u8) -> Vec<u8> {
    (0..UNITS_PER_CLASS)
        .filter(|unit| disk_equipment & (1 << (shift + unit)) != 0)
        .map(|unit| class | unit)
        .collect()
}

/// Returns the floppy DA/UAs of the disk equipment word: 2HD before 2DD.
pub fn floppy_devices(disk_equipment: u16) -> Vec<u8> {
    let mut devices = equipped_units(disk_equipment, FLOPPY_2HD_CLASS, EQUIPMENT_FLOPPY_2HD_SHIFT);
    devices.extend(equipped_units(
        disk_equipment,
        FLOPPY_2DD_CLASS,
        EQUIPMENT_FLOPPY_2DD_SHIFT,
    ));
    devices
}

/// Returns the fixed disk DA/UAs of the disk equipment word.
pub fn fixed_disk_devices(disk_equipment: u16) -> Vec<u8> {
    equipped_units(disk_equipment, FIXED_DISK_CLASS, EQUIPMENT_FIXED_DISK_SHIFT)
}

/// Builds the drive map of the physical drives from the BDA disk equipment
/// word.
///
/// The floppy drives come first when a floppy holds `AUTOEXEC.BAT` in its
/// root. Otherwise the fixed disks come first when there are any. The second
/// class starts at C: or behind the drives of the first class.
pub fn discover_drive_map(memory: &dyn MemoryAccess, disk: &mut dyn DiskIo) -> DriveMap {
    let disk_equipment = memory.read_word(tables::BDA_DISK_EQUIP);
    let floppies = floppy_devices(disk_equipment);
    let fixed_disks = fixed_disk_devices(disk_equipment);
    let floppy_has_autoexec = floppies
        .iter()
        .any(|&device_id| drive_has_root_autoexec(device_id, disk));

    let (first_class, second_class) = if floppy_has_autoexec || fixed_disks.is_empty() {
        (floppies, fixed_disks)
    } else {
        (fixed_disks, floppies)
    };
    drive_map_from_classes(&first_class, &second_class)
}

/// Assigns drive letters to two device classes in order.
pub fn drive_map_from_classes(first_class: &[u8], second_class: &[u8]) -> DriveMap {
    let mut map = DriveMap::new();
    let second_start = first_class.len().max(SECOND_CLASS_MINIMUM_START);
    let letters = (0..first_class.len()).chain(second_start..);
    for (drive_index, &device_id) in letters.zip(first_class.iter().chain(second_class)) {
        if let Some(entry) = drive_entry(device_id) {
            map.set(drive_index as u8, entry);
        }
    }
    map
}

/// Returns true when the volume of a DA/UA holds `AUTOEXEC.BAT` in its root.
fn drive_has_root_autoexec(device_id: u8, disk: &mut dyn DiskIo) -> bool {
    let Some(entry) = drive_entry(device_id) else {
        return false;
    };
    let Some(geometry) = entry.geometry else {
        return false;
    };
    let Ok(partition_offset) = entry.partition_scheme.volume_start(device_id, disk) else {
        return false;
    };
    let Ok(volume) = FatVolume::mount(device_id, partition_offset, &geometry, disk) else {
        return false;
    };
    let autoexec_name = fat_dir::name_to_fcb(b"AUTOEXEC.BAT");
    match fat_dir::find_entry(&volume, 0, &autoexec_name, disk) {
        Ok(Some(entry)) => entry.attribute & fat_dir::ATTR_DIRECTORY == 0,
        _ => false,
    }
}

/// Returns the DA/UA of a drive for the `IO.SYS` tables. Drives outside
/// `DiskIo` have DA/UA 0.
fn table_device_id(map: &DriveMap, drive_index: u8) -> u8 {
    map.entry(drive_index)
        .and_then(|entry| entry.device.disk_device_id())
        .unwrap_or(0)
}

/// Returns the DA/UA table of the drives A: to P: (`0060:006Ch`).
pub fn daua_table(map: &DriveMap) -> [u8; DAUA_TABLE_LENGTH] {
    std::array::from_fn(|drive_index| table_device_id(map, drive_index as u8))
}

/// Returns the extended DA/UA table (`0060:2C86h`): attribute 0 and the DA/UA
/// for every drive letter.
pub fn extended_daua_table(map: &DriveMap) -> [u8; EXTENDED_DAUA_TABLE_LENGTH] {
    let mut table = [0u8; EXTENDED_DAUA_TABLE_LENGTH];
    for (drive_index, pair) in table
        .as_chunks_mut::<EXTENDED_DAUA_ENTRY_SIZE>()
        .0
        .iter_mut()
        .enumerate()
    {
        pair[1] = table_device_id(map, drive_index as u8);
    }
    table
}

/// Mirrors the drive map into the DA/UA tables of the `IO.SYS` work area.
pub fn write_iosys_drive_tables(memory: &mut dyn MemoryAccess, map: &DriveMap) {
    memory.write_block(
        tables::IOSYS_BASE + tables::IOSYS_OFF_DAUA_TABLE,
        &daua_table(map),
    );
    memory.write_block(
        tables::IOSYS_BASE + tables::IOSYS_OFF_EXT_DAUA_TABLE,
        &extended_daua_table(map),
    );
}

/// Writes the DA/UA of the boot drive to the BIOS data area.
pub fn write_boot_device(memory: &mut dyn MemoryAccess, device_id: u8) {
    memory.write_byte(tables::BDA_BOOT_DEVICE, device_id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{drive_map::VIRTUAL_DRIVE_INDEX, test_support::MockMemory};

    /// Disk equipment bit of 2HD unit 0.
    const FLOPPY_2HD_UNIT_0: u16 = 1 << EQUIPMENT_FLOPPY_2HD_SHIFT;
    /// Disk equipment bit of 2HD unit 1.
    const FLOPPY_2HD_UNIT_1: u16 = 1 << (EQUIPMENT_FLOPPY_2HD_SHIFT + 1);
    /// Disk equipment bit of fixed disk unit 0.
    const FIXED_DISK_UNIT_0: u16 = 1 << EQUIPMENT_FIXED_DISK_SHIFT;
    /// Disk equipment bit of 2DD unit 0.
    const FLOPPY_2DD_UNIT_0: u16 = 1 << EQUIPMENT_FLOPPY_2DD_SHIFT;

    /// Disk without media: every access fails.
    struct EmptyDisk;

    impl DiskIo for EmptyDisk {
        fn read_sectors(&mut self, _device_id: u8, _lba: u32, _count: u32) -> Result<Vec<u8>, u8> {
            Err(0x60)
        }

        fn write_sectors(&mut self, _device_id: u8, _lba: u32, _data: &[u8]) -> Result<(), u8> {
            Err(0x60)
        }

        fn sector_size(&self, _device_id: u8) -> Option<u16> {
            None
        }

        fn total_sectors(&self, _device_id: u8) -> Option<u32> {
            None
        }

        fn drive_geometry(&self, _device_id: u8) -> Option<(u16, u8, u8)> {
            None
        }
    }

    /// Returns the drive letters and DA/UAs of a map.
    fn assignment(map: &DriveMap) -> Vec<(u8, Option<u8>)> {
        map.drives()
            .map(|(drive, entry)| (drive, entry.device.disk_device_id()))
            .collect()
    }

    /// Builds the map of a disk equipment word with empty drives.
    fn discover(disk_equipment: u16) -> DriveMap {
        let mut memory = MockMemory::with_extended_memory(0x0010_0000, 0);
        memory.write_word(tables::BDA_DISK_EQUIP, disk_equipment);
        discover_drive_map(&memory, &mut EmptyDisk)
    }

    #[test]
    fn scsi_and_unknown_device_classes_get_no_drive() {
        for device_id in [0xA0, 0xA1, 0x00, 0x30] {
            assert!(drive_entry(device_id).is_none(), "DA/UA {device_id:02X}h");
        }
    }

    #[test]
    fn equipment_word_lists_2hd_before_2dd() {
        let equipment = FLOPPY_2HD_UNIT_0 | FLOPPY_2HD_UNIT_1 | FLOPPY_2DD_UNIT_0;
        assert_eq!(floppy_devices(equipment), vec![0x90, 0x91, 0x70]);
        assert_eq!(
            fixed_disk_devices(FIXED_DISK_UNIT_0 | 0x0200),
            vec![0x80, 0x81]
        );
    }

    #[test]
    fn floppies_only_start_at_a() {
        let map = discover(FLOPPY_2HD_UNIT_0 | FLOPPY_2HD_UNIT_1);
        assert_eq!(assignment(&map), vec![(0, Some(0x90)), (1, Some(0x91))]);
    }

    #[test]
    fn fixed_disk_comes_first_without_a_floppy_autoexec() {
        let map = discover(FLOPPY_2HD_UNIT_0 | FLOPPY_2HD_UNIT_1 | FIXED_DISK_UNIT_0);
        assert_eq!(
            assignment(&map),
            vec![(0, Some(0x80)), (2, Some(0x90)), (3, Some(0x91))]
        );
    }

    #[test]
    fn second_class_follows_a_long_first_class() {
        let map = drive_map_from_classes(&[0x90, 0x91, 0x92], &[0x80]);
        assert_eq!(
            assignment(&map),
            vec![
                (0, Some(0x90)),
                (1, Some(0x91)),
                (2, Some(0x92)),
                (3, Some(0x80))
            ]
        );
    }

    #[test]
    fn entries_carry_class_unit_partition_scheme_and_geometry() {
        let floppy = drive_entry(0x91).unwrap();
        assert_eq!(floppy.device, DriveDevice::Floppy { device_id: 0x91 });
        assert_eq!(floppy.unit, 1);
        assert_eq!(floppy.partition_scheme, PartitionScheme::Unpartitioned);
        assert_eq!(floppy.geometry, Some(floppy_2hd_geometry()));

        let floppy_2dd = drive_entry(0x70).unwrap();
        assert_eq!(floppy_2dd.geometry.unwrap().bpb, FLOPPY_2DD_BPB);
        assert_eq!(
            floppy_2dd.geometry.unwrap().bpb_source,
            BpbSource::BootRecord
        );

        let fixed = drive_entry(0x81).unwrap();
        assert_eq!(fixed.device, DriveDevice::FixedDisk { device_id: 0x81 });
        assert_eq!(fixed.unit, 1);
        assert_eq!(fixed.partition_scheme, PartitionScheme::Pc98);

        assert!(drive_entry(0x00).is_none());
        assert!(drive_entry(0xA0).is_none());
    }

    #[test]
    fn default_bpbs_describe_the_standard_media() {
        assert_eq!(FLOPPY_2HD_BPB.total_sectors(), 77 * 2 * 8);
        assert_eq!(FLOPPY_2DD_BPB.total_sectors(), 80 * 2 * 8);
        let fixed = crate::layout::DpbGeometry::from_bpb(&FIXED_DISK_BPB);
        assert_eq!(fixed.first_root_sector, 17);
        assert_eq!(fixed.first_data_sector, 49);
        assert_eq!(fixed.max_cluster, 4080);
    }

    #[test]
    fn iosys_tables_mirror_the_drive_map() {
        let mut map = drive_map_from_classes(&[0x80], &[0x90, 0x91]);
        map.set(16, DriveEntry::cdrom());
        map.set(VIRTUAL_DRIVE_INDEX, DriveEntry::virtual_drive());

        let mut memory = MockMemory::with_extended_memory(0x0010_0000, 0);
        let daua_address = tables::IOSYS_BASE + tables::IOSYS_OFF_DAUA_TABLE;
        let extended_address = tables::IOSYS_BASE + tables::IOSYS_OFF_EXT_DAUA_TABLE;
        memory.write_block(daua_address, &[0xEE; DAUA_TABLE_LENGTH]);
        memory.write_block(extended_address, &[0xEE; EXTENDED_DAUA_TABLE_LENGTH]);
        write_iosys_drive_tables(&mut memory, &map);

        let mut daua = [0u8; DAUA_TABLE_LENGTH];
        memory.read_block(daua_address, &mut daua);
        let mut expected = [0u8; DAUA_TABLE_LENGTH];
        expected[0] = 0x80;
        expected[2] = 0x90;
        expected[3] = 0x91;
        assert_eq!(daua, expected);

        let mut extended = [0u8; EXTENDED_DAUA_TABLE_LENGTH];
        memory.read_block(extended_address, &mut extended);
        let mut expected_extended = [0u8; EXTENDED_DAUA_TABLE_LENGTH];
        expected_extended[1] = 0x80;
        expected_extended[5] = 0x90;
        expected_extended[7] = 0x91;
        assert_eq!(extended, expected_extended);
    }

    #[test]
    fn boot_device_goes_to_the_bios_data_area() {
        let mut memory = MockMemory::with_extended_memory(0x0010_0000, 0);
        write_boot_device(&mut memory, 0x80);
        assert_eq!(memory.read_byte(tables::BDA_BOOT_DEVICE), 0x80);
    }
}
