//! IBM PC/AT (CS4031, i486DX2) machine for DOS/V.
//!
//! This crate models a period-correct PC/AT clone built on the Chips &
//! Technologies CS4031 chipset with a Tseng ET4000AX SVGA card.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

mod bus;
mod cmos;
mod config;
mod machine;
mod memory;
mod rom;
mod scheduler;

pub use bus::{
    AT_KEY_CURSOR_DOWN, AT_KEY_CURSOR_LEFT, AT_KEY_CURSOR_RIGHT, AT_KEY_CURSOR_UP, AT_KEY_DELETE,
    AT_KEY_END, AT_KEY_HOME, AT_KEY_INSERT, AT_KEY_KEYPAD_DIVIDE, AT_KEY_KEYPAD_ENTER,
    AT_KEY_PAGE_DOWN, AT_KEY_PAGE_UP, AT_KEY_RIGHT_ALT, AT_KEY_RIGHT_CTRL, AtBus,
};
pub use config::{AtBootDevice, AtModel, ClockConfig, PIT_CLOCK_HZ};
pub use machine::AtMachine;
pub use rom::{LoadedRoms, RomError, load_rom_set};
