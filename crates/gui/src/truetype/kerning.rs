//! Pair kerning from the `kern` table and from GPOS pair positioning.

use std::{cmp::Ordering, fmt};

use super::{
    FontError, Tag,
    reader::{Span, binary_search, i16_at, slice, u16_at, u32_at},
};

/// The tag of the kerning feature.
const KERN_FEATURE: Tag = Tag(*b"kern");

/// The scripts whose kerning feature the reader prefers, in order.
const PREFERRED_SCRIPTS: [Tag; 2] = [Tag(*b"DFLT"), Tag(*b"latn")];

/// The GPOS lookup type of pair adjustments.
const PAIR_ADJUSTMENT: u16 = 2;

/// The GPOS lookup type of extension subtables.
const EXTENSION: u16 = 9;

/// The feature index of a language system without a required feature.
const NO_REQUIRED_FEATURE: u16 = 0xFFFF;

/// The most pair adjustment subtables of the kerning feature. A font with
/// more has no GPOS kerning.
const MAX_PAIR_SUBTABLES: usize = 1024;

/// The most records that the GPOS parse reads from the script, feature and
/// lookup lists. A font that needs more has no GPOS kerning.
const MAX_GPOS_STEPS: usize = 1 << 20;

/// The most pair records of all format 1 pair adjustment subtables of the
/// kerning feature. A font with more has no GPOS kerning.
const MAX_PAIR_RECORDS: usize = 1 << 16;

/// The most entries of the index of the first glyphs. The pair subtables of
/// a font whose coverage tables list more glyphs get no index.
const MAX_COVERED_GLYPHS: usize = 1 << 18;

/// The most format 0 subtables of the `kern` table that the reader keeps.
const MAX_KERN_SUBTABLES: usize = 64;

/// The value format bit of the x placement.
const X_PLACEMENT: u16 = 0x0001;

/// The value format bit of the y placement.
const Y_PLACEMENT: u16 = 0x0002;

/// The value format bit of the x advance.
const X_ADVANCE: u16 = 0x0004;

/// The size of a pair record of a `kern` format 0 subtable.
const KERN_PAIR_SIZE: usize = 6;

/// The coverage bits of a horizontal `kern` subtable of version 0 that holds
/// kerning values and no minimum values and no cross-stream values.
const KERN_HORIZONTAL: u16 = 0x0001;

/// The coverage bits of version 0 that select the kind of values.
const KERN_KIND_MASK: u16 = 0x0007;

/// The coverage bit of version 0 that replaces the accumulated value.
const KERN_OVERRIDE: u16 = 0x0008;

/// The coverage bits of a vertical, a cross-stream or a variation subtable of
/// `kern` version 1.
const KERN_APPLE_EXCLUDED: u16 = 0xE000;

/// The kerning of a font.
///
/// The subtables hold offsets relative to their table. A broken GPOS or
/// `kern` table counts as absent.
#[derive(Clone, Default)]
pub(crate) struct Kerning {
    /// The GPOS table.
    gpos: Span,
    /// The pair adjustment subtables of each GPOS lookup of the kerning
    /// feature, in lookup order.
    lookups: Vec<Vec<PairSubtable>>,
    /// The subtables whose coverage lists a glyph, sorted by glyph, lookup
    /// and subtable. Without the index every pair reads every subtable.
    covered: Option<Vec<CoveredGlyph>>,
    /// The `kern` table.
    kern: Span,
    /// The format 0 subtables of the `kern` table.
    pairs: Vec<KernSubtable>,
}

impl fmt::Debug for Kerning {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Kerning")
            .field("gpos_lookups", &self.lookups.len())
            .field(
                "gpos_subtables",
                &self.lookups.iter().map(Vec::len).sum::<usize>(),
            )
            .field("kern_subtables", &self.pairs.len())
            .finish()
    }
}

impl Kerning {
    /// The kerning of the GPOS table at `gpos` and the `kern` table at `kern`
    /// of the font data `data`. GPOS kerning takes the place of the `kern`
    /// table when the font has both.
    pub(crate) fn parse(data: &[u8], gpos: Option<Span>, kern: Option<Span>) -> Self {
        let mut kerning = Self::default();
        if let Some(gpos) = gpos
            && let Ok(lookups) = gpos_kern_lookups(gpos.get(data))
            && !lookups.is_empty()
        {
            kerning.covered = covered_glyphs(gpos.get(data), &lookups);
            kerning.gpos = gpos;
            kerning.lookups = lookups;
            return kerning;
        }
        if let Some(kern) = kern
            && let Ok(pairs) = kern_pairs(kern.get(data))
        {
            kerning.kern = kern;
            kerning.pairs = pairs;
        }
        kerning
    }

    /// The advance adjustment in font units between `left` and the glyph
    /// `right` that follows it in the font data `data`.
    pub(crate) fn adjustment(&self, data: &[u8], left: u16, right: u16) -> i16 {
        let mut total = 0i32;
        let gpos = self.gpos.get(data);
        if let Some(covered) = &self.covered {
            let start = covered.partition_point(|entry| entry.glyph < left);
            let mut applied = None;
            for entry in covered[start..]
                .iter()
                .take_while(|entry| entry.glyph == left)
            {
                if applied == Some(entry.lookup) {
                    continue;
                }
                let subtable =
                    &self.lookups[usize::from(entry.lookup)][usize::from(entry.subtable)];
                if let Some(value) = subtable.adjustment(gpos, left, right) {
                    total = total.saturating_add(i32::from(value));
                    applied = Some(entry.lookup);
                }
            }
        } else {
            for lookup in &self.lookups {
                let value = lookup
                    .iter()
                    .find_map(|subtable| subtable.adjustment(gpos, left, right));
                total = total.saturating_add(i32::from(value.unwrap_or(0)));
            }
        }
        let kern = self.kern.get(data);
        for subtable in &self.pairs {
            if let Some(value) = subtable.adjustment(kern, left, right) {
                total = if subtable.replaces {
                    i32::from(value)
                } else {
                    total.saturating_add(i32::from(value))
                };
            }
        }
        total.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
    }
}

/// A glyph that the coverage table of a pair adjustment subtable lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct CoveredGlyph {
    /// The glyph.
    glyph: u16,
    /// The index of the lookup in the kerning lookups.
    lookup: u16,
    /// The index of the subtable in its lookup.
    subtable: u16,
}

/// The index of the first glyphs of the pair subtables of `lookups` in the
/// GPOS table `gpos`. A subtable can apply to a pair only when its coverage
/// lists the first glyph. The index lists every glyph of the coverage tables
/// and returns `None` past [`MAX_COVERED_GLYPHS`] entries.
fn covered_glyphs(gpos: &[u8], lookups: &[Vec<PairSubtable>]) -> Option<Vec<CoveredGlyph>> {
    let mut remaining = MAX_COVERED_GLYPHS;
    let mut covered = Vec::new();
    for (lookup_index, lookup) in lookups.iter().enumerate() {
        let lookup_index = u16::try_from(lookup_index).ok()?;
        for (subtable_index, subtable) in lookup.iter().enumerate() {
            let subtable_index = u16::try_from(subtable_index).ok()?;
            subtable.coverage().glyphs(gpos, &mut remaining, |glyph| {
                covered.push(CoveredGlyph {
                    glyph,
                    lookup: lookup_index,
                    subtable: subtable_index,
                });
            })?;
        }
    }
    covered.sort_unstable();
    covered.dedup();
    Some(covered)
}

/// A format 0 subtable of the `kern` table.
#[derive(Clone, Copy, Debug)]
struct KernSubtable {
    /// The offset of the pair records in the `kern` table.
    offset: usize,
    /// The number of pair records.
    count: usize,
    /// Whether the value replaces the sum of the previous subtables.
    replaces: bool,
}

impl KernSubtable {
    /// The value of the pair in the `kern` table `kern`, or `None` when the
    /// subtable has no record for it.
    fn adjustment(&self, kern: &[u8], left: u16, right: u16) -> Option<i16> {
        let key = (u32::from(left) << 16) | u32::from(right);
        let record = binary_search(self.count, |index| {
            Some(
                u32_at(kern, self.offset + index * KERN_PAIR_SIZE)
                    .ok()?
                    .cmp(&key),
            )
        })?;
        i16_at(kern, self.offset + record * KERN_PAIR_SIZE + 4).ok()
    }
}

/// The horizontal format 0 subtables of the `kern` table `kern`. Both the
/// version 0 and the version 1 layout are read. A format 0 subtable of
/// version 0 takes its size from its pair count, because fonts with many
/// pairs overflow the 16-bit length field.
fn kern_pairs(kern: &[u8]) -> Result<Vec<KernSubtable>, FontError> {
    let mut pairs = Vec::new();
    match u16_at(kern, 0)? {
        0 => {
            let count = u16_at(kern, 2)?;
            let mut offset = 4;
            for index in 0..count {
                let coverage = u16_at(kern, offset + 4)?;
                let length = if coverage >> 8 == 0 {
                    let subtable = format_0_pairs(kern, offset + 6)?;
                    if coverage & KERN_KIND_MASK == KERN_HORIZONTAL {
                        pairs.push(KernSubtable {
                            replaces: coverage & KERN_OVERRIDE != 0,
                            ..subtable
                        });
                    }
                    subtable.offset + subtable.count * KERN_PAIR_SIZE - offset
                } else {
                    usize::from(u16_at(kern, offset + 2)?)
                };
                if pairs.len() == MAX_KERN_SUBTABLES {
                    break;
                }
                if index + 1 < count && length < 6 {
                    return Err(FontError::InvalidTable(Tag::KERN));
                }
                offset += length;
            }
        }
        1 => {
            let count = u32_at(kern, 4)?;
            let mut offset = 8;
            for index in 0..count {
                let length = u32_at(kern, offset)? as usize;
                let coverage = u16_at(kern, offset + 4)?;
                if coverage & 0xFF == 0 && coverage & KERN_APPLE_EXCLUDED == 0 {
                    pairs.push(format_0_pairs(kern, offset + 8)?);
                }
                if pairs.len() == MAX_KERN_SUBTABLES {
                    break;
                }
                if index + 1 < count && length < 8 {
                    return Err(FontError::InvalidTable(Tag::KERN));
                }
                offset = offset.checked_add(length).ok_or(FontError::Truncated)?;
            }
        }
        _ => return Err(FontError::InvalidTable(Tag::KERN)),
    }
    Ok(pairs)
}

/// The format 0 subtable body at `offset` of the `kern` table `kern`.
fn format_0_pairs(kern: &[u8], offset: usize) -> Result<KernSubtable, FontError> {
    let count = usize::from(u16_at(kern, offset)?);
    slice(kern, offset + 8, count * KERN_PAIR_SIZE)?;
    Ok(KernSubtable {
        offset: offset + 8,
        count,
        replaces: false,
    })
}

/// Counts the records that the GPOS parse reads and the pair records that
/// the lookups scan.
struct Budget {
    /// The records that the parse may still read.
    steps: usize,
    /// The pair records that the format 1 subtables may still hold.
    pair_records: usize,
}

impl Budget {
    /// Takes `count` steps. Past the limit the parse fails.
    fn take_steps(&mut self, count: usize) -> Result<(), FontError> {
        self.steps = self
            .steps
            .checked_sub(count)
            .ok_or(FontError::InvalidTable(Tag::GPOS))?;
        Ok(())
    }

    /// Takes `count` pair records. Past the limit the parse fails.
    fn take_pair_records(&mut self, count: usize) -> Result<(), FontError> {
        self.pair_records = self
            .pair_records
            .checked_sub(count)
            .ok_or(FontError::InvalidTable(Tag::GPOS))?;
        Ok(())
    }
}

/// The offset `relative` from `base`.
fn offset_from(base: usize, relative: impl Into<usize>) -> Result<usize, FontError> {
    base.checked_add(relative.into())
        .ok_or(FontError::Truncated)
}

/// The pair adjustment subtables of the lookups of the kerning feature of
/// the GPOS table `gpos`. The first of the preferred scripts with a kerning
/// feature wins, then the first other script with one.
fn gpos_kern_lookups(gpos: &[u8]) -> Result<Vec<Vec<PairSubtable>>, FontError> {
    if u16_at(gpos, 0)? != 1 {
        return Err(FontError::InvalidTable(Tag::GPOS));
    }
    let mut budget = Budget {
        steps: MAX_GPOS_STEPS,
        pair_records: MAX_PAIR_RECORDS,
    };
    let lists = Lists {
        features: usize::from(u16_at(gpos, 6)?),
        lookups: usize::from(u16_at(gpos, 8)?),
    };
    let script_list = usize::from(u16_at(gpos, 4)?);
    let script_count = usize::from(u16_at(gpos, script_list)?);
    let mut scripts = Vec::with_capacity(script_count);
    for index in 0..script_count {
        let record = script_list + 2 + index * 6;
        let tag = Tag(u32_at(gpos, record)?.to_be_bytes());
        let offset = offset_from(script_list, u16_at(gpos, record + 4)?)?;
        let rank = PREFERRED_SCRIPTS
            .iter()
            .position(|preferred| *preferred == tag)
            .unwrap_or(PREFERRED_SCRIPTS.len());
        scripts.push((rank, offset));
    }
    scripts.sort_by_key(|(rank, _)| *rank);

    let mut visited_features = vec![false; usize::from(u16_at(gpos, lists.features)?)];
    let mut selected_lookups = vec![false; usize::from(u16_at(gpos, lists.lookups)?)];
    for (_, script) in scripts {
        budget.take_steps(1)?;
        let found = select_kern_lookups(
            gpos,
            script,
            &lists,
            &mut visited_features,
            &mut selected_lookups,
            &mut budget,
        )?;
        if !found {
            continue;
        }
        let mut lookups = Vec::new();
        let mut remaining = MAX_PAIR_SUBTABLES;
        for (index, _) in selected_lookups
            .iter()
            .enumerate()
            .filter(|(_, selected)| **selected)
        {
            let subtables = pair_lookup(gpos, lists.lookups, index, &mut remaining, &mut budget)?;
            if !subtables.is_empty() {
                lookups.push(subtables);
            }
        }
        return Ok(lookups);
    }
    Ok(Vec::new())
}

/// The offsets of the feature list and the lookup list in the GPOS table.
struct Lists {
    /// The offset of the feature list.
    features: usize,
    /// The offset of the lookup list.
    lookups: usize,
}

/// Selects the lookups of the kerning features of the default language
/// system of the script table at `script`. A feature that an earlier script
/// visited adds nothing, because that script selected no lookup. Returns
/// whether the script selected a lookup.
fn select_kern_lookups(
    gpos: &[u8],
    script: usize,
    lists: &Lists,
    visited_features: &mut [bool],
    selected_lookups: &mut [bool],
    budget: &mut Budget,
) -> Result<bool, FontError> {
    let default_offset = u16_at(gpos, script)?;
    if default_offset == 0 {
        return Ok(false);
    }
    let language = offset_from(script, default_offset)?;
    let required = u16_at(gpos, language + 2)?;
    let feature_count = usize::from(u16_at(gpos, language + 4)?);
    let mut found = false;
    for position in 0..=feature_count {
        budget.take_steps(1)?;
        let feature_index = if position == 0 {
            if required == NO_REQUIRED_FEATURE {
                continue;
            }
            required
        } else {
            u16_at(gpos, language + 4 + position * 2)?
        };
        let feature_index = usize::from(feature_index);
        let visited = visited_features
            .get_mut(feature_index)
            .ok_or(FontError::InvalidTable(Tag::GPOS))?;
        if *visited {
            continue;
        }
        *visited = true;
        let record = lists.features + 2 + feature_index * 6;
        if u32_at(gpos, record)?.to_be_bytes() != KERN_FEATURE.0 {
            continue;
        }
        let feature = offset_from(lists.features, u16_at(gpos, record + 4)?)?;
        let lookup_count = usize::from(u16_at(gpos, feature + 2)?);
        budget.take_steps(lookup_count)?;
        for index in 0..lookup_count {
            let lookup = usize::from(u16_at(gpos, feature + 4 + index * 2)?);
            *selected_lookups
                .get_mut(lookup)
                .ok_or(FontError::InvalidTable(Tag::GPOS))? = true;
            found = true;
        }
    }
    Ok(found)
}

/// The pair adjustment subtables of the lookup `index` of the lookup list at
/// `lookup_list`. A lookup of another type has none. `remaining` counts the
/// subtables that the font may still add.
fn pair_lookup(
    gpos: &[u8],
    lookup_list: usize,
    index: usize,
    remaining: &mut usize,
    budget: &mut Budget,
) -> Result<Vec<PairSubtable>, FontError> {
    let lookup = offset_from(lookup_list, u16_at(gpos, lookup_list + 2 + index * 2)?)?;
    let lookup_type = u16_at(gpos, lookup)?;
    let subtable_count = usize::from(u16_at(gpos, lookup + 4)?);
    budget.take_steps(subtable_count)?;
    let mut subtables = Vec::new();
    for subtable_index in 0..subtable_count {
        let subtable = offset_from(lookup, u16_at(gpos, lookup + 6 + subtable_index * 2)?)?;
        let pair_subtable = match lookup_type {
            PAIR_ADJUSTMENT => subtable,
            EXTENSION => {
                if u16_at(gpos, subtable)? != 1 {
                    return Err(FontError::InvalidTable(Tag::GPOS));
                }
                if u16_at(gpos, subtable + 2)? != PAIR_ADJUSTMENT {
                    continue;
                }
                offset_from(subtable, u32_at(gpos, subtable + 4)? as usize)?
            }
            _ => continue,
        };
        *remaining = remaining
            .checked_sub(1)
            .ok_or(FontError::InvalidTable(Tag::GPOS))?;
        subtables.push(PairSubtable::parse(gpos, pair_subtable, budget)?);
    }
    Ok(subtables)
}

/// The size of a value record of `format`.
fn value_record_size(format: u16) -> usize {
    2 * (format & 0x00FF).count_ones() as usize
}

/// The offset of the x advance in a value record of `format`.
fn x_advance_offset(format: u16) -> Option<usize> {
    (format & X_ADVANCE != 0)
        .then(|| 2 * (format & (X_PLACEMENT | Y_PLACEMENT)).count_ones() as usize)
}

/// A pair adjustment subtable with offsets relative to the GPOS table.
#[derive(Clone, Copy, Debug)]
enum PairSubtable {
    /// Format 1 with a pair set for each covered first glyph.
    Glyphs(GlyphPairs),
    /// Format 2 with a value for each pair of glyph classes.
    Classes(ClassPairs),
}

/// A format 1 pair adjustment subtable.
#[derive(Clone, Copy, Debug)]
struct GlyphPairs {
    /// The offset of the subtable.
    offset: usize,
    /// The covered first glyphs.
    coverage: Coverage,
    /// The number of pair sets.
    set_count: usize,
    /// The size of a pair value record.
    record_size: usize,
    /// The offset of the x advance in the first value record.
    x_advance: Option<usize>,
    /// Whether every pair set holds its records in strictly ascending order
    /// of the second glyph.
    sorted: bool,
}

/// A format 2 pair adjustment subtable.
#[derive(Clone, Copy, Debug)]
struct ClassPairs {
    /// The covered first glyphs.
    coverage: Coverage,
    /// The classes of the first glyph.
    first_classes: ClassDefinition,
    /// The classes of the second glyph.
    second_classes: ClassDefinition,
    /// The number of first glyph classes.
    first_count: usize,
    /// The number of second glyph classes.
    second_count: usize,
    /// The offset of the value records of all class pairs.
    records: usize,
    /// The size of the two value records of a class pair.
    record_size: usize,
    /// The offset of the x advance in the first value record.
    x_advance: Option<usize>,
}

impl PairSubtable {
    /// Parses the pair adjustment subtable at `offset` of the GPOS table
    /// `gpos`. The pair sets of format 1 take their records from `budget`.
    fn parse(gpos: &[u8], offset: usize, budget: &mut Budget) -> Result<Self, FontError> {
        let format = u16_at(gpos, offset)?;
        let coverage = Coverage::parse(gpos, offset_from(offset, u16_at(gpos, offset + 2)?)?)?;
        let first_format = u16_at(gpos, offset + 4)?;
        let second_format = u16_at(gpos, offset + 6)?;
        let values_size = value_record_size(first_format) + value_record_size(second_format);
        let x_advance = x_advance_offset(first_format);
        match format {
            1 => {
                let set_count = usize::from(u16_at(gpos, offset + 8)?);
                budget.take_steps(set_count)?;
                let record_size = 2 + values_size;
                let mut sorted = true;
                for index in 0..set_count {
                    let set = offset_from(offset, u16_at(gpos, offset + 10 + index * 2)?)?;
                    let count = usize::from(u16_at(gpos, set)?);
                    let records = slice(gpos, set + 2, count * record_size)?;
                    budget.take_pair_records(count)?;
                    let mut previous = None;
                    for record in records.chunks_exact(record_size) {
                        let second = u16::from_be_bytes([record[0], record[1]]);
                        sorted &= previous < Some(second);
                        previous = Some(second);
                    }
                }
                Ok(Self::Glyphs(GlyphPairs {
                    offset,
                    coverage,
                    set_count,
                    record_size,
                    x_advance,
                    sorted,
                }))
            }
            2 => {
                let first_classes =
                    ClassDefinition::parse(gpos, offset, u16_at(gpos, offset + 8)?)?;
                let second_classes =
                    ClassDefinition::parse(gpos, offset, u16_at(gpos, offset + 10)?)?;
                let first_count = usize::from(u16_at(gpos, offset + 12)?);
                let second_count = usize::from(u16_at(gpos, offset + 14)?);
                let size = first_count
                    .checked_mul(second_count)
                    .and_then(|count| count.checked_mul(values_size))
                    .ok_or(FontError::InvalidTable(Tag::GPOS))?;
                slice(gpos, offset + 16, size)?;
                Ok(Self::Classes(ClassPairs {
                    coverage,
                    first_classes,
                    second_classes,
                    first_count,
                    second_count,
                    records: offset + 16,
                    record_size: values_size,
                    x_advance,
                }))
            }
            _ => Err(FontError::InvalidTable(Tag::GPOS)),
        }
    }

    /// The coverage table of the first glyphs.
    fn coverage(&self) -> Coverage {
        match self {
            Self::Glyphs(pairs) => pairs.coverage,
            Self::Classes(pairs) => pairs.coverage,
        }
    }

    /// The x advance adjustment of the pair, or `None` when the subtable does
    /// not apply to it.
    fn adjustment(&self, gpos: &[u8], left: u16, right: u16) -> Option<i16> {
        match self {
            Self::Glyphs(pairs) => pairs.adjustment(gpos, left, right),
            Self::Classes(pairs) => pairs.adjustment(gpos, left, right),
        }
    }
}

impl GlyphPairs {
    /// The x advance adjustment of the pair, or `None` when the pair set of
    /// `left` holds no record for `right`. The first record for `right`
    /// counts.
    fn adjustment(&self, gpos: &[u8], left: u16, right: u16) -> Option<i16> {
        let index = self.coverage.index(gpos, left)?;
        if index >= self.set_count {
            return None;
        }
        let set_offset = u16_at(gpos, self.offset + 10 + index * 2).ok()?;
        let set = self.offset + usize::from(set_offset);
        let count = usize::from(u16_at(gpos, set).ok()?);
        let second = |record: usize| u16_at(gpos, set + 2 + record * self.record_size).ok();
        // Some fonts store the records out of order. The scan of such a set
        // reads every record.
        let record = if self.sorted {
            binary_search(count, |record| Some(second(record)?.cmp(&right)))?
        } else {
            (0..count).find(|&record| second(record) == Some(right))?
        };
        match self.x_advance {
            Some(offset) => i16_at(gpos, set + 2 + record * self.record_size + 2 + offset).ok(),
            None => Some(0),
        }
    }
}

impl ClassPairs {
    /// The x advance adjustment of the class pair of `left` and `right`, or
    /// `None` when the subtable does not cover `left`.
    fn adjustment(&self, gpos: &[u8], left: u16, right: u16) -> Option<i16> {
        self.coverage.index(gpos, left)?;
        let first = usize::from(self.first_classes.class(gpos, left));
        let second = usize::from(self.second_classes.class(gpos, right));
        if first >= self.first_count || second >= self.second_count {
            return None;
        }
        let record = self.records + (first * self.second_count + second) * self.record_size;
        match self.x_advance {
            Some(offset) => i16_at(gpos, record + offset).ok(),
            None => Some(0),
        }
    }
}

/// A coverage table: the ordered set of glyphs that a subtable applies to.
#[derive(Clone, Copy, Debug)]
enum Coverage {
    /// Format 1 with a sorted glyph array.
    Glyphs {
        /// The offset of the glyph array.
        offset: usize,
        /// The number of glyphs.
        count: usize,
    },
    /// Format 2 with sorted glyph ranges of start, end and first index.
    Ranges {
        /// The offset of the range records.
        offset: usize,
        /// The number of ranges.
        count: usize,
    },
}

impl Coverage {
    /// Parses the coverage table at `offset` of the GPOS table `gpos`.
    fn parse(gpos: &[u8], offset: usize) -> Result<Self, FontError> {
        let count = usize::from(u16_at(gpos, offset + 2)?);
        match u16_at(gpos, offset)? {
            1 => {
                slice(gpos, offset + 4, count * 2)?;
                Ok(Self::Glyphs {
                    offset: offset + 4,
                    count,
                })
            }
            2 => {
                slice(gpos, offset + 4, count * 6)?;
                Ok(Self::Ranges {
                    offset: offset + 4,
                    count,
                })
            }
            _ => Err(FontError::InvalidTable(Tag::GPOS)),
        }
    }

    /// Calls `visit` with every glyph that the table lists, including the
    /// glyphs of a broken table that [`Coverage::index`] misses. Every glyph
    /// takes one of `remaining`. Returns `None` when they run out.
    fn glyphs(&self, gpos: &[u8], remaining: &mut usize, mut visit: impl FnMut(u16)) -> Option<()> {
        match *self {
            Self::Glyphs { offset, count } => {
                *remaining = remaining.checked_sub(count)?;
                for index in 0..count {
                    visit(u16_at(gpos, offset + index * 2).ok()?);
                }
            }
            Self::Ranges { offset, count } => {
                for index in 0..count {
                    let start = u16_at(gpos, offset + index * 6).ok()?;
                    let end = u16_at(gpos, offset + index * 6 + 2).ok()?;
                    if start > end {
                        continue;
                    }
                    *remaining = remaining.checked_sub(usize::from(end - start) + 1)?;
                    (start..=end).for_each(&mut visit);
                }
            }
        }
        Some(())
    }

    /// The coverage index of `glyph`, or `None` when the table does not cover
    /// it.
    fn index(&self, gpos: &[u8], glyph: u16) -> Option<usize> {
        match *self {
            Self::Glyphs { offset, count } => binary_search(count, |index| {
                Some(u16_at(gpos, offset + index * 2).ok()?.cmp(&glyph))
            }),
            Self::Ranges { offset, count } => {
                let range = offset + find_range(gpos, offset, count, glyph)? * 6;
                let start = u16_at(gpos, range).ok()?;
                let first_index = u16_at(gpos, range + 4).ok()?;
                Some(usize::from(first_index) + usize::from(glyph - start))
            }
        }
    }
}

/// The index of the record of the `count` range records at `offset` of
/// `gpos` with a start and an end glyph around `glyph`. A record holds the
/// start glyph, the end glyph and one more value.
fn find_range(gpos: &[u8], offset: usize, count: usize, glyph: u16) -> Option<usize> {
    binary_search(count, |index| {
        let start = u16_at(gpos, offset + index * 6).ok()?;
        let end = u16_at(gpos, offset + index * 6 + 2).ok()?;
        Some(if end < glyph {
            Ordering::Less
        } else if start > glyph {
            Ordering::Greater
        } else {
            Ordering::Equal
        })
    })
}

/// A class definition table: the class of each glyph. A glyph that the table
/// does not list has class zero.
#[derive(Clone, Copy, Debug)]
enum ClassDefinition {
    /// Format 1 with the classes of consecutive glyphs from `first`.
    Array {
        /// The first glyph with a class value.
        first: u16,
        /// The offset of the class values.
        offset: usize,
        /// The number of class values.
        count: usize,
    },
    /// Format 2 with sorted glyph ranges of start, end and class.
    Ranges {
        /// The offset of the range records.
        offset: usize,
        /// The number of ranges.
        count: usize,
    },
    /// No table, every glyph has class zero.
    Empty,
}

impl ClassDefinition {
    /// Parses the class definition at `relative` from the subtable at
    /// `subtable` of the GPOS table `gpos`. An offset of zero means no table.
    fn parse(gpos: &[u8], subtable: usize, relative: u16) -> Result<Self, FontError> {
        if relative == 0 {
            return Ok(Self::Empty);
        }
        let offset = offset_from(subtable, relative)?;
        match u16_at(gpos, offset)? {
            1 => {
                let first = u16_at(gpos, offset + 2)?;
                let count = usize::from(u16_at(gpos, offset + 4)?);
                slice(gpos, offset + 6, count * 2)?;
                Ok(Self::Array {
                    first,
                    offset: offset + 6,
                    count,
                })
            }
            2 => {
                let count = usize::from(u16_at(gpos, offset + 2)?);
                slice(gpos, offset + 4, count * 6)?;
                Ok(Self::Ranges {
                    offset: offset + 4,
                    count,
                })
            }
            _ => Err(FontError::InvalidTable(Tag::GPOS)),
        }
    }

    /// The class of `glyph`.
    fn class(&self, gpos: &[u8], glyph: u16) -> u16 {
        match *self {
            Self::Array {
                first,
                offset,
                count,
            } => glyph
                .checked_sub(first)
                .map(usize::from)
                .filter(|index| *index < count)
                .and_then(|index| u16_at(gpos, offset + index * 2).ok())
                .unwrap_or(0),
            Self::Ranges { offset, count } => find_range(gpos, offset, count, glyph)
                .and_then(|range| u16_at(gpos, offset + range * 6 + 4).ok())
                .unwrap_or(0),
            Self::Empty => 0,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A kerning pair of left glyph, right glyph and value.
    type KernPair = (u16, u16, i16);

    /// The kerning of GPOS and `kern` tables placed in one font data buffer.
    struct Tables {
        /// The font data.
        data: Vec<u8>,
        /// The parsed kerning.
        kerning: Kerning,
    }

    impl Tables {
        /// Places `gpos` and `kern` behind four bytes of padding and parses them.
        fn new(gpos: Option<&[u8]>, kern: Option<&[u8]>) -> Self {
            let mut data = vec![0; 4];
            let mut place = |table: Option<&[u8]>| {
                table.map(|table| {
                    let offset = data.len();
                    data.extend(table);
                    Span {
                        offset,
                        length: table.len(),
                    }
                })
            };
            let gpos = place(gpos);
            let kern = place(kern);
            let kerning = Kerning::parse(&data, gpos, kern);
            Self { data, kerning }
        }

        /// The advance adjustment of the pair.
        fn adjustment(&self, left: u16, right: u16) -> i16 {
            self.kerning.adjustment(&self.data, left, right)
        }
    }

    /// Appends big endian 16-bit values to `data`.
    fn push(data: &mut Vec<u8>, values: &[u16]) {
        for value in values {
            data.extend(value.to_be_bytes());
        }
    }

    /// A list of tagged tables: the count, the records and the tables.
    fn tagged_list(entries: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut list = Vec::new();
        push(&mut list, &[entries.len() as u16]);
        let mut offset = 2 + entries.len() * 6;
        for (tag, table) in entries {
            list.extend(tag);
            push(&mut list, &[offset as u16]);
            offset += table.len();
        }
        for (_, table) in entries {
            list.extend(table);
        }
        list
    }

    /// A GPOS table. Each script has a default language system with the
    /// feature indices. Each feature has its lookup indices. Each lookup has
    /// its type and subtables.
    fn gpos_table(
        scripts: &[([u8; 4], Vec<u16>)],
        features: &[([u8; 4], Vec<u16>)],
        lookups: &[(u16, Vec<Vec<u8>>)],
    ) -> Vec<u8> {
        let script_tables: Vec<_> = scripts
            .iter()
            .map(|(tag, indices)| {
                let mut script = Vec::new();
                push(
                    &mut script,
                    &[4, 0, 0, NO_REQUIRED_FEATURE, indices.len() as u16],
                );
                push(&mut script, indices);
                (*tag, script)
            })
            .collect();
        let feature_tables: Vec<_> = features
            .iter()
            .map(|(tag, indices)| {
                let mut feature = Vec::new();
                push(&mut feature, &[0, indices.len() as u16]);
                push(&mut feature, indices);
                (*tag, feature)
            })
            .collect();
        let mut lookup_list = Vec::new();
        push(&mut lookup_list, &[lookups.len() as u16]);
        let mut lookup_tables = Vec::new();
        let mut offset = 2 + lookups.len() * 2;
        for (lookup_type, subtables) in lookups {
            push(&mut lookup_list, &[offset as u16]);
            let mut lookup = Vec::new();
            push(&mut lookup, &[*lookup_type, 0, subtables.len() as u16]);
            let mut subtable_offset = 6 + subtables.len() * 2;
            for subtable in subtables {
                push(&mut lookup, &[subtable_offset as u16]);
                subtable_offset += subtable.len();
            }
            for subtable in subtables {
                lookup.extend(subtable);
            }
            offset += lookup.len();
            lookup_tables.extend(lookup);
        }
        lookup_list.extend(lookup_tables);

        let script_list = tagged_list(&script_tables);
        let feature_list = tagged_list(&feature_tables);
        let mut gpos = Vec::new();
        let feature_offset = 10 + script_list.len();
        let lookup_offset = feature_offset + feature_list.len();
        push(
            &mut gpos,
            &[1, 0, 10, feature_offset as u16, lookup_offset as u16],
        );
        gpos.extend(script_list);
        gpos.extend(feature_list);
        gpos.extend(lookup_list);
        gpos
    }

    /// A GPOS table that kerns the glyph pair of `left` and `right` by
    /// `value`.
    pub(crate) fn single_pair_gpos(left: u16, right: u16, value: i16) -> Vec<u8> {
        let pairs = pair_glyphs(&[left], X_ADVANCE, 0, &[vec![(right, vec![value])]]);
        gpos_table(
            &[(*b"DFLT", vec![0])],
            &[(*b"kern", vec![0])],
            &[(PAIR_ADJUSTMENT, vec![pairs])],
        )
    }

    /// A format 1 coverage table.
    fn coverage_glyphs(glyphs: &[u16]) -> Vec<u8> {
        let mut coverage = Vec::new();
        push(&mut coverage, &[1, glyphs.len() as u16]);
        push(&mut coverage, glyphs);
        coverage
    }

    /// A format 2 coverage table from `(start, end)` ranges.
    fn coverage_ranges(ranges: &[(u16, u16)]) -> Vec<u8> {
        let mut coverage = Vec::new();
        push(&mut coverage, &[2, ranges.len() as u16]);
        let mut index = 0;
        for (start, end) in ranges {
            push(&mut coverage, &[*start, *end, index]);
            index += end - start + 1;
        }
        coverage
    }

    /// A format 1 pair adjustment subtable. Each pair set of a covered glyph
    /// holds `(second glyph, value record words)` records.
    fn pair_glyphs(
        glyphs: &[u16],
        first_format: u16,
        second_format: u16,
        sets: &[Vec<(u16, Vec<i16>)>],
    ) -> Vec<u8> {
        let coverage = coverage_glyphs(glyphs);
        let mut subtable = Vec::new();
        let header = 10 + sets.len() * 2;
        push(
            &mut subtable,
            &[
                1,
                header as u16,
                first_format,
                second_format,
                sets.len() as u16,
            ],
        );
        let mut set_data = Vec::new();
        for set in sets {
            push(
                &mut subtable,
                &[(header + coverage.len() + set_data.len()) as u16],
            );
            push(&mut set_data, &[set.len() as u16]);
            for (second, values) in set {
                push(&mut set_data, &[*second]);
                for value in values {
                    set_data.extend(value.to_be_bytes());
                }
            }
        }
        subtable.extend(coverage);
        subtable.extend(set_data);
        subtable
    }

    /// A format 2 pair adjustment subtable with format 2 class definitions
    /// from `(start, end, class)` ranges and the value record words of each
    /// class pair.
    fn pair_classes(
        coverage: Vec<u8>,
        first_classes: &[(u16, u16, u16)],
        second_classes: &[(u16, u16, u16)],
        counts: (u16, u16),
        values: &[i16],
    ) -> Vec<u8> {
        let class_definition = |ranges: &[(u16, u16, u16)]| {
            let mut data = Vec::new();
            push(&mut data, &[2, ranges.len() as u16]);
            for (start, end, class) in ranges {
                push(&mut data, &[*start, *end, *class]);
            }
            data
        };
        let first = class_definition(first_classes);
        let second = class_definition(second_classes);
        let header = 16 + values.len() * 2;
        let mut subtable = Vec::new();
        push(
            &mut subtable,
            &[
                2,
                header as u16,
                X_ADVANCE,
                0,
                (header + coverage.len()) as u16,
                (header + coverage.len() + first.len()) as u16,
                counts.0,
                counts.1,
            ],
        );
        for value in values {
            subtable.extend(value.to_be_bytes());
        }
        subtable.extend(coverage);
        subtable.extend(first);
        subtable.extend(second);
        subtable
    }

    /// An extension subtable that wraps a subtable of `lookup_type`.
    fn extension(lookup_type: u16, subtable: Vec<u8>) -> Vec<u8> {
        let mut data = Vec::new();
        push(&mut data, &[1, lookup_type, 0, 8]);
        data.extend(subtable);
        data
    }

    /// A version 0 `kern` table with format 0 subtables of `(coverage, pairs)`.
    fn kern_table(subtables: &[(u16, &[KernPair])]) -> Vec<u8> {
        let mut kern = Vec::new();
        push(&mut kern, &[0, subtables.len() as u16]);
        for (coverage, pairs) in subtables {
            push(&mut kern, &[0, (14 + pairs.len() * 6) as u16, *coverage]);
            push(&mut kern, &[pairs.len() as u16, 0, 0, 0]);
            for (left, right, value) in *pairs {
                push(&mut kern, &[*left, *right]);
                kern.extend(value.to_be_bytes());
            }
        }
        kern
    }

    #[test]
    fn glyph_pairs_and_class_pairs_adjust_the_advance() {
        let glyphs = pair_glyphs(
            &[5, 9],
            X_PLACEMENT | X_ADVANCE,
            X_PLACEMENT,
            &[
                vec![(6, vec![7, -40, 3]), (8, vec![0, -20, 0])],
                vec![(5, vec![1, 15, 2])],
            ],
        );
        let classes = pair_classes(
            coverage_ranges(&[(5, 6), (20, 30)]),
            &[(5, 6, 1), (25, 30, 2)],
            &[(6, 6, 1), (8, 8, 2)],
            (3, 3),
            &[0, 0, 0, 0, -11, 0, 0, -22, 33],
        );
        let gpos = gpos_table(
            &[(*b"DFLT", vec![0])],
            &[(*b"kern", vec![0])],
            &[(PAIR_ADJUSTMENT, vec![glyphs, classes])],
        );
        let kerning = Tables::new(Some(&gpos), None);
        assert_eq!(kerning.adjustment(5, 6), -40);
        assert_eq!(kerning.adjustment(5, 8), -20);
        assert_eq!(kerning.adjustment(9, 5), 15);
        assert_eq!(kerning.adjustment(5, 7), 0);
        assert_eq!(kerning.adjustment(6, 6), -11);
        assert_eq!(kerning.adjustment(26, 6), -22);
        assert_eq!(kerning.adjustment(30, 8), 33);
        assert_eq!(kerning.adjustment(31, 8), 0);
    }

    #[test]
    fn the_lookups_of_the_feature_add_up() {
        let first = pair_glyphs(&[1], X_ADVANCE, 0, &[vec![(2, vec![-30])]]);
        let second = pair_glyphs(&[1], X_ADVANCE, 0, &[vec![(2, vec![-5])]]);
        let wrapped = pair_glyphs(&[1], X_ADVANCE, 0, &[vec![(2, vec![-1])]]);
        let unused = pair_glyphs(&[1], X_ADVANCE, 0, &[vec![(2, vec![-100])]]);
        let gpos = gpos_table(
            &[(*b"cyrl", vec![2]), (*b"DFLT", vec![0, 1])],
            &[
                (*b"kern", vec![2, 0]),
                (*b"kern", vec![1, 0]),
                (*b"kern", vec![3]),
                (*b"mark", vec![3]),
            ],
            &[
                (PAIR_ADJUSTMENT, vec![first]),
                (PAIR_ADJUSTMENT, vec![second]),
                (EXTENSION, vec![extension(PAIR_ADJUSTMENT, wrapped)]),
                (PAIR_ADJUSTMENT, vec![unused]),
            ],
        );
        let kerning = Tables::new(Some(&gpos), None);
        assert_eq!(kerning.adjustment(1, 2), -36);
    }

    #[test]
    fn a_script_without_kerning_falls_back_to_the_next() {
        let pairs = pair_glyphs(&[1], X_ADVANCE, 0, &[vec![(2, vec![-30])]]);
        let gpos = gpos_table(
            &[(*b"DFLT", vec![1]), (*b"latn", vec![0])],
            &[(*b"kern", vec![0]), (*b"mark", vec![0])],
            &[(PAIR_ADJUSTMENT, vec![pairs])],
        );
        let kerning = Tables::new(Some(&gpos), None);
        assert_eq!(kerning.adjustment(1, 2), -30);
    }

    #[test]
    fn the_kern_table_holds_pairs() {
        let kern = kern_table(&[
            (0x0001, &[(1, 2, -50), (1, 3, -10), (4, 1, 20)]),
            (0x0003, &[(1, 2, -1000)]),
            (0x0001, &[(1, 2, -5)]),
        ]);
        let kerning = Tables::new(None, Some(&kern));
        assert_eq!(kerning.adjustment(1, 2), -55);
        assert_eq!(kerning.adjustment(4, 1), 20);
        assert_eq!(kerning.adjustment(2, 1), 0);

        let mut apple = Vec::new();
        push(&mut apple, &[1, 0, 0, 1, 0, 14 + 6 + 2, 0, 0]);
        push(&mut apple, &[1, 0, 0, 0, 1, 2]);
        apple.extend((-70i16).to_be_bytes());
        let kerning = Tables::new(None, Some(&apple));
        assert_eq!(kerning.adjustment(1, 2), -70);
    }

    #[test]
    fn gpos_kerning_takes_the_place_of_the_kern_table() {
        let pairs = pair_glyphs(&[1], X_ADVANCE, 0, &[vec![(2, vec![-30])]]);
        let gpos = gpos_table(
            &[(*b"DFLT", vec![0])],
            &[(*b"kern", vec![0])],
            &[(PAIR_ADJUSTMENT, vec![pairs])],
        );
        let kern = kern_table(&[(0x0001, &[(1, 2, -50), (3, 4, -10)])]);
        let kerning = Tables::new(Some(&gpos), Some(&kern));
        assert_eq!(kerning.adjustment(1, 2), -30);
        assert_eq!(kerning.adjustment(3, 4), 0);

        let empty = gpos_table(&[(*b"DFLT", vec![])], &[], &[]);
        let kerning = Tables::new(Some(&empty), Some(&kern));
        assert_eq!(kerning.adjustment(3, 4), -10);
    }

    #[test]
    fn unsorted_pair_records_are_found() {
        let pairs = pair_glyphs(
            &[1],
            X_ADVANCE,
            0,
            &[vec![(9, vec![-5]), (2, vec![-30]), (4, vec![-7])]],
        );
        let gpos = gpos_table(
            &[(*b"DFLT", vec![0])],
            &[(*b"kern", vec![0])],
            &[(PAIR_ADJUSTMENT, vec![pairs])],
        );
        let kerning = Tables::new(Some(&gpos), None);
        assert_eq!(kerning.adjustment(1, 2), -30);
        assert_eq!(kerning.adjustment(1, 4), -7);
        assert_eq!(kerning.adjustment(1, 9), -5);
        assert_eq!(kerning.adjustment(1, 3), 0);
    }

    #[test]
    fn the_required_feature_counts() {
        let pairs = pair_glyphs(&[1], X_ADVANCE, 0, &[vec![(2, vec![-30])]]);
        let mut gpos = gpos_table(
            &[(*b"DFLT", vec![])],
            &[(*b"kern", vec![0])],
            &[(PAIR_ADJUSTMENT, vec![pairs])],
        );
        assert_eq!(Tables::new(Some(&gpos), None).adjustment(1, 2), 0);
        gpos[24..26].copy_from_slice(&[0, 0]);
        assert_eq!(Tables::new(Some(&gpos), None).adjustment(1, 2), -30);
    }

    #[test]
    fn repeated_features_select_their_lookups_once() {
        let pairs = pair_glyphs(&[1], X_ADVANCE, 0, &[vec![(2, vec![-30])]]);
        let gpos = gpos_table(
            &[(*b"DFLT", vec![0; 12000]), (*b"latn", vec![0; 12000])],
            &[(*b"kern", vec![0, 0, 0])],
            &[(PAIR_ADJUSTMENT, vec![pairs])],
        );
        assert_eq!(Tables::new(Some(&gpos), None).adjustment(1, 2), -30);
    }

    #[test]
    fn large_sums_saturate() {
        let first = pair_glyphs(&[1], X_ADVANCE, 0, &[vec![(2, vec![-30000])]]);
        let second = first.clone();
        let gpos = gpos_table(
            &[(*b"DFLT", vec![0])],
            &[(*b"kern", vec![0, 1])],
            &[
                (PAIR_ADJUSTMENT, vec![first]),
                (PAIR_ADJUSTMENT, vec![second]),
            ],
        );
        assert_eq!(Tables::new(Some(&gpos), None).adjustment(1, 2), i16::MIN);
    }

    #[test]
    fn broken_or_oversized_gpos_tables_fall_back_to_the_kern_table() {
        let pairs = pair_glyphs(&[1], X_ADVANCE, 0, &[vec![(2, vec![-30])]]);
        let gpos = gpos_table(
            &[(*b"DFLT", vec![0])],
            &[(*b"kern", vec![0])],
            &[(PAIR_ADJUSTMENT, vec![pairs.clone()])],
        );
        let kern = kern_table(&[(0x0001, &[(1, 2, -50)])]);
        let broken = &gpos[..12];
        assert_eq!(Tables::new(Some(broken), Some(&kern)).adjustment(1, 2), -50);
        assert_eq!(Tables::new(Some(broken), None).adjustment(1, 2), 0);

        let many = gpos_table(
            &[(*b"DFLT", vec![0])],
            &[(*b"kern", vec![0])],
            &[(PAIR_ADJUSTMENT, vec![pairs; MAX_PAIR_SUBTABLES + 1])],
        );
        assert!(gpos_kern_lookups(&many).is_err());
        assert_eq!(Tables::new(Some(&many), Some(&kern)).adjustment(1, 2), -50);
    }

    #[test]
    fn the_pair_records_are_limited() {
        let pairs = |last_count: u16| {
            let large: Vec<_> = (0..u16::MAX).map(|glyph| (glyph, vec![-1])).collect();
            let small: Vec<_> = (0..last_count).map(|glyph| (glyph, vec![-2])).collect();
            pair_glyphs(&[1, 2], X_ADVANCE, 0, &[small, large])
        };
        let gpos = |last_count: u16| {
            gpos_table(
                &[(*b"DFLT", vec![0])],
                &[(*b"kern", vec![0])],
                &[(PAIR_ADJUSTMENT, vec![pairs(last_count)])],
            )
        };
        let limit = gpos(1);
        assert_eq!(Tables::new(Some(&limit), None).adjustment(2, 7), -1);
        assert!(gpos_kern_lookups(&gpos(2)).is_err());
    }

    #[test]
    fn the_kern_length_field_is_ignored_for_format_0() {
        let mut kern = kern_table(&[(0x0001, &[(1, 2, -50)]), (0x0001, &[(3, 4, -10)])]);
        kern[6..8].copy_from_slice(&[0, 2]);
        let kerning = Tables::new(None, Some(&kern));
        assert_eq!(kerning.adjustment(1, 2), -50);
        assert_eq!(kerning.adjustment(3, 4), -10);
    }

    #[test]
    fn the_override_bit_replaces_the_sum() {
        let kern = kern_table(&[
            (0x0001, &[(1, 2, -50)]),
            (0x0009, &[(1, 2, -7)]),
            (0x0001, &[(1, 2, -1)]),
        ]);
        assert_eq!(Tables::new(None, Some(&kern)).adjustment(1, 2), -8);
    }

    #[test]
    fn the_kern_subtables_are_limited() {
        let subtables: Vec<(u16, &[KernPair])> = vec![(0x0001, &[(1, 2, -1)]); 70];
        let kern = kern_table(&subtables);
        assert_eq!(
            Tables::new(None, Some(&kern)).adjustment(1, 2),
            -(MAX_KERN_SUBTABLES as i16)
        );
    }

    #[test]
    fn the_first_glyph_index_keeps_every_adjustment() {
        let font = crate::fonts::tests::font();
        let kerning = &font.kerning;
        let covered = kerning.covered.as_ref().expect("the font has an index");
        let unindexed = Kerning {
            covered: None,
            ..kerning.clone()
        };
        let mut lefts: Vec<u16> = covered.iter().map(|entry| entry.glyph).collect();
        lefts.extend((0..font.glyph_count()).step_by(53));
        lefts.dedup();
        let rights: Vec<u16> = (0..400)
            .chain((400..font.glyph_count()).step_by(97))
            .collect();
        let mut kerned = 0;
        for left in lefts {
            for &right in &rights {
                let value = kerning.adjustment(&font.data, left, right);
                assert_eq!(value, unindexed.adjustment(&font.data, left, right));
                kerned += usize::from(value != 0);
            }
        }
        assert!(kerned > 1000, "{kerned}");
    }

    #[test]
    fn coverage_past_the_index_limit_reads_every_subtable() {
        let gpos = |count: u16| {
            let lookups: Vec<_> = (0..count)
                .map(|index| {
                    let value = -1 - index as i16;
                    let coverage = coverage_ranges(&[(0, 60000)]);
                    let classes = pair_classes(coverage, &[], &[], (1, 1), &[value]);
                    (PAIR_ADJUSTMENT, vec![classes])
                })
                .collect();
            gpos_table(
                &[(*b"DFLT", vec![0])],
                &[(*b"kern", (0..count).collect())],
                &lookups,
            )
        };
        let indexed = Tables::new(Some(&gpos(4)), None);
        assert!(indexed.kerning.covered.is_some());
        assert_eq!(indexed.adjustment(7, 9), -10);
        assert_eq!(indexed.adjustment(60001, 9), 0);
        let unindexed = Tables::new(Some(&gpos(5)), None);
        assert!(unindexed.kerning.covered.is_none());
        assert_eq!(unindexed.adjustment(7, 9), -15);
        assert_eq!(unindexed.adjustment(60001, 9), 0);
    }

    #[test]
    fn truncated_tables_fail() {
        let gpos = gpos_table(
            &[(*b"DFLT", vec![0])],
            &[(*b"kern", vec![0])],
            &[(
                PAIR_ADJUSTMENT,
                vec![pair_classes(coverage_glyphs(&[1]), &[], &[], (1, 1), &[-4])],
            )],
        );
        assert_eq!(Tables::new(Some(&gpos), None).adjustment(1, 9), -4);
        for length in 0..gpos.len() {
            assert!(gpos_kern_lookups(&gpos[..length]).is_err());
        }
        let kern = kern_table(&[(0x0001, &[(1, 2, -50)])]);
        for length in 0..kern.len() {
            assert!(kern_pairs(&kern[..length]).is_err());
        }
    }
}
