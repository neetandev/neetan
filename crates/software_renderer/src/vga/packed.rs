//! VGA 256-color packed pixel rasterizer (mode 13h, Mode X and the ET4000
//! SVGA modes).
//!
//! The scan-out fetches all four planes of consecutive plane addresses, which
//! in the dword-interleaved display memory is a linear byte walk. Chain-4
//! (mode 13h), unchained Mode X and the SVGA modes therefore share one
//! rasterizer; they differ only in the pre-scaled start address, row pitch and
//! the pixel rate. The CGA row scan address substitutions do not apply here
//! (the packed mode registers never select them).

use super::{
    FrameLayout, LINE_BUFFER_DOTS, RenderInputsVga, VGA_PIXEL_BYTES, VgaRenderer, line_pan,
    line_position, line_row_base,
};

impl VgaRenderer {
    /// Rasters the packed 256-color screen row by row into the framebuffer.
    pub(super) fn render_packed(&mut self, inputs: &RenderInputsVga, layout: &FrameLayout) {
        let byte_mask = inputs.vram.len() - 1;
        // 256-color panning moves in half pixel steps: only the even register
        // values shift, by one output dot each.
        let pan = u32::from(inputs.pel_pan & 0x06);

        for y in 0..layout.content_height {
            let position = line_position(inputs, y);
            let row_bytes = line_row_base(inputs, &position) as usize * 4;
            let pan = line_pan(inputs, &position, pan);

            // The unpanned common case gathers the palette straight into the
            // framebuffer row, skipping the line buffer copy. It reads the row's
            // palette indices directly from display memory, so it only applies
            // when the row does not wrap the display memory byte mask.
            #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
            if self.has_simd && pan == 0 {
                let content_width = layout.content_width as usize;
                let source_span = if inputs.packed_half_rate {
                    content_width.div_ceil(2)
                } else {
                    content_width
                };
                if row_bytes + source_span <= inputs.vram.len() {
                    let row_start = ((y + layout.border) * layout.total_width + layout.border)
                        as usize
                        * VGA_PIXEL_BYTES;
                    let content_bytes = content_width * VGA_PIXEL_BYTES;
                    let row_fb = &mut self.framebuffer[row_start..row_start + content_bytes];
                    let source = &inputs.vram[row_bytes..];
                    #[allow(unsafe_code)]
                    // SAFETY: `has_simd` was validated at renderer construction
                    // (`is_x86_feature_detected!("avx2")` on x86_64, baseline NEON
                    // on aarch64), and `source` spans the row without wrapping.
                    unsafe {
                        #[cfg(target_arch = "x86_64")]
                        super::avx2::render_packed_row_avx2(
                            row_fb,
                            source,
                            &inputs.pens_256,
                            inputs.packed_half_rate,
                        );
                        #[cfg(target_arch = "aarch64")]
                        super::neon::render_packed_row_neon(
                            row_fb,
                            source,
                            &inputs.pens_256,
                            inputs.packed_half_rate,
                        );
                    }
                    continue;
                }
            }

            let dots = (layout.content_width + pan).min(LINE_BUFFER_DOTS as u32);
            for dot in 0..dots as usize {
                let source = if inputs.packed_half_rate {
                    dot >> 1
                } else {
                    dot
                };
                let value = inputs.vram[(row_bytes + source) & byte_mask];
                self.line_buffer[dot] = inputs.pens_256[usize::from(value)];
            }
            self.commit_scanline(y, layout, pan);
        }
    }
}
