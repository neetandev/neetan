//! PC-98 CG-ROM font expansion.
//!
//! Converts a V98-format font ROM dump into the interleaved layout the CG-ROM
//! I/O ports and the text renderer read. The host image selector uses the same
//! expansion to render its overlay with the built-in font.

/// Byte size of the expanded CG-ROM buffer.
pub const PC98_CGROM_SIZE: usize = 0x84000;

/// Byte size of a V98-format font ROM dump.
pub const V98_FONT_ROM_SIZE: usize = 0x46800;

/// Expands a V98-format font ROM into the interleaved CG-ROM layout.
///
/// Returns `false` and leaves `font_rom` untouched when `data` is shorter than
/// [`V98_FONT_ROM_SIZE`].
pub fn expand_v98_font_rom(font_rom: &mut [u8; PC98_CGROM_SIZE], data: &[u8]) -> bool {
    if data.len() < V98_FONT_ROM_SIZE {
        return false;
    }

    // ANK 0x00-0x7F (8x16): V98 offset 0x0800, 128 chars * 16 bytes
    font_rom[0x80000..0x80800].copy_from_slice(&data[0x0800..0x1000]);

    // ANK 0x80-0xFF (8x16): V98 offset 0x1000, 128 chars * 16 bytes
    font_rom[0x80800..0x81000].copy_from_slice(&data[0x1000..0x1800]);

    // Kanji level 1 (rows 0x01..0x30)
    v98_kanji_copy(font_rom, data, 0x01, 0x30);
    // Kanji level 2 (rows 0x30..0x56)
    v98_kanji_copy(font_rom, data, 0x30, 0x56);
    // Extended kanji (rows 0x58..0x5D)
    v98_kanji_copy(font_rom, data, 0x58, 0x5D);

    // ANK8 (6×8): V98 offset 0x0000, 256 chars × 8 bytes, stored with 16-byte stride.
    load_ank8_bank(font_rom, &data[0x0000..0x0800]);

    // Build chargraph semigraphics banks (writes to 0x81000 and 0x82000+8 per char).
    rebuild_chargraph_bank(font_rom);

    true
}

/// Converts V98 kanji font data to the interleaved fontrom layout.
fn v98_kanji_copy(font_rom: &mut [u8; PC98_CGROM_SIZE], src: &[u8], from: usize, to: usize) {
    for i in from..to {
        let mut p = 0x1800 + 0x60 * 32 * (i - 1);
        let mut q = 0x20000 + (i << 4);
        for _j in 0x20..0x80 {
            for _k in 0..16 {
                if q + 0x800 < font_rom.len() && p + 16 < src.len() {
                    font_rom[q + 0x800] = src[p + 16];
                    font_rom[q] = src[p];
                }
                p += 1;
                q += 1;
            }
            p += 16;
            q += 0x1000 - 16;
        }
    }
}

/// Builds the chargraph (semigraphics) banks in font ROM.
///
/// Generates 2×4 block element patterns for 256 possible byte values:
/// - 16×16 patterns at `0x81000` (4 groups × 4 rows × 1 byte/row = 16 bytes/char)
/// - 8×8 patterns at `0x82000+8` per char (4 groups × 2 rows × 1 byte/row = 8 bytes/char)
///
/// Bit mapping per char code byte: bits 0-3 control left column (rows 0-3),
/// bits 4-7 control right column (rows 0-3).
fn rebuild_chargraph_bank(font_rom: &mut [u8; PC98_CGROM_SIZE]) {
    let mut p = 0x81000usize;
    let mut q = 0x82000usize;
    for i in 0u32..256 {
        q += 8;
        for j in 0..4u32 {
            let mut dbit: u32 = 0;
            if i & (0x01 << j) != 0 {
                dbit |= 0xF0F0_F0F0;
            }
            if i & (0x10 << j) != 0 {
                dbit |= 0x0F0F_0F0F;
            }
            let bytes = dbit.to_le_bytes();
            font_rom[p..p + 4].copy_from_slice(&bytes);
            p += 4;
            font_rom[q..q + 2].copy_from_slice(&bytes[..2]);
            q += 2;
        }
    }
    // NEC patch: clear first two bytes of char 0xF2 chargraph entries.
    let f2_16 = 0x81000 + 0xF2 * 16;
    font_rom[f2_16] = 0;
    font_rom[f2_16 + 1] = 0;
    let f2_8 = 0x82000 + 0xF2 * 16 + 8;
    font_rom[f2_8] = 0;
}

/// Loads ANK8 (6×8) font data into the font ROM at `0x82000` with 16-byte stride.
///
/// Each of 256 characters occupies bytes 0-7 of its 16-byte slot (bytes 8-15
/// are reserved for chargraph8 patterns, populated separately).
fn load_ank8_bank(font_rom: &mut [u8; PC98_CGROM_SIZE], data: &[u8]) {
    for char_index in 0..256usize {
        let src_offset = char_index * 8;
        let dst_offset = 0x82000 + char_index * 16;
        if src_offset + 8 <= data.len() {
            font_rom[dst_offset..dst_offset + 8].copy_from_slice(&data[src_offset..src_offset + 8]);
        }
    }
}
