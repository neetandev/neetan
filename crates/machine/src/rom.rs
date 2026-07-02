//! PC-98 ROM set loading.
//!
//! ROMs are selected by content hash rather than file name: the loader scans
//! every file in the ROM directory, computes its BLAKE3 digest, and matches it
//! against a table of accepted digests per slot. Any dump layout works
//! regardless of how the files are named, and stray files are ignored.
//!
//! Every slot is optional at this layer: the loader returns whatever it finds
//! and leaves the decision of which ROMs are required to the caller, which
//! depends on the machine model and whether real-BIOS mode is enabled.

use std::{collections::HashMap, fmt, path::Path};

use common::MachineModel;

/// Dual-bank BIOS image size (ITF bank + BIOS bank), 192 KB.
const BIOS_ROM_SIZE: usize = 0x30000;
/// V98-format font ROM size (288768 bytes).
const FONT_ROM_SIZE: usize = 0x46800;
/// PC-9801-26K sound BIOS ROM size (16 KB).
const SOUND_ROM_SIZE: usize = 0x4000;

/// One ROM slot: its expected size and the BLAKE3 digests that are accepted as
/// valid content for it. Multiple digests allow several known good dumps to
/// satisfy the same slot.
struct RomSlot {
    size: usize,
    accepted: &'static [&'static str],
}

const BIOS_F_SLOT: RomSlot = RomSlot {
    size: BIOS_ROM_SIZE,
    accepted: &["5587b89b968b005e81ea2bb4c2ef6fc762154d589e627920e3d9be9cd3e01b06"],
};
const BIOS_VM_SLOT: RomSlot = RomSlot {
    size: BIOS_ROM_SIZE,
    accepted: &["4377eeba8410c57f9a313ed2d24cd929cbfb7cac40244d5c6cafd1a27bf3495e"],
};
const BIOS_VX_SLOT: RomSlot = RomSlot {
    size: BIOS_ROM_SIZE,
    accepted: &["89ff271aa046bb6428761cdc3ec92d82e87350c5a4941974293c5b7fe2238aed"],
};
const BIOS_RA_SLOT: RomSlot = RomSlot {
    size: BIOS_ROM_SIZE,
    accepted: &["f18e91e8097661efe4543f30558383a02021047acfaa6d0a78e06d025094aa5e"],
};
const FONT_RS: &str = "4b6f751f34e633e072ded2a109c25ddb90ac70350792dc55914a4cefa4dbe005";
const FONT_UX: &str = "3c1efa858b80fc11bb7482bdc5e15004dd9a015d7d22d48159cd43ed63f540dc";
const FONT_AS: &str = "a567134a3d5c2a215b9573ee07b5204fff243631052e7a40be340e863aff8eef";
const FONT_AP2: &str = "7fb96af345c33f9bd7be5c22f75c650ac41da9b543ca5f9ca7b3d3906f2abb40";
const FONT_CE2: &str = "b38096265c76cf9f54cb47df905cfb6c8b4d4f27019a04835bbc3dc8782d33e1";

const FONT_STANDARD_SLOT: RomSlot = RomSlot {
    size: FONT_ROM_SIZE,
    accepted: &[FONT_RS, FONT_UX, FONT_AS, FONT_AP2, FONT_CE2],
};
const FONT_9821AS_SLOT: RomSlot = RomSlot {
    size: FONT_ROM_SIZE,
    accepted: &[FONT_AS, FONT_AP2, FONT_CE2, FONT_RS, FONT_UX],
};
const FONT_9821AP_SLOT: RomSlot = RomSlot {
    size: FONT_ROM_SIZE,
    accepted: &[FONT_AP2, FONT_AS, FONT_CE2, FONT_RS, FONT_UX],
};
const SOUND_SLOT: RomSlot = RomSlot {
    size: SOUND_ROM_SIZE,
    accepted: &["93816a6e42ed9a10135af634ed500e10b1d266e0b4158d3f8471910609255e24"],
};

/// Raw bytes of the ROMs found in a PC-98 ROM directory. Every entry is
/// optional; the caller decides which ones are required for the run.
pub struct LoadedRoms {
    /// Dual-bank BIOS ROM for the selected model, if present. `None` for the
    /// PC-9821 models, which have no supported real-BIOS boot path.
    pub bios: Option<Vec<u8>>,
    /// V98-format font ROM for the selected model, if present.
    pub font: Option<Vec<u8>>,
    /// PC-9801-26K sound BIOS ROM, if present.
    pub sound: Option<Vec<u8>>,
}

/// Error encountered while loading a PC-98 ROM set.
#[derive(Debug)]
pub enum RomError {
    /// The ROM directory could not be scanned.
    Read {
        /// The directory that failed to scan.
        directory: String,
        /// The underlying I/O error message.
        message: String,
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
        }
    }
}

impl std::error::Error for RomError {}

/// Returns the accepted BLAKE3 digests for the BIOS slot of a model, or `None`
/// for models without a supported real-BIOS boot path (PC-9821).
fn bios_slot(model: MachineModel) -> Option<&'static RomSlot> {
    match model {
        MachineModel::PC9801F => Some(&BIOS_F_SLOT),
        MachineModel::PC9801VM => Some(&BIOS_VM_SLOT),
        MachineModel::PC9801VX => Some(&BIOS_VX_SLOT),
        MachineModel::PC9801RA => Some(&BIOS_RA_SLOT),
        MachineModel::PC9821AS | MachineModel::PC9821AP => None,
    }
}

/// Returns the font slot for a model. The PC-9821 family prefers its own font
/// dump; every other model prefers the standard font. Every slot accepts all
/// known font dumps as a fallback.
fn font_slot(model: MachineModel) -> &'static RomSlot {
    match model {
        MachineModel::PC9801F
        | MachineModel::PC9801VM
        | MachineModel::PC9801VX
        | MachineModel::PC9801RA => &FONT_STANDARD_SLOT,
        MachineModel::PC9821AS => &FONT_9821AS_SLOT,
        MachineModel::PC9821AP => &FONT_9821AP_SLOT,
    }
}

/// Human-readable list of the accepted digests for a slot, used in caller error
/// messages when a required ROM is missing.
pub fn accepted_bios_digests(model: MachineModel) -> Vec<String> {
    bios_slot(model)
        .map(|slot| slot.accepted.iter().map(|d| d.to_string()).collect())
        .unwrap_or_default()
}

/// Loads the PC-98 ROMs found in `rom_dir`.
///
/// Every file is hashed and matched against the accepted digests for the BIOS,
/// font, and sound slots relevant to `model`, so the dump's file names do not
/// matter. All slots are optional here; missing ROMs come back as `None`.
pub fn load_rom_set(model: MachineModel, rom_dir: &Path) -> Result<LoadedRoms, RomError> {
    let by_digest = hash_directory(rom_dir)?;

    let take_optional = |slot: &RomSlot| -> Option<Vec<u8>> {
        for digest in slot.accepted {
            if let Some(data) = by_digest.get(*digest)
                && data.len() == slot.size
            {
                return Some(data.clone());
            }
        }
        None
    };

    let bios = bios_slot(model).and_then(take_optional);
    let font = take_optional(font_slot(model));
    let sound = take_optional(&SOUND_SLOT);

    Ok(LoadedRoms { bios, font, sound })
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
    [BIOS_ROM_SIZE, FONT_ROM_SIZE, SOUND_ROM_SIZE].contains(&size)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bios_slot_is_model_specific() {
        assert_eq!(
            accepted_bios_digests(MachineModel::PC9801F),
            BIOS_F_SLOT.accepted
        );
        assert_eq!(
            accepted_bios_digests(MachineModel::PC9801VM),
            BIOS_VM_SLOT.accepted
        );
        assert_eq!(
            accepted_bios_digests(MachineModel::PC9801VX),
            BIOS_VX_SLOT.accepted
        );
        assert_eq!(
            accepted_bios_digests(MachineModel::PC9801RA),
            BIOS_RA_SLOT.accepted
        );
        assert!(bios_slot(MachineModel::PC9821AS).is_none());
        assert!(bios_slot(MachineModel::PC9821AP).is_none());
    }

    #[test]
    fn font_slot_splits_pc9821() {
        assert_eq!(
            font_slot(MachineModel::PC9801VM).accepted,
            FONT_STANDARD_SLOT.accepted
        );
        assert_eq!(
            font_slot(MachineModel::PC9821AS).accepted,
            FONT_9821AS_SLOT.accepted
        );
        assert_eq!(
            font_slot(MachineModel::PC9821AP).accepted,
            FONT_9821AP_SLOT.accepted
        );
    }

    #[test]
    fn load_rom_set_matches_by_hash() {
        let dir = std::env::temp_dir().join(format!("neetan_pc98_rom_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");

        // A file whose content hashes to the VM BIOS digest cannot be forged
        // here, so instead verify the negative path: a directory with only a
        // wrong-sized stray file resolves every slot to None.
        std::fs::write(dir.join("stray.bin"), vec![0u8; 123]).expect("write stray");

        let roms = load_rom_set(MachineModel::PC9801VM, &dir).expect("scan succeeds");
        assert!(roms.bios.is_none());
        assert!(roms.font.is_none());
        assert!(roms.sound.is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
