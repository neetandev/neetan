//! The game manager: the library with the systems in a sidebar and the games
//! of the selected system in a table, the launch of a game, the editor of a
//! game file, the settings page, the delete of a game file and the error
//! dialogs.

mod counts;
mod delete;
mod editing;
mod entries;
mod error_dialog;
mod filter;
mod game_table;
mod hints;
mod library;
mod rows;
mod settings_page;
mod sidebar;
#[cfg(test)]
mod test_support;
mod toolbar;
mod view;

use std::path::PathBuf;

use common::{FlushError, debug, info};
use gui::{
    Rect,
    ui::{ActionButton, ConfirmResponse, RowKind, STATUS_LINE_HEIGHT, Shortcut, UiContext, UiId},
};
use machine_factory::config::systems::{System, compiled_systems};

use self::{
    delete::{DeleteOutcome, DeleteRequest},
    entries::{TableEntry, table_entries},
    error_dialog::{DialogAnswer, EditTarget},
    filter::Filter,
    game_table::GAME_TABLE_ID,
    library::LibraryTexts,
    rows::system_rows,
    sidebar::SystemEntry,
    toolbar::ADD_ID,
};
pub(crate) use self::{
    error_dialog::ErrorDialog,
    rows::{GameRow, shown_path},
    view::ManagerView,
};
use crate::{
    Result,
    catalog::Catalog,
    config::{
        EmulatorConfig, HostLayers, LaunchError, LaunchFailure, LaunchRequest, ManagerStart,
        build_configuration,
    },
    editor::Editor,
    input::KeyOverrides,
    rom_folders::{RomFolderStatus, check_all_folders},
    texts::LanguageCatalog,
};

/// The width of the sidebar in logical units.
const SIDEBAR_WIDTH: f32 = 240.0;

/// The id of the filter field.
const FILTER_ID: &str = "filter";

/// The id of the button that opens the settings page.
pub(crate) const SETTINGS_ID: &str = "settings";

/// A game that the user asked to start.
#[derive(Debug, Clone)]
pub(crate) struct Launch {
    /// The layers of the launch.
    pub(crate) request: LaunchRequest,
    /// The title of the game.
    pub(crate) title: String,
    /// The system of the game.
    pub(crate) system: &'static System,
    /// The machine model that the catalog gives the game.
    pub(crate) model: &'static str,
}

/// The widget that takes the focus in the next pass of the library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NextFocus {
    /// The game table, or the add button while the table has no entry.
    Start,
    /// The widget with this id.
    Widget(UiId),
}

/// The game manager.
pub(crate) struct Manager {
    /// The command line and the folders of the settings discovery.
    start: ManagerStart,
    /// The settings file and the command line flags.
    host: HostLayers,
    /// The config folder.
    config_folder: PathBuf,
    /// Every compiled system with its games and its ROM status.
    systems: Vec<SystemEntry>,
    /// The ROM status of every ROM folder from the check of the start or of
    /// the last Refresh.
    roms: Vec<RomFolderStatus>,
    /// The entries of the game table of the selected system.
    entries: Vec<TableEntry>,
    /// The row kinds of the entries in the game table. They refill with the
    /// entries and keep their buffer.
    row_kinds: Vec<RowKind>,
    /// The selection, the sort, the filter and the scroll positions.
    view: ManagerView,
    /// The open question before a delete.
    delete: Option<DeleteRequest>,
    /// The open error dialog.
    dialog: Option<ErrorDialog>,
    /// The open game editor. It takes the whole window while it is open.
    editor: Option<Editor>,
    /// The widget that takes the focus in the next pass of the library.
    next_focus: Option<NextFocus>,
    /// The game that runs.
    running: Option<Launch>,
    /// The catalog of a new language of the user interface after a save of
    /// the settings page.
    catalog_switch: Option<LanguageCatalog>,
    /// The texts of the library.
    texts: LibraryTexts,
}

impl Manager {
    /// Reads the settings file of `start`, checks the ROM folder of every
    /// compiled system and scans the catalog.
    pub(crate) fn new(start: ManagerStart) -> Result<Self> {
        let host = HostLayers::read(&start.request, &start.locations)?;
        let config_folder = host.config_folder();
        let roms = check_all_folders(&host);
        let systems = compiled_systems()
            .map(|system| SystemEntry::new(system, &host, &roms))
            .collect();
        let mut manager = Self {
            start,
            host,
            config_folder,
            systems,
            roms,
            entries: Vec::new(),
            row_kinds: Vec::new(),
            view: ManagerView::default(),
            delete: None,
            dialog: None,
            editor: None,
            next_focus: Some(NextFocus::Start),
            running: None,
            catalog_switch: None,
            texts: LibraryTexts::default(),
        };
        manager.rescan();
        info!("{}", manager.scan_summary());
        Ok(manager)
    }

    /// The settings file and the command line flags.
    pub(crate) fn host(&self) -> &HostLayers {
        &self.host
    }

    /// Takes the catalog of the language that a save of the settings page
    /// selected. A second call gives `None` until the next switch.
    pub(crate) fn take_catalog_switch(&mut self) -> Option<LanguageCatalog> {
        self.catalog_switch.take()
    }

    /// Scans the catalog again and restores the view on the new games.
    fn rescan(&mut self) {
        let catalog = Catalog::scan(&self.host);
        let root = self.host.root().to_path_buf();
        for entry in &mut self.systems {
            entry.rows = catalog
                .system(entry.system.name)
                .map(|system| system_rows(system, &root))
                .unwrap_or_default();
        }
        let names: Vec<&'static str> = self.systems.iter().map(|entry| entry.system.name).collect();
        let system = self.view.restore_system(&names);
        self.refresh_entries();
        self.view
            .restore(&self.entries, rows_at(&self.systems, system));
    }

    /// The line of the log about the last scan: the number of game files and
    /// the config folder.
    fn scan_summary(&self) -> String {
        let files: usize = self.systems.iter().map(|entry| entry.rows.len()).sum();
        format!(
            "Game manager: {files} game files in {}",
            self.config_folder.display()
        )
    }

    /// Checks the ROM folder of every system again and scans the catalog
    /// again. The view stays as [`Manager::rescan`] keeps it, with the
    /// nearest row when the selected game is gone.
    fn refresh(&mut self) {
        self.roms = check_all_folders(&self.host);
        for entry in &mut self.systems {
            entry.take_host(&self.host, &self.roms);
        }
        self.rescan();
        info!("{}", self.scan_summary());
    }

    /// Scans the catalog again after a delete.
    fn rescan_after_delete(&mut self) {
        self.rescan();
        debug!("{}", self.scan_summary());
    }

    /// The selected system.
    fn current(&self) -> Option<&SystemEntry> {
        self.systems.get(self.view.systems.selected)
    }

    /// The games and the files with errors of the selected system.
    fn rows(&self) -> &[GameRow] {
        rows_at(&self.systems, self.view.systems.selected)
    }

    /// Builds the entries of the game table of the selected system from the
    /// sort and the filter of the view.
    fn refresh_entries(&mut self) {
        let sort = self.view.table.sort.unwrap_or(entries::DEFAULT_SORT);
        let filter = Filter::new(&self.view.filter);
        self.entries = table_entries(self.rows(), sort, &filter);
        self.row_kinds.clear();
        self.row_kinds
            .extend(self.entries.iter().map(|entry| entry.kind()));
    }

    /// The row of the selected entry of the game table.
    fn selected_row(&self) -> Option<&GameRow> {
        self.view.selected_row(&self.entries, self.rows())
    }

    /// Whether the game table has an entry that takes the selection.
    fn has_entries(&self) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.kind() == RowKind::Normal)
    }

    /// Selects the system at `index` of the system list with the game table
    /// at its top.
    fn select_system(&mut self, index: usize) {
        let Some(name) = self.systems.get(index).map(|entry| entry.system.name) else {
            return;
        };
        self.view.select_system(index, name);
        self.refresh_entries();
        let rows = rows_at(&self.systems, self.view.systems.selected);
        self.view.record(&self.entries, rows);
    }

    /// The layers of a launch of `row`: the game file of the row in its
    /// system, with the settings file and the flags of the command line.
    pub(crate) fn launch_request(&self, row: &GameRow) -> LaunchRequest {
        LaunchRequest {
            settings: self.start.request.settings.clone(),
            game_files: vec![row.path.clone()],
            flags: self.start.request.flags.clone(),
            system: Some(row.system),
        }
    }

    /// The launch of `row`.
    fn launch(&self, row: &GameRow) -> Launch {
        Launch {
            request: self.launch_request(row),
            title: row.title.clone(),
            system: row.system,
            model: row.model,
        }
    }

    /// Builds the layered configuration of `launch`.
    pub(crate) fn configuration(
        &self,
        launch: &Launch,
    ) -> std::result::Result<(EmulatorConfig, KeyOverrides), LaunchError> {
        build_configuration(&launch.request, &self.start.locations)
    }

    /// Records that the game of `launch` runs.
    pub(crate) fn game_started(&mut self, launch: Launch) {
        self.running = Some(launch);
    }

    /// Opens the error dialog of `launch` that did not start with `failure`.
    /// Returns the id of the modal layer of the dialog, which the caller
    /// opens in the state of the user interface.
    pub(crate) fn launch_failed(&mut self, launch: &Launch, failure: &LaunchFailure) -> UiId {
        let dialog = ErrorDialog::launch_failed(&launch.title, failure);
        let dialog = match launch.request.game_files.first() {
            Some(path) => dialog.with_edit(EditTarget {
                system: launch.system,
                path: path.clone(),
            }),
            None => dialog,
        };
        self.open_dialog(dialog)
    }

    /// Returns from the running game with the view of the manager as it
    /// was. After a panic with `panic_message` it opens the panic dialog.
    /// After a failed write back of the disk images in `flush` it opens the
    /// dialog of the images that were not saved. Returns the id of the modal
    /// layer of an opened dialog, which the caller opens in the state of the
    /// user interface.
    pub(crate) fn game_ended(
        &mut self,
        panic_message: Option<&str>,
        flush: std::result::Result<(), FlushError>,
    ) -> Option<UiId> {
        let running = self.running.take();
        self.next_focus = Some(NextFocus::Start);
        let dialog = match (panic_message, &flush) {
            (Some(message), _) => {
                let label = running.as_ref().map(|launch| launch.system.label);
                Some(ErrorDialog::game_stopped(label, message, &flush))
            }
            (None, Err(error)) => {
                let title = running.as_ref().map(|launch| launch.title.as_str());
                Some(ErrorDialog::media_not_saved(title, error))
            }
            (None, Ok(())) => None,
        };
        dialog.map(|dialog| self.open_dialog(dialog))
    }

    /// Opens `dialog` over the library and returns the id of its modal layer.
    fn open_dialog(&mut self, dialog: ErrorDialog) -> UiId {
        self.dialog = Some(dialog);
        UiId::from(error_dialog::LAYER_ID)
    }

    /// The open error dialog.
    #[cfg(test)]
    pub(crate) fn dialog(&self) -> Option<&ErrorDialog> {
        self.dialog.as_ref()
    }

    /// Draws one pass of the manager. Returns the game to start, if the user
    /// asked for one.
    pub(crate) fn show(&mut self, context: &mut UiContext) -> Option<Launch> {
        if self.editor.is_some() {
            self.show_editor(context);
            return None;
        }
        let theme = context.theme;
        let bounds = context.bounds();
        context.fill(bounds, theme.colors.panel);
        let (status_area, screen) = bounds.cut_bottom(STATUS_LINE_HEIGHT);
        let (sidebar_area, main_area) = screen.cut_left(SIDEBAR_WIDTH);

        let start_focus = if self.has_entries() {
            UiId::from(GAME_TABLE_ID)
        } else {
            UiId::from(ADD_ID)
        };
        match self.next_focus.take() {
            Some(NextFocus::Start) => context.set_focus(start_focus),
            Some(NextFocus::Widget(id)) => context.set_focus(id),
            None => context.default_focus(start_focus),
        }
        if context.shortcut() == Some(Shortcut::ControlF)
            || context.action_button() == Some(ActionButton::Y)
        {
            context.set_focus(UiId::from(FILTER_ID));
        }

        self.show_systems(context, sidebar_area);
        let launch = self.show_main(context, main_area);
        let delete_response = self.show_dialogs(context, screen);
        self.show_status_line(context, status_area, delete_response);
        launch
    }

    /// Draws the open question and the open error dialog, centered in
    /// `screen`. A delete scans the catalog again. Returns the answer of the
    /// question in this pass.
    fn show_dialogs(&mut self, context: &mut UiContext, screen: Rect) -> Option<ConfirmResponse> {
        let mut delete_response = None;
        if let Some(mut request) = self.delete.take() {
            let (outcome, response) = request.show(context, screen);
            match outcome {
                DeleteOutcome::Open => {
                    delete_response = Some(response);
                    self.delete = Some(request);
                }
                DeleteOutcome::Kept => {}
                DeleteOutcome::Deleted => {
                    info!("Game manager: deleted {}", request.path.display());
                    self.rescan_after_delete();
                }
                DeleteOutcome::Failed { path, error } => {
                    self.dialog = Some(ErrorDialog::delete_failed(&request.name, path, error));
                    self.rescan_after_delete();
                }
            }
        }
        let answer = self
            .dialog
            .as_mut()
            .map_or(DialogAnswer::Open, |dialog| dialog.show(context, screen));
        match answer {
            DialogAnswer::Open => {}
            DialogAnswer::Closed | DialogAnswer::Overwrite => self.dialog = None,
            DialogAnswer::Edit => {
                let target = self
                    .dialog
                    .take()
                    .and_then(|dialog| dialog.edit_target().cloned());
                if let Some(target) = target {
                    self.open_target(context, &target);
                }
            }
        }
        delete_response
    }
}

/// The games and the files with errors of the system at `index` of `systems`.
fn rows_at(systems: &[SystemEntry], index: usize) -> &[GameRow] {
    systems
        .get(index)
        .map_or(&[], |entry| entry.rows.as_slice())
}

#[cfg(test)]
mod tests {
    use gui::ui::{Navigation, ThemeKind, Viewport};
    use machine_factory::rom_status::RomStatus;

    use super::{
        delete::DELETE_LAYER_ID,
        sidebar::SYSTEM_LIST_ID,
        test_support::{TestRoot, first_system, games, sample_root, select, steady_frame, titles},
        *,
    };
    use crate::gui_test_support::{GuiHarness, PassCounts};

    /// Verifies that the launch request of a row holds its game file, its system and the command line flags.
    #[test]
    fn the_launch_request_holds_the_game_file_and_its_system() {
        let test = TestRoot::new("launch_request");
        let system = first_system();
        let identity = format!("{}/edge.conf", system.name);
        test.game(&identity, "meta.title = Edge\n");
        let mut manager = test.manager();
        manager.start.request.flags = vec![(
            machine_factory::config::settings::find_setting("crt").expect("table key"),
            "off".to_owned(),
        )];
        let row = manager.rows()[0].clone();
        assert_eq!(row.title, "Edge");
        let request = manager.launch_request(&row);
        assert_eq!(
            request.game_files,
            vec![test.root.path().join("config").join(&identity)]
        );
        assert_eq!(request.system, Some(system));
        assert_eq!(request.flags.len(), 1);
        assert_eq!(request.settings, None);
        let launch = manager.launch(&row);
        let (config, _) = manager
            .configuration(&launch)
            .expect("configuration builds");
        assert!(!config.crt);
        assert_eq!(
            machine_factory::config::systems::system_of(config.target),
            system
        );
    }

    /// Verifies that the selected game returns after a refresh and that a removed game selects the nearest row.
    #[test]
    fn the_view_returns_after_a_refresh() {
        let test = TestRoot::new("view_restore");
        games(&test, &["a", "b", "c"]);
        let mut manager = test.manager();
        let mut harness = GuiHarness::new(960, 600);
        harness.settle(|context| manager.show(context));
        harness.input.navigation = Some(Navigation::End);
        harness.settle(|context| manager.show(context));
        let selected = format!("{}/c.conf", first_system().name);
        assert_eq!(manager.view.selected.as_deref(), Some(selected.as_str()));

        manager.refresh();
        assert_eq!(manager.view.selected.as_deref(), Some(selected.as_str()));
        assert_eq!(manager.view.table.list.selected, 2);

        test.remove(&selected);
        manager.refresh();
        assert_eq!(manager.view.table.list.selected, 1);
        assert!(manager.dialog().is_none());
    }

    /// Verifies that a return from a game keeps the view and scans neither the catalog nor the ROM folders.
    #[test]
    fn a_return_from_a_game_does_no_scan() {
        let test = TestRoot::new("return_no_scan");
        games(&test, &["a", "b"]);
        let mut manager = test.manager();
        let status = manager.systems[0].rom_status;
        let stale = match status {
            RomStatus::Found => RomStatus::Missing,
            RomStatus::Partial
            | RomStatus::Missing
            | RomStatus::NotFound
            | RomStatus::Unsupported => RomStatus::Found,
        };
        manager.systems[0].rom_status = stale;
        let view = manager.view.clone();
        manager.game_started(manager.launch(&manager.rows()[0].clone()));
        games(&test, &["c"]);
        test.remove(&format!("{}/a.conf", first_system().name));
        manager.game_ended(None, Ok(()));
        assert_eq!(titles(&manager), ["a", "b"]);
        assert_eq!(manager.systems[0].rom_status, stale);
        assert_eq!(manager.view, view);
    }

    /// Verifies that the sort, the filter, the system and the scroll position survive a game, and that a changed filter result after a refresh gives the selection to the nearest row.
    #[test]
    fn the_view_survives_a_game_with_its_sort_and_filter() {
        let test = TestRoot::new("view_survives");
        let names: Vec<String> = (0..40).map(|index| format!("game {index:02}")).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        games(&test, &names);
        let mut manager = test.manager();
        let mut harness = GuiHarness::new(960, 600);
        harness.settle(|context| manager.show(context));
        manager.view.table.sort_by(entries::TITLE_COLUMN);
        manager.refresh_entries();
        manager.view.filter = "game 1".to_owned();
        manager.refresh_entries();
        manager.view.table.list.select(5);
        let rows = manager.rows().to_vec();
        manager.view.record(&manager.entries, &rows);
        harness.settle(|context| manager.show(context));
        let before = manager.view.clone();
        assert_eq!(titles(&manager)[0], "game 19");
        assert_eq!(
            manager
                .view
                .selected
                .as_deref()
                .map(|id| id.ends_with("game 14.conf")),
            Some(true)
        );

        manager.game_started(manager.launch(&rows[0]));
        manager.game_ended(None, Ok(()));
        assert_eq!(manager.view, before);

        test.game(
            &format!("{}/game 14.conf", first_system().name),
            "meta.title = renamed\n",
        );
        manager.refresh();
        assert_eq!(manager.view.table.list.selected, 5);
        assert_eq!(titles(&manager)[5], "game 13");
        assert_eq!(manager.view.filter, "game 1");
    }

    /// Verifies the delete flow: cancel keeps the file, confirm removes it and selects the nearest row.
    #[test]
    fn delete_asks_first_and_removes_the_file() {
        let test = TestRoot::new("delete");
        games(&test, &["a", "b", "c"]);
        let mut manager = test.manager();
        let mut harness = GuiHarness::new(960, 600);
        harness.settle(|context| manager.show(context));
        harness.input.navigation = Some(Navigation::End);
        harness.settle(|context| manager.show(context));

        harness.input.shortcut = Some(Shortcut::Delete);
        harness.settle(|context| manager.show(context));
        assert!(manager.delete.is_some());
        harness.input.confirm = true;
        harness.settle(|context| manager.show(context));
        assert!(manager.delete.is_none());
        assert_eq!(titles(&manager), ["a", "b", "c"]);
        assert_eq!(harness.state.focused(), Some(UiId::from(GAME_TABLE_ID)));

        harness.input.shortcut = Some(Shortcut::Delete);
        harness.settle(|context| manager.show(context));
        harness.input.cancel = true;
        harness.settle(|context| manager.show(context));
        assert!(manager.delete.is_none());
        assert_eq!(titles(&manager), ["a", "b", "c"]);

        harness.input.shortcut = Some(Shortcut::Delete);
        harness.settle(|context| manager.show(context));
        harness.input.navigation = Some(Navigation::Right);
        harness.settle(|context| manager.show(context));
        harness.input.confirm = true;
        harness.settle(|context| manager.show(context));
        assert_eq!(titles(&manager), ["a", "b"]);
        assert_eq!(manager.view.table.list.selected, 1);
        assert!(
            !test
                .root
                .path()
                .join("config")
                .join(first_system().name)
                .join("c.conf")
                .exists()
        );
        assert!(manager.dialog().is_none());
    }

    /// Verifies that a failed delete opens the error dialog and that its OK ignores the key that answered the question.
    #[test]
    fn a_failed_delete_opens_the_error_dialog() {
        let test = TestRoot::new("delete_failed");
        games(&test, &["a"]);
        let mut manager = test.manager();
        let mut harness = GuiHarness::new(960, 600);
        harness.settle(|context| manager.show(context));
        harness.input.shortcut = Some(Shortcut::Delete);
        harness.settle(|context| manager.show(context));
        test.remove(&format!("{}/a.conf", first_system().name));
        harness.input.navigation = Some(Navigation::Right);
        harness.settle(|context| manager.show(context));
        harness.input.confirm = true;
        harness.settle(|context| manager.show(context));
        let dialog = manager.dialog().expect("the error dialog opens");
        assert_eq!(dialog.message(), Some("a was not deleted."));
        assert_eq!(harness.state.open_modals(), 1);
        harness.input.confirm = true;
        harness.settle(|context| manager.show(context));
        assert!(manager.dialog().is_none());
    }

    /// Verifies that the delete of the last game gives the focus to Add.
    #[test]
    fn the_delete_of_the_last_game_focuses_add() {
        let test = TestRoot::new("delete_last");
        games(&test, &["a"]);
        let mut manager = test.manager();
        let mut harness = GuiHarness::new(960, 600);
        harness.settle(|context| manager.show(context));
        harness.input.shortcut = Some(Shortcut::Delete);
        harness.settle(|context| manager.show(context));
        harness.input.navigation = Some(Navigation::Right);
        harness.settle(|context| manager.show(context));
        harness.input.confirm = true;
        harness.settle(|context| manager.show(context));
        assert!(titles(&manager).is_empty());
        assert_eq!(harness.state.focused(), Some(UiId::from(ADD_ID)));
    }

    /// Verifies that the delete of the game before the caption gives the selection to the game before it.
    #[test]
    fn a_delete_before_the_caption_selects_the_game_before() {
        let test = TestRoot::new("delete_caption");
        let system = first_system();
        games(&test, &["a", "b"]);
        test.game(&format!("{}/x.conf", system.name), "machine = FOOBAR\n");
        let mut manager = test.manager();
        let mut harness = GuiHarness::new(960, 600);
        harness.settle(|context| manager.show(context));
        harness.input.navigation = Some(Navigation::Down);
        harness.settle(|context| manager.show(context));
        assert_eq!(titles(&manager), ["a", "b", "|", "x.conf"]);
        assert_eq!(manager.view.table.list.selected, 1);
        test.remove(&format!("{}/b.conf", system.name));
        manager.rescan();
        assert_eq!(manager.view.table.list.selected, 0);
        let expected = format!("{}/a.conf", system.name);
        assert_eq!(manager.view.selected.as_deref(), Some(expected.as_str()));
    }

    /// Verifies that the error dialog keeps the navigation from the table, closes on confirm and returns the focus to the table.
    #[test]
    fn the_error_dialog_takes_the_input_until_it_closes() {
        let test = TestRoot::new("dialog");
        games(&test, &["A", "B"]);
        let mut manager = test.manager();
        let launch = manager.launch(&manager.rows()[0].clone());
        let layer = manager.launch_failed(&launch, &LaunchFailure::missing_image("/a.d88"));
        let mut harness = GuiHarness::new(960, 600);
        harness.settle(|context| manager.show(context));
        harness.state.open_layer(layer);
        harness.settle(|context| manager.show(context));
        assert_eq!(
            manager.dialog().and_then(ErrorDialog::message),
            Some("A did not start.")
        );
        let selected = manager.view.table.list.selected;
        harness.input.navigation = Some(Navigation::Down);
        assert!(harness.frame(|context| manager.show(context)).is_none());
        assert_eq!(manager.view.table.list.selected, selected);
        assert!(manager.dialog().is_some());
        harness.input.confirm = true;
        assert!(harness.settle(|context| manager.show(context)).is_none());
        assert!(manager.dialog().is_none());
        assert_eq!(harness.state.focused(), Some(UiId::from(GAME_TABLE_ID)));
        harness.input.navigation = Some(Navigation::Down);
        harness.settle(|context| manager.show(context));
        assert_eq!(manager.view.table.list.selected, selected + 1);

        manager.game_started(launch);
        let layer = manager
            .game_ended(Some("core halted"), Ok(()))
            .expect("the panic dialog opens");
        harness.state.open_layer(layer);
        harness.settle(|context| manager.show(context));
        let dialog = manager.dialog().expect("panic dialog opens");
        assert_eq!(dialog.detail(), Some("core halted"));
        assert_eq!(
            dialog.message(),
            Some(
                format!(
                    "The {} core stopped with an error. The disk images were saved.",
                    first_system().label
                )
                .as_str()
            )
        );
    }

    /// Verifies that a queued key after a dialog that opens between passes does not reach the table.
    #[test]
    fn a_queued_key_after_a_dialog_opened_between_passes_stays_out_of_the_table() {
        let test = TestRoot::new("queued_key");
        games(&test, &["a", "b", "c"]);
        let mut manager = test.manager();
        let mut harness = GuiHarness::new(960, 600);
        harness.settle(|context| manager.show(context));
        let selected = manager.view.table.list.selected;
        let launch = manager.launch(&manager.rows()[0].clone());
        let layer = manager.launch_failed(&launch, &LaunchFailure::missing_image("/a.d88"));
        harness.state.open_layer(layer);
        for navigation in [Navigation::Down, Navigation::PageDown] {
            harness.input.navigation = Some(navigation);
            assert!(harness.frame(|context| manager.show(context)).is_none());
            assert_eq!(manager.view.table.list.selected, selected);
        }
        harness.input.confirm = true;
        assert!(harness.frame(|context| manager.show(context)).is_none());
        assert!(manager.dialog().is_none());
        harness.input.navigation = Some(Navigation::Down);
        harness.settle(|context| manager.show(context));
        assert_eq!(manager.view.table.list.selected, selected + 1);
    }

    /// Verifies that a failed write back after a normal end of the game opens the dialog of the images.
    #[test]
    fn a_failed_flush_after_the_game_opens_the_media_dialog() {
        let test = TestRoot::new("flush");
        games(&test, &["A"]);
        let mut manager = test.manager();
        manager.game_started(manager.launch(&manager.rows()[0].clone()));
        let layer = manager
            .game_ended(
                None,
                Err(FlushError::new(
                    "hard disk image /a.hdi could not be written",
                )),
            )
            .expect("the media dialog opens");
        let mut harness = GuiHarness::new(960, 600);
        harness.state.open_layer(layer);
        harness.settle(|context| manager.show(context));
        let dialog = manager.dialog().expect("the media dialog opens");
        assert_eq!(
            dialog.message(),
            Some("A ended. Some of its disk images were not saved.")
        );
        assert_eq!(
            dialog.detail(),
            Some("hard disk image /a.hdi could not be written")
        );

        let mut manager = test.manager();
        manager.game_ended(None, Ok(()));
        assert!(manager.dialog().is_none());
    }

    /// Renders the library at `width` x `height` logical units and `scale`:
    /// the library with data, a filtered library, an empty system, a system
    /// with files with errors and the delete question, in the theme `kind`.
    fn screenshots(width: f32, height: f32, scale: f32, prefix: &str, kind: ThemeKind) {
        let viewport = Viewport::new((width * scale) as u32, (height * scale) as u32, scale);
        let test = TestRoot::new(&format!("screenshots_{prefix}_{scale}"));
        sample_root(&test);
        let mut manager = test.manager();
        let mut harness =
            GuiHarness::with_viewport(viewport, kind, crate::texts::LanguageCatalog::English);
        let render = |harness: &GuiHarness, name: &str| {
            harness.write_screenshot(&format!("manager_{prefix}{name}_{scale}x"));
        };
        harness.settle(|context| manager.show(context));
        harness.input.navigation = Some(Navigation::End);
        harness.settle(|context| manager.show(context));
        render(&harness, "library");
        if !prefix.is_empty() {
            return;
        }

        harness.input.shortcut = Some(Shortcut::Delete);
        harness.settle(|context| manager.show(context));
        render(&harness, "delete");
        harness.input.cancel = true;
        harness.settle(|context| manager.show(context));

        harness.input.shortcut = Some(Shortcut::ControlF);
        harness.settle(|context| manager.show(context));
        harness.input.text = "touhou".to_owned();
        harness.settle(|context| manager.show(context));
        render(&harness, "filtered");
        harness.input.text = "x".to_owned();
        harness.settle(|context| manager.show(context));
        render(&harness, "no_match");
        harness.input.cancel = true;
        harness.settle(|context| manager.show(context));

        select(&mut manager, "pc6000");
        harness.state.set_focus(UiId::from(SYSTEM_LIST_ID));
        harness.settle(|context| manager.show(context));
        render(&harness, "empty");

        select(&mut manager, "x68k");
        harness.state.set_focus(UiId::from(GAME_TABLE_ID));
        harness.settle(|context| manager.show(context));
        harness.input.navigation = Some(Navigation::End);
        harness.settle(|context| manager.show(context));
        render(&harness, "errors");

        select(&mut manager, "pc88");
        harness.state.set_focus(UiId::from(SYSTEM_LIST_ID));
        harness.settle(|context| manager.show(context));
        render(&harness, "system_list");

        select(&mut manager, "pc98");
        harness.state.set_focus(UiId::from(GAME_TABLE_ID));
        harness.settle(|context| manager.show(context));
        let launch = manager.launch(&manager.rows()[0].clone());
        let layer = manager.launch_failed(
            &launch,
            &LaunchFailure::missing_image(
                "/home/user/Games/Neetan/games/98/YU-NO/YU-NO (Disk 3 of 4).hdm",
            ),
        );
        harness.state.open_layer(layer);
        harness.settle(|context| manager.show(context));
        render(&harness, "launch_error");
        harness.input.confirm = true;
        harness.settle(|context| manager.show(context));

        manager.game_started(launch.clone());
        let layer = manager.game_ended(
            Some("panicked at crates/sample/src/screenshot.rs:1:1:\nsample panic message of the screenshot test"),
            Ok(()),
        );
        harness
            .state
            .open_layer(layer.expect("the panic dialog opens"));
        harness.settle(|context| manager.show(context));
        render(&harness, "game_stopped");
        harness.input.confirm = true;
        harness.settle(|context| manager.show(context));

        manager.game_started(launch);
        let layer = manager.game_ended(
            None,
            Err(FlushError::new(
                "hard disk image /home/user/Games/Neetan/games/98/Touhou/reiiden.hdi could not be written: Permission denied (os error 13)",
            )),
        );
        harness
            .state
            .open_layer(layer.expect("the media dialog opens"));
        harness.settle(|context| manager.show(context));
        render(&harness, "media_not_saved");
    }

    /// The title of the game of the steady dialog tests, long enough to wrap in every dialog.
    const LONG_TITLE: &str = "YU-NO: A Girl Who Chants Love at the Bound of this World";

    /// Verifies that a steady frame of the delete question allocates nothing, fills no text and
    /// wraps no text, with the focus on Cancel and on Delete.
    #[test]
    fn a_steady_frame_of_the_delete_question_allocates_nothing() {
        let test = TestRoot::new("steady_delete");
        games(&test, &[LONG_TITLE]);
        let mut manager = test.manager();
        let mut harness = GuiHarness::new(960, 600);
        harness.settle(|context| manager.show(context));
        harness.input.shortcut = Some(Shortcut::Delete);
        harness.settle(|context| manager.show(context));
        assert!(manager.delete.is_some());
        let cancel = UiId::from(DELETE_LAYER_ID).child("cancel");
        let action = UiId::from(DELETE_LAYER_ID).child("action");
        for focus in [cancel, action] {
            if focus == action {
                harness.input.navigation = Some(Navigation::Right);
                harness.settle(|context| manager.show(context));
            }
            assert_eq!(harness.state.focused(), Some(focus));
            assert_eq!(
                steady_frame(&mut harness, &mut manager),
                PassCounts::NONE,
                "a steady frame of the delete question with the focus on {focus:?}"
            );
        }
    }

    /// Verifies that a steady frame of the launch error with its OK and Edit game buttons and of
    /// the panic error allocates nothing, fills no text and wraps no text, with the focus on each
    /// button.
    #[test]
    fn a_steady_frame_of_an_error_dialog_allocates_nothing() {
        let test = TestRoot::new("steady_error");
        games(&test, &[LONG_TITLE]);
        let mut manager = test.manager();
        let mut harness = GuiHarness::new(960, 600);
        harness.settle(|context| manager.show(context));
        let launch = manager.launch(&manager.rows()[0].clone());
        let layer = manager.launch_failed(
            &launch,
            &LaunchFailure::missing_image(
                "/home/user/Emulation/PC-98/YU-NO/YU-NO (Disk 1 of 4).hdm",
            ),
        );
        harness.state.open_layer(layer);
        harness.settle(|context| manager.show(context));
        let ok = UiId::from(error_dialog::OK_ID);
        let edit = UiId::from(error_dialog::EDIT_ID);
        for focus in [ok, edit] {
            if focus == edit {
                harness.input.navigation = Some(Navigation::Left);
                harness.settle(|context| manager.show(context));
            }
            assert_eq!(harness.state.focused(), Some(focus));
            assert_eq!(
                steady_frame(&mut harness, &mut manager),
                PassCounts::NONE,
                "a steady frame of the launch error with the focus on {focus:?}"
            );
        }
        harness.input.navigation = Some(Navigation::Right);
        harness.settle(|context| manager.show(context));
        harness.input.confirm = true;
        harness.settle(|context| manager.show(context));
        assert!(manager.dialog().is_none());

        manager.game_started(launch);
        let layer = manager
            .game_ended(
                Some("The core halted at 0F000:FFF0 after an unknown opcode in the BIOS of the machine."),
                Ok(()),
            )
            .expect("the panic dialog opens");
        harness.state.open_layer(layer);
        harness.settle(|context| manager.show(context));
        assert_eq!(harness.state.focused(), Some(ok));
        assert_eq!(
            steady_frame(&mut harness, &mut manager),
            PassCounts::NONE,
            "a steady frame of the panic error"
        );
    }

    /// Writes the screenshots of the manager in both themes at the scales 1
    /// and 2, at the minimum window size and at 1920 x 1080.
    #[test]
    #[ignore = "writes screenshots into the directory of GUI_SHOTS"]
    fn manager_screenshots() {
        for kind in [ThemeKind::Dark, ThemeKind::Light] {
            for scale in [1.0, 2.0] {
                screenshots(960.0, 600.0, scale, "", kind);
                screenshots(1920.0, 1080.0, scale, "large_", kind);
            }
        }
    }
}
