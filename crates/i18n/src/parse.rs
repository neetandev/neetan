//! The parser of the language file format.
//!
//! A line is a section header `[section]`, a pair `key = value`, a comment that starts with `;`, or
//! empty. Section names and keys use ASCII letters, digits, `-`, `.`, `+`, `:` and `_`. The
//! section `[language]` holds the header with the `name` and the `plural` rule of the language.
//! The parser walks the source once and allocates nothing. Values are slices of the source.
//! [`read_header`] reads only the header of a file.

use core::fmt;

use crate::plural::{PluralCategory, PluralRule};

mod header;

pub use header::{Header, read_header};

/// The section that holds the header of a language file.
pub const HEADER_SECTION: &str = "language";

/// The header key with the name of the language in the language itself.
pub const NAME_KEY: &str = "name";

/// The header key with the plural rule of the language.
pub const PLURAL_KEY: &str = "plural";

/// The UTF-8 byte order mark that the parser skips at the start of a file.
const BYTE_ORDER_MARK: char = '\u{FEFF}';

/// A full key: the section and the key inside the section.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FullKey<'a> {
    /// The section name.
    pub section: &'a str,
    /// The key name inside the section.
    pub key: &'a str,
}

impl fmt::Display for FullKey<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}", self.section, self.key)
    }
}

/// A fault of a language file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fault<'a> {
    /// The line of the fault, starting at 1. A fault of a key that the file lacks has no line.
    pub line: Option<u32>,
    /// The key of the fault.
    pub key: Option<FullKey<'a>>,
    /// The kind of the fault.
    pub kind: FaultKind<'a>,
}

impl fmt::Display for Fault<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(line) = self.line {
            write!(formatter, "line {line}: ")?;
        }
        if let Some(key) = self.key {
            write!(formatter, "{key}: ")?;
        }
        write!(formatter, "{}", self.kind)
    }
}

/// The kind of a fault of a language file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultKind<'a> {
    /// A line of an unknown form, such as `[a` or text without `=`.
    UnknownLine,
    /// A pair comes before the first section header.
    PairOutsideSection,
    /// A section name or a key holds a character outside of its character set, or an empty part
    /// between dots.
    BadName(&'a str),
    /// A backslash starts no known escape.
    BadEscape,
    /// A `{` has no closing `}`.
    UnclosedPlaceholder,
    /// A placeholder name holds a character outside of `a-z`, `0-9` and `-`.
    BadPlaceholderName(&'a str),
    /// A single `}` outside a placeholder.
    UnmatchedBrace,
    /// The `plural` header holds an unknown rule.
    BadPluralRule(&'a str),
    /// The key appears a second time.
    DuplicateKey,
    /// The schema of the catalog has no such key.
    UnknownKey,
    /// The text uses a placeholder that the schema of the key lacks.
    UnknownPlaceholder(&'a str),
    /// A text of a translation lacks a placeholder of the schema of its key.
    MissingPlaceholder(&'a str),
    /// The file lacks the key.
    MissingKey,
    /// A plural text lacks the form of a category of its plural rule.
    MissingPluralCategory(PluralCategory),
    /// A plural text holds the form of a category that its plural rule lacks.
    CategoryOutsideRule(PluralCategory),
    /// The file exceeds the size that a catalog can index.
    TooLarge,
    /// A name gives a Rust keyword as its identifier.
    KeywordIdentifier(&'a str),
    /// A name gives no valid Rust identifier.
    BadIdentifier(&'a str),
    /// Two names give the same Rust identifier.
    IdentifierCollision(&'a str, &'a str),
    /// The schema exceeds the number of keys that a `Key` can index.
    TooManyKeys,
    /// A list pattern lacks `{first}` or `{second}`, repeats one of them, holds another placeholder
    /// or puts `{second}` first.
    BadListPattern,
    /// A translation or the language name of the header holds an empty value.
    EmptyValue,
    /// A text has more placeholders than a generated function takes as parameters.
    TooManyPlaceholders,
}

impl fmt::Display for FaultKind<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FaultKind::UnknownLine => write!(formatter, "unknown line"),
            FaultKind::PairOutsideSection => write!(formatter, "pair before the first section"),
            FaultKind::BadName(name) => write!(formatter, "bad name `{name}`"),
            FaultKind::BadEscape => write!(formatter, "bad escape"),
            FaultKind::UnclosedPlaceholder => write!(formatter, "unclosed placeholder"),
            FaultKind::BadPlaceholderName(name) => {
                write!(formatter, "bad placeholder name `{name}`")
            }
            FaultKind::UnmatchedBrace => write!(formatter, "unmatched `}}`"),
            FaultKind::BadPluralRule(rule) => write!(formatter, "unknown plural rule `{rule}`"),
            FaultKind::DuplicateKey => write!(formatter, "duplicate key"),
            FaultKind::UnknownKey => write!(formatter, "unknown key"),
            FaultKind::UnknownPlaceholder(name) => {
                write!(formatter, "unknown placeholder `{{{name}}}`")
            }
            FaultKind::MissingPlaceholder(name) => {
                write!(formatter, "missing placeholder `{{{name}}}`")
            }
            FaultKind::MissingKey => write!(formatter, "missing key"),
            FaultKind::MissingPluralCategory(category) => {
                write!(formatter, "missing plural form `{}`", category.name())
            }
            FaultKind::CategoryOutsideRule(category) => write!(
                formatter,
                "the plural rule has no form `{}`",
                category.name()
            ),
            FaultKind::TooLarge => write!(formatter, "the file is too large"),
            FaultKind::KeywordIdentifier(name) => {
                write!(formatter, "the name `{name}` gives a Rust keyword")
            }
            FaultKind::BadIdentifier(name) => {
                write!(formatter, "the name `{name}` gives no Rust identifier")
            }
            FaultKind::IdentifierCollision(first, second) => write!(
                formatter,
                "the names `{first}` and `{second}` give the same Rust identifier"
            ),
            FaultKind::TooManyKeys => write!(formatter, "the file holds too many keys"),
            FaultKind::BadListPattern => write!(
                formatter,
                "the list pattern lacks {{first}} and {{second}} once each in this order"
            ),
            FaultKind::EmptyValue => write!(formatter, "empty value"),
            FaultKind::TooManyPlaceholders => {
                write!(formatter, "the text has too many placeholders")
            }
        }
    }
}

/// The value of a pair: a slice of the source with its escapes, braces and placeholders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Value<'a> {
    /// The value as it appears in the source, trimmed.
    raw: &'a str,
}

impl<'a> Value<'a> {
    /// Creates a value from its source text.
    pub fn new(raw: &'a str) -> Value<'a> {
        Value { raw }
    }

    /// Returns the value as it appears in the source.
    pub fn raw(&self) -> &'a str {
        self.raw
    }

    /// Returns true when the placeholders of the value are the given names, each once and in the
    /// given order. A faulty value gives false.
    pub fn has_exact_placeholders(&self, names: &[&str]) -> bool {
        let mut expected = names.iter();
        for token in self.tokens() {
            match token {
                Ok(Token::Placeholder(name)) => {
                    if expected.next() != Some(&name) {
                        return false;
                    }
                }
                Ok(Token::Text(_) | Token::Character(_)) => {}
                Err(_) => return false,
            }
        }
        expected.next().is_none()
    }

    /// Returns the tokens of the value.
    pub fn tokens(&self) -> Tokens<'a> {
        Tokens { rest: self.raw }
    }

    /// Checks every escape, brace and placeholder of the value.
    pub fn check(&self) -> Result<(), FaultKind<'a>> {
        self.tokens().try_for_each(|token| token.map(|_| ()))
    }
}

/// A token of a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token<'a> {
    /// Literal text without escapes and braces.
    Text(&'a str),
    /// A character from an escape or from a doubled brace.
    Character(char),
    /// A placeholder with its name.
    Placeholder(&'a str),
}

/// The iterator over the tokens of a value. It ends after the first fault.
#[derive(Clone, Debug)]
pub struct Tokens<'a> {
    /// The text that follows the last token.
    rest: &'a str,
}

impl<'a> Iterator for Tokens<'a> {
    type Item = Result<Token<'a>, FaultKind<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        let bytes = self.rest.as_bytes();
        let first = *bytes.first()?;
        let second = bytes.get(1).copied();
        let (token, consumed) = match first {
            b'\\' => match second {
                Some(b'n') => (Token::Character('\n'), 2),
                Some(b'\\') => (Token::Character('\\'), 2),
                Some(b';') => (Token::Character(';'), 2),
                _ => return Some(self.fail(FaultKind::BadEscape)),
            },
            b'{' if second == Some(b'{') => (Token::Character('{'), 2),
            b'{' => {
                let Some(length) = self.rest[1..].find('}') else {
                    return Some(self.fail(FaultKind::UnclosedPlaceholder));
                };
                let name = &self.rest[1..1 + length];
                if !is_placeholder_name(name) {
                    return Some(self.fail(FaultKind::BadPlaceholderName(name)));
                }
                (Token::Placeholder(name), length + 2)
            }
            b'}' if second == Some(b'}') => (Token::Character('}'), 2),
            b'}' => return Some(self.fail(FaultKind::UnmatchedBrace)),
            _ => {
                let length = self.rest.find(['\\', '{', '}']).unwrap_or(self.rest.len());
                (Token::Text(&self.rest[..length]), length)
            }
        };
        self.rest = &self.rest[consumed..];
        Some(Ok(token))
    }
}

impl<'a> Tokens<'a> {
    /// Ends the iteration with a fault.
    fn fail(&mut self, kind: FaultKind<'a>) -> Result<Token<'a>, FaultKind<'a>> {
        self.rest = "";
        Err(kind)
    }
}

/// An event of the parser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event<'a> {
    /// A section header.
    Section {
        /// The line of the header.
        line: u32,
        /// The section name.
        name: &'a str,
    },
    /// The `name` header: the name of the language in the language itself.
    Name {
        /// The line of the pair.
        line: u32,
        /// The name. It holds no placeholder.
        value: Value<'a>,
    },
    /// The `plural` header: the plural rule of the language.
    Plural {
        /// The line of the pair.
        line: u32,
        /// The plural rule.
        rule: PluralRule,
    },
    /// A pair outside of the header. Its value passed the check of [`Value::check`].
    Pair {
        /// The line of the pair.
        line: u32,
        /// The section and the key of the pair.
        key: FullKey<'a>,
        /// The value of the pair.
        value: Value<'a>,
    },
    /// A fault of a line. A fault of a pair holds its key.
    Fault(Fault<'a>),
}

/// The section state of the parser.
#[derive(Clone, Copy, Debug)]
enum SectionState<'a> {
    /// No section header came yet.
    Outside,
    /// The last section header had a bad name. Its pairs are skipped.
    Skipped,
    /// The pairs belong to this section.
    Inside(&'a str),
}

/// The parser of a language file. It yields one [`Event`] for each section header, pair or faulty
/// line. The parser skips the pairs of a section with a bad name.
///
/// The line numbers are `u32` values. The parser serves sources below 4 GiB. A larger source can
/// overflow the line count and panic.
#[derive(Clone, Debug)]
pub struct Parser<'a> {
    /// The source after the last line.
    rest: &'a str,
    /// The number of the last line.
    line: u32,
    /// The current section.
    section: SectionState<'a>,
    /// True after the first `name` header.
    name_seen: bool,
    /// True after the first `plural` header.
    plural_seen: bool,
}

impl<'a> Parser<'a> {
    /// Creates a parser for a source. A byte order mark at the start is skipped.
    pub fn new(source: &'a str) -> Parser<'a> {
        Parser {
            rest: source.strip_prefix(BYTE_ORDER_MARK).unwrap_or(source),
            line: 0,
            section: SectionState::Outside,
            name_seen: false,
            plural_seen: false,
        }
    }

    /// Parses one trimmed line.
    fn parse_line(&mut self, text: &'a str) -> Option<Event<'a>> {
        let line = self.line;
        if text.is_empty() || text.starts_with(';') {
            return None;
        }
        if let Some(inner) = text.strip_prefix('[') {
            let Some(name) = inner.strip_suffix(']') else {
                return Some(self.fault(None, FaultKind::UnknownLine));
            };
            if !is_name(name) {
                self.section = SectionState::Skipped;
                return Some(self.fault(None, FaultKind::BadName(name)));
            }
            self.section = SectionState::Inside(name);
            return Some(Event::Section { line, name });
        }
        let Some((key, value)) = text.split_once('=') else {
            return Some(self.fault(None, FaultKind::UnknownLine));
        };
        let key = key.trim_ascii();
        let value = Value::new(value.trim_ascii());
        let section = match self.section {
            SectionState::Outside => {
                return Some(self.fault(None, FaultKind::PairOutsideSection));
            }
            SectionState::Skipped => return None,
            SectionState::Inside(section) => section,
        };
        if !is_name(key) {
            return Some(self.fault(None, FaultKind::BadName(key)));
        }
        let full_key = FullKey { section, key };
        if section == HEADER_SECTION {
            return Some(self.parse_header(full_key, value));
        }
        Some(match value.check() {
            Ok(()) => Event::Pair {
                line,
                key: full_key,
                value,
            },
            Err(kind) => self.fault(Some(full_key), kind),
        })
    }

    /// Parses a pair of the header section.
    fn parse_header(&mut self, key: FullKey<'a>, value: Value<'a>) -> Event<'a> {
        let line = self.line;
        let seen = match key.key {
            NAME_KEY => &mut self.name_seen,
            PLURAL_KEY => &mut self.plural_seen,
            _ => return self.fault(Some(key), FaultKind::UnknownKey),
        };
        if *seen {
            return self.fault(Some(key), FaultKind::DuplicateKey);
        }
        *seen = true;
        if key.key == PLURAL_KEY {
            return match PluralRule::from_name(value.raw()) {
                Some(rule) => Event::Plural { line, rule },
                None => self.fault(Some(key), FaultKind::BadPluralRule(value.raw())),
            };
        }
        if value.raw().is_empty() {
            return self.fault(Some(key), FaultKind::EmptyValue);
        }
        for token in value.tokens() {
            match token {
                Ok(Token::Placeholder(name)) => {
                    return self.fault(Some(key), FaultKind::UnknownPlaceholder(name));
                }
                Ok(Token::Text(_) | Token::Character(_)) => {}
                Err(kind) => return self.fault(Some(key), kind),
            }
        }
        Event::Name { line, value }
    }

    /// Adds a [`FaultKind::MissingKey`] fault for each header key without a pair in the lines
    /// that the parser has read.
    pub(crate) fn push_missing_header_keys(&self, faults: &mut Vec<Fault<'a>>) {
        for (seen, key) in [(self.name_seen, NAME_KEY), (self.plural_seen, PLURAL_KEY)] {
            if !seen {
                faults.push(Fault {
                    line: None,
                    key: Some(FullKey {
                        section: HEADER_SECTION,
                        key,
                    }),
                    kind: FaultKind::MissingKey,
                });
            }
        }
    }

    /// Returns a fault event for the current line.
    fn fault(&self, key: Option<FullKey<'a>>, kind: FaultKind<'a>) -> Event<'a> {
        Event::Fault(Fault {
            line: Some(self.line),
            key,
            kind,
        })
    }
}

impl<'a> Iterator for Parser<'a> {
    type Item = Event<'a>;

    fn next(&mut self) -> Option<Event<'a>> {
        while !self.rest.is_empty() {
            let (text, rest) = self.rest.split_once('\n').unwrap_or((self.rest, ""));
            self.rest = rest;
            self.line += 1;
            if let Some(event) = self.parse_line(text.trim_ascii()) {
                return Some(event);
            }
        }
        None
    }
}

/// Checks that a size in bytes fits the `u32` line numbers and offsets. A larger size gives a
/// [`FaultKind::TooLarge`] fault.
pub(crate) fn check_size<'a>(size: usize) -> Result<(), Vec<Fault<'a>>> {
    if u32::try_from(size).is_err() {
        return Err(vec![Fault {
            line: None,
            key: None,
            kind: FaultKind::TooLarge,
        }]);
    }
    Ok(())
}

/// Returns true for a valid section name or key: parts of ASCII letters, digits, `-`, `+`, `:` and
/// `_`, joined by `.`.
pub fn is_name(name: &str) -> bool {
    !name.is_empty()
        && name.split('.').all(|part| {
            !part.is_empty()
                && part.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'+' | b':' | b'_')
                })
        })
}

/// Returns true for a valid placeholder name of `a-z`, `0-9` and `-`.
pub fn is_placeholder_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_size_beyond_the_u32_range_is_too_large() {
        assert_eq!(check_size(u32::MAX as usize), Ok(()));
        if let Ok(size) = usize::try_from(u64::from(u32::MAX) + 1) {
            assert_eq!(
                check_size(size),
                Err(vec![Fault {
                    line: None,
                    key: None,
                    kind: FaultKind::TooLarge,
                }])
            );
        }
    }

    /// Returns every event of a source.
    fn events(source: &str) -> Vec<Event<'_>> {
        Parser::new(source).collect()
    }

    /// Returns the fault event of a line.
    fn fault(line: u32, key: Option<FullKey<'static>>, kind: FaultKind<'static>) -> Event<'static> {
        Event::Fault(Fault {
            line: Some(line),
            key,
            kind,
        })
    }

    /// Returns a full key.
    fn full_key(section: &'static str, key: &'static str) -> FullKey<'static> {
        FullKey { section, key }
    }

    /// Returns every token of a value.
    fn tokens(raw: &str) -> Vec<Result<Token<'_>, FaultKind<'_>>> {
        Value::new(raw).tokens().collect()
    }

    #[test]
    fn every_line_form() {
        let source = "\u{FEFF}; comment\n\n[language]\nname = English\nplural = one-other\n\
                      [common]\r\n  cancel   =   Cancel  \r\nempty =\n; second comment\n";
        assert_eq!(
            events(source),
            vec![
                Event::Section {
                    line: 3,
                    name: "language"
                },
                Event::Name {
                    line: 4,
                    value: Value::new("English")
                },
                Event::Plural {
                    line: 5,
                    rule: PluralRule::OneOther
                },
                Event::Section {
                    line: 6,
                    name: "common"
                },
                Event::Pair {
                    line: 7,
                    key: full_key("common", "cancel"),
                    value: Value::new("Cancel")
                },
                Event::Pair {
                    line: 8,
                    key: full_key("common", "empty"),
                    value: Value::new("")
                },
            ]
        );
    }

    #[test]
    fn a_value_keeps_further_equal_signs_and_semicolons() {
        assert_eq!(
            events("[a]\nb = c = d ; e"),
            vec![
                Event::Section { line: 1, name: "a" },
                Event::Pair {
                    line: 2,
                    key: full_key("a", "b"),
                    value: Value::new("c = d ; e")
                },
            ]
        );
    }

    #[test]
    fn dotted_names() {
        assert_eq!(
            events("[setting.cpu-mode]\nimages.one = x"),
            vec![
                Event::Section {
                    line: 1,
                    name: "setting.cpu-mode"
                },
                Event::Pair {
                    line: 2,
                    key: full_key("setting.cpu-mode", "images.one"),
                    value: Value::new("x")
                },
            ]
        );
    }

    #[test]
    fn choice_values_are_keys() {
        assert_eq!(
            events("[choice.aspect]\n4:3 = Four to three\nA = Drive A\n86+26k = Board\none = One"),
            vec![
                Event::Section {
                    line: 1,
                    name: "choice.aspect"
                },
                Event::Pair {
                    line: 2,
                    key: full_key("choice.aspect", "4:3"),
                    value: Value::new("Four to three")
                },
                Event::Pair {
                    line: 3,
                    key: full_key("choice.aspect", "A"),
                    value: Value::new("Drive A")
                },
                Event::Pair {
                    line: 4,
                    key: full_key("choice.aspect", "86+26k"),
                    value: Value::new("Board")
                },
                Event::Pair {
                    line: 5,
                    key: full_key("choice.aspect", "one"),
                    value: Value::new("One")
                },
            ]
        );
    }

    #[test]
    fn every_escape() {
        assert_eq!(
            tokens(r"a\nb\\c\;d{{e}}f"),
            vec![
                Ok(Token::Text("a")),
                Ok(Token::Character('\n')),
                Ok(Token::Text("b")),
                Ok(Token::Character('\\')),
                Ok(Token::Text("c")),
                Ok(Token::Character(';')),
                Ok(Token::Text("d")),
                Ok(Token::Character('{')),
                Ok(Token::Text("e")),
                Ok(Token::Character('}')),
                Ok(Token::Text("f")),
            ]
        );
    }

    #[test]
    fn placeholders() {
        assert_eq!(
            tokens("Delete {name}? {count}{first-2}"),
            vec![
                Ok(Token::Text("Delete ")),
                Ok(Token::Placeholder("name")),
                Ok(Token::Text("? ")),
                Ok(Token::Placeholder("count")),
                Ok(Token::Placeholder("first-2")),
            ]
        );
        assert_eq!(
            tokens("{name}を削除しますか？"),
            vec![
                Ok(Token::Placeholder("name")),
                Ok(Token::Text("を削除しますか？")),
            ]
        );
    }

    #[test]
    fn exact_placeholders() {
        let names = ["first", "second"];
        assert!(Value::new("{first} and {second}").has_exact_placeholders(&names));
        assert!(Value::new("{first}{second}").has_exact_placeholders(&names));
        assert!(!Value::new("{second} and {first}").has_exact_placeholders(&names));
        assert!(!Value::new("{first}{second}{second}").has_exact_placeholders(&names));
        assert!(!Value::new("{first}").has_exact_placeholders(&names));
        assert!(!Value::new("{first}{third}{second}").has_exact_placeholders(&names));
        assert!(!Value::new("{first}{second} }").has_exact_placeholders(&names));
        assert!(Value::new("plain").has_exact_placeholders(&[]));
    }

    #[test]
    fn value_faults() {
        assert_eq!(
            tokens(r"a\tb"),
            vec![Ok(Token::Text("a")), Err(FaultKind::BadEscape)]
        );
        assert_eq!(
            tokens("a\\"),
            vec![Ok(Token::Text("a")), Err(FaultKind::BadEscape)]
        );
        assert_eq!(
            tokens("a {name"),
            vec![Ok(Token::Text("a ")), Err(FaultKind::UnclosedPlaceholder)]
        );
        assert_eq!(
            tokens("{Name}"),
            vec![Err(FaultKind::BadPlaceholderName("Name"))]
        );
        assert_eq!(tokens("{}"), vec![Err(FaultKind::BadPlaceholderName(""))]);
        assert_eq!(
            tokens("{a.b}"),
            vec![Err(FaultKind::BadPlaceholderName("a.b"))]
        );
        assert_eq!(
            tokens("a } b"),
            vec![Ok(Token::Text("a ")), Err(FaultKind::UnmatchedBrace)]
        );
    }

    #[test]
    fn line_faults() {
        assert_eq!(
            events(
                "orphan = 1\n[a\n[b d]\nx = 1\n[b]\njust text\nk*y = 1\nc = {x\nd = \\q\ne = }\nf..g = 1"
            ),
            vec![
                fault(1, None, FaultKind::PairOutsideSection),
                fault(2, None, FaultKind::UnknownLine),
                fault(3, None, FaultKind::BadName("b d")),
                Event::Section { line: 5, name: "b" },
                fault(6, None, FaultKind::UnknownLine),
                fault(7, None, FaultKind::BadName("k*y")),
                fault(8, Some(full_key("b", "c")), FaultKind::UnclosedPlaceholder),
                fault(9, Some(full_key("b", "d")), FaultKind::BadEscape),
                fault(10, Some(full_key("b", "e")), FaultKind::UnmatchedBrace),
                fault(11, None, FaultKind::BadName("f..g")),
            ]
        );
    }

    #[test]
    fn header_faults() {
        assert_eq!(
            events(
                "[language]\nname = A\nname = B\nplural = many\nplural = none\nscript = latin\nname = {x}"
            ),
            vec![
                Event::Section {
                    line: 1,
                    name: "language"
                },
                Event::Name {
                    line: 2,
                    value: Value::new("A")
                },
                fault(
                    3,
                    Some(full_key("language", "name")),
                    FaultKind::DuplicateKey
                ),
                fault(
                    4,
                    Some(full_key("language", "plural")),
                    FaultKind::BadPluralRule("many")
                ),
                fault(
                    5,
                    Some(full_key("language", "plural")),
                    FaultKind::DuplicateKey
                ),
                fault(
                    6,
                    Some(full_key("language", "script")),
                    FaultKind::UnknownKey
                ),
                fault(
                    7,
                    Some(full_key("language", "name")),
                    FaultKind::DuplicateKey
                ),
            ]
        );
        assert_eq!(
            events("[language]\nname =   \nname = B"),
            vec![
                Event::Section {
                    line: 1,
                    name: "language"
                },
                fault(2, Some(full_key("language", "name")), FaultKind::EmptyValue),
                fault(
                    3,
                    Some(full_key("language", "name")),
                    FaultKind::DuplicateKey
                ),
            ]
        );
        assert_eq!(
            events("[language]\nname = {x}"),
            vec![
                Event::Section {
                    line: 1,
                    name: "language"
                },
                fault(
                    2,
                    Some(full_key("language", "name")),
                    FaultKind::UnknownPlaceholder("x")
                ),
            ]
        );
    }

    #[test]
    fn names() {
        assert!(is_name("setting.cpu-mode"));
        assert!(is_name("640k"));
        assert!(is_name("86+26k"));
        assert!(is_name("4:3"));
        assert!(is_name("A"));
        assert!(is_name("a_b"));
        assert!(!is_name(""));
        assert!(!is_name(".a"));
        assert!(!is_name("a."));
        assert!(!is_name("a b"));
        assert!(!is_name("a/b"));
        assert!(!is_name("a=b"));
        assert!(is_placeholder_name("first-name"));
        assert!(!is_placeholder_name("first.name"));
    }

    #[test]
    fn faults_display_their_line_and_key() {
        let fault = Fault {
            line: Some(12),
            key: Some(full_key("manager", "delete-question")),
            kind: FaultKind::UnknownPlaceholder("nam"),
        };
        assert_eq!(
            fault.to_string(),
            "line 12: manager.delete-question: unknown placeholder `{nam}`"
        );
        let fault = Fault {
            line: None,
            key: Some(full_key("common", "save")),
            kind: FaultKind::MissingKey,
        };
        assert_eq!(fault.to_string(), "common.save: missing key");
    }
}
