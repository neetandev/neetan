//! The notes of the form rows: the text after a control that tells the
//! inherited value, the range, the files, a fact of the value, or why the
//! model ignores the setting.
//!
//! A note is made of full sentences that end with a full stop. It shows the
//! inherited value only where the control does not show it: a set field
//! shows it in its note, and an unset field shows it in its dropdown entry
//! or its placeholder.

use machine_factory::config::settings::{DERIVED_DEFAULT_NOTE, NumberRange, file_types::FileTypes};

use super::{fields::InvalidValue, inherited::Inherited};

/// The placeholder and the inherited label of a key without a value.
pub(crate) const NOT_SET: &str = "Not set";

/// The placeholder of a single image field that the file empties.
pub(crate) const EMPTY_IMAGE: &str = "Empty";

/// The note of a missing single image.
pub(crate) const MISSING_IMAGE_NOTE: &str =
    "The image does not exist. The file saves, and the launch fails while it is missing.";

/// The note of the model of a new game while it keeps its preset.
pub(crate) const PRESET_NOTE: &str = "The model is preset from the system default.";

/// The note of a required field that the file does not set. Its control
/// shows the inherited value.
pub(crate) const REQUIRED_NOTE: &str = "The file inherits this value. The save writes it.";

/// The noun of the images of a drive without file types.
const DEFAULT_IMAGE_NOUN: &str = "disk";

/// The look of a note.
#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
pub(crate) enum NoteKind {
    /// Small secondary text.
    #[default]
    Plain,
    /// A warning: the alert icon and the text in the error color.
    Warning,
    /// A thing that exists: the check icon and the text in the marker color.
    Found,
    /// A status without a mark: small secondary text after an empty icon
    /// slot, in line with the text of a found or a warning status.
    Unmarked,
}

/// The text of a note and its look.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Note {
    /// The text. An empty text draws no note.
    pub(crate) text: String,
    /// The look of the note.
    pub(crate) kind: NoteKind,
    /// Whether the text is a path. A path keeps to one line and gives way
    /// in the middle.
    pub(crate) path: bool,
}

impl Note {
    /// A note with `text` of `kind`.
    fn of_kind(text: impl Into<String>, kind: NoteKind) -> Self {
        Self {
            text: text.into(),
            kind,
            path: false,
        }
    }

    /// A plain note with `text`.
    pub(crate) fn plain(text: impl Into<String>) -> Self {
        Self::of_kind(text, NoteKind::Plain)
    }

    /// A warning with `text`.
    pub(crate) fn warning(text: impl Into<String>) -> Self {
        Self::of_kind(text, NoteKind::Warning)
    }

    /// A note with `text` about a thing that exists.
    pub(crate) fn found(text: impl Into<String>) -> Self {
        Self::of_kind(text, NoteKind::Found)
    }

    /// A status note with `text` without a mark.
    pub(crate) fn unmarked(text: impl Into<String>) -> Self {
        Self::of_kind(text, NoteKind::Unmarked)
    }

    /// The same note with its text as a path.
    pub(crate) fn shown_as_path(self) -> Self {
        Self { path: true, ..self }
    }

    /// Whether the note is a warning.
    pub(crate) fn is_warning(&self) -> bool {
        self.kind == NoteKind::Warning
    }

    /// The note with the sentence `sentence` after its text. A warning
    /// keeps its text.
    pub(crate) fn followed_by(self, sentence: Option<String>) -> Self {
        match sentence {
            Some(sentence) if !self.is_warning() => Self {
                text: sentences(&[Some(self.text), Some(sentence)]),
                path: false,
                ..self
            },
            Some(_) | None => self,
        }
    }
}

/// Returns `text` as a sentence that ends with a full stop.
fn sentence(text: &str) -> String {
    let text = text.trim();
    if text.ends_with('.') {
        text.to_owned()
    } else {
        format!("{text}.")
    }
}

/// Joins the parts of a note into sentences. Each part ends with a full
/// stop, and a space separates the parts.
pub(crate) fn sentences(parts: &[Option<String>]) -> String {
    parts
        .iter()
        .flatten()
        .filter(|part| !part.trim().is_empty())
        .map(|part| sentence(part))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Returns `text` with its first letter in uppercase.
fn capitalized(text: &str) -> String {
    let mut characters = text.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => String::new(),
    }
}

/// The inherited value with its source, for example `Built-in default:
/// high`, as the placeholder of a text or number field and the inherited
/// entry of a dropdown. A key without an inherited value gives
/// [`NOT_SET`].
pub(crate) fn inherited_placeholder(inherited: Option<&Inherited>) -> String {
    inherited
        .and_then(Inherited::label)
        .unwrap_or_else(|| NOT_SET.to_owned())
}

/// The sentence of the inherited value of a set field, for example
/// `Built-in default: high.`
fn inherited_sentence(inherited: Option<&Inherited>) -> Option<String> {
    inherited
        .and_then(Inherited::label)
        .map(|label| sentence(&label))
}

/// The warning of a value of the file that the launch rejects, for example
/// `Invalid value 'On', expected on or off. The launch ignores the line.`
pub(crate) fn invalid_value_note(invalid: &InvalidValue) -> Note {
    Note::warning(sentences(&[
        Some(capitalized(&invalid.message)),
        Some("The launch ignores the line".to_owned()),
    ]))
}

/// The state of a choice field that its note shows.
#[derive(Debug, Clone, Default)]
pub(crate) struct ChoiceNote<'a> {
    /// Why the model ignores the setting.
    pub(crate) ignored: Option<String>,
    /// The value of the file that the launch rejects.
    pub(crate) invalid: Option<&'a InvalidValue>,
    /// Whether the field keeps the preset of a new game.
    pub(crate) preset: bool,
    /// Whether the file sets the key.
    pub(crate) set: bool,
    /// Whether the field always saves its key. Its dropdown shows the
    /// inherited value without the inherited entry.
    pub(crate) required: bool,
    /// The inherited value.
    pub(crate) inherited: Option<&'a Inherited>,
    /// Whether the default derives from other settings.
    pub(crate) derived: bool,
    /// The fact of the value that the table gives.
    pub(crate) fact: Option<String>,
}

/// The note of a choice field: why the model ignores it, the warning of an
/// invalid value, the preset of a new game, or else the inherited value,
/// the derived default and the fact of the value.
///
/// The note of a set key shows the inherited value. An unset key leaves it
/// to its dropdown, which shows it as its inherited entry. An unset
/// required key shows the inherited value as its choice, and its note tells
/// that the save writes it.
pub(crate) fn choice_note(state: ChoiceNote) -> Note {
    if let Some(ignored) = state.ignored {
        return Note::plain(ignored);
    }
    if let Some(invalid) = state.invalid {
        return invalid_value_note(invalid);
    }
    if state.preset {
        return Note::plain(PRESET_NOTE);
    }
    let inherited = if state.set {
        inherited_sentence(state.inherited)
    } else if state.required {
        Some(REQUIRED_NOTE.to_owned())
    } else {
        None
    };
    let derived = state.derived.then(|| DERIVED_DEFAULT_NOTE.to_owned());
    Note::plain(sentences(&[inherited, derived, state.fact]))
}

/// The digits after the decimal point of the numbers of `range`.
pub(crate) fn decimals_of(range: NumberRange) -> usize {
    if range.integer {
        return 0;
    }
    let step = format!("{}", range.step);
    step.split_once('.')
        .map_or(0, |(_, fraction)| fraction.len())
}

/// The note of a number field: why the model ignores it, or else the range,
/// after the inherited value of a set field. An unset field shows the
/// inherited value as its placeholder.
pub(crate) fn number_note(
    ignored: Option<String>,
    set: bool,
    range: NumberRange,
    inherited: Option<&Inherited>,
) -> Note {
    if let Some(ignored) = ignored {
        return Note::plain(ignored);
    }
    let decimals = decimals_of(range);
    let range_text = format!(
        "The range is {:.decimals$} to {:.decimals$}",
        range.minimum, range.maximum
    );
    let inherited = set.then(|| inherited_sentence(inherited)).flatten();
    Note::plain(sentences(&[inherited, Some(range_text)]))
}

/// The extensions of `types` with their dots, for example `.hdi .nhd .thd`.
pub(crate) fn extension_list(types: &FileTypes) -> String {
    types
        .extensions
        .iter()
        .map(|extension| format!(".{extension}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The sentence of the files of `types`, for example `Hard disk images:
/// .hdi .nhd .thd.`
pub(crate) fn extension_note(types: &FileTypes) -> String {
    sentence(&format!("{}: {}", types.name, extension_list(types)))
}

/// The note of a single path field.
///
/// A path that the model ignores tells why, and a missing image is a
/// warning. A set path shows its inherited value as a path. An unset path
/// shows the inherited value as its placeholder. Without an inherited value
/// to show, the note gives the extensions of the files.
pub(crate) fn path_note(
    ignored: Option<String>,
    missing: bool,
    set: bool,
    inherited: Option<&Inherited>,
    types: Option<&FileTypes>,
) -> Note {
    if let Some(ignored) = ignored {
        return Note::plain(ignored);
    }
    if missing {
        return Note::warning(MISSING_IMAGE_NOTE);
    }
    if set && let Some(inherited) = inherited_sentence(inherited) {
        return Note::plain(inherited).shown_as_path();
    }
    types.map_or_else(Note::default, |types| Note::plain(extension_note(types)))
}

/// The note below a media list that the file sets, with images or as an
/// empty list: the inherited list that the file overrides. A list without
/// images of the file shows the inherited list in the list itself. A note
/// of one inherited image is a path. A note of several images wraps.
pub(crate) fn media_note(set: bool, inherited: Option<&Inherited>) -> Note {
    match set.then(|| inherited_sentence(inherited)).flatten() {
        Some(text) if inherited.is_some_and(|inherited| inherited.values.len() == 1) => {
            Note::plain(text).shown_as_path()
        }
        Some(text) => Note::plain(text),
        None => Note::default(),
    }
}

/// The noun of the images of `types`, for example `disc`.
pub(crate) fn image_noun(types: Option<&FileTypes>) -> &'static str {
    types.map_or(DEFAULT_IMAGE_NOUN, |types| types.noun)
}

/// The text of a media list without images. An empty list of the file and
/// a drive without an inherited list start without an image. A list that
/// inherits images shows them with their source.
pub(crate) fn empty_drive_hint(
    types: Option<&FileTypes>,
    inherited: Option<&Inherited>,
    empty_list: bool,
) -> String {
    match inherited.and_then(Inherited::label) {
        Some(label) if !empty_list => label,
        Some(_) | None => format!("Empty. The drive starts without a {}.", image_noun(types)),
    }
}

/// The caption of the checkbox that empties the drive over its inherited list.
pub(crate) fn empty_list_caption(types: Option<&FileTypes>) -> String {
    format!("Start without a {}", image_noun(types))
}

#[cfg(test)]
mod tests {
    use machine_factory::config::settings::find_setting;

    use super::*;
    use crate::editor::inherited::InheritedSource;

    /// The inherited `values` of `source`.
    fn inherited(values: &[&str], source: InheritedSource) -> Inherited {
        Inherited {
            values: values.iter().map(|value| (*value).to_owned()).collect(),
            source,
        }
    }

    /// The file types of the hard disks of the first family that has them.
    fn hard_disks() -> Option<&'static FileTypes> {
        find_setting("hdd1").and_then(|setting| setting.file_types.first())
    }

    /// Verifies that every part of a note ends with a full stop and that empty parts leave the note.
    #[test]
    fn notes_are_full_sentences() {
        assert_eq!(
            sentences(&[
                Some("Built-in default: low".into()),
                None,
                Some(String::new()),
                Some("At low the CPU runs at 4 MHz.".into()),
            ]),
            "Built-in default: low. At low the CPU runs at 4 MHz."
        );
        assert_eq!(sentences(&[]), "");
        assert_eq!(
            inherited_placeholder(Some(&inherited(&["0"], InheritedSource::BuiltIn))),
            "Built-in default: 0"
        );
        assert_eq!(
            inherited_placeholder(Some(&inherited(&[], InheritedSource::BuiltIn))),
            NOT_SET
        );
        assert_eq!(inherited_placeholder(None), NOT_SET);
    }

    /// Verifies that a sentence follows a plain note, starts an empty note and leaves a warning as it is.
    #[test]
    fn a_sentence_follows_a_plain_note_only() {
        let models = Some("PC9821AS and AP only.".to_owned());
        assert_eq!(
            Note::plain("Built-in default: off.").followed_by(models.clone()),
            Note::plain("Built-in default: off. PC9821AS and AP only.")
        );
        assert_eq!(
            Note::default().followed_by(models.clone()),
            Note::plain("PC9821AS and AP only.")
        );
        assert_eq!(
            Note::warning(MISSING_IMAGE_NOTE).followed_by(models),
            Note::warning(MISSING_IMAGE_NOTE)
        );
        assert_eq!(
            Note::plain("Range 0 to 3").followed_by(None),
            Note::plain("Range 0 to 3")
        );
    }

    /// Verifies the choice note of an unset, a set and a required key, and the parts that win over them.
    #[test]
    fn choice_notes_show_the_inherited_value_only_for_a_set_key() {
        let inherited = inherited(&["v2"], InheritedSource::BuiltIn);
        let invalid = InvalidValue {
            value: "On".to_owned(),
            message: "invalid value 'On', expected on or off".to_owned(),
        };
        assert_eq!(
            choice_note(ChoiceNote {
                ignored: Some("PC9801RA runs at a fixed 20 MHz. PC9801F, VM and VX only.".into()),
                invalid: Some(&invalid),
                ..ChoiceNote::default()
            }),
            Note::plain("PC9801RA runs at a fixed 20 MHz. PC9801F, VM and VX only.")
        );
        assert_eq!(
            choice_note(ChoiceNote {
                invalid: Some(&invalid),
                preset: true,
                ..ChoiceNote::default()
            }),
            Note::warning("Invalid value 'On', expected on or off. The launch ignores the line.")
        );
        assert_eq!(
            choice_note(ChoiceNote {
                preset: true,
                set: true,
                required: true,
                inherited: Some(&inherited),
                ..ChoiceNote::default()
            }),
            Note::plain(PRESET_NOTE)
        );
        let fact = Some("The choices are v1s, v1h, v2, n, n80 and n80sr.".to_owned());
        assert_eq!(
            choice_note(ChoiceNote {
                set: true,
                inherited: Some(&inherited),
                fact: fact.clone(),
                ..ChoiceNote::default()
            }),
            Note::plain("Built-in default: v2. The choices are v1s, v1h, v2, n, n80 and n80sr.")
        );
        assert_eq!(
            choice_note(ChoiceNote {
                inherited: Some(&inherited),
                fact,
                ..ChoiceNote::default()
            }),
            Note::plain("The choices are v1s, v1h, v2, n, n80 and n80sr.")
        );
        assert_eq!(
            choice_note(ChoiceNote {
                set: true,
                inherited: Some(&self::inherited(&["low"], InheritedSource::BuiltIn)),
                derived: true,
                fact: Some("At low the CPU runs at 4 MHz.".into()),
                ..ChoiceNote::default()
            }),
            Note::plain(
                "Built-in default: low. The default follows the boot mode. At low the CPU runs \
                 at 4 MHz."
            )
        );
        assert_eq!(
            choice_note(ChoiceNote {
                inherited: Some(&inherited),
                derived: true,
                ..ChoiceNote::default()
            }),
            Note::plain(DERIVED_DEFAULT_NOTE)
        );
        let model = self::inherited(&["PC9801RA"], InheritedSource::SystemDefaults);
        assert_eq!(
            choice_note(ChoiceNote {
                required: true,
                inherited: Some(&model),
                ..ChoiceNote::default()
            }),
            Note::plain(REQUIRED_NOTE)
        );
        assert_eq!(
            choice_note(ChoiceNote {
                set: true,
                required: true,
                inherited: Some(&model),
                ..ChoiceNote::default()
            }),
            Note::plain("System default: PC9801RA.")
        );
        assert_eq!(choice_note(ChoiceNote::default()), Note::default());
    }

    /// Verifies the digits of the steps of a range.
    #[test]
    fn decimals_follow_the_step() {
        let range = |step: f64, integer: bool| NumberRange {
            minimum: 0.0,
            maximum: 1.0,
            step,
            integer,
        };
        assert_eq!(decimals_of(range(1.0, true)), 0);
        assert_eq!(decimals_of(range(0.1, false)), 1);
        assert_eq!(decimals_of(range(0.25, false)), 2);
        assert_eq!(decimals_of(range(1.0, false)), 0);
    }

    /// Verifies the note of an unset and a set number field: the inherited value only for a set field, and the range.
    #[test]
    fn number_notes_give_the_range() {
        let range = NumberRange {
            minimum: 0.0,
            maximum: 3.0,
            step: 1.0,
            integer: true,
        };
        let inherited = inherited(&["0"], InheritedSource::BuiltIn);
        assert_eq!(
            number_note(None, true, range, Some(&inherited)),
            Note::plain("Built-in default: 0. The range is 0 to 3.")
        );
        assert_eq!(
            number_note(None, false, range, Some(&inherited)),
            Note::plain("The range is 0 to 3.")
        );
        assert_eq!(
            number_note(None, true, range, None),
            Note::plain("The range is 0 to 3.")
        );
        assert_eq!(
            number_note(Some("Ignored.".into()), true, range, Some(&inherited)),
            Note::plain("Ignored.")
        );
    }

    /// Verifies the single path note in the unset and the set state, with and without an inherited value, and the warnings that win.
    #[test]
    fn path_notes_show_the_inherited_value_of_a_set_path_or_the_extensions() {
        let types = hard_disks();
        let settings = inherited(&["games/boot.hdi"], InheritedSource::SettingsFile);
        let nothing = inherited(&[], InheritedSource::BuiltIn);
        assert_eq!(
            path_note(None, false, true, Some(&settings), types),
            Note::plain("Settings default: games/boot.hdi.").shown_as_path()
        );
        let extensions =
            types.map_or_else(Note::default, |types| Note::plain(extension_note(types)));
        assert_eq!(
            path_note(None, false, false, Some(&settings), types),
            extensions
        );
        assert_eq!(
            path_note(None, false, true, Some(&nothing), types),
            extensions
        );
        assert_eq!(
            path_note(None, false, false, Some(&nothing), types),
            extensions
        );
        assert_eq!(
            path_note(None, true, true, Some(&settings), types),
            Note::warning(MISSING_IMAGE_NOTE)
        );
        assert_eq!(
            path_note(Some("Ignored.".into()), true, true, None, types),
            Note::plain("Ignored.")
        );
        assert_eq!(path_note(None, false, false, None, None), Note::default());
        if let Some(types) = types {
            assert_eq!(
                extension_note(types),
                format!("{}: {}.", types.name, extension_list(types))
            );
            assert!(extension_list(types).starts_with('.'));
        }
    }

    /// Verifies the note below a media list: the inherited list of a set list only, as a path for one image.
    #[test]
    fn media_notes_show_the_inherited_list_of_a_set_list() {
        let settings = inherited(&["games/boot.d88"], InheritedSource::SettingsFile);
        let nothing = inherited(&[], InheritedSource::BuiltIn);
        assert_eq!(
            media_note(true, Some(&settings)),
            Note::plain("Settings default: games/boot.d88.").shown_as_path()
        );
        let two = inherited(&["a.d88", "b.d88"], InheritedSource::SettingsFile);
        assert_eq!(
            media_note(true, Some(&two)),
            Note::plain("Settings default: a.d88, b.d88.")
        );
        assert_eq!(media_note(false, Some(&settings)), Note::default());
        assert_eq!(media_note(true, Some(&nothing)), Note::default());
        assert_eq!(media_note(true, None), Note::default());
    }

    /// Verifies the hint of an empty drive: the noun of its images, the inherited list and the empty list of the file.
    #[test]
    fn empty_drives_tell_what_they_start_with() {
        let cdrom = find_setting("cdrom").and_then(|setting| setting.file_types.first());
        let fdd1 = find_setting("fdd1").and_then(|setting| setting.file_types.first());
        let settings = inherited(&["games/boot.d88"], InheritedSource::SettingsFile);
        let nothing = inherited(&[], InheritedSource::BuiltIn);
        assert_eq!(
            empty_drive_hint(fdd1, Some(&nothing), false),
            "Empty. The drive starts without a disk."
        );
        if cdrom.is_some() {
            assert_eq!(
                empty_drive_hint(cdrom, None, false),
                "Empty. The drive starts without a disc."
            );
            assert_eq!(empty_list_caption(cdrom), "Start without a disc");
        }
        assert_eq!(
            empty_drive_hint(fdd1, Some(&settings), false),
            "Settings default: games/boot.d88"
        );
        assert_eq!(
            empty_drive_hint(fdd1, Some(&settings), true),
            "Empty. The drive starts without a disk."
        );
        assert_eq!(empty_list_caption(None), "Start without a disk");
    }
}
