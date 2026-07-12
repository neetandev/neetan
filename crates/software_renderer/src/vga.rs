//! VGA display renderer for the PC/AT.
//!
//! Rasters the alphanumeric mode, the 16-color planar modes, the 256-color
//! packed modes and the CGA compatibility modes from a register snapshot plus
//! the dword-interleaved display memory (plane `p` of plane offset `o` at
//! `vram[o * 4 + p]`). Scanlines are composed into a line buffer first so
//! horizontal pel panning is a plain copy offset.

#[cfg(target_arch = "x86_64")]
#[allow(unsafe_code)]
mod avx2;
mod cga;
#[cfg(target_arch = "aarch64")]
#[allow(unsafe_code)]
mod neon;
mod packed;
mod planar;
mod text;

use alloc::{boxed::Box, vec};

/// VGA visible surface width in pixels. Sized for the ET4000 800x600 mode;
/// lower-resolution frames occupy the top-left sub-rectangle.
pub const VGA_SURFACE_WIDTH: usize = 800;
/// VGA visible surface maximum height in pixels.
pub const VGA_SURFACE_HEIGHT: usize = 600;
/// Bytes per pixel (`R, G, B, A`).
pub const VGA_PIXEL_BYTES: usize = 4;
/// VGA framebuffer byte size.
pub const VGA_FRAMEBUFFER_BYTES: usize = VGA_SURFACE_WIDTH * VGA_SURFACE_HEIGHT * VGA_PIXEL_BYTES;

/// Fallback frame width before the first mode set (70 Hz text timing).
pub const VGA_FALLBACK_WIDTH: u32 = 720;
/// Fallback frame height before the first mode set.
pub const VGA_FALLBACK_HEIGHT: u32 = 400;

/// Opaque black, packed RGBA.
const BACKDROP: u32 = 0xFF00_0000;

/// Border ring thickness in dots when the overscan color is visible.
const BORDER_DOTS: u32 = 8;

/// Line buffer length in dots: the widest scanline plus pel panning slack.
const LINE_BUFFER_DOTS: usize = VGA_SURFACE_WIDTH + 16;

/// How the scan-out interprets display memory.
///
/// Mirrors the machine-side render mode classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VgaRenderMode {
    /// Alphanumeric mode through the character generator.
    Text,
    /// 16-color planar graphics (EGA and VGA planar modes).
    Planar16,
    /// 256-color packed pixel graphics (mode 13h, Mode X and the SVGA modes).
    Packed256,
    /// CGA compatible 4-color graphics through the interleaved shift register.
    CgaInterleaved,
    /// One bit per pixel graphics from plane zero (CGA mode 06h).
    Mono1bpp,
}

/// Register-derived scan-out state for one frame, in plane address units.
///
/// The fields mirror the machine-side resolved frame snapshot; display memory
/// is borrowed for the duration of the render call.
pub struct RenderInputsVga<'a> {
    /// Display memory, dword-interleaved across the four planes.
    pub vram: &'a [u8],
    /// How the scan-out interprets display memory.
    pub render_mode: VgaRenderMode,
    /// The screen is blanked.
    pub blanked: bool,
    /// Active character columns per row.
    pub columns: u32,
    /// Character cell width in dots (8 or 9).
    pub character_width: u32,
    /// Character cell height in scanlines; in graphics modes the number of
    /// times each memory row is scanned before the row advances.
    pub character_height: u32,
    /// Every row scan is emitted twice.
    pub scan_doubled: bool,
    /// Active scanlines per frame.
    pub active_scanlines: u32,
    /// Display start address in plane address units (addressing mode applied).
    pub start_address: u32,
    /// Plane address advance per character or memory row.
    pub row_pitch: u32,
    /// Plane address advance per character clock (2 in word mode, 1 in byte
    /// and doubleword modes).
    pub address_step: u32,
    /// Address mask for one plane in the selected VGA mapping mode.
    pub plane_address_mask: u32,
    /// Address bit 13 is replaced by row scan bit 0 (CGA interleave).
    pub map13_from_row_scan: bool,
    /// Address bit 14 is replaced by row scan bit 1.
    pub map14_from_row_scan: bool,
    /// Scanline at which the fetch address resets to zero (split screen).
    pub line_compare: u32,
    /// Pel panning is forced to zero below the split screen line.
    pub pel_pan_reset_on_split: bool,
    /// Row scan value the first displayed character row starts at.
    pub preset_row_scan: u8,
    /// Hardware cursor location in plane address units.
    pub cursor_address: u32,
    /// First character scanline of the cursor block.
    pub cursor_start_row: u8,
    /// Last character scanline of the cursor block.
    pub cursor_end_row: u8,
    /// The cursor is enabled and in its visible blink phase.
    pub cursor_visible: bool,
    /// Attribute bit 7 selects blink instead of a bright background.
    pub blink_enabled: bool,
    /// Blinking characters are in their visible phase.
    pub blink_visible: bool,
    /// Line graphics characters 0xC0-0xDF extend into the ninth dot.
    pub line_graphics: bool,
    /// Font plane offset used when attribute bit 3 is set.
    pub font_offset_map_a: u32,
    /// Font plane offset used when attribute bit 3 is clear.
    pub font_offset_map_b: u32,
    /// Horizontal pel panning value.
    pub pel_pan: u8,
    /// 256-color pixels are emitted for two dot clocks each (mode 13h rate);
    /// clear for the ET4000 one-pixel-per-dot SVGA modes.
    pub packed_half_rate: bool,
    /// Border color around the active area, packed RGBA.
    pub border_color: u32,
    /// The sixteen attribute colors resolved to packed RGBA.
    pub pens: [u32; 16],
    /// The full DAC palette resolved to packed RGBA (256-color modes).
    pub pens_256: [u32; 256],
}

/// Placement of the active area within the output frame.
#[derive(Debug, Clone, Copy)]
pub(super) struct FrameLayout {
    /// Active area width in dots.
    content_width: u32,
    /// Active area height in scanlines.
    content_height: u32,
    /// Border ring thickness in dots.
    border: u32,
    /// Total output frame width.
    total_width: u32,
    /// Total output frame height.
    total_height: u32,
}

/// Split screen aware fetch state for one output scanline.
#[derive(Debug, Clone, Copy)]
pub(super) struct LinePosition {
    /// Memory character or scan row index from the fetch base.
    row: u32,
    /// Row scan value within the character or memory row.
    row_scan: u32,
    /// The scanline lies at or below the split screen line.
    below_split: bool,
}

/// CPU-side renderer for the VGA scan-out.
pub struct VgaRenderer {
    /// Packed RGBA framebuffer sized for the maximum surface.
    framebuffer: Box<[u8]>,
    /// Scanline compose buffer reused across rows.
    line_buffer: [u32; LINE_BUFFER_DOTS],
    /// Cached SIMD availability (AVX2 on x86_64, NEON on aarch64).
    has_simd: bool,
    /// Frame width of the most recent unblanked frame.
    last_width: u32,
    /// Frame height of the most recent unblanked frame.
    last_height: u32,
}

impl Default for VgaRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl VgaRenderer {
    /// Creates the renderer with a black framebuffer at the fallback size.
    pub fn new() -> Self {
        let mut renderer = Self {
            framebuffer: vec![0; VGA_FRAMEBUFFER_BYTES].into_boxed_slice(),
            line_buffer: [BACKDROP; LINE_BUFFER_DOTS],
            has_simd: crate::detect_simd(),
            last_width: VGA_FALLBACK_WIDTH,
            last_height: VGA_FALLBACK_HEIGHT,
        };
        renderer.fill_region(VGA_FALLBACK_WIDTH, VGA_FALLBACK_HEIGHT, BACKDROP);
        renderer
    }

    /// Enables or disables the SIMD dispatch for the packed 256-color path.
    ///
    /// Intended for parity testing the scalar fallback against the SIMD path;
    /// production callers should leave the renderer at its default.
    pub fn set_simd_enabled(&mut self, enabled: bool) {
        self.has_simd = enabled && crate::detect_simd();
    }

    /// The packed RGBA framebuffer; rows are `width` pixels tightly packed.
    pub fn framebuffer(&self) -> &[u8] {
        &self.framebuffer
    }

    /// Renders one frame and returns its `(width, height)`.
    pub fn render(&mut self, inputs: &RenderInputsVga) -> (u32, u32) {
        let layout = frame_layout(inputs);
        if inputs.blanked || layout.content_width == 0 || layout.content_height == 0 {
            let (width, height) = (self.last_width, self.last_height);
            self.fill_region(width, height, BACKDROP);
            return (width, height);
        }
        self.last_width = layout.total_width;
        self.last_height = layout.total_height;
        if layout.border > 0 {
            self.fill_region(layout.total_width, layout.total_height, inputs.border_color);
        }
        match inputs.render_mode {
            VgaRenderMode::Text => self.render_text(inputs, &layout),
            VgaRenderMode::Planar16 => self.render_planar(inputs, &layout),
            VgaRenderMode::Packed256 => self.render_packed(inputs, &layout),
            VgaRenderMode::CgaInterleaved => self.render_cga(inputs, &layout),
            VgaRenderMode::Mono1bpp => self.render_mono(inputs, &layout),
        }
        (layout.total_width, layout.total_height)
    }

    /// Fills the top-left `width` x `height` region with a solid color.
    fn fill_region(&mut self, width: u32, height: u32, color: u32) {
        let bytes = (width * height) as usize * VGA_PIXEL_BYTES;
        for pixel in self.framebuffer[..bytes].chunks_exact_mut(VGA_PIXEL_BYTES) {
            pixel.copy_from_slice(&color.to_le_bytes());
        }
    }

    /// Copies the composed line buffer into a framebuffer row of the active
    /// area, applying the pel panning offset and the border placement.
    fn commit_scanline(&mut self, y: u32, layout: &FrameLayout, pan: u32) {
        let row_start =
            ((y + layout.border) * layout.total_width + layout.border) as usize * VGA_PIXEL_BYTES;
        let row = &mut self.framebuffer
            [row_start..row_start + layout.content_width as usize * VGA_PIXEL_BYTES];
        for (pixel, dot) in row
            .chunks_exact_mut(VGA_PIXEL_BYTES)
            .zip(self.line_buffer[pan as usize..].iter())
        {
            pixel.copy_from_slice(&dot.to_le_bytes());
        }
    }
}

/// The output frame layout for the current mode, clamped to the surface.
fn frame_layout(inputs: &RenderInputsVga) -> FrameLayout {
    let dots_per_column = match inputs.render_mode {
        VgaRenderMode::Text => inputs.character_width,
        VgaRenderMode::Planar16
        | VgaRenderMode::Packed256
        | VgaRenderMode::CgaInterleaved
        | VgaRenderMode::Mono1bpp => 8,
    };
    let content_width = (inputs.columns * dots_per_column).min(VGA_SURFACE_WIDTH as u32);
    let content_height = inputs.active_scanlines.min(VGA_SURFACE_HEIGHT as u32);
    let border_visible = inputs.border_color != BACKDROP;
    let border_fits = content_width + 2 * BORDER_DOTS <= VGA_SURFACE_WIDTH as u32
        && content_height + 2 * BORDER_DOTS <= VGA_SURFACE_HEIGHT as u32;
    let border = if border_visible && border_fits {
        BORDER_DOTS
    } else {
        0
    };
    FrameLayout {
        content_width,
        content_height,
        border,
        total_width: content_width + 2 * border,
        total_height: content_height + 2 * border,
    }
}

/// The fetch state for an output scanline, honoring the split screen, scan
/// doubling and the preset row scan.
fn line_position(inputs: &RenderInputsVga, y: u32) -> LinePosition {
    let scan_factor = if inputs.scan_doubled { 2 } else { 1 };
    let repeat = (inputs.character_height * scan_factor).max(1);
    let below_split = y >= inputs.line_compare && inputs.line_compare < inputs.active_scanlines;
    let relative = if below_split {
        y - inputs.line_compare
    } else {
        y + u32::from(inputs.preset_row_scan) * scan_factor
    };
    LinePosition {
        row: relative / repeat,
        row_scan: (relative % repeat) / scan_factor,
        below_split,
    }
}

/// The fetch base plane address of a scanline (zero below the split screen).
fn line_row_base(inputs: &RenderInputsVga, position: &LinePosition) -> u32 {
    let base = if position.below_split {
        0
    } else {
        inputs.start_address
    };
    base + position.row * inputs.row_pitch
}

/// The pel panning applied to a scanline (reset below the split screen when
/// the attribute controller pixel panning compatibility bit is set).
fn line_pan(inputs: &RenderInputsVga, position: &LinePosition, pan: u32) -> u32 {
    if position.below_split && inputs.pel_pan_reset_on_split {
        0
    } else {
        pan
    }
}

/// Replaces plane address bits 13 and 14 with row scan bits where the CRTC
/// mode control selects the CGA compatible substitutions.
fn substitute_row_scan(inputs: &RenderInputsVga, address: u32, row_scan: u32) -> u32 {
    let mut address = address;
    if inputs.map13_from_row_scan {
        address = (address & !(1 << 13)) | ((row_scan & 0x01) << 13);
    }
    if inputs.map14_from_row_scan {
        address = (address & !(1 << 14)) | ((row_scan & 0x02) << 13);
    }
    address
}

/// The effective pel panning shift in dots.
///
/// Nine-dot cells shift by the register value plus one, with the value eight
/// meaning no shift (the neutral value text mode sets).
fn effective_pel_pan(pel_pan: u8, character_width: u32) -> u32 {
    if character_width == 9 {
        if pel_pan >= 8 {
            0
        } else {
            u32::from(pel_pan) + 1
        }
    } else {
        u32::from(pel_pan & 0x07)
    }
}

/// Clamps the selected plane address mask to the available display memory.
fn plane_address_mask(inputs: &RenderInputsVga) -> u32 {
    inputs
        .plane_address_mask
        .min((inputs.vram.len() / 4) as u32 - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal text mode input over the given display memory.
    ///
    /// Text mode uses word addressing, so cells sit at every second plane
    /// address and the addresses scale by two.
    fn text_inputs(vram: &[u8]) -> RenderInputsVga<'_> {
        let mut pens = [0xFF00_0000; 16];
        pens[7] = 0xFFAA_AAAA;
        pens[15] = 0xFFFF_FFFF;
        RenderInputsVga {
            vram,
            render_mode: VgaRenderMode::Text,
            blanked: false,
            columns: 80,
            character_width: 9,
            character_height: 16,
            scan_doubled: false,
            active_scanlines: 400,
            start_address: 0,
            row_pitch: 160,
            address_step: 2,
            plane_address_mask: 0xFFFF,
            map13_from_row_scan: false,
            map14_from_row_scan: false,
            line_compare: 0x3FF,
            pel_pan_reset_on_split: false,
            preset_row_scan: 0,
            cursor_address: 0,
            cursor_start_row: 13,
            cursor_end_row: 14,
            cursor_visible: false,
            blink_enabled: true,
            blink_visible: true,
            line_graphics: true,
            font_offset_map_a: 0,
            font_offset_map_b: 0,
            pel_pan: 8,
            packed_half_rate: true,
            border_color: 0xFF00_0000,
            pens,
            pens_256: [0xFF00_0000; 256],
        }
    }

    /// A minimal 16-color planar input over the given display memory.
    fn planar_inputs(vram: &[u8]) -> RenderInputsVga<'_> {
        let mut pens = [0xFF00_0000u32; 16];
        for (index, pen) in pens.iter_mut().enumerate() {
            *pen = 0xFF00_0000 | index as u32;
        }
        RenderInputsVga {
            vram,
            render_mode: VgaRenderMode::Planar16,
            blanked: false,
            columns: 80,
            character_width: 8,
            character_height: 1,
            scan_doubled: false,
            active_scanlines: 480,
            start_address: 0,
            row_pitch: 80,
            address_step: 1,
            plane_address_mask: 0xFFFF,
            map13_from_row_scan: false,
            map14_from_row_scan: false,
            line_compare: 0x3FF,
            pel_pan_reset_on_split: false,
            preset_row_scan: 0,
            cursor_address: 0,
            cursor_start_row: 0,
            cursor_end_row: 0,
            cursor_visible: false,
            blink_enabled: false,
            blink_visible: true,
            line_graphics: false,
            font_offset_map_a: 0,
            font_offset_map_b: 0,
            pel_pan: 0,
            packed_half_rate: true,
            border_color: 0xFF00_0000,
            pens,
            pens_256: [0xFF00_0000; 256],
        }
    }

    /// Writes a text cell into interleaved display memory (word addressing).
    fn put_cell(vram: &mut [u8], cell: usize, character: u8, attribute: u8) {
        vram[cell * 2 * 4] = character;
        vram[cell * 2 * 4 + 1] = attribute;
    }

    /// Writes one font glyph row into plane 2.
    fn put_font_row(vram: &mut [u8], character: u8, row: usize, bits: u8) {
        vram[(character as usize * 32 + row) * 4 + 2] = bits;
    }

    fn pixel(renderer: &VgaRenderer, width: u32, x: u32, y: u32) -> u32 {
        let offset = ((y * width + x) as usize) * VGA_PIXEL_BYTES;
        u32::from_le_bytes(
            renderer.framebuffer()[offset..offset + VGA_PIXEL_BYTES]
                .try_into()
                .unwrap(),
        )
    }

    #[test]
    fn text_glyph_rasters_at_cell_origin() {
        let mut vram = vec![0u8; 0x10_0000];
        put_cell(&mut vram, 0, 1, 0x07);
        put_font_row(&mut vram, 1, 0, 0b1000_0001);
        let inputs = text_inputs(&vram);
        let mut renderer = VgaRenderer::new();
        let (width, height) = renderer.render(&inputs);
        assert_eq!((width, height), (720, 400));
        assert_eq!(pixel(&renderer, width, 0, 0), inputs.pens[7]);
        assert_eq!(pixel(&renderer, width, 1, 0), inputs.pens[0]);
        assert_eq!(pixel(&renderer, width, 7, 0), inputs.pens[7]);
        assert_eq!(pixel(&renderer, width, 8, 0), inputs.pens[0]);
    }

    #[test]
    fn ninth_dot_replicates_only_line_graphics_characters() {
        let mut vram = vec![0u8; 0x10_0000];
        put_cell(&mut vram, 0, 0xC4, 0x07);
        put_font_row(&mut vram, 0xC4, 0, 0xFF);
        put_cell(&mut vram, 1, b'X', 0x07);
        put_font_row(&mut vram, b'X', 0, 0xFF);
        let inputs = text_inputs(&vram);
        let mut renderer = VgaRenderer::new();
        let (width, _) = renderer.render(&inputs);
        // The line graphics character extends into the ninth dot.
        assert_eq!(pixel(&renderer, width, 8, 0), inputs.pens[7]);
        // The ordinary character shows background there.
        assert_eq!(pixel(&renderer, width, 17, 0), inputs.pens[0]);

        let mut inputs = text_inputs(&vram);
        inputs.line_graphics = false;
        let (width, _) = renderer.render(&inputs);
        assert_eq!(pixel(&renderer, width, 8, 0), inputs.pens[0]);
    }

    #[test]
    fn cursor_draws_a_solid_foreground_block() {
        let mut vram = vec![0u8; 0x10_0000];
        put_cell(&mut vram, 5, b' ', 0x07);
        let mut inputs = text_inputs(&vram);
        inputs.cursor_address = 10;
        inputs.cursor_visible = true;
        let mut renderer = VgaRenderer::new();
        let (width, _) = renderer.render(&inputs);
        // Above the cursor block the cell shows background.
        assert_eq!(pixel(&renderer, width, 45, 12), inputs.pens[0]);
        // Within the cursor block the whole cell is the foreground color.
        assert_eq!(pixel(&renderer, width, 45, 13), inputs.pens[7]);
        assert_eq!(pixel(&renderer, width, 53, 13), inputs.pens[7]);
        assert_eq!(pixel(&renderer, width, 45, 14), inputs.pens[7]);
        assert_eq!(pixel(&renderer, width, 45, 15), inputs.pens[0]);
    }

    #[test]
    fn preset_row_scan_offsets_the_first_character_row() {
        let mut vram = vec![0u8; 0x10_0000];
        put_cell(&mut vram, 0, 1, 0x07);
        put_font_row(&mut vram, 1, 2, 0x80);
        let mut inputs = text_inputs(&vram);
        inputs.preset_row_scan = 2;
        let mut renderer = VgaRenderer::new();
        let (width, _) = renderer.render(&inputs);
        // Glyph row 2 appears on the first output scanline.
        assert_eq!(pixel(&renderer, width, 0, 0), inputs.pens[7]);
        assert_eq!(pixel(&renderer, width, 0, 1), inputs.pens[0]);
    }

    #[test]
    fn text_split_screen_restarts_at_address_zero() {
        let mut vram = vec![0u8; 0x10_0000];
        // The cell at plane address zero is visible again below the split.
        put_cell(&mut vram, 0, 1, 0x07);
        put_font_row(&mut vram, 1, 0, 0x80);
        let mut inputs = text_inputs(&vram);
        inputs.start_address = 320;
        inputs.line_compare = 32;
        let mut renderer = VgaRenderer::new();
        let (width, _) = renderer.render(&inputs);
        // Above the split the start address skips the first two rows.
        assert_eq!(pixel(&renderer, width, 0, 0), inputs.pens[0]);
        // At the split the fetch restarts at address zero with row scan zero.
        assert_eq!(pixel(&renderer, width, 0, 32), inputs.pens[7]);
        assert_eq!(pixel(&renderer, width, 0, 33), inputs.pens[0]);
    }

    #[test]
    fn blink_hides_the_foreground_in_the_dark_phase() {
        let mut vram = vec![0u8; 0x10_0000];
        put_cell(&mut vram, 0, 1, 0x87);
        put_font_row(&mut vram, 1, 0, 0x80);
        let mut inputs = text_inputs(&vram);
        inputs.blink_visible = false;
        let mut renderer = VgaRenderer::new();
        let (width, _) = renderer.render(&inputs);
        assert_eq!(pixel(&renderer, width, 0, 0), inputs.pens[0]);

        // With blink disabled, attribute bit 7 selects a bright background.
        let mut vram = vec![0u8; 0x10_0000];
        put_cell(&mut vram, 0, 0, 0xF0);
        let mut inputs = text_inputs(&vram);
        inputs.blink_enabled = false;
        let (width, _) = renderer.render(&inputs);
        assert_eq!(pixel(&renderer, width, 0, 0), inputs.pens[15]);
        drop(vram);
    }

    #[test]
    fn blanked_frame_renders_black_at_the_previous_size() {
        let vram = vec![0u8; 0x10_0000];
        let mut inputs = text_inputs(&vram);
        let mut renderer = VgaRenderer::new();
        renderer.render(&inputs);
        inputs.blanked = true;
        let (width, height) = renderer.render(&inputs);
        assert_eq!((width, height), (720, 400));
        assert_eq!(pixel(&renderer, width, 0, 0), BACKDROP);
    }

    #[test]
    fn planar_mode_decodes_four_planes_msb_first() {
        let mut vram = vec![0u8; 0x10_0000];
        // First byte: plane pattern giving pixel colors 0x1, 0x2, 0x4, 0x8
        // on the first four pixels.
        vram[0] = 0b1000_0000;
        vram[1] = 0b0100_0000;
        vram[2] = 0b0010_0000;
        vram[3] = 0b0001_0000;
        let inputs = planar_inputs(&vram);
        let mut renderer = VgaRenderer::new();
        let (width, height) = renderer.render(&inputs);
        assert_eq!((width, height), (640, 480));
        assert_eq!(pixel(&renderer, width, 0, 0), inputs.pens[1]);
        assert_eq!(pixel(&renderer, width, 1, 0), inputs.pens[2]);
        assert_eq!(pixel(&renderer, width, 2, 0), inputs.pens[4]);
        assert_eq!(pixel(&renderer, width, 3, 0), inputs.pens[8]);
        assert_eq!(pixel(&renderer, width, 4, 0), inputs.pens[0]);
    }

    #[test]
    fn planar_scan_doubling_repeats_source_rows() {
        let mut vram = vec![0u8; 0x10_0000];
        vram[0] = 0x80;
        let mut inputs = planar_inputs(&vram);
        inputs.columns = 40;
        inputs.row_pitch = 40;
        inputs.active_scanlines = 400;
        inputs.scan_doubled = true;
        let mut renderer = VgaRenderer::new();
        let (width, _) = renderer.render(&inputs);
        assert_eq!(pixel(&renderer, width, 0, 0), inputs.pens[1]);
        assert_eq!(pixel(&renderer, width, 0, 1), inputs.pens[1]);
        assert_eq!(pixel(&renderer, width, 0, 2), inputs.pens[0]);
    }

    #[test]
    fn ibm_planar_addressing_wraps_each_plane_at_64k() {
        let mut vram = vec![0u8; 0x10_0000];
        vram[0x40 * 4] = 0x80;
        let mut inputs = planar_inputs(&vram);
        inputs.columns = 1;
        inputs.active_scanlines = 2;
        inputs.start_address = 0xFFF0;
        inputs.row_pitch = 80;
        let mut renderer = VgaRenderer::new();
        let (width, _) = renderer.render(&inputs);
        assert_eq!(pixel(&renderer, width, 0, 0), inputs.pens[0]);
        assert_eq!(pixel(&renderer, width, 0, 1), inputs.pens[1]);
    }

    #[test]
    fn ega_200_line_modes_repeat_rows() {
        let mut vram = vec![0u8; 0x10_0000];
        vram[0] = 0x80;
        vram[40 * 4] = 0x80;
        let mut inputs = planar_inputs(&vram);
        inputs.columns = 40;
        inputs.row_pitch = 40;
        inputs.active_scanlines = 400;
        // 200-line EGA modes double scan through the maximum scan line value.
        inputs.character_height = 2;
        let mut renderer = VgaRenderer::new();
        let (width, _) = renderer.render(&inputs);
        assert_eq!(pixel(&renderer, width, 0, 0), inputs.pens[1]);
        assert_eq!(pixel(&renderer, width, 0, 1), inputs.pens[1]);
        assert_eq!(pixel(&renderer, width, 0, 2), inputs.pens[1]);
        assert_eq!(pixel(&renderer, width, 0, 3), inputs.pens[1]);
        assert_eq!(pixel(&renderer, width, 0, 4), inputs.pens[0]);
    }

    #[test]
    fn line_compare_splits_and_resets_start_address() {
        let mut vram = vec![0u8; 0x10_0000];
        vram[0] = 0x80;
        let mut inputs = planar_inputs(&vram);
        inputs.start_address = 80 * 100;
        inputs.line_compare = 100;
        let mut renderer = VgaRenderer::new();
        let (width, _) = renderer.render(&inputs);
        // Above the split the pixel at plane address zero is not visible.
        assert_eq!(pixel(&renderer, width, 0, 0), inputs.pens[0]);
        // Below the split the fetch restarts at plane address zero.
        assert_eq!(pixel(&renderer, width, 0, 100), inputs.pens[1]);
        assert_eq!(pixel(&renderer, width, 0, 101), inputs.pens[0]);
    }

    #[test]
    fn line_compare_bottom_ignores_pel_pan_when_reset_bit_set() {
        let mut vram = vec![0u8; 0x10_0000];
        vram[0] = 0x80;
        vram[80 * 100 * 4] = 0x80;
        let mut inputs = planar_inputs(&vram);
        inputs.start_address = 80 * 100;
        inputs.line_compare = 100;
        inputs.pel_pan = 4;
        inputs.pel_pan_reset_on_split = true;
        let mut renderer = VgaRenderer::new();
        let (width, _) = renderer.render(&inputs);
        // Above the split the panning shifts the leading pixel out by four.
        assert_eq!(pixel(&renderer, width, 0, 0), inputs.pens[0]);
        // Below the split the panning is reset and the pixel is at the edge.
        assert_eq!(pixel(&renderer, width, 0, 100), inputs.pens[1]);
    }

    /// A minimal CGA 4-color input over the given display memory (word
    /// addressing, scanline interleave through address bit 13).
    fn cga_inputs(vram: &[u8]) -> RenderInputsVga<'_> {
        let mut inputs = planar_inputs(vram);
        inputs.render_mode = VgaRenderMode::CgaInterleaved;
        inputs.columns = 40;
        inputs.character_height = 2;
        inputs.scan_doubled = true;
        inputs.active_scanlines = 400;
        inputs.row_pitch = 80;
        inputs.address_step = 2;
        inputs.map13_from_row_scan = true;
        inputs
    }

    /// A minimal 256-color packed input over the given display memory
    /// (mode 13h addressing: 200 double scanned rows of 320 bytes).
    fn packed_inputs(vram: &[u8]) -> RenderInputsVga<'_> {
        let mut inputs = planar_inputs(vram);
        inputs.render_mode = VgaRenderMode::Packed256;
        inputs.columns = 80;
        inputs.character_height = 2;
        inputs.active_scanlines = 400;
        inputs.row_pitch = 80;
        inputs.packed_half_rate = true;
        let mut pens_256 = [0xFF00_0000u32; 256];
        for (index, pen) in pens_256.iter_mut().enumerate() {
            *pen = 0xFF00_0000 | (index as u32) << 8;
        }
        inputs.pens_256 = pens_256;
        inputs
    }

    #[test]
    fn cga4_decodes_two_bit_pixels_msb_first() {
        let mut vram = vec![0u8; 0x10_0000];
        // Even host byte in plane 0: pixels 0, 1, 2, 3.
        vram[0] = 0b0001_1011;
        // Odd host byte in plane 1 of the same plane address: pixels 3 first.
        vram[1] = 0b1110_0100;
        let inputs = cga_inputs(&vram);
        let mut renderer = VgaRenderer::new();
        let (width, height) = renderer.render(&inputs);
        assert_eq!((width, height), (320, 400));
        assert_eq!(pixel(&renderer, width, 0, 0), inputs.pens[0]);
        assert_eq!(pixel(&renderer, width, 1, 0), inputs.pens[1]);
        assert_eq!(pixel(&renderer, width, 2, 0), inputs.pens[2]);
        assert_eq!(pixel(&renderer, width, 3, 0), inputs.pens[3]);
        assert_eq!(pixel(&renderer, width, 4, 0), inputs.pens[3]);
        assert_eq!(pixel(&renderer, width, 7, 0), inputs.pens[0]);
    }

    #[test]
    fn cga_interleave_reads_odd_scanlines_from_0x2000_offset() {
        let mut vram = vec![0u8; 0x10_0000];
        vram[0] = 0xC0;
        vram[0x2000 * 4] = 0x30;
        let inputs = cga_inputs(&vram);
        let mut renderer = VgaRenderer::new();
        let (width, _) = renderer.render(&inputs);
        // Row scans 0 and 1 alternate the 8 KiB half per character row.
        assert_eq!(pixel(&renderer, width, 0, 0), inputs.pens[3]);
        assert_eq!(pixel(&renderer, width, 1, 0), inputs.pens[0]);
        assert_eq!(pixel(&renderer, width, 0, 2), inputs.pens[0]);
        assert_eq!(pixel(&renderer, width, 1, 2), inputs.pens[3]);
    }

    #[test]
    fn cga_double_scan_repeats_rows() {
        let mut vram = vec![0u8; 0x10_0000];
        vram[0] = 0xC0;
        let inputs = cga_inputs(&vram);
        let mut renderer = VgaRenderer::new();
        let (width, _) = renderer.render(&inputs);
        // Scanlines zero and one show the same interleave bank.
        assert_eq!(pixel(&renderer, width, 0, 0), inputs.pens[3]);
        assert_eq!(pixel(&renderer, width, 0, 1), inputs.pens[3]);
        // The second character row starts 80 host bytes in.
        assert_eq!(pixel(&renderer, width, 0, 4), inputs.pens[0]);
    }

    #[test]
    fn cga1_640_decodes_one_bit_pixels() {
        let mut vram = vec![0u8; 0x10_0000];
        // Plane 0 byte: alternating pixels.
        vram[0] = 0b1010_1010;
        vram[0x2000 * 4] = 0b0101_0101;
        let mut inputs = cga_inputs(&vram);
        inputs.render_mode = VgaRenderMode::Mono1bpp;
        inputs.columns = 80;
        inputs.address_step = 1;
        let mut renderer = VgaRenderer::new();
        let (width, height) = renderer.render(&inputs);
        assert_eq!((width, height), (640, 400));
        assert_eq!(pixel(&renderer, width, 0, 0), inputs.pens[1]);
        assert_eq!(pixel(&renderer, width, 1, 0), inputs.pens[0]);
        // The interleaved half supplies the second distinct scanline.
        assert_eq!(pixel(&renderer, width, 0, 2), inputs.pens[0]);
        assert_eq!(pixel(&renderer, width, 1, 2), inputs.pens[1]);
    }

    #[test]
    fn mode13_chain4_maps_sequential_bytes_to_pixels() {
        let mut vram = vec![0u8; 0x10_0000];
        vram[0] = 10;
        vram[1] = 20;
        vram[320] = 99;
        let inputs = packed_inputs(&vram);
        let mut renderer = VgaRenderer::new();
        let (width, height) = renderer.render(&inputs);
        assert_eq!((width, height), (640, 400));
        // Each source byte is emitted for two dots at the mode 13h rate.
        assert_eq!(pixel(&renderer, width, 0, 0), inputs.pens_256[10]);
        assert_eq!(pixel(&renderer, width, 1, 0), inputs.pens_256[10]);
        assert_eq!(pixel(&renderer, width, 2, 0), inputs.pens_256[20]);
        // Rows double scan and advance by 320 bytes.
        assert_eq!(pixel(&renderer, width, 0, 1), inputs.pens_256[10]);
        assert_eq!(pixel(&renderer, width, 0, 2), inputs.pens_256[99]);
    }

    #[test]
    fn packed_pel_pan_shifts_in_half_pixel_steps() {
        let mut vram = vec![0u8; 0x10_0000];
        vram[0] = 10;
        vram[1] = 20;
        let mut inputs = packed_inputs(&vram);
        inputs.pel_pan = 2;
        let mut renderer = VgaRenderer::new();
        let (width, _) = renderer.render(&inputs);
        // A pan of two shifts the scan-out left by one source pixel.
        assert_eq!(pixel(&renderer, width, 0, 0), inputs.pens_256[20]);
        assert_eq!(pixel(&renderer, width, 1, 0), inputs.pens_256[20]);
        assert_eq!(pixel(&renderer, width, 2, 0), inputs.pens_256[0]);
        // Odd pan values have no additional effect.
        inputs.pel_pan = 3;
        let (width, _) = renderer.render(&inputs);
        assert_eq!(pixel(&renderer, width, 0, 0), inputs.pens_256[20]);
    }

    #[test]
    fn mode_x_page_flip_start_address_offsets_scanout() {
        let mut vram = vec![0u8; 0x10_0000];
        vram[0x10000] = 5;
        let mut inputs = packed_inputs(&vram);
        // Unchained page one at plane address 0x4000 (byte mode scaling).
        inputs.start_address = 0x4000;
        let mut renderer = VgaRenderer::new();
        let (width, _) = renderer.render(&inputs);
        assert_eq!(pixel(&renderer, width, 0, 0), inputs.pens_256[5]);
    }

    #[test]
    fn svga_pitch_crosses_the_64k_boundary_without_seam() {
        let mut vram = vec![0u8; 0x10_0000];
        vram[0x10000] = 7;
        let mut inputs = packed_inputs(&vram);
        // One pixel per dot with a 640-byte row starting near the boundary.
        inputs.packed_half_rate = false;
        inputs.character_height = 1;
        inputs.active_scanlines = 480;
        inputs.row_pitch = 160;
        inputs.start_address = 0x3FF0;
        let mut renderer = VgaRenderer::new();
        let (width, _) = renderer.render(&inputs);
        // Byte 0x10000 lies 64 bytes into the first row.
        assert_eq!(pixel(&renderer, width, 64, 0), inputs.pens_256[7]);
        assert_eq!(pixel(&renderer, width, 65, 0), inputs.pens_256[0]);
    }

    #[test]
    fn packed_simd_matches_scalar() {
        let mut vram = vec![0u8; 0x10_0000];
        for (index, byte) in vram.iter_mut().enumerate() {
            *byte = (index as u8)
                .wrapping_mul(37)
                .wrapping_add((index >> 7) as u8);
        }

        // Both the mode 13h half rate path and the one pixel per dot SVGA path
        // must agree between the scalar and the SIMD rasterizer.
        for half_rate in [true, false] {
            let mut inputs = packed_inputs(&vram);
            if !half_rate {
                inputs.packed_half_rate = false;
                inputs.character_height = 1;
                inputs.active_scanlines = 480;
                inputs.row_pitch = 160;
            }

            let mut scalar = VgaRenderer::new();
            scalar.set_simd_enabled(false);
            let scalar_dims = scalar.render(&inputs);

            let mut simd = VgaRenderer::new();
            simd.set_simd_enabled(true);
            let simd_dims = simd.render(&inputs);

            assert_eq!(scalar_dims, simd_dims);
            let bytes = (scalar_dims.0 * scalar_dims.1) as usize * VGA_PIXEL_BYTES;
            assert_eq!(
                scalar.framebuffer()[..bytes],
                simd.framebuffer()[..bytes],
                "half_rate={half_rate}"
            );
        }
    }

    #[test]
    fn border_overscan_color_fills_the_margin() {
        let mut vram = vec![0u8; 0x10_0000];
        vram[0] = 0x80;
        let mut inputs = planar_inputs(&vram);
        let border = 0xFF00_2255;
        inputs.border_color = border;
        let mut renderer = VgaRenderer::new();
        let (width, height) = renderer.render(&inputs);
        assert_eq!((width, height), (656, 496));
        // The margin shows the overscan color on all four sides.
        assert_eq!(pixel(&renderer, width, 0, 0), border);
        assert_eq!(pixel(&renderer, width, 655, 495), border);
        assert_eq!(pixel(&renderer, width, 4, 250), border);
        // The active area starts after the border ring.
        assert_eq!(pixel(&renderer, width, 8, 8), inputs.pens[1]);
        assert_eq!(pixel(&renderer, width, 9, 8), inputs.pens[0]);
    }

    #[test]
    fn pel_pan_shifts_planar_pixels() {
        let mut vram = vec![0u8; 0x10_0000];
        vram[4] = 0x80;
        let mut inputs = planar_inputs(&vram);
        inputs.pel_pan = 3;
        let mut renderer = VgaRenderer::new();
        let (width, _) = renderer.render(&inputs);
        // The pixel at plane address one dot zero moves left by three dots.
        assert_eq!(pixel(&renderer, width, 5, 0), inputs.pens[1]);
        assert_eq!(pixel(&renderer, width, 8, 0), inputs.pens[0]);
    }
}
