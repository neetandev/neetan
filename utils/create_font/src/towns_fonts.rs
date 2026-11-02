//! FM Towns fonts: the layout of the font ROM (`FMT_FNT.ROM`) and of the 12-dot
//! font area of the system ROM, and the build of both images from the `.bit`
//! files.
//!
//! The font ROM holds 8192 cells of 32 bytes. A 16x16 cell stores row `r` in
//! byte `2r` (left 8 pixels) and byte `2r + 1` (right 8 pixels). A cell
//! without a glyph holds the fill pattern `00 01` on every row. The 8x8 ANK
//! font starts at `3D000h`, the 8x16 ANK font at `3D800h`.
//!
//! The 12-dot area starts at offset 0 of the system ROM. It holds the 8x12 ANK
//! font (12 bytes per code) and the 12x12 JIS font from `C00h` (rows
//! `21h`-`4Fh`, 94 cells per row, 24 bytes per glyph). Every other byte is zero.

use std::path::Path;

use crate::bdf::BitmapFont;

/// Size of the font ROM image.
const FONT_ROM_SIZE: usize = 0x4_0000;
/// Bytes of one font ROM cell.
const CELL_BYTES: usize = 32;
/// Number of cells in the font ROM.
const CELL_COUNT: usize = FONT_ROM_SIZE / CELL_BYTES;
/// Pattern of a font ROM cell without a glyph: `00 01` on every row.
const FILL_CELL: [u8; CELL_BYTES] = {
    let mut cell = [0u8; CELL_BYTES];
    let mut byte = 1;
    while byte < CELL_BYTES {
        cell[byte] = 1;
        byte += 2;
    }
    cell
};
/// Offset of the 8x8 ANK font in the font ROM.
const ANK_8X8_OFFSET: usize = 0x3_D000;
/// Offset of the 8x16 ANK font in the font ROM.
const ANK_8X16_OFFSET: usize = 0x3_D800;
/// First cell index of the two ANK fonts.
const ANK_FIRST_INDEX: usize = 0x1E80;
/// Cell index after the two ANK fonts.
const ANK_END_INDEX: usize = 0x1F40;
/// First cell index of the Fujitsu glyphs that no character code reaches.
const FUJITSU_FIRST_INDEX: usize = 0x0300;
/// Cell index after the Fujitsu glyphs.
const FUJITSU_END_INDEX: usize = 0x0400;
/// Cell index of the Neetan tripwire glyph. The original ROM fills this cell.
const TRIPWIRE_INDEX: usize = 0x1FFF;
/// Size of the 12-dot area that the build writes.
const FONT12_SIZE: usize = 0x1_B000;
/// Bytes of one 8x12 ANK glyph.
const FONT12_ANK_BYTES: usize = 12;
/// Offset of the 12x12 JIS font in the 12-dot area.
const FONT12_JIS_OFFSET: usize = 0x0C00;
/// Bytes of one 12x12 JIS glyph.
const FONT12_JIS_BYTES: usize = 24;
/// Last JIS row of the 12x12 font.
const FONT12_LAST_ROW: u8 = 0x4F;

/// Meaning of the `ENCODING` value of a font set.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum EncodingKind {
    /// The JIS code `H * 256 + L`.
    Jis,
    /// The ANK code.
    Ank,
    /// The font ROM cell index.
    RomIndex,
}

/// One `.bit` file of the FM Towns fonts.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FontSet {
    /// 16x16 glyphs of the JIS rows `21h`-`28h`.
    NonKanji,
    /// 16x16 level 1 kanji, rows `30h`-`4Fh`.
    Kanji1,
    /// 16x16 level 2 kanji, rows `50h`-`74h`.
    Kanji2,
    /// 16x16 Fujitsu glyphs at the cells `300h`-`3FFh`.
    Fujitsu,
    /// 8x8 ANK font of the font ROM.
    Ank8x8,
    /// 8x16 ANK font of the font ROM.
    Ank8x16,
    /// 8x12 ANK font of the 12-dot area.
    Ank12,
    /// 12x12 JIS font of the 12-dot area.
    Jis12,
    /// The Neetan tripwire glyph.
    Tripwire,
}

impl FontSet {
    /// File name of the set in the font directory.
    fn file_name(self) -> &'static str {
        match self {
            FontSet::NonKanji => "towns_16_nonkanji.bit",
            FontSet::Kanji1 => "towns_16_kanji1.bit",
            FontSet::Kanji2 => "towns_16_kanji2.bit",
            FontSet::Fujitsu => "towns_16_fujitsu.bit",
            FontSet::Ank8x8 => "towns_ank_8x8.bit",
            FontSet::Ank8x16 => "towns_ank_8x16.bit",
            FontSet::Ank12 => "towns_12_ank.bit",
            FontSet::Jis12 => "towns_12_jis.bit",
            FontSet::Tripwire => "towns_tripwire.bit",
        }
    }

    /// Meaning of the `ENCODING` values.
    fn encoding_kind(self) -> EncodingKind {
        match self {
            FontSet::NonKanji | FontSet::Kanji1 | FontSet::Kanji2 | FontSet::Jis12 => {
                EncodingKind::Jis
            }
            FontSet::Ank8x8 | FontSet::Ank8x16 | FontSet::Ank12 => EncodingKind::Ank,
            FontSet::Fujitsu | FontSet::Tripwire => EncodingKind::RomIndex,
        }
    }
}

/// Returns the font ROM cell index of a JIS code with a low byte in
/// `20h`-`7Fh`.
fn glyph_index(code: u32) -> usize {
    let jis_high = (code >> 8) as u8 & 0x7F;
    let jis_low = code as u8;
    let column = usize::from(jis_low & 0x1F);
    let cell_block = usize::from(jis_low.wrapping_sub(0x20) & 0x60);
    let swapped_block = ((cell_block & 0x20) << 4) | ((cell_block & 0x40) << 2);
    let index = match jis_high {
        0x00..=0x2F => swapped_block + usize::from(jis_high & 0x07) * 0x20 + column,
        0x30..=0x6F => {
            let row_group = usize::from((jis_high - 0x30) >> 4);
            0x400
                + (row_group * 3 + (cell_block >> 5)) * 0x200
                + usize::from(jis_high & 0x0F) * 0x20
                + column
        }
        _ => 0x1C00 + swapped_block + usize::from(jis_high & 0x07) * 0x20 + column,
    };
    index & (CELL_COUNT - 1)
}

/// Returns the offset of a JIS code in the 12x12 font of the 12-dot area.
fn font12_jis_offset(code: u32) -> usize {
    let jis_high = (code >> 8) as usize;
    let jis_low = (code & 0xFF) as usize;
    FONT12_JIS_OFFSET + ((jis_high - 0x21) * 94 + (jis_low - 0x21)) * FONT12_JIS_BYTES
}

/// Reads and parses the `.bit` file of a set.
fn load_set(fonts: &Path, set: FontSet) -> Result<BitmapFont, String> {
    BitmapFont::load(&fonts.join(set.file_name()))
}

/// Returns the font ROM cell index of a 16x16 glyph, checked against the
/// cells that its set may fill.
fn cell_index_16(set: FontSet, encoding: u32) -> Result<usize, String> {
    let row = encoding >> 8;
    let cell = encoding & 0xFF;
    let valid = match set {
        FontSet::NonKanji => (0x21..=0x28).contains(&row),
        FontSet::Kanji1 => (0x30..=0x4F).contains(&row),
        FontSet::Kanji2 => (0x50..=0x77).contains(&row),
        FontSet::Fujitsu => (FUJITSU_FIRST_INDEX..FUJITSU_END_INDEX).contains(&(encoding as usize)),
        FontSet::Tripwire => encoding as usize == TRIPWIRE_INDEX,
        FontSet::Ank8x8 | FontSet::Ank8x16 | FontSet::Ank12 | FontSet::Jis12 => false,
    };
    let jis = set.encoding_kind() == EncodingKind::Jis;
    if !valid || (jis && !(0x20..=0x7F).contains(&cell)) {
        return Err(format!("{}: invalid ENCODING {encoding}", set.file_name()));
    }
    let index = if jis {
        glyph_index(encoding)
    } else {
        encoding as usize
    };
    if jis && (ANK_FIRST_INDEX..ANK_END_INDEX).contains(&index) {
        return Err(format!(
            "{}: ENCODING {encoding} hits the ANK fonts",
            set.file_name()
        ));
    }
    Ok(index)
}

/// Builds the 256 KiB font ROM from the `.bit` files in `fonts`.
pub fn build_font_rom(fonts: &Path) -> Result<Vec<u8>, String> {
    let mut rom = FILL_CELL.repeat(CELL_COUNT);
    for set in [
        FontSet::NonKanji,
        FontSet::Kanji1,
        FontSet::Kanji2,
        FontSet::Fujitsu,
        FontSet::Tripwire,
    ] {
        let font = load_set(fonts, set)?;
        for encoding in font.encodings() {
            let index = cell_index_16(set, encoding)?;
            let glyph = font
                .get_16x16_rows(encoding)
                .ok_or_else(|| format!("{}: glyph {encoding} is broken", set.file_name()))?;
            rom[index * CELL_BYTES..(index + 1) * CELL_BYTES].copy_from_slice(&glyph);
        }
    }
    let ank_8x8 = load_set(fonts, FontSet::Ank8x8)?;
    let ank_8x16 = load_set(fonts, FontSet::Ank8x16)?;
    for code in 0..256u32 {
        let missing = |set: FontSet| format!("{}: ANK code {code} is missing", set.file_name());
        let glyph = ank_8x8
            .get_8x8(code)
            .ok_or_else(|| missing(FontSet::Ank8x8))?;
        let offset = ANK_8X8_OFFSET + code as usize * 8;
        rom[offset..offset + 8].copy_from_slice(&glyph);
        let glyph = ank_8x16
            .get_8x16(code)
            .ok_or_else(|| missing(FontSet::Ank8x16))?;
        let offset = ANK_8X16_OFFSET + code as usize * 16;
        rom[offset..offset + 16].copy_from_slice(&glyph);
    }
    Ok(rom)
}

/// Builds the 12-dot area of the system ROM (`FONT12_SIZE` bytes) from the
/// `.bit` files in `fonts`.
pub fn build_font12(fonts: &Path) -> Result<Vec<u8>, String> {
    let mut area = vec![0u8; FONT12_SIZE];
    let ank = load_set(fonts, FontSet::Ank12)?;
    for code in 0..256u32 {
        let glyph = ank
            .get_8x12(code)
            .ok_or_else(|| format!("{}: ANK code {code} is missing", FontSet::Ank12.file_name()))?;
        let offset = code as usize * FONT12_ANK_BYTES;
        area[offset..offset + FONT12_ANK_BYTES].copy_from_slice(&glyph);
    }
    let jis = load_set(fonts, FontSet::Jis12)?;
    for encoding in jis.encodings() {
        let row = encoding >> 8;
        let cell = encoding & 0xFF;
        if !(0x21..=u32::from(FONT12_LAST_ROW)).contains(&row) || !(0x21..=0x7E).contains(&cell) {
            return Err(format!(
                "{}: invalid ENCODING {encoding}",
                FontSet::Jis12.file_name()
            ));
        }
        let glyph = jis
            .get_12x12(encoding)
            .ok_or_else(|| format!("{}: glyph {encoding} is broken", FontSet::Jis12.file_name()))?;
        let offset = font12_jis_offset(encoding);
        area[offset..offset + FONT12_JIS_BYTES].copy_from_slice(&glyph);
    }
    Ok(area)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// Offset of the tripwire cell in the font ROM.
    const TRIPWIRE_OFFSET: usize = TRIPWIRE_INDEX * CELL_BYTES;

    /// Directory of the FM Towns `.bit` files.
    fn fonts_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../bios_towns/fonts")
    }

    /// Reads a file of `utils/bios_towns`.
    fn read_bios_towns(name: &str) -> Vec<u8> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../bios_towns")
            .join(name);
        std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
    }

    #[test]
    fn glyph_index_follows_the_cell_blocks() {
        assert_eq!(glyph_index(0x2121), 0x0021);
        assert_eq!(glyph_index(0x2821), 0x0001);
        assert_eq!(glyph_index(0x2840), 0x0200);
        assert_eq!(glyph_index(0x2860), 0x0100);
        assert_eq!(glyph_index(0x3021), 0x0401);
        assert_eq!(glyph_index(0x4F53), 0x0DF3);
        assert_eq!(glyph_index(0x7060), 0x1D00);
        assert_eq!(glyph_index(0x7426), 0x1C86);
        assert_eq!(glyph_index(0x7440), ANK_FIRST_INDEX);
    }

    #[test]
    fn font12_offsets_pack_94_cells_per_row() {
        assert_eq!(font12_jis_offset(0x2121), 0x0C00);
        assert_eq!(font12_jis_offset(0x2221), 0x0C00 + 94 * 24);
        assert_eq!(font12_jis_offset(0x4F7E), 0x1_AA18);
    }

    #[test]
    fn committed_images_match_a_build_of_the_bit_files() {
        let font_rom = build_font_rom(&fonts_dir()).unwrap();
        assert!(font_rom == read_bios_towns("fonts/font.rom"));
        let font12 = build_font12(&fonts_dir()).unwrap();
        assert!(font12 == read_bios_towns("sysrom.rom")[..FONT12_SIZE]);
    }

    #[test]
    fn tripwire_cell_holds_the_neetan_glyph() {
        let font_rom = read_bios_towns("fonts/font.rom");
        let cell = &font_rom[TRIPWIRE_OFFSET..TRIPWIRE_OFFSET + CELL_BYTES];
        let tripwire = load_set(&fonts_dir(), FontSet::Tripwire).unwrap();
        assert_eq!(tripwire.encodings(), vec![TRIPWIRE_INDEX as u32]);
        assert_eq!(
            cell,
            tripwire.get_16x16_rows(TRIPWIRE_INDEX as u32).unwrap()
        );
        assert_ne!(cell, FILL_CELL);
    }
}
