//! X1 bitmap graphics layer.
//!
//! The bitmap is three 1-bit colour planes (blue, red, green) laid out one
//! character cell per byte with each scanline in its own line bank. The three
//! planes live at offsets 0x0000, 0x4000 and 0x8000; the turbo adds a second
//! page at 0xC000. Line banks are normally 0x800 bytes (eight rows); the turbo
//! 400-line addressing mode switches to 0x400-byte banks with sixteen rows. On
//! a hi-res scan each character row spans sixteen rasters: by default the two
//! pages interleave per raster to give 400 distinct lines, or a mode bit fixes
//! the page and line-doubles 200-line content. Each pixel yields a 3-bit
//! graphics colour into the priority mixing.

use super::{RenderInputsX1, X1RendererModel, text::KSEN_UNDERLINE_MARKER};

const CELL_MASK: usize = 0x7FF;
const PLANE_RED: usize = 0x4000;
const PLANE_GREEN: usize = 0x8000;
/// One bitmap page (three planes).
const PAGE_SIZE: usize = 0xC000;

/// Renders one bitmap scanline into `cg_row` (one byte per pixel, colours
/// 0..7). In the kanji-underline mode the graphics planes are not read at all:
/// the underline marker left in `text_row` transfers into the graphics layer
/// as colour 1 and is cleared from the text.
pub(super) fn draw_cg_line(
    inputs: &RenderInputsX1<'_>,
    line: usize,
    text_row: &mut [u8],
    cg_row: &mut [u8],
) {
    let ch_height = usize::from(inputs.ch_height).max(1);
    let y = line / ch_height;
    let l = line % ch_height;
    if y >= usize::from(inputs.vt_disp) {
        return;
    }
    let width = inputs.cell_limit();
    let hz_disp = usize::from(inputs.hz_disp);
    let cells = hz_disp.min(width);

    let ksen = match inputs.model {
        X1RendererModel::Base => false,
        X1RendererModel::Turbo => inputs.mode1 & 0x80 != 0,
    };
    if ksen {
        let font16 = inputs.mode1 & 0x05 != 0;
        let underline_raster = if font16 { 18 } else { 9 };
        for x in 0..cells {
            let text_cell = &mut text_row[x * 8..x * 8 + 8];
            let cg_cell = &mut cg_row[x * 8..x * 8 + 8];
            if l == underline_raster && text_cell[0] == KSEN_UNDERLINE_MARKER {
                cg_cell.fill(1);
                text_cell.fill(0);
            } else {
                cg_cell.fill(0);
            }
        }
        return;
    }

    let offset = match inputs.model {
        X1RendererModel::Base => 0x800 * (l & 7),
        X1RendererModel::Turbo => {
            let page = if inputs.hires && inputs.mode1 & 0x02 == 0 {
                l & 1
            } else {
                usize::from(inputs.mode1 & 0x08 != 0)
            };
            let line_in_page = if inputs.hires { l >> 1 } else { l };
            let mut offset = if inputs.mode1 & 0x04 != 0 {
                0x400 * (line_in_page & 15)
            } else {
                0x800 * (line_in_page & 7)
            };
            if page != 0 {
                offset += PAGE_SIZE;
            }
            offset
        }
    };

    let mut src = usize::from(inputs.st_addr) + hz_disp * y;
    for x in 0..cells {
        src &= CELL_MASK;
        let blue = byte_at(inputs.bitmap, offset | src);
        let red = byte_at(inputs.bitmap, (offset + PLANE_RED) | src);
        let green = byte_at(inputs.bitmap, (offset + PLANE_GREEN) | src);
        src += 1;
        let cell = &mut cg_row[x * 8..x * 8 + 8];
        for (k, pixel) in cell.iter_mut().enumerate() {
            let bit = 7 - k;
            *pixel = ((blue >> bit) & 1) | (((red >> bit) & 1) << 1) | (((green >> bit) & 1) << 2);
        }
    }
}

fn byte_at(bitmap: &[u8], offset: usize) -> u8 {
    bitmap.get(offset).copied().unwrap_or(0)
}
