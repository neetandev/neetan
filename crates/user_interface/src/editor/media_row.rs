//! The form row of a path list: the media list of one drive, the checkbox
//! that empties the drive over its inherited list, and the note below the
//! list.

use gui::{
    Rect,
    ui::{Truncation, UiContext},
};
use machine_factory::config::settings::Setting;

use super::{
    BrowseTarget, Editor,
    fields::FieldValue,
    hints::{EditorWidget, WidgetKind},
    inherited::Inherited,
    media_list::{
        MediaList, button_ids, buttons_width, empty_list_height, empty_list_id, list_height,
        missing_note,
    },
    notes::{Note, empty_drive_hint, empty_list_caption, media_note},
    page::PageActions,
    rows::{RowAreas, draw_note, field_id, image_icon},
};

/// Returns the note below a media list: the warning of the images at the
/// 0-based `missing` indices, or else the inherited list of a list that the
/// file sets. A disabled list shows no note.
pub(crate) fn below_note(
    enabled: bool,
    missing: &[usize],
    set: bool,
    inherited: Option<&Inherited>,
) -> Note {
    if !enabled {
        return Note::default();
    }
    match missing_note(missing) {
        Some(text) => Note::warning(text),
        None => media_note(set, inherited),
    }
}

impl Editor {
    /// Draws the media list of the path list field at `index` and returns
    /// the height of its row.
    pub(super) fn draw_media(
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
        let enabled = ignored.is_none();
        let types = self.file_types(setting);
        let field = &self.fields[index];
        let has_images = !field.images().is_empty();
        let empty_list = field.is_empty_list();
        let set = field.is_set();
        let inherited = self.inherited[index].clone();
        let inherits_images = inherited
            .as_ref()
            .is_some_and(|inherited| !inherited.values.is_empty());
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
        let hint = match &ignored {
            Some(note) => note.clone(),
            None => empty_drive_hint(types, inherited.as_ref(), empty_list),
        };
        let hint_truncation = if ignored.is_none() && inherits_images && !empty_list {
            Truncation::Middle
        } else {
            Truncation::End
        };
        let caption = empty_list_caption(types);
        let missing: Vec<usize> = self.missing[index]
            .iter()
            .enumerate()
            .filter(|(_, missing)| **missing)
            .map(|(place, _)| place)
            .collect();
        let mut values = self.fields[index].images().to_vec();
        let height = list_height(theme, values.len());
        let wide = Rect::new(areas.wide.x, areas.wide.y, areas.wide.width, height);
        let response = MediaList {
            id,
            icon: image_icon(types),
            missing: &self.missing[index],
            enabled,
            hint: &hint,
            hint_truncation,
            empty_list_caption: shows_toggle.then_some(caption.as_str()),
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
        let note = below_note(enabled, &missing, set, inherited.as_ref())
            .followed_by(self.extra_note(setting));
        if !note.text.is_empty() {
            let below = Rect::new(
                wide.x,
                below_top + theme.gap * 0.75,
                (wide.width - buttons_width(theme) - theme.gap).max(0.0),
                theme.icon_size,
            );
            let note_height = draw_note(context, below, &note);
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
            notes::MISSING_IMAGE_NOTE,
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
        let warning = below_note(true, &[2], true, Some(&inherited));
        assert!(warning.is_warning());
        assert!(warning.text.starts_with("Image 3 does not exist."));
        assert_ne!(warning.text, MISSING_IMAGE_NOTE);
        assert_eq!(
            below_note(true, &[], true, Some(&inherited)),
            Note::plain("Settings default: games/boot.d88.").shown_as_path()
        );
        assert_eq!(
            below_note(true, &[], false, Some(&inherited)),
            Note::default()
        );
        assert_eq!(
            below_note(false, &[0], true, Some(&inherited)),
            Note::default()
        );
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
                let areas = RowAreas::new(context.theme, SCREEN, 0.0);
                let mut actions = PageActions::default();
                editor.draw_media(context, areas, index, fdd1, None, &mut actions)
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
