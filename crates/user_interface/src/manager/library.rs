//! The library screen of the manager: the sidebar with the systems, the header with the title
//! and the filter, the toolbar, the game table and the status line.

use gui::{
    Rect,
    ui::{
        ActionButton, Button, ButtonKind, EmptyHint, HEADER_HEIGHT, Icon, Navigation, Shortcut,
        StatusEnd, TextBuffer, TextField, UiContext, UiId, status_line,
    },
};

use super::{
    FILTER_ID, Manager, ManagerMessage, ManagerOverlay, SETTINGS_ID,
    counts::{GameCounts, write_counts},
    entries::TableEntry,
    filter::Filter,
    game_table::{self, GAME_TABLE_ID, GameRows, RowErrors, show_game_table},
    hints::{LibraryFocus, library_hints, status_end},
    rows::GameRow,
    rows_at,
    sidebar::{SYSTEM_DEFAULTS_ID, SYSTEM_LIST_ID, SidebarTexts, show_sidebar},
    toolbar::{
        ADD_ID, DELETE_ID, DUPLICATE_ID, EDIT_ID, LAUNCH_ID, REFRESH_ID, ToolbarState, show_toolbar,
    },
};
use crate::{
    config::Diagnostic,
    texts::{hints_in, library},
};

/// The width of the filter field in logical units.
const FILTER_WIDTH: f32 = 260.0;

/// The texts of the library that refill only when their data or the
/// catalog changes.
#[derive(Debug, Clone, Default)]
pub(super) struct LibraryTexts {
    /// The counts beside the system label.
    counts: TextBuffer<GameCounts>,
    /// The title of the hint of a system without games, for the index of the system.
    empty_title: TextBuffer<usize>,
    /// The detail of the hint of a system without games, for the index of
    /// the system and its folder.
    empty_detail: TextBuffer<(usize, String)>,
    /// The folder of the selected system at the end of the status line.
    folder: String,
    /// The error of the selected file with an error at the end of the
    /// status line, for its shown path and its error.
    located_error: TextBuffer<(String, Diagnostic)>,
    /// The texts of the errors of the rows of the game table.
    row_errors: RowErrors,
    /// The texts of the sidebar.
    sidebar: SidebarTexts,
}

impl Manager {
    /// Draws the sidebar into `area` and asks for a system switch on a new
    /// selection of the system list or on a shoulder button. The system
    /// defaults button asks for the editor on the system defaults of the
    /// selected system.
    pub(super) fn show_systems(
        &mut self,
        context: &mut UiContext,
        area: Rect,
        messages: &mut Vec<ManagerMessage>,
    ) {
        let count = self.systems.len();
        if count > 0 {
            let selected = self.view.systems.selected.min(count - 1);
            let switched = match context.tab_navigation() {
                Some(Navigation::PreviousTab) => Some((selected + count - 1) % count),
                Some(Navigation::NextTab) => Some((selected + 1) % count),
                Some(
                    Navigation::Up
                    | Navigation::Down
                    | Navigation::Left
                    | Navigation::Right
                    | Navigation::PageUp
                    | Navigation::PageDown
                    | Navigation::Home
                    | Navigation::End
                    | Navigation::Next
                    | Navigation::Previous,
                )
                | None => None,
            };
            if let Some(index) = switched {
                messages.push(ManagerMessage::SelectSystem(index));
            }
        }
        let response = show_sidebar(
            context,
            area,
            &self.systems,
            &mut self.view.systems,
            &mut self.texts.sidebar,
        );
        if response.list.selection_changed {
            messages.push(ManagerMessage::SelectSystem(self.view.systems.selected));
        }
        if response.list.activated.is_some() && self.has_entries() {
            context.set_focus(UiId::from(GAME_TABLE_ID));
        }
        if response.system_defaults {
            messages.push(ManagerMessage::OpenSystemDefaults);
        }
    }

    /// Draws the header with the filter, the toolbar and the game table into
    /// `area`, and asks for the actions of the input on the selected row.
    pub(super) fn show_main(
        &mut self,
        context: &mut UiContext,
        area: Rect,
        messages: &mut Vec<ManagerMessage>,
    ) {
        let theme = context.theme;
        let inner = Rect::new(
            area.x + theme.padding,
            area.y,
            (area.width - theme.padding * 2.0).max(0.0),
            (area.height - theme.padding).max(0.0),
        );
        let (header, rest) = inner.cut_top(HEADER_HEIGHT);
        let (toolbar_area, rest) = rest.cut_top(theme.control_height);
        let table_area = rest.cut_top(theme.gap * 1.5).1;

        let filter_id = UiId::from(FILTER_ID);
        let controls_y = header.y + (header.height - theme.control_height) * 0.5;
        let settings_area = Rect::new(
            header.right() - theme.control_height.min(header.width),
            controls_y,
            theme.control_height.min(header.width),
            theme.control_height,
        );
        let filter_width = FILTER_WIDTH.min((settings_area.x - theme.gap - header.x).max(0.0));
        let filter_area = Rect::new(
            settings_area.x - theme.gap - filter_width,
            controls_y,
            filter_width,
            theme.control_height,
        );
        let catalog = context.catalog;
        let filter = TextField::new(filter_id, catalog.text(library::FILTER_PLACEHOLDER)).show(
            context,
            filter_area,
            &mut self.view.filter,
        );
        if filter.changed {
            messages.push(ManagerMessage::FilterChanged);
        }
        if filter.submitted && self.has_entries() {
            context.set_focus(UiId::from(GAME_TABLE_ID));
        }
        let settings = Button::new(SETTINGS_ID, "")
            .icon(Icon::Settings)
            .kind(ButtonKind::Quiet)
            .default_choice(false)
            .show(context, settings_area);
        self.show_title(
            context,
            header
                .cut_right(header.right() - filter_area.x + theme.gap)
                .1,
        );

        let selected = self.selected_row();
        let state = ToolbarState {
            addable: self.current().is_some(),
            launchable: selected.is_some_and(GameRow::launchable),
            deletable: selected.is_some(),
        };
        let toolbar = show_toolbar(context, toolbar_area, state);
        if toolbar.refresh || context.shortcut() == Some(Shortcut::F5) {
            messages.push(ManagerMessage::Refresh);
        }
        if settings
            || context.shortcut() == Some(Shortcut::F10)
            || context.action_button() == Some(ActionButton::Start)
        {
            messages.push(ManagerMessage::OpenSettings);
            return;
        }

        let selected_system = self.entries_system;
        let hint = match self.systems.get(selected_system) {
            Some(entry) if entry.rows.is_empty() => {
                let label = entry.system.label;
                let title =
                    self.texts
                        .empty_title
                        .text(context, selected_system, |catalog, _, out| {
                            let _ = catalog.write(out, library::no_games(&label));
                        });
                let detail = self.texts.empty_detail.text(
                    context,
                    (selected_system, entry.folder.as_str()),
                    |catalog, (_, folder), out| {
                        let _ = catalog.write(out, library::add_games(&folder));
                    },
                );
                EmptyHint::Centered { title, detail }
            }
            Some(_) => EmptyHint::Top(catalog.text(library::NO_MATCH)),
            None => EmptyHint::Centered {
                title: "",
                detail: "",
            },
        };
        let rows = rows_at(&self.systems, self.entries_system);
        let response = show_game_table(
            context,
            table_area,
            &self.entries,
            &self.row_kinds,
            GameRows {
                rows,
                errors: &mut self.texts.row_errors,
            },
            &mut self.view.table,
            hint,
        );
        if response.sort_changed {
            messages.push(ManagerMessage::SortChanged);
        } else if response.selection_changed {
            messages.push(ManagerMessage::SelectionChanged);
        }

        let shortcut = context.shortcut();
        let delete_shortcut = shortcut == Some(Shortcut::Delete);
        let edit = toolbar.edit
            || shortcut == Some(Shortcut::F2)
            || context.action_button() == Some(ActionButton::X);
        let duplicate = toolbar.duplicate || shortcut == Some(Shortcut::ControlD);
        if (toolbar.add || shortcut == Some(Shortcut::Insert)) && state.addable {
            messages.push(ManagerMessage::AddGame);
            return;
        }
        let activated = response.activated.is_some();
        let Some(launchable) = self.selected_row().map(GameRow::launchable) else {
            return;
        };
        let message = if launchable && (activated || toolbar.launch) {
            ManagerMessage::Launch
        } else if launchable && edit {
            ManagerMessage::Edit
        } else if launchable && duplicate {
            ManagerMessage::Duplicate
        } else if delete_shortcut || toolbar.delete || activated {
            ManagerMessage::AskDelete
        } else {
            return;
        };
        messages.push(message);
    }

    /// Draws the label of the system of the entries and its game count into
    /// `area`.
    fn show_title(&mut self, context: &mut UiContext, area: Rect) {
        let Some(entry) = self.systems.get(self.entries_system) else {
            return;
        };
        let theme = context.theme;
        let title_style = theme.title();
        let baseline = context.centered_baseline(area, &title_style);
        context.text(area.x, baseline, &title_style, entry.system.label);
        let title_width = context.measure(&title_style, entry.system.label);
        let filtering = !Filter::new(&self.view.filter).is_empty();
        let shown_games = self
            .entries
            .iter()
            .filter(|entry| matches!(entry, TableEntry::Game(_)))
            .count();
        let shown_errors = self
            .entries
            .iter()
            .filter(|entry| matches!(entry, TableEntry::Broken(_)))
            .count();
        let counts = GameCounts {
            shown_games,
            games: entry.game_count(),
            shown_errors,
            errors: entry.error_count(),
            filtering,
        };
        let count = self
            .texts
            .counts
            .text(context, counts, |catalog, counts, out| {
                write_counts(catalog, counts, out);
            });
        context.text(
            area.x + title_width + theme.gap,
            baseline,
            &theme.small(),
            count,
        );
    }

    /// The widget of the library that holds the focus.
    fn library_focus(&self, context: &UiContext) -> LibraryFocus {
        let focused = |id: &str| context.has_focus(UiId::from(id));
        if focused(GAME_TABLE_ID) {
            LibraryFocus::Table
        } else if context.has_focus(game_table::header_id()) {
            LibraryFocus::Header
        } else if focused(SYSTEM_LIST_ID) {
            LibraryFocus::SystemList
        } else if focused(SYSTEM_DEFAULTS_ID) {
            LibraryFocus::SystemDefaults
        } else if focused(FILTER_ID) {
            LibraryFocus::Filter {
                has_text: !self.view.filter.is_empty(),
            }
        } else if focused(SETTINGS_ID) {
            LibraryFocus::Settings
        } else if focused(LAUNCH_ID) {
            LibraryFocus::Launch
        } else if focused(ADD_ID) {
            LibraryFocus::Add
        } else if focused(EDIT_ID) {
            LibraryFocus::Edit
        } else if focused(DUPLICATE_ID) {
            LibraryFocus::Duplicate
        } else if focused(DELETE_ID) {
            LibraryFocus::Delete
        } else if focused(REFRESH_ID) {
            LibraryFocus::Refresh
        } else {
            LibraryFocus::Nothing
        }
    }

    /// Draws the status line into `area`. With `dialog_open` the hints come
    /// from the open dialog, else from the focused widget of the library.
    pub(super) fn show_status_line(
        &mut self,
        context: &mut UiContext,
        area: Rect,
        dialog_open: bool,
    ) {
        let catalog = context.catalog;
        if dialog_open && let ManagerOverlay::Delete(request) = &self.overlay {
            let hints = hints_in(catalog, &request.hints());
            status_line(context, area, &hints, StatusEnd::Path(&request.shown_path));
            return;
        }
        let mut folder = std::mem::take(&mut self.texts.folder);
        folder.clear();
        if let Some(entry) = self.current() {
            folder.push_str(&entry.folder);
            folder.push('/');
        }
        let mut located = std::mem::take(&mut self.texts.located_error);
        let selected = self.selected_row();
        let hints = match &self.overlay {
            ManagerOverlay::Error(dialog) if dialog_open => {
                hints_in(catalog, &dialog.hints(context))
            }
            ManagerOverlay::None | ManagerOverlay::Delete(_) | ManagerOverlay::Error(_) => {
                library_hints(catalog, self.library_focus(context), selected)
            }
        };
        let end = status_end(
            catalog,
            context.catalog_generation,
            selected,
            &folder,
            &mut located,
        );
        status_line(context, area, &hints, end);
        self.texts.folder = folder;
        self.texts.located_error = located;
    }
}

#[cfg(test)]
mod tests {
    use gui::ui::EditCommand;
    use machine_factory::{config::systems::compiled_systems, rom_status::RomStatus};

    use super::*;
    use crate::{
        config::LaunchFailure,
        gui_test_support::{GuiHarness, PassCounts},
        manager::{
            ShellEffect,
            delete::DELETE_LAYER_ID,
            test_support::{
                TestRoot, delete_open, first_system, frame, games, sample_root, select, settle,
                steady_frame, titles,
            },
        },
        texts::{LanguageCatalog, test_catalog},
    };

    /// Verifies that confirm launches the selected game and that confirm on a file with an error asks to delete it.
    #[test]
    fn confirm_launches_and_an_error_file_asks_for_its_delete() {
        let test = TestRoot::new("confirm");
        let system = first_system();
        test.game(&format!("{}/a.conf", system.name), "meta.title = A\n");
        test.game(&format!("{}/b.conf", system.name), "machine = FOOBAR\n");
        let mut manager = test.manager();
        assert_eq!(titles(&manager), ["A", "|", "b.conf"]);
        let mut harness = GuiHarness::minimum();
        settle(&mut harness, &mut manager);
        harness.input.confirm = true;
        let effects = frame(&mut harness, &mut manager);
        let [ShellEffect::Launch(launch)] = effects.as_slice() else {
            panic!("confirm launches: {effects:?}");
        };
        assert_eq!(launch.title, "A");
        harness.input.navigation = Some(Navigation::Down);
        settle(&mut harness, &mut manager);
        assert_eq!(manager.view.table.list.selected, 2);
        harness.input.confirm = true;
        assert!(settle(&mut harness, &mut manager).is_empty());
        assert!(delete_open(&manager));
        assert_eq!(harness.state.open_modals(), 1);
        assert_eq!(
            harness.state.focused(),
            Some(UiId::from(DELETE_LAYER_ID).child("cancel"))
        );
    }

    /// Verifies that the Delete key removes a character of the filter and opens no delete question while the filter holds the focus, that Ctrl+F and Y focus the filter and that confirm in the filter gives the focus to the table without a launch.
    #[test]
    fn delete_edits_the_filter_while_it_holds_the_focus() {
        let test = TestRoot::new("shortcuts");
        games(&test, &["a", "b"]);
        let mut manager = test.manager();
        let mut harness = GuiHarness::minimum();
        settle(&mut harness, &mut manager);
        harness.input.shortcut = Some(Shortcut::ControlF);
        settle(&mut harness, &mut manager);
        assert_eq!(harness.state.focused(), Some(UiId::from(FILTER_ID)));
        harness.input.text = "ab".to_owned();
        settle(&mut harness, &mut manager);
        harness.input.navigation = Some(Navigation::Left);
        settle(&mut harness, &mut manager);
        harness.input.edit = Some(EditCommand::DeleteForward);
        settle(&mut harness, &mut manager);
        assert_eq!(manager.view.filter, "a");
        assert!(!delete_open(&manager));
        assert_eq!(harness.state.open_modals(), 0);
        assert_eq!(harness.state.focused(), Some(UiId::from(FILTER_ID)));
        harness.input.cancel = true;
        settle(&mut harness, &mut manager);
        assert_eq!(manager.view.filter, "");

        harness.state.set_focus(UiId::from(GAME_TABLE_ID));
        settle(&mut harness, &mut manager);
        harness.input.action = Some(ActionButton::Y);
        settle(&mut harness, &mut manager);
        assert_eq!(harness.state.focused(), Some(UiId::from(FILTER_ID)));
        harness.input.text = "B".to_owned();
        settle(&mut harness, &mut manager);
        assert_eq!(titles(&manager), ["b"]);
        harness.input.confirm = true;
        assert!(frame(&mut harness, &mut manager).is_empty());
        assert!(harness.state.take_pass_request());
        assert!(settle(&mut harness, &mut manager).is_empty());
        assert_eq!(harness.state.focused(), Some(UiId::from(GAME_TABLE_ID)));
        harness.input.action = Some(ActionButton::Y);
        settle(&mut harness, &mut manager);
        assert_eq!(harness.state.focused(), Some(UiId::from(FILTER_ID)));
        harness.input.cancel = true;
        settle(&mut harness, &mut manager);
        assert_eq!(manager.view.filter, "");
        assert_eq!(titles(&manager), ["a", "b"]);
        assert_eq!(harness.state.focused(), Some(UiId::from(FILTER_ID)));
    }

    /// Verifies that a switch from the table into a system without games gives the focus to Add.
    #[test]
    fn a_switch_into_an_empty_system_focuses_add() {
        if compiled_systems().count() < 2 {
            return;
        }
        let test = TestRoot::new("empty_switch");
        games(&test, &["a"]);
        let mut manager = test.manager();
        let mut harness = GuiHarness::minimum();
        settle(&mut harness, &mut manager);
        assert_eq!(harness.state.focused(), Some(UiId::from(GAME_TABLE_ID)));
        harness.input.navigation = Some(Navigation::NextTab);
        settle(&mut harness, &mut manager);
        assert_eq!(manager.view.systems.selected, 1);
        assert_eq!(harness.state.focused(), Some(UiId::from(ADD_ID)));
        harness.input.navigation = Some(Navigation::PreviousTab);
        settle(&mut harness, &mut manager);
        assert_eq!(manager.view.systems.selected, 0);
        assert_eq!(harness.state.focused(), Some(UiId::from(ADD_ID)));
    }

    /// Verifies that the system switch, the filter keys, Delete, F5, F10 and Start do nothing while the delete question or an error dialog is open.
    #[test]
    fn the_library_keys_do_nothing_under_a_dialog() {
        let test = TestRoot::new("keys_under_dialog");
        games(&test, &["a", "b"]);
        let mut manager = test.manager();
        let mut harness = GuiHarness::minimum();
        settle(&mut harness, &mut manager);
        let keys = |harness: &mut GuiHarness, manager: &mut Manager| {
            let focused = harness.state.focused();
            for navigation in [Navigation::NextTab, Navigation::PreviousTab] {
                harness.input.navigation = Some(navigation);
                settle(harness, manager);
            }
            harness.input.shortcut = Some(Shortcut::ControlF);
            settle(harness, manager);
            harness.input.action = Some(ActionButton::Y);
            settle(harness, manager);
            harness.input.edit = Some(EditCommand::DeleteForward);
            settle(harness, manager);
            harness.input.shortcut = Some(Shortcut::F10);
            settle(harness, manager);
            assert!(manager.editor().is_none());
            harness.input.action = Some(ActionButton::Start);
            settle(harness, manager);
            assert!(manager.editor().is_none());
            let files = manager.rows().len();
            games(&test, &["new"]);
            harness.input.shortcut = Some(Shortcut::F5);
            settle(harness, manager);
            test.remove(&format!("{}/new.conf", first_system().name));
            assert_eq!(manager.rows().len(), files);
            assert_eq!(manager.view.systems.selected, 0);
            assert_eq!(harness.state.focused(), focused);
            assert_eq!(harness.state.open_modals(), 1);
        };

        harness.input.edit = Some(EditCommand::DeleteForward);
        settle(&mut harness, &mut manager);
        assert!(delete_open(&manager));
        keys(&mut harness, &mut manager);
        assert!(delete_open(&manager));
        harness.input.cancel = true;
        settle(&mut harness, &mut manager);
        assert!(!delete_open(&manager));

        let launch = manager.launch(&manager.rows()[0].clone());
        let layer = manager.launch_failed(&launch, &LaunchFailure::missing_image("/a.d88"));
        harness.state.open_layer(layer);
        settle(&mut harness, &mut manager);
        keys(&mut harness, &mut manager);
        assert!(manager.dialog().is_some());
        assert!(!delete_open(&manager));
    }

    /// Verifies that a sort from the column header keeps the selected game.
    #[test]
    fn a_header_sort_keeps_the_selected_game() {
        let test = TestRoot::new("sort_keeps");
        games(&test, &["a", "b", "c"]);
        let mut manager = test.manager();
        let mut harness = GuiHarness::minimum();
        settle(&mut harness, &mut manager);
        harness.input.navigation = Some(Navigation::Down);
        settle(&mut harness, &mut manager);
        let selected = format!("{}/b.conf", first_system().name);
        assert_eq!(manager.view.selected.as_deref(), Some(selected.as_str()));
        harness.state.set_focus(game_table::header_id());
        settle(&mut harness, &mut manager);
        harness.input.confirm = true;
        assert!(frame(&mut harness, &mut manager).is_empty());
        settle(&mut harness, &mut manager);
        assert_eq!(titles(&manager), ["c", "b", "a"]);
        assert_eq!(manager.view.selected.as_deref(), Some(selected.as_str()));
        assert_eq!(manager.view.table.list.selected, 1);
        harness.input.navigation = Some(Navigation::Right);
        settle(&mut harness, &mut manager);
        harness.input.confirm = true;
        settle(&mut harness, &mut manager);
        assert_eq!(manager.view.selected.as_deref(), Some(selected.as_str()));
    }

    /// Verifies that the Refresh button and F5 pick up a changed ROM status and a new game file and keep the view.
    #[test]
    fn refresh_and_f5_rescan_and_keep_the_view() {
        let test = TestRoot::new("refresh");
        games(&test, &["a", "b", "c"]);
        let mut manager = test.manager();
        let actual = manager.systems[0].rom_status;
        let stale = match actual {
            RomStatus::Found => RomStatus::Missing,
            RomStatus::Partial
            | RomStatus::Missing
            | RomStatus::NotFound
            | RomStatus::Unsupported => RomStatus::Found,
        };
        let mut harness = GuiHarness::minimum();
        settle(&mut harness, &mut manager);
        harness.input.navigation = Some(Navigation::Down);
        settle(&mut harness, &mut manager);
        let selected = format!("{}/b.conf", first_system().name);
        assert_eq!(manager.view.selected.as_deref(), Some(selected.as_str()));

        for press in [false, true] {
            manager.systems[0].rom_status = stale;
            let name = if press { "e" } else { "d" };
            games(&test, &[name]);
            let view = manager.view.clone();
            if press {
                harness.state.set_focus(UiId::from(REFRESH_ID));
                settle(&mut harness, &mut manager);
                harness.input.confirm = true;
            } else {
                harness.input.shortcut = Some(Shortcut::F5);
            }
            settle(&mut harness, &mut manager);
            assert_eq!(manager.systems[0].rom_status, actual);
            assert!(titles(&manager).contains(&name.to_owned()), "{name}");
            assert_eq!(manager.view.selected, view.selected);
            assert_eq!(manager.view.filter, view.filter);
            assert_eq!(manager.view.systems, view.systems);
        }
        assert_eq!(manager.view.selected.as_deref(), Some(selected.as_str()));
        assert_eq!(manager.view.table.list.selected, 1);
    }

    /// Verifies that F5 is inactive while the filter holds the focus.
    #[test]
    fn f5_is_inactive_in_the_filter() {
        let test = TestRoot::new("refresh_filter");
        games(&test, &["a"]);
        let mut manager = test.manager();
        let mut harness = GuiHarness::minimum();
        settle(&mut harness, &mut manager);
        harness.input.shortcut = Some(Shortcut::ControlF);
        settle(&mut harness, &mut manager);
        assert_eq!(harness.state.focused(), Some(UiId::from(FILTER_ID)));
        games(&test, &["b"]);
        harness.input.shortcut = Some(Shortcut::F5);
        settle(&mut harness, &mut manager);
        assert_eq!(titles(&manager), ["a"]);
        harness.state.set_focus(UiId::from(GAME_TABLE_ID));
        settle(&mut harness, &mut manager);
        harness.input.shortcut = Some(Shortcut::F5);
        settle(&mut harness, &mut manager);
        assert_eq!(titles(&manager), ["a", "b"]);
    }

    /// Verifies that the shoulder buttons and Ctrl+PgUp and Ctrl+PgDn switch the system and wrap around.
    #[test]
    fn the_shoulder_buttons_switch_the_system() {
        let test = TestRoot::new("switch");
        let mut manager = test.manager();
        let count = manager.systems.len();
        let mut harness = GuiHarness::minimum();
        settle(&mut harness, &mut manager);
        assert_eq!(manager.view.systems.selected, 0);
        harness.input.navigation = Some(Navigation::NextTab);
        settle(&mut harness, &mut manager);
        assert_eq!(manager.view.systems.selected, 1 % count);
        assert_eq!(
            manager.view.system,
            Some(manager.systems[1 % count].system.name)
        );
        harness.input.navigation = Some(Navigation::PreviousTab);
        settle(&mut harness, &mut manager);
        harness.input.navigation = Some(Navigation::PreviousTab);
        settle(&mut harness, &mut manager);
        assert_eq!(manager.view.systems.selected, count - 1);

        harness.input.navigation = Some(Navigation::NextTab);
        settle(&mut harness, &mut manager);
        assert_eq!(manager.view.systems.selected, 0);
    }

    /// Verifies that a new selection in the system list switches the system and that confirm gives the focus to its games without a launch.
    #[test]
    fn the_system_list_switches_the_system() {
        let test = TestRoot::new("system_list");
        let second = compiled_systems().nth(1);
        let Some(second) = second else {
            return;
        };
        test.game(&format!("{}/edge.conf", second.name), "meta.title = Edge\n");
        let mut manager = test.manager();
        let mut harness = GuiHarness::minimum();
        settle(&mut harness, &mut manager);
        assert_eq!(harness.state.focused(), Some(UiId::from(ADD_ID)));
        harness.state.set_focus(UiId::from(SYSTEM_LIST_ID));
        settle(&mut harness, &mut manager);
        harness.input.navigation = Some(Navigation::Down);
        settle(&mut harness, &mut manager);
        assert_eq!(manager.view.system, Some(second.name));
        assert_eq!(titles(&manager), ["Edge"]);
        harness.input.confirm = true;
        assert!(frame(&mut harness, &mut manager).is_empty());
        assert!(harness.state.take_pass_request());
        assert!(settle(&mut harness, &mut manager).is_empty());
        assert_eq!(harness.state.focused(), Some(UiId::from(GAME_TABLE_ID)));
    }

    /// Verifies that a new selection in the system list draws the pass of the switch with the
    /// table of the system before it, and that the table of the new system follows.
    #[test]
    fn a_system_list_switch_draws_the_table_of_the_system_before_in_its_pass() {
        let Some(second) = compiled_systems().nth(1) else {
            return;
        };
        let test = TestRoot::new("system_list_switch");
        games(&test, &["a", "b", "c"]);
        test.game(&format!("{}/edge.conf", second.name), "meta.title = Edge\n");
        let mut manager = test.manager();
        let mut harness = GuiHarness::minimum();
        settle(&mut harness, &mut manager);
        harness.input.navigation = Some(Navigation::End);
        settle(&mut harness, &mut manager);
        assert_eq!(manager.view.table.list.selected, 2);
        harness.state.set_focus(UiId::from(SYSTEM_LIST_ID));
        settle(&mut harness, &mut manager);
        harness.input.navigation = Some(Navigation::Down);
        assert!(frame(&mut harness, &mut manager).is_empty());
        assert!(harness.state.take_pass_request());
        settle(&mut harness, &mut manager);
        assert_eq!(manager.view.system, Some(second.name));
        assert_eq!(titles(&manager), ["Edge"]);
        assert_eq!(manager.view.table.list.selected, 0);
    }

    /// The German texts of the count of the library and of the header line of the editor.
    const GERMAN_COUNTS: &str = "[library]\ngames.one = {count} Spiel\ngames.other = {count} Spiele\n\
                                 [editor]\ngame-line = {system} Spiel\n";

    /// Verifies that a switch of the catalog gives the buffered texts of the library and of the
    /// editor the new language in the next frame.
    #[test]
    fn a_catalog_switch_refills_the_buffered_texts_in_the_next_frame() {
        let german = || {
            let german = test_catalog(GERMAN_COUNTS);
            LanguageCatalog::Loaded(Box::new(german))
        };
        let test = TestRoot::new("catalog_switch");
        games(&test, &["A", "B"]);
        let mut manager = test.manager();
        let mut harness = GuiHarness::minimum();
        settle(&mut harness, &mut manager);
        frame(&mut harness, &mut manager);
        assert_eq!(manager.texts.counts.last_text(), Some("2 games"));
        harness.set_catalog(german());
        frame(&mut harness, &mut manager);
        assert_eq!(manager.texts.counts.last_text(), Some("2 Spiele"));

        harness.set_catalog(LanguageCatalog::English);
        harness.state.set_focus(UiId::from(GAME_TABLE_ID));
        harness.input.shortcut = Some(Shortcut::F2);
        settle(&mut harness, &mut manager);
        let label = manager
            .current()
            .expect("the library selects a game")
            .system
            .label;
        let line = |manager: &Manager| {
            manager
                .editor()
                .expect("F2 opens the editor")
                .cached_header_line()
                .expect("the editor draws its header line")
                .to_owned()
        };
        assert_eq!(line(&manager), format!("{label} game"));
        harness.set_catalog(german());
        frame(&mut harness, &mut manager);
        assert_eq!(line(&manager), format!("{label} Spiel"));
    }

    /// Verifies that a steady frame of the library, the second pass with an unchanged state,
    /// allocates nothing and fills no text again.
    #[test]
    fn a_steady_frame_of_the_library_allocates_nothing() {
        let test = TestRoot::new("steady_library");
        sample_root(&test);
        let mut manager = test.manager();
        let mut harness = GuiHarness::minimum();
        settle(&mut harness, &mut manager);
        assert_eq!(
            steady_frame(&mut harness, &mut manager),
            PassCounts::NONE,
            "a steady frame of the library"
        );
    }

    /// Verifies that a steady frame of the files with errors, with the error of the selected
    /// file at the end of the status line, allocates nothing and fills no text again, and that
    /// a language switch writes the errors in the new catalog.
    #[test]
    fn a_steady_frame_of_the_files_with_errors_allocates_nothing() {
        let test = TestRoot::new("steady_errors");
        let system = first_system();
        games(&test, &["Edge", "Brandish"]);
        test.game(
            &format!("{}/cyber-block.conf", system.name),
            "meta.title = Cyber Block\nmachine = X68040\n",
        );
        test.game(
            &format!("{}/nemesis-90.conf", system.name),
            "meta.title = Nemesis '90\nmachine = X68030\n",
        );
        let mut manager = test.manager();
        let mut harness = GuiHarness::minimum();
        select(&mut manager, system.name);
        harness.state.set_focus(UiId::from(GAME_TABLE_ID));
        settle(&mut harness, &mut manager);
        harness.input.navigation = Some(Navigation::End);
        settle(&mut harness, &mut manager);
        let selected = manager.selected_row().expect("a file with an error");
        assert!(!selected.launchable());
        assert_eq!(
            manager.texts.located_error.last_text(),
            Some(
                format!(
                    "config/{}/nemesis-90.conf line 2: unknown machine X68030",
                    system.name
                )
                .as_str()
            )
        );
        assert_eq!(
            steady_frame(&mut harness, &mut manager),
            PassCounts::NONE,
            "a steady frame of the files with errors"
        );
        harness.set_catalog(LanguageCatalog::Loaded(Box::new(test_catalog(
            "[diagnostic]\nunknown-machine = Maschine {value} unbekannt\n",
        ))));
        settle(&mut harness, &mut manager);
        assert_eq!(
            manager.texts.located_error.last_text(),
            Some(
                format!(
                    "config/{}/nemesis-90.conf line 2: Maschine X68030 unbekannt",
                    system.name
                )
                .as_str()
            )
        );
        let row_errors: Vec<&str> = manager
            .texts
            .row_errors
            .iter()
            .filter_map(TextBuffer::last_text)
            .collect();
        assert!(
            row_errors.contains(&"line 2: Maschine X68030 unbekannt"),
            "{row_errors:?}"
        );
    }
}
