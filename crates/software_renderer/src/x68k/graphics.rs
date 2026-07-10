//! X68000 graphic-screen rasterization.
//!
//! The video controller assembles the graphic screen from up to four ranked
//! GVRAM pages. Every displayed rank reads its physical page with that
//! page's own scroll registers; the palette codes are then resolved to a
//! color with the first-non-zero-plane rule in display order.

use super::{RenderInputsX68k, palette::graphic_color_65536};

/// Video-controller R0 bits selecting the graphic color mode.
const MEMORY_MODE_MASK: u16 = 0x0007;
/// Video-controller R2 bits enabling any 512-dot graphic plane.
const MIXING_GRAPHIC_512_ENABLES: u16 = 0x000F;
/// Video-controller R2 bit enabling the 1024-dot graphic screen.
const MIXING_GRAPHIC_1024_ENABLE: u16 = 0x0010;
/// Dot mask of one 512x512 graphic page.
const GRAPHIC_COORDINATE_MASK_512: usize = 0x01FF;
/// Dot mask of the 1024x1024 graphic screen.
const GRAPHIC_COORDINATE_MASK_1024: usize = 0x03FF;
/// Words covering one 512-dot GVRAM line.
const GRAPHIC_LINE_WORDS: usize = 512;

/// Graphic screen color mode selected by video-controller R0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GraphicMode {
    /// 512x512 16-color screen with four planes.
    Colors16,
    /// 512x512 256-color screen with two planes.
    Colors256,
    /// 512x512 65536-color screen with one plane.
    Colors65536,
    /// 1024x1024 16-color screen with one plane.
    Colors16Virtual1024,
}

/// Returns the graphic mode, folding undefined values onto real modes.
pub(super) const fn graphic_mode(memory_mode: u16) -> GraphicMode {
    match memory_mode & MEMORY_MODE_MASK {
        0 => GraphicMode::Colors16,
        1 => GraphicMode::Colors256,
        2 | 3 => GraphicMode::Colors65536,
        _ => GraphicMode::Colors16Virtual1024,
    }
}

/// Display-ordered graphic plane palette codes at one pixel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct GraphicCodes {
    /// Palette codes from front to back; disabled planes hold zero.
    pub codes: [u16; 4],
    /// Second-plane code fetched even when that plane is disabled.
    pub second_plane_code: u16,
    /// Number of display-ordered planes in the current mode.
    pub plane_count: usize,
    /// Whether any plane of the graphic screen is displayed.
    pub screen_enabled: bool,
}

/// Returns the graphic plane palette codes at one screen position.
pub(super) fn graphic_codes(
    inputs: &RenderInputsX68k,
    screen_x: usize,
    screen_y: usize,
) -> GraphicCodes {
    match graphic_mode(inputs.memory_mode) {
        GraphicMode::Colors16 => {
            let mut codes = [0_u16; 4];
            for (rank, code) in codes.iter_mut().enumerate() {
                if inputs.mixing & (1 << rank) != 0 {
                    *code = page_code(inputs, graphic_page(inputs, rank), screen_x, screen_y);
                }
            }
            GraphicCodes {
                codes,
                second_plane_code: page_code(inputs, graphic_page(inputs, 1), screen_x, screen_y),
                plane_count: 4,
                screen_enabled: inputs.mixing & MIXING_GRAPHIC_512_ENABLES != 0,
            }
        }
        GraphicMode::Colors256 => {
            let mut codes = [0_u16; 4];
            for (plane, code) in codes.iter_mut().take(2).enumerate() {
                if inputs.mixing & (0b11 << (plane * 2)) != 0 {
                    *code = pair_code(inputs, plane, screen_x, screen_y);
                }
            }
            GraphicCodes {
                codes,
                second_plane_code: pair_code(inputs, 1, screen_x, screen_y),
                plane_count: 2,
                screen_enabled: inputs.mixing & MIXING_GRAPHIC_512_ENABLES != 0,
            }
        }
        GraphicMode::Colors65536 => {
            let mut codes = [0_u16; 4];
            if inputs.mixing & MIXING_GRAPHIC_512_ENABLES != 0 {
                for rank in 0..4 {
                    codes[0] |= page_code(inputs, graphic_page(inputs, rank), screen_x, screen_y)
                        << (rank * 4);
                }
            }
            GraphicCodes {
                codes,
                second_plane_code: 0,
                plane_count: 1,
                screen_enabled: inputs.mixing & MIXING_GRAPHIC_512_ENABLES != 0,
            }
        }
        GraphicMode::Colors16Virtual1024 => {
            let mut codes = [0_u16; 4];
            if inputs.mixing & MIXING_GRAPHIC_1024_ENABLE != 0 {
                let source_x =
                    (screen_x + inputs.graphic_scroll_x[0] as usize) & GRAPHIC_COORDINATE_MASK_1024;
                let source_y =
                    (screen_y + inputs.graphic_scroll_y[0] as usize) & GRAPHIC_COORDINATE_MASK_1024;
                let quadrant = (source_y >> 9) << 1 | source_x >> 9;
                let page = graphic_page(inputs, quadrant);
                let word = inputs.graphic_vram[(source_y & GRAPHIC_COORDINATE_MASK_512)
                    * GRAPHIC_LINE_WORDS
                    + (source_x & GRAPHIC_COORDINATE_MASK_512)];
                codes[0] = (word >> (page * 4)) & 0x000F;
            }
            GraphicCodes {
                codes,
                second_plane_code: 0,
                plane_count: 1,
                screen_enabled: inputs.mixing & MIXING_GRAPHIC_1024_ENABLE != 0,
            }
        }
    }
}

/// Returns the first non-zero plane code in display order, or zero.
pub(super) fn first_graphic_code(graphic: &GraphicCodes) -> u16 {
    graphic.codes[..graphic.plane_count]
        .iter()
        .copied()
        .find(|&code| code != 0)
        .unwrap_or(0)
}

/// Resolves one palette code to a color in the current graphic mode.
pub(super) fn graphic_code_color(inputs: &RenderInputsX68k, code: u16) -> u16 {
    match graphic_mode(inputs.memory_mode) {
        GraphicMode::Colors16 | GraphicMode::Colors256 | GraphicMode::Colors16Virtual1024 => {
            inputs.graphics_palette[usize::from(code) & 0xFF]
        }
        GraphicMode::Colors65536 => graphic_color_65536(inputs.graphics_palette, code),
    }
}

/// Returns the graphic screen color under the first-non-zero-plane rule.
pub(super) fn graphic_color(inputs: &RenderInputsX68k, graphic: &GraphicCodes) -> u16 {
    if !graphic.screen_enabled {
        return 0;
    }
    graphic_code_color(inputs, first_graphic_code(graphic))
}

/// Returns the physical GVRAM page displayed at one rank.
const fn graphic_page(inputs: &RenderInputsX68k, rank: usize) -> usize {
    ((inputs.priority >> ((rank & 3) * 2)) & 3) as usize
}

/// Returns one 256-color plane's 8-bit code from its two ranked pages.
fn pair_code(inputs: &RenderInputsX68k, plane: usize, screen_x: usize, screen_y: usize) -> u16 {
    let low = page_code(inputs, graphic_page(inputs, plane * 2), screen_x, screen_y);
    let high = page_code(
        inputs,
        graphic_page(inputs, plane * 2 + 1),
        screen_x,
        screen_y,
    );
    high << 4 | low
}

/// Returns one page's 4-bit code at a screen position with its own scroll.
fn page_code(inputs: &RenderInputsX68k, page: usize, screen_x: usize, screen_y: usize) -> u16 {
    let source_x =
        (screen_x + inputs.graphic_scroll_x[page] as usize) & GRAPHIC_COORDINATE_MASK_512;
    let source_y =
        (screen_y + inputs.graphic_scroll_y[page] as usize) & GRAPHIC_COORDINATE_MASK_512;
    let word = inputs.graphic_vram[source_y * GRAPHIC_LINE_WORDS + source_x];
    (word >> (page * 4)) & 0x000F
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::x68k::FixtureX68k;

    #[test]
    fn sixteen_color_planes_follow_the_page_ranks() {
        let mut fixture = FixtureX68k::new(8, 1);
        fixture.video_registers[2] = 0x000F;
        fixture.set_graphic_word(0, 0, 0x4321);
        let graphic = graphic_codes(&fixture.inputs(), 0, 0);
        assert_eq!(graphic.codes, [1, 2, 3, 4]);
        assert_eq!(graphic.plane_count, 4);
        assert!(graphic.screen_enabled);
        fixture.video_registers[1] = 0x12E4 & 0xFF00 | 0x001B;
        let graphic = graphic_codes(&fixture.inputs(), 0, 0);
        assert_eq!(graphic.codes, [4, 3, 2, 1]);
    }

    #[test]
    fn each_page_scrolls_with_its_own_registers() {
        let mut fixture = FixtureX68k::new(8, 2);
        fixture.video_registers[2] = 0x000F;
        fixture.set_graphic_word(0, 0, 0x0001);
        fixture.set_graphic_word(1, 0, 0x0020);
        fixture.set_graphic_word(0, 1, 0x0300);
        fixture.graphic_scroll[1] = (1, 0);
        fixture.graphic_scroll[2] = (0, 1);
        let graphic = graphic_codes(&fixture.inputs(), 0, 0);
        assert_eq!(graphic.codes, [1, 2, 3, 0]);
    }

    #[test]
    fn scroll_wraps_within_one_page() {
        let mut fixture = FixtureX68k::new(8, 1);
        fixture.video_registers[2] = 0x0001;
        fixture.set_graphic_word(1, 1, 0x0007);
        fixture.graphic_scroll[0] = (513, 513);
        let graphic = graphic_codes(&fixture.inputs(), 0, 0);
        assert_eq!(graphic.codes[0], 7);
    }

    #[test]
    fn disabled_planes_contribute_zero_codes() {
        let mut fixture = FixtureX68k::new(8, 1);
        fixture.video_registers[2] = 0x000A;
        fixture.set_graphic_word(0, 0, 0x4321);
        let graphic = graphic_codes(&fixture.inputs(), 0, 0);
        assert_eq!(graphic.codes, [0, 2, 0, 4]);
        fixture.video_registers[2] = 0;
        let graphic = graphic_codes(&fixture.inputs(), 0, 0);
        assert!(!graphic.screen_enabled);
    }

    #[test]
    fn two_hundred_fifty_six_color_planes_pair_ranked_nibbles() {
        let mut fixture = FixtureX68k::new(8, 1);
        fixture.video_registers[0] = 1;
        fixture.video_registers[2] = 0x000F;
        fixture.set_graphic_word(0, 0, 0x4321);
        let graphic = graphic_codes(&fixture.inputs(), 0, 0);
        assert_eq!(graphic.codes, [0x21, 0x43, 0, 0]);
        assert_eq!(graphic.plane_count, 2);
        fixture.video_registers[2] = 0x0002;
        let graphic = graphic_codes(&fixture.inputs(), 0, 0);
        assert_eq!(graphic.codes, [0x21, 0, 0, 0]);
        fixture.video_registers[2] = 0x0004;
        let graphic = graphic_codes(&fixture.inputs(), 0, 0);
        assert_eq!(graphic.codes, [0, 0x43, 0, 0]);
    }

    #[test]
    fn two_hundred_fifty_six_color_nibbles_scroll_independently() {
        let mut fixture = FixtureX68k::new(8, 1);
        fixture.video_registers[0] = 1;
        fixture.video_registers[2] = 0x0003;
        fixture.set_graphic_word(0, 0, 0x0001);
        fixture.set_graphic_word(1, 0, 0x0050);
        fixture.graphic_scroll[1] = (1, 0);
        let graphic = graphic_codes(&fixture.inputs(), 0, 0);
        assert_eq!(graphic.codes[0], 0x51);
    }

    #[test]
    fn full_color_mode_assembles_one_word_code() {
        let mut fixture = FixtureX68k::new(8, 1);
        fixture.video_registers[0] = 3;
        fixture.video_registers[2] = 0x0008;
        fixture.set_graphic_word(0, 0, 0x4321);
        let graphic = graphic_codes(&fixture.inputs(), 0, 0);
        assert_eq!(graphic.codes, [0x4321, 0, 0, 0]);
        assert_eq!(graphic.plane_count, 1);
        assert!(graphic.screen_enabled);
    }

    #[test]
    fn virtual_1024_mode_reads_quadrant_nibbles() {
        let mut fixture = FixtureX68k::new(8, 1);
        fixture.video_registers[0] = 4;
        fixture.video_registers[2] = 0x0010;
        fixture.set_graphic_word(0, 0, 0x4321);
        let expected = [
            (0, 0, 1),
            (512, 0, 2),
            (0, 512, 3),
            (512, 512, 4),
            (1024, 1024, 1),
        ];
        for (scroll_x, scroll_y, code) in expected {
            fixture.graphic_scroll[0] = (scroll_x, scroll_y);
            let graphic = graphic_codes(&fixture.inputs(), 0, 0);
            assert_eq!(graphic.codes[0], code);
        }
        fixture.video_registers[2] = 0x000F;
        let graphic = graphic_codes(&fixture.inputs(), 0, 0);
        assert!(!graphic.screen_enabled);
    }

    #[test]
    fn graphic_color_takes_the_first_non_zero_plane() {
        let mut fixture = FixtureX68k::new(8, 1);
        fixture.video_registers[2] = 0x000F;
        fixture.graphics_palette[2] = 0x0FF0;
        fixture.set_graphic_word(0, 0, 0x0020);
        let graphic = graphic_codes(&fixture.inputs(), 0, 0);
        assert_eq!(graphic_color(&fixture.inputs(), &graphic), 0x0FF0);
    }

    #[test]
    fn zero_color_front_plane_punches_a_hole() {
        let mut fixture = FixtureX68k::new(8, 1);
        fixture.video_registers[2] = 0x000F;
        fixture.graphics_palette[1] = 0;
        fixture.graphics_palette[2] = 0x0FF0;
        fixture.set_graphic_word(0, 0, 0x0021);
        let graphic = graphic_codes(&fixture.inputs(), 0, 0);
        assert_eq!(graphic_color(&fixture.inputs(), &graphic), 0);
    }

    #[test]
    fn all_zero_codes_fall_back_to_palette_zero() {
        let mut fixture = FixtureX68k::new(8, 1);
        fixture.video_registers[2] = 0x0001;
        fixture.graphics_palette[0] = 0x1234;
        let graphic = graphic_codes(&fixture.inputs(), 0, 0);
        assert_eq!(graphic_color(&fixture.inputs(), &graphic), 0x1234);
        fixture.video_registers[2] = 0;
        let graphic = graphic_codes(&fixture.inputs(), 0, 0);
        assert_eq!(graphic_color(&fixture.inputs(), &graphic), 0);
    }
}
