//! The confirm dialog: a modal with a wrapping message, a cancel button and
//! the button of the action.

use super::{
    button::{Button, ButtonKind},
    context::{UiContext, UiId},
    icons::Icon,
    label::{Align, wrapped_height, wrapped_label},
    modal::{MODAL_HEADER_HEIGHT, modal_in},
};
use crate::Rect;

/// The width of a confirm dialog.
pub const DIALOG_WIDTH: f32 = 480.0;

/// The most lines of the message and of the detail of a confirm dialog.
const MAXIMUM_LINES: usize = 6;

/// What a [`ConfirmDialog`] did in a pass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConfirmResponse {
    /// The action button was activated.
    pub confirmed: bool,
    /// The cancel button, the close button or a cancel asked to close the
    /// dialog without the action.
    pub cancelled: bool,
}

/// A modal that asks before an action: a title, a wrapping message, an
/// optional detail in small text, a cancel button and the action button.
///
/// A destructive action takes the danger look, and the focus starts on the
/// cancel button. Any other action takes the primary look, and the focus
/// starts on the action. The dialog draws in its own modal layer, see
/// [`UiContext::layer`]. Either answer closes the layer, and the caller
/// stops drawing the dialog.
#[derive(Clone, Copy, Debug)]
pub struct ConfirmDialog<'a> {
    /// The id of the dialog.
    id: UiId,
    /// The title of the panel.
    title: &'a str,
    /// The question.
    message: &'a str,
    /// The small text below the question.
    detail: Option<&'a str>,
    /// The caption of the cancel button.
    cancel: &'a str,
    /// The caption of the action button.
    action: &'a str,
    /// The icon of the action button.
    action_icon: Option<Icon>,
    /// Whether the action is destructive.
    danger: bool,
    /// The area the dialog centers in.
    area: Option<Rect>,
}

impl<'a> ConfirmDialog<'a> {
    /// A dialog with the title `title` that asks `message` before the
    /// action `action`.
    pub fn new(id: impl Into<UiId>, title: &'a str, message: &'a str, action: &'a str) -> Self {
        Self {
            id: id.into(),
            title,
            message,
            detail: None,
            cancel: "Cancel",
            action,
            action_icon: None,
            danger: false,
            area: None,
        }
    }

    /// The same dialog with the small text `detail` below the message.
    pub fn detail(self, detail: &'a str) -> Self {
        Self {
            detail: Some(detail),
            ..self
        }
    }

    /// The same dialog with the caption `caption` on the cancel button.
    pub fn cancel_caption(self, caption: &'a str) -> Self {
        Self {
            cancel: caption,
            ..self
        }
    }

    /// The same dialog with `icon` on the action button.
    pub fn action_icon(self, icon: Icon) -> Self {
        Self {
            action_icon: Some(icon),
            ..self
        }
    }

    /// The same dialog for a destructive action when `danger` is set.
    pub fn danger(self, danger: bool) -> Self {
        Self { danger, ..self }
    }

    /// The same dialog, centered in `area`, such as the area above a status
    /// line.
    pub fn area(self, area: Rect) -> Self {
        Self {
            area: Some(area),
            ..self
        }
    }

    /// The id of the cancel button.
    pub fn cancel_id(&self) -> UiId {
        self.id.child("cancel")
    }

    /// The id of the action button.
    pub fn action_id(&self) -> UiId {
        self.id.child("action")
    }

    /// Draws one pass of the dialog and returns the answer of the pass.
    pub fn show(self, context: &mut UiContext) -> ConfirmResponse {
        context.layer(self.id, |context| {
            let response = self.panel(context);
            if response.confirmed || response.cancelled {
                context.close_layer();
            }
            response
        })
    }

    /// Draws the panel of the dialog in the current layer.
    fn panel(self, context: &mut UiContext) -> ConfirmResponse {
        let theme = context.theme;
        let width = DIALOG_WIDTH - theme.padding * 2.0;
        let message_style = theme.body();
        let detail_style = theme.small();
        let message_height =
            wrapped_height(context, &message_style, self.message, width, MAXIMUM_LINES);
        let detail_height = match self.detail {
            Some(detail) => {
                theme.gap + wrapped_height(context, &detail_style, detail, width, MAXIMUM_LINES)
            }
            None => 0.0,
        };
        let top = theme.gap * 0.25;
        let height = MODAL_HEADER_HEIGHT
            + top
            + message_height
            + detail_height
            + theme.gap * 2.0
            + theme.control_height
            + theme.padding;
        let area = self.area.unwrap_or_else(|| context.bounds());
        let modal = modal_in(context, self.id, area, (DIALOG_WIDTH, height), self.title);
        let content = modal.content;
        let message_area = Rect::new(content.x, content.y + top, content.width, message_height);
        wrapped_label(
            context,
            message_area,
            &message_style,
            self.message,
            Align::Start,
            MAXIMUM_LINES,
        );
        if let Some(detail) = self.detail {
            let detail_area = Rect::new(
                content.x,
                message_area.bottom() + theme.gap,
                content.width,
                detail_height,
            );
            wrapped_label(
                context,
                detail_area,
                &detail_style,
                detail,
                Align::Start,
                MAXIMUM_LINES,
            );
        }

        let kind = if self.danger {
            ButtonKind::Danger
        } else {
            ButtonKind::Primary
        };
        let mut action = Button::new(self.action_id(), self.action).kind(kind);
        if let Some(icon) = self.action_icon {
            action = action.icon(icon);
        }
        let cancel = Button::new(self.cancel_id(), self.cancel);
        let footer = content.cut_bottom(theme.control_height).0;
        let action_width = action.width(context);
        let cancel_width = cancel.width(context);
        let (action_area, rest) = footer.cut_right(action_width);
        let cancel_area = rest.cut_right(theme.gap).1.cut_right(cancel_width).0;
        context.default_focus(if self.danger {
            self.cancel_id()
        } else {
            self.action_id()
        });
        let cancelled = cancel.show(context, cancel_area);
        let confirmed = action.show(context, action_area);
        ConfirmResponse {
            confirmed,
            cancelled: (modal.closed || cancelled) && !confirmed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{
        Navigation, NavigationSet,
        tests::{Harness, pixel, srgb},
    };

    /// The delete question of the library.
    fn delete() -> ConfirmDialog<'static> {
        ConfirmDialog::new(
            "delete",
            "Delete game",
            "Delete YU-NO: A Girl Who Chants Love at the Bound of this World?",
            "Delete",
        )
        .detail("The file config/pc98/yu-no.conf is removed. The disk images stay where they are.")
        .action_icon(Icon::Delete)
        .danger(true)
    }

    /// Draws a button below and the dialog above it.
    fn draw(harness: &mut Harness, dialog: ConfirmDialog) -> ConfirmResponse {
        harness.frame(|context| {
            context.focusable(
                UiId::from("below"),
                Rect::new(0.0, 0.0, 100.0, 30.0),
                NavigationSet::EMPTY,
            );
            dialog.show(context)
        })
    }

    #[test]
    fn a_destructive_action_starts_on_cancel() {
        let mut harness = Harness::new();
        harness.state.set_focus(UiId::from("below"));
        draw(&mut harness, delete());
        assert_eq!(harness.state.focused(), Some(delete().cancel_id()));
        harness.navigate(Navigation::Right);
        draw(&mut harness, delete());
        assert_eq!(harness.state.focused(), Some(delete().action_id()));
        harness.input.confirm = true;
        let response = draw(&mut harness, delete());
        assert!(response.confirmed && !response.cancelled);
        assert_eq!(harness.state.open_modals(), 0);
        assert_eq!(harness.state.focused(), Some(UiId::from("below")));
    }

    #[test]
    fn a_confirm_leaves_no_dialog_in_the_next_pass() {
        let mut harness = Harness::new();
        let save = ConfirmDialog::new("save", "Unsaved changes", "Save the changes?", "Save");
        let mut open = true;
        let draw = |harness: &mut Harness, open: &mut bool| {
            harness.frame(|context| {
                if *open && save.show(context).confirmed {
                    *open = false;
                }
            });
        };
        draw(&mut harness, &mut open);
        harness.state.take_pass_request();
        harness.input.confirm = true;
        draw(&mut harness, &mut open);
        assert!(!open);
        assert!(harness.state.take_pass_request());
        draw(&mut harness, &mut open);
        assert!(harness.scene.is_empty());
        assert_eq!(harness.state.open_modals(), 0);
    }

    #[test]
    fn cancel_on_the_cancel_button_answers_cancel() {
        let mut harness = Harness::new();
        draw(&mut harness, delete());
        harness.input.confirm = true;
        let response = draw(&mut harness, delete());
        assert!(response.cancelled && !response.confirmed);
    }

    #[test]
    fn a_primary_action_starts_on_the_action() {
        let mut harness = Harness::new();
        let save = ConfirmDialog::new("save", "Unsaved changes", "Save the changes?", "Save");
        draw(&mut harness, save);
        assert_eq!(harness.state.focused(), Some(save.action_id()));
        harness.input.cancel = true;
        assert!(draw(&mut harness, save).cancelled);
    }

    #[test]
    fn the_dialog_wraps_its_message_inside_the_panel() {
        let mut harness = Harness::new();
        draw(&mut harness, delete());
        let theme = &harness.theme;
        let left = (800.0 - DIALOG_WIDTH) * 0.5 + theme.padding;
        for instance in harness.scene.instances() {
            if instance.kind[0] == crate::Instance::KIND_GLYPH {
                assert!(instance.rect[0] >= left - 1.0, "{instance:?}");
                assert!(
                    instance.rect[0] + instance.rect[2] <= left + DIALOG_WIDTH - theme.padding,
                    "{instance:?}"
                );
            }
        }
        let pixels = harness.render();
        let width = harness.viewport.width;
        let action = harness
            .scene
            .instances()
            .iter()
            .find(|instance| instance.color == theme.danger.to_linear())
            .expect("the danger button");
        let [x, y, _, height] = action.rect;
        assert_eq!(
            pixel(&pixels, width, x + 3.0, y + height * 0.5),
            srgb(theme.danger)
        );
    }
}
