//! PC-6000/PC-6600 ROM set loading.
//!
//! ROMs are selected by content hash rather than file name: the loader scans
//! every file in the ROM directory, computes its BLAKE3 digest, and matches it
//! against a table of accepted digests per slot. Any dump layout works
//! regardless of how the files are named, and stray files are ignored.
//!
//! The accepted digests below were recomputed from the reference dump and are
//! the authoritative identity table. Several dumps are bit-identical across
//! models (the kanji ROM, the SR system ROM halves, an extended CG), so a single
//! file satisfies the matching slot for more than one model.

use std::path::Path;

use rom_loader::{RomError, RomSlot, ScanOptions};

use crate::config::Pc6000Model;

const ROM_SIZE_4K: usize = 0x1000;
const ROM_SIZE_8K: usize = 0x2000;
const ROM_SIZE_16K: usize = 0x4000;
const ROM_SIZE_32K: usize = 0x8000;
const ROM_SIZE_64K: usize = 0x1_0000;

const BASIC_60_SLOT: RomSlot = RomSlot {
    label: "basic (PC-6001)",
    size: ROM_SIZE_16K,
    accepted: &["13bc0696487984f7836f094312b64fb0702dcb5ac3b941a79bd6f174e657697d"],
};
const BASIC_62_SLOT: RomSlot = RomSlot {
    label: "basic (PC-6001mkII)",
    size: ROM_SIZE_32K,
    accepted: &["d951eae886dec98a063e5fb11e12b0385f5dd4617c0546fe7cf9fd77b17ae41c"],
};
const BASIC_66_SLOT: RomSlot = RomSlot {
    label: "basic (PC-6601)",
    size: ROM_SIZE_32K,
    accepted: &["d9eaf3e5e6cb1f71db527e6eeadf7a1968f8a558234b74c6812198c588ae46d1"],
};
const BASIC_68_SLOT: RomSlot = RomSlot {
    label: "basic (PC-6601SR)",
    size: ROM_SIZE_32K,
    accepted: &["c4901a2149f3c8e65d3db78bbf3776fc2d963f270152923ba920274d44a0224b"],
};

const SYSTEM_ROM1_SLOT: RomSlot = RomSlot {
    label: "sr system rom 1",
    size: ROM_SIZE_64K,
    accepted: &["6ca4e747c8b17307a77150441e5d8721d5c242fcc8b8ef35737d3f5edf6e2d74"],
};
const SYSTEM_ROM2_SLOT: RomSlot = RomSlot {
    label: "sr system rom 2",
    size: ROM_SIZE_64K,
    accepted: &["998a90c4bd0bf4ae4a600a0d94f3eca96c3b8db754311ce1c8029126dbcf0a9a"],
};
const SUB_ROM_SLOT: RomSlot = RomSlot {
    label: "sr sub/disk rom",
    size: ROM_SIZE_8K,
    accepted: &["becb7c1502d41a9f160b651e142044610ffa172a8bbf47eaa11aa0086953a080"],
};

const CG_60_SLOT: RomSlot = RomSlot {
    label: "cg (PC-6001 base)",
    size: ROM_SIZE_4K,
    accepted: &["f537afe76997ec4f8b377a29771f45c39414a25f7e071d2d38b143cdd8bee7bc"],
};
const CG_62_SLOT: RomSlot = RomSlot {
    label: "cg (PC-6001mkII base)",
    size: ROM_SIZE_8K,
    accepted: &["581f6d2db80386732ed09706ad3b8961f8b77b7ea024e65cec37e56ad2adf07c"],
};
const CG_66_BASE_SLOT: RomSlot = RomSlot {
    label: "cg (PC-6601 base)",
    size: ROM_SIZE_8K,
    accepted: &["63829a1c32924a77f85716f445c445ab7be178c4438cfd8cf6ffaff5731a0965"],
};
const CG_68_BASE_SLOT: RomSlot = RomSlot {
    label: "cg (PC-6601SR base)",
    size: ROM_SIZE_8K,
    accepted: &["24e524d4938809a87720f98abfba71c8e9162d742c67a167d8b87566cc1d4258"],
};
const CG_EXT_SLOT: RomSlot = RomSlot {
    label: "cg (extended)",
    size: ROM_SIZE_8K,
    accepted: &["ba0dd650539dd3fdbf63da36982b41bfda8f4c2ea0dcda2c1c2ac56427ee26ed"],
};
const CG_66_EXT_68_SLOT: RomSlot = RomSlot {
    label: "cg (PC-6601SR extended)",
    size: ROM_SIZE_8K,
    accepted: &["067c732525260eadfcfecbb9fc4ef9535c0c2f77caa049453bf2ab992ec3fca3"],
};
const CG_68_SLOT: RomSlot = RomSlot {
    label: "cg (SR)",
    size: ROM_SIZE_16K,
    accepted: &["b49b056ca06bd0c2253e6db0806969787a6fca4fc78228728422c9cf63f1e472"],
};

const KANJI_SLOT: RomSlot = RomSlot {
    label: "kanji",
    size: ROM_SIZE_32K,
    accepted: &["f0af53e54b1b09b229d03efc9f65e65597a0c4f6aa9e3e7c0e553274ccd481fb"],
};

const VOICE_62_SLOT: RomSlot = RomSlot {
    label: "voice (PC-6001mkII)",
    size: ROM_SIZE_16K,
    accepted: &["633e73f55479bee65ed344d818a35b15ab109f188ad5c09826c066d6ec2596c5"],
};
const VOICE_66_SLOT: RomSlot = RomSlot {
    label: "voice (PC-6601)",
    size: ROM_SIZE_16K,
    accepted: &["88a747147725fd618668e07744b05f34288b4454698d6182c4db2e680c7b76d0"],
};
const VOICE_68_SLOT: RomSlot = RomSlot {
    label: "voice (PC-6601SR)",
    size: ROM_SIZE_16K,
    accepted: &["8ed4a9a3e9ae2e4aa0fccc0f170081f3f61c09e293812b7973a7ab9c23e22b68"],
};

/// Raw bytes of a successfully loaded PC-6000 ROM set for one model. The fields
/// present depend on the model; absent roles are `None`.
pub struct LoadedRoms {
    /// The model these ROMs were loaded for.
    pub model: Pc6000Model,
    /// BASIC / system firmware (non-SR models, and the PC-6601SR mkII-compat ROM).
    pub basic: Option<Vec<u8>>,
    /// First half of the 128 KiB SR system ROM.
    pub system_rom1: Option<Vec<u8>>,
    /// Second half of the 128 KiB SR system ROM.
    pub system_rom2: Option<Vec<u8>>,
    /// PC-6601SR sub / disk boot ROM (8 KiB).
    pub sub_rom: Option<Vec<u8>>,
    /// Character generator for the base text/semigraphics modes.
    pub cg_base: Option<Vec<u8>>,
    /// Extended character generator for the mkII / 6601 modes.
    pub cg_ext: Option<Vec<u8>>,
    /// Character generator for the native SR modes.
    pub cg_sr: Option<Vec<u8>>,
    /// Kanji font ROM.
    pub kanji: Option<Vec<u8>>,
    /// Data ROM for the uPD7752 voice synthesizer.
    pub voice: Option<Vec<u8>>,
}

impl LoadedRoms {
    /// The ROM mapped at the reset vector: the SR system ROM on SR machines,
    /// otherwise the BASIC ROM.
    pub fn boot_rom(&self) -> &[u8] {
        if self.model.is_sr() {
            self.system_rom1.as_deref().unwrap_or(&[])
        } else {
            self.basic.as_deref().unwrap_or(&[])
        }
    }

    /// The character generator used as the font ROM (base CG, or the SR CG when
    /// no base CG is present).
    pub fn font_rom(&self) -> &[u8] {
        self.cg_base
            .as_deref()
            .or(self.cg_sr.as_deref())
            .unwrap_or(&[])
    }
}

/// File sizes worth hashing when scanning a ROM directory.
const ROM_SIZES: &[usize] = &[
    CG_60_SLOT.size,
    CG_62_SLOT.size,
    BASIC_60_SLOT.size,
    BASIC_62_SLOT.size,
    SYSTEM_ROM1_SLOT.size,
];

/// Loads and validates the ROM set required by `model`.
///
/// Every file in `rom_dir` is hashed and matched against the accepted digests
/// for each ROM slot, so the dump's file names do not matter. Only the slots a
/// model needs to boot are required; the remaining character/kanji/voice ROMs
/// are loaded when present.
pub fn load_rom_set(model: Pc6000Model, rom_dir: &Path) -> Result<LoadedRoms, RomError> {
    let index = rom_loader::scan_directory(rom_dir, &ScanOptions::sizes(ROM_SIZES))?;
    let take = |slot: &RomSlot| index.take(slot);
    let take_optional = |slot: &RomSlot| index.take_optional(slot);

    let mut roms = LoadedRoms {
        model,
        basic: None,
        system_rom1: None,
        system_rom2: None,
        sub_rom: None,
        cg_base: None,
        cg_ext: None,
        cg_sr: None,
        kanji: None,
        voice: None,
    };

    match model {
        Pc6000Model::Pc6001 => {
            roms.basic = Some(take(&BASIC_60_SLOT)?);
            roms.cg_base = Some(take(&CG_60_SLOT)?);
        }
        Pc6000Model::Pc6001Mk2 => {
            roms.basic = Some(take(&BASIC_62_SLOT)?);
            roms.cg_base = Some(take(&CG_62_SLOT)?);
            roms.cg_ext = take_optional(&CG_EXT_SLOT);
            roms.kanji = take_optional(&KANJI_SLOT);
            roms.voice = take_optional(&VOICE_62_SLOT);
        }
        Pc6000Model::Pc6601 => {
            roms.basic = Some(take(&BASIC_66_SLOT)?);
            roms.cg_base = Some(take(&CG_66_BASE_SLOT)?);
            roms.cg_ext = take_optional(&CG_EXT_SLOT);
            roms.kanji = take_optional(&KANJI_SLOT);
            roms.voice = take_optional(&VOICE_66_SLOT);
        }
        Pc6000Model::Pc6001Mk2Sr => {
            roms.system_rom1 = Some(take(&SYSTEM_ROM1_SLOT)?);
            roms.system_rom2 = Some(take(&SYSTEM_ROM2_SLOT)?);
            roms.cg_sr = Some(take(&CG_68_SLOT)?);
            roms.cg_base = take_optional(&CG_68_BASE_SLOT);
            roms.cg_ext = take_optional(&CG_66_EXT_68_SLOT);
            roms.kanji = take_optional(&KANJI_SLOT);
            roms.voice = take_optional(&VOICE_68_SLOT);
        }
        Pc6000Model::Pc6601Sr => {
            roms.system_rom1 = Some(take(&SYSTEM_ROM1_SLOT)?);
            roms.system_rom2 = Some(take(&SYSTEM_ROM2_SLOT)?);
            roms.cg_sr = Some(take(&CG_68_SLOT)?);
            roms.basic = take_optional(&BASIC_68_SLOT);
            roms.sub_rom = take_optional(&SUB_ROM_SLOT);
            roms.cg_base = take_optional(&CG_68_BASE_SLOT);
            roms.cg_ext = take_optional(&CG_66_EXT_68_SLOT);
            roms.kanji = take_optional(&KANJI_SLOT);
            roms.voice = take_optional(&VOICE_68_SLOT);
        }
    }

    Ok(roms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_directory_reports_read_error() {
        let result = load_rom_set(Pc6000Model::Pc6001, Path::new("/nonexistent/rom/dir"));
        assert!(matches!(result, Err(RomError::Read { .. })));
    }
}
