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
    editor::{EditedFile, Editor, OpenError, SaveFailure},
    texts::hints_in,
};

/// Logs the save failure `failure` and returns the error dialog of the file
/// of `editor`. The text of each failure holds the path of the file.
fn save_failure_dialog(editor: &Editor, failure: &SaveFailure) -> ErrorDialog {
    warn!("Game manager: save failed: {failure}");
    let subject = editor.edited_file();
    match failure {
        SaveFailure::Error { .. } | SaveFailure::NoFreeName(_) => {
            ErrorDialog::save_failed(&subject, failure)
        }
        SaveFailure::ChangedOutside(shown) => ErrorDialog::changed_outside(&subject, shown),
    }
}

/// The error dialog of the game `name` of `system` that did not open in the
/// editor with `error`.
fn game_open_failed(system: &'static System, name: &str, error: &OpenError) -> ErrorDialog {
    let subject = EditedFile::Game {
        system,
        name: name.to_owned(),
    };
    ErrorDialog::open_failed(&subject, error)
}

impl Manager {
    /// Opens `editor`, or the error dialog of the editor that did not open.
    pub(super) fn open_editor(
        &mut self,
        context: &mut UiContext,
        editor: Result<Editor, ErrorDialog>,
    ) {
        match editor {
            Ok(editor) => {
                info!("Game manager: editing {}", editor.path().display());
                self.editor = Some(editor);
            }
            Err(dialog) => self.dialog = Some(dialog),
        }
        context.request_pass();
    }

    /// Opens the editor on a new game of the selected system.
    pub(super) fn open_new_game(&mut self, context: &mut UiContext) {
        let Some(system) = self.current().map(|entry| entry.system) else {
            return;
        };
        let editor = Editor::new_game(&self.host, system).ok_or_else(|| {
            ErrorDialog::new_game_open_failed(system, &OpenError::NotCompiled(system.name))
        });
        self.open_editor(context, editor);
    }

    /// Opens the editor on the game file of `row`.
    pub(super) fn open_game(&mut self, context: &mut UiContext, row: &GameRow) {
        let editor = Editor::open(&self.host, row.system, &row.path)
            .map_err(|error| game_open_failed(row.system, row.name(), &error));
        self.open_editor(context, editor);
    }

    /// Opens the editor on a copy of the game file of `row` with the copy
    /// pattern of the catalog of `context`.
    pub(super) fn open_duplicate(&mut self, context: &mut UiContext, row: &GameRow) {
        let editor = Editor::duplicate(&self.host, row.system, &row.path, context.catalog)
            .map_err(|error| game_open_failed(row.system, row.name(), &error));
        self.open_editor(context, editor);
    }

    /// Opens the editor on the system defaults file of the selected system.
    pub(super) fn open_system_defaults(&mut self, context: &mut UiContext) {
        let Some(system) = self.current().map(|entry| entry.system) else {
            return;
        };
        let editor = Editor::system_defaults(&self.host, system)
            .map_err(|error| ErrorDialog::open_failed(&EditedFile::SystemDefaults(system), &error));
        self.open_editor(context, editor);
    }

    /// Opens the editor on the game file of `target`, the game of an error dialog.
    pub(super) fn open_target(&mut self, context: &mut UiContext, target: &EditTarget) {
        let name = target
            .path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let editor = Editor::open(&self.host, target.system, &target.path)
            .map_err(|error| game_open_failed(target.system, &name, &error));
        self.open_editor(context, editor);
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
    /// the settings file. `active_language` is the tag of the catalog of
    /// the pass.
    fn file_saved(&mut self, edited: &EditedFile, path: &Path, active_language: &str) {
        match *edited {
            EditedFile::Game { system, .. } => self.game_saved(system, path),
            EditedFile::SystemDefaults(system) => self.system_defaults_saved(system, path),
            EditedFile::Settings => self.settings_saved(path, active_language),
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
        let catalog = context.catalog;
        let active_language = catalog.tag();
        let edited = pass.saved.as_ref().map(|_| editor.edited_file());
        if let Some(failure) = &pass.save_error {
            self.dialog = Some(save_failure_dialog(editor, failure));
        }
        if let (Some(path), Some(edited)) = (&pass.saved, &edited) {
            self.file_saved(edited, path, active_language);
        }
        let answer = self
            .dialog
            .as_mut()
            .map_or(DialogAnswer::Open, |dialog| dialog.show(context, screen));
        if answer != DialogAnswer::Open {
            self.dialog = None;
        }
        if answer == DialogAnswer::Overwrite
            && let Some(editor) = self.editor.as_mut()
        {
            let written = match editor.overwrite() {
                Ok(path) => Ok((path, editor.edited_file())),
                Err(failure) => Err(save_failure_dialog(editor, &failure)),
            };
            match written {
                Ok((path, edited)) => self.file_saved(&edited, &path, active_language),
                Err(dialog) => self.dialog = Some(dialog),
            }
            context.request_pass();
        }
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        let hints = match &self.dialog {
            Some(dialog) => hints_in(catalog, &dialog.hints(context)),
            None => hints_in(catalog, editor.hints()),
        };
        let shown_path = editor.shown_path_text();
        status_line(context, status_area, &hints, StatusEnd::Path(shown_path));
        if pass.closed {
            let next = match editor.edited_file() {
                EditedFile::Game { .. } => NextFocus::Start,
                EditedFile::SystemDefaults(_) => NextFocus::Widget(UiId::from(SYSTEM_DEFAULTS_ID)),
                EditedFile::Settings => NextFocus::Widget(UiId::from(SETTINGS_ID)),
            };
            self.editor = None;
            self.dialog = None;
            self.next_focus = Some(next);
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
    use gui::ui::{ActionButton, Navigation, Shortcut, UiId};
    #[cfg(feature = "pc98")]
    use gui::ui::{ThemeKind, Viewport};
    #[cfg(feature = "pc98")]
    use machine_factory::config::systems::compiled_systems;

    use super::*;
    use crate::{
        config::{LaunchFailure, LaunchRequest, ManagerStart},
        editor::test_support::EditorRoot,
        gui_test_support::GuiHarness,
        manager::{FILTER_ID, game_table::GAME_TABLE_ID, test_support::titles},
    };
    #[cfg(feature = "pc98")]
    use crate::{
        editor::EditorSection,
        gui_test_support::{MINIMUM_HEIGHT, MINIMUM_WIDTH, PassCounts},
        manager::test_support::{select, shown_help},
        texts::repository::{RepositoryLanguage, repository_languages},
    };

    /// The manager of `root`.
    fn manager(root: &EditorRoot) -> Manager {
        Manager::new(ManagerStart {
            request: LaunchRequest::default(),
            locations: root.locations(),
        })
        .expect("manager starts")
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
        let mut harness = GuiHarness::minimum();
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
        let mut harness = GuiHarness::minimum();
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
        let mut harness = GuiHarness::minimum();
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
        let mut harness = GuiHarness::minimum();
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
        let mut harness = GuiHarness::minimum();
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
        let mut harness = GuiHarness::minimum();
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
            Some(
                "Brandish! was not saved. Overwrite replaces the file with the changes of the \
                 editor. Cancel returns to the editor with the changes."
            )
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
            Some("Brandish! was not saved. The editor keeps the changes.")
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
        let mut harness = GuiHarness::minimum();
        settle(&mut harness, &mut manager);
        let launch = manager.launch(&manager.rows()[1].clone());
        let layer = manager.launch_failed(&launch, &LaunchFailure::missing_image("/a.d88"));
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
        let mut harness = GuiHarness::minimum();
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
        let mut harness = GuiHarness::minimum();
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

    #[cfg(feature = "pc98")]
    /// The settings of the game of the steady frame test after the game file of YU-NO: a
    /// second floppy drive, a missing hard disk image, display values and values that do not
    /// read. Every section holds a set value and a note of two lines at the minimum window
    /// size.
    const STEADY_SETTINGS: &str = "ems = maybe\nfdd2 = games/98/YU-NO/Saves/save.d88\n\
         hdd2 = games/98/YU-NO/Saves/missing.hdi\nadpcm-ram = maybe\ncrt = on\n\
         aspect-mode = wide\nenable-extractor = sometimes\n";

    #[cfg(feature = "pc98")]
    /// The stem of the game file of the steady frame test. The note of the title holds the
    /// stem and takes two lines at the minimum window size.
    const STEADY_STEM: &str = "yu-no-a-girl-who-chants-love-at-the-bound-of-this-world";

    #[cfg(feature = "pc98")]
    /// Opens YU-NO with [`STEADY_SETTINGS`] from the game file [`STEADY_STEM`] in the editor
    /// of a manager over [`sample_root`].
    fn steady_editor(name: &str) -> (EditorRoot, Manager, GuiHarness) {
        let root = sample_root(name);
        std::fs::remove_file(root.path("config/pc98/yu-no.conf")).expect("file is removed");
        root.game(STEADY_STEM, &format!("{YU_NO}{STEADY_SETTINGS}"));
        let mut manager = manager(&root);
        let mut harness = GuiHarness::minimum();
        settle(&mut harness, &mut manager);
        select(&mut manager, "pc98");
        select_game(
            &mut harness,
            &mut manager,
            "YU-NO: A Girl Who Chants Love at the Bound of this World",
        );
        harness.input.shortcut = Some(Shortcut::F2);
        settle(&mut harness, &mut manager);
        assert!(manager.editor().is_some());
        (root, manager, harness)
    }

    #[cfg(feature = "pc98")]
    /// Counts the allocations, the text fills and the wraps of a steady frame of `manager`, the
    /// second pass with an unchanged state, and checks that the frame keeps the revision of the
    /// editor.
    fn steady_frame(harness: &mut GuiHarness, manager: &mut Manager) -> PassCounts {
        harness.frame(|context| manager.show(context));
        let revision = manager.editor().map(Editor::revision);
        let (counts, _) = harness.counted_frame(|context| manager.show(context));
        assert_eq!(manager.editor().map(Editor::revision), revision);
        counts
    }

    #[cfg(feature = "pc98")]
    /// Verifies that a steady frame of each section of the game editor allocates nothing, fills
    /// no text again and keeps the revision of the editor. The game holds media images in
    /// several drives and a set value in every section. Every section after Metadata focuses a
    /// setting row with its help text.
    #[test]
    fn a_steady_frame_of_the_editor_allocates_nothing() {
        let (_root, mut manager, mut harness) = steady_editor("steady_editor");
        let sections = manager.editor().map(Editor::sections).unwrap_or_default();
        assert_eq!(sections.len(), 6);
        for section in sections {
            assert_eq!(
                manager.editor().map(Editor::current_section),
                Some(*section)
            );
            assert!(
                manager
                    .editor()
                    .is_some_and(|editor| editor.section_has_set_field(*section)),
                "the section {section:?} holds no set value"
            );
            assert!(
                manager
                    .editor()
                    .is_some_and(|editor| editor.most_note_lines(*section) > 1),
                "the section {section:?} draws no note of several lines"
            );
            assert_eq!(
                steady_frame(&mut harness, &mut manager),
                PassCounts::NONE,
                "a steady frame of the section {section:?}"
            );
            assert_eq!(
                shown_help(&mut harness, &mut manager).is_some(),
                *section != EditorSection::Metadata,
                "the help area of the section {section:?}"
            );
            harness.input.navigation = Some(Navigation::NextTab);
            settle(&mut harness, &mut manager);
        }
    }

    #[cfg(feature = "pc98")]
    /// Verifies that a steady frame of the file browser over the editor allocates nothing and
    /// fills no text again, with the current image of the drive selected.
    #[test]
    fn a_steady_frame_of_the_file_browser_allocates_nothing() {
        let (_root, mut manager, mut harness) = steady_editor("steady_browser");
        for _ in 0..2 {
            harness.input.navigation = Some(Navigation::NextTab);
            settle(&mut harness, &mut manager);
        }
        assert_eq!(
            manager.editor().map(Editor::current_section),
            Some(EditorSection::Media)
        );
        harness.input.confirm = true;
        settle(&mut harness, &mut manager);
        assert!(manager.editor().is_some_and(Editor::browser_open));
        assert_eq!(
            steady_frame(&mut harness, &mut manager),
            PassCounts::NONE,
            "a steady frame of the file browser"
        );
    }

    #[cfg(feature = "pc98")]
    /// Verifies that a steady frame of the discard question, of the question over a file that
    /// changed outside the manager and of a save error allocates nothing, fills no text and wraps
    /// no text, with the focus on each button.
    #[test]
    fn a_steady_frame_of_an_editor_dialog_allocates_nothing() {
        let (root, mut manager, mut harness) = steady_editor("steady_editor_dialogs");
        let assert_steady = |harness: &mut GuiHarness, manager: &mut Manager, focus: UiId| {
            assert_eq!(harness.state.focused(), Some(focus));
            assert_eq!(
                steady_frame(harness, manager),
                PassCounts::NONE,
                "a steady frame of the dialog with the focus on {focus:?}"
            );
        };
        harness.input.text = " (1997)".to_owned();
        settle(&mut harness, &mut manager);
        harness.input.cancel = true;
        settle(&mut harness, &mut manager);
        let discard = UiId::from(crate::editor::DISCARD_LAYER_ID);
        assert_steady(&mut harness, &mut manager, discard.child("cancel"));
        harness.input.navigation = Some(Navigation::Right);
        settle(&mut harness, &mut manager);
        assert_steady(&mut harness, &mut manager, discard.child("action"));
        harness.input.cancel = true;
        settle(&mut harness, &mut manager);
        assert!(manager.editor().is_some());

        let path = root.path(&format!("config/pc98/{STEADY_STEM}.conf"));
        std::fs::write(&path, format!("{YU_NO}crt = on\n")).expect("file is written");
        harness.input.shortcut = Some(Shortcut::ControlS);
        settle(&mut harness, &mut manager);
        assert!(manager.dialog().is_some());
        assert_steady(
            &mut harness,
            &mut manager,
            UiId::from(super::super::error_dialog::CANCEL_ID),
        );
        harness.input.navigation = Some(Navigation::Right);
        settle(&mut harness, &mut manager);
        assert_steady(
            &mut harness,
            &mut manager,
            UiId::from(super::super::error_dialog::OVERWRITE_ID),
        );

        std::fs::remove_file(&path).expect("file is removed");
        std::fs::create_dir_all(&path).expect("a folder takes the place of the file");
        harness.input.confirm = true;
        settle(&mut harness, &mut manager);
        assert!(
            manager
                .dialog()
                .and_then(ErrorDialog::message)
                .is_some_and(|message| message.ends_with("The editor keeps the changes.")),
            "the save error opens"
        );
        assert_steady(
            &mut harness,
            &mut manager,
            UiId::from(super::super::error_dialog::OK_ID),
        );
    }

    #[cfg(feature = "pc98")]
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

    #[cfg(feature = "pc98")]
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

    #[cfg(feature = "pc98")]
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

    #[cfg(feature = "pc98")]
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

    #[cfg(feature = "pc98")]
    /// Renders the editor at `width` x `height` logical units and `scale`:
    /// every section of a game, the dropdown popup, the file browser, the
    /// discard question, the question over a file that changed outside the
    /// manager, a save error, a new game, a duplicate, the warning about
    /// system defaults that do not read, the empty media lists and the
    /// sections of other families with the help text of the CPU mode. With
    /// `large` only the sections of a game. The texts come from the catalog
    /// of `language`.
    fn screenshots(
        width: f32,
        height: f32,
        scale: f32,
        large: bool,
        kind: ThemeKind,
        language: &RepositoryLanguage,
    ) {
        let prefix = if large { "large_" } else { "" };
        let viewport = Viewport::new((width * scale) as u32, (height * scale) as u32, scale);
        let root = sample_root(&format!("editor_shots_{prefix}{scale}"));
        let mut manager = manager(&root);
        let mut harness = GuiHarness::with_viewport(viewport, kind, language.catalog());
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
        empty_list_screenshot(viewport, scale, kind, language);
        defaults_screenshots(viewport, scale, kind, language);

        if compiled_systems().any(|system| system.name == "pc88") {
            select(&mut manager, "pc88");
            select_game(&mut harness, &mut manager, "Thexder");
            harness.input.shortcut = Some(Shortcut::F2);
            settle(&mut harness, &mut manager);
            harness.input.navigation = Some(Navigation::NextTab);
            settle(&mut harness, &mut manager);
            render(&harness, "pc88_machine");
            harness.state.set_focus(crate::editor::field_id("cpu-mode"));
            settle(&mut harness, &mut manager);
            render(&harness, "pc88_cpu_mode");
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

    #[cfg(feature = "pc98")]
    /// Renders the Media section of a game whose FDD 1 inherits the image
    /// of the settings file and whose FDD 2 holds `fdd2 =` over the
    /// inherited image of FDD 2, with the checkbox of FDD 1 focused, in the
    /// theme `kind` with the texts of the catalog of `language`.
    fn empty_list_screenshot(
        viewport: Viewport,
        scale: f32,
        kind: ThemeKind,
        language: &RepositoryLanguage,
    ) {
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
        let mut harness = GuiHarness::with_viewport(viewport, kind, language.catalog());
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

    #[cfg(feature = "pc98")]
    /// Renders the Machine and the Media section of the editor of the PC-98
    /// system defaults, whose file sets the model, the real BIOS and an HDD
    /// image, the Media section of a game that inherits the HDD image, and
    /// the Media section of a game that starts without the HDD image, in the
    /// theme `kind` with the texts of the catalog of `language`.
    fn defaults_screenshots(
        viewport: Viewport,
        scale: f32,
        kind: ThemeKind,
        language: &RepositoryLanguage,
    ) {
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
        let mut harness = GuiHarness::with_viewport(viewport, kind, language.catalog());
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

    #[cfg(feature = "pc98")]
    /// Writes the screenshots of the editor in both themes and in every language
    /// of the repository at the scales 1 and 2, at the minimum window size
    /// and at 1920 x 1080.
    #[test]
    #[ignore = "writes screenshots into the directory of GUI_SHOTS"]
    fn editor_screenshots() {
        for language in repository_languages() {
            for kind in [ThemeKind::Dark, ThemeKind::Light] {
                for scale in [1.0, 2.0] {
                    screenshots(MINIMUM_WIDTH, MINIMUM_HEIGHT, scale, false, kind, language);
                    screenshots(1920.0, 1080.0, scale, true, kind, language);
                }
            }
        }
    }
}
