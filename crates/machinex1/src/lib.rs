//! Sharp X1 / X1 turbo emulation.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

mod bus;
mod config;
mod interrupt;
mod machine;
mod memory;
mod rom;
mod scheduler;

pub use bus::X1Bus;
pub use common::MonitorTiming;
pub use config::{ClockConfig, X1KeyboardMode, X1Model};
pub use machine::X1Machine;
pub use rom::{LoadedRoms, RomError, load_rom_set};
pub use scheduler::{EventX1, X1Scheduler};
