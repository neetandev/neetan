//! PC-8801 emulation: a cycle-driven dual-Z80 machine.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

mod bus;
mod config;
mod machine;
mod memory;
mod rom;
mod scheduler;

pub use bus::Pc8801Bus;
pub use config::{
    BootMode, ClockConfig, ClockSelect, EightMhzWaitMode, MemoryWaitSwitch, MonitorTiming,
    Pc8801Model,
};
pub use machine::Pc8801Machine;
pub use memory::{GvramSelect, Pc8801MemoryState};
pub use rom::{LoadedRoms, RomError, load_rom_set};
pub use scheduler::{Event88, Pc8801Scheduler, Pc8801SchedulerState, ScheduledEvent88};
