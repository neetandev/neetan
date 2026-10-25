//! The game editor: one form for a game file with the sections Metadata,
//! Machine, Media, Sound, Display and Advanced. The same form without the
//! Metadata section edits the system defaults file of a system. The settings
//! page is the same form on the host keys of the settings file, with the
//! sections Folders, ROMs and Video and audio.
//!
//! The settings table generates the fields for the family of the game or
//! for the host. An unset field shows the value that it inherits from the
//! layers below the file. The editor keeps every line of the file that it
//! does not show as a field, and a save writes only the fields that changed.

mod browse;
mod derived_file;
mod discard;
mod fields;
mod file;
mod form;
mod hints;
mod host_settings;
mod inherited;
mod kept_lines;
mod language_row;
mod media_list;
mod media_row;
mod note_writer;
mod notes;
mod page;
mod rom_rows;
mod rows;
mod save;
mod system_defaults;
#[cfg(test)]
pub(crate) mod test_support;
mod value_rows;

use std::path::{Path, PathBuf};

use gui::{
    Rect,
    ui::{ScrollState, UiContext},
};
use i18n::Catalog;
use machine_factory::config::{
    MachineSelection, Target,
    settings::{
        Families, GUI_FONT_KEY, MACHINE_KEY, PathKind, Setting, ValueKind, file_types::FileTypes,
    },
    systems::System,
};

#[cfg(test)]
pub(crate) use self::rows::field_id;
use self::{
    browse::BrowseTarget,
    derived_file::DerivedFile,
    discard::{DiscardOutcome, DiscardQuestion},
    fields::{Field, FieldValue},
    file::{EditorFile, fill_required, write_file},
    form::{
        DEFAULTS_SECTIONS, EDITOR_SECTIONS, IgnoredBy, SETTINGS_SECTIONS, form_fields, host_fields,
        ignored_by, write_models_only,
    },
    hints::EditorWidget,
    host_settings::HostSettings,
    inherited::{BaseLayers, Inherited},
    kept_lines::kept_lines,
    media_list::is_missing,
    note_writer::NoteWriter,
    page::PageTexts,
    rows::RowTexts,
    save::{FileStamp, edited_document, read_document, validation_error},
};
#[cfg(all(test, feature = "pc98"))]
pub(crate) use self::{discard::DISCARD_LAYER_ID, media_list::empty_list_id};
pub(crate) use self::{
    file::SaveFailure, form::EditorSection, notes::ExtensionList, save::OpenError,
};
use crate::{
    catalog::copy_title,
    config::{ConfigDocument, HostLayers, metadata::TITLE_KEY},
    file_browser::{BrowserOutcome, FileBrowser},
    manager::shown_path,
    texts::{HintText, HintTexts, common, editor},
};

/// The keys that every game file of the editor holds. Their dropdowns have
/// no inherited entry, and a save writes their inherited value into a file
/// without them.
const GAME_REQUIRED_KEYS: &[&str] = &[MACHINE_KEY];

/// The error of an editor on a system that this build does not compile.
fn not_compiled(system: &System) -> OpenError {
    OpenError::NotCompiled(system.name)
}

/// The kind of the file of an editor, with the system and the name of a
/// game file or the system of a system defaults file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EditedFile {
    /// A game file of `system`.
    Game {
        /// The system of the game.
        system: &'static System,
        /// The name of the game: its title, else its file stem.
        name: String,
    },
    /// The system defaults file of the system.
    SystemDefaults(&'static System),
    /// The settings file.
    Settings,
}

impl EditedFile {
    /// Writes the name of the file in the header in `catalog`: the name of
    /// a game, the system defaults of a system such as `PC-98 system
    /// defaults`, or the name of the settings page.
    pub(crate) fn write_name(&self, catalog: &Catalog, out: &mut String) {
        let _ = match self {
            Self::Game { name, .. } => {
                out.push_str(name);
                Ok(())
            }
            Self::SystemDefaults(system) => {
                catalog.write(out, editor::defaults_title(&system.label))
            }
            Self::Settings => {
                out.push_str(catalog.text(common::SETTINGS));
                Ok(())
            }
        };
    }

    /// The name of the file in the header in English.
    #[cfg(test)]
    pub(crate) fn display_name(&self) -> String {
        crate::texts::english_text(|catalog, out| self.write_name(catalog, out))
    }
}

/// The state of the editor of a game file or of the system defaults file of
/// one system.
#[derive(Debug, Clone)]
struct SystemFiles {
    /// The system of the file.
    system: &'static System,
    /// The family of the system.
    target: Target,
    /// The layers below the file: the settings file, and the system defaults below a game.
    base: BaseLayers,
    /// The model that a new game starts with.
    preset_model: Option<&'static str>,
}

impl SystemFiles {
    /// The state of the editor of a game file of `system` in `host`, with
    /// the settings file and the system defaults below the file. A system
    /// that this build does not compile gives `None`.
    fn game(host: &HostLayers, system: &'static System) -> Option<Self> {
        let target = MachineSelection::of_system(system)?.target();
        Some(Self {
            system,
            target,
            base: BaseLayers::read(host, target),
            preset_model: None,
        })
    }

    /// The state of the editor of the system defaults file of `system` in
    /// `host`, with the settings file below the file. A system that this
    /// build does not compile gives `None`.
    fn system_defaults(host: &HostLayers, system: &'static System) -> Option<Self> {
        let target = MachineSelection::of_system(system)?.target();
        Some(Self {
            system,
            target,
            base: BaseLayers::read_below_system_defaults(host, target),
            preset_model: None,
        })
    }
}

/// The subject of the editor: the kind of the edited file with the state
/// that its form needs.
#[derive(Debug, Clone)]
enum Subject {
    /// A game file of one system.
    Game(SystemFiles),
    /// The system defaults file of one system.
    SystemDefaults(SystemFiles),
    /// The host keys of the settings file. The box keeps the variants of
    /// the subject close in size.
    Host(Box<HostSettings>),
}

impl Subject {
    /// The state of the editor of a game file or of a system defaults file.
    /// The settings page has none.
    fn system_files(&self) -> Option<&SystemFiles> {
        match self {
            Self::Game(files) | Self::SystemDefaults(files) => Some(files),
            Self::Host(_) => None,
        }
    }

    /// The sections of the form in tab order. A system defaults file has no
    /// Metadata section. The settings file has the sections of the settings
    /// page.
    fn sections(&self) -> &'static [EditorSection] {
        match self {
            Self::Game(_) => &EDITOR_SECTIONS,
            Self::SystemDefaults(_) => &DEFAULTS_SECTIONS,
            Self::Host(_) => &SETTINGS_SECTIONS,
        }
    }

    /// The keys that every save writes into the file.
    fn required_keys(&self) -> &'static [&'static str] {
        match self {
            Self::Game(_) => GAME_REQUIRED_KEYS,
            Self::SystemDefaults(_) | Self::Host(_) => &[],
        }
    }

    /// The fields of the form in tab order with the values of `document`:
    /// the keys of the family of the system in the sections of the
    /// subject, or the host keys of the settings page.
    fn fields(&self, document: &ConfigDocument) -> Vec<Field> {
        let files = match self {
            Self::Game(files) | Self::SystemDefaults(files) => files,
            Self::Host(settings) => {
                return host_fields()
                    .into_iter()
                    .map(|(section, source)| match source.setting() {
                        Some(setting) if setting.kind == ValueKind::Language => {
                            let choices = settings.language.choices(document.get(setting.key));
                            Field::with_choices(section, source, choices, document, false)
                        }
                        Some(_) | None => {
                            Field::read(section, source, Families::compiled(), document, false)
                        }
                    })
                    .collect();
            }
        };
        let sections = self.sections();
        let required = self.required_keys();
        form_fields(files.target)
            .into_iter()
            .filter(|(section, _)| sections.contains(section))
            .map(|(section, source)| {
                let required = required.contains(&source.key());
                Field::read(
                    section,
                    source,
                    Families::of(files.target),
                    document,
                    required,
                )
            })
            .collect()
    }
}

/// The response of the editor in a pass.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct EditorPass {
    /// The editor closes.
    pub(crate) closed: bool,
    /// The file at this path was saved.
    pub(crate) saved: Option<PathBuf>,
    /// The save failed.
    pub(crate) save_error: Option<SaveFailure>,
}

/// The open editor of a game file, a system defaults file or the settings file.
pub(crate) struct Editor {
    /// The kind of the edited file with the state of its form.
    subject: Subject,
    /// The root folder.
    root: PathBuf,
    /// The file that a save writes.
    file: EditorFile,
    /// The lines of the file when the editor opened or saved last.
    document: ConfigDocument,
    /// The modification time and the size of the file when the editor read
    /// or saved it last. A file that did not exist has none.
    stamp: Option<FileStamp>,
    /// The fields of the form in tab order.
    fields: Vec<Field>,
    /// The inherited value of each field. A metadata field has none.
    inherited: Vec<Option<Inherited>>,
    /// Whether each image of each field is missing, in the order of its values.
    missing: Vec<Vec<bool>>,
    /// The lines of the file that no field shows.
    kept: Vec<String>,
    /// The index of the selected section in the sections of the editor.
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
    hints: HintTexts,
    /// The count of the changes of the state that the texts of the page
    /// show. A text of the page refills when the count or the catalog
    /// generation changes.
    revision: u64,
    /// The texts of the page.
    texts: PageTexts,
    /// The texts of the row of each field, in the order of the fields.
    row_texts: Vec<RowTexts>,
    /// The path of the file to show, from the last refresh.
    shown: String,
    /// The model of the form of a game or of a system defaults file at the
    /// last refresh. The settings page has none.
    model: Option<MachineSelection>,
}

impl Editor {
    /// An editor of `subject` with the root `root` on the lines `document`
    /// of `file`, with the stamp of the read of the file. A file that did
    /// not exist has no stamp.
    fn with_document(
        subject: Subject,
        root: &Path,
        file: EditorFile,
        document: ConfigDocument,
        stamp: Option<FileStamp>,
    ) -> Self {
        let fields = subject.fields(&document);
        let kept = kept_lines(&document, &fields);
        let mut editor = Self {
            subject,
            root: root.to_path_buf(),
            file,
            document,
            stamp,
            inherited: vec![None; fields.len()],
            missing: vec![Vec::new(); fields.len()],
            row_texts: vec![RowTexts::default(); fields.len()],
            fields,
            kept,
            section: 0,
            scroll: ScrollState::new(),
            browser: None,
            discard: None,
            focus_first: true,
            widgets: Vec::new(),
            hints: HintTexts::new(),
            revision: 0,
            texts: PageTexts::default(),
            shown: String::new(),
            model: None,
        };
        editor.refresh();
        editor
    }

    /// An editor on the game file at `path` of `system`.
    pub(crate) fn open(
        host: &HostLayers,
        system: &'static System,
        path: &Path,
    ) -> Result<Self, OpenError> {
        let (document, stamp) = read_document(path)?;
        let files = SystemFiles::game(host, system).ok_or_else(|| not_compiled(system))?;
        Ok(Self::with_document(
            Subject::Game(files),
            host.root(),
            EditorFile::Existing(path.to_path_buf()),
            document,
            Some(stamp),
        ))
    }

    /// An editor on a new game of `system`. Its machine starts with the
    /// model that a game of the system takes without `machine`.
    pub(crate) fn new_game(host: &HostLayers, system: &'static System) -> Option<Self> {
        let mut files = SystemFiles::game(host, system)?;
        let model = files.base.model().0.name();
        files.preset_model = Some(model);
        let folder = host.config_folder().join(system.name);
        let mut editor = Self::with_document(
            Subject::Game(files),
            host.root(),
            EditorFile::New(DerivedFile::new(&folder, "")),
            ConfigDocument::new(),
            None,
        );
        if let Some(field) = editor.field_mut(MACHINE_KEY)
            && let Some(index) = field.choices.iter().position(|choice| *choice == model)
        {
            field.select(Some(index));
        }
        editor.refresh();
        Some(editor)
    }

    /// An editor on a copy of the game file at `source` of `system`. The
    /// copy keeps every line and gives the title the copy pattern of
    /// `catalog`, or the file stem of a game without a title.
    pub(crate) fn duplicate(
        host: &HostLayers,
        system: &'static System,
        source: &Path,
        catalog: &Catalog,
    ) -> Result<Self, OpenError> {
        let document =
            ConfigDocument::load(source).map_err(|error| OpenError::failed(source, error))?;
        let files = SystemFiles::game(host, system).ok_or_else(|| not_compiled(system))?;
        let stem = source
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let title = match document.get(TITLE_KEY).map(str::trim) {
            Some(title) if !title.is_empty() => title.to_owned(),
            Some(_) | None => stem,
        };
        let title = copy_title(catalog, &title);
        let folder = host.config_folder().join(system.name);
        let file = EditorFile::Copy {
            source: shown_path(source, host.root()),
            file: DerivedFile::new(&folder, &title),
        };
        let mut editor =
            Self::with_document(Subject::Game(files), host.root(), file, document, None);
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

    /// The state of the editor of a game file or of a system defaults file.
    /// The settings page has none.
    fn system_files(&self) -> Option<&SystemFiles> {
        self.subject.system_files()
    }

    /// The host keys that the settings page edits. The editor of a game or
    /// of a system defaults file has none.
    fn host_settings(&self) -> Option<&HostSettings> {
        match &self.subject {
            Subject::Game(_) | Subject::SystemDefaults(_) => None,
            Subject::Host(host) => Some(host),
        }
    }

    /// The files of the drive of `setting` on the family of the system. The
    /// settings page has none.
    fn file_types(&self, setting: &Setting) -> Option<&'static FileTypes> {
        self.system_files()
            .and_then(|files| setting.file_types_for(files.target))
    }

    /// The kind of the edited file with the name of a game: its title,
    /// else its file stem.
    pub(crate) fn edited_file(&self) -> EditedFile {
        match &self.subject {
            Subject::Game(files) => EditedFile::Game {
                system: files.system,
                name: self.game_name(),
            },
            Subject::SystemDefaults(files) => EditedFile::SystemDefaults(files.system),
            Subject::Host(_) => EditedFile::Settings,
        }
    }

    /// The sections of the editor in tab order.
    pub(crate) fn sections(&self) -> &'static [EditorSection] {
        self.subject.sections()
    }

    /// The path of the file that a save writes.
    pub(crate) fn path(&self) -> PathBuf {
        self.file.path()
    }

    /// The path of the file to show: below the root when it lies there. The
    /// settings page shows the whole path of the settings file.
    pub(crate) fn shown_path(&self) -> String {
        match &self.subject {
            Subject::Game(_) | Subject::SystemDefaults(_) => shown_path(&self.path(), &self.root),
            Subject::Host(_) => self.path().display().to_string(),
        }
    }

    /// The path of the file to show, as [`Editor::shown_path`] gives it
    /// at the last change of the editor.
    pub(crate) fn shown_path_text(&self) -> &str {
        &self.shown
    }

    /// The hints of the status line of the last pass.
    pub(crate) fn hints(&self) -> &[HintText] {
        &self.hints
    }

    /// The model of the form of a game or of a system defaults file with
    /// the state `files` at the last change of the editor: the chosen
    /// model, else the model of `files.base`.
    fn model_of(&self, files: &SystemFiles) -> MachineSelection {
        self.model.unwrap_or_else(|| self.current_model(files))
    }

    /// The model of the form of a game or of a system defaults file with
    /// the state `files` in the fields as they are: the chosen model, else
    /// the model of `files.base`.
    fn current_model(&self, files: &SystemFiles) -> MachineSelection {
        self.field(MACHINE_KEY)
            .and_then(Field::chosen)
            .and_then(|name| name.parse().ok())
            .unwrap_or_else(|| files.base.model().0)
    }

    /// The name of a game: its title, else its file stem.
    fn game_name(&self) -> String {
        let title = self.field(TITLE_KEY).map_or("", Field::text).trim();
        if !title.is_empty() {
            return title.to_owned();
        }
        self.path()
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// Whether a save would change the file: a field changed, or the editor
    /// holds a new game or a copy that has no file yet.
    fn is_dirty(&self) -> bool {
        self.file.derived().is_some() || self.fields.iter().any(Field::changed)
    }

    /// The count of the changes of the state that the texts of the page
    /// show.
    #[cfg(test)]
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    /// Whether a field of `section` holds a set value.
    #[cfg(all(test, feature = "pc98"))]
    pub(crate) fn section_has_set_field(&self, section: EditorSection) -> bool {
        self.fields
            .iter()
            .any(|field| field.section == section && field.is_set())
    }

    /// Whether the file browser is open.
    #[cfg(all(test, feature = "pc98"))]
    pub(crate) fn browser_open(&self) -> bool {
        self.browser.is_some()
    }

    /// Computes the inherited values, the missing files, the file of a game
    /// without a save and the folders of the settings page again after a
    /// change of a field. The settings page checks only a set GUI font. Its
    /// value resolves against the root alone.
    fn refresh(&mut self) {
        self.revision += 1;
        self.model = self.system_files().map(|files| self.current_model(files));
        let host = self.host_settings().is_some();
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
            self.inherited[index] = match &self.subject {
                Subject::Game(files) | Subject::SystemDefaults(files) => {
                    Some(files.base.inherited(setting, self.model_of(files), &game))
                }
                Subject::Host(settings) => Some(settings.inherited(setting, &self.fields)),
            };
            self.missing[index] = match setting.kind {
                ValueKind::Path(PathKind::File) | ValueKind::PathList
                    if !host || setting.key == GUI_FONT_KEY =>
                {
                    field
                        .values()
                        .iter()
                        .map(|value| is_missing(value, &self.root))
                        .collect()
                }
                ValueKind::Path(PathKind::File)
                | ValueKind::PathList
                | ValueKind::Path(PathKind::Folder)
                | ValueKind::Choice(_)
                | ValueKind::OnOff
                | ValueKind::Number(_)
                | ValueKind::Language => Vec::new(),
            };
        }
        let title = self.field(TITLE_KEY).map(|field| field.text().to_owned());
        match &mut self.file {
            EditorFile::Existing(_) => {}
            EditorFile::New(file) | EditorFile::Copy { file, .. } => {
                *file = DerivedFile::new(&file.folder, title.as_deref().unwrap_or_default());
            }
        }
        self.shown = self.shown_path();
        self.update_host_folders();
    }

    /// The selected section.
    pub(crate) fn current_section(&self) -> EditorSection {
        let sections = self.sections();
        sections[self.section.min(sections.len() - 1)]
    }

    /// Writes the changed fields into the file and returns its path.
    ///
    /// A required field without a value writes its inherited value. A
    /// file that changed since the editor read it stays as it is, unless
    /// `overwrite` asks to write over the change. A new game or a copy
    /// takes its derived file name once more before the write and keeps it
    /// after. Its write never replaces a file. A failed save leaves the
    /// fields as they are.
    fn save(&mut self, overwrite: bool) -> Result<PathBuf, SaveFailure> {
        self.refresh();
        let mut fields = self.fields.clone();
        fill_required(&mut fields, &self.inherited);
        let edited = edited_document(&self.document, &fields);
        let title = self.field(TITLE_KEY).map_or("", Field::text).to_owned();
        let written = write_file(
            &self.file, &edited, self.stamp, &title, &self.root, overwrite,
        )?;
        let path = written.path;
        self.stamp = written.stamp;
        self.document = edited;
        self.fields = fields;
        for field in &mut self.fields {
            field.mark_saved();
        }
        if self.file.derived().is_some() {
            self.file = EditorFile::Existing(path.clone());
        }
        if let Subject::Game(files) = &mut self.subject {
            files.preset_model = None;
        }
        self.kept = kept_lines(&self.document, &self.fields);
        self.refresh();
        Ok(path)
    }

    /// Saves the changes over a file that changed since the editor read
    /// it, and takes the stamp of the written file.
    pub(crate) fn overwrite(&mut self) -> Result<PathBuf, SaveFailure> {
        self.save(true)
    }

    /// Asks to close the editor: with unsaved changes the question opens,
    /// else the editor closes. Returns whether it closes.
    fn request_close(&mut self) -> bool {
        if self.is_dirty() {
            self.discard = Some(DiscardQuestion::new(self.edited_file()));
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
        if self.check_changed_roms(context) {
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

    /// Returns `None` when the editor enables `setting`, else the model and
    /// the limit under which the model ignores it. The editor of a system
    /// defaults file and the settings page enable every setting.
    pub(super) fn ignored_by(&self, setting: &Setting) -> Option<IgnoredBy> {
        match &self.subject {
            Subject::Game(files) => ignored_by(setting, self.model_of(files)),
            Subject::SystemDefaults(_) | Subject::Host(_) => None,
        }
    }

    /// Adds the sentences after the note of `setting`: the models that read
    /// it in the editor of a system defaults file, and the command line
    /// value and the restart of the settings page. The editor of a game adds
    /// none.
    pub(super) fn write_extra(&self, writer: &mut NoteWriter, setting: &Setting) {
        match &self.subject {
            Subject::Game(_) => {}
            Subject::SystemDefaults(files) => {
                if let Some(limit) = setting.limit_for(files.target) {
                    write_models_only(writer, limit);
                }
            }
            Subject::Host(host) => host.write_extra(writer, setting.key),
        }
    }

    /// The hints of the status line after a pass.
    fn status_hints(&self, context: &UiContext) -> HintTexts {
        if let Some((browser, _)) = &self.browser {
            return browser.hints().clone();
        }
        if let Some(question) = &self.discard {
            return question.hints().iter().copied().collect();
        }
        hints::editor_hints(context, &self.widgets)
    }
}

#[cfg(test)]
mod tests {
    use gui::ui::{Navigation, Shortcut};
    use machine_factory::config::settings::{find_setting, value_error::ValueError};

    use super::{
        media_list::MediaBrowse,
        rows::field_id,
        test_support::{EditorRoot, SCREEN, fdd1_model, index_of, show},
        *,
    };
    use crate::{config::check_value, gui_test_support::GuiHarness};

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
            crate::texts::english_labels(editor.hints()),
            ["Section", "Save", "Close"]
        );
    }

    /// Verifies that a typed title of a new game refills the cached header line of the next frame with the new file name.
    #[test]
    fn a_typed_title_refills_the_header_line_of_a_new_game() {
        let root = EditorRoot::new("editor_new_title");
        let system = root.system().name;
        let mut editor = Editor::new_game(&root.host(), root.system()).expect("the editor opens");
        let mut harness = GuiHarness::new(960, 600);
        show(&mut harness, &mut editor);
        assert_eq!(harness.state.focused(), Some(field_id(TITLE_KEY)));
        assert_eq!(
            editor.cached_header_line(),
            Some(format!("Saved as config/{system}/game.conf").as_str())
        );
        harness.input.text = "Edge".to_owned();
        show(&mut harness, &mut editor);
        assert_eq!(
            editor.cached_header_line(),
            Some(format!("Saved as config/{system}/edge.conf").as_str())
        );
    }

    #[cfg(feature = "pc98")]
    /// Verifies that a model that the dropdown chooses refills the cached note of a setting that the model limits in the next frame.
    #[test]
    fn a_chosen_model_refills_the_note_of_a_limited_setting() {
        let root = EditorRoot::with_system("editor_model_note", "pc98");
        let mut editor = open(&root, "edge", "meta.title = Edge\nmachine = PC9801VM\n");
        let mut harness = GuiHarness::new(960, 600);
        show(&mut harness, &mut editor);
        select_section(&mut harness, &mut editor, EditorSection::Machine);
        assert_eq!(
            editor.cached_note("ems"),
            Some("PC9801VM has no EMS memory. VX, RS, RA, AS and AP only.")
        );
        harness.state.set_focus(field_id(MACHINE_KEY));
        show(&mut harness, &mut editor);
        harness.input.confirm = true;
        show(&mut harness, &mut editor);
        harness.input.navigation = Some(Navigation::End);
        show(&mut harness, &mut editor);
        harness.input.confirm = true;
        show(&mut harness, &mut editor);
        let model = editor
            .field(MACHINE_KEY)
            .and_then(Field::chosen)
            .expect("a chosen model")
            .to_owned();
        assert_ne!(model, "PC9801VM");
        let mut fresh = open(
            &root,
            "fresh",
            &format!("meta.title = Edge\nmachine = {model}\n"),
        );
        let mut fresh_harness = GuiHarness::new(960, 600);
        show(&mut fresh_harness, &mut fresh);
        select_section(&mut fresh_harness, &mut fresh, EditorSection::Machine);
        let note = editor.cached_note("ems").expect("the row draws its note");
        assert!(!note.contains("has no EMS memory"), "{note}");
        assert_eq!(Some(note), fresh.cached_note("ems"));
    }

    #[cfg(feature = "pc98")]
    /// Verifies that a switch of the catalog gives the drawn note of a limited setting the new language in the next frame and that a switch back gives the English note.
    #[test]
    fn a_catalog_switch_refills_the_drawn_note() {
        /// The German join pattern and model sentence of the notes.
        const GERMAN_NOTE: &str = "[note]\n\
                                   join = {first} | {second}\nmodels-only = Nur {models}.\n";
        /// The English note of the EMS setting on the PC9801VM.
        const ENGLISH_NOTE: &str = "PC9801VM has no EMS memory. VX, RS, RA, AS and AP only.";
        let root = EditorRoot::with_system("editor_note_switch", "pc98");
        let mut editor = open(&root, "edge", "meta.title = Edge\nmachine = PC9801VM\n");
        let mut harness = GuiHarness::new(960, 600);
        show(&mut harness, &mut editor);
        select_section(&mut harness, &mut editor, EditorSection::Machine);
        assert_eq!(editor.cached_note("ems"), Some(ENGLISH_NOTE));
        harness.set_catalog(crate::texts::LanguageCatalog::Loaded(Box::new(
            crate::texts::test_catalog(GERMAN_NOTE),
        )));
        show(&mut harness, &mut editor);
        assert_eq!(
            editor.cached_note("ems"),
            Some("PC9801VM has no EMS memory. | Nur VX, RS, RA, AS and AP.")
        );
        harness.set_catalog(crate::texts::LanguageCatalog::English);
        show(&mut harness, &mut editor);
        assert_eq!(editor.cached_note("ems"), Some(ENGLISH_NOTE));
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
            crate::texts::english_labels(editor.hints()),
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
        let model = fdd1_model(&root);
        let mut editor = open(
            &root,
            "edge",
            &format!("machine = {model}\nfdd1 = games/missing.d88\n"),
        );
        let fdd1 = index_of(&editor, "fdd1");
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
            crate::texts::english_labels(editor.hints()),
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
        /// The model of the game form of `editor`.
        fn game_model(editor: &Editor) -> MachineSelection {
            let Subject::Game(files) = &editor.subject else {
                panic!("the editor edits a game");
            };
            editor.model_of(files)
        }

        let root = EditorRoot::with_system("editor_model", "pc98");
        let mut editor = open(&root, "edge", "machine = PC9801RA\n");
        let cpu_mode = find_setting("cpu-mode").expect("table key");
        let model = game_model(&editor);
        assert!(form::ignored_by(cpu_mode, model).is_some());
        let machine = editor.field_mut(MACHINE_KEY).expect("machine field");
        let index = machine
            .choices
            .iter()
            .position(|choice| *choice == "PC9801VM")
            .expect("a PC-98 model");
        machine.select(Some(index));
        editor.refresh();
        let model = game_model(&editor);
        assert_eq!(model.name(), "PC9801VM");
        assert!(form::ignored_by(cpu_mode, model).is_none());
        let bios = index_of(&editor, "bios");
        assert_eq!(
            editor.inherited[bios]
                .as_ref()
                .map(|inherited| inherited.values.join(", "))
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
        let first = machine.choices[0].as_ref();
        let files = editor.system_files().expect("a game has a system");
        if first == files.base.model().0.name() {
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
            failure.to_string().contains("changed outside the manager"),
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
        assert!(matches!(failure, SaveFailure::Error { .. }), "{failure:?}");
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
            crate::texts::english_labels(editor.hints()),
            ["Clear", "Section", "Save", "Close"]
        );
        harness.input.action = Some(gui::ui::ActionButton::X);
        show(&mut harness, &mut editor);
        let developer = editor.field("meta.developer").expect("developer field");
        assert!(!developer.is_set());
        assert!(editor.is_dirty());
        assert_eq!(
            crate::texts::english_labels(editor.hints()),
            ["Section", "Save", "Close"]
        );
    }

    /// Verifies that an empty value of a drive is an empty list over the inherited list, and that its checkbox returns the drive to the inherited list.
    #[test]
    fn an_empty_drive_overrides_the_inherited_list() {
        let root = EditorRoot::new("editor_empty_list");
        root.write("neetan.conf", "fdd1 = games/boot.d88\n");
        let model = fdd1_model(&root);
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

    /// Verifies that the game editor shows the image of the system defaults as the inherited list with its source, below a list of the game as well, and that the checkbox writes the empty value that clears the inherited list.
    #[test]
    fn the_game_editor_inherits_the_media_of_the_system_defaults() {
        let root = EditorRoot::new("editor_system_media");
        root.write("neetan.conf", "fdd1 = games/settings.d88\n");
        root.write(
            &format!("config/{}.conf", root.system().name),
            "fdd1 = games/system.d88\n",
        );
        let model = fdd1_model(&root);
        let mut editor = open(&root, "edge", &format!("machine = {model}\n"));
        let fdd1 = index_of(&editor, "fdd1");
        let inherited = editor.inherited[fdd1].clone().expect("fdd1 inherits");
        assert_eq!(inherited.values, ["games/system.d88"]);
        assert_eq!(inherited.source, inherited::InheritedSource::SystemDefaults);
        let types = find_setting("fdd1")
            .expect("fdd1 is a table key")
            .file_types_for(root.target());
        assert_eq!(
            crate::texts::english_text(|catalog, out| {
                notes::write_empty_drive_hint(catalog, out, types, Some(&inherited), false);
            }),
            "System default: games/system.d88"
        );
        assert_eq!(
            notes::tests::english_note(|writer| {
                media_row::below_note(writer, true, &[], true, Some(&inherited))
            }),
            notes::tests::Note::plain("System default: games/system.d88.").shown_as_path()
        );

        let mut harness = GuiHarness::new(960, 600);
        show(&mut harness, &mut editor);
        select_section(&mut harness, &mut editor, EditorSection::Media);
        let checkbox = media_list::empty_list_id(field_id("fdd1"));
        harness.state.set_focus(checkbox);
        show(&mut harness, &mut editor);
        harness.input.confirm = true;
        show(&mut harness, &mut editor);
        assert!(editor.fields[fdd1].is_empty_list());
        harness.input.shortcut = Some(Shortcut::ControlS);
        show(&mut harness, &mut editor);
        assert_eq!(
            std::fs::read_to_string(editor.path()).expect("file reads"),
            format!("machine = {model}\nfdd1 =\n")
        );
    }

    #[cfg(feature = "pc98")]
    /// Verifies that an empty value of a single image over the image of the system defaults shows the empty drive and saves unchanged, and that the checkbox removes and writes the empty value.
    #[test]
    fn an_empty_image_overrides_the_image_of_the_system_defaults() {
        let root = EditorRoot::with_system("editor_empty_image", "pc98");
        let hdd1 = find_setting("hdd1").expect("hdd1 is a table key");
        let model = "PC9801RA";
        assert!(hdd1.is_read_by(root.target(), model));
        root.write(
            &format!("config/{}.conf", root.system().name),
            "hdd1 = games/system.hdi\n",
        );
        let path = root.game("edge", &format!("machine = {model}\nhdd1 =\n"));
        let mut editor =
            Editor::open(&root.host(), root.system(), &path).expect("the editor opens");
        let index = index_of(&editor, "hdd1");
        assert!(editor.fields[index].is_empty_path());
        assert!(editor.fields[index].is_set());
        let inherited = editor.inherited[index].clone().expect("hdd1 inherits");
        assert_eq!(inherited.values, ["games/system.hdi"]);
        assert_eq!(inherited.source, inherited::InheritedSource::SystemDefaults);
        assert_eq!(
            crate::texts::english_text(|catalog, out| {
                editor.write_path_placeholder(catalog, out, index);
            }),
            "Empty"
        );
        assert!(!editor.is_dirty());
        assert_eq!(editor.save(false), Ok(path.clone()));
        assert_eq!(
            std::fs::read_to_string(&path).expect("file reads"),
            format!("machine = {model}\nhdd1 =\n")
        );

        let mut harness = GuiHarness::new(960, 600);
        show(&mut harness, &mut editor);
        select_section(&mut harness, &mut editor, EditorSection::Media);
        let checkbox = media_list::empty_list_id(field_id("hdd1"));
        harness.state.set_focus(checkbox);
        show(&mut harness, &mut editor);
        assert_eq!(harness.state.focused(), Some(checkbox));
        harness.input.confirm = true;
        show(&mut harness, &mut editor);
        assert!(!editor.fields[index].is_set());
        assert_eq!(harness.state.focused(), Some(checkbox));
        harness.input.shortcut = Some(Shortcut::ControlS);
        show(&mut harness, &mut editor);
        assert_eq!(
            std::fs::read_to_string(&path).expect("file reads"),
            format!("machine = {model}\n")
        );
        harness.input.confirm = true;
        show(&mut harness, &mut editor);
        assert!(editor.fields[index].is_empty_path());
        harness.input.shortcut = Some(Shortcut::ControlS);
        show(&mut harness, &mut editor);
        assert_eq!(
            std::fs::read_to_string(&path).expect("file reads"),
            format!("machine = {model}\nhdd1 =\n")
        );
        harness.input.action = Some(gui::ui::ActionButton::X);
        harness.state.set_focus(field_id("hdd1"));
        show(&mut harness, &mut editor);
        assert!(!editor.fields[index].is_set());
    }

    /// Verifies that a value of the file that the launch rejects stays in the file and is invalid in the editor.
    #[test]
    fn a_value_with_another_case_is_invalid() {
        let root = EditorRoot::new("editor_invalid");
        let editor = open(&root, "edge", "crt = On\n");
        let crt = editor.field("crt").expect("crt field");
        let invalid = crt.invalid_value().expect("the launch rejects On");
        let error = ValueError::OnOff {
            value: "On".to_owned(),
        };
        assert_eq!(
            invalid.reason,
            fields::InvalidReason::Rejected(error.clone())
        );
        assert_eq!(
            check_value(find_setting("crt").expect("crt"), "On").err(),
            Some(error)
        );
    }
}
