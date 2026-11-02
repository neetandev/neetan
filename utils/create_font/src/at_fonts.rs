//! PC/AT fonts: the layout of the font tables of the HLE VGA BIOS and of the
//! HLE system BIOS, and their build from the `.bit` files.
//!
//! A full table holds one glyph per character code in code order, one byte
//! per row with bit 7 as the leftmost pixel. The 8x8, 8x14 and 8x16 tables of
//! the VGA BIOS hold 256 glyphs. The 8x8 table of the system BIOS holds the
//! codes `00h`-`7Fh`.
//!
//! An alternate table holds the 9-dot replacement glyphs of a text font. An
//! entry is the character code followed by the rows of the glyph. The entries
//! follow in ascending code order and a code byte `00h` ends the table.

use std::path::Path;

use crate::bdf::BitmapFont;

/// Value of the code byte that ends an alternate table.
const ALTERNATE_END: u8 = 0x00;

/// One font table of the PC/AT HLE BIOS.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FontTable {
    /// 8x8 font of the VGA BIOS.
    Vga8x8,
    /// 8x14 font of the VGA BIOS.
    Vga8x14,
    /// 8x16 font of the VGA BIOS.
    Vga8x16,
    /// 9x14 alternate glyphs of the VGA BIOS.
    Vga9x14Alternate,
    /// 9x16 alternate glyphs of the VGA BIOS.
    Vga9x16Alternate,
    /// 8x8 font of the system BIOS, codes `00h`-`7Fh`.
    System8x8,
}

impl FontTable {
    /// File name of the table source in the font directory.
    fn file_name(self) -> &'static str {
        match self {
            FontTable::Vga8x8 => "at_vga_8x8.bit",
            FontTable::Vga8x14 => "at_vga_8x14.bit",
            FontTable::Vga8x16 => "at_vga_8x16.bit",
            FontTable::Vga9x14Alternate => "at_vga_9x14_alt.bit",
            FontTable::Vga9x16Alternate => "at_vga_9x16_alt.bit",
            FontTable::System8x8 => "at_system_8x8.bit",
        }
    }

    /// Number of character codes of a full table. `None` for an alternate
    /// table.
    fn code_count(self) -> Option<u32> {
        match self {
            FontTable::Vga8x8 | FontTable::Vga8x14 | FontTable::Vga8x16 => Some(256),
            FontTable::System8x8 => Some(128),
            FontTable::Vga9x14Alternate | FontTable::Vga9x16Alternate => None,
        }
    }
}

/// Returns the rows of the glyph of a character code.
fn glyph_rows(font: &BitmapFont, table: FontTable, code: u32) -> Option<Vec<u8>> {
    match table {
        FontTable::Vga8x8 | FontTable::System8x8 => font.get_8x8(code).map(Vec::from),
        FontTable::Vga8x14 | FontTable::Vga9x14Alternate => font.get_8x14(code).map(Vec::from),
        FontTable::Vga8x16 | FontTable::Vga9x16Alternate => font.get_8x16(code).map(Vec::from),
    }
}

/// Builds a font table from its `.bit` file in `fonts`.
pub fn build_table(fonts: &Path, table: FontTable) -> Result<Vec<u8>, String> {
    let font = BitmapFont::load(&fonts.join(table.file_name()))?;
    build_from_font(&font, table)
}

/// Builds a font table from a parsed font.
fn build_from_font(font: &BitmapFont, table: FontTable) -> Result<Vec<u8>, String> {
    let name = table.file_name();
    let encodings = font.encodings();
    let mut data = Vec::new();
    match table.code_count() {
        Some(count) => {
            if let Some(code) = encodings.iter().find(|&&code| code >= count) {
                return Err(format!("{name}: invalid ENCODING {code}"));
            }
            for code in 0..count {
                let rows = glyph_rows(font, table, code)
                    .ok_or_else(|| format!("{name}: character code {code} is missing"))?;
                data.extend_from_slice(&rows);
            }
        }
        None => {
            for code in encodings {
                if code == u32::from(ALTERNATE_END) || code > 0xFF {
                    return Err(format!("{name}: invalid ENCODING {code}"));
                }
                let rows = glyph_rows(font, table, code).ok_or_else(|| {
                    format!("{name}: glyph {code} has fewer BITMAP rows than its BBX height")
                })?;
                data.push(code as u8);
                data.extend_from_slice(&rows);
            }
            data.push(ALTERNATE_END);
        }
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// Offset of the font metadata words in the VGA BIOS image.
    const VGA_METADATA_OFFSET: usize = 0x10;
    /// Offset of the 8x8 font in the system BIOS image (F000:FA6E).
    const SYSTEM_FONT_OFFSET: usize = 0xFA6E;

    /// Directory of the PC/AT `.bit` files.
    fn fonts_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../bios_at/fonts")
    }

    /// Reads a file of `utils/bios_at`.
    fn read_bios_at(name: &str) -> Vec<u8> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../bios_at")
            .join(name);
        std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
    }

    /// Returns the source of one 8 pixel wide glyph with every row equal to
    /// the low byte of its code.
    fn glyph_source(code: u32, height: u32) -> String {
        let row: String = (0..8)
            .map(|bit| if code & (0x80 >> bit) != 0 { '@' } else { '.' })
            .collect();
        let mut source = format!("ENCODING {code}\nBBX 8 {height} 0 0\nBITMAP\n");
        for _ in 0..height {
            source.push_str(&row);
            source.push('\n');
        }
        source.push_str("ENDCHAR\n");
        source
    }

    /// Returns the source of a font with the codes `0..count`.
    fn full_font(height: u32, count: u32) -> String {
        let mut source = format!("FONT_ASCENT {height}\n");
        for code in 0..count {
            source.push_str(&glyph_source(code, height));
        }
        source
    }

    /// Reads the metadata word with the given index from the VGA BIOS image.
    fn metadata_word(rom: &[u8], index: usize) -> usize {
        let offset = VGA_METADATA_OFFSET + index * 2;
        usize::from(u16::from_le_bytes([rom[offset], rom[offset + 1]]))
    }

    #[test]
    fn committed_vga_bios_matches_a_build_of_the_bit_files() {
        let rom = read_bios_at("vgabios.rom");
        for (table, index) in [
            (FontTable::Vga8x8, 0),
            (FontTable::Vga8x14, 2),
            (FontTable::Vga8x16, 3),
            (FontTable::Vga9x14Alternate, 8),
            (FontTable::Vga9x16Alternate, 9),
        ] {
            let data = build_table(&fonts_dir(), table).unwrap();
            let offset = metadata_word(&rom, index);
            assert!(data == rom[offset..offset + data.len()], "{table:?}");
        }
        assert_eq!(metadata_word(&rom, 1), metadata_word(&rom, 0) + 0x400);
    }

    #[test]
    fn committed_system_bios_matches_a_build_of_the_bit_files() {
        let rom = read_bios_at("bios.rom");
        let data = build_table(&fonts_dir(), FontTable::System8x8).unwrap();
        assert!(data == rom[SYSTEM_FONT_OFFSET..SYSTEM_FONT_OFFSET + data.len()]);
    }

    #[test]
    fn full_tables_reject_codes_beyond_the_table() {
        let mut font = full_font(8, 128);
        font.push_str(&glyph_source(0x80, 8));
        let font = BitmapFont::parse(&font).unwrap();
        assert!(build_from_font(&font, FontTable::System8x8).is_err());
        assert!(build_from_font(&font, FontTable::Vga8x8).is_err());
    }

    #[test]
    fn full_tables_reject_a_missing_code() {
        let font =
            BitmapFont::parse(&full_font(14, 256).replace(&glyph_source(65, 14), "")).unwrap();
        let error = build_from_font(&font, FontTable::Vga8x14).unwrap_err();
        assert!(error.contains("code 65 is missing"), "{error}");
    }

    #[test]
    fn full_tables_place_glyphs_in_code_order() {
        let font = BitmapFont::parse(&full_font(16, 256)).unwrap();
        let data = build_from_font(&font, FontTable::Vga8x16).unwrap();
        assert_eq!(data.len(), 256 * 16);
        assert!(data[0x41 * 16..0x42 * 16].iter().all(|&byte| byte == 0x41));
    }

    #[test]
    fn alternate_tables_reject_code_zero_and_codes_above_ff() {
        for code in [0, 0x100] {
            let source = format!("FONT_ASCENT 16\n{}", glyph_source(code, 16));
            let font = BitmapFont::parse(&source).unwrap();
            assert!(build_from_font(&font, FontTable::Vga9x16Alternate).is_err());
        }
    }

    #[test]
    fn alternate_tables_write_code_rows_and_end_byte() {
        let source = format!(
            "FONT_ASCENT 14\n{}{}",
            glyph_source(0x9D, 14),
            glyph_source(0x1D, 14)
        );
        let font = BitmapFont::parse(&source).unwrap();
        let data = build_from_font(&font, FontTable::Vga9x14Alternate).unwrap();
        let mut expected = vec![0x1D];
        expected.extend([0x1D; 14]);
        expected.push(0x9D);
        expected.extend([0x9D; 14]);
        expected.push(ALTERNATE_END);
        assert_eq!(data, expected);
    }

    #[test]
    fn alternate_tables_list_ascending_codes_and_end_with_zero() {
        for (table, entries, height) in [
            (FontTable::Vga9x14Alternate, 20, 14),
            (FontTable::Vga9x16Alternate, 19, 16),
        ] {
            let data = build_table(&fonts_dir(), table).unwrap();
            let entry_size = 1 + height;
            assert_eq!(data.len(), entries * entry_size + 1);
            assert_eq!(data.last(), Some(&ALTERNATE_END));
            let codes: Vec<u8> = data[..data.len() - 1]
                .chunks(entry_size)
                .map(|entry| entry[0])
                .collect();
            assert!(codes.windows(2).all(|pair| pair[0] < pair[1]));
        }
    }
}
