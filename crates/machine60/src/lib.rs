//! PC-6000 / PC-6600 series emulation.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

mod bus;
mod config;
mod interrupt;
mod machine;
mod memory;
mod rom;
mod scheduler;

pub use bus::Pc6000Bus;
pub use config::{ClockConfig, Pc6000Model};
pub use machine::Pc6000Machine;
pub use rom::{LoadedRoms, RomError, load_rom_set};
pub use scheduler::{Event60, Pc6000Scheduler};
