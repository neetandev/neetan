//! PC-88VA text layer: character-generator lookup and per-raster text
//! rendering into palette-index scanlines.
//!
//! The text coordinate system is 1024 dots wide; the visible surface samples
//! the left 640. Text VRAM holds 2-byte little-endian hardware character codes,
//! with the attribute byte for each cell sitting `attroffset` bytes after the
//! code. Up to four split-screen frames are described by a table at
//! `texttable` inside text VRAM.

use alloc::{boxed::Box, vec};

use super::{VA_CHAR_WIDTH, VA_SURFACE_WIDTH, VA_TEXT_COORD_WIDTH};

/// Maximum scanlines per text row.
pub(super) const LINEHEIGHT_MAX: usize = 20;
/// Number of split-screen frames.
const FRAMES: usize = 4;
/// Per-frame descriptor stride in text VRAM bytes.
const FRAME_STRIDE: usize = 0x20;

const ATR_ST: u8 = 0x01;
const ATR_BL: u8 = 0x02;
const ATR_RV: u8 = 0x04;
const ATR_HL: u8 = 0x08;
const ATR_HL2: u8 = 0x10;

/// The 0xFF "tofu" glyph drawn for undefined external character codes.
const TOFU: u8 = 0xFF;

/// A character generator source for a looked-up glyph.
enum FontSource {
    /// An offset into the font ROM with the given per-raster byte stride.
    Rom { offset: usize, stride: usize },
    /// The fixed 0xFF tofu block.
    Tofu,
}

/// Resolves a hardware character code to its glyph source, mirroring
/// `cgromva_font`/`cgromva_width`. `eight_dot` selects the 8-dot ANK font.
fn cgrom_font(hccode: u16, eight_dot: bool) -> FontSource {
    let lr = usize::from(hccode >> 15);
    let jis1 = (hccode & 0x7F) + 0x20;
    let jis2 = (hccode >> 8) & 0x7F;
    let stride = if hccode & 0x7F00 == 0 { 1 } else { 2 };

    if jis2 == 0 && lr == 0 {
        let offset = if eight_dot {
            0x41000 + (usize::from(hccode & 0xFF) << 3)
        } else {
            0x40000 + (usize::from(hccode & 0xFF) << 4)
        };
        return FontSource::Rom { offset, stride };
    }

    let jis2 = usize::from(jis2);
    let jis1u = usize::from(jis1);
    let offset = if jis1 < 0x28 {
        lr + ((jis2 & 0x60) << 8) + ((jis1u & 0x07) << 10) + ((jis2 & 0x1F) << 5)
    } else if jis1 < 0x30 {
        lr + 0x40000 + ((jis2 & 0x60) << 8) + ((jis1u & 0x07) << 10) + ((jis2 & 0x1F) << 5)
    } else if jis1 < 0x40 {
        lr + ((jis2 & 0x60) << 10) + ((jis1u & 0x0F) << 10) + ((jis2 & 0x1F) << 5)
    } else if jis1 < 0x50 {
        lr + 0x4000 + ((jis2 & 0x60) << 10) + ((jis1u & 0x0F) << 10) + ((jis2 & 0x1F) << 5)
    } else if jis1 < 0x60 {
        lr + 0x20000 + ((jis2 & 0x60) << 10) + ((jis1u & 0x0F) << 10) + ((jis2 & 0x1F) << 5)
    } else if jis1 < 0x70 {
        lr + 0x24000 + ((jis2 & 0x60) << 10) + ((jis1u & 0x0F) << 10) + ((jis2 & 0x1F) << 5)
    } else if jis1 < 0x76 {
        lr + 0x20000 + ((jis2 & 0x60) << 8) + ((jis1u & 0x07) << 10) + ((jis2 & 0x1F) << 5)
    } else if jis1 < 0x78 {
        // External / gaiji codes live in backup RAM, which the renderer does
        // not carry; draw the tofu block for the whole range.
        return FontSource::Tofu;
    } else {
        0
    };
    FontSource::Rom { offset, stride }
}

/// Returns the glyph byte for raster `r`, or 0 past the source bounds.
fn font_byte(font_rom: &[u8], source: &FontSource, raster: usize) -> u8 {
    match *source {
        FontSource::Rom { offset, stride } => {
            font_rom.get(offset + raster * stride).copied().unwrap_or(0)
        }
        FontSource::Tofu => TOFU,
    }
}

/// A decoded attribute (foreground/background palette indices plus effect bits).
struct CharAttr {
    background: u8,
    foreground: u8,
    effect: u8,
}

/// A split-screen text frame descriptor (a copy of the table entry).
#[derive(Clone, Copy, Default)]
struct TextFrame {
    /// Frame-buffer width in bytes.
    width_bytes: u16,
    /// Display mode (bits 0-4); selects the attribute decode.
    mode: u8,
    /// Default foreground color (mode 1).
    foreground: u8,
    /// Default background color (mode 1).
    background: u8,
    /// First raster of the frame within a row.
    raster_offset: usize,
    /// Raster start address (byte offset into text VRAM).
    raster_start: usize,
    /// Frame height in rasters.
    height: usize,
    /// Width in characters (`rw / 8 + 2`).
    width_chars: usize,
    /// Horizontal start position in dots.
    x_position: usize,
}

/// Per-frame inputs needed to rasterize the text layer.
pub(super) struct TextContext<'a> {
    pub text_vram: &'a [u8],
    pub font_rom: &'a [u8],
    pub attr_offset: usize,
    pub horizontal_line_position: usize,
    pub blink_counter2: u8,
    pub text_magnify: bool,
    pub eight_dot: bool,
    pub text_off: bool,
    pub forty_column: bool,
}

/// Scratch state and output buffers for the text raster walk (one `_TEXTVAWORK`).
pub(super) struct TextWork {
    line_bitmap: Box<[u8]>,
    /// Current scanline's palette indices for the visible surface width.
    pub raster_out: Box<[u8]>,
    frames: [TextFrame; FRAMES],
    line_height: usize,
    screen_y: usize,
    text_y: usize,
    inner_y: usize,
    raster: usize,
    texty: usize,
    line_bitmap_ready: bool,
    frame_index: usize,
    frame_limit: usize,
}

impl TextWork {
    pub(super) fn new() -> Self {
        Self {
            line_bitmap: vec![0u8; VA_TEXT_COORD_WIDTH * LINEHEIGHT_MAX].into_boxed_slice(),
            raster_out: vec![0u8; VA_SURFACE_WIDTH].into_boxed_slice(),
            frames: [TextFrame::default(); FRAMES],
            line_height: 1,
            screen_y: 0,
            text_y: 0,
            inner_y: 0,
            raster: 0,
            texty: 0,
            line_bitmap_ready: false,
            frame_index: 0,
            frame_limit: 0,
        }
    }

    /// Parses the four frame descriptors and resets the per-frame walk.
    pub(super) fn begin(&mut self, text_vram: &[u8], texttable: usize, line_height: usize) {
        self.screen_y = 0;
        self.inner_y = 0;
        self.text_y = 0;
        self.line_height = line_height.clamp(1, LINEHEIGHT_MAX);

        let mut base = texttable;
        for frame in &mut self.frames {
            let read_word = |offset: usize| -> u16 {
                let index = base + offset;
                let low = text_vram.get(index).copied().unwrap_or(0);
                let high = text_vram.get(index + 1).copied().unwrap_or(0);
                u16::from(low) | (u16::from(high) << 8)
            };
            frame.width_bytes = read_word(0x08) & 0x03FF;
            let mode_word = read_word(0x0A);
            frame.mode = (mode_word & 0x1F) as u8;
            frame.background = ((mode_word & 0x0F00) >> 8) as u8;
            frame.foreground = ((mode_word & 0xF000) >> 12) as u8;
            frame.raster_offset =
                usize::from(text_vram.get(base + 0x0D).copied().unwrap_or(0) & 0x1F);
            frame.raster_start = usize::from(read_word(0x10));
            let mut height = usize::from(read_word(0x14) & 0x01FE);
            if height == 0 {
                height = 0x01FE;
            }
            frame.height = height;
            let raster_width = usize::from(read_word(0x16) & 0x03FF);
            frame.width_chars = raster_width / VA_CHAR_WIDTH + 2;
            frame.x_position = usize::from(read_word(0x1A) & 0x03FF);
            base += FRAME_STRIDE;
        }
        self.select_frame(0);
    }

    fn select_frame(&mut self, index: usize) {
        self.frame_index = index;
        let frame = &self.frames[index];
        self.frame_limit = if index == FRAMES - 1 {
            0x1FE
        } else {
            self.inner_y + frame.height
        };
        self.texty = 0;
        self.raster = frame.raster_offset;
        self.line_bitmap_ready = false;
    }

    /// Decodes an attribute byte per the current frame's mode.
    fn decode_attr(&self, attr: u8) -> CharAttr {
        let frame = &self.frames[self.frame_index];
        match frame.mode & 0x07 {
            1 => CharAttr {
                background: frame.background,
                foreground: attr >> 4,
                effect: attr & 0x0F,
            },
            _ => CharAttr {
                background: attr >> 4,
                foreground: attr & 0x0F,
                effect: 0,
            },
        }
    }

    /// Renders one text row into `line_bitmap`.
    fn make_line(&mut self, context: &TextContext<'_>, vram_index: usize, width_chars: usize) {
        let width_chars = width_chars.min(VA_TEXT_COORD_WIDTH / VA_CHAR_WIDTH);
        for byte in self.line_bitmap.iter_mut() {
            *byte = 0;
        }
        let font_height = if context.eight_dot { 8 } else { 16 };

        for cell in 0..width_chars {
            let code_index = vram_index + cell * 2;
            let low = context.text_vram.get(code_index).copied().unwrap_or(0);
            let high = context.text_vram.get(code_index + 1).copied().unwrap_or(0);
            let hccode = u16::from(low) | (u16::from(high) << 8);
            let attr = context
                .text_vram
                .get(code_index + context.attr_offset)
                .copied()
                .unwrap_or(0);
            let char_attr = self.decode_attr(attr);

            let (background, mut foreground) = if char_attr.effect & ATR_RV != 0 {
                (char_attr.foreground, char_attr.background)
            } else {
                (char_attr.background, char_attr.foreground)
            };
            let underline = foreground;
            // Secret (always) and blink (during the off phase) both hide the
            // glyph by forcing the foreground to the background color; the
            // underline keeps its original foreground color either way.
            let secret = char_attr.effect & ATR_ST != 0;
            let blinked = char_attr.effect & ATR_BL != 0 && context.blink_counter2 & 0x18 == 0x08;
            if secret || blinked {
                foreground = background;
            }

            let has_line = char_attr.effect & (ATR_HL | ATR_HL2) != 0;
            let column = cell * VA_CHAR_WIDTH;
            if (hccode == 0 || hccode == 0x20) && background == 0 && !has_line {
                continue;
            }

            let source = cgrom_font(hccode, context.eight_dot);
            for raster in 0..self.line_height {
                let row = column + raster * VA_TEXT_COORD_WIDTH;
                if has_line && raster == context.horizontal_line_position {
                    for offset in 0..VA_CHAR_WIDTH {
                        self.line_bitmap[row + offset] = underline;
                    }
                } else if raster < font_height {
                    let mut data = font_byte(context.font_rom, &source, raster);
                    for offset in 0..VA_CHAR_WIDTH {
                        self.line_bitmap[row + offset] = if data & 0x80 != 0 {
                            foreground
                        } else {
                            background
                        };
                        data <<= 1;
                    }
                } else {
                    for offset in 0..VA_CHAR_WIDTH {
                        self.line_bitmap[row + offset] = background;
                    }
                }
            }
        }
    }

    /// Doubles each dot horizontally within `line_bitmap` for 40-column mode.
    fn conv_40column(&mut self, width_chars: usize) {
        let width_chars = width_chars.min(VA_TEXT_COORD_WIDTH / VA_CHAR_WIDTH);
        for raster in 0..self.line_height {
            let base = raster * VA_TEXT_COORD_WIDTH;
            // Walk backwards so each source dot can be expanded in place.
            let mut source = base + width_chars * VA_CHAR_WIDTH / 2;
            let mut destination = base + width_chars * VA_CHAR_WIDTH;
            while destination > base {
                source -= 1;
                let value = self.line_bitmap.get(source).copied().unwrap_or(0);
                destination -= 1;
                self.line_bitmap[destination] = value;
                destination -= 1;
                self.line_bitmap[destination] = value;
            }
        }
    }

    /// Clears the output raster (text display off / blank line).
    pub(super) fn blank_raster(&mut self) {
        for value in self.raster_out.iter_mut() {
            *value = 0;
        }
    }

    /// Produces the next output scanline's text palette indices.
    pub(super) fn raster(&mut self, context: &TextContext<'_>) {
        if context.text_off {
            self.blank_raster();
            return;
        }

        if !context.text_magnify || self.screen_y & 1 == 0 {
            while self.inner_y >= self.frame_limit && self.frame_index + 1 < FRAMES {
                let next = self.frame_index + 1;
                self.select_frame(next);
            }
            let frame = self.frames[self.frame_index];

            if !self.line_bitmap_ready {
                let vram_index = frame.raster_start + frame.width_bytes as usize * self.texty;
                self.make_line(context, vram_index, frame.width_chars);
                if context.forty_column {
                    self.conv_40column(frame.width_chars);
                }
                self.texty += 1;
                self.line_bitmap_ready = true;
            }

            // Horizontal scroll (`rxp`): the visible 640 dots sample a window that
            // starts `1024 - rxp` into the 1024-wide line and wraps at 1024 back to
            // 0. When `rxp >= 640` the whole window fits before the wrap, so the
            // wrapped tail is never reached (matching `maketextva_raster`).
            let line_base = self.raster * VA_TEXT_COORD_WIDTH;
            let rxp = frame.x_position.min(VA_TEXT_COORD_WIDTH);
            let wrap_start = if rxp != 0 {
                VA_TEXT_COORD_WIDTH - rxp
            } else {
                0
            };
            if rxp < VA_SURFACE_WIDTH {
                for x in 0..rxp {
                    self.raster_out[x] = self.line_bitmap[line_base + wrap_start + x];
                }
                for x in rxp..VA_SURFACE_WIDTH {
                    self.raster_out[x] = self.line_bitmap[line_base + (x - rxp)];
                }
            } else {
                for x in 0..VA_SURFACE_WIDTH {
                    self.raster_out[x] = self.line_bitmap[line_base + wrap_start + x];
                }
            }

            self.raster += 1;
            if self.raster >= self.line_height {
                self.line_bitmap_ready = false;
                self.raster = 0;
            }
            self.inner_y += 1;
            self.text_y += 1;
        }
        self.screen_y += 1;
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    #[test]
    fn ank_lookup_targets_the_16_dot_block() {
        match cgrom_font(0x0041, false) {
            FontSource::Rom { offset, stride } => {
                assert_eq!(offset, 0x40000 + (0x41 << 4));
                assert_eq!(stride, 1);
            }
            FontSource::Tofu => panic!("ANK must use the ROM"),
        }
    }

    #[test]
    fn ank_glyph_renders_foreground_pixels() {
        let mut font = vec![0u8; 0x50000];
        // 'A' at 16-dot ANK slot: top raster solid.
        font[0x40000 + (0x41 << 4)] = 0b1010_0000;
        let mut work = TextWork::new();
        let mut vram = vec![0u8; 0x1000];
        // One cell: code 0x41, attribute fg=2 bg=0 (mode 0 packs bg<<4 | fg).
        vram[0] = 0x41;
        vram[1] = 0x00;
        vram[0x100] = 0x02; // attribute at attr_offset 0x100
        let context = TextContext {
            text_vram: &vram,
            font_rom: &font,
            attr_offset: 0x100,
            horizontal_line_position: 0xFF,
            blink_counter2: 0,
            text_magnify: false,
            eight_dot: false,
            text_off: false,
            forty_column: false,
        };
        // Single frame covering the whole row, 80 chars wide, height 16.
        work.frames[0] = TextFrame {
            width_bytes: 160,
            mode: 0,
            foreground: 0,
            background: 0,
            raster_offset: 0,
            raster_start: 0,
            height: 0x1FE,
            width_chars: 82,
            x_position: 0,
        };
        work.line_height = 16;
        work.frame_limit = 0x1FE;
        work.raster(&context);
        assert_eq!(work.raster_out[0], 2);
        assert_eq!(work.raster_out[1], 0);
        assert_eq!(work.raster_out[2], 2);
    }

    #[test]
    fn large_x_position_offsets_without_truncating() {
        // `rxp >= 640` samples a single 640-wide window starting `1024 - rxp`
        // into the line, with no wrap. The whole visible surface must come from
        // that window, not be clamped to a 640 offset (the prior bug).
        let mut work = TextWork::new();
        // Mark each line-bitmap dot with a recognizable ramp so the source
        // position is observable in the output.
        work.line_height = 1;
        for x in 0..VA_TEXT_COORD_WIDTH {
            work.line_bitmap[x] = (x & 0x0F) as u8;
        }
        // Bypass make_line: pretend the row bitmap is already prepared.
        work.line_bitmap_ready = true;
        work.frames[0] = TextFrame {
            width_bytes: 0,
            mode: 0,
            foreground: 0,
            background: 0,
            raster_offset: 0,
            raster_start: 0,
            height: 0x1FE,
            width_chars: 0,
            x_position: 1008, // 1024 - 1008 = 16
        };
        work.frame_limit = 0x1FE;
        work.raster = 0;
        let context = TextContext {
            text_vram: &[0u8; 16],
            font_rom: &[0u8; 0x50000],
            attr_offset: 0,
            horizontal_line_position: 0xFF,
            blink_counter2: 0,
            text_magnify: false,
            eight_dot: false,
            text_off: false,
            forty_column: false,
        };
        work.raster(&context);
        // Output dot x reads line_bitmap[16 + x].
        for x in 0..VA_SURFACE_WIDTH {
            assert_eq!(work.raster_out[x], ((16 + x) & 0x0F) as u8, "dot {x}");
        }
    }
}
