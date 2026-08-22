//! PC-8801 emulation: a cycle-driven dual-Z80 machine.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

extern crate alloc;

mod bus;
mod config;
mod machine;
mod memory;
mod rom;
mod scheduler;

pub use bus::Pc8801Bus;
pub use common::{EightMhzWaitMode, MemoryWaitSwitch};
pub use config::{BootMode, ClockConfig, ClockSelect, Pc8801Model};
pub use machine::{Pc8801Machine, build_automated_machine, build_untraced_machine};
pub use rom::{LoadedRoms, load_rom_set};
pub use rom_loader::RomError;
