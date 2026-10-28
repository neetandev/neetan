//! The create image dialog: a modal that creates a blank floppy or hard disk image for a drive
//! field of the editor.
//!
//! The dialog shows the folder of the new image with a Browse button, the list of the image
//! types that the model of the form takes, the name of the file and a line with the state of
//! the name. The name starts free in the folder and follows the extension of the selected type
//! until the user edits it. Create writes the image and gives its path, relative to the root
//! below the root and absolute elsewhere. An existing file stays untouched.

mod image_name;
mod type_rows;

use std::path::{Path, PathBuf};

use gui::{
    Rect,
    ui::{
        Align, Button, ButtonKind, Icon, List, ListState, PATH_LINE_HEIGHT, RowContent, TextBuffer,
        TextField, Truncation, UiContext, UiId, WrappedText, distinct_hints, label, modal_in,
    },
};
use i18n::{Catalog, ListKind};
use machine_factory::config::settings::{
    PathKind, Setting, ValueKind,
    blank_images::{BlankImage, BlankImages, blank_images_for, has_blank_images},
    file_types::FileTypes,
};
use stack_vec::StackVec;

use self::{
    image_name::{NameCheck, check_name, default_name, followed_name, name_stem},
    type_rows::{DottedExtension, write_type_row},
};
use super::{BrowseTarget, Editor, media_list::CreateButton, notes::NoteKind, rows};
use crate::{
    config::resolve_path,
    create::create_blank_image,
    file_browser::{BrowseNoun, BrowseRequest, BrowserOutcome, FileBrowser, path_value},
    media_dialog::{PANEL_SIZE, TAB_BAR_HEIGHT},
    texts::{
        ACTIVATE_KEYS, CANCEL_KEYS, HintText, HintTexts, MAXIMUM_HINTS, common, create_image,
        setting_label,
    },
};

/// The id of the modal layer of the dialog.
pub(crate) const CREATE_LAYER_ID: &str = "create image";

/// The id of the Browse button of the folder.
const FOLDER_ID: &str = "create image folder";

/// The id of the list of the image types.
const TYPE_LIST_ID: &str = "create image types";

/// The id of the name field.
const NAME_ID: &str = "create image name";

/// The id of the cancel button.
const CANCEL_ID: &str = "create image cancel";

/// The id of the create button.
const CREATE_ID: &str = "create image create";

/// The request of a drive field for a blank image.
#[derive(Debug, Clone)]
pub(crate) struct CreateRequest {
    /// The setting of the drive. The title shows its label, for example `HDD 1`.
    pub(crate) setting: &'static Setting,
    /// The files of the drive.
    pub(crate) types: &'static FileTypes,
    /// The blank images of the drive on the model of the form.
    pub(crate) images: &'static BlankImages,
    /// The stem of the default name, for example `yu-no-hdd1`.
    pub(crate) stem: String,
    /// The absolute folder the dialog starts in.
    pub(crate) folder: PathBuf,
}

/// The answer of the dialog in a pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CreateOutcome {
    /// The dialog stays open.
    Open,
    /// The dialog closes without an image.
    Cancelled,
    /// The image was created. The value is its path below the root relative to the root, any
    /// other path absolute.
    Created(String),
}

/// The result of a hint of the dialog. Two hints of one action lead to one result, and the
/// status line shows only the first of them.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum CreateAction {
    /// The image is created.
    Create,
    /// The folder browser opens.
    Browse,
    /// The dialog closes without an image.
    Close,
}

/// A hint with its action.
type ActionHint = (CreateAction, HintText);

/// The hint of a create from the type list, the name field and the create button.
const CREATE_HINT: ActionHint = (
    CreateAction::Create,
    HintText::new(ACTIVATE_KEYS, common::CREATE),
);

/// The hint of the Browse button of the folder.
const BROWSE_HINT: ActionHint = (
    CreateAction::Browse,
    HintText::new(ACTIVATE_KEYS, common::BROWSE),
);

/// The hint of the cancel button.
const CANCEL_HINT: ActionHint = (
    CreateAction::Close,
    HintText::new(ACTIVATE_KEYS, common::CANCEL),
);

/// The hint of the close of the dialog.
const CLOSE_HINT: ActionHint = (
    CreateAction::Close,
    HintText::new(CANCEL_KEYS, common::CLOSE),
);

/// The texts of the dialog that refill only when their data or the catalog changes.
#[derive(Debug, Clone, Default)]
struct CreateTexts {
    /// The title of the panel.
    title: TextBuffer<()>,
    /// The text of each type row, in the order of the images.
    rows: Vec<TextBuffer<()>>,
    /// The line below the name, for the check and the count of the changes of the state.
    message: TextBuffer<(NameCheck, u64)>,
}

/// The open create image dialog.
#[derive(Debug, Clone)]
pub(crate) struct CreateImageDialog {
    /// The request of the drive field.
    request: CreateRequest,
    /// The root folder.
    root: PathBuf,
    /// The absolute folder of the new image.
    folder: PathBuf,
    /// The file name of the new image.
    name: String,
    /// Whether the user edited the name.
    name_edited: bool,
    /// The selection and the scroll position of the type list.
    list: ListState,
    /// The state of the name in the folder.
    check: NameCheck,
    /// The error of the last failed write. The next change clears it.
    failure: Option<String>,
    /// The count of the changes of the check and the failure. The message refills when it
    /// changes.
    revision: u64,
    /// The open folder browser.
    browser: Option<FileBrowser>,
    /// The texts of the panel.
    texts: CreateTexts,
    /// The wrapped lines of a message with a fixed text. A message with a file name or an error
    /// keeps to one line and gives way in the middle.
    message_lines: WrappedText,
    /// The hints of the status line of the last pass.
    hints: HintTexts,
    /// Whether the state changed after the panel drew it, so the pass draws stale widgets.
    stale: bool,
}

impl CreateImageDialog {
    /// A dialog for `request` below `root`, with the default image of the drive selected and its
    /// free default name in the start folder.
    pub(crate) fn new(request: CreateRequest, root: &Path) -> Self {
        let mut list = ListState::new();
        list.select(request.images.default);
        let texts = CreateTexts {
            rows: vec![TextBuffer::new(); request.images.images.len()],
            ..CreateTexts::default()
        };
        let mut dialog = Self {
            folder: request.folder.clone(),
            request,
            root: root.to_path_buf(),
            name: String::new(),
            name_edited: false,
            list,
            check: NameCheck::Valid,
            failure: None,
            revision: 0,
            browser: None,
            texts,
            message_lines: WrappedText::default(),
            hints: HintTexts::new(),
            stale: false,
        };
        dialog.name = dialog.default_name();
        dialog.run_check();
        dialog
    }

    /// The hints of the status line of the last pass. The open folder browser gives its own.
    pub(crate) fn hints(&self) -> &HintTexts {
        self.browser
            .as_ref()
            .map_or(&self.hints, |browser| browser.hints())
    }

    /// The selected image.
    fn selected_image(&self) -> &'static BlankImage {
        let images = self.request.images.images;
        &images[self.list.selected.min(images.len() - 1)]
    }

    /// The default name of the selected image in the folder.
    fn default_name(&self) -> String {
        default_name(&self.folder, &self.request.stem, self.selected_image())
    }

    /// Checks the name again and clears the failure of the last write.
    fn run_check(&mut self) {
        self.check = check_name(&self.name, &self.folder, self.selected_image());
        self.failure = None;
        self.revision += 1;
        self.stale = true;
    }

    /// Answers a change of the selected type: an unedited name takes the default name of the
    /// new type, an edited name follows its extension.
    fn type_changed(&mut self) {
        if !self.name_edited {
            self.name = self.default_name();
        } else if let Some(name) = followed_name(&self.name, self.selected_image()) {
            self.name = name;
        }
        self.run_check();
    }

    /// Takes the folder `folder`. An unedited name takes the default name in the new folder.
    fn folder_changed(&mut self, folder: PathBuf) {
        self.folder = folder;
        if !self.name_edited {
            self.name = self.default_name();
        }
        self.run_check();
    }

    /// Checks the name once more and writes the image when the check passes.
    fn create(&mut self) -> CreateOutcome {
        self.run_check();
        if self.check != NameCheck::Valid {
            return CreateOutcome::Open;
        }
        self.write_image()
    }

    /// Writes the image of the selected type at the name in the folder. A failed write keeps the
    /// dialog open with the error.
    fn write_image(&mut self) -> CreateOutcome {
        let path = self.folder.join(self.name.trim());
        match create_blank_image(&path, self.selected_image().format) {
            Ok(()) => CreateOutcome::Created(path_value(&path, &self.root)),
            Err(error) => {
                self.failure = Some(format!("{error:#}"));
                self.revision += 1;
                self.stale = true;
                CreateOutcome::Open
            }
        }
    }

    /// The request of the folder browser, which starts at the folder of the dialog.
    fn folder_request(&self) -> BrowseRequest {
        BrowseRequest {
            noun: BrowseNoun::Folder,
            setting: Some(self.request.setting),
            kind: PathKind::Folder,
            types: None,
            icon: Icon::Folder,
            current: Some(path_value(&self.folder, &self.root)),
        }
    }

    /// Draws one pass of the dialog in its modal layer centered in `area`, with the open folder
    /// browser on top. Returns the answer of the pass.
    pub(crate) fn show(&mut self, context: &mut UiContext, area: Rect) -> CreateOutcome {
        let outcome = context.layer(CREATE_LAYER_ID, |context| {
            let outcome = self.panel(context, area);
            if outcome != CreateOutcome::Open {
                context.close_layer();
            }
            outcome
        });
        if outcome != CreateOutcome::Open {
            return outcome;
        }
        if let Some(mut browser) = self.browser.take() {
            match browser.show(context, area) {
                BrowserOutcome::Open => self.browser = Some(browser),
                BrowserOutcome::Cancelled => context.request_pass(),
                BrowserOutcome::Chosen(value) => {
                    self.folder_changed(resolve_path(&self.root, Path::new(&value)));
                    context.request_pass();
                }
            }
        }
        outcome
    }

    /// Draws the panel of the dialog centered in `area`.
    fn panel(&mut self, context: &mut UiContext, area: Rect) -> CreateOutcome {
        let outcome = self.draw_panel(context, area);
        if self.stale {
            self.stale = false;
            context.request_pass();
        }
        outcome
    }

    /// Writes the line below the name in `catalog`: the note of a blank image, the failure of
    /// the name check or the error of the last write.
    fn write_message(&self, catalog: &Catalog, out: &mut String) {
        if let Some(failure) = &self.failure {
            let _ = catalog.write(out, create_image::create_failed(failure));
            return;
        }
        match self.check {
            NameCheck::Valid => out.push_str(catalog.text(create_image::BLANK_NOTE)),
            NameCheck::EmptyName => out.push_str(catalog.text(create_image::EMPTY_NAME)),
            NameCheck::FolderInName => out.push_str(catalog.text(create_image::FOLDER_IN_NAME)),
            NameCheck::ReservedName => out.push_str(catalog.text(create_image::RESERVED_NAME)),
            NameCheck::WrongExtension => {
                let mut extensions: Vec<DottedExtension> = self
                    .selected_image()
                    .extensions
                    .iter()
                    .map(|extension| DottedExtension(extension))
                    .collect();
                extensions.sort_by_key(|extension| extension.0);
                let list = catalog.display_list(ListKind::Or, &extensions);
                let _ = catalog.write(out, create_image::wrong_extension(&list));
            }
            NameCheck::MissingFolder => out.push_str(catalog.text(create_image::MISSING_FOLDER)),
            NameCheck::FileExists => {
                let _ = catalog.write(out, create_image::file_exists(&self.name.trim()));
            }
        }
    }

    /// The text of the line below the name in English.
    #[cfg(test)]
    fn english_message(&self) -> String {
        crate::texts::english_text(|catalog, out| self.write_message(catalog, out))
    }

    /// Draws the widgets of the panel centered in `area` and returns the answer of the pass.
    fn draw_panel(&mut self, context: &mut UiContext, area: Rect) -> CreateOutcome {
        let theme = context.theme;
        let catalog = context.catalog;
        let gap = theme.gap;
        let list_id = UiId::from(TYPE_LIST_ID);
        let setting = self.request.setting;
        let mut texts = std::mem::take(&mut self.texts);
        let title = texts.title.text(context, (), |catalog, (), out| {
            let label = setting_label(catalog, setting);
            let _ = catalog.write(out, create_image::title(&label));
        });
        let response = modal_in(context, CREATE_LAYER_ID, area, PANEL_SIZE, title);
        let mut outcome = if response.closed {
            CreateOutcome::Cancelled
        } else {
            CreateOutcome::Open
        };

        let (bar, rest) = response.content.cut_top(TAB_BAR_HEIGHT);
        context.fill(
            Rect::new(bar.x, bar.bottom() - theme.stroke, bar.width, theme.stroke),
            theme.colors.panel_edge,
        );
        let browse = Button::new(FOLDER_ID, catalog.text(common::BROWSE));
        let (browse_slot, path_slot) = bar.cut_right(browse.width(context).min(bar.width));
        let browse_area = Rect::new(
            browse_slot.x,
            bar.y + (TAB_BAR_HEIGHT - theme.control_height) * 0.5,
            browse_slot.width,
            theme.control_height,
        );
        let (icon_area, path_area) = path_slot
            .cut_right(gap)
            .1
            .cut_left(theme.icon_size.min(path_slot.width));
        context.icon(
            icon_area,
            Icon::Folder,
            theme.icon_size,
            theme.colors.text_dim,
        );
        label(
            context,
            path_area.cut_left(gap).1,
            &theme.body(),
            &self.folder.to_string_lossy(),
            Align::Start,
            Truncation::Middle,
        );
        let browse_pressed = browse.show(context, browse_area);

        let rest = rest.cut_top(gap * 1.5).1;
        let (footer, rest) = rest.cut_bottom(theme.control_height);
        let (message_area, rest) = rest.cut_bottom(gap).1.cut_bottom(PATH_LINE_HEIGHT);
        let (name_area, rest) = rest.cut_bottom(gap).1.cut_bottom(theme.control_height);
        let list_area = rest.cut_bottom(gap * 1.5).1;

        let images = self.request.images;
        let icon = rows::image_icon(Some(self.request.types));
        context.default_focus(list_id);
        let mut list = List::new(TYPE_LIST_ID, images.images.len());
        if let Some(separator) = images.separator_after {
            list = list.separator_after(separator);
        }
        let row_texts = &mut texts.rows;
        let list_response = list.show(context, list_area, &mut self.list, |context, row| {
            let text = row_texts[row.index].text(context, (), |catalog, (), out| {
                write_type_row(catalog, &images.images[row.index], out);
            });
            RowContent::new(text).icon(Some(icon)).show(context, row);
        });
        if list_response.selection_changed {
            self.type_changed();
        }

        let name = TextField::new(NAME_ID, catalog.text(create_image::NAME_PLACEHOLDER))
            .icon(Some(icon))
            .clears_on_cancel(false)
            .error(self.check.marks_field())
            .show(context, name_area, &mut self.name);
        if name.changed {
            self.name_edited = true;
            self.run_check();
        }

        let kind = if self.failure.is_none() && self.check == NameCheck::Valid {
            NoteKind::Plain
        } else {
            NoteKind::Warning
        };
        let message =
            texts
                .message
                .text(context, (self.check, self.revision), |catalog, _, out| {
                    self.write_message(catalog, out)
                });
        let one_line = self.failure.is_some() || self.check == NameCheck::FileExists;
        rows::draw_note_text(
            context,
            message_area,
            message,
            kind,
            one_line,
            &mut self.message_lines,
        );
        self.texts = texts;

        let create = Button::new(CREATE_ID, catalog.text(common::CREATE))
            .icon(Icon::NewFile)
            .kind(ButtonKind::Primary)
            .enabled(self.check == NameCheck::Valid);
        let cancel = Button::new(CANCEL_ID, catalog.text(common::CANCEL));
        let (create_area, rest) = footer.cut_right(create.width(context));
        let cancel_area = rest.cut_right(gap).1.cut_right(cancel.width(context)).0;
        if cancel.show(context, cancel_area) {
            outcome = CreateOutcome::Cancelled;
        }
        let create_pressed = create.show(context, create_area);

        if browse_pressed {
            self.browser = Some(FileBrowser::new(self.folder_request(), &self.root));
        } else if outcome == CreateOutcome::Open
            && (create_pressed || list_response.activated.is_some() || name.submitted)
        {
            outcome = self.create();
        }
        self.hints = self.focus_hints(context);
        outcome
    }

    /// The hints of the widget that holds the focus. A hint whose action an earlier hint has
    /// leaves the line, such as Esc beside the focused Cancel button.
    fn focus_hints(&self, context: &UiContext) -> HintTexts {
        let focused = |id: &str| context.has_focus(UiId::from(id));
        let valid = self.check == NameCheck::Valid;
        let mut hints = StackVec::<ActionHint, MAXIMUM_HINTS>::new();
        if focused(TYPE_LIST_ID) || focused(NAME_ID) {
            if valid {
                hints.push(CREATE_HINT);
            }
        } else if focused(FOLDER_ID) {
            hints.push(BROWSE_HINT);
        } else if focused(CANCEL_ID) {
            hints.push(CANCEL_HINT);
        } else if focused(CREATE_ID) {
            hints.push(CREATE_HINT);
        }
        hints.push(CLOSE_HINT);
        distinct_hints(hints.iter().copied())
    }
}

impl Editor {
    /// The Create button of the drive of `setting` in a row that is `enabled`: hidden on a drive
    /// whose family takes no blank image, disabled on a disabled row and on a model without
    /// blank images, and enabled else. The settings page shows none.
    pub(super) fn create_button(&self, setting: &Setting, enabled: bool) -> CreateButton {
        let Some(files) = self.system_files() else {
            return CreateButton::Hidden;
        };
        let Some(types) = setting.file_types_for(files.target) else {
            return CreateButton::Hidden;
        };
        if !has_blank_images(types, files.target, setting.key) {
            return CreateButton::Hidden;
        }
        let model = self.model_of(files).name();
        if enabled && blank_images_for(types, files.target, setting.key, model).is_some() {
            CreateButton::Enabled
        } else {
            CreateButton::Disabled
        }
    }

    /// The request of the create image dialog for `target`. A field without files and a model
    /// without blank images of the drive give `None`.
    pub(super) fn create_request(&self, target: BrowseTarget) -> Option<CreateRequest> {
        let files = self.system_files()?;
        let index = match target {
            BrowseTarget::Path(index) | BrowseTarget::List(index, _) => index,
        };
        let setting = self.fields[index].source.setting()?;
        let types = setting.file_types_for(files.target)?;
        let images = blank_images_for(
            types,
            files.target,
            setting.key,
            self.model_of(files).name(),
        )?;
        let file_stem = self
            .path()
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        Some(CreateRequest {
            setting,
            types,
            images,
            stem: name_stem(&file_stem, setting.key),
            folder: self.start_folder(target),
        })
    }

    /// The folder the create image dialog of `target` starts in: the folder of the current image
    /// of the target, else the folder of the first image of a media field, else the root.
    fn start_folder(&self, target: BrowseTarget) -> PathBuf {
        let target_image = self.browse_request(target).current;
        let media_images = self
            .fields
            .iter()
            .filter(|field| {
                field.source.setting().is_some_and(|setting| {
                    matches!(
                        setting.kind,
                        ValueKind::Path(PathKind::File) | ValueKind::PathList
                    ) && self.file_types(setting).is_some()
                })
            })
            .filter_map(|field| field.values().into_iter().next());
        target_image
            .into_iter()
            .chain(media_images)
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .filter_map(|value| {
                resolve_path(&self.root, Path::new(&value))
                    .parent()
                    .map(Path::to_path_buf)
            })
            .find(|folder| folder.is_dir())
            .unwrap_or_else(|| self.root.clone())
    }
}

#[cfg(test)]
mod tests {
    use device::{disk::load_hdd_image, floppy::load_floppy_image};
    use gui::ui::{EditCommand, Navigation};

    use super::*;
    use crate::{
        editor::{
            media_list::MediaBrowse,
            test_support::{EditorRoot, fdd1_model, index_of},
        },
        gui_test_support::{GuiHarness, MINIMUM_SCREEN, assert_hints_fit},
        texts::{english_labels, english_text, hints_in, repository::repository_catalogs},
    };

    /// The area of the dialog in the tests.
    const AREA: Rect = MINIMUM_SCREEN;

    /// The stem of the game file of the tests.
    const GAME_STEM: &str = "yu-no";

    /// Opens the editor of the game [`GAME_STEM`] of `model` with the lines `lines` below `root`.
    fn open_editor(root: &EditorRoot, model: &str, lines: &str) -> Editor {
        let path = root.game(GAME_STEM, &format!("machine = {model}\n{lines}"));
        Editor::open(&root.host(), root.system(), &path).expect("the editor opens")
    }

    /// The target of the drive `key` of `editor`: a single path or the end of a list.
    fn target_of(editor: &Editor, key: &str) -> BrowseTarget {
        let index = index_of(editor, key);
        match editor.fields[index]
            .source
            .setting()
            .map(|setting| setting.kind)
        {
            Some(ValueKind::PathList) => BrowseTarget::List(index, MediaBrowse::Add),
            Some(
                ValueKind::Path(_)
                | ValueKind::Choice(_)
                | ValueKind::OnOff
                | ValueKind::Number(_)
                | ValueKind::Language,
            )
            | None => BrowseTarget::Path(index),
        }
    }

    /// The dialog of the drive `key` of `editor`.
    fn dialog_of(editor: &Editor, key: &str) -> CreateImageDialog {
        let request = editor
            .create_request(target_of(editor, key))
            .expect("the drive has blank images");
        CreateImageDialog::new(request, &editor.root)
    }

    /// The English text of the selected type row of `dialog`.
    fn selected_row(dialog: &CreateImageDialog) -> String {
        english_text(|catalog, out| write_type_row(catalog, dialog.selected_image(), out))
    }

    /// The English labels of the hints of the last pass of `dialog`.
    fn labels(dialog: &CreateImageDialog) -> Vec<&'static str> {
        english_labels(dialog.hints())
    }

    /// Draws passes of `dialog` until they settle and returns the answer of the last pass.
    fn show(harness: &mut GuiHarness, dialog: &mut CreateImageDialog) -> CreateOutcome {
        harness.settle(|context| dialog.show(context, AREA))
    }

    /// Replaces the name of the focused name field of `dialog` with `text` through the keyboard.
    fn type_name(harness: &mut GuiHarness, dialog: &mut CreateImageDialog, text: &str) {
        harness.state.set_focus(UiId::from(NAME_ID));
        show(harness, dialog);
        harness.input.edit = Some(EditCommand::SelectAll);
        show(harness, dialog);
        harness.input.text = text.to_owned();
        show(harness, dialog);
        assert_eq!(dialog.name, text);
    }

    #[cfg(feature = "pc98")]
    /// Verifies that the dialog opens with the focus on the type list, the default type of the
    /// model selected, a free default name and the hints Create and Close.
    #[test]
    fn the_dialog_opens_on_the_default_type_of_the_model() {
        let root = EditorRoot::with_system("create_image_defaults", "pc98");
        for (model, row) in [
            ("PC9821AS", "IDE 40 MB (.hdi)"),
            ("PC9801RA", "SASI 20 MB (.hdi)"),
        ] {
            let editor = open_editor(&root, model, "");
            let mut dialog = dialog_of(&editor, "hdd1");
            let mut harness = GuiHarness::minimum();
            assert_eq!(show(&mut harness, &mut dialog), CreateOutcome::Open);
            assert_eq!(harness.state.focused(), Some(UiId::from(TYPE_LIST_ID)));
            assert_eq!(selected_row(&dialog), row, "{model}");
            assert_eq!(dialog.name, "yu-no-hdd1.hdi");
            assert_eq!(dialog.folder, root.root);
            assert_eq!(dialog.check, NameCheck::Valid);
            assert_eq!(labels(&dialog), ["Create", "Close"]);
            assert_eq!(
                dialog.english_message(),
                "The image is blank. Format it in the emulated system before use."
            );
        }
    }

    #[cfg(feature = "x68k")]
    /// Verifies the default hard disk of the X68000 XVI and that Down selects the next floppy
    /// type, whose extension the unedited name follows, and an edited image name follows too.
    #[test]
    fn the_unedited_name_follows_the_selected_type() {
        let root = EditorRoot::with_system("create_image_follow", "x68k");
        let editor = open_editor(&root, "X68000XVI", "");
        let dialog = dialog_of(&editor, "hdd1");
        assert_eq!(selected_row(&dialog), "SCSI 40 MB (.hdf)");

        let mut dialog = dialog_of(&editor, "fdd1");
        let mut harness = GuiHarness::minimum();
        show(&mut harness, &mut dialog);
        assert_eq!(selected_row(&dialog), "2HD 1.2 MB (.xdf)");
        assert_eq!(dialog.name, "yu-no-fdd1.xdf");
        harness.input.navigation = Some(Navigation::Down);
        show(&mut harness, &mut dialog);
        assert_eq!(selected_row(&dialog), "2HD 1.2 MB (.d88)");
        assert_eq!(dialog.name, "yu-no-fdd1.d88");

        type_name(&mut harness, &mut dialog, "save.D88");
        harness.state.set_focus(UiId::from(TYPE_LIST_ID));
        show(&mut harness, &mut dialog);
        harness.input.navigation = Some(Navigation::Up);
        show(&mut harness, &mut dialog);
        assert_eq!(dialog.name, "save.xdf");
        type_name(&mut harness, &mut dialog, "notes.txt");
        harness.state.set_focus(UiId::from(TYPE_LIST_ID));
        show(&mut harness, &mut dialog);
        harness.input.navigation = Some(Navigation::Down);
        show(&mut harness, &mut dialog);
        assert_eq!(dialog.name, "notes.txt");
    }

    /// Verifies that a name without the extension of the type shows the warning, disables Create
    /// and leaves only the Close hint.
    #[test]
    fn a_wrong_extension_disables_create() {
        let root = EditorRoot::new("create_image_extension");
        let editor = open_editor(&root, fdd1_model(&root), "");
        let mut dialog = dialog_of(&editor, "fdd1");
        let mut harness = GuiHarness::minimum();
        show(&mut harness, &mut dialog);
        type_name(&mut harness, &mut dialog, "save");
        assert_eq!(dialog.check, NameCheck::WrongExtension);
        assert!(dialog.name_edited);
        assert!(
            dialog
                .english_message()
                .starts_with("The name needs the extension .")
        );
        assert_eq!(labels(&dialog), ["Close"]);
        harness.input.confirm = true;
        assert_eq!(show(&mut harness, &mut dialog), CreateOutcome::Open);
        assert!(!root.path("save").exists());
    }

    #[cfg(feature = "towns")]
    /// Verifies that the FM Towns hard disks take the extension of the SCSI ID of their drive:
    /// `.h0` on `hdd1` and `.h1` on `hdd2`, in the default name and in the row text, that a type
    /// change on `hdd2` keeps `.h1`, that every extension of the five stays valid, and that the
    /// warning of a wrong extension lists the five extensions in their order on both drives.
    #[test]
    fn the_towns_hard_disks_take_the_extension_of_their_scsi_id() {
        let root = EditorRoot::with_system("create_image_towns", "towns");
        let editor = open_editor(&root, "FMTowns", "");
        for (key, extension) in [("hdd1", "h0"), ("hdd2", "h1")] {
            let mut dialog = dialog_of(&editor, key);
            assert_eq!(selected_row(&dialog), format!("SCSI 100 MB (.{extension})"));
            assert_eq!(dialog.name, format!("yu-no-{key}.{extension}"));
            let mut harness = GuiHarness::minimum();
            show(&mut harness, &mut dialog);
            harness.input.navigation = Some(Navigation::Down);
            show(&mut harness, &mut dialog);
            assert_eq!(selected_row(&dialog), format!("SCSI 200 MB (.{extension})"));
            assert_eq!(dialog.name, format!("yu-no-{key}.{extension}"));
            type_name(&mut harness, &mut dialog, "save.h3");
            assert_eq!(dialog.check, NameCheck::Valid, "{key}");
            dialog.name = "save".to_owned();
            dialog.run_check();
            assert_eq!(
                dialog.english_message(),
                "The name needs the extension .h0, .h1, .h2, .h3 or .h4.",
                "{key}"
            );
        }
        let mut dialog = dialog_of(&editor, "hdd2");
        let mut harness = GuiHarness::minimum();
        show(&mut harness, &mut dialog);
        type_name(&mut harness, &mut dialog, "save.hdi");
        harness.state.set_focus(UiId::from(TYPE_LIST_ID));
        show(&mut harness, &mut dialog);
        harness.input.navigation = Some(Navigation::Up);
        show(&mut harness, &mut dialog);
        assert_eq!(dialog.name, "save.h1");
    }

    /// Verifies that an existing default name takes `-2` and that a typed name of an existing
    /// file shows the warning and keeps the file.
    #[test]
    fn an_existing_file_is_never_replaced() {
        let root = EditorRoot::new("create_image_exists");
        let editor = open_editor(&root, fdd1_model(&root), "");
        let taken = dialog_of(&editor, "fdd1").name;
        root.write(&taken, "kept");
        let mut dialog = dialog_of(&editor, "fdd1");
        let stem = taken.rsplit_once('.').expect("an extension").0;
        assert!(
            dialog.name.starts_with(&format!("{stem}-2.")),
            "{}",
            dialog.name
        );
        let mut harness = GuiHarness::minimum();
        show(&mut harness, &mut dialog);
        type_name(&mut harness, &mut dialog, &taken);
        assert_eq!(dialog.check, NameCheck::FileExists);
        assert_eq!(
            dialog.english_message(),
            format!("The file {taken} exists. Choose another name.")
        );
        harness.input.confirm = true;
        assert_eq!(show(&mut harness, &mut dialog), CreateOutcome::Open);
        assert_eq!(
            std::fs::read_to_string(root.path(&taken)).expect("the file reads"),
            "kept"
        );
    }

    #[cfg(feature = "pc98")]
    /// Verifies that Enter on the type list creates the selected image, gives its path relative
    /// to the root, and that the image loads back, for a hard disk and a floppy disk.
    #[test]
    fn enter_on_the_list_creates_the_image() {
        let root = EditorRoot::with_system("create_image_enter", "pc98");
        let editor = open_editor(&root, "PC9801RA", "");
        let mut dialog = dialog_of(&editor, "hdd1");
        let mut harness = GuiHarness::minimum();
        show(&mut harness, &mut dialog);
        harness.input.navigation = Some(Navigation::Home);
        show(&mut harness, &mut dialog);
        assert_eq!(selected_row(&dialog), "SASI 5 MB (.hdi)");
        harness.input.confirm = true;
        assert_eq!(
            harness.frame(|context| dialog.show(context, AREA)),
            CreateOutcome::Created("yu-no-hdd1.hdi".to_owned())
        );
        let path = root.path("yu-no-hdd1.hdi");
        let data = std::fs::read(&path).expect("the image reads");
        let image = load_hdd_image(&path, &data).expect("the image loads");
        assert_eq!(image.geometry.sector_size, 256);

        let mut dialog = dialog_of(&editor, "fdd1");
        show(&mut harness, &mut dialog);
        harness.input.confirm = true;
        assert_eq!(
            harness.frame(|context| dialog.show(context, AREA)),
            CreateOutcome::Created("yu-no-fdd1.d88".to_owned())
        );
        let path = root.path("yu-no-fdd1.d88");
        let data = std::fs::read(&path).expect("the image reads");
        let disk = load_floppy_image(&path, &data).expect("the image loads");
        assert_eq!(disk.sector_count(0), 8);
    }

    /// Verifies that a file that appears after the check stops the create: a press gives the
    /// warning of the existing file, and a write that meets the file gives the warning of the
    /// failed write. The dialog stays open, and the file keeps its bytes.
    #[test]
    fn a_file_that_appears_after_the_check_stays() {
        let root = EditorRoot::new("create_image_race");
        let editor = open_editor(&root, fdd1_model(&root), "");
        let mut dialog = dialog_of(&editor, "fdd1");
        let mut harness = GuiHarness::minimum();
        show(&mut harness, &mut dialog);
        assert_eq!(dialog.check, NameCheck::Valid);
        let name = dialog.name.clone();
        root.write(&name, "kept");
        assert_eq!(dialog.write_image(), CreateOutcome::Open);
        assert!(
            dialog
                .english_message()
                .starts_with("The image was not created: failed to create "),
            "{}",
            dialog.english_message()
        );
        show(&mut harness, &mut dialog);
        assert_eq!(labels(&dialog), ["Create", "Close"]);
        harness.input.confirm = true;
        assert_eq!(show(&mut harness, &mut dialog), CreateOutcome::Open);
        assert_eq!(dialog.check, NameCheck::FileExists);
        assert_eq!(dialog.failure, None);
        assert_eq!(
            std::fs::read_to_string(root.path(&name)).expect("the file reads"),
            "kept"
        );
    }

    /// Verifies that Esc closes the dialog without an image, from the type list and from the
    /// name field.
    #[test]
    fn escape_cancels_from_every_widget() {
        let root = EditorRoot::new("create_image_escape");
        let editor = open_editor(&root, fdd1_model(&root), "");
        for focus in [TYPE_LIST_ID, NAME_ID] {
            let mut dialog = dialog_of(&editor, "fdd1");
            let mut harness = GuiHarness::minimum();
            show(&mut harness, &mut dialog);
            harness.state.set_focus(UiId::from(focus));
            show(&mut harness, &mut dialog);
            harness.input.cancel = true;
            assert_eq!(
                harness.frame(|context| dialog.show(context, AREA)),
                CreateOutcome::Cancelled,
                "{focus}"
            );
            assert!(!root.path(&dialog.name).exists());
        }
    }

    /// Verifies the start folder of the dialog: the folder of the current image of the drive,
    /// then the folder of the first image of another media field, then the root.
    #[test]
    fn the_dialog_starts_in_the_folder_of_an_image() {
        let root = EditorRoot::new("create_image_start");
        root.write("games/a/disk.d88", "");
        root.write("games/b/disk.d88", "");
        let model = fdd1_model(&root);
        let editor = open_editor(&root, model, "fdd1 = games/a/disk.d88\n");
        assert_eq!(dialog_of(&editor, "fdd1").folder, root.path("games/a"));
        let editor = open_editor(&root, model, "fdd1 = games/missing/disk.d88\n");
        assert_eq!(dialog_of(&editor, "fdd1").folder, root.root);
        let has_fdd2 = editor.fields.iter().any(|field| {
            field.key() == "fdd2" && editor.create_request(target_of(&editor, "fdd2")).is_some()
        });
        if has_fdd2 {
            let editor = open_editor(&root, model, "fdd1 = games/b/disk.d88\n");
            assert_eq!(dialog_of(&editor, "fdd2").folder, root.path("games/b"));
        }
    }

    #[cfg(feature = "pc98")]
    /// Verifies that Browse opens the folder browser over the dialog with its hints and that a
    /// chosen folder updates the folder and the unedited default name.
    #[test]
    fn browse_chooses_the_folder() {
        let root = EditorRoot::with_system("create_image_browse", "pc98");
        root.write("games/98/disk.hdi", "");
        root.write("games/98/YU-NO/yu-no-hdd1.hdi", "");
        let editor = open_editor(&root, "PC9801RA", "hdd1 = games/98/disk.hdi\n");
        let mut dialog = dialog_of(&editor, "hdd1");
        assert_eq!(dialog.folder, root.path("games/98"));
        assert_eq!(dialog.name, "yu-no-hdd1.hdi");
        let mut harness = GuiHarness::minimum();
        show(&mut harness, &mut dialog);
        harness.state.set_focus(UiId::from(FOLDER_ID));
        show(&mut harness, &mut dialog);
        assert_eq!(labels(&dialog), ["Browse", "Close"]);
        harness.input.confirm = true;
        show(&mut harness, &mut dialog);
        assert!(dialog.browser.is_some());
        assert_eq!(harness.state.open_modals(), 2);
        assert_eq!(labels(&dialog), ["Open", "Parent folder", "Close"]);
        harness.input.confirm = true;
        show(&mut harness, &mut dialog);
        harness
            .state
            .set_focus(UiId::from(crate::file_browser::CHOOSE_ID));
        show(&mut harness, &mut dialog);
        harness.input.confirm = true;
        assert_eq!(show(&mut harness, &mut dialog), CreateOutcome::Open);
        assert!(dialog.browser.is_none());
        assert_eq!(harness.state.open_modals(), 1);
        assert_eq!(dialog.folder, root.path("games/98/YU-NO"));
        assert_eq!(dialog.name, "yu-no-hdd1-2.hdi");
        assert_eq!(dialog.check, NameCheck::Valid);
        assert_eq!(harness.state.focused(), Some(UiId::from(FOLDER_ID)));
    }

    /// Verifies that the messages of the dialog without a file name or an error fit one line of
    /// the panel in every language of the repository, with the alert icon of a warning.
    #[test]
    fn the_messages_fit_one_line_of_the_panel() {
        let mut harness = GuiHarness::minimum();
        let theme = &harness.theme;
        let width = PANEL_SIZE.0 - theme.padding * 2.0;
        let warning_width = width - theme.icon_size - theme.gap * 0.75;
        let extensions = ["h0", "h1", "h2", "h3", "h4"].map(DottedExtension);
        for catalog in repository_catalogs() {
            let mut messages = vec![(catalog.text(create_image::BLANK_NOTE).to_owned(), width)];
            for key in [
                create_image::EMPTY_NAME,
                create_image::FOLDER_IN_NAME,
                create_image::RESERVED_NAME,
                create_image::MISSING_FOLDER,
            ] {
                messages.push((catalog.text(key).to_owned(), warning_width));
            }
            let list = catalog.display_list(ListKind::Or, &extensions);
            messages.push((
                catalog
                    .display(create_image::wrong_extension(&list))
                    .to_string(),
                warning_width,
            ));
            harness.frame(|context| {
                let style = context.theme.small();
                for (message, width) in &messages {
                    let measured = context.measure(&style, message);
                    assert!(
                        measured <= *width,
                        "{} {message} {measured} {width}",
                        catalog.tag()
                    );
                }
            });
        }
    }

    /// Verifies that the warning of an existing file with a long name and the error of a write
    /// with a long path draw on one line in the warning color.
    #[test]
    fn a_long_name_and_a_long_path_keep_to_one_line() {
        let root = EditorRoot::new("create_image_long");
        let editor = open_editor(&root, fdd1_model(&root), "");
        let mut dialog = dialog_of(&editor, "fdd1");
        let extension = dialog.selected_image().extensions[0];
        let long_name = format!(
            "{}.{extension}",
            "a very long name of a save disk ".repeat(4)
        );
        root.write(&long_name, "");
        let long_error = format!(
            "failed to create {}/{long_name}: File exists (os error 17)",
            "/a/very/long/folder/of/the/games".repeat(4)
        );
        let mut harness = GuiHarness::minimum();
        for failure in [None, Some(long_error)] {
            dialog.name = long_name.clone();
            dialog.run_check();
            assert_eq!(dialog.check, NameCheck::FileExists);
            dialog.failure = failure;
            dialog.revision += 1;
            show(&mut harness, &mut dialog);
            let color = harness.theme.colors.danger_text.to_linear();
            let glyphs: Vec<[f32; 4]> = harness
                .scene
                .instances()
                .iter()
                .filter(|instance| {
                    instance.kind[0] == gui::Instance::KIND_GLYPH && instance.color == color
                })
                .map(|instance| instance.rect)
                .collect();
            assert!(!glyphs.is_empty());
            let top = glyphs.iter().map(|rect| rect[1]).fold(f32::MAX, f32::min);
            let bottom = glyphs
                .iter()
                .map(|rect| rect[1] + rect[3])
                .fold(f32::MIN, f32::max);
            assert!(bottom - top <= PATH_LINE_HEIGHT, "{top} {bottom}");
        }
    }

    /// Verifies that every hint set of the dialog shows every key cap at the minimum window size
    /// in every language of the repository.
    #[test]
    fn the_hints_of_the_dialog_fit_at_the_minimum_window_size() {
        for hints in [
            [CREATE_HINT.1, CLOSE_HINT.1].as_slice(),
            &[BROWSE_HINT.1, CLOSE_HINT.1],
            &[CANCEL_HINT.1],
            &[CLOSE_HINT.1],
        ] {
            assert_hints_fit(|catalog| hints_in(catalog, hints));
        }
    }
}
