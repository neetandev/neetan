//! The dropdown: a button that shows the chosen value and opens a list of
//! the choices on the popup layer.

use super::{
    context::{UiContext, UiId},
    focus::NavigationSet,
    icons::Icon,
    label::{Align, Truncation},
    list::{LIST_PADDING, List, ListRow},
    modal::SHADOW_OFFSET,
};
use crate::{Rect, RectStyle};

/// The rows a dropdown popup shows before it scrolls.
pub const POPUP_MAX_ROWS: usize = 8;

/// A button that shows the chosen value and opens a popup with the choices.
///
/// The value is the index of a choice, or `None` for the inherited value.
/// A dropdown with an inherited entry shows it first in the popup, with a
/// rule below it, and shows its label dimmed while the value is `None`.
///
/// A press or confirm opens the popup below the dropdown, or above it when
/// the target ends first. The dropdown shows the press while the button is
/// held over it. The popup has the width of the dropdown, marks
/// the value with a check and highlights it. While it is open only the
/// popup takes the focus, the navigation and the pointer. Up and down shift
/// the highlight, confirm or a click picks the highlighted entry, and
/// cancel or a press outside closes the popup without a change. A release
/// over an entry picks it only when the press started on it. A popup with
/// more than [`POPUP_MAX_ROWS`] rows scrolls like a list. A dropdown that
/// turns disabled while its popup is open closes the popup and gives the
/// focus back.
///
/// The captions of the choices are any text type, such as `&str`, `String`
/// or `Cow<str>`.
#[derive(Debug)]
pub struct Dropdown<'a, T: AsRef<str> = &'a str> {
    /// The id of the dropdown.
    id: UiId,
    /// The captions of the choices.
    choices: &'a [T],
    /// The label of the inherited value, the first entry of the popup.
    inherited: Option<&'a str>,
    /// Whether the dropdown reacts to input.
    enabled: bool,
}

impl<T: AsRef<str>> Clone for Dropdown<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: AsRef<str>> Copy for Dropdown<'_, T> {}

impl<'a, T: AsRef<str>> Dropdown<'a, T> {
    /// An enabled dropdown over `choices` without an inherited entry.
    pub fn new(id: impl Into<UiId>, choices: &'a [T]) -> Self {
        Self {
            id: id.into(),
            choices,
            inherited: None,
            enabled: true,
        }
    }

    /// The same dropdown with the inherited entry `label`, such as
    /// `System default: PC9801RA`. The entry selects the value `None`.
    pub fn inherited(self, label: &'a str) -> Self {
        Self {
            inherited: Some(label),
            ..self
        }
    }

    /// The same dropdown, disabled unless `enabled`. A disabled dropdown
    /// draws dimmed, ignores input and leaves the focus ring.
    pub fn enabled(self, enabled: bool) -> Self {
        Self { enabled, ..self }
    }

    /// The number of entries of the popup.
    fn entries(&self) -> usize {
        self.choices.len() + usize::from(self.inherited.is_some())
    }

    /// The popup entry of `value`.
    fn entry_of(&self, value: Option<usize>) -> usize {
        let offset = usize::from(self.inherited.is_some());
        match value {
            Some(choice) => choice.min(self.choices.len().saturating_sub(1)) + offset,
            None => 0,
        }
    }

    /// The value of the popup entry `entry`.
    fn value_of(&self, entry: usize) -> Option<usize> {
        if self.inherited.is_some() {
            entry.checked_sub(1)
        } else {
            Some(entry)
        }
    }

    /// The caption of `value` and whether it is the inherited label.
    fn caption(&self, value: Option<usize>) -> (&'a str, bool) {
        match value.and_then(|choice| self.choices.get(choice)) {
            Some(caption) => (caption.as_ref(), false),
            None => (self.inherited.unwrap_or(""), self.inherited.is_some()),
        }
    }

    /// The area of the popup of a dropdown in `rect` inside `bounds`: below
    /// the dropdown, or above it when the bounds end first.
    fn popup_area(&self, context: &UiContext, rect: Rect, bounds: Rect) -> Rect {
        let theme = context.theme;
        let entries = self.entries();
        let separator = if self.inherited.is_some() && entries > 1 {
            LIST_PADDING * 2.0 + theme.stroke
        } else {
            0.0
        };
        let shown = entries.clamp(1, POPUP_MAX_ROWS) as f32;
        let height = shown * theme.row_height + separator + LIST_PADDING * 2.0 + theme.stroke * 2.0;
        let offset = theme.gap * 0.5;
        let below = bounds.bottom() - (rect.bottom() + offset);
        let above = rect.y - offset - bounds.y;
        let (top, height) = if height <= below || below >= above {
            (rect.bottom() + offset, height.min(below.max(0.0)))
        } else {
            let height = height.min(above.max(0.0));
            (rect.y - offset - height, height)
        };
        context.snap_rect(Rect::new(rect.x, top, rect.width, height))
    }

    /// Draws the dropdown in `rect` over `value`. Returns whether the value
    /// changed in this pass.
    pub fn show(self, context: &mut UiContext, rect: Rect, value: &mut Option<usize>) -> bool {
        let id = self.id;
        let theme = context.theme;
        let list_id = id.child("popup");
        let before = *value;
        let mut open = context.popup_open(id);
        if open && !self.enabled {
            context.close_popup();
            open = false;
        }
        let mut opened = false;
        let mut pressed = false;
        let mut hovered = false;
        if self.enabled && !open {
            context.focusable(id, rect, NavigationSet::EMPTY);
            let pointer = context.button_behavior(id, rect);
            pressed = pointer.held && pointer.hovered;
            hovered = pointer.hovered || pointer.held;
            if pointer.pressed || context.confirmed(id) {
                context.clear_active(id);
                context.set_focus(id);
                context.open_popup(id);
                let mut list = context.popup_list();
                list.select(self.entry_of(*value));
                context.set_popup_list(list);
                context.set_focus(list_id);
                open = true;
                opened = true;
            }
        } else if self.enabled {
            context.focusable(id, rect, NavigationSet::EMPTY);
            let (x, y) = context.mouse();
            pressed = context.input.left && rect.contains(x, y);
        }

        let (fill, edge, text_color, icon_color) = if !self.enabled {
            (
                theme.colors.control.with_alpha(0.5),
                theme.colors.control_edge.with_alpha(0.5),
                theme.colors.text_disabled,
                theme.colors.text_disabled,
            )
        } else {
            let fill = if pressed {
                theme.colors.control_pressed
            } else if hovered {
                theme.colors.control_hover
            } else {
                theme.colors.control
            };
            let edge = if open {
                theme.colors.accent
            } else {
                theme.colors.control_edge
            };
            (fill, edge, theme.colors.text, theme.colors.text_dim)
        };
        context.rect(
            rect,
            &RectStyle::filled(fill, theme.radius).with_stroke(theme.stroke, edge),
        );
        let chevron = Rect::new(
            rect.right() - theme.gap - theme.icon_size,
            rect.y,
            theme.icon_size,
            rect.height,
        );
        context.icon(chevron, Icon::Down, theme.icon_size, icon_color);
        let left = rect.x + theme.gap * 1.25;
        let caption_area = Rect::new(
            left,
            rect.y,
            (chevron.x - theme.gap - left).max(0.0),
            rect.height,
        );
        let (caption, inherited) = self.caption(*value);
        let caption_color = if inherited && self.enabled {
            theme.colors.text_dim
        } else {
            text_color
        };
        context.text_in(
            caption_area,
            &theme.body().with_color(caption_color),
            caption,
            Align::Start,
            Truncation::End,
        );
        if self.enabled && context.shows_focus(id) {
            context.focus_ring(rect, theme.radius);
        }

        if open {
            let area = self.popup_area(context, rect, context.bounds());
            let current = self.entry_of(*value);
            let mut list_state = context.popup_list();
            let chosen = context.popup(id, area, |context| {
                context.scene.shadow(
                    area,
                    theme.radius,
                    theme.shadow_blur,
                    (0.0, SHADOW_OFFSET),
                    theme.colors.shadow,
                );
                context.rect(
                    area,
                    &RectStyle::filled(theme.colors.panel, theme.radius)
                        .with_stroke(theme.stroke, theme.colors.panel_edge),
                );
                let mut list = List::new(list_id, self.entries()).framed(false);
                if self.inherited.is_some() {
                    list = list.separator_after(0);
                }
                let response = list.show(
                    context,
                    area.inset(theme.stroke),
                    &mut list_state,
                    |context, row| self.draw_entry(context, row, current),
                );
                response.activated.or(response.clicked)
            });
            context.set_popup_list(list_state);
            if !opened && let Some(Some(entry)) = chosen {
                *value = self.value_of(entry);
                context.close_popup();
            }
        }
        *value != before
    }

    /// Draws the popup entry of `row`: its caption and the check of the
    /// entry `current` of the value.
    fn draw_entry(&self, context: &mut UiContext, row: &ListRow, current: usize) {
        let theme = context.theme;
        let inner = Rect::new(
            row.rect.x + theme.gap,
            row.rect.y,
            (row.rect.width - theme.gap * 2.0).max(0.0),
            row.rect.height,
        );
        let (marker, text_area) = inner.cut_right(theme.icon_size);
        if row.index == current {
            context.icon(marker, Icon::Check, theme.icon_size, theme.colors.marker);
        }
        let (caption, inherited) = match self.value_of(row.index) {
            Some(choice) => (self.choices.get(choice).map_or("", AsRef::as_ref), false),
            None => (self.inherited.unwrap_or(""), true),
        };
        let color = if inherited && !row.selected {
            theme.colors.text_dim
        } else {
            theme.colors.text
        };
        context.text_in(
            text_area.cut_right(theme.gap).1,
            &theme.body().with_color(color),
            caption,
            Align::Start,
            Truncation::End,
        );
    }
}

/// A dropdown over `choices` in `rect` with the value `value`, with the
/// behavior of [`Dropdown`]. Returns whether the value changed.
pub fn dropdown(
    context: &mut UiContext,
    id: impl Into<UiId>,
    rect: Rect,
    choices: &[&str],
    value: &mut Option<usize>,
) -> bool {
    Dropdown::new(id, choices).show(context, rect, value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{
        Navigation,
        tests::{Harness, pixel, srgb},
    };

    /// The models of the PC-98.
    const MODELS: [&str; 7] = [
        "PC9801F", "PC9801VM", "PC9801VX", "PC9801RS", "PC9801RA", "PC9821AS", "PC9821AP",
    ];

    /// The area of the dropdown.
    const FIELD: Rect = Rect::new(20.0, 20.0, 280.0, 32.0);

    /// The area of a button after the dropdown.
    const AFTER: Rect = Rect::new(20.0, 120.0, 100.0, 32.0);

    /// Draws the dropdown with the inherited entry over `value` and a
    /// button below it. Returns the change and whether the button was
    /// clicked.
    fn draw(harness: &mut Harness, value: &mut Option<usize>) -> (bool, bool) {
        harness.frame(|context| {
            let changed = Dropdown::new("model", &MODELS)
                .inherited("System default: PC9801RA")
                .show(context, FIELD, value);
            context.focusable(UiId::from("after"), AFTER, NavigationSet::EMPTY);
            let clicked = context.button_behavior(UiId::from("after"), AFTER).clicked;
            (changed, clicked)
        })
    }

    /// The center of the popup entry `entry` below [`FIELD`].
    fn entry_center(harness: &Harness, entry: usize) -> (f32, f32) {
        let theme = &harness.theme;
        let top = FIELD.bottom() + theme.gap * 0.5 + theme.stroke + LIST_PADDING;
        let separator = if entry > 0 {
            LIST_PADDING * 2.0 + theme.stroke
        } else {
            0.0
        };
        (
            FIELD.center().0,
            top + separator + (entry as f32 + 0.5) * theme.row_height,
        )
    }

    #[test]
    fn confirm_opens_the_popup_and_the_keys_pick_an_entry() {
        let mut harness = Harness::new();
        let mut value = Some(4);
        harness.state.set_focus(UiId::from("model"));
        draw(&mut harness, &mut value);
        harness.input.confirm = true;
        draw(&mut harness, &mut value);
        assert!(harness.state.popup_open());
        assert_eq!(
            harness.state.focused(),
            Some(UiId::from("model").child("popup"))
        );
        harness.navigate(Navigation::Up);
        draw(&mut harness, &mut value);
        harness.navigate(Navigation::Up);
        draw(&mut harness, &mut value);
        harness.input.confirm = true;
        assert_eq!(draw(&mut harness, &mut value), (true, false));
        assert_eq!(value, Some(2));
        assert!(!harness.state.popup_open());
        assert_eq!(harness.state.focused(), Some(UiId::from("model")));
        harness.navigate(Navigation::Down);
        draw(&mut harness, &mut value);
        assert_eq!(harness.state.focused(), Some(UiId::from("after")));
    }

    #[test]
    fn the_inherited_entry_picks_none() {
        let mut harness = Harness::new();
        let mut value = Some(0);
        harness.state.set_focus(UiId::from("model"));
        draw(&mut harness, &mut value);
        harness.input.confirm = true;
        draw(&mut harness, &mut value);
        harness.navigate(Navigation::Home);
        draw(&mut harness, &mut value);
        harness.input.confirm = true;
        assert!(draw(&mut harness, &mut value).0);
        assert_eq!(value, None);
    }

    #[test]
    fn cancel_closes_the_popup_without_a_change() {
        let mut harness = Harness::new();
        let mut value = Some(4);
        harness.state.set_focus(UiId::from("model"));
        draw(&mut harness, &mut value);
        harness.input.confirm = true;
        draw(&mut harness, &mut value);
        harness.navigate(Navigation::Down);
        draw(&mut harness, &mut value);
        harness.input.cancel = true;
        let cancelled = harness.frame(|context| {
            Dropdown::new("model", &MODELS)
                .inherited("System default: PC9801RA")
                .show(context, FIELD, &mut value);
            context.cancelled()
        });
        assert!(!cancelled);
        assert_eq!(value, Some(4));
        assert!(!harness.state.popup_open());
        assert_eq!(harness.state.focused(), Some(UiId::from("model")));
    }

    #[test]
    fn a_press_opens_the_popup_and_a_click_on_an_entry_picks_it() {
        let mut harness = Harness::new();
        let mut value = None;
        draw(&mut harness, &mut value);
        harness.press(FIELD.center().0, FIELD.center().1);
        draw(&mut harness, &mut value);
        assert!(harness.state.popup_open());
        let (x, y) = entry_center(&harness, 3);
        harness.mouse(x, y);
        harness.release();
        assert!(!draw(&mut harness, &mut value).0);
        assert!(harness.state.popup_open());
        assert_eq!(value, None);
        harness.press(x, y);
        draw(&mut harness, &mut value);
        harness.release();
        assert!(draw(&mut harness, &mut value).0);
        assert_eq!(value, Some(2));
        assert!(!harness.state.popup_open());
        assert!(harness.state.take_pass_request());
        draw(&mut harness, &mut value);
        let pixels = harness.render();
        assert_eq!(
            pixel(&pixels, harness.viewport.width, FIELD.x + 10.0, y),
            [0; 4]
        );
    }

    #[test]
    fn a_press_on_the_face_shows_the_press_and_the_open_edge() {
        let mut harness = Harness::new();
        let mut value = Some(4);
        draw(&mut harness, &mut value);
        harness.press(FIELD.center().0, FIELD.center().1);
        draw(&mut harness, &mut value);
        draw(&mut harness, &mut value);
        let theme = &harness.theme;
        let pixels = harness.render();
        let width = harness.viewport.width;
        assert_eq!(
            pixel(&pixels, width, FIELD.x + 4.0, FIELD.center().1),
            srgb(theme.colors.control_pressed)
        );
        assert_eq!(
            pixel(&pixels, width, FIELD.x + 40.0, FIELD.y + 0.5),
            srgb(theme.colors.accent)
        );
    }

    #[test]
    fn a_dropdown_with_many_entries_scrolls_to_the_value() {
        let mut harness = Harness::new();
        let choices: Vec<String> = (0..15).map(|index| format!("Choice {index}")).collect();
        let captions: Vec<&str> = choices.iter().map(String::as_str).collect();
        let mut value = Some(12);
        harness.state.set_focus(UiId::from("model"));
        let draw = |harness: &mut Harness, value: &mut Option<usize>| {
            harness.frame(|context| Dropdown::new("model", &captions).show(context, FIELD, value))
        };
        draw(&mut harness, &mut value);
        harness.input.confirm = true;
        draw(&mut harness, &mut value);
        let list = harness
            .state
            .popup
            .map(|popup| popup.list)
            .expect("open popup");
        assert_eq!(list.selected, 12);
        assert!(list.scroll > 0.0);
        for _ in 0..2 {
            harness.navigate(Navigation::Down);
            draw(&mut harness, &mut value);
        }
        harness.input.confirm = true;
        draw(&mut harness, &mut value);
        assert_eq!(value, Some(14));
    }

    #[test]
    fn tab_stays_in_the_open_popup() {
        let mut harness = Harness::new();
        let mut value = Some(1);
        harness.state.set_focus(UiId::from("model"));
        draw(&mut harness, &mut value);
        harness.input.confirm = true;
        draw(&mut harness, &mut value);
        for navigation in [Navigation::Next, Navigation::Previous, Navigation::Left] {
            harness.navigate(navigation);
            draw(&mut harness, &mut value);
            assert_eq!(
                harness.state.focused(),
                Some(UiId::from("model").child("popup"))
            );
        }
        assert!(harness.state.popup_open());
    }

    #[test]
    fn a_dropdown_that_turns_disabled_closes_its_popup() {
        let mut harness = Harness::new();
        let mut value = Some(1);
        harness.state.set_focus(UiId::from("model"));
        draw(&mut harness, &mut value);
        harness.input.confirm = true;
        draw(&mut harness, &mut value);
        harness.frame(|context| {
            Dropdown::new("model", &MODELS)
                .enabled(false)
                .show(context, FIELD, &mut value);
            context.focusable(UiId::from("after"), AFTER, NavigationSet::EMPTY);
        });
        assert!(!harness.state.popup_open());
        assert_ne!(
            harness.state.focused(),
            Some(UiId::from("model").child("popup"))
        );
        harness.navigate(Navigation::Down);
        harness.frame(|context| {
            Dropdown::new("model", &MODELS)
                .enabled(false)
                .show(context, FIELD, &mut value);
            context.focusable(UiId::from("after"), AFTER, NavigationSet::EMPTY);
        });
        assert_eq!(harness.state.focused(), Some(UiId::from("after")));
    }

    #[test]
    fn a_press_outside_closes_the_popup_and_reaches_nothing_below() {
        let mut harness = Harness::new();
        let mut value = Some(1);
        harness.state.set_focus(UiId::from("model"));
        draw(&mut harness, &mut value);
        harness.input.confirm = true;
        draw(&mut harness, &mut value);
        let (x, y) = (AFTER.right() + 400.0, AFTER.center().1);
        harness.press(x, y);
        draw(&mut harness, &mut value);
        harness.release();
        assert_eq!(draw(&mut harness, &mut value), (false, false));
        assert!(!harness.state.popup_open());
        assert_eq!(value, Some(1));
    }

    #[test]
    fn the_popup_opens_above_when_the_target_ends_below() {
        let mut harness = Harness::new();
        let low = Rect::new(20.0, 520.0, 280.0, 32.0);
        let (above, below) = harness.frame(|context| {
            let dropdown = Dropdown::new("model", &MODELS).inherited("System default");
            let bounds = context.bounds();
            (
                dropdown.popup_area(context, low, bounds),
                dropdown.popup_area(context, FIELD, bounds),
            )
        });
        assert!(above.bottom() <= low.y);
        assert!(below.y >= FIELD.bottom());
        let theme = &harness.theme;
        let rows = POPUP_MAX_ROWS as f32;
        let expected = rows * theme.row_height + LIST_PADDING * 4.0 + theme.stroke * 3.0;
        assert_eq!(below.height, expected);
        assert_eq!(below.width, FIELD.width);
    }

    #[test]
    fn the_open_popup_draws_over_the_widgets_below() {
        let mut harness = Harness::new();
        let mut value = Some(4);
        harness.state.set_focus(UiId::from("model"));
        draw(&mut harness, &mut value);
        harness.input.confirm = true;
        harness.frame(|context| {
            Dropdown::new("model", &MODELS)
                .inherited("System default: PC9801RA")
                .show(context, FIELD, &mut value);
            context.fill(Rect::new(0.0, 60.0, 800.0, 300.0), crate::Color::WHITE);
        });
        let pixels = harness.render();
        let width = harness.viewport.width;
        let theme = &harness.theme;
        let (_, highlighted) = entry_center(&harness, 5);
        assert_eq!(
            pixel(&pixels, width, FIELD.x + 10.0, highlighted),
            srgb(theme.colors.selection_focused)
        );
        let (_, plain) = entry_center(&harness, 3);
        assert_eq!(
            pixel(&pixels, width, FIELD.x + 10.0, plain),
            srgb(theme.colors.panel)
        );
        assert_eq!(
            pixel(&pixels, width, FIELD.x + 40.0, FIELD.y + 0.5),
            srgb(theme.colors.accent)
        );
    }

    #[test]
    fn a_disabled_dropdown_leaves_the_ring_and_draws_dimmed() {
        let mut harness = Harness::new();
        let mut value = Some(0);
        harness.navigate(Navigation::Next);
        harness.frame(|context| {
            Dropdown::new("model", &MODELS)
                .enabled(false)
                .show(context, FIELD, &mut value)
        });
        assert_eq!(harness.state.focused(), None);
        harness.input.confirm = true;
        harness.frame(|context| {
            Dropdown::new("model", &MODELS)
                .enabled(false)
                .show(context, FIELD, &mut value)
        });
        assert!(!harness.state.popup_open());
        let pixels = harness.render();
        let inside = pixel(
            &pixels,
            harness.viewport.width,
            FIELD.x + 4.0,
            FIELD.center().1,
        );
        assert!(inside[3] < 255);
    }
}
