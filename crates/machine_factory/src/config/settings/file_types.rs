//! The files of the path settings: the image types of each drive with their
//! extensions, their noun and their icon on each set of families.

use std::path::Path;

use super::{CDROM_FAMILIES, Families};

/// The files that a path setting takes on a set of families.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct FileTypes {
    /// The families that take the files.
    pub families: Families,
    /// The name of the files in the plural, for example `Disk images`.
    pub name: &'static str,
    /// The file extensions in lowercase without the dot.
    pub extensions: &'static [&'static str],
    /// The word for one image in a sentence, for example `disk`.
    pub noun: &'static str,
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

/// The extensions of the floppy disk images. One loader reads them on every family.
const FLOPPY_EXTENSIONS: &[&str] = &[
    "d88", "d98", "88d", "98d", "hdm", "nfd", "2d", "d77", "dim", "xdf", "2hd", "img", "ima", "dsk",
];

/// The files of a floppy drive.
pub(super) const FLOPPY_FILES: &[FileTypes] = &[FileTypes {
    families: Families::ALL,
    name: "Disk images",
    extensions: FLOPPY_EXTENSIONS,
    noun: "disk",
    icon: ImageIcon::Disk,
}];

/// The files of a CD-ROM drive.
pub(super) const CDROM_FILES: &[FileTypes] = &[FileTypes {
    families: CDROM_FAMILIES,
    name: "Disc images",
    extensions: &["cue", "ccd"],
    noun: "disc",
    icon: ImageIcon::Disc,
}];

/// The files of a hard disk drive on each family with a hard disk interface.
pub(super) const HARD_DISK_FILES: &[FileTypes] = &[
    FileTypes {
        families: Families::PC98,
        name: "Hard disk images",
        extensions: &["hdi", "nhd", "thd"],
        noun: "disk",
        icon: ImageIcon::Disk,
    },
    FileTypes {
        families: Families::TOWNS,
        name: "Hard disk images",
        extensions: &["h0", "h1", "h2", "h3", "h4"],
        noun: "disk",
        icon: ImageIcon::Disk,
    },
    FileTypes {
        families: Families::X68K,
        name: "Hard disk images",
        extensions: &["hdf"],
        noun: "disk",
        icon: ImageIcon::Disk,
    },
    FileTypes {
        families: Families::AT,
        name: "Hard disk images",
        extensions: &["hdd"],
        noun: "disk",
        icon: ImageIcon::Disk,
    },
];

/// The files of a cartridge slot.
pub(super) const CARTRIDGE_FILES: &[FileTypes] = &[FileTypes {
    families: Families::PC60.with(Families::MSX),
    name: "Cartridge images",
    extensions: &["rom"],
    noun: "cartridge",
    icon: ImageIcon::Disk,
}];

/// The files of a cassette recorder on each family with one.
pub(super) const CASSETTE_FILES: &[FileTypes] = &[
    FileTypes {
        families: Families::PC60,
        name: "Tape images",
        extensions: &["cas", "p6", "p6t"],
        noun: "tape",
        icon: ImageIcon::Disk,
    },
    FileTypes {
        families: Families::MSX,
        name: "Tape images",
        extensions: &["cas"],
        noun: "tape",
        icon: ImageIcon::Disk,
    },
    FileTypes {
        families: Families::X1,
        name: "Tape images",
        extensions: &["tap"],
        noun: "tape",
        icon: ImageIcon::Disk,
    },
    FileTypes {
        families: Families::FM7,
        name: "Tape images",
        extensions: &["t77"],
        noun: "tape",
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
                assert!(!covered.intersects(types.families), "{}", types.name);
                covered = covered.with(types.families);
                assert!(!types.extensions.is_empty(), "{}", types.name);
                assert!(!types.noun.is_empty(), "{}", types.name);
                let unique: BTreeSet<&str> = types.extensions.iter().copied().collect();
                assert_eq!(unique.len(), types.extensions.len(), "{}", types.name);
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
                assert_eq!(types.icon, icon, "{}", types.name);
            }
        }
    }
}
