//! Content-addressed X68000 ROM-set loading.

use std::{fmt, path::Path};

use rom_loader::{DirectoryScanError, RomIndex, RomSlot, ScanOptions};

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

/// Shared character-generator ROM slot definition.
const CGROM_SLOT: RomSlot = RomSlot::new("CGROM", CGROM_SIZE, &[CGROM_DIGEST]);

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

impl From<DirectoryScanError> for RomError {
    fn from(error: DirectoryScanError) -> Self {
        Self::Read {
            directory: error.directory,
            message: error.message,
        }
    }
}

/// Loads the ROMs required by `model` from one non-recursive directory scan.
pub fn load_rom_set(model: X68kModel, rom_directory: &Path) -> Result<LoadedRoms, RomError> {
    let options = ScanOptions::sizes(&[CGROM_SIZE, IPL_SIZE, SPLIT_IPL_SIZE, SCSI_ROM_SIZE]);
    let index = rom_loader::scan_directory(rom_directory, &options)?;
    load_from_files(model, &index, &production_slots())
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
            ipl_even: Some(RomSlot::new(
                "X68000 IPL even half",
                SPLIT_IPL_SIZE,
                &[ORIGINAL_IPL_EVEN_DIGEST],
            )),
            ipl_odd: Some(RomSlot::new(
                "X68000 IPL odd half",
                SPLIT_IPL_SIZE,
                &[ORIGINAL_IPL_ODD_DIGEST],
            )),
            scsi: None,
            assembled_ipl_digest: ORIGINAL_IPL_DIGEST,
        },
        ModelSlots {
            cgrom: CGROM_SLOT,
            ipl: Some(RomSlot::new(
                "X68000 SUPER IPL",
                IPL_SIZE,
                &[SUPER_IPL_DIGEST],
            )),
            ipl_even: None,
            ipl_odd: None,
            scsi: Some(RomSlot::new(
                "X68000 SUPER internal SCSI",
                SCSI_ROM_SIZE,
                &[SUPER_SCSI_DIGEST],
            )),
            assembled_ipl_digest: SUPER_IPL_DIGEST,
        },
        ModelSlots {
            cgrom: CGROM_SLOT,
            ipl: Some(RomSlot::new("X68000 XVI IPL", IPL_SIZE, &[XVI_IPL_DIGEST])),
            ipl_even: None,
            ipl_odd: None,
            scsi: Some(RomSlot::new(
                "X68000 XVI compatibility SCSI",
                SCSI_ROM_SIZE,
                &[XVI_SCSI_DIGEST],
            )),
            assembled_ipl_digest: XVI_IPL_DIGEST,
        },
    ]
}

/// Loads a model from previously hashed files.
fn load_from_files(
    model: X68kModel,
    index: &RomIndex,
    all_slots: &[ModelSlots; 3],
) -> Result<LoadedRoms, RomError> {
    let slots = &all_slots[match model {
        X68kModel::X68000 => 0,
        X68kModel::X68000Super => 1,
        X68kModel::X68000Xvi => 2,
    }];
    let cgrom = take_slot(index, slots.cgrom)?;
    let ipl = if let Some(slot) = slots.ipl {
        take_slot(index, slot)?
    } else {
        let even = take_slot(index, slots.ipl_even.expect("original even IPL slot"))?;
        let odd = take_slot(index, slots.ipl_odd.expect("original odd IPL slot"))?;
        let mut interleaved = Vec::with_capacity(IPL_SIZE);
        for (even_byte, odd_byte) in even.into_iter().zip(odd) {
            interleaved.push(even_byte);
            interleaved.push(odd_byte);
        }
        let digest = rom_loader::blake3_hex(&interleaved);
        if digest != slots.assembled_ipl_digest {
            return Err(RomError::InvalidInterleave { digest });
        }
        interleaved
    };
    let internal_scsi = slots.scsi.map(|slot| take_slot(index, slot)).transpose()?;
    Ok(LoadedRoms {
        model,
        cgrom,
        ipl,
        internal_scsi,
        uses_compatibility_scsi: model == X68kModel::X68000Xvi,
    })
}

/// Takes the unique file matching one ROM slot.
fn take_slot(index: &RomIndex, slot: RomSlot) -> Result<Vec<u8>, RomError> {
    let digest = slot.accepted[0];
    match index.match_count(digest) {
        0 => Err(RomError::Missing {
            label: slot.label.to_string(),
            size: slot.size,
            digest: digest.to_string(),
        }),
        1 => Ok(index.bytes(digest).expect("matched image").to_vec()),
        _ => Err(RomError::Duplicate {
            label: slot.label.to_string(),
            digest: digest.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(label: &'static str, data: &[u8]) -> RomSlot {
        let digest: &'static str = Box::leak(rom_loader::blake3_hex(data).into_boxed_str());
        let accepted: &'static [&'static str] = Box::leak(vec![digest].into_boxed_slice());
        RomSlot::new(label, data.len(), accepted)
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
            assembled_ipl_digest: Box::leak(rom_loader::blake3_hex(&ipl).into_boxed_str()),
        };
        let index = RomIndex::from_images([cgrom.clone(), ipl.clone(), scsi.clone(), vec![9; 5]]);
        let all_slots = [
            ModelSlots {
                scsi: None,
                ..slots_for_clone(&slots)
            },
            slots_for_clone(&slots),
            slots_for_clone(&slots),
        ];
        let loaded = load_from_files(X68kModel::X68000Super, &index, &all_slots).unwrap();
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
        let index = RomIndex::from_images([data.clone(), data]);
        assert!(matches!(
            take_slot(&index, selected),
            Err(RomError::Duplicate { .. })
        ));
    }
}
