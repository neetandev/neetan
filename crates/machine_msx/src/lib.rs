//! MSX, MSX2 and MSX2+ emulation.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

extern crate alloc;

mod bus;
mod cartridge;
mod cassette;
mod clock;
mod config;
mod machine;
mod memory;
mod rom;
mod scheduler;

pub use bus::{MainBusView, MsxBus, MsxControllerDevice, MsxJoystickState, SyntheticProgramError};
pub use cartridge::{
    CartridgeError, CartridgeLoadInfo, CartridgeMapper, CartridgePersistence, MapperIdentification,
    save_path_for_rom, sound_cartridge_for_disk_blake3,
};
pub use cassette::{MsxCassetteError, load_msx_cassette};
pub use config::{
    FirmwarePlacement, FirmwareRegion, MapperReadback, MsxClockProfile, MsxDiskController,
    MsxGeneration, MsxKeyboardLayout, MsxModel, MsxSlot, MsxSlotDevice, MsxSlotDeviceKind,
    MsxSlotLayout, MsxVdpVersion, SlotLayoutError,
};
pub use machine::{MsxMachine, build_untraced_machine};
pub use memory::FirmwareInstallError;
pub use rom::{FirmwareError, LoadedFirmware, LoadedFirmwareRegion, load_firmware_set};
