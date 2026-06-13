//! PC-88 graphics-layer rasterization into a per-pixel pen-index buffer.
//!
//! Each graphics mode reads the three
//! 16 KiB GVRAM planes and emits a pen index per pixel (0-7). The composite pass
//! turns pen indices into RGB. Plane addressing is linear: 80 bytes per scanline,
//! 200 scanlines per plane.

use super::{GraphicsMode88, PC88_WIDTH, RenderInputs88};

const BYTES_PER_LINE: usize = PC88_WIDTH / 8;
const LINES_PER_PLANE: usize = 200;
const MAX_COLUMNS: usize = 80;

/// Fills `pens` (640 x `height`) with the graphics-layer pen index per pixel.
/// Pixels left at 0 are background/transparent for the composite pass.
pub fn rasterize(inputs: &RenderInputs88<'_>, pens: &mut [u8], height: usize) {
    pens[..PC88_WIDTH * height].fill(0);
    if !inputs.graphics_enabled {
        return;
    }
    match inputs.graphics_mode {
        GraphicsMode88::Color8 => rasterize_color8(inputs, pens, height),
        GraphicsMode88::Attrib200 => rasterize_attrib(inputs, pens, height, false),
        GraphicsMode88::Attrib400 => rasterize_attrib(inputs, pens, height, true),
    }
}

fn plane_byte(plane: &[u8], address: usize) -> u8 {
    plane.get(address).copied().unwrap_or(0)
}

fn rasterize_color8(inputs: &RenderInputs88<'_>, pens: &mut [u8], height: usize) {
    let lines = height.min(LINES_PER_PLANE);
    for y in 0..lines {
        for column in 0..BYTES_PER_LINE {
            let address = y * BYTES_PER_LINE + column;
            let blue = plane_byte(inputs.gvram_blue, address);
            let red = plane_byte(inputs.gvram_red, address);
            let green = plane_byte(inputs.gvram_green, address);
            for bit in 0..8 {
                let mask = 0x80u8 >> bit;
                let pen = u8::from(blue & mask != 0)
                    | (u8::from(red & mask != 0) << 1)
                    | (u8::from(green & mask != 0) << 2);
                pens[y * PC88_WIDTH + column * 8 + bit] = pen;
            }
        }
    }
}

fn rasterize_attrib(inputs: &RenderInputs88<'_>, pens: &mut [u8], height: usize, line_400: bool) {
    let char_height = inputs.char_height.max(1) as usize;
    let disable_blue = inputs.plane_disable & 0x01 != 0;
    let disable_red = inputs.plane_disable & 0x02 != 0;
    let disable_green = inputs.plane_disable & 0x04 != 0;
    let lines = if line_400 { 400 } else { LINES_PER_PLANE }.min(height);

    for y in 0..lines {
        let text_line = if line_400 { y >> 1 } else { y };
        let text_row = text_line / char_height;
        for column in 0..BYTES_PER_LINE {
            let mut bits = if line_400 {
                let address = (y % LINES_PER_PLANE) * BYTES_PER_LINE + column;
                if y < LINES_PER_PLANE {
                    if disable_blue {
                        0
                    } else {
                        plane_byte(inputs.gvram_blue, address)
                    }
                } else if disable_red {
                    0
                } else {
                    plane_byte(inputs.gvram_red, address)
                }
            } else {
                let address = y * BYTES_PER_LINE + column;
                let mut value = 0u8;
                if !disable_blue {
                    value |= plane_byte(inputs.gvram_blue, address);
                }
                if !disable_red {
                    value |= plane_byte(inputs.gvram_red, address);
                }
                if !disable_green {
                    value |= plane_byte(inputs.gvram_green, address);
                }
                value
            };

            let cell = text_row * MAX_COLUMNS + column;
            let attrib = inputs.text_attrib.get(cell).copied().unwrap_or(0);
            let color = (attrib >> 5) & 7;
            if attrib & 0x01 != 0 {
                bits ^= 0xFF;
            }

            for bit in 0..8 {
                let mask = 0x80u8 >> bit;
                if bits & mask != 0 {
                    pens[y * PC88_WIDTH + column * 8 + bit] = color;
                }
            }
        }
    }
}
