//! The single line text field.

use super::{
    context::{PointerResponse, UiContext, UiId},
    field_pointer::point_value,
    field_value::{draw_value, edit_text},
    icons::Icon,
    label::Truncation,
};
use crate::{Color, Rect, RectStyle};

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

/// The state of a field that sets the look of its frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct FieldLook {
    /// The field holds the focus.
    pub(super) focused: bool,
    /// The pointer is over the field.
    pub(super) hovered: bool,
    /// The value of the field is wrong.
    pub(super) error: bool,
    /// The field reacts to input.
    pub(super) enabled: bool,
    /// The field shows a value the user cannot type.
    pub(super) read_only: bool,
}

impl FieldLook {
    /// The color of the value text.
    pub(super) fn text_color(self, context: &UiContext) -> Color {
        let theme = context.theme;
        if !self.enabled {
            theme.colors.text_disabled
        } else if self.read_only {
            theme.colors.text_dim
        } else {
            theme.colors.text
        }
    }
}

/// Draws the frame of a field in `rect` with the look `look`.
pub(super) fn field_frame(context: &mut UiContext, rect: Rect, look: FieldLook) {
    let theme = context.theme;
    let (fill, edge) = if !look.enabled {
        (
            theme.colors.field.with_alpha(0.5),
            theme.colors.control_edge.with_alpha(0.5),
        )
    } else if look.read_only {
        (theme.colors.well, theme.colors.well_edge)
    } else if look.error {
        (theme.colors.field, theme.colors.danger_text)
    } else if look.focused {
        (theme.colors.field, theme.colors.accent)
    } else if look.hovered {
        (
            theme.colors.field,
            theme.colors.control_edge.mix(theme.colors.text_dim, 0.35),
        )
    } else {
        (theme.colors.field, theme.colors.control_edge)
    };
    context.rect(
        rect,
        &RectStyle::filled(fill, theme.radius).with_stroke(theme.stroke, edge),
    );
}

/// A single line text field with options.
///
/// A press gives the field the focus and places the cursor at the position
/// nearest to the pointer. While the button stays held, the pointer extends
/// the selection and a pointer past an edge scrolls the text. A press with
/// the extend modifier extends the selection, and a double click selects the
/// whole text. A press on the clear button places no cursor. A keyboard
/// focus places the cursor at the end of the text. While the field holds the
/// focus, the committed text of the input replaces the selection at the
/// cursor up to [`TEXT_FIELD_LIMIT`] characters. Backspace and delete remove
/// the selection or the character before or after the cursor. Left and right
/// step the cursor by one character, or by one word with the word modifier.
/// Home and end, and left and right with the edge modifier, place it at the
/// start and the end. The extend modifier extends the selection with every
/// step. Select all selects the whole text. Copy and cut write the selection
/// into the clipboard output of the widget state. Cut also removes the
/// selection. A paste inserts the first line of the pasted text like typed
/// text. It drops the control characters. The composition of an input method
/// shows underlined at the cursor. A text wider than the focused field
/// scrolls, so the cursor stays visible. Without the focus a long text is
/// cut. A clear button for the mouse empties a field with text.
///
/// The default field is a search field: it shows the search icon, and
/// cancel empties a field with text and leaves an empty field to the panel.
/// A form field shows another icon or none and leaves cancel to the panel.
#[derive(Clone, Copy, Debug)]
pub struct TextField<'a> {
    /// The id of the field.
    id: UiId,
    /// The text of an empty field.
    placeholder: &'a str,
    /// The icon before the text.
    icon: Option<Icon>,
    /// Whether cancel empties a field with text.
    clears_on_cancel: bool,
    /// Whether the field draws the error edge.
    error: bool,
    /// Whether the field reacts to input.
    enabled: bool,
    /// Whether the field shows a value the user cannot type.
    read_only: bool,
    /// The cut of a long text without the focus.
    truncation: Truncation,
}

impl<'a> TextField<'a> {
    /// A search field that shows `placeholder` while it is empty.
    pub fn new(id: impl Into<UiId>, placeholder: &'a str) -> Self {
        Self {
            id: id.into(),
            placeholder,
            icon: Some(Icon::Search),
            clears_on_cancel: true,
            error: false,
            enabled: true,
            read_only: false,
            truncation: Truncation::End,
        }
    }

    /// The same field with `icon` before the text, or no icon.
    pub fn icon(self, icon: Option<Icon>) -> Self {
        Self { icon, ..self }
    }

    /// The same field, which empties itself on the first cancel when
    /// `clears` is set. A field that does not clear leaves cancel to the
    /// panel.
    pub fn clears_on_cancel(self, clears: bool) -> Self {
        Self {
            clears_on_cancel: clears,
            ..self
        }
    }

    /// The same field with the error edge when `error` is set.
    pub fn error(self, error: bool) -> Self {
        Self { error, ..self }
    }

    /// The same field, disabled unless `enabled`. A disabled field draws
    /// dimmed, ignores input and leaves the focus ring. It takes the pointer
    /// over it, and a press on it reaches no widget after it.
    pub fn enabled(self, enabled: bool) -> Self {
        Self { enabled, ..self }
    }

    /// The same field, read-only when `read_only` is set. A read-only field
    /// shows its text sunken and dimmed, ignores input and leaves the focus
    /// ring. It takes the pointer over it, and a press on it reaches no
    /// widget after it.
    pub fn read_only(self, read_only: bool) -> Self {
        Self { read_only, ..self }
    }

    /// The same field, which cuts a long text without the focus as
    /// `truncation` says. A path field cuts its text in the middle.
    pub fn truncation(self, truncation: Truncation) -> Self {
        Self { truncation, ..self }
    }

    /// Draws the field in `rect` over `text` and returns what it did in this
    /// pass.
    pub fn show(self, context: &mut UiContext, rect: Rect, text: &mut String) -> TextFieldResponse {
        let id = self.id;
        let theme = context.theme;
        let interactive = self.enabled && !self.read_only;
        let mut response = TextFieldResponse::default();
        if interactive && self.clears_on_cancel && context.cancel_taken(id) && !text.is_empty() {
            text.clear();
            response.changed = true;
        }
        let clear_id = id.child("clear");
        let clear_rect =
            Rect::new(rect.right() - rect.height, rect.y, rect.height, rect.height).inset(4.0);
        let mut clear_hot = false;
        if interactive && !text.is_empty() && !context.is_active(id) {
            let pointer = context.button_behavior(clear_id, clear_rect);
            clear_hot = pointer.hovered || pointer.held;
            if pointer.clicked {
                text.clear();
                response.changed = true;
                context.set_focus(id);
            }
        }
        let mut pointer = PointerResponse::default();
        if interactive {
            if context.has_focus(id) {
                let navigation = context.navigation(id);
                response.changed |=
                    edit_text(context, id, text, TEXT_FIELD_LIMIT, navigation, |_| true);
                response.submitted = context.confirmed(id);
            }
            context.text_focusable(id, rect, self.clears_on_cancel && !text.is_empty());
            pointer = context.button_behavior(id, rect);
        } else {
            context.absorb_pointer(rect);
        }
        let focused = interactive && context.has_focus(id);
        let focus_look = interactive && context.shows_focus(id);
        let look = FieldLook {
            focused: focus_look,
            hovered: pointer.hovered,
            error: self.error,
            enabled: self.enabled,
            read_only: self.read_only,
        };
        field_frame(context, rect, look);
        let left = match self.icon {
            Some(icon) => {
                let slot = Rect::new(rect.x + theme.gap, rect.y, theme.icon_size, rect.height);
                let color = if focus_look {
                    theme.colors.text_dim
                } else {
                    theme.colors.text_disabled
                };
                context.icon(slot, icon, theme.icon_size, color);
                slot.right() + theme.gap * 0.75
            }
            None => rect.x + theme.gap * 1.25,
        };
        let shows_clear = interactive && !text.is_empty();
        let right = if shows_clear {
            clear_rect.x - 2.0
        } else {
            rect.right() - theme.gap
        };
        let inner = Rect::new(left, rect.y, (right - left).max(0.0), rect.height);
        if focused {
            point_value(context, id, inner, text, self.truncation, pointer);
        }
        let color = look.text_color(context);
        draw_value(
            context,
            inner,
            text,
            self.placeholder,
            focused.then_some(id),
            color,
            self.truncation,
        );
        if shows_clear {
            if clear_hot {
                context.scene.fill(
                    clear_rect,
                    clear_rect.height * 0.5,
                    theme.colors.control_hover,
                );
            }
            let color = if clear_hot {
                theme.colors.text
            } else {
                theme.colors.text_dim
            };
            context.icon(clear_rect, Icon::Close, CLEAR_ICON_SIZE, color);
        }
        if focus_look {
            context.focus_ring(rect, theme.radius);
        }
        response
    }
}

/// A search field in `rect` that edits `text` and shows `placeholder` while
/// it is empty, with the behavior of [`TextField`].
pub fn text_field(
    context: &mut UiContext,
    id: impl Into<UiId>,
    rect: Rect,
    text: &mut String,
    placeholder: &str,
) -> TextFieldResponse {
    TextField::new(id, placeholder).show(context, rect, text)
}

#[cfg(test)]
mod tests {
    use allocation_counter::count_allocations;

    use super::*;
    use crate::{
        Instance,
        ui::{
            EditCommand, EditModifiers, Navigation, NavigationSet, ScrollState, Shortcut,
            field_pointer::tests::{position_x, press_at},
            field_value::tests::{SHIFT, SHIFT_WORD, WORD, cursor, ends, highlights},
            scroll_area,
            tests::{Harness, pixel, srgb},
        },
    };

    /// The area of the field.
    const FIELD: Rect = Rect::new(10.0, 10.0, 200.0, 32.0);

    /// The area of a button after the field.
    const BUTTON: Rect = Rect::new(10.0, 60.0, 200.0, 32.0);

    /// The area of a button beside the field.
    const BESIDE: Rect = Rect::new(300.0, 10.0, 100.0, 32.0);

    /// The area of a scroll area around the field.
    const SCROLL_AREA: Rect = Rect::new(0.0, 0.0, 250.0, 50.0);

    /// The value area of the search field with text.
    fn value_area(harness: &Harness) -> Rect {
        let theme = &harness.theme;
        let left = FIELD.x + theme.gap + theme.icon_size + theme.gap * 0.75;
        let right = FIELD.right() - FIELD.height + 4.0 - 2.0;
        Rect::new(left, FIELD.y, right - left, FIELD.height)
    }

    /// The x in logical units of the byte offset `offset` of `text` in the
    /// search field with the scroll offset of the last pass.
    fn text_x(harness: &mut Harness, text: &str, offset: usize) -> f32 {
        let theme = &harness.theme;
        let inner_x = FIELD.x + theme.gap + theme.icon_size + theme.gap * 0.75;
        position_x(harness, inner_x, text, offset)
    }

    /// The center of the clear button of the field.
    fn clear_center() -> (f32, f32) {
        Rect::new(
            FIELD.right() - FIELD.height,
            FIELD.y,
            FIELD.height,
            FIELD.height,
        )
        .center()
    }

    /// Releases the left button and draws the field over `text`.
    fn release(harness: &mut Harness, text: &mut String) -> TextFieldResponse {
        harness.release();
        draw(harness, text)
    }

    /// Sends `navigation` with `modifiers` to the field over `text`.
    fn key(
        harness: &mut Harness,
        text: &mut String,
        navigation: Navigation,
        modifiers: EditModifiers,
    ) -> TextFieldResponse {
        harness.navigate_with(navigation, modifiers);
        draw(harness, text)
    }

    /// Types `typed` into the field over `text`.
    fn typing(harness: &mut Harness, text: &mut String, typed: &str) -> TextFieldResponse {
        harness.input.text.push_str(typed);
        draw(harness, text)
    }

    /// A harness with the focus on the field over `text` after one pass.
    fn focused(text: &mut String) -> Harness {
        let mut harness = Harness::new();
        harness.state.set_focus(UiId::from("filter"));
        draw(&mut harness, text);
        harness
    }

    /// Draws the field over `text` and returns its response.
    fn draw(harness: &mut Harness, text: &mut String) -> TextFieldResponse {
        harness.frame(|context| text_field(context, "filter", FIELD, text, "Filter"))
    }

    /// Sends the edit command `command` to the field over `text`.
    fn command(
        harness: &mut Harness,
        text: &mut String,
        command: EditCommand,
    ) -> TextFieldResponse {
        harness.input.edit = Some(command);
        draw(harness, text)
    }

    /// Pastes the clipboard text `pasted` into the field over `text`.
    fn paste(harness: &mut Harness, text: &mut String, pasted: &str) -> TextFieldResponse {
        harness.input.paste.push_str(pasted);
        draw(harness, text)
    }

    /// Draws the field over `text` and returns the shortcut of the pass.
    fn shortcut_pass(harness: &mut Harness, text: &mut String) -> Option<Shortcut> {
        harness.frame(|context| {
            text_field(context, "filter", FIELD, text, "Filter");
            context.shortcut()
        })
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

    /// Verifies that a field that takes the focus in a layer that opens in
    /// a pass gets no text and no backspace of that pass.
    #[test]
    fn a_field_in_a_layer_that_opens_in_the_pass_takes_no_text() {
        let mut harness = Harness::new();
        let mut text = String::from("kept");
        let mut open = false;
        let draw = |harness: &mut Harness, open: bool, text: &mut String| {
            harness.frame(|context| {
                if !open {
                    return TextFieldResponse::default();
                }
                context.layer("dialog", |context| {
                    context.default_focus(UiId::from("filter"));
                    text_field(context, "filter", FIELD, text, "Filter")
                })
            })
        };
        draw(&mut harness, open, &mut text);
        open = true;
        harness.input.text.push_str("typed");
        harness.input.backspace = true;
        assert!(!draw(&mut harness, open, &mut text).changed);
        assert_eq!(text, "kept");
        assert_eq!(harness.state.focused(), Some(UiId::from("filter")));
        harness.input.text.push('!');
        assert!(draw(&mut harness, open, &mut text).changed);
        assert_eq!(text, "kept!");
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
                        NavigationSet::EMPTY,
                    );
                }
            });
        };
        harness.state.set_focus(UiId::from("filter"));
        draw_both(&mut harness, &mut text);
        for navigation in [
            Navigation::Left,
            Navigation::Right,
            Navigation::Home,
            Navigation::End,
        ] {
            harness.navigate(navigation);
            draw_both(&mut harness, &mut text);
            assert_eq!(harness.state.focused(), Some(UiId::from("filter")));
        }
        assert_eq!(text, "adv");
    }

    #[test]
    fn a_form_field_leaves_cancel_to_the_panel() {
        let mut harness = Harness::new();
        let mut text = String::from("Edge");
        let draw = |harness: &mut Harness, text: &mut String| {
            harness.frame(|context| {
                TextField::new("title", "Title")
                    .icon(None)
                    .clears_on_cancel(false)
                    .show(context, FIELD, text);
                context.cancelled()
            })
        };
        harness.state.set_focus(UiId::from("title"));
        draw(&mut harness, &mut text);
        harness.input.cancel = true;
        assert!(draw(&mut harness, &mut text));
        assert_eq!(text, "Edge");
    }

    #[test]
    fn disabled_and_read_only_fields_ignore_input_and_leave_the_ring() {
        for (enabled, read_only) in [(false, false), (true, true)] {
            let mut harness = Harness::new();
            let mut text = String::from("config/pc98/edge.conf");
            let draw = |harness: &mut Harness, text: &mut String| {
                harness.frame(|context| {
                    TextField::new("path", "Not set")
                        .icon(Some(Icon::Folder))
                        .enabled(enabled)
                        .read_only(read_only)
                        .show(context, FIELD, text)
                })
            };
            harness.navigate(Navigation::Next);
            draw(&mut harness, &mut text);
            assert_eq!(harness.state.focused(), None);
            harness.press(50.0, 20.0);
            draw(&mut harness, &mut text);
            harness.release();
            harness.input.text.push('x');
            harness.input.backspace = true;
            assert!(!draw(&mut harness, &mut text).changed);
            assert_eq!(text, "config/pc98/edge.conf");
            assert!(!harness.state.wants_text_input());
        }
    }

    #[test]
    fn the_leading_icon_sets_the_start_of_the_text() {
        let starts: Vec<f32> = [Some(Icon::Search), Some(Icon::Folder), None]
            .into_iter()
            .map(|icon| {
                let mut harness = Harness::new();
                let mut text = String::from("Brandish");
                harness.frame(|context| {
                    TextField::new("field", "Filter")
                        .icon(icon)
                        .show(context, FIELD, &mut text);
                });
                let color = harness.theme.colors.text.to_linear();
                harness
                    .scene
                    .instances()
                    .iter()
                    .filter(|instance| {
                        instance.kind[0] == Instance::KIND_GLYPH && instance.color == color
                    })
                    .map(|instance| instance.rect[0])
                    .fold(f32::MAX, f32::min)
            })
            .collect();
        let theme = Harness::new().theme;
        let after_icon = FIELD.x + theme.gap + theme.icon_size + theme.gap * 0.75;
        assert!(
            starts[0] >= after_icon && starts[0] < after_icon + 2.0,
            "{starts:?}"
        );
        assert_eq!(starts[0], starts[1]);
        let plain = FIELD.x + theme.gap * 1.25;
        assert!(starts[2] >= plain && starts[2] < plain + 2.0, "{starts:?}");
    }

    /// Verifies that a long placeholder takes the cut of the field: a path
    /// field cuts its placeholder in the middle like its text.
    #[test]
    fn a_long_placeholder_takes_the_cut_of_the_field() {
        let long = "Settings default: games/98/YU-NO/YU-NO (Disk 3 of 4) with a very long name.hdi";
        let glyphs = |placeholder: &str, text: &str, truncation: Truncation| {
            let mut harness = Harness::new();
            let mut text = text.to_owned();
            harness.frame(|context| {
                TextField::new("path", placeholder)
                    .icon(Some(Icon::Folder))
                    .read_only(true)
                    .truncation(truncation)
                    .show(context, FIELD, &mut text);
            });
            harness
                .scene
                .instances()
                .iter()
                .filter(|instance| instance.kind[0] == Instance::KIND_GLYPH)
                .map(|instance| instance.rect)
                .collect::<Vec<_>>()
        };
        let placeholder = glyphs(long, "", Truncation::Middle);
        assert!(!placeholder.is_empty());
        assert_eq!(placeholder, glyphs("", long, Truncation::Middle));
        assert_ne!(placeholder, glyphs(long, "", Truncation::End));
        assert_eq!(
            glyphs(long, "", Truncation::End),
            glyphs("", long, Truncation::End)
        );
    }

    #[test]
    fn a_long_text_without_the_focus_ends_inside_the_field() {
        let mut harness = Harness::new();
        let mut text = String::from("games/98/YU-NO/YU-NO (Disk 3 of 4) with a very long name.hdm");
        harness.frame(|context| {
            TextField::new("path", "Not set")
                .icon(Some(Icon::Folder))
                .truncation(Truncation::Middle)
                .show(context, FIELD, &mut text);
        });
        for instance in harness.scene.instances() {
            assert!(
                instance.rect[0] + instance.rect[2] <= FIELD.right(),
                "{instance:?}"
            );
        }
    }

    #[test]
    fn the_edges_of_the_field_follow_its_state() {
        let theme = Harness::new().theme;
        let cases = [
            (false, true, false, theme.colors.danger_text),
            (true, false, false, theme.colors.well_edge),
            (false, false, false, theme.colors.control_edge),
        ];
        for (read_only, error, focused, edge) in cases {
            let mut harness = Harness::new();
            let mut text = String::from("roms/pc88va");
            if focused {
                harness.state.set_focus(UiId::from("field"));
            }
            harness.frame(|context| {
                TextField::new("field", "Not set")
                    .error(error)
                    .read_only(read_only)
                    .show(context, FIELD, &mut text);
            });
            let pixels = harness.render();
            let width = harness.viewport.width;
            assert_eq!(
                pixel(&pixels, width, FIELD.center().0, FIELD.y + 0.5),
                srgb(edge)
            );
        }
        let mut harness = Harness::new();
        let mut text = String::new();
        harness.frame(|context| {
            TextField::new("field", "Not set")
                .read_only(true)
                .show(context, FIELD, &mut text);
            context.focusable(
                UiId::from("other"),
                Rect::new(300.0, 10.0, 50.0, 30.0),
                NavigationSet::EMPTY,
            );
        });
        let pixels = harness.render();
        let fill = pixel(
            &pixels,
            harness.viewport.width,
            FIELD.x + 4.0,
            FIELD.center().1,
        );
        assert_eq!(fill, srgb(harness.theme.colors.well));
    }

    /// Verifies that the edit state follows a text that changes outside the field: the caller,
    /// the clear button and cancel.
    #[test]
    fn the_edit_state_follows_a_change_of_the_text_outside_the_field() {
        let mut text = String::from("abcdef");
        let mut harness = focused(&mut text);
        key(&mut harness, &mut text, Navigation::Left, SHIFT);
        text.truncate(2);
        typing(&mut harness, &mut text, "X");
        assert_eq!(text, "abX");
        harness.input.cancel = true;
        draw(&mut harness, &mut text);
        assert!(text.is_empty());
        typing(&mut harness, &mut text, "Y");
        assert_eq!(text, "Y");
    }

    /// Verifies that a steady frame of a focused field with the cursor inside the text and a
    /// selection allocates nothing.
    #[test]
    fn a_steady_frame_with_a_selection_allocates_nothing() {
        let mut text = String::from("ディスク A and more text");
        let mut harness = focused(&mut text);
        key(&mut harness, &mut text, Navigation::Left, WORD);
        key(&mut harness, &mut text, Navigation::Left, SHIFT_WORD);
        assert!(!highlights(&harness).is_empty());
        draw(&mut harness, &mut text);
        let (allocations, response) = count_allocations(|| draw(&mut harness, &mut text));
        assert!(!response.changed);
        assert_eq!(allocations, 0);
        assert!(!highlights(&harness).is_empty());
    }

    /// Verifies that copy and cut take a selection in both directions, that cut removes it, and
    /// that the state hands out each copied text once.
    #[test]
    fn copy_and_cut_take_a_selection_in_both_directions() {
        let mut text = String::from("ディスク image");
        let mut harness = focused(&mut text);
        key(&mut harness, &mut text, Navigation::Left, SHIFT_WORD);
        let response = command(&mut harness, &mut text, EditCommand::Copy);
        assert!(!response.changed);
        assert_eq!(text, "ディスク image");
        assert_eq!(harness.state.take_copied(), Some("image"));
        assert_eq!(harness.state.take_copied(), None);
        assert!(!highlights(&harness).is_empty());

        key(
            &mut harness,
            &mut text,
            Navigation::Home,
            EditModifiers::default(),
        );
        key(&mut harness, &mut text, Navigation::Right, SHIFT_WORD);
        assert!(command(&mut harness, &mut text, EditCommand::Cut).changed);
        assert_eq!(text, " image");
        assert_eq!(harness.state.take_copied(), Some("ディスク"));
        assert_eq!(harness.state.take_copied(), None);
        assert!(highlights(&harness).is_empty());
        typing(&mut harness, &mut text, "A");
        assert_eq!(text, "A image");
    }

    /// Verifies that copy and cut without a selection change nothing and leave no copied text.
    #[test]
    fn copy_and_cut_without_a_selection_do_nothing() {
        let mut text = String::from("disk");
        let mut harness = focused(&mut text);
        key(
            &mut harness,
            &mut text,
            Navigation::Left,
            EditModifiers::default(),
        );
        for edit in [EditCommand::Copy, EditCommand::Cut] {
            assert!(!command(&mut harness, &mut text, edit).changed);
            assert_eq!(text, "disk");
            assert_eq!(harness.state.take_copied(), None);
        }
        typing(&mut harness, &mut text, "X");
        assert_eq!(text, "disXk");
    }

    /// Verifies that a later copy replaces the text that the state hands out.
    #[test]
    fn a_second_copy_replaces_the_copied_text() {
        let mut text = String::from("first second");
        let mut harness = focused(&mut text);
        key(&mut harness, &mut text, Navigation::Left, SHIFT_WORD);
        command(&mut harness, &mut text, EditCommand::Copy);
        command(&mut harness, &mut text, EditCommand::SelectAll);
        command(&mut harness, &mut text, EditCommand::Copy);
        assert_eq!(harness.state.take_copied(), Some("first second"));
        assert_eq!(harness.state.take_copied(), None);
    }

    /// Verifies that a paste goes to the cursor, replaces the selection, takes only the first
    /// line and drops control characters.
    #[test]
    fn a_paste_takes_the_first_line_without_control_characters() {
        let mut text = String::new();
        let mut harness = focused(&mut text);
        assert!(paste(&mut harness, &mut text, "ディスク").changed);
        assert_eq!(text, "ディスク");
        key(
            &mut harness,
            &mut text,
            Navigation::Left,
            EditModifiers::default(),
        );
        assert!(paste(&mut harness, &mut text, "A\tB\r\nsecond line").changed);
        assert_eq!(text, "ディスABク");
        typing(&mut harness, &mut text, "C");
        assert_eq!(text, "ディスABCク");

        command(&mut harness, &mut text, EditCommand::SelectAll);
        assert!(paste(&mut harness, &mut text, "one\u{2028}two").changed);
        assert_eq!(text, "one");
        key(&mut harness, &mut text, Navigation::Left, SHIFT);
        assert!(paste(&mut harness, &mut text, "\u{7}x\ny").changed);
        assert_eq!(text, "onx");
        assert!(!paste(&mut harness, &mut text, "\nsecond").changed);
        assert!(!paste(&mut harness, &mut text, "\u{1B}\t").changed);
        assert_eq!(text, "onx");
        assert_eq!(harness.state.take_copied(), None);
    }

    /// Verifies that a paste stops at the limit of the field and that a full field takes none.
    #[test]
    fn a_paste_stops_at_the_limit() {
        let mut text = "a".repeat(TEXT_FIELD_LIMIT - 2);
        let mut harness = focused(&mut text);
        assert!(paste(&mut harness, &mut text, "bcd").changed);
        assert_eq!(text.chars().count(), TEXT_FIELD_LIMIT);
        assert!(text.ends_with("abc"));
        assert!(!paste(&mut harness, &mut text, "e").changed);
        key(&mut harness, &mut text, Navigation::Left, SHIFT);
        assert!(paste(&mut harness, &mut text, "漢字").changed);
        assert_eq!(text.chars().count(), TEXT_FIELD_LIMIT);
        assert!(text.ends_with("ab漢"));
    }

    /// Verifies that a paste reaches only the focused field.
    #[test]
    fn a_paste_reaches_only_the_focused_field() {
        let mut harness = Harness::new();
        let mut text = String::new();
        assert!(!paste(&mut harness, &mut text, "ignored").changed);
        assert!(text.is_empty());
    }

    /// Verifies that select all, copy, cut and paste reach no shortcut of the screen and that a
    /// shortcut with the control key works inside the focused field.
    #[test]
    fn edit_commands_reach_no_shortcut() {
        let mut text = String::from("disk");
        let mut harness = focused(&mut text);
        for edit in [EditCommand::SelectAll, EditCommand::Copy, EditCommand::Cut] {
            harness.input.edit = Some(edit);
            assert_eq!(shortcut_pass(&mut harness, &mut text), None);
        }
        assert_eq!(harness.state.take_copied(), Some("disk"));
        harness.input.paste.push('x');
        assert_eq!(shortcut_pass(&mut harness, &mut text), None);
        assert_eq!(text, "x");
        harness.input.shortcut = Some(Shortcut::ControlF);
        assert_eq!(
            shortcut_pass(&mut harness, &mut text),
            Some(Shortcut::ControlF)
        );
    }

    /// Verifies that a press gives an unfocused field the focus and places the cursor at the
    /// pressed position, and that typed text goes there.
    #[test]
    fn a_press_focuses_the_field_and_places_the_cursor() {
        let mut harness = Harness::new();
        let mut text = String::from("disk image");
        draw(&mut harness, &mut text);
        assert_eq!(harness.state.focused(), None);
        let x = text_x(&mut harness, &text, 4) + 1.0;
        press_at(&mut harness, x, FIELD.center().1, |harness| {
            draw(harness, &mut text)
        });
        assert_eq!(harness.state.focused(), Some(UiId::from("filter")));
        assert_eq!(ends(&harness), (4, 4));
        assert!(cursor(&harness).is_some());
        release(&mut harness, &mut text);
        assert!(harness.state.wants_text_input());
        assert!(typing(&mut harness, &mut text, "X").changed);
        assert_eq!(text, "diskX image");
    }

    /// Verifies that a press with the extend modifier extends the selection from the anchor to
    /// the pressed position in both directions.
    #[test]
    fn a_press_with_shift_extends_the_selection() {
        let mut text = String::from("alpha beta gamma");
        let mut harness = focused(&mut text);
        let x = text_x(&mut harness, &text, 6);
        press_at(&mut harness, x, FIELD.center().1, |harness| {
            draw(harness, &mut text)
        });
        release(&mut harness, &mut text);
        assert_eq!(ends(&harness), (6, 6));
        let x = text_x(&mut harness, &text, 10);
        harness.input.modifiers = SHIFT;
        press_at(&mut harness, x, FIELD.center().1, |harness| {
            draw(harness, &mut text)
        });
        assert_eq!(ends(&harness), (6, 10));
        release(&mut harness, &mut text);
        let x = text_x(&mut harness, &text, 0);
        harness.input.modifiers = SHIFT;
        press_at(&mut harness, x, FIELD.center().1, |harness| {
            draw(harness, &mut text)
        });
        release(&mut harness, &mut text);
        assert_eq!(ends(&harness), (6, 0));
        assert_eq!(highlights(&harness).len(), 1);
        typing(&mut harness, &mut text, "X");
        assert_eq!(text, "Xbeta gamma");
    }

    /// Verifies that a double click selects the whole text and that the held button after it
    /// keeps the selection.
    #[test]
    fn a_double_click_selects_the_whole_text() {
        let mut text = String::from("ディスク image");
        let mut harness = focused(&mut text);
        let x = text_x(&mut harness, &text, 3);
        press_at(&mut harness, x, FIELD.center().1, |harness| {
            draw(harness, &mut text)
        });
        release(&mut harness, &mut text);
        assert_eq!(ends(&harness), (3, 3));
        harness.input.left_double = true;
        press_at(&mut harness, x, FIELD.center().1, |harness| {
            draw(harness, &mut text)
        });
        assert_eq!(ends(&harness), (0, text.len()));
        let x = text_x(&mut harness, &text, 9);
        harness.mouse(x, FIELD.center().1);
        draw(&mut harness, &mut text);
        assert_eq!(ends(&harness), (0, text.len()));
        release(&mut harness, &mut text);
        assert_eq!(ends(&harness), (0, text.len()));
        typing(&mut harness, &mut text, "A");
        assert_eq!(text, "A");
    }

    /// Verifies that a press on the clear button places no cursor and starts no drag, and that
    /// its click still empties the field.
    #[test]
    fn a_press_on_the_clear_button_places_no_cursor() {
        let mut text = String::from("disk image");
        let mut harness = focused(&mut text);
        key(
            &mut harness,
            &mut text,
            Navigation::Home,
            EditModifiers::default(),
        );
        let (x, y) = clear_center();
        harness.press(x, y);
        draw(&mut harness, &mut text);
        assert_eq!(ends(&harness), (0, 0));
        let inside = text_x(&mut harness, &text, 4);
        harness.mouse(inside, y);
        draw(&mut harness, &mut text);
        assert_eq!(ends(&harness), (0, 0));
        assert!(highlights(&harness).is_empty());
        harness.mouse(x, y);
        draw(&mut harness, &mut text);
        assert!(release(&mut harness, &mut text).changed);
        assert!(text.is_empty());
        assert_eq!(harness.state.focused(), Some(UiId::from("filter")));
    }

    /// Verifies that a drag from the text reaches neither the clear button nor a button after
    /// the field: no hover, no click and no clear on the release over them.
    #[test]
    fn a_drag_reaches_no_other_widget() {
        let mut text = String::from("disk image");
        let mut harness = Harness::new();
        let button = UiId::from("button");
        let draw_both = |harness: &mut Harness, text: &mut String| {
            harness.frame(|context| {
                let response = text_field(context, "filter", FIELD, text, "Filter");
                (response, context.button_behavior(button, BUTTON))
            })
        };
        draw_both(&mut harness, &mut text);
        let x = text_x(&mut harness, &text, 2);
        harness.press(x, FIELD.center().1);
        draw_both(&mut harness, &mut text);
        assert_eq!(ends(&harness), (2, 2));
        harness.mouse(BUTTON.center().0, BUTTON.center().1);
        let (_, over_button) = draw_both(&mut harness, &mut text);
        assert!(!over_button.hovered);
        assert_eq!(ends(&harness), (2, text.len()));
        harness.release();
        let (_, released) = draw_both(&mut harness, &mut text);
        assert!(!released.clicked);

        let x = text_x(&mut harness, &text, 2);
        harness.press(x, FIELD.center().1);
        draw_both(&mut harness, &mut text);
        let (x, y) = clear_center();
        harness.mouse(x, y);
        draw_both(&mut harness, &mut text);
        harness.release();
        let (response, _) = draw_both(&mut harness, &mut text);
        assert!(!response.changed);
        assert_eq!(text, "disk image");
        assert_eq!(ends(&harness), (2, text.len()));
    }

    /// Verifies that a steady frame during a drag without pointer motion allocates nothing and
    /// keeps the selection.
    #[test]
    fn a_steady_frame_during_a_drag_allocates_nothing() {
        let mut text = String::from("ディスク A and more text");
        let mut harness = focused(&mut text);
        key(
            &mut harness,
            &mut text,
            Navigation::Home,
            EditModifiers::default(),
        );
        let x = text_x(&mut harness, &text, 3);
        press_at(&mut harness, x, FIELD.center().1, |harness| {
            draw(harness, &mut text)
        });
        let x = text_x(&mut harness, &text, 9);
        harness.mouse(x, FIELD.center().1);
        draw(&mut harness, &mut text);
        draw(&mut harness, &mut text);
        assert_eq!(ends(&harness), (3, 9));
        let (allocations, response) = count_allocations(|| draw(&mut harness, &mut text));
        assert!(!response.changed);
        assert_eq!(allocations, 0);
        assert_eq!(ends(&harness), (3, 9));
        assert!(!highlights(&harness).is_empty());
    }

    /// Verifies that a press on the middle cut text of a field without the focus places the
    /// cursor at the pressed character of the start and of the end part, and that the focused
    /// field draws the cursor under the pointer.
    #[test]
    fn a_press_on_a_middle_cut_text_places_the_cursor_at_the_pressed_character() {
        let path = "/home/user/games/Dragon Slayer/ドラゴンスレイヤー英雄伝説 (ディスク1).hdm";
        let draw_path = |harness: &mut Harness, text: &mut String| {
            harness.frame(|context| {
                TextField::new("path", "Path")
                    .truncation(Truncation::Middle)
                    .show(context, FIELD, text)
            })
        };
        for from_end in [false, true] {
            let mut harness = Harness::new();
            let mut text = String::from(path);
            draw_path(&mut harness, &mut text);
            let inner = value_area(&harness);
            let style = harness.theme.body();
            let run = harness
                .fonts
                .fit_middle(style.font, style.size, &text, inner.width);
            assert!(run.truncated);
            let index = if from_end { run.glyphs.len() - 4 } else { 3 };
            let cluster = run.glyphs[index].cluster;
            let x = inner.x.round() + run.glyphs[index].x * style.size + 0.5;
            press_at(&mut harness, x, FIELD.center().1, |harness| {
                draw_path(harness, &mut text)
            });
            assert_eq!(harness.state.focused(), Some(UiId::from("path")));
            assert_eq!(ends(&harness), (cluster, cluster), "{from_end}");
            let drawn = cursor(&harness).unwrap();
            assert!((drawn[0] - x).abs() <= 2.0, "{from_end} {drawn:?} {x}");
            harness.release();
            draw_path(&mut harness, &mut text);
            assert_eq!(ends(&harness), (cluster, cluster));
            assert!((cursor(&harness).unwrap()[0] - drawn[0]).abs() < 0.01);
        }
    }

    /// Verifies that a press on the right half of the glyph before a glyph of a middle cut text
    /// of a field without the focus keeps the drawn text: a press in the start part keeps the
    /// scroll at zero, and a press in the end part keeps the glyph at its drawn x.
    #[test]
    fn a_press_beside_a_glyph_of_a_cut_text_keeps_the_glyph_at_its_drawn_x() {
        let path = "/home/user/games/Dragon Slayer/ドラゴンスレイヤー英雄伝説 (ディスク1).hdm";
        let draw_path = |harness: &mut Harness, text: &mut String| {
            harness.frame(|context| {
                TextField::new("path", "Path")
                    .truncation(Truncation::Middle)
                    .show(context, FIELD, text)
            })
        };
        for from_end in [false, true] {
            let mut harness = Harness::new();
            let mut text = String::from(path);
            draw_path(&mut harness, &mut text);
            let inner = value_area(&harness);
            let style = harness.theme.body();
            let run = harness
                .fonts
                .fit_middle(style.font, style.size, &text, inner.width);
            assert!(run.truncated);
            let index = if from_end { run.glyphs.len() - 2 } else { 3 };
            let cluster = run.glyphs[index].cluster;
            let glyph_x = inner.x.round() + run.glyphs[index].x * style.size;
            press_at(&mut harness, glyph_x - 2.0, FIELD.center().1, |harness| {
                draw_path(harness, &mut text)
            });
            assert_eq!(ends(&harness), (cluster, cluster), "{from_end}");
            if from_end {
                let drawn = cursor(&harness).unwrap();
                assert!((drawn[0] - glyph_x).abs() < 0.5, "{drawn:?} {glyph_x}");
            } else {
                assert_eq!(harness.state.text_scroll, 0.0);
            }
        }
    }

    /// Verifies that a drag from the text reaches no widget before the field and no widget after
    /// a scroll area around it: no hover during the drag and no click on the release. The widget
    /// before the field shows the hover in the pass of the release, and the widget after the
    /// scroll area in the pass after it.
    #[test]
    fn a_drag_reaches_no_widget_before_the_field_or_after_its_scroll_area() {
        let mut text = String::from("disk image");
        let mut harness = Harness::new();
        let mut scroll = ScrollState::default();
        let mut draw_all = |harness: &mut Harness, text: &mut String| {
            harness.frame(|context| {
                let beside = context.button_behavior(UiId::from("beside"), BESIDE);
                scroll_area(context, "area", SCROLL_AREA, &mut scroll, |context, _| {
                    text_field(context, "filter", FIELD, text, "Filter");
                    SCROLL_AREA.height
                });
                let after = context.button_behavior(UiId::from("after"), BUTTON);
                (beside, after)
            })
        };
        draw_all(&mut harness, &mut text);
        let x = text_x(&mut harness, &text, 2);
        press_at(&mut harness, x, FIELD.center().1, |harness| {
            draw_all(harness, &mut text)
        });
        assert_eq!(ends(&harness), (2, 2));
        for area in [BESIDE, BUTTON] {
            let (x, y) = area.center();
            harness.mouse(x, y);
            let (beside, after) = draw_all(&mut harness, &mut text);
            assert!(!beside.hovered && !after.hovered, "{area:?}");
        }
        let (x, y) = BESIDE.center();
        harness.mouse(x, y);
        harness.release();
        let (beside, after) = draw_all(&mut harness, &mut text);
        assert!(beside.hovered);
        assert!(!beside.clicked && !after.clicked);
        let (x, y) = BUTTON.center();
        harness.mouse(x, y);
        let (_, after) = draw_all(&mut harness, &mut text);
        assert!(after.hovered);
    }
}
