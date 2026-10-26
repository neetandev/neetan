//! The form row of a path list: the media list of one drive, the checkbox
//! that empties the drive over its inherited list, and the note below the
//! list.

use gui::{
    Rect,
    ui::{Truncation, UiContext, UiId},
};

use super::{
    BrowseTarget, Editor,
    fields::FieldValue,
    form::write_ignored,
    hints::{EditorWidget, WidgetKind},
    inherited::Inherited,
    media_list::{
        MediaList, button_ids, empty_list_height, empty_list_id, list_box_width, list_height,
        write_missing_note,
    },
    note_writer::{NoteView, NoteWriter},
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

/// Whether the checkbox that empties a drive shows below its control: for
/// an enabled drive without an own image that the file empties, that
/// inherits an image, or whose checkbox `empty_id` holds the focus.
pub(crate) fn shows_empty_toggle(
    context: &UiContext,
    empty_id: UiId,
    enabled: bool,
    has_image: bool,
    emptied: bool,
    inherited: Option<&Inherited>,
) -> bool {
    let inherits_images = inherited.is_some_and(|inherited| !inherited.values.is_empty());
    enabled && !has_image && (emptied || inherits_images || context.has_focus(empty_id))
}

/// Draws `note` on its own lines below a control of the media section,
/// a short gap below `top`, across `width` from `x`. Returns the bottom of
/// the row: a gap below the last line of the note, or `bottom` for an empty
/// note.
pub(crate) fn draw_note_below(
    context: &mut UiContext,
    x: f32,
    top: f32,
    width: f32,
    note: NoteView,
    bottom: f32,
) -> f32 {
    if note.text.is_empty() {
        return bottom;
    }
    let theme = context.theme;
    let area = Rect::new(x, top + theme.gap * 0.75, width, theme.icon_size);
    area.y + draw_note(context, area, note) + theme.gap
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
        let shows_toggle = shows_empty_toggle(
            context,
            empty_list_id(id),
            enabled,
            has_images,
            empty_list,
            inherited,
        );
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
        let height = list_height(theme, self.fields[index].images().len());
        let wide = Rect::new(areas.wide.x, areas.wide.y, areas.wide.width, height);
        let media_list = MediaList {
            id,
            icon: image_icon(types),
            missing: &self.missing[index],
            enabled,
            hint,
            hint_truncation,
            empty_list_caption: shows_toggle.then_some(caption),
            empty_list,
        };
        let field = &mut self.fields[index];
        let response = match &mut field.value {
            FieldValue::List(values) => media_list.show(context, wide, &mut field.list, values),
            FieldValue::Choice(_)
            | FieldValue::Text(_)
            | FieldValue::EmptyList
            | FieldValue::EmptyPath => {
                let mut values = Vec::new();
                let response = media_list.show(context, wide, &mut field.list, &mut values);
                if response.changed {
                    field.value = FieldValue::List(values);
                }
                response
            }
        };
        if response.changed {
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
        let mut below_top = wide.bottom();
        if shows_toggle {
            below_top += empty_list_height(theme);
        }
        let note = texts
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
        let bottom = draw_note_below(
            context,
            wide.x,
            below_top,
            list_box_width(theme, wide.width),
            note,
            below_top + theme.gap,
        );
        bottom - areas.top
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
            test_support::{EditorRoot, fdd1_model, index_of},
        },
        gui_test_support::{GuiHarness, MINIMUM_SCREEN},
    };

    /// Verifies that the warning of missing images wins over the inherited list, that only a set list shows the inherited list and that a disabled list shows nothing.
    #[test]
    fn the_note_below_a_list_warns_first_and_shows_the_inherited_list_of_a_set_list() {
        let inherited = Inherited::new(
            vec!["games/boot.d88".to_owned()],
            InheritedSource::SettingsFile,
        );
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
            let mut harness = GuiHarness::minimum();
            harness.frame(|context| {
                let mut actions = PageActions::default();
                let mut texts = RowTexts::default();
                let row = SettingRow {
                    areas: RowAreas::new(context.theme, MINIMUM_SCREEN, 0.0),
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
        let theme = GuiHarness::minimum().theme;
        let plain = inherited - empty_list_height(&theme);
        let note = own - plain;
        assert!(note > 0.0, "{own} {plain}");
        assert!((empty - (plain + empty_list_height(&theme) + note)).abs() < 0.01);
    }
}
