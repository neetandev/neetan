//! The texts of the settings table in a catalog: the labels and the help texts of the settings,
//! the texts of the file types, of the model limits and of the unset words, and the faults of a
//! setting value.

use std::fmt;

use i18n::{Catalog, ListKind, Message};
use machine_factory::config::{
    MachineSelection,
    settings::{
        Choice, Setting,
        file_types::FileTypes,
        model_limits::LimitReason,
        value_error::{ValueError, compiled_choice_values},
    },
};

use super::{CatalogText, limit, value_error};

/// The group of the sections of the setting texts, `[setting.<key>]`.
const SETTING_GROUP: &str = "setting";

/// The group of the sections of the file type texts, `[file-type.<id>]`.
const FILE_TYPE_GROUP: &str = "file-type";

/// The section of the sentences of the model limits.
const LIMIT_SECTION: &str = "limit";

/// The section of the words of the unset values.
const UNSET_SECTION: &str = "unset";

/// The key of the label of a setting.
const LABEL_KEY: &str = "label";

/// The key of the help text of a setting.
const HELP_KEY: &str = "help";

/// Returns the fixed text `key` of the section `<group>.<name>` in `catalog`.
fn group_text<'c>(catalog: &'c Catalog, group: &str, name: &str, key: &str) -> Option<&'c str> {
    catalog
        .find_in_group(group, name, key)
        .map(|key| catalog.text(key))
}

/// Returns the label of `setting` in `catalog`. A setting without a label shows its key.
pub fn setting_label<'c>(catalog: &'c Catalog, setting: &Setting) -> &'c str {
    group_text(catalog, SETTING_GROUP, setting.key, LABEL_KEY).unwrap_or(setting.key)
}

/// Returns the help text of `setting` in `catalog`. A setting without a help text has an empty
/// one.
pub fn setting_help<'c>(catalog: &'c Catalog, setting: &Setting) -> &'c str {
    group_text(catalog, SETTING_GROUP, setting.key, HELP_KEY).unwrap_or_default()
}

/// A text of the section of a file type.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum FileTypeText {
    /// The name of the files in the plural, for example `Disk images`.
    Name,
    /// The text of an empty drive, for example `Empty. The drive starts without a disk.`
    EmptyDrive,
    /// The caption of the checkbox that empties a drive, for example `Start without a disk`.
    StartWithout,
    /// The text of a folder without files of the type, for example `No disk images in this
    /// folder`.
    NoFiles,
}

impl FileTypeText {
    /// The key of the text in the section of the file type.
    const fn key(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::EmptyDrive => "empty-drive",
            Self::StartWithout => "start-without",
            Self::NoFiles => "no-files",
        }
    }
}

/// Returns the text `text` of the file type `types` in `catalog`. A type without the text shows
/// its id.
pub fn file_type_text<'c>(catalog: &'c Catalog, types: &FileTypes, text: FileTypeText) -> &'c str {
    group_text(catalog, FILE_TYPE_GROUP, types.id, text.key()).unwrap_or(types.id)
}

/// Returns the text of the unset word `word` in `catalog`. A word without a text shows itself.
pub fn unset_word<'c>(catalog: &'c Catalog, word: &'c str) -> &'c str {
    catalog
        .find(UNSET_SECTION, word)
        .map_or(word, |key| catalog.text(key))
}

/// The sentence of a model that ignores a setting under a model limit, for example `PC9801RA
/// has no CD-ROM drive.`
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct LimitSentence {
    /// The reason of the limit.
    pub reason: LimitReason,
    /// The model that ignores the setting.
    pub model: MachineSelection,
}

impl CatalogText for LimitSentence {
    /// Writes the sentence through the keys of the section `[limit]`. A phrase without a
    /// sentence writes its key.
    fn write_text(&self, catalog: &Catalog, out: &mut dyn fmt::Write) -> fmt::Result {
        let name = self.model.name();
        match self.reason {
            LimitReason::Phrase(phrase) => match catalog.find(LIMIT_SECTION, phrase) {
                Some(key) => catalog.write(out, Message::new(key, [&name])),
                None => out.write_str(phrase),
            },
            LimitReason::FixedClock => match self.reason.fixed_clock(self.model) {
                Some(clock) => catalog.write(out, limit::fixed_clock(&name, &clock)),
                None => catalog.write(out, limit::fixed_cpu_clock(&name)),
            },
        }
    }
}

/// The compiled values of the choices of an unknown choice, joined with the or patterns of a
/// catalog. A list without values gives the text `value-error.no-choices`.
struct ExpectedChoices<'a> {
    /// The catalog of the patterns.
    catalog: &'a Catalog,
    /// The choices of the setting.
    choices: &'static [Choice],
}

impl fmt::Display for ExpectedChoices<'_> {
    /// Writes the values joined with the or patterns.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let values: Vec<&str> = compiled_choice_values(self.choices).collect();
        if values.is_empty() {
            return formatter.write_str(self.catalog.text(value_error::NO_CHOICES));
        }
        self.catalog.write_list(formatter, ListKind::Or, &values)
    }
}

impl CatalogText for ValueError {
    /// Writes the fault through the keys of the section `[value-error]`.
    fn write_text(&self, catalog: &Catalog, out: &mut dyn fmt::Write) -> fmt::Result {
        match self {
            Self::OnOff { value } => catalog.write(out, value_error::on_off(value)),
            Self::EmptyFolder => out.write_str(catalog.text(value_error::EMPTY_FOLDER)),
            Self::EmptyFile => out.write_str(catalog.text(value_error::EMPTY_FILE)),
            Self::OutOfRange { value, range } => catalog.write(
                out,
                value_error::out_of_range(value, &range.minimum_end(), &range.maximum_end()),
            ),
            Self::UnknownChoice { value, choices } => {
                let expected = ExpectedChoices { catalog, choices };
                catalog.write(out, value_error::unknown_choice(value, &expected))
            }
            Self::Language { value } => catalog.write(out, value_error::language(value)),
            Self::Year { value, first, last } => {
                catalog.write(out, value_error::year(value, first, last))
            }
            Self::NoSystem => out.write_str(catalog.text(value_error::NO_SYSTEM)),
            Self::NotCompiled { system } => catalog.write(out, value_error::not_compiled(system)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use i18n::Key;
    use machine_factory::config::settings::{
        NumberRange, SETTINGS, UnsetText, ValueKind, find_setting,
    };

    use super::*;
    use crate::texts::{SCHEMA, english, in_catalog, test_catalog};

    /// The text of `text` in `catalog`.
    fn written(catalog: &Catalog, text: &impl CatalogText) -> String {
        in_catalog(catalog, text).to_string()
    }

    /// Verifies that every setting has a label and a help sentence in the English catalog.
    #[test]
    fn every_setting_has_a_label_and_a_help_sentence() {
        let catalog = english();
        for setting in SETTINGS {
            for key in [LABEL_KEY, HELP_KEY] {
                let text = group_text(catalog, SETTING_GROUP, setting.key, key);
                assert!(
                    text.is_some_and(|text| !text.is_empty()),
                    "{} has no {key}",
                    setting.key
                );
            }
            assert!(
                setting_help(catalog, setting).ends_with('.'),
                "the help of {} is a sentence",
                setting.key
            );
            assert!(
                !setting_label(catalog, setting).ends_with('.'),
                "{}",
                setting.key
            );
        }
    }

    /// Verifies that every section of the setting texts belongs to a key of the table and holds
    /// only a label and a help text.
    #[cfg(feature = "pc88")]
    #[test]
    fn every_setting_section_belongs_to_the_table() {
        let section_prefix = |group: &str| format!("{group}.");
        for info in SCHEMA.keys() {
            if let Some(key) = info.section().strip_prefix(&section_prefix(SETTING_GROUP)) {
                assert!(find_setting(key).is_some(), "{}", info.section());
                assert!(
                    [LABEL_KEY, HELP_KEY].contains(&info.key()),
                    "{}",
                    info.key()
                );
            }
        }
    }

    /// Verifies that every file type has its texts and the extension sentence of its files.
    #[test]
    fn every_file_type_has_its_texts() {
        let catalog = english();
        let ids: BTreeSet<&str> = SETTINGS
            .iter()
            .flat_map(|setting| setting.file_types)
            .map(|types| types.id)
            .collect();
        assert_eq!(ids.len(), 5);
        for types in SETTINGS.iter().flat_map(|setting| setting.file_types) {
            for text in [
                FileTypeText::Name,
                FileTypeText::EmptyDrive,
                FileTypeText::StartWithout,
                FileTypeText::NoFiles,
            ] {
                assert!(
                    group_text(catalog, FILE_TYPE_GROUP, types.id, text.key()).is_some(),
                    "{} {text:?}",
                    types.id
                );
            }
        }
        let fdd1 = find_setting("fdd1").and_then(|setting| setting.file_types.first());
        let fdd1 = fdd1.expect("fdd1 takes floppy disk images");
        assert_eq!(
            file_type_text(catalog, fdd1, FileTypeText::Name),
            "Disk images"
        );
    }

    /// Verifies that every section `[file-type.<id>]` belongs to a file type of the table and
    /// holds only the texts of a file type.
    #[test]
    fn every_file_type_section_belongs_to_a_file_type() {
        let ids: BTreeSet<&str> = SETTINGS
            .iter()
            .flat_map(|setting| setting.file_types)
            .map(|types| types.id)
            .collect();
        let keys = [
            FileTypeText::Name,
            FileTypeText::EmptyDrive,
            FileTypeText::StartWithout,
            FileTypeText::NoFiles,
        ]
        .map(FileTypeText::key);
        let section_prefix = format!("{FILE_TYPE_GROUP}.");
        for info in SCHEMA.keys() {
            if let Some(id) = info.section().strip_prefix(&section_prefix) {
                assert!(ids.contains(id), "{} has no file type", info.section());
                assert!(
                    keys.contains(&info.key()),
                    "{} {}",
                    info.section(),
                    info.key()
                );
            }
        }
    }

    /// Verifies that every phrase of a model limit is a sentence of `[limit]` with the model
    /// placeholder.
    #[test]
    fn every_limit_phrase_is_a_sentence_with_the_model() {
        for setting in SETTINGS {
            for model_limit in setting.model_limits {
                if let LimitReason::Phrase(phrase) = model_limit.reason {
                    let key = SCHEMA
                        .find(LIMIT_SECTION, phrase)
                        .expect("the phrase is a key of [limit]");
                    assert_eq!(SCHEMA.info(key).placeholders(), ["model"], "{phrase}");
                    assert!(
                        english()
                            .display(Message::new(key, [&"X"]))
                            .to_string()
                            .ends_with('.')
                    );
                }
            }
        }
    }

    /// Verifies that every key of `[limit]` is a phrase of a model limit of the table or a
    /// sentence of a fixed clock.
    #[test]
    fn every_limit_sentence_has_a_model_limit() {
        let mut used: BTreeSet<Key> = SETTINGS
            .iter()
            .flat_map(|setting| setting.model_limits)
            .filter_map(|model_limit| match model_limit.reason {
                LimitReason::Phrase(phrase) => SCHEMA.find(LIMIT_SECTION, phrase),
                LimitReason::FixedClock => None,
            })
            .collect();
        used.insert(limit::fixed_clock(&"", &"").key());
        used.insert(limit::fixed_cpu_clock(&"").key());
        for (index, info) in SCHEMA.keys().iter().enumerate() {
            if info.section() == LIMIT_SECTION {
                let key = Key::new(u16::try_from(index).expect("the schema index fits"));
                assert!(used.contains(&key), "{} has no model limit", info.key());
            }
        }
    }

    /// Verifies the sentences of a phrase and of a fixed clock limit.
    #[cfg(feature = "pc98")]
    #[test]
    fn limits_write_their_sentence_with_the_model() {
        let model: MachineSelection = "PC9801RA".parse().expect("a PC-98 model");
        let sentence = |reason| written(english(), &LimitSentence { reason, model });
        assert_eq!(
            sentence(LimitReason::Phrase("no-cdrom")),
            "PC9801RA has no CD-ROM drive."
        );
        assert_eq!(
            sentence(LimitReason::FixedClock),
            "PC9801RA runs at a fixed 20 MHz."
        );
    }

    /// Verifies that every unset word has its text.
    #[test]
    fn every_unset_word_has_its_text() {
        for setting in SETTINGS {
            for unset in setting.unset_values {
                if let UnsetText::Word(word) = unset.text {
                    assert!(SCHEMA.find(UNSET_SECTION, word).is_some(), "{word}");
                }
            }
        }
        assert_eq!(unset_word(english(), "auto"), "auto");
    }

    /// Verifies that every key of `[unset]` is the word of an unset value of the table.
    #[test]
    fn every_unset_text_belongs_to_an_unset_word() {
        let words: BTreeSet<&str> = SETTINGS
            .iter()
            .flat_map(|setting| setting.unset_values)
            .filter_map(|unset| match unset.text {
                UnsetText::Word(word) => Some(word),
                UnsetText::Value(_) => None,
            })
            .collect();
        for info in SCHEMA.keys() {
            if info.section() == UNSET_SECTION {
                assert!(
                    words.contains(info.key()),
                    "{} has no unset value",
                    info.key()
                );
            }
        }
    }

    /// Verifies that the English catalog writes every value fault as its English text, and that
    /// a catalog orders the choices with its own patterns.
    #[test]
    fn value_errors_write_their_english_text() {
        let range = NumberRange {
            minimum: 0.0,
            maximum: 3.0,
            step: 1.0,
            integer: true,
        };
        let scaling = find_setting("scaling").expect("scaling is a table key");
        let ValueKind::Choice(choices) = scaling.kind else {
            panic!("scaling is a choice setting");
        };
        let errors = [
            ValueError::OnOff {
                value: "maybe".to_owned(),
            },
            ValueError::EmptyFolder,
            ValueError::EmptyFile,
            ValueError::OutOfRange {
                value: "4".to_owned(),
                range,
            },
            ValueError::unknown_choice("sharp", choices),
            ValueError::unknown_choice("sharp", &[]),
            ValueError::Language {
                value: "pt_BR".to_owned(),
            },
            ValueError::Year {
                value: "93".to_owned(),
                first: 1970,
                last: 2100,
            },
            ValueError::NoSystem,
            ValueError::NotCompiled { system: "pc98" },
        ];
        for error in &errors {
            assert_eq!(written(english(), error), error.to_string());
        }
        let german = test_catalog(
            "[value-error]\nunknown-choice = {choices} statt '{value}'\n[list]\n\
             or-pair = {first} oder {second}\nor-middle = {first} / {second}\n\
             or-end = {first} oder {second}\n",
        );
        assert_eq!(
            written(&german, &errors[4]),
            "nearest / bilinear oder pixelart statt 'sharp'"
        );
    }
}
