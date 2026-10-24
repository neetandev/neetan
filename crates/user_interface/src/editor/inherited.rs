//! The inherited value of a field: the value that the layers below the game
//! file give a key, the system defaults, the settings file and the built-in
//! default, and the layer that gives it.

use std::path::{Path, PathBuf};

use machine_factory::config::{
    MachineSelection, Target,
    settings::{MACHINE_KEY, Setting, ValueKind, apply_derived_defaults},
};

use crate::{
    config::{
        DocumentError, HostLayers, ModelSource, check_value,
        layers::{apply_layers, launch_model, read_system_defaults, system_defaults_file},
        reading::{Layer, LayerKind, LayerLine},
        resolve_value,
    },
    manager::shown_path,
};

/// The layer that gives an inherited value.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) enum InheritedSource {
    /// The built-in default of the model.
    BuiltIn,
    /// The system defaults `<config-folder>/<system>.conf`.
    SystemDefaults,
    /// The settings file.
    SettingsFile,
}

impl InheritedSource {
    /// The caption of the source before the value, for example `System default`.
    pub(crate) fn caption(self) -> &'static str {
        match self {
            Self::BuiltIn => "Built-in default",
            Self::SystemDefaults => "System default",
            Self::SettingsFile => "Settings default",
        }
    }

    /// The source of the model that a launch takes from `source`.
    fn of_model(source: ModelSource) -> Self {
        match source {
            ModelSource::SystemDefaults => Self::SystemDefaults,
            ModelSource::SettingsFile => Self::SettingsFile,
            ModelSource::CommandLine | ModelSource::GameFile | ModelSource::BuiltIn => {
                Self::BuiltIn
            }
        }
    }
}

/// The value that a key inherits and its source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Inherited {
    /// The values as the file writes them. A path lies below the root when it lies there.
    /// Empty while no layer and no default sets the key.
    pub(crate) values: Vec<String>,
    /// The layer that gives the values.
    pub(crate) source: InheritedSource,
}

impl Inherited {
    /// The values joined with commas.
    pub(crate) fn text(&self) -> String {
        self.values.join(", ")
    }

    /// The value with its source, for example `Built-in default: high`. A
    /// key without a value gives `None`.
    pub(crate) fn label(&self) -> Option<String> {
        (!self.values.is_empty()).then(|| format!("{}: {}", self.source.caption(), self.text()))
    }
}

/// The layers below the game files of one family: the settings file and the
/// system defaults of the family.
#[derive(Debug, Clone)]
pub(crate) struct BaseLayers {
    /// The family of the games.
    target: Target,
    /// The root folder.
    root: PathBuf,
    /// The settings file without its key bindings.
    settings: Option<Layer>,
    /// The system defaults of the family without their key bindings.
    system_defaults: Option<Layer>,
    /// The warning about a system defaults file that does not read. The
    /// inherited values then leave the file out.
    warning: Option<String>,
}

/// The warning about the system defaults file at the shown path `shown`
/// that does not read with `error`, for example `System defaults ignored:
/// config/pc98.conf line 3: the file is not valid UTF-8`.
pub(crate) fn system_defaults_warning(shown: &str, error: &DocumentError) -> String {
    match error.line() {
        Some(_) => format!("System defaults ignored: {shown} {error}"),
        None => format!("System defaults ignored: {shown}: {error}"),
    }
}

/// Returns `layer` without its key binding lines, which give no inherited value.
fn without_key_bindings(layer: &Layer) -> Layer {
    Layer {
        lines: layer
            .lines
            .iter()
            .filter(|line| matches!(line, LayerLine::Setting(..)))
            .cloned()
            .collect(),
        ..layer.clone()
    }
}

/// Returns the value of `setting` as the file writes it: a path below `root`
/// relative to the root.
fn shown_value(setting: &Setting, value: &str, root: &Path) -> String {
    match setting.kind {
        ValueKind::Path(_) | ValueKind::PathList => {
            Path::new(value).strip_prefix(root).map_or_else(
                |_| value.to_owned(),
                |path| path.to_string_lossy().into_owned(),
            )
        }
        ValueKind::Choice(_) | ValueKind::OnOff | ValueKind::Number(_) => value.to_owned(),
    }
}

impl BaseLayers {
    /// Reads the layers below the game files of the family `target` from
    /// `host`. A system defaults file that does not read gives no layer and
    /// the warning about it.
    pub(crate) fn read(host: &HostLayers, target: Target) -> Self {
        let file = system_defaults_file(&host.config_folder(), target);
        let (system_defaults, warning) = match read_system_defaults(&file, target, host.root()) {
            Ok(defaults) => (
                defaults.map(|defaults| without_key_bindings(&defaults.layer)),
                None,
            ),
            Err(error) => {
                let shown = shown_path(&file, host.root());
                (None, Some(system_defaults_warning(&shown, &error)))
            }
        };
        Self {
            target,
            root: host.root().to_path_buf(),
            settings: host.settings_layer().map(without_key_bindings),
            system_defaults,
            warning,
        }
    }

    /// The warning about a system defaults file that does not read.
    pub(crate) fn warning(&self) -> Option<&str> {
        self.warning.as_deref()
    }

    /// The model that a game without `machine` takes and its source.
    pub(crate) fn model(&self) -> (MachineSelection, InheritedSource) {
        let command_line = Layer::command_line(&[]);
        let (value, source) = launch_model(
            self.target,
            self.settings.as_ref(),
            self.system_defaults.as_ref(),
            &[],
            &command_line,
        );
        let model = value
            .parse::<MachineSelection>()
            .unwrap_or_else(|_| MachineSelection::default_of(self.target));
        (model, InheritedSource::of_model(source))
    }

    /// The value that `setting` inherits on `model` below a game file with
    /// the values `game`, as the file writes them.
    ///
    /// The values of the game except those of `setting` take part. A
    /// default that derives from another setting, such as the PC-88 memory
    /// wait from the boot mode, follows the game this way. A value that the
    /// settings table rejects takes no part.
    pub(crate) fn inherited(
        &self,
        setting: &'static Setting,
        model: MachineSelection,
        game: &[(&'static Setting, Vec<String>)],
    ) -> Inherited {
        if setting.key == MACHINE_KEY {
            let (model, source) = self.model();
            return Inherited {
                values: vec![model.name().to_owned()],
                source,
            };
        }
        let source = [
            (&self.system_defaults, InheritedSource::SystemDefaults),
            (&self.settings, InheritedSource::SettingsFile),
        ]
        .into_iter()
        .find_map(|(layer, source)| {
            layer
                .as_ref()
                .is_some_and(|layer| layer.last_value(setting.key).is_some())
                .then_some(source)
        })
        .unwrap_or(InheritedSource::BuiltIn);
        let game_layer = Layer {
            kind: LayerKind::GameFile,
            source: "editor".to_owned(),
            lines: game
                .iter()
                .filter(|(other, _)| other.key != setting.key && other.key != MACHINE_KEY)
                .flat_map(|(other, values)| {
                    values.iter().filter_map(|value| {
                        let value = resolve_value(other, &self.root, value);
                        check_value(other, &value)
                            .is_ok()
                            .then_some(LayerLine::Setting(other, value))
                    })
                })
                .collect(),
        };
        let layers = self
            .settings
            .iter()
            .chain(&self.system_defaults)
            .chain(std::iter::once(&game_layer));
        let values = match apply_layers(model, layers) {
            Ok((mut config, explicit, _)) => {
                apply_derived_defaults(&mut config, explicit);
                (setting.format)(&config)
            }
            Err(_) => Vec::new(),
        };
        let values = if values.is_empty() {
            setting
                .unset_text(self.target)
                .map(|text| vec![text.to_owned()])
                .unwrap_or_default()
        } else {
            values
                .iter()
                .map(|value| shown_value(setting, value, &self.root))
                .collect()
        };
        Inherited { values, source }
    }
}

#[cfg(test)]
mod tests {
    use machine_factory::config::settings::find_setting;

    use super::*;
    use crate::editor::test_support::EditorRoot;

    /// The inherited value of `key` on `model` below a game without values.
    fn inherited(base: &BaseLayers, key: &str, model: MachineSelection) -> Inherited {
        let setting = find_setting(key).expect("table key");
        base.inherited(setting, model, &[])
    }

    /// Verifies the sources of the inherited values: the system defaults over the settings file over the built-in default.
    #[test]
    fn the_system_defaults_win_over_the_settings_file_and_the_built_in_default() {
        let root = EditorRoot::new("inherited_sources");
        let system = root.system();
        root.write(
            "neetan.conf",
            "crt = off\nscaling = bilinear\nkey.F1 = Stop\n",
        );
        root.write(
            &format!("config/{}.conf", system.name),
            "scaling = nearest\n",
        );
        let host = root.host();
        let target = root.target();
        let base = BaseLayers::read(&host, target);
        let (model, source) = base.model();
        assert_eq!(model.name(), system.default_model);
        assert_eq!(source, InheritedSource::BuiltIn);

        let scaling = inherited(&base, "scaling", model);
        assert_eq!(scaling.values, ["nearest"]);
        assert_eq!(scaling.source, InheritedSource::SystemDefaults);
        assert_eq!(scaling.label().as_deref(), Some("System default: nearest"));
        let crt = inherited(&base, "crt", model);
        assert_eq!(crt.values, ["off"]);
        assert_eq!(crt.label().as_deref(), Some("Settings default: off"));
        let aspect = inherited(&base, "aspect-mode", model);
        assert_eq!(aspect.source, InheritedSource::BuiltIn);
        assert_eq!(aspect.label().as_deref(), Some("Built-in default: 4:3"));
    }

    /// Verifies that the model of the system defaults is the inherited model, also when it is the built-in default, and that a path of the settings file lies below the root.
    #[test]
    fn the_model_and_the_paths_come_from_the_layers() {
        let root = EditorRoot::new("inherited_model");
        let system = root.system();
        let model_names: Vec<&str> = machine_factory::config::settings::MODEL_CHOICES
            .iter()
            .filter(|choice| choice.families.contains_target(root.target()))
            .map(|choice| choice.value)
            .collect();
        let other = model_names
            .iter()
            .find(|name| **name != system.default_model)
            .copied()
            .unwrap_or(system.default_model);
        root.write(
            &format!("config/{}.conf", system.name),
            &format!("machine = {other}\n"),
        );
        let host = root.host();
        let base = BaseLayers::read(&host, root.target());
        let machine = inherited(&base, MACHINE_KEY, base.model().0);
        assert_eq!(machine.values, [other]);
        assert_eq!(machine.source, InheritedSource::SystemDefaults);

        let fdd1 = find_setting("fdd1").expect("fdd1 is a table key");
        root.write("neetan.conf", "fdd1 = games/boot.d88\n");
        let base = BaseLayers::read(&root.host(), root.target());
        let inherited = base.inherited(fdd1, base.model().0, &[]);
        assert_eq!(inherited.values, ["games/boot.d88"]);
        assert_eq!(inherited.source, InheritedSource::SettingsFile);
        let game = [(fdd1, vec!["games/own.d88".to_owned()])];
        assert_eq!(
            base.inherited(fdd1, base.model().0, &game).values,
            ["games/boot.d88"]
        );
    }

    /// Verifies that a system defaults file that does not read gives no inherited values and the warning with its path and line.
    #[test]
    fn a_system_defaults_file_that_does_not_read_gives_a_warning() {
        let root = EditorRoot::new("inherited_broken");
        let system = root.system();
        let file = root.write(
            &format!("config/{}.conf", system.name),
            "scaling = nearest\n",
        );
        let base = BaseLayers::read(&root.host(), root.target());
        assert_eq!(base.warning(), None);
        assert_eq!(
            inherited(&base, "scaling", base.model().0).source,
            InheritedSource::SystemDefaults
        );
        std::fs::write(&file, b"scaling = nearest\ncrt = \xFF\n").expect("file is written");
        let base = BaseLayers::read(&root.host(), root.target());
        assert_eq!(
            base.warning(),
            Some(
                format!(
                    "System defaults ignored: config/{}.conf line 2: the file is not valid UTF-8",
                    system.name
                )
                .as_str()
            )
        );
        let scaling = inherited(&base, "scaling", base.model().0);
        assert_eq!(scaling.source, InheritedSource::BuiltIn);
    }

    #[cfg(feature = "pc88")]
    /// Verifies that the inherited memory wait follows the boot mode of the game and that the unset boot mode shows its machine value.
    #[test]
    fn the_pc88_memory_wait_follows_the_boot_mode_of_the_game() {
        let root = EditorRoot::with_system("inherited_pc88", "pc88");
        let base = BaseLayers::read(&root.host(), Target::Pc88);
        let model = base.model().0;
        let wait = find_setting("pc88-memory-wait").expect("table key");
        let boot_mode = find_setting("boot-mode").expect("table key");
        assert_eq!(base.inherited(wait, model, &[]).values, ["fast"]);
        let game = [(boot_mode, vec!["v1s".to_owned()])];
        let inherited = base.inherited(wait, model, &game);
        assert_eq!(inherited.values, ["compatible"]);
        assert_eq!(inherited.source, InheritedSource::BuiltIn);
        assert_eq!(base.inherited(boot_mode, model, &game).values, ["v2"]);
    }
}
