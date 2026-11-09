//! The question before the delete of a game file.

use std::{convert::Infallible, path::PathBuf};

use gui::{
    Rect,
    component::{Component, Layer, Status, Update},
    ui::{
        CONFIRM_HINTS, ConfirmDialog, ConfirmLines, ConfirmResponse, Icon, TextBuffer, UiContext,
        confirm_hints,
    },
};
use i18n::Catalog;
use stack_vec::StackVec;

use super::rows::GameRow;
use crate::texts::{CONFIRM_KEYS, HintText, common, delete};

/// The id of the modal layer of the question.
pub(crate) const DELETE_LAYER_ID: &str = "delete";

/// What the user can answer to the question.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) enum DeleteMessage {
    /// The user keeps the file.
    Keep,
    /// The user confirms the delete.
    Delete,
}

/// The question before the delete of one game file. It ends with `Done`
/// when the user confirms the delete and with `Closed` when the user keeps
/// the file. The owner removes the file.
#[derive(Debug, Clone)]
pub(crate) struct DeleteRequest {
    /// The message buffer.
    messages: Vec<DeleteMessage>,
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
    /// The answer of the dialog in the last pass, for the hints.
    response: ConfirmResponse,
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
            messages: Vec::new(),
            name: row.name().to_owned(),
            path: row.path.clone(),
            shown_path: row.shown_path.clone(),
            message: TextBuffer::new(),
            detail: TextBuffer::new(),
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

    /// The note below the question that the last pass drew. A question
    /// without a pass has none.
    #[cfg(test)]
    pub(crate) fn detail(&self) -> Option<&str> {
        self.detail.last_text()
    }

    /// The hints of the status line for the answer of the dialog in the
    /// last pass, in the order of the hints of a confirm dialog.
    pub(crate) fn hints(&self) -> StackVec<HintText, CONFIRM_HINTS> {
        confirm_hints(
            self.response,
            CONFIRM_KEYS,
            common::CANCEL,
            common::DELETE,
            HintText::new,
        )
    }
}

impl Component for DeleteRequest {
    type Message = DeleteMessage;
    type Output = ();
    type Effect = Infallible;
    type Input<'a> = ();

    const LAYER: Layer = Layer::Widget(DELETE_LAYER_ID);

    fn messages(&mut self) -> &mut Vec<DeleteMessage> {
        &mut self.messages
    }

    /// Draws one pass of the question centered in `area`.
    fn view(
        &mut self,
        context: &mut UiContext,
        area: Rect,
        _input: (),
        messages: &mut Vec<DeleteMessage>,
    ) {
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
        self.response = dialog(catalog, message, detail, area).show(context, lines);
        if self.response.confirmed {
            messages.push(DeleteMessage::Delete);
        } else if self.response.cancelled {
            messages.push(DeleteMessage::Keep);
        }
    }

    fn update(
        &mut self,
        message: DeleteMessage,
        _update: &mut Update<'_, Infallible>,
    ) -> Status<()> {
        match message {
            DeleteMessage::Keep => Status::Closed,
            DeleteMessage::Delete => Status::Done(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use ::common::temporary::TemporaryFolder;
    use gui::{
        component::run,
        ui::{Navigation, UiId},
    };

    use super::*;
    use crate::{
        gui_test_support::{GuiHarness, MINIMUM_SCREEN, apply},
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
    const AREA: Rect = MINIMUM_SCREEN;

    /// Draws the question in passes until it settles.
    fn draw(harness: &mut GuiHarness, request: &mut DeleteRequest) -> Status<()> {
        harness.settle(|context| pass(request, context))
    }

    /// Draws one pass of the question with an answer key in the input.
    fn answer(harness: &mut GuiHarness, request: &mut DeleteRequest) -> Status<()> {
        harness.frame(|context| pass(request, context))
    }

    /// Runs one pass of the question.
    fn pass(request: &mut DeleteRequest, context: &mut UiContext) -> Status<()> {
        run(request, context, AREA, (), &mut |_| {})
    }

    /// Verifies that keeping the file closes the question and that the delete ends it with a
    /// result.
    #[test]
    fn keep_closes_and_delete_ends_with_a_result() {
        let mut request = DeleteRequest::new(&game("edge", "Edge", "", "", ""));
        assert_eq!(
            apply(&mut request, DeleteMessage::Keep),
            (Status::Closed, Vec::new())
        );
        assert_eq!(
            apply(&mut request, DeleteMessage::Delete),
            (Status::Done(()), Vec::new())
        );
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
        let mut harness = GuiHarness::minimum();
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
        let mut harness = GuiHarness::minimum();
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
        assert_eq!(answer(&mut harness, &mut request), Status::Open);
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
        let mut harness = GuiHarness::minimum();
        assert_eq!(draw(&mut harness, &mut request), Status::Open);
        assert!(!request.response.action_focused);
        assert_eq!(english_labels(&request.hints()), ["Cancel"]);
        harness.input.confirm = true;
        assert_eq!(answer(&mut harness, &mut request), Status::Closed);
        assert!(folder.folder.path().join("edge.conf").is_file());

        draw(&mut harness, &mut request);
        harness.input.cancel = true;
        assert_eq!(answer(&mut harness, &mut request), Status::Closed);
        assert!(folder.folder.path().join("edge.conf").is_file());
    }

    /// Verifies that confirm on the action ends the question with the delete and leaves the
    /// removal of the file to the owner.
    #[test]
    fn confirm_asks_for_the_delete() {
        let folder = TestFolder::new("confirm");
        let mut request = folder.request();
        let mut harness = GuiHarness::minimum();
        draw(&mut harness, &mut request);
        harness.input.navigation = Some(Navigation::Right);
        draw(&mut harness, &mut request);
        assert!(request.response.action_focused);
        assert_eq!(
            harness.state.focused(),
            Some(UiId::from(DELETE_LAYER_ID).child("action"))
        );
        assert_eq!(english_labels(&request.hints()), ["Delete", "Cancel"]);
        harness.input.confirm = true;
        assert_eq!(answer(&mut harness, &mut request), Status::Done(()));
        assert!(folder.folder.path().join("edge.conf").is_file());
    }
}
