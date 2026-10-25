//! The error dialogs of the manager: a launch that failed, a game that
//! stopped after a panic of its core, disk images that were not saved, a
//! game file that was not deleted, a file of the editor that did not open
//! or did not save, a file of the editor that changed outside the manager
//! before a save, and a saved settings file that does not read.

use std::path::PathBuf;

use ::common::FlushError;
use gui::{
    Rect, RectStyle,
    ui::{
        ACTIVATE_KEYS, Align, Button, ButtonKind, CANCEL_KEYS, HEADER_HEIGHT, Icon, TextBuffer,
        UiContext, UiId, WrappedText, modal_in,
    },
};
use i18n::Key;
use machine_factory::config::systems::System;

use self::event::{ErrorDetail, ErrorEvent};
use crate::{
    editor::{EditedFile, OpenError, SaveFailure},
    texts::{HintText, HintTexts, common, errors},
};

mod event;

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
pub(super) const OK_ID: &str = "error ok";

/// The id of the button that opens the game in the editor.
pub(super) const EDIT_ID: &str = "error edit";

/// The id of the cancel button of a game file that changed outside the manager.
pub(super) const CANCEL_ID: &str = "error cancel";

/// The id of the button that saves over a game file that changed outside the manager.
pub(super) const OVERWRITE_ID: &str = "error overwrite";

/// The hint of the OK button.
const OK_HINT: HintText = HintText::new(ACTIVATE_KEYS, common::OK);

/// The hint of the focused cancel button.
const CANCEL_HINT: HintText = HintText::new(ACTIVATE_KEYS, common::CANCEL);

/// The hint of the focused overwrite button.
const OVERWRITE_HINT: HintText = HintText::new(ACTIVATE_KEYS, errors::OVERWRITE);

/// The hint of cancel beside the focused overwrite button.
const CANCEL_KEYS_HINT: HintText = HintText::new(CANCEL_KEYS, common::CANCEL);

/// The id of the modal layer of the dialog.
pub(super) const LAYER_ID: &str = "error";

/// The keys of the titles of the dialogs of the file of `subject` that did
/// not open in the editor and that did not save.
fn editor_titles(subject: &EditedFile) -> (Key, Key) {
    match subject {
        EditedFile::Game { .. } => (
            errors::GAME_OPEN_FAILED_TITLE,
            errors::GAME_SAVE_FAILED_TITLE,
        ),
        EditedFile::SystemDefaults(_) => (
            errors::DEFAULTS_OPEN_FAILED_TITLE,
            errors::DEFAULTS_SAVE_FAILED_TITLE,
        ),
        EditedFile::Settings => (
            errors::SETTINGS_OPEN_FAILED_TITLE,
            errors::SETTINGS_SAVE_FAILED_TITLE,
        ),
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
#[derive(Debug, Clone)]
pub(crate) struct ErrorDialog {
    /// The key of the title of the panel.
    title: Key,
    /// The event of the dialog.
    event: ErrorEvent,
    /// The error text.
    detail: ErrorDetail,
    /// The game file that the dialog offers to open in the editor.
    edit: Option<EditTarget>,
    /// Whether the dialog offers to save over a game file that changed
    /// outside the manager. It shows Cancel and Overwrite in place of OK.
    overwrite: bool,
    /// The sentence of the event in the catalog of the last pass.
    message_text: TextBuffer<()>,
    /// The error text in the catalog of the last pass.
    detail_text: TextBuffer<()>,
    /// The wrapped lines of the sentence of the event.
    message_lines: WrappedText,
    /// The wrapped lines of the error text.
    detail_lines: WrappedText,
}

impl ErrorDialog {
    /// The dialog with the title `title` about `event` with the detail `detail`.
    fn new(title: Key, event: ErrorEvent, detail: ErrorDetail) -> Self {
        Self {
            title,
            event,
            detail,
            edit: None,
            overwrite: false,
            message_text: TextBuffer::new(),
            detail_text: TextBuffer::new(),
            message_lines: WrappedText::new(),
            detail_lines: WrappedText::new(),
        }
    }

    /// The dialog of the game `game_title` that did not start with `error`.
    pub(crate) fn launch_failed(game_title: &str, error: &str) -> Self {
        Self::new(
            errors::LAUNCH_FAILED_TITLE,
            ErrorEvent::LaunchFailed(game_title.to_owned()),
            ErrorDetail::Error(error.to_owned()),
        )
    }

    /// The dialog of a saved settings file that does not read with `error`.
    /// The manager keeps the host settings that it read before.
    pub(crate) fn settings_not_applied(error: &str) -> Self {
        Self::new(
            errors::SETTINGS_NOT_APPLIED_TITLE,
            ErrorEvent::SettingsNotApplied,
            ErrorDetail::Error(error.to_owned()),
        )
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
    pub(crate) fn open_failed(subject: &EditedFile, error: &OpenError) -> Self {
        Self::new(
            editor_titles(subject).0,
            ErrorEvent::OpenFailed(subject.clone()),
            ErrorDetail::Open(error.clone()),
        )
    }

    /// The dialog of a new game of `system` that did not open in the editor
    /// with `error`.
    pub(crate) fn new_game_open_failed(system: &'static System, error: &OpenError) -> Self {
        Self::new(
            errors::GAME_OPEN_FAILED_TITLE,
            ErrorEvent::NewGameOpenFailed(system.label),
            ErrorDetail::Open(error.clone()),
        )
    }

    /// The dialog of the file of `subject` at the shown path `shown` that
    /// changed outside the manager since the editor read it. Cancel returns
    /// to the editor, and Overwrite saves over the file.
    pub(crate) fn changed_outside(subject: &EditedFile, shown: &str) -> Self {
        Self {
            overwrite: true,
            ..Self::new(
                editor_titles(subject).1,
                ErrorEvent::ChangedOutside(subject.clone()),
                ErrorDetail::Save(SaveFailure::ChangedOutside(shown.to_owned())),
            )
        }
    }

    /// The dialog of the file of `subject` that did not save with `failure`.
    pub(crate) fn save_failed(subject: &EditedFile, failure: &SaveFailure) -> Self {
        Self::new(
            editor_titles(subject).1,
            ErrorEvent::SaveFailed(subject.clone()),
            ErrorDetail::Save(failure.clone()),
        )
    }

    /// The dialog of a game of the system `system_label` whose core
    /// panicked with `panic_message`. A game that the manager does not know
    /// has no label. `flush` is the result of the write back of the disk
    /// images after the panic. Its errors come before the panic message, so
    /// the line limit of the detail cuts the panic message first.
    pub(crate) fn game_stopped(
        system_label: Option<&'static str>,
        panic_message: &str,
        flush: &Result<(), FlushError>,
    ) -> Self {
        let detail = match flush {
            Ok(()) => panic_message.to_owned(),
            Err(error) => format!("{error}\n{panic_message}"),
        };
        Self::new(
            errors::GAME_STOPPED_TITLE,
            ErrorEvent::GameStopped {
                system: system_label,
                saved: flush.is_ok(),
            },
            ErrorDetail::Error(detail),
        )
    }

    /// The dialog of the game `game_title` that ended while the write back
    /// of its disk images failed with `error`. A game that the manager does
    /// not know has no title.
    pub(crate) fn media_not_saved(game_title: Option<&str>, error: &FlushError) -> Self {
        Self::new(
            errors::MEDIA_NOT_SAVED_TITLE,
            ErrorEvent::MediaNotSaved(game_title.map(str::to_owned)),
            ErrorDetail::Error(error.to_string()),
        )
    }

    /// The dialog of the game `name` whose file at the shown path `path`
    /// was not removed with the error `error` of the operating system.
    pub(crate) fn delete_failed(name: &str, path: String, error: String) -> Self {
        Self::new(
            errors::DELETE_FAILED_TITLE,
            ErrorEvent::DeleteFailed(name.to_owned()),
            ErrorDetail::PathError { path, error },
        )
    }

    /// The game file that the dialog offers to open in the editor.
    pub(crate) fn edit_target(&self) -> Option<&EditTarget> {
        self.edit.as_ref()
    }

    /// The hints of the status line while the dialog is open. The focused
    /// button gives the first hint, and Esc and B cancel.
    pub(crate) fn hints(&self, context: &UiContext) -> HintTexts {
        if !self.overwrite {
            return HintTexts::from([OK_HINT]);
        }
        if context.focused() == Some(UiId::from(OVERWRITE_ID)) {
            HintTexts::from([OVERWRITE_HINT, CANCEL_KEYS_HINT])
        } else {
            HintTexts::from([CANCEL_HINT])
        }
    }

    /// The key of the title of the panel.
    #[cfg(test)]
    pub(crate) fn title(&self) -> Key {
        self.title
    }

    /// The sentence of the event that the last pass drew. A dialog without a
    /// pass has none.
    #[cfg(test)]
    pub(crate) fn message(&self) -> Option<&str> {
        self.message_text.last_text()
    }

    /// The error text that the last pass drew. A dialog without a pass has
    /// none.
    #[cfg(test)]
    pub(crate) fn detail(&self) -> Option<&str> {
        self.detail_text.last_text()
    }

    /// Draws one pass of the dialog in its modal layer, centered in `area`.
    /// The OK button holds the focus. Returns the answer of the pass, and
    /// closes the layer when the dialog closes.
    pub(crate) fn show(&mut self, context: &mut UiContext, area: Rect) -> DialogAnswer {
        context.layer(LAYER_ID, |context| {
            let answer = self.panel(context, area);
            if answer != DialogAnswer::Open {
                context.close_layer();
            }
            answer
        })
    }

    /// Draws the panel of the dialog centered in `area` and returns its answer.
    fn panel(&mut self, context: &mut UiContext, area: Rect) -> DialogAnswer {
        let theme = context.theme;
        let catalog = context.catalog;
        let Self {
            event,
            detail,
            message_text,
            detail_text,
            message_lines,
            detail_lines,
            ..
        } = self;
        let message = message_text.text(context, (), |catalog, (), out| {
            event.write(catalog, out);
        });
        let detail = detail_text.text(context, (), |catalog, (), out| {
            detail.write(catalog, out);
        });
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
        let message_height = message_lines.height(
            context,
            &message_style,
            message,
            message_width,
            MAXIMUM_MESSAGE_LINES,
        );
        let detail_height = detail_lines.height(
            context,
            &detail_style,
            detail,
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
        let response = modal_in(
            context,
            LAYER_ID,
            area,
            (DIALOG_WIDTH, height),
            catalog.text(self.title),
        );

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
        message_lines.show(
            context,
            message_area,
            &message_style,
            message,
            Align::Start,
            MAXIMUM_MESSAGE_LINES,
        );

        let detail_area = rest.cut_top(theme.gap).1.cut_top(detail_height).0;
        context.rect(
            detail_area,
            &RectStyle::filled(theme.colors.well, theme.radius)
                .with_stroke(theme.stroke, theme.colors.well_edge),
        );
        detail_lines.show(
            context,
            Rect::new(
                detail_area.x + detail_inset.0,
                detail_area.y + detail_inset.1,
                detail_width,
                detail_height - detail_inset.1 * 2.0,
            ),
            &detail_style,
            detail,
            Align::Start,
            MAXIMUM_DETAIL_LINES,
        );

        let footer = response.content.cut_bottom(theme.control_height).0;
        if self.overwrite {
            return Self::overwrite_buttons(context, footer, response.closed);
        }
        let ok = Button::new(ok_id, catalog.text(common::OK)).kind(ButtonKind::Primary);
        let (ok_area, rest) = footer.cut_right(ok.width(context));
        let edit = self.edit.is_some() && {
            let edit = Button::new(EDIT_ID, catalog.text(errors::EDIT_GAME)).icon(Icon::Edit);
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
        let catalog = context.catalog;
        let overwrite = Button::new(OVERWRITE_ID, catalog.text(errors::OVERWRITE))
            .icon(Icon::Check)
            .kind(ButtonKind::Danger);
        let cancel = Button::new(cancel_id, catalog.text(common::CANCEL));
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
    use crate::{
        editor::test_support::system_named,
        gui_test_support::GuiHarness,
        texts::{
            LanguageCatalog, english_labels,
            errors::{
                DEFAULTS_OPEN_FAILED_TITLE, DEFAULTS_SAVE_FAILED_TITLE, DELETE_FAILED_TITLE,
                GAME_OPEN_FAILED_TITLE, GAME_SAVE_FAILED_TITLE, GAME_STOPPED_TITLE,
                LAUNCH_FAILED_TITLE, MEDIA_NOT_SAVED_TITLE, SETTINGS_OPEN_FAILED_TITLE,
                SETTINGS_SAVE_FAILED_TITLE,
            },
            test_catalog,
        },
    };

    /// The area of a dialog in the tests.
    const AREA: Rect = Rect::new(0.0, 0.0, 960.0, 572.0);

    /// Returns `dialog` after one pass in the English texts.
    fn drawn(mut dialog: ErrorDialog) -> ErrorDialog {
        GuiHarness::new(960, 600).frame(|context| dialog.show(context, AREA));
        dialog
    }

    /// Verifies the title, the message and the detail of the launch error dialog.
    #[test]
    fn the_launch_dialog_shows_the_game_and_the_error() {
        let dialog = drawn(ErrorDialog::launch_failed(
            "Edge",
            "fdd1 image not found: /games/edge.d88",
        ));
        assert_eq!(dialog.title(), LAUNCH_FAILED_TITLE);
        assert_eq!(dialog.message(), Some("Edge did not start."));
        assert_eq!(
            dialog.detail(),
            Some("fdd1 image not found: /games/edge.d88")
        );
    }

    /// Verifies that the panic dialog mentions the saved disk images only after a flush without a panic.
    #[test]
    fn the_panic_dialog_mentions_the_saved_images_only_after_a_good_flush() {
        let saved = drawn(ErrorDialog::game_stopped(
            Some("PC-98"),
            "sample panic",
            &Ok(()),
        ));
        assert_eq!(saved.title(), GAME_STOPPED_TITLE);
        assert_eq!(
            saved.message(),
            Some("The PC-98 core stopped with an error. The disk images were saved.")
        );
        assert_eq!(saved.detail(), Some("sample panic"));
        let lost = drawn(ErrorDialog::game_stopped(
            Some("PC-98"),
            "sample panic",
            &Err(FlushError::new("floppy image /a.d88 could not be written")),
        ));
        assert_eq!(
            lost.message(),
            Some("The PC-98 core stopped with an error. Some disk images were not saved.")
        );
        assert_eq!(
            lost.detail(),
            Some("floppy image /a.d88 could not be written\nsample panic")
        );
    }

    /// Verifies the dialog of disk images that were not saved after a normal end of the game.
    #[test]
    fn the_media_dialog_lists_the_images_that_were_not_saved() {
        let error = FlushError {
            messages: vec!["first".to_owned(), "second".to_owned()],
        };
        let dialog = drawn(ErrorDialog::media_not_saved(Some("Edge"), &error));
        assert_eq!(dialog.title(), MEDIA_NOT_SAVED_TITLE);
        assert_eq!(
            dialog.message(),
            Some("Edge ended. Some of its disk images were not saved.")
        );
        assert_eq!(dialog.detail(), Some("first\nsecond"));
    }

    /// Verifies the dialog of a game file that was not deleted.
    #[test]
    fn the_delete_dialog_shows_the_game_and_the_error() {
        let dialog = drawn(ErrorDialog::delete_failed(
            "Edge",
            "config/pc98/edge.conf".to_owned(),
            "Permission denied".to_owned(),
        ));
        assert_eq!(dialog.title(), DELETE_FAILED_TITLE);
        assert_eq!(dialog.message(), Some("Edge was not deleted."));
        assert_eq!(
            dialog.detail(),
            Some("config/pc98/edge.conf: Permission denied")
        );
    }

    /// Verifies that OK holds the focus, that confirm and cancel close the dialog and that navigation does not.
    #[test]
    fn ok_holds_the_focus_and_confirm_or_cancel_close_the_dialog() {
        let mut harness = GuiHarness::new(960, 600);
        let area = Rect::new(0.0, 0.0, 960.0, 600.0);
        let mut dialog = ErrorDialog::launch_failed("Edge", "error");
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

        let mut dialog = ErrorDialog::game_stopped(Some("X1"), "halt", &Ok(()));
        harness.frame(|context| dialog.show(context, area));
        harness.input.cancel = true;
        assert_eq!(
            harness.frame(|context| dialog.show(context, area)),
            DialogAnswer::Closed
        );

        let mut dialog = ErrorDialog::launch_failed("Edge", "error");
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
        let mut dialog = ErrorDialog::launch_failed("Edge", "error").with_edit(target.clone());
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

    /// Verifies the titles and the events of the dialogs of the editor for a game file, a system defaults file and the settings file.
    #[test]
    fn the_editor_dialogs_follow_the_file_kind() {
        let defaults = EditedFile::SystemDefaults(system_named("pc98"));
        let settings = EditedFile::Settings;
        let open_error = OpenError::Failed {
            path: PathBuf::from("/root/config/pc98/edge.conf"),
            error: "gone".to_owned(),
        };
        let save_failure = SaveFailure::Error {
            path: PathBuf::from("/root/config/pc98/edge.conf"),
            error: "denied".to_owned(),
        };
        for (subject, open_title, save_title) in [
            (game(), GAME_OPEN_FAILED_TITLE, GAME_SAVE_FAILED_TITLE),
            (
                defaults,
                DEFAULTS_OPEN_FAILED_TITLE,
                DEFAULTS_SAVE_FAILED_TITLE,
            ),
            (
                settings,
                SETTINGS_OPEN_FAILED_TITLE,
                SETTINGS_SAVE_FAILED_TITLE,
            ),
        ] {
            let open = ErrorDialog::open_failed(&subject, &open_error);
            assert_eq!(open.title(), open_title);
            assert_eq!(open.event, ErrorEvent::OpenFailed(subject.clone()));
            assert_eq!(open.detail, ErrorDetail::Open(open_error.clone()));
            let save = ErrorDialog::save_failed(&subject, &save_failure);
            assert_eq!(save.title(), save_title);
            assert_eq!(save.event, ErrorEvent::SaveFailed(subject.clone()));
            assert_eq!(save.detail, ErrorDetail::Save(save_failure.clone()));
            let changed = ErrorDialog::changed_outside(&subject, "changed");
            assert_eq!(changed.title(), save_title);
            assert_eq!(changed.event, ErrorEvent::ChangedOutside(subject.clone()));
            assert_eq!(
                changed.detail,
                ErrorDetail::Save(SaveFailure::ChangedOutside("changed".to_owned()))
            );
        }
    }

    /// The German texts of a new game that did not open and of a system that is not compiled.
    const GERMAN_ERRORS: &str = "[errors]\n\
                                 new-game-open-failed = Ein neues {system} Spiel ging nicht auf.\n\
                                 not-compiled = das System {system} fehlt\n";

    /// Verifies that a switch of the catalog while the dialog is open gives the drawn message and detail the new language in the next pass and that a switch back gives the English texts.
    #[test]
    fn an_open_dialog_follows_a_catalog_switch() {
        let mut harness = GuiHarness::new(960, 600);
        let mut dialog = ErrorDialog::new_game_open_failed(
            system_named("pc98"),
            &OpenError::NotCompiled("pc98"),
        );
        let english_texts = (
            Some("A new PC-98 game did not open in the editor."),
            Some("the system pc98 is not compiled"),
        );
        harness.frame(|context| dialog.show(context, AREA));
        assert_eq!((dialog.message(), dialog.detail()), english_texts);
        harness.set_catalog(LanguageCatalog::Loaded(Box::new(test_catalog(
            GERMAN_ERRORS,
        ))));
        harness.frame(|context| dialog.show(context, AREA));
        assert_eq!(
            (dialog.message(), dialog.detail()),
            (
                Some("Ein neues PC-98 Spiel ging nicht auf."),
                Some("das System pc98 fehlt")
            )
        );
        harness.set_catalog(LanguageCatalog::English);
        harness.frame(|context| dialog.show(context, AREA));
        assert_eq!((dialog.message(), dialog.detail()), english_texts);
    }

    /// Verifies that the dialog of a file that changed outside the manager starts on Cancel, that its hints follow the focus and that each button gives its answer.
    #[test]
    fn the_changed_outside_dialog_offers_cancel_and_overwrite() {
        let mut harness = GuiHarness::new(960, 600);
        let area = Rect::new(0.0, 0.0, 960.0, 572.0);
        let mut dialog = drawn(ErrorDialog::changed_outside(
            &game(),
            "config/pc98/edge.conf",
        ));
        assert_eq!(
            dialog.detail(),
            Some("config/pc98/edge.conf changed outside the manager since the editor read it.")
        );
        assert_eq!(dialog.title(), GAME_SAVE_FAILED_TITLE);
        let mut labels = |harness: &mut GuiHarness| {
            harness.frame(|context| {
                dialog.show(context, area);
                english_labels(&dialog.hints(context))
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
        let plain = ErrorDialog::save_failed(
            &game(),
            &SaveFailure::Error {
                path: PathBuf::from("edge.conf"),
                error: "denied".to_owned(),
            },
        );
        let hints = harness.frame(|context| plain.hints(context));
        assert_eq!(*hints, [OK_HINT]);
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
            ErrorDialog::game_stopped(Some("X1"), "halt", &Ok(())),
            ErrorDialog::media_not_saved(Some("Edge"), &FlushError::new("lost")),
            ErrorDialog::delete_failed("Edge", "edge.conf".to_owned(), "denied".to_owned()),
            ErrorDialog::open_failed(
                &game(),
                &OpenError::Failed {
                    path: PathBuf::from("edge.conf"),
                    error: "gone".to_owned(),
                },
            ),
            ErrorDialog::save_failed(
                &game(),
                &SaveFailure::Error {
                    path: PathBuf::from("edge.conf"),
                    error: "denied".to_owned(),
                },
            ),
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
        for mut dialog in dialogs {
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
                .unwrap_or_else(|| panic!("the alert icon of {:?}", dialog.message()));
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
        let mut dialog = ErrorDialog::launch_failed("Edge", &"error text ".repeat(60));
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
