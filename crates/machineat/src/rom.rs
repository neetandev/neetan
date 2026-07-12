//! PC/AT ROM set loading.
//!
//! Like the other machines, ROMs are matched by BLAKE3 content hash rather
//! than file name: the loader scans every file in the ROM directory (and its
//! immediate subdirectories, so the `ct486` and `et4000` sets can sit side by
//! side) and picks the ones whose digests match the accepted images. The
//! machine needs the AMI CS4031 system BIOS (`chips_1.ami`) and the ET4000AX
//! VGA BIOS (`et4000.bin`, or the alternate `cvet4kax.bin`).

use std::{collections::HashMap, fmt, path::Path};

/// System BIOS ROM size in bytes.
const SYSTEM_BIOS_SIZE: usize = 0x1_0000;
/// VGA BIOS ROM size in bytes.
const VGA_BIOS_SIZE: usize = 0x8000;

/// File sizes of known ROM images; other files are skipped while scanning.
const KNOWN_ROM_SIZES: &[usize] = &[SYSTEM_BIOS_SIZE, VGA_BIOS_SIZE];

/// Accepted BLAKE3 digest for the AMI CS4031 system BIOS (`chips_1.ami`).
const SYSTEM_BIOS_DIGESTS: &[&str] =
    &["bcd8d7424756ca90c5853ac24c2a7f3621d5ff1f6f7a170027e0fbe2b10fd6f1"];

/// Accepted BLAKE3 digests for the ET4000AX VGA BIOS: `et4000.bin` and the
/// alternate ColorImage `cvet4kax.bin`.
const VGA_BIOS_DIGESTS: &[&str] = &[
    "24a00c4924d76f1bbb9afb554ff49009d004212d269bd7665fdfda9084bf3ec6",
    "5e5b51a62a2f5f20a09eb3dc6275d7c4b8af6ad2c9c67c02fc09150e974c4e23",
];

/// One ROM slot: its label and accepted content digests.
struct RomSlot {
    label: &'static str,
    accepted: &'static [&'static str],
}

const SYSTEM_BIOS_SLOT: RomSlot = RomSlot {
    label: "system-bios",
    accepted: SYSTEM_BIOS_DIGESTS,
};

const VGA_BIOS_SLOT: RomSlot = RomSlot {
    label: "vga-bios",
    accepted: VGA_BIOS_DIGESTS,
};

/// Raw bytes of a successfully loaded and validated PC/AT ROM set.
#[derive(Debug)]
pub struct LoadedRoms {
    /// System BIOS ROM (64 KiB), mapped at 0xF0000 and the 4 GiB-top alias.
    pub system_bios: Vec<u8>,
    /// ET4000AX VGA BIOS ROM (32 KiB), mapped at 0xC0000.
    pub vga_bios: Vec<u8>,
}

/// Error encountered while loading a PC/AT ROM set.
#[derive(Debug)]
pub enum RomError {
    /// The ROM directory could not be scanned.
    Read {
        /// The directory that failed to read.
        directory: String,
        /// The underlying error message.
        message: String,
    },
    /// No candidate image matched a slot's accepted digests.
    Missing {
        /// The ROM slot label.
        label: String,
        /// The accepted digests for that slot.
        accepted: Vec<String>,
    },
}

impl fmt::Display for RomError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RomError::Read { directory, message } => {
                write!(
                    formatter,
                    "failed to read ROM directory {directory}: {message}"
                )
            }
            RomError::Missing { label, accepted } => write!(
                formatter,
                "no ROM matched the {label} slot (accepted digests: {})",
                accepted.join(", ")
            ),
        }
    }
}

impl std::error::Error for RomError {}

/// Loads and validates the PC/AT ROM set from `rom_dir`.
///
/// Every regular file in the directory and its immediate subdirectories is
/// scanned; file names do not matter. Both slots are required.
pub fn load_rom_set(rom_dir: &Path) -> Result<LoadedRoms, RomError> {
    let mut by_digest = HashMap::new();
    hash_directory(rom_dir, &mut by_digest, true)?;

    let take = |slot: &RomSlot| -> Result<Vec<u8>, RomError> {
        for digest in slot.accepted {
            if let Some(data) = by_digest.get(*digest) {
                return Ok(data.clone());
            }
        }
        Err(RomError::Missing {
            label: slot.label.to_string(),
            accepted: slot.accepted.iter().map(|d| d.to_string()).collect(),
        })
    };

    Ok(LoadedRoms {
        system_bios: take(&SYSTEM_BIOS_SLOT)?,
        vga_bios: take(&VGA_BIOS_SLOT)?,
    })
}

/// Reads every regular file in `dir` and maps each candidate image's BLAKE3
/// digest to its bytes, keeping only files of a known ROM size. Descends one
/// level into subdirectories when `recurse` is set.
fn hash_directory(
    dir: &Path,
    by_digest: &mut HashMap<String, Vec<u8>>,
    recurse: bool,
) -> Result<(), RomError> {
    let entries = std::fs::read_dir(dir).map_err(|error| RomError::Read {
        directory: dir.display().to_string(),
        message: error.to_string(),
    })?;

    for entry in entries {
        let entry = entry.map_err(|error| RomError::Read {
            directory: dir.display().to_string(),
            message: error.to_string(),
        })?;
        let path = entry.path();
        if path.is_dir() {
            if recurse {
                hash_directory(&path, by_digest, false)?;
            }
            continue;
        }
        if !path.is_file() {
            continue;
        }
        let data = match std::fs::read(&path) {
            Ok(data) => data,
            Err(_) => continue,
        };
        if !KNOWN_ROM_SIZES.contains(&data.len()) {
            continue;
        }
        by_digest.entry(blake3_hex(&data)).or_insert(data);
    }

    Ok(())
}

/// Returns the lowercase hexadecimal BLAKE3 digest of `data`.
fn blake3_hex(data: &[u8]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(data);
    let mut digest = [0u8; 32];
    hasher.finalize(&mut digest);

    const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut hex = String::with_capacity(64);
    for byte in digest {
        hex.push(HEX_DIGITS[(byte >> 4) as usize] as char);
        hex.push(HEX_DIGITS[(byte & 0x0F) as usize] as char);
    }
    hex
}
