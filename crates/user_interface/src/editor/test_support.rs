//! Shared helpers of the editor tests: a temporary root folder with a
//! settings file, system defaults and game files.

use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

use machine_factory::config::{
    MachineSelection, Target,
    systems::{System, compiled_systems},
};

use crate::config::{HostLayers, LaunchRequest, Locations, SETTINGS_FILE_NAME};

/// Counter that keeps the temporary folders of the tests apart.
static FOLDER_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A root folder with a settings file. Dropping the value removes it.
pub(crate) struct EditorRoot {
    /// The root folder.
    pub(crate) root: PathBuf,
    /// The system of the games of the test.
    system: &'static System,
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
        let root = std::env::temp_dir().join(format!(
            "neetan_editor_test_{}_{}_{name}",
            std::process::id(),
            FOLDER_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("root is created");
        std::fs::write(root.join(SETTINGS_FILE_NAME), "").expect("settings are written");
        Self { root, system }
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

impl Drop for EditorRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
