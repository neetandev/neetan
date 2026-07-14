//! PC-98 ROM set loading from MAME ROM dumps.
//!
//! The loader reads the raw chip dumps found in an officially released MAME ROM
//! set and assembles them into the images the emulator consumes. Files are
//! selected by BLAKE3 content hash rather than by name: the loader scans every
//! file in the ROM directory, computes its digest, and matches it against the
//! chips a model needs. Any dump layout works regardless of file names, and a
//! single directory may hold several sets at once (the chips for the selected
//! model are picked out by hash).
//!
//! The BIOS is assembled per model from its individual mask-ROM chips following
//! MAME's ROM layout, producing the 192 KB dual-bank image the memory layer
//! expects. The font and sound ROMs are used directly (the MAME `font_*.rom`
//! and `sound.rom` files are already in the format we need).
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

/// Size of the low ITF/BIOS page assembled from a model's chips (96 KB). The
/// canonical dual-bank image places its top 32 KB as the ITF window and the
/// whole page as the BIOS window.
const BIOS_PAGE_SIZE: usize = 0x18000;
/// Offset of the ITF window inside the dual-bank image.
const ITF_WINDOW_OFFSET: usize = 0x10000;
/// Offset of the BIOS window inside the dual-bank image.
const BIOS_WINDOW_OFFSET: usize = 0x18000;

const KIB_16: usize = 0x4000;
const KIB_32: usize = 0x8000;

/// A single MAME chip dump: its expected size and BLAKE3 digest.
struct Chip {
    digest: &'static str,
    size: usize,
}

// PC-9801F CPU-board IPL chips (MAME set `pc9801f`, urm01-02 .. urm06-02).
const F_URM01: Chip = Chip {
    digest: "cbac44179293aa4ad530c72fa19f2e3ac8278f3e6816a8691db82fa81d82e11c",
    size: KIB_16,
};
const F_URM02: Chip = Chip {
    digest: "e2717e5f6145218f2ddfa53d57c51df4da117b3b97ce044f81e90032bda4db69",
    size: KIB_16,
};
const F_URM03: Chip = Chip {
    digest: "a3ae6097b2203e5a5434dedb83f3eaeb9417551649e96af0e07cae1b8e8d4a7f",
    size: KIB_16,
};
const F_URM04: Chip = Chip {
    digest: "6b171375a77c20d515babda27c6189be6c5caa32c84825b003e616932c2d99bb",
    size: KIB_16,
};
const F_URM05: Chip = Chip {
    digest: "76f8963dd66a65b05b862f6193a8bd05b2ce64b4c388394136544a297bcc2757",
    size: KIB_16,
};
const F_URM06: Chip = Chip {
    digest: "8c14048b85a320340a07a44fadb1d54b04fa3da50e38f36ed19ae2cc1870886f",
    size: KIB_16,
};

// PC-9801VM CPU-board IPL chips (MAME set `pc9801vm`).
const VM_CPU_1A: Chip = Chip {
    digest: "eb16e6050452c218497e6cf28591e8c049cca0a313bb5d9b8f30e2b22a58a939",
    size: KIB_16,
};
const VM_CPU_2A: Chip = Chip {
    digest: "f8b7cda3cf40c9feca6899cc1045cdd65cd85a90511560eace8028253e1ce1f3",
    size: KIB_32,
};
const VM_CPU_3A: Chip = Chip {
    digest: "13218482d54793a10a25ea712a5be362d1d490c568c3e0228939af3f2c244b9c",
    size: KIB_32,
};
const VM_CPU_4A: Chip = Chip {
    digest: "00b3558c12b28dff9ab823354b289ad559c59d3695e80c1586f07e23890d45a6",
    size: KIB_16,
};

// PC-9801VX CPU ext-board IPL chips (MAME set `pc9801vx`, yll01-04).
const VX_YLL01: Chip = Chip {
    digest: "04de20aabdf46d943cd5148e4bbfdd7ba843fe19d8fec7afbaa48630f6887e52",
    size: KIB_32,
};
const VX_YLL02: Chip = Chip {
    digest: "3de5476a72aadd3e32870f3525bc4fc945e941f8c3cfe81aeb0fd73c955294ee",
    size: KIB_32,
};
const VX_YLL03: Chip = Chip {
    digest: "05e3220b53a61c9325ceb694629bddc20c593dbf2218ee78a9e5635e8a7bf5f6",
    size: KIB_32,
};
const VX_YLL04: Chip = Chip {
    digest: "a0f6e1e87afa336c21648f972c96e20ea88dda792a1fc3d0acf26d8c50546158",
    size: KIB_32,
};

// PC-9801RS IPL images (MAME set `pc9801rs`). This set serves both the
// PC-9801RS and the PC-9801RA. These are already merged bank images.
const RS_ITF: Chip = Chip {
    digest: "c1881b44dc07a7f20ceff00a24fe4467a933fd2c94e64213c9a8526d60e4d3d1",
    size: KIB_32,
};
const RS_BIOS: Chip = Chip {
    digest: "ac5b46fbec4a5ac6b3185066d86af8e3d76cd1b66955301dad3cae8736b31f2d",
    size: 0x18000,
};

// Expected BLAKE3 of the assembled dual-bank BIOS image, per model. Used as a
// post-assembly integrity check: assembling the matched chips is deterministic,
// so a mismatch here means a corrupted dump or an assembly bug.
const ASSEMBLED_BIOS_F: &str = "5587b89b968b005e81ea2bb4c2ef6fc762154d589e627920e3d9be9cd3e01b06";
const ASSEMBLED_BIOS_VM: &str = "4377eeba8410c57f9a313ed2d24cd929cbfb7cac40244d5c6cafd1a27bf3495e";
const ASSEMBLED_BIOS_VX: &str = "89ff271aa046bb6428761cdc3ec92d82e87350c5a4941974293c5b7fe2238aed";
const ASSEMBLED_BIOS_RA: &str = "f18e91e8097661efe4543f30558383a02021047acfaa6d0a78e06d025094aa5e";

// Font ROMs (V98 format, 288768 bytes), used directly. Every model accepts all
// known dumps but prefers the one matching its family.
const FONT_RS: &str = "4b6f751f34e633e072ded2a109c25ddb90ac70350792dc55914a4cefa4dbe005";
const FONT_UX: &str = "3c1efa858b80fc11bb7482bdc5e15004dd9a015d7d22d48159cd43ed63f540dc";
const FONT_AS: &str = "a567134a3d5c2a215b9573ee07b5204fff243631052e7a40be340e863aff8eef";
const FONT_AP2: &str = "7fb96af345c33f9bd7be5c22f75c650ac41da9b543ca5f9ca7b3d3906f2abb40";
const FONT_CE2: &str = "b38096265c76cf9f54cb47df905cfb6c8b4d4f27019a04835bbc3dc8782d33e1";

const FONT_STANDARD: &[&str] = &[FONT_RS, FONT_UX, FONT_AS, FONT_AP2, FONT_CE2];
const FONT_9821AS: &[&str] = &[FONT_AS, FONT_AP2, FONT_CE2, FONT_RS, FONT_UX];
const FONT_9821AP: &[&str] = &[FONT_AP2, FONT_AS, FONT_CE2, FONT_RS, FONT_UX];

// PC-9801-26K sound BIOS ROM (16 KB), used directly.
const SOUND_DIGEST: &str = "93816a6e42ed9a10135af634ed500e10b1d266e0b4158d3f8471910609255e24";

/// Raw bytes of the ROMs found in a PC-98 ROM directory. Every entry is
/// optional; the caller decides which ones are required for the run.
pub struct LoadedRoms {
    /// Assembled dual-bank BIOS ROM for the selected model, if its chips were
    /// present. `None` for the PC-9821 models, which have no supported
    /// real-BIOS boot path.
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

/// Returns the MAME ROM set that supplies the BIOS chips for a model, or `None`
/// for models without a supported real-BIOS boot path (PC-9821).
pub fn required_mame_set(model: MachineModel) -> Option<&'static str> {
    match model {
        MachineModel::PC9801F => Some("pc9801f"),
        MachineModel::PC9801VM => Some("pc9801vm"),
        MachineModel::PC9801VX => Some("pc9801vx"),
        MachineModel::PC9801RS | MachineModel::PC9801RA => Some("pc9801rs"),
        MachineModel::PC9821AS | MachineModel::PC9821AP => None,
    }
}

/// The BIOS chips a model needs, together with the expected digest of the
/// assembled image. `None` for the PC-9821 family (no real-BIOS path).
fn bios_chips(model: MachineModel) -> Option<(&'static [&'static Chip], &'static str)> {
    match model {
        MachineModel::PC9801F => Some((
            &[&F_URM01, &F_URM02, &F_URM03, &F_URM04, &F_URM05, &F_URM06],
            ASSEMBLED_BIOS_F,
        )),
        MachineModel::PC9801VM => Some((
            &[&VM_CPU_1A, &VM_CPU_2A, &VM_CPU_3A, &VM_CPU_4A],
            ASSEMBLED_BIOS_VM,
        )),
        MachineModel::PC9801VX => Some((
            &[&VX_YLL01, &VX_YLL02, &VX_YLL03, &VX_YLL04],
            ASSEMBLED_BIOS_VX,
        )),
        MachineModel::PC9801RS | MachineModel::PC9801RA => {
            Some((&[&RS_ITF, &RS_BIOS], ASSEMBLED_BIOS_RA))
        }
        MachineModel::PC9821AS | MachineModel::PC9821AP => None,
    }
}

/// Returns the accepted font digests for a model. The PC-9821 family prefers
/// its own font dump; every other model prefers the standard font. Every model
/// accepts all known font dumps as a fallback.
fn font_digests(model: MachineModel) -> &'static [&'static str] {
    match model {
        MachineModel::PC9801F
        | MachineModel::PC9801VM
        | MachineModel::PC9801VX
        | MachineModel::PC9801RS
        | MachineModel::PC9801RA => FONT_STANDARD,
        MachineModel::PC9821AS => FONT_9821AS,
        MachineModel::PC9821AP => FONT_9821AP,
    }
}

/// Human-readable list of the BIOS chip digests required for a model, used in
/// caller error messages when the BIOS cannot be assembled.
pub fn accepted_bios_digests(model: MachineModel) -> Vec<String> {
    bios_chips(model)
        .map(|(chips, _)| chips.iter().map(|chip| chip.digest.to_string()).collect())
        .unwrap_or_default()
}

/// Loads the PC-98 ROMs found in `rom_dir`.
///
/// Every file is hashed; the BIOS is assembled from the model's chips and the
/// font and sound ROMs are matched directly. File names do not matter. All
/// slots are optional here; missing ROMs come back as `None`.
pub fn load_rom_set(model: MachineModel, rom_dir: &Path) -> Result<LoadedRoms, RomError> {
    let by_digest = hash_directory(rom_dir)?;

    let bios = bios_chips(model).and_then(|(_chips, expected)| {
        let image = assemble_bios(model, &by_digest)?;
        (blake3_hex(&image) == expected).then_some(image)
    });

    let font = font_digests(model).iter().find_map(|digest| {
        by_digest
            .get(*digest)
            .filter(|data| data.len() == FONT_ROM_SIZE)
            .cloned()
    });

    let sound = by_digest
        .get(SOUND_DIGEST)
        .filter(|data| data.len() == SOUND_ROM_SIZE)
        .cloned();

    Ok(LoadedRoms { bios, font, sound })
}

/// Fetches a chip's bytes by BLAKE3 digest, verifying the expected size.
fn chip<'a>(by_digest: &'a HashMap<String, Vec<u8>>, chip: &Chip) -> Option<&'a [u8]> {
    by_digest
        .get(chip.digest)
        .filter(|data| data.len() == chip.size)
        .map(Vec::as_slice)
}

/// Copies `src` into `dst` byte by byte at `start`, `start + 2`, ... matching
/// MAME's `ROM_LOAD16_BYTE` interleave of even/odd halves of a 16-bit bus.
fn interleave(dst: &mut [u8], src: &[u8], start: usize) {
    for (index, &byte) in src.iter().enumerate() {
        dst[start + index * 2] = byte;
    }
}

/// Wraps a 96 KB IPL page into the canonical 192 KB dual-bank image: the page's
/// top 32 KB become the ITF window and the whole page becomes the BIOS window.
fn canonical_dual_bank(page: &[u8]) -> Vec<u8> {
    let mut image = vec![0xFFu8; BIOS_ROM_SIZE];
    image[ITF_WINDOW_OFFSET..ITF_WINDOW_OFFSET + KIB_32]
        .copy_from_slice(&page[ITF_WINDOW_OFFSET..BIOS_PAGE_SIZE]);
    image[BIOS_WINDOW_OFFSET..BIOS_WINDOW_OFFSET + BIOS_PAGE_SIZE].copy_from_slice(page);
    image
}

/// Assembles the model's dual-bank BIOS image from its chips, or `None` if any
/// chip is missing.
fn assemble_bios(model: MachineModel, by_digest: &HashMap<String, Vec<u8>>) -> Option<Vec<u8>> {
    match model {
        MachineModel::PC9801F => {
            let mut page = vec![0xFFu8; BIOS_PAGE_SIZE];
            interleave(&mut page, chip(by_digest, &F_URM01)?, 0x00000);
            interleave(&mut page, chip(by_digest, &F_URM02)?, 0x00001);
            interleave(&mut page, chip(by_digest, &F_URM03)?, 0x08000);
            interleave(&mut page, chip(by_digest, &F_URM04)?, 0x08001);
            interleave(&mut page, chip(by_digest, &F_URM05)?, 0x10000);
            interleave(&mut page, chip(by_digest, &F_URM06)?, 0x10001);
            Some(canonical_dual_bank(&page))
        }
        MachineModel::PC9801VM => {
            let cpu_1a = chip(by_digest, &VM_CPU_1A)?;
            let cpu_2a = chip(by_digest, &VM_CPU_2A)?;
            let cpu_3a = chip(by_digest, &VM_CPU_3A)?;
            let cpu_4a = chip(by_digest, &VM_CPU_4A)?;
            let mut page = vec![0xFFu8; BIOS_PAGE_SIZE];
            interleave(&mut page, cpu_4a, 0x10000);
            interleave(&mut page, cpu_1a, 0x10001);
            interleave(&mut page, &cpu_3a[..KIB_16], 0x08000);
            interleave(&mut page, &cpu_3a[KIB_16..], 0x00000);
            interleave(&mut page, &cpu_2a[..KIB_16], 0x08001);
            interleave(&mut page, &cpu_2a[KIB_16..], 0x00001);
            Some(canonical_dual_bank(&page))
        }
        MachineModel::PC9801VX => {
            let mut biosrom = vec![0xFFu8; 0x20000];
            interleave(&mut biosrom, chip(by_digest, &VX_YLL01)?, 0x00000);
            interleave(&mut biosrom, chip(by_digest, &VX_YLL03)?, 0x00001);
            interleave(&mut biosrom, chip(by_digest, &VX_YLL02)?, 0x10000);
            interleave(&mut biosrom, chip(by_digest, &VX_YLL04)?, 0x10001);
            let mut image = vec![0xFFu8; BIOS_ROM_SIZE];
            image[0x10000..0x18000].copy_from_slice(&biosrom[0x18000..0x20000]);
            image[0x18000..0x20000].copy_from_slice(&biosrom[0x08000..0x10000]);
            image[0x20000..0x28000].copy_from_slice(&biosrom[0x00000..0x08000]);
            image[0x28000..0x30000].copy_from_slice(&biosrom[0x10000..0x18000]);
            Some(image)
        }
        MachineModel::PC9801RS | MachineModel::PC9801RA => {
            let itf = chip(by_digest, &RS_ITF)?;
            let bios = chip(by_digest, &RS_BIOS)?;
            let mut image = vec![0xFFu8; BIOS_ROM_SIZE];
            image[ITF_WINDOW_OFFSET..ITF_WINDOW_OFFSET + KIB_32].copy_from_slice(itf);
            image[BIOS_WINDOW_OFFSET..BIOS_WINDOW_OFFSET + BIOS_PAGE_SIZE].copy_from_slice(bios);
            Some(image)
        }
        MachineModel::PC9821AS | MachineModel::PC9821AP => None,
    }
}

/// Reads every regular file in `dir` and maps its BLAKE3 digest to its contents.
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
        by_digest.entry(blake3_hex(&data)).or_insert(data);
    }
    Ok(by_digest)
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
    fn required_mame_set_is_model_specific() {
        assert_eq!(required_mame_set(MachineModel::PC9801F), Some("pc9801f"));
        assert_eq!(required_mame_set(MachineModel::PC9801VM), Some("pc9801vm"));
        assert_eq!(required_mame_set(MachineModel::PC9801VX), Some("pc9801vx"));
        assert_eq!(required_mame_set(MachineModel::PC9801RS), Some("pc9801rs"));
        assert_eq!(required_mame_set(MachineModel::PC9801RA), Some("pc9801rs"));
        assert_eq!(required_mame_set(MachineModel::PC9821AS), None);
        assert_eq!(required_mame_set(MachineModel::PC9821AP), None);
    }

    #[test]
    fn accepted_bios_digests_lists_chips() {
        assert_eq!(accepted_bios_digests(MachineModel::PC9801F).len(), 6);
        assert_eq!(accepted_bios_digests(MachineModel::PC9801VM).len(), 4);
        assert_eq!(accepted_bios_digests(MachineModel::PC9801VX).len(), 4);
        assert_eq!(accepted_bios_digests(MachineModel::PC9801RS).len(), 2);
        assert_eq!(accepted_bios_digests(MachineModel::PC9801RA).len(), 2);
        assert!(accepted_bios_digests(MachineModel::PC9821AS).is_empty());
    }

    #[test]
    fn font_digests_split_pc9821() {
        assert_eq!(font_digests(MachineModel::PC9801VM), FONT_STANDARD);
        assert_eq!(font_digests(MachineModel::PC9821AS), FONT_9821AS);
        assert_eq!(font_digests(MachineModel::PC9821AP), FONT_9821AP);
    }

    /// Builds a digest map from `(bytes, digest-key)` pairs, keying each blob by
    /// an arbitrary digest string so assembly logic can be exercised without the
    /// real ROM content.
    fn map(entries: &[(&str, Vec<u8>)]) -> HashMap<String, Vec<u8>> {
        entries
            .iter()
            .map(|(digest, data)| (digest.to_string(), data.clone()))
            .collect()
    }

    #[test]
    fn assemble_ra_places_itf_and_bios_windows() {
        let by_digest = map(&[
            (RS_ITF.digest, vec![0xAA; RS_ITF.size]),
            (RS_BIOS.digest, vec![0xBB; RS_BIOS.size]),
        ]);
        let image = assemble_bios(MachineModel::PC9801RA, &by_digest).expect("assembled");
        assert_eq!(image.len(), BIOS_ROM_SIZE);
        assert!(image[0x00000..0x10000].iter().all(|&b| b == 0xFF));
        assert!(image[0x10000..0x18000].iter().all(|&b| b == 0xAA));
        assert!(image[0x18000..0x30000].iter().all(|&b| b == 0xBB));
    }

    #[test]
    fn assemble_vx_rearranges_biosrom() {
        // Fill each yll chip with a distinct byte so the ROM_COPY layout is
        // observable in the assembled image.
        let by_digest = map(&[
            (VX_YLL01.digest, vec![0x01; VX_YLL01.size]),
            (VX_YLL02.digest, vec![0x02; VX_YLL02.size]),
            (VX_YLL03.digest, vec![0x03; VX_YLL03.size]),
            (VX_YLL04.digest, vec![0x04; VX_YLL04.size]),
        ]);
        let image = assemble_bios(MachineModel::PC9801VX, &by_digest).expect("assembled");
        // biosrom[0x18000..0x20000] is the yll02/yll04 interleave -> ITF window.
        assert_eq!(image[0x10000], 0x02);
        assert_eq!(image[0x10001], 0x04);
        // biosrom[0x08000..0x10000] is the yll01/yll03 interleave -> BIOS window.
        assert_eq!(image[0x18000], 0x01);
        assert_eq!(image[0x18001], 0x03);
    }

    #[test]
    fn assemble_returns_none_when_chip_missing() {
        let by_digest = map(&[(RS_ITF.digest, vec![0xAA; RS_ITF.size])]);
        assert!(assemble_bios(MachineModel::PC9801RA, &by_digest).is_none());
    }

    /// Assembles every model's BIOS from a real MAME ROM directory and checks
    /// it against the known-good digest. Set `NEETAN_PC98_ROMS` to a directory
    /// holding the pc9801f/vm/vx/rs MAME sets to run it; otherwise it is a no-op.
    #[test]
    fn assembles_real_mame_sets() {
        let Ok(dir) = std::env::var("NEETAN_PC98_ROMS") else {
            return;
        };
        let dir = std::path::PathBuf::from(dir);
        for (model, expected) in [
            (MachineModel::PC9801F, ASSEMBLED_BIOS_F),
            (MachineModel::PC9801VM, ASSEMBLED_BIOS_VM),
            (MachineModel::PC9801VX, ASSEMBLED_BIOS_VX),
            (MachineModel::PC9801RA, ASSEMBLED_BIOS_RA),
        ] {
            let roms = load_rom_set(model, &dir).expect("scan succeeds");
            let bios = roms
                .bios
                .unwrap_or_else(|| panic!("no BIOS assembled for {model}"));
            assert_eq!(bios.len(), BIOS_ROM_SIZE);
            assert_eq!(
                blake3_hex(&bios),
                expected,
                "BIOS digest mismatch for {model}"
            );
        }
    }

    #[test]
    fn load_rom_set_ignores_stray_files() {
        let dir = std::env::temp_dir().join(format!("neetan_pc98_rom_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        std::fs::write(dir.join("stray.bin"), vec![0u8; 123]).expect("write stray");

        let roms = load_rom_set(MachineModel::PC9801VM, &dir).expect("scan succeeds");
        assert!(roms.bios.is_none());
        assert!(roms.font.is_none());
        assert!(roms.sound.is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
