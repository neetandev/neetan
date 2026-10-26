//! The label widget.

#[cfg(any(test, feature = "fill-counter"))]
use std::cell::Cell;
use std::sync::Arc;

use super::context::{TextStyle, UiContext};
use crate::{FontId, Line, Rect, Run};

/// The distance between the baselines of two lines of a wrapped label, in
/// units of the text size.
pub const WRAP_LINE_HEIGHT: f32 = 1.43;

/// The horizontal placement of a text in its rectangle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    /// At the left edge.
    #[default]
    Start,
    /// In the center.
    Center,
    /// At the right edge.
    End,
}

/// The cut of a text wider than its rectangle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Truncation {
    /// The end of the text gives way to an ellipsis.
    #[default]
    End,
    /// The middle of the text gives way to an ellipsis, and the end stays.
    /// File names keep their extension this way.
    Middle,
}

/// Draws `text` in `style` in `rect`, centered vertically and placed by
/// `align`. A text wider than `rect` is cut as `truncation` says. Returns
/// whether the text was cut.
pub fn label(
    context: &mut UiContext,
    rect: Rect,
    style: &TextStyle,
    text: &str,
    align: Align,
    truncation: Truncation,
) -> bool {
    context.text_in(rect, style, text, align, truncation)
}

/// The height of one line of a wrapped label in `style`.
pub fn wrapped_line_height(style: &TextStyle) -> f32 {
    style.size * WRAP_LINE_HEIGHT
}

#[cfg(any(test, feature = "fill-counter"))]
std::thread_local! {
    /// The number of wraps of every wrapped text on the current thread.
    static WRAPS: Cell<usize> = const { Cell::new(0) };
}

/// Runs `function` and returns the number of wraps of every [`WrappedText`] on the current
/// thread during the run, together with the result of `function`.
#[cfg(any(test, feature = "fill-counter"))]
pub fn count_wraps<T>(function: impl FnOnce() -> T) -> (usize, T) {
    let before = WRAPS.with(Cell::get);
    let result = function();
    let after = WRAPS.with(Cell::get);
    (after - before, result)
}

/// The layout that a [`WrappedText`] wrapped its lines for.
#[derive(Clone, Copy, Debug, PartialEq)]
struct WrapLayout {
    /// The font family.
    font: FontId,
    /// The bits of the text size.
    size: u32,
    /// The bits of the width of the lines.
    width: u32,
    /// The most lines.
    maximum_lines: usize,
}

/// The lines of a wrapped text that a caller keeps between passes.
///
/// The text wraps again only when its shaped run, its font, its size, the
/// width or the most lines change. The run of the font cache changes with
/// the text. The lines keep their buffer between the wraps. A text with more
/// lines than the most lines shows the most lines, and the last of them ends
/// in an ellipsis.
#[derive(Clone, Debug, Default)]
pub struct WrappedText {
    /// The shaped text of the last wrap.
    run: Option<Arc<Run>>,
    /// The layout of the last wrap.
    layout: Option<WrapLayout>,
    /// The lines of the last wrap.
    lines: Vec<Line>,
    /// The last line with its ellipsis, when the text holds more lines than
    /// the last wrap shows.
    cut: Option<Arc<Run>>,
}

impl WrappedText {
    /// Empty lines without a wrap.
    pub fn new() -> Self {
        Self::default()
    }

    /// Wraps `text` in `style` into lines no wider than `width`, at most
    /// `maximum_lines` of them, unless the last wrap holds them.
    fn wrap(
        &mut self,
        context: &mut UiContext,
        style: &TextStyle,
        text: &str,
        width: f32,
        maximum_lines: usize,
    ) {
        let width = width.max(0.0);
        let layout = WrapLayout {
            font: style.font,
            size: style.size.to_bits(),
            width: width.to_bits(),
            maximum_lines,
        };
        let run = context.fonts.shape(style.font, text);
        let current = self.layout == Some(layout)
            && self
                .run
                .as_ref()
                .is_some_and(|cached| Arc::ptr_eq(cached, &run));
        if current {
            return;
        }
        let run = context
            .fonts
            .wrap(style.font, style.size, text, width, &mut self.lines);
        let maximum_lines = maximum_lines.max(1);
        self.cut = None;
        if self.lines.len() > maximum_lines {
            let last = &mut self.lines[maximum_lines - 1];
            let cut = context.fonts.fit_continued(
                style.font,
                style.size,
                &text[last.bytes.clone()],
                width,
            );
            last.width = cut.advance * style.size;
            self.cut = Some(cut);
        }
        self.lines.truncate(maximum_lines);
        self.run = Some(run);
        self.layout = Some(layout);
        #[cfg(any(test, feature = "fill-counter"))]
        WRAPS.with(|wraps| wraps.set(wraps.get() + 1));
    }

    /// The number of lines of the last wrap.
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// The height that [`WrappedText::show`] takes for `text` in `style` at
    /// the width `width` with at most `maximum_lines` lines.
    pub fn height(
        &mut self,
        context: &mut UiContext,
        style: &TextStyle,
        text: &str,
        width: f32,
        maximum_lines: usize,
    ) -> f32 {
        self.wrap(context, style, text, width, maximum_lines);
        self.lines.len() as f32 * wrapped_line_height(style)
    }

    /// Draws `text` in `style` wrapped over lines no wider than `rect`, from
    /// the top of `rect` down, each line placed by `align`. A line breaks
    /// between words, between CJK characters and at line breaks. The text
    /// ends after `maximum_lines` lines, and a text with more lines ends the
    /// last line with an ellipsis. The lines ignore the height of `rect`.
    /// Returns the height of the lines.
    pub fn show(
        &mut self,
        context: &mut UiContext,
        rect: Rect,
        style: &TextStyle,
        text: &str,
        align: Align,
        maximum_lines: usize,
    ) -> f32 {
        self.wrap(context, style, text, rect.width, maximum_lines);
        let Some(run) = &self.run else {
            return 0.0;
        };
        let line_height = wrapped_line_height(style);
        let last = self.lines.len().saturating_sub(1);
        for (index, line) in self.lines.iter().enumerate() {
            let line_area = Rect::new(
                rect.x,
                rect.y + index as f32 * line_height,
                rect.width,
                line_height,
            );
            let x = match align {
                Align::Start => line_area.x,
                Align::Center => line_area.x + (line_area.width - line.width) * 0.5,
                Align::End => line_area.right() - line.width,
            };
            let baseline = context.centered_baseline(line_area, style);
            let (line_run, glyphs) = match &self.cut {
                Some(cut) if index == last => (cut, 0..cut.glyphs.len()),
                Some(_) | None => (run, line.glyphs.clone()),
            };
            context.scene.run_glyphs(
                (context.snap(x), baseline),
                style.size,
                line_run,
                glyphs,
                style.color,
            );
        }
        self.lines.len() as f32 * line_height
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::tests::{Harness, pixel};

    /// The left edge of the first and the right edge of the last glyph
    /// instance of the last pass.
    fn extent(harness: &Harness) -> (f32, f32) {
        let instances = harness.scene.instances();
        let first = instances.first().unwrap().rect;
        let last = instances.last().unwrap().rect;
        (first[0], last[0] + last[2])
    }

    #[test]
    fn a_short_label_fits_and_follows_its_alignment() {
        let mut harness = Harness::new();
        let rect = Rect::new(100.0, 10.0, 300.0, 30.0);
        let mut extents = Vec::new();
        for align in [Align::Start, Align::Center, Align::End] {
            let truncated = harness.frame(|context| {
                let style = context.theme.body();
                label(context, rect, &style, "Disk", align, Truncation::End)
            });
            assert!(!truncated);
            extents.push(extent(&harness));
        }
        assert!(extents[0].0 >= 100.0 && extents[0].0 < 102.0);
        let center = (extents[1].0 + extents[1].1) * 0.5;
        assert!((center - 250.0).abs() < 2.0, "{center}");
        assert!(extents[2].1 <= 400.0 && extents[2].1 > 397.0);
    }

    #[test]
    fn a_long_label_is_cut_at_the_end_or_in_the_middle() {
        let mut harness = Harness::new();
        let rect = Rect::new(0.0, 0.0, 160.0, 30.0);
        let name = "A very long disk image name for testing (Disk 2).d88";
        for truncation in [Truncation::End, Truncation::Middle] {
            let truncated = harness.frame(|context| {
                let style = context.theme.body();
                label(context, rect, &style, name, Align::Start, truncation)
            });
            assert!(truncated);
            assert!(extent(&harness).1 <= 160.0);
        }
        let middle = harness.frame(|context| {
            let style = context.theme.body();
            context
                .fonts
                .fit_middle(style.font, style.size, name, 160.0)
        });
        let last = middle.glyphs.last().unwrap();
        assert_eq!(&name[last.cluster..], "8");
    }

    #[test]
    fn a_wrapped_label_breaks_into_lines_inside_its_width() {
        let mut harness = Harness::new();
        let rect = Rect::new(20.0, 30.0, 250.0, 0.0);
        let text = "The changes to YU-NO: A Girl Who Chants Love at the Bound of this World are not saved.";
        let (height, measured) = harness.frame(|context| {
            let style = context.theme.body();
            let measured = WrappedText::new().height(context, &style, text, rect.width, usize::MAX);
            let height =
                WrappedText::new().show(context, rect, &style, text, Align::Start, usize::MAX);
            (height, measured)
        });
        let line_height = harness.theme.text_size * WRAP_LINE_HEIGHT;
        assert_eq!(height, measured);
        assert!(height >= line_height * 3.0, "{height}");
        for instance in harness.scene.instances() {
            let [x, y, width, glyph_height] = instance.rect;
            assert!(x >= 20.0 && x + width <= 271.0, "{instance:?}");
            assert!(
                y >= 30.0 && y + glyph_height <= 30.0 + height + 4.0,
                "{instance:?}"
            );
        }
        let two = harness.frame(|context| {
            let style = context.theme.small();
            WrappedText::new().show(context, rect, &style, text, Align::Center, 2)
        });
        assert_eq!(two, harness.theme.small_text_size * WRAP_LINE_HEIGHT * 2.0);
    }

    #[test]
    fn a_wrapped_japanese_label_breaks_between_characters() {
        let mut harness = Harness::new();
        let text = "この世の果てで恋を唄う少女YU-NOの変更は保存されていません。";
        let height = harness.frame(|context| {
            let style = context.theme.body();
            WrappedText::new().show(
                context,
                Rect::new(0.0, 0.0, 120.0, 0.0),
                &style,
                text,
                Align::Start,
                usize::MAX,
            )
        });
        assert!(height >= harness.theme.text_size * WRAP_LINE_HEIGHT * 3.0);
        assert!(extent(&harness).1 <= 121.0);
    }

    /// The rows of pixels of the last pass that hold any coverage.
    fn covered_rows(harness: &Harness) -> Vec<u32> {
        let pixels = harness.render();
        let width = harness.viewport.width;
        (0..harness.viewport.height)
            .filter(|&y| (0..width).any(|x| pixel(&pixels, width, x as f32, y as f32)[3] > 0))
            .collect()
    }

    #[test]
    fn a_label_draws_inside_its_rectangle() {
        let mut harness = Harness::new();
        let rect = Rect::new(20.0, 40.0, 200.0, 30.0);
        harness.frame(|context| {
            let style = context.theme.body();
            label(
                context,
                rect,
                &style,
                "Brandish ブランディッシュ",
                Align::Start,
                Truncation::End,
            );
        });
        let rows = covered_rows(&harness);
        assert!(!rows.is_empty());
        assert!(
            *rows.first().unwrap() >= 40 && *rows.last().unwrap() < 70,
            "{rows:?}"
        );
    }

    #[test]
    fn a_wrapped_label_draws_one_band_of_pixels_for_each_line() {
        let mut harness = Harness::new();
        let text = "The changes to YU-NO: A Girl Who Chants Love at the Bound of this World are not saved.";
        let height = harness.frame(|context| {
            let style = context.theme.body();
            WrappedText::new().show(
                context,
                Rect::new(20.0, 40.0, 250.0, 0.0),
                &style,
                text,
                Align::Start,
                usize::MAX,
            )
        });
        let rows = covered_rows(&harness);
        let bands = rows.windows(2).filter(|pair| pair[1] > pair[0] + 1).count() + 1;
        let lines = (height / (harness.theme.text_size * WRAP_LINE_HEIGHT)).round() as usize;
        assert_eq!(bands, lines);
        assert!(*rows.last().unwrap() as f32 <= 40.0 + height + 1.0);
    }

    /// Verifies that a text with more lines than the most lines ends its last line in an
    /// ellipsis inside the width, and that a text that fits its lines draws no ellipsis.
    #[test]
    fn a_cut_wrapped_text_ends_in_an_ellipsis() {
        let mut harness = Harness::new();
        let text = "The changes to YU-NO: A Girl Who Chants Love at the Bound of this World are not saved.";
        let rect = Rect::new(20.0, 40.0, 250.0, 0.0);
        let draw = |harness: &mut Harness, maximum_lines: usize| {
            harness.frame(|context| {
                let style = context.theme.body();
                let ellipsis = context.fonts.shape(style.font, "\u{2026}").glyphs[0].shape;
                let height = WrappedText::new().show(
                    context,
                    rect,
                    &style,
                    text,
                    Align::Start,
                    maximum_lines,
                );
                (height, ellipsis.expect("the font maps the ellipsis").index)
            })
        };
        let last_glyph = |harness: &Harness| *harness.scene.instances().last().unwrap();
        let line_height = harness.theme.text_size * WRAP_LINE_HEIGHT;
        let (height, ellipsis) = draw(&mut harness, 2);
        assert_eq!(height, line_height * 2.0);
        let last = last_glyph(&harness);
        assert_eq!(last.kind, [crate::Instance::KIND_GLYPH, ellipsis, 0, 0]);
        assert!(last.rect[0] + last.rect[2] <= 271.0, "{last:?}");
        assert!(last.rect[1] > 40.0 + line_height, "{last:?}");
        let (full, _) = draw(&mut harness, usize::MAX);
        assert!(full > line_height * 2.0, "{full}");
        assert_ne!(last_glyph(&harness).kind[1], ellipsis);
    }

    /// Verifies that a kept wrap draws the lines of a fresh wrap, wraps again for another
    /// width or text and neither wraps nor allocates while nothing changes.
    #[test]
    fn a_wrapped_text_keeps_its_lines_until_the_text_or_the_width_changes() {
        let mut harness = Harness::new();
        let text = "The changes to YU-NO: A Girl Who Chants Love at the Bound of this World are not saved.";
        let rect = Rect::new(20.0, 40.0, 250.0, 0.0);
        let mut wrapped = WrappedText::new();
        let draw = |harness: &mut Harness, wrapped: &mut WrappedText, rect: Rect, text: &str| {
            harness.frame(|context| {
                let style = context.theme.body();
                wrapped.show(context, rect, &style, text, Align::Start, usize::MAX)
            })
        };
        let fresh = harness.frame(|context| {
            let style = context.theme.body();
            WrappedText::new().show(context, rect, &style, text, Align::Start, usize::MAX)
        });
        let fresh_rows = covered_rows(&harness);
        assert_eq!(draw(&mut harness, &mut wrapped, rect, text), fresh);
        let (allocations, (wraps, kept)) = allocation_counter::count_allocations(|| {
            count_wraps(|| draw(&mut harness, &mut wrapped, rect, text))
        });
        assert_eq!(allocations, 0);
        assert_eq!(wraps, 0);
        assert_eq!(kept, fresh);
        assert_eq!(covered_rows(&harness), fresh_rows);
        let wide = Rect::new(20.0, 40.0, 600.0, 0.0);
        let (wraps, wide_height) = count_wraps(|| draw(&mut harness, &mut wrapped, wide, text));
        assert_eq!(wraps, 1);
        assert!(wide_height < fresh);
        let (wraps, short) = count_wraps(|| draw(&mut harness, &mut wrapped, rect, "Saved."));
        assert_eq!(wraps, 1);
        assert_eq!(short, harness.theme.text_size * WRAP_LINE_HEIGHT);
    }
}
