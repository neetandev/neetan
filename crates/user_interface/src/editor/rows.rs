//! The form rows of the editor: the label with its key, the control that the
//! value kind of a field picks and the note after it.

use gui::{
    Rect,
    ui::{
        ActionButton, Align, Button, Checkbox, Dropdown, Icon, NumberField, TextField, Theme,
        Truncation, UiContext, UiId, label, wrapped_height, wrapped_label,
    },
};
use machine_factory::config::settings::{
    MACHINE_KEY, NumberRange, Setting, ValueKind,
    file_types::{FileTypes, ImageIcon},
};

use super::{
    BrowseTarget, Editor, EditorFile, Subject,
    fields::{FieldValue, is_single_image},
    form::{EditorSection, FieldSource},
    hints::{EditorWidget, WidgetKind},
    host_settings::{HostSettings, draw_settings_file_block},
    kept_lines::draw_kept_lines,
    media_list::{empty_list_height, empty_list_id},
    notes::{
        ChoiceNote, EMPTY_IMAGE, NOT_SET, Note, NoteKind, choice_note, decimals_of,
        empty_list_caption, inherited_placeholder, number_note, path_note,
    },
    page::{PageActions, TABS_ID},
};
use crate::config::metadata::{FIRST_YEAR, LAST_YEAR, TITLE_KEY, YEAR_KEY};

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

/// The note of the system row of a new game.
const SYSTEM_NOTE: &str = "The game belongs to the system selected in the library.";

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

/// A row that shows a value the user cannot type, such as the system of a new game.
#[derive(Debug, Clone)]
pub(super) struct ReadOnlyRow {
    /// The label.
    pub(super) name: &'static str,
    /// The value.
    pub(super) text: String,
    /// Whether the value is a path, which takes the folder icon.
    pub(super) path: bool,
    /// The note after the value.
    pub(super) note: String,
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

/// Draws `note` as a line of a section from `y` in `content`, inset from
/// the labels of the rows. Returns the height of the line.
fn draw_section_note(context: &mut UiContext, content: Rect, y: f32, note: &Note) -> f32 {
    let theme = context.theme;
    let area = Rect::new(
        content.x + theme.gap * 1.5,
        y + theme.gap * 0.5,
        (content.width - theme.gap * 1.5).max(0.0),
        theme.control_height,
    );
    let height = draw_note(context, area, note);
    RowAreas::height(theme, height)
}

/// Draws `note` into `area`: small secondary text, a warning with the alert
/// icon in the error color, a found thing with the check icon in the marker
/// color, or a status without a mark after an empty icon slot. A note wider
/// than `area` wraps over up to [`NOTE_MAXIMUM_LINES`] lines, centered on
/// the control while the lines fit its height and from its top else. A path
/// keeps to one line and gives way in the middle. Returns the height of the
/// note, at least the height of `area`.
pub(crate) fn draw_note(context: &mut UiContext, area: Rect, note: &Note) -> f32 {
    draw_note_text(context, area, &note.text, note.kind, note.path)
}

/// Draws the note `text` of `kind` into `area` as [`draw_note`] does. A
/// `path` keeps to one line and gives way in the middle. Returns the height
/// of the note, at least the height of `area`.
pub(crate) fn draw_note_text(
    context: &mut UiContext,
    area: Rect,
    text: &str,
    kind: NoteKind,
    path: bool,
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
        let lines = wrapped_height(context, &style, text, text_area.width, NOTE_MAXIMUM_LINES);
        let top = if lines < area.height {
            area.y + (area.height - lines) * 0.5
        } else {
            area.y
        };
        (
            Rect::new(text_area.x, top, text_area.width, lines),
            lines.max(area.height),
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
        wrapped_label(
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
        let section = self.current_section();
        let mut y = content.y;
        if section == EditorSection::Metadata
            && let Some(file) = self.file.derived().cloned()
        {
            let rows = [
                ReadOnlyRow {
                    name: "System",
                    text: self
                        .system_files()
                        .map_or("", |files| files.system.label)
                        .to_owned(),
                    path: false,
                    note: SYSTEM_NOTE.to_owned(),
                },
                ReadOnlyRow {
                    name: "File",
                    text: self.shown_path(),
                    path: true,
                    note: file.note,
                },
            ];
            for row in &rows {
                y += Self::draw_read_only(context, content, y, row);
            }
        }
        if section == EditorSection::Folders
            && let Some(row) = self.root_row()
        {
            y += Self::draw_read_only(context, content, y, &row);
        }
        let indices: Vec<usize> = (0..self.fields.len())
            .filter(|&index| self.fields[index].section == section)
            .collect();
        if indices.is_empty()
            && section != EditorSection::Metadata
            && let Some(files) = self.system_files()
        {
            let note = Note::plain(section.empty_text(files.system.label));
            y += draw_section_note(context, content, y, &note);
        }
        if section == EditorSection::Roms
            && let Some(note) = self
                .host_settings()
                .and_then(HostSettings::roms_folder_flag_note)
        {
            y += draw_section_note(context, content, y, &note);
        }
        for index in indices {
            y += self.draw_field(context, content, y, index, actions);
        }
        if section == EditorSection::Advanced && !self.kept.is_empty() {
            y += draw_kept_lines(context, content, y, &self.kept);
        }
        if section == EditorSection::Folders && self.host_settings().is_some() {
            y += draw_settings_file_block(context, content, y);
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
        row: &ReadOnlyRow,
    ) -> f32 {
        let theme = context.theme;
        let areas = RowAreas::new(theme, content, y);
        draw_label(context, areas.label, row.name, "", false, true);
        let mut value = row.text.clone();
        TextField::new(UiId::from("read only").child(row.name), "")
            .icon(row.path.then_some(Icon::Folder))
            .clears_on_cancel(false)
            .read_only(true)
            .truncation(Truncation::Middle)
            .show(context, areas.control, &mut value);
        let note = draw_note(context, areas.note, &Note::plain(row.note.clone()));
        RowAreas::height(theme, note)
    }

    /// Draws the row of the field at `index` from `y` in `content` and
    /// returns its height.
    fn draw_field(
        &mut self,
        context: &mut UiContext,
        content: Rect,
        y: f32,
        index: usize,
        actions: &mut PageActions,
    ) -> f32 {
        let theme = context.theme;
        let areas = RowAreas::new(theme, content, y);
        let source = self.fields[index].source;
        let id = field_id(source.key());
        let set = self.fields[index].is_set();
        let setting = match source {
            FieldSource::Metadata(metadata) => {
                draw_label(
                    context,
                    areas.label,
                    metadata.label,
                    metadata.key,
                    set,
                    true,
                );
                self.claim_first_focus(context, id);
                let note = self.draw_metadata(context, areas, index, actions);
                return RowAreas::height(theme, note);
            }
            FieldSource::Setting(setting) => setting,
        };
        let ignored = self.ignored_note(setting);
        let enabled = ignored.is_none();
        draw_label(
            context,
            areas.label,
            setting.label,
            setting.key,
            set,
            enabled,
        );
        if enabled {
            self.claim_first_focus(context, id);
        }
        match setting.kind {
            ValueKind::Language if matches!(self.subject, Subject::Host(_)) => {
                let note = self.draw_language(context, areas, index, setting, actions);
                RowAreas::height(theme, note)
            }
            ValueKind::Choice(_) | ValueKind::OnOff | ValueKind::Language => {
                let note = self.draw_choice(context, areas, index, setting, ignored, actions);
                RowAreas::height(theme, note)
            }
            ValueKind::Number(range) => {
                let note = self.draw_number(context, areas, index, range, ignored, actions);
                RowAreas::height(theme, note)
            }
            ValueKind::Path(_) => self.draw_path(context, areas, index, setting, ignored, actions),
            ValueKind::PathList => {
                self.draw_media(context, areas, index, setting, ignored, actions)
            }
        }
    }

    /// Clears the text field at `index` when gamepad X reaches the focused
    /// field `id` while the field is set. Returns whether it cleared.
    fn clear_on_x(&mut self, context: &UiContext, id: UiId, index: usize) -> bool {
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

    /// Draws the text or year field of the metadata field at `index`.
    /// Returns the height of its note.
    fn draw_metadata(
        &mut self,
        context: &mut UiContext,
        areas: RowAreas,
        index: usize,
        actions: &mut PageActions,
    ) -> f32 {
        let theme = context.theme;
        let key = self.fields[index].key();
        let id = field_id(key);
        let note = match &self.file {
            EditorFile::Existing(path) if key == TITLE_KEY => path
                .file_stem()
                .map(|stem| {
                    format!(
                        "While the title is empty, the library shows the file name {}.",
                        stem.to_string_lossy()
                    )
                })
                .unwrap_or_default(),
            EditorFile::Existing(_) | EditorFile::New(_) | EditorFile::Copy { .. } => String::new(),
        };
        let set = self.fields[index].is_set();
        let kind = if key == YEAR_KEY {
            WidgetKind::Number
        } else {
            WidgetKind::Text
        };
        self.widgets.push(EditorWidget::new(id, kind).clears(set));
        if let Some(text) = self.fields[index].text_mut() {
            let response = if key == YEAR_KEY {
                let field = NumberField::new(id, NOT_SET)
                    .range(f64::from(FIRST_YEAR), f64::from(LAST_YEAR));
                let rect = Rect::new(
                    areas.control.x,
                    areas.control.y,
                    field.width(context).min(areas.control.width),
                    theme.control_height,
                );
                field.show(context, rect, text)
            } else {
                TextField::new(id, NOT_SET)
                    .icon(None)
                    .clears_on_cancel(false)
                    .show(context, areas.control, text)
            };
            actions.changed |= response.changed;
        }
        actions.changed |= self.clear_on_x(context, id, index);
        draw_note(context, areas.note, &Note::plain(note))
    }

    /// Draws the dropdown of the choice, on/off or language field at
    /// `index`. Returns the height of its note.
    fn draw_choice(
        &mut self,
        context: &mut UiContext,
        areas: RowAreas,
        index: usize,
        setting: &'static Setting,
        ignored: Option<String>,
        actions: &mut PageActions,
    ) -> f32 {
        let id = field_id(setting.key);
        let inherited = self.inherited[index].clone();
        let inherited_label = inherited_placeholder(inherited.as_ref());
        let enabled = ignored.is_none();
        if enabled {
            self.widgets
                .push(EditorWidget::new(id, WidgetKind::Dropdown));
        }
        let field = &mut self.fields[index];
        let required = field.required;
        let inherited_index = inherited
            .as_ref()
            .and_then(|inherited| inherited.values.first())
            .and_then(|value| field.choices.iter().position(|choice| choice == value));
        let mut selected = if required {
            field.selected().or(inherited_index)
        } else {
            field.selected()
        };
        let before = selected;
        let inherited_entry = (!required).then_some(inherited_label.as_str());
        let dropdown = Dropdown::new(id, &field.choices).enabled(enabled);
        if show_dropdown(
            context,
            areas.control,
            dropdown,
            inherited_entry,
            &mut selected,
        ) && selected != before
        {
            field.select(selected);
            actions.changed = true;
        }
        let field = &self.fields[index];
        let preset_model = self.system_files().and_then(|files| files.preset_model);
        let preset =
            setting.key == MACHINE_KEY && preset_model.is_some() && field.chosen() == preset_model;
        let fact = match &self.subject {
            Subject::Game(files) | Subject::SystemDefaults(files) => {
                setting.choice_fact(self.model_of(files), field.chosen())
            }
            Subject::Host(_) => None,
        };
        let note = choice_note(ChoiceNote {
            ignored,
            invalid: field.invalid_value(),
            preset,
            set: field.is_set(),
            required,
            inherited: inherited.as_ref(),
            derived: self
                .system_files()
                .is_some_and(|files| setting.has_derived_default(files.target)),
            fact,
        })
        .followed_by(self.extra_note(setting));
        draw_note(context, areas.note, &note)
    }

    /// Draws the number field of the field at `index` in the range `range`.
    /// Returns the height of its note.
    fn draw_number(
        &mut self,
        context: &mut UiContext,
        areas: RowAreas,
        index: usize,
        range: NumberRange,
        ignored: Option<String>,
        actions: &mut PageActions,
    ) -> f32 {
        let theme = context.theme;
        let key = self.fields[index].key();
        let id = field_id(key);
        let enabled = ignored.is_none();
        if enabled {
            let set = self.fields[index].is_set();
            self.widgets
                .push(EditorWidget::new(id, WidgetKind::Number).clears(set));
        }
        let inherited = self.inherited[index].clone();
        let placeholder = inherited_placeholder(inherited.as_ref());
        let fallback = inherited
            .as_ref()
            .and_then(|inherited| inherited.values.first())
            .and_then(|value| value.parse::<f64>().ok());
        let field = NumberField::new(id, &placeholder)
            .range(range.minimum, range.maximum)
            .step(range.step)
            .decimals(decimals_of(range))
            .fallback(fallback)
            .enabled(enabled);
        let rect = Rect::new(
            areas.control.x,
            areas.control.y,
            field.width(context).min(areas.control.width),
            theme.control_height,
        );
        if let Some(text) = self.fields[index].text_mut() {
            actions.changed |= field.show(context, rect, text).changed;
        }
        if enabled {
            actions.changed |= self.clear_on_x(context, id, index);
        }
        let set = self.fields[index].is_set();
        let extra = self.fields[index]
            .source
            .setting()
            .and_then(|setting| self.extra_note(setting));
        let note = number_note(ignored, set, range, inherited.as_ref()).followed_by(extra);
        draw_note(context, areas.note, &note)
    }

    /// The placeholder of the path field at `index`: [`EMPTY_IMAGE`] for a
    /// field that the file empties, else the inherited value.
    pub(crate) fn path_placeholder(&self, index: usize) -> String {
        if self.fields[index].is_empty_path() {
            EMPTY_IMAGE.to_owned()
        } else {
            inherited_placeholder(self.inherited[index].as_ref())
        }
    }

    /// The note, the note of a command line flag and the error edge of the
    /// path field at `index` of `setting`. A missing file on the settings
    /// page shows its warning with the notes of the key.
    pub(super) fn path_notes(
        &self,
        index: usize,
        setting: &'static Setting,
        ignored: Option<String>,
    ) -> (Note, Option<Note>, bool) {
        if let Some(host_note) = self.host_path_note(index, setting) {
            return (host_note.note, host_note.flag, host_note.error);
        }
        let missing = self.missing[index].first().copied().unwrap_or(false);
        let note = match self.host_settings() {
            Some(settings) if missing => settings.missing_file_note(setting),
            Some(_) | None => path_note(
                ignored,
                missing,
                self.fields[index].is_set(),
                self.inherited[index].as_ref(),
                self.file_types(setting),
            )
            .followed_by(self.extra_note(setting)),
        };
        (note, None, false)
    }

    /// Draws the path field and the Browse button of the field at `index`,
    /// and the checkbox that empties the drive below the single image of a
    /// drive that inherits an image or that the file empties. A path cuts
    /// in the middle. The note of a command line flag on the settings page
    /// takes its own line below the note. Returns the height of the row.
    fn draw_path(
        &mut self,
        context: &mut UiContext,
        areas: RowAreas,
        index: usize,
        setting: &'static Setting,
        ignored: Option<String>,
        actions: &mut PageActions,
    ) -> f32 {
        let theme = context.theme;
        let id = field_id(setting.key);
        let empty_id = empty_list_id(id);
        let enabled = ignored.is_none();
        let inherited = self.inherited[index].clone();
        let types = self.file_types(setting);
        let empty_path = self.fields[index].is_empty_path();
        let placeholder = self.path_placeholder(index);
        let inherits_image = inherited
            .as_ref()
            .is_some_and(|inherited| !inherited.values.is_empty());
        let shows_toggle = enabled
            && is_single_image(setting)
            && self.fields[index].text().trim().is_empty()
            && (empty_path || inherits_image || context.has_focus(empty_id));
        let browse = Button::new(browse_id(setting.key), "Browse").enabled(enabled);
        let browse_width = browse.width(context);
        let (browse_area, field_area) = areas
            .control
            .cut_right(browse_width.min(areas.control.width));
        let field_area = field_area.cut_right(theme.gap).1;
        let (note, flag, error) = self.path_notes(index, setting, ignored);
        if enabled {
            let set = self.fields[index].is_set();
            self.widgets
                .push(EditorWidget::new(id, WidgetKind::Text).clears(set));
            self.widgets.push(EditorWidget::new(
                browse_id(setting.key),
                WidgetKind::Browse,
            ));
        }
        let text_field = TextField::new(id, &placeholder)
            .icon(Some(Icon::Folder))
            .clears_on_cancel(false)
            .truncation(Truncation::Middle)
            .enabled(enabled)
            .error(error);
        let field = &mut self.fields[index];
        match field.text_mut() {
            Some(text) => actions.changed |= text_field.show(context, field_area, text).changed,
            None => {
                let mut typed = String::new();
                if text_field.show(context, field_area, &mut typed).changed && !typed.is_empty() {
                    field.value = FieldValue::Text(typed);
                    actions.changed = true;
                }
            }
        }
        if enabled {
            actions.changed |= self.clear_on_x(context, id, index);
        }
        if browse.show(context, browse_area) {
            actions.browse = Some(BrowseTarget::Path(index));
        }
        let note_height = draw_note(context, areas.note, &note);
        let mut below = 0.0;
        if let Some(flag) = &flag {
            let area = Rect::new(
                areas.note.x,
                areas.note.y + note_height,
                areas.note.width,
                theme.icon_size,
            );
            below = draw_note(context, area, flag);
        }
        let mut height = RowAreas::height(theme, note_height + below);
        if shows_toggle {
            self.widgets
                .push(EditorWidget::new(empty_id, WidgetKind::Checkbox));
            let area = Rect::new(
                field_area.x,
                areas.control.bottom() + theme.gap * 0.5,
                field_area.width,
                theme.control_height,
            );
            let caption = empty_list_caption(types);
            let mut checked = empty_path;
            if Checkbox::new(empty_id, &caption).show(context, area, &mut checked) {
                self.fields[index].value = if empty_path {
                    FieldValue::Text(String::new())
                } else {
                    FieldValue::EmptyPath
                };
                actions.changed = true;
            }
            height += empty_list_height(theme);
        }
        height
    }
}

#[cfg(test)]
mod tests {
    use gui::ui::{NUMBER_FIELD_WIDTH, wrapped_height};
    use machine_factory::config::{
        MachineSelection,
        settings::{MODEL_CHOICES, SETTINGS},
        systems::compiled_systems,
    };

    use super::*;
    use crate::{
        editor::{
            form::{EDITOR_SECTIONS, ignored_note},
            page::form_area,
            test_support::{EditorRoot, SCREEN},
        },
        gui_test_support::GuiHarness,
    };

    /// Verifies that every model-limit note of every compiled family fits the note column of a 960 x 600 window within its lines.
    #[test]
    fn every_model_limit_note_fits_at_the_minimum_window_size() {
        let mut harness = GuiHarness::new(960, 600);
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
        let mut notes = Vec::new();
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
                    if let Some(note) = ignored_note(setting, model) {
                        notes.push(note);
                    }
                }
            }
        }
        let lines = harness.frame(|context| {
            let style = context.theme.small();
            let line = gui::ui::wrapped_line_height(&style);
            notes
                .iter()
                .map(|note| {
                    let height = wrapped_height(context, &style, note, note_width, usize::MAX);
                    (note.clone(), (height / line).round() as usize)
                })
                .collect::<Vec<_>>()
        });
        for (note, count) in lines {
            assert!(count <= NOTE_MAXIMUM_LINES, "{note} takes {count} lines");
        }
    }

    /// Verifies that a note that fits draws in the height of the control and that a longer note grows its row.
    #[test]
    fn a_long_note_wraps_and_grows_its_row() {
        let mut harness = GuiHarness::new(960, 600);
        let control_height = harness.theme.control_height;
        let area = Rect::new(600.0, 100.0, 200.0, control_height);
        let (short, long) = harness.frame(|context| {
            (
                draw_note(context, area, &Note::plain("Range 0 to 3.")),
                draw_note(
                    context,
                    area,
                    &Note::warning(
                        "PC9801VM has no EMS memory. VX, RS, RA, AS and AP only. The note \
                         of this row takes three lines.",
                    ),
                ),
            )
        });
        assert_eq!(short, control_height);
        assert!(long > control_height, "{long}");
        let theme = &harness.theme;
        assert_eq!(RowAreas::height(theme, short), FORM_ROW_PITCH);
        assert!(RowAreas::height(theme, long) > FORM_ROW_PITCH);
    }

    /// Verifies that the number field of pc6000-phase keeps its grown width
    /// when the game sets the key.
    #[test]
    fn a_number_field_keeps_its_width_when_the_game_sets_it() {
        if !compiled_systems().any(|system| system.name == "pc6000") {
            return;
        }
        let widths: Vec<f32> = ["", "pc6000-phase = 1\n"]
            .into_iter()
            .map(|line| {
                let root = EditorRoot::with_system("number_width", "pc6000");
                let path = root.game("portopia", &format!("machine = PC6001MK2SR\n{line}"));
                let mut editor =
                    Editor::open(&root.host(), root.system(), &path).expect("the editor opens");
                editor.section = EDITOR_SECTIONS
                    .iter()
                    .position(|section| *section == EditorSection::Display)
                    .expect("the display section");
                let mut harness = GuiHarness::new(960, 600);
                harness.settle(|context| {
                    editor.show(context, SCREEN);
                });
                let theme = &harness.theme;
                let frames: Vec<[f32; 4]> = harness
                    .scene
                    .instances()
                    .iter()
                    .filter(|instance| {
                        instance.kind[0] == gui::Instance::KIND_RECTANGLE
                            && instance.color == theme.colors.field.to_linear()
                            && instance.rect[3] == theme.control_height
                    })
                    .map(|instance| instance.rect)
                    .collect();
                assert_eq!(frames.len(), 1, "{line:?}: {frames:?}");
                frames[0][2]
            })
            .collect();
        assert_eq!(widths[0], widths[1], "{widths:?}");
        assert!(widths[0] > NUMBER_FIELD_WIDTH, "{widths:?}");
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
