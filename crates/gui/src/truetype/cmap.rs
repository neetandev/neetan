//! The `cmap` table: the mapping from characters to glyphs.

use std::cmp::Ordering;

use super::{
    FontError, Tag,
    reader::{Reader, Span, binary_search, slice, tail, u16_at, u32_at},
};

/// The size of a group record of a format 12 subtable.
const GROUP_SIZE: usize = 12;

/// The format of a Unicode subtable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    /// Format 4 with segments of the Basic Multilingual Plane.
    SegmentMapping,
    /// Format 12 with groups for the full Unicode range.
    SegmentedCoverage,
}

/// The selected Unicode subtable of a `cmap` table.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Cmap {
    /// The format of the subtable.
    format: Format,
    /// The bytes of the font data from the subtable to the end of the table.
    subtable: Span,
}

/// A view of the selected subtable.
#[derive(Clone, Copy)]
enum Mapping<'a> {
    /// A format 4 subtable.
    SegmentMapping(SegmentMapping<'a>),
    /// The group records of a format 12 subtable.
    SegmentedCoverage(&'a [u8]),
}

/// The arrays of a format 4 subtable.
#[derive(Clone, Copy)]
struct SegmentMapping<'a> {
    /// The number of segments.
    segment_count: usize,
    /// The last character of each segment.
    end_codes: &'a [u8],
    /// The first character of each segment.
    start_codes: &'a [u8],
    /// The glyph id delta of each segment.
    deltas: &'a [u8],
    /// The range offset array followed by the glyph id array up to the end of
    /// the table. A range offset counts from its own position.
    range_offsets: &'a [u8],
}

impl Cmap {
    /// Selects the Unicode subtable of the `cmap` table at `table` of the
    /// font data `data`. A format 12 subtable for the full Unicode range wins
    /// over a format 4 subtable.
    pub(crate) fn parse(data: &[u8], table: Span) -> Result<Self, FontError> {
        let cmap = table.get(data);
        let mut reader = Reader::new(cmap);
        reader.skip(2)?;
        let table_count = reader.u16()?;
        let mut best: Option<(Format, usize)> = None;
        for _ in 0..table_count {
            let platform = reader.u16()?;
            let encoding = reader.u16()?;
            let offset = reader.u32()? as usize;
            let wanted = match (platform, encoding) {
                (3, 10) | (0, 4) | (0, 6) => (12, Format::SegmentedCoverage),
                (3, 1) | (0, 0..=3) => (4, Format::SegmentMapping),
                _ => continue,
            };
            if u16_at(cmap, offset).ok() != Some(wanted.0) {
                continue;
            }
            let better = match best {
                None => true,
                Some((format, _)) => {
                    format == Format::SegmentMapping && wanted.1 == Format::SegmentedCoverage
                }
            };
            if better {
                best = Some((wanted.1, offset));
            }
        }
        let (format, offset) = best.ok_or(FontError::InvalidTable(Tag::CMAP))?;
        let subtable = Span::new(data, table.offset + offset, table.length - offset)?;
        let cmap = Self { format, subtable };
        cmap.mapping(data)?;
        Ok(cmap)
    }

    /// The view of the subtable in the font data `data`.
    fn mapping<'a>(&self, data: &'a [u8]) -> Result<Mapping<'a>, FontError> {
        let subtable = self.subtable.get(data);
        match self.format {
            Format::SegmentMapping => {
                let segment_count = usize::from(u16_at(subtable, 6)? / 2);
                let array_size = segment_count * 2;
                let end_offset = 14;
                let start_offset = end_offset + array_size + 2;
                let delta_offset = start_offset + array_size;
                let range_offset = delta_offset + array_size;
                Ok(Mapping::SegmentMapping(SegmentMapping {
                    segment_count,
                    end_codes: slice(subtable, end_offset, array_size)?,
                    start_codes: slice(subtable, start_offset, array_size)?,
                    deltas: slice(subtable, delta_offset, array_size)?,
                    range_offsets: tail(subtable, range_offset)?,
                }))
            }
            Format::SegmentedCoverage => {
                let group_count = u32_at(subtable, 12)? as usize;
                let size = group_count
                    .checked_mul(GROUP_SIZE)
                    .ok_or(FontError::Truncated)?;
                Ok(Mapping::SegmentedCoverage(slice(subtable, 16, size)?))
            }
        }
    }

    /// The glyph id of `character` in the font data `data`. A character
    /// without a glyph maps to zero.
    pub(crate) fn glyph(&self, data: &[u8], character: char) -> u32 {
        let code = u32::from(character);
        match self.mapping(data) {
            Ok(Mapping::SegmentMapping(mapping)) => u16::try_from(code)
                .ok()
                .and_then(|code| mapping.glyph(code))
                .map_or(0, u32::from),
            Ok(Mapping::SegmentedCoverage(groups)) => {
                segmented_coverage_glyph(groups, code).unwrap_or(0)
            }
            Err(_) => 0,
        }
    }
}

impl SegmentMapping<'_> {
    /// The glyph id of `code`, or `None` when the data cannot be read.
    fn glyph(&self, code: u16) -> Option<u16> {
        let segment = self.segment(code)?;
        let start = u16_at(self.start_codes, segment * 2).ok()?;
        if code < start {
            return Some(0);
        }
        let delta = u16_at(self.deltas, segment * 2).ok()?;
        let range_offset = usize::from(u16_at(self.range_offsets, segment * 2).ok()?);
        if range_offset == 0 {
            return Some(code.wrapping_add(delta));
        }
        let position = segment * 2 + range_offset + usize::from(code - start) * 2;
        match u16_at(self.range_offsets, position).ok()? {
            0 => Some(0),
            glyph => Some(glyph.wrapping_add(delta)),
        }
    }

    /// The first segment whose end code is at or above `code`.
    fn segment(&self, code: u16) -> Option<usize> {
        let mut low = 0;
        let mut high = self.segment_count;
        while low < high {
            let middle = low + (high - low) / 2;
            if u16_at(self.end_codes, middle * 2).ok()? < code {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        (low < self.segment_count).then_some(low)
    }
}

/// The glyph id of `code` in the format 12 group records `groups`.
fn segmented_coverage_glyph(groups: &[u8], code: u32) -> Option<u32> {
    let group = binary_search(groups.len() / GROUP_SIZE, |index| {
        let start = u32_at(groups, index * GROUP_SIZE).ok()?;
        let end = u32_at(groups, index * GROUP_SIZE + 4).ok()?;
        Some(if end < code {
            Ordering::Less
        } else if start > code {
            Ordering::Greater
        } else {
            Ordering::Equal
        })
    })?;
    let start = u32_at(groups, group * GROUP_SIZE).ok()?;
    let start_glyph = u32_at(groups, group * GROUP_SIZE + 8).ok()?;
    start_glyph.checked_add(code - start)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A `cmap` table with one subtable for each encoding record.
    pub(crate) fn cmap_table(subtables: &[(u16, u16, Vec<u8>)]) -> Vec<u8> {
        let mut table = vec![0, 0];
        table.extend((subtables.len() as u16).to_be_bytes());
        let mut offset = 4 + subtables.len() * 8;
        for (platform, encoding, subtable) in subtables {
            table.extend(platform.to_be_bytes());
            table.extend(encoding.to_be_bytes());
            table.extend((offset as u32).to_be_bytes());
            offset += subtable.len();
        }
        for (_, _, subtable) in subtables {
            table.extend(subtable);
        }
        table
    }

    /// A format 4 subtable from `(start, end, delta, glyphs)` segments. A
    /// segment with glyphs maps through the glyph id array.
    pub(crate) fn format_4(segments: &[(u16, u16, i16, Vec<u16>)]) -> Vec<u8> {
        let count = segments.len() + 1;
        let mut subtable = Vec::new();
        for value in [4, 0, 0, (count * 2) as u16, 0, 0, 0] {
            subtable.extend(u16::to_be_bytes(value));
        }
        for (_, end, _, _) in segments {
            subtable.extend(end.to_be_bytes());
        }
        subtable.extend([0xFF, 0xFF, 0, 0]);
        for (start, _, _, _) in segments {
            subtable.extend(start.to_be_bytes());
        }
        subtable.extend([0xFF, 0xFF]);
        for (_, _, delta, _) in segments {
            subtable.extend(delta.to_be_bytes());
        }
        subtable.extend([0, 1]);
        let mut glyph_array: Vec<u16> = Vec::new();
        for (index, (_, _, _, glyphs)) in segments.iter().enumerate() {
            if glyphs.is_empty() {
                subtable.extend([0, 0]);
            } else {
                let offset = (count - index) * 2 + glyph_array.len() * 2;
                subtable.extend((offset as u16).to_be_bytes());
                glyph_array.extend(glyphs);
            }
        }
        subtable.extend([0, 0]);
        for glyph in glyph_array {
            subtable.extend(glyph.to_be_bytes());
        }
        subtable
    }

    /// A format 12 subtable from `(start, end, start glyph)` groups.
    pub(crate) fn format_12(groups: &[(u32, u32, u32)]) -> Vec<u8> {
        let mut subtable = vec![0, 12, 0, 0];
        subtable.extend(((16 + groups.len() * 12) as u32).to_be_bytes());
        subtable.extend(0u32.to_be_bytes());
        subtable.extend((groups.len() as u32).to_be_bytes());
        for (start, end, glyph) in groups {
            subtable.extend(start.to_be_bytes());
            subtable.extend(end.to_be_bytes());
            subtable.extend(glyph.to_be_bytes());
        }
        subtable
    }

    /// Selects the subtable of the `cmap` table `table`.
    fn parse(table: &[u8]) -> Result<Cmap, FontError> {
        Cmap::parse(
            table,
            Span {
                offset: 0,
                length: table.len(),
            },
        )
    }

    #[test]
    fn format_4_maps_by_delta_and_by_glyph_array() {
        let table = cmap_table(&[(
            3,
            1,
            format_4(&[
                (0x20, 0x7E, -29, vec![]),
                (0x3042, 0x3044, 0, vec![7, 0, 9]),
            ]),
        )]);
        let cmap = parse(&table).unwrap();
        assert_eq!(cmap.glyph(&table, ' '), 3);
        assert_eq!(cmap.glyph(&table, 'A'), 36);
        assert_eq!(cmap.glyph(&table, '\u{7F}'), 0);
        assert_eq!(cmap.glyph(&table, '\u{3042}'), 7);
        assert_eq!(cmap.glyph(&table, '\u{3043}'), 0);
        assert_eq!(cmap.glyph(&table, '\u{3044}'), 9);
        assert_eq!(cmap.glyph(&table, '\u{3045}'), 0);
        assert_eq!(cmap.glyph(&table, '\u{1F600}'), 0);
    }

    #[test]
    fn format_12_wins_over_format_4_and_format_14_is_ignored() {
        let variation_sequences = vec![0, 14, 0, 0, 0, 10, 0, 0, 0, 0];
        let table = cmap_table(&[
            (0, 5, variation_sequences),
            (3, 1, format_4(&[(0x41, 0x41, 1, vec![])])),
            (
                3,
                10,
                format_12(&[(0x41, 0x43, 10), (0x1F600, 0x1F600, 20)]),
            ),
        ]);
        let cmap = parse(&table).unwrap();
        assert_eq!(cmap.glyph(&table, 'A'), 10);
        assert_eq!(cmap.glyph(&table, 'C'), 12);
        assert_eq!(cmap.glyph(&table, 'D'), 0);
        assert_eq!(cmap.glyph(&table, '\u{1F600}'), 20);
        assert_eq!(cmap.glyph(&table, ' '), 0);
    }

    #[test]
    fn the_last_segment_maps_to_no_glyph() {
        let table = cmap_table(&[(0, 3, format_4(&[(0xFFFE, 0xFFFE, 2, vec![])]))]);
        let cmap = parse(&table).unwrap();
        assert_eq!(cmap.glyph(&table, '\u{FFFE}'), 0);
        assert_eq!(cmap.glyph(&table, '\u{FFFD}'), 0);
        assert_eq!(cmap.glyph(&table, '\u{FFFF}'), 0);
        let table = cmap_table(&[(0, 3, format_4(&[(0xFFFD, 0xFFFE, 4, vec![])]))]);
        let cmap = parse(&table).unwrap();
        assert_eq!(cmap.glyph(&table, '\u{FFFD}'), 1);
        assert_eq!(cmap.glyph(&table, '\u{FFFF}'), 0);
    }

    #[test]
    fn a_record_past_the_table_is_skipped() {
        let mut table = cmap_table(&[
            (3, 10, format_12(&[(0x41, 0x41, 7)])),
            (3, 1, format_4(&[(0x41, 0x41, 2, vec![])])),
        ]);
        table[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(parse(&table).unwrap().glyph(&table, 'A'), 0x43);
    }

    #[test]
    fn a_record_with_an_unexpected_format_is_skipped() {
        let table = cmap_table(&[
            (3, 10, format_4(&[(0x41, 0x41, 1, vec![])])),
            (3, 1, format_4(&[(0x41, 0x41, 2, vec![])])),
        ]);
        assert_eq!(parse(&table).unwrap().glyph(&table, 'A'), 0x43);
    }

    #[test]
    fn a_table_without_a_unicode_subtable_fails() {
        let table = cmap_table(&[(1, 0, vec![0, 6, 0, 0])]);
        assert_eq!(
            parse(&table).err(),
            Some(FontError::InvalidTable(Tag::CMAP))
        );
        let table = cmap_table(&[(3, 10, format_12(&[(0x41, 0x43, 10)]))]);
        assert_eq!(
            parse(&table[..table.len() - 1]).err(),
            Some(FontError::Truncated)
        );
    }
}
