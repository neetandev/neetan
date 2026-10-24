//! The label widget.

use super::context::{TextStyle, UiContext};
use crate::{Line, Rect};

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

/// The lines of `text` in `style` no wider than `width`, at most
/// `maximum_lines` of them, and the shaped text that they index.
fn wrap(
    context: &mut UiContext,
    style: &TextStyle,
    text: &str,
    width: f32,
    maximum_lines: usize,
) -> (std::sync::Arc<crate::Run>, Vec<Line>) {
    let mut lines = Vec::new();
    let run = context
        .fonts
        .wrap(style.font, style.size, text, width.max(0.0), &mut lines);
    lines.truncate(maximum_lines.max(1));
    (run, lines)
}

/// The height that [`wrapped_label`] takes for `text` in `style` at the
/// width `width` with at most `maximum_lines` lines.
pub fn wrapped_height(
    context: &mut UiContext,
    style: &TextStyle,
    text: &str,
    width: f32,
    maximum_lines: usize,
) -> f32 {
    let (_, lines) = wrap(context, style, text, width, maximum_lines);
    lines.len() as f32 * wrapped_line_height(style)
}

/// Draws `text` in `style` wrapped over lines no wider than `rect`, from
/// the top of `rect` down, each line placed by `align`. A line breaks
/// between words, between CJK characters and at line breaks. The text ends
/// after `maximum_lines` lines. The lines ignore the height of `rect`.
/// Returns the height of the lines.
pub fn wrapped_label(
    context: &mut UiContext,
    rect: Rect,
    style: &TextStyle,
    text: &str,
    align: Align,
    maximum_lines: usize,
) -> f32 {
    let (run, lines) = wrap(context, style, text, rect.width, maximum_lines);
    let line_height = wrapped_line_height(style);
    for (index, line) in lines.iter().enumerate() {
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
        context.scene.run_glyphs(
            (context.snap(x), baseline),
            style.size,
            &run,
            line.glyphs.clone(),
            style.color,
        );
    }
    lines.len() as f32 * line_height
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
            let measured = wrapped_height(context, &style, text, rect.width, usize::MAX);
            let height = wrapped_label(context, rect, &style, text, Align::Start, usize::MAX);
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
            wrapped_label(context, rect, &style, text, Align::Center, 2)
        });
        assert_eq!(two, harness.theme.small_text_size * WRAP_LINE_HEIGHT * 2.0);
    }

    #[test]
    fn a_wrapped_japanese_label_breaks_between_characters() {
        let mut harness = Harness::new();
        let text = "この世の果てで恋を唄う少女YU-NOの変更は保存されていません。";
        let height = harness.frame(|context| {
            let style = context.theme.body();
            wrapped_label(
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
            wrapped_label(
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
}
