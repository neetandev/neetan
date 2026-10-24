//! The game editor: one form for a game file with the sections Metadata,
//! Machine, Media, Sound, Display and Advanced.
//!
//! The settings table generates the fields for the family of the game. An
//! unset field shows the value that it inherits from the layers below the
//! game file. The editor keeps every line of the file that it does not show
//! as a field, and a save writes only the fields that changed.

mod browse;
mod derived_file;
mod discard;
mod fields;
mod file;
mod form;
mod hints;
mod inherited;
mod kept_lines;
mod media_list;
mod media_row;
mod notes;
mod page;
mod rows;
mod save;
#[cfg(test)]
pub(crate) mod test_support;

use std::path::{Path, PathBuf};

use gui::{
    Rect,
    ui::{Hint, ScrollState, UiContext},
};
use machine_factory::config::{
    MachineSelection, Target,
    settings::{MACHINE_KEY, PathKind, Setting, ValueKind},
    systems::System,
};

pub(crate) use self::file::SaveFailure;
use self::{
    browse::BrowseTarget,
    derived_file::DerivedFile,
    discard::{DiscardOutcome, DiscardQuestion},
    fields::{Field, FieldValue},
    file::{EditorFile, fill_required, write_game_file},
    form::{EDITOR_SECTIONS, EditorSection, form_fields},
    hints::EditorWidget,
    inherited::{BaseLayers, Inherited},
    kept_lines::kept_lines,
    media_list::is_missing,
    save::{FileStamp, edited_document, read_document, validation_error},
};
#[cfg(test)]
pub(crate) use self::{media_list::empty_list_id, rows::field_id};
use crate::{
    catalog::copy_title,
    config::{ConfigDocument, HostLayers, metadata::TITLE_KEY},
    file_browser::{BrowserOutcome, FileBrowser},
    manager::shown_path,
};

/// The keys that every game file of the editor holds. Their dropdowns have
/// no inherited entry, and a save writes their inherited value into a file
/// without them.
const GAME_REQUIRED_KEYS: &[&str] = &[MACHINE_KEY];

/// What the editor did in a pass.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct EditorPass {
    /// The editor closes.
    pub(crate) closed: bool,
    /// The game file at this path was saved.
    pub(crate) saved: Option<PathBuf>,
    /// The save failed.
    pub(crate) save_error: Option<SaveFailure>,
}

/// The open game editor.
pub(crate) struct Editor {
    /// The system of the game.
    system: &'static System,
    /// The family of the game.
    target: Target,
    /// The root folder.
    root: PathBuf,
    /// The file that a save writes.
    file: EditorFile,
    /// The lines of the file when the editor opened or saved last.
    document: ConfigDocument,
    /// The modification time and the size of the file when the editor read
    /// or saved it last. A new game has none.
    stamp: Option<FileStamp>,
    /// The fields of the form in tab order.
    fields: Vec<Field>,
    /// The settings file and the system defaults below the game.
    base: BaseLayers,
    /// The model that a new game starts with.
    preset_model: Option<&'static str>,
    /// The inherited value of each field. A metadata field has none.
    inherited: Vec<Option<Inherited>>,
    /// Whether each image of each field is missing, in the order of its values.
    missing: Vec<Vec<bool>>,
    /// The lines of the file that no field shows.
    kept: Vec<String>,
    /// The index of the selected section in `EDITOR_SECTIONS`.
    section: usize,
    /// The scroll position of the form.
    scroll: ScrollState,
    /// The open file browser and the field it fills.
    browser: Option<(FileBrowser, BrowseTarget)>,
    /// The open question before the changes go away.
    discard: Option<DiscardQuestion>,
    /// Whether the first widget of the section takes the focus in the next pass.
    focus_first: bool,
    /// The focusable widgets of the last pass.
    widgets: Vec<EditorWidget>,
    /// The hints of the status line of the last pass.
    hints: Vec<Hint<'static>>,
}

impl Editor {
    /// An editor on the lines `document` of `file` of a game of `system`,
    /// with the stamp of the read of an existing file. The keys of
    /// `required` are part of every file that the editor saves.
    fn with_document(
        host: &HostLayers,
        system: &'static System,
        file: EditorFile,
        document: ConfigDocument,
        stamp: Option<FileStamp>,
        required: &[&str],
    ) -> Option<Self> {
        let target = MachineSelection::of_system(system)?.target();
        let fields: Vec<Field> = form_fields(target)
            .into_iter()
            .map(|(section, source)| {
                let required = required.contains(&source.key());
                Field::read(section, source, target, &document, required)
            })
            .collect();
        let kept = kept_lines(&document, &fields);
        let mut editor = Self {
            system,
            target,
            root: host.root().to_path_buf(),
            file,
            document,
            stamp,
            inherited: vec![None; fields.len()],
            missing: vec![Vec::new(); fields.len()],
            fields,
            base: BaseLayers::read(host, target),
            preset_model: None,
            kept,
            section: 0,
            scroll: ScrollState::new(),
            browser: None,
            discard: None,
            focus_first: true,
            widgets: Vec::new(),
            hints: Vec::new(),
        };
        editor.refresh();
        Some(editor)
    }

    /// An editor on the game file at `path` of `system`.
    pub(crate) fn open(
        host: &HostLayers,
        system: &'static System,
        path: &Path,
    ) -> Result<Self, String> {
        let (document, stamp) = read_document(path)?;
        Self::with_document(
            host,
            system,
            EditorFile::Existing(path.to_path_buf()),
            document,
            Some(stamp),
            GAME_REQUIRED_KEYS,
        )
        .ok_or_else(|| format!("the system {} is not compiled", system.name))
    }

    /// An editor on a new game of `system`. Its machine starts with the
    /// model that a game of the system takes without `machine`.
    pub(crate) fn new_game(host: &HostLayers, system: &'static System) -> Option<Self> {
        let folder = host.config_folder().join(system.name);
        let file = EditorFile::New(DerivedFile::new(&folder, ""));
        let mut editor = Self::with_document(
            host,
            system,
            file,
            ConfigDocument::new(),
            None,
            GAME_REQUIRED_KEYS,
        )?;
        let model = editor.base.model().0.name();
        if let Some(field) = editor.field_mut(MACHINE_KEY)
            && let Some(index) = field.choices.iter().position(|choice| *choice == model)
        {
            field.select(Some(index));
        }
        editor.preset_model = Some(model);
        editor.refresh();
        Some(editor)
    }

    /// An editor on a copy of the game file at `source` of `system`. The
    /// copy keeps every line and appends ` (copy)` to the title, or to the
    /// file stem of a game without a title.
    pub(crate) fn duplicate(
        host: &HostLayers,
        system: &'static System,
        source: &Path,
    ) -> Result<Self, String> {
        let document = ConfigDocument::load(source)
            .map_err(|error| format!("{}: {error}", source.display()))?;
        let stem = source
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let title = match document.get(TITLE_KEY).map(str::trim) {
            Some(title) if !title.is_empty() => title.to_owned(),
            Some(_) | None => stem,
        };
        let title = copy_title(&title);
        let folder = host.config_folder().join(system.name);
        let file = EditorFile::Copy {
            source: shown_path(source, host.root()),
            file: DerivedFile::new(&folder, &title),
        };
        let mut editor =
            Self::with_document(host, system, file, document, None, GAME_REQUIRED_KEYS)
                .ok_or_else(|| format!("the system {} is not compiled", system.name))?;
        if let Some(field) = editor.field_mut(TITLE_KEY) {
            field.value = FieldValue::Text(title);
        }
        editor.refresh();
        Ok(editor)
    }

    /// The field of `key`.
    fn field_mut(&mut self, key: &str) -> Option<&mut Field> {
        self.fields.iter_mut().find(|field| field.key() == key)
    }

    /// The field of `key`.
    fn field(&self, key: &str) -> Option<&Field> {
        self.fields.iter().find(|field| field.key() == key)
    }

    /// The system of the game.
    pub(crate) fn system(&self) -> &'static System {
        self.system
    }

    /// The path of the file that a save writes.
    pub(crate) fn path(&self) -> PathBuf {
        self.file.path()
    }

    /// The path of the file to show: below the root when it lies there.
    pub(crate) fn shown_path(&self) -> String {
        shown_path(&self.path(), &self.root)
    }

    /// The hints of the status line of the last pass.
    pub(crate) fn hints(&self) -> &[Hint<'static>] {
        &self.hints
    }

    /// The model of the game: the chosen model, else the inherited model.
    fn model(&self) -> MachineSelection {
        self.field(MACHINE_KEY)
            .and_then(Field::chosen)
            .and_then(|name| name.parse().ok())
            .unwrap_or_else(|| self.base.model().0)
    }

    /// The name of the game: its title, else its file stem.
    pub(crate) fn display_name(&self) -> String {
        let title = self.field(TITLE_KEY).map_or("", Field::text).trim();
        if !title.is_empty() {
            return title.to_owned();
        }
        self.path()
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// Whether a save would change the file: a field changed, or the file
    /// does not exist yet.
    fn is_dirty(&self) -> bool {
        self.file.derived().is_some() || self.fields.iter().any(Field::changed)
    }

    /// Computes the inherited values, the missing images and the file of a
    /// game without a save again after a change of a field.
    fn refresh(&mut self) {
        let model = self.model();
        let game: Vec<(&'static Setting, Vec<String>)> = self
            .fields
            .iter()
            .filter_map(|field| {
                field
                    .source
                    .setting()
                    .map(|setting| (setting, field.values()))
            })
            .collect();
        for (index, field) in self.fields.iter().enumerate() {
            let Some(setting) = field.source.setting() else {
                continue;
            };
            self.inherited[index] = Some(self.base.inherited(setting, model, &game));
            self.missing[index] = match setting.kind {
                ValueKind::Path(PathKind::File) | ValueKind::PathList => field
                    .values()
                    .iter()
                    .map(|value| is_missing(value, &self.root))
                    .collect(),
                ValueKind::Path(PathKind::Folder)
                | ValueKind::Choice(_)
                | ValueKind::OnOff
                | ValueKind::Number(_) => Vec::new(),
            };
        }
        let title = self.field(TITLE_KEY).map(|field| field.text().to_owned());
        match &mut self.file {
            EditorFile::Existing(_) => {}
            EditorFile::New(file) | EditorFile::Copy { file, .. } => {
                *file = DerivedFile::new(&file.folder, title.as_deref().unwrap_or_default());
            }
        }
    }

    /// The selected section.
    fn current_section(&self) -> EditorSection {
        EDITOR_SECTIONS[self.section.min(EDITOR_SECTIONS.len() - 1)]
    }

    /// Writes the changed fields into the file and returns its path.
    ///
    /// A required field without a value writes its inherited value. A
    /// game file that changed since the editor read it stays as it is,
    /// unless `overwrite` asks to write over the change. A new game or a
    /// copy takes its derived file name once more before the write and
    /// keeps it after. Its write never replaces a file. A failed save
    /// leaves the fields as they are.
    fn save(&mut self, overwrite: bool) -> Result<PathBuf, SaveFailure> {
        self.refresh();
        let mut fields = self.fields.clone();
        fill_required(&mut fields, &self.inherited);
        let edited = edited_document(&self.document, &fields);
        let title = self.field(TITLE_KEY).map_or("", Field::text).to_owned();
        let written = write_game_file(
            &self.file, &edited, self.stamp, &title, &self.root, overwrite,
        )?;
        let path = written.path;
        self.stamp = written.stamp;
        self.document = edited;
        self.fields = fields;
        for field in &mut self.fields {
            field.mark_saved();
        }
        self.file = EditorFile::Existing(path.clone());
        self.preset_model = None;
        self.kept = kept_lines(&self.document, &self.fields);
        self.refresh();
        Ok(path)
    }

    /// Saves the changes over a game file that changed since the editor
    /// read it, and takes the stamp of the written file.
    pub(crate) fn overwrite(&mut self) -> Result<PathBuf, SaveFailure> {
        self.save(true)
    }

    /// Asks to close the editor: with unsaved changes the question opens,
    /// else the editor closes. Returns whether it closes.
    fn request_close(&mut self) -> bool {
        if self.is_dirty() {
            self.discard = Some(DiscardQuestion::new(&self.display_name()));
            false
        } else {
            true
        }
    }

    /// Draws one pass of the editor into `screen`, the window above the
    /// status line, with its file browser and its question on top.
    pub(crate) fn show(&mut self, context: &mut UiContext, screen: Rect) -> EditorPass {
        let mut pass = EditorPass::default();
        self.widgets.clear();
        let actions = self.draw_page(context, screen);
        if actions.changed {
            self.refresh();
            context.request_pass();
        }
        if (actions.save || actions.save_shortcut)
            && self.is_dirty()
            && validation_error(&self.fields).is_none()
        {
            match self.save(false) {
                Ok(path) => pass.saved = Some(path),
                Err(error) => pass.save_error = Some(error),
            }
            context.request_pass();
        }
        if actions.close && self.request_close() {
            pass.closed = true;
        }
        if let Some(target) = actions.browse {
            let request = self.browse_request(target);
            self.browser = Some((FileBrowser::new(request, &self.root), target));
        }

        if let Some((mut browser, target)) = self.browser.take() {
            match browser.show(context, screen) {
                BrowserOutcome::Open => self.browser = Some((browser, target)),
                BrowserOutcome::Cancelled => {}
                BrowserOutcome::Chosen(value) => {
                    self.apply_browsed(target, value);
                    context.request_pass();
                }
            }
        }
        if let Some(mut question) = self.discard.take() {
            match question.show(context, screen) {
                DiscardOutcome::Open => self.discard = Some(question),
                DiscardOutcome::Kept => {}
                DiscardOutcome::Discarded => pass.closed = true,
            }
        }
        self.hints = self.status_hints(context);
        pass
    }

    /// The hints of the status line after a pass.
    fn status_hints(&self, context: &UiContext) -> Vec<Hint<'static>> {
        if let Some((browser, _)) = &self.browser {
            return browser.hints().to_vec();
        }
        if let Some(question) = &self.discard {
            return question.hints();
        }
        hints::editor_hints(context, &self.widgets)
    }
}

#[cfg(test)]
mod tests {
    use gui::ui::{Navigation, Shortcut};
    use machine_factory::config::settings::find_setting;

    use super::{media_list::MediaBrowse, rows::field_id, test_support::EditorRoot, *};
    use crate::{config::check_value, gui_test_support::GuiHarness};

    /// The window above the status line in the tests.
    const SCREEN: Rect = Rect::new(0.0, 0.0, 960.0, 572.0);

    /// Draws passes of `editor` until they settle and returns the answers of the passes.
    fn show(harness: &mut GuiHarness, editor: &mut Editor) -> EditorPass {
        let mut answer = EditorPass::default();
        harness.settle(|context| {
            let pass = editor.show(context, SCREEN);
            answer.closed |= pass.closed;
            answer.saved = answer.saved.clone().or(pass.saved);
            answer.save_error = answer.save_error.clone().or(pass.save_error);
        });
        answer
    }

    /// The editor on the game `stem` of `root` with `contents`.
    fn open(root: &EditorRoot, stem: &str, contents: &str) -> Editor {
        let path = root.game(stem, contents);
        Editor::open(&root.host(), root.system(), &path).expect("the editor opens")
    }

    /// Selects the section `section` and draws it.
    fn select_section(harness: &mut GuiHarness, editor: &mut Editor, section: EditorSection) {
        editor.section = EDITOR_SECTIONS
            .iter()
            .position(|candidate| *candidate == section)
            .expect("a section");
        editor.focus_first = true;
        show(harness, editor);
    }

    /// Verifies that the editor starts on the title and that the shoulder buttons switch the section and focus its first field.
    #[test]
    fn the_editor_starts_on_the_title_and_switches_sections() {
        let root = EditorRoot::new("editor_sections");
        let mut editor = open(&root, "edge", "meta.title = Edge\n");
        let mut harness = GuiHarness::new(960, 600);
        show(&mut harness, &mut editor);
        assert_eq!(harness.state.focused(), Some(field_id("meta.title")));
        harness.input.navigation = Some(Navigation::NextTab);
        show(&mut harness, &mut editor);
        assert_eq!(editor.current_section(), EditorSection::Machine);
        assert_eq!(harness.state.focused(), Some(field_id(MACHINE_KEY)));
        harness.input.navigation = Some(Navigation::PreviousTab);
        show(&mut harness, &mut editor);
        harness.input.navigation = Some(Navigation::PreviousTab);
        show(&mut harness, &mut editor);
        assert_eq!(editor.current_section(), EditorSection::Advanced);
        assert_eq!(
            editor
                .hints()
                .iter()
                .map(|hint| hint.label)
                .collect::<Vec<_>>(),
            ["Section", "Save", "Close"]
        );
    }

    /// Verifies that a dropdown sets its key, that its inherited entry removes the key, that Ctrl+S saves from a text field and that the save writes the inherited model into a file without `machine`.
    #[test]
    fn a_dropdown_sets_and_removes_its_key() {
        let root = EditorRoot::new("editor_dropdown");
        let mut editor = open(&root, "edge", "meta.title = Edge\n");
        let mut harness = GuiHarness::new(960, 600);
        show(&mut harness, &mut editor);
        select_section(&mut harness, &mut editor, EditorSection::Display);
        harness.state.set_focus(field_id("crt"));
        show(&mut harness, &mut editor);
        harness.input.confirm = true;
        show(&mut harness, &mut editor);
        assert_eq!(
            editor
                .hints()
                .iter()
                .map(|hint| hint.label)
                .collect::<Vec<_>>(),
            ["Choose", "Close the popup"]
        );
        harness.input.navigation = Some(Navigation::Down);
        show(&mut harness, &mut editor);
        harness.input.confirm = true;
        show(&mut harness, &mut editor);
        assert_eq!(editor.field("crt").and_then(Field::chosen), Some("on"));
        assert!(editor.is_dirty());
        harness.input.shortcut = Some(Shortcut::ControlS);
        let pass = show(&mut harness, &mut editor);
        assert_eq!(pass.saved, Some(editor.path()));
        let model = root.system().default_model;
        assert_eq!(
            std::fs::read_to_string(editor.path()).expect("file reads"),
            format!("meta.title = Edge\nmachine = {model}\ncrt = on\n")
        );
        assert!(!editor.is_dirty());

        harness.input.confirm = true;
        show(&mut harness, &mut editor);
        harness.input.navigation = Some(Navigation::Home);
        show(&mut harness, &mut editor);
        harness.input.confirm = true;
        show(&mut harness, &mut editor);
        assert_eq!(editor.field("crt").and_then(Field::chosen), None);
        harness.state.set_focus(field_id("meta.title"));
        select_section(&mut harness, &mut editor, EditorSection::Metadata);
        harness.input.shortcut = Some(Shortcut::ControlS);
        show(&mut harness, &mut editor);
        assert_eq!(
            std::fs::read_to_string(editor.path()).expect("file reads"),
            format!("meta.title = Edge\nmachine = {model}\n")
        );
    }

    /// Verifies that the file browser of a media list adds the chosen image relative to the root and that a missing image is a warning.
    #[test]
    fn the_browser_adds_an_image_to_a_media_list() {
        let root = EditorRoot::new("editor_browse");
        root.write("games/disk.d88", "");
        let fdd1_setting = find_setting("fdd1").expect("fdd1 is a table key");
        let model = machine_factory::config::settings::MODEL_CHOICES
            .iter()
            .map(|choice| choice.value)
            .find(|model| fdd1_setting.is_read_by(root.target(), model))
            .expect("a model of the family reads fdd1");
        let mut editor = open(
            &root,
            "edge",
            &format!("machine = {model}\nfdd1 = games/missing.d88\n"),
        );
        let fdd1 = editor
            .fields
            .iter()
            .position(|field| field.key() == "fdd1")
            .expect("fdd1 field");
        assert_eq!(editor.missing[fdd1], [true]);
        let mut harness = GuiHarness::new(960, 600);
        show(&mut harness, &mut editor);
        select_section(&mut harness, &mut editor, EditorSection::Media);
        assert_eq!(harness.state.focused(), Some(field_id("fdd1")));
        let add = field_id("fdd1").child("add");
        harness.state.set_focus(add);
        show(&mut harness, &mut editor);
        harness.input.confirm = true;
        show(&mut harness, &mut editor);
        assert!(editor.browser.is_some());
        harness.input.confirm = true;
        show(&mut harness, &mut editor);
        assert!(editor.browser.is_none());
        assert_eq!(
            editor.fields[fdd1].values(),
            ["games/missing.d88", "games/disk.d88"]
        );
        assert_eq!(editor.missing[fdd1], [true, false]);
        assert_eq!(editor.fields[fdd1].list.selected, 1);

        let outside = std::env::temp_dir().join("neetan-outside.d88");
        editor.apply_browsed(
            BrowseTarget::List(fdd1, MediaBrowse::Replace(0)),
            crate::file_browser::path_value(&outside, &editor.root),
        );
        assert_eq!(editor.fields[fdd1].values()[0], outside.to_string_lossy());
    }

    /// Verifies that Esc closes an editor without changes and asks first with changes, and that the answer Keep editing keeps the changes.
    #[test]
    fn escape_asks_before_unsaved_changes_go_away() {
        let root = EditorRoot::new("editor_discard");
        let mut editor = open(&root, "edge", "meta.title = Edge\n");
        let mut harness = GuiHarness::new(960, 600);
        show(&mut harness, &mut editor);
        harness.input.cancel = true;
        assert!(show(&mut harness, &mut editor).closed);

        let mut editor = open(&root, "edge", "meta.title = Edge\n");
        assert!(!editor.is_dirty());
        show(&mut harness, &mut editor);
        harness.input.text = "!".to_owned();
        show(&mut harness, &mut editor);
        harness.input.cancel = true;
        let pass = harness.frame(|context| editor.show(context, SCREEN));
        assert!(!pass.closed);
        assert!(editor.discard.is_some());
        assert_eq!(harness.state.open_modals(), 1);
        show(&mut harness, &mut editor);
        assert!(editor.discard.is_some());
        assert_eq!(
            editor
                .hints()
                .iter()
                .map(|hint| hint.label)
                .collect::<Vec<_>>(),
            ["Keep editing"]
        );
        harness.input.confirm = true;
        assert!(!show(&mut harness, &mut editor).closed);
        assert!(editor.discard.is_none());
        assert_eq!(editor.field(TITLE_KEY).map(Field::text), Some("Edge!"));
        harness.input.cancel = true;
        show(&mut harness, &mut editor);
        harness.input.navigation = Some(Navigation::Right);
        show(&mut harness, &mut editor);
        harness.input.confirm = true;
        assert!(show(&mut harness, &mut editor).closed);
        assert_eq!(
            std::fs::read_to_string(editor.path()).expect("file reads"),
            "meta.title = Edge\n"
        );
    }

    /// Verifies that a new game starts with the default model, that its file follows the title until the first save and stays after it.
    #[test]
    fn a_new_game_keeps_its_file_after_the_first_save() {
        let root = EditorRoot::new("editor_new");
        let system = root.system();
        let mut editor = Editor::new_game(&root.host(), system).expect("compiled system");
        assert_eq!(
            editor.field(MACHINE_KEY).and_then(Field::chosen),
            Some(system.default_model)
        );
        assert!(editor.is_dirty());
        let mut harness = GuiHarness::new(960, 600);
        show(&mut harness, &mut editor);
        harness.input.text = "Ys II".to_owned();
        show(&mut harness, &mut editor);
        let folder = root.path(&format!("config/{}", system.name));
        assert_eq!(editor.path(), folder.join("ys-ii.conf"));
        harness.input.shortcut = Some(Shortcut::ControlS);
        let pass = show(&mut harness, &mut editor);
        assert_eq!(pass.saved, Some(folder.join("ys-ii.conf")));
        harness.input.text = " Eternal".to_owned();
        show(&mut harness, &mut editor);
        assert_eq!(editor.path(), folder.join("ys-ii.conf"));
        assert!(matches!(editor.file, EditorFile::Existing(_)));
        assert_eq!(
            std::fs::read_to_string(folder.join("ys-ii.conf")).expect("file reads"),
            format!("meta.title = Ys II\nmachine = {}\n", system.default_model)
        );
    }

    /// Verifies that a number field with a value outside its range blocks the save and shows the error in the footer.
    #[test]
    fn a_bad_year_blocks_the_save() {
        let root = EditorRoot::new("editor_year");
        let mut editor = open(&root, "edge", "meta.year = 1993\n");
        let mut harness = GuiHarness::new(960, 600);
        show(&mut harness, &mut editor);
        harness.state.set_focus(field_id("meta.year"));
        show(&mut harness, &mut editor);
        for _ in 0..4 {
            harness.input.backspace = true;
            show(&mut harness, &mut editor);
        }
        harness.input.text = "19".to_owned();
        show(&mut harness, &mut editor);
        harness.input.shortcut = Some(Shortcut::ControlS);
        assert_eq!(show(&mut harness, &mut editor).saved, None);
        assert_eq!(
            std::fs::read_to_string(editor.path()).expect("file reads"),
            "meta.year = 1993\n"
        );
    }

    #[cfg(feature = "pc98")]
    /// Verifies that a change of the model enables the settings that the new model reads.
    #[test]
    fn the_model_decides_the_enabled_settings() {
        let root = EditorRoot::with_system("editor_model", "pc98");
        let mut editor = open(&root, "edge", "machine = PC9801RA\n");
        let cpu_mode = find_setting("cpu-mode").expect("table key");
        let model = editor.model();
        assert!(form::ignored_note(cpu_mode, model).is_some());
        let machine = editor.field_mut(MACHINE_KEY).expect("machine field");
        let index = machine
            .choices
            .iter()
            .position(|choice| *choice == "PC9801VM")
            .expect("a PC-98 model");
        machine.select(Some(index));
        editor.refresh();
        assert_eq!(editor.model().name(), "PC9801VM");
        assert!(form::ignored_note(cpu_mode, editor.model()).is_none());
        let bios = editor
            .fields
            .iter()
            .position(|field| field.key() == "bios")
            .expect("bios field");
        assert_eq!(
            editor.inherited[bios]
                .as_ref()
                .map(Inherited::text)
                .as_deref(),
            Some("off")
        );
    }

    /// Verifies that the model dropdown of a game has no inherited entry and shows the inherited model of a file without `machine`.
    #[test]
    fn the_model_dropdown_has_no_inherited_entry() {
        let root = EditorRoot::new("editor_required");
        let mut editor = open(&root, "edge", "meta.title = Edge\n");
        let machine = editor.field(MACHINE_KEY).expect("machine field");
        assert!(machine.required && !machine.is_set());
        let mut harness = GuiHarness::new(960, 600);
        show(&mut harness, &mut editor);
        select_section(&mut harness, &mut editor, EditorSection::Machine);
        assert_eq!(harness.state.focused(), Some(field_id(MACHINE_KEY)));
        harness.input.confirm = true;
        show(&mut harness, &mut editor);
        harness.input.navigation = Some(Navigation::Home);
        show(&mut harness, &mut editor);
        harness.input.confirm = true;
        show(&mut harness, &mut editor);
        let machine = editor.field(MACHINE_KEY).expect("machine field");
        let first = machine.choices[0];
        if first == editor.base.model().0.name() {
            assert!(!editor.is_dirty());
        } else {
            assert_eq!(machine.chosen(), Some(first));
            assert!(editor.is_dirty());
        }
    }

    /// Verifies that a save over a game file that another program changed keeps the file, the changes and the unset required field, and that an overwrite writes over the change and takes the new stamp.
    #[test]
    fn a_file_changed_outside_stays_until_an_overwrite() {
        let root = EditorRoot::new("editor_outside");
        let mut editor = open(&root, "edge", "meta.title = Edge\n");
        let mut harness = GuiHarness::new(960, 600);
        show(&mut harness, &mut editor);
        harness.input.text = "!".to_owned();
        show(&mut harness, &mut editor);
        std::fs::write(editor.path(), "meta.title = Edge 2\ncrt = on\n").expect("file is written");
        harness.input.shortcut = Some(Shortcut::ControlS);
        let pass = show(&mut harness, &mut editor);
        assert_eq!(pass.saved, None);
        let failure = pass.save_error.expect("the save fails");
        assert!(
            matches!(failure, SaveFailure::ChangedOutside(_)),
            "{failure:?}"
        );
        assert!(
            failure.message().contains("changed outside the manager"),
            "{failure:?}"
        );
        assert_eq!(
            std::fs::read_to_string(editor.path()).expect("file reads"),
            "meta.title = Edge 2\ncrt = on\n"
        );
        assert!(editor.is_dirty());
        assert_eq!(editor.field(TITLE_KEY).map(Field::text), Some("Edge!"));
        let machine = editor.field(MACHINE_KEY).expect("machine field");
        assert!(!machine.is_set() && !machine.changed());

        let model = root.system().default_model;
        assert_eq!(editor.overwrite(), Ok(editor.path()));
        assert_eq!(
            std::fs::read_to_string(editor.path()).expect("file reads"),
            format!("meta.title = Edge!\nmachine = {model}\n")
        );
        assert!(!editor.is_dirty());
        assert!(editor.field(MACHINE_KEY).is_some_and(Field::is_set));
        harness.input.text = "?".to_owned();
        show(&mut harness, &mut editor);
        harness.input.shortcut = Some(Shortcut::ControlS);
        assert_eq!(show(&mut harness, &mut editor).saved, Some(editor.path()));
    }

    /// Verifies that a save that fails to write leaves the required field unset and the changes in place.
    #[test]
    fn a_failed_save_leaves_the_fields_as_they_are() {
        let root = EditorRoot::new("editor_failed_save");
        let mut editor = open(&root, "edge", "meta.title = Edge\n");
        let mut harness = GuiHarness::new(960, 600);
        show(&mut harness, &mut editor);
        harness.input.text = "!".to_owned();
        show(&mut harness, &mut editor);
        let path = editor.path();
        std::fs::remove_file(&path).expect("file is removed");
        std::fs::create_dir_all(&path).expect("a folder takes the place of the file");
        let failure = editor
            .overwrite()
            .expect_err("a folder takes the place of the file");
        assert!(matches!(failure, SaveFailure::Error(_)), "{failure:?}");
        let machine = editor.field(MACHINE_KEY).expect("machine field");
        assert!(!machine.is_set() && !machine.changed());
        assert_eq!(editor.field(TITLE_KEY).map(Field::text), Some("Edge!"));
        assert!(editor.is_dirty());
    }

    /// Verifies that the save of a new game takes the next suffix when a file takes its name first.
    #[test]
    fn a_new_game_never_replaces_a_file() {
        let root = EditorRoot::new("editor_no_clobber");
        let system = root.system();
        let mut editor = Editor::new_game(&root.host(), system).expect("compiled system");
        let mut harness = GuiHarness::new(960, 600);
        show(&mut harness, &mut editor);
        harness.input.text = "Edge".to_owned();
        show(&mut harness, &mut editor);
        let folder = root.path(&format!("config/{}", system.name));
        assert_eq!(editor.path(), folder.join("edge.conf"));
        root.game("edge", "meta.title = Another edge\n");
        harness.input.shortcut = Some(Shortcut::ControlS);
        let pass = show(&mut harness, &mut editor);
        assert_eq!(pass.saved, Some(folder.join("edge-2.conf")));
        assert_eq!(
            std::fs::read_to_string(folder.join("edge.conf")).expect("file reads"),
            "meta.title = Another edge\n"
        );
    }

    /// Verifies that gamepad X clears a focused set text field to unset.
    #[test]
    fn gamepad_x_clears_a_text_field() {
        let root = EditorRoot::new("editor_clear");
        let mut editor = open(&root, "edge", "meta.title = Edge\nmeta.developer = TGL\n");
        let mut harness = GuiHarness::new(960, 600);
        show(&mut harness, &mut editor);
        harness.state.set_focus(field_id("meta.developer"));
        show(&mut harness, &mut editor);
        assert_eq!(
            editor
                .hints()
                .iter()
                .map(|hint| hint.label)
                .collect::<Vec<_>>(),
            ["Clear", "Section", "Save", "Close"]
        );
        harness.input.action = Some(gui::ui::ActionButton::X);
        show(&mut harness, &mut editor);
        let developer = editor.field("meta.developer").expect("developer field");
        assert!(!developer.is_set());
        assert!(editor.is_dirty());
        assert_eq!(
            editor
                .hints()
                .iter()
                .map(|hint| hint.label)
                .collect::<Vec<_>>(),
            ["Section", "Save", "Close"]
        );
    }

    /// Verifies that an empty value of a drive is an empty list over the inherited list, and that its checkbox returns the drive to the inherited list.
    #[test]
    fn an_empty_drive_overrides_the_inherited_list() {
        let root = EditorRoot::new("editor_empty_list");
        root.write("neetan.conf", "fdd1 = games/boot.d88\n");
        let fdd1_setting = find_setting("fdd1").expect("fdd1 is a table key");
        let model = machine_factory::config::settings::MODEL_CHOICES
            .iter()
            .map(|choice| choice.value)
            .find(|model| fdd1_setting.is_read_by(root.target(), model))
            .expect("a model of the family reads fdd1");
        let mut editor = open(&root, "edge", &format!("machine = {model}\nfdd1 =\n"));
        let fdd1 = editor.field("fdd1").expect("fdd1 field");
        assert!(fdd1.is_empty_list());
        let mut harness = GuiHarness::new(960, 600);
        show(&mut harness, &mut editor);
        select_section(&mut harness, &mut editor, EditorSection::Media);
        let checkbox = media_list::empty_list_id(field_id("fdd1"));
        harness.state.set_focus(checkbox);
        show(&mut harness, &mut editor);
        assert_eq!(harness.state.focused(), Some(checkbox));
        harness.input.confirm = true;
        show(&mut harness, &mut editor);
        let fdd1 = editor.field("fdd1").expect("fdd1 field");
        assert!(!fdd1.is_set());
        assert_eq!(harness.state.focused(), Some(checkbox));
        harness.input.confirm = true;
        show(&mut harness, &mut editor);
        assert!(editor.field("fdd1").is_some_and(Field::is_empty_list));
        assert!(!editor.is_dirty());
        harness.input.confirm = true;
        show(&mut harness, &mut editor);
        harness.input.shortcut = Some(Shortcut::ControlS);
        show(&mut harness, &mut editor);
        assert_eq!(
            std::fs::read_to_string(editor.path()).expect("file reads"),
            format!("machine = {model}\n")
        );
    }

    /// Verifies that a value of the file that the launch rejects stays in the file and is invalid in the editor.
    #[test]
    fn a_value_with_another_case_is_invalid() {
        let root = EditorRoot::new("editor_invalid");
        let editor = open(&root, "edge", "crt = On\n");
        let crt = editor.field("crt").expect("crt field");
        let invalid = crt.invalid_value().expect("the launch rejects On");
        assert_eq!(invalid.message, "invalid value 'On', expected on or off");
        assert_eq!(
            check_value(find_setting("crt").expect("crt"), "On")
                .err()
                .as_deref(),
            Some(invalid.message.as_str())
        );
    }
}
