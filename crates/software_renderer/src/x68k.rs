//! Sharp X68000 display renderer.
//!
//! Composes the X68000 display layers into a packed RGBA framebuffer with
//! pixel-precise catch-up: the machine renders all pixels the beam has
//! passed before applying a visible state change, so mid-frame register
//! updates split the frame exactly at the beam position.

mod compose;
mod graphics;
mod palette;
mod sprite;
mod text;

use alloc::{vec, vec::Vec};

use compose::compose_pixel;
use graphics::graphic_codes;
pub use palette::{grbi_to_rgba, mix_averaged_grbi, mix_halved_grbi};
use sprite::rasterize_sprite_line;
use text::text_color_code;

/// Width used before the guest programs valid CRTC timing.
pub const X68K_INITIAL_WIDTH: u32 = 768;
/// Height used before the guest programs valid CRTC timing.
pub const X68K_INITIAL_HEIGHT: u32 = 512;
/// Bytes per pixel (`R, G, B, A`).
pub const X68K_PIXEL_BYTES: usize = 4;
/// Number of entries in each X68000 palette.
pub const X68K_PALETTE_ENTRIES: usize = 256;
/// Number of 16-bit words in the graphics VRAM.
pub const X68K_GVRAM_WORDS: usize = 0x4_0000;
/// Number of sprite scroll-register entries.
pub const X68K_SPRITE_COUNT: usize = 128;
/// Number of 16-bit words in the sprite pattern RAM.
pub const X68K_SPRITE_PATTERN_WORDS: usize = 0x4000;

save_state::runtime_state_enum! {
/// Vertical scan handling for the scanout buffers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanModeX68k {
    /// One work row per published row.
    Progressive = 0,
    /// Two consecutive work rows show the same content line.
    DoubleRead = 1,
    /// Each field weaves into every other row of a double-height frame.
    Interlace = 2,
}}

/// Per-frame inputs to the X68000 renderer.
pub struct RenderInputsX68k<'a> {
    /// Text VRAM image (four 128 KiB planes).
    pub text_vram: &'a [u8],
    /// Graphics VRAM image (512x512 packed 16-bit words).
    pub graphic_vram: &'a [u16],
    /// Text, sprite, and background palette (256 GRBI entries).
    pub text_palette: &'a [u16; X68K_PALETTE_ENTRIES],
    /// Graphics palette (256 GRBI entries).
    pub graphics_palette: &'a [u16; X68K_PALETTE_ENTRIES],
    /// Text horizontal scroll in dots.
    pub text_scroll_x: u16,
    /// Text vertical scroll in rasters.
    pub text_scroll_y: u16,
    /// Graphic horizontal scroll in dots per GVRAM page.
    pub graphic_scroll_x: [u16; 4],
    /// Graphic vertical scroll in rasters per GVRAM page.
    pub graphic_scroll_y: [u16; 4],
    /// Raw CRTC memory-mode register R20.
    pub crtc_memory_mode: u16,
    /// Raw video-controller memory-mode register R0.
    pub memory_mode: u16,
    /// Raw video-controller priority register R1.
    pub priority: u16,
    /// Raw video-controller mixing register R2.
    pub mixing: u16,
    /// Sprite scroll table (X, Y, pattern word, priority per sprite).
    pub sprite_scroll: &'a [[u16; 4]; X68K_SPRITE_COUNT],
    /// Sprite pattern RAM image including the background tile maps.
    pub sprite_pattern: &'a [u16],
    /// Background scroll registers (BG0 X, BG0 Y, BG1 X, BG1 Y).
    pub background_scroll: [u16; 4],
    /// Sprite background control register.
    pub background_control: u16,
    /// Sprite resolution register.
    pub sprite_resolution: u16,
    /// Sprite horizontal back-porch end register.
    pub sprite_horizontal_back_end: u16,
    /// Sprite vertical back-porch end register.
    pub sprite_vertical_back_end: u16,
    /// Raw CRTC horizontal back-porch end register R2.
    pub crtc_horizontal_back_end: u16,
    /// Raw CRTC vertical back-porch end register R6.
    pub crtc_vertical_back_end: u16,
    /// Whether the sprite area is accessible under the CRTC mode.
    pub sprite_area_accessible: bool,
    /// Screen contrast (0-15).
    pub contrast: u8,
    /// Visible width in pixels.
    pub width: u32,
    /// Visible height in pixels.
    pub height: u32,
    /// Current CRTC field parity.
    pub odd_field: bool,
}

/// X68000 scanout renderer with a work and a published framebuffer.
///
/// The work buffer always covers the CRTC raster span; in interlace the
/// published frame is twice as tall and each field weaves into every other
/// row while the previous field's rows are retained.
pub struct X68kRenderer {
    work: Vec<u8>,
    published: Vec<u8>,
    framed: Vec<u8>,
    sprite_line: Vec<u8>,
    sprite_line_raster: Option<usize>,
    width: u32,
    height: u32,
    published_height: u32,
    frame_width: u32,
    frame_published_height: u32,
    offset_x: u32,
    offset_y: u32,
    framing_active: bool,
    scan_mode: ScanModeX68k,
    work_field_odd: bool,
    rendered_pixels: usize,
}

save_state::runtime_state! {
/// Complete X68000 scanout and partial-frame state.
#[derive(Clone)]
pub struct X68kRendererState {
    work: Vec<u8>,
    published: Vec<u8>,
    framed: Vec<u8>,
    sprite_line: Vec<u8>,
    sprite_line_raster: Option<usize>,
    width: u32,
    height: u32,
    published_height: u32,
    frame_width: u32,
    frame_published_height: u32,
    offset_x: u32,
    offset_y: u32,
    framing_active: bool,
    scan_mode: ScanModeX68k,
    work_field_odd: bool,
    rendered_pixels: usize,
}}

impl Default for X68kRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl X68kRenderer {
    /// Creates a renderer publishing an opaque black initial frame.
    pub fn new() -> Self {
        let published = black_frame(X68K_INITIAL_WIDTH, X68K_INITIAL_HEIGHT);
        Self {
            work: published.clone(),
            published,
            framed: Vec::new(),
            sprite_line: vec![0; X68K_INITIAL_WIDTH as usize],
            sprite_line_raster: None,
            width: X68K_INITIAL_WIDTH,
            height: X68K_INITIAL_HEIGHT,
            published_height: X68K_INITIAL_HEIGHT,
            frame_width: X68K_INITIAL_WIDTH,
            frame_published_height: X68K_INITIAL_HEIGHT,
            offset_x: 0,
            offset_y: 0,
            framing_active: false,
            scan_mode: ScanModeX68k::Progressive,
            work_field_odd: false,
            rendered_pixels: 0,
        }
    }

    /// Captures published video and partial raster composition state.
    pub fn capture_state(&self) -> X68kRendererState {
        X68kRendererState {
            work: self.work.clone(),
            published: self.published.clone(),
            framed: self.framed.clone(),
            sprite_line: self.sprite_line.clone(),
            sprite_line_raster: self.sprite_line_raster,
            width: self.width,
            height: self.height,
            published_height: self.published_height,
            frame_width: self.frame_width,
            frame_published_height: self.frame_published_height,
            offset_x: self.offset_x,
            offset_y: self.offset_y,
            framing_active: self.framing_active,
            scan_mode: self.scan_mode,
            work_field_odd: self.work_field_odd,
            rendered_pixels: self.rendered_pixels,
        }
    }

    /// Restores published video and partial raster composition state.
    pub fn restore_state(
        &mut self,
        state: X68kRendererState,
    ) -> Result<(), save_state::StateValidationError> {
        let work_length = state.width as usize * state.height as usize * X68K_PIXEL_BYTES;
        let published_length =
            state.width as usize * state.published_height as usize * X68K_PIXEL_BYTES;
        let framed_length =
            state.frame_width as usize * state.frame_published_height as usize * X68K_PIXEL_BYTES;
        if state.width == 0
            || state.height == 0
            || state.width > 2048
            || state.height > 2048
            || state.published_height > 4096
            || state.frame_width > 2048
            || state.frame_published_height > 4096
            || state.work.len() != work_length
            || state.published.len() != published_length
            || state.sprite_line.len() != state.width as usize
            || state.rendered_pixels > state.width as usize * state.height as usize
            || (state.framing_active && state.framed.len() != framed_length)
        {
            return Err(save_state::StateValidationError::new(
                "X68000 renderer state is invalid",
            ));
        }
        self.work = state.work;
        self.published = state.published;
        self.framed = state.framed;
        self.sprite_line = state.sprite_line;
        self.sprite_line_raster = state.sprite_line_raster;
        self.width = state.width;
        self.height = state.height;
        self.published_height = state.published_height;
        self.frame_width = state.frame_width;
        self.frame_published_height = state.frame_published_height;
        self.offset_x = state.offset_x;
        self.offset_y = state.offset_y;
        self.framing_active = state.framing_active;
        self.scan_mode = state.scan_mode;
        self.work_field_odd = state.work_field_odd;
        self.rendered_pixels = state.rendered_pixels;
        Ok(())
    }

    /// Returns the last completed frame.
    pub fn framebuffer(&self) -> &[u8] {
        if self.framing_active {
            &self.framed
        } else {
            &self.published
        }
    }

    /// Returns the last completed frame dimensions.
    pub const fn dimensions(&self) -> (u32, u32) {
        if self.framing_active {
            (self.frame_width, self.frame_published_height)
        } else {
            (self.width, self.published_height)
        }
    }

    /// Returns the display window's offset inside the reference frame.
    pub const fn frame_offset(&self) -> (u32, u32) {
        (self.offset_x, self.offset_y)
    }

    /// Resizes the scanout buffers when the visible geometry or scan mode
    /// changes.
    pub fn ensure_geometry(&mut self, width: u32, height: u32, scan_mode: ScanModeX68k) {
        if self.width == width && self.height == height && self.scan_mode == scan_mode {
            return;
        }
        self.width = width;
        self.height = height;
        self.scan_mode = scan_mode;
        self.published_height = match scan_mode {
            ScanModeX68k::Progressive | ScanModeX68k::DoubleRead => height,
            ScanModeX68k::Interlace => height * 2,
        };
        self.work = black_frame(width, height);
        self.published = black_frame(width, self.published_height);
        self.sprite_line = vec![0; width as usize];
        self.sprite_line_raster = None;
        self.work_field_odd = false;
        self.rendered_pixels = 0;
        // A geometry change drops any prior back-porch framing until the bus
        // reconfigures it; tests that only call ensure_geometry stay unframed.
        self.framing_active = false;
        self.frame_width = width;
        self.frame_published_height = self.published_height;
        self.offset_x = 0;
        self.offset_y = 0;
    }

    /// Positions the display window inside a fixed reference frame.
    ///
    /// `frame_width` and `frame_height` are the reference dimensions in
    /// window units (the same units as [`ensure_geometry`]); `offset_x` and
    /// `offset_y` place the display window inside that frame. The area not
    /// covered by the window is filled with opaque black blanking. Passing a
    /// frame equal to the window with zero offset disables framing.
    pub fn configure_frame(
        &mut self,
        offset_x: u32,
        offset_y: u32,
        frame_width: u32,
        frame_height: u32,
    ) {
        let frame_published_height = match self.scan_mode {
            ScanModeX68k::Progressive | ScanModeX68k::DoubleRead => frame_height,
            ScanModeX68k::Interlace => frame_height * 2,
        };
        self.framing_active = offset_x != 0
            || offset_y != 0
            || frame_width != self.width
            || frame_height != self.height;
        self.offset_x = offset_x;
        self.offset_y = offset_y;
        self.frame_width = frame_width;
        self.frame_published_height = frame_published_height;
        if self.framing_active {
            let needed = frame_width as usize * frame_published_height as usize * X68K_PIXEL_BYTES;
            if self.framed.len() != needed {
                self.framed = black_frame(frame_width, frame_published_height);
            }
        }
    }

    /// Composes the published window into the reference frame with blanking.
    fn compose_framed(&mut self) {
        for byte in self.framed.iter_mut() {
            *byte = 0;
        }
        for alpha in self.framed.iter_mut().skip(3).step_by(X68K_PIXEL_BYTES) {
            *alpha = 0xFF;
        }
        let vertical_scale = (self.published_height / self.height).max(1) as usize;
        let offset_y_rows = self.offset_y as usize * vertical_scale;
        let source_row_bytes = self.width as usize * X68K_PIXEL_BYTES;
        let frame_row_bytes = self.frame_width as usize * X68K_PIXEL_BYTES;
        let offset_x_bytes = self.offset_x as usize * X68K_PIXEL_BYTES;
        for row in 0..self.published_height as usize {
            let source = row * source_row_bytes;
            let destination = (row + offset_y_rows) * frame_row_bytes + offset_x_bytes;
            self.framed[destination..destination + source_row_bytes]
                .copy_from_slice(&self.published[source..source + source_row_bytes]);
        }
    }

    /// Renders all pixels up to the target using the current inputs.
    pub fn catch_up(&mut self, inputs: &RenderInputsX68k<'_>, target_pixel: usize) {
        let total = self.width as usize * self.height as usize;
        let target = target_pixel.min(total);
        for pixel in self.rendered_pixels..target {
            let screen_x = pixel % self.width as usize;
            let screen_y = pixel / self.width as usize;
            let content_y = match self.scan_mode {
                ScanModeX68k::Progressive => screen_y,
                ScanModeX68k::DoubleRead => screen_y / 2,
                ScanModeX68k::Interlace => screen_y * 2 + usize::from(self.work_field_odd),
            };
            if self.sprite_line_raster != Some(content_y) {
                rasterize_sprite_line(inputs, content_y, &mut self.sprite_line);
                self.sprite_line_raster = Some(content_y);
            }
            let sprite_code = self.sprite_line[screen_x];
            let text_code = text_color_code(inputs, screen_x, content_y) as u8;
            let graphic = graphic_codes(inputs, screen_x, content_y);
            let color = compose_pixel(inputs, sprite_code, text_code, &graphic);
            let rgba = grbi_to_rgba(color, inputs.contrast);
            let offset = pixel * X68K_PIXEL_BYTES;
            self.work[offset..offset + X68K_PIXEL_BYTES].copy_from_slice(&rgba);
        }
        self.rendered_pixels = self.rendered_pixels.max(target);
    }

    /// Completes and publishes a frame, then starts a fresh scanout buffer.
    ///
    /// The CRTC toggles its field parity before the completed field is
    /// published, so `inputs.odd_field` names the field the work buffer
    /// accumulates next; the completed field keeps the latched parity.
    pub fn publish_frame(&mut self, inputs: &RenderInputsX68k<'_>) {
        let total = self.width as usize * self.height as usize;
        self.catch_up(inputs, total);
        match self.scan_mode {
            ScanModeX68k::Progressive | ScanModeX68k::DoubleRead => {
                self.published.copy_from_slice(&self.work);
            }
            ScanModeX68k::Interlace => {
                let row_bytes = self.width as usize * X68K_PIXEL_BYTES;
                let parity = usize::from(self.work_field_odd);
                for row in 0..self.height as usize {
                    let destination = (row * 2 + parity) * row_bytes;
                    self.published[destination..destination + row_bytes]
                        .copy_from_slice(&self.work[row * row_bytes..(row + 1) * row_bytes]);
                }
            }
        }
        if self.framing_active {
            self.compose_framed();
        }
        self.work.fill(0);
        for alpha in self.work.iter_mut().skip(3).step_by(X68K_PIXEL_BYTES) {
            *alpha = 0xFF;
        }
        self.sprite_line_raster = None;
        self.work_field_odd = inputs.odd_field;
        self.rendered_pixels = 0;
    }
}

fn black_frame(width: u32, height: u32) -> Vec<u8> {
    let mut frame = vec![0; width as usize * height as usize * X68K_PIXEL_BYTES];
    for alpha in frame.iter_mut().skip(3).step_by(X68K_PIXEL_BYTES) {
        *alpha = 0xFF;
    }
    frame
}

/// Owned buffers backing render inputs in renderer tests.
#[cfg(test)]
pub(crate) struct FixtureX68k {
    /// Text VRAM image.
    pub text_vram: Vec<u8>,
    /// Graphics VRAM image.
    pub graphic_vram: Vec<u16>,
    /// Text, sprite, and background palette.
    pub text_palette: [u16; X68K_PALETTE_ENTRIES],
    /// Graphics palette.
    pub graphics_palette: [u16; X68K_PALETTE_ENTRIES],
    /// Text scroll in dots and rasters.
    pub text_scroll: (u16, u16),
    /// Graphic scroll in dots and rasters per GVRAM page.
    pub graphic_scroll: [(u16, u16); 4],
    /// Raw CRTC memory-mode register R20.
    pub crtc_memory_mode: u16,
    /// Raw video-controller registers R0-R2.
    pub video_registers: [u16; 3],
    /// Sprite scroll table.
    pub sprite_scroll: [[u16; 4]; X68K_SPRITE_COUNT],
    /// Sprite pattern RAM image.
    pub sprite_pattern: Vec<u16>,
    /// Background scroll registers (BG0 X, BG0 Y, BG1 X, BG1 Y).
    pub background_scroll: [u16; 4],
    /// Sprite background control register.
    pub background_control: u16,
    /// Sprite resolution register.
    pub sprite_resolution: u16,
    /// Sprite horizontal and vertical back-porch end registers.
    pub sprite_back_ends: (u16, u16),
    /// CRTC horizontal and vertical back-porch end registers.
    pub crtc_back_ends: (u16, u16),
    /// Whether the sprite area is accessible.
    pub sprite_area_accessible: bool,
    /// Screen contrast.
    pub contrast: u8,
    /// Visible size in pixels.
    pub size: (u32, u32),
    /// Current CRTC field parity.
    pub odd_field: bool,
}

#[cfg(test)]
impl FixtureX68k {
    /// Creates a fixture with cleared memory and all layers disabled.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            text_vram: vec![0; 0x8_0000],
            graphic_vram: vec![0; X68K_GVRAM_WORDS],
            text_palette: [0; X68K_PALETTE_ENTRIES],
            graphics_palette: [0; X68K_PALETTE_ENTRIES],
            text_scroll: (0, 0),
            graphic_scroll: [(0, 0); 4],
            crtc_memory_mode: 0,
            video_registers: [0, 0x12E4, 0],
            sprite_scroll: [[0; 4]; X68K_SPRITE_COUNT],
            sprite_pattern: vec![0; X68K_SPRITE_PATTERN_WORDS],
            background_scroll: [0; 4],
            background_control: 0,
            sprite_resolution: 0,
            sprite_back_ends: (4, 0),
            crtc_back_ends: (0, 0),
            sprite_area_accessible: true,
            contrast: 15,
            size: (width, height),
            odd_field: false,
        }
    }

    /// Borrows the fixture as render inputs.
    pub fn inputs(&self) -> RenderInputsX68k<'_> {
        RenderInputsX68k {
            text_vram: &self.text_vram,
            graphic_vram: &self.graphic_vram,
            text_palette: &self.text_palette,
            graphics_palette: &self.graphics_palette,
            text_scroll_x: self.text_scroll.0,
            text_scroll_y: self.text_scroll.1,
            graphic_scroll_x: [
                self.graphic_scroll[0].0,
                self.graphic_scroll[1].0,
                self.graphic_scroll[2].0,
                self.graphic_scroll[3].0,
            ],
            graphic_scroll_y: [
                self.graphic_scroll[0].1,
                self.graphic_scroll[1].1,
                self.graphic_scroll[2].1,
                self.graphic_scroll[3].1,
            ],
            crtc_memory_mode: self.crtc_memory_mode,
            memory_mode: self.video_registers[0],
            priority: self.video_registers[1],
            mixing: self.video_registers[2],
            sprite_scroll: &self.sprite_scroll,
            sprite_pattern: &self.sprite_pattern,
            background_scroll: self.background_scroll,
            background_control: self.background_control,
            sprite_resolution: self.sprite_resolution,
            sprite_horizontal_back_end: self.sprite_back_ends.0,
            sprite_vertical_back_end: self.sprite_back_ends.1,
            crtc_horizontal_back_end: self.crtc_back_ends.0,
            crtc_vertical_back_end: self.crtc_back_ends.1,
            sprite_area_accessible: self.sprite_area_accessible,
            contrast: self.contrast,
            width: self.size.0,
            height: self.size.1,
            odd_field: self.odd_field,
        }
    }

    /// Stores one word in the graphics VRAM at page coordinates.
    pub fn set_graphic_word(&mut self, x: usize, y: usize, value: u16) {
        self.graphic_vram[y * 512 + x] = value;
    }

    /// Sets one sprite scroll entry.
    pub fn set_sprite(&mut self, index: usize, x: u16, y: u16, pattern_word: u16, priority: u16) {
        self.sprite_scroll[index] = [x, y, pattern_word, priority];
    }

    /// Fills one 16x16 sprite pattern with a solid 4-bit code.
    pub fn fill_sprite_pattern(&mut self, number: usize, code: u16) {
        let value = code << 12 | code << 8 | code << 4 | code;
        self.sprite_pattern[number * 64..number * 64 + 64].fill(value);
    }

    /// Fills one 8x8 background pattern with a solid 4-bit code.
    pub fn fill_background_pattern(&mut self, number: usize, code: u16) {
        let value = code << 12 | code << 8 | code << 4 | code;
        self.sprite_pattern[number * 16..number * 16 + 16].fill(value);
    }

    /// Stores one background tile-map entry.
    pub fn set_background_tile(&mut self, map: usize, column: usize, row: usize, entry: u16) {
        let offset = if map == 0 { 0x2000 } else { 0x3000 };
        self.sprite_pattern[offset + row * 64 + column] = entry;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catch_up_pixels_keep_their_state_at_render_time() {
        let mut fixture = FixtureX68k::new(16, 2);
        fixture.video_registers[2] = 0x0020;
        fixture.text_palette[0] = 0xFFFF;
        let mut renderer = X68kRenderer::new();
        renderer.ensure_geometry(16, 2, ScanModeX68k::Progressive);
        renderer.catch_up(&fixture.inputs(), 4);
        fixture.text_palette[0] = 0;
        renderer.publish_frame(&fixture.inputs());
        assert_eq!(&renderer.framebuffer()[..4], &[255, 255, 255, 255]);
        assert_eq!(&renderer.framebuffer()[12..16], &[255, 255, 255, 255]);
        assert_eq!(&renderer.framebuffer()[16..20], &[0, 0, 0, 255]);
    }

    #[test]
    fn sprite_lines_latch_at_the_first_pixel_of_each_scanline() {
        let mut fixture = FixtureX68k::new(16, 2);
        fixture.video_registers[2] = 0x0040;
        fixture.background_control = 0x0200;
        fixture.text_palette[0x31] = 0xFFFF;
        fixture.fill_sprite_pattern(1, 1);
        fixture.set_sprite(0, 16, 16, 0x0301, 3);
        let mut renderer = X68kRenderer::new();
        renderer.ensure_geometry(16, 2, ScanModeX68k::Progressive);
        renderer.catch_up(&fixture.inputs(), 4);
        fixture.set_sprite(0, 24, 16, 0x0301, 3);
        renderer.publish_frame(&fixture.inputs());
        assert_eq!(&renderer.framebuffer()[..4], &[255, 255, 255, 255]);
        assert_eq!(
            &renderer.framebuffer()[15 * 4..15 * 4 + 4],
            &[255, 255, 255, 255]
        );
        assert_eq!(&renderer.framebuffer()[16 * 4..16 * 4 + 4], &[0, 0, 0, 255]);
        assert_eq!(
            &renderer.framebuffer()[24 * 4..24 * 4 + 4],
            &[255, 255, 255, 255]
        );
    }

    /// Returns one published pixel of a 16-pixel-wide framebuffer.
    fn published_pixel(renderer: &X68kRenderer, x: usize, y: usize) -> [u8; 4] {
        let offset = (y * 16 + x) * X68K_PIXEL_BYTES;
        renderer.framebuffer()[offset..offset + X68K_PIXEL_BYTES]
            .try_into()
            .unwrap()
    }

    /// Opaque white at full contrast.
    const WHITE: [u8; 4] = [255, 255, 255, 255];
    /// Opaque black.
    const BLACK: [u8; 4] = [0, 0, 0, 255];

    #[test]
    fn double_read_rows_render_content_lines_twice() {
        let mut fixture = FixtureX68k::new(16, 4);
        fixture.video_registers[2] = 0x0020;
        fixture.text_palette[1] = 0xFFFF;
        fixture.text_vram[0] = 0xFF;
        let mut renderer = X68kRenderer::new();
        renderer.ensure_geometry(16, 4, ScanModeX68k::DoubleRead);
        renderer.publish_frame(&fixture.inputs());
        assert_eq!(renderer.dimensions(), (16, 4));
        assert_eq!(published_pixel(&renderer, 0, 0), WHITE);
        assert_eq!(published_pixel(&renderer, 0, 1), WHITE);
        assert_eq!(published_pixel(&renderer, 0, 2), BLACK);
        assert_eq!(published_pixel(&renderer, 0, 3), BLACK);
    }

    #[test]
    fn interlaced_fields_weave_and_retain_the_other_field() {
        let mut fixture = FixtureX68k::new(16, 2);
        fixture.video_registers[2] = 0x0020;
        fixture.text_palette[1] = 0xFFFF;
        let mut renderer = X68kRenderer::new();
        renderer.ensure_geometry(16, 2, ScanModeX68k::Interlace);
        assert_eq!(renderer.dimensions(), (16, 4));

        // Even field: content rows 0 and 2, parity toggles to odd at publish.
        fixture.text_vram[0] = 0xFF;
        fixture.odd_field = true;
        renderer.publish_frame(&fixture.inputs());
        assert_eq!(published_pixel(&renderer, 0, 0), WHITE);
        assert_eq!(published_pixel(&renderer, 0, 1), BLACK);
        assert_eq!(published_pixel(&renderer, 0, 2), BLACK);

        // Odd field: content rows 1 and 3; even rows retain the first field.
        fixture.text_vram[0] = 0x00;
        fixture.text_vram[128] = 0xFF;
        fixture.odd_field = false;
        renderer.publish_frame(&fixture.inputs());
        assert_eq!(published_pixel(&renderer, 0, 0), WHITE);
        assert_eq!(published_pixel(&renderer, 0, 1), WHITE);
        assert_eq!(published_pixel(&renderer, 0, 2), BLACK);
        assert_eq!(published_pixel(&renderer, 0, 3), BLACK);
    }

    #[test]
    fn geometry_or_scan_mode_change_resets_the_buffers() {
        let mut fixture = FixtureX68k::new(16, 2);
        fixture.video_registers[2] = 0x0020;
        fixture.text_palette[0] = 0xFFFF;
        let mut renderer = X68kRenderer::new();
        renderer.ensure_geometry(16, 2, ScanModeX68k::Progressive);
        renderer.publish_frame(&fixture.inputs());
        assert_eq!(published_pixel(&renderer, 0, 0), WHITE);
        renderer.ensure_geometry(16, 2, ScanModeX68k::Interlace);
        assert_eq!(renderer.dimensions(), (16, 4));
        assert_eq!(renderer.framebuffer().len(), 16 * 4 * X68K_PIXEL_BYTES);
        assert_eq!(published_pixel(&renderer, 0, 0), BLACK);
    }

    /// Returns one published pixel of a framebuffer of the given width.
    fn framed_pixel(renderer: &X68kRenderer, width: usize, x: usize, y: usize) -> [u8; 4] {
        let offset = (y * width + x) * X68K_PIXEL_BYTES;
        renderer.framebuffer()[offset..offset + X68K_PIXEL_BYTES]
            .try_into()
            .unwrap()
    }

    #[test]
    fn back_porch_framing_offsets_the_window_and_blanks_the_border() {
        let mut fixture = FixtureX68k::new(16, 2);
        fixture.video_registers[2] = 0x0020;
        fixture.text_palette[0] = 0xFFFF;
        let mut renderer = X68kRenderer::new();
        renderer.ensure_geometry(16, 2, ScanModeX68k::Progressive);
        renderer.configure_frame(4, 2, 20, 4);
        renderer.publish_frame(&fixture.inputs());
        assert_eq!(renderer.dimensions(), (20, 4));
        // The window is flush to the frame's bottom-right corner.
        assert_eq!(framed_pixel(&renderer, 20, 4, 2), WHITE);
        assert_eq!(framed_pixel(&renderer, 20, 19, 3), WHITE);
        // The blanking fills the top and left border.
        assert_eq!(framed_pixel(&renderer, 20, 0, 0), BLACK);
        assert_eq!(framed_pixel(&renderer, 20, 3, 1), BLACK);
        assert_eq!(framed_pixel(&renderer, 20, 3, 2), BLACK);
    }

    #[test]
    fn a_frame_equal_to_the_window_leaves_the_output_unframed() {
        let mut fixture = FixtureX68k::new(16, 2);
        fixture.video_registers[2] = 0x0020;
        fixture.text_palette[0] = 0xFFFF;
        let mut renderer = X68kRenderer::new();
        renderer.ensure_geometry(16, 2, ScanModeX68k::Progressive);
        renderer.configure_frame(0, 0, 16, 2);
        renderer.publish_frame(&fixture.inputs());
        assert_eq!(renderer.dimensions(), (16, 2));
        assert_eq!(published_pixel(&renderer, 0, 0), WHITE);
    }

    #[test]
    fn interlaced_framing_scales_the_vertical_offset() {
        let mut fixture = FixtureX68k::new(16, 2);
        fixture.video_registers[2] = 0x0020;
        fixture.text_palette[0] = 0xFFFF;
        let mut renderer = X68kRenderer::new();
        renderer.ensure_geometry(16, 2, ScanModeX68k::Interlace);
        // Window published height is 4; a one-raster offset shifts two rows.
        renderer.configure_frame(0, 1, 16, 3);
        renderer.publish_frame(&fixture.inputs());
        assert_eq!(renderer.dimensions(), (16, 6));
        assert_eq!(framed_pixel(&renderer, 16, 0, 0), BLACK);
        assert_eq!(framed_pixel(&renderer, 16, 0, 1), BLACK);
        assert_eq!(framed_pixel(&renderer, 16, 0, 2), WHITE);
    }

    #[test]
    fn publish_frame_resets_the_work_buffer_to_black() {
        let mut fixture = FixtureX68k::new(16, 2);
        fixture.video_registers[2] = 0x0020;
        fixture.text_palette[0] = 0xFFFF;
        let mut renderer = X68kRenderer::new();
        renderer.ensure_geometry(16, 2, ScanModeX68k::Progressive);
        renderer.publish_frame(&fixture.inputs());
        assert_eq!(&renderer.framebuffer()[..4], &[255, 255, 255, 255]);
        fixture.text_palette[0] = 0;
        renderer.publish_frame(&fixture.inputs());
        assert_eq!(&renderer.framebuffer()[..4], &[0, 0, 0, 255]);
    }
}
