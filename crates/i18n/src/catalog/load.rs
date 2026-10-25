//! The load of a catalog from a language file.
//!
//! The load parses the language file in two passes. The first pass checks the file and measures
//! the texts. The second pass writes the texts into storage of the exact size. A load makes the
//! same number of allocations for every number of keys.

use super::{
    Catalog, Entry, Form, Loaded, Segment, Span, offset,
    template::{Size, measure, write_form},
};
use crate::{
    parse::{Event, Fault, FaultKind, FullKey, Parser, Token, Value, check_size},
    plural::{PluralCategory, PluralRule, split_plural_key},
    schema::{Key, KeyInfo, LIST_PATTERN_KEYS, LIST_PLACEHOLDERS, LIST_SECTION, Schema},
};

/// The state of one form of one key during a load.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot<'a> {
    /// The file has no pair for the form.
    Unseen,
    /// The file has a faulty pair for the form, or the key lacks a form of its plural rule.
    Rejected {
        /// The line of the pair, if the file has one.
        line: Option<u32>,
    },
    /// The pair on this line supplies the form.
    Accepted {
        /// The line of the pair.
        line: u32,
        /// The value of the pair.
        value: Value<'a>,
        /// The size of the text of the pair.
        size: Size,
    },
}

impl Slot<'_> {
    /// Returns the line of the pair of the slot.
    fn line(self) -> Option<u32> {
        match self {
            Slot::Unseen => None,
            Slot::Rejected { line } => line,
            Slot::Accepted { line, .. } => Some(line),
        }
    }
}

/// The state of a load.
struct Loader<'a, 'f> {
    /// The source of the language file.
    source: &'a str,
    /// The schema of the catalog.
    schema: &'static Schema,
    /// The catalog that supplies the texts that the file lacks.
    fallback: Option<&'f Catalog>,
    /// The slots of the forms, [`PluralCategory::COUNT`] for each key.
    slots: Vec<Slot<'a>>,
    /// The faults of the file.
    faults: Vec<Fault<'a>>,
    /// The plural rule of the header.
    rule: Option<PluralRule>,
    /// The size of the language name of the header.
    name: Option<Size>,
}

/// Loads a catalog with an optional fallback catalog.
pub(super) fn load<'a>(
    tag: &str,
    source: &'a str,
    schema: &'static Schema,
    fallback: Option<&Catalog>,
) -> Result<Loaded<'a>, Vec<Fault<'a>>> {
    let (fallback_bytes, fallback_segments) = fallback.map_or((0, 0), |catalog| {
        (catalog.storage.len(), catalog.segments.len())
    });
    check_size(
        source
            .len()
            .saturating_add(tag.len())
            .saturating_add(fallback_bytes)
            .saturating_add(fallback_segments),
    )?;
    let mut loader = Loader {
        source,
        schema,
        fallback,
        slots: vec![Slot::Unseen; schema.len() * PluralCategory::COUNT],
        faults: Vec::new(),
        rule: None,
        name: None,
    };
    loader.survey();
    let (Some(rule), Some(name)) = (loader.rule, loader.name) else {
        return Err(loader.faults);
    };
    let mut total = loader.settle(rule);
    total.bytes += offset(tag.len()) + name.bytes;
    let catalog = loader.fill(tag, rule, total);
    Ok(Loaded {
        catalog,
        faults: loader.faults,
    })
}

impl<'a> Loader<'a, '_> {
    /// The first pass: checks every line and the header keys and measures the texts.
    fn survey(&mut self) {
        let mut parser = Parser::new(self.source);
        for event in parser.by_ref() {
            match event {
                Event::Section { .. } => {}
                Event::Name { value, .. } => self.name = measure(value, None).ok(),
                Event::Plural { rule, .. } => self.rule = Some(rule),
                Event::Pair { line, key, value } => self.survey_pair(line, key, value),
                Event::Fault(fault) => {
                    if let Some(key) = fault.key
                        && let Some(slot) = self.slot_index(key)
                        && self.slots[slot] == Slot::Unseen
                    {
                        self.slots[slot] = Slot::Rejected { line: fault.line };
                    }
                    self.faults.push(fault);
                }
            }
        }
        parser.push_missing_header_keys(&mut self.faults);
    }

    /// Checks and measures one pair of the first pass.
    fn survey_pair(&mut self, line: u32, key: FullKey<'a>, value: Value<'a>) {
        let Some((schema_key, category)) = resolve(self.schema, key) else {
            self.fault(Some(line), Some(key), FaultKind::UnknownKey);
            return;
        };
        let slot = slot_index(schema_key, category);
        if self.slots[slot] != Slot::Unseen {
            self.fault(Some(line), Some(key), FaultKind::DuplicateKey);
            return;
        }
        self.slots[slot] = match self.check_pair(key, value, schema_key) {
            Ok(size) => Slot::Accepted { line, value, size },
            Err(kind) => {
                self.fault(Some(line), Some(key), kind);
                Slot::Rejected { line: Some(line) }
            }
        };
    }

    /// Checks one pair and returns the size of its text.
    fn check_pair(
        &self,
        key: FullKey<'a>,
        value: Value<'a>,
        schema_key: Key,
    ) -> Result<Size, FaultKind<'a>> {
        if self.fallback.is_some() && value.raw().is_empty() {
            return Err(FaultKind::EmptyValue);
        }
        let size = measure(value, Some(self.schema.info(schema_key)))?;
        if key.section == LIST_SECTION
            && LIST_PATTERN_KEYS.contains(&key.key)
            && !value.has_exact_placeholders(&LIST_PLACEHOLDERS)
        {
            return Err(FaultKind::BadListPattern);
        }
        Ok(size)
    }

    /// Adds a fault for each placeholder of the schema that no accepted form of a key of a
    /// translation uses. The forms keep their texts. The count of a plural text needs no
    /// placeholder.
    fn check_placeholders(&mut self, index: usize, key: FullKey<'a>) {
        if self.fallback.is_none() {
            return;
        }
        let slots = index * PluralCategory::COUNT..(index + 1) * PluralCategory::COUNT;
        let first_line = self.slots[slots.clone()]
            .iter()
            .filter_map(|slot| match slot {
                Slot::Accepted { line, .. } => Some(*line),
                Slot::Unseen | Slot::Rejected { .. } => None,
            })
            .min();
        for &name in self.schema.keys()[index].placeholders() {
            let present = self.slots[slots.clone()].iter().any(|slot| match slot {
                Slot::Accepted { value, .. } => value
                    .tokens()
                    .any(|token| token == Ok(Token::Placeholder(name))),
                Slot::Unseen | Slot::Rejected { .. } => false,
            });
            if !present {
                self.fault(first_line, Some(key), FaultKind::MissingPlaceholder(name));
            }
        }
    }

    /// Decides for each key between the text of the file and the text of the fallback and checks
    /// the placeholders of the accepted texts. Returns the size of all texts.
    fn settle(&mut self, rule: PluralRule) -> Size {
        let mut total = Size::default();
        for (index, info) in self.schema.keys().iter().enumerate() {
            let key = FullKey {
                section: info.section(),
                key: info.key(),
            };
            let first_slot = index * PluralCategory::COUNT;
            if info.is_plural() {
                for category in PluralCategory::ALL {
                    let slot = first_slot + category.index();
                    if let Slot::Accepted { line, .. } = self.slots[slot]
                        && !rule.has(category)
                    {
                        self.fault(
                            Some(line),
                            Some(key),
                            FaultKind::CategoryOutsideRule(category),
                        );
                        self.slots[slot] = Slot::Rejected { line: Some(line) };
                    }
                }
            }
            let required = required_categories(info, rule);
            if self.is_complete(index, rule) {
                for category in required {
                    if let Slot::Accepted { size, .. } = self.slots[first_slot + category.index()] {
                        total.bytes += size.bytes;
                        total.segments += size.segments;
                    }
                }
                self.check_placeholders(index, key);
                continue;
            }
            let slots = first_slot..first_slot + PluralCategory::COUNT;
            if self.slots[slots.clone()]
                .iter()
                .all(|slot| *slot == Slot::Unseen)
            {
                self.fault(None, Some(key), FaultKind::MissingKey);
            } else {
                let first_line = self.slots[slots.clone()]
                    .iter()
                    .filter_map(|slot| slot.line())
                    .min();
                for category in required {
                    if self.slots[first_slot + category.index()] == Slot::Unseen {
                        self.fault(
                            first_line,
                            Some(key),
                            FaultKind::MissingPluralCategory(*category),
                        );
                    }
                }
            }
            for slot in &mut self.slots[slots] {
                *slot = Slot::Rejected { line: slot.line() };
            }
            if let Some(fallback) = self.fallback {
                let fallback_size = fallback.entry_size(index);
                total.bytes += fallback_size.bytes;
                total.segments += fallback_size.segments;
            }
        }
        total
    }

    /// The second pass: writes the texts into storage of the exact size.
    fn fill(&mut self, tag: &str, rule: PluralRule, total: Size) -> Catalog {
        let mut storage = String::with_capacity(total.bytes as usize);
        let mut segments = Vec::with_capacity(total.segments as usize);
        let mut entries = vec![Entry::EMPTY; self.schema.len()];
        storage.push_str(tag);
        let tag = Span {
            start: 0,
            end: offset(storage.len()),
        };
        if let Some(fallback) = self.fallback {
            for (index, entry) in entries.iter_mut().enumerate() {
                if !self.is_complete(index, rule) {
                    *entry = fallback.copy_entry(index, &mut storage, &mut segments);
                }
            }
        }
        let mut name = Span::EMPTY;
        for event in Parser::new(self.source) {
            match event {
                Event::Name { value, .. } => {
                    name = write_form(value, None, &mut storage, &mut segments).text;
                }
                Event::Pair { line, key, value } => {
                    let Some((schema_key, category)) = resolve(self.schema, key) else {
                        continue;
                    };
                    let accepted = matches!(
                        self.slots[slot_index(schema_key, category)],
                        Slot::Accepted { line: accepted_line, .. } if accepted_line == line
                    );
                    if !accepted {
                        continue;
                    }
                    let info = self.schema.info(schema_key);
                    let entry = &mut entries[schema_key.index()];
                    entry.forms[category.index()] =
                        write_form(value, Some(info), &mut storage, &mut segments);
                    entry.plural = info.is_plural().then_some(rule);
                }
                Event::Section { .. } | Event::Plural { .. } | Event::Fault(_) => {}
            }
        }
        assert_eq!(storage.len(), total.bytes as usize);
        assert_eq!(segments.len(), total.segments as usize);
        Catalog {
            schema: self.schema,
            storage: storage.into_boxed_str(),
            entries: entries.into_boxed_slice(),
            segments: segments.into_boxed_slice(),
            plural_rule: rule,
            name,
            tag,
        }
    }

    /// Returns true when the file supplies every form of the key that its plural rule requires.
    fn is_complete(&self, index: usize, rule: PluralRule) -> bool {
        let first_slot = index * PluralCategory::COUNT;
        required_categories(&self.schema.keys()[index], rule)
            .iter()
            .all(|category| {
                matches!(
                    self.slots[first_slot + category.index()],
                    Slot::Accepted { .. }
                )
            })
    }

    /// Returns the slot of a key of the file.
    fn slot_index(&self, key: FullKey<'_>) -> Option<usize> {
        resolve(self.schema, key).map(|(key, category)| slot_index(key, category))
    }

    /// Adds a fault.
    fn fault(&mut self, line: Option<u32>, key: Option<FullKey<'a>>, kind: FaultKind<'a>) {
        self.faults.push(Fault { line, key, kind });
    }
}

/// Returns the categories of a key that a file of a plural rule supplies.
fn required_categories(info: &KeyInfo, rule: PluralRule) -> &'static [PluralCategory] {
    if info.is_plural() {
        rule.categories()
    } else {
        &[PluralCategory::Other]
    }
}

/// Returns the slot index of a form of a key.
fn slot_index(key: Key, category: PluralCategory) -> usize {
    key.index() * PluralCategory::COUNT + category.index()
}

/// Resolves a key of a file to the key of the schema and the plural category of the form.
fn resolve(schema: &Schema, key: FullKey<'_>) -> Option<(Key, PluralCategory)> {
    let (base, category) = split_plural_key(key.key);
    match category {
        Some(category) => schema
            .find(key.section, base)
            .filter(|found| schema.info(*found).is_plural())
            .map(|found| (found, category)),
        None => schema
            .find(key.section, key.key)
            .filter(|found| !schema.info(*found).is_plural())
            .map(|found| (found, PluralCategory::Other)),
    }
}

impl Catalog {
    /// Returns the size of the texts of an entry.
    fn entry_size(&self, index: usize) -> Size {
        self.entries[index]
            .forms
            .iter()
            .fold(Size::default(), |size, form| Size {
                bytes: size.bytes + form.text.length(),
                segments: size.segments + form.segments.length(),
            })
    }

    /// Copies an entry into new storage and returns the copy.
    fn copy_entry(&self, index: usize, storage: &mut String, segments: &mut Vec<Segment>) -> Entry {
        let source = &self.entries[index];
        let mut entry = Entry {
            forms: [Form::EMPTY; PluralCategory::COUNT],
            plural: source.plural,
        };
        for (form, source_form) in entry.forms.iter_mut().zip(source.forms) {
            let text_start = offset(storage.len());
            storage.push_str(self.slice(source_form.text));
            let segment_start = offset(segments.len());
            segments.extend(
                self.segments[source_form.segments.range()]
                    .iter()
                    .map(|segment| match *segment {
                        Segment::Literal(span) => Segment::Literal(Span {
                            start: span.start - source_form.text.start + text_start,
                            end: span.end - source_form.text.start + text_start,
                        }),
                        Segment::Argument(_) | Segment::Count => *segment,
                    }),
            );
            *form = Form {
                text: Span {
                    start: text_start,
                    end: offset(storage.len()),
                },
                segments: Span {
                    start: segment_start,
                    end: offset(segments.len()),
                },
            };
        }
        entry
    }
}

#[cfg(test)]
mod tests {
    use core::fmt;

    use common::allocation_counter::count_allocations;

    use super::*;
    use crate::{
        Message,
        fixture::{self, ENGLISH_HEADER, ENGLISH_LIST, JAPANESE_HEADER, JAPANESE_LIST},
        generate::leak_schema,
        message::ListKind,
    };

    /// The sections of the reference source below the header and the list patterns.
    const REFERENCE_BODY: &str = "[common]\ncancel = Cancel\nsave = Save\n\
                                  [manager]\ndelete-question = Delete {name}?\n\
                                  images.one = {count} image in {folder}\n\
                                  images.other = {count} images in {folder}\n";

    /// A clean Japanese body for the schema of the reference source.
    const JAPANESE_BODY: &str = "[common]\ncancel = キャンセル\nsave = 保存\n[manager]\n\
                                 delete-question = {name}を削除しますか？\n\
                                 images.other = {folder}の{count}枚\n";

    /// Returns the reference source.
    fn reference_source() -> String {
        fixture::english(REFERENCE_BODY)
    }

    /// Returns the schema of the reference source.
    fn schema() -> &'static Schema {
        leak_schema(&reference_source())
    }

    /// Returns a fixed text of a catalog.
    fn text<'c>(catalog: &'c Catalog, section: &str, key: &str) -> &'c str {
        catalog.text(catalog.find(section, key).unwrap())
    }

    /// Returns the text `manager.delete-question` of a catalog for the name `X`.
    fn delete_question(catalog: &Catalog) -> String {
        let key = catalog.find("manager", "delete-question").unwrap();
        catalog.display(Message::new(key, [&"X"])).to_string()
    }

    /// Returns the plural text `manager.images` of a catalog for a count in the folder `A:`.
    fn images(catalog: &Catalog, count: u64) -> String {
        let key = catalog.find("manager", "images").unwrap();
        catalog
            .display(Message::plural(key, count, [&"A:"]))
            .to_string()
    }

    /// Returns a list of a catalog.
    fn list(catalog: &Catalog, kind: ListKind, items: &[impl fmt::Display]) -> String {
        let mut out = String::new();
        catalog.write_list(&mut out, kind, items).unwrap();
        out
    }

    /// Returns the fault of a key that the file lacks.
    fn missing(section: &'static str, key: &'static str) -> Fault<'static> {
        Fault {
            line: None,
            key: Some(FullKey { section, key }),
            kind: FaultKind::MissingKey,
        }
    }

    /// Returns the fault of a key on a line.
    fn fault<'a>(line: u32, section: &'a str, key: &'a str, kind: FaultKind<'a>) -> Fault<'a> {
        Fault {
            line: Some(line),
            key: Some(FullKey { section, key }),
            kind,
        }
    }

    #[test]
    fn the_reference_holds_every_text() {
        let source = reference_source();
        let catalog = Catalog::reference("en", &source, schema()).unwrap();
        assert_eq!(catalog.tag(), "en");
        assert_eq!(catalog.name(), "English");
        assert_eq!(catalog.plural_rule(), PluralRule::OneOther);
        assert_eq!(text(&catalog, "common", "cancel"), "Cancel");
        assert_eq!(text(&catalog, "common", "save"), "Save");
        assert_eq!(delete_question(&catalog), "Delete X?");
        assert_eq!(images(&catalog, 1), "1 image in A:");
        assert_eq!(
            format!("{catalog:?}"),
            "Catalog { tag: \"en\", name: \"English\", plural_rule: OneOther, keys: 10 }"
        );
    }

    #[test]
    fn a_reference_without_a_key_fails_to_load() {
        let schema = schema();
        let source = reference_source().replace("save = Save\n", "");
        assert_eq!(
            Catalog::reference("en", &source, schema).unwrap_err(),
            vec![missing("common", "save")]
        );
    }

    #[test]
    fn a_file_without_a_header_fails_to_load() {
        let schema = schema();
        let source = reference_source();
        let reference = Catalog::reference("en", &source, schema).unwrap();
        let faults =
            Catalog::load("ja", "[common]\ncancel = キャンセル\n", schema, &reference).unwrap_err();
        assert_eq!(
            faults,
            vec![missing("language", "name"), missing("language", "plural")]
        );
        let faults = Catalog::load(
            "ja",
            "[language]\nname =\nplural = none\n",
            schema,
            &reference,
        )
        .unwrap_err();
        assert_eq!(
            faults,
            vec![fault(2, "language", "name", FaultKind::EmptyValue)]
        );
        let faults = Catalog::load(
            "ja",
            "[language]\nname = 日本語\nplural = few\n",
            schema,
            &reference,
        )
        .unwrap_err();
        assert_eq!(
            faults,
            vec![fault(
                3,
                "language",
                "plural",
                FaultKind::BadPluralRule("few")
            )]
        );
    }

    #[test]
    fn a_missing_key_takes_the_text_of_the_fallback() {
        let schema = schema();
        let source = reference_source();
        let reference = Catalog::reference("en", &source, schema).unwrap();
        let japanese = fixture::japanese("[common]\ncancel = キャンセル\n");
        let loaded = Catalog::load("ja", &japanese, schema, &reference).unwrap();
        let catalog = loaded.catalog;
        assert_eq!(catalog.tag(), "ja");
        assert_eq!(catalog.name(), "日本語");
        assert_eq!(catalog.plural_rule(), PluralRule::None);
        assert_eq!(text(&catalog, "common", "cancel"), "キャンセル");
        assert_eq!(text(&catalog, "common", "save"), "Save");
        assert_eq!(delete_question(&catalog), "Delete X?");
        assert_eq!(
            loaded.faults,
            vec![
                missing("common", "save"),
                missing("manager", "delete-question"),
                missing("manager", "images")
            ]
        );
    }

    #[test]
    fn faulty_pairs_take_the_text_of_the_fallback() {
        let schema = schema();
        let source = reference_source();
        let reference = Catalog::reference("en", &source, schema).unwrap();
        let japanese = fixture::japanese(
            "[common]\ncancel = キャンセル\ncancel = 取消\nsave = 保存\\q\nextra = x\n\
             [manager]\ndelete-question = {title}を削除しますか？\nimages.one = {folder}\n\
             images.other = {folder}の{count}枚\n",
        );
        let loaded = Catalog::load("ja", &japanese, schema, &reference).unwrap();
        let catalog = &loaded.catalog;
        assert_eq!(text(catalog, "common", "cancel"), "キャンセル");
        assert_eq!(text(catalog, "common", "save"), "Save");
        assert_eq!(delete_question(catalog), "Delete X?");
        assert_eq!(images(catalog, 3), "A:の3枚");
        assert_eq!(
            loaded.faults,
            vec![
                fault(6, "common", "cancel", FaultKind::DuplicateKey),
                fault(7, "common", "save", FaultKind::BadEscape),
                fault(8, "common", "extra", FaultKind::UnknownKey),
                fault(
                    10,
                    "manager",
                    "delete-question",
                    FaultKind::UnknownPlaceholder("title")
                ),
                fault(
                    11,
                    "manager",
                    "images",
                    FaultKind::CategoryOutsideRule(PluralCategory::One)
                ),
            ]
        );
    }

    #[test]
    fn an_incomplete_plural_text_takes_the_text_of_the_fallback() {
        let schema = schema();
        let source = reference_source();
        let reference = Catalog::reference("en", &source, schema).unwrap();
        let german = format!(
            "[language]\nname = Deutsch\nplural = one-other\n[common]\ncancel = Abbrechen\n\
             save = Speichern\n[manager]\ndelete-question = {{name}} löschen?\n\
             images.other = {{count}} Bilder in {{folder}}\n{ENGLISH_LIST}"
        );
        let loaded = Catalog::load("de", &german, schema, &reference).unwrap();
        assert_eq!(images(&loaded.catalog, 1), "1 image in A:");
        assert_eq!(images(&loaded.catalog, 2), "2 images in A:");
        assert_eq!(
            loaded.faults,
            vec![fault(
                9,
                "manager",
                "images",
                FaultKind::MissingPluralCategory(PluralCategory::One)
            )]
        );
    }

    #[test]
    fn a_plural_text_without_an_accepted_form_lacks_its_categories() {
        let schema = schema();
        let source = reference_source();
        let reference = Catalog::reference("en", &source, schema).unwrap();
        let japanese = format!(
            "{JAPANESE_HEADER}[common]\ncancel = キャンセル\nsave = 保存\n[manager]\n\
             delete-question = {{name}}を削除しますか？\nimages.one = {{folder}}\n{JAPANESE_LIST}"
        );
        let loaded = Catalog::load("ja", &japanese, schema, &reference).unwrap();
        assert_eq!(images(&loaded.catalog, 1), "1 image in A:");
        assert_eq!(
            loaded.faults,
            vec![
                fault(
                    9,
                    "manager",
                    "images",
                    FaultKind::CategoryOutsideRule(PluralCategory::One)
                ),
                fault(
                    9,
                    "manager",
                    "images",
                    FaultKind::MissingPluralCategory(PluralCategory::Other)
                ),
            ]
        );
        let german = format!(
            "[language]\nname = Deutsch\nplural = one-other\n[common]\ncancel = Abbrechen\n\
             save = Speichern\n[manager]\ndelete-question = {{name}} löschen?\n\
             images.one = bad\\q\n{ENGLISH_LIST}"
        );
        let loaded = Catalog::load("de", &german, schema, &reference).unwrap();
        assert_eq!(images(&loaded.catalog, 1), "1 image in A:");
        assert_eq!(
            loaded.faults,
            vec![
                fault(9, "manager", "images.one", FaultKind::BadEscape),
                fault(
                    9,
                    "manager",
                    "images",
                    FaultKind::MissingPluralCategory(PluralCategory::Other)
                ),
            ]
        );
    }

    #[test]
    fn a_plural_form_of_a_fixed_text_is_an_unknown_key() {
        let schema = schema();
        let source = reference_source();
        let reference = Catalog::reference("en", &source, schema).unwrap();
        let japanese = fixture::japanese("[common]\ncancel.other = x\n");
        let loaded = Catalog::load("ja", &japanese, schema, &reference).unwrap();
        assert_eq!(loaded.faults[0].kind, FaultKind::UnknownKey);
        assert_eq!(text(&loaded.catalog, "common", "cancel"), "Cancel");
    }

    #[test]
    fn a_clean_file_has_no_faults() {
        let schema = schema();
        let source = reference_source();
        let reference = Catalog::reference("en", &source, schema).unwrap();
        let japanese = fixture::japanese(JAPANESE_BODY);
        let loaded = Catalog::load("ja", &japanese, schema, &reference).unwrap();
        assert!(loaded.faults.is_empty());
        assert_eq!(loaded.faults.capacity(), 0);
    }

    #[test]
    fn an_empty_value_of_a_translation_takes_the_text_of_the_fallback() {
        let schema = schema();
        let source = reference_source();
        let reference = Catalog::reference("en", &source, schema).unwrap();
        let japanese = fixture::japanese(&JAPANESE_BODY.replace("save = 保存", "save ="));
        let loaded = Catalog::load("ja", &japanese, schema, &reference).unwrap();
        assert_eq!(text(&loaded.catalog, "common", "save"), "Save");
        assert_eq!(
            loaded.faults,
            vec![fault(6, "common", "save", FaultKind::EmptyValue)]
        );
    }

    #[test]
    fn the_reference_accepts_an_empty_value() {
        let schema = schema();
        let source = reference_source().replace("save = Save", "save =");
        let reference = Catalog::reference("en", &source, schema).unwrap();
        assert_eq!(text(&reference, "common", "save"), "");
        let japanese = fixture::japanese(JAPANESE_BODY);
        let loaded = Catalog::load("ja", &japanese, schema, &reference).unwrap();
        assert_eq!(text(&loaded.catalog, "common", "save"), "保存");
    }

    #[test]
    fn a_missing_placeholder_of_a_translation_keeps_the_text() {
        let schema = schema();
        let source = reference_source();
        let reference = Catalog::reference("en", &source, schema).unwrap();
        let japanese = fixture::japanese(
            &JAPANESE_BODY
                .replace("{name}を削除しますか？", "削除しますか？")
                .replace("{folder}の{count}枚", "{count}枚"),
        );
        let loaded = Catalog::load("ja", &japanese, schema, &reference).unwrap();
        assert_eq!(delete_question(&loaded.catalog), "削除しますか？");
        assert_eq!(images(&loaded.catalog, 2), "2枚");
        assert_eq!(
            loaded.faults,
            vec![
                fault(
                    8,
                    "manager",
                    "delete-question",
                    FaultKind::MissingPlaceholder("name")
                ),
                fault(
                    9,
                    "manager",
                    "images",
                    FaultKind::MissingPlaceholder("folder")
                ),
            ]
        );
        let without_count =
            fixture::japanese(&JAPANESE_BODY.replace("{folder}の{count}枚", "{folder}"));
        let loaded = Catalog::load("ja", &without_count, schema, &reference).unwrap();
        assert!(loaded.faults.is_empty(), "{:?}", loaded.faults);
        assert_eq!(images(&loaded.catalog, 2), "A:");
    }

    #[test]
    fn a_placeholder_of_a_plural_text_needs_one_accepted_form() {
        let schema = schema();
        let source = reference_source();
        let reference = Catalog::reference("en", &source, schema).unwrap();
        let german = |one: &str, other: &str| {
            format!(
                "[language]\nname = Deutsch\nplural = one-other\n[common]\ncancel = Abbrechen\n\
                 save = Speichern\n[manager]\ndelete-question = {{name}} löschen?\n\
                 images.one = {one}\nimages.other = {other}\n{ENGLISH_LIST}"
            )
        };
        let source = german("Ein Bild", "{count} Bilder in {folder}");
        let loaded = Catalog::load("de", &source, schema, &reference).unwrap();
        assert!(loaded.faults.is_empty(), "{:?}", loaded.faults);
        assert_eq!(images(&loaded.catalog, 1), "Ein Bild");
        assert_eq!(images(&loaded.catalog, 2), "2 Bilder in A:");
        let source = german("Ein Bild", "{count} Bilder");
        let loaded = Catalog::load("de", &source, schema, &reference).unwrap();
        assert_eq!(
            loaded.faults,
            vec![fault(
                9,
                "manager",
                "images",
                FaultKind::MissingPlaceholder("folder")
            )]
        );
        assert_eq!(images(&loaded.catalog, 2), "2 Bilder");
    }

    #[test]
    fn a_dropped_form_gets_no_placeholder_fault() {
        let schema = schema();
        let source = reference_source();
        let reference = Catalog::reference("en", &source, schema).unwrap();
        let japanese = fixture::japanese(&JAPANESE_BODY.replace(
            "images.other = {folder}の{count}枚",
            "images.one = {count}枚\nimages.other = {folder}の{count}枚",
        ));
        let loaded = Catalog::load("ja", &japanese, schema, &reference).unwrap();
        assert_eq!(
            loaded.faults,
            vec![fault(
                9,
                "manager",
                "images",
                FaultKind::CategoryOutsideRule(PluralCategory::One)
            )]
        );
        assert_eq!(images(&loaded.catalog, 1), "A:の1枚");
        let without_other = fixture::japanese(&JAPANESE_BODY.replace(
            "images.other = {folder}の{count}枚",
            "images.one = {count}枚",
        ));
        let loaded = Catalog::load("ja", &without_other, schema, &reference).unwrap();
        assert_eq!(
            loaded.faults,
            vec![
                fault(
                    9,
                    "manager",
                    "images",
                    FaultKind::CategoryOutsideRule(PluralCategory::One)
                ),
                fault(
                    9,
                    "manager",
                    "images",
                    FaultKind::MissingPluralCategory(PluralCategory::Other)
                ),
            ]
        );
    }

    #[test]
    fn a_bad_list_pattern_takes_the_pattern_of_the_fallback() {
        let schema = schema();
        let source = reference_source();
        let reference = Catalog::reference("en", &source, schema).unwrap();
        let patterns = JAPANESE_LIST
            .replace(
                "and-middle = {first}、{second}",
                "and-middle = {first}{second}{second}",
            )
            .replace("or-middle = {first}、{second}", "or-middle = {first}");
        let japanese = format!("{JAPANESE_HEADER}{JAPANESE_BODY}{patterns}");
        let loaded = Catalog::load("ja", &japanese, schema, &reference).unwrap();
        assert_eq!(
            loaded.faults,
            vec![
                fault(12, "list", "and-middle", FaultKind::BadListPattern),
                fault(15, "list", "or-middle", FaultKind::BadListPattern),
            ]
        );
        let catalog = &loaded.catalog;
        assert_eq!(
            list(catalog, ListKind::And, &[&"a", &"b", &"c", &"d"]),
            "a, b, c、d"
        );
        assert_eq!(list(catalog, ListKind::Or, &[&"a", &"b", &"c"]), "a; bかc");
        let bad_reference = source.replace(
            "and-end = {first}, and {second}",
            "and-end = {second} and {first}",
        );
        assert_eq!(
            Catalog::reference("en", &bad_reference, schema).unwrap_err(),
            vec![fault(7, "list", "and-end", FaultKind::BadListPattern)]
        );
    }

    #[test]
    fn choice_values_are_keys() {
        let source = fixture::english(
            "[choice.aspect]\n4:3 = Four to three\nA = Drive A\none = One\nother = Other\n\
             [choice.sound]\n86+26k = Both boards\n",
        );
        let schema = leak_schema(&source);
        let reference = Catalog::reference("en", &source, schema).unwrap();
        let japanese = fixture::japanese(
            "[choice.aspect]\n4:3 = 四対三\nA = ドライブA\none = 一\nother = その他\n\
             [choice.sound]\n86+26k = 両方\n",
        );
        let loaded = Catalog::load("ja", &japanese, schema, &reference).unwrap();
        assert!(loaded.faults.is_empty(), "{:?}", loaded.faults);
        let catalog = &loaded.catalog;
        assert_eq!(text(catalog, "choice.aspect", "4:3"), "四対三");
        assert_eq!(text(catalog, "choice.aspect", "A"), "ドライブA");
        assert_eq!(text(catalog, "choice.aspect", "one"), "一");
        assert_eq!(text(catalog, "choice.aspect", "other"), "その他");
        assert_eq!(text(catalog, "choice.sound", "86+26k"), "両方");
        assert_eq!(text(&reference, "choice.aspect", "one"), "One");
    }

    #[test]
    #[should_panic(expected = "another schema")]
    fn a_fallback_of_another_schema_panics() {
        let source = reference_source();
        let reference = Catalog::reference("en", &source, schema()).unwrap();
        let _ = Catalog::load("en", &source, schema(), &reference);
    }

    /// Returns a reference source with the given number of texts with placeholders.
    fn sized_source(count: usize) -> String {
        let mut source = format!("{ENGLISH_HEADER}{ENGLISH_LIST}[many]\n");
        for index in 0..count {
            source.push_str(&format!(
                "text-{index:04} = Text {index} of {{name}} in {{folder}}\n"
            ));
        }
        source
    }

    #[test]
    fn the_load_count_of_allocations_does_not_depend_on_the_key_count() {
        let mut counts = Vec::new();
        for key_count in [3, 300] {
            let source = sized_source(key_count);
            let schema = leak_schema(&source);
            let reference = Catalog::reference("en", &source, schema).unwrap();
            let translation = source.replace("Text", "Texte");
            let (allocations, loaded) =
                count_allocations(|| Catalog::load("fr", &translation, schema, &reference));
            let loaded = loaded.unwrap();
            assert!(loaded.faults.is_empty());
            assert_eq!(loaded.catalog.schema().len(), key_count + 6);
            let (reference_allocations, _) =
                count_allocations(|| Catalog::reference("en", &source, schema));
            counts.push((allocations, reference_allocations));
        }
        assert_eq!(counts[0], counts[1]);
        assert_eq!(counts[0], (4, 4));
    }

    #[test]
    fn a_faulty_load_with_fallback_texts_allocates_a_fixed_number_of_times() {
        /// The allocations of a load: the slots, the entries, the text storage and the segments.
        const LOAD_ALLOCATIONS: usize = 4;
        /// The allocations of a fault list that holds at most four faults.
        const FAULT_LIST_ALLOCATIONS: usize = 1;
        let schema = schema();
        let source = reference_source();
        let reference = Catalog::reference("en", &source, schema).unwrap();
        let german = format!(
            "[language]\nname = Deutsch\nplural = one-other\n[common]\ncancel = Abbrechen\n\
             [manager]\nimages.other = {{count}} Bilder in {{folder}}\n{ENGLISH_LIST}"
        );
        let (allocations, loaded) =
            count_allocations(|| Catalog::load("de", &german, schema, &reference));
        let loaded = loaded.unwrap();
        assert_eq!(
            loaded.faults,
            vec![
                missing("common", "save"),
                missing("manager", "delete-question"),
                fault(
                    7,
                    "manager",
                    "images",
                    FaultKind::MissingPluralCategory(PluralCategory::One)
                ),
            ]
        );
        assert_eq!(loaded.faults.capacity(), 4);
        assert_eq!(allocations, LOAD_ALLOCATIONS + FAULT_LIST_ALLOCATIONS);
        let catalog = &loaded.catalog;
        assert_eq!(text(catalog, "common", "cancel"), "Abbrechen");
        assert_eq!(text(catalog, "common", "save"), "Save");
        assert_eq!(delete_question(catalog), "Delete X?");
        assert_eq!(images(catalog, 1), "1 image in A:");
        assert_eq!(images(catalog, 2), "2 images in A:");
    }
}
