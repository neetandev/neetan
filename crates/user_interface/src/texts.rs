//! The texts of the user interface: the generated keys of the reference language file
//! `language/en.ini`, the embedded English catalog and the catalog of the selected language.
//!
//! The build script generates the `SCHEMA` static, one module for each section with a `Key`
//! constant for each fixed text and a function for each text with placeholders.

use std::{fmt, sync::LazyLock};

use gui::ui::{Hint, HintKey};
use i18n::{Catalog, Key};
use stack_vec::StackVec;

pub use self::setting_texts::{
    FileTypeText, LimitSentence, file_type_text, setting_help, setting_label, unset_word,
};

mod setting_texts;

include!(concat!(env!("OUT_DIR"), "/texts.rs"));

/// A typed text that writes itself in a catalog: an error or a finding with its data.
pub trait CatalogText {
    /// Writes the text in `catalog`.
    fn write_text(&self, catalog: &Catalog, out: &mut dyn fmt::Write) -> fmt::Result;
}

impl<T: CatalogText + ?Sized> CatalogText for &T {
    /// Writes the text behind the reference.
    fn write_text(&self, catalog: &Catalog, out: &mut dyn fmt::Write) -> fmt::Result {
        (**self).write_text(catalog, out)
    }
}

/// A typed text in a catalog. It formats through [`fmt::Display`] and ignores the width, the
/// fill and every other formatting option of the formatter.
pub struct InCatalog<'a, T: ?Sized> {
    /// The catalog of the text.
    catalog: &'a Catalog,
    /// The typed text.
    text: &'a T,
}

impl<T: CatalogText + ?Sized> fmt::Display for InCatalog<'_, T> {
    /// Writes the text in the catalog.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.text.write_text(self.catalog, formatter)
    }
}

/// Returns the typed text `text` in `catalog` as a value that formats through
/// [`fmt::Display`].
pub fn in_catalog<'a, T: CatalogText + ?Sized>(
    catalog: &'a Catalog,
    text: &'a T,
) -> InCatalog<'a, T> {
    InCatalog { catalog, text }
}

/// The language tag of the reference language.
pub const ENGLISH_TAG: &str = "en";

/// The source of the reference language file.
pub const REFERENCE_SOURCE: &str = include_str!("../../../language/en.ini");

/// The English catalog, built on first use.
static ENGLISH: LazyLock<Catalog> = LazyLock::new(|| {
    Catalog::reference(ENGLISH_TAG, REFERENCE_SOURCE, &SCHEMA).unwrap_or_else(|faults| {
        let report: Vec<String> = faults.iter().map(ToString::to_string).collect();
        panic!("the embedded en.ini does not load: {}", report.join("; "))
    })
});

/// Returns the English catalog of the embedded reference language file.
#[must_use]
pub fn english() -> &'static Catalog {
    &ENGLISH
}

/// The language tag of the German test language.
#[cfg(test)]
const TEST_TAG: &str = "de";

/// The header of the German test language.
#[cfg(test)]
const TEST_HEADER: &str = "[language]\nname = Deutsch\nplural = one-other\n";

/// The source of a German test language file with the header and the sections `sections`.
#[cfg(test)]
pub(crate) fn test_language(sections: &str) -> String {
    format!("{TEST_HEADER}{sections}")
}

/// The German test catalog of the sections `sections` over the English texts.
#[cfg(test)]
pub(crate) fn test_catalog(sections: &str) -> Catalog {
    Catalog::load(TEST_TAG, &test_language(sections), &SCHEMA, english())
        .expect("the test texts load")
        .catalog
}

/// Returns the text that `write` writes with the English catalog.
#[cfg(test)]
pub(crate) fn english_text(write: impl FnOnce(&Catalog, &mut String)) -> String {
    let mut text = String::new();
    write(english(), &mut text);
    text
}

/// The catalog of the selected language: the embedded English catalog or a catalog that a
/// language file gives.
#[derive(Debug)]
pub enum LanguageCatalog {
    /// The embedded English catalog.
    English,
    /// The catalog of a language file.
    Loaded(Box<Catalog>),
}

impl LanguageCatalog {
    /// Returns the catalog.
    #[must_use]
    pub fn catalog(&self) -> &Catalog {
        match self {
            Self::English => english(),
            Self::Loaded(catalog) => catalog,
        }
    }
}

/// The most decimal digits of a `usize`.
const MAXIMUM_DIGITS: usize = 20;

/// The decimal digits of a number on the stack, for a count or a place in a list.
#[derive(Debug, Clone, Copy)]
pub(crate) struct NumberText {
    /// The digits from the start of the array.
    digits: [u8; MAXIMUM_DIGITS],
    /// The number of digits.
    length: usize,
}

impl NumberText {
    /// The digits of `number`.
    pub(crate) fn new(mut number: usize) -> Self {
        let mut reversed = [0; MAXIMUM_DIGITS];
        let mut length = 0;
        loop {
            reversed[length] = b'0' + (number % 10) as u8;
            length += 1;
            number /= 10;
            if number == 0 {
                break;
            }
        }
        let mut digits = [0; MAXIMUM_DIGITS];
        for (target, source) in digits.iter_mut().zip(reversed[..length].iter().rev()) {
            *target = *source;
        }
        Self { digits, length }
    }

    /// The digits as text.
    pub(crate) fn as_str(&self) -> &str {
        std::str::from_utf8(&self.digits[..self.length]).unwrap_or_default()
    }
}

/// A hint of the status line with the key of its label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HintText {
    /// The keys and buttons.
    pub(crate) keys: &'static [HintKey<'static>],
    /// The key of the label of the action.
    pub(crate) label: Key,
}

impl HintText {
    /// The hint of `keys` with the label of `label`.
    pub(crate) const fn new(keys: &'static [HintKey<'static>], label: Key) -> Self {
        Self { keys, label }
    }

    /// Returns the hint with its label in `catalog`.
    pub(crate) fn hint(self, catalog: &Catalog) -> Hint<'_> {
        Hint {
            keys: self.keys,
            label: catalog.text(self.label),
        }
    }
}

/// The most hints of a status line: six. A set number field of the editor shows step, type,
/// clear, section, save and close. A game that starts in the library shows launch, edit,
/// filter, system, refresh and settings.
pub(crate) const MAXIMUM_HINTS: usize = 6;

/// The hints of a status line with the keys of their labels.
pub(crate) type HintTexts = StackVec<HintText, MAXIMUM_HINTS>;

/// The hints of a status line with their labels.
pub(crate) type Hints<'c> = StackVec<Hint<'c>, MAXIMUM_HINTS>;

/// Returns `hints` with their labels in `catalog`.
pub(crate) fn hints_in<'c>(catalog: &'c Catalog, hints: &[HintText]) -> Hints<'c> {
    hints.iter().map(|hint| hint.hint(catalog)).collect()
}

/// Returns the labels of `hints` in the English catalog.
#[cfg(test)]
pub(crate) fn english_labels(hints: &[HintText]) -> Vec<&'static str> {
    hints
        .iter()
        .map(|hint| english().text(hint.label))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::ptr;

    use i18n::PluralRule;

    use super::*;

    /// Verifies that the embedded reference file gives the English catalog with its header.
    #[test]
    fn the_english_catalog_holds_the_reference_texts() {
        let catalog = english();
        assert_eq!(catalog.tag(), ENGLISH_TAG);
        assert_eq!(catalog.name(), "English");
        assert_eq!(catalog.plural_rule(), PluralRule::OneOther);
        assert!(ptr::eq(catalog.schema(), &SCHEMA));
        assert_eq!(catalog.text(common::SAVE), "Save");
        assert_eq!(catalog.text(common::CANCEL), "Cancel");
        assert_eq!(
            catalog.text(host_settings::LANGUAGE_AUTO),
            "System language"
        );
        let mut joined = String::new();
        catalog
            .write_list(&mut joined, i18n::ListKind::And, &["A", "B", "C"])
            .expect("a string takes the list");
        assert_eq!(joined, "A, B and C");
    }

    /// Verifies the digits of a number.
    #[test]
    fn a_number_text_holds_the_decimal_digits() {
        for number in [0, 7, 10, 1234, usize::MAX] {
            assert_eq!(NumberText::new(number).as_str(), number.to_string());
        }
    }

    /// Verifies that the English selection gives the embedded catalog.
    #[test]
    fn the_english_selection_borrows_the_embedded_catalog() {
        assert!(ptr::eq(LanguageCatalog::English.catalog(), english()));
    }
}
