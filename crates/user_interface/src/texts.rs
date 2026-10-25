//! The texts of the user interface: the generated keys of the reference language file
//! `language/en.ini`, the embedded English catalog and the catalog of the selected language.
//!
//! The build script generates the `SCHEMA` static, one module for each section with a `Key`
//! constant for each fixed text and a function for each text with placeholders.

use std::sync::LazyLock;

use i18n::Catalog;

include!(concat!(env!("OUT_DIR"), "/texts.rs"));

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

    /// Verifies that the English selection gives the embedded catalog.
    #[test]
    fn the_english_selection_borrows_the_embedded_catalog() {
        assert!(ptr::eq(LanguageCatalog::English.catalog(), english()));
    }
}
