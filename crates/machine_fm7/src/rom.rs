//! Fujitsu FM-7 / FM-77AV ROM set loading.
//!
//! ROMs are selected by content hash rather than file name: the loader scans
//! every file in the ROM directory, computes its BLAKE3 digest, and matches it
//! against a table of accepted digests per slot. Any dump layout works regardless
//! of how the files are named, and stray files are ignored.
//!
//! The 10 KiB SUBSYS_C monitor and the 128 KiB kanji ROM dumps are byte-identical
//! on both machines, so a single file can satisfy the matching slot for both
//! models.

use std::path::Path;

use rom_loader::{RomError, RomSlot, ScanOptions};

use crate::config::Fm7Model;

/// Size of the FM-7 BASIC/DOS boot ROM images.
const ROM_SIZE_512: usize = 0x0200;
/// Size of one FM-77AV 8 KiB ROM image.
const ROM_SIZE_8K: usize = 0x2000;
/// Size of the SUBSYS_C monitor ROM image.
const ROM_SIZE_10K: usize = 0x2800;
/// Size of the F-BASIC v3.0 ROM image.
const ROM_SIZE_31744: usize = 0x7C00;
/// Size of the kanji ROM image.
const ROM_SIZE_128K: usize = 0x2_0000;

/// ROM slot descriptor for F-BASIC v3.0.
const FBASIC_SLOT: RomSlot = RomSlot {
    label: "f-basic 3.0",
    size: ROM_SIZE_31744,
    accepted: &[
        "059a5c926109fc156f07d91aaad05307ff0bd9d3eb5bffa805d554863f4a01bc",
        "276f3953b3f8fe975d29d13463261d9e70ce9c339d2af12536cf2010ae0f2a8d",
    ],
};
/// ROM slot descriptor for the BASIC boot ROM.
const BOOT_BAS_SLOT: RomSlot = RomSlot {
    label: "boot rom (basic)",
    size: ROM_SIZE_512,
    accepted: &["d6a8dda5482a337e28aaf7b838be0543411277ba17f260ae62f9f1af46592b2d"],
};
/// ROM slot descriptor for the DOS boot ROM.
const BOOT_DOS_SLOT: RomSlot = RomSlot {
    label: "boot rom (dos)",
    size: ROM_SIZE_512,
    accepted: &["fbc9e9240f810deb8e28207b7a3362486f5f57294fb7ff8225628286479d26f3"],
};
/// ROM slot descriptor for the SUBSYS_C monitor ROM.
const SUBSYS_C_SLOT: RomSlot = RomSlot {
    label: "subsys c",
    size: ROM_SIZE_10K,
    accepted: &["55b0e4f72561ea0fafe6353376642d70595b08989a2d76c2b6423c7d85a9d1d2"],
};
/// ROM slot descriptor for the FM-77AV initiator ROM.
const INITIATE_SLOT: RomSlot = RomSlot {
    label: "initiate (AV)",
    size: ROM_SIZE_8K,
    accepted: &["4ac5111f650f4415763c1e0d9f6b997432f80c5ba9b60a38b68b308dcea9f404"],
};
/// ROM slot descriptor for the FM-77AV SUBSYS_A monitor ROM.
const SUBSYS_A_SLOT: RomSlot = RomSlot {
    label: "subsys a (AV)",
    size: ROM_SIZE_8K,
    accepted: &["413b20a42227ddf95e153685cc989dcf03b193aaf79f3429848db899bd6635e3"],
};
/// ROM slot descriptor for the FM-77AV SUBSYS_B monitor ROM.
const SUBSYS_B_SLOT: RomSlot = RomSlot {
    label: "subsys b (AV)",
    size: ROM_SIZE_8K,
    accepted: &["edf5fc537af21d93c73d3446e44654fbab0106edaf85f564abfad99bd28590e1"],
};
/// ROM slot descriptor for the FM-77AV sub CG font ROM.
const SUBSYSCG_SLOT: RomSlot = RomSlot {
    label: "subsys cg (AV)",
    size: ROM_SIZE_8K,
    accepted: &["7b430d28aebaf260a823e8585c31dacc2aaca9d4f69ab34672a1ded0b37cfd23"],
};
/// ROM slot descriptor for the kanji ROM.
const KANJI_SLOT: RomSlot = RomSlot {
    label: "kanji",
    size: ROM_SIZE_128K,
    accepted: &["482b314f15b6a063e06a8c3e6e7426d4de9b8513086ab0e72ff0ea1623ac51f6"],
};

/// Raw bytes of a successfully loaded FM-7 ROM set for one model. The fields
/// present depend on the model; absent roles are `None`.
pub struct LoadedRoms {
    /// The model these ROMs were loaded for.
    pub model: Fm7Model,
    /// F-BASIC v3.0 ROM.
    pub fbasic: Vec<u8>,
    /// Sub monitor type C.
    pub subsys_c: Vec<u8>,
    /// JIS level-1 kanji ROM. Optional on the FM-7, required on the FM-77AV.
    pub kanji: Option<Vec<u8>>,
    /// Boot ROM, BASIC mode (FM-7 only).
    pub boot_bas: Option<Vec<u8>>,
    /// Boot ROM, DOS mode (FM-7 only).
    pub boot_dos: Option<Vec<u8>>,
    /// Initiator ROM (FM-77AV only).
    pub initiate: Option<Vec<u8>>,
    /// Sub monitor type A (FM-77AV only).
    pub subsys_a: Option<Vec<u8>>,
    /// Sub monitor type B (FM-77AV only).
    pub subsys_b: Option<Vec<u8>>,
    /// Sub CG font ROM (FM-77AV only).
    pub subsyscg: Option<Vec<u8>>,
}

/// File sizes worth hashing when scanning a ROM directory.
const ROM_SIZES: &[usize] = &[
    BOOT_BAS_SLOT.size,
    SUBSYS_A_SLOT.size,
    SUBSYS_C_SLOT.size,
    FBASIC_SLOT.size,
    KANJI_SLOT.size,
];

/// Loads and validates the ROM set required by `model`.
///
/// Every file in `rom_dir` is hashed and matched against the accepted digests for
/// each ROM slot, so the dump's file names do not matter. The FM-7 needs F-BASIC,
/// both boot ROMs and SUBSYS_C (kanji is optional); the FM-77AV needs the
/// initiator, F-BASIC, all four sub monitors and the kanji ROM.
pub fn load_rom_set(model: Fm7Model, rom_dir: &Path) -> Result<LoadedRoms, RomError> {
    let index = rom_loader::scan_directory(rom_dir, &ScanOptions::sizes(ROM_SIZES))?;
    let take = |slot: &RomSlot| index.take(slot);

    let fbasic = take(&FBASIC_SLOT)?;
    let subsys_c = take(&SUBSYS_C_SLOT)?;

    let kanji = match model {
        Fm7Model::Fm7 => take(&KANJI_SLOT).ok(),
        Fm7Model::Fm77Av => Some(take(&KANJI_SLOT)?),
    };

    let (boot_bas, boot_dos) = match model {
        Fm7Model::Fm7 => (Some(take(&BOOT_BAS_SLOT)?), Some(take(&BOOT_DOS_SLOT)?)),
        Fm7Model::Fm77Av => (None, None),
    };

    let (initiate, subsys_a, subsys_b, subsyscg) = match model {
        Fm7Model::Fm7 => (None, None, None, None),
        Fm7Model::Fm77Av => (
            Some(take(&INITIATE_SLOT)?),
            Some(take(&SUBSYS_A_SLOT)?),
            Some(take(&SUBSYS_B_SLOT)?),
            Some(take(&SUBSYSCG_SLOT)?),
        ),
    };

    Ok(LoadedRoms {
        model,
        fbasic,
        subsys_c,
        kanji,
        boot_bas,
        boot_dos,
        initiate,
        subsys_a,
        subsys_b,
        subsyscg,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_directory_reports_read_error() {
        let result = load_rom_set(Fm7Model::Fm7, Path::new("/nonexistent/rom/dir"));
        assert!(matches!(result, Err(RomError::Read { .. })));
    }
}
