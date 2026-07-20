use std::path::Path;

use common::{Context, bail, info};
use device::disk::{HddFormat, HddGeometry, HddImage, load_hdd_image};

const IDE_GEOMETRIES: [(u16, u8, u8, u16); 5] = [
    (977, 5, 17, 512),   // ide40
    (977, 10, 17, 512),  // ide80
    (977, 15, 17, 512),  // ide120
    (977, 15, 28, 512),  // ide200
    (1015, 16, 63, 512), // ide500
];

const SASI_GEOMETRIES: [(u16, u8, u8, u16); 6] = [
    (153, 4, 33, 256), // sasi5
    (310, 4, 33, 256), // sasi10
    (310, 6, 33, 256), // sasi15
    (310, 8, 33, 256), // sasi20
    (615, 6, 33, 256), // sasi30
    (615, 8, 33, 256), // sasi40
];

fn find_smallest_geometry(
    geometries: &[(u16, u8, u8, u16)],
    minimum_bytes: u64,
) -> Option<HddGeometry> {
    geometries
        .iter()
        .find_map(|&(cylinders, heads, sectors_per_track, sector_size)| {
            let geometry = HddGeometry {
                cylinders,
                heads,
                sectors_per_track,
                sector_size,
            };
            if geometry.total_bytes() >= minimum_bytes {
                Some(geometry)
            } else {
                None
            }
        })
}

pub fn convert_hdd_image(input: &Path, output: &Path) -> crate::Result<()> {
    let extension = output.extension().and_then(|e| e.to_str());
    if !matches!(extension, Some("hdi")) {
        bail!("output path must have a .hdi extension");
    }

    let data =
        std::fs::read(input).with_context(|| format!("failed to read {}", input.display()))?;
    let source = load_hdd_image(input, &data)
        .with_context(|| format!("failed to parse {}", input.display()))?;

    let source_bytes = source.geometry.total_bytes();

    let (target_geometry, direction) = match source.geometry.sector_size {
        256 => {
            let geometry =
                find_smallest_geometry(&IDE_GEOMETRIES, source_bytes).ok_or_else(|| {
                    common::StringError(format!(
                        "SASI image is {} bytes but the largest IDE geometry only holds {} bytes",
                        source_bytes,
                        HddGeometry {
                            cylinders: 1015,
                            heads: 16,
                            sectors_per_track: 63,
                            sector_size: 512,
                        }
                        .total_bytes()
                    ))
                })?;
            (geometry, "SASI to IDE")
        }
        512 => {
            let geometry =
                find_smallest_geometry(&SASI_GEOMETRIES, source_bytes).ok_or_else(|| {
                    common::StringError(format!(
                        "IDE image is {} bytes but the largest SASI geometry only holds {} bytes",
                        source_bytes,
                        HddGeometry {
                            cylinders: 615,
                            heads: 8,
                            sectors_per_track: 33,
                            sector_size: 256,
                        }
                        .total_bytes()
                    ))
                })?;
            (geometry, "IDE to SASI")
        }
        other => bail!("unsupported sector size: {} bytes", other),
    };

    let target_capacity = target_geometry.total_bytes() as usize;
    let mut target_data = vec![0u8; target_capacity];
    target_data[..source.data().len()].copy_from_slice(source.data());

    let target_image = HddImage::from_raw(target_geometry, HddFormat::Hdi, target_data);
    let bytes = target_image.to_bytes();

    std::fs::write(output, &bytes)
        .with_context(|| format!("failed to write {}", output.display()))?;

    info!(
        "Converted {} ({} cyl, {} heads, {} spt, {} B/sector, {} bytes) to ({} cyl, {} heads, {} spt, {} B/sector, {} bytes): {}",
        direction,
        source.geometry.cylinders,
        source.geometry.heads,
        source.geometry.sectors_per_track,
        source.geometry.sector_size,
        source_bytes,
        target_geometry.cylinders,
        target_geometry.heads,
        target_geometry.sectors_per_track,
        target_geometry.sector_size,
        target_geometry.total_bytes(),
        output.display(),
    );

    Ok(())
}
