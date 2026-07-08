//! X1 text / PCG / kanji tilemap layer.
//!
//! Renders one scanline of character cells into 3-bit text colours. Each cell
//! reads a code from text VRAM and an attribute from attribute VRAM; the
//! attribute selects the colour, reverse and blink handling, double width and
//! height, and whether the glyph comes from a font ROM or the programmable
//! character generator (PCG, three colour planes). On the turbo the per-cell
//! kanji VRAM (kvram) can switch the glyph to a kanji ROM character or a
//! 16-line gaiji PCG character. Colour 0 is transparent in the priority mixing,
//! so reversed or blanked pixels let the graphics layer show through.

use super::{RenderInputsX1, X1RendererModel};

const TEXT_VRAM_MASK: usize = 0x7FF;
const PCG_PLANE_STRIDE: usize = 0x800;
const GAIJI_PLANE_STRIDE: usize = 0x800;
/// Offset of the 8x16 glyphs within the ANK font ROM.
const ANK16_ROM_OFFSET: usize = 0x1000;

const ATTR_DOUBLE_WIDTH: u8 = 0x80;
const ATTR_DOUBLE_HEIGHT: u8 = 0x40;
const ATTR_PCG_SELECT: u8 = 0x20;
const ATTR_BLINK: u8 = 0x10;
const ATTR_REVERSE: u8 = 0x08;
const ATTR_PEN_MASK: u8 = 0x07;

const KVRAM_KANJI_ENABLE: u8 = 0x80;
const KVRAM_KANJI_SIDE: u8 = 0x40;
const KVRAM_KANJI_UNDERLINE: u8 = 0x20;
const KVRAM_GAIJI: u8 = 0x90;
const KVRAM_KANJI_BANK: u8 = 0x0F;

/// Marker written on the kanji-underline raster; the graphics layer turns it
/// into graphics colour 1 and clears it from the text row.
pub(super) const KSEN_UNDERLINE_MARKER: u8 = 8;

/// Which glyph source a cell reads its pattern rows from.
enum Glyph {
    /// PCG colour planes, 8 rows per code.
    Pcg { code: usize },
    /// Gaiji PCG colour planes, 16 rows per code pair (turbo).
    Gaiji { code: usize },
    /// Kanji ROM half, 16 monochrome rows (turbo).
    Kanji { offset: usize },
    /// 8x16 ANK font ROM (turbo 16-line text modes).
    Ank16 { code: usize },
    /// 8x8 CG-ROM font.
    CgRom { code: usize },
}

/// Renders one text scanline into `text_row` (one byte per pixel, colours
/// 0..7). `raster` is the frame-global glyph-row counter shared by all lines:
/// cells without the double-height attribute reload it from the cell raster,
/// double-height cells keep it advancing at half rate.
pub(super) fn draw_text_line(
    inputs: &RenderInputsX1<'_>,
    font: &[u8],
    line: usize,
    raster: &mut u16,
    text_row: &mut [u8],
) {
    let ch_height = usize::from(inputs.ch_height).max(1);
    let y = line / ch_height;
    let l = line % ch_height;
    let width = inputs.cell_limit();
    let hz_disp = usize::from(inputs.hz_disp);
    let font16 = inputs.mode1 & 0x05 != 0;
    let ksen = match inputs.model {
        X1RendererModel::Base => false,
        X1RendererModel::Turbo => inputs.mode1 & 0x80 != 0,
    };
    let (ksen_blank, ksen_underline) = if ksen {
        let underline_start = if font16 { 16 } else { 8 };
        let underline_raster = if font16 { 18 } else { 9 };
        (l >= underline_start, l == underline_raster)
    } else {
        (false, false)
    };

    let mut src = usize::from(inputs.st_addr) + hz_disp * y;
    let mut last_vert_double = true;
    let mut prev_attr = 0u8;
    let mut cur_pattern = [0u8; 3];

    for x in 0..hz_disp.min(width) {
        src &= TEXT_VRAM_MASK;
        let code = usize::from(inputs.text_vram[src]);
        let attr = inputs.attr_vram[src];
        let kanji_attr = match inputs.model {
            X1RendererModel::Base => 0,
            X1RendererModel::Turbo => inputs.kvram.get(src).copied().unwrap_or(0),
        };
        let src_odd = src & 1 != 0;
        src += 1;

        let color = attr & ATTR_PEN_MASK;
        let blink = (attr & ATTR_BLINK) != 0 && (inputs.cblink & 0x20) != 0;
        let reverse = ((attr & ATTR_REVERSE) != 0) != blink;
        let vert_double = (attr & ATTR_DOUBLE_HEIGHT) != 0;
        if !vert_double {
            *raster = l as u16;
        }
        last_vert_double = vert_double;

        let mut max_line = 8usize;
        let mut shift = 0i16;
        let glyph = if attr & ATTR_PCG_SELECT != 0 {
            if kanji_attr & KVRAM_GAIJI != 0 {
                max_line = 16;
                Glyph::Gaiji { code }
            } else {
                shift = i16::from(inputs.mode1 & 0x01 != 0);
                Glyph::Pcg { code }
            }
        } else if kanji_attr & KVRAM_KANJI_ENABLE != 0 {
            max_line = 16;
            let bank = usize::from(kanji_attr & KVRAM_KANJI_BANK);
            let side = usize::from((kanji_attr & KVRAM_KANJI_SIDE) != 0);
            let tile = ((code + (bank << 8)) << 1) + side;
            Glyph::Kanji { offset: tile * 16 }
        } else if font16 {
            max_line = 16;
            Glyph::Ank16 { code }
        } else {
            Glyph::CgRom { code }
        };
        // A 16-line font in a smaller cell is fitted by mode1 (I/O 0x1FD0) bits
        // 0 and 2, not by the CRTC cell height: both set stretches 2x, exactly
        // one shows the rows as-is (so an 8-raster cell shows only the top
        // half), and both clear thins the odd rows away (rows 0, 2, ... 14).
        // Source: https://takeda-toshiya.my.coocan.jp/x1twin/index.html (2019/2/9)
        if max_line == 16 {
            shift = if inputs.mode1 & 0x05 == 0x05 {
                1
            } else if inputs.mode1 & 0x05 != 0 {
                0
            } else {
                -1
            };
        }

        let mut line_index = usize::from(*raster);
        match shift {
            1 => line_index >>= 1,
            -1 => {
                line_index <<= 1;
                if vert_double {
                    line_index |= l & 1;
                }
            }
            _ => {}
        }

        // A horizontally doubled cell at an odd VRAM address continues the
        // previous cell's glyph instead of loading its own pattern.
        if !(src_odd && (prev_attr & ATTR_DOUBLE_WIDTH) != 0) {
            cur_pattern = fetch_pattern(inputs, font, &glyph, line_index % max_line);
        }
        let [mut blue, mut red, mut green] = cur_pattern;
        if reverse {
            blue = if color & 1 == 0 { 0xFF } else { !blue };
            red = if color & 2 == 0 { 0xFF } else { !red };
            green = if color & 4 == 0 { 0xFF } else { !green };
        } else {
            blue = if color & 1 == 0 { 0 } else { blue };
            red = if color & 2 == 0 { 0 } else { red };
            green = if color & 4 == 0 { 0 } else { green };
        }

        let cell = &mut text_row[x * 8..x * 8 + 8];
        if ksen_blank {
            let value = if ksen_underline && (kanji_attr & KVRAM_KANJI_UNDERLINE) != 0 {
                KSEN_UNDERLINE_MARKER
            } else {
                0
            };
            cell.fill(value);
        } else if attr & ATTR_DOUBLE_WIDTH != 0 {
            for k in 0..4 {
                let pixel = pixel_color(blue, red, green, k);
                cell[k * 2] = pixel;
                cell[k * 2 + 1] = pixel;
            }
            cur_pattern = cur_pattern.map(|plane| plane << 4);
        } else {
            for (k, pixel) in cell.iter_mut().enumerate() {
                *pixel = pixel_color(blue, red, green, k);
            }
            cur_pattern = [0u8; 3];
        }
        prev_attr = attr;
    }

    if !last_vert_double || (l & 1) != 0 {
        *raster = (*raster + 1) % ch_height as u16;
    }
}

/// The three plane pattern bytes of `glyph` at `row`.
fn fetch_pattern(inputs: &RenderInputsX1<'_>, font: &[u8], glyph: &Glyph, row: usize) -> [u8; 3] {
    match *glyph {
        Glyph::Pcg { code } => {
            let base = (code * 8 + row) & TEXT_VRAM_MASK;
            [
                byte_at(inputs.pcg, base),
                byte_at(inputs.pcg, base + PCG_PLANE_STRIDE),
                byte_at(inputs.pcg, base + PCG_PLANE_STRIDE * 2),
            ]
        }
        Glyph::Gaiji { code } => {
            let base = (code >> 1) * 16 + row;
            [
                byte_at(inputs.gaiji, base),
                byte_at(inputs.gaiji, base + GAIJI_PLANE_STRIDE),
                byte_at(inputs.gaiji, base + GAIJI_PLANE_STRIDE * 2),
            ]
        }
        Glyph::Kanji { offset } => {
            let byte = byte_at(inputs.kanji_rom, offset + row);
            [byte; 3]
        }
        Glyph::Ank16 { code } => {
            let byte = byte_at(inputs.ank_rom, ANK16_ROM_OFFSET + code * 16 + row);
            [byte; 3]
        }
        Glyph::CgRom { code } => {
            let byte = byte_at(font, (code * 8 + row) & TEXT_VRAM_MASK);
            [byte; 3]
        }
    }
}

fn byte_at(data: &[u8], offset: usize) -> u8 {
    data.get(offset).copied().unwrap_or(0)
}

/// Packs pixel `k` (0 = leftmost) of the three plane bytes into a text colour.
fn pixel_color(blue: u8, red: u8, green: u8, k: usize) -> u8 {
    let bit = 7 - k;
    ((blue >> bit) & 1) | (((red >> bit) & 1) << 1) | (((green >> bit) & 1) << 2)
}
