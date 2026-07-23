//! INT 10h graphics codecs: pixel read/write, glyph rendering and window
//! scroll for the CGA interleaved, EGA/VGA planar and packed 8-bit modes.
//!
//! Planar operations program the graphics controller write modes through
//! the I/O ports and restore the mode register file values afterwards, so
//! the adapter state stays exactly what the mode set left behind.

use common::{Cpu, TraceSink};

use super::{
    super::AtBus,
    video::BDA_ACTIVE_PAGE,
    video_modes::{ModeFamily, VideoModeEntry},
};

/// VGA graphics controller index port.
const GC_INDEX_PORT: u16 = 0x03CE;
/// VGA graphics controller data port.
const GC_DATA_PORT: u16 = 0x03CF;
/// Graphics controller function select value for XOR drawing.
const GC_FUNCTION_XOR: u8 = 0x18;

impl<T: TraceSink> AtBus<T> {
    /// Writes one graphics controller register.
    fn gc_register_write(&mut self, index: u8, value: u8) {
        self.io_write(GC_INDEX_PORT, index);
        self.io_write(GC_DATA_PORT, value);
    }

    /// AH=0Ch: writes pixel AL at CX/DX on page BH.
    pub(super) fn int10h_write_pixel(&mut self, cpu: &mut impl Cpu) {
        let Some(entry) = self.active_mode_entry() else {
            return;
        };
        if entry.family == ModeFamily::Text {
            return;
        }
        self.graphics_pixel_write(entry, cpu.bh(), cpu.cx(), cpu.dx(), cpu.al());
    }

    /// AH=0Dh: reads the pixel at CX/DX on page BH into AL.
    pub(super) fn int10h_read_pixel(&mut self, cpu: &mut impl Cpu) {
        let Some(entry) = self.active_mode_entry() else {
            return;
        };
        if entry.family == ModeFamily::Text {
            return;
        }
        let color = self.graphics_pixel_read(entry, cpu.bh(), cpu.cx(), cpu.dx());
        cpu.set_al(color);
    }

    /// Byte address of a CGA interleaved scanline.
    fn cga_row_address(entry: &'static VideoModeEntry, y: u16) -> u32 {
        entry.regen_base + u32::from(y >> 1) * 80 + 0x2000 * u32::from(y & 1)
    }

    /// Per-plane byte offset of a planar mode page.
    fn planar_page_offset(entry: &'static VideoModeEntry, page: u8) -> u32 {
        u32::from(page & 0x07) * u32::from(entry.page_size)
    }

    /// Writes one pixel through the family codec. Bit 7 of the color XORs
    /// the pixel into the frame.
    pub(super) fn graphics_pixel_write(
        &mut self,
        entry: &'static VideoModeEntry,
        page: u8,
        x: u16,
        y: u16,
        color: u8,
    ) {
        if x >= entry.width || y >= entry.height {
            return;
        }
        let xor = color & 0x80 != 0;
        match entry.family {
            ModeFamily::Text => {}
            ModeFamily::Cga2 => {
                let address = Self::cga_row_address(entry, y) + u32::from(x / 8);
                let shift = 7 - (x & 7) as u8;
                let mask = 1u8 << shift;
                let bits = (color & 0x01) << shift;
                let byte = self.read_mem_byte(address);
                let byte = if xor {
                    byte ^ bits
                } else {
                    (byte & !mask) | bits
                };
                self.write_mem_byte(address, byte);
            }
            ModeFamily::Cga4 => {
                let address = Self::cga_row_address(entry, y) + u32::from(x / 4);
                let shift = 6 - 2 * (x & 3) as u8;
                let mask = 3u8 << shift;
                let bits = (color & 0x03) << shift;
                let byte = self.read_mem_byte(address);
                let byte = if xor {
                    byte ^ bits
                } else {
                    (byte & !mask) | bits
                };
                self.write_mem_byte(address, byte);
            }
            ModeFamily::Planar => {
                let bytes_per_row = u32::from(entry.width / 8);
                let address = entry.regen_base
                    + Self::planar_page_offset(entry, page)
                    + u32::from(y) * bytes_per_row
                    + u32::from(x / 8);
                self.gc_register_write(0x05, 0x02);
                self.gc_register_write(0x08, 0x80 >> (x & 7));
                if xor {
                    self.gc_register_write(0x03, GC_FUNCTION_XOR);
                }
                let _ = self.read_mem_byte(address);
                self.write_mem_byte(address, color & 0x0F);
                self.restore_vram_access_registers(entry.registers);
            }
            ModeFamily::Packed => {
                let offset = u32::from(y) * u32::from(entry.width) + u32::from(x);
                let byte = if xor {
                    self.packed_byte_read(entry, offset) ^ color
                } else {
                    color
                };
                self.packed_byte_write(entry, offset, byte);
            }
        }
    }

    /// Reads one pixel through the family codec.
    pub(super) fn graphics_pixel_read(
        &mut self,
        entry: &'static VideoModeEntry,
        page: u8,
        x: u16,
        y: u16,
    ) -> u8 {
        if x >= entry.width || y >= entry.height {
            return 0;
        }
        match entry.family {
            ModeFamily::Text => 0,
            ModeFamily::Cga2 => {
                let address = Self::cga_row_address(entry, y) + u32::from(x / 8);
                let shift = 7 - (x & 7) as u8;
                (self.read_mem_byte(address) >> shift) & 0x01
            }
            ModeFamily::Cga4 => {
                let address = Self::cga_row_address(entry, y) + u32::from(x / 4);
                let shift = 6 - 2 * (x & 3) as u8;
                (self.read_mem_byte(address) >> shift) & 0x03
            }
            ModeFamily::Planar => {
                let bytes_per_row = u32::from(entry.width / 8);
                let address = entry.regen_base
                    + Self::planar_page_offset(entry, page)
                    + u32::from(y) * bytes_per_row
                    + u32::from(x / 8);
                let shift = 7 - (x & 7) as u8;
                let mut color = 0u8;
                for plane in 0..4u8 {
                    self.gc_register_write(0x04, plane);
                    let bit = (self.read_mem_byte(address) >> shift) & 0x01;
                    color |= bit << plane;
                }
                self.restore_vram_access_registers(entry.registers);
                color
            }
            ModeFamily::Packed => {
                let offset = u32::from(y) * u32::from(entry.width) + u32::from(x);
                self.packed_byte_read(entry, offset)
            }
        }
    }

    /// Whether a packed mode's frame crosses the 64 KiB window (the Tseng
    /// SVGA modes, which need bank switching).
    fn packed_is_banked(entry: &'static VideoModeEntry) -> bool {
        u32::from(entry.width) * u32::from(entry.height) > 0x1_0000
    }

    /// Reads a byte of a packed mode frame, switching the ET4000 bank when
    /// the frame crosses the 64 KiB window.
    fn packed_byte_read(&mut self, entry: &'static VideoModeEntry, offset: u32) -> u8 {
        if Self::packed_is_banked(entry) {
            self.select_svga_bank((offset >> 16) as u8);
            let byte = self.read_mem_byte(entry.regen_base + (offset & 0xFFFF));
            self.select_svga_bank(0);
            byte
        } else {
            self.read_mem_byte(entry.regen_base + offset)
        }
    }

    /// Writes a byte of a packed mode frame, switching the ET4000 bank when
    /// the frame crosses the 64 KiB window.
    fn packed_byte_write(&mut self, entry: &'static VideoModeEntry, offset: u32, value: u8) {
        if Self::packed_is_banked(entry) {
            self.select_svga_bank((offset >> 16) as u8);
            self.write_mem_byte(entry.regen_base + (offset & 0xFFFF), value);
            self.select_svga_bank(0);
        } else {
            self.write_mem_byte(entry.regen_base + offset, value);
        }
    }

    /// Reads the glyph bitmap rows of a character from the font the IVT
    /// points at (INT 43h; INT 1Fh for the CGA upper half).
    fn glyph_bitmap(&mut self, entry: &'static VideoModeEntry, code: u8) -> Vec<u8> {
        let cga = matches!(entry.family, ModeFamily::Cga2 | ModeFamily::Cga4);
        let (vector, glyph_code) = if cga && code >= 0x80 {
            (0x1Fu32, u32::from(code - 0x80))
        } else {
            (0x43u32, u32::from(code))
        };
        let pointer = self.read_mem_dword(vector * 4);
        let height = u32::from(entry.char_height);
        let base = ((pointer >> 16) << 4)
            .wrapping_add(pointer & 0xFFFF)
            .wrapping_add(glyph_code * height);
        let mut rows = vec![0u8; height as usize];
        for (index, row) in rows.iter_mut().enumerate() {
            *row = self.read_mem_byte(base.wrapping_add(index as u32));
        }
        rows
    }

    /// Renders a glyph into a graphics mode text cell. Bit 7 of the color
    /// XORs the glyph into the frame, otherwise the background is cleared.
    pub(super) fn graphics_glyph_write(
        &mut self,
        entry: &'static VideoModeEntry,
        page: u8,
        row: u8,
        column: u8,
        code: u8,
        color: u8,
    ) {
        let rows = self.glyph_bitmap(entry, code);
        let xor = color & 0x80 != 0;
        let foreground = color & 0x7F;
        let height = entry.char_height;

        match entry.family {
            ModeFamily::Text => {}
            ModeFamily::Cga2 => {
                for (index, &bits) in rows.iter().enumerate() {
                    let y = u16::from(row) * height + index as u16;
                    let address = Self::cga_row_address(entry, y) + u32::from(column);
                    let byte = if foreground & 0x01 != 0 { bits } else { 0x00 };
                    let byte = if xor {
                        self.read_mem_byte(address) ^ byte
                    } else {
                        byte
                    };
                    self.write_mem_byte(address, byte);
                }
            }
            ModeFamily::Cga4 => {
                for (index, &bits) in rows.iter().enumerate() {
                    let y = u16::from(row) * height + index as u16;
                    let address = Self::cga_row_address(entry, y) + u32::from(column) * 2;
                    let mut expanded = [0u8; 2];
                    for pixel in 0..8u8 {
                        if bits & (0x80 >> pixel) != 0 {
                            let shift = 6 - 2 * (pixel & 3);
                            expanded[(pixel / 4) as usize] |= (foreground & 0x03) << shift;
                        }
                    }
                    for (half, &byte) in expanded.iter().enumerate() {
                        let address = address + half as u32;
                        let byte = if xor {
                            self.read_mem_byte(address) ^ byte
                        } else {
                            byte
                        };
                        self.write_mem_byte(address, byte);
                    }
                }
            }
            ModeFamily::Planar => {
                let bytes_per_row = u32::from(entry.width / 8);
                let base = entry.regen_base
                    + Self::planar_page_offset(entry, page)
                    + u32::from(row) * u32::from(height) * bytes_per_row
                    + u32::from(column);
                self.gc_register_write(0x05, 0x02);
                if xor {
                    self.gc_register_write(0x03, GC_FUNCTION_XOR);
                }
                for (index, &bits) in rows.iter().enumerate() {
                    let address = base + index as u32 * bytes_per_row;
                    self.gc_register_write(0x08, bits);
                    let _ = self.read_mem_byte(address);
                    self.write_mem_byte(address, foreground & 0x0F);
                    if !xor {
                        self.gc_register_write(0x08, !bits);
                        let _ = self.read_mem_byte(address);
                        self.write_mem_byte(address, 0x00);
                    }
                }
                self.restore_vram_access_registers(entry.registers);
            }
            ModeFamily::Packed => {
                for (index, &bits) in rows.iter().enumerate() {
                    let y = u32::from(row) * u32::from(height) + index as u32;
                    let offset = y * u32::from(entry.width) + u32::from(column) * 8;
                    for pixel in 0..8u8 {
                        let set = bits & (0x80 >> pixel) != 0;
                        let offset = offset + u32::from(pixel);
                        let byte = if xor {
                            let current = self.packed_byte_read(entry, offset);
                            if set { current ^ foreground } else { current }
                        } else if set {
                            foreground
                        } else {
                            0x00
                        };
                        self.packed_byte_write(entry, offset, byte);
                    }
                }
            }
        }
    }

    /// Recovers the character code of a graphics mode text cell by matching
    /// the cell's monochrome pattern against the current font. Returns zero
    /// when no glyph matches.
    pub(super) fn graphics_glyph_read(
        &mut self,
        entry: &'static VideoModeEntry,
        page: u8,
        row: u8,
        column: u8,
    ) -> u8 {
        let height = entry.char_height;
        let mut pattern = vec![0u8; usize::from(height)];
        for (index, bits) in pattern.iter_mut().enumerate() {
            let y = u16::from(row) * height + index as u16;
            for pixel in 0..8u16 {
                let x = u16::from(column) * 8 + pixel;
                if self.graphics_pixel_read(entry, page, x, y) != 0 {
                    *bits |= 0x80 >> pixel;
                }
            }
        }
        for code in 0..=255u8 {
            if self.glyph_bitmap(entry, code) == pattern {
                return code;
            }
        }
        0
    }

    /// Scrolls a graphics mode window by whole text rows, blanking the freed
    /// rows with the fill color.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn graphics_scroll(
        &mut self,
        entry: &'static VideoModeEntry,
        top: u8,
        left: u8,
        bottom: u8,
        right: u8,
        lines: u8,
        fill: u8,
        up: bool,
    ) {
        let page = self.read_mem_byte(BDA_ACTIVE_PAGE);
        let height = entry.char_height;
        let window_top = u16::from(top) * height;
        let window_bottom = (u16::from(bottom) + 1) * height - 1;
        let shift = u16::from(lines) * height;
        let move_scanlines = (window_bottom - window_top + 1) - shift;

        match entry.family {
            ModeFamily::Text => {}
            ModeFamily::Cga2 | ModeFamily::Cga4 => {
                let bytes_per_cell: u32 = if entry.family == ModeFamily::Cga2 {
                    1
                } else {
                    2
                };
                let first = u32::from(left) * bytes_per_cell;
                let last = (u32::from(right) + 1) * bytes_per_cell - 1;
                for step in 0..move_scanlines {
                    let (destination_y, source_y) = if up {
                        (window_top + step, window_top + shift + step)
                    } else {
                        (window_bottom - step, window_bottom - shift - step)
                    };
                    for byte_x in first..=last {
                        let source = Self::cga_row_address(entry, source_y) + byte_x;
                        let byte = self.read_mem_byte(source);
                        let destination = Self::cga_row_address(entry, destination_y) + byte_x;
                        self.write_mem_byte(destination, byte);
                    }
                }
                let fill_byte = if entry.family == ModeFamily::Cga2 {
                    if fill & 0x01 != 0 { 0xFF } else { 0x00 }
                } else {
                    let color = fill & 0x03;
                    color << 6 | color << 4 | color << 2 | color
                };
                for step in 0..shift {
                    let y = if up {
                        window_bottom - step
                    } else {
                        window_top + step
                    };
                    for byte_x in first..=last {
                        let address = Self::cga_row_address(entry, y) + byte_x;
                        self.write_mem_byte(address, fill_byte);
                    }
                }
            }
            ModeFamily::Planar => {
                let bytes_per_row = u32::from(entry.width / 8);
                let base = entry.regen_base + Self::planar_page_offset(entry, page);
                // Write mode 1 copies all four planes through the latches.
                self.gc_register_write(0x05, 0x01);
                for step in 0..u32::from(move_scanlines) {
                    let (destination_y, source_y) = if up {
                        (
                            u32::from(window_top) + step,
                            u32::from(window_top + shift) + step,
                        )
                    } else {
                        (
                            u32::from(window_bottom) - step,
                            u32::from(window_bottom - shift) - step,
                        )
                    };
                    for byte_x in u32::from(left)..=u32::from(right) {
                        let source = base + source_y * bytes_per_row + byte_x;
                        let _ = self.read_mem_byte(source);
                        let destination = base + destination_y * bytes_per_row + byte_x;
                        self.write_mem_byte(destination, 0x00);
                    }
                }
                // Write mode 2 blanks the freed scanlines with the fill color.
                self.gc_register_write(0x05, 0x02);
                self.gc_register_write(0x08, 0xFF);
                for step in 0..u32::from(shift) {
                    let y = if up {
                        u32::from(window_bottom) - step
                    } else {
                        u32::from(window_top) + step
                    };
                    for byte_x in u32::from(left)..=u32::from(right) {
                        let address = base + y * bytes_per_row + byte_x;
                        let _ = self.read_mem_byte(address);
                        self.write_mem_byte(address, fill & 0x0F);
                    }
                }
                self.restore_vram_access_registers(entry.registers);
            }
            ModeFamily::Packed => {
                let width = u32::from(entry.width);
                let first = u32::from(left) * 8;
                let last = (u32::from(right) + 1) * 8 - 1;
                for step in 0..u32::from(move_scanlines) {
                    let (destination_y, source_y) = if up {
                        (
                            u32::from(window_top) + step,
                            u32::from(window_top + shift) + step,
                        )
                    } else {
                        (
                            u32::from(window_bottom) - step,
                            u32::from(window_bottom - shift) - step,
                        )
                    };
                    for x in first..=last {
                        let byte = self.packed_byte_read(entry, source_y * width + x);
                        self.packed_byte_write(entry, destination_y * width + x, byte);
                    }
                }
                for step in 0..u32::from(shift) {
                    let y = if up {
                        u32::from(window_bottom) - step
                    } else {
                        u32::from(window_top) + step
                    };
                    for x in first..=last {
                        self.packed_byte_write(entry, y * width + x, fill);
                    }
                }
            }
        }
    }
}
