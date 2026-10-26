//! The checkbox: a check box with a caption.

use super::{
    button::{CHECK_BOX_SIZE, CheckBoxLook, check_box},
    context::{UiContext, UiId},
    focus::NavigationSet,
};
use crate::Rect;

/// A check box with a caption after it.
///
/// A click or confirm flips the value. The box darkens under a press and
/// the value flips on the release. The checkbox joins the focus ring. Its
/// area spans the box, the caption and a small padding after it, from the
/// left edge of the rectangle the caller gives it.
#[derive(Clone, Copy, Debug)]
pub struct Checkbox<'a> {
    /// The id of the checkbox.
    id: UiId,
    /// The caption.
    caption: &'a str,
    /// Whether the checkbox reacts to input.
    enabled: bool,
}

impl<'a> Checkbox<'a> {
    /// An enabled checkbox with the caption `caption`.
    pub fn new(id: impl Into<UiId>, caption: &'a str) -> Self {
        Self {
            id: id.into(),
            caption,
            enabled: true,
        }
    }

    /// The same checkbox, disabled unless `enabled`. A disabled checkbox
    /// draws dimmed, ignores input and leaves the focus ring. It takes the
    /// pointer over it, and a press on it reaches no widget after it.
    pub fn enabled(self, enabled: bool) -> Self {
        Self { enabled, ..self }
    }

    /// The width of the box, the gap, the caption and the padding after it.
    pub fn width(&self, context: &mut UiContext) -> f32 {
        let theme = context.theme;
        let caption = context.measure(&theme.body(), self.caption);
        (CHECK_BOX_SIZE + theme.gap * 0.75 + caption + theme.gap * 0.5).ceil()
    }

    /// Draws the checkbox at the left of `rect`, centered vertically, over
    /// `value`. Returns whether the value flipped in this pass.
    pub fn show(self, context: &mut UiContext, rect: Rect, value: &mut bool) -> bool {
        let theme = context.theme;
        let width = self.width(context).min(rect.width.max(0.0));
        let height = theme.control_height.min(rect.height.max(0.0));
        let area = context.snap_rect(Rect::new(
            rect.x,
            rect.y + (rect.height - height) * 0.5,
            width,
            height,
        ));
        let mut flipped = false;
        let look = if self.enabled {
            context.focusable(self.id, area, NavigationSet::EMPTY);
            let pointer = context.button_behavior(self.id, area);
            if pointer.clicked || context.confirmed(self.id) {
                *value = !*value;
                flipped = true;
            }
            if pointer.held && pointer.hovered {
                CheckBoxLook::Pressed
            } else if pointer.hovered || pointer.held {
                CheckBoxLook::Hovered
            } else {
                CheckBoxLook::Idle
            }
        } else {
            context.absorb_pointer(area);
            CheckBoxLook::Disabled
        };
        let (slot, rest) = area.cut_left(CHECK_BOX_SIZE);
        check_box(context, slot, *value, look);
        let color = if self.enabled {
            theme.colors.text
        } else {
            theme.colors.text_disabled
        };
        let caption = rest.cut_left(theme.gap * 0.75).1;
        context.text_left(caption, &theme.body().with_color(color), self.caption);
        if self.enabled && context.shows_focus(self.id) {
            context.focus_ring(area, theme.radius);
        }
        flipped
    }
}

/// A checkbox with the caption `caption` at the left of `rect` over
/// `value`, with the behavior of [`Checkbox`]. Returns whether the value
/// flipped.
pub fn checkbox(
    context: &mut UiContext,
    id: impl Into<UiId>,
    rect: Rect,
    caption: &str,
    value: &mut bool,
) -> bool {
    Checkbox::new(id, caption).show(context, rect, value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{
        Navigation,
        tests::{Harness, pixel, srgb},
    };

    /// The area the checkbox draws in.
    const AREA: Rect = Rect::new(10.0, 10.0, 300.0, 32.0);

    /// Draws the checkbox over `value` and returns whether it flipped.
    fn draw(harness: &mut Harness, value: &mut bool) -> bool {
        harness.frame(|context| checkbox(context, "all", AREA, "Show all files", value))
    }

    /// The center of the check box.
    fn box_center() -> (f32, f32) {
        (AREA.x + CHECK_BOX_SIZE * 0.5, AREA.center().1)
    }

    #[test]
    fn a_click_flips_the_value_on_the_release() {
        let mut harness = Harness::new();
        let mut value = false;
        draw(&mut harness, &mut value);
        harness.press(60.0, 26.0);
        assert!(!draw(&mut harness, &mut value));
        assert!(!value);
        harness.release();
        assert!(draw(&mut harness, &mut value));
        assert!(value);
        assert_eq!(harness.state.focused(), Some(UiId::from("all")));
        harness.press(400.0, 26.0);
        draw(&mut harness, &mut value);
        harness.release();
        assert!(!draw(&mut harness, &mut value));
    }

    #[test]
    fn confirm_flips_the_focused_checkbox() {
        let mut harness = Harness::new();
        let mut value = true;
        draw(&mut harness, &mut value);
        harness.navigate(Navigation::Next);
        draw(&mut harness, &mut value);
        harness.input.confirm = true;
        assert!(draw(&mut harness, &mut value));
        assert!(!value);
    }

    #[test]
    fn a_disabled_checkbox_ignores_input_and_leaves_the_ring() {
        let mut harness = Harness::new();
        let mut value = true;
        let draw = |harness: &mut Harness, value: &mut bool| {
            harness.frame(|context| {
                Checkbox::new("all", "Show all files")
                    .enabled(false)
                    .show(context, AREA, value)
            })
        };
        draw(&mut harness, &mut value);
        harness.navigate(Navigation::Next);
        draw(&mut harness, &mut value);
        assert_eq!(harness.state.focused(), None);
        harness.press(16.0, 26.0);
        draw(&mut harness, &mut value);
        harness.release();
        assert!(!draw(&mut harness, &mut value));
        assert!(value);
        let pixels = harness.render();
        let (x, y) = box_center();
        let center = pixel(&pixels, harness.viewport.width, x - 4.0, y + 3.0);
        assert_eq!(center, srgb(harness.theme.colors.text_disabled));
    }

    #[test]
    fn the_box_takes_the_accent_when_checked_and_darkens_under_a_press() {
        let mut harness = Harness::new();
        let mut value = true;
        draw(&mut harness, &mut value);
        let (x, y) = box_center();
        let theme_accent = srgb(harness.theme.colors.accent);
        let pixels = harness.render();
        assert_eq!(
            pixel(&pixels, harness.viewport.width, x - 4.0, y + 3.0),
            theme_accent
        );
        let mut value = false;
        harness.press(x, y);
        draw(&mut harness, &mut value);
        let pixels = harness.render();
        assert_eq!(
            pixel(&pixels, harness.viewport.width, x, y),
            srgb(harness.theme.colors.control_pressed)
        );
        harness.release();
        draw(&mut harness, &mut value);
        assert!(value);
    }

    #[test]
    fn the_width_spans_the_box_the_caption_and_the_padding() {
        let mut harness = Harness::new();
        let (width, caption) = harness.frame(|context| {
            let theme = context.theme;
            let caption = context.measure(&theme.body(), "Show all files");
            (
                Checkbox::new("all", "Show all files").width(context),
                caption,
            )
        });
        let theme = &harness.theme;
        let expected = CHECK_BOX_SIZE + theme.gap * 1.25 + caption;
        assert!(width >= expected && width < expected + 1.0);
    }
}
