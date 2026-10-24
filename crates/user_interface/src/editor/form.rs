//! The form of the game editor: its sections and the fields that the
//! settings table generates for the family of a game.

use machine_factory::config::{
    MachineSelection, Target,
    phrases::join_names,
    settings::{SETTINGS, Scope, Section, Setting, model_limits::ModelLimit},
};

use crate::config::metadata::{METADATA_FIELDS, MetadataField};

/// A section of the editor, one tab of its tab bar.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) enum EditorSection {
    /// The `meta.*` keys.
    Metadata,
    /// Model, CPU, boot and memory settings.
    Machine,
    /// Disk, disc, cartridge and cassette images.
    Media,
    /// Sound boards and MIDI.
    Sound,
    /// Screen presentation and video hardware.
    Display,
    /// Printer, text extractor and the lines of the file that the editor keeps as written.
    Advanced,
}

/// Every section of the editor in tab order.
pub(crate) const EDITOR_SECTIONS: [EditorSection; 6] = [
    EditorSection::Metadata,
    EditorSection::Machine,
    EditorSection::Media,
    EditorSection::Sound,
    EditorSection::Display,
    EditorSection::Advanced,
];

impl EditorSection {
    /// The caption of the tab of the section.
    pub(crate) fn caption(self) -> &'static str {
        match self {
            Self::Metadata => "Metadata",
            Self::Machine => "Machine",
            Self::Media => "Media",
            Self::Sound => "Sound",
            Self::Display => "Display",
            Self::Advanced => "Advanced",
        }
    }

    /// The section of the settings table that fills this section. The
    /// metadata section has none.
    pub(crate) fn table_section(self) -> Option<Section> {
        match self {
            Self::Metadata => None,
            Self::Machine => Some(Section::Machine),
            Self::Media => Some(Section::Media),
            Self::Sound => Some(Section::Sound),
            Self::Display => Some(Section::Display),
            Self::Advanced => Some(Section::Advanced),
        }
    }

    /// The sentence of a section without fields on the system `label`, for
    /// example `The PC-88 has no sound settings.`
    pub(crate) fn empty_text(self, label: &str) -> String {
        format!(
            "The {label} has no {} settings.",
            self.caption().to_lowercase()
        )
    }
}

/// The source of one field of the form.
#[derive(Debug, Copy, Clone)]
pub(crate) enum FieldSource {
    /// A `meta.*` key.
    Metadata(MetadataField),
    /// A key of the settings table.
    Setting(&'static Setting),
}

impl FieldSource {
    /// The key of the field.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Metadata(field) => field.key,
            Self::Setting(setting) => setting.key,
        }
    }

    /// The label of the field.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Metadata(field) => field.label,
            Self::Setting(setting) => setting.label,
        }
    }

    /// The settings table entry of the field. A metadata field has none.
    pub(crate) fn setting(self) -> Option<&'static Setting> {
        match self {
            Self::Metadata(_) => None,
            Self::Setting(setting) => Some(setting),
        }
    }
}

/// Returns the settings of `section` that a game file of the family `target`
/// sets, in the order of the settings table.
///
/// A game file sets the keys of the system and the game scope. A setting
/// outside every section, such as a debug setting, has no field.
pub(crate) fn section_settings(section: Section, target: Target) -> Vec<&'static Setting> {
    SETTINGS
        .iter()
        .filter(|setting| {
            setting.scope != Scope::Host
                && setting.section == Some(section)
                && setting.families.contains_target(target)
        })
        .collect()
}

/// Returns every field of the form of a game of the family `target`, with
/// its section, in tab order.
pub(crate) fn form_fields(target: Target) -> Vec<(EditorSection, FieldSource)> {
    let mut fields: Vec<(EditorSection, FieldSource)> = METADATA_FIELDS
        .iter()
        .map(|field| (EditorSection::Metadata, FieldSource::Metadata(*field)))
        .collect();
    for section in EDITOR_SECTIONS {
        if let Some(table_section) = section.table_section() {
            fields.extend(
                section_settings(table_section, target)
                    .into_iter()
                    .map(|setting| (section, FieldSource::Setting(setting))),
            );
        }
    }
    fields
}

/// Splits a model name into its family prefix up to its last digit and the
/// rest, for example `PC9801` and `VX`. A name without a digit, or with
/// nothing after its last digit, has no rest.
fn split_model_name(name: &str) -> (&str, &str) {
    match name.rfind(|character: char| character.is_ascii_digit()) {
        Some(end) => name.split_at(end + 1),
        None => (name, ""),
    }
}

/// Returns `models` for a note. Names with one family prefix keep the first
/// name whole and drop the prefix from the others, for example `PC9821AS`
/// and `AP`. Names with several prefixes drop the prefix from every name,
/// for example `VX`, `RS`, `RA`, `AS` and `AP` for the PC-9801 and PC-9821
/// models. The names stay whole when a name would lose all of its letters or
/// start with a digit.
pub(crate) fn short_model_names(models: &[&'static str]) -> Vec<&'static str> {
    let Some(first) = models.first() else {
        return Vec::new();
    };
    let parts: Vec<(&'static str, &'static str)> =
        models.iter().map(|name| split_model_name(name)).collect();
    let shortens = parts
        .iter()
        .all(|(_, rest)| rest.starts_with(|character: char| character.is_ascii_alphabetic()));
    if !shortens || models.len() < 2 {
        return models.to_vec();
    }
    let one_prefix = parts.iter().all(|(prefix, _)| *prefix == parts[0].0);
    if one_prefix {
        std::iter::once(*first)
            .chain(parts.iter().skip(1).map(|(_, rest)| *rest))
            .collect()
    } else {
        parts.iter().map(|(_, rest)| *rest).collect()
    }
}

/// The note of a setting that the model `model` ignores under `limit`, for
/// example `PC9801RA has no CD-ROM drive. PC9821AS and AP only.`
pub(crate) fn disabled_note(model: MachineSelection, limit: &ModelLimit) -> String {
    format!(
        "{} {}. {} only.",
        model.name(),
        limit.reason.phrase(model),
        join_names(&short_model_names(limit.models), "and")
    )
}

/// Returns `None` when the model `model` reads `setting`, else the note that
/// tells why the model ignores it.
pub(crate) fn ignored_note(setting: &Setting, model: MachineSelection) -> Option<String> {
    let target = model.target();
    if setting.is_read_by(target, model.name()) {
        return None;
    }
    setting
        .limit_for(target)
        .map(|limit| disabled_note(model, limit))
}

#[cfg(test)]
mod tests {
    use machine_factory::config::{
        settings::{MACHINE_KEY, MODEL_CHOICES},
        systems::compiled_systems,
    };

    use super::*;

    /// The keys of the fields of `section` for the family `target`.
    fn keys(target: Target, section: EditorSection) -> Vec<&'static str> {
        form_fields(target)
            .into_iter()
            .filter(|(field_section, _)| *field_section == section)
            .map(|(_, source)| source.key())
            .collect()
    }

    /// The model `name`.
    fn model(name: &str) -> MachineSelection {
        name.parse().expect("a compiled model")
    }

    /// The model names of the family `target`.
    fn models_of(target: Target) -> Vec<&'static str> {
        MODEL_CHOICES
            .iter()
            .filter(|choice| choice.families.contains_target(target))
            .map(|choice| choice.value)
            .collect()
    }

    /// Verifies the fields of every compiled family: the metadata keys
    /// first, the machine at the top of its section, no host key and only
    /// keys that the family reads.
    #[test]
    fn every_family_gets_its_own_fields() {
        for system in compiled_systems() {
            let target = MachineSelection::of_system(system)
                .expect("compiled system")
                .target();
            assert_eq!(
                keys(target, EditorSection::Metadata),
                [
                    "meta.title",
                    "meta.title-original",
                    "meta.developer",
                    "meta.publisher",
                    "meta.year",
                    "meta.genre",
                    "meta.notes"
                ]
            );
            assert_eq!(
                keys(target, EditorSection::Machine).first(),
                Some(&MACHINE_KEY),
                "{}",
                system.name
            );
            for (section, source) in form_fields(target) {
                let Some(setting) = source.setting() else {
                    continue;
                };
                assert_ne!(setting.scope, Scope::Host, "{}", setting.key);
                assert!(setting.families.contains_target(target), "{}", setting.key);
                assert_eq!(section.table_section(), setting.section, "{}", setting.key);
            }
            for key in ["debug-bios", "backend", "pc98-roms", "config-folder"] {
                assert!(
                    form_fields(target)
                        .iter()
                        .all(|(_, source)| source.key() != key),
                    "{key} on {}",
                    system.name
                );
            }
            assert!(
                keys(target, EditorSection::Display).contains(&"scaling"),
                "{}",
                system.name
            );
        }
    }

    #[cfg(feature = "pc98")]
    /// Verifies the sections of the PC-98 and the notes of its model limits.
    #[test]
    fn the_pc98_form_follows_the_design() {
        let target = Target::Pc98;
        assert_eq!(
            keys(target, EditorSection::Machine),
            ["machine", "cpu-mode", "boot-device", "bios", "ems", "xms"]
        );
        assert_eq!(
            keys(target, EditorSection::Media),
            ["fdd1", "fdd2", "cdrom", "hdd1", "hdd2"]
        );
        assert_eq!(
            keys(target, EditorSection::Sound),
            ["soundboard", "adpcm-ram", "midi"]
        );
        assert_eq!(
            keys(target, EditorSection::Display),
            [
                "aspect-mode",
                "window-mode",
                "crt",
                "scaling",
                "force-gdc-clock",
                "graphicboard"
            ]
        );
        assert_eq!(
            keys(target, EditorSection::Advanced),
            ["printer", "enable-extractor"]
        );
        let cdrom = machine_factory::config::settings::find_setting("cdrom").expect("key");
        assert_eq!(
            ignored_note(cdrom, model("PC9801RA")).as_deref(),
            Some("PC9801RA has no CD-ROM drive. PC9821AS and AP only.")
        );
        assert_eq!(ignored_note(cdrom, model("PC9821AP")), None);
        let cpu_mode = machine_factory::config::settings::find_setting("cpu-mode").expect("key");
        assert_eq!(
            ignored_note(cpu_mode, model("PC9801RA")).as_deref(),
            Some("PC9801RA runs at a fixed 20 MHz. PC9801F, VM and VX only.")
        );
    }

    #[cfg(feature = "pc60")]
    /// Verifies the media of the PC-6000 and the floppy drives that only the PC-6601 models have.
    #[test]
    fn the_pc6000_form_limits_its_floppy_drives() {
        let target = Target::Pc60;
        assert_eq!(
            keys(target, EditorSection::Media),
            ["fdd1", "fdd2", "cartridge", "cassette"]
        );
        assert!(keys(target, EditorSection::Sound).is_empty());
        let fdd1 = machine_factory::config::settings::find_setting("fdd1").expect("key");
        assert_eq!(
            ignored_note(fdd1, model("PC6001MK2SR")).as_deref(),
            Some("PC6001MK2SR has no floppy drive. PC6601 and PC6601SR only.")
        );
        assert_eq!(ignored_note(fdd1, model("PC6601SR")), None);
    }

    /// Verifies that every model limit disables its key on the other models of the family with a note, and enables it on its own models.
    #[test]
    fn model_limits_disable_the_other_models_with_a_note() {
        for system in compiled_systems() {
            let target = MachineSelection::of_system(system)
                .expect("compiled system")
                .target();
            for (_, source) in form_fields(target) {
                let Some(setting) = source.setting() else {
                    continue;
                };
                for model in models_of(target) {
                    let note = ignored_note(setting, self::model(model));
                    let limited = setting
                        .limit_for(target)
                        .is_some_and(|limit| !limit.models.contains(&model));
                    assert_eq!(note.is_some(), limited, "{} on {model}", setting.key);
                    if let Some(note) = note {
                        assert!(note.starts_with(model), "{note}");
                        assert!(note.ends_with(" only."), "{note}");
                    }
                }
            }
        }
    }

    /// Verifies that the model names of a note drop the start they share up to a digit only where a letter follows.
    #[test]
    fn model_names_shorten_after_their_shared_digits() {
        assert_eq!(
            short_model_names(&["PC9821AS", "PC9821AP"]),
            ["PC9821AS", "AP"]
        );
        assert_eq!(
            short_model_names(&["PC9801F", "PC9801VM", "PC9801VX"]),
            ["PC9801F", "VM", "VX"]
        );
        assert_eq!(
            short_model_names(&["PC6601", "PC6601SR"]),
            ["PC6601", "PC6601SR"]
        );
        assert_eq!(
            short_model_names(&["PC9801VX", "PC9801RS", "PC9801RA", "PC9821AS", "PC9821AP"]),
            ["VX", "RS", "RA", "AS", "AP"]
        );
        assert_eq!(
            short_model_names(&["X68000SUPER", "X68000XVI"]),
            ["X68000SUPER", "XVI"]
        );
        assert_eq!(
            short_model_names(&["FMTownsIICX", "FMTownsIIMX"]),
            ["FMTownsIICX", "FMTownsIIMX"]
        );
        assert_eq!(short_model_names(&["X1TURBO"]), ["X1TURBO"]);
        assert!(short_model_names(&[]).is_empty());
    }

    /// Verifies the sentence of a section without fields.
    #[test]
    fn an_empty_section_tells_its_system() {
        assert_eq!(
            EditorSection::Sound.empty_text("PC-88"),
            "The PC-88 has no sound settings."
        );
    }
}
