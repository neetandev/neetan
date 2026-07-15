//! Motorola 68000 CPU core ported from MAME's m68000 core by Olivier Galibert.
#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::unnecessary_wraps)]
#![cfg_attr(not(test), no_std)]

extern crate alloc;

mod m68000;

pub use common::{
    M68000AccessSize, M68000BusAccess, M68000BusError, M68000CycleKind, M68000FunctionCode,
};
pub use m68000::{
    M68000, M68000_DEFAULT_CLOCK_HZ, M68000BusCycle, M68000BusDirection, M68000BusSize,
    M68000Flags, M68000RuntimeState, M68000State,
};
