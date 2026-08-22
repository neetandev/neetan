//! MSX cartridge layouts, mapper detection and persistent storage.

mod known;

use std::{
    fmt,
    fs::File,
    io::Write,
    path::{Path, PathBuf},
};

use device::{
    eight_bit_dac::EightBitDac,
    opn_fm::{OpnFm, Ym2413},
    scc::{SccPlus, StandardScc},
};

/// Size of one 8 KiB cartridge bank.
const BANK_SIZE_8K: usize = 0x2000;
/// Size of one 16 KiB cartridge bank.
const BANK_SIZE_16K: usize = 0x4000;
/// First address of the normal cartridge area.
const CARTRIDGE_START: u16 = 0x4000;
/// First address of cartridge page 2.
const PAGE_2_START: u16 = 0x8000;
/// Address after the normal cartridge area.
const CARTRIDGE_END: u32 = 0xC000;
/// Maximum Konami cartridge size.
const KONAMI_MAX_SIZE: usize = 0x40_000;
/// Maximum Konami SCC cartridge size.
const KONAMI_SCC_MAX_SIZE: usize = 0x80_000;
/// Maximum ASCII8 cartridge size.
const ASCII8_MAX_SIZE: usize = 0x20_0000;
/// Maximum ASCII16 cartridge size.
const ASCII16_MAX_SIZE: usize = 0x40_0000;
/// Minimum score accepted by automatic mapper detection.
const MINIMUM_MAPPER_SCORE: u16 = 2;
/// Size of the smallest cartridge SRAM.
const SRAM_SIZE_2K: usize = 0x800;
/// Size of the standard cartridge SRAM.
const SRAM_SIZE_8K: usize = 0x2000;
/// Size of the large ASCII8 cartridge SRAM.
const SRAM_SIZE_32K: usize = 0x8000;
/// Size of Game Master 2 ROM images.
const GAME_MASTER_2_ROM_SIZE: usize = 0x20_000;
/// Size of R-Type ROM images.
const R_TYPE_ROM_SIZE: usize = 0x60_000;
/// Size of Halnote ROM images.
const HALNOTE_ROM_SIZE: usize = 0x10_0000;

/// Supported MSX cartridge layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CartridgeMapper {
    /// Plain 8 KiB ROM.
    Plain8,
    /// Plain 16 KiB ROM.
    Plain16,
    /// Plain 32 KiB ROM.
    Plain32,
    /// ROM repeated throughout the cartridge slot.
    Mirrored,
    /// ROM visible only in CPU page 2.
    Page2Only,
    /// Konami 8 KiB bank mapper without SCC.
    Konami,
    /// Konami 8 KiB bank mapper with SCC.
    KonamiScc,
    /// ASCII 8 KiB bank mapper.
    Ascii8,
    /// ASCII 16 KiB bank mapper.
    Ascii16,
    /// Generic 8 KiB bank mapper.
    Generic8,
    /// ASCII8 mapper with 2 KiB SRAM.
    Ascii8Sram2,
    /// ASCII8 mapper with 8 KiB SRAM.
    Ascii8Sram8,
    /// ASCII8 mapper with 32 KiB SRAM.
    Ascii8Sram32,
    /// ASCII16 mapper with 2 KiB SRAM.
    Ascii16Sram2,
    /// ASCII16 mapper with 8 KiB SRAM.
    Ascii16Sram8,
    /// Koei ASCII8 mapper with 8 KiB SRAM.
    KoeiSram8,
    /// Koei ASCII8 mapper with 32 KiB SRAM.
    KoeiSram32,
    /// Wizardry ASCII8 mapper with 8 KiB SRAM.
    Wizardry,
    /// Konami Game Master 2 mapper.
    GameMaster2,
    /// R-Type cartridge mapper.
    RType,
    /// Cross Blaim cartridge mapper.
    CrossBlaim,
    /// Harry Fox cartridge mapper.
    HarryFox,
    /// Super Lode Runner cartridge mapper.
    SuperLodeRunner,
    /// Super Swangi cartridge mapper.
    SuperSwangi,
    /// Majutsushi cartridge with an eight-bit DAC.
    Majutsushi,
    /// Konami Synthesizer cartridge with an eight-bit DAC.
    Synthesizer,
    /// Panasonic FM-PAC cartridge.
    FmPac,
    /// MSX-DOS2 cartridge with one banked 16 KiB window.
    MsxDos2,
    /// Halnote cartridge mapper.
    Halnote,
    /// MSX-Write cartridge mapper.
    MsxWrite,
    /// Nettou Yakyuu cartridge mapper.
    NettouYakyuu,
    /// PlayBall cartridge mapper.
    PlayBall,
    /// Generic SCC+ sound cartridge with 128 KiB RAM.
    SccPlus,
    /// Snatcher SCC+ sound cartridge.
    Snatcher,
    /// SD Snatcher SCC+ sound cartridge.
    SdSnatcher,
}

impl fmt::Display for CartridgeMapper {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Plain8 => "plain 8 KiB",
            Self::Plain16 => "plain 16 KiB",
            Self::Plain32 => "plain 32 KiB",
            Self::Mirrored => "mirrored",
            Self::Page2Only => "page 2 only",
            Self::Konami => "Konami",
            Self::KonamiScc => "Konami SCC",
            Self::Ascii8 => "ASCII8",
            Self::Ascii16 => "ASCII16",
            Self::Generic8 => "generic 8 KiB",
            Self::Ascii8Sram2 => "ASCII8 with 2 KiB SRAM",
            Self::Ascii8Sram8 => "ASCII8 with 8 KiB SRAM",
            Self::Ascii8Sram32 => "ASCII8 with 32 KiB SRAM",
            Self::Ascii16Sram2 => "ASCII16 with 2 KiB SRAM",
            Self::Ascii16Sram8 => "ASCII16 with 8 KiB SRAM",
            Self::KoeiSram8 => "Koei with 8 KiB SRAM",
            Self::KoeiSram32 => "Koei with 32 KiB SRAM",
            Self::Wizardry => "Wizardry",
            Self::GameMaster2 => "Game Master 2",
            Self::RType => "R-Type",
            Self::CrossBlaim => "Cross Blaim",
            Self::HarryFox => "Harry Fox",
            Self::SuperLodeRunner => "Super Lode Runner",
            Self::SuperSwangi => "Super Swangi",
            Self::Majutsushi => "Majutsushi",
            Self::Synthesizer => "Synthesizer",
            Self::FmPac => "FM-PAC",
            Self::MsxDos2 => "MSX-DOS2",
            Self::Halnote => "Halnote",
            Self::MsxWrite => "MSX-Write",
            Self::NettouYakyuu => "Nettou Yakyuu",
            Self::PlayBall => "PlayBall",
            Self::SccPlus => "SCC+",
            Self::Snatcher => "Snatcher SCC+",
            Self::SdSnatcher => "SD Snatcher SCC+",
        })
    }
}

/// Source used to identify a cartridge mapper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapperIdentification {
    /// Mapper selected by an exact BLAKE3 entry.
    KnownHash,
    /// Plain layout selected from the ROM header and size.
    Header,
    /// Mapper selected by conservative write-address scoring.
    Heuristic,
    /// Mapper selected explicitly by the caller.
    Explicit,
}

/// Result of loading and identifying a cartridge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CartridgeLoadInfo {
    /// BLAKE3 digest of the ROM image.
    pub digest: String,
    /// Selected mapper.
    pub mapper: CartridgeMapper,
    /// Identification source.
    pub identification: MapperIdentification,
    /// Warning that should be shown to the user.
    pub warning: Option<String>,
}

/// Failure while loading or changing a cartridge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CartridgeError {
    /// The cartridge connector number is not zero or one.
    InvalidSlot {
        /// Requested connector number.
        slot: usize,
    },
    /// The image size is invalid for the selected layout.
    UnsupportedSize {
        /// Supplied image size.
        size: usize,
    },
    /// An MSX-DOS2 cartridge uses an unknown bank-selection range.
    UnsupportedMsxDos2Control {
        /// Value stored in ROM header byte 0x94.
        value: u8,
    },
    /// Mapper scoring did not produce one reliable result.
    AmbiguousMapper {
        /// BLAKE3 digest of the image.
        digest: String,
        /// Human-readable candidate scores.
        scores: String,
    },
    /// Persistent data has an unexpected size.
    InvalidSaveSize {
        /// Save file path.
        path: PathBuf,
        /// Required number of bytes.
        expected: usize,
        /// Actual number of bytes.
        actual: usize,
    },
    /// Persistent storage could not be read or written.
    Persistence {
        /// Failed path.
        path: PathBuf,
        /// Filesystem error.
        message: String,
    },
}

impl fmt::Display for CartridgeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSlot { slot } => {
                write!(formatter, "MSX cartridge slot {slot} does not exist")
            }
            Self::UnsupportedSize { size } => {
                write!(formatter, "unsupported MSX cartridge size {size} bytes")
            }
            Self::UnsupportedMsxDos2Control { value } => {
                write!(
                    formatter,
                    "unsupported MSX-DOS2 cartridge control value {value:#04X}"
                )
            }
            Self::AmbiguousMapper { digest, scores } => {
                write!(
                    formatter,
                    "cannot identify MSX cartridge {digest}; mapper scores: {scores}"
                )
            }
            Self::InvalidSaveSize {
                path,
                expected,
                actual,
            } => write!(
                formatter,
                "save file {} is {actual} bytes, expected {expected}",
                path.display()
            ),
            Self::Persistence { path, message } => {
                write!(
                    formatter,
                    "persistent storage {}: {message}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for CartridgeError {}

/// File-backed battery data owned by one cartridge.
#[derive(Debug, Clone)]
pub struct CartridgePersistence {
    bytes: Vec<u8>,
    dirty: bool,
    path: Option<PathBuf>,
}

impl CartridgePersistence {
    /// Loads battery data or creates erased in-memory contents.
    pub fn load(
        size: usize,
        erased_value: u8,
        rom_path: Option<&Path>,
    ) -> Result<Self, CartridgeError> {
        let path = rom_path.map(save_path_for_rom);
        let bytes = if let Some(path) = path.as_ref().filter(|path| path.exists()) {
            let bytes = std::fs::read(path).map_err(|error| CartridgeError::Persistence {
                path: path.clone(),
                message: error.to_string(),
            })?;
            if bytes.len() != size {
                return Err(CartridgeError::InvalidSaveSize {
                    path: path.clone(),
                    expected: size,
                    actual: bytes.len(),
                });
            }
            bytes
        } else {
            vec![erased_value; size]
        };
        Ok(Self {
            bytes,
            dirty: false,
            path,
        })
    }

    /// Returns one persistent byte.
    pub fn read(&self, offset: usize) -> Option<u8> {
        self.bytes.get(offset).copied()
    }

    /// Changes one persistent byte and marks the region dirty.
    pub fn write(&mut self, offset: usize, value: u8) -> bool {
        let Some(byte) = self.bytes.get_mut(offset) else {
            return false;
        };
        if *byte != value {
            *byte = value;
            self.dirty = true;
        }
        true
    }

    /// Whether data changed since the last successful flush.
    pub const fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Flushes dirty data to the sibling `.sav` file.
    pub fn flush(&mut self) -> Result<(), CartridgeError> {
        if !self.dirty {
            return Ok(());
        }
        let Some(path) = self.path.as_ref() else {
            self.dirty = false;
            return Ok(());
        };
        let temporary_path = temporary_path_for_save(path);
        let result = (|| -> std::io::Result<()> {
            let mut file = File::create(&temporary_path)?;
            file.write_all(&self.bytes)?;
            file.sync_all()?;
            std::fs::rename(&temporary_path, path)
        })();
        match result {
            Ok(()) => {
                self.dirty = false;
                Ok(())
            }
            Err(error) => Err(CartridgeError::Persistence {
                path: path.clone(),
                message: error.to_string(),
            }),
        }
    }
}

/// Returns the `.sav` path associated with a cartridge ROM.
pub fn save_path_for_rom(rom_path: &Path) -> PathBuf {
    rom_path.with_extension("sav")
}

/// Selects a later Konami sound cartridge from a verified disk BLAKE3.
pub fn sound_cartridge_for_disk_blake3(digest: &str) -> Option<CartridgeMapper> {
    if SNATCHER_DISK_BLAKE3.contains(&digest) {
        Some(CartridgeMapper::Snatcher)
    } else if SD_SNATCHER_DISK_BLAKE3.contains(&digest) {
        Some(CartridgeMapper::SdSnatcher)
    } else {
        None
    }
}

/// Known Snatcher disk BLAKE3 digests.
const SNATCHER_DISK_BLAKE3: &[&str] = &[
    "3ea7ffd9039e38390648d062c40f9e58884604c189d155ea9101e95150ad7107",
    "f8bfe8fba0c74a509ea7a4c23ec75cfb6ac2d69e2e37dca1533f3b0a17ad6119",
    "7d8d354cd03d67cbc4b0e2efdec26e0a00b95726ce8c93f9efab96864a73d7a9",
    "c2724976f5e0f685d7ca6108fe8f74721ebdd3734567fd2ada7481851a417a4d",
    "0ca708cc5d63268af8529706cecd66a6d2b0a019c5c020d848c09eba34c748bd",
    "83f73a794510ec837df1c99e108a2eb0a2fd135ef5058c2b833e952de69472c7",
    "43f5384c0219a2c10735ef87773d59cd180e6e96cbcd62de0c275aa3c5ff038a",
    "4d9f9fc41ed58a9cf27b08ecdc0d57f26dc33c0898b7e7f5a655d6f07509e3ae",
    "7205d2595ce3e40cead0e890bb6d0f0833dc21de6b95865d612409e9806b1587",
    "efe439763ff3142ea5b7bd6778bcbb68d888c014c9c544745c5e0edb00ea15ea",
];
/// Known SD Snatcher disk BLAKE3 digests.
const SD_SNATCHER_DISK_BLAKE3: &[&str] = &[
    "203571ffcf8d7e2ac6998191f57635b391308e270f53bf25e05174f56e8b982f",
    "d64228b0366b5472b6d88a235a6c6d321367d28a6f62c1fa93fb297684c47022",
    "46270090e01e29f57fb985eda72645a1bc91bbece4189438dc0cf471313cabcd",
    "2358cd7a36ff27f980fc5a967a1b5769318c1f9cd433293d284a7f1f86e5dc57",
    "f38c92d77ffe82640449c0bb6184d37d9358f360d7f9a1529a0da115aa93c4f1",
    "ab5aaa974bbf2be7a4d443fbb91ea7108664225c13cebcb39e6c31af5453045c",
    "6300f2128cf914e5481b0c6526e78e1f82eb08bf750659c4f72de8b76d8e7d90",
];

fn temporary_path_for_save(save_path: &Path) -> PathBuf {
    let extension = save_path
        .extension()
        .and_then(|extension| extension.to_str())
        .map_or_else(|| "tmp".to_owned(), |extension| format!("{extension}.tmp"));
    save_path.with_extension(extension)
}

#[derive(Debug, Clone, Copy)]
struct PersistentRegion {
    size: usize,
    erased_value: u8,
}

#[derive(Debug, Clone)]
enum Layout {
    Plain {
        start: u16,
    },
    Mirrored,
    Page2Only,
    Konami {
        banks: [u8; 4],
    },
    KonamiScc {
        banks: [u8; 4],
        enabled: bool,
    },
    Generic8 {
        banks: [u8; 4],
    },
    Ascii8 {
        banks: [u8; 4],
        kind: Ascii8Kind,
    },
    Ascii16 {
        banks: [u8; 2],
        sram: bool,
    },
    GameMaster2 {
        banks: [u8; 4],
    },
    RType {
        bank: u8,
    },
    CrossBlaim {
        bank: u8,
    },
    HarryFox {
        banks: [u8; 2],
    },
    SuperLodeRunner {
        bank: u8,
    },
    SuperSwangi {
        bank: u8,
    },
    Majutsushi {
        banks: [u8; 4],
    },
    FmPac {
        bank: u8,
        control: u8,
        unlocked: u8,
    },
    MsxDos2 {
        bank: u8,
        control: u8,
    },
    Halnote {
        banks: [u8; 4],
        sub_banks: [u8; 2],
        sub_enabled: bool,
    },
    MsxWrite {
        banks: [u8; 2],
    },
    NettouYakyuu {
        banks: [u8; 4],
        redirected: [bool; 4],
        sample_control: u8,
    },
    PlayBall {
        sample_control: u8,
    },
    SccPlus {
        banks: [u8; 4],
        control: u8,
        minimum_bank: u8,
        maximum_bank: u8,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ascii8Kind {
    Rom,
    Sram,
    Koei,
    Wizardry,
}

/// Encodes mapper registers into a stable fixed-width representation.
fn encode_layout(layout: &Layout) -> (u8, [u8; 8]) {
    let mut values = [0; 8];
    let tag = match layout {
        Layout::Plain { start } => {
            values[..2].copy_from_slice(&start.to_le_bytes());
            0
        }
        Layout::Mirrored => 1,
        Layout::Page2Only => 2,
        Layout::Konami { banks } => {
            values[..4].copy_from_slice(banks);
            3
        }
        Layout::KonamiScc { banks, enabled } => {
            values[..4].copy_from_slice(banks);
            values[4] = u8::from(*enabled);
            4
        }
        Layout::Generic8 { banks } => {
            values[..4].copy_from_slice(banks);
            5
        }
        Layout::Ascii8 { banks, kind } => {
            values[..4].copy_from_slice(banks);
            values[4] = match kind {
                Ascii8Kind::Rom => 0,
                Ascii8Kind::Sram => 1,
                Ascii8Kind::Koei => 2,
                Ascii8Kind::Wizardry => 3,
            };
            6
        }
        Layout::Ascii16 { banks, sram } => {
            values[..2].copy_from_slice(banks);
            values[2] = u8::from(*sram);
            7
        }
        Layout::GameMaster2 { banks } => {
            values[..4].copy_from_slice(banks);
            8
        }
        Layout::RType { bank } => {
            values[0] = *bank;
            9
        }
        Layout::CrossBlaim { bank } => {
            values[0] = *bank;
            10
        }
        Layout::HarryFox { banks } => {
            values[..2].copy_from_slice(banks);
            11
        }
        Layout::SuperLodeRunner { bank } => {
            values[0] = *bank;
            12
        }
        Layout::SuperSwangi { bank } => {
            values[0] = *bank;
            13
        }
        Layout::Majutsushi { banks } => {
            values[..4].copy_from_slice(banks);
            14
        }
        Layout::FmPac {
            bank,
            control,
            unlocked,
        } => {
            values[..3].copy_from_slice(&[*bank, *control, *unlocked]);
            15
        }
        Layout::MsxDos2 { bank, control } => {
            values[..2].copy_from_slice(&[*bank, *control]);
            16
        }
        Layout::Halnote {
            banks,
            sub_banks,
            sub_enabled,
        } => {
            values[..4].copy_from_slice(banks);
            values[4..6].copy_from_slice(sub_banks);
            values[6] = u8::from(*sub_enabled);
            17
        }
        Layout::MsxWrite { banks } => {
            values[..2].copy_from_slice(banks);
            18
        }
        Layout::NettouYakyuu {
            banks,
            redirected,
            sample_control,
        } => {
            values[..4].copy_from_slice(banks);
            values[4] = redirected
                .iter()
                .enumerate()
                .fold(0, |bits, (index, value)| bits | (u8::from(*value) << index));
            values[5] = *sample_control;
            19
        }
        Layout::PlayBall { sample_control } => {
            values[0] = *sample_control;
            20
        }
        Layout::SccPlus {
            banks,
            control,
            minimum_bank,
            maximum_bank,
        } => {
            values[..4].copy_from_slice(banks);
            values[4..7].copy_from_slice(&[*control, *minimum_bank, *maximum_bank]);
            21
        }
    };
    (tag, values)
}

/// Decodes mapper registers from their fixed-width representation.
fn decode_layout(tag: u8, values: [u8; 8]) -> Result<Layout, save_state::StateValidationError> {
    let boolean = |value| match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(save_state::StateValidationError::new(
            "MSX cartridge mapper flag is invalid",
        )),
    };
    Ok(match tag {
        0 => Layout::Plain {
            start: u16::from_le_bytes([values[0], values[1]]),
        },
        1 => Layout::Mirrored,
        2 => Layout::Page2Only,
        3 => Layout::Konami {
            banks: values[..4].try_into().unwrap(),
        },
        4 => Layout::KonamiScc {
            banks: values[..4].try_into().unwrap(),
            enabled: boolean(values[4])?,
        },
        5 => Layout::Generic8 {
            banks: values[..4].try_into().unwrap(),
        },
        6 => Layout::Ascii8 {
            banks: values[..4].try_into().unwrap(),
            kind: match values[4] {
                0 => Ascii8Kind::Rom,
                1 => Ascii8Kind::Sram,
                2 => Ascii8Kind::Koei,
                3 => Ascii8Kind::Wizardry,
                _ => {
                    return Err(save_state::StateValidationError::new(
                        "MSX ASCII8 mapper kind is invalid",
                    ));
                }
            },
        },
        7 => Layout::Ascii16 {
            banks: values[..2].try_into().unwrap(),
            sram: boolean(values[2])?,
        },
        8 => Layout::GameMaster2 {
            banks: values[..4].try_into().unwrap(),
        },
        9 => Layout::RType { bank: values[0] },
        10 => Layout::CrossBlaim { bank: values[0] },
        11 => Layout::HarryFox {
            banks: values[..2].try_into().unwrap(),
        },
        12 => Layout::SuperLodeRunner { bank: values[0] },
        13 => Layout::SuperSwangi { bank: values[0] },
        14 => Layout::Majutsushi {
            banks: values[..4].try_into().unwrap(),
        },
        15 => Layout::FmPac {
            bank: values[0],
            control: values[1],
            unlocked: values[2],
        },
        16 => Layout::MsxDos2 {
            bank: values[0],
            control: values[1],
        },
        17 => Layout::Halnote {
            banks: values[..4].try_into().unwrap(),
            sub_banks: values[4..6].try_into().unwrap(),
            sub_enabled: boolean(values[6])?,
        },
        18 => Layout::MsxWrite {
            banks: values[..2].try_into().unwrap(),
        },
        19 => Layout::NettouYakyuu {
            banks: values[..4].try_into().unwrap(),
            redirected: core::array::from_fn(|index| values[4] & (1 << index) != 0),
            sample_control: values[5],
        },
        20 => Layout::PlayBall {
            sample_control: values[0],
        },
        21 if values[5] <= values[6] => Layout::SccPlus {
            banks: values[..4].try_into().unwrap(),
            control: values[4],
            minimum_bank: values[5],
            maximum_bank: values[6],
        },
        _ => {
            return Err(save_state::StateValidationError::new(
                "MSX cartridge mapper state is invalid",
            ));
        }
    })
}

save_state::runtime_state! {
/// Mutable cartridge mapper, RAM, and persistence state.
#[derive(Clone)]
pub(crate) struct CartridgeState {
    layout_tag: u8,
    layout_values: [u8; 8],
    mutable_bytes: Option<Vec<u8>>,
    persistence_bytes: Option<Vec<u8>>,
    persistence_dirty: Option<bool>,
    scc: Option<device::scc::SccState>,
    scc_plus: Option<device::scc::SccState>,
    dac: Option<device::eight_bit_dac::EightBitDacState>,
    opll: Option<device::opn_fm::OpnFmState<ymfm_oxide::Ym2413, ymfm_oxide::YmfmOutput2>>,
}}

pub(crate) struct Cartridge {
    bytes: Box<[u8]>,
    layout: Layout,
    bank_mask: u8,
    scc: Option<StandardScc>,
    scc_plus: Option<SccPlus>,
    dac: Option<EightBitDac>,
    opll: Option<OpnFm<Ym2413>>,
    persistence: Option<CartridgePersistence>,
    resource_identity: save_state::ResourceIdentity,
}

impl fmt::Debug for Cartridge {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Cartridge")
            .field("bytes", &self.bytes.len())
            .field("layout", &self.layout)
            .field("bank_mask", &self.bank_mask)
            .finish_non_exhaustive()
    }
}

impl Cartridge {
    pub(crate) fn detect(image: &[u8]) -> Result<(Self, CartridgeLoadInfo), CartridgeError> {
        Self::detect_with_path(image, None)
    }

    pub(crate) fn detect_with_path(
        image: &[u8],
        rom_path: Option<&Path>,
    ) -> Result<(Self, CartridgeLoadInfo), CartridgeError> {
        let digest = rom_loader::blake3_hex(image);
        let (mapper, identification) = if let Some(mapper) = known::mapper_for_digest(&digest) {
            (mapper, MapperIdentification::KnownHash)
        } else if let Some(mapper) = detect_plain(image) {
            (mapper, MapperIdentification::Header)
        } else {
            (
                detect_banked(image, &digest)?,
                MapperIdentification::Heuristic,
            )
        };
        let warning = (identification == MapperIdentification::Heuristic).then(|| {
            format!(
                "mapper for cartridge {digest} was inferred as {mapper}; add a verified hash entry if this is incorrect"
            )
        });
        let cartridge = Self::with_mapper_and_path(image, mapper, rom_path)?;
        Ok((
            cartridge,
            CartridgeLoadInfo {
                digest,
                mapper,
                identification,
                warning,
            },
        ))
    }

    #[cfg(test)]
    pub(crate) fn with_mapper(
        image: &[u8],
        mapper: CartridgeMapper,
    ) -> Result<Self, CartridgeError> {
        Self::with_mapper_and_path(image, mapper, None)
    }

    pub(crate) fn with_mapper_and_path(
        image: &[u8],
        mapper: CartridgeMapper,
        rom_path: Option<&Path>,
    ) -> Result<Self, CartridgeError> {
        validate_size(mapper, image.len())?;
        let layout = match mapper {
            CartridgeMapper::Plain8 | CartridgeMapper::Plain16 | CartridgeMapper::Plain32 => {
                Layout::Plain {
                    start: plain_start(image),
                }
            }
            CartridgeMapper::Mirrored => Layout::Mirrored,
            CartridgeMapper::Page2Only => Layout::Page2Only,
            CartridgeMapper::Konami => Layout::Konami {
                banks: [0, 1, 2, 3],
            },
            CartridgeMapper::KonamiScc => Layout::KonamiScc {
                banks: [0, 1, 2, 3],
                enabled: false,
            },
            CartridgeMapper::Generic8 => Layout::Generic8 {
                banks: [0, 1, 2, 3],
            },
            CartridgeMapper::Ascii8
            | CartridgeMapper::Ascii8Sram2
            | CartridgeMapper::Ascii8Sram8
            | CartridgeMapper::Ascii8Sram32
            | CartridgeMapper::KoeiSram8
            | CartridgeMapper::KoeiSram32
            | CartridgeMapper::Wizardry => Layout::Ascii8 {
                banks: [0; 4],
                kind: match mapper {
                    CartridgeMapper::Ascii8 => Ascii8Kind::Rom,
                    CartridgeMapper::KoeiSram8 | CartridgeMapper::KoeiSram32 => Ascii8Kind::Koei,
                    CartridgeMapper::Wizardry => Ascii8Kind::Wizardry,
                    _ => Ascii8Kind::Sram,
                },
            },
            CartridgeMapper::Ascii16
            | CartridgeMapper::Ascii16Sram2
            | CartridgeMapper::Ascii16Sram8 => Layout::Ascii16 {
                banks: [0; 2],
                sram: mapper != CartridgeMapper::Ascii16,
            },
            CartridgeMapper::GameMaster2 => Layout::GameMaster2 {
                banks: [0, 1, 2, 3],
            },
            CartridgeMapper::RType => Layout::RType { bank: 0 },
            CartridgeMapper::CrossBlaim => Layout::CrossBlaim { bank: 0 },
            CartridgeMapper::HarryFox => Layout::HarryFox { banks: [0, 1] },
            CartridgeMapper::SuperLodeRunner => Layout::SuperLodeRunner { bank: 0 },
            CartridgeMapper::SuperSwangi => Layout::SuperSwangi { bank: 0 },
            CartridgeMapper::Majutsushi => Layout::Majutsushi {
                banks: [0, 1, 2, 3],
            },
            CartridgeMapper::Synthesizer => Layout::Plain {
                start: CARTRIDGE_START,
            },
            CartridgeMapper::FmPac => Layout::FmPac {
                bank: 0,
                control: 0,
                unlocked: 0,
            },
            CartridgeMapper::MsxDos2 => {
                let control = image[0x94];
                if !matches!(control, 0x00 | 0x60 | 0x7F) {
                    return Err(CartridgeError::UnsupportedMsxDos2Control { value: control });
                }
                Layout::MsxDos2 { bank: 0, control }
            }
            CartridgeMapper::Halnote => Layout::Halnote {
                banks: [0, 1, 2, 3],
                sub_banks: [0; 2],
                sub_enabled: false,
            },
            CartridgeMapper::MsxWrite => Layout::MsxWrite { banks: [0; 2] },
            CartridgeMapper::NettouYakyuu => Layout::NettouYakyuu {
                banks: [0; 4],
                redirected: [false; 4],
                sample_control: 0,
            },
            CartridgeMapper::PlayBall => Layout::PlayBall { sample_control: 0 },
            CartridgeMapper::SccPlus | CartridgeMapper::Snatcher => Layout::SccPlus {
                banks: [0, 1, 2, 3],
                control: 0,
                minimum_bank: 0,
                maximum_bank: if mapper == CartridgeMapper::Snatcher {
                    7
                } else {
                    15
                },
            },
            CartridgeMapper::SdSnatcher => Layout::SccPlus {
                banks: [8, 9, 10, 11],
                control: 0,
                minimum_bank: 8,
                maximum_bank: 15,
            },
        };
        let bank_size = if matches!(
            mapper,
            CartridgeMapper::Ascii16
                | CartridgeMapper::Ascii16Sram2
                | CartridgeMapper::Ascii16Sram8
                | CartridgeMapper::MsxDos2
                | CartridgeMapper::MsxWrite
        ) {
            BANK_SIZE_16K
        } else {
            BANK_SIZE_8K
        };
        let bytes = if matches!(
            mapper,
            CartridgeMapper::SccPlus | CartridgeMapper::Snatcher | CartridgeMapper::SdSnatcher
        ) {
            vec![0; 0x20_000]
        } else {
            image.to_vec()
        };
        let bank_count = bytes.len().div_ceil(bank_size).max(1);
        let bank_mask = bank_count.next_power_of_two().saturating_sub(1).min(0xFF) as u8;
        let persistence = persistent_region(mapper)
            .map(|region| CartridgePersistence::load(region.size, region.erased_value, rom_path))
            .transpose()?;
        let mut resource_bytes = image.to_vec();
        resource_bytes.extend_from_slice(mapper.to_string().as_bytes());
        Ok(Self {
            bytes: bytes.into_boxed_slice(),
            layout,
            bank_mask,
            scc: matches!(
                mapper,
                CartridgeMapper::KonamiScc
                    | CartridgeMapper::SccPlus
                    | CartridgeMapper::Snatcher
                    | CartridgeMapper::SdSnatcher
            )
            .then(StandardScc::new),
            scc_plus: matches!(
                mapper,
                CartridgeMapper::SccPlus | CartridgeMapper::Snatcher | CartridgeMapper::SdSnatcher
            )
            .then(SccPlus::new),
            dac: matches!(
                mapper,
                CartridgeMapper::Majutsushi | CartridgeMapper::Synthesizer
            )
            .then(EightBitDac::new),
            opll: None,
            persistence,
            resource_identity: save_state::ResourceIdentity::from_bytes(&resource_bytes),
        })
    }

    /// Returns the immutable cartridge image and mapper identity.
    pub(crate) fn resource_identity(&self) -> save_state::ResourceIdentity {
        self.resource_identity
    }

    /// Captures mapper, writable storage, and sound state.
    pub(crate) fn capture_state(&self) -> CartridgeState {
        let (layout_tag, layout_values) = encode_layout(&self.layout);
        CartridgeState {
            layout_tag,
            layout_values,
            mutable_bytes: matches!(self.layout, Layout::SccPlus { .. })
                .then(|| self.bytes.to_vec()),
            persistence_bytes: self
                .persistence
                .as_ref()
                .map(|persistence| persistence.bytes.clone()),
            persistence_dirty: self
                .persistence
                .as_ref()
                .map(|persistence| persistence.dirty),
            scc: self.scc.as_ref().map(StandardScc::capture_state),
            scc_plus: self.scc_plus.as_ref().map(SccPlus::capture_state),
            dac: self.dac.as_ref().map(EightBitDac::capture_state),
            opll: self.opll.as_ref().map(OpnFm::capture_state),
        }
    }

    /// Restores mapper, writable storage, and sound state.
    pub(crate) fn restore_state(
        &mut self,
        state: CartridgeState,
    ) -> Result<(), save_state::StateValidationError> {
        let layout = decode_layout(state.layout_tag, state.layout_values)?;
        if core::mem::discriminant(&layout) != core::mem::discriminant(&self.layout) {
            return Err(save_state::StateValidationError::new(
                "MSX cartridge mapper configuration differs",
            ));
        }
        match state.mutable_bytes {
            Some(bytes)
                if matches!(layout, Layout::SccPlus { .. }) && bytes.len() == self.bytes.len() =>
            {
                self.bytes.copy_from_slice(&bytes);
            }
            None if !matches!(layout, Layout::SccPlus { .. }) => {}
            _ => {
                return Err(save_state::StateValidationError::new(
                    "MSX cartridge RAM state is invalid",
                ));
            }
        }
        match (
            self.persistence.as_mut(),
            state.persistence_bytes,
            state.persistence_dirty,
        ) {
            (Some(persistence), Some(bytes), Some(dirty))
                if bytes.len() == persistence.bytes.len() =>
            {
                persistence.bytes = bytes;
                persistence.dirty = dirty;
            }
            (None, None, None) => {}
            _ => {
                return Err(save_state::StateValidationError::new(
                    "MSX cartridge persistence state is invalid",
                ));
            }
        }
        match (&mut self.scc, state.scc) {
            (Some(scc), Some(scc_state)) => scc.restore_state(scc_state)?,
            (None, None) => {}
            _ => {
                return Err(save_state::StateValidationError::new(
                    "MSX cartridge SCC configuration differs",
                ));
            }
        }
        match (&mut self.scc_plus, state.scc_plus) {
            (Some(scc), Some(scc_state)) => scc.restore_state(scc_state)?,
            (None, None) => {}
            _ => {
                return Err(save_state::StateValidationError::new(
                    "MSX cartridge SCC+ configuration differs",
                ));
            }
        }
        match (&mut self.dac, state.dac) {
            (Some(dac), Some(dac_state)) => dac.restore_state(dac_state)?,
            (None, None) => {}
            _ => {
                return Err(save_state::StateValidationError::new(
                    "MSX cartridge DAC configuration differs",
                ));
            }
        }
        match (&mut self.opll, state.opll) {
            (Some(opll), Some(opll_state)) => opll.restore_state(opll_state)?,
            (None, None) => {}
            _ => {
                return Err(save_state::StateValidationError::new(
                    "MSX cartridge OPLL configuration differs",
                ));
            }
        }
        self.layout = layout;
        Ok(())
    }

    /// Configures cartridge sound devices for the machine stream.
    pub(crate) fn configure_audio(&mut self, cpu_clock_hz: u32, sample_rate: u32) {
        if let Some(scc) = self.scc.as_mut() {
            scc.configure_audio(cpu_clock_hz, sample_rate);
        }
        if let Some(scc) = self.scc_plus.as_mut() {
            scc.configure_audio(cpu_clock_hz, sample_rate);
        }
        if let Some(dac) = self.dac.as_mut() {
            dac.configure_audio(cpu_clock_hz, sample_rate);
        }
        if matches!(self.layout, Layout::FmPac { .. }) {
            self.opll = Some(OpnFm::new(cpu_clock_hz, sample_rate, cpu_clock_hz));
        }
    }

    pub(crate) fn read(&self, address: u16) -> Option<u8> {
        if let Layout::KonamiScc { enabled: true, .. } = self.layout
            && (0x9800..=0x9FFF).contains(&address)
            && let Some(value) = self.scc.as_ref().and_then(|scc| scc.read(address as u8))
        {
            return Some(value);
        }
        if let Layout::SccPlus { banks, control, .. } = self.layout {
            if control & 0x20 == 0
                && banks[2] & 0x3F == 0x3F
                && (0x9800..=0x9FFF).contains(&address)
                && let Some(value) = self.scc.as_ref().and_then(|scc| scc.read(address as u8))
            {
                return Some(value);
            }
            if control & 0x20 != 0
                && banks[3] & 0x80 != 0
                && (0xB800..=0xBFFF).contains(&address)
                && let Some(value) = self
                    .scc_plus
                    .as_ref()
                    .and_then(|scc| scc.read(address as u8))
            {
                return Some(value);
            }
        }

        let offset = match &self.layout {
            Layout::Plain { start } => usize::from(address.checked_sub(*start)?),
            Layout::Mirrored => {
                usize::from(address.wrapping_sub(CARTRIDGE_START)) % self.bytes.len()
            }
            Layout::Page2Only if (0x8000..=0xBFFF).contains(&address) => {
                usize::from(address - PAGE_2_START) % self.bytes.len()
            }
            Layout::Page2Only => return None,
            Layout::Konami { banks } => {
                let mapped = mirror_konami_address(address);
                self.bank_offset_8k(banks, mapped)?
            }
            Layout::KonamiScc { banks, .. } => {
                let mapped = mirror_konami_scc_address(address);
                self.bank_offset_8k(banks, mapped)?
            }
            Layout::Generic8 { banks } => self.bank_offset_8k(banks, address)?,
            Layout::Ascii8 { banks, kind } => {
                if !(CARTRIDGE_START..0xC000).contains(&address) {
                    return None;
                }
                let window = usize::from(address - CARTRIDGE_START) / BANK_SIZE_8K;
                if ascii8_sram_selected(*kind, window, banks[window], self.bank_mask) {
                    return self.read_ascii8_persistence(*kind, banks[window], address);
                }
                self.bank_offset_8k(banks, address)?
            }
            Layout::Ascii16 { banks, sram } => {
                if !(CARTRIDGE_START..0xC000).contains(&address) {
                    return None;
                }
                let window = usize::from(address - CARTRIDGE_START) / BANK_SIZE_16K;
                if *sram && banks[window] & 0x10 != 0 {
                    return self.read_persistence(usize::from(address) % BANK_SIZE_16K);
                }
                let bank = usize::from(banks[window] & self.bank_mask);
                bank * BANK_SIZE_16K + usize::from(address) % BANK_SIZE_16K
            }
            Layout::GameMaster2 { banks } => {
                if !(CARTRIDGE_START..0xC000).contains(&address) {
                    return None;
                }
                let window = usize::from(address - CARTRIDGE_START) / BANK_SIZE_8K;
                let selected = banks[window];
                if selected & 0x10 != 0 {
                    let offset = usize::from(selected & 0x20) << 7;
                    return self.read_persistence(offset | usize::from(address & 0x0FFF));
                }
                usize::from(selected & 0x0F) * BANK_SIZE_8K + usize::from(address) % BANK_SIZE_8K
            }
            Layout::RType { bank } => match address {
                0x4000..=0x7FFF => 0x17 * BANK_SIZE_16K + usize::from(address - 0x4000),
                0x8000..=0xBFFF => {
                    usize::from(*bank) * BANK_SIZE_16K + usize::from(address - 0x8000)
                }
                _ => return None,
            },
            Layout::CrossBlaim { bank } => {
                let selected = match (*bank & 3, address) {
                    (0 | 1, 0x0000..=0x3FFF) => 1,
                    (0 | 1, 0x4000..=0x7FFF) | (2 | 3, 0x4000..=0x7FFF) => 0,
                    (0 | 1, 0x8000..=0xBFFF) | (0 | 1, 0xC000..=0xFFFF) => 1,
                    (2, 0x8000..=0xBFFF) => 2,
                    (3, 0x8000..=0xBFFF) => 3,
                    _ => return None,
                };
                selected * BANK_SIZE_16K + usize::from(address) % BANK_SIZE_16K
            }
            Layout::HarryFox { banks } => {
                let window = match address {
                    0x4000..=0x7FFF => 0,
                    0x8000..=0xBFFF => 1,
                    _ => return None,
                };
                usize::from(banks[window]) * BANK_SIZE_16K + usize::from(address) % BANK_SIZE_16K
            }
            Layout::SuperLodeRunner { bank } if (0x8000..=0xBFFF).contains(&address) => {
                usize::from(*bank & self.bank_mask) * BANK_SIZE_16K + usize::from(address - 0x8000)
            }
            Layout::SuperLodeRunner { .. } => return None,
            Layout::SuperSwangi { bank } => match address {
                0x4000..=0x7FFF => usize::from(address - 0x4000),
                0x8000..=0xBFFF => {
                    usize::from(*bank & self.bank_mask) * BANK_SIZE_16K
                        + usize::from(address - 0x8000)
                }
                _ => return None,
            },
            Layout::Majutsushi { banks } => self.bank_offset_8k(banks, address)?,
            Layout::NettouYakyuu {
                banks, redirected, ..
            } => {
                if !(CARTRIDGE_START..0xC000).contains(&address) {
                    return None;
                }
                let window = usize::from(address - CARTRIDGE_START) / BANK_SIZE_8K;
                if redirected[window] {
                    return Some(0xFF);
                }
                self.bank_offset_8k(banks, address)?
            }
            Layout::FmPac { bank, unlocked, .. } if (0x4000..=0x7FFF).contains(&address) => {
                if *unlocked == 2 && address < 0x6000 {
                    return self.read_persistence(usize::from(address - 0x4000));
                }
                usize::from(*bank & 3) * BANK_SIZE_16K + usize::from(address - 0x4000)
            }
            Layout::FmPac { .. } => return None,
            Layout::MsxDos2 { bank, .. } if (0x4000..=0x7FFF).contains(&address) => {
                usize::from(*bank & self.bank_mask) * BANK_SIZE_16K
                    + usize::from(address - CARTRIDGE_START)
            }
            Layout::MsxDos2 { .. } => return None,
            Layout::Halnote {
                banks,
                sub_banks,
                sub_enabled,
            } => {
                if address < 0x4000 && banks[0] & 0x80 != 0 {
                    return self.read_persistence(usize::from(address));
                }
                if *sub_enabled && (0x7000..=0x7FFF).contains(&address) {
                    let window = usize::from(address - 0x7000) / SRAM_SIZE_2K;
                    0x80_000
                        + usize::from(sub_banks[window]) * SRAM_SIZE_2K
                        + usize::from(address) % SRAM_SIZE_2K
                } else {
                    self.bank_offset_8k(banks, address)?
                }
            }
            Layout::MsxWrite { banks, .. } => {
                if !(CARTRIDGE_START..0xC000).contains(&address) {
                    return None;
                }
                let window = usize::from(address - CARTRIDGE_START) / BANK_SIZE_16K;
                usize::from(banks[window] & self.bank_mask) * BANK_SIZE_16K
                    + usize::from(address) % BANK_SIZE_16K
            }
            Layout::PlayBall { .. } if address == 0xBFFF => return Some(0xFF),
            Layout::PlayBall { .. } => {
                usize::from(address.checked_sub(CARTRIDGE_START)?) % self.bytes.len()
            }
            Layout::SccPlus {
                banks,
                minimum_bank,
                maximum_bank,
                ..
            } => {
                if !(CARTRIDGE_START..0xC000).contains(&address) {
                    return None;
                }
                let window = usize::from(address - CARTRIDGE_START) / BANK_SIZE_8K;
                let bank = banks[window] & 0x0F;
                if !(*minimum_bank..=*maximum_bank).contains(&bank) {
                    return None;
                }
                usize::from(bank) * BANK_SIZE_8K + usize::from(address) % BANK_SIZE_8K
            }
        };
        self.bytes.get(offset).copied()
    }

    /// Synchronizes elapsed cartridge audio before a timed memory read.
    pub(crate) fn read_at(&mut self, address: u16, current_cycle: u64) -> Option<u8> {
        match self.layout {
            Layout::KonamiScc { enabled: true, .. } if (0x9800..=0x9FFF).contains(&address) => {
                if let Some(scc) = self.scc.as_mut() {
                    scc.sync(current_cycle);
                }
            }
            Layout::SccPlus { banks, control, .. }
                if control & 0x20 == 0
                    && banks[2] & 0x3F == 0x3F
                    && (0x9800..=0x9FFF).contains(&address) =>
            {
                if let Some(scc) = self.scc.as_mut() {
                    scc.sync(current_cycle);
                }
            }
            Layout::SccPlus { banks, control, .. }
                if control & 0x20 != 0
                    && banks[3] & 0x80 != 0
                    && (0xB800..=0xBFFF).contains(&address) =>
            {
                if let Some(scc) = self.scc_plus.as_mut() {
                    scc.sync(current_cycle);
                }
            }
            _ => {}
        }
        self.read(address)
    }

    fn read_persistence(&self, offset: usize) -> Option<u8> {
        let persistence = self.persistence.as_ref()?;
        persistence.read(offset % persistence.bytes.len())
    }

    /// Reads an ASCII8 persistent-memory window.
    fn read_ascii8_persistence(&self, kind: Ascii8Kind, bank: u8, address: u16) -> Option<u8> {
        let persistence = self.persistence.as_ref()?;
        let block = if kind == Ascii8Kind::Koei {
            usize::from(bank) & (persistence.bytes.len().div_ceil(BANK_SIZE_8K) - 1)
        } else {
            0
        };
        persistence.read(
            block * BANK_SIZE_8K
                + (usize::from(address) & (persistence.bytes.len() - 1) & (BANK_SIZE_8K - 1)),
        )
    }

    /// Applies cartridge writes decoded outside the selected slot.
    pub(crate) fn global_write(&mut self, address: u16, value: u8) -> bool {
        if let Layout::SuperLodeRunner { bank } = &mut self.layout
            && address == 0
        {
            *bank = value;
            return true;
        }
        false
    }

    #[cfg(test)]
    pub(crate) fn write(&mut self, address: u16, value: u8) -> bool {
        self.write_at(address, value, 0)
    }

    pub(crate) fn write_at(&mut self, address: u16, value: u8, current_cycle: u64) -> bool {
        let readable = self.read(address).is_some();
        let mut decoded = false;
        let persistence = &mut self.persistence;
        match &mut self.layout {
            Layout::Konami { banks } => {
                let index = match address {
                    0x6000..=0x7FFF => Some(1),
                    0x8000..=0x9FFF => Some(2),
                    0xA000..=0xBFFF => Some(3),
                    _ => None,
                };
                if let Some(index) = index {
                    banks[index] = value;
                    decoded = true;
                }
            }
            Layout::KonamiScc { banks, enabled } => {
                let index = match address {
                    0x5000..=0x57FF => Some(0),
                    0x7000..=0x77FF => Some(1),
                    0x9000..=0x97FF => Some(2),
                    0xB000..=0xB7FF => Some(3),
                    _ => None,
                };
                if let Some(index) = index {
                    banks[index] = value;
                    decoded = true;
                    if index == 2 {
                        *enabled = value & 0x3F == 0x3F;
                    }
                }
                if *enabled
                    && (0x9800..=0x9FFF).contains(&address)
                    && self
                        .scc
                        .as_mut()
                        .is_some_and(|scc| scc.write_at(address as u8, value, current_cycle))
                {
                    return true;
                }
            }
            Layout::Generic8 { banks } => {
                if (CARTRIDGE_START..0xC000).contains(&address) {
                    let window = usize::from(address - CARTRIDGE_START) / BANK_SIZE_8K;
                    banks[window] = value;
                    decoded = true;
                }
            }
            Layout::Ascii8 { banks, kind } => {
                let index = match address {
                    0x6000..=0x67FF => Some(0),
                    0x6800..=0x6FFF => Some(1),
                    0x7000..=0x77FF => Some(2),
                    0x7800..=0x7FFF => Some(3),
                    _ => None,
                };
                if let Some(index) = index {
                    banks[index] = value;
                    decoded = true;
                }
                if (CARTRIDGE_START..0xC000).contains(&address) {
                    let window = usize::from(address - CARTRIDGE_START) / BANK_SIZE_8K;
                    if ascii8_sram_selected(*kind, window, banks[window], self.bank_mask) {
                        let persistence_size =
                            persistence.as_ref().map_or(0, |region| region.bytes.len());
                        let block = if *kind == Ascii8Kind::Koei {
                            usize::from(banks[window])
                                & (persistence_size.div_ceil(BANK_SIZE_8K) - 1)
                        } else {
                            0
                        };
                        decoded |= write_persistence(
                            persistence,
                            block * BANK_SIZE_8K
                                + (usize::from(address)
                                    & (persistence_size - 1)
                                    & (BANK_SIZE_8K - 1)),
                            value,
                        );
                    }
                }
            }
            Layout::Ascii16 { banks, sram } => {
                let index = match address {
                    0x6000..=0x67FF => Some(0),
                    0x7000..=0x77FF => Some(1),
                    _ => None,
                };
                if let Some(index) = index {
                    banks[index] = value;
                    decoded = true;
                }
                if *sram && (0x8000..0xC000).contains(&address) {
                    let window = usize::from(address - CARTRIDGE_START) / BANK_SIZE_16K;
                    if banks[window] & 0x10 != 0 {
                        decoded |= write_persistence(
                            persistence,
                            usize::from(address) % BANK_SIZE_16K,
                            value,
                        );
                    }
                }
            }
            Layout::GameMaster2 { banks } => {
                let index = match address {
                    0x6000..=0x6FFF => Some(1),
                    0x8000..=0x8FFF => Some(2),
                    0xA000..=0xAFFF => Some(3),
                    _ => None,
                };
                if let Some(index) = index {
                    banks[index] = value;
                    decoded = true;
                } else if (0xB000..=0xBFFF).contains(&address) && banks[3] & 0x10 != 0 {
                    let offset =
                        (usize::from(banks[3] & 0x20) << 7) | usize::from(address & 0x0FFF);
                    decoded |= write_persistence(persistence, offset, value);
                }
            }
            Layout::RType { bank } => {
                if (0x4000..=0x7FFF).contains(&address) {
                    *bank = value & if value & 0x10 != 0 { 0x17 } else { 0x1F };
                    decoded = true;
                }
            }
            Layout::CrossBlaim { bank } => {
                *bank = value & 3;
                decoded = true;
            }
            Layout::HarryFox { banks } => {
                if (0x6000..=0x6FFF).contains(&address) {
                    banks[0] = 2 * (value & 1);
                    decoded = true;
                } else if (0x7000..=0x7FFF).contains(&address) {
                    banks[1] = 2 * (value & 1) + 1;
                    decoded = true;
                }
            }
            Layout::SuperLodeRunner { bank } => {
                let _ = bank;
            }
            Layout::SuperSwangi { bank } => {
                if address == 0x8000 {
                    *bank = value >> 1;
                    decoded = true;
                }
            }
            Layout::Majutsushi { banks } => {
                let index = match address {
                    0x6000..=0x7FFF => Some(1),
                    0x8000..=0x9FFF => Some(2),
                    0xA000..=0xBFFF => Some(3),
                    _ => None,
                };
                if let Some(index) = index {
                    banks[index] = value;
                    decoded = true;
                }
                if (0x5000..=0x5FFF).contains(&address) {
                    if let Some(dac) = self.dac.as_mut() {
                        dac.set_level(value, current_cycle);
                    }
                    decoded = true;
                }
            }
            Layout::Plain { .. } if address & 0xC010 == 0x4000 && self.dac.is_some() => {
                self.dac
                    .as_mut()
                    .expect("checked synthesizer DAC")
                    .set_level(value, current_cycle);
                decoded = true;
            }
            Layout::FmPac {
                bank,
                control,
                unlocked,
            } => match address {
                0x7FF4 => {
                    if let Some(opll) = self.opll.as_mut() {
                        opll.write_address(value, current_cycle);
                    }
                    decoded = true;
                }
                0x7FF5 => {
                    if let Some(opll) = self.opll.as_mut() {
                        opll.write_data(value, current_cycle);
                    }
                    decoded = true;
                }
                0x5FFE => {
                    *unlocked = u8::from(value == b'M');
                    decoded = true;
                }
                0x5FFF => {
                    *unlocked = u8::from(*unlocked == 1 && value == b'i') * 2;
                    decoded = true;
                }
                0x7FF6 => {
                    *control = value & 0x11;
                    if value & 0x10 != 0 {
                        *unlocked = 0;
                    }
                    decoded = true;
                }
                0x7FF7 => {
                    *bank = value & 3;
                    decoded = true;
                }
                0x4000..=0x5FFD if *unlocked == 2 => {
                    decoded |= write_persistence(persistence, usize::from(address - 0x4000), value);
                }
                _ => {}
            },
            Layout::MsxDos2 { bank, control } => {
                let selected = match *control {
                    0x00 => address == 0x7FF0,
                    0x60 => address & 0xF000 == 0x6000,
                    0x7F => address == 0x7FFE,
                    _ => false,
                };
                if selected {
                    *bank = value;
                    decoded = true;
                }
            }
            Layout::Halnote {
                banks,
                sub_banks,
                sub_enabled,
            } => {
                if address < 0x4000 && banks[0] & 0x80 != 0 {
                    decoded |= write_persistence(persistence, usize::from(address), value);
                } else if address == 0x77FF || address == 0x7FFF {
                    sub_banks[usize::from(address >= 0x7800)] = value;
                    decoded = true;
                } else if (address & 0x1FFF) == 0x0FFF
                    && (CARTRIDGE_START..0xC000).contains(&address)
                {
                    let window = usize::from(address - CARTRIDGE_START) / BANK_SIZE_8K;
                    banks[window] = value;
                    if window == 1 {
                        *sub_enabled = value & 0x80 != 0;
                    }
                    decoded = true;
                }
            }
            Layout::MsxWrite { banks } => {
                let index = if (0x6000..0x6800).contains(&address) || address == 0x6FFF {
                    Some(0)
                } else if (0x7000..0x7800).contains(&address) || address == 0x7FFF {
                    Some(1)
                } else {
                    None
                };
                if let Some(index) = index {
                    banks[index] = value;
                    decoded = true;
                }
            }
            Layout::NettouYakyuu {
                banks,
                redirected,
                sample_control,
            } => {
                let index = match address {
                    0x6000..=0x67FF => Some(0),
                    0x6800..=0x6FFF => Some(1),
                    0x7000..=0x77FF => Some(2),
                    0x7800..=0x7FFF => Some(3),
                    _ => None,
                };
                if let Some(index) = index {
                    redirected[index] = value & 0x80 != 0;
                    banks[index] = value;
                    decoded = true;
                } else if (CARTRIDGE_START..0xC000).contains(&address) {
                    let window = usize::from(address - CARTRIDGE_START) / BANK_SIZE_8K;
                    if redirected[window] {
                        *sample_control = value;
                        decoded = true;
                    }
                }
            }
            Layout::PlayBall { sample_control } => {
                if address == 0xBFFF && value <= 14 {
                    *sample_control = value;
                    decoded = true;
                }
            }
            Layout::SccPlus {
                banks,
                control,
                minimum_bank,
                maximum_bank,
            } => {
                if address == 0xBFFE || address == 0xBFFF {
                    *control = value;
                    decoded = true;
                } else if (CARTRIDGE_START..0xC000).contains(&address) {
                    let window = usize::from(address - CARTRIDGE_START) / BANK_SIZE_8K;
                    let ram_enabled = *control & 0x10 != 0
                        || (window < 2 && *control & (1 << window) != 0)
                        || (window == 2 && *control & 0x24 == 0x24);
                    let standard_enabled =
                        window == 2 && *control & 0x20 == 0 && banks[2] & 0x3F == 0x3F;
                    let plus_enabled = window == 3 && *control & 0x20 != 0 && banks[3] & 0x80 != 0;
                    if !ram_enabled && standard_enabled && (0x9800..=0x9FFF).contains(&address) {
                        decoded |= self
                            .scc
                            .as_mut()
                            .is_some_and(|scc| scc.write_at(address as u8, value, current_cycle));
                    } else if !ram_enabled && plus_enabled && (0xB800..=0xBFFF).contains(&address) {
                        decoded |= self
                            .scc_plus
                            .as_mut()
                            .is_some_and(|scc| scc.write_at(address as u8, value, current_cycle));
                    } else if ram_enabled {
                        let bank = banks[window] & 0x0F;
                        if (*minimum_bank..=*maximum_bank).contains(&bank) {
                            let offset = usize::from(bank) * BANK_SIZE_8K
                                + usize::from(address) % BANK_SIZE_8K;
                            self.bytes[offset] = value;
                            decoded = true;
                        }
                    } else {
                        let register_window = match address {
                            0x5000..=0x57FF => Some(0),
                            0x7000..=0x77FF => Some(1),
                            0x9000..=0x97FF => Some(2),
                            0xB000..=0xB7FF => Some(3),
                            _ => None,
                        };
                        if let Some(register_window) = register_window {
                            banks[register_window] = value;
                            decoded = true;
                        }
                    }
                }
            }
            Layout::Plain { .. } | Layout::Mirrored | Layout::Page2Only => {}
        }
        readable || decoded
    }

    pub(crate) fn mix_scc_samples(
        &mut self,
        frame_end_cycle: u64,
        cpu_clock_hz: u32,
        sample_rate: u32,
        volume: f32,
        output: &mut [f32],
    ) -> usize {
        let plus_mode =
            matches!(self.layout, Layout::SccPlus { control, .. } if control & 0x20 != 0);
        let scc_written = if plus_mode {
            0
        } else {
            self.scc.as_mut().map_or(0, |scc| {
                scc.mix_samples(frame_end_cycle, cpu_clock_hz, sample_rate, volume, output)
            })
        };
        let dac_written = self.dac.as_mut().map_or(0, |dac| {
            dac.mix_samples(frame_end_cycle, cpu_clock_hz, sample_rate, volume, output)
        });
        let plus_written = if plus_mode {
            self.scc_plus.as_mut().map_or(0, |scc| {
                scc.mix_samples(frame_end_cycle, cpu_clock_hz, sample_rate, volume, output)
            })
        } else {
            0
        };
        let opll_written = self.opll.as_mut().map_or(0, |opll| {
            opll.generate_samples(frame_end_cycle, cpu_clock_hz, volume, output);
            output.len()
        });
        scc_written
            .max(dac_written)
            .max(plus_written)
            .max(opll_written)
    }

    pub(crate) fn synchronize_audio(&mut self, current_cycle: u64) {
        if let Some(scc) = self.scc.as_mut() {
            scc.synchronize(current_cycle);
        }
        if let Some(dac) = self.dac.as_mut() {
            dac.synchronize(current_cycle);
        }
        if let Some(scc) = self.scc_plus.as_mut() {
            scc.synchronize(current_cycle);
        }
        if let Some(opll) = self.opll.as_mut() {
            opll.sync(current_cycle);
        }
    }

    /// Writes an enabled FM-PAC I/O register.
    pub(crate) fn fm_pac_io_write(&mut self, port: u8, value: u8, current_cycle: u64) -> bool {
        let Layout::FmPac { control, .. } = self.layout else {
            return false;
        };
        if control & 1 == 0 {
            return false;
        }
        let Some(opll) = self.opll.as_mut() else {
            return false;
        };
        if port & 1 == 0 {
            opll.write_address(value, current_cycle);
        } else {
            opll.write_data(value, current_cycle);
        }
        true
    }

    pub(crate) fn flush(&mut self) -> Result<(), CartridgeError> {
        self.persistence
            .as_mut()
            .map_or(Ok(()), CartridgePersistence::flush)
    }

    fn bank_offset_8k(&self, banks: &[u8; 4], address: u16) -> Option<usize> {
        if !(CARTRIDGE_START..0xC000).contains(&address) {
            return None;
        }
        let window = usize::from(address - CARTRIDGE_START) / BANK_SIZE_8K;
        let bank = usize::from(banks[window] & self.bank_mask);
        Some(bank * BANK_SIZE_8K + usize::from(address) % BANK_SIZE_8K)
    }
}

fn write_persistence(
    persistence: &mut Option<CartridgePersistence>,
    offset: usize,
    value: u8,
) -> bool {
    let Some(persistence) = persistence.as_mut() else {
        return false;
    };
    persistence.write(offset % persistence.bytes.len(), value)
}

fn ascii8_sram_selected(kind: Ascii8Kind, window: usize, bank: u8, bank_mask: u8) -> bool {
    let visible = window >= 2 || (kind == Ascii8Kind::Koei && window == 0);
    if !visible {
        return false;
    }
    match kind {
        Ascii8Kind::Rom => false,
        Ascii8Kind::Wizardry => bank & 0x80 != 0,
        Ascii8Kind::Sram | Ascii8Kind::Koei => bank & !bank_mask != 0,
    }
}

fn persistent_region(mapper: CartridgeMapper) -> Option<PersistentRegion> {
    let size = match mapper {
        CartridgeMapper::Ascii8Sram2 | CartridgeMapper::Ascii16Sram2 => SRAM_SIZE_2K,
        CartridgeMapper::Ascii8Sram8
        | CartridgeMapper::Ascii16Sram8
        | CartridgeMapper::KoeiSram8
        | CartridgeMapper::Wizardry
        | CartridgeMapper::GameMaster2
        | CartridgeMapper::FmPac => SRAM_SIZE_8K,
        CartridgeMapper::Ascii8Sram32 | CartridgeMapper::KoeiSram32 => SRAM_SIZE_32K,
        CartridgeMapper::Halnote => BANK_SIZE_16K,
        _ => return None,
    };
    Some(PersistentRegion {
        size,
        erased_value: 0xFF,
    })
}

fn validate_size(mapper: CartridgeMapper, size: usize) -> Result<(), CartridgeError> {
    let valid = match mapper {
        CartridgeMapper::Plain8 => size == BANK_SIZE_8K,
        CartridgeMapper::Plain16 => size == BANK_SIZE_16K,
        CartridgeMapper::Page2Only => matches!(size, BANK_SIZE_8K | BANK_SIZE_16K),
        CartridgeMapper::Plain32 => size == 2 * BANK_SIZE_16K,
        CartridgeMapper::Mirrored => {
            matches!(size, BANK_SIZE_8K | BANK_SIZE_16K) || matches!(size, 0x8000 | 0x1_0000)
        }
        CartridgeMapper::Konami | CartridgeMapper::Majutsushi => {
            (4 * BANK_SIZE_8K..=KONAMI_MAX_SIZE).contains(&size)
                && size.is_multiple_of(BANK_SIZE_8K)
        }
        CartridgeMapper::KonamiScc => {
            (4 * BANK_SIZE_8K..=KONAMI_SCC_MAX_SIZE).contains(&size)
                && size.is_multiple_of(BANK_SIZE_8K)
        }
        CartridgeMapper::Ascii8
        | CartridgeMapper::Generic8
        | CartridgeMapper::Ascii8Sram2
        | CartridgeMapper::Ascii8Sram8
        | CartridgeMapper::Ascii8Sram32
        | CartridgeMapper::KoeiSram8
        | CartridgeMapper::KoeiSram32
        | CartridgeMapper::Wizardry
        | CartridgeMapper::NettouYakyuu => {
            (4 * BANK_SIZE_8K..=ASCII8_MAX_SIZE).contains(&size)
                && size.is_multiple_of(BANK_SIZE_8K)
        }
        CartridgeMapper::Ascii16
        | CartridgeMapper::Ascii16Sram2
        | CartridgeMapper::Ascii16Sram8
        | CartridgeMapper::MsxWrite => {
            (2 * BANK_SIZE_16K..=ASCII16_MAX_SIZE).contains(&size)
                && size.is_multiple_of(BANK_SIZE_16K)
        }
        CartridgeMapper::MsxDos2 => {
            (2 * BANK_SIZE_16K..=ASCII16_MAX_SIZE).contains(&size)
                && size.is_multiple_of(BANK_SIZE_16K)
        }
        CartridgeMapper::GameMaster2 => size == GAME_MASTER_2_ROM_SIZE,
        CartridgeMapper::RType => matches!(size, R_TYPE_ROM_SIZE | 0x80_000),
        CartridgeMapper::CrossBlaim | CartridgeMapper::HarryFox => size == 0x1_0000,
        CartridgeMapper::SuperLodeRunner => size == 0x20_000,
        CartridgeMapper::SuperSwangi => size.is_multiple_of(BANK_SIZE_16K),
        CartridgeMapper::Synthesizer | CartridgeMapper::PlayBall => size == 0x8000,
        CartridgeMapper::FmPac => size == 0x1_0000,
        CartridgeMapper::Halnote => size == HALNOTE_ROM_SIZE,
        CartridgeMapper::SccPlus | CartridgeMapper::Snatcher | CartridgeMapper::SdSnatcher => {
            size == 0
        }
    };
    if valid {
        Ok(())
    } else {
        Err(CartridgeError::UnsupportedSize { size })
    }
}

fn detect_plain(image: &[u8]) -> Option<CartridgeMapper> {
    match image.len() {
        BANK_SIZE_8K | BANK_SIZE_16K | 0x8000 if page2_header(image) => {
            Some(CartridgeMapper::Page2Only)
        }
        BANK_SIZE_8K | BANK_SIZE_16K => Some(CartridgeMapper::Mirrored),
        0x8000 => Some(CartridgeMapper::Plain32),
        _ => None,
    }
}

fn page2_header(image: &[u8]) -> bool {
    if image.len() > BANK_SIZE_16K || image.len() < 10 || image.get(..2) != Some(b"AB") {
        return false;
    }
    let init = u16::from_le_bytes([image[2], image[3]]);
    let text = u16::from_le_bytes([image[8], image[9]]);
    if text & 0xC000 != PAGE_2_START {
        return false;
    }
    if init == 0 {
        return true;
    }
    init & 0xC000 == PAGE_2_START && image.get(usize::from(init) & (image.len() - 1)) == Some(&0xC9)
}

fn plain_start(image: &[u8]) -> u16 {
    if image.get(..2) != Some(b"AB") {
        return CARTRIDGE_START;
    }
    let init = u16::from_le_bytes([image[2], image[3]]);
    match init {
        0x0000..=0x3FFF => 0,
        0x4000..=0x7FFF => CARTRIDGE_START,
        _ => PAGE_2_START,
    }
}

fn detect_banked(image: &[u8], digest: &str) -> Result<CartridgeMapper, CartridgeError> {
    if image.len() < 4 * BANK_SIZE_8K || !image.len().is_multiple_of(BANK_SIZE_8K) {
        return Err(CartridgeError::UnsupportedSize { size: image.len() });
    }
    let mut konami = 0;
    let mut konami_scc = 0;
    let mut ascii8_common = 0;
    let mut ascii8_unique = 0;
    let mut ascii16 = 0;
    for instruction in image.windows(3) {
        if instruction[0] != 0x32 {
            continue;
        }
        let address = u16::from_le_bytes([instruction[1], instruction[2]]);
        match address {
            0x5000 | 0x9000 | 0xB000 => konami_scc += 1,
            0x4000 | 0x8000 | 0xA000 => konami += 1,
            0x6800 | 0x7800 => ascii8_unique += 2,
            0x77FF => ascii16 += 2,
            0x6000 => {
                konami += 1;
                ascii8_common += 1;
                ascii16 += 1;
            }
            0x7000 => {
                konami_scc += 1;
                ascii8_common += 1;
                ascii16 += 1;
            }
            _ => {}
        }
    }
    let ascii8 = if ascii8_unique == 0 {
        0
    } else {
        ascii8_common + ascii8_unique
    };
    let scores = [
        (CartridgeMapper::Konami, konami),
        (CartridgeMapper::KonamiScc, konami_scc),
        (CartridgeMapper::Ascii8, ascii8),
        (CartridgeMapper::Ascii16, ascii16),
    ];
    let best_score = scores.iter().map(|(_, score)| *score).max().unwrap_or(0);
    let mut best = scores
        .iter()
        .filter(|(_, score)| *score == best_score)
        .map(|(mapper, _)| *mapper);
    let mapper = best.next();
    let unique = mapper.is_some() && best.next().is_none();
    if best_score >= MINIMUM_MAPPER_SCORE && unique {
        return Ok(mapper.unwrap());
    }
    Err(CartridgeError::AmbiguousMapper {
        digest: digest.to_owned(),
        scores: format!(
            "Konami={konami}, Konami SCC={konami_scc}, ASCII8={ascii8}, ASCII16={ascii16}"
        ),
    })
}

fn mirror_konami_address(address: u16) -> u16 {
    match u32::from(address) {
        0x0000..=0x3FFF => address + 0x4000,
        CARTRIDGE_END..=0xFFFF => address - 0x4000,
        _ => address,
    }
}

fn mirror_konami_scc_address(address: u16) -> u16 {
    match u32::from(address) {
        0x0000..=0x3FFF => address + 0x8000,
        CARTRIDGE_END..=0xFFFF => address - 0x8000,
        _ => address,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn banked_image(bank_size: usize, banks: usize) -> Vec<u8> {
        (0..banks)
            .flat_map(|bank| vec![bank as u8; bank_size])
            .collect()
    }

    fn image_for_mapper(mapper: CartridgeMapper) -> Vec<u8> {
        match mapper {
            CartridgeMapper::Plain8 | CartridgeMapper::Mirrored | CartridgeMapper::Page2Only => {
                banked_image(BANK_SIZE_8K, 1)
            }
            CartridgeMapper::Plain16 => banked_image(BANK_SIZE_16K, 1),
            CartridgeMapper::Plain32 | CartridgeMapper::Synthesizer | CartridgeMapper::PlayBall => {
                banked_image(BANK_SIZE_16K, 2)
            }
            CartridgeMapper::GameMaster2 | CartridgeMapper::SuperLodeRunner => {
                banked_image(BANK_SIZE_8K, 16)
            }
            CartridgeMapper::RType => banked_image(BANK_SIZE_16K, 24),
            CartridgeMapper::CrossBlaim | CartridgeMapper::HarryFox | CartridgeMapper::FmPac => {
                banked_image(BANK_SIZE_16K, 4)
            }
            CartridgeMapper::SuperSwangi => banked_image(BANK_SIZE_16K, 8),
            CartridgeMapper::Halnote => banked_image(BANK_SIZE_8K, 128),
            CartridgeMapper::Ascii16
            | CartridgeMapper::Ascii16Sram2
            | CartridgeMapper::Ascii16Sram8
            | CartridgeMapper::MsxWrite => banked_image(BANK_SIZE_16K, 8),
            CartridgeMapper::MsxDos2 => {
                let mut image = banked_image(BANK_SIZE_16K, 8);
                image[0x94] = 0;
                image
            }
            CartridgeMapper::SccPlus | CartridgeMapper::Snatcher | CartridgeMapper::SdSnatcher => {
                Vec::new()
            }
            _ => banked_image(BANK_SIZE_8K, 32),
        }
    }

    #[test]
    fn every_mapper_constructs_with_its_hardware_image_shape() {
        let mappers = [
            CartridgeMapper::Plain8,
            CartridgeMapper::Plain16,
            CartridgeMapper::Plain32,
            CartridgeMapper::Mirrored,
            CartridgeMapper::Page2Only,
            CartridgeMapper::Konami,
            CartridgeMapper::KonamiScc,
            CartridgeMapper::Ascii8,
            CartridgeMapper::Ascii16,
            CartridgeMapper::Generic8,
            CartridgeMapper::Ascii8Sram2,
            CartridgeMapper::Ascii8Sram8,
            CartridgeMapper::Ascii8Sram32,
            CartridgeMapper::Ascii16Sram2,
            CartridgeMapper::Ascii16Sram8,
            CartridgeMapper::KoeiSram8,
            CartridgeMapper::KoeiSram32,
            CartridgeMapper::Wizardry,
            CartridgeMapper::GameMaster2,
            CartridgeMapper::RType,
            CartridgeMapper::CrossBlaim,
            CartridgeMapper::HarryFox,
            CartridgeMapper::SuperLodeRunner,
            CartridgeMapper::SuperSwangi,
            CartridgeMapper::Majutsushi,
            CartridgeMapper::Synthesizer,
            CartridgeMapper::FmPac,
            CartridgeMapper::MsxDos2,
            CartridgeMapper::Halnote,
            CartridgeMapper::MsxWrite,
            CartridgeMapper::NettouYakyuu,
            CartridgeMapper::PlayBall,
            CartridgeMapper::SccPlus,
            CartridgeMapper::Snatcher,
            CartridgeMapper::SdSnatcher,
        ];
        for mapper in mappers {
            let image = image_for_mapper(mapper);
            Cartridge::with_mapper(&image, mapper)
                .unwrap_or_else(|error| panic!("{mapper} failed construction: {error}"));
        }
    }

    #[test]
    fn ascii_sram_windows_and_koei_blocks_match_hardware() {
        let image = banked_image(BANK_SIZE_8K, 8);
        let mut ascii = Cartridge::with_mapper(&image, CartridgeMapper::Ascii8Sram8).unwrap();
        ascii.write(0x6000, 8);
        ascii.write(0x4123, 0x44);
        assert_ne!(ascii.read(0x4123), Some(0x44));
        ascii.write(0x7000, 8);
        ascii.write(0x8123, 0x55);
        assert_eq!(ascii.read(0x8123), Some(0x55));

        let mut koei = Cartridge::with_mapper(&image, CartridgeMapper::KoeiSram32).unwrap();
        koei.write(0x6000, 9);
        koei.write(0x4123, 0x11);
        koei.write(0x6000, 10);
        koei.write(0x4123, 0x22);
        koei.write(0x6000, 9);
        assert_eq!(koei.read(0x4123), Some(0x11));
        koei.write(0x6000, 10);
        assert_eq!(koei.read(0x4123), Some(0x22));
    }

    #[test]
    fn ascii16_first_sram_window_is_read_only() {
        let image = banked_image(BANK_SIZE_16K, 8);
        let mut cartridge = Cartridge::with_mapper(&image, CartridgeMapper::Ascii16Sram8).unwrap();
        cartridge.write(0x6000, 0x10);
        cartridge.write(0x4123, 0x11);
        assert_eq!(cartridge.read(0x4123), Some(0xFF));
        cartridge.write(0x7000, 0x10);
        cartridge.write(0x8123, 0x22);
        assert_eq!(cartridge.read(0x4123), Some(0x22));
    }

    #[test]
    fn cross_blaim_maps_outer_pages_and_nettou_redirects_reads() {
        let image = banked_image(BANK_SIZE_16K, 4);
        let mut cross = Cartridge::with_mapper(&image, CartridgeMapper::CrossBlaim).unwrap();
        assert_eq!(cross.read(0x0000), Some(1));
        assert_eq!(cross.read(0xC000), Some(1));
        cross.write(0x1234, 2);
        assert_eq!(cross.read(0x0000), None);
        assert_eq!(cross.read(0x8000), Some(2));

        let image = banked_image(BANK_SIZE_8K, 32);
        let mut nettou = Cartridge::with_mapper(&image, CartridgeMapper::NettouYakyuu).unwrap();
        nettou.write(0x7000, 0x80);
        assert_eq!(nettou.read(0x8000), Some(0xFF));
        assert!(nettou.write(0x8000, 0x81));
    }

    #[test]
    fn mapper_reset_and_bank_writes_are_table_driven() {
        let cases = [
            (CartridgeMapper::Konami, 0x8000, 0x8000, 5, 5),
            (CartridgeMapper::KonamiScc, 0x7000, 0x6000, 5, 5),
            (CartridgeMapper::Ascii8, 0x6800, 0x6000, 5, 5),
            (CartridgeMapper::Ascii16, 0x7000, 0x8000, 2, 2),
        ];
        for (mapper, write_address, read_address, bank, expected) in cases {
            let bank_size = if mapper == CartridgeMapper::Ascii16 {
                BANK_SIZE_16K
            } else {
                BANK_SIZE_8K
            };
            let image = banked_image(bank_size, 8);
            let mut cartridge = Cartridge::with_mapper(&image, mapper).unwrap();
            assert!(cartridge.write(write_address, bank));
            assert_eq!(cartridge.read(read_address), Some(expected));
        }
    }

    #[test]
    fn mapper_reset_banks_match_hardware_layouts() {
        let image8 = banked_image(BANK_SIZE_8K, 8);
        for mapper in [CartridgeMapper::Konami, CartridgeMapper::KonamiScc] {
            let cartridge = Cartridge::with_mapper(&image8, mapper).unwrap();
            for window in 0..4 {
                let address = CARTRIDGE_START + (window * BANK_SIZE_8K) as u16;
                assert_eq!(cartridge.read(address), Some(window as u8));
            }
        }

        let ascii8 = Cartridge::with_mapper(&image8, CartridgeMapper::Ascii8).unwrap();
        for window in 0..4 {
            let address = CARTRIDGE_START + (window * BANK_SIZE_8K) as u16;
            assert_eq!(ascii8.read(address), Some(0));
        }

        let image16 = banked_image(BANK_SIZE_16K, 4);
        let ascii16 = Cartridge::with_mapper(&image16, CartridgeMapper::Ascii16).unwrap();
        assert_eq!(ascii16.read(0x4000), Some(0));
        assert_eq!(ascii16.read(0x8000), Some(0));
    }

    #[test]
    fn simple_layouts_cover_plain_mirrored_and_page_two_roms() {
        let mut plain = vec![0x11; BANK_SIZE_8K];
        plain[..4].copy_from_slice(&[b'A', b'B', 0x00, 0x40]);
        let plain = Cartridge::with_mapper(&plain, CartridgeMapper::Plain8).unwrap();
        assert_eq!(plain.read(0x4000), Some(b'A'));
        assert_eq!(plain.read(0x5FFF), Some(0x11));
        assert_eq!(plain.read(0x6000), None);

        let mirrored_image = banked_image(BANK_SIZE_8K, 1);
        let mirrored = Cartridge::with_mapper(&mirrored_image, CartridgeMapper::Mirrored).unwrap();
        assert_eq!(mirrored.read(0x4000), Some(0));
        assert_eq!(mirrored.read(0x6000), Some(0));
        assert_eq!(mirrored.read(0x0000), Some(0));

        let page_image = vec![0x22; BANK_SIZE_16K];
        let page = Cartridge::with_mapper(&page_image, CartridgeMapper::Page2Only).unwrap();
        assert_eq!(page.read(0x7FFF), None);
        assert_eq!(page.read(0x8000), Some(0x22));
        assert_eq!(page.read(0xBFFF), Some(0x22));
        assert_eq!(page.read(0xC000), None);
    }

    #[test]
    fn mapper_control_addresses_remain_readable_rom() {
        let cases = [
            (CartridgeMapper::Konami, 0x8000),
            (CartridgeMapper::KonamiScc, 0x7000),
            (CartridgeMapper::Ascii8, 0x6800),
            (CartridgeMapper::Ascii16, 0x7000),
        ];
        for (mapper, address) in cases {
            let bank_size = if mapper == CartridgeMapper::Ascii16 {
                BANK_SIZE_16K
            } else {
                BANK_SIZE_8K
            };
            let image = banked_image(bank_size, 8);
            let mut cartridge = Cartridge::with_mapper(&image, mapper).unwrap();
            assert!(cartridge.read(address).is_some());
            assert!(cartridge.write(address, 3));
            assert!(cartridge.read(address).is_some());
        }
    }

    #[test]
    /// MSX-DOS2 exposes only page one and switches its single 16 KiB window.
    fn msx_dos2_mapper_uses_the_header_selected_control_range() {
        for (control, write_address) in [(0x00, 0x7FF0), (0x60, 0x6000), (0x7F, 0x7FFE)] {
            let mut image = banked_image(BANK_SIZE_16K, 4);
            image[0x94] = control;
            let mut cartridge = Cartridge::with_mapper(&image, CartridgeMapper::MsxDos2).unwrap();

            assert_eq!(cartridge.read(0x4000), Some(0));
            assert_eq!(cartridge.read(0x8000), None);
            assert!(cartridge.write(write_address, 2));
            assert_eq!(cartridge.read(0x4000), Some(2));
        }
    }

    #[test]
    /// Unknown MSX-DOS2 control ranges are rejected.
    fn msx_dos2_mapper_rejects_unknown_control_ranges() {
        let mut image = banked_image(BANK_SIZE_16K, 4);
        image[0x94] = 0x42;
        assert!(matches!(
            Cartridge::with_mapper(&image, CartridgeMapper::MsxDos2),
            Err(CartridgeError::UnsupportedMsxDos2Control { value: 0x42 })
        ));
    }

    #[test]
    fn non_power_of_two_missing_banks_are_open_bus() {
        let image = banked_image(BANK_SIZE_8K, 6);
        let mut cartridge = Cartridge::with_mapper(&image, CartridgeMapper::Ascii8).unwrap();
        cartridge.write(0x6000, 7);
        assert_eq!(cartridge.read(0x4000), None);
    }

    #[test]
    fn scc_window_requires_bank_enable() {
        let image = banked_image(BANK_SIZE_8K, 8);
        let mut cartridge = Cartridge::with_mapper(&image, CartridgeMapper::KonamiScc).unwrap();
        cartridge.write(0x9800, 0x55);
        assert_ne!(cartridge.read(0x9800), Some(0x55));
        cartridge.write(0x9000, 0x3F);
        cartridge.write(0x9800, 0x55);
        assert_eq!(cartridge.read(0x9800), Some(0x55));
    }

    #[test]
    fn heuristic_reports_each_unique_mapper_signature() {
        let cases: [(CartridgeMapper, [u16; 2]); 4] = [
            (CartridgeMapper::Konami, [0x8000, 0xA000]),
            (CartridgeMapper::KonamiScc, [0x5000, 0x9000]),
            (CartridgeMapper::Ascii8, [0x6800, 0x7800]),
            (CartridgeMapper::Ascii16, [0x77FF, 0x77FF]),
        ];
        for (mapper, addresses) in cases {
            let mut image = banked_image(BANK_SIZE_8K, 16);
            for (index, address) in addresses.into_iter().enumerate() {
                let offset = index * 3;
                image[offset] = 0x32;
                image[offset + 1..offset + 3].copy_from_slice(&address.to_le_bytes());
            }
            let (_, info) = Cartridge::detect(&image).unwrap();
            assert_eq!(info.mapper, mapper);
            assert_eq!(info.identification, MapperIdentification::Heuristic);
            assert!(info.warning.is_some());
        }
    }

    #[test]
    fn ambiguous_large_rom_reports_digest_and_scores() {
        let image = banked_image(BANK_SIZE_8K, 16);
        let error = Cartridge::detect(&image).unwrap_err();
        assert!(matches!(
            error,
            CartridgeError::AmbiguousMapper { digest, scores }
                if digest.len() == 64 && scores.contains("Konami=0")
        ));
    }

    #[test]
    fn save_path_replaces_the_rom_extension() {
        assert_eq!(
            save_path_for_rom(Path::new("/games/title.rom")),
            PathBuf::from("/games/title.sav")
        );
        assert_eq!(
            save_path_for_rom(Path::new("/games/title")),
            PathBuf::from("/games/title.sav")
        );
    }

    #[test]
    fn persistence_loads_marks_and_flushes() {
        let directory =
            std::env::temp_dir().join(format!("neetan-msx-persistence-{}", std::process::id(),));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let rom_path = directory.join("title.rom");
        let save_path = directory.join("title.sav");
        let mut persistence = CartridgePersistence::load(4, 0xFF, Some(&rom_path)).unwrap();
        assert!(!save_path.exists());
        assert!(persistence.write(1, 0x42));
        assert!(persistence.is_dirty());
        persistence.flush().unwrap();
        assert_eq!(std::fs::read(&save_path).unwrap(), [0xFF, 0x42, 0xFF, 0xFF]);
        assert!(!persistence.is_dirty());
        persistence.write(1, 0x24);
        persistence.flush().unwrap();
        assert_eq!(std::fs::read(&save_path).unwrap(), [0xFF, 0x24, 0xFF, 0xFF]);
        assert!(matches!(
            CartridgePersistence::load(3, 0, Some(&rom_path)),
            Err(CartridgeError::InvalidSaveSize {
                expected: 3,
                actual: 4,
                ..
            })
        ));

        let mut in_memory = CartridgePersistence::load(2, 0, None).unwrap();
        in_memory.write(0, 1);
        in_memory.flush().unwrap();
        assert!(!in_memory.is_dirty());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn ascii_sram_variants_select_mirrored_persistent_storage() {
        let image8 = banked_image(BANK_SIZE_8K, 8);
        let mut ascii8 = Cartridge::with_mapper(&image8, CartridgeMapper::Ascii8Sram8).unwrap();
        ascii8.write(0x7000, 8);
        ascii8.write(0x8123, 0x5A);
        assert_eq!(ascii8.read(0x8123), Some(0x5A));

        let image16 = banked_image(BANK_SIZE_16K, 4);
        let mut ascii16 = Cartridge::with_mapper(&image16, CartridgeMapper::Ascii16Sram2).unwrap();
        ascii16.write(0x7000, 0x10);
        ascii16.write(0x8ABC, 0xA5);
        assert_eq!(ascii16.read(0x92BC), Some(0xA5));
    }

    #[test]
    fn game_master_two_sram_is_selected_in_four_kibibyte_halves() {
        let image = banked_image(BANK_SIZE_8K, 16);
        let mut cartridge = Cartridge::with_mapper(&image, CartridgeMapper::GameMaster2).unwrap();
        cartridge.write(0xA000, 0x10);
        cartridge.write(0xB123, 0x4C);
        assert_eq!(cartridge.read(0xA123), Some(0x4C));
        assert_eq!(cartridge.read(0xB123), Some(0x4C));
    }

    #[test]
    fn special_sixteen_kibibyte_mappers_switch_their_documented_windows() {
        let cases = [
            (CartridgeMapper::RType, 0x4000, 2, 0x8000, 2),
            (CartridgeMapper::HarryFox, 0x6000, 1, 0x4000, 2),
            (CartridgeMapper::SuperSwangi, 0x8000, 6, 0x8000, 3),
        ];
        for (mapper, write_address, bank, read_address, expected) in cases {
            let banks = match mapper {
                CartridgeMapper::RType => 32,
                CartridgeMapper::HarryFox => 4,
                _ => 8,
            };
            let image = banked_image(BANK_SIZE_16K, banks);
            let mut cartridge = Cartridge::with_mapper(&image, mapper).unwrap();
            cartridge.write(write_address, bank);
            assert_eq!(cartridge.read(read_address), Some(expected));
        }
    }

    #[test]
    fn fm_pac_requires_the_magic_sram_unlock_sequence() {
        let image = banked_image(BANK_SIZE_16K, 4);
        let mut cartridge = Cartridge::with_mapper(&image, CartridgeMapper::FmPac).unwrap();
        cartridge.write(0x4000, 0x44);
        assert_ne!(cartridge.read(0x4000), Some(0x44));
        cartridge.write(0x5FFE, b'M');
        cartridge.write(0x5FFF, b'i');
        cartridge.write(0x4000, 0x44);
        assert_eq!(cartridge.read(0x4000), Some(0x44));
        cartridge.write(0x7FF6, 0x10);
        assert_ne!(cartridge.read(0x4000), Some(0x44));
    }

    #[test]
    fn fm_pac_drives_ym2413_through_memory_and_io_registers() {
        let image = banked_image(BANK_SIZE_16K, 4);
        let mut cartridge = Cartridge::with_mapper(&image, CartridgeMapper::FmPac).unwrap();
        cartridge.configure_audio(3_579_545, 48_000);
        for (register, value) in [(0x30, 0x10), (0x10, 0x98), (0x20, 0x15)] {
            cartridge.write_at(0x7FF4, register, 0);
            cartridge.write_at(0x7FF5, value, 0);
        }
        let mut output = vec![0.0; 4_096];
        cartridge.mix_scc_samples(200_000, 3_579_545, 48_000, 1.0, &mut output);
        assert!(output.iter().any(|sample| *sample != 0.0));

        cartridge.write(0x7FF6, 1);
        assert!(cartridge.fm_pac_io_write(0x7C, 0x20, 200_000));
        assert!(cartridge.fm_pac_io_write(0x7D, 0, 200_000));
        cartridge.write(0x7FF6, 0);
        assert!(!cartridge.fm_pac_io_write(0x7C, 0, 200_000));
    }

    #[test]
    fn halnote_main_sub_and_sram_windows_are_independent() {
        let image = banked_image(BANK_SIZE_8K, 128);
        let mut cartridge = Cartridge::with_mapper(&image, CartridgeMapper::Halnote).unwrap();
        cartridge.write(0x4FFF, 0x80);
        cartridge.write(0x0123, 0x66);
        assert_eq!(cartridge.read(0x0123), Some(0x66));
        cartridge.write(0x6FFF, 0x80);
        cartridge.write(0x77FF, 5);
        assert_eq!(cartridge.read(0x7000), Some(0x41));
    }

    #[test]
    fn scc_plus_supports_compatible_plus_and_ram_modes() {
        let mut cartridge = Cartridge::with_mapper(&[], CartridgeMapper::SccPlus).unwrap();
        cartridge.write(0x9000, 0x3F);
        cartridge.write(0x9800, 0x33);
        assert_eq!(cartridge.read(0x9800), Some(0x33));
        cartridge.write(0xBFFF, 0x20);
        cartridge.write(0xB000, 0x80);
        cartridge.write(0xB800, 0x55);
        assert_eq!(cartridge.read(0xB800), Some(0x55));
        cartridge.write(0xBFFF, 0x10);
        cartridge.write(0x4123, 0x77);
        assert_eq!(cartridge.read(0x4123), Some(0x77));
    }

    #[test]
    fn disk_hashes_select_the_required_sound_cartridge() {
        assert_eq!(
            sound_cartridge_for_disk_blake3(
                "3ea7ffd9039e38390648d062c40f9e58884604c189d155ea9101e95150ad7107"
            ),
            Some(CartridgeMapper::Snatcher)
        );
        assert_eq!(
            sound_cartridge_for_disk_blake3(
                "203571ffcf8d7e2ac6998191f57635b391308e270f53bf25e05174f56e8b982f"
            ),
            Some(CartridgeMapper::SdSnatcher)
        );
        assert_eq!(sound_cartridge_for_disk_blake3("unknown"), None);
    }
}
