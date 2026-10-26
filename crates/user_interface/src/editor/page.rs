//! The page of the editor: the header, the tab bar of the sections, the
//! scroll area of the form and the footer with its buttons.

use gui::{
    Rect,
    ui::{
        ActionButton, Align, Button, ButtonKind, CLOSE_SIZE, HEADER_HEIGHT, Icon, ScrollState,
        Shortcut, Tab, TextBuffer, Theme, Truncation, UiContext, UiId, WrappedText, scroll_area,
        tab_bar,
    },
};
use i18n::Catalog;

use super::{
    BrowseTarget, EditedFile, Editor, EditorFile, Subject,
    form::EDITOR_SECTIONS,
    help_area::help_area_height,
    hints::{EditorWidget, WidgetKind},
    note_writer::NoteText,
    notes::NoteKind,
    rows::{DOT_SIZE, draw_note_text},
    save::validation_error,
};
use crate::{
    media_dialog::TAB_BAR_HEIGHT,
    texts::{common, editor},
};

/// The widest that the tab bar, the form and the footer grow. They center in a wider window.
pub(crate) const FORM_MAX_WIDTH: f32 = 1280.0;

/// The id of the close button.
pub(crate) const CLOSE_ID: &str = "editor close";

/// The id of the tab bar.
pub(crate) const TABS_ID: &str = "editor sections";

/// The id of the scroll area of the form.
const FORM_ID: &str = "editor form";

/// The id of the cancel button.
pub(crate) const CANCEL_ID: &str = "editor cancel";

/// The id of the save button.
pub(crate) const SAVE_ID: &str = "editor save";

/// The most sections of a page.
const MAXIMUM_SECTIONS: usize = EDITOR_SECTIONS.len();

/// The part of the header that the title keeps at least beside the line
/// after it.
const TITLE_SHARE: f32 = 0.5;

/// The part of the header that the title keeps at least beside a line with
/// a warning.
const WARNED_TITLE_SHARE: f32 = 0.4;

/// The changes that a pass of the page asks for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct PageActions {
    /// A field changed.
    pub(super) changed: bool,
    /// The save button was activated.
    pub(super) save: bool,
    /// Ctrl+S or Start asked for a save.
    pub(super) save_shortcut: bool,
    /// The close button, the cancel button or cancel asked to close.
    pub(super) close: bool,
    /// A field asked the file browser for a path.
    pub(super) browse: Option<BrowseTarget>,
}

/// The areas of the page in `screen`: the header, the tab bar, the form, the
/// rule and the text of the help area and the footer. The tab bar, the form,
/// the help area and the footer keep inside [`FORM_MAX_WIDTH`] and center.
#[derive(Debug, Clone, Copy, PartialEq)]
struct PageAreas {
    /// The title, the line beside it and the close button.
    header: Rect,
    /// The tab bar of the sections.
    tabs: Rect,
    /// The scroll area of the form.
    form: Rect,
    /// The rule at the top of the help area across the width of the form.
    /// It separates the help text from the last visible row of the form.
    rule: Rect,
    /// The help text of the hovered or the focused row, inset to the names of
    /// the labels.
    help: Rect,
    /// The state of the changes and the buttons.
    footer: Rect,
}

impl PageAreas {
    /// The areas of the page in `screen` with the sizes of `theme`.
    fn new(theme: &Theme, screen: Rect) -> Self {
        let page = Rect::new(
            screen.x + theme.padding,
            screen.y,
            (screen.width - theme.padding * 2.0).max(0.0),
            (screen.height - theme.padding).max(0.0),
        );
        let (header, rest) = page.cut_top(HEADER_HEIGHT);
        let width = rest.width.min(FORM_MAX_WIDTH);
        let column = Rect::new(
            rest.x + (rest.width - width) * 0.5,
            rest.y,
            width,
            rest.height,
        );
        let (tabs, rest) = column.cut_top(TAB_BAR_HEIGHT);
        let (footer, rest) = rest.cut_bottom(theme.control_height);
        let rest = rest.cut_bottom(theme.gap).1;
        let (help, form) = rest.cut_bottom(help_area_height(theme));
        let help = help.cut_left(theme.gap * 1.5).1;
        let form = form.cut_top(theme.gap * 1.5).1;
        let form = form.cut_bottom(theme.gap * 1.5).1;
        let rule = Rect::new(
            form.x,
            help.y - theme.gap * 0.5 - theme.stroke,
            form.width,
            theme.stroke,
        );
        Self {
            header,
            tabs,
            form,
            rule,
            help,
            footer,
        }
    }
}

#[cfg(test)]
impl Editor {
    /// The line beside the title of the header of the last frame. An
    /// editor without a drawn header has none.
    pub(crate) fn cached_header_line(&self) -> Option<&str> {
        self.texts.line.last_text()
    }
}

/// The scroll area of the form of the page in `screen`.
#[cfg(test)]
pub(crate) fn form_area(theme: &Theme, screen: Rect) -> Rect {
    PageAreas::new(theme, screen).form
}

/// The help area of the page in `screen`.
#[cfg(test)]
pub(crate) fn help_area(theme: &Theme, screen: Rect) -> Rect {
    PageAreas::new(theme, screen).help
}

/// The texts of the page that refill only when the state of the editor or
/// the catalog changes, and the values of its read-only rows.
#[derive(Debug, Clone, Default)]
pub(crate) struct PageTexts {
    /// The title of the header.
    title: TextBuffer<u64>,
    /// The line beside the title.
    line: TextBuffer<u64>,
    /// The warning about system defaults that do not read.
    warning: TextBuffer<u64>,
    /// The validation error of the footer.
    error: TextBuffer<u64>,
    /// Whether a field holds a value that the file cannot hold, at the
    /// revision of the editor of the last check.
    invalid: Option<(u64, bool)>,
    /// The note of a section without fields or of the ROM folder of the
    /// command line, for the revision of the editor and the section.
    pub(super) section_note: NoteText<(u64, usize)>,
    /// The note of the file row of a new game.
    pub(super) file_note: TextBuffer<u64>,
    /// The value of the system row of a new game.
    pub(super) system_value: String,
    /// The value of the file row of a new game.
    pub(super) file_value: String,
    /// The value of the root row of the settings page.
    pub(super) root_value: String,
    /// The wrapped lines of the note of the system row of a new game.
    pub(super) system_lines: WrappedText,
    /// The wrapped lines of the note of the file row of a new game.
    pub(super) file_lines: WrappedText,
    /// The wrapped lines of the note of the root row of the settings page.
    pub(super) root_lines: WrappedText,
    /// The wrapped lines of the text of the settings file below the
    /// folders of the settings page.
    pub(super) settings_file_lines: WrappedText,
    /// The wrapped lines of the status of the footer: the validation error
    /// or the note of unsaved changes.
    status_lines: WrappedText,
}

/// Writes the title of the header for the file `file` of the kind `kind` in
/// `catalog`. A game file and a copy show the name of the game. A new game
/// shows its system. A system defaults file shows its system. The settings
/// file shows the name of the page.
fn write_header_title(catalog: &Catalog, kind: &EditedFile, file: &EditorFile, out: &mut String) {
    match (kind, file) {
        (EditedFile::Game { system, .. }, EditorFile::New(_)) => {
            let _ = catalog.write(out, editor::new_game_title(&system.label));
        }
        (EditedFile::Game { .. }, EditorFile::Existing(_) | EditorFile::Copy { .. })
        | (EditedFile::SystemDefaults(_) | EditedFile::Settings, _) => {
            kind.write_name(catalog, out)
        }
    }
}

/// Writes the line beside the title of the header for the file `file` of
/// the kind `kind` in `catalog`.
///
/// A game file shows its system, with the model when the model differs from
/// the model of a game without `machine`. A new game shows the file it
/// saves into. A copy shows the file it copies. A system defaults file
/// shows its system. The settings file shows its path.
fn write_header_line(
    catalog: &Catalog,
    kind: &EditedFile,
    file: &EditorFile,
    model: Option<&str>,
    shown_path: &str,
    out: &mut String,
) {
    let _ = match kind {
        EditedFile::Game { system, .. } => match file {
            EditorFile::Existing(_) => match model {
                Some(model) => catalog.write(out, editor::game_model_line(&system.label, &model)),
                None => catalog.write(out, editor::game_line(&system.label)),
            },
            EditorFile::New(_) => catalog.write(out, editor::saved_as(&shown_path)),
            EditorFile::Copy { source, .. } => catalog.write(out, editor::copy_of(source)),
        },
        EditedFile::SystemDefaults(system) => {
            catalog.write(out, editor::defaults_line(&system.label))
        }
        EditedFile::Settings => {
            out.push_str(shown_path);
            Ok(())
        }
    };
}

impl Editor {
    /// Draws the page into `screen` and returns the changes it asks for.
    pub(super) fn draw_page(&mut self, context: &mut UiContext, screen: Rect) -> PageActions {
        let theme = context.theme;
        let mut actions = PageActions::default();
        context.fill(screen, theme.colors.panel);
        let areas = PageAreas::new(theme, screen);
        if self.draw_header(context, areas.header) {
            actions.close = true;
        }

        let catalog = context.catalog;
        let sections = self.sections();
        let mut tabs = [Tab::new("", None); MAXIMUM_SECTIONS];
        for (tab, section) in tabs.iter_mut().zip(sections) {
            *tab = Tab::new(catalog.text(section.caption()), None);
        }
        let tabs_id = UiId::from(TABS_ID);
        if tab_bar(
            context,
            tabs_id,
            areas.tabs,
            &tabs[..sections.len()],
            &mut self.section,
        ) {
            self.scroll = ScrollState::new();
            self.help_tracking.leave_section();
            if !context.has_focus(tabs_id) {
                self.focus_first = true;
            }
        }
        self.widgets
            .push(EditorWidget::new(tabs_id, WidgetKind::Tabs));

        let mut scroll = std::mem::take(&mut self.scroll);
        scroll_area(
            context,
            FORM_ID,
            areas.form,
            &mut scroll,
            |context, content| self.draw_section(context, content, &mut actions),
        );
        self.scroll = scroll;

        let rule = context.snap_rect(areas.rule);
        context.fill(rule, theme.colors.panel_edge);
        self.track_help(context);
        let setting = self.help_setting(context);
        self.help.show(context, areas.help, setting);
        self.draw_footer(context, areas.footer, &mut actions);
        if context.shortcut() == Some(Shortcut::ControlS)
            || context.action_button() == Some(ActionButton::Start)
        {
            actions.save_shortcut = true;
        }
        if context.cancelled() {
            actions.close = true;
        }
        actions
    }

    /// Draws the title, the line beside it and the close button into
    /// `header`. A system defaults file that does not read adds its warning
    /// to the line, with the alert icon in the error color. Returns whether
    /// the close button was activated.
    fn draw_header(&mut self, context: &mut UiContext, header: Rect) -> bool {
        let theme = context.theme;
        let close_area = Rect::new(
            header.right() + theme.padding * 0.25 - CLOSE_SIZE,
            header.y + (header.height - CLOSE_SIZE) * 0.5,
            CLOSE_SIZE,
            CLOSE_SIZE,
        );
        let catalog = context.catalog;
        let generation = context.catalog_generation;
        let revision = self.revision;
        let mut texts = std::mem::take(&mut self.texts);
        let title = texts
            .title
            .text_in(catalog, generation, revision, |catalog, _, out| {
                write_header_title(catalog, &self.edited_file(), &self.file, out);
            });
        let small = texts
            .line
            .text_in(catalog, generation, revision, |catalog, _, out| {
                let shown_model = match &self.subject {
                    Subject::Game(files) | Subject::SystemDefaults(files) => {
                        let model = self.model_of(files);
                        (model != files.base.model().0).then(|| model.name())
                    }
                    Subject::Host(_) => None,
                };
                write_header_line(
                    catalog,
                    &self.edited_file(),
                    &self.file,
                    shown_model,
                    &self.shown,
                    out,
                );
            });
        let warning = self
            .system_files()
            .and_then(|files| files.base.warning())
            .map(|warning| {
                texts
                    .warning
                    .text_in(catalog, generation, revision, |catalog, _, out| {
                        warning.write(catalog, out);
                    })
            });
        let title_style = theme.title();
        let small_style = theme.small();
        let warning_style = theme.small().with_color(theme.colors.danger_text);
        let small_width = context.measure(&small_style, small);
        let warning_width = warning.map_or(0.0, |warning| {
            theme.gap * 1.5
                + theme.icon_size
                + theme.gap * 0.75
                + context.measure(&warning_style, warning)
        });
        let room = (close_area.x - theme.gap - header.x).max(0.0);
        let title_share = if warning.is_some() {
            WARNED_TITLE_SHARE
        } else {
            TITLE_SHARE
        };
        let title_room =
            (room - small_width - warning_width - theme.gap * 1.5).max(room * title_share);
        let title_width = context.measure(&title_style, title).min(title_room);
        let baseline = context.centered_baseline(header, &title_style);
        context.text_in(
            Rect::new(header.x, header.y, title_width, header.height),
            &title_style,
            title,
            Align::Start,
            Truncation::End,
        );
        let small_x = header.x + title_width + theme.gap * 1.5;
        let small_room = (close_area.x - theme.gap - small_x).max(0.0);
        if small_room > 0.0 {
            context
                .scene
                .push_clip(Rect::new(small_x, header.y, small_room, header.height));
            context.text(small_x, baseline, &small_style, small);
            if let Some(warning) = warning {
                let icon_x = small_x + small_width + theme.gap * 1.5;
                let icon_area = Rect::new(icon_x, header.y, theme.icon_size, header.height);
                context.icon(
                    icon_area,
                    Icon::Alert,
                    theme.icon_size,
                    theme.colors.danger_text,
                );
                let text_x = icon_area.right() + theme.gap * 0.75;
                let text_width = (small_x + small_room - text_x).max(0.0);
                context.text_in(
                    Rect::new(text_x, header.y, text_width, header.height),
                    &warning_style,
                    warning,
                    Align::Start,
                    Truncation::End,
                );
            }
            context.scene.pop_clip();
        }
        self.texts = texts;
        let close_id = UiId::from(CLOSE_ID);
        self.widgets
            .push(EditorWidget::new(close_id, WidgetKind::Close));
        Button::new(close_id, "")
            .icon(Icon::Close)
            .kind(ButtonKind::Quiet)
            .default_choice(false)
            .show(context, close_area)
    }

    /// Whether a field holds a value that the file cannot hold. The check
    /// runs again after a change of the editor.
    fn is_invalid(&mut self) -> bool {
        match self.texts.invalid {
            Some((revision, invalid)) if revision == self.revision => invalid,
            Some(_) | None => {
                let invalid = validation_error(&self.fields).is_some();
                self.texts.invalid = Some((self.revision, invalid));
                invalid
            }
        }
    }

    /// Draws the footer: the validation error or the unsaved changes, Cancel
    /// and Save.
    fn draw_footer(&mut self, context: &mut UiContext, footer: Rect, actions: &mut PageActions) {
        let theme = context.theme;
        let catalog = context.catalog;
        let dirty = self.is_dirty();
        let invalid = self.is_invalid();
        let save = Button::new(SAVE_ID, catalog.text(common::SAVE))
            .icon(Icon::Check)
            .kind(ButtonKind::Primary)
            .enabled(dirty && !invalid);
        let cancel = Button::new(CANCEL_ID, catalog.text(common::CANCEL));
        let (save_area, rest) = footer.cut_right(save.width(context));
        let cancel_area = rest
            .cut_right(theme.gap)
            .1
            .cut_right(cancel.width(context))
            .0;
        let status = Rect::new(
            footer.x,
            footer.y,
            (cancel_area.x - theme.gap - footer.x).max(0.0),
            footer.height,
        );
        if invalid {
            let mut texts = std::mem::take(&mut self.texts);
            let error = texts.error.text_in(
                catalog,
                context.catalog_generation,
                self.revision,
                |catalog, _, out| {
                    if let Some(error) = validation_error(&self.fields) {
                        error.write(catalog, out);
                    }
                },
            );
            draw_note_text(
                context,
                status,
                error,
                NoteKind::Warning,
                false,
                &mut texts.status_lines,
            );
            self.texts = texts;
        } else if dirty {
            let dot = Rect::new(
                status.x,
                status.y + (status.height - DOT_SIZE) * 0.5,
                DOT_SIZE,
                DOT_SIZE,
            );
            context
                .scene
                .fill(context.snap_rect(dot), DOT_SIZE * 0.5, theme.colors.accent);
            draw_note_text(
                context,
                status.cut_left(DOT_SIZE + theme.gap).1,
                catalog.text(editor::UNSAVED),
                NoteKind::Plain,
                false,
                &mut self.texts.status_lines,
            );
        }
        self.widgets
            .push(EditorWidget::new(UiId::from(CANCEL_ID), WidgetKind::Cancel));
        self.widgets
            .push(EditorWidget::new(UiId::from(SAVE_ID), WidgetKind::Save));
        if cancel.show(context, cancel_area) {
            actions.close = true;
        }
        if save.show(context, save_area) {
            actions.save = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use gui::ui::Navigation;
    use machine_factory::config::{
        settings::{SETTINGS, ValueKind},
        systems::compiled_systems,
    };

    use super::*;
    use crate::{
        editor::{
            derived_file::DerivedFile,
            media_list::{self, button_ids},
            rows::{RowAreas, browse_id, field_id},
            test_support::{
                EditorRoot, SHORT_SCREEN, media_section, row_of, show, show_in, system_named,
            },
        },
        gui_test_support::{GuiHarness, MINIMUM_SCREEN, MINIMUM_WIDTH},
        texts::english_text,
    };

    /// The English title of the header and the line beside it for the file
    /// `file` of the kind `kind`.
    fn header_texts(
        kind: &EditedFile,
        file: &EditorFile,
        model: Option<&str>,
        shown_path: &str,
    ) -> (String, String) {
        (
            english_text(|catalog, out| write_header_title(catalog, kind, file, out)),
            english_text(|catalog, out| {
                write_header_line(catalog, kind, file, model, shown_path, out);
            }),
        )
    }

    /// Verifies the header line of a game file, a game of another model, a new game, a copy, a system defaults file and the settings file.
    #[test]
    fn the_header_line_follows_the_file() {
        let pc98 = system_named("pc98");
        let game = |name: &str| EditedFile::Game {
            system: pc98,
            name: name.to_owned(),
        };
        let existing = EditorFile::Existing(PathBuf::from("/root/config/pc98/yu-no.conf"));
        assert_eq!(
            header_texts(&game("YU-NO"), &existing, None, "config/pc98/yu-no.conf"),
            ("YU-NO".to_owned(), "PC-98 game".to_owned())
        );
        assert_eq!(
            header_texts(&game("Policenauts"), &existing, Some("PC9821AP"), "a"),
            ("Policenauts".to_owned(), "PC-98 game, PC9821AP".to_owned())
        );
        let folder = std::env::temp_dir().join("neetan-header-test-absent");
        let derived = DerivedFile::new(&folder, "Brandish 2");
        assert_eq!(
            header_texts(
                &game("Brandish 2"),
                &EditorFile::New(derived.clone()),
                None,
                "config/pc98/brandish-2.conf"
            ),
            (
                "New PC-98 game".to_owned(),
                "Saved as config/pc98/brandish-2.conf".to_owned()
            )
        );
        let copy = EditorFile::Copy {
            source: "config/pc98/brandish.conf".to_owned(),
            file: derived,
        };
        assert_eq!(
            header_texts(&game("Brandish (copy)"), &copy, None, "x"),
            (
                "Brandish (copy)".to_owned(),
                "Copy of config/pc98/brandish.conf".to_owned()
            )
        );
        let defaults = EditorFile::Existing(PathBuf::from("/root/config/pc98.conf"));
        assert_eq!(
            header_texts(
                &EditedFile::SystemDefaults(pc98),
                &defaults,
                None,
                "config/pc98.conf"
            ),
            (
                "PC-98 system defaults".to_owned(),
                "Every PC-98 game starts from these values".to_owned()
            )
        );
        assert_eq!(
            header_texts(
                &EditedFile::Settings,
                &EditorFile::Existing(PathBuf::from("/root/neetan.conf")),
                None,
                "/root/neetan.conf"
            ),
            ("Settings".to_owned(), "/root/neetan.conf".to_owned())
        );
    }

    /// Verifies that a system defaults file that does not read shows its warning in the error color in the header, and that a file that reads shows none.
    #[test]
    fn the_header_warns_about_system_defaults_that_do_not_read() {
        let root = EditorRoot::new("page_warning");
        let defaults = root.write(&format!("config/{}.conf", root.system().name), "");
        let path = root.game("edge", "meta.title = Edge\n");
        let warned = |root: &EditorRoot| {
            let mut editor =
                Editor::open(&root.host(), root.system(), &path).expect("the editor opens");
            let mut harness = GuiHarness::minimum();
            harness.frame(|context| editor.show(context, MINIMUM_SCREEN));
            let danger = harness.theme.colors.danger_text.to_linear();
            harness.scene.instances().iter().any(|instance| {
                instance.kind[0] == gui::Instance::KIND_GLYPH
                    && instance.color == danger
                    && instance.rect[1] < HEADER_HEIGHT
            })
        };
        assert!(!warned(&root));
        std::fs::write(&defaults, b"crt = \xFF\n").expect("file is written");
        assert!(warned(&root));
    }

    /// Verifies that the page keeps the column inside its widest size and centers it, with the form between the tab bar and the rule of the help area above the footer.
    #[test]
    fn the_page_centers_its_column_in_a_wide_window() {
        let harness = GuiHarness::minimum();
        let theme = &harness.theme;
        let small = PageAreas::new(theme, MINIMUM_SCREEN);
        assert_eq!(small.tabs.x, theme.padding);
        assert_eq!(small.tabs.width, MINIMUM_SCREEN.width - theme.padding * 2.0);
        assert!(small.form.y > small.tabs.bottom());
        assert!(small.form.bottom() < small.rule.y);
        assert!(small.rule.bottom() < small.help.y);
        assert_eq!(small.rule.x, small.form.x);
        assert_eq!(small.rule.width, small.form.width);
        assert!(small.help.bottom() < small.footer.y);
        assert_eq!(small.help.height, help_area_height(theme));
        assert_eq!(small.help.right(), small.footer.right());
        let large = PageAreas::new(theme, Rect::new(0.0, 0.0, 1920.0, 1052.0));
        assert_eq!(large.tabs.width, FORM_MAX_WIDTH);
        assert_eq!(large.tabs.x, (1920.0 - FORM_MAX_WIDTH) * 0.5);
        assert_eq!(large.header.width, 1920.0 - theme.padding * 2.0);
        assert_eq!(form_area(theme, MINIMUM_SCREEN), small.form);
    }

    /// The kind and the field of the focused widget of the last pass of `editor`.
    fn focused_widget(
        harness: &GuiHarness,
        editor: &Editor,
    ) -> (Option<WidgetKind>, Option<usize>) {
        let focused = harness.state.focused();
        editor
            .widgets
            .iter()
            .find(|widget| Some(widget.id) == focused)
            .map_or((None, None), |widget| (Some(widget.kind), widget.field))
    }

    /// The key of the field `field` with the area of the main control of its row in the last
    /// pass of `editor` when the control starts outside `form`.
    fn hidden_control(
        harness: &GuiHarness,
        editor: &Editor,
        form: Rect,
        field: usize,
    ) -> Option<(&'static str, Rect)> {
        let row = row_of(editor, editor.fields[field].key());
        let control = RowAreas::new(&harness.theme, row.rect, row.rect.y).control;
        let shows = control.y >= form.y - 0.5 && control.y < form.bottom();
        (!shows).then_some((editor.fields[field].key(), control))
    }

    /// Verifies that Down walks every row of the Media section of a short page with tall media
    /// lists in order and leaves the form for the footer only from the last row, that Up walks
    /// back to the first row and leaves the form for the tab bar only from it, and that the
    /// main control of the focused row shows inside the form after every step. A media list
    /// with images advances its selection on the arrows before the focus leaves it. Covers
    /// every compiled system.
    #[test]
    fn the_arrows_walk_the_rows_of_a_short_page_in_order() {
        /// The most arrow presses of one walk.
        const MAXIMUM_STEPS: usize = 100;
        let mut harness = GuiHarness::new(MINIMUM_WIDTH as u32, 300);
        let form = form_area(&harness.theme, SHORT_SCREEN);
        let walk = |harness: &mut GuiHarness,
                    editor: &mut Editor,
                    navigation: Navigation,
                    leaves_to: WidgetKind| {
            let mut visited = vec![focused_widget(harness, editor).1.expect("a row")];
            let mut hidden = Vec::new();
            for _ in 0..MAXIMUM_STEPS {
                harness.input.navigation = Some(navigation);
                show_in(harness, editor, SHORT_SCREEN);
                match focused_widget(harness, editor) {
                    (Some(kind), _) if kind == leaves_to => return (visited, hidden),
                    (_, Some(field)) => {
                        hidden.extend(hidden_control(harness, editor, form, field));
                        if visited.last() != Some(&field) {
                            visited.push(field);
                        }
                    }
                    other => panic!("the focus left the form for {other:?} after {visited:?}"),
                }
            }
            panic!("the walk did not leave the form: {visited:?}");
        };
        for system in compiled_systems() {
            let name = system.name;
            let root = EditorRoot::with_system("page_short_walk", name);
            let mut lines = String::new();
            for setting in SETTINGS
                .iter()
                .filter(|setting| setting.kind == ValueKind::PathList)
            {
                for image in ["a.img", "b.img", "c.img"] {
                    lines.push_str(&format!("{} = {image}\n", setting.key));
                }
            }
            let mut editor = media_section(&root, &mut harness, SHORT_SCREEN, &lines);
            let mut rows: Vec<usize> = editor
                .widgets
                .iter()
                .filter_map(|widget| widget.field)
                .collect();
            rows.dedup();
            let tall = editor
                .rows
                .iter()
                .filter(|row| row.rect.height > form.height)
                .count();
            assert!(rows.len() >= 2 && tall >= 1, "{name} {rows:?} {tall}");
            assert!(
                editor
                    .rows
                    .last()
                    .is_some_and(|row| row.rect.y > form.bottom()),
                "{name}: the last row lies below the form"
            );

            harness
                .state
                .set_focus(field_id(editor.fields[rows[0]].key()));
            show_in(&mut harness, &mut editor, SHORT_SCREEN);
            let (down, hidden) = walk(
                &mut harness,
                &mut editor,
                Navigation::Down,
                WidgetKind::Cancel,
            );
            assert_eq!(down, rows, "{name}");
            assert_eq!(hidden, [], "{name}: the focused rows lie outside {form:?}");
            let last = rows[rows.len() - 1];
            harness.state.set_focus(field_id(editor.fields[last].key()));
            show_in(&mut harness, &mut editor, SHORT_SCREEN);
            let (up, hidden) = walk(&mut harness, &mut editor, Navigation::Up, WidgetKind::Tabs);
            rows.reverse();
            assert_eq!(up, rows, "{name}");
            assert_eq!(hidden, [], "{name}: the focused rows lie outside {form:?}");
        }
    }

    /// Sends the arrow `navigation` from the widget `from` and returns the widget that holds
    /// the focus after it.
    fn arrow_from(
        harness: &mut GuiHarness,
        editor: &mut Editor,
        from: UiId,
        navigation: Navigation,
    ) -> Option<UiId> {
        harness.state.set_focus(from);
        show(harness, editor);
        harness.input.navigation = Some(navigation);
        show(harness, editor);
        harness.state.focused()
    }

    /// Verifies that Right from the Browse button of a path row in the Media section keeps the
    /// focus, also from `hdd2` below the media lists of the pc98 and the x68k, that Left from
    /// it takes the path field, and that Left and Right walk the media list and the buttons of
    /// a media row with images. Covers every compiled system.
    #[test]
    fn the_sideways_arrows_stay_in_the_row_of_the_form() {
        let mut harness = GuiHarness::minimum();
        for system in compiled_systems() {
            let root = EditorRoot::with_system("page_sideways", system.name);
            let mut editor = media_section(&root, &mut harness, MINIMUM_SCREEN, "");
            let browse_rows: Vec<usize> = editor
                .widgets
                .iter()
                .filter(|widget| widget.kind == WidgetKind::Browse)
                .filter_map(|widget| widget.field)
                .collect();
            for row in browse_rows {
                let key = editor.fields[row].key();
                let browse = browse_id(key);
                assert_eq!(
                    arrow_from(&mut harness, &mut editor, browse, Navigation::Right),
                    Some(browse),
                    "{} {key}",
                    system.name
                );
                assert_eq!(
                    arrow_from(&mut harness, &mut editor, browse, Navigation::Left),
                    Some(field_id(key)),
                    "{} {key}",
                    system.name
                );
            }

            let mut editor = media_section(
                &root,
                &mut harness,
                MINIMUM_SCREEN,
                "fdd1 = a.img\nfdd1 = b.img\n",
            );
            let list = field_id("fdd1");
            let [add, remove, _, _] = button_ids(list);
            for (from, navigation, to) in [
                (list, Navigation::Right, add),
                (add, Navigation::Right, remove),
                (remove, Navigation::Left, add),
                (add, Navigation::Left, list),
            ] {
                assert_eq!(
                    arrow_from(&mut harness, &mut editor, from, navigation),
                    Some(to),
                    "{} {navigation:?}",
                    system.name
                );
            }
        }
    }

    /// Verifies that Left from the Browse button of `hdd1` after the wheel scrolled the button
    /// above the form takes the path field of `hdd1`, and that Right from the media list of
    /// `fdd1` with its top and its buttons above the form takes its Add button. Covers every
    /// compiled system.
    #[test]
    fn the_sideways_arrows_reach_widgets_above_the_view() {
        /// A page with a form shorter than one path row. The wheel scrolls the last rows of
        /// the Media section above it.
        const TINY_SCREEN: Rect = Rect::new(0.0, 0.0, MINIMUM_WIDTH, 230.0);
        /// The most wheel steps that scroll the Browse button above the form.
        const MAXIMUM_STEPS: usize = 40;
        let mut harness = GuiHarness::new(MINIMUM_WIDTH as u32, 300);
        let control_of = |harness: &GuiHarness, editor: &Editor, key: &str| {
            let row = row_of(editor, key);
            RowAreas::new(&harness.theme, row.rect, row.rect.y)
        };
        for system in compiled_systems() {
            let name = system.name;
            let root = EditorRoot::with_system("page_sideways_hidden", name);
            let mut editor = media_section(&root, &mut harness, TINY_SCREEN, "");
            if editor.fields.iter().any(|field| field.key() == "hdd1") {
                let form = form_area(&harness.theme, TINY_SCREEN);
                harness.state.set_focus(browse_id("hdd1"));
                show_in(&mut harness, &mut editor, TINY_SCREEN);
                harness.input.mouse_x = form.x + form.width * 0.5;
                harness.input.mouse_y = form.y + form.height * 0.5;
                for _ in 0..MAXIMUM_STEPS {
                    if control_of(&harness, &editor, "hdd1").control.bottom() < form.y {
                        break;
                    }
                    harness.input.wheel = -1.0;
                    show_in(&mut harness, &mut editor, TINY_SCREEN);
                }
                let control = control_of(&harness, &editor, "hdd1").control;
                assert!(control.bottom() < form.y, "{name} {control:?} {form:?}");
                assert_eq!(harness.state.focused(), Some(browse_id("hdd1")), "{name}");
                harness.input.navigation = Some(Navigation::Left);
                show_in(&mut harness, &mut editor, TINY_SCREEN);
                assert_eq!(harness.state.focused(), Some(field_id("hdd1")), "{name}");
            }

            let mut editor = media_section(
                &root,
                &mut harness,
                SHORT_SCREEN,
                "fdd1 = a.img\nfdd1 = b.img\nfdd1 = c.img\n",
            );
            let form = form_area(&harness.theme, SHORT_SCREEN);
            let button = harness.theme.control_height;
            let list_height = media_list::list_height(&harness.theme, 3);
            let wide = control_of(&harness, &editor, "fdd1").wide;
            editor.scroll.offset += wide.y + button + harness.theme.gap - form.y;
            let list = field_id("fdd1");
            harness.state.set_focus(list);
            show_in(&mut harness, &mut editor, SHORT_SCREEN);
            let wide = control_of(&harness, &editor, "fdd1").wide;
            assert!(
                wide.y + button < form.y && wide.y + list_height > form.y,
                "{name} {wide:?} {form:?}"
            );
            harness.input.navigation = Some(Navigation::Right);
            show_in(&mut harness, &mut editor, SHORT_SCREEN);
            let [add, _, _, _] = button_ids(list);
            assert_eq!(harness.state.focused(), Some(add), "{name}");
        }
    }
}
