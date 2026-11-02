//! PC-98 fonts: the V98 layout of the font ROM and its build from the `.bit`
//! files.
//!
//! The 8x8 ANK font starts at `0000h` (8 bytes per code) and the 8x16 ANK font
//! at `0800h` (16 bytes per code). The 16x16 font starts at `1800h` with
//! `0C00h` bytes for each JIS row `21h`-`7Ch` and 32 bytes for each cell of the
//! columns `20h`-`7Fh`. A cell holds the left 8 pixels of row `r` in byte `r`
//! and the right 8 pixels in byte `10h + r`. A half-width glyph of the rows
//! `29h`-`2Bh` fills bytes `00h`-`0Fh` of its cell. The fill cells hold `FFh`.
//! Every other byte without a glyph is zero.

use std::{ops::RangeInclusive, path::Path};

use crate::bdf::BitmapFont;

/// Size of the font ROM image.
const FONT_ROM_SIZE: usize = 0x4_6800;
/// Offset of the 8x8 ANK font.
const ANK_8X8_OFFSET: usize = 0x0000;
/// Offset of the 8x16 ANK font.
const ANK_8X16_OFFSET: usize = 0x0800;
/// Offset of the first cell of the 16x16 font (JIS row `21h`, column `20h`).
const KANJI_OFFSET: usize = 0x1800;
/// Bytes of one JIS row of the 16x16 font.
const ROW_BYTES: usize = 0x0C00;
/// Bytes of one 16x16 cell.
const CELL_BYTES: usize = 32;
/// Bytes of one half of a 16x16 cell.
const HALF_CELL_BYTES: usize = 16;
/// First JIS row of the 16x16 font.
const FIRST_ROW: u8 = 0x21;
/// First column of a JIS row of the 16x16 font.
const FIRST_COLUMN: u8 = 0x20;
/// Columns of a JIS row that a glyph may fill.
const GLYPH_COLUMNS: RangeInclusive<u8> = 0x21..=0x7E;
/// Value of a fill byte.
const FILL_BYTE: u8 = 0xFF;
/// JIS rows with every cell of the columns `20h`-`7Fh` filled.
const FILLED_ROWS: RangeInclusive<u8> = 0x74..=0x75;
/// JIS rows with the cell of `FILLED_COLUMN` filled.
const FILLED_COLUMN_ROWS: RangeInclusive<u8> = 0x79..=0x7C;
/// Column of the filled cell in the rows `FILLED_COLUMN_ROWS`.
const FILLED_COLUMN: u8 = 0x20;
/// Offset of the single fill byte, the last byte of the cell `7B7Fh`.
const FILLED_BYTE_OFFSET: usize = 0x4_5BFF;
/// JIS code of the Neetan tripwire glyph. The original ROM leaves this cell
/// empty.
const TRIPWIRE_CODE: u32 = 0x2E21;

/// One `.bit` file of the PC-98 fonts.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FontSet {
    /// 8x8 ANK font.
    Ank8x8,
    /// 8x16 ANK font.
    Ank8x16,
    /// 16x16 non-kanji, rows `21h`-`27h` and `2Ch`-`2Dh`.
    NonKanji,
    /// 8x16 half-width glyphs, rows `29h`-`2Bh`.
    HalfWidth,
    /// 16x16 level 1 kanji, rows `30h`-`4Fh`.
    Kanji1,
    /// 16x16 level 2 kanji, rows `50h`-`73h`.
    Kanji2,
    /// 16x16 NEC selected IBM extension, rows `79h`-`7Ch`.
    Extension,
    /// The Neetan tripwire glyph.
    Tripwire,
}

impl FontSet {
    /// Every set in build order.
    const ALL: [FontSet; 8] = [
        FontSet::Ank8x8,
        FontSet::Ank8x16,
        FontSet::NonKanji,
        FontSet::HalfWidth,
        FontSet::Kanji1,
        FontSet::Kanji2,
        FontSet::Extension,
        FontSet::Tripwire,
    ];

    /// File name of the set in the font directory.
    fn file_name(self) -> &'static str {
        match self {
            FontSet::Ank8x8 => "pc98_ank_8x8.bit",
            FontSet::Ank8x16 => "pc98_ank_8x16.bit",
            FontSet::NonKanji => "pc98_16_nonkanji.bit",
            FontSet::HalfWidth => "pc98_16_halfwidth.bit",
            FontSet::Kanji1 => "pc98_16_kanji1.bit",
            FontSet::Kanji2 => "pc98_16_kanji2.bit",
            FontSet::Extension => "pc98_16_extension.bit",
            FontSet::Tripwire => "pc98_tripwire.bit",
        }
    }

    /// Returns whether the set may fill the cell of a JIS code.
    fn accepts(self, code: u32) -> bool {
        let row = (code >> 8) as u8;
        let column = code as u8;
        if code > 0xFFFF || !GLYPH_COLUMNS.contains(&column) {
            return false;
        }
        match self {
            FontSet::NonKanji => matches!(row, 0x21..=0x27 | 0x2C..=0x2D),
            FontSet::HalfWidth => (0x29..=0x2B).contains(&row),
            FontSet::Kanji1 => (0x30..=0x4F).contains(&row),
            FontSet::Kanji2 => (0x50..=0x73).contains(&row),
            FontSet::Extension => (0x79..=0x7C).contains(&row),
            FontSet::Tripwire => code == TRIPWIRE_CODE,
            FontSet::Ank8x8 | FontSet::Ank8x16 => false,
        }
    }
}

/// Returns the offset of the cell of a JIS row and column in the 16x16 font.
fn cell_offset(row: u8, column: u8) -> usize {
    KANJI_OFFSET
        + usize::from(row - FIRST_ROW) * ROW_BYTES
        + usize::from(column - FIRST_COLUMN) * CELL_BYTES
}

/// Converts a 16x16 glyph in row order into the V98 cell layout.
fn v98_cell(rows: &[u8; CELL_BYTES]) -> [u8; CELL_BYTES] {
    let mut cell = [0u8; CELL_BYTES];
    for row in 0..HALF_CELL_BYTES {
        cell[row] = rows[2 * row];
        cell[HALF_CELL_BYTES + row] = rows[2 * row + 1];
    }
    cell
}

/// Writes the fill cells and the fill byte.
fn write_fill(rom: &mut [u8]) {
    let fill_cell = [FILL_BYTE; CELL_BYTES];
    for row in FILLED_ROWS {
        let start = cell_offset(row, FIRST_COLUMN);
        rom[start..start + ROW_BYTES].fill(FILL_BYTE);
    }
    for row in FILLED_COLUMN_ROWS {
        let offset = cell_offset(row, FILLED_COLUMN);
        rom[offset..offset + CELL_BYTES].copy_from_slice(&fill_cell);
    }
    rom[FILLED_BYTE_OFFSET] = FILL_BYTE;
}

/// Writes the glyphs of one set into the font ROM image.
fn place_set(rom: &mut [u8], set: FontSet, font: &BitmapFont) -> Result<(), String> {
    for encoding in font.encodings() {
        let invalid = || format!("{}: invalid ENCODING {encoding}", set.file_name());
        let broken = || format!("{}: glyph {encoding} is broken", set.file_name());
        match set {
            FontSet::Ank8x8 | FontSet::Ank8x16 if encoding > 0xFF => return Err(invalid()),
            FontSet::Ank8x8 => {
                let offset = ANK_8X8_OFFSET + encoding as usize * 8;
                let glyph = font.get_8x8(encoding).ok_or_else(broken)?;
                rom[offset..offset + 8].copy_from_slice(&glyph);
            }
            FontSet::Ank8x16 => {
                let offset = ANK_8X16_OFFSET + encoding as usize * 16;
                let glyph = font.get_8x16(encoding).ok_or_else(broken)?;
                rom[offset..offset + 16].copy_from_slice(&glyph);
            }
            _ if !set.accepts(encoding) => return Err(invalid()),
            FontSet::HalfWidth => {
                let offset = cell_offset((encoding >> 8) as u8, encoding as u8);
                let glyph = font.get_8x16(encoding).ok_or_else(broken)?;
                rom[offset..offset + HALF_CELL_BYTES].copy_from_slice(&glyph);
            }
            FontSet::NonKanji
            | FontSet::Kanji1
            | FontSet::Kanji2
            | FontSet::Extension
            | FontSet::Tripwire => {
                let offset = cell_offset((encoding >> 8) as u8, encoding as u8);
                let glyph = font.get_16x16_rows(encoding).ok_or_else(broken)?;
                rom[offset..offset + CELL_BYTES].copy_from_slice(&v98_cell(&glyph));
            }
        }
    }
    Ok(())
}

/// Builds the 282 KiB V98 font ROM from the `.bit` files in `fonts`.
pub fn build_font_rom(fonts: &Path) -> Result<Vec<u8>, String> {
    let mut rom = vec![0u8; FONT_ROM_SIZE];
    write_fill(&mut rom);
    for set in FontSet::ALL {
        let font = BitmapFont::load(&fonts.join(set.file_name()))?;
        place_set(&mut rom, set, &font)?;
    }
    Ok(rom)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// Offset of the tripwire cell in the font ROM.
    const TRIPWIRE_OFFSET: usize = 0xB420;

    /// Directory of the PC-98 `.bit` files.
    fn fonts_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../bios/fonts")
    }

    /// Reads the committed font ROM.
    fn read_font_rom() -> Vec<u8> {
        let path = fonts_dir().join("font.rom");
        std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
    }

    /// Builds a one-glyph font with a full-cell bounding box.
    fn single_glyph(encoding: u32, width: u32) -> BitmapFont {
        let mut source =
            format!("FONT_ASCENT 16\nENCODING {encoding}\nBBX {width} 16 0 0\nBITMAP\n");
        for _ in 0..16 {
            source.push_str(&"@".repeat(width as usize));
            source.push('\n');
        }
        source.push_str("ENDCHAR\n");
        BitmapFont::parse(&source).unwrap()
    }

    /// Places a one-glyph font of a set into an empty image.
    fn place_single(set: FontSet, encoding: u32, width: u32) -> Result<Vec<u8>, String> {
        let mut rom = vec![0u8; FONT_ROM_SIZE];
        place_set(&mut rom, set, &single_glyph(encoding, width))?;
        Ok(rom)
    }

    #[test]
    fn cell_offsets_follow_the_v98_layout() {
        assert_eq!(cell_offset(0x21, 0x20), KANJI_OFFSET);
        assert_eq!(cell_offset(0x2E, 0x21), TRIPWIRE_OFFSET);
        assert_eq!(cell_offset(0x7B, 0x7F) + CELL_BYTES - 1, FILLED_BYTE_OFFSET);
        assert_eq!(cell_offset(0x7C, 0x7F) + CELL_BYTES, FONT_ROM_SIZE);
    }

    #[test]
    fn v98_cell_splits_the_left_and_right_halves() {
        let mut rows = [0u8; CELL_BYTES];
        rows[0] = 0x80;
        rows[1] = 0x01;
        rows[31] = 0x42;
        let cell = v98_cell(&rows);
        assert_eq!(cell[0], 0x80);
        assert_eq!(cell[16], 0x01);
        assert_eq!(cell[31], 0x42);
        assert_eq!(cell.iter().filter(|&&byte| byte != 0).count(), 3);
    }

    #[test]
    fn fill_rule_writes_the_fill_cells() {
        let mut rom = vec![0u8; FONT_ROM_SIZE];
        write_fill(&mut rom);
        let cell = |row: u8, column: u8| {
            let offset = cell_offset(row, column);
            rom[offset..offset + CELL_BYTES].to_vec()
        };
        for (row, column) in [(0x74, 0x20), (0x75, 0x7F), (0x79, 0x20), (0x7C, 0x20)] {
            assert!(cell(row, column).iter().all(|&byte| byte == FILL_BYTE));
        }
        assert!(cell(0x79, 0x21).iter().all(|&byte| byte == 0));
        assert!(cell(0x73, 0x7F).iter().all(|&byte| byte == 0));
        assert_eq!(
            rom.iter().filter(|&&byte| byte == FILL_BYTE).count(),
            196 * 32 + 1
        );
    }

    #[test]
    fn committed_font_rom_matches_a_build_of_the_bit_files() {
        let font_rom = build_font_rom(&fonts_dir()).unwrap();
        assert!(font_rom == read_font_rom());
    }

    #[test]
    fn tripwire_cell_holds_the_neetan_glyph() {
        let font_rom = read_font_rom();
        let cell = &font_rom[TRIPWIRE_OFFSET..TRIPWIRE_OFFSET + CELL_BYTES];
        let tripwire = BitmapFont::load(&fonts_dir().join(FontSet::Tripwire.file_name())).unwrap();
        assert_eq!(tripwire.encodings(), vec![TRIPWIRE_CODE]);
        let glyph = tripwire.get_16x16_rows(TRIPWIRE_CODE).unwrap();
        assert_eq!(cell, v98_cell(&glyph));
        assert!(cell.iter().any(|&byte| byte != 0));
    }

    #[test]
    fn user_defined_rows_are_rejected() {
        for set in [
            FontSet::NonKanji,
            FontSet::Kanji1,
            FontSet::Kanji2,
            FontSet::Extension,
            FontSet::Tripwire,
        ] {
            assert!(place_single(set, 0x7621, 16).is_err());
            assert!(place_single(set, 0x777E, 16).is_err());
        }
        assert!(place_single(FontSet::HalfWidth, 0x7621, 8).is_err());
    }

    #[test]
    fn half_width_entries_outside_rows_29_to_2b_are_rejected() {
        assert!(place_single(FontSet::HalfWidth, 0x2921, 8).is_ok());
        assert!(place_single(FontSet::HalfWidth, 0x2B7E, 8).is_ok());
        assert!(place_single(FontSet::HalfWidth, 0x2821, 8).is_err());
        assert!(place_single(FontSet::HalfWidth, 0x2C21, 8).is_err());
    }

    #[test]
    fn full_width_entries_inside_rows_29_to_2b_are_rejected() {
        assert!(place_single(FontSet::NonKanji, 0x2D21, 16).is_ok());
        assert!(place_single(FontSet::NonKanji, 0x2921, 16).is_err());
        assert!(place_single(FontSet::NonKanji, 0x2B7E, 16).is_err());
    }

    #[test]
    fn two_entries_for_one_cell_are_rejected() {
        let source = "FONT_ASCENT 16\nENCODING 12321\nBBX 16 16 0 0\nBITMAP\nENDCHAR\n\
                      ENCODING 12321\nBBX 16 16 0 0\nBITMAP\nENDCHAR\n";
        assert!(BitmapFont::parse(source).is_err());
    }

    #[test]
    fn missing_ank_codes_stay_zero_and_codes_above_ff_are_rejected() {
        let rom = place_single(FontSet::Ank8x16, 0x41, 8).unwrap();
        assert!(
            rom[ANK_8X16_OFFSET + 0x41 * 16..ANK_8X16_OFFSET + 0x42 * 16]
                .iter()
                .all(|&byte| byte == 0xFF)
        );
        assert_eq!(rom.iter().filter(|&&byte| byte != 0).count(), 16);
        assert!(place_single(FontSet::Ank8x8, 0x100, 8).is_err());
    }
}
