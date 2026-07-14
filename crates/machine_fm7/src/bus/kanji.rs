//! JIS level-1 kanji ROM addressed through the main CPU `0xFD20-0xFD23` ports.
//!
//! The 128 KiB ROM holds one 16x16 glyph per JIS character code as 32 bytes: 16
//! rows of two bytes each. Software latches a character code into the 16-bit
//! address register (`0xFD20` high byte, `0xFD21` low byte) and then reads the two
//! bytes of one glyph row: `0xFD22` yields the left (even) byte and `0xFD23` the
//! right (odd) byte. The 16-bit code selects a two-byte word, so it is shifted
//! left by one to index the byte array. The ROM is optional on the FM-7; when it
//! is absent both data ports read as open bus.

/// Bytes stored per addressable word (left byte followed by right byte).
const BYTES_PER_WORD: u32 = 2;
/// Mask keeping the byte index within the 128 KiB ROM.
const ADDRESS_MASK: u32 = 0x1_FFFF;
/// Open-bus value returned when no kanji ROM is fitted.
const OPEN_BUS: u8 = 0xFF;

/// The kanji ROM window and its 16-bit address latch.
#[derive(Default)]
pub(super) struct KanjiRom {
    /// The 128 KiB ROM image, or `None` when no ROM is fitted.
    rom: Option<Box<[u8]>>,
    /// The latched character code selecting the current glyph word.
    address: u16,
}

impl KanjiRom {
    /// Creates an empty kanji window with no ROM installed.
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// Installs the kanji ROM image, or clears it when `rom` is `None`.
    pub(super) fn install_rom(&mut self, rom: Option<&[u8]>) {
        self.rom = rom.map(|bytes| bytes.to_vec().into_boxed_slice());
    }

    /// Latches the high byte of the character code (`0xFD20`).
    pub(super) fn write_address_high(&mut self, value: u8) {
        self.address = (self.address & 0x00FF) | (u16::from(value) << 8);
    }

    /// Latches the low byte of the character code (`0xFD21`).
    pub(super) fn write_address_low(&mut self, value: u8) {
        self.address = (self.address & 0xFF00) | u16::from(value);
    }

    /// Reads the left (even) byte of the latched glyph word (`0xFD22`).
    pub(super) fn read_left(&self) -> u8 {
        self.read_byte(self.word_base())
    }

    /// Reads the right (odd) byte of the latched glyph word (`0xFD23`).
    pub(super) fn read_right(&self) -> u8 {
        self.read_byte(self.word_base() + 1)
    }

    /// The byte index of the latched glyph word's left byte.
    fn word_base(&self) -> u32 {
        (u32::from(self.address) * BYTES_PER_WORD) & ADDRESS_MASK
    }

    /// Reads a byte from the ROM, or open bus when no ROM is fitted.
    fn read_byte(&self, index: u32) -> u8 {
        match &self.rom {
            Some(rom) => rom.get(index as usize).copied().unwrap_or(OPEN_BUS),
            None => OPEN_BUS,
        }
    }
}
