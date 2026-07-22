//! Shared test-only image builders and temp-file helper for the disk modules.

use std::path::PathBuf;

use crate::disk::{
    hdi::HDI_HEADER_SIZE,
    nhd::{NHD_HEADER_SIZE, NHD_SIGNATURE},
    thd::{THD_HEADER_SIZE, THD_HEADS, THD_SECTOR_SIZE, THD_SECTORS_PER_TRACK},
};

/// Builds a synthetic NHD image with the given geometry, each sector's first
/// byte set to its LBA index (mod 256).
pub(crate) fn build_nhd_image(cylinders: u16, heads: u8, spt: u8, sector_size: u16) -> Vec<u8> {
    let header_size = NHD_HEADER_SIZE as u32;
    let mut header = vec![0u8; NHD_HEADER_SIZE];

    header[..15].copy_from_slice(NHD_SIGNATURE);
    header[0x110..0x114].copy_from_slice(&header_size.to_le_bytes());
    header[0x114..0x118].copy_from_slice(&(cylinders as u32).to_le_bytes());
    header[0x118..0x11A].copy_from_slice(&(heads as u16).to_le_bytes());
    header[0x11A..0x11C].copy_from_slice(&(spt as u16).to_le_bytes());
    header[0x11C..0x11E].copy_from_slice(&sector_size.to_le_bytes());

    let total_sectors = cylinders as usize * heads as usize * spt as usize;
    let data_size = total_sectors * sector_size as usize;
    let mut data = vec![0u8; data_size];
    for lba in 0..total_sectors {
        data[lba * sector_size as usize] = lba as u8;
    }

    header.extend_from_slice(&data);
    header
}

/// Builds a synthetic HDI image with the given geometry, each sector's first
/// byte set to its LBA index (mod 256).
pub(crate) fn build_hdi_image(cylinders: u16, heads: u8, spt: u8, sector_size: u16) -> Vec<u8> {
    let header_size = HDI_HEADER_SIZE as u32;
    let total_sectors = cylinders as u32 * heads as u32 * spt as u32;
    let mut header = vec![0u8; HDI_HEADER_SIZE];

    header[8..12].copy_from_slice(&header_size.to_le_bytes());
    header[12..16].copy_from_slice(&total_sectors.to_le_bytes());
    header[16..20].copy_from_slice(&(sector_size as u32).to_le_bytes());
    header[20..24].copy_from_slice(&(spt as u32).to_le_bytes());
    header[24..28].copy_from_slice(&(heads as u32).to_le_bytes());
    header[28..32].copy_from_slice(&(cylinders as u32).to_le_bytes());

    let data_size = total_sectors as usize * sector_size as usize;
    let mut data = vec![0u8; data_size];
    for lba in 0..total_sectors as usize {
        data[lba * sector_size as usize] = lba as u8;
    }

    header.extend_from_slice(&data);
    header
}

/// Builds a synthetic THD image with the given cylinder count, each sector's
/// first byte set to its LBA index (mod 256).
pub(crate) fn build_thd_image(cylinders: u16) -> Vec<u8> {
    let mut header = vec![0u8; THD_HEADER_SIZE];
    header[0..2].copy_from_slice(&cylinders.to_le_bytes());

    let total_sectors = cylinders as usize * THD_HEADS as usize * THD_SECTORS_PER_TRACK as usize;
    let data_size = total_sectors * THD_SECTOR_SIZE as usize;
    let mut data = vec![0u8; data_size];
    for lba in 0..total_sectors {
        data[lba * THD_SECTOR_SIZE as usize] = lba as u8;
    }

    header.extend_from_slice(&data);
    header
}

/// Writes `bytes` to a uniquely-named file in the system temp directory and
/// returns its path.
pub(crate) fn tempfile_with(bytes: &[u8], suffix: &str) -> PathBuf {
    let dir = std::env::temp_dir();
    let unique = format!(
        "neetan_hdd_test_{}_{}{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        suffix
    );
    let path = dir.join(unique);
    std::fs::write(&path, bytes).expect("write temp file");
    path
}
