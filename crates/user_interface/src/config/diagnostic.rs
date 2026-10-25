//! The findings about a configuration file: their kinds with their data, their English text and
//! their text in a catalog.

use std::fmt;

use gui::ui::TextData;
use i18n::Catalog;
use machine_factory::config::settings::value_error::ValueError;

use super::{document::DocumentError, reading::KEY_BINDING_PREFIX};
use crate::texts::{CatalogText, diagnostic, in_catalog};

/// How much a diagnostic affects a configuration file.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Severity {
    /// The line is ignored. The file stays usable.
    Warning,
    /// The file cannot start its game.
    Error,
}

/// The kind of a finding with its data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticKind {
    /// A line with text but without a key.
    TextWithoutKey,
    /// A key outside the settings table.
    UnknownKey(String),
    /// A host key in a layer that sets no host keys.
    HostKey(&'static str),
    /// A `machine` value that no compiled family knows.
    UnknownMachine(String),
    /// A value that its key rejects.
    InvalidValue {
        /// The key of the line.
        key: &'static str,
        /// The fault of the value.
        error: ValueError,
    },
    /// A single key that an earlier line sets.
    DuplicateKey {
        /// The key of the line.
        key: &'static str,
        /// The earlier line of the key.
        previous: usize,
    },
    /// A `machine` value of a game file of another system than the folder of the file.
    ForeignMachine(String),
    /// A `machine` value of the system defaults of another system. The launch ignores the line.
    ForeignMachineIgnored(String),
    /// A key binding that the family does not know.
    InvalidKeyBinding {
        /// The host key name after `key.`.
        host: String,
        /// The native key name of the value.
        native: String,
    },
    /// The file does not read.
    Document(DocumentError),
    /// The system defaults file at this shown path does not read. It fails every launch of the
    /// system.
    BrokenDefaults {
        /// The shown path of the system defaults file.
        path: String,
        /// The error of the read.
        error: DocumentError,
    },
}

/// A finding about one line or about a whole configuration file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// The line of the finding, starting at 1. `None` for the whole file.
    pub line: Option<usize>,
    /// How much the finding affects the file.
    pub severity: Severity,
    /// The kind of the finding with its data.
    pub kind: DiagnosticKind,
}

impl Diagnostic {
    /// Returns a warning about the line `line`.
    #[must_use]
    pub fn warning(line: usize, kind: DiagnosticKind) -> Self {
        Self {
            line: Some(line),
            severity: Severity::Warning,
            kind,
        }
    }

    /// Returns an error about the line `line`.
    #[must_use]
    pub fn error(line: usize, kind: DiagnosticKind) -> Self {
        Self {
            line: Some(line),
            severity: Severity::Error,
            kind,
        }
    }

    /// Returns the error of a file that does not read with `error`, at the line of the error.
    #[must_use]
    pub fn of_document(error: DocumentError) -> Self {
        Self {
            line: error.line(),
            severity: Severity::Error,
            kind: DiagnosticKind::Document(error),
        }
    }

    /// Returns the English diagnostic with the file name `source` in front.
    #[must_use]
    pub fn located(&self, source: &str) -> String {
        match self.line {
            Some(_) => format!("{source} {self}"),
            None => format!("{source}: {self}"),
        }
    }

    /// Returns the diagnostic with the file name `source` in front as a typed text.
    #[must_use]
    pub fn located_at<'a>(&'a self, source: &'a str) -> Located<'a, &'a DiagnosticKind> {
        Located {
            source,
            line: self.line,
            message: &self.kind,
        }
    }
}

impl TextData<Diagnostic> for &Diagnostic {
    /// Whether the diagnostic equals the stored diagnostic.
    fn matches(&self, stored: &Diagnostic) -> bool {
        *self == stored
    }

    /// Replaces the stored diagnostic with a copy of the diagnostic.
    fn store_into(&self, stored: &mut Diagnostic) {
        stored.clone_from(self);
    }

    /// Returns a copy of the diagnostic.
    fn to_stored(&self) -> Diagnostic {
        (*self).clone()
    }
}

impl fmt::Display for DiagnosticKind {
    /// Writes the English text of the finding without its line.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TextWithoutKey => formatter.write_str("text without a key"),
            Self::UnknownKey(key) => write!(formatter, "unknown key {key}"),
            Self::HostKey(key) => write!(
                formatter,
                "host key {key} is ignored. Only the settings file and the command line set host \
                 keys."
            ),
            Self::UnknownMachine(value) => write!(formatter, "unknown machine {value}"),
            Self::InvalidValue { key, error } => write!(formatter, "invalid {key}: {error}"),
            Self::DuplicateKey { key, previous } => write!(
                formatter,
                "duplicate key {key}. It overrides line {previous}."
            ),
            Self::ForeignMachine(value) => {
                write!(formatter, "machine {value} belongs to another system")
            }
            Self::ForeignMachineIgnored(value) => write!(
                formatter,
                "machine {value} belongs to another system. The line is ignored."
            ),
            Self::InvalidKeyBinding { host, native } => write!(
                formatter,
                "invalid key binding {KEY_BINDING_PREFIX}{host} = {native}"
            ),
            Self::Document(error) => formatter.write_str(&error.message()),
            Self::BrokenDefaults { path, error } => match error.line() {
                Some(_) => write!(formatter, "system defaults {path} {error}"),
                None => write!(formatter, "system defaults {path}: {error}"),
            },
        }
    }
}

impl fmt::Display for Diagnostic {
    /// Writes the English text of the finding after its line.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(formatter, "line {line}: {}", self.kind),
            None => write!(formatter, "{}", self.kind),
        }
    }
}

/// Writes the message of `error` without its line in `catalog`.
fn write_document_message(
    catalog: &Catalog,
    out: &mut dyn fmt::Write,
    error: &DocumentError,
) -> fmt::Result {
    match error {
        DocumentError::Read(error) => catalog.write(out, diagnostic::cannot_read(error)),
        DocumentError::InvalidUtf8 { .. } => out.write_str(catalog.text(diagnostic::INVALID_UTF8)),
    }
}

/// The message of a document error without its line, as a typed text.
#[derive(Clone, Copy)]
pub struct DocumentMessage<'a>(&'a DocumentError);

impl CatalogText for DocumentMessage<'_> {
    /// Writes the message without its line.
    fn write_text(&self, catalog: &Catalog, out: &mut dyn fmt::Write) -> fmt::Result {
        write_document_message(catalog, out, self.0)
    }
}

/// Writes `message` after the line `line` in `catalog`. A text without a line writes only the
/// message.
fn write_at_line(
    catalog: &Catalog,
    out: &mut dyn fmt::Write,
    line: Option<usize>,
    message: &dyn CatalogText,
) -> fmt::Result {
    let message = in_catalog(catalog, message);
    match line {
        Some(line) => catalog.write(out, diagnostic::line(&line, &message)),
        None => write!(out, "{message}"),
    }
}

impl CatalogText for DocumentError {
    /// Writes the error after its line through the keys of the section `[diagnostic]`.
    fn write_text(&self, catalog: &Catalog, out: &mut dyn fmt::Write) -> fmt::Result {
        write_at_line(catalog, out, self.line(), &DocumentMessage(self))
    }
}

/// A text with the file name of its source and its line in front.
#[derive(Clone, Copy)]
pub struct Located<'a, M> {
    /// The file name of the source.
    source: &'a str,
    /// The line of the text, starting at 1. `None` for the whole file.
    line: Option<usize>,
    /// The text.
    message: M,
}

impl<'a> Located<'a, DocumentMessage<'a>> {
    /// Returns the message of `error` with the file name `source` and the line of the error in
    /// front.
    #[must_use]
    pub fn document(source: &'a str, error: &'a DocumentError) -> Self {
        Self {
            source,
            line: error.line(),
            message: DocumentMessage(error),
        }
    }
}

impl<M: CatalogText> CatalogText for Located<'_, M> {
    /// Writes the text after its source and its line, for example `pc98/edge.conf line 3:
    /// unknown key fdd3`.
    fn write_text(&self, catalog: &Catalog, out: &mut dyn fmt::Write) -> fmt::Result {
        let message = in_catalog(catalog, &self.message);
        match self.line {
            Some(line) => {
                catalog.write(out, diagnostic::located_line(&self.source, &line, &message))
            }
            None => catalog.write(out, diagnostic::located(&self.source, &message)),
        }
    }
}

impl CatalogText for DiagnosticKind {
    /// Writes the finding without its line through the keys of the section `[diagnostic]`.
    fn write_text(&self, catalog: &Catalog, out: &mut dyn fmt::Write) -> fmt::Result {
        match self {
            Self::TextWithoutKey => out.write_str(catalog.text(diagnostic::TEXT_WITHOUT_KEY)),
            Self::UnknownKey(key) => catalog.write(out, diagnostic::unknown_key(key)),
            Self::HostKey(key) => catalog.write(out, diagnostic::host_key(key)),
            Self::UnknownMachine(value) => catalog.write(out, diagnostic::unknown_machine(value)),
            Self::InvalidValue { key, error } => {
                let error = in_catalog(catalog, error);
                catalog.write(out, diagnostic::invalid_value(key, &error))
            }
            Self::DuplicateKey { key, previous } => {
                catalog.write(out, diagnostic::duplicate_key(key, previous))
            }
            Self::ForeignMachine(value) => catalog.write(out, diagnostic::foreign_machine(value)),
            Self::ForeignMachineIgnored(value) => {
                catalog.write(out, diagnostic::foreign_machine_ignored(value))
            }
            Self::InvalidKeyBinding { host, native } => {
                catalog.write(out, diagnostic::invalid_key_binding(host, native))
            }
            Self::Document(error) => write_document_message(catalog, out, error),
            Self::BrokenDefaults { path, error } => {
                let file = Located::document(path, error);
                let file = in_catalog(catalog, &file);
                catalog.write(out, diagnostic::broken_defaults(&file))
            }
        }
    }
}

impl CatalogText for Diagnostic {
    /// Writes the finding after its line through the keys of the section `[diagnostic]`.
    fn write_text(&self, catalog: &Catalog, out: &mut dyn fmt::Write) -> fmt::Result {
        write_at_line(catalog, out, self.line, &self.kind)
    }
}

#[cfg(test)]
mod tests {
    use machine_factory::config::settings::{find_setting, value_error::ValueError};

    use super::*;
    use crate::texts::{english, test_catalog};

    /// Every kind of finding with its English text.
    fn kinds() -> Vec<(DiagnosticKind, &'static str)> {
        let crt = find_setting("crt").expect("crt is a table key");
        vec![
            (DiagnosticKind::TextWithoutKey, "text without a key"),
            (
                DiagnosticKind::UnknownKey("fdd3".to_owned()),
                "unknown key fdd3",
            ),
            (
                DiagnosticKind::HostKey("backend"),
                "host key backend is ignored. Only the settings file and the command line set \
                 host keys.",
            ),
            (
                DiagnosticKind::UnknownMachine("X68030".to_owned()),
                "unknown machine X68030",
            ),
            (
                DiagnosticKind::InvalidValue {
                    key: crt.key,
                    error: ValueError::OnOff {
                        value: "maybe".to_owned(),
                    },
                },
                "invalid crt: invalid value 'maybe', expected on or off",
            ),
            (
                DiagnosticKind::DuplicateKey {
                    key: crt.key,
                    previous: 10,
                },
                "duplicate key crt. It overrides line 10.",
            ),
            (
                DiagnosticKind::ForeignMachine("X68000".to_owned()),
                "machine X68000 belongs to another system",
            ),
            (
                DiagnosticKind::ForeignMachineIgnored("X68000".to_owned()),
                "machine X68000 belongs to another system. The line is ignored.",
            ),
            (
                DiagnosticKind::InvalidKeyBinding {
                    host: "F1".to_owned(),
                    native: "Nope".to_owned(),
                },
                "invalid key binding key.F1 = Nope",
            ),
            (
                DiagnosticKind::Document(DocumentError::Read("denied".to_owned())),
                "cannot read the file: denied",
            ),
            (
                DiagnosticKind::Document(DocumentError::InvalidUtf8 { line: 2 }),
                "the file is not valid UTF-8",
            ),
            (
                DiagnosticKind::BrokenDefaults {
                    path: "config/pc98.conf".to_owned(),
                    error: DocumentError::InvalidUtf8 { line: 3 },
                },
                "system defaults config/pc98.conf line 3: the file is not valid UTF-8",
            ),
            (
                DiagnosticKind::BrokenDefaults {
                    path: "config/pc98.conf".to_owned(),
                    error: DocumentError::Read("denied".to_owned()),
                },
                "system defaults config/pc98.conf: cannot read the file: denied",
            ),
        ]
    }

    /// Verifies the English text of every kind and that the English catalog writes the same
    /// text.
    #[test]
    fn every_kind_writes_its_english_text() {
        for (kind, text) in kinds() {
            assert_eq!(kind.to_string(), text);
            assert_eq!(in_catalog(english(), &kind).to_string(), text);
        }
    }

    /// Verifies the text of a diagnostic with and without a line, with its source and in a
    /// catalog.
    #[test]
    fn diagnostics_show_their_line_and_source() {
        let warning = Diagnostic::warning(3, DiagnosticKind::UnknownKey("fdd3".to_owned()));
        assert_eq!(warning.to_string(), "line 3: unknown key fdd3");
        assert_eq!(
            in_catalog(english(), &warning).to_string(),
            warning.to_string()
        );
        assert_eq!(
            warning.located("pc98/edge.conf"),
            "pc98/edge.conf line 3: unknown key fdd3"
        );
        assert_eq!(
            in_catalog(english(), &warning.located_at("pc98/edge.conf")).to_string(),
            "pc98/edge.conf line 3: unknown key fdd3"
        );
        let whole = Diagnostic::of_document(DocumentError::Read("denied".to_owned()));
        assert_eq!(whole.severity, Severity::Error);
        assert_eq!(
            whole.located("pc98/edge.conf"),
            "pc98/edge.conf: cannot read the file: denied"
        );
        assert_eq!(
            in_catalog(english(), &whole.located_at("pc98/edge.conf")).to_string(),
            whole.located("pc98/edge.conf")
        );
        let german = test_catalog(
            "[diagnostic]\nline = Zeile {line}: {message}\nunknown-key = {key} ist unbekannt\n\
             located-line = {path}, Zeile {line}: {message}\n",
        );
        assert_eq!(
            in_catalog(&german, &warning).to_string(),
            "Zeile 3: fdd3 ist unbekannt"
        );
        assert_eq!(
            in_catalog(&german, &warning.located_at("edge.conf")).to_string(),
            "edge.conf, Zeile 3: fdd3 ist unbekannt"
        );
    }

    /// Verifies that a document error writes its line in front of its message.
    #[test]
    fn document_errors_write_their_line() {
        let error = DocumentError::InvalidUtf8 { line: 4 };
        assert_eq!(error.to_string(), "line 4: the file is not valid UTF-8");
        assert_eq!(in_catalog(english(), &error).to_string(), error.to_string());
    }
}
