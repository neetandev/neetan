//! PC-88VA2 kanji / character-generator ROM access window.
//!
//! Software programs a 16-bit hardware character code (0x14C low byte, 0x14D
//! high byte) and a raster row / font-half selector (0x14F), then reads glyph
//! bytes through 0x14E. Most codes map into the 320 KiB font ROM; the user-glyph
//! JIS regions (76xx/77xx) live in battery-backed memory and are writable.

use super::Pc88VaBus;

/// Where a glyph byte lives.
enum CgSource {
    /// Offset into the font ROM.
    Font(usize),
    /// Offset into battery-backed memory.
    Backup(usize),
    /// The "tofu" fallback glyph (all 0xFF).
    Tofu,
}

/// CGROM access-window registers.
#[derive(Default)]
pub(crate) struct CgromVa {
    /// Hardware character code (ports 0x14C/0x14D).
    cgaddr: u16,
    /// Raster row (bits 0-3) and font-half selector (bit 5), port 0x14F.
    cgrow: u8,
}

impl CgromVa {
    /// Writes the character-code low byte (port 0x14C).
    pub(crate) fn write_addr_low(&mut self, value: u8) {
        self.cgaddr = (self.cgaddr & 0xFF00) | u16::from(value);
    }

    /// Writes the character-code high byte (port 0x14D, masked to 7 bits).
    pub(crate) fn write_addr_high(&mut self, value: u8) {
        self.cgaddr = (self.cgaddr & 0x00FF) | (u16::from(value & 0x7F) << 8);
    }

    /// Writes the raster-row / font-half register (port 0x14F).
    pub(crate) fn write_row(&mut self, value: u8) {
        self.cgrow = value;
    }

    /// The effective hardware character code, with bit 15 (left/right half) taken
    /// from the row register's bit 5.
    fn hccode(&self) -> u16 {
        (self.cgaddr & 0x7FFF)
            | if self.cgrow & 0x20 != 0 {
                0x0000
            } else {
                0x8000
            }
    }

    /// Glyph width in bytes per raster (1 for ANK, 2 for full-width).
    fn width(hccode: u16) -> usize {
        if hccode & 0x7F00 == 0 { 1 } else { 2 }
    }

    /// Resolves the base location of a glyph for `hccode`.
    fn location(hccode: u16, txtmode8: bool) -> CgSource {
        let lr = usize::from(hccode >> 15);
        let jis1 = usize::from(hccode & 0x7F) + 0x20;
        let jis2 = usize::from((hccode >> 8) & 0x7F);
        let j60 = jis2 & 0x60;
        let j1f = jis2 & 0x1F;

        if jis2 == 0 && lr == 0 {
            if txtmode8 {
                CgSource::Font(0x41000 + (usize::from(hccode & 0xFF) << 3))
            } else {
                CgSource::Font(0x40000 + (usize::from(hccode & 0xFF) << 4))
            }
        } else if jis1 < 0x28 {
            CgSource::Font(lr + (j60 << 8) + ((jis1 & 0x07) << 10) + (j1f << 5))
        } else if jis1 < 0x30 {
            CgSource::Font(lr + 0x40000 + (j60 << 8) + ((jis1 & 0x07) << 10) + (j1f << 5))
        } else if jis1 < 0x40 {
            CgSource::Font(lr + (j60 << 10) + ((jis1 & 0x0F) << 10) + (j1f << 5))
        } else if jis1 < 0x50 {
            CgSource::Font(lr + 0x4000 + (j60 << 10) + ((jis1 & 0x0F) << 10) + (j1f << 5))
        } else if jis1 < 0x60 {
            CgSource::Font(lr + 0x20000 + (j60 << 10) + ((jis1 & 0x0F) << 10) + (j1f << 5))
        } else if jis1 < 0x70 {
            CgSource::Font(lr + 0x24000 + (j60 << 10) + ((jis1 & 0x0F) << 10) + (j1f << 5))
        } else if jis1 < 0x76 {
            CgSource::Font(lr + 0x20000 + (j60 << 8) + ((jis1 & 0x07) << 10) + (j1f << 5))
        } else if jis1 < 0x78 {
            if jis1 == 0x77 && (jis2 == 0x7E || jis2 == 0x7F) {
                CgSource::Tofu
            } else {
                CgSource::Backup(lr + (j60 << 6) + ((jis1 & 0x01) << 10) + (j1f << 5))
            }
        } else {
            CgSource::Font(0)
        }
    }

    /// Whether the current code addresses a writable user-glyph region.
    fn writable(hccode: u16) -> bool {
        let lr = hccode >> 15;
        let jis1 = usize::from(hccode & 0x7F) + 0x20;
        let jis2 = usize::from((hccode >> 8) & 0x7F);
        if jis2 == 0 && lr == 0 {
            return false;
        }
        match jis1 {
            0x76 => true,
            0x77 => jis2 != 0x7E && jis2 != 0x7F,
            _ => false,
        }
    }

    /// Reads the glyph byte at the current code and row (port 0x14E).
    pub(crate) fn read_data(&self, font_rom: &[u8], backup_ram: &[u8], txtmode8: bool) -> u8 {
        let hccode = self.hccode();
        let row = if hccode < 0x100 && txtmode8 {
            usize::from(self.cgrow & 0x07)
        } else {
            usize::from(self.cgrow & 0x0F)
        };
        let row_offset = Self::width(hccode) * row;
        match Self::location(hccode, txtmode8) {
            CgSource::Font(base) => font_rom.get(base + row_offset).copied().unwrap_or(0xFF),
            CgSource::Backup(base) => backup_ram.get(base + row_offset).copied().unwrap_or(0xFF),
            CgSource::Tofu => 0xFF,
        }
    }

    /// Writes a glyph byte (port 0x14E), honored only for the writable user-glyph
    /// regions in battery-backed memory.
    pub(crate) fn write_data(&self, value: u8, backup_ram: &mut [u8], txtmode8: bool) {
        let hccode = self.hccode();
        if !Self::writable(hccode) {
            return;
        }
        let row_offset = Self::width(hccode) * usize::from(self.cgrow & 0x0F);
        if let CgSource::Backup(base) = Self::location(hccode, txtmode8)
            && let Some(slot) = backup_ram.get_mut(base + row_offset)
        {
            *slot = value;
        }
    }
}

impl<T: common::TraceSink> Pc88VaBus<T> {
    /// The 8-dot ANK font select (text-mode register 0x148 bit 2).
    fn cgrom_font8(&self) -> bool {
        self.video.txtmode & 0x04 != 0
    }

    /// Reads the CGROM data window (port 0x14E).
    pub(crate) fn read_cgrom_data(&self) -> u8 {
        self.cgrom.read_data(
            self.memory.font_rom(),
            self.memory.backup_ram(),
            self.cgrom_font8(),
        )
    }

    /// Writes the CGROM data window (port 0x14E).
    pub(crate) fn write_cgrom_data(&mut self, value: u8) {
        let font8 = self.cgrom_font8();
        self.cgrom
            .write_data(value, self.memory.backup_ram_mut(), font8);
    }
}
