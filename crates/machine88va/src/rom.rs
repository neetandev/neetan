//! PC-88VA2 ROM set loading.
//!
//! ROMs are selected by content hash rather than file name: the loader scans
//! every file in the ROM directory, computes its BLAKE3 digest, and matches it
//! against a table of accepted digests per slot. This way any dump layout works
//! regardless of how the files are named, and stray files are ignored.

use std::{collections::HashMap, fmt, path::Path};

const ROM00_SIZE: usize = 0x8_0000;
const ROM08_SIZE: usize = 0x2_0000;
const ROM1_SIZE: usize = 0x2_0000;
const FONT_SIZE: usize = 0x5_0000;
const DICTIONARY_SIZE: usize = 0x8_0000;
const SUBSYS_SIZE: usize = 0x2000;

/// One ROM slot: its human label, expected size, and the BLAKE3 digests that
/// are accepted as valid content for it. Multiple digests allow several known
/// good dumps to satisfy the same slot.
struct RomSlot {
    label: &'static str,
    size: usize,
    accepted: &'static [&'static str],
}

/// The set of ROM slots for the PC-88VA2.
struct RomTables {
    rom00: RomSlot,
    rom08: RomSlot,
    rom1: RomSlot,
    font: RomSlot,
    dictionary: RomSlot,
    subsys: RomSlot,
}

const VA_TABLES: RomTables = RomTables {
    rom00: RomSlot {
        label: "rom00",
        size: ROM00_SIZE,
        accepted: &["bba5011412fb266b3c15ff08d2508716ba2ac54fec3aa172b59e441486807eab"],
    },
    rom08: RomSlot {
        label: "rom08",
        size: ROM08_SIZE,
        accepted: &["4cdf3da9a1423e874f9618a8d8859107fa5e3d20a91f4dcf908e042763c41bbb"],
    },
    rom1: RomSlot {
        label: "rom1",
        size: ROM1_SIZE,
        accepted: &["1239bf390d444ff205f70c700527cb50bc90107904050fa8713a415a17bf0e42"],
    },
    font: RomSlot {
        label: "font",
        size: FONT_SIZE,
        accepted: &["b47ec9f55ff199ac71f453385aec0f370afbb958fd47ad9bb5161bdf4e2bb3ee"],
    },
    dictionary: RomSlot {
        label: "dictionary",
        size: DICTIONARY_SIZE,
        accepted: &["21fcd88c97b881e55f015f22d62002022189572e171f1c5e485b751c84379b30"],
    },
    subsys: RomSlot {
        label: "subsys",
        size: SUBSYS_SIZE,
        accepted: &["531ab2aa2c7d7c4deb2ddd8303c6637ea7e273648825fb51e17c8660d7496565"],
    },
};

/// Raw bytes of a successfully loaded and validated PC-88VA2 ROM set.
#[derive(Debug)]
pub struct LoadedRoms {
    /// ROM0 low image (varom00, 512 KiB).
    pub rom00: Vec<u8>,
    /// ROM0 high image (varom08, 128 KiB).
    pub rom08: Vec<u8>,
    /// ROM1 image (varom1, 128 KiB).
    pub rom1: Vec<u8>,
    /// Kanji/font ROM (320 KiB).
    pub font: Vec<u8>,
    /// Dictionary (jisyo) ROM (512 KiB).
    pub dictionary: Vec<u8>,
    /// Floppy sub-CPU (Z80) ROM (8 KiB), driving the sub-CPU floppy path.
    pub subsys: Vec<u8>,
}

/// Error encountered while loading a PC-88VA2 ROM set.
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

fn missing_rom(slot: &RomSlot) -> RomError {
    RomError::Missing {
        label: slot.label.to_string(),
        accepted: slot.accepted.iter().map(|d| d.to_string()).collect(),
    }
}

/// Loads and validates the PC-88VA2 ROM set.
///
/// Every file in `rom_dir` is hashed and matched against the accepted digests
/// for each ROM slot, so the dump's file names do not matter. All slots are
/// required: rom00, rom08, rom1, font, dictionary, and the floppy sub-CPU ROM
/// (subsys), which drives the sub-CPU floppy path.
pub fn load_rom_set(rom_dir: &Path) -> Result<LoadedRoms, RomError> {
    load_rom_set_with_tables(rom_dir, &VA_TABLES)
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
    let rom00 = take(&tables.rom00)?;
    let rom08 = take(&tables.rom08)?;
    let rom1 = take(&tables.rom1)?;
    let font = take(&tables.font)?;
    let dictionary = take(&tables.dictionary)?;
    let subsys = take(&tables.subsys)?;

    Ok(LoadedRoms {
        rom00,
        rom08,
        rom1,
        font,
        dictionary,
        subsys,
    })
}

/// Reads every regular file in `dir` whose size matches a known ROM slot and
/// maps its BLAKE3 digest to its contents.
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
        if !is_known_rom_size(data.len(), tables) {
            continue;
        }
        by_digest.entry(blake3_hex(&data)).or_insert(data);
    }
    Ok(by_digest)
}

fn is_known_rom_size(size: usize, tables: &RomTables) -> bool {
    [
        tables.rom00.size,
        tables.rom08.size,
        tables.rom1.size,
        tables.font.size,
        tables.dictionary.size,
        tables.subsys.size,
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
                "neetan_va_rom_{name}_{}_{sequence}",
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

    fn fake(size: usize, fill: u8) -> Vec<u8> {
        vec![fill; size]
    }

    fn slot(label: &'static str, size: usize, accepted: &'static [&'static str]) -> RomSlot {
        RomSlot {
            label,
            size,
            accepted,
        }
    }

    type FakeRomSet = (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>);

    fn full_rom_set() -> FakeRomSet {
        (
            fake(ROM00_SIZE, 0x01),
            fake(ROM08_SIZE, 0x02),
            fake(ROM1_SIZE, 0x03),
            fake(FONT_SIZE, 0x04),
            fake(DICTIONARY_SIZE, 0x05),
            fake(SUBSYS_SIZE, 0x06),
        )
    }

    fn tables_from_bytes(
        rom00: &[u8],
        rom08: &[u8],
        rom1: &[u8],
        font: &[u8],
        dictionary: &[u8],
        subsys: &[u8],
    ) -> RomTables {
        RomTables {
            rom00: slot("rom00", ROM00_SIZE, leak(blake3_hex(rom00))),
            rom08: slot("rom08", ROM08_SIZE, leak(blake3_hex(rom08))),
            rom1: slot("rom1", ROM1_SIZE, leak(blake3_hex(rom1))),
            font: slot("font", FONT_SIZE, leak(blake3_hex(font))),
            dictionary: slot("dictionary", DICTIONARY_SIZE, leak(blake3_hex(dictionary))),
            subsys: slot("subsys", SUBSYS_SIZE, leak(blake3_hex(subsys))),
        }
    }

    fn leak(digest: String) -> &'static [&'static str] {
        let digest: &'static str = Box::leak(digest.into_boxed_str());
        Box::leak(Box::new([digest]))
    }

    #[test]
    fn matches_slots_by_digest_and_ignores_stray_files() {
        let (rom00, rom08, rom1, font, dictionary, subsys) = full_rom_set();
        let dir = TempRomDir::new("match");
        dir.write("a.bin", &rom00);
        dir.write("b.bin", &rom08);
        dir.write("c.bin", &rom1);
        dir.write("d.bin", &font);
        dir.write("e.bin", &dictionary);
        dir.write("f.bin", &subsys);
        dir.write("readme.txt", b"unknown size stray file");
        dir.write("g.bin", &fake(ROM1_SIZE, 0xEE));

        let tables = tables_from_bytes(&rom00, &rom08, &rom1, &font, &dictionary, &subsys);
        let loaded = load_rom_set_with_tables(&dir.path, &tables).expect("load");

        assert_eq!(loaded.rom00, rom00);
        assert_eq!(loaded.rom08, rom08);
        assert_eq!(loaded.rom1, rom1);
        assert_eq!(loaded.font, font);
        assert_eq!(loaded.dictionary, dictionary);
        assert_eq!(loaded.subsys, subsys);
    }

    #[test]
    fn missing_required_file_reports_the_right_slot() {
        let (rom00, rom08, rom1, font, dictionary, subsys) = full_rom_set();
        let dir = TempRomDir::new("missing");
        dir.write("a.bin", &rom00);
        dir.write("b.bin", &rom08);
        dir.write("c.bin", &rom1);
        dir.write("e.bin", &dictionary);
        dir.write("f.bin", &subsys);

        let tables = tables_from_bytes(&rom00, &rom08, &rom1, &font, &dictionary, &subsys);
        match load_rom_set_with_tables(&dir.path, &tables) {
            Err(RomError::Missing { label, .. }) => assert_eq!(label, "font"),
            other => panic!("expected Missing(font), got {other:?}"),
        }
    }

    #[test]
    fn missing_subsys_reports_the_subsys_slot() {
        let (rom00, rom08, rom1, font, dictionary, subsys) = full_rom_set();
        let dir = TempRomDir::new("nosubsys");
        dir.write("a.bin", &rom00);
        dir.write("b.bin", &rom08);
        dir.write("c.bin", &rom1);
        dir.write("d.bin", &font);
        dir.write("e.bin", &dictionary);

        let tables = tables_from_bytes(&rom00, &rom08, &rom1, &font, &dictionary, &subsys);
        match load_rom_set_with_tables(&dir.path, &tables) {
            Err(RomError::Missing { label, .. }) => assert_eq!(label, "subsys"),
            other => panic!("expected Missing(subsys), got {other:?}"),
        }
    }

    #[test]
    fn wrong_size_file_is_skipped_and_reports_missing() {
        let (rom00, rom08, rom1, font, dictionary, subsys) = full_rom_set();
        let dir = TempRomDir::new("wrongsize");
        dir.write("a.bin", &rom00);
        dir.write("b.bin", &rom08);
        dir.write("c.bin", &rom1);
        dir.write("d.bin", &fake(FONT_SIZE - 1, 0x04));
        dir.write("e.bin", &dictionary);

        let tables = tables_from_bytes(&rom00, &rom08, &rom1, &font, &dictionary, &subsys);
        match load_rom_set_with_tables(&dir.path, &tables) {
            Err(RomError::Missing { label, .. }) => assert_eq!(label, "font"),
            other => panic!("expected Missing(font), got {other:?}"),
        }
    }

    #[test]
    fn distinct_tables_select_matching_contents() {
        let dir = TempRomDir::new("twomodels");
        let rom00_a = fake(ROM00_SIZE, 0x11);
        let rom00_b = fake(ROM00_SIZE, 0x22);
        let rom08 = fake(ROM08_SIZE, 0x02);
        let rom1 = fake(ROM1_SIZE, 0x03);
        let font = fake(FONT_SIZE, 0x04);
        let dictionary = fake(DICTIONARY_SIZE, 0x05);
        let subsys = fake(SUBSYS_SIZE, 0x06);
        dir.write("rom00_a.bin", &rom00_a);
        dir.write("rom00_b.bin", &rom00_b);
        dir.write("b.bin", &rom08);
        dir.write("c.bin", &rom1);
        dir.write("d.bin", &font);
        dir.write("e.bin", &dictionary);
        dir.write("f.bin", &subsys);

        let tables_a = tables_from_bytes(&rom00_a, &rom08, &rom1, &font, &dictionary, &subsys);
        let tables_b = tables_from_bytes(&rom00_b, &rom08, &rom1, &font, &dictionary, &subsys);

        assert_eq!(
            load_rom_set_with_tables(&dir.path, &tables_a)
                .expect("load a")
                .rom00,
            rom00_a
        );
        assert_eq!(
            load_rom_set_with_tables(&dir.path, &tables_b)
                .expect("load b")
                .rom00,
            rom00_b
        );
    }

    #[test]
    fn real_model_tables_carry_documented_digests() {
        assert_eq!(
            VA_TABLES.rom00.accepted,
            &["bba5011412fb266b3c15ff08d2508716ba2ac54fec3aa172b59e441486807eab"]
        );
        assert_eq!(VA_TABLES.rom1.size, ROM1_SIZE);
    }
}
