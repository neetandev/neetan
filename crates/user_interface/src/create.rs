//! The blank disk images: the creation of an empty floppy or hard disk image file.

use std::{io::Write, path::Path};

use common::{Context, bail, info};
use device::{
    disk::HddSizeType,
    floppy::{
        FddType,
        d88::{D88Disk, D88MediaType, D88Sector},
    },
};
use machine_factory::config::settings::blank_images::{
    AT_HDD_EXTENSIONS, BlankFormat, D88_EXTENSIONS, HDF_EXTENSIONS, HDI_EXTENSIONS,
    TOWNS_SCSI_EXTENSIONS,
};

/// Returns the raw extensions of a floppy disk of `fdd_type` in lowercase without the dot.
fn raw_floppy_extensions(fdd_type: FddType) -> &'static [&'static str] {
    match fdd_type {
        FddType::Hd2 => &["xdf", "2hd"],
        FddType::Hd2Fmt144 => &["img", "ima"],
        FddType::Dd2Fmt720 => &["dsk", "img", "ima"],
        FddType::Dd2 | FddType::D2 => &[],
    }
}

/// Returns the extensions of a hard disk of `hdd_type` in lowercase without the dot.
fn hard_disk_extensions(hdd_type: HddSizeType) -> &'static [&'static str] {
    if hdd_type.is_scsi_raw() {
        TOWNS_SCSI_EXTENSIONS
    } else if hdd_type.is_x68k_hdf() {
        HDF_EXTENSIONS
    } else if hdd_type.is_at_flat() {
        AT_HDD_EXTENSIONS
    } else {
        HDI_EXTENSIONS
    }
}

/// Returns whether an image of `format` can be written with the extension `extension`, given
/// without the dot and compared ignoring the case.
#[must_use]
pub fn accepts_extension(format: BlankFormat, extension: &str) -> bool {
    match format {
        BlankFormat::Floppy(fdd_type) => D88_EXTENSIONS
            .iter()
            .chain(raw_floppy_extensions(fdd_type))
            .any(|known| known.eq_ignore_ascii_case(extension)),
        BlankFormat::HardDisk(hdd_type) => hard_disk_extensions(hdd_type)
            .iter()
            .any(|known| known.eq_ignore_ascii_case(extension)),
    }
}

/// Returns the extension of `path` without the dot, or an empty text for a path without one.
fn extension_of(path: &Path) -> &str {
    path.extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
}

/// Joins `extensions` with their dots for an error message, for example `.d88, .xdf or .2hd`.
fn extension_list(extensions: &[&str]) -> String {
    let dotted: Vec<String> = extensions
        .iter()
        .map(|extension| format!(".{extension}"))
        .collect();
    match dotted.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} or {last}", rest.join(", ")),
        None => String::new(),
    }
}

/// Writes `bytes` into a new file at `path`. An existing file is an error. A failed write
/// removes the file that the call created.
fn write_new_file(path: &Path, bytes: &[u8]) -> crate::Result<()> {
    let mut file = std::fs::File::options()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("failed to create {}", path.display()))?;
    let written = file.write_all(bytes);
    if written.is_err() {
        drop(file);
        let _ = std::fs::remove_file(path);
    }
    written.with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

/// Creates a blank floppy disk image of `fdd_type` at `path`. A `.d88` path writes the D88
/// container. A raw extension of the type writes the headerless sectors.
pub fn create_fdd_image(path: &Path, fdd_type: FddType) -> crate::Result<()> {
    let extension = extension_of(path);
    if !accepts_extension(BlankFormat::Floppy(fdd_type), extension) {
        let mut accepted = D88_EXTENSIONS.to_vec();
        accepted.extend_from_slice(raw_floppy_extensions(fdd_type));
        bail!(
            "output path of this floppy type must have a {} extension",
            extension_list(&accepted)
        );
    }
    let raw = !D88_EXTENSIONS
        .iter()
        .any(|d88| d88.eq_ignore_ascii_case(extension));

    let (media_type, cylinders, heads, sectors_per_track, sector_size, size_code) = match fdd_type {
        FddType::Hd2 => (D88MediaType::Disk2HD, 77, 2, 8, 1024, 3u8),
        FddType::Hd2Fmt144 => (D88MediaType::Disk2HD, 80, 2, 18, 512, 2u8),
        FddType::Dd2 => (D88MediaType::Disk2DD, 80, 2, 16, 256, 1u8),
        FddType::Dd2Fmt720 => (D88MediaType::Disk2DD, 80, 2, 9, 512, 2u8),
        FddType::D2 => (D88MediaType::Disk2D, 40, 2, 16, 256, 1u8),
    };

    if raw {
        let bytes = vec![0u8; cylinders * heads * sectors_per_track * sector_size];
        let size_kb = bytes.len() / 1024;
        write_new_file(path, &bytes)?;
        info!("Created {} KB floppy image: {}", size_kb, path.display());
        return Ok(());
    }

    let total_tracks = cylinders * heads;
    let mut track_sectors: Vec<Option<Vec<D88Sector>>> = Vec::with_capacity(total_tracks);

    for track_index in 0..total_tracks {
        let cylinder = (track_index / heads) as u8;
        let head = (track_index % heads) as u8;
        let mut sectors = Vec::with_capacity(sectors_per_track);

        for record in 1..=sectors_per_track as u8 {
            sectors.push(D88Sector {
                cylinder,
                head,
                record,
                size_code,
                sector_count: sectors_per_track as u16,
                mfm_flag: 0x00,
                deleted: 0x00,
                status: 0x00,
                reserved: [0u8; 5],
                data: vec![0u8; sector_size],
                source_offset: None,
            });
        }

        track_sectors.push(Some(sectors));
    }

    let disk = D88Disk::from_tracks(String::new(), false, media_type, track_sectors);
    let bytes = disk.to_bytes();
    let size_kb = bytes.len() / 1024;

    write_new_file(path, &bytes)?;

    info!("Created {} KB floppy image: {}", size_kb, path.display());
    Ok(())
}

/// Creates a blank hard disk image of `hdd_type` at `path`. The extension of `path` must be an
/// extension of the container of the type.
pub fn create_hdd_image(path: &Path, hdd_type: HddSizeType) -> crate::Result<()> {
    if !accepts_extension(BlankFormat::HardDisk(hdd_type), extension_of(path)) {
        bail!(
            "output path of this hard disk type must have a {} extension",
            extension_list(hard_disk_extensions(hdd_type))
        );
    }

    let image = device::disk::blank_hdd_image(hdd_type);
    let bytes = image.to_bytes();
    let size_mb = bytes.len() / (1024 * 1024);

    write_new_file(path, &bytes)?;

    info!("Created {} MB hard disk image: {}", size_mb, path.display());
    Ok(())
}

/// Creates a blank image of `format` at `path`.
pub fn create_blank_image(path: &Path, format: BlankFormat) -> crate::Result<()> {
    match format {
        BlankFormat::Floppy(fdd_type) => create_fdd_image(path, fdd_type),
        BlankFormat::HardDisk(hdd_type) => create_hdd_image(path, hdd_type),
    }
}

#[cfg(test)]
mod tests {
    use machine_factory::config::settings::blank_images::BLANK_IMAGES;

    use super::*;

    /// Verifies that the writer accepts every extension of every blank image of the table.
    #[test]
    fn every_blank_image_extension_is_accepted() {
        for entry in BLANK_IMAGES {
            for image in entry.images {
                for extension in image.extensions {
                    assert!(
                        accepts_extension(image.format, extension),
                        "{:?} {extension}",
                        image.format
                    );
                    assert!(
                        accepts_extension(image.format, &extension.to_ascii_uppercase()),
                        "{:?} {extension}",
                        image.format
                    );
                }
            }
        }
    }

    /// Verifies that the raw extensions follow the floppy type and the hard disk container.
    #[test]
    fn extensions_follow_the_type() {
        assert!(accepts_extension(BlankFormat::Floppy(FddType::D2), "d88"));
        assert!(!accepts_extension(BlankFormat::Floppy(FddType::D2), "xdf"));
        assert!(!accepts_extension(BlankFormat::Floppy(FddType::Hd2), "img"));
        assert!(accepts_extension(
            BlankFormat::Floppy(FddType::Dd2Fmt720),
            "dsk"
        ));
        assert!(!accepts_extension(
            BlankFormat::Floppy(FddType::Hd2Fmt144),
            ""
        ));
        assert!(accepts_extension(
            BlankFormat::HardDisk(HddSizeType::ScsiMb20),
            "h3"
        ));
        assert!(!accepts_extension(
            BlankFormat::HardDisk(HddSizeType::Mb20),
            "hdf"
        ));
        assert_eq!(extension_list(&["d88"]), ".d88");
        assert_eq!(extension_list(&["d88", "xdf", "2hd"]), ".d88, .xdf or .2hd");
    }
}
