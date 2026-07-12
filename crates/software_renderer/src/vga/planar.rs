//! VGA 16-color planar graphics rasterizer (EGA and VGA planar modes).

use super::{
    FrameLayout, LINE_BUFFER_DOTS, RenderInputsVga, VgaRenderer, effective_pel_pan, line_pan,
    line_position, line_row_base, plane_address_mask, substitute_row_scan,
};

/// Pixels decoded from one plane byte.
const PIXELS_PER_BYTE: u32 = 8;

impl VgaRenderer {
    /// Rasters the planar screen row by row into the framebuffer.
    pub(super) fn render_planar(&mut self, inputs: &RenderInputsVga, layout: &FrameLayout) {
        let plane_mask = plane_address_mask(inputs);
        let pan = effective_pel_pan(inputs.pel_pan, PIXELS_PER_BYTE);

        for y in 0..layout.content_height {
            let position = line_position(inputs, y);
            let row_base = line_row_base(inputs, &position);
            let pan = line_pan(inputs, &position, pan);
            let dots = (layout.content_width + pan).min(LINE_BUFFER_DOTS as u32);
            let bytes = dots.div_ceil(PIXELS_PER_BYTE);
            for byte_index in 0..bytes {
                let address = substitute_row_scan(
                    inputs,
                    row_base + byte_index * inputs.address_step,
                    position.row_scan,
                ) & plane_mask;
                let address = address as usize * 4;
                let planes: [u8; 4] = inputs.vram[address..address + 4].try_into().unwrap();
                let base = (byte_index * PIXELS_PER_BYTE) as usize;
                for dot in 0..PIXELS_PER_BYTE as usize {
                    let bit = 7 - dot;
                    let color = usize::from(planes[0] >> bit & 1)
                        | usize::from(planes[1] >> bit & 1) << 1
                        | usize::from(planes[2] >> bit & 1) << 2
                        | usize::from(planes[3] >> bit & 1) << 3;
                    self.line_buffer[base + dot] = inputs.pens[color];
                }
            }
            self.commit_scanline(y, layout, pan);
        }
    }
}
