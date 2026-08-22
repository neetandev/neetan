//! Software renderer for the PC-6000 video subsystems.
//!
//! The base PC-6001 (and the PC-6001mkII legacy path) is driven by a single
//! global attribute byte at the start of video RAM that selects between text,
//! semigraphics and the two bitmap graphics modes. The mkII adds extended
//! bitmap, 2bpp and 40x20 text modes selected by its video registers. The base
//! machine renders a 256x192 image; the mkII renders a 320x240 image with the
//! legacy modes centered inside a border.

mod base;
mod mkii;
mod sr;

/// Base PC-6001 display width in pixels.
pub const PC60_WIDTH: usize = 256;
/// Base PC-6001 display height in pixels.
pub const PC60_HEIGHT: usize = 192;
/// PC-6001mkII display width in pixels.
pub const PC60_MK2_WIDTH: usize = 320;
/// PC-6001mkII display height in pixels.
pub const PC60_MK2_HEIGHT: usize = 240;
/// Bytes per pixel in the output framebuffer (RGBA).
pub const PC60_PIXEL_BYTES: usize = 4;
/// Base framebuffer size in bytes.
pub const PC60_FRAMEBUFFER_BYTES: usize = PC60_WIDTH * PC60_HEIGHT * PC60_PIXEL_BYTES;
/// mkII framebuffer size in bytes.
pub const PC60_MK2_FRAMEBUFFER_BYTES: usize = PC60_MK2_WIDTH * PC60_MK2_HEIGHT * PC60_PIXEL_BYTES;

/// One RGBA palette entry.
type Rgba = [u8; 4];

/// Base palette. Entry 0 is the background (black); entries 1-7 and 8-15 hold
/// the eight RF colors (graphics and semigraphics), and entries 16-19 hold the
/// alphanumeric text colors.
pub(crate) static BASE_PALETTE: [Rgba; 20] = [
    [0x00, 0x00, 0x00, 0xFF], // 0: black
    [0xFF, 0xFF, 0x00, 0xFF], // 1: yellow
    [0x3B, 0x08, 0xFF, 0xFF], // 2: blue
    [0xCC, 0x00, 0x3B, 0xFF], // 3: red
    [0xFF, 0xFF, 0xFF, 0xFF], // 4: white
    [0x07, 0xE3, 0x99, 0xFF], // 5: cyan
    [0xFF, 0x1C, 0xFF, 0xFF], // 6: magenta
    [0xFF, 0x81, 0x00, 0xFF], // 7: orange
    [0x07, 0xFF, 0x00, 0xFF], // 8: green
    [0xFF, 0xFF, 0x00, 0xFF], // 9: yellow
    [0x3B, 0x08, 0xFF, 0xFF], // 10: blue
    [0xCC, 0x00, 0x3B, 0xFF], // 11: red
    [0xFF, 0xFF, 0xFF, 0xFF], // 12: white
    [0x07, 0xE3, 0x99, 0xFF], // 13: cyan
    [0xFF, 0x1C, 0xFF, 0xFF], // 14: magenta
    [0xFF, 0x81, 0x00, 0xFF], // 15: orange
    [0x00, 0x7C, 0x00, 0xFF], // 16: alphanumeric dark green
    [0x07, 0xFF, 0x00, 0xFF], // 17: alphanumeric bright green
    [0x91, 0x00, 0x00, 0xFF], // 18: alphanumeric dark orange
    [0xFF, 0x81, 0x00, 0xFF], // 19: alphanumeric bright orange
];

/// mkII palette. Pens 8-15 are the eight RF colors (shared with the legacy
/// modes); pens 16-31 are the sixteen mkII colors used by the extended modes.
pub(crate) static MK2_PALETTE: [Rgba; 32] = [
    [0x00, 0x00, 0x00, 0xFF], // 0
    [0x00, 0x00, 0x00, 0xFF], // 1
    [0x00, 0x00, 0x00, 0xFF], // 2
    [0x00, 0x00, 0x00, 0xFF], // 3
    [0x00, 0x00, 0x00, 0xFF], // 4
    [0x00, 0x00, 0x00, 0xFF], // 5
    [0x00, 0x00, 0x00, 0xFF], // 6
    [0x00, 0x00, 0x00, 0xFF], // 7
    [0x07, 0xFF, 0x00, 0xFF], // 8: green
    [0xFF, 0xFF, 0x00, 0xFF], // 9: yellow
    [0x3B, 0x08, 0xFF, 0xFF], // 10: blue
    [0xCC, 0x00, 0x3B, 0xFF], // 11: red
    [0xFF, 0xFF, 0xFF, 0xFF], // 12: white
    [0x07, 0xE3, 0x99, 0xFF], // 13: cyan
    [0xFF, 0x1C, 0xFF, 0xFF], // 14: magenta
    [0xFF, 0x81, 0x00, 0xFF], // 15: orange
    [0x00, 0x00, 0x00, 0xFF], // 16: black
    [0xFF, 0xAF, 0x00, 0xFF], // 17: orange
    [0x00, 0xFF, 0xAF, 0xFF], // 18: green tone
    [0xAF, 0xFF, 0x00, 0xFF], // 19: green tone
    [0xAF, 0x00, 0xFF, 0xFF], // 20: violet
    [0xFF, 0x00, 0xAF, 0xFF], // 21: scarlet
    [0x00, 0xAF, 0xFF, 0xFF], // 22: light blue
    [0xAF, 0xAF, 0xAF, 0xFF], // 23: gray
    [0x00, 0x00, 0x00, 0xFF], // 24: black
    [0xFF, 0x00, 0x00, 0xFF], // 25: red
    [0x00, 0xFF, 0x00, 0xFF], // 26: green
    [0xFF, 0xFF, 0x00, 0xFF], // 27: yellow
    [0x00, 0x00, 0xFF, 0xFF], // 28: blue
    [0xFF, 0x00, 0xFF, 0xFF], // 29: pink
    [0x00, 0xFF, 0xFF, 0xFF], // 30: cyan
    [0xFF, 0xFF, 0xFF, 0xFF], // 31: white
];

/// Which machine generation the frame is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pc60RenderModel {
    /// Base PC-6001 (256x192, MC6847 modes only).
    Base,
    /// PC-6001mkII (320x240, extended modes plus the legacy fallback).
    Mk2,
}

/// Inputs for a single PC-6000 frame render.
pub struct RenderInputs60<'a> {
    /// Which machine generation to render for.
    pub model: Pc60RenderModel,
    /// Video RAM window: the global attribute byte, the tile/attribute map and
    /// the bitmap data, starting at the active VRAM base.
    pub vram: &'a [u8],
    /// Character generator ROM (8x12 glyphs packed as 16 bytes each; on the
    /// mkII this is the base CG followed by the extended CG).
    pub cgrom: &'a [u8],
    /// mkII extended bitmap mode (160x200x4) active.
    pub exgfx_bitmap: bool,
    /// mkII extended 2bpp mode (320x200) active.
    pub exgfx_2bpp: bool,
    /// mkII extended text mode (40x20, 16 colors) active.
    pub exgfx_text: bool,
    /// mkII background color bank (port 0xC0).
    pub bgcol_bank: u8,
}

/// Inputs for a single native SR frame render (320x240).
pub struct RenderInputsSr<'a> {
    /// Text VRAM window (used in text mode): tile/attribute byte pairs.
    pub vram: &'a [u8],
    /// SR character generator (16 KiB).
    pub cgrom: &'a [u8],
    /// Graphics VRAM (used in bitmap mode): one byte per pixel, low nibble.
    pub gvram: &'a [u8],
    /// Text mode active (otherwise the 4bpp bitmap mode).
    pub text_mode: bool,
    /// Text rows (20 or 25).
    pub text_rows: u8,
    /// 80-column text (otherwise 40).
    pub width80: bool,
    /// Bitmap horizontal scroll (wraps at 320).
    pub scroll_x: u16,
    /// Bitmap vertical scroll (wraps at 204).
    pub scroll_y: u8,
}

impl<'a> RenderInputs60<'a> {
    /// Inputs for a base PC-6001 frame.
    pub fn base(vram: &'a [u8], cgrom: &'a [u8]) -> Self {
        Self {
            model: Pc60RenderModel::Base,
            vram,
            cgrom,
            exgfx_bitmap: false,
            exgfx_2bpp: false,
            exgfx_text: false,
            bgcol_bank: 0,
        }
    }
}

/// Renders one frame into `framebuffer`. The buffer must match the model's
/// dimensions: [`PC60_FRAMEBUFFER_BYTES`] for the base machine,
/// [`PC60_MK2_FRAMEBUFFER_BYTES`] for the mkII.
pub fn render(inputs: &RenderInputs60, framebuffer: &mut [u8]) {
    match inputs.model {
        Pc60RenderModel::Base => {
            let mut target = base::Target::new(framebuffer, PC60_WIDTH, PC60_HEIGHT, 0, 0);
            base::render(inputs, &mut target);
        }
        Pc60RenderModel::Mk2 => mkii::render(inputs, framebuffer),
    }
}

/// Renders one native SR frame into `framebuffer` ([`PC60_MK2_FRAMEBUFFER_BYTES`]).
pub fn render_sr(inputs: &RenderInputsSr, framebuffer: &mut [u8]) {
    sr::render(inputs, framebuffer);
}

/// Writes one RGBA pixel, clipping to the framebuffer bounds.
pub(crate) fn put_pixel(
    framebuffer: &mut [u8],
    width: usize,
    height: usize,
    x: usize,
    y: usize,
    color: Rgba,
) {
    if x >= width || y >= height {
        return;
    }
    let offset = (y * width + x) * PC60_PIXEL_BYTES;
    framebuffer[offset..offset + 4].copy_from_slice(&color);
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    #[test]
    fn base_text_mode_renders_glyph_pixels() {
        let mut vram = vec![0u8; 0x800];
        vram[0] = 0x00;
        vram[base::TILE_MAP_OFFSET] = 0x01;

        let mut cgrom = vec![0u8; 0x1000];
        cgrom[0x10] = 0xFF;

        let inputs = RenderInputs60::base(&vram, &cgrom);
        let mut framebuffer = vec![0u8; PC60_FRAMEBUFFER_BYTES];
        render(&inputs, &mut framebuffer);

        assert_eq!(&framebuffer[0..4], &BASE_PALETTE[0x11]);
    }

    #[test]
    fn base_graphics_mode_selects_bitmap_path() {
        let mut vram = vec![0u8; 0x2000];
        vram[0] = 0x80 | 0x10 | 0x0C;
        vram[base::TILE_MAP_OFFSET] = 0xFF;

        let cgrom = vec![0u8; 0x1000];
        let inputs = RenderInputs60::base(&vram, &cgrom);
        let mut framebuffer = vec![0u8; PC60_FRAMEBUFFER_BYTES];
        render(&inputs, &mut framebuffer);

        assert_ne!(&framebuffer[0..4], &BASE_PALETTE[0]);
    }

    #[test]
    fn mk2_extended_text_renders_a_non_blank_frame() {
        // 40x20 cells: tiles at +0x400, attributes at +0x000.
        let mut vram = vec![0u8; 0x4000];
        // Cell 0: tile 1, fg color 0x0f (white-ish), bg color 0.
        vram[0x000] = 0x0F;
        vram[0x400] = 0x01;

        // mkII gfx CG: tile (1 + 0x200) -> glyph at (0x201 * 0x10).
        let mut cgrom = vec![0u8; 0x4000];
        cgrom[0x201 * 0x10] = 0xFF;

        let inputs = RenderInputs60 {
            model: Pc60RenderModel::Mk2,
            vram: &vram,
            cgrom: &cgrom,
            exgfx_bitmap: false,
            exgfx_2bpp: false,
            exgfx_text: true,
            bgcol_bank: 0,
        };
        let mut framebuffer = vec![0u8; PC60_MK2_FRAMEBUFFER_BYTES];
        render(&inputs, &mut framebuffer);

        let lit = framebuffer
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[0] != 0 || pixel[1] != 0 || pixel[2] != 0);
        assert!(lit, "extended text should render foreground pixels");
    }

    #[test]
    fn mk2_bitmap_mode_has_priority() {
        let mut vram = vec![0u8; 0x4000];
        vram[0x0000] = 0xFF;
        vram[0x2000] = 0xFF;

        let cgrom = vec![0u8; 0x4000];
        let inputs = RenderInputs60 {
            model: Pc60RenderModel::Mk2,
            vram: &vram,
            cgrom: &cgrom,
            exgfx_bitmap: true,
            exgfx_2bpp: false,
            exgfx_text: false,
            bgcol_bank: 0,
        };
        let mut framebuffer = vec![0u8; PC60_MK2_FRAMEBUFFER_BYTES];
        render(&inputs, &mut framebuffer);

        let lit = framebuffer
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[0] != 0 || pixel[1] != 0 || pixel[2] != 0);
        assert!(lit, "bitmap mode should paint pixels");
    }
}
