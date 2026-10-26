//! The layered configuration of a launch.
//!
//! A launch reads its layers first: the settings file, the game files and the
//! command line flags. The family comes from the first source of: the
//! `machine` of the command line or of a game file, the system of the request,
//! the system folder of the last game file, the `machine` of the settings file
//! and the built-in default. A `machine` of another family in the settings
//! file or the system defaults does not count. The system defaults of the
//! family join as the third layer. The machine applies first, the remaining
//! lines follow in layer order. The built configuration refers only to images
//! that exist.

use std::path::{Path, PathBuf};

use common::{info, warn};
use machine_factory::config::{
    DEFAULT_AUDIO_VOLUME, DEFAULT_BACKEND, DEFAULT_CONFIG_FOLDER, DEFAULT_GUI_FONT,
    DEFAULT_GUI_LANGUAGE, DEFAULT_GUI_THEME, DEFAULT_ROMS_FOLDER, EmulatorConfig, GuiLanguage,
    MachineSelection, Target, parse_model,
    settings::{
        AUDIO_VOLUME_KEY, BACKEND_KEY, CONFIG_FOLDER_KEY, ExplicitSettings, GUI_FONT_KEY,
        GUI_LANGUAGE_KEY, GUI_THEME_KEY, MACHINE_KEY, ROMS_FOLDER_KEY, Setting,
        apply_derived_defaults,
    },
    systems::{System, apply_rom_folder_defaults, compiled_systems, system_of},
};

use super::{
    diagnostic::{Diagnostic, DiagnosticKind, Severity},
    discovery::{
        LANGUAGE_FOLDER, Locations, SettingsLocation, discover_settings, resolve_path,
        resource_folder,
    },
    document::{ConfigDocument, DocumentError},
    keymap::parse_key_binding,
    launch_error::LaunchError,
    reading::{
        FileReading, Layer, LayerKind, LayerLine, is_foreign_machine, read_document,
        read_game_document,
    },
};
use crate::input::KeyOverrides;

/// File extension of the system defaults files and the game files.
pub const CONFIG_FILE_EXTENSION: &str = "conf";

/// Returns the built-in value of the host key `key` as the settings file
/// writes it. A ROM key has none: its folder derives from the ROM folder.
#[must_use]
pub(crate) fn host_default(key: &str) -> Option<String> {
    match key {
        CONFIG_FOLDER_KEY => Some(DEFAULT_CONFIG_FOLDER.to_owned()),
        ROMS_FOLDER_KEY => Some(DEFAULT_ROMS_FOLDER.to_owned()),
        GUI_FONT_KEY => Some(DEFAULT_GUI_FONT.to_owned()),
        BACKEND_KEY => Some(DEFAULT_BACKEND.to_string()),
        GUI_THEME_KEY => Some(DEFAULT_GUI_THEME.to_string()),
        GUI_LANGUAGE_KEY => Some(DEFAULT_GUI_LANGUAGE.to_string()),
        AUDIO_VOLUME_KEY => Some(format!("{DEFAULT_AUDIO_VOLUME:?}")),
        _ => None,
    }
}

/// Returns the absolute folder of the folder key value `value` below
/// `root`, or the built-in folder `default` for an unset or empty value.
#[must_use]
pub(crate) fn folder_location(root: &Path, value: Option<&str>, default: &str) -> PathBuf {
    let value = value.filter(|value| !value.is_empty()).unwrap_or(default);
    resolve_path(root, Path::new(value))
}

/// The command line inputs of a launch.
#[derive(Debug, Clone, Default)]
pub struct LaunchRequest {
    /// The `--settings` file.
    pub settings: Option<PathBuf>,
    /// The `--config` game files in command line order.
    pub game_files: Vec<PathBuf>,
    /// The setting flags in command line order. A path value is absolute.
    pub flags: Vec<(&'static Setting, String)>,
    /// The system of the launch when neither the command line nor a game file sets `machine`.
    ///
    /// It outranks the `machine` of the settings file. `None` takes the system
    /// whose folder below the config folder holds the last game file.
    pub system: Option<&'static System>,
}

/// Fails with the first error of `diagnostics`, the findings about the file
/// at `source`, and logs each warning before it.
fn check_diagnostics(diagnostics: &[Diagnostic], source: &str) -> Result<(), LaunchError> {
    for diagnostic in diagnostics {
        match diagnostic.severity {
            Severity::Error => {
                return Err(LaunchError::Diagnostic {
                    source: source.to_owned(),
                    diagnostic: diagnostic.clone(),
                });
            }
            Severity::Warning => warn!("{}", diagnostic.located(source)),
        }
    }
    Ok(())
}

/// Reads a configuration file into a layer.
///
/// Relative path values resolve against `root`. A game file in the folder of
/// `folder_system` gets its machine checked against that system. An error in
/// a game file fails the read. Every other finding ends with a warning, and
/// its line leaves the layer. The findings come back with the layer.
fn read_layer(
    kind: LayerKind,
    file: &Path,
    root: &Path,
    folder_system: Option<&'static System>,
) -> Result<(Layer, Vec<Diagnostic>), LaunchError> {
    let document = ConfigDocument::load(file).map_err(|error| LaunchError::ReadFailed {
        path: file.to_path_buf(),
        error,
    })?;
    let source = file.display().to_string();
    let reading = match kind {
        LayerKind::GameFile => read_game_document(&document, root, folder_system),
        LayerKind::SettingsFile | LayerKind::SystemDefaults | LayerKind::CommandLine => {
            read_document(kind, &document, root)
        }
    };
    check_diagnostics(&reading.diagnostics, &source)?;
    let layer = Layer::from_reading(kind, source, &reading);
    Ok((layer, reading.diagnostics))
}

/// The source of the machine model of a launch.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ModelSource {
    /// The `--machine` flag.
    CommandLine,
    /// The `machine` line of a game file.
    GameFile,
    /// The `machine` line of the system defaults file.
    SystemDefaults,
    /// The `machine` line of the settings file.
    SettingsFile,
    /// The default model of the system.
    BuiltIn,
}

/// Returns the family of a launch.
///
/// The family comes from the first source of: the `machine` of the command
/// line or of the last game file that sets one, the system of the launch and
/// the `machine` of the settings file. A launch without any of them is an
/// error.
pub(crate) fn launch_target(
    command_line: &Layer,
    games: &[Layer],
    launch_system: Option<&'static System>,
    settings: Option<&Layer>,
) -> Result<Target, LaunchError> {
    let launch_machine = std::iter::once(command_line)
        .chain(games.iter().rev())
        .find_map(|layer| layer.last_value(MACHINE_KEY));
    let settings_machine = settings.and_then(|layer| layer.last_value(MACHINE_KEY));
    let family_model = launch_machine
        .or(launch_system.map(|system| system.default_model))
        .or(settings_machine);
    match family_model {
        Some(model) => parse_model(model)
            .map(MachineSelection::target)
            .map_err(LaunchError::Machine),
        None => Err(LaunchError::NoMachine),
    }
}

/// Returns the model of a launch of the family `target` and its source.
///
/// The model is the last `machine` of the layers in the order settings file,
/// system defaults, game files and command line. A `machine` of another
/// family in the settings file or the system defaults does not count. Without
/// a `machine` the model is the default model of the system.
pub(crate) fn launch_model<'a>(
    target: Target,
    settings: Option<&'a Layer>,
    system_defaults: Option<&'a Layer>,
    games: &'a [Layer],
    command_line: &'a Layer,
) -> (&'a str, ModelSource) {
    let same_family = |layer: &'a Layer| {
        layer.lines.iter().rev().find_map(|line| match line {
            LayerLine::Setting(setting, value)
                if setting.key == MACHINE_KEY && !is_foreign_machine(value, target) =>
            {
                Some(value.as_str())
            }
            LayerLine::Setting(..) | LayerLine::KeyBinding(..) => None,
        })
    };
    command_line
        .last_value(MACHINE_KEY)
        .map(|model| (model, ModelSource::CommandLine))
        .or_else(|| {
            games
                .iter()
                .rev()
                .find_map(|layer| layer.last_value(MACHINE_KEY))
                .map(|model| (model, ModelSource::GameFile))
        })
        .or_else(|| {
            system_defaults
                .and_then(same_family)
                .map(|model| (model, ModelSource::SystemDefaults))
        })
        .or_else(|| {
            settings
                .and_then(same_family)
                .map(|model| (model, ModelSource::SettingsFile))
        })
        .unwrap_or((system_of(target).default_model, ModelSource::BuiltIn))
}

/// The files of the user interface that a launch searches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuiResources {
    /// The paths searched for the font file in search order.
    pub font_paths: Vec<PathBuf>,
    /// The folders searched for the language files in search order.
    pub language_folders: Vec<PathBuf>,
}

/// The host layers of a launch: the settings file and the command line flags.
#[derive(Debug, Clone)]
pub struct HostLayers {
    /// The settings file and the root folder.
    location: SettingsLocation,
    /// The layer of the settings file, when the discovery found one.
    settings: Option<Layer>,
    /// The findings about the settings file.
    settings_diagnostics: Vec<Diagnostic>,
    /// The layer of the command line flags.
    command_line: Layer,
    /// The resource folder of the installation, when the folder of the
    /// executable is known.
    resource_folder: Option<PathBuf>,
}

impl HostLayers {
    /// Finds and reads the settings file of `request` and takes its command line flags.
    pub fn read(request: &LaunchRequest, locations: &Locations) -> Result<Self, LaunchError> {
        let location = discover_settings(request.settings.as_deref(), locations)?;
        match &location.file {
            Some(file) => info!("Settings file: {}", file.display()),
            None => info!("No settings file found"),
        }
        info!("Root folder: {}", location.root.display());
        let (settings, settings_diagnostics) = match location.file.as_deref() {
            Some(file) => {
                let (layer, diagnostics) =
                    read_layer(LayerKind::SettingsFile, file, &location.root, None)?;
                (Some(layer), diagnostics)
            }
            None => (None, Vec::new()),
        };
        Ok(Self {
            location,
            settings,
            settings_diagnostics,
            command_line: Layer::command_line(&request.flags),
            resource_folder: locations
                .executable_directory
                .as_deref()
                .map(resource_folder),
        })
    }

    /// Returns the settings file and the root folder.
    #[must_use]
    pub fn location(&self) -> &SettingsLocation {
        &self.location
    }

    /// Returns the root folder.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.location.root
    }

    /// Returns the findings about the settings file.
    #[must_use]
    pub fn settings_diagnostics(&self) -> &[Diagnostic] {
        &self.settings_diagnostics
    }

    /// Returns the last value of `key` on the command line, or else in the
    /// settings file. It reads the keys of every scope.
    #[must_use]
    pub fn host_value(&self, key: &str) -> Option<&str> {
        std::iter::once(&self.command_line)
            .chain(self.settings.as_ref())
            .find_map(|layer| layer.last_value(key))
    }

    /// Returns the config folder as an absolute path. An empty value
    /// takes the built-in folder.
    #[must_use]
    pub fn config_folder(&self) -> PathBuf {
        folder_location(
            self.root(),
            self.host_value(CONFIG_FOLDER_KEY),
            DEFAULT_CONFIG_FOLDER,
        )
    }

    /// Returns the ROM folder as an absolute path. An empty value takes the
    /// built-in folder.
    #[must_use]
    pub fn roms_folder(&self) -> PathBuf {
        folder_location(
            self.root(),
            self.host_value(ROMS_FOLDER_KEY),
            DEFAULT_ROMS_FOLDER,
        )
    }

    /// Returns the absolute paths searched for the font file of the user
    /// interface in search order.
    ///
    /// A value of the settings file lies below the root and a flag value
    /// below the current directory. The built-in default lies below the root
    /// and then below the resource folder of the installation, when the
    /// folder of the executable is known.
    #[must_use]
    pub fn gui_font_paths(&self) -> Vec<PathBuf> {
        match self.host_value(GUI_FONT_KEY) {
            Some(value) => vec![resolve_path(self.root(), Path::new(value))],
            None => {
                let mut paths: Vec<PathBuf> = std::iter::once(self.root())
                    .chain(self.resource_folder.as_deref())
                    .map(|folder| resolve_path(folder, Path::new(DEFAULT_GUI_FONT)))
                    .collect();
                paths.dedup();
                paths
            }
        }
    }

    /// Returns the absolute folders searched for the language files of the
    /// user interface in search order: the folder `language` below the root
    /// and then below the resource folder of the installation, when the
    /// folder of the executable is known.
    #[must_use]
    pub fn language_folders(&self) -> Vec<PathBuf> {
        let mut folders: Vec<PathBuf> = std::iter::once(self.root())
            .chain(self.resource_folder.as_deref())
            .map(|folder| resolve_path(folder, Path::new(LANGUAGE_FOLDER)))
            .collect();
        folders.dedup();
        folders
    }

    /// Returns the language of the user interface of the command line or the
    /// settings file. An unset key gives the built-in default.
    #[must_use]
    pub fn gui_language(&self) -> GuiLanguage {
        self.host_value(GUI_LANGUAGE_KEY)
            .and_then(|value| value.parse().ok())
            .unwrap_or(DEFAULT_GUI_LANGUAGE)
    }

    /// Returns the font paths and the language folders of the user interface.
    #[must_use]
    pub fn gui_resources(&self) -> GuiResources {
        GuiResources {
            font_paths: self.gui_font_paths(),
            language_folders: self.language_folders(),
        }
    }

    /// Returns the font file of the user interface: the first path of
    /// [`Self::gui_font_paths`] that exists, or else the first path.
    #[must_use]
    pub fn gui_font(&self) -> PathBuf {
        let paths = self.gui_font_paths();
        paths
            .iter()
            .find(|path| path.is_file())
            .unwrap_or(&paths[0])
            .clone()
    }

    /// Returns the layer of the settings file.
    pub(crate) fn settings_layer(&self) -> Option<&Layer> {
        self.settings.as_ref()
    }

    /// Returns the layer of the command line flags.
    pub(crate) fn command_line_layer(&self) -> &Layer {
        &self.command_line
    }
}

/// Returns the system whose folder below `config_folder` holds `file`.
pub(crate) fn folder_system(file: &Path, config_folder: &Path) -> Option<&'static System> {
    let folder = file.parent()?;
    compiled_systems().find(|system| folder == config_folder.join(system.name))
}

/// Builds the layered configuration of a launch.
///
/// The layers apply in the order built-in defaults, settings file, system
/// defaults, game files and command line flags. A later layer overrides an
/// earlier one. The first value of a path list in a layer replaces the list of
/// the earlier layers. A game file with an error fails the launch.
pub fn build_configuration(
    request: &LaunchRequest,
    locations: &Locations,
) -> Result<(EmulatorConfig, KeyOverrides), LaunchError> {
    build_host_configuration(&HostLayers::read(request, locations)?, request)
}

/// Builds the layered configuration of `request` on the read host layers `host`.
pub(crate) fn build_host_configuration(
    host: &HostLayers,
    request: &LaunchRequest,
) -> Result<(EmulatorConfig, KeyOverrides), LaunchError> {
    let root = host.root();
    let config_folder = host.config_folder();
    let game_layers = request
        .game_files
        .iter()
        .map(|file| {
            read_layer(
                LayerKind::GameFile,
                file,
                root,
                folder_system(file, &config_folder),
            )
            .map(|(layer, _)| layer)
        })
        .collect::<Result<Vec<Layer>, LaunchError>>()?;

    let launch_system = request
        .system
        .or_else(|| game_folder_system(&request.game_files, &config_folder));
    let target = launch_target(
        host.command_line_layer(),
        &game_layers,
        launch_system,
        host.settings_layer(),
    )?;

    let system_file = system_defaults_file(&config_folder, target);
    let system_layer = match read_system_defaults(&system_file, target, root) {
        Ok(None) => None,
        Ok(Some(defaults)) => {
            info!("System defaults: {}", system_file.display());
            check_diagnostics(&defaults.reading.diagnostics, &defaults.layer.source)?;
            Some(defaults.layer)
        }
        Err(error) => {
            return Err(LaunchError::ReadFailed {
                path: system_file,
                error,
            });
        }
    };

    let (machine, _) = launch_model(
        target,
        host.settings_layer(),
        system_layer.as_ref(),
        &game_layers,
        host.command_line_layer(),
    );
    let model = parse_model(machine).map_err(LaunchError::Machine)?;
    let layers = host
        .settings_layer()
        .into_iter()
        .chain(system_layer.as_ref())
        .chain(&game_layers)
        .chain(std::iter::once(host.command_line_layer()));
    let (mut config, explicit, key_map) = apply_layers(model, layers)?;

    config.config_folder = resolve_path(root, &config.config_folder);
    config.roms_folder = resolve_path(root, &config.roms_folder);
    config.gui_font = host.gui_font();
    let roms_folder = config.roms_folder.clone();
    apply_rom_folder_defaults(&mut config, &roms_folder);
    apply_derived_defaults(&mut config, explicit);
    validate_images(&config)?;
    Ok((config, key_map))
}

/// Returns the configuration of `model` with the lines of `layers` applied in order, except
/// `machine`, together with the settings that the layers set explicitly and the key bindings.
///
/// The defaults that derive from other settings are not applied yet, see
/// [`apply_derived_defaults`].
pub(crate) fn apply_layers<'a>(
    model: MachineSelection,
    layers: impl IntoIterator<Item = &'a Layer>,
) -> Result<(EmulatorConfig, ExplicitSettings, KeyOverrides), LaunchError> {
    let mut config = EmulatorConfig::for_model(model);
    let mut explicit = ExplicitSettings::default();
    let mut key_map = KeyOverrides::new();
    for layer in layers {
        apply_layer(&mut config, &mut explicit, &mut key_map, layer)?;
    }
    Ok((config, explicit, key_map))
}

/// The system defaults file of one family as it reads.
#[derive(Debug, Clone)]
pub(crate) struct SystemDefaultsFile {
    /// The accepted lines with their line numbers and the findings about the file.
    pub(crate) reading: FileReading,
    /// The layer of the accepted lines.
    pub(crate) layer: Layer,
}

/// Reads the system defaults file `file` of the family `target`. Relative path values resolve
/// against `root`. A `machine` of another family gets a warning, and the launch ignores it. A
/// file that does not exist gives `None`.
pub(crate) fn read_system_defaults(
    file: &Path,
    target: Target,
    root: &Path,
) -> Result<Option<SystemDefaultsFile>, DocumentError> {
    if !file.is_file() {
        return Ok(None);
    }
    let document = ConfigDocument::load(file)?;
    let mut reading = read_document(LayerKind::SystemDefaults, &document, root);
    let foreign: Vec<Diagnostic> = reading
        .lines
        .iter()
        .filter_map(|(line, content)| match content {
            LayerLine::Setting(setting, value)
                if setting.key == MACHINE_KEY && is_foreign_machine(value, target) =>
            {
                Some(Diagnostic::warning(
                    *line,
                    DiagnosticKind::ForeignMachineIgnored(value.clone()),
                ))
            }
            LayerLine::Setting(..) | LayerLine::KeyBinding(..) => None,
        })
        .collect();
    reading.diagnostics.extend(foreign);
    reading
        .diagnostics
        .sort_by_key(|diagnostic| diagnostic.line);
    let layer = Layer::from_reading(
        LayerKind::SystemDefaults,
        file.display().to_string(),
        &reading,
    );
    Ok(Some(SystemDefaultsFile { reading, layer }))
}

/// Returns the system defaults file of the family `target` below `config_folder`.
pub(crate) fn system_defaults_file(config_folder: &Path, target: Target) -> PathBuf {
    config_folder
        .join(system_of(target).name)
        .with_extension(CONFIG_FILE_EXTENSION)
}

/// Returns the system whose folder below `config_folder` holds the last game file of `game_files`.
fn game_folder_system(game_files: &[PathBuf], config_folder: &Path) -> Option<&'static System> {
    game_files
        .iter()
        .rev()
        .find_map(|file| folder_system(file, config_folder))
}

/// Returns the error of the image at `path` of the drive `key` that does not exist.
fn missing_image(key: &'static str, path: &Path) -> LaunchError {
    LaunchError::MissingImage {
        key,
        path: path.to_path_buf(),
    }
}

/// Checks that every disk and disc image of `config` exists.
fn validate_images(config: &EmulatorConfig) -> Result<(), LaunchError> {
    for (key, paths) in [
        ("fdd1", &config.fdd1),
        ("fdd2", &config.fdd2),
        ("cdrom", &config.cdrom),
    ] {
        if let Some(path) = paths.iter().find(|path| !path.exists()) {
            return Err(missing_image(key, path));
        }
    }
    for (key, path) in [("hdd1", &config.hdd1), ("hdd2", &config.hdd2)] {
        if let Some(path) = path
            && !path.exists()
        {
            return Err(missing_image(key, path));
        }
    }
    Ok(())
}

/// Applies the lines of one layer except `machine`.
///
/// An invalid value ends with a warning in a file and with an error on the
/// command line.
fn apply_layer(
    config: &mut EmulatorConfig,
    explicit: &mut ExplicitSettings,
    key_map: &mut KeyOverrides,
    layer: &Layer,
) -> Result<(), LaunchError> {
    let mut cleared_lists: Vec<&str> = Vec::new();
    for line in &layer.lines {
        match line {
            LayerLine::KeyBinding(host_name, native_name) => {
                match parse_key_binding(config.target, host_name, native_name) {
                    Some((host, code)) => key_map.set(host, code),
                    None => warn!(
                        "{}: {}",
                        layer.source,
                        DiagnosticKind::InvalidKeyBinding {
                            host: host_name.clone(),
                            native: native_name.clone(),
                        }
                    ),
                }
            }
            LayerLine::Setting(setting, value) => {
                if setting.key == MACHINE_KEY {
                    continue;
                }
                if let Some(clear) = setting.clear
                    && !cleared_lists.contains(&setting.key)
                {
                    clear(config);
                    cleared_lists.push(setting.key);
                }
                if let Err(error) = explicit.apply(config, setting, value) {
                    match layer.kind {
                        LayerKind::CommandLine => {
                            return Err(LaunchError::InvalidFlag {
                                key: setting.key,
                                error,
                            });
                        }
                        LayerKind::SettingsFile
                        | LayerKind::SystemDefaults
                        | LayerKind::GameFile => {
                            warn!("{}: invalid {}: {error}", layer.source, setting.key);
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    #[cfg(feature = "pc98")]
    use common::MachineModel;
    #[cfg(feature = "pc88")]
    use common::{CpuMode, MemoryWaitSwitch};
    use machine_factory::config::settings::find_setting;
    #[cfg(feature = "msx")]
    use machine_msx::MsxModel;

    use super::*;
    use crate::{
        config::{
            test_support::{
                TestLocations, first_model, first_model_config, touch, with_first_model,
                with_machine, write_file,
            },
            *,
        },
        gui_host::GuiHost,
        texts::LanguageCatalog,
    };

    /// Verifies that the file parser skips unknown keys and invalid values.
    #[test]
    fn config_file_skips_unknown_keys_and_invalid_values() {
        let test = TestLocations::new("invalid");
        let (config, _) = test.parse_game(
            "invalid.conf",
            &format!(
                "machine = {}\nfdd3 = disk.d88\ncpu-mode = turbo\ncrt = maybe\nscaling = nearest\n",
                first_model()
            ),
        );
        let (expected, _) = test.parse(&with_first_model(&["--scaling", "nearest"]));
        assert_eq!(config, expected);
    }

    /// Verifies that a game file with an unknown machine fails its launch.
    #[test]
    fn game_file_with_an_unknown_machine_fails() {
        let test = TestLocations::new("unknown_machine");
        let path = test.file("unknown.conf", "scaling = nearest\nmachine = FOOBAR\n");
        let error = test.error(&["--config", &path.to_string_lossy()]);
        assert!(
            error.ends_with("unknown.conf line 2: unknown machine FOOBAR"),
            "{error}"
        );
    }

    /// Verifies the empty values: a path list clears, a single path and a ROM key are unset, a folder key is invalid.
    #[test]
    fn empty_values_clear_unset_or_fail() {
        let test = TestLocations::new("empty");
        let root = test.current().to_path_buf();
        std::fs::create_dir_all(root.join("roms").join("pc98")).expect("folder is created");
        touch(&[&root.join("base.d88"), &root.join("disk.hdi")]);
        write_file(
            &root.join(SETTINGS_FILE_NAME),
            "fdd1 = base.d88\nhdd1 = disk.hdi\npc98-roms = /explicit/pc98\n",
        );
        let config = test.build(&with_machine(LaunchRequest::default()));
        assert_eq!(config.fdd1, vec![root.join("base.d88")]);
        assert_eq!(config.hdd1, Some(root.join("disk.hdi")));
        assert_eq!(config.pc98_roms, Some(PathBuf::from("/explicit/pc98")));

        let game = root.join("game.conf");
        write_file(&game, "fdd1 =\nhdd1 =\n");
        let config = test.build(&with_machine(LaunchRequest {
            game_files: vec![game],
            ..LaunchRequest::default()
        }));
        assert!(config.fdd1.is_empty());
        assert_eq!(config.hdd1, None);

        write_file(
            &root.join(SETTINGS_FILE_NAME),
            &format!("pc98-roms =\n{CONFIG_FOLDER_KEY} =\nroms-folder =\n"),
        );
        let config = test.build(&with_machine(LaunchRequest::default()));
        assert_eq!(config.pc98_roms, Some(root.join("roms").join("pc98")));
        assert_eq!(config.config_folder, root.join("config"));
        assert_eq!(config.roms_folder, root.join("roms"));
        let host =
            HostLayers::read(&LaunchRequest::default(), &test.locations).expect("settings read");
        assert_eq!(host.config_folder(), config.config_folder);
        assert_eq!(host.roms_folder(), config.roms_folder);
    }

    /// Verifies that the built-in value of every host key matches the built-in configuration as the settings file writes it.
    #[test]
    fn host_defaults_match_the_built_in_configuration() {
        use machine_factory::config::settings::{SETTINGS, Scope};

        let config = first_model_config();
        for setting in SETTINGS
            .iter()
            .filter(|setting| setting.scope == Scope::Host)
        {
            assert_eq!(
                host_default(setting.key).into_iter().collect::<Vec<_>>(),
                (setting.format)(&config),
                "{}",
                setting.key
            );
        }
    }

    /// Verifies that relative paths of files resolve against the root and those of flags against the current directory.
    #[test]
    fn relative_paths_resolve_against_the_root_and_the_current_directory() {
        let test = TestLocations::new("paths");
        let current = test.current().to_path_buf();
        let root = current.join("root");
        let settings = root.join("settings.conf");
        write_file(&settings, "printer = out/printer.txt\n");
        let game = current.join("games").join("game.conf");
        write_file(&game, "fdd1 = disks/a.d88\nhdd1 = ./disks/c.hdi\n");
        touch(&[
            &root.join("disks/a.d88"),
            &root.join("disks/c.hdi"),
            &current.join("disks/b.d88"),
        ]);
        let fdd2 = find_setting("fdd2").expect("fdd2 is a table key");

        let config = test.build(&with_machine(LaunchRequest {
            settings: Some(settings),
            game_files: vec![game],
            flags: vec![(fdd2, resolve_value(fdd2, &current, "disks/b.d88"))],
            system: None,
        }));
        assert_eq!(config.printer, Some(root.join("out/printer.txt")));
        assert_eq!(config.fdd1, vec![root.join("disks/a.d88")]);
        assert_eq!(config.hdd1, Some(root.join("disks/c.hdi")));
        assert_eq!(config.fdd2, vec![current.join("disks/b.d88")]);
        assert_eq!(config.config_folder, root.join("config"));
        assert_eq!(config.roms_folder, root.join("roms"));

        let (config, _) = test.parse(&with_first_model(&[
            "--roms-folder",
            "my-roms",
            "--x1-roms",
            "x1",
        ]));
        assert_eq!(config.roms_folder, current.join("my-roms"));
        assert_eq!(config.x1_roms, Some(current.join("x1")));
    }

    #[cfg(feature = "pc98")]
    /// Verifies the layer order: settings file, system defaults, game file and flags.
    #[test]
    fn later_layers_override_earlier_layers() {
        let test = TestLocations::new("layers");
        let root = test.current().to_path_buf();
        write_file(
            &root.join(SETTINGS_FILE_NAME),
            "scaling = nearest\naspect-mode = 1:1\ncrt = off\nwindow-mode = fullscreen\n",
        );
        write_file(
            &root.join("config").join("pc98.conf"),
            "scaling = bilinear\naspect-mode = 4:3\ncrt = on\n",
        );
        let game = root.join("config").join("pc98").join("game.conf");
        write_file(
            &game,
            "machine = PC9801VX\nscaling = pixelart\naspect-mode = 1:1\n",
        );
        let game_text = game.to_string_lossy().into_owned();

        let (config, _) = test.parse(&["--config", &game_text]);
        assert_eq!(config.window_mode, WindowMode::Fullscreen);
        assert!(config.crt);
        assert_eq!(config.scaling, ScalingMode::Pixelart);
        assert_eq!(config.aspect_mode, AspectMode::Aspect1By1);
        assert_eq!(config.machine, MachineModel::PC9801VX);

        let (config, _) = test.parse(&["--config", &game_text, "--aspect-mode", "4:3"]);
        assert_eq!(config.aspect_mode, AspectMode::Aspect4By3);
    }

    /// Verifies that the first media value of a layer replaces the list of the earlier layers.
    #[test]
    fn path_lists_replace_the_list_of_earlier_layers() {
        let test = TestLocations::new("lists");
        let root = test.current().to_path_buf();
        let images: Vec<PathBuf> = ["base", "a", "b", "c", "save", "x", "y"]
            .iter()
            .map(|name| root.join(format!("{name}.d88")))
            .collect();
        touch(&images.iter().map(PathBuf::as_path).collect::<Vec<_>>());
        write_file(&root.join(SETTINGS_FILE_NAME), "fdd1 = base.d88\n");
        let first = root.join("first.conf");
        write_file(&first, "fdd1 = a.d88\nfdd1 = b.d88\nfdd2 = save.d88\n");
        let second = root.join("second.conf");
        write_file(&second, "fdd1 = c.d88\n");
        let fdd1 = find_setting("fdd1").expect("fdd1 is a table key");

        let config = test.build(&with_machine(LaunchRequest::default()));
        assert_eq!(config.fdd1, vec![root.join("base.d88")]);

        let config = test.build(&with_machine(LaunchRequest {
            game_files: vec![first.clone()],
            ..LaunchRequest::default()
        }));
        assert_eq!(config.fdd1, vec![root.join("a.d88"), root.join("b.d88")]);

        let config = test.build(&with_machine(LaunchRequest {
            game_files: vec![first.clone(), second],
            ..LaunchRequest::default()
        }));
        assert_eq!(config.fdd1, vec![root.join("c.d88")]);
        assert_eq!(config.fdd2, vec![root.join("save.d88")]);

        let config = test.build(&with_machine(LaunchRequest {
            game_files: vec![first],
            flags: vec![
                (fdd1, root.join("x.d88").to_string_lossy().into_owned()),
                (fdd1, root.join("y.d88").to_string_lossy().into_owned()),
            ],
            ..LaunchRequest::default()
        }));
        assert_eq!(config.fdd1, vec![root.join("x.d88"), root.join("y.d88")]);
    }

    #[cfg(feature = "pc98")]
    /// Verifies that host keys apply only from the settings file and the command line, and that the media keys of the system defaults give way to those of the game.
    #[test]
    fn layers_ignore_keys_outside_their_scope() {
        let test = TestLocations::new("scopes");
        let root = test.current().to_path_buf();
        write_file(&root.join(SETTINGS_FILE_NAME), "");
        write_file(
            &root.join("config").join("pc98.conf"),
            "backend = legacy\nfdd1 = system.d88\nsoundboard = 26k\nconfig-folder = elsewhere\n",
        );
        touch(&[&root.join("game.d88"), &root.join("system.d88")]);
        let game = root.join("game.conf");
        write_file(
            &game,
            "machine = PC9801RA\naudio-volume = 0.5\npc98-roms = game-roms\nfdd1 = game.d88\n",
        );

        let config = test.build(&with_machine(LaunchRequest {
            game_files: vec![game],
            ..LaunchRequest::default()
        }));
        assert_eq!(config.backend, Backend::Modern);
        assert!((config.audio_volume - 1.0).abs() < f32::EPSILON);
        assert_eq!(config.pc98_roms, None);
        assert_eq!(config.config_folder, root.join("config"));
        assert_eq!(config.soundboard, SoundboardType::Sb26k);
        assert_eq!(config.fdd1, vec![root.join("game.d88")]);

        write_file(
            &root.join(SETTINGS_FILE_NAME),
            "backend = legacy\naudio-volume = 0.5\n",
        );
        let config = test.build(&with_machine(LaunchRequest::default()));
        assert_eq!(config.backend, Backend::Legacy);
        assert!((config.audio_volume - 0.5).abs() < f32::EPSILON);
    }

    /// Verifies that the media keys of the system defaults apply, that the first value of a game file replaces the list and that an empty value of a game file clears it.
    #[test]
    fn system_defaults_media_apply_until_a_game_replaces_or_clears_them() {
        let test = TestLocations::new("system_media");
        let root = test.current().to_path_buf();
        let system = compiled_systems().next().expect("one system is compiled");
        write_file(&root.join(SETTINGS_FILE_NAME), "fdd1 = settings.d88\n");
        touch(&[
            &root.join("settings.d88"),
            &root.join("system_a.d88"),
            &root.join("system_b.d88"),
            &root.join("system.hdi"),
            &root.join("game.d88"),
        ]);
        write_file(
            &root
                .join("config")
                .join(system.name)
                .with_extension(CONFIG_FILE_EXTENSION),
            "fdd1 = system_a.d88\nfdd1 = system_b.d88\nhdd1 = system.hdi\n",
        );
        let config = test.build(&with_machine(LaunchRequest::default()));
        assert_eq!(
            config.fdd1,
            vec![root.join("system_a.d88"), root.join("system_b.d88")]
        );
        assert_eq!(config.hdd1, Some(root.join("system.hdi")));

        let game = root.join("game.conf");
        let with_game = || {
            with_machine(LaunchRequest {
                game_files: vec![game.clone()],
                ..LaunchRequest::default()
            })
        };
        write_file(&game, "fdd1 = game.d88\n");
        let config = test.build(&with_game());
        assert_eq!(config.fdd1, vec![root.join("game.d88")]);
        assert_eq!(config.hdd1, Some(root.join("system.hdi")));

        write_file(&game, "fdd1 =\nhdd1 =\n");
        let config = test.build(&with_game());
        assert!(config.fdd1.is_empty());
        assert_eq!(config.hdd1, None);
    }

    #[cfg(feature = "pc98")]
    /// Verifies that the config folder of the settings file locates the system defaults.
    #[test]
    fn config_folder_locates_the_system_defaults() {
        let test = TestLocations::new("config_folder");
        let root = test.current().to_path_buf();
        write_file(
            &root.join(SETTINGS_FILE_NAME),
            &format!("{CONFIG_FOLDER_KEY} = games\n"),
        );
        write_file(&root.join("games").join("pc98.conf"), "soundboard = 14\n");
        write_file(&root.join("config").join("pc98.conf"), "soundboard = 86\n");

        let config = test.build(&with_machine(LaunchRequest::default()));
        assert_eq!(config.config_folder, root.join("games"));
        assert_eq!(config.soundboard, SoundboardType::Sb14);
    }

    #[cfg(all(feature = "pc98", feature = "msx"))]
    /// Verifies that the machine of the game file selects the system defaults of its family.
    #[test]
    fn machine_of_the_game_selects_the_system_defaults() {
        let test = TestLocations::new("system_family");
        let root = test.current().to_path_buf();
        write_file(
            &root.join(SETTINGS_FILE_NAME),
            "machine = PC9801VM\ncrt = off\n",
        );
        write_file(
            &root.join("config").join("pc98.conf"),
            "machine = PC9801VX\nscaling = nearest\n",
        );
        write_file(
            &root.join("config").join("msx.conf"),
            "machine = MSX2PLUS\nscaling = bilinear\n",
        );

        let config = test.build(&LaunchRequest::default());
        assert_eq!(config.machine, MachineModel::PC9801VX);
        assert_eq!(config.scaling, ScalingMode::Nearest);
        assert!(!config.crt);

        let game = root.join("game.conf");
        write_file(&game, "machine = MSX2\n");
        let config = test.build(&LaunchRequest {
            game_files: vec![game],
            ..LaunchRequest::default()
        });
        assert_eq!(config.target, Target::Msx);
        assert_eq!(config.msx_model, MsxModel::Msx2);
        assert_eq!(config.scaling, ScalingMode::Bilinear);

        write_file(
            &root.join("config").join("msx.conf"),
            "machine = PC9801RA\nscaling = bilinear\n",
        );
        let config = test.build(&LaunchRequest {
            flags: vec![(
                find_setting(MACHINE_KEY).expect("machine is a table key"),
                "MSX".to_owned(),
            )],
            ..LaunchRequest::default()
        });
        assert_eq!(config.target, Target::Msx);
        assert_eq!(config.msx_model, MsxModel::Msx);
        assert_eq!(config.scaling, ScalingMode::Bilinear);
    }

    #[cfg(feature = "msx")]
    /// Verifies that the system of the request selects the family when no layer sets `machine`.
    #[test]
    fn request_system_selects_the_family_without_a_machine() {
        let test = TestLocations::new("request_system");
        let root = test.current().to_path_buf();
        write_file(&root.join(SETTINGS_FILE_NAME), "crt = off\n");
        let msx = system_of(Target::Msx);

        let config = test.build(&LaunchRequest {
            system: Some(msx),
            ..LaunchRequest::default()
        });
        assert_eq!(config.target, Target::Msx);
        assert_eq!(config.msx_model, MsxModel::Msx);
        assert!(!config.crt);

        write_file(
            &root.join("config").join("msx.conf"),
            "machine = MSX2\nscaling = nearest\n",
        );
        let config = test.build(&LaunchRequest {
            system: Some(msx),
            ..LaunchRequest::default()
        });
        assert_eq!(config.msx_model, MsxModel::Msx2);
        assert_eq!(config.scaling, ScalingMode::Nearest);

        let game = root.join("game.conf");
        write_file(&game, "machine = MSX2PLUS\n");
        let config = test.build(&LaunchRequest {
            game_files: vec![game],
            system: Some(msx),
            ..LaunchRequest::default()
        });
        assert_eq!(config.msx_model, MsxModel::Msx2Plus);
    }

    #[cfg(feature = "msx")]
    /// Verifies that a game file in the folder of a system selects that system without a machine.
    #[test]
    fn game_folder_selects_the_system_without_a_machine() {
        let test = TestLocations::new("game_folder");
        let root = test.current().to_path_buf();
        write_file(&root.join(SETTINGS_FILE_NAME), "");
        write_file(&root.join("config").join("msx.conf"), "machine = MSX2\n");
        write_file(&root.join("config").join("msx").join("game.conf"), "");

        let (config, _) = test.parse(&["-c", "config/msx/game.conf"]);
        assert_eq!(config.target, Target::Msx);
        assert_eq!(config.msx_model, MsxModel::Msx2);

        write_file(&root.join("config").join("game.conf"), "");
        assert_eq!(
            test.error(&["-c", "config/game.conf"]),
            LaunchError::NoMachine.to_string()
        );
    }

    /// Verifies that the relative paths of the pref path settings file and the default folders resolve against the current directory.
    #[test]
    fn the_pref_path_settings_file_resolves_against_the_current_directory() {
        let test = TestLocations::new("pref_root");
        let current = test.current().to_path_buf();
        write_file(
            &test.pref().join(SETTINGS_FILE_NAME),
            "gui-font = fonts/ui.ttf\n",
        );
        let host =
            HostLayers::read(&LaunchRequest::default(), &test.locations).expect("settings read");
        assert_eq!(host.root(), current);
        assert_eq!(host.config_folder(), current.join("config"));
        assert_eq!(host.roms_folder(), current.join("roms"));
        assert_eq!(host.gui_font(), current.join("fonts").join("ui.ttf"));
    }

    /// Returns the default GUI font below the root and below the resource
    /// folder of `test`, whose settings file is empty.
    fn default_font_paths(test: &TestLocations) -> (PathBuf, PathBuf) {
        let root = test.current().to_path_buf();
        write_file(&root.join(SETTINGS_FILE_NAME), "");
        let executable = test
            .locations
            .executable_directory
            .as_deref()
            .expect("test locations have an executable folder");
        let resource_font = resource_folder(executable).join(DEFAULT_GUI_FONT);
        assert!(!resource_font.starts_with(&root));
        (root.join(DEFAULT_GUI_FONT), resource_font)
    }

    /// Returns the host layers of `test` without command line flags.
    fn default_host(test: &TestLocations) -> HostLayers {
        HostLayers::read(&LaunchRequest::default(), &test.locations).expect("settings read")
    }

    /// Verifies that the language folders lie below the root and then below the resource folder, and that the root wins a tag of both.
    #[test]
    fn the_language_folders_lie_below_the_root_and_the_resource_folder() {
        let test = TestLocations::new("language_folders");
        let (root_font, resource_font) = default_font_paths(&test);
        let root_folder = root_font
            .ancestors()
            .nth(2)
            .expect("the font lies two folders below the root")
            .join(LANGUAGE_FOLDER);
        let resource_folder = resource_font
            .ancestors()
            .nth(2)
            .expect("the font lies two folders below the resource folder")
            .join(LANGUAGE_FOLDER);
        let host = default_host(&test);
        assert_eq!(
            host.language_folders(),
            vec![root_folder.clone(), resource_folder.clone()]
        );
        assert_eq!(
            host.gui_resources(),
            GuiResources {
                font_paths: vec![root_font, resource_font],
                language_folders: vec![root_folder.clone(), resource_folder.clone()],
            }
        );
        write_file(&root_folder.join("ja.ini"), "");
        write_file(&resource_folder.join("ja.ini"), "");
        write_file(&resource_folder.join("de.ini"), "");
        let files = find_language_files(&host.language_folders());
        let found: Vec<(&str, PathBuf)> = files
            .iter()
            .map(|file| (file.tag.as_str(), file.path.clone()))
            .collect();
        assert_eq!(
            found,
            [
                ("ja", root_folder.join("ja.ini")),
                ("de", resource_folder.join("de.ini")),
            ]
        );
        let without_executable = Locations {
            executable_directory: None,
            ..test.locations.clone()
        };
        let host = HostLayers::read(&LaunchRequest::default(), &without_executable)
            .expect("settings read");
        assert_eq!(host.language_folders(), vec![root_folder]);
    }

    /// Verifies that the language of the user interface comes from the command line, then the settings file, then the built-in default.
    #[test]
    fn the_gui_language_follows_the_flag_and_the_settings_file() {
        let test = TestLocations::new("gui_language");
        write_file(&test.current().join(SETTINGS_FILE_NAME), "");
        assert_eq!(default_host(&test).gui_language(), GuiLanguage::Auto);
        write_file(
            &test.current().join(SETTINGS_FILE_NAME),
            "gui-language = ja\n",
        );
        assert_eq!(
            default_host(&test).gui_language(),
            GuiLanguage::Tag("ja".to_owned())
        );
        let request = LaunchRequest {
            flags: vec![(
                find_setting(GUI_LANGUAGE_KEY).expect("a table key"),
                "pt-BR".to_owned(),
            )],
            ..LaunchRequest::default()
        };
        let host = HostLayers::read(&request, &test.locations).expect("settings read");
        assert_eq!(host.gui_language(), GuiLanguage::Tag("pt-BR".to_owned()));
        assert_eq!(host_default(GUI_LANGUAGE_KEY).as_deref(), Some("auto"));
    }

    /// Verifies that the default GUI font is found below the root.
    #[test]
    fn the_default_gui_font_is_found_below_the_root() {
        let test = TestLocations::new("gui_font_root");
        let (root_font, _) = default_font_paths(&test);
        touch(&[&root_font]);
        assert_eq!(default_host(&test).gui_font(), root_font);
        let (config, _) = test.parse(&with_first_model(&[]));
        assert_eq!(config.gui_font, root_font);
    }

    /// Verifies that the default GUI font is found below the resource folder when the root has none.
    #[test]
    fn the_default_gui_font_is_found_below_the_resource_folder() {
        let test = TestLocations::new("gui_font_resource");
        let (_, resource_font) = default_font_paths(&test);
        touch(&[&resource_font]);
        assert_eq!(default_host(&test).gui_font(), resource_font);
        let (config, _) = test.parse(&with_first_model(&[]));
        assert_eq!(config.gui_font, resource_font);
    }

    /// Verifies that the default GUI font below the root wins over the one below the resource folder.
    #[test]
    fn the_default_gui_font_below_the_root_wins() {
        let test = TestLocations::new("gui_font_both");
        let (root_font, resource_font) = default_font_paths(&test);
        touch(&[&root_font, &resource_font]);
        assert_eq!(default_host(&test).gui_font(), root_font);
    }

    /// Verifies that a missing default GUI font gives both searched paths in the error.
    #[test]
    fn a_missing_default_gui_font_lists_both_paths() {
        let test = TestLocations::new("gui_font_missing");
        let (root_font, resource_font) = default_font_paths(&test);
        let host = default_host(&test);
        assert_eq!(
            host.gui_font_paths(),
            vec![root_font.clone(), resource_font.clone()]
        );
        assert_eq!(host.gui_font(), root_font);
        let Err(message) = GuiHost::load(
            &host.gui_font_paths(),
            gui::ui::ThemeKind::Dark,
            LanguageCatalog::English,
        ) else {
            panic!("a missing font does not load");
        };
        assert_eq!(
            message,
            format!(
                "the GUI font {} or {} was not found. The gui-font setting selects the font file",
                root_font.display(),
                resource_font.display()
            )
        );
    }

    /// Verifies that only the root is searched for the default GUI font when the folder of the executable is unknown.
    #[test]
    fn the_default_gui_font_lies_below_the_root_without_an_executable_folder() {
        let test = TestLocations::new("gui_font_no_executable");
        let (root_font, _) = default_font_paths(&test);
        let locations = Locations {
            executable_directory: None,
            ..test.locations.clone()
        };
        let host = HostLayers::read(&LaunchRequest::default(), &locations).expect("settings read");
        assert_eq!(host.gui_font_paths(), vec![root_font.clone()]);
        assert_eq!(host.gui_font(), root_font);
    }

    /// Verifies the GUI font values: a settings file value resolves against
    /// the root, a flag value against the current directory, neither is
    /// searched below the resource folder, and a game file cannot set it.
    #[test]
    fn the_gui_font_resolves_like_every_path_of_its_layer() {
        let test = TestLocations::new("gui_font");
        let current = test.current().to_path_buf();
        let root = current.join("root");
        let settings = root.join(SETTINGS_FILE_NAME);
        let settings_text = settings.to_string_lossy().into_owned();
        let (_, resource_font) = default_font_paths(&test);
        touch(&[&resource_font]);
        let host_font_paths = |args: &[&str]| match test.action(args) {
            Action::Manager(start) => HostLayers::read(&start.request, &start.locations)
                .expect("settings read")
                .gui_font_paths(),
            _ => panic!("expected the manager action"),
        };

        write_file(&settings, &format!("gui-font = {DEFAULT_GUI_FONT}\n"));
        let settings_font = root.join(DEFAULT_GUI_FONT);
        assert_eq!(
            host_font_paths(&["--settings", &settings_text]),
            vec![settings_font.clone()]
        );
        let (config, _) = test.parse(&with_first_model(&["--settings", &settings_text]));
        assert_eq!(config.gui_font, settings_font);

        let flag_font = current.join(DEFAULT_GUI_FONT);
        assert_eq!(
            host_font_paths(&["--settings", &settings_text, "--gui-font", DEFAULT_GUI_FONT]),
            vec![flag_font.clone()]
        );
        let gui_font_flag = format!("--gui-font={DEFAULT_GUI_FONT}");
        let (config, _) = test.parse(&with_first_model(&[
            "--settings",
            &settings_text,
            &gui_font_flag,
        ]));
        assert_eq!(config.gui_font, flag_font);

        let game = root.join("game.conf");
        write_file(
            &game,
            &format!("machine = {}\ngui-font = game.ttf\n", first_model()),
        );
        let game_text = game.to_string_lossy().into_owned();
        let (config, _) = test.parse(&["--settings", &settings_text, "-c", &game_text]);
        assert_eq!(config.gui_font, settings_font);
    }

    #[cfg(all(feature = "pc98", feature = "msx"))]
    /// Verifies that the system of the launch outranks the machine of the settings file.
    #[test]
    fn launch_system_outranks_the_settings_machine() {
        let test = TestLocations::new("settings_machine");
        let root = test.current().to_path_buf();
        write_file(&root.join(SETTINGS_FILE_NAME), "machine = PC9801RA\n");
        write_file(
            &root.join("config").join("msx.conf"),
            "machine = MSX2\nscaling = nearest\n",
        );
        write_file(&root.join("config").join("msx").join("game.conf"), "");

        let config = test.build(&LaunchRequest {
            system: Some(system_of(Target::Msx)),
            ..LaunchRequest::default()
        });
        assert_eq!(config.target, Target::Msx);
        assert_eq!(config.msx_model, MsxModel::Msx2);
        assert_eq!(config.scaling, ScalingMode::Nearest);

        let (config, _) = test.parse(&["-c", "config/msx/game.conf"]);
        assert_eq!(config.target, Target::Msx);
        assert_eq!(config.msx_model, MsxModel::Msx2);
        assert_eq!(config.scaling, ScalingMode::Nearest);

        std::fs::remove_file(root.join("config").join("msx.conf"))
            .expect("system defaults are removed");
        let (config, _) = test.parse(&["-c", "config/msx/game.conf"]);
        assert_eq!(config.target, Target::Msx);
        assert_eq!(config.msx_model, MsxModel::Msx);
    }

    #[cfg(feature = "pc98")]
    /// Verifies that the machine of the settings file applies when the game and the system defaults set none.
    #[test]
    fn settings_machine_of_the_same_family_applies() {
        let test = TestLocations::new("settings_same_family");
        let root = test.current().to_path_buf();
        write_file(&root.join(SETTINGS_FILE_NAME), "machine = PC9801VM\n");
        write_file(&root.join("config").join("pc98.conf"), "soundboard = 14\n");
        write_file(
            &root.join("config").join("pc98").join("game.conf"),
            "crt = off\n",
        );

        let (config, _) = test.parse(&["-c", "config/pc98/game.conf"]);
        assert_eq!(config.target, Target::Pc98);
        assert_eq!(config.machine, MachineModel::PC9801VM);
        assert_eq!(config.soundboard, SoundboardType::Sb14);
        assert!(!config.crt);
    }

    #[cfg(feature = "pc88")]
    /// Verifies that an explicit CPU mode of the system defaults survives the derived defaults of the game.
    #[test]
    fn explicit_cpu_mode_survives_across_layers() {
        let test = TestLocations::new("explicit");
        let root = test.current().to_path_buf();
        write_file(&root.join(SETTINGS_FILE_NAME), "");
        write_file(&root.join("config").join("pc88.conf"), "cpu-mode = high\n");
        let game = root.join("game.conf");
        write_file(&game, "machine = PC8801MC\nboot-mode = v1s\n");
        let request = LaunchRequest {
            game_files: vec![game],
            ..LaunchRequest::default()
        };
        let config = test.build(&request);
        assert_eq!(config.cpu_mode, CpuMode::High);
        assert_eq!(config.pc88_memory_wait, MemoryWaitSwitch::Compatible);

        std::fs::remove_file(root.join("config").join("pc88.conf"))
            .expect("system defaults are removed");
        let config = test.build(&request);
        assert_eq!(config.cpu_mode, CpuMode::Low);
    }

    /// Verifies the derived ROM folders: an explicit key wins and a missing optional folder stays unset.
    #[test]
    fn rom_keys_derive_from_the_rom_folder() {
        let test = TestLocations::new("roms");
        let root = test.current().to_path_buf();
        std::fs::create_dir_all(root.join("roms").join("pc98")).expect("folder is created");
        std::fs::create_dir_all(root.join("roms").join("mt32")).expect("folder is created");
        write_file(&root.join(SETTINGS_FILE_NAME), "x1-roms = /explicit/x1\n");

        let config = test.build(&with_machine(LaunchRequest::default()));
        assert_eq!(config.pc98_roms, Some(root.join("roms").join("pc98")));
        assert_eq!(config.mt32_roms, Some(root.join("roms").join("mt32")));
        assert_eq!(config.at_roms, None);
        assert_eq!(config.sc55_roms, None);
        assert_eq!(config.x1_roms, Some(PathBuf::from("/explicit/x1")));
        assert_eq!(config.pc88_roms, Some(root.join("roms").join("pc88")));

        write_file(
            &root.join(SETTINGS_FILE_NAME),
            "roms-folder = /elsewhere\npc98-roms = mine\n",
        );
        let config = test.build(&with_machine(LaunchRequest::default()));
        assert_eq!(config.pc98_roms, Some(root.join("mine")));
        assert_eq!(config.mt32_roms, None);
        assert_eq!(config.towns_roms, Some(PathBuf::from("/elsewhere/towns")));
    }

    #[cfg(feature = "at")]
    #[test]
    fn config_file_applies_named_pc_at_key_binding() {
        let test = TestLocations::new("at_key");
        let (config, key_map) = test.parse_game("at_key.conf", "machine=AT486DX66\nkey.B=A\n");
        assert_eq!(config.target, Target::At);
        assert_eq!(key_map.get(sdl3::keyboard::Scancode::B), Some(0x1E));
    }

    #[cfg(all(feature = "at", feature = "msx"))]
    /// Verifies that the key bindings of a file parse against the family of the final machine.
    #[test]
    fn key_bindings_parse_against_the_final_family() {
        let test = TestLocations::new("final_family");
        let path = test.file("final_family.conf", "machine=AT486DX66\nkey.B=A\n");
        let (config, key_map) =
            test.parse(&["--config", &path.to_string_lossy(), "--machine", "MSX"]);

        assert_eq!(config.target, Target::Msx);
        let expected = keymap::parse_key_binding(Target::Msx, "B", "A").map(|(_, code)| code);
        assert!(expected.is_some());
        assert_eq!(key_map.get(sdl3::keyboard::Scancode::B), expected);
    }
}
