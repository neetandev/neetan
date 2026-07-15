//! PC-8801 display renderer.
//!
//! Composites the graphics layer (3-plane GVRAM) under the uPD3301 text layer
//! into a packed RGBA framebuffer, reusing the crate's framebuffer/PPM plumbing.
//! It consumes a [`RenderInputs88`] snapshot of the expanded character and
//! attribute planes produced by the CRTC and the GVRAM planes, plus the built-in
//! 8x8 ANK character generator (the kanji ROM window at offset 0x1000).

mod compose;
mod glyph;
mod graphics;

use alloc::{boxed::Box, vec};

/// PC-88 framebuffer width in pixels.
pub const PC88_WIDTH: usize = 640;
/// PC-88 framebuffer maximum height in pixels (400-line mode).
pub const PC88_MAX_HEIGHT: usize = 400;
/// Bytes per pixel (`R, G, B, A`).
pub const PC88_PIXEL_BYTES: usize = 4;
/// PC-88 framebuffer byte size.
pub const PC88_FRAMEBUFFER_BYTES: usize = PC88_WIDTH * PC88_MAX_HEIGHT * PC88_PIXEL_BYTES;

/// Offset of the built-in 8x8 ANK character generator within the level-1 kanji
/// ROM (256 glyphs of 8 bytes each).
pub const ANK_FONT_OFFSET: usize = 0x1000;

/// PC-88 graphics display mode, selected by the gfx_ctrl (port 0x31) bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GraphicsMode88 {
    /// 640x200 8-color: the three planes combine into a 3-bit pen per pixel.
    #[default]
    Color8,
    /// 640x200 1bpp: the enabled planes are ORed; the color comes from the text
    /// attribute of the cell, with reverse video inverting the plane bits.
    Attrib200,
    /// 640x400 1bpp: blue plane is the upper 200 lines, red plane the lower 200;
    /// the color comes from the text attribute (sampled at line>>1).
    Attrib400,
}

/// Per-frame inputs to the PC-88 renderer.
pub struct RenderInputs88<'a> {
    /// Expanded per-cell character codes, row-major, 200 rows x 80 columns.
    pub text_codes: &'a [u8],
    /// Expanded per-cell attribute bytes, row-major, 200 rows x 80 columns.
    pub text_attrib: &'a [u8],
    /// Displayed character columns (up to 80).
    pub columns: u32,
    /// Displayed character rows.
    pub rows: u32,
    /// Scanlines per character row.
    pub char_height: u32,
    /// 40-column mode (each character doubled horizontally).
    pub width_40col: bool,
    /// Color text mode; when false the text layer is forced to white.
    pub color_mode: bool,
    /// Whether the text layer is enabled (display active and not layer-disabled).
    pub text_enabled: bool,
    /// Background color as 8-bit RGB (from port 0x52).
    pub background_rgb: [u8; 3],
    /// Whether the graphics layer is enabled (GRPHE, gfx_ctrl bit 3).
    pub graphics_enabled: bool,
    /// Selected graphics display mode.
    pub graphics_mode: GraphicsMode88,
    /// 400-line display (the graphics layer is 400 lines tall).
    pub line_400: bool,
    /// GVRAM blue plane (16 KiB).
    pub gvram_blue: &'a [u8],
    /// GVRAM red plane (16 KiB).
    pub gvram_red: &'a [u8],
    /// GVRAM green plane (16 KiB).
    pub gvram_green: &'a [u8],
    /// The eight graphics pens as 8-bit RGB (palette ports 0x54-0x5B).
    pub graphics_palette: [[u8; 3]; 8],
    /// Palette mode (PMODE, misc_ctrl bit 5): text uses the graphics palette.
    pub palette_mode: bool,
    /// Plane disable flags (port 0x53 bits 1-3): bit0 blue, bit1 red, bit2 green.
    /// Applied in attribute modes only, not in [`GraphicsMode88::Color8`].
    pub plane_disable: u8,
    /// Active framebuffer width in pixels.
    pub width: u32,
    /// Active framebuffer height in pixels.
    pub height: u32,
}

/// Persistent buffers owned by the PC-88 renderer.
pub struct Pc88RendererState {
    /// Internal copy of the level-1 kanji ROM (character generator source).
    pub font_rom: Box<[u8]>,
    /// Packed RGBA framebuffer (`R, G, B, A` little-endian).
    pub framebuffer: Box<[u8]>,
}

save_state::runtime_state! {
/// Save-state portion of the PC-88 renderer.
#[derive(Clone)]
pub struct Pc88RendererRuntimeState {
    framebuffer: Box<[u8]>,
}}

/// Number of pen-index entries in a full-frame scratch layer.
const LAYER_PIXELS: usize = PC88_WIDTH * PC88_MAX_HEIGHT;

/// CPU-side renderer for the PC-88 display.
pub struct Pc88Renderer {
    /// Embedded state for save/restore.
    pub state: Pc88RendererState,
    /// Generated 8x8 semigraphics block patterns (256 glyphs of 8 bytes).
    sg_pattern: Box<[u8; 256 * 8]>,
    /// Scratch graphics-layer pen indices (one byte per pixel).
    graph_pens: Box<[u8]>,
    /// Scratch text-layer pen indices (one byte per pixel, 0 = transparent).
    text_pens: Box<[u8]>,
}

impl Pc88Renderer {
    /// Creates a renderer with the given character-generator ROM (the level-1
    /// kanji ROM; the ANK font lives at [`ANK_FONT_OFFSET`]).
    pub fn new(font_rom_data: &[u8]) -> Self {
        let mut font_rom =
            vec![0u8; font_rom_data.len().max(ANK_FONT_OFFSET + 0x800)].into_boxed_slice();
        let copy_len = font_rom.len().min(font_rom_data.len());
        font_rom[..copy_len].copy_from_slice(&font_rom_data[..copy_len]);
        Self {
            state: Pc88RendererState {
                font_rom,
                framebuffer: vec![0u8; PC88_FRAMEBUFFER_BYTES].into_boxed_slice(),
            },
            sg_pattern: glyph::build_semigraphics_pattern(),
            graph_pens: vec![0u8; LAYER_PIXELS].into_boxed_slice(),
            text_pens: vec![0u8; LAYER_PIXELS].into_boxed_slice(),
        }
    }

    /// Replaces the character-generator ROM.
    pub fn update_font_rom(&mut self, font_rom_data: &[u8]) {
        if font_rom_data.len() > self.state.font_rom.len() {
            self.state.font_rom = vec![0u8; font_rom_data.len()].into_boxed_slice();
        } else {
            self.state.font_rom.fill(0);
        }
        let copy_len = self.state.font_rom.len().min(font_rom_data.len());
        self.state.font_rom[..copy_len].copy_from_slice(&font_rom_data[..copy_len]);
    }

    /// Renders one frame into the internal framebuffer.
    pub fn render(&mut self, inputs: &RenderInputs88<'_>) {
        compose::compose(
            &mut self.state.framebuffer,
            &self.state.font_rom,
            self.sg_pattern.as_slice(),
            &mut self.graph_pens,
            &mut self.text_pens,
            inputs,
        );
    }

    /// Returns the packed RGBA framebuffer.
    pub fn framebuffer(&self) -> &[u8] {
        &self.state.framebuffer
    }

    /// Captures the presented framebuffer without the immutable font.
    pub fn capture_state(&self) -> Pc88RendererRuntimeState {
        Pc88RendererRuntimeState {
            framebuffer: self.state.framebuffer.clone(),
        }
    }

    /// Restores the presented framebuffer without changing the font.
    pub fn restore_state(
        &mut self,
        state: Pc88RendererRuntimeState,
    ) -> Result<(), save_state::StateValidationError> {
        if state.framebuffer.len() != PC88_FRAMEBUFFER_BYTES {
            return Err(save_state::StateValidationError::new(
                "PC-88 renderer state is invalid",
            ));
        }
        self.state.framebuffer = state.framebuffer;
        Ok(())
    }
}
