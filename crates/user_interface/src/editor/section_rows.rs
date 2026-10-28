//! The rows of the selected section of the editor: the read-only rows, the
//! section note, the field rows and the metadata rows. A press on the label
//! or on the empty space of a field row gives the main control of the row
//! the focus.

use std::fmt::Write as _;

use gui::{
    Rect,
    ui::{ActionButton, Icon, NumberField, TextField, Truncation, UiContext, UiId, WrappedText},
};
use i18n::Key;
use machine_factory::config::settings::ValueKind;

use super::{
    Editor, EditorFile, Subject,
    fields::is_single_image,
    form::{EditorSection, FieldSource},
    help_area::FormRow,
    hints::{EditorWidget, WidgetKind},
    host_settings::draw_settings_file_block,
    kept_lines::draw_kept_lines,
    note_writer::NoteView,
    notes::{NoteKind, NoteLook},
    page::{PageActions, TABS_ID},
    rows::{RowAreas, RowTexts, SettingRow, draw_label, draw_note, draw_note_text, field_id},
};
use crate::{
    config::metadata::{FIRST_YEAR, LAST_YEAR, TITLE_KEY, YEAR_KEY},
    texts::{editor, host_settings, note, setting_label},
};

/// A row that shows a value the user cannot type, such as the system of a new game.
struct ReadOnlyRow<'a> {
    /// The name of the id of the row.
    id: &'static str,
    /// The key of the label.
    name: Key,
    /// The value.
    value: &'a mut String,
    /// Whether the value is a path, which takes the folder icon.
    path: bool,
    /// The note after the value.
    note: &'a str,
    /// The wrapped lines of the note.
    lines: &'a mut WrappedText,
}

/// Draws the plain `note` as a line of a section from `y` in `content`,
/// inset from the labels of the rows. Returns the height of the line.
fn draw_section_note(context: &mut UiContext, content: Rect, y: f32, note: NoteView) -> f32 {
    let theme = context.theme;
    let area = Rect::new(
        content.x + theme.gap * 1.5,
        y + theme.gap * 0.5,
        (content.width - theme.gap * 1.5).max(0.0),
        theme.control_height,
    );
    let height = draw_note_text(context, area, note.text, NoteKind::Plain, false, note.lines);
    RowAreas::height(theme, height)
}

impl Editor {
    /// Gives `id` the focus when the first widget of the section takes it.
    fn claim_first_focus(&mut self, context: &mut UiContext, id: UiId) {
        if std::mem::take(&mut self.focus_first) {
            context.set_focus(id);
        }
    }

    /// Draws the rows of the selected section into `content` and returns
    /// their height.
    pub(super) fn draw_section(
        &mut self,
        context: &mut UiContext,
        content: Rect,
        actions: &mut PageActions,
    ) -> f32 {
        let catalog = context.catalog;
        let generation = context.catalog_generation;
        let revision = self.revision;
        let section = self.current_section();
        let mut y = content.y;
        if section == EditorSection::Metadata && self.file.derived().is_some() {
            let mut texts = std::mem::take(&mut self.texts);
            texts.system_value.clear();
            if let Some(files) = self.system_files() {
                texts.system_value.push_str(files.system.label);
            }
            texts.file_value.clone_from(&self.shown);
            let file_note =
                texts
                    .file_note
                    .text_in(catalog, generation, revision, |catalog, _, out| {
                        if let Some(file) = self.file.derived() {
                            file.note.write(catalog, out);
                        }
                    });
            y += Self::draw_read_only(
                context,
                content,
                y,
                ReadOnlyRow {
                    id: "system",
                    name: editor::SYSTEM_LABEL,
                    value: &mut texts.system_value,
                    path: false,
                    note: catalog.text(editor::SYSTEM_NOTE),
                    lines: &mut texts.system_lines,
                },
            );
            y += Self::draw_read_only(
                context,
                content,
                y,
                ReadOnlyRow {
                    id: "file",
                    name: editor::FILE_LABEL,
                    value: &mut texts.file_value,
                    path: true,
                    note: file_note,
                    lines: &mut texts.file_lines,
                },
            );
            self.texts = texts;
        }
        if section == EditorSection::Folders
            && let Some(note) = self.root_note()
        {
            let mut value = std::mem::take(&mut self.texts.root_value);
            value.clear();
            let _ = write!(value, "{}", self.root.display());
            y += Self::draw_read_only(
                context,
                content,
                y,
                ReadOnlyRow {
                    id: "root",
                    name: host_settings::ROOT_LABEL,
                    value: &mut value,
                    path: true,
                    note: catalog.text(note),
                    lines: &mut self.texts.root_lines,
                },
            );
            self.texts.root_value = value;
        }
        let empty = !self.fields.iter().any(|field| field.section == section);
        let section_note = if empty
            && section != EditorSection::Metadata
            && let Some(files) = self.system_files()
        {
            section.empty_text(&files.system.label).is_some()
        } else {
            section == EditorSection::Roms
                && self
                    .host_settings()
                    .and_then(|settings| settings.roms_folder_flag())
                    .is_some()
        };
        if section_note {
            let mut texts = std::mem::take(&mut self.texts);
            let note = texts.section_note.note(
                catalog,
                generation,
                (revision, self.section),
                |writer, _| {
                    if let Some(files) = self.system_files()
                        && let Some(message) = section.empty_text(&files.system.label)
                    {
                        writer.message(message);
                    }
                    if let Some(folder) = self
                        .host_settings()
                        .and_then(|settings| settings.roms_folder_flag())
                    {
                        writer.message(host_settings::roms_folder_flag(&folder));
                    }
                    NoteLook::PLAIN
                },
            );
            y += draw_section_note(context, content, y, note);
            self.texts = texts;
        }
        for index in 0..self.fields.len() {
            if self.fields[index].section == section {
                y += self.draw_field(context, content, y, index, actions);
            }
        }
        if section == EditorSection::Advanced && !self.kept.is_empty() {
            y += draw_kept_lines(context, content, y, &self.kept);
        }
        if section == EditorSection::Folders && self.host_settings().is_some() {
            y += draw_settings_file_block(context, content, y, &mut self.texts.settings_file_lines);
        }
        if std::mem::take(&mut self.focus_first) {
            context.set_focus(UiId::from(TABS_ID));
        }
        y - content.y
    }

    /// Draws `row` as a read-only row from `y` in `content`. Returns the
    /// height of the row.
    fn draw_read_only(context: &mut UiContext, content: Rect, y: f32, row: ReadOnlyRow) -> f32 {
        let theme = context.theme;
        let catalog = context.catalog;
        let areas = RowAreas::new(theme, content, y);
        draw_label(
            context,
            areas.label,
            catalog.text(row.name),
            "",
            false,
            true,
        );
        TextField::new(UiId::from("read only").child(row.id), "")
            .icon(row.path.then_some(Icon::Folder))
            .clears_on_cancel(false)
            .read_only(true)
            .truncation(Truncation::Middle)
            .show(context, areas.control, row.value);
        let note = draw_note_text(
            context,
            areas.note,
            row.note,
            NoteKind::Plain,
            false,
            row.lines,
        );
        RowAreas::height(theme, note)
    }

    /// Draws the row of the field at `index` from `y` in `content` and
    /// returns its height. The widgets of the row record the field. The row
    /// records its area and whether the pointer is over it, tested before
    /// its widgets take the pointer. A press on the label or on the empty
    /// space of the row gives the main control of the row the focus.
    fn draw_field(
        &mut self,
        context: &mut UiContext,
        content: Rect,
        y: f32,
        index: usize,
        actions: &mut PageActions,
    ) -> f32 {
        let mut texts = std::mem::take(&mut self.row_texts[index]);
        let first_widget = self.widgets.len();
        let takes_pointer = context.takes_pointer();
        let height = self.draw_field_with(context, content, y, index, &mut texts, actions);
        self.row_texts[index] = texts;
        for widget in &mut self.widgets[first_widget..] {
            widget.field = Some(index);
        }
        let rect = Rect::new(content.x, y, content.width, height);
        context.focus_on_press(field_id(self.fields[index].key()), rect);
        let (pointer_x, pointer_y) = context.mouse();
        self.rows.push(FormRow {
            field: index,
            rect,
            hovered: takes_pointer && rect.contains(pointer_x, pointer_y),
        });
        height
    }

    /// Draws the row of the field at `index` with its texts `texts` from
    /// `y` in `content` and returns its height.
    fn draw_field_with(
        &mut self,
        context: &mut UiContext,
        content: Rect,
        y: f32,
        index: usize,
        texts: &mut RowTexts,
        actions: &mut PageActions,
    ) -> f32 {
        let theme = context.theme;
        let catalog = context.catalog;
        let areas = RowAreas::new(theme, content, y);
        let source = self.fields[index].source;
        let id = field_id(source.key());
        let set = self.fields[index].is_set();
        let setting = match source {
            FieldSource::Metadata(metadata) => {
                draw_label(
                    context,
                    areas.label,
                    catalog.text(metadata.label),
                    metadata.key,
                    set,
                    true,
                );
                self.claim_first_focus(context, id);
                let note = self.draw_metadata(context, areas, index, texts, actions);
                return RowAreas::height(theme, note);
            }
            FieldSource::Setting(setting) => setting,
        };
        let ignored = self.ignored_by(setting);
        let enabled = ignored.is_none();
        draw_label(
            context,
            areas.label,
            setting_label(context.catalog, setting),
            setting.key,
            set,
            enabled,
        );
        if enabled {
            self.claim_first_focus(context, id);
        }
        let row = SettingRow {
            areas,
            index,
            setting,
            ignored,
            texts,
        };
        match setting.kind {
            ValueKind::Language if matches!(self.subject, Subject::Host(_)) => {
                let note = self.draw_language(context, areas, index, setting, actions);
                RowAreas::height(theme, note)
            }
            ValueKind::Choice(_) | ValueKind::OnOff | ValueKind::Language => {
                let note = self.draw_choice(context, row, actions);
                RowAreas::height(theme, note)
            }
            ValueKind::Number(range) => {
                let note = self.draw_number(context, row, range, actions);
                RowAreas::height(theme, note)
            }
            ValueKind::Path(_) if is_single_image(setting) => {
                self.draw_image(context, row, actions)
            }
            ValueKind::Path(_) => self.draw_path(context, row, actions),
            ValueKind::PathList => self.draw_media(context, row, actions),
        }
    }

    /// Clears the text field at `index` when gamepad X reaches the focused
    /// field `id` while the field is set. Returns whether it cleared.
    pub(super) fn clear_on_x(&mut self, context: &UiContext, id: UiId, index: usize) -> bool {
        let field = &mut self.fields[index];
        if field.is_set()
            && context.has_focus(id)
            && context.action_button() == Some(ActionButton::X)
        {
            field.unset();
            return true;
        }
        false
    }

    /// Draws the text or year field of the metadata field at `index` with
    /// the texts `texts`. Returns the height of its note.
    fn draw_metadata(
        &mut self,
        context: &mut UiContext,
        areas: RowAreas,
        index: usize,
        texts: &mut RowTexts,
        actions: &mut PageActions,
    ) -> f32 {
        let theme = context.theme;
        let catalog = context.catalog;
        let key = self.fields[index].key();
        let id = field_id(key);
        let set = self.fields[index].is_set();
        let kind = if key == YEAR_KEY {
            WidgetKind::Number
        } else {
            WidgetKind::Text
        };
        self.widgets.push(EditorWidget::new(id, kind).clears(set));
        let not_set = catalog.text(note::NOT_SET);
        if let Some(text) = self.fields[index].text_mut() {
            let response = if key == YEAR_KEY {
                let field = NumberField::new(id, not_set)
                    .range(f64::from(FIRST_YEAR), f64::from(LAST_YEAR));
                let rect = Rect::new(
                    areas.control.x,
                    areas.control.y,
                    field.width(context).min(areas.control.width),
                    theme.control_height,
                );
                field.show(context, rect, text)
            } else {
                TextField::new(id, not_set)
                    .icon(None)
                    .clears_on_cancel(false)
                    .show(context, areas.control, text)
            };
            actions.changed |= response.changed;
        }
        actions.changed |= self.clear_on_x(context, id, index);
        let note = texts.note.note(
            catalog,
            context.catalog_generation,
            (self.revision, false),
            |writer, _| {
                if let EditorFile::Existing(path) = &self.file
                    && key == TITLE_KEY
                    && let Some(stem) = path.file_stem()
                {
                    writer.message(editor::title_note(&stem.to_string_lossy()));
                }
                NoteLook::PLAIN
            },
        );
        draw_note(context, areas.note, note)
    }
}

#[cfg(test)]
mod tests {
    use gui::ui::Navigation;
    use machine_factory::{
        config::{
            Target,
            settings::{MODEL_CHOICES, Setting},
            systems::compiled_systems,
        },
        rom_status::RomStatus,
    };

    use super::*;
    use crate::{
        editor::{
            hints::WidgetKind,
            media_list::{buttons_width, create_id},
            page::form_area,
            rows::{FORM_ROW_PITCH, browse_id},
            test_support::{
                EditorRoot, SHORT_SCREEN, SettingsRoot, index_of, media_section, row_of,
                settings_page, show, show_in, shown_help, shown_help_in, statuses_with,
            },
        },
        gui_test_support::{GuiHarness, MINIMUM_SCREEN, MINIMUM_WIDTH, PassCounts},
    };

    /// The kinds of the rows that the row press tests met.
    #[derive(Debug, Default)]
    struct PressedRows {
        /// The value kinds of the enabled setting rows, by the name of the kind.
        kinds: Vec<&'static str>,
        /// The number of enabled metadata rows.
        metadata: usize,
        /// The number of disabled rows.
        disabled: usize,
    }

    /// The name of the value kind of `setting`. The single image of a drive is an image.
    fn kind_name(setting: &Setting) -> &'static str {
        if is_single_image(setting) {
            return "image";
        }
        match setting.kind {
            ValueKind::Choice(_) => "choice",
            ValueKind::OnOff => "on/off",
            ValueKind::Number(_) => "number",
            ValueKind::Path(_) => "path",
            ValueKind::PathList => "media list",
            ValueKind::Language => "language",
        }
    }

    /// Presses and releases the left button at `x`, `y` over `editor`.
    fn click(harness: &mut GuiHarness, editor: &mut Editor, x: f32, y: f32) {
        harness.input.mouse_x = x;
        harness.input.mouse_y = y;
        harness.input.left = true;
        show(harness, editor);
        harness.input.left = false;
        show(harness, editor);
    }

    /// The id of the widget that holds the focus after a Tab from the main control of the
    /// row of `field` in the last pass of `editor`: the Browse button of a path row, the Create
    /// button of a media list with blank images, the add button of another media list, or else
    /// the main control of the next enabled row.
    fn after_tab(editor: &Editor, field: usize) -> Option<UiId> {
        let source = editor.fields[field].source;
        let key = source.key();
        match source.setting().map(|setting| setting.kind) {
            Some(ValueKind::Path(_)) => return Some(browse_id(key)),
            Some(ValueKind::PathList) => {
                let create = create_id(field_id(key));
                let has_create = editor
                    .widgets
                    .iter()
                    .any(|widget| widget.id == create && widget.kind == WidgetKind::CreateImage);
                return Some(if has_create {
                    create
                } else {
                    field_id(key).child("add")
                });
            }
            Some(
                ValueKind::Choice(_)
                | ValueKind::OnOff
                | ValueKind::Number(_)
                | ValueKind::Language,
            )
            | None => {}
        }
        let position = editor.rows.iter().position(|row| row.field == field)?;
        editor.rows[position + 1..]
            .iter()
            .find(|row| {
                editor
                    .widgets
                    .iter()
                    .any(|widget| widget.field == Some(row.field))
            })
            .map(|row| field_id(editor.fields[row.field].key()))
    }

    /// Presses the label and the empty space of every row of every section of `editor`, with
    /// the row scrolled to the top of the form and the focus on the tab bar before each press.
    /// A press on an enabled row gives its main control the focus, shows the help of its
    /// setting, leaves a steady frame without allocations, and a Tab after it takes the widget
    /// after the control. A press on a disabled row keeps the focus on the tab bar. Adds the
    /// rows to `pressed`.
    fn press_every_row(harness: &mut GuiHarness, editor: &mut Editor, pressed: &mut PressedRows) {
        let tabs = UiId::from(TABS_ID);
        let form = form_area(&harness.theme, MINIMUM_SCREEN);
        let gap = harness.theme.gap;
        show(harness, editor);
        for section in 0..editor.sections().len() {
            if section > 0 {
                harness.state.set_focus(tabs);
                harness.input.navigation = Some(Navigation::NextTab);
                show(harness, editor);
            }
            let fields: Vec<usize> = editor.rows.iter().map(|row| row.field).collect();
            for field in fields {
                let key = editor.fields[field].key();
                let setting = editor.fields[field].source.setting();
                let enabled = editor
                    .widgets
                    .iter()
                    .any(|widget| widget.field == Some(field));
                let mut points = Vec::new();
                for point in 0..3 {
                    harness.input.mouse_x = 1.0;
                    harness.input.mouse_y = 1.0;
                    harness.state.set_focus(tabs);
                    editor.scroll.offset += row_of(editor, key).rect.y - form.y - gap;
                    show(harness, editor);
                    let row = row_of(editor, key);
                    let label = RowAreas::new(&harness.theme, row.rect, row.rect.y).label;
                    let (x, y) = match point {
                        0 => (label.x + 4.0, label.y + label.height * 0.5),
                        1 => (
                            label.right() + harness.theme.padding * 0.5,
                            row.rect.y + 1.0,
                        ),
                        _ if row.rect.height > FORM_ROW_PITCH => {
                            (label.x + 4.0, row.rect.bottom().min(form.bottom()) - 2.0)
                        }
                        _ => continue,
                    };
                    assert!(y > form.y && y < form.bottom(), "{key} {y} {form:?}");
                    points.push((x, y));
                    click(harness, editor, x, y);
                    if !enabled {
                        assert_eq!(harness.state.focused(), Some(tabs), "{key} {x} {y}");
                        continue;
                    }
                    assert_eq!(
                        harness.state.focused(),
                        Some(field_id(key)),
                        "{key} {x} {y}"
                    );
                    let (counts, _) = harness.counted_frame(|context| {
                        editor.show(context, MINIMUM_SCREEN);
                    });
                    assert_eq!(counts, PassCounts::NONE, "{key}");
                    assert_eq!(
                        shown_help(harness, editor),
                        setting.map(|setting| setting.key),
                        "{key}"
                    );
                    let next = after_tab(editor, field);
                    harness.input.navigation = Some(Navigation::Next);
                    show(harness, editor);
                    if let Some(next) = next {
                        assert_eq!(harness.state.focused(), Some(next), "{key}");
                        let focused_setting = editor
                            .widgets
                            .iter()
                            .find(|widget| widget.id == next)
                            .and_then(|widget| widget.field)
                            .and_then(|field| editor.fields[field].source.setting());
                        assert_eq!(
                            shown_help(harness, editor),
                            focused_setting.map(|setting| setting.key),
                            "{key}"
                        );
                    }
                }
                assert!(points.len() >= 2, "{key} {points:?}");
                match setting {
                    _ if !enabled => pressed.disabled += 1,
                    Some(setting) => pressed.kinds.push(kind_name(setting)),
                    None => pressed.metadata += 1,
                }
            }
        }
    }

    /// The names of the value kinds of the settings of `editor` that the model `model` of the
    /// family `target` reads.
    fn read_kinds(editor: &Editor, target: Target, model: &str) -> Vec<&'static str> {
        editor
            .fields
            .iter()
            .filter_map(|field| field.source.setting())
            .filter(|setting| setting.is_read_by(target, model))
            .map(kind_name)
            .collect()
    }

    /// Verifies that a press on the label or on the empty space of a row gives the main control of
    /// the row the focus in the game editor of every model of every compiled system and in the
    /// system defaults editors: the dropdown of a choice and of an on/off value, the number field,
    /// the path field, the path field of an image row, the media list and the field of a metadata
    /// row. The help area shows the pressed row, a steady frame after the press allocates nothing,
    /// a Tab after the press takes the widget after the control. The presses meet every kind of the
    /// settings that a model of the build reads, and the build with every system meets every kind
    /// of the game editor.
    #[test]
    fn a_press_on_a_row_focuses_its_control_in_the_editors() {
        let mut harness = GuiHarness::minimum();
        let mut pressed = PressedRows::default();
        let mut read = Vec::new();
        for system in compiled_systems() {
            let root = EditorRoot::with_system("row_press", system.name);
            for choice in MODEL_CHOICES
                .iter()
                .filter(|choice| choice.families.contains_target(root.target()))
            {
                let path = root.game(
                    "edge",
                    &format!("meta.title = Edge\nmachine = {}\n", choice.value),
                );
                let mut editor =
                    Editor::open(&root.host(), root.system(), &path).expect("the editor opens");
                read.extend(read_kinds(&editor, root.target(), choice.value));
                press_every_row(&mut harness, &mut editor, &mut pressed);
            }
            let mut editor =
                Editor::system_defaults(&root.host(), root.system()).expect("the editor opens");
            press_every_row(&mut harness, &mut editor, &mut pressed);
        }
        for kind in read {
            assert!(pressed.kinds.contains(&kind), "{kind} {pressed:?}");
        }
        if cfg!(feature = "all-machines") {
            for kind in ["choice", "on/off", "number", "path", "image", "media list"] {
                assert!(pressed.kinds.contains(&kind), "{kind} {pressed:?}");
            }
        }
        assert!(pressed.metadata > 0, "{pressed:?}");
    }

    /// Verifies that a press on the label or on the empty space of a row that the model ignores
    /// keeps the focus, in the game editors of every model of every compiled system. The
    /// systems of the gate have a model that ignores a row inside the form.
    #[test]
    #[cfg(any(
        feature = "pc98",
        feature = "pc60",
        feature = "msx",
        feature = "towns",
        feature = "x1",
        feature = "x68k"
    ))]
    fn a_press_on_a_disabled_row_keeps_the_focus() {
        let mut harness = GuiHarness::minimum();
        let mut pressed = PressedRows::default();
        for system in compiled_systems() {
            let root = EditorRoot::with_system("row_press_disabled", system.name);
            for choice in MODEL_CHOICES
                .iter()
                .filter(|choice| choice.families.contains_target(root.target()))
            {
                let path = root.game(
                    "edge",
                    &format!("meta.title = Edge\nmachine = {}\n", choice.value),
                );
                let mut editor =
                    Editor::open(&root.host(), root.system(), &path).expect("the editor opens");
                press_every_row(&mut harness, &mut editor, &mut pressed);
            }
        }
        assert!(pressed.disabled > 0, "{pressed:?}");
    }

    /// Verifies that a press on the label or on the empty space of a row of the settings page
    /// gives the main control of the row the focus: the language dropdown, the dropdowns, the
    /// path fields and the number fields. A press on the read-only row of the root keeps the
    /// focus.
    #[test]
    fn a_press_on_a_row_focuses_its_control_on_the_settings_page() {
        let test = SettingsRoot::new("row_press_settings", Some(""));
        let host = test.host(&[]);
        let mut editor = settings_page(&host, &statuses_with(&host, RomStatus::NotFound));
        let mut harness = GuiHarness::minimum();
        let mut pressed = PressedRows::default();
        press_every_row(&mut harness, &mut editor, &mut pressed);
        for kind in ["language", "choice", "number", "path"] {
            assert!(pressed.kinds.contains(&kind), "{kind} {pressed:?}");
        }
        let tabs = UiId::from(TABS_ID);
        while editor.current_section() != EditorSection::Folders {
            harness.input.navigation = Some(Navigation::NextTab);
            show(&mut harness, &mut editor);
        }
        let form = form_area(&harness.theme, MINIMUM_SCREEN);
        let root_row = RowAreas::new(&harness.theme, form, form.y);
        for (x, y) in [
            (
                root_row.label.x + 4.0,
                root_row.label.y + root_row.label.height * 0.5,
            ),
            (
                root_row.label.right() + harness.theme.padding * 0.5,
                form.y + 1.0,
            ),
        ] {
            harness.state.set_focus(tabs);
            show(&mut harness, &mut editor);
            click(&mut harness, &mut editor, x, y);
            assert_eq!(harness.state.focused(), Some(tabs), "{x} {y}");
        }
    }

    /// Verifies that a press on the label of the read-only system row of a new game keeps the
    /// focus.
    #[test]
    fn a_press_on_a_read_only_row_keeps_the_focus() {
        let root = EditorRoot::new("row_press_read_only");
        let mut editor = Editor::new_game(&root.host(), root.system()).expect("the editor opens");
        let mut harness = GuiHarness::minimum();
        show(&mut harness, &mut editor);
        assert_eq!(editor.current_section(), EditorSection::Metadata);
        let tabs = UiId::from(TABS_ID);
        let form = form_area(&harness.theme, MINIMUM_SCREEN);
        let system_row = RowAreas::new(&harness.theme, form, form.y);
        assert!(editor.rows.iter().all(|row| row.rect.y > form.y));
        harness.state.set_focus(tabs);
        show(&mut harness, &mut editor);
        click(
            &mut harness,
            &mut editor,
            system_row.label.x + 4.0,
            system_row.label.y + system_row.label.height * 0.5,
        );
        assert_eq!(harness.state.focused(), Some(tabs));
    }

    /// Verifies that a press on the visible lower part of a tall media row whose label and
    /// list lie above the view gives the list the focus, that the list shows inside the form in
    /// the next frame, that a steady frame after the press allocates nothing, and that the
    /// help area shows the pressed row until the pointer enters another row. The press on
    /// `fdd2` near the top of the form leaves the pointer over the row of `fdd1` after the
    /// form scrolls. Covers every compiled system, and the pc98 reads `fdd2`.
    #[test]
    fn a_press_below_a_hidden_control_reveals_the_control() {
        /// The height of the part of the row that shows below the top of the form.
        const SHOWN: f32 = 24.0;
        let mut harness = GuiHarness::new(MINIMUM_WIDTH as u32, 300);
        let form = form_area(&harness.theme, SHORT_SCREEN);
        let tabs = UiId::from(TABS_ID);
        let cases: [(&str, &str, &[f32]); 2] = [
            (
                "fdd1",
                "fdd1 = a.img\nfdd1 = b.img\nfdd1 = c.img\n",
                &[SHOWN * 0.5],
            ),
            (
                "fdd2",
                "fdd1 = a.img\nfdd2 = a.img\nfdd2 = b.img\nfdd2 = c.img\n",
                &[1.0, 3.0],
            ),
        ];
        let mut second_drive_presses = 0;
        for system in compiled_systems() {
            let name = system.name;
            let root = EditorRoot::with_system("row_press_hidden", name);
            for (key, lines, heights) in cases {
                for height in heights {
                    let mut editor = media_section(&root, &mut harness, SHORT_SCREEN, lines);
                    if !editor
                        .widgets
                        .iter()
                        .any(|widget| widget.id == field_id(key))
                    {
                        continue;
                    }
                    editor.scroll.offset += row_of(&editor, key).rect.bottom() - form.y - SHOWN;
                    harness.state.set_focus(tabs);
                    show_in(&mut harness, &mut editor, SHORT_SCREEN);
                    let row = row_of(&editor, key);
                    let areas = RowAreas::new(&harness.theme, row.rect, row.rect.y);
                    assert!(
                        areas.control.bottom() < form.y,
                        "{name} {key} {row:?} {form:?}"
                    );
                    assert!(
                        row.rect.bottom() > form.y + height,
                        "{name} {key} {row:?} {form:?}"
                    );
                    harness.input.mouse_x = areas.label.x + 4.0;
                    harness.input.mouse_y = form.y + height;
                    harness.input.left = true;
                    show_in(&mut harness, &mut editor, SHORT_SCREEN);
                    harness.input.left = false;
                    show_in(&mut harness, &mut editor, SHORT_SCREEN);
                    assert_eq!(
                        harness.state.focused(),
                        Some(field_id(key)),
                        "{name} {key} {height}"
                    );
                    show_in(&mut harness, &mut editor, SHORT_SCREEN);
                    let row = row_of(&editor, key);
                    let control = RowAreas::new(&harness.theme, row.rect, row.rect.y).control;
                    assert!(
                        control.y >= form.y - 0.5 && control.bottom() <= form.bottom(),
                        "{name} {key} {control:?} {form:?}"
                    );
                    let (counts, _) = harness.counted_frame(|context| {
                        editor.show(context, SHORT_SCREEN);
                    });
                    assert_eq!(counts, PassCounts::NONE, "{name} {key} {height}");
                    let hovered = |editor: &Editor| {
                        editor
                            .rows
                            .iter()
                            .find(|row| row.hovered)
                            .map(|row| editor.fields[row.field].key())
                    };
                    if key == "fdd2" {
                        assert_eq!(hovered(&editor), Some("fdd1"), "{name} {height}");
                        second_drive_presses += 1;
                    }
                    assert_eq!(
                        shown_help_in(&mut harness, &mut editor, SHORT_SCREEN),
                        Some(key),
                        "{name} {key} {height}"
                    );
                    harness.input.mouse_y -= 1.0;
                    show_in(&mut harness, &mut editor, SHORT_SCREEN);
                    let entered = hovered(&editor);
                    if key == "fdd2" {
                        assert_eq!(entered, Some("fdd1"), "{name} {height}");
                    }
                    assert_eq!(
                        shown_help_in(&mut harness, &mut editor, SHORT_SCREEN),
                        entered.or(Some(key)),
                        "{name} {key} {height}"
                    );
                }
            }
        }
        if cfg!(feature = "pc98") {
            assert!(second_drive_presses > 0);
        }
    }

    /// Verifies that a press on a disabled button of an empty media list keeps the focus: the
    /// Remove button and the two buttons that shift an image, which take the second and the third
    /// column of the buttons.
    #[test]
    fn a_press_on_a_disabled_button_of_a_row_keeps_the_focus() {
        let root = EditorRoot::new("row_press_disabled_button");
        let mut harness = GuiHarness::minimum();
        let mut editor = media_section(&root, &mut harness, MINIMUM_SCREEN, "");
        let fdd1 = index_of(&editor, "fdd1");
        assert!(editor.fields[fdd1].images().is_empty());
        let row = row_of(&editor, "fdd1");
        let theme = &harness.theme;
        let wide = RowAreas::new(theme, row.rect, row.rect.y).wide;
        let size = theme.control_height;
        let pitch = size + theme.gap;
        let left = wide.right() - buttons_width(theme);
        let tabs = UiId::from(TABS_ID);
        for (column, line) in [(2.0, 0.0), (1.0, 1.0), (2.0, 1.0)] {
            let x = left + column * pitch + size * 0.5;
            let y = wide.y + line * pitch + size * 0.5;
            harness.state.set_focus(tabs);
            show(&mut harness, &mut editor);
            click(&mut harness, &mut editor, x, y);
            assert_eq!(harness.state.focused(), Some(tabs), "{x} {y}");
        }
        let add = (left + pitch + size * 0.5, wide.y + size * 0.5);
        harness.state.set_focus(tabs);
        show(&mut harness, &mut editor);
        harness.input.mouse_x = add.0;
        harness.input.mouse_y = add.1;
        harness.input.left = true;
        show(&mut harness, &mut editor);
        assert_eq!(harness.state.focused(), Some(field_id("fdd1").child("add")));
    }
}
