//! The ROM rows of the settings page: the status note of each ROM key and
//! the check of one ROM folder after the user changes its path.

use std::path::PathBuf;

use common::info;
use gui::ui::UiContext;
use machine_factory::{
    config::{
        phrases::join_names,
        settings::ROMS_FOLDER_KEY,
        systems::{RomFolder, RomOwner, rom_folders},
    },
    rom_status::{RomCheck, RomStatus, check_rom_folder},
};

use super::{Editor, Subject, host_settings::HostSettings, notes::Note, rows::field_id};
use crate::rom_folders::{RomSeverity, rom_severity};

/// The status of a ROM folder in which every model finds its ROM set.
const FOUND_TEXT: &str = "Found";

/// The status of a set ROM key whose folder holds no ROM set.
const EMPTY_FOLDER_TEXT: &str = "Missing. The folder holds no ROM set.";

/// The status of a set ROM key whose folder does not exist.
const NO_FOLDER_TEXT: &str = "Missing. The folder does not exist.";

/// The status of the ROM folder of a system that runs on its HLE BIOS
/// without the ROM set.
const HLE_BIOS_TEXT: &str = "Not found. The HLE BIOS runs.";

/// The status of the MT-32 ROM folder without a ROM set.
const MT32_TEXT: &str = "Not found. The setting midi = mt32 needs it.";

/// The status of the SC-55 ROM folder without a ROM set.
const SC55_TEXT: &str = "Not found. The setting midi = sc55 needs it.";

/// The status of a ROM folder that this build cannot check.
const UNSUPPORTED_TEXT: &str = "No ROM check in this build.";

/// The status of a ROM folder that the page has not checked yet.
const NOT_CHECKED_TEXT: &str = "Not checked yet.";

/// The note of the ROM status `check` of `folder` and whether the field
/// takes the error edge. The severity of the status picks the look of the
/// note. A set key whose folder does not exist takes the edge and a
/// warning. A set key whose existing folder holds no ROM set that the
/// system needs takes the edge as well. A MIDI module without its ROM set
/// in an existing folder leaves the launch running without the module and
/// its key takes no edge.
pub(crate) fn rom_status_note(folder: &RomFolder, check: &RomCheck, set: bool) -> (Note, bool) {
    let checked = check.status != RomStatus::Unsupported;
    if set && checked && !check.folder_exists {
        return (Note::warning(NO_FOLDER_TEXT), true);
    }
    let missing = check.status == RomStatus::Missing;
    let text = match check.status {
        RomStatus::Found => FOUND_TEXT.to_owned(),
        RomStatus::Partial => format!(
            "Partial. No ROM set for {}.",
            join_names(&check.models_without_set, "and")
        ),
        RomStatus::Missing if set => EMPTY_FOLDER_TEXT.to_owned(),
        RomStatus::Missing => format!("Missing. {} games cannot start.", folder.label),
        RomStatus::NotFound => match folder.owner {
            RomOwner::System(_) => HLE_BIOS_TEXT,
            RomOwner::Mt32 => MT32_TEXT,
            RomOwner::Sc55 => SC55_TEXT,
        }
        .to_owned(),
        RomStatus::Unsupported => UNSUPPORTED_TEXT.to_owned(),
    };
    let note = match rom_severity(check.status, folder.required) {
        RomSeverity::Found => Note::found(text),
        RomSeverity::Warning => Note::warning(text),
        RomSeverity::Unmarked => Note::unmarked(text),
    };
    (note, missing && set)
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
    /// The absolute folder that the last check read, and its result.
    /// `None` before the first check.
    pub(crate) checked: Option<(PathBuf, RomCheck)>,
}

impl Editor {
    /// The note of the ROM row `row` of `settings` for the field at `index`
    /// and whether the field takes the error edge. A key that the field or
    /// a command line flag sets counts as set. A field whose folder
    /// differs from the folder of the last check shows that the folder is
    /// not checked yet, without a mark and without the edge.
    pub(super) fn rom_note(
        &self,
        settings: &HostSettings,
        row: &RomRow,
        index: usize,
    ) -> (Note, bool) {
        let location = settings.rom_location(&row.folder, &self.fields);
        match &row.checked {
            Some((checked, check)) if *checked == location => {
                let set = self.fields[index].is_set() || settings.flag(row.folder.key).is_some();
                rom_status_note(&row.folder, check, set)
            }
            Some(_) | None => (Note::unmarked(NOT_CHECKED_TEXT), false),
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
        let pending: Vec<(usize, PathBuf)> = settings
            .roms
            .iter()
            .enumerate()
            .filter(|(_, row)| !context.has_focus(field_id(row.folder.key)))
            .filter_map(|(index, row)| {
                let location = settings.rom_location(&row.folder, &self.fields);
                let current = row
                    .checked
                    .as_ref()
                    .is_some_and(|(checked, _)| *checked == location);
                (!current).then_some((index, location))
            })
            .collect();
        let Subject::Host(settings) = &mut self.subject else {
            return false;
        };
        for (index, location) in &pending {
            let row = &mut settings.roms[*index];
            let check = check_rom_folder(&row.folder, location);
            info!(
                "Settings: checked the {} ROM folder {}: {:?}",
                row.folder.label,
                location.display(),
                check.status
            );
            row.checked = Some((location.clone(), check));
        }
        !pending.is_empty()
    }
}

#[cfg(test)]
mod tests {
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
            rows::browse_id,
            test_support::{
                SettingsRoot, first_rom_folder, index_of, settings_page, show, statuses_with,
            },
        },
        gui_test_support::GuiHarness,
        rom_folders::check_all_folders,
    };

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
            rom_status_note(&required, &check(RomStatus::Found, &[]), true),
            (Note::found("Found"), false)
        );
        let partial = check(RomStatus::Partial, &["PC9801F", "PC9801VM"]);
        assert_eq!(
            rom_status_note(&required, &partial, false),
            (
                Note::warning("Partial. No ROM set for PC9801F and PC9801VM."),
                false
            )
        );
        assert_eq!(
            rom_status_note(&optional, &partial, true),
            (
                Note::unmarked("Partial. No ROM set for PC9801F and PC9801VM."),
                false
            )
        );
        assert_eq!(
            rom_status_note(&required, &check(RomStatus::Missing, &[]), false),
            (
                Note::warning(format!("Missing. {} games cannot start.", required.label)),
                false
            )
        );
        assert_eq!(
            rom_status_note(&required, &check(RomStatus::Missing, &[]), true),
            (Note::warning("Missing. The folder holds no ROM set."), true)
        );
        let no_folder = RomCheck {
            folder_exists: false,
            ..check(RomStatus::Missing, &[])
        };
        assert_eq!(
            rom_status_note(&required, &no_folder, true),
            (Note::warning("Missing. The folder does not exist."), true)
        );
        assert_eq!(
            rom_status_note(&required, &no_folder, false),
            (
                Note::warning(format!("Missing. {} games cannot start.", required.label)),
                false
            )
        );
        assert_eq!(
            rom_status_note(&optional, &check(RomStatus::NotFound, &[]), false),
            (Note::unmarked("Not found. The HLE BIOS runs."), false)
        );
        assert_eq!(
            rom_status_note(&optional, &check(RomStatus::NotFound, &[]), true),
            (Note::unmarked("Not found. The HLE BIOS runs."), false)
        );
        let optional_no_folder = RomCheck {
            folder_exists: false,
            ..check(RomStatus::NotFound, &[])
        };
        assert_eq!(
            rom_status_note(&optional, &optional_no_folder, true),
            (Note::warning("Missing. The folder does not exist."), true)
        );
        assert_eq!(
            rom_status_note(&optional, &optional_no_folder, false),
            (Note::unmarked("Not found. The HLE BIOS runs."), false)
        );
        let unsupported_no_folder = RomCheck {
            folder_exists: false,
            ..check(RomStatus::Unsupported, &[])
        };
        assert_eq!(
            rom_status_note(&required, &unsupported_no_folder, true),
            (Note::unmarked("No ROM check in this build."), false)
        );
        assert_eq!(
            rom_status_note(&required, &check(RomStatus::Unsupported, &[]), true),
            (Note::unmarked("No ROM check in this build."), false)
        );
        let midi: Vec<RomFolder> = rom_folders()
            .filter(|folder| !matches!(folder.owner, RomOwner::System(_)))
            .collect();
        for folder in &midi {
            let expected = (
                Note::unmarked(format!(
                    "Not found. The setting midi = {} needs it.",
                    folder.name
                )),
                false,
            );
            let empty_folder = check(RomStatus::NotFound, &[]);
            assert_eq!(rom_status_note(folder, &empty_folder, true), expected);
            let no_folder = RomCheck {
                folder_exists: false,
                ..empty_folder
            };
            assert_eq!(
                rom_status_note(folder, &no_folder, true),
                (Note::warning("Missing. The folder does not exist."), true)
            );
            assert_eq!(rom_status_note(folder, &no_folder, false), expected);
        }

        let roms = TemporaryFolder::new("settings_page_test_statuses");
        for system in compiled_systems() {
            let folder = system.rom_folder();
            let path = roms.path().join(folder.name);
            std::fs::create_dir(&path).expect("folder is created");
            let result = check_rom_folder(&folder, &path);
            let (note, _) = rom_status_note(&folder, &result, false);
            let expected = if folder.required {
                Note::warning(format!("Missing. {} games cannot start.", folder.label))
            } else {
                Note::unmarked(HLE_BIOS_TEXT)
            };
            assert_eq!(note, expected, "{}", folder.name);
        }
    }

    /// Verifies that a new path of one ROM key shows that its folder is not checked yet while the field holds the focus, checks only that folder once the field loses the focus, and that the other rows keep their statuses.
    #[test]
    fn a_changed_rom_path_checks_only_its_folder() {
        let test = SettingsRoot::new("one_check", Some(""));
        let host = test.host(&[]);
        let mut editor = settings_page(&host, &statuses_with(&host, RomStatus::Found));
        let folder = first_rom_folder();
        let empty = test.root().join("empty");
        std::fs::create_dir(&empty).expect("folder is created");
        let mut harness = GuiHarness::new(960, 600);
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
        let rows = &editor.host_settings().expect("a page").roms;
        assert_eq!(*rows, before, "the focused field defers the check");
        assert_eq!(
            note_of(&editor, folder.key),
            (Note::unmarked(NOT_CHECKED_TEXT), false)
        );

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
        let mut harness = GuiHarness::new(960, 600);
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
