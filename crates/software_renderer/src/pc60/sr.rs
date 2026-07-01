//! Native PC-6001mkIISR / PC-6601SR rendering.
//!
//! Two native modes share a 320x240 frame: a 4bpp 320x204 bitmap with hardware
//! scroll, and a text mode of 20 or 25 rows of 8x12 cells. Both draw from the
//! fixed sixteen-colour SR palette, which occupies pens 16-31 of [`MK2_PALETTE`].

use super::{MK2_PALETTE, PC60_MK2_HEIGHT, PC60_MK2_WIDTH, RenderInputsSr, put_pixel};

/// Frame dimensions (shared with the mkII frame).
const WIDTH: usize = PC60_MK2_WIDTH;
const HEIGHT: usize = PC60_MK2_HEIGHT;

/// Native bitmap dimensions; the scroll arithmetic wraps at this extent.
const BITMAP_WIDTH: usize = 320;
const BITMAP_HEIGHT: usize = 204;

/// Text cell dimensions.
const CELL_WIDTH: usize = 8;
const CELL_HEIGHT: usize = 12;
/// Bytes per glyph in the character generator.
const GLYPH_STRIDE: usize = 0x10;

/// Palette pen of the first SR colour.
const SR_PEN_BASE: usize = 0x10;
/// Text background pens start eight entries above the foreground base.
const SR_BG_PEN_BASE: usize = 0x18;

/// Renders one native SR frame into `framebuffer` (320x240 RGBA).
pub(crate) fn render(inputs: &RenderInputsSr, framebuffer: &mut [u8]) {
    for pixel in framebuffer.chunks_exact_mut(4) {
        pixel.copy_from_slice(&MK2_PALETTE[0]);
    }
    if inputs.text_mode {
        draw_text(inputs, framebuffer);
    } else {
        draw_bitmap(inputs, framebuffer);
    }
}

/// Maps a 4-bit graphics nibble onto a palette pen. The hardware reorders the
/// colour bits before indexing the palette.
fn pen_from_nibble(nibble: u8) -> usize {
    let bit3 = (nibble >> 3) & 1;
    let bit2 = (nibble >> 2) & 1;
    let bit1 = (nibble >> 1) & 1;
    let bit0 = nibble & 1;
    let index = (bit3 << 3) | (bit0 << 2) | (bit2 << 1) | bit1;
    SR_PEN_BASE + index as usize
}

fn draw_bitmap(inputs: &RenderInputsSr, framebuffer: &mut [u8]) {
    let scroll_x = inputs.scroll_x as usize;
    let scroll_y = inputs.scroll_y as usize;
    let rows = BITMAP_HEIGHT.min(HEIGHT);
    for y in 0..rows {
        for x in 0..BITMAP_WIDTH {
            let address =
                ((x + scroll_x) % BITMAP_WIDTH) + ((y + scroll_y) % BITMAP_HEIGHT) * BITMAP_WIDTH;
            let nibble = inputs.gvram.get(address).copied().unwrap_or(0) & 0x0F;
            put_pixel(
                framebuffer,
                WIDTH,
                HEIGHT,
                x,
                y,
                MK2_PALETTE[pen_from_nibble(nibble)],
            );
        }
    }
}

fn draw_text(inputs: &RenderInputsSr, framebuffer: &mut [u8]) {
    let columns = if inputs.width80 { 80 } else { 40 };
    let rows = inputs.text_rows as usize;
    for row in 0..rows {
        for column in 0..columns {
            let cell = (column + row * columns) * 2;
            let tile = inputs.vram.get(cell).copied().unwrap_or(0) as usize;
            let attribute = inputs.vram.get(cell + 1).copied().unwrap_or(0);
            let tile = tile + (((attribute & 0x80) as usize) << 1);
            let foreground = SR_PEN_BASE + (attribute & 0x0F) as usize;
            let background = SR_BG_PEN_BASE + ((attribute & 0x70) >> 4) as usize;
            draw_glyph(
                inputs,
                framebuffer,
                tile,
                column * CELL_WIDTH,
                row * CELL_HEIGHT,
                foreground,
                background,
            );
        }
    }
}

fn draw_glyph(
    inputs: &RenderInputsSr,
    framebuffer: &mut [u8],
    tile: usize,
    origin_x: usize,
    origin_y: usize,
    foreground: usize,
    background: usize,
) {
    for line in 0..CELL_HEIGHT {
        let row_bits = inputs
            .cgrom
            .get(tile * GLYPH_STRIDE + line)
            .copied()
            .unwrap_or(0);
        for column in 0..CELL_WIDTH {
            let lit = (row_bits >> (7 - column)) & 1 != 0;
            let pen = if lit { foreground } else { background };
            put_pixel(
                framebuffer,
                WIDTH,
                HEIGHT,
                origin_x + column,
                origin_y + line,
                MK2_PALETTE[pen],
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;
    use crate::pc60::PC60_MK2_FRAMEBUFFER_BYTES;

    fn sr_inputs<'a>(vram: &'a [u8], cgrom: &'a [u8], gvram: &'a [u8]) -> RenderInputsSr<'a> {
        RenderInputsSr {
            vram,
            cgrom,
            gvram,
            text_mode: false,
            text_rows: 20,
            width80: false,
            scroll_x: 0,
            scroll_y: 0,
        }
    }

    #[test]
    fn bitmap_pen_maps_through_the_fixed_palette() {
        // A single white pixel (nibble that maps to pen 0x1F) at the origin.
        let mut gvram = vec![0u8; BITMAP_WIDTH * BITMAP_HEIGHT];
        gvram[0] = 0x0F;
        let inputs = sr_inputs(&[], &[], &gvram);
        let mut framebuffer = vec![0u8; PC60_MK2_FRAMEBUFFER_BYTES];
        render(&inputs, &mut framebuffer);
        assert_eq!(&framebuffer[0..4], &MK2_PALETTE[pen_from_nibble(0x0F)]);
    }

    #[test]
    fn bitmap_scroll_shifts_and_wraps() {
        let mut gvram = vec![0u8; BITMAP_WIDTH * BITMAP_HEIGHT];
        // Put a lit pixel at x=1; scrolling x by 1 brings it to the origin.
        gvram[1] = 0x0F;
        let mut inputs = sr_inputs(&[], &[], &gvram);
        inputs.scroll_x = 1;
        let mut framebuffer = vec![0u8; PC60_MK2_FRAMEBUFFER_BYTES];
        render(&inputs, &mut framebuffer);
        assert_eq!(&framebuffer[0..4], &MK2_PALETTE[pen_from_nibble(0x0F)]);
    }

    #[test]
    fn text_mode_draws_a_foreground_pixel() {
        // Cell 0: tile 1, foreground pen 0x1F (attribute low nibble 0x0F).
        let mut vram = vec![0u8; 0x1000];
        vram[0] = 0x01;
        vram[1] = 0x0F;
        let mut cgrom = vec![0u8; 0x4000];
        cgrom[GLYPH_STRIDE] = 0x80; // tile 1, line 0, leftmost pixel set
        let mut inputs = sr_inputs(&vram, &cgrom, &[]);
        inputs.text_mode = true;
        let mut framebuffer = vec![0u8; PC60_MK2_FRAMEBUFFER_BYTES];
        render(&inputs, &mut framebuffer);
        assert_eq!(&framebuffer[0..4], &MK2_PALETTE[SR_PEN_BASE + 0x0F]);
    }

    #[test]
    fn text_rows_select_the_drawn_height() {
        let vram = vec![0u8; 0x1000];
        let cgrom = vec![0u8; 0x4000];
        let mut inputs = sr_inputs(&vram, &cgrom, &[]);
        inputs.text_mode = true;
        inputs.text_rows = 25;
        let mut framebuffer = vec![0u8; PC60_MK2_FRAMEBUFFER_BYTES];
        // A blank glyph set still renders the background without panicking even
        // when 25 rows exceed the visible height.
        render(&inputs, &mut framebuffer);
    }
}
