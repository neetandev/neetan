//! Content-addressed X68000 ROM-set loading.

use std::{collections::HashMap, fmt, path::Path};

use crate::X68kModel;

/// Required character-generator ROM size.
const CGROM_SIZE: usize = 0xC_0000;
/// Required assembled IPL ROM size.
const IPL_SIZE: usize = 0x2_0000;
/// Required size of each original-model split IPL half.
const SPLIT_IPL_SIZE: usize = 0x1_0000;
/// Required internal-SCSI boot-ROM size.
const SCSI_ROM_SIZE: usize = 0x2000;

/// BLAKE3 digest of the shared character-generator ROM.
const CGROM_DIGEST: &str = "095cfc5c21d704cce7340982b717dadc9fa20bfb86637ce9a594af88c87dc6b8";
/// BLAKE3 digest of the original IPL even-byte half.
const ORIGINAL_IPL_EVEN_DIGEST: &str =
    "50f6e84f88feb32e1cf2421ea6376fed44851c269f8bd48706c2e8061ceba313";
/// BLAKE3 digest of the original IPL odd-byte half.
const ORIGINAL_IPL_ODD_DIGEST: &str =
    "2bc789c7b172ebbe70d5099a9b8820653234e26cb5f7b4a171b4d73ee647ddaa";
/// BLAKE3 digest of the interleaved original IPL.
const ORIGINAL_IPL_DIGEST: &str =
    "fe7832b87d5bb5f8d56d9f1d697ef9bb94c446334e17105e574c8314b7602d32";
/// BLAKE3 digest of the X68000 SUPER IPL.
const SUPER_IPL_DIGEST: &str = "10ecab1df03426f4823de6cca28a26818b471b9ca20943441ba73c8fd0cd710f";
/// BLAKE3 digest of the X68000 SUPER internal-SCSI ROM.
const SUPER_SCSI_DIGEST: &str = "7ac5c8fa53d2693ee61ada293efd1f681b1390ef50c1117ddcf52d2280468c20";
/// BLAKE3 digest of the X68000 XVI IPL.
const XVI_IPL_DIGEST: &str = "06d3d6365d2b4079abf37d362a393f9224e472b8321e1826fef0a263d9e26590";
/// BLAKE3 digest of the XVI-compatible internal-SCSI ROM.
const XVI_SCSI_DIGEST: &str = "08e08002db7e47bdf6f2f60066f7253eb94791fb2aa17b392e26d23d72e0c19f";

#[derive(Clone, Copy)]
struct RomSlot {
    label: &'static str,
    size: usize,
    digest: &'static str,
}

/// Shared character-generator ROM slot definition.
const CGROM_SLOT: RomSlot = RomSlot {
    label: "CGROM",
    size: CGROM_SIZE,
    digest: CGROM_DIGEST,
};

/// Raw bytes of a validated X68000 ROM set.
#[derive(Debug, Clone)]
pub struct LoadedRoms {
    /// Selected model.
    pub model: X68kModel,
    /// Character-generator ROM, 768 KiB.
    pub cgrom: Vec<u8>,
    /// Interleaved IPL ROM, 128 KiB.
    pub ipl: Vec<u8>,
    /// Internal SCSI ROM, 8 KiB, when present on the selected model.
    pub internal_scsi: Option<Vec<u8>>,
    /// Whether the XVI compatibility SCSI image is in use.
    pub uses_compatibility_scsi: bool,
}

/// Error encountered while loading an X68000 ROM set.
#[derive(Debug, PartialEq, Eq)]
pub enum RomError {
    /// The ROM directory could not be scanned.
    Read {
        /// Directory that could not be scanned.
        directory: String,
        /// Underlying filesystem error.
        message: String,
    },
    /// A required ROM slot was not found.
    Missing {
        /// Human-readable slot name.
        label: String,
        /// Required byte length.
        size: usize,
        /// Accepted BLAKE3 digest.
        digest: String,
    },
    /// More than one file matched a required content slot.
    Duplicate {
        /// Human-readable slot name.
        label: String,
        /// Duplicated BLAKE3 digest.
        digest: String,
    },
    /// Split IPL halves produced unexpected content after interleaving.
    InvalidInterleave {
        /// BLAKE3 digest of the assembled image.
        digest: String,
    },
}

impl fmt::Display for RomError {
    /// Formats a ROM-loading error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { directory, message } => {
                write!(
                    formatter,
                    "failed to read ROM directory {directory}: {message}"
                )
            }
            Self::Missing {
                label,
                size,
                digest,
            } => write!(
                formatter,
                "missing {label} ROM ({size} bytes, BLAKE3 {digest})"
            ),
            Self::Duplicate { label, digest } => {
                write!(
                    formatter,
                    "multiple files matched {label} ROM (BLAKE3 {digest})"
                )
            }
            Self::InvalidInterleave { digest } => write!(
                formatter,
                "split X68000 IPL interleaved to unexpected BLAKE3 digest {digest}"
            ),
        }
    }
}

impl std::error::Error for RomError {}

/// Loads the ROMs required by `model` from one non-recursive directory scan.
pub fn load_rom_set(model: X68kModel, rom_directory: &Path) -> Result<LoadedRoms, RomError> {
    let files = hash_directory(rom_directory)?;
    load_from_files(model, &files, &production_slots())
}

struct ModelSlots {
    cgrom: RomSlot,
    ipl: Option<RomSlot>,
    ipl_even: Option<RomSlot>,
    ipl_odd: Option<RomSlot>,
    scsi: Option<RomSlot>,
    assembled_ipl_digest: &'static str,
}

/// Returns the production slot table for every model.
fn production_slots() -> [ModelSlots; 3] {
    [
        ModelSlots {
            cgrom: CGROM_SLOT,
            ipl: None,
            ipl_even: Some(RomSlot {
                label: "X68000 IPL even half",
                size: SPLIT_IPL_SIZE,
                digest: ORIGINAL_IPL_EVEN_DIGEST,
            }),
            ipl_odd: Some(RomSlot {
                label: "X68000 IPL odd half",
                size: SPLIT_IPL_SIZE,
                digest: ORIGINAL_IPL_ODD_DIGEST,
            }),
            scsi: None,
            assembled_ipl_digest: ORIGINAL_IPL_DIGEST,
        },
        ModelSlots {
            cgrom: CGROM_SLOT,
            ipl: Some(RomSlot {
                label: "X68000 SUPER IPL",
                size: IPL_SIZE,
                digest: SUPER_IPL_DIGEST,
            }),
            ipl_even: None,
            ipl_odd: None,
            scsi: Some(RomSlot {
                label: "X68000 SUPER internal SCSI",
                size: SCSI_ROM_SIZE,
                digest: SUPER_SCSI_DIGEST,
            }),
            assembled_ipl_digest: SUPER_IPL_DIGEST,
        },
        ModelSlots {
            cgrom: CGROM_SLOT,
            ipl: Some(RomSlot {
                label: "X68000 XVI IPL",
                size: IPL_SIZE,
                digest: XVI_IPL_DIGEST,
            }),
            ipl_even: None,
            ipl_odd: None,
            scsi: Some(RomSlot {
                label: "X68000 XVI compatibility SCSI",
                size: SCSI_ROM_SIZE,
                digest: XVI_SCSI_DIGEST,
            }),
            assembled_ipl_digest: XVI_IPL_DIGEST,
        },
    ]
}

/// Loads a model from previously hashed files.
fn load_from_files(
    model: X68kModel,
    files: &HashMap<String, Vec<Vec<u8>>>,
    all_slots: &[ModelSlots; 3],
) -> Result<LoadedRoms, RomError> {
    let slots = &all_slots[match model {
        X68kModel::X68000 => 0,
        X68kModel::X68000Super => 1,
        X68kModel::X68000Xvi => 2,
    }];
    let cgrom = take_slot(files, slots.cgrom)?;
    let ipl = if let Some(slot) = slots.ipl {
        take_slot(files, slot)?
    } else {
        let even = take_slot(files, slots.ipl_even.expect("original even IPL slot"))?;
        let odd = take_slot(files, slots.ipl_odd.expect("original odd IPL slot"))?;
        let mut interleaved = Vec::with_capacity(IPL_SIZE);
        for (even_byte, odd_byte) in even.into_iter().zip(odd) {
            interleaved.push(even_byte);
            interleaved.push(odd_byte);
        }
        let digest = blake3_hex(&interleaved);
        if digest != slots.assembled_ipl_digest {
            return Err(RomError::InvalidInterleave { digest });
        }
        interleaved
    };
    let internal_scsi = slots.scsi.map(|slot| take_slot(files, slot)).transpose()?;
    Ok(LoadedRoms {
        model,
        cgrom,
        ipl,
        internal_scsi,
        uses_compatibility_scsi: model == X68kModel::X68000Xvi,
    })
}

/// Takes the unique file matching one ROM slot.
fn take_slot(files: &HashMap<String, Vec<Vec<u8>>>, slot: RomSlot) -> Result<Vec<u8>, RomError> {
    match files.get(slot.digest) {
        None => Err(RomError::Missing {
            label: slot.label.to_string(),
            size: slot.size,
            digest: slot.digest.to_string(),
        }),
        Some(matches) if matches.len() > 1 => Err(RomError::Duplicate {
            label: slot.label.to_string(),
            digest: slot.digest.to_string(),
        }),
        Some(matches) => Ok(matches[0].clone()),
    }
}

/// Hashes candidate files in one directory.
fn hash_directory(directory: &Path) -> Result<HashMap<String, Vec<Vec<u8>>>, RomError> {
    let entries = std::fs::read_dir(directory).map_err(|error| RomError::Read {
        directory: directory.display().to_string(),
        message: error.to_string(),
    })?;
    let mut files: HashMap<String, Vec<Vec<u8>>> = HashMap::new();
    for entry in entries {
        let entry = entry.map_err(|error| RomError::Read {
            directory: directory.display().to_string(),
            message: error.to_string(),
        })?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Ok(data) = std::fs::read(path) else {
            continue;
        };
        if !matches!(
            data.len(),
            CGROM_SIZE | IPL_SIZE | SPLIT_IPL_SIZE | SCSI_ROM_SIZE
        ) {
            continue;
        }
        files.entry(blake3_hex(&data)).or_default().push(data);
    }
    Ok(files)
}

/// Formats a BLAKE3 digest as lowercase hexadecimal.
fn blake3_hex(data: &[u8]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(data);
    let mut digest = [0; 32];
    hasher.finalize(&mut digest);
    /// Lowercase hexadecimal digits used to format BLAKE3 digests.
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(64);
    for byte in digest {
        text.push(HEX[(byte >> 4) as usize] as char);
        text.push(HEX[(byte & 15) as usize] as char);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(label: &'static str, data: &[u8]) -> RomSlot {
        RomSlot {
            label,
            size: data.len(),
            digest: Box::leak(blake3_hex(data).into_boxed_str()),
        }
    }

    #[test]
    fn synthetic_super_set_ignores_unrelated_files() {
        let cgrom = vec![1; 8];
        let ipl = vec![2; 8];
        let scsi = vec![3; 4];
        let slots = ModelSlots {
            cgrom: slot("cg", &cgrom),
            ipl: Some(slot("ipl", &ipl)),
            ipl_even: None,
            ipl_odd: None,
            scsi: Some(slot("scsi", &scsi)),
            assembled_ipl_digest: Box::leak(blake3_hex(&ipl).into_boxed_str()),
        };
        let mut files = HashMap::new();
        for data in [&cgrom, &ipl, &scsi, &vec![9; 5]] {
            files.insert(blake3_hex(data), vec![data.clone()]);
        }
        let all_slots = [
            ModelSlots {
                scsi: None,
                ..slots_for_clone(&slots)
            },
            slots_for_clone(&slots),
            slots_for_clone(&slots),
        ];
        let loaded = load_from_files(X68kModel::X68000Super, &files, &all_slots).unwrap();
        assert_eq!(loaded.cgrom, cgrom);
        assert_eq!(loaded.ipl, ipl);
        assert_eq!(loaded.internal_scsi, Some(scsi));
    }

    fn slots_for_clone(slots: &ModelSlots) -> ModelSlots {
        ModelSlots {
            cgrom: slots.cgrom,
            ipl: slots.ipl,
            ipl_even: slots.ipl_even,
            ipl_odd: slots.ipl_odd,
            scsi: slots.scsi,
            assembled_ipl_digest: slots.assembled_ipl_digest,
        }
    }

    #[test]
    fn duplicate_slot_is_rejected() {
        let data = vec![1; 8];
        let selected = slot("test", &data);
        let files = HashMap::from([(selected.digest.to_string(), vec![data.clone(), data])]);
        assert!(matches!(
            take_slot(&files, selected),
            Err(RomError::Duplicate { .. })
        ));
    }
}
