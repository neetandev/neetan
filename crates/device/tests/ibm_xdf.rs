use std::path::Path;

use device::floppy::{FloppyFormat, ibm_xdf, load_floppy_image};

/// Exact byte size of an IBM XDF image.
const IBM_XDF_FILE_SIZE: usize = 1_884_160;
/// Byte size of one per-cylinder blob (both heads).
const CYLINDER_BLOB_BYTES: usize = 23_552;

fn build_pattern_image() -> Vec<u8> {
    let mut data = vec![0u8; IBM_XDF_FILE_SIZE];
    for (i, byte) in data.iter_mut().enumerate() {
        *byte = (i % 253) as u8;
    }
    data
}

#[test]
fn cylinder0_logical_to_physical_mapping() {
    let mut data = vec![0u8; IBM_XDF_FILE_SIZE];
    // Logical blocks 0-45 of cylinder 0, stamped with their block number.
    for block in 0..46 {
        data[block * 512] = 0x80 | block as u8;
    }
    let disk = ibm_xdf::from_bytes(&data).unwrap();

    // Boot sector: block 0 = head 0, ID 129.
    assert_eq!(disk.find_sector(0, 0, 129, 2).unwrap().data[0], 0x80);
    // FAT blocks 1-10 = head 0, IDs 130-139.
    for id in 130..=139u8 {
        let block = id - 129;
        assert_eq!(
            disk.find_sector(0, 0, id, 2).unwrap().data[0],
            0x80 | block,
            "head 0 ID {id}"
        );
    }
    // FAT block 11 = head 1, ID 129.
    assert_eq!(disk.find_sector(0, 1, 129, 2).unwrap().data[0], 0x80 | 11);
    // Aux FS blocks 12-19 = head 0, IDs 1-8.
    for id in 1..=8u8 {
        let block = id + 11;
        assert_eq!(
            disk.find_sector(0, 0, id, 2).unwrap().data[0],
            0x80 | block,
            "head 0 aux ID {id}"
        );
    }
    // Root directory blocks 23-36 = head 1, IDs 130-143.
    for id in 130..=143u8 {
        let block = id - 130 + 23;
        assert_eq!(
            disk.find_sector(0, 1, id, 2).unwrap().data[0],
            0x80 | block,
            "head 1 ID {id}"
        );
    }
    // Data area blocks 42-45 = head 1, IDs 144-147.
    for id in 144..=147u8 {
        let block = id - 130 + 28;
        assert_eq!(
            disk.find_sector(0, 1, id, 2).unwrap().data[0],
            0x80 | block,
            "head 1 data ID {id}"
        );
    }
}

#[test]
fn mixed_size_sector_reads() {
    let data = build_pattern_image();
    let disk = ibm_xdf::from_bytes(&data).unwrap();

    // Cylinder 3, head 0: 1024 at blob offset 0, 512 at 11264,
    // 2048 at 1024, 8192 at 12288.
    let blob = 3 * CYLINDER_BLOB_BYTES;
    let cases_head0 = [
        (131u8, 3u8, 1024usize, 0usize),
        (130, 2, 512, 11_264),
        (132, 4, 2048, 1_024),
        (134, 6, 8192, 12_288),
    ];
    for (id, size_code, size, offset) in cases_head0 {
        let sector = disk.find_sector(3, 0, id, size_code).unwrap();
        assert_eq!(sector.data.len(), size, "head 0 ID {id}");
        assert_eq!(sector.data[..], data[blob + offset..blob + offset + size]);
        assert_eq!(sector.source_offset, Some((blob + offset) as u64));
    }
    // Cylinder 3, head 1: 2048 at 20480, 512 at 11776, 1024 at 22528,
    // 8192 at 3072.
    let cases_head1 = [
        (132u8, 4u8, 2048usize, 20_480usize),
        (130, 2, 512, 11_776),
        (131, 3, 1024, 22_528),
        (134, 6, 8192, 3_072),
    ];
    for (id, size_code, size, offset) in cases_head1 {
        let sector = disk.find_sector(3, 1, id, size_code).unwrap();
        assert_eq!(sector.data.len(), size, "head 1 ID {id}");
        assert_eq!(sector.data[..], data[blob + offset..blob + offset + size]);
        assert_eq!(sector.source_offset, Some((blob + offset) as u64));
    }
}

#[test]
fn full_image_readback_matches_flat_image() {
    let mut data = build_pattern_image();
    // The cylinder-0 padding blocks (20-22, 37-41) are not mapped by any
    // sector and re-emit as zeros.
    for block in (20..=22).chain(37..=41) {
        data[block * 512..(block + 1) * 512].fill(0);
    }
    let disk = ibm_xdf::from_bytes(&data).unwrap();
    assert_eq!(ibm_xdf::to_bytes(&disk), data);
}

#[test]
fn public_api_roundtrip() {
    let mut data = build_pattern_image();
    for block in (20..=22).chain(37..=41) {
        data[block * 512..(block + 1) * 512].fill(0);
    }
    let image = load_floppy_image(Path::new("pcdos7.xdf"), &data).unwrap();
    assert_eq!(image.format, FloppyFormat::IbmXdf);
    assert_eq!(image.to_bytes(), data);
}
