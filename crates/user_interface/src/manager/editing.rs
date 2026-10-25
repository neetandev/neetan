//! The editor in the manager: the add, the edit and the duplicate of a game,
//! the edit of the system defaults of a system, the editor page with its
//! status line, and the update of the library after a save.

use std::path::Path;

use common::{info, warn};
use gui::ui::{STATUS_LINE_HEIGHT, StatusEnd, UiContext, UiId, status_line};
use machine_factory::config::systems::System;

use super::{
    DialogAnswer, EditTarget, ErrorDialog, GameRow, Manager, NextFocus, SETTINGS_ID, rows_at,
    sidebar::SYSTEM_DEFAULTS_ID,
};
use crate::{
    catalog::{read_game, read_games},
    editor::{EditedFile, Editor, SaveFailure},
};

/// The error dialog of the file of `subject` after the save failure
/// `failure`, which the log records.
fn save_failure_dialog(subject: &EditedFile, failure: &SaveFailure) -> ErrorDialog {
    warn!(
        "Game manager: {} {} not saved: {}",
        subject.sentence_start(),
        subject.was(),
        failure.message()
    );
    match failure {
        SaveFailure::Error(error) => ErrorDialog::save_failed(subject, error),
        SaveFailure::ChangedOutside(detail) => ErrorDialog::changed_outside(subject, detail),
    }
}

impl Manager {
    /// Opens `editor`, or the error dialog of the file of `subject` with
    /// the error of the editor.
    pub(super) fn open_editor(
        &mut self,
        context: &mut UiContext,
        editor: Result<Editor, String>,
        subject: EditedFile,
    ) {
        match editor {
            Ok(editor) => {
                info!("Game manager: editing {}", editor.path().display());
                self.editor = Some(editor);
            }
            Err(error) => self.dialog = Some(ErrorDialog::open_failed(&subject, &error)),
        }
        context.request_pass();
    }

    /// Opens the editor on a new game of the selected system.
    pub(super) fn open_new_game(&mut self, context: &mut UiContext) {
        let Some(system) = self.current().map(|entry| entry.system) else {
            return;
        };
        let editor = Editor::new_game(&self.host, system)
            .ok_or_else(|| format!("the system {} is not compiled", system.name));
        self.open_editor(
            context,
            editor,
            EditedFile::Game {
                system,
                name: format!("A new {} game", system.label),
            },
        );
    }

    /// Opens the editor on the game file of `row`.
    pub(super) fn open_game(&mut self, context: &mut UiContext, row: &GameRow) {
        let editor = Editor::open(&self.host, row.system, &row.path);
        let file = EditedFile::Game {
            system: row.system,
            name: row.name().to_owned(),
        };
        self.open_editor(context, editor, file);
    }

    /// Opens the editor on a copy of the game file of `row`.
    pub(super) fn open_duplicate(&mut self, context: &mut UiContext, row: &GameRow) {
        let editor = Editor::duplicate(&self.host, row.system, &row.path);
        let file = EditedFile::Game {
            system: row.system,
            name: row.name().to_owned(),
        };
        self.open_editor(context, editor, file);
    }

    /// Opens the editor on the system defaults file of the selected system.
    pub(super) fn open_system_defaults(&mut self, context: &mut UiContext) {
        let Some(system) = self.current().map(|entry| entry.system) else {
            return;
        };
        let editor = Editor::system_defaults(&self.host, system);
        self.open_editor(context, editor, EditedFile::SystemDefaults(system));
    }

    /// Opens the editor on the game file of `target`, the game of an error dialog.
    pub(super) fn open_target(&mut self, context: &mut UiContext, target: &EditTarget) {
        let editor = Editor::open(&self.host, target.system, &target.path);
        let name = target
            .path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let file = EditedFile::Game {
            system: target.system,
            name,
        };
        self.open_editor(context, editor, file);
    }

    /// Reads the saved game file `path` of `system` again and updates its
    /// row in the library. The saved game becomes the selected game.
    pub(super) fn game_saved(&mut self, system: &'static System, path: &Path) {
        info!("Game manager: saved {}", path.display());
        let Some(entry) = read_game(&self.host, system, path) else {
            return;
        };
        let root = self.host.root().to_path_buf();
        let row = GameRow::new(system, &entry, &root);
        let identity = row.identity.clone();
        let Some(index) = self
            .systems
            .iter()
            .position(|entry| entry.system.name == system.name)
        else {
            return;
        };
        let rows = &mut self.systems[index].rows;
        match rows
            .iter()
            .position(|existing| existing.identity == identity)
        {
            Some(place) => rows[place] = row,
            None => rows.push(row),
        }
        if index == self.view.systems.selected {
            self.view.selected = Some(identity);
            self.refresh_entries();
            let rows = rows_at(&self.systems, index);
            self.view.restore(&self.entries, rows);
        }
    }

    /// Reads the game files of the library rows of `system` again after a
    /// save of its system defaults file `path`. The rows take the model
    /// and the findings that the new system defaults give them. The update
    /// reads only the files of the rows. It lists no folder and checks no
    /// ROM.
    pub(super) fn system_defaults_saved(&mut self, system: &'static System, path: &Path) {
        info!("Game manager: saved {}", path.display());
        let Some(index) = self
            .systems
            .iter()
            .position(|entry| entry.system.name == system.name)
        else {
            return;
        };
        let paths: Vec<_> = self.systems[index]
            .rows
            .iter()
            .map(|row| row.path.clone())
            .collect();
        let root = self.host.root().to_path_buf();
        self.systems[index].rows = read_games(&self.host, system, &paths)
            .iter()
            .map(|entry| GameRow::new(system, entry, &root))
            .collect();
        if index == self.view.systems.selected {
            self.refresh_entries();
            let rows = rows_at(&self.systems, index);
            self.view.restore(&self.entries, rows);
        }
    }

    /// Updates the library after a save of the file `path` of the editor
    /// on `edited`: the row of a game file, every row of the system after a
    /// save of its system defaults, or the host settings after a save of
    /// the settings file.
    fn file_saved(&mut self, edited: &EditedFile, path: &Path) {
        match *edited {
            EditedFile::Game { system, .. } => self.game_saved(system, path),
            EditedFile::SystemDefaults(system) => self.system_defaults_saved(system, path),
            EditedFile::Settings => self.settings_saved(path),
        }
    }

    /// Draws one pass of the open editor over the whole window with its
    /// status line, and the error dialog of a failed save over it. The
    /// Overwrite of the dialog of a game file that changed outside the
    /// manager saves over the file. A closed editor gives the library back
    /// with the focus on its game table, on the system defaults button after
    /// the system defaults, or on the settings button after the settings
    /// page.
    pub(super) fn show_editor(&mut self, context: &mut UiContext) {
        let (status_area, screen) = context.bounds().cut_bottom(STATUS_LINE_HEIGHT);
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        let pass = editor.show(context, screen);
        let edited = editor.edited_file();
        if let Some(path) = &pass.saved {
            self.file_saved(&edited, path);
        }
        if let Some(failure) = &pass.save_error {
            self.dialog = Some(save_failure_dialog(&edited, failure));
        }
        let answer = self
            .dialog
            .as_ref()
            .map_or(DialogAnswer::Open, |dialog| dialog.show(context, screen));
        if answer != DialogAnswer::Open {
            self.dialog = None;
        }
        if answer == DialogAnswer::Overwrite
            && let Some(editor) = self.editor.as_mut()
        {
            match editor.overwrite() {
                Ok(path) => self.file_saved(&edited, &path),
                Err(failure) => self.dialog = Some(save_failure_dialog(&edited, &failure)),
            }
            context.request_pass();
        }
        let Some(editor) = self.editor.as_ref() else {
            return;
        };
        let hints = match &self.dialog {
            Some(dialog) => dialog.hints(context),
            None => editor.hints().to_vec(),
        };
        let shown_path = editor.shown_path();
        status_line(context, status_area, &hints, StatusEnd::Path(&shown_path));
        if pass.closed {
            info!(
                "Game manager: closed the editor of {}",
                editor.path().display()
            );
            self.editor = None;
            self.dialog = None;
            self.next_focus = Some(match edited {
                EditedFile::Game { .. } => NextFocus::Start,
                EditedFile::SystemDefaults(_) => NextFocus::Widget(UiId::from(SYSTEM_DEFAULTS_ID)),
                EditedFile::Settings => NextFocus::Widget(UiId::from(SETTINGS_ID)),
            });
            context.request_pass();
        }
    }

    /// The open editor.
    #[cfg(test)]
    pub(crate) fn editor(&self) -> Option<&Editor> {
        self.editor.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use gui::ui::{ActionButton, Navigation, Shortcut, UiId, Viewport};
    use machine_factory::config::systems::compiled_systems;

    use super::*;
    use crate::{
        config::{LaunchRequest, ManagerStart},
        editor::test_support::EditorRoot,
        gui_test_support::GuiHarness,
        manager::{FILTER_ID, entries::TableEntry, game_table::GAME_TABLE_ID},
    };

    /// The manager of `root`.
    fn manager(root: &EditorRoot) -> Manager {
        Manager::new(ManagerStart {
            request: LaunchRequest::default(),
            locations: root.locations(),
        })
        .expect("manager starts")
    }

    /// Selects the system with the folder name `name`.
    fn select(manager: &mut Manager, name: &str) {
        let index = manager
            .systems
            .iter()
            .position(|entry| entry.system.name == name)
            .expect("the system is compiled");
        manager.select_system(index);
    }

    /// The titles of the games of the selected system in the order of the table.
    fn titles(manager: &Manager) -> Vec<String> {
        let rows = manager.rows();
        manager
            .entries
            .iter()
            .filter_map(|entry| match entry {
                TableEntry::Game(index) => Some(rows[*index].title.clone()),
                TableEntry::Broken(_) | TableEntry::Caption => None,
            })
            .collect()
    }

    /// Draws passes of `manager` until it settles.
    fn settle(harness: &mut GuiHarness, manager: &mut Manager) {
        harness.settle(|context| manager.show(context));
    }

    /// A root with two games of the first system.
    fn two_games(name: &str) -> EditorRoot {
        with_two_games(EditorRoot::new(name))
    }

    /// Writes the games Edge and Brandish of [`two_games`] into `root`.
    fn with_two_games(root: EditorRoot) -> EditorRoot {
        let model = root.system().default_model;
        root.game("edge", &format!("meta.title = Edge\nmachine = {model}\n"));
        root.game(
            "brandish",
            "; Falcom\nmeta.title = Brandish\nmeta.year = 1991\n",
        );
        root
    }

    /// Verifies that F2 and X open the editor on the selected game, that Esc closes it without changes and that the table takes the focus back.
    #[test]
    fn f2_and_x_open_the_editor_on_the_selected_game() {
        let root = two_games("edit_keys");
        let mut manager = manager(&root);
        let mut harness = GuiHarness::new(960, 600);
        settle(&mut harness, &mut manager);
        assert_eq!(titles(&manager), ["Brandish", "Edge"]);
        for open in [
            |harness: &mut GuiHarness| harness.input.shortcut = Some(Shortcut::F2),
            |harness: &mut GuiHarness| harness.input.action = Some(ActionButton::X),
        ] {
            open(&mut harness);
            settle(&mut harness, &mut manager);
            let editor = manager.editor().expect("the editor opens");
            assert_eq!(
                editor.path(),
                root.path(&format!("config/{}/brandish.conf", root.system().name))
            );
            assert_eq!(
                harness.state.focused(),
                Some(crate::editor::field_id("meta.title"))
            );
            harness.input.cancel = true;
            settle(&mut harness, &mut manager);
            assert!(manager.editor().is_none());
            assert_eq!(harness.state.focused(), Some(UiId::from(GAME_TABLE_ID)));
        }
    }

    /// Verifies that F2, Insert and Delete do nothing while the filter holds the focus, and that Ctrl+D works there.
    #[test]
    fn shortcuts_without_control_are_inactive_in_the_filter() {
        let root = two_games("edit_filter");
        let mut manager = manager(&root);
        let mut harness = GuiHarness::new(960, 600);
        settle(&mut harness, &mut manager);
        harness.state.set_focus(UiId::from(FILTER_ID));
        settle(&mut harness, &mut manager);
        for shortcut in [Shortcut::F2, Shortcut::Insert, Shortcut::Delete] {
            harness.input.shortcut = Some(shortcut);
            settle(&mut harness, &mut manager);
            assert!(manager.editor().is_none(), "{shortcut:?}");
            assert!(manager.delete.is_none(), "{shortcut:?}");
        }
        harness.input.shortcut = Some(Shortcut::ControlD);
        settle(&mut harness, &mut manager);
        let editor = manager.editor().expect("Ctrl+D duplicates");
        assert_eq!(editor.edited_file().display_name(), "Brandish (copy)");
    }

    /// Verifies that a save in the editor updates the row of the game and that Ctrl+S saves from a text field.
    #[test]
    fn a_save_updates_the_library_row() {
        let root = two_games("edit_save");
        let mut manager = manager(&root);
        let mut harness = GuiHarness::new(960, 600);
        settle(&mut harness, &mut manager);
        harness.input.shortcut = Some(Shortcut::F2);
        settle(&mut harness, &mut manager);
        harness.input.text = " II".to_owned();
        settle(&mut harness, &mut manager);
        harness.input.shortcut = Some(Shortcut::ControlS);
        settle(&mut harness, &mut manager);
        let path = root.path(&format!("config/{}/brandish.conf", root.system().name));
        let model = root.system().default_model;
        assert_eq!(
            std::fs::read_to_string(&path).expect("file reads"),
            format!("; Falcom\nmeta.title = Brandish II\nmeta.year = 1991\nmachine = {model}\n")
        );
        assert!(manager.editor().is_some());
        harness.input.cancel = true;
        settle(&mut harness, &mut manager);
        assert!(manager.editor().is_none());
        assert_eq!(titles(&manager), ["Brandish II", "Edge"]);
        let identity = format!("{}/brandish.conf", root.system().name);
        assert_eq!(manager.view.selected.as_deref(), Some(identity.as_str()));
    }

    /// Verifies that Insert adds a game whose file derives from the title, that the save writes the default model and that Start saves.
    #[test]
    fn insert_adds_a_game_with_a_derived_file() {
        let root = two_games("edit_add");
        let mut manager = manager(&root);
        let mut harness = GuiHarness::new(960, 600);
        settle(&mut harness, &mut manager);
        harness.input.shortcut = Some(Shortcut::Insert);
        settle(&mut harness, &mut manager);
        let folder = format!("config/{}", root.system().name);
        assert_eq!(
            manager.editor().expect("the editor opens").path(),
            root.path(&format!("{folder}/game.conf"))
        );
        harness.input.text = "Edge".to_owned();
        settle(&mut harness, &mut manager);
        assert_eq!(
            manager.editor().expect("open").path(),
            root.path(&format!("{folder}/edge-2.conf"))
        );
        harness.input.action = Some(ActionButton::Start);
        settle(&mut harness, &mut manager);
        let model = root.system().default_model;
        assert_eq!(
            std::fs::read_to_string(root.path(&format!("{folder}/edge-2.conf")))
                .expect("the new file reads"),
            format!("meta.title = Edge\nmachine = {model}\n")
        );
        harness.input.text = " Again".to_owned();
        settle(&mut harness, &mut manager);
        assert_eq!(
            manager.editor().expect("open").path(),
            root.path(&format!("{folder}/edge-2.conf"))
        );
        harness.input.cancel = true;
        settle(&mut harness, &mut manager);
        assert!(manager.editor().is_some(), "the question asks first");
        harness.input.navigation = Some(Navigation::Right);
        settle(&mut harness, &mut manager);
        harness.input.confirm = true;
        settle(&mut harness, &mut manager);
        assert!(manager.editor().is_none());
        assert_eq!(titles(&manager), ["Brandish", "Edge", "Edge"]);
    }

    /// Verifies that Duplicate copies the selected file with the title suffix and the suffix of an existing file name.
    #[test]
    fn duplicate_copies_the_file_with_a_new_title() {
        let root = two_games("edit_duplicate");
        root.game("brandish-copy", "meta.title = Older copy\n");
        let mut manager = manager(&root);
        let mut harness = GuiHarness::new(960, 600);
        settle(&mut harness, &mut manager);
        harness.input.shortcut = Some(Shortcut::ControlD);
        settle(&mut harness, &mut manager);
        let folder = format!("config/{}", root.system().name);
        assert_eq!(
            manager.editor().expect("open").path(),
            root.path(&format!("{folder}/brandish-copy-2.conf"))
        );
        harness.input.shortcut = Some(Shortcut::ControlS);
        settle(&mut harness, &mut manager);
        let model = root.system().default_model;
        assert_eq!(
            std::fs::read_to_string(root.path(&format!("{folder}/brandish-copy-2.conf")))
                .expect("the copy reads"),
            format!(
                "; Falcom\nmeta.title = Brandish (copy)\nmeta.year = 1991\nmachine = {model}\n"
            )
        );
    }

    /// Verifies that a save over a file that changed outside opens the dialog with Cancel and Overwrite, that Cancel keeps the editor and its changes, that an Overwrite that fails opens the error dialog, and that an Overwrite saves over the change.
    #[test]
    fn a_file_changed_outside_asks_before_an_overwrite() {
        let root = two_games("edit_save_error");
        let mut manager = manager(&root);
        let mut harness = GuiHarness::new(960, 600);
        settle(&mut harness, &mut manager);
        harness.input.shortcut = Some(Shortcut::F2);
        settle(&mut harness, &mut manager);
        let path = root.path(&format!("config/{}/brandish.conf", root.system().name));
        std::fs::write(&path, "meta.title = Brandish\ncrt = on\n").expect("file is written");
        harness.input.text = "!".to_owned();
        settle(&mut harness, &mut manager);
        harness.input.shortcut = Some(Shortcut::ControlS);
        settle(&mut harness, &mut manager);
        let dialog = manager.dialog().expect("the dialog opens");
        assert_eq!(
            dialog.message(),
            "Brandish! was not saved. Overwrite replaces the file with the changes of the \
             editor. Cancel returns to the editor with the changes."
        );
        assert_eq!(
            harness.state.focused(),
            Some(UiId::from("error cancel")),
            "Cancel holds the focus"
        );
        harness.input.confirm = true;
        settle(&mut harness, &mut manager);
        assert!(manager.dialog().is_none());
        let editor = manager.editor().expect("the editor stays");
        assert_eq!(editor.edited_file().display_name(), "Brandish!");
        assert_eq!(
            std::fs::read_to_string(&path).expect("file reads"),
            "meta.title = Brandish\ncrt = on\n"
        );

        std::fs::remove_file(&path).expect("file is removed");
        std::fs::create_dir_all(&path).expect("a folder takes the place of the file");
        harness.input.shortcut = Some(Shortcut::ControlS);
        settle(&mut harness, &mut manager);
        assert!(manager.dialog().is_some());
        harness.input.navigation = Some(Navigation::Right);
        settle(&mut harness, &mut manager);
        assert_eq!(
            harness.state.focused(),
            Some(UiId::from(super::super::error_dialog::OVERWRITE_ID))
        );
        harness.input.confirm = true;
        settle(&mut harness, &mut manager);
        let dialog = manager.dialog().expect("the error dialog opens");
        assert_eq!(
            dialog.message(),
            "Brandish! was not saved. The editor keeps the changes."
        );
        harness.input.confirm = true;
        settle(&mut harness, &mut manager);
        assert!(manager.dialog().is_none());
        assert!(manager.editor().is_some());

        std::fs::remove_dir(&path).expect("the folder is removed");
        std::fs::write(&path, "meta.title = Outside\n").expect("file is written");
        harness.input.shortcut = Some(Shortcut::ControlS);
        settle(&mut harness, &mut manager);
        harness.input.navigation = Some(Navigation::Right);
        settle(&mut harness, &mut manager);
        harness.input.confirm = true;
        settle(&mut harness, &mut manager);
        assert!(manager.dialog().is_none());
        let model = root.system().default_model;
        assert_eq!(
            std::fs::read_to_string(&path).expect("file reads"),
            format!("; Falcom\nmeta.title = Brandish!\nmeta.year = 1991\nmachine = {model}\n")
        );
        assert_eq!(titles(&manager), ["Brandish!", "Edge"]);
    }

    /// Verifies that the Edit game button of a launch error opens the editor on the game.
    #[test]
    fn a_launch_error_opens_its_game_in_the_editor() {
        let root = two_games("edit_launch_error");
        let mut manager = manager(&root);
        let mut harness = GuiHarness::new(960, 600);
        settle(&mut harness, &mut manager);
        let launch = manager.launch(&manager.rows()[1].clone());
        let layer = manager.launch_failed(&launch, "fdd1 image not found: /a.d88");
        harness.state.open_layer(layer);
        settle(&mut harness, &mut manager);
        harness.input.navigation = Some(Navigation::Left);
        settle(&mut harness, &mut manager);
        harness.input.confirm = true;
        settle(&mut harness, &mut manager);
        assert!(manager.dialog().is_none());
        let editor = manager.editor().expect("the editor opens");
        assert_eq!(editor.edited_file().display_name(), "Edge");
    }

    /// Verifies that the System defaults button opens the editor on the system defaults file of the selected system, that Esc closes it without changes and that the button takes the focus back.
    #[test]
    fn the_system_defaults_button_opens_the_defaults_editor() {
        let root = two_games("edit_defaults_open");
        let mut manager = manager(&root);
        let mut harness = GuiHarness::new(960, 600);
        settle(&mut harness, &mut manager);
        harness.state.set_focus(UiId::from(SYSTEM_DEFAULTS_ID));
        settle(&mut harness, &mut manager);
        harness.input.confirm = true;
        settle(&mut harness, &mut manager);
        let editor = manager.editor().expect("the editor opens");
        assert!(matches!(
            editor.edited_file(),
            EditedFile::SystemDefaults(_)
        ));
        assert_eq!(
            editor.path(),
            root.path(&format!("config/{}.conf", root.system().name))
        );
        assert_eq!(
            harness.state.focused(),
            Some(crate::editor::field_id(
                machine_factory::config::settings::MACHINE_KEY
            ))
        );
        harness.input.cancel = true;
        settle(&mut harness, &mut manager);
        assert!(manager.editor().is_none());
        assert_eq!(
            harness.state.focused(),
            Some(UiId::from(SYSTEM_DEFAULTS_ID))
        );
        assert!(
            !root
                .path(&format!("config/{}.conf", root.system().name))
                .exists()
        );
    }

    #[cfg(feature = "pc98")]
    /// Verifies that a save of the system defaults gives the rows of the system the new launch model without a scan of the system folder.
    #[test]
    fn a_defaults_save_updates_the_launch_model_of_the_rows() {
        let root = with_two_games(EditorRoot::with_system("edit_defaults_save", "pc98"));
        let system = root.system();
        let machine = machine_factory::config::settings::find_setting(
            machine_factory::config::settings::MACHINE_KEY,
        )
        .expect("machine is a table key");
        let choices: Vec<&'static str> = machine
            .choices_for(root.target())
            .map(|choice| choice.value)
            .collect();
        let other = *choices.last().expect("the PC-98 has models");
        assert_ne!(other, system.default_model);
        let mut manager = manager(&root);
        let mut harness = GuiHarness::new(960, 600);
        settle(&mut harness, &mut manager);
        select(&mut manager, "pc98");
        settle(&mut harness, &mut manager);
        let model_of = |manager: &Manager, title: &str| {
            manager
                .rows()
                .iter()
                .find(|row| row.title == title)
                .map(|row| row.model)
                .expect("the game is listed")
        };
        assert_eq!(model_of(&manager, "Brandish"), system.default_model);
        root.game("later", "meta.title = Later\n");

        harness.state.set_focus(UiId::from(SYSTEM_DEFAULTS_ID));
        settle(&mut harness, &mut manager);
        harness.input.confirm = true;
        settle(&mut harness, &mut manager);
        assert!(
            manager.editor().is_some_and(|editor| matches!(
                editor.edited_file(),
                EditedFile::SystemDefaults(_)
            ))
        );
        harness.input.confirm = true;
        settle(&mut harness, &mut manager);
        harness.input.navigation = Some(Navigation::End);
        settle(&mut harness, &mut manager);
        harness.input.confirm = true;
        settle(&mut harness, &mut manager);
        harness.input.shortcut = Some(Shortcut::ControlS);
        settle(&mut harness, &mut manager);
        assert_eq!(
            std::fs::read_to_string(root.path(&format!("config/{}.conf", system.name)))
                .expect("the system defaults read"),
            format!("machine = {other}\n")
        );
        assert_eq!(model_of(&manager, "Brandish"), other);
        assert_eq!(model_of(&manager, "Edge"), system.default_model);
        assert_eq!(titles(&manager), ["Brandish", "Edge"]);
    }

    /// The game file of YU-NO in the screenshots.
    const YU_NO: &str = "meta.title = YU-NO: A Girl Who Chants Love at the Bound of this World
meta.title-original = この世の果てで恋を唄う少女YU-NO
meta.developer = Elf
meta.publisher = Elf
meta.year = 1996
meta.notes = Install to HDD 1 first. Needs the 86 board for the music of the opening.
machine = PC9801RA
boot-device = hdd1
fdd1 = games/98/YU-NO/YU-NO (Disk 1 of 4).hdm
fdd1 = games/98/YU-NO/YU-NO (Disk 2 of 4).hdm
fdd1 = games/98/YU-NO/YU-NO (Disk 3 of 4).hdm
fdd1 = games/98/YU-NO/YU-NO (Disk 4 of 4).hdm
hdd1 = games/98/YU-NO/yu-no.hdi
soundboard = 86
window-mode = fullscreen
key.RAlt = Grph
key.PageUp = RollDown
key.PageDown = RollUp
; Patched disk 3 from the 1997 re-release
";

    /// Writes the sample games and images of the screenshots into a root.
    fn sample_root(name: &str) -> EditorRoot {
        let root = EditorRoot::with_system(name, "pc98");
        root.write(
            "config/pc98.conf",
            "machine = PC9801RA\nbios = on\ncrt = off\nscaling = nearest\n",
        );
        root.write("config/pc88.conf", "machine = PC8801MC\n");
        for image in [
            "games/98/YU-NO/YU-NO (Disk 1 of 4).hdm",
            "games/98/YU-NO/YU-NO (Disk 2 of 4).hdm",
            "games/98/YU-NO/YU-NO (Disk 3 of 4) [Patched 1997, fixed save bug in the Kumi route].hdm",
            "games/98/YU-NO/YU-NO (Disk 4 of 4).hdm",
            "games/98/YU-NO/ユーノ ユーザーディスク.d88",
            "games/98/YU-NO/yu-no.hdi",
            "games/98/YU-NO/Manual/manual.pdf",
            "games/98/YU-NO/Saves/save.d88",
            "games/p6/Portopia Renzoku Satsujin Jiken.p6t",
        ] {
            root.write(image, "");
        }
        root.game("yu-no", YU_NO);
        root.game(
            "brandish",
            "meta.title = Brandish\nmeta.title-original = ブランディッシュ\n\
             meta.developer = Nihon Falcom\nmeta.publisher = Nihon Falcom\nmeta.year = 1991\n\
             meta.genre = Action RPG\nmeta.notes = Keep the scenario disk in FDD 2.\n\
             machine = PC9801VM\n",
        );
        root.game("brandish-copy", "meta.title = Brandish (older copy)\n");
        root.write(
            "config/pc88/thexder.conf",
            "meta.title = Thexder\nmachine = PC8801MC\ncpu-mode = low\nboot-mode = v1s\n",
        );
        root.write(
            "config/pc6000/portopia.conf",
            "meta.title = Portopia Renzoku Satsujin Jiken\nmachine = PC6001MK2SR\n\
             cassette = games/p6/Portopia Renzoku Satsujin Jiken.p6t\npc6000-phase = 0\n",
        );
        root
    }

    /// Selects the game `title` of the selected system in the table.
    fn select_game(harness: &mut GuiHarness, manager: &mut Manager, title: &str) {
        let index = titles(manager)
            .iter()
            .position(|candidate| candidate == title)
            .expect("the game is listed");
        manager.view.table.list.select(index);
        let rows = manager.rows().to_vec();
        manager.view.record(&manager.entries, &rows);
        harness.state.set_focus(UiId::from(GAME_TABLE_ID));
        settle(harness, manager);
    }

    /// Opens YU-NO of the selected system in the editor, steps its year up
    /// by one and shows the Machine section with Model focused.
    fn open_with_year_step(harness: &mut GuiHarness, manager: &mut Manager) {
        select_game(
            harness,
            manager,
            "YU-NO: A Girl Who Chants Love at the Bound of this World",
        );
        harness.input.shortcut = Some(Shortcut::F2);
        settle(harness, manager);
        harness
            .state
            .set_focus(crate::editor::field_id(crate::config::metadata::YEAR_KEY));
        settle(harness, manager);
        harness.input.navigation = Some(Navigation::Right);
        settle(harness, manager);
        harness.input.navigation = Some(Navigation::NextTab);
        settle(harness, manager);
    }

    /// Renders the editor at `width` x `height` logical units and `scale`:
    /// every section of a game, the dropdown popup, the file browser, the
    /// discard question, the question over a file that changed outside the
    /// manager, a save error, a new game, a duplicate, the warning about
    /// system defaults that do not read, the empty media lists and the
    /// sections of other families. With `large` only the sections of a game.
    fn screenshots(width: f32, height: f32, scale: f32, large: bool) {
        let prefix = if large { "large_" } else { "" };
        let viewport = Viewport::new((width * scale) as u32, (height * scale) as u32, scale);
        let root = sample_root(&format!("editor_shots_{prefix}{scale}"));
        let mut manager = manager(&root);
        let mut harness = GuiHarness::with_viewport(viewport);
        let render = |harness: &GuiHarness, name: &str| {
            harness.write_screenshot(&format!("editor_{prefix}{name}_{scale}x"));
        };
        settle(&mut harness, &mut manager);
        select(&mut manager, "pc98");
        select_game(
            &mut harness,
            &mut manager,
            "YU-NO: A Girl Who Chants Love at the Bound of this World",
        );
        harness.input.shortcut = Some(Shortcut::F2);
        settle(&mut harness, &mut manager);
        render(&harness, "metadata");
        for name in ["machine", "media"] {
            harness.input.navigation = Some(Navigation::NextTab);
            settle(&mut harness, &mut manager);
            render(&harness, name);
        }
        if large {
            for name in ["sound", "display", "advanced"] {
                harness.input.navigation = Some(Navigation::NextTab);
                settle(&mut harness, &mut manager);
                render(&harness, name);
            }
            return;
        }
        harness.input.confirm = true;
        settle(&mut harness, &mut manager);
        harness.input.navigation = Some(Navigation::Down);
        settle(&mut harness, &mut manager);
        render(&harness, "browser");
        harness.input.cancel = true;
        settle(&mut harness, &mut manager);

        harness.input.navigation = Some(Navigation::NextTab);
        settle(&mut harness, &mut manager);
        harness.input.confirm = true;
        settle(&mut harness, &mut manager);
        harness.input.navigation = Some(Navigation::Down);
        settle(&mut harness, &mut manager);
        render(&harness, "sound");
        harness.input.cancel = true;
        settle(&mut harness, &mut manager);
        for name in ["display", "advanced"] {
            harness.input.navigation = Some(Navigation::NextTab);
            settle(&mut harness, &mut manager);
            render(&harness, name);
        }

        harness.input.navigation = Some(Navigation::NextTab);
        settle(&mut harness, &mut manager);
        harness.input.text = " (1997)".to_owned();
        settle(&mut harness, &mut manager);
        harness.input.cancel = true;
        settle(&mut harness, &mut manager);
        render(&harness, "discard");
        harness.input.confirm = true;
        settle(&mut harness, &mut manager);
        manager.editor = None;
        manager.next_focus = Some(NextFocus::Start);
        settle(&mut harness, &mut manager);

        open_with_year_step(&mut harness, &mut manager);
        let path = root.path("config/pc98/yu-no.conf");
        std::fs::write(&path, format!("{YU_NO}crt = on\n")).expect("file is written");
        harness.input.shortcut = Some(Shortcut::ControlS);
        settle(&mut harness, &mut manager);
        render(&harness, "changed_outside");
        harness.input.confirm = true;
        settle(&mut harness, &mut manager);

        std::fs::remove_file(&path).expect("file is removed");
        std::fs::create_dir_all(&path).expect("a folder takes the place of the file");
        harness.input.shortcut = Some(Shortcut::ControlS);
        settle(&mut harness, &mut manager);
        harness.input.navigation = Some(Navigation::Right);
        settle(&mut harness, &mut manager);
        harness.input.confirm = true;
        settle(&mut harness, &mut manager);
        render(&harness, "save_error");
        harness.input.confirm = true;
        settle(&mut harness, &mut manager);
        std::fs::remove_dir(&path).expect("the folder is removed");
        root.game("yu-no", YU_NO);
        manager.editor = None;
        manager.next_focus = Some(NextFocus::Start);
        settle(&mut harness, &mut manager);

        harness.input.shortcut = Some(Shortcut::Insert);
        settle(&mut harness, &mut manager);
        harness.input.text = "Brandish 2: The Planet Buster".to_owned();
        settle(&mut harness, &mut manager);
        render(&harness, "new");
        harness.input.navigation = Some(Navigation::NextTab);
        settle(&mut harness, &mut manager);
        render(&harness, "new_machine");
        manager.editor = None;
        settle(&mut harness, &mut manager);

        select_game(&mut harness, &mut manager, "Brandish");
        harness.input.shortcut = Some(Shortcut::ControlD);
        settle(&mut harness, &mut manager);
        render(&harness, "duplicate");
        manager.editor = None;

        let defaults = root.path("config/pc98.conf");
        let readable = std::fs::read(&defaults).expect("the system defaults read");
        std::fs::write(&defaults, b"machine = PC9801RA\nbios = on\ncrt = \xFF\n")
            .expect("file is written");
        open_with_year_step(&mut harness, &mut manager);
        render(&harness, "header_warning");
        manager.editor = None;
        std::fs::write(&defaults, readable).expect("file is written");
        empty_list_screenshot(viewport, scale);
        defaults_screenshots(viewport, scale);

        if compiled_systems().any(|system| system.name == "pc88") {
            select(&mut manager, "pc88");
            select_game(&mut harness, &mut manager, "Thexder");
            harness.input.shortcut = Some(Shortcut::F2);
            settle(&mut harness, &mut manager);
            harness.input.navigation = Some(Navigation::NextTab);
            settle(&mut harness, &mut manager);
            render(&harness, "pc88_machine");
            manager.editor = None;
        }
        if compiled_systems().any(|system| system.name == "pc6000") {
            select(&mut manager, "pc6000");
            select_game(
                &mut harness,
                &mut manager,
                "Portopia Renzoku Satsujin Jiken",
            );
            harness.input.shortcut = Some(Shortcut::F2);
            settle(&mut harness, &mut manager);
            for _ in 0..2 {
                harness.input.navigation = Some(Navigation::NextTab);
                settle(&mut harness, &mut manager);
            }
            render(&harness, "pc6000_media");
            for _ in 0..2 {
                harness.input.navigation = Some(Navigation::NextTab);
                settle(&mut harness, &mut manager);
            }
            harness
                .state
                .set_focus(crate::editor::field_id("pc6000-phase"));
            settle(&mut harness, &mut manager);
            harness.input.navigation = Some(Navigation::Right);
            settle(&mut harness, &mut manager);
            render(&harness, "pc6000_display");
        }
    }

    /// Renders the Media section of a game whose FDD 1 inherits the image
    /// of the settings file and whose FDD 2 holds `fdd2 =` over the
    /// inherited image of FDD 2, with the checkbox of FDD 1 focused.
    fn empty_list_screenshot(viewport: Viewport, scale: f32) {
        let root = sample_root(&format!("editor_shots_empty_{scale}"));
        root.write(
            "neetan.conf",
            "fdd1 = games/98/boot.d88\nfdd2 = games/98/data.d88\n",
        );
        root.game(
            "drives",
            "meta.title = A game with empty drives\nmachine = PC9801RA\nfdd2 =\n",
        );
        let mut manager = manager(&root);
        let mut harness = GuiHarness::with_viewport(viewport);
        settle(&mut harness, &mut manager);
        select(&mut manager, "pc98");
        select_game(&mut harness, &mut manager, "A game with empty drives");
        harness.input.shortcut = Some(Shortcut::F2);
        settle(&mut harness, &mut manager);
        for _ in 0..2 {
            harness.input.navigation = Some(Navigation::NextTab);
            settle(&mut harness, &mut manager);
        }
        harness
            .state
            .set_focus(crate::editor::empty_list_id(crate::editor::field_id(
                "fdd1",
            )));
        settle(&mut harness, &mut manager);
        harness.write_screenshot(&format!("editor_empty_list_{scale}x"));
    }

    /// Renders the Machine and the Media section of the editor of the PC-98
    /// system defaults, whose file sets the model, the real BIOS and an HDD
    /// image, the Media section of a game that inherits the HDD image, and
    /// the Media section of a game that starts without the HDD image.
    fn defaults_screenshots(viewport: Viewport, scale: f32) {
        let root = sample_root(&format!("editor_shots_defaults_{scale}"));
        root.write(
            "config/pc98.conf",
            "machine = PC9801RA\nbios = on\nhdd1 = games/98/system/MS-DOS 6.2.hdi\n",
        );
        root.write("games/98/system/MS-DOS 6.2.hdi", "");
        root.game(
            "diskless",
            "meta.title = Diskless start\nmachine = PC9801RA\nhdd1 =\n",
        );
        let mut manager = manager(&root);
        let mut harness = GuiHarness::with_viewport(viewport);
        let render = |harness: &GuiHarness, name: &str| {
            harness.write_screenshot(&format!("editor_{name}_{scale}x"));
        };
        settle(&mut harness, &mut manager);
        select(&mut manager, "pc98");
        settle(&mut harness, &mut manager);
        harness.state.set_focus(UiId::from(SYSTEM_DEFAULTS_ID));
        settle(&mut harness, &mut manager);
        render(&harness, "defaults_button");
        harness.input.confirm = true;
        settle(&mut harness, &mut manager);
        render(&harness, "defaults_machine");
        harness.input.navigation = Some(Navigation::NextTab);
        settle(&mut harness, &mut manager);
        render(&harness, "defaults_media");
        harness.input.cancel = true;
        settle(&mut harness, &mut manager);

        select_game(&mut harness, &mut manager, "Brandish");
        harness.input.shortcut = Some(Shortcut::F2);
        settle(&mut harness, &mut manager);
        for _ in 0..2 {
            harness.input.navigation = Some(Navigation::NextTab);
            settle(&mut harness, &mut manager);
        }
        render(&harness, "system_default_media");
        harness.input.cancel = true;
        settle(&mut harness, &mut manager);

        select_game(&mut harness, &mut manager, "Diskless start");
        harness.input.shortcut = Some(Shortcut::F2);
        settle(&mut harness, &mut manager);
        for _ in 0..2 {
            harness.input.navigation = Some(Navigation::NextTab);
            settle(&mut harness, &mut manager);
        }
        render(&harness, "empty_hdd_media");
    }

    /// Writes the screenshots of the editor at the scales 1 and 2, at the
    /// minimum window size and at 1920 x 1080.
    #[test]
    #[ignore = "writes screenshots into the directory of GUI_SHOTS"]
    fn editor_screenshots() {
        for scale in [1.0, 2.0] {
            screenshots(960.0, 600.0, scale, false);
            screenshots(1920.0, 1080.0, scale, true);
        }
    }
}
