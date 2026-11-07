use std::path::Path;

use device::floppy::{FloppyFormat, FloppyImage, fdi};

/// Returns the path of the test fixture `name`.
fn fixture_path(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixture")
        .join(name)
}

/// Reads the bytes of the test fixture `name`.
fn read_fixture(name: &str) -> Vec<u8> {
    let path = fixture_path(name);
    std::fs::read(&path)
        .unwrap_or_else(|error| panic!("Failed to read {}: {error}", path.display()))
}

/// Verifies that an FDI image of the HDM fixture behind the standard header holds the same disk as the HDM image.
#[test]
fn fdi_image_matches_the_hdm_image() {
    let hdm_bytes = read_fixture("blank.hdm");
    let mut fdi_bytes = fdi::standard_2hd_header().to_vec();
    fdi_bytes.extend_from_slice(&hdm_bytes);

    let hdm_image = FloppyImage::from_hdm_bytes(&hdm_bytes).expect("HDM fixture parses");
    let fdi_image = FloppyImage::from_fdi_bytes(&fdi_bytes).expect("FDI image parses");

    assert_eq!(fdi_image.format, FloppyFormat::Fdi);
    assert_eq!(fdi_image.media_type, hdm_image.media_type);
    assert_eq!(fdi_image.track_slot_count(), hdm_image.track_slot_count());
    for track_index in 0..hdm_image.track_slot_count() {
        let sector_count = hdm_image.sector_count(track_index);
        assert_eq!(
            fdi_image.sector_count(track_index),
            sector_count,
            "track {track_index}"
        );
        for slot in 0..sector_count {
            let hdm_sector = hdm_image.sector_at_index(track_index, slot).unwrap();
            let fdi_sector = fdi_image.sector_at_index(track_index, slot).unwrap();
            assert_eq!(
                (
                    fdi_sector.cylinder,
                    fdi_sector.head,
                    fdi_sector.record,
                    fdi_sector.size_code
                ),
                (
                    hdm_sector.cylinder,
                    hdm_sector.head,
                    hdm_sector.record,
                    hdm_sector.size_code
                ),
                "track {track_index} slot {slot}"
            );
            assert_eq!(
                fdi_sector.data, hdm_sector.data,
                "track {track_index} slot {slot}"
            );
        }
    }

    assert_eq!(fdi_image.physical_geometry(), hdm_image.physical_geometry());
    assert_eq!(fdi_image.to_bytes(), fdi_bytes);
}
