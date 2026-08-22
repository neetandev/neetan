//! PC-6001mkII extended video modes: a 160x200 four-color bitmap, a 320x200
//! 2bpp bitmap, and a 40x20 text mode with a sixteen-color palette. When none
//! of the extended modes is active the legacy MC6847 picture is drawn centered
//! inside the 320x240 canvas with a black border.

use crate::pc60::{
    MK2_PALETTE, PC60_HEIGHT, PC60_MK2_HEIGHT, PC60_MK2_WIDTH, PC60_WIDTH, RenderInputs60, base,
    put_pixel,
};

/// Active picture height for the extended bitmap modes.
const BITMAP_HEIGHT: usize = 200;
/// Width of the extended text grid in cells.
const TEXT_COLUMNS: usize = 40;
/// Height of the extended text grid in cells.
const TEXT_ROWS: usize = 20;
/// Character cell width in pixels.
const CELL_WIDTH: usize = 8;
/// Character cell height in pixels.
const CELL_HEIGHT: usize = 12;
/// Visible bitmap rows in the mkII extended text font.
const TEXT_GLYPH_HEIGHT: usize = 10;
/// Origin of the legacy picture inside the mkII canvas.
const LEGACY_ORIGIN_X: usize = (PC60_MK2_WIDTH - PC60_WIDTH) / 2;
const LEGACY_ORIGIN_Y: usize = (PC60_MK2_HEIGHT - PC60_HEIGHT) / 2;

fn put(framebuffer: &mut [u8], x: usize, y: usize, color: usize) {
    put_pixel(
        framebuffer,
        PC60_MK2_WIDTH,
        PC60_MK2_HEIGHT,
        x,
        y,
        MK2_PALETTE[color & 0x1F],
    );
}

/// Renders one mkII frame into the 320x240 `framebuffer`.
pub(crate) fn render(inputs: &RenderInputs60, framebuffer: &mut [u8]) {
    fill_black(framebuffer);

    if inputs.exgfx_bitmap {
        draw_bitmap(inputs, framebuffer);
    } else if inputs.exgfx_2bpp {
        draw_2bpp(inputs, framebuffer);
    } else if inputs.exgfx_text {
        draw_text(inputs, framebuffer);
    } else {
        let mut target = base::Target::new(
            framebuffer,
            PC60_MK2_WIDTH,
            PC60_MK2_HEIGHT,
            LEGACY_ORIGIN_X,
            LEGACY_ORIGIN_Y,
        );
        base::render(inputs, &mut target);
    }
}

fn fill_black(framebuffer: &mut [u8]) {
    for pixel in framebuffer.as_chunks_mut::<4>().0 {
        pixel.copy_from_slice(&MK2_PALETTE[0]);
    }
}

fn plane(vram: &[u8], index: usize) -> u8 {
    *vram.get(index).unwrap_or(&0)
}

fn draw_bitmap(inputs: &RenderInputs60, framebuffer: &mut [u8]) {
    let vram = inputs.vram;
    let mut count = 0;
    for y in 0..BITMAP_HEIGHT {
        for x in (0..160).step_by(4) {
            for i in 0..4 {
                let shift = 6 - i * 2;
                let pen0 = (plane(vram, count) >> shift) & 3;
                let pen1 = (plane(vram, count + 0x2000) >> shift) & 3;
                let color = 0x10
                    | (((pen0 & 1) as usize) << 2)
                    | (((pen0 & 2) as usize) >> 1)
                    | (((pen1 & 1) as usize) << 1)
                    | (((pen1 & 2) as usize) << 2);
                put(framebuffer, (x + i) * 2, y, color);
                put(framebuffer, (x + i) * 2 + 1, y, color);
            }
            count += 1;
        }
    }
}

fn draw_2bpp(inputs: &RenderInputs60, framebuffer: &mut [u8]) {
    let vram = inputs.vram;
    let bgcol_bank = inputs.bgcol_bank as usize;
    let mut count = 0;
    for y in 0..BITMAP_HEIGHT {
        for x in (0..PC60_MK2_WIDTH).step_by(8) {
            for i in 0..8 {
                let shift = 7 - i;
                let pen0 = ((plane(vram, count) >> shift) & 1) as usize;
                let pen1 = ((plane(vram, count + 0x2000) >> shift) & 1) as usize;
                let color = if bgcol_bank & 4 != 0 {
                    0x08 | pen0 | (pen1 << 1) | ((bgcol_bank & 1) << 2)
                } else {
                    0x10 | (pen0 << 2) | pen1 | ((bgcol_bank & 1) << 1) | ((bgcol_bank & 2) << 2)
                };
                put(framebuffer, x + i, y, color);
            }
            count += 1;
        }
    }
}

fn draw_text(inputs: &RenderInputs60, framebuffer: &mut [u8]) {
    let vram = inputs.vram;
    let bgcol_bank = inputs.bgcol_bank as usize;
    for y in 0..TEXT_ROWS {
        for x in 0..TEXT_COLUMNS {
            let cell = x + y * TEXT_COLUMNS;
            let attr = plane(vram, cell & 0x3FF) as usize;
            let mut tile = plane(vram, cell + 0x400) as usize + 0x200;
            tile += (attr & 0x80) << 1;

            let fgcol = (attr & 0x0F) + 0x10;
            let bgcol = ((attr & 0x70) >> 4) + 0x10 + ((bgcol_bank & 2) << 2);

            for yi in 0..CELL_HEIGHT {
                let glyph_row = if yi < TEXT_GLYPH_HEIGHT {
                    *inputs.cgrom.get(tile * 0x10 + yi).unwrap_or(&0)
                } else {
                    0
                };
                for xi in 0..CELL_WIDTH {
                    let pen = (glyph_row >> (7 - xi)) & 1;
                    let color = if pen != 0 { fgcol } else { bgcol };
                    put(
                        framebuffer,
                        x * CELL_WIDTH + xi,
                        y * CELL_HEIGHT + yi,
                        color,
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;
    use crate::pc60::{PC60_MK2_FRAMEBUFFER_BYTES, Pc60RenderModel};

    fn extended_text_inputs<'a>(vram: &'a [u8], cgrom: &'a [u8]) -> RenderInputs60<'a> {
        RenderInputs60 {
            model: Pc60RenderModel::Mk2,
            vram,
            cgrom,
            exgfx_bitmap: false,
            exgfx_2bpp: false,
            exgfx_text: true,
            bgcol_bank: 0,
        }
    }

    #[test]
    fn extended_text_ignores_padding_rows_in_the_font_rom() {
        let mut vram = vec![0u8; 0x800];
        vram[0] = 0x0F;
        vram[0x400] = 0x41;

        let mut cgrom = vec![0u8; 0x4000];
        let glyph = (0x200 + 0x41) * 0x10;
        cgrom[glyph] = 0x80;
        cgrom[glyph + 10] = 0xFF;
        cgrom[glyph + 11] = 0xFF;

        let inputs = extended_text_inputs(&vram, &cgrom);
        let mut framebuffer = vec![0u8; PC60_MK2_FRAMEBUFFER_BYTES];
        render(&inputs, &mut framebuffer);

        assert_eq!(&framebuffer[0..4], &MK2_PALETTE[0x1F]);
        let padding_offset = (10 * PC60_MK2_WIDTH) * 4;
        assert_eq!(
            &framebuffer[padding_offset..padding_offset + 4],
            &MK2_PALETTE[0x10]
        );
    }
}
