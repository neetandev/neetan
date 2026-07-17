//! MSX model definitions and static machine layouts.

use core::fmt;

pub use device::video_msx::MsxVdpVersion;

/// MSX hardware generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MsxGeneration {
    /// First-generation MSX.
    Msx1,
    /// MSX2.
    Msx2,
    /// MSX2+.
    Msx2Plus,
}

/// Built-in floppy-controller family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MsxDiskController {
    /// No built-in floppy controller.
    None,
    /// Sony WD2793-class memory-mapped controller.
    SonyWd2793,
}

/// Physical keyboard layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MsxKeyboardLayout {
    /// Japanese ANSI keyboard without a numeric keypad.
    JapaneseAnsi,
    /// Japanese JIS keyboard with a numeric keypad.
    JapaneseJis,
}

/// MSX machine model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MsxModel {
    /// MSX using Sony HB-201 firmware.
    Msx,
    /// MSX2 using Sony HB-F1XD firmware with 512 KiB mapper RAM.
    Msx2,
    /// MSX2+ using Sony HB-F1XDJ firmware with 512 KiB mapper RAM.
    Msx2Plus,
}

impl MsxModel {
    /// Every supported model.
    pub const ALL: [Self; 3] = [Self::Msx, Self::Msx2, Self::Msx2Plus];

    /// MSX generation.
    pub const fn generation(self) -> MsxGeneration {
        match self {
            Self::Msx => MsxGeneration::Msx1,
            Self::Msx2 => MsxGeneration::Msx2,
            Self::Msx2Plus => MsxGeneration::Msx2Plus,
        }
    }

    /// Master-clock configuration.
    pub const fn clock_profile(self) -> MsxClockProfile {
        MsxClockProfile {
            master_clock_hz: 21_477_270,
            normal_cpu_divisor: 6,
        }
    }

    /// Normal Z80 clock in integer Hz.
    pub const fn main_clock_hz(self) -> u32 {
        self.clock_profile().master_clock_hz / self.clock_profile().normal_cpu_divisor as u32
    }

    /// Video processor version.
    pub const fn vdp_version(self) -> MsxVdpVersion {
        match self {
            Self::Msx => MsxVdpVersion::Tms9118,
            Self::Msx2 => MsxVdpVersion::V9938,
            Self::Msx2Plus => MsxVdpVersion::V9958,
        }
    }

    /// Video RAM size in bytes.
    pub const fn vram_size(self) -> usize {
        match self.generation() {
            MsxGeneration::Msx1 => 16 << 10,
            MsxGeneration::Msx2 | MsxGeneration::Msx2Plus => 128 << 10,
        }
    }

    /// Main RAM size in bytes.
    pub const fn work_ram_size(self) -> usize {
        match self.memory_mapper_size() {
            Some(size) => size,
            None => 64 << 10,
        }
    }

    /// Installed memory-mapper RAM size, if the main RAM uses a mapper.
    pub const fn memory_mapper_size(self) -> Option<usize> {
        match self {
            Self::Msx2 | Self::Msx2Plus => Some(512 << 10),
            Self::Msx => None,
        }
    }

    /// Memory-mapper register readback wiring.
    pub const fn mapper_readback(self) -> Option<MapperReadback> {
        match self {
            Self::Msx2 | Self::Msx2Plus => Some(MapperReadback {
                mask: 0x1F,
                fixed_bits: 0x80,
            }),
            Self::Msx => None,
        }
    }

    /// Built-in floppy controller.
    pub const fn disk_controller(self) -> MsxDiskController {
        match self {
            Self::Msx => MsxDiskController::None,
            Self::Msx2 | Self::Msx2Plus => MsxDiskController::SonyWd2793,
        }
    }

    /// Number of built-in floppy drives.
    pub const fn drive_count(self) -> u8 {
        match self.disk_controller() {
            MsxDiskController::None => 0,
            MsxDiskController::SonyWd2793 => 1,
        }
    }

    /// Whether the machine has an RP5C01 RTC.
    pub const fn has_rtc(self) -> bool {
        !matches!(self, Self::Msx)
    }

    /// Installed Kanji ROM size in bytes.
    pub const fn kanji_rom_size(self) -> Option<usize> {
        match self {
            Self::Msx2Plus => Some(256 << 10),
            Self::Msx | Self::Msx2 => None,
        }
    }

    /// Whether an S1985 system controller is present.
    pub const fn has_s1985(self) -> bool {
        matches!(self, Self::Msx2 | Self::Msx2Plus)
    }

    /// Whether MSX-MUSIC is built in.
    pub const fn has_msx_music(self) -> bool {
        matches!(self, Self::Msx2Plus)
    }

    /// Physical keyboard layout.
    pub const fn keyboard_layout(self) -> MsxKeyboardLayout {
        match self {
            Self::Msx => MsxKeyboardLayout::JapaneseAnsi,
            Self::Msx2 | Self::Msx2Plus => MsxKeyboardLayout::JapaneseJis,
        }
    }

    /// Whether the keyboard has a numeric keypad.
    pub const fn has_numeric_keypad(self) -> bool {
        matches!(self.keyboard_layout(), MsxKeyboardLayout::JapaneseJis)
    }

    /// Keyboard-layout bit wired to PSG port A.
    pub const fn psg_keyboard_layout_bit(self) -> u8 {
        match self.keyboard_layout() {
            MsxKeyboardLayout::JapaneseAnsi => 0,
            MsxKeyboardLayout::JapaneseJis => 0x40,
        }
    }

    /// Static primary and secondary slot topology.
    pub const fn slot_layout(self) -> MsxSlotLayout {
        match self {
            Self::Msx => HB201_LAYOUT,
            Self::Msx2 => HBF1XD_LAYOUT,
            Self::Msx2Plus => HBF1XDJ_LAYOUT,
        }
    }

    /// Logical firmware placement within the slot topology.
    pub const fn firmware_layout(self) -> &'static [FirmwarePlacement] {
        match self {
            Self::Msx => HB201_FIRMWARE,
            Self::Msx2 => HBF1XD_FIRMWARE,
            Self::Msx2Plus => HBF1XDJ_FIRMWARE,
        }
    }
}

impl fmt::Display for MsxModel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Msx => "MSX",
            Self::Msx2 => "MSX2",
            Self::Msx2Plus => "MSX2PLUS",
        })
    }
}

impl core::str::FromStr for MsxModel {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text.to_ascii_uppercase().as_str() {
            "MSX" => Ok(Self::Msx),
            "MSX2" => Ok(Self::Msx2),
            "MSX2PLUS" => Ok(Self::Msx2Plus),
            _ => Err(format!("unknown MSX model '{text}'")),
        }
    }
}

/// Exact clock relationship for one model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MsxClockProfile {
    /// Shared master clock in Hz.
    pub master_clock_hz: u32,
    /// Master-clock divisor for the normal Z80 mode.
    pub normal_cpu_divisor: u8,
}

/// Memory-mapper register readback wiring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapperReadback {
    /// Bits supplied by the selected mapper segment.
    pub mask: u8,
    /// Bits forced by the system controller.
    pub fixed_bits: u8,
}

/// One primary or secondary MSX slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MsxSlot {
    /// Primary slot number.
    pub primary: u8,
    /// Secondary slot number when the primary slot is expanded.
    pub secondary: Option<u8>,
}

impl MsxSlot {
    const fn primary(primary: u8) -> Self {
        Self {
            primary,
            secondary: None,
        }
    }

    const fn secondary(primary: u8, secondary: u8) -> Self {
        Self {
            primary,
            secondary: Some(secondary),
        }
    }
}

/// Logical firmware region used by a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FirmwareRegion {
    /// BIOS and BASIC.
    Bios,
    /// Sony Personal Data Bank.
    PersonalDataBank,
    /// MSX2 or MSX2+ sub-ROM.
    SubRom,
    /// Floppy disk ROM.
    DiskRom,
    /// Kanji driver and BASIC extension.
    KanjiDriver,
    /// MSX-MUSIC firmware.
    MsxMusic,
    /// Banked manufacturer firmware.
    FirmwareMapper,
    /// Kanji character generator.
    KanjiFont,
    /// YM2413 built-in instrument data.
    OpllInstruments,
    /// Panasonic FS-CA1 MSX-AUDIO firmware.
    MsxAudio,
}

impl fmt::Display for FirmwareRegion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Bios => "BIOS",
            Self::PersonalDataBank => "Personal Data Bank",
            Self::SubRom => "sub-ROM",
            Self::DiskRom => "disk ROM",
            Self::KanjiDriver => "Kanji driver",
            Self::MsxMusic => "MSX-MUSIC",
            Self::FirmwareMapper => "firmware mapper",
            Self::KanjiFont => "Kanji font",
            Self::OpllInstruments => "YM2413 instruments",
            Self::MsxAudio => "FS-CA1 MSX-AUDIO",
        })
    }
}

/// Kind of device occupying a slot address range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsxSlotDeviceKind {
    /// A logical firmware region.
    Firmware(FirmwareRegion),
    /// Linear work RAM.
    PlainRam,
    /// Banked memory-mapper RAM.
    MapperRam,
    /// External cartridge connector.
    Cartridge(u8),
    /// Sony banked firmware mapper.
    SonyFirmwareMapper(FirmwareRegion),
}

/// One statically described device mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MsxSlotDevice {
    /// Slot containing the device.
    pub slot: MsxSlot,
    /// First CPU address where the device is visible.
    pub address: u16,
    /// Size of the visible CPU range.
    pub size: u32,
    /// Device category.
    pub kind: MsxSlotDeviceKind,
}

/// Validated static slot layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MsxSlotLayout {
    expanded_primary_mask: u8,
    devices: &'static [MsxSlotDevice],
}

impl MsxSlotLayout {
    const fn new(expanded_primary_mask: u8, devices: &'static [MsxSlotDevice]) -> Self {
        Self {
            expanded_primary_mask,
            devices,
        }
    }

    /// Whether a primary slot is expanded.
    pub const fn primary_is_expanded(self, primary: u8) -> bool {
        primary < 4 && self.expanded_primary_mask & (1 << primary) != 0
    }

    /// Statically described slot devices.
    pub const fn devices(self) -> &'static [MsxSlotDevice] {
        self.devices
    }

    /// Validates all slot coordinates and address ranges.
    pub fn validate(self) -> Result<(), SlotLayoutError> {
        if self.expanded_primary_mask & !0x0F != 0 {
            return Err(SlotLayoutError::InvalidExpandedMask(
                self.expanded_primary_mask,
            ));
        }
        for (index, device) in self.devices.iter().enumerate() {
            validate_device(self, index, device)?;
            for (other_index, other) in self.devices[..index].iter().enumerate() {
                if device.slot == other.slot && ranges_overlap(device, other) {
                    return Err(SlotLayoutError::Overlap {
                        first: other_index,
                        second: index,
                    });
                }
            }
        }
        Ok(())
    }

    #[cfg(test)]
    const fn for_test(expanded_primary_mask: u8, devices: &'static [MsxSlotDevice]) -> Self {
        Self::new(expanded_primary_mask, devices)
    }
}

/// Slot-layout validation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotLayoutError {
    /// The expanded-slot mask contains an invalid primary slot.
    InvalidExpandedMask(u8),
    /// A device names an invalid primary slot.
    InvalidPrimary {
        /// Device index.
        device: usize,
        /// Invalid primary slot.
        primary: u8,
    },
    /// A device names an invalid secondary slot.
    InvalidSecondary {
        /// Device index.
        device: usize,
        /// Invalid secondary slot.
        secondary: u8,
    },
    /// A secondary slot is used under an unexpanded primary slot.
    PrimaryNotExpanded {
        /// Device index.
        device: usize,
        /// Unexpanded primary slot.
        primary: u8,
    },
    /// A device has an empty or out-of-range CPU address span.
    InvalidAddressRange {
        /// Device index.
        device: usize,
    },
    /// Two devices overlap within one slot.
    Overlap {
        /// First device index.
        first: usize,
        /// Second device index.
        second: usize,
    },
}

impl fmt::Display for SlotLayoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for SlotLayoutError {}

/// Placement of one logical firmware region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FirmwarePlacement {
    /// Logical firmware region.
    pub region: FirmwareRegion,
    /// Slot containing the region.
    pub slot: MsxSlot,
    /// First CPU address where the region is mapped.
    pub address: u16,
    /// Size of the visible CPU window.
    pub mapped_size: u32,
    /// Whether source bytes repeat through the visible window.
    pub mirrored: bool,
}

const fn device(slot: MsxSlot, address: u16, size: u32, kind: MsxSlotDeviceKind) -> MsxSlotDevice {
    MsxSlotDevice {
        slot,
        address,
        size,
        kind,
    }
}

const fn firmware(
    region: FirmwareRegion,
    slot: MsxSlot,
    address: u16,
    mapped_size: u32,
    mirrored: bool,
) -> FirmwarePlacement {
    FirmwarePlacement {
        region,
        slot,
        address,
        mapped_size,
        mirrored,
    }
}

fn validate_device(
    layout: MsxSlotLayout,
    index: usize,
    device: &MsxSlotDevice,
) -> Result<(), SlotLayoutError> {
    if device.slot.primary >= 4 {
        return Err(SlotLayoutError::InvalidPrimary {
            device: index,
            primary: device.slot.primary,
        });
    }
    if let Some(secondary) = device.slot.secondary {
        if secondary >= 4 {
            return Err(SlotLayoutError::InvalidSecondary {
                device: index,
                secondary,
            });
        }
        if !layout.primary_is_expanded(device.slot.primary) {
            return Err(SlotLayoutError::PrimaryNotExpanded {
                device: index,
                primary: device.slot.primary,
            });
        }
    } else if layout.primary_is_expanded(device.slot.primary) {
        return Err(SlotLayoutError::PrimaryNotExpanded {
            device: index,
            primary: device.slot.primary,
        });
    }
    let end = u32::from(device.address).saturating_add(device.size);
    if device.size == 0 || end > 0x1_0000 {
        return Err(SlotLayoutError::InvalidAddressRange { device: index });
    }
    Ok(())
}

fn ranges_overlap(left: &MsxSlotDevice, right: &MsxSlotDevice) -> bool {
    let left_start = u32::from(left.address);
    let left_end = left_start + left.size;
    let right_start = u32::from(right.address);
    let right_end = right_start + right.size;
    left_start < right_end && right_start < left_end
}

/// Size of one complete CPU-visible slot.
const FULL_SLOT: u32 = 0x1_0000;

/// HB-201 firmware placement.
const HB201_FIRMWARE: &[FirmwarePlacement] = &[
    firmware(
        FirmwareRegion::Bios,
        MsxSlot::primary(0),
        0x0000,
        0x8000,
        false,
    ),
    firmware(
        FirmwareRegion::PersonalDataBank,
        MsxSlot::primary(0),
        0x8000,
        0x4000,
        false,
    ),
];
/// HB-201 slot devices.
const HB201_DEVICES: &[MsxSlotDevice] = &[
    device(
        MsxSlot::primary(0),
        0x0000,
        0x8000,
        MsxSlotDeviceKind::Firmware(FirmwareRegion::Bios),
    ),
    device(
        MsxSlot::primary(0),
        0x8000,
        0x4000,
        MsxSlotDeviceKind::Firmware(FirmwareRegion::PersonalDataBank),
    ),
    device(
        MsxSlot::primary(1),
        0x0000,
        FULL_SLOT,
        MsxSlotDeviceKind::Cartridge(0),
    ),
    device(
        MsxSlot::primary(2),
        0x0000,
        FULL_SLOT,
        MsxSlotDeviceKind::Cartridge(1),
    ),
    device(
        MsxSlot::primary(3),
        0x0000,
        FULL_SLOT,
        MsxSlotDeviceKind::PlainRam,
    ),
];
/// HB-201 slot layout.
const HB201_LAYOUT: MsxSlotLayout = MsxSlotLayout::new(0, HB201_DEVICES);

/// HB-F1XD firmware placement.
const HBF1XD_FIRMWARE: &[FirmwarePlacement] = &[
    firmware(
        FirmwareRegion::Bios,
        MsxSlot::primary(0),
        0x0000,
        0x8000,
        false,
    ),
    firmware(
        FirmwareRegion::SubRom,
        MsxSlot::secondary(3, 0),
        0x0000,
        0x4000,
        false,
    ),
    firmware(
        FirmwareRegion::DiskRom,
        MsxSlot::secondary(3, 0),
        0x4000,
        0x8000,
        true,
    ),
];
/// HB-F1XD slot devices with the target's 512 KiB mapper upgrade.
const HBF1XD_DEVICES: &[MsxSlotDevice] = &[
    device(
        MsxSlot::primary(0),
        0x0000,
        0x8000,
        MsxSlotDeviceKind::Firmware(FirmwareRegion::Bios),
    ),
    device(
        MsxSlot::primary(1),
        0x0000,
        FULL_SLOT,
        MsxSlotDeviceKind::Cartridge(0),
    ),
    device(
        MsxSlot::primary(2),
        0x0000,
        FULL_SLOT,
        MsxSlotDeviceKind::Cartridge(1),
    ),
    device(
        MsxSlot::secondary(3, 0),
        0x0000,
        0x4000,
        MsxSlotDeviceKind::Firmware(FirmwareRegion::SubRom),
    ),
    device(
        MsxSlot::secondary(3, 0),
        0x4000,
        0x8000,
        MsxSlotDeviceKind::Firmware(FirmwareRegion::DiskRom),
    ),
    device(
        MsxSlot::secondary(3, 3),
        0x0000,
        FULL_SLOT,
        MsxSlotDeviceKind::MapperRam,
    ),
];
/// HB-F1XD slot layout.
const HBF1XD_LAYOUT: MsxSlotLayout = MsxSlotLayout::new(1 << 3, HBF1XD_DEVICES);

/// HB-F1XDJ firmware placement.
const HBF1XDJ_FIRMWARE: &[FirmwarePlacement] = &[
    firmware(
        FirmwareRegion::Bios,
        MsxSlot::secondary(0, 0),
        0x0000,
        0x8000,
        false,
    ),
    firmware(
        FirmwareRegion::FirmwareMapper,
        MsxSlot::secondary(0, 3),
        0x0000,
        FULL_SLOT,
        false,
    ),
    firmware(
        FirmwareRegion::SubRom,
        MsxSlot::secondary(3, 1),
        0x0000,
        0x4000,
        false,
    ),
    firmware(
        FirmwareRegion::KanjiDriver,
        MsxSlot::secondary(3, 1),
        0x4000,
        0x8000,
        false,
    ),
    firmware(
        FirmwareRegion::DiskRom,
        MsxSlot::secondary(3, 2),
        0x4000,
        0x4000,
        false,
    ),
    firmware(
        FirmwareRegion::MsxMusic,
        MsxSlot::secondary(3, 3),
        0x4000,
        0x4000,
        false,
    ),
];
/// HB-F1XDJ slot devices.
const HBF1XDJ_DEVICES: &[MsxSlotDevice] = &[
    device(
        MsxSlot::secondary(0, 0),
        0x0000,
        0x8000,
        MsxSlotDeviceKind::Firmware(FirmwareRegion::Bios),
    ),
    device(
        MsxSlot::secondary(0, 3),
        0x0000,
        FULL_SLOT,
        MsxSlotDeviceKind::SonyFirmwareMapper(FirmwareRegion::FirmwareMapper),
    ),
    device(
        MsxSlot::primary(1),
        0x0000,
        FULL_SLOT,
        MsxSlotDeviceKind::Cartridge(0),
    ),
    device(
        MsxSlot::primary(2),
        0x0000,
        FULL_SLOT,
        MsxSlotDeviceKind::Cartridge(1),
    ),
    device(
        MsxSlot::secondary(3, 0),
        0x0000,
        FULL_SLOT,
        MsxSlotDeviceKind::MapperRam,
    ),
    device(
        MsxSlot::secondary(3, 1),
        0x0000,
        0x4000,
        MsxSlotDeviceKind::Firmware(FirmwareRegion::SubRom),
    ),
    device(
        MsxSlot::secondary(3, 1),
        0x4000,
        0x8000,
        MsxSlotDeviceKind::Firmware(FirmwareRegion::KanjiDriver),
    ),
    device(
        MsxSlot::secondary(3, 2),
        0x4000,
        0x4000,
        MsxSlotDeviceKind::Firmware(FirmwareRegion::DiskRom),
    ),
    device(
        MsxSlot::secondary(3, 3),
        0x4000,
        0x4000,
        MsxSlotDeviceKind::Firmware(FirmwareRegion::MsxMusic),
    ),
];
/// HB-F1XDJ slot layout.
const HBF1XDJ_LAYOUT: MsxSlotLayout = MsxSlotLayout::new((1 << 0) | (1 << 3), HBF1XDJ_DEVICES);

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy)]
    struct ExpectedCapabilities {
        model: MsxModel,
        generation: MsxGeneration,
        video_processor: MsxVdpVersion,
        video_ram_size: usize,
        work_ram_size: usize,
        memory_mapper_size: Option<usize>,
        mapper_readback: Option<MapperReadback>,
        disk_controller: MsxDiskController,
        drive_count: u8,
        has_rtc: bool,
        kanji_rom_size: Option<usize>,
        has_s1985: bool,
        has_msx_music: bool,
        expanded_primary_mask: u8,
        device_count: usize,
        firmware_count: usize,
    }

    #[test]
    fn all_model_capabilities_match_the_model_matrix() {
        let expected_capabilities = [
            ExpectedCapabilities {
                model: MsxModel::Msx,
                generation: MsxGeneration::Msx1,
                video_processor: MsxVdpVersion::Tms9118,
                video_ram_size: 16 << 10,
                work_ram_size: 64 << 10,
                memory_mapper_size: None,
                mapper_readback: None,
                disk_controller: MsxDiskController::None,
                drive_count: 0,
                has_rtc: false,
                kanji_rom_size: None,
                has_s1985: false,
                has_msx_music: false,
                expanded_primary_mask: 0,
                device_count: 5,
                firmware_count: 2,
            },
            ExpectedCapabilities {
                model: MsxModel::Msx2,
                generation: MsxGeneration::Msx2,
                video_processor: MsxVdpVersion::V9938,
                video_ram_size: 128 << 10,
                work_ram_size: 512 << 10,
                memory_mapper_size: Some(512 << 10),
                mapper_readback: Some(MapperReadback {
                    mask: 0x1F,
                    fixed_bits: 0x80,
                }),
                disk_controller: MsxDiskController::SonyWd2793,
                drive_count: 1,
                has_rtc: true,
                kanji_rom_size: None,
                has_s1985: true,
                has_msx_music: false,
                expanded_primary_mask: 1 << 3,
                device_count: 6,
                firmware_count: 3,
            },
            ExpectedCapabilities {
                model: MsxModel::Msx2Plus,
                generation: MsxGeneration::Msx2Plus,
                video_processor: MsxVdpVersion::V9958,
                video_ram_size: 128 << 10,
                work_ram_size: 512 << 10,
                memory_mapper_size: Some(512 << 10),
                mapper_readback: Some(MapperReadback {
                    mask: 0x1F,
                    fixed_bits: 0x80,
                }),
                disk_controller: MsxDiskController::SonyWd2793,
                drive_count: 1,
                has_rtc: true,
                kanji_rom_size: Some(256 << 10),
                has_s1985: true,
                has_msx_music: true,
                expanded_primary_mask: (1 << 0) | (1 << 3),
                device_count: 9,
                firmware_count: 6,
            },
        ];

        assert_eq!(
            expected_capabilities.map(|capabilities| capabilities.model),
            MsxModel::ALL
        );

        for capabilities in expected_capabilities {
            let model = capabilities.model;
            assert_eq!(model.main_clock_hz(), 3_579_545);
            assert_eq!(model.work_ram_size(), capabilities.work_ram_size);
            assert_eq!(model.generation(), capabilities.generation);
            assert_eq!(model.vdp_version(), capabilities.video_processor);
            assert_eq!(model.vram_size(), capabilities.video_ram_size);
            assert_eq!(model.memory_mapper_size(), capabilities.memory_mapper_size);
            assert_eq!(model.mapper_readback(), capabilities.mapper_readback);
            assert_eq!(model.disk_controller(), capabilities.disk_controller);
            assert_eq!(model.drive_count(), capabilities.drive_count);
            assert_eq!(model.has_rtc(), capabilities.has_rtc);
            assert_eq!(model.kanji_rom_size(), capabilities.kanji_rom_size);
            assert_eq!(model.has_s1985(), capabilities.has_s1985);
            assert_eq!(model.has_msx_music(), capabilities.has_msx_music);
            for primary in 0..4 {
                assert_eq!(
                    model.slot_layout().primary_is_expanded(primary),
                    capabilities.expanded_primary_mask & (1 << primary) != 0
                );
            }
            assert_eq!(
                model.slot_layout().devices().len(),
                capabilities.device_count
            );
            assert_eq!(model.firmware_layout().len(), capabilities.firmware_count);
            assert!(model.slot_layout().validate().is_ok());
            assert_eq!(model.to_string().parse::<MsxModel>(), Ok(model));
        }
    }

    #[test]
    fn every_firmware_placement_has_a_matching_slot_device() {
        for model in MsxModel::ALL {
            for placement in model.firmware_layout() {
                let matching_device = model.slot_layout().devices().iter().any(|device| {
                    let matching_region = match device.kind {
                        MsxSlotDeviceKind::Firmware(region)
                        | MsxSlotDeviceKind::SonyFirmwareMapper(region) => {
                            region == placement.region
                        }
                        MsxSlotDeviceKind::PlainRam
                        | MsxSlotDeviceKind::MapperRam
                        | MsxSlotDeviceKind::Cartridge(_) => false,
                    };
                    device.slot == placement.slot
                        && device.address == placement.address
                        && device.size == placement.mapped_size
                        && matching_region
                });
                assert!(
                    matching_device,
                    "{model} has no slot device for {:?}",
                    placement.region
                );
            }
        }
    }

    #[test]
    fn sony_disk_rom_visibility_matches_each_model() {
        let msx2 = MsxModel::Msx2
            .firmware_layout()
            .iter()
            .find(|placement| placement.region == FirmwareRegion::DiskRom)
            .unwrap();
        assert_eq!(msx2.address, 0x4000);
        assert_eq!(msx2.mapped_size, 0x8000);
        assert!(msx2.mirrored);

        let msx2_plus = MsxModel::Msx2Plus
            .firmware_layout()
            .iter()
            .find(|placement| placement.region == FirmwareRegion::DiskRom)
            .unwrap();
        assert_eq!(msx2_plus.address, 0x4000);
        assert_eq!(msx2_plus.mapped_size, 0x4000);
        assert!(!msx2_plus.mirrored);
    }

    #[test]
    fn invalid_slot_layouts_are_rejected() {
        static OUT_OF_RANGE: &[MsxSlotDevice] = &[device(
            MsxSlot::primary(4),
            0,
            1,
            MsxSlotDeviceKind::PlainRam,
        )];
        static UNEXPANDED: &[MsxSlotDevice] = &[device(
            MsxSlot::secondary(0, 0),
            0,
            1,
            MsxSlotDeviceKind::PlainRam,
        )];
        static OVERLAP: &[MsxSlotDevice] = &[
            device(MsxSlot::primary(0), 0, 0x4000, MsxSlotDeviceKind::PlainRam),
            device(
                MsxSlot::primary(0),
                0x2000,
                0x4000,
                MsxSlotDeviceKind::PlainRam,
            ),
        ];
        static CROSS_END: &[MsxSlotDevice] = &[device(
            MsxSlot::primary(0),
            0xFFFF,
            2,
            MsxSlotDeviceKind::PlainRam,
        )];

        assert!(matches!(
            MsxSlotLayout::for_test(0, OUT_OF_RANGE).validate(),
            Err(SlotLayoutError::InvalidPrimary { .. })
        ));
        assert!(matches!(
            MsxSlotLayout::for_test(0, UNEXPANDED).validate(),
            Err(SlotLayoutError::PrimaryNotExpanded { .. })
        ));
        assert!(matches!(
            MsxSlotLayout::for_test(0, OVERLAP).validate(),
            Err(SlotLayoutError::Overlap { .. })
        ));
        assert!(matches!(
            MsxSlotLayout::for_test(0, CROSS_END).validate(),
            Err(SlotLayoutError::InvalidAddressRange { .. })
        ));
    }
}
