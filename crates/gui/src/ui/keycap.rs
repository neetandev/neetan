//! The key cap: a key of the keyboard or a button of a gamepad drawn as a
//! small rounded cap with its text.

use i18n::Key;

use super::{context::UiContext, icons::Icon};
use crate::{Rect, RectStyle};

/// The height of a key cap.
pub const KEYCAP_HEIGHT: f32 = 18.0;

/// The padding of a key cap on both sides together in units of the gap.
const KEYCAP_PADDING_GAPS: f32 = 1.25;

/// The size of each arrow icon in the cap of [`HintKey::DpadLeftRight`].
const DPAD_ICON_SIZE: f32 = 12.0;

/// The width of the cap of [`HintKey::DpadLeftRight`]. Each arrow sits at
/// the center of a round end of the cap, and the centers lie one icon
/// apart. The two arrows keep a gap between them.
const DPAD_CAP_WIDTH: f32 = KEYCAP_HEIGHT + DPAD_ICON_SIZE;

/// A key of the keyboard or a button of a gamepad on a key cap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HintKey<'a> {
    /// A key cap of the keyboard with the catalog text of `Key`, for example `Enter`.
    Key(Key),
    /// A key cap of the keyboard with a fixed text, for example `M`.
    Text(&'a str),
    /// A round gamepad face button, for example `A`.
    Pad(&'a str),
    /// A gamepad shoulder button, for example `LB`.
    Shoulder(&'a str),
    /// A gamepad menu button with a key cap, for example `Start`.
    Menu(&'a str),
    /// The left and right of the gamepad D-pad: a round cap with a left
    /// and a right arrow.
    DpadLeftRight,
}

/// The width of the key cap of `key`: its text, the padding on both sides
/// and the border on both sides, at least the height of the cap.
pub fn keycap_width(context: &mut UiContext, key: HintKey) -> f32 {
    let theme = context.theme;
    let style = theme.small().with_color(theme.colors.text);
    let text = match key {
        HintKey::Pad(_) => return KEYCAP_HEIGHT,
        HintKey::DpadLeftRight => return DPAD_CAP_WIDTH,
        HintKey::Key(key) => context.catalog.text(key),
        HintKey::Text(text) | HintKey::Shoulder(text) | HintKey::Menu(text) => text,
    };
    (context.measure(&style, text) + theme.gap * KEYCAP_PADDING_GAPS + theme.stroke * 2.0)
        .max(KEYCAP_HEIGHT)
}

/// Draws the key cap of `key` at `x`, centered in the height of `line`.
/// Returns the right edge of the cap.
pub fn keycap(context: &mut UiContext, x: f32, line: Rect, key: HintKey) -> f32 {
    let theme = context.theme;
    let style = theme.small().with_color(theme.colors.text);
    let small_radius = theme.radius * 2.0 / 3.0;
    let half = KEYCAP_HEIGHT * 0.5;
    let width = keycap_width(context, key);
    let (text, radii) = match key {
        HintKey::Key(key) => (Some(context.catalog.text(key)), [small_radius; 4]),
        HintKey::Text(text) | HintKey::Menu(text) => (Some(text), [small_radius; 4]),
        HintKey::Pad(text) => (Some(text), [half; 4]),
        HintKey::Shoulder(text) => (Some(text), [half, half, small_radius, small_radius]),
        HintKey::DpadLeftRight => (None, [half; 4]),
    };
    let top = context.snap(line.y + (line.height - KEYCAP_HEIGHT) * 0.5);
    let cap = context.snap_rect(Rect::new(x, top, width, KEYCAP_HEIGHT));
    context.rect(
        cap,
        &RectStyle {
            radii,
            ..RectStyle::filled(theme.colors.control, 0.0)
                .with_stroke(theme.stroke, theme.colors.control_edge)
        },
    );
    if let Some(text) = text {
        context.text_centered(cap, &style, text);
    } else {
        let (_, middle) = cap.center();
        for (center, icon) in [
            (cap.x + half, Icon::Left),
            (cap.right() - half, Icon::Right),
        ] {
            let slot = Rect::new(
                center - DPAD_ICON_SIZE * 0.5,
                middle - DPAD_ICON_SIZE * 0.5,
                DPAD_ICON_SIZE,
                DPAD_ICON_SIZE,
            );
            context.icon(slot, icon, DPAD_ICON_SIZE, theme.colors.text);
        }
    }
    cap.right()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Instance,
        ui::tests::{Harness, test_catalog, test_key},
    };

    /// The line that the test caps center in.
    const LINE: Rect = Rect::new(0.0, 100.0, 400.0, 28.0);

    /// Verifies that a key cap holds its text, the padding of `gap * 0.625` and the border on
    /// each side, for a catalog key and a fixed text, and that a short key keeps the width of
    /// the height.
    #[test]
    fn a_key_cap_holds_its_text_its_padding_and_its_border() {
        let mut harness = Harness::new();
        let (catalog_text, fixed_text, catalog_cap, fixed_cap, shoulder, letter) =
            harness.frame(|context| {
                let style = context.theme.small().with_color(context.theme.colors.text);
                (
                    context.measure(&style, test_catalog().text(test_key("ctrl-page-up"))),
                    context.measure(&style, "Right Ctrl"),
                    keycap_width(context, HintKey::Key(test_key("ctrl-page-up"))),
                    keycap_width(context, HintKey::Text("Right Ctrl")),
                    keycap_width(context, HintKey::Shoulder("I")),
                    keycap_width(context, HintKey::Text("I")),
                )
            });
        let theme = &harness.theme;
        let border = theme.gap * 0.625 * 2.0 + theme.stroke * 2.0;
        for (width, text) in [(catalog_cap, catalog_text), (fixed_cap, fixed_text)] {
            assert!((width - text - border).abs() < 0.01, "{width} {text}");
        }
        assert_eq!(shoulder, KEYCAP_HEIGHT);
        assert_eq!(letter, KEYCAP_HEIGHT);
    }

    /// Verifies that the D-pad cap is a round cap of its width with a left and a right arrow
    /// inside it, a gap between the arrows and no text.
    #[test]
    fn the_dpad_cap_draws_a_left_and_a_right_arrow() {
        let mut harness = Harness::new();
        let right = harness.frame(|context| keycap(context, 16.0, LINE, HintKey::DpadLeftRight));
        let instances = harness.scene.instances();
        let caps: Vec<_> = instances
            .iter()
            .filter(|instance| {
                instance.kind[0] == Instance::KIND_RECTANGLE && instance.rect[3] == KEYCAP_HEIGHT
            })
            .collect();
        assert_eq!(caps.len(), 1);
        let cap = caps[0].rect;
        assert_eq!(cap[2], DPAD_CAP_WIDTH);
        assert_eq!(right, cap[0] + cap[2]);
        assert_eq!(caps[0].radii, [KEYCAP_HEIGHT * 0.5; 4]);
        let lines: Vec<_> = instances
            .iter()
            .filter(|instance| instance.kind[0] == Instance::KIND_LINE)
            .map(|instance| instance.rect)
            .collect();
        assert_eq!(lines.len(), 4);
        for line in &lines {
            assert!(
                line[0] >= cap[0] && line[0] + line[2] <= cap[0] + cap[2],
                "{line:?}"
            );
        }
        let left_end = lines[..2]
            .iter()
            .map(|line| line[0] + line[2])
            .fold(f32::MIN, f32::max);
        let right_start = lines[2..]
            .iter()
            .map(|line| line[0])
            .fold(f32::MAX, f32::min);
        assert!(right_start - left_end >= 2.0, "{lines:?}");
        assert!(
            instances
                .iter()
                .all(|instance| instance.kind[0] != Instance::KIND_GLYPH)
        );
    }
}
