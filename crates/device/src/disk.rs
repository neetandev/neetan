//! Hard disk image format parsers for SASI hard disk emulation.
//!
//! Supports the PC-98 HDD image formats:
//! - **NHD** (.nhd): T98Next format with signature and full geometry header.
//! - **HDI** (.hdi): Anex86 format with compact 32-byte geometry header.
//! - **THD** (.thd): Original T98 format with minimal header, fixed SASI geometry.
//!
//! and the FM Towns HDD image format:
//! - **RAW** (.h0-.h4): headerless flat 512-byte-sector image; the extension
//!   digit is the SCSI drive index.

mod hdi;
mod nhd;
mod thd;

use std::{
    error::Error,
    fmt,
    path::{Path, PathBuf},
};

use common::error;

use crate::disk_backend::DiskBackend;

/// HDI header size (fixed at 32 bytes).
const HDI_HEADER_SIZE: usize = 32;

/// NHD file signature: "T98HDDIMAGE.R0\0" (15 bytes).
const NHD_SIGNATURE: &[u8; 15] = b"T98HDDIMAGE.R0\0";

/// NHD header size (fixed at 512 bytes).
const NHD_HEADER_SIZE: usize = 512;

/// THD header size (fixed at 256 bytes).
const THD_HEADER_SIZE: usize = 256;

/// THD fixed geometry: 33 sectors per track.
const THD_SECTORS_PER_TRACK: u8 = 33;

/// THD fixed geometry: 8 heads.
const THD_HEADS: u8 = 8;

/// THD fixed sector size: 256 bytes.
const THD_SECTOR_SIZE: u16 = 256;

/// Raw (.h0-.h4) images use a fixed 512-byte sector.
const RAW_SECTOR_SIZE: u16 = 512;

/// Synthesized head count for the raw-image geometry container.
const RAW_HEADS: u8 = 8;

/// Synthesized sectors-per-track for the raw-image geometry container.
const RAW_SECTORS_PER_TRACK: u8 = 32;

/// Heads of the classic AT IDE translation geometry.
const AT_FLAT_HEADS: u8 = 16;

/// Sectors per track of the classic AT IDE translation geometry.
const AT_FLAT_SECTORS_PER_TRACK: u8 = 63;

/// Sector size of an AT IDE hard disk.
const AT_FLAT_SECTOR_SIZE: u16 = 512;

/// Cylinder ceiling of CHS addressing (10 cylinder bits).
const AT_FLAT_MAX_CYLINDERS: u16 = 1023;

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

    /// Loads a headerless flat 512-byte-sector image (.h0-.h4). The SCSI path
    /// is purely LBA-based; the CHS geometry is synthesized only to fill the
    /// `HddGeometry` container. The image size must be a nonzero multiple of
    /// 128 KiB (one synthesized cylinder), which every whole-megabyte image
    /// satisfies.
    pub fn from_raw_flat(data: Vec<u8>) -> Result<Self, HddError> {
        let sectors_per_cylinder = RAW_HEADS as usize * RAW_SECTORS_PER_TRACK as usize;
        let cylinder_bytes = sectors_per_cylinder * RAW_SECTOR_SIZE as usize;
        if data.is_empty() || !data.len().is_multiple_of(cylinder_bytes) {
            return Err(HddError::InvalidGeometry {
                field: "raw image size (must be a nonzero multiple of 128 KiB)",
                value: data.len() as u32,
            });
        }
        let cylinders = data.len() / cylinder_bytes;
        if cylinders > u16::MAX as usize {
            return Err(HddError::InvalidGeometry {
                field: "raw image cylinders",
                value: cylinders as u32,
            });
        }
        let geometry = HddGeometry {
            cylinders: cylinders as u16,
            heads: RAW_HEADS,
            sectors_per_track: RAW_SECTORS_PER_TRACK,
            sector_size: RAW_SECTOR_SIZE,
        };
        Ok(Self::from_raw(geometry, HddFormat::Raw, data))
    }

    /// Loads a headerless flat AT IDE image (.hdd) with the classic
    /// 16 head x 63 sector x 512 byte translation geometry. The image size
    /// must be a nonzero whole number of cylinders (516,096 bytes each) up
    /// to the 1023-cylinder CHS ceiling (about 504 MB).
    pub fn from_at_flat(data: Vec<u8>) -> Result<Self, HddError> {
        let cylinder_bytes = AT_FLAT_HEADS as usize
            * AT_FLAT_SECTORS_PER_TRACK as usize
            * AT_FLAT_SECTOR_SIZE as usize;
        if data.is_empty() || !data.len().is_multiple_of(cylinder_bytes) {
            return Err(HddError::InvalidGeometry {
                field: "AT flat image size (must be a nonzero multiple of 504 KiB)",
                value: data.len() as u32,
            });
        }
        let cylinders = data.len() / cylinder_bytes;
        if cylinders > AT_FLAT_MAX_CYLINDERS as usize {
            return Err(HddError::InvalidGeometry {
                field: "AT flat image cylinders (the CHS ceiling is 1023)",
                value: cylinders as u32,
            });
        }
        let geometry = HddGeometry {
            cylinders: cylinders as u16,
            heads: AT_FLAT_HEADS,
            sectors_per_track: AT_FLAT_SECTORS_PER_TRACK,
            sector_size: AT_FLAT_SECTOR_SIZE,
        };
        Ok(Self::from_raw(geometry, HddFormat::AtFlat, data))
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
        HddFormat::Nhd => {
            let mut header = vec![0u8; NHD_HEADER_SIZE];
            header[..15].copy_from_slice(NHD_SIGNATURE);
            header[0x110..0x114].copy_from_slice(&(NHD_HEADER_SIZE as u32).to_le_bytes());
            header[0x114..0x118].copy_from_slice(&(geometry.cylinders as u32).to_le_bytes());
            header[0x118..0x11A].copy_from_slice(&(geometry.heads as u16).to_le_bytes());
            header[0x11A..0x11C]
                .copy_from_slice(&(geometry.sectors_per_track as u16).to_le_bytes());
            header[0x11C..0x11E].copy_from_slice(&geometry.sector_size.to_le_bytes());
            header
        }
        HddFormat::Hdi => {
            let mut header = vec![0u8; HDI_HEADER_SIZE];
            let total_sectors = geometry.total_sectors();
            header[8..12].copy_from_slice(&(HDI_HEADER_SIZE as u32).to_le_bytes());
            header[12..16].copy_from_slice(&total_sectors.to_le_bytes());
            header[16..20].copy_from_slice(&(geometry.sector_size as u32).to_le_bytes());
            header[20..24].copy_from_slice(&(geometry.sectors_per_track as u32).to_le_bytes());
            header[24..28].copy_from_slice(&(geometry.heads as u32).to_le_bytes());
            header[28..32].copy_from_slice(&(geometry.cylinders as u32).to_le_bytes());
            header
        }
        HddFormat::Thd => {
            let mut header = vec![0u8; THD_HEADER_SIZE];
            header[0..2].copy_from_slice(&geometry.cylinders.to_le_bytes());
            header
        }
        // A raw image is the bare sector data with no header, so `to_bytes`
        // round-trips the file byte-for-byte.
        HddFormat::Raw | HddFormat::AtFlat => Vec::new(),
    }
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
    if data.len() >= 15 && &data[..15] == NHD_SIGNATURE {
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

/// Sectors per track of an X68000 SASI hard disk.
const X68K_SASI_SECTORS_PER_TRACK: u8 = 33;

/// Sector size of an X68000 SASI hard disk.
const X68K_SASI_SECTOR_SIZE: u16 = 256;

/// Exact byte size of a 10 MB X68000 SASI .hdf image (309 cylinders, 4 heads).
pub const X68K_SASI_HDF_10MB_BYTES: usize = 10_441_728;

/// Exact byte size of a 20 MB X68000 SASI .hdf image (614 cylinders, 4 heads).
pub const X68K_SASI_HDF_20MB_BYTES: usize = 20_748_288;

/// Exact byte size of a 40 MB X68000 SASI .hdf image (614 cylinders, 8 heads).
pub const X68K_SASI_HDF_40MB_BYTES: usize = 41_496_576;

/// Loads a headerless X68000 .hdf image. `sector_size` selects the
/// controller the image is meant for: 256 (SASI) must match one of the three
/// fixed drive capacities exactly and gets that drive's geometry; 512 (SCSI)
/// accepts any flat image size `from_raw_flat` accepts.
pub fn load_x68k_hdf(data: Vec<u8>, sector_size: u16) -> Result<HddImage, HddError> {
    match sector_size {
        256 => {
            let (cylinders, heads) = match data.len() {
                X68K_SASI_HDF_10MB_BYTES => (309, 4),
                X68K_SASI_HDF_20MB_BYTES => (614, 4),
                X68K_SASI_HDF_40MB_BYTES => (614, 8),
                _ => {
                    return Err(HddError::InvalidGeometry {
                        field: "SASI .hdf size (must be exactly a 10, 20, or 40 MB image)",
                        value: data.len() as u32,
                    });
                }
            };
            let geometry = HddGeometry {
                cylinders,
                heads,
                sectors_per_track: X68K_SASI_SECTORS_PER_TRACK,
                sector_size: X68K_SASI_SECTOR_SIZE,
            };
            Ok(HddImage::from_raw(geometry, HddFormat::Raw, data))
        }
        512 => HddImage::from_raw_flat(data),
        _ => Err(HddError::InvalidGeometry {
            field: "X68000 .hdf sector size (must be 256 or 512)",
            value: sector_size as u32,
        }),
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
    identity: save_state::ResourceIdentity,
    source_path: Option<save_state::MediaSourcePath>,
}

impl MountedHdd {
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
    use super::*;

    fn build_nhd_image(cylinders: u16, heads: u8, spt: u8, sector_size: u16) -> Vec<u8> {
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
        // Fill each sector's first byte with its LBA index (mod 256).
        for lba in 0..total_sectors {
            data[lba * sector_size as usize] = lba as u8;
        }

        header.extend_from_slice(&data);
        header
    }

    fn build_hdi_image(cylinders: u16, heads: u8, spt: u8, sector_size: u16) -> Vec<u8> {
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

    fn build_thd_image(cylinders: u16) -> Vec<u8> {
        let mut header = vec![0u8; THD_HEADER_SIZE];
        header[0..2].copy_from_slice(&cylinders.to_le_bytes());

        let total_sectors =
            cylinders as usize * THD_HEADS as usize * THD_SECTORS_PER_TRACK as usize;
        let data_size = total_sectors * THD_SECTOR_SIZE as usize;
        let mut data = vec![0u8; data_size];
        for lba in 0..total_sectors {
            data[lba * THD_SECTOR_SIZE as usize] = lba as u8;
        }

        header.extend_from_slice(&data);
        header
    }

    #[test]
    fn parse_nhd_5mb() {
        let image = build_nhd_image(153, 4, 33, 256);
        let hdd = HddImage::from_nhd(&image).unwrap();

        assert_eq!(hdd.geometry.cylinders, 153);
        assert_eq!(hdd.geometry.heads, 4);
        assert_eq!(hdd.geometry.sectors_per_track, 33);
        assert_eq!(hdd.geometry.sector_size, 256);
        assert_eq!(hdd.geometry.total_sectors(), 153 * 4 * 33);
        assert_eq!(hdd.format, HddFormat::Nhd);
    }

    #[test]
    fn parse_hdi_10mb() {
        let image = build_hdi_image(310, 4, 33, 256);
        let hdd = HddImage::from_hdi(&image).unwrap();

        assert_eq!(hdd.geometry.cylinders, 310);
        assert_eq!(hdd.geometry.heads, 4);
        assert_eq!(hdd.geometry.sectors_per_track, 33);
        assert_eq!(hdd.geometry.sector_size, 256);
        assert_eq!(hdd.format, HddFormat::Hdi);
    }

    #[test]
    fn parse_thd_20mb() {
        let image = build_thd_image(310);
        let hdd = HddImage::from_thd(&image).unwrap();

        assert_eq!(hdd.geometry.cylinders, 310);
        assert_eq!(hdd.geometry.heads, THD_HEADS);
        assert_eq!(hdd.geometry.sectors_per_track, THD_SECTORS_PER_TRACK);
        assert_eq!(hdd.geometry.sector_size, THD_SECTOR_SIZE);
        assert_eq!(hdd.format, HddFormat::Thd);
    }

    #[test]
    fn parse_raw_h0_round_trips() {
        // 1 MiB image: 2048 sectors of 512 bytes = 8 cyls x 8 heads x 32 spt.
        let mut data = vec![0u8; 1024 * 1024];
        for lba in 0..(data.len() / 512) {
            data[lba * 512] = lba as u8;
        }
        let hdd = load_hdd_image(Path::new("disk.h0"), &data).unwrap();

        assert_eq!(hdd.format, HddFormat::Raw);
        assert_eq!(hdd.geometry.sector_size, 512);
        assert_eq!(hdd.geometry.heads, RAW_HEADS);
        assert_eq!(hdd.geometry.sectors_per_track, RAW_SECTORS_PER_TRACK);
        assert_eq!(hdd.geometry.cylinders, 8);
        assert_eq!(hdd.geometry.total_sectors(), 2048);
        assert_eq!(hdd.read_sector(5).unwrap()[0], 5);
        // Headerless: serialization is byte-identical to the source file.
        assert_eq!(hdd.to_bytes(), data);
    }

    #[test]
    fn raw_h1_extension_also_parses() {
        let data = vec![0u8; 128 * 1024];
        let hdd = load_hdd_image(Path::new("disk.h1"), &data).unwrap();
        assert_eq!(hdd.format, HddFormat::Raw);
        assert_eq!(hdd.geometry.cylinders, 1);
    }

    #[test]
    fn raw_rejects_unaligned_size() {
        // 300 KiB is a multiple of 512 but not of the 128 KiB cylinder size.
        let data = vec![0u8; 300 * 1024];
        assert!(matches!(
            HddImage::from_raw_flat(data),
            Err(HddError::InvalidGeometry { .. })
        ));
    }

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
    fn nhd_roundtrip() {
        let image = build_nhd_image(153, 4, 33, 256);
        let hdd = HddImage::from_nhd(&image).unwrap();
        let serialized = hdd.to_bytes();

        assert_eq!(serialized.len(), image.len());
        // Header should match.
        assert_eq!(&serialized[..15], NHD_SIGNATURE);
        // Data should match.
        let data_start = NHD_HEADER_SIZE;
        assert_eq!(&serialized[data_start..], &image[data_start..]);
    }

    #[test]
    fn hdi_roundtrip() {
        let image = build_hdi_image(310, 4, 33, 256);
        let hdd = HddImage::from_hdi(&image).unwrap();
        let serialized = hdd.to_bytes();

        assert_eq!(serialized.len(), image.len());
        assert_eq!(&serialized[HDI_HEADER_SIZE..], &image[HDI_HEADER_SIZE..]);
    }

    #[test]
    fn thd_roundtrip() {
        let image = build_thd_image(153);
        let hdd = HddImage::from_thd(&image).unwrap();
        let serialized = hdd.to_bytes();

        assert_eq!(serialized.len(), image.len());
        assert_eq!(&serialized[..2], &image[..2]);
        assert_eq!(&serialized[THD_HEADER_SIZE..], &image[THD_HEADER_SIZE..]);
    }

    #[test]
    fn nhd_too_small_rejected() {
        let data = vec![0u8; 100];
        assert!(matches!(
            HddImage::from_nhd(&data),
            Err(HddError::TooSmall { format: "NHD", .. })
        ));
    }

    #[test]
    fn nhd_bad_signature_rejected() {
        let mut image = build_nhd_image(153, 4, 33, 256);
        image[0] = b'X';
        assert!(matches!(
            HddImage::from_nhd(&image),
            Err(HddError::InvalidSignature { format: "NHD", .. })
        ));
    }

    #[test]
    fn hdi_too_small_rejected() {
        let data = vec![0u8; 16];
        assert!(matches!(
            HddImage::from_hdi(&data),
            Err(HddError::TooSmall { format: "HDI", .. })
        ));
    }

    #[test]
    fn thd_too_small_rejected() {
        let data = vec![0u8; 100];
        assert!(matches!(
            HddImage::from_thd(&data),
            Err(HddError::TooSmall { format: "THD", .. })
        ));
    }

    #[test]
    fn thd_zero_cylinders_rejected() {
        let mut image = build_thd_image(153);
        image[0] = 0;
        image[1] = 0;
        assert!(matches!(
            HddImage::from_thd(&image),
            Err(HddError::InvalidGeometry {
                field: "cylinders",
                ..
            })
        ));
    }

    #[test]
    fn nhd_truncated_data_rejected() {
        let mut image = build_nhd_image(153, 4, 33, 256);
        image.truncate(NHD_HEADER_SIZE + 100);
        assert!(matches!(
            HddImage::from_nhd(&image),
            Err(HddError::DataTruncated { .. })
        ));
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
    fn nhd_with_512_byte_sectors() {
        let image = build_nhd_image(100, 4, 17, 512);
        let hdd = HddImage::from_nhd(&image).unwrap();

        assert_eq!(hdd.geometry.sector_size, 512);
        assert_eq!(hdd.geometry.total_sectors(), 100 * 4 * 17);

        let sector = hdd.read_sector(0).unwrap();
        assert_eq!(sector.len(), 512);
    }

    #[test]
    fn hdi_with_larger_header() {
        let mut image = build_hdi_image(153, 4, 33, 256);
        // Simulate a larger header by setting header_size and inserting padding.
        let new_header_size = 4096u32;
        image[8..12].copy_from_slice(&new_header_size.to_le_bytes());
        let padding = vec![0u8; (new_header_size as usize) - HDI_HEADER_SIZE];
        let data_portion = image[HDI_HEADER_SIZE..].to_vec();
        image.truncate(HDI_HEADER_SIZE);
        image.extend_from_slice(&padding);
        image.extend_from_slice(&data_portion);

        let hdd = HddImage::from_hdi(&image).unwrap();
        assert_eq!(hdd.geometry.cylinders, 153);
        assert_eq!(hdd.header_bytes.len(), 4096);

        // Roundtrip preserves the larger header byte-for-byte.
        let serialized = hdd.to_bytes();
        assert_eq!(serialized.len(), image.len());
        assert_eq!(&serialized[..4096], &image[..4096]);
    }

    fn tempfile_with(bytes: &[u8], suffix: &str) -> PathBuf {
        let dir = std::env::temp_dir();
        let unique = format!(
            "neetan_hdd_test_{}_{}{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            suffix
        );
        let path = dir.join(unique);
        std::fs::write(&path, bytes).expect("write temp file");
        path
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
    fn x68k_hdf_sasi_sizes_map_to_drive_geometries() {
        let cases = [
            (X68K_SASI_HDF_10MB_BYTES, 309u16, 4u8),
            (X68K_SASI_HDF_20MB_BYTES, 614, 4),
            (X68K_SASI_HDF_40MB_BYTES, 614, 8),
        ];
        for (bytes, cylinders, heads) in cases {
            let image = load_x68k_hdf(vec![0u8; bytes], 256).unwrap();
            assert_eq!(image.geometry.cylinders, cylinders);
            assert_eq!(image.geometry.heads, heads);
            assert_eq!(image.geometry.sectors_per_track, 33);
            assert_eq!(image.geometry.sector_size, 256);
            assert_eq!(image.geometry.total_bytes(), bytes as u64);
            assert_eq!(image.format, HddFormat::Raw);
            assert!(image.header_bytes.is_empty());
        }
    }

    #[test]
    fn x68k_hdf_sasi_rejects_other_sizes() {
        assert!(load_x68k_hdf(vec![0u8; X68K_SASI_HDF_10MB_BYTES - 256], 256).is_err());
        assert!(load_x68k_hdf(vec![0u8; X68K_SASI_HDF_10MB_BYTES + 256], 256).is_err());
        assert!(load_x68k_hdf(Vec::new(), 256).is_err());
    }

    #[test]
    fn x68k_hdf_scsi_derives_flat_geometry() {
        let bytes = 20 << 20;
        let image = load_x68k_hdf(vec![0u8; bytes], 512).unwrap();
        assert_eq!(image.geometry.heads, 8);
        assert_eq!(image.geometry.sectors_per_track, 32);
        assert_eq!(image.geometry.sector_size, 512);
        assert_eq!(image.geometry.total_bytes(), bytes as u64);
        assert_eq!(image.format, HddFormat::Raw);
        assert!(load_x68k_hdf(vec![0u8; 512], 512).is_err());
    }

    #[test]
    fn x68k_hdf_rejects_unknown_sector_size() {
        assert!(load_x68k_hdf(vec![0u8; X68K_SASI_HDF_10MB_BYTES], 1024).is_err());
    }

    #[test]
    fn x68k_hdf_flushes_headerless_round_trip() {
        let mut data = vec![0u8; X68K_SASI_HDF_10MB_BYTES];
        data[0] = 0x60;
        let path = tempfile_with(&data, ".hdf");

        let image = load_x68k_hdf(data, 256).unwrap();
        let mut mounted = MountedHdd::new(image, Some(path.clone()));
        assert!(mounted.write_sector(1, &[0xA5u8; 256]));
        mounted.flush();

        let written = std::fs::read(&path).unwrap();
        assert_eq!(written.len(), X68K_SASI_HDF_10MB_BYTES);
        assert_eq!(written[0], 0x60);
        assert_eq!(written[256], 0xA5);

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
}
