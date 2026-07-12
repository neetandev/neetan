//! CGA compatibility rasterizers: 4-color interleaved (modes 04h/05h) and
//! one bit per pixel (mode 06h).
//!
//! Both modes address display memory through the CGA scanline interleave: the
//! CRTC mode control substitutes row scan bit 0 into address bit 13, so odd
//! scanlines come from the 8 KiB half at plane address 0x2000. The 4-color
//! mode reads through the chained odd/even layout (even host bytes in plane
//! 0, odd host bytes in plane 1 at the same plane address).

use super::{
    FrameLayout, LINE_BUFFER_DOTS, RenderInputsVga, VgaRenderer, effective_pel_pan, line_pan,
    line_position, line_row_base, plane_address_mask, substitute_row_scan,
};

/// Pixels decoded from one byte in the 4-color mode.
const CGA4_PIXELS_PER_BYTE: u32 = 4;
/// Pixels decoded from one byte in the 2-color mode.
const CGA2_PIXELS_PER_BYTE: u32 = 8;

impl VgaRenderer {
    /// Rasters the CGA 4-color interleaved screen into the framebuffer.
    pub(super) fn render_cga(&mut self, inputs: &RenderInputsVga, layout: &FrameLayout) {
        let plane_mask = plane_address_mask(inputs);
        let pan = effective_pel_pan(inputs.pel_pan, 8);

        for y in 0..layout.content_height {
            let position = line_position(inputs, y);
            let row_base = line_row_base(inputs, &position);
            let pan = line_pan(inputs, &position, pan);
            let dots = (layout.content_width + pan).min(LINE_BUFFER_DOTS as u32);
            let bytes = dots.div_ceil(CGA4_PIXELS_PER_BYTE);
            for byte_index in 0..bytes {
                // Host byte addresses interleave the odd/even planes: the low
                // bit selects the plane at the even plane address.
                let host_address =
                    substitute_row_scan(inputs, row_base + byte_index, position.row_scan);
                let plane_address = (host_address & !1) & plane_mask;
                let plane = host_address & 1;
                let byte = inputs.vram[plane_address as usize * 4 + plane as usize];
                let base = (byte_index * CGA4_PIXELS_PER_BYTE) as usize;
                for pixel in 0..CGA4_PIXELS_PER_BYTE as usize {
                    let value = (byte >> (6 - 2 * pixel)) & 0x03;
                    self.line_buffer[base + pixel] = inputs.pens[usize::from(value)];
                }
            }
            self.commit_scanline(y, layout, pan);
        }
    }

    /// Rasters the CGA 2-color screen from plane zero into the framebuffer.
    pub(super) fn render_mono(&mut self, inputs: &RenderInputsVga, layout: &FrameLayout) {
        let plane_mask = plane_address_mask(inputs);
        let pan = effective_pel_pan(inputs.pel_pan, 8);

        for y in 0..layout.content_height {
            let position = line_position(inputs, y);
            let row_base = line_row_base(inputs, &position);
            let pan = line_pan(inputs, &position, pan);
            let dots = (layout.content_width + pan).min(LINE_BUFFER_DOTS as u32);
            let bytes = dots.div_ceil(CGA2_PIXELS_PER_BYTE);
            for byte_index in 0..bytes {
                let address = substitute_row_scan(
                    inputs,
                    row_base + byte_index * inputs.address_step,
                    position.row_scan,
                ) & plane_mask;
                let byte = inputs.vram[address as usize * 4];
                let base = (byte_index * CGA2_PIXELS_PER_BYTE) as usize;
                for pixel in 0..CGA2_PIXELS_PER_BYTE as usize {
                    let value = (byte >> (7 - pixel)) & 0x01;
                    self.line_buffer[base + pixel] = inputs.pens[usize::from(value)];
                }
            }
            self.commit_scanline(y, layout, pan);
        }
    }
}
