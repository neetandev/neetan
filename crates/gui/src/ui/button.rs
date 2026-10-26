//! The button and the toggle button.

use super::{
    context::{UiContext, UiId},
    focus::NavigationSet,
    icons::Icon,
    theme::BUTTON_EDGE_TINT_SHARE,
};
use crate::{Color, Rect, RectStyle};

/// The look of a button.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonKind {
    /// A raised button with an edge.
    #[default]
    Normal,
    /// The accent button of the main action of a panel.
    Primary,
    /// The red button of a destructive action, such as a delete.
    Danger,
    /// A flat button that shows its face only under the pointer, for icon
    /// buttons in a header.
    Quiet,
}

/// A button with a caption, an icon or both.
///
/// The button takes the focus on a click and joins the focus ring of the
/// pass. A click or confirm while it holds the focus activates it. A toggle
/// button shows whether it is selected, and the caller flips the selection
/// on activation.
#[derive(Clone, Copy, Debug)]
pub struct Button<'a> {
    /// The id of the button.
    id: UiId,
    /// The caption.
    caption: &'a str,
    /// The icon before the caption.
    icon: Option<Icon>,
    /// The look.
    kind: ButtonKind,
    /// The selection of a toggle button.
    selected: Option<bool>,
    /// Whether the button reacts to input.
    enabled: bool,
    /// Whether a modal that opens may give the button the focus.
    default_choice: bool,
}

/// The smallest width of a button with a caption.
pub const BUTTON_MIN_WIDTH: f32 = 96.0;

/// The smallest width of a primary or danger button with a caption.
pub const PRIMARY_BUTTON_MIN_WIDTH: f32 = 104.0;

impl<'a> Button<'a> {
    /// A normal button with the caption `caption`.
    pub fn new(id: impl Into<UiId>, caption: &'a str) -> Self {
        Self {
            id: id.into(),
            caption,
            icon: None,
            kind: ButtonKind::Normal,
            selected: None,
            enabled: true,
            default_choice: true,
        }
    }

    /// The same button, which a modal that opens never gives the focus
    /// unless `choice` is set, such as the close button of a header.
    pub fn default_choice(self, choice: bool) -> Self {
        Self {
            default_choice: choice,
            ..self
        }
    }

    /// The same button with `icon` before its caption.
    pub fn icon(self, icon: Icon) -> Self {
        Self {
            icon: Some(icon),
            ..self
        }
    }

    /// The same button with the look `kind`.
    pub fn kind(self, kind: ButtonKind) -> Self {
        Self { kind, ..self }
    }

    /// The same button as a toggle button in the state `selected`.
    pub fn selected(self, selected: bool) -> Self {
        Self {
            selected: Some(selected),
            ..self
        }
    }

    /// The same button, disabled unless `enabled`. A disabled button draws
    /// dimmed, ignores input and leaves the focus ring. It takes the pointer
    /// over it, and a press on it reaches no widget after it.
    pub fn enabled(self, enabled: bool) -> Self {
        Self { enabled, ..self }
    }

    /// The width the button takes in a row of buttons: the icon, the gap
    /// and the caption plus the padding on each side, and at least
    /// [`BUTTON_MIN_WIDTH`], or [`PRIMARY_BUTTON_MIN_WIDTH`] for a primary
    /// or danger button. A button without a caption is square.
    pub fn width(&self, context: &mut UiContext) -> f32 {
        let theme = context.theme;
        if self.caption.is_empty() {
            return theme.control_height;
        }
        let leading = match (self.selected, self.icon) {
            (Some(_), _) => CHECK_BOX_SIZE + theme.gap * 0.75,
            (None, Some(_)) => theme.icon_size + theme.gap * 0.75,
            (None, None) => 0.0,
        };
        let caption = context.measure(&theme.body(), self.caption);
        let minimum = match self.kind {
            ButtonKind::Primary | ButtonKind::Danger => PRIMARY_BUTTON_MIN_WIDTH,
            ButtonKind::Normal | ButtonKind::Quiet => BUTTON_MIN_WIDTH,
        };
        (leading + caption + theme.padding * 2.0)
            .ceil()
            .max(minimum)
    }

    /// Draws the button in `rect` and returns whether a click or confirm
    /// activated it in this pass.
    pub fn show(self, context: &mut UiContext, rect: Rect) -> bool {
        let theme = context.theme;
        if !self.enabled {
            context.absorb_pointer(rect);
            if self.kind != ButtonKind::Quiet {
                context.rect(
                    rect,
                    &RectStyle::filled(theme.colors.control.with_alpha(0.5), theme.radius)
                        .with_stroke(theme.stroke, theme.colors.control_edge.with_alpha(0.5)),
                );
            }
            self.content(context, rect, theme.colors.text_disabled);
            return false;
        }
        if !self.default_choice {
            context.focusable_secondary(self.id, rect, NavigationSet::EMPTY);
        } else if self.kind == ButtonKind::Primary {
            context.focusable_preferred(self.id, rect, NavigationSet::EMPTY);
        } else {
            context.focusable(self.id, rect, NavigationSet::EMPTY);
        }
        let pointer = context.button_behavior(self.id, rect);
        let focused = context.shows_focus(self.id);
        let activated = pointer.clicked || context.confirmed(self.id);
        let pressed = pointer.held && pointer.hovered;
        let hovered = pointer.hovered || pointer.held;
        let (style, color) = match self.kind {
            ButtonKind::Normal => {
                let fill = if pressed {
                    theme.colors.control_pressed
                } else if hovered {
                    theme.colors.control_hover
                } else {
                    theme.colors.control
                };
                (
                    RectStyle::filled(fill, theme.radius)
                        .with_stroke(theme.stroke, theme.colors.control_edge),
                    theme.colors.text,
                )
            }
            ButtonKind::Primary => {
                let fill = if pressed {
                    theme.colors.accent_pressed
                } else if hovered {
                    theme.colors.accent_hover
                } else {
                    theme.colors.accent
                };
                (
                    RectStyle::filled(fill, theme.radius).with_stroke(
                        theme.stroke,
                        fill.mix(theme.colors.button_edge_tint, BUTTON_EDGE_TINT_SHARE),
                    ),
                    theme.colors.accent_text,
                )
            }
            ButtonKind::Danger => {
                let fill = if pressed {
                    theme.colors.danger_pressed
                } else if hovered {
                    theme.colors.danger_hover
                } else {
                    theme.colors.danger
                };
                (
                    RectStyle::filled(fill, theme.radius).with_stroke(
                        theme.stroke,
                        fill.mix(theme.colors.button_edge_tint, BUTTON_EDGE_TINT_SHARE),
                    ),
                    theme.colors.accent_text,
                )
            }
            ButtonKind::Quiet => {
                let fill = if pressed {
                    theme.colors.control_pressed
                } else if hovered {
                    theme.colors.control_hover
                } else {
                    Color::TRANSPARENT
                };
                let color = if hovered || focused {
                    theme.colors.text
                } else {
                    theme.colors.text_dim
                };
                (RectStyle::filled(fill, theme.radius), color)
            }
        };
        context.rect(rect, &style);
        self.content(context, rect, color);
        if focused {
            context.focus_ring(rect, theme.radius);
        }
        activated
    }

    /// Draws the leading element and the caption as one centered group in
    /// `color`. The leading element is the check box of a toggle button or
    /// the icon.
    fn content(&self, context: &mut UiContext, rect: Rect, color: Color) {
        let theme = context.theme;
        let style = theme.body().with_color(color);
        let inner = rect.inset(theme.gap);
        let leading = match (self.selected, self.icon) {
            (Some(_), _) => CHECK_BOX_SIZE,
            (None, Some(_)) => theme.icon_size.min(rect.height - 4.0).max(0.0),
            (None, None) => {
                context.text_centered(inner, &style, self.caption);
                return;
            }
        };
        let (spacing, caption_width) = if self.caption.is_empty() {
            (0.0, 0.0)
        } else {
            let spacing = theme.gap * 0.75;
            let width = context
                .measure(&style, self.caption)
                .min(inner.width - leading - spacing)
                .max(0.0);
            (spacing, width)
        };
        let group = leading + spacing + caption_width;
        let left = context.snap(rect.x + (rect.width - group) * 0.5);
        let slot = Rect::new(left, rect.y, leading, rect.height);
        match (self.selected, self.icon) {
            (Some(selected), _) => {
                let look = if self.enabled {
                    CheckBoxLook::Idle
                } else {
                    CheckBoxLook::Disabled
                };
                check_box(context, slot, selected, look);
            }
            (None, Some(icon)) => context.icon(slot, icon, leading, color),
            (None, None) => {}
        }
        if !self.caption.is_empty() {
            let caption = Rect::new(
                slot.right() + spacing,
                rect.y,
                caption_width + 1.0,
                rect.height,
            );
            context.text_left(caption, &style, self.caption);
        }
    }
}

/// The size of the check box of a toggle button and a checkbox.
pub(super) const CHECK_BOX_SIZE: f32 = 14.0;

/// The corner radius of the check box.
const CHECK_BOX_RADIUS: f32 = 3.0;

/// The state of a check box beyond its value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum CheckBoxLook {
    /// Without the pointer.
    #[default]
    Idle,
    /// Under the pointer.
    Hovered,
    /// Under a press of the pointer.
    Pressed,
    /// Without input.
    Disabled,
}

/// Draws a check box centered vertically in `slot`, checked when
/// `selected`, in the state `look`.
pub(super) fn check_box(context: &mut UiContext, slot: Rect, selected: bool, look: CheckBoxLook) {
    let theme = context.theme;
    let top = context.snap(slot.y + (slot.height - CHECK_BOX_SIZE) * 0.5);
    let area = Rect::new(slot.x, top, CHECK_BOX_SIZE, CHECK_BOX_SIZE);
    if selected {
        let fill = match look {
            CheckBoxLook::Idle => theme.colors.accent,
            CheckBoxLook::Hovered => theme.colors.accent_hover,
            CheckBoxLook::Pressed => theme.colors.accent_pressed,
            CheckBoxLook::Disabled => theme.colors.text_disabled,
        };
        context.rect(area, &RectStyle::filled(fill, CHECK_BOX_RADIUS));
        context.icon(
            area,
            Icon::Check,
            CHECK_BOX_SIZE - 2.0,
            theme.colors.accent_text,
        );
    } else {
        let (fill, edge) = match look {
            CheckBoxLook::Idle => (theme.colors.field, theme.colors.text_dim),
            CheckBoxLook::Hovered => (theme.colors.field, theme.colors.text),
            CheckBoxLook::Pressed => (theme.colors.control_pressed, theme.colors.text),
            CheckBoxLook::Disabled => (theme.colors.field, theme.colors.text_disabled),
        };
        context.rect(
            area,
            &RectStyle::filled(fill, CHECK_BOX_RADIUS).with_stroke(theme.stroke * 1.5, edge),
        );
    }
}

/// A normal button with the caption `caption` in `rect`. Returns whether a
/// click or confirm activated it.
pub fn button(context: &mut UiContext, id: impl Into<UiId>, rect: Rect, caption: &str) -> bool {
    Button::new(id, caption).show(context, rect)
}

/// A toggle button with the caption `caption` in `rect` in the state
/// `selected`. Returns whether a click or confirm activated it. The caller
/// flips its state on activation.
pub fn toggle_button(
    context: &mut UiContext,
    id: impl Into<UiId>,
    rect: Rect,
    caption: &str,
    selected: bool,
) -> bool {
    Button::new(id, caption)
        .selected(selected)
        .show(context, rect)
}

/// A quiet button that shows `icon` in `rect`. Returns whether a click or
/// confirm activated it.
pub fn icon_button(context: &mut UiContext, id: impl Into<UiId>, rect: Rect, icon: Icon) -> bool {
    Button::new(id, "")
        .icon(icon)
        .kind(ButtonKind::Quiet)
        .show(context, rect)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{
        Navigation,
        tests::{Harness, pixel, srgb},
        ui_id,
    };

    /// The rectangles of two buttons side by side.
    const OK: Rect = Rect::new(10.0, 10.0, 80.0, 32.0);
    /// The rectangle of the second button.
    const CANCEL: Rect = Rect::new(100.0, 10.0, 80.0, 32.0);

    /// Draws the two buttons and returns their activations.
    fn draw(context: &mut UiContext) -> (bool, bool) {
        let ok = Button::new("ok", "OK")
            .kind(ButtonKind::Primary)
            .show(context, OK);
        let cancel = button(context, "cancel", CANCEL, "Cancel");
        (ok, cancel)
    }

    #[test]
    fn a_click_activates_a_button() {
        let mut harness = Harness::new();
        harness.frame(draw);
        harness.press(40.0, 20.0);
        assert_eq!(harness.frame(draw), (false, false));
        harness.release();
        assert_eq!(harness.frame(draw), (true, false));
        assert_eq!(harness.state.focused(), Some(ui_id("ok")));
    }

    #[test]
    fn a_press_that_leaves_the_button_does_not_activate_it() {
        let mut harness = Harness::new();
        harness.press(40.0, 20.0);
        harness.frame(draw);
        harness.mouse(140.0, 20.0);
        harness.frame(draw);
        harness.release();
        assert_eq!(harness.frame(draw), (false, false));
    }

    #[test]
    fn confirm_activates_the_focused_button() {
        let mut harness = Harness::new();
        harness.frame(draw);
        harness.input.confirm = true;
        assert_eq!(harness.frame(draw), (false, false));
        harness.navigate(Navigation::Next);
        harness.frame(draw);
        harness.navigate(Navigation::Right);
        harness.frame(draw);
        harness.input.confirm = true;
        assert_eq!(harness.frame(draw), (false, true));
        harness.navigate(Navigation::Left);
        harness.frame(draw);
        harness.input.confirm = true;
        assert_eq!(harness.frame(draw), (true, false));
    }

    #[test]
    fn a_button_that_becomes_disabled_loses_its_press() {
        let mut harness = Harness::new();
        harness.press(40.0, 20.0);
        harness.frame(|context| Button::new("ok", "OK").show(context, OK));
        harness.release();
        let activated =
            harness.frame(|context| Button::new("ok", "OK").enabled(false).show(context, OK));
        assert!(!activated);
        assert!(!harness.frame(|context| context.any_active()));
    }

    #[test]
    fn a_disabled_button_ignores_input_and_leaves_the_ring() {
        let mut harness = Harness::new();
        let draw =
            |context: &mut UiContext| Button::new("ok", "OK").enabled(false).show(context, OK);
        harness.press(40.0, 20.0);
        harness.frame(draw);
        harness.release();
        assert!(!harness.frame(draw));
        harness.navigate(Navigation::Next);
        harness.frame(draw);
        harness.input.confirm = true;
        assert!(!harness.frame(draw));
        assert_eq!(harness.state.focused(), None);
    }

    /// Verifies that a disabled button takes the pointer over it from the widgets after it.
    #[test]
    fn a_disabled_button_takes_the_pointer() {
        let mut harness = Harness::new();
        for (x, taken) in [(40.0, true), (OK.right() + 10.0, false)] {
            harness.press(x, 20.0);
            let takes_pointer = harness.frame(|context| {
                Button::new("ok", "OK").enabled(false).show(context, OK);
                context.takes_pointer()
            });
            assert_eq!(takes_pointer, !taken, "{x}");
            harness.release();
            harness.frame(|_| {});
        }
    }

    #[test]
    fn a_toggle_button_reports_its_activation() {
        let mut harness = Harness::new();
        let mut selected = false;
        for _ in 0..2 {
            harness.press(40.0, 20.0);
            harness.frame(|context| toggle_button(context, "protect", OK, "Protect", selected));
            harness.release();
            if harness.frame(|context| toggle_button(context, "protect", OK, "Protect", selected)) {
                selected = !selected;
            }
        }
        assert!(!selected);
        harness.input.confirm = true;
        assert!(harness.frame(|context| toggle_button(context, "protect", OK, "Protect", true)));
    }

    #[test]
    fn the_focus_ring_shows_after_a_key_and_hides_after_a_click() {
        let mut harness = Harness::new();
        harness.frame(draw);
        let unfocused = harness.scene.len();
        harness.navigate(Navigation::Next);
        harness.frame(draw);
        assert_eq!(harness.scene.len(), unfocused + 1);
        let theme = &harness.theme;
        let outer = OK.expand(theme.focus_gap + theme.focus_width);
        let expected = [outer.x, outer.y, outer.width, outer.height];
        let width = theme.focus_width;
        let ring = harness
            .scene
            .instances()
            .iter()
            .find(|instance| instance.rect == expected && instance.parameters[0] == width);
        assert!(ring.is_some());
        harness.press(40.0, 20.0);
        harness.frame(draw);
        assert_eq!(harness.scene.len(), unfocused);
    }

    #[test]
    fn a_danger_button_takes_the_danger_fills() {
        let theme = Harness::new().theme;
        for (point, fill) in [
            (None, theme.colors.danger),
            (Some((40.0, 20.0)), theme.colors.danger_hover),
        ] {
            let mut harness = Harness::new();
            if let Some((x, y)) = point {
                harness.mouse(x, y);
            }
            harness.frame(|context| {
                Button::new("delete", "Delete")
                    .icon(Icon::Delete)
                    .kind(ButtonKind::Danger)
                    .show(context, OK)
            });
            let pixels = harness.render();
            assert_eq!(
                pixel(&pixels, harness.viewport.width, OK.x + 4.0, OK.y + 4.0),
                srgb(fill)
            );
        }
        let mut harness = Harness::new();
        harness.press(40.0, 20.0);
        harness.frame(|context| {
            Button::new("delete", "Delete")
                .kind(ButtonKind::Danger)
                .show(context, OK)
        });
        let pixels = harness.render();
        assert_eq!(
            pixel(&pixels, harness.viewport.width, OK.x + 4.0, OK.y + 4.0),
            srgb(theme.colors.danger_pressed)
        );
    }

    #[test]
    fn the_width_follows_the_content_and_the_minimum() {
        let mut harness = Harness::new();
        let (cancel, delete, long, square) = harness.frame(|context| {
            (
                Button::new("cancel", "Cancel").width(context),
                Button::new("delete", "Delete")
                    .icon(Icon::Delete)
                    .kind(ButtonKind::Danger)
                    .width(context),
                Button::new("defaults", "System defaults")
                    .icon(Icon::Settings)
                    .width(context),
                Button::new("close", "").icon(Icon::Close).width(context),
            )
        });
        assert_eq!(cancel, BUTTON_MIN_WIDTH);
        assert_eq!(delete, PRIMARY_BUTTON_MIN_WIDTH);
        assert!(long > BUTTON_MIN_WIDTH + 30.0, "{long}");
        assert_eq!(square, harness.theme.control_height);
    }
}
