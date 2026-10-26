//! The ROM rows of the settings page: the status note of each ROM key and
//! the check of one ROM folder after the user changes its path.

use std::path::PathBuf;

use common::info;
use gui::ui::UiContext;
use i18n::ListKind;
use machine_factory::{
    config::{
        settings::ROMS_FOLDER_KEY,
        systems::{RomFolder, RomOwner, rom_folders},
    },
    rom_status::{RomCheck, RomStatus, check_rom_folder},
};

use super::{
    Editor, Subject, host_settings::HostSettings, note_writer::NoteWriter, notes::NoteKind,
    rows::field_id,
};
use crate::{
    rom_folders::{RomSeverity, rom_severity},
    texts::host_settings,
};

/// The ROM status of a ROM key that its note tells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RomNote<'a> {
    /// Every model finds its ROM set.
    Found,
    /// The ROM sets of these models are missing.
    Partial(&'a [&'static str]),
    /// The folder of a set ROM key holds no ROM set.
    EmptyFolder,
    /// The folder of a set ROM key does not exist.
    NoFolder,
    /// The games of the system with this label cannot start.
    Missing(&'static str),
    /// The system runs on its HLE BIOS without the ROM set.
    HleBios,
    /// The MT-32 ROM folder holds no ROM set.
    Mt32,
    /// The SC-55 ROM folder holds no ROM set.
    Sc55,
    /// This build cannot check the folder.
    Unsupported,
    /// The page has not checked the folder yet.
    NotChecked,
}

impl RomNote<'_> {
    /// Adds the sentence of the status.
    pub(crate) fn write(self, writer: &mut NoteWriter) {
        let key = match self {
            Self::Found => host_settings::ROM_FOUND,
            Self::Partial(models) => {
                let models = writer.catalog().display_list(ListKind::And, models);
                writer.message(host_settings::rom_partial(&models));
                return;
            }
            Self::Missing(system) => {
                writer.message(host_settings::rom_missing(&system));
                return;
            }
            Self::EmptyFolder => host_settings::ROM_EMPTY_FOLDER,
            Self::NoFolder => host_settings::ROM_NO_FOLDER,
            Self::HleBios => host_settings::ROM_HLE_BIOS,
            Self::Mt32 => host_settings::ROM_MT32,
            Self::Sc55 => host_settings::ROM_SC55,
            Self::Unsupported => host_settings::ROM_UNSUPPORTED,
            Self::NotChecked => host_settings::ROM_NOT_CHECKED,
        };
        writer.text(key);
    }
}

/// The note of the ROM status `check` of `folder`, its look and whether
/// the field takes the error edge. The severity of the status picks the
/// look of the note. A set key whose folder does not exist takes the edge
/// and a warning. A set key whose existing folder holds no ROM set that the
/// system needs takes the edge as well. A MIDI module without its ROM set
/// in an existing folder leaves the launch running without the module and
/// its key takes no edge.
pub(crate) fn rom_status_note<'a>(
    folder: &RomFolder,
    check: &'a RomCheck,
    set: bool,
) -> (RomNote<'a>, NoteKind, bool) {
    let checked = check.status != RomStatus::Unsupported;
    if set && checked && !check.folder_exists {
        return (RomNote::NoFolder, NoteKind::Warning, true);
    }
    let missing = check.status == RomStatus::Missing;
    let note = match check.status {
        RomStatus::Found => RomNote::Found,
        RomStatus::Partial => RomNote::Partial(&check.models_without_set),
        RomStatus::Missing if set => RomNote::EmptyFolder,
        RomStatus::Missing => RomNote::Missing(folder.label),
        RomStatus::NotFound => match folder.owner {
            RomOwner::System(_) => RomNote::HleBios,
            RomOwner::Mt32 => RomNote::Mt32,
            RomOwner::Sc55 => RomNote::Sc55,
        },
        RomStatus::Unsupported => RomNote::Unsupported,
    };
    let kind = match rom_severity(check.status, folder.required) {
        RomSeverity::Found => NoteKind::Found,
        RomSeverity::Warning => NoteKind::Warning,
        RomSeverity::Unmarked => NoteKind::Unmarked,
    };
    (note, kind, missing && set)
}

/// Returns the ROM folder entry of the ROM key `key`.
pub(super) fn rom_folder_of(key: &str) -> Option<RomFolder> {
    rom_folders().find(|folder| folder.key == key)
}

/// One ROM key of the settings page and the last check of its folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RomRow {
    /// The ROM folder entry of the key.
    pub(crate) folder: RomFolder,
    /// The absolute folder that the next launch reads with the values of
    /// the fields of the last update of the folders of the page.
    pub(crate) location: PathBuf,
    /// The absolute folder that the last check read, and its result.
    /// `None` before the first check.
    pub(crate) checked: Option<(PathBuf, RomCheck)>,
}

impl Editor {
    /// The note of the ROM row `row` of `settings` for the field at `index`,
    /// its look and whether the field takes the error edge. A key that the
    /// field or a command line flag sets counts as set. A field whose
    /// folder differs from the folder of the last check shows that the
    /// folder is not checked yet, without a mark and without the edge.
    pub(super) fn rom_note<'a>(
        &self,
        settings: &HostSettings,
        row: &'a RomRow,
        index: usize,
    ) -> (RomNote<'a>, NoteKind, bool) {
        match &row.checked {
            Some((checked, check)) if *checked == row.location => {
                let set = self.fields[index].is_set() || settings.flag(row.folder.key).is_some();
                rom_status_note(&row.folder, check, set)
            }
            Some(_) | None => (RomNote::NotChecked, NoteKind::Unmarked, false),
        }
    }

    /// Checks the ROM folder of every ROM key whose folder for the next
    /// launch differs from the folder of its last check, once the user
    /// leaves the field. A field of the key or the ROM folder field that
    /// holds the focus in `context` defers the check. Returns whether a
    /// check ran.
    pub(super) fn check_changed_roms(&mut self, context: &UiContext) -> bool {
        let Some(settings) = self.host_settings() else {
            return false;
        };
        if context.has_focus(field_id(ROMS_FOLDER_KEY)) {
            return false;
        }
        let pending: Vec<usize> = settings
            .roms
            .iter()
            .enumerate()
            .filter(|(_, row)| !context.has_focus(field_id(row.folder.key)))
            .filter(|(_, row)| {
                !row.checked
                    .as_ref()
                    .is_some_and(|(checked, _)| *checked == row.location)
            })
            .map(|(index, _)| index)
            .collect();
        let Subject::Host(settings) = &mut self.subject else {
            return false;
        };
        for index in &pending {
            let row = &mut settings.roms[*index];
            let check = check_rom_folder(&row.folder, &row.location);
            info!(
                "Settings: checked the {} ROM folder {}: {:?}",
                row.folder.label,
                row.location.display(),
                check.status
            );
            row.checked = Some((row.location.clone(), check));
        }
        if !pending.is_empty() {
            self.revision += 1;
        }
        !pending.is_empty()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use common::temporary::TemporaryFolder;
    use machine_factory::config::{
        settings::find_setting,
        systems::{SYSTEMS, compiled_systems},
    };

    use super::*;
    use crate::{
        editor::{
            EditorSection,
            fields::FieldValue,
            form::SETTINGS_SECTIONS,
            notes::{
                NoteLook,
                tests::{Note, english_note},
            },
            rows::browse_id,
            test_support::{
                SettingsRoot, first_rom_folder, index_of, settings_page, show, statuses_with,
            },
        },
        gui_test_support::GuiHarness,
        rom_folders::check_all_folders,
    };

    /// The status of a ROM folder in which every model finds its ROM set.
    const FOUND_TEXT: &str = "Found";

    /// The status of the ROM folder of a system that runs on its HLE BIOS.
    const HLE_BIOS_TEXT: &str = "Not found. The HLE BIOS runs.";

    /// The status of a ROM folder that the page has not checked yet.
    const NOT_CHECKED_TEXT: &str = "Not checked yet.";

    /// The English note of the ROM status `check` of `folder` and whether
    /// the field takes the error edge, as [`super::rom_status_note`] gives
    /// them.
    pub(crate) fn rom_status_text(folder: &RomFolder, check: &RomCheck, set: bool) -> (Note, bool) {
        let (note, kind, error) = super::rom_status_note(folder, check, set);
        let text = english_note(|writer| {
            note.write(writer);
            NoteLook::of(kind)
        });
        (text, error)
    }

    /// The note of the ROM key `key` on `editor` and whether its field takes the error edge.
    fn note_of(editor: &Editor, key: &str) -> (Note, bool) {
        let notes = editor
            .host_path_note(index_of(editor, key), find_setting(key).expect("a ROM key"))
            .expect("a ROM key has a status");
        (notes.note, notes.error)
    }

    /// Verifies the found, missing, optional and partial statuses with their texts and looks, and the statuses of the empty temporary ROM folders of every compiled system.
    #[test]
    fn rom_statuses_follow_the_check() {
        let check = |status: RomStatus, models: &[&'static str]| RomCheck {
            status,
            folder_exists: true,
            models_without_set: models.to_vec(),
        };
        let required = SYSTEMS
            .iter()
            .find(|system| system.roms_required)
            .expect("a system needs its ROMs")
            .rom_folder();
        let optional = SYSTEMS
            .iter()
            .find(|system| !system.roms_required)
            .expect("a system runs without its ROMs")
            .rom_folder();
        assert_eq!(
            rom_status_text(&required, &check(RomStatus::Found, &[]), true),
            (Note::found("Found"), false)
        );
        let partial = check(RomStatus::Partial, &["PC9801F", "PC9801VM"]);
        assert_eq!(
            rom_status_text(&required, &partial, false),
            (
                Note::warning("Partial. No ROM set for PC9801F and PC9801VM."),
                false
            )
        );
        assert_eq!(
            rom_status_text(&optional, &partial, true),
            (
                Note::unmarked("Partial. No ROM set for PC9801F and PC9801VM."),
                false
            )
        );
        assert_eq!(
            rom_status_text(&required, &check(RomStatus::Missing, &[]), false),
            (
                Note::warning(&format!("Missing. {} games cannot start.", required.label)),
                false
            )
        );
        assert_eq!(
            rom_status_text(&required, &check(RomStatus::Missing, &[]), true),
            (Note::warning("Missing. The folder holds no ROM set."), true)
        );
        let no_folder = RomCheck {
            folder_exists: false,
            ..check(RomStatus::Missing, &[])
        };
        assert_eq!(
            rom_status_text(&required, &no_folder, true),
            (Note::warning("Missing. The folder does not exist."), true)
        );
        assert_eq!(
            rom_status_text(&required, &no_folder, false),
            (
                Note::warning(&format!("Missing. {} games cannot start.", required.label)),
                false
            )
        );
        assert_eq!(
            rom_status_text(&optional, &check(RomStatus::NotFound, &[]), false),
            (Note::unmarked("Not found. The HLE BIOS runs."), false)
        );
        assert_eq!(
            rom_status_text(&optional, &check(RomStatus::NotFound, &[]), true),
            (Note::unmarked("Not found. The HLE BIOS runs."), false)
        );
        let optional_no_folder = RomCheck {
            folder_exists: false,
            ..check(RomStatus::NotFound, &[])
        };
        assert_eq!(
            rom_status_text(&optional, &optional_no_folder, true),
            (Note::warning("Missing. The folder does not exist."), true)
        );
        assert_eq!(
            rom_status_text(&optional, &optional_no_folder, false),
            (Note::unmarked("Not found. The HLE BIOS runs."), false)
        );
        let unsupported_no_folder = RomCheck {
            folder_exists: false,
            ..check(RomStatus::Unsupported, &[])
        };
        assert_eq!(
            rom_status_text(&required, &unsupported_no_folder, true),
            (Note::unmarked("No ROM check in this build."), false)
        );
        assert_eq!(
            rom_status_text(&required, &check(RomStatus::Unsupported, &[]), true),
            (Note::unmarked("No ROM check in this build."), false)
        );
        let midi: Vec<RomFolder> = rom_folders()
            .filter(|folder| !matches!(folder.owner, RomOwner::System(_)))
            .collect();
        for folder in &midi {
            let expected = (
                Note::unmarked(&format!(
                    "Not found. The setting midi = {} needs it.",
                    folder.name
                )),
                false,
            );
            let empty_folder = check(RomStatus::NotFound, &[]);
            assert_eq!(rom_status_text(folder, &empty_folder, true), expected);
            let no_folder = RomCheck {
                folder_exists: false,
                ..empty_folder
            };
            assert_eq!(
                rom_status_text(folder, &no_folder, true),
                (Note::warning("Missing. The folder does not exist."), true)
            );
            assert_eq!(rom_status_text(folder, &no_folder, false), expected);
        }

        let roms = TemporaryFolder::new("settings_page_test_statuses");
        for system in compiled_systems() {
            let folder = system.rom_folder();
            let path = roms.path().join(folder.name);
            std::fs::create_dir(&path).expect("folder is created");
            let result = check_rom_folder(&folder, &path);
            let (note, _) = rom_status_text(&folder, &result, false);
            let expected = if folder.required {
                Note::warning(&format!("Missing. {} games cannot start.", folder.label))
            } else {
                Note::unmarked(HLE_BIOS_TEXT)
            };
            assert_eq!(note, expected, "{}", folder.name);
        }
    }

    /// Verifies that a new path of one ROM key shows that its folder is not checked yet while the field holds the focus, checks only that folder once the field loses the focus, that the cached note of the next frame shows the result of the check, and that the other rows keep their statuses.
    #[test]
    fn a_changed_rom_path_checks_only_its_folder() {
        let test = SettingsRoot::new("one_check", Some(""));
        let host = test.host(&[]);
        let mut editor = settings_page(&host, &statuses_with(&host, RomStatus::Found));
        let folder = first_rom_folder();
        let empty = test.root().join("empty");
        std::fs::create_dir(&empty).expect("folder is created");
        let mut harness = GuiHarness::minimum();
        show(&mut harness, &mut editor);
        let before: Vec<RomRow> = editor.host_settings().expect("a page").roms.clone();
        assert!(before.iter().all(|row| row.checked.is_some()));
        assert_eq!(
            note_of(&editor, folder.key),
            (Note::found(FOUND_TEXT), false)
        );

        editor.section = SETTINGS_SECTIONS
            .iter()
            .position(|section| *section == EditorSection::Roms)
            .expect("the ROMs section");
        editor.focus_first = true;
        show(&mut harness, &mut editor);
        harness.state.set_focus(field_id(folder.key));
        show(&mut harness, &mut editor);
        harness.input.text = "empty".to_owned();
        show(&mut harness, &mut editor);
        let checked = |rows: &[RomRow]| -> Vec<Option<(PathBuf, RomCheck)>> {
            rows.iter().map(|row| row.checked.clone()).collect()
        };
        let rows = &editor.host_settings().expect("a page").roms;
        assert_eq!(
            checked(rows),
            checked(&before),
            "the focused field defers the check"
        );
        assert_eq!(
            note_of(&editor, folder.key),
            (Note::unmarked(NOT_CHECKED_TEXT), false)
        );
        assert_eq!(editor.cached_note(folder.key), Some(NOT_CHECKED_TEXT));

        harness.state.set_focus(browse_id(folder.key));
        show(&mut harness, &mut editor);
        let rows = &editor.host_settings().expect("a page").roms;
        for (row, old) in rows.iter().zip(&before) {
            if row.folder.key == folder.key {
                let (path, check) = row.checked.as_ref().expect("the row is checked");
                assert_eq!(*path, empty);
                let expected = if folder.required {
                    RomStatus::Missing
                } else {
                    RomStatus::NotFound
                };
                assert_eq!(check.status, expected);
            } else {
                assert_eq!(row, old, "{}", row.folder.key);
            }
        }
        let (note, error) = note_of(&editor, folder.key);
        assert_eq!(error, folder.required);
        assert_ne!(note.text, NOT_CHECKED_TEXT);
        assert_eq!(editor.cached_note(folder.key), Some(note.text.as_str()));
    }

    /// Verifies that the page opens on the statuses of the start without a check, for an empty ROM folder value and for a relative ROM key, and that a new ROM folder value shows an unset key as not checked yet.
    #[test]
    fn the_page_opens_without_a_check() {
        let folder = first_rom_folder();
        let relative = SettingsRoot::new(
            "no_check_relative",
            Some(&format!("{} = sets/first\n", folder.key)),
        );
        let host = relative.host(&[]);
        let statuses = check_all_folders(&host);
        assert!(statuses.iter().any(|status| status.folder.key == folder.key
            && status.location.path == relative.root().join("sets").join("first")));
        let mut editor = settings_page(&host, &statuses);
        let mut harness = GuiHarness::minimum();
        assert!(!harness.frame(|context| editor.check_changed_roms(context)));

        let empty = SettingsRoot::new("no_check_empty", Some("roms-folder =\n"));
        let host = empty.host(&[]);
        let mut editor = settings_page(&host, &check_all_folders(&host));
        assert!(!harness.frame(|context| editor.check_changed_roms(context)));
        assert_ne!(note_of(&editor, folder.key).0.text, NOT_CHECKED_TEXT);
        let roms = index_of(&editor, ROMS_FOLDER_KEY);
        editor.fields[roms].value = FieldValue::Text("firmware".to_owned());
        editor.refresh();
        assert_eq!(
            note_of(&editor, folder.key),
            (Note::unmarked(NOT_CHECKED_TEXT), false)
        );
    }
}
