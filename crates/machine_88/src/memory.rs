//! PC-8801 main-CPU memory map.
//!
//! The Z80's 64 KiB address space is decoded from a set of bank-control
//! registers (ports 0x31/0x32/0x34/0x35/0x5C-0x5F/0x70/0x71/0x78/0xE2/0xE3/
//! 0xF0/0xF1). Reads and writes are decoded separately: a region that reads ROM
//! still writes its underlying work RAM. The 0xC000-0xFFFF range can map a
//! single GVRAM plane, the dictionary ROM, or the graphic ALU; the 0xF000-0xFFFF
//! range can additionally map text VRAM. In N mode the PC-8001mkIISR-compatible
//! graphics window instead maps selected GVRAM at 0x8000-0xBFFF, leaving the
//! 0xC000-0xFFFF range as work RAM.

use crate::config::{BootMode, Pc8801Model};

const MAIN_RAM_SIZE: usize = 0x1_0000;
const EXT_RAM_BANK_SIZE: usize = 0x8000;
const EXT_RAM_BANKS: usize = 4;
const EXT_RAM_SIZE: usize = EXT_RAM_BANK_SIZE * EXT_RAM_BANKS;
const GVRAM_PLANE_SIZE: usize = 0x4000;
const GVRAM_PLANES: usize = 3;
const GVRAM_SIZE: usize = GVRAM_PLANE_SIZE * GVRAM_PLANES;
const TVRAM_SIZE: usize = 0x1000;

const N88_ROM_SIZE: usize = 0x8000;
const N88_EXT_BANK_SIZE: usize = 0x2000;
const N88_EXT_BANKS: usize = 4;
const N_BASIC_ROM_SIZE: usize = 0x8000;
const N80_ROM_SIZE: usize = 0x8000;
const N80SR_ROM_SIZE: usize = 0xA000;
const DICTIONARY_ROM_SIZE: usize = 0x8_0000;
const DICTIONARY_BANK_SIZE: usize = 0x4000;
const CDROM_BIOS_ROM_SIZE: usize = 0x1_0000;

const GVRAM_REGION_START: u16 = 0xC000;
const N80_GVRAM_REGION_START: u16 = 0x8000;
const HIGH_REGION_START: u16 = 0xF000;
const TEXT_WINDOW_MASK: usize = 0x03FF;

const PLANE_BLUE: usize = 0;
const PLANE_RED: usize = 1;
const PLANE_GREEN: usize = 2;

// Port 0x31 (gfx_ctrl).
const GFX_CTRL_MMODE: u8 = 0x02;
const GFX_CTRL_RMODE: u8 = 0x04;
// When the CD-ROM BIOS bank is mapped, gfx_ctrl bit 2 instead selects which
// 32 KiB half of the 64 KiB CD-ROM BIOS ROM is visible at 0x0000-0x7FFF.
const CDROM_BIOS_WINDOW_SELECT: u8 = 0x04;
const CDROM_BIOS_WINDOW_SIZE: usize = 0x8000;
// Port 0x32 (misc_ctrl).
const MISC_CTRL_EROMSL_MASK: u8 = 0x03;
const MISC_CTRL_TEXT_MODE: u8 = 0x10;
const MISC_CTRL_GVAM: u8 = 0x40;
// Port 0x35 (alu_ctrl2).
const ALU_CTRL2_PLANE_BLUE_NORMAL: u8 = 0x01;
const ALU_CTRL2_PLANE_RED_NORMAL: u8 = 0x02;
const ALU_CTRL2_PLANE_GREEN_NORMAL: u8 = 0x04;
const ALU_CTRL2_GDM_SHIFT: u8 = 4;
const ALU_CTRL2_GDM_MASK: u8 = 0x03;
const ALU_CTRL2_GAM: u8 = 0x80;
// Port 0x71 (ext_rom_bank).
const EXT_ROM_BANK_IEROM: u8 = 0x01;
// Port 0xE2 (extram_mode).
const EXTRAM_MODE_READ_ENABLE: u8 = 0x01;
const EXTRAM_MODE_WRITE_ENABLE: u8 = 0x10;
// Port 0xF0 (dic_bank).
const DICTIONARY_BANK_MASK: u8 = 0x1F;
// Port 0xF1 (dic_ctrl): bit 0 is active low (0 enables dictionary reads).
const DICTIONARY_CTRL_DISABLE: u8 = 0x01;

/// Per-plane logic operation codes for the ALU write path (one nibble pair per
/// plane in `alu_ctrl1`).
const ALU_OP_RESET: u8 = 0x00;
const ALU_OP_SET: u8 = 0x01;
const ALU_OP_XOR: u8 = 0x10;
const ALU_OP_NOOP: u8 = 0x11;

save_state::runtime_state_enum! {
/// GVRAM access selection driven by ports 0x5C-0x5F (only effective when the
/// GVAM ALU gate is clear).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum GvramSelect {
    /// 0x5F: 0xC000-0xFFFF maps main RAM (and text VRAM at 0xF000).
    #[default]
    MainRam = 0,
    /// 0x5C: blue plane.
    Blue = 1,
    /// 0x5D: red plane.
    Red = 2,
    /// 0x5E: green plane.
    Green = 3,
}}

/// Decoded target of a main-CPU memory access, used to select the memory-wait
/// timing applied for that access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pc8801MemoryTarget {
    /// Main work RAM.
    MainRam,
    /// N88 or N-BASIC ROM (including the extension ROM banks).
    BasicRom,
    /// Extension RAM.
    ExtensionRam,
    /// Relocatable text window (0x8000-0x83FF).
    TextWindow,
    /// Text VRAM mapped at 0xF000-0xFFFF.
    TextVram,
    /// A selected GVRAM plane.
    GvramPlane,
    /// Graphic ALU GVRAM access.
    GvramAlu,
    /// Dictionary ROM.
    DictionaryRom,
}

save_state::runtime_state! {
/// Mutable, save-state-relevant portion of the main-CPU memory.
#[derive(Clone)]
pub(crate) struct Pc8801MemoryState {
    /// Main RAM (64 KiB).
    pub(crate) ram: Box<[u8; MAIN_RAM_SIZE]>,
    /// Extension RAM (128 KiB, four 32 KiB banks).
    pub(crate) ext_ram: Box<[u8; EXT_RAM_SIZE]>,
    /// Graphics VRAM (3 planes x 16 KiB: blue, red, green).
    pub(crate) gvram: Box<[u8; GVRAM_SIZE]>,
    /// Text VRAM (4 KiB).
    pub(crate) tvram: Box<[u8; TVRAM_SIZE]>,
    /// Latched ALU plane registers (blue, red, green), captured on ALU reads.
    pub(crate) alu_registers: [u8; GVRAM_PLANES],
    /// Active GVRAM plane selection (ports 0x5C-0x5F).
    pub(crate) gvram_sel: GvramSelect,
    /// N88-BASIC boot mode (affects the 0xF000-0xFFFF decode).
    pub(crate) boot_mode: BootMode,
    /// Port 0x33 N80SR control latch. Bit 7 selects the N80SR ROM personality in
    /// N80-family modes.
    pub(crate) n80_ctrl: u8,
    /// Port 0x31 graphics control.
    pub(crate) gfx_ctrl: u8,
    /// Port 0x32 miscellaneous control.
    pub(crate) misc_ctrl: u8,
    /// Port 0x34 ALU control 1 (per-plane logic operations).
    pub(crate) alu_ctrl1: u8,
    /// Port 0x35 ALU control 2 (plane invert / GDM / GAM).
    pub(crate) alu_ctrl2: u8,
    /// Port 0x70 relocatable text-window base.
    pub(crate) window_bank: u8,
    /// Port 0x71 extension ROM bank control.
    pub(crate) ext_rom_bank: u8,
    /// Port 0xE2 extension RAM mode (read/write enable).
    pub(crate) extram_mode: u8,
    /// Port 0xE3 extension RAM bank select.
    pub(crate) extram_bank: u8,
    /// Port 0xF0 dictionary ROM bank select.
    pub(crate) dic_bank: u8,
    /// Port 0xF1 dictionary ROM control (bit 0 active low).
    pub(crate) dic_ctrl: u8,
    /// CD-ROM BIOS bank enable (port 0x99 bit 4). When set, the CD-ROM BIOS ROM
    /// is mapped at 0x0000-0x7FFF. Resets to enabled so the MC boots into the
    /// CD-System BIOS.
    pub(crate) cdrom_bank: bool,
}}

impl Pc8801MemoryState {
    fn new(model: Pc8801Model) -> Self {
        debug_assert_eq!(model.main_ram_size(), MAIN_RAM_SIZE);
        debug_assert_eq!(model.extension_ram_size(), EXT_RAM_SIZE);
        Self {
            ram: vec![0u8; MAIN_RAM_SIZE]
                .into_boxed_slice()
                .try_into()
                .unwrap(),
            ext_ram: vec![0u8; EXT_RAM_SIZE]
                .into_boxed_slice()
                .try_into()
                .unwrap(),
            gvram: vec![0u8; GVRAM_SIZE].into_boxed_slice().try_into().unwrap(),
            tvram: Box::new([0u8; TVRAM_SIZE]),
            alu_registers: [0u8; GVRAM_PLANES],
            gvram_sel: GvramSelect::MainRam,
            boot_mode: BootMode::V2,
            n80_ctrl: 0,
            gfx_ctrl: 0x31,
            misc_ctrl: 0x80,
            alu_ctrl1: 0,
            alu_ctrl2: 0,
            window_bank: 0x80,
            ext_rom_bank: 0xFF,
            extram_mode: 0,
            extram_bank: 0,
            dic_bank: 0,
            dic_ctrl: DICTIONARY_CTRL_DISABLE,
            cdrom_bank: true,
        }
    }
}

/// Main-CPU memory: mutable state plus immutable ROM arrays.
pub(crate) struct Pc8801Memory {
    pub(crate) state: Pc8801MemoryState,
    n88: Box<[u8; N88_ROM_SIZE]>,
    n88_ext: Box<[[u8; N88_EXT_BANK_SIZE]; N88_EXT_BANKS]>,
    n_basic: Box<[u8; N_BASIC_ROM_SIZE]>,
    n80_mkii: Option<Box<[u8; N80_ROM_SIZE]>>,
    n80_mkiisr: Option<Box<[u8; N80_ROM_SIZE]>>,
    n80sr: Option<Box<[u8; N80SR_ROM_SIZE]>>,
    dictionary: Box<[u8; DICTIONARY_ROM_SIZE]>,
    cdbios: Box<[u8; CDROM_BIOS_ROM_SIZE]>,
}

impl Pc8801Memory {
    pub(crate) fn new(model: Pc8801Model) -> Self {
        Self {
            state: Pc8801MemoryState::new(model),
            n88: vec![0u8; N88_ROM_SIZE]
                .into_boxed_slice()
                .try_into()
                .unwrap(),
            n88_ext: Box::new([[0u8; N88_EXT_BANK_SIZE]; N88_EXT_BANKS]),
            n_basic: vec![0u8; N_BASIC_ROM_SIZE]
                .into_boxed_slice()
                .try_into()
                .unwrap(),
            n80_mkii: None,
            n80_mkiisr: None,
            n80sr: None,
            dictionary: vec![0u8; DICTIONARY_ROM_SIZE]
                .into_boxed_slice()
                .try_into()
                .unwrap(),
            cdbios: vec![0u8; CDROM_BIOS_ROM_SIZE]
                .into_boxed_slice()
                .try_into()
                .unwrap(),
        }
    }

    /// Captures mutable memory and banking state without ROM bytes.
    pub(crate) fn capture_state(&self) -> Pc8801MemoryState {
        self.state.clone()
    }

    /// Restores mutable memory and banking state without changing ROM bytes.
    pub(crate) fn restore_state(
        &mut self,
        state: Pc8801MemoryState,
    ) -> Result<(), save_state::StateValidationError> {
        if usize::from(state.extram_bank) >= EXT_RAM_BANKS
            || usize::from(state.dic_bank) >= DICTIONARY_ROM_SIZE / DICTIONARY_BANK_SIZE
        {
            return Err(save_state::StateValidationError::new(
                "PC-88 memory banking state is invalid",
            ));
        }
        self.state = state;
        Ok(())
    }

    pub(crate) fn load_n88_rom(&mut self, data: &[u8]) {
        let length = data.len().min(self.n88.len());
        self.n88[..length].copy_from_slice(&data[..length]);
    }

    pub(crate) fn load_n88_ext_rom(&mut self, bank: usize, data: &[u8]) {
        let length = data.len().min(N88_EXT_BANK_SIZE);
        self.n88_ext[bank][..length].copy_from_slice(&data[..length]);
    }

    pub(crate) fn load_n_basic_rom(&mut self, data: &[u8]) {
        let length = data.len().min(self.n_basic.len());
        self.n_basic[..length].copy_from_slice(&data[..length]);
    }

    pub(crate) fn load_n80_mkii_rom(&mut self, data: Option<&[u8]>) {
        self.n80_mkii = data.map(copy_n80_rom);
    }

    pub(crate) fn load_n80_mkiisr_rom(&mut self, data: Option<&[u8]>) {
        self.n80_mkiisr = data.map(copy_n80_rom);
    }

    pub(crate) fn load_n80sr_rom(&mut self, data: Option<&[u8]>) {
        self.n80sr = data.map(|data| {
            let mut rom = Box::new([0u8; N80SR_ROM_SIZE]);
            let length = data.len().min(N80SR_ROM_SIZE);
            rom[..length].copy_from_slice(&data[..length]);
            rom
        });
    }

    pub(crate) fn load_dictionary_rom(&mut self, data: &[u8]) {
        let length = data.len().min(self.dictionary.len());
        self.dictionary[..length].copy_from_slice(&data[..length]);
    }

    pub(crate) fn load_cdbios_rom(&mut self, data: &[u8]) {
        let length = data.len().min(self.cdbios.len());
        self.cdbios[..length].copy_from_slice(&data[..length]);
    }

    pub(crate) fn set_boot_mode(&mut self, boot_mode: BootMode) {
        self.state.boot_mode = boot_mode;
        self.state.n80_ctrl = if boot_mode.is_n80sr() { 0x80 } else { 0x00 };
    }

    pub(crate) fn read_byte(&mut self, address: u16) -> u8 {
        self.read_byte_with_access(address).0
    }

    pub(crate) fn write_byte(&mut self, address: u16, value: u8) {
        self.write_byte_with_access(address, value);
    }

    pub(crate) fn read_byte_with_access(&mut self, address: u16) -> (u8, Pc8801MemoryTarget) {
        match address {
            0x0000..=0x5FFF => self.read_low_region(address),
            0x6000..=0x7FFF => self.read_ext_rom_region(address),
            0x8000..=0xBFFF if self.n80_graphics_mode() => self.read_n80_graphics_region(address),
            0x8000..=0xBFFF if self.state.boot_mode.is_n_family() => (
                self.state.ram[address as usize],
                Pc8801MemoryTarget::MainRam,
            ),
            0x8000..=0x83FF => (
                self.state.ram[self.text_window_offset(address)],
                Pc8801MemoryTarget::TextWindow,
            ),
            0x8400..=0xBFFF => (
                self.state.ram[address as usize],
                Pc8801MemoryTarget::MainRam,
            ),
            0xC000..=0xEFFF if self.state.boot_mode.is_n_family() => (
                self.state.ram[address as usize],
                Pc8801MemoryTarget::MainRam,
            ),
            0xC000..=0xEFFF => self.read_graphics_region(address),
            0xF000..=0xFFFF if self.state.boot_mode.is_n_family() => (
                self.state.ram[address as usize],
                Pc8801MemoryTarget::MainRam,
            ),
            0xF000..=0xFFFF => self.read_high_region(address),
        }
    }

    pub(crate) fn write_byte_with_access(&mut self, address: u16, value: u8) -> Pc8801MemoryTarget {
        match address {
            0x0000..=0x7FFF => self.write_low_region(address, value),
            0x8000..=0xBFFF if self.n80_graphics_mode() => {
                self.write_n80_graphics_region(address, value)
            }
            0x8000..=0xBFFF if self.state.boot_mode.is_n_family() => {
                self.state.ram[address as usize] = value;
                Pc8801MemoryTarget::MainRam
            }
            0x8000..=0x83FF => {
                let offset = self.text_window_offset(address);
                self.state.ram[offset] = value;
                Pc8801MemoryTarget::TextWindow
            }
            0x8400..=0xBFFF => {
                self.state.ram[address as usize] = value;
                Pc8801MemoryTarget::MainRam
            }
            0xC000..=0xEFFF if self.state.boot_mode.is_n_family() => {
                self.state.ram[address as usize] = value;
                Pc8801MemoryTarget::MainRam
            }
            0xC000..=0xEFFF => self.write_graphics_region(address, value),
            0xF000..=0xFFFF if self.state.boot_mode.is_n_family() => {
                self.state.ram[address as usize] = value;
                Pc8801MemoryTarget::MainRam
            }
            0xF000..=0xFFFF => self.write_high_region(address, value),
        }
    }

    fn read_low_region(&self, address: u16) -> (u8, Pc8801MemoryTarget) {
        if self.extram_read_enabled() {
            return (
                self.state.ext_ram[self.extram_offset(address)],
                Pc8801MemoryTarget::ExtensionRam,
            );
        }
        if self.mmode() {
            return (
                self.state.ram[address as usize],
                Pc8801MemoryTarget::MainRam,
            );
        }
        if self.state.cdrom_bank {
            return (self.cdbios_byte(address), Pc8801MemoryTarget::BasicRom);
        }
        if self.state.boot_mode.is_n_family() {
            return (self.n_basic_rom_byte(address), Pc8801MemoryTarget::BasicRom);
        }
        if self.rmode() {
            return (self.n_basic_rom_byte(address), Pc8801MemoryTarget::BasicRom);
        }
        (self.n88[address as usize], Pc8801MemoryTarget::BasicRom)
    }

    fn read_ext_rom_region(&self, address: u16) -> (u8, Pc8801MemoryTarget) {
        if self.extram_read_enabled() {
            return (
                self.state.ext_ram[self.extram_offset(address)],
                Pc8801MemoryTarget::ExtensionRam,
            );
        }
        if self.mmode() {
            return (
                self.state.ram[address as usize],
                Pc8801MemoryTarget::MainRam,
            );
        }
        if self.state.cdrom_bank {
            return (self.cdbios_byte(address), Pc8801MemoryTarget::BasicRom);
        }
        if self.state.boot_mode.is_n_family() {
            return (self.n_basic_rom_byte(address), Pc8801MemoryTarget::BasicRom);
        }
        if self.rmode() {
            return (self.n_basic_rom_byte(address), Pc8801MemoryTarget::BasicRom);
        }
        if self.ierom() {
            return (self.n88[address as usize], Pc8801MemoryTarget::BasicRom);
        }
        let bank = (self.state.misc_ctrl & MISC_CTRL_EROMSL_MASK) as usize;
        (
            self.n88_ext[bank][(address - 0x6000) as usize],
            Pc8801MemoryTarget::BasicRom,
        )
    }

    fn write_low_region(&mut self, address: u16, value: u8) -> Pc8801MemoryTarget {
        if self.extram_write_enabled() {
            let offset = self.extram_offset(address);
            self.state.ext_ram[offset] = value;
            return Pc8801MemoryTarget::ExtensionRam;
        }
        if self.state.boot_mode.is_n80_family() && !self.mmode() {
            return Pc8801MemoryTarget::MainRam;
        }
        self.state.ram[address as usize] = value;
        Pc8801MemoryTarget::MainRam
    }

    fn n_basic_rom_byte(&self, address: u16) -> u8 {
        match self.state.boot_mode {
            BootMode::N80 => self
                .n80_mkii
                .as_deref()
                .map_or(0xFF, |rom| rom[address as usize]),
            BootMode::N80SR => self.n80sr_rom_byte(address),
            BootMode::N | BootMode::V1S | BootMode::V1H | BootMode::V2 => {
                self.n_basic[address as usize]
            }
        }
    }

    fn n80sr_rom_byte(&self, address: u16) -> u8 {
        let sr_rom_mode = self.state.n80_ctrl & 0x80 != 0;
        if sr_rom_mode && let Some(rom) = self.n80sr.as_deref() {
            let offset = if address >= 0x6000 && !self.ierom() {
                0x8000 + (address - 0x6000) as usize
            } else {
                address as usize
            };
            if offset < rom.len() {
                return rom[offset];
            }
        }

        self.n80_mkiisr
            .as_deref()
            .map_or(0xFF, |rom| rom[address as usize])
    }

    fn read_n80_graphics_region(&mut self, address: u16) -> (u8, Pc8801MemoryTarget) {
        if self.gvam() {
            if self.gam() {
                return (
                    self.alu_read_at_offset((address - N80_GVRAM_REGION_START) as usize),
                    Pc8801MemoryTarget::GvramAlu,
                );
            }
            return (
                self.state.ram[address as usize],
                Pc8801MemoryTarget::MainRam,
            );
        }
        match self.state.gvram_sel {
            GvramSelect::Blue => (
                self.state.gvram
                    [gvram_offset_from_start(PLANE_BLUE, N80_GVRAM_REGION_START, address)],
                Pc8801MemoryTarget::GvramPlane,
            ),
            GvramSelect::Red => (
                self.state.gvram
                    [gvram_offset_from_start(PLANE_RED, N80_GVRAM_REGION_START, address)],
                Pc8801MemoryTarget::GvramPlane,
            ),
            GvramSelect::Green => (
                self.state.gvram
                    [gvram_offset_from_start(PLANE_GREEN, N80_GVRAM_REGION_START, address)],
                Pc8801MemoryTarget::GvramPlane,
            ),
            GvramSelect::MainRam => (
                self.state.ram[address as usize],
                Pc8801MemoryTarget::MainRam,
            ),
        }
    }

    fn write_n80_graphics_region(&mut self, address: u16, value: u8) -> Pc8801MemoryTarget {
        if self.gvam() {
            if self.gam() {
                self.alu_write_at_offset((address - N80_GVRAM_REGION_START) as usize, value);
                return Pc8801MemoryTarget::GvramAlu;
            }
            self.state.ram[address as usize] = value;
            return Pc8801MemoryTarget::MainRam;
        }
        match self.state.gvram_sel {
            GvramSelect::Blue => {
                self.state.gvram
                    [gvram_offset_from_start(PLANE_BLUE, N80_GVRAM_REGION_START, address)] = value;
                Pc8801MemoryTarget::GvramPlane
            }
            GvramSelect::Red => {
                self.state.gvram
                    [gvram_offset_from_start(PLANE_RED, N80_GVRAM_REGION_START, address)] = value;
                Pc8801MemoryTarget::GvramPlane
            }
            GvramSelect::Green => {
                self.state.gvram
                    [gvram_offset_from_start(PLANE_GREEN, N80_GVRAM_REGION_START, address)] = value;
                Pc8801MemoryTarget::GvramPlane
            }
            GvramSelect::MainRam => {
                self.state.ram[address as usize] = value;
                Pc8801MemoryTarget::MainRam
            }
        }
    }

    fn read_graphics_region(&mut self, address: u16) -> (u8, Pc8801MemoryTarget) {
        if self.dictionary_enabled() {
            return (
                self.read_dictionary(address),
                Pc8801MemoryTarget::DictionaryRom,
            );
        }
        if self.gvam() {
            if self.gam() {
                return (self.alu_read(address), Pc8801MemoryTarget::GvramAlu);
            }
            return (
                self.state.ram[address as usize],
                Pc8801MemoryTarget::MainRam,
            );
        }
        match self.state.gvram_sel {
            GvramSelect::Blue => (
                self.state.gvram[gvram_offset(PLANE_BLUE, address)],
                Pc8801MemoryTarget::GvramPlane,
            ),
            GvramSelect::Red => (
                self.state.gvram[gvram_offset(PLANE_RED, address)],
                Pc8801MemoryTarget::GvramPlane,
            ),
            GvramSelect::Green => (
                self.state.gvram[gvram_offset(PLANE_GREEN, address)],
                Pc8801MemoryTarget::GvramPlane,
            ),
            GvramSelect::MainRam => (
                self.state.ram[address as usize],
                Pc8801MemoryTarget::MainRam,
            ),
        }
    }

    fn write_graphics_region(&mut self, address: u16, value: u8) -> Pc8801MemoryTarget {
        if self.gvam() {
            if self.gam() {
                self.alu_write(address, value);
                return Pc8801MemoryTarget::GvramAlu;
            }
            self.state.ram[address as usize] = value;
            return Pc8801MemoryTarget::MainRam;
        }
        match self.state.gvram_sel {
            GvramSelect::Blue => {
                self.state.gvram[gvram_offset(PLANE_BLUE, address)] = value;
                Pc8801MemoryTarget::GvramPlane
            }
            GvramSelect::Red => {
                self.state.gvram[gvram_offset(PLANE_RED, address)] = value;
                Pc8801MemoryTarget::GvramPlane
            }
            GvramSelect::Green => {
                self.state.gvram[gvram_offset(PLANE_GREEN, address)] = value;
                Pc8801MemoryTarget::GvramPlane
            }
            GvramSelect::MainRam => {
                self.state.ram[address as usize] = value;
                Pc8801MemoryTarget::MainRam
            }
        }
    }

    fn read_high_region(&mut self, address: u16) -> (u8, Pc8801MemoryTarget) {
        if self.dictionary_enabled() {
            return (
                self.read_dictionary(address),
                Pc8801MemoryTarget::DictionaryRom,
            );
        }
        if self.gvam() {
            if self.gam() {
                return (self.alu_read(address), Pc8801MemoryTarget::GvramAlu);
            }
            return self.read_high_ram_or_text(address);
        }
        match self.state.gvram_sel {
            GvramSelect::Blue => (
                self.state.gvram[gvram_offset(PLANE_BLUE, address)],
                Pc8801MemoryTarget::GvramPlane,
            ),
            GvramSelect::Red => (
                self.state.gvram[gvram_offset(PLANE_RED, address)],
                Pc8801MemoryTarget::GvramPlane,
            ),
            GvramSelect::Green => (
                self.state.gvram[gvram_offset(PLANE_GREEN, address)],
                Pc8801MemoryTarget::GvramPlane,
            ),
            GvramSelect::MainRam => self.read_high_ram_or_text(address),
        }
    }

    fn write_high_region(&mut self, address: u16, value: u8) -> Pc8801MemoryTarget {
        if self.gvam() {
            if self.gam() {
                self.alu_write(address, value);
                return Pc8801MemoryTarget::GvramAlu;
            }
            return self.write_high_ram_or_text(address, value);
        }
        match self.state.gvram_sel {
            GvramSelect::Blue => {
                self.state.gvram[gvram_offset(PLANE_BLUE, address)] = value;
                Pc8801MemoryTarget::GvramPlane
            }
            GvramSelect::Red => {
                self.state.gvram[gvram_offset(PLANE_RED, address)] = value;
                Pc8801MemoryTarget::GvramPlane
            }
            GvramSelect::Green => {
                self.state.gvram[gvram_offset(PLANE_GREEN, address)] = value;
                Pc8801MemoryTarget::GvramPlane
            }
            GvramSelect::MainRam => self.write_high_ram_or_text(address, value),
        }
    }

    fn read_high_ram_or_text(&self, address: u16) -> (u8, Pc8801MemoryTarget) {
        if self.text_vram_visible() {
            return (
                self.state.tvram[(address - HIGH_REGION_START) as usize],
                Pc8801MemoryTarget::TextVram,
            );
        }
        (
            self.state.ram[address as usize],
            Pc8801MemoryTarget::MainRam,
        )
    }

    fn write_high_ram_or_text(&mut self, address: u16, value: u8) -> Pc8801MemoryTarget {
        if self.text_vram_visible() {
            self.state.tvram[(address - HIGH_REGION_START) as usize] = value;
            return Pc8801MemoryTarget::TextVram;
        }
        self.state.ram[address as usize] = value;
        Pc8801MemoryTarget::MainRam
    }

    fn read_dictionary(&self, address: u16) -> u8 {
        let bank = (self.state.dic_bank & DICTIONARY_BANK_MASK) as usize;
        let offset = bank * DICTIONARY_BANK_SIZE + (address - GVRAM_REGION_START) as usize;
        self.dictionary[offset]
    }

    fn alu_read(&mut self, address: u16) -> u8 {
        self.alu_read_at_offset((address - GVRAM_REGION_START) as usize)
    }

    fn alu_read_at_offset(&mut self, offset: usize) -> u8 {
        let blue = self.state.gvram[PLANE_BLUE * GVRAM_PLANE_SIZE + offset];
        let red = self.state.gvram[PLANE_RED * GVRAM_PLANE_SIZE + offset];
        let green = self.state.gvram[PLANE_GREEN * GVRAM_PLANE_SIZE + offset];
        self.state.alu_registers = [blue, red, green];

        let blue_term = if self.state.alu_ctrl2 & ALU_CTRL2_PLANE_BLUE_NORMAL != 0 {
            blue
        } else {
            !blue
        };
        let red_term = if self.state.alu_ctrl2 & ALU_CTRL2_PLANE_RED_NORMAL != 0 {
            red
        } else {
            !red
        };
        let green_term = if self.state.alu_ctrl2 & ALU_CTRL2_PLANE_GREEN_NORMAL != 0 {
            green
        } else {
            !green
        };
        blue_term & red_term & green_term
    }

    fn alu_write(&mut self, address: u16, value: u8) {
        self.alu_write_at_offset((address - GVRAM_REGION_START) as usize, value);
    }

    fn alu_write_at_offset(&mut self, offset: usize, value: u8) {
        let mode = (self.state.alu_ctrl2 >> ALU_CTRL2_GDM_SHIFT) & ALU_CTRL2_GDM_MASK;
        match mode {
            0 => {
                for plane in 0..GVRAM_PLANES {
                    let operation = (self.state.alu_ctrl1 >> plane) & ALU_OP_NOOP;
                    let cell = &mut self.state.gvram[plane * GVRAM_PLANE_SIZE + offset];
                    match operation {
                        ALU_OP_RESET => *cell &= !value,
                        ALU_OP_SET => *cell |= value,
                        ALU_OP_XOR => *cell ^= value,
                        _ => {}
                    }
                }
            }
            1 => {
                for plane in 0..GVRAM_PLANES {
                    self.state.gvram[plane * GVRAM_PLANE_SIZE + offset] =
                        self.state.alu_registers[plane];
                }
            }
            2 => {
                self.state.gvram[PLANE_BLUE * GVRAM_PLANE_SIZE + offset] =
                    self.state.alu_registers[PLANE_RED];
            }
            _ => {
                self.state.gvram[PLANE_RED * GVRAM_PLANE_SIZE + offset] =
                    self.state.alu_registers[PLANE_BLUE];
            }
        }
    }

    fn text_window_offset(&self, address: u16) -> usize {
        if self.text_window_active() {
            (((self.state.window_bank as usize) << 8) + ((address as usize) & TEXT_WINDOW_MASK))
                & (MAIN_RAM_SIZE - 1)
        } else {
            address as usize
        }
    }

    fn text_window_active(&self) -> bool {
        !self.mmode() && !self.rmode()
    }

    fn text_vram_visible(&self) -> bool {
        if self.state.boot_mode.forces_high_ram() {
            return false;
        }
        self.state.misc_ctrl & MISC_CTRL_TEXT_MODE == 0
    }

    pub(crate) fn high_region_opcode_fetch_uses_tvram_wait(&self) -> bool {
        if !self.text_vram_visible() {
            return false;
        }
        if self.gvam() && self.gam() {
            return false;
        }
        matches!(self.state.gvram_sel, GvramSelect::MainRam)
    }

    fn extram_offset(&self, address: u16) -> usize {
        let bank = (self.state.extram_bank as usize) % EXT_RAM_BANKS;
        bank * EXT_RAM_BANK_SIZE + address as usize
    }

    fn mmode(&self) -> bool {
        self.state.gfx_ctrl & GFX_CTRL_MMODE != 0
    }

    fn rmode(&self) -> bool {
        self.state.gfx_ctrl & GFX_CTRL_RMODE != 0
    }

    fn cdbios_byte(&self, address: u16) -> u8 {
        let window = if self.state.gfx_ctrl & CDROM_BIOS_WINDOW_SELECT != 0 {
            CDROM_BIOS_WINDOW_SIZE
        } else {
            0
        };
        self.cdbios[(address as usize) | window]
    }

    fn ierom(&self) -> bool {
        self.state.ext_rom_bank & EXT_ROM_BANK_IEROM != 0
    }

    fn gvam(&self) -> bool {
        self.state.misc_ctrl & MISC_CTRL_GVAM != 0
    }

    fn gam(&self) -> bool {
        self.state.alu_ctrl2 & ALU_CTRL2_GAM != 0
    }

    fn extram_read_enabled(&self) -> bool {
        self.state.extram_mode & EXTRAM_MODE_READ_ENABLE != 0
    }

    fn extram_write_enabled(&self) -> bool {
        self.state.extram_mode & EXTRAM_MODE_WRITE_ENABLE != 0
    }

    fn n80_graphics_mode(&self) -> bool {
        self.state.boot_mode.is_n80_family()
    }

    fn dictionary_enabled(&self) -> bool {
        self.state.dic_ctrl & DICTIONARY_CTRL_DISABLE == 0
    }
}

fn gvram_offset(plane: usize, address: u16) -> usize {
    gvram_offset_from_start(plane, GVRAM_REGION_START, address)
}

fn gvram_offset_from_start(plane: usize, start: u16, address: u16) -> usize {
    plane * GVRAM_PLANE_SIZE + (address - start) as usize
}

fn copy_n80_rom(data: &[u8]) -> Box<[u8; N80_ROM_SIZE]> {
    let mut rom = Box::new([0u8; N80_ROM_SIZE]);
    let length = data.len().min(N80_ROM_SIZE);
    rom[..length].copy_from_slice(&data[..length]);
    rom
}

#[cfg(test)]
mod tests {
    use super::*;

    const N88_FILL: u8 = 0x88;
    const N_BASIC_FILL: u8 = 0x80;
    const N80_MKII_FILL: u8 = 0x82;
    const N80_MKIISR_FILL: u8 = 0x83;
    const N80SR_FILL: u8 = 0x84;
    const N80SR_EXT_FILL: u8 = 0x85;

    fn memory_with_test_roms() -> Pc8801Memory {
        let mut memory = Pc8801Memory::new(Pc8801Model::PC8801MC);
        memory.load_n88_rom(&[N88_FILL; N88_ROM_SIZE]);
        memory.load_n_basic_rom(&[N_BASIC_FILL; N_BASIC_ROM_SIZE]);
        memory.load_n80_mkii_rom(Some(&[N80_MKII_FILL; N80_ROM_SIZE]));
        memory.load_n80_mkiisr_rom(Some(&[N80_MKIISR_FILL; N80_ROM_SIZE]));
        let mut n80sr = vec![N80SR_FILL; N80SR_ROM_SIZE];
        n80sr[0x8000..].fill(N80SR_EXT_FILL);
        memory.load_n80sr_rom(Some(&n80sr));
        for bank in 0..N88_EXT_BANKS {
            memory.load_n88_ext_rom(bank, &[0xE0 | bank as u8; N88_EXT_BANK_SIZE]);
        }
        // Dictionary ROM: each byte encodes its 16 KiB bank index.
        let mut dictionary = vec![0u8; DICTIONARY_ROM_SIZE];
        for (index, byte) in dictionary.iter_mut().enumerate() {
            *byte = (index / DICTIONARY_BANK_SIZE) as u8;
        }
        memory.load_dictionary_rom(&dictionary);
        // These fixtures exercise the N88/N80/RAM banking; the CD-ROM BIOS bank
        // (enabled at reset on the MC) would otherwise override the low region.
        memory.state.cdrom_bank = false;
        memory
    }

    #[test]
    fn cdrom_bank_overrides_low_region_with_window_select() {
        let mut memory = memory_with_test_roms();
        // Lower 32 KiB window encodes the offset; upper window sets bit 15.
        let mut cdbios = vec![0u8; CDROM_BIOS_ROM_SIZE];
        cdbios[0x0000] = 0xC0;
        cdbios[0x5FFF] = 0xC1;
        cdbios[0x8000] = 0xD0;
        cdbios[0x8000 | 0x5FFF] = 0xD1;
        memory.load_cdbios_rom(&cdbios);

        // Bank enabled, lower window (gfx_ctrl bit 2 clear): CD BIOS at 0x0000.
        memory.state.cdrom_bank = true;
        memory.state.gfx_ctrl = 0;
        assert_eq!(memory.read_byte(0x0000), 0xC0);
        assert_eq!(memory.read_byte(0x5FFF), 0xC1);

        // gfx_ctrl bit 2 selects the upper 32 KiB window.
        memory.state.gfx_ctrl = CDROM_BIOS_WINDOW_SELECT;
        assert_eq!(memory.read_byte(0x0000), 0xD0);
        assert_eq!(memory.read_byte(0x5FFF), 0xD1);

        // Bank disabled: the low region falls back to the N88 ROM.
        memory.state.cdrom_bank = false;
        memory.state.gfx_ctrl = 0;
        assert_eq!(memory.read_byte(0x0000), N88_FILL);
    }

    #[test]
    fn low_region_selects_n88_n80_and_ram() {
        let mut memory = memory_with_test_roms();
        // Default: N88 ROM.
        assert_eq!(memory.read_byte(0x0000), N88_FILL);
        assert_eq!(memory.read_byte(0x5FFF), N88_FILL);

        // RMODE selects N-BASIC.
        memory.state.gfx_ctrl = GFX_CTRL_RMODE;
        assert_eq!(memory.read_byte(0x0000), N_BASIC_FILL);

        // MMODE (full RAM) takes priority over the ROM select.
        memory.state.gfx_ctrl = GFX_CTRL_MMODE | GFX_CTRL_RMODE;
        memory.state.ram[0x0100] = 0x5A;
        assert_eq!(memory.read_byte(0x0100), 0x5A);
    }

    #[test]
    fn n_family_rom_selection_prefers_matching_optional_roms() {
        let mut memory = memory_with_test_roms();

        memory.set_boot_mode(BootMode::N);
        assert_eq!(memory.read_byte(0x0000), N_BASIC_FILL);

        memory.set_boot_mode(BootMode::N80);
        assert_eq!(memory.read_byte(0x0000), N80_MKII_FILL);
        // The PC-8001mkII has no N80SR personality: the port 0x33 ROM-select bit
        // does not switch the N80 ROM out.
        memory.state.n80_ctrl |= 0x80;
        assert_eq!(memory.read_byte(0x0000), N80_MKII_FILL);

        memory.set_boot_mode(BootMode::N80SR);
        assert_eq!(memory.read_byte(0x0000), N80SR_FILL);
        memory.state.ext_rom_bank = 0;
        assert_eq!(memory.read_byte(0x6000), N80SR_EXT_FILL);

        memory.state.n80_ctrl &= !0x80;
        assert_eq!(memory.read_byte(0x0000), N80_MKIISR_FILL);
    }

    #[test]
    fn n80_modes_protect_the_rom_window_until_all_ram_mode() {
        let mut memory = memory_with_test_roms();
        memory.set_boot_mode(BootMode::N80SR);

        memory.write_byte(0x0100, 0x5A);
        assert_eq!(memory.state.ram[0x0100], 0x00);
        assert_eq!(memory.read_byte(0x0100), N80SR_FILL);

        memory.state.gfx_ctrl = GFX_CTRL_MMODE;
        memory.write_byte(0x0100, 0x5A);
        assert_eq!(memory.state.ram[0x0100], 0x5A);
        assert_eq!(memory.read_byte(0x0100), 0x5A);

        memory.set_boot_mode(BootMode::N);
        memory.state.gfx_ctrl = 0;
        memory.write_byte(0x0101, 0x6B);
        assert_eq!(memory.state.ram[0x0101], 0x6B);
        assert_eq!(memory.read_byte(0x0101), N_BASIC_FILL);
    }

    #[test]
    fn ext_rom_region_banks_and_ierom() {
        let mut memory = memory_with_test_roms();
        // N88 mode, IEROM clear: extension banks selected by EROMSL.
        memory.state.ext_rom_bank = 0;
        for bank in 0..N88_EXT_BANKS {
            memory.state.misc_ctrl = bank as u8;
            assert_eq!(memory.read_byte(0x6000), 0xE0 | bank as u8);
        }
        // IEROM set: N88 main ROM at 0x6000-0x7FFF.
        memory.state.ext_rom_bank = EXT_ROM_BANK_IEROM;
        assert_eq!(memory.read_byte(0x6000), N88_FILL);
    }

    #[test]
    fn ext_ram_read_write_enable_and_bank() {
        let mut memory = memory_with_test_roms();
        // Seed distinct values in two banks.
        memory.state.ext_ram[0x0010] = 0x11;
        memory.state.ext_ram[EXT_RAM_BANK_SIZE + 0x0010] = 0x22;

        memory.state.extram_mode = EXTRAM_MODE_READ_ENABLE;
        memory.state.extram_bank = 0;
        assert_eq!(memory.read_byte(0x0010), 0x11);
        memory.state.extram_bank = 1;
        assert_eq!(memory.read_byte(0x0010), 0x22);

        // Without write-enable, writes go to main RAM, not EXT RAM.
        memory.state.extram_mode = EXTRAM_MODE_READ_ENABLE;
        memory.write_byte(0x0020, 0x33);
        assert_eq!(memory.state.ram[0x0020], 0x33);
        assert_eq!(memory.state.ext_ram[EXT_RAM_BANK_SIZE + 0x0020], 0x00);

        // With write-enable, writes land in the selected EXT RAM bank.
        memory.state.extram_mode = EXTRAM_MODE_READ_ENABLE | EXTRAM_MODE_WRITE_ENABLE;
        memory.write_byte(0x0020, 0x44);
        assert_eq!(memory.state.ext_ram[EXT_RAM_BANK_SIZE + 0x0020], 0x44);
    }

    #[test]
    fn writes_under_rom_update_ram_not_rom() {
        let mut memory = memory_with_test_roms();
        // N88 mode: reads come from ROM, writes go to the underlying RAM.
        memory.write_byte(0x0000, 0x99);
        assert_eq!(memory.read_byte(0x0000), N88_FILL);
        assert_eq!(memory.state.ram[0x0000], 0x99);

        // Switching to full RAM mode now reveals the written byte.
        memory.state.gfx_ctrl = GFX_CTRL_MMODE;
        assert_eq!(memory.read_byte(0x0000), 0x99);
    }

    #[test]
    fn text_window_maps_and_increments() {
        let mut memory = memory_with_test_roms();
        // N88 ROM mode: the text window is active.
        memory.state.window_bank = 0xF3;
        memory.write_byte(0x8001, 0x7E);
        let windowed = ((0xF3usize << 8) + (0x8001usize & TEXT_WINDOW_MASK)) & (MAIN_RAM_SIZE - 1);
        assert_eq!(memory.state.ram[windowed], 0x7E);
        assert_eq!(memory.read_byte(0x8001), 0x7E);

        // The 1 KiB CPU-side window can cross a 256-byte RAM page.
        memory.state.window_bank = 0x01;
        memory.write_byte(0x810F, 0xA5);
        assert_eq!(memory.state.ram[0x020F], 0xA5);
        assert_eq!(memory.read_byte(0x810F), 0xA5);

        // Port 0x78 increments the window base.
        memory.state.window_bank = memory.state.window_bank.wrapping_add(1);
        memory.write_byte(0x8002, 0x6D);
        let windowed = ((0x02usize << 8) + (0x8002usize & TEXT_WINDOW_MASK)) & (MAIN_RAM_SIZE - 1);
        assert_eq!(memory.state.ram[windowed], 0x6D);

        // Full RAM mode disables the window: 0x8000 is plain RAM.
        memory.state.gfx_ctrl = GFX_CTRL_MMODE;
        memory.write_byte(0x8003, 0x42);
        assert_eq!(memory.state.ram[0x8003], 0x42);
    }

    #[test]
    fn gvram_plane_select_at_c000() {
        let mut memory = memory_with_test_roms();
        memory.state.gvram_sel = GvramSelect::Blue;
        memory.write_byte(0xC000, 0x01);
        memory.state.gvram_sel = GvramSelect::Red;
        memory.write_byte(0xC000, 0x02);
        memory.state.gvram_sel = GvramSelect::Green;
        memory.write_byte(0xC000, 0x03);

        assert_eq!(memory.state.gvram[gvram_offset(PLANE_BLUE, 0xC000)], 0x01);
        assert_eq!(memory.state.gvram[gvram_offset(PLANE_RED, 0xC000)], 0x02);
        assert_eq!(memory.state.gvram[gvram_offset(PLANE_GREEN, 0xC000)], 0x03);

        memory.state.gvram_sel = GvramSelect::Green;
        assert_eq!(memory.read_byte(0xC000), 0x03);

        // 0x5F selects main RAM.
        memory.state.gvram_sel = GvramSelect::MainRam;
        memory.write_byte(0xC000, 0xAB);
        assert_eq!(memory.state.ram[0xC000], 0xAB);
    }

    #[test]
    fn n80_modes_map_selected_gvram_at_8000_and_keep_high_ram() {
        let mut memory = memory_with_test_roms();
        memory.set_boot_mode(BootMode::N80SR);

        memory.state.gvram_sel = GvramSelect::Blue;
        memory.write_byte(0x9800, 0x12);
        assert_eq!(
            memory.state.gvram[gvram_offset_from_start(PLANE_BLUE, N80_GVRAM_REGION_START, 0x9800)],
            0x12
        );
        assert_eq!(memory.read_byte(0x9800), 0x12);
        assert_eq!(memory.state.ram[0x9800], 0x00);

        memory.write_byte(0xC000, 0x34);
        assert_eq!(memory.state.ram[0xC000], 0x34);
        assert_eq!(memory.read_byte(0xC000), 0x34);
        assert_eq!(memory.state.gvram[gvram_offset(PLANE_BLUE, 0xC000)], 0x00);
    }

    #[test]
    fn plain_n_mode_keeps_8000_to_ffff_as_main_ram() {
        let mut memory = memory_with_test_roms();
        memory.set_boot_mode(BootMode::N);

        memory.state.gvram_sel = GvramSelect::Blue;
        memory.write_byte(0x9800, 0x12);
        assert_eq!(memory.state.ram[0x9800], 0x12);
        assert_eq!(memory.read_byte(0x9800), 0x12);
        assert_eq!(
            memory.state.gvram[gvram_offset_from_start(PLANE_BLUE, N80_GVRAM_REGION_START, 0x9800)],
            0x00
        );

        memory.write_byte(0xC000, 0x34);
        assert_eq!(memory.state.ram[0xC000], 0x34);
        assert_eq!(memory.read_byte(0xC000), 0x34);
        assert_eq!(memory.state.gvram[gvram_offset(PLANE_BLUE, 0xC000)], 0x00);
    }

    #[test]
    fn high_region_text_vram_vs_ram() {
        let mut memory = memory_with_test_roms();
        memory.state.gvram_sel = GvramSelect::MainRam;

        // V2 mode, text mode bit clear: TVRAM visible at 0xF000-0xFFFF.
        memory.set_boot_mode(BootMode::V2);
        memory.state.misc_ctrl = 0;
        memory.write_byte(0xF000, 0xC3);
        assert_eq!(memory.state.tvram[0], 0xC3);
        assert_eq!(memory.read_byte(0xF000), 0xC3);

        // Text mode bit set: main RAM appears instead.
        memory.state.misc_ctrl = MISC_CTRL_TEXT_MODE;
        memory.write_byte(0xF000, 0x3C);
        assert_eq!(memory.state.ram[0xF000], 0x3C);
        assert_eq!(memory.read_byte(0xF000), 0x3C);

        for boot_mode in [BootMode::V1S, BootMode::N, BootMode::N80, BootMode::N80SR] {
            memory.set_boot_mode(boot_mode);
            memory.state.misc_ctrl = 0;
            memory.write_byte(0xF010, 0x5E);
            assert_eq!(memory.state.ram[0xF010], 0x5E);
            assert_eq!(memory.read_byte(0xF010), 0x5E);
        }
    }

    #[test]
    fn dictionary_window_and_priority_over_alu() {
        let mut memory = memory_with_test_roms();

        // Dictionary enabled (bit 0 low): reads return the bank index byte and
        // take priority over main RAM, selected GVRAM planes, and the ALU.
        memory.state.dic_ctrl = 0x00;
        memory.state.dic_bank = 5;
        assert_eq!(memory.read_byte(0xC000), 5);
        memory.state.gvram_sel = GvramSelect::Blue;
        assert_eq!(memory.read_byte(0xC000), 5);
        memory.state.misc_ctrl = MISC_CTRL_GVAM;
        memory.state.alu_ctrl2 = ALU_CTRL2_GAM;
        memory.state.dic_bank = 17;
        assert_eq!(memory.read_byte(0xEFFF), 17);

        // Dictionary disabled (bit 0 high): the ALU read path is used instead.
        let (blue, red, green) = (0b1111_1100u8, 0b0111_1110u8, 0b0011_1111u8);
        memory.state.dic_ctrl = DICTIONARY_CTRL_DISABLE;
        memory.state.gvram[gvram_offset(PLANE_BLUE, 0xC000)] = blue;
        memory.state.gvram[gvram_offset(PLANE_RED, 0xC000)] = red;
        memory.state.gvram[gvram_offset(PLANE_GREEN, 0xC000)] = green;
        // All planes marked normal -> blue & red & green.
        memory.state.alu_ctrl2 = ALU_CTRL2_GAM
            | ALU_CTRL2_PLANE_BLUE_NORMAL
            | ALU_CTRL2_PLANE_RED_NORMAL
            | ALU_CTRL2_PLANE_GREEN_NORMAL;
        assert_eq!(memory.read_byte(0xC000), blue & red & green);
    }

    #[test]
    fn alu_read_combine_with_plane_invert() {
        let mut memory = memory_with_test_roms();
        memory.state.misc_ctrl = MISC_CTRL_GVAM;
        memory.state.dic_ctrl = DICTIONARY_CTRL_DISABLE;
        memory.state.gvram[gvram_offset(PLANE_BLUE, 0xC100)] = 0b1010_1010;
        memory.state.gvram[gvram_offset(PLANE_RED, 0xC100)] = 0b1100_1100;
        memory.state.gvram[gvram_offset(PLANE_GREEN, 0xC100)] = 0b1111_0000;

        // Blue inverted (bit clear), red and green normal.
        memory.state.alu_ctrl2 =
            ALU_CTRL2_GAM | ALU_CTRL2_PLANE_RED_NORMAL | ALU_CTRL2_PLANE_GREEN_NORMAL;
        let expected = (!0b1010_1010u8) & 0b1100_1100 & 0b1111_0000;
        assert_eq!(memory.read_byte(0xC100), expected);
        // Reading latches the (non-inverted) plane bytes into the ALU registers.
        assert_eq!(
            memory.state.alu_registers,
            [0b1010_1010, 0b1100_1100, 0b1111_0000]
        );
    }

    #[test]
    fn alu_write_logic_operations() {
        let mut memory = memory_with_test_roms();
        memory.state.misc_ctrl = MISC_CTRL_GVAM;
        memory.state.alu_ctrl2 = ALU_CTRL2_GAM; // GDM = 0 (per-plane logic op).
        memory.state.gvram[gvram_offset(PLANE_BLUE, 0xC200)] = 0b1111_0000;
        memory.state.gvram[gvram_offset(PLANE_RED, 0xC200)] = 0b1111_0000;
        memory.state.gvram[gvram_offset(PLANE_GREEN, 0xC200)] = 0b1111_0000;

        // Each plane uses bit i (low) and bit i+4 (high) of alu_ctrl1:
        // blue (plane 0) SET -> 0x01; red (plane 1) RESET -> 0x00;
        // green (plane 2) XOR -> 0x40.
        memory.state.alu_ctrl1 = 0x01 | 0x40;
        memory.write_byte(0xC200, 0b0000_1111);

        assert_eq!(
            memory.state.gvram[gvram_offset(PLANE_BLUE, 0xC200)],
            0b1111_0000 | 0b0000_1111
        );
        assert_eq!(
            memory.state.gvram[gvram_offset(PLANE_RED, 0xC200)],
            0b1111_0000 & !0b0000_1111
        );
        assert_eq!(
            memory.state.gvram[gvram_offset(PLANE_GREEN, 0xC200)],
            0b1111_0000 ^ 0b0000_1111
        );
    }

    #[test]
    fn alu_write_register_and_plane_copies() {
        let mut memory = memory_with_test_roms();
        memory.state.misc_ctrl = MISC_CTRL_GVAM;

        // GDM = 1 (0x10): write the three latched ALU registers to the planes.
        memory.state.alu_registers = [0xAA, 0xBB, 0xCC];
        memory.state.alu_ctrl2 = ALU_CTRL2_GAM | (0x01 << ALU_CTRL2_GDM_SHIFT);
        memory.write_byte(0xC300, 0x00);
        assert_eq!(memory.state.gvram[gvram_offset(PLANE_BLUE, 0xC300)], 0xAA);
        assert_eq!(memory.state.gvram[gvram_offset(PLANE_RED, 0xC300)], 0xBB);
        assert_eq!(memory.state.gvram[gvram_offset(PLANE_GREEN, 0xC300)], 0xCC);

        // GDM = 2 (0x20): copy red (plane 1) to blue (plane 0).
        memory.state.alu_ctrl2 = ALU_CTRL2_GAM | (0x02 << ALU_CTRL2_GDM_SHIFT);
        memory.write_byte(0xC300, 0x00);
        assert_eq!(memory.state.gvram[gvram_offset(PLANE_BLUE, 0xC300)], 0xBB);

        // GDM = 3 (0x30): copy blue (plane 0) to red (plane 1).
        memory.state.gvram[gvram_offset(PLANE_BLUE, 0xC300)] = 0x5A;
        memory.state.alu_ctrl2 = ALU_CTRL2_GAM | (0x03 << ALU_CTRL2_GDM_SHIFT);
        memory.write_byte(0xC300, 0x00);
        assert_eq!(memory.state.gvram[gvram_offset(PLANE_RED, 0xC300)], 0xAA);
    }
}
