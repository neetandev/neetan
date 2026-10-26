//! The question before the editor closes with unsaved changes.

use gui::{
    Rect,
    ui::{
        CONFIRM_HINTS, ConfirmDialog, ConfirmLines, ConfirmResponse, Icon, TextBuffer, UiContext,
        confirm_hints,
    },
};
use i18n::Catalog;
use stack_vec::StackVec;

use super::EditedFile;
use crate::texts::{CONFIRM_KEYS, HintText, discard};

/// The id of the modal layer of the question.
pub(crate) const DISCARD_LAYER_ID: &str = "discard";

/// The answer of the question in a pass.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) enum DiscardOutcome {
    /// The question stays open.
    Open,
    /// The user keeps editing.
    Kept,
    /// The user drops the changes and the editor closes.
    Discarded,
}

/// Writes the question about the changes to the file of `subject` in
/// `catalog`.
fn write_message(catalog: &Catalog, subject: &EditedFile, out: &mut String) {
    let _ = match subject {
        EditedFile::Game { name, .. } => catalog.write(out, discard::game(name)),
        EditedFile::SystemDefaults(system) => catalog.write(out, discard::defaults(&system.label)),
        EditedFile::Settings => {
            out.push_str(catalog.text(discard::SETTINGS));
            Ok(())
        }
    };
}

/// The question before the changes to one file go away.
#[derive(Debug, Clone)]
pub(crate) struct DiscardQuestion {
    /// The file of the changes.
    subject: EditedFile,
    /// The question in the catalog of the last pass.
    message: TextBuffer<()>,
    /// The wrapped lines of the question.
    lines: ConfirmLines,
    /// The answer of the last pass, for the hints.
    response: ConfirmResponse,
}

impl DiscardQuestion {
    /// The question about the changes to the file of `subject`.
    pub(crate) fn new(subject: EditedFile) -> Self {
        Self {
            subject,
            message: TextBuffer::new(),
            lines: ConfirmLines::new(),
            response: ConfirmResponse::default(),
        }
    }

    /// The question that the last pass drew. A question without a pass has
    /// none.
    #[cfg(test)]
    pub(crate) fn message(&self) -> Option<&str> {
        self.message.last_text()
    }

    /// The hints of the status line for the answer of the last pass, in the
    /// order of the hints of a confirm dialog.
    pub(crate) fn hints(&self) -> StackVec<HintText, CONFIRM_HINTS> {
        confirm_hints(
            self.response,
            CONFIRM_KEYS,
            discard::KEEP_EDITING,
            discard::ACTION,
            HintText::new,
        )
    }

    /// Draws one pass of the question centered in `area` and returns its answer.
    pub(crate) fn show(&mut self, context: &mut UiContext, area: Rect) -> DiscardOutcome {
        let catalog = context.catalog;
        let Self {
            subject,
            message,
            lines,
            ..
        } = self;
        let message = message.text(context, (), |catalog, (), out| {
            write_message(catalog, subject, out);
        });
        self.response = ConfirmDialog::new(
            DISCARD_LAYER_ID,
            catalog.text(discard::TITLE),
            message,
            catalog.text(discard::KEEP_EDITING),
            catalog.text(discard::ACTION),
        )
        .action_icon(Icon::Delete)
        .danger(true)
        .area(area)
        .show(context, lines);
        if self.response.confirmed {
            DiscardOutcome::Discarded
        } else if self.response.cancelled {
            DiscardOutcome::Kept
        } else {
            DiscardOutcome::Open
        }
    }
}

#[cfg(test)]
mod tests {
    use gui::ui::{Navigation, UiId};

    use super::*;
    use crate::{
        editor::test_support::system_named,
        gui_test_support::{GuiHarness, MINIMUM_SCREEN},
        texts::{LanguageCatalog, english_labels, test_catalog},
    };

    /// The area of the question in the tests.
    const AREA: Rect = MINIMUM_SCREEN;

    /// The game file of the PC-98 game `name`.
    fn game(name: &str) -> EditedFile {
        EditedFile::Game {
            system: system_named("pc98"),
            name: name.to_owned(),
        }
    }

    /// The game file of Edge.
    fn edge() -> EditedFile {
        game("Edge")
    }

    /// Returns the question that one pass of the question about `subject` draws.
    fn drawn_message(subject: EditedFile) -> Option<String> {
        let mut question = DiscardQuestion::new(subject);
        GuiHarness::minimum().frame(|context| question.show(context, AREA));
        question.message().map(str::to_owned)
    }

    /// Verifies the question about the system defaults and about the settings.
    #[test]
    fn the_question_reads_the_subject_of_the_file() {
        assert_eq!(
            drawn_message(EditedFile::SystemDefaults(system_named("pc98"))).as_deref(),
            Some("The changes to the PC-98 system defaults are not saved.")
        );
        assert_eq!(
            drawn_message(EditedFile::Settings).as_deref(),
            Some("The changes to the settings are not saved.")
        );
    }

    /// The German text of the question about a game.
    const GERMAN_DISCARD: &str = "[discard]\n\
                                  game = Die Änderungen an {game} sind nicht gespeichert.\n";

    /// Verifies that a switch of the catalog while the question is open gives the drawn question the new language in the next pass.
    #[test]
    fn an_open_question_follows_a_catalog_switch() {
        let mut question = DiscardQuestion::new(edge());
        let mut harness = GuiHarness::minimum();
        harness.settle(|context| question.show(context, AREA));
        assert_eq!(
            question.message(),
            Some("The changes to Edge are not saved.")
        );
        harness.set_catalog(LanguageCatalog::Loaded(Box::new(test_catalog(
            GERMAN_DISCARD,
        ))));
        assert_eq!(
            harness.frame(|context| question.show(context, AREA)),
            DiscardOutcome::Open
        );
        assert_eq!(
            question.message(),
            Some("Die Änderungen an Edge sind nicht gespeichert.")
        );
    }

    /// Verifies the texts of the question, that Keep editing holds the focus first and that each answer closes it.
    #[test]
    fn keep_editing_holds_the_focus_and_discard_drops_the_changes() {
        let mut question = DiscardQuestion::new(game(
            "YU-NO: A Girl Who Chants Love at the Bound of this World",
        ));
        let mut harness = GuiHarness::minimum();
        assert_eq!(
            harness.settle(|context| question.show(context, AREA)),
            DiscardOutcome::Open
        );
        assert_eq!(
            question.message(),
            Some(
                "The changes to YU-NO: A Girl Who Chants Love at the Bound of this World are not saved."
            )
        );
        assert_eq!(
            harness.state.focused(),
            Some(UiId::from(DISCARD_LAYER_ID).child("cancel"))
        );
        assert_eq!(english_labels(&question.hints()), ["Keep editing"]);
        harness.input.confirm = true;
        assert_eq!(
            harness.frame(|context| question.show(context, AREA)),
            DiscardOutcome::Kept
        );

        let mut question = DiscardQuestion::new(edge());
        harness.settle(|context| question.show(context, AREA));
        harness.input.cancel = true;
        assert_eq!(
            harness.frame(|context| question.show(context, AREA)),
            DiscardOutcome::Kept
        );

        let mut question = DiscardQuestion::new(edge());
        harness.settle(|context| question.show(context, AREA));
        harness.input.navigation = Some(Navigation::Right);
        harness.settle(|context| question.show(context, AREA));
        harness.input.confirm = true;
        assert_eq!(
            harness.frame(|context| question.show(context, AREA)),
            DiscardOutcome::Discarded
        );
    }
}
