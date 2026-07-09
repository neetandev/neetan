//! Main CPU memory map for the FM-7 and FM-77AV.
//!
//! The base FM-7 uses a flat 64 KiB map with a single F-BASIC ROM/RAM bank
//! toggle at `0xFD0F`. The FM-77AV adds a 256 KiB physical address space reached
//! through the Memory Management Register (MMR) paging unit, a relocatable
//! window, and a boot region that is RAM seeded from the initiator ROM rather
//! than a fixed boot ROM.

use crate::{
    config::{BootMode, Fm7Model},
    rom::LoadedRoms,
};

/// Size of the lower main RAM bank.
const LOWER_RAM_SIZE: usize = 0x8000;
/// Size of the upper RAM bank behind the F-BASIC ROM window.
const UPPER_RAM_SIZE: usize = 0x7C00;
/// Size of the F-BASIC ROM window.
const BASIC_ROM_WINDOW_SIZE: usize = 0x7C00;
/// Size of the BIOS work RAM window.
const BIOS_WORK_SIZE: usize = 0x80;
/// Size of one FM-7 boot ROM image.
const BOOT_ROM_SIZE: usize = 0x200;
/// Size of the writable interrupt-vector RAM window.
const VECTOR_RAM_SIZE: usize = 0x1E;

/// First address of lower main RAM.
const LOWER_RAM_START: u16 = 0x0000;
/// Last address of lower main RAM.
const LOWER_RAM_END: u16 = 0x7FFF;
/// First address of the upper RAM / F-BASIC ROM bank.
const UPPER_BANK_START: u16 = 0x8000;
/// Last address of the upper RAM / F-BASIC ROM bank.
const UPPER_BANK_END: u16 = 0xFBFF;
/// First address of BIOS work RAM.
const BIOS_WORK_START: u16 = 0xFC00;
/// Last address of BIOS work RAM.
const BIOS_WORK_END: u16 = 0xFC7F;
/// First address of the main/sub shared RAM window.
const SHARED_WINDOW_START: u16 = 0xFC80;
/// Last address of the main/sub shared RAM window.
const SHARED_WINDOW_END: u16 = 0xFCFF;
/// First address of the main CPU memory-mapped I/O page.
const MMIO_START: u16 = 0xFD00;
/// Last address of the main CPU memory-mapped I/O page.
const MMIO_END: u16 = 0xFDFF;
/// First address of the boot ROM / boot RAM window.
const BOOT_ROM_START: u16 = 0xFE00;
/// Last address of the boot ROM / boot RAM window.
const BOOT_ROM_END: u16 = 0xFFDF;
/// First address of writable vector RAM.
const VECTOR_RAM_START: u16 = 0xFFE0;
/// Last address of writable vector RAM.
const VECTOR_RAM_END: u16 = 0xFFFD;
/// First address of the forced reset vector.
const RESET_VECTOR_START: u16 = 0xFFFE;

/// Reset vector bytes returned at `0xFFFE-0xFFFF`, pointing at the boot entry.
const RESET_VECTOR: [u8; 2] = [0xFE, 0x00];

/// Size of the FM-77AV MMR bank-0 DRAM (physical `0x00000-0x0FFFF`), 64 KiB. This
/// is also the target of the relocatable window.
const AV_RAM_PAGE0_SIZE: usize = 0x1_0000;
/// Size of the FM-77AV initiator ROM image.
const INITIATOR_ROM_SIZE: usize = 0x2000;
/// Size of the FM-77AV boot RAM (`0xFE00-0xFFFF`).
const BOOT_RAM_SIZE: usize = 0x200;
/// Number of MMR page registers: four segments of sixteen 4 KiB blocks each.
const MMR_PAGE_REGISTER_COUNT: usize = 64;

/// First address of the FM-77AV initiator ROM overlay (CPU `0x6000-0x7FFF`).
const INITIATOR_OVERLAY_START: u16 = 0x6000;
/// Last address of the FM-77AV initiator ROM overlay.
const INITIATOR_OVERLAY_END: u16 = 0x7FFF;
/// First address of the relocatable window (`0x7C00-0x7FFF`).
const WINDOW_START: u16 = 0x7C00;
/// Last address of the relocatable window.
const WINDOW_END: u16 = 0x7FFF;
/// First address that always bypasses MMR translation (I/O, boot, vectors).
const MMR_BYPASS_START: u16 = 0xFC00;

/// Size in bytes of one MMR / physical page (4 KiB).
const PAGE_SIZE: usize = 0x1000;
/// Shift converting an address to its 4 KiB block index.
const PAGE_SHIFT: u16 = 12;
/// Mask selecting the offset within a 4 KiB page.
const PAGE_OFFSET_MASK: u16 = 0x0FFF;
/// Shift converting a segment number into its page-register base index.
const SEGMENT_SHIFT: usize = 4;
/// Mask reducing the segment select to the four base-AV segments.
const MMR_SEGMENT_MASK: u8 = 0x03;
/// Mask selecting the physical bank number from a page register.
const MMR_BANK_MASK: u8 = 0x3F;
/// Highest physical bank served by MMR bank-0 DRAM (`0x00000-0x0FFFF`).
const BANK_RAM0_LAST: u8 = 0x0F;
/// Lowest physical bank in the direct-VRAM window range (`0x10000-0x1FFFF`).
const BANK_VRAM_FIRST: u8 = 0x10;
/// Highest physical bank in the direct-VRAM window range (`0x10000-0x1FFFF`).
const BANK_VRAM_LAST: u8 = 0x1F;
/// Lowest physical bank in the optional page-2 RAM range (`0x20000-0x2FFFF`).
const BANK_PAGE2_FIRST: u8 = 0x20;
/// Highest physical bank in the optional page-2 RAM range (`0x20000-0x2FFFF`),
/// unfitted on the base machine and reading open bus.
const BANK_PAGE2_LAST: u8 = 0x2F;

/// Byte multiplier applied to the window offset (offset steps of 256 bytes).
const WINDOW_OFFSET_STRIDE: usize = 256;

/// `0xFD93` control bit enabling MMR translation.
const CONTROL_MMR_ENABLE: u8 = 0x80;
/// `0xFD93` control bit enabling the relocatable window.
const CONTROL_WINDOW_ENABLE: u8 = 0x40;
/// `0xFD93` control bit enabling boot-RAM writes.
const CONTROL_BOOT_RAM_WRITE: u8 = 0x01;

/// Initiator-ROM offset seeding the boot RAM in BASIC boot mode.
const INITIATOR_BOOT_BASIC_OFFSET: usize = 0x1800;
/// Initiator-ROM offset seeding the boot RAM in DOS boot mode.
const INITIATOR_BOOT_DOS_OFFSET: usize = 0x1A00;
/// Number of bytes copied from the initiator ROM into the boot RAM.
const BOOT_RAM_SEED_LENGTH: usize = 0x1E0;
/// Boot-RAM offset of the forced reset-vector high byte (`0xFFFE`).
const BOOT_RAM_RESET_VECTOR_OFFSET: usize = 0x1FE;
/// Initiator-ROM offset of the reset vector fetched while the initiator is
/// enabled (`0xFFFE-0xFFFF`).
const INITIATOR_RESET_VECTOR_OFFSET: usize = 0x1FFE;

/// Value returned by reads of unfitted or write-only physical banks.
const OPEN_BUS: u8 = 0xFF;

/// Main CPU memory visible in the base 64 KiB address space, plus the FM-77AV
/// MMR paging state and initiator boot RAM.
pub(crate) struct Fm7Memory {
    model: Fm7Model,
    lower_ram: [u8; LOWER_RAM_SIZE],
    upper_ram: [u8; UPPER_RAM_SIZE],
    basic_rom: [u8; BASIC_ROM_WINDOW_SIZE],
    bios_work: [u8; BIOS_WORK_SIZE],
    boot_rom: [u8; BOOT_ROM_SIZE],
    vector_ram: [u8; VECTOR_RAM_SIZE],
    reset_vector: [u8; 2],
    basic_rom_mapped: bool,
    /// FM-77AV MMR bank-0 DRAM and window target; empty on the FM-7.
    av_ram_page0: Vec<u8>,
    /// FM-77AV initiator ROM image; empty on the FM-7.
    initiator_rom: Vec<u8>,
    /// FM-77AV boot RAM at `0xFE00-0xFFFF`, seeded from the initiator ROM.
    boot_ram: [u8; BOOT_RAM_SIZE],
    /// MMR page registers: four segments x sixteen 4 KiB blocks.
    mmr_page_registers: [u8; MMR_PAGE_REGISTER_COUNT],
    /// Segment selected by `0xFD90` for the `0xFD80-0xFD8F` register window.
    mmr_current_segment: u8,
    /// Relocatable window base offset latched by `0xFD92`.
    mmr_window_offset: u8,
    /// Whether MMR translation is enabled (`0xFD93` bit 7).
    mmr_enabled: bool,
    /// Whether the relocatable window is enabled (`0xFD93` bit 6).
    window_enabled: bool,
    /// Whether boot-RAM writes are permitted (`0xFD93` bit 0).
    boot_ram_write: bool,
    /// Whether the initiator ROM overlay is active (`0xFD10`).
    initiator_enabled: bool,
}

impl Fm7Memory {
    /// Creates zero-filled memory for `model` with reset-time banking for
    /// `boot_mode`.
    pub(crate) fn empty(model: Fm7Model, boot_mode: BootMode) -> Self {
        let has_av = model.has_boot_ram();
        Self {
            model,
            lower_ram: [0; LOWER_RAM_SIZE],
            upper_ram: [0; UPPER_RAM_SIZE],
            basic_rom: [0; BASIC_ROM_WINDOW_SIZE],
            bios_work: [0; BIOS_WORK_SIZE],
            boot_rom: [0; BOOT_ROM_SIZE],
            vector_ram: [0; VECTOR_RAM_SIZE],
            reset_vector: RESET_VECTOR,
            basic_rom_mapped: basic_rom_mapped_at_reset(boot_mode),
            av_ram_page0: if has_av {
                vec![0; AV_RAM_PAGE0_SIZE]
            } else {
                Vec::new()
            },
            initiator_rom: if has_av {
                vec![0; INITIATOR_ROM_SIZE]
            } else {
                Vec::new()
            },
            boot_ram: [0; BOOT_RAM_SIZE],
            mmr_page_registers: [0; MMR_PAGE_REGISTER_COUNT],
            mmr_current_segment: 0,
            mmr_window_offset: 0,
            mmr_enabled: false,
            window_enabled: false,
            boot_ram_write: true,
            initiator_enabled: has_av,
        }
    }

    /// Creates memory initialized from the loaded ROM set.
    pub(crate) fn new(roms: &LoadedRoms, boot_mode: BootMode) -> Self {
        let mut memory = Self::empty(roms.model, boot_mode);
        copy_prefix(&roms.fbasic, &mut memory.basic_rom);

        match roms.model {
            Fm7Model::Fm7 => {
                let boot_image = match boot_mode {
                    BootMode::Basic => roms.boot_bas.as_deref(),
                    BootMode::Dos => roms.boot_dos.as_deref(),
                };
                if let Some(boot_image) = boot_image {
                    copy_prefix(boot_image, &mut memory.boot_rom);
                }
            }
            Fm7Model::Fm77Av => {
                if let Some(initiator) = roms.initiate.as_deref() {
                    copy_prefix(initiator, &mut memory.initiator_rom);
                    memory.seed_boot_ram(boot_mode);
                }
            }
        }

        memory
    }

    /// Seeds the FM-77AV boot RAM from the initiator ROM for `boot_mode` and
    /// forces the reset vector to the boot entry.
    fn seed_boot_ram(&mut self, boot_mode: BootMode) {
        let source_offset = match boot_mode {
            BootMode::Basic => INITIATOR_BOOT_BASIC_OFFSET,
            BootMode::Dos => INITIATOR_BOOT_DOS_OFFSET,
        };
        let end = source_offset + BOOT_RAM_SEED_LENGTH;
        if end <= self.initiator_rom.len() {
            self.boot_ram[..BOOT_RAM_SEED_LENGTH]
                .copy_from_slice(&self.initiator_rom[source_offset..end]);
        }
        self.boot_ram[BOOT_RAM_RESET_VECTOR_OFFSET] = RESET_VECTOR[0];
        self.boot_ram[BOOT_RAM_RESET_VECTOR_OFFSET + 1] = RESET_VECTOR[1];
    }

    /// Reads a byte from the non-MMIO memory map.
    pub(crate) fn read(&self, address: u16) -> u8 {
        match self.model {
            Fm7Model::Fm7 => self.fm7_read(address),
            Fm7Model::Fm77Av => self.av_read(address),
        }
    }

    /// Writes a byte to the non-MMIO memory map.
    pub(crate) fn write(&mut self, address: u16, value: u8) {
        match self.model {
            Fm7Model::Fm7 => self.fm7_write(address, value),
            Fm7Model::Fm77Av => self.av_write(address, value),
        }
    }

    /// Reads a byte from the flat FM-7 memory map.
    fn fm7_read(&self, address: u16) -> u8 {
        match address {
            LOWER_RAM_START..=LOWER_RAM_END => {
                self.lower_ram[usize::from(address - LOWER_RAM_START)]
            }
            UPPER_BANK_START..=UPPER_BANK_END => {
                let index = usize::from(address - UPPER_BANK_START);
                if self.basic_rom_mapped {
                    self.basic_rom[index]
                } else {
                    self.upper_ram[index]
                }
            }
            BIOS_WORK_START..=BIOS_WORK_END => {
                self.bios_work[usize::from(address - BIOS_WORK_START)]
            }
            // The shared-RAM window and the MMIO page are intercepted by the bus
            // before reaching here; this arm only satisfies match exhaustiveness.
            SHARED_WINDOW_START..=SHARED_WINDOW_END => OPEN_BUS,
            MMIO_START..=MMIO_END => OPEN_BUS,
            BOOT_ROM_START..=BOOT_ROM_END => self.boot_rom[usize::from(address - BOOT_ROM_START)],
            VECTOR_RAM_START..=VECTOR_RAM_END => {
                self.vector_ram[usize::from(address - VECTOR_RAM_START)]
            }
            RESET_VECTOR_START..=u16::MAX => {
                self.reset_vector[usize::from(address - RESET_VECTOR_START)]
            }
        }
    }

    /// Writes a byte to the flat FM-7 memory map.
    fn fm7_write(&mut self, address: u16, value: u8) {
        match address {
            LOWER_RAM_START..=LOWER_RAM_END => {
                self.lower_ram[usize::from(address - LOWER_RAM_START)] = value;
            }
            UPPER_BANK_START..=UPPER_BANK_END => {
                if !self.basic_rom_mapped {
                    self.upper_ram[usize::from(address - UPPER_BANK_START)] = value;
                }
            }
            BIOS_WORK_START..=BIOS_WORK_END => {
                self.bios_work[usize::from(address - BIOS_WORK_START)] = value;
            }
            SHARED_WINDOW_START..=SHARED_WINDOW_END
            | MMIO_START..=MMIO_END
            | BOOT_ROM_START..=BOOT_ROM_END
            | RESET_VECTOR_START..=u16::MAX => {}
            VECTOR_RAM_START..=VECTOR_RAM_END => {
                self.vector_ram[usize::from(address - VECTOR_RAM_START)] = value;
            }
        }
    }

    /// Reads a byte through the FM-77AV translation chain: initiator overlay,
    /// then the relocatable window, then MMR paging, then the FM-7-compatible
    /// image.
    fn av_read(&self, address: u16) -> u8 {
        if self.initiator_enabled {
            match address {
                INITIATOR_OVERLAY_START..=INITIATOR_OVERLAY_END => {
                    return self.initiator_rom[usize::from(address - INITIATOR_OVERLAY_START)];
                }
                RESET_VECTOR_START..=u16::MAX => {
                    let offset =
                        INITIATOR_RESET_VECTOR_OFFSET + usize::from(address - RESET_VECTOR_START);
                    return self.initiator_rom[offset];
                }
                _ => {}
            }
        }

        if self.window_enabled && (WINDOW_START..=WINDOW_END).contains(&address) {
            return self.av_ram_page0[self.window_physical_address(address)];
        }

        if self.mmr_enabled && address < MMR_BYPASS_START {
            let (bank, offset) = self.mmr_translate(address);
            return self.read_physical_bank(bank, offset);
        }

        self.av_normal_read(address)
    }

    /// Writes a byte through the FM-77AV translation chain.
    fn av_write(&mut self, address: u16, value: u8) {
        if self.initiator_enabled
            && matches!(
                address,
                INITIATOR_OVERLAY_START..=INITIATOR_OVERLAY_END | RESET_VECTOR_START..=u16::MAX
            )
        {
            return;
        }

        if self.window_enabled && (WINDOW_START..=WINDOW_END).contains(&address) {
            let index = self.window_physical_address(address);
            self.av_ram_page0[index] = value;
            return;
        }

        if self.mmr_enabled && address < MMR_BYPASS_START {
            let (bank, offset) = self.mmr_translate(address);
            self.write_physical_bank(bank, offset, value);
            return;
        }

        self.av_normal_write(address, value);
    }

    /// Maps a windowed address into MMR bank-0 DRAM at the latched offset.
    fn window_physical_address(&self, address: u16) -> usize {
        (usize::from(self.mmr_window_offset) * WINDOW_OFFSET_STRIDE + usize::from(address))
            & (AV_RAM_PAGE0_SIZE - 1)
    }

    /// Translates a CPU address into a physical `(bank, offset)` through the MMR
    /// page register for the active segment.
    fn mmr_translate(&self, address: u16) -> (u8, u16) {
        let block = usize::from(address >> PAGE_SHIFT);
        let index = (usize::from(self.mmr_current_segment) << SEGMENT_SHIFT) | block;
        let bank = self.mmr_page_registers[index] & MMR_BANK_MASK;
        (bank, address & PAGE_OFFSET_MASK)
    }

    /// Resolves a main-CPU address that the MMR maps into the direct-VRAM window
    /// (`0x10000-0x1FFFF`). Returns the stripped 16-bit sub address space location
    /// (VRAM and sub I/O) the window exposes, or `None` when the address is not
    /// windowed. The initiator overlay and the relocatable window take precedence,
    /// mirroring [`Fm7Memory::av_read`]. The caller gates the access on the sub
    /// CPU being halted and performs the VRAM/sub decode.
    pub(crate) fn direct_vram_target(&self, address: u16) -> Option<u16> {
        if self.initiator_enabled
            && matches!(
                address,
                INITIATOR_OVERLAY_START..=INITIATOR_OVERLAY_END | RESET_VECTOR_START..=u16::MAX
            )
        {
            return None;
        }
        if self.window_enabled && (WINDOW_START..=WINDOW_END).contains(&address) {
            return None;
        }
        if self.mmr_enabled && address < MMR_BYPASS_START {
            let (bank, offset) = self.mmr_translate(address);
            if (BANK_VRAM_FIRST..=BANK_VRAM_LAST).contains(&bank) {
                return Some((u16::from(bank & 0x0F) << PAGE_SHIFT) | offset);
            }
        }
        None
    }

    /// Reads a byte from a physical 4 KiB bank. Bank-0 DRAM is fitted; the
    /// direct-VRAM window is intercepted by the bus before this is reached (open
    /// bus here as a fallback); the unfitted page-2 expansion reads open bus; the
    /// high banks fall through to the FM-7-compatible image.
    fn read_physical_bank(&self, bank: u8, offset: u16) -> u8 {
        match bank {
            0..=BANK_RAM0_LAST => {
                self.av_ram_page0[usize::from(bank) * PAGE_SIZE + usize::from(offset)]
            }
            BANK_VRAM_FIRST..=BANK_VRAM_LAST => OPEN_BUS,
            BANK_PAGE2_FIRST..=BANK_PAGE2_LAST => OPEN_BUS,
            _ => self.av_normal_read(compatible_image_address(bank, offset)),
        }
    }

    /// Writes a byte to a physical 4 KiB bank, mirroring [`read_physical_bank`].
    fn write_physical_bank(&mut self, bank: u8, offset: u16, value: u8) {
        match bank {
            0..=BANK_RAM0_LAST => {
                self.av_ram_page0[usize::from(bank) * PAGE_SIZE + usize::from(offset)] = value;
            }
            BANK_VRAM_FIRST..=BANK_VRAM_LAST => {}
            BANK_PAGE2_FIRST..=BANK_PAGE2_LAST => {}
            _ => self.av_normal_write(compatible_image_address(bank, offset), value),
        }
    }

    /// Reads a byte from the FM-77AV FM-7-compatible image, where the boot
    /// region is RAM rather than a fixed boot ROM.
    fn av_normal_read(&self, address: u16) -> u8 {
        match address {
            LOWER_RAM_START..=LOWER_RAM_END => {
                self.lower_ram[usize::from(address - LOWER_RAM_START)]
            }
            UPPER_BANK_START..=UPPER_BANK_END => {
                let index = usize::from(address - UPPER_BANK_START);
                if self.basic_rom_mapped {
                    self.basic_rom[index]
                } else {
                    self.upper_ram[index]
                }
            }
            BIOS_WORK_START..=BIOS_WORK_END => {
                self.bios_work[usize::from(address - BIOS_WORK_START)]
            }
            SHARED_WINDOW_START..=SHARED_WINDOW_END => OPEN_BUS,
            MMIO_START..=MMIO_END => OPEN_BUS,
            BOOT_ROM_START..=BOOT_ROM_END => self.boot_ram[usize::from(address - BOOT_ROM_START)],
            VECTOR_RAM_START..=VECTOR_RAM_END => {
                self.vector_ram[usize::from(address - VECTOR_RAM_START)]
            }
            RESET_VECTOR_START..=u16::MAX => self.boot_ram[usize::from(address - BOOT_ROM_START)],
        }
    }

    /// Writes a byte to the FM-77AV FM-7-compatible image.
    fn av_normal_write(&mut self, address: u16, value: u8) {
        match address {
            LOWER_RAM_START..=LOWER_RAM_END => {
                self.lower_ram[usize::from(address - LOWER_RAM_START)] = value;
            }
            UPPER_BANK_START..=UPPER_BANK_END => {
                if !self.basic_rom_mapped {
                    self.upper_ram[usize::from(address - UPPER_BANK_START)] = value;
                }
            }
            BIOS_WORK_START..=BIOS_WORK_END => {
                self.bios_work[usize::from(address - BIOS_WORK_START)] = value;
            }
            SHARED_WINDOW_START..=SHARED_WINDOW_END | MMIO_START..=MMIO_END => {}
            BOOT_ROM_START..=BOOT_ROM_END | RESET_VECTOR_START..=u16::MAX => {
                if self.boot_ram_write {
                    self.boot_ram[usize::from(address - BOOT_ROM_START)] = value;
                }
            }
            VECTOR_RAM_START..=VECTOR_RAM_END => {
                self.vector_ram[usize::from(address - VECTOR_RAM_START)] = value;
            }
        }
    }

    /// Maps the F-BASIC ROM bank into `0x8000-0xFBFF`.
    pub(crate) fn map_rom(&mut self) {
        self.basic_rom_mapped = true;
    }

    /// Maps RAM into `0x8000-0xFBFF`.
    pub(crate) fn map_ram(&mut self) {
        self.basic_rom_mapped = false;
    }

    /// Whether the F-BASIC ROM bank is currently mapped.
    pub(crate) fn basic_rom_mapped(&self) -> bool {
        self.basic_rom_mapped
    }

    /// Sets the FM-77AV initiator ROM overlay state (`0xFD10`).
    pub(crate) fn set_initiator_enabled(&mut self, enabled: bool) {
        self.initiator_enabled = enabled;
    }

    /// Whether the FM-77AV initiator ROM overlay is active.
    pub(crate) fn initiator_enabled(&self) -> bool {
        self.initiator_enabled
    }

    /// Writes an MMR page register of the active segment (`0xFD80-0xFD8F`).
    pub(crate) fn write_mmr_page_register(&mut self, block_index: u8, value: u8) {
        let index = (usize::from(self.mmr_current_segment) << SEGMENT_SHIFT)
            | usize::from(block_index & 0x0F);
        self.mmr_page_registers[index] = value;
    }

    /// Reads an MMR page register of the active segment (`0xFD80-0xFD8F`).
    pub(crate) fn read_mmr_page_register(&self, block_index: u8) -> u8 {
        let index = (usize::from(self.mmr_current_segment) << SEGMENT_SHIFT)
            | usize::from(block_index & 0x0F);
        self.mmr_page_registers[index]
    }

    /// Selects the MMR segment addressed by `0xFD80-0xFD8F` (`0xFD90`).
    pub(crate) fn set_mmr_segment(&mut self, value: u8) {
        self.mmr_current_segment = value & MMR_SEGMENT_MASK;
    }

    /// Sets the relocatable window base offset (`0xFD92`).
    pub(crate) fn set_mmr_window_offset(&mut self, value: u8) {
        self.mmr_window_offset = value;
    }

    /// Writes the MMR control register (`0xFD93`).
    pub(crate) fn write_mmr_control(&mut self, value: u8) {
        self.mmr_enabled = value & CONTROL_MMR_ENABLE != 0;
        self.window_enabled = value & CONTROL_WINDOW_ENABLE != 0;
        self.boot_ram_write = value & CONTROL_BOOT_RAM_WRITE != 0;
    }

    /// Reads back the MMR control register (`0xFD93`).
    pub(crate) fn read_mmr_control(&self) -> u8 {
        let mut value = 0;
        if self.mmr_enabled {
            value |= CONTROL_MMR_ENABLE;
        }
        if self.window_enabled {
            value |= CONTROL_WINDOW_ENABLE;
        }
        if self.boot_ram_write {
            value |= CONTROL_BOOT_RAM_WRITE;
        }
        value
    }

    /// Whether MMR or the relocatable window is active, which slows the main
    /// clock to the MMR rate.
    pub(crate) fn mmr_translation_active(&self) -> bool {
        self.mmr_enabled || self.window_enabled
    }
}

/// Reconstructs the FM-7-compatible CPU address for a high physical bank.
fn compatible_image_address(bank: u8, offset: u16) -> u16 {
    (u16::from(bank & 0x0F) << PAGE_SHIFT) | offset
}

/// Returns the reset-time F-BASIC bank state for `boot_mode`.
fn basic_rom_mapped_at_reset(boot_mode: BootMode) -> bool {
    match boot_mode {
        BootMode::Basic => true,
        BootMode::Dos => false,
    }
}

/// Copies the common prefix of `source` into `dest`.
fn copy_prefix(source: &[u8], dest: &mut [u8]) {
    let len = source.len().min(dest.len());
    dest[..len].copy_from_slice(&source[..len]);
}
