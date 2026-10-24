//! Settings discovery, the root folder and the layered configuration of a launch.
//!
//! A launch reads its layers first: the settings file, the game files and the
//! command line flags. The family comes from the first source of: the
//! `machine` of the command line or of a game file, the system of the request,
//! the system folder of the last game file, the `machine` of the settings file
//! and the built-in default. A `machine` of another family leaves the settings
//! file and the system defaults. The system defaults of the family join as the
//! third layer. The machine applies first, the remaining lines follow in layer
//! order. The built configuration names only images that exist.

use std::path::{Component, Path, PathBuf};

use common::{Context, StringError, bail, info, warn};
use machine_factory::config::{
    EmulatorConfig, Target, resolve_model,
    settings::{
        CONFIG_FOLDER_KEY, ExplicitSettings, MACHINE_KEY, Scope, Setting, ValueKind,
        apply_derived_defaults, find_setting,
    },
    systems::{System, apply_rom_folder_defaults, compiled_systems, system_of},
};

use super::keymap::parse_key_binding;
use crate::input::KeyOverrides;

/// File name of the settings file.
pub const SETTINGS_FILE_NAME: &str = "neetan.conf";
/// File extension of the system defaults files.
const SYSTEM_DEFAULTS_EXTENSION: &str = "conf";
/// Prefix of the key binding lines.
const KEY_BINDING_PREFIX: &str = "key.";

/// Folders that the settings discovery searches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locations {
    /// The current directory of the process.
    pub current_directory: PathBuf,
    /// The folder of the executable, when it is known.
    pub executable_directory: Option<PathBuf>,
    /// The SDL pref path, when SDL provides one.
    pub pref_path: Option<PathBuf>,
}

impl Locations {
    /// Returns the folders of the running process.
    pub fn of_process() -> crate::Result<Self> {
        let current_directory =
            std::env::current_dir().context("Failed to read the current directory")?;
        let executable_directory = std::env::current_exe()
            .ok()
            .and_then(|executable| executable.parent().map(Path::to_path_buf));
        let pref_path = sdl3::filesystem::get_pref_path(crate::COMPANY_NAME, crate::GAME_NAME);
        Ok(Self {
            current_directory,
            executable_directory,
            pref_path,
        })
    }
}

/// The settings file of a launch and its root folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsLocation {
    /// The settings file. `None` when the discovery found no file.
    pub file: Option<PathBuf>,
    /// The folder of the settings file. Without a file it is the SDL pref path.
    pub root: PathBuf,
}

/// Finds the settings file and the root folder.
///
/// The settings file is the first match of the explicit file, `neetan.conf` in
/// the current directory, `neetan.conf` beside the executable and `neetan.conf`
/// in the SDL pref path. An explicit file must exist. Without a match the root
/// is the SDL pref path, or the current directory when SDL has no pref path.
pub fn discover_settings(
    explicit: Option<&Path>,
    locations: &Locations,
) -> crate::Result<SettingsLocation> {
    if let Some(file) = explicit {
        let file = resolve_path(&locations.current_directory, file);
        if !file.is_file() {
            bail!("settings file not found: {}", file.display());
        }
        return Ok(SettingsLocation {
            root: parent_folder(&file),
            file: Some(file),
        });
    }
    let candidates = [
        Some(&locations.current_directory),
        locations.executable_directory.as_ref(),
        locations.pref_path.as_ref(),
    ];
    for folder in candidates.into_iter().flatten() {
        let file = folder.join(SETTINGS_FILE_NAME);
        if file.is_file() {
            return Ok(SettingsLocation {
                root: resolve_path(&locations.current_directory, folder),
                file: Some(file),
            });
        }
    }
    let root = locations
        .pref_path
        .as_ref()
        .unwrap_or(&locations.current_directory);
    Ok(SettingsLocation {
        file: None,
        root: resolve_path(&locations.current_directory, root),
    })
}

/// Returns the folder that holds `file`.
fn parent_folder(file: &Path) -> PathBuf {
    file.parent().map_or_else(PathBuf::new, Path::to_path_buf)
}

/// Returns `path` as an absolute path. A relative path lies below `base`.
#[must_use]
pub fn resolve_path(base: &Path, path: &Path) -> PathBuf {
    base.join(path)
        .components()
        .filter(|component| !matches!(component, Component::CurDir))
        .collect()
}

/// Returns the value of `setting` with a relative path resolved against `base`.
///
/// An empty value and a value of a kind without a path stay as written.
#[must_use]
pub fn resolve_value(setting: &Setting, base: &Path, value: &str) -> String {
    match setting.kind {
        ValueKind::Path(_) | ValueKind::PathList if !value.is_empty() => {
            resolve_path(base, Path::new(value))
                .to_string_lossy()
                .into_owned()
        }
        ValueKind::Choice(_)
        | ValueKind::OnOff
        | ValueKind::Number(_)
        | ValueKind::Path(_)
        | ValueKind::PathList => value.to_owned(),
    }
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

/// Origin of a layer.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum LayerKind {
    /// The settings file.
    SettingsFile,
    /// The system defaults `<config-folder>/<system>.conf`.
    SystemDefaults,
    /// A game file from `--config`.
    GameFile,
    /// The command line flags.
    CommandLine,
}

impl LayerKind {
    /// Returns whether a layer of this kind sets keys of `scope`.
    ///
    /// Host keys belong to the settings file and the command line. Media keys
    /// belong to the games.
    fn accepts(self, scope: Scope) -> bool {
        match (self, scope) {
            (LayerKind::SettingsFile | LayerKind::CommandLine, _) => true,
            (LayerKind::SystemDefaults, Scope::System) => true,
            (LayerKind::SystemDefaults, Scope::Host | Scope::Game) => false,
            (LayerKind::GameFile, Scope::System | Scope::Game) => true,
            (LayerKind::GameFile, Scope::Host) => false,
        }
    }
}

/// One line of a layer.
#[derive(Debug, Clone)]
enum LayerLine {
    /// A key of the settings table with its value. A path value is absolute.
    Setting(&'static Setting, String),
    /// A `key.<host> = <native>` binding with the host and the native key name.
    KeyBinding(String, String),
}

/// The lines of one configuration source.
#[derive(Debug, Clone)]
struct Layer {
    /// The origin of the layer.
    kind: LayerKind,
    /// The name of the source in warnings: a file path or the command line.
    source: String,
    /// The lines in source order.
    lines: Vec<LayerLine>,
}

impl Layer {
    /// Returns the last value of `key` in this layer.
    fn last_value(&self, key: &str) -> Option<&str> {
        self.lines.iter().rev().find_map(|line| match line {
            LayerLine::Setting(setting, value) if setting.key == key => Some(value.as_str()),
            LayerLine::Setting(..) | LayerLine::KeyBinding(..) => None,
        })
    }
}

/// Returns the family that the model `value` belongs to.
fn machine_target(value: &str) -> Result<Target, String> {
    let mut config = EmulatorConfig::default();
    resolve_model(&mut config, value)?;
    Ok(config.target)
}

/// Checks a value of `setting` against the settings table.
pub fn check_value(setting: &Setting, value: &str) -> Result<(), String> {
    (setting.apply)(&mut EmulatorConfig::default(), value)
}

/// Reads a configuration file into a layer.
///
/// Relative path values resolve against `root`. Unknown keys and invalid
/// values end with a warning and leave the layer.
fn read_layer(kind: LayerKind, file: &Path, root: &Path) -> crate::Result<Layer> {
    let contents = std::fs::read_to_string(file)
        .with_context(|| format!("Failed to read {}", file.display()))?;
    let source = file.display().to_string();
    let mut lines = Vec::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if let Some(host_name) = key.strip_prefix(KEY_BINDING_PREFIX) {
            lines.push(LayerLine::KeyBinding(
                host_name.to_owned(),
                value.to_owned(),
            ));
            continue;
        }
        let Some(setting) = find_setting(key) else {
            warn!("{source}: unknown key {key}");
            continue;
        };
        let value = resolve_value(setting, root, value);
        if let Err(message) = check_value(setting, &value) {
            warn!("{source}: invalid {key}: {message}");
            continue;
        }
        lines.push(LayerLine::Setting(setting, value));
    }
    Ok(Layer {
        kind,
        source,
        lines,
    })
}

/// Removes the machine lines of `layer` that name another family than `target`.
///
/// Returns the removed machine values.
fn drop_foreign_machines(layer: &mut Layer, target: Target) -> Vec<String> {
    let mut dropped = Vec::new();
    layer.lines.retain(|line| match line {
        LayerLine::Setting(setting, value) if setting.key == MACHINE_KEY => {
            let matches = machine_target(value).is_ok_and(|family| family == target);
            if !matches {
                dropped.push(value.clone());
            }
            matches
        }
        LayerLine::Setting(..) | LayerLine::KeyBinding(..) => true,
    });
    dropped
}

/// Builds the layered configuration of a launch.
///
/// The layers apply in the order built-in defaults, settings file, system
/// defaults, game files and command line flags. A later layer overrides an
/// earlier one. The first value of a path list in a layer replaces the list of
/// the earlier layers.
pub fn build_configuration(
    request: &LaunchRequest,
    locations: &Locations,
) -> crate::Result<(EmulatorConfig, KeyOverrides)> {
    let settings = discover_settings(request.settings.as_deref(), locations)?;
    match &settings.file {
        Some(file) => info!("Settings file: {}", file.display()),
        None => info!("No settings file found"),
    }
    info!("Root folder: {}", settings.root.display());
    let root = &settings.root;

    let mut settings_layer = settings
        .file
        .as_deref()
        .map(|file| read_layer(LayerKind::SettingsFile, file, root))
        .transpose()?;
    let game_layers = request
        .game_files
        .iter()
        .map(|file| read_layer(LayerKind::GameFile, file, root))
        .collect::<crate::Result<Vec<Layer>>>()?;
    let command_line = Layer {
        kind: LayerKind::CommandLine,
        source: "command line".to_owned(),
        lines: request
            .flags
            .iter()
            .map(|(setting, value)| LayerLine::Setting(setting, value.clone()))
            .collect(),
    };

    let config_folder = std::iter::once(&command_line)
        .chain(settings_layer.as_ref())
        .find_map(|layer| layer.last_value(CONFIG_FOLDER_KEY))
        .map_or_else(|| EmulatorConfig::default().config_folder, PathBuf::from);
    let config_folder = resolve_path(root, &config_folder);

    let launch_machine = std::iter::once(&command_line)
        .chain(game_layers.iter().rev())
        .find_map(|layer| layer.last_value(MACHINE_KEY));
    let launch_system = request
        .system
        .or_else(|| game_folder_system(&request.game_files, &config_folder));
    let settings_machine = settings_layer
        .as_ref()
        .and_then(|layer| layer.last_value(MACHINE_KEY));
    let family_model = launch_machine
        .or(launch_system.map(|system| system.default_model))
        .or(settings_machine);
    let target = match family_model {
        Some(model) => machine_target(model).map_err(StringError)?,
        None => EmulatorConfig::default().target,
    };
    if let Some(layer) = settings_layer.as_mut() {
        drop_foreign_machines(layer, target);
    }

    let system_file = config_folder
        .join(system_of(target).name)
        .with_extension(SYSTEM_DEFAULTS_EXTENSION);
    let system_layer = if system_file.is_file() {
        info!("System defaults: {}", system_file.display());
        let mut layer = read_layer(LayerKind::SystemDefaults, &system_file, root)?;
        for value in drop_foreign_machines(&mut layer, target) {
            warn!(
                "{}: machine {value} belongs to another system. The line is ignored.",
                layer.source
            );
        }
        Some(layer)
    } else {
        None
    };

    let mut layers: Vec<Layer> = Vec::new();
    layers.extend(settings_layer);
    layers.extend(system_layer);
    layers.extend(game_layers);
    layers.push(command_line);

    let mut config = EmulatorConfig::default();
    let mut explicit = ExplicitSettings::default();
    let mut key_map = KeyOverrides::new();

    let machine = layers
        .iter()
        .rev()
        .find_map(|layer| layer.last_value(MACHINE_KEY))
        .unwrap_or(system_of(target).default_model);
    if let Some(setting) = find_setting(MACHINE_KEY) {
        explicit
            .apply(&mut config, setting, machine)
            .map_err(StringError)?;
    }

    for layer in &layers {
        apply_layer(&mut config, &mut explicit, &mut key_map, layer)?;
    }

    config.config_folder = resolve_path(root, &config.config_folder);
    config.roms_folder = resolve_path(root, &config.roms_folder);
    let roms_folder = config.roms_folder.clone();
    apply_rom_folder_defaults(&mut config, &roms_folder).map_err(StringError)?;
    apply_derived_defaults(&mut config, explicit);
    validate_images(&config)?;
    Ok((config, key_map))
}

/// Returns the system whose folder below `config_folder` holds the last game file of `game_files`.
fn game_folder_system(game_files: &[PathBuf], config_folder: &Path) -> Option<&'static System> {
    game_files.iter().rev().find_map(|file| {
        let folder = file.parent()?;
        compiled_systems().find(|system| folder == config_folder.join(system.name))
    })
}

/// Checks that every disk and disc image of `config` exists.
fn validate_images(config: &EmulatorConfig) -> crate::Result<()> {
    for (key, paths) in [
        ("fdd1", &config.fdd1),
        ("fdd2", &config.fdd2),
        ("cdrom", &config.cdrom),
    ] {
        if let Some(path) = paths.iter().find(|path| !path.exists()) {
            bail!("{key} image not found: {}", path.display());
        }
    }
    for (key, path) in [("hdd1", &config.hdd1), ("hdd2", &config.hdd2)] {
        if let Some(path) = path
            && !path.exists()
        {
            bail!("{key} image not found: {}", path.display());
        }
    }
    Ok(())
}

/// Applies the lines of one layer except `machine`.
///
/// A line of a scope that the layer does not set ends with a warning. An
/// invalid value ends with a warning in a file and with an error on the
/// command line.
fn apply_layer(
    config: &mut EmulatorConfig,
    explicit: &mut ExplicitSettings,
    key_map: &mut KeyOverrides,
    layer: &Layer,
) -> crate::Result<()> {
    let mut cleared_lists: Vec<&str> = Vec::new();
    for line in &layer.lines {
        match line {
            LayerLine::KeyBinding(host_name, native_name) => {
                match parse_key_binding(config.target, host_name, native_name) {
                    Some((host, code)) => key_map.set(host, code),
                    None => warn!(
                        "{}: invalid key binding {KEY_BINDING_PREFIX}{host_name} = {native_name}",
                        layer.source
                    ),
                }
            }
            LayerLine::Setting(setting, value) => {
                if setting.key == MACHINE_KEY {
                    continue;
                }
                if !layer.kind.accepts(setting.scope) {
                    warn_out_of_scope(layer, setting);
                    continue;
                }
                if let Some(clear) = setting.clear
                    && !cleared_lists.contains(&setting.key)
                {
                    clear(config);
                    cleared_lists.push(setting.key);
                }
                if let Err(message) = explicit.apply(config, setting, value) {
                    match layer.kind {
                        LayerKind::CommandLine => bail!("--{}: {message}", setting.key),
                        LayerKind::SettingsFile
                        | LayerKind::SystemDefaults
                        | LayerKind::GameFile => {
                            warn!("{}: invalid {}: {message}", layer.source, setting.key);
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// Warns about a line whose scope the layer does not set.
fn warn_out_of_scope(layer: &Layer, setting: &Setting) {
    match setting.scope {
        Scope::Host => warn!(
            "{}: host key {} is ignored. Only the settings file and the command line set host keys.",
            layer.source, setting.key
        ),
        Scope::Game => warn!(
            "{}: media key {} is ignored. Media keys belong to the game files.",
            layer.source, setting.key
        ),
        Scope::System => warn!("{}: key {} is ignored.", layer.source, setting.key),
    }
}
