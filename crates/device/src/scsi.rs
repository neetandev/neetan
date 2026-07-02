//! SCSI emulation.
//!
//! A machine-agnostic SCSI command core ([`command`]) and a direct-access disk
//! target ([`disk`]) sit below a per-machine host adapter front-end. The FM
//! Towns MB89352-class SPC front-end is [`towns_spc`]; a different machine (for
//! example a PC-98 WD33C93) could reuse the command core under its own
//! front-end.

pub mod command;
pub mod disk;
pub mod towns_spc;

pub use disk::ScsiDisk;
pub use towns_spc::{Phase, ScsiDmaRequest, TownsScsiController};
