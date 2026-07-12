use std::path::Path;

use device::floppy::{FloppyFormat, d88::D88MediaType, img, load_floppy_image};

/// Byte sizes of the six recognized IBM PC floppy formats.
const SIZE_360K: usize = 368_640;
const SIZE_720K: usize = 737_280;
const SIZE_1200K: usize = 1_228_800;
const SIZE_1232K: usize = 1_261_568;
const SIZE_1440K: usize = 1_474_560;
const SIZE_IBM_XDF: usize = 1_884_160;

fn load_img(name: &str, data: &[u8]) -> device::floppy::FloppyImage {
    load_floppy_image(Path::new(name), data)
        .unwrap_or_else(|error| panic!("Failed to load {name}: {error}"))
}

#[test]
fn geometry_detection_all_sizes() {
    let cases = [
        (SIZE_360K, 40u8, 9u8, 512usize, D88MediaType::Disk2D),
        (SIZE_720K, 80, 9, 512, D88MediaType::Disk2DD),
        (SIZE_1200K, 80, 15, 512, D88MediaType::Disk2HD),
        (SIZE_1232K, 77, 8, 1024, D88MediaType::Disk2HD),
        (SIZE_1440K, 80, 18, 512, D88MediaType::Disk2HD),
    ];
    for (size, cylinders, sectors, sector_size, media_type) in cases {
        let geometry = img::detect_geometry(size)
            .unwrap_or_else(|| panic!("size {size} should be recognized"));
        assert_eq!(geometry.cylinders, cylinders);
        assert_eq!(geometry.heads, 2);
        assert_eq!(geometry.sectors, sectors);
        assert_eq!(geometry.sector_size, sector_size);
        assert_eq!(geometry.media_type, media_type);
    }
}

#[test]
fn reject_unknown_sizes() {
    for size in [0usize, 512, SIZE_360K - 512, SIZE_1440K + 512, 2_949_120] {
        assert!(
            img::detect_geometry(size).is_none(),
            "size {size} must not be recognized"
        );
        assert!(img::from_bytes(&vec![0u8; size]).is_err());
    }
}

#[test]
fn img_extension_dispatch() {
    for size in [SIZE_360K, SIZE_720K, SIZE_1200K, SIZE_1232K, SIZE_1440K] {
        let image = load_img("test.img", &vec![0u8; size]);
        assert_eq!(image.format, FloppyFormat::Img, "size {size}");
    }
    let image = load_img("test.ima", &vec![0u8; SIZE_1440K]);
    assert_eq!(image.format, FloppyFormat::Img);
}

#[test]
fn img_extension_dispatches_ibm_xdf_by_size() {
    let image = load_img("pcdos7.img", &vec![0u8; SIZE_IBM_XDF]);
    assert_eq!(image.format, FloppyFormat::IbmXdf);
}

#[test]
fn xdf_extension_disambiguates_by_size() {
    // 1,261,568 bytes: the X68000 raw 2HD format keeps its existing path.
    let image = load_img("game.xdf", &vec![0u8; SIZE_1232K]);
    assert_eq!(image.format, FloppyFormat::Xdf);

    // 1,884,160 bytes: IBM XDF.
    let image = load_img("pcdos7.xdf", &vec![0u8; SIZE_IBM_XDF]);
    assert_eq!(image.format, FloppyFormat::IbmXdf);
}

#[test]
fn img_roundtrip_through_public_api() {
    let mut data = vec![0u8; SIZE_1440K];
    for (i, byte) in data.iter_mut().enumerate() {
        *byte = (i % 251) as u8;
    }
    let image = load_img("test.img", &data);
    assert_eq!(image.to_bytes(), data);
}

#[test]
fn img_sector_layout_1440k() {
    let mut data = vec![0u8; SIZE_1440K];
    // Mark C=0 H=0 R=1 and C=0 H=1 R=1 (offset 18 * 512).
    data[0] = 0xA1;
    data[18 * 512] = 0xB2;
    let image = load_img("test.img", &data);

    let sector = image.find_sector(0, 0, 1, 2).unwrap();
    assert_eq!(sector.data[0], 0xA1);
    let sector = image.find_sector(0, 1, 1, 2).unwrap();
    assert_eq!(sector.data[0], 0xB2);
    assert!(image.find_sector(0, 0, 19, 2).is_none());
}
