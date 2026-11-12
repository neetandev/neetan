//! The status line at the bottom of a screen: the keys and gamepad buttons
//! of the actions of the top dialog, and a path or an error at its end.

use stack_vec::StackVec;

use super::{
    context::UiContext,
    icons::Icon,
    keycap::{HintKey, keycap, keycap_width},
    label::{Align, Truncation},
};
use crate::Rect;

/// The height of the status line.
pub const STATUS_LINE_HEIGHT: f32 = 28.0;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Instance,
        ui::{
            KEYCAP_HEIGHT,
            tests::{Harness, TEST_ACTIVATE_KEYS, TEST_CANCEL_KEYS, pixel, srgb, test_key},
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

    /// Verifies that the line draws each key cap at the width of the cap.
    #[test]
    fn the_line_draws_each_cap_at_its_width() {
        let mut harness = Harness::new();
        let width =
            harness.frame(|context| keycap_width(context, HintKey::Key(test_key("ctrl-page-up"))));
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
        assert!((first[2] - width.round()).abs() <= 1.0, "{first:?} {width}");
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
