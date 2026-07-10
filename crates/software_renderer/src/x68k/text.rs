//! X68000 text-plane rasterization.

use super::RenderInputsX68k;

/// Byte size of one text VRAM plane.
const TEXT_PLANE_BYTES: usize = 0x2_0000;
/// Bytes covering one 1024-dot text raster line.
const TEXT_LINE_BYTES: usize = 128;
/// Dot mask of the 1024x1024 text coordinate space.
const TEXT_COORDINATE_MASK: usize = 0x03FF;
/// Video-controller mixing bit that enables the text layer.
const MIXING_TEXT_ENABLE: u16 = 0x0020;
/// CRTC memory-mode bit that switches text VRAM to storage mode.
const CRTC_TEXT_STORAGE: u16 = 0x1000;

/// Returns whether the text layer contributes to scanout.
pub(super) const fn text_layer_visible(inputs: &RenderInputsX68k) -> bool {
    inputs.mixing & MIXING_TEXT_ENABLE != 0 && inputs.crtc_memory_mode & CRTC_TEXT_STORAGE == 0
}

/// Returns the 4-bit text color code at the given screen position.
pub(super) fn text_color_code(
    inputs: &RenderInputsX68k,
    screen_x: usize,
    screen_y: usize,
) -> usize {
    if !text_layer_visible(inputs) {
        return 0;
    }
    let source_x = (screen_x + inputs.text_scroll_x as usize) & TEXT_COORDINATE_MASK;
    let source_y = (screen_y + inputs.text_scroll_y as usize) & TEXT_COORDINATE_MASK;
    let byte = source_y * TEXT_LINE_BYTES + source_x / 8;
    let mask = 0x80 >> (source_x & 7);
    let mut color_code = 0_usize;
    for plane in 0..4 {
        if inputs.text_vram[plane * TEXT_PLANE_BYTES + byte] & mask != 0 {
            color_code |= 1 << plane;
        }
    }
    color_code
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::x68k::FixtureX68k;

    #[test]
    fn text_planes_combine_into_a_four_bit_code() {
        let mut fixture = FixtureX68k::new(8, 1);
        fixture.video_registers[2] = MIXING_TEXT_ENABLE;
        fixture.text_vram[0] = 0x80;
        fixture.text_vram[2 * TEXT_PLANE_BYTES] = 0x80;
        assert_eq!(text_color_code(&fixture.inputs(), 0, 0), 0b0101);
        assert_eq!(text_color_code(&fixture.inputs(), 1, 0), 0);
    }

    #[test]
    fn scroll_wraps_within_the_text_coordinate_space() {
        let mut fixture = FixtureX68k::new(8, 1);
        fixture.video_registers[2] = MIXING_TEXT_ENABLE;
        fixture.text_vram[0] = 0x80;
        fixture.text_scroll = (1023, 1023);
        assert_eq!(text_color_code(&fixture.inputs(), 1, 1), 0b0001);
    }

    #[test]
    fn storage_mode_and_disabled_text_blank_the_layer() {
        let mut fixture = FixtureX68k::new(8, 1);
        fixture.video_registers[2] = MIXING_TEXT_ENABLE;
        fixture.text_vram[0] = 0x80;
        fixture.crtc_memory_mode = CRTC_TEXT_STORAGE;
        assert_eq!(text_color_code(&fixture.inputs(), 0, 0), 0);
        fixture.crtc_memory_mode = 0;
        fixture.video_registers[2] = 0;
        assert_eq!(text_color_code(&fixture.inputs(), 0, 0), 0);
    }
}
