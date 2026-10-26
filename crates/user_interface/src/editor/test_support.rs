//! Shared helpers of the editor tests: a temporary root folder with a
//! settings file, system defaults and game files, and a root folder of the
//! settings page with its ROM statuses, and the probes of the help area.

use std::path::{Path, PathBuf};

use common::temporary::TemporaryFolder;
use gui::{Instance, Rect, ui::UiContext};
use machine_factory::{
    config::{
        MachineSelection, Target,
        settings::{MODEL_CHOICES, Setting, find_setting},
        systems::{RomFolder, SYSTEMS, System, compiled_systems, rom_folders},
    },
    rom_status::{RomCheck, RomStatus},
};

use super::{Editor, EditorPass, page::help_area};
use crate::{
    config::{HostLayers, LaunchRequest, Locations, SETTINGS_FILE_NAME},
    gui_test_support::GuiHarness,
    rom_folders::{RomFolderStatus, rom_folder_path},
    texts::setting_help,
};

/// The window above the status line in the tests.
pub(crate) const SCREEN: Rect = Rect::new(0.0, 0.0, 960.0, 572.0);

/// Draws passes of `editor` until they settle and returns the answers of the passes.
pub(crate) fn show(harness: &mut GuiHarness, editor: &mut Editor) -> EditorPass {
    let mut answer = EditorPass::default();
    harness.settle(|context| {
        let pass = editor.show(context, SCREEN);
        answer.closed |= pass.closed;
        answer.saved = answer.saved.clone().or(pass.saved);
        answer.save_error = answer.save_error.clone().or(pass.save_error);
    });
    answer
}

/// The glyph indices of the last pass inside the help area of the page in
/// [`SCREEN`], in the order of the draw.
pub(crate) fn help_glyphs(harness: &GuiHarness) -> Vec<u32> {
    help_glyphs_in(harness, SCREEN)
}

/// The glyph indices of the last pass inside the help area of the page in
/// `screen`, in the order of the draw.
pub(crate) fn help_glyphs_in(harness: &GuiHarness, screen: Rect) -> Vec<u32> {
    let area = help_area(&harness.theme, screen);
    harness
        .scene
        .instances()
        .iter()
        .filter(|instance| {
            instance.kind[0] == Instance::KIND_GLYPH
                && instance.rect[1] >= area.y - 1.0
                && instance.rect[1] + instance.rect[3] <= area.bottom() + 1.0
        })
        .map(|instance| instance.kind[1])
        .collect()
}

/// The glyph indices of the help text of `setting` shaped in the small
/// text of `context`. The glyphs without an outline, the spaces for
/// example, draw nothing and leave the list.
pub(crate) fn shaped_help(context: &mut UiContext, setting: &Setting) -> Vec<u32> {
    let font = context.theme.small().font;
    context
        .fonts
        .shape(font, setting_help(context.catalog, setting))
        .glyphs
        .iter()
        .filter_map(|glyph| glyph.shape.map(|shape| shape.index))
        .collect()
}

/// The key of the setting of the help area of `subject`, read in one more
/// pass from the rows and the widgets of the last pass. `shown` gives the
/// setting of the hovered or the focused row and `draw` draws `subject`
/// again in the pass. The probe asserts that the help area of the page in
/// `screen` draws the glyphs of the shaped help text of that setting in
/// their order, or no glyph without a setting.
pub(crate) fn shown_help_of<S>(
    harness: &mut GuiHarness,
    subject: &mut S,
    screen: Rect,
    shown: impl FnOnce(&S, &UiContext) -> Option<&'static Setting>,
    draw: impl FnOnce(&mut S, &mut UiContext),
) -> Option<&'static str> {
    let (key, expected) = harness.frame(|context| {
        let setting = shown(subject, context);
        let expected = setting.map_or_else(Vec::new, |setting| shaped_help(context, setting));
        draw(subject, context);
        (setting.map(|setting| setting.key), expected)
    });
    assert_eq!(
        help_glyphs_in(harness, screen),
        expected,
        "the help area of {key:?}"
    );
    key
}

/// The key of the setting of the help area of `editor`, read by
/// [`shown_help_of`] in one more pass of `editor` in [`SCREEN`].
pub(crate) fn shown_help(harness: &mut GuiHarness, editor: &mut Editor) -> Option<&'static str> {
    shown_help_in(harness, editor, SCREEN)
}

/// The key of the setting of the help area of `editor`, read by
/// [`shown_help_of`] in one more pass of `editor` in `screen`.
pub(crate) fn shown_help_in(
    harness: &mut GuiHarness,
    editor: &mut Editor,
    screen: Rect,
) -> Option<&'static str> {
    shown_help_of(
        harness,
        editor,
        screen,
        |editor, context| editor.help_setting(context),
        |editor, context| {
            editor.show(context, screen);
        },
    )
}

/// The system `name` of the systems table.
pub(crate) fn system_named(name: &str) -> &'static System {
    SYSTEMS
        .iter()
        .find(|system| system.name == name)
        .expect("the systems table holds the system")
}

/// The index of the field `key` of `editor`.
pub(crate) fn index_of(editor: &Editor, key: &str) -> usize {
    editor
        .fields
        .iter()
        .position(|field| field.key() == key)
        .expect("the form has the key")
}

/// A model of the family of `root` that reads `fdd1`.
pub(crate) fn fdd1_model(root: &EditorRoot) -> &'static str {
    let fdd1 = find_setting("fdd1").expect("fdd1 is a table key");
    MODEL_CHOICES
        .iter()
        .map(|choice| choice.value)
        .find(|model| fdd1.is_read_by(root.target(), model))
        .expect("a model of the family reads fdd1")
}

/// A root folder with a settings file. Dropping the value removes it.
pub(crate) struct EditorRoot {
    /// The root folder.
    pub(crate) root: PathBuf,
    /// The system of the games of the test.
    system: &'static System,
    /// The temporary folder that holds the root.
    _folder: TemporaryFolder,
}

impl EditorRoot {
    /// Creates a root folder with an empty settings file for the first compiled system.
    pub(crate) fn new(name: &str) -> Self {
        let system = compiled_systems().next().expect("one system is compiled");
        Self::create(name, system)
    }

    /// Creates a root folder with an empty settings file for the system
    /// with the folder name `system`.
    pub(crate) fn with_system(name: &str, system: &str) -> Self {
        let system = compiled_systems()
            .find(|candidate| candidate.name == system)
            .expect("the system is compiled");
        Self::create(name, system)
    }

    /// Creates the root folder `name` for `system`.
    fn create(name: &str, system: &'static System) -> Self {
        let folder = TemporaryFolder::new(&format!("editor_test_{name}"));
        let root = folder.path().to_path_buf();
        std::fs::write(root.join(SETTINGS_FILE_NAME), "").expect("settings are written");
        Self {
            root,
            system,
            _folder: folder,
        }
    }

    /// The system of the games of the test.
    pub(crate) fn system(&self) -> &'static System {
        self.system
    }

    /// The family of the system of the test.
    pub(crate) fn target(&self) -> Target {
        MachineSelection::of_system(self.system)
            .expect("compiled system")
            .target()
    }

    /// Writes `contents` into the file `relative` below the root and returns its path.
    pub(crate) fn write(&self, relative: &str, contents: &str) -> PathBuf {
        let path = self.root.join(relative);
        std::fs::create_dir_all(path.parent().expect("a file lies in a folder"))
            .expect("folder is created");
        std::fs::write(&path, contents).expect("file is written");
        path
    }

    /// Writes the game file `stem` of the system with `contents` and returns its path.
    pub(crate) fn game(&self, stem: &str, contents: &str) -> PathBuf {
        self.write(
            &format!("config/{}/{stem}.conf", self.system.name),
            contents,
        )
    }

    /// The path of `relative` below the root.
    pub(crate) fn path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    /// The host layers of the root: its settings file without flags.
    pub(crate) fn host(&self) -> HostLayers {
        HostLayers::read(&LaunchRequest::default(), &self.locations()).expect("the host reads")
    }

    /// The locations of the settings discovery with the root as the current directory.
    pub(crate) fn locations(&self) -> Locations {
        Locations {
            current_directory: self.root.clone(),
            executable_directory: None,
            pref_path: None,
        }
    }
}

/// A root folder of the settings page as the current directory, with or
/// without a settings file. Dropping the value removes it.
pub(crate) struct SettingsRoot {
    /// The temporary folder of the root.
    folder: TemporaryFolder,
}

impl SettingsRoot {
    /// A root folder with the settings file `contents`, or without a
    /// settings file for `None`.
    pub(crate) fn new(name: &str, contents: Option<&str>) -> Self {
        let folder = TemporaryFolder::new(&format!("settings_page_test_{name}"));
        if let Some(contents) = contents {
            std::fs::write(folder.path().join(SETTINGS_FILE_NAME), contents)
                .expect("settings are written");
        }
        Self { folder }
    }

    /// The root folder.
    pub(crate) fn root(&self) -> &Path {
        self.folder.path()
    }

    /// The settings file of the root.
    pub(crate) fn settings_file(&self) -> PathBuf {
        self.root().join(SETTINGS_FILE_NAME)
    }

    /// The host layers of the root with the setting flags `flags`.
    pub(crate) fn host(&self, flags: &[(&str, &str)]) -> HostLayers {
        let request = LaunchRequest {
            flags: flags
                .iter()
                .map(|(key, value)| (find_setting(key).expect("a table key"), (*value).to_owned()))
                .collect(),
            ..LaunchRequest::default()
        };
        let locations = Locations {
            current_directory: self.root().to_path_buf(),
            executable_directory: None,
            pref_path: None,
        };
        HostLayers::read(&request, &locations).expect("the host reads")
    }
}

/// The ROM statuses of `host` with the status `status` for every folder.
pub(crate) fn statuses_with(host: &HostLayers, status: RomStatus) -> Vec<RomFolderStatus> {
    rom_folders()
        .map(|folder| RomFolderStatus {
            folder,
            location: rom_folder_path(host, &folder),
            check: RomCheck {
                status,
                folder_exists: true,
                models_without_set: Vec::new(),
            },
        })
        .collect()
}

/// The settings page of `host` with the ROM statuses `statuses`.
pub(crate) fn settings_page(host: &HostLayers, statuses: &[RomFolderStatus]) -> Editor {
    Editor::settings(host, statuses).expect("the page opens")
}

/// The ROM folder of the first compiled system.
pub(crate) fn first_rom_folder() -> RomFolder {
    compiled_systems()
        .next()
        .expect("one system is compiled")
        .rom_folder()
}
