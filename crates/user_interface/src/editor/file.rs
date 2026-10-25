//! The file of the editor that a save writes: the write of a file at a
//! known path, the first write of a new game or a copy, and the required
//! values that a save writes into a file without them.

use std::{
    fmt,
    path::{Path, PathBuf},
};

use i18n::Catalog;

use super::{
    derived_file::DerivedFile,
    fields::Field,
    inherited::Inherited,
    save::{FileStamp, create_document, save_document},
};
use crate::{
    config::ConfigDocument,
    manager::shown_path,
    texts::{english, errors},
};

/// The most derived file names that a save of a new game tries while other
/// files take them.
const MAXIMUM_NEW_FILE_ATTEMPTS: usize = 100;

/// The file that the editor writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EditorFile {
    /// A file at a known path: a game file, a system defaults file or the
    /// settings file. A file that did not exist when the editor read it
    /// has no stamp, and its first save creates it.
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

/// The reason that a save did not write the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SaveFailure {
    /// The write failed.
    Error {
        /// The path of the file.
        path: PathBuf,
        /// The error of the operating system in English.
        error: String,
    },
    /// The file at this shown path changed, went away or appeared since
    /// the editor read it. A save that overwrites the file can follow.
    ChangedOutside(String),
    /// Other files take every derived file name of a new game in this
    /// folder.
    NoFreeName(PathBuf),
}

impl SaveFailure {
    /// The failure of the write of the file at `path` with `error`.
    pub(crate) fn error(path: &Path, error: &dyn fmt::Display) -> Self {
        Self::Error {
            path: path.to_path_buf(),
            error: error.to_string(),
        }
    }

    /// Writes the text of the failure in `catalog`: the error of the write,
    /// the sentence about a file that changed outside the manager, or the
    /// sentence about the taken file names.
    pub(crate) fn write(&self, catalog: &Catalog, out: &mut impl fmt::Write) -> fmt::Result {
        match self {
            Self::Error { path, error } => {
                catalog.write(out, errors::path_error(&path.display(), error))
            }
            Self::ChangedOutside(shown) => {
                catalog.write(out, errors::changed_outside_detail(shown))
            }
            Self::NoFreeName(folder) => catalog.write(out, errors::no_free_name(&folder.display())),
        }
    }
}

impl fmt::Display for SaveFailure {
    /// Writes the text of the failure in English.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write(english(), formatter)
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

/// A file that a save wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WrittenFile {
    /// The path of the file.
    pub(crate) path: PathBuf,
    /// The stamp of the written bytes, taken from the written file itself.
    pub(crate) stamp: Option<FileStamp>,
}

/// Writes `document` into `file` and returns the written file.
///
/// A file whose stamp differs from `stamp` changed since the editor read it
/// and stays as it is, unless `overwrite` asks to write over the change. A
/// file without a stamp did not exist when the editor read it. Its first
/// write never replaces a file that appeared since. A new game or a copy
/// takes the file name that derives from `title` at the time of the write.
/// Its write never replaces a file: a file that takes the name first gives
/// the next suffix. `root` gives the shown path of a message.
pub(crate) fn write_file(
    file: &EditorFile,
    document: &ConfigDocument,
    stamp: Option<FileStamp>,
    title: &str,
    root: &Path,
    overwrite: bool,
) -> Result<WrittenFile, SaveFailure> {
    match file {
        EditorFile::Existing(path) => {
            let changed_outside = || SaveFailure::ChangedOutside(shown_path(path, root));
            if !overwrite && FileStamp::of(path) != stamp {
                return Err(changed_outside());
            }
            let stamp = if overwrite || stamp.is_some() {
                save_document(document, path)?
            } else {
                create_document(document, path).map_err(|error| {
                    if error.kind() == std::io::ErrorKind::AlreadyExists {
                        changed_outside()
                    } else {
                        SaveFailure::error(path, &error)
                    }
                })?
            };
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
                        return Err(SaveFailure::error(&path, &error));
                    }
                }
            }
            Err(SaveFailure::NoFreeName(derived.folder.clone()))
        }
    }
}

#[cfg(test)]
mod tests {
    use machine_factory::config::settings::{Families, MACHINE_KEY, find_setting};

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
            Families::of(root.target()),
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
        let failure = write_file(&file, &document, stamp, "Edge 2", &root.root, false)
            .expect_err("the file changed");
        let shown = format!("config/{}/edge.conf", root.system().name);
        assert_eq!(failure, SaveFailure::ChangedOutside(shown.clone()));
        assert!(failure.to_string().starts_with(&shown));
        assert_eq!(
            std::fs::read_to_string(&path).expect("file reads"),
            "meta.title = Changed outside\ncrt = on\n"
        );
        let written = write_file(&file, &document, stamp, "Edge 2", &root.root, true)
            .expect("the overwrite writes");
        assert_eq!(written.path, path);
        assert_eq!(written.stamp, FileStamp::of(&path));
        assert!(written.stamp.is_some());
        assert_eq!(
            std::fs::read_to_string(&path).expect("file reads"),
            "meta.title = Edge 2\n"
        );
        let again = write_file(&file, &document, written.stamp, "Edge 2", &root.root, false)
            .expect("the stamp of the write matches the file");
        assert_eq!(again.stamp, FileStamp::of(&path));
    }

    /// Verifies that the first write of a file without a stamp creates it, that a file that appeared since the read stays until an overwrite, and that the stamp of the write follows the file.
    #[test]
    fn a_file_that_appeared_stays_until_an_overwrite() {
        let root = EditorRoot::new("file_defaults");
        let path = root.path(&format!("config/{}.conf", root.system().name));
        let file = EditorFile::Existing(path.clone());
        assert_eq!(file.derived(), None);
        assert_eq!(file.path(), path);
        let document = ConfigDocument::parse("crt = on\n");
        let written =
            write_file(&file, &document, None, "", &root.root, false).expect("the file is new");
        assert_eq!(written.path, path);
        assert_eq!(written.stamp, FileStamp::of(&path));
        assert_eq!(
            std::fs::read_to_string(&path).expect("file reads"),
            "crt = on\n"
        );

        std::fs::remove_file(&path).expect("file is removed");
        assert!(std::fs::symlink_metadata(&path).is_err());
        root.write(
            &format!("config/{}.conf", root.system().name),
            "scaling = nearest\n",
        );
        let failure =
            write_file(&file, &document, None, "", &root.root, false).expect_err("a file appeared");
        assert!(
            matches!(failure, SaveFailure::ChangedOutside(_)),
            "{failure:?}"
        );
        assert_eq!(
            std::fs::read_to_string(&path).expect("file reads"),
            "scaling = nearest\n"
        );
        write_file(&file, &document, None, "", &root.root, true).expect("the overwrite writes");
        assert_eq!(
            std::fs::read_to_string(&path).expect("file reads"),
            "crt = on\n"
        );
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
        let written =
            write_file(&file, &document, None, "Edge", &root.root, false).expect("the file is new");
        assert_eq!(written.path, folder.join("edge-2.conf"));
        assert_eq!(written.stamp, FileStamp::of(&written.path));
        assert!(written.stamp.is_some());
        assert_eq!(
            std::fs::read_to_string(folder.join("edge.conf")).expect("file reads"),
            ""
        );
        root.write("blocked", "");
        let blocked = EditorFile::New(DerivedFile::new(&root.path("blocked/pc98"), ""));
        let failure = write_file(&blocked, &document, None, "Edge", &root.root, false)
            .expect_err("the folder is a file");
        assert!(matches!(failure, SaveFailure::Error { .. }), "{failure:?}");
        assert_eq!(EditorFile::Existing(written.path).derived(), None);
    }
}
