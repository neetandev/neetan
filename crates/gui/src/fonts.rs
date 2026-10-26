//! The font of the user interface, the loaded faces, shaping and the cache of
//! the shaped runs.
//!
//! The user interface uses Sarasa UI J Regular by Renzhi Li (Belleve Invis)
//! under the SIL Open Font License 1.1. One face covers Latin, Greek,
//! Cyrillic, Vietnamese, kana, kanji and Hangul. The file is the unmodified
//! unhinted TTF file of the official release 1.0.42. The license text is
//! `licenses/LICENSE_sarasa`.
//!
//! The repository holds the font in its `font` folder. The caller locates
//! and reads the file and passes the bytes to [`Fonts::add`].
//!
//! Shaping maps every character to one glyph through the `cmap` table and
//! places the glyphs one after another by their advance widths and the pair
//! kerning. Every glyph keeps the default form of its character and sits on
//! the baseline.
//!
//! A cut or a wrap takes its width in logical units at a font size. A width
//! in em that is negative or NaN counts as zero. An infinite width holds any
//! text.

mod cache;
mod wrap;

use std::{collections::HashMap, ops::Range, sync::Arc};

use cache::{Fitting, RunCache};

use crate::{
    Path, ShapeStore, SlugData, SlugShape,
    truetype::{Font, FontError, GlyphId},
};

/// The character that ends a cut text.
const ELLIPSIS: char = '\u{2026}';

/// The text that ends a cut text when no face of the family maps
/// [`ELLIPSIS`].
const ASCII_ELLIPSIS: &str = "...";

/// A font family: an ordered list of loaded faces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FontId(usize);

/// The vertical metrics of a face in em, with the y-axis pointing up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontMetrics {
    /// The distance from the baseline to the top of the line.
    pub ascent: f32,
    /// The distance from the baseline to the bottom of the line. It is
    /// negative.
    pub descent: f32,
    /// The extra space between two lines.
    pub line_gap: f32,
    /// The height of a flat capital letter.
    pub cap_height: f32,
    /// The height of a flat lowercase letter.
    pub x_height: f32,
}

impl FontMetrics {
    /// The distance between the baselines of two lines.
    pub fn line_height(&self) -> f32 {
        self.ascent - self.descent + self.line_gap
    }
}

/// A glyph of a shaped run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RunGlyph {
    /// The horizontal position of the glyph origin in em.
    pub x: f32,
    /// The outline in the Slug arrays. A glyph without an outline, a space for
    /// example, has none.
    pub shape: Option<SlugShape>,
    /// The byte offset of the character of the glyph in the text.
    pub cluster: usize,
    /// The face and the glyph id. A default ignorable character has none.
    pub(crate) source: Option<(usize, GlyphId)>,
    /// The kerning against the next glyph that the advance of the glyph
    /// holds, in em.
    pub(crate) kerning: f32,
}

impl RunGlyph {
    /// A glyph of the character at the byte offset `cluster` without a face.
    #[cfg(test)]
    pub(crate) fn plain(shape: Option<SlugShape>, cluster: usize) -> Self {
        Self {
            x: 0.0,
            shape,
            cluster,
            source: None,
            kerning: 0.0,
        }
    }
}

/// A shaped line of text. Positions are in em and scale with the font size.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Run {
    /// The glyphs in visual order.
    pub glyphs: Vec<RunGlyph>,
    /// The advance of the glyphs, one entry for each glyph.
    pub advances: Vec<f32>,
    /// The advance width of the whole run in em.
    pub advance: f32,
    /// Whether a cut removed characters of the text.
    pub truncated: bool,
}

impl Run {
    /// Appends `glyph` with `advance` at the end of the run. The glyph takes
    /// the end of the run as its position.
    pub(crate) fn push(&mut self, glyph: RunGlyph, advance: f32) {
        self.glyphs.push(RunGlyph {
            x: self.advance,
            ..glyph
        });
        self.advances.push(advance);
        self.advance += advance;
    }

    /// Removes all glyphs and keeps the buffers.
    fn clear(&mut self) {
        self.glyphs.clear();
        self.advances.clear();
        self.advance = 0.0;
    }

    /// Appends the glyphs `range` of `source` at the end of the run.
    fn extend_from(&mut self, source: &Run, range: Range<usize>) {
        for (glyph, &advance) in source.glyphs[range.clone()]
            .iter()
            .zip(&source.advances[range])
        {
            self.push(*glyph, advance);
        }
    }

    /// Appends the glyphs of `ellipsis` at the end of the run. They take the
    /// byte offset `cluster` of the first character that the cut removed.
    fn push_ellipsis(&mut self, ellipsis: &Run, cluster: usize) {
        for (glyph, &advance) in ellipsis.glyphs.iter().zip(&ellipsis.advances) {
            self.push(RunGlyph { cluster, ..*glyph }, advance);
        }
    }
}

/// A line of a wrapped text.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// The byte range of the line in the text.
    pub bytes: Range<usize>,
    /// The glyph range of the line in the shaped text.
    pub glyphs: Range<usize>,
    /// The advance width of the line in logical units.
    pub width: f32,
}

/// A loaded face.
struct Face {
    /// The parsed font.
    font: Font,
    /// The vertical metrics in em.
    metrics: FontMetrics,
    /// The number of font units in one em.
    units_per_em: f32,
    /// The converted glyphs. `None` marks a glyph without an outline.
    shapes: HashMap<GlyphId, Option<SlugShape>>,
}

/// The loaded faces and families, the Slug data of every glyph and path in
/// use and the cache of the shaped runs.
///
/// A glyph outline becomes Slug data when a run uses the glyph for the first
/// time. The cache keeps the runs of the current and the last round of
/// frames. Call [`Fonts::begin_frame`] once at the start of every frame of
/// every window. The cache frees runs only when a new round starts, so
/// without these calls it grows with every new text.
#[derive(Default)]
pub struct Fonts {
    /// The loaded faces.
    faces: Vec<Face>,
    /// The faces of every family in fallback order. No family is empty.
    families: Vec<Vec<usize>>,
    /// The Slug data of the glyphs and the paths.
    shapes: ShapeStore,
    /// The shaped runs.
    cache: RunCache,
    /// The count of the shaped texts.
    shaped: u64,
}

impl Fonts {
    /// No faces and no glyphs.
    pub fn new() -> Self {
        Self::default()
    }

    /// Loads the font file `data` as a new family that holds this face.
    pub fn add(&mut self, data: impl Into<Box<[u8]>>) -> Result<FontId, FontError> {
        let font = Font::parse(data)?;
        let units_per_em = f32::from(font.units_per_em());
        let metrics = font.metrics();
        let em = |value: i16| f32::from(value) / units_per_em;
        let metrics = FontMetrics {
            ascent: em(metrics.ascent),
            descent: em(metrics.descent),
            line_gap: em(metrics.line_gap),
            cap_height: em(metrics.cap_height),
            x_height: em(metrics.x_height),
        };
        self.faces.push(Face {
            font,
            metrics,
            units_per_em,
            shapes: HashMap::new(),
        });
        self.families.push(vec![self.faces.len() - 1]);
        Ok(FontId(self.families.len() - 1))
    }

    /// A new family of the faces of `first` followed by the faces of each of
    /// `fallbacks`. A character takes the glyph of the first face that maps
    /// it.
    pub fn family(&mut self, first: FontId, fallbacks: &[FontId]) -> FontId {
        let mut faces = self.families[first.0].clone();
        for fallback in fallbacks {
            faces.extend_from_slice(&self.families[fallback.0]);
        }
        self.families.push(faces);
        FontId(self.families.len() - 1)
    }

    /// The vertical metrics of the first face of `font`.
    pub fn metrics(&self, font: FontId) -> FontMetrics {
        self.faces[self.families[font.0][0]].metrics
    }

    /// The Slug data of every converted glyph and path.
    pub fn slug(&self) -> &SlugData {
        self.shapes.slug()
    }

    /// Converts `path` into Slug data. A path without contours returns
    /// `None`.
    pub fn add_path(&mut self, path: &Path) -> Option<SlugShape> {
        self.shapes.add_path(path)
    }

    /// Starts a frame of the window `window`, a number that tells the
    /// windows that share these fonts apart. Every window calls it once at
    /// the start of each frame.
    ///
    /// The second frame of a window in a round starts the next round, and so
    /// does a new window when the round holds many windows already. The runs
    /// that neither the last nor this round drew give their buffers to the
    /// new runs.
    pub fn begin_frame(&mut self, window: u64) {
        self.cache.begin_frame(window);
    }

    /// The number of texts shaped since the creation.
    pub fn shaped(&self) -> u64 {
        self.shaped
    }

    /// `text` shaped as one line. The run comes from the cache when the same
    /// text was shaped in this or the last round of frames, see
    /// [`Fonts::begin_frame`].
    pub fn shape(&mut self, font: FontId, text: &str) -> Arc<Run> {
        if let Some(run) = self.cache.cached(font, Fitting::Whole, text) {
            return run;
        }
        let position = self.cache.entry(font, Fitting::Whole, text);
        let mut run = self.cache.take_run(position);
        self.shape_into(font, text, &mut run);
        self.cache.put_run(position, run)
    }

    /// The advance width of `text` at `size`.
    pub fn measure(&mut self, font: FontId, size: f32, text: &str) -> f32 {
        self.shape(font, text).advance * size
    }

    /// The length in bytes of the longest start of `text` that fits into
    /// `width` at `size`. The start ends at a character boundary.
    pub fn fitting_prefix(&mut self, font: FontId, size: f32, text: &str, width: f32) -> usize {
        let run = self.shape(font, text);
        let limit = em_limit(width, size);
        let mut advance = 0.0;
        for (glyph, &glyph_advance) in run.glyphs.iter().zip(&run.advances) {
            advance += glyph_advance;
            if advance > limit {
                return glyph.cluster;
            }
        }
        text.len()
    }

    /// `text` shaped as one line that fits into `width` at `size`. A text that
    /// does not fit loses characters at its end and gets an ellipsis, and the
    /// run is marked as truncated. A width below the ellipsis gives an empty
    /// truncated run. The cut run comes from the cache when the same text was
    /// cut at the same width before.
    pub fn fit(&mut self, font: FontId, size: f32, text: &str, width: f32) -> Arc<Run> {
        let limit = em_limit(width, size);
        self.cut(font, text, limit, Fitting::End(limit.to_bits()))
    }

    /// `text` shaped as one line that always ends in an ellipsis and fits
    /// into `width` at `size`. The line loses characters at its end until
    /// the ellipsis fits, and the run is marked as truncated. A width below
    /// the ellipsis gives an empty truncated run. The cut run comes from the
    /// cache when the same text was cut at the same width before.
    pub(crate) fn fit_continued(
        &mut self,
        font: FontId,
        size: f32,
        text: &str,
        width: f32,
    ) -> Arc<Run> {
        let limit = em_limit(width, size);
        self.cut(font, text, limit, Fitting::Continued(limit.to_bits()))
    }

    /// `text` shaped as one line that fits into `width` at `size`. A text that
    /// does not fit loses characters in its middle, an ellipsis takes their
    /// place, and the run is marked as truncated. The end of the text first
    /// takes up to half of the width and then any room that the start leaves,
    /// so a file name keeps its extension. A width below the ellipsis gives
    /// an empty truncated run. The cut run comes from the cache when the same
    /// text was cut at the same width before.
    pub fn fit_middle(&mut self, font: FontId, size: f32, text: &str, width: f32) -> Arc<Run> {
        let limit = em_limit(width, size);
        self.cut(font, text, limit, Fitting::Middle(limit.to_bits()))
    }

    /// Splits `text` into lines no wider than `width` at `size`, writes them
    /// into `lines` and returns the shaped text that the glyph ranges of the
    /// lines index.
    ///
    /// A line breaks after spaces, after a zero width space and between two
    /// CJK characters. The kinsoku rules keep closing punctuation, the
    /// prolonged sound mark and small kana off the start of a line and
    /// opening brackets off its end. A line feed, a carriage return and the
    /// pair of both always break. A word wider than the width breaks at a
    /// character boundary. The lines leave out the trailing spaces and the
    /// line breaks.
    pub fn wrap(
        &mut self,
        font: FontId,
        size: f32,
        text: &str,
        width: f32,
        lines: &mut Vec<Line>,
    ) -> Arc<Run> {
        let run = self.shape(font, text);
        wrap::break_lines(&run, text, em_limit(width, size), lines);
        for line in lines.iter_mut() {
            line.width *= size;
        }
        run
    }

    /// `text` cut to the width `limit` in em as `fitting` says: in the
    /// middle, or at the end with an ellipsis. A [`Fitting::Continued`] cut
    /// ends in an ellipsis also when the whole text fits.
    fn cut(&mut self, font: FontId, text: &str, limit: f32, fitting: Fitting) -> Arc<Run> {
        let run = self.shape(font, text);
        let continued = matches!(fitting, Fitting::Continued(_));
        if run.advance <= limit && !continued {
            return run;
        }
        let middle = matches!(fitting, Fitting::Middle(_));
        if let Some(fitted) = self.cache.cached(font, fitting, text) {
            return fitted;
        }
        let ellipsis = self.ellipsis(font);
        let position = self.cache.entry(font, fitting, text);
        let mut fitted = self.cache.take_run(position);
        if ellipsis.advance <= limit {
            let room = limit - ellipsis.advance;
            let count = run.glyphs.len();
            let advances = &run.advances;
            let (mut head_end, mut tail_start) = if middle {
                let mut tail_start = count - fitting_count(advances.iter().rev(), room * 0.5);
                let tail: f32 = advances[tail_start..].iter().sum();
                let head_end = fitting_count(advances[..tail_start].iter(), room - tail);
                let head: f32 = advances[..head_end].iter().sum();
                tail_start -= fitting_count(
                    advances[head_end..tail_start].iter().rev(),
                    room - head - tail,
                );
                (head_end, tail_start)
            } else {
                (fitting_count(advances.iter(), room), count)
            };
            loop {
                self.assemble(&mut fitted, &run, &ellipsis, head_end, tail_start, text);
                if fitted.advance <= limit || (head_end == 0 && tail_start == count) {
                    break;
                }
                if head_end > 0 {
                    head_end -= 1;
                } else {
                    tail_start += 1;
                }
            }
        }
        fitted.truncated = true;
        self.cache.put_run(position, fitted)
    }

    /// Fills `fitted` with the glyphs of `run` before `head_end`, the
    /// ellipsis and the glyphs from `tail_start` on. The glyphs at the joins
    /// kern against their new neighbors.
    fn assemble(
        &self,
        fitted: &mut Run,
        run: &Run,
        ellipsis: &Run,
        head_end: usize,
        tail_start: usize,
        text: &str,
    ) {
        fitted.clear();
        fitted.extend_from(run, 0..head_end);
        join(&self.faces, fitted, ellipsis.glyphs.first());
        fitted.push_ellipsis(ellipsis, cluster_at(run, head_end, text));
        if let Some(first) = run.glyphs.get(tail_start) {
            join(&self.faces, fitted, Some(first));
            fitted.extend_from(run, tail_start..run.glyphs.len());
        }
    }

    /// The shaped ellipsis of `font`.
    fn ellipsis(&mut self, font: FontId) -> Arc<Run> {
        let mapped = self.families[font.0]
            .iter()
            .any(|&face| self.faces[face].font.glyph_index(ELLIPSIS).is_some());
        if mapped {
            self.shape(font, ELLIPSIS.encode_utf8(&mut [0; 4]))
        } else {
            self.shape(font, ASCII_ELLIPSIS)
        }
    }

    /// Shapes `text` into the empty run `run`. Every character gives one
    /// glyph.
    fn shape_into(&mut self, font: FontId, text: &str, run: &mut Run) {
        self.shaped += 1;
        let family = &self.families[font.0];
        for (cluster, character) in text.char_indices() {
            let mut glyph = RunGlyph {
                x: 0.0,
                shape: None,
                cluster,
                source: None,
                kerning: 0.0,
            };
            let mut advance = 0.0;
            if !is_default_ignorable(character) {
                let (face_index, glyph_id) = select_glyph(&self.faces, family, character);
                let face = &mut self.faces[face_index];
                glyph.shape = *face.shapes.entry(glyph_id).or_insert_with(|| {
                    convert_glyph(&face.font, glyph_id, face.units_per_em, &mut self.shapes)
                });
                glyph.source = Some((face_index, glyph_id));
                advance = f32::from(face.font.advance_width(glyph_id)) / face.units_per_em;
            }
            join(&self.faces, run, Some(&glyph));
            run.push(glyph, advance);
        }
    }
}

/// Sets the kerning in the advance of the last glyph of `run` to the kerning
/// against `next`. Two glyphs kern when they come from the same face.
fn join(faces: &[Face], run: &mut Run, next: Option<&RunGlyph>) {
    let Some(last) = run.glyphs.last_mut() else {
        return;
    };
    let kerning = match (last.source, next.and_then(|glyph| glyph.source)) {
        (Some((face, left)), Some((next_face, right))) if face == next_face => {
            let face = &faces[face];
            f32::from(face.font.kerning(left, right)) / face.units_per_em
        }
        _ => 0.0,
    };
    let change = kerning - last.kerning;
    last.kerning = kerning;
    if let Some(advance) = run.advances.last_mut() {
        *advance += change;
    }
    run.advance += change;
}

/// The face and the glyph of `character` in the family of the faces
/// `family`. A control character and a character that no face maps take the
/// `.notdef` glyph of the first face.
fn select_glyph(faces: &[Face], family: &[usize], character: char) -> (usize, GlyphId) {
    if !character.is_control() {
        for &face in family {
            if let Some(glyph) = faces[face].font.glyph_index(character) {
                return (face, glyph);
            }
        }
    }
    (family[0], GlyphId::NOTDEF)
}

/// Whether `character` has the Unicode property Default_Ignorable_Code_Point.
/// Such a character takes no glyph outline and no advance.
fn is_default_ignorable(character: char) -> bool {
    matches!(
        character,
        '\u{AD}'
            | '\u{34F}'
            | '\u{61C}'
            | '\u{115F}'..='\u{1160}'
            | '\u{17B4}'..='\u{17B5}'
            | '\u{180B}'..='\u{180F}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{3164}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FEFF}'
            | '\u{FFA0}'
            | '\u{FFF0}'..='\u{FFF8}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0000}'..='\u{E0FFF}'
    )
}

/// Converts the outline of `glyph` with `units_per_em` font units in one em
/// into Slug data. A glyph without contours or with a broken outline has no
/// shape.
fn convert_glyph(
    font: &Font,
    glyph: GlyphId,
    units_per_em: f32,
    shapes: &mut ShapeStore,
) -> Option<SlugShape> {
    let em = units_per_em.recip();
    shapes
        .add_outline([em, em], [0.0, 0.0], |sink| font.outline(glyph, sink))
        .ok()
        .flatten()
}

/// The room in em that a width in logical units leaves above the exact
/// quotient. A text then fits the width that [`Fonts::measure`] gave for it
/// despite the rounding of the multiplication and the division.
const FIT_TOLERANCE: f32 = 1e-4;

/// The width in em of `width` logical units at `size`, with the tolerance
/// of the rounding. A negative or NaN result counts as zero.
fn em_limit(width: f32, size: f32) -> f32 {
    let limit = width / size;
    if limit > 0.0 {
        limit + FIT_TOLERANCE
    } else {
        0.0
    }
}

/// The byte offset of the character of the glyph `index` of `run`, the shaped
/// `text`, or the length of `text` past the last glyph.
fn cluster_at(run: &Run, index: usize, text: &str) -> usize {
    run.glyphs
        .get(index)
        .map_or(text.len(), |glyph| glyph.cluster)
}

/// The number of leading `advances` whose sum stays within `room`.
fn fitting_count<'a>(advances: impl Iterator<Item = &'a f32>, room: f32) -> usize {
    let mut used = 0.0;
    let mut count = 0;
    for &advance in advances {
        if used + advance > room {
            break;
        }
        used += advance;
        count += 1;
    }
    count
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::OnceLock;

    use super::*;
    use crate::{
        Color, Rect, Scene,
        truetype::tests::{font_file, kern_table, small_font_tables},
    };

    /// The path of the font file below the repository root.
    const REPOSITORY_FONT_PATH: &str = "font/SarasaUiJ-Regular.ttf";

    /// Returns the path of the font file in the repository.
    fn repository_font_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(REPOSITORY_FONT_PATH)
    }

    /// The bytes of the font file in the repository.
    pub(crate) fn font_data() -> &'static [u8] {
        static DATA: OnceLock<Vec<u8>> = OnceLock::new();
        DATA.get_or_init(|| {
            std::fs::read(repository_font_path()).expect("the font file is missing")
        })
    }

    /// The parsed font file of the repository.
    pub(crate) fn font() -> &'static Font {
        static FONT: OnceLock<Font> = OnceLock::new();
        FONT.get_or_init(|| Font::parse(font_data()).expect("the font does not parse"))
    }

    /// Fonts with the font of the repository as their only family.
    fn fonts() -> (Fonts, FontId) {
        let mut fonts = Fonts::new();
        let font = fonts.add(font_data()).unwrap();
        (fonts, font)
    }

    /// The characters of `text` that the glyphs of `run` belong to, with the
    /// glyph of the ellipsis of `font` as the ellipsis.
    fn characters(fonts: &mut Fonts, font: FontId, run: &Run, text: &str) -> String {
        let ellipsis = fonts.shape(font, "\u{2026}").glyphs[0].shape;
        run.glyphs
            .iter()
            .map(|glyph| {
                if glyph.shape == ellipsis {
                    '\u{2026}'
                } else {
                    text[glyph.cluster..].chars().next().unwrap()
                }
            })
            .collect()
    }

    /// Asserts that the glyphs of `run` sit at the sums of the advances and
    /// that their clusters are character boundaries of `text`.
    fn assert_consistent(run: &Run, text: &str) {
        assert_eq!(run.glyphs.len(), run.advances.len());
        let mut x = 0.0;
        for (glyph, advance) in run.glyphs.iter().zip(&run.advances) {
            assert!((glyph.x - x).abs() < 1e-4, "{run:?}");
            assert!(text.is_char_boundary(glyph.cluster), "{text:?}");
            assert!(glyph.cluster <= text.len());
            x += advance;
        }
        assert!((run.advance - x).abs() < 1e-5);
    }

    #[test]
    fn the_font_file_name_matches_the_repository() {
        assert!(repository_font_path().is_file());
        assert!(font().glyph_index('A').is_some());
    }

    /// The folder of the language files below the repository root.
    const REPOSITORY_LANGUAGE_FOLDER: &str = "language";

    /// The file extension of a language file.
    const LANGUAGE_FILE_EXTENSION: &str = "ini";

    /// Verifies that the font of the repository holds a glyph for every character of every
    /// language file of the repository.
    #[test]
    fn the_font_covers_every_language_file() {
        let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(REPOSITORY_LANGUAGE_FOLDER);
        let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&folder)
            .expect("the language folder reads")
            .map(|entry| entry.expect("the language folder lists").path())
            .filter(|path| {
                path.extension().is_some_and(|extension| {
                    extension.eq_ignore_ascii_case(LANGUAGE_FILE_EXTENSION)
                })
            })
            .collect();
        files.sort();
        let names: Vec<String> = files
            .iter()
            .filter_map(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .collect();
        assert!(names.iter().any(|name| name == "en.ini"), "{names:?}");
        assert!(names.iter().any(|name| name == "ja.ini"), "{names:?}");
        for path in &files {
            let source = std::fs::read_to_string(path).expect("the language file reads");
            let missing: std::collections::BTreeSet<char> = source
                .chars()
                .filter(|character| !character.is_control())
                .filter(|character| font().glyph_index(*character).is_none())
                .collect();
            assert!(missing.is_empty(), "{}: {missing:?}", path.display());
        }
    }

    #[test]
    fn broken_data_fails_to_load() {
        let mut fonts = Fonts::new();
        assert!(fonts.add(&font_data()[..1000]).is_err());
        assert!(fonts.add(Vec::new()).is_err());
    }

    #[test]
    fn metrics_are_in_em() {
        let (fonts, font) = fonts();
        let metrics = fonts.metrics(font);
        assert_eq!(
            metrics,
            FontMetrics {
                ascent: 0.969,
                descent: -0.241,
                line_gap: 0.0,
                cap_height: 0.727,
                x_height: 0.545,
            }
        );
        assert!((metrics.line_height() - 1.21).abs() < 1e-6);
    }

    #[test]
    fn kerning_narrows_a_pair() {
        let (mut fonts, font) = fonts();
        let pair = fonts.measure(font, 1.0, "AV");
        let single = fonts.measure(font, 1.0, "A") + fonts.measure(font, 1.0, "V");
        assert!((single - pair - 0.068).abs() < 1e-6, "{pair} {single}");
        let run = fonts.shape(font, "AV");
        assert!((run.advances[0] - (0.69 - 0.068)).abs() < 1e-6);
        assert_eq!(run.glyphs[1].x, run.advances[0]);
    }

    #[test]
    fn widths_scale_with_the_size() {
        let (mut fonts, font) = fonts();
        let small = fonts.measure(font, 10.0, "Disk 1 \u{30C7}\u{30A3}\u{30B9}\u{30AF}");
        let large = fonts.measure(font, 20.0, "Disk 1 \u{30C7}\u{30A3}\u{30B9}\u{30AF}");
        assert!(small > 0.0);
        assert!((large - small * 2.0).abs() < 1e-4);
        assert_eq!(fonts.measure(font, 16.0, "漢字"), 32.0);
    }

    #[test]
    fn a_long_text_gets_cut_with_an_ellipsis() {
        let (mut fonts, font) = fonts();
        let text = "A very long disk image name.d88";
        let full = fonts.shape(font, text);
        let fitted = fonts.fit(font, 10.0, text, 60.0);
        assert!(fitted.advance * 10.0 <= 60.0);
        assert!(fitted.glyphs.len() < full.glyphs.len());
        assert_consistent(&fitted, text);
        let ellipsis = fonts.shape(font, "\u{2026}");
        let last = fitted.glyphs.last().unwrap();
        assert_eq!(last.shape, ellipsis.glyphs[0].shape);
        assert_eq!(last.x, fitted.advance - ellipsis.advance);
        let kept = fitted.glyphs.len() - 1;
        assert_eq!(
            characters(&mut fonts, font, &fitted, text)[..kept],
            text[..kept]
        );
        assert_eq!(last.cluster, kept);
        assert!(fitted.truncated);
        let again = fonts.fit(font, 10.0, text, 60.0);
        assert!(Arc::ptr_eq(&fitted, &again));
        let short = fonts.fit(font, 10.0, "Disk", 60.0);
        assert!(!short.truncated);
        assert_eq!(short, fonts.shape(font, "Disk"));
        assert!(fonts.fit(font, 10.0, text, 1.0).glyphs.is_empty());
    }

    /// Verifies that a continued cut ends in an ellipsis inside its width, for a line that fits
    /// and for a line that loses characters.
    #[test]
    fn a_continued_line_ends_in_an_ellipsis() {
        let (mut fonts, font) = fonts();
        let ellipsis = fonts.shape(font, "\u{2026}");
        let short = fonts.fit_continued(font, 10.0, "Disk", 60.0);
        assert!(short.truncated);
        assert_eq!(short.glyphs.len(), 5);
        assert_eq!(short.glyphs.last().unwrap().shape, ellipsis.glyphs[0].shape);
        assert_consistent(&short, "Disk");
        let text = "A very long disk image name.d88";
        let long = fonts.fit_continued(font, 10.0, text, 60.0);
        assert!(long.advance * 10.0 <= 60.0);
        assert_eq!(long.glyphs.last().unwrap().shape, ellipsis.glyphs[0].shape);
        assert_eq!(*long, *fonts.fit(font, 10.0, text, 60.0));
        assert!(Arc::ptr_eq(
            &long,
            &fonts.fit_continued(font, 10.0, text, 60.0)
        ));
        assert!(
            fonts
                .fit_continued(font, 10.0, "Disk", 1.0)
                .glyphs
                .is_empty()
        );
    }

    /// Verifies that a text fits the width that its measure gave at every size.
    #[test]
    fn a_text_fits_its_own_measure() {
        let (mut fonts, font) = fonts();
        for text in [
            "config/pc88/thexder.conf",
            "config/pc98/yu-no.conf",
            "YU-NO (Disk 3 of 4).hdm",
            "ディスク画像",
        ] {
            for size in [10.0, 12.0, 13.5, 14.0, 17.0, 24.0] {
                let width = fonts.measure(font, size, text);
                assert!(
                    !fonts.fit(font, size, text, width).truncated,
                    "{text} {size}"
                );
                assert!(
                    !fonts.fit_middle(font, size, text, width).truncated,
                    "{text} {size}"
                );
                assert!(fonts.fit(font, size, text, width * 0.9).truncated);
            }
        }
    }

    #[test]
    fn a_prefix_fits_the_width() {
        let (mut fonts, font) = fonts();
        let text = "Disk 01 long";
        let width = fonts.measure(font, 10.0, "Disk 01");
        let length = fonts.fitting_prefix(font, 10.0, text, width + 0.01);
        assert_eq!(&text[..length], "Disk 01");
        assert_eq!(fonts.fitting_prefix(font, 10.0, text, 1000.0), text.len());
        let japanese = "ディスク画像";
        let width = fonts.measure(font, 10.0, "ディスク");
        let length = fonts.fitting_prefix(font, 10.0, japanese, width + 0.01);
        assert_eq!(&japanese[..length], "ディスク");
        assert_eq!(fonts.fitting_prefix(font, 10.0, japanese, 0.0), 0);
    }

    #[test]
    fn glyphs_are_converted_once() {
        let (mut fonts, font) = fonts();
        fonts.shape(font, "abc");
        let glyphs = fonts.slug().glyphs().len();
        assert_eq!(glyphs, 3);
        fonts.shape(font, "cab abba");
        assert_eq!(fonts.slug().glyphs().len(), glyphs);
        let run = fonts.shape(font, "a ba");
        assert!(run.glyphs[1].shape.is_none());
        assert!(run.advances[1] > 0.0);
        assert_eq!(run.glyphs[0].shape, run.glyphs[3].shape);
        assert_eq!(fonts.slug().glyphs().len(), glyphs);
    }

    #[test]
    fn the_cache_shapes_a_text_once() {
        let (mut fonts, font) = fonts();
        let first = fonts.shape(font, "Disk");
        let shaped = fonts.shaped();
        let second = fonts.shape(font, "Disk");
        assert_eq!(fonts.shaped(), shaped);
        assert!(Arc::ptr_eq(&first, &second));
        fonts.fit(font, 10.0, "Disk", 5.0);
        fonts.fit_middle(font, 10.0, "Disk", 5.0);
        assert_eq!(fonts.cache.len(), 4);
        let text = "Disk image.d88";
        assert_ne!(
            fonts.fit(font, 10.0, text, 40.0),
            fonts.fit_middle(font, 10.0, text, 40.0)
        );
    }

    #[test]
    fn two_windows_share_a_round() {
        let (mut fonts, font) = fonts();
        fonts.begin_frame(1);
        fonts.shape(font, "one");
        fonts.begin_frame(2);
        fonts.shape(font, "two");
        fonts.begin_frame(1);
        fonts.shape(font, "one");
        fonts.begin_frame(2);
        assert_eq!(fonts.cache.len(), 2);
        fonts.begin_frame(1);
        assert_eq!(fonts.cache.len(), 1);
        let shaped = fonts.shaped();
        fonts.shape(font, "one");
        assert_eq!(fonts.shaped(), shaped);
    }

    #[test]
    fn many_windows_start_new_rounds() {
        let (mut fonts, font) = fonts();
        fonts.begin_frame(0);
        fonts.shape(font, "old");
        for window in 1..=2 * cache::WINDOW_LIMIT as u64 {
            fonts.begin_frame(window);
        }
        assert_eq!(fonts.cache.len(), 0);
    }

    #[test]
    fn every_width_caches_its_own_cut() {
        let (mut fonts, font) = fonts();
        let text = "A very long disk image name.d88";
        let narrow = fonts.fit(font, 10.0, text, 60.0);
        let wide = fonts.fit(font, 10.0, text, 90.0);
        assert!(narrow.advance < wide.advance);
        assert!(Arc::ptr_eq(&narrow, &fonts.fit(font, 10.0, text, 60.0)));
        assert!(Arc::ptr_eq(&wide, &fonts.fit(font, 10.0, text, 90.0)));
        let middle = fonts.fit_middle(font, 10.0, text, 60.0);
        assert_ne!(*middle, *narrow);
        assert!(Arc::ptr_eq(
            &middle,
            &fonts.fit_middle(font, 10.0, text, 60.0)
        ));
    }

    #[test]
    fn unused_runs_give_their_entries_to_new_runs() {
        let (mut fonts, font) = fonts();
        fonts.begin_frame(0);
        fonts.shape(font, "old");
        fonts.shape(font, "kept");
        fonts.begin_frame(0);
        fonts.shape(font, "kept");
        fonts.begin_frame(0);
        assert_eq!(fonts.cache.len(), 1);
        fonts.shape(font, "new");
        assert_eq!(fonts.cache.len(), 2);
        assert_eq!(fonts.cache.entry_count(), 2);
        let shaped = fonts.shaped();
        fonts.shape(font, "kept");
        assert_eq!(fonts.shaped(), shaped);
    }

    #[test]
    fn a_full_cache_reuses_the_runs_of_old_rounds() {
        let (mut fonts, font) = fonts();
        fonts.begin_frame(0);
        for number in 0..cache::RUN_CACHE_LIMIT {
            fonts.shape(font, &number.to_string());
        }
        fonts.begin_frame(0);
        fonts.shape(font, "new");
        assert_eq!(fonts.cache.entry_count(), cache::RUN_CACHE_LIMIT);
        assert_eq!(fonts.cache.len(), 1);
    }

    #[test]
    fn control_characters_take_the_notdef_glyph() {
        let (mut fonts, font) = fonts();
        let text = "a\u{0}\t\n\u{7F}\u{9F}b";
        let run = fonts.shape(font, text);
        assert_eq!(run.glyphs.len(), 7);
        let notdef = run.glyphs[1];
        assert!(notdef.shape.is_some());
        for index in 2..6 {
            assert_eq!(run.glyphs[index].shape, notdef.shape);
            assert_eq!(run.advances[index], run.advances[1]);
        }
        let missing = fonts.shape(font, "\u{10FFFD}");
        assert_eq!(missing.glyphs[0].shape, notdef.shape);
    }

    /// A text from random bytes that holds characters of every UTF-8 length,
    /// control characters, combining marks and unmapped characters.
    fn random_text(state: &mut u64, length: usize) -> String {
        const POOL: &[char] = &[
            'a',
            'W',
            ' ',
            '\r',
            '.',
            '\u{0}',
            '\n',
            '\u{301}',
            '\u{E9}',
            '\u{2026}',
            '\u{3042}',
            '\u{6F22}',
            '\u{D55C}',
            '\u{FEFF}',
            '\u{200B}',
            '\u{10FFFD}',
            '\u{1F600}',
            '\u{3002}',
            '\u{300C}',
            '\u{20B9F}',
            '\u{A0}',
        ];
        (0..length)
            .map(|_| {
                *state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                POOL[(*state >> 33) as usize % POOL.len()]
            })
            .collect()
    }

    #[test]
    fn any_text_cuts_and_wraps_on_character_boundaries() {
        let (mut fonts, font) = fonts();
        let mut state = 1;
        let mut lines = Vec::new();
        for length in 0..40 {
            let text = random_text(&mut state, length);
            for size in [10.0, 0.0, -10.0, f32::NAN, f32::INFINITY] {
                for width in [-1.0, 0.0, 3.0, 9.0, 25.0, 80.0, f32::NAN, f32::INFINITY] {
                    let limit = em_limit(width, size);
                    let whole = fonts.shape(font, &text);
                    assert_consistent(&whole, &text);
                    for fitted in [
                        fonts.fit(font, size, &text, width),
                        fonts.fit_middle(font, size, &text, width),
                    ] {
                        assert_consistent(&fitted, &text);
                        assert!(fitted.advance <= limit + 1e-4, "{text:?} {size} {width}");
                        assert_eq!(fitted.truncated, whole.advance > limit);
                        if !fitted.truncated {
                            assert_eq!(*fitted, *whole);
                        }
                    }
                    let prefix = fonts.fitting_prefix(font, size, &text, width);
                    assert!(text.is_char_boundary(prefix));
                    let run = fonts.wrap(font, size, &text, width, &mut lines);
                    assert_wrapped(&run, &text, &lines, limit);
                }
            }
        }
    }

    /// Asserts that `lines` split the shaped `text` in `run` at character
    /// boundaries, that a line is empty only on an empty or blank source
    /// line, and that only a line of one visible character exceeds `limit`.
    fn assert_wrapped(run: &Run, text: &str, lines: &[Line], limit: f32) {
        let position = |index: usize| run.glyphs.get(index).map_or(run.advance, |glyph| glyph.x);
        assert!(!lines.is_empty());
        let mut end = 0;
        for line in lines {
            assert!(line.bytes.start >= end && line.bytes.end >= line.bytes.start);
            assert!(text.is_char_boundary(line.bytes.start));
            assert!(text.is_char_boundary(line.bytes.end));
            assert_eq!(cluster_at(run, line.glyphs.start, text), line.bytes.start);
            assert_eq!(cluster_at(run, line.glyphs.end, text), line.bytes.end);
            end = line.bytes.end;
            if line.bytes.is_empty() {
                let breaks = ['\n', '\r'];
                let source_start = text[..line.bytes.start]
                    .rfind(breaks)
                    .map_or(0, |index| index + 1);
                let source_end = text[line.bytes.start..]
                    .find(breaks)
                    .map_or(text.len(), |index| line.bytes.start + index);
                assert!(
                    text[source_start..source_end].chars().all(wrap::is_space),
                    "{text:?}"
                );
            }
            if position(line.glyphs.end) - position(line.glyphs.start) > limit + 1e-4 {
                let visible = line
                    .glyphs
                    .clone()
                    .filter(|&index| {
                        run.advances[index] > 0.0
                            && !text[run.glyphs[index].cluster..].starts_with(wrap::is_space)
                    })
                    .count();
                assert!(visible <= 1, "{text:?} {limit} {line:?}");
            }
        }
    }

    #[test]
    fn default_ignorable_characters_take_no_room() {
        let (mut fonts, font) = fonts();
        let text = "a\u{FE0F}\u{200D}b\u{AD}\u{200B}\u{E0101}\u{2060}";
        let run = fonts.shape(font, text);
        assert_consistent(&run, text);
        assert_eq!(run.glyphs.len(), 8);
        for index in [1, 2, 4, 5, 6, 7] {
            assert_eq!(run.glyphs[index].shape, None);
            assert_eq!(run.advances[index], 0.0);
        }
        assert_eq!(run.advance, fonts.measure(font, 1.0, "ab"));
    }

    #[test]
    fn the_glyph_before_the_ellipsis_drops_the_kerning_of_the_cut() {
        let (mut fonts, font) = fonts();
        let joined = fonts.shape(font, "A\u{2026}");
        let kerned = fonts.shape(font, "AV").advances[0];
        assert!(joined.advances[0] > kerned);
        let fitted = fonts.fit(font, 1.0, "AVVVVVVV", joined.advance + 0.01);
        assert_eq!(fitted.glyphs.len(), 2);
        assert!((fitted.advances[0] - joined.advances[0]).abs() < 1e-6);
        assert!((fitted.advance - joined.advance).abs() < 1e-6);

        let tail = fonts.shape(font, "\u{2026}V");
        let width = joined.advances[0] + tail.advance + 0.01;
        let fitted = fonts.fit_middle(font, 1.0, "AVVVVVVV", width);
        assert_eq!(fitted.glyphs.len(), 3);
        assert!((fitted.advances[0] - joined.advances[0]).abs() < 1e-6);
        assert!((fitted.advances[1] - tail.advances[0]).abs() < 1e-6);
        assert!(fitted.advance <= width);
    }

    #[test]
    fn the_middle_ellipsis_keeps_the_end() {
        let (mut fonts, font) = fonts();
        let text = "Super Long Adventure Game Disk Image (Disk 2 of 3).d88";
        let fitted = fonts.fit_middle(font, 10.0, text, 150.0);
        assert!(fitted.advance * 10.0 <= 150.0);
        assert_consistent(&fitted, text);
        let shown = characters(&mut fonts, font, &fitted, text);
        let (head, tail) = shown.split_once('\u{2026}').unwrap();
        assert!(text.starts_with(head) && !head.is_empty());
        assert!(text.ends_with(tail) && tail.ends_with(").d88"));
        assert!(head.len() + tail.len() < text.len());
        let room = 15.0 - fonts.measure(font, 1.0, "\u{2026}");
        let used = fitted.advance - fonts.measure(font, 1.0, "\u{2026}");
        let next = fonts.measure(font, 1.0, "W");
        assert!(room - used < next, "{room} {used}");
        let again = fonts.fit_middle(font, 10.0, text, 150.0);
        assert!(Arc::ptr_eq(&fitted, &again));

        let japanese = "とても長いディスクイメージの名前です.d88";
        let fitted = fonts.fit_middle(font, 10.0, japanese, 100.0);
        let shown = characters(&mut fonts, font, &fitted, japanese);
        let (head, tail) = shown.split_once('\u{2026}').unwrap();
        assert!(japanese.starts_with(head) && japanese.ends_with(tail));
        assert!(tail.ends_with("す.d88"), "{shown}");
        assert!(fitted.advance * 10.0 <= 100.0);
    }

    #[test]
    fn a_family_falls_back_to_the_japanese_face() {
        let japanese = font();
        let v = japanese.glyph_index('V').unwrap().0;
        let mut tables = small_font_tables();
        tables.push((
            crate::truetype::Tag::KERN,
            kern_table(&[(1, 2, -80), (1, v, -300)]),
        ));
        let mut fonts = Fonts::new();
        let latin = fonts.add(font_file(&tables)).unwrap();
        let sarasa = fonts.add(font_data()).unwrap();
        let family = fonts.family(latin, &[sarasa]);

        let text = "AB\u{6F22}AV\u{10FFFD}";
        let run = fonts.shape(family, text);
        assert_consistent(&run, text);
        let kanji = fonts.measure(sarasa, 1.0, "\u{6F22}");
        let letter = fonts.measure(sarasa, 1.0, "V");
        let expected = [0.52, 0.6, kanji, 0.6, letter, 0.5];
        assert_eq!(run.advances.len(), expected.len());
        for (advance, expected) in run.advances.iter().zip(expected) {
            assert!((advance - expected).abs() < 1e-6, "{:?}", run.advances);
        }
        let shapes: Vec<_> = run.glyphs.iter().map(|glyph| glyph.shape).collect();
        assert!((shapes[0].unwrap().bounds.x_max - 0.1).abs() < 1e-6);
        assert_eq!(shapes[2], fonts.shape(sarasa, "\u{6F22}").glyphs[0].shape);
        assert_eq!(shapes[4], fonts.shape(sarasa, "V").glyphs[0].shape);
        assert_eq!(shapes[5], None);
        assert_eq!(fonts.metrics(family), fonts.metrics(latin));

        let reversed = fonts.family(sarasa, &[latin]);
        let run = fonts.shape(reversed, "A\u{10FFFD}");
        assert_eq!(
            run.glyphs[0].shape,
            fonts.shape(sarasa, "A").glyphs[0].shape
        );
        assert_eq!(
            run.glyphs[1].shape,
            fonts.shape(sarasa, "\u{0}").glyphs[0].shape
        );
        assert!(run.glyphs[1].shape.is_some());
    }

    #[test]
    fn a_family_without_the_ellipsis_cuts_with_periods() {
        let mut fonts = Fonts::new();
        let latin = fonts.add(font_file(&small_font_tables())).unwrap();
        let fitted = fonts.fit(latin, 1.0, "ABABAB", 3.0);
        assert_eq!(fitted.advances, [0.6, 0.6, 0.5, 0.5, 0.5]);
        assert_eq!(fitted.glyphs[2].cluster, 2);
        assert_eq!(fitted.glyphs[4].cluster, 2);
    }

    #[test]
    fn japanese_text_wraps_with_kinsoku() {
        let (mut fonts, font) = fonts();
        let text = "これは日本語の文章です。「括弧」もあります。";
        let mut lines = Vec::new();
        let run = fonts.wrap(font, 10.0, text, 50.0, &mut lines);
        let shown: Vec<&str> = lines.iter().map(|line| &text[line.bytes.clone()]).collect();
        assert_eq!(
            shown,
            [
                "これは日本",
                "語の文章で",
                "す。「括",
                "弧」もあり",
                "ます。"
            ]
        );
        assert_eq!(lines[0].glyphs, 0..5);
        assert_eq!(lines[0].width, 50.0);
        assert_eq!(lines[4].glyphs, 19..22);
        assert!(Arc::ptr_eq(&run, &fonts.shape(font, text)));
        let latin = "Insert the disk into drive 1";
        fonts.wrap(font, 10.0, latin, 100.0, &mut lines);
        for line in &lines {
            let measured = fonts.measure(font, 10.0, &latin[line.bytes.clone()]);
            assert!(measured <= 100.0);
            assert!((line.width - measured).abs() < 0.5);
            assert!(!latin[line.bytes.clone()].starts_with(' '));
        }
        assert!(lines.len() > 1);
    }

    /// A text of `length` characters of Latin letters, digits, spaces, kana
    /// and kanji. Every `seed` gives another text.
    fn mixed_text(seed: u64, length: usize) -> String {
        const POOL: &[char] = &[
            'a', 'e', 'i', 'o', 'n', 's', 't', 'r', 'A', 'V', 'T', 'W', 'L', 'y', '.', ',', '0',
            '7', ' ', ' ', ' ', 'あ', 'の', 'ア', 'ス', 'ク', '日', '本', '語', '画', '像', '。',
            '「', '」',
        ];
        let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        (0..length)
            .map(|_| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                POOL[(state >> 33) as usize % POOL.len()]
            })
            .collect()
    }

    /// The mean time in microseconds of `operation` over `count` runs. Every
    /// run takes its own index.
    fn time_each(count: u64, mut operation: impl FnMut(u64)) -> f64 {
        let start = std::time::Instant::now();
        for index in 0..count {
            operation(index);
        }
        start.elapsed().as_secs_f64() * 1e6 / count as f64
    }

    #[test]
    #[ignore = "prints timings, run with --release --nocapture"]
    fn text_benchmark() {
        let (mut fonts, font) = fonts();
        fonts.shape(font, &mixed_text(u64::MAX, 30_000));
        let mut lines = Vec::new();
        let mut seed = 0;
        println!("chars    shape   measure       fit  fit_middle      wrap   (us, us/char)");
        for length in [100, 1_000, 10_000, 30_000] {
            let count = (300_000 / length).clamp(3, 300) as u64;
            let texts: Vec<Vec<String>> = (0..5)
                .map(|_| {
                    (0..count)
                        .map(|_| {
                            seed += 1;
                            mixed_text(seed, length)
                        })
                        .collect()
                })
                .collect();
            let timings = [
                time_each(count, |index| {
                    fonts.shape(font, &texts[0][index as usize]);
                }),
                time_each(count, |index| {
                    fonts.measure(font, 14.0, &texts[1][index as usize]);
                }),
                time_each(count, |index| {
                    fonts.fit(font, 14.0, &texts[2][index as usize], 300.0);
                }),
                time_each(count, |index| {
                    fonts.fit_middle(font, 14.0, &texts[3][index as usize], 300.0);
                }),
                time_each(count, |index| {
                    fonts.wrap(font, 14.0, &texts[4][index as usize], 300.0, &mut lines);
                }),
            ];
            print!("{length:>6}");
            for timing in timings {
                print!(" {timing:>9.1}/{:<5.3}", timing / length as f64);
            }
            println!();
            fonts.begin_frame(0);
            fonts.begin_frame(0);
        }

        let characters = concat!(
            "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789",
            "あいうえおかきくけこさしすせそたちつてとアイウエオカキクケコ日本語画像漢字表示"
        );
        let glyphs = characters.chars().count() as f64;
        let mut fresh = Fonts::new();
        let fresh_font = fresh.add(font_data()).unwrap();
        let first = time_each(1, |_| {
            fresh.shape(fresh_font, characters);
        }) / glyphs;
        let mut reversed: String = characters.chars().rev().collect();
        let later = time_each(100, |index| {
            reversed.push(char::from(b'a' + (index % 26) as u8));
            fresh.shape(fresh_font, &reversed);
        }) / glyphs;
        println!("glyph first use {first:.2} us, later use {later:.3} us");
    }

    #[test]
    #[ignore = "writes a screenshot into the directory of GUI_SHOTS"]
    fn text_screenshot() {
        const WIDTH: u32 = 1280;
        const HEIGHT: u32 = 1100;
        const SAMPLES: [&str; 5] = [
            "The quick brown fox jumps over the lazy dog. AV To LT 0123456789 (Disk 1).d88",
            "いろはにほへと ちりぬるを カタカナ 漢字の表示テスト。「引用」と半角ｶﾅ",
            "다람쥐 헌 쳇바퀴에 타고파. 한국어 디스크 이미지",
            "Съешь же ещё этих мягких французских булок, да выпей чаю. Ґанок",
            "Tiếng Việt có dấu, Română: șțăîâ, Ελληνικά, Français: œuvre",
        ];
        let (mut fonts, font) = fonts();
        let metrics = fonts.metrics(font);
        let text = Color::hex(0xE6E6E6);
        let mut scene = Scene::new();
        scene.begin(1.0, Color::hex(0x202228));
        let mut y = 4.0;
        for size in [10.0, 12.0, 14.0, 16.0, 20.0] {
            for sample in SAMPLES {
                y += metrics.ascent * size;
                scene.run((16.0, y), size, &fonts.shape(font, sample), text);
                y -= metrics.descent * size;
            }
            y += 6.0;
        }
        for size in [28.0, 44.0, 72.0] {
            y += metrics.ascent * size;
            let run = fonts.shape(font, "Aa Wg あア漢字 한글 Жж ếș");
            scene.run((16.0, y), size, &run, text);
            y -= metrics.descent * size;
        }

        let panel = Rect::new(16.0, y + 12.0, 1248.0, 220.0);
        scene.fill(panel, 8.0, Color::hex(0xF2F2F2));
        let dark = Color::hex(0x202020);
        let name = "とても長いディスクイメージの名前です Super Long Adventure (Disk 2 of 3).d88";
        let mut line = panel.y + 12.0;
        for width in [180.0, 260.0, 400.0] {
            line += 26.0;
            let fitted = fonts.fit(font, 14.0, name, width);
            scene.fill(
                Rect::new(32.0, line - 14.0, width, 18.0),
                0.0,
                Color::hex(0xD8E0F0),
            );
            scene.run((32.0, line), 14.0, &fitted, dark);
            let fitted = fonts.fit_middle(font, 14.0, name, width);
            scene.fill(
                Rect::new(460.0, line - 14.0, width, 18.0),
                0.0,
                Color::hex(0xD8E0F0),
            );
            scene.run((460.0, line), 14.0, &fitted, dark);
        }
        let paragraph = "これは日本語の文章を折り返すテストです。「括弧」や句読点、小さい「ゃ」「っ」は行頭に来ません。Latin words wrap at spaces.";
        let mut lines = Vec::new();
        let run = fonts.wrap(font, 16.0, paragraph, 300.0, &mut lines);
        scene.fill(
            Rect::new(880.0, panel.y + 8.0, 360.0, 200.0),
            0.0,
            Color::hex(0xD8E0F0),
        );
        let mut baseline = panel.y + 8.0;
        for line in &lines {
            baseline += metrics.line_height() * 16.0;
            let left = 880.0 + (360.0 - line.width) * 0.5;
            scene.run_glyphs((left, baseline), 16.0, &run, line.glyphs.clone(), dark);
        }

        let mut output = vec![0; WIDTH as usize * HEIGHT as usize * 4];
        crate::Rasterizer::new().rasterize(&scene, fonts.slug(), WIDTH, HEIGHT, &mut output);
        crate::rasterizer::tests::write_ppm("text_sample", WIDTH, HEIGHT, &output);
    }
}
