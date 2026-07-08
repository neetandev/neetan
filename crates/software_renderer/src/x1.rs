//! Sharp X1 video composition.
//!
//! Composes the X1 display per scanline: the text / PCG / kanji layer and the
//! three-plane bitmap graphics layer each render into 3-bit colour buffers,
//! and a per-line priority table (built from the priority register, the
//! programmable palette gun latches and mode register 2) maps every
//! (graphics colour, text colour) pair to one of sixteen fixed digital RGBA
//! colours. The machine layer owns the VRAM, registers and frame-latched CRTC
//! geometry and passes them in through [`RenderInputsX1`]; the renderer holds
//! the CG-ROM font, the line buffers and the framebuffer.

mod graphics;
mod palette;
mod text;

use alloc::{boxed::Box, vec};

use palette::{FIXED_RGBA, PRI_LUT_SIZE, build_pri_lut};

/// Debug layer selector for renderer diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum X1DebugLayer {
    /// Normal full composition.
    Full,
    /// Text / PCG / kanji layer only.
    Text,
    /// Bitmap graphics layer only.
    Bitmap,
}

/// Machine variant driving the renderer; gates the turbo-only paths (kanji,
/// gaiji, the mode registers and the hi-res scan).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum X1RendererModel {
    /// Base X1.
    Base,
    /// X1 turbo.
    Turbo,
}

/// X1 framebuffer width in pixels (80 columns x 8 pixels).
pub const X1_SURFACE_WIDTH: usize = 640;
/// Base-X1 framebuffer height in pixels (25 rows x 8 scanlines).
pub const X1_SURFACE_HEIGHT: usize = 200;
/// Maximum framebuffer height (turbo 400-line hi-res mode).
pub const X1_MAX_HEIGHT: usize = 400;
/// Bytes per pixel (packed RGBA).
pub const X1_PIXEL_BYTES: usize = 4;
/// Total framebuffer byte size, sized for the tallest (400-line) mode.
pub const X1_FRAMEBUFFER_BYTES: usize = X1_SURFACE_WIDTH * X1_MAX_HEIGHT * X1_PIXEL_BYTES;

const LAYER_PIXELS: usize = X1_SURFACE_WIDTH * X1_MAX_HEIGHT;
const BLACK: [u8; 4] = [0x00, 0x00, 0x00, 0xFF];

/// Per-scanline inputs borrowed from the machine.
pub struct RenderInputsX1<'a> {
    /// Text VRAM window (character codes).
    pub text_vram: &'a [u8],
    /// Attribute VRAM window (per-cell attribute bytes).
    pub attr_vram: &'a [u8],
    /// Kanji text-VRAM window (turbo); empty on the base X1.
    pub kvram: &'a [u8],
    /// PCG RAM: three colour planes at offsets 0x000, 0x800, 0x1000.
    pub pcg: &'a [u8],
    /// Gaiji (16-line PCG) RAM: three colour planes of 128 codes x 16 rows
    /// (turbo); empty on the base X1.
    pub gaiji: &'a [u8],
    /// 8x16 ANK font ROM (turbo); empty on the base X1.
    pub ank_rom: &'a [u8],
    /// De-interleaved kanji ROM (turbo); empty on the base X1.
    pub kanji_rom: &'a [u8],
    /// Bitmap VRAM: two pages of blue/red/green planes.
    pub bitmap: &'a [u8],
    /// Programmable palette gun latches [blue, red, green].
    pub palette: [u8; 3],
    /// Priority register: bit `c` set draws graphics colour `c` over text.
    pub priority: u8,
    /// Turbo mode register 1 (`0x1FD0`), raw.
    pub mode1: u8,
    /// Turbo mode register 2 (`0x1FE0`), raw.
    pub mode2: u8,
    /// Frame counter for the text blink attribute (phase bit 5).
    pub cblink: u8,
    /// Whether the CRTC display-enable skew turns the display off (R8).
    pub display_off: bool,
    /// Displayed character columns (CRTC R1), frame-latched.
    pub hz_disp: u16,
    /// Displayed character rows (CRTC R6), frame-latched.
    pub vt_disp: u16,
    /// Scanlines per character row (CRTC R9 + 1), frame-latched.
    pub ch_height: u16,
    /// Display-memory start address (CRTC R12/R13), frame-latched.
    pub st_addr: u16,
    /// Blanked lines above the display (CRTC R5 vertical adjust), latched.
    pub vt_ofs: u16,
    /// 24 kHz hi-res scan: the CRTC vertical total exceeds 400 raster lines.
    pub hires: bool,
    /// 40-column / 320-pixel hi-speed mode (PPI port C bit 6).
    pub column40: bool,
    /// Machine variant.
    pub model: X1RendererModel,
}

impl RenderInputsX1<'_> {
    /// Horizontal pixel-doubling factor. The 320-dot hi-speed mode (`column40`)
    /// draws each source pixel twice so a narrow 40-column screen fills the
    /// full display width. It only applies while the CRTC is programmed narrow
    /// enough for the doubled output to fit; an 80-column CRTC drives the full
    /// 640-dot width and is never doubled, even with the 320-dot bit set (e.g.
    /// Arcus, which runs a 640x400 screen with the bit asserted).
    fn horizontal_scale(&self) -> usize {
        if self.column40 && (self.hz_disp as usize) * 8 * 2 <= X1_SURFACE_WIDTH {
            2
        } else {
            1
        }
    }

    /// Character cells drawn per line: 40 in the doubled 320-dot mode, else
    /// the full 80. Follows [`Self::horizontal_scale`], so an 80-column screen
    /// with the 320-dot bit asserted still draws all of its cells.
    pub(super) fn cell_limit(&self) -> usize {
        if self.horizontal_scale() == 2 { 40 } else { 80 }
    }

    fn surface_height(&self) -> usize {
        if self.hires {
            X1_MAX_HEIGHT
        } else {
            X1_SURFACE_HEIGHT
        }
    }
}

/// X1 software renderer: owns the CG-ROM font, the per-line colour buffers and
/// priority tables, and the framebuffer.
pub struct X1Renderer {
    font: Box<[u8]>,
    /// Text colours (0..7) per pixel, one row per latched scanline.
    text: Box<[u8]>,
    /// Graphics colours (0..7) per pixel, one row per latched scanline.
    cg: Box<[u8]>,
    /// Per-line priority tables mapping (cg, text) to a palette index.
    pri_lines: Box<[u8]>,
    /// Frame-global glyph-row counter for the double-height text attribute.
    raster: u16,
    /// Top blanking latched for the presented frame.
    frame_vt_ofs: usize,
    /// Horizontal doubling latched for the presented frame.
    frame_hscale: usize,
    framebuffer: Box<[u8]>,
}

impl X1Renderer {
    /// Creates a renderer with the given CG-ROM (8x8 ANK font) data.
    pub fn new(cg_rom: &[u8]) -> Self {
        Self {
            font: cg_rom.to_vec().into_boxed_slice(),
            text: vec![0u8; LAYER_PIXELS].into_boxed_slice(),
            cg: vec![0u8; LAYER_PIXELS].into_boxed_slice(),
            pri_lines: vec![0u8; X1_MAX_HEIGHT * PRI_LUT_SIZE].into_boxed_slice(),
            raster: 0,
            frame_vt_ofs: 0,
            frame_hscale: 1,
            framebuffer: vec![0u8; X1_FRAMEBUFFER_BYTES].into_boxed_slice(),
        }
    }

    /// Replaces the CG-ROM font (e.g. after a ROM reload).
    pub fn update_font(&mut self, cg_rom: &[u8]) {
        self.font = cg_rom.to_vec().into_boxed_slice();
    }

    /// The last composited framebuffer (packed RGBA).
    pub fn framebuffer(&self) -> &[u8] {
        &self.framebuffer
    }

    /// Clears the scanline-latched buffers for the next frame.
    pub fn clear_latched_frame(&mut self) {
        self.text.fill(0);
        self.cg.fill(0);
        self.pri_lines.fill(0);
        self.raster = 0;
    }

    /// Latches one scanline from the current VRAM and register state: the text
    /// and graphics layers render into their line buffers and the priority
    /// table for the line is snapshotted. Lines outside the displayed area (or
    /// with the display turned off) stay cleared and present as black.
    pub fn latch_scanline(&mut self, inputs: &RenderInputsX1<'_>, line: usize) {
        if line >= inputs.surface_height() {
            return;
        }
        self.frame_vt_ofs = usize::from(inputs.vt_ofs);
        self.frame_hscale = inputs.horizontal_scale();
        if inputs.display_off {
            return;
        }
        let ch_height = usize::from(inputs.ch_height).max(1);
        if line >= usize::from(inputs.vt_disp) * ch_height {
            return;
        }

        let row_start = line * X1_SURFACE_WIDTH;
        let row_end = row_start + X1_SURFACE_WIDTH;
        text::draw_text_line(
            inputs,
            &self.font,
            line,
            &mut self.raster,
            &mut self.text[row_start..row_end],
        );
        graphics::draw_cg_line(
            inputs,
            line,
            &mut self.text[row_start..row_end],
            &mut self.cg[row_start..row_end],
        );
        let lut = build_pri_lut(inputs.palette, inputs.priority, inputs.mode2, inputs.model);
        let lut_start = line * PRI_LUT_SIZE;
        self.pri_lines[lut_start..lut_start + PRI_LUT_SIZE].copy_from_slice(&lut);
    }

    /// Composites the latched scanlines into the packed RGBA framebuffer.
    /// Content shifts down by the latched top blanking; every uncovered line
    /// presents black.
    pub fn present_latched_frame(&mut self, height: usize) -> (u32, u32) {
        let height = height.min(X1_MAX_HEIGHT);
        let vt_ofs = self.frame_vt_ofs;
        let hscale = self.frame_hscale.max(1);
        for dest_line in 0..X1_MAX_HEIGHT {
            let framebuffer_start = dest_line * X1_SURFACE_WIDTH * X1_PIXEL_BYTES;
            let framebuffer_row = &mut self.framebuffer
                [framebuffer_start..framebuffer_start + X1_SURFACE_WIDTH * X1_PIXEL_BYTES];
            if dest_line >= height || dest_line < vt_ofs {
                for pixel in framebuffer_row.chunks_exact_mut(X1_PIXEL_BYTES) {
                    pixel.copy_from_slice(&BLACK);
                }
                continue;
            }
            let source_line = dest_line - vt_ofs;
            let row_start = source_line * X1_SURFACE_WIDTH;
            let lut_start = source_line * PRI_LUT_SIZE;
            let lut = &self.pri_lines[lut_start..lut_start + PRI_LUT_SIZE];
            for x in 0..X1_SURFACE_WIDTH / hscale {
                let text_color = self.text[row_start + x] & 7;
                let cg_color = self.cg[row_start + x] & 7;
                let entry = lut[usize::from(cg_color) * 8 + usize::from(text_color)];
                let color = FIXED_RGBA[usize::from(entry)];
                for h in 0..hscale {
                    let offset = (x * hscale + h) * X1_PIXEL_BYTES;
                    framebuffer_row[offset..offset + X1_PIXEL_BYTES].copy_from_slice(&color);
                }
            }
        }
        (X1_SURFACE_WIDTH as u32, height as u32)
    }

    /// Renders a full frame from one register snapshot and composites the
    /// selected layer, returning the displayed `(width, height)`. Diagnostics
    /// only; the machine drives the per-scanline latch path.
    pub fn render_debug_layer(
        &mut self,
        inputs: &RenderInputsX1<'_>,
        layer: X1DebugLayer,
    ) -> (u32, u32) {
        let height = inputs.surface_height();
        self.clear_latched_frame();
        for line in 0..height {
            self.latch_scanline(inputs, line);
        }
        match layer {
            X1DebugLayer::Full => self.present_latched_frame(height),
            X1DebugLayer::Text => self.present_debug(height, |lut, _cg, text| {
                let _ = lut;
                text
            }),
            X1DebugLayer::Bitmap => {
                // The priority table maps a transparent text pixel straight to
                // the remapped graphics colour.
                self.present_debug(height, |lut, cg, _text| lut[usize::from(cg) * 8])
            }
        }
    }

    fn present_debug(&mut self, height: usize, entry: impl Fn(&[u8], u8, u8) -> u8) -> (u32, u32) {
        let height = height.min(X1_MAX_HEIGHT);
        let hscale = self.frame_hscale.max(1);
        for dest_line in 0..X1_MAX_HEIGHT {
            let framebuffer_start = dest_line * X1_SURFACE_WIDTH * X1_PIXEL_BYTES;
            let framebuffer_row = &mut self.framebuffer
                [framebuffer_start..framebuffer_start + X1_SURFACE_WIDTH * X1_PIXEL_BYTES];
            if dest_line >= height {
                for pixel in framebuffer_row.chunks_exact_mut(X1_PIXEL_BYTES) {
                    pixel.copy_from_slice(&BLACK);
                }
                continue;
            }
            let row_start = dest_line * X1_SURFACE_WIDTH;
            let lut_start = dest_line * PRI_LUT_SIZE;
            let lut = &self.pri_lines[lut_start..lut_start + PRI_LUT_SIZE];
            for x in 0..X1_SURFACE_WIDTH / hscale {
                let text_color = self.text[row_start + x] & 7;
                let cg_color = self.cg[row_start + x] & 7;
                let index = entry(lut, cg_color, text_color);
                let color = FIXED_RGBA[usize::from(index & 0x0F)];
                for h in 0..hscale {
                    let offset = (x * hscale + h) * X1_PIXEL_BYTES;
                    framebuffer_row[offset..offset + X1_PIXEL_BYTES].copy_from_slice(&color);
                }
            }
        }
        (X1_SURFACE_WIDTH as u32, height as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE: [u8; 4] = [0xFF, 0xFF, 0xFF, 0xFF];
    const BLUE: [u8; 4] = [0x00, 0x00, 0xFF, 0xFF];
    const RED: [u8; 4] = [0xFF, 0x00, 0x00, 0xFF];

    /// The identity gun latches: each graphics colour maps to itself.
    const IDENTITY_GUNS: [u8; 3] = [0xAA, 0xCC, 0xF0];

    struct Memory {
        text: Vec<u8>,
        attr: Vec<u8>,
        kvram: Vec<u8>,
        pcg: Vec<u8>,
        gaiji: Vec<u8>,
        ank: Vec<u8>,
        kanji: Vec<u8>,
        bitmap: Vec<u8>,
        font: Vec<u8>,
    }

    impl Memory {
        fn new() -> Self {
            Self {
                text: vec![0u8; 0x800],
                attr: vec![0u8; 0x800],
                kvram: vec![0u8; 0x800],
                pcg: vec![0u8; 0x1800],
                gaiji: vec![0u8; 0x1800],
                ank: vec![0u8; 0x2000],
                kanji: vec![0u8; 0x20000],
                bitmap: vec![0u8; 0x18000],
                font: vec![0u8; 0x800],
            }
        }
    }

    fn inputs(memory: &Memory) -> RenderInputsX1<'_> {
        RenderInputsX1 {
            text_vram: &memory.text,
            attr_vram: &memory.attr,
            kvram: &memory.kvram,
            pcg: &memory.pcg,
            gaiji: &memory.gaiji,
            ank_rom: &memory.ank,
            kanji_rom: &memory.kanji,
            bitmap: &memory.bitmap,
            palette: IDENTITY_GUNS,
            priority: 0x00,
            mode1: 0x00,
            mode2: 0x00,
            cblink: 0x00,
            display_off: false,
            hz_disp: 80,
            vt_disp: 25,
            ch_height: 8,
            st_addr: 0,
            vt_ofs: 0,
            hires: false,
            column40: false,
            model: X1RendererModel::Turbo,
        }
    }

    fn render_frame(renderer: &mut X1Renderer, inputs: &RenderInputsX1<'_>) {
        renderer.clear_latched_frame();
        let height = inputs.surface_height();
        for line in 0..height {
            renderer.latch_scanline(inputs, line);
        }
        renderer.present_latched_frame(height);
    }

    fn pixel(framebuffer: &[u8], x: usize, y: usize) -> [u8; 4] {
        let offset = (y * X1_SURFACE_WIDTH + x) * X1_PIXEL_BYTES;
        framebuffer[offset..offset + 4].try_into().unwrap()
    }

    #[test]
    fn hi_speed_bit_doubles_only_a_narrow_screen() {
        let memory = Memory::new();
        let mut narrow = inputs(&memory);
        narrow.hz_disp = 40;
        assert_eq!(narrow.horizontal_scale(), 1);
        narrow.column40 = true;
        assert_eq!(narrow.horizontal_scale(), 2);

        let mut wide = inputs(&memory);
        wide.column40 = true;
        wide.hz_disp = 80;
        assert_eq!(wide.horizontal_scale(), 1);
    }

    #[test]
    fn ank_glyph_draws_in_the_cell_colour() {
        let mut memory = Memory::new();
        memory.font[8] = 0xFF; // glyph 1, row 0 all lit
        memory.text[0] = 1;
        memory.attr[0] = 0x07;

        let mut renderer = X1Renderer::new(&memory.font);
        render_frame(&mut renderer, &inputs(&memory));

        assert_eq!(pixel(renderer.framebuffer(), 0, 0), WHITE);
        assert_eq!(pixel(renderer.framebuffer(), 7, 0), WHITE);
        assert_eq!(pixel(renderer.framebuffer(), 0, 1), BLACK);
    }

    #[test]
    fn reverse_makes_lit_glyph_pixels_transparent() {
        let mut memory = Memory::new();
        memory.font[8] = 0xF0; // glyph 1, row 0: left half lit
        memory.text[0] = 1;
        memory.attr[0] = 0x08 | 0x07; // reverse, colour 7
        memory.bitmap[0] = 0xFF; // graphics colour 1 underneath

        let mut renderer = X1Renderer::new(&memory.font);
        render_frame(&mut renderer, &inputs(&memory));

        // Lit glyph pixels invert to text colour 0 and let the graphics
        // (colour 1 = blue) show; unlit pixels become opaque colour 7.
        assert_eq!(pixel(renderer.framebuffer(), 0, 0), BLUE);
        assert_eq!(pixel(renderer.framebuffer(), 7, 0), WHITE);
    }

    #[test]
    fn priority_register_flips_text_and_bitmap() {
        let mut memory = Memory::new();
        memory.attr[0] = 0x08; // reversed colour 0: opaque white cell
        memory.bitmap[0] = 0xFF; // graphics colour 1

        let mut renderer = X1Renderer::new(&memory.font);
        let mut frame = inputs(&memory);
        frame.palette = [0x02, 0x00, 0x00]; // graphics colour 1 -> blue

        frame.priority = 0x00;
        render_frame(&mut renderer, &frame);
        assert_eq!(pixel(renderer.framebuffer(), 0, 0), WHITE);

        frame.priority = 0xFF;
        render_frame(&mut renderer, &frame);
        assert_eq!(pixel(renderer.framebuffer(), 0, 0), BLUE);
    }

    #[test]
    fn blink_reverses_the_cell_at_the_32_frame_phase() {
        let mut memory = Memory::new();
        memory.font[8] = 0xFF;
        memory.text[0] = 1;
        memory.attr[0] = 0x10 | 0x07; // blink, colour 7

        let mut renderer = X1Renderer::new(&memory.font);
        let mut frame = inputs(&memory);
        frame.cblink = 0x00;
        render_frame(&mut renderer, &frame);
        assert_eq!(pixel(renderer.framebuffer(), 0, 0), WHITE);

        frame.cblink = 0x20;
        render_frame(&mut renderer, &frame);
        // Reversed phase: the lit pixels turn transparent (black background).
        assert_eq!(pixel(renderer.framebuffer(), 0, 0), BLACK);
        assert_eq!(pixel(renderer.framebuffer(), 0, 1), WHITE);
    }

    #[test]
    fn double_width_renders_nibble_halves_across_two_cells() {
        let mut memory = Memory::new();
        memory.font[8] = 0xF1; // glyph 1, row 0: 1111_0001
        memory.text[0] = 1;
        memory.attr[0] = 0x80 | 0x07;
        memory.text[1] = 1;
        memory.attr[1] = 0x80 | 0x07;

        let mut renderer = X1Renderer::new(&memory.font);
        render_frame(&mut renderer, &inputs(&memory));

        // Cell 0 doubles the upper nibble (all lit).
        for x in 0..8 {
            assert_eq!(pixel(renderer.framebuffer(), x, 0), WHITE, "x={x}");
        }
        // Cell 1 (odd address) continues with the lower nibble: only the low
        // bit is lit, doubled into the last two pixels.
        for x in 8..14 {
            assert_eq!(pixel(renderer.framebuffer(), x, 0), BLACK, "x={x}");
        }
        assert_eq!(pixel(renderer.framebuffer(), 14, 0), WHITE);
        assert_eq!(pixel(renderer.framebuffer(), 15, 0), WHITE);
    }

    #[test]
    fn double_height_continues_the_raster_counter_across_rows() {
        let mut memory = Memory::new();
        memory.font[8] = 0xFF; // glyph 1, row 0
        memory.font[2 * 8 + 4] = 0xFF; // glyph 2, row 4
        for x in 0..80 {
            memory.text[x] = 1;
            memory.attr[x] = 0x40 | 0x07;
            memory.text[80 + x] = 2;
            memory.attr[80 + x] = 0x40 | 0x07;
        }

        let mut renderer = X1Renderer::new(&memory.font);
        render_frame(&mut renderer, &inputs(&memory));

        // Rows double vertically: glyph row 0 covers scanlines 0 and 1.
        assert_eq!(pixel(renderer.framebuffer(), 0, 0), WHITE);
        assert_eq!(pixel(renderer.framebuffer(), 0, 1), WHITE);
        assert_eq!(pixel(renderer.framebuffer(), 0, 2), BLACK);
        // The second character row continues at glyph row 4, even though its
        // codes differ from the row above.
        assert_eq!(pixel(renderer.framebuffer(), 0, 8), WHITE);
        assert_eq!(pixel(renderer.framebuffer(), 0, 9), WHITE);
        assert_eq!(pixel(renderer.framebuffer(), 0, 10), BLACK);
    }

    #[test]
    fn pcg_glyph_uses_three_colour_planes() {
        let mut memory = Memory::new();
        memory.text[0] = 2;
        memory.attr[0] = 0x20 | 0x07;
        memory.pcg[2 * 8 + 0x800] = 0xFF; // red plane, glyph 2 row 0

        let mut renderer = X1Renderer::new(&memory.font);
        render_frame(&mut renderer, &inputs(&memory));

        assert_eq!(pixel(renderer.framebuffer(), 0, 0), RED);
    }

    #[test]
    fn gaiji_glyph_reads_sixteen_rows_per_code_pair() {
        let mut memory = Memory::new();
        memory.text[0] = 0x42;
        memory.attr[0] = 0x20 | 0x01; // PCG select, blue only
        memory.kvram[0] = 0x10; // gaiji
        memory.gaiji[(0x42 >> 1) * 16] = 0x80; // row 0
        memory.gaiji[(0x42 >> 1) * 16 + 1] = 0x40; // row 1

        let mut renderer = X1Renderer::new(&memory.font);
        let mut frame = inputs(&memory);
        frame.ch_height = 16;
        frame.mode1 = 0x01; // 16-line text: glyph rows map one-to-one
        render_frame(&mut renderer, &frame);

        assert_eq!(pixel(renderer.framebuffer(), 0, 0), BLUE);
        assert_eq!(pixel(renderer.framebuffer(), 1, 1), BLUE);
        assert_eq!(pixel(renderer.framebuffer(), 0, 1), BLACK);
    }

    #[test]
    fn kanji_glyph_compresses_sixteen_rows_into_eight_scanlines() {
        let mut memory = Memory::new();
        memory.text[0] = 5;
        memory.attr[0] = 0x07;
        memory.kvram[0] = 0x80; // kanji enable, bank 0, left half
        memory.kanji[(5 << 1) * 16] = 0xFF; // glyph row 0
        memory.kanji[(5 << 1) * 16 + 2] = 0xFF; // glyph row 2

        let mut renderer = X1Renderer::new(&memory.font);
        render_frame(&mut renderer, &inputs(&memory));

        // 8-line cell: scanline N samples glyph row 2 * N.
        assert_eq!(pixel(renderer.framebuffer(), 0, 0), WHITE);
        assert_eq!(pixel(renderer.framebuffer(), 0, 1), WHITE);
        assert_eq!(pixel(renderer.framebuffer(), 0, 2), BLACK);
    }

    #[test]
    fn cg_stride_mode_switches_the_line_banks() {
        let mut memory = Memory::new();
        memory.bitmap[0x400] = 0x80; // 0x400-stride: line 1, cell 0
        memory.bitmap[0x800] = 0x80; // 0x800-stride: line 1, cell 0

        let mut renderer = X1Renderer::new(&memory.font);
        let mut frame = inputs(&memory);
        frame.priority = 0xFF;

        frame.mode1 = 0x00;
        render_frame(&mut renderer, &frame);
        assert_eq!(pixel(renderer.framebuffer(), 0, 1), BLUE);

        frame.mode1 = 0x04;
        render_frame(&mut renderer, &frame);
        // The 0x400 stride reads the same line from the narrow bank instead.
        assert_eq!(pixel(renderer.framebuffer(), 0, 1), BLUE);
        // Line 2 now reads bank 0x800, which carries the other marker.
        assert_eq!(pixel(renderer.framebuffer(), 0, 2), BLUE);
    }

    #[test]
    fn cg_page_bit_selects_the_second_bank() {
        let mut memory = Memory::new();
        memory.bitmap[0] = 0x80; // page 0, colour 1
        memory.bitmap[0xC000] = 0x40; // page 1: pixel 1, colour 1

        let mut renderer = X1Renderer::new(&memory.font);
        let mut frame = inputs(&memory);
        frame.priority = 0xFF;

        frame.mode1 = 0x00;
        render_frame(&mut renderer, &frame);
        assert_eq!(pixel(renderer.framebuffer(), 0, 0), BLUE);
        assert_eq!(pixel(renderer.framebuffer(), 1, 0), BLACK);

        frame.mode1 = 0x08;
        render_frame(&mut renderer, &frame);
        assert_eq!(pixel(renderer.framebuffer(), 0, 0), BLACK);
        assert_eq!(pixel(renderer.framebuffer(), 1, 0), BLUE);
    }

    #[test]
    fn hires_interleaves_the_pages_unless_disabled() {
        let mut memory = Memory::new();
        memory.bitmap[0] = 0x80; // page 0 line bank 0
        memory.bitmap[0xC000] = 0x40; // page 1 line bank 0

        let mut renderer = X1Renderer::new(&memory.font);
        let mut frame = inputs(&memory);
        frame.priority = 0xFF;
        frame.hires = true;
        frame.ch_height = 16;

        // Interleaved: even rasters read page 0, odd rasters page 1.
        render_frame(&mut renderer, &frame);
        assert_eq!(pixel(renderer.framebuffer(), 0, 0), BLUE);
        assert_eq!(pixel(renderer.framebuffer(), 1, 0), BLACK);
        assert_eq!(pixel(renderer.framebuffer(), 0, 1), BLACK);
        assert_eq!(pixel(renderer.framebuffer(), 1, 1), BLUE);

        // Interleave disabled: both rasters read the selected page doubled.
        frame.mode1 = 0x02;
        render_frame(&mut renderer, &frame);
        assert_eq!(pixel(renderer.framebuffer(), 0, 0), BLUE);
        assert_eq!(pixel(renderer.framebuffer(), 0, 1), BLUE);
        assert_eq!(pixel(renderer.framebuffer(), 1, 1), BLACK);
    }

    #[test]
    fn kanji_underline_mode_transfers_the_underline_to_the_graphics_layer() {
        let mut memory = Memory::new();
        memory.font[8] = 0xFF;
        memory.text[0] = 1;
        memory.attr[0] = 0x07;
        memory.kvram[0] = 0x20; // underline
        memory.bitmap[0] = 0xFF; // graphics content is not displayed in KSEN

        let mut renderer = X1Renderer::new(&memory.font);
        let mut frame = inputs(&memory);
        frame.mode1 = 0x80; // kanji-underline mode, 8-line fonts
        frame.ch_height = 16;
        render_frame(&mut renderer, &frame);

        // Normal glyph rows still render; the graphics planes are blanked.
        assert_eq!(pixel(renderer.framebuffer(), 0, 0), WHITE);
        assert_eq!(pixel(renderer.framebuffer(), 0, 2), BLACK);
        // The underline appears on raster 9 as graphics colour 1.
        assert_eq!(pixel(renderer.framebuffer(), 0, 9), BLUE);
        assert_eq!(pixel(renderer.framebuffer(), 0, 8), BLACK);
    }

    #[test]
    fn vt_ofs_shifts_the_content_down() {
        let mut memory = Memory::new();
        memory.font[8] = 0xFF;
        memory.text[0] = 1;
        memory.attr[0] = 0x07;

        let mut renderer = X1Renderer::new(&memory.font);
        let mut frame = inputs(&memory);
        frame.vt_ofs = 2;
        render_frame(&mut renderer, &frame);

        assert_eq!(pixel(renderer.framebuffer(), 0, 0), BLACK);
        assert_eq!(pixel(renderer.framebuffer(), 0, 1), BLACK);
        assert_eq!(pixel(renderer.framebuffer(), 0, 2), WHITE);
    }

    #[test]
    fn display_off_presents_black() {
        let mut memory = Memory::new();
        memory.font[8] = 0xFF;
        memory.text[0] = 1;
        memory.attr[0] = 0x07;
        memory.bitmap[0] = 0xFF;

        let mut renderer = X1Renderer::new(&memory.font);
        let mut frame = inputs(&memory);
        frame.priority = 0xFF;
        frame.display_off = true;
        render_frame(&mut renderer, &frame);

        assert_eq!(pixel(renderer.framebuffer(), 0, 0), BLACK);
    }

    #[test]
    fn column40_on_an_80_column_screen_still_draws_the_right_half() {
        let mut memory = Memory::new();
        memory.font[8] = 0x80;
        memory.text[79] = 1;
        memory.attr[79] = 0x07;
        memory.bitmap[79] = 0x01; // graphics colour 1, rightmost cell pixel

        let mut renderer = X1Renderer::new(&memory.font);
        let mut frame = inputs(&memory);
        frame.column40 = true; // 320-dot bit asserted at 80 columns (Arcus)
        frame.priority = 0x02;
        render_frame(&mut renderer, &frame);

        assert_eq!(pixel(renderer.framebuffer(), 632, 0), WHITE);
        assert_eq!(pixel(renderer.framebuffer(), 639, 0), BLUE);
    }

    #[test]
    fn column40_doubles_the_pixels_at_present_time() {
        let mut memory = Memory::new();
        memory.font[8] = 0x80; // single lit pixel
        memory.text[0] = 1;
        memory.attr[0] = 0x07;

        let mut renderer = X1Renderer::new(&memory.font);
        let mut frame = inputs(&memory);
        frame.hz_disp = 40;
        frame.column40 = true;
        render_frame(&mut renderer, &frame);

        assert_eq!(pixel(renderer.framebuffer(), 0, 0), WHITE);
        assert_eq!(pixel(renderer.framebuffer(), 1, 0), WHITE);
        assert_eq!(pixel(renderer.framebuffer(), 2, 0), BLACK);
    }

    #[test]
    fn rows_below_the_displayed_area_stay_black() {
        let mut memory = Memory::new();
        memory.font[8] = 0xFF;
        memory.text.fill(1);
        memory.attr.fill(0x07);

        let mut renderer = X1Renderer::new(&memory.font);
        let mut frame = inputs(&memory);
        frame.vt_disp = 10; // 80 displayed scanlines
        render_frame(&mut renderer, &frame);

        assert_eq!(pixel(renderer.framebuffer(), 0, 72), WHITE);
        assert_eq!(pixel(renderer.framebuffer(), 0, 80), BLACK);
    }
}
