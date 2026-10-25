//! The file of a new or duplicated game: a name that derives from the title
//! until the first save, and the note that explains it.

use std::path::{Path, PathBuf};

use i18n::Catalog;

use crate::{
    catalog::{is_reserved_stem, new_game_file_name, title_file_stem},
    config::CONFIG_FILE_EXTENSION,
    texts::editor,
};

/// The reason for the form of a derived file name. The note beside the name shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DerivedNote {
    /// The name follows the title without a suffix.
    Follows,
    /// The stem is a device name of Windows and the file takes the suffix.
    DeviceName {
        /// The stem that derives from the title.
        stem: String,
        /// The number of the suffix.
        suffix: String,
    },
    /// A file takes the plain name and the name takes the suffix.
    FileExists {
        /// The plain file name.
        file: String,
        /// The number of the suffix.
        suffix: String,
    },
}

impl DerivedNote {
    /// Writes the note in `catalog`.
    pub(crate) fn write(&self, catalog: &Catalog, out: &mut String) {
        let _ = match self {
            Self::Follows => {
                out.push_str(catalog.text(editor::DERIVED_NOTE));
                Ok(())
            }
            Self::DeviceName { stem, suffix } => {
                catalog.write(out, editor::device_name_note(stem, suffix))
            }
            Self::FileExists { file, suffix } => {
                catalog.write(out, editor::file_exists_note(file, suffix))
            }
        };
    }
}

/// The file of a game that the editor has not saved yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DerivedFile {
    /// The folder of the system, `<config-folder>/<system>`.
    pub(crate) folder: PathBuf,
    /// The file name, for example `brandish-copy-2.conf`.
    pub(crate) file_name: String,
    /// The note beside the file name.
    pub(crate) note: DerivedNote,
}

impl DerivedFile {
    /// The file in `folder` for the title `title`.
    ///
    /// The name derives from the title as `title_file_stem` says. A name of
    /// an existing file and a name that Windows reserves for a device take
    /// the suffix `-2`, `-3` and so on, and the note tells why.
    pub(crate) fn new(folder: &Path, title: &str) -> Self {
        let file_name = new_game_file_name(folder, title);
        let stem = title_file_stem(title);
        let plain = format!("{stem}.{CONFIG_FILE_EXTENSION}");
        let suffix = file_name
            .strip_suffix(&format!(".{CONFIG_FILE_EXTENSION}"))
            .and_then(|name| name.strip_prefix(&format!("{stem}-")))
            .filter(|_| file_name != plain);
        let note = match suffix {
            Some(suffix) if is_reserved_stem(&stem) => DerivedNote::DeviceName {
                stem: stem.clone(),
                suffix: suffix.to_owned(),
            },
            Some(suffix) => DerivedNote::FileExists {
                file: plain.clone(),
                suffix: suffix.to_owned(),
            },
            None => DerivedNote::Follows,
        };
        Self {
            folder: folder.to_path_buf(),
            file_name,
            note,
        }
    }

    /// The path of the file.
    pub(crate) fn path(&self) -> PathBuf {
        self.folder.join(&self.file_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{editor::test_support::EditorRoot, texts::english_text};

    /// The English note of `file`.
    fn note(file: &DerivedFile) -> String {
        english_text(|catalog, out| file.note.write(catalog, out))
    }

    /// Verifies the names and the notes of the examples of the design document.
    #[test]
    fn derived_files_follow_the_design_examples() {
        let root = EditorRoot::new("derived_file");
        let folder = root.path("config/pc98");
        let file = DerivedFile::new(&folder, "Brandish 2: The Planet Buster");
        assert_eq!(file.file_name, "brandish-2-the-planet-buster.conf");
        assert_eq!(
            note(&file),
            "The name follows the title until the first save."
        );
        assert_eq!(
            file.path(),
            folder.join("brandish-2-the-planet-buster.conf")
        );

        root.write("config/pc98/brandish-copy.conf", "");
        let file = DerivedFile::new(&folder, "Brandish (copy)");
        assert_eq!(file.file_name, "brandish-copy-2.conf");
        assert_eq!(
            note(&file),
            "The file brandish-copy.conf exists, so the name takes -2."
        );

        let file = DerivedFile::new(&folder, "エッジ");
        assert_eq!(file.file_name, "game.conf");
        root.write("config/pc98/game.conf", "");
        root.write("config/pc98/game-2.conf", "");
        let file = DerivedFile::new(&folder, "エッジ");
        assert_eq!(file.file_name, "game-3.conf");
        assert_eq!(
            note(&file),
            "The file game.conf exists, so the name takes -3."
        );
        let file = DerivedFile::new(&folder, "");
        assert_eq!(file.file_name, "game-3.conf");

        let file = DerivedFile::new(&folder, "CON");
        assert_eq!(file.file_name, "con-2.conf");
        assert_eq!(
            note(&file),
            "The name con is a device name of Windows, so the file takes -2."
        );
        root.write("config/pc98/con-2.conf", "");
        let file = DerivedFile::new(&folder, "Con");
        assert_eq!(file.file_name, "con-3.conf");
        assert_eq!(
            note(&file),
            "The name con is a device name of Windows, so the file takes -3."
        );
    }
}
