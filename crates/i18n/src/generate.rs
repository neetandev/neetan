//! The code generator. It turns a reference language file into Rust source.
//!
//! The generated file holds the `SCHEMA` static, one module for each section, one `Key` constant
//! for each fixed text and one function for each text with placeholders. A key name gives an
//! upper case constant and a lower case function with `_` for `-` and `.`. A section name gives a
//! lower case module the same way. The names of these generated sections use lower case letters,
//! digits, `-` and `.`. The sections `[list]` and `[setting.*]` get no module.

use std::{
    collections::{BTreeMap, btree_map},
    fmt::Write as _,
    fs,
    path::Path,
};

use crate::{
    catalog::COUNT_PLACEHOLDER,
    parse::{Event, Fault, FaultKind, FullKey, Parser, Token, Value, check_size},
    plural::{PluralCategory, split_plural_key},
    schema::{LIST_PATTERN_KEYS, LIST_PLACEHOLDERS, LIST_SECTION, MAXIMUM_KEY_COUNT},
};

/// The prefix of the sections with the texts of the settings.
const SETTING_SECTION_PREFIX: &str = "setting.";

/// The largest number of parameters of a generated function.
const MAXIMUM_PARAMETER_COUNT: usize = 7;

/// The shortest fence of a code block in a doc comment.
const MINIMUM_FENCE_LENGTH: usize = 3;

/// The strict, reserved and edition 2024 keywords of Rust.
const RUST_KEYWORDS: [&str; 52] = [
    "abstract", "as", "async", "await", "become", "box", "break", "const", "continue", "crate",
    "do", "dyn", "else", "enum", "extern", "false", "final", "fn", "for", "gen", "if", "impl",
    "in", "let", "loop", "macro", "match", "mod", "move", "mut", "override", "priv", "pub", "ref",
    "return", "self", "static", "struct", "super", "trait", "true", "try", "type", "typeof",
    "unsafe", "unsized", "use", "virtual", "where", "while", "yield", "union",
];

/// Generates the Rust code for a reference language file.
///
/// The generation fails with the faults of the file. A source of 4 GiB or more fails with
/// [`FaultKind::TooLarge`].
pub fn generate(source: &str) -> Result<String, Vec<Fault<'_>>> {
    let reference = Reference::read(source)?;
    Ok(write_code(&reference))
}

/// Generates the Rust code for the reference language file at one path and writes it to another
/// path. The output file keeps its modification time when its content stays the same.
///
/// The error holds one line for each fault, starting with the path and the line of the fault.
pub fn generate_file(reference: &Path, output: &Path) -> Result<(), String> {
    let source = fs::read_to_string(reference)
        .map_err(|error| format!("{}: {error}", reference.display()))?;
    let code = generate(&source).map_err(|faults| report(reference, &faults))?;
    if fs::read_to_string(output).ok().as_deref() != Some(code.as_str()) {
        fs::write(output, code).map_err(|error| format!("{}: {error}", output.display()))?;
    }
    Ok(())
}

/// Returns the faults of a reference file as a report with one line for each fault.
fn report(reference: &Path, faults: &[Fault<'_>]) -> String {
    let mut text = String::new();
    for fault in faults {
        let _ = writeln!(text, "{}: {fault}", reference.display());
    }
    text
}

/// The keys of a reference language file, sorted by section and key.
pub(crate) struct Reference<'a> {
    /// The keys sorted by section and key.
    pub(crate) keys: Vec<ReferenceKey<'a>>,
}

/// One key of a reference language file.
pub(crate) struct ReferenceKey<'a> {
    /// The section name.
    pub(crate) section: &'a str,
    /// The key inside the section. A plural text has the key without its category.
    pub(crate) key: &'a str,
    /// True for a plural text.
    pub(crate) plural: bool,
    /// The line of the first pair of the key.
    line: u32,
    /// The placeholder names in the order of their first use, without the `count` of a plural
    /// text.
    pub(crate) placeholders: Vec<&'a str>,
    /// The forms of the text with their values as they appear in the source.
    forms: Vec<(PluralCategory, &'a str)>,
}

impl ReferenceKey<'_> {
    /// Returns true when the key gets a function.
    fn is_function(&self) -> bool {
        self.plural || !self.placeholders.is_empty()
    }

    /// Returns the full key.
    fn full_key(&self) -> FullKey<'_> {
        FullKey {
            section: self.section,
            key: self.key,
        }
    }
}

impl<'a> Reference<'a> {
    /// Reads a reference language file and checks it.
    pub(crate) fn read(source: &'a str) -> Result<Reference<'a>, Vec<Fault<'a>>> {
        check_size(source.len())?;
        let mut faults = Vec::new();
        let mut keys: BTreeMap<(&'a str, &'a str), ReferenceKey<'a>> = BTreeMap::new();
        let mut section_lines: BTreeMap<&'a str, u32> = BTreeMap::new();
        let mut rule = None;
        let mut parser = Parser::new(source);
        for event in parser.by_ref() {
            match event {
                Event::Section { line, name } => {
                    section_lines.entry(name).or_insert(line);
                }
                Event::Name { .. } => {}
                Event::Plural {
                    rule: plural_rule, ..
                } => rule = Some(plural_rule),
                Event::Fault(fault) => faults.push(fault),
                Event::Pair { line, key, value } => {
                    let (base, category) = split_plural_key(key.key);
                    let plural = category.is_some();
                    let category = category.unwrap_or(PluralCategory::Other);
                    let entry = match keys.entry((key.section, base)) {
                        btree_map::Entry::Vacant(vacant) => vacant.insert(ReferenceKey {
                            section: key.section,
                            key: base,
                            plural,
                            line,
                            placeholders: Vec::new(),
                            forms: Vec::new(),
                        }),
                        btree_map::Entry::Occupied(occupied) => {
                            let existing = occupied.into_mut();
                            let duplicate = existing.plural != plural
                                || existing.forms.iter().any(|(form, _)| *form == category);
                            if duplicate {
                                faults.push(Fault {
                                    line: Some(line),
                                    key: Some(key),
                                    kind: FaultKind::DuplicateKey,
                                });
                                continue;
                            }
                            existing
                        }
                    };
                    entry.forms.push((category, value.raw()));
                    for token in value.tokens().flatten() {
                        if let Token::Placeholder(name) = token
                            && !(plural && name == COUNT_PLACEHOLDER)
                            && !entry.placeholders.contains(&name)
                        {
                            entry.placeholders.push(name);
                        }
                    }
                }
            }
        }
        let keys: Vec<ReferenceKey<'a>> = keys.into_values().collect();
        parser.push_missing_header_keys(&mut faults);
        if let Some(rule) = rule {
            for key in keys.iter().filter(|key| key.plural) {
                for (category, _) in &key.forms {
                    if !rule.has(*category) {
                        faults.push(key_fault(key, FaultKind::CategoryOutsideRule(*category)));
                    }
                }
                for category in rule.categories() {
                    if !key.forms.iter().any(|(form, _)| form == category) {
                        faults.push(key_fault(key, FaultKind::MissingPluralCategory(*category)));
                    }
                }
            }
        }
        check_list_patterns(&keys, &mut faults);
        check_identifiers(&keys, &section_lines, &mut faults);
        if keys.len() > MAXIMUM_KEY_COUNT {
            faults.push(Fault {
                line: None,
                key: None,
                kind: FaultKind::TooManyKeys,
            });
        }
        if faults.is_empty() {
            Ok(Reference { keys })
        } else {
            faults.sort_by_key(|fault| (fault.line.is_none(), fault.line));
            Err(faults)
        }
    }
}

/// Returns a fault of a key at the line of its first pair.
fn key_fault<'a>(key: &ReferenceKey<'a>, kind: FaultKind<'a>) -> Fault<'a> {
    Fault {
        line: Some(key.line),
        key: Some(FullKey {
            section: key.section,
            key: key.key,
        }),
        kind,
    }
}

/// Checks that every list pattern exists and holds `{first}` and `{second}` once each and in this
/// order.
fn check_list_patterns<'a>(keys: &[ReferenceKey<'a>], faults: &mut Vec<Fault<'a>>) {
    for pattern in LIST_PATTERN_KEYS {
        match keys
            .iter()
            .find(|key| key.section == LIST_SECTION && key.key == pattern)
        {
            None => faults.push(Fault {
                line: None,
                key: Some(FullKey {
                    section: LIST_SECTION,
                    key: pattern,
                }),
                kind: FaultKind::MissingKey,
            }),
            Some(key)
                if key.plural
                    || !key.forms.iter().all(|(_, raw)| {
                        Value::new(raw).has_exact_placeholders(&LIST_PLACEHOLDERS)
                    }) =>
            {
                faults.push(key_fault(key, FaultKind::BadListPattern));
            }
            Some(_) => {}
        }
    }
}

/// Returns true when a section gets a module.
fn is_generated_section(section: &str) -> bool {
    section != LIST_SECTION && !section.starts_with(SETTING_SECTION_PREFIX)
}

/// Returns true for a section name or a key of a generated section: lower case letters, digits,
/// `-` and `.`.
fn is_generated_name(name: &str) -> bool {
    name.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'.')
    })
}

/// Returns the identifier of a name: `_` for `-` and `.`, in upper or lower case.
fn identifier(name: &str, upper_case: bool) -> String {
    name.chars()
        .map(|character| match character {
            '-' | '.' => '_',
            character if upper_case => character.to_ascii_uppercase(),
            character => character,
        })
        .collect()
}

/// Checks the identifier of a name. A keyword gives a fault for a lower case identifier.
fn identifier_fault<'a>(name: &'a str, lower_case_use: bool) -> Option<FaultKind<'a>> {
    if !name.starts_with(|character: char| character.is_ascii_lowercase()) {
        return Some(FaultKind::BadIdentifier(name));
    }
    if lower_case_use && RUST_KEYWORDS.contains(&identifier(name, false).as_str()) {
        return Some(FaultKind::KeywordIdentifier(name));
    }
    None
}

/// Checks the names and the module, constant, function and parameter identifiers of the
/// generated sections. A fault of a section carries the line of its first header.
fn check_identifiers<'a>(
    keys: &[ReferenceKey<'a>],
    section_lines: &BTreeMap<&'a str, u32>,
    faults: &mut Vec<Fault<'a>>,
) {
    let mut modules: BTreeMap<String, &'a str> = BTreeMap::new();
    let mut items: BTreeMap<String, &'a str> = BTreeMap::new();
    let mut current_section = None;
    for key in keys.iter().filter(|key| is_generated_section(key.section)) {
        if current_section != Some(key.section) {
            current_section = Some(key.section);
            items.clear();
            let section_fault = |kind| Fault {
                line: section_lines.get(key.section).copied(),
                key: None,
                kind,
            };
            if !is_generated_name(key.section) {
                faults.push(section_fault(FaultKind::BadName(key.section)));
            } else {
                if let Some(kind) = identifier_fault(key.section, true) {
                    faults.push(section_fault(kind));
                }
                if let Some(other) = modules.insert(identifier(key.section, false), key.section) {
                    faults.push(section_fault(FaultKind::IdentifierCollision(
                        other,
                        key.section,
                    )));
                }
            }
        }
        if !is_generated_name(key.key) {
            faults.push(key_fault(key, FaultKind::BadName(key.key)));
        } else {
            if let Some(kind) = identifier_fault(key.key, key.is_function()) {
                faults.push(key_fault(key, kind));
            }
            if let Some(other) = items.insert(identifier(key.key, false), key.key) {
                faults.push(key_fault(
                    key,
                    FaultKind::IdentifierCollision(other, key.key),
                ));
            }
        }
        for placeholder in &key.placeholders {
            if let Some(kind) = identifier_fault(placeholder, true) {
                faults.push(key_fault(key, kind));
            }
        }
        if key.placeholders.len() + usize::from(key.plural) > MAXIMUM_PARAMETER_COUNT {
            faults.push(key_fault(key, FaultKind::TooManyPlaceholders));
        }
    }
}

/// Writes the Rust code of a checked reference file.
fn write_code(reference: &Reference<'_>) -> String {
    let mut code = String::new();
    let _ = writeln!(
        code,
        "/// The keys of the reference language file in the order of their `Key`."
    );
    let _ = writeln!(
        code,
        "static KEYS: [::i18n::KeyInfo; {}] = [",
        reference.keys.len()
    );
    for key in &reference.keys {
        let placeholders: Vec<String> = key
            .placeholders
            .iter()
            .map(|placeholder| format!("{placeholder:?}"))
            .collect();
        let _ = writeln!(
            code,
            "    ::i18n::KeyInfo::new({:?}, {:?}, &[{}], {}),",
            key.section,
            key.key,
            placeholders.join(", "),
            key.plural
        );
    }
    let _ = writeln!(code, "];");
    let _ = writeln!(code);
    let _ = writeln!(code, "/// The schema of the reference language file.");
    let _ = writeln!(
        code,
        "pub static SCHEMA: ::i18n::Schema = ::i18n::Schema::new(&KEYS);"
    );
    let mut current_section = None;
    for (index, key) in reference.keys.iter().enumerate() {
        if !is_generated_section(key.section) {
            continue;
        }
        if current_section != Some(key.section) {
            if current_section.is_some() {
                let _ = writeln!(code, "}}");
            }
            current_section = Some(key.section);
            let _ = writeln!(code);
            let _ = writeln!(code, "/// The texts of the section `[{}]`.", key.section);
            let _ = writeln!(code, "pub mod {} {{", identifier(key.section, false));
        } else {
            let _ = writeln!(code);
        }
        write_item(&mut code, index, key);
    }
    if current_section.is_some() {
        let _ = writeln!(code, "}}");
    }
    code
}

/// Writes the constant or the function of one key.
fn write_item(code: &mut String, index: usize, key: &ReferenceKey<'_>) {
    let kind = if key.plural { "plural text" } else { "text" };
    let _ = writeln!(code, "    /// The {kind} `{}`.", key.full_key());
    let _ = writeln!(code, "    ///");
    let lines: Vec<String> = key
        .forms
        .iter()
        .map(|(category, value)| {
            let value = doc_text(value);
            if key.plural {
                format!("{} = {value}", category.name())
            } else {
                value
            }
        })
        .collect();
    let fence = "`".repeat(fence_length(&lines));
    let _ = writeln!(code, "    /// {fence}text");
    for line in &lines {
        let _ = writeln!(code, "    /// {line}");
    }
    let _ = writeln!(code, "    /// {fence}");
    if !key.is_function() {
        let _ = writeln!(
            code,
            "    pub const {}: ::i18n::Key = ::i18n::Key::new({index});",
            identifier(key.key, true)
        );
        return;
    }
    let lifetime = match key.placeholders.len() {
        0 => "'static",
        1 => "'_",
        _ => "'a",
    };
    let reference = if key.placeholders.len() > 1 {
        "&'a "
    } else {
        "&"
    };
    let mut parameters = Vec::new();
    if key.plural {
        parameters.push(format!("{COUNT_PLACEHOLDER}: u64"));
    }
    let arguments: Vec<String> = key
        .placeholders
        .iter()
        .map(|placeholder| identifier(placeholder, false))
        .collect();
    for argument in &arguments {
        parameters.push(format!("{argument}: {reference}dyn ::core::fmt::Display"));
    }
    let generics = if key.placeholders.len() > 1 {
        "<'a>"
    } else {
        ""
    };
    let _ = writeln!(
        code,
        "    pub fn {}{generics}({}) -> ::i18n::Message<{lifetime}, {}> {{",
        identifier(key.key, false),
        parameters.join(", "),
        arguments.len()
    );
    let constructor = if key.plural {
        format!("plural(::i18n::Key::new({index}), {COUNT_PLACEHOLDER}, ")
    } else {
        format!("new(::i18n::Key::new({index}), ")
    };
    let _ = writeln!(
        code,
        "        ::i18n::Message::{constructor}[{}])",
        arguments.join(", ")
    );
    let _ = writeln!(code, "    }}");
}

/// Returns a value for a doc comment line. Control characters and the bidirectional formatting
/// characters become spaces. rustc denies the latter in comments.
fn doc_text(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control()
                || matches!(character, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
            {
                ' '
            } else {
                character
            }
        })
        .collect()
}

/// Returns the length of a code block fence that is longer than every backtick run of the lines.
fn fence_length(lines: &[String]) -> usize {
    let longest_run = lines
        .iter()
        .flat_map(|line| line.split(|character| character != '`'))
        .map(str::len)
        .max()
        .unwrap_or(0);
    (longest_run + 1).max(MINIMUM_FENCE_LENGTH)
}

/// Builds a schema from a reference source. The schema and its names live until the end of the
/// process.
#[cfg(test)]
pub(crate) fn leak_schema(source: &str) -> &'static crate::schema::Schema {
    use crate::schema::{KeyInfo, Schema};

    let source: &'static str = String::from(source).leak();
    let reference = Reference::read(source).unwrap_or_else(|faults| panic!("{faults:?}"));
    let keys: Vec<KeyInfo> = reference
        .keys
        .into_iter()
        .map(|key| KeyInfo::new(key.section, key.key, key.placeholders.leak(), key.plural))
        .collect();
    Box::leak(Box::new(Schema::new(keys.leak())))
}

#[cfg(test)]
#[rustfmt::skip]
mod sample;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Catalog, ListKind,
        fixture::{self, ENGLISH_HEADER, ENGLISH_LIST},
    };

    /// The reference file of the sample.
    const SAMPLE_SOURCE: &str = include_str!("generate/sample.ini");

    /// The generated code of the sample.
    const SAMPLE_CODE: &str = include_str!("generate/sample.rs");

    /// Returns the faults of a reference with a body below the header and the list patterns. The
    /// body starts at line 11.
    fn faults(body: &str) -> Vec<String> {
        let source = fixture::english(body);
        match generate(&source) {
            Ok(_) => Vec::new(),
            Err(faults) => faults.iter().map(ToString::to_string).collect(),
        }
    }

    #[test]
    fn the_generated_code_of_the_sample() {
        assert_eq!(generate(SAMPLE_SOURCE).unwrap(), SAMPLE_CODE);
    }

    #[test]
    fn the_generated_code_serves_a_catalog() {
        let catalog = Catalog::reference("en", SAMPLE_SOURCE, &sample::SCHEMA).unwrap();
        assert_eq!(catalog.text(sample::common::CANCEL), "Cancel");
        assert_eq!(catalog.text(sample::common::SAVE), "Save");
        assert_eq!(
            catalog.text(sample::manager::FILTER_PLACEHOLDER),
            "Filter games"
        );
        assert_eq!(
            catalog
                .display(sample::manager::delete_question(&"Lemmings"))
                .to_string(),
            "Delete Lemmings?"
        );
        assert_eq!(
            catalog
                .display(sample::manager::full_name(&"Taro", &"Yamada"))
                .to_string(),
            "Taro Yamada"
        );
        assert_eq!(
            catalog.display(sample::manager::images(1)).to_string(),
            "One image"
        );
        assert_eq!(
            catalog.display(sample::manager::images(4)).to_string(),
            "4 images"
        );
        assert_eq!(
            catalog
                .display(sample::manager::files(2, &"A:"))
                .to_string(),
            "2 files in A:"
        );
        assert_eq!(catalog.text(sample::common::QUOTE), "Press `Enter` {now}");
        let label = sample::SCHEMA.find("setting.cpu-mode", "label").unwrap();
        assert_eq!(catalog.text(label), "CPU mode");
        let mut list = String::new();
        catalog
            .write_list(&mut list, ListKind::And, &[&"a", &"b", &"c"])
            .unwrap();
        assert_eq!(list, "a, b, and c");
    }

    #[test]
    fn a_clean_reference_has_no_faults() {
        assert_eq!(faults("[common]\ncancel = Cancel\n"), Vec::<String>::new());
    }

    #[test]
    fn faults_of_the_parser_carry_their_line() {
        assert_eq!(
            faults("[common]\ncancel = Cancel {\n"),
            vec!["line 12: common.cancel: unclosed placeholder"]
        );
    }

    #[test]
    fn header_faults() {
        assert_eq!(
            generate(&format!("{ENGLISH_LIST}[language]\nname = English\n")).unwrap_err(),
            vec![Fault {
                line: None,
                key: Some(FullKey {
                    section: "language",
                    key: "plural"
                }),
                kind: FaultKind::MissingKey,
            }]
        );
        let faults: Vec<String> = generate(&format!(
            "[language]\nname =  \nplural = one-other\n{ENGLISH_LIST}"
        ))
        .unwrap_err()
        .iter()
        .map(ToString::to_string)
        .collect();
        assert_eq!(faults, vec!["line 2: language.name: empty value"]);
    }

    #[test]
    fn list_pattern_faults() {
        let source = format!(
            "{ENGLISH_HEADER}[list]\nand-pair = {{second}} and {{first}}\nand-middle = {{first}}, {{second}}\n\
             and-end = {{first}} and {{second}}{{second}}\nor-pair = {{first}} or {{second}}\n\
             or-middle = {{first}}, {{third}}\n"
        );
        let faults: Vec<String> = generate(&source)
            .unwrap_err()
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            faults,
            vec![
                "line 5: list.and-pair: the list pattern lacks {first} and {second} once each in this order",
                "line 7: list.and-end: the list pattern lacks {first} and {second} once each in this order",
                "line 9: list.or-middle: the list pattern lacks {first} and {second} once each in this order",
                "list.or-end: missing key",
            ]
        );
    }

    #[test]
    fn plural_faults() {
        assert_eq!(
            faults(
                "[manager]\nimages.other = {count} images\nfiles.one = One file\nfiles.other = {count} files\nfiles = Files\n"
            ),
            vec![
                "line 12: manager.images: missing plural form `one`",
                "line 15: manager.files: duplicate key",
            ]
        );
        assert_eq!(
            faults("[manager]\nimages.one = One\nimages.other = Many\nimages.one = Again\n"),
            vec!["line 14: manager.images.one: duplicate key"]
        );
    }

    #[test]
    fn identifier_faults() {
        assert_eq!(
            faults(
                "[type]\nlabel = x\n[a-b]\nc = x\n[a.b]\nd = x\n[e]\nfoo-bar = x\nfoo.bar = x\n\
                 3d = x\nmatch = {x}\nself = x\nholder = {type}\n"
            ),
            vec![
                "line 11: the name `type` gives a Rust keyword",
                "line 15: the names `a-b` and `a.b` give the same Rust identifier",
                "line 19: e.foo.bar: the names `foo-bar` and `foo.bar` give the same Rust identifier",
                "line 20: e.3d: the name `3d` gives no Rust identifier",
                "line 21: e.match: the name `match` gives a Rust keyword",
                "line 23: e.holder: the name `type` gives a Rust keyword",
            ]
        );
    }

    #[test]
    fn generated_sections_allow_lower_case_names() {
        assert_eq!(
            faults(
                "[Common]
label = x
[e]
Cancel = x
a_b = x
c+d = x
ok = x
[f:g]
h = x
"
            ),
            vec![
                "line 11: bad name `Common`",
                "line 14: e.Cancel: bad name `Cancel`",
                "line 15: e.a_b: bad name `a_b`",
                "line 16: e.c+d: bad name `c+d`",
                "line 18: bad name `f:g`",
            ]
        );
    }

    #[test]
    fn skipped_sections_accept_any_name() {
        assert_eq!(
            faults(
                "[setting.memory]\n640k = 640 KB\ntype = Type\n86+26k = Both\n4:3 = Wide\nA = Drive A\n\
                 one = One\nother = Other\n[setting.type]\nlabel = {type}\n"
            ),
            Vec::<String>::new()
        );
    }

    /// Verifies that a section `[choice.*]` takes the name rules of a generated section.
    #[test]
    fn a_choice_section_is_a_generated_section() {
        assert_eq!(
            faults("[choice.memory]\nA = Drive A\n"),
            vec!["line 12: choice.memory.A: bad name `A`"]
        );
    }

    #[test]
    fn a_text_with_too_many_placeholders_is_a_fault() {
        assert_eq!(
            faults("[e]\nmany = {a}{b}{c}{d}{e}{f}{g}{h}\n"),
            vec!["line 12: e.many: the text has too many placeholders"]
        );
        assert_eq!(
            faults("[e]\nseven = {a}{b}{c}{d}{e}{f}{g}\n"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn the_report_starts_each_fault_with_the_path() {
        let source = fixture::english("[common]\ncancel = \\x\nsave = {y\n");
        let faults = generate(&source).unwrap_err();
        assert_eq!(
            report(Path::new("language/en.ini"), &faults),
            "language/en.ini: line 12: common.cancel: bad escape\n\
             language/en.ini: line 13: common.save: unclosed placeholder\n"
        );
    }

    #[test]
    fn fences_outgrow_the_backtick_runs() {
        assert_eq!(fence_length(&[String::from("plain")]), 3);
        assert_eq!(
            fence_length(&[String::from("a ``` b"), String::from("````")]),
            5
        );
        assert_eq!(doc_text("a\tb"), "a b");
        assert_eq!(doc_text("a\u{202E}b\u{2066}c"), "a b c");
    }
}
