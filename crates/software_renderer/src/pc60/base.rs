//! Base MC6847-style PC-6001 video modes: text, semigraphics, four-color
//! graphics and the two 2bpp bitmap modes. The mkII reuses these for its legacy
//! display path, rendering them centered inside the larger 320x240 canvas.

use crate::pc60::{BASE_PALETTE, RenderInputs60, put_pixel};

/// Number of text columns.
const TEXT_COLUMNS: usize = 32;
/// Number of text rows.
const TEXT_ROWS: usize = 16;
/// Character cell width in pixels.
const CELL_WIDTH: usize = 8;
/// Character cell height in pixels.
const CELL_HEIGHT: usize = 12;
/// Offset of the tile map within video RAM.
pub(crate) const TILE_MAP_OFFSET: usize = 0x200;

/// Active picture height in scanlines.
const PICTURE_HEIGHT: usize = 192;

/// Foreground palette index for the monochrome hi-res mode (white/buff).
const MODE4_FOREGROUND: usize = 4;

/// A render target: a framebuffer plus the canvas size and the origin the base
/// picture is drawn at (non-zero when centered on the mkII canvas).
pub(crate) struct Target<'a> {
    framebuffer: &'a mut [u8],
    width: usize,
    height: usize,
    origin_x: usize,
    origin_y: usize,
}

impl<'a> Target<'a> {
    pub(crate) fn new(
        framebuffer: &'a mut [u8],
        width: usize,
        height: usize,
        origin_x: usize,
        origin_y: usize,
    ) -> Self {
        Self {
            framebuffer,
            width,
            height,
            origin_x,
            origin_y,
        }
    }

    fn put(&mut self, x: usize, y: usize, color: usize) {
        put_pixel(
            self.framebuffer,
            self.width,
            self.height,
            self.origin_x + x,
            self.origin_y + y,
            BASE_PALETTE[color & 0x1F],
        );
    }
}

fn tile_at(vram: &[u8], index: usize) -> u8 {
    *vram.get(TILE_MAP_OFFSET + index).unwrap_or(&0)
}

/// Renders the base picture selected by the global attribute byte.
pub(crate) fn render(inputs: &RenderInputs60, target: &mut Target) {
    let attr = *inputs.vram.first().unwrap_or(&0);

    if attr & 0x80 != 0 {
        if attr & 0x10 != 0 {
            draw_gfx_mode4(inputs.vram, target);
        } else {
            draw_bitmap_2bpp(inputs.vram, attr, target);
        }
    } else {
        draw_text(inputs, target);
    }
}

fn draw_text(inputs: &RenderInputs60, target: &mut Target) {
    for cell_y in 0..TEXT_ROWS {
        for cell_x in 0..TEXT_COLUMNS {
            let cell = cell_x + cell_y * TEXT_COLUMNS;
            let tile = tile_at(inputs.vram, cell);
            let attr = *inputs.vram.get(cell & 0x1FF).unwrap_or(&0);

            if attr & 0x40 != 0 {
                draw_semigraphics_tile(cell_x, cell_y, tile, attr, target);
            } else {
                draw_text_tile(inputs.cgrom, cell_x, cell_y, tile, attr, target);
            }
        }
    }
}

fn draw_text_tile(
    cgrom: &[u8],
    cell_x: usize,
    cell_y: usize,
    tile: u8,
    attr: u8,
    target: &mut Target,
) {
    let fgcol = if attr & 2 != 0 { 0x12 } else { 0x10 };
    for yi in 0..CELL_HEIGHT {
        let glyph_row = *cgrom.get((tile as usize) * 0x10 + yi).unwrap_or(&0);
        for xi in 0..CELL_WIDTH {
            let pixel = (glyph_row >> (7 - xi)) & 1;
            let color = if attr & 1 != 0 {
                if pixel != 0 { fgcol } else { fgcol + 1 }
            } else if pixel != 0 {
                fgcol + 1
            } else {
                fgcol
            };
            target.put(cell_x * CELL_WIDTH + xi, cell_y * CELL_HEIGHT + yi, color);
        }
    }
}

fn draw_semigraphics_tile(cell_x: usize, cell_y: usize, tile: u8, attr: u8, target: &mut Target) {
    let pen = if attr & 0x10 != 0 {
        ((tile & 0x70) >> 4) as usize
    } else {
        (((tile & 0xC0) >> 6) | ((attr & 2) << 1)) as usize
    };

    for yi in 0..CELL_HEIGHT {
        for xi in 0..CELL_WIDTH {
            let mut bit_index = (xi & 4) >> 2;
            if attr & 0x10 != 0 {
                bit_index += if yi >= 6 { 2 } else { 0 };
            } else {
                bit_index += (yi & 4) >> 1;
                bit_index += (yi & 8) >> 1;
            }

            let color = if (tile >> bit_index) & 1 != 0 {
                pen + 8
            } else {
                0
            };
            target.put(
                cell_x * CELL_WIDTH + (7 - xi),
                cell_y * CELL_HEIGHT + (11 - yi),
                color,
            );
        }
    }
}

// Emits the raw 256x192 monochrome dot pattern (one bit per pixel) that the
// hardware placed on the composite line. The green/pink "four-color" look is a
// composite artifact of this pattern, synthesized downstream by the composite
// CRT shader.
fn draw_gfx_mode4(vram: &[u8], target: &mut Target) {
    for y in 0..PICTURE_HEIGHT {
        for x in 0..TEXT_COLUMNS {
            let tile = tile_at(vram, x + y * TEXT_COLUMNS);
            for bit in 0..CELL_WIDTH {
                let color = if (tile >> (7 - bit)) & 1 != 0 {
                    MODE4_FOREGROUND
                } else {
                    0
                };
                target.put(x * CELL_WIDTH + bit, y, color);
            }
        }
    }
}

fn draw_bitmap_2bpp(vram: &[u8], attr: u8, target: &mut Target) {
    let shrink_y = if attr & 8 != 0 { 1 } else { 2 };
    let col_bank = ((attr & 2) << 1) as usize;

    if attr & 4 != 0 {
        for y in 0..(PICTURE_HEIGHT / shrink_y) {
            for x in 0..TEXT_COLUMNS {
                let tile = tile_at(vram, x + y * TEXT_COLUMNS);
                for yi in 0..shrink_y {
                    for xi in 0..CELL_WIDTH {
                        let bit_index = xi & 0x06;
                        let color = (((tile >> bit_index) & 3) as usize) + 8 + col_bank;
                        target.put(x * CELL_WIDTH + (7 - xi), y * shrink_y + yi, color);
                    }
                }
            }
        }
    } else {
        let mut y = 0;
        while y < PICTURE_HEIGHT / shrink_y {
            for x in 0..TEXT_COLUMNS {
                let tile = tile_at(vram, x + (y / 3) * TEXT_COLUMNS);
                for yi in 0..shrink_y {
                    for xi in 0..CELL_WIDTH {
                        let bit_index = xi & 0x06;
                        let color = (((tile >> bit_index) & 3) as usize) + 8 + col_bank;
                        for row in 0..3 {
                            target.put(x * CELL_WIDTH + (7 - xi), (y + row) * shrink_y + yi, color);
                        }
                    }
                }
            }
            y += 3;
        }
    }
}
