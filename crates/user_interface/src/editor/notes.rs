//! The notes of the form rows: the text after a control that tells the
//! inherited value, the range, the files, a fact of the value, or why the
//! model ignores the setting.
//!
//! A note is made of full sentences. Each sentence of the catalog ends with
//! its own sentence mark, and the catalog pattern `note.join` joins the
//! sentences of a note. A note shows the inherited value only where the
//! control does not show it: a set field shows it in its note, and an unset
//! field shows it in its dropdown entry or its placeholder.

use std::fmt::{self, Write as _};

use i18n::{Catalog, ListKind};
use machine_factory::config::settings::{
    NumberRange, file_types::FileTypes, model_limits::FactData,
};

use super::{
    fields::{InvalidReason, InvalidValue},
    form::{IgnoredBy, write_ignored},
    inherited::Inherited,
    note_writer::NoteWriter,
};
use crate::texts::{FileTypeText, file_type_text, in_catalog, note, unset_word};

/// The most bytes of the shortest text of a number step.
const STEP_TEXT_CAPACITY: usize = 32;

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

/// The look of a note and whether its text is a path.
#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
pub(crate) struct NoteLook {
    /// The look of the note.
    pub(crate) kind: NoteKind,
    /// Whether the text is a path. A path keeps to one line and gives way
    /// in the middle.
    pub(crate) path: bool,
}

impl NoteLook {
    /// A plain note.
    pub(crate) const PLAIN: Self = Self::of(NoteKind::Plain);

    /// A warning.
    pub(crate) const WARNING: Self = Self::of(NoteKind::Warning);

    /// The look `kind` of a text that wraps.
    pub(crate) const fn of(kind: NoteKind) -> Self {
        Self { kind, path: false }
    }

    /// The same look for a text that is a path.
    pub(crate) const fn shown_as_path(self) -> Self {
        Self { path: true, ..self }
    }

    /// Whether the note is a warning.
    pub(crate) fn is_warning(self) -> bool {
        self.kind == NoteKind::Warning
    }
}

/// Adds the sentences of `extra` after a plain, found or unmarked note of
/// the look `look`. A note with a sentence of `extra` wraps. Returns the
/// look of the note.
pub(crate) fn followed_by(
    writer: &mut NoteWriter,
    look: NoteLook,
    extra: impl FnOnce(&mut NoteWriter),
) -> NoteLook {
    if look.is_warning() {
        return look;
    }
    let count = writer.sentence_count();
    extra(writer);
    if writer.sentence_count() == count {
        look
    } else {
        NoteLook {
            path: false,
            ..look
        }
    }
}

/// The values of an inherited value, joined with the catalog pattern
/// `note.values-join`.
struct InheritedValues<'a> {
    /// The catalog of the pattern.
    catalog: &'a Catalog,
    /// The values.
    values: &'a [String],
}

impl fmt::Display for InheritedValues<'_> {
    /// Writes the values joined with the `note.values-join` pattern.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.values {
            [] => Ok(()),
            [value] => formatter.write_str(value),
            [first, rest @ ..] => {
                let rest = InheritedValues {
                    catalog: self.catalog,
                    values: rest,
                };
                self.catalog
                    .write(formatter, note::values_join(&first.as_str(), &rest))
            }
        }
    }
}

/// The inherited value with its source in a catalog, for example
/// `Built-in default: high`.
#[derive(Clone, Copy)]
pub(crate) struct InheritedLabel<'a> {
    /// The catalog of the texts.
    catalog: &'a Catalog,
    /// The inherited value.
    inherited: &'a Inherited,
}

impl fmt::Display for InheritedLabel<'_> {
    /// Writes the values, or the word of a value that the machine picks,
    /// with the label of their source.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let source = self.inherited.source;
        match (self.inherited.values.as_slice(), self.inherited.word) {
            ([], Some(word)) => self
                .catalog
                .write(formatter, source.label(&unset_word(self.catalog, word))),
            (values, _) => {
                let values = InheritedValues {
                    catalog: self.catalog,
                    values,
                };
                self.catalog.write(formatter, source.label(&values))
            }
        }
    }
}

/// The inherited value `inherited` with its source in `catalog`. A key
/// without a value gives `None`.
pub(crate) fn inherited_label<'a>(
    catalog: &'a Catalog,
    inherited: Option<&'a Inherited>,
) -> Option<InheritedLabel<'a>> {
    inherited
        .filter(|inherited| inherited.has_value())
        .map(|inherited| InheritedLabel { catalog, inherited })
}

/// Writes the inherited value with its source, for example `Built-in
/// default: high`, as the placeholder of a text or number field and the
/// inherited entry of a dropdown. A key without an inherited value writes
/// the text `note.not-set`.
pub(crate) fn write_inherited_placeholder(
    catalog: &Catalog,
    out: &mut String,
    inherited: Option<&Inherited>,
) {
    match inherited_label(catalog, inherited) {
        Some(label) => {
            let _ = write!(out, "{label}");
        }
        None => out.push_str(catalog.text(note::NOT_SET)),
    }
}

/// Adds the sentence of the inherited value of a set field, for example
/// `Built-in default: high.` Returns whether the key has an inherited value.
fn inherited_sentence(writer: &mut NoteWriter, inherited: Option<&Inherited>) -> bool {
    let catalog = writer.catalog();
    match inherited_label(catalog, inherited) {
        Some(label) => {
            writer.message(note::sentence(&label));
            true
        }
        None => false,
    }
}

/// Returns `text` with its first letter in uppercase.
fn capitalized(text: &str) -> String {
    let mut characters = text.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => String::new(),
    }
}

/// Writes the warning of a value of the file that the launch rejects, for
/// example `Invalid value 'On', expected on or off. The launch ignores the
/// line.`
pub(crate) fn write_invalid_value(writer: &mut NoteWriter, invalid: &InvalidValue) -> NoteLook {
    match &invalid.reason {
        InvalidReason::Rejected(error) => {
            let message = capitalized(&in_catalog(writer.catalog(), error).to_string());
            writer.message(note::sentence(&message));
        }
        InvalidReason::OtherSystem => writer.message(note::other_system(&invalid.value)),
    }
    writer.text(note::LAUNCH_IGNORES);
    NoteLook::WARNING
}

/// The state of a choice field that its note shows.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ChoiceNote<'a> {
    /// The limit under which the model ignores the setting.
    pub(crate) ignored: Option<IgnoredBy>,
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
    /// The data of the fact of the value that the table gives.
    pub(crate) fact: Option<&'a FactData>,
}

/// Adds the sentence of the fact `fact` of a choice, for example `The
/// choices are auto, 15k and 24k.` or `At low the CPU runs at 4 MHz.`
fn write_fact(writer: &mut NoteWriter, fact: &FactData) {
    match fact {
        FactData::Choices(values) => {
            let choices = writer.catalog().display_list(ListKind::And, values);
            writer.message(note::choices(&choices));
        }
        FactData::CpuClock { mode, clock } => writer.message(note::cpu_clock(mode, clock)),
    }
}

/// Writes the note of a choice field: the reason that the model ignores it, the warning
/// of an invalid value, the preset of a new game, or else the inherited
/// value, the derived default and the fact of the value.
///
/// The note of a set key shows the inherited value. An unset key leaves it
/// to its dropdown, which shows it as its inherited entry. An unset
/// required key shows the inherited value as its choice, and its note tells
/// that the save writes it.
pub(crate) fn choice_note(writer: &mut NoteWriter, state: ChoiceNote) -> NoteLook {
    if let Some(ignored) = state.ignored {
        write_ignored(writer, ignored);
        return NoteLook::PLAIN;
    }
    if let Some(invalid) = state.invalid {
        return write_invalid_value(writer, invalid);
    }
    if state.preset {
        writer.text(note::PRESET);
        return NoteLook::PLAIN;
    }
    if state.set {
        inherited_sentence(writer, state.inherited);
    } else if state.required {
        writer.text(note::REQUIRED);
    }
    if state.derived {
        writer.text(note::DERIVED_DEFAULT);
    }
    if let Some(fact) = state.fact {
        write_fact(writer, fact);
    }
    NoteLook::PLAIN
}

/// A text of up to [`STEP_TEXT_CAPACITY`] bytes on the stack.
struct StackText {
    /// The bytes of the text.
    bytes: [u8; STEP_TEXT_CAPACITY],
    /// The number of bytes of the text.
    length: usize,
}

impl fmt::Write for StackText {
    /// Appends `text`. A text past the capacity fails and keeps the bytes before it.
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let end = self.length + text.len();
        if end > STEP_TEXT_CAPACITY {
            return Err(fmt::Error);
        }
        self.bytes[self.length..end].copy_from_slice(text.as_bytes());
        self.length = end;
        Ok(())
    }
}

/// The digits after the decimal point of the numbers of `range`.
pub(crate) fn decimals_of(range: NumberRange) -> usize {
    if range.integer {
        return 0;
    }
    let mut step = StackText {
        bytes: [0; STEP_TEXT_CAPACITY],
        length: 0,
    };
    if write!(step, "{}", range.step).is_err() {
        return 0;
    }
    let digits = &step.bytes[..step.length];
    digits
        .iter()
        .position(|&byte| byte == b'.')
        .map_or(0, |point| digits.len() - point - 1)
}

/// A number with a fixed count of digits after the decimal point.
struct Fixed {
    /// The number.
    value: f64,
    /// The digits after the decimal point.
    decimals: usize,
}

impl fmt::Display for Fixed {
    /// Writes the number with its digits after the decimal point.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:.*}", self.decimals, self.value)
    }
}

/// Writes the note of a number field: the reason that the model ignores it, or else
/// the range, after the inherited value of a set field. An unset field
/// shows the inherited value as its placeholder.
pub(crate) fn number_note(
    writer: &mut NoteWriter,
    ignored: Option<IgnoredBy>,
    set: bool,
    range: NumberRange,
    inherited: Option<&Inherited>,
) -> NoteLook {
    if let Some(ignored) = ignored {
        write_ignored(writer, ignored);
        return NoteLook::PLAIN;
    }
    if set {
        inherited_sentence(writer, inherited);
    }
    let decimals = decimals_of(range);
    let minimum = Fixed {
        value: range.minimum,
        decimals,
    };
    let maximum = Fixed {
        value: range.maximum,
        decimals,
    };
    writer.message(note::range(&minimum, &maximum));
    NoteLook::PLAIN
}

/// The extensions of the files of a drive with their dots, for example
/// `.hdi .nhd .thd`.
#[derive(Clone, Copy)]
pub(crate) struct ExtensionList<'a>(pub(crate) &'a FileTypes);

impl fmt::Display for ExtensionList<'_> {
    /// Writes the extensions with their dots, separated by spaces.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, extension) in self.0.extensions.iter().enumerate() {
            if index > 0 {
                formatter.write_str(" ")?;
            }
            write!(formatter, ".{extension}")?;
        }
        Ok(())
    }
}

/// Adds the sentence of the files of `types`, for example `Hard disk
/// images: .hdi .nhd .thd.`
fn extension_note(writer: &mut NoteWriter, types: &FileTypes) {
    let name = file_type_text(writer.catalog(), types, FileTypeText::Name);
    writer.message(note::extensions(&name, &ExtensionList(types)));
}

/// Writes the note of a single path field.
///
/// A path that the model ignores tells why, and a missing image is a
/// warning. A set path shows its inherited value as a path. An unset path
/// shows the inherited value as its placeholder. Without an inherited value
/// to show, the note gives the extensions of the files.
pub(crate) fn path_note(
    writer: &mut NoteWriter,
    ignored: Option<IgnoredBy>,
    missing: bool,
    set: bool,
    inherited: Option<&Inherited>,
    types: Option<&FileTypes>,
) -> NoteLook {
    if let Some(ignored) = ignored {
        write_ignored(writer, ignored);
        return NoteLook::PLAIN;
    }
    if missing {
        writer.text(note::MISSING_IMAGE);
        return NoteLook::WARNING;
    }
    if set && inherited_sentence(writer, inherited) {
        return NoteLook::PLAIN.shown_as_path();
    }
    if let Some(types) = types {
        extension_note(writer, types);
    }
    NoteLook::PLAIN
}

/// Writes the note below a media list that the file sets, with images or
/// as an empty list: the inherited list that the file overrides. A list
/// without images of the file shows the inherited list in the list itself.
/// A note of one inherited image is a path. A note of several images wraps.
pub(crate) fn media_note(
    writer: &mut NoteWriter,
    set: bool,
    inherited: Option<&Inherited>,
) -> NoteLook {
    if set
        && inherited_sentence(writer, inherited)
        && inherited.is_some_and(|inherited| inherited.values.len() == 1)
    {
        return NoteLook::PLAIN.shown_as_path();
    }
    NoteLook::PLAIN
}

/// Writes the text of a media list without images. An empty list of the
/// file and a drive without an inherited list start without an image. A
/// list that inherits images shows them with their source.
pub(crate) fn write_empty_drive_hint(
    catalog: &Catalog,
    out: &mut String,
    types: Option<&FileTypes>,
    inherited: Option<&Inherited>,
    empty_list: bool,
) {
    match (inherited_label(catalog, inherited), types) {
        (Some(label), _) if !empty_list => {
            let _ = write!(out, "{label}");
        }
        (Some(_) | None, Some(types)) => {
            out.push_str(file_type_text(catalog, types, FileTypeText::EmptyDrive));
        }
        (Some(_) | None, None) => out.push_str(catalog.text(note::EMPTY_DISK_DRIVE)),
    }
}

/// Writes the caption of the checkbox that empties the drive over its
/// inherited list.
pub(crate) fn write_empty_list_caption(
    catalog: &Catalog,
    out: &mut String,
    types: Option<&FileTypes>,
) {
    match types {
        Some(types) => out.push_str(file_type_text(catalog, types, FileTypeText::StartWithout)),
        None => out.push_str(catalog.text(note::START_WITHOUT_DISK)),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use common::CpuMode;
    use machine_factory::config::{
        phrases::Megahertz,
        settings::{find_setting, value_error::ValueError},
    };

    use super::*;
    use crate::{
        editor::inherited::InheritedSource,
        texts::{english, english_text, test_catalog},
    };

    /// The English label of `inherited` with its source. A key without a
    /// value gives `None`.
    pub(crate) fn english_inherited_label(inherited: Option<&Inherited>) -> Option<String> {
        inherited_label(english(), inherited).map(|label| label.to_string())
    }

    /// The text and the look of a note.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub(crate) struct Note {
        /// The text.
        pub(crate) text: String,
        /// The look.
        pub(crate) look: NoteLook,
    }

    impl Note {
        /// A plain note with `text`.
        pub(crate) fn plain(text: &str) -> Self {
            Self::of(text, NoteLook::PLAIN)
        }

        /// A warning with `text`.
        pub(crate) fn warning(text: &str) -> Self {
            Self::of(text, NoteLook::WARNING)
        }

        /// A note with `text` about a thing that exists.
        pub(crate) fn found(text: &str) -> Self {
            Self::of(text, NoteLook::of(NoteKind::Found))
        }

        /// A status note with `text` without a mark.
        pub(crate) fn unmarked(text: &str) -> Self {
            Self::of(text, NoteLook::of(NoteKind::Unmarked))
        }

        /// A note with `text` and `look`.
        pub(crate) fn of(text: &str, look: NoteLook) -> Self {
            Self {
                text: text.to_owned(),
                look,
            }
        }

        /// The same note with its text as a path.
        pub(crate) fn shown_as_path(self) -> Self {
            Self {
                look: self.look.shown_as_path(),
                ..self
            }
        }
    }

    /// The note that `build` writes with the English texts.
    pub(crate) fn english_note(build: impl FnOnce(&mut NoteWriter) -> NoteLook) -> Note {
        catalog_note(english(), build)
    }

    /// The note that `build` writes with the texts of `catalog`.
    pub(crate) fn catalog_note(
        catalog: &Catalog,
        build: impl FnOnce(&mut NoteWriter) -> NoteLook,
    ) -> Note {
        let mut scratch = String::new();
        let mut writer = NoteWriter::new(catalog, &mut scratch);
        let look = build(&mut writer);
        let mut text = String::new();
        writer.finish(&mut text);
        Note { text, look }
    }

    /// The inherited `values` of `source`.
    fn inherited(values: &[&str], source: InheritedSource) -> Inherited {
        Inherited::new(
            values.iter().map(|value| (*value).to_owned()).collect(),
            source,
        )
    }

    /// The file types of the hard disks of the first family that has them.
    fn hard_disks() -> Option<&'static FileTypes> {
        find_setting("hdd1").and_then(|setting| setting.file_types.first())
    }

    /// The English placeholder of `inherited`.
    fn placeholder(inherited: Option<&Inherited>) -> String {
        english_text(|catalog, out| write_inherited_placeholder(catalog, out, inherited))
    }

    /// The German texts of the note join, the sentence and the values join.
    const GERMAN: &str = "[note]\njoin = {first} | {second}\nsentence = {text}!\n";

    /// Verifies that the word of a value that the machine picks shows in the catalog with its
    /// source.
    #[test]
    fn an_unset_word_shows_in_the_catalog() {
        let word = Inherited {
            values: Vec::new(),
            word: Some("auto"),
            source: InheritedSource::BuiltIn,
        };
        assert_eq!(placeholder(Some(&word)), "Built-in default: auto");
        let german = test_catalog("[unset]\nauto = automatisch\n");
        assert_eq!(
            inherited_label(&german, Some(&word)).map(|label| label.to_string()),
            Some("Built-in default: automatisch".to_owned())
        );
    }

    /// Verifies that the sentence of an inherited value follows the catalog patterns and that the placeholder shows the inherited value or the text of an unset key.
    #[test]
    fn inherited_values_follow_the_catalog_patterns() {
        let german = test_catalog(GERMAN);
        let mut scratch = String::new();
        let mut writer = NoteWriter::new(&german, &mut scratch);
        let label = InheritedLabel {
            catalog: &german,
            inherited: &inherited(&["a", "b", "c"], InheritedSource::BuiltIn),
        };
        writer.message(note::sentence(&label));
        writer.sentence("Zwei.");
        let mut text = String::new();
        writer.finish(&mut text);
        assert_eq!(text, "Built-in default: a, b, c! | Zwei.");
        assert_eq!(
            placeholder(Some(&inherited(&["0"], InheritedSource::BuiltIn))),
            "Built-in default: 0"
        );
        assert_eq!(
            placeholder(Some(&inherited(&[], InheritedSource::BuiltIn))),
            "Not set"
        );
        assert_eq!(placeholder(None), "Not set");
    }

    /// Verifies the choice note of an unset, a set and a required key, and the parts that win over them.
    #[test]
    fn choice_notes_show_the_inherited_value_only_for_a_set_key() {
        let inherited = inherited(&["v2"], InheritedSource::BuiltIn);
        let invalid = InvalidValue {
            value: "On".to_owned(),
            reason: InvalidReason::Rejected(ValueError::OnOff {
                value: "On".to_owned(),
            }),
        };
        let note = |state: ChoiceNote| english_note(|writer| choice_note(writer, state));
        assert_eq!(
            note(ChoiceNote {
                invalid: Some(&invalid),
                preset: true,
                ..ChoiceNote::default()
            }),
            Note::warning("Invalid value 'On', expected on or off. The launch ignores the line.")
        );
        assert_eq!(
            note(ChoiceNote {
                preset: true,
                set: true,
                required: true,
                inherited: Some(&inherited),
                ..ChoiceNote::default()
            }),
            Note::plain("The model is preset from the system default.")
        );
        let fact = FactData::Choices(vec!["v1s", "v1h", "v2", "n", "n80", "n80sr"]);
        assert_eq!(
            note(ChoiceNote {
                set: true,
                inherited: Some(&inherited),
                fact: Some(&fact),
                ..ChoiceNote::default()
            }),
            Note::plain("Built-in default: v2. The choices are v1s, v1h, v2, n, n80 and n80sr.")
        );
        assert_eq!(
            note(ChoiceNote {
                inherited: Some(&inherited),
                fact: Some(&fact),
                ..ChoiceNote::default()
            }),
            Note::plain("The choices are v1s, v1h, v2, n, n80 and n80sr.")
        );
        let clock = FactData::CpuClock {
            mode: CpuMode::Low,
            clock: Megahertz(3_993_600),
        };
        assert_eq!(
            note(ChoiceNote {
                set: true,
                inherited: Some(&self::inherited(&["low"], InheritedSource::BuiltIn)),
                derived: true,
                fact: Some(&clock),
                ..ChoiceNote::default()
            }),
            Note::plain(
                "Built-in default: low. The default follows the boot mode. At low the CPU runs \
                 at 4 MHz."
            )
        );
        assert_eq!(
            note(ChoiceNote {
                inherited: Some(&inherited),
                derived: true,
                ..ChoiceNote::default()
            }),
            Note::plain("The default follows the boot mode.")
        );
        let model = self::inherited(&["PC9801RA"], InheritedSource::SystemDefaults);
        assert_eq!(
            note(ChoiceNote {
                required: true,
                inherited: Some(&model),
                ..ChoiceNote::default()
            }),
            Note::plain("The file inherits this value. The save writes it.")
        );
        assert_eq!(
            note(ChoiceNote {
                set: true,
                required: true,
                inherited: Some(&model),
                ..ChoiceNote::default()
            }),
            Note::plain("System default: PC9801RA.")
        );
        assert_eq!(note(ChoiceNote::default()), Note::default());
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
        let note = |set: bool, inherited: Option<&Inherited>| {
            english_note(|writer| number_note(writer, None, set, range, inherited))
        };
        assert_eq!(
            note(true, Some(&inherited)),
            Note::plain("Built-in default: 0. The range is 0 to 3.")
        );
        assert_eq!(
            note(false, Some(&inherited)),
            Note::plain("The range is 0 to 3.")
        );
        assert_eq!(note(true, None), Note::plain("The range is 0 to 3."));
        let fraction = NumberRange {
            minimum: 0.0,
            maximum: 1.0,
            step: 0.05,
            integer: false,
        };
        assert_eq!(
            english_note(|writer| number_note(writer, None, false, fraction, None)),
            Note::plain("The range is 0.00 to 1.00.")
        );
    }

    /// Verifies the single path note in the unset and the set state, with and without an inherited value, and the warning that wins.
    #[test]
    fn path_notes_show_the_inherited_value_of_a_set_path_or_the_extensions() {
        let types = hard_disks();
        let settings = inherited(&["games/boot.hdi"], InheritedSource::SettingsFile);
        let nothing = inherited(&[], InheritedSource::BuiltIn);
        let note = |missing: bool, set: bool, inherited: Option<&Inherited>, types| {
            english_note(|writer| path_note(writer, None, missing, set, inherited, types))
        };
        assert_eq!(
            note(false, true, Some(&settings), types),
            Note::plain("Settings default: games/boot.hdi.").shown_as_path()
        );
        let extensions = types.map_or_else(Note::default, |types| {
            Note::plain(&format!("Hard disk images: {}.", ExtensionList(types)))
        });
        assert_eq!(note(false, false, Some(&settings), types), extensions);
        assert_eq!(note(false, true, Some(&nothing), types), extensions);
        assert_eq!(note(false, false, Some(&nothing), types), extensions);
        assert_eq!(
            note(true, true, Some(&settings), types),
            Note::warning(
                "The image does not exist. The file saves, and the launch fails while it is \
                 missing."
            )
        );
        assert_eq!(note(false, false, None, None), Note::default());
        if let Some(types) = types {
            assert!(ExtensionList(types).to_string().starts_with('.'));
        }
    }

    /// Verifies the note below a media list: the inherited list of a set list only, as a path for one image.
    #[test]
    fn media_notes_show_the_inherited_list_of_a_set_list() {
        let settings = inherited(&["games/boot.d88"], InheritedSource::SettingsFile);
        let nothing = inherited(&[], InheritedSource::BuiltIn);
        let note = |set: bool, inherited: Option<&Inherited>| {
            english_note(|writer| media_note(writer, set, inherited))
        };
        assert_eq!(
            note(true, Some(&settings)),
            Note::plain("Settings default: games/boot.d88.").shown_as_path()
        );
        let two = inherited(&["a.d88", "b.d88"], InheritedSource::SettingsFile);
        assert_eq!(
            note(true, Some(&two)),
            Note::plain("Settings default: a.d88, b.d88.")
        );
        assert_eq!(note(false, Some(&settings)), Note::default());
        assert_eq!(note(true, Some(&nothing)), Note::default());
        assert_eq!(note(true, None), Note::default());
    }

    /// Verifies the hint of an empty drive: the noun of its images, the inherited list and the empty list of the file.
    #[test]
    fn empty_drives_tell_what_they_start_with() {
        let cdrom = find_setting("cdrom").and_then(|setting| setting.file_types.first());
        let fdd1 = find_setting("fdd1").and_then(|setting| setting.file_types.first());
        let settings = inherited(&["games/boot.d88"], InheritedSource::SettingsFile);
        let nothing = inherited(&[], InheritedSource::BuiltIn);
        let hint = |types, inherited: Option<&Inherited>, empty_list: bool| {
            english_text(|catalog, out| {
                write_empty_drive_hint(catalog, out, types, inherited, empty_list);
            })
        };
        let caption =
            |types| english_text(|catalog, out| write_empty_list_caption(catalog, out, types));
        assert_eq!(
            hint(fdd1, Some(&nothing), false),
            "Empty. The drive starts without a disk."
        );
        if cdrom.is_some() {
            assert_eq!(
                hint(cdrom, None, false),
                "Empty. The drive starts without a disc."
            );
            assert_eq!(caption(cdrom), "Start without a disc");
        }
        assert_eq!(
            hint(fdd1, Some(&settings), false),
            "Settings default: games/boot.d88"
        );
        assert_eq!(
            hint(fdd1, Some(&settings), true),
            "Empty. The drive starts without a disk."
        );
        assert_eq!(
            hint(None, None, false),
            "Empty. The drive starts without a disk."
        );
        assert_eq!(caption(None), "Start without a disk");
    }
}
