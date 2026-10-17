//! The single line text field.

use super::{
    context::{UiContext, UiId},
    icons::Icon,
};
use crate::{Rect, RectStyle};

/// The width of the text cursor.
const CURSOR_WIDTH: f32 = 1.5;

/// The size of the icon of the clear button.
const CLEAR_ICON_SIZE: f32 = 12.0;

/// The most characters a [`text_field`] holds.
pub const TEXT_FIELD_LIMIT: usize = 256;

/// What a [`text_field`] did in a pass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextFieldResponse {
    /// Whether the text changed.
    pub changed: bool,
    /// Whether confirm was pressed while the field held the focus.
    pub submitted: bool,
}

/// A single line text field in `rect` that edits `text`. An empty field
/// shows `placeholder`.
///
/// A click gives the field the focus. While it holds the focus, the
/// committed text of the input goes to the end of the text up to
/// [`TEXT_FIELD_LIMIT`] characters, and backspace removes the last
/// character. The composition of an input method shows underlined at the
/// cursor. The cursor stays at the end, and the field takes left, right,
/// home and end without a change. Cancel empties a field with text and
/// leaves an empty field to the panel. A text wider than the field scrolls,
/// so its end and the cursor stay visible. A clear button for the mouse
/// empties a field with text.
pub fn text_field(
    context: &mut UiContext,
    id: impl Into<UiId>,
    rect: Rect,
    text: &mut String,
    placeholder: &str,
) -> TextFieldResponse {
    let id = id.into();
    let theme = context.theme;
    let mut response = TextFieldResponse::default();
    if context.cancel_taken(id) && !text.is_empty() {
        text.clear();
        response.changed = true;
    }
    let clear_id = id.child("clear");
    let clear_rect =
        Rect::new(rect.right() - rect.height, rect.y, rect.height, rect.height).inset(4.0);
    let mut clear_hot = false;
    if !text.is_empty() {
        let pointer = context.button_behavior(clear_id, clear_rect);
        clear_hot = pointer.hovered || pointer.held;
        if pointer.clicked {
            text.clear();
            response.changed = true;
            context.set_focus(id);
        }
    }
    if context.has_focus(id) {
        let input = context.input;
        let room = TEXT_FIELD_LIMIT.saturating_sub(text.chars().count());
        let before = text.len();
        text.extend(
            input
                .text
                .chars()
                .filter(|character| !character.is_control())
                .take(room),
        );
        response.changed |= text.len() != before;
        if input.backspace {
            response.changed |= text.pop().is_some();
        }
        response.submitted = context.confirmed(id);
    }
    context.text_focusable(id, rect, !text.is_empty());
    let pointer = context.button_behavior(id, rect);
    let focused = context.has_focus(id);
    let composition = if focused {
        context.input.composition.as_str()
    } else {
        ""
    };

    let edge = if focused {
        theme.accent
    } else if pointer.hovered {
        theme.control_edge.mix(theme.text_dim, 0.35)
    } else {
        theme.control_edge
    };
    context.rect(
        rect,
        &RectStyle::filled(theme.field, theme.radius).with_stroke(theme.stroke, edge),
    );
    let icon_slot = Rect::new(rect.x + theme.gap, rect.y, theme.icon_size, rect.height);
    let icon_color = if focused {
        theme.text_dim
    } else {
        theme.text_disabled
    };
    context.icon(icon_slot, Icon::Search, theme.icon_size, icon_color);
    let left = icon_slot.right() + theme.gap * 0.75;
    let right = if text.is_empty() {
        rect.right() - theme.gap
    } else {
        clear_rect.x - 2.0
    };
    let inner = Rect::new(left, rect.y, (right - left).max(0.0), rect.height);

    let style = theme.body();
    let baseline = context.centered_baseline(rect, &style);
    if text.is_empty() && composition.is_empty() {
        context.text_left(inner, &style.with_color(theme.text_disabled), placeholder);
    }
    let cursor_width = context.snap(CURSOR_WIDTH).max(1.0 / context.scale());
    context.scene.push_clip(Rect::new(
        inner.x - cursor_width,
        rect.y + theme.stroke,
        inner.width + cursor_width * 2.0,
        rect.height - theme.stroke * 2.0,
    ));
    let width = context.measure(&style, text);
    let composition_width = context.measure(&style, composition);
    let scroll = (width + composition_width + cursor_width - inner.width).max(0.0);
    let x = context.snap(inner.x - scroll);
    if !text.is_empty() {
        context.text(x, baseline, &style, text);
    }
    if !composition.is_empty() {
        let start = context.snap(x + width);
        context.text(start, baseline, &style, composition);
        let underline = (1.0 / context.scale()).max(context.snap(1.0));
        context.scene.fill(
            Rect::new(start, baseline + 2.0, composition_width, underline),
            0.0,
            theme.text,
        );
    }
    if focused {
        let height = (style.size * 1.3).round();
        let cursor_x = context.snap(x + width + composition_width);
        context.set_text_caret(cursor_x);
        context.scene.fill(
            Rect::new(
                cursor_x,
                context.snap(rect.y + (rect.height - height) * 0.5),
                cursor_width,
                height,
            ),
            0.0,
            theme.text,
        );
    }
    context.scene.pop_clip();

    if !text.is_empty() {
        if clear_hot {
            context
                .scene
                .fill(clear_rect, clear_rect.height * 0.5, theme.control_hover);
        }
        let color = if clear_hot {
            theme.text
        } else {
            theme.text_dim
        };
        context.icon(clear_rect, Icon::Close, CLEAR_ICON_SIZE, color);
    }
    if focused {
        context.focus_ring(rect, theme.radius);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Instance, ui::tests::Harness};

    /// The area of the field.
    const FIELD: Rect = Rect::new(10.0, 10.0, 200.0, 32.0);

    /// Draws the field over `text` and returns its response.
    fn draw(harness: &mut Harness, text: &mut String) -> TextFieldResponse {
        harness.frame(|context| text_field(context, "filter", FIELD, text, "Filter"))
    }

    /// The rectangle of the text cursor of the last pass, a fill of the
    /// cursor width in physical pixels.
    fn cursor(harness: &Harness) -> Option<[f32; 4]> {
        let width = (CURSOR_WIDTH * harness.viewport.scale).round().max(1.0);
        let height = (harness.theme.text_size * 1.3).round() * harness.viewport.scale;
        harness
            .scene
            .instances()
            .iter()
            .find(|instance| {
                instance.kind[0] == Instance::KIND_RECTANGLE
                    && instance.rect[2] == width
                    && instance.rect[3] == height.round()
            })
            .map(|instance| instance.rect)
    }

    #[test]
    fn typed_text_reaches_only_the_focused_field() {
        let mut harness = Harness::new();
        let mut text = String::new();
        harness.input.text.push_str("ignored");
        assert!(!draw(&mut harness, &mut text).changed);
        assert!(text.is_empty());
        assert!(cursor(&harness).is_none());

        harness.press(50.0, 20.0);
        draw(&mut harness, &mut text);
        harness.release();
        assert!(harness.state.wants_text_input());
        harness.input.text.push_str("ディスク\tA");
        let response = draw(&mut harness, &mut text);
        assert!(response.changed);
        assert_eq!(text, "ディスクA");
        assert!(cursor(&harness).is_some());
    }

    #[test]
    fn backspace_removes_whole_characters() {
        let mut harness = Harness::new();
        let mut text = String::from("aé漢");
        harness.state.set_focus(UiId::from("filter"));
        harness.input.backspace = true;
        draw(&mut harness, &mut text);
        assert_eq!(text, "aé");
        for _ in 0..2 {
            harness.input.backspace = true;
            assert!(draw(&mut harness, &mut text).changed);
        }
        assert_eq!(text, "");
        harness.input.backspace = true;
        assert!(!draw(&mut harness, &mut text).changed);
    }

    #[test]
    fn confirm_submits_the_focused_field() {
        let mut harness = Harness::new();
        let mut text = String::from("adv");
        harness.state.set_focus(UiId::from("filter"));
        harness.input.confirm = true;
        assert!(draw(&mut harness, &mut text).submitted);
        harness.state.clear_focus();
        harness.input.confirm = true;
        assert!(!draw(&mut harness, &mut text).submitted);
    }

    #[test]
    fn a_long_text_scrolls_and_keeps_the_cursor_inside() {
        let mut harness = Harness::new();
        let mut text = String::new();
        harness.state.set_focus(UiId::from("filter"));
        draw(&mut harness, &mut text);
        let empty = cursor(&harness).unwrap();
        harness.input.text = "The quick brown fox jumps over the lazy dog".into();
        draw(&mut harness, &mut text);
        let long = cursor(&harness).unwrap();
        assert!(long[0] > empty[0]);
        assert!(long[0] + long[2] <= FIELD.right() - 20.0, "{long:?}");
        let glyphs = harness
            .scene
            .instances()
            .iter()
            .filter(|instance| instance.kind[0] == Instance::KIND_GLYPH)
            .count();
        assert!(glyphs < text.chars().filter(|character| *character != ' ').count());
    }

    #[test]
    fn the_clear_button_empties_the_field() {
        let mut harness = Harness::new();
        let mut text = String::from("adv");
        draw(&mut harness, &mut text);
        let (x, y) = Rect::new(
            FIELD.right() - FIELD.height,
            FIELD.y,
            FIELD.height,
            FIELD.height,
        )
        .center();
        harness.press(x, y);
        draw(&mut harness, &mut text);
        harness.release();
        assert!(draw(&mut harness, &mut text).changed);
        assert!(text.is_empty());
        assert_eq!(harness.state.focused(), Some(UiId::from("filter")));
    }

    #[test]
    fn cancel_empties_the_field_before_it_reaches_the_panel() {
        let mut harness = Harness::new();
        let mut text = String::from("adv");
        let draw_field = |harness: &mut Harness, text: &mut String| {
            harness.frame(|context| {
                let response = text_field(context, "filter", FIELD, text, "Filter");
                (response.changed, context.cancelled())
            })
        };
        harness.state.set_focus(UiId::from("filter"));
        draw_field(&mut harness, &mut text);
        harness.input.cancel = true;
        assert_eq!(draw_field(&mut harness, &mut text), (true, false));
        assert!(text.is_empty());
        harness.input.cancel = true;
        assert_eq!(draw_field(&mut harness, &mut text), (false, true));
    }

    #[test]
    fn the_field_keeps_the_focus_on_left_right_home_and_end() {
        let mut harness = Harness::new();
        let mut text = String::from("adv");
        let draw_both = |harness: &mut Harness, text: &mut String| {
            harness.frame(|context| {
                text_field(context, "filter", FIELD, text, "Filter");
                for (index, x) in [-200.0, 300.0].into_iter().enumerate() {
                    context.focusable(
                        UiId::from("neighbor").indexed(index),
                        Rect::new(x, FIELD.y, 100.0, FIELD.height),
                        crate::ui::NavigationSet::EMPTY,
                    );
                }
            });
        };
        harness.state.set_focus(UiId::from("filter"));
        draw_both(&mut harness, &mut text);
        for navigation in [
            crate::ui::Navigation::Left,
            crate::ui::Navigation::Right,
            crate::ui::Navigation::Home,
            crate::ui::Navigation::End,
        ] {
            harness.navigate(navigation);
            draw_both(&mut harness, &mut text);
            assert_eq!(harness.state.focused(), Some(UiId::from("filter")));
        }
        assert_eq!(text, "adv");
    }

    #[test]
    fn the_text_stops_at_the_limit() {
        let mut harness = Harness::new();
        let mut text = "a".repeat(TEXT_FIELD_LIMIT - 2);
        harness.state.set_focus(UiId::from("filter"));
        harness.input.text.push_str("漢字かな");
        assert!(draw(&mut harness, &mut text).changed);
        assert_eq!(text.chars().count(), TEXT_FIELD_LIMIT);
        assert!(text.ends_with("漢字"));
        harness.input.text.push('b');
        assert!(!draw(&mut harness, &mut text).changed);
    }

    #[test]
    fn the_composition_shows_at_the_cursor() {
        let mut harness = Harness::new();
        let mut text = String::from("ab");
        harness.state.set_focus(UiId::from("filter"));
        draw(&mut harness, &mut text);
        let plain = cursor(&harness).unwrap();
        harness.input.composition.push_str("かな");
        draw(&mut harness, &mut text);
        let composing = cursor(&harness).unwrap();
        assert!(composing[0] > plain[0] + 20.0);
        assert_eq!(text, "ab");
        harness.input.composition.clear();
        harness.input.text.push_str("かな");
        draw(&mut harness, &mut text);
        assert_eq!(text, "abかな");
        assert_eq!(cursor(&harness).unwrap()[0], composing[0]);
    }
}
