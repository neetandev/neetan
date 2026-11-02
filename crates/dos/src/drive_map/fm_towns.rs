//! FM Towns drives: the block devices of the `IO.SYS` block driver.
//!
//! The `DiskIo` device id of an FM Towns drive is the device code of the disk
//! BIOS: `20h`-`23h` for the floppy drives, `42h` for the OS ROM and `B0h`-`B6h`
//! for the SCSI hard disks. The `IO.SYS` block driver serves the floppy drives,
//! the OS ROM drive and the SCSI partitions. Other block devices of the device
//! chain (`MENU_DRV`, the IC memory card, `CONFIG.SYS` drivers such as
//! `TICM.SYS`) are guest driver units.

use super::{BpbSource, DriveDevice, DriveEntry, DriveGeometry, GuestBlockUnit, PartitionScheme};
use crate::{
    DiskIo, MemoryAccess,
    filesystem::{fat_bpb::Bpb, towns_partition},
};

/// Device code of floppy drive 0. Drive n has code `FLOPPY_DEVICE_BASE + n`.
pub const FLOPPY_DEVICE_BASE: u8 = 0x20;

/// Number of floppy drives of the disk BIOS.
pub const FLOPPY_DRIVE_COUNT: u8 = 4;

/// Device code of the write protected OS ROM window `C2000000h`-`C207FFFFh`.
pub const OS_ROM_DEVICE_ID: u8 = 0x42;

/// Device code of SCSI ID 0. SCSI ID n has code `SCSI_DEVICE_BASE + n`.
pub const SCSI_DEVICE_BASE: u8 = 0xB0;

/// Number of SCSI IDs the disk BIOS serves.
pub const SCSI_TARGET_COUNT: u8 = 7;

/// Bytes per physical sector of the OS ROM drive.
pub const OS_ROM_SECTOR_SIZE: u16 = 512;

/// BPB of a 2HD floppy: 77 cylinders, 2 heads, 8 sectors of 1024 bytes,
/// media `01h`.
pub const FLOPPY_BPB: Bpb = Bpb {
    bytes_per_sector: 0x0400,
    sectors_per_cluster: 1,
    reserved_sectors: 1,
    num_fats: 2,
    root_entry_count: 0x00C0,
    total_sectors_16: 0x04D0,
    media_descriptor: 0x01,
    sectors_per_fat: 2,
    total_sectors_32: 0,
};

/// BPB of a 2HD floppy with 15 sectors of 512 bytes per track, media `02h`.
pub const FLOPPY_2HD_512_BPB: Bpb = Bpb {
    bytes_per_sector: 0x0200,
    sectors_per_cluster: 1,
    reserved_sectors: 1,
    num_fats: 2,
    root_entry_count: 0x00E0,
    total_sectors_16: 0x0960,
    media_descriptor: 0x02,
    sectors_per_fat: 7,
    total_sectors_32: 0,
};

/// BPB of a 1.44 MB floppy: 80 cylinders, 2 heads, 18 sectors of 512 bytes,
/// media `F0h`.
pub const FLOPPY_1440K_BPB: Bpb = Bpb {
    bytes_per_sector: 0x0200,
    sectors_per_cluster: 1,
    reserved_sectors: 1,
    num_fats: 2,
    root_entry_count: 0x00E0,
    total_sectors_16: 0x0B40,
    media_descriptor: 0xF0,
    sectors_per_fat: 9,
    total_sectors_32: 0,
};

/// FAT ID of the first 2DD format. The 2DD formats use FAT IDs `F8h`-`FFh`.
const FLOPPY_2DD_FIRST_FAT_ID: u8 = 0xF8;

/// Returns the BPB of a 2DD floppy with 512-byte sectors. The FAT ID selects
/// the format and is the media byte of the BPB.
const fn floppy_2dd_bpb(
    fat_id: u8,
    sectors_per_cluster: u8,
    root_entry_count: u16,
    total_sectors: u16,
    sectors_per_fat: u16,
) -> Bpb {
    Bpb {
        bytes_per_sector: 0x0200,
        sectors_per_cluster,
        reserved_sectors: 1,
        num_fats: 2,
        root_entry_count,
        total_sectors_16: total_sectors,
        media_descriptor: fat_id,
        sectors_per_fat,
        total_sectors_32: 0,
    }
}

/// BPBs of the 2DD floppy formats, indexed by the FAT ID minus `F8h`.
const FLOPPY_2DD_BPBS: [Bpb; 8] = [
    floppy_2dd_bpb(0xF8, 2, 0x70, 0x02D0, 2),
    floppy_2dd_bpb(0xF9, 2, 0x70, 0x05A0, 3),
    floppy_2dd_bpb(0xFA, 2, 0x70, 0x0280, 1),
    floppy_2dd_bpb(0xFB, 2, 0x70, 0x0500, 2),
    floppy_2dd_bpb(0xFC, 1, 0x40, 0x0168, 2),
    floppy_2dd_bpb(0xFD, 2, 0x70, 0x02D0, 2),
    floppy_2dd_bpb(0xFE, 1, 0x40, 0x0140, 1),
    floppy_2dd_bpb(0xFF, 2, 0x70, 0x0280, 1),
];

/// BPB of an 8-inch floppy with 128-byte sectors in the 2HD mode: 2002
/// sectors, media `00h`.
pub const FLOPPY_128_BPB: Bpb = Bpb {
    bytes_per_sector: 0x0080,
    sectors_per_cluster: 4,
    reserved_sectors: 1,
    num_fats: 2,
    root_entry_count: 0x0044,
    total_sectors_16: 0x07D2,
    media_descriptor: 0x00,
    sectors_per_fat: 6,
    total_sectors_32: 0,
};

/// BPB of FAT ID `FEh` in the 1.44 MB mode: 2464 sectors of 1024 bytes,
/// media `03h`.
pub const FLOPPY_1440K_MODE_1024_BPB: Bpb = Bpb {
    bytes_per_sector: 0x0400,
    sectors_per_cluster: 1,
    reserved_sectors: 1,
    num_fats: 2,
    root_entry_count: 0x00C0,
    total_sectors_16: 0x09A0,
    media_descriptor: 0x03,
    sectors_per_fat: 4,
    total_sectors_32: 0,
};

/// FAT IDs of a 2HD floppy.
const FLOPPY_2HD_FAT_IDS: [u8; 2] = [0xFE, 0xF9];

/// FAT ID of a 1.44 MB floppy.
const FLOPPY_1440K_FAT_ID: u8 = 0xF0;

/// FAT ID of a floppy with 1024-byte sectors in the 1.44 MB mode.
const FLOPPY_1440K_MODE_1024_FAT_ID: u8 = 0xFE;

/// Physical sector size of a 2HD floppy in the FM Towns format.
const FLOPPY_LARGE_SECTOR_SIZE: u16 = 1024;

/// Physical sector size of the 2DD, the 1.44 MB and the 15 sector 2HD floppy.
const FLOPPY_SMALL_SECTOR_SIZE: u16 = 512;

/// Physical sector size of an 8-inch floppy.
const FLOPPY_TINY_SECTOR_SIZE: u16 = 128;

/// Highest sectors per track of a 2DD floppy.
const FLOPPY_2DD_MAX_SECTORS_PER_TRACK: u8 = 9;

/// Sectors per track of a 1.44 MB floppy.
const FLOPPY_1440K_SECTORS_PER_TRACK: u8 = 18;

/// Drive mode of the disk BIOS for a floppy medium.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FloppyDriveMode {
    /// 2HD mode with the physical sector size of the medium.
    TwoHd(u16),
    /// 2DD or 2D mode.
    TwoDd,
    /// 1.44 MB mode.
    HighDensity,
}

impl FloppyDriveMode {
    /// Returns the drive mode that the disk BIOS reports for a medium of the
    /// given physical format.
    fn of_format(sector_size: u16, sectors_per_track: u8) -> Self {
        match (sector_size, sectors_per_track) {
            (FLOPPY_SMALL_SECTOR_SIZE, FLOPPY_1440K_SECTORS_PER_TRACK) => Self::HighDensity,
            (FLOPPY_SMALL_SECTOR_SIZE, sectors) if sectors <= FLOPPY_2DD_MAX_SECTORS_PER_TRACK => {
                Self::TwoDd
            }
            (sector_size, _) => Self::TwoHd(sector_size),
        }
    }
}

/// Returns the BPB that the `IO.SYS` block driver builds for a floppy, or
/// `None` where the driver fails BUILD BPB with error `07h` (unknown media).
///
/// The driver takes the FAT ID, the first byte of sector 1, and the drive
/// mode of `INT 93h AH=01h`. It never reads the boot record. NeetanDos
/// derives the drive mode from the physical format of the medium.
///
/// * 1.44 MB mode: FAT ID `F0h` gets `FLOPPY_1440K_BPB`, FAT ID `FEh` gets
///   `FLOPPY_1440K_MODE_1024_BPB`.
/// * 2DD mode: FAT IDs `F8h`-`FFh` get the BPB of the FAT ID.
/// * 2HD mode with FAT ID `FEh` or `F9h`: 1024-byte sectors get `FLOPPY_BPB`,
///   512-byte sectors get `FLOPPY_2HD_512_BPB`, 128-byte sectors get
///   `FLOPPY_128_BPB`. 256-byte sectors are unknown media.
pub fn floppy_media_bpb(fat_id: u8, sector_size: u16, sectors_per_track: u8) -> Option<Bpb> {
    match FloppyDriveMode::of_format(sector_size, sectors_per_track) {
        FloppyDriveMode::HighDensity => match fat_id {
            FLOPPY_1440K_FAT_ID => Some(FLOPPY_1440K_BPB),
            FLOPPY_1440K_MODE_1024_FAT_ID => Some(FLOPPY_1440K_MODE_1024_BPB),
            _ => None,
        },
        FloppyDriveMode::TwoDd => fat_id
            .checked_sub(FLOPPY_2DD_FIRST_FAT_ID)
            .map(|index| FLOPPY_2DD_BPBS[usize::from(index)]),
        FloppyDriveMode::TwoHd(_) if !FLOPPY_2HD_FAT_IDS.contains(&fat_id) => None,
        FloppyDriveMode::TwoHd(FLOPPY_LARGE_SECTOR_SIZE) => Some(FLOPPY_BPB),
        FloppyDriveMode::TwoHd(FLOPPY_SMALL_SECTOR_SIZE) => Some(FLOPPY_2HD_512_BPB),
        FloppyDriveMode::TwoHd(FLOPPY_TINY_SECTOR_SIZE) => Some(FLOPPY_128_BPB),
        FloppyDriveMode::TwoHd(_) => None,
    }
}

/// BPB of the OS ROM drive: 1280 sectors of 512 bytes, media `FBh`. The boot
/// record of the OS ROM holds no BPB.
pub const OS_ROM_BPB: Bpb = Bpb {
    bytes_per_sector: OS_ROM_SECTOR_SIZE,
    sectors_per_cluster: 2,
    reserved_sectors: 1,
    num_fats: 2,
    root_entry_count: 0x0070,
    total_sectors_16: 0x0500,
    media_descriptor: 0xFB,
    sectors_per_fat: 2,
    total_sectors_32: 0,
};

/// Physical address of the CMOS. CMOS byte `n` is at `CMOS_BASE + n`.
const CMOS_BASE: u32 = 0xC214_0000;

/// Number of block descriptors in the CMOS header.
const CMOS_BLOCK_COUNT: u32 = 16;

/// Size of one block descriptor: flags, id, four name bytes, address word
/// and size word.
const CMOS_BLOCK_DESCRIPTOR_SIZE: u32 = 10;

/// Flag bit of a present block.
const CMOS_BLOCK_PRESENT: u8 = 0x01;

/// Offset of the block id in a descriptor.
const CMOS_BLOCK_ID_OFFSET: u32 = 1;

/// Offset of the block address word in a descriptor.
const CMOS_BLOCK_ADDRESS_OFFSET: u32 = 6;

/// Id of the setup block `SETU`.
const CMOS_SETUP_BLOCK_ID: u8 = 0xFE;

/// Offset of the drive assignment table in the setup block.
const DRIVE_ASSIGNMENT_OFFSET: u32 = 0x06;

/// Number of drive letters of the drive assignment table.
pub const DRIVE_ASSIGNMENT_SLOTS: usize = 16;

/// Device type of a SCSI hard disk partition in the drive assignment table.
const ASSIGNED_SCSI: u8 = 0x02;

/// The SCSI partition that the CMOS drive assignment table names for one
/// drive letter: the high nibble of the unit byte is the SCSI ID, the low
/// nibble counts the MS-DOS partitions of the partition table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AssignedPartition {
    /// SCSI ID of the disk.
    pub target: u8,
    /// Index among the MS-DOS partitions of the table.
    pub partition_index: u8,
}

/// The SCSI partitions of the CMOS drive assignment table by drive letter.
/// The `IO.SYS` block driver serves its unit `n` from slot `n`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DriveAssignment {
    /// Partition per drive letter, `None` for any other device.
    pub slots: [Option<AssignedPartition>; DRIVE_ASSIGNMENT_SLOTS],
}

impl DriveAssignment {
    /// Reads the table from the setup block of the CMOS. A CMOS without a
    /// setup block assigns no partition.
    pub fn read_cmos(memory: &dyn MemoryAccess) -> Self {
        let mut assignment = Self::default();
        let Some(setup_block) = (0..CMOS_BLOCK_COUNT).find_map(|block| {
            let descriptor = CMOS_BASE + block * CMOS_BLOCK_DESCRIPTOR_SIZE;
            (memory.read_byte(descriptor) & CMOS_BLOCK_PRESENT != 0
                && memory.read_byte(descriptor + CMOS_BLOCK_ID_OFFSET) == CMOS_SETUP_BLOCK_ID)
                .then(|| memory.read_word(descriptor + CMOS_BLOCK_ADDRESS_OFFSET))
        }) else {
            return assignment;
        };
        let table = CMOS_BASE + u32::from(setup_block) + DRIVE_ASSIGNMENT_OFFSET;
        for (slot, partition) in assignment.slots.iter_mut().enumerate() {
            let entry = table + slot as u32 * 2;
            if memory.read_byte(entry) == ASSIGNED_SCSI {
                let unit = memory.read_byte(entry + 1);
                *partition = Some(AssignedPartition {
                    target: unit >> 4,
                    partition_index: unit & 0x0F,
                });
            }
        }
        assignment
    }

    /// Returns the partition of the drive letter `slot`.
    pub fn partition(&self, slot: usize) -> Option<AssignedPartition> {
        self.slots.get(slot).copied().flatten()
    }
}

/// Returns the device code of a floppy drive.
pub const fn floppy_device_id(drive: u8) -> u8 {
    FLOPPY_DEVICE_BASE + drive
}

/// Returns the device code of a SCSI target.
pub const fn scsi_device_id(target: u8) -> u8 {
    SCSI_DEVICE_BASE + target
}

/// Returns the drive entry of floppy drive `drive`, unit `unit` of the
/// block driver. The BPB of a medium follows `floppy_media_bpb`.
pub const fn floppy_entry(drive: u8, unit: u8) -> DriveEntry {
    DriveEntry {
        device: DriveDevice::Floppy {
            device_id: floppy_device_id(drive),
        },
        unit,
        partition_scheme: PartitionScheme::Unpartitioned,
        geometry: Some(DriveGeometry::new(BpbSource::FmTownsFloppy, FLOPPY_BPB)),
    }
}

/// Returns the drive entry of the OS ROM drive, unit `unit` of the block
/// driver.
pub const fn os_rom_entry(unit: u8) -> DriveEntry {
    DriveEntry {
        device: DriveDevice::MemoryImage {
            device_id: OS_ROM_DEVICE_ID,
        },
        unit,
        partition_scheme: PartitionScheme::Unpartitioned,
        geometry: Some(DriveGeometry::new(BpbSource::Default, OS_ROM_BPB)),
    }
}

/// Returns the drive entry of a SCSI partition.
///
/// `partition_index` counts the MS-DOS partitions of the partition table.
/// `bpb` is the BPB that the `IO.SYS` block driver reports for the unit.
pub const fn scsi_partition_entry(
    target: u8,
    partition_index: u8,
    unit: u8,
    bpb: Bpb,
) -> DriveEntry {
    DriveEntry {
        device: DriveDevice::FixedDisk {
            device_id: scsi_device_id(target),
        },
        unit,
        partition_scheme: PartitionScheme::FmTowns { partition_index },
        geometry: Some(DriveGeometry::new(BpbSource::Default, bpb)),
    }
}

/// Returns the drive entry of a block unit of a guest driver. The driver
/// reports the BPB. `initial_bpb` is the BPB of its INIT answer.
pub const fn guest_unit_entry(unit: GuestBlockUnit, initial_bpb: Bpb) -> DriveEntry {
    DriveEntry {
        device: DriveDevice::GuestDriver(unit),
        unit: unit.unit,
        partition_scheme: PartitionScheme::Unpartitioned,
        geometry: Some(DriveGeometry::new(BpbSource::Driver, initial_bpb)),
    }
}

/// Returns true for the BPB that the `IO.SYS` block driver reports for a
/// floppy drive. The media byte differs between the versions.
fn is_floppy_bpb(bpb: &Bpb) -> bool {
    bpb.bytes_per_sector == FLOPPY_BPB.bytes_per_sector
        && bpb.total_sectors_16 == FLOPPY_BPB.total_sectors_16
}

/// Returns the native drive entries of the units of the `IO.SYS` block
/// driver, `None` for a unit that NeetanDos serves through guest calls.
///
/// The driver reports its units in a fixed order: the floppy drives, the OS
/// ROM drive and the SCSI partitions of the drive assignment (`HDD` boot
/// only). The BPB of a unit names its class. A unit with the floppy BPB
/// is the next floppy drive. A unit with the OS ROM BPB is the OS ROM drive.
/// Any other unit `n` is the partition of slot `n` of the drive assignment,
/// like the `IO.SYS` block driver: the n-th MS-DOS partition of the
/// partition table of its SCSI ID. The table comes from the CMOS in
/// `memory`. A unit without an assigned partition goes through guest calls.
pub fn io_sys_unit_entries(
    unit_bpbs: &[Bpb],
    memory: &dyn MemoryAccess,
    disk: &mut dyn DiskIo,
) -> Vec<Option<DriveEntry>> {
    let mut next_floppy = 0u8;
    let mut assignment: Option<DriveAssignment> = None;
    unit_bpbs
        .iter()
        .enumerate()
        .map(|(slot, bpb)| {
            let unit = slot as u8;
            if is_floppy_bpb(bpb) && next_floppy < FLOPPY_DRIVE_COUNT {
                next_floppy += 1;
                return Some(floppy_entry(next_floppy - 1, unit));
            }
            if *bpb == OS_ROM_BPB {
                return Some(os_rom_entry(unit));
            }
            let partition = assignment
                .get_or_insert_with(|| DriveAssignment::read_cmos(memory))
                .partition(slot)?;
            let table =
                towns_partition::read_partition_table(scsi_device_id(partition.target), disk)
                    .ok()?;
            table.ms_dos_partition(partition.partition_index)?;
            Some(scsi_partition_entry(
                partition.target,
                partition.partition_index,
                unit,
                *bpb,
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::DpbGeometry;

    /// Sectors of the MS-DOS partition of the test disk (Towns OS V1.1 L20).
    const PARTITION_SECTORS: u32 = 0x7803;

    /// Sectors of the larger partition of the test disk.
    const LARGE_PARTITION_SECTORS: u32 = 0x0002_0000;

    /// Bytes per physical sector of the test disk.
    const PHYSICAL_SECTOR_SIZE: u16 = 512;

    /// BPB that `IO.SYS` reports for the partition of `PARTITION_SECTORS`.
    const PARTITION_BPB: Bpb = Bpb {
        bytes_per_sector: 0x0800,
        sectors_per_cluster: 4,
        reserved_sectors: 1,
        num_fats: 2,
        root_entry_count: 0x0200,
        total_sectors_16: 0x1E00,
        media_descriptor: 0xF9,
        sectors_per_fat: 3,
        total_sectors_32: 0,
    };

    /// BPB that `IO.SYS` reports for the partition of
    /// `LARGE_PARTITION_SECTORS`.
    const LARGE_PARTITION_BPB: Bpb = Bpb {
        bytes_per_sector: 0x0800,
        sectors_per_cluster: 2,
        reserved_sectors: 1,
        num_fats: 2,
        root_entry_count: 0x0400,
        total_sectors_16: 0x8000,
        media_descriptor: 0xFA,
        sectors_per_fat: 32,
        total_sectors_32: 0,
    };

    /// CMOS address of the setup block of the default CMOS.
    const SETUP_BLOCK: u16 = 0x00E8;

    /// SCSI disk at ID 0 with an FM Towns partition table. Its entries are
    /// `(system type, length)` pairs in table order. No other device answers.
    struct ScsiDisk {
        partitions: Vec<(u8, u32)>,
    }

    impl ScsiDisk {
        /// Returns the disk of Towns OS V1.1 L20: one MS-DOS partition.
        fn l20() -> Self {
            Self {
                partitions: vec![(0x01, PARTITION_SECTORS)],
            }
        }
    }

    impl DiskIo for ScsiDisk {
        fn read_sectors(&mut self, device_id: u8, lba: u32, count: u32) -> Result<Vec<u8>, u8> {
            if device_id != scsi_device_id(0)
                || lba != towns_partition::PARTITION_TABLE_SECTOR
                || count != 1
            {
                return Err(0x80);
            }
            let mut sector = vec![0u8; usize::from(PHYSICAL_SECTOR_SIZE)];
            sector[..6].copy_from_slice(&towns_partition::FUJITSU_SIGNATURE);
            sector[0x0E..0x10].copy_from_slice(&PHYSICAL_SECTOR_SIZE.to_le_bytes());
            let mut start = 3u32;
            for (index, (system_type, length)) in self.partitions.iter().enumerate() {
                let entry = &mut sector[0x20 + index * 0x30..0x50 + index * 0x30];
                entry[1] = *system_type;
                entry[2..6].copy_from_slice(&start.to_le_bytes());
                entry[6..10].copy_from_slice(&length.to_le_bytes());
                start += length;
            }
            Ok(sector)
        }

        fn write_sectors(&mut self, _device_id: u8, _lba: u32, _data: &[u8]) -> Result<(), u8> {
            Err(0x80)
        }

        fn sector_size(&self, device_id: u8) -> Option<u16> {
            (device_id == scsi_device_id(0)).then_some(PHYSICAL_SECTOR_SIZE)
        }

        fn total_sectors(&self, device_id: u8) -> Option<u32> {
            (device_id == scsi_device_id(0)).then_some(0x0010_0000)
        }

        fn drive_geometry(&self, _device_id: u8) -> Option<(u16, u8, u8)> {
            None
        }
    }

    /// CMOS of the FM Towns at `C2140000h`: the setup block descriptor and
    /// the drive assignment table. Every other address reads `00h`.
    struct CmosMemory {
        cmos: Vec<u8>,
    }

    impl CmosMemory {
        /// Returns a CMOS whose drive assignment table holds `slots`: one
        /// `(type, unit)` pair per drive letter from A: on.
        fn with_assignment(slots: &[(u8, u8)]) -> Self {
            let mut cmos = vec![0u8; 0x800];
            let descriptor = CMOS_BLOCK_DESCRIPTOR_SIZE as usize;
            cmos[descriptor] = CMOS_BLOCK_PRESENT;
            cmos[descriptor + 1] = CMOS_SETUP_BLOCK_ID;
            cmos[descriptor + 2..descriptor + 6].copy_from_slice(b"SETU");
            cmos[descriptor + 6..descriptor + 8].copy_from_slice(&SETUP_BLOCK.to_le_bytes());
            let table = usize::from(SETUP_BLOCK) + DRIVE_ASSIGNMENT_OFFSET as usize;
            cmos[table..table + DRIVE_ASSIGNMENT_SLOTS * 2].fill(0xFF);
            for (slot, (device_type, unit)) in slots.iter().enumerate() {
                cmos[table + slot * 2] = *device_type;
                cmos[table + slot * 2 + 1] = *unit;
            }
            Self { cmos }
        }

        /// Returns the CMOS of the HDD boot of Towns OS V1.1 L20: floppy 0
        /// and 1, the OS ROM and SCSI ID 0 partition 0.
        fn l20() -> Self {
            Self::with_assignment(&[(0x00, 0x00), (0x00, 0x01), (0x05, 0x00), (0x02, 0x00)])
        }
    }

    impl MemoryAccess for CmosMemory {
        fn read_byte(&self, address: u32) -> u8 {
            address
                .checked_sub(CMOS_BASE)
                .and_then(|offset| self.cmos.get(offset as usize).copied())
                .unwrap_or(0)
        }

        fn write_byte(&mut self, _address: u32, _value: u8) {}

        fn read_word(&self, address: u32) -> u16 {
            u16::from_le_bytes([self.read_byte(address), self.read_byte(address + 1)])
        }

        fn write_word(&mut self, _address: u32, _value: u16) {}

        fn read_block(&self, address: u32, buffer: &mut [u8]) {
            for (offset, byte) in buffer.iter_mut().enumerate() {
                *byte = self.read_byte(address + offset as u32);
            }
        }

        fn write_block(&mut self, _address: u32, _data: &[u8]) {}
    }

    #[test]
    fn io_sys_units_of_the_hdd_boot_map_to_native_devices() {
        let entries = io_sys_unit_entries(
            &[FLOPPY_BPB, FLOPPY_BPB, OS_ROM_BPB, PARTITION_BPB],
            &CmosMemory::l20(),
            &mut ScsiDisk::l20(),
        );
        assert_eq!(
            entries,
            vec![
                Some(floppy_entry(0, 0)),
                Some(floppy_entry(1, 1)),
                Some(os_rom_entry(2)),
                Some(scsi_partition_entry(0, 0, 3, PARTITION_BPB)),
            ]
        );
    }

    #[test]
    fn io_sys_unit_with_an_unknown_bpb_is_left_to_its_driver() {
        let mut floppy_with_other_media = FLOPPY_BPB;
        floppy_with_other_media.media_descriptor = 0xFE;
        let bpb = PARTITION_BPB;
        let entries = io_sys_unit_entries(
            &[floppy_with_other_media, bpb, bpb],
            &CmosMemory::with_assignment(&[(0x00, 0x00), (0x02, 0x00), (0x02, 0x01)]),
            &mut ScsiDisk::l20(),
        );
        assert_eq!(
            entries[0],
            Some(floppy_entry(0, 0)),
            "the media byte varies"
        );
        assert!(entries[1].is_some());
        assert_eq!(entries[2], None, "the disk holds one partition");
    }

    #[test]
    fn partitions_with_one_bpb_follow_the_drive_assignment() {
        let bpb = PARTITION_BPB;
        let mut disk = ScsiDisk {
            partitions: vec![(0x01, PARTITION_SECTORS), (0x01, PARTITION_SECTORS)],
        };
        let entries = io_sys_unit_entries(
            &[FLOPPY_BPB, bpb, bpb],
            &CmosMemory::with_assignment(&[(0x00, 0x00), (0x02, 0x01), (0x02, 0x00)]),
            &mut disk,
        );
        assert_eq!(entries[1], Some(scsi_partition_entry(0, 1, 1, bpb)));
        assert_eq!(entries[2], Some(scsi_partition_entry(0, 0, 2, bpb)));
    }

    #[test]
    fn partition_index_counts_only_the_ms_dos_partitions() {
        let small = PARTITION_BPB;
        let large = LARGE_PARTITION_BPB;
        let mut disk = ScsiDisk {
            partitions: vec![
                (0x01, LARGE_PARTITION_SECTORS),
                (0x90, PARTITION_SECTORS),
                (0x01, PARTITION_SECTORS),
            ],
        };
        let entries = io_sys_unit_entries(
            &[small, large],
            &CmosMemory::with_assignment(&[(0x02, 0x01), (0x02, 0x00)]),
            &mut disk,
        );
        assert_eq!(entries[0], Some(scsi_partition_entry(0, 1, 0, small)));
        assert_eq!(entries[1], Some(scsi_partition_entry(0, 0, 1, large)));
    }

    #[test]
    fn unit_without_a_scsi_slot_is_left_to_its_driver() {
        let bpb = PARTITION_BPB;
        let entries = io_sys_unit_entries(
            &[bpb, bpb],
            &CmosMemory::with_assignment(&[(0x05, 0x00), (0x02, 0x10)]),
            &mut ScsiDisk::l20(),
        );
        assert_eq!(entries, vec![None, None]);
    }

    #[test]
    fn drive_assignment_reads_the_setup_block_of_the_cmos() {
        let assignment = DriveAssignment::read_cmos(&CmosMemory::with_assignment(&[
            (0x00, 0x00),
            (0x02, 0x31),
            (0x05, 0x00),
        ]));
        assert_eq!(assignment.partition(0), None);
        assert_eq!(
            assignment.partition(1),
            Some(AssignedPartition {
                target: 3,
                partition_index: 1,
            })
        );
        assert_eq!(assignment.partition(2), None);
        assert_eq!(assignment.partition(DRIVE_ASSIGNMENT_SLOTS), None);

        let empty = CmosMemory {
            cmos: vec![0; 0x800],
        };
        assert_eq!(
            DriveAssignment::read_cmos(&empty),
            DriveAssignment::default()
        );
    }

    #[test]
    fn device_codes_follow_the_disk_bios() {
        assert_eq!(floppy_device_id(0), 0x20);
        assert_eq!(floppy_device_id(FLOPPY_DRIVE_COUNT - 1), 0x23);
        assert_eq!(scsi_device_id(0), 0xB0);
        assert_eq!(scsi_device_id(SCSI_TARGET_COUNT - 1), 0xB6);
    }

    #[test]
    fn floppy_geometry_is_2hd_with_1024_byte_sectors() {
        let entry = floppy_entry(1, 1);
        assert_eq!(entry.device, DriveDevice::Floppy { device_id: 0x21 });
        assert_eq!(entry.unit, 1);
        let geometry = entry.geometry.unwrap();
        assert_eq!(geometry.bpb_source, BpbSource::FmTownsFloppy);
        assert_eq!(geometry.bpb.total_sectors(), 77 * 2 * 8);
        let dpb = geometry.dpb_geometry();
        assert_eq!(dpb.bytes_per_sector, 1024);
        assert_eq!(dpb.first_root_sector, 5);
        assert_eq!(dpb.first_data_sector, 11);
        assert_eq!(dpb.max_cluster, 1222);
        assert_eq!(dpb.media_descriptor, 0x01);
    }

    #[test]
    fn floppy_bpb_follows_the_fat_id_and_the_format() {
        assert_eq!(floppy_media_bpb(0xFE, 1024, 8), Some(FLOPPY_BPB));
        assert_eq!(floppy_media_bpb(0xF9, 1024, 8), Some(FLOPPY_BPB));
        assert_eq!(floppy_media_bpb(0xF0, 1024, 8), None);
        assert_eq!(floppy_media_bpb(0xFE, 512, 15), Some(FLOPPY_2HD_512_BPB));
        assert_eq!(floppy_media_bpb(0xF8, 512, 15), None);
        assert_eq!(floppy_media_bpb(0xF0, 512, 18), Some(FLOPPY_1440K_BPB));
        assert_eq!(floppy_media_bpb(0xF9, 512, 18), None);
        let two_dd = floppy_media_bpb(0xFB, 512, 8).unwrap();
        assert_eq!(two_dd.media_descriptor, 0xFB);
        assert_eq!(two_dd.total_sectors(), 80 * 2 * 8);
        let two_dd = floppy_media_bpb(0xF9, 512, 9).unwrap();
        assert_eq!(two_dd.total_sectors(), 80 * 2 * 9);
        assert_eq!(floppy_media_bpb(0xF0, 512, 9), None);
        assert_eq!(FLOPPY_2HD_512_BPB.total_sectors(), 80 * 2 * 15);
        assert_eq!(FLOPPY_1440K_BPB.total_sectors(), 80 * 2 * 18);
    }

    #[test]
    fn eight_inch_floppy_with_128_byte_sectors_gets_the_single_sided_bpb() {
        assert_eq!(floppy_media_bpb(0xFE, 128, 26), Some(FLOPPY_128_BPB));
        assert_eq!(floppy_media_bpb(0xF9, 128, 26), Some(FLOPPY_128_BPB));
        assert_eq!(floppy_media_bpb(0xF8, 128, 26), None);
        assert_eq!(floppy_media_bpb(0xFE, 256, 26), None);
        assert_eq!(FLOPPY_128_BPB.total_sectors(), 77 * 26);
        assert!(FLOPPY_128_BPB.is_supported_with_physical_sector_size(128));
    }

    #[test]
    fn fat_id_fe_in_the_1440k_mode_gets_the_1024_byte_bpb() {
        assert_eq!(
            floppy_media_bpb(0xFE, 512, 18),
            Some(FLOPPY_1440K_MODE_1024_BPB)
        );
        assert_eq!(FLOPPY_1440K_MODE_1024_BPB.bytes_per_sector, 1024);
        assert_eq!(FLOPPY_1440K_MODE_1024_BPB.total_sectors(), 0x09A0);
        assert_eq!(FLOPPY_1440K_MODE_1024_BPB.media_descriptor, 0x03);
    }

    #[test]
    fn os_rom_drive_uses_the_platform_bpb() {
        let entry = os_rom_entry(2);
        assert_eq!(entry.device, DriveDevice::MemoryImage { device_id: 0x42 });
        assert_eq!(entry.unit, 2);
        assert!(!entry.device.is_removable());
        let geometry = entry.geometry.unwrap();
        assert_eq!(geometry.bpb_source, BpbSource::Default);
        let dpb = geometry.dpb_geometry();
        assert_eq!(dpb.bytes_per_sector, 512);
        assert_eq!(dpb.cluster_mask, 1);
        assert_eq!(dpb.cluster_shift, 1);
        assert_eq!(dpb.first_fat_sector, 1);
        assert_eq!(dpb.fat_count, 2);
        assert_eq!(dpb.root_entries, 0x70);
        assert_eq!(dpb.sectors_per_fat, 2);
        assert_eq!(dpb.first_root_sector, 5);
        assert_eq!(dpb.first_data_sector, 12);
        assert_eq!(dpb.max_cluster, 635);
        assert_eq!(dpb.media_descriptor, 0xFB);
    }

    #[test]
    fn os_rom_clusters_match_the_image_layout() {
        let bytes_per_sector = u32::from(OS_ROM_BPB.bytes_per_sector);
        let fat_offset = u32::from(OS_ROM_BPB.reserved_sectors) * bytes_per_sector;
        let root_offset = OS_ROM_BPB.first_root_sector() * bytes_per_sector;
        let cluster_size = OS_ROM_BPB.cluster_size();
        let cluster_two_offset = OS_ROM_BPB.first_data_sector() * bytes_per_sector;
        let kernel_cluster = 0x4E;
        assert_eq!(fat_offset, 0x0200);
        assert_eq!(
            fat_offset + u32::from(OS_ROM_BPB.sectors_per_fat) * bytes_per_sector,
            0x0600
        );
        assert_eq!(root_offset, 0x0A00);
        assert_eq!(cluster_two_offset, 0x1800);
        assert_eq!(
            cluster_two_offset + (kernel_cluster - 2) * cluster_size,
            0x14800
        );
    }

    #[test]
    fn scsi_partition_takes_the_driver_bpb() {
        let entry = scsi_partition_entry(0, 0, 3, PARTITION_BPB);
        assert_eq!(entry.device, DriveDevice::FixedDisk { device_id: 0xB0 });
        assert_eq!(
            entry.partition_scheme,
            PartitionScheme::FmTowns { partition_index: 0 }
        );
        assert_eq!(entry.unit, 3);
        let geometry = entry.geometry.unwrap();
        assert_eq!(geometry.bpb_source, BpbSource::Default);
        let dpb = DpbGeometry::from_bpb(&geometry.bpb);
        assert_eq!(dpb.bytes_per_sector, 0x0800);
        assert_eq!(dpb.cluster_mask, 3);
        assert_eq!(dpb.root_entries, 0x0200);
        assert_eq!(dpb.media_descriptor, 0xF9);
        assert_eq!(geometry.bpb.total_sectors(), 0x1E00);
    }

    #[test]
    fn guest_unit_takes_the_driver_bpb() {
        let unit = GuestBlockUnit {
            header_segment: 0x1500,
            header_offset: 0x0000,
            unit: 0,
        };
        let entry = guest_unit_entry(unit, FLOPPY_BPB);
        assert_eq!(entry.device, DriveDevice::GuestDriver(unit));
        assert_eq!(entry.device.disk_device_id(), None);
        assert_eq!(entry.geometry.unwrap().bpb_source, BpbSource::Driver);
    }
}
