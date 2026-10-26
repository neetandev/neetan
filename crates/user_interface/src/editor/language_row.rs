//! The language row of the settings page: the dropdown of the language setting with the names
//! of the languages, its inherited entry and its note. The texts refill only when the catalog
//! generation or their data changes.

use std::borrow::Cow;

use gui::ui::{Dropdown, TextBuffer, TextValue, UiContext};
use i18n::{Catalog, is_language_tag};
use machine_factory::config::{
    AUTO_LANGUAGE,
    settings::{GUI_LANGUAGE_KEY, Setting},
};

use super::{
    Editor, Subject,
    fields::{BUILT_IN_LANGUAGE_CHOICES, Field},
    hints::{EditorWidget, WidgetKind},
    host_settings::write_extra_sentences,
    inherited::{Inherited, InheritedSource},
    note_writer::{NoteText, NoteView},
    notes::{NoteLook, write_invalid_value},
    page::PageActions,
    rows::{RowAreas, draw_note, field_id, show_dropdown},
};
use crate::{
    config::compare_tags,
    language::{LanguageOption, Selection, tag_match},
    texts::{ENGLISH_TAG, english, host_settings, note},
};

impl TextValue for InheritedSource {}

/// The state of a value of the language setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LanguageState {
    /// The value needs no language file, or its file has a valid header.
    Available,
    /// The language file of the value does not read or its header fails.
    Broken,
    /// No language file has or covers the tag of the value.
    Missing,
}

/// The kind of the note of the language row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LanguageNote {
    /// The value of the file is no language tag.
    Invalid,
    /// No language file has or covers the chosen tag.
    Missing,
    /// The language file of the chosen tag does not load.
    Broken,
    /// The file sets a language that loads. The note shows the inherited value.
    Set,
    /// The file does not set the key.
    Unset,
}

impl TextValue for LanguageNote {}

/// Returns the caption of the language value `value` among `options`: the text of `auto` in
/// `catalog`, the name of English, the name of a language file, or the value itself for a file
/// without a name and a tag without a file.
fn language_caption<'a>(
    options: &'a [LanguageOption],
    catalog: &'a Catalog,
    value: &'a str,
) -> &'a str {
    if value.eq_ignore_ascii_case(AUTO_LANGUAGE) {
        return catalog.text(host_settings::LANGUAGE_AUTO);
    }
    if value.eq_ignore_ascii_case(ENGLISH_TAG) {
        return english().name();
    }
    options
        .iter()
        .find(|option| option.file.tag.eq_ignore_ascii_case(value))
        .and_then(|option| option.name.as_deref())
        .unwrap_or(value)
}

/// The languages of the language files and the texts of the language row.
#[derive(Debug, Clone)]
pub(crate) struct LanguageRow {
    /// The languages of the language files in the order of the match.
    options: Vec<LanguageOption>,
    /// The dropdown values of the last fill of the captions.
    captioned: Vec<Cow<'static, str>>,
    /// The catalog generation of the last fill of the captions. `None` before the first fill.
    captions_generation: Option<u64>,
    /// The captions of the dropdown values, in the order of the values.
    captions: Vec<String>,
    /// The inherited entry of the dropdown for the source and the value of the inherited value.
    inherited_label: TextBuffer<(InheritedSource, String)>,
    /// The note of the row for its kind, the inherited source and the inherited value or the
    /// invalid value.
    note: NoteText<(LanguageNote, InheritedSource, String)>,
}

impl LanguageRow {
    /// The row of the languages `options`.
    pub(super) fn new(options: Vec<LanguageOption>) -> Self {
        Self {
            options,
            captioned: Vec::new(),
            captions_generation: None,
            captions: Vec::new(),
            inherited_label: TextBuffer::new(),
            note: NoteText::default(),
        }
    }

    /// The values of the dropdown: `auto`, `en`, the tags of the language files sorted by tag
    /// and the tag `saved` of the file when it is a language tag that no language file has,
    /// compared without regard to case.
    pub(super) fn choices(&self, saved: Option<&str>) -> Vec<Cow<'static, str>> {
        let mut tags: Vec<&str> = self
            .options
            .iter()
            .map(|option| option.file.tag.as_str())
            .collect();
        tags.sort_by(|first, second| compare_tags(first, second));
        let mut choices: Vec<Cow<'static, str>> = BUILT_IN_LANGUAGE_CHOICES
            .into_iter()
            .map(Cow::Borrowed)
            .chain(tags.into_iter().map(|tag| Cow::Owned(tag.to_owned())))
            .collect();
        if let Some(saved) = saved.filter(|saved| is_language_tag(saved))
            && !choices
                .iter()
                .any(|choice| choice.eq_ignore_ascii_case(saved))
        {
            choices.push(Cow::Owned(saved.to_owned()));
        }
        choices
    }

    /// The state of the language value `value`, compared without regard to case. A tag takes
    /// the language of the match of the user interface: its own file or the first file that
    /// covers it.
    pub(super) fn state(&self, value: &str) -> LanguageState {
        if value.eq_ignore_ascii_case(AUTO_LANGUAGE) {
            return LanguageState::Available;
        }
        match tag_match(&self.options, value) {
            Some(Selection::English) => LanguageState::Available,
            Some(Selection::File(option)) if option.name.is_some() => LanguageState::Available,
            Some(Selection::File(_)) => LanguageState::Broken,
            None => LanguageState::Missing,
        }
    }

    /// The caption of the language value `value` in the texts of `catalog`.
    #[cfg(test)]
    pub(super) fn caption<'a>(&'a self, catalog: &'a Catalog, value: &'a str) -> &'a str {
        language_caption(&self.options, catalog, value)
    }

    /// The captions of the dropdown values `choices` and the inherited entry of `inherited` in
    /// the texts of `catalog` at the catalog generation `generation`.
    ///
    /// The captions refill when the generation or the values change. The entry refills when the
    /// generation or the inherited value changes.
    pub(super) fn dropdown_texts(
        &mut self,
        catalog: &Catalog,
        generation: u64,
        choices: &[Cow<'static, str>],
        inherited: Option<&Inherited>,
    ) -> (&[String], &str) {
        let Self {
            options,
            captioned,
            captions_generation,
            captions,
            inherited_label,
            ..
        } = self;
        if *captions_generation != Some(generation) || captioned.as_slice() != choices {
            captioned.clear();
            captioned.extend_from_slice(choices);
            captions.clear();
            captions.extend(
                choices
                    .iter()
                    .map(|value| language_caption(options, catalog, value).to_owned()),
            );
            *captions_generation = Some(generation);
        }
        let source = inherited.map_or(InheritedSource::BuiltIn, |inherited| inherited.source);
        let value = inherited
            .and_then(|inherited| inherited.values.first())
            .map_or("", String::as_str);
        let label = inherited_label.text_in(
            catalog,
            generation,
            (source, value),
            |catalog, (source, value), out| {
                if value.is_empty() {
                    out.push_str(catalog.text(note::NOT_SET));
                } else {
                    let caption = language_caption(options, catalog, value);
                    let _ = catalog.write(out, source.label(&caption));
                }
            },
        );
        (captions, label)
    }

    /// The note of the language field `field` with the inherited value `inherited` and the
    /// command line `flags` in the texts of `catalog` at the catalog generation `generation`,
    /// and its look.
    ///
    /// An invalid value, a tag without a file and a file that does not load give a warning. A
    /// set field shows the inherited value. The sentences of the command line flag follow every
    /// note but the warning of an invalid value. The text refills when the generation or the
    /// data of the note changes.
    pub(super) fn note(
        &mut self,
        catalog: &Catalog,
        generation: u64,
        field: &Field,
        inherited: Option<&Inherited>,
        flags: &[(&'static str, String)],
    ) -> NoteView<'_> {
        let invalid = field.invalid_value();
        let kind = match (invalid, field.chosen()) {
            (Some(_), _) => LanguageNote::Invalid,
            (None, Some(value)) => match self.state(value) {
                LanguageState::Missing => LanguageNote::Missing,
                LanguageState::Broken => LanguageNote::Broken,
                LanguageState::Available if field.is_set() => LanguageNote::Set,
                LanguageState::Available => LanguageNote::Unset,
            },
            (None, None) if field.is_set() => LanguageNote::Set,
            (None, None) => LanguageNote::Unset,
        };
        let source = inherited.map_or(InheritedSource::BuiltIn, |inherited| inherited.source);
        let inherited_value = inherited
            .and_then(|inherited| inherited.values.first())
            .map_or("", String::as_str);
        let detail = match kind {
            LanguageNote::Invalid => invalid.map_or("", |invalid| invalid.value.as_str()),
            LanguageNote::Set => inherited_value,
            LanguageNote::Missing | LanguageNote::Broken | LanguageNote::Unset => "",
        };
        let Self { options, note, .. } = self;
        note.note(
            catalog,
            generation,
            (kind, source, detail),
            |writer, (kind, source, detail)| {
                let extra = |writer: &mut _| write_extra_sentences(writer, flags, GUI_LANGUAGE_KEY);
                match kind {
                    LanguageNote::Invalid => match invalid {
                        Some(invalid) => write_invalid_value(writer, invalid),
                        None => NoteLook::WARNING,
                    },
                    LanguageNote::Missing => {
                        writer.text(host_settings::MISSING_LANGUAGE);
                        extra(writer);
                        NoteLook::WARNING
                    }
                    LanguageNote::Broken => {
                        writer.text(host_settings::BROKEN_LANGUAGE);
                        extra(writer);
                        NoteLook::WARNING
                    }
                    LanguageNote::Set => {
                        if !detail.is_empty() {
                            let catalog = writer.catalog();
                            let caption = language_caption(options, catalog, detail);
                            let label = catalog.display(source.label(&caption));
                            writer.message(note::sentence(&label));
                        }
                        extra(writer);
                        NoteLook::PLAIN
                    }
                    LanguageNote::Unset => {
                        extra(writer);
                        NoteLook::PLAIN
                    }
                }
            },
        )
    }
}

impl Editor {
    /// Draws the dropdown of the language field at `index` of the settings page with the names
    /// of the languages. Returns the height of its note.
    pub(super) fn draw_language(
        &mut self,
        context: &mut UiContext,
        areas: RowAreas,
        index: usize,
        setting: &'static Setting,
        actions: &mut PageActions,
    ) -> f32 {
        let id = field_id(setting.key);
        self.widgets
            .push(EditorWidget::new(id, WidgetKind::Dropdown));
        let Subject::Host(settings) = &mut self.subject else {
            return areas.note.height;
        };
        let row = &mut settings.language;
        let catalog = context.catalog;
        let generation = context.catalog_generation;
        let inherited = self.inherited[index].as_ref();
        let field = &mut self.fields[index];
        let mut selected = field.selected();
        let before = selected;
        let (captions, label) = row.dropdown_texts(catalog, generation, &field.choices, inherited);
        if show_dropdown(
            context,
            areas.control,
            Dropdown::new(id, captions),
            Some(label),
            &mut selected,
        ) && selected != before
        {
            field.select(selected);
            actions.changed = true;
        }
        let note = row.note(catalog, generation, field, inherited, &settings.flags);
        draw_note(context, areas.note, note)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use machine_factory::{
        config::settings::{GUI_LANGUAGE_KEY, find_setting},
        rom_status::RomStatus,
    };

    use super::*;
    use crate::{
        config::{LANGUAGE_FOLDER, LanguageFile},
        editor::{
            EditorSection,
            form::SETTINGS_SECTIONS,
            notes::NoteKind,
            test_support::{SettingsRoot, index_of, settings_page, show, statuses_with},
        },
        gui_test_support::GuiHarness,
        texts::{test_catalog, test_language},
    };

    /// The sections of a German language file with its own text of `auto`.
    const GERMAN: &str = "[host-settings]\nlanguage-auto = Systemsprache\n";

    /// A root with the settings file `settings` and the language files of Japanese, German and
    /// a file without a header.
    fn language_root(name: &str, settings: &str) -> SettingsRoot {
        let test = SettingsRoot::new(name, Some(settings));
        let languages = test.root().join(LANGUAGE_FOLDER);
        std::fs::create_dir(&languages).expect("folder is created");
        for (file, contents) in [
            ("ja.ini", "[language]\nname = 日本語\nplural = none\n"),
            ("de.ini", test_language(GERMAN).as_str()),
            ("broken.ini", "[common]\nsave = Save\n"),
        ] {
            std::fs::write(languages.join(file), contents).expect("file is written");
        }
        test
    }

    /// A language option of `tag` with the language name `name` and no further tags.
    fn option(tag: &str, name: Option<&str>) -> LanguageOption {
        LanguageOption {
            file: LanguageFile {
                tag: tag.to_owned(),
                path: PathBuf::from(format!("/languages/{tag}.ini")),
            },
            name: name.map(str::to_owned),
            covers: Vec::new(),
        }
    }

    /// The language row of the settings page of `editor`.
    fn row(editor: &Editor) -> &LanguageRow {
        &editor.host_settings().expect("a settings page").language
    }

    /// The note of the language row of `editor` in `catalog` at `generation` and its look.
    fn note_of(editor: &mut Editor, catalog: &Catalog, generation: u64) -> (String, NoteKind) {
        let index = index_of(editor, GUI_LANGUAGE_KEY);
        let Subject::Host(settings) = &mut editor.subject else {
            panic!("a settings page");
        };
        let note = settings.language.note(
            catalog,
            generation,
            &editor.fields[index],
            editor.inherited[index].as_ref(),
            &settings.flags,
        );
        (note.text.to_owned(), note.look.kind)
    }

    /// The captions and the inherited entry of the language row of `editor` in `catalog` at
    /// `generation`.
    fn dropdown_of(
        editor: &mut Editor,
        catalog: &Catalog,
        generation: u64,
    ) -> (Vec<String>, String) {
        let index = index_of(editor, GUI_LANGUAGE_KEY);
        let Subject::Host(settings) = &mut editor.subject else {
            panic!("a settings page");
        };
        let (captions, label) = settings.language.dropdown_texts(
            catalog,
            generation,
            &editor.fields[index].choices,
            editor.inherited[index].as_ref(),
        );
        (captions.to_vec(), label.to_owned())
    }

    /// Verifies that the dropdown lists auto, English and the language files by tag with their names, shows the inherited value by its caption, warns about a tag without a file and a file that does not load, and saves another language.
    #[test]
    fn the_language_row_lists_the_language_files() {
        let setting = find_setting(GUI_LANGUAGE_KEY).expect("a key");
        let test = language_root("languages", "gui-language = fr\n");
        let host = test.host(&[]);
        let mut editor = settings_page(&host, &statuses_with(&host, RomStatus::Found));
        let index = index_of(&editor, GUI_LANGUAGE_KEY);
        assert_eq!(
            editor.fields[index].choices,
            ["auto", "en", "broken", "de", "ja", "fr"]
        );
        assert_eq!(editor.fields[index].chosen(), Some("fr"));
        assert_eq!(editor.extra_note(setting), None);
        let language = row(&editor);
        assert_eq!(language.state("AUTO"), LanguageState::Available);
        assert_eq!(language.state("en"), LanguageState::Available);
        assert_eq!(language.state("DE"), LanguageState::Available);
        assert_eq!(language.state("broken"), LanguageState::Broken);
        assert_eq!(language.state("fr"), LanguageState::Missing);
        assert_eq!(language.caption(english(), "broken"), "broken");
        assert_eq!(
            dropdown_of(&mut editor, english(), 0),
            (
                [
                    "System language",
                    "English",
                    "broken",
                    "Deutsch",
                    "日本語",
                    "fr"
                ]
                .map(str::to_owned)
                .to_vec(),
                "Built-in default: System language".to_owned()
            )
        );
        assert_eq!(
            note_of(&mut editor, english(), 0),
            (
                "The language file does not exist. The user interface takes the file of a \
                 shorter form of the tag or English."
                    .to_owned(),
                NoteKind::Warning
            )
        );
        editor.fields[index].select(Some(2));
        assert_eq!(
            note_of(&mut editor, english(), 0),
            (
                "The language file does not load. The user interface shows English.".to_owned(),
                NoteKind::Warning
            )
        );
        editor.fields[index].select(Some(3));
        assert_eq!(
            note_of(&mut editor, english(), 0),
            (
                "Built-in default: System language.".to_owned(),
                NoteKind::Plain
            )
        );

        editor.section = SETTINGS_SECTIONS
            .iter()
            .position(|section| *section == EditorSection::VideoAndAudio)
            .expect("the video and audio section");
        let mut harness = GuiHarness::minimum();
        show(&mut harness, &mut editor);
        editor.save(false).expect("the save writes");
        assert_eq!(
            std::fs::read_to_string(test.settings_file()).expect("file reads"),
            "gui-language = de\n"
        );
    }

    /// Verifies that a tag that a language file covers is available without the note of a missing
    /// file, and that the choices list the covering file once.
    #[test]
    fn a_covered_tag_takes_the_covering_file() {
        let test = SettingsRoot::new("covered_language", Some("gui-language = zh-HK\n"));
        let languages = test.root().join(LANGUAGE_FOLDER);
        std::fs::create_dir(&languages).expect("folder is created");
        std::fs::write(
            languages.join("zh-TW.ini"),
            "[language]\nname = 繁體中文（台灣）\nplural = none\ncovers = zh-HK zh-MO zh-Hant\n",
        )
        .expect("file is written");
        let host = test.host(&[]);
        let mut editor = settings_page(&host, &statuses_with(&host, RomStatus::Found));
        let index = index_of(&editor, GUI_LANGUAGE_KEY);
        assert_eq!(
            editor.fields[index].choices,
            ["auto", "en", "zh-TW", "zh-HK"]
        );
        assert_eq!(editor.fields[index].chosen(), Some("zh-HK"));
        let language = row(&editor);
        assert_eq!(language.state("zh-HK"), LanguageState::Available);
        assert_eq!(language.state("ZH-hant"), LanguageState::Available);
        assert_eq!(language.state("zh-CN"), LanguageState::Missing);
        assert_eq!(
            note_of(&mut editor, english(), 0),
            (
                "Built-in default: System language.".to_owned(),
                NoteKind::Plain
            )
        );
    }

    /// Verifies that an invalid value warns without a choice of its own and that the flag sentence follows the note of an unset field.
    #[test]
    fn an_invalid_value_warns_and_a_flag_follows_the_note() {
        let test = language_root("invalid_language", "gui-language = pt_BR\n");
        let host = test.host(&[]);
        let mut editor = settings_page(&host, &statuses_with(&host, RomStatus::Found));
        let index = index_of(&editor, GUI_LANGUAGE_KEY);
        assert_eq!(
            editor.fields[index].choices,
            ["auto", "en", "broken", "de", "ja"]
        );
        let (text, kind) = note_of(&mut editor, english(), 0);
        assert!(text.starts_with("Invalid GUI language 'pt_BR'"), "{text}");
        assert!(text.ends_with("The launch ignores the line."), "{text}");
        assert_eq!(kind, NoteKind::Warning);

        let test = language_root("flag_language", "");
        let host = test.host(&[(GUI_LANGUAGE_KEY, "ja")]);
        let mut editor = settings_page(&host, &statuses_with(&host, RomStatus::Found));
        assert_eq!(
            note_of(&mut editor, english(), 0),
            (
                "The command line sets ja for this run.".to_owned(),
                NoteKind::Plain
            )
        );
    }

    /// Verifies that the choices leave out an invalid saved value and a saved tag that differs from a known tag only in case.
    #[test]
    fn the_choices_add_only_a_new_valid_saved_tag() {
        let language = LanguageRow::new(vec![option("ja", Some("日本語"))]);
        let built_in = ["auto", "en", "ja"];
        assert_eq!(language.choices(None), built_in);
        assert_eq!(language.choices(Some("pt_BR")), built_in);
        assert_eq!(language.choices(Some("JA")), built_in);
        assert_eq!(language.choices(Some("EN")), built_in);
        assert_eq!(language.choices(Some("AUTO")), built_in);
        assert_eq!(
            language.choices(Some("pt-BR")),
            ["auto", "en", "ja", "pt-BR"]
        );
    }

    /// Verifies that the captions and the inherited entry keep their texts for the same generation and values, and refill on a new generation or new values.
    #[test]
    fn the_dropdown_texts_refill_on_a_generation_or_a_choice_change() {
        let mut language = LanguageRow::new(vec![option("de", Some("Deutsch"))]);
        let german = test_catalog(GERMAN);
        let choices = language.choices(None);
        let inherited = Inherited::new(vec!["auto".to_owned()], InheritedSource::BuiltIn);
        let texts = |language: &mut LanguageRow, catalog, generation, choices: &[_]| {
            let (captions, label) =
                language.dropdown_texts(catalog, generation, choices, Some(&inherited));
            (captions.as_ptr(), captions.to_vec(), label.to_owned())
        };
        let (pointer, captions, label) = texts(&mut language, english(), 0, &choices);
        assert_eq!(captions, ["System language", "English", "Deutsch"]);
        assert_eq!(label, "Built-in default: System language");
        let (kept, captions, label) = texts(&mut language, &german, 0, &choices);
        assert_eq!(kept, pointer);
        assert_eq!(captions, ["System language", "English", "Deutsch"]);
        assert_eq!(label, "Built-in default: System language");
        let (_, captions, label) = texts(&mut language, &german, 1, &choices);
        assert_eq!(captions, ["Systemsprache", "English", "Deutsch"]);
        assert_eq!(label, "Built-in default: Systemsprache");
        let more = language.choices(Some("fr"));
        let (_, captions, _) = texts(&mut language, &german, 1, &more);
        assert_eq!(captions, ["Systemsprache", "English", "Deutsch", "fr"]);
    }
}
