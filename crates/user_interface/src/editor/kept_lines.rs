//! The lines of a game file that the editor keeps as written: the comments,
//! the key bindings, the unknown keys and the keys that the family does not
//! read, and their block at the end of the Advanced section.

use gui::{
    Rect,
    ui::{Align, Truncation, UiContext, label},
};

use super::fields::Field;
use crate::{
    config::{ConfigDocument, LineContent, parse_line},
    texts::editor,
};

/// The line height of the kept lines, in units of the small text size.
const KEPT_LINE_SPACING: f32 = 1.8;

/// Returns the lines of `document` that no field of `fields` shows: the
/// comments, the key bindings, the unknown keys and the keys that the
/// family does not read, as their normalized text. Blank lines stay out.
pub(crate) fn kept_lines(document: &ConfigDocument, fields: &[Field]) -> Vec<String> {
    document
        .lines()
        .filter_map(|(_, text)| match parse_line(text) {
            LineContent::Blank => None,
            LineContent::Comment | LineContent::Text => Some(text.trim().to_owned()),
            LineContent::Entry { key, value } => {
                (!fields.iter().any(|field| field.key() == key)).then(|| format!("{key} = {value}"))
            }
        })
        .collect()
}

/// Draws a rule with `caption` from `y` in `content`: the caption in small
/// secondary text and a line from the caption to the right edge, a gap
/// below the content before it. Returns the bottom of the rule.
pub(crate) fn draw_rule(context: &mut UiContext, content: Rect, y: f32, caption: &str) -> f32 {
    let theme = context.theme;
    let rule = Rect::new(
        content.x,
        y + theme.gap,
        content.width,
        theme.control_height,
    );
    let style = theme.small();
    let caption_width = context.measure(&style, caption);
    context.text_left(rule, &style, caption);
    let line_x = rule.x + caption_width + theme.gap;
    context.fill(
        Rect::new(
            line_x,
            context.snap(rule.y + rule.height * 0.5),
            (rule.right() - line_x).max(0.0),
            theme.stroke,
        ),
        theme.colors.panel_edge,
    );
    rule.bottom()
}

/// Draws the rule, the sentence and the kept `lines` from `y` in `content`.
/// Returns the height of the block.
pub(crate) fn draw_kept_lines(
    context: &mut UiContext,
    content: Rect,
    y: f32,
    lines: &[String],
) -> f32 {
    let theme = context.theme;
    let catalog = context.catalog;
    let style = theme.small();
    let line_height = theme.small_text_size * KEPT_LINE_SPACING;
    let mut top = draw_rule(context, content, y, catalog.text(editor::KEPT_CAPTION));
    label(
        context,
        Rect::new(content.x, top, content.width, line_height),
        &style,
        catalog.text(editor::KEPT_TEXT),
        Align::Start,
        Truncation::End,
    );
    top += line_height;
    let line_style = theme.small().with_color(theme.colors.text);
    for line in lines {
        label(
            context,
            Rect::new(content.x, top, content.width, line_height),
            &line_style,
            line,
            Align::Start,
            Truncation::End,
        );
        top += line_height;
    }
    top - y
}

#[cfg(test)]
mod tests {
    use machine_factory::config::settings::Families;

    use super::*;
    use crate::{
        editor::{form::form_fields, test_support::EditorRoot},
        gui_test_support::GuiHarness,
    };

    /// The height of the block of `count` kept lines in the theme of `context`.
    fn block_height(context: &UiContext, count: usize) -> f32 {
        let theme = context.theme;
        let line_height = theme.small_text_size * KEPT_LINE_SPACING;
        theme.gap + theme.control_height + line_height * (count + 1) as f32
    }

    /// Verifies the lines that no field shows: comments, key bindings, unknown keys and keys without a field, without blank lines.
    #[test]
    fn the_kept_lines_hold_what_no_field_shows() {
        let root = EditorRoot::new("kept_lines");
        let document = ConfigDocument::parse(
            "; comment\nmeta.title = Edge\nmeta.custom=x\n\nkey.F1=Stop\nfdd3 = a.d88\n\
             debug-bios = bios.rom\nbackend = legacy\ncrt = on\nfree text\n",
        );
        let fields: Vec<Field> = form_fields(root.target())
            .into_iter()
            .map(|(section, source)| {
                Field::read(
                    section,
                    source,
                    Families::of(root.target()),
                    &document,
                    false,
                )
            })
            .collect();
        assert_eq!(
            kept_lines(&document, &fields),
            [
                "; comment",
                "meta.custom = x",
                "key.F1 = Stop",
                "fdd3 = a.d88",
                "debug-bios = bios.rom",
                "backend = legacy",
                "free text"
            ]
        );
    }

    /// Verifies that the block holds the rule, the sentence and one line for each kept line.
    #[test]
    fn the_block_grows_with_its_lines() {
        let mut harness = GuiHarness::minimum();
        let content = Rect::new(16.0, 100.0, 900.0, 400.0);
        let lines = ["; comment".to_owned(), "key.F1 = Stop".to_owned()];
        let (empty, two, expected) = harness.frame(|context| {
            (
                draw_kept_lines(context, content, 100.0, &[]),
                draw_kept_lines(context, content, 100.0, &lines),
                block_height(context, lines.len()),
            )
        });
        assert!((two - expected).abs() < 0.01, "{two} {expected}");
        assert!(two > empty);
    }
}
