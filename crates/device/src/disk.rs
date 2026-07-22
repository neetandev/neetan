//! Hard disk image format parsers for SASI, SCSI, and AT/IDE hard disks.
//!
//! Serves PC-98 SASI, FM Towns SCSI, AT/IDE, and X68000 SASI/SCSI machines.
//! Supported formats:
//! - **NHD** (.nhd): T98-Next format with a "T98HDDIMAGE.R0" signature and a
//!   512-byte header carrying full CHS + sector-size geometry.
//! - **HDI** (.hdi): Anex86 format with a compact 32-byte geometry header.
//! - **THD** (.thd): original T98 format with a 256-byte header; heads/SPT/
//!   sector-size are fixed SASI geometry.
//! - **RAW** (.h0-.h4): headerless flat 512-byte-sector image (FM Towns /
//!   X68000 SCSI); the extension digit is the SCSI drive index.
//! - **HDD** (.hdd): headerless flat AT/IDE image with the classic
//!   16 head x 63 SPT x 512-byte translation geometry.
//! - **HDF** (.hdf): headerless X68000 image; the machine model selects its
//!   SASI or SCSI geometry.

pub mod at_flat;
pub mod format;
pub mod hdi;
pub mod nhd;
pub mod raw;
#[cfg(test)]
mod test_support;
pub mod thd;
pub mod x68k_sasi;

use std::{
    error::Error,
    fmt,
    path::{Path, PathBuf},
};

use common::error;

pub use self::x68k_sasi::{
    X68K_SASI_HDF_10MB_BYTES, X68K_SASI_HDF_20MB_BYTES, X68K_SASI_HDF_40MB_BYTES,
};
use crate::disk_backend::DiskBackend;

/// Legacy SENSE (INT 1Bh Function 04h) return values per SASI HDD type index.
const SASI_LEGACY_SENSE: [u8; 7] = [0x00, 0x01, 0x02, 0x03, 0x04, 0x04, 0x05];

/// New SENSE (INT 1Bh Function 84h) return values per SASI HDD type index.
const SASI_NEW_SENSE: [u8; 7] = [0x00, 0x01, 0x02, 0x03, 0x05, 0x05, 0x07];

/// Standard SASI HDD geometry presets (sectors, heads, cylinders).
const SASI_HDD_TYPES: [(u8, u8, u16); 7] = [
    (33, 4, 153), // 5 MB
    (33, 4, 310), // 10 MB
    (33, 6, 310), // 15 MB
    (33, 8, 310), // 20 MB
    (33, 4, 615), // 20 MB (alternate)
    (33, 6, 615), // 30 MB
    (33, 8, 615), // 40 MB
];

/// Disk geometry describing CHS layout and sector size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HddGeometry {
    /// Number of cylinders.
    pub cylinders: u16,
    /// Number of heads (surfaces).
    pub heads: u8,
    /// Number of sectors per track.
    pub sectors_per_track: u8,
    /// Bytes per sector.
    pub sector_size: u16,
}

impl HddGeometry {
    /// Total number of sectors on the disk.
    pub fn total_sectors(&self) -> u32 {
        self.cylinders as u32 * self.heads as u32 * self.sectors_per_track as u32
    }

    /// Total data size in bytes (excluding any image header).
    pub fn total_bytes(&self) -> u64 {
        self.total_sectors() as u64 * self.sector_size as u64
    }

    /// Returns the SASI media type index (0-6) if this geometry matches a
    /// standard SASI HDD type, or `None` if it does not.
    pub fn sasi_media_type(&self) -> Option<u8> {
        if self.sector_size != 256 {
            return None;
        }
        SASI_HDD_TYPES
            .iter()
            .position(|&(spt, heads, cyls)| {
                self.sectors_per_track == spt && self.heads == heads && self.cylinders == cyls
            })
            .map(|i| i as u8)
    }

    /// Returns the legacy SENSE (INT 1Bh Function 04h) capacity code.
    pub fn sasi_legacy_sense_type(&self) -> Option<u8> {
        self.sasi_media_type()
            .map(|i| SASI_LEGACY_SENSE[i as usize])
    }

    /// Returns the new SENSE (INT 1Bh Function 84h) capacity code.
    pub fn sasi_new_sense_type(&self) -> Option<u8> {
        self.sasi_media_type().map(|i| SASI_NEW_SENSE[i as usize])
    }
}

/// The original format of a loaded HDD image (used for serialization).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HddFormat {
    /// T98Next (.nhd).
    Nhd,
    /// Anex86 (.hdi).
    Hdi,
    /// Original T98 (.thd).
    Thd,
    /// Headerless flat 512-byte-sector image (FM Towns .h0-.h4).
    Raw,
    /// Headerless flat AT IDE image with 16 head x 63 sector geometry (.hdd).
    AtFlat,
}

/// A parsed hard disk image.
#[derive(Debug, Clone)]
pub struct HddImage {
    /// Disk geometry.
    pub geometry: HddGeometry,
    /// Original image format.
    pub format: HddFormat,
    /// Raw sector data (geometry.total_sectors() * geometry.sector_size bytes).
    data: Vec<u8>,
    /// Verbatim copy of the source-image header. `to_bytes` emits
    /// `header_bytes ++ data`.
    pub(crate) header_bytes: Vec<u8>,
}

impl HddImage {
    /// Creates an HDD image from raw components (for testing and programmatic creation).
    pub fn from_raw(geometry: HddGeometry, format: HddFormat, data: Vec<u8>) -> Self {
        let header_bytes = synthesize_default_header(format, geometry);
        Self {
            geometry,
            format,
            data,
            header_bytes,
        }
    }

    /// Returns the raw sector data.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Returns a human-readable format name.
    pub fn format_name(&self) -> &'static str {
        match self.format {
            HddFormat::Nhd => "NHD",
            HddFormat::Hdi => "HDI",
            HddFormat::Thd => "THD",
            HddFormat::Raw => "RAW",
            HddFormat::AtFlat => "AT flat",
        }
    }

    /// Reads sector data at the given LBA.
    pub fn read_sector(&self, lba: u32) -> Option<&[u8]> {
        if lba >= self.geometry.total_sectors() {
            return None;
        }
        let offset = lba as usize * self.geometry.sector_size as usize;
        let end = offset + self.geometry.sector_size as usize;
        if end > self.data.len() {
            return None;
        }
        Some(&self.data[offset..end])
    }

    /// Writes sector data at the given LBA. Returns `false` if LBA is out of range
    /// or `data` length does not match the sector size.
    pub fn write_sector(&mut self, lba: u32, sector_data: &[u8]) -> bool {
        if lba >= self.geometry.total_sectors() {
            return false;
        }
        if sector_data.len() != self.geometry.sector_size as usize {
            return false;
        }
        let offset = lba as usize * self.geometry.sector_size as usize;
        let end = offset + self.geometry.sector_size as usize;
        if end > self.data.len() {
            return false;
        }
        self.data[offset..end].copy_from_slice(sector_data);
        true
    }

    /// Formats a track starting at the given LBA by filling sectors with 0xE5.
    pub fn format_track(&mut self, start_lba: u32) -> bool {
        let sectors_per_track = self.geometry.sectors_per_track as u32;
        for i in 0..sectors_per_track {
            let lba = start_lba + i;
            if lba >= self.geometry.total_sectors() {
                return false;
            }
            let offset = lba as usize * self.geometry.sector_size as usize;
            let end = offset + self.geometry.sector_size as usize;
            if end > self.data.len() {
                return false;
            }
            self.data[offset..end].fill(0xE5);
        }
        true
    }

    /// Serializes the image to bytes by emitting the preserved header
    /// followed by the in-memory sector data.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.header_bytes.len() + self.data.len());
        out.extend_from_slice(&self.header_bytes);
        out.extend_from_slice(&self.data);
        out
    }
}

/// Synthesizes a default header for the given format and geometry, matching
/// what the format-specific parsers would have produced for a freshly-built
/// image.
fn synthesize_default_header(format: HddFormat, geometry: HddGeometry) -> Vec<u8> {
    match format {
        HddFormat::Nhd => nhd::synth_header(geometry),
        HddFormat::Hdi => hdi::synth_header(geometry),
        HddFormat::Thd => thd::synth_header(geometry),
        // A raw image is the bare sector data with no header, so `to_bytes`
        // round-trips the file byte-for-byte.
        HddFormat::Raw | HddFormat::AtFlat => Vec::new(),
    }
}

impl HddFormat {
    /// Returns the canonical file extension for this image format. Used when a
    /// programmatically built image is re-parsed by [`load_hdd_image`].
    pub fn file_extension(self) -> &'static str {
        match self {
            HddFormat::Nhd => "nhd",
            HddFormat::Hdi => "hdi",
            HddFormat::Thd => "thd",
            HddFormat::Raw => "h0",
            HddFormat::AtFlat => "hdd",
        }
    }
}

/// A named hard-disk capacity and container format, selectable on the command
/// line and by the automation `create-hdd!` procedure.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum HddSizeType {
    /// PC-98 SASI 5 MB (.hdi).
    Mb5,
    /// PC-98 SASI 10 MB (.hdi).
    Mb10,
    /// PC-98 SASI 15 MB (.hdi).
    Mb15,
    /// PC-98 SASI 20 MB (.hdi).
    Mb20,
    /// PC-98 SASI 30 MB (.hdi).
    Mb30,
    /// PC-98 SASI 40 MB (.hdi).
    Mb40,
    /// PC-98 IDE 40 MB (.hdi).
    IdeMb40,
    /// PC-98 IDE 80 MB (.hdi).
    IdeMb80,
    /// PC-98 IDE 120 MB (.hdi).
    IdeMb120,
    /// PC-98 IDE 200 MB (.hdi).
    IdeMb200,
    /// PC-98 IDE 500 MB (.hdi).
    IdeMb500,
    /// FM Towns SCSI 20 MB (.h0-.h4).
    ScsiMb20,
    /// FM Towns SCSI 40 MB (.h0-.h4).
    ScsiMb40,
    /// FM Towns SCSI 100 MB (.h0-.h4).
    ScsiMb100,
    /// FM Towns SCSI 200 MB (.h0-.h4).
    ScsiMb200,
    /// FM Towns SCSI 340 MB (.h0-.h4).
    ScsiMb340,
    /// FM Towns SCSI 540 MB (.h0-.h4).
    ScsiMb540,
    /// X68000 SASI 10 MB (.hdf).
    X68kSasiMb10,
    /// X68000 SASI 20 MB (.hdf).
    X68kSasiMb20,
    /// X68000 SASI 40 MB (.hdf).
    X68kSasiMb40,
    /// X68000 SCSI 20 MB (.hdf).
    X68kScsiMb20,
    /// X68000 SCSI 40 MB (.hdf).
    X68kScsiMb40,
    /// PC/AT flat 40 MB (.hdd).
    AtMb40,
    /// PC/AT flat 100 MB (.hdd).
    AtMb100,
    /// PC/AT flat 250 MB (.hdd).
    AtMb250,
    /// PC/AT flat 504 MB (.hdd).
    AtMb504,
}

impl std::str::FromStr for HddSizeType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "sasi5" => Ok(Self::Mb5),
            "sasi10" => Ok(Self::Mb10),
            "sasi15" => Ok(Self::Mb15),
            "sasi20" => Ok(Self::Mb20),
            "sasi30" => Ok(Self::Mb30),
            "sasi40" => Ok(Self::Mb40),
            "ide40" => Ok(Self::IdeMb40),
            "ide80" => Ok(Self::IdeMb80),
            "ide120" => Ok(Self::IdeMb120),
            "ide200" => Ok(Self::IdeMb200),
            "ide500" => Ok(Self::IdeMb500),
            "scsi20" => Ok(Self::ScsiMb20),
            "scsi40" => Ok(Self::ScsiMb40),
            "scsi100" => Ok(Self::ScsiMb100),
            "scsi200" => Ok(Self::ScsiMb200),
            "scsi340" => Ok(Self::ScsiMb340),
            "scsi540" => Ok(Self::ScsiMb540),
            "x68sasi10" => Ok(Self::X68kSasiMb10),
            "x68sasi20" => Ok(Self::X68kSasiMb20),
            "x68sasi40" => Ok(Self::X68kSasiMb40),
            "x68scsi20" => Ok(Self::X68kScsiMb20),
            "x68scsi40" => Ok(Self::X68kScsiMb40),
            "at40" => Ok(Self::AtMb40),
            "at100" => Ok(Self::AtMb100),
            "at250" => Ok(Self::AtMb250),
            "at504" => Ok(Self::AtMb504),
            _ => Err(format!(
                "unknown HDD size '{s}', expected sasi5, sasi10, sasi15, sasi20, sasi30, sasi40, \
                 ide40, ide80, ide120, ide200, ide500, scsi20, scsi40, scsi100, scsi200, scsi340, \
                 scsi540, x68sasi10, x68sasi20, x68sasi40, x68scsi20, x68scsi40, at40, at100, \
                 at250, or at504"
            )),
        }
    }
}

impl HddSizeType {
    /// Whether this size denotes an FM Towns raw SCSI image (.h0-.h4) rather
    /// than a PC-98 SASI/IDE header format (.hdi).
    pub fn is_scsi_raw(self) -> bool {
        matches!(
            self,
            Self::ScsiMb20
                | Self::ScsiMb40
                | Self::ScsiMb100
                | Self::ScsiMb200
                | Self::ScsiMb340
                | Self::ScsiMb540
        )
    }

    /// Whether this size denotes an X68000 headerless .hdf image.
    pub fn is_x68k_hdf(self) -> bool {
        matches!(
            self,
            Self::X68kSasiMb10
                | Self::X68kSasiMb20
                | Self::X68kSasiMb40
                | Self::X68kScsiMb20
                | Self::X68kScsiMb40
        )
    }

    /// Whether this size denotes an AT headerless flat .hdd image.
    pub fn is_at_flat(self) -> bool {
        matches!(
            self,
            Self::AtMb40 | Self::AtMb100 | Self::AtMb250 | Self::AtMb504
        )
    }

    /// Returns the canonical short token for this size, the inverse of the
    /// [`std::str::FromStr`] parse (for example [`Self::Mb40`] is `"sasi40"`).
    pub fn as_token(self) -> &'static str {
        match self {
            Self::Mb5 => "sasi5",
            Self::Mb10 => "sasi10",
            Self::Mb15 => "sasi15",
            Self::Mb20 => "sasi20",
            Self::Mb30 => "sasi30",
            Self::Mb40 => "sasi40",
            Self::IdeMb40 => "ide40",
            Self::IdeMb80 => "ide80",
            Self::IdeMb120 => "ide120",
            Self::IdeMb200 => "ide200",
            Self::IdeMb500 => "ide500",
            Self::ScsiMb20 => "scsi20",
            Self::ScsiMb40 => "scsi40",
            Self::ScsiMb100 => "scsi100",
            Self::ScsiMb200 => "scsi200",
            Self::ScsiMb340 => "scsi340",
            Self::ScsiMb540 => "scsi540",
            Self::X68kSasiMb10 => "x68sasi10",
            Self::X68kSasiMb20 => "x68sasi20",
            Self::X68kSasiMb40 => "x68sasi40",
            Self::X68kScsiMb20 => "x68scsi20",
            Self::X68kScsiMb40 => "x68scsi40",
            Self::AtMb40 => "at40",
            Self::AtMb100 => "at100",
            Self::AtMb250 => "at250",
            Self::AtMb504 => "at504",
        }
    }

    /// Returns the CHS geometry and container format for this size.
    pub fn geometry(self) -> (HddGeometry, HddFormat) {
        // The SCSI geometry is purely a container: 8 heads x 32 sectors of
        // 512 bytes per cylinder (128 KiB), so cylinders = megabytes x 8.
        let (cylinders, heads, sectors_per_track, sector_size, format) = match self {
            Self::Mb5 => (153u16, 4u8, 33u8, 256u16, HddFormat::Hdi),
            Self::Mb10 => (310, 4, 33, 256, HddFormat::Hdi),
            Self::Mb15 => (310, 6, 33, 256, HddFormat::Hdi),
            Self::Mb20 => (310, 8, 33, 256, HddFormat::Hdi),
            Self::Mb30 => (615, 6, 33, 256, HddFormat::Hdi),
            Self::Mb40 => (615, 8, 33, 256, HddFormat::Hdi),
            Self::IdeMb40 => (977, 5, 17, 512, HddFormat::Hdi),
            Self::IdeMb80 => (977, 10, 17, 512, HddFormat::Hdi),
            Self::IdeMb120 => (977, 15, 17, 512, HddFormat::Hdi),
            Self::IdeMb200 => (977, 15, 28, 512, HddFormat::Hdi),
            Self::IdeMb500 => (1015, 16, 63, 512, HddFormat::Hdi),
            Self::ScsiMb20 => (20 * 8, 8, 32, 512, HddFormat::Raw),
            Self::ScsiMb40 => (40 * 8, 8, 32, 512, HddFormat::Raw),
            Self::ScsiMb100 => (100 * 8, 8, 32, 512, HddFormat::Raw),
            Self::ScsiMb200 => (200 * 8, 8, 32, 512, HddFormat::Raw),
            Self::ScsiMb340 => (340 * 8, 8, 32, 512, HddFormat::Raw),
            Self::ScsiMb540 => (540 * 8, 8, 32, 512, HddFormat::Raw),
            Self::X68kSasiMb10 => (309, 4, 33, 256, HddFormat::Raw),
            Self::X68kSasiMb20 => (614, 4, 33, 256, HddFormat::Raw),
            Self::X68kSasiMb40 => (614, 8, 33, 256, HddFormat::Raw),
            Self::X68kScsiMb20 => (20 * 8, 8, 32, 512, HddFormat::Raw),
            Self::X68kScsiMb40 => (40 * 8, 8, 32, 512, HddFormat::Raw),
            Self::AtMb40 => (81, 16, 63, 512, HddFormat::AtFlat),
            Self::AtMb100 => (203, 16, 63, 512, HddFormat::AtFlat),
            Self::AtMb250 => (507, 16, 63, 512, HddFormat::AtFlat),
            Self::AtMb504 => (1023, 16, 63, 512, HddFormat::AtFlat),
        };
        (
            HddGeometry {
                cylinders,
                heads,
                sectors_per_track,
                sector_size,
            },
            format,
        )
    }
}

/// Builds a blank, zero-filled hard-disk image for the given size in memory.
pub fn blank_hdd_image(size: HddSizeType) -> HddImage {
    let (geometry, format) = size.geometry();
    let data = vec![0u8; geometry.total_bytes() as usize];
    HddImage::from_raw(geometry, format, data)
}

/// Validates geometry parameters are within acceptable bounds.
fn validate_geometry(
    cylinders: u32,
    heads: u32,
    sectors_per_track: u32,
    sector_size: u16,
) -> Result<(), HddError> {
    if cylinders == 0 {
        return Err(HddError::InvalidGeometry {
            field: "cylinders",
            value: cylinders,
        });
    }
    if heads == 0 {
        return Err(HddError::InvalidGeometry {
            field: "heads",
            value: heads,
        });
    }
    if sectors_per_track == 0 {
        return Err(HddError::InvalidGeometry {
            field: "sectors_per_track",
            value: sectors_per_track,
        });
    }
    if sector_size == 0 || !sector_size.is_power_of_two() {
        return Err(HddError::InvalidGeometry {
            field: "sector_size",
            value: sector_size as u32,
        });
    }
    Ok(())
}

/// Loads an HDD image, auto-detecting the format by file extension and signature.
pub fn load_hdd_image(path: &Path, data: &[u8]) -> Result<HddImage, HddError> {
    // Try NHD first (has a signature).
    if data.len() >= 15 && &data[..15] == nhd::NHD_SIGNATURE {
        return HddImage::from_nhd(data);
    }

    // Fall back to extension-based detection.
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());

    match extension.as_deref() {
        Some("nhd") => HddImage::from_nhd(data),
        Some("hdi") => HddImage::from_hdi(data),
        Some("thd") => HddImage::from_thd(data),
        Some("h0" | "h1" | "h2" | "h3" | "h4") => HddImage::from_raw_flat(data.to_vec()),
        Some("hdd") => HddImage::from_at_flat(data.to_vec()),
        _ => Err(HddError::UnrecognizedFormat),
    }
}

/// Error type for HDD image parsing.
#[derive(Debug, Clone)]
pub enum HddError {
    /// Image data too small for the format header.
    TooSmall {
        /// Format name.
        format: &'static str,
        /// Minimum required size.
        minimum: usize,
        /// Actual data size.
        actual: usize,
    },
    /// File signature does not match expected value.
    InvalidSignature {
        /// Format name.
        format: &'static str,
        /// Expected signature string.
        expected: &'static str,
    },
    /// A geometry field has an invalid value.
    InvalidGeometry {
        /// Which field is invalid.
        field: &'static str,
        /// The invalid value.
        value: u32,
    },
    /// Image data is shorter than the geometry requires.
    DataTruncated {
        /// Expected minimum file size.
        expected: usize,
        /// Actual file size.
        actual: usize,
    },
    /// File extension not recognized as a supported HDD format.
    UnrecognizedFormat,
}

impl fmt::Display for HddError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HddError::TooSmall {
                format,
                minimum,
                actual,
            } => write!(
                f,
                "{format} image too small: need at least {minimum} bytes, got {actual}"
            ),
            HddError::InvalidSignature { format, expected } => {
                write!(
                    f,
                    "{format} image has invalid signature, expected {expected}"
                )
            }
            HddError::InvalidGeometry { field, value } => {
                write!(f, "invalid HDD geometry: {field} = {value}")
            }
            HddError::DataTruncated { expected, actual } => {
                write!(
                    f,
                    "HDD image data truncated: expected {expected} bytes, got {actual}"
                )
            }
            HddError::UnrecognizedFormat => write!(f, "unrecognized HDD image format"),
        }
    }
}

impl Error for HddError {}

/// A hard disk image bound to its source file for synchronous write-through.
#[derive(Debug)]
pub struct MountedHdd {
    image: HddImage,
    backend: Option<DiskBackend>,
    dirty: bool,
    read_only: bool,
    identity: save_state::ResourceIdentity,
    source_path: Option<save_state::MediaSourcePath>,
}

/// Builds a mount from a media backing.
pub fn mounted_hdd_from_backing(image: HddImage, backing: common::MediaBacking) -> MountedHdd {
    match backing {
        common::MediaBacking::Ram => MountedHdd::new(image, None),
        common::MediaBacking::WriteThrough(path) => MountedHdd::new(image, Some(path)),
        common::MediaBacking::ReadOnly => MountedHdd::new_read_only(image),
    }
}

impl MountedHdd {
    /// Constructs a read-only mount whose guest writes are dropped.
    ///
    /// The image stays pristine in memory and nothing is written back.
    pub fn new_read_only(image: HddImage) -> Self {
        let mut mount = Self::new(image, None);
        mount.read_only = true;
        mount
    }

    /// Returns the current in-memory image bytes.
    pub fn image_bytes(&self) -> Vec<u8> {
        self.image.to_bytes()
    }

    /// Returns whether guest writes to this mount are dropped.
    pub const fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Constructs a new mount. If `path` is `None` or the file cannot be
    /// opened for write, writes only land in memory.
    pub fn new(image: HddImage, path: Option<PathBuf>) -> Self {
        let structure = hdd_identity_structure(&image);
        let byte_length = image.header_bytes.len() as u64 + image.data.len() as u64;
        let source_path = path.as_deref().map(save_state::MediaSourcePath::from_path);
        let identity = match source_path.as_ref() {
            Some(source_path) => crate::media_identity::path_identity(
                "neetan-hdd-source-v1",
                source_path,
                byte_length,
                &structure,
            ),
            None => crate::media_identity::anonymous_identity(
                "neetan-hdd-source-v1",
                byte_length,
                &structure,
            ),
        };
        let backend = path.and_then(|p| match DiskBackend::open(p.clone()) {
            Ok(b) => Some(b),
            Err(err) => {
                error!(
                    "Failed to open HDD {} for write-through: {err}",
                    p.display()
                );
                None
            }
        });
        Self {
            image,
            backend,
            dirty: false,
            read_only: false,
            identity,
            source_path,
        }
    }

    /// Returns a read-only reference to the parsed image.
    pub fn image(&self) -> &HddImage {
        &self.image
    }

    /// Returns the stable identity recorded when the image was mounted.
    pub const fn identity(&self) -> save_state::ResourceIdentity {
        self.identity
    }

    /// Returns the normalized configured source path, when file-backed.
    pub const fn source_path(&self) -> Option<&save_state::MediaSourcePath> {
        self.source_path.as_ref()
    }

    /// Returns the disk geometry.
    pub fn geometry(&self) -> HddGeometry {
        self.image.geometry
    }

    /// Returns whether the image has unwritten changes.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Reads sector data at the given LBA.
    pub fn read_sector(&self, lba: u32) -> Option<&[u8]> {
        self.image.read_sector(lba)
    }

    /// Writes sector data at the given LBA.
    pub fn write_sector(&mut self, lba: u32, data: &[u8]) -> bool {
        if self.read_only {
            return true;
        }
        if !self.image.write_sector(lba, data) {
            return false;
        }
        if let Some(backend) = self.backend.as_mut() {
            let offset = self.image.header_bytes.len() as u64
                + lba as u64 * self.image.geometry.sector_size as u64;
            if let Err(err) = backend.write_at(offset, data) {
                self.dirty = true;
                error!("HDD write-through failed at LBA {lba}: {err}");
            }
        } else {
            self.dirty = true;
        }
        true
    }

    /// Formats the track containing `start_lba` by filling its sectors
    /// with 0xE5.
    pub fn format_track(&mut self, start_lba: u32) -> bool {
        if self.read_only {
            return true;
        }
        if !self.image.format_track(start_lba) {
            return false;
        }
        if let Some(backend) = self.backend.as_mut() {
            let sector_size = self.image.geometry.sector_size as usize;
            let spt = self.image.geometry.sectors_per_track as usize;
            let offset =
                self.image.header_bytes.len() as u64 + start_lba as u64 * sector_size as u64;
            let buf = vec![0xE5u8; spt * sector_size];
            if let Err(err) = backend.write_at(offset, &buf) {
                self.dirty = true;
                error!("HDD format_track write-through failed: {err}");
            }
        } else {
            self.dirty = true;
        }
        true
    }

    /// Re-emits the entire image if dirty. The dirty flag remains set
    /// only when an earlier per-sector write-through reported an error,
    /// so under normal use this is a no-op.
    pub fn flush_if_dirty(&mut self) {
        if !self.dirty {
            return;
        }
        let Some(backend) = self.backend.as_mut() else {
            return;
        };
        let bytes = self.image.to_bytes();
        if let Err(err) = backend.replace_atomic(&bytes) {
            error!("HDD eject-time flush failed: {err}");
            return;
        }
        self.dirty = false;
    }

    /// Flushes dirty fallback data and any buffered successful writes.
    pub fn flush(&mut self) {
        self.flush_if_dirty();
        if let Some(backend) = self.backend.as_mut()
            && let Err(err) = backend.flush()
        {
            self.dirty = true;
            error!("HDD flush failed: {err}");
        }
    }

    /// Flushes pending writes and releases the backend.
    pub fn eject(mut self) {
        self.flush();
    }
}

fn hdd_identity_structure(image: &HddImage) -> Vec<u8> {
    let mut structure = Vec::with_capacity(18);
    structure.push(match image.format {
        HddFormat::Nhd => 0,
        HddFormat::Hdi => 1,
        HddFormat::Thd => 2,
        HddFormat::Raw => 3,
        HddFormat::AtFlat => 4,
    });
    structure.extend_from_slice(&image.geometry.cylinders.to_le_bytes());
    structure.push(image.geometry.heads);
    structure.push(image.geometry.sectors_per_track);
    structure.extend_from_slice(&image.geometry.sector_size.to_le_bytes());
    structure.extend_from_slice(&(image.header_bytes.len() as u64).to_le_bytes());
    structure
}

#[cfg(test)]
mod tests {
    use super::{
        hdi::HDI_HEADER_SIZE,
        nhd::NHD_HEADER_SIZE,
        test_support::{build_hdi_image, build_nhd_image, build_thd_image, tempfile_with},
        thd::{THD_HEADER_SIZE, THD_SECTOR_SIZE},
        *,
    };

    #[test]
    fn read_sector_at_various_lbas() {
        let image = build_nhd_image(153, 4, 33, 256);
        let hdd = HddImage::from_nhd(&image).unwrap();

        // LBA 0
        let sector = hdd.read_sector(0).unwrap();
        assert_eq!(sector[0], 0);

        // LBA 42
        let sector = hdd.read_sector(42).unwrap();
        assert_eq!(sector[0], 42);

        // LBA 255
        let sector = hdd.read_sector(255).unwrap();
        assert_eq!(sector[0], 255);

        // LBA 256 wraps in our test pattern
        let sector = hdd.read_sector(256).unwrap();
        assert_eq!(sector[0], 0);
    }

    #[test]
    fn read_last_sector() {
        let image = build_nhd_image(153, 4, 33, 256);
        let hdd = HddImage::from_nhd(&image).unwrap();

        let last_lba = hdd.geometry.total_sectors() - 1;
        assert!(hdd.read_sector(last_lba).is_some());
        assert!(hdd.read_sector(last_lba + 1).is_none());
    }

    #[test]
    fn read_out_of_bounds_returns_none() {
        let image = build_nhd_image(153, 4, 33, 256);
        let hdd = HddImage::from_nhd(&image).unwrap();

        assert!(hdd.read_sector(hdd.geometry.total_sectors()).is_none());
        assert!(hdd.read_sector(u32::MAX).is_none());
    }

    #[test]
    fn write_sector_and_read_back() {
        let image = build_nhd_image(153, 4, 33, 256);
        let mut hdd = HddImage::from_nhd(&image).unwrap();

        let new_data = vec![0xAB; 256];
        assert!(hdd.write_sector(10, &new_data));

        let sector = hdd.read_sector(10).unwrap();
        assert_eq!(sector, &new_data[..]);
    }

    #[test]
    fn write_sector_wrong_size_fails() {
        let image = build_nhd_image(153, 4, 33, 256);
        let mut hdd = HddImage::from_nhd(&image).unwrap();

        let wrong_size = vec![0xAB; 512];
        assert!(!hdd.write_sector(0, &wrong_size));
    }

    #[test]
    fn write_sector_out_of_bounds_fails() {
        let image = build_nhd_image(153, 4, 33, 256);
        let mut hdd = HddImage::from_nhd(&image).unwrap();

        let data = vec![0xAB; 256];
        assert!(!hdd.write_sector(hdd.geometry.total_sectors(), &data));
    }

    #[test]
    fn format_track_fills_with_e5() {
        let image = build_nhd_image(153, 4, 33, 256);
        let mut hdd = HddImage::from_nhd(&image).unwrap();

        assert!(hdd.format_track(0));

        for lba in 0..33 {
            let sector = hdd.read_sector(lba).unwrap();
            assert!(
                sector.iter().all(|&b| b == 0xE5),
                "LBA {lba} not filled with 0xE5"
            );
        }
    }

    #[test]
    fn auto_detect_nhd_by_signature() {
        let image = build_nhd_image(153, 4, 33, 256);
        let hdd = load_hdd_image(Path::new("test.nhd"), &image).unwrap();
        assert_eq!(hdd.format, HddFormat::Nhd);
    }

    #[test]
    fn auto_detect_nhd_by_signature_regardless_of_extension() {
        let image = build_nhd_image(153, 4, 33, 256);
        let hdd = load_hdd_image(Path::new("test.hdi"), &image).unwrap();
        assert_eq!(hdd.format, HddFormat::Nhd);
    }

    #[test]
    fn auto_detect_hdi_by_extension() {
        let image = build_hdi_image(310, 4, 33, 256);
        let hdd = load_hdd_image(Path::new("test.hdi"), &image).unwrap();
        assert_eq!(hdd.format, HddFormat::Hdi);
    }

    #[test]
    fn auto_detect_thd_by_extension() {
        let image = build_thd_image(153);
        let hdd = load_hdd_image(Path::new("test.thd"), &image).unwrap();
        assert_eq!(hdd.format, HddFormat::Thd);
    }

    #[test]
    fn unknown_extension_rejected() {
        let data = vec![0u8; 1024];
        assert!(matches!(
            load_hdd_image(Path::new("test.xyz"), &data),
            Err(HddError::UnrecognizedFormat)
        ));
    }

    #[test]
    fn sasi_media_type_detection() {
        let geometry_5mb = HddGeometry {
            cylinders: 153,
            heads: 4,
            sectors_per_track: 33,
            sector_size: 256,
        };
        assert_eq!(geometry_5mb.sasi_media_type(), Some(0));

        let geometry_40mb = HddGeometry {
            cylinders: 615,
            heads: 8,
            sectors_per_track: 33,
            sector_size: 256,
        };
        assert_eq!(geometry_40mb.sasi_media_type(), Some(6));

        let non_sasi = HddGeometry {
            cylinders: 100,
            heads: 4,
            sectors_per_track: 33,
            sector_size: 512,
        };
        assert_eq!(non_sasi.sasi_media_type(), None);
    }

    #[test]
    fn mounted_hdd_nhd_sector_write_through() {
        let image_bytes = build_nhd_image(50, 4, 17, 512);
        let path = tempfile_with(&image_bytes, ".nhd");

        let image = HddImage::from_nhd(&image_bytes).unwrap();
        let mut mounted = MountedHdd::new(image, Some(path.clone()));
        let identity = mounted.identity();

        let pattern = vec![0xAAu8; 512];
        assert!(mounted.write_sector(123, &pattern));

        drop(mounted);
        let raw = std::fs::read(&path).unwrap();
        let offset = NHD_HEADER_SIZE + 123 * 512;
        assert_eq!(&raw[offset..offset + 512], &pattern[..]);
        let remounted = MountedHdd::new(HddImage::from_nhd(&raw).unwrap(), Some(path.clone()));
        assert_eq!(remounted.identity(), identity);
        assert_eq!(
            remounted.source_path(),
            Some(&save_state::MediaSourcePath::from_path(&path))
        );
        drop(remounted);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn mounted_hdd_hdi_format_track_writes_e5() {
        let image_bytes = build_hdi_image(20, 4, 17, 256);
        let path = tempfile_with(&image_bytes, ".hdi");

        let image = HddImage::from_hdi(&image_bytes).unwrap();
        let mut mounted = MountedHdd::new(image, Some(path.clone()));

        // Format the track containing LBA 17 (cylinder 0, head 1).
        assert!(mounted.format_track(17));

        drop(mounted);
        let raw = std::fs::read(&path).unwrap();
        let track_start = HDI_HEADER_SIZE + 17 * 256;
        let track_end = track_start + 17 * 256;
        assert!(
            raw[track_start..track_end].iter().all(|&b| b == 0xE5),
            "track region should be filled with 0xE5"
        );

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn mounted_hdd_thd_sector_write_through() {
        let image_bytes = build_thd_image(50);
        let path = tempfile_with(&image_bytes, ".thd");

        let image = HddImage::from_thd(&image_bytes).unwrap();
        let mut mounted = MountedHdd::new(image, Some(path.clone()));

        let pattern = vec![0x77u8; THD_SECTOR_SIZE as usize];
        assert!(mounted.write_sector(42, &pattern));

        drop(mounted);
        let raw = std::fs::read(&path).unwrap();
        let offset = THD_HEADER_SIZE + 42 * THD_SECTOR_SIZE as usize;
        assert_eq!(
            &raw[offset..offset + THD_SECTOR_SIZE as usize],
            &pattern[..]
        );

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn mounted_hdd_write_sector_preserves_existing_dirty_bit() {
        let image_bytes = build_nhd_image(50, 4, 17, 512);
        let path = tempfile_with(&image_bytes, ".nhd");

        let image = HddImage::from_nhd(&image_bytes).unwrap();
        let mut mounted = MountedHdd::new(image, Some(path.clone()));

        // Simulate a prior write-through error: in-memory ahead of disk.
        mounted.image.write_sector(7, &[0x11u8; 512]);
        mounted.dirty = true;

        // A successful unrelated write must NOT clear dirty, otherwise the
        // earlier in-memory-only mutation is silently lost on flush.
        let pattern = vec![0xAAu8; 512];
        assert!(mounted.write_sector(123, &pattern));
        assert!(
            mounted.is_dirty(),
            "dirty must remain set after later successful write"
        );

        drop(mounted);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn mounted_hdd_format_track_preserves_existing_dirty_bit() {
        let image_bytes = build_nhd_image(50, 4, 17, 512);
        let path = tempfile_with(&image_bytes, ".nhd");

        let image = HddImage::from_nhd(&image_bytes).unwrap();
        let mut mounted = MountedHdd::new(image, Some(path.clone()));

        // Simulate a prior write-through error: in-memory ahead of disk.
        mounted.image.write_sector(7, &[0x11u8; 512]);
        mounted.dirty = true;

        // A successful format_track must NOT clear dirty, otherwise the
        // earlier in-memory-only mutation is silently lost on flush.
        assert!(mounted.format_track(34));
        assert!(
            mounted.is_dirty(),
            "dirty must remain set after later successful format_track"
        );

        drop(mounted);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn ram_backing_keeps_writes_in_memory_only() {
        let image_bytes = build_nhd_image(50, 4, 17, 512);
        let image = HddImage::from_nhd(&image_bytes).unwrap();
        let mut mounted = mounted_hdd_from_backing(image, common::MediaBacking::Ram);
        assert!(!mounted.is_read_only());

        let pattern = [0x42u8; 512];
        assert!(mounted.write_sector(7, &pattern));
        assert_eq!(mounted.read_sector(7), Some(&pattern[..]));

        // The write is visible in the serialized in-memory image.
        let reparsed = HddImage::from_nhd(&mounted.image_bytes()).unwrap();
        assert_eq!(reparsed.read_sector(7), Some(&pattern[..]));
    }

    #[test]
    fn read_only_backing_drops_writes_and_stays_pristine() {
        let image_bytes = build_nhd_image(50, 4, 17, 512);
        let pristine = HddImage::from_nhd(&image_bytes).unwrap().to_bytes();
        let image = HddImage::from_nhd(&image_bytes).unwrap();
        let mut mounted = mounted_hdd_from_backing(image, common::MediaBacking::ReadOnly);
        assert!(mounted.is_read_only());

        let pattern = [0x42u8; 512];
        // The write reports success to the guest but is dropped.
        assert!(mounted.write_sector(7, &pattern));

        assert_eq!(
            mounted.image_bytes(),
            pristine,
            "read-only image must stay pristine"
        );
    }

    #[test]
    fn write_through_backing_persists_to_host_file() {
        let image_bytes = build_nhd_image(50, 4, 17, 512);
        let path = tempfile_with(&image_bytes, ".nhd");
        let image = HddImage::from_nhd(&image_bytes).unwrap();
        let mut mounted =
            mounted_hdd_from_backing(image, common::MediaBacking::WriteThrough(path.clone()));
        assert!(!mounted.is_read_only());

        let pattern = [0x42u8; 512];
        assert!(mounted.write_sector(7, &pattern));
        mounted.flush();
        drop(mounted);

        let raw = std::fs::read(&path).unwrap();
        let reparsed = HddImage::from_nhd(&raw).unwrap();
        assert_eq!(reparsed.read_sector(7), Some(&pattern[..]));

        std::fs::remove_file(&path).ok();
    }
}
