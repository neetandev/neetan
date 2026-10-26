//! The parts of the form rows of the editor: the areas of a row, the label
//! with its key, the ids of the controls and the note after the control.

use gui::{
    Rect,
    ui::{
        Align, Dropdown, Icon, TextBuffer, Theme, Truncation, UiContext, UiId, WrappedText, label,
    },
};
use machine_factory::config::settings::{
    Setting,
    file_types::{FileTypes, ImageIcon},
};

#[cfg(test)]
use super::{
    Editor,
    form::EditorSection,
    notes::{NoteLook, tests::english_note},
};
use super::{
    form::IgnoredBy,
    note_writer::{NoteText, NoteView},
    notes::NoteKind,
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
        let note = english_note(|writer| {
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
            notes::{ChoiceNote, NoteLook, choice_note, tests::catalog_note},
            page::form_area,
        },
        gui_test_support::{GuiHarness, MINIMUM_SCREEN},
        texts::{note, repository::repository_catalogs},
    };

    /// The width of the note column of a window of the minimum size.
    fn minimum_note_width(harness: &GuiHarness) -> f32 {
        let theme = &harness.theme;
        let form = form_area(theme, MINIMUM_SCREEN);
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

    /// Asserts that every note of `notes` fits the note column of a window of the minimum size within
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
    /// window of the minimum size within its lines in every language of the repository.
    #[test]
    fn every_model_limit_note_fits_at_the_minimum_window_size() {
        let mut harness = GuiHarness::minimum();
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

    /// Verifies that the note of a choice with a fact fits the note column of a window of the
    /// minimum size within its lines in every language of the repository, for every model of every
    /// compiled family. The note holds the longest sentences before the fact: the inherited
    /// value of each source and the derived default. The models that the system defaults
    /// editor lists follow the fact. A clock note holds both clocks of the model.
    #[test]
    fn every_choice_fact_note_fits_at_the_minimum_window_size() {
        let mut harness = GuiHarness::minimum();
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
                                let sentence =
                                    catalog.display(note::cpu_clock(low, high)).to_string();
                                assert!(note.text.contains(&sentence), "{}", note.text);
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
        let mut harness = GuiHarness::minimum();
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
