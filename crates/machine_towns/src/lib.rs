//! FM Towns machine family.
//!
//! # Acknowledgement
//!
//! This crate relied heavily on the hardware information and errata provided by the Tsugaru project
//! for its implementation.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

extern crate alloc;

mod bus;
mod config;
mod machine;
mod memory;
mod rom;
mod scheduler;

pub use bus::TownsBus;
pub use config::{ClockConfig, TownsBootDevice, TownsModel, TownsPadType};
pub use machine::{TownsMachine, build_automated_machine, build_untraced_machine};
pub use rom::{LoadedRoms, RomError, load_rom_set};
