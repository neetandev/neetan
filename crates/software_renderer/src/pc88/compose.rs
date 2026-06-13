//! PC-88 compose pass: graphics layer under text, into a packed RGBA framebuffer.
//!
//! Two pen-index layers are built (graphics via [`super::graphics`], text here),
//! then composited per the PC-88 screen priority rules: text has
//! priority and the graphics layer shows through text "0" pixels.

use super::{
    ANK_FONT_OFFSET, GraphicsMode88, PC88_MAX_HEIGHT, PC88_PIXEL_BYTES, PC88_WIDTH, RenderInputs88,
    glyph::TEXT_PALETTE, graphics,
};

const MAX_COLUMNS: usize = 80;

/// Composites the graphics and text layers into `framebuffer` (packed RGBA).
pub fn compose(
    framebuffer: &mut [u8],
    font_rom: &[u8],
    sg_pattern: &[u8],
    graph_pens: &mut [u8],
    text_pens: &mut [u8],
    inputs: &RenderInputs88<'_>,
) {
    let width = inputs.width.min(PC88_WIDTH as u32) as usize;
    let height = (inputs.height as usize).min(PC88_MAX_HEIGHT);
    let char_height = inputs.char_height.max(1) as usize;
    let text_native = (inputs.rows as usize * char_height).min(PC88_MAX_HEIGHT);

    graphics::rasterize(inputs, graph_pens, height);
    rasterize_text(inputs, font_rom, sg_pattern, text_pens, text_native);

    composite(
        framebuffer,
        graph_pens,
        text_pens,
        inputs,
        width,
        height,
        text_native,
    );
}

/// Rasterizes the text layer into `text_pens` as pen indices (0 = transparent).
fn rasterize_text(
    inputs: &RenderInputs88<'_>,
    font_rom: &[u8],
    sg_pattern: &[u8],
    text_pens: &mut [u8],
    text_native: usize,
) {
    text_pens[..PC88_WIDTH * text_native].fill(0);
    if !inputs.text_enabled {
        return;
    }

    let char_height = inputs.char_height.max(1) as usize;
    let columns = inputs.columns.min(MAX_COLUMNS as u32) as usize;
    let rows = inputs.rows as usize;
    let color_mask = if inputs.color_mode { 0 } else { 7 };

    for cy in 0..rows {
        for cx in 0..columns {
            if inputs.width_40col && (cx & 1) != 0 {
                continue;
            }
            let cell = cy * MAX_COLUMNS + cx;
            let attrib = inputs.text_attrib.get(cell).copied().unwrap_or(0);
            let color = ((attrib >> 5) | color_mask) & 7;
            let under_line = attrib & 0x08 != 0;
            let upper_line = attrib & 0x04 != 0;
            let secret = attrib & 0x02 != 0;
            let reverse = attrib & 0x01 != 0;

            let code = if secret {
                0
            } else {
                inputs.text_codes.get(cell).copied().unwrap_or(0)
            } as usize;
            let glyph = if attrib & 0x10 != 0 {
                &sg_pattern[code * 8..code * 8 + 8]
            } else {
                let base = ANK_FONT_OFFSET + code * 8;
                if base + 8 <= font_rom.len() {
                    &font_rom[base..base + 8]
                } else {
                    &sg_pattern[0..8]
                }
            };

            for l in 0..char_height {
                let glyph_row = (l * 8) / char_height;
                let mut pattern = if glyph_row < 8 { glyph[glyph_row] } else { 0 };
                if (upper_line && l == 0) || (under_line && l == char_height - 1) {
                    pattern = 0xFF;
                }
                if reverse {
                    pattern ^= 0xFF;
                }

                let y = cy * char_height + l;
                if y >= text_native {
                    break;
                }
                draw_text_row(text_pens, y, cx, pattern, color, inputs.width_40col);
            }
        }
    }
}

fn draw_text_row(text_pens: &mut [u8], y: usize, cx: usize, pattern: u8, color: u8, wide: bool) {
    let row = y * PC88_WIDTH;
    for bit in 0..8 {
        if pattern & (0x80 >> bit) == 0 {
            continue;
        }
        if wide {
            let x = cx * 8 + bit * 2;
            if x + 1 < PC88_WIDTH {
                text_pens[row + x] = color;
                text_pens[row + x + 1] = color;
            }
        } else {
            let x = cx * 8 + bit;
            if x < PC88_WIDTH {
                text_pens[row + x] = color;
            }
        }
    }
}

fn composite(
    framebuffer: &mut [u8],
    graph_pens: &[u8],
    text_pens: &[u8],
    inputs: &RenderInputs88<'_>,
    width: usize,
    height: usize,
    text_native: usize,
) {
    let background = inputs.background_rgb;
    let color8 = inputs.graphics_mode == GraphicsMode88::Color8;

    // Effective per-layer palettes. In non-Color8 modes pen 0 resolves to the
    // background color (palette[0] = palette[8]).
    let mut graph_palette = inputs.graphics_palette;
    let mut text_palette = TEXT_PALETTE8;
    if !color8 {
        graph_palette[0] = background;
        text_palette[0] = background;
    }
    let (text_layer, graph_layer) = if color8 {
        (TEXT_PALETTE8, inputs.graphics_palette)
    } else if inputs.palette_mode {
        (graph_palette, graph_palette)
    } else {
        (text_palette, text_palette)
    };

    for y in 0..height {
        // When the surface is taller than the text grid (a 400-line graphics mode
        // over a 200-line text grid), the text layer is doubled into it.
        let text_y = if text_native != 0 && height > text_native {
            y * text_native / height
        } else {
            y
        };
        for x in 0..width {
            let text_pen = if text_y < text_native {
                text_pens[text_y * PC88_WIDTH + x]
            } else {
                0
            };
            let rgb = if text_pen != 0 {
                text_layer[text_pen as usize]
            } else if inputs.graphics_enabled {
                graph_layer[graph_pens[y * PC88_WIDTH + x] as usize]
            } else {
                background
            };
            let index = (y * PC88_WIDTH + x) * PC88_PIXEL_BYTES;
            framebuffer[index] = rgb[0];
            framebuffer[index + 1] = rgb[1];
            framebuffer[index + 2] = rgb[2];
            framebuffer[index + 3] = 0xFF;
        }
    }
}

/// The eight text pens (the fixed GRB text palette, dropping the no-color slot).
const TEXT_PALETTE8: [[u8; 3]; 8] = [
    TEXT_PALETTE[0],
    TEXT_PALETTE[1],
    TEXT_PALETTE[2],
    TEXT_PALETTE[3],
    TEXT_PALETTE[4],
    TEXT_PALETTE[5],
    TEXT_PALETTE[6],
    TEXT_PALETTE[7],
];
