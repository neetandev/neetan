//! The reading of a configuration file: the classification of its lines
//! against the settings table and the findings about the lines that a layer
//! ignores.

use std::{collections::HashMap, fmt, path::Path};

use machine_factory::config::{
    EmulatorConfig, MachineSelection, Target,
    settings::{MACHINE_KEY, Scope, Setting, ValueKind, find_setting},
    systems::{System, compiled_systems, system_of},
};

use super::{
    discovery::resolve_value,
    document::{ConfigDocument, LineContent, parse_line},
    metadata::META_PREFIX,
};

/// Prefix of the key binding lines.
pub(crate) const KEY_BINDING_PREFIX: &str = "key.";

/// Origin of a layer.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) enum LayerKind {
    /// The settings file.
    SettingsFile,
    /// The system defaults `<config-folder>/<system>.conf`.
    SystemDefaults,
    /// A game file from `--config`.
    GameFile,
    /// The command line flags.
    CommandLine,
}

impl LayerKind {
    /// Returns whether a layer of this kind sets keys of `scope`.
    ///
    /// Host keys belong to the settings file and the command line. Media keys
    /// belong to the games.
    pub(crate) fn accepts(self, scope: Scope) -> bool {
        match (self, scope) {
            (LayerKind::SettingsFile | LayerKind::CommandLine, _) => true,
            (LayerKind::SystemDefaults, Scope::System) => true,
            (LayerKind::SystemDefaults, Scope::Host | Scope::Game) => false,
            (LayerKind::GameFile, Scope::System | Scope::Game) => true,
            (LayerKind::GameFile, Scope::Host) => false,
        }
    }
}

/// One line of a layer.
#[derive(Debug, Clone)]
pub(crate) enum LayerLine {
    /// A key of the settings table with its value. A path value is absolute.
    Setting(&'static Setting, String),
    /// A `key.<host> = <native>` binding with the host and the native key name.
    KeyBinding(String, String),
}

/// The lines of one configuration source.
#[derive(Debug, Clone)]
pub(crate) struct Layer {
    /// The origin of the layer.
    pub(crate) kind: LayerKind,
    /// The name of the source in warnings: a file path or the command line.
    pub(crate) source: String,
    /// The lines in source order.
    pub(crate) lines: Vec<LayerLine>,
}

impl Layer {
    /// Builds a layer of `kind` from the lines of a file reading.
    pub(crate) fn from_reading(kind: LayerKind, source: String, reading: &FileReading) -> Self {
        Self {
            kind,
            source,
            lines: reading.lines.iter().map(|(_, line)| line.clone()).collect(),
        }
    }

    /// Builds the layer of the command line flags.
    pub(crate) fn command_line(flags: &[(&'static Setting, String)]) -> Self {
        Self {
            kind: LayerKind::CommandLine,
            source: "command line".to_owned(),
            lines: flags
                .iter()
                .map(|(setting, value)| LayerLine::Setting(setting, value.clone()))
                .collect(),
        }
    }

    /// Returns the last value of `key` in this layer.
    pub(crate) fn last_value(&self, key: &str) -> Option<&str> {
        self.lines.iter().rev().find_map(|line| match line {
            LayerLine::Setting(setting, value) if setting.key == key => Some(value.as_str()),
            LayerLine::Setting(..) | LayerLine::KeyBinding(..) => None,
        })
    }
}

/// Checks a value of `setting` against the settings table.
///
/// The value applies to a scratch configuration of a compiled system that
/// reads the setting. A setting that no compiled system reads checks against
/// the first compiled system.
pub fn check_value(setting: &Setting, value: &str) -> Result<(), String> {
    let system = compiled_systems()
        .find(|system| setting.families.intersects(system.family))
        .or_else(|| compiled_systems().next())
        .ok_or_else(|| "no system is compiled".to_owned())?;
    let model = MachineSelection::of_system(system)
        .ok_or_else(|| format!("the system {} is not compiled", system.name))?;
    (setting.apply)(&mut EmulatorConfig::for_model(model), value)
}

/// How much a diagnostic affects a configuration file.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Severity {
    /// The line is ignored. The file stays usable.
    Warning,
    /// The file cannot start its game.
    Error,
}

/// A finding about one line or about a whole configuration file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// The line of the finding, starting at 1. `None` for the whole file.
    pub line: Option<usize>,
    /// How much the finding affects the file.
    pub severity: Severity,
    /// The message without the line.
    pub message: String,
}

impl Diagnostic {
    /// Returns a warning about the line `line`.
    #[must_use]
    pub fn warning(line: usize, message: String) -> Self {
        Self {
            line: Some(line),
            severity: Severity::Warning,
            message,
        }
    }

    /// Returns an error about the line `line`.
    #[must_use]
    pub fn error(line: usize, message: String) -> Self {
        Self {
            line: Some(line),
            severity: Severity::Error,
            message,
        }
    }

    /// Returns the diagnostic with the file name `source` in front.
    #[must_use]
    pub fn located(&self, source: &str) -> String {
        match self.line {
            Some(_) => format!("{source} {self}"),
            None => format!("{source}: {self}"),
        }
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(formatter, "line {line}: {}", self.message),
            None => formatter.write_str(&self.message),
        }
    }
}

/// The lines of one configuration file that a layer reads, with the findings about the file.
#[derive(Debug, Clone)]
pub(crate) struct FileReading {
    /// The accepted lines with their line numbers, in file order.
    pub lines: Vec<(usize, LayerLine)>,
    /// The findings about the lines that the layer ignores or overrides.
    pub diagnostics: Vec<Diagnostic>,
}

/// Returns the message of a `machine` value that belongs to another system.
pub(crate) fn foreign_machine_message(value: &str) -> String {
    format!("machine {value} belongs to another system")
}

/// Returns the message of a key binding that the family does not know.
pub(crate) fn invalid_key_binding_message(host_name: &str, native_name: &str) -> String {
    format!("invalid key binding {KEY_BINDING_PREFIX}{host_name} = {native_name}")
}

/// Reads the lines of `document` that a layer of `kind` accepts.
///
/// Relative path values resolve against `root`. The `meta.*` keys leave the
/// layer without a finding. An unknown key, a key of
/// another scope, an invalid value and a line without a key leave the layer
/// with a warning. An unknown machine leaves it with an error in a game file
/// and with a warning elsewhere. A repeated single key stays in the layer with
/// a warning, and its last line wins.
pub(crate) fn read_document(
    kind: LayerKind,
    document: &ConfigDocument,
    root: &Path,
) -> FileReading {
    let mut lines = Vec::new();
    let mut diagnostics = Vec::new();
    let mut previous_lines: HashMap<&str, usize> = HashMap::new();
    for (line, text) in document.lines() {
        let (key, value) = match parse_line(text) {
            LineContent::Blank | LineContent::Comment => continue,
            LineContent::Text => {
                diagnostics.push(Diagnostic::warning(line, "text without a key".to_owned()));
                continue;
            }
            LineContent::Entry { key, value } => (key, value),
        };
        if key.starts_with(META_PREFIX) {
            continue;
        }
        if let Some(host_name) = key.strip_prefix(KEY_BINDING_PREFIX) {
            lines.push((
                line,
                LayerLine::KeyBinding(host_name.to_owned(), value.to_owned()),
            ));
            continue;
        }
        let Some(setting) = find_setting(key) else {
            diagnostics.push(Diagnostic::warning(line, format!("unknown key {key}")));
            continue;
        };
        if !kind.accepts(setting.scope) {
            diagnostics.push(Diagnostic::warning(line, out_of_scope_message(setting)));
            continue;
        }
        if setting.key == MACHINE_KEY && value.parse::<MachineSelection>().is_err() {
            let message = format!("unknown machine {value}");
            diagnostics.push(match kind {
                LayerKind::GameFile => Diagnostic::error(line, message),
                LayerKind::SettingsFile | LayerKind::SystemDefaults | LayerKind::CommandLine => {
                    Diagnostic::warning(line, message)
                }
            });
            continue;
        }
        let value = resolve_value(setting, root, value);
        if let Err(message) = check_value(setting, &value) {
            diagnostics.push(Diagnostic::warning(
                line,
                format!("invalid {key}: {message}"),
            ));
            continue;
        }
        if setting.kind != ValueKind::PathList
            && let Some(previous) = previous_lines.insert(setting.key, line)
        {
            diagnostics.push(Diagnostic::warning(
                line,
                format!("duplicate key {key}. It overrides line {previous}."),
            ));
        }
        lines.push((line, LayerLine::Setting(setting, value)));
    }
    FileReading { lines, diagnostics }
}

/// Reads a game file and checks its machine against the system of its folder.
///
/// A game file in the folder of a system that sets a machine of another family
/// gets an error on its last `machine` line.
pub(crate) fn read_game_document(
    document: &ConfigDocument,
    root: &Path,
    folder_system: Option<&'static System>,
) -> FileReading {
    let mut reading = read_document(LayerKind::GameFile, document, root);
    let last_machine = reading
        .lines
        .iter()
        .rev()
        .find_map(|(line, content)| match content {
            LayerLine::Setting(setting, value) if setting.key == MACHINE_KEY => {
                Some((*line, value))
            }
            LayerLine::Setting(..) | LayerLine::KeyBinding(..) => None,
        });
    if let (Some(system), Some((line, value))) = (folder_system, last_machine)
        && value
            .parse::<MachineSelection>()
            .is_ok_and(|model| system_of(model.target()) != system)
    {
        let diagnostic = Diagnostic::error(line, foreign_machine_message(value));
        reading.diagnostics.push(diagnostic);
        reading
            .diagnostics
            .sort_by_key(|diagnostic| diagnostic.line);
    }
    reading
}

/// Returns whether the machine `value` selects another family than `target`.
pub(crate) fn is_foreign_machine(value: &str, target: Target) -> bool {
    !value
        .parse::<MachineSelection>()
        .is_ok_and(|model| model.target() == target)
}

/// Returns the message of a line whose scope the layer does not set.
fn out_of_scope_message(setting: &Setting) -> String {
    match setting.scope {
        Scope::Host => format!(
            "host key {} is ignored. Only the settings file and the command line set host keys.",
            setting.key
        ),
        Scope::Game => format!(
            "media key {} is ignored. Media keys belong to the game files.",
            setting.key
        ),
        Scope::System => format!("key {} is ignored.", setting.key),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Returns the findings of `text` read as a layer of `kind`.
    fn findings(kind: LayerKind, text: &str) -> Vec<String> {
        read_document(kind, &ConfigDocument::parse(text), Path::new("/root"))
            .diagnostics
            .iter()
            .map(|diagnostic| format!("{:?} {diagnostic}", diagnostic.severity))
            .collect()
    }

    /// Verifies the findings of a game file and the lines it keeps.
    #[test]
    fn game_files_keep_valid_lines_and_report_the_rest() {
        let text = "; comment\nmeta.title = Edge\nmeta.unknown = x\nfdd3 = a.d88\n\
                    backend = legacy\nfree text\ncrt = maybe\nfdd1 = a.d88\nfdd1 = b.d88\n\
                    crt = on\ncrt = off\nkey.F1 = Stop\n";
        assert_eq!(
            findings(LayerKind::GameFile, text),
            vec![
                "Warning line 4: unknown key fdd3",
                "Warning line 5: host key backend is ignored. Only the settings file and the \
                 command line set host keys.",
                "Warning line 6: text without a key",
                "Warning line 7: invalid crt: invalid value 'maybe', expected on or off",
                "Warning line 11: duplicate key crt. It overrides line 10.",
            ]
        );
        let reading = read_document(
            LayerKind::GameFile,
            &ConfigDocument::parse(text),
            Path::new("/root"),
        );
        let lines: Vec<(usize, String)> = reading
            .lines
            .iter()
            .map(|(line, content)| {
                let text = match content {
                    LayerLine::Setting(setting, value) => format!("{} {value}", setting.key),
                    LayerLine::KeyBinding(host, native) => format!("key {host} {native}"),
                };
                (*line, text)
            })
            .collect();
        assert_eq!(
            lines,
            vec![
                (8, "fdd1 /root/a.d88".to_owned()),
                (9, "fdd1 /root/b.d88".to_owned()),
                (10, "crt on".to_owned()),
                (11, "crt off".to_owned()),
                (12, "key F1 Stop".to_owned()),
            ]
        );
    }

    /// Verifies the scope findings of the system defaults and the severity of an unknown machine.
    #[test]
    fn kinds_decide_scope_and_severity() {
        assert_eq!(
            findings(
                LayerKind::SystemDefaults,
                "fdd1 = a.d88\nmachine = X68030\n"
            ),
            vec![
                "Warning line 1: media key fdd1 is ignored. Media keys belong to the game files.",
                "Warning line 2: unknown machine X68030",
            ]
        );
        assert_eq!(
            findings(
                LayerKind::SettingsFile,
                "backend = legacy\nmachine = X68030\n"
            ),
            vec!["Warning line 2: unknown machine X68030"]
        );
        assert_eq!(
            findings(LayerKind::GameFile, "machine = X68030\n"),
            vec!["Error line 1: unknown machine X68030"]
        );
    }

    /// Verifies that a game file in a system folder must select a machine of that family.
    #[cfg(all(feature = "pc98", feature = "x68k"))]
    #[test]
    fn game_files_name_a_machine_of_their_folder() {
        use machine_factory::config::systems::{PC98_SYSTEM, X68K_SYSTEM};

        let document = ConfigDocument::parse("machine = PC9801RA\nmachine = X68000\n");
        let root = Path::new("/root");
        let reading = read_game_document(&document, root, Some(&PC98_SYSTEM));
        assert_eq!(
            reading
                .diagnostics
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            vec![
                "line 2: duplicate key machine. It overrides line 1.",
                "line 2: machine X68000 belongs to another system",
            ]
        );
        let reading = read_game_document(&document, root, Some(&X68K_SYSTEM));
        assert_eq!(reading.diagnostics.len(), 1);
        let reading = read_game_document(&document, root, None);
        assert_eq!(reading.diagnostics.len(), 1);
    }

    /// Verifies the text of a diagnostic with and without a line.
    #[test]
    fn diagnostics_name_their_line_and_source() {
        let warning = Diagnostic::warning(3, "unknown key fdd3".to_owned());
        assert_eq!(warning.to_string(), "line 3: unknown key fdd3");
        assert_eq!(
            warning.located("pc98/edge.conf"),
            "pc98/edge.conf line 3: unknown key fdd3"
        );
        let whole = Diagnostic {
            line: None,
            severity: Severity::Error,
            message: "cannot read the file".to_owned(),
        };
        assert_eq!(
            whole.located("pc98/edge.conf"),
            "pc98/edge.conf: cannot read the file"
        );
    }
}
