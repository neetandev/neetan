//! The fields of the game editor: the value of each key when the editor
//! opened and its current value, the choices of a dropdown, and the lines
//! that a changed field writes into the document.

use std::borrow::Cow;

use gui::ui::ListState;
use machine_factory::config::{
    AUTO_LANGUAGE,
    settings::{Families, MACHINE_KEY, PathKind, Section, Setting, ValueKind},
};

use super::form::{EditorSection, FieldSource};
use crate::{
    config::{
        ConfigDocument, check_value,
        metadata::{META_PREFIX, YEAR_KEY, parse_year},
    },
    texts::ENGLISH_TAG,
};

/// The values of an on/off setting in the order of its dropdown.
pub(crate) const ON_OFF_CHOICES: [&str; 2] = ["on", "off"];

/// The values of a language setting that need no language file, in the
/// order of its dropdown.
pub(crate) const BUILT_IN_LANGUAGE_CHOICES: [&str; 2] = [AUTO_LANGUAGE, ENGLISH_TAG];

/// A value of the file that the launch rejects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InvalidValue {
    /// The value as the file writes it.
    pub(crate) value: String,
    /// Why the launch rejects it.
    pub(crate) message: String,
}

/// The value of a dropdown field.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ChoiceValue {
    /// The index of the chosen value. `None` inherits the value.
    pub(crate) selected: Option<usize>,
    /// The value of the file that the launch rejects. It stays in the file
    /// until the user picks another entry.
    pub(crate) invalid: Option<InvalidValue>,
}

/// The value of one field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FieldValue {
    /// A choice or on/off setting.
    Choice(ChoiceValue),
    /// A metadata key, a number or a single path. An empty text is unset.
    Text(String),
    /// A path list. An empty list is unset and inherits the list.
    List(Vec<String>),
    /// A path list that the file empties with one empty value, such as
    /// `fdd1 =`. It overrides the inherited list.
    EmptyList,
    /// A single image of a drive that the file empties with an empty value,
    /// such as `hdd1 =`. It overrides the inherited image.
    EmptyPath,
}

/// One field of the editor.
#[derive(Debug, Clone)]
pub(crate) struct Field {
    /// The section of the field.
    pub(crate) section: EditorSection,
    /// The key of the field.
    pub(crate) source: FieldSource,
    /// The values of a dropdown field in the order of its entries.
    pub(crate) choices: Vec<Cow<'static, str>>,
    /// Whether a save writes the key in every case. Its dropdown has no
    /// inherited entry, and an unset field saves its inherited value.
    pub(crate) required: bool,
    /// The value when the editor opened or saved last.
    pub(crate) initial: FieldValue,
    /// The current value.
    pub(crate) value: FieldValue,
    /// The selection and the scroll position of a path list.
    pub(crate) list: ListState,
}

/// Returns the values of the dropdown of `setting` that one of the
/// families `families` accepts. A language setting has the values that need
/// no language file. A field of another kind has none.
pub(crate) fn choices_of(setting: &Setting, families: Families) -> Vec<Cow<'static, str>> {
    match setting.kind {
        ValueKind::Choice(_) => setting
            .choices_for_families(families)
            .map(|choice| Cow::Borrowed(choice.value))
            .collect(),
        ValueKind::OnOff => ON_OFF_CHOICES.map(Cow::Borrowed).to_vec(),
        ValueKind::Language => BUILT_IN_LANGUAGE_CHOICES.map(Cow::Borrowed).to_vec(),
        ValueKind::Number(_) | ValueKind::Path(_) | ValueKind::PathList => Vec::new(),
    }
}

/// Returns whether `setting` is the single image of a drive, such as `hdd1`.
/// An empty value of such a key empties the drive over the inherited image.
pub(crate) fn is_single_image(setting: &Setting) -> bool {
    setting.kind == ValueKind::Path(PathKind::File) && setting.section == Some(Section::Media)
}

/// Returns the dropdown value of the file value `value` of `setting` among
/// `choices`. A value that the launch accepts picks its choice, an exact
/// match before a match that ignores the case. A value that the launch
/// rejects, and a value of another family, is invalid with the reason.
fn choice_value(setting: &Setting, choices: &[Cow<'static, str>], value: &str) -> ChoiceValue {
    let invalid = |message: String| ChoiceValue {
        selected: None,
        invalid: Some(InvalidValue {
            value: value.to_owned(),
            message,
        }),
    };
    if let Err(message) = check_value(setting, value) {
        return invalid(message);
    }
    let selected = choices
        .iter()
        .position(|choice| *choice == value)
        .or_else(|| {
            choices
                .iter()
                .position(|choice| choice.eq_ignore_ascii_case(value))
        });
    match selected {
        Some(index) => ChoiceValue {
            selected: Some(index),
            invalid: None,
        },
        None => invalid(format!("{value} belongs to another system")),
    }
}

impl Field {
    /// The field of `source` in `section` for the families `families`, with
    /// the value of `document`. A `required` field saves its key in every
    /// case.
    ///
    /// A single key reads its last active line. A single image of a drive
    /// whose last line holds an empty value is an empty image that
    /// overrides the inherited image. A path list reads every active line
    /// of its key in file order and leaves out empty values. A path list of
    /// only empty values is an empty list that overrides the inherited list.
    pub(crate) fn read(
        section: EditorSection,
        source: FieldSource,
        families: Families,
        document: &ConfigDocument,
        required: bool,
    ) -> Self {
        let choices = source
            .setting()
            .map(|setting| choices_of(setting, families))
            .unwrap_or_default();
        Self::with_choices(section, source, choices, document, required)
    }

    /// The field of `source` in `section` with the dropdown values
    /// `choices` and the value of `document`, read as [`Field::read`] does.
    pub(crate) fn with_choices(
        section: EditorSection,
        source: FieldSource,
        choices: Vec<Cow<'static, str>>,
        document: &ConfigDocument,
        required: bool,
    ) -> Self {
        let key = source.key();
        let text = || document.get(key).unwrap_or_default().to_owned();
        let value = match source.setting() {
            None => FieldValue::Text(text()),
            Some(setting) => match setting.kind {
                ValueKind::Path(_) if is_single_image(setting) && document.get(key) == Some("") => {
                    FieldValue::EmptyPath
                }
                ValueKind::Number(_) | ValueKind::Path(_) => FieldValue::Text(text()),
                ValueKind::Choice(_) | ValueKind::OnOff | ValueKind::Language => {
                    FieldValue::Choice(match document.get(key) {
                        None => ChoiceValue::default(),
                        Some(value) => choice_value(setting, &choices, value),
                    })
                }
                ValueKind::PathList => {
                    let lines = document.get_all(key);
                    let values: Vec<String> = lines
                        .iter()
                        .filter(|value| !value.is_empty())
                        .map(|value| (*value).to_owned())
                        .collect();
                    if values.is_empty() && !lines.is_empty() {
                        FieldValue::EmptyList
                    } else {
                        FieldValue::List(values)
                    }
                }
            },
        };
        Self {
            section,
            source,
            choices,
            required,
            initial: value.clone(),
            value,
            list: ListState::new(),
        }
    }

    /// The key of the field.
    pub(crate) fn key(&self) -> &'static str {
        self.source.key()
    }

    /// Whether the value sets the key.
    pub(crate) fn is_set(&self) -> bool {
        match &self.value {
            FieldValue::Choice(choice) => choice.selected.is_some() || choice.invalid.is_some(),
            FieldValue::Text(text) => !text.trim().is_empty(),
            FieldValue::List(values) => !values.is_empty(),
            FieldValue::EmptyList | FieldValue::EmptyPath => true,
        }
    }

    /// Whether the value differs from the value when the editor opened or saved last.
    pub(crate) fn changed(&self) -> bool {
        self.value != self.initial
    }

    /// Takes the current value as the value of the file, after a save.
    pub(crate) fn mark_saved(&mut self) {
        self.initial = self.value.clone();
    }

    /// Unsets the field: a dropdown inherits its value, a text and a list
    /// are empty and inherit their value.
    pub(crate) fn unset(&mut self) {
        self.value = match self.value {
            FieldValue::Choice(_) => FieldValue::Choice(ChoiceValue::default()),
            FieldValue::Text(_) | FieldValue::EmptyPath => FieldValue::Text(String::new()),
            FieldValue::List(_) | FieldValue::EmptyList => FieldValue::List(Vec::new()),
        };
    }

    /// The values that the current value writes into the file. An unset
    /// field writes none, an empty list and an empty image write one empty
    /// value.
    pub(crate) fn values(&self) -> Vec<String> {
        match &self.value {
            FieldValue::Choice(choice) => match (choice.selected, &choice.invalid) {
                (Some(index), _) => self
                    .choices
                    .get(index)
                    .map(|value| vec![value.to_string()])
                    .unwrap_or_default(),
                (None, Some(invalid)) => vec![invalid.value.clone()],
                (None, None) => Vec::new(),
            },
            FieldValue::Text(text) => {
                let text = text.trim();
                if text.is_empty() {
                    Vec::new()
                } else {
                    vec![text.to_owned()]
                }
            }
            FieldValue::List(values) => values.clone(),
            FieldValue::EmptyList | FieldValue::EmptyPath => vec![String::new()],
        }
    }

    /// The text of a text field. Another field gives an empty text.
    pub(crate) fn text(&self) -> &str {
        match &self.value {
            FieldValue::Text(text) => text,
            FieldValue::Choice(_)
            | FieldValue::List(_)
            | FieldValue::EmptyList
            | FieldValue::EmptyPath => "",
        }
    }

    /// The text of a text field to edit.
    pub(crate) fn text_mut(&mut self) -> Option<&mut String> {
        match &mut self.value {
            FieldValue::Text(text) => Some(text),
            FieldValue::Choice(_)
            | FieldValue::List(_)
            | FieldValue::EmptyList
            | FieldValue::EmptyPath => None,
        }
    }

    /// The images of a path list. An empty list has none.
    pub(crate) fn images(&self) -> &[String] {
        match &self.value {
            FieldValue::List(values) => values,
            FieldValue::Choice(_)
            | FieldValue::Text(_)
            | FieldValue::EmptyList
            | FieldValue::EmptyPath => &[],
        }
    }

    /// Whether the path list is an empty list that overrides the inherited list.
    pub(crate) fn is_empty_list(&self) -> bool {
        self.value == FieldValue::EmptyList
    }

    /// Whether the single image is an empty image that overrides the inherited image.
    pub(crate) fn is_empty_path(&self) -> bool {
        self.value == FieldValue::EmptyPath
    }

    /// The selected entry of a dropdown field. `None` inherits the value.
    pub(crate) fn selected(&self) -> Option<usize> {
        match &self.value {
            FieldValue::Choice(choice) => choice.selected,
            FieldValue::Text(_)
            | FieldValue::List(_)
            | FieldValue::EmptyList
            | FieldValue::EmptyPath => None,
        }
    }

    /// Picks the entry `selected` of a dropdown field. `None` inherits the value.
    pub(crate) fn select(&mut self, selected: Option<usize>) {
        self.value = FieldValue::Choice(ChoiceValue {
            selected,
            invalid: None,
        });
    }

    /// The chosen value of a dropdown field.
    pub(crate) fn chosen(&self) -> Option<&str> {
        match &self.value {
            FieldValue::Choice(choice) => choice
                .selected
                .and_then(|index| self.choices.get(index).map(AsRef::as_ref)),
            FieldValue::Text(_)
            | FieldValue::List(_)
            | FieldValue::EmptyList
            | FieldValue::EmptyPath => None,
        }
    }

    /// The value of the file that the launch rejects.
    pub(crate) fn invalid_value(&self) -> Option<&InvalidValue> {
        match &self.value {
            FieldValue::Choice(choice) => choice.invalid.as_ref(),
            FieldValue::Text(_)
            | FieldValue::List(_)
            | FieldValue::EmptyList
            | FieldValue::EmptyPath => None,
        }
    }

    /// Writes a changed field into `document`.
    ///
    /// A set metadata key joins the other `meta.*` lines, a set path list
    /// replaces the group of its key, and a set single key updates its first
    /// line. A `machine` line that the file lacks joins the document after
    /// the `meta.*` lines, or before the first key line of a file without
    /// them. Another key that the file lacks joins it after its last line.
    /// An unset field removes every active line of its key. A field without
    /// a change leaves the document as it is.
    pub(crate) fn write(&self, document: &mut ConfigDocument) {
        if !self.changed() {
            return;
        }
        let key = self.key();
        let values = self.values();
        let list = self
            .source
            .setting()
            .is_some_and(|setting| setting.kind == ValueKind::PathList);
        match (values.first(), list) {
            (_, true) => document.set_all(key, &values),
            (None, false) => document.remove(key),
            (Some(value), false) if self.source.setting().is_none() || key == MACHINE_KEY => {
                document.set_in_group(key, value, META_PREFIX);
            }
            (Some(value), false) => document.set(key, value),
        }
    }

    /// The error of a value that the file cannot hold, for a number or a
    /// year that lies outside its range.
    pub(crate) fn error(&self) -> Option<String> {
        let text = self.text().trim();
        if text.is_empty() {
            return None;
        }
        let label = self.source.label();
        match self.source.setting() {
            None if self.key() == YEAR_KEY => parse_year(text)
                .err()
                .map(|message| format!("{label}: {message}")),
            None => None,
            Some(setting) => match setting.kind {
                ValueKind::Number(_) => check_value(setting, text)
                    .err()
                    .map(|message| format!("{label}: {message}")),
                ValueKind::Choice(_)
                | ValueKind::OnOff
                | ValueKind::Path(_)
                | ValueKind::PathList
                | ValueKind::Language => None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use machine_factory::config::{
        MachineSelection, Target, settings::find_setting, systems::compiled_systems,
    };

    use super::*;
    use crate::config::metadata::{METADATA_FIELDS, TITLE_KEY};

    /// The first compiled family.
    fn first_target() -> Target {
        let system = compiled_systems().next().expect("one system is compiled");
        MachineSelection::of_system(system)
            .expect("compiled system")
            .target()
    }

    /// The field of the table key `key` read from `text`.
    fn setting_field(key: &str, text: &str) -> (Field, ConfigDocument) {
        let document = ConfigDocument::parse(text);
        let setting = find_setting(key).expect("table key");
        let field = Field::read(
            EditorSection::Machine,
            FieldSource::Setting(setting),
            Families::of(first_target()),
            &document,
            false,
        );
        (field, document)
    }

    /// Writes `field` into `document` and returns the text.
    fn written(field: &Field, mut document: ConfigDocument) -> String {
        field.write(&mut document);
        document.to_text()
    }

    /// Verifies the set, unset and invalid values of an on/off field.
    #[test]
    fn on_off_fields_pick_unset_and_keep_an_invalid_value() {
        let (mut field, document) = setting_field("crt", "; display\ncrt=off\nscaling = nearest\n");
        assert_eq!(field.choices, ON_OFF_CHOICES);
        assert_eq!(field.chosen(), Some("off"));
        assert!(field.is_set() && !field.changed());
        assert_eq!(written(&field, document.clone()), document.to_text());

        field.value = FieldValue::Choice(ChoiceValue {
            selected: Some(0),
            invalid: None,
        });
        assert!(field.changed());
        assert_eq!(
            written(&field, document.clone()),
            "; display\ncrt = on\nscaling = nearest\n"
        );
        field.unset();
        assert!(!field.is_set());
        assert_eq!(
            written(&field, document.clone()),
            "; display\nscaling = nearest\n"
        );

        for (text, value) in [("crt = maybe\n", "maybe"), ("crt = On\n", "On")] {
            let (mut field, document) = setting_field("crt", text);
            let invalid = field.invalid_value().expect("the launch rejects the value");
            assert_eq!(invalid.value, value);
            assert_eq!(
                invalid.message,
                format!("invalid value '{value}', expected on or off")
            );
            assert!(field.is_set());
            assert_eq!(field.values(), [value]);
            assert_eq!(written(&field, document.clone()), text);
            field.value = FieldValue::Choice(ChoiceValue::default());
            assert_eq!(written(&field, document), "");
        }
    }

    /// Verifies that a choice field lists the values of the family, takes a value that the launch accepts and writes the value of the table.
    #[test]
    fn choice_fields_list_the_values_of_the_family() {
        let (field, document) = setting_field("scaling", "scaling = pixelart\n");
        assert_eq!(field.choices, ["nearest", "bilinear", "pixelart"]);
        assert_eq!(field.chosen(), Some("pixelart"));
        let mut field = field;
        field.value = FieldValue::Choice(ChoiceValue {
            selected: Some(1),
            invalid: None,
        });
        assert_eq!(written(&field, document), "scaling = bilinear\n");

        let (field, _) = setting_field(MACHINE_KEY, "");
        assert!(!field.choices.is_empty());
        let target = first_target();
        for value in &field.choices {
            let model: MachineSelection = value.parse().expect("a model");
            assert_eq!(model.target(), target);
        }
    }

    /// Verifies that an added `machine` line follows the metadata lines, or comes before the first key line of a file without them, and that an existing line keeps its place.
    #[test]
    fn an_added_machine_line_follows_the_metadata() {
        let machine = |text: &str| {
            let document = ConfigDocument::parse(text);
            let mut field = Field::read(
                EditorSection::Machine,
                FieldSource::Setting(find_setting(MACHINE_KEY).expect("table key")),
                Families::of(first_target()),
                &document,
                true,
            );
            field.select(Some(0));
            let model = field.choices[0].clone();
            (written(&field, document), model)
        };
        let (text, model) =
            machine("; header\nmeta.title = Edge\ncrt = on\nscaling = nearest\n; trailing\n");
        assert_eq!(
            text,
            format!(
                "; header\nmeta.title = Edge\nmachine = {model}\ncrt = on\nscaling = nearest\n\
                 ; trailing\n"
            )
        );
        let (text, model) = machine("; header\ncrt = on\n; trailing\n");
        assert_eq!(
            text,
            format!("; header\nmachine = {model}\ncrt = on\n; trailing\n")
        );
        let (text, model) = machine("; only a comment\n");
        assert_eq!(text, format!("; only a comment\nmachine = {model}\n"));
        let (text, model) = machine("meta.title = Edge\ncrt = on\nmachine = X\n; trailing\n");
        assert_eq!(
            text,
            format!("meta.title = Edge\ncrt = on\nmachine = {model}\n; trailing\n")
        );
    }

    /// Verifies that a single path field sets its first line, drops the later lines and removes every line when it is cleared.
    #[test]
    fn path_fields_set_and_clear_their_key() {
        let (mut field, document) = setting_field(
            "hdd1",
            "hdd1 = games/a.hdi\n;hdd1 = games/old.hdi\nhdd1=games/b.hdi\n",
        );
        assert_eq!(field.text(), "games/b.hdi");
        field.value = FieldValue::Text("games/c.hdi".to_owned());
        assert_eq!(
            written(&field, document.clone()),
            "hdd1 = games/c.hdi\n;hdd1 = games/old.hdi\n"
        );
        field.value = FieldValue::Text("   ".to_owned());
        assert!(!field.is_set());
        assert_eq!(written(&field, document), ";hdd1 = games/old.hdi\n");
    }

    /// Verifies that a path list keeps its order, leaves out empty values and replaces its group in place.
    #[test]
    fn path_lists_replace_their_group_in_place() {
        let (mut field, document) = setting_field(
            "fdd1",
            "machine = X\nfdd1 = a.d88\n; between\nfdd1 =\nfdd1 = b.d88\ncrt = on\n",
        );
        assert_eq!(
            field.value,
            FieldValue::List(vec!["a.d88".to_owned(), "b.d88".to_owned()])
        );
        assert_eq!(
            written(&field, document.clone()),
            "machine = X\nfdd1 = a.d88\n; between\nfdd1 =\nfdd1 = b.d88\ncrt = on\n"
        );
        field.value = FieldValue::List(vec!["b.d88".to_owned(), "a.d88".to_owned()]);
        assert_eq!(
            written(&field, document.clone()),
            "machine = X\nfdd1 = b.d88\n; between\nfdd1 = a.d88\ncrt = on\n"
        );
        field.value = FieldValue::List(vec!["c.d88".to_owned()]);
        assert_eq!(
            written(&field, document.clone()),
            "machine = X\nfdd1 = c.d88\n; between\ncrt = on\n"
        );
        field.unset();
        assert_eq!(
            written(&field, document),
            "machine = X\n; between\ncrt = on\n"
        );
    }

    /// Verifies that a group of only empty values is an empty list that the file keeps, writes and drops on unset.
    #[test]
    fn an_empty_value_is_an_empty_list() {
        let (mut field, document) = setting_field("fdd1", "fdd1 =\nfdd1 =\ncrt = on\n");
        assert!(field.is_empty_list() && field.is_set());
        assert!(field.images().is_empty());
        assert_eq!(field.values(), [""]);
        assert_eq!(
            written(&field, document.clone()),
            "fdd1 =\nfdd1 =\ncrt = on\n"
        );
        field.unset();
        assert!(!field.is_set());
        assert_eq!(written(&field, document), "crt = on\n");

        let (mut field, document) = setting_field("fdd1", "crt = on\n");
        assert_eq!(field.value, FieldValue::List(Vec::new()));
        field.value = FieldValue::EmptyList;
        assert_eq!(written(&field, document), "crt = on\nfdd1 =\n");
    }

    /// Verifies that an empty value of a single image is an empty image that the file keeps, writes and drops on unset, and that an empty value of another single path is unset.
    #[test]
    fn an_empty_value_is_an_empty_image() {
        let (mut field, document) = setting_field("hdd1", "hdd1 = a.hdi\nhdd1 =\ncrt = on\n");
        assert!(field.is_empty_path() && field.is_set());
        assert_eq!(field.text(), "");
        assert_eq!(field.values(), [""]);
        assert_eq!(
            written(&field, document.clone()),
            "hdd1 = a.hdi\nhdd1 =\ncrt = on\n"
        );
        field.unset();
        assert!(!field.is_set());
        assert_eq!(field.value, FieldValue::Text(String::new()));
        assert_eq!(written(&field, document), "crt = on\n");

        let (mut field, document) = setting_field("hdd1", "crt = on\n");
        field.value = FieldValue::EmptyPath;
        assert_eq!(written(&field, document), "crt = on\nhdd1 =\n");

        let printer = find_setting("printer").expect("printer is a table key");
        assert!(!is_single_image(printer));
        let (field, _) = setting_field("printer", "printer =\n");
        assert!(!field.is_set());
    }

    /// Verifies that a number field checks its range and clears to unset.
    #[test]
    fn number_fields_check_their_range() {
        let Some(setting) = find_setting("pc6000-phase") else {
            return;
        };
        let document = ConfigDocument::parse("pc6000-phase = 1\n");
        let mut field = Field::read(
            EditorSection::Display,
            FieldSource::Setting(setting),
            Families::of(first_target()),
            &document,
            false,
        );
        assert_eq!(field.text(), "1");
        assert_eq!(field.error(), None);
        field.value = FieldValue::Text("7".to_owned());
        assert!(
            field
                .error()
                .is_some_and(|error| error.starts_with("Color phase: "))
        );
        field.value = FieldValue::Text(String::new());
        assert_eq!(field.error(), None);
        assert_eq!(written(&field, document), "");
    }

    /// Verifies that a new metadata key joins the other metadata lines and that a bad year is an error.
    #[test]
    fn metadata_fields_join_their_group_and_check_the_year() {
        let document = ConfigDocument::parse("meta.title = Edge\nmachine = X\n");
        let field = |key: &str| {
            let metadata = METADATA_FIELDS
                .iter()
                .find(|field| field.key == key)
                .expect("metadata key");
            Field::read(
                EditorSection::Metadata,
                FieldSource::Metadata(*metadata),
                Families::of(first_target()),
                &document,
                false,
            )
        };
        let mut year = field(YEAR_KEY);
        assert!(!year.is_set());
        year.value = FieldValue::Text("1993".to_owned());
        assert_eq!(year.error(), None);
        assert_eq!(
            written(&year, document.clone()),
            "meta.title = Edge\nmeta.year = 1993\nmachine = X\n"
        );
        year.value = FieldValue::Text("93".to_owned());
        assert_eq!(
            year.error().as_deref(),
            Some("Year: '93' is not a year from 1970 to 2100")
        );
        let mut title = field(TITLE_KEY);
        assert_eq!(title.text(), "Edge");
        title.unset();
        assert_eq!(written(&title, document), "machine = X\n");
    }
}
