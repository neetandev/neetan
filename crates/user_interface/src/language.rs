//! The language of the user interface: the selection of the language file for the setting
//! `gui-language`, the load of its catalog and the names of the languages of the language files.
//!
//! A language tag tries its forms in order: the tag itself, the language with the script and the
//! region, the language with the script, the language with the region and the language alone.
//! The script decides the writing system and comes before the region. Each form takes the file
//! of its tag and then the files whose `covers` header holds it.

use std::path::PathBuf;

use common::{info, warn};
use i18n::{Catalog, Fault, read_header};
use machine_factory::config::GuiLanguage;
use sdl3::locale::Locale;

use crate::{
    config::{LanguageFile, find_language_files},
    texts::{ENGLISH_TAG, LanguageCatalog, SCHEMA, english},
};

/// The language that a selection gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Selection<'a> {
    /// The embedded English texts.
    English,
    /// A language file.
    File(&'a LanguageOption),
}

impl Selection<'_> {
    /// Returns the language tag of the selection.
    fn tag(&self) -> &str {
        match self {
            Self::English => ENGLISH_TAG,
            Self::File(option) => &option.file.tag,
        }
    }
}

/// A language file with the name of its language and the further language tags of its header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LanguageOption {
    /// The language file.
    pub(crate) file: LanguageFile,
    /// The name of the language in the language itself. A file that does not read or whose
    /// header fails has none. Its catalog does not load.
    pub(crate) name: Option<String>,
    /// The further language tags of the `covers` header of the file. A file that does not read
    /// or whose header fails has none.
    pub(crate) covers: Vec<String>,
}

/// The length of a script part of a language tag, such as `Hant`.
const SCRIPT_LENGTH: usize = 4;

/// The length of a region part of letters, such as `TW`.
const LETTER_REGION_LENGTH: usize = 2;

/// The length of a region part of digits, such as `419`.
const DIGIT_REGION_LENGTH: usize = 3;

/// Returns whether `part` of a language tag is a script: four ASCII letters.
fn is_script(part: &str) -> bool {
    part.len() == SCRIPT_LENGTH && part.bytes().all(|byte| byte.is_ascii_alphabetic())
}

/// Returns whether `part` of a language tag is a region: two ASCII letters or three digits.
fn is_region(part: &str) -> bool {
    (part.len() == LETTER_REGION_LENGTH && part.bytes().all(|byte| byte.is_ascii_alphabetic()))
        || (part.len() == DIGIT_REGION_LENGTH && part.bytes().all(|byte| byte.is_ascii_digit()))
}

/// Returns the forms of the language tag `tag` in the order of the match: the tag itself, the
/// language with the script and the region, the language with the script, the language with
/// the region, and the language alone. Each form appears once, compared without regard to case.
fn tag_forms(tag: &str) -> Vec<String> {
    let mut parts = tag.split('-');
    let language = parts.next().unwrap_or(tag);
    let mut script = None;
    let mut region = None;
    for part in parts {
        if script.is_none() && region.is_none() && is_script(part) {
            script = Some(part);
        } else if region.is_none() && is_region(part) {
            region = Some(part);
        }
    }
    let mut forms = vec![tag.to_owned()];
    let mut add = |form: String| {
        if !forms.iter().any(|known| known.eq_ignore_ascii_case(&form)) {
            forms.push(form);
        }
    };
    if let (Some(script), Some(region)) = (script, region) {
        add(format!("{language}-{script}-{region}"));
    }
    if let Some(script) = script {
        add(format!("{language}-{script}"));
    }
    if let Some(region) = region {
        add(format!("{language}-{region}"));
    }
    add(language.to_owned());
    forms
}

/// Returns the language tag of a preferred locale: the language and the country part joined
/// by `-`. SDL gives the last part of a tag as the country, which is a script for `zh-Hant`.
fn locale_tag(locale: &Locale) -> String {
    match &locale.country {
        Some(country) => format!("{}-{country}", locale.language),
        None => locale.language.clone(),
    }
}

/// Returns the languages of the form `form` among `files` in order, compared without regard to
/// case: English for `en`, the file of the tag `form`, then the files that cover `form`.
fn form_matches<'a>(files: &'a [LanguageOption], form: &str) -> Vec<Selection<'a>> {
    let english = form
        .eq_ignore_ascii_case(ENGLISH_TAG)
        .then_some(Selection::English);
    let own = files
        .iter()
        .filter(|option| option.file.tag.eq_ignore_ascii_case(form));
    let covering = files.iter().filter(|option| {
        option
            .covers
            .iter()
            .any(|covered| covered.eq_ignore_ascii_case(form))
    });
    english
        .into_iter()
        .chain(own.chain(covering).map(Selection::File))
        .collect()
}

/// Returns the language of the explicit tag `tag` among `files`, compared without regard to
/// case: English for `en`, the file of the tag, or the first file that covers the tag.
pub(crate) fn tag_match<'a>(files: &'a [LanguageOption], tag: &str) -> Option<Selection<'a>> {
    form_matches(files, tag).first().copied()
}

/// Returns the languages that the forms `forms` give among `files` in order.
fn forms_matches<'a>(files: &'a [LanguageOption], forms: &[String]) -> Vec<Selection<'a>> {
    forms
        .iter()
        .flat_map(|form| form_matches(files, form))
        .collect()
}

/// Returns the languages that `auto` tries for the preferred `locales` in order: the matches of
/// the forms of the tag of each locale.
fn auto_candidates<'a>(
    locales: &'a [Locale],
    files: &'a [LanguageOption],
) -> impl Iterator<Item = Selection<'a>> {
    locales
        .iter()
        .flat_map(move |locale| forms_matches(files, &tag_forms(&locale_tag(locale))))
}

/// Selects the language of `language` among `files`.
///
/// `auto` walks the preferred `locales`, and the first match of the tag of a locale wins. A tag
/// takes the file of the tag or the first file that covers it. A tag without such a file falls
/// back to the matches of its shorter forms and then to English, with a warning.
pub(crate) fn select<'a>(
    language: &GuiLanguage,
    locales: &'a [Locale],
    files: &'a [LanguageOption],
) -> Selection<'a> {
    match language {
        GuiLanguage::Auto => auto_candidates(locales, files)
            .next()
            .unwrap_or(Selection::English),
        GuiLanguage::Tag(tag) => {
            if let Some(found) = tag_match(files, tag) {
                return found;
            }
            let fallback = forms_matches(files, &tag_forms(tag)[1..])
                .first()
                .copied()
                .unwrap_or(Selection::English);
            warn!(
                "No language file exists for the GUI language {tag}, the user interface takes {}",
                fallback.tag()
            );
            fallback
        }
    }
}

/// Returns the faults of a language file as one line.
fn fault_line(faults: &[Fault<'_>]) -> String {
    faults
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

/// Loads the catalog of the language file `file` with the English texts as the fallback.
///
/// The load logs each fault of the file with its path and its line. A file that fails to read
/// or to load gives `None` with one warning.
pub(crate) fn load(file: &LanguageFile) -> Option<LanguageCatalog> {
    let path = file.path.display();
    let source = std::fs::read_to_string(&file.path)
        .map_err(|error| warn!("Failed to read the language file {path}: {error}"))
        .ok()?;
    match Catalog::load(&file.tag, &source, &SCHEMA, english()) {
        Ok(loaded) => {
            for fault in &loaded.faults {
                warn!("{path}: {fault}");
            }
            info!("GUI language: {} ({path})", loaded.catalog.name());
            Some(LanguageCatalog::Loaded(Box::new(loaded.catalog)))
        }
        Err(faults) => {
            warn!(
                "The language file {path} does not load: {}",
                fault_line(&faults)
            );
            None
        }
    }
}

/// Returns the embedded English catalog and logs the selection.
fn english_catalog() -> LanguageCatalog {
    info!("GUI language: English");
    LanguageCatalog::English
}

/// Returns the catalog of `language` among `files` for the preferred `locales`.
///
/// `auto` tries the languages of the locales in order and takes the first file that loads.
/// A language tag takes the file of [`select`]. A file that does not load gives English.
pub(crate) fn catalog_for(
    language: &GuiLanguage,
    locales: &[Locale],
    files: &[LanguageOption],
) -> LanguageCatalog {
    match language {
        GuiLanguage::Auto => {
            for candidate in auto_candidates(locales, files) {
                match candidate {
                    Selection::English => return english_catalog(),
                    Selection::File(option) => {
                        if let Some(catalog) = load(&option.file) {
                            return catalog;
                        }
                    }
                }
            }
            english_catalog()
        }
        GuiLanguage::Tag(_) => match select(language, locales, files) {
            Selection::English => english_catalog(),
            Selection::File(option) => load(&option.file).unwrap_or_else(english_catalog),
        },
    }
}

/// Returns the catalog of `language` with the language files of `folders`.
///
/// `auto` asks the system for its preferred locales.
pub(crate) fn catalog_of(language: &GuiLanguage, folders: &[PathBuf]) -> LanguageCatalog {
    let files = language_options(find_language_files(folders));
    let locales = match language {
        GuiLanguage::Auto => sdl3::locale::preferred_locales(),
        GuiLanguage::Tag(_) => Vec::new(),
    };
    catalog_for(language, &locales, &files)
}

/// Returns the languages of `files` in the order of the files. One read of the header of each
/// file gives the name of its language and its further language tags. A file that does not
/// read or whose header fails keeps its tag without a name and without further tags, with a
/// warning.
pub(crate) fn language_options(files: Vec<LanguageFile>) -> Vec<LanguageOption> {
    files
        .into_iter()
        .map(|file| {
            let (name, covers) = header_texts(&file).unzip();
            LanguageOption {
                file,
                name,
                covers: covers.unwrap_or_default(),
            }
        })
        .collect()
}

/// Returns the name of the language and the further language tags in the header of `file`. A
/// file that does not read or whose header fails gives `None` with a warning.
fn header_texts(file: &LanguageFile) -> Option<(String, Vec<String>)> {
    let path = file.path.display();
    let source = std::fs::read_to_string(&file.path)
        .map_err(|error| warn!("Failed to read the language file {path}: {error}"))
        .ok()?;
    match read_header(&source) {
        Ok(header) => {
            let covers = header.covered_tags().map(str::to_owned).collect();
            Some((header.name.into_owned(), covers))
        }
        Err(faults) => {
            warn!(
                "The language file {path} has no valid header: {}",
                fault_line(&faults)
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use common::temporary::TemporaryFolder;

    use super::*;
    use crate::texts::{self, repository::repository_language_folder};

    /// A language file of `tag` below `/languages` with the name `tag` that covers the tags
    /// `covers`.
    fn covering(tag: &str, covers: &[&str]) -> LanguageOption {
        LanguageOption {
            file: LanguageFile {
                tag: tag.to_owned(),
                path: PathBuf::from(format!("/languages/{tag}.ini")),
            },
            name: Some(tag.to_owned()),
            covers: covers.iter().map(|&covered| covered.to_owned()).collect(),
        }
    }

    /// A language file of `tag` below `/languages` without further tags.
    fn file(tag: &str) -> LanguageOption {
        covering(tag, &[])
    }

    /// A preferred locale of `language` and the optional `country`.
    fn locale(language: &str, country: Option<&str>) -> Locale {
        Locale {
            language: language.to_owned(),
            country: country.map(str::to_owned),
        }
    }

    /// Returns the tag that `language` selects among `files` for `locales`.
    fn selected(language: &str, locales: &[Locale], files: &[LanguageOption]) -> String {
        let language: GuiLanguage = language.parse().expect("a valid language");
        select(&language, locales, files).tag().to_owned()
    }

    /// The sections of a German language file with two texts and a key that the schema lacks.
    const GERMAN: &str = "[common]\nsave = Speichern\ncancel = Abbrechen\nunknown = Unbekannt\n";

    /// Verifies that an explicit tag finds its file without regard to case and that `en` gives English.
    #[test]
    fn an_explicit_tag_takes_its_file() {
        let files = [file("de"), file("ja"), file("pt-BR")];
        assert_eq!(selected("ja", &[], &files), "ja");
        assert_eq!(selected("JA", &[], &files), "ja");
        assert_eq!(selected("pt-br", &[], &files), "pt-BR");
        assert_eq!(selected("en", &[], &files), "en");
        assert_eq!(selected("EN", &[], &[]), "en");
    }

    /// Verifies that a tag without a file falls back to its language part and then to English.
    #[test]
    fn a_missing_tag_falls_back_to_its_language_and_then_to_english() {
        let files = [file("ja"), file("pt-BR")];
        assert_eq!(selected("ja-JP", &[], &files), "ja");
        assert_eq!(selected("pt-PT", &[], &files), "en");
        assert_eq!(selected("pt", &[], &files), "en");
        assert_eq!(selected("fr", &[], &files), "en");
        assert_eq!(selected("en-GB", &[], &files), "en");
    }

    /// Verifies that auto prefers the regional file of a locale over its plain file.
    #[test]
    fn auto_prefers_the_regional_file_of_a_locale() {
        let files = [file("pt"), file("pt-BR"), file("ja")];
        let brazil = [locale("pt", Some("BR"))];
        assert_eq!(selected("auto", &brazil, &files), "pt-BR");
        let portugal = [locale("pt", Some("PT"))];
        assert_eq!(selected("auto", &portugal, &files), "pt");
        let lower = [locale("PT", Some("br"))];
        assert_eq!(selected("auto", &lower, &files), "pt-BR");
        let plain = [locale("ja", None)];
        assert_eq!(selected("auto", &plain, &files), "ja");
    }

    /// Verifies that auto takes the first locale with a file, English included, and English without a match.
    #[test]
    fn auto_takes_the_first_locale_with_a_file() {
        let files = [file("ja"), file("de")];
        let locales = [
            locale("fr", Some("FR")),
            locale("ja", Some("JP")),
            locale("de", None),
        ];
        assert_eq!(selected("auto", &locales, &files), "ja");
        let english_first = [locale("en", Some("US")), locale("ja", None)];
        assert_eq!(selected("auto", &english_first, &files), "en");
        assert_eq!(selected("auto", &[locale("fr", None)], &files), "en");
        assert_eq!(selected("auto", &[], &files), "en");
        assert_eq!(selected("auto", &locales, &[]), "en");
    }

    /// The language files of the Chinese tests: Japanese, Traditional Chinese for Taiwan that
    /// covers Hong Kong, Macao and the script `Hant`, and Portuguese for Brazil.
    fn chinese_files() -> [LanguageOption; 3] {
        [
            file("ja"),
            covering("zh-TW", &["zh-HK", "zh-MO", "zh-Hant"]),
            file("pt-BR"),
        ]
    }

    /// Verifies the forms of a tag: the tag, the tag without its region, the tag without its
    /// script and the language.
    #[test]
    fn a_tag_tries_its_shorter_forms() {
        assert_eq!(
            tag_forms("zh-Hant-TW"),
            ["zh-Hant-TW", "zh-Hant", "zh-TW", "zh"]
        );
        assert_eq!(tag_forms("zh-Hant"), ["zh-Hant", "zh"]);
        assert_eq!(tag_forms("pt-BR"), ["pt-BR", "pt"]);
        assert_eq!(tag_forms("es-419"), ["es-419", "es"]);
        assert_eq!(tag_forms("ja"), ["ja"]);
        assert_eq!(
            tag_forms("ca-ES-valencia"),
            ["ca-ES-valencia", "ca-ES", "ca"]
        );
        assert_eq!(
            tag_forms("sr-Latn-RS-x"),
            ["sr-Latn-RS-x", "sr-Latn-RS", "sr-Latn", "sr-RS", "sr"]
        );
    }

    /// Verifies the tag of each locale form that SDL gives on Windows, macOS and Linux.
    #[test]
    fn a_locale_gives_its_tag() {
        assert_eq!(locale_tag(&locale("zh", Some("TW"))), "zh-TW");
        assert_eq!(locale_tag(&locale("zh-Hant", Some("TW"))), "zh-Hant-TW");
        assert_eq!(locale_tag(&locale("zh", Some("Hant"))), "zh-Hant");
        assert_eq!(locale_tag(&locale("ja", None)), "ja");
    }

    /// Verifies that auto takes the file of a tag and then the file that covers it, also for
    /// the script forms of the locales.
    #[test]
    fn auto_takes_the_file_that_covers_a_locale() {
        let files = chinese_files();
        for (language, country) in [
            ("zh", Some("TW")),
            ("zh", Some("HK")),
            ("zh", Some("MO")),
            ("zh-Hant", Some("TW")),
            ("zh-Hant", Some("HK")),
            ("zh", Some("Hant")),
            ("ZH", Some("hk")),
        ] {
            assert_eq!(
                selected("auto", &[locale(language, country)], &files),
                "zh-TW",
                "{language} {country:?}"
            );
        }
        for (language, country) in [("zh", None), ("zh", Some("CN")), ("zh-Hans", Some("CN"))] {
            assert_eq!(
                selected("auto", &[locale(language, country)], &files),
                "en",
                "{language} {country:?}"
            );
        }
        let chinese_later = [locale("zh", Some("CN")), locale("ja", Some("JP"))];
        assert_eq!(selected("auto", &chinese_later, &files), "ja");
    }

    /// Verifies that the script of a locale wins over its region: Simplified Chinese for Hong
    /// Kong takes the file that covers `zh-Hans` and Traditional Chinese for China takes the file
    /// that covers `zh-Hant`.
    #[test]
    fn the_script_of_a_locale_wins_over_its_region() {
        let files = [
            covering("zh-TW", &["zh-HK", "zh-Hant"]),
            covering("zh-CN", &["zh-Hans"]),
        ];
        let simplified_hong_kong = [locale("zh-Hans", Some("HK"))];
        assert_eq!(selected("auto", &simplified_hong_kong, &files), "zh-CN");
        let traditional_china = [locale("zh-Hant", Some("CN"))];
        assert_eq!(selected("auto", &traditional_china, &files), "zh-TW");
        assert_eq!(selected("zh-Hans-HK", &[], &files), "zh-CN");
    }

    /// Verifies that the file of a tag wins over a file that covers the tag, and a covering
    /// file of the full tag over the file of the language.
    #[test]
    fn the_own_file_of_a_tag_wins_over_a_covering_file() {
        let files = [
            covering("zh-TW", &["zh-HK", "zh-Hant"]),
            file("zh-HK"),
            covering("zh-CN", &["zh", "zh-Hans"]),
        ];
        let hong_kong = [locale("zh", Some("HK"))];
        assert_eq!(selected("auto", &hong_kong, &files), "zh-HK");
        assert_eq!(selected("zh-HK", &[], &files), "zh-HK");
        let macao = [locale("zh-Hant", Some("MO"))];
        assert_eq!(selected("auto", &macao, &files), "zh-TW");
        let singapore = [locale("zh", Some("SG"))];
        assert_eq!(selected("auto", &singapore, &files), "zh-CN");
    }

    /// Verifies that an explicit tag takes the file that covers it and falls back through the
    /// shorter forms of the tag.
    #[test]
    fn an_explicit_tag_takes_the_file_that_covers_it() {
        let files = chinese_files();
        assert_eq!(selected("zh-HK", &[], &files), "zh-TW");
        assert_eq!(selected("zh-mo", &[], &files), "zh-TW");
        assert_eq!(selected("zh-Hant", &[], &files), "zh-TW");
        assert_eq!(selected("zh-Hant-TW", &[], &files), "zh-TW");
        assert_eq!(selected("zh-Hant-HK", &[], &files), "zh-TW");
        assert_eq!(selected("zh-TW", &[], &files), "zh-TW");
        assert_eq!(selected("zh-CN", &[], &files), "en");
        assert_eq!(selected("zh", &[], &files), "en");
        assert_eq!(selected("pt-PT", &[], &files), "en");
    }

    /// Verifies that the Chinese files of the repository serve their locales and tags: the
    /// Simplified Chinese file serves `zh`, mainland China, Singapore and the script `Hans`, the
    /// Traditional Chinese file serves Taiwan, Hong Kong and the script `Hant`, and the first
    /// preferred locale decides between them.
    #[test]
    fn the_chinese_files_of_the_repository_serve_their_locales() {
        let files = language_options(find_language_files(&[repository_language_folder()]));
        let simplified = [
            ("zh", None),
            ("zh", Some("CN")),
            ("zh", Some("SG")),
            ("zh-Hans", Some("CN")),
            ("zh-Hans", Some("SG")),
            ("zh", Some("Hans")),
        ];
        let traditional = [
            ("zh", Some("TW")),
            ("zh", Some("HK")),
            ("zh-Hant", Some("TW")),
            ("zh", Some("Hant")),
        ];
        for (expected, locales) in [("zh-CN", &simplified[..]), ("zh-TW", &traditional[..])] {
            for &(language, country) in locales {
                assert_eq!(
                    selected("auto", &[locale(language, country)], &files),
                    expected,
                    "{language} {country:?}"
                );
            }
        }
        for tag in ["zh", "zh-CN", "zh-SG", "zh-sg", "zh-Hans", "zh-Hans-CN"] {
            assert_eq!(selected(tag, &[], &files), "zh-CN", "{tag}");
        }
        for tag in ["zh-TW", "zh-HK", "zh-Hant-TW"] {
            assert_eq!(selected(tag, &[], &files), "zh-TW", "{tag}");
        }
        let taiwan_first = [locale("zh", Some("TW")), locale("zh", Some("CN"))];
        assert_eq!(selected("auto", &taiwan_first, &files), "zh-TW");
        let china_first = [locale("zh", Some("CN")), locale("zh", Some("TW"))];
        assert_eq!(selected("auto", &china_first, &files), "zh-CN");
    }

    /// Verifies that the further tags come from the headers of the files, and that a file that
    /// does not read or whose header fails covers no tag.
    #[test]
    fn the_covered_tags_come_from_the_headers() {
        let folder = TemporaryFolder::new("language_covers");
        let chinese = folder.write_file(
            "zh-TW.ini",
            "[language]\nname = 繁體中文\nplural = none\ncovers = zh-HK zh-Hant\n".as_bytes(),
        );
        let broken = folder.write_file(
            "ko.ini",
            "[language]\nname = 한국어\nplural = none\ncovers = ko_KP\n".as_bytes(),
        );
        let files = language_options(vec![
            LanguageFile {
                tag: "zh-TW".to_owned(),
                path: chinese,
            },
            LanguageFile {
                tag: "ko".to_owned(),
                path: broken,
            },
            LanguageFile {
                tag: "it".to_owned(),
                path: folder.path().join("it.ini"),
            },
        ]);
        let covers: Vec<(&str, Vec<&str>)> = files
            .iter()
            .map(|option| {
                (
                    option.file.tag.as_str(),
                    option.covers.iter().map(String::as_str).collect(),
                )
            })
            .collect();
        assert_eq!(
            covers,
            [
                ("zh-TW", vec!["zh-HK", "zh-Hant"]),
                ("ko", vec![]),
                ("it", vec![]),
            ]
        );
        assert_eq!(selected("zh-HK", &[], &files), "zh-TW");
        assert_eq!(selected("ko-KP", &[], &files), "ko");
    }

    /// Verifies that a language file loads over the English texts and keeps the English text of a missing key.
    #[test]
    fn a_language_file_loads_over_the_english_texts() {
        let folder = TemporaryFolder::new("language_load");
        let path = folder.write_file("de.ini", texts::test_language(GERMAN).as_bytes());
        let loaded = load(&LanguageFile {
            tag: "de".to_owned(),
            path,
        });
        let Some(LanguageCatalog::Loaded(catalog)) = &loaded else {
            panic!("the German file loads");
        };
        assert_eq!(catalog.tag(), "de");
        assert_eq!(catalog.name(), "Deutsch");
        assert_eq!(catalog.text(texts::common::SAVE), "Speichern");
        assert_eq!(catalog.text(texts::common::BROWSE), "Browse");
    }

    /// Verifies that a file that fails to read or to load gives no catalog.
    #[test]
    fn a_broken_language_file_does_not_load() {
        let folder = TemporaryFolder::new("language_broken");
        let missing = LanguageFile {
            tag: "de".to_owned(),
            path: folder.path().join("de.ini"),
        };
        assert!(load(&missing).is_none());
        let headless = LanguageFile {
            tag: "fr".to_owned(),
            path: folder.write_file("fr.ini", b"[common]\nsave = Enregistrer\n"),
        };
        assert!(load(&headless).is_none());
    }

    /// Verifies that auto takes the next locale with a file when a file does not load, and that a
    /// language tag of a file that does not load gives English.
    #[test]
    fn auto_skips_a_language_file_that_does_not_load() {
        let folder = TemporaryFolder::new("language_auto_broken");
        let japanese = folder.write_file("ja.ini", b"[language]\nplural = none\n");
        let german = folder.write_file("de.ini", texts::test_language(GERMAN).as_bytes());
        let files = language_options(vec![
            LanguageFile {
                tag: "ja".to_owned(),
                path: japanese,
            },
            LanguageFile {
                tag: "de".to_owned(),
                path: german,
            },
        ]);
        let locales = [locale("ja", Some("JP")), locale("de", None)];
        let catalog = catalog_for(&GuiLanguage::Auto, &locales, &files);
        assert_eq!(catalog.catalog().tag(), "de");
        assert_eq!(catalog.catalog().text(texts::common::SAVE), "Speichern");
        let english_first = [locale("ja", None), locale("en", None), locale("de", None)];
        let catalog = catalog_for(&GuiLanguage::Auto, &english_first, &files);
        assert!(matches!(catalog, LanguageCatalog::English));
        let explicit = catalog_for(&GuiLanguage::Tag("ja".to_owned()), &locales, &files);
        assert!(matches!(explicit, LanguageCatalog::English));
    }

    /// Verifies that an explicit tag of a folder loads its file and a missing tag gives English.
    #[test]
    fn the_catalog_of_a_language_comes_from_the_folders() {
        let folder = TemporaryFolder::new("language_catalog");
        folder.write_file("de.ini", texts::test_language(GERMAN).as_bytes());
        let folders = [folder.path().to_path_buf()];
        let german = catalog_of(&GuiLanguage::Tag("DE".to_owned()), &folders);
        assert_eq!(german.catalog().text(texts::common::SAVE), "Speichern");
        let french = catalog_of(&GuiLanguage::Tag("fr".to_owned()), &folders);
        assert!(matches!(french, LanguageCatalog::English));
    }

    /// Verifies that the options take the names of the headers in the order of the files, and
    /// keep a file that does not read or whose header fails without a name.
    #[test]
    fn the_options_take_the_names_of_the_headers() {
        let folder = TemporaryFolder::new("language_options");
        let japanese = folder.write_file(
            "ja.ini",
            "[language]\nname = 日本語\nplural = none\n".as_bytes(),
        );
        let german = folder.write_file("de.ini", texts::test_language(GERMAN).as_bytes());
        let broken = folder.write_file("fr.ini", b"[language]\nname =\nplural = one-other\n");
        let files = vec![
            LanguageFile {
                tag: "ja".to_owned(),
                path: japanese,
            },
            LanguageFile {
                tag: "fr".to_owned(),
                path: broken,
            },
            LanguageFile {
                tag: "de".to_owned(),
                path: german,
            },
            LanguageFile {
                tag: "it".to_owned(),
                path: folder.path().join("it.ini"),
            },
        ];
        let names: Vec<(String, Option<String>)> = language_options(files)
            .into_iter()
            .map(|option| (option.file.tag, option.name))
            .collect();
        let expected = [
            ("ja", Some("日本語")),
            ("fr", None),
            ("de", Some("Deutsch")),
            ("it", None),
        ]
        .map(|(tag, name)| (tag.to_owned(), name.map(str::to_owned)));
        assert_eq!(names, expected);
    }
}
