//! PC-88VA2 emulation: a NEC V30 machine with a custom graphics chipset.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

mod bus;
mod config;
mod machine;
mod memory;
mod rom;
mod scheduler;

pub use bus::Pc88VaBus;
pub use config::{ClockConfig, Pc88VaModel};
pub use machine::{Pc88VaMachine, reset_cpu};
pub use rom::{LoadedRoms, RomError, load_rom_set};
