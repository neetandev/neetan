//! X1 digital colour decode and priority mixing.
//!
//! Every displayed pixel resolves to one of sixteen palette indices: 0..7 for
//! text colours and 8..15 for the (gun-remapped) graphics colours. Both banks
//! decode the low three bits digitally: blue = bit 0, red = bit 1, green =
//! bit 2. The programmable palette latches (blue at `0x1000`, red at `0x1100`,
//! green at `0x1200`) hold one bit per graphics colour and remap each graphics
//! colour to a new 3-bit colour before display. The priority register decides
//! per graphics colour whether it covers opaque text; text colour 0 is
//! transparent and always shows the graphics underneath.

use super::X1RendererModel;

/// Number of entries in the final palette (8 text + 8 graphics).
pub(super) const PALETTE_ENTRIES: usize = 16;

/// Bytes per per-line priority table: one entry per (cg, text) colour pair.
pub(super) const PRI_LUT_SIZE: usize = 64;

/// The fixed RGBA colours for the sixteen palette indices; the colour comes
/// from the low three bits of the index.
pub(super) const FIXED_RGBA: [[u8; 4]; PALETTE_ENTRIES] = build_fixed_rgba();

const fn build_fixed_rgba() -> [[u8; 4]; PALETTE_ENTRIES] {
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

/// Builds the per-line priority table mapping a (cg colour, text colour) pair
/// to a final palette index. The gun latches remap each graphics colour, mode
/// register 2 can force graphics colours 0/1 black and key one text colour
/// transparent, and the priority register decides which layer wins.
pub(super) fn build_pri_lut(
    guns: [u8; 3],
    priority: u8,
    mode2: u8,
    model: X1RendererModel,
) -> [u8; PRI_LUT_SIZE] {
    let mut remapped = [0u8; 8];
    for (color, entry) in remapped.iter_mut().enumerate() {
        let bit = 1u8 << color;
        *entry = u8::from(guns[0] & bit != 0)
            | (u8::from(guns[1] & bit != 0) << 1)
            | (u8::from(guns[2] & bit != 0) << 2)
            | 8;
    }
    match model {
        X1RendererModel::Base => {}
        X1RendererModel::Turbo => {
            if mode2 & 0x10 != 0 {
                remapped[0] = 8;
            }
            if mode2 & 0x20 != 0 {
                remapped[1] = 8;
            }
        }
    }

    let mut lut = [0u8; PRI_LUT_SIZE];
    for cg in 0..8usize {
        for text in 0..8u8 {
            lut[cg * 8 + usize::from(text)] = if priority & (1 << cg) != 0 {
                remapped[cg]
            } else if text != 0 {
                let keyed = match model {
                    X1RendererModel::Base => false,
                    X1RendererModel::Turbo => mode2 & 0x08 != 0 && mode2 & 0x07 == text,
                };
                if keyed { 0 } else { text }
            } else {
                remapped[cg]
            };
        }
    }
    lut
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_palette_decodes_the_low_three_bits() {
        assert_eq!(FIXED_RGBA[0], [0x00, 0x00, 0x00, 0xFF]);
        assert_eq!(FIXED_RGBA[7], [0xFF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(FIXED_RGBA[1], [0x00, 0x00, 0xFF, 0xFF]); // blue
        assert_eq!(FIXED_RGBA[2], [0xFF, 0x00, 0x00, 0xFF]); // red
        assert_eq!(FIXED_RGBA[4], [0x00, 0xFF, 0x00, 0xFF]); // green
        assert_eq!(FIXED_RGBA[8 | 3], FIXED_RGBA[3]);
    }

    #[test]
    fn gun_latches_remap_graphics_colours() {
        // Blue latch bit 3: graphics colour 3 remaps to blue (index 9).
        let lut = build_pri_lut([0x08, 0x00, 0x00], 0xFF, 0, X1RendererModel::Base);
        assert_eq!(lut[3 * 8], 8 | 1);
        assert_eq!(lut[2 * 8], 8);
    }

    #[test]
    fn priority_bit_decides_the_winning_layer() {
        // Identity guns; only graphics colour 5 has priority over text.
        let guns = [0xAA, 0xCC, 0xF0];
        let lut = build_pri_lut(guns, 0x20, 0, X1RendererModel::Base);
        assert_eq!(lut[5 * 8 + 3], 8 | 5); // cg 5 covers text 3
        assert_eq!(lut[4 * 8 + 3], 3); // text 3 covers cg 4
        assert_eq!(lut[4 * 8], 8 | 4); // transparent text shows cg 4
    }

    #[test]
    fn mode2_forces_graphics_colours_black_and_keys_one_text_colour() {
        let guns = [0xAA, 0xCC, 0xF0];
        let lut = build_pri_lut(guns, 0x03, 0x30 | 0x08 | 0x05, X1RendererModel::Turbo);
        assert_eq!(lut[0], 8); // cg 0 forced black
        assert_eq!(lut[8], 8); // cg 1 forced black
        assert_eq!(lut[2 * 8 + 5], 0); // text colour 5 keyed out
        assert_eq!(lut[2 * 8 + 3], 3); // other text colours unaffected
    }
}
