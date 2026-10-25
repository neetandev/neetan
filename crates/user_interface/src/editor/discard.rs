//! The question before the editor closes with unsaved changes.

use gui::{
    Rect,
    ui::{ConfirmDialog, ConfirmResponse, Hint, Icon, UiContext},
};

use super::EditedFile;

/// The id of the modal layer of the question.
pub(crate) const DISCARD_LAYER_ID: &str = "discard";

/// The title of the question.
const DISCARD_TITLE: &str = "Discard changes";

/// The caption of the action of the question.
const DISCARD_ACTION: &str = "Discard";

/// The caption of the button that keeps the editor open.
const KEEP_EDITING: &str = "Keep editing";

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

/// The question before the changes to one file go away.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiscardQuestion {
    /// The question.
    message: String,
    /// The answer of the last pass, for the hints.
    response: ConfirmResponse,
}

impl DiscardQuestion {
    /// The question about the changes to the file of `subject`.
    pub(crate) fn new(subject: &EditedFile) -> Self {
        Self {
            message: format!("The changes to {} are not saved.", subject.text()),
            response: ConfirmResponse::default(),
        }
    }

    /// The question.
    #[cfg(test)]
    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    /// The confirm dialog of the question, centered in `area`.
    fn dialog(&self, area: Rect) -> ConfirmDialog<'_> {
        ConfirmDialog::new(
            DISCARD_LAYER_ID,
            DISCARD_TITLE,
            &self.message,
            DISCARD_ACTION,
        )
        .cancel_caption(KEEP_EDITING)
        .action_icon(Icon::Delete)
        .danger(true)
        .area(area)
    }

    /// The hints of the status line for the answer of the last pass.
    pub(crate) fn hints(&self) -> Vec<Hint<'static>> {
        ConfirmDialog::new(DISCARD_LAYER_ID, DISCARD_TITLE, "", DISCARD_ACTION)
            .cancel_caption(KEEP_EDITING)
            .danger(true)
            .hints(self.response)
    }

    /// Draws one pass of the question centered in `area` and returns its answer.
    pub(crate) fn show(&mut self, context: &mut UiContext, area: Rect) -> DiscardOutcome {
        self.response = self.dialog(area).show(context);
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
    use crate::{editor::test_support::system_named, gui_test_support::GuiHarness};

    /// The area of the question in the tests.
    const AREA: Rect = Rect::new(0.0, 0.0, 960.0, 572.0);

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

    /// Verifies the question about the system defaults and about the settings.
    #[test]
    fn the_question_reads_the_subject_of_the_file() {
        assert_eq!(
            DiscardQuestion::new(&EditedFile::SystemDefaults(system_named("pc98"))).message(),
            "The changes to the PC-98 system defaults are not saved."
        );
        assert_eq!(
            DiscardQuestion::new(&EditedFile::Settings).message(),
            "The changes to the settings are not saved."
        );
    }

    /// Verifies the texts of the question, that Keep editing holds the focus first and that each answer closes it.
    #[test]
    fn keep_editing_holds_the_focus_and_discard_drops_the_changes() {
        let mut question = DiscardQuestion::new(&game(
            "YU-NO: A Girl Who Chants Love at the Bound of this World",
        ));
        assert_eq!(
            question.message(),
            "The changes to YU-NO: A Girl Who Chants Love at the Bound of this World are not saved."
        );
        let mut harness = GuiHarness::new(960, 600);
        assert_eq!(
            harness.settle(|context| question.show(context, AREA)),
            DiscardOutcome::Open
        );
        assert_eq!(
            harness.state.focused(),
            Some(UiId::from(DISCARD_LAYER_ID).child("cancel"))
        );
        assert_eq!(
            question
                .hints()
                .iter()
                .map(|hint| hint.label)
                .collect::<Vec<_>>(),
            ["Keep editing"]
        );
        harness.input.confirm = true;
        assert_eq!(
            harness.frame(|context| question.show(context, AREA)),
            DiscardOutcome::Kept
        );

        let mut question = DiscardQuestion::new(&edge());
        harness.settle(|context| question.show(context, AREA));
        harness.input.cancel = true;
        assert_eq!(
            harness.frame(|context| question.show(context, AREA)),
            DiscardOutcome::Kept
        );

        let mut question = DiscardQuestion::new(&edge());
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
