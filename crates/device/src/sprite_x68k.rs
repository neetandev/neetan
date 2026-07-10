//! Sharp X68000 CYNTHIA sprite and background controller state.
//!
//! The controller owns the 128-entry sprite scroll table, the background
//! and screen-timing registers, and the 32 KiB sprite pattern RAM. The
//! background tile maps live inside the pattern RAM. Holes in the window
//! read `0xFFFF` and ignore writes; byte writes to the scroll table and
//! the pattern RAM duplicate the byte into both lanes of the word.

/// Number of sprite scroll-register entries.
pub const X68K_SPRITE_COUNT: usize = 128;
/// Number of 16-bit words in the sprite pattern RAM.
pub const X68K_SPRITE_PATTERN_WORDS: usize = 0x4000;

/// Base address of the sprite window.
const SPRITE_BASE: u32 = 0xEB0000;
/// First address past the sprite scroll table.
const SCROLL_TABLE_END: u32 = 0xEB0400;
/// First address of the register block.
const REGISTER_BASE: u32 = 0xEB0800;
/// First address past the register block.
const REGISTER_END: u32 = 0xEB0812;
/// First address of the pattern RAM.
const PATTERN_BASE: u32 = 0xEB8000;

/// Writable-bit masks of the four sprite scroll words.
const SCROLL_WORD_MASKS: [u16; 4] = [0x03FF, 0x03FF, 0xCFFF, 0x0007];
/// Writable-bit mask of the background control register.
const BACKGROUND_CONTROL_MASK: u16 = 0x063F;
/// Background control bit routing CPU pattern access to the absent chip.
const BACKGROUND_CONTROL_MPU_CHIP_SELECT: u16 = 0x0400;
/// Writable-bit mask of the horizontal front-porch end register.
const HORIZONTAL_FRONT_END_MASK: u16 = 0x00FF;
/// Writable-bit mask of the horizontal back-porch end register.
const HORIZONTAL_BACK_END_MASK: u16 = 0x003F;
/// Writable-bit mask of the vertical back-porch end register.
const VERTICAL_BACK_END_MASK: u16 = 0x00FF;
/// Writable-bit mask of the resolution register.
const RESOLUTION_MASK: u16 = 0x001F;
/// Writable-bit mask of the background scroll registers.
const BACKGROUND_SCROLL_MASK: u16 = 0x03FF;

/// Sharp X68000 CYNTHIA sprite controller.
#[derive(Debug, Clone)]
pub struct SpriteX68k {
    scroll: [[u16; 4]; X68K_SPRITE_COUNT],
    background_scroll: [u16; 4],
    background_control: u16,
    horizontal_front_end: u16,
    horizontal_back_end: u16,
    vertical_back_end: u16,
    resolution: u16,
    pattern: Box<[u16]>,
}

impl Default for SpriteX68k {
    fn default() -> Self {
        Self::new()
    }
}

impl SpriteX68k {
    /// Creates cleared sprite-controller state.
    pub fn new() -> Self {
        Self {
            scroll: [[0; 4]; X68K_SPRITE_COUNT],
            background_scroll: [0; 4],
            background_control: 0,
            horizontal_front_end: 0,
            horizontal_back_end: 0,
            vertical_back_end: 0,
            resolution: 0,
            pattern: vec![0; X68K_SPRITE_PATTERN_WORDS].into_boxed_slice(),
        }
    }

    /// Resets all registers and the pattern RAM.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Reads one word of the sprite window.
    pub fn read_word(&self, address: u32) -> u16 {
        let relative = address - SPRITE_BASE;
        if address < SCROLL_TABLE_END {
            let entry = (relative >> 3) as usize;
            self.scroll[entry][((relative >> 1) & 3) as usize]
        } else if address < REGISTER_BASE {
            0xFFFF
        } else if address < REGISTER_END {
            match address & 0x001E {
                0x00 => self.background_scroll[0],
                0x02 => self.background_scroll[1],
                0x04 => self.background_scroll[2],
                0x06 => self.background_scroll[3],
                0x08 => self.background_control,
                0x0A => self.horizontal_front_end,
                0x0C => self.horizontal_back_end,
                0x0E => self.vertical_back_end,
                _ => self.resolution,
            }
        } else if address < PATTERN_BASE {
            0xFFFF
        } else if self.mpu_chip_selected() {
            0x0000
        } else {
            self.pattern[((relative >> 1) & (X68K_SPRITE_PATTERN_WORDS as u32 - 1)) as usize]
        }
    }

    /// Returns whether CPU pattern access targets the absent second chip.
    const fn mpu_chip_selected(&self) -> bool {
        self.background_control & BACKGROUND_CONTROL_MPU_CHIP_SELECT != 0
    }

    /// Writes one word of the sprite window.
    pub fn write_word(&mut self, address: u32, value: u16) {
        let relative = address - SPRITE_BASE;
        if address < SCROLL_TABLE_END {
            let entry = (relative >> 3) as usize;
            let word = ((relative >> 1) & 3) as usize;
            self.scroll[entry][word] = value & SCROLL_WORD_MASKS[word];
        } else if address < REGISTER_BASE {
        } else if address < REGISTER_END {
            match address & 0x001E {
                0x00 => self.background_scroll[0] = value & BACKGROUND_SCROLL_MASK,
                0x02 => self.background_scroll[1] = value & BACKGROUND_SCROLL_MASK,
                0x04 => self.background_scroll[2] = value & BACKGROUND_SCROLL_MASK,
                0x06 => self.background_scroll[3] = value & BACKGROUND_SCROLL_MASK,
                0x08 => self.background_control = value & BACKGROUND_CONTROL_MASK,
                0x0A => self.horizontal_front_end = value & HORIZONTAL_FRONT_END_MASK,
                0x0C => self.horizontal_back_end = value & HORIZONTAL_BACK_END_MASK,
                0x0E => self.vertical_back_end = value & VERTICAL_BACK_END_MASK,
                _ => self.resolution = value & RESOLUTION_MASK,
            }
        } else if address < PATTERN_BASE {
        } else if !self.mpu_chip_selected() {
            let index = ((relative >> 1) & (X68K_SPRITE_PATTERN_WORDS as u32 - 1)) as usize;
            self.pattern[index] = value;
        }
    }

    /// Reads one byte of the sprite window.
    pub fn read_byte(&self, address: u32) -> u8 {
        let word = self.read_word(address & !1);
        if address & 1 == 0 {
            (word >> 8) as u8
        } else {
            word as u8
        }
    }

    /// Writes one byte; the scroll table and pattern RAM take both lanes.
    pub fn write_byte(&mut self, address: u32, value: u8) {
        let aligned = address & !1;
        if !(SCROLL_TABLE_END..PATTERN_BASE).contains(&address) {
            self.write_word(aligned, u16::from(value) << 8 | u16::from(value));
        } else {
            let old = self.read_word(aligned);
            let merged = if address & 1 == 0 {
                u16::from(value) << 8 | old & 0x00FF
            } else {
                old & 0xFF00 | u16::from(value)
            };
            self.write_word(aligned, merged);
        }
    }

    /// Returns one sprite's horizontal position word.
    pub const fn sprite_x(&self, index: usize) -> u16 {
        self.scroll[index & (X68K_SPRITE_COUNT - 1)][0]
    }

    /// Returns one sprite's vertical position word.
    pub const fn sprite_y(&self, index: usize) -> u16 {
        self.scroll[index & (X68K_SPRITE_COUNT - 1)][1]
    }

    /// Returns one sprite's flip, palette-block, and pattern word.
    pub const fn sprite_pattern_word(&self, index: usize) -> u16 {
        self.scroll[index & (X68K_SPRITE_COUNT - 1)][2]
    }

    /// Returns one sprite's priority (zero hides the sprite).
    pub const fn sprite_priority(&self, index: usize) -> u16 {
        self.scroll[index & (X68K_SPRITE_COUNT - 1)][3] & 0x0003
    }

    /// Returns one background layer's horizontal scroll.
    pub const fn background_scroll_x(&self, layer: usize) -> u16 {
        self.background_scroll[(layer & 1) * 2]
    }

    /// Returns one background layer's vertical scroll.
    pub const fn background_scroll_y(&self, layer: usize) -> u16 {
        self.background_scroll[(layer & 1) * 2 + 1]
    }

    /// Returns the background control register.
    pub const fn background_control(&self) -> u16 {
        self.background_control
    }

    /// Returns the horizontal front-porch end register.
    pub const fn horizontal_front_end(&self) -> u16 {
        self.horizontal_front_end
    }

    /// Returns the horizontal back-porch end register.
    pub const fn horizontal_back_end(&self) -> u16 {
        self.horizontal_back_end
    }

    /// Returns the vertical back-porch end register.
    pub const fn vertical_back_end(&self) -> u16 {
        self.vertical_back_end
    }

    /// Returns the resolution register.
    pub const fn resolution(&self) -> u16 {
        self.resolution
    }

    /// Returns the sprite scroll table image.
    pub const fn scroll_data(&self) -> &[[u16; 4]; X68K_SPRITE_COUNT] {
        &self.scroll
    }

    /// Returns the pattern RAM image.
    pub fn pattern_data(&self) -> &[u16] {
        &self.pattern
    }
}
