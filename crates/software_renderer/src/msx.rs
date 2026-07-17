//! Scanline renderer for the MSX video processor family.

use alloc::{boxed::Box, vec, vec::Vec};

use device::video_msx::{
    MsxSpriteLineStatus, MsxVdpDisplayMode, MsxVdpRenderState, MsxVdpVersion, signed_adjust,
};

/// Width of the physically visible MSX1 NTSC output.
pub const MSX_SURFACE_WIDTH: usize = 284;
/// Width of the physically visible MSX2 NTSC output.
pub const MSX2_SURFACE_WIDTH: usize = 568;
/// Height of the physically visible NTSC output in scanlines.
pub const MSX_SURFACE_HEIGHT: usize = 240;
/// Bytes per packed RGBA pixel.
pub const MSX_PIXEL_BYTES: usize = 4;
/// Total byte size of the MSX1 framebuffer.
pub const MSX_FRAMEBUFFER_BYTES: usize = MSX_SURFACE_WIDTH * MSX_SURFACE_HEIGHT * MSX_PIXEL_BYTES;
/// Total byte size of the MSX2 framebuffer.
pub const MSX2_FRAMEBUFFER_BYTES: usize = MSX2_SURFACE_WIDTH * MSX_SURFACE_HEIGHT * MSX_PIXEL_BYTES;
/// First physical NTSC line included in the visible surface.
pub const MSX_VISIBLE_START_LINE: u16 = 18;
/// Last physical NTSC line included in the visible surface.
pub const MSX_VISIBLE_END_LINE: u16 = 257;
/// First physical line of the neutral 192-line active display.
pub const MSX_ACTIVE_START_LINE: u16 = 35;
/// Last physical line of the neutral 192-line active display.
pub const MSX_ACTIVE_END_LINE: u16 = 226;
/// First horizontal pixel of the MSX1 active display.
pub const MSX_ACTIVE_START_X: usize = 14;
/// Width of the TMS active display in pixels.
pub const MSX_ACTIVE_WIDTH: usize = 256;

/// TMS-compatible VRAM address mask.
const TMS_VRAM_ADDRESS_MASK: usize = 0x3FFF;
/// V9938 VRAM address mask.
const V9938_VRAM_ADDRESS_MASK: usize = 0x1FFFF;
/// Text-mode character columns.
const TEXT_COLUMNS: usize = 40;
/// Text 2 character columns.
const TEXT_TWO_COLUMNS: usize = 80;
/// Graphics-mode character columns.
const GRAPHICS_COLUMNS: usize = 32;
/// Pattern height in scanlines.
const PATTERN_HEIGHT: usize = 8;
/// Text glyph width in pixels.
const TEXT_GLYPH_WIDTH: usize = 6;
/// Graphics glyph width in pixels.
const GRAPHICS_GLYPH_WIDTH: usize = 8;
/// Left margin inside the active area in Text 1 mode.
const TEXT_ACTIVE_MARGIN_LEFT: usize = 6;
/// Left margin inside the active area in Text 2 mode.
const TEXT_TWO_ACTIVE_MARGIN_LEFT: usize = 16;
/// Multicolor source pixels represented by one output block.
const MULTICOLOR_BLOCK_WIDTH: usize = 4;
/// Maximum sprites evaluated by the VDP.
const SPRITE_COUNT: usize = 32;
/// Bytes in one sprite attribute entry.
const SPRITE_ATTRIBUTE_BYTES: usize = 4;
/// Sprite-list Y value terminating mode-one evaluation.
const SPRITE_MODE_ONE_TERMINATOR_Y: u8 = 0xD0;
/// Sprite-list Y value terminating mode-two evaluation.
const SPRITE_MODE_TWO_TERMINATOR_Y: u8 = 0xD8;
/// Sprite attribute bit enabling the early clock.
const SPRITE_EARLY_CLOCK: u8 = 0x80;
/// Sprite mode-two color-combine bit.
const SPRITE_COLOR_COMBINE: u8 = 0x40;
/// Sprite mode-two collision-inhibit bit.
const SPRITE_COLLISION_INHIBIT: u8 = 0x20;
/// Mask selecting the sprite color.
const SPRITE_COLOR_MASK: u8 = 0x0F;
/// Number of mode-one sprites displayed on one scanline.
const MODE_ONE_SPRITES_PER_LINE: usize = 4;
/// Number of mode-two sprites displayed on one scanline.
const MODE_TWO_SPRITES_PER_LINE: usize = 8;
/// Width of the expanded sprite pattern used by the V9938 renderer.
const SPRITE_PATTERN_BITS: usize = 32;
/// Horizontal displacement caused by the sprite early clock.
const SPRITE_EARLY_CLOCK_PIXELS: i16 = 32;
/// Number of fixed TMS colors.
const PALETTE_ENTRIES: usize = 16;
/// Marker distinguishing fixed SCREEN 8 colors from palette indexes.
const FIXED_COLOR_MARKER: u16 = 0x100;
/// Marker distinguishing the fixed SCREEN 8 sprite palette.
const FIXED_SPRITE_COLOR_MARKER: u16 = 0x200;
/// Marker distinguishing V9958 YJK colors.
const YJK_COLOR_MARKER: u16 = 0x8000;
/// Empty pixel in the temporary sprite plane.
const TRANSPARENT_SPRITE_PIXEL: u16 = 0xFFFF;
/// Fixed SCREEN 8 sprite palette in GRB bit order.
const SCREEN_EIGHT_SPRITE_PALETTE: [u16; PALETTE_ENTRIES] = [
    0x000, 0x002, 0x030, 0x032, 0x300, 0x302, 0x330, 0x332, 0x472, 0x007, 0x070, 0x077, 0x700,
    0x707, 0x770, 0x777,
];
/// Fixed TMS9118 palette in packed RGBA order.
const TMS9118_RGBA: [[u8; 4]; PALETTE_ENTRIES] = [
    [0, 0, 0, 0xFF],
    [0, 0, 0, 0xFF],
    [33, 200, 66, 0xFF],
    [94, 220, 120, 0xFF],
    [84, 85, 237, 0xFF],
    [125, 118, 252, 0xFF],
    [212, 82, 77, 0xFF],
    [66, 235, 245, 0xFF],
    [252, 85, 84, 0xFF],
    [255, 121, 120, 0xFF],
    [212, 193, 84, 0xFF],
    [230, 206, 128, 0xFF],
    [33, 176, 59, 0xFF],
    [201, 91, 186, 0xFF],
    [204, 204, 204, 0xFF],
    [255, 255, 255, 0xFF],
];

/// Per-scanline VDP inputs borrowed from the machine.
pub struct RenderInputsMsx<'a> {
    /// Complete physical VDP RAM.
    pub vram: &'a [u8],
    /// Masked control-register and palette snapshot.
    pub state: MsxVdpRenderState,
}

/// MSX software renderer with scanline-latched packed pixels.
pub struct MsxRenderer {
    version: MsxVdpVersion,
    surface_width: usize,
    line_pixels: Box<[u16]>,
    line_rgba: Box<[u8]>,
    framebuffer: Box<[u8]>,
    sprite_pixels: Box<[u16; 512]>,
    sprite_physical_line: Option<u16>,
}

save_state::runtime_state! {
/// Complete MSX scanline renderer state.
#[derive(Clone)]
pub struct MsxRendererState {
    version: u8,
    surface_width: usize,
    line_pixels: Vec<u16>,
    line_rgba: Vec<u8>,
    framebuffer: Vec<u8>,
    sprite_pixels: Vec<u16>,
    sprite_physical_line: Option<u16>,
}}

/// Sprite data visible on one V9938 scanline.
#[derive(Clone, Copy, Default)]
struct ModeTwoSpriteLine {
    pattern: u32,
    x: i16,
    color_attribute: u8,
}

/// VRAM source for sprite pattern rows.
#[derive(Clone, Copy)]
struct SpritePatternSource<'a> {
    vram: &'a [u8],
    mask: usize,
    pattern_base: usize,
    planar: bool,
}

impl MsxRenderer {
    /// Creates an MSX1 renderer.
    pub fn new() -> Self {
        Self::new_for_version(MsxVdpVersion::Tms9118)
    }

    /// Creates a renderer for one VDP generation.
    pub fn new_for_version(version: MsxVdpVersion) -> Self {
        let surface_width = if version.is_v99x8() {
            MSX2_SURFACE_WIDTH
        } else {
            MSX_SURFACE_WIDTH
        };
        let pixels = surface_width * MSX_SURFACE_HEIGHT;
        Self {
            version,
            surface_width,
            line_pixels: vec![0; pixels].into_boxed_slice(),
            line_rgba: vec![0; pixels * MSX_PIXEL_BYTES].into_boxed_slice(),
            framebuffer: vec![0; pixels * MSX_PIXEL_BYTES].into_boxed_slice(),
            sprite_pixels: Box::new([TRANSPARENT_SPRITE_PIXEL; 512]),
            sprite_physical_line: None,
        }
    }

    /// Captures published video and partial scanline composition state.
    pub fn capture_state(&self) -> MsxRendererState {
        MsxRendererState {
            version: match self.version {
                MsxVdpVersion::Tms9118 => 0,
                MsxVdpVersion::V9938 => 1,
                MsxVdpVersion::V9958 => 2,
            },
            surface_width: self.surface_width,
            line_pixels: self.line_pixels.to_vec(),
            line_rgba: self.line_rgba.to_vec(),
            framebuffer: self.framebuffer.to_vec(),
            sprite_pixels: self.sprite_pixels.to_vec(),
            sprite_physical_line: self.sprite_physical_line,
        }
    }

    /// Restores published video and partial scanline composition state.
    pub fn restore_state(
        &mut self,
        state: MsxRendererState,
    ) -> Result<(), save_state::StateValidationError> {
        let version = match state.version {
            0 => MsxVdpVersion::Tms9118,
            1 => MsxVdpVersion::V9938,
            2 => MsxVdpVersion::V9958,
            _ => {
                return Err(save_state::StateValidationError::new(
                    "MSX renderer version is invalid",
                ));
            }
        };
        let expected_width = if version.is_v99x8() {
            MSX2_SURFACE_WIDTH
        } else {
            MSX_SURFACE_WIDTH
        };
        let pixel_count = expected_width * MSX_SURFACE_HEIGHT;
        if state.surface_width != expected_width
            || state.line_pixels.len() != pixel_count
            || state.line_rgba.len() != pixel_count * MSX_PIXEL_BYTES
            || state.framebuffer.len() != pixel_count * MSX_PIXEL_BYTES
            || state.sprite_pixels.len() != 512
            || state
                .sprite_physical_line
                .is_some_and(|line| line >= MSX_SURFACE_HEIGHT as u16)
        {
            return Err(save_state::StateValidationError::new(
                "MSX renderer state is invalid",
            ));
        }
        let sprite_pixels: Box<[u16; 512]> = state
            .sprite_pixels
            .into_boxed_slice()
            .try_into()
            .map_err(|_| save_state::StateValidationError::new("MSX sprite state is invalid"))?;
        self.version = version;
        self.surface_width = state.surface_width;
        self.line_pixels = state.line_pixels.into_boxed_slice();
        self.line_rgba = state.line_rgba.into_boxed_slice();
        self.framebuffer = state.framebuffer.into_boxed_slice();
        self.sprite_pixels = sprite_pixels;
        self.sprite_physical_line = state.sprite_physical_line;
        Ok(())
    }

    /// Clears all scanline-latched pixels for the next frame.
    pub fn clear_latched_frame(&mut self) {
        self.line_pixels.fill(0);
        self.line_rgba.fill(0);
        self.sprite_pixels.fill(TRANSPARENT_SPRITE_PIXEL);
        self.sprite_physical_line = None;
    }

    /// Returns the configured physical surface dimensions.
    pub const fn dimensions(&self) -> (u32, u32) {
        (self.surface_width as u32, MSX_SURFACE_HEIGHT as u32)
    }

    /// Latches one physical NTSC scanline and returns its sprite status.
    pub fn latch_scanline(
        &mut self,
        inputs: &RenderInputsMsx<'_>,
        physical_line: u16,
    ) -> MsxSpriteLineStatus {
        if !(MSX_VISIBLE_START_LINE..=MSX_VISIBLE_END_LINE).contains(&physical_line) {
            return MsxSpriteLineStatus::default();
        }

        let output_line = usize::from(physical_line - MSX_VISIBLE_START_LINE);
        let row_start = output_line * self.surface_width;
        let row = &mut self.line_pixels[row_start..row_start + self.surface_width];
        let backdrop = backdrop_color(inputs.state);
        row.fill(backdrop);

        let active_lines = inputs.state.active_lines();
        let neutral_start = if active_lines == 212 {
            25i16
        } else {
            i16::try_from(MSX_ACTIVE_START_LINE).unwrap()
        };
        let active_start = (neutral_start
            + i16::from(signed_adjust(inputs.state.register(18) >> 4)))
        .clamp(0, 261);
        let active_end = active_start + i16::try_from(active_lines).unwrap();
        let physical_line_signed = i16::try_from(physical_line).unwrap();
        if physical_line_signed < active_start
            || physical_line_signed >= active_end
            || inputs.state.register(1) & 0x40 == 0
        {
            let mut status = MsxSpriteLineStatus::default();
            if physical_line_signed + 1 == active_start && inputs.state.register(1) & 0x40 != 0 {
                self.sprite_pixels.fill(TRANSPARENT_SPRITE_PIXEL);
                status = draw_sprite_line(inputs, 0, self.sprite_pixels.as_mut());
                self.sprite_physical_line = Some(physical_line + 1);
            }
            convert_row(inputs.state, row, &mut self.line_rgba, row_start);
            return status;
        }

        let active_line = usize::try_from(physical_line_signed - active_start).unwrap();
        let horizontal_adjust = i16::from(signed_adjust(inputs.state.register(18) & 0x0F)) * 2;
        let active_start_x = if self.version.is_v99x8() {
            28i16 + horizontal_adjust
        } else {
            i16::try_from(MSX_ACTIVE_START_X).unwrap()
        };
        let mode = inputs.state.display_mode();
        let source_width = if mode.is_high_resolution() && !inputs.state.yjk_enabled() {
            512
        } else {
            256
        };
        let mut source = [backdrop; 512];
        draw_scrolled_background(
            inputs,
            mode,
            active_line,
            backdrop,
            source_width,
            &mut source,
        );

        let mut sprites = [TRANSPARENT_SPRITE_PIXEL; 512];
        let cached = self.sprite_physical_line == Some(physical_line);
        let current_status = if cached {
            sprites.copy_from_slice(self.sprite_pixels.as_ref());
            MsxSpriteLineStatus::default()
        } else {
            draw_sprite_line(inputs, active_line, &mut sprites)
        };
        self.sprite_pixels.fill(TRANSPARENT_SPRITE_PIXEL);
        let next_active_line = active_line + 1;
        let next_status = if next_active_line < usize::from(active_lines) {
            let status = draw_sprite_line(inputs, next_active_line, self.sprite_pixels.as_mut());
            self.sprite_physical_line = Some(physical_line + 1);
            status
        } else {
            self.sprite_physical_line = None;
            MsxSpriteLineStatus::default()
        };
        let status = if cached { next_status } else { current_status };

        let fine_scroll = if horizontal_scroll_applies(inputs.state, mode) {
            usize::from(inputs.state.horizontal_adjust()) * source_width / MSX_ACTIVE_WIDTH
        } else {
            0
        };
        if inputs.state.horizontal_mask_enabled() && horizontal_scroll_applies(inputs.state, mode) {
            let mask_width = 8 * source_width / MSX_ACTIVE_WIDTH;
            source[..mask_width.saturating_sub(fine_scroll)].fill(backdrop);
        }
        let output_active_width = if self.version.is_v99x8() { 512 } else { 256 };
        let horizontal_scale = output_active_width / source_width;
        for (source_x, color) in source.iter().copied().enumerate().take(source_width) {
            for scale in 0..horizontal_scale {
                let destination = active_start_x
                    + i16::try_from((source_x + fine_scroll) * horizontal_scale + scale).unwrap();
                if (0..i16::try_from(self.surface_width).unwrap()).contains(&destination) {
                    row[usize::try_from(destination).unwrap()] = color;
                }
            }
        }
        for (source_x, color) in sprites.iter().copied().enumerate().take(source_width) {
            if color == TRANSPARENT_SPRITE_PIXEL {
                continue;
            }
            for scale in 0..horizontal_scale {
                let destination =
                    active_start_x + i16::try_from(source_x * horizontal_scale + scale).unwrap();
                if (0..i16::try_from(self.surface_width).unwrap()).contains(&destination) {
                    row[usize::try_from(destination).unwrap()] = color;
                }
            }
        }
        convert_row(inputs.state, row, &mut self.line_rgba, row_start);
        status
    }

    /// Copies the completed scanline image to the presented framebuffer.
    pub fn present_latched_frame(&mut self) -> (u32, u32) {
        self.framebuffer.copy_from_slice(&self.line_rgba);
        self.dimensions()
    }

    /// Last presented packed RGBA framebuffer.
    pub fn framebuffer(&self) -> &[u8] {
        &self.framebuffer
    }
}

impl Default for MsxRenderer {
    /// Creates an MSX1 renderer.
    fn default() -> Self {
        Self::new()
    }
}

/// Draws and horizontally scrolls one background scanline.
fn draw_scrolled_background(
    inputs: &RenderInputsMsx<'_>,
    mode: MsxVdpDisplayMode,
    line: usize,
    backdrop: u16,
    source_width: usize,
    output: &mut [u16; 512],
) {
    if !horizontal_scroll_applies(inputs.state, mode) {
        draw_background(inputs, mode, line, backdrop, output);
        return;
    }

    let multipage = inputs.state.horizontal_multipage_enabled() && mode.is_bitmap();
    let first_state = if multipage && inputs.state.horizontal_scroll() & 0x20 == 0 {
        inputs.state.with_toggled_display_page()
    } else {
        inputs.state
    };
    let first_inputs = RenderInputsMsx {
        vram: inputs.vram,
        state: first_state,
    };
    let mut first_page = [backdrop; 512];
    draw_background(&first_inputs, mode, line, backdrop, &mut first_page);

    let mut second_page = [backdrop; 512];
    if multipage {
        let second_inputs = RenderInputsMsx {
            vram: inputs.vram,
            state: first_state.with_toggled_display_page(),
        };
        draw_background(&second_inputs, mode, line, backdrop, &mut second_page);
    }

    let coarse_scroll =
        usize::from(inputs.state.horizontal_scroll() & 0x1F) * 8 * source_width / MSX_ACTIVE_WIDTH;
    for (destination, pixel) in output.iter_mut().enumerate().take(source_width) {
        let source = coarse_scroll + destination;
        let page = source / source_width;
        let source = source % source_width;
        *pixel = if multipage && page & 1 != 0 {
            second_page[source]
        } else {
            first_page[source]
        };
    }
}

/// Whether V9958 horizontal scrolling applies to this display mode.
const fn horizontal_scroll_applies(state: MsxVdpRenderState, mode: MsxVdpDisplayMode) -> bool {
    matches!(state.version(), MsxVdpVersion::V9958)
        && !matches!(
            mode,
            MsxVdpDisplayMode::Text1 | MsxVdpDisplayMode::Text2 | MsxVdpDisplayMode::Unsupported
        )
}

/// Draws the background layer for one active scanline.
fn draw_background(
    inputs: &RenderInputsMsx<'_>,
    mode: MsxVdpDisplayMode,
    line: usize,
    backdrop: u16,
    output: &mut [u16; 512],
) {
    match mode {
        MsxVdpDisplayMode::Graphics1 => draw_graphics_one(inputs, line, backdrop, output),
        MsxVdpDisplayMode::Text1 => draw_text_one(inputs, line, backdrop, output),
        MsxVdpDisplayMode::Graphics2 | MsxVdpDisplayMode::Graphics3 => {
            draw_graphics_two(inputs, line, backdrop, output)
        }
        MsxVdpDisplayMode::Multicolor => draw_multicolor(inputs, line, backdrop, output),
        MsxVdpDisplayMode::Text2 => draw_text_two(inputs, line, backdrop, output),
        MsxVdpDisplayMode::Graphics4 => draw_bitmap(inputs, line, 4, output),
        MsxVdpDisplayMode::Graphics5 => draw_bitmap(inputs, line, 5, output),
        MsxVdpDisplayMode::Graphics6 | MsxVdpDisplayMode::Graphics7
            if inputs.state.yjk_enabled() =>
        {
            draw_yjk_bitmap(inputs, line, output)
        }
        MsxVdpDisplayMode::Graphics6 => draw_bitmap(inputs, line, 6, output),
        MsxVdpDisplayMode::Graphics7 => draw_bitmap(inputs, line, 7, output),
        MsxVdpDisplayMode::Unsupported => {}
    }
}

/// Draws one V9958 YJK or YAE bitmap scanline.
fn draw_yjk_bitmap(inputs: &RenderInputsMsx<'_>, active_line: usize, output: &mut [u16]) {
    let source_line = (active_line + usize::from(inputs.state.register(23))) & 0xFF;
    let field_page = inputs.state.register(9) & 0x0C == 0x0C && inputs.state.field();
    let mut page_base = usize::from(inputs.state.register(2) & 0x20) << 11;
    if field_page {
        page_base ^= 0x10000;
    }
    let line_base = page_base | (source_line * 128);
    for group in 0..64 {
        let address = line_base | (group * 2);
        let pixels = [
            read_vram(inputs.vram, address, V9938_VRAM_ADDRESS_MASK),
            read_vram(inputs.vram, 0x10000 | address, V9938_VRAM_ADDRESS_MASK),
            read_vram(inputs.vram, address + 1, V9938_VRAM_ADDRESS_MASK),
            read_vram(inputs.vram, 0x10000 | address | 1, V9938_VRAM_ADDRESS_MASK),
        ];
        let j =
            i16::from(pixels[2] & 7) + i16::from(pixels[3] & 3) * 8 - i16::from(pixels[3] & 4) * 8;
        let k =
            i16::from(pixels[0] & 7) + i16::from(pixels[1] & 3) * 8 - i16::from(pixels[1] & 4) * 8;
        for (offset, value) in pixels.into_iter().enumerate() {
            output[group * 4 + offset] = if inputs.state.yae_enabled() && value & 0x08 != 0 {
                u16::from(value >> 4)
            } else {
                YJK_COLOR_MARKER | yjk_color(value >> 3, j, k)
            };
        }
    }
}

/// Converts one YJK triplet to packed five-bit RGB.
fn yjk_color(y: u8, j: i16, k: i16) -> u16 {
    let y = i16::from(y);
    let red = (y + j).clamp(0, 31) as u16;
    let green = (y + k).clamp(0, 31) as u16;
    let blue = ((5 * y - 2 * j - k + 2) / 4).clamp(0, 31) as u16;
    red << 10 | green << 5 | blue
}

/// Draws one Graphics 1 scanline.
fn draw_graphics_one(inputs: &RenderInputsMsx<'_>, line: usize, backdrop: u16, output: &mut [u16]) {
    let registers = inputs.state.registers();
    let mask = address_mask(inputs.state.version());
    let name_base = table_address(inputs.state, 2, 10);
    let color_base = if inputs.state.version().is_v99x8() {
        (usize::from(inputs.state.register(10)) << 14) | (usize::from(registers[3]) << 6)
    } else {
        usize::from(registers[3]) << 6
    };
    let pattern_base = table_address(inputs.state, 4, 11);
    let row = line / PATTERN_HEIGHT;
    let pattern_line = line & (PATTERN_HEIGHT - 1);
    for column in 0..GRAPHICS_COLUMNS {
        let character = usize::from(read_vram(
            inputs.vram,
            name_base + row * GRAPHICS_COLUMNS + column,
            mask,
        ));
        let pattern = read_vram(
            inputs.vram,
            pattern_base + character * PATTERN_HEIGHT + pattern_line,
            mask,
        );
        let colors = read_vram(inputs.vram, color_base + character / 8, mask);
        draw_pattern(
            pattern,
            u16::from(colors >> 4),
            u16::from(colors & 0x0F),
            backdrop,
            &mut output[column * 8..column * 8 + 8],
        );
    }
}

/// Draws one Text 1 scanline.
fn draw_text_one(inputs: &RenderInputsMsx<'_>, line: usize, backdrop: u16, output: &mut [u16]) {
    let mask = address_mask(inputs.state.version());
    let name_base = table_address(inputs.state, 2, 10);
    let pattern_base = table_address(inputs.state, 4, 11);
    let row = line / PATTERN_HEIGHT;
    let pattern_line = line & (PATTERN_HEIGHT - 1);
    let foreground = u16::from(inputs.state.register(7) >> 4);
    for column in 0..TEXT_COLUMNS {
        let character = usize::from(read_vram(
            inputs.vram,
            name_base + row * TEXT_COLUMNS + column,
            mask,
        ));
        let pattern = read_vram(
            inputs.vram,
            pattern_base + character * PATTERN_HEIGHT + pattern_line,
            mask,
        );
        let start = TEXT_ACTIVE_MARGIN_LEFT + column * TEXT_GLYPH_WIDTH;
        draw_pattern(
            pattern,
            foreground,
            backdrop,
            backdrop,
            &mut output[start..start + TEXT_GLYPH_WIDTH],
        );
    }
}

/// Draws one Graphics 2 or Graphics 3 scanline.
fn draw_graphics_two(inputs: &RenderInputsMsx<'_>, line: usize, backdrop: u16, output: &mut [u16]) {
    let registers = inputs.state.registers();
    let mask = address_mask(inputs.state.version());
    let name_base = table_address(inputs.state, 2, 10);
    let color_base = if inputs.state.version().is_v99x8() {
        (usize::from(inputs.state.register(10)) << 14) | (usize::from(registers[3] & 0x80) << 6)
    } else {
        usize::from(registers[3] & 0x80) << 6
    };
    let color_mask = (usize::from(registers[3] & 0x7F) << 3) | 0x07;
    let pattern_base = usize::from(registers[4] & 0x3C) << 11;
    let pattern_mask = (usize::from(registers[4] & 0x03) << 8) | 0xFF;
    let row = line / PATTERN_HEIGHT;
    let pattern_line = line & (PATTERN_HEIGHT - 1);
    let region = (line >> 6) << 8;
    for column in 0..GRAPHICS_COLUMNS {
        let name = usize::from(read_vram(
            inputs.vram,
            name_base + row * GRAPHICS_COLUMNS + column,
            mask,
        ));
        let character = name | region;
        let pattern = read_vram(
            inputs.vram,
            pattern_base + (character & pattern_mask) * PATTERN_HEIGHT + pattern_line,
            mask,
        );
        let colors = read_vram(
            inputs.vram,
            color_base + (character & color_mask) * PATTERN_HEIGHT + pattern_line,
            mask,
        );
        draw_pattern(
            pattern,
            u16::from(colors >> 4),
            u16::from(colors & 0x0F),
            backdrop,
            &mut output[column * 8..column * 8 + 8],
        );
    }
}

/// Draws one Multicolor scanline.
fn draw_multicolor(inputs: &RenderInputsMsx<'_>, line: usize, backdrop: u16, output: &mut [u16]) {
    let mask = address_mask(inputs.state.version());
    let name_base = table_address(inputs.state, 2, 10);
    let pattern_base = table_address(inputs.state, 4, 11);
    let row = line / PATTERN_HEIGHT;
    let color_line = (line >> 2) & (PATTERN_HEIGHT - 1);
    for column in 0..GRAPHICS_COLUMNS {
        let character = usize::from(read_vram(
            inputs.vram,
            name_base + row * GRAPHICS_COLUMNS + column,
            mask,
        ));
        let colors = read_vram(
            inputs.vram,
            pattern_base + character * PATTERN_HEIGHT + color_line,
            mask,
        );
        let start = column * GRAPHICS_GLYPH_WIDTH;
        output[start..start + MULTICOLOR_BLOCK_WIDTH]
            .fill(resolve_color(u16::from(colors >> 4), backdrop));
        output[start + MULTICOLOR_BLOCK_WIDTH..start + GRAPHICS_GLYPH_WIDTH]
            .fill(resolve_color(u16::from(colors & 0x0F), backdrop));
    }
}

/// Draws one Text 2 scanline with blink attributes.
fn draw_text_two(inputs: &RenderInputsMsx<'_>, line: usize, backdrop: u16, output: &mut [u16]) {
    let mask = V9938_VRAM_ADDRESS_MASK;
    let name_base = usize::from(inputs.state.register(2) & 0x7C) << 10;
    let pattern_base = table_address(inputs.state, 4, 11);
    let color_base = (usize::from(inputs.state.register(10)) << 14)
        | (usize::from(inputs.state.register(3) & 0xF8) << 6);
    let row = line / PATTERN_HEIGHT;
    let pattern_line = line & (PATTERN_HEIGHT - 1);
    for column in 0..TEXT_TWO_COLUMNS {
        let offset = row * TEXT_TWO_COLUMNS + column;
        let character = usize::from(read_vram(inputs.vram, name_base + offset, mask));
        let pattern = read_vram(
            inputs.vram,
            pattern_base + character * PATTERN_HEIGHT + pattern_line,
            mask,
        );
        let blink = read_vram(inputs.vram, color_base + offset / 8, mask) & (0x80 >> (offset & 7))
            != 0
            && inputs.state.blink();
        let colors = if blink {
            inputs.state.register(12)
        } else {
            inputs.state.register(7)
        };
        let start = TEXT_TWO_ACTIVE_MARGIN_LEFT + column * TEXT_GLYPH_WIDTH;
        draw_pattern(
            pattern,
            u16::from(colors >> 4),
            u16::from(colors & 0x0F),
            backdrop,
            &mut output[start..start + TEXT_GLYPH_WIDTH],
        );
    }
}

/// Draws one V9938 bitmap scanline.
fn draw_bitmap(inputs: &RenderInputsMsx<'_>, active_line: usize, screen: u8, output: &mut [u16]) {
    let source_line = (active_line + usize::from(inputs.state.register(23))) & 0xFF;
    let field_page = inputs.state.register(9) & 0x0C == 0x0C && inputs.state.field();
    let page_base = match screen {
        4 | 5 => {
            let mut base = usize::from(inputs.state.register(2) & 0x60) << 10;
            if field_page {
                base ^= 0x8000;
            }
            base
        }
        6 | 7 => {
            let mut base = usize::from(inputs.state.register(2) & 0x20) << 11;
            if field_page {
                base ^= 0x10000;
            }
            base
        }
        _ => 0,
    };
    match screen {
        4 => {
            let line_base = page_base + source_line * 128;
            for byte_index in 0..128 {
                let value = read_vram(inputs.vram, line_base + byte_index, V9938_VRAM_ADDRESS_MASK);
                output[byte_index * 2] = u16::from(value >> 4);
                output[byte_index * 2 + 1] = u16::from(value & 0x0F);
            }
        }
        5 => {
            let line_base = page_base + source_line * 128;
            for byte_index in 0..128 {
                let value = read_vram(inputs.vram, line_base + byte_index, V9938_VRAM_ADDRESS_MASK);
                for pixel in 0..4 {
                    output[byte_index * 4 + pixel] = u16::from((value >> ((3 - pixel) * 2)) & 3);
                }
            }
        }
        6 => {
            for byte_index in 0..256 {
                let address =
                    ((byte_index & 1) << 16) | page_base | (source_line * 128) | (byte_index >> 1);
                let value = read_vram(inputs.vram, address, V9938_VRAM_ADDRESS_MASK);
                output[byte_index * 2] = u16::from(value >> 4);
                output[byte_index * 2 + 1] = u16::from(value & 0x0F);
            }
        }
        7 => {
            for (source_x, pixel) in output.iter_mut().enumerate().take(256) {
                let address =
                    ((source_x & 1) << 16) | page_base | (source_line * 128) | (source_x >> 1);
                *pixel = FIXED_COLOR_MARKER
                    | u16::from(read_vram(inputs.vram, address, V9938_VRAM_ADDRESS_MASK));
            }
        }
        _ => {}
    }
    if !matches!(inputs.state.display_mode(), MsxVdpDisplayMode::Graphics7)
        && inputs.state.register(8) & 0x20 == 0
    {
        let backdrop = backdrop_color(inputs.state);
        for color in output {
            *color = resolve_color(*color, backdrop);
        }
    }
}

/// Expands one pattern byte into resolved color indexes.
fn draw_pattern(pattern: u8, foreground: u16, background: u16, backdrop: u16, output: &mut [u16]) {
    for (pixel, value) in output.iter_mut().zip((0..8).map(|bit| 0x80 >> bit)) {
        let color = if pattern & value != 0 {
            foreground
        } else {
            background
        };
        *pixel = resolve_color(color, backdrop);
    }
}

/// Evaluates the sprite plane for one active-display line.
fn draw_sprite_line(
    inputs: &RenderInputsMsx<'_>,
    line: usize,
    output: &mut [u16],
) -> MsxSpriteLineStatus {
    match inputs.state.display_mode().sprite_mode() {
        1 => draw_sprites_mode_one(inputs, line, output),
        2 if inputs.state.register(8) & 0x02 == 0 => draw_sprites_mode_two(inputs, line, output),
        _ => MsxSpriteLineStatus::default(),
    }
}

/// Evaluates and draws TMS sprite mode one for one scanline.
fn draw_sprites_mode_one(
    inputs: &RenderInputsMsx<'_>,
    line: usize,
    output: &mut [u16],
) -> MsxSpriteLineStatus {
    let registers = inputs.state.registers();
    let mask = address_mask(inputs.state.version());
    let attribute_base = sprite_attribute_base(inputs.state);
    let pattern_base = table_address(inputs.state, 6, 11);
    let sprite_size = if registers[1] & 0x02 != 0 { 16 } else { 8 };
    let magnified = registers[1] & 0x01 != 0;
    let displayed_size = sprite_size * if magnified { 2 } else { 1 };
    let sprite_line = (line + usize::from(inputs.state.register(23))) & 0xFF;
    let mut occupied = [false; MSX_ACTIVE_WIDTH];
    let mut visual = [false; MSX_ACTIVE_WIDTH];
    let mut visible_count = 0;
    let mut status = MsxSpriteLineStatus::default();
    let transparency = !inputs.state.version().is_v99x8() || inputs.state.register(8) & 0x20 == 0;

    for sprite in 0..SPRITE_COUNT {
        let entry = attribute_base + sprite * SPRITE_ATTRIBUTE_BYTES;
        let raw_y = read_vram(inputs.vram, entry, mask);
        status.last_sprite = sprite as u8;
        if raw_y == SPRITE_MODE_ONE_TERMINATOR_Y {
            break;
        }
        let top = sprite_top(raw_y);
        let relative = sprite_line as i16 - top;
        if relative < 0 || relative >= displayed_size as i16 {
            continue;
        }
        if visible_count == MODE_ONE_SPRITES_PER_LINE {
            status.overflow_sprite = Some(sprite as u8);
            break;
        }
        visible_count += 1;
        let color_attribute = read_vram(inputs.vram, entry + 3, mask);
        let color = color_attribute & SPRITE_COLOR_MASK;
        draw_sprite_pattern(
            inputs.vram,
            mask,
            pattern_base,
            false,
            read_vram(inputs.vram, entry + 2, mask),
            read_vram(inputs.vram, entry + 1, mask),
            color_attribute,
            color,
            line,
            relative,
            sprite_size,
            magnified,
            &mut occupied,
            &mut visual,
            output,
            &mut status,
            true,
            transparency,
        );
    }
    status
}

/// Evaluates and draws V9938 sprite mode two for one scanline.
fn draw_sprites_mode_two(
    inputs: &RenderInputsMsx<'_>,
    line: usize,
    output: &mut [u16],
) -> MsxSpriteLineStatus {
    let mask = V9938_VRAM_ADDRESS_MASK;
    let (attribute_base, color_base) = sprite_mode_two_table_bases(inputs.state);
    let pattern_base = table_address(inputs.state, 6, 11);
    let planar = inputs.state.display_mode().is_planar();
    let sprite_size = if inputs.state.register(1) & 0x02 != 0 {
        16
    } else {
        8
    };
    let magnified = inputs.state.register(1) & 0x01 != 0;
    let displayed_size = sprite_size * if magnified { 2 } else { 1 };
    let sprite_line = (line + usize::from(inputs.state.register(23))) & 0xFF;
    let mut collision = [false; MSX_ACTIVE_WIDTH];
    let mut visible_sprites = [ModeTwoSpriteLine::default(); MODE_TWO_SPRITES_PER_LINE];
    let pattern_source = SpritePatternSource {
        vram: inputs.vram,
        mask,
        pattern_base,
        planar,
    };
    let mut visible_count = 0;
    let mut status = MsxSpriteLineStatus::default();
    let transparency = inputs.state.register(8) & 0x20 == 0;

    for sprite in 0..SPRITE_COUNT {
        let entry = attribute_base + sprite * SPRITE_ATTRIBUTE_BYTES;
        let raw_y = read_sprite_vram(inputs.vram, entry, mask, planar);
        status.last_sprite = sprite as u8;
        if raw_y == SPRITE_MODE_TWO_TERMINATOR_Y {
            break;
        }
        let top = sprite_top(raw_y);
        let relative = sprite_line as i16 - top;
        if relative < 0 || relative >= displayed_size as i16 {
            continue;
        }
        if visible_count == MODE_TWO_SPRITES_PER_LINE {
            status.overflow_sprite = Some(sprite as u8);
            break;
        }
        let source_line = usize::try_from(relative).unwrap() / if magnified { 2 } else { 1 };
        let color_attribute = read_sprite_vram(
            inputs.vram,
            color_base + sprite * 16 + source_line,
            mask,
            planar,
        );
        let color = color_attribute & SPRITE_COLOR_MASK;
        let collision_enabled = color_attribute & (SPRITE_COLOR_COMBINE | SPRITE_COLLISION_INHIBIT)
            == 0
            && (color != 0 || !transparency);
        let mut occupied = [false; MSX_ACTIVE_WIDTH];
        draw_sprite_occupancy(
            pattern_source,
            read_sprite_vram(inputs.vram, entry + 2, mask, planar),
            read_sprite_vram(inputs.vram, entry + 1, mask, planar),
            color_attribute,
            relative,
            sprite_size,
            magnified,
            &mut occupied,
        );
        for x in 0..MSX_ACTIVE_WIDTH {
            if occupied[x] && collision_enabled {
                if collision[x] && status.collision.is_none() {
                    status.collision = Some((x as u16, line as u16));
                }
                collision[x] = true;
            }
        }
        let mut x = i16::from(read_sprite_vram(inputs.vram, entry + 1, mask, planar));
        if color_attribute & SPRITE_EARLY_CLOCK != 0 {
            x -= SPRITE_EARLY_CLOCK_PIXELS;
        }
        visible_sprites[visible_count] = ModeTwoSpriteLine {
            pattern: sprite_line_pattern(
                pattern_source,
                read_sprite_vram(inputs.vram, entry + 2, mask, planar),
                source_line,
                sprite_size,
                magnified,
            ),
            x,
            color_attribute,
        };
        visible_count += 1;
    }
    if let Some(first_base) = visible_sprites[..visible_count]
        .iter()
        .position(|sprite| sprite.color_attribute & SPRITE_COLOR_COMBINE == 0)
    {
        for sprite_index in (first_base..visible_count).rev() {
            draw_sprite_mode_two_line(
                inputs.state,
                &visible_sprites[..visible_count],
                sprite_index,
                output,
                transparency,
            );
        }
    }
    status
}

/// Draws one collected V9938 sprite line with color-combine lookahead.
fn draw_sprite_mode_two_line(
    state: MsxVdpRenderState,
    visible_sprites: &[ModeTwoSpriteLine],
    sprite_index: usize,
    output: &mut [u16],
    transparency: bool,
) {
    let sprite = visible_sprites[sprite_index];
    let color = sprite.color_attribute & SPRITE_COLOR_MASK;
    if color == 0 && transparency {
        return;
    }
    let mut pattern = sprite.pattern;
    let mut x = sprite.x;
    while pattern != 0 {
        if pattern & 0x8000_0000 != 0 && (0..MSX_ACTIVE_WIDTH as i16).contains(&x) {
            let color = sprite_mode_two_pixel_color(visible_sprites, sprite_index, x, color);
            write_sprite_mode_two_pixel(state, output, usize::try_from(x).unwrap(), color);
        }
        pattern <<= 1;
        x += 1;
    }
}

/// Returns the color after OR-ing following color-combine sprites.
fn sprite_mode_two_pixel_color(
    visible_sprites: &[ModeTwoSpriteLine],
    sprite_index: usize,
    x: i16,
    mut color: u8,
) -> u8 {
    for combined in &visible_sprites[sprite_index + 1..] {
        if combined.color_attribute & SPRITE_COLOR_COMBINE == 0 {
            break;
        }
        let shift = x - combined.x;
        if (0..SPRITE_PATTERN_BITS as i16).contains(&shift)
            && (combined.pattern << u32::try_from(shift).unwrap()) & 0x8000_0000 != 0
        {
            color |= combined.color_attribute & SPRITE_COLOR_MASK;
        }
    }
    color
}

/// Writes one mode-two sprite pixel in the active display format.
fn write_sprite_mode_two_pixel(state: MsxVdpRenderState, output: &mut [u16], x: usize, color: u8) {
    if state.yjk_enabled() {
        output[x] = u16::from(color);
        return;
    }
    match state.display_mode() {
        MsxVdpDisplayMode::Graphics5 => {
            output[x * 2] = u16::from(color >> 2);
            output[x * 2 + 1] = u16::from(color & 3);
        }
        MsxVdpDisplayMode::Graphics6 => {
            output[x * 2] = u16::from(color);
            output[x * 2 + 1] = u16::from(color);
        }
        MsxVdpDisplayMode::Graphics7 => {
            output[x] = FIXED_SPRITE_COLOR_MARKER | u16::from(color);
        }
        _ => output[x] = u16::from(color),
    }
}

/// Draws one sprite pattern and reports its occupied pixels.
#[allow(clippy::too_many_arguments)]
fn draw_sprite_pattern(
    vram: &[u8],
    mask: usize,
    pattern_base: usize,
    planar: bool,
    pattern_number: u8,
    raw_x: u8,
    color_attribute: u8,
    color: u8,
    line: usize,
    relative: i16,
    sprite_size: usize,
    magnified: bool,
    occupied: &mut [bool; MSX_ACTIVE_WIDTH],
    visual: &mut [bool; MSX_ACTIVE_WIDTH],
    output: &mut [u16],
    status: &mut MsxSpriteLineStatus,
    compose: bool,
    transparency: bool,
) {
    let displayed_size = sprite_size * if magnified { 2 } else { 1 };
    let mut x = i16::from(raw_x);
    if color_attribute & SPRITE_EARLY_CLOCK != 0 {
        x -= SPRITE_EARLY_CLOCK_PIXELS;
    }
    let source_line = usize::try_from(relative).unwrap() / if magnified { 2 } else { 1 };
    let base_pattern = if sprite_size == 16 {
        pattern_number & 0xFC
    } else {
        pattern_number
    };
    for displayed_x in 0..displayed_size {
        let source_x = displayed_x / if magnified { 2 } else { 1 };
        let pattern_offset = if source_x < 8 { 0 } else { 16 };
        let pattern = read_sprite_vram(
            vram,
            pattern_base
                + usize::from(base_pattern) * PATTERN_HEIGHT
                + pattern_offset
                + source_line,
            mask,
            planar,
        );
        if pattern & (0x80 >> (source_x & 7)) == 0 {
            continue;
        }
        let target_x = x + displayed_x as i16;
        if !(0..MSX_ACTIVE_WIDTH as i16).contains(&target_x) {
            continue;
        }
        let target = usize::try_from(target_x).unwrap();
        if compose && occupied[target] && status.collision.is_none() {
            status.collision = Some((target as u16, line as u16));
        }
        occupied[target] = true;
        if compose && (color != 0 || !transparency) && !visual[target] {
            output[target] = u16::from(color);
            visual[target] = true;
        }
    }
}

/// Marks the occupied pixels of one sprite pattern.
#[allow(clippy::too_many_arguments)]
fn draw_sprite_occupancy(
    source: SpritePatternSource<'_>,
    pattern_number: u8,
    raw_x: u8,
    color_attribute: u8,
    relative: i16,
    sprite_size: usize,
    magnified: bool,
    occupied: &mut [bool; MSX_ACTIVE_WIDTH],
) {
    let mut x = i16::from(raw_x);
    if color_attribute & SPRITE_EARLY_CLOCK != 0 {
        x -= SPRITE_EARLY_CLOCK_PIXELS;
    }
    let source_line = usize::try_from(relative).unwrap() / if magnified { 2 } else { 1 };
    let mut pattern =
        sprite_line_pattern(source, pattern_number, source_line, sprite_size, magnified);
    while pattern != 0 {
        if pattern & 0x8000_0000 != 0 && (0..MSX_ACTIVE_WIDTH as i16).contains(&x) {
            occupied[usize::try_from(x).unwrap()] = true;
        }
        pattern <<= 1;
        x += 1;
    }
}

/// Returns one expanded sprite pattern row.
fn sprite_line_pattern(
    source: SpritePatternSource<'_>,
    pattern_number: u8,
    source_line: usize,
    sprite_size: usize,
    magnified: bool,
) -> u32 {
    let base_pattern = if sprite_size == 16 {
        pattern_number & 0xFC
    } else {
        pattern_number
    };
    let mut pattern = u32::from(read_sprite_vram(
        source.vram,
        source.pattern_base + usize::from(base_pattern) * PATTERN_HEIGHT + source_line,
        source.mask,
        source.planar,
    )) << 24;
    if sprite_size == 16 {
        pattern |= u32::from(read_sprite_vram(
            source.vram,
            source.pattern_base + usize::from(base_pattern) * PATTERN_HEIGHT + 16 + source_line,
            source.mask,
            source.planar,
        )) << 16;
    }
    if magnified {
        double_sprite_pattern(pattern)
    } else {
        pattern
    }
}

/// Doubles each occupied bit in a sixteen-pixel sprite pattern.
const fn double_sprite_pattern(pattern: u32) -> u32 {
    let pattern = (pattern | (pattern >> 8)) & 0xFF00_FF00;
    let pattern = (pattern | (pattern >> 4)) & 0xF0F0_F0F0;
    let pattern = (pattern | (pattern >> 2)) & 0xCCCC_CCCC;
    let pattern = (pattern | (pattern >> 1)) & 0xAAAA_AAAA;
    pattern | (pattern >> 1)
}

/// Converts one raw sprite Y value to its signed top coordinate.
fn sprite_top(raw_y: u8) -> i16 {
    if raw_y > 0xE0 {
        i16::from(raw_y) - 255
    } else {
        i16::from(raw_y) + 1
    }
}

/// Returns the sprite attribute table base.
fn sprite_attribute_base(state: MsxVdpRenderState) -> usize {
    if state.version().is_v99x8() {
        (usize::from(state.register(11)) << 15) | (usize::from(state.register(5)) << 7)
    } else {
        usize::from(state.register(5) & 0x7F) << 7
    }
}

/// Returns the mode-two attribute and color table bases.
fn sprite_mode_two_table_bases(state: MsxVdpRenderState) -> (usize, usize) {
    let register_five = usize::from(state.register(5));
    let color_base = (usize::from(state.register(11)) << 15) | ((register_five & 0xF8) << 7);
    let attribute_base = color_base | 0x0200;
    (attribute_base, color_base)
}

/// Returns one table base using the complete masked register.
fn table_address(state: MsxVdpRenderState, register: usize, shift: usize) -> usize {
    usize::from(state.register(register)) << shift
}

/// Returns the physical address mask for one VDP version.
const fn address_mask(version: MsxVdpVersion) -> usize {
    if version.is_v99x8() {
        V9938_VRAM_ADDRESS_MASK
    } else {
        TMS_VRAM_ADDRESS_MASK
    }
}

/// Replaces transparent color zero with the backdrop color.
fn resolve_color(color: u16, backdrop: u16) -> u16 {
    if color & 0x0F == 0 { backdrop } else { color }
}

/// Returns the backdrop color code for the selected mode.
fn backdrop_color(state: MsxVdpRenderState) -> u16 {
    if matches!(state.display_mode(), MsxVdpDisplayMode::Graphics7) {
        FIXED_COLOR_MARKER | u16::from(state.register(7))
    } else {
        u16::from(state.register(7) & 0x0F)
    }
}

/// Converts one latched logical row to packed RGBA.
fn convert_row(state: MsxVdpRenderState, row: &[u16], line_rgba: &mut [u8], row_start: usize) {
    let byte_start = row_start * MSX_PIXEL_BYTES;
    let destination = &mut line_rgba[byte_start..byte_start + row.len() * MSX_PIXEL_BYTES];
    for (color, pixel) in row.iter().copied().zip(destination.chunks_exact_mut(4)) {
        let rgba = if color & YJK_COLOR_MARKER != 0 {
            yjk_rgba(color)
        } else if color & FIXED_SPRITE_COLOR_MARKER != 0 {
            palette_rgba(SCREEN_EIGHT_SPRITE_PALETTE[usize::from(color as u8 & 0x0F)])
        } else if color & FIXED_COLOR_MARKER != 0 {
            screen_eight_rgba(color as u8)
        } else if state.version().is_v99x8() {
            palette_rgba(state.palette()[usize::from(color as u8 & 0x0F)])
        } else {
            TMS9118_RGBA[usize::from(color as u8 & 0x0F)]
        };
        pixel.copy_from_slice(&rgba);
    }
}

/// Converts packed five-bit RGB to packed RGBA.
fn yjk_rgba(color: u16) -> [u8; 4] {
    [
        expand_five_bits(((color >> 10) & 0x1F) as u8),
        expand_five_bits(((color >> 5) & 0x1F) as u8),
        expand_five_bits((color & 0x1F) as u8),
        0xFF,
    ]
}

/// Converts a V9938 GRB palette entry to packed RGBA.
fn palette_rgba(color: u16) -> [u8; 4] {
    [
        expand_three_bits(((color >> 4) & 7) as u8),
        expand_three_bits(((color >> 8) & 7) as u8),
        expand_three_bits((color & 7) as u8),
        0xFF,
    ]
}

/// Converts one SCREEN 8 GRB byte to packed RGBA.
fn screen_eight_rgba(color: u8) -> [u8; 4] {
    [
        expand_three_bits((color >> 2) & 7),
        expand_three_bits(color >> 5),
        expand_two_bits(color & 3),
        0xFF,
    ]
}

/// Expands a three-bit gun value to eight bits.
const fn expand_three_bits(value: u8) -> u8 {
    (value << 5) | (value << 2) | (value >> 1)
}

/// Expands a two-bit gun value to eight bits.
const fn expand_two_bits(value: u8) -> u8 {
    value * 0x55
}

/// Expands a five-bit gun value to eight bits.
const fn expand_five_bits(value: u8) -> u8 {
    (value << 3) | (value >> 2)
}

/// Reads physical VRAM with hardware address wrapping.
fn read_vram(vram: &[u8], address: usize, mask: usize) -> u8 {
    vram.get(address & mask).copied().unwrap_or(0xFF)
}

/// Reads a sprite table byte through the planar address transformation.
fn read_sprite_vram(vram: &[u8], address: usize, mask: usize, planar: bool) -> u8 {
    let address = if planar {
        ((address << 16) | (address >> 1)) & V9938_VRAM_ADDRESS_MASK
    } else {
        address
    };
    read_vram(vram, address, mask)
}

#[cfg(test)]
mod tests {
    use device::video_msx::MsxVdp;

    use super::*;

    /// Creates a VDP and applies the supplied TMS register values.
    fn state(registers: [u8; 8]) -> MsxVdp {
        let mut vdp = MsxVdp::new(MsxVdpVersion::Tms9118, 0x4000);
        for (register, value) in registers.into_iter().enumerate() {
            vdp.control_write(value);
            vdp.control_write(0x80 | register as u8);
        }
        vdp
    }

    /// Writes bytes through the CPU VRAM port.
    fn write_vram(vdp: &mut MsxVdp, address: u32, values: &[u8]) {
        if vdp.version().is_v99x8() {
            vdp.control_write((address >> 14) as u8);
            vdp.control_write(0x80 | 14);
        }
        vdp.control_write(address as u8);
        vdp.control_write(0x40 | ((address >> 8) as u8 & 0x3F));
        for value in values {
            vdp.data_write(*value);
        }
    }

    /// Writes one VDP register.
    fn write_register(vdp: &mut MsxVdp, register: u8, value: u8) {
        vdp.control_write(value);
        vdp.control_write(0x80 | register);
    }

    /// Returns one packed framebuffer pixel.
    fn pixel(renderer: &MsxRenderer, x: usize, y: usize) -> [u8; 4] {
        let width = renderer.dimensions().0 as usize;
        let start = (y * width + x) * MSX_PIXEL_BYTES;
        renderer.framebuffer()[start..start + MSX_PIXEL_BYTES]
            .try_into()
            .unwrap()
    }

    #[test]
    /// Graphics 1 preserves the existing MSX1 border and active-area layout.
    fn graphics_one_and_physical_borders_are_latched() {
        let mut vdp = state([0, 0x40, 6, 0x20, 0, 0, 0, 0x04]);
        write_vram(&mut vdp, 0x1800, &[1]);
        write_vram(&mut vdp, 8, &[0x80]);
        write_vram(&mut vdp, 0x0800, &[0xF2]);

        let mut renderer = MsxRenderer::new();
        renderer.latch_scanline(
            &RenderInputsMsx {
                vram: vdp.vram(),
                state: vdp.render_state(),
            },
            MSX_ACTIVE_START_LINE,
        );
        renderer.present_latched_frame();
        assert_eq!(renderer.dimensions(), (284, 240));
        assert_eq!(pixel(&renderer, 14, 17), TMS9118_RGBA[15]);
        assert_eq!(pixel(&renderer, 15, 17), TMS9118_RGBA[2]);
    }

    #[test]
    /// MSX2 low-resolution pixels double into the 568-pixel surface.
    fn v9938_low_resolution_modes_are_doubled() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        for (register, value) in [0, 0x40, 6, 0x20, 0, 0, 0, 0x04].into_iter().enumerate() {
            write_register(&mut vdp, register as u8, value);
        }
        write_vram(&mut vdp, 0x1800, &[1]);
        write_vram(&mut vdp, 8, &[0x80]);
        write_vram(&mut vdp, 0x0800, &[0xF2]);
        let mut renderer = MsxRenderer::new_for_version(MsxVdpVersion::V9938);
        renderer.latch_scanline(
            &RenderInputsMsx {
                vram: vdp.vram(),
                state: vdp.render_state(),
            },
            MSX_ACTIVE_START_LINE,
        );
        renderer.present_latched_frame();
        assert_eq!(renderer.dimensions(), (568, 240));
        assert_eq!(pixel(&renderer, 28, 17), palette_rgba(0x777));
        assert_eq!(pixel(&renderer, 29, 17), palette_rgba(0x777));
    }

    #[test]
    /// SCREEN 8 converts all fixed-color component bits.
    fn screen_eight_color_conversion_is_exhaustive() {
        for color in 0..=u8::MAX {
            let rgba = screen_eight_rgba(color);
            assert_eq!(rgba[0], expand_three_bits((color >> 2) & 7));
            assert_eq!(rgba[1], expand_three_bits(color >> 5));
            assert_eq!(rgba[2], expand_two_bits(color & 3));
        }
    }

    #[test]
    /// V9938 palette conversion expands every gun value monotonically.
    fn palette_conversion_expands_all_gun_values() {
        for value in 0..8 {
            assert_eq!(
                palette_rgba(u16::from(value) << 8 | u16::from(value) << 4 | u16::from(value)),
                [
                    expand_three_bits(value),
                    expand_three_bits(value),
                    expand_three_bits(value),
                    0xFF,
                ]
            );
        }
    }

    #[test]
    /// Sprite mode one retains priority, collision, and overflow reporting.
    fn sprite_mode_one_priority_collision_and_overflow_are_reported() {
        let mut vdp = state([0, 0x40, 0, 0, 0, 0x20, 1, 1]);
        for sprite in 0..5 {
            let entry = 0x1000 + sprite * 4;
            write_vram(
                &mut vdp,
                entry,
                &[0xFF, 32, sprite as u8, 0x80 | (sprite as u8 + 2)],
            );
            write_vram(&mut vdp, 0x0800 + sprite * 8, &[0x80]);
        }
        let mut renderer = MsxRenderer::new();
        let status = renderer.latch_scanline(
            &RenderInputsMsx {
                vram: vdp.vram(),
                state: vdp.render_state(),
            },
            MSX_ACTIVE_START_LINE,
        );
        assert!(status.collision.is_some());
        assert_eq!(status.overflow_sprite, Some(4));
    }

    #[test]
    /// Graphics 4 and Graphics 5 unpack their distinct pixel formats.
    fn bitmap_modes_unpack_four_and_two_bit_pixels() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        write_register(&mut vdp, 0, 0x06);
        write_register(&mut vdp, 1, 0x40);
        write_vram(&mut vdp, 0, &[0xA3]);
        let mut renderer = MsxRenderer::new_for_version(MsxVdpVersion::V9938);
        renderer.latch_scanline(
            &RenderInputsMsx {
                vram: vdp.vram(),
                state: vdp.render_state(),
            },
            MSX_ACTIVE_START_LINE,
        );
        renderer.present_latched_frame();
        assert_eq!(pixel(&renderer, 28, 17), palette_rgba(0x661));
        assert_eq!(pixel(&renderer, 30, 17), palette_rgba(0x733));

        write_register(&mut vdp, 0, 0x08);
        write_vram(&mut vdp, 0, &[0b11_10_01_00]);
        renderer.clear_latched_frame();
        renderer.latch_scanline(
            &RenderInputsMsx {
                vram: vdp.vram(),
                state: vdp.render_state(),
            },
            MSX_ACTIVE_START_LINE,
        );
        renderer.present_latched_frame();
        assert_eq!(pixel(&renderer, 28, 17), palette_rgba(0x733));
        assert_eq!(pixel(&renderer, 29, 17), palette_rgba(0x611));
    }

    #[test]
    /// Text 2 renders eighty independent six-pixel cells.
    fn text_two_uses_its_name_pattern_and_color_tables() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        write_register(&mut vdp, 0, 0x04);
        write_register(&mut vdp, 1, 0x50);
        write_register(&mut vdp, 2, 0x03);
        write_register(&mut vdp, 3, 0x47);
        write_register(&mut vdp, 4, 1);
        write_register(&mut vdp, 7, 0xF1);
        write_vram(&mut vdp, 0, &[1]);
        write_vram(&mut vdp, 0x0808, &[0x80]);

        let mut renderer = MsxRenderer::new_for_version(MsxVdpVersion::V9938);
        renderer.latch_scanline(
            &RenderInputsMsx {
                vram: vdp.vram(),
                state: vdp.render_state(),
            },
            MSX_ACTIVE_START_LINE,
        );
        renderer.present_latched_frame();
        assert_eq!(pixel(&renderer, 44, 17), palette_rgba(0x777));
        assert_eq!(pixel(&renderer, 45, 17), palette_rgba(0x000));
        assert_eq!(pixel(&renderer, 523, 17), palette_rgba(0x000));

        write_register(&mut vdp, 12, 0x21);
        write_register(&mut vdp, 13, 0x10);
        write_vram(&mut vdp, 0x1000, &[0x80]);
        renderer.clear_latched_frame();
        renderer.latch_scanline(
            &RenderInputsMsx {
                vram: vdp.vram(),
                state: vdp.render_state(),
            },
            MSX_ACTIVE_START_LINE,
        );
        renderer.present_latched_frame();
        assert_eq!(pixel(&renderer, 44, 17), palette_rgba(0x611));
    }

    #[test]
    /// Graphics 6 interleaving and Graphics 7 fixed colors select both banks.
    fn bitmap_modes_unpack_interleaved_vram() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        write_register(&mut vdp, 0, 0x0A);
        write_register(&mut vdp, 1, 0x40);
        write_vram(&mut vdp, 0, &[0xA3, 0x5C]);
        let mut renderer = MsxRenderer::new_for_version(MsxVdpVersion::V9938);
        renderer.latch_scanline(
            &RenderInputsMsx {
                vram: vdp.vram(),
                state: vdp.render_state(),
            },
            MSX_ACTIVE_START_LINE,
        );
        renderer.present_latched_frame();
        assert_eq!(pixel(&renderer, 28, 17), palette_rgba(0x661));
        assert_eq!(pixel(&renderer, 29, 17), palette_rgba(0x733));
        assert_eq!(pixel(&renderer, 30, 17), palette_rgba(0x327));
        assert_eq!(pixel(&renderer, 31, 17), palette_rgba(0x411));

        write_register(&mut vdp, 0, 0x0E);
        write_vram(&mut vdp, 0, &[0xE3, 0x1C]);
        renderer.clear_latched_frame();
        renderer.latch_scanline(
            &RenderInputsMsx {
                vram: vdp.vram(),
                state: vdp.render_state(),
            },
            MSX_ACTIVE_START_LINE,
        );
        renderer.present_latched_frame();
        assert_eq!(pixel(&renderer, 28, 17), screen_eight_rgba(0xE3));
        assert_eq!(pixel(&renderer, 30, 17), screen_eight_rgba(0x1C));
    }

    #[test]
    /// Bitmap color zero follows the backdrop only while transparency is enabled.
    fn bitmap_color_zero_honors_the_v9938_transparency_bit() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        write_register(&mut vdp, 0, 0x0A);
        write_register(&mut vdp, 1, 0x40);
        write_register(&mut vdp, 7, 0x0F);
        let mut renderer = MsxRenderer::new_for_version(MsxVdpVersion::V9938);
        renderer.latch_scanline(
            &RenderInputsMsx {
                vram: vdp.vram(),
                state: vdp.render_state(),
            },
            MSX_ACTIVE_START_LINE,
        );
        renderer.present_latched_frame();
        assert_eq!(pixel(&renderer, 28, 17), palette_rgba(0x777));

        write_register(&mut vdp, 8, 0x20);
        renderer.clear_latched_frame();
        renderer.latch_scanline(
            &RenderInputsMsx {
                vram: vdp.vram(),
                state: vdp.render_state(),
            },
            MSX_ACTIVE_START_LINE,
        );
        renderer.present_latched_frame();
        assert_eq!(pixel(&renderer, 28, 17), palette_rgba(0x000));
    }

    #[test]
    /// Page, field, and adjustment registers change raster fetch placement.
    fn bitmap_page_interlace_and_adjustment_are_applied_per_scanline() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        write_register(&mut vdp, 0, 0x06);
        write_register(&mut vdp, 1, 0x40);
        write_register(&mut vdp, 2, 0x20);
        write_register(&mut vdp, 9, 0x0C);
        write_register(&mut vdp, 18, 0x11);
        write_vram(&mut vdp, 0, &[0xA0]);
        write_vram(&mut vdp, 0x8000, &[0x30]);
        let mut renderer = MsxRenderer::new_for_version(MsxVdpVersion::V9938);

        renderer.latch_scanline(
            &RenderInputsMsx {
                vram: vdp.vram(),
                state: vdp.render_state(),
            },
            MSX_ACTIVE_START_LINE,
        );
        renderer.latch_scanline(
            &RenderInputsMsx {
                vram: vdp.vram(),
                state: vdp.render_state(),
            },
            MSX_ACTIVE_START_LINE + 1,
        );
        renderer.present_latched_frame();
        assert_eq!(pixel(&renderer, 30, 17), palette_rgba(0x000));
        assert_eq!(pixel(&renderer, 30, 18), palette_rgba(0x733));

        vdp.start_frame();
        renderer.clear_latched_frame();
        renderer.latch_scanline(
            &RenderInputsMsx {
                vram: vdp.vram(),
                state: vdp.render_state(),
            },
            MSX_ACTIVE_START_LINE + 1,
        );
        renderer.present_latched_frame();
        assert_eq!(pixel(&renderer, 30, 18), palette_rgba(0x661));
    }

    #[test]
    /// Sprite mode two combines colors and reports collision and overflow.
    fn sprite_mode_two_priority_combine_collision_and_limit_are_reported() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        write_register(&mut vdp, 0, 0x04);
        write_register(&mut vdp, 1, 0x40);
        write_register(&mut vdp, 5, 0x27);
        write_register(&mut vdp, 6, 1);
        for sprite in 0..9 {
            write_vram(&mut vdp, 0x1200 + sprite * 4, &[0xFF, 32, sprite as u8, 0]);
            write_vram(&mut vdp, 0x0800 + sprite * 8, &[0x80]);
            let color = if sprite == 1 { 0x41 } else { sprite as u8 + 2 };
            write_vram(&mut vdp, 0x1000 + sprite * 16, &[color]);
        }
        let mut renderer = MsxRenderer::new_for_version(MsxVdpVersion::V9938);
        let status = renderer.latch_scanline(
            &RenderInputsMsx {
                vram: vdp.vram(),
                state: vdp.render_state(),
            },
            MSX_ACTIVE_START_LINE,
        );
        renderer.present_latched_frame();
        assert_eq!(status.collision, Some((32, 0)));
        assert_eq!(status.overflow_sprite, Some(8));
        assert_eq!(pixel(&renderer, 92, 17), palette_rgba(0x733));
    }

    #[test]
    /// Sprite patterns are fixed one scanline before their pixels are displayed.
    fn sprite_patterns_are_latched_one_line_ahead() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        write_register(&mut vdp, 0, 0x04);
        write_register(&mut vdp, 1, 0x40);
        write_register(&mut vdp, 5, 0x27);
        write_register(&mut vdp, 6, 1);
        write_vram(&mut vdp, 0x1200, &[0, 32, 0, 0]);
        write_vram(&mut vdp, 0x1000, &[3]);
        write_vram(&mut vdp, 0x0800, &[0x80]);
        let mut renderer = MsxRenderer::new_for_version(MsxVdpVersion::V9938);

        renderer.latch_scanline(
            &RenderInputsMsx {
                vram: vdp.vram(),
                state: vdp.render_state(),
            },
            MSX_ACTIVE_START_LINE,
        );
        write_vram(&mut vdp, 0x0800, &[0x40]);
        renderer.latch_scanline(
            &RenderInputsMsx {
                vram: vdp.vram(),
                state: vdp.render_state(),
            },
            MSX_ACTIVE_START_LINE + 1,
        );
        renderer.present_latched_frame();

        assert_eq!(pixel(&renderer, 92, 18), palette_rgba(0x733));
        assert_eq!(pixel(&renderer, 94, 18), palette_rgba(0x000));
    }

    #[test]
    /// High-resolution and fixed-color modes format sprite pixels separately.
    fn sprite_mode_two_uses_each_bitmap_mode_color_format() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        write_register(&mut vdp, 1, 0x40);
        write_register(&mut vdp, 5, 0x27);
        write_register(&mut vdp, 6, 1);
        let mut renderer = MsxRenderer::new_for_version(MsxVdpVersion::V9938);

        write_register(&mut vdp, 0, 0x08);
        write_vram(&mut vdp, 0x1200, &[0xFF, 1, 0, 0]);
        write_vram(&mut vdp, 0x0800, &[0x80]);
        write_vram(&mut vdp, 0x1000, &[0x0B]);
        renderer.latch_scanline(
            &RenderInputsMsx {
                vram: vdp.vram(),
                state: vdp.render_state(),
            },
            MSX_ACTIVE_START_LINE,
        );
        renderer.present_latched_frame();
        assert_eq!(pixel(&renderer, 30, 17), palette_rgba(0x611));
        assert_eq!(pixel(&renderer, 31, 17), palette_rgba(0x733));

        write_register(&mut vdp, 0, 0x0A);
        write_vram(&mut vdp, 0x1200, &[0xFF, 1, 0, 0]);
        write_vram(&mut vdp, 0x0800, &[0x80]);
        write_vram(&mut vdp, 0x1000, &[0x0B]);
        renderer.clear_latched_frame();
        renderer.latch_scanline(
            &RenderInputsMsx {
                vram: vdp.vram(),
                state: vdp.render_state(),
            },
            MSX_ACTIVE_START_LINE,
        );
        renderer.present_latched_frame();
        assert_eq!(pixel(&renderer, 30, 17), palette_rgba(0x664));
        assert_eq!(pixel(&renderer, 31, 17), palette_rgba(0x664));

        write_register(&mut vdp, 0, 0x0E);
        renderer.clear_latched_frame();
        renderer.latch_scanline(
            &RenderInputsMsx {
                vram: vdp.vram(),
                state: vdp.render_state(),
            },
            MSX_ACTIVE_START_LINE,
        );
        renderer.present_latched_frame();
        assert_eq!(pixel(&renderer, 30, 17), palette_rgba(0x077));
        assert_eq!(pixel(&renderer, 31, 17), palette_rgba(0x077));
    }

    #[test]
    /// Sprite mode two ignores the two low R5 bits and selects its fixed table halves.
    fn sprite_mode_two_uses_the_v9938_table_address_mask() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        write_register(&mut vdp, 0, 0x04);
        write_register(&mut vdp, 1, 0x40);
        write_register(&mut vdp, 5, 0xE7);
        write_register(&mut vdp, 6, 1);
        write_register(&mut vdp, 7, 1);
        write_vram(&mut vdp, 0x7200, &[0xFF, 32, 0, 0]);
        write_vram(&mut vdp, 0x7000, &[2]);
        write_vram(&mut vdp, 0x0800, &[0x80]);
        write_vram(&mut vdp, 0x7380, &[0xFF, 0, 1, 0]);
        write_vram(&mut vdp, 0x7180, &[4]);
        write_vram(&mut vdp, 0x0808, &[0x80]);

        let mut renderer = MsxRenderer::new_for_version(MsxVdpVersion::V9938);
        renderer.latch_scanline(
            &RenderInputsMsx {
                vram: vdp.vram(),
                state: vdp.render_state(),
            },
            MSX_ACTIVE_START_LINE,
        );
        renderer.present_latched_frame();

        assert_eq!(pixel(&renderer, 28, 17), palette_rgba(0x000));
        assert_eq!(pixel(&renderer, 92, 17), palette_rgba(0x611));
    }

    #[test]
    /// A color-combine sprite draws itself and merges with lower-numbered sprites.
    fn sprite_mode_two_color_combine_draws_and_merges_pixels() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        write_register(&mut vdp, 0, 0x04);
        write_register(&mut vdp, 1, 0x40);
        write_register(&mut vdp, 5, 0x27);
        write_register(&mut vdp, 6, 1);
        write_register(&mut vdp, 7, 1);
        write_vram(&mut vdp, 0x1200, &[0xFF, 32, 0, 0]);
        write_vram(&mut vdp, 0x1204, &[0xFF, 32, 1, 0]);
        write_vram(&mut vdp, 0x1208, &[0xD8]);
        write_vram(&mut vdp, 0x1000, &[2]);
        write_vram(&mut vdp, 0x1010, &[0x44]);
        write_vram(&mut vdp, 0x0800, &[0x80]);
        write_vram(&mut vdp, 0x0808, &[0xC0]);

        let mut renderer = MsxRenderer::new_for_version(MsxVdpVersion::V9938);
        renderer.latch_scanline(
            &RenderInputsMsx {
                vram: vdp.vram(),
                state: vdp.render_state(),
            },
            MSX_ACTIVE_START_LINE,
        );
        renderer.present_latched_frame();

        assert_eq!(pixel(&renderer, 92, 17), palette_rgba(0x151));
        assert_eq!(pixel(&renderer, 94, 17), palette_rgba(0x117));
    }

    #[test]
    /// YJK conversion clamps every signed chroma combination.
    fn yjk_conversion_covers_the_complete_color_domain() {
        for y in 0..32u8 {
            for j in -32..32i16 {
                for k in -32..32i16 {
                    let color = yjk_color(y, j, k);
                    let red = ((color >> 10) & 0x1F) as i16;
                    let green = ((color >> 5) & 0x1F) as i16;
                    let blue = (color & 0x1F) as i16;
                    assert_eq!(red, (i16::from(y) + j).clamp(0, 31));
                    assert_eq!(green, (i16::from(y) + k).clamp(0, 31));
                    assert_eq!(blue, ((5 * i16::from(y) - 2 * j - k + 2) / 4).clamp(0, 31));
                }
            }
        }
    }

    #[test]
    /// YJK filters both planar base modes into a 256-pixel display.
    fn yjk_filter_applies_to_graphics_six_and_seven() {
        for register_zero in [0x0A, 0x0E] {
            let mut vdp = MsxVdp::new(MsxVdpVersion::V9958, 0x20000);
            write_register(&mut vdp, 0, register_zero);
            write_register(&mut vdp, 1, 0x40);
            write_register(&mut vdp, 8, 0x02);
            write_register(&mut vdp, 25, 0x08);
            write_vram(&mut vdp, 0, &[0x78, 0x71, 0x72, 0x70]);
            let inputs = RenderInputsMsx {
                vram: vdp.vram(),
                state: vdp.render_state(),
            };

            let mut renderer = MsxRenderer::new_for_version(MsxVdpVersion::V9958);
            renderer.latch_scanline(&inputs, MSX_ACTIVE_START_LINE);
            renderer.present_latched_frame();

            let first = yjk_rgba(yjk_color(15, 2, 8));
            let second = yjk_rgba(yjk_color(14, 2, 8));
            assert_eq!(pixel(&renderer, 28, 17), first, "{register_zero:02X}");
            assert_eq!(pixel(&renderer, 29, 17), first, "{register_zero:02X}");
            assert_eq!(pixel(&renderer, 30, 17), second, "{register_zero:02X}");
            assert_eq!(pixel(&renderer, 31, 17), second, "{register_zero:02X}");
        }
    }

    #[test]
    /// YJK sprites use the programmable palette and 256-pixel coordinates.
    fn yjk_sprites_use_the_programmable_palette() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::V9958, 0x20000);
        write_register(&mut vdp, 0, 0x0E);
        write_register(&mut vdp, 1, 0x40);
        write_register(&mut vdp, 5, 0x27);
        write_register(&mut vdp, 6, 1);
        write_register(&mut vdp, 25, 0x08);
        write_vram(&mut vdp, 0x1200, &[0xFF, 1, 0, 0, 0xD8]);
        write_vram(&mut vdp, 0x1000, &[0x0B]);
        write_vram(&mut vdp, 0x0800, &[0x80]);

        let mut renderer = MsxRenderer::new_for_version(MsxVdpVersion::V9958);
        renderer.latch_scanline(
            &RenderInputsMsx {
                vram: vdp.vram(),
                state: vdp.render_state(),
            },
            MSX_ACTIVE_START_LINE,
        );
        renderer.present_latched_frame();

        assert_eq!(pixel(&renderer, 30, 17), palette_rgba(0x664));
        assert_eq!(pixel(&renderer, 31, 17), palette_rgba(0x664));
    }

    #[test]
    /// YAE selects the palette per pixel while preserving shared YJK chroma.
    fn yae_selects_palette_and_yjk_pixels() {
        for register_zero in [0x0A, 0x0E] {
            let mut vdp = MsxVdp::new(MsxVdpVersion::V9958, 0x20000);
            write_register(&mut vdp, 0, register_zero);
            write_register(&mut vdp, 1, 0x40);
            write_register(&mut vdp, 8, 0x02);
            write_register(&mut vdp, 25, 0x18);
            write_vram(&mut vdp, 0, &[0xF8, 0x70, 0x78, 0x70]);

            let mut renderer = MsxRenderer::new_for_version(MsxVdpVersion::V9958);
            renderer.latch_scanline(
                &RenderInputsMsx {
                    vram: vdp.vram(),
                    state: vdp.render_state(),
                },
                MSX_ACTIVE_START_LINE,
            );
            renderer.present_latched_frame();

            assert_eq!(pixel(&renderer, 28, 17), palette_rgba(0x777));
            assert_eq!(pixel(&renderer, 30, 17), yjk_rgba(yjk_color(14, 0, 0)));
            assert_eq!(pixel(&renderer, 32, 17), palette_rgba(0x627));
            assert_eq!(pixel(&renderer, 34, 17), yjk_rgba(yjk_color(14, 0, 0)));
        }
    }

    #[test]
    /// Multipage scrolling switches bitmap pages at the horizontal wrap.
    fn horizontal_scroll_wraps_into_the_second_bitmap_page() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::V9958, 0x20000);
        write_register(&mut vdp, 0, 0x06);
        write_register(&mut vdp, 1, 0x40);
        write_register(&mut vdp, 2, 0x20);
        write_register(&mut vdp, 25, 0x01);
        write_register(&mut vdp, 26, 31);
        write_vram(&mut vdp, 124, &[0x11; 4]);
        write_vram(&mut vdp, 0x8000, &[0x22; 4]);
        let inputs = RenderInputsMsx {
            vram: vdp.vram(),
            state: vdp.render_state(),
        };
        let mut output = [0; 512];

        draw_scrolled_background(
            &inputs,
            MsxVdpDisplayMode::Graphics4,
            0,
            0,
            256,
            &mut output,
        );

        assert_eq!(&output[..8], &[1; 8]);
        assert_eq!(&output[8..16], &[2; 8]);
    }

    #[test]
    /// Fine scrolling and left masking retain an eight-pixel border.
    fn fine_scroll_and_mask_share_the_left_border() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::V9958, 0x20000);
        write_register(&mut vdp, 0, 0x06);
        write_register(&mut vdp, 1, 0x40);
        write_register(&mut vdp, 25, 0x02);
        write_register(&mut vdp, 27, 1);
        write_vram(&mut vdp, 0, &[0x22; 8]);

        let mut renderer = MsxRenderer::new_for_version(MsxVdpVersion::V9958);
        renderer.latch_scanline(
            &RenderInputsMsx {
                vram: vdp.vram(),
                state: vdp.render_state(),
            },
            MSX_ACTIVE_START_LINE,
        );
        renderer.present_latched_frame();

        assert_eq!(pixel(&renderer, 42, 17), palette_rgba(0x000));
        assert_eq!(pixel(&renderer, 44, 17), palette_rgba(0x611));
        assert_eq!(pixel(&renderer, 46, 17), palette_rgba(0x611));
    }

    #[test]
    /// V9958 background scrolling and masking leave sprite coordinates unchanged.
    fn horizontal_scroll_and_mask_do_not_move_sprites() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::V9958, 0x20000);
        write_register(&mut vdp, 0, 0x06);
        write_register(&mut vdp, 1, 0x40);
        write_register(&mut vdp, 5, 0x27);
        write_register(&mut vdp, 6, 1);
        write_register(&mut vdp, 25, 0x02);
        write_register(&mut vdp, 26, 1);
        write_register(&mut vdp, 27, 7);
        write_vram(&mut vdp, 0x1200, &[0xFF, 32, 0, 0, 0xD8]);
        write_vram(&mut vdp, 0x1000, &[2]);
        write_vram(&mut vdp, 0x0800, &[0x80]);

        let mut renderer = MsxRenderer::new_for_version(MsxVdpVersion::V9958);
        renderer.latch_scanline(
            &RenderInputsMsx {
                vram: vdp.vram(),
                state: vdp.render_state(),
            },
            MSX_ACTIVE_START_LINE,
        );
        renderer.present_latched_frame();

        assert_eq!(pixel(&renderer, 92, 17), palette_rgba(0x611));
        assert_ne!(pixel(&renderer, 76, 17), palette_rgba(0x611));
    }
}
