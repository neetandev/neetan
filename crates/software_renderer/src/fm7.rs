//! Fujitsu FM-7 video composition.
//!
//! The base FM-7 draws a single 640x200 image from three 16 KiB bitplanes
//! (blue, red, green). Each plane byte carries eight horizontal pixels, most
//! significant bit leftmost; the three same-position bits form a three-bit
//! color code (bit 0 blue, bit 1 red, bit 2 green) that is remapped through
//! the eight-entry digital palette and finally expanded to a fixed RGBA color.
//!
//! The machine layer owns the VRAM, the palette and the display registers and
//! passes them in through [`RenderInputsFm7`]; the renderer holds only the
//! per-pixel color line buffer and the packed RGBA framebuffer. Lines are
//! latched one at a time as the emulated beam advances and composited into the
//! framebuffer at frame end, mirroring the other machines' renderers.

use alloc::{boxed::Box, vec};

/// FM-7 framebuffer width in pixels (80 columns x 8 pixels).
pub const FM7_SURFACE_WIDTH: usize = 640;
/// FM-7 framebuffer height in pixels (200 visible scanlines).
pub const FM7_SURFACE_HEIGHT: usize = 200;
/// Bytes per pixel (packed RGBA).
pub const FM7_PIXEL_BYTES: usize = 4;
/// Total framebuffer byte size.
pub const FM7_FRAMEBUFFER_BYTES: usize = FM7_SURFACE_WIDTH * FM7_SURFACE_HEIGHT * FM7_PIXEL_BYTES;

/// Byte offset of the blue plane within the VRAM blob.
const PLANE_BLUE: usize = 0x0000;
/// Byte offset of the red plane within the VRAM blob.
const PLANE_RED: usize = 0x4000;
/// Byte offset of the green plane within the VRAM blob.
const PLANE_GREEN: usize = 0x8000;
/// Address wrap mask applied within a single 16 KiB plane.
const PLANE_WRAP_MASK: usize = 0x3FFF;
/// Plane bytes per displayed scanline (640 pixels / 8).
const BYTES_PER_LINE: usize = 80;
/// Pixels encoded in one plane byte.
const PIXELS_PER_BYTE: usize = 8;
/// Colour codes stored per frame (one per displayed pixel).
const LAYER_PIXELS: usize = FM7_SURFACE_WIDTH * FM7_SURFACE_HEIGHT;
/// Number of digital palette / fixed-colour entries.
const PALETTE_ENTRIES: usize = 8;
/// Mask selecting the three significant bits of a colour code.
const COLOR_CODE_MASK: u8 = 0x07;
/// Colour code latched for blanked pixels and lines.
const BLACK_COLOR_CODE: u8 = 0x00;
/// Display-mask bit hiding the blue plane.
const DISPLAY_MASK_BLUE: u8 = 0x01;
/// Display-mask bit hiding the red plane.
const DISPLAY_MASK_RED: u8 = 0x02;
/// Display-mask bit hiding the green plane.
const DISPLAY_MASK_GREEN: u8 = 0x04;

/// Byte distance from VRAM page 0 to page 1 (one page of three planes).
const VRAM_PAGE_SIZE: usize = 0xC000;
/// Total VRAM size fed to the renderer on the FM-77AV (two pages, 96 KiB).
#[cfg(test)]
const VRAM_SIZE: usize = VRAM_PAGE_SIZE * 2;

/// Logical pixel width of the FM-77AV 320x200 (4096-color) mode.
const MODE320_WIDTH: usize = 320;
/// Plane bytes per scanline in 320 mode (320 pixels / 8).
const MODE320_BYTES_PER_LINE: usize = MODE320_WIDTH / PIXELS_PER_BYTE;
/// Address wrap mask within one 8 KiB sub-plane in 320 mode.
const SUBPLANE_WRAP_MASK: usize = 0x1FFF;
/// Twelve-bit indices stored per 320-mode frame (one per logical pixel).
const MODE320_LAYER_PIXELS: usize = MODE320_WIDTH * FM7_SURFACE_HEIGHT;
/// Number of entries in the FM-77AV analog palette (a 12-bit index).
const ANALOG_PALETTE_ENTRIES: usize = 4096;
/// Mask selecting the 12-bit analog palette index.
const ANALOG_INDEX_MASK: u16 = 0x0FFF;

/// Blue nibble of a packed 12-bit analog colour / palette index.
const ANALOG_BLUE_NIBBLE: u16 = 0x00F;
/// Red nibble of a packed 12-bit analog colour / palette index.
const ANALOG_RED_NIBBLE: u16 = 0x0F0;
/// Green nibble of a packed 12-bit analog colour / palette index.
const ANALOG_GREEN_NIBBLE: u16 = 0xF00;
/// Bit shift of the red nibble within a packed 12-bit analog value.
const ANALOG_RED_SHIFT: u16 = 4;
/// Bit shift of the green nibble within a packed 12-bit analog value.
const ANALOG_GREEN_SHIFT: u16 = 8;
/// Mask selecting the four significant bits of an analog channel value.
const ANALOG_CHANNEL_MASK: u16 = 0x0F;
/// Number of bit-planes composing one 4096-mode colour channel.
const SUBPLANES_PER_CHANNEL: usize = 4;

/// Absolute VRAM byte offsets of the four blue bit-planes in 4096 mode, most
/// significant channel bit first (two sub-planes in page 0, two in page 1).
const BLUE_SUBPLANES: [usize; SUBPLANES_PER_CHANNEL] = [0x00000, 0x02000, 0x0C000, 0x0E000];
/// Absolute VRAM byte offsets of the four red bit-planes in 4096 mode.
const RED_SUBPLANES: [usize; SUBPLANES_PER_CHANNEL] = [0x04000, 0x06000, 0x10000, 0x12000];
/// Absolute VRAM byte offsets of the four green bit-planes in 4096 mode.
const GREEN_SUBPLANES: [usize; SUBPLANES_PER_CHANNEL] = [0x08000, 0x0A000, 0x14000, 0x16000];

/// The eight fixed RGBA colours addressed by a three-bit colour code; blue is
/// bit 0, red is bit 1 and green is bit 2, each channel fully on or off.
const FM7_DIGITAL_RGBA: [[u8; 4]; PALETTE_ENTRIES] = build_digital_rgba();

/// Builds the fixed colour table decoding a three-bit code into RGBA.
const fn build_digital_rgba() -> [[u8; 4]; PALETTE_ENTRIES] {
    let mut palette = [[0u8; 4]; PALETTE_ENTRIES];
    let mut index = 0;
    while index < PALETTE_ENTRIES {
        let blue = if index & 1 != 0 { 0xFF } else { 0x00 };
        let red = if index & 2 != 0 { 0xFF } else { 0x00 };
        let green = if index & 4 != 0 { 0xFF } else { 0x00 };
        palette[index] = [red, green, blue, 0xFF];
        index += 1;
    }
    palette
}

/// Per-scanline video inputs borrowed from the machine.
pub struct RenderInputsFm7<'a> {
    /// VRAM blob holding the blue, red and green planes at their fixed offsets.
    /// On the FM-77AV this spans both display pages (96 KiB).
    pub planes: &'a [u8],
    /// Frame-latched digital palette: each entry is a three-bit colour code.
    pub digital_palette: [u8; PALETTE_ENTRIES],
    /// Frame-latched FM-77AV analog palette: each entry packs a 12-bit colour as
    /// `blue | red << 4 | green << 8`. Only consulted in 4096-color mode.
    pub analog_palette: &'a [u16],
    /// Display mask: a set bit excludes that plane from the output.
    pub display_mask: u8,
    /// Display start offsets for AV page 0 and page 1, already masked to the
    /// hardware granularity. 8-colour mode uses the selected display page;
    /// 4096-colour mode composes subplanes from both pages with their own
    /// offsets.
    pub display_offsets: [u16; 2],
    /// Whether the CRT output is enabled; a disabled CRT latches black.
    pub crt_enabled: bool,
    /// Whether the FM-77AV 320x200 (4096-color) mode is active.
    pub mode320: bool,
    /// Displayed VRAM page selected in 640x200 mode (ignored in 4096 mode, which
    /// composes from both pages).
    pub display_page: bool,
}

/// FM-7 / FM-77AV software renderer: owns the per-pixel line buffers and the
/// packed RGBA framebuffer.
pub struct Fm7Renderer {
    /// Resolved colour code (0..7) per pixel for 640x200 8-color scanlines.
    line_colors: Box<[u8]>,
    /// Resolved 12-bit palette index per pixel for 320x200 4096-color scanlines.
    line_indices: Box<[u16]>,
    /// Whether each latched scanline is a 4096-color line (else 8-color).
    line_is_4096: Box<[bool]>,
    /// Analog palette snapshot captured for the presented frame.
    frame_analog_palette: Box<[u16]>,
    /// Composited framebuffer (packed RGBA).
    framebuffer: Box<[u8]>,
}

save_state::runtime_state! {
/// Partially latched FM-7 frame state.
#[derive(Clone)]
pub struct Fm7RendererState {
    line_colors: Box<[u8]>,
    line_indices: Box<[u16]>,
    line_is_4096: Box<[bool]>,
    frame_analog_palette: Box<[u16]>,
}}

impl Default for Fm7Renderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Fm7Renderer {
    /// Creates a renderer with cleared line and frame buffers.
    pub fn new() -> Self {
        Self {
            line_colors: vec![BLACK_COLOR_CODE; LAYER_PIXELS].into_boxed_slice(),
            line_indices: vec![0u16; MODE320_LAYER_PIXELS].into_boxed_slice(),
            line_is_4096: vec![false; FM7_SURFACE_HEIGHT].into_boxed_slice(),
            frame_analog_palette: vec![0u16; ANALOG_PALETTE_ENTRIES].into_boxed_slice(),
            framebuffer: vec![0u8; FM7_FRAMEBUFFER_BYTES].into_boxed_slice(),
        }
    }

    /// The last composited framebuffer (packed RGBA).
    pub fn framebuffer(&self) -> &[u8] {
        &self.framebuffer
    }

    /// Captures the partially latched frame without the derived framebuffer.
    pub fn capture_state(&self) -> Fm7RendererState {
        Fm7RendererState {
            line_colors: self.line_colors.clone(),
            line_indices: self.line_indices.clone(),
            line_is_4096: self.line_is_4096.clone(),
            frame_analog_palette: self.frame_analog_palette.clone(),
        }
    }

    /// Restores the partially latched frame and rebuilds the framebuffer.
    pub fn restore_state(
        &mut self,
        state: Fm7RendererState,
    ) -> Result<(), save_state::StateValidationError> {
        if state.line_colors.len() != LAYER_PIXELS
            || state.line_indices.len() != MODE320_LAYER_PIXELS
            || state.line_is_4096.len() != FM7_SURFACE_HEIGHT
            || state.frame_analog_palette.len() != ANALOG_PALETTE_ENTRIES
        {
            return Err(save_state::StateValidationError::new(
                "FM-7 renderer state is invalid",
            ));
        }
        self.line_colors = state.line_colors;
        self.line_indices = state.line_indices;
        self.line_is_4096 = state.line_is_4096;
        self.frame_analog_palette = state.frame_analog_palette;
        self.present_latched_frame();
        Ok(())
    }

    /// Clears the scanline-latched buffers for the next frame.
    pub fn clear_latched_frame(&mut self) {
        self.line_colors.fill(BLACK_COLOR_CODE);
        self.line_indices.fill(0);
        self.line_is_4096.fill(false);
    }

    /// Latches one scanline from the current VRAM, palette and register state.
    /// Lines outside the visible area are ignored; a disabled CRT latches the
    /// line as black. The analog palette snapshot is captured once per frame.
    pub fn latch_scanline(&mut self, inputs: &RenderInputsFm7<'_>, line: usize) {
        if line >= FM7_SURFACE_HEIGHT {
            return;
        }
        if line == 0 {
            self.frame_analog_palette
                .copy_from_slice(inputs.analog_palette);
        }
        if !inputs.crt_enabled {
            self.line_is_4096[line] = false;
            let row_start = line * FM7_SURFACE_WIDTH;
            self.line_colors[row_start..row_start + FM7_SURFACE_WIDTH].fill(BLACK_COLOR_CODE);
            return;
        }
        if inputs.mode320 {
            self.latch_scanline_4096(inputs, line);
        } else {
            self.latch_scanline_8color(inputs, line);
        }
    }

    /// Latches a 640x200 8-color scanline into the colour line buffer, applying
    /// the display page, display mask, scroll offset and digital palette.
    fn latch_scanline_8color(&mut self, inputs: &RenderInputsFm7<'_>, line: usize) {
        self.line_is_4096[line] = false;
        let row_start = line * FM7_SURFACE_WIDTH;
        let row = &mut self.line_colors[row_start..row_start + FM7_SURFACE_WIDTH];

        let page_base = if inputs.display_page {
            VRAM_PAGE_SIZE
        } else {
            0
        };
        let blue_hidden = inputs.display_mask & DISPLAY_MASK_BLUE != 0;
        let red_hidden = inputs.display_mask & DISPLAY_MASK_RED != 0;
        let green_hidden = inputs.display_mask & DISPLAY_MASK_GREEN != 0;
        let display_offset = inputs.display_offsets[usize::from(inputs.display_page)];
        let line_base = line * BYTES_PER_LINE + usize::from(display_offset);

        for column in 0..BYTES_PER_LINE {
            let plane_index = (line_base + column) & PLANE_WRAP_MASK;
            let blue = plane_byte(
                inputs.planes,
                page_base + PLANE_BLUE + plane_index,
                blue_hidden,
            );
            let red = plane_byte(
                inputs.planes,
                page_base + PLANE_RED + plane_index,
                red_hidden,
            );
            let green = plane_byte(
                inputs.planes,
                page_base + PLANE_GREEN + plane_index,
                green_hidden,
            );

            let cell =
                &mut row[column * PIXELS_PER_BYTE..column * PIXELS_PER_BYTE + PIXELS_PER_BYTE];
            for (offset, pixel) in cell.iter_mut().enumerate() {
                let bit = (PIXELS_PER_BYTE - 1 - offset) as u8;
                let code =
                    ((blue >> bit) & 1) | (((red >> bit) & 1) << 1) | (((green >> bit) & 1) << 2);
                *pixel = inputs.digital_palette[usize::from(code)] & COLOR_CODE_MASK;
            }
        }
    }

    /// Latches a 320x200 4096-color scanline into the index line buffer. Each
    /// channel is assembled from four bit-planes (two per VRAM page); the display
    /// mask blanks whole channels.
    fn latch_scanline_4096(&mut self, inputs: &RenderInputsFm7<'_>, line: usize) {
        self.line_is_4096[line] = true;
        let row_start = line * MODE320_WIDTH;
        let row = &mut self.line_indices[row_start..row_start + MODE320_WIDTH];

        let mut channel_mask = 0u16;
        if inputs.display_mask & DISPLAY_MASK_BLUE == 0 {
            channel_mask |= ANALOG_BLUE_NIBBLE;
        }
        if inputs.display_mask & DISPLAY_MASK_RED == 0 {
            channel_mask |= ANALOG_RED_NIBBLE;
        }
        if inputs.display_mask & DISPLAY_MASK_GREEN == 0 {
            channel_mask |= ANALOG_GREEN_NIBBLE;
        }
        let page0_base = line * MODE320_BYTES_PER_LINE + usize::from(inputs.display_offsets[0]);
        let page1_base = line * MODE320_BYTES_PER_LINE + usize::from(inputs.display_offsets[1]);

        for column in 0..MODE320_BYTES_PER_LINE {
            let page0_index = (page0_base + column) & SUBPLANE_WRAP_MASK;
            let page1_index = (page1_base + column) & SUBPLANE_WRAP_MASK;
            let blue_bytes =
                subplane_bytes(inputs.planes, &BLUE_SUBPLANES, page0_index, page1_index);
            let red_bytes = subplane_bytes(inputs.planes, &RED_SUBPLANES, page0_index, page1_index);
            let green_bytes =
                subplane_bytes(inputs.planes, &GREEN_SUBPLANES, page0_index, page1_index);

            let cell =
                &mut row[column * PIXELS_PER_BYTE..column * PIXELS_PER_BYTE + PIXELS_PER_BYTE];
            for (offset, pixel) in cell.iter_mut().enumerate() {
                let bit = (PIXELS_PER_BYTE - 1 - offset) as u8;
                let blue = u16::from(channel_value(&blue_bytes, bit));
                let red = u16::from(channel_value(&red_bytes, bit)) << ANALOG_RED_SHIFT;
                let green = u16::from(channel_value(&green_bytes, bit)) << ANALOG_GREEN_SHIFT;
                *pixel = (blue | red | green) & channel_mask;
            }
        }
    }

    /// Composites the latched scanlines into the packed RGBA framebuffer and
    /// returns the surface dimensions. 4096-color lines are pixel-doubled
    /// horizontally to fill the 640-wide surface.
    pub fn present_latched_frame(&mut self) -> (u32, u32) {
        for line in 0..FM7_SURFACE_HEIGHT {
            let row_bytes = line * FM7_SURFACE_WIDTH * FM7_PIXEL_BYTES;
            if self.line_is_4096[line] {
                let indices =
                    &self.line_indices[line * MODE320_WIDTH..line * MODE320_WIDTH + MODE320_WIDTH];
                for (column, &index) in indices.iter().enumerate() {
                    let color = analog_rgba(&self.frame_analog_palette, index);
                    let base = row_bytes + column * 2 * FM7_PIXEL_BYTES;
                    self.framebuffer[base..base + FM7_PIXEL_BYTES].copy_from_slice(&color);
                    self.framebuffer[base + FM7_PIXEL_BYTES..base + 2 * FM7_PIXEL_BYTES]
                        .copy_from_slice(&color);
                }
            } else {
                let codes = &self.line_colors
                    [line * FM7_SURFACE_WIDTH..line * FM7_SURFACE_WIDTH + FM7_SURFACE_WIDTH];
                for (column, &code) in codes.iter().enumerate() {
                    let color = FM7_DIGITAL_RGBA[usize::from(code & COLOR_CODE_MASK)];
                    let base = row_bytes + column * FM7_PIXEL_BYTES;
                    self.framebuffer[base..base + FM7_PIXEL_BYTES].copy_from_slice(&color);
                }
            }
        }
        (FM7_SURFACE_WIDTH as u32, FM7_SURFACE_HEIGHT as u32)
    }
}

/// Reads a plane byte, returning zero when the plane is hidden or the index is
/// out of range.
fn plane_byte(planes: &[u8], index: usize, hidden: bool) -> u8 {
    if hidden {
        0
    } else {
        planes.get(index).copied().unwrap_or(0)
    }
}

/// Reads the four bit-plane bytes of one 4096-mode colour channel at the given
/// in-plane index, most significant channel bit first.
fn subplane_bytes(
    planes: &[u8],
    subplanes: &[usize; SUBPLANES_PER_CHANNEL],
    page0_index: usize,
    page1_index: usize,
) -> [u8; SUBPLANES_PER_CHANNEL] {
    let mut bytes = [0u8; SUBPLANES_PER_CHANNEL];
    for (plane, (byte, base)) in bytes.iter_mut().zip(subplanes).enumerate() {
        let index = if plane < 2 { page0_index } else { page1_index };
        *byte = planes.get(base + index).copied().unwrap_or(0);
    }
    bytes
}

/// Assembles the four-bit channel value for one pixel from its four bit-planes.
/// The first sub-plane is the most significant channel bit.
fn channel_value(bytes: &[u8; SUBPLANES_PER_CHANNEL], bit: u8) -> u8 {
    let mut value = 0u8;
    for (plane, byte) in bytes.iter().enumerate() {
        let plane_bit = (byte >> bit) & 1;
        value |= plane_bit << (SUBPLANES_PER_CHANNEL - 1 - plane);
    }
    value
}

/// Expands a four-bit analog channel value to eight bits, filling the low nibble
/// when the value is non-zero (matching the FM-77AV DAC).
const fn expand_nibble(value: u8) -> u8 {
    if value == 0 { 0 } else { (value << 4) | 0x0F }
}

/// Resolves a 12-bit palette index to a packed RGBA colour through the analog
/// palette snapshot.
fn analog_rgba(palette: &[u16], index: u16) -> [u8; FM7_PIXEL_BYTES] {
    let entry = palette
        .get(usize::from(index & ANALOG_INDEX_MASK))
        .copied()
        .unwrap_or(0);
    let blue = expand_nibble((entry & ANALOG_CHANNEL_MASK) as u8);
    let red = expand_nibble(((entry >> ANALOG_RED_SHIFT) & ANALOG_CHANNEL_MASK) as u8);
    let green = expand_nibble(((entry >> ANALOG_GREEN_SHIFT) & ANALOG_CHANNEL_MASK) as u8);
    [red, green, blue, 0xFF]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds an identity analog palette (`entry == index`) for the tests.
    fn identity_analog_palette() -> [u16; ANALOG_PALETTE_ENTRIES] {
        let mut palette = [0u16; ANALOG_PALETTE_ENTRIES];
        for (index, entry) in palette.iter_mut().enumerate() {
            *entry = index as u16;
        }
        palette
    }

    /// Assembles the base 8-color render inputs over `planes`.
    fn inputs_8color<'a>(
        planes: &'a [u8],
        palette: &'a [u16],
        display_mask: u8,
        crt_enabled: bool,
    ) -> RenderInputsFm7<'a> {
        RenderInputsFm7 {
            planes,
            digital_palette: [0, 1, 2, 3, 4, 5, 6, 7],
            analog_palette: palette,
            display_mask,
            display_offsets: [0, 0],
            crt_enabled,
            mode320: false,
            display_page: false,
        }
    }

    #[test]
    fn digital_colors_decode_the_low_three_bits() {
        assert_eq!(FM7_DIGITAL_RGBA[0], [0x00, 0x00, 0x00, 0xFF]);
        assert_eq!(FM7_DIGITAL_RGBA[1], [0x00, 0x00, 0xFF, 0xFF]); // blue
        assert_eq!(FM7_DIGITAL_RGBA[2], [0xFF, 0x00, 0x00, 0xFF]); // red
        assert_eq!(FM7_DIGITAL_RGBA[4], [0x00, 0xFF, 0x00, 0xFF]); // green
        assert_eq!(FM7_DIGITAL_RGBA[7], [0xFF, 0xFF, 0xFF, 0xFF]); // white
    }

    #[test]
    fn latches_leftmost_pixel_from_the_most_significant_bit() {
        let palette = identity_analog_palette();
        let mut planes = vec![0u8; VRAM_SIZE];
        // White (all three planes) in the leftmost pixel of line 0.
        planes[PLANE_BLUE] = 0x80;
        planes[PLANE_RED] = 0x80;
        planes[PLANE_GREEN] = 0x80;
        let inputs = inputs_8color(&planes, &palette, 0, true);
        let mut renderer = Fm7Renderer::new();
        renderer.latch_scanline(&inputs, 0);
        renderer.present_latched_frame();
        assert_eq!(&renderer.framebuffer()[0..4], &[0xFF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(&renderer.framebuffer()[4..8], &[0x00, 0x00, 0x00, 0xFF]);
    }

    #[test]
    fn display_mask_hides_a_plane() {
        let palette = identity_analog_palette();
        let mut planes = vec![0u8; VRAM_SIZE];
        planes[PLANE_RED] = 0xFF;
        let inputs = inputs_8color(&planes, &palette, DISPLAY_MASK_RED, true);
        let mut renderer = Fm7Renderer::new();
        renderer.latch_scanline(&inputs, 0);
        renderer.present_latched_frame();
        assert_eq!(&renderer.framebuffer()[0..4], &[0x00, 0x00, 0x00, 0xFF]);
    }

    #[test]
    fn disabled_crt_latches_black() {
        let palette = identity_analog_palette();
        let mut planes = vec![0u8; VRAM_SIZE];
        planes[PLANE_GREEN] = 0xFF;
        let inputs = inputs_8color(&planes, &palette, 0, false);
        let mut renderer = Fm7Renderer::new();
        renderer.latch_scanline(&inputs, 0);
        renderer.present_latched_frame();
        assert_eq!(&renderer.framebuffer()[0..4], &[0x00, 0x00, 0x00, 0xFF]);
    }

    #[test]
    fn display_page_selects_the_second_vram_page() {
        let palette = identity_analog_palette();
        let mut planes = vec![0u8; VRAM_SIZE];
        // Blue leftmost pixel only in page 1.
        planes[VRAM_PAGE_SIZE + PLANE_BLUE] = 0x80;
        let mut inputs = inputs_8color(&planes, &palette, 0, true);
        inputs.display_page = true;
        let mut renderer = Fm7Renderer::new();
        renderer.latch_scanline(&inputs, 0);
        renderer.present_latched_frame();
        assert_eq!(&renderer.framebuffer()[0..4], &[0x00, 0x00, 0xFF, 0xFF]);
    }

    #[test]
    fn latches_a_4096_color_pixel_and_doubles_it() {
        let palette = identity_analog_palette();
        let mut planes = vec![0u8; VRAM_SIZE];
        // Leftmost pixel: blue = 8 (MSB sub-plane), red = 15, green = 0.
        planes[BLUE_SUBPLANES[0]] = 0x80; // blue bit 3
        for base in RED_SUBPLANES {
            planes[base] = 0x80; // red = 0b1111
        }
        let inputs = RenderInputsFm7 {
            planes: &planes,
            digital_palette: [0, 1, 2, 3, 4, 5, 6, 7],
            analog_palette: &palette,
            display_mask: 0,
            display_offsets: [0, 0],
            crt_enabled: true,
            mode320: true,
            display_page: false,
        };
        let mut renderer = Fm7Renderer::new();
        renderer.latch_scanline(&inputs, 0);
        renderer.present_latched_frame();
        // index = blue(8) | red(15)<<4 | green(0)<<8 = 0x0F8; identity palette maps
        // to blue nibble 8 -> 0x8F, red nibble 15 -> 0xFF, green 0 -> 0x00.
        let expected = [0xFF, 0x00, 0x8F, 0xFF];
        assert_eq!(&renderer.framebuffer()[0..4], &expected); // pixel doubled
        assert_eq!(&renderer.framebuffer()[4..8], &expected);
    }
}
