//! The editor of a system defaults file `<config-folder>/<system>.conf`:
//! the form of the game editor without the Metadata section. The file may
//! not exist. Its inherited values come from the settings file and the
//! built-in defaults, and it enables every setting of the family.

use machine_factory::config::systems::System;

use super::{
    Editor, EditorFile, OpenError, Subject, SystemFiles, not_compiled, save::read_optional_document,
};
use crate::config::{HostLayers, layers::system_defaults_file};

impl Editor {
    /// An editor on the system defaults file `<config-folder>/<system>.conf`
    /// of `system`. A missing file gives an empty form, and the first save
    /// creates the file. No key is required.
    pub(crate) fn system_defaults(
        host: &HostLayers,
        system: &'static System,
    ) -> Result<Self, OpenError> {
        let files =
            SystemFiles::system_defaults(host, system).ok_or_else(|| not_compiled(system))?;
        let path = system_defaults_file(&host.config_folder(), files.target);
        let (document, stamp) = read_optional_document(&path)?;
        Ok(Self::with_document(
            Subject::SystemDefaults(files),
            host.root(),
            EditorFile::Existing(path),
            document,
            stamp,
        ))
    }
}

#[cfg(test)]
mod tests {
    use gui::ui::{Navigation, Shortcut};
    use machine_factory::config::settings::MACHINE_KEY;

    use super::*;
    use crate::{
        editor::{
            EditedFile, EditorSection,
            form::DEFAULTS_SECTIONS,
            notes::tests::english_inherited_label,
            rows::field_id,
            test_support::{EditorRoot, index_of, show},
        },
        gui_test_support::GuiHarness,
    };

    /// Verifies that the editor of the system defaults has no Metadata section, starts on the model, shows the system in its header and offers the unset entry of the model that inherits the built-in default model.
    #[test]
    fn the_defaults_editor_has_no_metadata_and_an_unset_model() {
        let root = EditorRoot::new("editor_defaults_form");
        let system = root.system();
        let mut editor = Editor::system_defaults(&root.host(), system).expect("the editor opens");
        assert_eq!(editor.edited_file(), EditedFile::SystemDefaults(system));
        assert_eq!(editor.sections(), DEFAULTS_SECTIONS);
        assert!(
            editor
                .fields
                .iter()
                .all(|field| field.section != EditorSection::Metadata)
        );
        assert_eq!(
            editor.path(),
            root.path(&format!("config/{}.conf", system.name))
        );
        assert_eq!(
            editor.edited_file().display_name(),
            format!("{} system defaults", system.label)
        );
        let machine = index_of(&editor, MACHINE_KEY);
        assert!(!editor.fields[machine].required);
        assert!(!editor.fields[machine].is_set());
        assert_eq!(
            english_inherited_label(editor.inherited[machine].as_ref()),
            Some(format!("Built-in default: {}", system.default_model))
        );
        assert!(!editor.is_dirty());

        let mut harness = GuiHarness::new(960, 600);
        show(&mut harness, &mut editor);
        assert_eq!(editor.current_section(), EditorSection::Machine);
        assert_eq!(harness.state.focused(), Some(field_id(MACHINE_KEY)));
        let last = editor.fields[machine].choices.len() - 1;
        editor.fields[machine].select(Some(last));
        editor.refresh();
        assert!(editor.is_dirty());
        harness.input.confirm = true;
        show(&mut harness, &mut editor);
        harness.input.navigation = Some(Navigation::Home);
        show(&mut harness, &mut editor);
        harness.input.confirm = true;
        show(&mut harness, &mut editor);
        assert!(!editor.fields[machine].is_set());
        assert!(!editor.is_dirty());
    }

    /// Verifies that the first save of a missing system defaults file creates it with only the changed keys, without `machine` while the model stays unset, and that a later `machine` joins before the other keys.
    #[test]
    fn a_new_defaults_file_holds_only_the_changed_keys() {
        let root = EditorRoot::new("editor_defaults_save");
        let system = root.system();
        let path = root.path(&format!("config/{}.conf", system.name));
        assert!(!path.exists());
        let mut editor = Editor::system_defaults(&root.host(), system).expect("the editor opens");
        let crt = index_of(&editor, "crt");
        editor.fields[crt].select(Some(0));
        editor.refresh();
        let mut harness = GuiHarness::new(960, 600);
        harness.input.shortcut = Some(Shortcut::ControlS);
        let pass = show(&mut harness, &mut editor);
        assert_eq!(pass.saved, Some(path.clone()));
        assert_eq!(
            std::fs::read_to_string(&path).expect("file reads"),
            "crt = on\n"
        );
        assert_eq!(editor.edited_file(), EditedFile::SystemDefaults(system));
        assert!(!editor.is_dirty());

        let machine = index_of(&editor, MACHINE_KEY);
        editor.fields[machine].select(Some(0));
        editor.refresh();
        let model = editor.fields[machine].choices[0].clone();
        harness.input.shortcut = Some(Shortcut::ControlS);
        assert_eq!(show(&mut harness, &mut editor).saved, Some(path.clone()));
        assert_eq!(
            std::fs::read_to_string(&path).expect("file reads"),
            format!("machine = {model}\ncrt = on\n")
        );
    }

    /// Verifies that the editor of the system defaults keeps the `meta.*` lines of the file as other lines of the file.
    #[test]
    fn the_defaults_editor_keeps_metadata_lines() {
        let root = EditorRoot::new("editor_defaults_meta");
        let system = root.system();
        let path = root.write(
            &format!("config/{}.conf", system.name),
            "meta.title = Defaults\nscaling = nearest\n",
        );
        let mut editor = Editor::system_defaults(&root.host(), system).expect("the editor opens");
        assert_eq!(editor.kept, ["meta.title = Defaults"]);
        let crt = index_of(&editor, "crt");
        editor.fields[crt].select(Some(1));
        assert_eq!(editor.save(false), Ok(path.clone()));
        assert_eq!(
            std::fs::read_to_string(&path).expect("file reads"),
            "meta.title = Defaults\nscaling = nearest\ncrt = off\n"
        );
    }

    /// Verifies that the inherited values of the system defaults editor come from the settings file and the built-in defaults, never from the file itself.
    #[test]
    fn the_defaults_editor_inherits_from_the_settings_file() {
        let root = EditorRoot::new("editor_defaults_inherited");
        let system = root.system();
        root.write("neetan.conf", "crt = off\n");
        root.write(
            &format!("config/{}.conf", system.name),
            "scaling = bilinear\ncrt = on\n",
        );
        let editor = Editor::system_defaults(&root.host(), system).expect("the editor opens");
        let label =
            |key: &str| english_inherited_label(editor.inherited[index_of(&editor, key)].as_ref());
        assert_eq!(label("crt").as_deref(), Some("Settings default: off"));
        assert!(
            label("scaling").is_some_and(|label| label.starts_with("Built-in default: ")),
            "{:?}",
            label("scaling")
        );
        assert_eq!(
            editor.system_files().expect("a system").base.warning(),
            None
        );
    }

    #[cfg(feature = "pc98")]
    /// Verifies that the editor of the system defaults enables a setting that some models ignore and tells its models in the note.
    #[test]
    fn the_defaults_editor_enables_every_key_of_the_family() {
        use machine_factory::config::settings::find_setting;

        let root = EditorRoot::with_system("editor_defaults_limits", "pc98");
        root.write("config/pc98.conf", "machine = PC9801RA\n");
        let system = root.system();
        let cpu_mode = find_setting("cpu-mode").expect("table key");
        let path = root.game("edge", "machine = PC9801RA\n");
        let game = Editor::open(&root.host(), system, &path).expect("the editor opens");
        assert!(game.ignored_by(cpu_mode).is_some());
        assert_eq!(game.extra_note(cpu_mode), None);
        let defaults = Editor::system_defaults(&root.host(), system).expect("the editor opens");
        assert!(defaults.ignored_by(cpu_mode).is_none());
        assert_eq!(
            defaults.extra_note(cpu_mode).as_deref(),
            Some("PC9801F, VM and VX only.")
        );
        assert_eq!(
            defaults.extra_note(find_setting("crt").expect("table key")),
            None
        );
    }
}
