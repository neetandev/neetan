//! The errors of the configuration of a launch and of the host layers: their kinds with their
//! data, their English text and their text in a catalog.

use std::{fmt, path::PathBuf};

use i18n::Catalog;
use machine_factory::config::settings::{MACHINE_KEY, value_error::ValueError};

use super::{diagnostic::Diagnostic, document::DocumentError};
use crate::texts::{CatalogText, diagnostic, errors, in_catalog};

/// An error of the configuration of a launch or of the host layers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchError {
    /// The `--settings` file at this path does not exist.
    SettingsNotFound(PathBuf),
    /// A configuration file does not read.
    ReadFailed {
        /// The path of the file.
        path: PathBuf,
        /// The error of the read.
        error: DocumentError,
    },
    /// A configuration file holds an error.
    Diagnostic {
        /// The path of the file as text.
        source: String,
        /// The error.
        diagnostic: Diagnostic,
    },
    /// Neither the command line, a game file, the system of the launch nor the settings file
    /// selects a machine.
    NoMachine,
    /// The `machine` of the launch matches no model.
    Machine(ValueError),
    /// The value of a command line flag is invalid.
    InvalidFlag {
        /// The key of the flag.
        key: &'static str,
        /// The fault of the value.
        error: ValueError,
    },
    /// A disk or disc image of the configuration does not exist.
    MissingImage {
        /// The key of the drive.
        key: &'static str,
        /// The path of the image.
        path: PathBuf,
    },
}

impl fmt::Display for LaunchError {
    /// Writes the English text of the error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SettingsNotFound(path) => {
                write!(formatter, "settings file not found: {}", path.display())
            }
            Self::ReadFailed { path, error } => {
                write!(formatter, "Failed to read {}: {error}", path.display())
            }
            Self::Diagnostic { source, diagnostic } => {
                formatter.write_str(&diagnostic.located(source))
            }
            Self::NoMachine => formatter.write_str(
                "the game needs a machine: set machine in the game file or in the settings file, \
                 pass --machine, or put the game file into the folder of its system",
            ),
            Self::Machine(error) => write!(formatter, "invalid {MACHINE_KEY}: {error}"),
            Self::InvalidFlag { key, error } => write!(formatter, "--{key}: {error}"),
            Self::MissingImage { key, path } => {
                write!(formatter, "{key} image not found: {}", path.display())
            }
        }
    }
}

impl std::error::Error for LaunchError {}

impl From<LaunchError> for common::Error {
    /// Turns the error into its English text.
    fn from(error: LaunchError) -> Self {
        Self::Message(common::StringError(error.to_string()))
    }
}

impl CatalogText for LaunchError {
    /// Writes the error through the keys of the section `[errors]`.
    fn write_text(&self, catalog: &Catalog, out: &mut dyn fmt::Write) -> fmt::Result {
        match self {
            Self::SettingsNotFound(path) => {
                catalog.write(out, errors::settings_not_found(&path.display()))
            }
            Self::ReadFailed { path, error } => {
                let error = in_catalog(catalog, error);
                catalog.write(out, errors::read_failed(&path.display(), &error))
            }
            Self::Diagnostic { source, diagnostic } => {
                diagnostic.located_at(source).write_text(catalog, out)
            }
            Self::NoMachine => out.write_str(catalog.text(errors::NO_MACHINE)),
            Self::Machine(error) => {
                let error = in_catalog(catalog, error);
                catalog.write(out, diagnostic::invalid_value(&MACHINE_KEY, &error))
            }
            Self::InvalidFlag { key, error } => {
                let error = in_catalog(catalog, error);
                catalog.write(out, errors::invalid_flag(key, &error))
            }
            Self::MissingImage { key, path } => {
                catalog.write(out, errors::missing_image(key, &path.display()))
            }
        }
    }
}

/// The failure of a launch: a faulty configuration, a machine that fails to start, or a panic
/// while the machine starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchFailure {
    /// The configuration of the launch does not build.
    Configuration(LaunchError),
    /// The machine or the window did not start. The text is the error in English.
    Machine(String),
    /// The machine panicked while it started. The text is the panic message.
    Panic(String),
}

#[cfg(test)]
impl LaunchFailure {
    /// The failure of a launch whose image of FDD 1 at `path` does not exist.
    pub(crate) fn missing_image(path: &str) -> Self {
        Self::Configuration(LaunchError::MissingImage {
            key: "fdd1",
            path: PathBuf::from(path),
        })
    }
}

impl fmt::Display for LaunchFailure {
    /// Writes the English text of the failure.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Configuration(error) => write!(formatter, "{error}"),
            Self::Machine(error) => formatter.write_str(error),
            Self::Panic(message) => {
                write!(formatter, "the machine panicked while starting: {message}")
            }
        }
    }
}

impl CatalogText for LaunchFailure {
    /// Writes the failure through the keys of the section `[errors]`. The error of the machine
    /// and the panic message stay in English.
    fn write_text(&self, catalog: &Catalog, out: &mut dyn fmt::Write) -> fmt::Result {
        match self {
            Self::Configuration(error) => error.write_text(catalog, out),
            Self::Machine(error) => out.write_str(error),
            Self::Panic(message) => catalog.write(out, errors::machine_panicked(message)),
        }
    }
}

#[cfg(test)]
mod tests {
    use machine_factory::config::settings::{MODEL_CHOICES, value_error::ValueError};

    use super::*;
    use crate::{
        config::diagnostic::DiagnosticKind,
        texts::{english, test_catalog},
    };

    /// Verifies the English text of every error and that the English catalog writes the same
    /// text.
    #[test]
    fn every_error_writes_its_english_text() {
        let cases = [
            (
                LaunchError::SettingsNotFound(PathBuf::from("/root/neetan.conf")),
                "settings file not found: /root/neetan.conf",
            ),
            (
                LaunchError::ReadFailed {
                    path: PathBuf::from("/root/edge.conf"),
                    error: DocumentError::InvalidUtf8 { line: 2 },
                },
                "Failed to read /root/edge.conf: line 2: the file is not valid UTF-8",
            ),
            (
                LaunchError::Diagnostic {
                    source: "/root/edge.conf".to_owned(),
                    diagnostic: Diagnostic::error(
                        1,
                        DiagnosticKind::UnknownMachine("X68030".to_owned()),
                    ),
                },
                "/root/edge.conf line 1: unknown machine X68030",
            ),
            (
                LaunchError::NoMachine,
                "the game needs a machine: set machine in the game file or in the settings file, \
                 pass --machine, or put the game file into the folder of its system",
            ),
            (
                LaunchError::InvalidFlag {
                    key: "bios",
                    error: ValueError::OnOff {
                        value: "maybe".to_owned(),
                    },
                },
                "--bios: invalid value 'maybe', expected on or off",
            ),
            (
                LaunchError::MissingImage {
                    key: "fdd1",
                    path: PathBuf::from("/a.d88"),
                },
                "fdd1 image not found: /a.d88",
            ),
        ];
        for (error, text) in cases {
            assert_eq!(error.to_string(), text);
            assert_eq!(in_catalog(english(), &error).to_string(), text);
        }
        let machine = LaunchError::Machine(ValueError::unknown_choice("X68030", MODEL_CHOICES));
        assert!(
            machine
                .to_string()
                .starts_with("invalid machine: unknown value 'X68030', ")
        );
        assert_eq!(
            in_catalog(english(), &machine).to_string(),
            machine.to_string()
        );
    }

    /// Verifies that a catalog writes the error of the machine with the key of the setting and
    /// its own texts.
    #[test]
    fn a_catalog_writes_the_machine_error_with_the_key() {
        let german = test_catalog(
            "[diagnostic]\ninvalid-value = {key} ist ungueltig: {error}\n\
             [value-error]\nunknown-choice = unbekannter Wert {value}, erwartet {choices}\n",
        );
        let machine = LaunchError::Machine(ValueError::unknown_choice("X68030", MODEL_CHOICES));
        let text = in_catalog(&german, &machine).to_string();
        assert!(
            text.starts_with("machine ist ungueltig: unbekannter Wert X68030, erwartet "),
            "{text}"
        );
    }

    /// Verifies the English text of the failures of the machine.
    #[test]
    fn machine_failures_keep_the_english_error() {
        let panic = LaunchFailure::Panic("index out of bounds".to_owned());
        assert_eq!(
            panic.to_string(),
            "the machine panicked while starting: index out of bounds"
        );
        assert_eq!(in_catalog(english(), &panic).to_string(), panic.to_string());
        let german = test_catalog(
            "[errors]\nmachine-panicked = Die Maschine brach beim Start ab: {panic}\n",
        );
        assert_eq!(
            in_catalog(&german, &panic).to_string(),
            "Die Maschine brach beim Start ab: index out of bounds"
        );
        let machine = LaunchFailure::Machine("no ROM".to_owned());
        assert_eq!(in_catalog(&german, &machine).to_string(), "no ROM");
    }

    /// Verifies that a catalog writes an error with its own texts.
    #[test]
    fn a_catalog_writes_the_error_with_its_texts() {
        let german = test_catalog("[errors]\nmissing-image = Das Abbild {path} von {key} fehlt\n");
        let error = LaunchError::MissingImage {
            key: "fdd1",
            path: PathBuf::from("/a.d88"),
        };
        assert_eq!(
            in_catalog(&german, &error).to_string(),
            "Das Abbild /a.d88 von fdd1 fehlt"
        );
    }
}
