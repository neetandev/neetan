//! FM Towns ROM set loading.
//!
//! ROMs are selected by content hash rather than file name: the loader scans
//! every file in the ROM directory, computes the BLAKE3 digest of each candidate
//! image, and matches it against a table of accepted digests per slot. Both dump
//! layouts are accepted:
//!
//! - The packed MAME `fmtownsiimxbios.m79` (2 MiB), which concatenates the five
//!   ROM images in flash-chip order (not memory-map order). It is sliced at the
//!   known offsets and each slice is treated as an individual candidate.
//! - The loose `FMT_SYS/DOS/FNT/DIC/F20.ROM` file set, matched directly.
//!
//! The 32-byte serial machine-identity ROM (`mytownsmx.rom`) is matched the same
//! way.

use std::{collections::HashMap, fmt, path::Path};

use crate::config::TownsModel;

/// Size of each ROM image, in bytes.
const DOS_SIZE: usize = 0x8_0000;
const FONT_SIZE: usize = 0x4_0000;
const SYSTEM_SIZE: usize = 0x4_0000;
const F20_SIZE: usize = 0x8_0000;
const DIC_SIZE: usize = 0x8_0000;
const SERIAL_SIZE: usize = 0x20;

/// Size and slice layout of the packed `fmtownsiimxbios.m79` image. The chunks
/// appear in flash-chip order, which differs from the memory-map order.
const PACKED_BIOS_SIZE: usize = 0x20_0000;
const PACKED_DOS_OFFSET: usize = 0x00_0000;
const PACKED_FONT_OFFSET: usize = 0x08_0000;
const PACKED_SYSTEM_OFFSET: usize = 0x0C_0000;
const PACKED_F20_OFFSET: usize = 0x10_0000;
const PACKED_DIC_OFFSET: usize = 0x18_0000;

/// One ROM slot: its human label, expected size, and the BLAKE3 digests that are
/// accepted as valid content for it. Multiple digests allow several known-good
/// dumps to satisfy the same slot.
struct RomSlot {
    label: &'static str,
    size: usize,
    accepted: &'static [&'static str],
}

/// The set of ROM slots for an FM Towns model.
struct RomTables {
    dos: RomSlot,
    font: RomSlot,
    system: RomSlot,
    f20: RomSlot,
    dictionary: RomSlot,
    serial: RomSlot,
}

const MX_TABLES: RomTables = RomTables {
    dos: RomSlot {
        label: "dos",
        size: DOS_SIZE,
        accepted: &["7f07a3c51743b51b02f347251057cfd1bfff9ff718b6c0fd3540e0da77c8a4da"],
    },
    font: RomSlot {
        label: "font",
        size: FONT_SIZE,
        accepted: &["0c365fb76a886c9f426893949d73390456ed6fc6c83f3109f699b0ded8b1ef24"],
    },
    system: RomSlot {
        label: "system",
        size: SYSTEM_SIZE,
        accepted: &["fba6e75d9727b6a192bf6b3e351f6ed7ae118162a0f71fea9c825a6b5f143022"],
    },
    f20: RomSlot {
        label: "f20",
        size: F20_SIZE,
        accepted: &["1dde131510456c9660c2217774853822674459412d8e6f98312fff0ee83ca9a7"],
    },
    dictionary: RomSlot {
        label: "dictionary",
        size: DIC_SIZE,
        accepted: &["0fbcbecb5b62c8fa4e9a60885f887b0a2cafd680a1174b0f7ddf57f49c65ab60"],
    },
    serial: RomSlot {
        label: "serial",
        size: SERIAL_SIZE,
        accepted: &["d5dc70e34d072889c28bed51ef3ccaac7f6f3fdd9e448d89297847247a901538"],
    },
};

const fn tables_for(model: TownsModel) -> &'static RomTables {
    match model {
        // No CX dump is available yet; the MX set is compatible (the SYSROM
        // layout is identical across the full 32-bit models).
        TownsModel::FmTownsIICx => &MX_TABLES,
        TownsModel::FmTownsIIMx => &MX_TABLES,
    }
}

/// Raw bytes of a successfully loaded and validated FM Towns ROM set.
#[derive(Debug)]
pub struct LoadedRoms {
    /// OS ROM (FMT_DOS, 512 KiB), mapped at 0xC2000000.
    pub dos: Vec<u8>,
    /// FONT ROM (256 KiB), mapped at 0xC2100000; also backs the ANK glyph banks.
    pub font: Vec<u8>,
    /// SYSTEM ROM (256 KiB), mapped at 0xFFFC0000 with its last 32 KiB shadowed
    /// at 0x000F8000.
    pub system: Vec<u8>,
    /// F20 font ROM (512 KiB), mapped at 0xC2180000.
    pub f20: Vec<u8>,
    /// DIC dictionary ROM (512 KiB), mapped at 0xC2080000.
    pub dictionary: Vec<u8>,
    /// Serial machine-identity ROM (32 bytes).
    pub serial: Vec<u8>,
}

/// Error encountered while loading an FM Towns ROM set.
#[derive(Debug)]
pub enum RomError {
    /// The ROM directory could not be scanned.
    Read { directory: String, message: String },
    /// No candidate image matched a slot's accepted digests.
    Missing {
        label: String,
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

fn missing_rom(slot: &RomSlot) -> RomError {
    RomError::Missing {
        label: slot.label.to_string(),
        accepted: slot.accepted.iter().map(|d| d.to_string()).collect(),
    }
}

/// Loads and validates the FM Towns ROM set for `model`.
///
/// Every file in `rom_dir` is scanned; a packed 2 MiB BIOS image is sliced into
/// its five component ROMs, and loose images are taken as-is. Each candidate is
/// matched against the accepted digests for every ROM slot, so file names do not
/// matter. All slots are required.
pub fn load_rom_set(model: TownsModel, rom_dir: &Path) -> Result<LoadedRoms, RomError> {
    load_rom_set_with_tables(rom_dir, tables_for(model))
}

fn load_rom_set_with_tables(rom_dir: &Path, tables: &RomTables) -> Result<LoadedRoms, RomError> {
    let by_digest = hash_directory(rom_dir, tables)?;

    let take = |slot: &RomSlot| -> Result<Vec<u8>, RomError> {
        for digest in slot.accepted {
            if let Some(data) = by_digest.get(*digest) {
                return Ok(data.clone());
            }
        }
        Err(missing_rom(slot))
    };

    Ok(LoadedRoms {
        dos: take(&tables.dos)?,
        font: take(&tables.font)?,
        system: take(&tables.system)?,
        f20: take(&tables.f20)?,
        dictionary: take(&tables.dictionary)?,
        serial: take(&tables.serial)?,
    })
}

/// Reads every regular file in `dir`, expands a packed BIOS image into its
/// component slices, and maps each candidate image's BLAKE3 digest to its bytes.
fn hash_directory(dir: &Path, tables: &RomTables) -> Result<HashMap<String, Vec<u8>>, RomError> {
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
        for candidate in candidate_images(data) {
            by_digest.entry(blake3_hex(&candidate)).or_insert(candidate);
        }
    }

    // Retain only candidates whose size matches some slot, to keep the map small.
    by_digest.retain(|_, data| is_known_rom_size(data.len(), tables));
    Ok(by_digest)
}

/// Expands a raw file into the candidate images it may contribute: a packed BIOS
/// image yields its five slices, any other file yields itself.
fn candidate_images(data: Vec<u8>) -> Vec<Vec<u8>> {
    if data.len() == PACKED_BIOS_SIZE {
        let slice = |offset: usize, size: usize| data[offset..offset + size].to_vec();
        vec![
            slice(PACKED_DOS_OFFSET, DOS_SIZE),
            slice(PACKED_FONT_OFFSET, FONT_SIZE),
            slice(PACKED_SYSTEM_OFFSET, SYSTEM_SIZE),
            slice(PACKED_F20_OFFSET, F20_SIZE),
            slice(PACKED_DIC_OFFSET, DIC_SIZE),
        ]
    } else {
        vec![data]
    }
}

fn is_known_rom_size(size: usize, tables: &RomTables) -> bool {
    [
        tables.dos.size,
        tables.font.size,
        tables.system.size,
        tables.f20.size,
        tables.dictionary.size,
        tables.serial.size,
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

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use super::*;

    static TEMP_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

    struct TempRomDir {
        path: PathBuf,
    }

    impl TempRomDir {
        fn new(name: &str) -> Self {
            let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "neetan_towns_rom_{name}_{}_{sequence}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).expect("create temp ROM dir");
            Self { path }
        }

        fn write(&self, file_name: &str, data: &[u8]) {
            std::fs::write(self.path.join(file_name), data).expect("write temp ROM file");
        }
    }

    impl Drop for TempRomDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    /// Builds a distinct fill pattern per slot so the sliced digests differ.
    fn pattern(size: usize, seed: u8) -> Vec<u8> {
        (0..size).map(|i| seed.wrapping_add(i as u8)).collect()
    }

    struct FakeRoms {
        dos: Vec<u8>,
        font: Vec<u8>,
        system: Vec<u8>,
        f20: Vec<u8>,
        dictionary: Vec<u8>,
        serial: Vec<u8>,
    }

    fn fake_roms() -> FakeRoms {
        FakeRoms {
            dos: pattern(DOS_SIZE, 0x10),
            font: pattern(FONT_SIZE, 0x20),
            system: pattern(SYSTEM_SIZE, 0x30),
            f20: pattern(F20_SIZE, 0x40),
            dictionary: pattern(DIC_SIZE, 0x50),
            serial: pattern(SERIAL_SIZE, 0x60),
        }
    }

    fn slot(label: &'static str, size: usize, digest: String) -> RomSlot {
        RomSlot {
            label,
            size,
            accepted: leak(digest),
        }
    }

    fn leak(digest: String) -> &'static [&'static str] {
        let digest: &'static str = Box::leak(digest.into_boxed_str());
        Box::leak(Box::new([digest]))
    }

    fn tables_from(roms: &FakeRoms) -> RomTables {
        RomTables {
            dos: slot("dos", DOS_SIZE, blake3_hex(&roms.dos)),
            font: slot("font", FONT_SIZE, blake3_hex(&roms.font)),
            system: slot("system", SYSTEM_SIZE, blake3_hex(&roms.system)),
            f20: slot("f20", F20_SIZE, blake3_hex(&roms.f20)),
            dictionary: slot("dictionary", DIC_SIZE, blake3_hex(&roms.dictionary)),
            serial: slot("serial", SERIAL_SIZE, blake3_hex(&roms.serial)),
        }
    }

    fn packed(roms: &FakeRoms) -> Vec<u8> {
        let mut image = vec![0u8; PACKED_BIOS_SIZE];
        image[PACKED_DOS_OFFSET..PACKED_DOS_OFFSET + DOS_SIZE].copy_from_slice(&roms.dos);
        image[PACKED_FONT_OFFSET..PACKED_FONT_OFFSET + FONT_SIZE].copy_from_slice(&roms.font);
        image[PACKED_SYSTEM_OFFSET..PACKED_SYSTEM_OFFSET + SYSTEM_SIZE]
            .copy_from_slice(&roms.system);
        image[PACKED_F20_OFFSET..PACKED_F20_OFFSET + F20_SIZE].copy_from_slice(&roms.f20);
        image[PACKED_DIC_OFFSET..PACKED_DIC_OFFSET + DIC_SIZE].copy_from_slice(&roms.dictionary);
        image
    }

    #[test]
    fn loads_from_packed_bios_and_serial() {
        let roms = fake_roms();
        let dir = TempRomDir::new("packed");
        dir.write("fmtownsiimxbios.m79", &packed(&roms));
        dir.write("mytownsmx.rom", &roms.serial);

        let tables = tables_from(&roms);
        let loaded = load_rom_set_with_tables(&dir.path, &tables).expect("load");
        assert_eq!(loaded.dos, roms.dos);
        assert_eq!(loaded.font, roms.font);
        assert_eq!(loaded.system, roms.system);
        assert_eq!(loaded.f20, roms.f20);
        assert_eq!(loaded.dictionary, roms.dictionary);
        assert_eq!(loaded.serial, roms.serial);
    }

    #[test]
    fn loads_from_loose_files() {
        let roms = fake_roms();
        let dir = TempRomDir::new("loose");
        dir.write("FMT_DOS.ROM", &roms.dos);
        dir.write("FMT_FNT.ROM", &roms.font);
        dir.write("FMT_SYS.ROM", &roms.system);
        dir.write("FMT_F20.ROM", &roms.f20);
        dir.write("FMT_DIC.ROM", &roms.dictionary);
        dir.write("mytownsmx.rom", &roms.serial);
        dir.write("readme.txt", b"stray");

        let tables = tables_from(&roms);
        let loaded = load_rom_set_with_tables(&dir.path, &tables).expect("load");
        assert_eq!(loaded.dos, roms.dos);
        assert_eq!(loaded.system, roms.system);
        assert_eq!(loaded.serial, roms.serial);
    }

    #[test]
    fn missing_serial_reports_the_serial_slot() {
        let roms = fake_roms();
        let dir = TempRomDir::new("noserial");
        dir.write("fmtownsiimxbios.m79", &packed(&roms));

        let tables = tables_from(&roms);
        match load_rom_set_with_tables(&dir.path, &tables) {
            Err(RomError::Missing { label, .. }) => assert_eq!(label, "serial"),
            other => panic!("expected Missing(serial), got {other:?}"),
        }
    }

    #[test]
    fn real_mx_table_carries_documented_digests() {
        assert_eq!(
            MX_TABLES.system.accepted,
            &["fba6e75d9727b6a192bf6b3e351f6ed7ae118162a0f71fea9c825a6b5f143022"]
        );
        assert_eq!(MX_TABLES.system.size, SYSTEM_SIZE);
        assert_eq!(MX_TABLES.serial.size, SERIAL_SIZE);
    }
}
