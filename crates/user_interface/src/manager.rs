//! The game manager: one list with the games of every system, the launch of
//! the selected game and the error dialogs of a launch.

mod error_dialog;
mod rows;
mod status_line;

use std::path::PathBuf;

use common::{FlushError, info};
use gui::{
    Rect,
    ui::{Align, Button, ButtonKind, Truncation, UiContext, UiId, label, list},
};
use machine_factory::config::systems::System;

pub(crate) use self::{
    error_dialog::ErrorDialog,
    rows::{GameRow, ManagerView},
};
use self::{
    rows::catalog_rows,
    status_line::{Hint, HintKey, STATUS_LINE_HEIGHT, status_line},
};
use crate::{
    Result,
    catalog::Catalog,
    config::{EmulatorConfig, HostLayers, LaunchRequest, ManagerStart, build_configuration},
    gui_host::GuiEvent,
    input::KeyOverrides,
};

/// The height of the header in logical units.
const HEADER_HEIGHT: f32 = 52.0;

/// The width of the launch button in logical units.
const LAUNCH_WIDTH: f32 = 104.0;

/// The width of the system column of the list in logical units.
const SYSTEM_COLUMN_WIDTH: f32 = 96.0;

/// The height of the line below the list in logical units.
const DETAIL_LINE_HEIGHT: f32 = 24.0;

/// The id of the game list.
const LIST_ID: &str = "games";

/// The id of the launch button.
const LAUNCH_ID: &str = "launch";

/// The hint of the launch action.
const LAUNCH_HINT: Hint = Hint {
    keys: &[HintKey::Key("Enter"), HintKey::Pad("A")],
    label: "Launch",
};

/// The hint of the quit action.
const QUIT_HINT: Hint = Hint {
    keys: &[HintKey::Key("Right Ctrl+Q")],
    label: "Quit",
};

/// The hint of the OK button of a dialog.
const OK_HINT: Hint = Hint {
    keys: &[HintKey::Key("Enter"), HintKey::Pad("A")],
    label: "OK",
};

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
    /// The rows of the list.
    rows: Vec<GameRow>,
    /// The selection and the scroll position of the list.
    view: ManagerView,
    /// The open error dialog.
    dialog: Option<ErrorDialog>,
    /// Whether the next pass gives the list the focus.
    focus_list: bool,
    /// The game that runs.
    running: Option<Launch>,
}

impl Manager {
    /// Reads the settings file of `start` and scans the catalog.
    pub(crate) fn new(start: ManagerStart) -> Result<Self> {
        let host = HostLayers::read(&start.request, &start.locations)?;
        let config_folder = host.config_folder();
        let mut manager = Self {
            start,
            host,
            config_folder,
            rows: Vec::new(),
            view: ManagerView::default(),
            dialog: None,
            focus_list: true,
            running: None,
        };
        manager.rescan();
        Ok(manager)
    }

    /// The settings file and the command line flags.
    pub(crate) fn host(&self) -> &HostLayers {
        &self.host
    }

    /// Scans the catalog again and restores the view on the new rows.
    pub(crate) fn rescan(&mut self) {
        let catalog = Catalog::scan(&self.host);
        self.rows = catalog_rows(&catalog, self.host.root());
        self.view.restore(&self.rows);
        info!(
            "Game manager: {} game files in {}",
            self.rows.len(),
            self.config_folder.display()
        );
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

    /// Builds the layered configuration of `launch`.
    pub(crate) fn configuration(&self, launch: &Launch) -> Result<(EmulatorConfig, KeyOverrides)> {
        build_configuration(&launch.request, &self.start.locations)
    }

    /// Records that the game of `launch` runs.
    pub(crate) fn game_started(&mut self, launch: Launch) {
        self.running = Some(launch);
    }

    /// Opens the error dialog of `launch` that did not start with `error`.
    pub(crate) fn launch_failed(&mut self, launch: &Launch, error: &str) {
        self.open_dialog(ErrorDialog::launch_failed(&launch.title, error));
    }

    /// Returns from the running game: scans the catalog again. After a
    /// panic with `panic_message` it opens the panic dialog. After a failed
    /// write back of the disk images in `flush` it opens the dialog of the
    /// images that were not saved.
    pub(crate) fn game_ended(
        &mut self,
        panic_message: Option<&str>,
        flush: std::result::Result<(), FlushError>,
    ) {
        let running = self.running.take();
        self.rescan();
        self.focus_list = true;
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
        if let Some(dialog) = dialog {
            self.open_dialog(dialog);
        }
    }

    /// Opens `dialog` over the list.
    fn open_dialog(&mut self, dialog: ErrorDialog) {
        self.dialog = Some(dialog);
    }

    /// The open error dialog.
    #[cfg(test)]
    pub(crate) fn dialog(&self) -> Option<&ErrorDialog> {
        self.dialog.as_ref()
    }

    /// Returns whether `event` reaches the widgets. An open dialog keeps the
    /// focus on its OK button and takes no navigation.
    pub(crate) fn accepts(&self, event: &GuiEvent) -> bool {
        self.dialog.is_none() || !matches!(event, GuiEvent::Navigation(_))
    }

    /// Draws one pass of the manager. Returns the game to start, if the user
    /// asked for one.
    pub(crate) fn show(&mut self, context: &mut UiContext) -> Option<Launch> {
        let theme = context.theme;
        let bounds = context.bounds();
        context.fill(bounds, theme.panel);
        let (status_area, screen) = bounds.cut_bottom(STATUS_LINE_HEIGHT);
        let dialog_open = self.dialog.is_some();
        if dialog_open {
            context.block_input();
        }
        let mut launch = self.show_library(context, screen);
        if let Some(dialog) = self.dialog.as_mut()
            && context.layer(|context| dialog.show(context))
        {
            self.dialog = None;
            self.focus_list = true;
        }
        if dialog_open {
            launch = None;
        }
        self.show_status_line(context, status_area, dialog_open);
        launch
    }

    /// Draws the header, the launch button, the list and the line below the
    /// list into `screen`. Returns the game to start.
    fn show_library(&mut self, context: &mut UiContext, screen: Rect) -> Option<Launch> {
        let theme = context.theme;
        let area = Rect::new(
            screen.x + theme.padding,
            screen.y,
            (screen.width - theme.padding * 2.0).max(0.0),
            (screen.height - theme.padding).max(0.0),
        );
        let (header, rest) = area.cut_top(HEADER_HEIGHT);
        self.show_header(context, header);

        let (toolbar, rest) = rest.cut_top(theme.control_height);
        let (detail_line, list_area) = rest.cut_bottom(DETAIL_LINE_HEIGHT + theme.gap);
        let list_area = list_area.cut_top(theme.gap * 1.5).1;
        let detail_line = detail_line.cut_top(theme.gap).1;

        let selected_launchable = self
            .view
            .selected_row(&self.rows)
            .is_some_and(GameRow::launchable);
        let launch_pressed = Button::new(LAUNCH_ID, "Launch")
            .kind(ButtonKind::Primary)
            .enabled(selected_launchable)
            .show(context, toolbar.cut_left(LAUNCH_WIDTH).0);

        let list_id = UiId::from(LIST_ID);
        if std::mem::take(&mut self.focus_list) {
            context.set_focus(list_id);
        } else {
            context.default_focus(list_id);
        }
        let rows = &self.rows;
        let response = list(
            context,
            list_id,
            list_area,
            rows.len(),
            &mut self.view.list,
            |context, row| draw_row(context, row.rect, row.selected, &rows[row.index]),
        );
        self.view.record(&self.rows);
        if self.rows.is_empty() {
            self.show_empty_hint(context, list_area);
        }

        if let Some(error) = self
            .view
            .selected_row(&self.rows)
            .and_then(|row| row.error.as_deref())
        {
            let style = theme.small().with_color(theme.danger_text);
            context.text_in(detail_line, &style, error, Align::Start, Truncation::Middle);
        }

        let activated = response.activated.is_some() || launch_pressed;
        let row = self.view.selected_row(&self.rows)?;
        if !activated || !row.launchable() {
            return None;
        }
        Some(Launch {
            request: self.launch_request(row),
            title: row.title.clone(),
            system: row.system,
        })
    }

    /// Draws the name of the application and the game count into `header`.
    fn show_header(&self, context: &mut UiContext, header: Rect) {
        let theme = context.theme;
        let title_style = theme.title();
        let baseline = context.centered_baseline(header, &title_style);
        context.text(header.x, baseline, &title_style, crate::APPLICATION_TITLE);
        let title_width = context.measure(&title_style, crate::APPLICATION_TITLE);
        let games = self.rows.iter().filter(|row| row.launchable()).count();
        let errors = self.rows.len() - games;
        let mut count = match games {
            1 => "1 game".to_owned(),
            count => format!("{count} games"),
        };
        match errors {
            0 => {}
            1 => count.push_str(", 1 file with errors"),
            errors => count.push_str(&format!(", {errors} files with errors")),
        }
        context.text(
            header.x + title_width + theme.gap * 1.5,
            baseline,
            &theme.small(),
            &count,
        );
    }

    /// Draws the hint of an empty catalog into the list area `list_area`.
    fn show_empty_hint(&self, context: &mut UiContext, list_area: Rect) {
        let theme = context.theme;
        let top = list_area.y + theme.gap + theme.row_height;
        let first = Rect::new(list_area.x, top, list_area.width, theme.row_height);
        let folder = self.config_folder.display().to_string();
        label(
            context,
            first,
            &theme.dim(),
            &format!("No games in {folder}"),
            Align::Center,
            Truncation::Middle,
        );
        let second = Rect::new(
            list_area.x,
            top + theme.row_height,
            list_area.width,
            theme.row_height,
        );
        label(
            context,
            second,
            &theme.small(),
            "Add one game file for each game in the folder of its system, for example pc98/edge.conf.",
            Align::Center,
            Truncation::End,
        );
    }

    /// Draws the status line into `area`.
    fn show_status_line(&self, context: &mut UiContext, area: Rect, dialog_open: bool) {
        let theme = context.theme;
        let selected = self.view.selected_row(&self.rows);
        let hints: &[Hint] = if dialog_open {
            &[OK_HINT]
        } else if selected.is_some_and(GameRow::launchable) {
            &[LAUNCH_HINT, QUIT_HINT]
        } else {
            &[QUIT_HINT]
        };
        let end = selected.map_or("", |row| row.shown_path.as_str());
        status_line(context, area, hints, end, theme.text_dim);
    }
}

/// Draws the system label and the title of `game` into the row area `rect`.
/// A file with an error draws dimmed. The system label of the `selected` row
/// draws in the text color.
fn draw_row(context: &mut UiContext, rect: Rect, selected: bool, game: &GameRow) {
    let theme = context.theme;
    let inner = Rect::new(
        rect.x + theme.gap,
        rect.y,
        (rect.width - theme.gap * 2.0).max(0.0),
        rect.height,
    );
    let (system_area, title_area) = inner.cut_left(SYSTEM_COLUMN_WIDTH);
    let (system_color, title_color) = if !game.launchable() && selected {
        (theme.text_dim, theme.text_dim)
    } else if !game.launchable() {
        (theme.text_disabled, theme.text_disabled)
    } else if selected {
        (theme.text, theme.text)
    } else {
        (theme.text_dim, theme.text)
    };
    context.text_in(
        system_area,
        &theme.body().with_color(system_color),
        game.system.label,
        Align::Start,
        Truncation::End,
    );
    context.text_in(
        title_area.cut_left(theme.gap).1,
        &theme.body().with_color(title_color),
        &game.title,
        Align::Start,
        Truncation::End,
    );
}

#[cfg(test)]
mod tests {
    use gui::ui::{Navigation, Viewport};
    use machine_factory::config::systems::compiled_systems;

    use super::*;
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
        let row = manager.rows[0].clone();
        assert_eq!(row.title, "Edge");
        let request = manager.launch_request(&row);
        assert_eq!(
            request.game_files,
            vec![test.root.join("config").join(&identity)]
        );
        assert_eq!(request.system, Some(system));
        assert_eq!(request.flags.len(), 1);
        assert_eq!(request.settings, None);
        let launch = Launch {
            request,
            title: row.title.clone(),
            system,
        };
        let (config, _) = manager
            .configuration(&launch)
            .expect("configuration builds");
        assert!(!config.crt);
        assert_eq!(system_of_config(&config), system);
    }

    /// The system of the family of `config`.
    fn system_of_config(config: &EmulatorConfig) -> &'static System {
        machine_factory::config::systems::system_of(config.target)
    }

    /// Verifies that the selected game returns after a rescan and that a removed game selects the nearest row.
    #[test]
    fn the_view_returns_after_a_rescan() {
        let test = TestRoot::new("view_restore");
        let system = first_system();
        for name in ["a", "b", "c"] {
            test.game(
                &format!("{}/{name}.conf", system.name),
                &format!("meta.title = {name}\n"),
            );
        }
        let mut manager = test.manager();
        let mut harness = GuiHarness::new(960, 600);
        harness.frame(|context| manager.show(context));
        harness.input.navigation = Some(Navigation::End);
        harness.frame(|context| manager.show(context));
        let selected = format!("{}/c.conf", system.name);
        assert_eq!(manager.view.selected.as_deref(), Some(selected.as_str()));

        manager.game_ended(None, Ok(()));
        assert_eq!(manager.view.selected.as_deref(), Some(selected.as_str()));
        assert_eq!(manager.view.list.selected, 2);

        test.remove(&selected);
        manager.game_ended(None, Ok(()));
        assert_eq!(manager.view.list.selected, 1);
        assert!(manager.dialog().is_none());
    }

    /// Verifies that confirm launches the selected game and ignores a file with an error.
    #[test]
    fn confirm_launches_and_an_error_file_does_not() {
        let test = TestRoot::new("confirm");
        let system = first_system();
        test.game(&format!("{}/a.conf", system.name), "meta.title = A\n");
        test.game(&format!("{}/b.conf", system.name), "machine = FOOBAR\n");
        let mut manager = test.manager();
        assert!(manager.rows[1].error.is_some());
        let mut harness = GuiHarness::new(960, 600);
        harness.frame(|context| manager.show(context));
        harness.input.confirm = true;
        let launch = harness
            .frame(|context| manager.show(context))
            .expect("confirm launches");
        assert_eq!(launch.title, "A");
        harness.input.navigation = Some(Navigation::Down);
        harness.frame(|context| manager.show(context));
        harness.input.confirm = true;
        assert!(harness.frame(|context| manager.show(context)).is_none());
    }

    /// Verifies that the error dialog blocks the navigation, closes on confirm and returns the focus to the list.
    #[test]
    fn the_error_dialog_takes_the_input_until_it_closes() {
        let test = TestRoot::new("dialog");
        let system = first_system();
        test.game(&format!("{}/a.conf", system.name), "meta.title = A\n");
        let mut manager = test.manager();
        let launch = Launch {
            request: manager.launch_request(&manager.rows[0].clone()),
            title: "A".to_owned(),
            system,
        };
        manager.launch_failed(&launch, "fdd1 image not found: /a.d88");
        assert_eq!(
            manager.dialog().map(ErrorDialog::message),
            Some("A did not start.")
        );
        assert!(!manager.accepts(&GuiEvent::Navigation(Navigation::Down)));
        assert!(manager.accepts(&GuiEvent::Confirm));
        let mut harness = GuiHarness::new(960, 600);
        assert!(harness.frame(|context| manager.show(context)).is_none());
        harness.input.confirm = true;
        assert!(harness.frame(|context| manager.show(context)).is_none());
        assert!(manager.dialog().is_none());
        harness.frame(|context| manager.show(context));
        assert_eq!(harness.state.focused(), Some(UiId::from(LIST_ID)));

        manager.game_started(launch);
        manager.game_ended(Some("core halted"), Ok(()));
        let dialog = manager.dialog().expect("panic dialog opens");
        assert_eq!(dialog.detail(), "core halted");
        assert_eq!(
            dialog.message(),
            format!(
                "The {} core stopped with an error. The disk images were saved.",
                system.label
            )
        );
    }

    /// Verifies that a failed write back after a normal end of the game opens the dialog of the images.
    #[test]
    fn a_failed_flush_after_the_game_opens_the_media_dialog() {
        let test = TestRoot::new("flush");
        let system = first_system();
        test.game(&format!("{}/a.conf", system.name), "meta.title = A\n");
        let mut manager = test.manager();
        let launch = Launch {
            request: manager.launch_request(&manager.rows[0].clone()),
            title: "A".to_owned(),
            system,
        };
        manager.game_started(launch);
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

    /// The sample games of the screenshots: the system, the file stem and the title.
    const SAMPLE_GAMES: [(&str, &str, &str); 12] = [
        ("pc98", "brandish", "Brandish"),
        ("pc98", "edge", "Edge"),
        ("pc98", "policenauts", "Policenauts"),
        (
            "pc98",
            "touhou-reiiden",
            "Touhou Reiiden: Highly Responsive to Prayers",
        ),
        (
            "pc98",
            "yu-no",
            "YU-NO: A Girl Who Chants Love at the Bound of this World",
        ),
        ("pc98", "dokyusei", "同級生"),
        ("pc88", "thexder", "Thexder"),
        (
            "pc88",
            "ys-2",
            "Ys II: Ancient Ys Vanished The Final Chapter",
        ),
        ("x68k", "gradius-2", "Gradius II"),
        ("x68k", "akumajou-dracula", "悪魔城ドラキュラ"),
        ("towns", "after-burner-3", "After Burner III"),
        ("msx", "metal-gear", "Metal Gear"),
    ];

    /// Renders the manager at `scale` with the sample games, with a file with
    /// an error selected, with each error dialog and without games.
    fn screenshots(scale: f32) {
        let width = (960.0 * scale) as u32;
        let height = (600.0 * scale) as u32;
        let test = TestRoot::new(&format!("screenshots_{scale}"));
        for (system, stem, title) in SAMPLE_GAMES {
            test.game(
                &format!("{system}/{stem}.conf"),
                &format!("meta.title = {title}\n"),
            );
        }
        test.game("x68k/nemesis-90.conf", "machine = X68030\n");
        let mut manager = test.manager();
        let mut harness = GuiHarness::with_viewport(Viewport::new(width, height, scale));
        let render = |harness: &GuiHarness, name: &str| {
            harness.write_screenshot(&format!("manager_{name}_{scale}x"));
        };
        harness.frame(|context| manager.show(context));
        harness.input.navigation = Some(Navigation::Down);
        harness.frame(|context| manager.show(context));
        render(&harness, "library");

        let error_row = manager
            .rows
            .iter()
            .position(|row| !row.launchable())
            .expect("one file has an error");
        manager.view.list.select(error_row);
        harness.frame(|context| manager.show(context));
        render(&harness, "error_file");

        harness.input.navigation = Some(Navigation::Home);
        harness.frame(|context| manager.show(context));
        let launch = Launch {
            request: manager.launch_request(&manager.rows[4].clone()),
            title: manager.rows[4].title.clone(),
            system: manager.rows[4].system,
        };
        manager.launch_failed(
            &launch,
            "fdd1 image not found: /home/user/Games/Neetan/games/98/YU-NO/YU-NO (Disk 3 of 4).hdm",
        );
        harness.frame(|context| manager.show(context));
        render(&harness, "launch_error");
        harness.input.confirm = true;
        harness.frame(|context| manager.show(context));

        manager.game_started(launch);
        manager.game_ended(
            Some("panicked at crates/sample/src/screenshot.rs:1:1:\nsample panic message of the screenshot test"),
            Ok(()),
        );
        harness.frame(|context| manager.show(context));
        render(&harness, "game_stopped");
        harness.input.confirm = true;
        harness.frame(|context| manager.show(context));

        let launch = Launch {
            request: manager.launch_request(&manager.rows[3].clone()),
            title: manager.rows[3].title.clone(),
            system: manager.rows[3].system,
        };
        manager.game_started(launch);
        manager.game_ended(
            None,
            Err(FlushError::new(
                "hard disk image /home/user/Games/Neetan/games/98/Touhou/reiiden.hdi could not be written: Permission denied (os error 13)",
            )),
        );
        harness.frame(|context| manager.show(context));
        render(&harness, "media_not_saved");

        let empty = TestRoot::new(&format!("screenshots_empty_{scale}"));
        let mut manager = empty.manager();
        let mut harness = GuiHarness::with_viewport(Viewport::new(width, height, scale));
        harness.frame(|context| manager.show(context));
        render(&harness, "empty");
    }

    /// Writes the screenshots of the manager at the scales 1 and 2.
    #[test]
    #[ignore = "writes screenshots into the directory of GUI_SHOTS"]
    fn manager_screenshots() {
        screenshots(1.0);
        screenshots(2.0);
    }
}
