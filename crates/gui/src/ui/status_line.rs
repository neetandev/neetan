//! The status line at the bottom of a screen: the keys and gamepad buttons
//! of the actions of the top dialog, and a path or an error at its end.

use i18n::Key;
use stack_vec::StackVec;

use super::{
    context::UiContext,
    icons::Icon,
    label::{Align, Truncation},
};
use crate::{Rect, RectStyle};

/// The height of the status line.
pub const STATUS_LINE_HEIGHT: f32 = 28.0;

/// The height of a key cap in the status line.
pub const KEYCAP_HEIGHT: f32 = 18.0;

/// The padding of a key cap on both sides together in units of the gap.
const KEYCAP_PADDING_GAPS: f32 = 1.25;

/// The size of each arrow icon in the cap of [`HintKey::DpadLeftRight`].
const DPAD_ICON_SIZE: f32 = 12.0;

/// The width of the cap of [`HintKey::DpadLeftRight`]. Each arrow sits at
/// the center of a round end of the cap, and the centers lie one icon
/// apart. The two arrows keep a gap between them.
const DPAD_CAP_WIDTH: f32 = KEYCAP_HEIGHT + DPAD_ICON_SIZE;

/// A key of the keyboard or a button of a gamepad in a [`Hint`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HintKey<'a> {
    /// A key cap of the keyboard with the catalog text of `Key`, for example `Enter`.
    Key(Key),
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

/// The keys of an action and the label of the action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hint<'a> {
    /// The keys and buttons.
    pub keys: &'a [HintKey<'a>],
    /// The label of the action.
    pub label: &'a str,
}

/// Returns the hints of `hints` without each hint whose action an earlier
/// hint already has. Each hint comes with its action. Two hints of one
/// action lead to one result, such as Enter on a close button and Esc, and
/// the status line shows the first of them. `H` is any hint type. The
/// result holds at most `N` hints.
///
/// Panics when `hints` holds more than `N` distinct actions.
pub fn distinct_hints<A: PartialEq + Copy, H, const N: usize>(
    hints: impl IntoIterator<Item = (A, H)>,
) -> StackVec<H, N> {
    let mut actions = StackVec::<A, N>::new();
    let mut distinct = StackVec::new();
    for (action, hint) in hints {
        if !actions.contains(&action) {
            actions.push(action);
            distinct.push(hint);
        }
    }
    distinct
}

/// The text at the end of the status line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StatusEnd<'a> {
    /// No text.
    #[default]
    None,
    /// A path in the secondary color. A long path gives way in the middle.
    Path(&'a str),
    /// An error with the alert icon in the error color. A long error gives
    /// way at its end.
    Error(&'a str),
}

/// Draws the status line into `rect`: the `hints` from the start and `end`
/// in the room after them. A hint that does not fit is left out, together
/// with the hints after it. The end gives way to the hints and never draws
/// past the end of the line. An end that has less room than its full width
/// and less than `Theme::status_end_min_width` is left out.
pub fn status_line(context: &mut UiContext, rect: Rect, hints: &[Hint], end: StatusEnd) {
    let theme = context.theme;
    context.fill(rect, theme.colors.well);
    context.fill(
        Rect::new(rect.x, rect.y, rect.width, theme.stroke),
        theme.colors.panel_edge,
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
        let width = hint_width(context, hint);
        if x + width > inner.right() {
            break;
        }
        for key in hint.keys {
            x = keycap(context, x, rect, *key) + theme.gap * 0.5;
        }
        x += theme.gap * 0.25;
        let label_width = context.measure(&style, hint.label);
        context.text_left(
            Rect::new(x, rect.y, label_width + 1.0, rect.height),
            &style,
            hint.label,
        );
        x += label_width + theme.gap * 2.0;
    }
    let room = (inner.right() - x).max(0.0);
    let spacing = theme.icon_size + theme.gap * 0.75;
    let full_width = match end {
        StatusEnd::None => return,
        StatusEnd::Path(path) => context.measure(&style, path),
        StatusEnd::Error(error) => context.measure(&style, error) + spacing,
    };
    if room < full_width.min(theme.status_end_min_width) {
        return;
    }
    let width = full_width.min(room);
    match end {
        StatusEnd::None => {}
        StatusEnd::Path(path) => {
            let area = Rect::new(inner.right() - width, rect.y, width, rect.height);
            context.text_in(area, &style, path, Align::End, Truncation::Middle);
        }
        StatusEnd::Error(error) => {
            let style = style.with_color(theme.colors.danger_text);
            let (icon, text) = Rect::new(inner.right() - width, rect.y, width, rect.height)
                .cut_left(theme.icon_size);
            context.icon(icon, Icon::Alert, theme.icon_size, theme.colors.danger_text);
            let text = text.cut_left(theme.gap * 0.75).1;
            context.text_in(text, &style, error, Align::End, Truncation::End);
        }
    }
}

/// The width of `hint`: its key caps with their gaps and its label.
fn hint_width(context: &mut UiContext, hint: &Hint) -> f32 {
    let theme = context.theme;
    let style = theme.small();
    let keys: f32 = hint
        .keys
        .iter()
        .map(|key| keycap_width(context, *key) + theme.gap * 0.5)
        .sum();
    keys + theme.gap * 0.25 + context.measure(&style, hint.label)
}

/// The width of the key cap of `key`: its text, the padding on both sides
/// and the border on both sides, at least the height of the cap.
fn keycap_width(context: &mut UiContext, key: HintKey) -> f32 {
    let theme = context.theme;
    let style = theme.small().with_color(theme.colors.text);
    let text = match key {
        HintKey::Pad(_) => return KEYCAP_HEIGHT,
        HintKey::DpadLeftRight => return DPAD_CAP_WIDTH,
        HintKey::Key(key) => context.catalog.text(key),
        HintKey::Shoulder(text) | HintKey::Menu(text) => text,
    };
    (context.measure(&style, text) + theme.gap * KEYCAP_PADDING_GAPS + theme.stroke * 2.0)
        .max(KEYCAP_HEIGHT)
}

/// Draws the key cap of `key` at `x`, centered in the height of `line`.
/// Returns the right edge of the cap.
fn keycap(context: &mut UiContext, x: f32, line: Rect, key: HintKey) -> f32 {
    let theme = context.theme;
    let style = theme.small().with_color(theme.colors.text);
    let small_radius = theme.radius * 2.0 / 3.0;
    let half = KEYCAP_HEIGHT * 0.5;
    let width = keycap_width(context, key);
    let (text, radii) = match key {
        HintKey::Key(key) => (Some(context.catalog.text(key)), [small_radius; 4]),
        HintKey::Menu(text) => (Some(text), [small_radius; 4]),
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
        ui::tests::{
            Harness, TEST_ACTIVATE_KEYS, TEST_CANCEL_KEYS, pixel, srgb, test_catalog, test_key,
        },
    };

    /// The area of the status line at the bottom of the target.
    const LINE: Rect = Rect::new(0.0, 572.0, 800.0, STATUS_LINE_HEIGHT);

    /// The hints of the library.
    const HINTS: [Hint; 2] = [
        Hint {
            keys: TEST_ACTIVATE_KEYS,
            label: "Launch",
        },
        Hint {
            keys: &[
                HintKey::Key(test_key("ctrl-page-up")),
                HintKey::Shoulder("LB"),
                HintKey::Shoulder("RB"),
            ],
            label: "System",
        },
    ];

    #[test]
    fn the_line_draws_its_well_its_rule_and_its_caps() {
        let mut harness = Harness::new();
        harness.frame(|context| {
            status_line(
                context,
                LINE,
                &HINTS,
                StatusEnd::Path("config/pc98/yu-no.conf"),
            );
        });
        let pixels = harness.render();
        let width = harness.viewport.width;
        let theme = &harness.theme;
        assert_eq!(
            pixel(&pixels, width, 400.0, LINE.y + 0.5),
            srgb(theme.colors.panel_edge)
        );
        assert_eq!(
            pixel(&pixels, width, 400.0, LINE.y + 3.0),
            srgb(theme.colors.well)
        );
        let caps = harness
            .scene
            .instances()
            .iter()
            .filter(|instance| {
                instance.kind[0] == Instance::KIND_RECTANGLE && instance.rect[3] == KEYCAP_HEIGHT
            })
            .count();
        assert_eq!(caps, 5);
        let shoulder = harness
            .scene
            .instances()
            .iter()
            .filter(|instance| instance.rect[3] == KEYCAP_HEIGHT)
            .find(|instance| instance.radii[0] != instance.radii[2])
            .expect("a shoulder cap");
        assert_eq!(shoulder.radii[0], KEYCAP_HEIGHT * 0.5);
    }

    /// Verifies that a key cap holds its text, the padding of `gap * 0.625` and the border on each side, and that a short key keeps the width of the height.
    #[test]
    fn a_key_cap_holds_its_text_its_padding_and_its_border() {
        let mut harness = Harness::new();
        let (text, wide, short) = harness.frame(|context| {
            let style = context.theme.small().with_color(context.theme.colors.text);
            (
                context.measure(&style, test_catalog().text(test_key("ctrl-page-up"))),
                keycap_width(context, HintKey::Key(test_key("ctrl-page-up"))),
                keycap_width(context, HintKey::Shoulder("I")),
            )
        });
        let theme = &harness.theme;
        let expected = text + theme.gap * 0.625 * 2.0 + theme.stroke * 2.0;
        assert!((wide - expected).abs() < 0.01, "{wide} {expected}");
        assert_eq!(short, KEYCAP_HEIGHT);
        harness.frame(|context| {
            status_line(
                context,
                LINE,
                &HINTS[1..],
                StatusEnd::Path("config/pc98/yu-no.conf"),
            );
        });
        let first = harness
            .scene
            .instances()
            .iter()
            .find(|instance| {
                instance.kind[0] == Instance::KIND_RECTANGLE && instance.rect[3] == KEYCAP_HEIGHT
            })
            .expect("the first cap")
            .rect;
        assert!(
            (first[2] - expected.round()).abs() <= 1.0,
            "{first:?} {expected}"
        );
    }

    /// Verifies that the D-pad cap is a round cap of its width with a left and a right arrow
    /// inside it, a gap between the arrows and no text.
    #[test]
    fn the_dpad_cap_draws_a_left_and_a_right_arrow() {
        let mut harness = Harness::new();
        harness.frame(|context| {
            let hint = Hint {
                keys: &[HintKey::DpadLeftRight],
                label: "",
            };
            status_line(context, LINE, &[hint], StatusEnd::None);
        });
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

    #[test]
    fn hints_that_do_not_fit_are_left_out() {
        let mut harness = Harness::new();
        let narrow = Rect::new(0.0, 572.0, 200.0, STATUS_LINE_HEIGHT);
        harness.frame(|context| {
            status_line(
                context,
                narrow,
                &HINTS,
                StatusEnd::Path("config/pc98/yu-no.conf"),
            );
        });
        let caps = harness
            .scene
            .instances()
            .iter()
            .filter(|instance| {
                instance.kind[0] == Instance::KIND_RECTANGLE && instance.rect[3] == KEYCAP_HEIGHT
            })
            .count();
        assert_eq!(caps, 2);
        for instance in harness.scene.instances() {
            if instance.kind[0] != Instance::KIND_RECTANGLE {
                assert!(
                    instance.rect[0] + instance.rect[2] <= 200.0 - 15.0,
                    "{instance:?}"
                );
            }
        }
    }

    #[test]
    fn an_end_without_its_minimum_room_is_left_out() {
        let mut harness = Harness::new();
        let minimum = harness.theme.status_end_min_width;
        let width = harness.frame(|context| {
            let style = context.theme.small();
            let hint = &HINTS[..1];
            let mut x = 0.0;
            for key in hint[0].keys {
                x += keycap_width(context, *key) + context.theme.gap * 0.5;
            }
            x + context.theme.gap * 2.25 + context.measure(&style, hint[0].label)
        });
        for (extra, shown) in [(minimum - 10.0, false), (minimum + 10.0, true)] {
            let line = Rect::new(0.0, 572.0, 32.0 + width + extra, STATUS_LINE_HEIGHT);
            harness.frame(|context| {
                status_line(
                    context,
                    line,
                    &HINTS[..1],
                    StatusEnd::Path("config/pc98/yu-no.conf"),
                );
            });
            let glyphs: Vec<f32> = harness
                .scene
                .instances()
                .iter()
                .filter(|instance| instance.kind[0] == Instance::KIND_GLYPH)
                .map(|instance| instance.rect[0])
                .collect();
            let end_glyphs = glyphs.iter().filter(|&&x| x > 16.0 + width).count();
            assert_eq!(end_glyphs > 0, shown, "{extra}");
        }
    }

    #[test]
    fn a_short_end_that_fits_a_small_room_shows() {
        let mut harness = Harness::new();
        let path = "a.conf";
        let (hint_width, path_width) = harness.frame(|context| {
            let style = context.theme.small();
            (
                hint_width(context, &HINTS[0]),
                context.measure(&style, path),
            )
        });
        let gap = harness.theme.gap * 2.0;
        let width = 32.0 + hint_width + gap + path_width + 4.0;
        assert!(width - 32.0 - hint_width - gap < harness.theme.status_end_min_width);
        let line = Rect::new(0.0, 572.0, width, STATUS_LINE_HEIGHT);
        harness.frame(|context| status_line(context, line, &HINTS[..1], StatusEnd::Path(path)));
        let end_glyphs = harness
            .scene
            .instances()
            .iter()
            .filter(|instance| {
                instance.kind[0] == Instance::KIND_GLYPH
                    && instance.rect[0] > 16.0 + hint_width + gap
            })
            .count();
        assert!(end_glyphs >= path.len() - 1, "{end_glyphs}");
    }

    #[test]
    fn the_end_gives_way_to_the_hints() {
        let mut harness = Harness::new();
        let narrow = Rect::new(0.0, 572.0, 480.0, STATUS_LINE_HEIGHT);
        harness.frame(|context| {
            status_line(
                context,
                narrow,
                &HINTS,
                StatusEnd::Error("config/x68k/nemesis-90.conf line 4: unknown machine X68030"),
            );
        });
        for instance in harness.scene.instances() {
            if instance.kind[0] != Instance::KIND_RECTANGLE {
                assert!(
                    instance.rect[0] + instance.rect[2] <= 480.0 - 15.0,
                    "{instance:?}"
                );
            }
        }
        let alert = harness
            .scene
            .instances()
            .iter()
            .filter(|instance| instance.kind[0] == Instance::KIND_GLYPH)
            .any(|instance| instance.color == harness.theme.colors.danger_text.to_linear());
        assert!(alert);
    }

    /// Verifies that a hint with the action of an earlier hint leaves the line whatever its label, and that the order stays.
    #[test]
    fn a_repeated_result_shows_once() {
        let close_button = Hint {
            keys: TEST_ACTIVATE_KEYS,
            label: "Cancel",
        };
        let section = Hint {
            keys: &[HintKey::Shoulder("LB")],
            label: "Section",
        };
        let close = Hint {
            keys: TEST_CANCEL_KEYS,
            label: "Close",
        };
        let hints: StackVec<Hint, 3> = distinct_hints([
            ("close", close_button),
            ("section", section),
            ("close", close),
        ]);
        assert_eq!(*hints, [close_button, section]);
        let other = Hint {
            keys: &[HintKey::Pad("X")],
            label: "Section",
        };
        let hints: StackVec<Hint, 2> = distinct_hints([("section", section), ("clear", other)]);
        assert_eq!(*hints, [section, other]);
    }
}
