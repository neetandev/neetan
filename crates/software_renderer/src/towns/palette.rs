//! FM Towns color conversion into packed RGBA.
//!
//! The analog palettes store 8-bit R/G/B components (the 16-color palettes carry
//! 4-bit precision, replicated into the high nibble by the register file). The
//! 32768-color direct mode packs a 5-5-5 value with green in the high bits.

/// Packs 8-bit red, green, and blue into `0xAA_BB_GG_RR` with opaque alpha, the
/// layout the display pipeline expects.
pub fn towns_color_to_rgba(red: u8, green: u8, blue: u8) -> u32 {
    u32::from(red) | (u32::from(green) << 8) | (u32::from(blue) << 16) | 0xFF00_0000
}

/// Expands a 5-bit channel value to 8 bits by replicating the top bits.
const fn level5(value: u16) -> u8 {
    ((value << 3) | (value >> 2)) as u8
}

/// Converts a 32768-color (5-5-5) direct-color value to packed RGBA. The layout
/// is `G[14:10] R[9:5] B[4:0]`; bit 15 is the transparency flag and is ignored
/// by the color conversion.
pub fn towns_color15_to_rgba(color: u16) -> u32 {
    let green = level5((color >> 10) & 0x1F);
    let red = level5((color >> 5) & 0x1F);
    let blue = level5(color & 0x1F);
    towns_color_to_rgba(red, green, blue)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgba_packs_components() {
        assert_eq!(towns_color_to_rgba(0x12, 0x34, 0x56), 0xFF56_3412);
    }

    #[test]
    fn color15_channels_land_in_the_right_byte() {
        // Pure red (bits 5-9).
        assert_eq!(towns_color15_to_rgba(0x03E0) & 0x00FF_FFFF, 0x0000_00FF);
        // Pure green (bits 10-14).
        assert_eq!(towns_color15_to_rgba(0x7C00) & 0x00FF_FFFF, 0x0000_FF00);
        // Pure blue (bits 0-4).
        assert_eq!(towns_color15_to_rgba(0x001F) & 0x00FF_FFFF, 0x00FF_0000);
    }
}
