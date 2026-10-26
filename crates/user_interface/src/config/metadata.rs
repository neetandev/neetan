//! The metadata of a game: the `meta.*` keys of its game file.
//!
//! The emulator ignores every `meta.*` key. The metadata gives the title of
//! the game, its makers, its year and its genre, and holds free notes.

use i18n::Key;
use machine_factory::config::settings::value_error::ValueError;

use super::{
    diagnostic::{Diagnostic, DiagnosticKind},
    document::ConfigDocument,
};
use crate::texts::metadata;

/// Prefix of every metadata key.
pub const META_PREFIX: &str = "meta.";
/// Key of the display title.
pub const TITLE_KEY: &str = "meta.title";
/// Key of the title in its original script.
pub const TITLE_ORIGINAL_KEY: &str = "meta.title-original";
/// Key of the developer.
pub const DEVELOPER_KEY: &str = "meta.developer";
/// Key of the publisher.
pub const PUBLISHER_KEY: &str = "meta.publisher";
/// Key of the release year.
pub const YEAR_KEY: &str = "meta.year";
/// Key of the genre.
pub const GENRE_KEY: &str = "meta.genre";
/// Key of the notes.
pub const NOTES_KEY: &str = "meta.notes";

/// A metadata key with its label.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct MetadataField {
    /// The key, for example `meta.title`.
    pub key: &'static str,
    /// The key of the short human-readable name of the key.
    pub label: Key,
}

/// Every metadata key in display order.
pub const METADATA_FIELDS: [MetadataField; 7] = [
    MetadataField {
        key: TITLE_KEY,
        label: metadata::TITLE,
    },
    MetadataField {
        key: TITLE_ORIGINAL_KEY,
        label: metadata::ORIGINAL_TITLE,
    },
    MetadataField {
        key: DEVELOPER_KEY,
        label: metadata::DEVELOPER,
    },
    MetadataField {
        key: PUBLISHER_KEY,
        label: metadata::PUBLISHER,
    },
    MetadataField {
        key: YEAR_KEY,
        label: metadata::YEAR,
    },
    MetadataField {
        key: GENRE_KEY,
        label: metadata::GENRE,
    },
    MetadataField {
        key: NOTES_KEY,
        label: metadata::NOTES,
    },
];

/// The earliest accepted release year.
pub const FIRST_YEAR: u16 = 1970;
/// The latest accepted release year.
pub const LAST_YEAR: u16 = 2100;

/// The metadata of one game. An empty text is unset.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GameMetadata {
    /// The display title, `meta.title`.
    pub title: String,
    /// The title in its original script, `meta.title-original`.
    pub title_original: String,
    /// The developer, `meta.developer`.
    pub developer: String,
    /// The publisher, `meta.publisher`.
    pub publisher: String,
    /// The release year as the file writes it, `meta.year`. `year()` parses it.
    pub year: String,
    /// The genre, `meta.genre`.
    pub genre: String,
    /// Free notes, `meta.notes`.
    pub notes: String,
}

impl GameMetadata {
    /// Reads the metadata of `document`.
    ///
    /// Each key reads its last active line. A year that `parse_year` rejects
    /// keeps its text and adds a warning to `diagnostics`.
    pub fn read(document: &ConfigDocument, diagnostics: &mut Vec<Diagnostic>) -> Self {
        let text = |key: &str| document.get(key).unwrap_or_default().to_owned();
        if let Some(entry) = document
            .entries()
            .rfind(|entry| entry.key == YEAR_KEY && !entry.value.is_empty())
            && let Err(error) = parse_year(entry.value)
        {
            diagnostics.push(Diagnostic::warning(
                entry.line,
                DiagnosticKind::InvalidValue {
                    key: YEAR_KEY,
                    error,
                },
            ));
        }
        Self {
            title: text(TITLE_KEY),
            title_original: text(TITLE_ORIGINAL_KEY),
            developer: text(DEVELOPER_KEY),
            publisher: text(PUBLISHER_KEY),
            year: text(YEAR_KEY),
            genre: text(GENRE_KEY),
            notes: text(NOTES_KEY),
        }
    }

    /// Returns the release year. An empty or invalid year is `None`.
    #[must_use]
    pub fn year(&self) -> Option<u16> {
        parse_year(self.year.trim()).ok()
    }

    /// Writes the metadata into `document`.
    ///
    /// A field takes its key with `set_in_group`, so a new key joins the other
    /// `meta.*` lines. An empty field removes every line of its key that holds
    /// a value.
    pub fn write(&self, document: &mut ConfigDocument) {
        for (key, value) in [
            (TITLE_KEY, &self.title),
            (TITLE_ORIGINAL_KEY, &self.title_original),
            (DEVELOPER_KEY, &self.developer),
            (PUBLISHER_KEY, &self.publisher),
            (YEAR_KEY, &self.year),
            (GENRE_KEY, &self.genre),
            (NOTES_KEY, &self.notes),
        ] {
            if !value.trim().is_empty() {
                document.set_in_group(key, value, META_PREFIX);
            } else if document.get_all(key).iter().any(|value| !value.is_empty()) {
                document.remove(key);
            }
        }
    }
}

/// Parses a release year from `FIRST_YEAR` to `LAST_YEAR`.
pub fn parse_year(value: &str) -> Result<u16, ValueError> {
    match value.parse::<u16>() {
        Ok(year) if (FIRST_YEAR..=LAST_YEAR).contains(&year) => Ok(year),
        _ => Err(ValueError::Year {
            value: value.to_owned(),
            first: FIRST_YEAR,
            last: LAST_YEAR,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::diagnostic::Severity, texts::english};

    /// A game file with every metadata key.
    const EDGE: &str = "meta.title = Edge\nmeta.title-original = エッジ\nmeta.developer = TGL\n\
                        meta.publisher = TGL\nmeta.year = 1993\nmeta.genre = Shooter\n\
                        meta.notes = Needs the 26k board for music.\nmachine = PC9801RA\n\
                        fdd1 = games/98/Edge (Disk A).hdm\n";

    /// Verifies that every key reads into its field from its last active line.
    #[test]
    fn metadata_reads_every_key() {
        let mut diagnostics = Vec::new();
        let metadata = GameMetadata::read(&ConfigDocument::parse(EDGE), &mut diagnostics);
        assert!(diagnostics.is_empty());
        assert_eq!(
            metadata,
            GameMetadata {
                title: "Edge".to_owned(),
                title_original: "エッジ".to_owned(),
                developer: "TGL".to_owned(),
                publisher: "TGL".to_owned(),
                year: "1993".to_owned(),
                genre: "Shooter".to_owned(),
                notes: "Needs the 26k board for music.".to_owned(),
            }
        );
        assert_eq!(metadata.year(), Some(1993));

        let document =
            ConfigDocument::parse("meta.title = Old\n;meta.title = Commented\nmeta.title = New\n");
        assert_eq!(GameMetadata::read(&document, &mut Vec::new()).title, "New");
    }

    /// Verifies that writing unchanged metadata and setting every key to its value changes only the form.
    #[test]
    fn unchanged_writes_keep_normalized_files() {
        for text in [
            EDGE,
            "\u{FEFF}; ヘッダー\r\nmeta.title=Edge  \r\nmeta.year = 93\r\nmachine=PC9821AP\r\nfdd1 = a.hdm",
            "meta.genre =\nmachine = X1\n",
        ] {
            let normalized = ConfigDocument::parse(text).to_text();
            let mut document = ConfigDocument::parse(text);
            let metadata = GameMetadata::read(&document, &mut Vec::new());
            metadata.write(&mut document);
            let entries: Vec<(String, String)> = document
                .entries()
                .map(|entry| (entry.key.to_owned(), entry.value.to_owned()))
                .collect();
            for (key, value) in entries {
                document.set(&key, &value);
            }
            assert_eq!(document.to_text(), normalized);
        }
        assert_eq!(ConfigDocument::parse(EDGE).to_text(), EDGE);
    }

    /// Verifies that an empty field removes its key and a new field joins the other metadata lines.
    #[test]
    fn writing_updates_removes_and_groups() {
        let mut document = ConfigDocument::parse(EDGE);
        let mut metadata = GameMetadata::read(&document, &mut Vec::new());
        metadata.title_original.clear();
        metadata.year.clear();
        metadata.notes = "  ".to_owned();
        metadata.genre = "Shoot 'em up".to_owned();
        metadata.write(&mut document);
        assert_eq!(
            document.to_text(),
            "meta.title = Edge\nmeta.developer = TGL\nmeta.publisher = TGL\n\
             meta.genre = Shoot 'em up\nmachine = PC9801RA\nfdd1 = games/98/Edge (Disk A).hdm\n"
        );

        let mut document = ConfigDocument::parse("; Dragon Knight\nmachine=PC9801VM\nfdd1=a.d88\n");
        GameMetadata {
            title: "Dragon Knight".to_owned(),
            year: "1989".to_owned(),
            ..GameMetadata::default()
        }
        .write(&mut document);
        assert_eq!(
            document.to_text(),
            "; Dragon Knight\nmeta.title = Dragon Knight\nmeta.year = 1989\nmachine = PC9801VM\n\
             fdd1 = a.d88\n"
        );
    }

    /// Verifies that an invalid year keeps its text with a warning on its line.
    #[test]
    fn invalid_year_is_a_warning() {
        for value in ["93", "early nineties", "2525"] {
            let text = format!("machine = X1\nmeta.year = {value}\n");
            let mut document = ConfigDocument::parse(&text);
            let mut diagnostics = Vec::new();
            let metadata = GameMetadata::read(&document, &mut diagnostics);
            assert_eq!(metadata.year, value);
            assert_eq!(metadata.year(), None, "{value}");
            assert_eq!(diagnostics.len(), 1, "{value}");
            assert_eq!(diagnostics[0].line, Some(2));
            assert_eq!(diagnostics[0].severity, Severity::Warning);
            metadata.write(&mut document);
            assert_eq!(document.to_text(), text);
        }
        let mut diagnostics = Vec::new();
        let metadata =
            GameMetadata::read(&ConfigDocument::parse("meta.year =\n"), &mut diagnostics);
        assert_eq!(metadata.year(), None);
        assert!(diagnostics.is_empty());
        assert_eq!(parse_year("1970"), Ok(1970));
        assert_eq!(parse_year("2100"), Ok(2100));
    }

    /// Verifies that the metadata fields hold every key of the struct once, with the prefix and a label.
    #[test]
    fn metadata_fields_hold_every_key_once() {
        let keys: Vec<&str> = METADATA_FIELDS.iter().map(|field| field.key).collect();
        assert_eq!(
            keys,
            [
                TITLE_KEY,
                TITLE_ORIGINAL_KEY,
                DEVELOPER_KEY,
                PUBLISHER_KEY,
                YEAR_KEY,
                GENRE_KEY,
                NOTES_KEY
            ]
        );
        for field in METADATA_FIELDS {
            assert!(field.key.starts_with(META_PREFIX), "{}", field.key);
            assert!(!english().text(field.label).is_empty(), "{}", field.key);
        }
    }
}
