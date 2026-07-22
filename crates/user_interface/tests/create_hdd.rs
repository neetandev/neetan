//! Tests for the `neetan create-hdd` subcommand.

use std::sync::atomic::{AtomicU64, Ordering};

use device::disk::{
    HddFormat, HddImage, X68K_SASI_HDF_10MB_BYTES, X68K_SASI_HDF_20MB_BYTES,
    X68K_SASI_HDF_40MB_BYTES, load_hdd_image,
};
use user_interface::{config::HddSizeType, create::create_hdd_image};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_path(label: &str, extension: &str) -> std::path::PathBuf {
    let pid = std::process::id();
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("neetan_create_{label}_{pid}_{counter}.{extension}"))
}

#[test]
fn creates_raw_scsi_20mb_image() {
    let path = temp_path("scsi20", "h0");
    create_hdd_image(&path, HddSizeType::ScsiMb20).expect("create 20 MB SCSI image");

    let data = std::fs::read(&path).expect("read created image");
    // 20 MiB of flat sector data, no header.
    assert_eq!(data.len(), 20 * 1024 * 1024);

    let image = load_hdd_image(&path, &data).expect("parse created image");
    assert_eq!(image.format, HddFormat::Raw);
    assert_eq!(image.geometry.sector_size, 512);
    assert_eq!(image.geometry.total_sectors() as usize, data.len() / 512);

    std::fs::remove_file(&path).ok();
}

#[test]
fn creates_raw_scsi_100mb_image() {
    let path = temp_path("scsi100", "h1");
    create_hdd_image(&path, HddSizeType::ScsiMb100).expect("create 100 MB SCSI image");

    let data = std::fs::read(&path).expect("read created image");
    assert_eq!(data.len(), 100 * 1024 * 1024);

    let image = load_hdd_image(&path, &data).expect("parse created image");
    assert_eq!(image.format, HddFormat::Raw);
    assert_eq!(image.geometry.total_sectors() as usize, data.len() / 512);

    std::fs::remove_file(&path).ok();
}

#[test]
fn scsi_type_rejects_hdi_extension() {
    let path = temp_path("scsi_wrong_ext", "hdi");
    assert!(create_hdd_image(&path, HddSizeType::ScsiMb20).is_err());
    std::fs::remove_file(&path).ok();
}

#[test]
fn sasi_type_rejects_raw_extension() {
    let path = temp_path("sasi_wrong_ext", "h0");
    assert!(create_hdd_image(&path, HddSizeType::Mb5).is_err());
    std::fs::remove_file(&path).ok();
}

#[test]
fn creates_x68k_sasi_hdf_images_with_exact_sizes() {
    let cases = [
        (HddSizeType::X68kSasiMb10, X68K_SASI_HDF_10MB_BYTES),
        (HddSizeType::X68kSasiMb20, X68K_SASI_HDF_20MB_BYTES),
        (HddSizeType::X68kSasiMb40, X68K_SASI_HDF_40MB_BYTES),
    ];
    for (hdd_type, expected_bytes) in cases {
        let path = temp_path("x68sasi", "hdf");
        create_hdd_image(&path, hdd_type).expect("create X68000 SASI image");

        let data = std::fs::read(&path).expect("read created image");
        assert_eq!(data.len(), expected_bytes);

        let image = HddImage::from_x68k_sasi(data).expect("parse created image");
        assert_eq!(image.format, HddFormat::Raw);
        assert_eq!(image.geometry.sector_size, 256);

        std::fs::remove_file(&path).ok();
    }
}

#[test]
fn creates_x68k_scsi_hdf_images() {
    let cases = [
        (HddSizeType::X68kScsiMb20, 20 * 1024 * 1024),
        (HddSizeType::X68kScsiMb40, 40 * 1024 * 1024),
    ];
    for (hdd_type, expected_bytes) in cases {
        let path = temp_path("x68scsi", "hdf");
        create_hdd_image(&path, hdd_type).expect("create X68000 SCSI image");

        let data = std::fs::read(&path).expect("read created image");
        assert_eq!(data.len(), expected_bytes);

        let image = HddImage::from_raw_flat(data).expect("parse created image");
        assert_eq!(image.format, HddFormat::Raw);
        assert_eq!(image.geometry.sector_size, 512);

        std::fs::remove_file(&path).ok();
    }
}

#[test]
fn creates_at_flat_images_with_derived_geometry() {
    let cases = [
        (HddSizeType::AtMb40, 81u16),
        (HddSizeType::AtMb100, 203),
        (HddSizeType::AtMb250, 507),
        (HddSizeType::AtMb504, 1023),
    ];
    for (hdd_type, expected_cylinders) in cases {
        let path = temp_path("atflat", "hdd");
        create_hdd_image(&path, hdd_type).expect("create AT flat image");

        let data = std::fs::read(&path).expect("read created image");
        assert_eq!(data.len(), expected_cylinders as usize * 16 * 63 * 512);

        let image = load_hdd_image(&path, &data).expect("parse created image");
        assert_eq!(image.format, HddFormat::AtFlat);
        assert_eq!(image.geometry.cylinders, expected_cylinders);
        assert_eq!(image.geometry.heads, 16);
        assert_eq!(image.geometry.sectors_per_track, 63);
        assert_eq!(image.geometry.sector_size, 512);

        std::fs::remove_file(&path).ok();
    }
}

#[test]
fn at_type_rejects_other_extensions() {
    let path = temp_path("at_wrong_ext", "hdi");
    assert!(create_hdd_image(&path, HddSizeType::AtMb40).is_err());
    let path = temp_path("at_wrong_ext", "img");
    assert!(create_hdd_image(&path, HddSizeType::AtMb100).is_err());
    std::fs::remove_file(&path).ok();
}

#[test]
fn x68k_type_rejects_other_extensions() {
    let path = temp_path("x68_wrong_ext", "hdi");
    assert!(create_hdd_image(&path, HddSizeType::X68kSasiMb10).is_err());
    let path = temp_path("x68_wrong_ext", "h0");
    assert!(create_hdd_image(&path, HddSizeType::X68kScsiMb20).is_err());
    std::fs::remove_file(&path).ok();
}
