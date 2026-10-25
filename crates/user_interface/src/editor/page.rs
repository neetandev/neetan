//! The page of the editor: the header, the tab bar of the sections, the
//! scroll area of the form and the footer with its buttons.

use gui::{
    Rect,
    ui::{
        ActionButton, Align, Button, ButtonKind, CLOSE_SIZE, HEADER_HEIGHT, Icon, ScrollState,
        Shortcut, Tab, Theme, Truncation, UiContext, UiId, scroll_area, tab_bar,
    },
};

use super::{
    BrowseTarget, EditedFile, Editor, EditorFile, Subject,
    hints::{EditorWidget, WidgetKind},
    notes::Note,
    rows::{DOT_SIZE, draw_note},
};
use crate::media_dialog::TAB_BAR_HEIGHT;

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

/// The footer text with unsaved changes.
const UNSAVED_TEXT: &str = "Unsaved changes";

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

/// The scroll area of the form of the page in `screen`.
#[cfg(test)]
pub(crate) fn form_area(theme: &Theme, screen: Rect) -> Rect {
    PageAreas::new(theme, screen).form
}

/// The title of the header and the line beside it for the file `file` of
/// the kind `kind`.
///
/// A game file shows its name and its system, with the model when the
/// model differs from the model of a game without `machine`. A new game
/// shows its system and the file it saves into. A copy shows its name and
/// the file it copies. A system defaults file shows its system. The
/// settings file shows its path.
fn header_texts(
    kind: &EditedFile,
    file: &EditorFile,
    model: Option<&str>,
    shown_path: &str,
) -> (String, String) {
    match kind {
        EditedFile::Game { system, name } => match file {
            EditorFile::Existing(_) => {
                let line = match model {
                    Some(model) => format!("{} game, {model}", system.label),
                    None => format!("{} game", system.label),
                };
                (name.clone(), line)
            }
            EditorFile::New(_) => (
                format!("New {} game", system.label),
                format!("Saved as {shown_path}"),
            ),
            EditorFile::Copy { source, .. } => (name.clone(), format!("Copy of {source}")),
        },
        EditedFile::SystemDefaults(system) => (
            kind.display_name(),
            format!("Every {} game starts from these values", system.label),
        ),
        EditedFile::Settings => (kind.display_name(), shown_path.to_owned()),
    }
}

impl Editor {
    /// Draws the page into `screen` and returns the changes it asks for.
    pub(super) fn draw_page(&mut self, context: &mut UiContext, screen: Rect) -> PageActions {
        let theme = context.theme;
        let mut actions = PageActions::default();
        context.fill(screen, theme.panel);
        let areas = PageAreas::new(theme, screen);
        if self.draw_header(context, areas.header) {
            actions.close = true;
        }

        let tabs: Vec<Tab> = self
            .sections()
            .iter()
            .map(|section| Tab::new(section.caption(), None))
            .collect();
        let tabs_id = UiId::from(TABS_ID);
        if tab_bar(context, tabs_id, areas.tabs, &tabs, &mut self.section) {
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
        let shown_model = match &self.subject {
            Subject::Game(files) | Subject::SystemDefaults(files) => {
                let model = self.model_of(files);
                (model != files.base.model().0).then(|| model.name())
            }
            Subject::Host(_) => None,
        };
        let (title, small) = header_texts(
            &self.edited_file(),
            &self.file,
            shown_model,
            &self.shown_path(),
        );
        let title_style = theme.title();
        let small_style = theme.small();
        let warning_style = theme.small().with_color(theme.danger_text);
        let warning = self.system_files().and_then(|files| files.base.warning());
        let small_width = context.measure(&small_style, &small);
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
        let title_width = context.measure(&title_style, &title).min(title_room);
        let baseline = context.centered_baseline(header, &title_style);
        context.text_in(
            Rect::new(header.x, header.y, title_width, header.height),
            &title_style,
            &title,
            Align::Start,
            Truncation::End,
        );
        let small_x = header.x + title_width + theme.gap * 1.5;
        let small_room = (close_area.x - theme.gap - small_x).max(0.0);
        if small_room > 0.0 {
            context
                .scene
                .push_clip(Rect::new(small_x, header.y, small_room, header.height));
            context.text(small_x, baseline, &small_style, &small);
            if let Some(warning) = warning {
                let icon_x = small_x + small_width + theme.gap * 1.5;
                let icon_area = Rect::new(icon_x, header.y, theme.icon_size, header.height);
                context.icon(icon_area, Icon::Alert, theme.icon_size, theme.danger_text);
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
        let close_id = UiId::from(CLOSE_ID);
        self.widgets
            .push(EditorWidget::new(close_id, WidgetKind::Close));
        Button::new(close_id, "")
            .icon(Icon::Close)
            .kind(ButtonKind::Quiet)
            .default_choice(false)
            .show(context, close_area)
    }

    /// Draws the footer: the validation error or the unsaved changes, Cancel
    /// and Save.
    fn draw_footer(&mut self, context: &mut UiContext, footer: Rect, actions: &mut PageActions) {
        let theme = context.theme;
        let dirty = self.is_dirty();
        let error = super::save::validation_error(&self.fields);
        let save = Button::new(SAVE_ID, "Save")
            .icon(Icon::Check)
            .kind(ButtonKind::Primary)
            .enabled(dirty && error.is_none());
        let cancel = Button::new(CANCEL_ID, "Cancel");
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
        if let Some(error) = &error {
            draw_note(context, status, &Note::warning(error.clone()));
        } else if dirty {
            let dot = Rect::new(
                status.x,
                status.y + (status.height - DOT_SIZE) * 0.5,
                DOT_SIZE,
                DOT_SIZE,
            );
            context
                .scene
                .fill(context.snap_rect(dot), DOT_SIZE * 0.5, theme.accent);
            draw_note(
                context,
                status.cut_left(DOT_SIZE + theme.gap).1,
                &Note::plain(UNSAVED_TEXT),
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
    };

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
            let danger = harness.theme.danger_text.to_linear();
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
