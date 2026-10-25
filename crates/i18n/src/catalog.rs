//! The catalog of one language: the texts of every key of a schema.
//!
//! A catalog holds all of its texts in one text storage, one entry for each key of its schema and
//! one list with the segments of the texts with placeholders.

mod load;
mod template;

use core::{fmt, ops::Range, ptr};

use crate::{
    parse::Fault,
    plural::{PluralCategory, PluralRule},
    schema::{Key, Schema},
};

/// The placeholder of a plural text that takes the count.
pub const COUNT_PLACEHOLDER: &str = "count";

/// A byte range of the text storage or an index range of the segment list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Span {
    /// The first index of the range.
    pub(crate) start: u32,
    /// The index after the last index of the range.
    pub(crate) end: u32,
}

impl Span {
    /// The empty range at the start.
    const EMPTY: Span = Span { start: 0, end: 0 };

    /// Returns the range as indices.
    pub(crate) fn range(self) -> Range<usize> {
        self.start as usize..self.end as usize
    }

    /// Returns the length of the range.
    fn length(self) -> u32 {
        self.end - self.start
    }

    /// Returns true when the range is empty.
    pub(crate) fn is_empty(self) -> bool {
        self.start == self.end
    }
}

/// A segment of a text with placeholders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Segment {
    /// Literal text of the text storage.
    Literal(Span),
    /// The argument with this index in the argument order of the schema.
    Argument(u16),
    /// The count of a plural text.
    Count,
}

/// One form of a text: its literal text and its segments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Form {
    /// All literal text of the form. For a fixed text it is the whole text.
    pub(crate) text: Span,
    /// The segments of the form. A fixed text has no segments.
    pub(crate) segments: Span,
}

impl Form {
    /// The form without text.
    const EMPTY: Form = Form {
        text: Span::EMPTY,
        segments: Span::EMPTY,
    };
}

/// The texts of one key, indexed by [`PluralCategory::index`]. A fixed text and a text with
/// placeholders hold their form at the index of [`PluralCategory::Other`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Entry {
    /// The forms of the text for each plural category.
    pub(crate) forms: [Form; PluralCategory::COUNT],
    /// The plural rule that selects the form of a plural text. It is the rule of the catalog that
    /// supplied the text.
    pub(crate) plural: Option<PluralRule>,
}

impl Entry {
    /// The entry without text.
    const EMPTY: Entry = Entry {
        forms: [Form::EMPTY; PluralCategory::COUNT],
        plural: None,
    };

    /// Returns the form for a count.
    pub(crate) fn form(&self, count: Option<u64>) -> Form {
        let category = match (self.plural, count) {
            (Some(rule), Some(count)) => rule.category(count),
            (Some(_) | None, _) => PluralCategory::Other,
        };
        self.forms[category.index()]
    }
}

/// The catalog of one language.
pub struct Catalog {
    /// The schema of the catalog.
    pub(crate) schema: &'static Schema,
    /// All unescaped texts, the language name and the tag.
    pub(crate) storage: Box<str>,
    /// One entry for each key of the schema, indexed by [`Key`].
    pub(crate) entries: Box<[Entry]>,
    /// The segments of the texts with placeholders.
    pub(crate) segments: Box<[Segment]>,
    /// The plural rule of the language.
    plural_rule: PluralRule,
    /// The name of the language in the language itself.
    name: Span,
    /// The language tag.
    tag: Span,
}

/// A loaded catalog together with the faults of its file.
#[derive(Debug)]
pub struct Loaded<'a> {
    /// The catalog.
    pub catalog: Catalog,
    /// The faults of the file. The list is empty for a clean file.
    pub faults: Vec<Fault<'a>>,
}

impl Catalog {
    /// Builds the reference catalog. The source holds every key of the schema.
    ///
    /// The load fails with the faults of the source when the source has any fault. A source that
    /// exceeds the size of the text storage fails with
    /// [`FaultKind::TooLarge`](crate::FaultKind::TooLarge).
    pub fn reference<'a>(
        tag: &str,
        source: &'a str,
        schema: &'static Schema,
    ) -> Result<Catalog, Vec<Fault<'a>>> {
        let loaded = load::load(tag, source, schema, None)?;
        if loaded.faults.is_empty() {
            Ok(loaded.catalog)
        } else {
            Err(loaded.faults)
        }
    }

    /// Loads the catalog of a language. A key that the source lacks takes the text of the fallback
    /// catalog.
    ///
    /// The load fails when the header lacks the name or the plural rule of the language, or when
    /// the name is empty. It also fails with [`FaultKind::TooLarge`](crate::FaultKind::TooLarge)
    /// when the source and the fallback exceed the size of the text storage. A text that lacks a
    /// placeholder of its key keeps its text and adds
    /// [`FaultKind::MissingPlaceholder`](crate::FaultKind::MissingPlaceholder) to the fault list.
    /// A plural text lacks a placeholder when none of its forms uses it. Every other fault drops
    /// the faulty text and lands in the fault list.
    ///
    /// # Panics
    ///
    /// Panics when the fallback catalog has another schema.
    pub fn load<'a>(
        tag: &str,
        source: &'a str,
        schema: &'static Schema,
        fallback: &Catalog,
    ) -> Result<Loaded<'a>, Vec<Fault<'a>>> {
        assert!(
            ptr::eq(fallback.schema, schema),
            "the fallback catalog has another schema"
        );
        load::load(tag, source, schema, Some(fallback))
    }

    /// Returns the language tag.
    pub fn tag(&self) -> &str {
        self.slice(self.tag)
    }

    /// Returns the name of the language in the language itself.
    pub fn name(&self) -> &str {
        self.slice(self.name)
    }

    /// Returns the plural rule of the language.
    pub fn plural_rule(&self) -> PluralRule {
        self.plural_rule
    }

    /// Returns the schema of the catalog.
    pub fn schema(&self) -> &'static Schema {
        self.schema
    }

    /// Finds a key by its section and its key inside the section.
    pub fn find(&self, section: &str, key: &str) -> Option<Key> {
        self.schema.find(section, key)
    }

    /// Finds a key by the two parts of its section `<group>.<name>` and its key inside the
    /// section.
    pub fn find_in_group(&self, group: &str, name: &str, key: &str) -> Option<Key> {
        self.schema.find_in_group(group, name, key)
    }

    /// Returns a range of the text storage.
    pub(crate) fn slice(&self, span: Span) -> &str {
        &self.storage[span.range()]
    }
}

impl fmt::Debug for Catalog {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Catalog")
            .field("tag", &self.tag())
            .field("name", &self.name())
            .field("plural_rule", &self.plural_rule)
            .field("keys", &self.entries.len())
            .finish()
    }
}

/// Returns an index of the text storage or of the segment list as an offset.
fn offset(index: usize) -> u32 {
    u32::try_from(index).expect("the load checks the size of the file")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{fixture, generate::leak_schema};

    #[test]
    fn a_catalog_describes_itself() {
        let source =
            fixture::english("[common]\ncancel = Cancel\n[setting.cpu-mode]\nlabel = CPU mode\n");
        let schema = leak_schema(&source);
        let catalog = Catalog::reference("en", &source, schema).unwrap();
        assert_eq!(catalog.tag(), "en");
        assert_eq!(catalog.name(), "English");
        assert_eq!(catalog.plural_rule(), PluralRule::OneOther);
        assert!(ptr::eq(catalog.schema(), schema));
        assert_eq!(catalog.find("common", "cancel"), Some(Key::new(0)));
        assert_eq!(catalog.find("common", "save"), None);
        assert_eq!(
            catalog.find_in_group("setting", "cpu-mode", "label"),
            Some(Key::new(7))
        );
        assert_eq!(catalog.find_in_group("setting", "cpu-mode", "help"), None);
        assert_eq!(catalog.find_in_group("common", "", "cancel"), None);
        assert_eq!(
            format!("{catalog:?}"),
            "Catalog { tag: \"en\", name: \"English\", plural_rule: OneOther, keys: 8 }"
        );
    }

    #[test]
    fn entries_select_their_form_by_count() {
        let form = |start| Form {
            text: Span {
                start,
                end: start + 1,
            },
            segments: Span::EMPTY,
        };
        let plural = Entry {
            forms: [form(0), form(1)],
            plural: Some(PluralRule::OneOther),
        };
        assert_eq!(plural.form(Some(1)), form(0));
        assert_eq!(plural.form(Some(2)), form(1));
        assert_eq!(plural.form(None), form(1));
        let fixed = Entry {
            forms: [Form::EMPTY, form(5)],
            plural: None,
        };
        assert_eq!(fixed.form(Some(1)), form(5));
    }
}
