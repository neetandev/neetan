//! The game manager: the library with the systems in a sidebar and the games
//! of the selected system in a table, the launch of a game, the editor of a
//! game file, the delete of a game file and the error dialogs.

mod delete;
mod editing;
mod entries;
mod error_dialog;
mod filter;
mod game_table;
mod hints;
mod rows;
mod sidebar;
mod toolbar;
mod view;

use std::path::PathBuf;

use common::{FlushError, debug, info};
use gui::{
    Rect,
    ui::{
        ActionButton, ConfirmResponse, EmptyHint, HEADER_HEIGHT, Navigation, RowKind,
        STATUS_LINE_HEIGHT, Shortcut, StatusEnd, TextField, UiContext, UiId, status_line,
    },
};
use machine_factory::config::systems::{System, compiled_systems};

use self::{
    delete::{DeleteOutcome, DeleteRequest},
    entries::{TableEntry, table_entries},
    error_dialog::{DialogAnswer, EditTarget},
    filter::Filter,
    game_table::{GAME_TABLE_ID, show_game_table},
    hints::{LibraryFocus, library_hints, status_end},
    rows::system_rows,
    sidebar::{SYSTEM_LIST_ID, SystemEntry, show_sidebar},
    toolbar::{
        ADD_ID, DELETE_ID, DUPLICATE_ID, EDIT_ID, LAUNCH_ID, REFRESH_ID, ToolbarState, show_toolbar,
    },
};
pub(crate) use self::{
    error_dialog::ErrorDialog,
    rows::{GameRow, shown_path},
    view::ManagerView,
};
use crate::{
    Result,
    catalog::Catalog,
    config::{EmulatorConfig, HostLayers, LaunchRequest, ManagerStart, build_configuration},
    editor::Editor,
    input::KeyOverrides,
};

/// The width of the sidebar in logical units.
const SIDEBAR_WIDTH: f32 = 240.0;

/// The width of the filter field in logical units.
const FILTER_WIDTH: f32 = 260.0;

/// The id of the filter field.
const FILTER_ID: &str = "filter";

/// The placeholder of the filter field.
const FILTER_PLACEHOLDER: &str = "Filter games";

/// The hint of a filter without a match.
const NO_MATCH_HINT: &str = "No games match the filter";

/// A game that the user asked to start.
#[derive(Debug, Clone)]
pub(crate) struct Launch {
    /// The layers of the launch.
    pub(crate) request: LaunchRequest,
    /// The title of the game.
    pub(crate) title: String,
    /// The system of the game.
    pub(crate) system: &'static System,
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
    /// The entries of the game table of the selected system.
    entries: Vec<TableEntry>,
    /// The selection, the sort, the filter and the scroll positions.
    view: ManagerView,
    /// The open question before a delete.
    delete: Option<DeleteRequest>,
    /// The open error dialog.
    dialog: Option<ErrorDialog>,
    /// The open game editor. It takes the whole window while it is open.
    editor: Option<Editor>,
    /// Whether the next pass gives the focus to the game table, or to the
    /// add button while the table has no entry.
    give_start_focus: bool,
    /// The game that runs.
    running: Option<Launch>,
}

impl Manager {
    /// Reads the settings file of `start`, checks the ROM folder of every
    /// compiled system and scans the catalog.
    pub(crate) fn new(start: ManagerStart) -> Result<Self> {
        let host = HostLayers::read(&start.request, &start.locations)?;
        let config_folder = host.config_folder();
        let systems = compiled_systems()
            .map(|system| SystemEntry::new(system, &host))
            .collect();
        let mut manager = Self {
            start,
            host,
            config_folder,
            systems,
            entries: Vec::new(),
            view: ManagerView::default(),
            delete: None,
            dialog: None,
            editor: None,
            give_start_focus: true,
            running: None,
        };
        manager.rescan();
        info!("{}", manager.scan_summary());
        Ok(manager)
    }

    /// The settings file and the command line flags.
    pub(crate) fn host(&self) -> &HostLayers {
        &self.host
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
        for entry in &mut self.systems {
            entry.check_roms(&self.host);
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
        }
    }

    /// Builds the layered configuration of `launch`.
    pub(crate) fn configuration(&self, launch: &Launch) -> Result<(EmulatorConfig, KeyOverrides)> {
        build_configuration(&launch.request, &self.start.locations)
    }

    /// Records that the game of `launch` runs.
    pub(crate) fn game_started(&mut self, launch: Launch) {
        self.running = Some(launch);
    }

    /// Opens the error dialog of `launch` that did not start with `error`.
    /// Returns the id of the modal layer of the dialog, which the caller
    /// opens in the state of the user interface.
    pub(crate) fn launch_failed(&mut self, launch: &Launch, error: &str) -> UiId {
        let dialog = ErrorDialog::launch_failed(&launch.title, error);
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
        self.give_start_focus = true;
        let dialog = match (panic_message, &flush) {
            (Some(message), _) => {
                let label = running
                    .as_ref()
                    .map_or("emulator", |launch| launch.system.label);
                Some(ErrorDialog::game_stopped(label, message, &flush))
            }
            (None, Err(error)) => {
                let title = running
                    .as_ref()
                    .map_or("The game", |launch| launch.title.as_str());
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
        context.fill(bounds, theme.panel);
        let (status_area, screen) = bounds.cut_bottom(STATUS_LINE_HEIGHT);
        let (sidebar_area, main_area) = screen.cut_left(SIDEBAR_WIDTH);

        let start_focus = if self.has_entries() {
            UiId::from(GAME_TABLE_ID)
        } else {
            UiId::from(ADD_ID)
        };
        if std::mem::take(&mut self.give_start_focus) {
            context.set_focus(start_focus);
        } else {
            context.default_focus(start_focus);
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

    /// Draws the sidebar into `area` and switches the system on a new
    /// selection of the system list or on a shoulder button.
    fn show_systems(&mut self, context: &mut UiContext, area: Rect) {
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
                self.select_system(index);
            }
        }
        let response = show_sidebar(context, area, &self.systems, &mut self.view.systems);
        if response.selection_changed {
            self.select_system(self.view.systems.selected);
            context.request_pass();
        }
        if response.activated.is_some() && self.has_entries() {
            context.set_focus(UiId::from(GAME_TABLE_ID));
        }
    }

    /// Draws the header with the filter, the toolbar and the game table into
    /// `area`. Returns the game to start.
    fn show_main(&mut self, context: &mut UiContext, area: Rect) -> Option<Launch> {
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
        let filter_area = Rect::new(
            header.right() - FILTER_WIDTH.min(header.width),
            header.y + (header.height - theme.control_height) * 0.5,
            FILTER_WIDTH.min(header.width),
            theme.control_height,
        );
        let filter = TextField::new(filter_id, FILTER_PLACEHOLDER).show(
            context,
            filter_area,
            &mut self.view.filter,
        );
        if filter.changed {
            self.refresh_entries();
            let rows = rows_at(&self.systems, self.view.systems.selected);
            self.view.refilter(&self.entries, rows);
        }
        if filter.submitted && self.has_entries() {
            context.set_focus(UiId::from(GAME_TABLE_ID));
        }
        self.show_title(context, header.cut_right(filter_area.width + theme.gap).1);

        let selected = self.selected_row();
        let state = ToolbarState {
            addable: self.current().is_some(),
            launchable: selected.is_some_and(GameRow::launchable),
            deletable: selected.is_some(),
        };
        let toolbar = show_toolbar(context, toolbar_area, state);
        if toolbar.refresh || context.shortcut() == Some(Shortcut::F5) {
            self.refresh();
            context.request_pass();
        }

        let (empty_title, empty_detail) = self.current().map_or_else(Default::default, |entry| {
            (
                format!("No games for {}", entry.system.label),
                format!("Add a game, or place game files in {}/", entry.folder),
            )
        });
        let hint = if self.rows().is_empty() {
            EmptyHint::Centered {
                title: &empty_title,
                detail: &empty_detail,
            }
        } else {
            EmptyHint::Top(NO_MATCH_HINT)
        };
        let rows = rows_at(&self.systems, self.view.systems.selected);
        let response = show_game_table(
            context,
            table_area,
            &self.entries,
            rows,
            &mut self.view.table,
            hint,
        );
        if response.sort_changed {
            self.refresh_entries();
            let rows = rows_at(&self.systems, self.view.systems.selected);
            self.view.restore(&self.entries, rows);
            context.request_pass();
        } else if response.selection_changed {
            let rows = rows_at(&self.systems, self.view.systems.selected);
            self.view.record(&self.entries, rows);
            context.request_pass();
        }

        let shortcut = context.shortcut();
        let delete_shortcut = shortcut == Some(Shortcut::Delete);
        let edit = toolbar.edit
            || shortcut == Some(Shortcut::F2)
            || context.action_button() == Some(ActionButton::X);
        let duplicate = toolbar.duplicate || shortcut == Some(Shortcut::ControlD);
        if (toolbar.add || shortcut == Some(Shortcut::Insert)) && state.addable {
            self.open_new_game(context);
            return None;
        }
        let activated = response.activated.is_some();
        let row = self.selected_row()?.clone();
        if row.launchable() && (activated || toolbar.launch) {
            return Some(self.launch(&row));
        }
        if row.launchable() && edit {
            self.open_game(context, &row);
            return None;
        }
        if row.launchable() && duplicate {
            self.open_duplicate(context, &row);
            return None;
        }
        if delete_shortcut || toolbar.delete || activated {
            self.delete = Some(DeleteRequest::new(&row));
        }
        None
    }

    /// Draws the label of the selected system and its game count into
    /// `area`.
    fn show_title(&self, context: &mut UiContext, area: Rect) {
        let Some(entry) = self.current() else {
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
        let count = count_text(
            (shown_games, entry.game_count()),
            (shown_errors, entry.error_count()),
            filtering,
        );
        context.text(
            area.x + title_width + theme.gap,
            baseline,
            &theme.small(),
            &count,
        );
    }

    /// Draws the open question and the open error dialog, centered in
    /// `screen`. A delete scans the catalog again. Returns the answer of the
    /// question in this pass.
    fn show_dialogs(&mut self, context: &mut UiContext, screen: Rect) -> Option<ConfirmResponse> {
        let mut delete_response = None;
        if let Some(request) = self.delete.take() {
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
                DeleteOutcome::Failed(error) => {
                    self.dialog = Some(ErrorDialog::delete_failed(&request.name, &error));
                    self.rescan_after_delete();
                }
            }
        }
        let answer = self
            .dialog
            .as_ref()
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

    /// The widget of the library that holds the focus.
    fn library_focus(&self, context: &UiContext) -> LibraryFocus {
        let focused = |id: &str| context.has_focus(UiId::from(id));
        if focused(GAME_TABLE_ID) {
            LibraryFocus::Table
        } else if context.has_focus(game_table::header_id()) {
            LibraryFocus::Header
        } else if focused(SYSTEM_LIST_ID) {
            LibraryFocus::SystemList
        } else if focused(FILTER_ID) {
            LibraryFocus::Filter {
                has_text: !self.view.filter.is_empty(),
            }
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

    /// Draws the status line into `area`. `delete_response` is the answer
    /// of the open question in this pass.
    fn show_status_line(
        &self,
        context: &mut UiContext,
        area: Rect,
        delete_response: Option<ConfirmResponse>,
    ) {
        if let Some(request) = &self.delete {
            let hints = request.hints(delete_response.unwrap_or_default());
            status_line(context, area, &hints, StatusEnd::Path(&request.shown_path));
            return;
        }
        let selected = self.selected_row();
        let folder = self
            .current()
            .map(|entry| format!("{}/", entry.folder))
            .unwrap_or_default();
        let hints = match &self.dialog {
            Some(dialog) => dialog.hints(context),
            None => library_hints(self.library_focus(context), selected),
        };
        status_line(context, area, &hints, status_end(selected, &folder));
    }
}

/// The games and the files with errors of the system at `index` of `systems`.
fn rows_at(systems: &[SystemEntry], index: usize) -> &[GameRow] {
    systems
        .get(index)
        .map_or(&[], |entry| entry.rows.as_slice())
}

/// The plural form of `count` things of the singular `thing`, for example
/// `1 game` and `2 games`.
fn counted(count: usize, thing: &str) -> String {
    match count {
        1 => format!("1 {thing}"),
        count => format!("{count} {thing}s"),
    }
}

/// The count beside the system label: the shown and the total number of
/// `games`, and of the files with `errors`. While `filtering` the count
/// tells the shown number of the total, for example `3 of 18 games`.
fn count_text(games: (usize, usize), errors: (usize, usize), filtering: bool) -> String {
    let part = |(shown, total): (usize, usize), thing: &str| {
        if filtering {
            format!("{shown} of {}", counted(total, thing))
        } else {
            counted(total, thing)
        }
    };
    let mut text = part(games, "game");
    if errors.1 > 0 {
        text.push_str(", ");
        text.push_str(&part(errors, "file"));
        text.push_str(" with errors");
    }
    text
}

#[cfg(test)]
mod tests {
    use gui::ui::{Navigation, Viewport};
    use machine_factory::rom_status::RomStatus;

    use super::{delete::DELETE_LAYER_ID, *};
    use crate::{
        config::{Locations, SETTINGS_FILE_NAME},
        gui_test_support::GuiHarness,
    };

    /// A root folder with a settings file and game files. Dropping the value removes it.
    struct TestRoot {
        /// The root folder.
        root: PathBuf,
    }

    impl TestRoot {
        /// Creates a root folder with an empty settings file.
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir()
                .join(format!("neetan_manager_test_{}_{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).expect("root is created");
            std::fs::write(root.join(SETTINGS_FILE_NAME), "").expect("settings are written");
            Self { root }
        }

        /// Writes the game file `identity` below the config folder with `contents`.
        fn game(&self, identity: &str, contents: &str) {
            let path = self.root.join("config").join(identity);
            std::fs::create_dir_all(path.parent().expect("game files lie in a folder"))
                .expect("folder is created");
            std::fs::write(path, contents).expect("game file is written");
        }

        /// Removes the game file `identity`.
        fn remove(&self, identity: &str) {
            std::fs::remove_file(self.root.join("config").join(identity))
                .expect("game file is removed");
        }

        /// The manager of the root.
        fn manager(&self) -> Manager {
            let locations = Locations {
                current_directory: self.root.clone(),
                executable_directory: None,
                pref_path: None,
            };
            Manager::new(ManagerStart {
                request: LaunchRequest::default(),
                locations,
            })
            .expect("manager starts")
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    /// The first compiled system.
    fn first_system() -> &'static System {
        compiled_systems().next().expect("one system is compiled")
    }

    /// The titles of the entries of the game table of `manager`, with `|` for the caption.
    fn titles(manager: &Manager) -> Vec<String> {
        let rows = manager.rows();
        manager
            .entries
            .iter()
            .map(|entry| match entry {
                TableEntry::Game(index) => rows[*index].title.clone(),
                TableEntry::Broken(index) => rows[*index].file_name.clone(),
                TableEntry::Caption => "|".to_owned(),
            })
            .collect()
    }

    /// Writes the games `names` with their names as titles into the first system of `test`.
    fn games(test: &TestRoot, names: &[&str]) {
        let system = first_system();
        for name in names {
            test.game(
                &format!("{}/{name}.conf", system.name),
                &format!("meta.title = {name}\n"),
            );
        }
    }

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
            vec![test.root.join("config").join(&identity)]
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

    /// Verifies that confirm launches the selected game and that confirm on a file with an error asks to delete it.
    #[test]
    fn confirm_launches_and_an_error_file_asks_for_its_delete() {
        let test = TestRoot::new("confirm");
        let system = first_system();
        test.game(&format!("{}/a.conf", system.name), "meta.title = A\n");
        test.game(&format!("{}/b.conf", system.name), "machine = FOOBAR\n");
        let mut manager = test.manager();
        assert_eq!(titles(&manager), ["A", "|", "b.conf"]);
        let mut harness = GuiHarness::new(960, 600);
        harness.settle(|context| manager.show(context));
        harness.input.confirm = true;
        let launch = harness
            .frame(|context| manager.show(context))
            .expect("confirm launches");
        assert_eq!(launch.title, "A");
        harness.input.navigation = Some(Navigation::Down);
        harness.settle(|context| manager.show(context));
        assert_eq!(manager.view.table.list.selected, 2);
        harness.input.confirm = true;
        assert!(harness.settle(|context| manager.show(context)).is_none());
        assert!(manager.delete.is_some());
        assert_eq!(harness.state.open_modals(), 1);
        assert_eq!(
            harness.state.focused(),
            Some(UiId::from(DELETE_LAYER_ID).child("cancel"))
        );
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
        assert_eq!(dialog.message(), "a was not deleted.");
        assert_eq!(harness.state.open_modals(), 1);
        harness.input.confirm = true;
        harness.settle(|context| manager.show(context));
        assert!(manager.dialog().is_none());
    }

    /// Verifies that the Delete key is inactive while the filter holds the focus, that Ctrl+F and Y focus the filter and that confirm in the filter gives the focus to the table without a launch.
    #[test]
    fn delete_is_inactive_while_the_filter_holds_the_focus() {
        let test = TestRoot::new("shortcuts");
        games(&test, &["a", "b"]);
        let mut manager = test.manager();
        let mut harness = GuiHarness::new(960, 600);
        harness.settle(|context| manager.show(context));
        harness.input.shortcut = Some(Shortcut::ControlF);
        harness.settle(|context| manager.show(context));
        assert_eq!(harness.state.focused(), Some(UiId::from(FILTER_ID)));
        harness.input.shortcut = Some(Shortcut::Delete);
        harness.settle(|context| manager.show(context));
        assert!(manager.delete.is_none());
        assert_eq!(harness.state.open_modals(), 0);

        harness.state.set_focus(UiId::from(GAME_TABLE_ID));
        harness.settle(|context| manager.show(context));
        harness.input.action = Some(ActionButton::Y);
        harness.settle(|context| manager.show(context));
        assert_eq!(harness.state.focused(), Some(UiId::from(FILTER_ID)));
        harness.input.text = "B".to_owned();
        harness.settle(|context| manager.show(context));
        assert_eq!(titles(&manager), ["b"]);
        harness.input.confirm = true;
        assert!(harness.frame(|context| manager.show(context)).is_none());
        assert!(harness.state.take_pass_request());
        assert!(harness.settle(|context| manager.show(context)).is_none());
        assert_eq!(harness.state.focused(), Some(UiId::from(GAME_TABLE_ID)));
        harness.input.action = Some(ActionButton::Y);
        harness.settle(|context| manager.show(context));
        assert_eq!(harness.state.focused(), Some(UiId::from(FILTER_ID)));
        harness.input.cancel = true;
        harness.settle(|context| manager.show(context));
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
        let mut harness = GuiHarness::new(960, 600);
        harness.settle(|context| manager.show(context));
        assert_eq!(harness.state.focused(), Some(UiId::from(GAME_TABLE_ID)));
        harness.input.navigation = Some(Navigation::NextTab);
        harness.settle(|context| manager.show(context));
        assert_eq!(manager.view.systems.selected, 1);
        assert_eq!(harness.state.focused(), Some(UiId::from(ADD_ID)));
        harness.input.navigation = Some(Navigation::PreviousTab);
        harness.settle(|context| manager.show(context));
        assert_eq!(manager.view.systems.selected, 0);
        assert_eq!(harness.state.focused(), Some(UiId::from(ADD_ID)));
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

    /// Verifies that the system switch, the filter keys, Delete and F5 do nothing while the delete question or an error dialog is open.
    #[test]
    fn the_library_keys_do_nothing_under_a_dialog() {
        let test = TestRoot::new("keys_under_dialog");
        games(&test, &["a", "b"]);
        let mut manager = test.manager();
        let mut harness = GuiHarness::new(960, 600);
        harness.settle(|context| manager.show(context));
        let keys = |harness: &mut GuiHarness, manager: &mut Manager| {
            let focused = harness.state.focused();
            for navigation in [Navigation::NextTab, Navigation::PreviousTab] {
                harness.input.navigation = Some(navigation);
                harness.settle(|context| manager.show(context));
            }
            harness.input.shortcut = Some(Shortcut::ControlF);
            harness.settle(|context| manager.show(context));
            harness.input.action = Some(ActionButton::Y);
            harness.settle(|context| manager.show(context));
            harness.input.shortcut = Some(Shortcut::Delete);
            harness.settle(|context| manager.show(context));
            let files = manager.rows().len();
            games(&test, &["new"]);
            harness.input.shortcut = Some(Shortcut::F5);
            harness.settle(|context| manager.show(context));
            test.remove(&format!("{}/new.conf", first_system().name));
            assert_eq!(manager.rows().len(), files);
            assert_eq!(manager.view.systems.selected, 0);
            assert_eq!(harness.state.focused(), focused);
            assert_eq!(harness.state.open_modals(), 1);
        };

        harness.input.shortcut = Some(Shortcut::Delete);
        harness.settle(|context| manager.show(context));
        assert!(manager.delete.is_some());
        keys(&mut harness, &mut manager);
        assert!(manager.delete.is_some());
        harness.input.cancel = true;
        harness.settle(|context| manager.show(context));
        assert!(manager.delete.is_none());

        let launch = manager.launch(&manager.rows()[0].clone());
        let layer = manager.launch_failed(&launch, "fdd1 image not found: /a.d88");
        harness.state.open_layer(layer);
        harness.settle(|context| manager.show(context));
        keys(&mut harness, &mut manager);
        assert!(manager.dialog().is_some());
        assert!(manager.delete.is_none());
    }

    /// Verifies that a sort from the column header keeps the selected game.
    #[test]
    fn a_header_sort_keeps_the_selected_game() {
        let test = TestRoot::new("sort_keeps");
        games(&test, &["a", "b", "c"]);
        let mut manager = test.manager();
        let mut harness = GuiHarness::new(960, 600);
        harness.settle(|context| manager.show(context));
        harness.input.navigation = Some(Navigation::Down);
        harness.settle(|context| manager.show(context));
        let selected = format!("{}/b.conf", first_system().name);
        assert_eq!(manager.view.selected.as_deref(), Some(selected.as_str()));
        harness.state.set_focus(game_table::header_id());
        harness.settle(|context| manager.show(context));
        harness.input.confirm = true;
        assert!(harness.frame(|context| manager.show(context)).is_none());
        harness.settle(|context| manager.show(context));
        assert_eq!(titles(&manager), ["c", "b", "a"]);
        assert_eq!(manager.view.selected.as_deref(), Some(selected.as_str()));
        assert_eq!(manager.view.table.list.selected, 1);
        harness.input.navigation = Some(Navigation::Right);
        harness.settle(|context| manager.show(context));
        harness.input.confirm = true;
        harness.settle(|context| manager.show(context));
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
        let mut harness = GuiHarness::new(960, 600);
        harness.settle(|context| manager.show(context));
        harness.input.navigation = Some(Navigation::Down);
        harness.settle(|context| manager.show(context));
        let selected = format!("{}/b.conf", first_system().name);
        assert_eq!(manager.view.selected.as_deref(), Some(selected.as_str()));

        for press in [false, true] {
            manager.systems[0].rom_status = stale;
            let name = if press { "e" } else { "d" };
            games(&test, &[name]);
            let view = manager.view.clone();
            if press {
                harness.state.set_focus(UiId::from(REFRESH_ID));
                harness.settle(|context| manager.show(context));
                harness.input.confirm = true;
            } else {
                harness.input.shortcut = Some(Shortcut::F5);
            }
            harness.settle(|context| manager.show(context));
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
        let mut harness = GuiHarness::new(960, 600);
        harness.settle(|context| manager.show(context));
        harness.input.shortcut = Some(Shortcut::ControlF);
        harness.settle(|context| manager.show(context));
        assert_eq!(harness.state.focused(), Some(UiId::from(FILTER_ID)));
        games(&test, &["b"]);
        harness.input.shortcut = Some(Shortcut::F5);
        harness.settle(|context| manager.show(context));
        assert_eq!(titles(&manager), ["a"]);
        harness.state.set_focus(UiId::from(GAME_TABLE_ID));
        harness.settle(|context| manager.show(context));
        harness.input.shortcut = Some(Shortcut::F5);
        harness.settle(|context| manager.show(context));
        assert_eq!(titles(&manager), ["a", "b"]);
    }

    /// Verifies that the shoulder buttons and Ctrl+PgUp and Ctrl+PgDn switch the system and wrap around.
    #[test]
    fn the_shoulder_buttons_switch_the_system() {
        let test = TestRoot::new("switch");
        let mut manager = test.manager();
        let count = manager.systems.len();
        let mut harness = GuiHarness::new(960, 600);
        harness.settle(|context| manager.show(context));
        assert_eq!(manager.view.systems.selected, 0);
        harness.input.navigation = Some(Navigation::NextTab);
        harness.settle(|context| manager.show(context));
        assert_eq!(manager.view.systems.selected, 1 % count);
        assert_eq!(
            manager.view.system,
            Some(manager.systems[1 % count].system.name)
        );
        harness.input.navigation = Some(Navigation::PreviousTab);
        harness.settle(|context| manager.show(context));
        harness.input.navigation = Some(Navigation::PreviousTab);
        harness.settle(|context| manager.show(context));
        assert_eq!(manager.view.systems.selected, count - 1);

        harness.input.navigation = Some(Navigation::NextTab);
        harness.settle(|context| manager.show(context));
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
        let mut harness = GuiHarness::new(960, 600);
        harness.settle(|context| manager.show(context));
        assert_eq!(harness.state.focused(), Some(UiId::from(ADD_ID)));
        harness.state.set_focus(UiId::from(SYSTEM_LIST_ID));
        harness.settle(|context| manager.show(context));
        harness.input.navigation = Some(Navigation::Down);
        harness.settle(|context| manager.show(context));
        assert_eq!(manager.view.system, Some(second.name));
        assert_eq!(titles(&manager), ["Edge"]);
        harness.input.confirm = true;
        assert!(harness.frame(|context| manager.show(context)).is_none());
        assert!(harness.state.take_pass_request());
        assert!(harness.settle(|context| manager.show(context)).is_none());
        assert_eq!(harness.state.focused(), Some(UiId::from(GAME_TABLE_ID)));
    }

    /// Verifies that the error dialog keeps the navigation from the table, closes on confirm and returns the focus to the table.
    #[test]
    fn the_error_dialog_takes_the_input_until_it_closes() {
        let test = TestRoot::new("dialog");
        games(&test, &["A", "B"]);
        let mut manager = test.manager();
        let launch = manager.launch(&manager.rows()[0].clone());
        let layer = manager.launch_failed(&launch, "fdd1 image not found: /a.d88");
        assert_eq!(
            manager.dialog().map(ErrorDialog::message),
            Some("A did not start.")
        );
        let mut harness = GuiHarness::new(960, 600);
        harness.settle(|context| manager.show(context));
        harness.state.open_layer(layer);
        harness.settle(|context| manager.show(context));
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
        manager.game_ended(Some("core halted"), Ok(()));
        let dialog = manager.dialog().expect("panic dialog opens");
        assert_eq!(dialog.detail(), "core halted");
        assert_eq!(
            dialog.message(),
            format!(
                "The {} core stopped with an error. The disk images were saved.",
                first_system().label
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
        let layer = manager.launch_failed(&launch, "fdd1 image not found: /a.d88");
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
        manager.game_ended(
            None,
            Err(FlushError::new(
                "hard disk image /a.hdi could not be written",
            )),
        );
        let dialog = manager.dialog().expect("the media dialog opens");
        assert_eq!(
            dialog.message(),
            "A ended. Some of its disk images were not saved."
        );
        assert_eq!(
            dialog.detail(),
            "hard disk image /a.hdi could not be written"
        );

        let mut manager = test.manager();
        manager.game_ended(None, Ok(()));
        assert!(manager.dialog().is_none());
    }

    /// Verifies the count beside the system label with and without a filter.
    #[test]
    fn the_count_tells_the_games_and_the_files_with_errors() {
        assert_eq!(count_text((18, 18), (0, 0), false), "18 games");
        assert_eq!(count_text((1, 1), (0, 0), false), "1 game");
        assert_eq!(count_text((0, 0), (0, 0), false), "0 games");
        assert_eq!(
            count_text((5, 5), (2, 2), false),
            "5 games, 2 files with errors"
        );
        assert_eq!(count_text((0, 18), (0, 0), true), "0 of 18 games");
        assert_eq!(
            count_text((1, 5), (1, 1), true),
            "1 of 5 games, 1 of 1 file with errors"
        );
    }

    /// The sample games of the screenshots: the system, the file stem, the
    /// title, the original title, the developer and the year.
    const SAMPLE_GAMES: [(&str, &str, &str, &str, &str, &str); 22] = [
        (
            "pc98",
            "brandish",
            "Brandish",
            "ブランディッシュ",
            "Nihon Falcom",
            "1991",
        ),
        ("pc98", "dokyusei", "Dokyusei", "同級生", "Elf", "1992"),
        (
            "pc98",
            "dragon-slayer",
            "Dragon Slayer: The Legend of Heroes",
            "ドラゴンスレイヤー英雄伝説",
            "Nihon Falcom",
            "1989",
        ),
        ("pc98", "edge", "Edge", "エッジ", "TGL", "1993"),
        (
            "pc98",
            "geneitoshi",
            "Gen'ei Toshi: Illusion City",
            "幻影都市",
            "Microcabin",
            "1991",
        ),
        (
            "pc98",
            "lord-monarch",
            "Lord Monarch",
            "ロードモナーク",
            "Nihon Falcom",
            "1991",
        ),
        (
            "pc98",
            "policenauts",
            "Policenauts",
            "ポリスノーツ",
            "Konami",
            "1994",
        ),
        (
            "pc98",
            "princess-maker-2",
            "Princess Maker 2",
            "プリンセスメーカー2",
            "Gainax",
            "1993",
        ),
        (
            "pc98",
            "puyo-puyo",
            "Puyo Puyo",
            "ぷよぷよ",
            "Compile",
            "1993",
        ),
        (
            "pc98",
            "sangokushi-2",
            "Romance of the Three Kingdoms II",
            "三國志II",
            "Koei",
            "1989",
        ),
        ("pc98", "rusty", "Rusty", "ラスティ", "C-Lab", "1993"),
        (
            "pc98",
            "tokimeki",
            "Tokimeki Memorial",
            "ときめきメモリアル",
            "Konami",
            "1995",
        ),
        (
            "pc98",
            "touhou-fuumaroku",
            "Touhou Fuumaroku: Story of Eastern Wonderland",
            "東方封魔録",
            "Amusement Makers",
            "1997",
        ),
        (
            "pc98",
            "touhou-gensoukyou",
            "Touhou Gensoukyou: Lotus Land Story",
            "東方幻想郷",
            "Amusement Makers",
            "1998",
        ),
        (
            "pc98",
            "touhou-reiiden",
            "Touhou Reiiden: Highly Responsive to Prayers",
            "東方靈異伝",
            "Amusement Makers",
            "1997",
        ),
        (
            "pc98",
            "touhou-yumejikuu",
            "Touhou Yumejikuu: Phantasmagoria of Dim.Dream",
            "東方夢時空",
            "Amusement Makers",
            "1997",
        ),
        (
            "pc98",
            "ys-2",
            "Ys II: Ancient Ys Vanished The Final Chapter",
            "イースII",
            "Nihon Falcom",
            "1988",
        ),
        (
            "pc98",
            "yu-no",
            "YU-NO: A Girl Who Chants Love at the Bound of this World",
            "この世の果てで恋を唄う少女YU-NO",
            "Elf",
            "1996",
        ),
        (
            "x68k",
            "akumajou-dracula",
            "Akumajou Dracula",
            "悪魔城ドラキュラ",
            "Konami",
            "1993",
        ),
        (
            "x68k",
            "final-fight",
            "Final Fight",
            "ファイナルファイト",
            "Capcom",
            "1992",
        ),
        (
            "x68k",
            "gradius-2",
            "Gradius II",
            "グラディウスII",
            "Konami",
            "1992",
        ),
        ("pc88", "ys-3", "", "", "", ""),
    ];

    /// Writes the sample games and two files with errors into `test`.
    fn sample_root(test: &TestRoot) {
        for (system, stem, title, original, developer, year) in SAMPLE_GAMES {
            let mut contents = String::new();
            for (key, value) in [
                ("meta.title", title),
                ("meta.title-original", original),
                ("meta.developer", developer),
                ("meta.year", year),
            ] {
                if !value.is_empty() {
                    contents.push_str(&format!("{key} = {value}\n"));
                }
            }
            test.game(&format!("{system}/{stem}.conf"), &contents);
        }
        test.game(
            "x68k/nemesis-90.conf",
            "meta.title = Nemesis '90\nmachine = X68030\n",
        );
        test.game(
            "x68k/cyberblock-metal-orange.conf",
            "meta.title = Cyber Block\nmachine = PC9801RA\n",
        );
    }

    /// Selects the system `name` in `manager`.
    fn select(manager: &mut Manager, name: &str) {
        let index = manager
            .systems
            .iter()
            .position(|entry| entry.system.name == name)
            .expect("the system is compiled");
        manager.select_system(index);
    }

    /// Renders the library at `width` x `height` logical units and `scale`:
    /// the library with data, a filtered library, an empty system, a system
    /// with files with errors and the delete question.
    fn screenshots(width: f32, height: f32, scale: f32, prefix: &str) {
        let viewport = Viewport::new((width * scale) as u32, (height * scale) as u32, scale);
        let test = TestRoot::new(&format!("screenshots_{prefix}_{scale}"));
        sample_root(&test);
        let mut manager = test.manager();
        let mut harness = GuiHarness::with_viewport(viewport);
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
            "fdd1 image not found: /home/user/Games/Neetan/games/98/YU-NO/YU-NO (Disk 3 of 4).hdm",
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

    /// Writes the screenshots of the manager at the scales 1 and 2, at the
    /// minimum window size and at 1920 x 1080.
    #[test]
    #[ignore = "writes screenshots into the directory of GUI_SHOTS"]
    fn manager_screenshots() {
        for scale in [1.0, 2.0] {
            screenshots(960.0, 600.0, scale, "");
            screenshots(1920.0, 1080.0, scale, "large_");
        }
    }
}
