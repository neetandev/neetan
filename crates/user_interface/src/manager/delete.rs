//! The delete of a game file: the question before it and the removal of the
//! file.

use std::path::PathBuf;

use gui::{
    Rect,
    ui::{
        CONFIRM_HINTS, ConfirmDialog, ConfirmLines, ConfirmResponse, Icon, TextBuffer, UiContext,
        confirm_hints,
    },
};
use i18n::Catalog;
use stack_vec::StackVec;

use super::rows::GameRow;
use crate::texts::{HintText, common, delete};

/// The id of the modal layer of the question.
pub(crate) const DELETE_LAYER_ID: &str = "delete";

/// The answer of the question in a pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DeleteOutcome {
    /// The question stays open.
    Open,
    /// The user kept the file.
    Kept,
    /// The file was removed.
    Deleted,
    /// The removal of the file failed.
    Failed {
        /// The shown path of the game file.
        path: String,
        /// The error of the operating system in English.
        error: String,
    },
}

/// The question before the delete of one game file.
#[derive(Debug, Clone)]
pub(crate) struct DeleteRequest {
    /// The name of the game in the question.
    pub(crate) name: String,
    /// The absolute path of the game file.
    pub(crate) path: PathBuf,
    /// The path of the game file to show.
    pub(crate) shown_path: String,
    /// The question in the catalog of the last pass.
    message: TextBuffer<()>,
    /// The note below the question in the catalog of the last pass.
    detail: TextBuffer<()>,
    /// The wrapped lines of the question and of the note.
    lines: ConfirmLines,
}

/// The confirm dialog of the question `message` with the note `detail` in
/// the texts of `catalog`, centered in `area`.
fn dialog<'a>(
    catalog: &'a Catalog,
    message: &'a str,
    detail: &'a str,
    area: Rect,
) -> ConfirmDialog<'a> {
    ConfirmDialog::new(
        DELETE_LAYER_ID,
        catalog.text(delete::TITLE),
        message,
        catalog.text(common::CANCEL),
        catalog.text(common::DELETE),
    )
    .detail(detail)
    .action_icon(Icon::Delete)
    .danger(true)
    .area(area)
}

impl DeleteRequest {
    /// The question before the delete of `row`.
    pub(crate) fn new(row: &GameRow) -> Self {
        Self {
            name: row.name().to_owned(),
            path: row.path.clone(),
            shown_path: row.shown_path.clone(),
            message: TextBuffer::new(),
            detail: TextBuffer::new(),
            lines: ConfirmLines::new(),
        }
    }

    /// The question that the last pass drew. A question without a pass has
    /// none.
    #[cfg(test)]
    pub(crate) fn message(&self) -> Option<&str> {
        self.message.last_text()
    }

    /// The note below the question that the last pass drew. A question
    /// without a pass has none.
    #[cfg(test)]
    pub(crate) fn detail(&self) -> Option<&str> {
        self.detail.last_text()
    }

    /// The hints of the status line while the question shows the answer
    /// `response` of this pass, in the order of the hints of a confirm
    /// dialog.
    pub(crate) fn hints(&self, response: ConfirmResponse) -> StackVec<HintText, CONFIRM_HINTS> {
        confirm_hints(response, common::CANCEL, common::DELETE, HintText::new)
    }

    /// Draws one pass of the question centered in `area`. A confirm removes
    /// the game file. Returns the answer of the pass and the answer of the
    /// dialog for the hints.
    pub(crate) fn show(
        &mut self,
        context: &mut UiContext,
        area: Rect,
    ) -> (DeleteOutcome, ConfirmResponse) {
        let catalog = context.catalog;
        let Self {
            name,
            shown_path,
            message,
            detail,
            lines,
            ..
        } = self;
        let message = message.text(context, (), |catalog, (), out| {
            let _ = catalog.write(out, delete::question(name));
        });
        let detail = detail.text(context, (), |catalog, (), out| {
            let _ = catalog.write(out, delete::detail(shown_path));
        });
        let response = dialog(catalog, message, detail, area).show(context, lines);
        let outcome = if response.confirmed {
            match std::fs::remove_file(&self.path) {
                Ok(()) => DeleteOutcome::Deleted,
                Err(error) => DeleteOutcome::Failed {
                    path: self.shown_path.clone(),
                    error: error.to_string(),
                },
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
    use ::common::temporary::TemporaryFolder;
    use gui::ui::{Navigation, UiId};

    use super::*;
    use crate::{
        gui_test_support::GuiHarness,
        manager::rows::tests::game,
        texts::{LanguageCatalog, english_labels, test_catalog},
    };

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
    fn draw(
        harness: &mut GuiHarness,
        request: &mut DeleteRequest,
    ) -> (DeleteOutcome, ConfirmResponse) {
        harness.settle(|context| request.show(context, AREA))
    }

    /// Draws one pass of the question with an answer key in the input.
    fn answer(harness: &mut GuiHarness, request: &mut DeleteRequest) -> DeleteOutcome {
        harness.frame(|context| request.show(context, AREA)).0
    }

    /// Verifies the texts of the question for a game.
    #[test]
    fn the_question_shows_the_title_and_the_file() {
        let mut request = DeleteRequest::new(&game(
            "yu-no",
            "YU-NO: A Girl Who Chants Love at the Bound of this World",
            "",
            "",
            "",
        ));
        assert_eq!(request.message(), None);
        let mut harness = GuiHarness::new(960, 600);
        draw(&mut harness, &mut request);
        assert_eq!(
            request.message(),
            Some("Delete YU-NO: A Girl Who Chants Love at the Bound of this World?")
        );
        let detail = request.detail().unwrap_or_default();
        assert!(detail.starts_with("The file config/"), "{detail}");
        assert!(
            detail.ends_with("/yu-no.conf is removed. The disk images stay where they are."),
            "{detail}"
        );
    }

    /// The German texts of the question.
    const GERMAN_DELETE: &str = "[delete]\n\
                                 question = {name} löschen?\n\
                                 detail = Die Datei {path} wird entfernt.\n";

    /// Verifies that a switch of the catalog while the question is open gives the drawn question and note the new language in the next pass.
    #[test]
    fn an_open_question_follows_a_catalog_switch() {
        let folder = TestFolder::new("catalog_switch");
        let mut request = folder.request();
        let mut harness = GuiHarness::new(960, 600);
        draw(&mut harness, &mut request);
        assert_eq!(request.message(), Some("Delete Edge?"));
        let english_detail = format!(
            "The file {} is removed. The disk images stay where they are.",
            request.shown_path
        );
        assert_eq!(request.detail(), Some(english_detail.as_str()));
        harness.set_catalog(LanguageCatalog::Loaded(Box::new(test_catalog(
            GERMAN_DELETE,
        ))));
        assert_eq!(answer(&mut harness, &mut request), DeleteOutcome::Open);
        assert_eq!(request.message(), Some("Edge löschen?"));
        let german_detail = format!("Die Datei {} wird entfernt.", request.shown_path);
        assert_eq!(request.detail(), Some(german_detail.as_str()));
        assert!(folder.folder.path().join("edge.conf").is_file());
    }

    /// Verifies that the question starts on cancel and that cancel keeps the file.
    #[test]
    fn cancel_keeps_the_file() {
        let folder = TestFolder::new("cancel");
        let mut request = folder.request();
        let mut harness = GuiHarness::new(960, 600);
        let (outcome, response) = draw(&mut harness, &mut request);
        assert_eq!(outcome, DeleteOutcome::Open);
        assert!(!response.action_focused);
        assert_eq!(english_labels(&request.hints(response)), ["Cancel"]);
        harness.input.confirm = true;
        assert_eq!(answer(&mut harness, &mut request), DeleteOutcome::Kept);
        assert!(folder.folder.path().join("edge.conf").is_file());

        draw(&mut harness, &mut request);
        harness.input.cancel = true;
        assert_eq!(answer(&mut harness, &mut request), DeleteOutcome::Kept);
        assert!(folder.folder.path().join("edge.conf").is_file());
    }

    /// Verifies that confirm on the action removes the file.
    #[test]
    fn confirm_removes_the_file() {
        let folder = TestFolder::new("confirm");
        let mut request = folder.request();
        let mut harness = GuiHarness::new(960, 600);
        draw(&mut harness, &mut request);
        harness.input.navigation = Some(Navigation::Right);
        let (_, response) = draw(&mut harness, &mut request);
        assert!(response.action_focused);
        assert_eq!(
            harness.state.focused(),
            Some(UiId::from(DELETE_LAYER_ID).child("action"))
        );
        assert_eq!(
            english_labels(&request.hints(response)),
            ["Delete", "Cancel"]
        );
        harness.input.confirm = true;
        assert_eq!(answer(&mut harness, &mut request), DeleteOutcome::Deleted);
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
        let mut request = folder.request();
        let mut harness = GuiHarness::new(960, 600);
        draw(&mut harness, &mut request);
        harness.input.navigation = Some(Navigation::Right);
        draw(&mut harness, &mut request);
        harness.input.confirm = true;
        assert_eq!(answer(&mut harness, &mut request), DeleteOutcome::Deleted);
        assert!(std::fs::symlink_metadata(folder.folder.path().join("edge.conf")).is_err());
        assert!(target.is_file());
    }

    /// Verifies that a file that cannot be removed reports the error with its path.
    #[test]
    fn a_failed_removal_reports_the_error() {
        let folder = TestFolder::new("failed");
        let mut request = folder.request();
        std::fs::remove_file(&request.path).expect("the file is removed before");
        let mut harness = GuiHarness::new(960, 600);
        draw(&mut harness, &mut request);
        harness.input.navigation = Some(Navigation::Right);
        draw(&mut harness, &mut request);
        harness.input.confirm = true;
        match answer(&mut harness, &mut request) {
            DeleteOutcome::Failed { path, error } => {
                assert_eq!(path, request.shown_path);
                assert!(!error.is_empty());
            }
            outcome => panic!("the removal fails: {outcome:?}"),
        }
    }
}
