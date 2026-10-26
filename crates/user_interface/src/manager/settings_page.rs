//! The settings page in the manager: opening the settings page from the
//! library, and the host settings that the manager applies after a save of
//! the settings file.

use std::path::Path;

use common::{info, warn};
use gui::ui::UiContext;
use machine_factory::config::systems::rom_folders;

use super::{ErrorDialog, Manager};
use crate::{
    config::HostLayers,
    editor::{EditedFile, Editor},
    language::catalog_of,
    rom_folders::rom_folder_path,
};

/// Whether the host settings `before` and `after` give another config
/// folder, ROM folder or folder of a ROM key.
fn folders_changed(before: &HostLayers, after: &HostLayers) -> bool {
    before.config_folder() != after.config_folder()
        || before.roms_folder() != after.roms_folder()
        || rom_folders()
            .any(|folder| rom_folder_path(before, &folder) != rom_folder_path(after, &folder))
}

impl Manager {
    /// Opens the settings page on the settings file with the ROM statuses
    /// of the last ROM check.
    pub(super) fn open_settings(&mut self, context: &mut UiContext) {
        let editor = Editor::settings(&self.host, &self.roms)
            .map_err(|error| ErrorDialog::open_failed(&EditedFile::Settings, &error));
        self.open_editor(context, editor);
    }

    /// Reads the host settings again after a save of the settings file
    /// `path` and applies them: the next launch and the editors take the
    /// new settings. A save that changes the config folder, the ROM folder
    /// or the folder of a ROM key checks the ROM folders and scans the
    /// catalog again, as Refresh does. A save that changes the language of
    /// the user interface loads the catalog of the new value and keeps it
    /// for the switch when its tag differs from `active_language` without
    /// regard to case. A settings file that does not read keeps the host
    /// settings and opens the error dialog.
    pub(super) fn settings_saved(&mut self, path: &Path, active_language: &str) {
        info!("Game manager: saved {}", path.display());
        let host = match HostLayers::read(&self.start.request, &self.start.locations) {
            Ok(host) => host,
            Err(error) => {
                warn!("Game manager: the settings do not read: {error}");
                self.dialog = Some(ErrorDialog::settings_not_applied(&error));
                return;
            }
        };
        let rescan = folders_changed(&self.host, &host);
        let language = host.gui_language();
        if language != self.host.gui_language() {
            let catalog = catalog_of(&language, &host.language_folders());
            let tag = catalog.catalog().tag();
            if !tag.eq_ignore_ascii_case(active_language) {
                info!("Game manager: the GUI language is {tag}");
                self.catalog_switch = Some(catalog);
            }
        }
        self.host = host;
        self.config_folder = self.host.config_folder();
        if rescan {
            self.refresh();
        }
        let statuses = rescan.then_some(self.roms.as_slice());
        if let Some(editor) = self.editor.as_mut() {
            editor.host_changed(&self.host, statuses);
        }
    }
}

#[cfg(test)]
mod tests {
    use gui::ui::{ActionButton, Navigation, Shortcut, ThemeKind, UiId, Viewport};
    use machine_factory::{
        config::settings::{
            AUDIO_VOLUME_KEY, BACKEND_KEY, CONFIG_FOLDER_KEY, GUI_LANGUAGE_KEY, MACHINE_KEY,
            ROMS_FOLDER_KEY, find_setting,
        },
        rom_status::{RomCheck, RomStatus},
    };

    use super::*;
    use crate::{
        config::{LaunchRequest, ManagerStart},
        editor::{
            EditedFile, field_id,
            test_support::{EditorRoot, SettingsRoot, first_rom_folder},
        },
        gui_test_support::{GuiHarness, MINIMUM_HEIGHT, MINIMUM_WIDTH, PassCounts},
        manager::{FILTER_ID, SETTINGS_ID, game_table::GAME_TABLE_ID, test_support::shown_help},
        rom_folders::RomFolderStatus,
        texts::repository::{RepositoryLanguage, repository_languages},
    };

    /// The manager of `root` with the setting flags `flags`.
    fn manager_with(root: &EditorRoot, flags: &[(&str, &str)]) -> Manager {
        let request = LaunchRequest {
            flags: flags
                .iter()
                .map(|(key, value)| (find_setting(key).expect("a key"), (*value).to_owned()))
                .collect(),
            ..LaunchRequest::default()
        };
        Manager::new(ManagerStart {
            request,
            locations: root.locations(),
        })
        .expect("manager starts")
    }

    /// Draws passes of `manager` until it settles.
    fn settle(harness: &mut GuiHarness, manager: &mut Manager) {
        harness.settle(|context| manager.show(context));
    }

    /// The titles of the games of the selected system.
    fn titles(manager: &Manager) -> Vec<String> {
        let mut titles: Vec<String> = manager.rows().iter().map(|row| row.title.clone()).collect();
        titles.sort();
        titles
    }

    /// Whether the open editor is the settings page.
    fn settings_open(manager: &Manager) -> bool {
        manager
            .editor()
            .is_some_and(|editor| editor.edited_file() == EditedFile::Settings)
    }

    /// Verifies that the settings button, F10 and Start open the settings page, that Esc gives the focus back to the button, and that F10 is inactive in the filter.
    #[test]
    fn the_settings_button_f10_and_start_open_the_page() {
        let root = EditorRoot::new("settings_open");
        root.game("edge", "meta.title = Edge\n");
        let mut manager = manager_with(&root, &[]);
        let mut harness = GuiHarness::minimum();
        settle(&mut harness, &mut manager);
        assert_eq!(harness.state.focused(), Some(UiId::from(GAME_TABLE_ID)));
        let opens: [fn(&mut GuiHarness); 3] = [
            |harness| harness.input.shortcut = Some(Shortcut::F10),
            |harness| harness.input.action = Some(ActionButton::Start),
            |harness| {
                harness.state.set_focus(UiId::from(SETTINGS_ID));
                harness.input.confirm = true;
            },
        ];
        for open in opens {
            open(&mut harness);
            settle(&mut harness, &mut manager);
            assert!(settings_open(&manager));
            assert_eq!(
                manager.editor().expect("the page is open").path(),
                root.path("neetan.conf")
            );
            assert_eq!(harness.state.focused(), Some(field_id(CONFIG_FOLDER_KEY)));
            harness.input.cancel = true;
            settle(&mut harness, &mut manager);
            assert!(manager.editor().is_none());
            assert_eq!(harness.state.focused(), Some(UiId::from(SETTINGS_ID)));
        }
        harness.state.set_focus(UiId::from(FILTER_ID));
        settle(&mut harness, &mut manager);
        harness.input.shortcut = Some(Shortcut::F10);
        settle(&mut harness, &mut manager);
        assert!(manager.editor().is_none());
    }

    /// Verifies that the settings page shows the help text of its focused host setting row and
    /// follows a focus change to another row.
    #[test]
    fn the_settings_page_shows_the_help_of_the_focused_row() {
        let root = EditorRoot::new("settings_help");
        let mut manager = manager_with(&root, &[]);
        let mut harness = GuiHarness::minimum();
        settle(&mut harness, &mut manager);
        harness.input.shortcut = Some(Shortcut::F10);
        settle(&mut harness, &mut manager);
        assert_eq!(
            shown_help(&mut harness, &mut manager),
            Some(CONFIG_FOLDER_KEY)
        );
        harness.input.navigation = Some(Navigation::Down);
        settle(&mut harness, &mut manager);
        let below = shown_help(&mut harness, &mut manager).expect("the row below shows its help");
        assert_ne!(below, CONFIG_FOLDER_KEY);
        for _ in 0..2 {
            harness.input.navigation = Some(Navigation::NextTab);
            settle(&mut harness, &mut manager);
        }
        harness.state.set_focus(field_id(GUI_LANGUAGE_KEY));
        settle(&mut harness, &mut manager);
        assert_eq!(
            shown_help(&mut harness, &mut manager),
            Some(GUI_LANGUAGE_KEY)
        );
    }

    /// Verifies that a save that changes the config folder scans the catalog of the new folder, and that a save of the audio volume alone scans nothing.
    #[test]
    fn a_new_config_folder_rescans_and_the_volume_does_not() {
        let root = EditorRoot::new("settings_rescan");
        let system = root.system().name;
        root.game("edge", "meta.title = Edge\n");
        root.write(
            &format!("games/{system}/brandish.conf"),
            "meta.title = Brandish\n",
        );
        let mut manager = manager_with(&root, &[]);
        let mut harness = GuiHarness::minimum();
        settle(&mut harness, &mut manager);
        assert_eq!(titles(&manager), ["Edge"]);

        harness.input.shortcut = Some(Shortcut::F10);
        settle(&mut harness, &mut manager);
        harness.input.text = "games".to_owned();
        settle(&mut harness, &mut manager);
        harness.input.shortcut = Some(Shortcut::ControlS);
        settle(&mut harness, &mut manager);
        assert_eq!(
            std::fs::read_to_string(root.path("neetan.conf")).expect("settings read"),
            "config-folder = games\n"
        );
        assert_eq!(manager.config_folder, root.path("games"));
        assert_eq!(titles(&manager), ["Brandish"]);
        harness.input.cancel = true;
        settle(&mut harness, &mut manager);
        assert!(manager.editor().is_none());

        root.write(
            &format!("games/{system}/later.conf"),
            "meta.title = Later\n",
        );
        harness.input.shortcut = Some(Shortcut::F10);
        settle(&mut harness, &mut manager);
        for _ in 0..2 {
            harness.input.navigation = Some(Navigation::NextTab);
            settle(&mut harness, &mut manager);
        }
        harness.state.set_focus(field_id(AUDIO_VOLUME_KEY));
        settle(&mut harness, &mut manager);
        harness.input.text = "0.5".to_owned();
        settle(&mut harness, &mut manager);
        harness.input.shortcut = Some(Shortcut::ControlS);
        settle(&mut harness, &mut manager);
        assert_eq!(
            std::fs::read_to_string(root.path("neetan.conf")).expect("settings read"),
            "config-folder = games\naudio-volume = 0.5\n"
        );
        assert_eq!(titles(&manager), ["Brandish"], "the save scans nothing");
        harness.input.cancel = true;
        settle(&mut harness, &mut manager);
        assert!(manager.editor().is_none());
        harness.input.shortcut = Some(Shortcut::F5);
        settle(&mut harness, &mut manager);
        assert_eq!(titles(&manager), ["Brandish", "Later"]);
    }

    /// Opens the popup of the focused dropdown, renders the frame and closes the popup again.
    fn popup_pixels(harness: &mut GuiHarness, manager: &mut Manager) -> Vec<u8> {
        harness.input.confirm = true;
        settle(harness, manager);
        assert!(harness.state.popup_open());
        let pixels = harness.render();
        harness.input.cancel = true;
        settle(harness, manager);
        assert!(!harness.state.popup_open());
        pixels
    }

    /// Verifies that a save of another language on the settings page switches the catalog once and that the next frame draws the texts of the new catalog.
    #[test]
    fn a_saved_language_switches_the_texts_of_the_next_frame() {
        let root = EditorRoot::new("settings_language");
        root.write(
            "language/de.ini",
            &crate::texts::test_language("[host-settings]\nlanguage-auto = Systemsprache\n"),
        );
        let mut manager = manager_with(&root, &[]);
        let mut harness = GuiHarness::minimum();
        settle(&mut harness, &mut manager);
        harness.input.shortcut = Some(Shortcut::F10);
        settle(&mut harness, &mut manager);
        for _ in 0..2 {
            harness.input.navigation = Some(Navigation::NextTab);
            settle(&mut harness, &mut manager);
        }
        harness.state.set_focus(field_id(GUI_LANGUAGE_KEY));
        settle(&mut harness, &mut manager);
        harness.input.confirm = true;
        settle(&mut harness, &mut manager);
        harness.input.navigation = Some(Navigation::End);
        settle(&mut harness, &mut manager);
        harness.input.confirm = true;
        settle(&mut harness, &mut manager);
        harness.input.shortcut = Some(Shortcut::ControlS);
        settle(&mut harness, &mut manager);
        assert_eq!(
            std::fs::read_to_string(root.path("neetan.conf")).expect("settings read"),
            "gui-language = de\n"
        );
        assert!(settings_open(&manager));

        let english = popup_pixels(&mut harness, &mut manager);
        assert_eq!(popup_pixels(&mut harness, &mut manager), english);
        let catalog = manager
            .take_catalog_switch()
            .expect("the save switches the language");
        assert_eq!(catalog.catalog().name(), "Deutsch");
        assert!(manager.take_catalog_switch().is_none());
        harness.set_catalog(catalog);
        assert_ne!(popup_pixels(&mut harness, &mut manager), english);
    }

    /// Verifies that a save whose language resolves to the active language keeps the catalog: the active tag in other case, and `en` over an `auto` that gives English.
    #[test]
    fn a_save_that_keeps_the_language_switches_nothing() {
        let root = EditorRoot::new("settings_same_language");
        root.write("language/de.ini", &crate::texts::test_language(""));
        root.write("neetan.conf", "gui-language = de\n");
        let mut manager = manager_with(&root, &[]);
        root.write("neetan.conf", "gui-language = DE\n");
        manager.settings_saved(&root.path("neetan.conf"), "de");
        assert!(manager.take_catalog_switch().is_none());
        root.write("neetan.conf", "gui-language = en\n");
        manager.settings_saved(&root.path("neetan.conf"), "de");
        let catalog = manager
            .take_catalog_switch()
            .expect("English differs from German");
        assert_eq!(catalog.catalog().tag(), "en");

        let root = EditorRoot::new("settings_auto_english");
        root.write("neetan.conf", "gui-language = auto\n");
        let mut manager = manager_with(&root, &[]);
        root.write("neetan.conf", "gui-language = en\n");
        manager.settings_saved(&root.path("neetan.conf"), "en");
        assert!(manager.take_catalog_switch().is_none());
    }

    /// Verifies that another folder of a ROM key changes the folders and that another audio volume does not.
    #[test]
    fn a_rom_key_changes_the_folders_and_the_volume_does_not() {
        let test = SettingsRoot::new("folders_changed", Some(""));
        let folder = first_rom_folder();
        let before = test.host(&[]);
        let elsewhere = test.root().join("elsewhere").display().to_string();
        assert!(folders_changed(
            &before,
            &test.host(&[(folder.key, &elsewhere)])
        ));
        assert!(!folders_changed(
            &before,
            &test.host(&[(AUDIO_VOLUME_KEY, "0.5")])
        ));
        assert!(!folders_changed(&before, &test.host(&[])));
    }

    /// The ROM statuses of the screenshots, one of each kind.
    fn sample_statuses(host: &HostLayers) -> Vec<RomFolderStatus> {
        rom_folders()
            .map(|folder| {
                let (status, models_without_set): (RomStatus, &[&'static str]) = match folder.name {
                    "fm7" | "pc6000" | "pc88va" => (RomStatus::Missing, &[]),
                    "at" | "sc55" => (RomStatus::NotFound, &[]),
                    "pc98" => (RomStatus::Partial, &["PC9801F", "PC9801VM"]),
                    "towns" => (RomStatus::Partial, &["FMTownsIIMX"]),
                    _ => (RomStatus::Found, &[]),
                };
                RomFolderStatus {
                    folder,
                    location: rom_folder_path(host, &folder),
                    check: RomCheck {
                        status,
                        folder_exists: true,
                        models_without_set: models_without_set.to_vec(),
                    },
                }
            })
            .collect()
    }

    /// A root with a settings file, a config folder and a ROM folder for the
    /// screenshots. The settings file ends with the lines `extra`.
    fn sample_root(name: &str, extra: &str) -> EditorRoot {
        let root = EditorRoot::new(name);
        root.write(
            "neetan.conf",
            &format!(
                "; Neetan settings\naudio-volume = 0.8\npc88va-roms = /mnt/archive/mame/pc88va2\n\
                 key.F12 = Stop\n{extra}"
            ),
        );
        root.write("config/.keep", "");
        root.write("roms/.keep", "");
        root
    }

    /// Renders the sections of the settings page at `width` x `height`
    /// logical units and `scale`: Folders with a missing GUI font, ROMs and
    /// Video and audio, and a run with a config folder flag, a ROM folder
    /// flag and a backend flag.
    /// With `large` only the ROMs section. The pages draw in the theme `kind`
    /// with the texts of the catalog of `language`.
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
        let render = |harness: &GuiHarness, name: &str| {
            harness.write_screenshot(&format!("settings_{prefix}{name}_{scale}x"));
        };
        let root = sample_root(
            &format!("settings_shots_{prefix}{scale}"),
            "gui-font = font/SarasaUiJ.ttf\n",
        );
        let mut manager = manager_with(&root, &[]);
        manager.roms = sample_statuses(&manager.host);
        let mut harness = GuiHarness::with_viewport(viewport, kind, language.catalog());
        settle(&mut harness, &mut manager);
        harness.state.set_focus(UiId::from(SETTINGS_ID));
        settle(&mut harness, &mut manager);
        if !large {
            render(&harness, "button");
        }
        harness.input.shortcut = Some(Shortcut::F10);
        settle(&mut harness, &mut manager);
        if !large {
            render(&harness, "folders");
        }
        harness.input.navigation = Some(Navigation::NextTab);
        settle(&mut harness, &mut manager);
        render(&harness, "roms");
        if large {
            return;
        }
        harness.input.navigation = Some(Navigation::NextTab);
        settle(&mut harness, &mut manager);
        harness.state.set_focus(field_id(AUDIO_VOLUME_KEY));
        settle(&mut harness, &mut manager);
        render(&harness, "video");

        let root = sample_root(&format!("settings_shots_flag_{scale}"), "");
        let mut manager = manager_with(
            &root,
            &[
                (BACKEND_KEY, "legacy"),
                (CONFIG_FOLDER_KEY, "config"),
                (ROMS_FOLDER_KEY, "/mnt/archive/firmware"),
            ],
        );
        manager.roms = sample_statuses(&manager.host);
        let mut harness = GuiHarness::with_viewport(viewport, kind, language.catalog());
        settle(&mut harness, &mut manager);
        harness.input.shortcut = Some(Shortcut::F10);
        settle(&mut harness, &mut manager);
        render(&harness, "folders_flag");
        harness.input.navigation = Some(Navigation::NextTab);
        settle(&mut harness, &mut manager);
        render(&harness, "roms_flag");
        harness.input.navigation = Some(Navigation::NextTab);
        settle(&mut harness, &mut manager);
        harness.state.set_focus(field_id(BACKEND_KEY));
        settle(&mut harness, &mut manager);
        render(&harness, "video_flag");
    }

    /// Verifies that a steady frame of each section of the settings page, the second pass with an
    /// unchanged state, allocates nothing, fills no text again and keeps the revision of the
    /// editor, with the help text of the focused setting row.
    #[test]
    fn a_steady_frame_of_the_settings_page_allocates_nothing() {
        let root = sample_root(
            "steady_settings",
            "gui-font = font/missing.ttf\ngui-theme = sepia\n",
        );
        let folder = first_rom_folder();
        let mut manager = manager_with(
            &root,
            &[(
                folder.key,
                "/home/user/Emulation/Firmware/The ROM sets of the first compiled system",
            )],
        );
        manager.roms = sample_statuses(&manager.host);
        let machine = find_setting(MACHINE_KEY).expect("machine is a table key");
        let models: Vec<&'static str> = machine
            .choices_for_families(root.system().family)
            .map(|choice| choice.value)
            .take(2)
            .collect();
        let status = manager
            .roms
            .iter_mut()
            .find(|status| status.folder.name == folder.name)
            .expect("the first system has a ROM folder");
        status.check = RomCheck {
            status: RomStatus::Partial,
            folder_exists: true,
            models_without_set: models,
        };
        let mut harness = GuiHarness::minimum();
        settle(&mut harness, &mut manager);
        harness.input.shortcut = Some(Shortcut::F10);
        settle(&mut harness, &mut manager);
        assert!(settings_open(&manager));
        let sections = manager.editor().map(Editor::sections).unwrap_or_default();
        assert_eq!(sections.len(), 3);
        for section in sections {
            assert_eq!(
                manager.editor().map(Editor::current_section),
                Some(*section)
            );
            assert!(
                manager
                    .editor()
                    .is_some_and(|editor| editor.most_note_lines(*section) > 1),
                "the section {section:?} draws no note of several lines"
            );
            harness.frame(|context| manager.show(context));
            let revision = manager.editor().map(Editor::revision);
            let (counts, _) = harness.counted_frame(|context| manager.show(context));
            assert_eq!(
                counts,
                PassCounts::NONE,
                "a steady frame of the section {section:?}"
            );
            assert_eq!(manager.editor().map(Editor::revision), revision);
            assert!(
                shown_help(&mut harness, &mut manager).is_some(),
                "the section {section:?} shows the help of its focused row"
            );
            harness.input.navigation = Some(Navigation::NextTab);
            settle(&mut harness, &mut manager);
        }
    }

    /// Writes the screenshots of the settings page in both themes and in
    /// every language of the repository at the scales 1 and 2, at the minimum window
    /// size and at 1920 x 1080.
    #[test]
    #[ignore = "writes screenshots into the directory of GUI_SHOTS"]
    fn settings_screenshots() {
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
