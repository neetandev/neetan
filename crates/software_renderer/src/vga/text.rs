//! VGA text mode rasterizer.

use super::{
    FrameLayout, RenderInputsVga, VgaRenderer, effective_pel_pan, line_pan, line_position,
    line_row_base, plane_address_mask, substitute_row_scan,
};

/// Size of one glyph in plane 2, in bytes.
const GLYPH_STRIDE: u32 = 32;

impl VgaRenderer {
    /// Rasters the text screen row by row into the framebuffer.
    pub(super) fn render_text(&mut self, inputs: &RenderInputsVga, layout: &FrameLayout) {
        let plane_mask = plane_address_mask(inputs);
        let pan = effective_pel_pan(inputs.pel_pan, inputs.character_width);
        let cursor_address = inputs.cursor_address & plane_mask;

        for y in 0..layout.content_height {
            let position = line_position(inputs, y);
            let row_base = line_row_base(inputs, &position);
            let cell_row = position.row_scan;
            let cursor_row_active = inputs.cursor_visible
                && cell_row >= u32::from(inputs.cursor_start_row)
                && cell_row <= u32::from(inputs.cursor_end_row);

            for column in 0..inputs.columns {
                let cell_address = substitute_row_scan(
                    inputs,
                    row_base + column * inputs.address_step,
                    position.row_scan,
                ) & plane_mask;
                let character = inputs.vram[cell_address as usize * 4];
                let attribute = inputs.vram[cell_address as usize * 4 + 1];
                let cell_start = (column * inputs.character_width) as usize;

                if cursor_row_active && cell_address == cursor_address {
                    // The cursor scanlines show a solid foreground block.
                    let pen = inputs.pens[usize::from(attribute & 0x0F)];
                    for dot in 0..inputs.character_width as usize {
                        self.line_buffer[cell_start + dot] = pen;
                    }
                    continue;
                }

                let font_offset = if attribute & 0x08 != 0 {
                    inputs.font_offset_map_a
                } else {
                    inputs.font_offset_map_b
                };
                let glyph_address =
                    (font_offset + u32::from(character) * GLYPH_STRIDE + cell_row) & plane_mask;
                let glyph_row = inputs.vram[glyph_address as usize * 4 + 2];

                let mut foreground = usize::from(attribute & 0x0F);
                let background;
                if inputs.blink_enabled {
                    background = usize::from((attribute >> 4) & 0x07);
                    if attribute & 0x80 != 0 && !inputs.blink_visible {
                        foreground = background;
                    }
                } else {
                    background = usize::from(attribute >> 4);
                }

                let line_graphics_character =
                    inputs.line_graphics && (0xC0..=0xDF).contains(&character);
                for dot in 0..inputs.character_width {
                    let lit = if dot < 8 {
                        glyph_row & (0x80 >> dot) != 0
                    } else {
                        line_graphics_character && glyph_row & 0x01 != 0
                    };
                    let pen = inputs.pens[if lit { foreground } else { background }];
                    self.line_buffer[cell_start + dot as usize] = pen;
                }
            }
            self.commit_scanline(y, layout, line_pan(inputs, &position, pan));
        }
    }
}
