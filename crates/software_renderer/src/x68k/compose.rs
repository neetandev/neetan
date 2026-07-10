//! X68000 layer composition under the video-controller priority rules.
//!
//! The sprite and text screens are first reduced to one representative
//! screen: the front one whose 4-bit palette code is non-zero wins, then
//! the back one, then a sprite with a non-zero palette block, then the
//! back screen. The representative and the graphic screen are then ordered
//! by the graphic priority and the front color wins unless it is zero.
//! Priority 3 shows the graphic screen in front with sprite and text off.
//!
//! The extension bits change how the graphic screen resolves: special
//! priority jumps selected pixels to the very front, translucency blends
//! the front graphic color with the second graphic plane or with the
//! sprite/text color behind the graphic screen, and brightness modulation
//! blends the whole graphic screen with text palette 0.

use super::{
    RenderInputsX68k,
    graphics::{
        GraphicCodes, GraphicMode, first_graphic_code, graphic_code_color, graphic_color,
        graphic_mode,
    },
    palette::mix_averaged_grbi,
    text::text_layer_visible,
};

/// Video-controller R2 bit that enables the sprite screen.
const MIXING_SPRITE_ENABLE: u16 = 0x0040;
/// Video-controller R2 bit that blends graphics with text palette 0.
const MIXING_BRIGHTNESS_MODULATION: u16 = 0x4000;
/// Video-controller R2 bit that enables the extension modes.
const MIXING_EXTENSION_ENABLE: u16 = 0x1000;
/// Video-controller R2 bit selecting translucency over special priority.
const MIXING_HALF_SELECT: u16 = 0x0800;
/// Video-controller R2 bit selecting the region by palette code.
const MIXING_PALETTE_SELECT: u16 = 0x0400;
/// Video-controller R2 bit blending the first with the second graphic plane.
const MIXING_GRAPHIC_TARGET: u16 = 0x0200;
/// Video-controller R2 bit blending graphics with sprite and text behind.
const MIXING_SPRITE_TEXT_TARGET: u16 = 0x0100;
/// Shift of the sprite screen priority in video-controller R1.
const SPRITE_PRIORITY_SHIFT: u32 = 12;
/// Shift of the text screen priority in video-controller R1.
const TEXT_PRIORITY_SHIFT: u32 = 10;
/// Shift of the graphic screen priority in video-controller R1.
const GRAPHIC_PRIORITY_SHIFT: u32 = 8;
/// Graphic priority that blanks the sprite and text screens.
const GRAPHIC_PRIORITY_FRONTMOST: u16 = 3;

/// Graphic screen output of the extension evaluation.
enum GraphicResult {
    /// Displayed in front of everything regardless of priority.
    FrontJump(u16),
    /// Final at the graphic layer: an opaque front still wins, zero is black.
    Opaque(u16),
    /// Ordinary graphic color: zero is transparent.
    Normal(u16),
}

/// Composes one pixel into a GRBI color under the priority rules.
pub(super) fn compose_pixel(
    inputs: &RenderInputsX68k,
    sprite_code: u8,
    text_code: u8,
    graphic: &GraphicCodes,
) -> u16 {
    if inputs.mixing & MIXING_BRIGHTNESS_MODULATION != 0 {
        if !graphic.screen_enabled {
            return 0;
        }
        return mix_averaged_grbi(graphic_color(inputs, graphic), inputs.text_palette[0]);
    }
    let graphic_priority = (inputs.priority >> GRAPHIC_PRIORITY_SHIFT) & 3;
    let graphic_frontmost = graphic_priority == GRAPHIC_PRIORITY_FRONTMOST;
    let sprite_screen_on = inputs.mixing & MIXING_SPRITE_ENABLE != 0 && !graphic_frontmost;
    let text_screen_on = text_layer_visible(inputs) && !graphic_frontmost;
    if !sprite_screen_on && !text_screen_on && !graphic.screen_enabled {
        return 0;
    }
    let sprite_code = if sprite_screen_on { sprite_code } else { 0 };
    let text_code = if text_screen_on { text_code } else { 0 };
    let sprite_in_front = ((inputs.priority >> SPRITE_PRIORITY_SHIFT) & 3)
        < ((inputs.priority >> TEXT_PRIORITY_SHIFT) & 3);
    let (front_code, back_code) = if sprite_in_front {
        (sprite_code, text_code)
    } else {
        (text_code, sprite_code)
    };
    let (representative_code, representative_in_front) = if front_code & 0x0F != 0 {
        (front_code, true)
    } else if back_code & 0x0F != 0 {
        (back_code, false)
    } else if sprite_code != 0 {
        (sprite_code, sprite_in_front)
    } else {
        (back_code, false)
    };
    let representative_color = inputs.text_palette[usize::from(representative_code)];
    let graphic_in_front = match graphic_priority {
        1 => !representative_in_front,
        2 => false,
        _ => true,
    };
    let sprite_text_partner = if graphic_in_front {
        representative_color
    } else {
        0
    };
    match graphic_result(inputs, graphic, sprite_text_partner) {
        GraphicResult::FrontJump(color) => color,
        GraphicResult::Opaque(color) => {
            if graphic_in_front || representative_color == 0 {
                color
            } else {
                representative_color
            }
        }
        GraphicResult::Normal(color) => {
            if graphic_in_front {
                if color != 0 {
                    color
                } else {
                    representative_color
                }
            } else if representative_color != 0 {
                representative_color
            } else {
                color
            }
        }
    }
}

/// Evaluates the graphic screen under the selected extension mode.
fn graphic_result(
    inputs: &RenderInputsX68k,
    graphic: &GraphicCodes,
    sprite_text_partner: u16,
) -> GraphicResult {
    if !graphic.screen_enabled {
        return GraphicResult::Normal(0);
    }
    if inputs.mixing & MIXING_EXTENSION_ENABLE == 0 {
        return GraphicResult::Normal(graphic_color(inputs, graphic));
    }
    let multi_plane = matches!(
        graphic_mode(inputs.memory_mode),
        GraphicMode::Colors16 | GraphicMode::Colors256
    );
    let graphic_target = inputs.mixing & MIXING_GRAPHIC_TARGET != 0;
    let sprite_text_target = inputs.mixing & MIXING_SPRITE_TEXT_TARGET != 0;
    let palette_selected = inputs.mixing & MIXING_PALETTE_SELECT != 0;
    if inputs.mixing & MIXING_HALF_SELECT == 0 {
        if palette_selected {
            special_priority_by_palette(inputs, graphic)
        } else {
            special_priority_by_color(inputs, graphic)
        }
    } else if !sprite_text_target && !graphic_target || graphic_target && !multi_plane {
        GraphicResult::Normal(graphic_color(inputs, graphic))
    } else if palette_selected {
        translucent_by_palette(
            inputs,
            graphic,
            graphic_target,
            sprite_text_target,
            sprite_text_partner,
        )
    } else {
        translucent_by_color(
            inputs,
            graphic,
            graphic_target,
            sprite_text_target,
            sprite_text_partner,
        )
    }
}

/// Returns the region-selection code: evened, or odded in 65536 colors.
fn region_code(inputs: &RenderInputsX68k, code: u16) -> u16 {
    if graphic_mode(inputs.memory_mode) == GraphicMode::Colors65536 {
        code | 1
    } else {
        code & !1
    }
}

/// Special priority selected by the region color being odd.
fn special_priority_by_color(inputs: &RenderInputsX68k, graphic: &GraphicCodes) -> GraphicResult {
    let code = first_graphic_code(graphic);
    if graphic_code_color(inputs, region_code(inputs, code)) & 1 != 0 {
        GraphicResult::FrontJump(graphic_code_color(inputs, code))
    } else {
        GraphicResult::Normal(graphic_code_color(inputs, code))
    }
}

/// Special priority selected by the first plane's palette code being odd.
fn special_priority_by_palette(inputs: &RenderInputsX68k, graphic: &GraphicCodes) -> GraphicResult {
    let first_code = graphic.codes[0];
    if first_code >= 2 && first_code & 1 != 0 {
        GraphicResult::FrontJump(graphic_code_color(inputs, first_code & !1))
    } else {
        GraphicResult::Normal(palette_selected_base_color(inputs, graphic, first_code))
    }
}

/// Translucency selected by the region color being odd.
fn translucent_by_color(
    inputs: &RenderInputsX68k,
    graphic: &GraphicCodes,
    graphic_target: bool,
    sprite_text_target: bool,
    sprite_text_partner: u16,
) -> GraphicResult {
    let code = first_graphic_code(graphic);
    let region_is_odd = graphic_code_color(inputs, region_code(inputs, code)) & 1 != 0;
    if !graphic_target {
        return if region_is_odd {
            GraphicResult::Opaque(mix_averaged_grbi(
                graphic_code_color(inputs, code),
                sprite_text_partner,
            ))
        } else {
            GraphicResult::Normal(graphic_code_color(inputs, code))
        };
    }
    let partner_color = graphic_code_color(inputs, graphic.second_plane_code | 1);
    if region_is_odd {
        let mixed = mix_averaged_grbi(graphic_code_color(inputs, code & !1), partner_color);
        if sprite_text_target {
            GraphicResult::Opaque(mix_averaged_grbi(mixed, sprite_text_partner))
        } else {
            GraphicResult::Opaque(mixed)
        }
    } else if code & 1 != 0 {
        GraphicResult::Normal(partner_color)
    } else {
        GraphicResult::Normal(graphic_code_color(inputs, code))
    }
}

/// Translucency selected by the first plane's palette code being odd.
fn translucent_by_palette(
    inputs: &RenderInputsX68k,
    graphic: &GraphicCodes,
    graphic_target: bool,
    sprite_text_target: bool,
    sprite_text_partner: u16,
) -> GraphicResult {
    let first_code = graphic.codes[0];
    if first_code < 2 || first_code & 1 == 0 {
        return GraphicResult::Normal(palette_selected_base_color(inputs, graphic, first_code));
    }
    let evened_color = graphic_code_color(inputs, first_code & !1);
    if !graphic_target {
        return GraphicResult::Opaque(mix_averaged_grbi(evened_color, sprite_text_partner));
    }
    let partner_color = graphic_code_color(inputs, graphic.second_plane_code | 1);
    let mixed = mix_averaged_grbi(evened_color, partner_color);
    if sprite_text_target {
        GraphicResult::Opaque(mix_averaged_grbi(mixed, sprite_text_partner))
    } else {
        GraphicResult::Normal(mixed)
    }
}

/// Returns the unblended color of the palette-selected modes.
///
/// A first-plane code of zero shows the deeper planes through evened
/// palette codes; a code of one is evened to zero and shows palette 0.
fn palette_selected_base_color(
    inputs: &RenderInputsX68k,
    graphic: &GraphicCodes,
    first_code: u16,
) -> u16 {
    if first_code >= 2 {
        return graphic_code_color(inputs, first_code);
    }
    if first_code == 1 {
        return graphic_code_color(inputs, 0);
    }
    let deeper_code = graphic.codes[1..graphic.plane_count]
        .iter()
        .map(|&code| code & !1)
        .find(|&code| code != 0)
        .unwrap_or(0);
    graphic_code_color(inputs, deeper_code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::x68k::{FixtureX68k, graphics::graphic_codes};

    /// GRBI color of graphics palette entry 1 in the fixtures.
    const GRAPHIC_COLOR: u16 = 0x003E;
    /// GRBI color of text palette entry 1 in the fixtures.
    const TEXT_COLOR: u16 = 0x07C0;
    /// GRBI color of text palette entry 0x32 in the fixtures.
    const SPRITE_COLOR: u16 = 0xF800;
    /// GRBI color of text palette entry 0 in the fixtures.
    const BACKDROP_COLOR: u16 = 0x0842;

    /// Builds one priority register value from priorities and page ranks.
    const fn priority(sprite: u16, text: u16, graphic: u16) -> u16 {
        sprite << 12 | text << 10 | graphic << 8 | 0x00E4
    }

    fn priority_fixture(
        sprite_priority: u16,
        text_priority: u16,
        graphic_priority: u16,
    ) -> FixtureX68k {
        let mut fixture = FixtureX68k::new(8, 1);
        fixture.video_registers[1] = priority(sprite_priority, text_priority, graphic_priority);
        fixture.video_registers[2] = 0x0061;
        fixture.graphics_palette[1] = GRAPHIC_COLOR;
        fixture.text_palette[1] = TEXT_COLOR;
        fixture.text_palette[0x32] = SPRITE_COLOR;
        fixture.set_graphic_word(0, 0, 0x0001);
        fixture
    }

    fn compose(fixture: &FixtureX68k, sprite_code: u8, text_code: u8) -> u16 {
        let inputs = fixture.inputs();
        let graphic = graphic_codes(&inputs, 0, 0);
        compose_pixel(&inputs, sprite_code, text_code, &graphic)
    }

    #[test]
    fn text_beats_sprite_on_equal_priority() {
        let fixture = priority_fixture(0, 0, 2);
        assert_eq!(compose(&fixture, 0x32, 1), TEXT_COLOR);
        let fixture = priority_fixture(0, 1, 2);
        assert_eq!(compose(&fixture, 0x32, 1), SPRITE_COLOR);
        let fixture = priority_fixture(1, 0, 2);
        assert_eq!(compose(&fixture, 0x32, 1), TEXT_COLOR);
    }

    #[test]
    fn graphic_priority_zero_puts_the_graphic_screen_in_front() {
        let fixture = priority_fixture(1, 0, 0);
        assert_eq!(compose(&fixture, 0x32, 1), GRAPHIC_COLOR);
    }

    #[test]
    fn graphic_priority_one_sits_between_sprite_and_text() {
        let mut fixture = priority_fixture(0, 1, 1);
        assert_eq!(compose(&fixture, 0x32, 1), SPRITE_COLOR);
        assert_eq!(compose(&fixture, 0x00, 1), GRAPHIC_COLOR);
        fixture.set_graphic_word(0, 0, 0);
        assert_eq!(compose(&fixture, 0x00, 1), TEXT_COLOR);
    }

    #[test]
    fn graphic_priority_two_puts_the_graphic_screen_behind() {
        let fixture = priority_fixture(1, 0, 2);
        assert_eq!(compose(&fixture, 0, 1), TEXT_COLOR);
        assert_eq!(compose(&fixture, 0x32, 0), SPRITE_COLOR);
        assert_eq!(compose(&fixture, 0, 0), GRAPHIC_COLOR);
    }

    #[test]
    fn graphic_priority_three_blanks_sprite_and_text() {
        let mut fixture = priority_fixture(1, 0, 3);
        fixture.text_palette[0] = BACKDROP_COLOR;
        assert_eq!(compose(&fixture, 0x32, 1), GRAPHIC_COLOR);
        fixture.set_graphic_word(0, 0, 0);
        fixture.graphics_palette[0] = 0;
        assert_eq!(compose(&fixture, 0x32, 1), BACKDROP_COLOR);
        fixture.video_registers[2] = 0x0060;
        assert_eq!(compose(&fixture, 0x32, 1), 0);
    }

    #[test]
    fn representative_rules_follow_the_palette_block() {
        let mut fixture = priority_fixture(0, 1, 2);
        assert_eq!(compose(&fixture, 0x30, 1), TEXT_COLOR);
        fixture.text_palette[0x30] = 0x1F00;
        assert_eq!(compose(&fixture, 0x30, 0), 0x1F00);
        let mut fixture = priority_fixture(1, 0, 2);
        fixture.text_palette[0x30] = 0x1F00;
        assert_eq!(compose(&fixture, 0x30, 0), 0x1F00);
    }

    #[test]
    fn transparent_screens_fall_through_by_color() {
        let mut fixture = priority_fixture(1, 0, 2);
        fixture.text_palette[5] = 0;
        assert_eq!(compose(&fixture, 0, 5), GRAPHIC_COLOR);
        fixture.set_graphic_word(0, 0, 0);
        fixture.graphics_palette[0] = 0x1234;
        assert_eq!(compose(&fixture, 0, 5), 0x1234);
    }

    #[test]
    fn opaque_palette_zero_screens_occlude_the_layers_behind() {
        let mut fixture = priority_fixture(1, 0, 2);
        fixture.text_palette[0] = BACKDROP_COLOR;
        assert_eq!(compose(&fixture, 0, 0), BACKDROP_COLOR);
        fixture.video_registers[1] = priority(1, 0, 1);
        assert_eq!(compose(&fixture, 0, 0), GRAPHIC_COLOR);
    }

    #[test]
    fn disabled_screens_are_treated_as_code_zero() {
        let mut fixture = priority_fixture(1, 0, 2);
        fixture.video_registers[2] = 0x0041;
        assert_eq!(compose(&fixture, 0, 1), GRAPHIC_COLOR);
        fixture.video_registers[2] = 0x0021;
        assert_eq!(compose(&fixture, 0x32, 0), GRAPHIC_COLOR);
        fixture.video_registers[2] = 0x0060;
        fixture.text_palette[0] = BACKDROP_COLOR;
        assert_eq!(compose(&fixture, 0, 0), BACKDROP_COLOR);
    }

    #[test]
    fn everything_disabled_renders_black() {
        let mut fixture = priority_fixture(1, 0, 2);
        fixture.video_registers[2] = 0;
        fixture.text_palette[0] = BACKDROP_COLOR;
        assert_eq!(compose(&fixture, 0, 0), 0);
    }

    /// GRBI color used as the even red entry of the reference table.
    const RED_EVEN: u16 = 0x07C0;
    /// GRBI color used as the odd red entry of the reference table.
    const RED_ODD: u16 = 0x07C1;
    /// GRBI color used as the blue entry of the reference table.
    const BLUE: u16 = 0x003E;
    /// GRBI color used as the purple entry of the reference table.
    const PURPLE: u16 = 0x07FE;
    /// GRBI color used as the cyan entry of the reference table.
    const CYAN: u16 = 0xF83E;
    /// Averaged color of odd red and blue.
    const PURPLE_MIX: u16 = 0x03DE;
    /// Averaged color of odd red and cyan.
    const GRAY_MIX: u16 = 0x7BDE;

    #[test]
    fn special_priority_by_color_jumps_selected_pixels_to_the_front() {
        let mut fixture = priority_fixture(1, 0, 2);
        fixture.video_registers[2] = 0x1021;
        fixture.graphics_palette[0] = 0x0001;
        assert_eq!(compose(&fixture, 0, 1), GRAPHIC_COLOR);
        fixture.graphics_palette[0] = 0x0002;
        assert_eq!(compose(&fixture, 0, 1), TEXT_COLOR);
        assert_eq!(compose(&fixture, 0, 0), GRAPHIC_COLOR);
    }

    #[test]
    fn special_priority_by_palette_selects_on_the_first_plane_code() {
        let mut fixture = priority_fixture(1, 0, 2);
        fixture.video_registers[2] = 0x143F;
        fixture.graphics_palette[0] = 0x1234;
        fixture.graphics_palette[2] = RED_EVEN;
        fixture.set_graphic_word(0, 0, 0x0003);
        assert_eq!(compose(&fixture, 0, 1), RED_EVEN);
        fixture.set_graphic_word(0, 0, 0x0002);
        assert_eq!(compose(&fixture, 0, 1), TEXT_COLOR);
        assert_eq!(compose(&fixture, 0, 0), RED_EVEN);
        fixture.set_graphic_word(0, 0, 0x0001);
        assert_eq!(compose(&fixture, 0, 0), 0x1234);
        fixture.set_graphic_word(0, 0, 0x3110);
        assert_eq!(compose(&fixture, 0, 0), RED_EVEN);
        fixture.set_graphic_word(0, 0, 0x1110);
        assert_eq!(compose(&fixture, 0, 0), 0x1234);
    }

    #[test]
    fn translucency_with_sprite_text_mixes_the_screen_behind() {
        let mut fixture = priority_fixture(1, 0, 0);
        fixture.video_registers[2] = 0x1921;
        fixture.text_palette[1] = 0xF800;
        fixture.graphics_palette[0] = 0x0001;
        fixture.graphics_palette[1] = RED_EVEN;
        assert_eq!(compose(&fixture, 0, 1), 0x7BC0);
        assert_eq!(compose(&fixture, 0, 0), 0x03C0);
        fixture.video_registers[1] = priority(1, 0, 2);
        assert_eq!(compose(&fixture, 0, 1), 0xF800);
        assert_eq!(compose(&fixture, 0, 0), 0x03C0);
        fixture.video_registers[1] = priority(1, 0, 0);
        fixture.graphics_palette[0] = 0x0002;
        assert_eq!(compose(&fixture, 0, 1), RED_EVEN);
    }

    #[test]
    fn translucency_with_graphics_matches_the_reference_table() {
        let mut fixture = FixtureX68k::new(8, 1);
        fixture.video_registers[2] = 0x1A03;
        fixture.graphics_palette[3] = BLUE;
        fixture.graphics_palette[4] = PURPLE;
        fixture.graphics_palette[5] = CYAN;
        fixture.graphics_palette[2] = RED_EVEN;
        let even_rows = [
            (2, 2, RED_EVEN),
            (2, 3, RED_EVEN),
            (2, 4, RED_EVEN),
            (2, 5, RED_EVEN),
            (3, 2, BLUE),
            (3, 3, BLUE),
            (3, 4, CYAN),
            (3, 5, CYAN),
        ];
        for (first, second, expected) in even_rows {
            fixture.set_graphic_word(0, 0, second << 4 | first);
            assert_eq!(compose(&fixture, 0, 0), expected, "even {first} {second}");
        }
        fixture.graphics_palette[2] = RED_ODD;
        let odd_rows = [
            (2, 2, PURPLE_MIX),
            (2, 3, PURPLE_MIX),
            (2, 4, GRAY_MIX),
            (2, 5, GRAY_MIX),
            (3, 2, PURPLE_MIX),
            (3, 3, PURPLE_MIX),
            (3, 4, GRAY_MIX),
            (3, 5, GRAY_MIX),
        ];
        for (first, second, expected) in odd_rows {
            fixture.set_graphic_word(0, 0, second << 4 | first);
            assert_eq!(compose(&fixture, 0, 0), expected, "odd {first} {second}");
        }
    }

    #[test]
    fn translucency_second_plane_contributes_even_when_disabled() {
        let mut fixture = FixtureX68k::new(8, 1);
        fixture.video_registers[2] = 0x1A01;
        fixture.graphics_palette[2] = RED_ODD;
        fixture.graphics_palette[5] = CYAN;
        fixture.set_graphic_word(0, 0, 0x0043);
        assert_eq!(compose(&fixture, 0, 0), GRAY_MIX);
    }

    #[test]
    fn translucency_with_graphics_and_sprite_text_mixes_twice() {
        let mut fixture = priority_fixture(1, 0, 0);
        fixture.video_registers[2] = 0x1B23;
        fixture.text_palette[1] = 0xF800;
        fixture.graphics_palette[2] = RED_ODD;
        fixture.graphics_palette[3] = BLUE;
        fixture.set_graphic_word(0, 0, 0x0023);
        assert_eq!(compose(&fixture, 0, 1), 0x79CE);
    }

    #[test]
    fn palette_translucency_with_sprite_text_selects_on_the_first_code() {
        let mut fixture = priority_fixture(1, 0, 0);
        fixture.video_registers[2] = 0x1D23;
        fixture.text_palette[1] = 0xF800;
        fixture.graphics_palette[0] = 0x0842;
        fixture.graphics_palette[2] = RED_EVEN;
        fixture.set_graphic_word(0, 0, 0x0003);
        assert_eq!(compose(&fixture, 0, 1), 0x7BC0);
        fixture.set_graphic_word(0, 0, 0x0002);
        assert_eq!(compose(&fixture, 0, 1), RED_EVEN);
        fixture.set_graphic_word(0, 0, 0x0001);
        assert_eq!(compose(&fixture, 0, 1), 0x0842);
        fixture.set_graphic_word(0, 0, 0x0030);
        assert_eq!(compose(&fixture, 0, 1), RED_EVEN);
    }

    #[test]
    fn palette_translucency_with_graphics_falls_back_when_transparent() {
        let mut fixture = priority_fixture(1, 0, 0);
        fixture.video_registers[2] = 0x1E23;
        fixture.text_palette[1] = 0xF800;
        fixture.graphics_palette[1] = 0;
        fixture.set_graphic_word(0, 0, 0x0003);
        assert_eq!(compose(&fixture, 0, 1), 0xF800);
        fixture.video_registers[2] = 0x1F23;
        assert_eq!(compose(&fixture, 0, 1), 0x7800);
    }

    #[test]
    fn brightness_modulation_blends_graphics_with_text_palette_zero() {
        let mut fixture = priority_fixture(1, 0, 2);
        fixture.video_registers[2] = 0x4021;
        fixture.text_palette[0] = 0x0842;
        fixture.text_palette[1] = 0xF800;
        fixture.graphics_palette[1] = RED_EVEN;
        assert_eq!(compose(&fixture, 0, 1), 0x0400);
        fixture.video_registers[2] = 0x4020;
        assert_eq!(compose(&fixture, 0, 1), 0);
    }

    #[test]
    fn full_color_extensions_use_odd_region_codes() {
        let mut fixture = priority_fixture(1, 0, 2);
        fixture.video_registers[0] = 3;
        fixture.video_registers[2] = 0x102F;
        fixture.graphics_palette[1] = 0x2200;
        fixture.graphics_palette[2] = 0x0001;
        fixture.graphic_vram[0] = 0x0002;
        assert_eq!(compose(&fixture, 0, 1), 0x2200);
        fixture.video_registers[2] = 0x1A2F;
        assert_eq!(compose(&fixture, 0, 1), TEXT_COLOR);
    }
}
