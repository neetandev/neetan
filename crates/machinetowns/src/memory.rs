//! FM Towns physical memory map and banking.
//!
//! The full 32-bit address space is decoded here: main and extended RAM, the
//! banked low-memory windows (FMR VRAM, DIC ROM, CMOS, and the SYSROM shadow),
//! the linear and interleaved VRAM windows, sprite RAM, and the native ROM and
//! CMOS windows high in the address space. The SYSROM is always mapped at the
//! top of the space so the CPU can fetch from the reset vector.
//!
//! Banking is controlled by three I/O registers, dispatched to this module by
//! the bus: 0x0404 (FMR VRAM vs main RAM for 0xC0000-0xDFFFF), 0x0480 (SYSROM
//! shadow and DIC/CMOS mapping), and 0x0484 (DIC ROM bank).

use crate::{config::TownsModel, rom::LoadedRoms};

/// Main RAM below the banked window region.
const MAIN_RAM_END: u32 = 0x000C_0000;

/// FMR compatibility window (plane VRAM, character VRAM, ANK font banks, FMR
/// registers) when FMR-VRAM mode is selected; otherwise main RAM.
const FMR_WINDOW_BASE: u32 = 0x000C_0000;
const FMR_WINDOW_END: u32 = 0x000D_0000;

/// FMR planar graphics VRAM (8 pixels per byte, 4 planes). Writes here are
/// translated into native packed VRAM; reads reconstruct a plane from it.
const FMR_GRAPHICS_END: u32 = 0x000C_8000;

/// Base of the FMR character/text VRAM (0xC8000). This region physically shares
/// the sprite RAM: the character codes at 0xC8000, the kanji JIS codes at
/// 0xCA000, and the sprite pattern/attribute RAM at native 0x81000000 are the
/// same bytes. The firmware font driver writes text through this window and
/// reads it back through the native sprite-RAM address to rasterize glyphs into
/// graphics VRAM, so the two views must alias the same buffer.
const FMR_CVRAM_BASE: u32 = 0x000C_8000;

/// FMR ANK font banks: 8x8 at 0xCA000, 8x16 at 0xCB000. When the ANK-font
/// overlay is enabled these read the FONT ROM at [`FONT_ANK8_OFFSET`] /
/// [`FONT_ANK16_OFFSET`]; otherwise they are character VRAM.
const FMR_ANK8_BASE: u32 = 0x000C_A000;
const FMR_ANK16_BASE: u32 = 0x000C_B000;
const FMR_ANK_END: u32 = 0x000C_C000;

/// FMR memory-mapped register block (mask, display mode, page, sync status,
/// kanji latch, ANK-font select).
const FMR_MMIO_BASE: u32 = 0x000C_FF80;
const FMR_MMIO_END: u32 = 0x000C_FFA0;
const FMR_REG_GVRAM_MASK: u32 = 0x000C_FF81;
const FMR_REG_GVRAM_DISP_MODE: u32 = 0x000C_FF82;
const FMR_REG_GVRAM_PAGE_SEL: u32 = 0x000C_FF83;
const FMR_REG_HSYNC_VSYNC: u32 = 0x000C_FF86;
const FMR_REG_KANJI_JIS_HIGH: u32 = 0x000C_FF94;
const FMR_REG_KANJI_JIS_LOW: u32 = 0x000C_FF95;
const FMR_REG_KANJI_PTN_HIGH: u32 = 0x000C_FF96;
const FMR_REG_KANJI_PTN_LOW: u32 = 0x000C_FF97;
const FMR_REG_KVRAM_OR_ANKFONT: u32 = 0x000C_FF99;

/// FMR page offset that page 1 maps to inside layer 0 (the lower half).
const FMR_PAGE_OFFSET: u32 = 0x0002_0000;

/// One kanji glyph occupies 32 bytes in the FONT ROM (16 rows, 2 bytes each).
const KANJI_GLYPH_BYTES: usize = 32;
/// The FONT ROM kanji index wraps at 8192 glyphs.
const KANJI_CODE_MASK: usize = 0x1FFF;

/// DIC ROM bank window (32 KiB of the dictionary ROM selected by 0x0484) when
/// DIC/CMOS mapping is enabled; otherwise main RAM.
const DIC_WINDOW_BASE: u32 = 0x000D_0000;
const DIC_WINDOW_END: u32 = 0x000D_8000;
const DIC_ROM_BANK_SIZE: u32 = 0x0000_8000;

/// CMOS / backup RAM window when DIC/CMOS mapping is enabled; otherwise main RAM.
const CMOS_WINDOW_BASE: u32 = 0x000D_8000;
const CMOS_WINDOW_END: u32 = 0x000D_A000;

/// Reserved RAM between the banked window region and the SYSROM shadow.
const RESERVED_RAM_BASE: u32 = 0x000D_A000;
const SYSROM_SHADOW_BASE: u32 = 0x000F_8000;
const SYSROM_SHADOW_END: u32 = 0x0010_0000;
const SYSROM_SHADOW_SIZE: usize = 0x0000_8000;

/// Extended RAM begins at 1 MiB.
const EXTENDED_RAM_BASE: u32 = 0x0010_0000;

/// Linear VRAM window (1 MiB of VRAM, mirrored across the window).
const VRAM_LINEAR_BASE: u32 = 0x8000_0000;
const VRAM_LINEAR_END: u32 = 0x8008_0000;

/// Single-page interleaved VRAM window.
const VRAM_INTERLEAVED_BASE: u32 = 0x8010_0000;
const VRAM_INTERLEAVED_END: u32 = 0x8018_0000;

/// MX high-resolution VRAM windows. Page 0 and page 1 are linear 512 KiB
/// apertures (page 1 displaced by one page); the third is a 1 MiB single-page
/// interleaved aperture spanning both pages.
const VRAM_HIGH_RES_PAGE0_BASE: u32 = 0x8200_0000;
const VRAM_HIGH_RES_PAGE0_END: u32 = 0x8208_0000;
const VRAM_HIGH_RES_PAGE1_BASE: u32 = 0x8280_0000;
const VRAM_HIGH_RES_PAGE1_END: u32 = 0x8288_0000;
const VRAM_HIGH_RES_SINGLE_BASE: u32 = 0x8300_0000;
const VRAM_HIGH_RES_SINGLE_END: u32 = 0x8310_0000;
/// The second high-res VRAM page sits 512 KiB into the 1 MiB VRAM.
const VRAM_HIGH_RES_PAGE1_DISPLACEMENT: u32 = 0x0008_0000;

/// VRAM is 1 MiB.
const VRAM_SIZE: usize = 0x0010_0000;
const VRAM_MASK: u32 = 0x000F_FFFF;

/// Sprite RAM (128 KiB).
const SPRITE_RAM_BASE: u32 = 0x8100_0000;
const SPRITE_RAM_END: u32 = 0x8102_0000;
const SPRITE_RAM_SIZE: usize = 0x0002_0000;
const SPRITE_RAM_MASK: u32 = 0x0001_FFFF;

/// Native OS (DOS) ROM window (512 KiB).
const OS_ROM_BASE: u32 = 0xC200_0000;
const OS_ROM_END: u32 = 0xC208_0000;
const OS_ROM_MASK: u32 = 0x0007_FFFF;

/// Native DIC ROM window (512 KiB).
const DIC_ROM_BASE: u32 = 0xC208_0000;
const DIC_ROM_END: u32 = 0xC210_0000;
const DIC_ROM_MASK: u32 = 0x0007_FFFF;

/// Native FONT ROM window (256 KiB).
const FONT_ROM_BASE: u32 = 0xC210_0000;
const FONT_ROM_END: u32 = 0xC214_0000;
const FONT_ROM_MASK: u32 = 0x0003_FFFF;

/// Native CMOS mirror window (8 KiB).
const CMOS_MIRROR_BASE: u32 = 0xC214_0000;
const CMOS_MIRROR_END: u32 = 0xC214_2000;

/// Native F20 font ROM window (512 KiB).
const FONT20_ROM_BASE: u32 = 0xC218_0000;
const FONT20_ROM_END: u32 = 0xC220_0000;
const FONT20_ROM_MASK: u32 = 0x0007_FFFF;

/// SYSROM, always mapped at the top of the 32-bit space (256 KiB). The reset
/// vector at 0xFFFFFFF0 lands here.
const SYSROM_BASE: u32 = 0xFFFC_0000;
const SYSROM_MASK: u32 = 0x0003_FFFF;

/// The SYSROM shadow at 0x000F8000 exposes the last 32 KiB of the 256 KiB SYSROM.
const SYSROM_SHADOW_SOURCE_OFFSET: usize = 0x0004_0000 - SYSROM_SHADOW_SIZE;

/// CMOS / backup RAM size (8 KiB).
const CMOS_SIZE: usize = 0x2000;

/// CMOS I/O window base. Each CMOS byte occupies two consecutive ports, so the
/// 0x3000-0x3FFF window reaches the low 2 KiB of the CMOS.
const CMOS_IO_BASE: u16 = 0x3000;

/// CMOS byte index of the boot-device *type* (I/O 0x3182: 1=HD, 2=FD, 8=CD).
const CMOS_BOOT_DEV_TYPE_INDEX: usize = ((0x3182 - 0x3000) / 2) as usize;
/// CMOS byte index of the boot device (I/O 0x3C28: 0x80=CD, 0x20=FD, 0x10=SCSI).
const CMOS_BOOT_DEV_INDEX: usize = ((0x3C28 - 0x3000) / 2) as usize;

/// CMOS drive-assignment table (I/O 0x31DC..0x321A): 16 drive-letter slots
/// (A..P), each a `(type, unit)` byte pair. `type` is 0 for a floppy, 2 for a
/// SCSI device, 5 for the ROM drive, or 0xFF for an unassigned slot. For a SCSI
/// device the `unit` byte packs `(scsi_id << 4) | partition`. Towns OS mounts a
/// drive letter for each populated slot; without a SCSI entry the OS never sees
/// a hard disk and falls back to the floppy.
const CMOS_DRIVE_ASSIGN_INDEX: usize = ((0x31DC - 0x3000) / 2) as usize;
/// Number of drive-letter slots (A..P) in the drive-assignment table.
const CMOS_DRIVE_ASSIGN_SLOTS: usize = 16;
/// Drive-assignment `type` byte for a SCSI device.
const CMOS_DRIVE_TYPE_SCSI: u8 = 0x02;
/// Marker for an unassigned drive-assignment slot.
const CMOS_DRIVE_TYPE_FREE: u8 = 0xFF;
/// CMOS byte index of the drive-assignment block checksum (I/O 0x33CE). The
/// checksummed block sums to a constant modulo 256, so a change to the table is
/// balanced by adjusting this byte.
const CMOS_DRIVE_CHECKSUM_INDEX: usize = ((0x33CE - 0x3000) / 2) as usize;

/// ANK 8x8 and 8x16 glyph offsets inside the FONT ROM, backing the FMR text
/// modes and the CA000/CB000 font banks.
const FONT_ANK8_OFFSET: usize = 0x3D000;
const FONT_ANK16_OFFSET: usize = 0x3D800;

/// Value returned for reads of unmapped physical addresses.
const OPEN_BUS: u8 = 0xFF;

/// Bytes returned when a `MEMSIZE` query divides the RAM size.
const BYTES_PER_MEGABYTE: usize = 1 << 20;

/// Default CMOS contents for the MX, dumped from our own emulator after the
/// Towns OS initialised it with the two floppy drives and the ROM drive
/// assigned (drives A/B/C) and no hard disk registered. Hard disks are added to
/// the drive-assignment table at insert time by [`TownsMemory::register_scsi_hdd`].
const MX_DEFAULT_CMOS: &[u8; CMOS_SIZE] = include_bytes!("cmos_default_mx.bin");

/// Default CMOS image for `model`.
const fn default_cmos(model: TownsModel) -> &'static [u8; CMOS_SIZE] {
    match model {
        TownsModel::FmTownsIICx => MX_DEFAULT_CMOS,
        TownsModel::FmTownsIIMx => MX_DEFAULT_CMOS,
    }
}

/// FM Towns physical memory and its low-memory banking state.
pub(crate) struct TownsMemory {
    /// Contiguous DRAM covering the low 1 MiB and the extended region. The
    /// banked windows overlay ROM/VRAM/CMOS on top of these addresses.
    ram: Box<[u8]>,
    /// Native VRAM (1 MiB).
    vram: Box<[u8]>,
    /// Sprite RAM (128 KiB).
    sprite_ram: Box<[u8]>,
    /// Battery-backed CMOS / backup RAM (8 KiB), seeded from the model defaults.
    cmos: Box<[u8]>,
    /// SYSROM image (256 KiB).
    system_rom: Box<[u8]>,
    /// OS (DOS) ROM image (512 KiB).
    os_rom: Box<[u8]>,
    /// DIC dictionary ROM image (512 KiB).
    dic_rom: Box<[u8]>,
    /// FONT ROM image (256 KiB).
    font_rom: Box<[u8]>,
    /// F20 font ROM image (512 KiB).
    font20_rom: Box<[u8]>,
    /// Serial machine-identity ROM (32 bytes), read one bit at a time through
    /// the I/O 0x0032 serial-EEPROM interface.
    serial_rom: Box<[u8]>,
    /// FMR-VRAM mode flag (0x0404). When true (bit 7 clear), 0xC0000-0xCFFFF
    /// maps the FMR VRAM window; when false, the whole 0xC0000-0xDFFFF area is
    /// main RAM.
    fmr_vram: bool,
    /// SYSROM-shadow flag (0x0480 bit 1 clear). When true, 0xF8000-0xFFFFF reads
    /// the SYSROM; when false, it reads main RAM.
    system_rom_shadow: bool,
    /// DIC/CMOS mapping flag (0x0480 bit 0 set). When true, the DIC ROM and CMOS
    /// are mapped into 0xD0000-0xD9FFF; when false, that area is main RAM.
    dic_rom_mapped: bool,
    /// Selected 32 KiB DIC ROM bank for the 0xD0000 window (0x0484).
    dic_rom_bank: u8,
    /// 32-bit native VRAM write mask, one byte per address lane (I/O 0x045A/B).
    /// Defaults to all-ones (writes pass through unchanged).
    native_vram_mask: [u8; 4],
    /// True while `native_vram_mask` is not all-ones, so the write path applies
    /// read-modify-write masking; false selects the plain fast path.
    vram_mask_active: bool,
    /// Selects which 16-bit half of `native_vram_mask` I/O 0x045A/B writes to
    /// (I/O 0x0458).
    vram_mask_latch: u8,
    /// FMR planar plane mask (CFF81 / I/O 0xFF81); low nibble is the write plane
    /// set, bits 6-7 select the read plane.
    fmr_vram_mask: u8,
    /// FMR display plane mask (CFF82), consumed by the renderer in FMR mode.
    fmr_display_planes: u8,
    /// FMR display page offset into layer 0 (CFF82 bit 4).
    fmr_display_page_offset: u32,
    /// FMR write page offset into layer 0 (CFF83 bit 4): 0 or [`FMR_PAGE_OFFSET`].
    fmr_write_page_offset: u32,
    /// Kanji ROM access latch (CFF94/CFF95) and current glyph row (CFF96/CFF97).
    kanji_jis_high: u8,
    kanji_jis_low: u8,
    kanji_row: u8,
    /// ANK-font overlay flag (CFF99 bit 0): overlays FONT ROM at CA000/CB000.
    ank_font_overlay: bool,
    /// TVRAM dirty flag: set when the character/text VRAM is written, reported
    /// and cleared through the 0x05C8 status port so the firmware font driver
    /// knows when to re-rasterize the text plane into graphics VRAM.
    tvram_written: bool,
    /// Current CRTC vertical-sync state, mirrored for the CFF86 status read.
    vsync_active: bool,
    /// Current CRTC horizontal-sync state, mirrored for the CFF86 status read.
    hsync_active: bool,
    /// Whether the MX high-resolution VRAM windows (0x82000000/0x82800000/
    /// 0x83000000) are decoded. Only the high-res-capable models expose them.
    high_res_available: bool,
}

impl TownsMemory {
    /// Builds the memory map for `model` from a loaded ROM set.
    pub(crate) fn new(model: TownsModel, roms: LoadedRoms) -> Self {
        let total_ram = EXTENDED_RAM_BASE as usize + model.extended_ram_size();
        let mut memory = Self {
            ram: vec![0u8; total_ram].into_boxed_slice(),
            vram: vec![0u8; VRAM_SIZE].into_boxed_slice(),
            sprite_ram: vec![0u8; SPRITE_RAM_SIZE].into_boxed_slice(),
            cmos: default_cmos(model).to_vec().into_boxed_slice(),
            system_rom: roms.system.into_boxed_slice(),
            os_rom: roms.dos.into_boxed_slice(),
            dic_rom: roms.dictionary.into_boxed_slice(),
            font_rom: roms.font.into_boxed_slice(),
            font20_rom: roms.f20.into_boxed_slice(),
            serial_rom: roms.serial.into_boxed_slice(),
            fmr_vram: true,
            system_rom_shadow: true,
            dic_rom_mapped: false,
            dic_rom_bank: 0,
            native_vram_mask: [0xFF; 4],
            vram_mask_active: false,
            vram_mask_latch: 0,
            fmr_vram_mask: 0x0F,
            fmr_display_planes: 0x0F,
            fmr_display_page_offset: 0,
            fmr_write_page_offset: 0,
            kanji_jis_high: 0,
            kanji_jis_low: 0,
            kanji_row: 0,
            ank_font_overlay: false,
            tvram_written: false,
            vsync_active: false,
            hsync_active: false,
            high_res_available: model.high_res_available(),
        };
        memory.reset_banking();
        memory
    }

    /// Restores the power-on banking configuration.
    pub(crate) fn reset_banking(&mut self) {
        self.fmr_vram = true;
        self.system_rom_shadow = true;
        self.dic_rom_mapped = false;
        self.dic_rom_bank = 0;
    }

    /// Whether the FMR compatibility window (0xC0000-0xCFFFF) is currently
    /// mapped, so its memory-mapped registers (e.g. the buzzer control at
    /// 0xCFF98) respond instead of plain RAM.
    pub(crate) fn fmr_window_mapped(&self) -> bool {
        self.fmr_vram
    }

    /// Reads and clears the TVRAM dirty flag (I/O 0x05C8): returns 0x80 when the
    /// text VRAM has been written since the last read, otherwise 0x00. The
    /// firmware font driver polls this to decide when to re-rasterize the text
    /// plane.
    pub(crate) fn take_tvram_written(&mut self) -> u8 {
        if self.tvram_written {
            self.tvram_written = false;
            0x80
        } else {
            0x00
        }
    }

    /// Reads a byte from a physical address. Takes `&mut self` because some FMR
    /// memory-mapped reads (the kanji glyph port) advance internal state.
    pub(crate) fn read_byte(&mut self, address: u32) -> u8 {
        match address {
            0..MAIN_RAM_END => self.ram[address as usize],
            FMR_WINDOW_BASE..FMR_WINDOW_END => {
                if self.fmr_vram {
                    self.read_fmr_window(address)
                } else {
                    self.ram[address as usize]
                }
            }
            DIC_WINDOW_BASE..DIC_WINDOW_END => {
                if self.fmr_vram && self.dic_rom_mapped {
                    let bank_offset = self.dic_rom_bank as u32 * DIC_ROM_BANK_SIZE;
                    self.dic_rom[(bank_offset + (address - DIC_WINDOW_BASE)) as usize]
                } else {
                    self.ram[address as usize]
                }
            }
            CMOS_WINDOW_BASE..CMOS_WINDOW_END => {
                if self.fmr_vram && self.dic_rom_mapped {
                    self.cmos[(address - CMOS_WINDOW_BASE) as usize]
                } else {
                    self.ram[address as usize]
                }
            }
            RESERVED_RAM_BASE..SYSROM_SHADOW_BASE => self.ram[address as usize],
            SYSROM_SHADOW_BASE..SYSROM_SHADOW_END => {
                if self.system_rom_shadow {
                    let offset =
                        SYSROM_SHADOW_SOURCE_OFFSET + (address - SYSROM_SHADOW_BASE) as usize;
                    self.system_rom[offset]
                } else {
                    self.ram[address as usize]
                }
            }
            VRAM_LINEAR_BASE..VRAM_LINEAR_END => {
                self.vram[((address - VRAM_LINEAR_BASE) & VRAM_MASK) as usize]
            }
            VRAM_INTERLEAVED_BASE..VRAM_INTERLEAVED_END => {
                self.vram[interleaved_offset(address - VRAM_INTERLEAVED_BASE) as usize]
            }
            VRAM_HIGH_RES_PAGE0_BASE..VRAM_HIGH_RES_PAGE0_END if self.high_res_available => {
                self.vram[((address - VRAM_HIGH_RES_PAGE0_BASE) & VRAM_MASK) as usize]
            }
            VRAM_HIGH_RES_PAGE1_BASE..VRAM_HIGH_RES_PAGE1_END if self.high_res_available => {
                let offset = (address - VRAM_HIGH_RES_PAGE1_BASE
                    + VRAM_HIGH_RES_PAGE1_DISPLACEMENT)
                    & VRAM_MASK;
                self.vram[offset as usize]
            }
            VRAM_HIGH_RES_SINGLE_BASE..VRAM_HIGH_RES_SINGLE_END if self.high_res_available => {
                self.vram[high_res_interleaved_offset(address - VRAM_HIGH_RES_SINGLE_BASE) as usize]
            }
            SPRITE_RAM_BASE..SPRITE_RAM_END => {
                self.sprite_ram[((address - SPRITE_RAM_BASE) & SPRITE_RAM_MASK) as usize]
            }
            OS_ROM_BASE..OS_ROM_END => {
                self.os_rom[((address - OS_ROM_BASE) & OS_ROM_MASK) as usize]
            }
            DIC_ROM_BASE..DIC_ROM_END => {
                self.dic_rom[((address - DIC_ROM_BASE) & DIC_ROM_MASK) as usize]
            }
            FONT_ROM_BASE..FONT_ROM_END => {
                self.font_rom[((address - FONT_ROM_BASE) & FONT_ROM_MASK) as usize]
            }
            CMOS_MIRROR_BASE..CMOS_MIRROR_END => self.cmos[(address - CMOS_MIRROR_BASE) as usize],
            FONT20_ROM_BASE..FONT20_ROM_END => {
                self.font20_rom[((address - FONT20_ROM_BASE) & FONT20_ROM_MASK) as usize]
            }
            SYSROM_BASE..=u32::MAX => {
                self.system_rom[((address - SYSROM_BASE) & SYSROM_MASK) as usize]
            }
            _ => {
                if (address as usize) < self.ram.len() {
                    self.ram[address as usize]
                } else {
                    OPEN_BUS
                }
            }
        }
    }

    /// Writes a byte to a physical address. Writes to ROM windows are ignored.
    pub(crate) fn write_byte(&mut self, address: u32, value: u8) {
        match address {
            0..MAIN_RAM_END => self.ram[address as usize] = value,
            FMR_WINDOW_BASE..FMR_WINDOW_END => {
                if self.fmr_vram {
                    self.write_fmr_window(address, value);
                } else {
                    self.ram[address as usize] = value;
                }
            }
            DIC_WINDOW_BASE..DIC_WINDOW_END => {
                if self.fmr_vram && self.dic_rom_mapped {
                    // The DIC ROM window is read-only; ignore writes.
                } else {
                    self.ram[address as usize] = value;
                }
            }
            CMOS_WINDOW_BASE..CMOS_WINDOW_END => {
                if self.fmr_vram && self.dic_rom_mapped {
                    self.cmos[(address - CMOS_WINDOW_BASE) as usize] = value;
                } else {
                    self.ram[address as usize] = value;
                }
            }
            RESERVED_RAM_BASE..SYSROM_SHADOW_BASE => self.ram[address as usize] = value,
            SYSROM_SHADOW_BASE..SYSROM_SHADOW_END => {
                if !self.system_rom_shadow {
                    self.ram[address as usize] = value;
                }
                // The SYSROM shadow is read-only; ignore writes while mapped.
            }
            VRAM_LINEAR_BASE..VRAM_LINEAR_END => {
                let offset = (address - VRAM_LINEAR_BASE) & VRAM_MASK;
                self.write_vram_masked(offset, value);
            }
            VRAM_INTERLEAVED_BASE..VRAM_INTERLEAVED_END => {
                let offset = interleaved_offset(address - VRAM_INTERLEAVED_BASE);
                self.write_vram_masked(offset, value);
            }
            VRAM_HIGH_RES_PAGE0_BASE..VRAM_HIGH_RES_PAGE0_END if self.high_res_available => {
                let offset = (address - VRAM_HIGH_RES_PAGE0_BASE) & VRAM_MASK;
                self.write_vram_masked(offset, value);
            }
            VRAM_HIGH_RES_PAGE1_BASE..VRAM_HIGH_RES_PAGE1_END if self.high_res_available => {
                let offset = (address - VRAM_HIGH_RES_PAGE1_BASE
                    + VRAM_HIGH_RES_PAGE1_DISPLACEMENT)
                    & VRAM_MASK;
                self.write_vram_masked(offset, value);
            }
            VRAM_HIGH_RES_SINGLE_BASE..VRAM_HIGH_RES_SINGLE_END if self.high_res_available => {
                let offset = high_res_interleaved_offset(address - VRAM_HIGH_RES_SINGLE_BASE);
                self.write_vram_masked(offset, value);
            }
            SPRITE_RAM_BASE..SPRITE_RAM_END => {
                self.sprite_ram[((address - SPRITE_RAM_BASE) & SPRITE_RAM_MASK) as usize] = value;
            }
            CMOS_MIRROR_BASE..CMOS_MIRROR_END => {
                self.cmos[(address - CMOS_MIRROR_BASE) as usize] = value;
            }
            // ROM windows and the reset SYSROM ignore writes.
            OS_ROM_BASE..OS_ROM_END
            | DIC_ROM_BASE..DIC_ROM_END
            | FONT_ROM_BASE..FONT_ROM_END
            | FONT20_ROM_BASE..FONT20_ROM_END
            | SYSROM_BASE..=u32::MAX => {}
            _ => {
                if (address as usize) < self.ram.len() {
                    self.ram[address as usize] = value;
                }
            }
        }
    }

    /// Handles a write to the FMR-VRAM select register (0x0404). Bit 7 clear
    /// selects FMR VRAM; bit 7 set forces the 0xC0000-0xDFFFF area to main RAM.
    pub(crate) fn write_fmr_vram_select(&mut self, value: u8) {
        self.fmr_vram = value & 0x80 == 0;
    }

    /// Reads back the FMR-VRAM select register (0x0404).
    pub(crate) fn read_fmr_vram_select(&self) -> u8 {
        if self.fmr_vram { 0x00 } else { 0x80 }
    }

    /// Handles a write to the SYSROM/DIC mapping register (0x0480). Bit 1 clear
    /// maps the SYSROM shadow; bit 0 set maps the DIC ROM and CMOS.
    pub(crate) fn write_sysrom_dic_select(&mut self, value: u8) {
        self.system_rom_shadow = value & 0x02 == 0;
        self.dic_rom_mapped = value & 0x01 != 0;
    }

    /// Reads back the SYSROM/DIC mapping register (0x0480).
    pub(crate) fn read_sysrom_dic_select(&self) -> u8 {
        let mut value = 0;
        if !self.system_rom_shadow {
            value |= 0x02;
        }
        if self.dic_rom_mapped {
            value |= 0x01;
        }
        value
    }

    /// Handles a write to the DIC ROM bank register (0x0484).
    pub(crate) fn write_dic_rom_bank(&mut self, value: u8) {
        self.dic_rom_bank = value & 0x0F;
    }

    /// Reads back the DIC ROM bank register (0x0484).
    pub(crate) fn read_dic_rom_bank(&self) -> u8 {
        self.dic_rom_bank
    }

    /// Reads a CMOS byte through the I/O window (0x3000-0x3FFF), where each CMOS
    /// byte occupies two consecutive port addresses.
    pub(crate) fn read_cmos_io(&self, port: u16) -> u8 {
        let index = ((port - CMOS_IO_BASE) / 2) as usize;
        self.cmos.get(index).copied().unwrap_or(0xFF)
    }

    /// Writes a CMOS byte through the I/O window (0x3000-0x3FFF).
    pub(crate) fn write_cmos_io(&mut self, port: u16, value: u8) {
        let index = ((port - CMOS_IO_BASE) / 2) as usize;
        if let Some(byte) = self.cmos.get_mut(index) {
            *byte = value;
        }
    }

    /// Registers a SCSI hard-disk partition in the CMOS drive-assignment table
    /// so the Towns OS mounts a drive letter for it. Without this the OS never
    /// sees the disk and cannot boot from it. The partition is placed in the
    /// first free slot (drive letter) and the block checksum is repaired.
    pub(crate) fn register_scsi_hdd(&mut self, scsi_id: u8, partition: u8) {
        let unit = (scsi_id << 4) | (partition & 0x0F);
        for slot in 0..CMOS_DRIVE_ASSIGN_SLOTS {
            let type_index = CMOS_DRIVE_ASSIGN_INDEX + slot * 2;
            let unit_index = type_index + 1;
            if self.cmos[type_index] != CMOS_DRIVE_TYPE_FREE {
                continue;
            }
            let old_sum = i32::from(self.cmos[type_index]) + i32::from(self.cmos[unit_index]);
            self.cmos[type_index] = CMOS_DRIVE_TYPE_SCSI;
            self.cmos[unit_index] = unit;
            let new_sum = i32::from(CMOS_DRIVE_TYPE_SCSI) + i32::from(unit);
            let checksum = self.cmos[CMOS_DRIVE_CHECKSUM_INDEX];
            self.cmos[CMOS_DRIVE_CHECKSUM_INDEX] =
                (i32::from(checksum) - (new_sum - old_sum)).rem_euclid(256) as u8;
            return;
        }
    }

    /// Sets the CMOS boot-device type and boot-device bytes the SYSROM IPL reads.
    pub(crate) fn set_boot_device_cmos(&mut self, device_type: u8, boot_device: u8) {
        if let Some(byte) = self.cmos.get_mut(CMOS_BOOT_DEV_TYPE_INDEX) {
            *byte = device_type;
        }
        if let Some(byte) = self.cmos.get_mut(CMOS_BOOT_DEV_INDEX) {
            *byte = boot_device;
        }
    }

    /// Total installed RAM in megabytes, for the 0x05E8 MEMSIZE query.
    pub(crate) fn total_ram_megabytes(&self) -> u8 {
        (self.ram.len() / BYTES_PER_MEGABYTE) as u8
    }

    /// The FONT ROM image (for the video/text path in a later phase).
    pub(crate) fn font_rom(&self) -> &[u8] {
        &self.font_rom
    }

    /// The serial machine-identity ROM, read through the I/O 0x0032 interface.
    pub(crate) fn serial_rom(&self) -> &[u8] {
        &self.serial_rom
    }

    /// The native VRAM image, for the renderer.
    pub(crate) fn vram(&self) -> &[u8] {
        &self.vram
    }

    /// Blits the enabled sprites into VRAM layer 1. Kept here so the mutable VRAM
    /// and the read-only sprite RAM are borrowed as disjoint fields of `self`.
    pub(crate) fn render_sprites(&mut self, params: &software_renderer::SpriteRenderParams) {
        software_renderer::render_sprites(&mut self.vram, &self.sprite_ram, params);
    }

    /// The FMR display plane mask (CFF82), applied by the renderer in FMR mode.
    pub(crate) fn fmr_display_planes(&self) -> u8 {
        self.fmr_display_planes
    }

    /// The FMR display page offset (CFF82 bit 4), applied to layer 0.
    pub(crate) fn fmr_display_page_offset(&self) -> usize {
        self.fmr_display_page_offset as usize
    }

    /// Mirrors the CRTC sync state so the CFF86 status read reflects it.
    pub(crate) fn set_sync_status(&mut self, vsync: bool, hsync: bool) {
        self.vsync_active = vsync;
        self.hsync_active = hsync;
    }

    /// Writes one native VRAM byte, applying the 32-bit plane mask when active.
    fn write_vram_masked(&mut self, offset: u32, value: u8) {
        let index = offset as usize;
        if self.vram_mask_active {
            let mask = self.native_vram_mask[(offset & 3) as usize];
            self.vram[index] = (self.vram[index] & !mask) | (value & mask);
        } else {
            self.vram[index] = value;
        }
    }

    /// Selects the 16-bit half of the plane mask that 0x045A/0x045B update
    /// (I/O 0x0458).
    pub(crate) fn write_vram_mask_latch(&mut self, value: u8) {
        self.vram_mask_latch = value & 1;
    }

    /// Reads back the plane-mask half latch (I/O 0x0458).
    pub(crate) fn read_vram_mask_latch(&self) -> u8 {
        self.vram_mask_latch
    }

    /// Writes the low mask byte of the selected half (I/O 0x045A).
    pub(crate) fn write_vram_mask_low(&mut self, value: u8) {
        self.native_vram_mask[(self.vram_mask_latch << 1) as usize] = value;
        self.refresh_vram_mask_active();
    }

    /// Writes the high mask byte of the selected half (I/O 0x045B).
    pub(crate) fn write_vram_mask_high(&mut self, value: u8) {
        self.native_vram_mask[((self.vram_mask_latch << 1) + 1) as usize] = value;
        self.refresh_vram_mask_active();
    }

    /// Reads back the low mask byte of the selected half (I/O 0x045A).
    pub(crate) fn read_vram_mask_low(&self) -> u8 {
        self.native_vram_mask[(self.vram_mask_latch << 1) as usize]
    }

    /// Reads back the high mask byte of the selected half (I/O 0x045B).
    pub(crate) fn read_vram_mask_high(&self) -> u8 {
        self.native_vram_mask[((self.vram_mask_latch << 1) + 1) as usize]
    }

    /// Recomputes whether the plane mask requires read-modify-write masking.
    fn refresh_vram_mask_active(&mut self) {
        self.vram_mask_active = self.native_vram_mask != [0xFF; 4];
    }

    /// Reads a byte from the FMR compatibility window: planar graphics, the ANK
    /// font banks, the CFF80 register block, or character VRAM.
    fn read_fmr_window(&mut self, address: u32) -> u8 {
        match address {
            FMR_WINDOW_BASE..FMR_GRAPHICS_END => self.read_fmr_planar(address - FMR_WINDOW_BASE),
            FMR_MMIO_BASE..FMR_MMIO_END => self.read_fmr_register(address),
            FMR_ANK8_BASE..FMR_ANK16_BASE if self.ank_font_overlay => {
                self.font_rom[FONT_ANK8_OFFSET + (address - FMR_ANK8_BASE) as usize]
            }
            FMR_ANK16_BASE..FMR_ANK_END if self.ank_font_overlay => {
                self.font_rom[FONT_ANK16_OFFSET + (address - FMR_ANK16_BASE) as usize]
            }
            _ => self.sprite_ram[((address - FMR_CVRAM_BASE) & SPRITE_RAM_MASK) as usize],
        }
    }

    /// Writes a byte to the FMR compatibility window.
    fn write_fmr_window(&mut self, address: u32, value: u8) {
        match address {
            FMR_WINDOW_BASE..FMR_GRAPHICS_END => {
                self.write_fmr_planar(address - FMR_WINDOW_BASE, value)
            }
            FMR_MMIO_BASE..FMR_MMIO_END => self.write_fmr_register(address, value),
            _ => {
                self.sprite_ram[((address - FMR_CVRAM_BASE) & SPRITE_RAM_MASK) as usize] = value;
                self.tvram_written = true;
            }
        }
    }

    /// Translates an FMR planar graphics write into four native packed VRAM
    /// bytes, spreading the eight source pixels across the plane set selected by
    /// the low nibble of the FMR mask.
    fn write_fmr_planar(&mut self, fmr_offset: u32, value: u8) {
        let vram_addr = ((fmr_offset << 2) + self.fmr_write_page_offset) as usize;
        let mask_low = self.fmr_vram_mask & 0x0F;
        let mask_high = mask_low << 4;
        let clear = !(mask_low | mask_high);
        let mut bit_high = 0x40u8;
        let mut bit_low = 0x80u8;
        for lane in 0..4 {
            let byte = &mut self.vram[vram_addr + lane];
            *byte &= clear;
            if value & bit_high != 0 {
                *byte |= mask_high;
            }
            if value & bit_low != 0 {
                *byte |= mask_low;
            }
            bit_high >>= 2;
            bit_low >>= 2;
        }
    }

    /// Reconstructs an FMR planar graphics byte from native packed VRAM, reading
    /// back the single plane selected by bits 6-7 of the FMR mask.
    fn read_fmr_planar(&self, fmr_offset: u32) -> u8 {
        let vram_addr = ((fmr_offset << 2) + self.fmr_write_page_offset) as usize;
        let shift = (self.fmr_vram_mask >> 6) & 3;
        let test_high = 0x10u8 << shift;
        let test_low = 1u8 << shift;
        let mut bit_high = 0x40u8;
        let mut bit_low = 0x80u8;
        let mut data = 0u8;
        for lane in 0..4 {
            let byte = self.vram[vram_addr + lane];
            if byte & test_high != 0 {
                data |= bit_high;
            }
            if byte & test_low != 0 {
                data |= bit_low;
            }
            bit_high >>= 2;
            bit_low >>= 2;
        }
        data
    }

    /// Reads an FMR memory-mapped register (CFF80 block).
    fn read_fmr_register(&mut self, address: u32) -> u8 {
        match address {
            FMR_REG_GVRAM_MASK => self.fmr_vram_mask,
            FMR_REG_GVRAM_PAGE_SEL => {
                if self.fmr_write_page_offset == 0 {
                    0x00
                } else {
                    0x10
                }
            }
            FMR_REG_HSYNC_VSYNC => {
                let mut data = 0x10;
                if self.vsync_active {
                    data |= 0x04;
                }
                if self.hsync_active {
                    data |= 0x80;
                }
                data
            }
            FMR_REG_KANJI_JIS_HIGH => 0x80,
            FMR_REG_KANJI_PTN_HIGH => {
                let code = self.font_rom_code() & KANJI_CODE_MASK;
                self.font_rom[KANJI_GLYPH_BYTES * code + usize::from(self.kanji_row) * 2]
            }
            FMR_REG_KANJI_PTN_LOW => {
                let code = self.font_rom_code() & KANJI_CODE_MASK;
                let byte =
                    self.font_rom[KANJI_GLYPH_BYTES * code + usize::from(self.kanji_row) * 2 + 1];
                self.kanji_row = (self.kanji_row + 1) & 0x0F;
                byte
            }
            _ => 0xFF,
        }
    }

    /// Writes an FMR register through its I/O-port alias. The CFF80 register
    /// block is mirrored at I/O ports 0xFF80-0xFF9F; the FMR-compatible boot code
    /// drives the plane mask (0xFF81), display mode (0xFF82), and page select
    /// (0xFF83) through these ports rather than the memory-mapped window.
    pub(crate) fn write_fmr_io_register(&mut self, port: u16, value: u8) {
        self.write_fmr_register(FMR_MMIO_BASE | u32::from(port & 0x00FF), value);
    }

    /// Reads an FMR register through its I/O-port alias (0xFF80-0xFF9F).
    pub(crate) fn read_fmr_io_register(&mut self, port: u16) -> u8 {
        self.read_fmr_register(FMR_MMIO_BASE | u32::from(port & 0x00FF))
    }

    /// Writes an FMR memory-mapped register (CFF80 block).
    fn write_fmr_register(&mut self, address: u32, value: u8) {
        match address {
            FMR_REG_GVRAM_MASK => self.fmr_vram_mask = value,
            FMR_REG_GVRAM_DISP_MODE => {
                self.fmr_display_planes = ((value >> 2) & 0x08) | (value & 0x07);
                self.fmr_display_page_offset = if value & 0x10 != 0 {
                    FMR_PAGE_OFFSET
                } else {
                    0
                };
            }
            FMR_REG_GVRAM_PAGE_SEL => {
                self.fmr_write_page_offset = if value & 0x10 != 0 {
                    FMR_PAGE_OFFSET
                } else {
                    0
                };
            }
            FMR_REG_KANJI_JIS_HIGH => self.kanji_jis_high = value & 0x7F,
            FMR_REG_KANJI_JIS_LOW => {
                self.kanji_jis_low = value;
                self.kanji_row = 0;
            }
            FMR_REG_KVRAM_OR_ANKFONT => self.ank_font_overlay = value & 1 != 0,
            _ => {}
        }
    }

    /// Maps the latched JIS code to a FONT ROM glyph index, matching the FM Towns
    /// kanji ROM layout (32x8 blocks below JIS row 0x28, 32x16 blocks above).
    fn font_rom_code(&self) -> usize {
        let jis_high = usize::from(self.kanji_jis_high);
        let jis_low = usize::from(self.kanji_jis_low);
        if jis_high < 0x28 {
            let mut block = (jis_low.wrapping_sub(0x20)) >> 5;
            let x = jis_low & 0x1F;
            let y = jis_high & 7;
            if block == 1 {
                block = 2;
            } else if block == 2 {
                block = 1;
            }
            block * 32 * 8 + y * 32 + x
        } else {
            let block_x = (jis_low.wrapping_sub(0x20)) >> 5;
            let block_y = (jis_high.wrapping_sub(0x30)) >> 4;
            let block = block_y * 3 + block_x;
            let x = jis_low & 0x1F;
            let y = jis_high & 0x0F;
            0x400 + block * 32 * 16 + y * 32 + x
        }
    }
}

/// Applies the single-page interleaved VRAM address transform and masks the
/// result into the 1 MiB VRAM.
fn interleaved_offset(offset: u32) -> u32 {
    (((offset & 4) << 16) | ((offset & 0x7_FFF8) >> 1) | (offset & 3)) & VRAM_MASK
}

/// The high-res single-page interleaved transform: the same 4-byte swizzle as
/// [`interleaved_offset`] plus the 0x80000 page-select bit.
fn high_res_interleaved_offset(offset: u32) -> u32 {
    ((offset & 0x0008_0000) | ((offset & 4) << 16) | ((offset & 0x7_FFF8) >> 1) | (offset & 3))
        & VRAM_MASK
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_roms() -> LoadedRoms {
        LoadedRoms {
            dos: vec![0x11; 0x8_0000],
            font: vec![0x22; 0x4_0000],
            system: (0..0x4_0000).map(|i| i as u8).collect(),
            f20: vec![0x33; 0x8_0000],
            // Each 32 KiB DIC bank gets a distinct constant (its bank index).
            dictionary: (0..0x8_0000).map(|i| (i / 0x8000) as u8).collect(),
            serial: vec![0x44; 0x20],
        }
    }

    fn test_memory() -> TownsMemory {
        TownsMemory::new(TownsModel::FmTownsIIMx, test_roms())
    }

    #[test]
    fn cvram_window_aliases_native_sprite_ram() {
        // The FMR character VRAM window (0xC8000+) physically shares the native
        // sprite RAM at 0x81000000. The firmware font driver writes glyph cells
        // through the window while the renderer reads them through the native
        // address, so both views must observe the same bytes.
        let mut memory = test_memory();
        memory.write_byte(0x000C_8000, 0x41);
        memory.write_byte(0x000C_9500, 0xA7);
        assert_eq!(memory.read_byte(0x8100_0000), 0x41);
        assert_eq!(memory.read_byte(0x8100_1500), 0xA7);

        // A native write is visible through the window.
        memory.write_byte(0x8100_0002, 0x5A);
        assert_eq!(memory.read_byte(0x000C_8002), 0x5A);
    }

    #[test]
    fn cvram_write_sets_self_clearing_tvram_dirty_flag() {
        // I/O 0x05C8 reports (bit 7) whether the text VRAM was written since the
        // last read, then clears. The firmware polls it to know when to redraw.
        let mut memory = test_memory();
        assert_eq!(memory.take_tvram_written(), 0x00);
        memory.write_byte(0x000C_8000, 0x41);
        assert_eq!(memory.take_tvram_written(), 0x80);
        assert_eq!(memory.take_tvram_written(), 0x00);
    }

    #[test]
    fn reset_vector_reads_from_sysrom_top() {
        let mut memory = test_memory();
        // 0xFFFFFFF0 maps to SYSROM offset 0x3FFF0.
        let offset = (0xFFFF_FFF0u32 & SYSROM_MASK) as u8;
        assert_eq!(memory.read_byte(0xFFFF_FFF0), offset);
    }

    #[test]
    fn sysrom_shadow_exposes_last_32k() {
        let mut memory = test_memory();
        // 0xF8000 shadows SYSROM offset 0x38000.
        let expected = (SYSROM_SHADOW_SOURCE_OFFSET as u32 & SYSROM_MASK) as u8;
        assert_eq!(memory.read_byte(0x000F_8000), expected);
        // The last shadow byte matches the last SYSROM byte.
        assert_eq!(memory.read_byte(0x000F_FFFF), memory.read_byte(0xFFFF_FFFF));
    }

    #[test]
    fn main_and_extended_ram_round_trip() {
        let mut memory = test_memory();
        memory.write_byte(0x0000_1234, 0xA5);
        assert_eq!(memory.read_byte(0x0000_1234), 0xA5);
        memory.write_byte(0x0020_0000, 0x5A);
        assert_eq!(memory.read_byte(0x0020_0000), 0x5A);
    }

    #[test]
    fn banking_0480_truth_table() {
        let mut memory = test_memory();
        // Seed the DRAM behind the F8000 and D8000 windows.
        memory.write_byte_raw_ram(0x000F_8000, 0xDD);
        memory.write_byte_raw_ram(0x000D_8000, 0xCC);

        // Reset defaults: SYSROM shadow on, DIC/CMOS off.
        assert_ne!(memory.read_byte(0x000F_8000), 0xDD); // reads SYSROM
        assert_eq!(memory.read_byte(0x000D_8000), 0xCC); // reads RAM

        // bit1 set -> F8000 reads RAM; bit0 set -> D8000 reads CMOS.
        memory.write_sysrom_dic_select(0x03);
        assert_eq!(memory.read_byte(0x000F_8000), 0xDD);
        assert_eq!(memory.read_byte(0x000D_8000), MX_DEFAULT_CMOS[0]);

        // 0x0404 bit7 set forces the whole area to RAM, overriding DIC/CMOS.
        memory.write_fmr_vram_select(0x80);
        assert_eq!(memory.read_byte(0x000D_8000), 0xCC);
    }

    #[test]
    fn cmos_window_and_mirror_are_writable() {
        let mut memory = test_memory();
        memory.write_sysrom_dic_select(0x01); // map DIC/CMOS
        memory.write_byte(0x000D_8010, 0x7E);
        assert_eq!(memory.read_byte(0x000D_8010), 0x7E);
        // The native mirror at 0xC2140000 sees the same CMOS byte.
        assert_eq!(memory.read_byte(0xC214_0010), 0x7E);
    }

    #[test]
    fn register_scsi_hdd_populates_drive_assign_and_balances_checksum() {
        let mut memory = test_memory();
        let table = CMOS_DRIVE_ASSIGN_INDEX;
        let checksum = CMOS_DRIVE_CHECKSUM_INDEX;

        // The default fixture assigns A/B to floppies and C to the ROM drive;
        // slot D (index 3) is the first free entry.
        assert_eq!(memory.cmos[table + 6], CMOS_DRIVE_TYPE_FREE);
        let checksum_before = memory.cmos[checksum];

        memory.register_scsi_hdd(0, 0);

        // Drive D now maps to SCSI id 0, partition 0.
        assert_eq!(memory.cmos[table + 6], CMOS_DRIVE_TYPE_SCSI);
        assert_eq!(memory.cmos[table + 7], 0x00);
        // The checksummed block sums to a constant, so the checksum byte moves
        // by the negative of the data change. Slot D went from (FREE, FREE) to
        // (SCSI, unit 0x00).
        let new_bytes = i32::from(CMOS_DRIVE_TYPE_SCSI);
        let old_bytes = 2 * i32::from(CMOS_DRIVE_TYPE_FREE);
        let data_delta = new_bytes - old_bytes;
        let checksum_delta = i32::from(memory.cmos[checksum]) - i32::from(checksum_before);
        assert_eq!((checksum_delta + data_delta).rem_euclid(256), 0);

        // A second disk (SCSI id 1) lands in the next free slot E, its unit byte
        // packing the SCSI id in the high nibble.
        memory.register_scsi_hdd(1, 0);
        assert_eq!(memory.cmos[table + 8], CMOS_DRIVE_TYPE_SCSI);
        assert_eq!(memory.cmos[table + 9], 0x10);
    }

    #[test]
    fn dic_rom_bank_selects_window_contents() {
        let mut memory = test_memory();
        memory.write_sysrom_dic_select(0x01); // map DIC/CMOS
        memory.write_dic_rom_bank(0);
        let bank0 = memory.read_byte(0x000D_0000);
        memory.write_dic_rom_bank(2);
        let bank2 = memory.read_byte(0x000D_0000);
        // Each bank is filled with its own index in the test fixture.
        assert_eq!(bank0, 0);
        assert_eq!(bank2, 2);
    }

    #[test]
    fn interleaved_transform_matches_reference() {
        // Spot-check the documented transform.
        assert_eq!(interleaved_offset(0), 0);
        assert_eq!(interleaved_offset(4), 4 << 16);
        assert_eq!(interleaved_offset(8), (8 >> 1));
        assert_eq!(interleaved_offset(3), 3);
    }

    #[test]
    fn memsize_reports_total_ram() {
        let memory = test_memory();
        assert_eq!(memory.total_ram_megabytes(), 8);
    }

    #[test]
    fn all_ones_plane_mask_passes_writes_through() {
        let mut memory = test_memory();
        memory.write_byte(0x8000_0000, 0xAB);
        assert_eq!(memory.read_byte(0x8000_0000), 0xAB);
    }

    #[test]
    fn plane_mask_read_modify_writes_selected_lanes() {
        let mut memory = test_memory();
        // Half 0: lane 0 keeps low nibble, lane 1 is fully masked out.
        memory.write_vram_mask_latch(0);
        memory.write_vram_mask_low(0x0F);
        memory.write_vram_mask_high(0x00);
        memory.write_byte(0x8000_0000, 0xFF); // lane 0
        memory.write_byte(0x8000_0001, 0xFF); // lane 1
        assert_eq!(memory.read_byte(0x8000_0000), 0x0F);
        assert_eq!(memory.read_byte(0x8000_0001), 0x00);
    }

    #[test]
    fn fmr_planar_write_round_trips_through_native_vram() {
        let mut memory = test_memory();
        // FMR mapping is on by default; set the plane mask to all four planes.
        memory.write_byte(0x000C_FF81, 0x0F);
        memory.write_byte(0x000C_0000, 0xAA);
        assert_eq!(memory.read_byte(0x000C_0000), 0xAA);
    }

    #[test]
    fn fmr_plane_mask_io_port_alias_selects_single_planes() {
        let mut memory = test_memory();
        // The FMR-compatible boot code drives the plane mask through I/O port
        // 0xFF81, not the memory-mapped CFF81 register. Selecting one plane at a
        // time must let successive writes accumulate into a multi-plane color
        // index rather than collapsing every pixel to one index.
        memory.write_fmr_io_register(0xFF81, 0x01);
        assert_eq!(memory.read_fmr_io_register(0xFF81), 0x01);

        // Set the leftmost pixel in plane 0: native lane 0 low nibble => index 1.
        memory.write_byte(0x000C_0000, 0x80);
        assert_eq!(memory.read_byte(0x8000_0000) & 0x0F, 0x01);

        // Selecting plane 1 and writing the same pixel accumulates to index 3.
        memory.write_fmr_io_register(0xFF81, 0x02);
        memory.write_byte(0x000C_0000, 0x80);
        assert_eq!(memory.read_byte(0x8000_0000) & 0x0F, 0x03);
    }

    #[test]
    fn ank_font_overlay_reads_font_rom() {
        let mut memory = test_memory();
        // Enable the ANK-font overlay (CFF99 bit 0).
        memory.write_byte(0x000C_FF99, 0x01);
        // The test font ROM is filled with 0x22.
        assert_eq!(memory.read_byte(0x000C_A000), 0x22);
        assert_eq!(memory.read_byte(0x000C_B000), 0x22);
    }

    #[test]
    fn kanji_jis_high_reads_back_fixed_value() {
        let mut memory = test_memory();
        assert_eq!(memory.read_byte(0x000C_FF94), 0x80);
    }

    #[test]
    fn high_res_vram_windows_map_into_vram() {
        let mut memory = test_memory();
        // Page 0 window: linear into the low half of VRAM.
        memory.write_byte(VRAM_HIGH_RES_PAGE0_BASE + 0x100, 0xA1);
        assert_eq!(memory.vram()[0x100], 0xA1);
        assert_eq!(memory.read_byte(VRAM_HIGH_RES_PAGE0_BASE + 0x100), 0xA1);
        // Page 1 window: linear into the high half (displaced by 0x80000).
        memory.write_byte(VRAM_HIGH_RES_PAGE1_BASE + 0x200, 0xB2);
        assert_eq!(memory.vram()[0x8_0200], 0xB2);
        assert_eq!(memory.read_byte(VRAM_HIGH_RES_PAGE1_BASE + 0x200), 0xB2);
        // Single-page window: the high-res interleave swizzle.
        memory.write_byte(VRAM_HIGH_RES_SINGLE_BASE + 8, 0xC3);
        let target = high_res_interleaved_offset(8) as usize;
        assert_eq!(memory.vram()[target], 0xC3);
        assert_eq!(memory.read_byte(VRAM_HIGH_RES_SINGLE_BASE + 8), 0xC3);
    }

    #[test]
    fn high_res_vram_windows_absent_on_cx() {
        let mut memory = TownsMemory::new(TownsModel::FmTownsIICx, test_roms());
        // The CX has no high-res windows: the write does not reach VRAM and the
        // read returns open bus.
        memory.write_byte(VRAM_HIGH_RES_PAGE0_BASE + 0x100, 0xA1);
        assert_eq!(memory.vram()[0x100], 0x00);
        assert_eq!(memory.read_byte(VRAM_HIGH_RES_PAGE0_BASE + 0x100), OPEN_BUS);
    }

    impl TownsMemory {
        /// Test helper: writes directly to the DRAM behind a windowed address,
        /// bypassing the banking overlay, so a test can prove the overlay hides
        /// or exposes the underlying RAM.
        fn write_byte_raw_ram(&mut self, address: u32, value: u8) {
            self.ram[address as usize] = value;
        }
    }
}
