use std::path::Path;

use common::{Context, bail, info};
use device::{
    disk::{HddFormat, HddGeometry, HddImage},
    floppy::d88::{D88Disk, D88MediaType, D88Sector},
};

use crate::config::{FddType, HddSizeType};

pub fn create_fdd_image(path: &Path, fdd_type: FddType) -> crate::Result<()> {
    let extension = path.extension().and_then(|e| e.to_str());
    if !matches!(extension, Some("d88")) {
        bail!("output path must have a .d88 extension");
    }

    let (media_type, cylinders, heads, sectors_per_track, sector_size, size_code) = match fdd_type {
        FddType::Hd2 => (D88MediaType::Disk2HD, 77, 2, 8, 1024, 3u8),
        FddType::Dd2 => (D88MediaType::Disk2DD, 80, 2, 16, 256, 1u8),
        FddType::D2 => (D88MediaType::Disk2D, 40, 2, 16, 256, 1u8),
    };

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

    std::fs::write(path, &bytes).with_context(|| format!("failed to write {}", path.display()))?;

    info!("Created {} KB floppy image: {}", size_kb, path.display());
    Ok(())
}

pub fn create_hdd_image(path: &Path, hdd_type: HddSizeType) -> crate::Result<()> {
    let extension = path.extension().and_then(|e| e.to_str());
    if !matches!(extension, Some("hdi")) {
        bail!("output path must have a .hdi extension");
    }

    let (cylinders, heads, sectors_per_track, sector_size) = match hdd_type {
        HddSizeType::Mb5 => (153u16, 4u8, 33u8, 256u16),
        HddSizeType::Mb10 => (310, 4, 33, 256),
        HddSizeType::Mb15 => (310, 6, 33, 256),
        HddSizeType::Mb20 => (310, 8, 33, 256),
        HddSizeType::Mb30 => (615, 6, 33, 256),
        HddSizeType::Mb40 => (615, 8, 33, 256),
        HddSizeType::IdeMb40 => (977, 5, 17, 512),
        HddSizeType::IdeMb80 => (977, 10, 17, 512),
        HddSizeType::IdeMb120 => (977, 15, 17, 512),
        HddSizeType::IdeMb200 => (977, 15, 28, 512),
        HddSizeType::IdeMb500 => (1015, 16, 63, 512),
    };

    let geometry = HddGeometry {
        cylinders,
        heads,
        sectors_per_track,
        sector_size,
    };

    let data = vec![0u8; geometry.total_bytes() as usize];
    let image = HddImage::from_raw(geometry, HddFormat::Hdi, data);
    let bytes = image.to_bytes();
    let size_mb = bytes.len() / (1024 * 1024);

    std::fs::write(path, &bytes).with_context(|| format!("failed to write {}", path.display()))?;

    info!("Created {} MB hard disk image: {}", size_mb, path.display());
    Ok(())
}
