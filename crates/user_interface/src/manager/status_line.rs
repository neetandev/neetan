//! The status line of the manager: the keys and gamepad buttons of the main
//! actions and a text at its end.

use gui::{
    Color, Rect, RectStyle,
    ui::{Align, Truncation, UiContext},
};

/// The height of the status line in logical units.
pub(super) const STATUS_LINE_HEIGHT: f32 = 28.0;

/// The height of a key cap in logical units.
const KEYCAP_HEIGHT: f32 = 18.0;

/// A key of the keyboard or a button of a gamepad in a hint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HintKey {
    /// A key cap with the text of a key, for example `Enter`.
    Key(&'static str),
    /// A round gamepad face button, for example `A`.
    Pad(&'static str),
}

/// The keys of an action and the label of the action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Hint {
    /// The keys and buttons.
    pub(super) keys: &'static [HintKey],
    /// The label of the action.
    pub(super) label: &'static str,
}

/// Draws the status line into `rect`: the `hints` from the start and `end`
/// in the color `end_color` at the end.
pub(super) fn status_line(
    context: &mut UiContext,
    rect: Rect,
    hints: &[Hint],
    end: &str,
    end_color: Color,
) {
    let theme = context.theme;
    context.fill(rect, theme.well);
    context.fill(
        Rect::new(rect.x, rect.y, rect.width, theme.stroke),
        theme.panel_edge,
    );
    let style = theme.small();
    let inner = Rect::new(
        rect.x + theme.padding,
        rect.y,
        (rect.width - theme.padding * 2.0).max(0.0),
        rect.height,
    );
    let mut x = inner.x;
    for hint in hints {
        for key in hint.keys {
            x = keycap(context, x, rect, *key) + theme.gap * 0.5;
        }
        x += theme.gap * 0.25;
        let width = context.measure(&style, hint.label);
        context.text_left(
            Rect::new(x, rect.y, width + 1.0, rect.height),
            &style,
            hint.label,
        );
        x += width + theme.gap * 2.0;
    }
    if end.is_empty() {
        return;
    }
    let room = (inner.right() - x).max(0.0);
    let end_width = context.measure(&style, end).min(room);
    let end_area = Rect::new(inner.right() - end_width, rect.y, end_width, rect.height);
    label_with_color(context, end_area, end, end_color);
}

/// Draws the small text `text` in `color` at the end of `rect`, cut in the middle.
fn label_with_color(context: &mut UiContext, rect: Rect, text: &str, color: Color) {
    let style = context.theme.small().with_color(color);
    context.text_in(rect, &style, text, Align::End, Truncation::Middle);
}

/// Draws the key cap of `key` at `x`, centered in the height of `line`.
/// Returns the right edge of the cap.
fn keycap(context: &mut UiContext, x: f32, line: Rect, key: HintKey) -> f32 {
    let theme = context.theme;
    let style = theme.small().with_color(theme.text);
    let (text, width, radius) = match key {
        HintKey::Key(text) => {
            let width = (context.measure(&style, text) + theme.gap * 1.25).max(KEYCAP_HEIGHT);
            (text, width, theme.radius * 2.0 / 3.0)
        }
        HintKey::Pad(text) => (text, KEYCAP_HEIGHT, KEYCAP_HEIGHT * 0.5),
    };
    let top = context.snap(line.y + (line.height - KEYCAP_HEIGHT) * 0.5);
    let cap = context.snap_rect(Rect::new(x, top, width, KEYCAP_HEIGHT));
    context.rect(
        cap,
        &RectStyle::filled(theme.control, radius).with_stroke(theme.stroke, theme.control_edge),
    );
    context.text_centered(cap, &style, text);
    cap.right()
}
