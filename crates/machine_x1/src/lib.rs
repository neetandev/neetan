//! Sharp X1 / X1 turbo emulation.

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

pub use bus::X1Bus;
pub use config::{ClockConfig, X1KeyboardMode, X1Model};
pub use machine::{X1Machine, build_automated_machine, build_untraced_machine};
pub use rom::{LoadedRoms, RomError, load_rom_set};
