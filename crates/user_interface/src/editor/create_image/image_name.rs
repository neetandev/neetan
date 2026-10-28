//! The name of a new image in the create image dialog: the default name of a drive, the rule
//! by which an edited name follows the extension of a new image type, and the check of a name
//! in its folder.

use std::path::Path;

use gui::ui::TextValue;
use machine_factory::config::settings::blank_images::{BlankImage, is_blank_image_extension};

use crate::catalog::{MAXIMUM_FILE_STEM_LENGTH, free_file_name, is_reserved_stem, title_file_stem};

/// The state of the name in the folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NameCheck {
    /// The image can be created.
    Valid,
    /// The name is empty.
    EmptyName,
    /// The name holds a folder separator or a drive colon, or it is `.` or `..`.
    FolderInName,
    /// The part of the name before its first dot is a device name that Windows reserves, such
    /// as `CON`.
    ReservedName,
    /// The extension of the name is none of the extensions of the selected image.
    WrongExtension,
    /// The folder does not exist.
    MissingFolder,
    /// A file of the name exists in the folder.
    FileExists,
}

impl TextValue for NameCheck {}

impl NameCheck {
    /// Whether the name field shows the check as an error of its value.
    pub(crate) fn marks_field(self) -> bool {
        match self {
            Self::EmptyName
            | Self::FolderInName
            | Self::ReservedName
            | Self::WrongExtension
            | Self::FileExists => true,
            Self::Valid | Self::MissingFolder => false,
        }
    }
}

/// Returns the stem of the default name of the drive `key` of the game file with the stem
/// `file_stem`, for example `yu-no-hdd1`. A long game stem ends at its last hyphen within the
/// room that leaves `-{key}` inside [`MAXIMUM_FILE_STEM_LENGTH`], or at the edge of the room
/// when it has no hyphen there.
pub(crate) fn name_stem(file_stem: &str, key: &str) -> String {
    let base = title_file_stem(file_stem);
    let room = MAXIMUM_FILE_STEM_LENGTH.saturating_sub(key.len() + 1);
    let mut cut = base.as_str();
    if cut.len() > room {
        let word_ends_at_room = cut.as_bytes()[room] == b'-';
        cut = &cut[..room];
        if !word_ends_at_room && let Some(hyphen) = cut.rfind('-') {
            cut = &cut[..hyphen];
        }
    }
    format!("{}-{key}", cut.trim_end_matches('-'))
}

/// Returns the default name of `image` in `folder`: the stem `stem` with the default extension
/// of the image. A stem that a file of the folder holds takes the suffix `-2`, `-3` and so on.
pub(crate) fn default_name(folder: &Path, stem: &str, image: &BlankImage) -> String {
    free_file_name(
        folder,
        stem,
        image.extensions.first().copied().unwrap_or_default(),
    )
}

/// Returns the edited name `name` after a change of the type to `image`. A trimmed name with
/// the extension of a blank image takes the default extension of `image`, unless `image` takes
/// its extension. Any other name stays as typed and gives `None`.
pub(crate) fn followed_name(name: &str, image: &BlankImage) -> Option<String> {
    let (stem, extension) = name.trim().rsplit_once('.')?;
    if !is_blank_image_extension(extension) || image.takes_extension(extension) {
        return None;
    }
    let default = image.extensions.first()?;
    Some(format!("{stem}.{default}"))
}

/// Checks the name `name` of a new image of `image` in `folder`. The trimmed name gives the
/// first failure in the order of the variants of [`NameCheck`]. `/`, `\\` and the drive colon `:`
/// count as folder separators on every platform. A reserved device name counts on every platform
/// too.
pub(crate) fn check_name(name: &str, folder: &Path, image: &BlankImage) -> NameCheck {
    let name = name.trim();
    if name.is_empty() {
        return NameCheck::EmptyName;
    }
    if name.contains(['/', '\\', ':']) || name == "." || name == ".." {
        return NameCheck::FolderInName;
    }
    let device = name.split('.').next().unwrap_or(name).trim_end();
    if is_reserved_stem(&device.to_ascii_lowercase()) {
        return NameCheck::ReservedName;
    }
    let extension = Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str());
    if !extension.is_some_and(|extension| image.takes_extension(extension)) {
        return NameCheck::WrongExtension;
    }
    if !folder.is_dir() {
        return NameCheck::MissingFolder;
    }
    if std::fs::symlink_metadata(folder.join(name)).is_ok() {
        return NameCheck::FileExists;
    }
    NameCheck::Valid
}

#[cfg(test)]
mod tests {
    use common::temporary::TemporaryFolder;
    use device::{disk::HddSizeType, floppy::FddType};
    use machine_factory::config::settings::blank_images::BlankFormat;

    use super::*;

    /// A PC-98 SASI hard disk.
    const SASI: BlankImage = BlankImage {
        format: BlankFormat::HardDisk(HddSizeType::Mb20),
        extensions: &["hdi"],
    };

    /// An FM Towns SCSI hard disk.
    const TOWNS_SCSI: BlankImage = BlankImage {
        format: BlankFormat::HardDisk(HddSizeType::ScsiMb100),
        extensions: &["h0", "h1", "h2", "h3", "h4"],
    };

    /// A raw MSX floppy disk.
    const MSX_DSK: BlankImage = BlankImage {
        format: BlankFormat::Floppy(FddType::Dd2Fmt720),
        extensions: &["dsk"],
    };

    /// A D88 MSX floppy disk.
    const MSX_D88: BlankImage = BlankImage {
        format: BlankFormat::Floppy(FddType::Dd2Fmt720),
        extensions: &["d88"],
    };

    /// Verifies the stem of the default name from the stem of the game file and the drive key.
    #[test]
    fn the_stem_joins_the_game_and_the_drive() {
        assert_eq!(name_stem("yu-no", "hdd1"), "yu-no-hdd1");
        assert_eq!(name_stem("\u{30e6}\u{30fc}\u{30ce}", "fdd2"), "game-fdd2");
        assert_eq!(name_stem("Ys II", "fdd1"), "ys-ii-fdd1");
        let long_stem = format!("{}-{}", "a".repeat(40), "b".repeat(38));
        assert_eq!(long_stem.len(), 79);
        let stem = name_stem(&long_stem, "hdd1");
        assert_eq!(stem, format!("{}-hdd1", "a".repeat(40)));
        assert!(stem.len() <= MAXIMUM_FILE_STEM_LENGTH);
        assert_eq!(title_file_stem(&stem), stem);
        let single_word = "x".repeat(79);
        let stem = name_stem(&single_word, "fdd2");
        assert_eq!(stem, format!("{}-fdd2", "x".repeat(75)));
        let folder = TemporaryFolder::new("image_name_long");
        assert_eq!(
            default_name(folder.path(), &stem, &SASI),
            format!("{stem}.hdi")
        );
    }

    /// Verifies that the default name takes `-2` and `-3` for existing files, ignoring the case.
    #[test]
    fn a_free_name_avoids_existing_files() {
        let folder = TemporaryFolder::new("image_name_free");
        assert_eq!(
            default_name(folder.path(), "yu-no-hdd1", &SASI),
            "yu-no-hdd1.hdi"
        );
        folder.write_file("yu-no-hdd1.hdi", b"");
        assert_eq!(
            default_name(folder.path(), "yu-no-hdd1", &SASI),
            "yu-no-hdd1-2.hdi"
        );
        folder.write_file("YU-NO-HDD1-2.HDI", b"");
        assert_eq!(
            default_name(folder.path(), "yu-no-hdd1", &SASI),
            "yu-no-hdd1-3.hdi"
        );
        assert_eq!(
            default_name(folder.path(), "yu-no-hdd1", &TOWNS_SCSI),
            "yu-no-hdd1.h0"
        );
    }

    /// Verifies the follow rule of an edited name: a known image extension takes the default
    /// extension of the new type, an extension of the new type stays, and any other name stays.
    #[test]
    fn an_edited_name_follows_only_an_image_extension() {
        assert_eq!(
            followed_name("save.HDI", &TOWNS_SCSI).as_deref(),
            Some("save.h0")
        );
        assert_eq!(followed_name("save.h3", &TOWNS_SCSI), None);
        assert_eq!(
            followed_name(" save.dsk ", &MSX_D88).as_deref(),
            Some("save.d88")
        );
        assert_eq!(
            followed_name("save.d88", &MSX_DSK).as_deref(),
            Some("save.dsk")
        );
        assert_eq!(followed_name("notes.txt", &MSX_DSK), None);
        assert_eq!(followed_name("save", &MSX_DSK), None);
    }

    /// Verifies every variant of the check in its order.
    #[test]
    fn the_check_gives_the_first_failure() {
        let folder = TemporaryFolder::new("image_name_check");
        let path = folder.path();
        folder.write_file("taken.hdi", b"");
        assert_eq!(check_name("  ", path, &SASI), NameCheck::EmptyName);
        assert_eq!(check_name("a/b.hdi", path, &SASI), NameCheck::FolderInName);
        assert_eq!(check_name("a\\b.hdi", path, &SASI), NameCheck::FolderInName);
        assert_eq!(check_name("..", path, &SASI), NameCheck::FolderInName);
        assert_eq!(check_name("C:x.hdi", path, &SASI), NameCheck::FolderInName);
        assert_eq!(
            check_name("foo:bar.hdi", path, &SASI),
            NameCheck::FolderInName
        );
        assert_eq!(check_name("CON.hdi", path, &SASI), NameCheck::ReservedName);
        assert_eq!(
            check_name("lpt1.save.hdi", path, &SASI),
            NameCheck::ReservedName
        );
        assert_eq!(check_name("console.hdi", path, &SASI), NameCheck::Valid);
        assert_eq!(check_name("nul.d88", path, &SASI), NameCheck::ReservedName);
        assert_eq!(check_name("a.d88", path, &SASI), NameCheck::WrongExtension);
        assert_eq!(check_name("a", path, &SASI), NameCheck::WrongExtension);
        assert_eq!(
            check_name("a.hdi", &path.join("absent"), &SASI),
            NameCheck::MissingFolder
        );
        assert_eq!(check_name("taken.hdi", path, &SASI), NameCheck::FileExists);
        assert_eq!(check_name(" new.HDI ", path, &SASI), NameCheck::Valid);
        assert!(NameCheck::FileExists.marks_field());
        assert!(NameCheck::ReservedName.marks_field());
        assert!(!NameCheck::MissingFolder.marks_field());
        assert!(!NameCheck::Valid.marks_field());
    }
}
