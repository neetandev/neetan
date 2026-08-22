//! PC/AT ROM set loading.
//!
//! Like the other machines, ROMs are matched by BLAKE3 content hash rather
//! than file name: the loader scans every file in the ROM directory (and its
//! immediate subdirectories, so the `ct486` and `et4000` sets can sit side by
//! side) and picks the ones whose digests match the accepted images. The
//! machine needs the AMI CS4031 system BIOS (`chips_1.ami`) and the ET4000AX
//! VGA BIOS (`et4000.bin`, or the alternate `cvet4kax.bin`).

use std::path::Path;

use rom_loader::{RomError, RomSlot, ScanOptions};

/// System BIOS ROM size in bytes.
const SYSTEM_BIOS_SIZE: usize = 0x1_0000;
/// VGA BIOS ROM size in bytes.
const VGA_BIOS_SIZE: usize = 0x8000;

/// File sizes of known ROM images; other files are skipped while scanning.
const KNOWN_ROM_SIZES: &[usize] = &[SYSTEM_BIOS_SIZE, VGA_BIOS_SIZE];

/// Accepted BLAKE3 digest for the AMI CS4031 system BIOS (`chips_1.ami`).
const SYSTEM_BIOS_DIGESTS: &[&str] =
    &["bcd8d7424756ca90c5853ac24c2a7f3621d5ff1f6f7a170027e0fbe2b10fd6f1"];

/// Accepted BLAKE3 digests for the ET4000AX VGA BIOS: `et4000.bin` and the
/// alternate ColorImage `cvet4kax.bin`.
const VGA_BIOS_DIGESTS: &[&str] = &[
    "24a00c4924d76f1bbb9afb554ff49009d004212d269bd7665fdfda9084bf3ec6",
    "5e5b51a62a2f5f20a09eb3dc6275d7c4b8af6ad2c9c67c02fc09150e974c4e23",
];

const SYSTEM_BIOS_SLOT: RomSlot =
    RomSlot::new("system-bios", SYSTEM_BIOS_SIZE, SYSTEM_BIOS_DIGESTS);

const VGA_BIOS_SLOT: RomSlot = RomSlot::new("vga-bios", VGA_BIOS_SIZE, VGA_BIOS_DIGESTS);

/// Embedded HLE system BIOS stub ROM (64 KiB), built from `utils/bios_at/bios.asm`.
static HLE_SYSTEM_BIOS: &[u8; SYSTEM_BIOS_SIZE] = include_bytes!("../../../utils/bios_at/bios.rom");

/// Embedded HLE VGA BIOS stub ROM (32 KiB), built from `utils/bios_at/vgabios.asm`.
static HLE_VGA_BIOS: &[u8; VGA_BIOS_SIZE] = include_bytes!("../../../utils/bios_at/vgabios.rom");

/// Raw bytes of a successfully loaded and validated PC/AT ROM set.
#[derive(Debug)]
pub struct LoadedRoms {
    /// System BIOS ROM (64 KiB), mapped at 0xF0000 and the 4 GiB-top alias.
    pub system_bios: Vec<u8>,
    /// ET4000AX VGA BIOS ROM (32 KiB), mapped at 0xC0000.
    pub vga_bios: Vec<u8>,
    /// Whether the set is the HLE stub pair. HLE-only paths like the fixed
    /// disk parameter table patching stay away from real ROM images.
    pub hle: bool,
}

impl LoadedRoms {
    /// Builds the ROM set from the embedded HLE stub images.
    ///
    /// The VGA stub reserves zero space for the video parameter table and the
    /// video save pointer table, which are filled here from the mode tables.
    /// Doing it during the build keeps the ROM identity a pure function of the
    /// images, which the save-state resource bindings depend on.
    pub fn hle_stub_set() -> Self {
        let mut vga_bios = HLE_VGA_BIOS.to_vec();
        crate::bus::write_video_parameter_tables(&mut vga_bios);
        Self {
            system_bios: HLE_SYSTEM_BIOS.to_vec(),
            vga_bios,
            hle: true,
        }
    }
}

/// Loads and validates the PC/AT ROM set from `rom_dir`.
///
/// Every regular file in the directory and its immediate subdirectories is
/// scanned; file names do not matter. Both slots are required.
pub fn load_rom_set(rom_dir: &Path) -> Result<LoadedRoms, RomError> {
    let options = ScanOptions {
        accepted_sizes: KNOWN_ROM_SIZES,
        subdirectory_depth: 1,
        expand: None,
    };
    let index = rom_loader::scan_directory(rom_dir, &options)?;
    let take = |slot: &RomSlot| index.take(slot);

    Ok(LoadedRoms {
        system_bios: take(&SYSTEM_BIOS_SLOT)?,
        vga_bios: take(&VGA_BIOS_SLOT)?,
        hle: false,
    })
}
