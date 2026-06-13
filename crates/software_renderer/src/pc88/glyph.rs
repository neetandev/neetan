//! PC-88 character-generator helpers: the fixed text palette and the generated
//! semigraphics block patterns.

use alloc::boxed::Box;

/// Fixed 8-color GRB text palette plus a black entry at index 8.
///
/// Color index bits follow the PC-88 GRB ordering: bit 0 blue, bit 1 red, bit 2
/// green. Index 8 is the "no color attribute" cell, which draws black.
pub const TEXT_PALETTE: [[u8; 3]; 9] = [
    [0, 0, 0],       // 0: black
    [0, 0, 255],     // 1: blue
    [255, 0, 0],     // 2: red
    [255, 0, 255],   // 3: magenta
    [0, 255, 0],     // 4: green
    [0, 255, 255],   // 5: cyan
    [255, 255, 0],   // 6: yellow
    [255, 255, 255], // 7: white
    [0, 0, 0],       // 8: no-color cell
];

/// Builds the 256-entry semigraphics block pattern (8 bytes per glyph). Each
/// character code maps to a 2x4 block layout: bits 0-3 select the left half of
/// each of the four vertical bands, bits 4-7 the right half.
pub fn build_semigraphics_pattern() -> Box<[u8; 256 * 8]> {
    let mut pattern = Box::new([0u8; 256 * 8]);
    for code in 0..256 {
        let base = code * 8;
        let byte = code as u8;
        let band = |left: u8, right: u8| {
            (if byte & left != 0 { 0xF0 } else { 0 }) | (if byte & right != 0 { 0x0F } else { 0 })
        };
        let rows = [
            band(0x01, 0x10),
            band(0x02, 0x20),
            band(0x04, 0x40),
            band(0x08, 0x80),
        ];
        pattern[base] = rows[0];
        pattern[base + 1] = rows[0];
        pattern[base + 2] = rows[1];
        pattern[base + 3] = rows[1];
        pattern[base + 4] = rows[2];
        pattern[base + 5] = rows[2];
        pattern[base + 6] = rows[3];
        pattern[base + 7] = rows[3];
    }
    pattern
}
