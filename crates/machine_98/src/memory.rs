use std::{
    fmt,
    ops::{Deref, DerefMut},
};

use common::MachineModel;

/// Main RAM start address (00000h). 640 KB on the PC-9801VM.
const RAM_START: u32 = 0x00000;
/// Main RAM end address, inclusive (9FFFFh).
const RAM_END: u32 = 0x9FFFF;
/// Main RAM size in bytes (640 KB).
const RAM_SIZE: usize = 0xA0000;

/// Extended RAM start address (1 MB).
const EXTENDED_RAM_START: u32 = 0x100000;

/// Text VRAM start address (A0000h). 16 KB (characters + attributes).
const TEXT_VRAM_START: u32 = 0xA0000;
/// Text VRAM end address, inclusive (A3FFFh).
const TEXT_VRAM_END: u32 = 0xA3FFF;
/// Text VRAM size in bytes (16 KB).
const TEXT_VRAM_SIZE: usize = 0x4000;

/// Unmapped gap between text VRAM and graphics VRAM (A4000h-A7FFFh).
/// Reads return 0xFF, writes are ignored.
const TEXT_VRAM_GAP_START: u32 = 0xA4000;
/// End of unmapped text/graphics gap, inclusive.
const TEXT_VRAM_GAP_END: u32 = 0xA7FFF;

/// Graphics VRAM B-plane start (A8000h). Each plane is 32 KB.
/// The three base planes (B, R, G) are contiguous: A8000-BFFFF (96 KB total).
const GRAPHICS_VRAM_START: u32 = 0xA8000;
/// Graphics VRAM G-plane end, inclusive (BFFFFh).
const GRAPHICS_VRAM_END: u32 = 0xBFFFF;
/// Graphics VRAM size per page for B/R/G planes (96 KB = 3 * 32 KB).
const GRAPHICS_VRAM_PAGE_SIZE: usize = 0x18000;
/// Total B/R/G backing size for two graphics pages.
const GRAPHICS_VRAM_SIZE: usize = GRAPHICS_VRAM_PAGE_SIZE * 2;

/// Unmapped gap between graphics VRAM and E-plane (C0000h-DFFFFh).
/// Reads return 0xFF, writes are ignored.
const GRAPHICS_GAP_START: u32 = 0xC0000;
/// End of unmapped gap, inclusive.
const GRAPHICS_GAP_END: u32 = 0xDFFFF;

/// E-plane (extended) graphics VRAM start (E0000h). 32 KB.
/// Only mapped when the 16-color graphics extension board is installed and
/// mode2 bit 0 selects 16-color analog mode. Otherwise reads 0xFF and writes are ignored.
const E_PLANE_VRAM_START: u32 = 0xE0000;
/// E-plane VRAM end, inclusive (E7FFFh).
const E_PLANE_VRAM_END: u32 = 0xE7FFF;
/// E-plane VRAM size per page in bytes (32 KB).
const E_PLANE_VRAM_PAGE_SIZE: usize = 0x8000;
/// Total E-plane backing size for two graphics pages.
const E_PLANE_VRAM_SIZE: usize = E_PLANE_VRAM_PAGE_SIZE * 2;

/// BIOS ROM start address (E8000h). Up to 96 KB (E8000-FFFFF).
const BIOS_ROM_START: u32 = 0xE8000;
/// BIOS ROM end, inclusive (FFFFFh).
const BIOS_ROM_END: u32 = 0xFFFFF;
/// BIOS ROM size in bytes (96 KB, covering E8000-FFFFF).
const BIOS_ROM_SIZE: usize = 0x18000;
/// Start of the ITF/BIOS bank-switched region (F8000h).
///
/// Port 0x043D controls which ROM bank is visible at F8000-FFFFF (32 KB).
/// The lower region E8000-F7FFF always shows the BIOS bank in dual-bank
/// configurations.
///
/// Ref: undoc98 `io_mem.txt` - "F8000h-FFFFFh switches to ITF ROM."
const ITF_BANK_SWITCH_START: u32 = 0xF8000;
/// Dual-bank BIOS image size (192 KB total).
///
/// Required binary layout for `load_rom`:
/// - `0x00000..0x18000` (bank 0 / ITF window): the upper 32 KB
///   (`0x10000..0x18000`) is mapped to F8000-FFFFF when port `0x43D`
///   selects ITF (`0x00`, `0x10`, `0x18`).
/// - `0x18000..0x30000` (bank 1 / BIOS window): mapped to E8000-FFFFF
///   when port `0x43D` selects BIOS (`0x12`). The lower 64 KB
///   (`0x18000..0x28000`) is always visible at E8000-F7FFF.
///
/// Recreating from separate dumps:
/// - If you have full 96 KB ITF-window and 96 KB BIOS-window images:
///   `dual = itf_96k || bios_96k`
/// - If only an ITF top 32 KB ROM and a 96 KB BIOS ROM are available:
///   fill bank 0 with `0xFF`, copy ITF ROM to bank0 offset `0x10000`,
///   then copy BIOS ROM to bank1 offset `0x18000`.
const BIOS_ROM_DUAL_BANK_IMAGE_SIZE: usize = BIOS_ROM_SIZE * 2;

/// PEGC extended VRAM size (512 KB). Only allocated for PC-9821 machines.
const PEGC_VRAM_SIZE: usize = 0x80000;

/// Character generator ROM size (528 KB).
const FONT_ROM_SIZE: usize = 0x84000;

/// EMS page frame start address (C0000h). 64 KB, 4 x 16 KB physical pages.
const EMS_PAGE_FRAME_START: u32 = 0xC0000;
/// EMS page frame end address, inclusive (CFFFFh).
const EMS_PAGE_FRAME_END: u32 = 0xCFFFF;
/// EMS page frame size in bytes (64 KB).
const EMS_PAGE_FRAME_SIZE: usize = 0x10000;
/// EMS physical page count (4 x 16 KB).
const EMS_PHYSICAL_PAGE_COUNT: usize = 4;
/// EMS physical page size in bytes.
const EMS_PHYSICAL_PAGE_SIZE: u32 = 0x4000;

/// UMB (Upper Memory Block) region start address (C0000h).
const UMB_START: u32 = 0xC0000;
/// UMB region size in bytes (128 KB at C0000h-DFFFFh).
const UMB_REGION_SIZE: usize = 0x20000;

/// Sound ROM start address (CC000h). 16 KB.
const SOUND_ROM_START: u32 = 0xCC000;
/// Sound ROM end address, inclusive (CFFFFh).
const SOUND_ROM_END: u32 = 0xCFFFF;
/// Sound ROM size in bytes (16 KB).
const SOUND_ROM_SIZE: usize = 0x4000;
/// Offset within sound ROM for the BIOS stub (INT D2h handler).
const SOUND_ROM_STUB_OFFSET: usize = 0x2E00;
/// Minimal BIOS stub installed when no full sound ROM is present.
/// Sets up a far-return header and a RETF at the INT D2h entry point.
const SOUND_ROM_STUB: [u8; 9] = [0x01, 0x00, 0x00, 0x00, 0xD2, 0x00, 0x08, 0x00, 0xCB];

/// Stub BIOS ROM (96 KB) embedded at compile time.
static STUB_BIOS_ROM: &[u8; BIOS_ROM_SIZE] = include_bytes!("../../../utils/bios/bios.rom");

const NEC_COPYRIGHT: &[u8] = b"Copyright (C) 1983 by NEC Corporation";
const NEC_COPYRIGHT_OFFSET_F: usize = 0x0DE2;
const NEC_COPYRIGHT_OFFSET_VM: usize = 0x0DD8;

/// V98 font ROM file size in bytes.
const V98_FONT_ROM_SIZE: usize = 0x46800;

save_state::runtime_state! {
/// Snapshot of mutable memory, bank selection, and character generator data.
#[derive(Clone, PartialEq, Eq)]
pub struct Pc9801MemoryState {
    /// Main RAM (640 KB).
    pub ram: Box<[u8; RAM_SIZE]>,
    /// Extended RAM above 1 MB.
    pub extended_ram: Box<[u8]>,
    /// Text VRAM (16 KB).
    pub text_vram: Box<[u8; TEXT_VRAM_SIZE]>,
    /// Base graphics VRAM (B/R/G planes, 96 KB per page, two pages total).
    pub graphics_vram: Box<[u8; GRAPHICS_VRAM_SIZE]>,
    /// Extended graphics VRAM E-plane (32 KB per page, two pages total).
    pub e_plane_vram: Box<[u8; E_PLANE_VRAM_SIZE]>,
    /// PEGC extended VRAM (512 KB). Only present on PC-9821 machines.
    pub pegc_vram: Option<Box<[u8; PEGC_VRAM_SIZE]>>,
    /// Whether E-plane VRAM is currently mapped at E0000-E7FFF.
    pub e_plane_enabled: bool,
    /// CPU address mask.
    pub address_mask: u32,
    /// Shadow RAM backing store (96 KB, E8000-FFFFF). Only allocated for i386+ machines.
    pub shadow_ram: Option<Box<[u8; BIOS_ROM_SIZE]>>,
    /// Shadow RAM control register (port 0x053D). See struct-level doc.
    pub shadow_control: u8,
    /// EMS page frame backing (64 KB at C0000-CFFFF). Enabled by the HLE memory manager.
    pub ems_page_frame: Option<Box<[u8; EMS_PAGE_FRAME_SIZE]>>,
    /// Live slot mappings for the EMS page frame. Unmapped slots use `ems_page_frame`.
    pub ems_page_frame_slot_mappings: [Option<u32>; EMS_PHYSICAL_PAGE_COUNT],
    /// UMB region backing (128 KB at C0000-DFFFF). Enabled by the HLE memory manager.
    pub umb_region: Option<Box<[u8; UMB_REGION_SIZE]>>,
    /// Optional extended-RAM backing address for the UMB window.
    pub umb_region_backing_linear_addr: Option<u32>,
    /// Active ROM bank selector for the banked BIOS window.
    pub bios_bank_is_bank1: bool,
    /// Character generator ROM and writable gaiji data (528 KB).
    ///
    /// Layout:
    /// - `0x00000-0x7FFFF`: Double-byte kanji glyphs, with 16x16 left and right halves interleaved.
    /// - `0x80000-0x80FFF`: ANK16 half-width 8x16 font, with 256 characters by 16 bytes.
    /// - `0x81000-0x81FFF`: Chargraph16 semigraphics 2x4 block patterns.
    /// - `0x82000-0x82FFF`: ANK8 and Chargraph8 interleaved, with 256 characters by 16 bytes.
    pub font_rom: Box<[u8; FONT_ROM_SIZE]>,
    /// Whether character generator data needs a host upload.
    pub font_rom_dirty: bool,
}}

impl fmt::Debug for Pc9801MemoryState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pc9801MemoryState")
            .field("ram", &format_args!("[u8; {:#X}]", self.ram.len()))
            .field(
                "extended_ram",
                &format_args!("[u8; {:#X}]", self.extended_ram.len()),
            )
            .field(
                "text_vram",
                &format_args!("[u8; {:#X}]", self.text_vram.len()),
            )
            .field(
                "graphics_vram",
                &format_args!("[u8; {:#X}]", self.graphics_vram.len()),
            )
            .field(
                "e_plane_vram",
                &format_args!("[u8; {:#X}]", self.e_plane_vram.len()),
            )
            .field(
                "pegc_vram",
                &self
                    .pegc_vram
                    .as_ref()
                    .map(|v| format!("[u8; {:#X}]", v.len())),
            )
            .field("e_plane_enabled", &self.e_plane_enabled)
            .field("address_mask", &format_args!("{:#010X}", self.address_mask))
            .field(
                "shadow_ram",
                &self
                    .shadow_ram
                    .as_ref()
                    .map(|v| format!("[u8; {:#X}]", v.len())),
            )
            .field(
                "shadow_control",
                &format_args!("{:#04X}", self.shadow_control),
            )
            .field("ems_page_frame", &self.ems_page_frame.is_some())
            .field(
                "ems_page_frame_slot_mappings",
                &self.ems_page_frame_slot_mappings,
            )
            .field("umb_region", &self.umb_region.is_some())
            .field(
                "umb_region_backing_linear_addr",
                &self.umb_region_backing_linear_addr,
            )
            .finish()
    }
}

/// PC-9801 series memory subsystem: RAM, VRAM, ROM, and address decoding.
///
/// # Memory map (V30 / 20-bit address space)
///
/// | Address range   | Read                              | Write                             |
/// |-----------------|-----------------------------------|-----------------------------------|
/// | 00000-9FFFF     | Main RAM (640 KB)                 | Main RAM                          |
/// | A0000-A3FFF     | Text VRAM (16 KB)                 | Text VRAM                         |
/// | A4000-A7FFF     | 0xFF (unmapped)                   | Ignored                           |
/// | A8000-BFFFF     | Graphics VRAM (96 KB, B/R/G)      | Graphics VRAM                     |
/// | C0000-CBFFF     | 0xFF (unmapped)                   | Ignored                           |
/// | CC000-CFFFF     | Sound ROM (16 KB, if loaded)      | Ignored                           |
/// | D0000-DFFFF     | 0xFF (unmapped)                   | Ignored                           |
/// | E0000-E7FFF     | E-plane VRAM (if enabled) / 0xFF  | E-plane VRAM (if enabled)         |
/// | E8000-FFFFF     | BIOS ROM (96 KB)                  | Shadow RAM (i386+) / Ignored (VM) |
///
/// # Extended memory (i286+ / 24-bit or 32-bit address space)
///
/// | Address range   | Read                              | Write                             |
/// |-----------------|-----------------------------------|-----------------------------------|
/// | 100000+         | Extended RAM (if backed) / 0xFF   | Extended RAM (if backed)          |
///
/// # Shadow RAM / BIOS RAM (I/O 053Dh) - 386+ machines only
///
/// On i386+ machines (RA, RA21, etc.), port 053Dh bit 1 selects whether
/// E8000-FFFFF reads come from ROM or a writable shadow RAM copy. The BIOS
/// copies ROM contents into shadow RAM at boot, then switches to RAM mode.
/// This feature does NOT exist on V30/V33 machines (VM, VX).
/// See `undoc98/io_mem.txt` lines 449-496.
///
/// Port 053Dh bit fields (write-only):
/// - bit 7: Sound BIOS enable
/// - bit 6: SASI HD-BIOS enable (D7000-D7FFF)
/// - bit 5: SCSI HD-BIOS enable
/// - bit 4: IDE HD-BIOS enable
/// - bit 2: BIOS RAM access enable (0=enabled, 1=disabled)
///   - RA2: controls E0000-FFFFF, RA21: controls C0000-FFFFF
/// - bit 1: BIOS RAM/ROM select (0=ROM, 1=RAM) for E8000-FFFFF
pub(crate) struct Pc9801Memory {
    /// Embedded state for save/restore.
    pub(crate) state: Pc9801MemoryState,
    rom: Box<[u8; BIOS_ROM_SIZE]>,
    /// Optional alternate 96 KB ROM bank selected via port 0x43D.
    bios_bank1: Option<Box<[u8; BIOS_ROM_SIZE]>>,
    /// Optional 16 KB sound ROM (CC000-CFFFF).
    sound_rom: Option<Box<[u8; SOUND_ROM_SIZE]>>,
}

impl Deref for Pc9801Memory {
    type Target = Pc9801MemoryState;
    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl DerefMut for Pc9801Memory {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

impl Pc9801Memory {
    /// Returns identities for immutable PC-98 memory resources.
    pub(crate) fn resource_manifest(
        &self,
    ) -> Result<save_state::ResourceManifest, save_state::StateValidationError> {
        let mut bindings = vec![save_state::ResourceBinding {
            identifier: save_state::ResourceBindingId::new("pc98-bios-bank-0")?,
            identity: save_state::ResourceIdentity::from_bytes(self.rom.as_slice()),
        }];
        if let Some(bios_bank) = self.bios_bank1.as_ref() {
            bindings.push(save_state::ResourceBinding {
                identifier: save_state::ResourceBindingId::new("pc98-bios-bank-1")?,
                identity: save_state::ResourceIdentity::from_bytes(bios_bank.as_slice()),
            });
        }
        if let Some(sound_rom) = self.sound_rom.as_ref() {
            bindings.push(save_state::ResourceBinding {
                identifier: save_state::ResourceBindingId::new("pc98-sound-rom")?,
                identity: save_state::ResourceIdentity::from_bytes(sound_rom.as_slice()),
            });
        }
        save_state::ResourceManifest::new(bindings)
    }

    /// Creates a new memory subsystem for the given machine model and extended RAM size.
    ///
    /// The BIOS probes extended RAM by writing test patterns in protected mode;
    /// addresses backed by RAM return the pattern, unmapped addresses read 0xFF.
    /// The detected size is stored at 0x0401 (EXPMMSZ) in 128 KB units.
    pub(crate) fn new(machine_model: MachineModel, extended_ram_size: usize) -> Self {
        let extended_ram_size = if matches!(
            machine_model,
            MachineModel::PC9801F | MachineModel::PC9801VM
        ) {
            0
        } else {
            extended_ram_size
        };
        let shadow_ram = if machine_model.has_shadow_ram() {
            Some(
                vec![0u8; BIOS_ROM_SIZE]
                    .into_boxed_slice()
                    .try_into()
                    .unwrap(),
            )
        } else {
            None
        };
        Self {
            state: Pc9801MemoryState {
                ram: vec![0u8; RAM_SIZE].into_boxed_slice().try_into().unwrap(),
                extended_ram: vec![0u8; extended_ram_size].into_boxed_slice(),
                text_vram: Box::new([0u8; TEXT_VRAM_SIZE]),
                graphics_vram: vec![0u8; GRAPHICS_VRAM_SIZE]
                    .into_boxed_slice()
                    .try_into()
                    .unwrap(),
                e_plane_vram: Box::new([0u8; E_PLANE_VRAM_SIZE]),
                pegc_vram: if machine_model.has_pegc() {
                    Some(
                        vec![0u8; PEGC_VRAM_SIZE]
                            .into_boxed_slice()
                            .try_into()
                            .unwrap(),
                    )
                } else {
                    None
                },
                e_plane_enabled: false,
                address_mask: machine_model.address_mask(),
                shadow_ram,
                shadow_control: 0x00,
                ems_page_frame: None,
                ems_page_frame_slot_mappings: [None; EMS_PHYSICAL_PAGE_COUNT],
                umb_region: None,
                umb_region_backing_linear_addr: None,
                bios_bank_is_bank1: false,
                font_rom: vec![0u8; FONT_ROM_SIZE]
                    .into_boxed_slice()
                    .try_into()
                    .unwrap(),
                font_rom_dirty: false,
            },
            rom: vec![0u8; BIOS_ROM_SIZE]
                .into_boxed_slice()
                .try_into()
                .unwrap(),
            bios_bank1: None,
            sound_rom: None,
        }
    }

    /// Loads the compiled stub BIOS ROM into the ROM buffer.
    pub(crate) fn load_stub_bios_rom(&mut self) {
        self.rom.copy_from_slice(STUB_BIOS_ROM);
        self.bios_bank1 = None;
        self.state.bios_bank_is_bank1 = false;
    }

    /// Installs the NEC copyright marker at the model-specific BIOS offset.
    pub(crate) fn install_nec_copyright(&mut self, machine_model: MachineModel) {
        let offset = match machine_model {
            MachineModel::PC9801F => NEC_COPYRIGHT_OFFSET_F,
            MachineModel::PC9801VM
            | MachineModel::PC9801VX
            | MachineModel::PC9801RS
            | MachineModel::PC9801RA
            | MachineModel::PC9821AS
            | MachineModel::PC9821AP => NEC_COPYRIGHT_OFFSET_VM,
        };
        self.rom[offset..offset + NEC_COPYRIGHT.len()].copy_from_slice(NEC_COPYRIGHT);
    }

    /// Reads a byte directly from the ROM buffer at the given ROM-relative offset.
    pub(crate) fn rom_byte(&self, offset: usize) -> u8 {
        self.rom[offset]
    }

    /// Writes a byte directly into the ROM buffer at the given ROM-relative offset.
    pub(crate) fn set_rom_byte(&mut self, offset: usize, value: u8) {
        self.rom[offset] = value;
    }

    /// Writes keyboard translation tables into the ROM at the BIOS code segment.
    pub(crate) fn install_keyboard_tables(
        &mut self,
        tables: &[[u8; 0x60]; 8],
        offset: usize,
        machine_model: MachineModel,
    ) {
        let rom_offset = 0xFD800 + offset - 0xE8000;
        for (i, table) in tables.iter().enumerate() {
            let dest = rom_offset + i * 0x60;
            self.rom[dest..dest + 0x60].copy_from_slice(table);
        }
        if machine_model == MachineModel::PC9801F {
            self.rom[rom_offset + 0x51] = 0xFF;
            self.rom[rom_offset + 0x60 + 0x51] = 0xFF;
        }
    }

    /// Installs FDD format parameter tables in the HLE BIOS ROM.
    ///
    /// Games and boot loaders follow the F2HD_POINTER / F2DD_POINTER BDA
    /// entries to read sectors-per-track and gap-length parameters from ROM.
    /// Without these tables the pointer chain dereferences into 0xFF-filled
    /// ROM, producing garbage disk parameters.
    ///
    /// VM/VX indirection at 0x1AB4/0x1ADC, data at 0x1ABC/0x1AE4.
    /// RA indirection at 0x1AAF/0x1AD7, data at 0x1AB7/0x1ADF.
    /// Each machine type gets only its own indirection to avoid overlap.
    pub(crate) fn install_disk_format_tables(&mut self, machine_model: MachineModel) {
        const BIOS_SEG_PHYS: usize = 0xFD800;
        const ROM_BASE: usize = 0xE8000;

        #[rustfmt::skip]
        const FDFMT_2HD: [u8; 32] = [
            0x00, 0x00, 0x00, 0x00, 0x1A, 0x07, 0x1A, 0x1B,
            0x1A, 0x0E, 0x1A, 0x36, 0x0F, 0x0E, 0x0F, 0x2A,
            0x0F, 0x1B, 0x0F, 0x54, 0x08, 0x1B, 0x08, 0x3A,
            0x08, 0x35, 0x08, 0x74, 0x00, 0x00, 0x00, 0x00,
        ];
        #[rustfmt::skip]
        const FDFMT_2DD: [u8; 32] = [
            0x00, 0x00, 0x00, 0x00, 0x10, 0x07, 0x10, 0x1B,
            0x10, 0x0E, 0x10, 0x36, 0x09, 0x0E, 0x09, 0x2A,
            0x09, 0x2A, 0x09, 0x50, 0x05, 0x1B, 0x05, 0x3A,
            0x05, 0x35, 0x05, 0x74, 0x00, 0x00, 0x00, 0x00,
        ];

        let write_ind = |rom: &mut [u8], ind_off: usize, data_off: u16| {
            for i in (0..8).step_by(2) {
                let dest = BIOS_SEG_PHYS + ind_off + i - ROM_BASE;
                rom[dest] = data_off as u8;
                rom[dest + 1] = (data_off >> 8) as u8;
            }
        };

        // Format table offsets differ between BIOS generations (RA vs others).
        if machine_model == MachineModel::PC9801F {
            #[rustfmt::skip]
            const F_BIOS_BYTES_1AB4: [u8; 8] = [
                0x00, 0xB1, 0x05, 0xD2, 0xE4, 0xE8, 0x7D, 0x02,
            ];
            #[rustfmt::skip]
            const F_BIOS_BYTES_1ABC: [u8; 32] = [
                0xB4, 0x00, 0xB2, 0x04, 0xE8, 0x76, 0x02, 0xFE,
                0xCA, 0x75, 0xF9, 0xE8, 0x9A, 0x02, 0xF6, 0xC4,
                0xF0, 0x75, 0x08, 0xE8, 0xF6, 0x01, 0x2E, 0x0A,
                0xA7, 0x14, 0x00, 0xC3, 0xB8, 0x40, 0x08, 0xE8,
            ];
            #[rustfmt::skip]
            const F_BIOS_BYTES_1ADC: [u8; 8] = [
                0x6D, 0x00, 0x80, 0xFC, 0x08, 0x75, 0x09, 0xF6,
            ];
            #[rustfmt::skip]
            const F_BIOS_BYTES_1AE4: [u8; 32] = [
                0x46, 0x01, 0x20, 0x74, 0x19, 0x80, 0xCC, 0xB0,
                0xC3, 0xB8, 0x44, 0x08, 0xE8, 0x58, 0x00, 0x80,
                0xFC, 0x08, 0x75, 0x09, 0xF6, 0x46, 0x01, 0x20,
                0x74, 0x04, 0x80, 0xCC, 0xB0, 0xC3, 0xB4, 0x40,
            ];

            let dest = BIOS_SEG_PHYS + 0x1AB4 - ROM_BASE;
            self.rom[dest..dest + F_BIOS_BYTES_1AB4.len()].copy_from_slice(&F_BIOS_BYTES_1AB4);
            let dest = BIOS_SEG_PHYS + 0x1ABC - ROM_BASE;
            self.rom[dest..dest + F_BIOS_BYTES_1ABC.len()].copy_from_slice(&F_BIOS_BYTES_1ABC);
            let dest = BIOS_SEG_PHYS + 0x1ADC - ROM_BASE;
            self.rom[dest..dest + F_BIOS_BYTES_1ADC.len()].copy_from_slice(&F_BIOS_BYTES_1ADC);
            let dest = BIOS_SEG_PHYS + 0x1AE4 - ROM_BASE;
            self.rom[dest..dest + F_BIOS_BYTES_1AE4.len()].copy_from_slice(&F_BIOS_BYTES_1AE4);
            return;
        }

        let (f2hd_ind, f2hd_data, f2dd_ind, f2dd_data): (usize, usize, usize, usize) =
            match machine_model {
                MachineModel::PC9801RS
                | MachineModel::PC9801RA
                | MachineModel::PC9821AS
                | MachineModel::PC9821AP => (0x1AAF, 0x1AB7, 0x1AD7, 0x1ADF),
                MachineModel::PC9801VM | MachineModel::PC9801VX => (0x1AB4, 0x1ABC, 0x1ADC, 0x1AE4),
                MachineModel::PC9801F => unreachable!(),
            };

        let dest = BIOS_SEG_PHYS + f2hd_data - ROM_BASE;
        self.rom[dest..dest + 32].copy_from_slice(&FDFMT_2HD);
        write_ind(&mut *self.rom, f2hd_ind, f2hd_data as u16);

        let dest = BIOS_SEG_PHYS + f2dd_data - ROM_BASE;
        self.rom[dest..dest + 32].copy_from_slice(&FDFMT_2DD);
        write_ind(&mut *self.rom, f2dd_ind, f2dd_data as u16);
    }

    /// Loads BIOS ROM data mapped at E8000-FFFFF.
    ///
    /// Accepted layouts:
    /// - 96 KB (`0x18000`): single-bank image (VM-style BIOS)
    /// - 192 KB (`0x30000`): dual-bank image (ITF bank + BIOS bank), used by
    ///   VX-class and newer machines that switch banks via port `0x43D`
    pub(crate) fn load_rom(&mut self, data: &[u8]) {
        self.bios_bank1 = None;
        self.bios_bank_is_bank1 = false;
        self.rom.fill(0xFF);
        if data.len() >= BIOS_ROM_DUAL_BANK_IMAGE_SIZE {
            self.rom.copy_from_slice(&data[..BIOS_ROM_SIZE]);
            let mut bank1: Box<[u8; BIOS_ROM_SIZE]> = vec![0u8; BIOS_ROM_SIZE]
                .into_boxed_slice()
                .try_into()
                .unwrap();
            bank1.copy_from_slice(&data[BIOS_ROM_SIZE..BIOS_ROM_DUAL_BANK_IMAGE_SIZE]);
            self.bios_bank1 = Some(bank1);
        } else {
            let length = data.len().min(self.rom.len());
            self.rom[..length].copy_from_slice(&data[..length]);
        }
    }

    /// Selects the active BIOS/ITF bank for the F8000-FFFFF window.
    pub(crate) fn select_banked_rom_window(&mut self, bank1: bool) {
        if self.bios_bank1.is_some() {
            self.bios_bank_is_bank1 = bank1;
        } else {
            self.bios_bank_is_bank1 = false;
        }
    }

    /// Enables or disables E-plane VRAM mapping at E0000-E7FFF.
    pub(crate) fn set_e_plane_enabled(&mut self, enabled: bool) {
        self.state.e_plane_enabled = enabled;
    }

    /// Sets the shadow RAM control register (port 0x053D). Only effective on i386+ machines.
    pub(crate) fn set_shadow_control(&mut self, value: u8) {
        if self.state.shadow_ram.is_some() {
            self.state.shadow_control = value;
        }
    }

    /// Copies the current ROM contents into shadow RAM (E8000-FFFFF).
    ///
    /// The real BIOS performs this copy before switching to shadow RAM read mode.
    /// Without it, shadow RAM reads return zeros instead of the expected ROM data.
    pub(crate) fn copy_rom_to_shadow_ram(&mut self) {
        if let Some(ref mut shadow) = self.state.shadow_ram {
            shadow.copy_from_slice(&*self.rom);
        }
    }

    /// Returns true when shadow RAM is selected for E8000-FFFFF reads/writes (bit 1 set).
    pub(crate) fn shadow_ram_selected(&self) -> bool {
        self.state.shadow_ram.is_some() && (self.state.shadow_control & 0x02) != 0
    }

    /// Returns true when the sound BIOS ROM is enabled (bit 7 of shadow control set).
    pub(crate) fn sound_bios_enabled(&self) -> bool {
        (self.state.shadow_control & 0x80) != 0
    }

    /// Loads the PC-9801-26K sound ROM (mapped at CC000-CFFFF, 16 KB).
    ///
    /// If `data` is `Some`, the full ROM is loaded. If `None`, a minimal
    /// stub is installed at offset 0x2E00 that provides a no-op INT D2h
    /// handler so that software probing the sound BIOS does not crash.
    pub(crate) fn load_sound_rom(&mut self, data: Option<&[u8]>) {
        let mut rom: Box<[u8; SOUND_ROM_SIZE]> = vec![0xFFu8; SOUND_ROM_SIZE]
            .into_boxed_slice()
            .try_into()
            .unwrap();
        match data {
            Some(bytes) => {
                let length = bytes.len().min(SOUND_ROM_SIZE);
                rom[..length].copy_from_slice(&bytes[..length]);
            }
            None => {
                rom[SOUND_ROM_STUB_OFFSET..SOUND_ROM_STUB_OFFSET + SOUND_ROM_STUB.len()]
                    .copy_from_slice(&SOUND_ROM_STUB);
            }
        }
        self.sound_rom = Some(rom);
    }

    /// Returns whether a sound ROM has been loaded.
    pub(crate) fn has_sound_rom(&self) -> bool {
        self.sound_rom.is_some()
    }

    /// Enables the EMS page frame backing at C0000-CFFFF (64 KB).
    pub(crate) fn enable_ems_page_frame(&mut self) {
        if self.state.ems_page_frame.is_none() {
            self.state.ems_page_frame = Some(Box::new([0u8; EMS_PAGE_FRAME_SIZE]));
        }
    }

    /// Maps or unmaps one 16 KB EMS page-frame slot.
    pub(crate) fn map_ems_page_frame_slot(
        &mut self,
        physical_page: u8,
        backing_linear_addr: Option<u32>,
    ) {
        let physical_page = usize::from(physical_page);
        if physical_page >= EMS_PHYSICAL_PAGE_COUNT {
            return;
        }
        self.state.ems_page_frame_slot_mappings[physical_page] = backing_linear_addr;
    }

    /// Enables the UMB region backing at C0000-DFFFF (128 KB).
    pub(crate) fn enable_umb_region(&mut self, backing_linear_addr: Option<u32>) {
        self.state.umb_region_backing_linear_addr = backing_linear_addr;
        if backing_linear_addr.is_none() && self.state.umb_region.is_none() {
            self.state.umb_region = Some(Box::new([0u8; UMB_REGION_SIZE]));
        }
    }

    pub(crate) fn umb_region_enabled(&self) -> bool {
        self.state.umb_region.is_some() || self.state.umb_region_backing_linear_addr.is_some()
    }

    /// Returns the size of extended RAM in bytes (0 for V30 machines).
    pub(crate) fn extended_memory_size(&self) -> u32 {
        self.state.extended_ram.len() as u32
    }

    fn ems_page_frame_linear_address(&self, address: u32) -> Option<u32> {
        if !(EMS_PAGE_FRAME_START..=EMS_PAGE_FRAME_END).contains(&address) {
            return None;
        }
        let slot = ((address - EMS_PAGE_FRAME_START) / EMS_PHYSICAL_PAGE_SIZE) as usize;
        let slot_offset = (address - EMS_PAGE_FRAME_START) % EMS_PHYSICAL_PAGE_SIZE;
        self.state.ems_page_frame_slot_mappings[slot].map(|base| base + slot_offset)
    }

    pub(crate) fn read_byte(&self, address: u32) -> u8 {
        let address = address & self.address_mask;
        if address >= EXTENDED_RAM_START {
            let offset = (address - EXTENDED_RAM_START) as usize;
            if offset < self.extended_ram.len() {
                return self.extended_ram[offset];
            }
            return 0xFF;
        }
        match address {
            RAM_START..=RAM_END => self.ram[address as usize],
            TEXT_VRAM_START..=TEXT_VRAM_END => self.text_vram[(address - TEXT_VRAM_START) as usize],
            TEXT_VRAM_GAP_START..=TEXT_VRAM_GAP_END => 0xFF,
            GRAPHICS_VRAM_START..=GRAPHICS_VRAM_END => {
                // Raw memory reads target page 0. Access/display page routing is applied by the bus.
                self.graphics_vram[(address - GRAPHICS_VRAM_START) as usize]
            }
            GRAPHICS_GAP_START..=GRAPHICS_GAP_END => {
                if let Some(linear_address) = self.ems_page_frame_linear_address(address) {
                    return self.read_byte(linear_address);
                }
                if let Some(ref pf) = self.state.ems_page_frame
                    && address <= EMS_PAGE_FRAME_END
                {
                    return pf[(address - EMS_PAGE_FRAME_START) as usize];
                }
                if address >= UMB_START {
                    let offset = address - UMB_START;
                    if let Some(backing) = self.state.umb_region_backing_linear_addr {
                        return self.read_byte(backing + offset);
                    }
                    if let Some(ref umb) = self.state.umb_region {
                        return umb[offset as usize];
                    }
                }
                if let Some(ref rom) = self.sound_rom
                    && (SOUND_ROM_START..=SOUND_ROM_END).contains(&address)
                    && (self.state.shadow_ram.is_none() || self.sound_bios_enabled())
                {
                    rom[(address - SOUND_ROM_START) as usize]
                } else {
                    0xFF
                }
            }
            E_PLANE_VRAM_START..=E_PLANE_VRAM_END if self.e_plane_enabled => {
                // Raw memory reads target page 0. Access/display page routing is applied by the bus.
                self.e_plane_vram[(address - E_PLANE_VRAM_START) as usize]
            }
            E_PLANE_VRAM_START..=E_PLANE_VRAM_END => 0xFF,
            BIOS_ROM_START..=BIOS_ROM_END => {
                let offset = (address - BIOS_ROM_START) as usize;
                if let Some(ref shadow) = self.state.shadow_ram
                    && self.shadow_ram_selected()
                {
                    // Shadow RAM only covers E8000-F7FFF. The bank-switched
                    // region F8000-FFFFF continues to read from ROM so that
                    // ITF/BIOS bank switching via port 043D still works after
                    // the shadow copy.
                    if address < ITF_BANK_SWITCH_START {
                        return shadow[offset];
                    }
                }
                if let Some(ref bank1) = self.bios_bank1 {
                    // Dual-bank ROM: E8000-F7FFF always from BIOS bank,
                    // F8000-FFFFF bank-switched via port 0x043D.
                    if address < ITF_BANK_SWITCH_START || self.bios_bank_is_bank1 {
                        return bank1[offset];
                    }
                }
                self.rom[offset]
            }
            _ => 0xFF,
        }
    }

    pub(crate) fn write_byte(&mut self, address: u32, value: u8) {
        let address = address & self.address_mask;
        if address >= EXTENDED_RAM_START {
            let offset = (address - EXTENDED_RAM_START) as usize;
            if offset < self.extended_ram.len() {
                self.extended_ram[offset] = value;
            }
            return;
        }
        match address {
            RAM_START..=RAM_END => self.ram[address as usize] = value,
            TEXT_VRAM_START..=TEXT_VRAM_END => {
                self.text_vram[(address - TEXT_VRAM_START) as usize] = value;
            }
            TEXT_VRAM_GAP_START..=TEXT_VRAM_GAP_END => {}
            GRAPHICS_VRAM_START..=GRAPHICS_VRAM_END => {
                // Raw memory writes target page 0. Access/display page routing is applied by the bus.
                self.graphics_vram[(address - GRAPHICS_VRAM_START) as usize] = value;
            }
            GRAPHICS_GAP_START..=GRAPHICS_GAP_END => {
                if let Some(linear_address) = self.ems_page_frame_linear_address(address) {
                    self.write_byte(linear_address, value);
                    return;
                }
                if let Some(ref mut pf) = self.state.ems_page_frame
                    && address <= EMS_PAGE_FRAME_END
                {
                    pf[(address - EMS_PAGE_FRAME_START) as usize] = value;
                    return;
                }
                if address >= UMB_START {
                    let offset = address - UMB_START;
                    if let Some(backing) = self.state.umb_region_backing_linear_addr {
                        self.write_byte(backing + offset, value);
                        return;
                    }
                    if let Some(ref mut umb) = self.state.umb_region {
                        umb[offset as usize] = value;
                    }
                }
            }
            E_PLANE_VRAM_START..=E_PLANE_VRAM_END if self.e_plane_enabled => {
                // Raw memory writes target page 0. Access/display page routing is applied by the bus.
                self.e_plane_vram[(address - E_PLANE_VRAM_START) as usize] = value;
            }
            BIOS_ROM_START..=BIOS_ROM_END => {
                if let Some(ref mut shadow) = self.state.shadow_ram {
                    shadow[(address - BIOS_ROM_START) as usize] = value;
                }
            }
            _ => {}
        }
    }

    pub(crate) fn read_byte_with_access_page(
        &self,
        address: u32,
        access_page: usize,
        b_bank_ems: bool,
        vram_ems_bank: u8,
    ) -> u8 {
        let address = address & self.address_mask;
        match address {
            GRAPHICS_VRAM_START..=GRAPHICS_VRAM_END => {
                if (0xB0000..=0xBFFFF).contains(&address) && b_bank_ems && vram_ems_bank & 0x02 != 0
                {
                    self.read_byte(EXTENDED_RAM_START + (address - 0xB0000))
                } else {
                    let page_base = access_page * GRAPHICS_VRAM_PAGE_SIZE;
                    self.graphics_vram[page_base + (address - GRAPHICS_VRAM_START) as usize]
                }
            }
            E_PLANE_VRAM_START..=E_PLANE_VRAM_END if self.e_plane_enabled => {
                let page_base = access_page * E_PLANE_VRAM_PAGE_SIZE;
                self.e_plane_vram[page_base + (address - E_PLANE_VRAM_START) as usize]
            }
            E_PLANE_VRAM_START..=E_PLANE_VRAM_END => 0xFF,
            _ => self.read_byte(address),
        }
    }

    pub(crate) fn write_byte_with_access_page(
        &mut self,
        address: u32,
        access_page: usize,
        b_bank_ems: bool,
        vram_ems_bank: u8,
        value: u8,
    ) {
        let address = address & self.address_mask;
        match address {
            GRAPHICS_VRAM_START..=GRAPHICS_VRAM_END => {
                if (0xB0000..=0xBFFFF).contains(&address) && b_bank_ems && vram_ems_bank & 0x02 != 0
                {
                    self.write_byte(EXTENDED_RAM_START + (address - 0xB0000), value);
                } else {
                    let page_base = access_page * GRAPHICS_VRAM_PAGE_SIZE;
                    self.graphics_vram[page_base + (address - GRAPHICS_VRAM_START) as usize] =
                        value;
                }
            }
            E_PLANE_VRAM_START..=E_PLANE_VRAM_END if self.e_plane_enabled => {
                let page_base = access_page * E_PLANE_VRAM_PAGE_SIZE;
                self.e_plane_vram[page_base + (address - E_PLANE_VRAM_START) as usize] = value;
            }
            E_PLANE_VRAM_START..=E_PLANE_VRAM_END => {}
            _ => self.write_byte(address, value),
        }
    }

    /// Loads a V98-format font ROM (0x46800 bytes) into the internal font buffer.
    ///
    /// Converts from V98 file layout to the interleaved fontrom format used by
    /// the CGROM I/O ports.
    pub(crate) fn load_font_rom(&mut self, data: &[u8]) {
        if data.len() < V98_FONT_ROM_SIZE {
            common::warn!(
                "Font ROM too small ({} bytes, expected {})",
                data.len(),
                V98_FONT_ROM_SIZE
            );
            return;
        }

        // ANK 0x00-0x7F (8x16): V98 offset 0x0800, 128 chars * 16 bytes
        self.font_rom[0x80000..0x80800].copy_from_slice(&data[0x0800..0x1000]);

        // ANK 0x80-0xFF (8x16): V98 offset 0x1000, 128 chars * 16 bytes
        self.font_rom[0x80800..0x81000].copy_from_slice(&data[0x1000..0x1800]);

        // Kanji level 1 (rows 0x01..0x30)
        self.v98_kanji_copy(data, 0x01, 0x30);
        // Kanji level 2 (rows 0x30..0x56)
        self.v98_kanji_copy(data, 0x30, 0x56);
        // Extended kanji (rows 0x58..0x5D)
        self.v98_kanji_copy(data, 0x58, 0x5D);

        // ANK8 (6×8): V98 offset 0x0000, 256 chars × 8 bytes, stored with 16-byte stride.
        self.load_ank8_bank(&data[0x0000..0x0800]);

        // Build chargraph semigraphics banks (writes to 0x81000 and 0x82000+8 per char).
        self.rebuild_chargraph_bank();
    }

    /// Converts V98 kanji font data to the interleaved fontrom layout.
    fn v98_kanji_copy(&mut self, src: &[u8], from: usize, to: usize) {
        for i in from..to {
            let mut p = 0x1800 + 0x60 * 32 * (i - 1);
            let mut q = 0x20000 + (i << 4);
            for _j in 0x20..0x80 {
                for _k in 0..16 {
                    if q + 0x800 < self.font_rom.len() && p + 16 < src.len() {
                        self.font_rom[q + 0x800] = src[p + 16];
                        self.font_rom[q] = src[p];
                    }
                    p += 1;
                    q += 1;
                }
                p += 16;
                q += 0x1000 - 16;
            }
        }
    }

    pub(crate) fn font_rom_data(&self) -> &[u8] {
        &self.font_rom[0x00000..0x83000]
    }

    /// Builds the chargraph (semigraphics) banks in font ROM.
    ///
    /// Generates 2×4 block element patterns for 256 possible byte values:
    /// - 16×16 patterns at `0x81000` (4 groups × 4 rows × 1 byte/row = 16 bytes/char)
    /// - 8×8 patterns at `0x82000+8` per char (4 groups × 2 rows × 1 byte/row = 8 bytes/char)
    ///
    /// Bit mapping per char code byte: bits 0-3 control left column (rows 0-3),
    /// bits 4-7 control right column (rows 0-3).
    fn rebuild_chargraph_bank(&mut self) {
        let mut p = 0x81000usize;
        let mut q = 0x82000usize;
        for i in 0u32..256 {
            q += 8;
            for j in 0..4u32 {
                let mut dbit: u32 = 0;
                if i & (0x01 << j) != 0 {
                    dbit |= 0xF0F0_F0F0;
                }
                if i & (0x10 << j) != 0 {
                    dbit |= 0x0F0F_0F0F;
                }
                let bytes = dbit.to_le_bytes();
                self.font_rom[p..p + 4].copy_from_slice(&bytes);
                p += 4;
                self.font_rom[q..q + 2].copy_from_slice(&bytes[..2]);
                q += 2;
            }
        }
        // NEC patch: clear first two bytes of char 0xF2 chargraph entries.
        let f2_16 = 0x81000 + 0xF2 * 16;
        self.font_rom[f2_16] = 0;
        self.font_rom[f2_16 + 1] = 0;
        let f2_8 = 0x82000 + 0xF2 * 16 + 8;
        self.font_rom[f2_8] = 0;
    }

    /// Loads ANK8 (6×8) font data into the font ROM at `0x82000` with 16-byte stride.
    ///
    /// Each of 256 characters occupies bytes 0-7 of its 16-byte slot (bytes 8-15
    /// are reserved for chargraph8 patterns, populated separately).
    fn load_ank8_bank(&mut self, data: &[u8]) {
        for char_index in 0..256usize {
            let src_offset = char_index * 8;
            let dst_offset = 0x82000 + char_index * 16;
            if src_offset + 8 <= data.len() {
                self.font_rom[dst_offset..dst_offset + 8]
                    .copy_from_slice(&data[src_offset..src_offset + 8]);
            }
        }
    }

    pub(crate) fn font_read(&self, address: usize) -> u8 {
        if address < self.font_rom.len() {
            self.font_rom[address]
        } else {
            0
        }
    }

    pub(crate) fn font_write(&mut self, address: usize, value: u8) {
        if address < self.font_rom.len() {
            self.font_rom[address] = value;
            self.font_rom_dirty = true;
        }
    }

    pub(crate) fn take_font_rom_dirty(&mut self) -> bool {
        std::mem::replace(&mut self.font_rom_dirty, false)
    }
}

#[cfg(test)]
mod tests {
    use common::MachineModel;

    use super::{Pc9801Memory, V98_FONT_ROM_SIZE};

    fn expected_chargraph16_bytes(char_code: u8) -> [u8; 16] {
        let mut output = [0u8; 16];
        let char_value = u32::from(char_code);
        for row in 0..4u32 {
            let mut dot_pattern = 0u32;
            if char_value & (0x01 << row) != 0 {
                dot_pattern |= 0xF0F0_F0F0;
            }
            if char_value & (0x10 << row) != 0 {
                dot_pattern |= 0x0F0F_0F0F;
            }
            let bytes = dot_pattern.to_le_bytes();
            let output_offset = row as usize * 4;
            output[output_offset..output_offset + 4].copy_from_slice(&bytes);
        }
        output
    }

    fn expected_chargraph8_bytes(char_code: u8) -> [u8; 8] {
        let chargraph16 = expected_chargraph16_bytes(char_code);
        let mut output = [0u8; 8];
        for row in 0..4usize {
            let source_offset = row * 4;
            let output_offset = row * 2;
            output[output_offset..output_offset + 2]
                .copy_from_slice(&chargraph16[source_offset..source_offset + 2]);
        }
        output
    }

    fn test_v98_font_rom_data() -> Vec<u8> {
        let mut data = vec![0u8; V98_FONT_ROM_SIZE];
        for char_index in 0..256usize {
            let source_offset = char_index * 8;
            for line in 0..8usize {
                data[source_offset + line] =
                    (char_index as u8).wrapping_mul(3).wrapping_add(line as u8);
            }
        }
        data
    }

    #[test]
    fn load_font_rom_populates_chargraph16_and_chargraph8_banks() {
        let mut memory = Pc9801Memory::new(MachineModel::PC9801VM, 0);
        let data = test_v98_font_rom_data();
        memory.load_font_rom(&data);

        for &char_code in &[0x01u8, 0x10, 0x5A] {
            let expected_chargraph16 = expected_chargraph16_bytes(char_code);
            let chargraph16_base = 0x81000 + usize::from(char_code) * 16;
            assert_eq!(
                &memory.font_rom[chargraph16_base..chargraph16_base + 16],
                expected_chargraph16.as_slice(),
                "chargraph16 mismatch for char 0x{char_code:02X}",
            );

            let expected_chargraph8 = expected_chargraph8_bytes(char_code);
            let chargraph8_base = 0x82000 + usize::from(char_code) * 16 + 8;
            assert_eq!(
                &memory.font_rom[chargraph8_base..chargraph8_base + 8],
                expected_chargraph8.as_slice(),
                "chargraph8 mismatch for char 0x{char_code:02X}",
            );
        }
    }

    #[test]
    fn load_font_rom_keeps_ank8_bytes_separate_from_chargraph8_bytes() {
        let mut memory = Pc9801Memory::new(MachineModel::PC9801VM, 0);
        let data = test_v98_font_rom_data();
        memory.load_font_rom(&data);

        for &char_code in &[0x00u8, 0x34, 0xA5, 0xF2] {
            let ank8_base = 0x82000 + usize::from(char_code) * 16;
            let source_offset = usize::from(char_code) * 8;
            assert_eq!(
                &memory.font_rom[ank8_base..ank8_base + 8],
                &data[source_offset..source_offset + 8],
                "ANK8 bytes changed by chargraph generation for char 0x{char_code:02X}",
            );
        }
    }

    #[test]
    fn load_font_rom_applies_f2_nec_patch_to_chargraph_banks() {
        let mut memory = Pc9801Memory::new(MachineModel::PC9801VM, 0);
        let data = test_v98_font_rom_data();
        memory.load_font_rom(&data);

        let char_code = 0xF2u8;
        let chargraph16_base = 0x81000 + usize::from(char_code) * 16;
        assert_eq!(memory.font_rom[chargraph16_base], 0);
        assert_eq!(memory.font_rom[chargraph16_base + 1], 0);
        let expected_chargraph16 = expected_chargraph16_bytes(char_code);
        assert_eq!(
            memory.font_rom[chargraph16_base + 2],
            expected_chargraph16[2]
        );

        let chargraph8_base = 0x82000 + usize::from(char_code) * 16 + 8;
        assert_eq!(memory.font_rom[chargraph8_base], 0);
        let expected_chargraph8 = expected_chargraph8_bytes(char_code);
        assert_eq!(memory.font_rom[chargraph8_base + 1], expected_chargraph8[1]);
    }

    #[test]
    fn shadow_ram_not_allocated_for_v30() {
        let memory = Pc9801Memory::new(MachineModel::PC9801VM, 0);
        assert!(memory.state.shadow_ram.is_none());
    }

    #[test]
    fn shadow_ram_not_allocated_for_i286() {
        let memory = Pc9801Memory::new(MachineModel::PC9801VX, 0x100000);
        assert!(memory.state.shadow_ram.is_none());
    }

    #[test]
    fn shadow_ram_allocated_for_i386() {
        use super::BIOS_ROM_SIZE;
        let memory = Pc9801Memory::new(MachineModel::PC9801RA, 0x100000);
        assert!(memory.state.shadow_ram.is_some());
        assert_eq!(BIOS_ROM_SIZE, 0x18000);
    }

    #[test]
    fn shadow_ram_reads_rom_by_default() {
        let mut memory = Pc9801Memory::new(MachineModel::PC9801RA, 0x100000);
        let mut rom = vec![0xFFu8; 0x18000];
        rom[0] = 0xAB;
        memory.load_rom(&rom);
        assert_eq!(memory.read_byte(0xE8000), 0xAB);
    }

    #[test]
    fn shadow_ram_reads_ram_when_selected() {
        let mut memory = Pc9801Memory::new(MachineModel::PC9801RA, 0x100000);
        let mut rom = vec![0xFFu8; 0x18000];
        rom[0] = 0xAB;
        memory.load_rom(&rom);
        memory.state.shadow_ram.as_mut().unwrap()[0] = 0xCD;
        memory.set_shadow_control(0x02);
        assert_eq!(memory.read_byte(0xE8000), 0xCD);
    }

    #[test]
    fn shadow_ram_write_goes_to_ram_when_selected() {
        let mut memory = Pc9801Memory::new(MachineModel::PC9801RA, 0x100000);
        memory.load_rom(&vec![0xFFu8; 0x18000]);
        memory.set_shadow_control(0x02);
        memory.write_byte(0xE8000, 0x42);
        assert_eq!(memory.state.shadow_ram.as_ref().unwrap()[0], 0x42);
        assert_eq!(memory.rom[0], 0xFF);
    }

    #[test]
    fn shadow_ram_write_works_regardless_of_read_select() {
        let mut memory = Pc9801Memory::new(MachineModel::PC9801RA, 0x100000);
        memory.load_rom(&vec![0xFFu8; 0x18000]);
        // shadow_control=0x00 (bit 1=0, ROM reads) - writes should still go to shadow RAM.
        memory.write_byte(0xE8000, 0x42);
        assert_eq!(memory.state.shadow_ram.as_ref().unwrap()[0], 0x42);
        // Reads still return ROM data (bit 1 not set).
        assert_eq!(memory.read_byte(0xE8000), 0xFF);
    }

    #[test]
    fn shadow_ram_rom_to_ram_copy_sequence() {
        let mut memory = Pc9801Memory::new(MachineModel::PC9801RA, 0x100000);
        let mut rom = vec![0xFFu8; 0x18000];
        rom[0] = 0xAA;
        rom[0x17FFF] = 0xBB;
        memory.load_rom(&rom);

        // Step 1: Read ROM byte while in ROM-read mode (bit 1=0).
        let val_start = memory.read_byte(0xE8000);
        let val_end = memory.read_byte(0xFFFFF);
        assert_eq!(val_start, 0xAA);
        assert_eq!(val_end, 0xBB);

        // Step 2: Write to same address - goes to shadow RAM (bit 1 still 0).
        memory.write_byte(0xE8000, val_start);
        memory.write_byte(0xFFFFF, val_end);

        // Step 3: Switch to RAM-read mode (set bit 1).
        memory.set_shadow_control(0x02);

        // Step 4: Reads now return shadow RAM contents (the copied values).
        assert_eq!(memory.read_byte(0xE8000), 0xAA);
        assert_eq!(memory.read_byte(0xFFFFF), 0xBB);
    }

    #[test]
    fn shadow_ram_covers_full_bios_range() {
        let mut memory = Pc9801Memory::new(MachineModel::PC9801RA, 0x100000);
        memory.set_shadow_control(0x02);
        memory.write_byte(0xE8000, 0x11);
        memory.write_byte(0xFFFFF, 0x22);
        // Writes always go to shadow RAM for the full E8000-FFFFF range.
        assert_eq!(memory.state.shadow_ram.as_ref().unwrap()[0], 0x11);
        assert_eq!(memory.state.shadow_ram.as_ref().unwrap()[0x17FFF], 0x22);
        // Reads from E8000-F7FFF come from shadow RAM.
        assert_eq!(memory.read_byte(0xE8000), 0x11);
        // Reads from F8000-FFFFF come from ROM (bank-switched region),
        // not shadow RAM, so bank switching via port 043D still works.
        assert_eq!(memory.read_byte(0xFFFFF), 0x00);
    }

    #[test]
    fn sound_rom_respects_enable_bit_on_386() {
        let mut memory = Pc9801Memory::new(MachineModel::PC9801RA, 0x100000);
        memory.load_sound_rom(None);
        // On 386, shadow_control bit 7 clear => sound ROM hidden.
        assert_eq!(memory.read_byte(0xCC000), 0xFF);
        // Set bit 7 => sound ROM visible.
        memory.set_shadow_control(0x80);
        assert_ne!(memory.read_byte(0xCC000 + 0x2E00), 0xFF);
    }
}
