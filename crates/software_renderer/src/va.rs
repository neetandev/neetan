//! PC-88VA display renderer.
//!
//! Composes the VA text, sprite, and graphic layers over the backdrop color
//! into a packed RGBA framebuffer.

mod compose;
mod graphics;
mod palette;
mod sprite;
mod text;

use alloc::{boxed::Box, vec};

pub use common::FramebufferVa;
use compose::{ComposeRegs, RasterLayers, compose_raster};
use graphics::GraphicsWork;
pub use palette::{adjust_color12, rgb8_to_va_color, va_color_to_rgba};
use sprite::SpriteWork;
use text::{LINEHEIGHT_MAX, TextContext, TextWork};

/// VA visible surface width in pixels.
pub const VA_SURFACE_WIDTH: usize = 640;
/// VA visible surface maximum height in pixels.
pub const VA_SURFACE_HEIGHT: usize = 480;
/// Text coordinate-system width in dots (the text plane is 1024 wide).
pub const VA_TEXT_COORD_WIDTH: usize = 1024;
/// Character cell width in dots.
pub const VA_CHAR_WIDTH: usize = 8;
/// Bytes per pixel (`R, G, B, A`).
pub const VA_PIXEL_BYTES: usize = 4;
/// VA framebuffer byte size.
pub const VA_FRAMEBUFFER_BYTES: usize = VA_SURFACE_WIDTH * VA_SURFACE_HEIGHT * VA_PIXEL_BYTES;

const ROW_BYTES: usize = VA_SURFACE_WIDTH * VA_PIXEL_BYTES;

/// Horizontal sync mode, selecting the display geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HsyncModeVa {
    /// 15.98 kHz non-interlaced.
    Khz15_98,
    /// 15.73 kHz interlaced.
    Khz15_73,
    /// 24.8 kHz non-interlaced.
    Khz24_8,
}

/// Per-frame inputs to the VA renderer.
pub struct RenderInputsVa<'a> {
    /// Text VRAM image (256 KiB).
    pub text_vram: &'a [u8],
    /// Text-table base offset (TSP.
    pub text_table: usize,
    /// Attribute byte offset relative to a character code.
    pub attr_offset: usize,
    /// Scanlines per text row.
    pub line_height: usize,
    /// Horizontal-line raster position.
    pub horizontal_line_position: usize,
    /// Text blink counter stage 2.
    pub blink_counter2: u8,
    /// Text 2x vertical magnification.
    pub text_magnify: bool,
    /// Programmed screen line count.
    pub screen_lines: usize,
    /// SYNC parameter 0 (selects the 200-line text doubling case).
    pub sync_param0: u8,
    /// Horizontal sync mode.
    pub hsync_mode: HsyncModeVa,

    /// Sprite-table base offset within text VRAM.
    pub sprite_table: usize,
    /// Sprite display enabled.
    pub sprite_enabled: bool,
    /// Maximum sprites per raster minus one.
    pub sprite_count_limit: u8,
    /// Sprite 2x vertical magnification.
    pub sprite_magnify: bool,
    /// Sprite grouping mode; carried but not used by the renderer.
    pub sprite_grouping: bool,
    /// Cursor sprite number.
    pub cursor_sprite: u8,
    /// Cursor blink enable.
    pub cursor_blink_enable: bool,

    /// Text control port 0; 40-column when zero.
    pub txtmode8: u8,
    /// Text control port 1; bit 2 = 8-dot font, bit 7 = display off.
    pub txtmode: u8,
    /// Display-mode register.
    pub graphics_mode: u16,
    /// Graphics-resolution register.
    pub graphics_resolution: u16,
    /// Palette-screen composition register.
    pub color_composition: u16,
    /// Direct-color screen composition register.
    pub rgb_composition: u16,
    /// Palette-mode register.
    pub palette_mode: u16,
    /// Color-code / plane-mask register.
    pub page_mask: u16,
    /// Backdrop color.
    pub backdrop_color: u16,
    /// Text/sprite transparent-color register.
    pub transparent_text_sprite: u16,
    /// Graphic-0 transparent-color register.
    pub transparent_graphic0: u16,
    /// Graphic-1 transparent-color register.
    pub transparent_graphic1: u16,
    /// Screen-mask mode register.
    pub mask_mode: u16,
    /// Screen-mask left bound in dots.
    pub mask_left: u16,
    /// Screen-mask right bound in dots.
    pub mask_right: u16,
    /// Screen-mask top bound in half-lines.
    pub mask_top: u16,
    /// Screen-mask bottom bound in half-lines.
    pub mask_bottom: u16,
    /// Palette-blink frame counter.
    pub palette_blink_counter: u16,
    /// The 32 palette entries as 16-bit VA color codes.
    pub palette: &'a [u16; 32],
    /// Graphics VRAM image (256 KiB).
    pub graphics_vram: &'a [u8],
    /// The four graphics framebuffer descriptors.
    pub framebuffers: [FramebufferVa; 4],
}

/// Persistent buffers owned by the VA renderer.
pub struct VaRendererState {
    /// Internal copy of the VA font ROM (character generator source).
    pub font_rom: Box<[u8]>,
    /// Packed RGBA framebuffer (`R, G, B, A` little-endian).
    pub framebuffer: Box<[u8]>,
}

save_state::runtime_state! {
/// Save-state portion of the PC-88VA renderer.
#[derive(Clone)]
pub struct VaRendererRuntimeState {
    framebuffer: Box<[u8]>,
}}

/// CPU-side renderer for the PC-88VA display.
pub struct VaRenderer {
    /// Embedded state for save/restore.
    pub state: VaRendererState,
    /// Scratch text-layer walk state, reused every frame.
    text: TextWork,
    /// Scratch sprite-layer walk state, reused every frame.
    sprite: SpriteWork,
    /// Scratch graphic-layer walk state, reused every frame.
    graphics: GraphicsWork,
}

impl VaRenderer {
    /// Creates a renderer with the given VA font ROM.
    pub fn new(font_rom_data: &[u8]) -> Self {
        Self {
            state: VaRendererState {
                font_rom: copy_font(font_rom_data),
                framebuffer: vec![0u8; VA_FRAMEBUFFER_BYTES].into_boxed_slice(),
            },
            text: TextWork::new(),
            sprite: SpriteWork::new(),
            graphics: GraphicsWork::new(),
        }
    }

    /// Replaces the font ROM.
    pub fn update_font_rom(&mut self, font_rom_data: &[u8]) {
        self.state.font_rom = copy_font(font_rom_data);
    }

    /// Returns the packed RGBA framebuffer.
    pub fn framebuffer(&self) -> &[u8] {
        &self.state.framebuffer
    }

    /// Captures the presented framebuffer without the immutable font.
    pub fn capture_state(&self) -> VaRendererRuntimeState {
        VaRendererRuntimeState {
            framebuffer: self.state.framebuffer.clone(),
        }
    }

    /// Restores the presented framebuffer without changing the font.
    pub fn restore_state(
        &mut self,
        state: VaRendererRuntimeState,
    ) -> Result<(), save_state::StateValidationError> {
        if state.framebuffer.len() != VA_FRAMEBUFFER_BYTES {
            return Err(save_state::StateValidationError::new(
                "PC-88VA renderer state is invalid",
            ));
        }
        self.state.framebuffer = state.framebuffer;
        Ok(())
    }

    /// Renders one frame and returns the `(width, height)` of the valid region.
    pub fn render(&mut self, inputs: &RenderInputsVa<'_>) -> (u32, u32) {
        let mut lines = inputs.screen_lines;
        if inputs.hsync_mode != HsyncModeVa::Khz24_8 {
            lines *= 2;
        }
        lines = lines.min(VA_SURFACE_HEIGHT);

        let regs = ComposeRegs {
            color_composition: inputs.color_composition,
            rgb_composition: inputs.rgb_composition,
            palette_mode: inputs.palette_mode,
            page_mask: inputs.page_mask,
            transparent_text_sprite: inputs.transparent_text_sprite,
            transparent_graphic0: inputs.transparent_graphic0,
            transparent_graphic1: inputs.transparent_graphic1,
            graphics_mode: inputs.graphics_mode,
            graphics_resolution: inputs.graphics_resolution,
            mask_mode: inputs.mask_mode,
            mask_left: inputs.mask_left,
            mask_right: inputs.mask_right,
            mask_top: inputs.mask_top,
            mask_bottom: inputs.mask_bottom,
            palette_blink_counter: inputs.palette_blink_counter,
        };
        let mut palette_rgba = [0u32; 32];
        for (entry, color) in palette_rgba.iter_mut().zip(inputs.palette.iter()) {
            *entry = va_color_to_rgba(*color);
        }
        let backdrop = va_color_to_rgba(inputs.backdrop_color);

        let line_height = inputs.line_height.clamp(1, LINEHEIGHT_MAX);
        let text_200 =
            inputs.hsync_mode != HsyncModeVa::Khz24_8 && (inputs.sync_param0 & 0xC0) != 0x40;

        let Self {
            state,
            text,
            sprite,
            graphics,
        } = self;
        let VaRendererState {
            font_rom,
            framebuffer,
        } = state;

        let grph200 = graphics.begin(
            inputs.graphics_mode,
            inputs.graphics_resolution,
            &inputs.framebuffers,
        );

        text.begin(inputs.text_vram, inputs.text_table, line_height);
        sprite.begin(
            inputs.text_vram,
            inputs.sprite_table,
            inputs.sprite_enabled,
            inputs.cursor_sprite,
            inputs.cursor_blink_enable,
            inputs.blink_counter2,
        );
        let context = TextContext {
            text_vram: inputs.text_vram,
            font_rom,
            attr_offset: inputs.attr_offset,
            horizontal_line_position: inputs.horizontal_line_position,
            blink_counter2: inputs.blink_counter2,
            text_magnify: inputs.text_magnify,
            eight_dot: inputs.txtmode & 0x04 != 0,
            text_off: inputs.txtmode & 0x80 != 0,
            forty_column: inputs.txtmode8 == 0,
        };

        let not_24khz = inputs.hsync_mode != HsyncModeVa::Khz24_8;
        let interlace_mode = inputs.graphics_mode & 0x00C0;
        let sprite_two_hundred = inputs.sync_param0 & 0xC0 == 0x40;
        let draw_graphics = |graphics: &mut GraphicsWork| {
            graphics.raster(
                inputs.graphics_vram,
                inputs.graphics_mode,
                &inputs.framebuffers,
                inputs.page_mask,
            );
        };
        let draw_sprite = |sprite: &mut SpriteWork| {
            sprite.raster(
                inputs.text_vram,
                inputs.sprite_magnify,
                sprite_two_hundred,
                inputs.sprite_count_limit,
            );
        };

        let mut y = 0;
        while y + 1 < lines {
            // Even scanline: text, sprite, and graphics are active. The sprite
            // layer is driven in lockstep with the text layer; both are the
            // unified TSP layer.
            text.raster(&context);
            draw_sprite(sprite);
            draw_graphics(graphics);
            compose_into(
                &regs,
                &palette_rgba,
                backdrop,
                text,
                sprite,
                graphics,
                y,
                framebuffer,
            );
            y += 1;

            // Odd scanline.
            if not_24khz {
                match interlace_mode {
                    0x80 => {
                        // Interlace mode 0: text repeats unless 200-line; graphics held.
                        if !text_200 {
                            text.raster(&context);
                            draw_sprite(sprite);
                        }
                    }
                    0xC0 => {
                        // Interlace mode 1: text and graphics repeat unless 200-line.
                        if !text_200 {
                            text.raster(&context);
                            draw_sprite(sprite);
                        }
                        if !grph200 {
                            draw_graphics(graphics);
                        }
                    }
                    _ => {
                        // Non-interlace: the doubled line is blank.
                        text.blank_raster();
                        sprite.blank_raster();
                        graphics.blank_raster();
                    }
                }
            } else {
                match interlace_mode {
                    0x00 => {
                        text.raster(&context);
                        draw_sprite(sprite);
                        if grph200 {
                            graphics.blank_raster();
                        } else {
                            draw_graphics(graphics);
                        }
                    }
                    0x40 => {
                        text.raster(&context);
                        draw_sprite(sprite);
                        if !grph200 {
                            draw_graphics(graphics);
                        }
                    }
                    _ => {
                        // Interlace is disabled in 24 kHz mode: blank.
                        text.blank_raster();
                        sprite.blank_raster();
                        graphics.blank_raster();
                    }
                }
            }
            compose_into(
                &regs,
                &palette_rgba,
                backdrop,
                text,
                sprite,
                graphics,
                y,
                framebuffer,
            );
            y += 1;
        }

        (VA_SURFACE_WIDTH as u32, lines as u32)
    }
}

#[allow(clippy::too_many_arguments)]
fn compose_into(
    regs: &ComposeRegs,
    palette_rgba: &[u32; 32],
    backdrop: u32,
    text: &TextWork,
    sprite: &SpriteWork,
    graphics: &GraphicsWork,
    y: usize,
    framebuffer: &mut [u8],
) {
    let row = &mut framebuffer[y * ROW_BYTES..][..ROW_BYTES];
    let layers = RasterLayers {
        text: &text.raster_out,
        sprite: Some(&sprite.sprraster[..VA_SURFACE_WIDTH]),
        graphic0: graphics.has_raster(0).then(|| graphics.raster_for(0)),
        graphic1: graphics.has_raster(1).then(|| graphics.raster_for(1)),
    };
    compose_raster(regs, palette_rgba, backdrop, &layers, y, row);
}

fn copy_font(font_rom_data: &[u8]) -> Box<[u8]> {
    let mut font_rom = vec![0u8; font_rom_data.len().max(0x5_0000)].into_boxed_slice();
    let copy_len = font_rom.len().min(font_rom_data.len());
    font_rom[..copy_len].copy_from_slice(&font_rom_data[..copy_len]);
    font_rom
}
