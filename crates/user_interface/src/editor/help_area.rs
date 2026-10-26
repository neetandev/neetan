//! The help area of the editor and of the settings page: the help text of
//! the setting of a form row below the form. The later of a hover change
//! and a focus change selects the row.

use gui::{
    Rect,
    ui::{Align, Theme, UiContext, UiId, WrappedText, wrapped_line_height},
};
use machine_factory::config::settings::Setting;

use super::{BrowseTarget, Editor};
use crate::texts::setting_help;

/// The most lines of a help text. A longer text ends the last line in an
/// ellipsis.
pub(crate) const HELP_MAXIMUM_LINES: usize = 2;

/// The height of the help area in `theme`: the lines of a help text in the
/// small text.
pub(crate) fn help_area_height(theme: &Theme) -> f32 {
    wrapped_line_height(&theme.small()) * HELP_MAXIMUM_LINES as f32
}

/// The help text of one setting in a fixed area.
///
/// The wrapped lines stay cached between passes. They wrap again only when
/// the text changes with the setting or the catalog, or when the width or
/// the font size changes.
#[derive(Debug, Clone, Default)]
pub(crate) struct HelpArea {
    /// The wrapped lines of the help text.
    lines: WrappedText,
}

impl HelpArea {
    /// Draws the help text of `setting` into `area` in the small text of
    /// the notes, from the top of `area` down. No setting leaves the area
    /// empty.
    pub(crate) fn show(
        &mut self,
        context: &mut UiContext,
        area: Rect,
        setting: Option<&'static Setting>,
    ) {
        let Some(setting) = setting else {
            return;
        };
        let text = setting_help(context.catalog, setting);
        if text.is_empty() || area.width <= 0.0 {
            return;
        }
        let style = context.theme.small();
        self.lines.show(
            context,
            area,
            &style,
            text,
            Align::Start,
            HELP_MAXIMUM_LINES,
        );
    }
}

/// The row of one field that a pass drew.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FormRow {
    /// The index of the field.
    pub(crate) field: usize,
    /// The area of the row across the width of the form: the label, the
    /// value, the note and the space between them.
    pub(crate) rect: Rect,
    /// Whether the pointer is over the row, no layer above the form blocks
    /// it and no widget drawn before the row took it.
    pub(crate) hovered: bool,
}

/// The change that selects the row of the help area.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum HelpSource {
    /// The pointer entered or pressed a setting row after the last focus
    /// change.
    Hover,
    /// The focus changed after the pointer entered the last setting row.
    #[default]
    Focus,
}

/// The pointer and the focus of the last pass that decide the source of the
/// help area.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct HelpTracking {
    /// The pointer position of the last pass. The first pass sets it.
    pointer: Option<(f32, f32)>,
    /// Whether the wheel turned in the last pass. A scroll area applies the
    /// wheel after its rows, and the rows of the next pass show the turn.
    wheel_turned: bool,
    /// The field of the row that the pointer entered or pressed last.
    entered_field: Option<usize>,
    /// The focused widget of the last pass.
    focused: Option<UiId>,
    /// The change that selects the row of the help area.
    source: HelpSource,
}

impl HelpTracking {
    /// Forgets the row that the pointer entered last. The rows of a new
    /// section under a resting pointer enter no row.
    pub(crate) fn leave_section(&mut self) {
        self.entered_field = None;
    }
}

impl Editor {
    /// Updates the source of the help area from the rows of this pass. A
    /// focus that differs from the last pass selects the focused row. A
    /// pointer that moves, or a wheel that turns in this pass or the last
    /// one, records the row under the pointer as entered when it differs
    /// from the row entered last. An entered setting row selects the
    /// entered row. A press of the left button over a setting row enters
    /// that row and selects it. The entered row stays selected until the
    /// pointer or the wheel enters another row. A form that scrolls under a
    /// resting pointer and a section switch enter no row.
    pub(super) fn track_help(&mut self, context: &UiContext) {
        let tracking = &mut self.help_tracking;
        let focused = context.focused();
        if focused != tracking.focused {
            tracking.focused = focused;
            tracking.source = HelpSource::Focus;
        }
        let pointer = context.mouse();
        let wheel_turned = context.input.wheel != 0.0;
        let pointer_acted = tracking.pointer.is_some_and(|last| last != pointer)
            || wheel_turned
            || tracking.wheel_turned;
        tracking.pointer = Some(pointer);
        tracking.wheel_turned = wheel_turned;
        let hovered = self
            .rows
            .iter()
            .find(|row| row.hovered)
            .map(|row| row.field);
        let hovered_setting =
            hovered.is_some_and(|field| self.fields[field].source.setting().is_some());
        if pointer_acted && hovered != tracking.entered_field {
            tracking.entered_field = hovered;
            if hovered_setting {
                tracking.source = HelpSource::Hover;
            }
        }
        if hovered_setting && context.input.left_pressed() {
            tracking.entered_field = hovered;
            tracking.source = HelpSource::Hover;
        }
    }

    /// The setting of the help area. After a hover change it is the setting
    /// of the row that the pointer entered or pressed last, and after a
    /// focus change the setting of the focused row. Without an entered
    /// setting row it is the setting of the focused row.
    pub(crate) fn help_setting(&self, context: &UiContext) -> Option<&'static Setting> {
        let entered = match self.help_tracking.source {
            HelpSource::Hover => self.entered_setting(),
            HelpSource::Focus => None,
        };
        entered.or_else(|| self.focused_setting(context))
    }

    /// The setting of the row that the pointer entered or pressed last. A
    /// row without a setting and a row outside the rows of this pass give
    /// none.
    fn entered_setting(&self) -> Option<&'static Setting> {
        let field = self.help_tracking.entered_field?;
        if !self.rows.iter().any(|row| row.field == field) {
            return None;
        }
        self.fields[field].source.setting()
    }

    /// The setting of the focused row: the row of the widget with the
    /// focus, the row of a dropdown with an open popup, or the row that the
    /// open file browser fills. A metadata row, a widget outside the form
    /// and a dialog give none.
    fn focused_setting(&self, context: &UiContext) -> Option<&'static Setting> {
        let index = match self.browser.as_ref().map(|(_, target)| *target) {
            Some(BrowseTarget::Path(index) | BrowseTarget::List(index, _)) => Some(index),
            None => self
                .widgets
                .iter()
                .find(|widget| {
                    context.popup_open(widget.id) || context.focused() == Some(widget.id)
                })
                .and_then(|widget| widget.field),
        }?;
        self.fields[index].source.setting()
    }
}

#[cfg(test)]
mod tests {
    use gui::ui::{Navigation, UiId};
    use machine_factory::{
        config::{MachineSelection, settings::MACHINE_KEY, systems::compiled_systems},
        rom_status::RomStatus,
    };

    use super::*;
    use crate::{
        config::metadata::TITLE_KEY,
        editor::{
            EditorSection,
            form::{FieldSource, form_fields, host_fields},
            page::{CANCEL_ID, CLOSE_ID, TABS_ID, form_area, help_area},
            rows::{RowAreas, field_id},
            test_support::{
                EditorRoot, SCREEN, SHORT_SCREEN, SettingsRoot, fdd1_model, help_glyphs,
                media_section, row_of, settings_page, show, show_in, shown_help, shown_help_in,
                statuses_with,
            },
        },
        gui_test_support::{GuiHarness, PassCounts},
        texts::{LanguageCatalog, repository::repository_catalogs, test_catalog},
    };

    /// Places the pointer of `harness` at `x`, `y`.
    fn point_at(harness: &mut GuiHarness, x: f32, y: f32) {
        harness.input.mouse_x = x;
        harness.input.mouse_y = y;
    }

    /// Places the pointer of `harness` on the label of the row of `key` in the last pass of
    /// `editor`.
    fn point_at_row(harness: &mut GuiHarness, editor: &Editor, key: &str) {
        let row = row_of(editor, key).rect;
        let label = RowAreas::new(&harness.theme, row, row.y).label;
        point_at(harness, label.x + 4.0, label.y + label.height * 0.5);
    }

    /// The key of the field of the focused widget of the last pass of `editor`.
    fn focused_key(harness: &GuiHarness, editor: &Editor) -> Option<&'static str> {
        let focused = harness.state.focused()?;
        let widget = editor.widgets.iter().find(|widget| widget.id == focused)?;
        widget.field.map(|field| editor.fields[field].key())
    }

    /// Asserts that a steady frame of `editor` in `screen` allocates nothing, fills nothing and
    /// wraps nothing.
    fn assert_steady(harness: &mut GuiHarness, editor: &mut Editor, screen: Rect) {
        let (counts, _) = harness.counted_frame(|context| {
            editor.show(context, screen);
        });
        assert_eq!(counts, PassCounts::NONE);
    }

    /// The settings page of `test` on the Video and audio section with its first row focused,
    /// and the keys of its first `N` rows.
    fn video_section<const N: usize>(
        test: &SettingsRoot,
        harness: &mut GuiHarness,
    ) -> (Editor, [&'static str; N]) {
        let host = test.host(&[]);
        let mut editor = settings_page(&host, &statuses_with(&host, RomStatus::NotFound));
        show(harness, &mut editor);
        for _ in 0..2 {
            harness.input.navigation = Some(Navigation::NextTab);
            show(harness, &mut editor);
        }
        assert_eq!(editor.current_section(), EditorSection::VideoAndAudio);
        let mut keys = editor
            .fields
            .iter()
            .filter(|field| field.section == EditorSection::VideoAndAudio)
            .map(|field| field.key());
        let rows = [(); N].map(|()| keys.next().expect("the section has the row"));
        assert_eq!(harness.state.focused(), Some(field_id(rows[0])));
        (editor, rows)
    }

    /// Verifies that the help area shows the help of the focused setting row of the game editor
    /// and of the settings page, follows the focus of Tab, of the arrows and of a click, keeps
    /// the row of an open popup, and stays empty for a metadata row, the tab bar, the close
    /// button and the cancel button of the footer.
    #[test]
    fn the_help_area_follows_the_focused_row() {
        let root = EditorRoot::new("help_focus");
        let mut harness = GuiHarness::new(960, 600);
        let path = root.game("edge", "meta.title = Edge\n");
        let mut editor =
            Editor::open(&root.host(), root.system(), &path).expect("the editor opens");
        show(&mut harness, &mut editor);
        assert_eq!(harness.state.focused(), Some(field_id("meta.title")));
        assert_eq!(shown_help(&mut harness, &mut editor), None);
        harness.input.navigation = Some(Navigation::NextTab);
        show(&mut harness, &mut editor);
        assert_eq!(editor.current_section(), EditorSection::Machine);
        assert_eq!(harness.state.focused(), Some(field_id(MACHINE_KEY)));
        assert_eq!(shown_help(&mut harness, &mut editor), Some(MACHINE_KEY));
        assert!(!help_glyphs(&harness).is_empty());
        for id in [CANCEL_ID, CLOSE_ID, TABS_ID] {
            harness.state.set_focus(UiId::from(id));
            show(&mut harness, &mut editor);
            assert_eq!(harness.state.focused(), Some(UiId::from(id)));
            assert_eq!(shown_help(&mut harness, &mut editor), None, "{id}");
        }

        let test = SettingsRoot::new("help_focus_settings", Some(""));
        let (mut editor, [first, second]) = video_section(&test, &mut harness);
        assert_eq!(shown_help(&mut harness, &mut editor), Some(first));
        harness.input.navigation = Some(Navigation::Next);
        show(&mut harness, &mut editor);
        assert_eq!(harness.state.focused(), Some(field_id(second)));
        assert_eq!(shown_help(&mut harness, &mut editor), Some(second));
        harness.input.navigation = Some(Navigation::Up);
        show(&mut harness, &mut editor);
        assert_eq!(shown_help(&mut harness, &mut editor), Some(first));
        harness.input.navigation = Some(Navigation::Down);
        show(&mut harness, &mut editor);
        assert_eq!(shown_help(&mut harness, &mut editor), Some(second));

        let theme = &harness.theme;
        let form = form_area(theme, SCREEN);
        let control = RowAreas::new(theme, form, form.y).control;
        harness.input.mouse_x = control.x + control.width * 0.5;
        harness.input.mouse_y = control.y + control.height * 0.5;
        harness.input.left = true;
        show(&mut harness, &mut editor);
        harness.input.left = false;
        show(&mut harness, &mut editor);
        assert!(harness.state.popup_open());
        assert_eq!(shown_help(&mut harness, &mut editor), Some(first));
        harness.input.cancel = true;
        show(&mut harness, &mut editor);
        assert!(!harness.state.popup_open());
        assert_eq!(harness.state.focused(), Some(field_id(first)));
        assert_eq!(shown_help(&mut harness, &mut editor), Some(first));
    }

    /// Verifies that the pointer entering a row after the last focus change shows its help: a
    /// hover over the label, the value and the empty space of a row shows its help, a pointer
    /// outside the form gives the help of the focused row, a steady frame with a hovered row
    /// allocates nothing, and an open popup blocks the hover of the rows.
    #[test]
    fn the_help_area_follows_the_hovered_row() {
        let test = SettingsRoot::new("help_hover", Some(""));
        let mut harness = GuiHarness::new(960, 600);
        let (mut editor, [first, second]) = video_section(&test, &mut harness);
        let row = row_of(&editor, second).rect;
        let areas = RowAreas::new(&harness.theme, row, row.y);
        let padding = harness.theme.padding;
        let label = areas.label;
        let control = areas.control;
        let points = [
            (label.x + 4.0, label.y + label.height * 0.5),
            (
                control.x + control.width * 0.5,
                control.y + control.height * 0.5,
            ),
            (label.right() + padding * 0.5, row.y + 1.0),
        ];
        for (x, y) in points {
            point_at(&mut harness, x, y);
            show(&mut harness, &mut editor);
            assert_eq!(harness.state.focused(), Some(field_id(first)));
            assert_eq!(
                shown_help(&mut harness, &mut editor),
                Some(second),
                "{x} {y}"
            );
        }
        assert_steady(&mut harness, &mut editor, SCREEN);

        point_at(&mut harness, 1.0, 1.0);
        show(&mut harness, &mut editor);
        assert_eq!(shown_help(&mut harness, &mut editor), Some(first));

        let control = RowAreas::new(&harness.theme, row, row_of(&editor, first).rect.y).control;
        point_at(
            &mut harness,
            control.x + control.width * 0.5,
            control.y + control.height * 0.5,
        );
        harness.input.left = true;
        show(&mut harness, &mut editor);
        harness.input.left = false;
        show(&mut harness, &mut editor);
        assert!(harness.state.popup_open());
        let (x, y) = points[0];
        point_at(&mut harness, x, y);
        show(&mut harness, &mut editor);
        assert!(editor.rows.iter().all(|row| !row.hovered));
        assert_eq!(shown_help(&mut harness, &mut editor), Some(first));
    }

    /// Verifies that a hover over a row below an open dialog leaves the help area to the rule of
    /// the focused row.
    #[test]
    fn a_dialog_blocks_the_hover_of_the_rows() {
        let root = EditorRoot::new("help_hover_dialog");
        let path = root.game("edge", "meta.title = Edge\n");
        let mut editor =
            Editor::open(&root.host(), root.system(), &path).expect("the editor opens");
        let mut harness = GuiHarness::new(960, 600);
        show(&mut harness, &mut editor);
        assert_eq!(harness.state.focused(), Some(field_id(TITLE_KEY)));
        point_at_row(&mut harness, &editor, TITLE_KEY);
        show(&mut harness, &mut editor);
        assert!(editor.rows.iter().any(|row| row.hovered));
        assert_eq!(shown_help(&mut harness, &mut editor), None);

        harness.input.navigation = Some(Navigation::NextTab);
        show(&mut harness, &mut editor);
        assert_eq!(editor.current_section(), EditorSection::Machine);
        assert_eq!(harness.state.focused(), Some(field_id(MACHINE_KEY)));
        point_at_row(&mut harness, &editor, MACHINE_KEY);
        show(&mut harness, &mut editor);
        assert_eq!(shown_help(&mut harness, &mut editor), Some(MACHINE_KEY));
        harness.input.navigation = Some(Navigation::PreviousTab);
        show(&mut harness, &mut editor);
        assert_eq!(editor.current_section(), EditorSection::Metadata);
        assert_eq!(harness.state.focused(), Some(field_id(TITLE_KEY)));

        harness.input.text = "!".to_owned();
        point_at(&mut harness, 1.0, 1.0);
        show(&mut harness, &mut editor);
        harness.input.navigation = Some(Navigation::NextTab);
        show(&mut harness, &mut editor);
        assert_eq!(editor.current_section(), EditorSection::Machine);
        harness.state.set_focus(UiId::from(TABS_ID));
        show(&mut harness, &mut editor);
        assert_eq!(shown_help(&mut harness, &mut editor), None);
        let row = row_of(&editor, MACHINE_KEY).rect;
        point_at(&mut harness, row.x + 4.0, row.y + row.height * 0.5);
        show(&mut harness, &mut editor);
        assert_eq!(shown_help(&mut harness, &mut editor), Some(MACHINE_KEY));

        harness.input.cancel = true;
        show(&mut harness, &mut editor);
        show(&mut harness, &mut editor);
        assert_eq!(harness.state.open_modals(), 1);
        assert!(editor.rows.iter().all(|row| !row.hovered));
        assert_eq!(shown_help(&mut harness, &mut editor), None);
    }

    /// Verifies that the later of a hover change and a focus change selects the row of the help
    /// area: a Tab and an arrow under a resting pointer show the focused row, the pointer
    /// entering another row shows that row, a pointer outside the form shows the focused row,
    /// and a steady frame after each step allocates nothing.
    #[test]
    fn the_later_of_a_hover_and_a_focus_change_wins() {
        let test = SettingsRoot::new("help_later", Some(""));
        let mut harness = GuiHarness::new(960, 600);
        let (mut editor, rows) = video_section::<4>(&test, &mut harness);
        point_at_row(&mut harness, &editor, rows[3]);
        show(&mut harness, &mut editor);
        assert_steady(&mut harness, &mut editor, SCREEN);
        assert_eq!(shown_help(&mut harness, &mut editor), Some(rows[3]));

        for (navigation, focused) in [(Navigation::Next, rows[1]), (Navigation::Down, rows[2])] {
            harness.input.navigation = Some(navigation);
            show(&mut harness, &mut editor);
            assert_eq!(harness.state.focused(), Some(field_id(focused)));
            assert!(
                row_of(&editor, rows[3])
                    .rect
                    .contains(harness.input.mouse_x, harness.input.mouse_y)
            );
            assert_steady(&mut harness, &mut editor, SCREEN);
            assert_eq!(shown_help(&mut harness, &mut editor), Some(focused));
        }

        point_at_row(&mut harness, &editor, rows[0]);
        show(&mut harness, &mut editor);
        assert_steady(&mut harness, &mut editor, SCREEN);
        assert_eq!(harness.state.focused(), Some(field_id(rows[2])));
        assert_eq!(shown_help(&mut harness, &mut editor), Some(rows[0]));

        point_at(&mut harness, 1.0, 1.0);
        show(&mut harness, &mut editor);
        assert_steady(&mut harness, &mut editor, SCREEN);
        assert_eq!(shown_help(&mut harness, &mut editor), Some(rows[2]));
    }

    /// Verifies that a click on the label or on the empty space of a hovered row after a Tab to
    /// another row gives the clicked row the focus and shows its help, that a steady frame
    /// after the click allocates nothing, and that a Tab after the click shows the help of the
    /// next row.
    #[test]
    fn a_click_on_a_row_shows_its_help() {
        let test = SettingsRoot::new("help_click", Some(""));
        let mut harness = GuiHarness::new(960, 600);
        let (mut editor, [first, second]) = video_section(&test, &mut harness);
        let row = row_of(&editor, first).rect;
        let label = RowAreas::new(&harness.theme, row, row.y).label;
        let points = [
            (label.x + 4.0, label.y + label.height * 0.5),
            (label.right() + harness.theme.padding * 0.5, row.y + 1.0),
        ];
        for (pointer_x, pointer_y) in points {
            harness.state.set_focus(field_id(first));
            show(&mut harness, &mut editor);
            point_at(&mut harness, pointer_x, pointer_y);
            show(&mut harness, &mut editor);
            assert_eq!(shown_help(&mut harness, &mut editor), Some(first));
            harness.input.navigation = Some(Navigation::Next);
            show(&mut harness, &mut editor);
            assert_eq!(harness.state.focused(), Some(field_id(second)));
            assert_eq!(shown_help(&mut harness, &mut editor), Some(second));
            harness.input.left = true;
            show(&mut harness, &mut editor);
            harness.input.left = false;
            show(&mut harness, &mut editor);
            assert_eq!(harness.state.focused(), Some(field_id(first)));
            assert_steady(&mut harness, &mut editor, SCREEN);
            assert_eq!(
                shown_help(&mut harness, &mut editor),
                Some(first),
                "{pointer_x} {pointer_y}"
            );
            harness.input.navigation = Some(Navigation::Next);
            show(&mut harness, &mut editor);
            assert_eq!(harness.state.focused(), Some(field_id(second)));
            assert_eq!(shown_help(&mut harness, &mut editor), Some(second));
        }
    }

    /// Verifies that the help area follows the focus of the arrows while the pointer rests on the
    /// row above the lowest visible row and the form scrolls other rows under the pointer to
    /// reveal the focus, that a steady frame after each step allocates nothing, and that a turn
    /// of the wheel after a focus change to the tab bar shows the row that the wheel brings
    /// under the resting pointer. A short page makes the Video and audio section of the
    /// settings page scroll.
    #[test]
    fn the_help_area_follows_the_focus_while_the_form_scrolls() {
        let test = SettingsRoot::new("help_scroll", Some(""));
        let host = test.host(&[]);
        let mut editor = settings_page(&host, &statuses_with(&host, RomStatus::NotFound));
        let mut harness = GuiHarness::new(960, 600);
        show_in(&mut harness, &mut editor, SHORT_SCREEN);
        for _ in 0..2 {
            harness.input.navigation = Some(Navigation::NextTab);
            show_in(&mut harness, &mut editor, SHORT_SCREEN);
        }
        assert_eq!(editor.current_section(), EditorSection::VideoAndAudio);
        let form = form_area(&harness.theme, SHORT_SCREEN);
        let last = editor.rows.last().expect("the section has rows");
        assert!(
            last.rect.y > form.bottom(),
            "the last row lies below the form"
        );
        let visible = editor
            .rows
            .iter()
            .filter(|row| row.rect.bottom() <= form.bottom())
            .count();
        assert!(visible >= 2, "the form shows two rows");
        let resting = editor.fields[editor.rows[visible - 2].field].key();
        point_at_row(&mut harness, &editor, resting);
        show_in(&mut harness, &mut editor, SHORT_SCREEN);
        assert_eq!(
            shown_help_in(&mut harness, &mut editor, SHORT_SCREEN),
            Some(resting)
        );

        let top = editor.rows[0].rect.y;
        let mut focused = focused_key(&harness, &editor);
        let mut scrolled = false;
        for _ in 1..editor.rows.len() {
            harness.input.navigation = Some(Navigation::Down);
            show_in(&mut harness, &mut editor, SHORT_SCREEN);
            let now = focused_key(&harness, &editor);
            assert!(now.is_some() && now != focused, "{focused:?} {now:?}");
            focused = now;
            assert_steady(&mut harness, &mut editor, SHORT_SCREEN);
            assert_eq!(
                shown_help_in(&mut harness, &mut editor, SHORT_SCREEN),
                focused
            );
            scrolled |= editor.rows[0].rect.y < top;
        }
        assert!(scrolled, "the form scrolls to the focus");

        let under_pointer = |editor: &Editor| {
            editor
                .rows
                .iter()
                .find(|row| row.hovered)
                .map(|row| editor.fields[row.field].key())
        };
        let before = under_pointer(&editor);
        harness.input.mouse_y += 1.0;
        show_in(&mut harness, &mut editor, SHORT_SCREEN);
        assert_eq!(
            shown_help_in(&mut harness, &mut editor, SHORT_SCREEN),
            before
        );
        harness.state.set_focus(UiId::from(TABS_ID));
        show_in(&mut harness, &mut editor, SHORT_SCREEN);
        assert_eq!(under_pointer(&editor), before);
        assert_eq!(shown_help_in(&mut harness, &mut editor, SHORT_SCREEN), None);
        harness.input.wheel = 1.0;
        show_in(&mut harness, &mut editor, SHORT_SCREEN);
        let after = under_pointer(&editor);
        assert!(after.is_some() && after != before, "{before:?} {after:?}");
        assert_steady(&mut harness, &mut editor, SHORT_SCREEN);
        assert_eq!(
            shown_help_in(&mut harness, &mut editor, SHORT_SCREEN),
            after
        );
    }

    /// Asserts that Left and Right on the focused tab bar of `editor` with the pointer resting
    /// on the row of `key` leave the help area empty, that a following motion of the pointer
    /// shows the help of `key`, and that a steady frame after each step allocates nothing.
    fn assert_a_section_switch_enters_no_row(
        harness: &mut GuiHarness,
        editor: &mut Editor,
        key: &'static str,
    ) {
        let tabs = UiId::from(TABS_ID);
        harness.state.set_focus(tabs);
        show(harness, editor);
        point_at_row(harness, editor, key);
        show(harness, editor);
        assert_eq!(shown_help(harness, editor), Some(key));
        let section = editor.current_section();
        for navigation in [Navigation::Left, Navigation::Right] {
            harness.input.navigation = Some(navigation);
            show(harness, editor);
            assert_eq!(harness.state.focused(), Some(tabs));
            assert_steady(harness, editor, SCREEN);
            assert_eq!(shown_help(harness, editor), None, "{navigation:?}");
        }
        assert_eq!(editor.current_section(), section);
        assert!(row_of(editor, key).hovered);
        harness.input.mouse_y += 1.0;
        show(harness, editor);
        assert_steady(harness, editor, SCREEN);
        assert_eq!(shown_help(harness, editor), Some(key));
    }

    /// Verifies that a section switch of the game editor under a resting pointer leaves the
    /// help area to the focused tab bar until the pointer moves.
    #[test]
    fn a_section_switch_of_the_game_editor_enters_no_row() {
        let root = EditorRoot::new("help_switch_game");
        let mut harness = GuiHarness::new(960, 600);
        let mut editor = media_section(&root, &mut harness, SCREEN, "");
        assert_a_section_switch_enters_no_row(&mut harness, &mut editor, "fdd1");
    }

    /// Verifies that a section switch of the settings page under a resting pointer leaves the
    /// help area to the focused tab bar until the pointer moves.
    #[test]
    fn a_section_switch_of_the_settings_page_enters_no_row() {
        let test = SettingsRoot::new("help_switch_settings", Some(""));
        let mut harness = GuiHarness::new(960, 600);
        let (mut editor, [_, second]) = video_section(&test, &mut harness);
        assert_a_section_switch_enters_no_row(&mut harness, &mut editor, second);
    }

    /// Verifies that the system defaults editor shows the help of its focused machine row.
    #[test]
    fn the_defaults_editor_shows_the_help_of_the_machine_row() {
        let root = EditorRoot::new("help_defaults");
        let mut editor =
            Editor::system_defaults(&root.host(), root.system()).expect("the editor opens");
        let mut harness = GuiHarness::new(960, 600);
        show(&mut harness, &mut editor);
        assert_eq!(editor.current_section(), EditorSection::Machine);
        harness.state.set_focus(field_id(MACHINE_KEY));
        show(&mut harness, &mut editor);
        assert_eq!(harness.state.focused(), Some(field_id(MACHINE_KEY)));
        assert_eq!(shown_help(&mut harness, &mut editor), Some(MACHINE_KEY));
        assert!(!help_glyphs(&harness).is_empty());
    }

    /// Verifies that the open file browser keeps the help of the row that it fills and that the
    /// question before unsaved changes go away empties the help area.
    #[test]
    fn the_browser_keeps_the_help_and_a_dialog_empties_it() {
        let root = EditorRoot::new("help_browser");
        let model = fdd1_model(&root);
        let path = root.game("edge", &format!("meta.title = Edge\nmachine = {model}\n"));
        let mut editor =
            Editor::open(&root.host(), root.system(), &path).expect("the editor opens");
        let mut harness = GuiHarness::new(960, 600);
        show(&mut harness, &mut editor);
        harness.input.text = "!".to_owned();
        show(&mut harness, &mut editor);
        for _ in 0..2 {
            harness.input.navigation = Some(Navigation::NextTab);
            show(&mut harness, &mut editor);
        }
        assert_eq!(editor.current_section(), EditorSection::Media);
        harness.state.set_focus(field_id("fdd1").child("add"));
        show(&mut harness, &mut editor);
        assert_eq!(shown_help(&mut harness, &mut editor), Some("fdd1"));
        harness.input.confirm = true;
        show(&mut harness, &mut editor);
        assert!(editor.browser.is_some());
        assert_eq!(shown_help(&mut harness, &mut editor), Some("fdd1"));
        harness.input.cancel = true;
        show(&mut harness, &mut editor);
        assert!(editor.browser.is_none());
        assert_eq!(shown_help(&mut harness, &mut editor), Some("fdd1"));

        harness.input.cancel = true;
        show(&mut harness, &mut editor);
        assert_eq!(harness.state.open_modals(), 1);
        assert_eq!(shown_help(&mut harness, &mut editor), None);
    }

    /// Verifies that the help area of the floppy drive 2 draws the glyphs of its own help text.
    /// The help texts of the floppy drives 1 and 2 have the same length and differ in the
    /// number of the drive.
    #[test]
    #[cfg(any(
        feature = "pc98",
        feature = "pc88",
        feature = "pc88va",
        feature = "pc60",
        feature = "towns",
        feature = "x1",
        feature = "fm7",
        feature = "x68k",
        feature = "at"
    ))]
    fn the_help_area_tells_the_floppy_drives_apart() {
        use machine_factory::config::settings::{MODEL_CHOICES, find_setting};

        use crate::editor::test_support::shaped_help;

        let drives = ["fdd1", "fdd2"].map(|key| find_setting(key).expect("a table key"));
        let (system, model) = compiled_systems()
            .find_map(|system| {
                let target = MachineSelection::of_system(system)?.target();
                MODEL_CHOICES
                    .iter()
                    .map(|choice| choice.value)
                    .find(|model| drives.iter().all(|drive| drive.is_read_by(target, model)))
                    .map(|model| (system, model))
            })
            .expect("a compiled system reads two floppy drives");
        let root = EditorRoot::with_system("help_drives", system.name);
        let path = root.game("edge", &format!("meta.title = Edge\nmachine = {model}\n"));
        let mut editor =
            Editor::open(&root.host(), root.system(), &path).expect("the editor opens");
        let mut harness = GuiHarness::new(960, 600);
        let [first, second] =
            harness.frame(|context| drives.map(|drive| shaped_help(context, drive)));
        assert_eq!(first.len(), second.len());
        assert_ne!(first, second);
        show(&mut harness, &mut editor);
        for _ in 0..2 {
            harness.input.navigation = Some(Navigation::NextTab);
            show(&mut harness, &mut editor);
        }
        assert_eq!(editor.current_section(), EditorSection::Media);
        for (key, glyphs) in [("fdd2", &second), ("fdd1", &first)] {
            harness.state.set_focus(field_id(key).child("add"));
            show(&mut harness, &mut editor);
            assert_eq!(&help_glyphs(&harness), glyphs, "{key}");
            assert_eq!(shown_help(&mut harness, &mut editor), Some(key));
        }
    }

    /// Verifies that the arrows walk the focus through every row of the ROMs section of the
    /// settings page, that each focused row stays inside the form above the help area, and that
    /// the help area shows the help of each row.
    #[test]
    fn the_focused_row_stays_visible_above_the_help_area() {
        let test = SettingsRoot::new("help_visible", Some(""));
        let host = test.host(&[]);
        let mut editor = settings_page(&host, &statuses_with(&host, RomStatus::NotFound));
        let mut harness = GuiHarness::new(960, 600);
        show(&mut harness, &mut editor);
        harness.input.navigation = Some(Navigation::NextTab);
        show(&mut harness, &mut editor);
        assert_eq!(editor.current_section(), EditorSection::Roms);
        let form = form_area(&harness.theme, SCREEN);
        let help = help_area(&harness.theme, SCREEN);
        assert!(form.bottom() < help.y);
        let keys: Vec<&str> = editor
            .fields
            .iter()
            .filter(|field| field.section == EditorSection::Roms)
            .map(|field| field.key())
            .collect();
        for (row, key) in keys.iter().enumerate() {
            if row > 0 {
                harness.input.navigation = Some(Navigation::Down);
                show(&mut harness, &mut editor);
            }
            assert_eq!(shown_help(&mut harness, &mut editor), Some(*key));
            let area = harness
                .state
                .text_input_area()
                .expect("a ROM row holds a path field");
            assert!(
                area.y >= form.y - 0.5 && area.bottom() <= form.bottom() + 0.5,
                "the row of {key} lies outside the form: {area:?}"
            );
        }
    }

    /// Verifies that a focus change wraps the help text once, that a steady frame wraps
    /// nothing, and that a catalog switch refills the help area in the next frame.
    #[test]
    fn a_focus_change_and_a_catalog_switch_refill_the_help_area() {
        /// The German help text of the first row.
        const GERMAN_HELP: &str = "Kurz.";
        let test = SettingsRoot::new("help_refill", Some(""));
        let mut harness = GuiHarness::new(960, 600);
        let (mut editor, [first, second]) = video_section(&test, &mut harness);
        let english = help_glyphs(&harness);
        let counted = |harness: &mut GuiHarness, editor: &mut Editor| {
            harness
                .counted_frame(|context| {
                    editor.show(context, SCREEN);
                })
                .0
        };
        harness.state.set_focus(field_id(second));
        assert_eq!(counted(&mut harness, &mut editor).wraps, 1);
        assert_eq!(shown_help(&mut harness, &mut editor), Some(second));
        harness.state.set_focus(field_id(first));
        assert_eq!(counted(&mut harness, &mut editor).wraps, 1);
        assert_eq!(shown_help(&mut harness, &mut editor), Some(first));
        assert_eq!(counted(&mut harness, &mut editor).wraps, 0);

        harness.set_catalog(LanguageCatalog::Loaded(Box::new(test_catalog(&format!(
            "[setting.{first}]\nhelp = {GERMAN_HELP}\n"
        )))));
        assert_eq!(counted(&mut harness, &mut editor).wraps, 1);
        assert_eq!(help_glyphs(&harness).len(), GERMAN_HELP.chars().count());
        assert_eq!(shown_help(&mut harness, &mut editor), Some(first));
        harness.set_catalog(LanguageCatalog::English);
        assert_eq!(shown_help(&mut harness, &mut editor), Some(first));
        assert_eq!(help_glyphs(&harness), english);
    }

    /// Verifies that the help text of every setting row of every compiled system and of the
    /// settings page fits the help area of a 960 x 600 window in every language of the
    /// repository.
    #[test]
    fn every_help_text_fits_at_the_minimum_window_size() {
        let mut harness = GuiHarness::new(960, 600);
        let width = help_area(&harness.theme, SCREEN).width;
        let mut settings: Vec<&'static Setting> = Vec::new();
        let rows = compiled_systems()
            .flat_map(|system| {
                let target = MachineSelection::of_system(system)
                    .expect("compiled system")
                    .target();
                form_fields(target)
            })
            .chain(host_fields());
        for (_, source) in rows {
            if let FieldSource::Setting(setting) = source
                && !settings.iter().any(|known| known.key == setting.key)
            {
                settings.push(setting);
            }
        }
        for catalog in repository_catalogs() {
            let measured = harness.frame(|context| {
                let style = context.theme.small();
                let line = wrapped_line_height(&style);
                settings
                    .iter()
                    .map(|setting| {
                        let text = setting_help(catalog, setting);
                        let height =
                            WrappedText::new().height(context, &style, text, width, usize::MAX);
                        (setting.key, (height / line).round() as usize, text)
                    })
                    .collect::<Vec<_>>()
            });
            for (key, lines, text) in measured {
                assert!(
                    lines <= HELP_MAXIMUM_LINES,
                    "the help of {key} takes {lines} lines in {}: {text}",
                    catalog.tag()
                );
            }
        }
    }
}
