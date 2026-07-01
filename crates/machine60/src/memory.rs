//! PC-6000 memory maps.
//!
//! Two generations live here. The base PC-6001 uses a small fixed map: a
//! BASIC/system ROM, a cartridge window, a bank-switched window exposing either
//! the character generator or the upper half of the cartridge, and work RAM,
//! with video RAM living inside work RAM. The PC-6001mkII (and, later, the
//! PC-6601) divide the 64 KiB Z80 space into eight 8 KiB pages, each with an
//! independent read source and an independent write target selected from a flat
//! physical backing store that holds the BASIC, voice, character, kanji,
//! cartridge ROMs and two 64 KiB RAM banks.

use crate::config::Pc6000Model;

/// Work RAM size (mapped at 0x8000-0xFFFF) on the base PC-6001.
const WORK_RAM_SIZE: usize = 0x8000;
/// Base address of work RAM in the Z80 address space.
const WORK_RAM_BASE: u16 = 0x8000;
/// Cartridge window base (0x4000-0x5FFF maps the low half of the cartridge).
const CART_WINDOW_BASE: u16 = 0x4000;
/// Bank-switched window base (0x6000-0x7FFF).
const BANK_WINDOW_BASE: u16 = 0x6000;
/// Size of each 8 KiB window.
const WINDOW_SIZE: usize = 0x2000;
/// Open-bus value for reads from an empty cartridge window.
const OPEN_BUS: u8 = 0xFF;

/// Selects what the 0x6000-0x7FFF window exposes on the base PC-6001.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BankWindow {
    /// The character generator ROM.
    CharacterGenerator,
    /// The upper 8 KiB of the cartridge.
    CartridgeUpper,
}

/// PC-6000 memory, dispatching to the generation that matches the model.
pub enum Pc6000Memory {
    /// Base PC-6001 fixed map.
    Base(BaseMemory),
    /// PC-6001mkII / PC-6601 paged map.
    Banked(BankedMemory),
    /// PC-6001mkIISR / PC-6601SR 16 x 8 KiB read/write banked map.
    Sr(SrMemory),
}

impl Pc6000Memory {
    /// Creates the memory map for `model` with cleared RAM and no ROMs loaded.
    pub fn new(model: Pc6000Model) -> Self {
        match model {
            Pc6000Model::Pc6001 => Pc6000Memory::Base(BaseMemory::new()),
            Pc6000Model::Pc6001Mk2 | Pc6000Model::Pc6601 => {
                Pc6000Memory::Banked(BankedMemory::new())
            }
            Pc6000Model::Pc6001Mk2Sr | Pc6000Model::Pc6601Sr => Pc6000Memory::Sr(SrMemory::new()),
        }
    }

    /// Loads the BASIC/system ROM.
    pub fn load_basic_rom(&mut self, rom: &[u8]) {
        match self {
            Pc6000Memory::Base(memory) => memory.load_basic_rom(rom),
            Pc6000Memory::Banked(memory) => memory.load_region(PHYS_BASIC, rom),
            Pc6000Memory::Sr(_) => {}
        }
    }

    /// Loads the base character generator ROM.
    pub fn load_cgrom(&mut self, cgrom: &[u8]) {
        match self {
            Pc6000Memory::Base(memory) => memory.load_cgrom(cgrom),
            Pc6000Memory::Banked(memory) => memory.load_region(PHYS_CGROM, cgrom),
            Pc6000Memory::Sr(memory) => {
                memory.load_region(SR_CGROM, cgrom);
                memory.load_region(SR_COMPAT_CGROM, cgrom);
            }
        }
    }

    /// Loads the extended character generator ROM (banked models only).
    pub fn load_ext_cgrom(&mut self, cgrom: &[u8]) {
        match self {
            Pc6000Memory::Banked(memory) => memory.load_region(PHYS_CGROM + WINDOW_SIZE, cgrom),
            Pc6000Memory::Sr(memory) => {
                memory.load_region(SR_COMPAT_CGROM + WINDOW_SIZE, cgrom);
            }
            Pc6000Memory::Base(_) => {}
        }
    }

    /// Loads the voice synthesizer data ROM (banked models only).
    pub fn load_voice_rom(&mut self, voice: &[u8]) {
        match self {
            Pc6000Memory::Banked(memory) => memory.load_region(PHYS_VOICE, voice),
            Pc6000Memory::Sr(memory) => memory.load_region(SR_COMPAT_VOICE, voice),
            Pc6000Memory::Base(_) => {}
        }
    }

    /// Loads the kanji ROM (banked models only).
    pub fn load_kanji_rom(&mut self, kanji: &[u8]) {
        match self {
            Pc6000Memory::Banked(memory) => memory.load_region(PHYS_KANJI, kanji),
            Pc6000Memory::Sr(memory) => memory.load_region(SR_COMPAT_KANJI, kanji),
            Pc6000Memory::Base(_) => {}
        }
    }

    /// Loads the two halves of the SR system ROM (SR models only).
    pub fn load_sr_system_rom(&mut self, half1: &[u8], half2: &[u8]) {
        if let Pc6000Memory::Sr(memory) = self {
            memory.load_region(SR_SYSROM1, half1);
            memory.load_region(SR_SYSROM2, half2);
        }
    }

    /// Loads the base character generator used by the SR mkII-compatible modes.
    pub fn load_sr_compat_cgrom(&mut self, cgrom: &[u8]) {
        if let Pc6000Memory::Sr(memory) = self {
            memory.load_region(SR_COMPAT_CGROM, cgrom);
        }
    }

    /// Loads a cartridge image.
    pub fn load_cartridge(&mut self, image: &[u8]) {
        match self {
            Pc6000Memory::Base(memory) => memory.load_cartridge(image),
            Pc6000Memory::Banked(memory) => memory.load_cartridge(image),
            Pc6000Memory::Sr(memory) => memory.load_cartridge(image),
        }
    }

    /// Whether a cartridge is present.
    pub fn has_cartridge(&self) -> bool {
        match self {
            Pc6000Memory::Base(memory) => memory.has_cartridge(),
            Pc6000Memory::Banked(memory) => memory.has_cartridge(),
            Pc6000Memory::Sr(memory) => memory.has_cartridge(),
        }
    }

    /// Selects the base-model bank window.
    pub fn set_bank_window(&mut self, window: BankWindow) {
        if let Pc6000Memory::Base(memory) = self {
            memory.set_bank_window(window);
        }
    }

    /// Selects the base-model video RAM start address.
    pub fn set_video_ram_base(&mut self, base: u16) {
        if let Pc6000Memory::Base(memory) = self {
            memory.set_video_ram_base(base);
        }
    }

    /// The character generator ROM data (base CG plus, on banked models, the
    /// extended CG that follows it).
    pub fn cgrom(&self) -> &[u8] {
        match self {
            Pc6000Memory::Base(memory) => memory.cgrom(),
            Pc6000Memory::Banked(memory) => memory.gfx_cgrom(),
            Pc6000Memory::Sr(memory) => memory.cgrom(),
        }
    }

    /// The video RAM window the renderer reads, starting at the active base.
    pub fn video_ram(&self) -> &[u8] {
        match self {
            Pc6000Memory::Base(memory) => memory.video_ram(),
            Pc6000Memory::Banked(memory) => memory.video_window(),
            Pc6000Memory::Sr(memory) => memory.text_window(),
        }
    }

    /// Reads a byte.
    pub fn read(&self, address: u16) -> u8 {
        match self {
            Pc6000Memory::Base(memory) => memory.read(address),
            Pc6000Memory::Banked(memory) => memory.read(address),
            Pc6000Memory::Sr(memory) => memory.read(address),
        }
    }

    /// Writes a byte. Writes into ROM regions are ignored.
    pub fn write(&mut self, address: u16, value: u8) {
        match self {
            Pc6000Memory::Base(memory) => memory.write(address, value),
            Pc6000Memory::Banked(memory) => memory.write(address, value),
            Pc6000Memory::Sr(memory) => memory.write(address, value),
        }
    }

    /// The banked map, when this is a mkII / PC-6601 model.
    pub fn banked_mut(&mut self) -> Option<&mut BankedMemory> {
        match self {
            Pc6000Memory::Banked(memory) => Some(memory),
            Pc6000Memory::Base(_) | Pc6000Memory::Sr(_) => None,
        }
    }

    /// The banked map, when this is a mkII / PC-6601 model.
    pub fn banked(&self) -> Option<&BankedMemory> {
        match self {
            Pc6000Memory::Banked(memory) => Some(memory),
            Pc6000Memory::Base(_) | Pc6000Memory::Sr(_) => None,
        }
    }

    /// The SR map, when this is an SR model.
    pub fn sr_mut(&mut self) -> Option<&mut SrMemory> {
        match self {
            Pc6000Memory::Sr(memory) => Some(memory),
            Pc6000Memory::Base(_) | Pc6000Memory::Banked(_) => None,
        }
    }

    /// The SR map, when this is an SR model.
    pub fn sr(&self) -> Option<&SrMemory> {
        match self {
            Pc6000Memory::Sr(memory) => Some(memory),
            Pc6000Memory::Base(_) | Pc6000Memory::Banked(_) => None,
        }
    }
}

/// Base PC-6001 fixed memory map.
pub struct BaseMemory {
    basic_rom: Vec<u8>,
    work_ram: Vec<u8>,
    cgrom: Vec<u8>,
    cartridge: Vec<u8>,
    bank_window: BankWindow,
    video_ram_base: u16,
}

impl BaseMemory {
    fn new() -> Self {
        Self {
            basic_rom: Vec::new(),
            work_ram: vec![0; WORK_RAM_SIZE],
            cgrom: Vec::new(),
            cartridge: Vec::new(),
            bank_window: BankWindow::CharacterGenerator,
            video_ram_base: 0xC000,
        }
    }

    fn load_basic_rom(&mut self, rom: &[u8]) {
        let length = rom.len().min(CART_WINDOW_BASE as usize);
        self.basic_rom = rom[..length].to_vec();
    }

    fn load_cgrom(&mut self, cgrom: &[u8]) {
        self.cgrom = cgrom.to_vec();
    }

    fn load_cartridge(&mut self, image: &[u8]) {
        self.cartridge = image.to_vec();
    }

    fn has_cartridge(&self) -> bool {
        !self.cartridge.is_empty()
    }

    fn set_bank_window(&mut self, window: BankWindow) {
        self.bank_window = window;
    }

    fn set_video_ram_base(&mut self, base: u16) {
        self.video_ram_base = base;
    }

    fn cgrom(&self) -> &[u8] {
        &self.cgrom
    }

    fn video_ram(&self) -> &[u8] {
        let offset = (self.video_ram_base - WORK_RAM_BASE) as usize;
        &self.work_ram[offset..]
    }

    fn read(&self, address: u16) -> u8 {
        match address {
            0x0000..=0x3FFF => *self.basic_rom.get(address as usize).unwrap_or(&OPEN_BUS),
            0x4000..=0x5FFF => self.read_window(&self.cartridge, 0, address - CART_WINDOW_BASE),
            0x6000..=0x7FFF => self.read_bank_window(address - BANK_WINDOW_BASE),
            _ => self.work_ram[(address - WORK_RAM_BASE) as usize],
        }
    }

    fn write(&mut self, address: u16, value: u8) {
        if address >= WORK_RAM_BASE {
            self.work_ram[(address - WORK_RAM_BASE) as usize] = value;
        }
    }

    fn read_bank_window(&self, offset: u16) -> u8 {
        match self.bank_window {
            BankWindow::CharacterGenerator => {
                let index = offset as usize % self.cgrom.len().max(1);
                *self.cgrom.get(index).unwrap_or(&OPEN_BUS)
            }
            BankWindow::CartridgeUpper => self.read_window(&self.cartridge, WINDOW_SIZE, offset),
        }
    }

    fn read_window(&self, source: &[u8], base: usize, offset: u16) -> u8 {
        *source.get(base + offset as usize).unwrap_or(&OPEN_BUS)
    }
}

/// One 8 KiB page in the Z80 address space.
const PAGE_SIZE: usize = 0x2000;
/// Number of pages covering the 64 KiB Z80 address space.
const PAGE_COUNT: usize = 8;
/// Size of the flat physical backing store.
const PHYSICAL_SIZE: usize = 0x50000;

/// Physical offsets of each region inside the backing store. The layout matches
/// the reference hardware so the bank tables can index ROM and RAM uniformly.
const PHYS_BASIC: usize = 0x10000;
const PHYS_VOICE: usize = 0x18000;
const PHYS_CGROM: usize = 0x1C000;
const PHYS_KANJI: usize = 0x20000;
const PHYS_WORK_RAM: usize = 0x28000;
const PHYS_EX_WORK_RAM: usize = 0x38000;
const PHYS_EXROM: usize = 0x48000;

/// Size of the character generator window the video ROM bank exposes: the base
/// CG followed by the extended CG.
const GFX_CGROM_SIZE: usize = 0x4000;
/// Maximum cartridge image copied into the exROM region.
const EXROM_SIZE: usize = 0x4000;

/// Per-page mask into the write-bank register that selects work RAM (bit set)
/// versus extended work RAM (bit clear). Page pairs share a bit.
const WRITE_SELECT_MASK: [u8; PAGE_COUNT] = [0x01, 0x01, 0x04, 0x04, 0x10, 0x10, 0x40, 0x40];

const fn basic_page(page: u32) -> u32 {
    PHYS_BASIC as u32 + PAGE_SIZE as u32 * page
}
const fn voice_page(page: u32) -> u32 {
    PHYS_VOICE as u32 + PAGE_SIZE as u32 * page
}
const fn cgrom_page(page: u32) -> u32 {
    PHYS_CGROM as u32 + PAGE_SIZE as u32 * page
}
const fn kanji_page(page: u32) -> u32 {
    PHYS_KANJI as u32 + PAGE_SIZE as u32 * page
}
const fn work_ram_page(page: u32) -> u32 {
    PHYS_WORK_RAM as u32 + PAGE_SIZE as u32 * page
}
const fn ex_work_ram_page(page: u32) -> u32 {
    PHYS_EX_WORK_RAM as u32 + PAGE_SIZE as u32 * page
}
const fn exrom_page(page: u32) -> u32 {
    PHYS_EXROM as u32 + PAGE_SIZE as u32 * page
}
const fn invalid_page() -> u32 {
    0x4C000
}

/// Read-bank decode table for the low four Z80 pages (ports 0xF0, 0xC2). Indexed
/// by `(bank nibble) + (opt bank * 0x10)`; each row gives the physical base of
/// four consecutive 8 KiB pages.
const READ_TABLE_LOW: [[u32; 4]; 0x40] = build_read_table_low();
/// Read-bank decode table for the high four Z80 pages (ports 0xF1, 0xC2).
const READ_TABLE_HIGH: [[u32; 4]; 0x40] = build_read_table_high();

const fn build_read_table_low() -> [[u32; 4]; 0x40] {
    let invalid = invalid_page();
    let mut table = [[invalid; 4]; 0x40];
    let mut opt = 0;
    while opt < 4 {
        let base = opt * 0x10;
        // 0x00 / 0x0f: invalid (already filled).
        // 0x01: BASIC 0..3.
        table[base + 0x01] = [basic_page(0), basic_page(1), basic_page(2), basic_page(3)];
        // 0x03 / 0x04: whole window from one exROM page.
        table[base + 0x03] = [exrom_page(1); 4];
        table[base + 0x04] = [exrom_page(0); 4];
        // 0x07 / 0x08: alternating exROM pages.
        table[base + 0x07] = [exrom_page(0), exrom_page(1), exrom_page(0), exrom_page(1)];
        table[base + 0x08] = [exrom_page(1), exrom_page(0), exrom_page(1), exrom_page(0)];
        // 0x09 / 0x0a: exROM mixed with BASIC.
        table[base + 0x09] = [exrom_page(1), basic_page(1), exrom_page(1), basic_page(3)];
        table[base + 0x0A] = [basic_page(0), exrom_page(1), basic_page(2), exrom_page(1)];
        // 0x0d / 0x0e: work RAM and extended work RAM.
        table[base + 0x0D] = [
            work_ram_page(0),
            work_ram_page(1),
            work_ram_page(2),
            work_ram_page(3),
        ];
        table[base + 0x0E] = [
            ex_work_ram_page(0),
            ex_work_ram_page(1),
            ex_work_ram_page(2),
            ex_work_ram_page(3),
        ];
        opt += 1;
    }

    // opt bank selects the character source: TV/voice for 0 and 2, kanji bank 0
    // for 1, kanji bank 1 for 3.
    table[0x02] = [cgrom_page(0), cgrom_page(1), voice_page(0), voice_page(1)];
    table[0x05] = [cgrom_page(1), basic_page(1), voice_page(0), basic_page(3)];
    table[0x06] = [basic_page(0), cgrom_page(2), basic_page(2), voice_page(1)];
    table[0x0B] = [exrom_page(0), cgrom_page(2), exrom_page(0), voice_page(1)];
    table[0x0C] = [cgrom_page(1), exrom_page(0), voice_page(0), exrom_page(0)];

    table[0x12] = [kanji_page(0), kanji_page(1), kanji_page(0), kanji_page(1)];
    table[0x15] = [kanji_page(0), basic_page(1), kanji_page(0), basic_page(3)];
    table[0x16] = [basic_page(0), kanji_page(1), basic_page(2), kanji_page(1)];
    table[0x1B] = [exrom_page(0), kanji_page(1), exrom_page(0), kanji_page(1)];
    table[0x1C] = [kanji_page(0), exrom_page(0), kanji_page(0), exrom_page(0)];

    table[0x22] = [cgrom_page(0), cgrom_page(1), voice_page(0), voice_page(1)];
    table[0x25] = [cgrom_page(1), basic_page(1), voice_page(0), basic_page(3)];
    table[0x26] = [basic_page(0), cgrom_page(2), basic_page(2), voice_page(1)];
    table[0x2B] = [exrom_page(0), cgrom_page(2), exrom_page(0), voice_page(1)];
    table[0x2C] = [cgrom_page(1), exrom_page(0), voice_page(0), exrom_page(0)];

    table[0x32] = [kanji_page(2), kanji_page(3), kanji_page(2), kanji_page(3)];
    table[0x35] = [kanji_page(2), basic_page(1), kanji_page(2), basic_page(3)];
    table[0x36] = [basic_page(0), kanji_page(3), basic_page(2), kanji_page(3)];
    table[0x3B] = [exrom_page(0), kanji_page(3), exrom_page(0), kanji_page(3)];
    table[0x3C] = [kanji_page(2), exrom_page(0), kanji_page(2), exrom_page(0)];

    table
}

const fn build_read_table_high() -> [[u32; 4]; 0x40] {
    let invalid = invalid_page();
    let mut table = [[invalid; 4]; 0x40];
    let mut opt = 0;
    while opt < 4 {
        let base = opt * 0x10;
        table[base + 0x01] = [basic_page(0), basic_page(1), basic_page(2), basic_page(3)];
        table[base + 0x03] = [exrom_page(1); 4];
        table[base + 0x04] = [exrom_page(0); 4];
        table[base + 0x07] = [exrom_page(0), exrom_page(1), exrom_page(0), exrom_page(1)];
        table[base + 0x08] = [exrom_page(1), exrom_page(0), exrom_page(1), exrom_page(0)];
        table[base + 0x09] = [exrom_page(1), basic_page(1), exrom_page(1), basic_page(3)];
        table[base + 0x0A] = [basic_page(0), exrom_page(1), basic_page(2), exrom_page(1)];
        // The high window maps work RAM pages 4..7.
        table[base + 0x0D] = [
            work_ram_page(4),
            work_ram_page(5),
            work_ram_page(6),
            work_ram_page(7),
        ];
        table[base + 0x0E] = [
            ex_work_ram_page(4),
            ex_work_ram_page(5),
            ex_work_ram_page(6),
            ex_work_ram_page(7),
        ];
        opt += 1;
    }

    table[0x02] = [voice_page(0), voice_page(1), voice_page(0), voice_page(1)];
    table[0x05] = [voice_page(0), basic_page(1), voice_page(0), basic_page(3)];
    table[0x06] = [basic_page(0), voice_page(1), basic_page(2), voice_page(1)];
    table[0x0B] = [exrom_page(0), voice_page(1), exrom_page(0), voice_page(1)];
    table[0x0C] = [voice_page(0), exrom_page(0), voice_page(0), exrom_page(0)];

    table[0x12] = [kanji_page(0), kanji_page(1), kanji_page(0), kanji_page(1)];
    table[0x15] = [kanji_page(0), basic_page(1), kanji_page(0), basic_page(3)];
    table[0x16] = [basic_page(0), kanji_page(1), basic_page(2), kanji_page(1)];
    table[0x1B] = [exrom_page(0), kanji_page(1), exrom_page(0), kanji_page(1)];
    table[0x1C] = [kanji_page(0), exrom_page(0), kanji_page(0), exrom_page(0)];

    table[0x22] = [voice_page(0), voice_page(1), voice_page(0), voice_page(1)];
    table[0x25] = [voice_page(0), basic_page(1), voice_page(0), basic_page(3)];
    table[0x26] = [basic_page(0), voice_page(1), basic_page(2), voice_page(1)];
    table[0x2B] = [exrom_page(0), voice_page(1), exrom_page(0), voice_page(1)];
    table[0x2C] = [voice_page(0), exrom_page(0), voice_page(0), exrom_page(0)];

    table[0x32] = [kanji_page(2), kanji_page(3), kanji_page(2), kanji_page(3)];
    table[0x35] = [kanji_page(2), basic_page(1), kanji_page(2), basic_page(3)];
    table[0x36] = [basic_page(0), kanji_page(3), basic_page(2), kanji_page(3)];
    table[0x3B] = [exrom_page(0), kanji_page(3), exrom_page(0), kanji_page(3)];
    table[0x3C] = [kanji_page(2), exrom_page(0), kanji_page(2), exrom_page(0)];

    table
}

/// PC-6001mkII / PC-6601 paged memory.
pub struct BankedMemory {
    physical: Vec<u8>,
    has_cartridge: bool,
    read_base: [u32; PAGE_COUNT],
    bank_low: u8,
    bank_high: u8,
    bank_write: u8,
    opt_bank: u8,
    gfx_bank_on: bool,
    cgrom_bank_addr: usize,
    video_base: usize,
}

impl BankedMemory {
    fn new() -> Self {
        let mut physical = vec![OPEN_BUS; PHYSICAL_SIZE];
        // RAM regions power on cleared so the screen starts blank.
        for byte in &mut physical[PHYS_WORK_RAM..PHYS_EXROM] {
            *byte = 0;
        }

        let mut memory = Self {
            physical,
            has_cartridge: false,
            read_base: [0; PAGE_COUNT],
            // Power-on bank selection mapping BASIC low, exROM, work RAM high.
            bank_low: 0x71,
            bank_high: 0xDD,
            bank_write: 0x50,
            opt_bank: 0x02,
            gfx_bank_on: false,
            cgrom_bank_addr: 0,
            video_base: PHYS_WORK_RAM + 0xC000,
        };
        memory.resolve_read_banks();
        memory
    }

    fn load_region(&mut self, offset: usize, data: &[u8]) {
        let length = data.len().min(self.physical.len().saturating_sub(offset));
        self.physical[offset..offset + length].copy_from_slice(&data[..length]);
    }

    fn load_cartridge(&mut self, image: &[u8]) {
        let length = image.len().min(EXROM_SIZE);
        self.physical[PHYS_EXROM..PHYS_EXROM + length].copy_from_slice(&image[..length]);
        self.has_cartridge = length != 0;
    }

    fn has_cartridge(&self) -> bool {
        self.has_cartridge
    }

    /// Sets the low read-bank register (port 0xF0).
    pub fn set_read_bank_low(&mut self, value: u8) {
        self.bank_low = value;
        self.resolve_read_banks();
    }

    /// Sets the high read-bank register (port 0xF1).
    pub fn set_read_bank_high(&mut self, value: u8) {
        self.bank_high = value;
        self.resolve_read_banks();
    }

    /// Sets the write-bank register (port 0xF2).
    pub fn set_write_bank(&mut self, value: u8) {
        self.bank_write = value;
    }

    /// The low read-bank register (port 0xF0).
    pub fn read_bank_low(&self) -> u8 {
        self.bank_low
    }

    /// The high read-bank register (port 0xF1).
    pub fn read_bank_high(&self) -> u8 {
        self.bank_high
    }

    /// The write-bank register (port 0xF2).
    pub fn write_bank(&self) -> u8 {
        self.bank_write
    }

    /// Sets the optional ROM bank selector (port 0xC2).
    pub fn set_opt_bank(&mut self, value: u8) {
        self.opt_bank = value & 3;
        self.resolve_read_banks();
    }

    /// Turns the character-generator gfx bank on or off for the 0x6000 window
    /// (PPI control words 0x04 / 0x05).
    pub fn set_gfx_bank(&mut self, on: bool) {
        self.gfx_bank_on = on;
        self.resolve_read_banks();
    }

    /// Selects the half of the character generator the gfx bank exposes
    /// (port 0xC1).
    pub fn set_cgrom_bank_addr(&mut self, addr: usize) {
        self.cgrom_bank_addr = addr;
        self.resolve_read_banks();
    }

    /// Sets the video base as an offset inside the work RAM region (port 0xC1
    /// and the system latch).
    pub fn set_video_base(&mut self, work_ram_offset: usize) {
        self.video_base = PHYS_WORK_RAM + work_ram_offset;
    }

    fn resolve_read_banks(&mut self) {
        let opt = (self.opt_bank as usize) * 0x10;
        let low_lo = (self.bank_low & 0x0F) as usize + opt;
        let low_hi = ((self.bank_low >> 4) & 0x0F) as usize + opt;
        self.read_base[0] = READ_TABLE_LOW[low_lo][0];
        self.read_base[1] = READ_TABLE_LOW[low_lo][1];
        self.read_base[2] = READ_TABLE_LOW[low_hi][2];
        self.read_base[3] = if self.gfx_bank_on {
            (PHYS_CGROM + self.cgrom_bank_addr) as u32
        } else {
            READ_TABLE_LOW[low_hi][3]
        };

        let high_lo = (self.bank_high & 0x0F) as usize + opt;
        let high_hi = ((self.bank_high >> 4) & 0x0F) as usize + opt;
        self.read_base[4] = READ_TABLE_HIGH[high_lo][0];
        self.read_base[5] = READ_TABLE_HIGH[high_lo][1];
        self.read_base[6] = READ_TABLE_HIGH[high_hi][2];
        self.read_base[7] = READ_TABLE_HIGH[high_hi][3];
    }

    fn gfx_cgrom(&self) -> &[u8] {
        &self.physical[PHYS_CGROM..PHYS_CGROM + GFX_CGROM_SIZE]
    }

    fn video_window(&self) -> &[u8] {
        &self.physical[self.video_base..]
    }

    fn read(&self, address: u16) -> u8 {
        let page = (address >> 13) as usize;
        let offset = (address & 0x1FFF) as usize;
        self.physical[self.read_base[page] as usize + offset]
    }

    fn write(&mut self, address: u16, value: u8) {
        let page = (address >> 13) as usize;
        let offset = (address & 0x1FFF) as usize;
        let region = if self.bank_write & WRITE_SELECT_MASK[page] != 0 {
            PHYS_WORK_RAM
        } else {
            PHYS_EX_WORK_RAM
        };
        self.physical[region + PAGE_SIZE * page + offset] = value;
    }
}

/// SR physical address space: eight 8 KiB pages into 1 MiB.
const SR_PHYSICAL_SIZE: usize = 0x10_0000;
/// Number of 8 KiB pages covering the SR Z80 address space.
const SR_PAGE_COUNT: usize = 8;

/// Region bases inside the 1 MiB SR physical space.
const SR_WORK_RAM: usize = 0x00000;
const SR_WORK_RAM_SIZE: usize = 0x10000;
const SR_EX_RAM: usize = 0x20000;
const SR_EX_RAM_SIZE: usize = 0x10000;
const SR_COMPAT_CGROM: usize = 0x10000;
const SR_COMPAT_VOICE: usize = 0x18000;
const SR_COMPAT_KANJI: usize = 0x40000;
const SR_COMPAT_KANJI_SIZE: usize = 0x8000;
const SR_OPEN_BUS_PAGE: usize = 0x7E000;
const SR_CART_EXROM: usize = 0xB4000;
const SR_CART_EXROM_SIZE: usize = 0x4000;
const SR_CGROM: usize = 0xD0000;
const SR_CGROM_SIZE: usize = 0x4000;
const SR_SYSROM2: usize = 0xE0000;
const SR_SYSROM1: usize = 0xF0000;

/// Graphics VRAM overlay backing (bitmap mode), sized as on the reference.
const SR_GVRAM_SIZE: usize = 320 * 256 * 8;
/// Bitmap-mode scanline pitch used by the GVRAM overlay address arithmetic.
const SR_GVRAM_PITCH: usize = 320;
/// Text VRAM bank granularity selected by port 0xC9 (a 4 KiB step in work RAM).
const SR_TEXT_BANK_STEP: usize = 0x1000;
/// Power-on native text VRAM base in work RAM.
const SR_DEFAULT_TEXT_BASE: usize = 0xE000;

/// Power-on read-bank page indices for Z80 pages 0-7. These map the SR system
/// ROM low, the cartridge windows and the upper half of work RAM at reset.
const SR_DEFAULT_READ_PAGES: [u8; SR_PAGE_COUNT] = [0x7C, 0x7D, 0x60, 0x58, 0x04, 0x05, 0x06, 0x07];
/// Power-on write-bank page indices: all eight pages address work RAM, so RAM
/// writes land linearly while the read banks expose ROM.
const SR_DEFAULT_WRITE_PAGES: [u8; SR_PAGE_COUNT] =
    [0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07];

/// PC-6001mkIISR / PC-6601SR memory.
///
/// The Z80 space is eight 8 KiB pages, each with an independent read-page and
/// write-page register (ports 0x60-0x67 and 0x68-0x6F) selecting an 8 KiB page
/// inside a flat 1 MiB physical space. In bitmap mode the first 8 KiB of work
/// RAM is overlaid by the graphics VRAM, and because the read and write pages
/// are independent the system ROM can stay readable while the screen is written
/// underneath it.
pub struct SrMemory {
    physical: Vec<u8>,
    gvram: Vec<u8>,
    has_cartridge: bool,
    read_page: [u8; SR_PAGE_COUNT],
    write_page: [u8; SR_PAGE_COUNT],
    bitmap_mode: bool,
    bitmap_x_offset: usize,
    bitmap_y_offset: usize,
    text_base: usize,
    legacy_video_base: usize,
    compat_bank_low: u8,
    compat_bank_high: u8,
    compat_bank_write: u8,
    compat_opt_bank: u8,
    compat_gfx_bank_on: bool,
    compat_cgrom_bank_addr: usize,
}

impl SrMemory {
    fn new() -> Self {
        let mut physical = vec![OPEN_BUS; SR_PHYSICAL_SIZE];
        for byte in &mut physical[SR_WORK_RAM..SR_WORK_RAM + SR_WORK_RAM_SIZE] {
            *byte = 0;
        }
        for byte in &mut physical[SR_EX_RAM..SR_EX_RAM + SR_EX_RAM_SIZE] {
            *byte = 0;
        }
        Self {
            physical,
            gvram: vec![0; SR_GVRAM_SIZE],
            has_cartridge: false,
            read_page: SR_DEFAULT_READ_PAGES,
            write_page: SR_DEFAULT_WRITE_PAGES,
            bitmap_mode: false,
            bitmap_x_offset: 0,
            bitmap_y_offset: 0,
            text_base: SR_DEFAULT_TEXT_BASE,
            legacy_video_base: SR_DEFAULT_TEXT_BASE,
            compat_bank_low: 0x71,
            compat_bank_high: 0xDD,
            compat_bank_write: 0x50,
            compat_opt_bank: 0x02,
            compat_gfx_bank_on: false,
            compat_cgrom_bank_addr: 0,
        }
    }

    fn load_region(&mut self, offset: usize, data: &[u8]) {
        let length = data.len().min(self.physical.len().saturating_sub(offset));
        self.physical[offset..offset + length].copy_from_slice(&data[..length]);
    }

    fn load_cartridge(&mut self, image: &[u8]) {
        let length = image.len().min(SR_CART_EXROM_SIZE);
        self.physical[SR_CART_EXROM..SR_CART_EXROM + length].copy_from_slice(&image[..length]);
        self.has_cartridge = length != 0;
    }

    fn has_cartridge(&self) -> bool {
        self.has_cartridge
    }

    /// Sets one read-page register (port 0x60-0x67). Bit 0 is ignored on the
    /// hardware; the remaining bits select the 8 KiB physical page.
    pub fn set_read_page(&mut self, page: usize, value: u8) {
        self.read_page[page & 7] = value >> 1;
    }

    /// Sets one write-page register (port 0x68-0x6F).
    pub fn set_write_page(&mut self, page: usize, value: u8) {
        self.write_page[page & 7] = value >> 1;
    }

    /// Sets the compatibility low read-bank register (port 0xF0).
    pub fn set_compat_read_bank_low(&mut self, value: u8) {
        self.compat_bank_low = value;
        self.resolve_compat_read_banks();
    }

    /// Sets the compatibility high read-bank register (port 0xF1).
    pub fn set_compat_read_bank_high(&mut self, value: u8) {
        self.compat_bank_high = value;
        self.resolve_compat_read_banks();
    }

    /// Sets the compatibility write-bank register (port 0xF2).
    pub fn set_compat_write_bank(&mut self, value: u8) {
        self.compat_bank_write = value;
        self.resolve_compat_write_banks();
    }

    /// Applies the current compatibility write-bank register to the SR write pages.
    pub fn apply_compat_write_bank(&mut self) {
        self.resolve_compat_write_banks();
    }

    /// The read-page register, read back through port 0x60-0x67.
    pub fn read_page(&self, page: usize) -> u8 {
        self.read_page[page & 7] << 1
    }

    /// The write-page register, read back through port 0x68-0x6F.
    pub fn write_page(&self, page: usize) -> u8 {
        self.write_page[page & 7] << 1
    }

    /// The compatibility low read-bank register (port 0xF0).
    pub fn compat_read_bank_low(&self) -> u8 {
        self.compat_bank_low
    }

    /// The compatibility high read-bank register (port 0xF1).
    pub fn compat_read_bank_high(&self) -> u8 {
        self.compat_bank_high
    }

    /// The compatibility write-bank register (port 0xF2).
    pub fn compat_write_bank(&self) -> u8 {
        self.compat_bank_write
    }

    /// Sets the compatibility optional ROM bank selector (port 0xC2).
    pub fn set_compat_opt_bank(&mut self, value: u8) {
        self.compat_opt_bank = value & 3;
        self.resolve_compat_read_banks();
    }

    /// Turns the compatibility character-generator bank on or off.
    pub fn set_compat_gfx_bank(&mut self, on: bool) {
        self.compat_gfx_bank_on = on;
        self.resolve_compat_read_banks();
    }

    /// Selects the compatibility character-generator half exposed by the gfx bank.
    pub fn set_compat_cgrom_bank_addr(&mut self, addr: usize) {
        self.compat_cgrom_bank_addr = addr;
        self.resolve_compat_read_banks();
    }

    /// Sets the legacy renderer video base as an offset inside SR work RAM.
    pub fn set_legacy_video_base(&mut self, work_ram_offset: usize) {
        self.legacy_video_base = work_ram_offset.min(SR_WORK_RAM_SIZE - 1);
    }

    /// Selects bitmap mode (graphics VRAM overlay) or text mode (port 0xC8).
    pub fn set_bitmap_mode(&mut self, on: bool) {
        self.bitmap_mode = on;
    }

    /// Sets the bitmap X/Y offsets used by the GVRAM overlay (ports 0xCF/0xCE).
    pub fn set_bitmap_offsets(&mut self, x_offset: u8, y_offset: u8) {
        self.bitmap_x_offset = x_offset as usize;
        self.bitmap_y_offset = y_offset as usize;
    }

    /// Selects the text VRAM bank inside work RAM (port 0xC9).
    pub fn set_text_bank(&mut self, bank: u8) {
        self.text_base = (bank as usize & 0x0F) * SR_TEXT_BANK_STEP;
    }

    fn gvram_base(&self) -> usize {
        (self.bitmap_x_offset * 16 + self.bitmap_y_offset) * SR_GVRAM_PITCH
    }

    fn resolve_compat_read_banks(&mut self) {
        let opt = (self.compat_opt_bank as usize) * 0x10;
        let low_lo = (self.compat_bank_low & 0x0F) as usize + opt;
        let low_hi = ((self.compat_bank_low >> 4) & 0x0F) as usize + opt;
        self.read_page[0] =
            sr_compat_page_from_banked_base(READ_TABLE_LOW[low_lo][0], 0, self.has_cartridge);
        self.read_page[1] =
            sr_compat_page_from_banked_base(READ_TABLE_LOW[low_lo][1], 1, self.has_cartridge);
        self.read_page[2] =
            sr_compat_page_from_banked_base(READ_TABLE_LOW[low_hi][2], 2, self.has_cartridge);
        self.read_page[3] = if self.compat_gfx_bank_on {
            page_index(SR_COMPAT_CGROM + self.compat_cgrom_bank_addr)
        } else {
            sr_compat_page_from_banked_base(READ_TABLE_LOW[low_hi][3], 3, self.has_cartridge)
        };

        let high_lo = (self.compat_bank_high & 0x0F) as usize + opt;
        let high_hi = ((self.compat_bank_high >> 4) & 0x0F) as usize + opt;
        self.read_page[4] =
            sr_compat_page_from_banked_base(READ_TABLE_HIGH[high_lo][0], 4, self.has_cartridge);
        self.read_page[5] =
            sr_compat_page_from_banked_base(READ_TABLE_HIGH[high_lo][1], 5, self.has_cartridge);
        self.read_page[6] =
            sr_compat_page_from_banked_base(READ_TABLE_HIGH[high_hi][2], 6, self.has_cartridge);
        self.read_page[7] =
            sr_compat_page_from_banked_base(READ_TABLE_HIGH[high_hi][3], 7, self.has_cartridge);
    }

    fn resolve_compat_write_banks(&mut self) {
        for (page, write_select_mask) in WRITE_SELECT_MASK.iter().enumerate() {
            let region = if self.compat_bank_write & *write_select_mask != 0 {
                SR_WORK_RAM
            } else {
                SR_EX_RAM
            };
            self.write_page[page] = page_index(region + PAGE_SIZE * page);
        }
    }

    fn read(&self, address: u16) -> u8 {
        let page = (address >> 13) as usize & 7;
        let offset = (address & 0x1FFF) as usize;
        let physical = (self.read_page[page] as usize) * PAGE_SIZE + offset;
        if self.bitmap_mode && physical < 0x2000 {
            let index = (self.gvram_base() + physical) % self.gvram.len();
            return self.gvram[index];
        }
        self.physical[physical]
    }

    fn write(&mut self, address: u16, value: u8) {
        let page = (address >> 13) as usize & 7;
        let offset = (address & 0x1FFF) as usize;
        let physical = (self.write_page[page] as usize) * PAGE_SIZE + offset;
        if self.bitmap_mode && physical < 0x2000 {
            let index = (self.gvram_base() + physical) % self.gvram.len();
            self.gvram[index] = value;
            return;
        }
        if Self::is_writable(physical) {
            self.physical[physical] = value;
        }
    }

    fn is_writable(physical: usize) -> bool {
        (SR_WORK_RAM..SR_WORK_RAM + SR_WORK_RAM_SIZE).contains(&physical)
            || (SR_EX_RAM..SR_EX_RAM + SR_EX_RAM_SIZE).contains(&physical)
    }

    /// The SR character generator (16 KiB).
    pub fn cgrom(&self) -> &[u8] {
        &self.physical[SR_CGROM..SR_CGROM + SR_CGROM_SIZE]
    }

    /// The character generator used by the mkII-compatible renderer.
    pub fn compat_cgrom(&self) -> &[u8] {
        &self.physical[SR_COMPAT_CGROM..SR_COMPAT_CGROM + GFX_CGROM_SIZE]
    }

    /// The text VRAM window the renderer reads in text mode.
    pub fn text_window(&self) -> &[u8] {
        &self.physical[SR_WORK_RAM + self.text_base..SR_WORK_RAM + SR_WORK_RAM_SIZE]
    }

    /// The legacy video RAM window the mkII-compatible renderer reads.
    pub fn legacy_video_window(&self) -> &[u8] {
        &self.physical[SR_WORK_RAM + self.legacy_video_base..SR_WORK_RAM + SR_WORK_RAM_SIZE]
    }

    /// The graphics VRAM the renderer reads in bitmap mode.
    pub fn gvram(&self) -> &[u8] {
        &self.gvram
    }
}

fn page_index(physical: usize) -> u8 {
    (physical / PAGE_SIZE) as u8
}

fn sr_compat_page_from_banked_base(base: u32, z80_page: usize, has_cartridge: bool) -> u8 {
    let base = base as usize;
    let physical = if (PHYS_BASIC..PHYS_BASIC + 0x8000).contains(&base) {
        SR_SYSROM1 + (base - PHYS_BASIC)
    } else if (PHYS_VOICE..PHYS_VOICE + 0x4000).contains(&base) {
        SR_COMPAT_VOICE + (base - PHYS_VOICE)
    } else if (PHYS_CGROM..PHYS_CGROM + GFX_CGROM_SIZE).contains(&base) {
        SR_COMPAT_CGROM + (base - PHYS_CGROM)
    } else if (PHYS_KANJI..PHYS_KANJI + SR_COMPAT_KANJI_SIZE).contains(&base) {
        SR_COMPAT_KANJI + (base - PHYS_KANJI)
    } else if (PHYS_WORK_RAM..PHYS_WORK_RAM + WORK_RAM_SIZE * 2).contains(&base) {
        SR_WORK_RAM + (base - PHYS_WORK_RAM)
    } else if (PHYS_EX_WORK_RAM..PHYS_EX_WORK_RAM + WORK_RAM_SIZE * 2).contains(&base) {
        SR_EX_RAM + (base - PHYS_EX_WORK_RAM)
    } else if (PHYS_EXROM..PHYS_EXROM + EXROM_SIZE).contains(&base) {
        if has_cartridge {
            SR_CART_EXROM + (base - PHYS_EXROM)
        } else {
            SR_WORK_RAM + PAGE_SIZE * z80_page
        }
    } else {
        SR_OPEN_BUS_PAGE
    };
    page_index(physical)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_memory_with_roms() -> Pc6000Memory {
        let mut memory = Pc6000Memory::new(Pc6000Model::Pc6001);
        memory.load_basic_rom(&[0xAA; 0x4000]);
        memory.load_cgrom(&[0xCC; 0x1000]);
        memory
    }

    #[test]
    fn base_basic_rom_is_read_only() {
        let mut memory = base_memory_with_roms();
        assert_eq!(memory.read(0x0000), 0xAA);
        memory.write(0x0000, 0x55);
        assert_eq!(memory.read(0x0000), 0xAA);
    }

    #[test]
    fn base_work_ram_reads_back_writes() {
        let mut memory = base_memory_with_roms();
        memory.write(0x8000, 0x42);
        assert_eq!(memory.read(0x8000), 0x42);
        memory.write(0xFFFF, 0x99);
        assert_eq!(memory.read(0xFFFF), 0x99);
    }

    #[test]
    fn base_bank_window_switches_between_cg_and_cartridge() {
        let mut memory = base_memory_with_roms();
        let mut cartridge = vec![0x11; 0x2000];
        cartridge.extend_from_slice(&[0x22; 0x2000]);
        memory.load_cartridge(&cartridge);

        memory.set_bank_window(BankWindow::CharacterGenerator);
        assert_eq!(memory.read(0x6000), 0xCC);

        memory.set_bank_window(BankWindow::CartridgeUpper);
        assert_eq!(memory.read(0x6000), 0x22);
        assert_eq!(memory.read(0x4000), 0x11);
    }

    #[test]
    fn base_empty_cartridge_window_reads_open_bus() {
        let memory = base_memory_with_roms();
        assert_eq!(memory.read(0x4000), OPEN_BUS);
    }

    #[test]
    fn base_video_ram_base_selects_work_ram_window() {
        let mut memory = base_memory_with_roms();
        memory.write(0xC000, 0x7E);
        memory.set_video_ram_base(0xC000);
        assert_eq!(memory.video_ram()[0], 0x7E);
        memory.write(0x8000, 0x3C);
        memory.set_video_ram_base(0x8000);
        assert_eq!(memory.video_ram()[0], 0x3C);
    }

    #[test]
    fn banked_reset_maps_basic_low_and_work_ram_high() {
        let mut memory = Pc6000Memory::new(Pc6000Model::Pc6001Mk2);
        memory.load_basic_rom(&[0xB1; 0x8000]);

        // 0x0000-0x3FFF reads the first two BASIC pages.
        assert_eq!(memory.read(0x0000), 0xB1);
        assert_eq!(memory.read(0x3FFF), 0xB1);

        // 0xC000-0xFFFF is work RAM and reads back writes.
        memory.write(0xC000, 0x5A);
        assert_eq!(memory.read(0xC000), 0x5A);
    }

    #[test]
    fn banked_write_routes_to_work_or_extended_ram() {
        let banked = match Pc6000Memory::new(Pc6000Model::Pc6001Mk2) {
            Pc6000Memory::Banked(memory) => memory,
            Pc6000Memory::Base(_) | Pc6000Memory::Sr(_) => panic!("mkII must build a banked map"),
        };
        // Reset write bank 0x50: low pages go to extended RAM, high to work RAM.
        assert_eq!(banked.bank_write & WRITE_SELECT_MASK[0], 0);
        assert_ne!(banked.bank_write & WRITE_SELECT_MASK[6], 0);
    }

    #[test]
    fn banked_gfx_bank_exposes_character_generator() {
        let mut memory = Pc6000Memory::new(Pc6000Model::Pc6001Mk2);
        memory.load_cgrom(&[0xC6; 0x2000]);
        let banked = memory.banked_mut().expect("banked");
        banked.set_cgrom_bank_addr(0);
        banked.set_gfx_bank(true);
        // The 0x6000 window now reads the character generator.
        assert_eq!(memory.read(0x6000), 0xC6);
    }

    #[test]
    fn sr_reset_text_window_uses_high_work_ram() {
        let mut memory = Pc6000Memory::new(Pc6000Model::Pc6001Mk2Sr);

        memory.write(0xE000, 0x5A);

        assert_eq!(memory.video_ram()[0], 0x5A);
    }

    #[test]
    fn sr_text_bank_uses_4k_steps() {
        let mut memory = Pc6000Memory::new(Pc6000Model::Pc6001Mk2Sr);
        memory.write(0xE000, 0x5A);
        memory.write(0xF000, 0xA5);

        let sr = memory.sr_mut().expect("sr memory");
        sr.set_text_bank(0x0F);

        assert_eq!(memory.video_ram()[0], 0xA5);
    }
}
