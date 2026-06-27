//! PC-88VA color conversion: the 16-bit VA color code to packed RGBA.
//!
//! A VA color code is 16 bits with green in bits 10-15 (6 bits), red in bits
//! 5-9 (5 bits) and blue in bits 0-4 (5 bits). Palette writes pass through
//! [`adjust_color12`], which replicates the low bit of each channel so that a
//! 12-bit value written by software expands to the full channel width.

/// Replicates the low bit of each color channel of a 12-bit palette value into
/// the extra bits the hardware carries, matching `adjustcolor12`.
pub fn adjust_color12(mut color: u16) -> u16 {
    if color & 0xF000 != 0 {
        color |= 0x0C00;
    }
    if color & 0x03C0 != 0 {
        color |= 0x0020;
    }
    if color & 0x001E != 0 {
        color |= 0x0001;
    }
    color
}

/// Expands a 5-bit channel value to 8 bits (`level << 3`, with the low 3 bits
/// set when the level is non-zero).
const fn level5(value: u16) -> u8 {
    let scaled = (value << 3) as u8;
    if value != 0 { scaled | 0x07 } else { scaled }
}

/// Expands a 6-bit channel value to 8 bits (`level << 2`, with the low 2 bits
/// set when the level is non-zero).
const fn level6(value: u16) -> u8 {
    let scaled = (value << 2) as u8;
    if value != 0 { scaled | 0x03 } else { scaled }
}

/// Converts a 16-bit VA color code to packed `0xAA_BB_GG_RR` (the same layout
/// the PC-98 renderer uses), with alpha forced opaque.
pub fn va_color_to_rgba(color: u16) -> u32 {
    let green = level6((color & 0xFC00) >> 10);
    let red = level5((color & 0x03E0) >> 5);
    let blue = level5(color & 0x001F);
    u32::from(red) | (u32::from(green) << 8) | (u32::from(blue) << 16) | 0xFF00_0000
}

/// Converts an 8-bit direct-color value (3-3-2 green/red/blue) to a 16-bit VA
/// color code, matching the `rgb8to16` table.
pub fn rgb8_to_va_color(value: u8) -> u16 {
    let blue = u16::from(value) & 0x3;
    let red = (u16::from(value) >> 2) & 0x7;
    let green = (u16::from(value) >> 5) & 0x7;
    (green << 13)
        | (if green == 0 { 0 } else { 0x1C00 })
        | (red << 7)
        | (if red == 0 { 0 } else { 0x0060 })
        | (blue << 3)
        | (if blue == 0 { 0 } else { 0x0007 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adjust_color12_replicates_channel_low_bits() {
        // Each channel's high group pulls in its replicated low bit.
        assert_eq!(adjust_color12(0xF000), 0xFC00);
        assert_eq!(adjust_color12(0x03C0), 0x03E0);
        assert_eq!(adjust_color12(0x001E), 0x001F);
        // Zero channels stay zero.
        assert_eq!(adjust_color12(0x0000), 0x0000);
    }

    #[test]
    fn full_white_and_black_convert() {
        assert_eq!(va_color_to_rgba(0xFFFF), 0xFFFF_FFFF);
        assert_eq!(va_color_to_rgba(0x0000), 0xFF00_0000);
    }

    #[test]
    fn pure_channels_land_in_the_right_byte() {
        // Pure red: bits 5-9.
        assert_eq!(va_color_to_rgba(0x03E0) & 0x00FF_FFFF, 0x0000_00FF);
        // Pure green: bits 10-15.
        assert_eq!(va_color_to_rgba(0xFC00) & 0x00FF_FFFF, 0x0000_FF00);
        // Pure blue: bits 0-4.
        assert_eq!(va_color_to_rgba(0x001F) & 0x00FF_FFFF, 0x00FF_0000);
    }
}
