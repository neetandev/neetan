//! The rows of the image types of the create image dialog: the technical name and the capacity
//! of each blank format and the text of a row, for example `IDE 40 MB (.hdi)`.

use std::fmt;

use device::{disk::HddSizeType, floppy::FddType};
use i18n::Catalog;
use machine_factory::config::settings::blank_images::{BlankFormat, BlankImage};

use crate::texts::create_image;

/// Returns the technical name of the format of `format`, for example `2HD` or `SCSI`.
pub(crate) fn format_name(format: BlankFormat) -> &'static str {
    match format {
        BlankFormat::Floppy(FddType::Hd2 | FddType::Hd2Fmt144) => "2HD",
        BlankFormat::Floppy(FddType::Dd2 | FddType::Dd2Fmt720) => "2DD",
        BlankFormat::Floppy(FddType::D2) => "2D",
        BlankFormat::HardDisk(
            HddSizeType::Mb5
            | HddSizeType::Mb10
            | HddSizeType::Mb15
            | HddSizeType::Mb20
            | HddSizeType::Mb30
            | HddSizeType::Mb40
            | HddSizeType::X68kSasiMb10
            | HddSizeType::X68kSasiMb20
            | HddSizeType::X68kSasiMb40,
        ) => "SASI",
        BlankFormat::HardDisk(
            HddSizeType::IdeMb40
            | HddSizeType::IdeMb80
            | HddSizeType::IdeMb120
            | HddSizeType::IdeMb200
            | HddSizeType::IdeMb500
            | HddSizeType::AtMb40
            | HddSizeType::AtMb100
            | HddSizeType::AtMb250
            | HddSizeType::AtMb504,
        ) => "IDE",
        BlankFormat::HardDisk(
            HddSizeType::ScsiMb20
            | HddSizeType::ScsiMb40
            | HddSizeType::ScsiMb100
            | HddSizeType::ScsiMb200
            | HddSizeType::ScsiMb340
            | HddSizeType::ScsiMb540
            | HddSizeType::X68kScsiMb20
            | HddSizeType::X68kScsiMb40,
        ) => "SCSI",
    }
}

/// Returns the capacity of an image of `format`, for example `1.2 MB`.
pub(crate) fn capacity(format: BlankFormat) -> &'static str {
    match format {
        BlankFormat::Floppy(FddType::Hd2) => "1.2 MB",
        BlankFormat::Floppy(FddType::Hd2Fmt144) => "1.44 MB",
        BlankFormat::Floppy(FddType::Dd2) => "640 KB",
        BlankFormat::Floppy(FddType::Dd2Fmt720) => "720 KB",
        BlankFormat::Floppy(FddType::D2) => "320 KB",
        BlankFormat::HardDisk(HddSizeType::Mb5) => "5 MB",
        BlankFormat::HardDisk(HddSizeType::Mb10 | HddSizeType::X68kSasiMb10) => "10 MB",
        BlankFormat::HardDisk(HddSizeType::Mb15) => "15 MB",
        BlankFormat::HardDisk(
            HddSizeType::Mb20
            | HddSizeType::ScsiMb20
            | HddSizeType::X68kSasiMb20
            | HddSizeType::X68kScsiMb20,
        ) => "20 MB",
        BlankFormat::HardDisk(HddSizeType::Mb30) => "30 MB",
        BlankFormat::HardDisk(
            HddSizeType::Mb40
            | HddSizeType::IdeMb40
            | HddSizeType::ScsiMb40
            | HddSizeType::X68kSasiMb40
            | HddSizeType::X68kScsiMb40
            | HddSizeType::AtMb40,
        ) => "40 MB",
        BlankFormat::HardDisk(HddSizeType::IdeMb80) => "80 MB",
        BlankFormat::HardDisk(HddSizeType::ScsiMb100 | HddSizeType::AtMb100) => "100 MB",
        BlankFormat::HardDisk(HddSizeType::IdeMb120) => "120 MB",
        BlankFormat::HardDisk(HddSizeType::IdeMb200 | HddSizeType::ScsiMb200) => "200 MB",
        BlankFormat::HardDisk(HddSizeType::AtMb250) => "250 MB",
        BlankFormat::HardDisk(HddSizeType::ScsiMb340) => "340 MB",
        BlankFormat::HardDisk(HddSizeType::IdeMb500) => "500 MB",
        BlankFormat::HardDisk(HddSizeType::AtMb504) => "504 MB",
        BlankFormat::HardDisk(HddSizeType::ScsiMb540) => "540 MB",
    }
}

/// An extension that writes itself with its dot, for example `.hdi`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DottedExtension<'a>(pub(crate) &'a str);

impl fmt::Display for DottedExtension<'_> {
    /// Writes the dot and the extension.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, ".{}", self.0)
    }
}

/// Writes the text of the row of `image` in `catalog`: its format, its capacity and its default
/// extension, for example `IDE 40 MB (.hdi)`.
pub(crate) fn write_type_row(catalog: &Catalog, image: &BlankImage, out: &mut String) {
    let extension = DottedExtension(image.extensions.first().copied().unwrap_or_default());
    let _ = catalog.write(
        out,
        create_image::type_row(
            &format_name(image.format),
            &capacity(image.format),
            &extension,
        ),
    );
}

#[cfg(test)]
mod tests {
    use machine_factory::config::settings::blank_images::BLANK_IMAGES;

    use super::*;
    use crate::texts::english_text;

    /// The English text of the row of `format` with the extensions `extensions`.
    fn english_row(format: BlankFormat, extensions: &'static [&'static str]) -> String {
        let image = BlankImage { format, extensions };
        english_text(|catalog, out| write_type_row(catalog, &image, out))
    }

    /// Verifies the English row texts of the examples of the design.
    #[test]
    fn the_rows_name_the_format_the_capacity_and_the_extension() {
        assert_eq!(
            english_row(BlankFormat::HardDisk(HddSizeType::IdeMb40), &["hdi"]),
            "IDE 40 MB (.hdi)"
        );
        assert_eq!(
            english_row(BlankFormat::HardDisk(HddSizeType::ScsiMb100), &["h0"]),
            "SCSI 100 MB (.h0)"
        );
        assert_eq!(
            english_row(BlankFormat::Floppy(FddType::Hd2), &["xdf"]),
            "2HD 1.2 MB (.xdf)"
        );
        assert_eq!(
            english_row(BlankFormat::Floppy(FddType::Dd2Fmt720), &["dsk"]),
            "2DD 720 KB (.dsk)"
        );
        assert_eq!(
            english_row(BlankFormat::HardDisk(HddSizeType::X68kSasiMb20), &["hdf"]),
            "SASI 20 MB (.hdf)"
        );
    }

    /// Verifies that every image of every entry of the table has a name and a capacity.
    #[test]
    fn every_blank_image_has_a_name_and_a_capacity() {
        for entry in BLANK_IMAGES {
            for image in entry.images {
                assert!(!format_name(image.format).is_empty(), "{:?}", image.format);
                assert!(!capacity(image.format).is_empty(), "{:?}", image.format);
                assert!(format_name(image.format).is_ascii());
                assert!(capacity(image.format).is_ascii());
            }
        }
    }
}
