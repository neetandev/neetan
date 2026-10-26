//! The language files of the repository: their sources, their catalogs and the tests of their
//! keys and placeholders against the reference file.

use std::{
    path::{Path, PathBuf},
    sync::LazyLock,
};

use i18n::Catalog;

use super::{ENGLISH_TAG, LanguageCatalog, REFERENCE_SOURCE, SCHEMA, english, note};
use crate::config::{LANGUAGE_FOLDER, find_language_files};

/// The language tag of the Japanese language file of the repository.
const JAPANESE_TAG: &str = "ja";

/// A language file of the repository.
struct RepositoryLanguage {
    /// The language tag: the file name without its extension.
    tag: String,
    /// The source of the file.
    source: String,
}

/// Returns the language folder of the repository.
fn repository_language_folder() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(LANGUAGE_FOLDER)
}

/// Every language of the repository, read on first use: English with the embedded reference
/// file first, then the files that the language discovery finds in the language folder of the
/// repository in the order of their tags.
static REPOSITORY_LANGUAGES: LazyLock<Vec<RepositoryLanguage>> = LazyLock::new(|| {
    let folder = repository_language_folder();
    let english = RepositoryLanguage {
        tag: ENGLISH_TAG.to_owned(),
        source: REFERENCE_SOURCE.to_owned(),
    };
    let languages: Vec<RepositoryLanguage> = std::iter::once(english)
        .chain(find_language_files(&[folder]).into_iter().map(|file| {
            RepositoryLanguage {
                source: std::fs::read_to_string(&file.path)
                    .unwrap_or_else(|error| panic!("{}: {error}", file.path.display())),
                tag: file.tag,
            }
        }))
        .collect();
    assert!(
        languages
            .iter()
            .any(|language| language.tag == JAPANESE_TAG),
        "the language folder of the repository holds ja.ini"
    );
    languages
});

/// Returns every language of the repository: English first, then the language files in the
/// order of their tags.
fn repository_languages() -> &'static [RepositoryLanguage] {
    &REPOSITORY_LANGUAGES
}

/// Returns the catalog of the repository language file `language`. The English file gives the
/// embedded catalog. A fault of another file panics.
fn repository_catalog(language: &RepositoryLanguage) -> LanguageCatalog {
    if language.tag == ENGLISH_TAG {
        return LanguageCatalog::English;
    }
    let report = |faults: &[i18n::Fault<'_>]| {
        faults
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ")
    };
    let loaded = Catalog::load(&language.tag, &language.source, &SCHEMA, english())
        .unwrap_or_else(|faults| panic!("{}.ini does not load: {}", language.tag, report(&faults)));
    assert!(
        loaded.faults.is_empty(),
        "{}.ini has faults: {}",
        language.tag,
        report(&loaded.faults)
    );
    LanguageCatalog::Loaded(Box::new(loaded.catalog))
}

/// The catalogs of every language of the repository, built on first use.
static REPOSITORY_CATALOGS: LazyLock<Vec<LanguageCatalog>> = LazyLock::new(|| {
    repository_languages()
        .iter()
        .map(repository_catalog)
        .collect()
});

/// Returns the catalogs of every language of the repository: English first, then the language
/// files in the order of their tags.
pub(crate) fn repository_catalogs() -> impl Iterator<Item = &'static Catalog> {
    REPOSITORY_CATALOGS.iter().map(LanguageCatalog::catalog)
}

/// Returns a catalog of the Japanese language file of the repository that the caller owns.
/// Each call parses the file again. A fault of the file panics.
pub(crate) fn japanese_catalog() -> LanguageCatalog {
    let language = repository_languages()
        .iter()
        .find(|language| language.tag == JAPANESE_TAG)
        .expect("the repository holds ja.ini");
    repository_catalog(language)
}

/// Returns the shared catalog of the Japanese language file of the repository, built once.
pub(crate) fn japanese() -> &'static Catalog {
    repository_catalogs()
        .find(|catalog| catalog.tag() == JAPANESE_TAG)
        .expect("the repository holds ja.ini")
}

/// Returns the mark that ends a sentence in `catalog`: the last character of the pattern
/// `note.sentence`.
pub(crate) fn sentence_mark(catalog: &Catalog) -> char {
    catalog
        .display(note::sentence(&""))
        .to_string()
        .chars()
        .last()
        .expect("the sentence pattern ends with a mark")
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use i18n::{PluralCategory, PluralRule};

    use super::*;
    use crate::config::LANGUAGE_FILE_EXTENSION;

    /// The section of the header of a language file.
    const HEADER_SECTION: &str = "language";

    /// Returns the texts of a language file outside its header by their full key
    /// `<section>.<key>`.
    fn file_texts(source: &str) -> BTreeMap<String, &str> {
        let mut section = "";
        let mut texts = BTreeMap::new();
        for line in source.lines().map(str::trim) {
            if line.is_empty() || line.starts_with(';') {
                continue;
            }
            if let Some(name) = line
                .strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
            {
                section = name;
            } else if let Some((key, value)) = line.split_once('=')
                && section != HEADER_SECTION
            {
                texts.insert(format!("{section}.{}", key.trim()), value.trim());
            }
        }
        texts
    }

    /// Returns the placeholder names of the text `text`. The escaped braces `{{` and `}}` are
    /// literal text.
    fn placeholders(text: &str) -> BTreeSet<&str> {
        let mut names = BTreeSet::new();
        let mut rest = text;
        while let Some(start) = rest.find('{') {
            if rest[start + 1..].starts_with('{') {
                rest = &rest[start + 2..];
                continue;
            }
            let end = start + rest[start..].find('}').expect("a placeholder closes");
            names.insert(&rest[start + 1..end]);
            rest = &rest[end + 1..];
        }
        names
    }

    /// Returns whether the full key `full_key` of a file is a form of a plural text of the
    /// schema that the plural rule `rule` lacks.
    fn is_outside_rule(full_key: &str, rule: PluralRule) -> bool {
        let Some((base, category)) = full_key.rsplit_once('.') else {
            return false;
        };
        let Some((section, key)) = base.rsplit_once('.') else {
            return false;
        };
        let plural = SCHEMA
            .find(section, key)
            .is_some_and(|key| SCHEMA.info(key).is_plural());
        plural && PluralCategory::from_name(category).is_some_and(|category| !rule.has(category))
    }

    /// Verifies that every language file of the repository loads without faults, holds every
    /// key of the English file for its plural rule and only those, and that each text uses
    /// exactly the placeholders of its English text.
    #[test]
    fn every_language_file_holds_every_english_key_with_its_placeholders() {
        let english_texts = file_texts(REFERENCE_SOURCE);
        for language in repository_languages() {
            if language.tag == ENGLISH_TAG {
                continue;
            }
            let file = format!("{}.ini", language.tag);
            let loaded = Catalog::load(&language.tag, &language.source, &SCHEMA, english())
                .unwrap_or_else(|faults| panic!("{file} does not load: {faults:?}"));
            assert!(loaded.faults.is_empty(), "{file}: {:?}", loaded.faults);
            let catalog = &loaded.catalog;
            assert_eq!(catalog.tag(), language.tag);
            assert!(!catalog.name().is_empty(), "{file} has no name");

            let texts = file_texts(&language.source);
            let expected: BTreeSet<&String> = english_texts
                .keys()
                .filter(|key| !is_outside_rule(key, catalog.plural_rule()))
                .collect();
            let found: BTreeSet<&String> = texts.keys().collect();
            assert_eq!(
                expected.difference(&found).collect::<Vec<_>>(),
                Vec::<&&String>::new(),
                "keys of en.ini that {file} lacks"
            );
            assert_eq!(
                found.difference(&expected).collect::<Vec<_>>(),
                Vec::<&&String>::new(),
                "keys of {file} that en.ini lacks"
            );
            for (key, text) in &texts {
                assert!(!text.is_empty(), "{file}: {key} is empty");
                assert_eq!(
                    placeholders(text),
                    placeholders(english_texts[key]),
                    "{file}: the placeholders of {key}"
                );
            }
        }
    }

    /// Verifies that the language discovery finds every `*.ini` file of the language folder of
    /// the repository besides `en.ini`. A file without a language tag as its name escapes the
    /// discovery and with it the key test.
    #[test]
    fn the_discovery_finds_every_language_file_of_the_repository() {
        let folder = repository_language_folder();
        let mut files: Vec<String> = std::fs::read_dir(&folder)
            .expect("the language folder reads")
            .map(|entry| entry.expect("the language folder lists").path())
            .filter(|path| {
                path.extension().is_some_and(|extension| {
                    extension.eq_ignore_ascii_case(LANGUAGE_FILE_EXTENSION)
                })
            })
            .filter_map(|path| Some(path.file_name()?.to_string_lossy().into_owned()))
            .collect();
        files.sort();
        let english_file = format!("{ENGLISH_TAG}.{LANGUAGE_FILE_EXTENSION}");
        assert!(files.contains(&english_file), "{files:?}");
        assert_eq!(
            files.len(),
            find_language_files(&[folder]).len() + 1,
            "language files of the repository: {files:?}"
        );
    }

    /// Verifies the header and the sentence mark of the Japanese language file.
    #[test]
    fn the_japanese_file_has_its_header() {
        let catalog = japanese();
        assert_eq!(catalog.tag(), JAPANESE_TAG);
        assert_eq!(catalog.name(), "日本語");
        assert_eq!(catalog.plural_rule(), PluralRule::None);
        assert_eq!(sentence_mark(catalog), '。');
    }
}
