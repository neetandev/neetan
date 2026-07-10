//! X68000 color conversion.

/// GRBI bits of the green and blue channels.
const GREEN_BLUE_CHANNELS: u16 = 0xF83E;
/// GRBI bits of the red channel.
const RED_CHANNEL: u16 = 0x07C0;
/// GRBI bits of the red channel and the intensity bit.
const RED_CHANNEL_AND_INTENSITY: u16 = 0x07C1;

/// Halves each GRBI channel of a color and clears the intensity bit.
pub const fn mix_halved_grbi(front: u16) -> u16 {
    (front >> 1) & 0x7BDE
}

/// Averages two GRBI colors; the intensity bit comes from the back color.
pub const fn mix_averaged_grbi(front: u16, back: u16) -> u16 {
    let green_blue =
        (((front & GREEN_BLUE_CHANNELS) + (back & GREEN_BLUE_CHANNELS)) >> 1) & GREEN_BLUE_CHANNELS;
    let red_intensity = (((front & RED_CHANNEL | 1) + (back & RED_CHANNEL_AND_INTENSITY)) >> 1)
        & RED_CHANNEL_AND_INTENSITY;
    green_blue | red_intensity
}

/// Returns the 65536-color mode color for one 16-bit palette code.
///
/// Even graphics palette entries supply the low color byte and odd entries
/// the high color byte; bit 0 of each code half selects the entry byte.
pub(super) fn graphic_color_65536(palette: &[u16; 256], code: u16) -> u16 {
    let low_code = usize::from(code & 0x00FF);
    let low_entry = palette[low_code & !1];
    let low_byte = if low_code & 1 == 0 {
        low_entry >> 8
    } else {
        low_entry & 0x00FF
    };
    let high_code = usize::from(code >> 8);
    let high_entry = palette[high_code | 1];
    let high_byte = if high_code & 1 == 0 {
        high_entry >> 8
    } else {
        high_entry & 0x00FF
    };
    high_byte << 8 | low_byte
}

/// Converts an X68000 GRBI value to RGBA at the selected contrast.
pub fn grbi_to_rgba(value: u16, contrast: u8) -> [u8; 4] {
    let intensity = value & 1;
    let blue = (((value >> 1) & 0x1F) * 2) | intensity;
    let red = (((value >> 6) & 0x1F) * 2) | intensity;
    let green = (((value >> 11) & 0x1F) * 2) | intensity;
    let scale = |channel: u16| -> u8 {
        let full = (u32::from(channel) * 255 + 31) / 63;
        ((full * u32::from(contrast.min(15)) + 7) / 15) as u8
    };
    [scale(red), scale(green), scale(blue), 0xFF]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn averaged_colors_round_down_and_take_the_back_intensity() {
        assert_eq!(mix_averaged_grbi(0xFFFF, 0x0000), 0x7BDE);
        assert_eq!(mix_averaged_grbi(0xFFFE, 0x0001), 0x7BDF);
        assert_eq!(mix_averaged_grbi(0x0000, 0xFFFF), 0x7BDF);
        assert_eq!(mix_averaged_grbi(0x1800, 0x0800), 0x1000);
        assert_eq!(mix_averaged_grbi(0x0040, 0x00C0), 0x0080);
        assert_eq!(mix_averaged_grbi(0x0002, 0x0006), 0x0004);
        assert_eq!(mix_averaged_grbi(0xF800, 0x003E), 0x781E);
    }

    #[test]
    fn halving_matches_averaging_with_black() {
        for color in [0x0000_u16, 0x0001, 0xFFFF, 0x1234, 0xF83E, 0x07C1] {
            assert_eq!(mix_halved_grbi(color), mix_averaged_grbi(color, 0));
        }
    }

    #[test]
    fn full_color_codes_pair_even_and_odd_palette_bytes() {
        let mut palette = [0_u16; 256];
        palette[0] = 0x1234;
        palette[1] = 0x5678;
        palette[254] = 0x9ABC;
        palette[255] = 0xDEF0;
        assert_eq!(graphic_color_65536(&palette, 0x0000), 0x5612);
        assert_eq!(graphic_color_65536(&palette, 0x0001), 0x5634);
        assert_eq!(graphic_color_65536(&palette, 0x0100), 0x7812);
        assert_eq!(graphic_color_65536(&palette, 0x00FF), 0x56BC);
        assert_eq!(graphic_color_65536(&palette, 0xFF00), 0xF012);
        assert_eq!(graphic_color_65536(&palette, 0xFFFF), 0xF0BC);
    }

    #[test]
    fn grbi_intensity_is_the_low_channel_bit() {
        assert_eq!(grbi_to_rgba(0, 15), [0, 0, 0, 0xFF]);
        assert_eq!(grbi_to_rgba(1, 15), [4, 4, 4, 0xFF]);
        assert_eq!(grbi_to_rgba(0xFFFE, 15), [251, 251, 251, 0xFF]);
        assert_eq!(grbi_to_rgba(0xFFFF, 15), [255, 255, 255, 0xFF]);
        assert_eq!(grbi_to_rgba(0xFFFF, 0), [0, 0, 0, 0xFF]);
    }
}
