//! The delete of a game file: the question before it and the removal of the
//! file.

use std::path::PathBuf;

use gui::{
    Rect,
    ui::{ConfirmDialog, ConfirmResponse, Hint, Icon, UiContext},
};

use super::rows::GameRow;

/// The id of the modal layer of the question.
pub(crate) const DELETE_LAYER_ID: &str = "delete";

/// The title of the question.
const DELETE_TITLE: &str = "Delete game";

/// The caption of the action of the question.
const DELETE_ACTION: &str = "Delete";

/// The answer of the question in a pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DeleteOutcome {
    /// The question stays open.
    Open,
    /// The user kept the file.
    Kept,
    /// The file was removed.
    Deleted,
    /// The removal failed with this error.
    Failed(String),
}

/// The question before the delete of one game file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeleteRequest {
    /// The name of the game in the question.
    pub(crate) name: String,
    /// The absolute path of the game file.
    pub(crate) path: PathBuf,
    /// The path of the game file to show.
    pub(crate) shown_path: String,
    /// The question.
    message: String,
    /// The note below the question.
    detail: String,
}

impl DeleteRequest {
    /// The question before the delete of `row`.
    pub(crate) fn new(row: &GameRow) -> Self {
        Self {
            name: row.name().to_owned(),
            path: row.path.clone(),
            shown_path: row.shown_path.clone(),
            message: format!("Delete {}?", row.name()),
            detail: format!(
                "The file {} is removed. The disk images stay where they are.",
                row.shown_path
            ),
        }
    }

    /// The question.
    #[cfg(test)]
    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    /// The note below the question.
    #[cfg(test)]
    pub(crate) fn detail(&self) -> &str {
        &self.detail
    }

    /// The confirm dialog of the question, centered in `area`.
    fn dialog(&self, area: Rect) -> ConfirmDialog<'_> {
        ConfirmDialog::new(DELETE_LAYER_ID, DELETE_TITLE, &self.message, DELETE_ACTION)
            .detail(&self.detail)
            .action_icon(Icon::Delete)
            .danger(true)
            .area(area)
    }

    /// The hints of the status line while the question shows the answer
    /// `response` of this pass.
    pub(crate) fn hints(&self, response: ConfirmResponse) -> Vec<Hint<'_>> {
        self.dialog(Rect::default()).hints(response)
    }

    /// Draws one pass of the question centered in `area`. A confirm removes
    /// the game file. Returns the answer of the pass and the answer of the
    /// dialog for the hints.
    pub(crate) fn show(
        &self,
        context: &mut UiContext,
        area: Rect,
    ) -> (DeleteOutcome, ConfirmResponse) {
        let response = self.dialog(area).show(context);
        let outcome = if response.confirmed {
            match std::fs::remove_file(&self.path) {
                Ok(()) => DeleteOutcome::Deleted,
                Err(error) => DeleteOutcome::Failed(format!("{}: {error}", self.shown_path)),
            }
        } else if response.cancelled {
            DeleteOutcome::Kept
        } else {
            DeleteOutcome::Open
        };
        (outcome, response)
    }
}

#[cfg(test)]
mod tests {
    use common::temporary::TemporaryFolder;
    use gui::ui::{Navigation, UiId};

    use super::*;
    use crate::{gui_test_support::GuiHarness, manager::rows::tests::game};

    /// A folder with one game file. Dropping the value removes it.
    struct TestFolder {
        /// The folder.
        folder: TemporaryFolder,
    }

    impl TestFolder {
        /// Creates a folder from `name` with the game file `edge.conf`.
        fn new(name: &str) -> Self {
            let folder = TemporaryFolder::new(&format!("manager_delete_{name}"));
            folder.write_file("edge.conf", b"meta.title = Edge\n");
            Self { folder }
        }

        /// The request for the game file of the folder.
        fn request(&self) -> DeleteRequest {
            let row = game("edge", "Edge", "", "", "");
            DeleteRequest::new(&GameRow {
                path: self.folder.path().join("edge.conf"),
                ..row
            })
        }
    }

    /// The area of the question in the tests.
    const AREA: Rect = Rect::new(0.0, 0.0, 960.0, 572.0);

    /// Draws the question in passes until it settles.
    fn draw(harness: &mut GuiHarness, request: &DeleteRequest) -> (DeleteOutcome, ConfirmResponse) {
        harness.settle(|context| request.show(context, AREA))
    }

    /// Draws one pass of the question with an answer key in the input.
    fn answer(harness: &mut GuiHarness, request: &DeleteRequest) -> DeleteOutcome {
        harness.frame(|context| request.show(context, AREA)).0
    }

    /// Verifies the texts of the question for a game.
    #[test]
    fn the_question_shows_the_title_and_the_file() {
        let request = DeleteRequest::new(&game(
            "yu-no",
            "YU-NO: A Girl Who Chants Love at the Bound of this World",
            "",
            "",
            "",
        ));
        assert_eq!(
            request.message(),
            "Delete YU-NO: A Girl Who Chants Love at the Bound of this World?"
        );
        assert!(request.detail().starts_with("The file config/"));
        assert!(
            request
                .detail()
                .ends_with("/yu-no.conf is removed. The disk images stay where they are.")
        );
    }

    /// Verifies that the question starts on cancel and that cancel keeps the file.
    #[test]
    fn cancel_keeps_the_file() {
        let folder = TestFolder::new("cancel");
        let request = folder.request();
        let mut harness = GuiHarness::new(960, 600);
        let (outcome, response) = draw(&mut harness, &request);
        assert_eq!(outcome, DeleteOutcome::Open);
        assert!(!response.action_focused);
        assert_eq!(request.hints(response).len(), 1);
        harness.input.confirm = true;
        assert_eq!(answer(&mut harness, &request), DeleteOutcome::Kept);
        assert!(folder.folder.path().join("edge.conf").is_file());

        draw(&mut harness, &request);
        harness.input.cancel = true;
        assert_eq!(answer(&mut harness, &request), DeleteOutcome::Kept);
        assert!(folder.folder.path().join("edge.conf").is_file());
    }

    /// Verifies that confirm on the action removes the file.
    #[test]
    fn confirm_removes_the_file() {
        let folder = TestFolder::new("confirm");
        let request = folder.request();
        let mut harness = GuiHarness::new(960, 600);
        draw(&mut harness, &request);
        harness.input.navigation = Some(Navigation::Right);
        let (_, response) = draw(&mut harness, &request);
        assert!(response.action_focused);
        assert_eq!(
            harness.state.focused(),
            Some(UiId::from(DELETE_LAYER_ID).child("action"))
        );
        assert_eq!(request.hints(response).len(), 2);
        harness.input.confirm = true;
        assert_eq!(answer(&mut harness, &request), DeleteOutcome::Deleted);
        assert!(!folder.folder.path().join("edge.conf").exists());
    }

    /// Verifies that the delete of a game file that is a symbolic link removes the link and keeps its target.
    #[cfg(unix)]
    #[test]
    fn a_link_loses_only_itself() {
        let folder = TestFolder::new("link");
        let target = folder.folder.path().join("target.conf");
        std::fs::rename(folder.folder.path().join("edge.conf"), &target)
            .expect("the file is renamed");
        std::os::unix::fs::symlink(&target, folder.folder.path().join("edge.conf")).expect("link");
        let request = folder.request();
        let mut harness = GuiHarness::new(960, 600);
        draw(&mut harness, &request);
        harness.input.navigation = Some(Navigation::Right);
        draw(&mut harness, &request);
        harness.input.confirm = true;
        assert_eq!(answer(&mut harness, &request), DeleteOutcome::Deleted);
        assert!(std::fs::symlink_metadata(folder.folder.path().join("edge.conf")).is_err());
        assert!(target.is_file());
    }

    /// Verifies that a file that cannot be removed reports the error with its path.
    #[test]
    fn a_failed_removal_reports_the_error() {
        let folder = TestFolder::new("failed");
        let request = folder.request();
        std::fs::remove_file(&request.path).expect("the file is removed before");
        let mut harness = GuiHarness::new(960, 600);
        draw(&mut harness, &request);
        harness.input.navigation = Some(Navigation::Right);
        draw(&mut harness, &request);
        harness.input.confirm = true;
        match answer(&mut harness, &request) {
            DeleteOutcome::Failed(error) => {
                assert!(error.starts_with(&request.shown_path), "{error}");
            }
            outcome => panic!("the removal fails: {outcome:?}"),
        }
    }
}
