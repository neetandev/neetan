//! The form row of a path list: the media list of one drive, the checkbox
//! that empties the drive over its inherited list, and the note below the
//! list.

use gui::{
    Rect,
    ui::{Truncation, UiContext},
};

use super::{
    BrowseTarget, Editor,
    fields::FieldValue,
    form::write_ignored,
    hints::{EditorWidget, WidgetKind},
    inherited::Inherited,
    media_list::{
        MediaList, button_ids, buttons_width, empty_list_height, empty_list_id, list_height,
        write_missing_note,
    },
    note_writer::NoteWriter,
    notes::{NoteLook, followed_by, media_note, write_empty_drive_hint, write_empty_list_caption},
    page::PageActions,
    rows::{SettingRow, draw_note, field_id, image_icon},
};

/// Writes the note below a media list: the warning of the images that
/// `missing` marks, or else the inherited list of a list that the file
/// sets. A disabled list shows no note.
pub(crate) fn below_note(
    writer: &mut NoteWriter,
    enabled: bool,
    missing: &[bool],
    set: bool,
    inherited: Option<&Inherited>,
) -> NoteLook {
    if !enabled {
        return NoteLook::PLAIN;
    }
    if write_missing_note(writer, missing) {
        return NoteLook::WARNING;
    }
    media_note(writer, set, inherited)
}

impl Editor {
    /// Draws the media list of the path list field of `row` and returns the
    /// height of its row.
    pub(super) fn draw_media(
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
        let enabled = ignored.is_none();
        let types = self.file_types(setting);
        let field = &self.fields[index];
        let has_images = !field.images().is_empty();
        let empty_list = field.is_empty_list();
        let inherited = self.inherited[index].as_ref();
        let inherits_images = inherited.is_some_and(|inherited| !inherited.values.is_empty());
        let shows_toggle = enabled
            && !has_images
            && (empty_list || inherits_images || context.has_focus(empty_list_id(id)));
        if enabled {
            self.widgets
                .push(EditorWidget::new(id, WidgetKind::MediaList).clears(has_images));
            let [add, remove, up, down] = button_ids(id);
            self.widgets.extend([
                EditorWidget::new(add, WidgetKind::AddImage),
                EditorWidget::new(remove, WidgetKind::RemoveImage),
                EditorWidget::new(up, WidgetKind::ShiftUp),
                EditorWidget::new(down, WidgetKind::ShiftDown),
            ]);
            if shows_toggle {
                self.widgets
                    .push(EditorWidget::new(empty_list_id(id), WidgetKind::Checkbox));
            }
        }
        let hint = texts.hint.text_in(
            catalog,
            generation,
            revision,
            |catalog, _, out| match ignored {
                Some(ignored) => {
                    let mut scratch = String::new();
                    let mut writer = NoteWriter::new(catalog, &mut scratch);
                    write_ignored(&mut writer, ignored);
                    writer.finish(out);
                }
                None => write_empty_drive_hint(catalog, out, types, inherited, empty_list),
            },
        );
        let hint_truncation = if ignored.is_none() && inherits_images && !empty_list {
            Truncation::Middle
        } else {
            Truncation::End
        };
        let caption = texts
            .caption
            .text_in(catalog, generation, revision, |catalog, _, out| {
                write_empty_list_caption(catalog, out, types);
            });
        let mut values = self.fields[index].images().to_vec();
        let height = list_height(theme, values.len());
        let wide = Rect::new(areas.wide.x, areas.wide.y, areas.wide.width, height);
        let response = MediaList {
            id,
            icon: image_icon(types),
            missing: &self.missing[index],
            enabled,
            hint,
            hint_truncation,
            empty_list_caption: shows_toggle.then_some(caption),
            empty_list,
        }
        .show(context, wide, &mut self.fields[index].list, &mut values);
        let field = &mut self.fields[index];
        if response.changed {
            field.value = FieldValue::List(values);
            actions.changed = true;
        }
        if response.empty_list_flipped {
            field.value = if empty_list {
                FieldValue::List(Vec::new())
            } else {
                FieldValue::EmptyList
            };
            actions.changed = true;
        }
        if let Some(browse) = response.browse {
            actions.browse = Some(BrowseTarget::List(index, browse));
        }
        let mut row = theme.gap * 0.5 + height + theme.gap;
        let mut below_top = wide.bottom();
        if shows_toggle {
            row += empty_list_height(theme);
            below_top += empty_list_height(theme);
        }
        let (note, look) = texts
            .note
            .note(catalog, generation, (revision, false), |writer, _| {
                let look = below_note(
                    writer,
                    enabled,
                    &self.missing[index],
                    self.fields[index].is_set(),
                    self.inherited[index].as_ref(),
                );
                followed_by(writer, look, |writer| self.write_extra(writer, setting))
            });
        if !note.is_empty() {
            let below = Rect::new(
                wide.x,
                below_top + theme.gap * 0.75,
                (wide.width - buttons_width(theme) - theme.gap).max(0.0),
                theme.icon_size,
            );
            let note_height = draw_note(context, below, note, look);
            row += theme.gap * 0.75 + note_height;
        }
        row
    }
}

#[cfg(test)]
mod tests {
    use machine_factory::config::settings::find_setting;

    use super::*;
    use crate::{
        editor::{
            EditorSection,
            inherited::InheritedSource,
            notes::tests::{Note, english_note},
            rows::{RowAreas, RowTexts},
            test_support::{EditorRoot, SCREEN, fdd1_model, index_of},
        },
        gui_test_support::GuiHarness,
    };

    /// Verifies that the warning of missing images wins over the inherited list, that only a set list shows the inherited list and that a disabled list shows nothing.
    #[test]
    fn the_note_below_a_list_warns_first_and_shows_the_inherited_list_of_a_set_list() {
        let inherited = Inherited {
            values: vec!["games/boot.d88".to_owned()],
            source: InheritedSource::SettingsFile,
        };
        let below_note = |enabled: bool, missing: &[bool], set: bool| {
            english_note(|writer| below_note(writer, enabled, missing, set, Some(&inherited)))
        };
        let warning = below_note(true, &[false, false, true], true);
        assert!(warning.look.is_warning());
        assert!(warning.text.starts_with("Image 3 does not exist."));
        assert_eq!(
            below_note(true, &[], true),
            Note::plain("Settings default: games/boot.d88.").shown_as_path()
        );
        assert_eq!(below_note(true, &[], false), Note::default());
        assert_eq!(below_note(false, &[true], true), Note::default());
    }

    /// Verifies that the row of a list that inherits images holds the checkbox, that a set list grows by the note of the inherited list, and that an empty list holds both.
    #[test]
    fn a_set_list_over_an_inherited_list_grows_by_its_note() {
        let root = EditorRoot::new("media_row");
        root.write("neetan.conf", "fdd1 = games/boot.d88\n");
        root.write("games/own.d88", "");
        let fdd1 = find_setting("fdd1").expect("fdd1 is a table key");
        let model = fdd1_model(&root);
        let height = |contents: &str| {
            let path = root.game("edge", &format!("machine = {model}\n{contents}"));
            let mut editor =
                Editor::open(&root.host(), root.system(), &path).expect("the editor opens");
            let index = index_of(&editor, "fdd1");
            assert_eq!(editor.fields[index].section, EditorSection::Media);
            let mut harness = GuiHarness::new(960, 600);
            harness.frame(|context| {
                let mut actions = PageActions::default();
                let mut texts = RowTexts::default();
                let row = SettingRow {
                    areas: RowAreas::new(context.theme, SCREEN, 0.0),
                    index,
                    setting: fdd1,
                    ignored: None,
                    texts: &mut texts,
                };
                editor.draw_media(context, row, &mut actions)
            })
        };
        let inherited = height("");
        let own = height("fdd1 = games/own.d88\n");
        let empty = height("fdd1 =\n");
        let theme = GuiHarness::new(960, 600).theme;
        let plain = inherited - empty_list_height(&theme);
        let note = own - plain;
        assert!(note > 0.0, "{own} {plain}");
        assert!((empty - (plain + empty_list_height(&theme) + note)).abs() < 0.01);
    }
}
