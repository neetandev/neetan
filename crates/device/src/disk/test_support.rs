//! Shared test-only image builders for the disk modules.

use crate::disk::{
    HddFormat, HddGeometry, HddImage,
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

/// Builds an image laid out in its image geometry like NEC FDISK and
/// FORMAT with one partition from cylinder 1 to cylinder `cylinders - 5`.
/// Every sector outside the partition table and the boot record holds its
/// LBA.
pub(crate) fn image_in_image_geometry(
    cylinders: u16,
    heads: u8,
    sectors_per_track: u8,
) -> HddImage {
    let span = u32::from(cylinders - 5) * u32::from(heads) * u32::from(sectors_per_track);
    let volume = (span - u32::from(heads) * u32::from(sectors_per_track)) & !3;
    pc98_image_in_image_geometry(
        cylinders,
        heads,
        sectors_per_track,
        &[(1, cylinders - 5, volume)],
    )
}

/// Builds an image laid out in its image geometry like NEC FDISK and FORMAT.
/// Each partition is its first cylinder, its last cylinder and the size of
/// its volume in 512-byte sectors, a multiple of 4. The boot record of each
/// volume has 2048-byte logical sectors. Every sector outside the partition
/// table and the boot records holds its LBA.
pub(crate) fn pc98_image_in_image_geometry(
    cylinders: u16,
    heads: u8,
    sectors_per_track: u8,
    partitions: &[(u16, u16, u32)],
) -> HddImage {
    let geometry = HddGeometry {
        cylinders,
        heads,
        sectors_per_track,
        sector_size: 512,
    };
    let mut data = vec![0u8; geometry.total_bytes() as usize];
    for (lba, sector) in data.as_chunks_mut::<512>().0.iter_mut().enumerate() {
        sector[..4].copy_from_slice(&(lba as u32).to_le_bytes());
    }
    let mut image = HddImage::from_raw(geometry, HddFormat::Hdi, data);
    let sectors_per_cylinder = u32::from(heads) * u32::from(sectors_per_track);

    let mut table = vec![0u8; 512];
    for (index, &(first_cylinder, last_cylinder, volume)) in partitions.iter().enumerate() {
        let entry = &mut table[index * 32..][..32];
        entry[0] = 0xA1;
        entry[1] = 0x91;
        entry[6..8].copy_from_slice(&first_cylinder.to_le_bytes());
        entry[10..12].copy_from_slice(&first_cylinder.to_le_bytes());
        entry[14..16].copy_from_slice(&last_cylinder.to_le_bytes());
        entry[16..32].copy_from_slice(b"MS-DOS 6.20     ");

        let start = u32::from(first_cylinder) * sectors_per_cylinder;
        let logical_sectors = volume / 4;
        let mut boot_record = image.read_sector(start).unwrap().to_vec();
        boot_record[..3].copy_from_slice(&[0xEB, 0x45, 0x90]);
        boot_record[11..13].copy_from_slice(&2048u16.to_le_bytes());
        match u16::try_from(logical_sectors) {
            Ok(total) => boot_record[19..21].copy_from_slice(&total.to_le_bytes()),
            Err(_) => {
                boot_record[19..21].copy_from_slice(&0u16.to_le_bytes());
                boot_record[32..36].copy_from_slice(&logical_sectors.to_le_bytes());
            }
        }
        boot_record[22..24].copy_from_slice(&20u16.to_le_bytes());
        boot_record[24..26].copy_from_slice(&u16::from(sectors_per_track).to_le_bytes());
        boot_record[26..28].copy_from_slice(&u16::from(heads).to_le_bytes());
        boot_record[28..32].copy_from_slice(&start.to_le_bytes());
        image.write_sector(start, &boot_record);
    }
    image.write_sector(1, &table);
    image
}

/// Builds an image of a PC-98 IDE disk type formatted by `format_hdd_image`
/// before the BIOS geometry: the partition table and the boot record use
/// the image geometry, the volume starts on head 1 of cylinder 0 and fills
/// the rest of the disk. Every sector outside the partition table and the
/// boot record holds its LBA.
pub(crate) fn neetan_formatted_in_image_geometry(size: crate::disk::HddSizeType) -> HddImage {
    use crate::disk::format::{hdd_boot_sector, hdd_bpb_params, pc98_partition_table_sector};

    let (geometry, format) = size.geometry();
    let mut data = vec![0u8; geometry.total_bytes() as usize];
    for (lba, sector) in data.as_chunks_mut::<512>().0.iter_mut().enumerate() {
        sector[..4].copy_from_slice(&(lba as u32).to_le_bytes());
    }
    let mut image = HddImage::from_raw(geometry, format, data);
    let total_sectors = geometry.total_sectors();
    let start = u32::from(geometry.sectors_per_track);
    let partition_sectors = total_sectors - start;
    image.write_sector(
        1,
        &pc98_partition_table_sector(
            512,
            total_sectors,
            geometry.heads,
            geometry.sectors_per_track,
        ),
    );
    let bpb = hdd_bpb_params(512, partition_sectors);
    let boot = hdd_boot_sector(
        &bpb,
        512,
        partition_sectors,
        geometry.sectors_per_track,
        geometry.heads,
        start,
    );
    image.write_sector(start, &boot);
    image
}

/// Builds an empty HDI image of a SASI disk with 256-byte sectors and the
/// neetan partition name. Its volume starts on head 1 of cylinder 0 and
/// fills the rest of the disk.
pub(crate) fn neetan_formatted_sasi_in_cylinder_0(geometry: HddGeometry) -> HddImage {
    use crate::disk::format::{
        fat_region, hdd_boot_sector, hdd_bpb_params, pc98_partition_table_sector,
    };

    let mut image = HddImage::from_raw(
        geometry,
        HddFormat::Hdi,
        vec![0u8; geometry.total_bytes() as usize],
    );
    let total_sectors = geometry.total_sectors();
    let start = u32::from(geometry.sectors_per_track);
    let partition_sectors = total_sectors - start;
    image.write_sector(
        1,
        &pc98_partition_table_sector(
            256,
            total_sectors,
            geometry.heads,
            geometry.sectors_per_track,
        ),
    );
    let bpb = hdd_bpb_params(256, partition_sectors);
    let boot = hdd_boot_sector(
        &bpb,
        256,
        partition_sectors,
        geometry.sectors_per_track,
        geometry.heads,
        start,
    );
    image.write_sector(start, &boot);
    let fat = fat_region(&bpb, 256);
    for copy in 0..u32::from(bpb.num_fats) {
        let base = start + u32::from(bpb.reserved_sectors) + copy * u32::from(bpb.sectors_per_fat);
        for (offset, sector) in fat.as_chunks::<256>().0.iter().enumerate() {
            image.write_sector(base + offset as u32, sector);
        }
    }
    image
}
