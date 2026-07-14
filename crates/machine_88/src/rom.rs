//! PC-8801MC ROM set loading.
//!
//! ROMs are selected by content hash rather than file name: the loader scans
//! every file in the ROM directory, computes its BLAKE3 digest, and matches it
//! against a table of accepted digests per slot. This way any dump layout works
//! regardless of how the files are named, and stray files are ignored.

use std::{collections::HashMap, fmt, path::Path};

use crate::config::BootMode;

const N88_ROM_SIZE: usize = 0x8000;
const N88_EXT_ROM_SIZE: usize = 0x2000;
const N88_EXT_ROM_COUNT: usize = 4;
const N_BASIC_ROM_SIZE: usize = 0x8000;
const N80_ROM_SIZE: usize = 0x8000;
const N80SR_ROM_SIZE: usize = 0xA000;
const DICTIONARY_ROM_SIZE: usize = 0x8_0000;
const KANJI_ROM_SIZE: usize = 0x2_0000;
const DISK_ROM_SIZE: usize = 0x2000;
const CDROM_BIOS_ROM_SIZE: usize = 0x1_0000;

/// One ROM slot: its human label, expected size, and the BLAKE3 digests that
/// are accepted as valid content for it. Multiple digests allow several known
/// good dumps to satisfy the same slot.
struct RomSlot {
    label: &'static str,
    size: usize,
    accepted: &'static [&'static str],
}

const N88_SLOT: RomSlot = RomSlot {
    label: "n88",
    size: N88_ROM_SIZE,
    accepted: &["40457b507b82dd57cce0fcecf6bc65543a60bd46558ca947b0f69dd3658cdad8"],
};
const N88_EXT_SLOTS: [RomSlot; N88_EXT_ROM_COUNT] = [
    RomSlot {
        label: "n88_ext0",
        size: N88_EXT_ROM_SIZE,
        accepted: &["6a50a88231062ec871c65f63266fa7062a303ab870aed81c49f1f333f594a518"],
    },
    RomSlot {
        label: "n88_ext1",
        size: N88_EXT_ROM_SIZE,
        accepted: &["d5583fcce4eabf078d17666a1fddefa6a0d8bdc7f56d4499d526818728777252"],
    },
    RomSlot {
        label: "n88_ext2",
        size: N88_EXT_ROM_SIZE,
        accepted: &["ca200799765cb02a001bd55215b0daaf6d0593118a05e8d85754bddd92e5e8f7"],
    },
    RomSlot {
        label: "n88_ext3",
        size: N88_EXT_ROM_SIZE,
        accepted: &["ac31c1fbabfada9890669bebd471d60fac0be0e88ddfde81f17c600d5b0a1757"],
    },
];
const N_BASIC_SLOT: RomSlot = RomSlot {
    label: "n_basic",
    size: N_BASIC_ROM_SIZE,
    accepted: &["652eacc1ed6073bc3da1856c9c4f74ac14abef3f966f0d0fc89c40386de3d1a1"],
};
const N80_MKII_SLOT: RomSlot = RomSlot {
    label: "n80_mkii",
    size: N80_ROM_SIZE,
    accepted: &["9e4ec9c53f4432a88583dccd04ae3186f4d7849f80ea7774ac1efbdb93c992f2"],
};
const N80_MKIISR_SLOT: RomSlot = RomSlot {
    label: "n80_mkiisr",
    size: N80_ROM_SIZE,
    accepted: &["56406a79fd664a197c458cb3feeeb6994c34266a1e02728877b6ea5ef86e15ba"],
};
const N80SR_SLOT: RomSlot = RomSlot {
    label: "n80sr",
    size: N80SR_ROM_SIZE,
    accepted: &["7b81e27b831ad00f264170d1d98c645298fa688b07d5a9f0c19c1d6a73fe4273"],
};
const DICTIONARY_SLOT: RomSlot = RomSlot {
    label: "jisyo",
    size: DICTIONARY_ROM_SIZE,
    accepted: &["283dcd1c4a69f8049d19021d34d1cc2094f10de8b4e1ddf85da6a4b258dd8d12"],
};
const KANJI1_SLOT: RomSlot = RomSlot {
    label: "kanji1",
    size: KANJI_ROM_SIZE,
    accepted: &["10fd26424ae9e28be721846491d2d7b10e946da2d2ff39542248e819bc2339ba"],
};
const KANJI2_SLOT: RomSlot = RomSlot {
    label: "kanji2",
    size: KANJI_ROM_SIZE,
    accepted: &["f528e78bbe43e3d36c3def6ef30140e22ba9e69f422736605c2c4570c7d3fbe7"],
};
const DISK_SLOT: RomSlot = RomSlot {
    label: "disk",
    size: DISK_ROM_SIZE,
    accepted: &["081d2ca8ad7066de207b7360e45b5d6f3bab01769aefb9057141becbbaec5aa5"],
};
const CDROM_BIOS_SLOT: RomSlot = RomSlot {
    label: "cdbios",
    size: CDROM_BIOS_ROM_SIZE,
    accepted: &["de4d49437344806850b22356f9e5537e413e6113902fb8fbc803f902a5728827"],
};

/// Raw bytes of a successfully loaded and validated PC-8801MC ROM set.
pub struct LoadedRoms {
    /// N88-BASIC main ROM (32 KiB).
    pub n88: Vec<u8>,
    /// N88 extension ROM banks (4 x 8 KiB).
    pub n88_ext: [Vec<u8>; N88_EXT_ROM_COUNT],
    /// N-BASIC ROM (PC-8001, 1979) (32 KiB).
    pub n_basic: Vec<u8>,
    /// PC-8001mkII N80-BASIC ROM (32 KiB), required by boot-mode=n80.
    pub n80_mkii: Option<Vec<u8>>,
    /// PC-8001mkIISR N80-BASIC ROM (32 KiB), required by boot-mode=n80sr.
    pub n80_mkiisr: Option<Vec<u8>>,
    /// PC-8001mkIISR N80SR-BASIC ROM (40 KiB), required by boot-mode=n80sr.
    pub n80sr: Option<Vec<u8>>,
    /// Dictionary (jisyo) ROM (512 KiB).
    pub dictionary: Vec<u8>,
    /// Level-1 kanji ROM (128 KiB).
    pub kanji1: Vec<u8>,
    /// MC level-2 kanji ROM (128 KiB).
    pub kanji2: Vec<u8>,
    /// Disk sub-CPU (PC80S31K) ROM (8 KiB).
    pub disk: Vec<u8>,
    /// CD-ROM BIOS ROM (64 KiB) for the PC-8801-31 CD-ROM interface.
    pub cdrom_bios: Vec<u8>,
}

/// Error encountered while loading a PC-8801MC ROM set.
#[derive(Debug)]
pub enum RomError {
    /// The ROM directory could not be scanned.
    Read {
        /// The directory that failed to read.
        directory: String,
        /// The underlying error message.
        message: String,
    },
    /// No file in the directory matched a slot's accepted digests.
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
                "no ROM in the directory matched the {label} slot (accepted digests: {})",
                accepted.join(", ")
            ),
        }
    }
}

impl std::error::Error for RomError {}

impl LoadedRoms {
    /// Verifies that the ROMs a given boot mode depends on are present. The plain
    /// N-BASIC ROM is always loaded; the PC-8001mkII/mkIISR personalities, however,
    /// need their own N80 ROMs and must not silently fall back to N-BASIC.
    pub fn validate_for_boot_mode(&self, boot_mode: BootMode) -> Result<(), RomError> {
        match boot_mode {
            BootMode::N80 => {
                if self.n80_mkii.is_none() {
                    return Err(missing_rom(&N80_MKII_SLOT));
                }
            }
            BootMode::N80SR => {
                if self.n80sr.is_none() {
                    return Err(missing_rom(&N80SR_SLOT));
                }
                if self.n80_mkiisr.is_none() {
                    return Err(missing_rom(&N80_MKIISR_SLOT));
                }
            }
            BootMode::N | BootMode::V1S | BootMode::V1H | BootMode::V2 => {}
        }
        Ok(())
    }
}

fn missing_rom(slot: &RomSlot) -> RomError {
    RomError::Missing {
        label: slot.label.to_string(),
        accepted: slot.accepted.iter().map(|d| d.to_string()).collect(),
    }
}

/// Loads and validates the PC-8801MC ROM set.
///
/// Every file in `rom_dir` is hashed and matched against the accepted digests
/// for each ROM slot, so the dump's file names do not matter. The MC set covers
/// the N88, extension, N80, dictionary, disk sub-CPU, CD-ROM BIOS, and both
/// kanji ROMs.
pub fn load_rom_set(rom_dir: &Path) -> Result<LoadedRoms, RomError> {
    let by_digest = hash_directory(rom_dir)?;

    let take = |slot: &RomSlot| -> Result<Vec<u8>, RomError> {
        for digest in slot.accepted {
            if let Some(data) = by_digest.get(*digest) {
                return Ok(data.clone());
            }
        }
        Err(missing_rom(slot))
    };
    let take_optional = |slot: &RomSlot| -> Option<Vec<u8>> {
        for digest in slot.accepted {
            if let Some(data) = by_digest.get(*digest) {
                return Some(data.clone());
            }
        }
        None
    };

    let n88 = take(&N88_SLOT)?;
    let n88_ext = [
        take(&N88_EXT_SLOTS[0])?,
        take(&N88_EXT_SLOTS[1])?,
        take(&N88_EXT_SLOTS[2])?,
        take(&N88_EXT_SLOTS[3])?,
    ];
    let n_basic = take(&N_BASIC_SLOT)?;
    let n80_mkii = take_optional(&N80_MKII_SLOT);
    let n80_mkiisr = take_optional(&N80_MKIISR_SLOT);
    let n80sr = take_optional(&N80SR_SLOT);
    let dictionary = take(&DICTIONARY_SLOT)?;
    let kanji1 = take(&KANJI1_SLOT)?;
    let kanji2 = take(&KANJI2_SLOT)?;
    let disk = take(&DISK_SLOT)?;
    let cdrom_bios = take(&CDROM_BIOS_SLOT)?;

    Ok(LoadedRoms {
        n88,
        n88_ext,
        n_basic,
        n80_mkii,
        n80_mkiisr,
        n80sr,
        dictionary,
        kanji1,
        kanji2,
        disk,
        cdrom_bios,
    })
}

/// Reads every regular file in `dir` whose size matches a known ROM slot and
/// maps its BLAKE3 digest to its contents.
fn hash_directory(dir: &Path) -> Result<HashMap<String, Vec<u8>>, RomError> {
    let entries = std::fs::read_dir(dir).map_err(|error| RomError::Read {
        directory: dir.display().to_string(),
        message: error.to_string(),
    })?;

    let mut by_digest = HashMap::new();
    for entry in entries {
        let entry = entry.map_err(|error| RomError::Read {
            directory: dir.display().to_string(),
            message: error.to_string(),
        })?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let data = match std::fs::read(&path) {
            Ok(data) => data,
            Err(_) => continue,
        };
        if !is_known_rom_size(data.len()) {
            continue;
        }
        by_digest.entry(blake3_hex(&data)).or_insert(data);
    }
    Ok(by_digest)
}

fn is_known_rom_size(size: usize) -> bool {
    [
        N88_SLOT.size,
        N88_EXT_SLOTS[0].size,
        N_BASIC_SLOT.size,
        N80SR_SLOT.size,
        DICTIONARY_SLOT.size,
        KANJI1_SLOT.size,
        KANJI2_SLOT.size,
        DISK_SLOT.size,
        CDROM_BIOS_SLOT.size,
    ]
    .contains(&size)
}

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
