//! The blank images of the drives: the formats of a new empty floppy or hard disk image that each
//! model takes, in the order of the list, with the default of each drive.

use device::{disk::HddSizeType, floppy::FddType};

use super::{
    Families,
    file_types::{FLOPPY_ID, FileTypes, HARD_DISK_ID},
    model_limits::{MSX_FDD_MODELS, PC60_FDD_MODELS},
};
use crate::config::Target;

/// The format of a blank image.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum BlankFormat {
    /// A floppy disk of this type.
    Floppy(FddType),
    /// A hard disk of this size and container.
    HardDisk(HddSizeType),
}

/// A blank image that a drive takes.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct BlankImage {
    /// The format of the image.
    pub format: BlankFormat,
    /// The extensions of the image in lowercase without the dot. The first one is the default.
    pub extensions: &'static [&'static str],
}

impl BlankImage {
    /// Returns whether the image takes the extension `extension`, given without the dot and
    /// compared ignoring the case.
    #[must_use]
    pub fn takes_extension(&self, extension: &str) -> bool {
        self.extensions
            .iter()
            .any(|known| known.eq_ignore_ascii_case(extension))
    }
}

/// The blank images of the drives of one file type on some models of one family.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct BlankImages {
    /// The id of the files of the drives: `FLOPPY_ID` or `HARD_DISK_ID`.
    pub types_id: &'static str,
    /// The keys of the drive settings of the entry, for example `hdd1`.
    pub keys: &'static [&'static str],
    /// The family of the models.
    pub family: Families,
    /// The models in the form of the `machine` values.
    pub models: &'static [&'static str],
    /// The images in the order of the list.
    pub images: &'static [BlankImage],
    /// The index of the default image in `images`.
    pub default: usize,
    /// The index of the image after which the list draws a separator.
    pub separator_after: Option<usize>,
}

/// The extensions of a D88 floppy disk image.
pub const D88_EXTENSIONS: &[&str] = &["d88"];
/// The extensions of a PC-98 hard disk image.
pub const HDI_EXTENSIONS: &[&str] = &["hdi"];
/// The extensions of an FM Towns SCSI hard disk image, with `h0` first for SCSI ID 0. The digit
/// of an extension names the SCSI ID by convention.
pub const TOWNS_SCSI_EXTENSIONS: &[&str] = &["h0", "h1", "h2", "h3", "h4"];
/// The extensions of an FM Towns SCSI hard disk image on SCSI ID 1, with `h1` first.
pub const TOWNS_SCSI_ID1_EXTENSIONS: &[&str] = &["h1", "h0", "h2", "h3", "h4"];
/// The extensions of an X68000 hard disk image.
pub const HDF_EXTENSIONS: &[&str] = &["hdf"];
/// The extensions of a PC/AT hard disk image.
pub const AT_HDD_EXTENSIONS: &[&str] = &["hdd"];

/// The keys of the floppy disk drives.
const FLOPPY_KEYS: &[&str] = &["fdd1", "fdd2"];
/// The keys of the hard disk drives.
const HARD_DISK_KEYS: &[&str] = &["hdd1", "hdd2"];
/// The key of the FM Towns hard disk drive on SCSI ID 0.
const TOWNS_SCSI_ID0_KEYS: &[&str] = &["hdd1"];
/// The key of the FM Towns hard disk drive on SCSI ID 1.
const TOWNS_SCSI_ID1_KEYS: &[&str] = &["hdd2"];

/// Returns a D88 floppy disk image of `floppy_type`.
const fn d88_floppy(floppy_type: FddType) -> BlankImage {
    BlankImage {
        format: BlankFormat::Floppy(floppy_type),
        extensions: D88_EXTENSIONS,
    }
}

/// Returns a hard disk image of `size` with the extensions `extensions`.
const fn hard_disk(size: HddSizeType, extensions: &'static [&'static str]) -> BlankImage {
    BlankImage {
        format: BlankFormat::HardDisk(size),
        extensions,
    }
}

/// The PC-98 models with a SASI hard disk interface.
const PC9801_MODELS: &[&str] = &["PC9801F", "PC9801VM", "PC9801VX", "PC9801RS", "PC9801RA"];
/// The PC-98 models with an IDE hard disk interface.
const PC9821_MODELS: &[&str] = &["PC9821AS", "PC9821AP"];
/// Every PC-98 model.
const PC98_MODELS: &[&str] = &[
    "PC9801F", "PC9801VM", "PC9801VX", "PC9801RS", "PC9801RA", "PC9821AS", "PC9821AP",
];
/// The PC-88 models.
const PC88_MODELS: &[&str] = &["PC8801MC"];
/// The PC-88VA models.
const PC88VA_MODELS: &[&str] = &["PC88VA2"];
/// The FM Towns models.
const TOWNS_MODELS: &[&str] = &["FMTowns", "FMTownsIICX", "FMTownsIIMX"];
/// The X1 models.
const X1_MODELS: &[&str] = &["X1", "X1TURBO"];
/// The FM-7 models.
const FM7_MODELS: &[&str] = &["FM7", "FM77AV"];
/// Every X68000 model.
const X68K_MODELS: &[&str] = &["X68000", "X68000SUPER", "X68000XVI"];
/// The X68000 models with a SASI hard disk interface.
const X68K_SASI_MODELS: &[&str] = &["X68000"];
/// The X68000 models with an internal SCSI hard disk interface.
const X68K_SCSI_MODELS: &[&str] = &["X68000SUPER", "X68000XVI"];
/// The PC/AT models.
const AT_MODELS: &[&str] = &["AT486DX50", "AT486DX66"];

/// The 2HD 1.2 MB D88 floppy disk.
const HD2_D88_IMAGES: &[BlankImage] = &[d88_floppy(FddType::Hd2)];
/// The 2D 320 KB D88 floppy disk.
const D2_D88_IMAGES: &[BlankImage] = &[d88_floppy(FddType::D2)];
/// The floppy disks of the PC-88: 2D and 2HD.
const PC88_FLOPPY_IMAGES: &[BlankImage] = &[d88_floppy(FddType::D2), d88_floppy(FddType::Hd2)];
/// The floppy disks of the PC-88VA: 2HD, 2DD and 2D.
const PC88VA_FLOPPY_IMAGES: &[BlankImage] = &[
    d88_floppy(FddType::Hd2),
    d88_floppy(FddType::Dd2),
    d88_floppy(FddType::D2),
];
/// The floppy disks of the MSX: 2DD 720 KB as a raw DSK and as a D88.
const MSX_FLOPPY_IMAGES: &[BlankImage] = &[
    BlankImage {
        format: BlankFormat::Floppy(FddType::Dd2Fmt720),
        extensions: &["dsk"],
    },
    d88_floppy(FddType::Dd2Fmt720),
];
/// The floppy disks of the X68000: 2HD 1.2 MB as a raw XDF and as a D88.
const X68K_FLOPPY_IMAGES: &[BlankImage] = &[
    BlankImage {
        format: BlankFormat::Floppy(FddType::Hd2),
        extensions: &["xdf"],
    },
    d88_floppy(FddType::Hd2),
];
/// The floppy disks of the PC/AT: 2HD 1.44 MB and 2DD 720 KB as raw IMG files.
const AT_FLOPPY_IMAGES: &[BlankImage] = &[
    BlankImage {
        format: BlankFormat::Floppy(FddType::Hd2Fmt144),
        extensions: &["img"],
    },
    BlankImage {
        format: BlankFormat::Floppy(FddType::Dd2Fmt720),
        extensions: &["img"],
    },
];

/// The SASI hard disks of the PC-98.
const PC98_SASI_IMAGES: &[BlankImage] = &[
    hard_disk(HddSizeType::Mb5, HDI_EXTENSIONS),
    hard_disk(HddSizeType::Mb10, HDI_EXTENSIONS),
    hard_disk(HddSizeType::Mb15, HDI_EXTENSIONS),
    hard_disk(HddSizeType::Mb20, HDI_EXTENSIONS),
    hard_disk(HddSizeType::Mb30, HDI_EXTENSIONS),
    hard_disk(HddSizeType::Mb40, HDI_EXTENSIONS),
];
/// The IDE hard disks of the PC-98, then the SASI hard disks that the IDE models also accept.
const PC98_IDE_IMAGES: &[BlankImage] = &[
    hard_disk(HddSizeType::IdeMb40, HDI_EXTENSIONS),
    hard_disk(HddSizeType::IdeMb80, HDI_EXTENSIONS),
    hard_disk(HddSizeType::IdeMb120, HDI_EXTENSIONS),
    hard_disk(HddSizeType::IdeMb200, HDI_EXTENSIONS),
    hard_disk(HddSizeType::IdeMb500, HDI_EXTENSIONS),
    hard_disk(HddSizeType::Mb5, HDI_EXTENSIONS),
    hard_disk(HddSizeType::Mb10, HDI_EXTENSIONS),
    hard_disk(HddSizeType::Mb15, HDI_EXTENSIONS),
    hard_disk(HddSizeType::Mb20, HDI_EXTENSIONS),
    hard_disk(HddSizeType::Mb30, HDI_EXTENSIONS),
    hard_disk(HddSizeType::Mb40, HDI_EXTENSIONS),
];
/// The index of the last IDE hard disk in `PC98_IDE_IMAGES`.
const PC98_LAST_IDE_IMAGE: usize = 4;
/// The index of the SASI 20 MB hard disk in `PC98_SASI_IMAGES`.
const PC98_SASI_DEFAULT: usize = 3;
/// The SCSI hard disks of the FM Towns on SCSI ID 0.
const TOWNS_SCSI_ID0_IMAGES: &[BlankImage] = &[
    hard_disk(HddSizeType::ScsiMb20, TOWNS_SCSI_EXTENSIONS),
    hard_disk(HddSizeType::ScsiMb40, TOWNS_SCSI_EXTENSIONS),
    hard_disk(HddSizeType::ScsiMb100, TOWNS_SCSI_EXTENSIONS),
    hard_disk(HddSizeType::ScsiMb200, TOWNS_SCSI_EXTENSIONS),
    hard_disk(HddSizeType::ScsiMb340, TOWNS_SCSI_EXTENSIONS),
    hard_disk(HddSizeType::ScsiMb540, TOWNS_SCSI_EXTENSIONS),
];
/// The SCSI hard disks of the FM Towns on SCSI ID 1.
const TOWNS_SCSI_ID1_IMAGES: &[BlankImage] = &[
    hard_disk(HddSizeType::ScsiMb20, TOWNS_SCSI_ID1_EXTENSIONS),
    hard_disk(HddSizeType::ScsiMb40, TOWNS_SCSI_ID1_EXTENSIONS),
    hard_disk(HddSizeType::ScsiMb100, TOWNS_SCSI_ID1_EXTENSIONS),
    hard_disk(HddSizeType::ScsiMb200, TOWNS_SCSI_ID1_EXTENSIONS),
    hard_disk(HddSizeType::ScsiMb340, TOWNS_SCSI_ID1_EXTENSIONS),
    hard_disk(HddSizeType::ScsiMb540, TOWNS_SCSI_ID1_EXTENSIONS),
];
/// The index of the SCSI 100 MB hard disk in both lists of the FM Towns hard disks.
const TOWNS_SCSI_DEFAULT: usize = 2;
/// The SASI hard disks of the X68000.
const X68K_SASI_IMAGES: &[BlankImage] = &[
    hard_disk(HddSizeType::X68kSasiMb10, HDF_EXTENSIONS),
    hard_disk(HddSizeType::X68kSasiMb20, HDF_EXTENSIONS),
    hard_disk(HddSizeType::X68kSasiMb40, HDF_EXTENSIONS),
];
/// The index of the SASI 20 MB hard disk in `X68K_SASI_IMAGES`.
const X68K_SASI_DEFAULT: usize = 1;
/// The SCSI hard disks of the X68000 SUPER and XVI.
const X68K_SCSI_IMAGES: &[BlankImage] = &[
    hard_disk(HddSizeType::X68kScsiMb20, HDF_EXTENSIONS),
    hard_disk(HddSizeType::X68kScsiMb40, HDF_EXTENSIONS),
];
/// The index of the SCSI 40 MB hard disk in `X68K_SCSI_IMAGES`.
const X68K_SCSI_DEFAULT: usize = 1;
/// The IDE hard disks of the PC/AT.
const AT_IDE_IMAGES: &[BlankImage] = &[
    hard_disk(HddSizeType::AtMb40, AT_HDD_EXTENSIONS),
    hard_disk(HddSizeType::AtMb100, AT_HDD_EXTENSIONS),
    hard_disk(HddSizeType::AtMb250, AT_HDD_EXTENSIONS),
    hard_disk(HddSizeType::AtMb504, AT_HDD_EXTENSIONS),
];
/// The index of the IDE 100 MB hard disk in `AT_IDE_IMAGES`.
const AT_IDE_DEFAULT: usize = 1;

/// Returns the floppy disk images `images` of the models `models` of `family`, with the first
/// image as the default.
const fn floppy_images(
    family: Families,
    models: &'static [&'static str],
    images: &'static [BlankImage],
) -> BlankImages {
    BlankImages {
        types_id: FLOPPY_ID,
        keys: FLOPPY_KEYS,
        family,
        models,
        images,
        default: 0,
        separator_after: None,
    }
}

/// Returns the hard disk images `images` of the drives `keys` of the models `models` of
/// `family`, with the image at `default` as the default.
const fn hard_disk_images(
    keys: &'static [&'static str],
    family: Families,
    models: &'static [&'static str],
    images: &'static [BlankImage],
    default: usize,
) -> BlankImages {
    BlankImages {
        types_id: HARD_DISK_ID,
        keys,
        family,
        models,
        images,
        default,
        separator_after: None,
    }
}

/// The blank images of every drive kind, family and model.
pub const BLANK_IMAGES: &[BlankImages] = &[
    floppy_images(Families::PC98, PC98_MODELS, HD2_D88_IMAGES),
    floppy_images(Families::PC88, PC88_MODELS, PC88_FLOPPY_IMAGES),
    floppy_images(Families::PC88VA, PC88VA_MODELS, PC88VA_FLOPPY_IMAGES),
    floppy_images(Families::PC60, PC60_FDD_MODELS, D2_D88_IMAGES),
    floppy_images(Families::MSX, MSX_FDD_MODELS, MSX_FLOPPY_IMAGES),
    floppy_images(Families::TOWNS, TOWNS_MODELS, HD2_D88_IMAGES),
    floppy_images(Families::X1, X1_MODELS, D2_D88_IMAGES),
    floppy_images(Families::FM7, FM7_MODELS, D2_D88_IMAGES),
    floppy_images(Families::X68K, X68K_MODELS, X68K_FLOPPY_IMAGES),
    floppy_images(Families::AT, AT_MODELS, AT_FLOPPY_IMAGES),
    hard_disk_images(
        HARD_DISK_KEYS,
        Families::PC98,
        PC9801_MODELS,
        PC98_SASI_IMAGES,
        PC98_SASI_DEFAULT,
    ),
    BlankImages {
        types_id: HARD_DISK_ID,
        keys: HARD_DISK_KEYS,
        family: Families::PC98,
        models: PC9821_MODELS,
        images: PC98_IDE_IMAGES,
        default: 0,
        separator_after: Some(PC98_LAST_IDE_IMAGE),
    },
    hard_disk_images(
        TOWNS_SCSI_ID0_KEYS,
        Families::TOWNS,
        TOWNS_MODELS,
        TOWNS_SCSI_ID0_IMAGES,
        TOWNS_SCSI_DEFAULT,
    ),
    hard_disk_images(
        TOWNS_SCSI_ID1_KEYS,
        Families::TOWNS,
        TOWNS_MODELS,
        TOWNS_SCSI_ID1_IMAGES,
        TOWNS_SCSI_DEFAULT,
    ),
    hard_disk_images(
        HARD_DISK_KEYS,
        Families::X68K,
        X68K_SASI_MODELS,
        X68K_SASI_IMAGES,
        X68K_SASI_DEFAULT,
    ),
    hard_disk_images(
        HARD_DISK_KEYS,
        Families::X68K,
        X68K_SCSI_MODELS,
        X68K_SCSI_IMAGES,
        X68K_SCSI_DEFAULT,
    ),
    hard_disk_images(
        HARD_DISK_KEYS,
        Families::AT,
        AT_MODELS,
        AT_IDE_IMAGES,
        AT_IDE_DEFAULT,
    ),
];

/// Returns whether `extension` is an extension of a blank image of any drive, given without the
/// dot and compared ignoring the case.
#[must_use]
pub fn is_blank_image_extension(extension: &str) -> bool {
    BLANK_IMAGES.iter().any(|entry| {
        entry
            .images
            .iter()
            .any(|image| image.takes_extension(extension))
    })
}

impl BlankImages {
    /// Returns whether the entry covers the drive `key` with the files `types` on the family
    /// `target`. The key compares ignoring the case.
    fn covers(&self, types: &FileTypes, target: Target, key: &str) -> bool {
        self.types_id == types.id
            && self.family.contains_target(target)
            && self
                .keys
                .iter()
                .any(|known| known.eq_ignore_ascii_case(key))
    }
}

/// Returns whether any model of the family `target` has blank images for the drive `key` with
/// the files `types`.
#[must_use]
pub fn has_blank_images(types: &FileTypes, target: Target, key: &str) -> bool {
    BLANK_IMAGES
        .iter()
        .any(|entry| entry.covers(types, target, key))
}

/// Returns the blank images of the drive `key` with the files `types` on the model `model` of
/// the family `target`. The key and the model compare ignoring the case.
#[must_use]
pub fn blank_images_for(
    types: &FileTypes,
    target: Target,
    key: &str,
    model: &str,
) -> Option<&'static BlankImages> {
    BLANK_IMAGES.iter().find(|entry| {
        entry.covers(types, target, key)
            && entry
                .models
                .iter()
                .any(|name| name.eq_ignore_ascii_case(model))
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{
        super::{MODEL_CHOICES, find_setting},
        *,
    };
    use crate::config::MachineSelection;

    /// The drive settings that take blank images.
    const DRIVE_KEYS: [&str; 4] = ["fdd1", "fdd2", "hdd1", "hdd2"];

    /// Verifies that every entry has images, a default and a separator inside the list, and
    /// distinct lowercase extensions without the dot.
    #[test]
    fn every_entry_has_a_valid_default_separator_and_extensions() {
        for entry in BLANK_IMAGES {
            assert!(!entry.images.is_empty(), "{:?}", entry.models);
            assert!(entry.default < entry.images.len(), "{:?}", entry.models);
            if let Some(separator) = entry.separator_after {
                assert!(separator < entry.images.len() - 1, "{:?}", entry.models);
            }
            assert!(
                [FLOPPY_ID, HARD_DISK_ID].contains(&entry.types_id),
                "{}",
                entry.types_id
            );
            for image in entry.images {
                assert!(!image.extensions.is_empty(), "{:?}", image.format);
                let unique: BTreeSet<&str> = image.extensions.iter().copied().collect();
                assert_eq!(unique.len(), image.extensions.len(), "{:?}", image.format);
                for extension in image.extensions {
                    assert!(!extension.starts_with('.'), "{extension}");
                    assert_eq!(extension.to_ascii_lowercase(), *extension);
                }
            }
        }
    }

    /// Verifies that the models of the entries are models of their family, that the keys of the
    /// entries are drive keys of their file type, and that no drive of a model appears in two
    /// entries of one file type.
    #[test]
    fn every_drive_of_a_model_belongs_to_one_entry_of_its_family() {
        let mut seen = BTreeSet::new();
        for entry in BLANK_IMAGES {
            assert!(!entry.models.is_empty(), "{}", entry.types_id);
            assert!(!entry.keys.is_empty(), "{:?}", entry.models);
            let drive_keys = if entry.types_id == FLOPPY_ID {
                FLOPPY_KEYS
            } else {
                HARD_DISK_KEYS
            };
            for key in entry.keys {
                assert!(drive_keys.contains(key), "{key}");
            }
            for model in entry.models {
                assert!(
                    MODEL_CHOICES
                        .iter()
                        .any(|choice| choice.value == *model && choice.families == entry.family),
                    "{model}"
                );
                for key in entry.keys {
                    assert!(
                        seen.insert((entry.types_id, *key, *model)),
                        "{model} {key} in two {} entries",
                        entry.types_id
                    );
                }
            }
        }
    }

    /// Verifies that every compiled model that reads a drive setting has blank images for it.
    #[test]
    fn every_model_that_reads_a_drive_has_blank_images() {
        for key in DRIVE_KEYS {
            let setting = find_setting(key).expect("a drive setting");
            for choice in MODEL_CHOICES {
                let Ok(model) = choice.value.parse::<MachineSelection>() else {
                    continue;
                };
                let target = model.target();
                if !setting.is_read_by(target, choice.value) {
                    continue;
                }
                let types = setting.file_types_for(target).expect("drive files");
                assert!(
                    has_blank_images(types, target, key),
                    "{key} {}",
                    choice.value
                );
                assert!(
                    blank_images_for(types, target, key, choice.value).is_some(),
                    "{key} {}",
                    choice.value
                );
                assert!(
                    blank_images_for(
                        types,
                        target,
                        &key.to_ascii_uppercase(),
                        &choice.value.to_ascii_lowercase()
                    )
                    .is_some(),
                    "{key} {}",
                    choice.value
                );
            }
        }
    }

    /// Verifies that the drive files of the family accept every extension of every image.
    #[test]
    fn the_drive_files_accept_every_blank_image_extension() {
        for key in DRIVE_KEYS {
            let setting = find_setting(key).expect("a drive setting");
            for choice in MODEL_CHOICES {
                let Ok(model) = choice.value.parse::<MachineSelection>() else {
                    continue;
                };
                let target = model.target();
                let Some(types) = setting.file_types_for(target) else {
                    continue;
                };
                let Some(entry) = blank_images_for(types, target, key, choice.value) else {
                    continue;
                };
                for image in entry.images {
                    for extension in image.extensions {
                        assert!(
                            types.extensions.contains(extension),
                            "{key} {} {extension}",
                            choice.value
                        );
                    }
                }
            }
        }
    }

    #[cfg(feature = "pc98")]
    /// Verifies the defaults of the PC-98 hard disks on both interfaces and the separator of the IDE list.
    #[test]
    fn the_pc98_hard_disks_default_per_interface() {
        let setting = find_setting("hdd1").expect("hdd1 is a table key");
        let types = setting
            .file_types_for(Target::Pc98)
            .expect("hard disk files");
        let sasi = blank_images_for(types, Target::Pc98, "hdd1", "PC9801RA").expect("SASI images");
        assert_eq!(
            sasi.images[sasi.default].format,
            BlankFormat::HardDisk(HddSizeType::Mb20)
        );
        let ide = blank_images_for(types, Target::Pc98, "hdd2", "PC9821AS").expect("IDE images");
        assert_eq!(
            ide.images[ide.default].format,
            BlankFormat::HardDisk(HddSizeType::IdeMb40)
        );
        assert_eq!(ide.separator_after, Some(PC98_LAST_IDE_IMAGE));
        assert_eq!(
            ide.images[PC98_LAST_IDE_IMAGE + 1].format,
            BlankFormat::HardDisk(HddSizeType::Mb5)
        );
    }

    /// Verifies that the extension checks of an image and of the table ignore the case and need
    /// a known extension.
    #[test]
    fn extensions_match_ignoring_the_case() {
        let image = BlankImage {
            format: BlankFormat::HardDisk(HddSizeType::ScsiMb20),
            extensions: TOWNS_SCSI_EXTENSIONS,
        };
        assert!(image.takes_extension("H3"));
        assert!(!image.takes_extension("hdi"));
        assert!(!image.takes_extension(""));
        assert!(is_blank_image_extension("HDI"));
        assert!(is_blank_image_extension("dsk"));
        assert!(!is_blank_image_extension("txt"));
    }

    #[cfg(feature = "msx")]
    /// Verifies that only the MSX models with a floppy drive have blank floppy disks.
    #[test]
    fn only_the_msx_models_with_a_drive_have_blank_floppy_disks() {
        let setting = find_setting("fdd1").expect("fdd1 is a table key");
        let types = setting.file_types_for(Target::Msx).expect("floppy files");
        assert!(has_blank_images(types, Target::Msx, "fdd1"));
        assert!(blank_images_for(types, Target::Msx, "fdd1", "MSX").is_none());
        assert!(blank_images_for(types, Target::Msx, "fdd1", "MSX2").is_some());
    }

    #[cfg(feature = "towns")]
    /// Verifies that the FM Towns hard disks default to the extension of the SCSI ID of their
    /// drive, SCSI 100 MB on both, and that both drives take all five extensions.
    #[test]
    fn the_towns_hard_disks_default_to_the_extension_of_their_scsi_id() {
        let setting = find_setting("hdd1").expect("hdd1 is a table key");
        let types = setting
            .file_types_for(Target::Towns)
            .expect("hard disk files");
        for (key, extension) in [("hdd1", "h0"), ("hdd2", "h1")] {
            let entry =
                blank_images_for(types, Target::Towns, key, "FMTownsIIMX").expect("SCSI images");
            let image = entry.images[entry.default];
            assert_eq!(image.format, BlankFormat::HardDisk(HddSizeType::ScsiMb100));
            for image in entry.images {
                assert_eq!(image.extensions[0], extension, "{key}");
                for digit in TOWNS_SCSI_EXTENSIONS {
                    assert!(image.takes_extension(digit), "{key} {digit}");
                }
            }
        }
        assert!(blank_images_for(types, Target::Towns, "fdd1", "FMTowns").is_none());
    }
}
