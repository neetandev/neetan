//! The settings page: the editor on the host keys of the settings file. It
//! shows the root, the folder of each folder key, the ROM rows and the
//! values of the command line flags of the host keys.

use std::path::{Path, PathBuf};

use gui::{
    Rect,
    ui::{Align, UiContext, wrapped_height, wrapped_label},
};
use machine_factory::config::{
    settings::{
        BACKEND_KEY, CONFIG_FOLDER_KEY, GUI_FONT_KEY, GUI_THEME_KEY, ROMS_FOLDER_KEY, Scope,
        Setting,
    },
    systems::RomFolder,
};

use super::{
    Editor, EditorFile, Subject,
    fields::Field,
    form::host_fields,
    inherited::{Inherited, InheritedSource},
    kept_lines::draw_rule,
    notes::{Note, sentences},
    rom_rows::{RomRow, rom_folder_of},
    rows::{NOTE_MAXIMUM_LINES, ReadOnlyRow},
    save::read_optional_document,
};
use crate::{
    config::{
        HostLayers, SettingsLocation, SettingsSource,
        layers::{folder_location, host_default},
        reading::LayerLine,
    },
    rom_folders::{RomFolderStatus, rom_folder_location},
};

/// The keys whose change applies after a restart. The window creates its
/// graphics engine and loads its font and its theme once.
const RESTART_KEYS: [&str; 3] = [BACKEND_KEY, GUI_FONT_KEY, GUI_THEME_KEY];

/// The sentence of a key whose change applies after a restart.
const RESTART_NOTE: &str = "The change applies after a restart.";

/// The warning of a set GUI font whose file does not exist.
pub(crate) const MISSING_FONT_NOTE: &str =
    "The font file does not exist. The manager does not start without it.";

/// The label of the read-only row of the root.
const ROOT_LABEL: &str = "Root";

/// The caption of the rule above the discovery order.
const SETTINGS_FILE_CAPTION: &str = "Settings file";

/// The discovery order of the settings file below its rule.
const SETTINGS_FILE_TEXT: &str = "Neetan reads the first neetan.conf of: --settings <file>, the \
     current directory, the folder of the executable, the SDL pref path. Relative paths in \
     configuration files resolve against the root.";

/// The note of the root after the step of the discovery `source`.
pub(crate) fn root_note(source: SettingsSource) -> &'static str {
    match source {
        SettingsSource::CommandLine => "The root is the folder of the --settings file.",
        SettingsSource::CurrentDirectory => {
            "The root is the folder of neetan.conf in the current directory."
        }
        SettingsSource::ExecutableFolder => {
            "The root is the folder of neetan.conf beside the executable."
        }
        SettingsSource::PrefPath => {
            "The settings file lies in the SDL pref path. The root is the current directory."
        }
        SettingsSource::NotFound => {
            "No settings file exists. The root is the current directory, and Save creates neetan.conf there."
        }
    }
}

/// The sentence of a key that the command line sets to `value`.
pub(crate) fn flag_sentence(value: &str) -> String {
    format!("The command line sets {value} for this run.")
}

/// The sentence of an unset ROM key whose ROM folder the command line
/// sets to `folder`.
fn rom_folder_flag_sentence(folder: &str) -> String {
    format!("The command line sets the ROM folder {folder} for this run.")
}

/// The notes of a path field on the settings page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HostPathNote {
    /// The folder of the field or its ROM status.
    pub(super) note: Note,
    /// The value that a command line flag sets for this run, on its own
    /// line below the note.
    pub(super) flag: Option<Note>,
    /// Whether the field takes the error edge.
    pub(super) error: bool,
}

/// Returns the trimmed text of the field of `key` among `fields`. An unset
/// field gives `None`.
fn field_text<'a>(fields: &'a [Field], key: &str) -> Option<&'a str> {
    fields
        .iter()
        .find(|field| field.key() == key)
        .map(|field| field.text().trim())
        .filter(|text| !text.is_empty())
}

/// The state of the settings page beside its fields.
#[derive(Debug, Clone)]
pub(crate) struct HostSettings {
    /// The settings file, the root and the step of the discovery that found the file.
    location: SettingsLocation,
    /// The values of the command line flags of the host keys in command line order.
    flags: Vec<(&'static str, String)>,
    /// The ROM keys of the page with the last check of their folders.
    pub(super) roms: Vec<RomRow>,
}

impl HostSettings {
    /// The state of the settings page of `host`, with the ROM statuses
    /// `statuses` of the last ROM check.
    pub(super) fn new(host: &HostLayers, statuses: &[RomFolderStatus]) -> Self {
        let flags = host
            .command_line_layer()
            .lines
            .iter()
            .filter_map(|line| match line {
                LayerLine::Setting(setting, value) if setting.scope == Scope::Host => {
                    Some((setting.key, value.clone()))
                }
                LayerLine::Setting(..) | LayerLine::KeyBinding(..) => None,
            })
            .collect();
        let roms = host_fields()
            .into_iter()
            .filter_map(|(_, source)| source.setting())
            .filter_map(|setting| rom_folder_of(setting.key))
            .map(|folder| RomRow {
                folder,
                checked: None,
            })
            .collect();
        let mut settings = Self {
            location: host.location().clone(),
            flags,
            roms,
        };
        settings.take_statuses(statuses);
        settings
    }

    /// Takes the results of a ROM check from `statuses` for the ROM keys
    /// that they hold.
    fn take_statuses(&mut self, statuses: &[RomFolderStatus]) {
        for row in &mut self.roms {
            if let Some(status) = statuses
                .iter()
                .find(|status| status.folder.key == row.folder.key)
            {
                row.checked = Some((status.location.path.clone(), status.check.clone()));
            }
        }
    }

    /// The value that the command line sets for `key` in this run.
    pub(crate) fn flag(&self, key: &str) -> Option<&str> {
        self.flags
            .iter()
            .rev()
            .find(|(flag_key, _)| *flag_key == key)
            .map(|(_, value)| value.as_str())
    }

    /// The note above the ROM rows when the command line sets the ROM
    /// folder for this run.
    pub(super) fn roms_folder_flag_note(&self) -> Option<Note> {
        self.flag(ROMS_FOLDER_KEY)
            .map(|folder| Note::plain(rom_folder_flag_sentence(folder)))
    }

    /// The ROM row of the ROM key `key`.
    pub(crate) fn rom_row(&self, key: &str) -> Option<&RomRow> {
        self.roms.iter().find(|row| row.folder.key == key)
    }

    /// The value of `key` for the next launch, as written: the command line
    /// value, else the value of its field in `fields`.
    fn launch_value<'a>(&'a self, key: &str, fields: &'a [Field]) -> Option<&'a str> {
        self.flag(key).or_else(|| field_text(fields, key))
    }

    /// The absolute folder that the next launch reads for `folder` with the
    /// values of `fields`: the value of its key, else the folder below the
    /// ROM folder.
    pub(crate) fn rom_location(&self, folder: &RomFolder, fields: &[Field]) -> PathBuf {
        rom_folder_location(
            &self.location.root,
            self.launch_value(ROMS_FOLDER_KEY, fields),
            self.launch_value(folder.key, fields),
            folder,
        )
        .path
    }

    /// The absolute folder of the folder key `key` with the values of
    /// `fields`: its value, else its built-in value, below the root.
    fn folder_location(&self, key: &str, fields: &[Field]) -> PathBuf {
        let default = host_default(key).unwrap_or_default();
        folder_location(&self.location.root, field_text(fields, key), &default)
    }

    /// The value that `setting` inherits on the settings page with the
    /// values of `fields`: its built-in value. A ROM key inherits its folder
    /// below the ROM folder of its field, as the file writes it.
    pub(crate) fn inherited(&self, setting: &Setting, fields: &[Field]) -> Inherited {
        let values = match rom_folder_of(setting.key) {
            Some(folder) => {
                let roms = field_text(fields, ROMS_FOLDER_KEY)
                    .map(str::to_owned)
                    .or_else(|| host_default(ROMS_FOLDER_KEY))
                    .unwrap_or_default();
                vec![
                    Path::new(&roms)
                        .join(folder.name)
                        .to_string_lossy()
                        .into_owned(),
                ]
            }
            None => host_default(setting.key).into_iter().collect(),
        };
        Inherited {
            values,
            source: InheritedSource::BuiltIn,
        }
    }

    /// The sentences after the note of `setting`: the value of its command
    /// line flag and the restart that its change needs.
    pub(crate) fn extra_note(&self, setting: &Setting) -> Option<String> {
        let flag = self.flag(setting.key).map(flag_sentence);
        let restart = RESTART_KEYS
            .contains(&setting.key)
            .then(|| RESTART_NOTE.to_owned());
        let text = sentences(&[flag, restart]);
        (!text.is_empty()).then_some(text)
    }

    /// The warning of `setting` while its file does not exist, followed by
    /// the sentences of [`Self::extra_note`].
    pub(super) fn missing_file_note(&self, setting: &Setting) -> Note {
        Note::warning(sentences(&[
            Some(MISSING_FONT_NOTE.to_owned()),
            self.extra_note(setting),
        ]))
    }
}

/// Draws the rule of the settings file and the discovery order below it
/// from `y` in `content`. Returns the height of the block.
pub(super) fn draw_settings_file_block(context: &mut UiContext, content: Rect, y: f32) -> f32 {
    let theme = context.theme;
    let top = draw_rule(context, content, y, SETTINGS_FILE_CAPTION);
    let style = theme.small();
    let height = wrapped_height(
        context,
        &style,
        SETTINGS_FILE_TEXT,
        content.width,
        NOTE_MAXIMUM_LINES,
    );
    wrapped_label(
        context,
        Rect::new(content.x, top, content.width, height),
        &style,
        SETTINGS_FILE_TEXT,
        Align::Start,
        NOTE_MAXIMUM_LINES,
    );
    top + height + theme.gap - y
}

impl Editor {
    /// The settings page on the host keys of the settings file of `host`,
    /// with the ROM statuses `statuses` of the last ROM check. Without a
    /// settings file the page edits `neetan.conf` in the root, and the
    /// first save creates it. No key is required.
    pub(crate) fn settings(
        host: &HostLayers,
        statuses: &[RomFolderStatus],
    ) -> Result<Self, String> {
        let path = host.location().writable_file();
        let (document, stamp) = read_optional_document(&path)?;
        Ok(Self::with_document(
            Subject::Host(HostSettings::new(host, statuses)),
            host.root(),
            EditorFile::Existing(path),
            document,
            stamp,
        ))
    }

    /// Takes the settings file and the root of `host` after a save, and the
    /// results of the ROM check `statuses` when the manager checked the
    /// ROM folders again.
    pub(crate) fn host_changed(&mut self, host: &HostLayers, statuses: Option<&[RomFolderStatus]>) {
        if let Subject::Host(settings) = &mut self.subject {
            settings.location = host.location().clone();
            if let Some(statuses) = statuses {
                settings.take_statuses(statuses);
            }
        }
    }

    /// The read-only row of the root of the settings page.
    pub(super) fn root_row(&self) -> Option<ReadOnlyRow> {
        let settings = self.host_settings()?;
        Some(ReadOnlyRow {
            name: ROOT_LABEL,
            text: self.root.display().to_string(),
            path: true,
            note: root_note(settings.location.source).to_owned(),
        })
    }

    /// The notes of the path field at `index` of `setting` on the settings
    /// page: the ROM status of a ROM key, or the absolute folder of the
    /// config folder and the ROM folder. Another path field and the editor
    /// of a game give `None`.
    pub(super) fn host_path_note(&self, index: usize, setting: &Setting) -> Option<HostPathNote> {
        let settings = self.host_settings()?;
        let key_flag = settings.flag(setting.key);
        if let Some(row) = settings.rom_row(setting.key) {
            let (note, error) = self.rom_note(settings, row, index);
            let flag = key_flag.map(flag_sentence);
            return Some(HostPathNote {
                note,
                flag: flag.map(Note::plain),
                error,
            });
        }
        if setting.key != CONFIG_FOLDER_KEY && setting.key != ROMS_FOLDER_KEY {
            return None;
        }
        let folder = settings.folder_location(setting.key, &self.fields);
        let text = folder.display().to_string();
        let note = if folder.is_dir() {
            Note::found(text)
        } else {
            Note::unmarked(text)
        };
        Some(HostPathNote {
            note: note.shown_as_path(),
            flag: key_flag.map(|value| Note::plain(flag_sentence(value))),
            error: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use gui::ui::{Navigation, Shortcut};
    use machine_factory::{
        config::{
            DEFAULT_CONFIG_FOLDER,
            settings::{AUDIO_VOLUME_KEY, find_setting},
            systems::rom_folders,
        },
        rom_status::{RomStatus, check_rom_folder},
    };

    use super::*;
    use crate::{
        editor::{
            EditorSection,
            form::SETTINGS_SECTIONS,
            rom_rows::rom_status_note,
            rows::field_id,
            test_support::{
                SettingsRoot, first_rom_folder, index_of, settings_page, show, statuses_with,
            },
        },
        gui_test_support::GuiHarness,
        rom_folders::{RomFolderPath, check_all_folders},
    };

    /// The placeholder of the field of `key`.
    fn placeholder(editor: &Editor, key: &str) -> Option<String> {
        editor.inherited[index_of(editor, key)]
            .as_ref()
            .and_then(Inherited::label)
    }

    /// Verifies that the page opens on the Folders section with the root, edits the settings file and shows its whole path, and gives the host keys their built-in values.
    #[test]
    fn the_page_edits_the_host_keys_of_the_settings_file() {
        let test = SettingsRoot::new("open", Some("; my settings\n"));
        let host = test.host(&[]);
        let mut editor = settings_page(&host, &check_all_folders(&host));
        assert_eq!(editor.edited_file(), super::super::EditedFile::Settings);
        assert_eq!(editor.sections(), SETTINGS_SECTIONS);
        assert_eq!(editor.path(), test.settings_file());
        assert_eq!(
            editor.shown_path(),
            test.settings_file().display().to_string()
        );
        assert_eq!(editor.edited_file().display_name(), "Settings");
        let keys: Vec<&str> = editor.fields.iter().map(Field::key).collect();
        assert_eq!(
            keys,
            host_fields()
                .iter()
                .map(|(_, source)| source.key())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            placeholder(&editor, CONFIG_FOLDER_KEY).as_deref(),
            Some("Built-in default: config")
        );
        assert_eq!(
            placeholder(&editor, BACKEND_KEY).as_deref(),
            Some("Built-in default: modern")
        );
        assert_eq!(
            placeholder(&editor, AUDIO_VOLUME_KEY).as_deref(),
            Some("Built-in default: 1.0")
        );
        let row = editor.root_row().expect("the page shows the root");
        assert_eq!(row.text, test.root().display().to_string());
        assert_eq!(row.note, root_note(SettingsSource::CurrentDirectory));

        let mut harness = GuiHarness::new(960, 600);
        show(&mut harness, &mut editor);
        assert_eq!(editor.current_section(), EditorSection::Folders);
        assert_eq!(harness.state.focused(), Some(field_id(CONFIG_FOLDER_KEY)));
        let notes = editor
            .host_path_note(
                index_of(&editor, CONFIG_FOLDER_KEY),
                find_setting(CONFIG_FOLDER_KEY).expect("a key"),
            )
            .expect("the config folder shows its folder");
        assert_eq!(
            notes,
            HostPathNote {
                note: Note::unmarked(test.root().join("config").display().to_string())
                    .shown_as_path(),
                flag: None,
                error: false,
            }
        );
        std::fs::create_dir(test.root().join("roms")).expect("folder is created");
        let notes = editor
            .host_path_note(
                index_of(&editor, ROMS_FOLDER_KEY),
                find_setting(ROMS_FOLDER_KEY).expect("a key"),
            )
            .expect("the ROM folder shows its folder");
        assert_eq!(
            notes.note,
            Note::found(test.root().join("roms").display().to_string()).shown_as_path()
        );
    }

    /// Verifies that an unset ROM key shows its folder below the ROM folder as its placeholder, and that a new ROM folder updates the placeholders.
    #[test]
    fn unset_rom_keys_inherit_their_folder_below_the_rom_folder() {
        let test = SettingsRoot::new("placeholders", Some(""));
        let host = test.host(&[]);
        let mut editor = settings_page(&host, &statuses_with(&host, RomStatus::Found));
        let folder = first_rom_folder();
        let derived = |roms: &str| {
            format!(
                "Built-in default: {}",
                Path::new(roms).join(folder.name).display()
            )
        };
        assert_eq!(placeholder(&editor, folder.key), Some(derived("roms")));
        let roms = index_of(&editor, ROMS_FOLDER_KEY);
        editor.fields[roms].value =
            super::super::fields::FieldValue::Text("/archive/firmware".to_owned());
        editor.refresh();
        assert_eq!(
            placeholder(&editor, folder.key),
            Some(derived("/archive/firmware"))
        );
        for row in rom_folders().filter(|row| {
            editor
                .host_settings()
                .expect("a page")
                .rom_row(row.key)
                .is_some()
        }) {
            assert_eq!(
                placeholder(&editor, row.key),
                Some(format!(
                    "Built-in default: {}",
                    Path::new("/archive/firmware").join(row.name).display()
                ))
            );
        }
    }

    /// Verifies that a save keeps the comments, the unknown keys, the key bindings and the other lines of the settings file and writes only the changed keys.
    #[test]
    fn a_save_writes_only_the_changed_keys() {
        let text = "; Neetan settings\nconfig-folder = games\nkey.F1 = Stop\nmystery = 1\n\n\
                    crt=on\n# end\n";
        let test = SettingsRoot::new("save", Some(text));
        let host = test.host(&[]);
        let mut editor = settings_page(&host, &statuses_with(&host, RomStatus::Found));
        let volume = index_of(&editor, AUDIO_VOLUME_KEY);
        editor.fields[volume].value = super::super::fields::FieldValue::Text("0.5".to_owned());
        let path = editor.save(false).expect("the save writes");
        assert_eq!(path, test.settings_file());
        assert_eq!(
            std::fs::read_to_string(&path).expect("file reads"),
            "; Neetan settings\nconfig-folder = games\nkey.F1 = Stop\nmystery = 1\n\n\
             crt = on\n# end\naudio-volume = 0.5\n"
        );
    }

    /// Verifies that without a settings file the page edits neetan.conf in the root and that Save creates it.
    #[test]
    fn a_save_without_a_settings_file_creates_it_in_the_root() {
        let test = SettingsRoot::new("create", None);
        let host = test.host(&[]);
        assert_eq!(host.location().file, None);
        let mut editor = settings_page(&host, &statuses_with(&host, RomStatus::Found));
        assert_eq!(editor.path(), test.settings_file());
        assert_eq!(
            editor.root_row().expect("a root row").note,
            root_note(SettingsSource::NotFound)
        );
        let backend = index_of(&editor, BACKEND_KEY);
        let legacy = editor.fields[backend]
            .choices
            .iter()
            .position(|choice| *choice == "legacy")
            .expect("the legacy backend");
        editor.fields[backend].select(Some(legacy));
        editor.save(false).expect("the save creates the file");
        assert_eq!(
            std::fs::read_to_string(test.settings_file()).expect("file reads"),
            "backend = legacy\n"
        );
        let host = test.host(&[]);
        editor.host_changed(&host, None);
        assert_eq!(
            editor.root_row().expect("a root row").note,
            root_note(SettingsSource::CurrentDirectory)
        );
    }

    /// Verifies that a key that a command line flag sets shows the value of the flag, that the ROM status follows the folder of the flag, and that Save still writes the value of the field.
    #[test]
    fn a_flag_shows_its_value_and_the_save_writes_the_file_value() {
        let test = SettingsRoot::new("flags", Some("audio-volume = 0.8\n"));
        let folder = first_rom_folder();
        let flagged = test.root().join("flagged");
        let flagged_text = flagged.display().to_string();
        let host = test.host(&[
            (AUDIO_VOLUME_KEY, "0.25"),
            (BACKEND_KEY, "legacy"),
            (folder.key, &flagged_text),
            (CONFIG_FOLDER_KEY, "elsewhere"),
        ]);
        let statuses = check_all_folders(&host);
        assert!(statuses.iter().any(|status| status.location
            == RomFolderPath {
                path: flagged.clone(),
                explicit: true
            }));
        let mut editor = settings_page(&host, &statuses);
        let volume = find_setting(AUDIO_VOLUME_KEY).expect("a key");
        assert_eq!(
            editor.extra_note(volume).as_deref(),
            Some("The command line sets 0.25 for this run.")
        );
        assert_eq!(
            editor
                .extra_note(find_setting(BACKEND_KEY).expect("a key"))
                .as_deref(),
            Some("The command line sets legacy for this run. The change applies after a restart.")
        );
        let config_folder = find_setting(CONFIG_FOLDER_KEY).expect("a key");
        let notes = editor
            .host_path_note(index_of(&editor, CONFIG_FOLDER_KEY), config_folder)
            .expect("the config folder shows its folder");
        assert_eq!(
            notes,
            HostPathNote {
                note: Note::unmarked(
                    test.root()
                        .join(DEFAULT_CONFIG_FOLDER)
                        .display()
                        .to_string()
                )
                .shown_as_path(),
                flag: Some(Note::plain("The command line sets elsewhere for this run.")),
                error: false,
            }
        );
        let notes = editor
            .host_path_note(
                index_of(&editor, folder.key),
                find_setting(folder.key).expect("a key"),
            )
            .expect("a ROM key has a status");
        let expected_status = check_rom_folder(&folder, &flagged);
        let (status, error) = rom_status_note(&folder, &expected_status, true);
        assert_eq!(
            notes,
            HostPathNote {
                note: status,
                flag: Some(Note::plain(format!(
                    "The command line sets {flagged_text} for this run."
                ))),
                error,
            }
        );
        let mut harness = GuiHarness::new(960, 600);
        show(&mut harness, &mut editor);
        let rows = &editor.host_settings().expect("a page").roms;
        let row = rows
            .iter()
            .find(|row| row.folder.key == folder.key)
            .expect("the row of the flag");
        assert_eq!(
            row.checked.as_ref().map(|(path, _)| path.clone()),
            Some(flagged),
            "the page keeps the check of the start"
        );

        let index = index_of(&editor, AUDIO_VOLUME_KEY);
        assert_eq!(editor.fields[index].text(), "0.8");
        editor.fields[index].value = super::super::fields::FieldValue::Text("0.6".to_owned());
        editor.save(false).expect("the save writes");
        assert_eq!(
            std::fs::read_to_string(test.settings_file()).expect("file reads"),
            "audio-volume = 0.6\n"
        );
    }

    /// Verifies that the row of the GUI theme shows its built-in default and the restart note, also under a flag.
    #[test]
    fn the_gui_theme_row_shows_the_restart_note() {
        let theme = find_setting(GUI_THEME_KEY).expect("a key");
        let test = SettingsRoot::new("gui_theme", Some("gui-theme = light\n"));
        let host = test.host(&[]);
        let editor = settings_page(&host, &statuses_with(&host, RomStatus::Found));
        assert_eq!(editor.extra_note(theme).as_deref(), Some(RESTART_NOTE));
        assert_eq!(
            placeholder(&editor, GUI_THEME_KEY).as_deref(),
            Some("Built-in default: dark")
        );
        let host = test.host(&[(GUI_THEME_KEY, "dark")]);
        let editor = settings_page(&host, &statuses_with(&host, RomStatus::Found));
        assert_eq!(
            editor.extra_note(theme).as_deref(),
            Some("The command line sets dark for this run. The change applies after a restart.")
        );
    }

    /// Verifies that a set GUI font whose file does not exist shows the warning with the restart note, and that an existing font and an unset key show the plain notes without a warning.
    #[test]
    fn a_missing_gui_font_shows_a_warning() {
        let font = find_setting(GUI_FONT_KEY).expect("a key");
        let restart = (Note::plain(RESTART_NOTE), None, false);

        let missing = SettingsRoot::new("missing_font", Some("gui-font = font/missing.ttf\n"));
        let host = missing.host(&[]);
        let editor = settings_page(&host, &statuses_with(&host, RomStatus::Found));
        let index = index_of(&editor, GUI_FONT_KEY);
        assert_eq!(editor.missing[index], [true]);
        assert_eq!(
            editor.path_notes(index, font, None),
            (
                Note::warning(format!("{MISSING_FONT_NOTE} {RESTART_NOTE}")),
                None,
                false
            )
        );

        let present = SettingsRoot::new("present_font", Some("gui-font = font/present.ttf\n"));
        std::fs::create_dir(present.root().join("font")).expect("folder is created");
        std::fs::write(present.root().join("font/present.ttf"), b"font").expect("font is written");
        let host = present.host(&[]);
        let editor = settings_page(&host, &statuses_with(&host, RomStatus::Found));
        let index = index_of(&editor, GUI_FONT_KEY);
        assert_eq!(editor.missing[index], [false]);
        let inherited = placeholder(&editor, GUI_FONT_KEY).expect("a built-in font");
        assert_eq!(
            editor.path_notes(index, font, None),
            (
                Note::plain(format!("{inherited}. {RESTART_NOTE}")),
                None,
                false
            )
        );

        let unset = SettingsRoot::new("unset_font", Some(""));
        let host = unset.host(&[]);
        let editor = settings_page(&host, &statuses_with(&host, RomStatus::Found));
        let index = index_of(&editor, GUI_FONT_KEY);
        assert_eq!(editor.path_notes(index, font, None), restart);
    }

    /// Verifies that a ROM folder flag shows once as the note above the ROM rows, that an unset ROM row shows only the status of the folder below the flag, and that a ROM key of the file shows no flag note.
    #[test]
    fn a_rom_folder_flag_shows_once_above_the_rom_rows() {
        let folder = first_rom_folder();
        let test = SettingsRoot::new("rom_folder_flag", Some(""));
        let flagged = test.root().join("firmware");
        let flagged_text = flagged.display().to_string();
        let host = test.host(&[(ROMS_FOLDER_KEY, &flagged_text)]);
        let statuses = check_all_folders(&host);
        let editor = settings_page(&host, &statuses);
        let notes = editor
            .host_path_note(
                index_of(&editor, folder.key),
                find_setting(folder.key).expect("a key"),
            )
            .expect("a ROM key has a status");
        let check = check_rom_folder(&folder, &flagged.join(folder.name));
        let (status, error) = rom_status_note(&folder, &check, false);
        assert_eq!(
            notes,
            HostPathNote {
                note: status,
                flag: None,
                error,
            }
        );
        assert!(!notes.error, "an unset key takes no edge");
        assert_eq!(
            editor
                .host_settings()
                .expect("a page")
                .roms_folder_flag_note(),
            Some(Note::plain(format!(
                "The command line sets the ROM folder {flagged_text} for this run."
            )))
        );
        let notes = editor
            .host_path_note(
                index_of(&editor, ROMS_FOLDER_KEY),
                find_setting(ROMS_FOLDER_KEY).expect("a key"),
            )
            .expect("the ROM folder shows its folder");
        assert_eq!(notes.flag, Some(Note::plain(flag_sentence(&flagged_text))));

        let own = SettingsRoot::new(
            "rom_folder_flag_own",
            Some(&format!("{} = sets/own\n", folder.key)),
        );
        let host = own.host(&[(ROMS_FOLDER_KEY, &flagged_text)]);
        let editor = settings_page(&host, &check_all_folders(&host));
        let notes = editor
            .host_path_note(
                index_of(&editor, folder.key),
                find_setting(folder.key).expect("a key"),
            )
            .expect("a ROM key has a status");
        assert_eq!(notes.flag, None, "the key of the file wins over the flag");

        let plain = SettingsRoot::new("rom_folder_flag_none", Some(""));
        let host = plain.host(&[]);
        let editor = settings_page(&host, &check_all_folders(&host));
        assert_eq!(
            editor
                .host_settings()
                .expect("a page")
                .roms_folder_flag_note(),
            None
        );
    }

    /// Verifies that Ctrl+S saves the page and that a file that changed outside stays until an overwrite.
    #[test]
    fn the_page_saves_with_the_editor_keys_and_checks_the_stamp() {
        let test = SettingsRoot::new("stamp", Some("backend = modern\n"));
        let host = test.host(&[]);
        let mut editor = settings_page(&host, &statuses_with(&host, RomStatus::Found));
        let mut harness = GuiHarness::new(960, 600);
        show(&mut harness, &mut editor);
        harness.input.text = "games".to_owned();
        show(&mut harness, &mut editor);
        std::fs::write(test.settings_file(), "backend = legacy\n").expect("file is written");
        harness.input.shortcut = Some(Shortcut::ControlS);
        let pass = show(&mut harness, &mut editor);
        assert!(matches!(
            pass.save_error,
            Some(crate::editor::SaveFailure::ChangedOutside(_))
        ));
        assert_eq!(
            std::fs::read_to_string(test.settings_file()).expect("file reads"),
            "backend = legacy\n"
        );
        editor.overwrite().expect("the overwrite writes");
        assert_eq!(
            std::fs::read_to_string(test.settings_file()).expect("file reads"),
            "backend = modern\nconfig-folder = games\n"
        );
        harness.input.navigation = Some(Navigation::NextTab);
        show(&mut harness, &mut editor);
        assert_eq!(editor.current_section(), EditorSection::Roms);
        let first_rom = host_fields()
            .into_iter()
            .find(|(section, _)| *section == EditorSection::Roms)
            .map(|(_, source)| field_id(source.key()));
        assert_eq!(harness.state.focused(), first_rom);
    }

    /// Verifies the note of the root after each step of the discovery.
    #[test]
    fn the_root_note_follows_the_discovery() {
        for (source, text) in [
            (SettingsSource::CommandLine, "--settings"),
            (SettingsSource::CurrentDirectory, "current directory"),
            (SettingsSource::ExecutableFolder, "executable"),
            (SettingsSource::PrefPath, "pref path"),
            (SettingsSource::NotFound, "Save creates"),
        ] {
            assert!(root_note(source).contains(text), "{source:?}");
            assert!(root_note(source).ends_with('.'), "{source:?}");
        }
    }
}
