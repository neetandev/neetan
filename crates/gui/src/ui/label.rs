//! The label widget.

use super::context::{TextStyle, UiContext};
use crate::Rect;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::tests::Harness;

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
}
