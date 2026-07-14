//! PC-88VA memory map and banking.
//!
//! The V30 addresses a 1 MiB physical space. The high nibble of the 20-bit
//! address selects a 64 KiB page: pages 0-7 are main RAM, pages 8-9 are
//! unpopulated bus-expansion memory, pages A-D are the banked system-memory
//! window (selected by `sysm_bank`), page E is the ROM0 window (selected by
//! `rom0_bank`) and page F is the ROM1 window (selected by `rom1_bank`).
//!
//! The banking and memory-control I/O ports live here because they manipulate
//! this state directly.

use crate::{config::Pc88VaModel, rom::LoadedRoms};

const BACKUP_SIZE: usize = 0x4000;
const ROM0_SIZE: usize = 0xA_0000;
const TEXT_VRAM_SIZE: usize = 0x4_0000;
const GRAPHICS_VRAM_SIZE: usize = 0x4_0000;

/// `sysm_bank` value that windows graphics VRAM into `0xA0000-0xDFFFF`.
const SYSM_BANK_GRAPHICS: u8 = 0x4;

const SYSM_BASE: u32 = 0xA_0000;
const ROM0_BASE: u32 = 0xE_0000;
const ROM1_BASE: u32 = 0xF_0000;
const BACKUP_BASE: u32 = 0xB_0000;

const ADDRESS_MASK: u32 = 0xF_FFFF;
const OPEN_BUS: u8 = 0xFF;

/// Backup-memory offsets exposed through the identity ports (0x030/0x031).
const BACKUP_IDENTITY_0: usize = 0x1FC2;
const BACKUP_IDENTITY_1: usize = 0x1FC6;

/// Backup-memory write-protect window (CPU addresses).
const BACKUP_WP_START: u32 = 0xB_1FC0;
const BACKUP_WP_END: u32 = 0xB_2000;

pub(crate) struct Pc88VaMemory {
    ram: Box<[u8]>,
    rom0: Box<[u8]>,
    rom1: Box<[u8]>,
    font: Box<[u8]>,
    dictionary: Box<[u8]>,
    backup: Box<[u8]>,
    text: Box<[u8]>,
    graphics: Box<[u8]>,
    sysm_bank: u8,
    rom0_bank: u8,
    rom1_bank: u8,
    dma_sysm_bank: u8,
    backupmem_wp: bool,
    gmsp: u8,
    upd9002_tcks: u8,
}

impl Pc88VaMemory {
    pub(crate) fn new(model: Pc88VaModel, roms: LoadedRoms) -> Self {
        let mut rom0 = Vec::with_capacity(ROM0_SIZE);
        rom0.extend_from_slice(&roms.rom00);
        rom0.extend_from_slice(&roms.rom08);

        let mut memory = Self {
            ram: vec![0u8; model.main_ram_size()].into_boxed_slice(),
            rom0: rom0.into_boxed_slice(),
            rom1: roms.rom1.into_boxed_slice(),
            font: roms.font.into_boxed_slice(),
            dictionary: roms.dictionary.into_boxed_slice(),
            backup: vec![0u8; BACKUP_SIZE].into_boxed_slice(),
            text: vec![0u8; TEXT_VRAM_SIZE].into_boxed_slice(),
            graphics: vec![0u8; GRAPHICS_VRAM_SIZE].into_boxed_slice(),
            sysm_bank: 0,
            rom0_bank: 0,
            rom1_bank: 0,
            dma_sysm_bank: 0,
            backupmem_wp: false,
            gmsp: 0,
            upd9002_tcks: 0,
        };
        memory.reset();
        memory
    }

    fn reset(&mut self) {
        self.write_rom_bank(0);
        self.write_sysm_bank(0x41);
        self.backupmem_wp = true;
    }

    pub(crate) fn read_byte(&self, address: u32) -> u8 {
        let address = address & ADDRESS_MASK;
        match (address >> 16) & 0xF {
            0x0..=0x7 => self.ram[address as usize],
            0x8 | 0x9 => OPEN_BUS,
            0xA..=0xD => self.sysm_read(address),
            0xE => self.rom0_read(address),
            _ => self.rom1_read(address),
        }
    }

    pub(crate) fn write_byte(&mut self, address: u32, value: u8) {
        let address = address & ADDRESS_MASK;
        match (address >> 16) & 0xF {
            0x0..=0x7 => self.ram[address as usize] = value,
            0xA..=0xD => self.sysm_write(address, value),
            _ => {}
        }
    }

    fn sysm_read(&self, address: u32) -> u8 {
        match self.sysm_bank & 0x0F {
            0x1 => self.text[(address - SYSM_BASE) as usize],
            0x8 => self.font_byte(address - SYSM_BASE),
            0x9 => {
                if address < BACKUP_BASE {
                    // 0xA0000-0xAFFFF: ANK font ROM (high part of the font image).
                    self.font_byte(address - (SYSM_BASE - 0x4_0000))
                } else if address < BACKUP_BASE + BACKUP_SIZE as u32 {
                    // 0xB0000-0xB3FFF: battery-backed RAM / PCG, the same store the
                    // write path targets.
                    self.backup[(address - BACKUP_BASE) as usize]
                } else {
                    OPEN_BUS
                }
            }
            0xC => self.dictionary_byte(address - SYSM_BASE),
            0xD => self.dictionary_byte(address - (SYSM_BASE - 0x4_0000)),
            _ => OPEN_BUS,
        }
    }

    fn sysm_write(&mut self, address: u32, value: u8) {
        match self.sysm_bank & 0x0F {
            0x1 => self.text[(address - SYSM_BASE) as usize] = value,
            0x9 => self.backup_write(address, value),
            _ => {}
        }
    }

    /// The text/attribute VRAM image (256 KiB), for the renderer.
    pub(crate) fn text_vram(&self) -> &[u8] {
        &self.text
    }

    /// Mutable text/attribute VRAM, for TSP sprite-table writes.
    pub(crate) fn text_vram_mut(&mut self) -> &mut [u8] {
        &mut self.text
    }

    /// The graphics VRAM image (256 KiB), for the renderer.
    pub(crate) fn graphics_vram(&self) -> &[u8] {
        &self.graphics
    }

    /// Mutable graphics VRAM, for writes through the access controller.
    pub(crate) fn graphics_vram_mut(&mut self) -> &mut [u8] {
        &mut self.graphics
    }

    /// If `address` falls in the system-memory window while the graphics bank
    /// is selected, returns its offset into graphics VRAM. The controller owns
    /// the access, so the bus routes these reads/writes there instead of here.
    pub(crate) fn graphics_window_offset(&self, address: u32) -> Option<u32> {
        if self.sysm_bank & 0x0F != SYSM_BANK_GRAPHICS {
            return None;
        }
        let address = address & ADDRESS_MASK;
        if (0xA..=0xD).contains(&((address >> 16) & 0xF)) {
            Some(address - SYSM_BASE)
        } else {
            None
        }
    }

    /// The VA font ROM image, for the renderer and the CGROM window.
    pub(crate) fn font_rom(&self) -> &[u8] {
        &self.font
    }

    /// The battery-backed memory, read by the CGROM window's user-glyph region.
    pub(crate) fn backup_ram(&self) -> &[u8] {
        &self.backup
    }

    /// The battery-backed memory, written by the CGROM window's user-glyph region.
    pub(crate) fn backup_ram_mut(&mut self) -> &mut [u8] {
        &mut self.backup
    }

    fn backup_write(&mut self, address: u32, value: u8) {
        if (BACKUP_WP_START..BACKUP_WP_END).contains(&address) && self.backupmem_wp {
            return;
        }
        if let Some(offset) = address.checked_sub(BACKUP_BASE)
            && let Some(slot) = self.backup.get_mut(offset as usize)
        {
            *slot = value;
        }
    }

    fn rom0_read(&self, address: u32) -> u8 {
        let bank = u32::from(self.rom0_bank & 0x1F);
        if bank >= 0x0A {
            return OPEN_BUS;
        }
        let index = ((bank << 16) + (address - ROM0_BASE)) as usize;
        self.rom0.get(index).copied().unwrap_or(OPEN_BUS)
    }

    fn rom1_read(&self, address: u32) -> u8 {
        let bank = self.rom1_bank & 0x03;
        if bank & 0x02 != 0 {
            return rom1_invalid(address);
        }
        let index = ((u32::from(bank) << 16) + (address - ROM1_BASE)) as usize;
        self.rom1.get(index).copied().unwrap_or(OPEN_BUS)
    }

    fn font_byte(&self, index: u32) -> u8 {
        self.font.get(index as usize).copied().unwrap_or(OPEN_BUS)
    }

    fn dictionary_byte(&self, index: u32) -> u8 {
        self.dictionary
            .get(index as usize)
            .copied()
            .unwrap_or(OPEN_BUS)
    }

    /// The uPD9002 timer input-clock selector (port 0xFFF0). The low two bits
    /// divide the PIT input clock by `1 << (value & 3)`.
    pub(crate) fn upd9002_tcks(&self) -> u8 {
        self.upd9002_tcks
    }

    pub(crate) fn io_read_byte(&self, port: u16) -> Option<u8> {
        let value = match port {
            0x030 => self.backup[BACKUP_IDENTITY_0],
            0x031 => self.backup[BACKUP_IDENTITY_1],
            0x152 => self.read_rom_bank(),
            0x153 => (self.sysm_bank & 0x0F) | self.gmsp | 0x40,
            0x156 => 0xFF,
            0x180 => self.dma_sysm_bank,
            0xFFF0 => self.upd9002_tcks,
            _ => return None,
        };
        Some(value)
    }

    pub(crate) fn io_write_byte(&mut self, port: u16, value: u8) -> bool {
        match port {
            0x152 => self.write_rom_bank(value),
            0x153 => self.write_sysm_bank(value),
            0x180 => self.dma_sysm_bank = value & 0x8F,
            0x198 => self.backupmem_wp = true,
            0x19A => self.backupmem_wp = false,
            0xFFF0 => self.upd9002_tcks = value,
            _ => return false,
        }
        true
    }

    fn write_rom_bank(&mut self, value: u8) {
        self.rom0_bank = ((value & 0x40) >> 2) | (value & 0x0F);
        self.rom1_bank = (value & 0xB0) >> 4;
    }

    fn read_rom_bank(&self) -> u8 {
        (self.rom0_bank & 0x0F) | ((self.rom0_bank & 0x10) << 2) | ((self.rom1_bank & 0x0B) << 4)
    }

    fn write_sysm_bank(&mut self, value: u8) {
        self.sysm_bank = value & 0x0F;
        self.gmsp = value & 0x10;
    }

    /// The stored GMSP bit (port `0x153` bit 4), used to detect changes.
    pub(crate) fn gmsp_bit(&self) -> u8 {
        self.gmsp
    }

    /// Reads a little-endian word from the SGP address space: main RAM
    /// (`0x000000`), kanji/font ROM (`0x100000`), text VRAM (`0x180000`) and
    /// graphics VRAM (`0x200000`). Unmapped regions read as `0xFFFF`.
    pub(crate) fn sgp_read_word(&self, address: u32) -> u16 {
        let offset = (address & 0x3F_FFFF) as usize;
        let (slice, base) = match (address >> 16) & 0x3F {
            0x00..=0x07 => (&self.ram[..], 0x00_0000usize),
            0x10..=0x14 => (&self.font[..], 0x10_0000),
            0x18..=0x1B => (&self.text[..], 0x18_0000),
            0x20..=0x23 => (&self.graphics[..], 0x20_0000),
            _ => return 0xFFFF,
        };
        let index = offset - base;
        let low = slice.get(index).copied().unwrap_or(0xFF);
        let high = slice.get(index + 1).copied().unwrap_or(0xFF);
        u16::from(low) | (u16::from(high) << 8)
    }

    /// Writes a little-endian word into the SGP address space. The kanji/font
    /// ROM and unmapped regions drop the write.
    pub(crate) fn sgp_write_word(&mut self, address: u32, value: u16) {
        let offset = (address & 0x3F_FFFF) as usize;
        let (slice, base): (&mut [u8], usize) = match (address >> 16) & 0x3F {
            0x00..=0x07 => (&mut self.ram[..], 0x00_0000),
            0x18..=0x1B => (&mut self.text[..], 0x18_0000),
            0x20..=0x23 => (&mut self.graphics[..], 0x20_0000),
            _ => return,
        };
        let index = offset - base;
        if let Some(slot) = slice.get_mut(index) {
            *slot = value as u8;
        }
        if let Some(slot) = slice.get_mut(index + 1) {
            *slot = (value >> 8) as u8;
        }
    }
}

fn rom1_invalid(address: u32) -> u8 {
    let masked = (address as u16) & 0xFFFE;
    if address & 1 != 0 {
        masked as u8
    } else {
        (masked >> 8) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill(seed: u8, len: usize) -> Vec<u8> {
        (0..len)
            .map(|index| {
                seed.wrapping_add(index as u8)
                    .wrapping_add((index >> 8) as u8)
                    .wrapping_add((index >> 16) as u8)
            })
            .collect()
    }

    const ROM00_SEED: u8 = 0x10;
    const ROM08_SEED: u8 = 0x20;
    const ROM1_SEED: u8 = 0x30;
    const FONT_SEED: u8 = 0x40;
    const DICTIONARY_SEED: u8 = 0x50;

    fn synthetic_roms() -> LoadedRoms {
        LoadedRoms {
            rom00: fill(ROM00_SEED, 0x8_0000),
            rom08: fill(ROM08_SEED, 0x2_0000),
            rom1: fill(ROM1_SEED, 0x2_0000),
            font: fill(FONT_SEED, 0x5_0000),
            dictionary: fill(DICTIONARY_SEED, 0x8_0000),
            subsys: fill(0x60, 0x2000),
        }
    }

    fn rom0_expected(index: usize) -> u8 {
        if index < 0x8_0000 {
            fill(ROM00_SEED, 0x8_0000)[index]
        } else {
            fill(ROM08_SEED, 0x2_0000)[index - 0x8_0000]
        }
    }

    fn memory(model: Pc88VaModel) -> Pc88VaMemory {
        Pc88VaMemory::new(model, synthetic_roms())
    }

    #[test]
    fn reset_banking_defaults() {
        let memory = memory(Pc88VaModel::PC88VA2);
        assert_eq!(memory.rom0_bank, 0);
        assert_eq!(memory.rom1_bank, 0);
        assert_eq!(memory.sysm_bank, 1);
        assert_eq!(memory.gmsp, 0);
        assert!(memory.backupmem_wp);
        // sysm bank 1 is the text VRAM window, zero-initialized.
        assert_eq!(memory.read_byte(0xA_0000), 0);
    }

    #[test]
    fn text_vram_window_round_trips() {
        let mut memory = memory(Pc88VaModel::PC88VA2);
        // Reset already selects bank 1 (TVRAM).
        memory.write_byte(0xA_0000, 0x12);
        memory.write_byte(0xD_FFFF, 0x34);
        assert_eq!(memory.read_byte(0xA_0000), 0x12);
        assert_eq!(memory.read_byte(0xD_FFFF), 0x34);
        assert_eq!(memory.text_vram()[0], 0x12);
        assert_eq!(memory.text_vram()[0x3_FFFF], 0x34);
    }

    #[test]
    fn reset_vector_reads_rom1() {
        let memory = memory(Pc88VaModel::PC88VA2);
        let rom1 = fill(ROM1_SEED, 0x2_0000);
        assert_eq!(memory.read_byte(0xF_FFF0), rom1[0xFFF0]);
        assert_eq!(memory.read_byte(0xF_0000), rom1[0]);
    }

    #[test]
    fn rom0_banking() {
        let mut memory = memory(Pc88VaModel::PC88VA2);
        for bank in 0..0x0Au8 {
            memory.io_write_byte(0x152, bank);
            assert_eq!(memory.rom0_bank, bank);
            assert_eq!(
                memory.read_byte(ROM0_BASE),
                rom0_expected((u32::from(bank) << 16) as usize)
            );
            assert_eq!(memory.io_read_byte(0x152), Some(bank));
        }
        // banks 0x0A and above are unmapped.
        memory.io_write_byte(0x152, 0x0A);
        assert_eq!(memory.read_byte(ROM0_BASE), OPEN_BUS);
    }

    #[test]
    fn rom_bank_decode_va2() {
        let mut memory = memory(Pc88VaModel::PC88VA2);
        // Bit 6 forms the high bit of the ROM0 bank; ROM1 takes bits 7,5,4.
        memory.io_write_byte(0x152, 0x55);
        assert_eq!(memory.rom0_bank, ((0x55 & 0x40) >> 2) | (0x55 & 0x0F));
        assert_eq!(memory.rom1_bank, (0x55 & 0xB0) >> 4);
        let expected = (memory.rom0_bank & 0x0F)
            | ((memory.rom0_bank & 0x10) << 2)
            | ((memory.rom1_bank & 0x0B) << 4);
        assert_eq!(memory.io_read_byte(0x152), Some(expected));
    }

    #[test]
    fn rom1_banking_and_invalid_bank() {
        let mut memory = memory(Pc88VaModel::PC88VA2);
        let rom1 = fill(ROM1_SEED, 0x2_0000);
        // Bank 1 maps the second 64 KiB of ROM1 (rom1_bank in bits 7,5,4).
        memory.io_write_byte(0x152, 0x10);
        assert_eq!(memory.rom1_bank, 1);
        assert_eq!(memory.read_byte(ROM1_BASE), rom1[0x1_0000]);
        // Bank 2 sets bit 1: the invalid path returns the address-derived byte.
        memory.io_write_byte(0x152, 0x20);
        assert_eq!(memory.rom1_bank, 2);
        assert_eq!(memory.read_byte(0xF_1234), rom1_invalid(0xF_1234));
        assert_eq!(memory.read_byte(0xF_1235), rom1_invalid(0xF_1235));
    }

    #[test]
    fn sysm_dictionary_and_font_windows() {
        let mut memory = memory(Pc88VaModel::PC88VA2);
        let font = fill(FONT_SEED, 0x5_0000);
        let dictionary = fill(DICTIONARY_SEED, 0x8_0000);

        memory.io_write_byte(0x153, 0x08);
        assert_eq!(memory.read_byte(0xA_0000), font[0]);
        assert_eq!(memory.read_byte(0xA_1234), font[0x1234]);

        memory.io_write_byte(0x153, 0x0C);
        assert_eq!(memory.read_byte(0xA_0000), dictionary[0]);
        memory.io_write_byte(0x153, 0x0D);
        assert_eq!(memory.read_byte(0xA_0000), dictionary[0x4_0000]);

        // Bank 1 is the text VRAM window (zero-initialized here).
        memory.io_write_byte(0x153, 0x01);
        assert_eq!(memory.read_byte(0xA_0000), 0);
        // Still-deferred banks read open bus.
        memory.io_write_byte(0x153, 0x04);
        assert_eq!(memory.read_byte(0xA_0000), OPEN_BUS);
    }

    #[test]
    fn sysm_bank_read_encoding() {
        let mut memory = memory(Pc88VaModel::PC88VA2);
        memory.io_write_byte(0x153, 0x1A);
        assert_eq!(memory.sysm_bank, 0x0A);
        assert_eq!(memory.gmsp, 0x10);
        assert_eq!(memory.io_read_byte(0x153), Some(0x0A | 0x10 | 0x40));
    }

    #[test]
    fn backup_write_protect_gates_config_region() {
        let mut memory = memory(Pc88VaModel::PC88VA2);
        memory.io_write_byte(0x153, 0x09);

        // Write protect is on after reset: identity-region writes are dropped.
        memory.io_write_byte(0x198, 0);
        memory.write_byte(0xB_1FC2, 0xAA);
        assert_eq!(memory.io_read_byte(0x030), Some(0x00));

        // Disabling write protect lets the write through.
        memory.io_write_byte(0x19A, 0);
        memory.write_byte(0xB_1FC2, 0xAA);
        memory.write_byte(0xB_1FC6, 0xBB);
        assert_eq!(memory.io_read_byte(0x030), Some(0xAA));
        assert_eq!(memory.io_read_byte(0x031), Some(0xBB));

        // Re-enabling write protect blocks further identity-region writes.
        memory.io_write_byte(0x198, 0);
        memory.write_byte(0xB_1FC2, 0x55);
        assert_eq!(memory.io_read_byte(0x030), Some(0xAA));
    }

    #[test]
    fn backup_write_outside_protected_region_always_allowed() {
        let mut memory = memory(Pc88VaModel::PC88VA2);
        memory.io_write_byte(0x153, 0x09);
        memory.io_write_byte(0x198, 0);
        memory.write_byte(0xB_0000, 0x77);
        assert_eq!(memory.backup[0], 0x77);
    }

    #[test]
    fn sysm_bank9_reads_back_backup_ram() {
        let mut memory = memory(Pc88VaModel::PC88VA2);
        memory.io_write_byte(0x153, 0x09);
        memory.io_write_byte(0x19A, 0); // disable write protect

        // Writes to 0xB0000-0xB3FFF land in backup RAM and read back through the
        // same window, rather than aliasing the font ROM.
        memory.write_byte(0xB_0000, 0x12);
        memory.write_byte(0xB_2C00, 0x34);
        memory.write_byte(0xB_3FFF, 0x56);
        assert_eq!(memory.read_byte(0xB_0000), 0x12);
        assert_eq!(memory.read_byte(0xB_2C00), 0x34);
        assert_eq!(memory.read_byte(0xB_3FFF), 0x56);

        // The ANK font ROM still shows through below the backup window.
        assert_eq!(memory.read_byte(0xA_0000), memory.font[0x4_0000]);
        // Past the backup window the bank reads open bus.
        assert_eq!(memory.read_byte(0xB_4000), OPEN_BUS);
    }

    #[test]
    fn dma_and_upd9002_ports() {
        let mut memory = memory(Pc88VaModel::PC88VA2);
        memory.io_write_byte(0x180, 0xFF);
        assert_eq!(memory.io_read_byte(0x180), Some(0x8F));
        memory.io_write_byte(0xFFF0, 0x6A);
        assert_eq!(memory.io_read_byte(0xFFF0), Some(0x6A));
        assert_eq!(memory.io_read_byte(0x156), Some(0xFF));
    }
}
