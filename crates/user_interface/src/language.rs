//! The language of the user interface: the selection of the language file for the setting
//! `gui-language`, the load of its catalog and the names of the languages of the language files.

use std::path::PathBuf;

use common::{info, warn};
use i18n::{Catalog, Fault, read_header};
use machine_factory::config::GuiLanguage;
use sdl3::locale::Locale;

use crate::{
    config::{LanguageFile, compare_tags, find_language_files},
    texts::{ENGLISH_TAG, LanguageCatalog, SCHEMA, english},
};

/// The language that a selection gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Selection<'a> {
    /// The embedded English texts.
    English,
    /// A language file.
    File(&'a LanguageFile),
}

impl Selection<'_> {
    /// Returns the language tag of the selection.
    fn tag(&self) -> &str {
        match self {
            Self::English => ENGLISH_TAG,
            Self::File(file) => &file.tag,
        }
    }
}

/// A language file with the name of its language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LanguageOption {
    /// The language tag of the file.
    pub(crate) tag: String,
    /// The name of the language in the language itself. A file that does not read or whose
    /// header fails has none. Its catalog does not load.
    pub(crate) name: Option<String>,
}

/// Returns the language of `tag` among `files`, compared without regard to case. The tag `en`
/// gives the embedded English texts.
fn find<'a>(files: &'a [LanguageFile], tag: &str) -> Option<Selection<'a>> {
    if tag.eq_ignore_ascii_case(ENGLISH_TAG) {
        return Some(Selection::English);
    }
    files
        .iter()
        .find(|file| file.tag.eq_ignore_ascii_case(tag))
        .map(Selection::File)
}

/// Returns the file of the tag `<language>-<country>` among `files`, compared without regard to
/// case.
fn find_regional<'a>(
    files: &'a [LanguageFile],
    language: &str,
    country: &str,
) -> Option<Selection<'a>> {
    files
        .iter()
        .find(|file| {
            file.tag
                .split_once('-')
                .is_some_and(|(file_language, file_country)| {
                    file_language.eq_ignore_ascii_case(language)
                        && file_country.eq_ignore_ascii_case(country)
                })
        })
        .map(Selection::File)
}

/// Returns the languages that `auto` tries for the preferred `locales` in order. A locale
/// gives the file of `<language>-<COUNTRY>` and then the file of `<language>`.
fn auto_candidates<'a>(
    locales: &'a [Locale],
    files: &'a [LanguageFile],
) -> impl Iterator<Item = Selection<'a>> {
    locales.iter().flat_map(move |locale| {
        let regional = locale
            .country
            .as_deref()
            .and_then(|country| find_regional(files, &locale.language, country));
        regional.into_iter().chain(find(files, &locale.language))
    })
}

/// Selects the language of `language` among `files`.
///
/// `auto` walks the preferred `locales`. A locale tries `<language>-<COUNTRY>` and then
/// `<language>`, and the first tag with a file wins. A tag without a file falls back to its
/// language part and then to English, with a warning. Every other case gives English.
pub(crate) fn select<'a>(
    language: &GuiLanguage,
    locales: &'a [Locale],
    files: &'a [LanguageFile],
) -> Selection<'a> {
    match language {
        GuiLanguage::Auto => auto_candidates(locales, files)
            .next()
            .unwrap_or(Selection::English),
        GuiLanguage::Tag(tag) => {
            if let Some(found) = find(files, tag) {
                return found;
            }
            let base = tag.split('-').next().unwrap_or(tag);
            let fallback = if base.len() < tag.len() {
                find(files, base).unwrap_or(Selection::English)
            } else {
                Selection::English
            };
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
    files: &[LanguageFile],
) -> LanguageCatalog {
    match language {
        GuiLanguage::Auto => {
            for candidate in auto_candidates(locales, files) {
                match candidate {
                    Selection::English => return english_catalog(),
                    Selection::File(file) => {
                        if let Some(catalog) = load(file) {
                            return catalog;
                        }
                    }
                }
            }
            english_catalog()
        }
        GuiLanguage::Tag(_) => match select(language, locales, files) {
            Selection::English => english_catalog(),
            Selection::File(file) => load(file).unwrap_or_else(english_catalog),
        },
    }
}

/// Returns the catalog of `language` with the language files of `folders`.
///
/// `auto` asks the system for its preferred locales.
pub(crate) fn catalog_of(language: &GuiLanguage, folders: &[PathBuf]) -> LanguageCatalog {
    let files = find_language_files(folders);
    let locales = match language {
        GuiLanguage::Auto => sdl3::locale::preferred_locales(),
        GuiLanguage::Tag(_) => Vec::new(),
    };
    catalog_for(language, &locales, &files)
}

/// Returns the languages of `files` sorted by tag without regard to case. Each language takes
/// the name of the header of its file. A file that does not read or whose header fails keeps
/// its tag without a name, with a warning.
pub(crate) fn language_options(files: &[LanguageFile]) -> Vec<LanguageOption> {
    let mut options: Vec<LanguageOption> = files
        .iter()
        .map(|file| LanguageOption {
            tag: file.tag.clone(),
            name: header_name(file),
        })
        .collect();
    options.sort_by(|first, second| compare_tags(&first.tag, &second.tag));
    options
}

/// Returns the name of the language in the header of `file`. A file that does not read or
/// whose header fails gives `None` with a warning.
fn header_name(file: &LanguageFile) -> Option<String> {
    let path = file.path.display();
    let source = std::fs::read_to_string(&file.path)
        .map_err(|error| warn!("Failed to read the language file {path}: {error}"))
        .ok()?;
    match read_header(&source) {
        Ok(header) => Some(header.name.into_owned()),
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
    use crate::texts;

    /// A language file of `tag` below `/languages`.
    fn file(tag: &str) -> LanguageFile {
        LanguageFile {
            tag: tag.to_owned(),
            path: PathBuf::from(format!("/languages/{tag}.ini")),
        }
    }

    /// A preferred locale of `language` and the optional `country`.
    fn locale(language: &str, country: Option<&str>) -> Locale {
        Locale {
            language: language.to_owned(),
            country: country.map(str::to_owned),
        }
    }

    /// Returns the tag that `language` selects among `files` for `locales`.
    fn selected(language: &str, locales: &[Locale], files: &[LanguageFile]) -> String {
        let language: GuiLanguage = language.parse().expect("a valid language");
        select(&language, locales, files).tag().to_owned()
    }

    /// A German language file with the header, two texts and a key that the schema lacks.
    const GERMAN: &str = "[language]\nname = Deutsch\nplural = one-other\n\
                          [common]\nsave = Speichern\ncancel = Abbrechen\nunknown = Unbekannt\n";

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

    /// Verifies that a language file loads over the English texts and keeps the English text of a missing key.
    #[test]
    fn a_language_file_loads_over_the_english_texts() {
        let folder = TemporaryFolder::new("language_load");
        let path = folder.write_file("de.ini", GERMAN.as_bytes());
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
        let german = folder.write_file("de.ini", GERMAN.as_bytes());
        let files = [
            LanguageFile {
                tag: "ja".to_owned(),
                path: japanese,
            },
            LanguageFile {
                tag: "de".to_owned(),
                path: german,
            },
        ];
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
        folder.write_file("de.ini", GERMAN.as_bytes());
        let folders = [folder.path().to_path_buf()];
        let german = catalog_of(&GuiLanguage::Tag("DE".to_owned()), &folders);
        assert_eq!(german.catalog().text(texts::common::SAVE), "Speichern");
        let french = catalog_of(&GuiLanguage::Tag("fr".to_owned()), &folders);
        assert!(matches!(french, LanguageCatalog::English));
    }

    /// Verifies that the options take the names of the headers, sorted by tag, and keep a file
    /// that does not read or whose header fails without a name.
    #[test]
    fn the_options_take_the_names_of_the_headers() {
        let folder = TemporaryFolder::new("language_options");
        let japanese = folder.write_file(
            "ja.ini",
            "[language]\nname = 日本語\nplural = none\n".as_bytes(),
        );
        let german = folder.write_file("de.ini", GERMAN.as_bytes());
        let broken = folder.write_file("fr.ini", b"[language]\nname =\nplural = one-other\n");
        let files = [
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
        let option = |tag: &str, name: Option<&str>| LanguageOption {
            tag: tag.to_owned(),
            name: name.map(str::to_owned),
        };
        assert_eq!(
            language_options(&files),
            [
                option("de", Some("Deutsch")),
                option("fr", None),
                option("it", None),
                option("ja", Some("日本語")),
            ]
        );
    }
}
