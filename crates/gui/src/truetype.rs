//! A reader for TrueType fonts with `glyf` outlines.
//!
//! [`Font`] parses the tables that text layout needs: the metrics, the
//! character to glyph mapping, the advance widths, the pair kerning and the
//! quadratic glyph outlines. Every read checks its bounds. Broken data returns
//! a [`FontError`] and never panics. All values are in font units.

mod cmap;
mod glyf;
mod kerning;
mod reader;

use std::fmt;

use cmap::Cmap;
use glyf::Outlines;
use kerning::Kerning;
use reader::{Reader, Span, i16_at, slice, u16_at, u32_at};

use crate::OutlineSink;

/// The magic number of the `head` table.
const HEAD_MAGIC: u32 = 0x5F0F_3CF5;

/// The sfnt version of a font with TrueType outlines.
const TRUETYPE_VERSION: u32 = 0x0001_0000;

/// The legacy Apple sfnt version of a font with TrueType outlines.
const APPLE_TRUETYPE_VERSION: [u8; 4] = *b"true";

/// The sfnt version of a font with CFF outlines.
const CFF_VERSION: [u8; 4] = *b"OTTO";

/// The tag of a font collection.
const COLLECTION_TAG: [u8; 4] = *b"ttcf";

/// The size of a record of the table directory.
const TABLE_RECORD_SIZE: usize = 16;

/// The `OS/2` version that adds the x height and the cap height.
const OS2_HEIGHTS_VERSION: u16 = 2;

/// A four byte table or feature tag.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Tag(pub [u8; 4]);

impl Tag {
    /// The font header table.
    pub const HEAD: Self = Self(*b"head");
    /// The maximum profile table.
    pub const MAXP: Self = Self(*b"maxp");
    /// The horizontal header table.
    pub const HHEA: Self = Self(*b"hhea");
    /// The horizontal metrics table.
    pub const HMTX: Self = Self(*b"hmtx");
    /// The OS/2 and Windows metrics table.
    pub const OS2: Self = Self(*b"OS/2");
    /// The character to glyph mapping table.
    pub const CMAP: Self = Self(*b"cmap");
    /// The glyph location table.
    pub const LOCA: Self = Self(*b"loca");
    /// The glyph data table.
    pub const GLYF: Self = Self(*b"glyf");
    /// The kerning table.
    pub const KERN: Self = Self(*b"kern");
    /// The glyph positioning table.
    pub const GPOS: Self = Self(*b"GPOS");
    /// The compact font format table.
    pub const CFF: Self = Self(*b"CFF ");
    /// The compact font format 2 table.
    pub const CFF2: Self = Self(*b"CFF2");
}

impl fmt::Display for Tag {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            let character = if byte.is_ascii_graphic() || byte == b' ' {
                char::from(byte)
            } else {
                '?'
            };
            write!(formatter, "{character}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Tag {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Tag({self})")
    }
}

/// The index of a glyph in a font.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GlyphId(pub u16);

impl GlyphId {
    /// The glyph of characters that the font does not map.
    pub const NOTDEF: Self = Self(0);
}

/// An error of the font reader.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontError {
    /// The data ends inside a structure.
    Truncated,
    /// The data does not start with the signature of a TrueType font.
    UnknownSignature,
    /// The data holds a font collection.
    Collection,
    /// The font has CFF outlines.
    CffOutlines,
    /// The font lacks a required table.
    MissingTable(Tag),
    /// A table holds an invalid value.
    InvalidTable(Tag),
    /// The font has no glyph with this id.
    UnknownGlyph(GlyphId),
    /// A glyph outline nests too deep or draws too many components or points.
    OutlineLimit,
}

impl fmt::Display for FontError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => write!(formatter, "the font data is truncated"),
            Self::UnknownSignature => write!(formatter, "the data is not a TrueType font"),
            Self::Collection => write!(formatter, "font collections are not supported"),
            Self::CffOutlines => write!(formatter, "fonts with CFF outlines are not supported"),
            Self::MissingTable(tag) => write!(formatter, "the font has no '{tag}' table"),
            Self::InvalidTable(tag) => write!(formatter, "the '{tag}' table is invalid"),
            Self::UnknownGlyph(glyph) => write!(formatter, "the font has no glyph {}", glyph.0),
            Self::OutlineLimit => write!(formatter, "a glyph outline is too large"),
        }
    }
}

impl std::error::Error for FontError {}

/// The vertical metrics of a font in font units with the y-axis pointing up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Metrics {
    /// The distance from the baseline to the top of the line.
    pub ascent: i16,
    /// The distance from the baseline to the bottom of the line. It is
    /// negative.
    pub descent: i16,
    /// The extra space between two lines.
    pub line_gap: i16,
    /// The height of a flat capital letter.
    pub cap_height: i16,
    /// The height of a flat lowercase letter.
    pub x_height: i16,
}

/// A parsed TrueType font that owns its data.
///
/// The parse validates the tables and keeps their positions. Lookups read the
/// tables in place and allocate nothing.
pub struct Font {
    /// The font file.
    data: Box<[u8]>,
    /// The number of font units in one em.
    units_per_em: u16,
    /// The number of glyphs.
    glyph_count: u16,
    /// The vertical metrics.
    metrics: Metrics,
    /// The `hmtx` table.
    horizontal_metrics: Span,
    /// The number of glyphs with their own advance width in `hmtx`.
    long_metric_count: u16,
    /// The Unicode subtable of `cmap`.
    cmap: Cmap,
    /// The `loca` table.
    locations: Span,
    /// The `glyf` table.
    glyphs: Span,
    /// Whether `loca` holds 32-bit offsets.
    long_offsets: bool,
    /// The pair kerning.
    kerning: Kerning,
}

impl fmt::Debug for Font {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Font")
            .field("size", &self.data.len())
            .field("units_per_em", &self.units_per_em)
            .field("glyph_count", &self.glyph_count)
            .field("long_metric_count", &self.long_metric_count)
            .field("metrics", &self.metrics)
            .field("kerning", &self.kerning)
            .finish()
    }
}

impl Font {
    /// Parses the font file `data` and takes ownership of it. Fonts without
    /// a `glyf` table fail. A broken `OS/2`, GPOS or `kern` table counts as
    /// absent.
    pub fn parse(data: impl Into<Box<[u8]>>) -> Result<Self, FontError> {
        let data = data.into();
        let directory = TableDirectory::parse(&data)?;
        let Some(glyphs) = directory.find(Tag::GLYF)? else {
            if directory.find(Tag::CFF)?.is_some() || directory.find(Tag::CFF2)?.is_some() {
                return Err(FontError::CffOutlines);
            }
            return Err(FontError::MissingTable(Tag::GLYF));
        };

        let head = directory.require(Tag::HEAD)?.get(&data);
        if u32_at(head, 12)? != HEAD_MAGIC {
            return Err(FontError::InvalidTable(Tag::HEAD));
        }
        let units_per_em = u16_at(head, 18)?;
        if units_per_em == 0 {
            return Err(FontError::InvalidTable(Tag::HEAD));
        }
        let long_offsets = match i16_at(head, 50)? {
            0 => false,
            1 => true,
            _ => return Err(FontError::InvalidTable(Tag::HEAD)),
        };

        let glyph_count = u16_at(directory.require(Tag::MAXP)?.get(&data), 4)?;
        if glyph_count == 0 {
            return Err(FontError::InvalidTable(Tag::MAXP));
        }

        let hhea = directory.require(Tag::HHEA)?.get(&data);
        let mut reader = Reader::at(hhea, 4)?;
        let ascent = reader.i16()?;
        let descent = reader.i16()?;
        let line_gap = reader.i16()?;
        let long_metric_count = u16_at(hhea, 34)?;
        if long_metric_count == 0 {
            return Err(FontError::InvalidTable(Tag::HHEA));
        }
        let horizontal_metrics = directory.require(Tag::HMTX)?;
        slice(
            horizontal_metrics.get(&data),
            0,
            usize::from(long_metric_count) * 4,
        )?;

        let (cap_height, x_height) = directory
            .optional(Tag::OS2)
            .and_then(|os2| os2_heights(os2.get(&data)))
            .unwrap_or_else(|| {
                let ascent = i32::from(ascent);
                ((ascent * 7 / 10) as i16, (ascent / 2) as i16)
            });

        let cmap = Cmap::parse(&data, directory.require(Tag::CMAP)?)?;
        let locations = directory.require(Tag::LOCA)?;
        Outlines::new(
            locations.get(&data),
            glyphs.get(&data),
            long_offsets,
            glyph_count,
        )?;
        let kerning = Kerning::parse(
            &data,
            directory.optional(Tag::GPOS),
            directory.optional(Tag::KERN),
        );

        Ok(Self {
            data,
            units_per_em,
            glyph_count,
            metrics: Metrics {
                ascent,
                descent,
                line_gap,
                cap_height,
                x_height,
            },
            horizontal_metrics,
            long_metric_count,
            cmap,
            locations,
            glyphs,
            long_offsets,
            kerning,
        })
    }

    /// The number of font units in one em.
    pub fn units_per_em(&self) -> u16 {
        self.units_per_em
    }

    /// The number of glyphs.
    pub fn glyph_count(&self) -> u16 {
        self.glyph_count
    }

    /// The vertical metrics.
    pub fn metrics(&self) -> Metrics {
        self.metrics
    }

    /// The glyph of `character`, or `None` when the font does not map it.
    pub fn glyph_index(&self, character: char) -> Option<GlyphId> {
        let glyph = self.cmap.glyph(&self.data, character);
        if glyph == 0 || glyph >= u32::from(self.glyph_count) {
            return None;
        }
        Some(GlyphId(glyph as u16))
    }

    /// The advance width of `glyph`. A glyph outside the font has zero.
    pub fn advance_width(&self, glyph: GlyphId) -> u16 {
        if glyph.0 >= self.glyph_count {
            return 0;
        }
        let index = glyph.0.min(self.long_metric_count - 1);
        u16_at(
            self.horizontal_metrics.get(&self.data),
            usize::from(index) * 4,
        )
        .unwrap_or(0)
    }

    /// The left side bearing of `glyph`. A glyph outside the font or without
    /// an entry has zero.
    pub fn left_side_bearing(&self, glyph: GlyphId) -> i16 {
        if glyph.0 >= self.glyph_count {
            return 0;
        }
        let long_count = usize::from(self.long_metric_count);
        let index = usize::from(glyph.0);
        let offset = if index < long_count {
            index * 4 + 2
        } else {
            long_count * 4 + (index - long_count) * 2
        };
        i16_at(self.horizontal_metrics.get(&self.data), offset).unwrap_or(0)
    }

    /// The advance adjustment between `left` and the glyph `right` that
    /// follows it. GPOS pair positioning of the `kern` feature takes the place
    /// of the `kern` table when the font has both.
    pub fn kerning(&self, left: GlyphId, right: GlyphId) -> i16 {
        self.kerning.adjustment(&self.data, left.0, right.0)
    }

    /// Emits the outline of `glyph` to `sink` with the y-axis pointing up.
    /// The outline has quadratic curves only. A glyph without contours emits
    /// nothing.
    pub fn outline(&self, glyph: GlyphId, sink: &mut dyn OutlineSink) -> Result<(), FontError> {
        Outlines::new(
            self.locations.get(&self.data),
            self.glyphs.get(&self.data),
            self.long_offsets,
            self.glyph_count,
        )?
        .draw(glyph.0, sink)
    }
}

/// The cap height and the x height of the `OS/2` table `os2`, or `None` when
/// the table is too short, predates them or holds no values.
fn os2_heights(os2: &[u8]) -> Option<(i16, i16)> {
    if u16_at(os2, 0).ok()? < OS2_HEIGHTS_VERSION {
        return None;
    }
    let x_height = i16_at(os2, 86).ok()?;
    let cap_height = i16_at(os2, 88).ok()?;
    (cap_height > 0 && x_height > 0).then_some((cap_height, x_height))
}

/// The table records of a font file.
struct TableDirectory<'a> {
    /// The font file.
    data: &'a [u8],
    /// The table records.
    records: &'a [u8],
}

impl<'a> TableDirectory<'a> {
    /// Parses the table directory at the start of `data`.
    fn parse(data: &'a [u8]) -> Result<Self, FontError> {
        let mut reader = Reader::new(data);
        let version = reader.tag()?.0;
        match version {
            CFF_VERSION => return Err(FontError::CffOutlines),
            COLLECTION_TAG => return Err(FontError::Collection),
            APPLE_TRUETYPE_VERSION => {}
            _ if u32::from_be_bytes(version) == TRUETYPE_VERSION => {}
            _ => return Err(FontError::UnknownSignature),
        }
        let table_count = reader.u16()?;
        reader.skip(6)?;
        let records = reader.bytes(usize::from(table_count) * TABLE_RECORD_SIZE)?;
        Ok(Self { data, records })
    }

    /// The table with `tag`, or `None` when the font has none. A table past
    /// the end of the data fails.
    fn find(&self, tag: Tag) -> Result<Option<Span>, FontError> {
        for record in self.records.as_chunks::<{ TABLE_RECORD_SIZE }>().0 {
            let mut reader = Reader::new(record);
            if reader.tag()? != tag {
                continue;
            }
            reader.skip(4)?;
            let offset = reader.u32()? as usize;
            let length = reader.u32()? as usize;
            return Span::new(self.data, offset, length).map(Some);
        }
        Ok(None)
    }

    /// The table with `tag`. A missing table fails.
    fn require(&self, tag: Tag) -> Result<Span, FontError> {
        self.find(tag)?.ok_or(FontError::MissingTable(tag))
    }

    /// The optional table with `tag`. A table past the end of the data counts
    /// as absent.
    fn optional(&self, tag: Tag) -> Option<Span> {
        self.find(tag).ok().flatten()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{
        glyf::tests::{Recorder, simple_glyph},
        *,
    };
    use crate::fonts::tests::{font, font_data};

    /// A font file from tables of tag and data.
    pub(crate) fn font_file(tables: &[(Tag, Vec<u8>)]) -> Vec<u8> {
        let mut file = TRUETYPE_VERSION.to_be_bytes().to_vec();
        file.extend((tables.len() as u16).to_be_bytes());
        file.extend([0; 6]);
        let mut offset = 12 + tables.len() * 16;
        for (tag, data) in tables {
            file.extend(tag.0);
            file.extend([0; 4]);
            file.extend((offset as u32).to_be_bytes());
            file.extend((data.len() as u32).to_be_bytes());
            offset += data.len().next_multiple_of(4);
        }
        for (_, data) in tables {
            file.extend(data);
            file.resize(file.len().next_multiple_of(4), 0);
        }
        file
    }

    /// The tables of a small font with three glyphs: an empty `.notdef`, a
    /// square for `A` and a triangle for `B`. `hhea` lists two long metrics.
    pub(crate) fn small_font_tables() -> Vec<(Tag, Vec<u8>)> {
        let mut head = vec![0; 54];
        head[12..16].copy_from_slice(&HEAD_MAGIC.to_be_bytes());
        head[18..20].copy_from_slice(&1000u16.to_be_bytes());
        head[50..52].copy_from_slice(&1i16.to_be_bytes());
        let mut maxp = vec![0, 0, 0x50, 0];
        maxp.extend(3u16.to_be_bytes());
        let mut hhea = vec![0; 36];
        hhea[4..6].copy_from_slice(&800i16.to_be_bytes());
        hhea[6..8].copy_from_slice(&(-200i16).to_be_bytes());
        hhea[8..10].copy_from_slice(&50i16.to_be_bytes());
        hhea[34..36].copy_from_slice(&2u16.to_be_bytes());
        let mut hmtx = Vec::new();
        for value in [500u16, 0, 600, 20, 30] {
            hmtx.extend(value.to_be_bytes());
        }
        let square = simple_glyph(&[&[
            (0, 0, true),
            (100, 0, true),
            (100, 100, true),
            (0, 100, true),
        ]]);
        let triangle = simple_glyph(&[&[(0, 0, true), (100, 0, true), (0, 100, true)]]);
        let (loca, glyf) = glyf::tests::glyph_tables(&[Vec::new(), square, triangle]);
        let cmap = cmap::tests::cmap_table(&[(
            3,
            1,
            cmap::tests::format_4(&[(0x41, 0x42, -0x40, vec![])]),
        )]);
        vec![
            (Tag::HEAD, head),
            (Tag::MAXP, maxp),
            (Tag::HHEA, hhea),
            (Tag::HMTX, hmtx),
            (Tag::CMAP, cmap),
            (Tag::LOCA, loca),
            (Tag::GLYF, glyf),
        ]
    }

    /// A `kern` table with one format 0 subtable of the pairs `(left, right,
    /// value)`, sorted by the glyph pair.
    pub(crate) fn kern_table(pairs: &[(u16, u16, i16)]) -> Vec<u8> {
        let mut kern = vec![0, 0, 0, 1, 0, 0];
        kern.extend((14 + pairs.len() as u16 * 6).to_be_bytes());
        kern.extend([0, 1]);
        kern.extend((pairs.len() as u16).to_be_bytes());
        kern.extend([0; 6]);
        for &(left, right, value) in pairs {
            kern.extend(left.to_be_bytes());
            kern.extend(right.to_be_bytes());
            kern.extend(value.to_be_bytes());
        }
        kern
    }

    /// The characters that the font tests look up.
    const SAMPLE: &str =
        "AVTo L\u{C4}-.,\u{218}\u{219}\u{1EAB}\u{490}あア漢字\u{D55C}\u{1F100}\u{20B9F}\u{FFFF}";

    /// Calls every lookup of `font` for every `step`-th glyph and the sample
    /// characters. Broken data may fail but never panics.
    fn exercise(font: &Font, step: usize) {
        let mut recorder = Recorder::default();
        let sampled = SAMPLE
            .chars()
            .filter_map(|character| font.glyph_index(character));
        let stepped = (0..font.glyph_count()).step_by(step).map(GlyphId);
        for glyph in sampled.chain(stepped) {
            font.advance_width(glyph);
            font.left_side_bearing(glyph);
            font.kerning(glyph, GlyphId(glyph.0 / 2));
            font.kerning(GlyphId(glyph.0 / 2), glyph);
            recorder.commands.clear();
            let _ = font.outline(glyph, &mut recorder);
        }
    }

    /// The tables of the small font with an `OS/2` table `os2`.
    fn with_os2(os2: Vec<u8>) -> Metrics {
        let mut tables = small_font_tables();
        tables.push((Tag::OS2, os2));
        Font::parse(font_file(&tables)).unwrap().metrics()
    }

    /// An `OS/2` table of `version` with the x height and the cap height.
    fn os2_table(version: u16, x_height: i16, cap_height: i16) -> Vec<u8> {
        let mut os2 = vec![0; 96];
        os2[0..2].copy_from_slice(&version.to_be_bytes());
        os2[86..88].copy_from_slice(&x_height.to_be_bytes());
        os2[88..90].copy_from_slice(&cap_height.to_be_bytes());
        os2
    }

    #[test]
    fn a_small_font_parses() {
        let file = font_file(&small_font_tables());
        let font = Font::parse(file.as_slice()).unwrap();
        assert_eq!(font.units_per_em(), 1000);
        assert_eq!(font.glyph_count(), 3);
        assert_eq!(
            font.metrics(),
            Metrics {
                ascent: 800,
                descent: -200,
                line_gap: 50,
                cap_height: 560,
                x_height: 400,
            }
        );
        assert_eq!(font.glyph_index('A'), Some(GlyphId(1)));
        assert_eq!(font.glyph_index('B'), Some(GlyphId(2)));
        assert_eq!(font.glyph_index('C'), None);
        assert_eq!(font.advance_width(GlyphId(1)), 600);
        assert_eq!(font.advance_width(GlyphId(2)), 600);
        assert_eq!(font.advance_width(GlyphId(3)), 0);
        assert_eq!(font.left_side_bearing(GlyphId(1)), 20);
        assert_eq!(font.left_side_bearing(GlyphId(2)), 30);
        assert_eq!(font.kerning(GlyphId(1), GlyphId(2)), 0);

        let mut recorder = Recorder::default();
        font.outline(GlyphId(0), &mut recorder).unwrap();
        assert!(recorder.commands.is_empty());
        font.outline(GlyphId(2), &mut recorder).unwrap();
        assert_eq!(recorder.contours(), 1);
        assert_eq!(
            font.outline(GlyphId(3), &mut recorder),
            Err(FontError::UnknownGlyph(GlyphId(3)))
        );
    }

    #[test]
    fn the_kern_table_applies_without_gpos() {
        let mut tables = small_font_tables();
        tables.push((Tag::KERN, kern_table(&[(1, 2, -80)])));
        let file = font_file(&tables);
        let font = Font::parse(file.as_slice()).unwrap();
        assert_eq!(font.kerning(GlyphId(1), GlyphId(2)), -80);
        assert_eq!(font.kerning(GlyphId(2), GlyphId(1)), 0);
    }

    #[test]
    fn unsupported_fonts_fail() {
        let without = |tag: Tag| {
            let tables: Vec<_> = small_font_tables()
                .into_iter()
                .filter(|(table, _)| *table != tag)
                .collect();
            Font::parse(font_file(&tables)).err()
        };
        assert_eq!(without(Tag::GLYF), Some(FontError::MissingTable(Tag::GLYF)));
        assert_eq!(without(Tag::HMTX), Some(FontError::MissingTable(Tag::HMTX)));
        assert_eq!(without(Tag::CMAP), Some(FontError::MissingTable(Tag::CMAP)));

        let mut tables: Vec<_> = small_font_tables()
            .into_iter()
            .filter(|(table, _)| *table != Tag::GLYF && *table != Tag::LOCA)
            .collect();
        tables.push((Tag::CFF, vec![1, 0, 4, 4]));
        assert_eq!(
            Font::parse(font_file(&tables)).err(),
            Some(FontError::CffOutlines)
        );

        let mut file = font_file(&small_font_tables());
        file[0..4].copy_from_slice(b"OTTO");
        assert_eq!(
            Font::parse(file.as_slice()).err(),
            Some(FontError::CffOutlines)
        );
        file[0..4].copy_from_slice(b"ttcf");
        assert_eq!(
            Font::parse(file.as_slice()).err(),
            Some(FontError::Collection)
        );
        file[0..4].copy_from_slice(b"wOFF");
        assert_eq!(
            Font::parse(file.as_slice()).err(),
            Some(FontError::UnknownSignature)
        );
        file[0..4].copy_from_slice(b"true");
        assert!(Font::parse(file.as_slice()).is_ok());
    }

    #[test]
    fn invalid_header_values_fail() {
        let with_head = |position: usize, bytes: [u8; 2]| {
            let mut tables = small_font_tables();
            tables[0].1[position..position + 2].copy_from_slice(&bytes);
            Font::parse(font_file(&tables)).err()
        };
        assert_eq!(
            with_head(12, [0, 0]),
            Some(FontError::InvalidTable(Tag::HEAD))
        );
        assert_eq!(
            with_head(18, [0, 0]),
            Some(FontError::InvalidTable(Tag::HEAD))
        );
        assert_eq!(
            with_head(50, [0, 2]),
            Some(FontError::InvalidTable(Tag::HEAD))
        );
    }

    #[test]
    fn the_os2_heights_fall_back_to_the_ascent() {
        let fallback = (560, 400);
        let heights = |metrics: Metrics| (metrics.cap_height, metrics.x_height);
        assert_eq!(heights(with_os2(os2_table(2, 450, 650))), (650, 450));
        assert_eq!(heights(with_os2(os2_table(1, 450, 650))), fallback);
        assert_eq!(heights(with_os2(os2_table(4, 0, 650))), fallback);
        assert_eq!(heights(with_os2(os2_table(4, 450, 0))), fallback);
        assert_eq!(
            heights(with_os2(os2_table(4, 450, 650)[..80].to_vec())),
            fallback
        );
        assert_eq!(heights(with_os2(vec![0])), fallback);
    }

    #[test]
    fn broken_optional_tables_count_as_absent() {
        let mut tables = small_font_tables();
        tables.push((Tag::OS2, os2_table(4, 450, 650)));
        tables.push((Tag::GPOS, vec![0, 1, 0, 0, 0xFF]));
        let mut kern = vec![0, 0, 0, 1, 0, 0, 0, 20, 0, 1];
        kern.extend([0, 1, 0, 0, 0, 0, 0, 0]);
        kern.extend([0, 1, 0, 2]);
        kern.extend((-80i16).to_be_bytes());
        tables.push((Tag::KERN, kern));
        let mut file = font_file(&tables);
        let font = Font::parse(file.as_slice()).unwrap();
        assert_eq!(font.kerning(GlyphId(1), GlyphId(2)), -80);
        assert_eq!(font.metrics().cap_height, 650);

        let record =
            |tag: Tag| 12 + tables.iter().position(|(table, _)| *table == tag).unwrap() * 16;
        for tag in [Tag::OS2, Tag::GPOS, Tag::KERN] {
            let offset = record(tag) + 8;
            file[offset..offset + 4].copy_from_slice(&u32::MAX.to_be_bytes());
        }
        let font = Font::parse(file.as_slice()).unwrap();
        assert_eq!(font.kerning(GlyphId(1), GlyphId(2)), 0);
        assert_eq!(font.metrics().cap_height, 560);
    }

    #[test]
    fn small_fonts_with_corruption_never_panic() {
        let mut tables = small_font_tables();
        tables.push((Tag::OS2, os2_table(4, 450, 650)));
        tables.push((Tag::GPOS, kerning::tests::single_pair_gpos(1, 2, -30)));
        let file = font_file(&tables);
        assert_eq!(
            Font::parse(file.as_slice())
                .unwrap()
                .kerning(GlyphId(1), GlyphId(2)),
            -30
        );
        for length in 0..file.len() {
            if let Ok(font) = Font::parse(&file[..length]) {
                exercise(&font, 1);
            }
        }
        let mut random = random_numbers();
        for _ in 0..20000 {
            let mut data = file.clone();
            for _ in 0..1 + random() % 4 {
                let position = (random() % data.len() as u64) as usize;
                data[position] = random() as u8;
            }
            if let Ok(font) = Font::parse(data) {
                exercise(&font, 1);
            }
        }
    }

    #[test]
    fn the_font_reads() {
        let font = font();
        assert_eq!(font.units_per_em(), 1000);
        assert_eq!(font.glyph_count(), 49814);
        let debug = format!("{font:?}");
        assert!(
            debug.contains("glyph_count: 49814") && debug.len() < 400,
            "{debug}"
        );
        assert_eq!(
            font.metrics(),
            Metrics {
                ascent: 969,
                descent: -241,
                line_gap: 0,
                cap_height: 727,
                x_height: 545,
            }
        );
        let glyphs = [
            ('A', 2),
            ('\u{C4}', 3),
            (' ', 1411),
            ('\u{218}', 1897),
            ('\u{219}', 1903),
            ('\u{1EAB}', 517),
            ('\u{490}', 1058),
            ('\u{3042}', 3553),
            ('\u{30A2}', 3647),
            ('\u{FF71}', 3422),
            ('\u{6F22}', 34356),
            ('\u{D55C}', 15940),
            ('\u{1F100}', 4331),
            ('\u{20B9F}', 47653),
        ];
        for (character, glyph) in glyphs {
            assert_eq!(
                font.glyph_index(character),
                Some(GlyphId(glyph)),
                "{character}"
            );
        }
        assert_eq!(font.glyph_index('\u{FFFF}'), None);
        assert_eq!(font.advance_width(GlyphId(1411)), 269);
        assert_eq!(font.advance_width(GlyphId(2)), 690);
        assert_eq!(font.left_side_bearing(GlyphId(2)), 25);

        let kanji = GlyphId(34356);
        assert_eq!(font.advance_width(kanji), 1000);
        assert_eq!(font.left_side_bearing(kanji), 38);
        let last = GlyphId(font.glyph_count() - 1);
        assert_eq!(font.advance_width(last), 1000);
        assert_eq!(font.left_side_bearing(last), 43);

        let mut recorder = Recorder::default();
        font.outline(GlyphId::NOTDEF, &mut recorder).unwrap();
        assert_eq!(recorder.contours(), 10);
        for character in "あア漢字\u{D55C}\u{1EAB}".chars() {
            let mut recorder = Recorder::default();
            font.outline(font.glyph_index(character).unwrap(), &mut recorder)
                .unwrap();
            assert!(recorder.contours() > 0, "{character} has no outline");
        }
    }

    #[test]
    fn composite_glyphs_draw_their_components() {
        let font = font();
        let draw = |glyph: u16| {
            let mut recorder = Recorder::default();
            font.outline(GlyphId(glyph), &mut recorder).unwrap();
            recorder
        };
        let letter = draw(2);
        let dieresis = draw(2506);
        let composite = draw(3);
        assert_eq!(
            composite.contours(),
            letter.contours() + dieresis.contours()
        );
        let mut expected = letter.commands.clone();
        expected.extend(dieresis.commands.iter().map(|command| {
            let shift = |[x, y]: [f32; 2]| [x + 49.0, y + 182.0];
            match *command {
                glyf::tests::Command::Move(point) => glyf::tests::Command::Move(shift(point)),
                glyf::tests::Command::Line(point) => glyf::tests::Command::Line(shift(point)),
                glyf::tests::Command::Quad(control, point) => {
                    glyf::tests::Command::Quad(shift(control), shift(point))
                }
                glyf::tests::Command::Close => glyf::tests::Command::Close,
            }
        }));
        assert_eq!(composite.commands, expected);
    }

    #[test]
    fn known_pairs_kern() {
        let font = font();
        let pair = |text: &str| {
            let mut characters = text.chars();
            let left = font.glyph_index(characters.next().unwrap()).unwrap();
            let right = font.glyph_index(characters.next().unwrap()).unwrap();
            font.kerning(left, right)
        };
        assert_eq!(pair("AV"), -68);
        assert_eq!(pair("To"), -78);
        assert_eq!(pair("LT"), -96);
        assert_eq!(pair("oo"), 0);
    }

    #[test]
    fn truncated_fonts_fail() {
        let data = font_data();
        let directory = TableDirectory::parse(data).unwrap();
        let used = [
            Tag::HEAD,
            Tag::MAXP,
            Tag::HHEA,
            Tag::HMTX,
            Tag::CMAP,
            Tag::LOCA,
            Tag::GLYF,
        ];
        let ends: Vec<_> = used
            .iter()
            .map(|tag| {
                let span = directory.require(*tag).unwrap();
                span.offset + span.length
            })
            .collect();
        let required_end = *ends.iter().max().unwrap();
        let mut lengths = vec![0, 4, 11, 12, 100];
        lengths.extend(ends.iter().map(|end| end - 1));
        lengths.extend((1..20).map(|part| required_end * part / 20));
        for length in lengths {
            assert!(Font::parse(&data[..length]).is_err(), "length {length}");
        }
        exercise(&Font::parse(&data[..required_end]).unwrap(), 97);
    }

    /// A generator of pseudo random numbers.
    fn random_numbers() -> impl FnMut() -> u64 {
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        }
    }

    #[test]
    fn corrupted_headers_cmap_gpos_lookups_and_pair_sets_never_panic() {
        let data = font_data();
        let directory = TableDirectory::parse(data).unwrap();
        let mut regions = vec![Span {
            offset: 0,
            length: 12 + 16 * usize::from(u16_at(data, 4).unwrap()),
        }];
        for tag in [
            Tag::HEAD,
            Tag::MAXP,
            Tag::HHEA,
            Tag::OS2,
            Tag::CMAP,
            Tag::GPOS,
        ] {
            let span = directory.require(tag).unwrap();
            regions.push(Span {
                offset: span.offset,
                length: span.length.min(4096),
            });
        }
        let gpos = directory.require(Tag::GPOS).unwrap();
        let lookup_list = gpos.offset + usize::from(u16_at(data, gpos.offset + 8).unwrap());
        regions.push(Span {
            offset: lookup_list,
            length: 4096,
        });
        for index in [1, 6, 7, 8, 20] {
            let lookup =
                lookup_list + usize::from(u16_at(data, lookup_list + 2 + index * 2).unwrap());
            let subtable = lookup + usize::from(u16_at(data, lookup + 6).unwrap());
            let pairs = if u16_at(data, lookup).unwrap() == 9 {
                subtable + u32_at(data, subtable + 4).unwrap() as usize
            } else {
                subtable
            };
            regions.push(Span {
                offset: lookup,
                length: 256,
            });
            regions.push(Span {
                offset: pairs,
                length: 8192,
            });
        }
        let mut random = random_numbers();
        let mut data = data.to_vec();
        for _ in 0..300 {
            let region = regions[(random() % regions.len() as u64) as usize];
            let mut changes = Vec::new();
            for _ in 0..1 + random() % 4 {
                let position = region.offset + (random() % region.length as u64) as usize;
                changes.push((position, data[position]));
                data[position] = random() as u8;
            }
            if let Ok(font) = Font::parse(data.as_slice()) {
                exercise(&font, 997);
            }
            for (position, value) in changes.into_iter().rev() {
                data[position] = value;
            }
        }
    }
}
