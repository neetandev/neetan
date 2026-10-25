//! The form rows of the setting values: the dropdown of a choice, the
//! number field and the path field with its Browse button, each with its
//! note.

use gui::{
    Rect,
    ui::{Button, Checkbox, Dropdown, Icon, NumberField, TextField, Truncation, UiContext},
};
use machine_factory::config::settings::{MACHINE_KEY, NumberRange, Setting};

use super::{
    BrowseTarget, Editor, Subject,
    fields::{FieldValue, is_single_image},
    form::IgnoredBy,
    hints::{EditorWidget, WidgetKind},
    media_list::{empty_list_height, empty_list_id},
    note_writer::NoteWriter,
    notes::{
        ChoiceNote, NoteKind, NoteLook, choice_note, decimals_of, followed_by, number_note,
        path_note, write_empty_list_caption, write_inherited_placeholder,
    },
    page::PageActions,
    rows::{RowAreas, SettingRow, browse_id, draw_note, field_id, show_dropdown},
};
use crate::texts::{common, host_settings, note};

impl Editor {
    /// Writes the note of the choice, on/off or language field at `index`
    /// of `setting` that the model of the editor ignores under `ignored`.
    fn write_choice_note(
        &self,
        writer: &mut NoteWriter,
        index: usize,
        setting: &'static Setting,
        ignored: Option<IgnoredBy>,
    ) -> NoteLook {
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
        let look = choice_note(
            writer,
            ChoiceNote {
                ignored,
                invalid: field.invalid_value(),
                preset,
                set: field.is_set(),
                required: field.required,
                inherited: self.inherited[index].as_ref(),
                derived: self
                    .system_files()
                    .is_some_and(|files| setting.has_derived_default(files.target)),
                fact: fact.as_deref(),
            },
        );
        followed_by(writer, look, |writer| self.write_extra(writer, setting))
    }

    /// Draws the dropdown of the choice, on/off or language field of `row`.
    /// Returns the height of its note.
    pub(super) fn draw_choice(
        &mut self,
        context: &mut UiContext,
        row: SettingRow,
        actions: &mut PageActions,
    ) -> f32 {
        let SettingRow {
            areas,
            index,
            setting,
            ignored,
            texts,
        } = row;
        let catalog = context.catalog;
        let generation = context.catalog_generation;
        let revision = self.revision;
        let id = field_id(setting.key);
        let enabled = ignored.is_none();
        if enabled {
            self.widgets
                .push(EditorWidget::new(id, WidgetKind::Dropdown));
        }
        let inherited = self.inherited[index].as_ref();
        let inherited_label =
            texts
                .placeholder
                .text_in(catalog, generation, revision, |catalog, _, out| {
                    write_inherited_placeholder(catalog, out, inherited);
                });
        let field = &mut self.fields[index];
        let required = field.required;
        let inherited_index = inherited
            .and_then(|inherited| inherited.values.first())
            .and_then(|value| field.choices.iter().position(|choice| choice == value));
        let mut selected = if required {
            field.selected().or(inherited_index)
        } else {
            field.selected()
        };
        let before = selected;
        let inherited_entry = (!required).then_some(inherited_label);
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
        let note = texts
            .note
            .note(catalog, generation, (revision, false), |writer, _| {
                self.write_choice_note(writer, index, setting, ignored)
            });
        draw_note(context, areas.note, note)
    }

    /// Draws the number field of `row` in the range `range`. Returns the
    /// height of its note.
    pub(super) fn draw_number(
        &mut self,
        context: &mut UiContext,
        row: SettingRow,
        range: NumberRange,
        actions: &mut PageActions,
    ) -> f32 {
        let SettingRow {
            areas,
            index,
            setting,
            ignored,
            texts,
        } = row;
        let theme = context.theme;
        let catalog = context.catalog;
        let generation = context.catalog_generation;
        let revision = self.revision;
        let id = field_id(setting.key);
        let enabled = ignored.is_none();
        if enabled {
            let set = self.fields[index].is_set();
            self.widgets
                .push(EditorWidget::new(id, WidgetKind::Number).clears(set));
        }
        let inherited = self.inherited[index].as_ref();
        let placeholder =
            texts
                .placeholder
                .text_in(catalog, generation, revision, |catalog, _, out| {
                    write_inherited_placeholder(catalog, out, inherited);
                });
        let fallback = inherited
            .and_then(|inherited| inherited.values.first())
            .and_then(|value| value.parse::<f64>().ok());
        let field = NumberField::new(id, placeholder)
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
        let note = texts
            .note
            .note(catalog, generation, (revision, false), |writer, _| {
                let field = &self.fields[index];
                let look = number_note(
                    writer,
                    ignored,
                    field.is_set(),
                    range,
                    self.inherited[index].as_ref(),
                );
                followed_by(writer, look, |writer| self.write_extra(writer, setting))
            });
        draw_note(context, areas.note, note)
    }

    /// Writes the placeholder of the path field at `index`: the text
    /// `note.empty-image` for a field that the file empties, else the
    /// inherited value.
    pub(super) fn write_path_placeholder(
        &self,
        catalog: &i18n::Catalog,
        out: &mut String,
        index: usize,
    ) {
        if self.fields[index].is_empty_path() {
            out.push_str(catalog.text(note::EMPTY_IMAGE));
        } else {
            write_inherited_placeholder(catalog, out, self.inherited[index].as_ref());
        }
    }

    /// Writes the note of the path field at `index` of `setting` that the
    /// model of the editor ignores under `ignored`. A missing file on the
    /// settings page shows its warning with the notes of the key.
    pub(super) fn write_path_note(
        &self,
        writer: &mut NoteWriter,
        index: usize,
        setting: &'static Setting,
        ignored: Option<IgnoredBy>,
    ) -> NoteLook {
        if let Some(host_note) = self.host_path_state(index, setting) {
            host_note.note.write(writer);
            return host_note.look;
        }
        let missing = self.missing[index].first().copied().unwrap_or(false);
        match self.host_settings() {
            Some(settings) if missing => settings.missing_file_note(writer, setting),
            Some(_) | None => {
                let look = path_note(
                    writer,
                    ignored,
                    missing,
                    self.fields[index].is_set(),
                    self.inherited[index].as_ref(),
                    self.file_types(setting),
                );
                followed_by(writer, look, |writer| self.write_extra(writer, setting))
            }
        }
    }

    /// Draws the path field and the Browse button of the field of `row`,
    /// and the checkbox that empties the drive below the single image of a
    /// drive that inherits an image or that the file empties. A path cuts in
    /// the middle. The note of a command line flag on the settings page
    /// takes its own line below the note. Returns the height of the row.
    pub(super) fn draw_path(
        &mut self,
        context: &mut UiContext,
        row: SettingRow,
        actions: &mut PageActions,
    ) -> f32 {
        let SettingRow {
            areas,
            index,
            setting,
            ignored,
            texts,
        } = row;
        let theme = context.theme;
        let catalog = context.catalog;
        let generation = context.catalog_generation;
        let revision = self.revision;
        let id = field_id(setting.key);
        let empty_id = empty_list_id(id);
        let enabled = ignored.is_none();
        let types = self.file_types(setting);
        let empty_path = self.fields[index].is_empty_path();
        let inherits_image = self.inherited[index]
            .as_ref()
            .is_some_and(|inherited| !inherited.values.is_empty());
        let shows_toggle = enabled
            && is_single_image(setting)
            && self.fields[index].text().trim().is_empty()
            && (empty_path || inherits_image || context.has_focus(empty_id));
        let (error, found) = self
            .host_path_state(index, setting)
            .map_or((false, false), |host| {
                (host.error, host.look.kind == NoteKind::Found)
            });
        let flag = self
            .host_settings()
            .and_then(|settings| settings.flag(setting.key))
            .is_some();
        let browse =
            Button::new(browse_id(setting.key), catalog.text(common::BROWSE)).enabled(enabled);
        let browse_width = browse.width(context);
        let (browse_area, field_area) = areas
            .control
            .cut_right(browse_width.min(areas.control.width));
        let field_area = field_area.cut_right(theme.gap).1;
        if enabled {
            let set = self.fields[index].is_set();
            self.widgets
                .push(EditorWidget::new(id, WidgetKind::Text).clears(set));
            self.widgets.push(EditorWidget::new(
                browse_id(setting.key),
                WidgetKind::Browse,
            ));
        }
        let placeholder =
            texts
                .placeholder
                .text_in(catalog, generation, revision, |catalog, _, out| {
                    self.write_path_placeholder(catalog, out, index);
                });
        let text_field = TextField::new(id, placeholder)
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
        let note = texts
            .note
            .note(catalog, generation, (revision, found), |writer, _| {
                self.write_path_note(writer, index, setting, ignored)
            });
        let note_height = draw_note(context, areas.note, note);
        let mut below = 0.0;
        if flag {
            let note = texts.flag.note(catalog, generation, revision, |writer, _| {
                if let Some(value) = self
                    .host_settings()
                    .and_then(|settings| settings.flag(setting.key))
                {
                    writer.message(host_settings::flag(&value));
                }
                NoteLook::PLAIN
            });
            let area = Rect::new(
                areas.note.x,
                areas.note.y + note_height,
                areas.note.width,
                theme.icon_size,
            );
            below = draw_note(context, area, note);
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
            let caption =
                texts
                    .caption
                    .text_in(catalog, generation, revision, |catalog, _, out| {
                        write_empty_list_caption(catalog, out, types);
                    });
            let mut checked = empty_path;
            if Checkbox::new(empty_id, caption).show(context, area, &mut checked) {
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
impl Editor {
    /// The note, the note of a command line flag and the error edge of the
    /// path field at `index` of `setting` in English.
    pub(super) fn path_notes(
        &self,
        index: usize,
        setting: &'static Setting,
        ignored: Option<IgnoredBy>,
    ) -> (
        super::notes::tests::Note,
        Option<super::notes::tests::Note>,
        bool,
    ) {
        let note = super::notes::tests::english_note(|writer| {
            self.write_path_note(writer, index, setting, ignored)
        });
        match self.host_path_note(index, setting) {
            Some(host) => (note, host.flag, host.error),
            None => (note, None, false),
        }
    }
}

#[cfg(test)]
mod tests {
    use gui::ui::NUMBER_FIELD_WIDTH;
    use machine_factory::config::systems::compiled_systems;

    use super::*;
    use crate::{
        editor::{
            form::{EDITOR_SECTIONS, EditorSection},
            test_support::{EditorRoot, SCREEN},
        },
        gui_test_support::GuiHarness,
    };

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
}
