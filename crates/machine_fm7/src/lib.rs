//! Fujitsu FM-7 / FM-77AV emulation.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

extern crate alloc;

mod bus;
mod config;
mod interrupt;
mod machine;
mod memory;
mod rom;
mod scheduler;

pub use bus::{Fm7Bus, MainBusView, SubBusView};
pub use config::{BootMode, ClockConfig, Fm7Model};
pub use machine::Fm7Machine;
pub use rom::{LoadedRoms, RomError, load_rom_set};
