//! Drive map: the drive letters of DOS and the devices behind them.
//!
//! An entry names the device of a drive and its class, the unit number of the
//! drive in its device driver, the partition scheme of the medium and the
//! geometry of the volume. The geometry holds the BPB of the mounted volume and
//! the BPB that the platform supplies for media without one. The DPB of a drive
//! derives from that BPB.
//!
//! The platform modules build the map: `pc98` from the BIOS data area and
//! `fm_towns` for the block devices of the FM Towns `IO.SYS`.

pub mod fm_towns;
pub mod pc98;

use crate::{
    DiskIo, ERROR_INVALID_DRIVE,
    filesystem::{fat_bpb::Bpb, fat_partition, mbr_partition, towns_partition},
    layout::DpbGeometry,
};

/// Number of drive letters (A: to Z:).
pub const DRIVE_LETTER_COUNT: usize = 26;

/// Drive index of the virtual Z: drive.
pub const VIRTUAL_DRIVE_INDEX: u8 = 25;

save_state::runtime_state! {
    /// A block unit of a guest device driver in emulated memory.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct GuestBlockUnit {
        /// Segment of the device driver header.
        pub header_segment: u16,
        /// Offset of the device driver header.
        pub header_offset: u16,
        /// Unit number inside the driver.
        pub unit: u8,
    }
}

save_state::runtime_state! {
    /// A CD-ROM device of a guest device chain, reached through MSCDEX.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct GuestCdromDevice {
        /// Segment of the device driver header.
        pub header_segment: u16,
        /// Offset of the device driver header.
        pub header_offset: u16,
    }
}

/// Block device of a guest device chain with the BPBs that INIT returned
/// for its units.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChainBlockDevice {
    /// Segment of the device driver header.
    pub header_segment: u16,
    /// Offset of the device driver header.
    pub header_offset: u16,
    /// BPB of every unit in unit order.
    pub unit_bpbs: Vec<Bpb>,
}

/// Device behind a drive letter. The variant is the device class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriveDevice {
    /// Removable diskette drive read through `DiskIo`.
    Floppy {
        /// `DiskIo` device id.
        device_id: u8,
    },
    /// Partition of a fixed disk read through `DiskIo`.
    FixedDisk {
        /// `DiskIo` device id.
        device_id: u8,
    },
    /// Read-only memory image read through `DiskIo`.
    MemoryImage {
        /// `DiskIo` device id.
        device_id: u8,
    },
    /// CD-ROM drive read through `CdromIo` and served by the HLE MSCDEX.
    Cdrom,
    /// Block unit served by a guest device driver.
    GuestDriver(GuestBlockUnit),
    /// CD-ROM device of a guest driver, served through guest calls.
    GuestCdrom(GuestCdromDevice),
    /// Virtual drive of NeetanDos.
    Virtual,
}

/// Save state tag of `DriveDevice::Floppy`.
const DEVICE_TAG_FLOPPY: u8 = 0;
/// Save state tag of `DriveDevice::FixedDisk`.
const DEVICE_TAG_FIXED_DISK: u8 = 1;
/// Save state tag of `DriveDevice::MemoryImage`.
const DEVICE_TAG_MEMORY_IMAGE: u8 = 2;
/// Save state tag of `DriveDevice::Cdrom`.
const DEVICE_TAG_CDROM: u8 = 3;
/// Save state tag of `DriveDevice::GuestDriver`.
const DEVICE_TAG_GUEST_DRIVER: u8 = 4;
/// Save state tag of `DriveDevice::Virtual`.
const DEVICE_TAG_VIRTUAL: u8 = 5;
/// Save state tag of `DriveDevice::GuestCdrom`.
const DEVICE_TAG_GUEST_CDROM: u8 = 6;

impl save_state::StateEncode for DriveDevice {
    fn encode_state(&self, output: &mut Vec<u8>) {
        match self {
            Self::Floppy { device_id } => {
                save_state::StateEncode::encode_state(&DEVICE_TAG_FLOPPY, output);
                save_state::StateEncode::encode_state(device_id, output);
            }
            Self::FixedDisk { device_id } => {
                save_state::StateEncode::encode_state(&DEVICE_TAG_FIXED_DISK, output);
                save_state::StateEncode::encode_state(device_id, output);
            }
            Self::MemoryImage { device_id } => {
                save_state::StateEncode::encode_state(&DEVICE_TAG_MEMORY_IMAGE, output);
                save_state::StateEncode::encode_state(device_id, output);
            }
            Self::Cdrom => save_state::StateEncode::encode_state(&DEVICE_TAG_CDROM, output),
            Self::GuestDriver(unit) => {
                save_state::StateEncode::encode_state(&DEVICE_TAG_GUEST_DRIVER, output);
                save_state::StateEncode::encode_state(unit, output);
            }
            Self::Virtual => save_state::StateEncode::encode_state(&DEVICE_TAG_VIRTUAL, output),
            Self::GuestCdrom(device) => {
                save_state::StateEncode::encode_state(&DEVICE_TAG_GUEST_CDROM, output);
                save_state::StateEncode::encode_state(device, output);
            }
        }
    }
}

impl save_state::StateDecode for DriveDevice {
    fn decode_state(
        decoder: &mut save_state::StateDecoder<'_>,
    ) -> Result<Self, save_state::StateDecodeError> {
        match <u8 as save_state::StateDecode>::decode_state(decoder)? {
            DEVICE_TAG_FLOPPY => Ok(Self::Floppy {
                device_id: save_state::StateDecode::decode_state(decoder)?,
            }),
            DEVICE_TAG_FIXED_DISK => Ok(Self::FixedDisk {
                device_id: save_state::StateDecode::decode_state(decoder)?,
            }),
            DEVICE_TAG_MEMORY_IMAGE => Ok(Self::MemoryImage {
                device_id: save_state::StateDecode::decode_state(decoder)?,
            }),
            DEVICE_TAG_CDROM => Ok(Self::Cdrom),
            DEVICE_TAG_GUEST_DRIVER => Ok(Self::GuestDriver(
                save_state::StateDecode::decode_state(decoder)?,
            )),
            DEVICE_TAG_VIRTUAL => Ok(Self::Virtual),
            DEVICE_TAG_GUEST_CDROM => Ok(Self::GuestCdrom(save_state::StateDecode::decode_state(
                decoder,
            )?)),
            _ => Err(save_state::StateDecodeError::InvalidTag),
        }
    }
}

impl DriveDevice {
    /// Returns the `DiskIo` device id of a device read through `DiskIo`.
    pub const fn disk_device_id(&self) -> Option<u8> {
        match self {
            Self::Floppy { device_id }
            | Self::FixedDisk { device_id }
            | Self::MemoryImage { device_id } => Some(*device_id),
            Self::Cdrom | Self::GuestDriver(_) | Self::GuestCdrom(_) | Self::Virtual => None,
        }
    }

    /// Returns true for devices with removable media.
    pub const fn is_removable(&self) -> bool {
        match self {
            Self::Floppy { .. } | Self::Cdrom | Self::GuestDriver(_) | Self::GuestCdrom(_) => true,
            Self::FixedDisk { .. } | Self::MemoryImage { .. } | Self::Virtual => false,
        }
    }

    /// Returns true for devices with a drive parameter block.
    pub const fn has_drive_parameter_block(&self) -> bool {
        match self {
            Self::Floppy { .. }
            | Self::FixedDisk { .. }
            | Self::MemoryImage { .. }
            | Self::GuestDriver(_)
            | Self::Virtual => true,
            Self::Cdrom | Self::GuestCdrom(_) => false,
        }
    }

    /// Returns true for devices with an ISO 9660 file system.
    pub const fn has_cdrom_file_system(&self) -> bool {
        match self {
            Self::Cdrom | Self::GuestCdrom(_) => true,
            Self::Floppy { .. }
            | Self::FixedDisk { .. }
            | Self::MemoryImage { .. }
            | Self::GuestDriver(_)
            | Self::Virtual => false,
        }
    }
}

/// Partition scheme of the medium in a drive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartitionScheme {
    /// The volume starts at the first sector of the device.
    Unpartitioned,
    /// PC-98 partition table in sector 1. The first active DOS partition.
    Pc98,
    /// PC/AT master boot record. The first FAT partition, an active one first.
    MasterBootRecord,
    /// FM Towns partition table in sector 1. The n-th MS-DOS partition.
    FmTowns {
        /// Index among the MS-DOS partitions of the table (0-based).
        partition_index: u8,
    },
}

/// Save state tag of `PartitionScheme::Unpartitioned`.
const PARTITION_TAG_UNPARTITIONED: u8 = 0;
/// Save state tag of `PartitionScheme::Pc98`.
const PARTITION_TAG_PC98: u8 = 1;
/// Save state tag of `PartitionScheme::MasterBootRecord`.
const PARTITION_TAG_MASTER_BOOT_RECORD: u8 = 2;
/// Save state tag of `PartitionScheme::FmTowns`.
const PARTITION_TAG_FM_TOWNS: u8 = 3;

impl save_state::StateEncode for PartitionScheme {
    fn encode_state(&self, output: &mut Vec<u8>) {
        match self {
            Self::Unpartitioned => {
                save_state::StateEncode::encode_state(&PARTITION_TAG_UNPARTITIONED, output)
            }
            Self::Pc98 => save_state::StateEncode::encode_state(&PARTITION_TAG_PC98, output),
            Self::MasterBootRecord => {
                save_state::StateEncode::encode_state(&PARTITION_TAG_MASTER_BOOT_RECORD, output)
            }
            Self::FmTowns { partition_index } => {
                save_state::StateEncode::encode_state(&PARTITION_TAG_FM_TOWNS, output);
                save_state::StateEncode::encode_state(partition_index, output);
            }
        }
    }
}

impl save_state::StateDecode for PartitionScheme {
    fn decode_state(
        decoder: &mut save_state::StateDecoder<'_>,
    ) -> Result<Self, save_state::StateDecodeError> {
        match <u8 as save_state::StateDecode>::decode_state(decoder)? {
            PARTITION_TAG_UNPARTITIONED => Ok(Self::Unpartitioned),
            PARTITION_TAG_PC98 => Ok(Self::Pc98),
            PARTITION_TAG_MASTER_BOOT_RECORD => Ok(Self::MasterBootRecord),
            PARTITION_TAG_FM_TOWNS => Ok(Self::FmTowns {
                partition_index: save_state::StateDecode::decode_state(decoder)?,
            }),
            _ => Err(save_state::StateDecodeError::InvalidTag),
        }
    }
}

impl PartitionScheme {
    /// Returns the first physical sector of the volume on the device.
    pub fn volume_start(&self, device_id: u8, disk: &mut dyn DiskIo) -> Result<u32, u16> {
        match self {
            Self::Unpartitioned => Ok(0),
            Self::Pc98 => fat_partition::find_partition_offset(device_id, disk),
            Self::MasterBootRecord => mbr_partition::find_mbr_partition_offset(device_id, disk),
            Self::FmTowns { partition_index } => {
                towns_partition::find_partition_offset(device_id, *partition_index, disk)
            }
        }
    }
}

save_state::runtime_state_enum! {
    /// Source of the BPB of a volume.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum BpbSource {
        /// The boot record of the volume holds the BPB.
        BootRecord = 0,
        /// The boot record holds the BPB. Old media without one use the default
        /// BPB when the size of the device and the FAT ID match it.
        BootRecordOrDefault = 1,
        /// The volume uses the default BPB of the platform.
        Default = 2,
        /// The guest device driver of the drive reports the BPB.
        Driver = 3,
        /// The FM Towns `IO.SYS` block driver selects the BPB from the FAT ID
        /// of the medium and its physical format. It ignores the boot record.
        FmTownsFloppy = 4,
    }
}

save_state::runtime_state! {
    /// Volume geometry of a drive.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct DriveGeometry {
        /// Source of the BPB of a mounted volume.
        pub bpb_source: BpbSource,
        /// BPB that the platform supplies for the standard medium of the drive.
        pub default_bpb: Bpb,
        /// BPB of the mounted volume, or the default BPB before a volume mounts.
        pub bpb: Bpb,
    }
}

impl DriveGeometry {
    /// Creates the geometry of a drive before a volume mounts.
    pub const fn new(bpb_source: BpbSource, default_bpb: Bpb) -> Self {
        Self {
            bpb_source,
            default_bpb,
            bpb: default_bpb,
        }
    }

    /// Returns the DPB geometry of the current BPB.
    pub fn dpb_geometry(&self) -> DpbGeometry {
        DpbGeometry::from_bpb(&self.bpb)
    }
}

save_state::runtime_state! {
    /// One drive letter of the drive map.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct DriveEntry {
        /// Device and device class.
        pub device: DriveDevice,
        /// Unit number of the drive in its device driver (DPB byte 01h).
        pub unit: u8,
        /// Partition scheme of the medium.
        pub partition_scheme: PartitionScheme,
        /// Volume geometry. Drives without a DPB have none.
        pub geometry: Option<DriveGeometry>,
    }
}

impl DriveEntry {
    /// Creates the entry of the virtual drive.
    pub const fn virtual_drive() -> Self {
        Self {
            device: DriveDevice::Virtual,
            unit: 0,
            partition_scheme: PartitionScheme::Unpartitioned,
            geometry: Some(DriveGeometry::new(BpbSource::Default, VIRTUAL_DRIVE_BPB)),
        }
    }

    /// Creates the entry of the CD-ROM drive.
    pub const fn cdrom() -> Self {
        Self {
            device: DriveDevice::Cdrom,
            unit: 0,
            partition_scheme: PartitionScheme::Unpartitioned,
            geometry: None,
        }
    }
}

/// BPB of the virtual drive: one sector of FAT, one sector of root directory
/// and one data cluster.
pub const VIRTUAL_DRIVE_BPB: Bpb = Bpb {
    bytes_per_sector: 512,
    sectors_per_cluster: 1,
    reserved_sectors: 1,
    num_fats: 1,
    root_entry_count: 16,
    total_sectors_16: 4,
    media_descriptor: 0xF8,
    sectors_per_fat: 1,
    total_sectors_32: 0,
};

save_state::runtime_state! {
    /// The drive letters A: to Z: and their devices.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct DriveMap {
        /// One slot per drive letter.
        entries: Vec<Option<DriveEntry>>,
    }
}

impl Default for DriveMap {
    fn default() -> Self {
        Self::new()
    }
}

impl DriveMap {
    /// Creates a map without drives.
    pub fn new() -> Self {
        Self {
            entries: vec![None; DRIVE_LETTER_COUNT],
        }
    }

    /// Returns the entry of a drive (0 = A:).
    pub fn entry(&self, drive_index: u8) -> Option<&DriveEntry> {
        self.entries
            .get(usize::from(drive_index))
            .and_then(Option::as_ref)
    }

    /// Returns the mutable entry of a drive (0 = A:).
    pub fn entry_mut(&mut self, drive_index: u8) -> Option<&mut DriveEntry> {
        self.entries
            .get_mut(usize::from(drive_index))
            .and_then(Option::as_mut)
    }

    /// Assigns a drive letter. Indices beyond Z: are ignored.
    pub fn set(&mut self, drive_index: u8, entry: DriveEntry) {
        if let Some(slot) = self.entries.get_mut(usize::from(drive_index)) {
            *slot = Some(entry);
        }
    }

    /// Removes a drive letter.
    pub fn remove(&mut self, drive_index: u8) {
        if let Some(slot) = self.entries.get_mut(usize::from(drive_index)) {
            *slot = None;
        }
    }

    /// Iterates over the assigned drives in letter order.
    pub fn drives(&self) -> impl Iterator<Item = (u8, &DriveEntry)> {
        self.entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| entry.as_ref().map(|entry| (index as u8, entry)))
    }

    /// Iterates over the drives with a drive parameter block in letter order.
    pub fn drives_with_parameter_block(&self) -> impl Iterator<Item = (u8, &DriveEntry)> {
        self.drives()
            .filter(|(_, entry)| entry.device.has_drive_parameter_block())
    }

    /// Iterates over the drives on the `DiskIo` device `device_id`.
    pub fn drives_on_disk_device(&self, device_id: u8) -> impl Iterator<Item = u8> + '_ {
        self.drives()
            .filter(move |(_, entry)| entry.device.disk_device_id() == Some(device_id))
            .map(|(drive_index, _)| drive_index)
    }

    /// Returns the `DiskIo` device id of a drive.
    pub fn disk_device_id(&self, drive_index: u8) -> Result<u8, u16> {
        self.entry(drive_index)
            .and_then(|entry| entry.device.disk_device_id())
            .ok_or(ERROR_INVALID_DRIVE)
    }

    /// Builds the drive map of the block devices of a guest device chain.
    /// The drive letters count up over the units in chain order from A:.
    /// The rule of `machine` names the native device behind a unit of the
    /// first block device. It may read the machine configuration from
    /// `memory`. Every other unit is a guest unit.
    pub fn from_chain_block_devices(
        machine: crate::DosMachine,
        devices: &[ChainBlockDevice],
        memory: &dyn crate::MemoryAccess,
        disk: &mut dyn DiskIo,
    ) -> Self {
        let mut map = Self::new();
        let mut drive_index = 0u8;
        for (device_index, device) in devices.iter().enumerate() {
            let native_entries = match (machine, device_index) {
                (crate::DosMachine::FmTowns, 0) => {
                    fm_towns::io_sys_unit_entries(&device.unit_bpbs, memory, disk)
                }
                (crate::DosMachine::FmTowns, _) | (crate::DosMachine::Pc98, _) => Vec::new(),
            };
            for (unit, bpb) in device.unit_bpbs.iter().enumerate() {
                if usize::from(drive_index) >= DRIVE_LETTER_COUNT {
                    return map;
                }
                let entry = native_entries
                    .get(unit)
                    .copied()
                    .flatten()
                    .unwrap_or_else(|| {
                        fm_towns::guest_unit_entry(
                            GuestBlockUnit {
                                header_segment: device.header_segment,
                                header_offset: device.header_offset,
                                unit: unit as u8,
                            },
                            *bpb,
                        )
                    });
                map.set(drive_index, entry);
                drive_index += 1;
            }
        }
        map
    }

    /// Checks the slot count of a decoded map.
    pub fn is_valid(&self) -> bool {
        self.entries.len() == DRIVE_LETTER_COUNT
            && self
                .entries
                .iter()
                .flatten()
                .all(|entry| entry.device.has_drive_parameter_block() == entry.geometry.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Machine without disks.
    struct NoDisks;

    impl DiskIo for NoDisks {
        fn read_sectors(&mut self, _device_id: u8, _lba: u32, _count: u32) -> Result<Vec<u8>, u8> {
            Err(0x80)
        }

        fn write_sectors(&mut self, _device_id: u8, _lba: u32, _data: &[u8]) -> Result<(), u8> {
            Err(0x80)
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

    /// Returns a chain block device at `0A00h:offset`.
    fn chain_device(offset: u16, unit_bpbs: Vec<Bpb>) -> ChainBlockDevice {
        ChainBlockDevice {
            header_segment: 0x0A00,
            header_offset: offset,
            unit_bpbs,
        }
    }

    /// Returns the guest unit entry of `unit` of the device at `0A00h:offset`.
    fn guest_unit(offset: u16, unit: u8, bpb: Bpb) -> DriveEntry {
        fm_towns::guest_unit_entry(
            GuestBlockUnit {
                header_segment: 0x0A00,
                header_offset: offset,
                unit,
            },
            bpb,
        )
    }

    #[test]
    fn chain_units_count_up_from_a_over_every_block_device() {
        let devices = [
            chain_device(0x0062, vec![fm_towns::FLOPPY_BPB, fm_towns::OS_ROM_BPB]),
            chain_device(0x0074, vec![VIRTUAL_DRIVE_BPB]),
        ];
        let memory = crate::test_support::MockMemory::with_extended_memory(0x1000, 0);
        let map = DriveMap::from_chain_block_devices(
            crate::DosMachine::FmTowns,
            &devices,
            &memory,
            &mut NoDisks,
        );
        assert_eq!(map.entry(0), Some(&fm_towns::floppy_entry(0, 0)));
        assert_eq!(map.entry(1), Some(&fm_towns::os_rom_entry(1)));
        assert_eq!(
            map.entry(2),
            Some(&guest_unit(0x0074, 0, VIRTUAL_DRIVE_BPB))
        );
        assert_eq!(map.entry(3), None);
    }

    #[test]
    fn units_of_a_later_device_are_guest_units_with_any_bpb() {
        let devices = [
            chain_device(0x0062, vec![fm_towns::OS_ROM_BPB]),
            chain_device(0x0074, vec![fm_towns::FLOPPY_BPB]),
        ];
        let memory = crate::test_support::MockMemory::with_extended_memory(0x1000, 0);
        let map = DriveMap::from_chain_block_devices(
            crate::DosMachine::FmTowns,
            &devices,
            &memory,
            &mut NoDisks,
        );
        assert_eq!(map.entry(0), Some(&fm_towns::os_rom_entry(0)));
        assert_eq!(
            map.entry(1),
            Some(&guest_unit(0x0074, 0, fm_towns::FLOPPY_BPB))
        );
    }

    #[test]
    fn pc98_chain_units_are_guest_units() {
        let devices = [chain_device(
            0x0062,
            vec![fm_towns::FLOPPY_BPB, fm_towns::OS_ROM_BPB],
        )];
        let memory = crate::test_support::MockMemory::with_extended_memory(0x1000, 0);
        let map = DriveMap::from_chain_block_devices(
            crate::DosMachine::Pc98,
            &devices,
            &memory,
            &mut NoDisks,
        );
        assert_eq!(
            map.entry(0),
            Some(&guest_unit(0x0062, 0, fm_towns::FLOPPY_BPB))
        );
        assert_eq!(
            map.entry(1),
            Some(&guest_unit(0x0062, 1, fm_towns::OS_ROM_BPB))
        );
    }

    /// Floppy entry with the given device id and the virtual drive BPB.
    fn floppy(device_id: u8) -> DriveEntry {
        DriveEntry {
            device: DriveDevice::Floppy { device_id },
            unit: device_id & 0x0F,
            partition_scheme: PartitionScheme::Unpartitioned,
            geometry: Some(DriveGeometry::new(BpbSource::BootRecord, VIRTUAL_DRIVE_BPB)),
        }
    }

    #[test]
    fn new_map_has_no_drives() {
        let map = DriveMap::new();
        assert_eq!(map.drives().count(), 0);
        assert!(map.is_valid());
        assert_eq!(map.disk_device_id(0), Err(ERROR_INVALID_DRIVE));
    }

    #[test]
    fn drives_iterate_in_letter_order() {
        let mut map = DriveMap::new();
        map.set(VIRTUAL_DRIVE_INDEX, DriveEntry::virtual_drive());
        map.set(16, DriveEntry::cdrom());
        map.set(1, floppy(0x91));
        map.set(0, floppy(0x90));
        let letters: Vec<u8> = map.drives().map(|(drive, _)| drive).collect();
        assert_eq!(letters, vec![0, 1, 16, VIRTUAL_DRIVE_INDEX]);
        let with_dpb: Vec<u8> = map
            .drives_with_parameter_block()
            .map(|(drive, _)| drive)
            .collect();
        assert_eq!(with_dpb, vec![0, 1, VIRTUAL_DRIVE_INDEX]);
    }

    #[test]
    fn set_ignores_indices_beyond_z() {
        let mut map = DriveMap::new();
        map.set(26, floppy(0x90));
        assert_eq!(map.drives().count(), 0);
    }

    #[test]
    fn disk_device_lookup() {
        let mut map = DriveMap::new();
        map.set(0, floppy(0x90));
        map.set(2, floppy(0x90));
        map.set(VIRTUAL_DRIVE_INDEX, DriveEntry::virtual_drive());
        assert_eq!(map.disk_device_id(0), Ok(0x90));
        assert_eq!(
            map.disk_device_id(VIRTUAL_DRIVE_INDEX),
            Err(ERROR_INVALID_DRIVE)
        );
        let drives: Vec<u8> = map.drives_on_disk_device(0x90).collect();
        assert_eq!(drives, vec![0, 2]);
        map.remove(2);
        assert_eq!(map.drives_on_disk_device(0x90).count(), 1);
    }

    #[test]
    fn device_classes() {
        let guest = DriveDevice::GuestDriver(GuestBlockUnit {
            header_segment: 0x1234,
            header_offset: 0x0010,
            unit: 1,
        });
        assert_eq!(guest.disk_device_id(), None);
        assert!(guest.is_removable());
        assert!(guest.has_drive_parameter_block());
        assert!(!DriveDevice::Cdrom.has_drive_parameter_block());
        assert!(!DriveDevice::FixedDisk { device_id: 0x80 }.is_removable());
        assert_eq!(
            DriveDevice::MemoryImage { device_id: 0x42 }.disk_device_id(),
            Some(0x42)
        );
    }

    #[test]
    fn virtual_drive_dpb_has_one_data_cluster() {
        let geometry = DriveEntry::virtual_drive().geometry.unwrap().dpb_geometry();
        assert_eq!(geometry.bytes_per_sector, 512);
        assert_eq!(geometry.cluster_mask, 0);
        assert_eq!(geometry.first_fat_sector, 1);
        assert_eq!(geometry.fat_count, 1);
        assert_eq!(geometry.root_entries, 16);
        assert_eq!(geometry.first_root_sector, 2);
        assert_eq!(geometry.first_data_sector, 3);
        assert_eq!(geometry.max_cluster, 2);
        assert_eq!(geometry.media_descriptor, 0xF8);
    }

    #[test]
    fn map_round_trips_through_the_save_state() {
        let mut map = DriveMap::new();
        map.set(0, floppy(0x90));
        map.set(
            3,
            DriveEntry {
                device: DriveDevice::FixedDisk { device_id: 0xB0 },
                unit: 3,
                partition_scheme: PartitionScheme::FmTowns { partition_index: 1 },
                geometry: Some(DriveGeometry::new(BpbSource::Default, VIRTUAL_DRIVE_BPB)),
            },
        );
        map.set(
            4,
            DriveEntry {
                device: DriveDevice::GuestDriver(GuestBlockUnit {
                    header_segment: 0x2000,
                    header_offset: 0x0000,
                    unit: 0,
                }),
                unit: 0,
                partition_scheme: PartitionScheme::Unpartitioned,
                geometry: Some(DriveGeometry::new(BpbSource::Driver, VIRTUAL_DRIVE_BPB)),
            },
        );
        map.set(16, DriveEntry::cdrom());
        map.set(VIRTUAL_DRIVE_INDEX, DriveEntry::virtual_drive());
        let encoded = save_state::encode_runtime_state(&map);
        let decoded: DriveMap =
            save_state::decode_runtime_state(&encoded, DRIVE_LETTER_COUNT).unwrap();
        assert_eq!(decoded, map);
        assert!(decoded.is_valid());
    }

    #[test]
    fn map_without_geometry_on_a_dpb_drive_is_invalid() {
        let mut map = DriveMap::new();
        let mut entry = floppy(0x90);
        entry.geometry = None;
        map.set(0, entry);
        assert!(!map.is_valid());
    }
}
