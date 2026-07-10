//! Sharp X68000 family machine implementation.
//!
//! # Acknowledgement
//!
//! This crate relied heavily on the hardware information and timings provided by the XEiJ project
//! for its implementation.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod bus;
mod clock;
mod interrupt;
mod machine;
mod model;
mod rom;
mod scheduler;
mod sram;

pub use bus::{X68K_DEFAULT_MAIN_RAM_SIZE, X68K_MAX_MAIN_RAM_SIZE, X68kBus, X68kRegion};
pub use interrupt::{InterruptSource, IocSource};
pub use machine::X68kMachine;
pub use model::{X68kModel, X68kStorageController, X68kVideoController};
pub use rom::{LoadedRoms, RomError, load_rom_set};
pub use sram::{
    BOOT_DEVICE_OFFSET, ROM_BOOT_HANDLE_OFFSET, SASI_HDMAX_OFFSET, SRAM_SIZE, initial_sram,
};
