//! The files of the path settings: the image types of each drive with their
//! extensions and their icon on each set of families. The section
//! `[file-type.<id>]` of the language files holds the texts of each type.

use std::path::Path;

use super::{CDROM_FAMILIES, Families};

/// The files that a path setting takes on a set of families.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct FileTypes {
    /// The id of the type, for example `floppy`. The section `[file-type.<id>]` of the language
    /// files holds its texts.
    pub id: &'static str,
    /// The families that take the files.
    pub families: Families,
    /// The file extensions in lowercase without the dot.
    pub extensions: &'static [&'static str],
    /// The icon of the images.
    pub icon: ImageIcon,
}

/// The icon of the images of a drive.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ImageIcon {
    /// A disk: a floppy disk, a hard disk, a cartridge or a tape.
    Disk,
    /// An optical disc.
    Disc,
}

impl FileTypes {
    /// Returns whether the extension of `path` is one of the extensions, ignoring the case.
    #[must_use]
    pub fn matches(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                self.extensions
                    .iter()
                    .any(|known| known.eq_ignore_ascii_case(extension))
            })
    }
}

/// The id of the floppy disk images.
pub const FLOPPY_ID: &str = "floppy";
/// The id of the CD-ROM disc images.
pub const CDROM_ID: &str = "cdrom";
/// The id of the hard disk images.
pub const HARD_DISK_ID: &str = "hard-disk";
/// The id of the cartridge images.
pub const CARTRIDGE_ID: &str = "cartridge";
/// The id of the cassette tape images.
pub const TAPE_ID: &str = "tape";

/// The extensions of the floppy disk images. One loader reads them on every family.
const FLOPPY_EXTENSIONS: &[&str] = &[
    "d88", "d98", "88d", "98d", "hdm", "nfd", "2d", "d77", "dim", "xdf", "2hd", "img", "ima", "dsk",
];

/// The files of a floppy drive.
pub(super) const FLOPPY_FILES: &[FileTypes] = &[FileTypes {
    id: FLOPPY_ID,
    families: Families::ALL,
    extensions: FLOPPY_EXTENSIONS,
    icon: ImageIcon::Disk,
}];

/// The files of a CD-ROM drive.
pub(super) const CDROM_FILES: &[FileTypes] = &[FileTypes {
    id: CDROM_ID,
    families: CDROM_FAMILIES,
    extensions: &["cue", "ccd"],
    icon: ImageIcon::Disc,
}];

/// The files of a hard disk drive on each family with a hard disk interface.
pub(super) const HARD_DISK_FILES: &[FileTypes] = &[
    FileTypes {
        id: HARD_DISK_ID,
        families: Families::PC98,
        extensions: &["hdi", "nhd", "thd"],
        icon: ImageIcon::Disk,
    },
    FileTypes {
        id: HARD_DISK_ID,
        families: Families::TOWNS,
        extensions: &["h0", "h1", "h2", "h3", "h4"],
        icon: ImageIcon::Disk,
    },
    FileTypes {
        id: HARD_DISK_ID,
        families: Families::X68K,
        extensions: &["hdf"],
        icon: ImageIcon::Disk,
    },
    FileTypes {
        id: HARD_DISK_ID,
        families: Families::AT,
        extensions: &["hdd"],
        icon: ImageIcon::Disk,
    },
];

/// The files of a cartridge slot.
pub(super) const CARTRIDGE_FILES: &[FileTypes] = &[FileTypes {
    id: CARTRIDGE_ID,
    families: Families::PC60.with(Families::MSX),
    extensions: &["rom"],
    icon: ImageIcon::Disk,
}];

/// The files of a cassette recorder on each family with one.
pub(super) const CASSETTE_FILES: &[FileTypes] = &[
    FileTypes {
        id: TAPE_ID,
        families: Families::PC60,
        extensions: &["cas", "p6", "p6t"],
        icon: ImageIcon::Disk,
    },
    FileTypes {
        id: TAPE_ID,
        families: Families::MSX,
        extensions: &["cas"],
        icon: ImageIcon::Disk,
    },
    FileTypes {
        id: TAPE_ID,
        families: Families::X1,
        extensions: &["tap"],
        icon: ImageIcon::Disk,
    },
    FileTypes {
        id: TAPE_ID,
        families: Families::FM7,
        extensions: &["t77"],
        icon: ImageIcon::Disk,
    },
];

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    /// Every file list of the drives.
    const FILE_LISTS: [&[FileTypes]; 5] = [
        FLOPPY_FILES,
        CDROM_FILES,
        HARD_DISK_FILES,
        CARTRIDGE_FILES,
        CASSETTE_FILES,
    ];

    /// Verifies that the entries of each list take apart families and that their extensions are distinct, lowercase and without the dot.
    #[test]
    fn every_file_list_has_distinct_families_and_extensions() {
        for list in FILE_LISTS {
            let mut covered = Families::NONE;
            for types in list {
                assert!(!covered.intersects(types.families), "{}", types.id);
                covered = covered.with(types.families);
                assert!(!types.extensions.is_empty(), "{}", types.id);
                let unique: BTreeSet<&str> = types.extensions.iter().copied().collect();
                assert_eq!(unique.len(), types.extensions.len(), "{}", types.id);
                for extension in types.extensions {
                    assert!(!extension.starts_with('.'), "{extension}");
                    assert_eq!(extension.to_ascii_lowercase(), *extension);
                }
            }
        }
    }

    /// Verifies that the extension check of the files ignores the case and needs an extension.
    #[test]
    fn file_types_match_their_extensions() {
        let floppy = FLOPPY_FILES[0];
        assert!(floppy.matches(Path::new("games/Ys (Disk A).D88")));
        assert!(floppy.matches(Path::new("disk.hdm")));
        assert!(!floppy.matches(Path::new("disk.cue")));
        assert!(!floppy.matches(Path::new("d88")));
        let extensions: Vec<&str> = HARD_DISK_FILES
            .iter()
            .flat_map(|types| types.extensions.iter().copied())
            .collect();
        assert_eq!(
            extensions,
            [
                "hdi", "nhd", "thd", "h0", "h1", "h2", "h3", "h4", "hdf", "hdd"
            ]
        );
    }

    /// Verifies that only the CD-ROM images show the disc icon.
    #[test]
    fn only_the_cdrom_images_show_the_disc_icon() {
        for (list, icon) in [
            (FLOPPY_FILES, ImageIcon::Disk),
            (CDROM_FILES, ImageIcon::Disc),
            (HARD_DISK_FILES, ImageIcon::Disk),
            (CARTRIDGE_FILES, ImageIcon::Disk),
            (CASSETTE_FILES, ImageIcon::Disk),
        ] {
            for types in list {
                assert_eq!(types.icon, icon, "{}", types.id);
            }
        }
    }
}
