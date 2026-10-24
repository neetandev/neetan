//! The file of the editor: the game file that a save writes, the write of a
//! game file that exists, the first write of a new game or a copy, and the
//! required values that a save writes into a file without them.

use std::path::{Path, PathBuf};

use super::{
    derived_file::DerivedFile,
    fields::Field,
    inherited::Inherited,
    save::{FileStamp, changed_outside_message, create_document, save_document},
};
use crate::{config::ConfigDocument, manager::shown_path};

/// The most derived file names that a save of a new game tries while other
/// files take them.
const MAXIMUM_NEW_FILE_ATTEMPTS: usize = 100;

/// The file that the editor writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EditorFile {
    /// A game file that exists, or that the editor saved once.
    Existing(PathBuf),
    /// A new game whose file derives from its title until the first save.
    New(DerivedFile),
    /// A copy of the game file at the shown path `source`, whose file
    /// derives from its title until the first save.
    Copy {
        /// The shown path of the copied file.
        source: String,
        /// The file of the copy.
        file: DerivedFile,
    },
}

impl EditorFile {
    /// The path of the file.
    pub(crate) fn path(&self) -> PathBuf {
        match self {
            Self::Existing(path) => path.clone(),
            Self::New(file) | Self::Copy { file, .. } => file.path(),
        }
    }

    /// The derived file of a game that the editor has not saved yet.
    pub(crate) fn derived(&self) -> Option<&DerivedFile> {
        match self {
            Self::Existing(_) => None,
            Self::New(file) | Self::Copy { file, .. } => Some(file),
        }
    }
}

/// Why a save did not write the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SaveFailure {
    /// The write failed with this error.
    Error(String),
    /// The game file changed or went away since the editor read it, as this
    /// text says. A save that overwrites the file can follow.
    ChangedOutside(String),
}

impl SaveFailure {
    /// The text of the failure.
    pub(crate) fn message(&self) -> &str {
        match self {
            Self::Error(message) | Self::ChangedOutside(message) => message,
        }
    }
}

/// Gives every required field of `fields` without a value its inherited
/// value from `inherited`, which holds the inherited value of each field.
pub(crate) fn fill_required(fields: &mut [Field], inherited: &[Option<Inherited>]) {
    for (field, inherited) in fields.iter_mut().zip(inherited) {
        if !field.required || field.is_set() {
            continue;
        }
        let index = inherited
            .as_ref()
            .and_then(|inherited| inherited.values.first())
            .and_then(|value| field.choices.iter().position(|choice| choice == value));
        if let Some(index) = index {
            field.select(Some(index));
        }
    }
}

/// A game file that a save wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WrittenFile {
    /// The path of the file.
    pub(crate) path: PathBuf,
    /// The stamp of the written bytes, taken from the written file itself.
    pub(crate) stamp: Option<FileStamp>,
}

/// Writes `document` into `file` and returns the written file.
///
/// A game file whose stamp differs from `stamp` changed since the editor
/// read it and stays as it is, unless `overwrite` asks to write over the
/// change. A new game or a copy takes the file name that derives from
/// `title` at the time of the write. Its write never replaces a file: a
/// file that takes the name first gives the next suffix. `root` gives the
/// shown path of a message.
pub(crate) fn write_game_file(
    file: &EditorFile,
    document: &ConfigDocument,
    stamp: Option<FileStamp>,
    title: &str,
    root: &Path,
    overwrite: bool,
) -> Result<WrittenFile, SaveFailure> {
    match file {
        EditorFile::Existing(path) => {
            if !overwrite && FileStamp::of(path) != stamp {
                return Err(SaveFailure::ChangedOutside(changed_outside_message(
                    &shown_path(path, root),
                )));
            }
            let stamp = save_document(document, path).map_err(SaveFailure::Error)?;
            Ok(WrittenFile {
                path: path.clone(),
                stamp,
            })
        }
        EditorFile::New(derived) | EditorFile::Copy { file: derived, .. } => {
            for _ in 0..MAXIMUM_NEW_FILE_ATTEMPTS {
                let path = DerivedFile::new(&derived.folder, title).path();
                match create_document(document, &path) {
                    Ok(stamp) => return Ok(WrittenFile { path, stamp }),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => {
                        return Err(SaveFailure::Error(format!("{}: {error}", path.display())));
                    }
                }
            }
            Err(SaveFailure::Error(format!(
                "{}: every derived file name exists",
                derived.folder.display()
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use machine_factory::config::settings::{MACHINE_KEY, find_setting};

    use super::*;
    use crate::editor::{
        form::{EditorSection, FieldSource},
        inherited::InheritedSource,
        test_support::EditorRoot,
    };

    /// The required machine field of a file without `machine`.
    fn machine_field(root: &EditorRoot) -> Field {
        Field::read(
            EditorSection::Machine,
            FieldSource::Setting(find_setting(MACHINE_KEY).expect("table key")),
            root.target(),
            &ConfigDocument::new(),
            true,
        )
    }

    /// Verifies that only a required field without a value takes its inherited value.
    #[test]
    fn required_fields_take_their_inherited_value() {
        let root = EditorRoot::new("file_required");
        let model = root.system().default_model;
        let inherited = Some(Inherited {
            values: vec![model.to_owned()],
            source: InheritedSource::BuiltIn,
        });
        let mut fields = vec![machine_field(&root), machine_field(&root)];
        fields[1].required = false;
        fill_required(&mut fields, &[inherited.clone(), inherited.clone()]);
        assert_eq!(fields[0].chosen(), Some(model));
        assert_eq!(fields[1].chosen(), None);

        let mut chosen = vec![machine_field(&root)];
        let other = chosen[0].choices.len() - 1;
        chosen[0].select(Some(other));
        fill_required(&mut chosen, &[inherited]);
        assert_eq!(chosen[0].selected(), Some(other));
    }

    /// Verifies that a file that changed since the editor read it stays as it is, and that an overwrite writes over the change.
    #[test]
    fn a_changed_file_stays_unless_the_save_overwrites_it() {
        let root = EditorRoot::new("file_changed");
        let path = root.game("edge", "meta.title = Edge\n");
        let stamp = FileStamp::of(&path);
        let file = EditorFile::Existing(path.clone());
        let document = ConfigDocument::parse("meta.title = Edge 2\n");
        std::fs::write(&path, "meta.title = Changed outside\ncrt = on\n").expect("written");
        let failure = write_game_file(&file, &document, stamp, "Edge 2", &root.root, false)
            .expect_err("the file changed");
        let shown = format!("config/{}/edge.conf", root.system().name);
        assert_eq!(
            failure,
            SaveFailure::ChangedOutside(changed_outside_message(&shown))
        );
        assert!(failure.message().starts_with(&shown));
        assert_eq!(
            std::fs::read_to_string(&path).expect("file reads"),
            "meta.title = Changed outside\ncrt = on\n"
        );
        let written = write_game_file(&file, &document, stamp, "Edge 2", &root.root, true)
            .expect("the overwrite writes");
        assert_eq!(written.path, path);
        assert_eq!(written.stamp, FileStamp::of(&path));
        assert!(written.stamp.is_some());
        assert_eq!(
            std::fs::read_to_string(&path).expect("file reads"),
            "meta.title = Edge 2\n"
        );
        let again = write_game_file(&file, &document, written.stamp, "Edge 2", &root.root, false)
            .expect("the stamp of the write matches the file");
        assert_eq!(again.stamp, FileStamp::of(&path));
    }

    /// Verifies that the first write of a new game takes the name of its title at the time of the write and the next suffix of a taken name.
    #[test]
    fn a_new_game_takes_the_next_free_name() {
        let root = EditorRoot::new("file_new");
        let folder = root.path(&format!("config/{}", root.system().name));
        let file = EditorFile::New(DerivedFile::new(&folder, ""));
        assert_eq!(
            file.derived().map(|derived| derived.path()),
            Some(file.path())
        );
        root.game("edge", "");
        let document = ConfigDocument::parse("meta.title = Edge\n");
        let written = write_game_file(&file, &document, None, "Edge", &root.root, false)
            .expect("the file is new");
        assert_eq!(written.path, folder.join("edge-2.conf"));
        assert_eq!(written.stamp, FileStamp::of(&written.path));
        assert!(written.stamp.is_some());
        assert_eq!(
            std::fs::read_to_string(folder.join("edge.conf")).expect("file reads"),
            ""
        );
        root.write("blocked", "");
        let blocked = EditorFile::New(DerivedFile::new(&root.path("blocked/pc98"), ""));
        let failure = write_game_file(&blocked, &document, None, "Edge", &root.root, false)
            .expect_err("the folder is a file");
        assert!(matches!(failure, SaveFailure::Error(_)), "{failure:?}");
        assert_eq!(EditorFile::Existing(written.path).derived(), None);
    }
}
