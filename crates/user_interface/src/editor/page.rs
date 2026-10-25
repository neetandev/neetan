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

/// The areas of the page in `screen`: the header, the tab bar, the form and
/// the footer. The tab bar, the form and the footer keep inside
/// [`FORM_MAX_WIDTH`] and center.
#[derive(Debug, Clone, Copy, PartialEq)]
struct PageAreas {
    /// The title, the line beside it and the close button.
    header: Rect,
    /// The tab bar of the sections.
    tabs: Rect,
    /// The scroll area of the form.
    form: Rect,
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
        let (footer, form) = rest.cut_bottom(theme.control_height);
        let form = form.cut_top(theme.gap * 1.5).1;
        let form = form.cut_bottom(theme.gap * 1.5).1;
        Self {
            header,
            tabs,
            form,
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

    use super::*;
    use crate::{
        editor::{
            derived_file::DerivedFile,
            test_support::{EditorRoot, system_named},
        },
        gui_test_support::GuiHarness,
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
            let mut harness = GuiHarness::new(960, 600);
            harness.frame(|context| editor.show(context, Rect::new(0.0, 0.0, 960.0, 572.0)));
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

    /// Verifies that the page keeps the column inside its widest size and centers it, with the form between the tab bar and the footer.
    #[test]
    fn the_page_centers_its_column_in_a_wide_window() {
        let harness = GuiHarness::new(960, 600);
        let theme = &harness.theme;
        let small = PageAreas::new(theme, Rect::new(0.0, 0.0, 960.0, 572.0));
        assert_eq!(small.tabs.x, theme.padding);
        assert_eq!(small.tabs.width, 960.0 - theme.padding * 2.0);
        assert!(small.form.y > small.tabs.bottom());
        assert!(small.form.bottom() < small.footer.y);
        let large = PageAreas::new(theme, Rect::new(0.0, 0.0, 1920.0, 1052.0));
        assert_eq!(large.tabs.width, FORM_MAX_WIDTH);
        assert_eq!(large.tabs.x, (1920.0 - FORM_MAX_WIDTH) * 0.5);
        assert_eq!(large.header.width, 1920.0 - theme.padding * 2.0);
        assert_eq!(
            form_area(theme, Rect::new(0.0, 0.0, 960.0, 572.0)),
            small.form
        );
    }
}
