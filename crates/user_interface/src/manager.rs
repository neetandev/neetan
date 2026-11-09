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

use std::{convert::Infallible, path::PathBuf, result};

use common::{FlushError, debug, info};
use gui::{
    Rect,
    component::{ChildEvent, Component, Ended, Status, Update, run_child},
    ui::{ActionButton, RowKind, STATUS_LINE_HEIGHT, Shortcut, UiContext, UiId},
};
use machine_factory::config::systems::{System, compiled_systems};

use self::{
    delete::DeleteRequest,
    entries::{TableEntry, table_entries},
    error_dialog::{EditTarget, ErrorChoice},
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
    editor::{Editor, EditorEffect},
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

/// What the manager asks the shell to do.
#[derive(Debug)]
pub(crate) enum ShellEffect {
    /// Start this game.
    Launch(Launch),
    /// Switch the user interface to this language.
    SwitchCatalog(LanguageCatalog),
}

/// What the user can ask the manager to do.
#[derive(Debug)]
pub(crate) enum ManagerMessage {
    /// Selects the system at this index of the system list.
    SelectSystem(usize),
    /// The text of the filter field changed.
    FilterChanged,
    /// The sort of the game table changed.
    SortChanged,
    /// The selection of the game table changed.
    SelectionChanged,
    /// Checks the ROM folders and scans the catalog again.
    Refresh,
    /// Opens the editor on a new game of the selected system.
    AddGame,
    /// Starts the selected game.
    Launch,
    /// Opens the selected game in the editor.
    Edit,
    /// Opens a copy of the selected game in the editor.
    Duplicate,
    /// Asks before the delete of the file of the selected row.
    AskDelete,
    /// Opens the settings page.
    OpenSettings,
    /// Opens the editor on the system defaults of the selected system.
    OpenSystemDefaults,
    /// The question before the delete ended. `Done` confirms the delete.
    Delete(Ended<()>),
    /// The error dialog ended.
    Error(Ended<ErrorChoice>),
    /// The editor reported an effect or ended.
    Editor(ChildEvent<EditorEffect, ()>),
}

/// What the manager shows.
enum ManagerMode {
    /// The library with the systems and their games.
    Library,
    /// The editor of a game file, a system defaults file or the settings
    /// file. It takes the whole window while it is open.
    Editor(Box<Editor>),
}

/// The open dialog of the manager.
enum ManagerOverlay {
    /// No dialog is open.
    None,
    /// The question before the delete of a game file is open.
    Delete(Box<DeleteRequest>),
    /// The error dialog is open.
    Error(Box<ErrorDialog>),
}

/// A focus request of the library for the next pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManagerFocus {
    /// The game table, or the add button while the table has no entry.
    Start,
    /// The widget with this id.
    Widget(UiId),
}

/// The game manager.
pub(crate) struct Manager {
    /// The message buffer.
    messages: Vec<ManagerMessage>,
    /// The command line and the folders of the settings discovery.
    start: ManagerStart,
    /// The settings file and the command line flags.
    host: HostLayers,
    /// The config folder.
    config_folder: PathBuf,
    /// The shown systems with their games and their ROM status.
    systems: Vec<SystemEntry>,
    /// The ROM status of every ROM folder from the check of the start or of
    /// the last Refresh.
    roms: Vec<RomFolderStatus>,
    /// The entries of the game table of the selected system.
    entries: Vec<TableEntry>,
    /// The index of the system whose rows the entries hold. The view pairs the rows of this
    /// system with the entries until the update of a system switch builds them again.
    entries_system: usize,
    /// Whether a filter text chose the entries.
    filtering: bool,
    /// The row kinds of the entries in the game table. They refill with the
    /// entries and keep their buffer.
    row_kinds: Vec<RowKind>,
    /// The selection, the sort, the filter and the scroll positions.
    view: ManagerView,
    /// The library or the open editor.
    mode: ManagerMode,
    /// The open dialog.
    overlay: ManagerOverlay,
    /// The widget of the library that takes the focus in the next pass.
    focus: Option<ManagerFocus>,
    /// The game that runs.
    running: Option<Launch>,
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
        let mut manager = Self {
            messages: Vec::new(),
            start,
            host,
            config_folder,
            systems: Vec::new(),
            roms,
            entries: Vec::new(),
            entries_system: 0,
            filtering: false,
            row_kinds: Vec::new(),
            view: ManagerView::default(),
            mode: ManagerMode::Library,
            overlay: ManagerOverlay::None,
            focus: Some(ManagerFocus::Start),
            running: None,
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

    /// Scans the catalog again, builds the shown systems with their games
    /// and restores the view on them.
    fn rescan(&mut self) {
        let catalog = Catalog::scan(&self.host);
        let root = self.host.root().to_path_buf();
        self.systems = compiled_systems()
            .filter_map(|system| {
                let mut entry = SystemEntry::new(system, &self.host, &self.roms);
                let scanned = catalog.system(system.name);
                entry.rows = scanned
                    .map(|scanned| system_rows(scanned, &root))
                    .unwrap_or_default();
                let configured = !entry.rows.is_empty()
                    || scanned.is_some_and(|scanned| scanned.defaults_file.is_file());
                entry.shown(configured).then_some(entry)
            })
            .collect();
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

    /// The games and the files with errors of the system of the entries.
    fn rows(&self) -> &[GameRow] {
        rows_at(&self.systems, self.entries_system)
    }

    /// Builds the entries of the game table of the selected system from the
    /// sort and the filter of the view.
    fn refresh_entries(&mut self) {
        let sort = self.view.table.sort.unwrap_or(entries::DEFAULT_SORT);
        let filter = Filter::new(&self.view.filter);
        self.entries_system = self.view.systems.selected;
        self.entries = table_entries(self.rows(), sort, &filter);
        self.filtering = !filter.is_empty();
        self.row_kinds.clear();
        self.row_kinds
            .extend(self.entries.iter().map(|entry| entry.kind()));
    }

    /// The row of the selected entry of the game table.
    fn selected_row(&self) -> Option<&GameRow> {
        self.view.selected_row(&self.entries, self.rows())
    }

    /// The row of the selected entry of the game table when its game starts.
    fn launchable_row(&self) -> Option<&GameRow> {
        self.selected_row().filter(|row| row.launchable())
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
        let rows = rows_at(&self.systems, self.entries_system);
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
    ) -> result::Result<(EmulatorConfig, KeyOverrides), LaunchError> {
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
        flush: result::Result<(), FlushError>,
    ) -> Option<UiId> {
        let running = self.running.take();
        self.focus = Some(ManagerFocus::Start);
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
        self.overlay = ManagerOverlay::Error(Box::new(dialog));
        UiId::from(error_dialog::LAYER_ID)
    }

    /// Opens `dialog` with its layer from an update.
    fn show_error(&mut self, dialog: Box<ErrorDialog>, update: &mut Update<'_, ShellEffect>) {
        self.overlay = ManagerOverlay::Error(dialog);
        update.open::<ErrorDialog>();
    }

    /// The open error dialog.
    #[cfg(test)]
    pub(crate) fn dialog(&self) -> Option<&ErrorDialog> {
        match &self.overlay {
            ManagerOverlay::Error(dialog) => Some(dialog),
            ManagerOverlay::None | ManagerOverlay::Delete(_) => None,
        }
    }

    /// Draws one pass of the library into `area`, the whole window, with the
    /// open dialog over it and the status line at the bottom.
    fn view_library(
        &mut self,
        context: &mut UiContext,
        area: Rect,
        messages: &mut Vec<ManagerMessage>,
    ) {
        let theme = context.theme;
        context.fill(area, theme.colors.panel);
        let (status_area, screen) = area.cut_bottom(STATUS_LINE_HEIGHT);
        let (sidebar_area, main_area) = screen.cut_left(SIDEBAR_WIDTH);

        let start_focus = if self.has_entries() {
            UiId::from(GAME_TABLE_ID)
        } else {
            UiId::from(ADD_ID)
        };
        match self.focus.take() {
            Some(ManagerFocus::Start) => context.set_focus(start_focus),
            Some(ManagerFocus::Widget(id)) => context.set_focus(id),
            None => context.default_focus(start_focus),
        }
        if context.shortcut() == Some(Shortcut::ControlF)
            || context.action_button() == Some(ActionButton::Y)
        {
            context.set_focus(UiId::from(FILTER_ID));
        }

        self.show_systems(context, sidebar_area, messages);
        self.show_main(context, main_area, messages);
        let dialog_open = self.show_dialog(context, screen, messages);
        self.show_status_line(context, status_area, dialog_open);
    }

    /// Draws the open dialog centered in `screen`. Returns whether a dialog
    /// stays open after the pass.
    fn show_dialog(
        &mut self,
        context: &mut UiContext,
        screen: Rect,
        messages: &mut Vec<ManagerMessage>,
    ) -> bool {
        match &mut self.overlay {
            ManagerOverlay::None => false,
            ManagerOverlay::Delete(request) => {
                run_child(&mut **request, context, screen, (), messages, |event| {
                    ManagerMessage::Delete(event.into_ended())
                })
            }
            ManagerOverlay::Error(dialog) => {
                run_child(&mut **dialog, context, screen, (), messages, |event| {
                    ManagerMessage::Error(event.into_ended())
                })
            }
        }
    }

    /// Opens the question before the delete of the file of the selected row.
    fn ask_delete(&mut self, update: &mut Update<'_, ShellEffect>) {
        let Some(row) = self.selected_row() else {
            return;
        };
        self.overlay = ManagerOverlay::Delete(Box::new(DeleteRequest::new(row)));
        update.open::<DeleteRequest>();
    }

    /// Takes the end `ended` of the question before a delete. A confirmed
    /// delete removes the game file and scans the catalog again. A failed
    /// removal opens the error dialog.
    fn delete_ended(&mut self, ended: Ended<()>, update: &mut Update<'_, ShellEffect>) {
        let overlay = std::mem::replace(&mut self.overlay, ManagerOverlay::None);
        if let (ManagerOverlay::Delete(request), Ended::Done(())) = (overlay, ended) {
            if let Some(dialog) = remove_game_file(&request) {
                self.show_error(Box::new(dialog), update);
            }
            self.rescan_after_delete();
        }
    }

    /// Takes the end `ended` of the error dialog. Edit opens the game of the
    /// dialog in the editor over the library, and Overwrite saves the changes
    /// of the editor over its file. Over an open editor, Edit only closes the
    /// dialog.
    fn error_ended(&mut self, ended: Ended<ErrorChoice>, update: &mut Update<'_, ShellEffect>) {
        let overlay = std::mem::replace(&mut self.overlay, ManagerOverlay::None);
        let ManagerOverlay::Error(dialog) = overlay else {
            return;
        };
        match ended {
            Ended::Closed => {}
            Ended::Done(ErrorChoice::Edit) => {
                if matches!(self.mode, ManagerMode::Library)
                    && let Some(target) = dialog.edit_target()
                {
                    self.open_target(target, update);
                }
            }
            Ended::Done(ErrorChoice::Overwrite) => self.overwrite(update),
        }
    }
}

impl Component for Manager {
    type Message = ManagerMessage;
    type Output = Infallible;
    type Effect = ShellEffect;
    type Input<'a> = ();

    fn messages(&mut self) -> &mut Vec<ManagerMessage> {
        &mut self.messages
    }

    /// Draws one pass of the manager into `area`, the whole window: the
    /// library or the open editor.
    fn view(
        &mut self,
        context: &mut UiContext,
        area: Rect,
        _input: (),
        messages: &mut Vec<ManagerMessage>,
    ) {
        match self.mode {
            ManagerMode::Library => self.view_library(context, area, messages),
            ManagerMode::Editor(_) => self.view_editor(context, area, messages),
        }
    }

    fn update(
        &mut self,
        message: ManagerMessage,
        update: &mut Update<'_, ShellEffect>,
    ) -> Status<Infallible> {
        match message {
            ManagerMessage::SelectSystem(index) => self.select_system(index),
            ManagerMessage::FilterChanged => {
                self.refresh_entries();
                let rows = rows_at(&self.systems, self.entries_system);
                self.view.refilter(&self.entries, rows);
            }
            ManagerMessage::SortChanged => {
                self.refresh_entries();
                let rows = rows_at(&self.systems, self.entries_system);
                self.view.restore(&self.entries, rows);
            }
            ManagerMessage::SelectionChanged => {
                let rows = rows_at(&self.systems, self.entries_system);
                self.view.record(&self.entries, rows);
            }
            ManagerMessage::Refresh => self.refresh(),
            ManagerMessage::AddGame => self.open_new_game(update),
            ManagerMessage::Launch => {
                if let Some(row) = self.launchable_row() {
                    update.emit(ShellEffect::Launch(self.launch(row)));
                }
            }
            ManagerMessage::Edit => {
                if let Some(row) = self.launchable_row().cloned() {
                    self.open_game(&row, update);
                }
            }
            ManagerMessage::Duplicate => {
                if let Some(row) = self.launchable_row().cloned() {
                    self.open_duplicate(&row, update);
                }
            }
            ManagerMessage::AskDelete => self.ask_delete(update),
            ManagerMessage::OpenSettings => self.open_settings(update),
            ManagerMessage::OpenSystemDefaults => self.open_system_defaults(update),
            ManagerMessage::Delete(ended) => self.delete_ended(ended, update),
            ManagerMessage::Error(ended) => self.error_ended(ended, update),
            ManagerMessage::Editor(ChildEvent::Effect(effect)) => {
                self.editor_effect(effect, update);
            }
            ManagerMessage::Editor(ChildEvent::Ended(_)) => self.editor_ended(),
        }
        Status::Open
    }
}

/// Removes the game file of `request`. A failed removal returns the error
/// dialog that reports it.
fn remove_game_file(request: &DeleteRequest) -> Option<ErrorDialog> {
    match std::fs::remove_file(&request.path) {
        Ok(()) => {
            info!("Game manager: deleted {}", request.path.display());
            None
        }
        Err(error) => Some(ErrorDialog::delete_failed(
            &request.name,
            request.shown_path.clone(),
            error.to_string(),
        )),
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
    use common::temporary::TemporaryFolder;
    use gui::{
        component::run,
        ui::{EditCommand, EditModifiers, Navigation, ThemeKind, Viewport},
    };
    use machine_factory::rom_status::RomStatus;

    use super::{
        delete::DELETE_LAYER_ID,
        rows::tests::game,
        sidebar::SYSTEM_LIST_ID,
        test_support::{
            TestRoot, delete_open, first_system, frame, games, sample_root, select, settle,
            steady_frame, titles,
        },
        *,
    };
    use crate::{
        editor::SaveFailure,
        gui_test_support::{
            GuiHarness, MINIMUM_HEIGHT, MINIMUM_SCREEN, MINIMUM_WIDTH, PassCounts,
            apply_with_effects,
        },
        texts::repository::{RepositoryLanguage, repository_languages},
    };

    /// The names of the shown systems of `manager`.
    fn shown_systems(manager: &Manager) -> Vec<&'static str> {
        manager
            .systems
            .iter()
            .map(|entry| entry.system.name)
            .collect()
    }

    /// Verifies that a system without the ROMs it needs shows only with a system defaults file or
    /// a file in its folder, and that a refresh picks up both.
    #[test]
    fn a_system_without_needed_roms_shows_only_with_configuration() {
        let Some(required) = compiled_systems().find(|system| system.roms_required) else {
            return;
        };
        let test = TestRoot::new("hidden_system");
        let mut manager = test.manager();
        assert!(!shown_systems(&manager).contains(&required.name));
        for system in compiled_systems().filter(|system| !system.roms_required) {
            assert!(shown_systems(&manager).contains(&system.name));
        }

        test.game(&format!("{}.conf", required.name), "");
        manager.refresh();
        assert!(shown_systems(&manager).contains(&required.name));
        let entry = manager
            .systems
            .iter()
            .find(|entry| entry.system.name == required.name)
            .expect("the system shows");
        assert_eq!(entry.rom_status, RomStatus::Missing);

        test.remove(&format!("{}.conf", required.name));
        manager.refresh();
        assert!(!shown_systems(&manager).contains(&required.name));

        test.game(
            &format!("{}/edge.conf", required.name),
            "machine = FOOBAR\n",
        );
        manager.refresh();
        assert!(shown_systems(&manager).contains(&required.name));
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
        let mut harness = GuiHarness::minimum();
        settle(&mut harness, &mut manager);
        harness.input.navigation = Some(Navigation::End);
        settle(&mut harness, &mut manager);
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
        let mut harness = GuiHarness::minimum();
        settle(&mut harness, &mut manager);
        manager.view.table.sort_by(entries::TITLE_COLUMN);
        manager.refresh_entries();
        manager.view.filter = "game 1".to_owned();
        manager.refresh_entries();
        manager.view.table.list.select(5);
        let rows = manager.rows().to_vec();
        manager.view.record(&manager.entries, &rows);
        settle(&mut harness, &mut manager);
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
        let mut harness = GuiHarness::minimum();
        settle(&mut harness, &mut manager);
        harness.input.navigation = Some(Navigation::End);
        settle(&mut harness, &mut manager);

        harness.input.edit = Some(EditCommand::DeleteForward);
        settle(&mut harness, &mut manager);
        assert!(delete_open(&manager));
        harness.input.confirm = true;
        settle(&mut harness, &mut manager);
        assert!(!delete_open(&manager));
        assert_eq!(titles(&manager), ["a", "b", "c"]);
        assert_eq!(harness.state.focused(), Some(UiId::from(GAME_TABLE_ID)));

        harness.input.edit = Some(EditCommand::DeleteForward);
        settle(&mut harness, &mut manager);
        harness.input.cancel = true;
        settle(&mut harness, &mut manager);
        assert!(!delete_open(&manager));
        assert_eq!(titles(&manager), ["a", "b", "c"]);

        harness.input.edit = Some(EditCommand::DeleteForward);
        settle(&mut harness, &mut manager);
        harness.input.navigation = Some(Navigation::Right);
        settle(&mut harness, &mut manager);
        harness.input.confirm = true;
        settle(&mut harness, &mut manager);
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
        let mut harness = GuiHarness::minimum();
        settle(&mut harness, &mut manager);
        harness.input.edit = Some(EditCommand::DeleteForward);
        settle(&mut harness, &mut manager);
        test.remove(&format!("{}/a.conf", first_system().name));
        harness.input.navigation = Some(Navigation::Right);
        settle(&mut harness, &mut manager);
        harness.input.confirm = true;
        settle(&mut harness, &mut manager);
        let dialog = manager.dialog().expect("the error dialog opens");
        assert_eq!(dialog.message(), Some("a was not deleted."));
        assert_eq!(harness.state.open_modals(), 1);
        harness.input.confirm = true;
        settle(&mut harness, &mut manager);
        assert!(manager.dialog().is_none());
    }

    /// The delete request of the game file `name` in `folder`.
    fn delete_request(folder: &TemporaryFolder, name: &str) -> DeleteRequest {
        DeleteRequest::new(&GameRow {
            path: folder.path().join(name),
            ..game("edge", "Edge", "", "", "")
        })
    }

    /// Verifies that the delete of a game file that is a symbolic link removes the link and keeps its target.
    #[cfg(unix)]
    #[test]
    fn a_link_loses_only_itself() {
        let folder = TemporaryFolder::new("manager_delete_link");
        folder.write_file("target.conf", b"meta.title = Edge\n");
        let target = folder.path().join("target.conf");
        std::os::unix::fs::symlink(&target, folder.path().join("edge.conf")).expect("link");
        assert!(remove_game_file(&delete_request(&folder, "edge.conf")).is_none());
        assert!(std::fs::symlink_metadata(folder.path().join("edge.conf")).is_err());
        assert!(target.is_file());
    }

    /// Verifies that a file that cannot be removed reports the error with its path.
    #[test]
    fn a_failed_removal_reports_the_error() {
        let folder = TemporaryFolder::new("manager_delete_failed");
        let request = delete_request(&folder, "edge.conf");
        let mut dialog = remove_game_file(&request).expect("the removal fails");
        GuiHarness::minimum()
            .frame(|context| run(&mut dialog, context, MINIMUM_SCREEN, (), &mut |_| {}));
        assert_eq!(dialog.message(), Some("Edge was not deleted."));
        let detail = dialog.detail().unwrap_or_default();
        let error = detail
            .strip_prefix(&format!("{}: ", request.shown_path))
            .unwrap_or_else(|| panic!("the detail starts with the path: {detail}"));
        assert!(!error.is_empty());
    }

    /// Verifies that the delete of the last game gives the focus to Add.
    #[test]
    fn the_delete_of_the_last_game_focuses_add() {
        let test = TestRoot::new("delete_last");
        games(&test, &["a"]);
        let mut manager = test.manager();
        let mut harness = GuiHarness::minimum();
        settle(&mut harness, &mut manager);
        harness.input.edit = Some(EditCommand::DeleteForward);
        settle(&mut harness, &mut manager);
        harness.input.navigation = Some(Navigation::Right);
        settle(&mut harness, &mut manager);
        harness.input.confirm = true;
        settle(&mut harness, &mut manager);
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
        let mut harness = GuiHarness::minimum();
        settle(&mut harness, &mut manager);
        harness.input.navigation = Some(Navigation::Down);
        settle(&mut harness, &mut manager);
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
        let mut harness = GuiHarness::minimum();
        settle(&mut harness, &mut manager);
        harness.state.open_layer(layer);
        settle(&mut harness, &mut manager);
        assert_eq!(
            manager.dialog().and_then(ErrorDialog::message),
            Some("A did not start.")
        );
        let selected = manager.view.table.list.selected;
        harness.input.navigation = Some(Navigation::Down);
        assert!(frame(&mut harness, &mut manager).is_empty());
        assert_eq!(manager.view.table.list.selected, selected);
        assert!(manager.dialog().is_some());
        harness.input.confirm = true;
        assert!(settle(&mut harness, &mut manager).is_empty());
        assert!(manager.dialog().is_none());
        assert_eq!(harness.state.focused(), Some(UiId::from(GAME_TABLE_ID)));
        harness.input.navigation = Some(Navigation::Down);
        settle(&mut harness, &mut manager);
        assert_eq!(manager.view.table.list.selected, selected + 1);

        manager.game_started(launch);
        let layer = manager
            .game_ended(Some("core halted"), Ok(()))
            .expect("the panic dialog opens");
        harness.state.open_layer(layer);
        settle(&mut harness, &mut manager);
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
        let mut harness = GuiHarness::minimum();
        settle(&mut harness, &mut manager);
        let selected = manager.view.table.list.selected;
        let launch = manager.launch(&manager.rows()[0].clone());
        let layer = manager.launch_failed(&launch, &LaunchFailure::missing_image("/a.d88"));
        harness.state.open_layer(layer);
        for navigation in [Navigation::Down, Navigation::PageDown] {
            harness.input.navigation = Some(navigation);
            assert!(frame(&mut harness, &mut manager).is_empty());
            assert_eq!(manager.view.table.list.selected, selected);
        }
        harness.input.confirm = true;
        assert!(frame(&mut harness, &mut manager).is_empty());
        assert!(manager.dialog().is_none());
        harness.input.navigation = Some(Navigation::Down);
        settle(&mut harness, &mut manager);
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
        let mut harness = GuiHarness::minimum();
        harness.state.open_layer(layer);
        settle(&mut harness, &mut manager);
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

    /// The id of the modal layer of the error dialog.
    fn error_layer() -> UiId {
        UiId::from(error_dialog::LAYER_ID)
    }

    /// Verifies that the launch message emits the launch of the selected game and that a file
    /// with an error starts nothing.
    #[test]
    fn the_launch_message_emits_the_launch_of_the_selected_game() {
        let test = TestRoot::new("update_launch");
        let system = first_system();
        test.game(&format!("{}/a.conf", system.name), "meta.title = A\n");
        test.game(&format!("{}/b.conf", system.name), "machine = FOOBAR\n");
        let mut manager = test.manager();
        let (status, layers, effects) = apply_with_effects(&mut manager, ManagerMessage::Launch);
        assert_eq!((status, layers), (Status::Open, Vec::new()));
        let [ShellEffect::Launch(launch)] = effects.as_slice() else {
            panic!("the message launches: {effects:?}");
        };
        assert_eq!(launch.title, "A");
        assert_eq!(launch.request.game_files, [manager.rows()[0].path.clone()]);

        manager.view.table.list.select(2);
        let (_, _, effects) = apply_with_effects(&mut manager, ManagerMessage::Launch);
        assert!(effects.is_empty(), "{effects:?}");
    }

    /// Verifies that the filter message rebuilds the entries from the filter text and keeps the
    /// selected game when the filter shows it.
    #[test]
    fn the_filter_message_rebuilds_the_entries() {
        let test = TestRoot::new("update_filter");
        games(&test, &["alpha", "beta", "gamma"]);
        let mut manager = test.manager();
        manager.view.table.list.select(1);
        apply_with_effects(&mut manager, ManagerMessage::SelectionChanged);
        manager.view.filter = "ta".to_owned();
        apply_with_effects(&mut manager, ManagerMessage::FilterChanged);
        assert_eq!(titles(&manager), ["beta"]);
        let selected = format!("{}/beta.conf", first_system().name);
        assert_eq!(manager.view.selected.as_deref(), Some(selected.as_str()));
        assert_eq!(manager.view.table.list.selected, 0);
    }

    /// Verifies that the delete message opens the question with its layer, that a kept file
    /// stays, and that a confirmed delete removes the file and scans the catalog again.
    #[test]
    fn the_delete_messages_ask_first_and_remove_the_file() {
        let test = TestRoot::new("update_delete");
        games(&test, &["a", "b"]);
        let mut manager = test.manager();
        let (status, layers, effects) = apply_with_effects(&mut manager, ManagerMessage::AskDelete);
        assert_eq!(status, Status::Open);
        assert_eq!(layers, [UiId::from(DELETE_LAYER_ID)]);
        assert!(effects.is_empty());
        assert!(delete_open(&manager));
        let (_, layers, _) =
            apply_with_effects(&mut manager, ManagerMessage::Delete(Ended::Closed));
        assert!(layers.is_empty());
        assert!(!delete_open(&manager));
        assert_eq!(titles(&manager), ["a", "b"]);

        apply_with_effects(&mut manager, ManagerMessage::AskDelete);
        let (_, layers, _) =
            apply_with_effects(&mut manager, ManagerMessage::Delete(Ended::Done(())));
        assert!(layers.is_empty());
        assert!(!delete_open(&manager));
        assert!(manager.dialog().is_none());
        assert_eq!(titles(&manager), ["b"]);
        let path = test.root.path().join("config").join(first_system().name);
        assert!(!path.join("a.conf").exists());
    }

    /// Verifies that a confirmed delete whose removal fails opens the error dialog with its
    /// layer.
    #[test]
    fn a_failed_removal_opens_the_error_dialog_with_its_layer() {
        let test = TestRoot::new("update_delete_failed");
        games(&test, &["a"]);
        let mut manager = test.manager();
        apply_with_effects(&mut manager, ManagerMessage::AskDelete);
        test.remove(&format!("{}/a.conf", first_system().name));
        let (_, layers, _) =
            apply_with_effects(&mut manager, ManagerMessage::Delete(Ended::Done(())));
        assert_eq!(layers, [error_layer()]);
        assert_eq!(
            manager.dialog().map(ErrorDialog::title),
            Some(crate::texts::errors::DELETE_FAILED_TITLE)
        );
        assert!(!delete_open(&manager));
    }

    /// Verifies that both ends of the editor give the library back with the focus request of
    /// the edited file: the start widget after a game, the system defaults button after the
    /// system defaults and the settings button after the settings page.
    #[test]
    fn the_end_of_the_editor_returns_to_the_library_with_its_focus() {
        let test = TestRoot::new("update_editor_end");
        games(&test, &["a"]);
        let mut manager = test.manager();
        let cases = [
            (ManagerMessage::Edit, Ended::Closed, ManagerFocus::Start),
            (
                ManagerMessage::OpenSystemDefaults,
                Ended::Done(()),
                ManagerFocus::Widget(UiId::from(sidebar::SYSTEM_DEFAULTS_ID)),
            ),
            (
                ManagerMessage::OpenSettings,
                Ended::Closed,
                ManagerFocus::Widget(UiId::from(SETTINGS_ID)),
            ),
        ];
        for (open, ended, focus) in cases {
            manager.focus = None;
            let (_, layers, effects) = apply_with_effects(&mut manager, open);
            assert!(layers.is_empty());
            assert!(effects.is_empty());
            assert!(manager.editor().is_some());
            let message = ManagerMessage::Editor(ChildEvent::Ended(ended));
            apply_with_effects(&mut manager, message);
            assert!(manager.editor().is_none());
            assert_eq!(manager.focus, Some(focus));
        }
    }

    /// Verifies that a failed save of the editor opens the error dialog of the file with its
    /// layer over the editor, and that the end of the dialog keeps the editor.
    #[test]
    fn a_failed_save_opens_the_error_dialog_over_the_editor() {
        let test = TestRoot::new("update_save_failed");
        games(&test, &["a"]);
        let mut manager = test.manager();
        apply_with_effects(&mut manager, ManagerMessage::Edit);
        let failure = SaveFailure::NoFreeName(test.root.path().to_path_buf());
        let message = ManagerMessage::Editor(ChildEvent::Effect(EditorEffect::SaveFailed(failure)));
        let (_, layers, effects) = apply_with_effects(&mut manager, message);
        assert_eq!(layers, [error_layer()]);
        assert!(effects.is_empty());
        assert_eq!(
            manager.dialog().map(ErrorDialog::title),
            Some(crate::texts::errors::GAME_SAVE_FAILED_TITLE)
        );
        assert!(manager.editor().is_some());
        apply_with_effects(&mut manager, ManagerMessage::Error(Ended::Closed));
        assert!(manager.dialog().is_none());
        assert!(manager.editor().is_some());
    }

    /// Verifies that the Edit of an error dialog opens its game over the library, and that over
    /// an open editor it only closes the dialog and keeps the editor.
    #[test]
    fn the_edit_of_an_error_dialog_keeps_an_open_editor() {
        let test = TestRoot::new("update_error_edit");
        games(&test, &["a", "b"]);
        let mut manager = test.manager();
        let launch = manager.launch(&manager.rows()[1].clone());
        manager.launch_failed(&launch, &LaunchFailure::missing_image("/b.d88"));
        apply_with_effects(
            &mut manager,
            ManagerMessage::Error(Ended::Done(ErrorChoice::Edit)),
        );
        assert!(manager.dialog().is_none());
        assert_eq!(
            manager
                .editor()
                .map(|editor| editor.edited_file().display_name()),
            Some("b".to_owned())
        );

        manager.mode = ManagerMode::Library;
        apply_with_effects(&mut manager, ManagerMessage::Edit);
        manager.launch_failed(&launch, &LaunchFailure::missing_image("/b.d88"));
        apply_with_effects(
            &mut manager,
            ManagerMessage::Error(Ended::Done(ErrorChoice::Edit)),
        );
        assert!(manager.dialog().is_none());
        assert_eq!(
            manager
                .editor()
                .map(|editor| editor.edited_file().display_name()),
            Some("a".to_owned())
        );
    }

    /// Renders the library at `width` x `height` logical units and `scale`:
    /// the library with data, a filtered library, the focused filter with a
    /// part of its text selected, an empty system, a system with files with
    /// errors and the delete question, in the theme `kind`
    /// with the texts of the catalog of `language`.
    fn screenshots(
        width: f32,
        height: f32,
        scale: f32,
        prefix: &str,
        kind: ThemeKind,
        language: &RepositoryLanguage,
    ) {
        let viewport = Viewport::new((width * scale) as u32, (height * scale) as u32, scale);
        let test = TestRoot::new(&format!("screenshots_{prefix}_{scale}"));
        sample_root(&test);
        let mut manager = test.manager();
        let mut harness = GuiHarness::with_viewport(viewport, kind, language.catalog());
        let render = |harness: &GuiHarness, name: &str| {
            harness.write_screenshot(&format!("manager_{prefix}{name}_{scale}x"));
        };
        settle(&mut harness, &mut manager);
        harness.input.navigation = Some(Navigation::End);
        settle(&mut harness, &mut manager);
        render(&harness, "library");
        if !prefix.is_empty() {
            return;
        }

        harness.input.edit = Some(EditCommand::DeleteForward);
        settle(&mut harness, &mut manager);
        render(&harness, "delete");
        harness.input.cancel = true;
        settle(&mut harness, &mut manager);

        harness.input.shortcut = Some(Shortcut::ControlF);
        settle(&mut harness, &mut manager);
        harness.input.text = "touhou".to_owned();
        settle(&mut harness, &mut manager);
        render(&harness, "filtered");
        for _ in 0..3 {
            harness.input.navigation = Some(Navigation::Left);
            harness.input.modifiers = EditModifiers {
                extend: true,
                ..EditModifiers::default()
            };
            settle(&mut harness, &mut manager);
        }
        render(&harness, "filter_selection");
        harness.input.navigation = Some(Navigation::End);
        settle(&mut harness, &mut manager);
        harness.input.text = "x".to_owned();
        settle(&mut harness, &mut manager);
        render(&harness, "no_match");
        harness.input.cancel = true;
        settle(&mut harness, &mut manager);

        select(&mut manager, "towns");
        harness.state.set_focus(UiId::from(SYSTEM_LIST_ID));
        settle(&mut harness, &mut manager);
        render(&harness, "empty");

        select(&mut manager, "x68k");
        harness.state.set_focus(UiId::from(GAME_TABLE_ID));
        settle(&mut harness, &mut manager);
        harness.input.navigation = Some(Navigation::End);
        settle(&mut harness, &mut manager);
        render(&harness, "errors");

        select(&mut manager, "pc88");
        harness.state.set_focus(UiId::from(SYSTEM_LIST_ID));
        settle(&mut harness, &mut manager);
        render(&harness, "system_list");

        select(&mut manager, "pc98");
        harness.state.set_focus(UiId::from(GAME_TABLE_ID));
        settle(&mut harness, &mut manager);
        let launch = manager.launch(&manager.rows()[0].clone());
        let layer = manager.launch_failed(
            &launch,
            &LaunchFailure::missing_image(
                "/home/user/Games/Neetan/games/98/YU-NO/YU-NO (Disk 3 of 4).hdm",
            ),
        );
        harness.state.open_layer(layer);
        settle(&mut harness, &mut manager);
        render(&harness, "launch_error");
        harness.input.confirm = true;
        settle(&mut harness, &mut manager);

        manager.game_started(launch.clone());
        let layer = manager.game_ended(
            Some("panicked at crates/sample/src/screenshot.rs:1:1:\nsample panic message of the screenshot test"),
            Ok(()),
        );
        harness
            .state
            .open_layer(layer.expect("the panic dialog opens"));
        settle(&mut harness, &mut manager);
        render(&harness, "game_stopped");
        harness.input.confirm = true;
        settle(&mut harness, &mut manager);

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
        settle(&mut harness, &mut manager);
        render(&harness, "media_not_saved");
    }

    /// The title of the game of the steady dialog tests, long enough to wrap in every dialog.
    const LONG_TITLE: &str = "YU-NO: A Girl Who Chants Love at the Bound of this World";

    /// Writes the game file `yu-no.conf` with the title [`LONG_TITLE`]. The colon of the title
    /// is not valid in a Windows file name.
    fn long_title_game(test: &TestRoot) {
        test.game(
            &format!("{}/yu-no.conf", first_system().name),
            &format!("meta.title = {LONG_TITLE}\n"),
        );
    }

    /// Verifies that a steady frame of the delete question allocates nothing, fills no text and
    /// wraps no text, with the focus on Cancel and on Delete.
    #[test]
    fn a_steady_frame_of_the_delete_question_allocates_nothing() {
        let test = TestRoot::new("steady_delete");
        long_title_game(&test);
        let mut manager = test.manager();
        let mut harness = GuiHarness::minimum();
        settle(&mut harness, &mut manager);
        harness.input.edit = Some(EditCommand::DeleteForward);
        settle(&mut harness, &mut manager);
        assert!(delete_open(&manager));
        let cancel = UiId::from(DELETE_LAYER_ID).child("cancel");
        let action = UiId::from(DELETE_LAYER_ID).child("action");
        for focus in [cancel, action] {
            if focus == action {
                harness.input.navigation = Some(Navigation::Right);
                settle(&mut harness, &mut manager);
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
        long_title_game(&test);
        let mut manager = test.manager();
        let mut harness = GuiHarness::minimum();
        settle(&mut harness, &mut manager);
        let launch = manager.launch(&manager.rows()[0].clone());
        let layer = manager.launch_failed(
            &launch,
            &LaunchFailure::missing_image(
                "/home/user/Emulation/PC-98/YU-NO/YU-NO (Disk 1 of 4).hdm",
            ),
        );
        harness.state.open_layer(layer);
        settle(&mut harness, &mut manager);
        let ok = UiId::from(error_dialog::OK_ID);
        let edit = UiId::from(error_dialog::EDIT_ID);
        for focus in [ok, edit] {
            if focus == edit {
                harness.input.navigation = Some(Navigation::Left);
                settle(&mut harness, &mut manager);
            }
            assert_eq!(harness.state.focused(), Some(focus));
            assert_eq!(
                steady_frame(&mut harness, &mut manager),
                PassCounts::NONE,
                "a steady frame of the launch error with the focus on {focus:?}"
            );
        }
        harness.input.navigation = Some(Navigation::Right);
        settle(&mut harness, &mut manager);
        harness.input.confirm = true;
        settle(&mut harness, &mut manager);
        assert!(manager.dialog().is_none());

        manager.game_started(launch);
        let layer = manager
            .game_ended(
                Some("The core halted at 0F000:FFF0 after an unknown opcode in the BIOS of the machine."),
                Ok(()),
            )
            .expect("the panic dialog opens");
        harness.state.open_layer(layer);
        settle(&mut harness, &mut manager);
        assert_eq!(harness.state.focused(), Some(ok));
        assert_eq!(
            steady_frame(&mut harness, &mut manager),
            PassCounts::NONE,
            "a steady frame of the panic error"
        );
    }

    /// Writes the screenshots of the manager in both themes and in every language
    /// of the repository at the scales 1 and 2, at the minimum window size
    /// and at 1920 x 1080.
    #[test]
    #[ignore = "writes screenshots into the directory of GUI_SHOTS"]
    fn manager_screenshots() {
        for language in repository_languages() {
            for kind in [ThemeKind::Dark, ThemeKind::Light] {
                for scale in [1.0, 2.0] {
                    screenshots(MINIMUM_WIDTH, MINIMUM_HEIGHT, scale, "", kind, language);
                    screenshots(1920.0, 1080.0, scale, "large_", kind, language);
                }
            }
        }
    }
}
