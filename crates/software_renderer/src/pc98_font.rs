//! PC-98 character generator (CG-ROM) image built from a V98 font ROM.
//!
//! The CG-ROM image is addressed by the character code word of text VRAM.
//! The high byte of the word holds the second JIS byte (the column, 20h-7Fh).
//! The low byte holds the first JIS byte minus 20h (the code row, 01h-5Fh).
//! ANK characters use a zero high byte and the ANK code in the low byte.
//!
//! - Kanji cell, left half: `column * 1000h + code_row * 10h + line` for the
//!   lines 0-15. The right half follows at `+ 800h`. The image spans
//!   `20000h-7FFFFh`.
//! - ANK 8x16 font: `80000h + code * 10h + line` for the lines 0-15.
//! - Semigraphics 8x16 font: `81000h + code * 10h + line` for the lines 0-15.
//! - ANK 8x8 font: `82000h + code * 10h + line` for the lines 0-7.
//! - Semigraphics 8x8 font: `82000h + code * 10h + 8 + line` for the lines 0-7.
//!
//! A semigraphics code divides the character face into a grid of two columns
//! and four bands. Bits 0-3 light the bands of the left column from top to
//! bottom. Bits 4-7 light the bands of the right column.
//!
//! The JIS rows 76h-77h hold the user-defined characters in CG RAM. The
//! expansion leaves these rows unchanged.

use std::ops::RangeInclusive;

/// Byte size of the expanded CG-ROM buffer.
pub const PC98_CGROM_SIZE: usize = 0x84000;

/// Byte size of a V98-format font ROM dump.
pub const V98_FONT_ROM_SIZE: usize = 0x46800;

/// Offset of the 8x8 ANK font in a V98 image.
const V98_ANK_8X8_OFFSET: usize = 0x0000;

/// Offset of the 8x16 ANK font in a V98 image.
const V98_ANK_8X16_OFFSET: usize = 0x0800;

/// Offset of the first 16x16 cell (JIS row 21h, column 20h) in a V98 image.
const V98_KANJI_OFFSET: usize = 0x1800;

/// Bytes of one JIS row in a V98 image.
const V98_ROW_BYTES: usize = 0x0C00;

/// Bytes of one 16x16 cell in a V98 image.
const V98_CELL_BYTES: usize = 32;

/// Offset of the right half inside a V98 cell.
const V98_RIGHT_HALF_OFFSET: usize = 16;

/// Bytes of one 8x8 ANK glyph in a V98 image.
const V98_ANK_8X8_GLYPH_BYTES: usize = 8;

/// Number of ANK codes.
const ANK_CODE_COUNT: usize = 256;

/// Lines of a 16 line glyph.
const GLYPH_LINES_16: usize = 16;

/// Lines of an 8 line glyph.
const GLYPH_LINES_8: usize = 8;

/// JIS rows present in a V98 image.
const JIS_ROWS: RangeInclusive<u8> = 0x21..=0x7C;

/// JIS columns of a V98 row, both padding columns included.
const JIS_COLUMNS: RangeInclusive<u8> = 0x20..=0x7F;

/// JIS rows of the user-defined characters in CG RAM.
const USER_DEFINED_ROWS: RangeInclusive<u8> = 0x76..=0x77;

/// Value subtracted from the first JIS byte to form the code row.
const CODE_ROW_BIAS: u8 = 0x20;

/// CG-ROM address step of one JIS column.
const CGROM_COLUMN_STRIDE: usize = 0x1000;

/// CG-ROM address step of one code row.
const CGROM_ROW_STRIDE: usize = 0x10;

/// Offset of the right half of a kanji cell in the CG-ROM image.
const CGROM_RIGHT_HALF_OFFSET: usize = 0x800;

/// Base of the 8x16 ANK font in the CG-ROM image.
const CGROM_ANK_8X16_BASE: usize = 0x80000;

/// Base of the 8x16 semigraphics font in the CG-ROM image.
const CGROM_SEMIGRAPHICS_8X16_BASE: usize = 0x81000;

/// Base of the 8x8 ANK font in the CG-ROM image.
const CGROM_ANK_8X8_BASE: usize = 0x82000;

/// Offset of an 8x8 semigraphics glyph inside its 8x8 ANK slot.
const CGROM_SEMIGRAPHICS_8X8_OFFSET: usize = 8;

/// CG-ROM address step of one ANK or semigraphics code.
const CGROM_ANK_STRIDE: usize = 0x10;

/// Number of bands of a semigraphics glyph.
const SEMIGRAPHICS_BANDS: usize = 4;

/// Bit of the left column for the top band of a semigraphics code.
const SEMIGRAPHICS_LEFT_BIT: u8 = 0x01;

/// Bit of the right column for the top band of a semigraphics code.
const SEMIGRAPHICS_RIGHT_BIT: u8 = 0x10;

/// Dots of the left column in one glyph line.
const SEMIGRAPHICS_LEFT_DOTS: u8 = 0xF0;

/// Dots of the right column in one glyph line.
const SEMIGRAPHICS_RIGHT_DOTS: u8 = 0x0F;

/// Semigraphics code whose NEC glyphs start with blank lines.
const SEMIGRAPHICS_BLANK_TOP_CODE: usize = 0xF2;

/// Blank top lines of the 8x16 glyph of `SEMIGRAPHICS_BLANK_TOP_CODE`.
const SEMIGRAPHICS_BLANK_TOP_LINES_16: usize = 2;

/// Blank top lines of the 8x8 glyph of `SEMIGRAPHICS_BLANK_TOP_CODE`.
const SEMIGRAPHICS_BLANK_TOP_LINES_8: usize = 1;

/// Expands a V98-format font ROM into the CG-ROM image.
///
/// Returns `false` and leaves `font_rom` untouched when `data` is shorter than
/// [`V98_FONT_ROM_SIZE`].
pub fn expand_v98_font_rom(font_rom: &mut [u8; PC98_CGROM_SIZE], data: &[u8]) -> bool {
    if data.len() < V98_FONT_ROM_SIZE {
        return false;
    }

    copy_ank_8x16(font_rom, data);
    copy_ank_8x8(font_rom, data);
    copy_kanji(font_rom, data);
    write_semigraphics(font_rom);

    true
}

/// Copies the 256 glyphs of the 8x16 ANK font.
fn copy_ank_8x16(font_rom: &mut [u8; PC98_CGROM_SIZE], data: &[u8]) {
    let length = ANK_CODE_COUNT * CGROM_ANK_STRIDE;
    font_rom[CGROM_ANK_8X16_BASE..CGROM_ANK_8X16_BASE + length]
        .copy_from_slice(&data[V98_ANK_8X16_OFFSET..V98_ANK_8X16_OFFSET + length]);
}

/// Copies the 256 glyphs of the 8x8 ANK font into the first half of each slot.
fn copy_ank_8x8(font_rom: &mut [u8; PC98_CGROM_SIZE], data: &[u8]) {
    for code in 0..ANK_CODE_COUNT {
        let source = V98_ANK_8X8_OFFSET + code * V98_ANK_8X8_GLYPH_BYTES;
        let target = CGROM_ANK_8X8_BASE + code * CGROM_ANK_STRIDE;
        font_rom[target..target + GLYPH_LINES_8]
            .copy_from_slice(&data[source..source + V98_ANK_8X8_GLYPH_BYTES]);
    }
}

/// Copies every 16x16 cell of the ROM rows.
fn copy_kanji(font_rom: &mut [u8; PC98_CGROM_SIZE], data: &[u8]) {
    let rom_rows = JIS_ROWS.filter(|row| !USER_DEFINED_ROWS.contains(row));
    for row in rom_rows {
        for column in JIS_COLUMNS {
            let source = v98_cell_offset(row, column);
            let left = cgrom_cell_offset(row, column);
            let right = left + CGROM_RIGHT_HALF_OFFSET;
            font_rom[left..left + GLYPH_LINES_16]
                .copy_from_slice(&data[source..source + GLYPH_LINES_16]);
            let source_right = source + V98_RIGHT_HALF_OFFSET;
            font_rom[right..right + GLYPH_LINES_16]
                .copy_from_slice(&data[source_right..source_right + GLYPH_LINES_16]);
        }
    }
}

/// Returns the offset of the cell of JIS `row` and `column` in a V98 image.
fn v98_cell_offset(row: u8, column: u8) -> usize {
    let row_index = usize::from(row - JIS_ROWS.start());
    let column_index = usize::from(column - JIS_COLUMNS.start());
    V98_KANJI_OFFSET + row_index * V98_ROW_BYTES + column_index * V98_CELL_BYTES
}

/// Returns the CG-ROM address of line 0 of the left half of JIS `row` and `column`.
fn cgrom_cell_offset(row: u8, column: u8) -> usize {
    usize::from(column) * CGROM_COLUMN_STRIDE + usize::from(row - CODE_ROW_BIAS) * CGROM_ROW_STRIDE
}

/// Writes the 8x16 and 8x8 semigraphics glyphs of all 256 codes.
///
/// The glyphs of code F2h start with blank lines as in the NEC CG-ROM.
fn write_semigraphics(font_rom: &mut [u8; PC98_CGROM_SIZE]) {
    let lines_per_band_16 = GLYPH_LINES_16 / SEMIGRAPHICS_BANDS;
    let lines_per_band_8 = GLYPH_LINES_8 / SEMIGRAPHICS_BANDS;
    for code in 0..ANK_CODE_COUNT {
        let code_byte = code as u8;
        let glyph_start_8x16 = CGROM_SEMIGRAPHICS_8X16_BASE + code * CGROM_ANK_STRIDE;
        for line in 0..GLYPH_LINES_16 {
            font_rom[glyph_start_8x16 + line] =
                semigraphics_line(code_byte, line / lines_per_band_16);
        }
        let glyph_start_8x8 =
            CGROM_ANK_8X8_BASE + code * CGROM_ANK_STRIDE + CGROM_SEMIGRAPHICS_8X8_OFFSET;
        for line in 0..GLYPH_LINES_8 {
            font_rom[glyph_start_8x8 + line] =
                semigraphics_line(code_byte, line / lines_per_band_8);
        }
    }

    let glyph_start_8x16 =
        CGROM_SEMIGRAPHICS_8X16_BASE + SEMIGRAPHICS_BLANK_TOP_CODE * CGROM_ANK_STRIDE;
    font_rom[glyph_start_8x16..glyph_start_8x16 + SEMIGRAPHICS_BLANK_TOP_LINES_16].fill(0);
    let glyph_start_8x8 = CGROM_ANK_8X8_BASE
        + SEMIGRAPHICS_BLANK_TOP_CODE * CGROM_ANK_STRIDE
        + CGROM_SEMIGRAPHICS_8X8_OFFSET;
    font_rom[glyph_start_8x8..glyph_start_8x8 + SEMIGRAPHICS_BLANK_TOP_LINES_8].fill(0);
}

/// Returns the dot pattern of a glyph line in `band` of semigraphics `code`.
fn semigraphics_line(code: u8, band: usize) -> u8 {
    let mut dots = 0;
    if code & (SEMIGRAPHICS_LEFT_BIT << band) != 0 {
        dots |= SEMIGRAPHICS_LEFT_DOTS;
    }
    if code & (SEMIGRAPHICS_RIGHT_BIT << band) != 0 {
        dots |= SEMIGRAPHICS_RIGHT_DOTS;
    }
    dots
}
