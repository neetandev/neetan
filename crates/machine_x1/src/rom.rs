//! Sharp X1 ROM set loading.
//!
//! ROMs are selected by content hash rather than file name: the loader scans
//! every file in the ROM directory, computes its BLAKE3 digest, and matches it
//! against a table of accepted digests per slot. Any dump layout works regardless
//! of how the files are named, and stray files are ignored.
//!
//! The 8x16 ANK font dump is byte-identical on both machines, so a single file
//! can satisfy the matching slot for both models.

use std::path::Path;

use rom_loader::{RomError, RomSlot, ScanOptions};

use crate::config::X1Model;

const ROM_SIZE_2K: usize = 0x0800;
const ROM_SIZE_4K: usize = 0x1000;
const ROM_SIZE_8K: usize = 0x2000;
const ROM_SIZE_32K: usize = 0x8000;

const IPL_X1_SLOT: RomSlot = RomSlot {
    label: "ipl (X1)",
    size: ROM_SIZE_4K,
    accepted: &["194f351bc1024188162856e2374d92bc608d9c742ca007d8c19a4b4eed44abbc"],
};
const IPL_TURBO_SLOT: RomSlot = RomSlot {
    label: "ipl (X1turbo)",
    size: ROM_SIZE_32K,
    accepted: &["871c77226a6e65bf1820c0a3e6f63a330cb1d2eb6c135fc9e4da9741ce38106c"],
};
const CGROM_X1_SLOT: RomSlot = RomSlot {
    label: "cgrom 8x8 (X1)",
    size: ROM_SIZE_2K,
    accepted: &["61440d736fdec066b825428f4d26fbdb04b3a4fcc7f05bbdd4b5bbe9e55318c3"],
};
const CGROM_TURBO_SLOT: RomSlot = RomSlot {
    label: "cgrom 8x8 (X1turbo)",
    size: ROM_SIZE_2K,
    accepted: &["f26c67af04f3b4819e0bd474ded7b083e3d370a62ea0672f09787b8ca4ebc4a6"],
};

const ANK_SLOT: RomSlot = RomSlot {
    label: "ank 8x16",
    size: ROM_SIZE_8K,
    accepted: &["a8695470e98492a2d969ba3fdeee76ee9b3573f525eee20f98627fb5e98279a0"],
};

const KANJI1_SLOT: RomSlot = RomSlot {
    label: "kanji1",
    size: ROM_SIZE_32K,
    accepted: &["212d081a600377a1068d56f4049d03916ea705465eb2feca950b6df186a12ba4"],
};
const KANJI2_SLOT: RomSlot = RomSlot {
    label: "kanji2",
    size: ROM_SIZE_32K,
    accepted: &["0bd59d087b3197c8136e5664e311234930ec566b61d184204144f04a84ba769b"],
};
const KANJI3_SLOT: RomSlot = RomSlot {
    label: "kanji3",
    size: ROM_SIZE_32K,
    accepted: &["f2495255441c15bfce5c7441f6d94809d4f0e0dba1c7f43f9153991e326b881a"],
};
const KANJI4_SLOT: RomSlot = RomSlot {
    label: "kanji4",
    size: ROM_SIZE_32K,
    accepted: &["84e0afa27e1f4ef01b6e5dac452835f487c98968e14fceaac3c93331524b51d7"],
};

/// Raw bytes of a successfully loaded X1 ROM set for one model. The fields present
/// depend on the model; absent roles are `None`.
pub struct LoadedRoms {
    /// The model these ROMs were loaded for.
    pub model: X1Model,
    /// IPL boot ROM (4 KiB on the base X1, 32 KiB on the turbo).
    pub ipl: Vec<u8>,
    /// 8x8 character generator ROM.
    pub cgrom_8x8: Vec<u8>,
    /// 8x16 ANK font ROM.
    pub ank: Vec<u8>,
    /// De-interleaved 128 KiB kanji ROM (turbo only): a glyph occupies
    /// `char_code * 0x20` bytes, sixteen rows per left/right half. `None` on the
    /// base X1.
    pub kanji: Option<Vec<u8>>,
}

const ROM_SIZES: &[usize] = &[ROM_SIZE_2K, ROM_SIZE_4K, ROM_SIZE_8K, ROM_SIZE_32K];

/// Loads and validates the ROM set required by `model`.
///
/// Every file in `rom_dir` is hashed and matched against the accepted digests for
/// each ROM slot, so the dump's file names do not matter. The base X1 needs only
/// the IPL, 8x8 CG and ANK fonts; the turbo additionally requires the four kanji
/// ROMs, concatenated into the linear 128 KiB kanji region.
pub fn load_rom_set(model: X1Model, rom_dir: &Path) -> Result<LoadedRoms, RomError> {
    let index = rom_loader::scan_directory(rom_dir, &ScanOptions::sizes(ROM_SIZES))?;
    let take = |slot: &RomSlot| index.take(slot);

    let ipl_slot = match model {
        X1Model::X1 => &IPL_X1_SLOT,
        X1Model::X1Turbo => &IPL_TURBO_SLOT,
    };
    let cgrom_slot = match model {
        X1Model::X1 => &CGROM_X1_SLOT,
        X1Model::X1Turbo => &CGROM_TURBO_SLOT,
    };

    let kanji = match model {
        X1Model::X1 => None,
        X1Model::X1Turbo => {
            // The raw dumps are laid out in the order kanji4, kanji2, kanji3,
            // kanji1 (the order the two 64 KiB halves are wired on the board);
            // the de-interleave below relies on that arrangement.
            let mut raw = Vec::with_capacity(4 * ROM_SIZE_32K);
            raw.extend_from_slice(&take(&KANJI4_SLOT)?);
            raw.extend_from_slice(&take(&KANJI2_SLOT)?);
            raw.extend_from_slice(&take(&KANJI3_SLOT)?);
            raw.extend_from_slice(&take(&KANJI1_SLOT)?);
            Some(deinterleave_kanji(&raw))
        }
    };

    Ok(LoadedRoms {
        model,
        ipl: take(ipl_slot)?,
        cgrom_8x8: take(cgrom_slot)?,
        ank: take(&ANK_SLOT)?,
        kanji,
    })
}

/// De-interleaves the raw 128 KiB kanji dump into the addressing the video and
/// kanji-port paths expect: after this a full-width glyph occupies `char * 0x20`
/// bytes, sixteen consecutive rows for the left half and sixteen for the right.
/// The two 64 KiB halves are de-interleaved independently in 16-byte groups that
/// alternate between the low and high output blocks.
fn deinterleave_kanji(raw: &[u8]) -> Vec<u8> {
    const HALF: usize = 0x1_0000;
    let mut kanji = vec![0u8; 2 * HALF];
    let mut source = 0usize;
    for half in 0..2usize {
        let mut group = half * 16;
        while group < half * 16 + HALF {
            for row in 0..16usize {
                kanji[group + row] = raw.get(source).copied().unwrap_or(0);
                kanji[group + row + HALF] = raw.get(HALF + source).copied().unwrap_or(0);
                source += 1;
            }
            group += 32;
        }
    }
    kanji
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_directory_reports_read_error() {
        let result = load_rom_set(X1Model::X1, Path::new("/nonexistent/rom/dir"));
        assert!(matches!(result, Err(RomError::Read { .. })));
    }
}
