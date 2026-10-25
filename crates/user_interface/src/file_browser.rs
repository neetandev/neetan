//! The file browser: a modal that lists the folders and the files of one
//! folder for a path setting, filtered by the extensions of the setting.
//!
//! The browser starts in the folder of the current file, or in the root. The
//! first row opens the parent folder, and Backspace does the same while the
//! file list holds the focus. Folders come first, then the files. A chosen
//! path below the root comes back relative to the root, as the layers of a
//! game file resolve it, and any other path comes back absolute.

use std::path::{Path, PathBuf};

use gui::{
    Rect,
    ui::{
        ACTIVATE_KEYS, Align, Button, ButtonKind, CANCEL_KEYS, Checkbox, HintKey, Icon, ListState,
        PATH_LINE_HEIGHT, RowContent, RowMark, Shortcut, TextBuffer, TextField, TextValue,
        Truncation, UiContext, UiId, distinct_hints, label, list, modal_in,
    },
};
use i18n::Catalog;
use machine_factory::config::settings::{PathKind, file_types::FileTypes};
use stack_vec::StackVec;

use crate::{
    config::resolve_path,
    editor::ExtensionList,
    media_dialog::{ImageEntry, PANEL_SIZE, TAB_BAR_HEIGHT, fold_filter, matching_entries},
    texts::{self, HintText, HintTexts, MAXIMUM_HINTS, browser, common},
};

/// The id of the modal layer of the browser.
pub(crate) const BROWSER_LAYER_ID: &str = "file browser";

/// The id of the file list.
const LIST_ID: &str = "browser files";

/// The id of the filter field.
const FILTER_ID: &str = "browser filter";

/// The id of the checkbox that lifts the extension filter.
const SHOW_ALL_ID: &str = "browser show all";

/// The id of the cancel button.
const CANCEL_ID: &str = "browser cancel";

/// The id of the choose button.
const CHOOSE_ID: &str = "browser choose";

/// The result of a hint of the browser. Two hints of one action lead to
/// one result, and the status line shows only the first of them.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum BrowserAction {
    /// The chosen file or folder returns.
    Choose,
    /// The selected folder opens.
    OpenFolder,
    /// The parent folder opens.
    OpenParent,
    /// The checkbox flips.
    Switch,
    /// The filter empties.
    Clear,
    /// The browser closes without a choice.
    Close,
}

/// A hint with its action.
type ActionHint = (BrowserAction, HintText);

/// The hint of a file row and of the choose button.
const CHOOSE_HINT: ActionHint = (
    BrowserAction::Choose,
    HintText::new(ACTIVATE_KEYS, texts::common::CHOOSE),
);

/// The hint of a folder row.
const OPEN_HINT: ActionHint = (
    BrowserAction::OpenFolder,
    HintText::new(ACTIVATE_KEYS, texts::common::OPEN),
);

/// The hint of the row that opens the parent folder.
const OPEN_PARENT_HINT: ActionHint = (
    BrowserAction::OpenParent,
    HintText::new(ACTIVATE_KEYS, texts::common::OPEN),
);

/// The hint of Backspace in the file list.
const PARENT_HINT: ActionHint = (
    BrowserAction::OpenParent,
    HintText::new(&[HintKey::Key("Backspace")], browser::PARENT_FOLDER),
);

/// The hint of the checkbox.
const SWITCH_HINT: ActionHint = (
    BrowserAction::Switch,
    HintText::new(ACTIVATE_KEYS, texts::common::SWITCH),
);

/// The hint of the cancel button.
const CANCEL_HINT: ActionHint = (
    BrowserAction::Close,
    HintText::new(ACTIVATE_KEYS, texts::common::CANCEL),
);

/// The hint of a filter with text, which cancel empties.
const CLEAR_HINT: ActionHint = (
    BrowserAction::Clear,
    HintText::new(CANCEL_KEYS, texts::common::CLEAR),
);

/// The hint of the close of the browser.
const CLOSE_HINT: ActionHint = (
    BrowserAction::Close,
    HintText::new(CANCEL_KEYS, texts::common::CLOSE),
);

/// The kind of item that the browser chooses. The title of the browser shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BrowseNoun {
    /// A folder.
    Folder,
    /// A disk image.
    Image,
    /// Any other file.
    File,
}

/// What a path setting asks the browser for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BrowseRequest {
    /// The kind of item that the browser chooses.
    pub(crate) noun: BrowseNoun,
    /// The label of the setting, for example `FDD 1`.
    pub(crate) label: &'static str,
    /// Whether the setting takes a file or a folder.
    pub(crate) kind: PathKind,
    /// The files that the setting takes. `None` takes every file.
    pub(crate) types: Option<&'static FileTypes>,
    /// The icon of the files.
    pub(crate) icon: Icon,
    /// The current value of the setting, as the file writes it.
    pub(crate) current: Option<String>,
}

/// The answer of the browser in a pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BrowserOutcome {
    /// The browser stays open.
    Open,
    /// The browser closes without a choice.
    Cancelled,
    /// The browser closes with this value: a path below the root relative
    /// to the root, any other path absolute.
    Chosen(String),
}

/// One row of the file list.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum BrowserRow {
    /// The row that opens the parent folder.
    Parent,
    /// The folder at this index of the folders.
    Folder(usize),
    /// The file at this index of the files.
    File(usize),
}

/// Returns the value of `path` as a configuration file writes it: relative
/// to `root` when the path lies below it, else the whole path.
pub(crate) fn path_value(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .ok()
        .filter(|relative| !relative.as_os_str().is_empty())
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

/// Writes the title of the browser for `request` in `catalog`, for example
/// `Choose image for FDD 1`.
fn write_title(catalog: &Catalog, request: &BrowseRequest, out: &mut String) {
    let label = &request.label;
    let message = match request.noun {
        BrowseNoun::Folder => browser::choose_folder(label),
        BrowseNoun::Image => browser::choose_image(label),
        BrowseNoun::File => browser::choose_file(label),
    };
    let _ = catalog.write(out, message);
}

/// The title of the browser for `request` in English.
#[cfg(test)]
pub(crate) fn english_title(request: &BrowseRequest) -> String {
    texts::english_text(|catalog, out| write_title(catalog, request, out))
}

/// Writes the text of the line under the filter in `catalog`: the name and
/// the extensions of the files, or that every file shows.
fn write_extension_line(
    catalog: &Catalog,
    out: &mut String,
    types: Option<&FileTypes>,
    show_all: bool,
    kind: PathKind,
) {
    match (kind, types) {
        (PathKind::Folder, _) => out.push_str(catalog.text(browser::FOLDERS)),
        (PathKind::File, Some(types)) if !show_all => {
            let _ = catalog.write(out, browser::extensions(&types.name, &ExtensionList(types)));
        }
        (PathKind::File, Some(_) | None) => out.push_str(catalog.text(browser::ALL_FILES)),
    }
}

/// The text of a list without a file row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EmptyText {
    /// The folder does not read.
    ReadError,
    /// No file passes the filter.
    NoMatch,
    /// The folder holds no folder.
    NoFolders,
    /// The folder holds no file of the types of the setting.
    NoTypedFiles,
    /// The folder holds no file.
    NoFiles,
}

impl TextValue for EmptyText {}

/// The texts of the browser that refill only when their data or the
/// catalog changes.
#[derive(Debug, Clone, Default)]
struct BrowserTexts {
    /// The title of the panel.
    title: TextBuffer<()>,
    /// The line under the filter, for whether every file shows.
    extensions: TextBuffer<bool>,
    /// The text of a list without a file row, keyed on the read error of the folder.
    empty: TextBuffer<(EmptyText, String)>,
}

/// The open file browser.
#[derive(Debug, Clone)]
pub(crate) struct FileBrowser {
    /// What the setting asks for.
    request: BrowseRequest,
    /// The root folder.
    root: PathBuf,
    /// The absolute path of the current file.
    current: Option<PathBuf>,
    /// The folder that the list shows.
    folder: PathBuf,
    /// The folders of the folder, sorted by name.
    folders: Vec<ImageEntry>,
    /// The files of the folder, sorted by name.
    files: Vec<ImageEntry>,
    /// The error of a folder that does not read.
    read_error: Option<String>,
    /// The texts of the panel.
    texts: BrowserTexts,
    /// The filter text.
    filter: String,
    /// Whether the extension filter is lifted.
    show_all: bool,
    /// The visible rows.
    rows: Vec<BrowserRow>,
    /// The selection and the scroll position of the list.
    list: ListState,
    /// The hints of the status line of the last pass.
    hints: HintTexts,
    /// Whether the rows changed after the list drew them, so the pass draws stale widgets.
    stale: bool,
}

impl FileBrowser {
    /// A browser for `request` below `root`. It starts in the folder of the
    /// current file when that folder exists, else in the root, with the
    /// current file selected.
    pub(crate) fn new(request: BrowseRequest, root: &Path) -> Self {
        let current = request
            .current
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(|value| resolve_path(root, Path::new(value.trim())));
        let folder = current
            .as_deref()
            .and_then(|current| match request.kind {
                PathKind::File => current.parent(),
                PathKind::Folder => current.parent().filter(|_| current.is_dir()),
            })
            .filter(|folder| folder.is_dir())
            .map_or_else(|| root.to_path_buf(), Path::to_path_buf);
        let mut browser = Self {
            request,
            root: root.to_path_buf(),
            current,
            folder: PathBuf::new(),
            folders: Vec::new(),
            files: Vec::new(),
            read_error: None,
            texts: BrowserTexts::default(),
            filter: String::new(),
            show_all: false,
            rows: Vec::new(),
            list: ListState::new(),
            hints: HintTexts::new(),
            stale: false,
        };
        let selected = browser.current.clone();
        browser.open_folder(folder, selected.as_deref());
        browser
    }

    /// The folder that the list shows.
    #[cfg(test)]
    pub(crate) fn folder(&self) -> &Path {
        &self.folder
    }

    /// The hints of the status line of the last pass.
    pub(crate) fn hints(&self) -> &HintTexts {
        &self.hints
    }

    /// Reads `folder` and shows it with the row of `selected` selected, or
    /// with its first entry selected. The filter empties.
    fn open_folder(&mut self, folder: PathBuf, selected: Option<&Path>) {
        self.folders.clear();
        self.files.clear();
        self.read_error = None;
        match std::fs::read_dir(&folder) {
            Ok(entries) => {
                for entry in entries.filter_map(Result::ok) {
                    if entry.file_name().to_string_lossy().starts_with('.') {
                        continue;
                    }
                    let path = entry.path();
                    if path.is_dir() {
                        self.folders.push(ImageEntry::new(path));
                    } else {
                        self.files.push(ImageEntry::new(path));
                    }
                }
            }
            Err(error) => self.read_error = Some(error.to_string()),
        }
        for entries in [&mut self.folders, &mut self.files] {
            entries.sort_by_cached_key(|entry| (entry.name.to_lowercase(), entry.name.clone()));
        }
        self.folder = folder;
        self.filter.clear();
        self.rebuild_rows(selected);
    }

    /// Builds the visible rows: the parent row, the folders and the files
    /// that pass the filter and the extensions. Selects the row of
    /// `selected`, else the first entry after the parent row.
    fn rebuild_rows(&mut self, selected: Option<&Path>) {
        let folded_filter = fold_filter(&self.filter);
        let mut rows = Vec::new();
        if self.folder.parent().is_some() {
            rows.push(BrowserRow::Parent);
        }
        rows.extend(matching_entries(&self.folders, &folded_filter).map(BrowserRow::Folder));
        if self.request.kind == PathKind::File {
            let types = self.request.types.filter(|_| !self.show_all);
            rows.extend(
                matching_entries(&self.files, &folded_filter)
                    .filter(|&index| {
                        types.is_none_or(|types| types.matches(&self.files[index].path))
                    })
                    .map(BrowserRow::File),
            );
        }
        self.rows = rows;
        let found = selected.and_then(|selected| {
            self.rows
                .iter()
                .position(|row| self.row_path(*row).as_deref() == Some(selected))
        });
        let first_entry = usize::from(self.rows.first() == Some(&BrowserRow::Parent))
            .min(self.rows.len().saturating_sub(1));
        self.list.select(found.unwrap_or(first_entry));
        self.stale = true;
    }

    /// The path of the entry of `row`. The parent row holds the parent folder.
    fn row_path(&self, row: BrowserRow) -> Option<PathBuf> {
        match row {
            BrowserRow::Parent => self.folder.parent().map(Path::to_path_buf),
            BrowserRow::Folder(index) => Some(self.folders[index].path.clone()),
            BrowserRow::File(index) => Some(self.files[index].path.clone()),
        }
    }

    /// The selected row.
    fn selected_row(&self) -> Option<BrowserRow> {
        self.rows.get(self.list.selected).copied()
    }

    /// Opens the parent folder with the folder that the list shows selected.
    fn open_parent(&mut self) {
        if let Some(parent) = self.folder.parent().map(Path::to_path_buf) {
            let child = self.folder.clone();
            self.open_folder(parent, Some(&child));
        }
    }

    /// The path that a choice of `row` gives: a file of a file setting, a
    /// folder of a folder setting, or the shown folder from the parent row
    /// of a folder setting.
    fn chosen_path(&self, row: BrowserRow) -> Option<&Path> {
        match (self.request.kind, row) {
            (PathKind::File, BrowserRow::File(index)) => Some(&self.files[index].path),
            (PathKind::Folder, BrowserRow::Folder(index)) => Some(&self.folders[index].path),
            (PathKind::Folder, BrowserRow::Parent) => Some(&self.folder),
            (PathKind::File, BrowserRow::Parent | BrowserRow::Folder(_))
            | (PathKind::Folder, BrowserRow::File(_)) => None,
        }
    }

    /// The value that a choice of `row` gives, as [`Self::chosen_path`]
    /// finds it.
    fn chosen(&self, row: BrowserRow) -> Option<String> {
        self.chosen_path(row)
            .map(|path| path_value(path, &self.root))
    }

    /// Answers the activation of `row`: the parent row and a folder of a
    /// file setting open their folder, a choosable row gives its value.
    fn activate(&mut self, row: BrowserRow) -> BrowserOutcome {
        match (self.request.kind, row) {
            (_, BrowserRow::Parent) => {
                self.open_parent();
                BrowserOutcome::Open
            }
            (_, BrowserRow::Folder(index)) => {
                let folder = self.folders[index].path.clone();
                self.open_folder(folder, None);
                BrowserOutcome::Open
            }
            (PathKind::File, BrowserRow::File(_)) | (PathKind::Folder, BrowserRow::File(_)) => self
                .chosen(row)
                .map_or(BrowserOutcome::Open, BrowserOutcome::Chosen),
        }
    }

    /// The text of a list without a file row.
    fn empty_text(&self) -> Option<EmptyText> {
        if self.read_error.is_some() {
            return Some(EmptyText::ReadError);
        }
        if self
            .rows
            .iter()
            .any(|row| matches!(row, BrowserRow::File(_) | BrowserRow::Folder(_)))
        {
            return None;
        }
        if !self.filter.trim().is_empty() {
            return Some(EmptyText::NoMatch);
        }
        Some(match (self.request.kind, self.request.types) {
            (PathKind::Folder, _) => EmptyText::NoFolders,
            (PathKind::File, Some(_)) if !self.show_all => EmptyText::NoTypedFiles,
            (PathKind::File, Some(_) | None) => EmptyText::NoFiles,
        })
    }

    /// Writes the text `empty` of a list without a file row in `catalog`.
    fn write_empty_text(&self, catalog: &Catalog, empty: EmptyText, out: &mut String) {
        match empty {
            EmptyText::ReadError => {
                let error = self.read_error.as_deref().unwrap_or_default();
                let _ = catalog.write(out, browser::read_error(&error));
            }
            EmptyText::NoMatch => out.push_str(catalog.text(browser::NO_MATCH)),
            EmptyText::NoFolders => out.push_str(catalog.text(browser::NO_FOLDERS)),
            EmptyText::NoTypedFiles => {
                let files = self
                    .request
                    .types
                    .map(|types| types.name.to_lowercase())
                    .unwrap_or_default();
                let _ = catalog.write(out, browser::no_typed_files(&files));
            }
            EmptyText::NoFiles => out.push_str(catalog.text(browser::NO_FILES)),
        }
    }

    /// The text of a list without a file row in English.
    #[cfg(test)]
    fn english_empty_text(&self) -> Option<String> {
        self.empty_text().map(|empty| {
            texts::english_text(|catalog, out| self.write_empty_text(catalog, empty, out))
        })
    }

    /// Draws one pass of the browser in its modal layer, centered in `area`.
    /// Returns the answer of the pass.
    pub(crate) fn show(&mut self, context: &mut UiContext, area: Rect) -> BrowserOutcome {
        context.layer(BROWSER_LAYER_ID, |context| {
            let outcome = self.panel(context, area);
            if outcome != BrowserOutcome::Open {
                context.close_layer();
            }
            outcome
        })
    }

    /// Draws the panel of the browser centered in `area`.
    fn panel(&mut self, context: &mut UiContext, area: Rect) -> BrowserOutcome {
        let outcome = self.draw_panel(context, area);
        if self.stale {
            self.stale = false;
            context.request_pass();
        }
        outcome
    }

    /// Draws the widgets of the panel centered in `area` and returns the
    /// answer of the pass.
    fn draw_panel(&mut self, context: &mut UiContext, area: Rect) -> BrowserOutcome {
        let theme = context.theme;
        let catalog = context.catalog;
        let gap = theme.gap;
        let list_id = UiId::from(LIST_ID);
        let mut texts = std::mem::take(&mut self.texts);
        let title = texts.title.text(context, (), |catalog, (), out| {
            write_title(catalog, &self.request, out);
        });
        let response = modal_in(context, BROWSER_LAYER_ID, area, PANEL_SIZE, title);
        let mut outcome = if response.closed {
            BrowserOutcome::Cancelled
        } else {
            BrowserOutcome::Open
        };

        let (location, rest) = response.content.cut_top(TAB_BAR_HEIGHT);
        context.fill(
            Rect::new(
                location.x,
                location.bottom() - theme.stroke,
                location.width,
                theme.stroke,
            ),
            theme.colors.panel_edge,
        );
        let (icon_area, path_area) = location.cut_left(theme.icon_size);
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

        let (filter_area, rest) = rest.cut_top(gap * 1.5).1.cut_top(theme.control_height);
        let filter = TextField::new(FILTER_ID, catalog.text(browser::FILTER_PLACEHOLDER)).show(
            context,
            filter_area,
            &mut self.filter,
        );
        if filter.changed {
            let selected = self.selected_row().and_then(|row| self.row_path(row));
            self.rebuild_rows(selected.as_deref());
        }
        let (extension_area, rest) = rest.cut_top(gap).1.cut_top(PATH_LINE_HEIGHT);
        let extensions = texts
            .extensions
            .text(context, self.show_all, |catalog, show_all, out| {
                write_extension_line(
                    catalog,
                    out,
                    self.request.types,
                    show_all,
                    self.request.kind,
                );
            });
        label(
            context,
            extension_area,
            &theme.small(),
            extensions,
            Align::Start,
            Truncation::End,
        );
        let (footer, rest) = rest.cut_bottom(theme.control_height);
        let list_area = rest.cut_top(gap * 0.5).1.cut_bottom(gap * 1.5).1;

        context.default_focus(list_id);
        if context.has_focus(list_id) && context.shortcut() == Some(Shortcut::Backspace) {
            self.open_parent();
        }
        let rows = &self.rows;
        let current = self.current.as_deref();
        let icon = self.request.icon;
        let list_response = list(
            context,
            LIST_ID,
            list_area,
            rows.len(),
            &mut self.list,
            |context, row| match rows[row.index] {
                BrowserRow::Parent => {
                    RowContent::new(catalog.text(browser::PARENT_FOLDER))
                        .icon(Some(Icon::Up))
                        .show(context, row);
                }
                BrowserRow::Folder(index) => {
                    RowContent::new(&self.folders[index].name)
                        .icon(Some(Icon::Folder))
                        .show(context, row);
                }
                BrowserRow::File(index) => {
                    let entry = &self.files[index];
                    let marker = (current == Some(entry.path.as_path()))
                        .then_some(RowMark::Marker(Icon::Check));
                    RowContent::new(&entry.name)
                        .icon(Some(icon))
                        .mark(marker)
                        .show(context, row);
                }
            },
        );
        if let Some(empty) = self.empty_text() {
            let read_error = self.read_error.as_deref().unwrap_or_default();
            let text =
                texts
                    .empty
                    .text(context, (empty, read_error), |catalog, (empty, _), out| {
                        self.write_empty_text(catalog, empty, out);
                    });
            let first = usize::from(self.rows.first() == Some(&BrowserRow::Parent)) as f32;
            let hint_area = Rect::new(
                list_area.x,
                list_area.y + gap + theme.row_height * first,
                list_area.width,
                theme.row_height,
            );
            label(
                context,
                hint_area,
                &theme.small(),
                text,
                Align::Center,
                Truncation::End,
            );
        }
        self.texts = texts;

        let selected = self.selected_row();
        let choosable = selected.filter(|row| self.chosen_path(*row).is_some());
        let choose = Button::new(CHOOSE_ID, catalog.text(common::CHOOSE))
            .icon(Icon::Check)
            .kind(ButtonKind::Primary)
            .enabled(choosable.is_some());
        let cancel = Button::new(CANCEL_ID, catalog.text(common::CANCEL));
        let (choose_area, rest) = footer.cut_right(choose.width(context));
        let cancel_area = rest.cut_right(gap).1.cut_right(cancel.width(context));
        let show_all_offered = self.request.kind == PathKind::File && self.request.types.is_some();
        if show_all_offered {
            let checkbox = Checkbox::new(SHOW_ALL_ID, catalog.text(browser::SHOW_ALL));
            let width = checkbox.width(context).min(cancel_area.1.width);
            let checkbox_area = cancel_area.1.cut_left(width).0;
            if checkbox.show(context, checkbox_area, &mut self.show_all) {
                let selected = selected.and_then(|row| self.row_path(row));
                self.rebuild_rows(selected.as_deref());
            }
        }
        if cancel.show(context, cancel_area.0) {
            outcome = BrowserOutcome::Cancelled;
        }
        if choose.show(context, choose_area)
            && let Some(value) = choosable.and_then(|row| self.chosen(row))
        {
            outcome = BrowserOutcome::Chosen(value);
        }
        if let Some(index) = list_response.activated
            && let Some(row) = self.rows.get(index).copied()
        {
            outcome = self.activate(row);
        }
        self.hints = self.focus_hints(context);
        outcome
    }

    /// The hints of the widget that holds the focus. A hint whose action an
    /// earlier hint has leaves the line, such as Esc beside the focused
    /// Cancel button and Backspace beside the selected parent row.
    fn focus_hints(&self, context: &UiContext) -> HintTexts {
        let focused = |id: &str| context.has_focus(UiId::from(id));
        let mut hints = StackVec::<ActionHint, MAXIMUM_HINTS>::new();
        if focused(LIST_ID) {
            match self.selected_row() {
                Some(BrowserRow::File(_)) => hints.push(CHOOSE_HINT),
                Some(BrowserRow::Parent) => hints.push(OPEN_PARENT_HINT),
                Some(BrowserRow::Folder(_)) => hints.push(OPEN_HINT),
                None => {}
            }
            if self.folder.parent().is_some() {
                hints.push(PARENT_HINT);
            }
        } else if focused(SHOW_ALL_ID) {
            hints.push(SWITCH_HINT);
        } else if focused(CANCEL_ID) {
            hints.push(CANCEL_HINT);
        } else if focused(CHOOSE_ID) {
            hints.push(CHOOSE_HINT);
        } else if focused(FILTER_ID) && !self.filter.is_empty() {
            return distinct_hints([CLEAR_HINT]);
        }
        hints.push(CLOSE_HINT);
        distinct_hints(hints.iter().copied())
    }
}

#[cfg(test)]
mod tests {
    use machine_factory::config::settings::find_setting;

    use super::*;
    use crate::{editor::test_support::EditorRoot, gui_test_support::GuiHarness};

    /// The area of the browser in the tests.
    const AREA: Rect = Rect::new(0.0, 0.0, 960.0, 572.0);

    /// A request for an image of FDD 1 with the current value `current`.
    fn floppy_request(current: Option<&str>) -> BrowseRequest {
        let fdd1 = find_setting("fdd1").expect("fdd1 is a table key");
        BrowseRequest {
            noun: BrowseNoun::Image,
            label: "FDD 1",
            kind: PathKind::File,
            types: fdd1.file_types.first(),
            icon: Icon::Disk,
            current: current.map(str::to_owned),
        }
    }

    /// The names of the visible rows of `browser`, with `..` for the parent row.
    fn names(browser: &FileBrowser) -> Vec<String> {
        browser
            .rows
            .iter()
            .map(|row| match row {
                BrowserRow::Parent => "..".to_owned(),
                BrowserRow::Folder(index) => format!("{}/", browser.folders[*index].name),
                BrowserRow::File(index) => browser.files[*index].name.clone(),
            })
            .collect()
    }

    /// The name of the selected row of `browser`.
    fn selected(browser: &FileBrowser) -> String {
        names(browser)[browser.list.selected].clone()
    }

    /// A root with games below it: two disks, a cue sheet, a hidden file and two folders.
    fn games_root(name: &str) -> EditorRoot {
        let root = EditorRoot::new(name);
        for file in [
            "games/98/YU-NO/YU-NO (Disk 2 of 4).hdm",
            "games/98/YU-NO/YU-NO (Disk 1 of 4).HDM",
            "games/98/YU-NO/readme.txt",
            "games/98/YU-NO/.hidden.hdm",
            "games/98/YU-NO/ユーノ ユーザーディスク.d88",
            "games/98/YU-NO/Saves/save.d88",
            "games/98/YU-NO/Manual/manual.pdf",
            "games/98/disc.cue",
        ] {
            root.write(file, "");
        }
        root
    }

    /// Verifies the start in the folder of the current image with the image selected, the order of the rows and the extension filter.
    #[test]
    fn the_browser_starts_at_the_current_image_and_filters_by_extension() {
        let root = games_root("browser_start");
        let browser = FileBrowser::new(
            floppy_request(Some("games/98/YU-NO/YU-NO (Disk 1 of 4).HDM")),
            &root.root,
        );
        assert_eq!(browser.folder(), root.path("games/98/YU-NO"));
        assert_eq!(
            names(&browser),
            [
                "..",
                "Manual/",
                "Saves/",
                "YU-NO (Disk 1 of 4).HDM",
                "YU-NO (Disk 2 of 4).hdm",
                "ユーノ ユーザーディスク.d88"
            ]
        );
        assert_eq!(selected(&browser), "YU-NO (Disk 1 of 4).HDM");

        let mut browser = FileBrowser::new(floppy_request(None), &root.root);
        assert_eq!(browser.folder(), root.root);
        assert_eq!(names(&browser), ["..", "games/"]);
        assert_eq!(selected(&browser), "games/");
        let fresh = FileBrowser::new(floppy_request(Some("gone/disk.d88")), &root.root);
        assert_eq!(fresh.folder(), root.root);

        browser.open_folder(root.path("games/98/YU-NO"), None);
        browser.show_all = true;
        browser.rebuild_rows(None);
        assert!(names(&browser).contains(&"readme.txt".to_owned()));
        assert!(!names(&browser).contains(&".hidden.hdm".to_owned()));
        browser.filter = "disk 2".to_owned();
        browser.rebuild_rows(None);
        assert_eq!(names(&browser), ["..", "YU-NO (Disk 2 of 4).hdm"]);
    }

    /// Verifies that a chosen path below the root comes back relative and any other path absolute, as the layers resolve them.
    #[test]
    fn chosen_paths_are_relative_below_the_root() {
        let root = games_root("browser_paths");
        let inside = root.path("games/98/disc.cue");
        assert_eq!(path_value(&inside, &root.root), "games/98/disc.cue");
        assert_eq!(
            resolve_path(&root.root, Path::new(&path_value(&inside, &root.root))),
            inside
        );
        let outside = std::env::temp_dir().join("elsewhere").join("disk.d88");
        assert_eq!(path_value(&outside, &root.root), outside.to_string_lossy());
        assert_eq!(
            resolve_path(&root.root, Path::new(&path_value(&outside, &root.root))),
            outside
        );
        assert_eq!(
            path_value(&root.root, &root.root),
            root.root.to_string_lossy()
        );

        let mut browser = FileBrowser::new(
            floppy_request(Some("games/98/YU-NO/YU-NO (Disk 2 of 4).hdm")),
            &root.root,
        );
        let row = browser.selected_row().expect("a row is selected");
        assert_eq!(
            browser.activate(row),
            BrowserOutcome::Chosen("games/98/YU-NO/YU-NO (Disk 2 of 4).hdm".to_owned())
        );
        browser.open_parent();
        assert_eq!(browser.folder(), root.path("games/98"));
        assert_eq!(selected(&browser), "YU-NO/");
        browser.open_parent();
        browser.open_parent();
        assert_eq!(browser.folder(), root.root);
        assert_eq!(selected(&browser), "games/");
        browser.open_parent();
        let parent = root
            .root
            .parent()
            .expect("the root has a parent")
            .to_path_buf();
        assert_eq!(browser.folder(), parent);
    }

    /// Verifies the keys of the browser: confirm opens a folder, Backspace opens the parent folder from the list, and Backspace edits the filter while the filter holds the focus.
    #[test]
    fn backspace_opens_the_parent_folder_from_the_list() {
        let root = games_root("browser_keys");
        let mut browser = FileBrowser::new(floppy_request(None), &root.root);
        let mut harness = GuiHarness::new(960, 600);
        harness.settle(|context| browser.show(context, AREA));
        assert_eq!(harness.state.focused(), Some(UiId::from(LIST_ID)));
        assert_eq!(selected(&browser), "games/");
        harness.input.confirm = true;
        assert_eq!(
            harness.settle(|context| browser.show(context, AREA)),
            BrowserOutcome::Open
        );
        assert_eq!(browser.folder(), root.path("games"));
        harness.input.confirm = true;
        harness.settle(|context| browser.show(context, AREA));
        assert_eq!(names(&browser), ["..", "YU-NO/"]);
        harness.input.confirm = true;
        harness.settle(|context| browser.show(context, AREA));
        assert_eq!(browser.folder(), root.path("games/98/YU-NO"));
        assert_eq!(
            texts::english_labels(browser.hints()),
            ["Open", "Parent folder", "Close"]
        );

        harness.input.backspace = true;
        harness.settle(|context| browser.show(context, AREA));
        assert_eq!(browser.folder(), root.path("games/98"));
        assert_eq!(selected(&browser), "YU-NO/");
        harness.input.confirm = true;
        harness.settle(|context| browser.show(context, AREA));
        harness.input.navigation = Some(gui::ui::Navigation::Down);
        harness.settle(|context| browser.show(context, AREA));
        assert_eq!(selected(&browser), "Saves/");
        assert_eq!(harness.state.focused(), Some(UiId::from(LIST_ID)));
        harness.input.backspace = true;
        harness.settle(|context| browser.show(context, AREA));

        harness.state.set_focus(UiId::from(FILTER_ID));
        harness.input.text = "yu".to_owned();
        harness.settle(|context| browser.show(context, AREA));
        assert_eq!(names(&browser), ["..", "YU-NO/"]);
        harness.input.backspace = true;
        harness.settle(|context| browser.show(context, AREA));
        assert_eq!(browser.folder(), root.path("games/98"));
        assert_eq!(browser.filter, "y");
        assert_eq!(texts::english_labels(browser.hints()), ["Clear"]);
        harness.input.cancel = true;
        harness.settle(|context| browser.show(context, AREA));
        assert_eq!(browser.filter, "");
        harness.input.cancel = true;
        assert_eq!(
            harness.frame(|context| browser.show(context, AREA)),
            BrowserOutcome::Cancelled
        );
    }

    /// Verifies that a folder without images disables Choose, shows the hint and that the checkbox lifts the filter.
    #[test]
    fn a_folder_without_images_disables_choose() {
        let root = games_root("browser_empty");
        let cdrom = find_setting("cdrom").expect("cdrom is a table key");
        let request = BrowseRequest {
            noun: BrowseNoun::Image,
            label: "CD-ROM",
            kind: PathKind::File,
            types: cdrom.file_types.first(),
            icon: Icon::Disc,
            current: Some("games/98/YU-NO/Manual/x.cue".to_owned()),
        };
        let mut browser = FileBrowser::new(request, &root.root);
        assert_eq!(browser.folder(), root.path("games/98/YU-NO/Manual"));
        assert_eq!(names(&browser), [".."]);
        assert_eq!(
            browser.english_empty_text().as_deref(),
            Some("No disc images in this folder")
        );
        assert_eq!(
            browser.selected_row().and_then(|row| browser.chosen(row)),
            None
        );
        let mut harness = GuiHarness::new(960, 600);
        harness.settle(|context| browser.show(context, AREA));
        harness.state.set_focus(UiId::from(SHOW_ALL_ID));
        harness.settle(|context| browser.show(context, AREA));
        assert_eq!(texts::english_labels(browser.hints()), ["Switch", "Close"]);
        harness.input.confirm = true;
        harness.settle(|context| browser.show(context, AREA));
        assert!(browser.show_all);
        assert_eq!(names(&browser), ["..", "manual.pdf"]);
        assert_eq!(browser.english_empty_text(), None);
    }

    /// Verifies that a hint with the result of an earlier hint leaves the line: Backspace beside the selected parent row, and Esc beside the focused Cancel button.
    #[test]
    fn hints_of_one_result_show_once() {
        let root = games_root("browser_hints");
        let cdrom = find_setting("cdrom").expect("cdrom is a table key");
        let request = BrowseRequest {
            noun: BrowseNoun::Image,
            label: "CD-ROM",
            kind: PathKind::File,
            types: cdrom.file_types.first(),
            icon: Icon::Disc,
            current: Some("games/98/YU-NO/Manual/x.cue".to_owned()),
        };
        let mut browser = FileBrowser::new(request, &root.root);
        assert_eq!(names(&browser), [".."]);
        let mut harness = GuiHarness::new(960, 600);
        harness.settle(|context| browser.show(context, AREA));
        harness.state.set_focus(UiId::from(LIST_ID));
        harness.settle(|context| browser.show(context, AREA));
        assert_eq!(selected(&browser), "..");
        let labels = |browser: &FileBrowser| texts::english_labels(browser.hints());
        assert_eq!(labels(&browser), ["Open", "Close"]);
        harness.state.set_focus(UiId::from(CANCEL_ID));
        harness.settle(|context| browser.show(context, AREA));
        assert_eq!(labels(&browser), ["Cancel"]);
    }

    /// Verifies that a folder setting lists only folders and chooses the selected folder or the folder of the parent row.
    #[test]
    fn a_folder_setting_chooses_folders() {
        let root = games_root("browser_folders");
        let request = BrowseRequest {
            noun: BrowseNoun::Folder,
            label: "Config folder",
            kind: PathKind::Folder,
            types: None,
            icon: Icon::Folder,
            current: Some("games/98/YU-NO".to_owned()),
        };
        let browser = FileBrowser::new(request, &root.root);
        assert_eq!(browser.folder(), root.path("games/98"));
        assert_eq!(names(&browser), ["..", "YU-NO/"]);
        assert_eq!(selected(&browser), "YU-NO/");
        let row = browser.selected_row().expect("selected");
        assert_eq!(browser.chosen(row).as_deref(), Some("games/98/YU-NO"));
        assert_eq!(
            browser.chosen(BrowserRow::Parent).as_deref(),
            Some("games/98")
        );
    }
}
