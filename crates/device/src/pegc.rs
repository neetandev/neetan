//! PEGC (Packed-pixel Extended Graphics Controller) - 256-color graphics for PC-9821.
//!
//! Provides two operating modes:
//! - **Packed pixel mode**: Each VRAM byte = one 8-bit palette index.
//!   Two 32 KB bank-switched windows at A8000-AFFFF and B0000-B7FFF
//!   select from 16 banks of 32 KB within 512 KB extended VRAM.
//! - **Plane mode**: 8-plane architecture with ROP operations, similar to EGC
//!   but extended to 8 planes for 256 colors.
//!
//! MMIO registers live at E0000-E7FFF (replacing E-plane VRAM when active).
//! 256-color palette uses the same I/O ports (0xA8/0xAA/0xAC/0xAE) as the
//! analog palette but with 8-bit index (0-255) and 8-bit color components.

/// PEGC extended VRAM size: 512 KB.
pub const PEGC_VRAM_SIZE: usize = 0x80000;

/// PEGC bank size: 32 KB.
const BANK_SIZE: usize = 0x8000;

/// MMIO region 1 offsets (E0000-E00FF): bank select registers.
const MMIO1_BANK_A8: u32 = 0x0004;
const MMIO1_BANK_B0: u32 = 0x0006;

/// MMIO region 2 base offset.
const MMIO2_BASE: u32 = 0x0100;

/// MMIO region 2 offsets (relative to E0100).
const REG_MODE: u32 = 0x00;
const REG_VRAM_ENABLE: u32 = 0x02;
const REG_PLANE_ACCESS: u32 = 0x04;
const REG_PLANE_ROP_LOW: u32 = 0x08;
const REG_PLANE_ROP_HIGH: u32 = 0x09;
const REG_DATA_SELECT: u32 = 0x0A;
const REG_MASK_BYTE0: u32 = 0x0C;
const REG_MASK_BYTE1: u32 = 0x0D;
const REG_MASK_BYTE2: u32 = 0x0E;
const REG_MASK_BYTE3: u32 = 0x0F;
const REG_LENGTH_LOW: u32 = 0x10;
const REG_LENGTH_HIGH: u32 = 0x11;
const REG_SHIFT_READ: u32 = 0x12;
const REG_SHIFT_WRITE: u32 = 0x13;
const REG_PALETTE1: u32 = 0x14;
const REG_PALETTE2: u32 = 0x18;
const REG_PATTERN: u32 = 0x20;

/// Screen mode for PEGC 256-color display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PegcScreenMode {
    /// 640x400, 2-screen mode (two 256 KB pages).
    TwoScreen,
    /// 640x480, 1-screen mode (single 512 KB page).
    OneScreen,
}

/// Snapshot of the PEGC state for save/restore.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PegcState {
    /// Master 256-color mode enable (port 0x6A commands 0x20/0x21).
    pub enabled: bool,
    /// Screen mode selection (port 0x6A commands 0x68/0x69).
    pub screen_mode: PegcScreenMode,
    /// Bank index (0-15) for the A8000-AFFFF window (MMIO E0004h).
    pub bank_a8: u8,
    /// Bank index (0-15) for the B0000-B7FFF window (MMIO E0006h).
    pub bank_b0: u8,
    /// Mode register (MMIO E0100h). Bit 0: 0 = packed pixel, 1 = plane mode.
    pub mode_register: u8,
    /// Upper VRAM enable (MMIO E0102h bit 0). Enables flat access at F00000h.
    pub upper_vram_enabled: bool,
    /// Plane access mask (MMIO E0104h). Bit per plane: 0 = allow write, 1 = inhibit.
    pub plane_access_mask: u8,
    /// ROP register (MMIO E0108h). See bit field documentation on the struct methods.
    pub rop_register: u16,
    /// Data select register (MMIO E010Ah).
    pub data_select: u8,
    /// Write mask (MMIO E010Ch-E010Fh). Per-pixel write enable: 1 = allow, 0 = inhibit.
    pub write_mask: u32,
    /// Block transfer length minus 1 (MMIO E0110h, 12 bits).
    pub block_length: u16,
    /// Read shift amount (MMIO E0112h bits 4-0).
    pub shift_read: u8,
    /// Write shift amount (MMIO E0113h bits 4-0).
    pub shift_write: u8,
    /// Foreground palette color (MMIO E0114h).
    pub palette_color_1: u8,
    /// Background palette color (MMIO E0118h).
    pub palette_color_2: u8,
    /// Pattern register raw bytes (MMIO E0120h-E019Fh, 128 bytes).
    pub pattern_data: Box<[u8; 128]>,
    /// Last VRAM data read buffer for plane mode source operations.
    pub last_vram_data: [u8; 64],
    /// Number of valid bytes in `last_vram_data`.
    pub last_data_length: i32,
    /// Remaining block transfer count for plane mode.
    pub remain: u32,
    /// 256-color palette: 256 entries of [green, red, blue], 8-bit components.
    pub palette_256: Box<[[u8; 3]; 256]>,
    /// Currently selected palette index (port 0xA8 in PEGC mode).
    pub palette_index: u8,
}

/// PEGC controller.
pub struct Pegc {
    /// Embedded state for save/restore.
    pub state: PegcState,
}

impl Default for Pegc {
    fn default() -> Self {
        Self::new()
    }
}

impl Pegc {
    /// Creates a new PEGC controller in its default (disabled) state.
    pub fn new() -> Self {
        Self {
            state: PegcState {
                enabled: false,
                screen_mode: PegcScreenMode::TwoScreen,
                bank_a8: 0,
                bank_b0: 0,
                mode_register: 0,
                upper_vram_enabled: false,
                plane_access_mask: 0,
                rop_register: 0,
                data_select: 0,
                write_mask: 0,
                block_length: 0,
                shift_read: 0,
                shift_write: 0,
                palette_color_1: 0,
                palette_color_2: 0,
                pattern_data: Box::new([0u8; 128]),
                last_vram_data: [0u8; 64],
                last_data_length: 0,
                remain: 0,
                palette_256: Box::new([[0u8; 3]; 256]),
                palette_index: 0,
            },
        }
    }

    /// Returns whether 256-color mode is active.
    pub fn is_256_color_active(&self) -> bool {
        self.state.enabled
    }

    /// Returns whether packed pixel mode is selected (mode register bit 0 = 0).
    pub fn is_packed_pixel_mode(&self) -> bool {
        self.state.mode_register & 1 == 0
    }

    /// Returns whether plane mode is selected (mode register bit 0 = 1).
    pub fn is_plane_mode(&self) -> bool {
        self.state.mode_register & 1 != 0
    }

    /// Returns whether flat VRAM access at F00000h is enabled.
    pub fn is_upper_vram_enabled(&self) -> bool {
        self.state.upper_vram_enabled
    }

    /// Enables or disables 256-color mode (called from port 0x6A handler).
    pub fn set_256_color_enabled(&mut self, enabled: bool) {
        self.state.enabled = enabled;
    }

    /// Sets the screen mode (called from port 0x6A handler).
    pub fn set_screen_mode(&mut self, one_screen: bool) {
        self.state.screen_mode = if one_screen {
            PegcScreenMode::OneScreen
        } else {
            PegcScreenMode::TwoScreen
        };
    }

    /// Sets VRAM access mode to plane (called from port 0x6A command 0x62).
    pub fn set_vram_access_mode_plane(&mut self) {
        self.state.mode_register = 1;
    }

    /// Sets VRAM access mode to packed pixel (called from port 0x6A command 0x63).
    pub fn set_vram_access_mode_packed(&mut self) {
        self.state.mode_register = 0;
    }

    fn reset_transfer_state_from_length(&mut self) {
        self.state.remain = u32::from(self.state.block_length) + 1;
        self.state.last_data_length = 0;
    }

    fn source_buffer_length(&self) -> usize {
        self.state
            .last_data_length
            .clamp(0, self.state.last_vram_data.len() as i32) as usize
    }

    fn append_source_buffer(&mut self, index: usize, value: u8) {
        let write_index = self.source_buffer_length() + index;
        if write_index < self.state.last_vram_data.len() {
            self.state.last_vram_data[write_index] = value;
        }
    }

    fn finish_source_buffer_append(&mut self, pixels: u32) {
        let length = self
            .source_buffer_length()
            .saturating_add(pixels as usize)
            .min(self.state.last_vram_data.len());
        self.state.last_data_length = length as i32;
    }

    fn consume_source_buffer(&mut self, pixels: usize) {
        let length = self.source_buffer_length();
        let consumed = pixels.min(length);
        if consumed == 0 {
            return;
        }

        self.state.last_vram_data.copy_within(consumed..length, 0);
        self.state.last_vram_data[length - consumed..length].fill(0);
        self.state.last_data_length = (length - consumed) as i32;
    }

    /// Writes the 256-color palette index register (port 0xA8 in PEGC mode).
    pub fn write_palette_index(&mut self, value: u8) {
        self.state.palette_index = value;
    }

    /// Writes a 256-color palette component for the currently selected index.
    ///
    /// `component`: 0 = green (0xAA), 1 = red (0xAC), 2 = blue (0xAE).
    pub fn write_palette_component(&mut self, component: usize, value: u8) {
        let index = self.state.palette_index as usize;
        self.state.palette_256[index][component] = value;
    }

    /// Reads a 256-color palette component for the currently selected index.
    ///
    /// `component`: 0 = green (0xAA), 1 = red (0xAC), 2 = blue (0xAE).
    pub fn read_palette_component(&self, component: usize) -> u8 {
        let index = self.state.palette_index as usize;
        self.state.palette_256[index][component]
    }

    /// Reads a byte from the PEGC MMIO register space (offset within E0000-E7FFF).
    pub fn mmio_read_byte(&self, offset: u32) -> u8 {
        match offset {
            MMIO1_BANK_A8 => self.state.bank_a8,
            MMIO1_BANK_B0 => self.state.bank_b0,
            o if o >= MMIO2_BASE => self.mmio2_read_byte(o - MMIO2_BASE),
            _ => 0x00,
        }
    }

    /// Writes a byte to the PEGC MMIO register space (offset within E0000-E7FFF).
    pub fn mmio_write_byte(&mut self, offset: u32, value: u8) {
        match offset {
            MMIO1_BANK_A8 => self.state.bank_a8 = value & 0x0F,
            MMIO1_BANK_B0 => self.state.bank_b0 = value & 0x0F,
            o if o >= MMIO2_BASE => self.mmio2_write_byte(o - MMIO2_BASE, value),
            _ => {}
        }
    }

    /// Reads a word from the PEGC MMIO register space (offset within E0000-E7FFF).
    pub fn mmio_read_word(&self, offset: u32) -> u16 {
        if offset >= MMIO2_BASE {
            return self.mmio2_read_word(offset - MMIO2_BASE);
        }
        let low = self.mmio_read_byte(offset) as u16;
        let high = self.mmio_read_byte(offset + 1) as u16;
        low | (high << 8)
    }

    /// Writes a word to the PEGC MMIO register space (offset within E0000-E7FFF).
    pub fn mmio_write_word(&mut self, offset: u32, value: u16) {
        if offset >= MMIO2_BASE {
            self.mmio2_write_word(offset - MMIO2_BASE, value);
            return;
        }
        self.mmio_write_byte(offset, value as u8);
        self.mmio_write_byte(offset + 1, (value >> 8) as u8);
    }

    /// Reads a dword from the PEGC MMIO register space (offset within E0000-E7FFF).
    pub fn mmio_read_dword(&self, offset: u32) -> u32 {
        if offset >= MMIO2_BASE {
            return self.mmio2_read_dword(offset - MMIO2_BASE);
        }
        let low = u32::from(self.mmio_read_word(offset));
        let high = u32::from(self.mmio_read_word(offset + 2));
        low | (high << 16)
    }

    /// Writes a dword to the PEGC MMIO register space (offset within E0000-E7FFF).
    pub fn mmio_write_dword(&mut self, offset: u32, value: u32) {
        if offset >= MMIO2_BASE {
            self.mmio2_write_dword(offset - MMIO2_BASE, value);
            return;
        }
        self.mmio_write_word(offset, value as u16);
        self.mmio_write_word(offset + 2, (value >> 16) as u16);
    }

    fn mmio2_read_byte(&self, offset: u32) -> u8 {
        if (REG_PATTERN..=0x9F).contains(&offset) {
            return self.pattern_read_byte(offset - REG_PATTERN);
        }
        match offset {
            REG_MODE => self.state.mode_register,
            REG_VRAM_ENABLE => u8::from(self.state.upper_vram_enabled),
            REG_PLANE_ACCESS => self.state.plane_access_mask,
            REG_PLANE_ROP_LOW => self.state.rop_register as u8,
            REG_PLANE_ROP_HIGH => (self.state.rop_register >> 8) as u8,
            REG_DATA_SELECT => self.state.data_select,
            REG_MASK_BYTE0 => self.state.write_mask as u8,
            REG_MASK_BYTE1 => (self.state.write_mask >> 8) as u8,
            REG_MASK_BYTE2 => (self.state.write_mask >> 16) as u8,
            REG_MASK_BYTE3 => (self.state.write_mask >> 24) as u8,
            REG_LENGTH_LOW => self.state.block_length as u8,
            REG_LENGTH_HIGH => (self.state.block_length >> 8) as u8,
            REG_SHIFT_READ => self.state.shift_read,
            REG_SHIFT_WRITE => self.state.shift_write,
            REG_PALETTE1 => self.state.palette_color_1,
            REG_PALETTE2 => self.state.palette_color_2,
            _ => 0x00,
        }
    }

    fn mmio2_write_byte(&mut self, offset: u32, value: u8) {
        if (REG_PATTERN..=0x9F).contains(&offset) {
            self.pattern_write_byte(offset - REG_PATTERN, value);
            return;
        }
        match offset {
            REG_MODE => self.state.mode_register = value & 0x01,
            REG_VRAM_ENABLE => self.state.upper_vram_enabled = value & 0x01 != 0,
            REG_PLANE_ACCESS => self.state.plane_access_mask = value,
            REG_PLANE_ROP_LOW => {
                self.state.rop_register = (self.state.rop_register & 0xFF00) | u16::from(value);
            }
            REG_PLANE_ROP_HIGH => {
                self.state.rop_register =
                    (self.state.rop_register & 0x00FF) | (u16::from(value) << 8);
            }
            REG_DATA_SELECT => self.state.data_select = value,
            REG_MASK_BYTE0 => {
                self.state.write_mask = (self.state.write_mask & 0xFFFF_FF00) | u32::from(value);
            }
            REG_MASK_BYTE1 => {
                self.state.write_mask =
                    (self.state.write_mask & 0xFFFF_00FF) | (u32::from(value) << 8);
            }
            REG_MASK_BYTE2 => {
                self.state.write_mask =
                    (self.state.write_mask & 0xFF00_FFFF) | (u32::from(value) << 16);
            }
            REG_MASK_BYTE3 => {
                self.state.write_mask =
                    (self.state.write_mask & 0x00FF_FFFF) | (u32::from(value) << 24);
            }
            REG_LENGTH_LOW => {
                self.state.block_length = (self.state.block_length & 0x0F00) | u16::from(value);
            }
            REG_LENGTH_HIGH => {
                self.state.block_length =
                    (self.state.block_length & 0x00FF) | ((u16::from(value) & 0x0F) << 8);
            }
            REG_SHIFT_READ => self.state.shift_read = value & 0x1F,
            REG_SHIFT_WRITE => self.state.shift_write = value & 0x1F,
            REG_PALETTE1 => self.state.palette_color_1 = value,
            REG_PALETTE2 => self.state.palette_color_2 = value,
            _ => {}
        }

        if offset < REG_PATTERN {
            self.reset_transfer_state_from_length();
        }
    }

    fn mmio2_read_word(&self, offset: u32) -> u16 {
        if (REG_PATTERN..=0x9F).contains(&offset) {
            return self.pattern_read_word(offset - REG_PATTERN);
        }
        let low = self.mmio2_read_byte(offset) as u16;
        let high = self.mmio2_read_byte(offset + 1) as u16;
        low | (high << 8)
    }

    fn mmio2_write_word(&mut self, offset: u32, value: u16) {
        if (REG_PATTERN..=0x9F).contains(&offset) {
            self.pattern_write_word(offset - REG_PATTERN, value);
            return;
        }
        self.mmio2_write_byte(offset, value as u8);
        self.mmio2_write_byte(offset + 1, (value >> 8) as u8);
    }

    fn mmio2_read_dword(&self, offset: u32) -> u32 {
        if (REG_PATTERN..=0x9F).contains(&offset) {
            return self.pattern_read_dword(offset - REG_PATTERN);
        }
        let low = u32::from(self.mmio2_read_word(offset));
        let high = u32::from(self.mmio2_read_word(offset + 2));
        low | (high << 16)
    }

    fn mmio2_write_dword(&mut self, offset: u32, value: u32) {
        if (REG_PATTERN..=0x9F).contains(&offset) {
            self.pattern_write_dword(offset - REG_PATTERN, value);
            return;
        }
        self.mmio2_write_word(offset, value as u16);
        self.mmio2_write_word(offset + 2, (value >> 16) as u16);
    }

    fn plane_pattern_read_u32(&self, plane: u32) -> u32 {
        let plane_offset = (plane * 4) as usize;
        u32::from(self.state.pattern_data[plane_offset])
            | (u32::from(self.state.pattern_data[plane_offset + 1]) << 8)
            | (u32::from(self.state.pattern_data[plane_offset + 2]) << 16)
            | (u32::from(self.state.pattern_data[plane_offset + 3]) << 24)
    }

    fn plane_pattern_write_bit(&mut self, plane: u32, bit: u32, value: u32) {
        let plane_offset = (plane * 4) as usize;
        let mut reg = self.plane_pattern_read_u32(plane);
        reg = (reg & !(1u32 << bit)) | ((value & 1) << bit);
        self.state.pattern_data[plane_offset] = reg as u8;
        self.state.pattern_data[plane_offset + 1] = (reg >> 8) as u8;
        self.state.pattern_data[plane_offset + 2] = (reg >> 16) as u8;
        self.state.pattern_data[plane_offset + 3] = (reg >> 24) as u8;
    }

    fn pattern_read_byte(&self, pattern_pos: u32) -> u8 {
        if pattern_pos & 0x03 != 0 {
            return 0x00;
        }
        if self.state.rop_register & 0x8000 != 0 {
            if pattern_pos >= 0x80 {
                return 0x00;
            }
            let bit = pattern_pos / 4;
            let mut color = 0u8;
            for plane in 0..8u32 {
                let reg = self.plane_pattern_read_u32(plane);
                color |= (((reg >> bit) & 1) as u8) << plane;
            }
            color
        } else {
            if pattern_pos >= 0x40 {
                return 0x00;
            }
            self.state.pattern_data[pattern_pos as usize]
        }
    }

    fn pattern_write_byte(&mut self, pattern_pos: u32, value: u8) {
        if pattern_pos & 0x03 != 0 {
            return;
        }
        if self.state.rop_register & 0x8000 != 0 {
            if pattern_pos >= 0x80 {
                return;
            }
            let bit = pattern_pos / 4;
            for plane in 0..8u32 {
                let val = (u32::from(value) >> plane) & 1;
                self.plane_pattern_write_bit(plane, bit, val);
            }
        } else {
            if pattern_pos >= 0x40 {
                return;
            }
            self.state.pattern_data[pattern_pos as usize] = value;
        }
    }

    fn pattern_read_word(&self, pattern_pos: u32) -> u16 {
        if pattern_pos & 0x03 != 0 {
            return 0x0000;
        }
        if self.state.rop_register & 0x8000 != 0 {
            if pattern_pos >= 0x80 {
                return 0x0000;
            }
            let bit = pattern_pos / 4;
            let mut color = 0u16;
            for plane in 0..8u32 {
                let reg = self.plane_pattern_read_u32(plane);
                color |= (((reg >> bit) & 1) as u16) << plane;
            }
            color
        } else {
            if pattern_pos >= 0x40 {
                return 0x0000;
            }
            u16::from(self.state.pattern_data[pattern_pos as usize])
                | (u16::from(self.state.pattern_data[pattern_pos as usize + 1]) << 8)
        }
    }

    fn pattern_write_word(&mut self, pattern_pos: u32, value: u16) {
        if pattern_pos & 0x03 != 0 {
            return;
        }
        if self.state.rop_register & 0x8000 != 0 {
            if pattern_pos >= 0x80 {
                return;
            }
            let bit = pattern_pos / 4;
            for plane in 0..8u32 {
                let val = (u32::from(value) >> plane) & 1;
                self.plane_pattern_write_bit(plane, bit, val);
            }
        } else {
            if pattern_pos >= 0x40 {
                return;
            }
            self.state.pattern_data[pattern_pos as usize] = value as u8;
            self.state.pattern_data[pattern_pos as usize + 1] = (value >> 8) as u8;
        }
    }

    fn pattern_read_dword(&self, pattern_pos: u32) -> u32 {
        if pattern_pos & 0x03 != 0 {
            return 0;
        }
        if self.state.rop_register & 0x8000 != 0 {
            if pattern_pos >= 0x80 {
                return 0;
            }
            let bit = pattern_pos / 4;
            let mut color = 0u32;
            for plane in 0..8u32 {
                let reg = self.plane_pattern_read_u32(plane);
                color |= ((reg >> bit) & 1) << plane;
            }
            color
        } else {
            if pattern_pos >= 0x40 {
                return 0;
            }
            self.plane_pattern_read_u32(pattern_pos / 4)
        }
    }

    fn pattern_write_dword(&mut self, pattern_pos: u32, value: u32) {
        if pattern_pos & 0x03 != 0 {
            return;
        }
        if self.state.rop_register & 0x8000 != 0 {
            if pattern_pos >= 0x80 {
                return;
            }
            let bit = pattern_pos / 4;
            for plane in 0..8u32 {
                let val = (value >> plane) & 1;
                self.plane_pattern_write_bit(plane, bit, val);
            }
        } else {
            if pattern_pos >= 0x40 {
                return;
            }
            let plane_offset = pattern_pos as usize;
            self.state.pattern_data[plane_offset] = value as u8;
            self.state.pattern_data[plane_offset + 1] = (value >> 8) as u8;
            self.state.pattern_data[plane_offset + 2] = (value >> 16) as u8;
            self.state.pattern_data[plane_offset + 3] = (value >> 24) as u8;
        }
    }

    /// Reads a byte from PEGC VRAM in packed pixel mode via bank-switched window.
    ///
    /// `window`: 0 = A8000-AFFFF, 1 = B0000-B7FFF.
    /// `offset`: byte offset within the 32 KB window.
    pub fn packed_read_byte(&self, window: u8, offset: u32, vram: &[u8]) -> u8 {
        let bank = if window == 0 {
            self.state.bank_a8
        } else {
            self.state.bank_b0
        };
        let address = bank as usize * BANK_SIZE + (offset as usize & (BANK_SIZE - 1));
        vram[address & (PEGC_VRAM_SIZE - 1)]
    }

    /// Writes a byte to PEGC VRAM in packed pixel mode via bank-switched window.
    pub fn packed_write_byte(&self, window: u8, offset: u32, value: u8, vram: &mut [u8]) {
        let bank = if window == 0 {
            self.state.bank_a8
        } else {
            self.state.bank_b0
        };
        let address = bank as usize * BANK_SIZE + (offset as usize & (BANK_SIZE - 1));
        vram[address & (PEGC_VRAM_SIZE - 1)] = value;
    }

    /// Reads a word from PEGC VRAM in packed pixel mode via bank-switched window.
    pub fn packed_read_word(&self, window: u8, offset: u32, vram: &[u8]) -> u16 {
        let low = self.packed_read_byte(window, offset, vram) as u16;
        let high = self.packed_read_byte(window, offset + 1, vram) as u16;
        low | (high << 8)
    }

    /// Writes a word to PEGC VRAM in packed pixel mode via bank-switched window.
    pub fn packed_write_word(&self, window: u8, offset: u32, value: u16, vram: &mut [u8]) {
        self.packed_write_byte(window, offset, value as u8, vram);
        self.packed_write_byte(window, offset + 1, (value >> 8) as u8, vram);
    }

    /// Reads a word from PEGC VRAM in plane mode.
    ///
    /// Compares 16 consecutive VRAM pixels against palette color 1, returning
    /// a 16-bit mask where bit N is set if pixel N differs from palette color 1
    /// (considering only the planes allowed by the plane access mask).
    /// Optionally updates the pattern register from the read data.
    pub fn plane_read_word(&mut self, offset: u32, vram: &[u8]) -> u16 {
        self.plane_read_impl(offset, vram, 16) as u16
    }

    /// Reads a dword from PEGC VRAM in plane mode.
    ///
    /// Same semantics as `plane_read_word` but processes 32 consecutive pixels
    /// in a single transaction. Shift register bit 4 is valid (16-31 pixel shift)
    /// and the pattern register is updated across the full 32-pixel width.
    pub fn plane_read_dword(&mut self, offset: u32, vram: &[u8]) -> u32 {
        self.plane_read_impl(offset, vram, 32)
    }

    fn plane_read_impl(&mut self, offset: u32, vram: &[u8], pixels: u32) -> u32 {
        let rop_reg = self.state.rop_register;
        let source_shift = u32::from(self.state.shift_read);
        let dest_shift = u32::from(self.state.shift_write);
        let block_length = u32::from(self.state.block_length);
        let shift_direction_decrement = rop_reg & (1 << 9) != 0;
        let source_from_cpu = rop_reg & (1 << 8) != 0;
        let pattern_update = rop_reg & (1 << 13) != 0;
        let plane_mask = self.state.plane_access_mask;
        let palette1 = self.state.palette_color_1;

        let base_address = offset.wrapping_mul(8).wrapping_add(source_shift);
        let base_address = if self.state.remain != block_length + 1 {
            base_address.wrapping_sub(dest_shift)
        } else {
            base_address
        };

        let mut result: u32 = 0;

        if !source_from_cpu {
            for i in 0u32..pixels {
                let pixel_address = if shift_direction_decrement {
                    base_address.wrapping_sub(i) & (PEGC_VRAM_SIZE as u32 - 1)
                } else {
                    base_address.wrapping_add(i) & (PEGC_VRAM_SIZE as u32 - 1)
                };

                let data = vram[pixel_address as usize];

                if (data ^ palette1) & !plane_mask != 0 {
                    result |= 1u32 << i;
                }

                self.append_source_buffer(i as usize, data);

                if pattern_update {
                    for plane in 0..8u32 {
                        let bit_value = u32::from((data >> plane) & 1);
                        self.plane_pattern_write_bit(plane, i, bit_value);
                    }
                }
            }
            self.finish_source_buffer_append(pixels);
        }

        result
    }

    /// Writes a word to PEGC VRAM in plane mode with ROP operations.
    ///
    /// Processes 16 consecutive pixels, applying the configured raster operation
    /// (source, destination, pattern truth table) with plane masking and pixel masking.
    pub fn plane_write_word(&mut self, offset: u32, value: u16, vram: &mut [u8]) {
        self.plane_write_impl(offset, u32::from(value), vram, 16);
    }

    /// Writes a dword to PEGC VRAM in plane mode with ROP operations.
    ///
    /// Same semantics as `plane_write_word` but processes 32 pixels per call.
    /// Shift register bit 4 is valid and the write mask covers all 32 pixels.
    pub fn plane_write_dword(&mut self, offset: u32, value: u32, vram: &mut [u8]) {
        self.plane_write_impl(offset, value, vram, 32);
    }

    fn plane_write_impl(&mut self, offset: u32, value: u32, vram: &mut [u8], pixels: u32) {
        let rop_reg = self.state.rop_register;
        let rop_code = (rop_reg & 0xFF) as u8;
        let rop_method = ((rop_reg >> 10) & 0x03) as u8;
        let rop_enabled = rop_reg & (1 << 12) != 0;
        let source_from_cpu = rop_reg & (1 << 8) != 0;
        let shift_direction_decrement = rop_reg & (1 << 9) != 0;
        let plane_mask = self.state.plane_access_mask;
        let pixel_mask = self.state.write_mask;
        let block_length = u32::from(self.state.block_length);
        let dest_shift = u32::from(self.state.shift_write);

        if self.state.remain == 0 {
            self.state.remain = block_length + 1;
            self.state.last_data_length = 0;
        }

        let pixels_signed = pixels as i32;
        let extended_shift_mode = !source_from_cpu || ((block_length + 1) & (pixels - 1)) != 0;

        let mut base_address = offset.wrapping_mul(8);
        let mut data_length: i32 = pixels_signed;

        if extended_shift_mode {
            if self.state.remain == block_length + 1 {
                base_address = base_address.wrapping_add(dest_shift);
                data_length -= dest_shift as i32;
            }
        } else {
            base_address = base_address.wrapping_add(dest_shift);
        }
        base_address &= PEGC_VRAM_SIZE as u32 - 1;

        let source_buffer_length = self.source_buffer_length();

        for i in 0..data_length {
            let pixel_address = if shift_direction_decrement {
                base_address.wrapping_sub(i as u32) & (PEGC_VRAM_SIZE as u32 - 1)
            } else {
                base_address.wrapping_add(i as u32) & (PEGC_VRAM_SIZE as u32 - 1)
            };
            let pixel_mask_bit = pixel_mask_position(i as u32);

            if pixel_mask & pixel_mask_bit != 0 {
                let source = if source_from_cpu {
                    if value & pixel_mask_bit != 0 {
                        0xFF
                    } else {
                        0x00
                    }
                } else if (i as usize) < source_buffer_length {
                    self.state.last_vram_data[i as usize]
                } else {
                    self.state.remain -= 1;
                    if self.state.remain == 0 {
                        break;
                    }
                    continue;
                };

                let destination = vram[pixel_address as usize];

                if rop_enabled {
                    let (pattern1, pattern2) = self.get_pattern_colors(rop_method, i as u32);
                    let result = apply_rop(
                        rop_code,
                        source,
                        destination,
                        pattern1,
                        pattern2,
                        plane_mask,
                    );
                    vram[pixel_address as usize] = (destination & plane_mask) | result;
                } else {
                    vram[pixel_address as usize] =
                        apply_source_copy(source, destination, plane_mask);
                }
            }

            self.state.remain -= 1;
            if self.state.remain == 0 {
                break;
            }
        }

        if source_from_cpu {
            self.state.last_data_length = 0;
        } else {
            self.consume_source_buffer(pixels as usize);
        }
    }

    fn get_pattern_colors(&self, method: u8, pixel_index: u32) -> (u8, u8) {
        match method {
            0 => {
                let mut color = 0u8;
                for plane in (0..8u32).rev() {
                    let reg_data = self.plane_pattern_read_u32(plane);
                    color |= (((reg_data >> pixel_index) & 1) as u8) << plane;
                }
                (color, color)
            }
            1 => {
                let color = self.state.palette_color_2;
                (color, color)
            }
            2 => {
                let color = self.state.palette_color_1;
                (color, color)
            }
            3 => (self.state.palette_color_1, self.state.palette_color_2),
            _ => (0, 0),
        }
    }
}

/// Computes the pixel mask bit position for a given pixel index within a 16-pixel group.
///
/// The bit layout maps pixel index to bit position within the 32-bit mask:
/// pixels 0-7 map to bits 7-0 (reversed), pixels 8-15 map to bits 15-8 (reversed).
fn pixel_mask_position(pixel_index: u32) -> u32 {
    let byte_group = (pixel_index / 8) * 8;
    let bit_within_byte = 7 - (pixel_index & 7);
    1 << (byte_group + bit_within_byte)
}

/// Applies the 8-bit ROP truth table to source, destination, and pattern values.
///
/// The ROP code encodes 8 possible outcomes for the (S, D, P) truth table:
/// - Bit 7: S=1, D=1, P=1
/// - Bit 6: S=1, D=1, P=0
/// - Bit 5: S=1, D=0, P=1
/// - Bit 4: S=1, D=0, P=0
/// - Bit 3: S=0, D=1, P=1  (uses pattern2)
/// - Bit 2: S=0, D=1, P=0  (uses pattern2)
/// - Bit 1: S=0, D=0, P=1  (uses pattern2)
/// - Bit 0: S=0, D=0, P=0  (uses pattern2)
fn apply_rop(
    rop_code: u8,
    source: u8,
    destination: u8,
    pattern1: u8,
    pattern2: u8,
    plane_mask: u8,
) -> u8 {
    let not_mask = !plane_mask;
    let mut result: u8 = 0;
    if rop_code & (1 << 7) != 0 {
        result |= (source & destination & pattern1) & not_mask;
    }
    if rop_code & (1 << 6) != 0 {
        result |= (source & destination & !pattern1) & not_mask;
    }
    if rop_code & (1 << 5) != 0 {
        result |= (source & !destination & pattern1) & not_mask;
    }
    if rop_code & (1 << 4) != 0 {
        result |= (source & !destination & !pattern1) & not_mask;
    }
    if rop_code & (1 << 3) != 0 {
        result |= (!source & destination & pattern2) & not_mask;
    }
    if rop_code & (1 << 2) != 0 {
        result |= (!source & destination & !pattern2) & not_mask;
    }
    if rop_code & (1 << 1) != 0 {
        result |= (!source & !destination & pattern2) & not_mask;
    }
    if rop_code & 1 != 0 {
        result |= (!source & !destination & !pattern2) & not_mask;
    }
    result
}

/// Applies source copy with plane masking (non-ROP mode).
///
/// For each bit: if source bit is 1, result = (!plane_mask | destination);
/// if source bit is 0, result = (plane_mask & destination).
fn apply_source_copy(source: u8, destination: u8, plane_mask: u8) -> u8 {
    let mut result: u8 = 0;
    for bit in 0..8u8 {
        let mask = 1 << bit;
        if source & mask != 0 {
            result |= (!plane_mask | destination) & mask;
        } else {
            result |= (plane_mask & destination) & mask;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pegc_defaults_to_disabled() {
        let pegc = Pegc::new();
        assert!(!pegc.is_256_color_active());
        assert!(pegc.is_packed_pixel_mode());
        assert!(!pegc.is_plane_mode());
        assert!(!pegc.is_upper_vram_enabled());
        assert_eq!(pegc.state.bank_a8, 0);
        assert_eq!(pegc.state.bank_b0, 0);
        assert_eq!(pegc.state.screen_mode, PegcScreenMode::TwoScreen);
    }

    #[test]
    fn pegc_enable_disable() {
        let mut pegc = Pegc::new();
        pegc.set_256_color_enabled(true);
        assert!(pegc.is_256_color_active());
        pegc.set_256_color_enabled(false);
        assert!(!pegc.is_256_color_active());
    }

    #[test]
    fn pegc_screen_mode_switching() {
        let mut pegc = Pegc::new();
        assert_eq!(pegc.state.screen_mode, PegcScreenMode::TwoScreen);
        pegc.set_screen_mode(true);
        assert_eq!(pegc.state.screen_mode, PegcScreenMode::OneScreen);
        pegc.set_screen_mode(false);
        assert_eq!(pegc.state.screen_mode, PegcScreenMode::TwoScreen);
    }

    #[test]
    fn vram_access_mode_plane_and_packed() {
        let mut pegc = Pegc::new();
        assert!(pegc.is_packed_pixel_mode());

        pegc.set_vram_access_mode_plane();
        assert!(pegc.is_plane_mode());
        assert!(!pegc.is_packed_pixel_mode());

        pegc.set_vram_access_mode_packed();
        assert!(pegc.is_packed_pixel_mode());
        assert!(!pegc.is_plane_mode());
    }

    #[test]
    fn mmio_bank_select_a8_read_write() {
        let mut pegc = Pegc::new();
        pegc.mmio_write_byte(MMIO1_BANK_A8, 0x0A);
        assert_eq!(pegc.mmio_read_byte(MMIO1_BANK_A8), 0x0A);
        pegc.mmio_write_byte(MMIO1_BANK_A8, 0xFF);
        assert_eq!(pegc.mmio_read_byte(MMIO1_BANK_A8), 0x0F);
    }

    #[test]
    fn mmio_bank_select_b0_read_write() {
        let mut pegc = Pegc::new();
        pegc.mmio_write_byte(MMIO1_BANK_B0, 0x05);
        assert_eq!(pegc.mmio_read_byte(MMIO1_BANK_B0), 0x05);
        pegc.mmio_write_byte(MMIO1_BANK_B0, 0xF3);
        assert_eq!(pegc.mmio_read_byte(MMIO1_BANK_B0), 0x03);
    }

    #[test]
    fn mmio_mode_register_packed_vs_plane() {
        let mut pegc = Pegc::new();
        assert!(pegc.is_packed_pixel_mode());
        assert!(!pegc.is_plane_mode());

        pegc.mmio_write_byte(MMIO2_BASE + REG_MODE, 0x01);
        assert!(!pegc.is_packed_pixel_mode());
        assert!(pegc.is_plane_mode());

        pegc.mmio_write_byte(MMIO2_BASE + REG_MODE, 0x00);
        assert!(pegc.is_packed_pixel_mode());
    }

    #[test]
    fn mmio_upper_vram_enable() {
        let mut pegc = Pegc::new();
        assert!(!pegc.is_upper_vram_enabled());

        pegc.mmio_write_byte(MMIO2_BASE + REG_VRAM_ENABLE, 0x01);
        assert!(pegc.is_upper_vram_enabled());
        assert_eq!(pegc.mmio_read_byte(MMIO2_BASE + REG_VRAM_ENABLE), 0x01);

        pegc.mmio_write_byte(MMIO2_BASE + REG_VRAM_ENABLE, 0x00);
        assert!(!pegc.is_upper_vram_enabled());
    }

    #[test]
    fn mmio_plane_access_mask() {
        let mut pegc = Pegc::new();
        pegc.mmio_write_byte(MMIO2_BASE + REG_PLANE_ACCESS, 0xA5);
        assert_eq!(pegc.mmio_read_byte(MMIO2_BASE + REG_PLANE_ACCESS), 0xA5);
        assert_eq!(pegc.state.plane_access_mask, 0xA5);
    }

    #[test]
    fn mmio_rop_register_bit_fields() {
        let mut pegc = Pegc::new();
        pegc.mmio_write_word(MMIO2_BASE + REG_PLANE_ROP_LOW, 0xABCD);
        assert_eq!(pegc.state.rop_register, 0xABCD);

        let rop_code = pegc.state.rop_register & 0xFF;
        assert_eq!(rop_code, 0xCD);

        let source_from_cpu = (pegc.state.rop_register >> 8) & 1;
        assert_eq!(source_from_cpu, 1);

        let shift_direction = (pegc.state.rop_register >> 9) & 1;
        assert_eq!(shift_direction, 1);

        let rop_method = (pegc.state.rop_register >> 10) & 3;
        assert_eq!(rop_method, 2);

        let rop_enabled = (pegc.state.rop_register >> 12) & 1;
        assert_eq!(rop_enabled, 0);

        let pattern_update = (pegc.state.rop_register >> 13) & 1;
        assert_eq!(pattern_update, 1);
    }

    #[test]
    fn mmio_write_mask() {
        let mut pegc = Pegc::new();
        pegc.mmio_write_byte(MMIO2_BASE + REG_MASK_BYTE0, 0x12);
        pegc.mmio_write_byte(MMIO2_BASE + REG_MASK_BYTE1, 0x34);
        pegc.mmio_write_byte(MMIO2_BASE + REG_MASK_BYTE2, 0x56);
        pegc.mmio_write_byte(MMIO2_BASE + REG_MASK_BYTE3, 0x78);
        assert_eq!(pegc.state.write_mask, 0x78563412);
    }

    #[test]
    fn mmio_block_length() {
        let mut pegc = Pegc::new();
        pegc.mmio_write_byte(MMIO2_BASE + REG_LENGTH_LOW, 0xFF);
        pegc.mmio_write_byte(MMIO2_BASE + REG_LENGTH_HIGH, 0xFF);
        assert_eq!(pegc.state.block_length, 0x0FFF);
    }

    #[test]
    fn mmio_shift_registers() {
        let mut pegc = Pegc::new();
        pegc.mmio_write_byte(MMIO2_BASE + REG_SHIFT_READ, 0x1F);
        assert_eq!(pegc.state.shift_read, 0x1F);
        pegc.mmio_write_byte(MMIO2_BASE + REG_SHIFT_READ, 0xFF);
        assert_eq!(pegc.state.shift_read, 0x1F);
        pegc.mmio_write_byte(MMIO2_BASE + REG_SHIFT_WRITE, 0x0A);
        assert_eq!(pegc.state.shift_write, 0x0A);
    }

    #[test]
    fn mmio_palette_colors() {
        let mut pegc = Pegc::new();
        pegc.mmio_write_byte(MMIO2_BASE + REG_PALETTE1, 0x42);
        assert_eq!(pegc.state.palette_color_1, 0x42);
        assert_eq!(pegc.mmio_read_byte(MMIO2_BASE + REG_PALETTE1), 0x42);
        pegc.mmio_write_byte(MMIO2_BASE + REG_PALETTE2, 0xAB);
        assert_eq!(pegc.state.palette_color_2, 0xAB);
        assert_eq!(pegc.mmio_read_byte(MMIO2_BASE + REG_PALETTE2), 0xAB);
    }

    #[test]
    fn mmio_pattern_register_normal_mode_aligned() {
        let mut pegc = Pegc::new();
        for i in (0..0x40u32).step_by(4) {
            let value = (i as u8).wrapping_mul(7);
            pegc.mmio_write_byte(MMIO2_BASE + REG_PATTERN + i, value);
        }
        for i in (0..0x40u32).step_by(4) {
            let expected = (i as u8).wrapping_mul(7);
            assert_eq!(pegc.mmio_read_byte(MMIO2_BASE + REG_PATTERN + i), expected,);
        }
    }

    #[test]
    fn mmio_pattern_register_normal_mode_rejects_unaligned() {
        let mut pegc = Pegc::new();
        pegc.mmio_write_byte(MMIO2_BASE + REG_PATTERN + 1, 0xAA);
        assert_eq!(pegc.mmio_read_byte(MMIO2_BASE + REG_PATTERN + 1), 0x00);
        pegc.mmio_write_byte(MMIO2_BASE + REG_PATTERN + 2, 0xBB);
        assert_eq!(pegc.mmio_read_byte(MMIO2_BASE + REG_PATTERN + 2), 0x00);
    }

    #[test]
    fn mmio_pattern_register_normal_mode_rejects_above_0x40() {
        let mut pegc = Pegc::new();
        pegc.mmio_write_byte(MMIO2_BASE + REG_PATTERN + 0x40, 0xCC);
        assert_eq!(pegc.mmio_read_byte(MMIO2_BASE + REG_PATTERN + 0x40), 0x00);
    }

    #[test]
    fn mmio_pattern_register_transposed_mode_scatter_gather() {
        let mut pegc = Pegc::new();
        pegc.state.rop_register = 0x8000;

        pegc.mmio_write_byte(MMIO2_BASE + REG_PATTERN, 0x42);

        let readback = pegc.mmio_read_byte(MMIO2_BASE + REG_PATTERN);
        assert_eq!(readback, 0x42);
    }

    #[test]
    fn mmio_pattern_register_transposed_mode_all_pixels() {
        let mut pegc = Pegc::new();
        pegc.state.rop_register = 0x8000;

        for pixel in 0..16u32 {
            let color = (pixel as u8).wrapping_mul(17);
            pegc.mmio_write_byte(MMIO2_BASE + REG_PATTERN + pixel * 4, color);
        }
        for pixel in 0..16u32 {
            let expected = (pixel as u8).wrapping_mul(17);
            assert_eq!(
                pegc.mmio_read_byte(MMIO2_BASE + REG_PATTERN + pixel * 4),
                expected,
            );
        }
    }

    #[test]
    fn mmio_pattern_register_transposed_rejects_above_0x80() {
        let mut pegc = Pegc::new();
        pegc.state.rop_register = 0x8000;
        pegc.mmio_write_byte(MMIO2_BASE + REG_PATTERN + 0x80, 0xDD);
        assert_eq!(pegc.mmio_read_byte(MMIO2_BASE + REG_PATTERN + 0x80), 0x00);
    }

    #[test]
    fn mmio_pattern_register_word_normal_mode() {
        let mut pegc = Pegc::new();
        pegc.mmio_write_word(MMIO2_BASE + REG_PATTERN, 0xBEEF);
        assert_eq!(pegc.mmio_read_word(MMIO2_BASE + REG_PATTERN), 0xBEEF);
        assert_eq!(pegc.state.pattern_data[0], 0xEF);
        assert_eq!(pegc.state.pattern_data[1], 0xBE);
    }

    #[test]
    fn mmio_pattern_register_word_transposed_mode() {
        let mut pegc = Pegc::new();
        pegc.state.rop_register = 0x8000;
        pegc.mmio_write_word(MMIO2_BASE + REG_PATTERN, 0x00FF);
        let readback = pegc.mmio_read_word(MMIO2_BASE + REG_PATTERN);
        assert_eq!(readback, 0x00FF);
    }

    #[test]
    fn packed_pixel_bank_read_write() {
        let pegc = Pegc::new();
        let mut vram = vec![0u8; PEGC_VRAM_SIZE];

        pegc.packed_write_byte(0, 0x100, 0x42, &mut vram);
        assert_eq!(pegc.packed_read_byte(0, 0x100, &vram), 0x42);
        assert_eq!(vram[0x100], 0x42);
    }

    #[test]
    fn packed_pixel_bank_switching() {
        let mut pegc = Pegc::new();
        let mut vram = vec![0u8; PEGC_VRAM_SIZE];

        pegc.state.bank_a8 = 0;
        pegc.packed_write_byte(0, 0, 0xAA, &mut vram);

        pegc.state.bank_a8 = 1;
        pegc.packed_write_byte(0, 0, 0xBB, &mut vram);

        pegc.state.bank_a8 = 0;
        assert_eq!(pegc.packed_read_byte(0, 0, &vram), 0xAA);

        pegc.state.bank_a8 = 1;
        assert_eq!(pegc.packed_read_byte(0, 0, &vram), 0xBB);

        assert_eq!(vram[0], 0xAA);
        assert_eq!(vram[BANK_SIZE], 0xBB);
    }

    #[test]
    fn packed_pixel_both_windows_independent() {
        let mut pegc = Pegc::new();
        let mut vram = vec![0u8; PEGC_VRAM_SIZE];

        pegc.state.bank_a8 = 2;
        pegc.state.bank_b0 = 5;

        pegc.packed_write_byte(0, 0, 0x11, &mut vram);
        pegc.packed_write_byte(1, 0, 0x22, &mut vram);

        assert_eq!(vram[2 * BANK_SIZE], 0x11);
        assert_eq!(vram[5 * BANK_SIZE], 0x22);
    }

    #[test]
    fn packed_pixel_cross_bank_visibility() {
        let mut pegc = Pegc::new();
        let mut vram = vec![0u8; PEGC_VRAM_SIZE];

        pegc.state.bank_a8 = 3;
        pegc.packed_write_byte(0, 42, 0xCC, &mut vram);

        pegc.state.bank_b0 = 3;
        assert_eq!(pegc.packed_read_byte(1, 42, &vram), 0xCC);
    }

    #[test]
    fn palette_256_all_entries_independent() {
        let mut pegc = Pegc::new();

        for i in 0..=255u8 {
            pegc.write_palette_index(i);
            pegc.write_palette_component(0, i);
            pegc.write_palette_component(1, i.wrapping_mul(2));
            pegc.write_palette_component(2, i.wrapping_mul(3));
        }

        for i in 0..=255u8 {
            pegc.write_palette_index(i);
            assert_eq!(pegc.read_palette_component(0), i);
            assert_eq!(pegc.read_palette_component(1), i.wrapping_mul(2));
            assert_eq!(pegc.read_palette_component(2), i.wrapping_mul(3));
        }
    }

    #[test]
    fn palette_256_8bit_components() {
        let mut pegc = Pegc::new();
        pegc.write_palette_index(100);
        pegc.write_palette_component(0, 0xFF);
        pegc.write_palette_component(1, 0x80);
        pegc.write_palette_component(2, 0x00);
        assert_eq!(pegc.read_palette_component(0), 0xFF);
        assert_eq!(pegc.read_palette_component(1), 0x80);
        assert_eq!(pegc.read_palette_component(2), 0x00);
    }

    #[test]
    fn palette_256_index_full_range() {
        let mut pegc = Pegc::new();
        pegc.write_palette_index(255);
        pegc.write_palette_component(0, 0x42);
        assert_eq!(pegc.state.palette_index, 255);
        assert_eq!(pegc.state.palette_256[255][0], 0x42);
    }

    #[test]
    fn mmio_control_write_resets_block_transfer() {
        let mut pegc = Pegc::new();
        pegc.state.block_length = 99;
        pegc.state.remain = 42;
        pegc.state.last_data_length = 16;

        pegc.mmio_write_byte(MMIO2_BASE + REG_PLANE_ACCESS, 0xFF);
        assert_eq!(pegc.state.remain, 100);
        assert_eq!(pegc.state.last_data_length, 0);
    }

    #[test]
    fn plane_write_resets_state_when_remain_zero() {
        let mut pegc = Pegc::new();
        pegc.state.mode_register = 1;
        pegc.state.plane_access_mask = 0x00;
        pegc.state.rop_register = 0x0100;
        pegc.state.write_mask = 0xFFFF;
        pegc.state.block_length = 31;
        pegc.state.remain = 0;
        pegc.state.last_data_length = 16;

        let mut vram = vec![0u8; PEGC_VRAM_SIZE];
        pegc.plane_write_word(0, 0xFFFF, &mut vram);

        assert_eq!(pegc.state.last_data_length, 0);
        assert!(
            pegc.state.remain <= 32,
            "remain should have been re-initialized from block_length + 1"
        );
    }

    #[test]
    fn mmio_pattern_write_does_not_reset_block_transfer() {
        let mut pegc = Pegc::new();
        pegc.state.block_length = 99;
        pegc.state.remain = 42;
        pegc.state.last_data_length = 16;

        pegc.mmio_write_byte(MMIO2_BASE + REG_PATTERN, 0xFF);
        assert_eq!(pegc.state.remain, 42);
        assert_eq!(pegc.state.last_data_length, 16);
    }

    #[test]
    fn plane_mode_word_read_compare() {
        let mut pegc = Pegc::new();
        pegc.state.mode_register = 1;
        pegc.state.palette_color_1 = 0x42;
        pegc.state.plane_access_mask = 0x00;
        pegc.state.rop_register = 0;

        let mut vram = vec![0u8; PEGC_VRAM_SIZE];
        let base = 0u32;
        for i in 0..16u32 {
            vram[i as usize] = if i % 2 == 0 { 0x42 } else { 0x00 };
        }

        let result = pegc.plane_read_word(base / 8, &vram);
        for i in 0..16 {
            let bit_set = result & (1 << i) != 0;
            if i % 2 == 0 {
                assert!(!bit_set, "pixel {i} matches palette1, bit should be clear");
            } else {
                assert!(
                    bit_set,
                    "pixel {i} differs from palette1, bit should be set"
                );
            }
        }
    }

    #[test]
    fn plane_mode_shifted_vram_source_write_consumes_full_words() {
        let mut pegc = Pegc::new();
        pegc.state.mode_register = 1;
        pegc.state.plane_access_mask = 0x00;
        pegc.state.rop_register = 0x10F0;
        pegc.state.write_mask = 0xFFFF;
        pegc.state.block_length = 31;
        pegc.state.remain = 32;
        pegc.state.shift_read = 0;
        pegc.state.shift_write = 5;

        let mut vram = vec![0u8; PEGC_VRAM_SIZE];
        let source_offset = 0x1000u32;
        let source_pixel_base = source_offset * 8;
        for i in 0..48u32 {
            vram[(source_pixel_base + i) as usize] = (i + 1) as u8;
        }

        pegc.plane_read_word(source_offset, &vram);
        pegc.plane_read_word(source_offset + 2, &vram);
        pegc.plane_read_word(source_offset + 4, &vram);

        let destination_offset = 0x2000u32;
        let destination_pixel_base = destination_offset * 8;
        pegc.plane_write_word(destination_offset, 0, &mut vram);
        pegc.plane_write_word(destination_offset + 2, 0, &mut vram);
        pegc.plane_write_word(destination_offset + 4, 0, &mut vram);

        for i in 0..11u32 {
            assert_eq!(
                vram[(destination_pixel_base + 5 + i) as usize],
                (i + 1) as u8,
                "first shifted write copied source pixel {i} incorrectly"
            );
        }
        for i in 0..16u32 {
            assert_eq!(
                vram[(destination_pixel_base + 16 + i) as usize],
                (17 + i) as u8,
                "second shifted write did not advance by a full source word"
            );
        }
        for i in 0..5u32 {
            assert_eq!(
                vram[(destination_pixel_base + 32 + i) as usize],
                (33 + i) as u8,
                "final shifted write did not continue from the next source word"
            );
        }
        assert_eq!(vram[(destination_pixel_base + 37) as usize], 0x00);
        assert_eq!(pegc.state.remain, 0);
    }

    #[test]
    fn plane_mode_word_write_simple_no_rop() {
        let mut pegc = Pegc::new();
        pegc.state.mode_register = 1;
        pegc.state.plane_access_mask = 0x00;
        pegc.state.rop_register = 0x0100;
        pegc.state.write_mask = 0xFFFF;
        pegc.state.block_length = 0x0FFF;

        let mut vram = vec![0u8; PEGC_VRAM_SIZE];
        for byte in vram.iter_mut().take(16) {
            *byte = 0xAA;
        }

        pegc.plane_write_word(0, 0xFF00, &mut vram);

        for i in 0..8 {
            let expected_source: u8 = if 0xFF00u16 & pixel_mask_position(i) as u16 != 0 {
                0xFF
            } else {
                0x00
            };
            let expected = apply_source_copy(expected_source, 0xAA, 0x00);
            assert_eq!(
                vram[i as usize], expected,
                "pixel {i}: expected {expected:#04X}"
            );
        }
    }

    #[test]
    fn plane_mode_write_mask_inhibits() {
        let mut pegc = Pegc::new();
        pegc.state.mode_register = 1;
        pegc.state.plane_access_mask = 0x00;
        pegc.state.rop_register = 0x0100;
        pegc.state.write_mask = 0x0000;
        pegc.state.block_length = 0x0FFF;

        let mut vram = vec![0xBB; PEGC_VRAM_SIZE];

        pegc.plane_write_word(0, 0xFFFF, &mut vram);

        for (i, byte) in vram.iter().enumerate().take(16) {
            assert_eq!(*byte, 0xBB, "pixel {i} should be unchanged (mask = 0)");
        }
    }

    #[test]
    fn plane_mode_rop_source_copy() {
        let mut pegc = Pegc::new();
        pegc.state.mode_register = 1;
        pegc.state.plane_access_mask = 0x00;
        pegc.state.rop_register = 0x10F0 | (1 << 8);
        pegc.state.write_mask = 0xFFFF;
        pegc.state.block_length = 0x0FFF;
        pegc.state.palette_color_1 = 0xFF;

        let mut vram = vec![0u8; PEGC_VRAM_SIZE];

        pegc.plane_write_word(0, 0xFFFF, &mut vram);

        for (i, byte) in vram.iter().enumerate().take(16) {
            assert_eq!(
                *byte, 0xFF,
                "pixel {i} should be 0xFF after ROP 0xF0 with all-ones source"
            );
        }
    }

    #[test]
    fn plane_mode_block_length_countdown() {
        let mut pegc = Pegc::new();
        pegc.state.mode_register = 1;
        pegc.state.plane_access_mask = 0x00;
        pegc.state.rop_register = 0x0100;
        pegc.state.write_mask = 0xFFFF;
        pegc.state.block_length = 7;

        let mut vram = vec![0u8; PEGC_VRAM_SIZE];

        pegc.plane_write_word(0, 0xFFFF, &mut vram);

        assert_eq!(pegc.state.remain, 0);

        for (i, byte) in vram.iter().enumerate().take(8) {
            assert_ne!(*byte, 0, "pixel {i} within block should be written");
        }
    }

    #[test]
    fn packed_pixel_word_operations() {
        let mut pegc = Pegc::new();
        let mut vram = vec![0u8; PEGC_VRAM_SIZE];

        pegc.state.bank_a8 = 0;
        pegc.packed_write_word(0, 0, 0xBEEF, &mut vram);
        assert_eq!(pegc.packed_read_word(0, 0, &vram), 0xBEEF);
        assert_eq!(vram[0], 0xEF);
        assert_eq!(vram[1], 0xBE);
    }

    #[test]
    fn apply_rop_copy_source() {
        let result = apply_rop(0xF0, 0xFF, 0x00, 0xFF, 0xFF, 0x00);
        assert_eq!(result, 0xFF);
        let result = apply_rop(0xF0, 0x00, 0xFF, 0xFF, 0xFF, 0x00);
        assert_eq!(result, 0x00);
    }

    #[test]
    fn apply_rop_copy_destination() {
        let result = apply_rop(0xCC, 0x00, 0xFF, 0x00, 0x00, 0x00);
        assert_eq!(result, 0xFF);
    }

    #[test]
    fn apply_rop_with_plane_mask() {
        let result = apply_rop(0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F);
        assert_eq!(result, 0xF0);
    }

    #[test]
    fn apply_source_copy_basic() {
        assert_eq!(apply_source_copy(0xFF, 0x00, 0x00), 0xFF);
        assert_eq!(apply_source_copy(0x00, 0xFF, 0x00), 0x00);
        assert_eq!(apply_source_copy(0xFF, 0xAA, 0xF0), 0xAF);
    }

    #[test]
    fn pixel_mask_position_layout() {
        assert_eq!(pixel_mask_position(0), 0x80);
        assert_eq!(pixel_mask_position(1), 0x40);
        assert_eq!(pixel_mask_position(7), 0x01);
        assert_eq!(pixel_mask_position(8), 0x8000);
        assert_eq!(pixel_mask_position(15), 0x0100);
        // The same formula maps pixels 16..31 to bits 16..31 of the mask.
        assert_eq!(pixel_mask_position(16), 0x0080_0000);
        assert_eq!(pixel_mask_position(23), 0x0001_0000);
        assert_eq!(pixel_mask_position(24), 0x8000_0000);
        assert_eq!(pixel_mask_position(31), 0x0100_0000);
    }

    #[test]
    fn plane_dword_processes_32_pixels() {
        let mut pegc = Pegc::new();
        pegc.state.mode_register = 1;
        pegc.state.plane_access_mask = 0x00;
        pegc.state.rop_register = 0x0100; // source = CPU data
        pegc.state.write_mask = 0xFFFF_FFFF;
        pegc.state.block_length = 0x0FFF;

        let mut vram = vec![0u8; PEGC_VRAM_SIZE];
        // Source value with alternating bits across all 32 pixel slots.
        pegc.plane_write_dword(0, 0xAAAA_AAAA, &mut vram);

        // 32 pixels written. Pixel 0 maps to bit 7 of the low byte (0xAA bit 7 = 1)
        // so VRAM[0] should be 0xFF. Pixel 1 maps to bit 6 (0xAA bit 6 = 0), so 0x00.
        // The full pattern alternates 0xFF / 0x00 / ... across 32 bytes.
        for (i, byte) in vram.iter().enumerate().take(32) {
            let expected = if i % 2 == 0 { 0xFF } else { 0x00 };
            assert_eq!(*byte, expected, "pixel {i}: expected {expected:#04X}");
        }
    }

    #[test]
    fn plane_dword_shift_write_bit_4_valid() {
        let mut pegc = Pegc::new();
        pegc.state.mode_register = 1;
        pegc.state.plane_access_mask = 0x00;
        pegc.state.rop_register = 0x0100; // source = CPU data
        pegc.state.write_mask = 0xFFFF_FFFF;
        pegc.state.block_length = 0x0FFF;
        pegc.state.shift_write = 0x10; // shift by 16 pixels (bit 4 set)

        let mut vram = vec![0u8; PEGC_VRAM_SIZE];
        // Source = all-ones: every pixel from byte 16 onwards must be 0xFF.
        pegc.plane_write_dword(0, 0xFFFF_FFFF, &mut vram);
        for (i, byte) in vram.iter().enumerate().take(16) {
            assert_eq!(*byte, 0x00, "pixel {i} before shift should be untouched");
        }
        for (i, byte) in vram.iter().enumerate().take(32).skip(16) {
            assert_eq!(*byte, 0xFF, "pixel {i} after shift should be 0xFF");
        }
    }

    #[test]
    fn pattern_register_dword_transposed_32_pixels() {
        let mut pegc = Pegc::new();
        pegc.state.rop_register = 0x8000; // transposed mode
        for pixel in 0..32u32 {
            let color = (pixel as u8).wrapping_mul(7);
            pegc.mmio_write_byte(MMIO2_BASE + REG_PATTERN + pixel * 4, color);
        }
        for pixel in 0..32u32 {
            let expected = (pixel as u8).wrapping_mul(7);
            assert_eq!(
                pegc.mmio_read_byte(MMIO2_BASE + REG_PATTERN + pixel * 4),
                expected,
                "pixel {pixel} colour roundtrip",
            );
        }
    }

    #[test]
    fn pattern_register_dword_normal_per_plane_32_bits() {
        let mut pegc = Pegc::new();
        // Normal mode (rop bit 15 = 0). Write 32-bit pattern words per plane.
        for plane in 0..8u32 {
            let value = 0xDEAD_BE00u32 | plane;
            pegc.mmio_write_dword(MMIO2_BASE + REG_PATTERN + plane * 4, value);
        }
        for plane in 0..8u32 {
            let expected = 0xDEAD_BE00u32 | plane;
            assert_eq!(
                pegc.mmio_read_dword(MMIO2_BASE + REG_PATTERN + plane * 4),
                expected,
                "plane {plane} 32-bit pattern roundtrip",
            );
        }
    }
}
