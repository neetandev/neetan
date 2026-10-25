//! A configuration document in its normalized form.
//!
//! The document holds every line of a `key = value` file: comments, blank
//! lines, commented-out keys and keys it does not know. Its text is the
//! normalized form of the file. Every active key line reads `key = value`
//! with no indentation, one space on each side of `=` and a trimmed value. A
//! comment, a blank line and a line without a key keep their text without
//! trailing whitespace. A commented-out key is a comment. The text keeps the
//! line order, the line ending style of the file, a missing final newline and
//! a UTF-8 byte order mark. The text of a normalized file is the file itself.

use std::{fmt, path::Path};

use super::atomic_file::write_atomically;

/// The characters that start a comment line.
pub const COMMENT_PREFIXES: [char; 2] = [';', '#'];
/// The UTF-8 byte order mark.
const BYTE_ORDER_MARK: char = '\u{FEFF}';
/// The separator of a normalized key line.
const SEPARATOR: &str = " = ";

/// The content of one line of a configuration file.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum LineContent<'a> {
    /// A line that holds only whitespace.
    Blank,
    /// A line whose first visible character is `;` or `#`.
    Comment,
    /// An active `key = value` line. The key and the value are trimmed.
    Entry {
        /// The key before the first `=`.
        key: &'a str,
        /// The value after the first `=`.
        value: &'a str,
    },
    /// A line without a key: no `=` or nothing before it.
    Text,
}

/// Classifies one line of a configuration file.
///
/// The line is trimmed. A blank line and a comment hold no entry. The first `=`
/// splits an entry into its key and its value, and both are trimmed.
#[must_use]
pub fn parse_line(text: &str) -> LineContent<'_> {
    let line = text.trim();
    if line.is_empty() {
        return LineContent::Blank;
    }
    if line.starts_with(COMMENT_PREFIXES) {
        return LineContent::Comment;
    }
    match line.split_once('=') {
        Some((key, value)) if !key.trim().is_empty() => LineContent::Entry {
            key: key.trim(),
            value: value.trim(),
        },
        Some(_) | None => LineContent::Text,
    }
}

/// One active `key = value` line of a document.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct DocumentEntry<'a> {
    /// The line number, starting at 1.
    pub line: usize,
    /// The trimmed key.
    pub key: &'a str,
    /// The trimmed value.
    pub value: &'a str,
}

/// An error of reading a configuration document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentError {
    /// The file cannot be read. The text is the error of the operating system in English.
    Read(String),
    /// The file holds bytes that are not UTF-8.
    InvalidUtf8 {
        /// The line of the first invalid byte, starting at 1.
        line: usize,
    },
}

impl DocumentError {
    /// Returns the error of a file that the operating system cannot read with `error`.
    #[must_use]
    pub fn read(error: &std::io::Error) -> Self {
        Self::Read(error.to_string())
    }

    /// Returns the line of the error, when the error has one.
    #[must_use]
    pub fn line(&self) -> Option<usize> {
        match self {
            DocumentError::Read(_) => None,
            DocumentError::InvalidUtf8 { line } => Some(*line),
        }
    }

    /// Returns the English message of the error without its line.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            DocumentError::Read(error) => format!("cannot read the file: {error}"),
            DocumentError::InvalidUtf8 { .. } => "the file is not valid UTF-8".to_owned(),
        }
    }
}

impl fmt::Display for DocumentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line() {
            Some(line) => write!(formatter, "line {line}: {}", self.message()),
            None => formatter.write_str(&self.message()),
        }
    }
}

impl std::error::Error for DocumentError {}

/// The line ending style of a file.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum LineEnding {
    /// A line feed.
    Lf,
    /// A carriage return and a line feed.
    CrLf,
}

impl LineEnding {
    /// Returns the bytes of the line ending.
    const fn text(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::CrLf => "\r\n",
        }
    }
}

/// A configuration file as an ordered list of lines.
///
/// `get`, `get_all`, `set`, `set_all` and `remove` read and edit the active
/// lines of a key. A comment, a commented-out key such as `;fdd1 = a.d88`, a
/// blank line and a line without a key stay in their place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigDocument {
    /// The text of each line without its line ending, as the file holds it.
    lines: Vec<String>,
    /// The line ending style: the ending of the first line of the file that has one.
    line_ending: LineEnding,
    /// Whether the last line ends with a line ending.
    final_newline: bool,
    /// Whether the file starts with a UTF-8 byte order mark.
    byte_order_mark: bool,
}

impl Default for ConfigDocument {
    fn default() -> Self {
        Self::new()
    }
}

impl ConfigDocument {
    /// Returns an empty document with line feeds and a final newline.
    #[must_use]
    pub fn new() -> Self {
        Self {
            lines: Vec::new(),
            line_ending: LineEnding::Lf,
            final_newline: true,
            byte_order_mark: false,
        }
    }

    /// Reads a document from the text of a file.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let (byte_order_mark, text) = match text.strip_prefix(BYTE_ORDER_MARK) {
            Some(rest) => (true, rest),
            None => (false, text),
        };
        let mut document = Self {
            byte_order_mark,
            ..Self::new()
        };
        let mut first_ending = None;
        for piece in text.split_inclusive('\n') {
            let (line, ending) = if let Some(line) = piece.strip_suffix("\r\n") {
                (line, Some(LineEnding::CrLf))
            } else if let Some(line) = piece.strip_suffix('\n') {
                (line, Some(LineEnding::Lf))
            } else {
                (piece, None)
            };
            first_ending = first_ending.or(ending);
            document.final_newline = ending.is_some();
            document.lines.push(line.to_owned());
        }
        document.line_ending = first_ending.unwrap_or(LineEnding::Lf);
        document
    }

    /// Reads a document from the bytes of a file.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, DocumentError> {
        match std::str::from_utf8(bytes) {
            Ok(text) => Ok(Self::parse(text)),
            Err(error) => {
                let valid = &bytes[..error.valid_up_to()];
                let line = valid.iter().filter(|byte| **byte == b'\n').count() + 1;
                Err(DocumentError::InvalidUtf8 { line })
            }
        }
    }

    /// Reads the document of the file at `path`.
    pub fn load(path: &Path) -> Result<Self, DocumentError> {
        let bytes = std::fs::read(path).map_err(|error| DocumentError::read(&error))?;
        Self::from_bytes(&bytes)
    }

    /// Returns the normalized text of the document.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut text = String::new();
        if self.byte_order_mark {
            text.push(BYTE_ORDER_MARK);
        }
        let last = self.lines.len().saturating_sub(1);
        for (index, line) in self.lines.iter().enumerate() {
            text.push_str(&normalized_line(line));
            if index < last || self.final_newline {
                text.push_str(self.line_ending.text());
            }
        }
        text
    }

    /// Writes the normalized text of the document to `path` in one atomic step.
    ///
    /// A symbolic link at `path` is followed to its real target. An existing
    /// target keeps its permissions. The folder of the target is created when
    /// it is absent. Returns the metadata of the written file.
    pub fn save(&self, path: &Path) -> std::io::Result<std::fs::Metadata> {
        write_atomically(path, self.to_text().as_bytes())
    }

    /// Returns the text of every line as the file holds it, with its line number starting at 1.
    pub fn lines(&self) -> impl Iterator<Item = (usize, &str)> {
        self.lines
            .iter()
            .enumerate()
            .map(|(index, line)| (index + 1, line.as_str()))
    }

    /// Returns every active `key = value` line in file order.
    pub fn entries(&self) -> impl DoubleEndedIterator<Item = DocumentEntry<'_>> {
        self.lines
            .iter()
            .enumerate()
            .filter_map(|(index, line)| match parse_line(line) {
                LineContent::Entry { key, value } => Some(DocumentEntry {
                    line: index + 1,
                    key,
                    value,
                }),
                LineContent::Blank | LineContent::Comment | LineContent::Text => None,
            })
    }

    /// Returns the value of the last active line of `key`: the value that applies at a launch.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries()
            .rfind(|entry| entry.key == key)
            .map(|entry| entry.value)
    }

    /// Returns the values of every active line of `key` in file order.
    #[must_use]
    pub fn get_all(&self, key: &str) -> Vec<&str> {
        self.entries()
            .filter(|entry| entry.key == key)
            .map(|entry| entry.value)
            .collect()
    }

    /// Returns whether the document has an active line of `key`.
    #[must_use]
    pub fn contains(&self, key: &str) -> bool {
        self.entries().any(|entry| entry.key == key)
    }

    /// Sets the single value of `key`.
    ///
    /// The first active line of the key takes the value in place. Every later
    /// active line of the key goes away. Without a line the key joins the
    /// document after its last line that is not blank.
    pub fn set(&mut self, key: &str, value: &str) {
        self.set_all(key, &[value]);
    }

    /// Sets the single value of `key`, a member of the key group `prefix`.
    ///
    /// An existing key updates as `set` does. A new key joins the document
    /// after the last active line whose key starts with `prefix`. Without such
    /// a line it goes before the first active line, else after the last line
    /// that is not blank.
    pub fn set_in_group(&mut self, key: &str, value: &str, prefix: &str) {
        let position = self.group_position(prefix);
        self.set_values(key, &[value], position);
    }

    /// Sets the values of the repeated key `key`.
    ///
    /// The active lines of the key form its group. The first values replace
    /// the values of the group lines in file order. Surplus group lines at the
    /// end of the group go away. Surplus values follow the last line of the
    /// group. The lines between the group lines stay where they are. Without a
    /// group the values join the document after its last line that is not
    /// blank. An empty list removes the group. A line break in a value becomes
    /// a space.
    pub fn set_all<V: AsRef<str>>(&mut self, key: &str, values: &[V]) {
        let position = self.append_position();
        self.set_values(key, values, position);
    }

    /// Removes every active line of `key`.
    pub fn remove(&mut self, key: &str) {
        let positions = self.key_positions(key);
        self.remove_positions(&positions);
    }

    /// Sets the values of `key`. Without a group the values go before the line at `position`.
    fn set_values<V: AsRef<str>>(&mut self, key: &str, values: &[V], position: usize) {
        let positions = self.key_positions(key);
        let lines = values.iter().map(|value| entry_text(key, value.as_ref()));
        let Some(&last_position) = positions.last() else {
            self.lines.splice(position..position, lines);
            return;
        };
        let mut lines = lines.collect::<Vec<String>>().into_iter();
        for &group_position in &positions {
            match lines.next() {
                Some(line) => self.lines[group_position] = line,
                None => break,
            }
        }
        let surplus = &positions[values.len().min(positions.len())..];
        let insert_at = last_position + 1;
        self.lines.splice(insert_at..insert_at, lines);
        self.remove_positions(surplus);
    }

    /// Returns the position of a new key of the group `prefix`.
    fn group_position(&self, prefix: &str) -> usize {
        let first = self.entries().next().map(|entry| entry.line - 1);
        let last_member = self
            .entries()
            .rfind(|entry| entry.key.starts_with(prefix))
            .map(|entry| entry.line);
        last_member
            .or(first)
            .unwrap_or_else(|| self.append_position())
    }

    /// Returns the indices of the active lines of `key`.
    fn key_positions(&self, key: &str) -> Vec<usize> {
        self.entries()
            .filter(|entry| entry.key == key)
            .map(|entry| entry.line - 1)
            .collect()
    }

    /// Removes the lines at the ascending indices `positions`.
    fn remove_positions(&mut self, positions: &[usize]) {
        for &position in positions.iter().rev() {
            self.lines.remove(position);
        }
    }

    /// Returns the index after the last line that is not blank.
    fn append_position(&self) -> usize {
        self.lines
            .iter()
            .rposition(|line| parse_line(line) != LineContent::Blank)
            .map_or(0, |index| index + 1)
    }
}

/// Returns the normalized form of one line.
///
/// An active key line becomes `key = value`. Every other line loses its trailing whitespace.
fn normalized_line(line: &str) -> String {
    match parse_line(line) {
        LineContent::Entry { key, value } => entry_text(key, value),
        LineContent::Blank | LineContent::Comment | LineContent::Text => line.trim_end().to_owned(),
    }
}

/// Builds the normalized text of a key line.
///
/// A line break in the value becomes a space, and the value is trimmed. An
/// empty value gives `key =`.
fn entry_text(key: &str, value: &str) -> String {
    let value = value.replace(['\r', '\n'], " ");
    let value = value.trim();
    if value.is_empty() {
        format!("{key}{}", SEPARATOR.trim_end())
    } else {
        format!("{key}{SEPARATOR}{value}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies that the text of a normalized file is the file itself.
    #[test]
    fn normalized_documents_reproduce_their_text() {
        for text in [
            "",
            "\n",
            "machine = PC9801RA\n",
            "machine = PC9801RA",
            "machine = PC9801RA\r\nfdd1 = a.d88\r\n",
            "machine = PC9801RA\r\nfdd1 = a.d88",
            "\u{FEFF}machine = PC9801RA\n",
            "hdd1 =\n",
            "; コメント\n\n  ;fdd1=x.d88\n# note\nfree text\n\n\n",
        ] {
            let document = ConfigDocument::parse(text);
            assert_eq!(document.to_text(), text, "{text:?}");
            assert_eq!(ConfigDocument::parse(&document.to_text()), document);
        }
    }

    /// Verifies the normalized form of each kind of line.
    #[test]
    fn documents_normalize_every_line() {
        let text = "\u{FEFF}  machine=PC9821AP  \r\n; 26K の BGM   \r\n  ;fdd1=old.d88 \n\
                    \t\r\nforce-gdc-clock   =2.5\r\nhdd1=\r\nfree text  \r\nmeta.notes= a = b ";
        let document = ConfigDocument::parse(text);
        let normalized = document.to_text();
        assert_eq!(
            normalized,
            "\u{FEFF}machine = PC9821AP\r\n; 26K の BGM\r\n  ;fdd1=old.d88\r\n\r\n\
             force-gdc-clock = 2.5\r\nhdd1 =\r\nfree text\r\nmeta.notes = a = b"
        );
        let again = ConfigDocument::parse(&normalized);
        assert_eq!(again.to_text(), normalized);
        let entries = |document: &ConfigDocument| -> Vec<(String, String)> {
            document
                .entries()
                .map(|entry| (entry.key.to_owned(), entry.value.to_owned()))
                .collect()
        };
        assert_eq!(entries(&document), entries(&again));
    }

    /// Verifies the line classification and the value trimming.
    #[test]
    fn lines_classify_like_the_layer_reader() {
        assert_eq!(parse_line("   "), LineContent::Blank);
        assert_eq!(parse_line(" ;fdd1=a.d88"), LineContent::Comment);
        assert_eq!(parse_line("# x = y"), LineContent::Comment);
        assert_eq!(parse_line("no key here"), LineContent::Text);
        assert_eq!(parse_line(" = value"), LineContent::Text);
        assert_eq!(
            parse_line("  fdd1 =  a b.d88  "),
            LineContent::Entry {
                key: "fdd1",
                value: "a b.d88"
            }
        );
        assert_eq!(
            parse_line("meta.notes=a = b"),
            LineContent::Entry {
                key: "meta.notes",
                value: "a = b"
            }
        );
        assert_eq!(
            parse_line("hdd1 ="),
            LineContent::Entry {
                key: "hdd1",
                value: ""
            }
        );
    }

    /// Verifies that a byte order mark stays out of the first key.
    #[test]
    fn byte_order_mark_stays_out_of_the_first_key() {
        let document = ConfigDocument::parse("\u{FEFF}machine=X1\n");
        assert_eq!(document.get("machine"), Some("X1"));
    }

    /// Verifies the line of an invalid UTF-8 byte.
    #[test]
    fn invalid_utf8_reports_its_line() {
        let error = ConfigDocument::from_bytes(b"a=1\nb=2\nc=\xff\n").unwrap_err();
        assert_eq!(error.line(), Some(3));
        assert_eq!(error.to_string(), "line 3: the file is not valid UTF-8");
    }

    /// Verifies that `get` reads the last active line and `get_all` every active line.
    #[test]
    fn get_reads_only_active_lines() {
        let document = ConfigDocument::parse(
            ";fdd1=old.d88\nfdd1=a.d88\nmachine = X1\nfdd1 = b.d88\nmachine=X1TURBO\n",
        );
        assert_eq!(document.get("machine"), Some("X1TURBO"));
        assert_eq!(document.get_all("fdd1"), vec!["a.d88", "b.d88"]);
        assert_eq!(document.get("fdd2"), None);
        assert!(document.get_all("fdd2").is_empty());
    }

    /// Verifies that `set` updates the first line in place and drops later duplicates.
    #[test]
    fn set_updates_the_first_line_and_drops_duplicates() {
        let mut document = ConfigDocument::parse(
            "; head\nmachine=PC9801VM\n;machine=PC9801F\nsoundboard = 26k\nmachine = PC9801VX\n",
        );
        document.set("machine", "PC9801RA");
        assert_eq!(
            document.to_text(),
            "; head\nmachine = PC9801RA\n;machine=PC9801F\nsoundboard = 26k\n"
        );
        document.set("soundboard", "86");
        assert_eq!(
            document.to_text(),
            "; head\nmachine = PC9801RA\n;machine=PC9801F\nsoundboard = 86\n"
        );
    }

    /// Verifies that `set` appends after the last line that is not blank.
    #[test]
    fn set_appends_before_trailing_blank_lines() {
        let mut document = ConfigDocument::parse("machine=PC9801RA\n; end\n\n\n");
        document.set("bios", "on");
        assert_eq!(
            document.to_text(),
            "machine = PC9801RA\n; end\nbios = on\n\n\n"
        );
    }

    /// Verifies that a line break in a value becomes a space and an empty value has no trailing space.
    #[test]
    fn values_normalize() {
        let mut document = ConfigDocument::new();
        document.set("meta.notes", "first\nsecond\r\n");
        document.set("hdd1", "  ");
        assert_eq!(document.to_text(), "meta.notes = first second\nhdd1 =\n");
    }

    /// Verifies the placement rule of `set_all`.
    #[test]
    fn set_all_updates_the_group_in_place() {
        let text =
            "machine=PC9801VM\nfdd1=a.d88\n; disk 2 is patched\nfdd1=b.d88\nsoundboard=26k\n";

        let mut document = ConfigDocument::parse(text);
        document.set_all("fdd1", &["b.d88", "a.d88"]);
        assert_eq!(
            document.to_text(),
            "machine = PC9801VM\nfdd1 = b.d88\n; disk 2 is patched\nfdd1 = a.d88\nsoundboard = 26k\n"
        );

        let mut document = ConfigDocument::parse(text);
        document.set_all("fdd1", &["a.d88", "b.d88", "c.d88", "d.d88"]);
        assert_eq!(
            document.to_text(),
            "machine = PC9801VM\nfdd1 = a.d88\n; disk 2 is patched\nfdd1 = b.d88\nfdd1 = c.d88\n\
             fdd1 = d.d88\nsoundboard = 26k\n"
        );

        let mut document = ConfigDocument::parse(text);
        document.set_all("fdd1", &["z.d88"]);
        assert_eq!(
            document.to_text(),
            "machine = PC9801VM\nfdd1 = z.d88\n; disk 2 is patched\nsoundboard = 26k\n"
        );

        let mut document = ConfigDocument::parse(text);
        document.set_all::<&str>("fdd1", &[]);
        assert_eq!(
            document.to_text(),
            "machine = PC9801VM\n; disk 2 is patched\nsoundboard = 26k\n"
        );

        let mut document = ConfigDocument::parse("machine=PC9801VM\n");
        document.set_all("fdd2", &["x.d88", "y.d88"]);
        assert_eq!(
            document.to_text(),
            "machine = PC9801VM\nfdd2 = x.d88\nfdd2 = y.d88\n"
        );
    }

    /// Verifies that `remove` deletes every active line and keeps commented-out keys.
    #[test]
    fn remove_deletes_every_active_line() {
        let mut document =
            ConfigDocument::parse(";fdd1=a.d88\nfdd1=b.d88\n#fdd1=c.d88\nfdd1 = d.d88\nbios=on\n");
        document.remove("fdd1");
        assert_eq!(document.to_text(), ";fdd1=a.d88\n#fdd1=c.d88\nbios = on\n");
        document.remove("cdrom");
        assert_eq!(document.to_text(), ";fdd1=a.d88\n#fdd1=c.d88\nbios = on\n");
    }

    /// Verifies that edits keep CRLF endings, a byte order mark and a missing final newline.
    #[test]
    fn edits_keep_the_file_format() {
        let mut document = ConfigDocument::parse("\u{FEFF}machine=X1\r\nfdd1=a.d88");
        document.set("fdd1", "b.d88");
        document.set("bios", "on");
        assert_eq!(
            document.to_text(),
            "\u{FEFF}machine = X1\r\nfdd1 = b.d88\r\nbios = on"
        );
        document.remove("bios");
        assert_eq!(document.to_text(), "\u{FEFF}machine = X1\r\nfdd1 = b.d88");
        document.remove("fdd1");
        document.remove("machine");
        assert_eq!(document.to_text(), "\u{FEFF}");
    }

    /// Verifies the placement of a new key of a group.
    #[test]
    fn group_keys_join_their_group() {
        let mut document = ConfigDocument::parse(
            "; header\nmeta.title=Edge\nmachine=PC9821AP\nmeta.year=1993\nfdd1=a\n",
        );
        document.set_in_group("meta.genre", "Shooter", "meta.");
        document.set_in_group("meta.title", "Edge 2", "meta.");
        assert_eq!(
            document.to_text(),
            "; header\nmeta.title = Edge 2\nmachine = PC9821AP\nmeta.year = 1993\n\
             meta.genre = Shooter\nfdd1 = a\n"
        );

        let mut document = ConfigDocument::parse("; header\n\nmachine = X1\n");
        document.set_in_group("meta.title", "Hydlide", "meta.");
        assert_eq!(
            document.to_text(),
            "; header\n\nmeta.title = Hydlide\nmachine = X1\n"
        );

        let mut document = ConfigDocument::parse("; only a comment\n\n");
        document.set_in_group("meta.title", "Hydlide", "meta.");
        assert_eq!(
            document.to_text(),
            "; only a comment\nmeta.title = Hydlide\n\n"
        );
    }
}
