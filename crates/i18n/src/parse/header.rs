//! The header of a language file: the name and the plural rule of the language and the further
//! language tags that the file serves.

use std::{borrow::Cow, str::SplitAsciiWhitespace};

use super::{Event, Fault, HEADER_SECTION, Parser, Token, Value, check_size};
use crate::plural::PluralRule;

/// The header of a language file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header<'a> {
    /// The name of the language in the language itself with its escapes resolved.
    pub name: Cow<'a, str>,
    /// The plural rule of the language.
    pub plural: PluralRule,
    /// The further language tags that the file serves, separated by ASCII whitespace. A header
    /// without `covers` has an empty text.
    pub covers: &'a str,
}

impl<'a> Header<'a> {
    /// Returns the further language tags that the file serves in the order of the header.
    pub fn covered_tags(&self) -> SplitAsciiWhitespace<'a> {
        self.covers.split_ascii_whitespace()
    }
}

/// Reads the header of a language file without building a catalog.
///
/// The read fails with the faults of the header when the header lacks the name or the plural rule
/// of the language, when the name is empty, or when the `covers` list is empty or holds a word
/// that is no language tag. A source of 4 GiB or more fails with
/// [`FaultKind::TooLarge`](crate::FaultKind::TooLarge). A name without escapes and braces borrows
/// from the source and the read allocates nothing.
pub fn read_header(source: &str) -> Result<Header<'_>, Vec<Fault<'_>>> {
    check_size(source.len())?;
    let mut name = None;
    let mut plural = None;
    let mut covers = "";
    let mut faults = Vec::new();
    let mut parser = Parser::new(source);
    for event in parser.by_ref() {
        match event {
            Event::Name { value, .. } => name = Some(value),
            Event::Plural { rule, .. } => plural = Some(rule),
            Event::Covers { tags, .. } => covers = tags,
            Event::Fault(fault) if fault.key.is_some_and(|key| key.section == HEADER_SECTION) => {
                faults.push(fault);
            }
            Event::Section { .. } | Event::Pair { .. } | Event::Fault(_) => {}
        }
    }
    if let (Some(name), Some(plural), false) = (name, plural, parser.covers_rejected()) {
        return Ok(Header {
            name: unescape(name),
            plural,
            covers,
        });
    }
    parser.push_missing_header_keys(&mut faults);
    Err(faults)
}

/// Returns a checked value without placeholders with its escapes and doubled braces resolved.
fn unescape(value: Value<'_>) -> Cow<'_, str> {
    let raw = value.raw();
    if !raw.contains(['\\', '{', '}']) {
        return Cow::Borrowed(raw);
    }
    let mut text = String::with_capacity(raw.len());
    for token in value.tokens().flatten() {
        match token {
            Token::Text(part) => text.push_str(part),
            Token::Character(character) => text.push(character),
            Token::Placeholder(_) => {}
        }
    }
    Cow::Owned(text)
}

#[cfg(test)]
mod tests {
    use allocation_counter::count_allocations;

    use super::*;
    use crate::parse::{COVERS_KEY, FaultKind, FullKey, NAME_KEY, PLURAL_KEY};

    /// Returns the fault of a header key on a line.
    fn fault(line: Option<u32>, key: &'static str, kind: FaultKind<'static>) -> Fault<'static> {
        Fault {
            line,
            key: Some(FullKey {
                section: HEADER_SECTION,
                key,
            }),
            kind,
        }
    }

    #[test]
    fn a_header_without_escapes_borrows_its_name() {
        let source = "[common]\ncancel = Cancel\n[language]\nname = 日本語\nplural = none\n";
        let (allocations, header) = count_allocations(|| read_header(source));
        assert_eq!(allocations, 0);
        let header = header.unwrap();
        assert!(matches!(header.name, Cow::Borrowed("日本語")));
        assert_eq!(header.plural, PluralRule::None);
        assert_eq!(header.covered_tags().count(), 0);
    }

    #[test]
    fn the_covered_tags_follow_the_header() {
        let source = "[language]\nname = 繁體中文\nplural = none\ncovers = zh-HK  zh-MO zh-Hant\n";
        let (allocations, header) = count_allocations(|| read_header(source));
        assert_eq!(allocations, 0);
        let header = header.unwrap();
        assert_eq!(
            header.covered_tags().collect::<Vec<_>>(),
            ["zh-HK", "zh-MO", "zh-Hant"]
        );
    }

    #[test]
    fn a_bad_covers_list_fails_the_header() {
        assert_eq!(
            read_header("[language]\nname = 繁體中文\nplural = none\ncovers = zh-HK zh_MO\n")
                .unwrap_err(),
            vec![fault(
                Some(4),
                COVERS_KEY,
                FaultKind::BadLanguageTag("zh_MO")
            )]
        );
        assert_eq!(
            read_header("[language]\nname = 繁體中文\nplural = none\ncovers =\n").unwrap_err(),
            vec![fault(Some(4), COVERS_KEY, FaultKind::EmptyValue)]
        );
        let duplicate =
            "[language]\nname = 繁體中文\nplural = none\ncovers = zh-HK\ncovers = x_y\n";
        assert_eq!(
            read_header(duplicate)
                .unwrap()
                .covered_tags()
                .collect::<Vec<_>>(),
            ["zh-HK"]
        );
    }

    #[test]
    fn a_name_with_escapes_resolves_them() {
        let header =
            read_header("[language]\nname = A\\;B {{x}}\\\\\nplural = one-other\n").unwrap();
        assert_eq!(header.name, "A;B {x}\\");
        assert!(matches!(header.name, Cow::Owned(_)));
        assert_eq!(header.plural, PluralRule::OneOther);
    }

    #[test]
    fn faults_outside_the_header_and_extra_header_keys_keep_the_header() {
        let source = "[language]\nname = English\nplural = one-other\nscript = latin\n\
                      [common]\nsave = \\q\nbroken line\n";
        let header = read_header(source).unwrap();
        assert_eq!(header.name, "English");
    }

    #[test]
    fn a_header_without_a_name_or_a_rule_fails() {
        assert_eq!(
            read_header("[common]\ncancel = Cancel\n").unwrap_err(),
            vec![
                fault(None, NAME_KEY, FaultKind::MissingKey),
                fault(None, PLURAL_KEY, FaultKind::MissingKey),
            ]
        );
        assert_eq!(
            read_header("[language]\nname =\nplural = few\n[common]\nsave = \\q\n").unwrap_err(),
            vec![
                fault(Some(2), NAME_KEY, FaultKind::EmptyValue),
                fault(Some(3), PLURAL_KEY, FaultKind::BadPluralRule("few")),
            ]
        );
        assert_eq!(
            read_header("[language]\nname =\n").unwrap_err(),
            vec![
                fault(Some(2), NAME_KEY, FaultKind::EmptyValue),
                fault(None, PLURAL_KEY, FaultKind::MissingKey),
            ]
        );
    }
}
