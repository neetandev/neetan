//! The error dialogs of the manager: a launch that failed, a game that
//! stopped after a panic of its core, disk images that were not saved and a
//! game file that was not deleted.

use common::FlushError;
use gui::{
    Rect, RectStyle,
    ui::{
        Align, Button, ButtonKind, HEADER_HEIGHT, UiContext, UiId, modal_in, wrapped_height,
        wrapped_label,
    },
};

/// The width of the dialog panel in logical units.
const DIALOG_WIDTH: f32 = 560.0;

/// The most lines of the message. Longer messages end after them.
const MAXIMUM_MESSAGE_LINES: usize = 4;

/// The most lines of the detail text. Longer texts end after them.
const MAXIMUM_DETAIL_LINES: usize = 8;

/// The id of the OK button.
const OK_ID: &str = "error ok";

/// The id of the modal layer of the dialog.
pub(super) const LAYER_ID: &str = "error";

/// The title of the dialog of a launch that failed.
pub(super) const LAUNCH_FAILED_TITLE: &str = "Could not start the game";

/// The title of the dialog of a game that stopped after a panic.
pub(super) const GAME_STOPPED_TITLE: &str = "The game stopped";

/// The title of the dialog of disk images that were not saved.
pub(super) const MEDIA_NOT_SAVED_TITLE: &str = "The disk images were not saved";

/// The title of the dialog of a game file that was not deleted.
pub(super) const DELETE_FAILED_TITLE: &str = "Could not delete the game";

/// An open error dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ErrorDialog {
    /// The title of the panel.
    title: &'static str,
    /// The sentence about what happened.
    message: String,
    /// The error text.
    detail: String,
}

impl ErrorDialog {
    /// The dialog of the game `game_title` that did not start with `error`.
    pub(crate) fn launch_failed(game_title: &str, error: &str) -> Self {
        Self {
            title: LAUNCH_FAILED_TITLE,
            message: format!("{game_title} did not start."),
            detail: error.to_owned(),
        }
    }

    /// The dialog of a game of `system_label` whose core panicked with
    /// `panic_message`. `flush` is the result of the write back of the
    /// disk images after the panic. Its errors come before the panic
    /// message, so the line limit of the detail cuts the panic message first.
    pub(crate) fn game_stopped(
        system_label: &str,
        panic_message: &str,
        flush: &Result<(), FlushError>,
    ) -> Self {
        let mut message = format!("The {system_label} core stopped with an error.");
        let detail = match flush {
            Ok(()) => {
                message.push_str(" The disk images were saved.");
                panic_message.to_owned()
            }
            Err(error) => {
                message.push_str(" Some disk images were not saved.");
                format!("{error}\n{panic_message}")
            }
        };
        Self {
            title: GAME_STOPPED_TITLE,
            message,
            detail,
        }
    }

    /// The dialog of the game `game_title` that ended while the write back
    /// of its disk images failed with `error`.
    pub(crate) fn media_not_saved(game_title: &str, error: &FlushError) -> Self {
        Self {
            title: MEDIA_NOT_SAVED_TITLE,
            message: format!("{game_title} ended. Some of its disk images were not saved."),
            detail: error.to_string(),
        }
    }

    /// The dialog of the game `name` whose file was not removed with `error`.
    pub(crate) fn delete_failed(name: &str, error: &str) -> Self {
        Self {
            title: DELETE_FAILED_TITLE,
            message: format!("{name} was not deleted."),
            detail: error.to_owned(),
        }
    }

    /// The title of the panel.
    #[cfg(test)]
    pub(crate) fn title(&self) -> &str {
        self.title
    }

    /// The sentence about what happened.
    #[cfg(test)]
    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    /// The error text.
    #[cfg(test)]
    pub(crate) fn detail(&self) -> &str {
        &self.detail
    }

    /// Draws one pass of the dialog in its modal layer, centered in `area`.
    /// The OK button holds the focus. Returns whether the dialog closes, and
    /// closes the layer then.
    pub(crate) fn show(&self, context: &mut UiContext, area: Rect) -> bool {
        context.layer(LAYER_ID, |context| {
            let closed = self.panel(context, area);
            if closed {
                context.close_layer();
            }
            closed
        })
    }

    /// Draws the panel of the dialog centered in `area`. Returns whether the
    /// close button, OK or cancel closed it.
    fn panel(&self, context: &mut UiContext, area: Rect) -> bool {
        let theme = context.theme;
        let ok_id = UiId::from(OK_ID);
        let message_style = theme.body();
        let detail_style = theme.small().with_color(theme.text);
        let detail_padding = (theme.gap * 1.25, theme.gap);
        let message_width = DIALOG_WIDTH - theme.padding * 2.0;
        let detail_width = message_width - detail_padding.0 * 2.0;
        let message_height = wrapped_height(
            context,
            &message_style,
            &self.message,
            message_width,
            MAXIMUM_MESSAGE_LINES,
        );
        let detail_height = wrapped_height(
            context,
            &detail_style,
            &self.detail,
            detail_width,
            MAXIMUM_DETAIL_LINES,
        ) + detail_padding.1 * 2.0;
        let height = HEADER_HEIGHT
            + message_height
            + theme.gap
            + detail_height
            + theme.gap * 2.0
            + theme.control_height
            + theme.padding;
        let response = modal_in(context, LAYER_ID, area, (DIALOG_WIDTH, height), self.title);

        let (message_area, rest) = response.content.cut_top(message_height);
        wrapped_label(
            context,
            message_area,
            &message_style,
            &self.message,
            Align::Start,
            MAXIMUM_MESSAGE_LINES,
        );

        let (detail_area, rest) = rest.cut_top(theme.gap).1.cut_top(detail_height);
        context.rect(
            detail_area,
            &RectStyle::filled(theme.well, theme.radius).with_stroke(theme.stroke, theme.well_edge),
        );
        wrapped_label(
            context,
            Rect::new(
                detail_area.x + detail_padding.0,
                detail_area.y + detail_padding.1,
                detail_width,
                detail_height - detail_padding.1 * 2.0,
            ),
            &detail_style,
            &self.detail,
            Align::Start,
            MAXIMUM_DETAIL_LINES,
        );

        let footer = rest.cut_bottom(theme.control_height).0;
        let ok = Button::new(ok_id, "OK").kind(ButtonKind::Primary);
        let ok_area = footer.cut_right(ok.width(context)).0;
        let ok = ok.show(context, ok_area);
        response.closed || ok
    }
}

#[cfg(test)]
mod tests {
    use gui::ui::Navigation;

    use super::*;
    use crate::gui_test_support::GuiHarness;

    /// Verifies the title, the message and the detail of the launch error dialog.
    #[test]
    fn the_launch_dialog_shows_the_game_and_the_error() {
        let dialog = ErrorDialog::launch_failed("Edge", "fdd1 image not found: /games/edge.d88");
        assert_eq!(dialog.title(), LAUNCH_FAILED_TITLE);
        assert_eq!(dialog.message(), "Edge did not start.");
        assert_eq!(dialog.detail(), "fdd1 image not found: /games/edge.d88");
    }

    /// Verifies that the panic dialog mentions the saved disk images only after a flush without a panic.
    #[test]
    fn the_panic_dialog_mentions_the_saved_images_only_after_a_good_flush() {
        let saved = ErrorDialog::game_stopped("PC-98", "sample panic", &Ok(()));
        assert_eq!(saved.title(), GAME_STOPPED_TITLE);
        assert_eq!(
            saved.message(),
            "The PC-98 core stopped with an error. The disk images were saved."
        );
        assert_eq!(saved.detail(), "sample panic");
        let lost = ErrorDialog::game_stopped(
            "PC-98",
            "sample panic",
            &Err(FlushError::new("floppy image /a.d88 could not be written")),
        );
        assert_eq!(
            lost.message(),
            "The PC-98 core stopped with an error. Some disk images were not saved."
        );
        assert_eq!(
            lost.detail(),
            "floppy image /a.d88 could not be written\nsample panic"
        );
    }

    /// Verifies the dialog of disk images that were not saved after a normal end of the game.
    #[test]
    fn the_media_dialog_lists_the_images_that_were_not_saved() {
        let error = FlushError {
            messages: vec!["first".to_owned(), "second".to_owned()],
        };
        let dialog = ErrorDialog::media_not_saved("Edge", &error);
        assert_eq!(dialog.title(), MEDIA_NOT_SAVED_TITLE);
        assert_eq!(
            dialog.message(),
            "Edge ended. Some of its disk images were not saved."
        );
        assert_eq!(dialog.detail(), "first\nsecond");
    }

    /// Verifies the dialog of a game file that was not deleted.
    #[test]
    fn the_delete_dialog_shows_the_game_and_the_error() {
        let dialog = ErrorDialog::delete_failed("Edge", "config/pc98/edge.conf: Permission denied");
        assert_eq!(dialog.title(), DELETE_FAILED_TITLE);
        assert_eq!(dialog.message(), "Edge was not deleted.");
        assert_eq!(dialog.detail(), "config/pc98/edge.conf: Permission denied");
    }

    /// Verifies that OK holds the focus, that confirm and cancel close the dialog and that navigation does not.
    #[test]
    fn ok_holds_the_focus_and_confirm_or_cancel_close_the_dialog() {
        let mut harness = GuiHarness::new(960, 600);
        let area = Rect::new(0.0, 0.0, 960.0, 600.0);
        let dialog = ErrorDialog::launch_failed("Edge", "error");
        assert!(!harness.frame(|context| dialog.show(context, area)));
        assert_eq!(harness.state.focused(), Some(UiId::from(OK_ID)));
        harness.input.confirm = true;
        assert!(harness.frame(|context| dialog.show(context, area)));

        let dialog = ErrorDialog::game_stopped("X1", "halt", &Ok(()));
        harness.frame(|context| dialog.show(context, area));
        harness.input.cancel = true;
        assert!(harness.frame(|context| dialog.show(context, area)));

        let dialog = ErrorDialog::launch_failed("Edge", "error");
        harness.frame(|context| dialog.show(context, area));
        harness.input.navigation = Some(Navigation::Down);
        assert!(!harness.frame(|context| dialog.show(context, area)));
    }

    /// Verifies that the panel centers in the area it receives and keeps the margin to its edges.
    #[test]
    fn the_dialog_centers_in_its_area() {
        let mut harness = GuiHarness::new(960, 600);
        let area = Rect::new(0.0, 0.0, 960.0, 572.0);
        let dialog = ErrorDialog::launch_failed("Edge", &"error text ".repeat(60));
        harness.frame(|context| dialog.show(context, area));
        let panel = harness
            .scene
            .instances()
            .iter()
            .find(|instance| {
                instance.kind[0] == gui::Instance::KIND_RECTANGLE
                    && instance.rect[2] == DIALOG_WIDTH
            })
            .expect("the panel")
            .rect;
        let top = panel[1];
        let bottom = panel[1] + panel[3];
        assert!((top - (572.0 - bottom)).abs() <= 1.0, "{panel:?}");
        assert!(bottom <= 572.0);
    }
}
