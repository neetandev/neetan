//! The language files of the repository: their sources, their catalogs and the tests of their
//! keys and placeholders against the reference file.

use std::{
    path::{Path, PathBuf},
    sync::LazyLock,
};

use i18n::{Catalog, Fault};

use super::{ENGLISH_TAG, LanguageCatalog, REFERENCE_SOURCE, SCHEMA, english, note};
use crate::config::{LANGUAGE_FOLDER, find_language_files};

/// The language tag of the Japanese language file of the repository.
const JAPANESE_TAG: &str = "ja";

/// The language tag of the German language file of the repository.
const GERMAN_TAG: &str = "de";

/// The language tag of the French language file of the repository.
const FRENCH_TAG: &str = "fr";

/// The language tag of the Spanish language file of the repository.
const SPANISH_TAG: &str = "es";

/// The language tag of the Traditional Chinese (Taiwan) language file of the repository.
const TRADITIONAL_CHINESE_TAG: &str = "zh-TW";

/// A language of the repository: the embedded English file or a file of the language folder.
pub(crate) struct RepositoryLanguage {
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
    for tag in [
        JAPANESE_TAG,
        GERMAN_TAG,
        FRENCH_TAG,
        SPANISH_TAG,
        TRADITIONAL_CHINESE_TAG,
    ] {
        assert!(
            languages.iter().any(|language| language.tag == tag),
            "the language folder of the repository holds {tag}.ini"
        );
    }
    languages
});

/// Returns every language of the repository: English first, then the language files in the
/// order of their tags.
pub(crate) fn repository_languages() -> &'static [RepositoryLanguage] {
    &REPOSITORY_LANGUAGES
}

impl RepositoryLanguage {
    /// Returns a catalog of the language that the caller owns. Each call parses the file again.
    /// The English file gives the embedded catalog. A fault of another file panics.
    pub(crate) fn catalog(&self) -> LanguageCatalog {
        if self.tag == ENGLISH_TAG {
            return LanguageCatalog::English;
        }
        let report = |faults: &[Fault<'_>]| {
            faults
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        };
        let loaded = Catalog::load(&self.tag, &self.source, &SCHEMA, english())
            .unwrap_or_else(|faults| panic!("{}.ini does not load: {}", self.tag, report(&faults)));
        assert!(
            loaded.faults.is_empty(),
            "{}.ini has faults: {}",
            self.tag,
            report(&loaded.faults)
        );
        LanguageCatalog::Loaded(Box::new(loaded.catalog))
    }
}

/// The catalogs of every language of the repository, built on first use.
static REPOSITORY_CATALOGS: LazyLock<Vec<LanguageCatalog>> = LazyLock::new(|| {
    repository_languages()
        .iter()
        .map(RepositoryLanguage::catalog)
        .collect()
});

/// Returns the catalogs of every language of the repository: English first, then the language
/// files in the order of their tags.
pub(crate) fn repository_catalogs() -> impl Iterator<Item = &'static Catalog> {
    REPOSITORY_CATALOGS.iter().map(LanguageCatalog::catalog)
}

/// Returns the shared catalog of the repository language `tag`, built once.
fn shared_catalog(tag: &str) -> &'static Catalog {
    repository_catalogs()
        .find(|catalog| catalog.tag() == tag)
        .unwrap_or_else(|| panic!("the repository holds {tag}.ini"))
}

/// Returns the shared catalog of the Japanese language file of the repository, built once.
pub(crate) fn japanese() -> &'static Catalog {
    shared_catalog(JAPANESE_TAG)
}

/// Returns the shared catalog of the German language file of the repository, built once.
pub(crate) fn german() -> &'static Catalog {
    shared_catalog(GERMAN_TAG)
}

/// Returns the shared catalog of the French language file of the repository, built once.
pub(crate) fn french() -> &'static Catalog {
    shared_catalog(FRENCH_TAG)
}

/// Returns the shared catalog of the Spanish language file of the repository, built once.
pub(crate) fn spanish() -> &'static Catalog {
    shared_catalog(SPANISH_TAG)
}

/// Returns the shared catalog of the Traditional Chinese (Taiwan) language file of the
/// repository, built once.
pub(crate) fn traditional_chinese() -> &'static Catalog {
    shared_catalog(TRADITIONAL_CHINESE_TAG)
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
    use std::{
        collections::{BTreeMap, BTreeSet},
        ops::RangeInclusive,
    };

    use i18n::{ListKind, PluralCategory, PluralRule, read_header};

    use super::*;
    use crate::{config::LANGUAGE_FILE_EXTENSION, texts::library};

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

    /// Returns the game count text of `count` games in `catalog`.
    fn game_count(catalog: &Catalog, count: u64) -> String {
        catalog.display(library::games(count)).to_string()
    }

    /// Returns the names `A`, `B` and `C` joined with the list pattern `kind` of `catalog`.
    fn joined(catalog: &Catalog, kind: ListKind) -> String {
        let mut joined = String::new();
        catalog
            .write_list(&mut joined, kind, &["A", "B", "C"])
            .expect("a string takes the list");
        joined
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

    /// Verifies the header, the sentence mark and the plural forms of the German language file.
    #[test]
    fn the_german_file_has_its_header() {
        let catalog = german();
        assert_eq!(catalog.tag(), GERMAN_TAG);
        assert_eq!(catalog.name(), "Deutsch");
        assert_eq!(catalog.plural_rule(), PluralRule::OneOther);
        assert_eq!(sentence_mark(catalog), '.');
        assert_eq!(game_count(catalog, 1), "1 Spiel");
        assert_eq!(game_count(catalog, 2), "2 Spiele");
        assert_eq!(joined(catalog, ListKind::Or), "A, B oder C");
    }

    /// Verifies the header, the sentence mark, the plural forms and the list patterns of the
    /// French language file. The counts 0 and 1 take the singular.
    #[test]
    fn the_french_file_has_its_header() {
        let catalog = french();
        assert_eq!(catalog.tag(), FRENCH_TAG);
        assert_eq!(catalog.name(), "Français");
        assert_eq!(catalog.plural_rule(), PluralRule::OneBelowTwo);
        assert_eq!(sentence_mark(catalog), '.');
        assert_eq!(game_count(catalog, 0), "0 jeu");
        assert_eq!(game_count(catalog, 1), "1 jeu");
        assert_eq!(game_count(catalog, 2), "2 jeux");
        assert_eq!(joined(catalog, ListKind::And), "A, B et C");
    }

    /// Verifies the header, the sentence mark, the plural forms and the list patterns of the
    /// Spanish language file. The count 0 takes the plural.
    #[test]
    fn the_spanish_file_has_its_header() {
        let catalog = spanish();
        assert_eq!(catalog.tag(), SPANISH_TAG);
        assert_eq!(catalog.name(), "Español");
        assert_eq!(catalog.plural_rule(), PluralRule::OneOther);
        assert_eq!(sentence_mark(catalog), '.');
        assert_eq!(game_count(catalog, 0), "0 juegos");
        assert_eq!(game_count(catalog, 1), "1 juego");
        assert_eq!(game_count(catalog, 2), "2 juegos");
        assert_eq!(joined(catalog, ListKind::And), "A, B y C");
        assert_eq!(joined(catalog, ListKind::Or), "A, B o C");
    }

    /// Verifies the header, the covered tags, the sentence mark, the game counts and the list
    /// patterns of the Traditional Chinese (Taiwan) language file.
    #[test]
    fn the_traditional_chinese_file_has_its_header() {
        let catalog = traditional_chinese();
        assert_eq!(catalog.tag(), TRADITIONAL_CHINESE_TAG);
        assert_eq!(catalog.name(), "繁體中文（台灣）");
        assert_eq!(catalog.plural_rule(), PluralRule::None);
        assert_eq!(sentence_mark(catalog), '。');
        assert_eq!(game_count(catalog, 0), "0款遊戲");
        assert_eq!(game_count(catalog, 1), "1款遊戲");
        assert_eq!(game_count(catalog, 2), "2款遊戲");
        assert_eq!(joined(catalog, ListKind::And), "A、B和C");
        assert_eq!(joined(catalog, ListKind::Or), "A、B或C");
        let language = repository_languages()
            .iter()
            .find(|language| language.tag == TRADITIONAL_CHINESE_TAG)
            .expect("the repository holds zh-TW.ini");
        let header = read_header(&language.source).expect("the header of zh-TW.ini reads");
        assert_eq!(
            header.covered_tags().collect::<Vec<_>>(),
            ["zh-HK", "zh-MO", "zh-Hant"]
        );
    }

    /// The ranges of the combining marks of the scripts of the language files: the combining
    /// diacritical marks with their extensions and supplement, the combining marks for symbols
    /// and the half marks, and the combining voiced sound marks of kana.
    const COMBINING_MARKS: [RangeInclusive<char>; 6] = [
        '\u{0300}'..='\u{036F}',
        '\u{1AB0}'..='\u{1AFF}',
        '\u{1DC0}'..='\u{1DFF}',
        '\u{20D0}'..='\u{20FF}',
        '\u{FE20}'..='\u{FE2F}',
        '\u{3099}'..='\u{309A}',
    ];

    /// The ranges of the characters outside of [`COMBINING_MARKS`] that NFC replaces, after the
    /// Unicode 16.0 data: the singletons such as the Greek question mark, the ohm sign, the
    /// Kelvin sign and the angstrom sign, the composition exclusions of the Indic, Tibetan,
    /// Greek and Hebrew scripts and of the musical symbols, and the CJK compatibility
    /// ideographs without the twelve unified ideographs of their block.
    const NFC_REPLACED_CHARACTERS: [RangeInclusive<char>; 71] = [
        '\u{0374}'..='\u{0374}',
        '\u{037E}'..='\u{037E}',
        '\u{0387}'..='\u{0387}',
        '\u{0958}'..='\u{095F}',
        '\u{09DC}'..='\u{09DD}',
        '\u{09DF}'..='\u{09DF}',
        '\u{0A33}'..='\u{0A33}',
        '\u{0A36}'..='\u{0A36}',
        '\u{0A59}'..='\u{0A5B}',
        '\u{0A5E}'..='\u{0A5E}',
        '\u{0B5C}'..='\u{0B5D}',
        '\u{0F43}'..='\u{0F43}',
        '\u{0F4D}'..='\u{0F4D}',
        '\u{0F52}'..='\u{0F52}',
        '\u{0F57}'..='\u{0F57}',
        '\u{0F5C}'..='\u{0F5C}',
        '\u{0F69}'..='\u{0F69}',
        '\u{0F73}'..='\u{0F73}',
        '\u{0F75}'..='\u{0F76}',
        '\u{0F78}'..='\u{0F78}',
        '\u{0F81}'..='\u{0F81}',
        '\u{0F93}'..='\u{0F93}',
        '\u{0F9D}'..='\u{0F9D}',
        '\u{0FA2}'..='\u{0FA2}',
        '\u{0FA7}'..='\u{0FA7}',
        '\u{0FAC}'..='\u{0FAC}',
        '\u{0FB9}'..='\u{0FB9}',
        '\u{1F71}'..='\u{1F71}',
        '\u{1F73}'..='\u{1F73}',
        '\u{1F75}'..='\u{1F75}',
        '\u{1F77}'..='\u{1F77}',
        '\u{1F79}'..='\u{1F79}',
        '\u{1F7B}'..='\u{1F7B}',
        '\u{1F7D}'..='\u{1F7D}',
        '\u{1FBB}'..='\u{1FBB}',
        '\u{1FBE}'..='\u{1FBE}',
        '\u{1FC9}'..='\u{1FC9}',
        '\u{1FCB}'..='\u{1FCB}',
        '\u{1FD3}'..='\u{1FD3}',
        '\u{1FDB}'..='\u{1FDB}',
        '\u{1FE3}'..='\u{1FE3}',
        '\u{1FEB}'..='\u{1FEB}',
        '\u{1FEE}'..='\u{1FEF}',
        '\u{1FF9}'..='\u{1FF9}',
        '\u{1FFB}'..='\u{1FFB}',
        '\u{1FFD}'..='\u{1FFD}',
        '\u{2000}'..='\u{2001}',
        '\u{2126}'..='\u{2126}',
        '\u{212A}'..='\u{212B}',
        '\u{2329}'..='\u{232A}',
        '\u{2ADC}'..='\u{2ADC}',
        '\u{F900}'..='\u{FA0D}',
        '\u{FA10}'..='\u{FA10}',
        '\u{FA12}'..='\u{FA12}',
        '\u{FA15}'..='\u{FA1E}',
        '\u{FA20}'..='\u{FA20}',
        '\u{FA22}'..='\u{FA22}',
        '\u{FA25}'..='\u{FA26}',
        '\u{FA2A}'..='\u{FA6D}',
        '\u{FA70}'..='\u{FAD9}',
        '\u{FB1D}'..='\u{FB1D}',
        '\u{FB1F}'..='\u{FB1F}',
        '\u{FB2A}'..='\u{FB36}',
        '\u{FB38}'..='\u{FB3C}',
        '\u{FB3E}'..='\u{FB3E}',
        '\u{FB40}'..='\u{FB41}',
        '\u{FB43}'..='\u{FB44}',
        '\u{FB46}'..='\u{FB4E}',
        '\u{1D15E}'..='\u{1D164}',
        '\u{1D1BB}'..='\u{1D1C0}',
        '\u{2F800}'..='\u{2FA1D}',
    ];

    /// Returns true for a combining mark or a character that NFC replaces.
    fn is_combining_mark_or_nfc_replaced(character: char) -> bool {
        COMBINING_MARKS
            .iter()
            .chain(&NFC_REPLACED_CHARACTERS)
            .any(|characters| characters.contains(&character))
    }

    /// Verifies that no language file of the repository holds a combining mark or a character
    /// that NFC replaces.
    ///
    /// Every text holds precomposed characters (Unicode NFC). The check itself finds the mark
    /// of a decomposed letter, a singleton and a compatibility ideograph, and it passes a
    /// unified ideograph of the compatibility block.
    #[test]
    fn no_language_file_holds_a_combining_mark_or_a_character_that_nfc_replaces() {
        for language in repository_languages() {
            for (number, line) in language.source.lines().enumerate() {
                let faulty: Vec<char> = line
                    .chars()
                    .filter(|&character| is_combining_mark_or_nfc_replaced(character))
                    .collect();
                assert!(
                    faulty.is_empty(),
                    "{}.ini line {}: {faulty:?}",
                    language.tag,
                    number + 1
                );
            }
        }
        for character in ['\u{0308}', '\u{037E}', '\u{212B}', '\u{F91D}', '\u{2F800}'] {
            assert!(
                is_combining_mark_or_nfc_replaced(character),
                "{character:?}"
            );
        }
        for character in ['ü', ';', 'Å', '\u{FA0E}', '\u{FA29}', '漢'] {
            assert!(
                !is_combining_mark_or_nfc_replaced(character),
                "{character:?}"
            );
        }
    }
}
