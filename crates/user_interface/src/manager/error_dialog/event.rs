//! The events and the details of the error dialogs and their texts in a catalog.

use i18n::{Catalog, Key, Message};

use crate::{
    editor::{EditedFile, OpenError, SaveFailure},
    texts::errors,
};

/// The event of an error dialog. The message of the dialog tells it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ErrorEvent {
    /// The game with this title did not start.
    LaunchFailed(String),
    /// A new game of the system with this label did not open in the editor.
    NewGameOpenFailed(&'static str),
    /// A saved settings file does not read.
    SettingsNotApplied,
    /// The file did not open in the editor.
    OpenFailed(EditedFile),
    /// The file changed outside the manager since the editor read it.
    ChangedOutside(EditedFile),
    /// The file did not save.
    SaveFailed(EditedFile),
    /// The core of the system with this label panicked. The label of a game
    /// that the manager does not know is `None`.
    GameStopped {
        /// The label of the system.
        system: Option<&'static str>,
        /// Whether the disk images were saved after the panic.
        saved: bool,
    },
    /// A game ended while the write back of its disk images failed. The
    /// title of a game that the manager does not know is `None`.
    MediaNotSaved(Option<String>),
    /// The game file with this name was not deleted.
    DeleteFailed(String),
}

/// The texts of one sentence about the file of the editor for each kind of file.
#[derive(Clone, Copy)]
struct SubjectMessages {
    /// The sentence about a game with its name.
    game: fn(&dyn std::fmt::Display) -> Message<'_, 1>,
    /// The sentence about the system defaults of a system.
    defaults: fn(&dyn std::fmt::Display) -> Message<'_, 1>,
    /// The sentence about the settings.
    settings: Key,
}

/// Writes the sentence about the file of the editor of `subject` with the
/// texts `messages` of the kinds of files.
fn write_subject(
    catalog: &Catalog,
    out: &mut String,
    subject: &EditedFile,
    messages: SubjectMessages,
) {
    let _ = match subject {
        EditedFile::Game { name, .. } => catalog.write(out, (messages.game)(name)),
        EditedFile::SystemDefaults(system) => {
            catalog.write(out, (messages.defaults)(&system.label))
        }
        EditedFile::Settings => {
            out.push_str(catalog.text(messages.settings));
            Ok(())
        }
    };
}

impl ErrorEvent {
    /// Writes the sentence about the event in `catalog`.
    pub(super) fn write(&self, catalog: &Catalog, out: &mut String) {
        let _ = match self {
            Self::LaunchFailed(game) => catalog.write(out, errors::launch_failed(game)),
            Self::NewGameOpenFailed(system) => {
                catalog.write(out, errors::new_game_open_failed(system))
            }
            Self::SettingsNotApplied => {
                out.push_str(catalog.text(errors::SETTINGS_NOT_APPLIED));
                Ok(())
            }
            Self::OpenFailed(subject) => {
                write_subject(
                    catalog,
                    out,
                    subject,
                    SubjectMessages {
                        game: errors::game_open_failed,
                        defaults: errors::defaults_open_failed,
                        settings: errors::SETTINGS_OPEN_FAILED,
                    },
                );
                Ok(())
            }
            Self::ChangedOutside(subject) => {
                write_subject(
                    catalog,
                    out,
                    subject,
                    SubjectMessages {
                        game: errors::game_changed_outside,
                        defaults: errors::defaults_changed_outside,
                        settings: errors::SETTINGS_CHANGED_OUTSIDE,
                    },
                );
                Ok(())
            }
            Self::SaveFailed(subject) => {
                write_subject(
                    catalog,
                    out,
                    subject,
                    SubjectMessages {
                        game: errors::game_save_failed,
                        defaults: errors::defaults_save_failed,
                        settings: errors::SETTINGS_SAVE_FAILED,
                    },
                );
                Ok(())
            }
            Self::GameStopped {
                system: Some(system),
                saved: true,
            } => catalog.write(out, errors::core_stopped_saved(system)),
            Self::GameStopped {
                system: Some(system),
                saved: false,
            } => catalog.write(out, errors::core_stopped_not_saved(system)),
            Self::GameStopped {
                system: None,
                saved: true,
            } => {
                out.push_str(catalog.text(errors::EMULATOR_STOPPED_SAVED));
                Ok(())
            }
            Self::GameStopped {
                system: None,
                saved: false,
            } => {
                out.push_str(catalog.text(errors::EMULATOR_STOPPED_NOT_SAVED));
                Ok(())
            }
            Self::MediaNotSaved(Some(game)) => catalog.write(out, errors::media_not_saved(game)),
            Self::MediaNotSaved(None) => {
                out.push_str(catalog.text(errors::GAME_MEDIA_NOT_SAVED));
                Ok(())
            }
            Self::DeleteFailed(name) => catalog.write(out, errors::delete_failed(name)),
        };
    }
}

/// The text in the detail well of an error dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ErrorDetail {
    /// The error text of the operating system, of the machine or of a
    /// file, which stays in English.
    Error(String),
    /// The error of the operating system in English at the file of the
    /// shown path `path`.
    PathError {
        /// The shown path of the file.
        path: String,
        /// The error of the operating system in English.
        error: String,
    },
    /// The fault that kept the editor from opening its file.
    Open(OpenError),
    /// The reason that a save did not write the file of the editor.
    Save(SaveFailure),
}

impl ErrorDetail {
    /// Writes the detail in `catalog`.
    pub(super) fn write(&self, catalog: &Catalog, out: &mut String) {
        let _ = match self {
            Self::Error(error) => {
                out.push_str(error);
                Ok(())
            }
            Self::PathError { path, error } => catalog.write(out, errors::path_error(path, error)),
            Self::Open(error) => error.write(catalog, out),
            Self::Save(failure) => failure.write(catalog, out),
        };
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::{
        editor::test_support::system_named,
        texts::{english_text, test_catalog},
    };

    /// The German texts of a new game that did not open and of the faults of the editor.
    const GERMAN_ERRORS: &str = "[errors]\n\
                                 new-game-open-failed = Ein neues {system} Spiel ging nicht auf.\n\
                                 not-compiled = das System {system} fehlt\n\
                                 no-free-name = {folder}: jeder Name ist vergeben\n\
                                 path-error = {error} bei {path}\n";

    /// The game file of the PC-98 game Edge.
    fn game() -> EditedFile {
        EditedFile::Game {
            system: system_named("pc98"),
            name: "Edge".to_owned(),
        }
    }

    /// The English sentence of `event`.
    fn english_event(event: &ErrorEvent) -> String {
        english_text(|catalog, out| event.write(catalog, out))
    }

    /// The English text of `detail`.
    fn english_detail(detail: &ErrorDetail) -> String {
        english_text(|catalog, out| detail.write(catalog, out))
    }

    /// Verifies the English sentence of every event of a game, of the emulator and of the settings.
    #[test]
    fn every_event_writes_its_sentence() {
        let cases = [
            (
                ErrorEvent::LaunchFailed("Edge".to_owned()),
                "Edge did not start.",
            ),
            (
                ErrorEvent::NewGameOpenFailed("PC-98"),
                "A new PC-98 game did not open in the editor.",
            ),
            (
                ErrorEvent::SettingsNotApplied,
                "The settings file was saved but does not read. The manager keeps the settings \
                 that it read before.",
            ),
            (
                ErrorEvent::GameStopped {
                    system: Some("PC-98"),
                    saved: true,
                },
                "The PC-98 core stopped with an error. The disk images were saved.",
            ),
            (
                ErrorEvent::GameStopped {
                    system: Some("PC-98"),
                    saved: false,
                },
                "The PC-98 core stopped with an error. Some disk images were not saved.",
            ),
            (
                ErrorEvent::GameStopped {
                    system: None,
                    saved: true,
                },
                "The emulator core stopped with an error. The disk images were saved.",
            ),
            (
                ErrorEvent::GameStopped {
                    system: None,
                    saved: false,
                },
                "The emulator core stopped with an error. Some disk images were not saved.",
            ),
            (
                ErrorEvent::MediaNotSaved(Some("Edge".to_owned())),
                "Edge ended. Some of its disk images were not saved.",
            ),
            (
                ErrorEvent::MediaNotSaved(None),
                "The game ended. Some of its disk images were not saved.",
            ),
            (
                ErrorEvent::DeleteFailed("Edge".to_owned()),
                "Edge was not deleted.",
            ),
        ];
        for (event, expected) in cases {
            assert_eq!(english_event(&event), expected, "{event:?}");
        }
    }

    /// Verifies the sentences of the events of the editor for a game file, a system defaults file and the settings file.
    #[test]
    fn the_editor_events_follow_the_file_kind() {
        let defaults = EditedFile::SystemDefaults(system_named("pc98"));
        let settings = EditedFile::Settings;
        let cases = [
            (
                ErrorEvent::SaveFailed(game()),
                "Edge was not saved. The editor keeps the changes.",
            ),
            (
                ErrorEvent::SaveFailed(defaults.clone()),
                "The PC-98 system defaults were not saved. The editor keeps the changes.",
            ),
            (
                ErrorEvent::SaveFailed(settings.clone()),
                "The settings were not saved. The editor keeps the changes.",
            ),
            (
                ErrorEvent::OpenFailed(game()),
                "Edge did not open in the editor.",
            ),
            (
                ErrorEvent::OpenFailed(defaults.clone()),
                "The PC-98 system defaults did not open in the editor.",
            ),
            (
                ErrorEvent::OpenFailed(settings.clone()),
                "The settings did not open in the editor.",
            ),
        ];
        for (event, expected) in cases {
            assert_eq!(english_event(&event), expected, "{event:?}");
        }
        for (subject, start) in [
            (game(), "Edge was"),
            (defaults, "The PC-98 system defaults were"),
            (settings, "The settings were"),
        ] {
            let message = english_event(&ErrorEvent::ChangedOutside(subject));
            assert!(
                message.starts_with(&format!("{start} not saved. Overwrite replaces")),
                "{message}"
            );
        }
    }

    /// Verifies that the faults of an open and of a save of the editor write their sentences in English and in the catalog of a language file, and that an error text stays as it is.
    #[test]
    fn the_details_follow_the_catalog() {
        let folder = PathBuf::from("/root/config/pc98");
        let file = folder.join("edge.conf");
        let cases = [
            (ErrorDetail::Error("gone".to_owned()), "gone".to_owned()),
            (
                ErrorDetail::PathError {
                    path: "config/pc98/edge.conf".to_owned(),
                    error: "Permission denied".to_owned(),
                },
                "config/pc98/edge.conf: Permission denied".to_owned(),
            ),
            (
                ErrorDetail::Open(OpenError::Failed {
                    path: file.clone(),
                    error: "gone".to_owned(),
                }),
                "/root/config/pc98/edge.conf: gone".to_owned(),
            ),
            (
                ErrorDetail::Save(SaveFailure::Error {
                    path: file.clone(),
                    error: "denied".to_owned(),
                }),
                "/root/config/pc98/edge.conf: denied".to_owned(),
            ),
            (
                ErrorDetail::Open(OpenError::NoFile(file.clone())),
                "/root/config/pc98/edge.conf: cannot read the file: the path holds no file"
                    .to_owned(),
            ),
            (
                ErrorDetail::Open(OpenError::KeptWriting(file)),
                "/root/config/pc98/edge.conf: another program kept writing the file while the \
                 editor read it"
                    .to_owned(),
            ),
            (
                ErrorDetail::Open(OpenError::NotCompiled("pc98")),
                OpenError::NotCompiled("pc98").to_string(),
            ),
            (
                ErrorDetail::Save(SaveFailure::NoFreeName(folder.clone())),
                SaveFailure::NoFreeName(folder.clone()).to_string(),
            ),
            (
                ErrorDetail::Save(SaveFailure::ChangedOutside(
                    "config/pc98/edge.conf".to_owned(),
                )),
                "config/pc98/edge.conf changed outside the manager since the editor read it."
                    .to_owned(),
            ),
        ];
        for (detail, expected) in &cases {
            assert_eq!(&english_detail(detail), expected, "{detail:?}");
        }
        assert_eq!(
            english_detail(&ErrorDetail::Open(OpenError::NotCompiled("pc98"))),
            "the system pc98 is not compiled"
        );
        assert_eq!(
            english_detail(&ErrorDetail::Save(SaveFailure::NoFreeName(folder.clone()))),
            "/root/config/pc98: every derived file name exists"
        );
        let german = test_catalog(GERMAN_ERRORS);
        let mut text = String::new();
        ErrorDetail::Open(OpenError::NotCompiled("pc98")).write(&german, &mut text);
        assert_eq!(text, "das System pc98 fehlt");
        text.clear();
        ErrorDetail::Save(SaveFailure::NoFreeName(folder)).write(&german, &mut text);
        assert_eq!(text, "/root/config/pc98: jeder Name ist vergeben");
        text.clear();
        ErrorDetail::PathError {
            path: "config/pc98/edge.conf".to_owned(),
            error: "Permission denied".to_owned(),
        }
        .write(&german, &mut text);
        assert_eq!(text, "Permission denied bei config/pc98/edge.conf");
        text.clear();
        ErrorEvent::NewGameOpenFailed("PC-98").write(&german, &mut text);
        assert_eq!(text, "Ein neues PC-98 Spiel ging nicht auf.");
    }
}
