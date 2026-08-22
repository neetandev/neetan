//! MSX ROM set loading.
//!
//! ROMs are selected by content hash rather than file name: the loader scans
//! every file in the ROM directory, computes its BLAKE3 digest, and matches it
//! against a table of accepted digests per slot. Any dump layout works regardless
//! of how the files are named, and stray files are ignored.

use std::{collections::HashMap, fmt, path::Path, sync::Arc};

use crate::{FirmwareRegion, MsxModel};

/// Size of the Panasonic FS-CA1 firmware.
const FS_CA1_FIRMWARE_SIZE: usize = 0x20_000;
/// BLAKE3 of the clean Panasonic FS-CA1 EEPROM dump.
const FS_CA1_CLEAN_BLAKE3: &str =
    "fa7cf919162d7118e4b434b9442501d166b18c127521c1614506f7535c02f69a";
/// BLAKE3 of the MAME Panasonic FS-CA1 firmware variant.
const FS_CA1_MAME_BLAKE3: &str = "5a2241ea860d89951ca2eab1402d22b4d6c06a4b40929e9d0f5b11c7ef62e257";
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum FirmwareImage {
    Hb201Bios,
    Hb201PersonalDataBank,
    HbF1XdBios,
    HbF1XdSub,
    HbF1XdDisk,
    HbF1XdJMain,
    HbF1XdJFirmware,
    HbF1XdJKanji,
    HbF1XdJOpllInstruments,
    PanasonicFsCa1,
}

/// One ROM slot and the BLAKE3 digests accepted as valid content for it.
#[derive(Clone, Copy)]
struct RomSlot<'a> {
    image: FirmwareImage,
    label: &'a str,
    size: usize,
    accepted: &'a [&'a str],
}

#[derive(Clone, Copy)]
struct FirmwareRegionSpec<'a> {
    region: FirmwareRegion,
    image: FirmwareImage,
    offset: usize,
    size: usize,
    digest: &'a str,
}

/// Physical firmware images used by the HB-201.
const HB201_IMAGES: &[RomSlot<'static>] = &[
    image(
        FirmwareImage::Hb201Bios,
        "HB-201 BIOS",
        0x8000,
        &["038b7767b129a59483f1b0f0e94bfc479c3895a7222928515e8d0084736033aa"],
    ),
    image(
        FirmwareImage::Hb201PersonalDataBank,
        "HB-201 Personal Data Bank",
        0x4000,
        &["7acf34a44a5798c4ca4382e33c8096347de552633722533878ab24b3d4a002b3"],
    ),
    FS_CA1_SLOT,
];
/// Logical firmware regions supplied by the HB-201 images.
const HB201_REGIONS: &[FirmwareRegionSpec<'static>] = &[
    region(
        FirmwareRegion::Bios,
        FirmwareImage::Hb201Bios,
        0,
        0x8000,
        "038b7767b129a59483f1b0f0e94bfc479c3895a7222928515e8d0084736033aa",
    ),
    region(
        FirmwareRegion::PersonalDataBank,
        FirmwareImage::Hb201PersonalDataBank,
        0,
        0x4000,
        "7acf34a44a5798c4ca4382e33c8096347de552633722533878ab24b3d4a002b3",
    ),
];

/// Physical firmware images used by the HB-F1XD.
const HBF1XD_IMAGES: &[RomSlot<'static>] = &[
    image(
        FirmwareImage::HbF1XdBios,
        "HB-F1XD BIOS",
        0x8000,
        &["d5425c727a090ab43aca5d98a8092b28d87aa55bc275f5a72a65f16363a7f72f"],
    ),
    image(
        FirmwareImage::HbF1XdSub,
        "HB-F1XD sub ROM",
        0x4000,
        &["9f4b9133b43f82833916ef9e1771245b23ed9ab5426218c7531c453e675a4443"],
    ),
    image(
        FirmwareImage::HbF1XdDisk,
        "HB-F1XD disk ROM",
        0x4000,
        &["f89293a24e85a897a12193bff2a75edead4667f2fa0a43f2fe969abecd14ee44"],
    ),
    FS_CA1_SLOT,
];
/// Logical firmware regions supplied by the HB-F1XD images.
const HBF1XD_REGIONS: &[FirmwareRegionSpec<'static>] = &[
    region(
        FirmwareRegion::Bios,
        FirmwareImage::HbF1XdBios,
        0,
        0x8000,
        "d5425c727a090ab43aca5d98a8092b28d87aa55bc275f5a72a65f16363a7f72f",
    ),
    region(
        FirmwareRegion::SubRom,
        FirmwareImage::HbF1XdSub,
        0,
        0x4000,
        "9f4b9133b43f82833916ef9e1771245b23ed9ab5426218c7531c453e675a4443",
    ),
    region(
        FirmwareRegion::DiskRom,
        FirmwareImage::HbF1XdDisk,
        0,
        0x4000,
        "f89293a24e85a897a12193bff2a75edead4667f2fa0a43f2fe969abecd14ee44",
    ),
];

/// Physical firmware images used by the HB-F1XDJ.
const HBF1XDJ_IMAGES: &[RomSlot<'static>] = &[
    image(
        FirmwareImage::HbF1XdJMain,
        "HB-F1XDJ main ROM",
        0x20000,
        &["cf60edceac5ceff127f719da30f1cf0335b4fce9866ce92edf4823d75bbd0797"],
    ),
    image(
        FirmwareImage::HbF1XdJFirmware,
        "HB-F1XDJ firmware mapper",
        0x100000,
        &["1535249326208c0447d09b918dd025b8e9e3dbd542c7625e509ef41c83c84be1"],
    ),
    image(
        FirmwareImage::HbF1XdJKanji,
        "HB-F1XDJ Kanji font",
        0x40000,
        &["9953b4914d1567ac414fb57279872df830aecbb7dfa83e64ec63eb44b8df42e0"],
    ),
    image(
        FirmwareImage::HbF1XdJOpllInstruments,
        "YM2413 instruments",
        144,
        &["bc83b081e75dd31e0bacd92d5828be6dc8b0ec9b54a715cfc09d3d8dae60c0d3"],
    ),
    FS_CA1_SLOT,
];

/// ROM slot descriptor for the Panasonic FS-CA1 firmware.
const FS_CA1_SLOT: RomSlot<'static> = RomSlot {
    image: FirmwareImage::PanasonicFsCa1,
    label: "Panasonic FS-CA1",
    size: FS_CA1_FIRMWARE_SIZE,
    accepted: &[FS_CA1_CLEAN_BLAKE3, FS_CA1_MAME_BLAKE3],
};
/// Logical firmware regions supplied by the HB-F1XDJ images.
const HBF1XDJ_REGIONS: &[FirmwareRegionSpec<'static>] = &[
    region(
        FirmwareRegion::Bios,
        FirmwareImage::HbF1XdJMain,
        0x00000,
        0x8000,
        "1f8cdb2549c9ea263b150094766e9c0d4970b0312453b2ab2d199f1af43c8405",
    ),
    region(
        FirmwareRegion::SubRom,
        FirmwareImage::HbF1XdJMain,
        0x08000,
        0x4000,
        "6d69d2b8f926a3698125ee631095a132dd2500e15a2f203666e45b5785e5df4d",
    ),
    region(
        FirmwareRegion::DiskRom,
        FirmwareImage::HbF1XdJMain,
        0x0C000,
        0x4000,
        "f89293a24e85a897a12193bff2a75edead4667f2fa0a43f2fe969abecd14ee44",
    ),
    region(
        FirmwareRegion::KanjiDriver,
        FirmwareImage::HbF1XdJMain,
        0x10000,
        0x8000,
        "9185b497aeedfe844eb5ad9f182de75484e3e5511aac070b7e8925029dd28f8a",
    ),
    region(
        FirmwareRegion::MsxMusic,
        FirmwareImage::HbF1XdJMain,
        0x18000,
        0x4000,
        "150b8baa9701275586ae36cd46f15bd24ee30c0ba0d54288377bbb7ff3934840",
    ),
    region(
        FirmwareRegion::FirmwareMapper,
        FirmwareImage::HbF1XdJFirmware,
        0,
        0x100000,
        "1535249326208c0447d09b918dd025b8e9e3dbd542c7625e509ef41c83c84be1",
    ),
    region(
        FirmwareRegion::KanjiFont,
        FirmwareImage::HbF1XdJKanji,
        0,
        0x40000,
        "9953b4914d1567ac414fb57279872df830aecbb7dfa83e64ec63eb44b8df42e0",
    ),
    region(
        FirmwareRegion::OpllInstruments,
        FirmwareImage::HbF1XdJOpllInstruments,
        0,
        144,
        "bc83b081e75dd31e0bacd92d5828be6dc8b0ec9b54a715cfc09d3d8dae60c0d3",
    ),
];

const fn image<'a>(
    image: FirmwareImage,
    label: &'a str,
    size: usize,
    accepted: &'a [&'a str],
) -> RomSlot<'a> {
    RomSlot {
        image,
        label,
        size,
        accepted,
    }
}

const fn region(
    region: FirmwareRegion,
    image: FirmwareImage,
    offset: usize,
    size: usize,
    digest: &'static str,
) -> FirmwareRegionSpec<'static> {
    FirmwareRegionSpec {
        region,
        image,
        offset,
        size,
        digest,
    }
}

/// One loaded logical firmware-region view.
#[derive(Debug, Clone)]
pub struct LoadedFirmwareRegion {
    region: FirmwareRegion,
    image: Arc<[u8]>,
    offset: usize,
    size: usize,
}

impl LoadedFirmwareRegion {
    /// Logical firmware role.
    pub const fn region(&self) -> FirmwareRegion {
        self.region
    }

    /// Bytes in this logical region.
    pub fn bytes(&self) -> &[u8] {
        &self.image[self.offset..self.offset + self.size]
    }

    /// Whether this view shares its physical image with another view.
    pub fn shares_image_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.image, &other.image)
    }
}

/// Hash-verified firmware regions for one model.
#[derive(Debug)]
pub struct LoadedFirmware {
    model: MsxModel,
    regions: Vec<LoadedFirmwareRegion>,
    msx_audio: Option<Arc<[u8]>>,
}

impl LoadedFirmware {
    /// Model for which the firmware was loaded.
    pub const fn model(&self) -> MsxModel {
        self.model
    }

    /// Every logical firmware region in manifest order.
    pub fn regions(&self) -> &[LoadedFirmwareRegion] {
        &self.regions
    }

    /// Finds a logical firmware region.
    pub fn region(&self, region: FirmwareRegion) -> Option<&LoadedFirmwareRegion> {
        self.regions.iter().find(|loaded| loaded.region == region)
    }

    /// Panasonic FS-CA1 firmware.
    pub fn msx_audio(&self) -> Option<&[u8]> {
        self.msx_audio.as_deref()
    }

    #[cfg(test)]
    pub(crate) fn synthetic(model: MsxModel, regions: Vec<(FirmwareRegion, Vec<u8>)>) -> Self {
        Self {
            model,
            regions: regions
                .into_iter()
                .map(|(region, bytes)| {
                    let size = bytes.len();
                    LoadedFirmwareRegion {
                        region,
                        image: Arc::from(bytes),
                        offset: 0,
                        size,
                    }
                })
                .collect(),
            msx_audio: None,
        }
    }
}

/// Firmware loading or validation error.
#[derive(Debug)]
pub enum FirmwareError {
    /// The selected firmware directory could not be scanned.
    Read {
        /// Directory that could not be read.
        directory: String,
        /// Underlying error text.
        message: String,
    },
    /// A required physical image was not found.
    Missing {
        /// Requested model.
        model: MsxModel,
        /// ROM slot label.
        label: String,
        /// Logical regions supplied by the image.
        regions: Vec<FirmwareRegion>,
        /// Accepted BLAKE3 digests.
        accepted: Vec<String>,
    },
    /// The static source layout is invalid or disagrees with its region hash.
    InvalidLayout {
        /// Requested model.
        model: MsxModel,
        /// Logical region with invalid metadata.
        region: FirmwareRegion,
        /// Validation detail.
        message: String,
    },
}

impl fmt::Display for FirmwareError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { directory, message } => {
                write!(
                    formatter,
                    "failed to read firmware directory {directory}: {message}"
                )
            }
            Self::Missing {
                model,
                label,
                regions,
                accepted,
            } => write!(
                formatter,
                "no ROM in the directory matched the {model} {label} slot for regions {} (accepted BLAKE3 digests: {})",
                region_names(regions),
                accepted.join(", ")
            ),
            Self::InvalidLayout {
                model,
                region,
                message,
            } => write!(
                formatter,
                "{model} {region} firmware layout is invalid: {message}"
            ),
        }
    }
}

impl std::error::Error for FirmwareError {}

/// Loads and validates every firmware region required by `model`.
pub fn load_firmware_set(
    model: MsxModel,
    directory: &Path,
) -> Result<LoadedFirmware, FirmwareError> {
    let (images, regions) = model_manifest(model);
    load_from_manifest(model, directory, images, regions)
}

/// Loads a validated manifest from a directory-wide digest map.
fn load_from_manifest(
    model: MsxModel,
    directory: &Path,
    images: &[RomSlot<'_>],
    regions: &[FirmwareRegionSpec<'_>],
) -> Result<LoadedFirmware, FirmwareError> {
    validate_manifest(model, images, regions)?;
    let by_digest = hash_directory(directory, images)?;
    let mut loaded_images = Vec::with_capacity(images.len());

    for slot in images {
        let bytes = slot
            .accepted
            .iter()
            .find_map(|digest| by_digest.get(*digest).cloned())
            .ok_or_else(|| missing_rom(model, slot, regions))?;
        loaded_images.push((slot.image, bytes));
    }

    let mut loaded_regions = Vec::with_capacity(regions.len());
    for region_specification in regions {
        let image = loaded_images
            .iter()
            .find_map(|(image, bytes)| {
                (*image == region_specification.image).then(|| Arc::clone(bytes))
            })
            .ok_or_else(|| FirmwareError::InvalidLayout {
                model,
                region: region_specification.region,
                message: "physical image was not loaded".to_owned(),
            })?;
        let bytes = &image
            [region_specification.offset..region_specification.offset + region_specification.size];
        let actual_hash = rom_loader::blake3_hex(bytes);
        if actual_hash != region_specification.digest {
            return Err(FirmwareError::InvalidLayout {
                model,
                region: region_specification.region,
                message: format!(
                    "region BLAKE3 {actual_hash} does not match {}",
                    region_specification.digest
                ),
            });
        }
        loaded_regions.push(LoadedFirmwareRegion {
            region: region_specification.region,
            image,
            offset: region_specification.offset,
            size: region_specification.size,
        });
    }
    let msx_audio = loaded_images.iter().find_map(|(image, bytes)| {
        (*image == FirmwareImage::PanasonicFsCa1).then(|| Arc::clone(bytes))
    });

    Ok(LoadedFirmware {
        model,
        regions: loaded_regions,
        msx_audio,
    })
}

/// Builds the missing-ROM error for one unmatched slot.
fn missing_rom(
    model: MsxModel,
    slot: &RomSlot<'_>,
    regions: &[FirmwareRegionSpec<'_>],
) -> FirmwareError {
    FirmwareError::Missing {
        model,
        label: slot.label.to_owned(),
        regions: image_regions(slot.image, regions),
        accepted: slot
            .accepted
            .iter()
            .map(|digest| (*digest).to_owned())
            .collect(),
    }
}

/// Validates physical-image and logical-region relationships.
fn validate_manifest(
    model: MsxModel,
    images: &[RomSlot<'_>],
    regions: &[FirmwareRegionSpec<'_>],
) -> Result<(), FirmwareError> {
    for region in regions {
        let Some(image) = images.iter().find(|image| image.image == region.image) else {
            return Err(FirmwareError::InvalidLayout {
                model,
                region: region.region,
                message: "physical image specification is missing".to_owned(),
            });
        };
        let end = region.offset.checked_add(region.size);
        if region.size == 0 || end.is_none_or(|end| end > image.size) {
            return Err(FirmwareError::InvalidLayout {
                model,
                region: region.region,
                message: "source range exceeds the physical image".to_owned(),
            });
        }
    }
    Ok(())
}

/// Maps every known-size ROM in `directory` by its BLAKE3 digest.
fn hash_directory(
    directory: &Path,
    slots: &[RomSlot<'_>],
) -> Result<HashMap<String, Arc<[u8]>>, FirmwareError> {
    let entries = std::fs::read_dir(directory).map_err(|error| FirmwareError::Read {
        directory: directory.display().to_string(),
        message: error.to_string(),
    })?;

    let mut by_digest = HashMap::new();
    for entry in entries {
        let entry = entry.map_err(|error| FirmwareError::Read {
            directory: directory.display().to_string(),
            message: error.to_string(),
        })?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        if !slots.iter().any(|slot| slot.size == bytes.len()) {
            continue;
        }
        by_digest
            .entry(rom_loader::blake3_hex(&bytes))
            .or_insert_with(|| Arc::from(bytes));
    }
    Ok(by_digest)
}

/// Lists the logical regions supplied by one physical image.
fn image_regions(image: FirmwareImage, regions: &[FirmwareRegionSpec<'_>]) -> Vec<FirmwareRegion> {
    if image == FirmwareImage::PanasonicFsCa1 {
        return vec![FirmwareRegion::MsxAudio];
    }
    regions
        .iter()
        .filter_map(|region| (region.image == image).then_some(region.region))
        .collect()
}

/// Formats logical firmware-region names.
fn region_names(regions: &[FirmwareRegion]) -> String {
    regions
        .iter()
        .map(FirmwareRegion::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

fn model_manifest(
    model: MsxModel,
) -> (
    &'static [RomSlot<'static>],
    &'static [FirmwareRegionSpec<'static>],
) {
    match model {
        MsxModel::Msx => (HB201_IMAGES, HB201_REGIONS),
        MsxModel::Msx2 => (HBF1XD_IMAGES, HBF1XD_REGIONS),
        MsxModel::Msx2Plus => (HBF1XDJ_IMAGES, HBF1XDJ_REGIONS),
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use super::*;
    fn temp_directory(tag: &str) -> PathBuf {
        let unique = format!(
            "neetan_msx_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let directory = std::env::temp_dir().join(unique);
        fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn synthetic_manifest_splits_shared_image_without_copying() {
        let directory = temp_directory("split");
        let bytes = b"abcdefgh";
        fs::write(directory.join("arbitrary-name.dat"), bytes).unwrap();
        let image_hash = rom_loader::blake3_hex(bytes);
        let left_hash = rom_loader::blake3_hex(&bytes[..4]);
        let right_hash = rom_loader::blake3_hex(&bytes[4..]);
        let accepted = [image_hash.as_str()];
        let images = [RomSlot {
            image: FirmwareImage::Hb201Bios,
            label: "synthetic combined ROM",
            size: bytes.len(),
            accepted: &accepted,
        }];
        let regions = [
            FirmwareRegionSpec {
                region: FirmwareRegion::Bios,
                image: FirmwareImage::Hb201Bios,
                offset: 0,
                size: 4,
                digest: &left_hash,
            },
            FirmwareRegionSpec {
                region: FirmwareRegion::SubRom,
                image: FirmwareImage::Hb201Bios,
                offset: 4,
                size: 4,
                digest: &right_hash,
            },
        ];

        let loaded = load_from_manifest(MsxModel::Msx, &directory, &images, &regions).unwrap();
        assert_eq!(
            loaded.region(FirmwareRegion::Bios).unwrap().bytes(),
            b"abcd"
        );
        assert_eq!(
            loaded.region(FirmwareRegion::SubRom).unwrap().bytes(),
            b"efgh"
        );
        assert!(
            loaded
                .region(FirmwareRegion::Bios)
                .unwrap()
                .shares_image_with(loaded.region(FirmwareRegion::SubRom).unwrap())
        );

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn missing_slot_reports_label_and_accepted_digests() {
        let directory = temp_directory("missing");
        let error = load_firmware_set(MsxModel::Msx, &directory).unwrap_err();
        let FirmwareError::Missing {
            label, accepted, ..
        } = error
        else {
            panic!("expected missing ROM error");
        };
        assert_eq!(label, HB201_IMAGES[0].label);
        assert_eq!(accepted, HB201_IMAGES[0].accepted);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn missing_directory_reports_read_error() {
        let model = MsxModel::Msx2Plus;
        let result = load_firmware_set(model, Path::new("/nonexistent/msx/firmware"));
        assert!(matches!(result, Err(FirmwareError::Read { .. })));
    }

    #[test]
    fn out_of_range_source_view_is_rejected() {
        let images = [image(
            FirmwareImage::Hb201Bios,
            "synthetic ROM",
            4,
            &["unused"],
        )];
        let regions = [FirmwareRegionSpec {
            region: FirmwareRegion::Bios,
            image: FirmwareImage::Hb201Bios,
            offset: 3,
            size: 2,
            digest: "unused",
        }];
        let result = validate_manifest(MsxModel::Msx, &images, &regions);
        assert!(matches!(result, Err(FirmwareError::InvalidLayout { .. })));
    }
}
