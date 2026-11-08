//! FAT12 floppy formatting: the boot sector, both FAT copies and the empty root
//! directory of the floppy formats of the DOS machine families.

use super::{FloppyGeometry, FloppyImage};
use crate::disk::format::{
    BpbParams, DOS_BOOT_JUMP, FormatError, fat_region, write_bpb_fields,
    write_extended_bpb_and_signature,
};

/// The machine family whose FAT12 floppy formats a formatter writes.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum FloppyLayout {
    /// NEC PC-98 with MS-DOS: 2HD 1.2 MB and 2DD 640 KB.
    Pc98,
    /// IBM PC/AT with DOS: 2HD 1.44 MB and 2DD 720 KB.
    At,
    /// Fujitsu FM Towns with Towns OS: 2HD 1.2 MB, 2HC 1.2 MB and 2HD 1.44 MB.
    Towns,
    /// MSX with MSX-DOS: 2DD 720 KB.
    Msx,
}

/// BIOS drive number of the first floppy drive, stored in the extended BPB.
const FLOPPY_PHYSICAL_DRIVE_NUMBER: u8 = 0x00;

/// The `JMP $` and `NOP` at the start of an MSX-DOS boot sector.
const MSX_BOOT_JUMP: [u8; 3] = [0xEB, 0xFE, 0x90];

/// Offset of the boot routine that the MSX disk ROM calls at `C01Eh`.
const MSX_BOOT_ROUTINE_OFFSET: usize = 0x1E;

/// The Z80 `RET` that ends the MSX boot routine. The disk ROM then starts
/// Disk BASIC.
const Z80_RET: u8 = 0xC9;

/// Returns the FAT12 BPB of a floppy with 2 heads, `sectors_per_track` sectors
/// of `bytes_per_sector` bytes, `sectors_per_cluster`, `root_entry_count`,
/// `media_descriptor` and `sectors_per_fat`.
const fn floppy_bpb(
    bytes_per_sector: u16,
    sectors_per_cluster: u8,
    root_entry_count: u16,
    media_descriptor: u8,
    sectors_per_fat: u16,
) -> BpbParams {
    BpbParams {
        bytes_per_sector,
        sectors_per_cluster,
        reserved_sectors: 1,
        num_fats: 2,
        root_entry_count,
        media_descriptor,
        sectors_per_fat,
        is_fat16: false,
    }
}

/// 2HD 1.2 MB with 8 sectors of 1024 bytes on 77 cylinders.
const BPB_2HD_1024: BpbParams = floppy_bpb(1024, 1, 192, 0xFE, 2);
/// 2DD 640 KB with 8 sectors of 512 bytes on 80 cylinders.
const BPB_2DD_640: BpbParams = floppy_bpb(512, 2, 112, 0xFE, 3);
/// 2HC 1.2 MB with 15 sectors of 512 bytes on 80 cylinders.
const BPB_2HC_1200: BpbParams = floppy_bpb(512, 1, 224, 0xF9, 7);
/// 2HD 1.44 MB with 18 sectors of 512 bytes on 80 cylinders.
const BPB_2HD_1440: BpbParams = floppy_bpb(512, 1, 224, 0xF0, 9);
/// 2DD 720 KB with 9 sectors of 512 bytes on 80 cylinders.
const BPB_2DD_720: BpbParams = floppy_bpb(512, 2, 112, 0xF9, 3);

/// Returns the FAT12 BPB of a floppy of `geometry` on `layout`, or `None` for a
/// geometry the family does not format.
pub fn floppy_bpb_params(layout: FloppyLayout, geometry: &FloppyGeometry) -> Option<BpbParams> {
    if geometry.heads != 2 {
        return None;
    }
    let shape = (
        geometry.sector_size,
        geometry.sectors_per_track,
        geometry.cylinders,
    );
    match (layout, shape) {
        (FloppyLayout::Pc98 | FloppyLayout::Towns, (1024, 8, 77)) => Some(BPB_2HD_1024),
        (FloppyLayout::Pc98, (512, 8, 80)) => Some(BPB_2DD_640),
        (FloppyLayout::Towns, (512, 15, 80)) => Some(BPB_2HC_1200),
        (FloppyLayout::At | FloppyLayout::Towns, (512, 18, 80)) => Some(BPB_2HD_1440),
        (FloppyLayout::At | FloppyLayout::Msx, (512, 9, 80)) => Some(BPB_2DD_720),
        _ => None,
    }
}

/// Returns the total sector count of a floppy of `geometry`.
fn total_sectors(geometry: &FloppyGeometry) -> u32 {
    u32::from(geometry.cylinders)
        * u32::from(geometry.heads)
        * u32::from(geometry.sectors_per_track)
}

/// Builds the DOS boot sector of a floppy of `geometry` with `bpb`: the BPB,
/// the extended BPB, a parking loop and the `55 AA` signature.
pub fn floppy_boot_sector(bpb: &BpbParams, geometry: &FloppyGeometry) -> Vec<u8> {
    let mut boot = vec![0u8; usize::from(geometry.sector_size)];
    write_bpb_fields(
        &mut boot,
        DOS_BOOT_JUMP,
        bpb,
        total_sectors(geometry),
        geometry.sectors_per_track,
        geometry.heads,
        0,
    );
    write_extended_bpb_and_signature(&mut boot, bpb, FLOPPY_PHYSICAL_DRIVE_NUMBER);
    boot
}

/// Builds the MSX-DOS boot sector of a floppy of `geometry` with `bpb`: the
/// BPB and a boot routine that returns to the disk ROM.
fn msx_boot_sector(bpb: &BpbParams, geometry: &FloppyGeometry) -> Vec<u8> {
    let mut boot = vec![0u8; usize::from(geometry.sector_size)];
    write_bpb_fields(
        &mut boot,
        MSX_BOOT_JUMP,
        bpb,
        total_sectors(geometry),
        geometry.sectors_per_track,
        geometry.heads,
        0,
    );
    boot[MSX_BOOT_ROUTINE_OFFSET] = Z80_RET;
    boot
}

/// Writes `data` to the logical sector `lba` of `image`.
fn write_logical_sector(
    image: &mut FloppyImage,
    geometry: &FloppyGeometry,
    lba: u32,
    data: &[u8],
) -> Result<(), FormatError> {
    let sectors_per_track = u32::from(geometry.sectors_per_track);
    let heads = u32::from(geometry.heads);
    let track = lba / sectors_per_track;
    let rank = (lba % sectors_per_track) as usize;
    let cylinder = u8::try_from(track / heads).map_err(|_| FormatError::SectorOutOfRange)?;
    let head = (track % heads) as u8;
    let (id_cylinder, id_head, record, size_code) = image
        .logical_sector_id(geometry, cylinder, head, rank)
        .ok_or(FormatError::SectorOutOfRange)?;
    let track_index = usize::from(cylinder) * 2 + usize::from(head);
    image
        .write_sector_payload(track_index, id_cylinder, id_head, record, size_code, data)
        .map(|_| ())
        .ok_or(FormatError::SectorOutOfRange)
}

/// Formats an in-memory floppy image with the FAT12 format of `layout`: writes
/// the boot sector, both FAT copies and an empty root directory. The data area
/// keeps its content.
pub fn format_floppy_image(
    image: &mut FloppyImage,
    layout: FloppyLayout,
) -> Result<(), FormatError> {
    let geometry = image
        .physical_geometry()
        .ok_or(FormatError::UnsupportedGeometry)?;
    let bpb = floppy_bpb_params(layout, &geometry).ok_or(FormatError::UnsupportedGeometry)?;
    let sector_size = geometry.sector_size;

    let boot = match layout {
        FloppyLayout::Pc98 | FloppyLayout::At | FloppyLayout::Towns => {
            floppy_boot_sector(&bpb, &geometry)
        }
        FloppyLayout::Msx => msx_boot_sector(&bpb, &geometry),
    };
    write_logical_sector(image, &geometry, 0, &boot)?;

    let fat = fat_region(&bpb, sector_size);
    let fat_base = u32::from(bpb.reserved_sectors);
    for fat_index in 0..u32::from(bpb.num_fats) {
        let base = fat_base + fat_index * u32::from(bpb.sectors_per_fat);
        for (offset, chunk) in fat.chunks_exact(usize::from(sector_size)).enumerate() {
            write_logical_sector(image, &geometry, base + offset as u32, chunk)?;
        }
    }

    let root_base = fat_base + u32::from(bpb.num_fats) * u32::from(bpb.sectors_per_fat);
    let root_dir_sectors = (u32::from(bpb.root_entry_count) * 32).div_ceil(u32::from(sector_size));
    let empty = vec![0u8; usize::from(sector_size)];
    for offset in 0..root_dir_sectors {
        write_logical_sector(image, &geometry, root_base + offset, &empty)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::floppy::{
        FddType,
        blank::{FloppyContainer, blank_floppy_image_bytes},
        load_floppy_image,
    };

    /// Builds a blank image of `fdd_type` in `container` and parses it as `name`.
    fn blank_image(fdd_type: FddType, container: FloppyContainer, name: &str) -> FloppyImage {
        let bytes = blank_floppy_image_bytes(fdd_type, container);
        load_floppy_image(Path::new(name), &bytes).expect("parse blank image")
    }

    /// Reads the logical sector `lba` of `image`.
    fn read_logical_sector(image: &FloppyImage, lba: u32) -> Vec<u8> {
        let geometry = image.physical_geometry().expect("geometry");
        let sectors_per_track = u32::from(geometry.sectors_per_track);
        let track = lba / sectors_per_track;
        let cylinder = (track / 2) as u8;
        let head = (track % 2) as u8;
        let rank = (lba % sectors_per_track) as usize;
        let (c, h, r, n) = image
            .logical_sector_id(&geometry, cylinder, head, rank)
            .expect("sector id");
        image
            .find_sector_near_track_index(usize::from(cylinder) * 2 + usize::from(head), c, h, r, n)
            .expect("sector")
            .data
            .clone()
    }

    /// Verifies the BPB, the FAT media byte and the root directory of every
    /// supported layout and geometry, after a round trip through the bytes.
    #[test]
    fn every_supported_floppy_formats_and_parses_back() {
        let cases = [
            (
                FloppyLayout::Pc98,
                FddType::Hd2,
                FloppyContainer::D88,
                "f.d88",
                0xFE,
            ),
            (
                FloppyLayout::Pc98,
                FddType::Hd2,
                FloppyContainer::Fdi,
                "f.fdi",
                0xFE,
            ),
            (
                FloppyLayout::Towns,
                FddType::Hd2,
                FloppyContainer::D88,
                "f.d88",
                0xFE,
            ),
            (
                FloppyLayout::Towns,
                FddType::Hd2Fmt144,
                FloppyContainer::D88,
                "f.d88",
                0xF0,
            ),
            (
                FloppyLayout::At,
                FddType::Hd2Fmt144,
                FloppyContainer::Raw,
                "f.img",
                0xF0,
            ),
            (
                FloppyLayout::At,
                FddType::Dd2Fmt720,
                FloppyContainer::Raw,
                "f.img",
                0xF9,
            ),
            (
                FloppyLayout::Msx,
                FddType::Dd2Fmt720,
                FloppyContainer::Raw,
                "f.dsk",
                0xF9,
            ),
        ];
        for (layout, fdd_type, container, name, media) in cases {
            let tag = format!("{layout:?} {fdd_type:?} {name}");
            let mut image = blank_image(fdd_type, container, name);
            format_floppy_image(&mut image, layout)
                .unwrap_or_else(|error| panic!("{tag}: {error}"));
            let image = load_floppy_image(Path::new(name), &image.to_bytes()).expect("reparse");

            let geometry = fdd_type.geometry();
            let bpb = floppy_bpb_params(layout, &geometry).expect("bpb");
            let boot = read_logical_sector(&image, 0);
            assert_eq!(
                u16::from_le_bytes([boot[11], boot[12]]),
                geometry.sector_size,
                "{tag}"
            );
            assert_eq!(
                u32::from(u16::from_le_bytes([boot[19], boot[20]])),
                total_sectors(&geometry),
                "{tag}"
            );
            assert_eq!(boot[21], media, "{tag}");
            match layout {
                FloppyLayout::Msx => {
                    assert_eq!(boot[0..3], MSX_BOOT_JUMP, "{tag}");
                    assert_eq!(boot[MSX_BOOT_ROUTINE_OFFSET], Z80_RET, "{tag}");
                }
                FloppyLayout::Pc98 | FloppyLayout::At | FloppyLayout::Towns => {
                    assert_eq!(boot[0..3], DOS_BOOT_JUMP, "{tag}");
                    assert_eq!(boot[boot.len() - 2..], [0x55, 0xAA], "{tag}");
                }
            }

            for fat_index in 0..u32::from(bpb.num_fats) {
                let fat =
                    read_logical_sector(&image, 1 + fat_index * u32::from(bpb.sectors_per_fat));
                assert_eq!(fat[0..3], [media, 0xFF, 0xFF], "{tag} FAT {fat_index}");
            }
        }
    }

    /// Verifies that a geometry outside the formats of the family is rejected.
    #[test]
    fn unsupported_geometry_is_rejected() {
        let mut image = blank_image(FddType::Hd2Fmt144, FloppyContainer::D88, "f.d88");
        assert_eq!(
            format_floppy_image(&mut image, FloppyLayout::Pc98),
            Err(FormatError::UnsupportedGeometry)
        );
        let mut image = blank_image(FddType::D2, FloppyContainer::D88, "f.d88");
        assert_eq!(
            format_floppy_image(&mut image, FloppyLayout::At),
            Err(FormatError::UnsupportedGeometry)
        );
    }
}
