//! The form rows of the editor: the label with its key, the control that the
//! value kind of a field picks and the note after it.

use std::fmt::Write as _;

use gui::{
    Rect,
    ui::{
        ActionButton, Align, Dropdown, Icon, NumberField, TextBuffer, TextField, Theme, Truncation,
        UiContext, UiId, WrappedText, label,
    },
};
use i18n::Key;
use machine_factory::config::settings::{
    Setting, ValueKind,
    file_types::{FileTypes, ImageIcon},
};

use super::{
    Editor, EditorFile, Subject,
    form::{EditorSection, FieldSource, IgnoredBy},
    help_area::FormRow,
    hints::{EditorWidget, WidgetKind},
    host_settings::draw_settings_file_block,
    kept_lines::draw_kept_lines,
    note_writer::{NoteText, NoteView},
    notes::{NoteKind, NoteLook},
    page::{PageActions, TABS_ID},
};
use crate::{
    config::metadata::{FIRST_YEAR, LAST_YEAR, TITLE_KEY, YEAR_KEY},
    texts::{editor, host_settings, note, setting_label},
};

/// The width of the label column of a form row.
pub(crate) const FORM_LABEL_WIDTH: f32 = 184.0;

/// The width of the control column of a form row.
pub(crate) const FORM_FIELD_WIDTH: f32 = 360.0;

/// The height of a form row with one control.
pub(crate) const FORM_ROW_PITCH: f32 = 40.0;

/// The size of the dot before the label of a key that the file sets.
pub(crate) const DOT_SIZE: f32 = 6.0;

/// The most lines of a note that does not fit its row in one line.
pub(crate) const NOTE_MAXIMUM_LINES: usize = 3;

/// The line height of the label and of the key of a form row, in units of
/// their text sizes: the normal line height of the font of the theme.
const LABEL_LINE_SPACING: f32 = 1.2;

/// Draws `dropdown` with the inherited entry `inherited` in `rect` over the
/// value `selected`. Returns whether the value changed.
pub(super) fn show_dropdown<T: AsRef<str>>(
    context: &mut UiContext,
    rect: Rect,
    dropdown: Dropdown<'_, T>,
    inherited: Option<&str>,
    selected: &mut Option<usize>,
) -> bool {
    match inherited {
        Some(label) => dropdown.inherited(label).show(context, rect, selected),
        None => dropdown.show(context, rect, selected),
    }
}

/// Returns the id of the widget of the field `key`.
pub(crate) fn field_id(key: &str) -> UiId {
    UiId::from("field").child(key)
}

/// Returns the id of the Browse button of the field `key`.
pub(crate) fn browse_id(key: &str) -> UiId {
    field_id(key).child("browse")
}

/// Returns the icon of the images of `types`.
pub(crate) fn image_icon(types: Option<&FileTypes>) -> Icon {
    match types.map(|types| types.icon) {
        Some(ImageIcon::Disc) => Icon::Disc,
        Some(ImageIcon::Disk) | None => Icon::Disk,
    }
}

/// The texts of the row of one field that refill only when the state of
/// the editor or the catalog changes.
#[derive(Debug, Clone, Default)]
pub(crate) struct RowTexts {
    /// The note after the control or below a media list, for the revision
    /// of the editor and whether the folder of the note exists.
    pub(crate) note: NoteText<(u64, bool)>,
    /// The note of a command line flag below the note of a path.
    pub(crate) flag: NoteText<u64>,
    /// The placeholder of a field or the inherited entry of a dropdown.
    pub(crate) placeholder: TextBuffer<u64>,
    /// The caption of the checkbox that empties a drive.
    pub(crate) caption: TextBuffer<u64>,
    /// The text of a media list without images.
    pub(crate) hint: TextBuffer<u64>,
}

/// A row that shows a value the user cannot type, such as the system of a new game.
pub(super) struct ReadOnlyRow<'a> {
    /// The name of the id of the row.
    pub(super) id: &'static str,
    /// The key of the label.
    pub(super) name: Key,
    /// The value.
    pub(super) value: &'a mut String,
    /// Whether the value is a path, which takes the folder icon.
    pub(super) path: bool,
    /// The note after the value.
    pub(super) note: &'a str,
    /// The wrapped lines of the note.
    pub(super) lines: &'a mut WrappedText,
}

/// The form row of a setting field with the parts that its control draws.
pub(super) struct SettingRow<'a> {
    /// The areas of the row.
    pub(super) areas: RowAreas,
    /// The index of the field.
    pub(super) index: usize,
    /// The setting of the field.
    pub(super) setting: &'static Setting,
    /// The model that ignores the setting. A setting that the model uses has none.
    pub(super) ignored: Option<IgnoredBy>,
    /// The texts of the row that refill only when their data changes.
    pub(super) texts: &'a mut RowTexts,
}

/// The areas of one form row from its top `y` in `content`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RowAreas {
    /// The label and the key.
    pub(crate) label: Rect,
    /// The control.
    pub(crate) control: Rect,
    /// The note after the control.
    pub(crate) note: Rect,
    /// The control and the note together, for a media list.
    pub(crate) wide: Rect,
}

impl RowAreas {
    /// The areas of a row from `y` in `content` with the gaps of `theme`.
    pub(crate) fn new(theme: &Theme, content: Rect, y: f32) -> Self {
        let top = y + theme.gap * 0.5;
        let height = theme.control_height;
        let label = Rect::new(content.x, top, FORM_LABEL_WIDTH, height);
        let control_x = label.right() + theme.padding;
        let control_width = FORM_FIELD_WIDTH.min((content.right() - control_x).max(0.0));
        let control = Rect::new(control_x, top, control_width, height);
        let note_x = control.right() + theme.padding;
        let note = Rect::new(note_x, top, (content.right() - note_x).max(0.0), height);
        let wide = Rect::new(
            control_x,
            top,
            (content.right() - control_x).max(0.0),
            height,
        );
        Self {
            label,
            control,
            note,
            wide,
        }
    }

    /// The height of a row whose note takes `note_height`: the row pitch,
    /// or more for a note over several lines.
    pub(crate) fn height(theme: &Theme, note_height: f32) -> f32 {
        FORM_ROW_PITCH.max(note_height + theme.gap)
    }
}

/// Draws the label `name` with the key `key` below it into `area`, a dot
/// before it when `set`, and the name dimmed unless `enabled`. A row
/// without a key centers the name.
pub(crate) fn draw_label(
    context: &mut UiContext,
    area: Rect,
    name: &str,
    key: &str,
    set: bool,
    enabled: bool,
) {
    let theme = context.theme;
    if set {
        let dot = Rect::new(
            area.x,
            area.y + (theme.control_height - DOT_SIZE) * 0.5,
            DOT_SIZE,
            DOT_SIZE,
        );
        context
            .scene
            .fill(context.snap_rect(dot), DOT_SIZE * 0.5, theme.colors.accent);
    }
    let x = area.x + theme.gap * 1.5;
    let width = (area.width - theme.gap * 1.5).max(0.0);
    let name_color = if enabled {
        theme.colors.text
    } else {
        theme.colors.text_disabled
    };
    let name_style = theme.body().with_color(name_color);
    if key.is_empty() {
        label(
            context,
            Rect::new(x, area.y, width, theme.control_height),
            &name_style,
            name,
            Align::Start,
            Truncation::End,
        );
        return;
    }
    let name_height = theme.text_size * LABEL_LINE_SPACING;
    let key_height = theme.small_text_size * LABEL_LINE_SPACING;
    let top = area.y + (theme.control_height - name_height - key_height) * 0.5;
    label(
        context,
        Rect::new(x, top, width, name_height),
        &name_style,
        name,
        Align::Start,
        Truncation::End,
    );
    label(
        context,
        Rect::new(x, top + name_height, width, key_height),
        &theme.small(),
        key,
        Align::Start,
        Truncation::End,
    );
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

/// Draws the note `text` of `kind` into `area`: small secondary text, a
/// warning with the alert icon in the error color, a found thing with the
/// check icon in the marker color, or a status without a mark after an
/// empty icon slot. A note wider than `area` wraps over up to
/// [`NOTE_MAXIMUM_LINES`] lines, centered on the control while the lines
/// fit its height and from its top else. A `path` keeps to one line and
/// gives way in the middle. `lines` keeps the wrapped lines between passes.
/// Returns the height of the note, at least the height of `area`.
pub(crate) fn draw_note_text(
    context: &mut UiContext,
    area: Rect,
    text: &str,
    kind: NoteKind,
    path: bool,
    lines: &mut WrappedText,
) -> f32 {
    let theme = context.theme;
    if text.is_empty() || area.width <= 0.0 {
        return area.height;
    }
    let (icon, color) = match kind {
        NoteKind::Plain | NoteKind::Unmarked => (None, theme.colors.text_dim),
        NoteKind::Warning => (Some(Icon::Alert), theme.colors.danger_text),
        NoteKind::Found => (Some(Icon::Check), theme.colors.marker),
    };
    let (icon_area, text_area) = match kind {
        NoteKind::Plain => (None, area),
        NoteKind::Warning | NoteKind::Found | NoteKind::Unmarked => {
            let (icon_area, rest) = area.cut_left(theme.icon_size.min(area.width));
            (Some(icon_area), rest.cut_left(theme.gap * 0.75).1)
        }
    };
    let style = match kind {
        NoteKind::Plain | NoteKind::Unmarked => theme.small(),
        NoteKind::Warning | NoteKind::Found => theme.small().with_color(color),
    };
    let fits = path || context.measure(&style, text) <= text_area.width;
    let (lines_area, height) = if fits {
        (text_area, area.height)
    } else {
        let height = lines.height(context, &style, text, text_area.width, NOTE_MAXIMUM_LINES);
        let top = if height < area.height {
            area.y + (area.height - height) * 0.5
        } else {
            area.y
        };
        (
            Rect::new(text_area.x, top, text_area.width, height),
            height.max(area.height),
        )
    };
    if let (Some(icon), Some(icon_area)) = (icon, icon_area) {
        let line = if fits {
            icon_area
        } else {
            Rect::new(
                icon_area.x,
                lines_area.y,
                icon_area.width,
                gui::ui::wrapped_line_height(&style),
            )
        };
        context.icon(line, icon, theme.icon_size, color);
    }
    if fits {
        let truncation = if path {
            Truncation::Middle
        } else {
            Truncation::End
        };
        label(context, lines_area, &style, text, Align::Start, truncation);
    } else {
        lines.show(
            context,
            lines_area,
            &style,
            text,
            Align::Start,
            NOTE_MAXIMUM_LINES,
        );
    }
    height
}

/// Draws `note` with its look into `area` as [`draw_note_text`] does.
/// Returns the height of the note.
pub(crate) fn draw_note(context: &mut UiContext, area: Rect, note: NoteView) -> f32 {
    draw_note_text(
        context,
        area,
        note.text,
        note.look.kind,
        note.look.path,
        note.lines,
    )
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
    pub(super) fn draw_read_only(
        context: &mut UiContext,
        content: Rect,
        y: f32,
        row: ReadOnlyRow,
    ) -> f32 {
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
    /// its widgets take the pointer.
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
impl Editor {
    /// The most lines of the wrapped note of a field of `section`, from the last frame that
    /// drew each row.
    pub(crate) fn most_note_lines(&self, section: EditorSection) -> usize {
        self.fields
            .iter()
            .zip(&self.row_texts)
            .filter(|(field, _)| field.section == section)
            .map(|(_, texts)| texts.note.line_count().max(texts.flag.line_count()))
            .max()
            .unwrap_or(0)
    }

    /// The note of the row of the field `key` of the last frame. A row
    /// without a drawn note has none.
    pub(crate) fn cached_note(&self, key: &str) -> Option<&str> {
        let index = self.fields.iter().position(|field| field.key() == key)?;
        self.row_texts[index].note.last_text()
    }

    /// The sentences after the note of `setting` in English, as
    /// [`Editor::write_extra`] writes them. A setting without them gives
    /// `None`.
    pub(super) fn extra_note(&self, setting: &Setting) -> Option<String> {
        let note = super::notes::tests::english_note(|writer| {
            self.write_extra(writer, setting);
            NoteLook::PLAIN
        });
        (!note.text.is_empty()).then_some(note.text)
    }
}

#[cfg(test)]
mod tests {
    use machine_factory::config::{
        MachineSelection,
        settings::{
            MODEL_CHOICES, SETTINGS,
            model_limits::{ChoiceFact, FactData},
        },
        systems::compiled_systems,
    };

    use super::*;
    use crate::{
        editor::{
            form::{tests::ignored_note_in, write_models_only},
            inherited::{Inherited, InheritedSource},
            notes::{ChoiceNote, choice_note, tests::catalog_note},
            page::form_area,
            test_support::SCREEN,
        },
        gui_test_support::GuiHarness,
        texts::repository::repository_catalogs,
    };

    /// The width of the note column of a 960 x 600 window.
    fn minimum_note_width(harness: &GuiHarness) -> f32 {
        let theme = &harness.theme;
        let form = form_area(theme, SCREEN);
        let content = Rect::new(
            form.x,
            form.y,
            (form.width - theme.padding).max(0.0),
            form.height,
        );
        let note_width = RowAreas::new(theme, content, 0.0).note.width;
        assert!(note_width > 200.0, "{note_width}");
        note_width
    }

    /// Asserts that every note of `notes` fits the note column of a 960 x 600 window within
    /// [`NOTE_MAXIMUM_LINES`] lines.
    fn assert_notes_fit(harness: &mut GuiHarness, notes: &[String]) {
        let note_width = minimum_note_width(harness);
        let lines = harness.frame(|context| {
            let style = context.theme.small();
            let line = gui::ui::wrapped_line_height(&style);
            notes
                .iter()
                .map(|note| {
                    let height =
                        WrappedText::new().height(context, &style, note, note_width, usize::MAX);
                    (note.clone(), (height / line).round() as usize)
                })
                .collect::<Vec<_>>()
        });
        for (note, count) in lines {
            assert!(count <= NOTE_MAXIMUM_LINES, "{note} takes {count} lines");
        }
    }

    /// Verifies that every model-limit note of every compiled family fits the note column of a
    /// 960 x 600 window within its lines in every language of the repository.
    #[test]
    fn every_model_limit_note_fits_at_the_minimum_window_size() {
        let mut harness = GuiHarness::new(960, 600);
        let mut notes = Vec::new();
        for catalog in repository_catalogs() {
            for system in compiled_systems() {
                let target = MachineSelection::of_system(system)
                    .expect("compiled system")
                    .target();
                for choice in MODEL_CHOICES
                    .iter()
                    .filter(|choice| choice.families.contains_target(target))
                {
                    let model: MachineSelection = choice.value.parse().expect("a model");
                    for setting in SETTINGS {
                        if let Some(note) = ignored_note_in(catalog, setting, model) {
                            notes.push(note);
                        }
                    }
                }
            }
        }
        assert_notes_fit(&mut harness, &notes);
    }

    /// Verifies that the note of a choice with a fact fits the note column of a 960 x 600
    /// window within its lines in every language of the repository, for every model of every
    /// compiled family. The note holds the longest sentences before the fact: the inherited
    /// value of each source and the derived default. The models that the system defaults
    /// editor lists follow the fact. A clock note holds both clocks of the model.
    #[test]
    fn every_choice_fact_note_fits_at_the_minimum_window_size() {
        let mut harness = GuiHarness::new(960, 600);
        let mut notes = Vec::new();
        let sources = [
            InheritedSource::BuiltIn,
            InheritedSource::SystemDefaults,
            InheritedSource::SettingsFile,
        ];
        for catalog in repository_catalogs() {
            for system in compiled_systems() {
                let target = MachineSelection::of_system(system)
                    .expect("compiled system")
                    .target();
                for choice in MODEL_CHOICES
                    .iter()
                    .filter(|choice| choice.families.contains_target(target))
                {
                    let model: MachineSelection = choice.value.parse().expect("a model");
                    for setting in SETTINGS.iter().filter(|setting| {
                        setting.is_read_by(target, choice.value) && setting.fact != ChoiceFact::None
                    }) {
                        let Some(fact) = setting.choice_fact(model) else {
                            continue;
                        };
                        let value = setting
                            .choices_for(target)
                            .map(|choice| choice.value)
                            .max_by_key(|value| value.len())
                            .unwrap_or_default();
                        for source in sources {
                            let inherited = Inherited::new(vec![value.to_owned()], source);
                            let note = catalog_note(catalog, |writer| {
                                let look = choice_note(
                                    writer,
                                    ChoiceNote {
                                        set: true,
                                        inherited: Some(&inherited),
                                        derived: setting.has_derived_default(target),
                                        fact: Some(&fact),
                                        ..ChoiceNote::default()
                                    },
                                );
                                if let Some(limit) = setting.limit_for(target) {
                                    write_models_only(writer, limit);
                                }
                                look
                            });
                            if let FactData::CpuClock { low, high } = &fact {
                                for clock in [low, high] {
                                    let clock = clock.to_string();
                                    assert!(note.text.contains(&clock), "{}", note.text);
                                }
                            }
                            notes.push(note.text);
                        }
                    }
                }
            }
        }
        assert_notes_fit(&mut harness, &notes);
    }

    /// Verifies that a note that fits draws in the height of the control and that a longer note grows its row.
    #[test]
    fn a_long_note_wraps_and_grows_its_row() {
        let mut harness = GuiHarness::new(960, 600);
        let control_height = harness.theme.control_height;
        let area = Rect::new(600.0, 100.0, 200.0, control_height);
        let (short, long) = harness.frame(|context| {
            let short = NoteView {
                text: "Range 0 to 3.",
                look: NoteLook::PLAIN,
                lines: &mut WrappedText::new(),
            };
            let long = NoteView {
                text: "PC9801VM has no EMS memory. VX, RS, RA, AS and AP only. The note of this \
                       row takes three lines.",
                look: NoteLook::WARNING,
                lines: &mut WrappedText::new(),
            };
            (
                draw_note(context, area, short),
                draw_note(context, area, long),
            )
        });
        assert_eq!(short, control_height);
        assert!(long > control_height, "{long}");
        let theme = &harness.theme;
        assert_eq!(RowAreas::height(theme, short), FORM_ROW_PITCH);
        assert!(RowAreas::height(theme, long) > FORM_ROW_PITCH);
    }

    /// Verifies the icon of the images of each drive.
    #[test]
    fn drives_show_the_icon_of_their_images() {
        for setting in SETTINGS {
            for types in setting.file_types {
                let expected = match types.icon {
                    ImageIcon::Disc => Icon::Disc,
                    ImageIcon::Disk => Icon::Disk,
                };
                assert_eq!(image_icon(Some(types)), expected, "{}", setting.key);
            }
        }
        assert_eq!(image_icon(None), Icon::Disk);
    }
}
