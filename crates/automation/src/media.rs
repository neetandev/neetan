//! Session-side media model for the automation frontend.
//!
//! Writable floppy and hard-disk fixtures are never mounted by their baseline
//! paths. The session reads the source bytes and mounts them with a RAM backing
//! so guest writes stay in memory and the on-disk fixture is left byte-identical.
//! Read-only media (CD-ROM, cartridge, cassette) is mounted from its source path
//! and never written back. Printer output is the one writable host artifact.

use std::path::PathBuf;

/// A kind of media slot exposed to scripts.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum MediaKind {
    /// A removable floppy disk (two drives).
    Floppy,
    /// A fixed hard disk (two drives).
    Hdd,
    /// A CD-ROM disc.
    Cdrom,
    /// A cartridge.
    Cartridge,
    /// A cassette tape.
    Cassette,
    /// The printer output path.
    Printer,
}

impl MediaKind {
    /// Returns the number of addressable slots for this kind.
    #[must_use]
    pub fn slot_count(self) -> usize {
        match self {
            MediaKind::Floppy | MediaKind::Hdd => 2,
            MediaKind::Cdrom | MediaKind::Cartridge | MediaKind::Cassette | MediaKind::Printer => 1,
        }
    }

    /// Returns whether the trait exposes an eject operation for this kind.
    ///
    /// There is no `eject_hdd` and no printer-detach in the `Machine` trait, so
    /// those two kinds cannot be ejected.
    #[must_use]
    pub fn supports_eject(self) -> bool {
        match self {
            MediaKind::Floppy | MediaKind::Cdrom | MediaKind::Cartridge | MediaKind::Cassette => {
                true
            }
            MediaKind::Hdd | MediaKind::Printer => false,
        }
    }

    /// Returns whether the guest can write to this kind under automation.
    #[must_use]
    pub fn writable(self) -> bool {
        matches!(self, MediaKind::Floppy | MediaKind::Hdd)
    }

    /// Returns the stable symbol name for this kind.
    #[must_use]
    pub fn symbol(self) -> &'static str {
        match self {
            MediaKind::Floppy => "floppy",
            MediaKind::Hdd => "hdd",
            MediaKind::Cdrom => "cdrom",
            MediaKind::Cartridge => "cartridge",
            MediaKind::Cassette => "cassette",
            MediaKind::Printer => "printer",
        }
    }
}

/// Resolves a media type symbol, including the accepted aliases, to a kind.
#[must_use]
pub fn media_kind_from_name(name: &str) -> Option<MediaKind> {
    match name {
        "floppy" | "fdd" => Some(MediaKind::Floppy),
        "hdd" | "hard-disk" => Some(MediaKind::Hdd),
        "cdrom" | "cd" => Some(MediaKind::Cdrom),
        "cartridge" | "cart" => Some(MediaKind::Cartridge),
        "cassette" | "tape" => Some(MediaKind::Cassette),
        "printer" => Some(MediaKind::Printer),
        _ => None,
    }
}

/// A declared or runtime request to mount media into a slot.
#[derive(Clone, Debug)]
pub struct MediaRequest {
    /// The kind of media to mount.
    pub kind: MediaKind,
    /// The zero-based slot.
    pub slot: usize,
    /// The source path as requested, resolved beneath the read or write root.
    pub source: String,
}

/// A currently mounted media entry, reported by `media-info`.
#[derive(Clone, Debug)]
pub struct MediaMount {
    /// The kind of the mounted media.
    pub kind: MediaKind,
    /// The zero-based slot.
    pub slot: usize,
    /// The original path string as requested by the script or specification.
    pub requested: String,
    /// A media format label derived from the source file extension.
    pub format: String,
    /// The machine's human-readable description of the mounted image.
    pub description: String,
    /// Whether guest writes to this mount are dropped.
    pub write_protected: bool,
    /// Whether the guest may have written since the last flush.
    pub dirty: bool,
    /// The printer artifact path for the printer kind, otherwise `None`.
    pub printer_artifact: Option<PathBuf>,
}
