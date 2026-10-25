//! The error dialogs of the manager: a launch that failed, a game that
//! stopped after a panic of its core, disk images that were not saved, a
//! game file that was not deleted, a file of the editor that did not open
//! or did not save, a file of the editor that changed outside the manager
//! before a save, and a saved settings file that does not read.

use std::path::PathBuf;

use common::FlushError;
use gui::{
    Rect, RectStyle,
    ui::{
        ACTIVATE_KEYS, Align, Button, ButtonKind, CANCEL_KEYS, HEADER_HEIGHT, Hint, Icon,
        UiContext, UiId, modal_in, wrapped_height, wrapped_label,
    },
};
use machine_factory::config::systems::System;

use crate::editor::EditedFile;

/// The width of the dialog panel in logical units.
const DIALOG_WIDTH: f32 = 560.0;

/// The space between the alert icon and the text column in units of the gap.
const ICON_GAP_FACTOR: f32 = 1.5;

/// The offset of the alert icon below the top of the message in units of the gap.
const ICON_OFFSET_FACTOR: f32 = 0.25;

/// The space above the message in units of the gap.
const MESSAGE_TOP_FACTOR: f32 = 0.25;

/// The horizontal padding inside the detail well in units of the gap.
const DETAIL_PADDING_FACTOR: f32 = 1.25;

/// The most lines of the message. Longer messages end after them.
const MAXIMUM_MESSAGE_LINES: usize = 4;

/// The most lines of the detail text. Longer texts end after them.
const MAXIMUM_DETAIL_LINES: usize = 8;

/// The id of the OK button.
const OK_ID: &str = "error ok";

/// The id of the button that opens the game in the editor.
pub(super) const EDIT_ID: &str = "error edit";

/// The id of the cancel button of a game file that changed outside the manager.
const CANCEL_ID: &str = "error cancel";

/// The id of the button that saves over a game file that changed outside the manager.
pub(super) const OVERWRITE_ID: &str = "error overwrite";

/// The hint of the OK button.
const OK_HINT: Hint<'static> = Hint {
    keys: ACTIVATE_KEYS,
    label: "OK",
};

/// The hint of the focused cancel button.
const CANCEL_HINT: Hint<'static> = Hint {
    keys: ACTIVATE_KEYS,
    label: "Cancel",
};

/// The hint of the focused overwrite button.
const OVERWRITE_HINT: Hint<'static> = Hint {
    keys: ACTIVATE_KEYS,
    label: "Overwrite",
};

/// The hint of cancel beside the focused overwrite button.
const CANCEL_KEYS_HINT: Hint<'static> = Hint {
    keys: CANCEL_KEYS,
    label: "Cancel",
};

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

/// The title of the dialog of a game file that did not open in the editor.
pub(super) const OPEN_FAILED_TITLE: &str = "Could not open the game";

/// The title of the dialog of a game file that did not save.
pub(super) const SAVE_FAILED_TITLE: &str = "Could not save the game";

/// The title of the dialog of a system defaults file that did not open in the editor.
pub(super) const DEFAULTS_OPEN_FAILED_TITLE: &str = "Could not open the system defaults";

/// The title of the dialog of a system defaults file that did not save.
pub(super) const DEFAULTS_SAVE_FAILED_TITLE: &str = "Could not save the system defaults";

/// The title of the dialog of a settings page that did not open.
pub(super) const SETTINGS_OPEN_FAILED_TITLE: &str = "Could not open the settings";

/// The title of the dialog of a settings file that did not save.
pub(super) const SETTINGS_SAVE_FAILED_TITLE: &str = "Could not save the settings";

/// The title of the dialog of a saved settings file that does not read.
pub(super) const SETTINGS_NOT_APPLIED_TITLE: &str = "Could not apply the settings";

/// The titles of the dialogs of the file of `subject` that did not open in
/// the editor and that did not save.
fn editor_titles(subject: &EditedFile) -> (&'static str, &'static str) {
    match subject {
        EditedFile::Game { .. } => (OPEN_FAILED_TITLE, SAVE_FAILED_TITLE),
        EditedFile::SystemDefaults(_) => (DEFAULTS_OPEN_FAILED_TITLE, DEFAULTS_SAVE_FAILED_TITLE),
        EditedFile::Settings => (SETTINGS_OPEN_FAILED_TITLE, SETTINGS_SAVE_FAILED_TITLE),
    }
}

/// The answer of an error dialog in a pass.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) enum DialogAnswer {
    /// The dialog stays open.
    Open,
    /// The close button, OK or cancel closed the dialog.
    Closed,
    /// The dialog closed to open its game in the editor.
    Edit,
    /// The dialog closed to save the changes of the editor over its game file.
    Overwrite,
}

/// The game file that an error dialog opens in the editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EditTarget {
    /// The system of the game.
    pub(crate) system: &'static System,
    /// The game file.
    pub(crate) path: PathBuf,
}

/// An open error dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ErrorDialog {
    /// The title of the panel.
    title: &'static str,
    /// The sentence about what happened.
    message: String,
    /// The error text.
    detail: String,
    /// The game file that the dialog offers to open in the editor.
    edit: Option<EditTarget>,
    /// Whether the dialog offers to save over a game file that changed
    /// outside the manager. It shows Cancel and Overwrite in place of OK.
    overwrite: bool,
}

impl ErrorDialog {
    /// The dialog of the game `game_title` that did not start with `error`.
    pub(crate) fn launch_failed(game_title: &str, error: &str) -> Self {
        Self {
            title: LAUNCH_FAILED_TITLE,
            message: format!("{game_title} did not start."),
            detail: error.to_owned(),
            edit: None,
            overwrite: false,
        }
    }

    /// The dialog of a saved settings file that does not read with `error`.
    /// The manager keeps the host settings that it read before.
    pub(crate) fn settings_not_applied(error: &str) -> Self {
        Self {
            title: SETTINGS_NOT_APPLIED_TITLE,
            message: "The settings file was saved but does not read. The manager keeps the \
                      settings that it read before."
                .to_owned(),
            detail: error.to_owned(),
            edit: None,
            overwrite: false,
        }
    }

    /// The same dialog with a button that opens the game file of `target` in the editor.
    pub(crate) fn with_edit(self, target: EditTarget) -> Self {
        Self {
            edit: Some(target),
            ..self
        }
    }

    /// The dialog of the file of `subject` that did not open in the editor
    /// with `error`.
    pub(crate) fn open_failed(subject: &EditedFile, error: &str) -> Self {
        Self {
            title: editor_titles(subject).0,
            message: format!("{} did not open in the editor.", subject.sentence_start()),
            detail: error.to_owned(),
            edit: None,
            overwrite: false,
        }
    }

    /// The dialog of the file of `subject` that changed outside the manager
    /// since the editor read it, as `detail` says. Cancel returns to the
    /// editor, and Overwrite saves over the file.
    pub(crate) fn changed_outside(subject: &EditedFile, detail: &str) -> Self {
        Self {
            title: editor_titles(subject).1,
            message: format!(
                "{} {} not saved. Overwrite replaces the file with the changes of the editor. \
                 Cancel returns to the editor with the changes.",
                subject.sentence_start(),
                subject.was()
            ),
            detail: detail.to_owned(),
            edit: None,
            overwrite: true,
        }
    }

    /// The dialog of the file of `subject` that did not save with `error`.
    pub(crate) fn save_failed(subject: &EditedFile, error: &str) -> Self {
        Self {
            title: editor_titles(subject).1,
            message: format!(
                "{} {} not saved. The editor keeps the changes.",
                subject.sentence_start(),
                subject.was()
            ),
            detail: error.to_owned(),
            edit: None,
            overwrite: false,
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
            edit: None,
            overwrite: false,
        }
    }

    /// The dialog of the game `game_title` that ended while the write back
    /// of its disk images failed with `error`.
    pub(crate) fn media_not_saved(game_title: &str, error: &FlushError) -> Self {
        Self {
            title: MEDIA_NOT_SAVED_TITLE,
            message: format!("{game_title} ended. Some of its disk images were not saved."),
            detail: error.to_string(),
            edit: None,
            overwrite: false,
        }
    }

    /// The dialog of the game `name` whose file was not removed with `error`.
    pub(crate) fn delete_failed(name: &str, error: &str) -> Self {
        Self {
            title: DELETE_FAILED_TITLE,
            message: format!("{name} was not deleted."),
            detail: error.to_owned(),
            edit: None,
            overwrite: false,
        }
    }

    /// The game file that the dialog offers to open in the editor.
    pub(crate) fn edit_target(&self) -> Option<&EditTarget> {
        self.edit.as_ref()
    }

    /// The hints of the status line while the dialog is open. The focused
    /// button gives the first hint, and Esc and B cancel.
    pub(crate) fn hints(&self, context: &UiContext) -> Vec<Hint<'static>> {
        if !self.overwrite {
            return vec![OK_HINT];
        }
        if context.focused() == Some(UiId::from(OVERWRITE_ID)) {
            vec![OVERWRITE_HINT, CANCEL_KEYS_HINT]
        } else {
            vec![CANCEL_HINT]
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
    /// The OK button holds the focus. Returns the answer of the pass, and
    /// closes the layer when the dialog closes.
    pub(crate) fn show(&self, context: &mut UiContext, area: Rect) -> DialogAnswer {
        context.layer(LAYER_ID, |context| {
            let answer = self.panel(context, area);
            if answer != DialogAnswer::Open {
                context.close_layer();
            }
            answer
        })
    }

    /// Draws the panel of the dialog centered in `area` and returns its answer.
    fn panel(&self, context: &mut UiContext, area: Rect) -> DialogAnswer {
        let theme = context.theme;
        let ok_id = UiId::from(OK_ID);
        let message_style = theme.body();
        let detail_style = theme.small().with_color(theme.colors.text);
        let detail_padding = (theme.gap * DETAIL_PADDING_FACTOR, theme.gap);
        let text_indent = theme.icon_size + theme.gap * ICON_GAP_FACTOR;
        let message_top = theme.gap * MESSAGE_TOP_FACTOR;
        let message_width = DIALOG_WIDTH - theme.padding * 2.0 - text_indent;
        let detail_inset = (
            detail_padding.0 + theme.stroke,
            detail_padding.1 + theme.stroke,
        );
        let detail_width = message_width - detail_inset.0 * 2.0;
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
        ) + detail_inset.1 * 2.0;
        let height = HEADER_HEIGHT
            + message_top
            + message_height
            + theme.gap
            + detail_height
            + theme.gap * 2.0
            + theme.control_height
            + theme.padding;
        let response = modal_in(context, LAYER_ID, area, (DIALOG_WIDTH, height), self.title);

        let (icon_column, text_column) = response
            .content
            .cut_top(message_top)
            .1
            .cut_left(text_indent);
        context.icon(
            Rect::new(
                icon_column.x,
                icon_column.y + theme.gap * ICON_OFFSET_FACTOR,
                theme.icon_size,
                theme.icon_size,
            ),
            Icon::Alert,
            theme.icon_size,
            theme.colors.danger_text,
        );
        let (message_area, rest) = text_column.cut_top(message_height);
        wrapped_label(
            context,
            message_area,
            &message_style,
            &self.message,
            Align::Start,
            MAXIMUM_MESSAGE_LINES,
        );

        let detail_area = rest.cut_top(theme.gap).1.cut_top(detail_height).0;
        context.rect(
            detail_area,
            &RectStyle::filled(theme.colors.well, theme.radius)
                .with_stroke(theme.stroke, theme.colors.well_edge),
        );
        wrapped_label(
            context,
            Rect::new(
                detail_area.x + detail_inset.0,
                detail_area.y + detail_inset.1,
                detail_width,
                detail_height - detail_inset.1 * 2.0,
            ),
            &detail_style,
            &self.detail,
            Align::Start,
            MAXIMUM_DETAIL_LINES,
        );

        let footer = response.content.cut_bottom(theme.control_height).0;
        if self.overwrite {
            return Self::overwrite_buttons(context, footer, response.closed);
        }
        let ok = Button::new(ok_id, "OK").kind(ButtonKind::Primary);
        let (ok_area, rest) = footer.cut_right(ok.width(context));
        let edit = self.edit.is_some() && {
            let edit = Button::new(EDIT_ID, "Edit game").icon(Icon::Edit);
            let edit_area = rest.cut_right(theme.gap).1.cut_right(edit.width(context)).0;
            edit.show(context, edit_area)
        };
        let ok = ok.show(context, ok_area);
        if edit {
            DialogAnswer::Edit
        } else if response.closed || ok {
            DialogAnswer::Closed
        } else {
            DialogAnswer::Open
        }
    }

    /// Draws Cancel and the danger button Overwrite into `footer`. Cancel
    /// takes the focus first. Returns the answer of the pass, with `closed`
    /// for the close button and cancel of the modal.
    fn overwrite_buttons(context: &mut UiContext, footer: Rect, closed: bool) -> DialogAnswer {
        let theme = context.theme;
        let cancel_id = UiId::from(CANCEL_ID);
        let overwrite = Button::new(OVERWRITE_ID, "Overwrite")
            .icon(Icon::Check)
            .kind(ButtonKind::Danger);
        let cancel = Button::new(cancel_id, "Cancel");
        let (overwrite_area, rest) = footer.cut_right(overwrite.width(context));
        let cancel_area = rest
            .cut_right(theme.gap)
            .1
            .cut_right(cancel.width(context))
            .0;
        context.default_focus(cancel_id);
        let cancelled = cancel.show(context, cancel_area);
        let overwritten = overwrite.show(context, overwrite_area);
        if overwritten {
            DialogAnswer::Overwrite
        } else if closed || cancelled {
            DialogAnswer::Closed
        } else {
            DialogAnswer::Open
        }
    }
}

#[cfg(test)]
mod tests {
    use gui::ui::Navigation;

    use super::*;
    use crate::{editor::test_support::system_named, gui_test_support::GuiHarness};

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
        assert_eq!(
            harness.frame(|context| dialog.show(context, area)),
            DialogAnswer::Open
        );
        assert_eq!(harness.state.focused(), Some(UiId::from(OK_ID)));
        harness.input.confirm = true;
        assert_eq!(
            harness.frame(|context| dialog.show(context, area)),
            DialogAnswer::Closed
        );

        let dialog = ErrorDialog::game_stopped("X1", "halt", &Ok(()));
        harness.frame(|context| dialog.show(context, area));
        harness.input.cancel = true;
        assert_eq!(
            harness.frame(|context| dialog.show(context, area)),
            DialogAnswer::Closed
        );

        let dialog = ErrorDialog::launch_failed("Edge", "error");
        harness.frame(|context| dialog.show(context, area));
        harness.input.navigation = Some(Navigation::Down);
        assert_eq!(
            harness.frame(|context| dialog.show(context, area)),
            DialogAnswer::Open
        );
    }

    /// Verifies that the launch dialog with a game file offers its editor beside OK and that OK keeps the focus.
    #[test]
    fn the_launch_dialog_opens_the_editor_from_its_edit_button() {
        let mut harness = GuiHarness::new(960, 600);
        let area = Rect::new(0.0, 0.0, 960.0, 600.0);
        let system = machine_factory::config::systems::compiled_systems()
            .next()
            .expect("one system is compiled");
        let target = EditTarget {
            system,
            path: PathBuf::from("/root/config/edge.conf"),
        };
        let dialog = ErrorDialog::launch_failed("Edge", "error").with_edit(target.clone());
        assert_eq!(dialog.edit_target(), Some(&target));
        harness.frame(|context| dialog.show(context, area));
        harness.frame(|context| dialog.show(context, area));
        assert_eq!(harness.state.focused(), Some(UiId::from(OK_ID)));
        harness.input.navigation = Some(Navigation::Left);
        harness.frame(|context| dialog.show(context, area));
        assert_eq!(harness.state.focused(), Some(UiId::from(EDIT_ID)));
        harness.input.confirm = true;
        assert_eq!(
            harness.frame(|context| dialog.show(context, area)),
            DialogAnswer::Edit
        );
    }

    /// The game file of the PC-98 game Edge.
    fn game() -> EditedFile {
        EditedFile::Game {
            system: system_named("pc98"),
            name: "Edge".to_owned(),
        }
    }

    /// Verifies the titles and the texts of the dialogs of the editor for a game file, a system defaults file and the settings file.
    #[test]
    fn the_editor_dialogs_follow_the_file_kind() {
        let defaults = EditedFile::SystemDefaults(system_named("pc98"));
        let settings = EditedFile::Settings;
        let save = |subject: &EditedFile| {
            let dialog = ErrorDialog::save_failed(subject, "/root/config/pc98/edge.conf: denied");
            (dialog.title().to_owned(), dialog.message().to_owned())
        };
        assert_eq!(
            save(&game()),
            (
                "Could not save the game".to_owned(),
                "Edge was not saved. The editor keeps the changes.".to_owned()
            )
        );
        assert_eq!(
            save(&defaults),
            (
                "Could not save the system defaults".to_owned(),
                "The PC-98 system defaults were not saved. The editor keeps the changes."
                    .to_owned()
            )
        );
        assert_eq!(
            save(&settings),
            (
                "Could not save the settings".to_owned(),
                "The settings were not saved. The editor keeps the changes.".to_owned()
            )
        );
        let open = |subject: &EditedFile| {
            let dialog = ErrorDialog::open_failed(subject, "gone");
            assert_eq!(dialog.detail(), "gone");
            (dialog.title().to_owned(), dialog.message().to_owned())
        };
        assert_eq!(
            open(&game()),
            (
                "Could not open the game".to_owned(),
                "Edge did not open in the editor.".to_owned()
            )
        );
        assert_eq!(
            open(&defaults),
            (
                "Could not open the system defaults".to_owned(),
                "The PC-98 system defaults did not open in the editor.".to_owned()
            )
        );
        assert_eq!(
            open(&settings),
            (
                "Could not open the settings".to_owned(),
                "The settings did not open in the editor.".to_owned()
            )
        );
        for (kind, title, subject) in [
            (game(), SAVE_FAILED_TITLE, "Edge was"),
            (
                defaults,
                DEFAULTS_SAVE_FAILED_TITLE,
                "The PC-98 system defaults were",
            ),
            (settings, SETTINGS_SAVE_FAILED_TITLE, "The settings were"),
        ] {
            let dialog = ErrorDialog::changed_outside(&kind, "changed");
            assert_eq!(dialog.title(), title);
            assert!(
                dialog
                    .message()
                    .starts_with(&format!("{subject} not saved. Overwrite replaces")),
                "{}",
                dialog.message()
            );
        }
    }

    /// Verifies that the dialog of a file that changed outside the manager starts on Cancel, that its hints follow the focus and that each button gives its answer.
    #[test]
    fn the_changed_outside_dialog_offers_cancel_and_overwrite() {
        let mut harness = GuiHarness::new(960, 600);
        let area = Rect::new(0.0, 0.0, 960.0, 572.0);
        let dialog = ErrorDialog::changed_outside(
            &game(),
            "config/pc98/edge.conf changed outside the manager since the editor read it.",
        );
        assert_eq!(dialog.title(), SAVE_FAILED_TITLE);
        let labels = |harness: &mut GuiHarness| {
            harness.frame(|context| {
                dialog.show(context, area);
                dialog
                    .hints(context)
                    .iter()
                    .map(|hint| hint.label)
                    .collect::<Vec<_>>()
            })
        };
        labels(&mut harness);
        assert_eq!(labels(&mut harness), ["Cancel"]);
        assert_eq!(harness.state.focused(), Some(UiId::from(CANCEL_ID)));
        harness.input.navigation = Some(Navigation::Right);
        assert_eq!(labels(&mut harness), ["Overwrite", "Cancel"]);
        assert_eq!(harness.state.focused(), Some(UiId::from(OVERWRITE_ID)));
        harness.input.confirm = true;
        assert_eq!(
            harness.frame(|context| dialog.show(context, area)),
            DialogAnswer::Overwrite
        );
        for press in [
            |harness: &mut GuiHarness| harness.input.cancel = true,
            |harness: &mut GuiHarness| harness.input.confirm = true,
        ] {
            let mut harness = GuiHarness::new(960, 600);
            harness.frame(|context| dialog.show(context, area));
            harness.frame(|context| dialog.show(context, area));
            press(&mut harness);
            assert_eq!(
                harness.frame(|context| dialog.show(context, area)),
                DialogAnswer::Closed
            );
        }
        let plain = ErrorDialog::save_failed(&game(), "denied");
        let hints = harness.frame(|context| plain.hints(context));
        assert_eq!(hints, [OK_HINT]);
    }

    /// The top of the message below the top of the panel in the DESIGN figures.
    const DESIGN_MESSAGE_TOP: f32 = 54.0;

    /// The top of the alert icon below the top of the panel in the DESIGN figures.
    const DESIGN_ICON_TOP: f32 = 56.0;

    /// The width of the detail well in the DESIGN figures.
    const DESIGN_WELL_WIDTH: f32 = 500.0;

    /// The height of a detail well with one line in the DESIGN figures:
    /// a line of small text, the padding and the border.
    const DESIGN_ONE_LINE_WELL_HEIGHT: f32 = 12.0 * 1.43 + 8.0 * 2.0 + 1.0 * 2.0;

    /// The height of one line of the message in the DESIGN figures.
    const DESIGN_MESSAGE_LINE_HEIGHT: f32 = 14.0 * 1.43;

    /// Verifies that every dialog kind draws the alert icon in the danger text color at the start of the content, before the text column, and that the message top, the icon top and the detail well follow the DESIGN figures.
    #[test]
    fn every_dialog_draws_the_alert_icon_before_its_text() {
        let area = Rect::new(0.0, 0.0, 960.0, 572.0);
        let dialogs = [
            ErrorDialog::launch_failed("Edge", "error"),
            ErrorDialog::game_stopped("X1", "halt", &Ok(())),
            ErrorDialog::media_not_saved("Edge", &FlushError::new("lost")),
            ErrorDialog::delete_failed("Edge", "denied"),
            ErrorDialog::open_failed(&game(), "gone"),
            ErrorDialog::save_failed(&game(), "denied"),
            ErrorDialog::changed_outside(&game(), "changed"),
        ];
        let icon_inset = {
            let mut harness = GuiHarness::new(960, 600);
            harness.frame(|context| {
                let theme = context.theme;
                let size = theme.icon_size;
                let box_area = Rect::new(0.0, 0.0, size, size);
                context.icon(box_area, Icon::Alert, size, theme.colors.danger_text);
            });
            harness.scene.instances()[0].rect[1]
        };
        for dialog in dialogs {
            let mut harness = GuiHarness::new(960, 600);
            harness.frame(|context| dialog.show(context, area));
            let theme = &harness.theme;
            let instances = harness.scene.instances();
            let panel = instances
                .iter()
                .find(|instance| {
                    instance.kind[0] == gui::Instance::KIND_RECTANGLE
                        && instance.rect[2] == DIALOG_WIDTH
                })
                .expect("the panel")
                .rect;
            let content_left = panel[0] + theme.padding;
            let text_left = content_left + theme.icon_size + theme.gap * ICON_GAP_FACTOR;
            let danger = theme.colors.danger_text.to_linear();
            let icon = instances
                .iter()
                .find(|instance| {
                    instance.kind[0] == gui::Instance::KIND_GLYPH && instance.color == danger
                })
                .unwrap_or_else(|| panic!("the alert icon of {}", dialog.title()));
            assert!(icon.rect[0] >= content_left - 1.0, "{:?}", icon.rect);
            assert!(icon.rect[0] + icon.rect[2] <= text_left, "{:?}", icon.rect);
            assert!(
                (icon.rect[1] - panel[1] - DESIGN_ICON_TOP - icon_inset).abs() <= 0.5,
                "{:?} {panel:?}",
                icon.rect
            );
            let well = instances
                .iter()
                .find(|instance| {
                    instance.kind[0] == gui::Instance::KIND_RECTANGLE
                        && instance.color == theme.colors.well.to_linear()
                })
                .expect("the detail well")
                .rect;
            assert!((well[0] - text_left).abs() <= 0.5, "{well:?}");
            assert!((well[2] - DESIGN_WELL_WIDTH).abs() <= 0.5, "{well:?}");
            assert!(
                (well[3] - DESIGN_ONE_LINE_WELL_HEIGHT).abs() <= 0.5,
                "{well:?}"
            );
            let message_lines = ((well[1] - panel[1] - DESIGN_MESSAGE_TOP - theme.gap)
                / DESIGN_MESSAGE_LINE_HEIGHT)
                .round();
            assert!(message_lines >= 1.0, "{well:?}");
            assert!(
                (well[1]
                    - panel[1]
                    - DESIGN_MESSAGE_TOP
                    - message_lines * DESIGN_MESSAGE_LINE_HEIGHT
                    - theme.gap)
                    .abs()
                    <= 0.5,
                "{well:?} {panel:?}"
            );
            let message_left = instances
                .iter()
                .filter(|instance| {
                    instance.kind[0] == gui::Instance::KIND_GLYPH
                        && instance.color == theme.colors.text.to_linear()
                        && instance.rect[1] > panel[1] + HEADER_HEIGHT
                })
                .map(|instance| instance.rect[0])
                .fold(f32::MAX, f32::min);
            assert!(message_left < panel[0] + DIALOG_WIDTH, "the message");
            assert!(
                message_left >= text_left - 1.0,
                "{message_left} {text_left}"
            );
        }
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
