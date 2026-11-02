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
//!
//! The real firmware set needs every slot. The HLE firmware set reads no ROM
//! file. It runs the embedded HLE system ROM (`utils/bios_towns/sysrom.asm`),
//! the embedded HLE OS ROM image (`utils/dos_towns`) and the embedded font ROM
//! (`utils/bios_towns/fonts`). Its dictionary and F20 ROMs read `FFh` and its
//! serial ROM is generated.

use std::path::Path;

use rom_loader::{RomError, RomIndex, RomSlot, ScanOptions};

use crate::{config::TownsModel, serial_rom};

/// Size of each ROM image, in bytes.
const DOS_SIZE: usize = 0x8_0000;
const FONT_SIZE: usize = 0x4_0000;
const SYSTEM_SIZE: usize = 0x4_0000;
const F20_SIZE: usize = 0x8_0000;
const DIC_SIZE: usize = 0x8_0000;
const SERIAL_SIZE: usize = serial_rom::SERIAL_ROM_SIZE;

/// Content of the dictionary and F20 ROMs when the ROM folder has no image.
const EMPTY_ROM_FILL: u8 = 0xFF;

/// Embedded HLE system ROM stub (256 KiB), built from `utils/bios_towns/sysrom.asm`.
static HLE_SYSTEM_ROM: &[u8; SYSTEM_SIZE] = include_bytes!("../../../utils/bios_towns/sysrom.rom");

/// Embedded HLE OS ROM image (512 KiB), built by `utils/dos_towns`.
static HLE_OS_ROM: &[u8; DOS_SIZE] = include_bytes!("../../../utils/dos_towns/osrom.rom");

/// Embedded font ROM (256 KiB), built from the `.bit` files of `utils/bios_towns/fonts`.
static HLE_FONT_ROM: &[u8; FONT_SIZE] = include_bytes!("../../../utils/bios_towns/fonts/font.rom");

/// Size and slice layout of the packed `fmtownsiimxbios.m79` image. The chunks
/// appear in flash-chip order, which differs from the memory-map order.
const PACKED_BIOS_SIZE: usize = 0x20_0000;
const PACKED_DOS_OFFSET: usize = 0x00_0000;
const PACKED_FONT_OFFSET: usize = 0x08_0000;
const PACKED_SYSTEM_OFFSET: usize = 0x0C_0000;
const PACKED_F20_OFFSET: usize = 0x10_0000;
const PACKED_DIC_OFFSET: usize = 0x18_0000;

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
        // No base-model or CX dump is available yet; the MX set is compatible
        // (the SYSROM layout is identical across the full 32-bit models).
        TownsModel::FmTowns => &MX_TABLES,
        TownsModel::FmTownsIICx => &MX_TABLES,
        TownsModel::FmTownsIIMx => &MX_TABLES,
    }
}

/// System ROM and OS ROM of a loaded ROM set.
#[derive(Debug)]
pub enum TownsFirmware {
    /// The embedded HLE system ROM and the embedded HLE OS ROM image. NeetanDos
    /// serves as the DOS kernel of the `IO.SYS` that the machine boots.
    Hle,
    /// The real firmware of the ROM folder.
    Rom {
        /// SYSTEM ROM (256 KiB), mapped at 0xFFFC0000 with its last 32 KiB
        /// shadowed at 0x000F8000.
        system: Vec<u8>,
        /// OS ROM (FMT_DOS, 512 KiB), mapped at 0xC2000000.
        os: Vec<u8>,
    },
}

/// Raw bytes of a successfully loaded and validated FM Towns ROM set.
#[derive(Debug)]
pub struct LoadedRoms {
    /// System ROM and OS ROM.
    pub firmware: TownsFirmware,
    /// FONT ROM (256 KiB), mapped at 0xC2100000; also backs the ANK glyph banks.
    pub font: Vec<u8>,
    /// F20 font ROM (512 KiB), mapped at 0xC2180000.
    pub f20: Vec<u8>,
    /// DIC dictionary ROM (512 KiB), mapped at 0xC2080000.
    pub dictionary: Vec<u8>,
    /// Serial machine-identity ROM (32 bytes).
    pub serial: Vec<u8>,
}

impl LoadedRoms {
    /// Returns the HLE firmware ROM set of `model`. It holds only built-in images.
    pub fn builtin_hle_set(model: TownsModel) -> Self {
        Self {
            firmware: TownsFirmware::Hle,
            font: HLE_FONT_ROM.to_vec(),
            f20: vec![EMPTY_ROM_FILL; F20_SIZE],
            dictionary: vec![EMPTY_ROM_FILL; DIC_SIZE],
            serial: serial_rom::generate_serial_rom(model),
        }
    }

    /// Whether the set runs the HLE firmware.
    pub fn uses_hle_firmware(&self) -> bool {
        matches!(self.firmware, TownsFirmware::Hle)
    }
}

/// Returns the bytes of the embedded HLE system ROM.
pub(crate) fn hle_system_rom_image() -> &'static [u8] {
    HLE_SYSTEM_ROM
}

/// Returns the bytes of the embedded HLE OS ROM image.
pub(crate) fn hle_os_rom_image() -> &'static [u8] {
    HLE_OS_ROM
}

/// Loads and validates the real ROM set for `model`.
///
/// Every file in `rom_dir` is scanned; a packed 2 MiB BIOS image is sliced into
/// its five component ROMs, and loose images are taken as-is. Each candidate is
/// matched against the accepted digests for every ROM slot, so file names do not
/// matter. All slots are required.
pub fn load_rom_set(model: TownsModel, rom_dir: &Path) -> Result<LoadedRoms, RomError> {
    take_tables(
        &scan_with_sizes(rom_dir, &rom_sizes(tables_for(model)))?,
        tables_for(model),
    )
}

/// Scans `rom_dir` for the ROMs of every model. Each accepted file is read and hashed once.
pub fn scan_rom_directory(rom_dir: &Path) -> Result<RomIndex, RomError> {
    let mut sizes: Vec<usize> = [
        TownsModel::FmTowns,
        TownsModel::FmTownsIICx,
        TownsModel::FmTownsIIMx,
    ]
    .into_iter()
    .flat_map(|model| rom_sizes(tables_for(model)))
    .collect();
    sizes.sort_unstable();
    sizes.dedup();
    scan_with_sizes(rom_dir, &sizes)
}

/// Loads and validates the real ROM set of `model` from a scanned ROM directory.
pub fn load_rom_set_from_index(
    model: TownsModel,
    index: &RomIndex,
) -> Result<LoadedRoms, RomError> {
    take_tables(index, tables_for(model))
}

/// Scans `rom_dir` for files of the sizes `sizes`, with packed BIOS images split into their parts.
fn scan_with_sizes(rom_dir: &Path, sizes: &[usize]) -> Result<RomIndex, RomError> {
    let options = ScanOptions {
        accepted_sizes: sizes,
        subdirectory_depth: 0,
        expand: Some(expand_packed_bios),
    };
    Ok(rom_loader::scan_directory(rom_dir, &options)?)
}

/// Takes every slot of `tables` from a scanned ROM directory.
fn take_tables(index: &RomIndex, tables: &RomTables) -> Result<LoadedRoms, RomError> {
    let take = |slot: &RomSlot| index.take(slot);

    Ok(LoadedRoms {
        firmware: TownsFirmware::Rom {
            system: take(&tables.system)?,
            os: take(&tables.dos)?,
        },
        font: take(&tables.font)?,
        f20: take(&tables.f20)?,
        dictionary: take(&tables.dictionary)?,
        serial: take(&tables.serial)?,
    })
}

/// Expands a packed 2 MiB BIOS image into its five component ROMs. Any other
/// image is passed through unchanged.
fn expand_packed_bios(data: &[u8]) -> Vec<Vec<u8>> {
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
        vec![data.to_vec()]
    }
}

/// File sizes worth hashing when scanning a ROM directory.
fn rom_sizes(tables: &RomTables) -> [usize; 6] {
    [
        tables.dos.size,
        tables.font.size,
        tables.system.size,
        tables.f20.size,
        tables.dictionary.size,
        tables.serial.size,
    ]
}

#[cfg(test)]
mod tests {
    use common::temporary::TemporaryFolder;

    use super::*;

    /// Builds a distinct fill pattern per slot so the sliced digests differ.
    fn pattern(size: usize, seed: u8) -> Vec<u8> {
        (0..size).map(|i| seed.wrapping_add(i as u8)).collect()
    }

    /// Distinct ROM images of every slot.
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
            dos: slot("dos", DOS_SIZE, rom_loader::blake3_hex(&roms.dos)),
            font: slot("font", FONT_SIZE, rom_loader::blake3_hex(&roms.font)),
            system: slot("system", SYSTEM_SIZE, rom_loader::blake3_hex(&roms.system)),
            f20: slot("f20", F20_SIZE, rom_loader::blake3_hex(&roms.f20)),
            dictionary: slot(
                "dictionary",
                DIC_SIZE,
                rom_loader::blake3_hex(&roms.dictionary),
            ),
            serial: slot("serial", SERIAL_SIZE, rom_loader::blake3_hex(&roms.serial)),
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

    /// Loads the real ROM set of `tables` from `dir`.
    fn load_real(dir: &TemporaryFolder, tables: &RomTables) -> Result<LoadedRoms, RomError> {
        take_tables(&scan_with_sizes(dir.path(), &rom_sizes(tables))?, tables)
    }

    /// Returns the system ROM and the OS ROM of a real firmware set.
    fn real_firmware(roms: &LoadedRoms) -> (&[u8], &[u8]) {
        match &roms.firmware {
            TownsFirmware::Rom { system, os } => (system, os),
            TownsFirmware::Hle => panic!("expected the real firmware"),
        }
    }

    #[test]
    fn loads_from_packed_bios_and_serial() {
        let roms = fake_roms();
        let dir = TemporaryFolder::new("towns_rom_packed");
        dir.write_file("fmtownsiimxbios.m79", &packed(&roms));
        dir.write_file("mytownsmx.rom", &roms.serial);

        let loaded = load_real(&dir, &tables_from(&roms)).expect("load");
        assert_eq!(real_firmware(&loaded), (&roms.system[..], &roms.dos[..]));
        assert_eq!(loaded.font, roms.font);
        assert_eq!(loaded.f20, roms.f20);
        assert_eq!(loaded.dictionary, roms.dictionary);
        assert_eq!(loaded.serial, roms.serial);
    }

    #[test]
    fn loads_from_loose_files() {
        let roms = fake_roms();
        let dir = TemporaryFolder::new("towns_rom_loose");
        dir.write_file("FMT_DOS.ROM", &roms.dos);
        dir.write_file("FMT_FNT.ROM", &roms.font);
        dir.write_file("FMT_SYS.ROM", &roms.system);
        dir.write_file("FMT_F20.ROM", &roms.f20);
        dir.write_file("FMT_DIC.ROM", &roms.dictionary);
        dir.write_file("mytownsmx.rom", &roms.serial);
        dir.write_file("readme.txt", b"stray");

        let loaded = load_real(&dir, &tables_from(&roms)).expect("load");
        assert_eq!(real_firmware(&loaded), (&roms.system[..], &roms.dos[..]));
        assert_eq!(loaded.serial, roms.serial);
    }

    #[test]
    fn missing_serial_reports_the_serial_slot() {
        let roms = fake_roms();
        let dir = TemporaryFolder::new("towns_rom_noserial");
        dir.write_file("fmtownsiimxbios.m79", &packed(&roms));

        match load_real(&dir, &tables_from(&roms)) {
            Err(RomError::Missing { label, .. }) => assert_eq!(label, "serial"),
            other => panic!("expected Missing(serial), got {other:?}"),
        }
    }

    #[test]
    fn real_set_requires_the_system_rom() {
        let roms = fake_roms();
        let dir = TemporaryFolder::new("towns_rom_nosystem");
        dir.write_file("FMT_DOS.ROM", &roms.dos);
        dir.write_file("FMT_FNT.ROM", &roms.font);
        dir.write_file("FMT_F20.ROM", &roms.f20);
        dir.write_file("FMT_DIC.ROM", &roms.dictionary);
        dir.write_file("mytownsmx.rom", &roms.serial);

        match load_real(&dir, &tables_from(&roms)) {
            Err(RomError::Missing { label, .. }) => assert_eq!(label, "system"),
            other => panic!("expected Missing(system), got {other:?}"),
        }
    }

    #[test]
    fn builtin_hle_set_holds_only_builtin_images() {
        for model in [
            TownsModel::FmTowns,
            TownsModel::FmTownsIICx,
            TownsModel::FmTownsIIMx,
        ] {
            let loaded = LoadedRoms::builtin_hle_set(model);
            assert!(loaded.uses_hle_firmware());
            assert_eq!(loaded.font, HLE_FONT_ROM);
            assert_eq!(loaded.f20, vec![EMPTY_ROM_FILL; F20_SIZE]);
            assert_eq!(loaded.dictionary, vec![EMPTY_ROM_FILL; DIC_SIZE]);
            assert_eq!(loaded.serial, serial_rom::generate_serial_rom(model));
        }
    }

    #[test]
    fn builtin_font_rom_carries_the_tripwire_glyph() {
        /// Offset of the tripwire cell (index 1FFFh) in the font ROM.
        const TRIPWIRE_OFFSET: usize = 0x3_FFE0;
        /// The Neetan glyph of the tripwire cell: an N in a frame.
        const TRIPWIRE_GLYPH: [u8; 32] = [
            0xFF, 0xFF, 0x80, 0x01, 0x80, 0x01, 0x9C, 0x19, 0x9E, 0x19, 0x9B, 0x19, 0x9B, 0x19,
            0x99, 0x99, 0x98, 0xD9, 0x98, 0x79, 0x98, 0x79, 0x98, 0x39, 0x98, 0x19, 0x80, 0x01,
            0x80, 0x01, 0xFF, 0xFF,
        ];
        let font = &LoadedRoms::builtin_hle_set(TownsModel::FmTownsIIMx).font;
        assert_eq!(font.len(), FONT_SIZE);
        assert_eq!(
            font[TRIPWIRE_OFFSET..TRIPWIRE_OFFSET + TRIPWIRE_GLYPH.len()],
            TRIPWIRE_GLYPH
        );
    }

    #[test]
    fn hle_os_rom_is_the_dos_towns_image() {
        let image = hle_os_rom_image();
        assert_eq!(image.len(), DOS_SIZE);
        assert_eq!(&image[..5], b"IPL4\xCB");
    }

    #[test]
    fn hle_system_rom_carries_entry_table_and_machine_id() {
        /// Offset of the FMR family machine ID byte (physical FFFF5h).
        const MACHINE_ID_OFFSET: usize = 0x3_FFF5;
        /// FMR family machine ID of every FM Towns.
        const FM_TOWNS_MACHINE_ID: u8 = 0x51;

        let image = hle_system_rom_image();
        assert_eq!(image.len(), SYSTEM_SIZE);
        assert_eq!(image[MACHINE_ID_OFFSET], FM_TOWNS_MACHINE_ID);
        // Reset vector and the eight far jumps of the FFFB entry table.
        assert_eq!(image[0x3_FFF0], 0xEA);
        for entry in 0..8 {
            assert_eq!(image[0x3_FFB0 + entry * 5], 0xEA);
        }
        // Below the code segment the image holds the 12-dot font area at
        // 00000h-1AFFFh and the system icons at 28000h-2FFFFh.
        let ank_12_letter_a = 0x41 * 12;
        assert!(
            image[ank_12_letter_a..ank_12_letter_a + 12]
                .iter()
                .any(|&byte| byte != 0)
        );
        assert!(image[0x1_B000..0x2_8000].iter().all(|&byte| byte == 0xFF));
        assert!(image[0x2_8000..0x3_0000].iter().any(|&byte| byte != 0xFF));
        assert!(image[0x3_0000..0x3_C000].iter().all(|&byte| byte == 0xFF));
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
