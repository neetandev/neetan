//! The form row of the single image of a drive, such as `hdd1`: the path
//! field with its Browse button across the box of a media list, the
//! checkbox that empties the drive over its inherited image, and the note
//! below them.

use gui::{
    Rect,
    ui::{Checkbox, UiContext},
};

use super::{
    Editor,
    fields::FieldValue,
    hints::{EditorWidget, WidgetKind},
    media_list::{empty_list_height, empty_list_id, list_box_width},
    media_row::{draw_note_below, shows_empty_toggle},
    notes::write_empty_list_caption,
    page::PageActions,
    rows::{FORM_ROW_PITCH, SettingRow, field_id},
};

impl Editor {
    /// Draws the image row of the single image field of `row` and returns
    /// the height of the row. The path field and the Browse button span the
    /// width of the box of a media list. The checkbox that empties the
    /// drive shows below them for an enabled drive without a path that
    /// inherits an image, that the file empties or whose checkbox holds the
    /// focus. The note takes its own lines below the field and the
    /// checkbox, with the space after it of the note below a media list.
    pub(super) fn draw_image(
        &mut self,
        context: &mut UiContext,
        mut row: SettingRow,
        actions: &mut PageActions,
    ) -> f32 {
        let theme = context.theme;
        let catalog = context.catalog;
        let generation = context.catalog_generation;
        let revision = self.revision;
        let index = row.index;
        let setting = row.setting;
        let ignored = row.ignored;
        let top = row.areas.top;
        let wide = row.areas.wide;
        let empty_id = empty_list_id(field_id(setting.key));
        let empty_path = self.fields[index].is_empty_path();
        let shows_toggle = shows_empty_toggle(
            context,
            empty_id,
            ignored.is_none(),
            !self.fields[index].text().trim().is_empty(),
            empty_path,
            self.inherited[index].as_ref(),
        );
        let control = Rect::new(
            wide.x,
            wide.y,
            list_box_width(theme, wide.width),
            theme.control_height,
        );
        self.draw_path_field(context, control, &mut row, false, actions);
        let texts = row.texts;
        let mut bottom = top + FORM_ROW_PITCH;
        let mut below_top = control.bottom();
        if shows_toggle {
            self.widgets
                .push(EditorWidget::new(empty_id, WidgetKind::Checkbox));
            let area = Rect::new(
                control.x,
                control.bottom() + theme.gap * 0.5,
                control.width,
                theme.control_height,
            );
            let types = self.file_types(setting);
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
            bottom += empty_list_height(theme);
            below_top += empty_list_height(theme);
        }
        let note = texts
            .note
            .note(catalog, generation, (revision, false), |writer, _| {
                self.write_path_note(writer, index, setting, ignored)
            });
        draw_note_below(context, control.x, below_top, control.width, note, bottom) - top
    }
}

#[cfg(test)]
mod tests {
    use gui::{
        Color, Instance,
        ui::{
            Button, Icon, Navigation, TextField, Truncation, UiId, WrappedText, wrapped_line_height,
        },
    };
    use machine_factory::config::{
        MachineSelection,
        settings::{MODEL_CHOICES, Setting, ValueKind},
        systems::compiled_systems,
    };

    use super::*;
    use crate::{
        editor::{
            EditorSection,
            fields::is_single_image,
            form::{ignored_by, write_ignored, write_models_only},
            inherited::{Inherited, InheritedSource},
            notes::{NoteKind, NoteLook, path_note, tests::catalog_note},
            rows::{NOTE_MAXIMUM_LINES, RowAreas, browse_id},
            test_support::{EditorRoot, index_of, row_of, show_in},
        },
        gui_test_support::{GuiHarness, MINIMUM_SCREEN},
        texts::{
            common, english, note,
            repository::{repository_catalogs, repository_languages},
        },
    };

    /// The screen of a large window.
    const LARGE_SCREEN: Rect = Rect::new(0.0, 0.0, 1920.0, 1080.0);

    /// A path longer than the path field of a path row of the control column.
    const LONG_PATH: &str = "games/A folder with the long name of a game series/The long \
                             name of one game of the series/The hard disk image of the game.img";

    /// The single image settings of the form of the family of `root`.
    fn image_settings(root: &EditorRoot) -> Vec<&'static Setting> {
        let path = root.game("probe", "");
        let editor = Editor::open(&root.host(), root.system(), &path).expect("the editor opens");
        editor
            .fields
            .iter()
            .filter_map(|field| field.source.setting())
            .filter(|setting| is_single_image(setting))
            .collect()
    }

    /// Opens the Media section of the game editor of the first model of the family of `root`
    /// that reads `setting`, for a file with the lines `lines`, and draws it in `screen`. A
    /// family without such a model gives `None`.
    fn image_section(
        root: &EditorRoot,
        harness: &mut GuiHarness,
        screen: Rect,
        setting: &Setting,
        lines: &str,
    ) -> Option<Editor> {
        let target = root.target();
        let model = MODEL_CHOICES
            .iter()
            .filter(|choice| choice.families.contains_target(target))
            .map(|choice| choice.value)
            .find(|model| setting.is_read_by(target, model))?;
        let path = root.game(
            "image",
            &format!("meta.title = Image\nmachine = {model}\n{lines}"),
        );
        let mut editor =
            Editor::open(&root.host(), root.system(), &path).expect("the editor opens");
        show_in(harness, &mut editor, screen);
        while editor.current_section() != EditorSection::Media {
            harness.input.navigation = Some(Navigation::NextTab);
            show_in(harness, &mut editor, screen);
        }
        Some(editor)
    }

    /// The rectangles of the last pass of `harness` with the fill `color` whose tops lie in
    /// `row`.
    fn rectangles_in(harness: &GuiHarness, color: Color, row: Rect) -> Vec<Rect> {
        let color = color.to_linear();
        harness
            .scene
            .instances()
            .iter()
            .filter(|instance| {
                instance.kind[0] == Instance::KIND_RECTANGLE
                    && instance.color == color
                    && instance.rect[1] >= row.y
                    && instance.rect[1] < row.bottom()
            })
            .map(|instance| {
                let [x, y, width, height] = instance.rect;
                Rect::new(x, y, width, height)
            })
            .collect()
    }

    /// The path field of the row of `key` in the last pass of `editor`.
    fn path_field(harness: &GuiHarness, editor: &Editor, key: &str) -> Rect {
        let fields: Vec<Rect> = rectangles_in(
            harness,
            harness.theme.colors.field,
            row_of(editor, key).rect,
        )
        .into_iter()
        .filter(|rect| rect.height == harness.theme.control_height)
        .collect();
        assert_eq!(fields.len(), 1, "{key} {fields:?}");
        fields[0]
    }

    /// The key of the first media list of the Media section of `editor`. A section without a
    /// media list gives `None`.
    fn first_media_list(editor: &Editor) -> Option<&'static str> {
        editor
            .fields
            .iter()
            .filter(|field| field.section == EditorSection::Media)
            .filter_map(|field| field.source.setting())
            .find(|setting| setting.kind == ValueKind::PathList)
            .map(|setting| setting.key)
    }

    /// The box of the first media list of the last pass of `editor`. A section without a
    /// media list gives `None`.
    fn list_box(harness: &GuiHarness, editor: &Editor) -> Option<Rect> {
        let row = row_of(editor, first_media_list(editor)?).rect;
        rectangles_in(harness, harness.theme.colors.well, row)
            .into_iter()
            .find(|rect| rect.height > harness.theme.control_height)
    }

    /// The key of the first media list of the Media section of the form of the family of
    /// `root`. A section without a media list gives `None`.
    fn media_list_key(root: &EditorRoot) -> Option<&'static str> {
        let path = root.game("probe", "");
        let editor = Editor::open(&root.host(), root.system(), &path).expect("the editor opens");
        first_media_list(&editor)
    }

    /// The width of the Browse button of `key` in `harness`.
    fn browse_width(harness: &mut GuiHarness, key: &str) -> f32 {
        harness.frame(|context| {
            Button::new(browse_id(key), context.catalog.text(common::BROWSE)).width(context)
        })
    }

    /// The glyphs of the last pass of `harness` with their centers inside `area`.
    fn glyphs_in(harness: &GuiHarness, area: Rect) -> Vec<Rect> {
        harness
            .scene
            .instances()
            .iter()
            .filter(|instance| instance.kind[0] == Instance::KIND_GLYPH)
            .map(|instance| {
                let [x, y, width, height] = instance.rect;
                Rect::new(x, y, width, height)
            })
            .filter(|glyph| {
                let (x, y) = glyph.center();
                area.contains(x, y)
            })
            .collect()
    }

    /// The glyphs of the last pass of `harness` in the color `color` with their centers inside
    /// `area`.
    fn colored_glyphs_in(harness: &GuiHarness, color: Color, area: Rect) -> Vec<Rect> {
        let color = color.to_linear();
        harness
            .scene
            .instances()
            .iter()
            .filter(|instance| instance.kind[0] == Instance::KIND_GLYPH && instance.color == color)
            .map(|instance| {
                let [x, y, width, height] = instance.rect;
                Rect::new(x, y, width, height)
            })
            .filter(|glyph| {
                let (x, y) = glyph.center();
                area.contains(x, y)
            })
            .collect()
    }

    /// Verifies that the path field of every single image of every compiled system starts at
    /// the left edge of the box of the media lists and that the field with the Browse button
    /// after it ends at the right edge of the box, at the minimum window size and in a large
    /// window, in every language of the repository. Each language and window size checks at
    /// least one system when the build compiles a system with a media list and a single image.
    #[test]
    fn the_image_field_spans_the_box_of_the_media_lists() {
        let mut harnesses = [
            (GuiHarness::minimum(), MINIMUM_SCREEN),
            (
                GuiHarness::new(LARGE_SCREEN.width as u32, LARGE_SCREEN.height as u32),
                LARGE_SCREEN,
            ),
        ];
        let expects_check = compiled_systems().any(|system| {
            let root = EditorRoot::with_system("image_row_span_systems", system.name);
            !image_settings(&root).is_empty() && media_list_key(&root).is_some()
        });
        for language in repository_languages() {
            for (harness, screen) in &mut harnesses {
                let catalog = language.catalog();
                let tag = catalog.catalog().tag().to_owned();
                harness.set_catalog(catalog);
                let mut checked = 0;
                for system in compiled_systems() {
                    let root = EditorRoot::with_system("image_row_span", system.name);
                    for setting in image_settings(&root) {
                        let Some(editor) = image_section(&root, harness, *screen, setting, "")
                        else {
                            continue;
                        };
                        let Some(list_box) = list_box(harness, &editor) else {
                            continue;
                        };
                        let key = setting.key;
                        let field = path_field(harness, &editor, key);
                        let browse = browse_width(harness, key);
                        let gap = harness.theme.gap;
                        assert!(
                            (field.x - list_box.x).abs() < 0.5,
                            "{tag} {} {key} {field:?} {list_box:?}",
                            system.name
                        );
                        assert!(
                            (field.right() + gap + browse - list_box.right()).abs() < 0.5,
                            "{tag} {} {key} {field:?} {browse} {list_box:?}",
                            system.name
                        );
                        checked += 1;
                    }
                }
                if expects_check {
                    assert!(checked > 0, "{tag} {screen:?}");
                }
                if cfg!(feature = "pc98") {
                    assert!(checked >= 2, "{tag} {screen:?} {checked}");
                }
            }
        }
    }

    /// Verifies that the warning of a missing image of every single image of every compiled
    /// system draws below the path field across the width of the field and the button, and
    /// that the row grows by the note.
    #[test]
    fn the_note_of_an_image_row_shows_below_its_field() {
        let mut harness = GuiHarness::minimum();
        let mut checked = 0;
        for system in compiled_systems() {
            let root = EditorRoot::with_system("image_row_note", system.name);
            for setting in image_settings(&root) {
                let key = setting.key;
                let lines = format!("{key} = games/missing.img\n");
                let Some(editor) =
                    image_section(&root, &mut harness, MINIMUM_SCREEN, setting, &lines)
                else {
                    continue;
                };
                assert_eq!(
                    editor.cached_note(key),
                    Some(english().text(note::MISSING_IMAGE)),
                    "{} {key}",
                    system.name
                );
                let row = row_of(&editor, key).rect;
                let field = path_field(&harness, &editor, key);
                let warning = colored_glyphs_in(&harness, harness.theme.colors.danger_text, row);
                assert!(!warning.is_empty(), "{} {key}", system.name);
                let browse = browse_width(&mut harness, key);
                let span = Rect::new(
                    field.x,
                    field.bottom(),
                    field.width + harness.theme.gap + browse,
                    row.bottom() - field.bottom(),
                );
                for glyph in &warning {
                    assert!(
                        glyph.y >= span.y
                            && glyph.bottom() <= span.bottom()
                            && glyph.x >= span.x - 0.5
                            && glyph.right() <= span.right() + 0.5,
                        "{} {key} {glyph:?} {span:?}",
                        system.name
                    );
                }
                assert!(row.height > FORM_ROW_PITCH, "{} {key} {row:?}", system.name);
                checked += 1;
            }
        }
        if cfg!(feature = "pc98") {
            assert!(checked >= 2, "{checked}");
        }
    }

    /// Verifies that an image row of an empty drive over an inherited image holds the checkbox
    /// below the field and the note of the inherited image below the checkbox: the caption lies
    /// in the area of the checkbox below the field, and the note lies below the caption. The
    /// note ends as far above the bottom of its row as the same note below a media list. The
    /// large window shows every row of the section.
    #[test]
    fn the_checkbox_of_an_image_row_shows_above_its_note() {
        let mut harness = GuiHarness::new(LARGE_SCREEN.width as u32, LARGE_SCREEN.height as u32);
        let mut checked = 0;
        let mut compared = 0;
        for system in compiled_systems() {
            let root = EditorRoot::with_system("image_row_checkbox", system.name);
            let list_key = media_list_key(&root);
            root.write("games/default.img", "");
            for setting in image_settings(&root) {
                let key = setting.key;
                let mut defaults = format!("{key} = games/default.img\n");
                let mut lines = format!("{key} =\n");
                if let Some(list_key) = list_key {
                    defaults.push_str(&format!("{list_key} = games/default.img\n"));
                    lines.push_str(&format!("{list_key} = games/default.img\n"));
                }
                root.write(&format!("config/{}.conf", system.name), &defaults);
                let Some(editor) =
                    image_section(&root, &mut harness, LARGE_SCREEN, setting, &lines)
                else {
                    continue;
                };
                assert_eq!(
                    editor.cached_note(key),
                    Some("System default: games/default.img."),
                    "{} {key}",
                    system.name
                );
                let theme = &harness.theme;
                let row = row_of(&editor, key).rect;
                let field = path_field(&harness, &editor, key);
                let below = Rect::new(
                    field.x,
                    field.bottom(),
                    row.right() - field.x,
                    row.bottom() - field.bottom(),
                );
                let caption = colored_glyphs_in(&harness, theme.colors.text, below);
                let note = colored_glyphs_in(&harness, theme.colors.text_dim, below);
                assert!(!caption.is_empty(), "{} {key}", system.name);
                assert!(!note.is_empty(), "{} {key}", system.name);
                let caption_top = caption.iter().map(|glyph| glyph.y).fold(f32::MAX, f32::min);
                let caption_bottom = caption
                    .iter()
                    .map(|glyph| glyph.bottom())
                    .fold(f32::MIN, f32::max);
                let note_top = note.iter().map(|glyph| glyph.y).fold(f32::MAX, f32::min);
                let checkbox_top = field.bottom() + theme.gap * 0.5;
                assert!(
                    caption_top >= checkbox_top
                        && caption_bottom <= checkbox_top + theme.control_height,
                    "{} {key} {caption_top} {caption_bottom} {field:?}",
                    system.name
                );
                assert!(
                    note_top >= caption_bottom,
                    "{} {key} {note_top} {caption_bottom}",
                    system.name
                );
                checked += 1;
                let Some(list_key) = list_key else {
                    continue;
                };
                let Some(list_box) = list_box(&harness, &editor) else {
                    continue;
                };
                if editor.cached_note(list_key) != editor.cached_note(key) {
                    continue;
                }
                let list_row = row_of(&editor, list_key).rect;
                let list_below = Rect::new(
                    list_box.x,
                    list_box.bottom(),
                    list_box.width,
                    list_row.bottom() - list_box.bottom(),
                );
                let list_note = colored_glyphs_in(&harness, theme.colors.text_dim, list_below);
                assert!(!list_note.is_empty(), "{} {list_key}", system.name);
                let space_below = |row: Rect, glyphs: &[Rect]| {
                    row.bottom()
                        - glyphs
                            .iter()
                            .map(|glyph| glyph.bottom())
                            .fold(f32::MIN, f32::max)
                };
                let image_space = space_below(row, &note);
                let list_space = space_below(list_row, &list_note);
                assert!(
                    (image_space - list_space).abs() < 0.5,
                    "{} {key} {image_space} {list_key} {list_space}",
                    system.name
                );
                compared += 1;
            }
        }
        if cfg!(feature = "pc98") {
            assert!(checked >= 2, "{checked}");
            assert!(compared >= 2, "{compared}");
        }
    }

    /// Verifies that the note of an image row gives no extensions of the files, for an unset
    /// drive and for a set drive, in every compiled system.
    #[test]
    fn an_image_row_gives_no_extensions() {
        let mut harness = GuiHarness::minimum();
        let mut checked = 0;
        for system in compiled_systems() {
            let root = EditorRoot::with_system("image_row_extensions", system.name);
            for setting in image_settings(&root) {
                let key = setting.key;
                root.write(&format!("games/{key}.img"), "");
                for lines in [String::new(), format!("{key} = games/{key}.img\n")] {
                    let Some(editor) =
                        image_section(&root, &mut harness, MINIMUM_SCREEN, setting, &lines)
                    else {
                        continue;
                    };
                    assert_eq!(
                        editor.cached_note(key),
                        Some(""),
                        "{} {key} {lines:?}",
                        system.name
                    );
                    let index = index_of(&editor, key);
                    assert!(editor.fields[index].is_set() == !lines.is_empty());
                    checked += 1;
                }
            }
        }
        if cfg!(feature = "pc98") {
            assert!(checked >= 2, "{checked}");
        }
    }

    /// Verifies that the path field of an image row shows more glyphs of a long path than the
    /// path field of a path row in the control column, at the minimum window size.
    #[test]
    fn a_long_path_shows_more_of_its_text_in_an_image_row() {
        let mut harness = GuiHarness::minimum();
        let mut checked = 0;
        for system in compiled_systems() {
            let root = EditorRoot::with_system("image_row_long_path", system.name);
            for setting in image_settings(&root) {
                let key = setting.key;
                root.write(LONG_PATH, "");
                let lines = format!("{key} = {LONG_PATH}\n");
                let Some(editor) =
                    image_section(&root, &mut harness, MINIMUM_SCREEN, setting, &lines)
                else {
                    continue;
                };
                let field = path_field(&harness, &editor, key);
                let shown = glyphs_in(&harness, field).len();
                let row = row_of(&editor, key).rect;
                let control = RowAreas::new(&harness.theme, row, row.y).control;
                let browse = browse_width(&mut harness, key);
                let gap = harness.theme.gap;
                let narrow = Rect::new(
                    control.x,
                    control.y,
                    control.width - browse - gap,
                    control.height,
                );
                let mut text = LONG_PATH.to_owned();
                harness.frame(|context| {
                    TextField::new(UiId::from("narrow"), "")
                        .icon(Some(Icon::Folder))
                        .clears_on_cancel(false)
                        .truncation(Truncation::Middle)
                        .show(context, narrow, &mut text);
                });
                let narrow_shown = glyphs_in(&harness, narrow).len();
                assert!(
                    shown > narrow_shown + 10,
                    "{} {key} {shown} {narrow_shown}",
                    system.name
                );
                checked += 1;
            }
        }
        if cfg!(feature = "pc98") {
            assert!(checked >= 2, "{checked}");
        }
    }

    /// Verifies that every note below an image row fits its width at the minimum window size
    /// within [`NOTE_MAXIMUM_LINES`] lines in every language of the repository: the reason that
    /// each model of each compiled family ignores the drive, the warning of a missing image,
    /// and the inherited image of a set drive with the models of the system defaults editor.
    #[test]
    fn every_note_below_an_image_row_fits_at_the_minimum_window_size() {
        let mut harness = GuiHarness::minimum();
        let mut width = None;
        let mut notes = Vec::new();
        for system in compiled_systems() {
            let root = EditorRoot::with_system("image_row_fit", system.name);
            let target = root.target();
            for setting in image_settings(&root) {
                if width.is_none()
                    && let Some(editor) =
                        image_section(&root, &mut harness, MINIMUM_SCREEN, setting, "")
                {
                    let row = row_of(&editor, setting.key).rect;
                    let wide = RowAreas::new(&harness.theme, row, row.y).wide;
                    width = Some(list_box_width(&harness.theme, wide.width));
                }
                let inherited = Inherited::new(
                    vec!["games/system/MS-DOS 6.2.hdi".to_owned()],
                    InheritedSource::SettingsFile,
                );
                for catalog in repository_catalogs() {
                    for choice in MODEL_CHOICES
                        .iter()
                        .filter(|choice| choice.families.contains_target(target))
                    {
                        let model: MachineSelection = choice.value.parse().expect("a model");
                        if let Some(ignored) = ignored_by(setting, model) {
                            notes.push(catalog_note(catalog, |writer| {
                                write_ignored(writer, ignored);
                                NoteLook::PLAIN
                            }));
                        }
                    }
                    notes.push(catalog_note(catalog, |writer| {
                        path_note(writer, None, true, true, Some(&inherited))
                    }));
                    if let Some(limit) = setting.limit_for(target) {
                        notes.push(catalog_note(catalog, |writer| {
                            let look = path_note(writer, None, false, true, Some(&inherited));
                            write_models_only(writer, limit);
                            NoteLook {
                                path: false,
                                ..look
                            }
                        }));
                    }
                }
            }
        }
        let Some(width) = width else {
            return;
        };
        let theme = &harness.theme;
        let marked_width = width - theme.icon_size - theme.gap * 0.75;
        let lines = harness.frame(|context| {
            let style = context.theme.small();
            let line = wrapped_line_height(&style);
            notes
                .iter()
                .filter(|note| !note.look.path)
                .map(|note| {
                    let width = match note.look.kind {
                        NoteKind::Plain => width,
                        NoteKind::Warning | NoteKind::Found | NoteKind::Unmarked => marked_width,
                    };
                    let height =
                        WrappedText::new().height(context, &style, &note.text, width, usize::MAX);
                    (note.text.clone(), (height / line).round() as usize)
                })
                .collect::<Vec<_>>()
        });
        assert!(!lines.is_empty());
        for (note, count) in lines {
            assert!(count <= NOTE_MAXIMUM_LINES, "{note} takes {count} lines");
        }
    }
}
