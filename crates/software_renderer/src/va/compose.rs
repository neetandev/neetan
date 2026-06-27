//! PC-88VA 4-layer compositor.
//!
//! Composes the text, sprite, and two graphic screens (palette-indexed) plus
//! the two direct-color (RGB) screens over the backdrop color, in priority
//! order. The sprite layer has no raster yet (phase 6) and is treated as absent.

use super::{VA_PIXEL_BYTES, VA_SURFACE_WIDTH, palette};

/// Number of palette-indexed composite screens.
const PALETTE_SCREENS: usize = 4;
/// Number of direct-color (RGB) composite screens.
const RGB_SCREENS: usize = 2;
/// Total composite screens in priority order.
const TOTAL_SCREENS: usize = PALETTE_SCREENS + RGB_SCREENS;

const LAYER_TEXT: u8 = 0;
const LAYER_SPRITE: u8 = 1;
const LAYER_GRAPHIC0: u8 = 2;
const LAYER_GRAPHIC1: u8 = 3;

const INSIDE: usize = 0;
const OUTSIDE: usize = 1;

/// Video-controller register values the compositor reads.
pub(super) struct ComposeRegs {
    pub color_composition: u16,
    pub rgb_composition: u16,
    pub palette_mode: u16,
    pub page_mask: u16,
    pub transparent_text_sprite: u16,
    pub transparent_graphic0: u16,
    pub transparent_graphic1: u16,
    pub graphics_mode: u16,
    pub graphics_resolution: u16,
    pub mask_mode: u16,
    pub mask_left: u16,
    pub mask_right: u16,
    pub mask_top: u16,
    pub mask_bottom: u16,
    pub palette_blink_counter: u16,
}

/// The per-scanline layer rasters fed to the compositor. The graphic rasters
/// are `None` when their screen produced no raster this line.
pub(super) struct RasterLayers<'a> {
    pub text: &'a [u8],
    pub sprite: Option<&'a [u8]>,
    pub graphic0: Option<&'a [u16]>,
    pub graphic1: Option<&'a [u16]>,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum ScreenKind {
    #[default]
    None,
    Text,
    Sprite,
    Graphic0Palette,
    Graphic1Palette,
    Graphic0Rgb,
    Graphic1Rgb,
}

/// A composite screen's per-raster sampling parameters.
#[derive(Clone, Copy, Default)]
struct CompositeScreen {
    kind: ScreenKind,
    mask: [bool; 2],
    pixel_mask: u16,
    palette_flip: u8,
    transparent: u32,
    pixel_mode: u8,
}

/// Composes one scanline of all layers over the backdrop into `out` (one row of
/// packed RGBA, `VA_SURFACE_WIDTH` pixels).
pub(super) fn compose_raster(
    regs: &ComposeRegs,
    palette_rgba: &[u32; 32],
    backdrop_rgba: u32,
    layers: &RasterLayers<'_>,
    y: usize,
    out: &mut [u8],
) {
    // Split the merged TSP raster into text and sprite layers by the
    // text/sprite boundary color.
    let boundary = regs.page_mask >> 12;
    let mut text_layer = [0u8; VA_SURFACE_WIDTH];
    let mut sprite_layer = [0u8; VA_SURFACE_WIDTH];
    for x in 0..VA_SURFACE_WIDTH {
        let mut code = layers.sprite.and_then(|r| r.get(x).copied()).unwrap_or(0);
        if code == 0 {
            code = layers.text.get(x).copied().unwrap_or(0);
        }
        if u16::from(code) > boundary {
            text_layer[x] = code;
        } else {
            sprite_layer[x] = code;
        }
    }

    let palette_field = (regs.palette_mode >> 6) & 3;
    let palette_set1_screen = ((regs.palette_mode >> 4) & 3) as u8;
    let mut default_flip = if palette_field == 1 { 0x10 } else { 0x00 };
    let blink_frequency = (regs.palette_mode >> 2) & 3;
    if palette_field < 2 && blink_frequency > 0 {
        const ON_TIME: [u16; 4] = [1, 2, 4, 6];
        const CYCLE_LENGTH: [u16; 4] = [0, 5, 6, 7];
        let shift = CYCLE_LENGTH[blink_frequency as usize] - 3;
        let current = (regs.palette_blink_counter >> shift) & 7;
        if current >= ON_TIME[(regs.palette_mode & 3) as usize] {
            default_flip ^= 0x10;
        }
    }

    let graphics_enabled = regs.graphics_mode & 0x8000 != 0;
    let mask_position = ((regs.mask_mode >> 4) & 3) as usize;
    let mut screens = [CompositeScreen::default(); TOTAL_SCREENS];

    // Palette-indexed screens, driven by colcomp.
    let mut composition = regs.color_composition;
    for (index, screen) in screens.iter_mut().enumerate().take(PALETTE_SCREENS) {
        let kind = (composition & 0x0F) as u8;
        composition >>= 4;
        if kind >= 8 {
            let layer = kind & 0x03;
            screen.palette_flip = default_flip;
            if palette_field == 2 && layer == palette_set1_screen {
                screen.palette_flip = 0x10;
            }
            screen.pixel_mask = 0x0F;
            match layer {
                LAYER_TEXT => {
                    screen.kind = ScreenKind::Text;
                    screen.transparent = txtspr_transparent(regs);
                }
                LAYER_SPRITE => {
                    screen.kind = ScreenKind::Sprite;
                    screen.transparent = txtspr_transparent(regs);
                }
                LAYER_GRAPHIC0 if graphics_enabled && layers.graphic0.is_some() => {
                    screen.kind = ScreenKind::Graphic0Palette;
                    configure_graphic_palette(
                        screen,
                        (regs.graphics_resolution & 0x0003) as u8,
                        regs.transparent_graphic0,
                        palette_field,
                    );
                }
                LAYER_GRAPHIC1 if graphics_enabled && layers.graphic1.is_some() => {
                    screen.kind = ScreenKind::Graphic1Palette;
                    configure_graphic_palette(
                        screen,
                        ((regs.graphics_resolution >> 8) & 0x0003) as u8,
                        regs.transparent_graphic1,
                        palette_field,
                    );
                }
                _ => {}
            }
        }
        assign_mask(screen, index, mask_position, regs.mask_mode);
    }

    // Direct-color (RGB) screens, driven by rgbcomp.
    let mut composition = regs.rgb_composition;
    for (index, screen) in screens
        .iter_mut()
        .enumerate()
        .skip(PALETTE_SCREENS)
        .take(RGB_SCREENS)
    {
        let kind = (composition & 0x0F) as u8;
        composition >>= 4;
        if (8..=9).contains(&kind) && graphics_enabled {
            let layer = kind & 0x01;
            if layer == 0 && layers.graphic0.is_some() {
                screen.kind = ScreenKind::Graphic0Rgb;
                screen.pixel_mode = (regs.graphics_resolution & 0x0003) as u8;
            } else if layer == 1 && layers.graphic1.is_some() {
                screen.kind = ScreenKind::Graphic1Rgb;
                screen.pixel_mode = ((regs.graphics_resolution >> 8) & 0x0003) as u8;
            }
        }
        assign_mask_rgb(screen, index, mask_position, regs.mask_mode);
    }

    let video_enabled = regs.graphics_mode & 0x3000 == 0x3000;
    for x in 0..VA_SURFACE_WIDTH {
        let color = if video_enabled {
            let side = if y < usize::from(regs.mask_top) * 2
                || y > usize::from(regs.mask_bottom) * 2 + 1
                || x < usize::from(regs.mask_left)
                || x > usize::from(regs.mask_right)
            {
                OUTSIDE
            } else {
                INSIDE
            };

            let mut resolved = backdrop_rgba;
            for screen in &screens {
                if screen.kind == ScreenKind::None || screen.mask[side] {
                    continue;
                }
                if let Some(color) =
                    sample_screen(screen, x, &text_layer, &sprite_layer, layers, palette_rgba)
                {
                    resolved = color;
                    break;
                }
            }
            resolved
        } else {
            palette::va_color_to_rgba(0)
        };

        let base = x * VA_PIXEL_BYTES;
        out[base] = color as u8;
        out[base + 1] = (color >> 8) as u8;
        out[base + 2] = (color >> 16) as u8;
        out[base + 3] = (color >> 24) as u8;
    }
}

fn txtspr_transparent(regs: &ComposeRegs) -> u32 {
    u32::from(regs.transparent_text_sprite) | (u32::from(regs.transparent_text_sprite) << 16)
}

fn configure_graphic_palette(
    screen: &mut CompositeScreen,
    pixel_mode: u8,
    transparent: u16,
    palette_field: u16,
) {
    let mut transparent_high = transparent;
    if palette_field == 3 && pixel_mode >= 2 {
        // 32-color mode: the full 5-bit index, no high-half transparency.
        screen.pixel_mask = 0x1F;
        transparent_high = 0x0000;
    } else {
        screen.pixel_mask = 0x0F;
    }
    screen.transparent = u32::from(transparent) | (u32::from(transparent_high) << 16);
}

fn assign_mask(screen: &mut CompositeScreen, index: usize, mask_position: usize, mask_mode: u16) {
    if index == mask_position + 1 {
        screen.mask[OUTSIDE] = mask_mode & 0x04 != 0;
        screen.mask[INSIDE] = mask_mode & 0x01 != 0;
    } else if index > mask_position {
        screen.mask[OUTSIDE] = mask_mode & 0x0C == 0x0C;
        screen.mask[INSIDE] = mask_mode & 0x03 == 0x03;
    } else {
        screen.mask[OUTSIDE] = mask_mode & 0x0C == 0x08;
        screen.mask[INSIDE] = mask_mode & 0x03 == 0x02;
    }
    if screen.kind == ScreenKind::None {
        screen.mask = [true, true];
    }
}

fn assign_mask_rgb(
    screen: &mut CompositeScreen,
    index: usize,
    mask_position: usize,
    mask_mode: u16,
) {
    if index == mask_position + 1 {
        screen.mask[OUTSIDE] = mask_mode & 0x04 != 0;
        screen.mask[INSIDE] = mask_mode & 0x01 != 0;
    } else {
        screen.mask[OUTSIDE] = mask_mode & 0x0C == 0x0C;
        screen.mask[INSIDE] = mask_mode & 0x03 == 0x03;
    }
    if screen.kind == ScreenKind::None {
        screen.mask = [true, true];
    }
}

fn sample_screen(
    screen: &CompositeScreen,
    x: usize,
    text_layer: &[u8],
    sprite_layer: &[u8],
    layers: &RasterLayers<'_>,
    palette_rgba: &[u32; 32],
) -> Option<u32> {
    match screen.kind {
        ScreenKind::Text => sample_palette(screen, u16::from(text_layer[x]), palette_rgba),
        ScreenKind::Sprite => sample_palette(screen, u16::from(sprite_layer[x]), palette_rgba),
        ScreenKind::Graphic0Palette => {
            let value = layers.graphic0?.get(x).copied().unwrap_or(0);
            sample_palette(screen, value, palette_rgba)
        }
        ScreenKind::Graphic1Palette => {
            let value = layers.graphic1?.get(x).copied().unwrap_or(0);
            sample_palette(screen, value, palette_rgba)
        }
        ScreenKind::Graphic0Rgb => {
            let value = layers.graphic0?.get(x).copied().unwrap_or(0);
            sample_rgb(screen.pixel_mode, value)
        }
        ScreenKind::Graphic1Rgb => {
            let value = layers.graphic1?.get(x).copied().unwrap_or(0);
            sample_rgb(screen.pixel_mode, value)
        }
        ScreenKind::None => None,
    }
}

fn sample_palette(screen: &CompositeScreen, value: u16, palette_rgba: &[u32; 32]) -> Option<u32> {
    let code = (value & screen.pixel_mask) ^ u16::from(screen.palette_flip);
    if screen.transparent & (1 << code) == 0 {
        Some(palette_rgba[usize::from(code) & 0x1F])
    } else {
        None
    }
}

fn sample_rgb(pixel_mode: u8, value: u16) -> Option<u32> {
    let color = match pixel_mode {
        2 => palette::rgb8_to_va_color(value as u8),
        3 => value,
        _ => 0,
    };
    if color != 0 {
        Some(palette::va_color_to_rgba(color))
    } else {
        None
    }
}
