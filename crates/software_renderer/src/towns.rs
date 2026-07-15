//! FM Towns display renderer.
//!
//! Composes the two CRTC layers over a black backdrop into a packed RGBA
//! framebuffer. In two-page mode the non-priority page is drawn opaque and the
//! priority page transparent on top; in single-page mode one full-VRAM layer is
//! drawn opaque using the interleaved bank transform.

mod compose;
mod palette;
mod sprite;

use alloc::{boxed::Box, vec};

pub use common::{HighResCursor, TownsLayer};
pub use palette::{towns_color_to_rgba, towns_color15_to_rgba};
pub use sprite::{SpriteRenderParams, TOWNS_SPRITE_LAYER_VRAM_OFFSET, render_sprites};

/// FM Towns visible surface width in pixels. Sized for the MX high-resolution
/// 1024x768 mode; lower-resolution frames occupy the top-left sub-rectangle.
pub const TOWNS_SURFACE_WIDTH: usize = 1024;
/// FM Towns visible surface maximum height in pixels.
pub const TOWNS_SURFACE_HEIGHT: usize = 768;
/// Bytes per pixel (`R, G, B, A`).
pub const TOWNS_PIXEL_BYTES: usize = 4;
/// FM Towns framebuffer byte size.
pub const TOWNS_FRAMEBUFFER_BYTES: usize =
    TOWNS_SURFACE_WIDTH * TOWNS_SURFACE_HEIGHT * TOWNS_PIXEL_BYTES;

/// Opaque black backdrop, packed RGBA.
const BACKDROP: u32 = 0xFF00_0000;

/// Per-frame inputs to the FM Towns renderer.
pub struct RenderInputsTowns<'a> {
    /// Native VRAM image (1 MiB).
    pub vram: &'a [u8],
    /// Single-page mode: one full-VRAM layer with the interleaved transform.
    pub single_page: bool,
    /// Priority (front) page index in two-page mode.
    pub priority_page: usize,
    /// The two display layers.
    pub layers: [TownsLayer; 2],
    /// The two 16-color analog palettes, pre-converted to RGBA.
    pub palette_16: [[u32; 16]; 2],
    /// The 256-color analog palette, pre-converted to RGBA.
    pub palette_256: [u32; 256],
    /// Valid display width in pixels.
    pub width: u32,
    /// Valid display height in pixels.
    pub height: u32,
    /// Whether the high-resolution CRTC is driving this frame (selects the
    /// high-res single-page VRAM interleave and the mouse-cursor overlay).
    pub high_res: bool,
    /// The hardware mouse cursor, composited last when present.
    pub mouse_cursor: Option<HighResCursor>,
}

/// Persistent buffers owned by the renderer.
pub struct TownsRendererState {
    /// Packed RGBA framebuffer (`R, G, B, A` little-endian).
    pub framebuffer: Box<[u8]>,
}

save_state::runtime_state! {
/// Authoritative persistent FM Towns renderer buffers.
#[derive(Clone)]
pub struct TownsRendererRuntimeState {
    framebuffer: Box<[u8]>,
    last_frame_width: usize,
}}

/// CPU-side renderer for the FM Towns display.
pub struct TownsRenderer {
    /// Embedded state for save/restore.
    pub state: TownsRendererState,
    /// Pixel width of the most recently composed frame. The framebuffer is
    /// packed at this stride, so consumers read `width * height * 4` bytes.
    last_frame_width: usize,
}

impl Default for TownsRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl TownsRenderer {
    /// Creates a renderer with a cleared framebuffer.
    pub fn new() -> Self {
        Self {
            state: TownsRendererState {
                framebuffer: vec![0u8; TOWNS_FRAMEBUFFER_BYTES].into_boxed_slice(),
            },
            last_frame_width: 0,
        }
    }

    /// Captures the presented framebuffer and its packed row stride.
    pub fn capture_state(&self) -> TownsRendererRuntimeState {
        TownsRendererRuntimeState {
            framebuffer: self.state.framebuffer.clone(),
            last_frame_width: self.last_frame_width,
        }
    }

    /// Restores the presented framebuffer and its packed row stride.
    pub fn restore_state(
        &mut self,
        state: TownsRendererRuntimeState,
    ) -> Result<(), save_state::StateValidationError> {
        if state.framebuffer.len() != self.state.framebuffer.len()
            || state.last_frame_width > TOWNS_SURFACE_WIDTH
        {
            return Err(save_state::StateValidationError::new(
                "FM Towns renderer state is invalid",
            ));
        }
        self.state.framebuffer = state.framebuffer;
        self.last_frame_width = state.last_frame_width;
        Ok(())
    }

    /// Returns the packed RGBA framebuffer.
    pub fn framebuffer(&self) -> &[u8] {
        &self.state.framebuffer
    }

    /// Renders one frame and returns the `(width, height)` of the valid region.
    pub fn render(&mut self, inputs: &RenderInputsTowns<'_>) -> (u32, u32) {
        let framebuffer = &mut self.state.framebuffer;
        for pixel in framebuffer.chunks_exact_mut(TOWNS_PIXEL_BYTES) {
            pixel[0] = BACKDROP as u8;
            pixel[1] = (BACKDROP >> 8) as u8;
            pixel[2] = (BACKDROP >> 16) as u8;
            pixel[3] = (BACKDROP >> 24) as u8;
        }

        let width = inputs.width.min(TOWNS_SURFACE_WIDTH as u32).max(1);
        let height = inputs.height.min(TOWNS_SURFACE_HEIGHT as u32).max(1);
        let frame_width = width as usize;
        let frame_height = height as usize;

        if inputs.single_page {
            compose::draw_layer(
                framebuffer,
                inputs,
                &inputs.layers[0],
                frame_width,
                frame_height,
                false,
            );
        } else {
            let priority = inputs.priority_page & 1;
            let background = priority ^ 1;
            compose::draw_layer(
                framebuffer,
                inputs,
                &inputs.layers[background],
                frame_width,
                frame_height,
                false,
            );
            compose::draw_layer(
                framebuffer,
                inputs,
                &inputs.layers[priority],
                frame_width,
                frame_height,
                true,
            );
        }

        // The cursor is only ever supplied in high-res mode.
        if let Some(cursor) = inputs.mouse_cursor {
            composite_high_res_cursor(framebuffer, &cursor, frame_width, frame_height);
        }

        self.last_frame_width = frame_width;
        (width, height)
    }
}

/// Composites the 64x64 hardware mouse cursor over the finished frame. A set
/// AND-plane bit leaves the pixel untouched; otherwise the OR-plane bit selects
/// white or black. The frame is packed at `width` pixels per row.
fn composite_high_res_cursor(
    framebuffer: &mut [u8],
    cursor: &HighResCursor,
    width: usize,
    height: usize,
) {
    for row in 0..64usize {
        let and_row = &cursor.and_pattern[row * 8..row * 8 + 8];
        let or_row = &cursor.or_pattern[row * 8..row * 8 + 8];
        for col in 0..64usize {
            let byte = col / 8;
            let bit = 0x80u8 >> (col % 8);
            if and_row[byte] & bit != 0 {
                continue;
            }
            let x = cursor
                .x
                .wrapping_add(col as u32)
                .wrapping_sub(cursor.origin_x);
            let y = cursor
                .y
                .wrapping_add(row as u32)
                .wrapping_sub(cursor.origin_y);
            if x as usize >= width || y as usize >= height {
                continue;
            }
            let value = if or_row[byte] & bit != 0 { 0xFF } else { 0x00 };
            let base = (y as usize * width + x as usize) * TOWNS_PIXEL_BYTES;
            framebuffer[base] = value;
            framebuffer[base + 1] = value;
            framebuffer[base + 2] = value;
            framebuffer[base + 3] = 0xFF;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: u32 = 0xFF00_00FF;
    const GREEN: u32 = 0xFF00_FF00;
    const BLUE: u32 = 0xFFFF_0000;

    fn pixel(renderer: &TownsRenderer, x: usize, y: usize) -> u32 {
        let base = (y * renderer.last_frame_width + x) * TOWNS_PIXEL_BYTES;
        let bytes = &renderer.framebuffer()[base..base + 4];
        u32::from(bytes[0])
            | (u32::from(bytes[1]) << 8)
            | (u32::from(bytes[2]) << 16)
            | (u32::from(bytes[3]) << 24)
    }

    fn base_layer() -> TownsLayer {
        TownsLayer {
            shown: true,
            bits_per_pixel: 4,
            vram_addr: 0,
            bytes_per_line: 320,
            scroll_offset: 0,
            h_scroll_mask: usize::MAX,
            v_scroll_mask: 0x0007_FFFF,
            vram_h_skip_bytes: 0,
            width: 2,
            height: 1,
            origin_x: 0,
            origin_y: 0,
            zoom_x: 2,
            zoom_y: 2,
            plane_mask: 0x0F,
            palette_bank: 0,
            high_res_rgb_swap: 0,
        }
    }

    fn palettes16(bank0_1: u32, bank0_2: u32, bank1_1: u32) -> [[u32; 16]; 2] {
        let mut palettes = [[0u32; 16]; 2];
        palettes[0][1] = bank0_1;
        palettes[0][2] = bank0_2;
        palettes[1][1] = bank1_1;
        palettes
    }

    #[test]
    fn decodes_16_color_packed_pixels() {
        let mut vram = vec![0u8; 0x10_0000];
        vram[0] = 0x21; // low nibble 1 (left pixel), high nibble 2 (right pixel)
        let inputs = RenderInputsTowns {
            vram: &vram,
            single_page: true,
            high_res: false,
            mouse_cursor: None,
            priority_page: 0,
            layers: [base_layer(), TownsLayer::default()],
            palette_16: palettes16(RED, GREEN, 0),
            palette_256: [0; 256],
            width: 2,
            height: 1,
        };
        let mut renderer = TownsRenderer::new();
        renderer.render(&inputs);
        assert_eq!(pixel(&renderer, 0, 0), RED);
        assert_eq!(pixel(&renderer, 1, 0), GREEN);
    }

    #[test]
    fn plane_mask_clears_masked_bits() {
        let mut vram = vec![0u8; 0x10_0000];
        vram[0] = 0x30; // left pixel index 0, right pixel index 3
        let mut layer = base_layer();
        layer.plane_mask = 0x01; // keep only plane 0
        let mut palettes = [[0u32; 16]; 2];
        palettes[0][1] = RED; // index 3 & 0x01 == 1 -> red
        let inputs = RenderInputsTowns {
            vram: &vram,
            single_page: true,
            high_res: false,
            mouse_cursor: None,
            priority_page: 0,
            layers: [layer, TownsLayer::default()],
            palette_16: palettes,
            palette_256: [0; 256],
            width: 2,
            height: 1,
        };
        let mut renderer = TownsRenderer::new();
        renderer.render(&inputs);
        assert_eq!(pixel(&renderer, 1, 0), RED);
    }

    #[test]
    fn decodes_256_color_pixel() {
        let mut vram = vec![0u8; 0x10_0000];
        vram[0] = 5;
        let mut layer = base_layer();
        layer.bits_per_pixel = 8;
        layer.width = 1;
        let mut palette_256 = [0u32; 256];
        palette_256[5] = BLUE;
        let inputs = RenderInputsTowns {
            vram: &vram,
            single_page: true,
            high_res: false,
            mouse_cursor: None,
            priority_page: 0,
            layers: [layer, TownsLayer::default()],
            palette_16: [[0u32; 16]; 2],
            palette_256,
            width: 1,
            height: 1,
        };
        let mut renderer = TownsRenderer::new();
        renderer.render(&inputs);
        assert_eq!(pixel(&renderer, 0, 0), BLUE);
    }

    #[test]
    fn decodes_32768_color_pixel() {
        let mut vram = vec![0u8; 0x10_0000];
        // Pure red 5-5-5 (0x03E0), little-endian.
        vram[0] = 0xE0;
        vram[1] = 0x03;
        let mut layer = base_layer();
        layer.bits_per_pixel = 16;
        layer.width = 1;
        let inputs = RenderInputsTowns {
            vram: &vram,
            single_page: true,
            high_res: false,
            mouse_cursor: None,
            priority_page: 0,
            layers: [layer, TownsLayer::default()],
            palette_16: [[0u32; 16]; 2],
            palette_256: [0; 256],
            width: 1,
            height: 1,
        };
        let mut renderer = TownsRenderer::new();
        renderer.render(&inputs);
        assert_eq!(pixel(&renderer, 0, 0), RED);
    }

    #[test]
    fn priority_page_transparency_shows_lower_page() {
        let mut vram = vec![0u8; 0x10_0000];
        vram[0] = 0x22; // page 0: both pixels index 2
        vram[0x0004_0000] = 0x10; // page 1: left index 0 (transparent), right index 1
        let mut background = base_layer();
        background.vram_addr = 0;
        background.palette_bank = 0;
        background.v_scroll_mask = 0x0003_FFFF;
        let mut foreground = base_layer();
        foreground.vram_addr = 0x0004_0000;
        foreground.palette_bank = 1;
        foreground.v_scroll_mask = 0x0003_FFFF;
        let inputs = RenderInputsTowns {
            vram: &vram,
            single_page: false,
            high_res: false,
            mouse_cursor: None,
            priority_page: 1,
            layers: [background, foreground],
            palette_16: palettes16(0, GREEN, RED),
            palette_256: [0; 256],
            width: 2,
            height: 1,
        };
        let mut renderer = TownsRenderer::new();
        renderer.render(&inputs);
        // Left pixel: page 1 index 0 is transparent, so page 0 (green) shows.
        assert_eq!(pixel(&renderer, 0, 0), GREEN);
        // Right pixel: page 1 index 1 (red) wins.
        assert_eq!(pixel(&renderer, 1, 0), RED);
    }

    #[test]
    fn vertical_scroll_wraps_within_layer_page() {
        let mut vram = vec![0u8; 0x10_0000];
        // Pure red 5-5-5 at the top of page 0.
        vram[0] = 0xE0;
        vram[1] = 0x03;
        // Pure green 5-5-5 at the start of the sprite page.
        vram[0x0004_0000] = 0x00;
        vram[0x0004_0001] = 0x7C;
        // Pure blue 5-5-5 at the last line of page 0.
        vram[0x0003_FE00] = 0x1F;
        vram[0x0003_FE01] = 0x00;
        let mut layer = base_layer();
        layer.bits_per_pixel = 16;
        layer.bytes_per_line = 512;
        layer.h_scroll_mask = 511;
        layer.v_scroll_mask = 0x0003_FFFF;
        layer.scroll_offset = 0x0003_FE00;
        layer.width = 1;
        layer.height = 2;
        let inputs = RenderInputsTowns {
            vram: &vram,
            single_page: false,
            high_res: false,
            mouse_cursor: None,
            priority_page: 1,
            layers: [layer, TownsLayer::default()],
            palette_16: [[0u32; 16]; 2],
            palette_256: [0; 256],
            width: 1,
            height: 2,
        };
        let mut renderer = TownsRenderer::new();
        renderer.render(&inputs);
        // Row 0 samples the last line of page 0.
        assert_eq!(pixel(&renderer, 0, 0), BLUE);
        // Row 1 wraps to the top of page 0 instead of the sprite page.
        assert_eq!(pixel(&renderer, 0, 1), RED);
    }

    #[test]
    fn horizontal_scroll_wraps_within_line() {
        let mut vram = vec![0u8; 0x10_0000];
        vram[0] = 5;
        vram[512] = 7;
        let mut layer = base_layer();
        layer.bits_per_pixel = 8;
        layer.bytes_per_line = 512;
        layer.h_scroll_mask = 511;
        layer.v_scroll_mask = 0x0003_FFFF;
        layer.scroll_offset = 510;
        layer.width = 4;
        let mut palette_256 = [0u32; 256];
        palette_256[5] = BLUE;
        palette_256[7] = RED;
        let inputs = RenderInputsTowns {
            vram: &vram,
            single_page: false,
            high_res: false,
            mouse_cursor: None,
            priority_page: 1,
            layers: [layer, TownsLayer::default()],
            palette_16: [[0u32; 16]; 2],
            palette_256,
            width: 4,
            height: 1,
        };
        let mut renderer = TownsRenderer::new();
        renderer.render(&inputs);
        // Pixel 2 wraps to the start of the same line, not into the next one.
        assert_eq!(pixel(&renderer, 2, 0), BLUE);
    }

    #[test]
    fn line_end_clamp_leaves_backdrop() {
        let mut vram = vec![0u8; 0x10_0000];
        for byte in &mut vram[0..8] {
            *byte = 1;
        }
        let mut layer = base_layer();
        layer.bits_per_pixel = 8;
        layer.bytes_per_line = 4;
        layer.h_scroll_mask = 3;
        layer.v_scroll_mask = 0x0003_FFFF;
        layer.width = 8;
        let mut palette_256 = [0u32; 256];
        palette_256[1] = RED;
        let inputs = RenderInputsTowns {
            vram: &vram,
            single_page: false,
            high_res: false,
            mouse_cursor: None,
            priority_page: 1,
            layers: [layer, TownsLayer::default()],
            palette_16: [[0u32; 16]; 2],
            palette_256,
            width: 8,
            height: 1,
        };
        let mut renderer = TownsRenderer::new();
        renderer.render(&inputs);
        assert_eq!(pixel(&renderer, 3, 0), RED);
        assert_eq!(pixel(&renderer, 4, 0), BACKDROP);
    }

    #[test]
    fn line_end_clamp_16bpp_extends_by_origin() {
        let mut vram = vec![0u8; 0x10_0000];
        // Pure red 5-5-5 in the two bytes past the programmed line length.
        vram[6] = 0xE0;
        vram[7] = 0x03;
        let mut layer = base_layer();
        layer.bits_per_pixel = 16;
        layer.bytes_per_line = 6;
        layer.v_scroll_mask = 0x0003_FFFF;
        layer.origin_x = 1;
        layer.width = 4;
        let inputs = RenderInputsTowns {
            vram: &vram,
            single_page: false,
            high_res: false,
            mouse_cursor: None,
            priority_page: 1,
            layers: [layer, TownsLayer::default()],
            palette_16: [[0u32; 16]; 2],
            palette_256: [0; 256],
            width: 8,
            height: 1,
        };
        let mut renderer = TownsRenderer::new();
        renderer.render(&inputs);
        // The fetch window extends by twice the on-monitor origin, so the
        // fourth pixel (in-line offset 6) is still drawn.
        assert_eq!(pixel(&renderer, 4, 0), RED);
    }

    #[test]
    fn h_skip_bytes_offsets_line_start() {
        let mut vram = vec![0u8; 0x10_0000];
        vram[2] = 1;
        let mut layer = base_layer();
        layer.bits_per_pixel = 8;
        layer.vram_h_skip_bytes = 2;
        layer.width = 1;
        let mut palette_256 = [0u32; 256];
        palette_256[1] = RED;
        let inputs = RenderInputsTowns {
            vram: &vram,
            single_page: false,
            high_res: false,
            mouse_cursor: None,
            priority_page: 1,
            layers: [layer, TownsLayer::default()],
            palette_16: [[0u32; 16]; 2],
            palette_256,
            width: 1,
            height: 1,
        };
        let mut renderer = TownsRenderer::new();
        renderer.render(&inputs);
        assert_eq!(pixel(&renderer, 0, 0), RED);
    }

    #[test]
    fn fractional_zoom_alternates_repeat_widths() {
        let mut vram = vec![0u8; 0x10_0000];
        vram[0] = 1;
        vram[1] = 2;
        let mut layer = base_layer();
        layer.bits_per_pixel = 8;
        layer.zoom_x = 5;
        layer.width = 5;
        let mut palette_256 = [0u32; 256];
        palette_256[1] = RED;
        palette_256[2] = GREEN;
        let inputs = RenderInputsTowns {
            vram: &vram,
            single_page: false,
            high_res: false,
            mouse_cursor: None,
            priority_page: 1,
            layers: [layer, TownsLayer::default()],
            palette_16: [[0u32; 16]; 2],
            palette_256,
            width: 5,
            height: 1,
        };
        let mut renderer = TownsRenderer::new();
        renderer.render(&inputs);
        // 2.5x zoom repeats source pixels 2-wide then 3-wide.
        assert_eq!(pixel(&renderer, 0, 0), RED);
        assert_eq!(pixel(&renderer, 1, 0), RED);
        assert_eq!(pixel(&renderer, 2, 0), GREEN);
        assert_eq!(pixel(&renderer, 3, 0), GREEN);
        assert_eq!(pixel(&renderer, 4, 0), GREEN);
    }

    #[test]
    fn four_bpp_line_start_wraps_with_vertical_mask() {
        let mut vram = vec![0u8; 0x10_0000];
        // Native offset 4 maps to VRAM index 0x40000 through the single-page
        // interleave transform.
        vram[0x0004_0000] = 0x01;
        let mut layer = base_layer();
        layer.bytes_per_line = 8;
        layer.scroll_offset = 0x0007_FFFC;
        layer.width = 1;
        layer.height = 2;
        let inputs = RenderInputsTowns {
            vram: &vram,
            single_page: true,
            high_res: false,
            mouse_cursor: None,
            priority_page: 0,
            layers: [layer, TownsLayer::default()],
            palette_16: palettes16(RED, 0, 0),
            palette_256: [0; 256],
            width: 1,
            height: 2,
        };
        let mut renderer = TownsRenderer::new();
        renderer.render(&inputs);
        // Line 1 starts at (0x7FFFC + 8) & 0x7FFFF == 4 within the page.
        assert_eq!(pixel(&renderer, 0, 1), RED);
    }

    #[test]
    fn horizontal_zoom_replicates_pixels() {
        let mut vram = vec![0u8; 0x10_0000];
        vram[0] = 0x01; // left pixel index 1
        let mut layer = base_layer();
        layer.zoom_x = 4; // 2x
        layer.width = 2;
        let inputs = RenderInputsTowns {
            vram: &vram,
            single_page: true,
            high_res: false,
            mouse_cursor: None,
            priority_page: 0,
            layers: [layer, TownsLayer::default()],
            palette_16: palettes16(RED, 0, 0),
            palette_256: [0; 256],
            width: 2,
            height: 1,
        };
        let mut renderer = TownsRenderer::new();
        renderer.render(&inputs);
        // Both output columns sample source pixel 0 under 2x zoom.
        assert_eq!(pixel(&renderer, 0, 0), RED);
        assert_eq!(pixel(&renderer, 1, 0), RED);
    }

    fn draw_24bpp(swap: u8) -> TownsRenderer {
        let mut vram = vec![0u8; 0x10_0000];
        vram[0..6].copy_from_slice(&[0x11, 0x22, 0x33, 0x44, 0x55, 0x66]);
        let mut layer = base_layer();
        layer.bits_per_pixel = 24;
        layer.width = 2;
        layer.bytes_per_line = 6;
        layer.high_res_rgb_swap = swap;
        let inputs = RenderInputsTowns {
            vram: &vram,
            single_page: false,
            high_res: true,
            mouse_cursor: None,
            priority_page: 0,
            layers: [layer, TownsLayer::default()],
            palette_16: [[0u32; 16]; 2],
            palette_256: [0; 256],
            width: 2,
            height: 1,
        };
        let mut renderer = TownsRenderer::new();
        renderer.render(&inputs);
        renderer
    }

    #[test]
    fn decodes_24bpp_identity_order() {
        let renderer = draw_24bpp(0x06);
        assert_eq!(
            pixel(&renderer, 0, 0),
            towns_color_to_rgba(0x11, 0x22, 0x33)
        );
        assert_eq!(
            pixel(&renderer, 1, 0),
            towns_color_to_rgba(0x44, 0x55, 0x66)
        );
    }

    #[test]
    fn decodes_24bpp_reversed_order() {
        // Swap 0x24: R from source byte 2, G from byte 1, B from byte 0.
        let renderer = draw_24bpp(0x24);
        assert_eq!(
            pixel(&renderer, 0, 0),
            towns_color_to_rgba(0x33, 0x22, 0x11)
        );
        assert_eq!(
            pixel(&renderer, 1, 0),
            towns_color_to_rgba(0x66, 0x55, 0x44)
        );
    }

    #[test]
    fn high_res_mouse_cursor_paints_white_and_leaves_transparent() {
        let vram = vec![0u8; 0x10_0000];
        let mut and_pattern = [0xFFu8; 512];
        let mut or_pattern = [0u8; 512];
        // Row 0, column 0: AND bit clear (draw), OR bit set (white). Column 1
        // keeps its AND bit set, so it stays transparent.
        and_pattern[0] = 0x7F;
        or_pattern[0] = 0x80;
        let cursor = HighResCursor {
            x: 1,
            y: 1,
            origin_x: 0,
            origin_y: 0,
            and_pattern,
            or_pattern,
        };
        let inputs = RenderInputsTowns {
            vram: &vram,
            single_page: true,
            high_res: true,
            mouse_cursor: Some(cursor),
            priority_page: 0,
            layers: [TownsLayer::default(), TownsLayer::default()],
            palette_16: [[0u32; 16]; 2],
            palette_256: [0; 256],
            width: 4,
            height: 4,
        };
        let mut renderer = TownsRenderer::new();
        renderer.render(&inputs);
        assert_eq!(pixel(&renderer, 1, 1), 0xFFFF_FFFF);
        assert_eq!(pixel(&renderer, 2, 1), BACKDROP);
        assert_eq!(pixel(&renderer, 0, 0), BACKDROP);
    }
}
